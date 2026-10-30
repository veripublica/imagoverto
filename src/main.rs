//! imagoverto's CLI, following the **veripublica CLI convention v0.7**
//! (<https://github.com/veripublica/conventions>).
//!
//! `convert` (the default) is a *transformer*: exactly one input, a converted
//! copy written beside it, never in place and never over an existing file
//! without `-f`. `inspect` reads and reports. Neither asks anything, so there
//! are no prompts.
//!
//! The argument grammar is kepubverto's (itself epubsana's), ported
//! deliberately: one family, one parser, one set of surprises.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use imagoverto::{
    Compression, Conversion, Error, Finding, Format, Info, Options, Quality, Severity,
};
use serde::Serialize;

const HELP: &str = "\
imagoverto — convert still images between WebP, AVIF, JPEG XL, JPEG and PNG

USAGE:
    imagoverto [convert] -i <PATH> --image-format <FORMAT> [OPTIONS]
    imagoverto inspect -i <PATH> [OPTIONS]

SUBCOMMANDS:
    convert     Convert one image (the default). See imagoverto convert --help.
    inspect     Report what an image is and whether it decodes. See
                imagoverto inspect --help.

OPTIONS:
    -V, --version           Print imagoverto <version> to stdout and exit 0.
    -h, --help              Print this help to stdout and exit 0.

EXAMPLES:
    imagoverto -i cover.webp --image-format jpeg --quality 85   # writes cover.jpg
    imagoverto -i scan.png --image-format jxl                   # lossless
    imagoverto inspect -i cover.avif

Conforms to veripublica conventions v0.7.";

const CONVERT_HELP: &str = "\
imagoverto convert — convert one image to another format

USAGE:
    imagoverto [convert] -i <PATH> --image-format <FORMAT> [OPTIONS]

OPTIONS:
    -i, --input <PATH>      The input. The only input form; positional paths are
                            not accepted.
    -o, --output <PATH>     Where to write the output. Defaults to
                            <input-stem>.<ext> beside the input, or
                            <input-stem>_converted.<ext> when the extension
                            would not change.
    -f, --force             Permit replacing existing output files. Never lifts
                            the output-equals-input refusal.
        --image-format <webp|avif|jxl|jpeg|png>
                            The format to encode to. Required.
        --quality <1-100>   Encode lossily at this quality, on one scale across
                            formats. Without it the conversion is lossless:
                            WebP, AVIF, JPEG XL and PNG in their lossless
                            modes, and JPEG to JPEG XL keeps the original JPEG
                            inside. JPEG needs a quality; PNG takes none.
        --image-compression <smaller|balanced|faster>
                            Encoding time against file size: smaller files
                            take longer, faster encodes give larger files.
                            The visual quality, the pixels of a lossless
                            conversion and the image's width and height stay
                            the same. Default: balanced. JPEG has no such
                            setting.
        --background <white|black|#RRGGBB>
                            Flatten transparency onto this colour; the output
                            has no alpha. Needed to convert a transparent
                            image to JPEG.
        --format <FORMAT>   Report format: human (the default), or json: the
                            shared machine envelope (one JSON object on stdout,
                            see the veripublica FORMATS spec).
    -V, --version           Print imagoverto <version> to stdout and exit 0.
    -h, --help              Print this help to stdout and exit 0.

EXAMPLES:
    imagoverto -i cover.webp --image-format jpeg --quality 85
    imagoverto -i logo.png --image-format jpeg --quality 90 --background white
    imagoverto -i photo.jpg --image-format jxl    # lossless, JPEG kept inside
    imagoverto -i photo.jpg --image-format jpeg --quality 70 -o small.jpg

The original is never modified, and an existing output file is never silently
replaced (use -f). The output may be larger than the input; imagoverto reports
the sizes and leaves the choice to you. Animated images are not converted.

EXIT CODES:
    0   the image was converted and written.
    2   imagoverto could not convert: a usage error (including a JPEG target
        without --quality, or PNG with one), an image it cannot read or
        convert (not one of the five formats, animated, transparent to JPEG
        without --background, 16 bits to an 8-bit format losslessly, CMYK,
        HDR), an output path that is the input, an existing output file
        without -f, or an I/O failure. Nothing is written. With --format json,
        an image that cannot be converted is still described in the envelope;
        the rest print no envelope.

