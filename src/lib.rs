//! Convert PPTX to PNG or PDF without an Office runtime dependency.
//!
//! ```no_run
//! use rhodopsin::{convert, ConvertOptions};
//! let report = convert("slides.pptx", "images", &ConvertOptions::default())?;
//! println!("Rendered {} slides", report.slides.len());
//! # Ok::<(), anyhow::Error>(())
//! ```
#[cfg(feature = "cli")]
pub mod cli;
mod pdf;
#[cfg(feature = "pdf-input")]
pub mod pdf_input;
mod pdf_output;
mod pptx;
#[cfg(feature = "xlsx")]
pub mod xlsx;
pub use pdf_output::{PdfConversionReport, PdfSlideReport, convert_pdf};

use anyhow::{Context, Result, bail};
pub use pptx::{Diagnostic, DiagnosticCode, FontResolution};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Conversion options shared with the CLI.
#[derive(Clone, Debug)]
pub struct ConvertOptions {
    /// Output width in pixels, from 64 to 4096.
    pub width: u32,
    /// Return StrictModeError on warnings without writing files.
    pub strict: bool,
    /// Allow family substitution and approximate styles. Glyph fallback is allowed by default.
    pub font_fallback: bool,
    /// Experimental automatic punctuation compression.
    pub punctuation_compression: bool,
    /// Additional font files to load.
    pub font_file: Vec<PathBuf>,
    /// Do not load system fonts.
    pub no_system_fonts: bool,
    /// Explicit requested-to-replacement family mappings (one step only).
    pub font_mapping: std::collections::HashMap<String, String>,
    /// Also render slides hidden with `show="0"`. By default they are skipped.
    pub include_hidden: bool,
}
impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            width: 1600,
            strict: false,
            font_fallback: false,
            punctuation_compression: false,
            font_file: Vec::new(),
            no_system_fonts: false,
            font_mapping: Default::default(),
            include_hidden: false,
        }
    }
}

/// Same content as the written manifest.json. The caller handles warnings.
#[derive(Clone, Debug, Serialize)]
pub struct ConversionReport {
    pub schema_version: u32,
    pub version: &'static str,
    pub status: &'static str,
    pub width: u32,
    pub height: u32,
    pub punctuation_compression: bool,
    /// Whether hidden slides were rendered (`--include-hidden`).
    pub include_hidden: bool,
    /// One-based deck positions of hidden slides that were skipped, in deck order.
    pub skipped_hidden_slides: Vec<usize>,
    pub system_fonts: bool,
    pub fonts: Vec<FontReport>,
    pub line_heights: Vec<LineHeightReport>,
    pub slides: Vec<SlideReport>,
}
/// What a requested family's natural line height is based on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineHeightBasis {
    /// Measured in PowerPoint with the family's own font.
    Powerpoint,
    /// The font's own metrics; PowerPoint's line height for the family is not measured.
    Font,
    /// The family is absent; PowerPoint's measured line height stands in for it.
    Estimate,
}
#[derive(Clone, Debug, Serialize)]
pub struct LineHeightReport {
    pub requested_family: String,
    pub basis: LineHeightBasis,
}
#[derive(Clone, Debug, Serialize)]
pub struct FontReport {
    pub family: String,
    pub post_script_name: String,
    pub face_index: u32,
}
#[derive(Clone, Debug, Serialize)]
pub struct SlideReport {
    /// One-based position in the deck. Skipped hidden slides leave gaps in the numbering.
    pub slide: usize,
    /// The slide is hidden in the deck (`show="0"`); only rendered when `include_hidden` is set.
    pub hidden: bool,
    pub image: String,
    pub warnings: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Diagnostics when strict mode stops. image names are planned outputs, not written files.
/// Access through `downcast_ref::<StrictModeError>()` on the returned `anyhow::Error`.
#[derive(Debug)]
pub struct StrictModeError {
    pub slides: Vec<SlideReport>,
}
impl std::fmt::Display for StrictModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Strict mode stopped because unsupported or approximate rendering was detected"
        )
    }
}
impl std::error::Error for StrictModeError {}

