//! The `rhodopsin` command-line interface, available with the `cli` feature (on by default).
//!
//! A dependent crate can build the same binary as upstream with
//! `fn main() -> std::process::ExitCode { rhodopsin::cli::run() }`: arguments, help and
//! version text, result documents, exit codes, and `manifest.json` are those of this crate.
mod output;
#[cfg(feature = "xlsx")]
mod xlsx;
use crate::{ConvertOptions, SlideReport, convert, convert_pdf};
use clap::{Parser, ValueEnum};
use std::{ffi::OsString, path::PathBuf, process::ExitCode};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Format {
    #[default]
    Png,
    Pdf,
}

/// Locale for XLSX number and date formats.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Locale {
    #[value(name = "ja-JP")]
    JaJp,
    #[value(name = "en-US")]
    EnUs,
}

#[derive(Parser)]
#[command(
    version,
    about = "Convert PPTX to PNG or PDF without Office (early release)",
    after_help = "Fonts: system fonts are loaded by default; application bundles and inaccessible paths may not be searched. Add available font files with --font-file.
Individual missing glyphs use available fallback fonts with warnings; unrenderable glyphs fail. Family substitution requires --font-fallback.
Results: YAML on stdout for success, stderr for failure. PNG diagnostics are saved in manifest.json; PDF diagnostics are included in the result document.
PDF input: experimental basic paths, Type 3 text and opaque RGB images only; unsupported content fails. See docs/pdf-input.md.\nXLSX: .xlsx input is converted to PDF only and always stops on unsupported content (as with --strict). Supported options: --out, --font-file, --no-system-fonts, --strict, --locale. Only the layout measured against Excel for Mac is rendered (cells in Noto Sans JP, 6-40 pt); see docs/xlsx.md."
)]
struct Args {
    /// Input PPTX, PDF (supported subset to PNG), or XLSX workbook (PDF only)
    input: PathBuf,
    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Png)]
    format: Format,
    /// PNG directory or PDF file (default: <input>.rhodopsin or <input>.pdf; never overwritten)
    #[arg(long)]
    out: Option<PathBuf>,
    /// PNG image width; ignored for PDF page size
    #[arg(long, default_value_t = 1600, value_parser = clap::value_parser!(u32).range(64..=4096))]
    width: u32,
    /// Fail on warnings without writing images
    #[arg(long)]
    strict: bool,
    /// Allow font-family substitution and approximate weight or italic style
    #[arg(long)]
    font_fallback: bool,
    /// Compress punctuation spacing to fit lines (experimental; disabled by default)
    #[arg(long)]
    punctuation_compression: bool,
    /// Also render slides hidden in the deck (skipped by default)
    #[arg(long)]
    include_hidden: bool,
    /// Additional font file (repeatable)
    #[arg(long)]
    font_file: Vec<PathBuf>,
    /// Load only explicitly supplied fonts, without system fonts
    #[arg(long)]
    no_system_fonts: bool,
    /// Requested=Replacement font mapping (repeatable; one step only)
    #[arg(long, value_parser = parse_font_mapping)]
    font_map: Vec<(String, String)>,
    /// Locale for number and date formats of XLSX input [default: ja-JP]
    #[arg(long, value_enum)]
    locale: Option<Locale>,
}