Conforms to veripublica conventions v0.7.";

const INSPECT_HELP: &str = "\
imagoverto inspect — report what an image is and whether it decodes

USAGE:
    imagoverto inspect -i <PATH> [OPTIONS]

OPTIONS:
    -i, --input <PATH>      The input. The only input form; positional paths are
                            not accepted.
        --format <FORMAT>   Report format: human (the default), or json: the
                            shared machine envelope (one JSON object on stdout,
                            see the veripublica FORMATS spec).
    -V, --version           Print imagoverto <version> to stdout and exit 0.
    -h, --help              Print this help to stdout and exit 0.

It reports the format (from the file's signature, not its name), size, alpha,
gray, bit depth, animation, colour profile, whether the encoding is lossless
where the header tells, and for a JPEG the quality it was saved at. It then
decodes the pixels, under the same limits a conversion uses.

EXAMPLES:
    imagoverto inspect -i cover.webp
    imagoverto inspect -i cover.webp --format json

EXIT CODES:
    0   the image decodes.
    1   the header reads, but the image does not decode (or is past the
        decode limits, or animated); the reason is reported.
    2   imagoverto could not run: a usage error, a file that is not one of the
        five formats or whose header cannot be read, or an I/O failure. With
        --format json, an unreadable image is still described in the envelope.

Conforms to veripublica conventions v0.7.";

/// The convention version this CLI and its envelope implement (FORMATS §1.1):
/// asserted here, never inherited.
const CONVENTION: &str = "0.7";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sub {
    Convert,
    Inspect,
}

/// The outcome of parsing `argv`, decided entirely before any work is done.
#[derive(Debug, PartialEq)]
enum Cli {
    /// `-h`/`--help` (short-circuits everything else), for this subcommand,
    /// or for the tool when none was named.
    Help(Option<Sub>),
    Version,
    /// The short problem message, without the `error:` prefix or the
    /// `--help` pointer main adds.
    Usage(String),
    Convert(ConvertRun),
    Inspect(InspectRun),
}

#[derive(Debug, PartialEq)]
struct ConvertRun {
    input: String,
    output: Option<String>,
    force: bool,
    options: Options,
    json: bool,
}

#[derive(Debug, PartialEq)]
struct InspectRun {
    input: String,
    json: bool,
}

/// Parse the arguments after the program name into a [`Cli`] decision.
///
/// The accepted syntaxes are the convention's (§3.3): `--name value` and
/// `--name=value`; `-i value` and the attached `-ivalue`; boolean short flags
/// bundle (`-fh`); a value-taking short flag consumes the rest of its token, or
/// the next token, as its value (POSIX: `-iv` means `-i v`); and the token after
/// a value-taking option is *always* its value, never re-parsed as an option
/// (`-i -q.png` names the file `-q.png`). The only bare word is a subcommand,
/// and only as the first argument.
fn parse(args: &[String]) -> Cli {
    let (sub, named, args) = match args.first().map(String::as_str) {
        Some("convert") => (Sub::Convert, true, &args[1..]),
        Some("inspect") => (Sub::Inspect, true, &args[1..]),
        _ => (Sub::Convert, false, args),
    };
    let mut inputs: Vec<String> = Vec::new();
    let mut output: Option<String> = None;
    let mut format: Option<String> = None;
    let mut image_format: Option<String> = None;
    let mut quality: Option<String> = None;
    let mut background: Option<String> = None;
    let mut compression: Option<String> = None;
    let mut force = false;
    let mut help = false;
    let mut version = false;
    let mut error: Option<String> = None;

    // Record the first usage error but keep scanning, so a later `-h` can still
    // short-circuit a malformed line (§5). Help wins over any error below.
    macro_rules! fail {
        ($($a:tt)*) => {{ if error.is_none() { error = Some(format!($($a)*)); } }};
    }
    // Assign a value to a single-valued option, rejecting a second answer (§3.4).
    macro_rules! set_single {
        ($slot:expr, $name:literal, $value:expr) => {{
            if $slot.is_some() {
                fail!(concat!("option '", $name, "' given more than once"));
            } else {
                $slot = Some($value);
            }
        }};
    }
    // An option that exists, but not on this subcommand.
    let convert_only = |name: &str| -> Option<String> {
        (sub == Sub::Inspect).then(|| format!("option '{name}' is for convert, not inspect"))
    };

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            // Accepted and ignored; the convention gives it no other meaning.
        } else if let Some(long) = arg.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            match name {
                "help" => help = true,
                "version" => version = true,
                "force" => match convert_only("--force") {
                    Some(m) => fail!("{m}"),
                    None => force = true,
                },
                "input" | "output" | "format" | "image-format" | "quality" | "background"
                | "image-compression" => {
                    let value = match attached {
                        Some(v) => v,
                        None => {
                            i += 1;
                            match args.get(i) {
                                Some(v) => v.clone(),
                                None => {
                                    fail!("option '--{name}' needs a value");
                                    break;
                                }
                            }
                        }
                    };
                    if let (
                        "output" | "image-format" | "quality" | "background" | "image-compression",
                        Some(m),
                    ) = (name, convert_only(&format!("--{name}")))
                    {
                        fail!("{m}");
                    }
                    match name {
                        "input" => inputs.push(value),
                        "output" => set_single!(output, "--output", value),
                        "format" => set_single!(format, "--format", value),
                        "image-format" => set_single!(image_format, "--image-format", value),
                        "quality" => set_single!(quality, "--quality", value),
                        "background" => set_single!(background, "--background", value),
                        "image-compression" => {
                            set_single!(compression, "--image-compression", value)
                        }
                        _ => unreachable!(),
                    }
                }
                _ => fail!("unexpected option '--{name}'"),
            }
        } else if arg.len() > 1 && arg.starts_with('-') {
            // A short cluster: booleans bundle; the first value-taking flag ends
            // it by consuming the remainder of the token (or the next token).
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'h' => help = true,
                    'V' => version = true,
                    'f' => match convert_only("-f") {
                        Some(m) => fail!("{m}"),
                        None => force = true,
                    },
                    c @ ('i' | 'o') => {
                        let rest: String = chars[j + 1..].iter().collect();
                        let value = if !rest.is_empty() {
                            rest
                        } else {
                            i += 1;
                            match args.get(i) {
                                Some(v) => v.clone(),
                                None => {
                                    fail!("option '-{c}' needs a value");
                                    break;
                                }
                            }
                        };
                        match c {
                            'i' => inputs.push(value),
                            _ => match convert_only("-o") {
                                Some(m) => fail!("{m}"),
                                None => set_single!(output, "--output", value),
                            },
                        }
                        break; // the value-taking flag consumed the rest of the cluster
                    }
                    c => {
                        fail!("unexpected option '-{c}'");
                        break;
                    }
                }
                j += 1;
            }
        } else if arg == "convert" || arg == "inspect" {
            fail!("the subcommand '{arg}' must come first");
        } else {
            // A bare word: positional inputs are not accepted (§2). Point the
            // user straight at the form that works.
            fail!("unexpected argument '{arg}'; use -i {arg}");
        }
        i += 1;
    }

    // Reject out-of-set values (§3.5) after the scan, so a `-h` anywhere
    // still short-circuits to help.
    let json = match format.as_deref() {
        None | Some("human") => false,
        Some("json") => true,
        Some(f) => {
            fail!("invalid value '{f}' for --format; supported values: human, json");
            false
        }
    };
    let target = match image_format.as_deref() {
        None => None,
        Some(name) => match Format::from_name(name) {
            Some(f) => Some(f),
            None => {
                fail!(
                    "invalid value '{name}' for --image-format; supported values: \
                     webp, avif, jxl, jpeg, png"
                );
                None
            }
        },
    };
    let quality = match quality.as_deref() {
        None => None,
        Some(q) => match q.parse::<u8>() {
            Ok(n) if (1..=100).contains(&n) => Some(n),
            _ => {
                fail!("invalid value '{q}' for --quality; supported values: 1 to 100");
                None
            }
        },
    };
    let background = match background.as_deref() {
        None => None,
        Some(b) => match parse_colour(b) {
            Some(rgb) => Some(rgb),
            None => {
                fail!(
                    "invalid value '{b}' for --background; supported values: white, black, #RRGGBB"
                );
                None
            }
        },
    };

    let compression = match compression.as_deref() {
        None => None,
        Some(c) => match c {
            "smaller" => Some(Compression::Smaller),
            "balanced" => Some(Compression::Balanced),
            "faster" => Some(Compression::Faster),
            _ => {
                fail!(
                    "invalid value '{c}' for --image-compression; supported values: smaller, balanced, faster"
                );
                None
            }
        },
    };

    // Precedence: help short-circuits even a malformed line; a usage error
    // outranks a version request; version outranks a run; a run needs an input.
    if help {
        return Cli::Help(named.then_some(sub));
    }
    if let Some(msg) = error {
        return Cli::Usage(msg);
    }
    if version {
        return Cli::Version;
    }
    let input = match inputs.len() {
        0 => return Cli::Usage("missing required -i".to_string()),
        1 => inputs.remove(0),
        n => {
            return Cli::Usage(format!(
                "imagoverto takes one image at a time: expected 1 input, got {n}"
            ));
        }
    };
    if sub == Sub::Inspect {
        return Cli::Inspect(InspectRun { input, json });
    }
    let Some(target) = target else {
        return Cli::Usage("missing required --image-format".to_string());
    };
    let mut options = Options::new(target);
    if let Some(q) = quality {
        options = options.with_quality(q);
    }
    if let Some(rgb) = background {
        options = options.with_background(rgb);
    }
    if let Some(level) = compression {
        options = options.with_compression(level);
    }
    // Options that no image could satisfy are the invocation's fault.
    if let Err(Error::InvalidOptions { reason }) = options.check() {
        return Cli::Usage(reason);
    }
    if let Some(o) = &output
        && let Some(named) = format_of_extension(Path::new(o))
        && named != target
    {
        return Cli::Usage(format!(
            "-o '{o}' names a {named} file, but --image-format is {}",
            target.name()
        ));
    }
    Cli::Convert(ConvertRun {
        input,
        output,
        force,
        options,
        json,
    })
}

