//! The conversion: one image in, one image out, and the facts about it.

use image::DynamicImage;

use crate::decode::decode;
use crate::encode::{self, recompress_jpeg_to_jxl};
use crate::{Cancel, Error, Format, Info, inspect};

/// What to convert to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// The format to encode to.
    pub format: Format,
    /// The encoder quality, 1–100, on one scale across formats (WebP's).
    /// Always a lossy encode; 100 is the highest lossy quality, not lossless.
    /// `None` converts losslessly, which JPEG cannot do.
    pub quality: Option<u8>,
    /// Flatten transparency onto this colour (R, G, B); the output then has
    /// no alpha. Needed to convert a transparent image to JPEG.
    pub background: Option<[u8; 3]>,
    /// Refuse an image with more pixels than this (width × height), read
    /// from its header before anything is decoded. Conversion time grows
    /// with the pixel count, so this bounds how long one image can take;
    /// the always-on decode limits only bound memory.
    pub max_pixels: Option<u64>,
    /// Stop the conversion when this token is cancelled or its deadline
    /// passes.
    pub cancel: Option<Cancel>,
    /// How much time the encoder spends on a smaller file. The default is
    /// [`Compression::Balanced`].
    pub compression: Compression,
}

impl Options {
    /// Convert to `format`, losslessly, keeping transparency.
    pub fn new(format: Format) -> Self {
        Options {
            format,
            quality: None,
            background: None,
            max_pixels: None,
            cancel: None,
            compression: Compression::Balanced,
        }
    }

    /// Encode lossily at `quality` (1–100).
    pub fn with_quality(mut self, quality: u8) -> Self {
        self.quality = Some(quality);
        self
    }

    /// Flatten transparency onto `rgb`.
    pub fn with_background(mut self, rgb: [u8; 3]) -> Self {
        self.background = Some(rgb);
        self
    }

    /// Trade encoding time for file size.
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Refuse images with more than `max` pixels.
    pub fn with_max_pixels(mut self, max: u64) -> Self {
        self.max_pixels = Some(max);
        self
    }

    /// Stop when `cancel` is cancelled or its deadline passes.
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Whether these options can be honoured for some image, checked without
    /// one: fails with [`Error::InvalidOptions`] for an AVIF target (not
    /// written in this release), a JPEG target without a quality, PNG with
    /// one, or a quality outside 1–100.
    pub fn check(&self) -> Result<(), Error> {
        check_options(self).map(|_| ())
    }
}

/// How much time the encoder spends on a smaller file. It never changes
/// what the image looks like: the same `quality` gives the same visual
/// quality at every level (each level has its own calibration), and a
/// lossless conversion gives the same pixels. Only the file size and the
/// encoding time differ. Nor does it change the image's width or height.
///
/// The levels are relative to each other, not promises: `Smaller` gives a
/// smaller file than `Balanced` or the same one, `Faster` a quicker encode or
/// the same one. Where an encoder has nothing to trade, two levels use the
/// same setting, and JPEG has no setting at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Compression {
    /// A smaller file, for a longer encode.
    Smaller,
    /// The default.
    #[default]
    Balanced,
    /// A quicker encode, for a larger file.
    Faster,
}

/// How the output was encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// Nothing was lost: the output decodes to the pixels the input decodes
    /// to (or, for JPEG → JPEG XL, holds the original JPEG itself).
    Lossless,
    /// Encoded lossily at this quality.
    Lossy(u8),
}

/// How much a [`Finding`] matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
}

/// Something the conversion did that the caller should know about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Finding {
    /// The output was encoded lossily.
    LossyEncode,
    /// The input was stored losslessly and the output is lossy.
    LosslessToLossy,
    /// Transparency was flattened onto the background colour.
    Flattened,
    /// 16 bits per sample became 8.
    BitDepthReduced,
    /// A lossy WebP whose colour was codec noise around gray was encoded as
    /// gray.
    TreatedAsGray,
    /// JPEG → JPEG XL kept the original JPEG: a decoder can return it byte
    /// for byte.
    JpegKept,
    /// JPEG → JPEG XL could not keep the original JPEG, so its decoded pixels
    /// were stored losslessly instead.
    JpegNotKept,
}

