//! Synthetic test images. Every test image is generated here; no file from
//! anyone's library enters the repository.

use image::{GrayImage, RgbImage, RgbaImage};

/// R ramps with x, G with y, B is 128 (the generator of imazen/zenavif#78).
pub fn gradient_rgb(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([
            (x * 255 / (w - 1).max(1)) as u8,
            (y * 255 / (h - 1).max(1)) as u8,
            128,
        ])
    })
}

/// [`gradient_rgb`] with alpha ramping along x, from transparent to opaque.
pub fn gradient_rgba(w: u32, h: u32) -> RgbaImage {
    let rgb = gradient_rgb(w, h);
    RgbaImage::from_fn(w, h, |x, y| {
        let [r, g, b] = rgb.get_pixel(x, y).0;
        image::Rgba([r, g, b, (x * 255 / (w - 1).max(1)) as u8])
    })
}

/// Uniform gray noise from a fixed LCG (the generator of imazen/zenavif#78).
pub fn gray_noise(w: u32, h: u32) -> GrayImage {
    let mut s: u32 = 0x1234_5678;
    GrayImage::from_fn(w, h, |_, _| {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        image::Luma([(s >> 24) as u8])
    })
}