/// `white`, `black`, or `#RRGGBB` (hex, either case).
fn parse_colour(s: &str) -> Option<[u8; 3]> {
    match s {
        "white" => Some([255; 3]),
        "black" => Some([0; 3]),
        _ => {
            let hex = s.strip_prefix('#')?;
            if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let byte = |k: usize| u8::from_str_radix(&hex[k..k + 2], 16).ok();
            Some([byte(0)?, byte(2)?, byte(4)?])
        }
    }
}

/// The format a file name's extension names, if it names one.
fn format_of_extension(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some(Format::Jpeg),
        "png" => Some(Format::Png),
        "webp" => Some(Format::WebP),
        "jxl" => Some(Format::Jxl),
        "avif" => Some(Format::Avif),
        _ => None,
    }
}

/// CLI.md §4: `<stem>.<ext>` beside the input (an extension-changing
/// transformer needs no verb), or `<stem>_converted.<ext>` when the extension
/// would not change, so the output can never be the input.
fn default_output(input: &Path, target: Format) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let ext = target.extension();
    let same = input
        .extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext));
    let name = if same {
        format!("{stem}_converted.{ext}")
    } else {
        format!("{stem}.{ext}")
    };
    input.with_file_name(name)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Cli::Help(sub) => {
            println!(
                "{}",
                match sub {
                    None => HELP,
                    Some(Sub::Convert) => CONVERT_HELP,
                    Some(Sub::Inspect) => INSPECT_HELP,
                }
            );
            ExitCode::SUCCESS
        }
        Cli::Version => {
            println!("imagoverto {}", imagoverto::VERSION);
            ExitCode::SUCCESS
        }
        Cli::Usage(msg) => {
            // Short stderr message + a pointer to --help; never the full help.
            eprintln!("error: {msg} (see --help)");
            ExitCode::from(2)
        }
        Cli::Convert(run) => finish(convert(&run), &run.input, run.json),
        Cli::Inspect(run) => finish(inspect(&run), &run.input, run.json),
    }
}

