//! Converts still images between WebP, AVIF, JPEG XL, JPEG and PNG.
//!
//! [`convert`] turns one image into another; [`inspect`] reads what a file's
//! header says. Being built: AVIF input is refused for now.

mod convert;
mod decode;
mod encode;
mod error;
mod format;
mod inspect;
mod jpeg_quality;
mod limits;
#[cfg(test)]
mod testimg;

pub use convert::{Conversion, Finding, Options, Quality, Severity, convert};
pub use error::Error;
pub use format::Format;
pub use inspect::{Info, inspect};
pub use limits::{MAX_DECODED_BYTES, MAX_DIMENSION};

/// The crate version, with a `+<short-hash>[.dirty]` suffix when built from a
/// git checkout (see `build.rs`).
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), env!("IMAGOVERTO_BUILD"));
