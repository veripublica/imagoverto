//! Estimating the quality a JPEG was saved at.

/// The JPEG Annex K luminance quantisation table, in natural (row-major)
/// order: the table libjpeg scales for its quality 1–100.
#[rustfmt::skip]
const ANNEX_K_LUMA: [u16; 64] = [
    16, 11, 10, 16,  24,  40,  51,  61,
    12, 12, 14, 19,  26,  58,  60,  55,
    14, 13, 16, 24,  40,  57,  69,  56,
    14, 17, 22, 29,  51,  87,  80,  62,
    18, 22, 37, 56,  68, 109, 103,  77,
    24, 35, 55, 64,  81, 104, 113,  92,
    49, 64, 78, 87, 103, 121, 120, 101,
    72, 92, 95, 98, 112, 100, 103,  99,
];

/// Where the k-th value of a DQT segment goes in natural order. A DQT stores
/// its table in zigzag order, so it must be mapped back before it is compared
/// with [`ANNEX_K_LUMA`].
#[rustfmt::skip]
const ZIGZAG_TO_NATURAL: [usize; 64] = [
     0,  1,  8, 16,  9,  2,  3, 10,
    17, 24, 32, 25, 18, 11,  4,  5,
    12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13,  6,  7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46,
    53, 60, 61, 54, 47, 55, 62, 63,
];

/// Estimate the libjpeg quality (1–100) a JPEG was saved at, from its
/// luminance quantisation table. `None` when the file has no readable table
/// 0 before its first scan.
///
/// It is exact for files whose tables are libjpeg's scaled Annex K tables
/// (libjpeg, mozjpeg's default tables, the `image` crate) and an
/// approximation for encoders with their own tables.
pub fn estimate(data: &[u8]) -> Option<u8> {
    let table = luma_table(data)?;
    // libjpeg: scale = q < 50 ? 5000 / q : 200 - 2q, entry = (base * scale + 50) / 100
    // clamped to 1..=255. Invert by choosing the quality whose scaled table
    // is closest to this one.
    (1..=100u8).min_by_key(|&q| {
        let scale = if q < 50 {
            5000 / u32::from(q)
        } else {
            200 - 2 * u32::from(q)
        };
        table
            .iter()
            .zip(ANNEX_K_LUMA)
            .map(|(&got, base)| {
                let want = ((u32::from(base) * scale + 50) / 100).clamp(1, 255);
                want.abs_diff(u32::from(got))
            })
            .sum::<u32>()
    })
}

/// The first table with id 0 (luminance), in natural order.
fn luma_table(data: &[u8]) -> Option<[u16; 64]> {
    if !data.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1; // fill byte
            continue;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            return None; // the first scan or the end, and no table yet
        }
        let len = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        let end = i + 2 + len;
        if len < 2 || end > data.len() {
            return None;
        }
        if marker == 0xDB {
            let mut p = i + 4;
            while p < end {
                let precision = data[p] >> 4;
                let id = data[p] & 0x0F;
                p += 1;
                let n = if precision == 0 { 64 } else { 128 };
                if p + n > end {
                    return None;
                }
                if id == 0 {
                    let mut table = [0u16; 64];
                    for (k, &natural) in ZIGZAG_TO_NATURAL.iter().enumerate() {
                        table[natural] = if precision == 0 {
                            u16::from(data[p + k])
                        } else {
                            u16::from_be_bytes([data[p + 2 * k], data[p + 2 * k + 1]])
                        };
                    }
                    return Some(table);
                }
                p += n;
            }
        }
        i = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimg;

    fn jpeg_at(q: u8) -> Vec<u8> {
        let img = testimg::gradient_rgb(64, 48);
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q)
            .encode_image(&img)
            .unwrap();
        out
    }

    #[test]
    fn recovers_the_quality_the_image_crate_used() {
        for q in 1..=100 {
            let got = estimate(&jpeg_at(q)).unwrap();
            // Near 100 several qualities give the same table (every entry
            // clamps to 1), so only the table, not the number, is recoverable.
            if q >= 98 {
                assert!(got >= 98, "q{q} estimated as {got}");
            } else {
                assert_eq!(got, q, "q{q}");
            }
        }
    }

    #[test]
    fn none_for_what_is_not_a_jpeg_with_tables() {
        assert_eq!(estimate(b""), None);
        assert_eq!(estimate(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(estimate(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
        // Truncated inside the DQT segment.
        let jpeg = jpeg_at(80);
        let dqt = jpeg.windows(2).position(|w| w == [0xFF, 0xDB]).unwrap();
        assert_eq!(estimate(&jpeg[..dqt + 10]), None);
    }
}