/// Report a run that stopped with exit `2` (FORMATS §1, as kepubverto splits
/// it), or pass its exit code through.
fn finish(result: Result<ExitCode, Stop>, input: &str, json: bool) -> ExitCode {
    match result {
        Ok(code) => code,
        Err(Stop::Refused(e)) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
        Err(Stop::Input(e)) => {
            eprintln!("error: {e}");
            if json {
                print_envelope(EnvelopeStatus::Error, input_error(input, &e));
            }
            ExitCode::from(2)
        }
    }
}

/// Why a run stopped with exit `2`.
enum Stop {
    /// The tool would not run: output = input, an existing output without
    /// `-f`. No envelope.
    Refused(String),
    /// The input could not be processed: unreadable, or an image imagoverto
    /// cannot convert, or the output could not be written. Described in the
    /// envelope.
    Input(String),
}

fn convert(run: &ConvertRun) -> Result<ExitCode, Stop> {
    let input = Path::new(&run.input);
    let target = run.options.format;
    // Resolve the output and enforce the file-safety rules before any work.
    let out = match &run.output {
        Some(o) => PathBuf::from(o),
        None => default_output(input, target),
    };
    let bytes = std::fs::read(input)
        .map_err(|e| Stop::Input(format!("cannot read {}: {e}", input.display())))?;
    if same_path(input, &out) {
        return Err(Stop::Refused(format!(
            "output path is the input ({}); refusing to modify the original in place \
             — choose a different -o (-f does not lift this)",
            out.display()
        )));
    }
    if out.exists() && !run.force {
        return Err(Stop::Refused(format!(
            "'{}' exists; use -f to replace it",
            out.display()
        )));
    }
    let conversion = imagoverto::convert(&bytes, &run.options)
        .map_err(|e| Stop::Input(format!("cannot convert {}: {e}", run.input)))?;
    std::fs::write(&out, &conversion.bytes)
        .map_err(|e| Stop::Input(format!("cannot write {}: {e}", out.display())))?;

    if run.json {
        print_envelope(
            EnvelopeStatus::Ok,
            input_converted(&conversion, &run.input, &out),
        );
    } else {
        print_conversion(&conversion, &run.input, &out);
    }
    Ok(ExitCode::SUCCESS)
}

