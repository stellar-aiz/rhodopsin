//! Verify the CLI contract for `.xlsx` input: PDF only, YAML result documents,
//! and no workbook content or paths in diagnostics.
use std::{ffi::OsStr, fs, io::Write, path::Path, process::Command};
use zip::{ZipWriter, write::SimpleFileOptions};

const SS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PR: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Minimal workbook whose file and sheet names must never appear in output.
fn workbook(dir: &Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("機密の顧客名.xlsx");
    let mut zip = ZipWriter::new(fs::File::create(&path).unwrap());
    let mut add = |name: &str, value: String| {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    };
    add(
        "[Content_Types].xml",
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#.into(),
    );
    add(
        "_rels/.rels",
        format!(
            r#"<Relationships xmlns="{PR}"><Relationship Id="rId1" Type="{R}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
        ),
    );
    add(
        "xl/workbook.xml",
        format!(
            r#"<workbook xmlns="{SS}" xmlns:r="{R}"><sheets><sheet name="機密の案件名" sheetId="1" r:id="rId1"/></sheets></workbook>"#
        ),
    );
    add(
        "xl/_rels/workbook.xml.rels",
        format!(
            r#"<Relationships xmlns="{PR}"><Relationship Id="rId1" Type="{R}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
        ),
    );
    add(
        "xl/worksheets/sheet1.xml",
        format!(r#"<worksheet xmlns="{SS}">{body}</worksheet>"#),
    );
    zip.finish().unwrap();
    path
}

fn run<I, S>(args: I) -> (Option<i32>, String, String)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(env!("CARGO_BIN_EXE_rhodopsin"))
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn sample_font() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/samples.ttf")
}

