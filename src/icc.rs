//! Recognising an ICC profile that says nothing beyond sRGB, and fitting a
//! profile to the pixels it is written over.
//!
//! Every format imagoverto writes reads an image without a profile as sRGB,
//! so an sRGB profile only costs bytes (3,152 for the common IEC61966-2.1
//! one). A profile counts as sRGB when it is a plain RGB matrix/TRC profile
//! whose colorants are sRGB's and whose tone curves give back the same
//! 8-bit value as sRGB's for every input. Anything else (another gamut, a
//! gamma-2.2 curve, a LUT-based or CICP-tagged profile that is not sRGB, a
//! profile that does not parse) is kept.

use moxcms::{
    CicpColorPrimaries, ColorProfile, DataColorSpace, ToneReprCurve, TransferCharacteristics,
};

/// How far a colorant (D50-adapted XYZ) may be from sRGB's. Real sRGB
/// profiles differ from the computed colorants by about 1e-4 (s15Fixed16
/// rounding, slightly different adaptation matrices); Display P3's red is
/// 0.08 away.
const COLORANT_TOLERANCE: f64 = 0.002;

/// Whether `icc` describes sRGB, so that leaving it out changes no colour.
pub(crate) fn is_srgb(icc: &[u8]) -> bool {
    let Ok(p) = ColorProfile::new_from_slice(icc) else {
        return false;
    };
    if p.color_space != DataColorSpace::Rgb || p.pcs != DataColorSpace::Xyz {
        return false;
    }
    // A colour manager may prefer these tables over the matrix and curves.
    let luts = [
        &p.lut_a_to_b_perceptual,
        &p.lut_a_to_b_colorimetric,
        &p.lut_a_to_b_saturation,
        &p.lut_b_to_a_perceptual,
        &p.lut_b_to_a_colorimetric,
        &p.lut_b_to_a_saturation,
    ];
    if luts.iter().any(|l| l.is_some()) {
        return false;
    }
    // A CICP tag takes precedence over the rest where it is read.
    if let Some(c) = &p.cicp
        && (c.color_primaries != CicpColorPrimaries::Bt709
            || c.transfer_characteristics != TransferCharacteristics::Srgb)
    {
        return false;
    }
    let srgb = ColorProfile::new_srgb();
    let colorants = [
        (p.red_colorant, srgb.red_colorant),
        (p.green_colorant, srgb.green_colorant),
        (p.blue_colorant, srgb.blue_colorant),
    ];
    let close = colorants.iter().all(|(a, b)| {
        (a.x - b.x).abs() <= COLORANT_TOLERANCE
            && (a.y - b.y).abs() <= COLORANT_TOLERANCE
            && (a.z - b.z).abs() <= COLORANT_TOLERANCE
    });
    close
        && [&p.red_trc, &p.green_trc, &p.blue_trc]
            .iter()
            .all(|t| t.as_ref().is_some_and(is_srgb_curve))
}

/// The colour space an ICC profile's header names (bytes 16–19), whether or
/// not the rest of it parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Space {
    Rgb,
    Gray,
    /// CMYK, Lab, XYZ, n-colour, or no readable header.
    Other,
}

pub(crate) fn space(icc: &[u8]) -> Space {
    match icc.get(16..20) {
        Some(b"RGB ") => Space::Rgb,
        Some(b"GRAY") => Space::Gray,
        _ => Space::Other,
    }
}

/// What to write for a profile over pixels that are, or are not, gray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fit {
    /// The profile describes the pixels as they are.
    Keep,
    /// A gray profile over RGB pixels: this RGB profile means the same.
    Replace(Vec<u8>),
    /// No profile can be written that means what this one does.
    Drop,
}

/// A profile is written only over pixels in its own colour space: a viewer
/// may ignore a mismatched one or misrender the image with it.
pub(crate) fn fit(icc: &[u8], gray_pixels: bool) -> Fit {
    match (space(icc), gray_pixels) {
        (Space::Rgb, false) | (Space::Gray, true) => Fit::Keep,
        (Space::Gray, false) => rgb_from_gray(icc).map_or(Fit::Drop, Fit::Replace),
        _ => Fit::Drop,
    }
}