fn inspect(run: &InspectRun) -> Result<ExitCode, Stop> {
    let bytes = std::fs::read(&run.input)
        .map_err(|e| Stop::Input(format!("cannot read {}: {e}", run.input)))?;
    // A file that is not one of the five formats, or whose header does not
    // read, gives no report at all.
    let info = imagoverto::inspect(&bytes)
        .map_err(|e| Stop::Input(format!("cannot inspect {}: {e}", run.input)))?;
    let decodes = imagoverto::decode_check(&bytes).map(|_| ());
    if run.json {
        let status = if decodes.is_ok() {
            EnvelopeStatus::Ok
        } else {
            EnvelopeStatus::Problems
        };
        print_envelope(
            status,
            input_inspected(&info, &decodes, &run.input, bytes.len()),
        );
    } else {
        println!("{}: {}", run.input, describe(&info, bytes.len()));
        match &decodes {
            Ok(()) => println!("decodes: yes"),
            Err(e) => println!("decodes: no — {e}"),
        }
    }
    Ok(if decodes.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Whether two paths name the same file. Compares canonical paths when the
/// output exists, else the output's canonical parent plus its name.
fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| -> Option<PathBuf> {
        if let Ok(c) = p.canonicalize() {
            return Some(c);
        }
        let parent = match p.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => Path::new("."),
        };
        Some(parent.canonicalize().ok()?.join(p.file_name()?))
    };
    match (canon(a), canon(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn compression_name(c: Compression) -> &'static str {
    match c {
        Compression::Smaller => "smaller",
        Compression::Faster => "faster",
        _ => "balanced",
    }
}

fn quality_label(q: Quality) -> String {
    match q {
        Quality::Lossless => "lossless".into(),
        Quality::Lossy(n) => format!("quality {n}"),
    }
}

fn size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}

fn describe(info: &Info, bytes: usize) -> String {
    let mut parts = vec![
        format!("{} {}x{}", info.format, info.width, info.height),
        size(bytes),
    ];
    parts.push(match info.lossless {
        Some(true) => "lossless".into(),
        Some(false) => "lossy".into(),
        None => "lossless or lossy: not stated".into(),
    });
    if let Some(q) = info.jpeg_quality {
        parts.push(format!("saved at quality ~{q}"));
    }
    parts.push(if info.gray {
        "gray".into()
    } else {
        "colour".into()
    });
    if info.has_alpha {
        parts.push("alpha".into());
    }
    parts.push(format!("{} bit", info.bit_depth));
    if info.icc_profile {
        parts.push("ICC profile".into());
    }
    if info.animated {
        parts.push("animated".into());
    }
    if !info.within_limits() {
        parts.push("past the decode limits".into());
    }
    parts.join(", ")
}

fn print_conversion(c: &Conversion, input: &str, out: &Path) {
    for f in &c.findings {
        if f.severity() == Severity::Warning {
            eprintln!("warning: {}", f.message());
        }
    }
    let level = match c.compression {
        Compression::Balanced => String::new(),
        other => format!(", {}", compression_name(other)),
    };
    println!(
        "{input}: {} {}x{} → {}, {}{level}, {} → {}",
        c.source.format,
        c.source.width,
        c.source.height,
        c.format,
        quality_label(c.quality),
        size(c.input_size),
        size(c.bytes.len())
    );
    for f in &c.findings {
        if f.severity() == Severity::Info {
            println!("note: {}", f.message());
        }
    }
    println!("wrote {}", out.display());
}

// --- The machine envelope (FORMATS v0.7 §1) --------------------------------
//
// imagoverto's own types for the shared skeleton: the JSON is the contract.
// FORMATS §2's reference types live in epubveri, which an image converter
// should not depend on (promotion trigger raised with conventions).

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum EnvelopeStatus {
    Ok,
    Problems,
    Error,
}

#[derive(Serialize)]
struct Envelope<'a> {
    tool: &'static str,
    tool_version: &'static str,
    convention: &'static str,
    status: EnvelopeStatus,
    inputs: [InputObject<'a>; 1],
}

#[derive(Serialize)]
struct InputObject<'a> {
    path: &'a str,
    status: EnvelopeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<Summary>,
    items: Vec<Item>,
}

