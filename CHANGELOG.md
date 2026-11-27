# Changelog

All notable changes to `imagoverto` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
imagoverto is pre-1.0, so breaking changes land as minor-version bumps
(`0.x.0`), per [Cargo's SemVer compatibility
rules](https://doc.rust-lang.org/cargo/reference/semver.html).

## [0.1.4] - 2026-11-28

### Fixed

- **CMYK JPEGs are refused, as documented.** The check never fired for JPEG:
  `image` 0.25.10 reports a CMYK or YCCK JPEG as RGB, so it was converted to
  RGB without its profile (the colours shift) and the CMYK profile was
  written over the RGB pixels (557 KB in one WebP on a 562-book shelf). It is
  now recognised by its frame header and refused (`Unsupported`).
- **A profile is written only over pixels in its own colour space.** A gray
  profile over RGB output (every WebP, which has no gray mode) is replaced by
  the RGB profile that renders R = G = B as the gray one renders that gray,
  so a colour-managed viewer shows the same image. Exactly gray RGB under a
  gray profile now goes out as gray, with the same values. Gray pixels under
  an RGB profile are widened to RGB. A profile that fits no written pixels is
  dropped and reported as the new finding `IMG-009`
  (`convert.profile_dropped`, warning).

## [0.1.3] - 2026-11-27

### Changed

- **An sRGB ICC profile is no longer written.** Every output format reads an
  image without a profile as sRGB, so the profile only cost bytes (3,144 for
  the common IEC61966-2.1 one; on a 562-book shelf, 2.46 MB over 638 WebPs).
  A profile counts as sRGB when it is a plain RGB matrix/TRC profile with
  sRGB's colorants and a tone curve that gives back sRGB's value for every
  8-bit input; any other profile (Adobe RGB, Display P3, gamma 2.2, gray,
  CMYK, LUT-based) is kept as before. This includes the profile a JPEG XL
  decoder synthesises for an sRGB JPEG XL, which was written into every
  conversion from JPEG XL until now. The conversion reports it as the new
  finding `IMG-008` (`convert.srgb_profile_dropped`, info). JPEG → JPEG XL
  recompression keeps the original JPEG as before.
- **Gray with an sRGB profile goes out as gray.** An RGB image whose pixels
  are exactly gray was kept as RGB when it carried a profile; with an sRGB
  profile it is now stored as gray, with the same pixel values. A lossy WebP
  whose colour is codec noise around gray is now treated as gray (`IMG-005`)
  under an sRGB profile too.
- **Manifest:** `homepage` dropped from `imagoverto`'s `Cargo.toml`, where it
  equalled `repository`. crates.io shows the repository link either way.
  `imagoverto-wasm` keeps it for npm.

## [0.1.2] - 2026-11-23

A maintenance release: no change to conversion, options or output.

### Changed

- **Minimum Rust version: 1.95** (was 1.93), for both `imagoverto` and
  `imagoverto-wasm`. The whole veripublica family now builds with one
  toolchain, and some crates already in the lock (`windows-*` 0.100,
  `generator` 0.8.12) need 1.95.
- **Dependencies:** `Cargo.lock` brought to the veripublica family's shared
  versions (archmage 0.9.33, bumpalo 3.20.4, cc 1.11.3, pin-project-lite
  0.2.18, regex-automata 0.4.19, regex-syntax 0.8.12, smallvec 1.16.6,
  zerocopy 0.8.64 and others). hashbrown stays on 0.16: jxl-encoder and
  zenwebp, which pull it in, have no release on 0.17 yet.

### Known limitations

Unchanged from 0.1.0: no AVIF output, JPEG XL encodes cannot be cancelled,
and `inspect` reports an AVIF's lossless flag as unknown.

## [0.1.1] - 2026-11-11

A maintenance release: no change to conversion, options or output.

### Changed

- **Dependencies:** `Cargo.lock` brought to the veripublica family's shared
  versions (archmage 0.9.31, cc 1.11.1, crc32fast 1.5.5, flate2 1.1.11,
  log 0.4.35, smallvec 1.16.5, syn 3.0.10, yuv 0.8.20 and others). The
  minimum Rust version stays 1.93.
- **Documentation:** the detail (scope, command line, API, codecs, limits,
  measurements) moved from `README.md` to `AGENTS.md`, which ships in the
  crate. The README is now short.
- **Web page:** the veripublica family's page template v8.

### Known limitations

Unchanged from 0.1.0: no AVIF output, JPEG XL encodes cannot be cancelled,
and `inspect` reports an AVIF's lossless flag as unknown.

## [0.1.0] - 2026-10-31

The first release. It reads all five formats and writes four: **AVIF output
is not in this release**, because the only AVIF encoder imagoverto trusts
(zenravif) has its fixes only on git so far, and crates.io takes no git
dependency. It returns once zenravif 0.2.0 is published.

### Added

- **`convert`:** one image in, one image out. It reads WebP, AVIF, JPEG XL,
  JPEG and PNG, and writes WebP, JPEG XL, JPEG and PNG. It reports the facts
  about the result: sizes, the quality used or `lossless`, and findings such
  as transparency flattened onto a background. Asking for AVIF output is
  refused (`Error::InvalidOptions`; exit 2 on the command line).
- **Lossless without a quality.** Without a quality, WebP, JPEG XL and PNG
  are written lossless, and JPEG → JPEG XL keeps the original JPEG,
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
  - `--format json` in the veripublica family's report format;
  - binaries for Linux (glibc and musl), macOS and Windows, x86_64 and
    ARM64, with `SHA256SUMS.txt` and a build provenance attestation.
- **In the browser:** the npm package `@veripublica/imagoverto-wasm`
  (`convert`, `inspect`, `version`, with TypeScript types), and a web page
  that converts an image without uploading it.
- **Platforms:** the library builds for `wasm32-unknown-unknown` and is
  tested on `wasm32-wasip1`.

### Known limitations

- **No AVIF output** until zenravif 0.2.0 is on crates.io (above).
- **JPEG XL cancellation:** the JPEG XL encoder (jxl-encoder 0.3.4) does not
  check the cancellation token yet. A started JPEG XL encode runs to its end.
- **AVIF lossless flag:** `inspect` cannot yet tell whether an AVIF is
  lossless, and reports it as unknown (imazen/zenavif#46).
