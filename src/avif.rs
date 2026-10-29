//! AVIF decoding: the container with zenavif-parse, the AV1 data with
//! rav1d-safe, and the rest here.
//!
//! zenavif is not used: its 0.1.6 decodes through rav1d-safe 0.5, which hangs
//! with worker threads and returns wrong pixels without an error
//! (imazen/zenavif#78). rav1d-safe 0.6.0 matches dav1d byte for byte.

use image::DynamicImage;
use rav1d_safe::src::managed::{
    ColorPrimaries, ColorRange, Decoder, Frame, MatrixCoefficients, PixelLayout, Planes,
    TransferCharacteristics,
};
use zenavif_parse::{AvifParser, ColorInformation};

use crate::limits::MAX_DIMENSION;
use crate::{Error, Format};

fn malformed(reason: impl std::fmt::Display) -> Error {
    Error::Malformed {
        format: Format::Avif,
        reason: reason.to_string(),
    }
}

fn unsupported(reason: impl Into<String>) -> Error {
    Error::Unsupported {
        format: Format::Avif,
        reason: reason.into(),
    }
}

/// Decode the primary image: tiles stitched, alpha attached, `irot` and
/// `imir` applied. Returns the pixels and the ICC profile they are in.
pub(crate) fn decode(bytes: &[u8]) -> Result<(DynamicImage, Option<Vec<u8>>), Error> {
    let parser = AvifParser::from_bytes(bytes).map_err(malformed)?;
    if parser.clean_aperture().is_some() {
        return Err(unsupported(
            "a clean-aperture crop (clap) is not applied yet",
        ));
    }
    let (color, frame_color) = decode_item(&parser, false)?;
    let colr = parser.color_info();
    let icc = icc_for(colr)?;
    let matrix = match colr {
        Some(ColorInformation::Nclx {
            matrix_coefficients,
            full_range,
            ..
        }) => (*matrix_coefficients, *full_range),
        _ => (
            frame_color.0 as u16,
            matches!(frame_color.1, ColorRange::Full),
        ),
    };
    let mut img = to_rgb(&color, matrix)?;

    if parser.alpha_data().is_some() {
        let (alpha, alpha_color) = decode_item(&parser, true)?;
        let a = alpha_plane(&alpha, matches!(alpha_color.1, ColorRange::Full), &img)?;
        img = attach_alpha(img, &a, parser.premultiplied_alpha());
    }

    if let Some(r) = parser.rotation() {
        img = match r.angle {
            90 => img.rotate270(), // counter-clockwise
            180 => img.rotate180(),
            270 => img.rotate90(),
            _ => img,
        };
    }
    // ISO/IEC 23008-12:2022 §6.5.12: axis 0 exchanges top and bottom, axis 1
    // left and right (libavif, Chromium, Firefox and libheif read it so).
    // zenavif-parse's and zenravif's documentation give the 2017 wording,
    // which is the reverse.
    if let Some(m) = parser.mirror() {
        img = if m.axis == 0 {
            img.flipv()
        } else {
            img.fliph()
        };
    }
    Ok((img, icc))
}

/// The colour (or alpha) item as one frame of planes, grids stitched.
/// Also returns the frame's own matrix and range.
fn decode_item(
    parser: &AvifParser<'_>,
    alpha: bool,
) -> Result<(Decoded, (MatrixCoefficients, ColorRange)), Error> {
    let Some(grid) = parser.grid_config().cloned() else {
        let data = if alpha {
            parser
                .alpha_data()
                .expect("checked by the caller")
                .map_err(malformed)?
        } else {
            parser.primary_data().map_err(malformed)?
        };
        let f = decode_av1(&data)?;
        let ci = f.color_info();
        return Ok((
            Decoded::from_frame(&f),
            (ci.matrix_coefficients, ci.color_range),
        ));
    };
    if alpha {
        return Err(unsupported("alpha on a grid image is not decoded yet"));
    }
    let n = parser.grid_tile_count();
    if n != usize::from(grid.rows) * usize::from(grid.columns) {
        return Err(malformed(
            "the grid's tile count does not match its rows and columns",
        ));
    }
    let mut tiles = Vec::with_capacity(n);
    for i in 0..n {
        let data = parser.tile_data(i).map_err(malformed)?;
        tiles.push(decode_av1(&data)?);
    }
    let ci = tiles[0].color_info();
    let stitched = Decoded::stitch(
        &tiles,
        grid.columns.into(),
        grid.output_width,
        grid.output_height,
    )?;
    Ok((stitched, (ci.matrix_coefficients, ci.color_range)))
}