/// imagoverto's per-input summary (FORMATS §2, tool-owned): the sizes, and
/// a counter for every severity it has a concept of (§1.4).
#[derive(Serialize)]
struct Summary {
    input_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_bytes: Option<usize>,
    errors: usize,
    warnings: usize,
    infos: usize,
}

#[derive(Serialize)]
struct Item {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<&'static str>,
    code: &'static str,
    rule: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    severity: Option<&'static str>,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

fn print_envelope(status: EnvelopeStatus, input: InputObject<'_>) {
    let env = Envelope {
        tool: "imagoverto",
        tool_version: imagoverto::VERSION,
        convention: CONVENTION,
        status,
        inputs: [input],
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&env).expect("the envelope serializes")
    );
}

fn input_error<'a>(path: &'a str, error: &str) -> InputObject<'a> {
    InputObject {
        path,
        status: EnvelopeStatus::Error,
        error: Some(error.to_string()),
        output: None,
        summary: None,
        items: Vec::new(),
    }
}

fn severity_name(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
    }
}

fn finding_item(f: Finding) -> Item {
    Item {
        kind: "finding",
        outcome: None,
        code: f.code(),
        rule: f.rule(),
        severity: Some(severity_name(f.severity())),
        message: f.message().to_string(),
        data: None,
    }
}

fn quality_json(q: Quality) -> serde_json::Value {
    match q {
        Quality::Lossless => "lossless".into(),
        Quality::Lossy(n) => n.into(),
    }
}

