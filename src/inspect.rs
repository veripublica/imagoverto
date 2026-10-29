//! What an image file says about itself, read from its header only.

use std::io::Cursor;

use image::ImageDecoder;

use crate::{Error, Format, jpeg_quality, limits};

/// What an image's header says. Nothing here needs the pixels to be decoded,
/// so it is cheap even for a large file; whether the pixels actually decode
/// is a separate question.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Info {
    pub format: Format,
    /// Width in pixels, as stored (before any EXIF or `irot` rotation).
    pub width: u32,
    /// Height in pixels, as stored.
    pub height: u32,
    /// Whether the file has an alpha channel. It may still be fully opaque.
    pub has_alpha: bool,
    /// Whether the file stores the image as grayscale. Always `false` for
    /// WebP, which has no grayscale mode.
    pub gray: bool,
    /// Bits per sample, as stored.
    pub bit_depth: u8,
    /// Whether the file is an animation.
    pub animated: bool,
    /// Whether the file embeds an ICC colour profile.
    pub icc_profile: bool,
    /// Whether the encoding is lossless, where the header says so: always
    /// for PNG, never for JPEG; for WebP and JPEG XL from their headers.
    /// `None` when the header does not tell, and for AVIF, whose header
    /// reader misreads this (imazen/zenavif#46).
    pub lossless: Option<bool>,
    /// For a JPEG, the libjpeg quality (1–100) its luminance table matches.
    pub jpeg_quality: Option<u8>,
}

impl Info {
    /// Whether the image is within the decode limits that every conversion
    /// enforces ([`MAX_DIMENSION`](crate::MAX_DIMENSION) per side,
    /// [`MAX_DECODED_BYTES`](crate::MAX_DECODED_BYTES) decoded).
    pub fn within_limits(&self) -> bool {
        let channels = match (self.gray, self.has_alpha) {
            (true, false) => 1,
            (true, true) => 2,
            (false, false) => 3,
            (false, true) => 4,
        };
        let bytes_per_sample = if self.bit_depth > 8 { 2 } else { 1 };
        limits::within(self.width, self.height, channels * bytes_per_sample)
    }
}

