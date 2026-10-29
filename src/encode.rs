//! Encoding pixels into each format, lossless or at a quality.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use image::{ColorType, DynamicImage, ImageEncoder};

use crate::{Error, Format};

/// How to encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Lossless,
    /// Quality 1–100 on the WebP scale.
    Lossy(u8),
}

/// The AVIF encoder speed: 4 gives the smallest files (epublift's CLI default,
/// and the speed epublift's calibration was measured at).
const AVIF_SPEED: u8 = 4;

/// Encode `img` to `format`. The caller has already settled what the format
/// cannot hold: no transparency for JPEG, no 16-bit for a lossless target that
/// stores 8 bits. `img` carries no fully-opaque alpha channel.
pub(crate) fn encode(
    img: &DynamicImage,
    icc: Option<&[u8]>,
    format: Format,
    mode: Mode,
) -> Result<Vec<u8>, Error> {
    let run = || match format {
        Format::Png => encode_png(img, icc),
        Format::Jpeg => match mode {
            Mode::Lossy(q) => encode_jpeg(img, icc, q),
            Mode::Lossless => unreachable!("JPEG has no lossless mode; refused before encoding"),
        },
        Format::WebP => encode_webp(img, icc, mode),
        Format::Jxl => encode_jxl(img, icc, mode),
        Format::Avif => encode_avif(img, icc, mode),
    };
    catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|_| {
        Err(Error::Internal {
            format,
            reason: "the encoder panicked".into(),
        })
    })
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

/// PNG, lossless, at the strongest compression the encoder offers. 16-bit
/// stays 16-bit.
fn encode_png(img: &DynamicImage, icc: Option<&[u8]>) -> Result<Vec<u8>, Error> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut out = Vec::new();
    let mut enc = PngEncoder::new_with_quality(
        Cursor::new(&mut out),
        CompressionType::Best,
        FilterType::Adaptive,
    );
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

/// The WebP-scale quality as the `image` crate's JPEG quality. Not calibrated
/// yet (plan, A3): passed through.
fn jpeg_quality(webp_quality: u8) -> u8 {
    webp_quality.clamp(1, 100)
}

/// WebP, 8-bit. Lossless keeps RGB under fully transparent pixels (`exact`).
fn encode_webp(img: &DynamicImage, icc: Option<&[u8]>, mode: Mode) -> Result<Vec<u8>, Error> {
    use zenwebp::{EncodeRequest, LosslessConfig, LossyConfig, PixelLayout};
    let (px, layout) = match (is_gray(img), img.color().has_alpha()) {
        (true, false) => (img.to_luma8().into_raw(), PixelLayout::L8),
        (true, true) => (img.to_luma_alpha8().into_raw(), PixelLayout::La8),
        (false, false) => (img.to_rgb8().into_raw(), PixelLayout::Rgb8),
        (false, true) => (img.to_rgba8().into_raw(), PixelLayout::Rgba8),
    };
    let (w, h) = (img.width(), img.height());
    let lossless = LosslessConfig::new().with_exact(true);
    let lossy;
    let mut req = match mode {
        Mode::Lossless => EncodeRequest::lossless(&lossless, &px, layout, w, h),
        Mode::Lossy(q) => {
            lossy = LossyConfig::new().with_quality(f32::from(q));
            EncodeRequest::lossy(&lossy, &px, layout, w, h)
        }
    };
    if let Some(icc) = icc {
        req = req.with_icc_profile(icc);
    }
    req.encode().map_err(|e| failed(Format::WebP)(&e))
}

/// The JPEG XL butteraugli distance for a WebP-scale quality (epublift's
/// quadratic fit over 40 JPEG-source images from 11 books, anchored on WebP
/// q80; to be re-measured in A3). Clamped to the fitted range.
fn jxl_distance(webp_quality: u8) -> f32 {
    let q = f32::from(webp_quality).clamp(45.0, 95.0);
    (-0.000494 * q * q + 0.01808 * q + 3.639).clamp(0.4, 15.0)
}

/// JPEG XL. 16-bit stays 16-bit, lossless or lossy.
fn encode_jxl(img: &DynamicImage, icc: Option<&[u8]>, mode: Mode) -> Result<Vec<u8>, Error> {
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
            let cfg = zenjxl::LosslessConfig::new();
            let mut req = cfg.encode_request(w, h, layout);
            if let Some(meta) = &meta {
                req = req.with_metadata(meta);
            }
            req.encode(&px)
        }
        Mode::Lossy(q) => {
            let cfg = zenjxl::LossyConfig::new(jxl_distance(q));
            let mut req = cfg.encode_request(w, h, layout);
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

/// The AVIF quality for a WebP-scale quality (epublift's fit, as for
/// [`jxl_distance`]).
fn avif_quality(webp_quality: u8) -> f32 {
    let q = f32::from(webp_quality).clamp(45.0, 95.0);
    (0.017529 * q * q - 1.70862 * q + 95.783).clamp(1.0, 100.0)
}

/// AVIF, 8-bit. Lossless uses quantizer 0 in the RGB colour model at 4:4:4,
/// so no RGB → YCbCr rounding happens. Gray is expanded to RGB (exactly).
fn encode_avif(img: &DynamicImage, icc: Option<&[u8]>, mode: Mode) -> Result<Vec<u8>, Error> {
    use rgb::FromSlice;
    use zenravif::{BitDepth, ColorModel, Encoder};
    let mut enc = Encoder::new()
        .with_speed(AVIF_SPEED)
        .with_bit_depth(BitDepth::Eight);
    enc = match mode {
        // Quantizer 0 alone is not lossless: quantisation matrices and
        // variance-adaptive quantisation still vary q per block.
        Mode::Lossless => enc
            .with_lossless(true)
            .with_qm(false)
            .with_vaq(false, 1.0)
            .with_internal_color_model(ColorModel::RGB),
        Mode::Lossy(q) => enc.with_quality(avif_quality(q)),
    };
    if let Some(icc) = icc {
        enc = enc.with_icc_profile(icc.to_vec());
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    let res = if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        enc.encode_rgba(imgref::Img::new(rgba.as_raw().as_rgba(), w, h))
    } else {
        let rgb = img.to_rgb8();
        enc.encode_rgb(imgref::Img::new(rgb.as_raw().as_rgb(), w, h))
    };
    res.map(|e| e.avif_file)
        .map_err(|e| failed(Format::Avif)(&e))
}
