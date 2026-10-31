//! WebAssembly bindings for [`imagoverto`]: convert a still image in the
//! browser. The bytes never leave the page.
//!
//! ```js
//! import init, { convert, inspect } from "imagoverto-wasm";
//! await init();
//! const bytes = new Uint8Array(await file.arrayBuffer());
//! const r = convert(bytes, { format: "webp", quality: 80 });
//! r.data;      // Uint8Array, the new file → save as <name>.<r.ext>
//! r.quality;   // 80, or "lossless" when no quality was given
//! r.findings;  // [{ code: "IMG-001", rule, severity, message }]
//! inspect(bytes); // the header facts, nothing decoded
//! ```

#![warn(missing_docs)]

use imagoverto::{Compression, Error, Finding, Format, Info, Options, Quality, Severity};
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

/// The shapes `convert` and `inspect` take, return and throw, for the
/// generated `.d.ts`. The objects are built by hand in [`conversion`],
/// [`info`] and [`error`]; native tests keep every name listed here.
// Read by the macro below at build time and by a native test; rustc's
// dead-code pass sees neither use on wasm32.
#[allow(dead_code)]
const TS_TYPES: &str = r##"
/** A format imagoverto reads. `"avif"` is read but not written in this release. */
export type FormatName = "webp" | "avif" | "jxl" | "jpeg" | "png";

/** How to convert: the CLI's options, by the same names. */
export interface ConvertOptions {
  /** The format to encode to (the CLI's --image-format). `"avif"` throws InvalidOptions in this release. */
  format: FormatName;
  /** 1–100, one scale across formats (WebP's). Always lossy; leave it out for a lossless conversion, which JPEG cannot do. */
  quality?: number;
  /** Encoding time against file size. Never changes what the image looks like. Default "balanced". */
  compression?: "smaller" | "balanced" | "faster";
  /** Flatten transparency onto this colour: "white", "black" or "#RRGGBB". Needed for a transparent image to JPEG. */
  background?: string;
  /** Refuse an image with more pixels than this (width × height), read from its header before anything is decoded. */
  maxPixels?: number;
}

/** What an image's header says. Nothing is decoded to read it. */
export interface Info {
  format: FormatName;
  /** As stored, before any EXIF or irot rotation. */
  width: number;
  height: number;
  hasAlpha: boolean;
  gray: boolean;
  bitDepth: number;
  animated: boolean;
  iccProfile: boolean;
  /** Where the header says; null when it does not tell (and for AVIF). */
  lossless: boolean | null;
  /** For a JPEG, the libjpeg quality its luminance table matches. */
  jpegQuality: number | null;
  /** Whether the image is within the decode limits every conversion enforces. */
  withinLimits: boolean;
}

/** Something the conversion did that the caller should know about. */
export interface Finding {
  /** "IMG-001" … "IMG-007", as the CLI's --format json carries them. */
  code: string;
  rule: string;
  severity: "info" | "warning";
  message: string;
}

/** The result of a conversion. The output may be larger than the input: that is a fact, not a failure. */
export interface Conversion {
  /** The new file's bytes. */
  data: Uint8Array;
  format: FormatName;
  mediaType: string;
  /** The usual extension, without the dot ("jpg" for JPEG). */
  ext: string;
  /** The quality encoded at, or "lossless". */
  quality: number | "lossless";
  compression: "smaller" | "balanced" | "faster";
  inputSize: number;
  outputSize: number;
  /** The input's header facts. */
  source: Info;
  /** Most severe first. */
  findings: Finding[];
}

/** Why an image could not be read or converted; the `kind` of a thrown `ImagovertoError`. */
export type ErrorKind =
  | "Unrecognized"
  | "Malformed"
  | "TooLarge"
  | "Animated"
  | "Unsupported"
  | "InvalidOptions"
  | "TransparencyNeedsBackground"
  | "BitDepthTooHigh"
  | "TooManyPixels"
  | "Cancelled"
  | "Internal";