/// Decode one AV1 still on the caller's thread. With one thread no worker
/// exists, so a panic inside the decoder unwinds to the caller's guard
/// instead of leaving it waiting (the hang in imazen/zenavif#78).
fn decode_av1(data: &[u8]) -> Result<Frame, Error> {
    let mut settings = rav1d_safe::Settings::default();
    settings.threads = 1;
    settings.frame_size_limit = MAX_DIMENSION * MAX_DIMENSION;
    let mut dec = Decoder::with_settings(settings).map_err(malformed)?;
    let mut frame = dec.decode(data).map_err(malformed)?;
    if frame.is_none() {
        frame = dec.flush().map_err(malformed)?.into_iter().next();
    }
    frame.ok_or_else(|| malformed("the AV1 data holds no frame"))
}

/// Planes copied out of a frame, at 16 bits whatever the depth.
struct Decoded {
    width: usize,
    height: usize,
    bit_depth: u8,
    layout: PixelLayout,
    y: Vec<u16>,
    u: Vec<u16>,
    v: Vec<u16>,
}

impl Decoded {
    fn from_frame(f: &Frame) -> Decoded {
        fn copy8(p: &rav1d_safe::src::managed::PlaneView8<'_>) -> Vec<u16> {
            (0..p.height())
                .flat_map(|y| p.row(y)[..p.width()].iter().map(|&v| u16::from(v)))
                .collect()
        }
        fn copy16(p: &rav1d_safe::src::managed::PlaneView16<'_>) -> Vec<u16> {
            (0..p.height())
                .flat_map(|y| p.row(y)[..p.width()].iter().copied())
                .collect()
        }
        let (y, u, v) = match f.planes() {
            Planes::Depth8(p) => (
                copy8(&p.y()),
                p.u().map(|u| copy8(&u)).unwrap_or_default(),
                p.v().map(|v| copy8(&v)).unwrap_or_default(),
            ),
            Planes::Depth16(p) => (
                copy16(&p.y()),
                p.u().map(|u| copy16(&u)).unwrap_or_default(),
                p.v().map(|v| copy16(&v)).unwrap_or_default(),
            ),
        };
        Decoded {
            width: f.width() as usize,
            height: f.height() as usize,
            bit_depth: f.bit_depth(),
            layout: f.pixel_layout(),
            y,
            u,
            v,
        }
    }

    /// The width and height of a chroma plane.
    fn chroma_size(&self) -> (usize, usize) {
        match self.layout {
            PixelLayout::I400 => (0, 0),
            PixelLayout::I420 => (self.width.div_ceil(2), self.height.div_ceil(2)),
            PixelLayout::I422 => (self.width.div_ceil(2), self.height),
            PixelLayout::I444 => (self.width, self.height),
        }
    }

    /// Tiles laid out row by row, cropped to the grid's output size.
    fn stitch(tiles: &[Frame], columns: usize, out_w: u32, out_h: u32) -> Result<Decoded, Error> {
        let parts: Vec<Decoded> = tiles.iter().map(Decoded::from_frame).collect();
        let first = &parts[0];
        if parts.iter().any(|t| {
            (t.width, t.height, t.bit_depth, t.layout)
                != (first.width, first.height, first.bit_depth, first.layout)
        }) {
            return Err(malformed("the grid's tiles differ in size or format"));
        }
        if matches!(first.layout, PixelLayout::I420 | PixelLayout::I422)
            && !first.width.is_multiple_of(2)
            || matches!(first.layout, PixelLayout::I420) && !first.height.is_multiple_of(2)
        {
            return Err(unsupported("grid tiles of odd size with subsampled chroma"));
        }
        let rows = parts.len() / columns;
        let (w, h) = (out_w as usize, out_h as usize);
        if w > first.width * columns || h > first.height * rows || w == 0 || h == 0 {
            return Err(malformed("the grid's output size does not fit its tiles"));
        }
        let mut out = Decoded {
            width: w,
            height: h,
            bit_depth: first.bit_depth,
            layout: first.layout,
            y: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
        };
        let (cw, ch) = out.chroma_size();
        let (tcw, tch) = first.chroma_size();
        let place =
            |plane: fn(&Decoded) -> &Vec<u16>, pw: usize, ph: usize, tw: usize, th: usize| {
                let mut buf = vec![0u16; pw * ph];
                for (i, t) in parts.iter().enumerate() {
                    let (ox, oy) = ((i % columns) * tw, (i / columns) * th);
                    for ty in 0..th {
                        let y = oy + ty;
                        if y >= ph {
                            break;
                        }
                        let n = tw.min(pw.saturating_sub(ox));
                        let src = &plane(t)[ty * tw..ty * tw + n];
                        buf[y * pw + ox..y * pw + ox + n].copy_from_slice(src);
                    }
                }
                buf
            };
        out.y = place(|d| &d.y, w, h, first.width, first.height);
        if cw > 0 {
            out.u = place(|d| &d.u, cw, ch, tcw, tch);
            out.v = place(|d| &d.v, cw, ch, tcw, tch);
        }
        Ok(out)
    }
}

