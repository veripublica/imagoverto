//! Encoding pixels into each format, lossless or at a quality.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use image::{ColorType, DynamicImage, ImageEncoder};

use crate::{Cancel, Compression, Error, Format};

/// How to encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Lossless,
    /// Quality 1–100 on the WebP scale.
    Lossy(u8),
}

// Each encoder's setting per compression level, from a measurement of every
// setting's size and time on 60 photos and 60 drawings from Baris's shelf
// (2026-10-30; lossy compared at equal butteraugli distance). Where a higher
// setting bought nothing, a level keeps the lower one: JPEG XL lossy effort 5
// is as small as 7 and 2.4x quicker, WebP method 6 no smaller than 4.

/// Encode `img` to `format`. The caller has already settled what the format
/// cannot hold: no transparency for JPEG, no 16-bit for a lossless target that
/// stores 8 bits, no AVIF target. `img` carries no fully-opaque alpha
/// channel. WebP stops early, with [`Error::Cancelled`], once `cancel` stops
/// (JPEG XL is handed it too, but jxl-encoder 0.3.4 does not check it yet).
pub(crate) fn encode(
    img: &DynamicImage,
    icc: Option<&[u8]>,
    format: Format,
    mode: Mode,
    level: Compression,
    cancel: Option<&Cancel>,
) -> Result<Vec<u8>, Error> {
    let stop: &dyn enough::Stop = match cancel {
        Some(c) => c,
        None => &enough::Unstoppable,
    };
    let run = || match format {
        Format::Png => encode_png(img, icc, level),
        Format::Jpeg => match mode {
            Mode::Lossy(q) => encode_jpeg(img, icc, q),
            Mode::Lossless => unreachable!("JPEG has no lossless mode; refused before encoding"),
        },
        Format::WebP => encode_webp(img, icc, mode, level, stop),
        Format::Jxl => encode_jxl(img, icc, mode, level, stop),
        Format::Avif => unreachable!("no AVIF encoder in this release; refused before encoding"),
    };
    let res = catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|_| {
        Err(Error::Internal {
            format,
            reason: "the encoder panicked".into(),
        })
    });
    // Each codec reports a stop in its own error type; the token says why.
    match res {
        Err(_) if cancel.is_some_and(Cancel::is_cancelled) => Err(Error::Cancelled),
        res => res,
    }
}

/// JPEG → JPEG XL that keeps the JPEG itself: the JXL holds the original's
/// DCT coefficients and a reconstruction box, so a decoder can return the
/// original file byte for byte.
pub(crate) fn recompress_jpeg_to_jxl(jpeg: &[u8]) -> Result<Vec<u8>, Error> {
    let run = || {
        let fail = |e: &dyn std::fmt::Display| Error::Unsupported {
            format: Format::Jpeg,
            reason: format!("cannot be recompressed to JPEG XL: {e}"),
        };
        let data = jxl_encoder::jpeg::read_jpeg(jpeg).map_err(|e| fail(&e))?;
        jxl_encoder::jpeg::encode_jpeg_to_jxl_container(&data).map_err(|e| fail(&e))
    };
    catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|_| {
        Err(Error::Internal {
            format: Format::Jxl,
            reason: "the JPEG recompressor panicked".into(),
        })
    })
}

fn failed(format: Format) -> impl Fn(&dyn std::fmt::Display) -> Error {
    move |e| Error::Internal {
        format,
        reason: format!("the encoder failed: {e}"),
    }
}

fn is_gray(img: &DynamicImage) -> bool {
    !img.color().has_color()
}

/// PNG, lossless. 16-bit stays 16-bit. `Default` deflate is within 1% of
/// `Best` at a third of the time; `Fast` is about 30% larger.
fn encode_png(
    img: &DynamicImage,
    icc: Option<&[u8]>,
    level: Compression,
) -> Result<Vec<u8>, Error> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let compression = match level {
        Compression::Smaller => CompressionType::Best,
        Compression::Balanced => CompressionType::Default,
        Compression::Faster => CompressionType::Fast,
    };
    let mut out = Vec::new();
    let mut enc =
        PngEncoder::new_with_quality(Cursor::new(&mut out), compression, FilterType::Adaptive);
    if let Some(icc) = icc {
        enc.set_icc_profile(icc.to_vec())
            .map_err(|e| failed(Format::Png)(&e))?;
    }
    img.write_with_encoder(enc)
        .map_err(|e| failed(Format::Png)(&e))?;
    Ok(out)
}

/// Baseline JPEG. Gray is written as one channel.
fn encode_jpeg(img: &DynamicImage, icc: Option<&[u8]>, quality: u8) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut enc =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, jpeg_quality(quality));
    if let Some(icc) = icc {
        enc.set_icc_profile(icc.to_vec())
            .map_err(|e| failed(Format::Jpeg)(&e))?;
    }
    let res = if is_gray(img) {
        img.to_luma8().write_with_encoder(enc)
    } else {
        img.to_rgb8().write_with_encoder(enc)
    };
    res.map_err(|e| failed(Format::Jpeg)(&e))?;
    Ok(out)
}

/// The `image` crate's JPEG quality for a WebP-scale quality: the JPEG quality
/// whose mean butteraugli distance (3-norm) matches WebP's, fitted over 60
/// photos and 60 drawings from Baris's shelf (measured 2026-10-30,
/// `examples/calibrate.rs`; photos match about 5 below the curve, drawings
/// about 5 above). Clamped to the fitted range, as for [`jxl_distance`].
fn jpeg_quality(webp_quality: u8) -> u8 {
    let q = f32::from(webp_quality).clamp(45.0, 95.0);
    (0.004615 * q * q - 0.17453 * q + 65.691).round() as u8
}