/** What `convert` and `inspect` throw: an `Error` whose `kind` names the reason. */
export interface ImagovertoError extends Error {
  kind: ErrorKind;
}
"##;

#[wasm_bindgen(typescript_custom_section)]
const TS_SECTION: &str = TS_TYPES;

/// The crate version, with the git hash it was built from.
#[wasm_bindgen]
pub fn version() -> String {
    imagoverto::VERSION.to_string()
}

/// Convert an image. `options` takes the CLI's options by the same names:
/// `format` (required), `quality`, `compression`, `background`, `maxPixels`.
/// Throws an `ImagovertoError`: an `Error` whose `kind` names the reason and
/// whose `message` says it in English.
#[wasm_bindgen(unchecked_return_type = "Conversion")]
pub fn convert(
    input: &[u8],
    #[wasm_bindgen(unchecked_param_type = "ConvertOptions")] options: JsValue,
) -> Result<JsValue, JsValue> {
    let get = |key: &str| Reflect::get(&options, &key.into()).unwrap_or(JsValue::UNDEFINED);
    let present = |v: JsValue| (!v.is_undefined() && !v.is_null()).then_some(v);
    let opts = parse_options(&Raw {
        format: get("format").as_string(),
        quality: present(get("quality")).map(|v| v.as_f64().unwrap_or(f64::NAN)),
        compression: present(get("compression")).map(|v| v.as_string().unwrap_or_default()),
        background: present(get("background")).map(|v| v.as_string().unwrap_or_default()),
        max_pixels: present(get("maxPixels")).map(|v| v.as_f64().unwrap_or(f64::NAN)),
    })
    .map_err(|e| error(&e))?;
    let c = imagoverto::convert(input, &opts).map_err(|e| error(&e))?;
    let out = Object::new();
    set(&out, "data", &Uint8Array::from(c.bytes.as_slice()).into());
    set(&out, "format", &c.format.name().into());
    set(&out, "mediaType", &c.format.media_type().into());
    set(&out, "ext", &c.format.extension().into());
    let quality = match c.quality {
        Quality::Lossless => JsValue::from("lossless"),
        Quality::Lossy(q) => JsValue::from(q),
    };
    set(&out, "quality", &quality);
    set(&out, "compression", &compression_name(c.compression).into());
    set(&out, "inputSize", &JsValue::from(c.input_size as f64));
    set(&out, "outputSize", &JsValue::from(c.bytes.len() as f64));
    set(&out, "source", &info(&c.source));
    let findings = Array::new();
    for f in &c.findings {
        findings.push(&finding(*f));
    }
    set(&out, "findings", &findings);
    Ok(out.into())
}

/// Read an image's header: format, size, alpha, depth, and so on. Nothing is
/// decoded. Throws an `ImagovertoError` (`Unrecognized`, `Malformed`).
#[wasm_bindgen(unchecked_return_type = "Info")]
pub fn inspect(input: &[u8]) -> Result<JsValue, JsValue> {
    imagoverto::inspect(input)
        .map(|i| info(&i))
        .map_err(|e| error(&e))
}

/// The options as they came from JavaScript, before they are checked.
struct Raw {
    format: Option<String>,
    quality: Option<f64>,
    compression: Option<String>,
    background: Option<String>,
    max_pixels: Option<f64>,
}