fn input_converted<'a>(c: &Conversion, path: &'a str, out: &Path) -> InputObject<'a> {
    let operation = Item {
        kind: "operation",
        outcome: Some("applied"),
        code: "IMG-000",
        rule: "convert",
        severity: None,
        message: format!(
            "converted {} to {}, {}",
            c.source.format,
            c.format,
            quality_label(c.quality)
        ),
        data: Some(serde_json::json!({
            "from": c.source.format.name(),
            "to": c.format.name(),
            "quality": quality_json(c.quality),
            "compression": compression_name(c.compression),
            "width": c.source.width,
            "height": c.source.height,
            "input_bytes": c.input_size,
            "output_bytes": c.bytes.len(),
        })),
    };
    let count = |s: Severity| c.findings.iter().filter(|f| f.severity() == s).count();
    InputObject {
        path,
        status: EnvelopeStatus::Ok,
        error: None,
        output: Some(out.display().to_string()),
        summary: Some(Summary {
            input_bytes: c.input_size,
            output_bytes: Some(c.bytes.len()),
            errors: 0,
            warnings: count(Severity::Warning),
            infos: count(Severity::Info),
        }),
        items: std::iter::once(operation)
            .chain(c.findings.iter().copied().map(finding_item))
            .collect(),
    }
}

