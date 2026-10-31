# imagoverto-wasm

WebAssembly bindings for [imagoverto](https://github.com/veripublica/imagoverto):
convert a still image to WebP, JPEG XL, JPEG or PNG in the browser, or in any
JavaScript runtime. It reads WebP, AVIF, JPEG XL, JPEG and PNG. The bytes
never leave the page.

Live: <https://veripublica.github.io/imagoverto/>

```sh
npm install @veripublica/imagoverto-wasm
```

```js
import { convert, inspect } from "@veripublica/imagoverto-wasm"; // bundler build
// or, without a bundler: wasm-pack build --target web, then
// import init, { convert } from "./pkg/imagoverto_wasm.js"; await init();
const bytes = new Uint8Array(await file.arrayBuffer());

const r = convert(bytes, { format: "webp", quality: 80 });
// r.data: Uint8Array, the new file; save it as <name>.<r.ext>
// r.quality: 80, or "lossless" when no quality was given
// r.inputSize, r.outputSize, r.source (the input's header facts)
// r.findings: [{ code: "IMG-001", rule, severity: "info" | "warning", message }]

convert(bytes, { format: "jxl" });                      // lossless; a JPEG is kept inside
convert(bytes, { format: "jpeg", quality: 85, background: "white" });
convert(bytes, { format: "png", compression: "smaller" });

inspect(bytes); // { format, width, height, hasAlpha, bitDepth, jpegQuality, … }, nothing decoded
```

- **The options are the CLI's**, by the same names: `format` (required),
  `quality` (1–100, one scale across formats; leave it out for lossless),
  `compression` (`"smaller"`, `"balanced"`, `"faster"`), `background`
  (`"white"`, `"black"`, `"#RRGGBB"`), and `maxPixels`.
- **Typed.** The `.d.ts` declares `ConvertOptions`, `Conversion`, `Info`,
  `Finding`, `ErrorKind` and `ImagovertoError`.
- **Errors throw** an `ImagovertoError`: an `Error` whose `kind` names the
  reason (`Unrecognized`, `Animated`, `TransparencyNeedsBackground`,
  `InvalidOptions`, …).
- **No AVIF output in 0.1.** `format: "avif"` throws `InvalidOptions`: the
  AVIF encoder imagoverto uses is not on crates.io yet. AVIF input is read.
- **Run it in a worker.** A large image takes seconds, and a conversion
  cannot be interrupted from inside; ending the worker stops it. A panic
  cannot be caught in WebAssembly either: it traps the instance. The demo
  page runs each conversion in a worker and starts a new one when needed.
- Licence: AGPL-3.0-only (`LICENSE`, in the package).
- The demo page (`demo/`) uses family-web's template v8 (family-web v0.8.0);
  `tokens.css` and `demo.css` are unedited copies.

Build and serve locally:

```sh
wasm-pack build imagoverto-wasm --target web --no-pack
python3 -m http.server -d imagoverto-wasm 8000   # then open /demo/
```