/// Diagnostics when font requirements prevent output.
/// image names are planned; no images or manifest are written.
#[derive(Debug)]
pub struct FontFallbackError {
    pub slides: Vec<SlideReport>,
}
impl std::fmt::Display for FontFallbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Required fonts, glyphs, or styles cannot be used as requested."
        )?;
        let mut entries =
            std::collections::BTreeMap::<String, std::collections::BTreeSet<usize>>::new();
        for slide in &self.slides {
            for diagnostic in &slide.diagnostics {
                let detail = if let Some(font) = &diagnostic.font {
                    let reason = match font.reason.as_str() {
                        "family_missing" => "family unavailable",
                        "mapped_family_missing" => "mapped family unavailable",
                        "glyph_fallback" => "required glyph unavailable",
                        "style_approximation" => "weight or italic style mismatch",
                        "explicit_mapping" => "explicit mapping",
                        _ => "substitution required",
                    };
                    format!("{} ({})", font.requested_family, reason)
                } else if diagnostic.code == DiagnosticCode::MissingGlyph {
                    diagnostic.message.clone()
                } else {
                    continue;
                };
                entries.entry(detail).or_default().insert(slide.slide);
            }
        }
        for (detail, slides) in entries {
            writeln!(
                f,
                "  {detail} - slides {}",
                slides
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        write!(
            f,
            "Add fonts with --font-file. Use --font-fallback to allow font-family substitution."
        )
    }
}
impl std::error::Error for FontFallbackError {}

