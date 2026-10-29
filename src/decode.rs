//! Decoding any of the formats into pixels, under the always-on rules.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use image::{DynamicImage, ExtendedColorType, ImageDecoder};

use crate::limits::{MAX_DECODED_BYTES, MAX_DIMENSION};
use crate::{Error, Format, Info, inspect};

/// A decoded image, upright, with what must travel with its pixels.
#[derive(Debug, Clone)]
pub(crate) struct Decoded {
    /// The pixels, with EXIF orientation already applied. Gray images are
    /// `Luma`/`LumaA`; 16-bit sources stay 16-bit.
    pub image: DynamicImage,
    /// The ICC profile the pixels are in, if the file carried one.
    pub icc: Option<Vec<u8>>,
    /// A lossy WebP whose chroma lies within codec noise of neutral: gray to
    /// the eye, though not exactly R = G = B. Lossy WebP has no grayscale
    /// mode, so a gray source comes back slightly tinted. Whether to treat it
    /// as gray is the encoder's call: doing so changes pixel values, which a
    /// lossless conversion must not.
    pub near_gray: bool,
    /// The header facts, read before decoding.
    pub info: Info,
}

/// Decode `bytes`. Refuses animation and images past the limits, before any
/// pixel is allocated.
pub(crate) fn decode(bytes: &[u8]) -> Result<Decoded, Error> {
    let info = inspect(bytes)?;
    let format = info.format;
    if info.animated {
        return Err(Error::Animated { format });
    }
    if !info.within_limits() {
        return Err(Error::TooLarge {
            width: info.width,
            height: info.height,
        });
    }
    let (image, icc, near_gray) = guard(format, || match format {
        Format::Jpeg => decode_jpeg(bytes),
        Format::Png => decode_png(bytes),
        Format::WebP => decode_webp(bytes, &info),
        Format::Jxl => decode_jxl(bytes),
        Format::Avif => crate::avif::decode(bytes).map(|(img, icc)| (img, icc, false)),
    })?;
    // A colour profile describes RGB; collapsing to gray would orphan it.
    let image = if icc.is_none() {
        collapse_exact_gray(image)
    } else {
        image
    };
    Ok(Decoded {
        image,
        icc,
        near_gray,
        info,
    })
}

/// Decode `bytes` fully, under the same rules as a conversion, and return
/// the header facts: the answer to "does this image actually decode?". The
/// pixels are discarded.
pub fn decode_check(bytes: &[u8]) -> Result<Info, Error> {
    decode(bytes).map(|d| d.info)
}

type Pixels = (DynamicImage, Option<Vec<u8>>, bool);

/// Run a codec so that a panic inside it comes back as an error, never as a
/// crash of the caller.
fn guard(format: Format, f: impl FnOnce() -> Result<Pixels, Error>) -> Result<Pixels, Error> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        Err(Error::Internal {
            format,
            reason: "the decoder panicked".into(),
        })
    })
}

fn image_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    limits
}

/// JPEG and PNG through the `image` crate's decoders.
fn decode_with_image(format: Format, mut d: impl ImageDecoder) -> Result<Pixels, Error> {
    let malformed = |e: image::ImageError| Error::Malformed {
        format,
        reason: e.to_string(),
    };
    if matches!(d.original_color_type(), ExtendedColorType::Cmyk8) {
        // `image` turns CMYK into RGB without the profile, so the colours
        // would shift. Refused rather than converted wrongly.
        return Err(Error::Unsupported {
            format,
            reason: "CMYK images are not converted".into(),
        });
    }
    d.set_limits(image_limits()).map_err(malformed)?;
    let icc = d.icc_profile().map_err(malformed)?;
    let orientation = d.orientation().map_err(malformed)?;
    let mut img = DynamicImage::from_decoder(d).map_err(malformed)?;
    img.apply_orientation(orientation);
    Ok((img, icc, false))
}

fn decode_jpeg(bytes: &[u8]) -> Result<Pixels, Error> {
    let d = image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes)).map_err(|e| {
        Error::Malformed {
            format: Format::Jpeg,
            reason: e.to_string(),
        }
    })?;
    decode_with_image(Format::Jpeg, d)
}