fn parse_font_mapping(value: &str) -> std::result::Result<(String, String), String> {
    let (from, to) = value
        .split_once('=')
        .ok_or("Specify Requested=Replacement")?;
    if from.trim().is_empty() || to.trim().is_empty() {
        return Err("Font family names must not be empty".into());
    }
    Ok((from.trim().into(), to.trim().into()))
}
fn print_warnings(slides: &[SlideReport]) {
    let mut fonts = std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for slide in slides {
        for diagnostic in &slide.diagnostics {
            if let Some(font) = &diagnostic.font {
                fonts
                    .entry(font.requested_family.clone())
                    .or_default()
                    .insert(format!("{} ({})", font.post_script_name, font.reason));
            } else {
                eprintln!("Warning [slide {}]: {}", slide.slide, diagnostic.message);
            }
        }
    }
    for (requested, actual) in &fonts {
        eprintln!(
            "Font substitution: {requested} -> {}",
            actual.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    if !fonts.is_empty() {
        eprintln!(
            "Add fonts with --font-file or select replacements with --font-map 'Requested=Replacement'. See diagnostics.font in manifest.json for details."
        );
    }
}
/// Run the CLI with the process arguments; see [`run_from_args`].
pub fn run() -> ExitCode {
    run_from_args(std::env::args_os())
}

/// Run the CLI with `args`, the first of which is the program name (as in
/// `clap::Parser::parse_from`). Help, version, result documents, and warnings are written to
/// stdout and stderr exactly as the `rhodopsin` binary does, and the returned code is the
/// binary's exit status: 0 on success, 1 when conversion fails, 2 for invalid arguments. The
/// process is never exited, so the function can be called in-process. Like `print!`, it
/// panics only if writing to stdout or stderr fails.
pub fn run_from_args<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = match Args::try_parse_from(args) {
        Ok(args) => args,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            // clap prints help and version to stdout and exits with 0.
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprint!("{}", output::invalid_arguments(&error.to_string()));
            return ExitCode::from(2);
        }
    };
    let out = args.out.unwrap_or_else(|| {
        args.input.with_extension(match args.format {
            Format::Png => "rhodopsin",
            Format::Pdf => "pdf",
        })
    });
    if args
        .input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
    {
        #[cfg(not(feature = "pdf-input"))]
        {
            eprint!(
                "{}",
                output::invalid_arguments("PDF input support is disabled.")
            );
            return ExitCode::from(2);
        }
        #[cfg(feature = "pdf-input")]
        {
            if matches!(args.format, Format::Pdf)
                || args.font_fallback
                || !args.font_map.is_empty()
                || args.include_hidden
                || args.punctuation_compression
                || !args.font_file.is_empty()
                || args.no_system_fonts
                || args.locale.is_some()
            {
                eprint!(
                    "{}",
                    output::invalid_arguments(
                        "PDF input supports PNG output, --width, --out and --strict only."
                    )
                );
                return ExitCode::from(2);
            }
            match crate::pdf::render(
                &args.input,
                &out,
                &crate::pdf::Options {
                    width: args.width,
                    strict: args.strict,
                },
            ) {
                Ok(report) => println!(
                    "status: completed\nformat: png\nresult: {}",
                    serde_json::to_string(&report).expect("report serialization")
                ),
                Err(error) => {
                    eprintln!(
                        "status: failed\nmessage: {}",
                        output::quoted(&error.to_string())
                    );
                    return ExitCode::from(1);
                }
            }
            return ExitCode::SUCCESS;
        }
    }
    #[cfg(feature = "xlsx")]
    if xlsx::is_spreadsheet(&args.input) {
        if matches!(args.format, Format::Png) {
            eprint!("{}", xlsx::unsupported_format());
            return ExitCode::from(2);
        }
        let rejected = [
            (args.font_fallback, "--font-fallback"),
            (!args.font_map.is_empty(), "--font-map"),
            (args.include_hidden, "--include-hidden"),
            (args.punctuation_compression, "--punctuation-compression"),
        ];
        if let Some((_, option)) = rejected.iter().find(|(set, _)| *set) {
            eprint!("{}", xlsx::unsupported_option(option));
            return ExitCode::from(2);
        }
        let options = xlsx::Options {
            font_file: args.font_file,
            no_system_fonts: args.no_system_fonts,
            locale: match args.locale {
                Some(Locale::EnUs) => xlsx::Locale::English,
                Some(Locale::JaJp) | None => xlsx::Locale::Japanese,
            },
        };
        match xlsx::convert(&args.input, &out, options) {
            Ok(document) => print!("{document}"),
            Err(document) => {
                eprint!("{document}");
                return ExitCode::from(1);
            }
        }
        return ExitCode::SUCCESS;
    }
    if args.locale.is_some() {
        eprint!(
            "{}",
            output::invalid_arguments("--locale applies to XLSX input only.")
        );
        return ExitCode::from(2);
    }
    let existed = out.exists();
    let settings = ConvertOptions {
        width: args.width,
        strict: args.strict,
        font_fallback: args.font_fallback,
        punctuation_compression: args.punctuation_compression,
        font_file: args.font_file,
        no_system_fonts: args.no_system_fonts,
        font_mapping: args.font_map.into_iter().collect(),
        include_hidden: args.include_hidden,
    };
    if matches!(args.format, Format::Pdf) {
        match convert_pdf(&args.input, &out, &settings) {
            Ok(report) => print!("{}", output::completed_pdf(&report)),
            Err(error) => {
                eprint!("{}", output::failed(&error, Some(&out), false));
                return ExitCode::from(1);
            }
        }
        return ExitCode::SUCCESS;
    }
    match convert(&args.input, &out, &settings) {
        Ok(report) => {
            print_warnings(&report.slides);
            print!("{}", output::completed(&out, &report.slides));
        }
        Err(error) => {
            eprint!(
                "{}",
                output::failed(&error, Some(&out), !existed && out.exists())
            );
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}
