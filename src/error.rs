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
        }
    }
}

impl std::error::Error for Error {}