fn decode_png(bytes: &[u8]) -> Result<Pixels, Error> {
    let d =
        image::codecs::png::PngDecoder::new(Cursor::new(bytes)).map_err(|e| Error::Malformed {
            format: Format::Png,
            reason: e.to_string(),
        })?;
    decode_with_image(Format::Png, d)
}

fn decode_webp(bytes: &[u8], info: &Info) -> Result<Pixels, Error> {
    let malformed = |e: &dyn std::fmt::Display| Error::Malformed {
        format: Format::WebP,
        reason: e.to_string(),
    };
    let meta = zenwebp::ImageInfo::from_webp(bytes).map_err(|e| malformed(&e))?;
    let short = || Error::Internal {
        format: Format::WebP,
        reason: "the decoder returned a short buffer".into(),
    };
    let mut img = if info.has_alpha {
        let (px, w, h) = zenwebp::oneshot::decode_rgba(bytes).map_err(|e| malformed(&e))?;
        DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, px).ok_or_else(short)?)
    } else {
        let (px, w, h) = zenwebp::oneshot::decode_rgb(bytes).map_err(|e| malformed(&e))?;
        DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, px).ok_or_else(short)?)
    };
    if let Some(o) = meta
        .orientation
        .and_then(|o| image::metadata::Orientation::from_exif(o as u8))
    {
        img.apply_orientation(o);
    }
    let near_gray = !info.has_alpha && info.lossless == Some(false) && lossy_webp_is_gray(bytes);
    Ok((img, meta.icc_profile, near_gray))
}

/// Whether a lossy WebP's chroma planes are within codec noise of neutral.
/// zenwebp, encoding a gray source lossily, leaves U and V up to 4 away from
/// 128 (measured by epublift at quality 30, 50, 80 and 95), so ±4 is that
/// noise; a tinted scan sits well outside it.
fn lossy_webp_is_gray(bytes: &[u8]) -> bool {
    const NEUTRAL: i16 = 128;
    const NOISE: i16 = 4;
    zenwebp::oneshot::decode_yuv420(bytes).is_ok_and(|p| {
        p.u.iter()
            .chain(&p.v)
            .all(|&c| (i16::from(c) - NEUTRAL).abs() <= NOISE)
    })
}

/// JPEG XL. The decoder applies the orientation itself.
fn decode_jxl(bytes: &[u8]) -> Result<Pixels, Error> {
    use zenpixels::{ChannelLayout, ChannelType};
    let limits = zenjxl::JxlLimits {
        max_pixels: Some(u64::from(MAX_DIMENSION) * u64::from(MAX_DIMENSION)),
        max_memory_bytes: Some(MAX_DECODED_BYTES),
    };
    let out = zenjxl::decode(bytes, Some(&limits), &[]).map_err(|e| Error::Malformed {
        format: Format::Jxl,
        reason: e.to_string(),
    })?;
    let pb = &out.pixels;
    let desc = pb.descriptor();
    let (w, h) = (pb.width(), pb.height());
    let raw = pb.copy_to_contiguous_bytes();
    let short = || Error::Internal {
        format: Format::Jxl,
        reason: "the decoder returned a short buffer".into(),
    };
    let img = match (desc.channel_type(), desc.layout()) {
        (ChannelType::U8, layout) => match layout {
            ChannelLayout::Gray => {
                DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, raw).ok_or_else(short)?)
            }
            ChannelLayout::GrayAlpha => DynamicImage::ImageLumaA8(
                image::GrayAlphaImage::from_raw(w, h, raw).ok_or_else(short)?,
            ),
            ChannelLayout::Rgb => {
                DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, raw).ok_or_else(short)?)
            }
            ChannelLayout::Rgba => {
                DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, raw).ok_or_else(short)?)
            }
            other => return Err(unsupported_layout(format!("{other:?}"))),
        },
        (ChannelType::U16, layout) => {
            let px: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_ne_bytes(c))
                .collect();
            match layout {
                ChannelLayout::Gray => DynamicImage::ImageLuma16(
                    image::ImageBuffer::from_raw(w, h, px).ok_or_else(short)?,
                ),
                ChannelLayout::GrayAlpha => DynamicImage::ImageLumaA16(
                    image::ImageBuffer::from_raw(w, h, px).ok_or_else(short)?,
                ),
                ChannelLayout::Rgb => DynamicImage::ImageRgb16(
                    image::ImageBuffer::from_raw(w, h, px).ok_or_else(short)?,
                ),
                ChannelLayout::Rgba => DynamicImage::ImageRgba16(
                    image::ImageBuffer::from_raw(w, h, px).ok_or_else(short)?,
                ),
                other => return Err(unsupported_layout(format!("{other:?}"))),
            }
        }
        (t, l) => return Err(unsupported_layout(format!("{t:?} {l:?}"))),
    };
    // The decoder may hand a gray image back as RGB; the header says it is gray.
    let img = if out.info.is_gray {
        collapse_exact_gray(img)
    } else {
        img
    };
    Ok((img, out.info.icc_profile.clone(), false))
}