/// The library's options, or the [`Error::InvalidOptions`] the CLI would
/// give for the same values (it reports those as usage errors).
fn parse_options(raw: &Raw) -> Result<Options, Error> {
    let invalid = |reason: String| Error::InvalidOptions { reason };
    let name = raw
        .format
        .as_deref()
        .ok_or_else(|| invalid("format is required: webp, jxl, jpeg or png".into()))?;
    let format = Format::from_name(name).ok_or_else(|| {
        invalid(format!(
            "invalid format '{name}'; supported values: webp, jxl, jpeg, png"
        ))
    })?;
    let mut opts = Options::new(format);
    if let Some(q) = raw.quality {
        if !(q.fract() == 0.0 && (1.0..=100.0).contains(&q)) {
            return Err(invalid(
                "the quality must be a whole number from 1 to 100".into(),
            ));
        }
        opts = opts.with_quality(q as u8);
    }
    if let Some(c) = &raw.compression {
        let level = match c.as_str() {
            "smaller" => Compression::Smaller,
            "balanced" => Compression::Balanced,
            "faster" => Compression::Faster,
            _ => {
                return Err(invalid(format!(
                    "invalid compression '{c}'; supported values: smaller, balanced, faster"
                )));
            }
        };
        opts = opts.with_compression(level);
    }
    if let Some(b) = &raw.background {
        let rgb = parse_colour(b).ok_or_else(|| {
            invalid(format!(
                "invalid background '{b}'; supported values: white, black, #RRGGBB"
            ))
        })?;
        opts = opts.with_background(rgb);
    }
    if let Some(n) = raw.max_pixels {
        if !(n.fract() == 0.0 && n >= 1.0 && n <= u64::MAX as f64) {
            return Err(invalid(
                "maxPixels must be a whole number of at least 1".into(),
            ));
        }
        opts = opts.with_max_pixels(n as u64);
    }
    opts.check()?;
    Ok(opts)
}

/// `white`, `black`, or `#RRGGBB` (hex, either case), as the CLI's
/// `--background`.
fn parse_colour(s: &str) -> Option<[u8; 3]> {
    match s {
        "white" => Some([255; 3]),
        "black" => Some([0; 3]),
        _ => {
            let hex = s.strip_prefix('#')?;
            if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            Some([byte(0)?, byte(2)?, byte(4)?])
        }
    }
}

fn compression_name(c: Compression) -> &'static str {
    match c {
        Compression::Smaller => "smaller",
        Compression::Faster => "faster",
        _ => "balanced",
    }
}

fn info(i: &Info) -> JsValue {
    let o = Object::new();
    let opt = |v: Option<JsValue>| v.unwrap_or(JsValue::NULL);
    set(&o, "format", &i.format.name().into());
    set(&o, "width", &i.width.into());
    set(&o, "height", &i.height.into());
    set(&o, "hasAlpha", &i.has_alpha.into());
    set(&o, "gray", &i.gray.into());
    set(&o, "bitDepth", &i.bit_depth.into());
    set(&o, "animated", &i.animated.into());
    set(&o, "iccProfile", &i.icc_profile.into());
    set(&o, "lossless", &opt(i.lossless.map(Into::into)));
    set(&o, "jpegQuality", &opt(i.jpeg_quality.map(Into::into)));
    set(&o, "withinLimits", &i.within_limits().into());
    o.into()
}

fn finding(f: Finding) -> JsValue {
    let o = Object::new();
    set(&o, "code", &f.code().into());
    set(&o, "rule", &f.rule().into());
    let severity = match f.severity() {
        Severity::Warning => "warning",
        Severity::Info => "info",
    };
    set(&o, "severity", &severity.into());
    set(&o, "message", &f.message().into());
    o.into()
}

/// The name of an error's kind, as the `kind` property carries it.
fn kind(e: &Error) -> &'static str {
    match e {
        Error::Unrecognized => "Unrecognized",
        Error::Malformed { .. } => "Malformed",
        Error::TooLarge { .. } => "TooLarge",
        Error::Animated { .. } => "Animated",
        Error::Unsupported { .. } => "Unsupported",
        Error::InvalidOptions { .. } => "InvalidOptions",
        Error::TransparencyNeedsBackground { .. } => "TransparencyNeedsBackground",
        Error::BitDepthTooHigh { .. } => "BitDepthTooHigh",
        Error::TooManyPixels { .. } => "TooManyPixels",
        Error::Cancelled => "Cancelled",
        _ => "Internal",
    }
}

