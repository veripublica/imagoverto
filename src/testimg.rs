//! Synthetic test images. Every test image is generated here; no file from
//! anyone's library enters the repository.

use image::{GenericImageView, GrayImage, RgbImage, RgbaImage};

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

/// PSNR in dB between two images of the same size, over all channels of
/// their RGBA8 forms. 99 when identical.
pub fn psnr(a: &image::DynamicImage, b: &image::DynamicImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions());
    let (a, b) = (a.to_rgba8(), b.to_rgba8());
    let se: f64 = a
        .as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2))
        .sum();
    if se == 0.0 {
        return 99.0;
    }
    let mse = se / a.as_raw().len() as f64;
    10.0 * (255.0f64 * 255.0 / mse).log10()
}

/// A hand-built baseline JPEG of 8×8 mid-gray pixels with `components`
/// components (1, 3 or 4), each 1×1 sampled. No encoder at hand writes four
/// (CMYK); this does. Every block is a zero DC difference and an EOB, one
/// 1-bit code each.
pub fn jpeg_8x8(components: u8) -> Vec<u8> {
    let n = usize::from(components);
    let mut j = vec![0xFF, 0xD8];
    // One quantisation table, all ones.
    j.extend([0xFF, 0xDB, 0x00, 0x43, 0x00]);
    j.extend([1; 64]);
    // SOF0: 8-bit, 8×8.
    j.extend([
        0xFF,
        0xC0,
        0x00,
        (8 + 3 * n) as u8,
        8,
        0,
        8,
        0,
        8,
        components,
    ]);
    for c in 1..=components {
        j.extend([c, 0x11, 0]);
    }
    // DC and AC table 0, one 1-bit code each, for symbol 0.
    for class in [0x00, 0x10] {
        j.extend([0xFF, 0xC4, 0x00, 0x14, class, 1]);
        j.extend([0; 15]);
        j.push(0);
    }
    j.extend([0xFF, 0xDA, 0x00, (6 + 2 * n) as u8, components]);
    for c in 1..=components {
        j.extend([c, 0x00]);
    }
    j.extend([0, 63, 0]);
    // Two bits per block, padded with ones.
    let bits = 2 * n;
    j.extend(std::iter::repeat_n(0x00, bits / 8));
    if bits % 8 != 0 {
        j.push(0xFF >> (bits % 8));
    }
    j.extend([0xFF, 0xD9]);
    j
}