impl Finding {
    pub fn severity(self) -> Severity {
        match self {
            Finding::LosslessToLossy | Finding::JpegNotKept => Severity::Warning,
            _ => Severity::Info,
        }
    }

    /// The stable code machine output carries (veripublica FORMATS §1.3,
    /// tool-owned `IMG-` prefix).
    pub fn code(self) -> &'static str {
        match self {
            Finding::LossyEncode => "IMG-001",
            Finding::LosslessToLossy => "IMG-002",
            Finding::Flattened => "IMG-003",
            Finding::BitDepthReduced => "IMG-004",
            Finding::TreatedAsGray => "IMG-005",
            Finding::JpegKept => "IMG-006",
            Finding::JpegNotKept => "IMG-007",
        }
    }

    /// A readable name for [`code`](Finding::code).
    pub fn rule(self) -> &'static str {
        match self {
            Finding::LossyEncode => "convert.lossy",
            Finding::LosslessToLossy => "convert.lossless_to_lossy",
            Finding::Flattened => "convert.flattened",
            Finding::BitDepthReduced => "convert.depth_reduced",
            Finding::TreatedAsGray => "convert.near_gray",
            Finding::JpegKept => "convert.jpeg_kept",
            Finding::JpegNotKept => "convert.jpeg_not_kept",
        }
    }

    /// A one-line English description.
    pub fn message(self) -> &'static str {
        match self {
            Finding::LossyEncode => "lossy encode: the output differs from the input",
            Finding::LosslessToLossy => "the input was lossless and the output is lossy",
            Finding::Flattened => "transparency was flattened onto the background colour",
            Finding::BitDepthReduced => "16 bits per sample were reduced to 8",
            Finding::TreatedAsGray => "near-gray colour (lossy WebP noise) was encoded as gray",
            Finding::JpegKept => {
                "the original JPEG is kept inside the JPEG XL and can be reconstructed"
            }
            Finding::JpegNotKept => {
                "the original JPEG could not be kept; its decoded pixels were stored losslessly"
            }
        }
    }
}

/// The result of a conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Conversion {
    /// The encoded output.
    pub bytes: Vec<u8>,
    /// The output format.
    pub format: Format,
    /// How the output was encoded.
    pub quality: Quality,
    /// The compression level asked for. JPEG output and the JPEG → JPEG XL
    /// recompression have no setting it changes.
    pub compression: Compression,
    /// The input's header facts.
    pub source: Info,
    /// The input's size in bytes.
    pub input_size: usize,
    /// What the conversion did that the caller should know, most severe first.
    pub findings: Vec<Finding>,
}

