# imagoverto

**Converts still images between WebP, AVIF, JPEG XL, JPEG and PNG**, in pure
Rust: a library, and a command-line tool for people and scripts who want to
convert images without writing code. The name is Latin: *imago*, image, and
*verto*, to turn.

> Part of the **veripublica** family. [epublift](https://github.com/veripublica/epublift)
> will use it for the images in EPUB books; imagoverto itself knows nothing
> about books.

Both directions matter equally. Some e-readers cannot show WebP, AVIF or JPEG
XL, and need a way back to JPEG or PNG. Others gain a lot from the newer
formats and need the way forward.

## Status

Working, and tested, but **not released**: nothing is on crates.io and there
are no binaries yet. The API may still change. Until it is stable, install it
from source (below).

## What it does, and what it does not

It converts **one image file into another**, and tells you what happened: the
sizes before and after, the quality used (or `lossless`), and anything you
should know, such as transparency flattened onto a background.

- **Without `--quality`, nothing is lost.** WebP, AVIF, JPEG XL and PNG are
  written in their lossless modes. A JPEG converted to JPEG XL keeps the
  original JPEG inside, so it can be restored bit for bit. JPEG has no
  lossless mode, so a JPEG target needs a quality.
- **`--quality` is one scale across formats**, 1–100, calibrated so that the
  same number gives about the same visual quality in WebP, AVIF, JPEG XL and
  JPEG. It is always lossy: 100 is the highest lossy quality, not lossless.
- **`--image-compression smaller | balanced | faster`** trades encoding time
  for file size. It never changes the visual quality, the pixels of a
  lossless conversion, or the width and height. See
  [Compression levels](#compression-levels).
- **Always on:**
  - Images larger than 16384 pixels per side, or 512 MiB once decoded, are
    refused before any pixel is allocated.
  - Animated images are refused rather than cut down to one frame.
  - Gray images stay gray.
  - Transparency is never dropped silently. Converting a transparent image
    to JPEG needs `--background`.
  - EXIF orientation is applied to the pixels.
  - ICC colour profiles are kept.
  - Other metadata is dropped, except in the JPEG → JPEG XL recompression.
- **Errors are values.** A codec that panics or fails comes back as an error,
  never as a crash of the program that called it. The exception is
  WebAssembly, where a panic aborts the instance: wasm cannot catch it.

It does **not**:

- pick a format for you;
- decide whether the result is worth keeping;
- resize images;
- work on whole books.

The output may be larger than the input; imagoverto reports that and leaves
the choice to you.

## Install

From source, with Rust 1.93 or later:

```sh
cargo install --git https://github.com/veripublica/imagoverto --locked
```

## Usage

```sh
imagoverto -i cover.webp --image-format jpeg --quality 85
imagoverto -i logo.png --image-format jpeg --quality 90 --background white
imagoverto -i photo.jpg --image-format jxl                  # lossless, JPEG kept inside
imagoverto -i photo.jpg --image-format avif --quality 80 --image-compression smaller
imagoverto inspect -i cover.webp                           # what is this file?
```

- **Output path:** the output goes beside the input as `<stem>.<ext>`. If
  the extension would not change, it is `<stem>_converted.<ext>`. Give `-o`
  for another path.
- **Overwriting:** an existing file is never replaced without `-f`, and the
  input is never modified.
- **One image per run;** there is no batch mode.
- **`inspect`** reports a file's format, size, alpha, bit depth, colour
  profile and, for a JPEG, the quality it was saved at. It then checks that
  the pixels actually decode.
- **Machine-readable output:** `--format json` prints one JSON object in the
  veripublica family's shared report format.
- **All options and the exit codes** are in `imagoverto convert --help` and
  `imagoverto inspect --help`.

### Compression levels

`balanced` is the default. `smaller` and `faster` are relative to it: a
smaller file for a longer encode, or a quicker encode for a larger file. Where
an encoder has nothing to trade, two levels use the same setting.

| Format | `smaller` | `balanced` | `faster` |
|---|---|---|---|
| AVIF | slowest, smallest setting | about 8× quicker; about 8% larger on photos, 38% on drawings | as `balanced` |
| JPEG XL | as `balanced` | — | about 4× quicker; 9–20% larger |
| WebP | as `balanced` | — | slightly quicker, about 1% larger |
| PNG | about 1% smaller, 2–3× slower | — | far quicker, about 30% larger |
| JPEG | no setting | no setting | no setting |

These are means over 60 photos and 60 drawings from e-books, at the same
visual quality. For lossless conversions the levels differ a little. For
example, lossless JPEG XL `smaller` is about 5% smaller than `balanced` and
takes 6× as long.

## Library

```toml
[dependencies]
imagoverto = { git = "https://github.com/veripublica/imagoverto", default-features = false }

# Needed until zenravif 0.2.0 is on crates.io (see below).
[patch.crates-io]
zenavif-serialize = { git = "https://github.com/imazen/zenavif", rev = "94116d266299f347cfd10a75d440a5bc3b598d2c" }
```

```rust
use imagoverto::{Compression, Format, Options, convert};

let input = std::fs::read("cover.webp")?;
let options = Options::new(Format::Avif)
    .with_quality(80)
    .with_compression(Compression::Balanced);
let out = convert(&input, &options)?;
std::fs::write("cover.avif", &out.bytes)?;
println!("{} → {} bytes, {:?}", out.input_size, out.bytes.len(), out.quality);
```

- **No CLI:** `default-features = false` leaves out the command-line tool and
  its JSON dependencies.
- **WebAssembly:** the library builds for `wasm32-unknown-unknown` and is
  tested on `wasm32-wasip1`.
- **Bounding the work:** for input you do not trust, such as on a server, two
  options bound how long one conversion can take:
  - `Options::with_max_pixels` refuses an image from its header, before
    anything is decoded.
  - `Options::with_cancel` takes a `Cancel` token. You can cancel it from
    another thread, or create it with a deadline (`Cancel::after`). The WebP
    and AVIF encoders stop within a fraction of a second. The JPEG XL encoder
    does not check the token yet, so a started JPEG XL encode runs to its
    end.
- **Threads:** `convert` keeps no state, so it can be called from several
  threads at once. That is how a batch of images uses more than one core.

### Why the patch

AVIF is encoded with [zenravif](https://github.com/imazen/cavif-rs) from git,
pinned to a reviewed commit, not with the 0.1.3 release on crates.io. That
release damages some real images and its lossless mode is not lossless. The
git version fixes both, but it depends on a crate that is not yet published
in the matching version. Cargo applies `[patch]` only from the top-level
project, so your project needs the same line. It goes away once zenravif 0.2.0
is published.

## Codecs

| Format | Read | Write |
|---|---|---|
| JPEG | [image](https://crates.io/crates/image) | image (baseline) |
| PNG | image | image |
| WebP | [zenwebp](https://crates.io/crates/zenwebp) | zenwebp |
| JPEG XL | [zenjxl](https://crates.io/crates/zenjxl) | zenjxl, jxl-encoder (JPEG recompression) |
| AVIF | [zenavif-parse](https://crates.io/crates/zenavif-parse) + [rav1d-safe](https://crates.io/crates/rav1d-safe) | [zenravif](https://github.com/imazen/cavif-rs) (git) |

Everything is Rust; nothing needs a C compiler. AVIF decoding is tested byte
for byte against dav1d.

## How it is measured

- **Where the numbers come from:** the quality calibration and the
  compression levels were measured on images from a private shelf of e-books.
  The books themselves are not in this repository and never will be. The
  numbers are.
- **Test images:** every test image is generated in code.
- **Measurement tools:** the tools that produced the numbers are in
  `examples/`. They are for imagoverto's maintainers and are not shipped.

## License

AGPL-3.0-only ([LICENSE](LICENSE)). Converting an image with imagoverto puts
no license on the result. See [LICENSING.md](LICENSING.md) for what this means
in practice, why there is no commercial option, and what imagoverto is built
on. Contributions: [CONTRIBUTING.md](CONTRIBUTING.md).
