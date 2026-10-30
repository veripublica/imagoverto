//! Measures `--quality` across formats: is quality N in one format as good as
//! quality N in WebP, the reference scale? A measurement tool for
//! imagoverto's maintainers, not a veripublica family tool, and not shipped.
//!
//! ```text
//! cargo run --release --example calibrate -- <IMAGE_DIR> [MAX_IMAGES]
//! ```
//!
//! For each image under `IMAGE_DIR` (JPEG, PNG, WebP, JPEG XL, AVIF; searched
//! recursively), it decodes the source once as the reference, encodes it at a
//! grid of qualities, decodes the result back and measures the butteraugli
//! distance to the reference (3-norm, the score epublift's calibration used).
//! WebP's mean distance at each quality is the anchor. It then reports:
//!
//! - **JPEG**: the raw JPEG quality whose mean distance matches each WebP
//!   anchor, and a quadratic fit of that mapping (what `jpeg_quality` in
//!   `src/encode.rs` should be);
//! - **AVIF and JPEG XL**: the mean distance imagoverto's *calibrated* encode
//!   reaches at each quality, against WebP's, to check the existing mapping.
//!
//! Only numbers are printed; nothing from the images is written anywhere.

use std::path::{Path, PathBuf};

use imagoverto::{Format, Options, convert};
use rgb::FromSlice;

const WEBP_GRID: [u8; 6] = [45, 55, 65, 75, 85, 95];
const JPEG_GRID: [u8; 14] = [10, 20, 30, 40, 50, 60, 65, 70, 75, 80, 85, 90, 95, 98];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first() else {
        eprintln!("usage: calibrate <IMAGE_DIR> [MAX_IMAGES]");
        std::process::exit(2);
    };
    let max: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);

    let mut files = Vec::new();
    collect(Path::new(dir), &mut files);
    files.sort();
    // An even spread, not the first N: one book's images sit together.
    let step = (files.len() / max).max(1);
    let files: Vec<PathBuf> = files.into_iter().step_by(step).take(max).collect();

    let mut sources = Vec::new();
    for f in &files {
        let Ok(bytes) = std::fs::read(f) else {
            continue;
        };
        if let Some(rgb) = to_rgb(&bytes)
            && rgb.width() >= 8
            && rgb.height() >= 8
        {
            // Re-encode the reference as PNG so every codec starts from the
            // same pixels, whatever the source format.
            let png = png_of(&rgb);
            sources.push((rgb, png));
        }
    }
    eprintln!(
        "{} images measured (of {} found)",
        sources.len(),
        files.len()
    );
    if sources.is_empty() {
        std::process::exit(1);
    }

    let mean = |format: Format, q: u8| -> (f64, f64) {
        let (mut sum, mut bytes, mut n) = (0.0, 0.0, 0usize);
        for (rgb, png) in &sources {
            let Ok(c) = convert(png, &Options::new(format).with_quality(q)) else {
                continue;
            };
            let Some(back) = to_rgb(&c.bytes) else {
                continue;
            };
            if let Some(d) = distance(rgb, &back) {
                sum += d;
                bytes += c.bytes.len() as f64;
                n += 1;
            }
        }
        (sum / n as f64, bytes / n as f64 / 1024.0)
    };

    let jpeg: Vec<(u8, f64)> = JPEG_GRID
        .iter()
        .map(|&q| (q, mean(Format::Jpeg, q).0))
        .collect();

    println!("webp_q  webp_dist  webp_KB | jpeg_q(match) | avif_dist  avif_KB | jxl_dist  jxl_KB");
    let mut pairs = Vec::new();
    for q in WEBP_GRID {
        let (w, wkb) = mean(Format::WebP, q);
        let (a, akb) = mean(Format::Avif, q);
        let (x, xkb) = mean(Format::Jxl, q);
        let jq = match_quality(&jpeg, w);
        if let Some(jq) = jq {
            pairs.push((f64::from(q), jq));
        }
        println!(
            "{q:>6}  {w:>9.3}  {wkb:>7.1} | {:>13} | {a:>9.3}  {akb:>7.1} | {x:>8.3}  {xkb:>6.1}",
            jq.map_or("out of range".into(), |v| format!("{v:.1}")),
        );
    }
    if let Some((a, b, c)) = quadratic_fit(&pairs) {
        println!(
            "\njpeg_q ≈ {a:.6}·q² + {b:.5}·q + {c:.3}   (fit over {} anchors)",
            pairs.len()
        );
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if let Ok(b) = std::fs::read(&p).map(|b| b[..b.len().min(64)].to_vec())
            && Format::sniff(&b).is_some()
        {
            out.push(p);
        }
    }
}

/// Decode any of the five formats through imagoverto (lossless to PNG), then
/// to 8-bit RGB.
fn to_rgb(bytes: &[u8]) -> Option<image::RgbImage> {
    let png = convert(bytes, &Options::new(Format::Png).with_background([255; 3])).ok()?;
    Some(image::load_from_memory(&png.bytes).ok()?.to_rgb8())
}

fn png_of(rgb: &image::RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    rgb.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("PNG encodes");
    out
}

fn distance(a: &image::RgbImage, b: &image::RgbImage) -> Option<f64> {
    if a.dimensions() != b.dimensions() {
        return None;
    }
    let (w, h) = (a.width() as usize, a.height() as usize);
    let ia = imgref::Img::new(a.as_raw().as_rgb(), w, h);
    let ib = imgref::Img::new(b.as_raw().as_rgb(), w, h);
    let r = butteraugli::butteraugli(ia, ib, &butteraugli::ButteraugliParams::new()).ok()?;
    r.pnorm(3.0)
}

/// The quality on `curve` (quality, distance; distance falls as quality
/// rises) whose distance is `target`, interpolated linearly.
fn match_quality(curve: &[(u8, f64)], target: f64) -> Option<f64> {
    curve.windows(2).find_map(|w| {
        let ((q0, d0), (q1, d1)) = (w[0], w[1]);
        let (lo, hi) = (d1.min(d0), d1.max(d0));
        (lo <= target && target <= hi && d0 != d1)
            .then(|| f64::from(q0) + (target - d0) / (d1 - d0) * f64::from(q1 - q0))
    })
}

/// Least-squares y ≈ a·x² + b·x + c.
fn quadratic_fit(pts: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    if pts.len() < 3 {
        return None;
    }
    let s = |k: i32| pts.iter().map(|(x, _)| x.powi(k)).sum::<f64>();
    let t = |k: i32| pts.iter().map(|(x, y)| y * x.powi(k)).sum::<f64>();
    let m = [
        [s(4), s(3), s(2)],
        [s(3), s(2), s(1)],
        [s(2), s(1), pts.len() as f64],
    ];
    let v = [t(2), t(1), t(0)];
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-12 {
        return None;
    }
    let col = |i: usize| {
        let mut c = m;
        for r in 0..3 {
            c[r][i] = v[r];
        }
        det(c) / d
    };
    Some((col(0), col(1), col(2)))
}