/// Read an image's header. Fails with [`Error::Unrecognized`] for anything
/// that is not one of the five formats, and [`Error::Malformed`] when the
/// header cannot be read.
pub fn inspect(bytes: &[u8]) -> Result<Info, Error> {
    let format = Format::sniff(bytes).ok_or(Error::Unrecognized)?;
    let malformed = |reason: String| Error::Malformed { format, reason };
    match format {
        Format::Jpeg => {
            let mut d = image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes))
                .map_err(|e| malformed(e.to_string()))?;
            let (width, height) = d.dimensions();
            let color = d.color_type();
            let icc = d.icc_profile().ok().flatten().is_some();
            Ok(Info {
                format,
                width,
                height,
                has_alpha: false,
                gray: color.channel_count() == 1,
                bit_depth: 8,
                animated: false,
                icc_profile: icc,
                lossless: Some(false),
                jpeg_quality: jpeg_quality::estimate(bytes),
            })
        }
        Format::Png => {
            let mut d = image::codecs::png::PngDecoder::new(Cursor::new(bytes))
                .map_err(|e| malformed(e.to_string()))?;
            let (width, height) = d.dimensions();
            let color = d.color_type();
            let animated = d.is_apng().map_err(|e| malformed(e.to_string()))?;
            let icc = d.icc_profile().ok().flatten().is_some();
            Ok(Info {
                format,
                width,
                height,
                has_alpha: color.has_alpha(),
                gray: !color.has_color(),
                bit_depth: (color.bytes_per_pixel() / color.channel_count() * 8),
                animated,
                icc_profile: icc,
                lossless: Some(true),
                jpeg_quality: None,
            })
        }
        Format::WebP => {
            let p = zenwebp::detect::probe(bytes).map_err(|e| malformed(format!("{e:?}")))?;
            Ok(Info {
                format,
                width: p.width,
                height: p.height,
                has_alpha: p.has_alpha,
                gray: false,
                bit_depth: 8,
                animated: p.has_animation,
                icc_profile: p.icc_profile.is_some(),
                lossless: Some(matches!(
                    p.bitstream,
                    zenwebp::detect::BitstreamType::Lossless
                )),
                jpeg_quality: None,
            })
        }
        Format::Jxl => {
            let p = zenjxl::probe(bytes).map_err(|e| malformed(e.to_string()))?;
            Ok(Info {
                format,
                width: p.width,
                height: p.height,
                has_alpha: p.has_alpha,
                gray: p.is_gray,
                bit_depth: p.bit_depth.unwrap_or(8),
                animated: p.has_animation,
                icc_profile: p.icc_profile.is_some(),
                // Modular lossy exists but is rare; VarDCT is always lossy.
                lossless: if p.xyb_encoded { Some(false) } else { None },
                jpeg_quality: None,
            })
        }
        Format::Avif => {
            let p = zenavif_parse::AvifParser::from_bytes(bytes)
                .map_err(|e| malformed(e.to_string()))?;
            let meta = p.primary_metadata().map_err(|e| malformed(e.to_string()))?;
            let (width, height) = match p.grid_config() {
                Some(g) if g.output_width != 0 && g.output_height != 0 => {
                    (g.output_width, g.output_height)
                }
                _ => (meta.max_frame_width.get(), meta.max_frame_height.get()),
            };
            Ok(Info {
                format,
                width,
                height,
                has_alpha: p.alpha_data().is_some(),
                gray: meta.monochrome,
                bit_depth: meta.bit_depth,
                animated: p.animation_info().is_some(),
                icc_profile: matches!(
                    p.color_info(),
                    Some(zenavif_parse::ColorInformation::IccProfile(_))
                ),
                // zenavif-parse 0.6.2 misreads the frame header of the reduced
                // still-picture headers most encoders write (imazen/zenavif#46,
                // fixed on its main after 0.6.2): a libavif lossless file reads
                // as base_q_idx 32, lossless false. Not trusted until a release
                // with the fix.
                lossless: None,
                jpeg_quality: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimg;

    fn png(img: &image::DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn jpeg_header() {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 72)
            .encode_image(&testimg::gradient_rgb(40, 30))
            .unwrap();
        let i = inspect(&out).unwrap();
        assert_eq!((i.format, i.width, i.height), (Format::Jpeg, 40, 30));
        assert!(!i.gray && !i.has_alpha && !i.animated);
        assert_eq!(i.lossless, Some(false));
        assert_eq!(i.jpeg_quality, Some(72));
        assert!(i.within_limits());
    }

    #[test]
    fn png_header_gray_alpha_and_depth() {
        let gray = image::DynamicImage::ImageLuma8(testimg::gray_noise(9, 7));
        let i = inspect(&png(&gray)).unwrap();
        assert_eq!((i.width, i.height, i.bit_depth), (9, 7, 8));
        assert!(i.gray && !i.has_alpha);

        let rgba16 = image::DynamicImage::ImageRgba8(testimg::gradient_rgba(5, 4)).to_rgba16();
        let i = inspect(&png(&image::DynamicImage::ImageRgba16(rgba16))).unwrap();
        assert!(!i.gray && i.has_alpha);
        assert_eq!(i.bit_depth, 16);
        assert_eq!(i.lossless, Some(true));
    }

    #[test]
    fn webp_header() {
        let img = testimg::gradient_rgb(33, 17);
        let cfg = zenwebp::LossyConfig::new().with_quality(80.0);
        let lossy = zenwebp::EncodeRequest::lossy(&cfg, &img, zenwebp::PixelLayout::Rgb8, 33, 17)
            .encode()
            .unwrap();
        let i = inspect(&lossy).unwrap();
        assert_eq!((i.format, i.width, i.height), (Format::WebP, 33, 17));
        assert_eq!(i.lossless, Some(false));

        let cfg = zenwebp::LosslessConfig::new();
        let lossless =
            zenwebp::EncodeRequest::lossless(&cfg, &img, zenwebp::PixelLayout::Rgb8, 33, 17)
                .encode()
                .unwrap();
        assert_eq!(inspect(&lossless).unwrap().lossless, Some(true));
    }

    #[test]
    fn jxl_header() {
        let img = testimg::gradient_rgb(21, 13);
        let jxl = zenjxl::LossyConfig::new(1.0)
            .encode(&img, 21, 13, zenjxl::PixelLayout::Rgb8)
            .unwrap();
        let i = inspect(&jxl).unwrap();
        assert_eq!((i.format, i.width, i.height), (Format::Jxl, 21, 13));
        assert!(!i.has_alpha && !i.animated);
        assert_eq!(i.lossless, Some(false));
    }

    fn avif(depth: zenravif::BitDepth) -> Vec<u8> {
        use rgb::FromSlice;
        let img = testimg::gradient_rgb(24, 18);
        zenravif::Encoder::new()
            .with_quality(70.0)
            .with_speed(10)
            .with_bit_depth(depth)
            .encode_rgb(imgref::Img::new(img.as_raw().as_rgb(), 24, 18))
            .unwrap()
            .avif_file
    }

    #[test]
    fn avif_header() {
        let i = inspect(&avif(zenravif::BitDepth::Eight)).unwrap();
        assert_eq!((i.format, i.width, i.height), (Format::Avif, 24, 18));
        assert!(!i.has_alpha && !i.animated && !i.gray);
        assert_eq!(i.bit_depth, 8);
        // zenravif's default stores 8-bit input at 10 bits, and the header
        // reports what is stored.
        assert_eq!(
            inspect(&avif(zenravif::BitDepth::Auto)).unwrap().bit_depth,
            10
        );
    }

    #[test]
    fn unrecognised_and_malformed() {
        assert_eq!(inspect(b"GIF89a......"), Err(Error::Unrecognized));
        let truncated = &png(&image::DynamicImage::ImageLuma8(testimg::gray_noise(4, 4)))[..12];
        assert!(matches!(
            inspect(truncated),
            Err(Error::Malformed {
                format: Format::Png,
                ..
            })
        ));
    }

    #[test]
    fn limits_from_the_header() {
        // A 1x1 gray PNG whose IHDR claims 20000 x 20000.
        let mut big = png(&image::DynamicImage::ImageLuma8(testimg::gray_noise(1, 1)));
        big[16..20].copy_from_slice(&20_000u32.to_be_bytes());
        big[20..24].copy_from_slice(&20_000u32.to_be_bytes());
        // Fix the IHDR CRC so the header still parses.
        let crc = crc32(&big[12..29]);
        big[29..33].copy_from_slice(&crc.to_be_bytes());
        let i = inspect(&big).unwrap();
        assert_eq!((i.width, i.height), (20_000, 20_000));
        assert!(!i.within_limits());
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
}