/// The RGB profile that renders R = G = B as `icc`, a gray matrix/TRC
/// profile, renders that gray: its tone curve on all three channels, under
/// sRGB's colorants, which add up to the PCS white as the gray axis does.
/// `None` for a gray profile with tables, or one that does not parse.
fn rgb_from_gray(icc: &[u8]) -> Option<Vec<u8>> {
    let mut p = ColorProfile::new_from_slice(icc).ok()?;
    if p.color_space != DataColorSpace::Gray || p.pcs != DataColorSpace::Xyz {
        return None;
    }
    let luts = [
        &p.lut_a_to_b_perceptual,
        &p.lut_a_to_b_colorimetric,
        &p.lut_a_to_b_saturation,
        &p.lut_b_to_a_perceptual,
        &p.lut_b_to_a_colorimetric,
        &p.lut_b_to_a_saturation,
    ];
    if luts.iter().any(|l| l.is_some()) {
        return None;
    }
    let trc = p.gray_trc.take()?;
    let srgb = ColorProfile::new_srgb();
    p.color_space = DataColorSpace::Rgb;
    // Gray press profiles (Dot Gain 20%) are output profiles; a matrix/TRC
    // RGB profile has to be a display (or input) one.
    p.profile_class = moxcms::ProfileClass::DisplayDevice;
    p.red_colorant = srgb.red_colorant;
    p.green_colorant = srgb.green_colorant;
    p.blue_colorant = srgb.blue_colorant;
    p.red_trc = Some(trc.clone());
    p.green_trc = Some(trc.clone());
    p.blue_trc = Some(trc);
    p.cicp = None;
    p.encode().ok()
}

/// Whether decoding every 8-bit value through `curve` and encoding it again
/// with sRGB's curve gives back the same 8-bit value.
fn is_srgb_curve(curve: &ToneReprCurve) -> bool {
    (0..=255u8).all(|v| {
        let x = f64::from(v) / 255.0;
        eval(curve, x).is_some_and(|lin| (srgb_encode(lin) * 255.0 - f64::from(v)).abs() < 0.5)
    })
}

/// The linear value `curve` gives for `x` in 0..=1, `None` for a malformed
/// curve.
fn eval(curve: &ToneReprCurve, x: f64) -> Option<f64> {
    let y = match curve {
        ToneReprCurve::Lut(t) => match t.len() {
            0 => x,
            // u8Fixed8 gamma.
            1 => x.powf(f64::from(t[0]) / 256.0),
            n => {
                let pos = x * (n - 1) as f64;
                let i = (pos.floor() as usize).min(n - 2);
                let f = pos - i as f64;
                let (a, b) = (f64::from(t[i]), f64::from(t[i + 1]));
                (a + (b - a) * f) / 65535.0
            }
        },
        // ICC parametric curve types 0–4, gamma first.
        ToneReprCurve::Parametric(p) => {
            let p: Vec<f64> = p.iter().map(|&v| f64::from(v)).collect();
            match p.as_slice() {
                [g] => x.powf(*g),
                [g, a, b] => {
                    if x >= -b / a {
                        (a * x + b).max(0.0).powf(*g)
                    } else {
                        0.0
                    }
                }
                [g, a, b, c] => {
                    if x >= -b / a {
                        (a * x + b).max(0.0).powf(*g) + c
                    } else {
                        *c
                    }
                }
                [g, a, b, c, d] => {
                    if x >= *d {
                        (a * x + b).max(0.0).powf(*g)
                    } else {
                        c * x
                    }
                }
                [g, a, b, c, d, e, f] => {
                    if x >= *d {
                        (a * x + b).max(0.0).powf(*g) + e
                    } else {
                        c * x + f
                    }
                }
                _ => return None,
            }
        }
    };
    y.is_finite().then_some(y)
}