fn unsupported_layout(what: String) -> Error {
    Error::Unsupported {
        format: Format::Jxl,
        reason: format!("decoded pixel format {what}"),
    }
}

/// An RGB(A) image whose every pixel has R = G = B, as gray. Exact, so no
/// value changes: the encoder that follows just stops spending bits on
/// empty colour channels.
fn collapse_exact_gray(img: DynamicImage) -> DynamicImage {
    fn gray<P: Copy + PartialEq>(px: &[P], channels: usize) -> bool {
        px.chunks_exact(channels)
            .all(|p| p[0] == p[1] && p[1] == p[2])
    }
    fn keep<P: Copy>(px: &[P], channels: usize, alpha: bool) -> Vec<P> {
        px.chunks_exact(channels)
            .flat_map(|p| {
                let a = if alpha { Some(p[3]) } else { None };
                std::iter::once(p[0]).chain(a)
            })
            .collect()
    }
    let (w, h) = (img.width(), img.height());
    match &img {
        DynamicImage::ImageRgb8(i) if gray(i.as_raw(), 3) => DynamicImage::ImageLuma8(
            image::ImageBuffer::from_raw(w, h, keep(i.as_raw(), 3, false)).unwrap(),
        ),
        DynamicImage::ImageRgba8(i) if gray(i.as_raw(), 4) => DynamicImage::ImageLumaA8(
            image::ImageBuffer::from_raw(w, h, keep(i.as_raw(), 4, true)).unwrap(),
        ),
        DynamicImage::ImageRgb16(i) if gray(i.as_raw(), 3) => DynamicImage::ImageLuma16(
            image::ImageBuffer::from_raw(w, h, keep(i.as_raw(), 3, false)).unwrap(),
        ),
        DynamicImage::ImageRgba16(i) if gray(i.as_raw(), 4) => DynamicImage::ImageLumaA16(
            image::ImageBuffer::from_raw(w, h, keep(i.as_raw(), 4, true)).unwrap(),
        ),
        _ => img,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimg;
    use image::ColorType;

    fn png(img: &DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn webp_lossy(rgb: &image::RgbImage, q: f32) -> Vec<u8> {
        let cfg = zenwebp::LossyConfig::new().with_quality(q);
        zenwebp::EncodeRequest::lossy(
            &cfg,
            rgb,
            zenwebp::PixelLayout::Rgb8,
            rgb.width(),
            rgb.height(),
        )
        .encode()
        .unwrap()
    }

    #[test]
    fn png_round_trips_exactly_and_keeps_depth() {
        let rgba = DynamicImage::ImageRgba8(testimg::gradient_rgba(31, 17));
        let d = decode(&png(&rgba)).unwrap();
        assert_eq!(d.image, rgba);
        assert!(d.icc.is_none() && !d.near_gray);

        let rgb16 = DynamicImage::ImageRgb16(
            DynamicImage::ImageRgb8(testimg::gradient_rgb(8, 8)).to_rgb16(),
        );
        assert_eq!(
            decode(&png(&rgb16)).unwrap().image.color(),
            ColorType::Rgb16
        );
    }

    #[test]
    fn gray_stays_gray() {
        let gray = testimg::gray_noise(16, 9);
        // Stored as gray.
        let d = decode(&png(&DynamicImage::ImageLuma8(gray.clone()))).unwrap();
        assert_eq!(d.image, DynamicImage::ImageLuma8(gray.clone()));
        // Stored as RGB with R = G = B: collapsed, values unchanged.
        let as_rgb = DynamicImage::ImageLuma8(gray.clone()).to_rgb8();
        let d = decode(&png(&DynamicImage::ImageRgb8(as_rgb))).unwrap();
        assert_eq!(d.image, DynamicImage::ImageLuma8(gray));
        // Not gray: one pixel off.
        let mut tinted = testimg::gradient_rgb(4, 4);
        for p in tinted.pixels_mut() {
            p.0 = [p.0[0], p.0[0], p.0[0]];
        }
        tinted.get_pixel_mut(1, 1).0[2] ^= 1;
        let d = decode(&png(&DynamicImage::ImageRgb8(tinted))).unwrap();
        assert_eq!(d.image.color(), ColorType::Rgb8);
    }

    #[test]
    fn lossy_webp_of_a_gray_source_is_near_gray() {
        let gray = DynamicImage::ImageLuma8(testimg::gray_noise(32, 32)).to_rgb8();
        for q in [30.0, 80.0, 95.0] {
            let d = decode(&webp_lossy(&gray, q)).unwrap();
            assert!(d.near_gray, "q{q}");
        }
        let d = decode(&webp_lossy(&testimg::gradient_rgb(32, 32), 80.0)).unwrap();
        assert!(!d.near_gray);
    }

    #[test]
    fn webp_lossless_round_trips_exactly() {
        let rgba = testimg::gradient_rgba(19, 23);
        // `exact`: by default the encoder zeroes RGB under fully transparent
        // pixels, which is invisible but not lossless.
        let cfg = zenwebp::LosslessConfig::new().with_exact(true);
        let webp =
            zenwebp::EncodeRequest::lossless(&cfg, &rgba, zenwebp::PixelLayout::Rgba8, 19, 23)
                .encode()
                .unwrap();
        assert_eq!(decode(&webp).unwrap().image, DynamicImage::ImageRgba8(rgba));
    }

    #[test]
    fn jxl_lossless_round_trips_exactly() {
        let rgb = testimg::gradient_rgb(27, 11);
        let jxl = zenjxl::LosslessConfig::new()
            .encode(&rgb, 27, 11, zenjxl::PixelLayout::Rgb8)
            .unwrap();
        assert_eq!(decode(&jxl).unwrap().image, DynamicImage::ImageRgb8(rgb));
    }

    #[test]
    fn jpeg_orientation_is_applied() {
        // A 2x1 JPEG with EXIF orientation 6 (rotate 90° clockwise) decodes 1x2.
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode_image(&testimg::gradient_rgb(16, 8))
            .unwrap();
        let exif: &[u8] =
            b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
        let mut app1 = vec![0xFF, 0xE1];
        app1.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        app1.extend_from_slice(exif);
        jpeg.splice(2..2, app1);
        let d = decode(&jpeg).unwrap();
        assert_eq!((d.image.width(), d.image.height()), (8, 16));
    }

    #[test]
    fn refusals() {
        // Past the limits, refused from the header.
        let mut big = png(&DynamicImage::ImageLuma8(testimg::gray_noise(1, 1)));
        big[16..20].copy_from_slice(&20_000u32.to_be_bytes());
        assert!(matches!(
            decode(&big),
            Err(Error::TooLarge { .. }) | Err(Error::Malformed { .. })
        ));
        // Garbage after a valid signature.
        let mut bad = png(&DynamicImage::ImageLuma8(testimg::gray_noise(8, 8)));
        let n = bad.len();
        bad.truncate(n / 2);
        assert!(decode(&bad).is_err());
    }

    #[test]
    fn animated_webp_is_refused() {
        let frame = testimg::gradient_rgba(8, 8);
        let mut anim = zenwebp::mux::AnimationEncoder::new(8, 8, Default::default()).unwrap();
        let cfg = zenwebp::EncoderConfig::new_lossless();
        anim.add_frame(frame.as_raw(), zenwebp::PixelLayout::Rgba8, 0, &cfg)
            .unwrap();
        anim.add_frame(frame.as_raw(), zenwebp::PixelLayout::Rgba8, 100, &cfg)
            .unwrap();
        let webp = anim.finalize(100).unwrap();
        assert_eq!(
            decode(&webp).unwrap_err(),
            Error::Animated {
                format: Format::WebP
            }
        );
    }
}
