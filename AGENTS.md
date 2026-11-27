# imagoverto: detail for code agents and maintainers

`README.md` is the short introduction. This file holds the detail and is kept
true at every release. Past changes: `CHANGELOG.md`. API docs:
<https://docs.rs/imagoverto>.

Figures below name the versions they were measured on. Current version: 0.1.4.

## Scope

imagoverto converts one still image file into another: WebP, AVIF, JPEG XL,
JPEG and PNG. It ships as a CLI and library (crate `imagoverto`) and a
WebAssembly package (`@veripublica/imagoverto-wasm`), released together under
one version. The web page at <https://veripublica.github.io/imagoverto/> runs
the WebAssembly build; the image never leaves the browser.

Both directions have equal priority: readers that cannot show WebP, AVIF or
JPEG XL need a way back to JPEG or PNG, and
[epublift](https://github.com/ePubLift/epublift) will need the way forward.
imagoverto is an independent product and knows nothing about books.

Input is an image, a target format and an optional quality. Output is the new
file and the facts: bytes, the quality used or `lossless`, sizes, findings.

**0.1 reads AVIF but writes no AVIF.** The AVIF encoder, zenravif, has the fixes
it needs only on git (see [Codecs](#codecs)), and crates.io refuses a crate with
a git dependency. An AVIF target returns `InvalidOptions` in the library and
exit 2 in the CLI. AVIF output returns when zenravif 0.2.0 is on crates.io.

Rules the code follows:

- **No quality means lossless.** WebP, JPEG XL and PNG use their lossless
  modes. JPEG to JPEG XL recompresses losslessly: the original JPEG is kept
  inside and a decoder can return it byte for byte. A JPEG target without a
  quality is refused, because JPEG has no lossless mode.
- **A quality is always lossy.** 100 is the highest lossy quality, not lossless.
  PNG with a quality is refused.
- **Lossless never lowers bit depth.** 16 bits to an 8-bit format losslessly is
  refused. With a quality, it is allowed and reported (`IMG-004`). PNG and
  JPEG XL keep 16 bits, lossless or lossy.
- **Decode limits are always on:** 16384 pixels per side and 512 MiB decoded,
  checked before any pixel is allocated. No caller can raise them.
- **Animation is refused,** not cut down to one frame.
- **Gray stays gray.** It goes to the encoder as L8, except WebP (see
  [Codecs](#codecs)), which has no gray mode.
- **Alpha is never dropped silently.** Transparent to an alpha-less target is
  refused unless a background is given; the flattening is then reported.
- **Metadata:** ICC is kept, unless it describes sRGB: every output format
  reads an image without a profile as sRGB, so such a profile is not written
  and the conversion reports it (`IMG-008`). A profile is written only over
  pixels in its own colour space. A gray profile over RGB output (always so in
  WebP, which has no gray mode) becomes the RGB profile that renders R = G = B
  as the gray one renders that gray: its tone curve on all three channels,
  under sRGB's colorants. Exactly gray RGB under a gray profile goes out as
  gray. Gray pixels under an RGB profile are widened to RGB. A profile that
  fits no written pixels (Lab, a gray one with tables, an unreadable header)
  is dropped and reported (`IMG-009`). EXIF orientation is applied to the pixels. Other
  EXIF and XMP are dropped, except in JPEG to JPEG XL recompression, which keeps
  the file as it was.
- **Errors are values.** A codec panic is caught and returned as
  `Error::Internal`; it must never reach or hang the caller. The exception is
  WebAssembly, where a panic traps the instance.
- **No C toolchain.** Everything is Rust.

## What it declines to do

- **It only converts.** It has no `best` or "smallest" mode, picks no format,
  and does not decide whether a result is worth keeping. The output may be
  larger than the input; that is reported, not refused.
- **It does not resize.** `--image-compression smaller` means a smaller file,
  never smaller dimensions.
- **It does no EPUB-level work.** Book policies belong to epublift.
- **It does not handle video or animation.**
- **It has no batch mode:** one `-i` per run. `convert` keeps no state, so a
  batch uses shell loops or library calls from several threads.
- **It has no `--dry-run`, `-q`, `-v` or prompts.**

## Install

Binaries for {x86_64, aarch64} × {unknown-linux-gnu, unknown-linux-musl,
apple-darwin, pc-windows-msvc} are on the
[releases page](https://github.com/veripublica/imagoverto/releases), with
`SHA256SUMS.txt` and a build provenance attestation. Each archive,
`imagoverto-<triple>.tar.gz` or `.zip`, holds one folder `imagoverto-<triple>/`
with the binary, `LICENSE`, `LICENSING.md` and `README.md`.

```sh
cargo install imagoverto --locked   # Rust 1.95 or later
npm install @veripublica/imagoverto-wasm
```

## CLI

```
imagoverto [convert] -i <PATH> --image-format <FORMAT> [OPTIONS]
imagoverto inspect -i <PATH> [OPTIONS]
```

The CLI follows the family spec, [veripublica conventions
v0.7](https://github.com/veripublica/conventions): `CLI.md` and `FORMATS.md`.
`convert` is the default subcommand. There is exactly one `-i` and no positional
paths.

| Option | Applies to | Meaning |
| --- | --- | --- |
| `-i, --input <PATH>` | both | The input. |
| `--format <human\|json>` | both | Report format. Default `human`. |
| `-V, --version` | both | Print the version, with the git hash when built from a checkout. |
| `-h, --help` | both | Print help. |
| `-o, --output <PATH>` | convert | The output file. Default `<stem>.<ext>` beside the input, or `<stem>_converted.<ext>` when the extension would not change. An extension naming another format is a usage error. |
| `-f, --force` | convert | Allow replacing an existing output. Never lifts the output-equals-input refusal. |
| `--image-format <webp\|jxl\|jpeg\|png>` | convert | The format to encode to. Required. `avif` is recognised and refused in 0.1. |
| `--quality <1-100>` | convert | Encode lossily, on one scale across formats (see [Measurements](#measurements)). |
| `--image-compression <smaller\|balanced\|faster>` | convert | Encoding time against file size (see [Compression levels](#compression-levels)). Default `balanced`. |
| `--background <white\|black\|#RRGGBB>` | convert | Flatten transparency onto this colour; the output has no alpha. |

Exit codes:

| Command | Code | Meaning |
| --- | --- | --- |
| convert | 0 | The image was converted and written. |
| convert | 2 | Nothing was written: a usage error (including JPEG without `--quality`, or PNG with one), an image that cannot be read or converted, an output that is the input, an existing output without `-f`, or an I/O failure. |
| inspect | 0 | The image decodes. |
| inspect | 1 | The header reads, but the image does not decode, is past the limits, or is animated. |
| inspect | 2 | A usage error, a file that is not one of the five formats or whose header cannot be read, or an I/O failure. |

With `--format json`, an image that cannot be converted is still described in
the envelope, with `status: "error"`. Option refusals and the other failures
print no envelope. `inspect` also describes an unreadable image in the envelope.

### JSON report

`--format json` prints one object on stdout, the family's shared envelope:

```json
{
  "tool": "imagoverto",
  "tool_version": "0.1.4",
  "convention": "0.7",
  "status": "ok",
  "inputs": [{
    "path": "t.png",
    "status": "ok",
    "output": "t.webp",
    "summary": { "input_bytes": 166, "output_bytes": 54, "errors": 0, "warnings": 0, "infos": 0 },
    "items": [{
      "type": "operation",
      "outcome": "applied",
      "code": "IMG-000",
      "rule": "convert",
      "message": "converted PNG to WebP, lossless",
      "data": { "compression": "balanced", "from": "png", "to": "webp", "width": 8, "height": 8,
                "input_bytes": 166, "output_bytes": 54, "quality": "lossless" }
    }]
  }]
}
```

The envelope types are imagoverto's own, until the family moves the shared ones
to a crate.

| Code | Rule | Severity | Meaning |
| --- | --- | --- | --- |
| `IMG-000` | `convert` | | The operation itself. |
| `IMG-001` | `convert.lossy` | info | Lossy encode: the output differs from the input. |
| `IMG-002` | `convert.lossless_to_lossy` | warning | The input was lossless and the output is lossy. |
| `IMG-003` | `convert.flattened` | info | Transparency was flattened onto the background colour. |
| `IMG-004` | `convert.depth_reduced` | info | 16 bits per sample were reduced to 8. |
| `IMG-005` | `convert.near_gray` | info | Near-gray colour (lossy WebP noise) was encoded as gray. |
| `IMG-006` | `convert.jpeg_kept` | info | The original JPEG is kept inside the JPEG XL. |
| `IMG-007` | `convert.jpeg_not_kept` | warning | The original JPEG could not be kept; its decoded pixels were stored losslessly. |
| `IMG-008` | `convert.srgb_profile_dropped` | info | The ICC profile is sRGB, the default without one, so it was not written. |
| `IMG-009` | `convert.profile_dropped` | warning | The ICC profile does not describe the written pixels' colour space, so it was not written. |
| `IMG-100` | `inspect.header` | info | What the header says. |
| `IMG-101` | `inspect.not_decodable` | error | The image does not decode; the envelope's status is then `problems`. |

## API

```toml
[dependencies]
imagoverto = { version = "0.1", default-features = false }
```

`default-features = false` leaves out the CLI and its serde dependencies.

```rust
use imagoverto::{Compression, Format, Options, convert};

let input = std::fs::read("cover.avif")?;
let options = Options::new(Format::WebP)
    .with_quality(80)
    .with_compression(Compression::Balanced);
let out = convert(&input, &options)?;
std::fs::write("cover.webp", &out.bytes)?;
println!("{} → {} bytes, {:?}", out.input_size, out.bytes.len(), out.quality);
```

- `convert(&[u8], &Options) -> Result<Conversion, Error>`: `Conversion` holds
  `bytes`, `format`, `quality` (`Lossless` or `Lossy(u8)`), `compression`,
  `source` (the input's `Info`), `input_size` and `findings`, most severe first.
- `inspect(&[u8]) -> Result<Info, Error>` reads the header only.
  `decode_check(&[u8]) -> Result<Info, Error>` also decodes under the same rules
  as a conversion, and discards the pixels.
- `Info`: `format`, `width`, `height` (as stored, before rotation), `has_alpha`,
  `gray`, `bit_depth`, `animated`, `icc_profile`, `lossless` (`Option`),
  `jpeg_quality` (`Option`, the libjpeg quality the luminance table matches).
- `Options::check` tells whether options can be honoured for any image, without
  one.
- `Error`: `Unrecognized`, `Malformed`, `TooLarge`, `Animated`, `Unsupported`,
  `InvalidOptions`, `TransparencyNeedsBackground`, `BitDepthTooHigh`,
  `TooManyPixels`, `Cancelled`, `Internal`.
- `Format` has `sniff`, `name`, `from_name`, `extension`, `media_type` and
  `has_alpha`. `MAX_DIMENSION`, `MAX_DECODED_BYTES` and `VERSION` are public.

For input you do not trust, two options bound the work:

- `Options::with_max_pixels` refuses an image from its header, before anything
  is decoded. It is what bounds a JPEG XL encode.
- `Options::with_cancel` takes a `Cancel` token, cancelled from another thread
  or created with a deadline (`Cancel::after`). See [Limits](#limits).

### WebAssembly

The library builds for `wasm32-unknown-unknown` and is tested on
`wasm32-wasip1`. `imagoverto-wasm/` exports `convert`, `inspect` and `version`,
with option names matching the CLI's; `imagoverto-wasm/README.md` documents the
package. It is not on crates.io. Its JS objects are built by hand, and native
tests keep every error kind in the `.d.ts`. Run it in a worker: a conversion cannot be
interrupted from inside, and a panic traps the instance. The demo page starts a
new worker when a new option or file ends a running conversion.

## Codecs

| Format | Read | Write |
| --- | --- | --- |
| JPEG | `image` | `image` (baseline) |
| PNG | `image` | `image` |
| WebP | `zenwebp` | `zenwebp` |
| JPEG XL | `zenjxl` | `zenjxl`, `jxl-encoder` (JPEG recompression) |
| AVIF | `zenavif-parse` + `rav1d-safe` 0.6 | none in 0.1 |

`zenravif` is a dev-dependency only: the AVIF decoder's tests encode their input
with it. It is pinned to a git revision without its `asm` feature, and cargo
drops a version-less dev-dependency from the published crate, so users need no
`[patch]`. When zenravif 0.2.0 is on crates.io, it becomes a normal dependency
again, AVIF output is restored, the AVIF quality curves are refitted, and the
reference tests and measurements are re-run.

Traps found by measurement:

- **zenavif is not used.** It always pulls rav1d-safe, and 0.1.6 pins the 0.5
  line, which hangs and mis-decodes (imazen/zenavif#78). rav1d-safe 0.6.0 is the
  first that matches dav1d byte for byte.
- **AVIF decode runs on one thread:** no worker that could hang the caller, and
  it works on wasm.
- **zenravif 0.1.3:** `with_lossless` is lossy; `AlphaColorMode::Premultiplied`
  writes alpha 255 as about 1; `with_rotation` takes the irot code 0–3, not
  degrees; lossy output is visibly corrupt on some real images (avifenc is fine
  on the same images). All are fixed on upstream main, unpublished.
- **zenravif** tiles images over 1 MP and then uses the whole rayon pool unless
  `with_num_threads` is set. Output bytes do not depend on the thread count.
- **zenwebp 0.4.6:** lossy L8 is copied unscaled into limited-range Y, stretching
  and clipping tones (fixed on main only), and lossless L8 skips the full VP8L
  encoder (imazen/zenwebp#57). Gray goes in as RGB.
- **imir:** read as HEIF 2022 does (axis 0 is a top-bottom flip), as libavif and
  the browsers do. The docs of zenavif-parse and zenravif give the reverse.
- **zenavif-parse 0.6.2** misreads `base_q_idx` and lossless (imazen/zenavif#46,
  fixed on main only), so `inspect` reports AVIF `lossless` as unknown.
- **jxl-encoder 0.3.4** stores the cancellation token but never checks it
  (imazen/jxl-encoder#77; fixed on main in 0.4.0, unreleased).

Known `cargo audit` warnings, unmaintained and not vulnerabilities: `paste` (via
zenwebp, rav1d-safe) and `proc-macro-error2` (via zenjxl).

## Limits

- 16384 pixels per side and 512 MiB decoded. Chosen, not measured: comfortably
  above any real e-book image.
- **Cancellation:** imagoverto checks the token before decoding and before
  encoding, and the WebP encoder checks it as it works, stopping within about
  0.1 s. A started JPEG XL encode runs to its end. The JPEG and PNG encoders and
  the decoders do not check it.
- CMYK JPEGs, floating-point JPEG XL and HDR are refused (`Unsupported`).
  A CMYK (or YCCK) JPEG is recognised by its frame header's four components:
  `image` 0.25.10 reports it as RGB and converts it without the profile.
- `inspect` reports AVIF `lossless` as unknown.

## Measurements

All measured on 0.1.0's encoders, 2026-10-30, on 60 photos and 60 drawings from
a private shelf of e-books. The books are not in this repository and never will
be; the numbers are. `examples/calibrate.rs` re-checks them. Test images are
generated in code.

**Quality scale.** `--quality` is WebP's scale. For JPEG and JPEG XL, imagoverto
maps it to the setting whose mean butteraugli distance (3-norm) matches WebP's at
that quality, clamped to the fitted range of 45–95 (`src/encode.rs`).

- JPEG: photos match about 5 below the curve, drawings about 5 above.
- JPEG XL: the worst point is 0.14 from the curve at effort 5, 0.48 at effort 3.
- WebP method 2 needs no mapping of its own: it fits within 0.5 of the same
  quality.
- Two 60-image samples of the shelf differ by about 4 AVIF q at q80, so all
  levels of a format are refitted together, on one sample.

### Compression levels

`balanced` is the default. `smaller` and `faster` are relative to it, never
promises. No level changes the visual quality, the pixels of a lossless
conversion, or the dimensions; each level has its own quality fit.

| Format | Setting (`smaller` / `balanced` / `faster`) | `smaller` | `faster` |
| --- | --- | --- | --- |
| JPEG XL lossy | effort 5 / 5 / 3 | as `balanced` | about 4× quicker; 9–20% larger |
| JPEG XL lossless | effort 9 / 7 / 7 | about 5% smaller, 6× the time | as `balanced` |
| WebP lossy | method 4 / 4 / 2 | as `balanced` | slightly quicker, about 1% larger |
| WebP lossless | `-z` 9 / 6 / 2 | 2% smaller, 12–23× the time | |
| PNG | deflate Best / Default / Fast | about 1% smaller, 2–3× slower | far quicker, about 30% larger |
| JPEG | none | | |

Where a higher setting bought nothing, a level keeps the lower one: JPEG XL lossy
effort 5 is as small as 7 and 2.4× quicker, and WebP method 6 is no smaller than
4. JPEG XL lossless effort 6 and below is 40–80% larger than 7, larger than PNG
on drawings, so `faster` keeps 7.

## Layout

```
src/lib.rs             public API
src/main.rs            CLI (feature `cli`, default on); the parser is kepubverto's grammar
src/convert.rs         Options, Conversion, Finding, convert()
src/decode.rs          decoding under the limits, decode_check()
src/avif.rs            AVIF decode (zenavif-parse + rav1d-safe)
src/encode.rs          per-encoder settings, quality fits
src/inspect.rs         header facts
src/jpeg_quality.rs    a JPEG's libjpeg quality, from its luminance table
src/format.rs          Format, sniffing
src/limits.rs          decode limits
src/cancel.rs          Cancel
src/error.rs           Error
src/testimg.rs         generated test images
build.rs               stamps the git hash into VERSION
examples/              measurement tools, not shipped: calibrate.rs, booktime.rs
imagoverto-wasm/       WebAssembly bindings; demo/ is the web page
scripts/               bump.sh, preflight.sh, wait-runs.sh, release-verify.sh
```

In `imagoverto-wasm/demo/`, `tokens.css` and `demo.css` are unedited copies from
the family's web template; do not edit them.

## Build and test

```sh
cargo build --locked
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo check --lib --no-default-features --locked
cargo build --lib --target wasm32-unknown-unknown --locked
CARGO_TARGET_WASM32_WASIP1_RUNNER=wasmtime cargo test --lib --locked --target wasm32-wasip1 -- --test-threads=1
wasm-pack build imagoverto-wasm --target web --no-pack
python3 -m http.server -d imagoverto-wasm 8000   # then open /demo/
```

Reference tools in tests: `dav1d`, `djxl`, `avifenc`. A test whose tool is
missing passes with a note; CI installs them on Linux. AVIF planes are tested
byte for byte against dav1d, and there is a no-hang test. Test images are
generated in code; a real file, if ever needed, goes in `tests/fixtures/` with
its source and licence in `tests/fixtures/SOURCES.md`.

`scripts/preflight.sh X.Y.Z` runs every check before a release and ends in READY
or NOT READY.

## Contributing and licence

AGPL-3.0-only, with no commercial option, because the `zen*` codecs are
`AGPL-3.0-only OR LicenseRef-Imazen-Commercial`. `LICENSING.md` says what this
means in practice. Converting an image puts no licence on the result. External
pull requests are not accepted until a CLA exists (`CONTRIBUTING.md`).