fn input_inspected<'a>(
    info: &Info,
    decodes: &Result<(), Error>,
    path: &'a str,
    bytes: usize,
) -> InputObject<'a> {
    let mut items = vec![Item {
        kind: "finding",
        outcome: None,
        code: "IMG-100",
        rule: "inspect.header",
        severity: Some("info"),
        message: describe(info, bytes),
        data: Some(serde_json::json!({
            "format": info.format.name(),
            "width": info.width,
            "height": info.height,
            "has_alpha": info.has_alpha,
            "gray": info.gray,
            "bit_depth": info.bit_depth,
            "animated": info.animated,
            "icc_profile": info.icc_profile,
            "lossless": info.lossless,
            "jpeg_quality": info.jpeg_quality,
            "within_limits": info.within_limits(),
        })),
    }];
    if let Err(e) = decodes {
        items.push(Item {
            kind: "finding",
            outcome: None,
            code: "IMG-101",
            rule: "inspect.not_decodable",
            severity: Some("error"),
            message: format!("does not decode: {e}"),
            data: None,
        });
    }
    let errors = usize::from(decodes.is_err());
    InputObject {
        path,
        status: if errors == 0 {
            EnvelopeStatus::Ok
        } else {
            EnvelopeStatus::Problems
        },
        error: None,
        output: None,
        summary: Some(Summary {
            input_bytes: bytes,
            output_bytes: None,
            errors,
            warnings: 0,
            infos: 1,
        }),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Cli {
        parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn usage(args: &[&str]) -> String {
        match p(args) {
            Cli::Usage(m) => m,
            other => panic!("{args:?} gave {other:?}"),
        }
    }

    #[test]
    fn convert_is_the_default_and_takes_its_options() {
        let want = Cli::Convert(ConvertRun {
            input: "a.webp".into(),
            output: None,
            force: false,
            options: Options::new(Format::Jpeg).with_quality(80),
            json: false,
        });
        assert_eq!(
            p(&["-i", "a.webp", "--image-format", "jpeg", "--quality", "80"]),
            want
        );
        assert_eq!(
            p(&["convert", "-ia.webp", "--image-format=jpeg", "--quality=80"]),
            want
        );
        match p(&[
            "-i",
            "a.png",
            "--image-format",
            "avif",
            "--image-compression",
            "faster",
        ]) {
            Cli::Convert(r) => assert_eq!(r.options.compression, Compression::Faster),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn syntaxes_of_section_3_3() {
        // -i's value is never re-parsed as an option.
        match p(&["-i", "-q.png", "--image-format", "png"]) {
            Cli::Convert(r) => assert_eq!(r.input, "-q.png"),
            other => panic!("{other:?}"),
        }
        // Bundled booleans, then a value-taking flag consuming the rest.
        match p(&["-fi", "a.png", "--image-format", "webp"]) {
            Cli::Convert(r) => assert!(r.force),
            other => panic!("{other:?}"),
        }
        assert!(matches!(p(&["-i", "a.png", "--"]), Cli::Usage(_))); // no --image-format
    }

    #[test]
    fn help_and_version() {
        assert_eq!(p(&["-h"]), Cli::Help(None));
        assert_eq!(p(&["convert", "--help"]), Cli::Help(Some(Sub::Convert)));
        assert_eq!(p(&["inspect", "-h"]), Cli::Help(Some(Sub::Inspect)));
        // Help short-circuits a malformed line.
        assert_eq!(p(&["--bogus", "x", "-h"]), Cli::Help(None));
        assert_eq!(p(&["-V"]), Cli::Version);
        assert!(matches!(p(&["-V", "--bogus"]), Cli::Usage(_)));
    }

    #[test]
    fn usage_errors() {
        assert_eq!(usage(&[]), "missing required -i");
        assert!(usage(&["a.png"]).contains("use -i a.png"));
        assert!(
            usage(&["-i", "a", "-i", "b", "--image-format", "png"])
                .contains("expected 1 input, got 2")
        );
        assert!(
            usage(&["-i", "a", "--format", "x", "--format", "json"]).contains("more than once")
        );
        assert!(usage(&["-i", "a", "--format", "xml"]).contains("supported values: human, json"));
        assert!(
            usage(&["-i", "a", "--image-format", "gif"]).contains("webp, avif, jxl, jpeg, png")
        );
        assert!(
            usage(&["-i", "a", "--image-format", "png", "--quality", "0"]).contains("1 to 100")
        );
        assert!(
            usage(&["-i", "a", "--image-format", "png", "--background", "red"]).contains("#RRGGBB")
        );
        assert!(
            usage(&[
                "-i",
                "a",
                "--image-format",
                "png",
                "--image-compression",
                "smallest"
            ])
            .contains("smaller, balanced, faster")
        );
        assert!(
            usage(&["inspect", "-i", "a", "--image-compression", "faster"]).contains("for convert")
        );
        assert_eq!(usage(&["-i", "a"]), "missing required --image-format");
        // Option-level refusals, before any file is read.
        assert!(
            usage(&["-i", "a", "--image-format", "jpeg"]).contains("JPEG has no lossless mode")
        );
        assert!(usage(&["-i", "a", "--image-format", "png", "--quality", "80"]).contains("PNG"));
        // -o's extension must not name another format.
        assert!(
            usage(&["-i", "a", "--image-format", "png", "-o", "b.jpg"])
                .contains("names a JPEG file")
        );
        // Options of the other subcommand, and a misplaced subcommand.
        assert!(usage(&["inspect", "-i", "a", "--quality", "80"]).contains("for convert"));
        assert!(usage(&["inspect", "-i", "a", "-f"]).contains("for convert"));
        assert!(usage(&["-i", "a", "inspect"]).contains("must come first"));
    }

    #[test]
    fn colours() {
        assert_eq!(parse_colour("white"), Some([255, 255, 255]));
        assert_eq!(parse_colour("#00ff7F"), Some([0, 255, 127]));
        assert_eq!(parse_colour("#00ff7"), None);
        assert_eq!(parse_colour("00ff77"), None);
        assert_eq!(parse_colour("#00gg77"), None);
    }

    #[test]
    fn default_output_names() {
        let d = |i: &str, f| default_output(Path::new(i), f);
        assert_eq!(
            d("dir/cover.webp", Format::Jpeg),
            Path::new("dir/cover.jpg")
        );
        assert_eq!(
            d("cover.jpg", Format::Jpeg),
            Path::new("cover_converted.jpg")
        );
        assert_eq!(
            d("cover.JPG", Format::Jpeg),
            Path::new("cover_converted.jpg")
        );
        assert_eq!(d("cover.jpeg", Format::Jpeg), Path::new("cover.jpg"));
        assert_eq!(d("scan", Format::Png), Path::new("scan.png"));
    }
}
