//! Verify the CLI contract for `.xlsx` input: PDF only, YAML result documents,
//! and no workbook content or paths in diagnostics.
#[path = "xlsx_support/mod.rs"]
mod support;
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

/// Workbook inside the measured default layout: Normal and every cell font are
/// Noto Sans JP 11 pt (the test font only borrows that family name), A4 portrait,
/// a print area, and cells given as `cells` (a `<sheetData>` body).
/// `fonts` replaces the cell font of style 1; `extra_xf` is appended to `cellXfs`.
fn measured_workbook(dir: &Path, cells: &str, fonts: &str, extra_xf: &str) -> std::path::PathBuf {
    let path = dir.join("機密の見積.xlsx");
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
            r#"<workbook xmlns="{SS}" xmlns:r="{R}"><sheets><sheet name="機密の案件名" sheetId="1" r:id="rId1"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">'機密の案件名'!$A$1:$B$2</definedName></definedNames></workbook>"#
        ),
    );
    add(
        "xl/_rels/workbook.xml.rels",
        format!(
            r#"<Relationships xmlns="{PR}"><Relationship Id="rId1" Type="{R}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{R}/styles" Target="styles.xml"/></Relationships>"#
        ),
    );
    add(
        "xl/styles.xml",
        format!(
            r#"<styleSheet xmlns="{SS}"><numFmts count="1"><numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0"/></numFmts><fonts><font><sz val="11"/><name val="Noto Sans JP"/></font>{fonts}</fonts><fills><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders><border><left/><right/><top/><bottom/><diagonal/></border><border><left/><right/><top/><bottom style="thin"><color rgb="FF000000"/></bottom><diagonal/></border></borders><cellStyleXfs><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="164" fontId="1" fillId="0" borderId="1" xfId="0" applyNumberFormat="1" applyFont="1" applyBorder="1"/>{extra_xf}</cellXfs><cellStyles><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
        ),
    );
    add(
        "xl/worksheets/sheet1.xml",
        format!(
            r#"<worksheet xmlns="{SS}"><sheetFormatPr defaultRowHeight="19"/><cols><col min="1" max="2" width="20" customWidth="1"/></cols><sheetData>{cells}</sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup paperSize="9" orientation="portrait"/></worksheet>"#
        ),
    );
    zip.finish().unwrap();
    path
}

const MEASURED_FONT: &str = r#"<font><sz val="11"/><name val="Noto Sans JP"/></font>"#;
const MEASURED_CELLS: &str = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Total</t></is></c><c r="B1" s="1"><v>1200000</v></c></row>"#;

fn measured_font(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("noto-name.ttf");
    fs::write(&path, support::font("Noto Sans JP", false, false, 0)).unwrap();
    path
}

