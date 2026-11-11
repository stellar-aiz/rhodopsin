//! PDF API and no-clobber publication. Keep the PNG report and API unchanged.
use super::writer;
use crate::{
    ConvertOptions, Diagnostic, FontReport, LineHeightReport, prepare, report_fonts,
    report_line_heights,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Serialize)]
pub struct PdfSlideReport {
    /// Original one-based deck slide number.
    pub slide: usize,
    /// One-based PDF page number.
    pub page: usize,
    pub hidden: bool,
    pub warnings: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize)]
pub struct PdfConversionReport {
    pub schema_version: u32,
    pub version: &'static str,
    pub status: &'static str,
    pub file: PathBuf,
    pub pages: usize,
    pub width_pt: f64,
    pub height_pt: f64,
    pub punctuation_compression: bool,
    pub include_hidden: bool,
    pub skipped_hidden_slides: Vec<usize>,
    pub system_fonts: bool,
    pub fonts: Vec<FontReport>,
    pub line_heights: Vec<LineHeightReport>,
    pub slides: Vec<PdfSlideReport>,
}

/// Convert to one PDF with real Type 3 text, without an external runtime.
/// `settings.width` is ignored. Unsupported PDF features fail before publication.
/// Existing destinations (including symlinks) are never replaced. The returned
/// report contains diagnostics; no sidecar is written. See docs/rendering.md for limits.
pub fn convert_pdf(
    input: impl AsRef<Path>,
    out: impl AsRef<Path>,
    settings: &ConvertOptions,
) -> Result<PdfConversionReport> {
    let input = input.as_ref().to_path_buf();
    let out = out.as_ref().to_path_buf();
    let settings = settings.clone();
    std::thread::Builder::new()
        .name("pptx-pdf".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || convert_inner(&input, &out, &settings))
        .context("Cannot start conversion thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("Conversion thread panicked"))?
}

fn convert_inner(
    input: &Path,
    out: &Path,
    settings: &ConvertOptions,
) -> Result<PdfConversionReport> {
    ensure!(
        fs::symlink_metadata(out).is_err(),
        "Output already exists: {}",
        out.display()
    );
    let (deck, options, _) = prepare(input, settings, true)?;
    ensure!(
        !deck.slides.is_empty(),
        "PDF requires at least one rendered slide"
    );
    let mut writer = writer::Pdf::new();
    let mut slides = Vec::new();
    for (index, slide) in deck.slides.iter().enumerate() {
        let tree = resvg::usvg::Tree::from_str(&slide.svg, &options)
            .with_context(|| format!("Cannot parse slide {}", slide.number))?;
        let notes = writer
            .page(&tree)
            .with_context(|| format!("PDF slide {}", slide.number))?;
        let mut warnings = slide.warnings.clone();
        let mut diagnostics = slide.diagnostics.clone();
        // What only a raster could show in PDF is reported like any other approximation.
        for (code, message, location) in notes {
            if !warnings.contains(&message) {
                warnings.push(message.clone());
            }
            diagnostics.push(Diagnostic {
                code,
                part: slide.part.clone(),
                object_id: None,
                object_name: None,
                location: format!("pdf/{location}"),
                message,
                font: None,
                required_font: None,
                missing_character: None,
            });
        }
        slides.push(PdfSlideReport {
            slide: slide.number,
            page: index + 1,
            hidden: slide.hidden,
            warnings,
            diagnostics,
        });
    }
    // Strict mode already stopped on rendering diagnostics; PDF rasters add their own.
    if settings.strict && slides.iter().any(|s| !s.warnings.is_empty()) {
        return Err(crate::StrictModeError {
            slides: slides
                .iter()
                .map(|s| crate::SlideReport {
                    slide: s.slide,
                    hidden: s.hidden,
                    image: out.display().to_string(),
                    warnings: s.warnings.clone(),
                    diagnostics: s.diagnostics.clone(),
                })
                .collect(),
        }
        .into());
    }
    let report = PdfConversionReport {
        schema_version: 2,
        version: env!("CARGO_PKG_VERSION"),
        status: if slides.iter().any(|s| !s.warnings.is_empty()) {
            "preview_with_warnings"
        } else {
            "rendered"
        },
        file: out.to_path_buf(),
        pages: slides.len(),
        width_pt: deck.width * 0.75,
        height_pt: deck.height * 0.75,
        punctuation_compression: settings.punctuation_compression,
        include_hidden: settings.include_hidden,
        skipped_hidden_slides: deck.skipped_hidden.clone(),
        system_fonts: !settings.no_system_fonts,
        fonts: report_fonts(&deck, &options),
        line_heights: report_line_heights(&deck),
        slides,
    };
    let bytes = writer.finish()?;
    publish(out, |file| {
        file.write_all(&bytes)?;
        Ok(())
    })?;
    Ok(report)
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// A same-directory hard link atomically publishes a complete file without
/// replacing an existing file. Filesystems without hard links fail explicitly.
fn publish(out: &Path, write: impl FnOnce(&mut fs::File) -> Result<()>) -> Result<()> {
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut candidate = None;
    for _ in 0..32 {
        let path = parent.join(format!(
            ".rhodopsin-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                candidate = Some((Temporary(path), file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).context("Cannot create temporary PDF"),
        }
    }
    let (temp, mut file) = candidate.context("Cannot allocate a temporary PDF name")?;
    write(&mut file)?;
    file.sync_all().context("Cannot flush PDF")?;
    drop(file);
    fs::hard_link(&temp.0, out).with_context(|| {
        format!(
            "Cannot publish PDF without overwriting {} (filesystem must support hard links)",
            out.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_write_and_destination_race_leave_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.pdf");
        assert!(
            publish(&out, |file| {
                file.write_all(b"partial")?;
                anyhow::bail!("simulated I/O failure")
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(
            publish(&out, |file| {
                file.write_all(b"new")?;
                fs::write(&out, b"original")?;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(fs::read(&out).unwrap(), b"original");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