fn error(e: &Error) -> JsValue {
    let err = js_sys::Error::new(&e.to_string());
    set(&err, "kind", &kind(e).into());
    err.into()
}

fn set(target: &JsValue, key: &str, value: &JsValue) {
    // Cannot fail on a plain object or Error.
    let _ = Reflect::set(target, &key.into(), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(format: &str) -> Raw {
        Raw {
            format: Some(format.into()),
            quality: None,
            compression: None,
            background: None,
            max_pixels: None,
        }
    }

    fn refused(r: &Raw) -> String {
        match parse_options(r) {
            Err(Error::InvalidOptions { reason }) => reason,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn options_parse_as_the_cli_does() {
        let o = parse_options(&Raw {
            quality: Some(80.0),
            compression: Some("faster".into()),
            background: Some("#FfFfFf".into()),
            max_pixels: Some(4e6),
            ..raw("jpeg")
        })
        .unwrap();
        assert_eq!(o.format, Format::Jpeg);
        assert_eq!(o.quality, Some(80));
        assert_eq!(o.compression, Compression::Faster);
        assert_eq!(o.background, Some([255; 3]));
        assert_eq!(o.max_pixels, Some(4_000_000));
        assert_eq!(parse_options(&raw("png")).unwrap().quality, None);
    }

    #[test]
    fn bad_options_are_refused() {
        assert!(
            refused(&Raw {
                format: None,
                ..raw("")
            })
            .contains("required")
        );
        assert!(refused(&raw("gif")).contains("webp, jxl, jpeg, png"));
        assert!(refused(&raw("avif")).contains("AVIF output"));
        assert!(refused(&raw("jpeg")).contains("JPEG has no lossless mode"));
        for q in [0.0, 101.0, 80.5, f64::NAN] {
            let r = Raw {
                quality: Some(q),
                ..raw("webp")
            };
            assert!(refused(&r).contains("1 to 100"), "{q}");
        }
        let r = Raw {
            compression: Some("smallest".into()),
            ..raw("webp")
        };
        assert!(refused(&r).contains("smaller, balanced, faster"));
        for b in ["red", "#fff", "#gggggg", ""] {
            let r = Raw {
                background: Some(b.into()),
                ..raw("jpeg")
            };
            assert!(refused(&r).contains("#RRGGBB"), "{b}");
        }
        let r = Raw {
            max_pixels: Some(0.0),
            ..raw("webp")
        };
        assert!(refused(&r).contains("maxPixels"));
    }

    #[test]
    fn every_kind_is_in_the_typescript_types() {
        let all = [
            Error::Unrecognized,
            Error::Malformed {
                format: Format::Png,
                reason: String::new(),
            },
            Error::TooLarge {
                width: 0,
                height: 0,
            },
            Error::Animated {
                format: Format::WebP,
            },
            Error::Unsupported {
                format: Format::Jpeg,
                reason: String::new(),
            },
            Error::InvalidOptions {
                reason: String::new(),
            },
            Error::TransparencyNeedsBackground {
                target: Format::Jpeg,
            },
            Error::BitDepthTooHigh {
                bits: 16,
                target: Format::WebP,
            },
            Error::TooManyPixels {
                width: 0,
                height: 0,
                max_pixels: 0,
            },
            Error::Cancelled,
            Error::Internal {
                format: Format::Png,
                reason: String::new(),
            },
        ];
        for e in &all {
            let k = format!("\"{}\"", kind(e));
            assert!(TS_TYPES.contains(&k), "{k} missing from ErrorKind");
        }
        for name in [
            "smaller", "balanced", "faster", "webp", "jxl", "jpeg", "png", "avif",
        ] {
            assert!(TS_TYPES.contains(&format!("\"{name}\"")), "{name}");
        }
    }
}