/// Convert `input` as `options` say. The output may be larger than the input:
/// that is a fact to report, not a failure. Whether to use it is the caller's
/// decision.
pub fn convert(input: &[u8], options: &Options) -> Result<Conversion, Error> {
    let target = options.format;
    let quality = check_options(options)?;
    let cancel = options.cancel.as_ref();
    let check = || cancel.map_or(Ok(()), Cancel::check);
    check()?;
    if let Some(max) = options.max_pixels {
        let info = inspect(input)?;
        if u64::from(info.width) * u64::from(info.height) > max {
            return Err(Error::TooManyPixels {
                width: info.width,
                height: info.height,
                max_pixels: max,
            });
        }
    }
    let mut findings = Vec::new();

    if quality == Quality::Lossless
        && options.background.is_none()
        && target == Format::Jxl
        && Format::sniff(input) == Some(Format::Jpeg)
    {
        let source = inspect(input)?;
        if !source.within_limits() {
            return Err(Error::TooLarge {
                width: source.width,
                height: source.height,
            });
        }
        match recompress_jpeg_to_jxl(input) {
            Ok(bytes) => {
                return Ok(Conversion {
                    bytes,
                    format: target,
                    quality,
                    compression: options.compression,
                    source,
                    input_size: input.len(),
                    findings: vec![Finding::JpegKept],
                });
            }
            Err(Error::Unsupported { .. }) => findings.push(Finding::JpegNotKept),
            Err(e) => return Err(e),
        }
    }

    let decoded = decode(input)?;
    check()?;
    let mut img = decoded.image;

    if img.color().has_alpha() {
        if !has_transparency(&img) {
            img = drop_alpha(img);
        } else if let Some(bg) = options.background {
            img = flatten(&img, bg);
            findings.push(Finding::Flattened);
        } else if !target.has_alpha() {
            return Err(Error::TransparencyNeedsBackground { target });
        }
    }

    let sixteen = is_16_bit(&img);
    if sixteen && fits_in_8_bits(&img) {
        img = to_8_bit(&img);
    } else if sixteen {
        let keeps_16 = matches!(target, Format::Png | Format::Jxl);
        match quality {
            Quality::Lossless if !keeps_16 => {
                return Err(Error::BitDepthTooHigh { bits: 16, target });
            }
            Quality::Lossy(_) if !keeps_16 => {
                img = to_8_bit(&img);
                findings.push(Finding::BitDepthReduced);
            }
            _ => {}
        }
    }

    if matches!(quality, Quality::Lossy(_))
        && decoded.near_gray
        && decoded.icc.is_none()
        && img.color().has_color()
    {
        img = DynamicImage::ImageLuma8(img.to_luma8());
        findings.push(Finding::TreatedAsGray);
    }

    let mode = match quality {
        Quality::Lossless => encode::Mode::Lossless,
        Quality::Lossy(q) => encode::Mode::Lossy(q),
    };
    check()?;
    let bytes = encode::encode(
        &img,
        decoded.icc.as_deref(),
        target,
        mode,
        options.compression,
        cancel,
    )?;

    if let Quality::Lossy(_) = quality {
        findings.push(Finding::LossyEncode);
        if decoded.info.lossless == Some(true) {
            findings.push(Finding::LosslessToLossy);
        }
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity()));
    Ok(Conversion {
        bytes,
        format: target,
        quality,
        compression: options.compression,
        source: decoded.info,
        input_size: input.len(),
        findings,
    })
}

fn check_options(options: &Options) -> Result<Quality, Error> {
    let invalid = |reason: &str| {
        Err(Error::InvalidOptions {
            reason: reason.into(),
        })
    };
    match (options.format, options.quality) {
        (Format::Avif, _) => invalid(
            "AVIF output is not available in this release, because its encoder \
             (zenravif 0.2) is not on crates.io yet; AVIF input is read",
        ),
        (Format::Jpeg, None) => {
            invalid("JPEG has no lossless mode; give a quality (1-100) to convert to JPEG")
        }
        (Format::Png, Some(_)) => {
            invalid("PNG is always lossless and has no quality; convert to PNG without one")
        }
        (_, Some(q)) if !(1..=100).contains(&q) => invalid("the quality must be from 1 to 100"),
        (_, Some(q)) => Ok(Quality::Lossy(q)),
        (_, None) => Ok(Quality::Lossless),
    }
}

/// Whether any pixel is not fully opaque.
fn has_transparency(img: &DynamicImage) -> bool {
    match img {
        DynamicImage::ImageLumaA8(i) => i.pixels().any(|p| p[1] != u8::MAX),
        DynamicImage::ImageRgba8(i) => i.pixels().any(|p| p[3] != u8::MAX),
        DynamicImage::ImageLumaA16(i) => i.pixels().any(|p| p[1] != u16::MAX),
        DynamicImage::ImageRgba16(i) => i.pixels().any(|p| p[3] != u16::MAX),
        other => other.color().has_alpha(),
    }
}

/// The image without its alpha channel, which the caller knows is all opaque.
fn drop_alpha(img: DynamicImage) -> DynamicImage {
    match img {
        DynamicImage::ImageLumaA8(_) => DynamicImage::ImageLuma8(img.to_luma8()),
        DynamicImage::ImageRgba8(_) => DynamicImage::ImageRgb8(img.to_rgb8()),
        DynamicImage::ImageLumaA16(_) => DynamicImage::ImageLuma16(img.to_luma16()),
        DynamicImage::ImageRgba16(_) => DynamicImage::ImageRgb16(img.to_rgb16()),
        other => other,
    }
}