#[test]
fn xlsx_in_the_measured_layout_converts_to_one_page_with_a_success_document() {
    let dir = tempfile::tempdir().unwrap();
    let input = measured_workbook(dir.path(), MEASURED_CELLS, MEASURED_FONT, "");
    let font = measured_font(dir.path());
    let out = dir.path().join("見積.pdf");
    let (code, stdout, stderr) = run([
        input.as_os_str(),
        OsStr::new("--format"),
        OsStr::new("pdf"),
        OsStr::new("--out"),
        out.as_os_str(),
        OsStr::new("--no-system-fonts"),
        OsStr::new("--font-file"),
        font.as_os_str(),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.starts_with("status: completed\noutput:\n  created: true\n"),
        "{stdout}"
    );
    assert!(stdout.contains("  pages: 1\n"));
    assert!(
        stdout.contains(r#""first_cell":"A1","last_cell":"B2""#),
        "{stdout}"
    );
    assert!(stdout.contains(r#""embedded":true"#));
    assert!(!stdout.contains("機密"));
    assert!(fs::read(&out).unwrap().starts_with(b"%PDF-"));
    // A second run never overwrites the published PDF.
    let before = fs::read(&out).unwrap();
    let (code, _, stderr) = run([
        input.as_os_str(),
        OsStr::new("--format"),
        OsStr::new("pdf"),
        OsStr::new("--out"),
        out.as_os_str(),
        OsStr::new("--no-system-fonts"),
        OsStr::new("--font-file"),
        font.as_os_str(),
    ]);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("reason: output_already_exists\n"));
    assert_eq!(fs::read(&out).unwrap(), before);
}

#[test]
fn xlsx_locale_is_ja_jp_by_default_and_rejects_unknown_values_and_pptx_input() {
    let dir = tempfile::tempdir().unwrap();
    let font = measured_font(dir.path());
    // Built-in date id 14 resolves only under en-US; ja-JP stops with its reason.
    let input = measured_workbook(
        dir.path(),
        r#"<row r="1"><c r="A1" s="2"><v>46328</v></c></row>"#,
        MEASURED_FONT,
        r#"<xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>"#,
    );
    for (locale, expected) in [
        (None, Some(1)),
        (Some("ja-JP"), Some(1)),
        (Some("en-US"), Some(0)),
    ] {
        let out = dir.path().join(format!("{locale:?}.pdf"));
        let mut args = vec![
            input.clone().into_os_string(),
            "--format".into(),
            "pdf".into(),
            "--out".into(),
            out.clone().into_os_string(),
            "--no-system-fonts".into(),
            "--font-file".into(),
            font.clone().into_os_string(),
        ];
        if let Some(locale) = locale {
            args.extend(["--locale".into(), locale.into()]);
        }
        let (code, _, stderr) = run(args);
        assert_eq!(code, expected, "{locale:?} {stderr}");
        if expected == Some(1) {
            assert!(stderr.contains("reason: builtin_date_locale\n"), "{stderr}");
            assert!(!out.exists());
        }
    }
    let (code, stdout, stderr) = run([
        input.as_os_str(),
        OsStr::new("--format"),
        OsStr::new("pdf"),
        OsStr::new("--locale"),
        OsStr::new("fr-FR"),
    ]);
    assert_eq!(code, Some(2));
    assert!(stdout.is_empty());
    assert!(stderr.contains("code: INVALID_ARGUMENTS"));
    let deck = dir.path().join("deck.pptx");
    let (code, _, stderr) = run([
        deck.as_os_str(),
        OsStr::new("--format"),
        OsStr::new("pdf"),
        OsStr::new("--locale"),
        OsStr::new("en-US"),
    ]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("--locale applies to XLSX input only."));
}

#[test]
fn xlsx_outside_the_measured_layout_stops_with_a_located_reason_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let font = measured_font(dir.path());
    let theme_font =
        r#"<font><sz val="11"/><name val="Noto Sans JP"/><scheme val="minor"/></font>"#;
    let other_family = r#"<font><sz val="11"/><name val="Arial"/></font>"#;
    let unmeasured_size = r#"<font><sz val="41"/><name val="Noto Sans JP"/></font>"#;
    let double_underline =
        r#"<font><u val="double"/><sz val="11"/><name val="Noto Sans JP"/></font>"#;
    for (fonts, cells, reason, cell) in [
        (theme_font, MEASURED_CELLS, "cell_layout_unmeasured", "B1"),
        (other_family, MEASURED_CELLS, "cell_layout_unmeasured", "B1"),
        (
            unmeasured_size,
            MEASURED_CELLS,
            "cell_layout_unmeasured",
            "B1",
        ),
        (
            double_underline,
            MEASURED_CELLS,
            "underlined_cell_text",
            "B1",
        ),
        (
            MEASURED_FONT,
            r#"<row r="1"><c r="A1"><f>機密の数式</f></c></row>"#,
            "formula_cached_value_missing",
            "A1",
        ),
        (
            MEASURED_FONT,
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Two
lines</t></is></c></row>"#,
            "text_control_character",
            "A1",
        ),
    ] {
        let input = measured_workbook(dir.path(), cells, fonts, "");
        let out = dir.path().join("out.pdf");
        let (code, stdout, stderr) = run([
            input.as_os_str(),
            OsStr::new("--format"),
            OsStr::new("pdf"),
            OsStr::new("--out"),
            out.as_os_str(),
            OsStr::new("--no-system-fonts"),
            OsStr::new("--font-file"),
            font.as_os_str(),
        ]);
        assert_eq!(code, Some(1), "{reason} {stderr}");
        assert!(stdout.is_empty());
        assert!(
            stderr.contains(&format!(
                "reason: {reason}\n    location:\n      sheet: 1\n      cell: \"{cell}\"\n"
            )),
            "{reason} {stderr}"
        );
        assert!(!stderr.contains("機密"));
        assert!(!out.exists());
        assert!(!dir.path().read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".pdf")
        }));
    }
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
        // The minimal workbook has no styles part, so no cell layout can be resolved.
        assert!(
            stderr.contains("  - code: UNSUPPORTED_FEATURE\n    reason: cell_styles_unresolved\n")
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
            "cell_styles_unresolved"
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
    let input = measured_workbook(dir.path(), MEASURED_CELLS, MEASURED_FONT, "")
        .canonicalize()
        .unwrap();
    let output = dir.path().canonicalize().unwrap().join("成果物.pdf");
    let font = dir.path().join("機密の書体.ttf");
    fs::write(&font, support::font("Noto Sans JP", false, false, 0)).unwrap();
    let font = font.canonicalize().unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_rhodopsin"))
        .canonicalize()
        .unwrap();
    // Reads for the OS runtime stay broadly allowed; the input and font denials are
    // added per case. This is not evidence for a production read allowlist.
    let base = r#"(version 1)(deny default)(allow sysctl-read)(allow file-read*)(allow process-exec (literal (param "BINARY")))"#;
    // Allowed reads convert; the output directory is writable for the published PDF.
    let mut allowed = Command::new("/usr/bin/sandbox-exec");
    for (key, path) in [("BINARY", &binary), ("INPUT", &input), ("FONT", &font)] {
        allowed.arg("-D").arg(format!("{key}={}", path.display()));
    }
    allowed
        .arg("-D")
        .arg(format!(
            "OUTDIR={}",
            dir.path().canonicalize().unwrap().display()
        ))
        .arg("-p")
        .arg(format!(
            r#"{base}(allow file-write* (subpath (param "OUTDIR")))"#
        ))
        .arg(&binary)
        .arg(&input)
        .args(["--format", "pdf", "--no-system-fonts", "--font-file"])
        .arg(&font)
        .arg("--out")
        .arg(&output);
    let result = allowed.output().unwrap();
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(stdout.starts_with("status: completed\n"));
    assert!(fs::read(&output).unwrap().starts_with(b"%PDF-"));
    fs::remove_file(&output).unwrap();
    for (extra, use_font, reason) in [
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
