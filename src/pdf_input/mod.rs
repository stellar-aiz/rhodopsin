//! Experimental, bounded interpretation of a small PDF subset.
mod render;
mod syntax;
use anyhow::{Result, ensure};
use serde::Serialize;
use std::{fs, io::Read, path::Path};

/// Options for the supported PDF subset. Unsupported rendering always fails.
#[derive(Clone, Debug)]
pub struct Options {
    /// PNG width in pixels (64 through 4096).
    pub width: u32,
    /// Reserved for parity with the CLI: unsupported content fails in either mode.
    pub strict: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            width: 1600,
            strict: false,
        }
    }
}
/// A successfully rendered page.
#[derive(Debug, Serialize)]
pub struct Page {
    pub page: usize,
    pub file: String,
    pub width: u32,
    pub height: u32,
}
/// Published page manifest; font outlines are read from Type 3 programs.
#[derive(Debug, Serialize)]
pub struct Report {
    pub pages: Vec<Page>,
}
fn safe_io<T>(r: std::io::Result<T>) -> Result<T> {
    r.map_err(|_| anyhow::anyhow!("PDF input/output operation failed"))
}
/// Convert the documented PDF subset into a new directory. Never overwrites.
/// On handled failure no partial final directory is published.
pub fn convert(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    options: &Options,
) -> Result<Report> {
    let output = output.as_ref();
    ensure!(
        (64..=4096).contains(&options.width),
        "Output width out of range"
    );
    match output.symlink_metadata() {
        Ok(_) => anyhow::bail!("Output already exists"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => anyhow::bail!("Cannot inspect output"),
    }
    let mut data = Vec::new();
    safe_io(
        safe_io(fs::File::open(input))?
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut data),
    )?;
    ensure!(data.len() <= 32 * 1024 * 1024, "Input byte limit");
    let doc = syntax::Document::parse(&data)?;
    let pages = render::pages(&doc)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = safe_io(
        tempfile::Builder::new()
            .prefix(".rhodopsin-pdf-")
            .tempdir_in(parent),
    )?;
    let result = (|| -> Result<Report> {
        let mut renderer = render::Renderer::new(&doc);
        let mut report = Report { pages: Vec::new() };
        for (index, page) in pages.iter().enumerate() {
            let object = match page.get("Contents") {
                Some(syntax::Value::Ref(id)) => *id,
                _ => 0,
            };
            let pix = renderer.page(page, options.width).map_err(|e| {
                anyhow::anyhow!("PDF page {} content object {}: {}", index + 1, object, e)
            })?;
            let file = format!("page-{:03}.png", index + 1);
            pix.save_png(stage.path().join(&file))
                .map_err(|_| anyhow::anyhow!("Cannot write page image"))?;
            report.pages.push(Page {
                page: index + 1,
                file,
                width: pix.width(),
                height: pix.height(),
            });
        }
        safe_io(fs::write(
            stage.path().join("manifest.json"),
            serde_json::to_vec_pretty(&report)?,
        ))?;
        publish(stage.path(), output)?;
        Ok(report)
    })();
    match result {
        Ok(report) => {
            let _ = stage.keep();
            Ok(report)
        }
        Err(error) => {
            safe_io(stage.close())?;
            Err(error)
        }
    }
}
#[cfg(unix)]
fn publish(stage: &Path, out: &Path) -> Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage,
        rustix::fs::CWD,
        out,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|_| anyhow::anyhow!("Cannot publish PDF output without replacement"))?;
    Ok(())
}
#[cfg(windows)]
fn publish(stage: &Path, out: &Path) -> Result<()> {
    safe_io(fs::rename(stage, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_refuses_a_destination_created_after_staging() {
        let dir = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(dir.path()).unwrap();
        fs::write(stage.path().join("page-001.png"), b"staged").unwrap();
        let out = dir.path().join("out");
        fs::create_dir(&out).unwrap();
        assert!(publish(stage.path(), &out).is_err());
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
        assert!(stage.path().join("page-001.png").exists());
    }
}
