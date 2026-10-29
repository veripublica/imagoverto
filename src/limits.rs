//! The decode limits, checked before any pixel is allocated.
//!
//! Chosen, not measured: "comfortably above any real e-book image" (epublift's
//! input hardening, cli 1.x). Always on; no caller can raise them.

/// The largest width or height accepted.
pub const MAX_DIMENSION: u32 = 16_384;

/// The most memory a decoded image may take.
pub const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;

/// Whether an image of this size, at `bytes_per_pixel` once decoded, is
/// within the limits.
pub(crate) fn within(width: u32, height: u32, bytes_per_pixel: u64) -> bool {
    width != 0
        && height != 0
        && width <= MAX_DIMENSION
        && height <= MAX_DIMENSION
        && u64::from(width) * u64::from(height) * bytes_per_pixel <= MAX_DECODED_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        assert!(within(MAX_DIMENSION, 1, 4));
        assert!(!within(MAX_DIMENSION + 1, 1, 4));
        assert!(!within(0, 10, 4));
        // 16384 x 8192 RGBA8 is exactly 512 MiB; one more row is over.
        assert!(within(16_384, 8_192, 4));
        assert!(!within(16_384, 8_193, 4));
        // 16-bit RGBA halves what fits.
        assert!(!within(16_384, 8_192, 8));
    }
}