/// sRGB's encoding (inverse EOTF), 0..=1.
fn srgb_encode(lin: f64) -> f64 {
    let lin = lin.clamp(0.0, 1.0);
    if lin <= 0.003_130_8 {
        12.92 * lin
    } else {
        1.055 * lin.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_curve(mut p: ColorProfile, curve: ToneReprCurve) -> Vec<u8> {
        p.red_trc = Some(curve.clone());
        p.green_trc = Some(curve.clone());
        p.blue_trc = Some(curve);
        p.encode().unwrap()
    }

    /// The common IEC61966-2.1 profile's shape: a 1024-entry table and the
    /// colorants it stores (0.4361 0.2225 0.0139, …), no CICP.
    fn table_srgb() -> Vec<u8> {
        let table = (0..1024)
            .map(|i| {
                let x = f64::from(i) / 1023.0;
                let lin = if x <= 0.04045 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                };
                (lin * 65535.0).round() as u16
            })
            .collect();
        let mut p = ColorProfile::new_srgb();
        p.cicp = None;
        p.red_colorant = moxcms::Xyzd::new(0.4361, 0.2225, 0.0139);
        p.green_colorant = moxcms::Xyzd::new(0.3851, 0.7169, 0.0971);
        p.blue_colorant = moxcms::Xyzd::new(0.1431, 0.0606, 0.7141);
        with_curve(p, ToneReprCurve::Lut(table))
    }

    #[test]
    fn srgb_profiles_are_recognised() {
        assert!(is_srgb(&ColorProfile::new_srgb().encode().unwrap()));
        assert!(is_srgb(&table_srgb()));
    }

    #[test]
    fn other_profiles_are_kept() {
        for p in [
            ColorProfile::new_display_p3(),
            ColorProfile::new_adobe_rgb(),
            ColorProfile::new_bt2020(),
            ColorProfile::new_pro_photo_rgb(),
            ColorProfile::new_gray_with_gamma(2.2),
        ] {
            assert!(!is_srgb(&p.encode().unwrap()), "{:?}", p.description);
        }
        // sRGB's primaries with a plain gamma of 2.2: darker shadows.
        let mut gamma = ColorProfile::new_srgb();
        gamma.cicp = None;
        let gamma = with_curve(gamma, ToneReprCurve::Parametric(vec![2.2]));
        assert!(!is_srgb(&gamma));
        assert!(!is_srgb(b"not a profile"));
        assert!(!is_srgb(&[]));
    }

    /// Each gray value through `gray`, and the same value as R = G = B
    /// through `rgb`, into sRGB: within one 8-bit step.
    fn renders_like(gray: &[u8], rgb: &[u8]) {
        use moxcms::{Layout, TransformOptions};
        let (gray, rgb) = (
            ColorProfile::new_from_slice(gray).unwrap(),
            ColorProfile::new_from_slice(rgb).unwrap(),
        );
        let srgb = ColorProfile::new_srgb();
        let opts = TransformOptions::default();
        let g = gray
            .create_transform_8bit(Layout::Gray, &srgb, Layout::Rgb, opts)
            .unwrap();
        let r = rgb
            .create_transform_8bit(Layout::Rgb, &srgb, Layout::Rgb, opts)
            .unwrap();
        let src: Vec<u8> = (0..=255).collect();
        let wide: Vec<u8> = src.iter().flat_map(|&v| [v; 3]).collect();
        let (mut a, mut b) = (vec![0; 768], vec![0; 768]);
        g.transform(&src, &mut a).unwrap();
        r.transform(&wide, &mut b).unwrap();
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert!(x.abs_diff(*y) <= 1, "value {}: {x} vs {y}", i / 3);
        }
    }

    #[test]
    fn profiles_fit_their_pixels() {
        let rgb = ColorProfile::new_display_p3().encode().unwrap();
        let gray = ColorProfile::new_gray_with_gamma(2.2).encode().unwrap();
        assert_eq!(fit(&rgb, false), Fit::Keep);
        assert_eq!(fit(&gray, true), Fit::Keep);
        assert_eq!(fit(&rgb, true), Fit::Drop);
        let Fit::Replace(wide) = fit(&gray, false) else {
            panic!("a gray profile over RGB pixels is replaced");
        };
        assert_eq!(space(&wide), Space::Rgb);
        renders_like(&gray, &wide);
        // CMYK, Lab and anything unreadable describe no pixels written.
        let mut cmyk = rgb.clone();
        cmyk[16..20].copy_from_slice(b"CMYK");
        for p in [&cmyk[..], b"not a profile", &[]] {
            assert_eq!(fit(p, false), Fit::Drop);
            assert_eq!(fit(p, true), Fit::Drop);
        }
    }

    /// The macOS gray profiles, when present: profiles imagoverto did not
    /// make, with tone curves that are not a plain gamma.
    #[test]
    fn system_gray_profiles_widen_to_rgb() {
        let dir = std::path::Path::new("/System/Library/ColorSync/Profiles");
        for name in [
            "Generic Gray Gamma 2.2 Profile.icc",
            "Generic Gray Profile.icc",
        ] {
            if let Ok(gray) = std::fs::read(dir.join(name)) {
                let Fit::Replace(wide) = fit(&gray, false) else {
                    panic!("{name}: not replaced");
                };
                renders_like(&gray, &wide);
            }
        }
    }

    /// The macOS system profiles, when present: a check against profiles
    /// imagoverto did not make.
    #[test]
    fn system_profiles() {
        let dir = std::path::Path::new("/System/Library/ColorSync/Profiles");
        for (name, srgb) in [
            ("sRGB Profile.icc", true),
            ("Display P3.icc", false),
            ("AdobeRGB1998.icc", false),
            ("ITU-2020.icc", false),
            ("Generic Gray Gamma 2.2 Profile.icc", false),
        ] {
            if let Ok(icc) = std::fs::read(dir.join(name)) {
                assert_eq!(is_srgb(&icc), srgb, "{name}");
            }
        }
    }
}