/// The ICC profile the decoded RGB is in: the file's own, or one made from
/// its CICP description when that is not sRGB. HDR transfers are refused:
/// mapping them to SDR is a tone-mapping decision imagoverto does not make.
fn icc_for(colr: Option<&ColorInformation>) -> Result<Option<Vec<u8>>, Error> {
    let (primaries, transfer) = match colr {
        Some(ColorInformation::IccProfile(icc)) => return Ok(Some(icc.clone())),
        Some(ColorInformation::Nclx {
            color_primaries,
            transfer_characteristics,
            ..
        }) => (*color_primaries, *transfer_characteristics),
        None => return Ok(None),
    };
    const PQ: u16 = TransferCharacteristics::SMPTE2084 as u16;
    const HLG: u16 = TransferCharacteristics::HLG as u16;
    if transfer == PQ || transfer == HLG {
        return Err(unsupported("HDR (PQ or HLG) images are not converted"));
    }
    let profile = match primaries {
        p if p == ColorPrimaries::BT709 as u16 || p == ColorPrimaries::Unspecified as u16 => {
            return Ok(None);
        }
        p if p == ColorPrimaries::SMPTE432 as u16 => moxcms::ColorProfile::new_display_p3(),
        p if p == ColorPrimaries::BT2020 as u16 => moxcms::ColorProfile::new_bt2020(),
        p => {
            return Err(unsupported(format!(
                "colour primaries {p} are not converted"
            )));
        }
    };
    profile.encode().map(Some).map_err(|e| Error::Internal {
        format: Format::Avif,
        reason: format!("cannot write an ICC profile: {e}"),
    })
}

/// Kr and Kb for a matrix; `None` for identity (GBR).
fn coefficients(mc: u16) -> Result<Option<(f32, f32)>, Error> {
    Ok(match mc {
        m if m == MatrixCoefficients::Identity as u16 => None,
        m if m == MatrixCoefficients::BT709 as u16 => Some((0.2126, 0.0722)),
        // Unspecified: BT.601, as libavif assumes.
        m if m == MatrixCoefficients::Unspecified as u16
            || m == MatrixCoefficients::BT470BG as u16
            || m == MatrixCoefficients::BT601 as u16 =>
        {
            Some((0.299, 0.114))
        }
        m if m == MatrixCoefficients::BT2020NCL as u16 => Some((0.2627, 0.0593)),
        m => {
            return Err(unsupported(format!(
                "matrix coefficients {m} are not converted"
            )));
        }
    })
}

