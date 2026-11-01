//! CLI for the PPTX-to-PNG conversion library.
mod cli_output;
mod cli_xlsx;
use clap::{Parser, ValueEnum};
use rhodopsin::{ConvertOptions, SlideReport, convert, convert_pdf};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Format {
    #[default]
    Png,
    Pdf,
}

#[derive(Parser)]
#[command(
    version,
    about = "Convert PPTX to PNG or PDF without Office (early release)",
    after_help = "Fonts: system fonts are loaded by default; application bundles and inaccessible paths may not be searched. Add available font files with --font-file.
Individual missing glyphs use available fallback fonts with warnings; unrenderable glyphs fail. Family substitution requires --font-fallback.
Results: YAML on stdout for success, stderr for failure. PNG diagnostics are saved in manifest.json; PDF diagnostics are included in the result document.
XLSX: .xlsx input is converted to PDF only and always stops on unsupported content (as with --strict). Supported options: --out, --font-file, --no-system-fonts, --strict. Rendering is not complete yet."
)]
struct Args {
    /// Input PPTX, or XLSX workbook (PDF only)
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
fn main() {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            error.exit()
        }
        Err(error) => {
            eprint!("{}", cli_output::invalid_arguments(&error.to_string()));
            std::process::exit(2);
        }
    };
    let out = args.out.unwrap_or_else(|| {
        args.input.with_extension(match args.format {
            Format::Png => "rhodopsin",
            Format::Pdf => "pdf",
        })
    });
    if cli_xlsx::is_spreadsheet(&args.input) {
        if matches!(args.format, Format::Png) {
            eprint!("{}", cli_xlsx::unsupported_format());
            std::process::exit(2);
        }
        let rejected = [
            (args.font_fallback, "--font-fallback"),
            (!args.font_map.is_empty(), "--font-map"),
            (args.include_hidden, "--include-hidden"),
            (args.punctuation_compression, "--punctuation-compression"),
        ];
        if let Some((_, option)) = rejected.iter().find(|(set, _)| *set) {
            eprint!("{}", cli_xlsx::unsupported_option(option));
            std::process::exit(2);
        }
        let options = cli_xlsx::Options {
            font_file: args.font_file,
            no_system_fonts: args.no_system_fonts,
        };
        match cli_xlsx::convert(&args.input, &out, options) {
            Ok(document) => print!("{document}"),
            Err(document) => {
                eprint!("{document}");
                std::process::exit(1);
            }
        }
        return;
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
            Ok(report) => print!("{}", cli_output::completed_pdf(&report)),
            Err(error) => {
                eprint!("{}", cli_output::failed(&error, Some(&out), false));
                std::process::exit(1);
            }
        }
        return;
    }
    match convert(&args.input, &out, &settings) {
        Ok(report) => {
            print_warnings(&report.slides);
            print!("{}", cli_output::completed(&out, &report.slides));
        }
        Err(error) => {
            eprint!(
                "{}",
                cli_output::failed(&error, Some(&out), !existed && out.exists())
            );
            std::process::exit(1);
        }
    }
}