/// Composite onto `bg` (alpha-weighted, rounded), keeping the bit depth.
/// Gray stays gray when the background is gray.
fn flatten(img: &DynamicImage, bg: [u8; 3]) -> DynamicImage {
    fn mix(c: u32, a: u32, bg: u32, max: u32) -> u32 {
        (c * a + bg * (max - a) + max / 2) / max
    }
    let gray = !img.color().has_color() && bg[0] == bg[1] && bg[1] == bg[2];
    if is_16_bit(img) {
        let bg16 = bg.map(|c| u32::from(c) * 257);
        let rgba = img.to_rgba16();
        let rgb = image::ImageBuffer::from_fn(rgba.width(), rgba.height(), |x, y| {
            let p = rgba.get_pixel(x, y).0;
            let a = u32::from(p[3]);
            image::Rgb([0, 1, 2].map(|i| mix(u32::from(p[i]), a, bg16[i], 65_535) as u16))
        });
        let out = DynamicImage::ImageRgb16(rgb);
        if gray {
            DynamicImage::ImageLuma16(out.to_luma16())
        } else {
            out
        }
    } else {
        let rgba = img.to_rgba8();
        let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            let p = rgba.get_pixel(x, y).0;
            let a = u32::from(p[3]);
            image::Rgb([0, 1, 2].map(|i| mix(u32::from(p[i]), a, u32::from(bg[i]), 255) as u8))
        });
        let out = DynamicImage::ImageRgb8(rgb);
        if gray {
            DynamicImage::ImageLuma8(out.to_luma8())
        } else {
            out
        }
    }
}

fn is_16_bit(img: &DynamicImage) -> bool {
    img.color().bytes_per_pixel() / img.color().channel_count() == 2
}

/// Whether a 16-bit image holds only values an 8-bit image can (each a
/// multiple of 257, as an 8-bit source widened to 16 bits is). Then going
/// back to 8 bits loses nothing.
fn fits_in_8_bits(img: &DynamicImage) -> bool {
    let all = |px: &[u16]| px.iter().all(|v| v % 257 == 0);
    match img {
        DynamicImage::ImageLuma16(i) => all(i.as_raw()),
        DynamicImage::ImageLumaA16(i) => all(i.as_raw()),
        DynamicImage::ImageRgb16(i) => all(i.as_raw()),
        DynamicImage::ImageRgba16(i) => all(i.as_raw()),
        _ => false,
    }
}