/// YUV → RGB (or gray), 8-bit for 8-bit sources, 16-bit above.
fn to_rgb(d: &Decoded, (mc, full): (u16, bool)) -> Result<DynamicImage, Error> {
    let (w, h) = (d.width, d.height);
    let max = f32::from((1u16 << d.bit_depth) - 1);
    let scale = f32::from(1u16 << (d.bit_depth - 8));
    // Normalised luma and chroma from code values.
    let luma = |v: u16| -> f32 {
        if full {
            f32::from(v) / max
        } else {
            (f32::from(v) - 16.0 * scale) / (219.0 * scale)
        }
    };
    let chroma = |v: u16| -> f32 {
        if full {
            (f32::from(v) - (max + 1.0) / 2.0) / max
        } else {
            (f32::from(v) - 128.0 * scale) / (224.0 * scale)
        }
    };
    let out8 = d.bit_depth == 8;
    let quantise = |x: f32| -> u16 {
        let x = x.clamp(0.0, 1.0);
        if out8 {
            (x * 255.0).round() as u16
        } else {
            (x * 65535.0).round() as u16
        }
    };

    if d.layout == PixelLayout::I400 {
        let px: Vec<u16> = d.y.iter().map(|&v| quantise(luma(v))).collect();
        return Ok(if out8 {
            DynamicImage::ImageLuma8(buffer(w, h, px.iter().map(|&v| v as u8).collect()))
        } else {
            DynamicImage::ImageLuma16(buffer(w, h, px))
        });
    }

    let k = coefficients(mc)?;
    let (cw, ch) = d.chroma_size();
    let (sx, sy) = (w as f32 / cw as f32, h as f32 / ch as f32);
    // Bilinear chroma, sample centres aligned.
    let sample = |plane: &[u16], x: usize, y: usize| -> f32 {
        if (cw, ch) == (w, h) {
            return f32::from(plane[y * cw + x]);
        }
        let fx = ((x as f32 + 0.5) / sx - 0.5).clamp(0.0, (cw - 1) as f32);
        let fy = ((y as f32 + 0.5) / sy - 0.5).clamp(0.0, (ch - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(ch - 1));
        let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let p = |x, y| f32::from(plane[y * cw + x]);
        let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
        let bottom = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
        top * (1.0 - ay) + bottom * ay
    };

    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let yv = d.y[y * w + x];
            match k {
                // GBR: Y is G, U is B, V is R — exact, no arithmetic.
                None if (cw, ch) == (w, h) && full => {
                    let (u, v) = (d.u[y * cw + x], d.v[y * cw + x]);
                    px.extend([v, yv, u].map(|c| if out8 { c } else { widen(c, d.bit_depth) }));
                }
                None => {
                    return Err(unsupported(
                        "identity matrix with subsampled chroma or limited range",
                    ));
                }
                Some((kr, kb)) => {
                    let kg = 1.0 - kr - kb;
                    let yy = luma(yv);
                    let cb = chroma(sample(&d.u, x, y).round() as u16);
                    let cr = chroma(sample(&d.v, x, y).round() as u16);
                    let r = yy + 2.0 * (1.0 - kr) * cr;
                    let b = yy + 2.0 * (1.0 - kb) * cb;
                    let g = (yy - kr * r - kb * b) / kg;
                    px.extend([r, g, b].map(quantise));
                }
            }
        }
    }
    Ok(if out8 {
        DynamicImage::ImageRgb8(buffer(w, h, px.iter().map(|&v| v as u8).collect()))
    } else {
        DynamicImage::ImageRgb16(buffer(w, h, px))
    })
}

/// A 10- or 12-bit code value at 16 bits, the top bits repeated into the
/// bottom ones so that the maximum maps to 65535.
fn widen(v: u16, bit_depth: u8) -> u16 {
    let shift = 16 - u32::from(bit_depth);
    (v << shift) | (v >> (u32::from(bit_depth) - shift))
}

fn buffer<P: image::Pixel>(
    w: usize,
    h: usize,
    data: Vec<P::Subpixel>,
) -> image::ImageBuffer<P, Vec<P::Subpixel>> {
    image::ImageBuffer::from_raw(w as u32, h as u32, data).expect("sized from the planes")
}

/// The alpha item's luma as alpha values at the colour image's depth.
fn alpha_plane(a: &Decoded, full: bool, color: &DynamicImage) -> Result<Vec<u16>, Error> {
    if (a.width, a.height) != (color.width() as usize, color.height() as usize) {
        return Err(malformed("the alpha plane's size differs from the image's"));
    }
    let max = f32::from((1u16 << a.bit_depth) - 1);
    let scale = f32::from(1u16 << (a.bit_depth - 8));
    let out_max = if color.color().bytes_per_pixel() / color.color().channel_count() == 2 {
        65535.0
    } else {
        255.0
    };
    Ok(a.y
        .iter()
        .map(|&v| {
            let x = if full {
                f32::from(v) / max
            } else {
                (f32::from(v) - 16.0 * scale) / (219.0 * scale)
            };
            (x.clamp(0.0, 1.0) * out_max).round() as u16
        })
        .collect())
}

