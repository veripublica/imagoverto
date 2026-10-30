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
//! WebP's mean distance at each quality, at the default compression level,
//! is the anchor. For JPEG, and for WebP, AVIF and JPEG XL at every
//! compression level, it reports the mean distance imagoverto's *calibrated*
//! encode reaches at the same quality, against the anchor, to check each
//! mapping: a level must change the size, not the quality.
//!
//! The JPEG mapping itself was fitted with this tool's first version (commit
//! 70f466f), when `jpeg_quality` still passed the quality through: it
//! interpolated, on a grid of raw JPEG qualities, the one matching each WebP
//! anchor. To re-fit, make `jpeg_quality` a passthrough again and use that
//! version.
//!
//! Only numbers are printed; nothing from the images is written anywhere.

use std::path::{Path, PathBuf};

use imagoverto::{Compression, Format, Options, convert};
use rgb::FromSlice;

const WEBP_GRID: [u8; 6] = [45, 55, 65, 75, 85, 95];

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

    // The anchor first, then JPEG (which has no levels), then each level.
    let mut columns = vec![
        ("webp", Format::WebP, Compression::Balanced),
        ("jpeg", Format::Jpeg, Compression::Balanced),
    ];
    for (name, level) in [
        ("smaller", Compression::Smaller),
        ("balanced", Compression::Balanced),
        ("faster", Compression::Faster),
    ] {
        for (f, format) in [
            ("webp", Format::WebP),
            ("avif", Format::Avif),
            ("jxl", Format::Jxl),
        ] {
            if !(f == "webp" && name == "balanced") {
                columns.push((leak(format!("{f}/{name}")), format, level));
            }
        }
    }
    let jobs: Vec<(Format, Compression, u8)> = WEBP_GRID
        .iter()
        .flat_map(|&q| columns.iter().map(move |&(_, f, l)| (f, l, q)))
        .collect();
    let means = par_means(&sources, &jobs, |&(format, level, q), (rgb, png)| {
        let opts = Options::new(format).with_quality(q).with_compression(level);
        let c = convert(png, &opts).ok()?;
        let back = to_rgb(&c.bytes)?;
        Some((distance(rgb, &back)?, c.bytes.len() as f64))
    });

    // One line per column: the mean distance at each quality, then the size.
    println!(
        "{:<14} {}",
        "dist",
        WEBP_GRID.map(|q| format!("{q:>7}")).join("")
    );
    for (c, (name, ..)) in columns.iter().enumerate() {
        let row = (0..WEBP_GRID.len()).map(|i| means[i * columns.len() + c]);
        let d: String = row.clone().map(|(d, _)| format!("{d:>7.3}")).collect();
        let kb: String = row.map(|(_, kb)| format!("{kb:>7.1}")).collect();
        println!("{name:<14} {d}   KB {kb}");
    }
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
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

/// The mean (distance, KB) of `f` over `sources` for each job, on all cores
/// but two.
/// Every (job, image) pair is one task, largest images first, so no core
/// waits for a slow image at the end of each job; `None`s are skipped.
fn par_means<J: Sync>(
    sources: &[(image::RgbImage, Vec<u8>)],
    jobs: &[J],
    f: impl Fn(&J, &(image::RgbImage, Vec<u8>)) -> Option<(f64, f64)> + Sync,
) -> Vec<(f64, f64)> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut by_size: Vec<usize> = (0..sources.len()).collect();
    by_size.sort_by_key(|&i| std::cmp::Reverse(sources[i].0.len()));
    let tasks: Vec<(usize, usize)> = by_size
        .iter()
        .flat_map(|&img| (0..jobs.len()).map(move |job| (job, img)))
        .collect();
    let next = AtomicUsize::new(0);
    // Every core but two, which stay free for the system (Baris, 2026-10-30).
    let threads =
        std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(2).max(1));
    let mut totals = vec![(0.0, 0.0, 0usize); jobs.len()];
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut part = vec![(0.0, 0.0, 0usize); jobs.len()];
                    while let Some(&(job, img)) = tasks.get(next.fetch_add(1, Ordering::Relaxed)) {
                        if let Some((d, b)) = f(&jobs[job], &sources[img]) {
                            let t = &mut part[job];
                            *t = (t.0 + d, t.1 + b, t.2 + 1);
                        }
                    }
                    part
                })
            })
            .collect();
        for w in workers {
            for (t, p) in totals.iter_mut().zip(w.join().expect("a worker panicked")) {
                *t = (t.0 + p.0, t.1 + p.1, t.2 + p.2);
            }
        }
    });
    totals
        .into_iter()
        .map(|(d, b, n)| (d / n as f64, b / n as f64 / 1024.0))
        .collect()
}