/// WebP, 8-bit. Lossless keeps RGB under fully transparent pixels (`exact`).
///
/// Gray goes in as RGB: zenwebp 0.4.6's lossy path copies L8 into the
/// limited-range Y plane unscaled (contrast-stretched, clipped; fixed on
/// upstream main only), and its lossless L8 path skips the full VP8L
/// encoder (imazen/zenwebp#57). WebP stores no gray mode either way.
fn encode_webp(
    img: &DynamicImage,
    icc: Option<&[u8]>,
    mode: Mode,
    level: Compression,
    stop: &dyn enough::Stop,
) -> Result<Vec<u8>, Error> {
    use zenwebp::{EncodeRequest, LosslessConfig, LossyConfig, PixelLayout};
    let (px, layout) = if img.color().has_alpha() {
        (img.to_rgba8().into_raw(), PixelLayout::Rgba8)
    } else {
        (img.to_rgb8().into_raw(), PixelLayout::Rgb8)
    };
    let (w, h) = (img.width(), img.height());
    // Lossless: libwebp's -z presets 9, 6 (the default) and 2 as (method,
    // effort); -z 9 is 2% smaller at 12-23x the time. Lossy: method 4 (the
    // default) or 2, which needs no quality mapping of its own (fitted
    // within 0.5 of the same quality).
    let ((z_method, z_effort), method) = match level {
        Compression::Smaller => ((6, 100.0), 4),
        Compression::Balanced => ((4, 75.0), 4),
        Compression::Faster => ((2, 25.0), 2),
    };
    let mut lossless = LosslessConfig::new().with_exact(true).with_method(z_method);
    lossless.quality = z_effort;
    let lossy;
    let mut req = match mode {
        Mode::Lossless => EncodeRequest::lossless(&lossless, &px, layout, w, h),
        Mode::Lossy(q) => {
            lossy = LossyConfig::new()
                .with_quality(f32::from(q))
                .with_method(method);
            EncodeRequest::lossy(&lossy, &px, layout, w, h)
        }
    };
    if let Some(icc) = icc {
        req = req.with_icc_profile(icc);
    }
    req.with_stop(stop)
        .encode()
        .map_err(|e| failed(Format::WebP)(&e))
}

/// The JPEG XL lossy effort and lossless effort per level. Lossless effort 6
/// and below is 40-80% larger than 7 (larger than PNG on drawings), so
/// `Faster` keeps 7; effort 9 is 5% smaller than 7 at 6x the time.
fn jxl_effort(level: Compression) -> (u8, u8) {
    match level {
        Compression::Smaller => (5, 9),
        Compression::Balanced => (5, 7),
        Compression::Faster => (3, 7),
    }
}

/// The JPEG XL butteraugli distance for a WebP-scale quality at a lossy
/// effort: the distance whose mean butteraugli distance (3-norm) matches
/// WebP's, fitted over 60 photos and 60 drawings from Baris's shelf
/// (measured 2026-10-30, `fit_levels`; worst point 0.14 from the curve at
/// effort 5, 0.48 at effort 3). Clamped to the fitted range.
fn jxl_distance(webp_quality: u8, effort: u8) -> f32 {
    let q = f32::from(webp_quality).clamp(45.0, 95.0);
    let d = match effort {
        3 => -0.000301 * q * q - 0.00165 * q + 3.5782,
        _ => -0.000696 * q * q + 0.05173 * q + 2.1431,
    };
    d.clamp(0.4, 15.0)
}

/// JPEG XL. 16-bit stays 16-bit, lossless or lossy.
fn encode_jxl(
    img: &DynamicImage,
    icc: Option<&[u8]>,
    mode: Mode,
    level: Compression,
    stop: &dyn enough::Stop,
) -> Result<Vec<u8>, Error> {
    use zenjxl::PixelLayout as L;
    let (px, layout): (Vec<u8>, L) = match img.color() {
        ColorType::L8 => (img.to_luma8().into_raw(), L::Gray8),
        ColorType::La8 => (img.to_luma_alpha8().into_raw(), L::GrayAlpha8),
        ColorType::Rgb8 => (img.to_rgb8().into_raw(), L::Rgb8),
        ColorType::Rgba8 => (img.to_rgba8().into_raw(), L::Rgba8),
        ColorType::L16 => (ne_bytes(img.to_luma16().into_raw()), L::Gray16),
        ColorType::La16 => (ne_bytes(img.to_luma_alpha16().into_raw()), L::GrayAlpha16),
        ColorType::Rgb16 => (ne_bytes(img.to_rgb16().into_raw()), L::Rgb16),
        _ => (ne_bytes(img.to_rgba16().into_raw()), L::Rgba16),
    };
    let (w, h) = (img.width(), img.height());
    let meta = icc.map(|icc| jxl_encoder::ImageMetadata::new().with_icc_profile(icc));
    let res = match mode {
        Mode::Lossless => {
            let cfg = zenjxl::LosslessConfig::new().with_effort(jxl_effort(level).1);
            let mut req = cfg.encode_request(w, h, layout).with_stop(stop);
            if let Some(meta) = &meta {
                req = req.with_metadata(meta);
            }
            req.encode(&px)
        }
        Mode::Lossy(q) => {
            let effort = jxl_effort(level).0;
            let cfg = zenjxl::LossyConfig::new(jxl_distance(q, effort)).with_effort(effort);
            let mut req = cfg.encode_request(w, h, layout).with_stop(stop);
            if let Some(meta) = &meta {
                req = req.with_metadata(meta);
            }
            req.encode(&px)
        }
    };
    res.map_err(|e| failed(Format::Jxl)(&e))
}

fn ne_bytes(px: Vec<u16>) -> Vec<u8> {
    px.into_iter().flat_map(u16::to_ne_bytes).collect()
}
