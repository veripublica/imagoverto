//! How long converting every image of a book takes, per format: the number
//! that says how long a user waits for one book, and what more cores buy. A
//! measurement tool for imagoverto's maintainers, not a veripublica family
//! tool, and not shipped.
//!
//! ```text
//! cargo run --release --example booktime -- <BOOKS_DIR> [QUALITY]
//! ```
//!
//! `BOOKS_DIR` holds one directory per book with that book's images (any of
//! the five formats; other files are skipped). Each book is converted on one
//! thread, image after image, to WebP, JPEG and JPEG XL at `QUALITY`
//! (default 80); books run side by side on all cores but two. Prints one
//! line per book (images, megapixels, seconds per format), slowest last.
//! Only numbers are printed; nothing from the images is written anywhere.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use imagoverto::{Format, Options, convert, inspect};

// AVIF output returns with zenravif 0.2.0 (the AVIF column of the
// 2026-10-30 measurement used zenravif main at speed 4).
const FORMATS: [Format; 3] = [Format::WebP, Format::Jpeg, Format::Jxl];

struct Book {
    name: String,
    images: Vec<Vec<u8>>,
    megapixels: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first() else {
        eprintln!("usage: booktime <BOOKS_DIR> [QUALITY]");
        std::process::exit(2);
    };
    let quality: u8 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(80);

    let mut books: Vec<Book> = std::fs::read_dir(dir)
        .expect("BOOKS_DIR reads")
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| load(&e.path()))
        .filter(|b| !b.images.is_empty())
        .collect();
    // Largest first, so the slowest book does not start last.
    books.sort_by(|a, b| b.megapixels.total_cmp(&a.megapixels));

    let next = AtomicUsize::new(0);
    let threads =
        std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(2).max(1));
    let mut rows: Vec<(f64, String)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut rows = Vec::new();
                    while let Some(book) = books.get(next.fetch_add(1, Ordering::Relaxed)) {
                        rows.push(time_book(book, quality));
                    }
                    rows
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("a worker panicked"))
            .collect()
    });
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));

    eprintln!(
        "{} books, quality {quality}, {threads} threads",
        books.len()
    );
    println!("images      MP    webp_s    jpeg_s     jxl_s  book");
    for (_, line) in &rows {
        println!("{line}");
    }
}

fn load(dir: &Path) -> Book {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    let mut images = Vec::new();
    let mut megapixels = 0.0;
    for f in files {
        let Ok(bytes) = std::fs::read(&f) else {
            continue;
        };
        if let Ok(info) = inspect(&bytes) {
            megapixels += f64::from(info.width) * f64::from(info.height) / 1e6;
            images.push(bytes);
        }
    }
    let name = dir
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Book {
        name,
        images,
        megapixels,
    }
}

/// The book's line, keyed by its slowest format's seconds.
fn time_book(book: &Book, quality: u8) -> (f64, String) {
    let secs: Vec<f64> = FORMATS
        .iter()
        .map(|&f| {
            let opts = Options::new(f)
                .with_quality(quality)
                .with_background([255; 3]);
            let start = Instant::now();
            for img in &book.images {
                let _ = convert(img, &opts);
            }
            start.elapsed().as_secs_f64()
        })
        .collect();
    let line = format!(
        "{:>6} {:>7.1} {:>9.1} {:>9.1} {:>9.1}  {}",
        book.images.len(),
        book.megapixels,
        secs[0],
        secs[1],
        secs[2],
        book.name
    );
    (secs.iter().copied().fold(0.0, f64::max), line)
}