fn to_8_bit(img: &DynamicImage) -> DynamicImage {
    match img.color() {
        image::ColorType::L16 => DynamicImage::ImageLuma8(img.to_luma8()),
        image::ColorType::La16 => DynamicImage::ImageLumaA8(img.to_luma_alpha8()),
        image::ColorType::Rgb16 => DynamicImage::ImageRgb8(img.to_rgb8()),
        _ => DynamicImage::ImageRgba8(img.to_rgba8()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimg;
    use std::io::Cursor;

    fn png(img: &DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn pixels(bytes: &[u8]) -> DynamicImage {
        decode(bytes).unwrap().image
    }

    fn rgba() -> DynamicImage {
        DynamicImage::ImageRgba8(testimg::gradient_rgba(37, 21))
    }

    fn rgb() -> DynamicImage {
        DynamicImage::ImageRgb8(testimg::gradient_rgb(37, 21))
    }

    #[test]
    fn lossless_round_trips_exactly() {
        for src in [
            rgb(),
            rgba(),
            DynamicImage::ImageLuma8(testimg::gray_noise(23, 19)),
        ] {
            for target in [Format::Png, Format::WebP, Format::Jxl] {
                let c = convert(&png(&src), &Options::new(target)).unwrap();
                assert_eq!((c.format, c.quality), (target, Quality::Lossless));
                assert!(c.findings.is_empty(), "{target}: {:?}", c.findings);
                assert_eq!(pixels(&c.bytes), src, "{target} from {:?}", src.color());
            }
        }
    }

    /// Runs `tool args…` if it is installed; `None` when it is not.
    fn run(tool: &str, args: &[&std::path::Path]) -> Option<()> {
        if cfg!(target_family = "wasm") {
            return None; // no processes
        }
        let o = std::process::Command::new(tool).args(args).output().ok()?;
        assert!(
            o.status.success(),
            "{tool}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        Some(())
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("imagoverto-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// AVIF is read but not written in this release, whatever the image.
    #[test]
    fn avif_output_is_refused() {
        for opts in [
            Options::new(Format::Avif),
            Options::new(Format::Avif).with_quality(80),
        ] {
            assert!(matches!(opts.check(), Err(Error::InvalidOptions { .. })));
            assert!(matches!(
                convert(&png(&rgb()), &opts),
                Err(Error::InvalidOptions { .. })
            ));
        }
    }

    #[test]
    fn lossy_reports_what_it_did() {
        let c = convert(&png(&rgb()), &Options::new(Format::WebP).with_quality(80)).unwrap();
        assert_eq!(c.quality, Quality::Lossy(80));
        assert_eq!(c.findings, [Finding::LosslessToLossy, Finding::LossyEncode]);
        let jpeg = convert(&png(&rgb()), &Options::new(Format::Jpeg).with_quality(80)).unwrap();
        let again = convert(&jpeg.bytes, &Options::new(Format::Jxl).with_quality(70)).unwrap();
        // From a lossy source: no lossless-to-lossy warning.
        assert_eq!(again.findings, [Finding::LossyEncode]);
        assert_eq!(again.input_size, jpeg.bytes.len());
    }

    #[test]
    fn option_refusals() {
        let input = png(&rgb());
        for opts in [
            Options::new(Format::Jpeg),
            Options::new(Format::Png).with_quality(80),
            Options::new(Format::WebP).with_quality(0),
            Options::new(Format::WebP).with_quality(101),
        ] {
            assert!(
                matches!(convert(&input, &opts), Err(Error::InvalidOptions { .. })),
                "{opts:?}"
            );
        }
    }

    #[test]
    fn transparency_is_never_dropped_silently() {
        let input = png(&rgba());
        assert_eq!(
            convert(&input, &Options::new(Format::Jpeg).with_quality(90)),
            Err(Error::TransparencyNeedsBackground {
                target: Format::Jpeg
            })
        );
        let c = convert(
            &input,
            &Options::new(Format::Jpeg)
                .with_quality(90)
                .with_background([255; 3]),
        )
        .unwrap();
        assert!(c.findings.contains(&Finding::Flattened));
        // Flattening on request works for a target with alpha too, losslessly.
        let c = convert(
            &input,
            &Options::new(Format::Png).with_background([0, 0, 255]),
        )
        .unwrap();
        let out = pixels(&c.bytes).to_rgb8();
        // x = 0 is fully transparent: pure background.
        assert_eq!(out.get_pixel(0, 5).0, [0, 0, 255]);
        // The last column is opaque: the source colour.
        let src = rgba().to_rgba8();
        let p = src.get_pixel(36, 5).0;
        assert_eq!(out.get_pixel(36, 5).0, [p[0], p[1], p[2]]);
        // An alpha channel that is all opaque is not transparency.
        let opaque = DynamicImage::ImageRgba8(rgb().to_rgba8());
        let c = convert(&png(&opaque), &Options::new(Format::Jpeg).with_quality(90)).unwrap();
        assert!(c.findings.iter().all(|f| *f != Finding::Flattened));
    }

    #[test]
    fn sixteen_bits_are_kept_or_refused_never_cut_silently() {
        let deep = DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(9, 7, |x, y| {
            image::Rgb([(x * 7001) as u16, (y * 9001) as u16, 12_345])
        }));
        let input = png(&deep);
        for target in [Format::Png, Format::Jxl] {
            let c = convert(&input, &Options::new(target)).unwrap();
            assert_eq!(pixels(&c.bytes), deep, "{target}");
        }
        assert_eq!(
            convert(&input, &Options::new(Format::WebP)),
            Err(Error::BitDepthTooHigh {
                bits: 16,
                target: Format::WebP
            })
        );
        let c = convert(&input, &Options::new(Format::WebP).with_quality(90)).unwrap();
        assert!(c.findings.contains(&Finding::BitDepthReduced));
        // 8-bit values widened to 16 bits lose nothing going back.
        let widened = DynamicImage::ImageRgb16(rgb().to_rgb16());
        let c = convert(&png(&widened), &Options::new(Format::WebP)).unwrap();
        assert_eq!(pixels(&c.bytes), rgb());
    }

    /// zenwebp 0.4.6 copies L8 straight into VP8's limited-range Y plane, so
    /// gray came back contrast-stretched and clipped (butteraugli 9.9 where
    /// cwebp gave 0.5 on a book photo). Every lossy target must keep the tones.
    #[test]
    fn lossy_gray_keeps_its_tones() {
        let gray = DynamicImage::ImageLuma8(rgb().to_luma8());
        let gray_alpha = DynamicImage::ImageLumaA8(rgba().to_luma_alpha8());
        for src in [gray, gray_alpha] {
            for target in [Format::WebP, Format::Jxl, Format::Jpeg] {
                let opts = Options::new(target)
                    .with_quality(90)
                    .with_background([255; 3]);
                let c = convert(&png(&src), &opts).unwrap();
                let back = pixels(&c.bytes);
                let want = if back.color().has_alpha() {
                    src.clone()
                } else {
                    flatten_on_white(&src)
                };
                let db = testimg::psnr(&back.to_luma_alpha8().into(), &want);
                assert!(db > 35.0, "{target} from {:?}: {db:.1} dB", src.color());
            }
        }
    }

    fn flatten_on_white(img: &DynamicImage) -> DynamicImage {
        let mut out = img.to_luma_alpha8();
        for p in out.pixels_mut() {
            let [l, a] = p.0;
            let v = (u32::from(l) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255;
            p.0 = [v as u8, 255];
        }
        out.into()
    }

    #[test]
    fn near_gray_webp_becomes_gray_only_when_lossy() {
        let gray = DynamicImage::ImageLuma8(testimg::gray_noise(32, 32));
        let webp = convert(&png(&gray), &Options::new(Format::WebP).with_quality(60)).unwrap();
        let lossy = convert(&webp.bytes, &Options::new(Format::Jpeg).with_quality(80)).unwrap();
        assert!(lossy.findings.contains(&Finding::TreatedAsGray));
        assert!(inspect(&lossy.bytes).unwrap().gray);
        let lossless = convert(&webp.bytes, &Options::new(Format::Png)).unwrap();
        assert!(!lossless.findings.contains(&Finding::TreatedAsGray));
        assert_eq!(pixels(&lossless.bytes), pixels(&webp.bytes));
    }

    #[test]
    fn icc_profile_travels() {
        let icc = moxcms::ColorProfile::new_srgb().encode().unwrap();
        let mut out = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(Cursor::new(&mut out));
        image::ImageEncoder::set_icc_profile(&mut enc, icc.clone()).unwrap();
        rgb().write_with_encoder(enc).unwrap();
        for target in [Format::Png, Format::WebP, Format::Jxl] {
            let c = convert(&out, &Options::new(target)).unwrap();
            assert_eq!(decode(&c.bytes).unwrap().icc, Some(icc.clone()), "{target}");
        }
    }

    #[test]
    fn jpeg_to_jxl_keeps_the_jpeg() {
        let jpeg = convert(&png(&rgb()), &Options::new(Format::Jpeg).with_quality(85)).unwrap();
        let c = convert(&jpeg.bytes, &Options::new(Format::Jxl)).unwrap();
        assert_eq!(c.quality, Quality::Lossless);
        assert_eq!(c.findings, [Finding::JpegKept]);
        // When djxl is installed, check the reconstruction byte for byte.
        if cfg!(target_family = "wasm") {
            return; // no processes, no temp dir
        }
        let (jxl, back) = (tmp("kept.jxl"), tmp("kept.jpg"));
        std::fs::write(&jxl, &c.bytes).unwrap();
        match run("djxl", &[&jxl, &back]) {
            Some(()) => assert_eq!(std::fs::read(&back).unwrap(), jpeg.bytes),
            None => eprintln!("djxl not available: reconstruction not checked"),
        }
    }

    #[test]
    fn errors_come_back_as_values() {
        assert_eq!(
            convert(b"not an image", &Options::new(Format::Png)),
            Err(Error::Unrecognized)
        );
        let mut truncated = png(&rgb());
        truncated.truncate(60);
        assert!(convert(&truncated, &Options::new(Format::WebP)).is_err());
    }

    #[test]
    fn max_pixels_refuses_from_the_header() {
        let input = png(&rgb());
        let (w, h) = (rgb().width(), rgb().height());
        let px = u64::from(w) * u64::from(h);
        let opts = Options::new(Format::WebP).with_quality(80);
        assert!(convert(&input, &opts.clone().with_max_pixels(px)).is_ok());
        assert_eq!(
            convert(&input, &opts.with_max_pixels(px - 1)),
            Err(Error::TooManyPixels {
                width: w,
                height: h,
                max_pixels: px - 1
            })
        );
        // The JPEG -> JPEG XL recompression is held to it too.
        let jpeg = convert(&input, &Options::new(Format::Jpeg).with_quality(85)).unwrap();
        assert!(matches!(
            convert(&jpeg.bytes, &Options::new(Format::Jxl).with_max_pixels(1)),
            Err(Error::TooManyPixels { .. })
        ));
    }

    #[test]
    fn a_cancelled_conversion_stops() {
        let input = png(&rgb());
        let cancel = Cancel::new();
        cancel.cancel();
        for format in [Format::WebP, Format::Jxl, Format::Jpeg, Format::Png] {
            let opts = Options::new(format).with_cancel(cancel.clone());
            let opts = match format {
                Format::Png => opts,
                _ => opts.with_quality(80),
            };
            assert_eq!(convert(&input, &opts), Err(Error::Cancelled), "{format}");
        }
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        {
            let late = Cancel::after(std::time::Duration::ZERO);
            let opts = Options::new(Format::Png).with_cancel(late);
            assert_eq!(convert(&input, &opts), Err(Error::Cancelled));
        }
    }

    /// The WebP encoder itself honours the token, not only imagoverto's
    /// checks around it: a token stopped before the encode makes it give up
    /// at its first check. (How soon it stops once under way is measured,
    /// not tested: it depends on the machine.) jxl-encoder
    /// 0.3.4 takes a token but never checks it.
    #[test]
    fn the_encoders_check_the_token() {
        let img = DynamicImage::ImageRgb8(testimg::gradient_rgb(256, 256));
        let cancel = Cancel::new();
        cancel.cancel();
        // JPEG XL joins once jxl-encoder checks the token (0.4.0).
        let format = Format::WebP;
        for mode in [encode::Mode::Lossless, encode::Mode::Lossy(80)] {
            let with = |c: &Cancel| {
                encode::encode(&img, None, format, mode, Compression::Balanced, Some(c))
            };
            assert_eq!(with(&cancel), Err(Error::Cancelled), "{format} {mode:?}");
            assert!(with(&Cancel::new()).is_ok());
        }
    }

    const LEVELS: [Compression; 3] = [
        Compression::Smaller,
        Compression::Balanced,
        Compression::Faster,
    ];

    /// A level changes the file, never the pixels.
    #[test]
    fn every_level_is_lossless() {
        let gray = DynamicImage::ImageLuma8(testimg::gray_noise(31, 17));
        for img in [rgb(), rgba(), gray] {
            let input = png(&img);
            for format in [Format::WebP, Format::Jxl, Format::Png] {
                for level in LEVELS {
                    let opts = Options::new(format).with_compression(level);
                    let c = convert(&input, &opts).unwrap();
                    assert_eq!(c.quality, Quality::Lossless);
                    assert_eq!(pixels(&c.bytes), img, "{format} {level:?}");
                }
            }
        }
    }

    /// Every level encodes lossily at about the quality asked for.
    #[test]
    fn every_level_keeps_the_quality() {
        let img = DynamicImage::ImageRgb8(testimg::gradient_rgb(64, 48));
        let input = png(&img);
        for format in [Format::WebP, Format::Jxl, Format::Jpeg] {
            for level in LEVELS {
                let opts = Options::new(format)
                    .with_quality(90)
                    .with_compression(level);
                let c = convert(&input, &opts).unwrap();
                let p = testimg::psnr(&img, &pixels(&c.bytes));
                assert!(p > 35.0, "{format} {level:?}: PSNR {p:.1}");
            }
        }
    }
}