/// Add the alpha channel; undo premultiplication when the file says the
/// colour was multiplied by it.
fn attach_alpha(img: DynamicImage, alpha: &[u16], premultiplied: bool) -> DynamicImage {
    let unpremul = |c: u16, a: u16, max: u32| -> u16 {
        if !premultiplied || a == 0 {
            return if a == 0 && premultiplied { 0 } else { c };
        }
        ((u32::from(c) * max + u32::from(a) / 2) / u32::from(a)).min(max) as u16
    };
    match img {
        DynamicImage::ImageLuma8(g) => {
            let (w, h) = g.dimensions();
            let px = g
                .pixels()
                .zip(alpha)
                .flat_map(|(p, &a)| [unpremul(p[0].into(), a, 255) as u8, a as u8]);
            DynamicImage::ImageLumaA8(
                image::ImageBuffer::from_raw(w, h, px.collect()).expect("sized"),
            )
        }
        DynamicImage::ImageLuma16(g) => {
            let (w, h) = g.dimensions();
            let px = g
                .pixels()
                .zip(alpha)
                .flat_map(|(p, &a)| [unpremul(p[0], a, 65535), a]);
            DynamicImage::ImageLumaA16(
                image::ImageBuffer::from_raw(w, h, px.collect()).expect("sized"),
            )
        }
        DynamicImage::ImageRgb16(rgb) => {
            let (w, h) = rgb.dimensions();
            let px = rgb.pixels().zip(alpha).flat_map(|(p, &a)| {
                [
                    unpremul(p[0], a, 65535),
                    unpremul(p[1], a, 65535),
                    unpremul(p[2], a, 65535),
                    a,
                ]
            });
            DynamicImage::ImageRgba16(
                image::ImageBuffer::from_raw(w, h, px.collect()).expect("sized"),
            )
        }
        other => {
            let rgb = other.to_rgb8();
            let (w, h) = rgb.dimensions();
            let px = rgb.pixels().zip(alpha).flat_map(|(p, &a)| {
                let c = |i: usize| unpremul(p[i].into(), a, 255) as u8;
                [c(0), c(1), c(2), a as u8]
            });
            DynamicImage::ImageRgba8(
                image::ImageBuffer::from_raw(w, h, px.collect()).expect("sized"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimg::{self, psnr};
    use rgb::FromSlice;
    use zenravif::{AlphaColorMode, BitDepth, ChromaSubsampling, Encoder};

    fn encode_rgb(img: &image::RgbImage, enc: Encoder<'_>) -> Vec<u8> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        enc.encode_rgb(imgref::Img::new(img.as_raw().as_rgb(), w, h))
            .unwrap()
            .avif_file
    }

    fn encode_rgba(img: &image::RgbaImage, enc: Encoder<'_>) -> Vec<u8> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        enc.encode_rgba(imgref::Img::new(img.as_raw().as_rgba(), w, h))
            .unwrap()
            .avif_file
    }

    fn lossy(q: f32) -> Encoder<'static> {
        Encoder::new()
            .with_quality(q)
            .with_speed(10)
            .with_bit_depth(BitDepth::Eight)
    }

    /// High quality, 4:4:4: as close to the source as zenravif gets (it has
    /// no working lossless mode).
    fn fine() -> Encoder<'static> {
        lossy(97.0)
    }

    #[test]
    fn rgb_and_rgba_decode_close_to_the_source() {
        let rgb = testimg::gradient_rgb(41, 23);
        let (img, icc) = decode(&encode_rgb(&rgb, fine())).unwrap();
        assert_eq!(img.color(), image::ColorType::Rgb8);
        assert!(psnr(&img, &DynamicImage::ImageRgb8(rgb)) > 40.0);
        assert!(icc.is_none());
        let rgba = testimg::gradient_rgba(41, 23);
        let dirty = fine().with_alpha_color_mode(AlphaColorMode::UnassociatedDirty);
        let (img, _) = decode(&encode_rgba(&rgba, dirty)).unwrap();
        let out = img.to_rgba8();
        for (p, q) in out.pixels().zip(rgba.pixels()) {
            assert!(p[3].abs_diff(q[3]) <= 2, "alpha {} for {}", p[3], q[3]);
        }
    }

    #[test]
    fn lossy_444_and_420_are_close_to_the_source() {
        let src = testimg::gradient_rgb(64, 48);
        for sub in [ChromaSubsampling::Yuv444, ChromaSubsampling::Yuv420] {
            let (img, _) =
                decode(&encode_rgb(&src, lossy(80.0).with_chroma_subsampling(sub))).unwrap();
            let db = psnr(&img, &DynamicImage::ImageRgb8(src.clone()));
            assert!(db > 38.0, "{sub:?}: {db:.1} dB");
        }
    }

    /// The gradient of imazen/zenavif#78: zenavif 0.1.6 decodes it at 14.4
    /// dB against the source, ffmpeg/libdav1d at 48.6 dB.
    #[test]
    fn issue_78_gradient_decodes_correctly() {
        let src = testimg::gradient_rgb(582, 55);
        let avif = encode_rgb(&src, Encoder::new().with_quality(71.3).with_speed(4));
        let (img, _) = decode(&avif).unwrap();
        let db = psnr(&img, &DynamicImage::ImageRgb8(src));
        assert!(db > 40.0, "{db:.1} dB");
    }

    /// The gray noise of imazen/zenavif#78, which hung zenavif 0.1.6's
    /// threaded decode every time. One thread here, and a deadline.
    #[test]
    fn issue_78_gray_noise_does_not_hang() {
        let gray = image::DynamicImage::ImageLuma8(testimg::gray_noise(787, 503)).to_rgb8();
        let avif = encode_rgb(&gray, Encoder::new().with_quality(71.3).with_speed(10));
        // wasm has no threads to watch from; there the test only decodes.
        #[cfg(target_family = "wasm")]
        let got = decode(&avif).map(|(i, _)| (i.width(), i.height()));
        #[cfg(not(target_family = "wasm"))]
        let got = {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                tx.send(decode(&avif).map(|(i, _)| (i.width(), i.height())))
            });
            rx.recv_timeout(std::time::Duration::from_secs(60))
                .expect("decode hung")
        };
        assert_eq!(got.unwrap(), (787, 503));
    }

    #[test]
    fn ten_bits_become_sixteen() {
        let src = testimg::gradient_rgb(32, 24);
        let avif = encode_rgb(
            &src,
            Encoder::new()
                .with_quality(90.0)
                .with_speed(10)
                .with_bit_depth(BitDepth::Ten),
        );
        let (img, _) = decode(&avif).unwrap();
        assert_eq!(img.color(), image::ColorType::Rgb16);
        assert!(psnr(&img, &DynamicImage::ImageRgb8(src)) > 38.0);
        assert_eq!(widen(1023, 10), 65535);
        assert_eq!(widen(0, 10), 0);
        assert_eq!(widen(4095, 12), 65535);
    }

    #[test]
    fn rotation_and_mirror_are_applied() {
        let src = DynamicImage::ImageRgb8(testimg::gradient_rgb(32, 16));
        let rgb = src.to_rgb8();
        // zenravif's `with_rotation` takes the irot code (1 = 90° CCW), not
        // degrees as its documentation says: it masks the value with 3.
        let cases = [
            (fine().with_rotation(1), src.rotate270()),
            (fine().with_rotation(2), src.rotate180()),
            (fine().with_rotation(3), src.rotate90()),
            // zenravif writes the axis number it is given; read per HEIF 2022.
            (fine().with_mirror(0), src.flipv()),
            (fine().with_mirror(1), src.fliph()),
        ];
        for (i, (enc, want)) in cases.into_iter().enumerate() {
            let (img, _) = decode(&encode_rgb(&rgb, enc)).unwrap();
            assert_eq!(
                (img.width(), img.height()),
                (want.width(), want.height()),
                "case {i}"
            );
            let db = psnr(&img, &want);
            assert!(db > 35.0, "case {i}: {db:.1} dB");
        }
        // The flips are far from each other, so the PSNR above tells them apart.
        assert!(psnr(&src.fliph(), &src.flipv()) < 20.0);
    }

    /// zenravif's premultiplied mode cannot make a test file: it writes alpha
    /// 255 as about 1 (dav1d decodes the same), so the undo is tested on
    /// planes built by hand.
    #[test]
    fn premultiplied_alpha_is_undone() {
        let premul =
            image::RgbImage::from_raw(4, 1, vec![100, 50, 0, 50, 25, 0, 0, 0, 0, 255, 255, 255])
                .unwrap();
        let alpha = [255u16, 128, 0, 255];
        let out = attach_alpha(DynamicImage::ImageRgb8(premul), &alpha, true).to_rgba8();
        assert_eq!(out.get_pixel(0, 0).0, [100, 50, 0, 255]);
        assert_eq!(out.get_pixel(1, 0).0, [100, 50, 0, 128]);
        assert_eq!(out.get_pixel(2, 0).0, [0, 0, 0, 0]);
        assert_eq!(out.get_pixel(3, 0).0, [255, 255, 255, 255]);
        let straight = image::RgbImage::from_raw(1, 1, vec![10, 20, 30]).unwrap();
        let out = attach_alpha(DynamicImage::ImageRgb8(straight), &[7], false).to_rgba8();
        assert_eq!(out.get_pixel(0, 0).0, [10, 20, 30, 7]);
    }

    /// Our planes against dav1d's, byte for byte, when dav1d is installed.
    #[test]
    fn planes_match_dav1d() {
        if cfg!(target_family = "wasm") {
            return; // no processes
        }
        let src = testimg::gradient_rgb(582, 55);
        let dir = std::env::temp_dir().join(format!("imagoverto-avif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (i, sub) in [ChromaSubsampling::Yuv420, ChromaSubsampling::Yuv444]
            .into_iter()
            .enumerate()
        {
            let avif = encode_rgb(
                &src,
                Encoder::new()
                    .with_quality(71.3)
                    .with_speed(6)
                    .with_chroma_subsampling(sub),
            );
            let parser = AvifParser::from_bytes(&avif).unwrap();
            let obu = parser.primary_data().unwrap();
            let ours = Decoded::from_frame(&decode_av1(&obu).unwrap());
            let (obu_path, yuv) = (dir.join(format!("{i}.obu")), dir.join(format!("{i}.yuv")));
            std::fs::write(&obu_path, &*obu).unwrap();
            let ran = std::process::Command::new("dav1d")
                .args(["--demuxer", "section5", "-i"])
                .arg(&obu_path)
                .arg("-o")
                .arg(&yuv)
                .output();
            let Ok(out) = ran else {
                eprintln!("dav1d not available: planes not compared");
                return;
            };
            assert!(out.status.success());
            let theirs = std::fs::read(&yuv).unwrap();
            // dav1d writes samples above 8 bits as 16-bit little-endian.
            let mine: Vec<u8> = ours
                .y
                .iter()
                .chain(&ours.u)
                .chain(&ours.v)
                .flat_map(|&v| {
                    if ours.bit_depth == 8 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            assert_eq!(mine.len(), theirs.len(), "{sub:?}");
            assert!(mine == theirs, "{sub:?}: planes differ from dav1d");
        }
    }

    /// A reference AVIF from libavif's `avifenc`, made from `src` with `args`;
    /// `None` when avifenc is not installed.
    fn avifenc(src: &DynamicImage, args: &[&str], name: &str) -> Option<Vec<u8>> {
        if cfg!(target_family = "wasm") {
            return None; // no processes
        }
        let dir = std::env::temp_dir().join(format!("imagoverto-avifenc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (png, avif) = (
            dir.join(format!("{name}.png")),
            dir.join(format!("{name}.avif")),
        );
        src.save(&png).unwrap();
        let out = std::process::Command::new("avifenc")
            .args(args)
            .arg(&png)
            .arg(&avif)
            .output()
            .ok()?;
        assert!(
            out.status.success(),
            "avifenc {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Some(std::fs::read(&avif).unwrap())
    }

    macro_rules! reference {
        ($src:expr, $args:expr, $name:expr) => {
            match avifenc($src, $args, $name) {
                Some(b) => b,
                None => {
                    eprintln!("avifenc not available: libavif reference not checked");
                    return;
                }
            }
        };
    }

    #[test]
    fn libavif_lossless_is_exact() {
        let rgb = DynamicImage::ImageRgb8(testimg::gradient_rgb(45, 31));
        let noise = DynamicImage::ImageLuma8(testimg::gray_noise(45, 31)).to_rgb8();
        for (name, src) in [("rgb", rgb), ("noise", DynamicImage::ImageRgb8(noise))] {
            let avif = reference!(&src, &["-l", "-s", "8"], name);
            let (img, icc) = decode(&avif).unwrap();
            assert_eq!(img, src, "{name}");
            assert!(icc.is_none());
        }
        let rgba = DynamicImage::ImageRgba8(testimg::gradient_rgba(45, 31));
        let avif = reference!(&rgba, &["-l", "-s", "8"], "rgba");
        assert_eq!(decode(&avif).unwrap().0, rgba);
    }

    #[test]
    fn libavif_monochrome_and_grid() {
        let gray = DynamicImage::ImageLuma8(testimg::gray_noise(40, 24));
        let avif = reference!(&gray, &["-l", "-y", "400", "-s", "8"], "mono");
        let (img, _) = decode(&avif).unwrap();
        assert_eq!(img, gray);
        // A 2x2 grid of one image's four quarters.
        // avifenc wants at least 64 pixels per cell.
        let rgb = DynamicImage::ImageRgb8(testimg::gradient_rgb(160, 128));
        let avif = reference!(&rgb, &["-l", "-g", "2x2", "-s", "8"], "grid");
        assert!(
            AvifParser::from_bytes(&avif)
                .unwrap()
                .grid_config()
                .is_some()
        );
        assert_eq!(decode(&avif).unwrap().0, rgb);
    }

    #[test]
    fn libavif_orientation_matches() {
        let src = DynamicImage::ImageRgb8(testimg::gradient_rgb(32, 16));
        let cases: [(&[&str], DynamicImage); 4] = [
            (&["-l", "--irot", "1"], src.rotate270()),
            (&["-l", "--irot", "3"], src.rotate90()),
            (&["-l", "--imir", "0"], src.flipv()),
            (&["-l", "--imir", "1"], src.fliph()),
        ];
        for (i, (args, want)) in cases.into_iter().enumerate() {
            let avif = reference!(&src, args, &format!("orient{i}"));
            assert_eq!(decode(&avif).unwrap().0, want, "{args:?}");
        }
    }

    #[test]
    fn libavif_lossy_variants_are_close() {
        let src = DynamicImage::ImageRgb8(testimg::gradient_rgb(64, 48));
        let cases: [(&[&str], &str, f64); 5] = [
            (&["-q", "90", "-y", "420"], "420", 38.0),
            (&["-q", "90", "-y", "422"], "422", 38.0),
            (&["-q", "90", "-r", "limited"], "limited", 38.0),
            (&["-q", "90", "-d", "10"], "10bit", 38.0),
            (&["-q", "90", "-d", "12", "-y", "444"], "12bit", 38.0),
        ];
        for (args, name, min) in cases {
            let avif = reference!(&src, args, name);
            let (img, _) = decode(&avif).unwrap();
            let db = psnr(&img, &src);
            assert!(db > min, "{name}: {db:.1} dB");
        }
        // Premultiplied alpha, undone.
        let rgba = DynamicImage::ImageRgba8(testimg::gradient_rgba(64, 48));
        let avif = reference!(&rgba, &["-q", "95", "--qalpha", "100", "-p"], "prem");
        assert!(AvifParser::from_bytes(&avif).unwrap().premultiplied_alpha());
        let out = decode(&avif).unwrap().0.to_rgba8();
        let src = rgba.to_rgba8();
        let worst = out
            .pixels()
            .zip(src.pixels())
            .filter(|(_, q)| q[3] >= 64)
            .flat_map(|(p, q)| (0..3).map(move |c| p[c].abs_diff(q[c])))
            .max()
            .unwrap();
        assert!(worst <= 12, "premultiplied colour off by {worst}");
    }

    #[test]
    fn libavif_display_p3_gets_a_profile() {
        let src = DynamicImage::ImageRgb8(testimg::gradient_rgb(16, 16));
        let avif = reference!(&src, &["-q", "90", "--cicp", "12/13/1"], "p3");
        let (_, icc) = decode(&avif).unwrap();
        let icc = icc.expect("a Display P3 profile");
        assert_eq!(&icc[36..40], b"acsp");
        let hdr = reference!(&src, &["-q", "90", "--cicp", "9/16/9", "-d", "10"], "pq");
        assert!(matches!(decode(&hdr), Err(Error::Unsupported { .. })));
    }
}
