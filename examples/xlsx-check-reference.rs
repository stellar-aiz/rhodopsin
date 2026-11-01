//! 開発専用。参照 fixture の記録とファイルを照合する。生成元アプリの真偽は証明しない。
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path},
};
type Result<T> = std::result::Result<T, &'static str>;
const FILE_LIMIT: u64 = 128 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    file: String,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Export {
    artifact: Artifact,
    input_sha256: String,
    application: String,
    version: String,
    os: String,
    procedure: String,
    recalculated: bool,
    fonts_substituted: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Font {
    family: String,
    version: String,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    input: Artifact,
    excel: Export,
    libreoffice: Option<Export>,
    locale: String,
    date_system: String,
    fonts: Vec<Font>,
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn field(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        Err("invalid_generation_metadata")
    } else {
        Ok(())
    }
}
fn verify_file(root: &Path, artifact: &Artifact, extension: &str) -> Result<()> {
    if !valid_hash(&artifact.sha256) {
        return Err("invalid_sha256");
    }
    let path = Path::new(&artifact.file);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || path.extension().and_then(|s| s.to_str()) != Some(extension)
    {
        return Err("invalid_artifact_path");
    }
    let path = root
        .join(path)
        .canonicalize()
        .map_err(|_| "artifact_unreadable")?;
    if !path.starts_with(root) {
        return Err("artifact_outside_fixture");
    }
    let file = File::open(path).map_err(|_| "artifact_unreadable")?;
    let metadata = file.metadata().map_err(|_| "artifact_unreadable")?;
    if !metadata.is_file() {
        return Err("artifact_not_regular_file");
    }
    if metadata.len() > FILE_LIMIT {
        return Err("artifact_size_limit");
    }
    let mut source = file.take(FILE_LIMIT + 1);
    let mut buffer = [0u8; 65536];
    let mut size = 0u64;
    let mut hash = Sha256::new();
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|_| "artifact_unreadable")?;
        if read == 0 {
            break;
        }
        size += read as u64;
        if size > FILE_LIMIT {
            return Err("artifact_size_limit");
        }
        hash.update(&buffer[..read]);
    }
    if format!("{:x}", hash.finalize()) != artifact.sha256 {
        return Err("artifact_hash_mismatch");
    }
    Ok(())
}
fn verify_export(root: &Path, export: &Export, input: &Artifact, application: &str) -> Result<()> {
    if export.application != application {
        return Err("unexpected_export_application");
    }
    for value in [&export.version, &export.os, &export.procedure] {
        field(value)?;
    }
    if export.input_sha256 != input.sha256 {
        return Err("export_input_mismatch");
    }
    if export.recalculated || export.fonts_substituted {
        return Err("export_conditions_differ");
    }
    verify_file(root, &export.artifact, "pdf")
}
fn verify(root: &Path, manifest: &Manifest) -> Result<usize> {
    let root = root
        .canonicalize()
        .map_err(|_| "fixture_directory_unreadable")?;
    if manifest.schema_version != 1 {
        return Err("reference_schema_version");
    }
    field(&manifest.locale)?;
    if !matches!(manifest.date_system.as_str(), "1900" | "1904") {
        return Err("invalid_date_system");
    }
    if manifest.fonts.is_empty() || manifest.fonts.len() > 1024 {
        return Err("reference_font_count");
    }
    for font in &manifest.fonts {
        field(&font.family)?;
        field(&font.version)?;
        if !valid_hash(&font.sha256) {
            return Err("invalid_sha256");
        }
    }
    verify_file(&root, &manifest.input, "xlsx")?;
    verify_export(&root, &manifest.excel, &manifest.input, "Microsoft Excel")?;
    let mut count = 2;
    if let Some(lo) = &manifest.libreoffice {
        if lo.artifact.file == manifest.excel.artifact.file {
            return Err("duplicate_export_path");
        }
        verify_export(&root, lo, &manifest.input, "LibreOffice")?;
        count += 1;
    }
    Ok(count)
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("expected_reference_manifest_path");
    }
    let path = Path::new(&args[0]);
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "manifest_unreadable")?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "manifest_unreadable")?;
    if bytes.len() > 65536 {
        return Err("manifest_size_limit");
    }
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| "invalid_reference_manifest")?;
    let root = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let count = verify(root, &manifest)?;
    serde_json::to_writer(std::io::stdout().lock(),&serde_json::json!({"status":"checked","verified_artifacts":count,
        "provenance":"declared_not_authenticated","overall_verdict":"not_evaluated",
        "unchecked":["file_format_validity","application_authenticity","export_procedure_execution","font_files","workbook_generation_conditions","excel_fidelity"]})).map_err(|_| "report_write_failed")?;
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => 0.into(),
        Err(reason) => {
            eprintln!("{}", serde_json::json!({"status":"failed","reason":reason}));
            2.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Manifest) {
        let dir = tempfile::tempdir().unwrap();
        // ファイル書式の検証ではないため、ハッシュ照合用の合成バイト列を使う。
        std::fs::write(dir.path().join("input.xlsx"), b"synthetic input").unwrap();
        std::fs::write(dir.path().join("excel.pdf"), b"synthetic reference").unwrap();
        let input_hash = format!("{:x}", Sha256::digest(b"synthetic input"));
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "schema_version":1,"input":{"file":"input.xlsx","sha256":input_hash},
            "excel":{"artifact":{"file":"excel.pdf","sha256":format!("{:x}",Sha256::digest(b"synthetic reference"))},
                "input_sha256":input_hash,"application":"Microsoft Excel","version":"試験版","os":"試験OS","procedure":"試験用の記録","recalculated":false,"fonts_substituted":false},
            "libreoffice":null,"locale":"ja-JP","date_system":"1900",
            "fonts":[{"family":"試験書体","version":"1","sha256":"a".repeat(64)}]
        })).unwrap();
        (dir, manifest)
    }
    #[test]
    fn 改変ファイルと異なる入力からの出力を検出する() {
        let (dir, mut manifest) = fixture();
        assert_eq!(verify(dir.path(), &manifest).unwrap(), 2);
        manifest.excel.input_sha256 = "b".repeat(64);
        assert_eq!(verify(dir.path(), &manifest), Err("export_input_mismatch"));
        manifest.excel.input_sha256 = manifest.input.sha256.clone();
        std::fs::write(dir.path().join("excel.pdf"), b"changed").unwrap();
        assert_eq!(verify(dir.path(), &manifest), Err("artifact_hash_mismatch"));
    }
    #[test]
    fn 再計算や書体代用を同じ生成条件と扱わない() {
        let (dir, mut manifest) = fixture();
        manifest.excel.recalculated = true;
        assert_eq!(
            verify(dir.path(), &manifest),
            Err("export_conditions_differ")
        );
        manifest.excel.recalculated = false;
        manifest.excel.fonts_substituted = true;
        assert_eq!(
            verify(dir.path(), &manifest),
            Err("export_conditions_differ")
        );
        manifest.excel.fonts_substituted = false;
        manifest.excel.application = "LibreOffice".into();
        assert_eq!(
            verify(dir.path(), &manifest),
            Err("unexpected_export_application")
        );
    }
    #[test]
    fn 入力パスと必須記録を検査し自由文を診断に返さない() {
        let (dir, mut manifest) = fixture();
        for path in ["../機密.xlsx", "/機密.xlsx", "機密.pdf"] {
            manifest.input.file = path.into();
            assert_eq!(verify(dir.path(), &manifest), Err("invalid_artifact_path"));
        }
        manifest.input.file = "input.xlsx".into();
        manifest.excel.version.clear();
        assert_eq!(
            verify(dir.path(), &manifest),
            Err("invalid_generation_metadata")
        );
        manifest.excel.version = "試験版".into();
        manifest.fonts[0].sha256 = "機密".into();
        assert_eq!(verify(dir.path(), &manifest), Err("invalid_sha256"));
    }
    #[cfg(unix)]
    #[test]
    fn fixture外へのシンボリックリンクを拒否する() {
        let (dir, mut manifest) = fixture();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("outside.xlsx")).unwrap();
        manifest.input.file = "outside.xlsx".into();
        assert_eq!(
            verify(dir.path(), &manifest),
            Err("artifact_outside_fixture")
        );
    }
}
