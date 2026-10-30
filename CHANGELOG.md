# Changelog

All notable changes to `imagoverto` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
imagoverto is pre-1.0, so breaking changes land as minor-version bumps
(`0.x.0`), per [Cargo's SemVer compatibility
rules](https://doc.rust-lang.org/cargo/reference/semver.html).

## [Unreleased]

Nothing is released yet. The first release, 0.1.0, will contain:

### Added

- **`convert`:** one image in, one image out, between WebP, AVIF, JPEG XL,
  JPEG and PNG, with the facts about the result: sizes, the quality used or
  `lossless`, and findings such as transparency flattened onto a background.
- **Lossless without a quality.** Without a quality, WebP, AVIF, JPEG XL and
  PNG are written lossless, and JPEG → JPEG XL keeps the original JPEG,
  restorable bit for bit.
- **One quality scale.** A quality of 1–100 means the same across formats. It
  was calibrated with butteraugli against WebP on 60 photos and 60 drawings
  from e-books, separately for each compression level.
- **Compression levels:** `Compression` (`--image-compression smaller |
  balanced | faster`) trades encoding time for file size. It never changes
  the visual quality, the pixels of a lossless conversion, or the image's
  width and height.
- **`inspect`:** what a file's header says (format, size, alpha, bit depth,
  colour profile, JPEG quality), and whether its pixels decode.
- **Limits that are always on:**
  - 16384 pixels per side and 512 MiB decoded, checked before any pixel is
    allocated;
  - animated images refused;
  - gray kept gray;
  - transparency never dropped without `--background`;
  - EXIF orientation applied;
  - ICC profiles kept.
- **Bounding the work:** `Options::with_max_pixels` refuses large images from
  the header. `Cancel` (`Options::with_cancel`) stops a conversion that is
  under way, from another thread or by a deadline.
- **Command-line tool** (feature `cli`, on by default):
  - `convert` (the default subcommand) and `inspect`;
  - `--format json` in the veripublica family's report format.
- **Platforms:** the library builds for `wasm32-unknown-unknown` and is
  tested on `wasm32-wasip1`.

### Known limitations

- **AVIF encoder from git:** imagoverto takes `zenravif` from git, pinned,
  until 0.2.0 is on crates.io. A project that depends on imagoverto needs
  the `[patch.crates-io]` line from the README.
- **JPEG XL cancellation:** the JPEG XL encoder (jxl-encoder 0.3.4) does not
  check the cancellation token yet. A started JPEG XL encode runs to its end.
- **AVIF lossless flag:** `inspect` cannot yet tell whether an AVIF is
  lossless, and reports it as unknown (imazen/zenavif#46).
