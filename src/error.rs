//! What can go wrong, as values.

use std::fmt;

use crate::Format;
use crate::limits::{MAX_DECODED_BYTES, MAX_DIMENSION};

/// Why imagoverto could not read or convert an image.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The bytes are not a JPEG, PNG, WebP, JPEG XL or AVIF file.
    Unrecognized,
    /// The file has the format's signature but cannot be read.
    Malformed { format: Format, reason: String },
    /// The image is larger than the decode limits allow
    /// ([`MAX_DIMENSION`] per side, [`MAX_DECODED_BYTES`] decoded).
    TooLarge { width: u32, height: u32 },
    /// The image is animated. One frame of an animation is not the image, so
    /// it is refused rather than flattened.
    Animated { format: Format },
    /// The file is valid, but imagoverto does not convert this kind of image
    /// (a CMYK JPEG, a floating-point JPEG XL, AVIF input for now).
    Unsupported { format: Format, reason: String },
    /// The options cannot be honoured whatever the image: a JPEG target with
    /// no quality (JPEG has no lossless mode), a quality for PNG (PNG has no
    /// quality setting), a quality outside 1–100.
    InvalidOptions { reason: String },
    /// The image has transparency and the target cannot store it. Give a
    /// background colour to flatten onto, or choose a target with alpha.
    TransparencyNeedsBackground { target: Format },
    /// A lossless conversion would lower the bit depth: the image has more
    /// bits per sample than the target stores. Give a quality to accept the
    /// loss, or choose PNG or JPEG XL, which keep 16 bits.
    BitDepthTooHigh { bits: u8, target: Format },
    /// The image has more pixels than [`Options::max_pixels`](crate::Options)
    /// allows. Nothing was decoded.
    TooManyPixels {
        width: u32,
        height: u32,
        max_pixels: u64,
    },
    /// The conversion was stopped through [`Options::cancel`](crate::Options):
    /// cancelled, or past its deadline.
    Cancelled,
    /// A codec failed in a way that is a bug, not a property of the file (it
    /// panicked, or returned a buffer of the wrong size). Worth reporting.
    Internal { format: Format, reason: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unrecognized => f.write_str("not a JPEG, PNG, WebP, JPEG XL or AVIF image"),
            Error::Malformed { format, reason } => write!(f, "cannot read this {format}: {reason}"),
            Error::TooLarge { width, height } => write!(
                f,
                "the image is {width}x{height}, past the decode limits \
                 ({MAX_DIMENSION} pixels per side, {} MiB decoded)",
                MAX_DECODED_BYTES / (1024 * 1024)
            ),
            Error::Animated { format } => {
                write!(
                    f,
                    "this {format} is animated, and animated images are not converted"
                )
            }
            Error::Unsupported { format, reason } => {
                write!(f, "this {format} is not converted: {reason}")
            }
            Error::InvalidOptions { reason } => f.write_str(reason),
            Error::TransparencyNeedsBackground { target } => write!(
                f,
                "the image has transparency, which {target} cannot store; \
                 give a background colour to flatten it onto, or choose a format with alpha"
            ),
            Error::BitDepthTooHigh { bits, target } => write!(
                f,
                "the image has {bits} bits per sample and {target} stores 8, so a lossless \
                 conversion is impossible; give a quality to accept the loss, or choose PNG or JPEG XL"
            ),
            Error::TooManyPixels {
                width,
                height,
                max_pixels,
            } => write!(
                f,
                "the image is {width}x{height}, more than the {max_pixels} pixels allowed"
            ),
            Error::Cancelled => f.write_str("the conversion was stopped before it finished"),
            Error::Internal { format, reason } => {
                write!(f, "the {format} codec failed: {reason}")
            }
        }
    }
}

impl std::error::Error for Error {}