#[test]
fn xlsx_pdf_stops_with_unsupported_feature_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let input = workbook(dir.path(), "<sheetData/>");
    for strict in [false, true] {
        let mut args = vec![
            input.clone().into_os_string(),
            "--format".into(),
            "pdf".into(),
            "--no-system-fonts".into(),
            "--font-file".into(),
            sample_font().into_os_string(),
        ];
        if strict {
            args.push("--strict".into());
        }
        let (code, stdout, stderr) = run(args);
        assert_eq!(code, Some(1), "{stderr}");
        assert!(stdout.is_empty());
        assert!(stderr.starts_with("status: failed\noutput:\n  created: false\nerrors:\n"));
        assert!(
            stderr
                .contains("  - code: UNSUPPORTED_FEATURE\n    reason: rendering_not_implemented\n")
        );
        assert!(stderr.contains("    suggestion: "));
        assert!(!stderr.contains("機密"));
    }
    // The default destination sits next to the input and is not created on failure.
    assert!(!input.with_extension("pdf").exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn xlsx_rejects_png_and_pptx_only_options_before_reading_the_workbook() {
    let dir = tempfile::tempdir().unwrap();
    let input = workbook(dir.path(), "<sheetData/>");
    let out = dir.path().join("out");
    let (code, stdout, stderr) = run([input.as_os_str(), OsStr::new("--out"), out.as_os_str()]);
    assert_eq!(code, Some(2));
    assert!(stdout.is_empty());
    assert!(stderr.starts_with("status: failed\n"));
    assert!(stderr.contains("code: FORMAT_NOT_SUPPORTED"));
    assert!(!out.exists());
    for option in [
        vec!["--font-fallback"],
        vec!["--font-map", "機密=機密"],
        vec!["--include-hidden"],
        vec!["--punctuation-compression"],
    ] {
        let mut args = vec![input.to_str().unwrap(), "--format", "pdf"];
        args.extend(option.iter().copied());
        let (code, stdout, stderr) = run(&args);
        assert_eq!(code, Some(2), "{option:?}");
        assert!(stdout.is_empty());
        assert!(stderr.contains("code: OPTION_NOT_SUPPORTED"));
        assert!(stderr.contains(&format!("option: \"{}\"", option[0])));
        assert!(!stderr.contains("機密"));
    }
    assert!(!input.with_extension("pdf").exists());
}

#[test]
fn xlsx_diagnostics_carry_cell_locations_without_contents() {
    let dir = tempfile::tempdir().unwrap();
    let input = workbook(
        dir.path(),
        r#"<sheetData><row r="1"><c r="A1"><f>機密の数式</f></c></row></sheetData>"#,
    );
    let out = dir.path().join("out.pdf");
    let (code, stdout, stderr) = run([
        input.as_os_str(),
        OsStr::new("--format"),
        OsStr::new("pdf"),
        OsStr::new("--out"),
        out.as_os_str(),
        OsStr::new("--no-system-fonts"),
    ]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stdout.is_empty());
    assert!(stderr.contains("  - code: INVALID_INPUT\n    reason: formula_cached_value_missing\n    location:\n      sheet: 1\n      cell: \"A1\"\n"));
    assert!(!stderr.contains("機密"));
    assert!(!out.exists());
}

#[test]
fn xlsx_refuses_to_overwrite_and_hides_font_paths() {
    let dir = tempfile::tempdir().unwrap();
    let input = workbook(dir.path(), "<sheetData/>");
    let out = dir.path().join("out.pdf");
    fs::write(&out, b"original").unwrap();
    let (code, _, stderr) = run([
        input.as_os_str(),
        OsStr::new("--format"),
        OsStr::new("pdf"),
        OsStr::new("--out"),
        out.as_os_str(),
        OsStr::new("--no-system-fonts"),
    ]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("code: INVALID_INPUT\n    reason: output_already_exists\n"));
    assert_eq!(fs::read(&out).unwrap(), b"original");

    let missing = dir.path().join("機密の書体");
    let (code, stdout, stderr) = run([
        input.as_os_str(),
        OsStr::new("--format"),
        OsStr::new("pdf"),
        OsStr::new("--no-system-fonts"),
        OsStr::new("--font-file"),
        missing.as_os_str(),
    ]);
    assert_eq!(code, Some(1));
    assert!(stdout.is_empty());
    assert!(stderr.contains("code: FONT_NOT_FOUND\n"));
    assert!(stderr.contains("--font-file"));
    assert!(!stderr.contains("機密"));
    assert!(!input.with_extension("pdf").exists());
}

#[test]
fn xlsx_system_font_search_follows_the_shared_flag() {
    let dir = tempfile::tempdir().unwrap();
    let input = workbook(dir.path(), "<sheetData/>");
    for no_system_fonts in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rhodopsin"));
        command.arg(&input).args(["--format", "pdf"]);
        // Relative home paths are rejected by the system font search; they must not be echoed.
        for key in ["HOME", "WINDIR", "LOCALAPPDATA", "XDG_DATA_HOME"] {
            command.env(key, "機密の相対パス");
        }
        if no_system_fonts {
            command.arg("--no-system-fonts");
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(output.stdout.is_empty());
        let reason = if no_system_fonts {
            "rendering_not_implemented"
        } else {
            "invalid_system_font_directory"
        };
        assert!(stderr.contains(&format!("reason: {reason}\n")), "{stderr}");
        assert!(!stderr.contains("機密"));
    }
    assert!(!input.with_extension("pdf").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn xlsx_denied_reads_fail_without_leaking_under_seatbelt() {
    let dir = tempfile::tempdir().unwrap();
    let input = workbook(dir.path(), "<sheetData/>").canonicalize().unwrap();
    let output = dir.path().canonicalize().unwrap().join("成果物.pdf");
    let font = dir.path().join("機密の書体.ttf");
    fs::write(&font, "unreadable test data").unwrap();
    let font = font.canonicalize().unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_rhodopsin"))
        .canonicalize()
        .unwrap();
    // Reads for the OS runtime stay broadly allowed; the input and font denials are
    // added per case. This is not evidence for a production read allowlist.
    let base = r#"(version 1)(deny default)(allow sysctl-read)(allow file-read*)(allow process-exec (literal (param "BINARY")))"#;
    for (extra, use_font, reason) in [
        ("", false, "rendering_not_implemented"),
        (
            r#"(deny file-read-data (literal (param "INPUT")))"#,
            false,
            "input_unreadable",
        ),
        (
            r#"(deny file-read-data (literal (param "FONT")))"#,
            true,
            "font_file_unreadable",
        ),
    ] {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        for (key, path) in [("BINARY", &binary), ("INPUT", &input), ("FONT", &font)] {
            command.arg("-D").arg(format!("{key}={}", path.display()));
        }
        command
            .arg("-p")
            .arg(format!("{base}{extra}"))
            .arg(&binary)
            .arg(&input)
            .args(["--format", "pdf", "--no-system-fonts", "--out"])
            .arg(&output);
        if use_font {
            command.arg("--font-file").arg(&font);
        }
        let result = command.output().unwrap();
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert_eq!(result.status.code(), Some(1), "{stderr}");
        assert!(result.stdout.is_empty());
        assert!(stderr.contains(&format!("reason: {reason}\n")), "{stderr}");
        assert!(!stderr.contains("機密"));
        assert!(!output.exists());
    }
}

#[test]
fn xls_and_xlsm_are_refused_as_spreadsheets_not_as_pptx() {
    let dir = tempfile::tempdir().unwrap();
    // A legacy binary workbook signature, and a renamed OOXML workbook with macros enabled.
    let legacy = dir.path().join("book.xls");
    fs::write(
        &legacy,
        [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1, 0, 0, 0, 0],
    )
    .unwrap();
    let macro_enabled = dir.path().join("book.XLSM");
    fs::copy(workbook(dir.path(), "<sheetData/>"), &macro_enabled).unwrap();
    for input in [&legacy, &macro_enabled] {
        let (code, stdout, stderr) = run([input.as_os_str(), "--format".as_ref(), "pdf".as_ref()]);
        assert_eq!(code, Some(1), "{stderr}");
        assert!(stdout.is_empty());
        assert!(stderr.contains("  - code: INVALID_INPUT\n"), "{stderr}");
        assert!(
            stderr.contains("reason: input_extension_not_xlsx\n"),
            "{stderr}"
        );
        assert!(!stderr.contains("presentation"), "{stderr}");
        assert!(!input.with_extension("pdf").exists());
    }
    // A legacy file renamed to .xlsx is refused for its contents.
    let renamed = dir.path().join("legacy.xlsx");
    fs::copy(&legacy, &renamed).unwrap();
    let (code, _, stderr) = run([renamed.as_os_str(), "--format".as_ref(), "pdf".as_ref()]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("reason: encrypted_or_legacy_workbook\n"),
        "{stderr}"
    );
}