/// Synchronously convert a PPTX into the specified directory.
///
/// Never overwrite an existing destination. Return warnings and diagnostics without
/// writing to stdout or stderr. Return failures as anyhow::Error.
/// An I/O failure may leave partial images; write the manifest only on completion.
/// Use an internal thread with an 8 MiB stack for recursive parsing.
/// Rendering includes approximations; absence of warnings does not guarantee PowerPoint compatibility.
pub fn convert(
    input: impl AsRef<Path>,
    out: impl AsRef<Path>,
    settings: &ConvertOptions,
) -> Result<ConversionReport> {
    let input = input.as_ref().to_path_buf();
    let out = out.as_ref().to_path_buf();
    let settings = settings.clone();
    std::thread::Builder::new()
        .name("pptx-render".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || convert_inner(input, out, settings))
        .context("Cannot start conversion thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("Conversion thread panicked"))?
}
fn convert_inner(
    input: PathBuf,
    out: PathBuf,
    settings: ConvertOptions,
) -> Result<ConversionReport> {
    if !(64..=4096).contains(&settings.width) {
        bail!("Output image width must be between 64 and 4096");
    }
    if out.exists() {
        bail!("Output already exists: {}", out.display());
    }
    let (deck, options, height) = prepare(&input, &settings, false)?;
    let has_warnings = deck.slides.iter().any(|s| !s.warnings.is_empty());
    // Validate all slides before creating the destination; do not leave a completion manifest on failure.
    let trees: Vec<_> = deck
        .slides
        .iter()
        .map(|slide| resvg::usvg::Tree::from_str(&slide.svg, &options))
        .collect::<Result<_, _>>()?;
    fs::create_dir(&out).context("Cannot create output directory")?;
    let mut reports = Vec::new();
    for (slide, tree) in deck.slides.iter().zip(trees) {
        let name = format!("slide-{:03}.png", slide.number);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(settings.width, height)
            .context("Cannot allocate rendering buffer")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(
                settings.width as f32 / deck.width as f32,
                height as f32 / deck.height as f32,
            ),
            &mut pixmap.as_mut(),
        );
        pixmap.save_png(out.join(&name))?;
        reports.push(SlideReport {
            slide: slide.number,
            hidden: slide.hidden,
            image: name,
            warnings: slide.warnings.clone(),
            diagnostics: slide.diagnostics.clone(),
        });
    }
    let manifest = ConversionReport {
        schema_version: 4,
        version: env!("CARGO_PKG_VERSION"),
        status: if has_warnings {
            "preview_with_warnings"
        } else {
            "rendered"
        },
        width: settings.width,
        height,
        punctuation_compression: settings.punctuation_compression,
        include_hidden: settings.include_hidden,
        skipped_hidden_slides: deck.skipped_hidden.clone(),
        system_fonts: !settings.no_system_fonts,
        fonts: report_fonts(&deck, &options),
        line_heights: report_line_heights(&deck),
        slides: reports,
    };
    fs::write(
        out.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn prepare(
    input: &Path,
    settings: &ConvertOptions,
    pdf: bool,
) -> Result<(pptx::Deck, resvg::usvg::Options<'static>, u32)> {
    let mut options = resvg::usvg::Options::default();
    if !settings.no_system_fonts {
        options.fontdb_mut().load_system_fonts();
    }
    for path in &settings.font_file {
        options
            .fontdb_mut()
            .load_font_file(path)
            .with_context(|| format!("Cannot read font: {}", path.display()))?;
    }
    // Pass measured face IDs to rendering to avoid reselecting a same-name font.
    let aliases: std::collections::HashMap<_, _> = options
        .fontdb
        .faces()
        .map(|f| (format!("rhodopsin-face-{}", f.id), f.id))
        .collect();
    options.font_resolver.select_font = Box::new(move |font, _| {
        font.families().iter().find_map(|family| match family {
            resvg::usvg::FontFamily::Named(name) => aliases.get(name).copied(),
            _ => None,
        })
    });
    // Resolve fallback during measurement; do not substitute only at rendering time.
    options.font_resolver.select_fallback = Box::new(|_, _, _| None);
    let deck = pptx::read(
        input,
        &pptx::RenderOptions {
            fonts: pptx::FontResolver::new(&options.fontdb, &settings.font_mapping),
            punctuation_compression: settings.punctuation_compression,
            slide_number: Default::default(),
        },
        settings.include_hidden,
    )?;
    let height = if pdf {
        0
    } else {
        let height = (f64::from(settings.width) * deck.height / deck.width).round();
        if !(1.0..=8192.0).contains(&height) {
            bail!("Output image height exceeds the limit");
        }
        let height = height as u32;
        if u64::from(settings.width) * u64::from(height) > 16_777_216 {
            bail!("Output image pixel count exceeds the limit");
        }
        height
    };
    // Always stop for unrenderable glyphs. Successful glyph fallback is a normal warning.
    let blocks = |d: &Diagnostic| {
        d.code == DiagnosticCode::MissingGlyph
            || (!settings.font_fallback
                && d.code == DiagnosticCode::FontSubstitution
                && !d.font.as_ref().is_some_and(|font| {
                    font.reason == "glyph_fallback"
                        || (font.reason == "style_approximation" && d.missing_character.is_some())
                }))
    };
    if deck
        .slides
        .iter()
        .any(|slide| slide.diagnostics.iter().any(&blocks))
    {
        return Err(FontFallbackError {
            slides: deck
                .slides
                .iter()
                .map(|slide| SlideReport {
                    slide: slide.number,
                    hidden: slide.hidden,
                    image: format!("slide-{:03}.png", slide.number),
                    warnings: slide.warnings.clone(),
                    diagnostics: slide
                        .diagnostics
                        .iter()
                        .filter(|d| blocks(d))
                        .cloned()
                        .collect(),
                })
                .collect(),
        }
        .into());
    }
    if options.fontdb.is_empty() && deck.slides.iter().any(|s| s.svg.contains("<text")) {
        bail!("No fonts available; provide one with --font-file");
    }
    let has_warnings = deck.slides.iter().any(|s| !s.warnings.is_empty());
    if settings.strict && has_warnings {
        return Err(StrictModeError {
            slides: deck
                .slides
                .iter()
                .map(|slide| SlideReport {
                    slide: slide.number,
                    hidden: slide.hidden,
                    image: format!("slide-{:03}.png", slide.number),
                    warnings: slide.warnings.clone(),
                    diagnostics: slide.diagnostics.clone(),
                })
                .collect(),
        }
        .into());
    }
    Ok((deck, options, height))
}

fn report_line_heights(deck: &pptx::Deck) -> Vec<LineHeightReport> {
    deck.line_heights
        .iter()
        .map(|(family, basis)| LineHeightReport {
            requested_family: family.clone(),
            basis: *basis,
        })
        .collect()
}
fn report_fonts(deck: &pptx::Deck, options: &resvg::usvg::Options<'_>) -> Vec<FontReport> {
    options
        .fontdb
        .faces()
        .filter(|f| {
            deck.slides.iter().any(|s| {
                s.svg
                    .contains(&format!("font-family=\"rhodopsin-face-{}\"", f.id))
            })
        })
        .map(|f| FontReport {
            family: f.families.first().map(|f| f.0.clone()).unwrap_or_default(),
            post_script_name: f.post_script_name.clone(),
            face_index: f.index,
        })
        .collect()
}
