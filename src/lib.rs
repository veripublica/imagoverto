//! Converts still images between WebP, AVIF, JPEG XL, JPEG and PNG.
//!
//! Being built: today it recognises the five formats and reads their headers
//! ([`inspect`]).

mod error;
mod format;
mod inspect;
mod jpeg_quality;
mod limits;
#[cfg(test)]
mod testimg;

pub use error::Error;
pub use format::Format;
pub use inspect::{Info, inspect};
pub use limits::{MAX_DECODED_BYTES, MAX_DIMENSION};

/// The crate version, with a `+<short-hash>[.dirty]` suffix when built from a
/// git checkout (see `build.rs`).
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), env!("IMAGOVERTO_BUILD"));
