//! Exercise the PDF API and CLI using deterministic local fonts and OOXML.
use rhodopsin::{ConvertOptions, convert, convert_pdf};
use std::{fs, io::Write, path::Path, process::Command};
use zip::{ZipWriter, write::SimpleFileOptions};
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PR: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
fn fixture(path: &Path, fill: &str, hidden: bool) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    let mut add = |name: &str, text: String| {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    };
    add(
        "_rels/.rels",
        format!(
            r#"<Relationships xmlns="{PR}"><Relationship Id="r" Type="{R}/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#
        ),
    );
    add(
        "ppt/presentation.xml",
        format!(
            r#"<p:presentation xmlns:p="{P}" xmlns:r="{R}"><p:sldIdLst><p:sldId id="256" r:id="r2"/><p:sldId id="257" r:id="r1"/></p:sldIdLst><p:sldSz cx="1905000" cy="952500"/></p:presentation>"#
        ),
    );
    add(
        "ppt/_rels/presentation.xml.rels",
        format!(
            r#"<Relationships xmlns="{PR}"><Relationship Id="r1" Type="{R}/slide" Target="slides/slide1.xml"/><Relationship Id="r2" Type="{R}/slide" Target="slides/slide2.xml"/></Relationships>"#
        ),
    );
    for (number, text) in [(1, "A"), (2, "あ")] {
        let show = if hidden && number == 1 { "0" } else { "1" };
        add(
            &format!("ppt/slides/slide{number}.xml"),
            format!(
                r#"<p:sld xmlns:p="{P}" xmlns:a="{A}" show="{show}"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Text"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="95250" y="95250"/><a:ext cx="1714500" cy="762000"/></a:xfrm><a:prstGeom prst="rect"/>{fill}</p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1800"><a:solidFill><a:srgbClr val="000000"/></a:solidFill><a:latin typeface="Pptx CJK"/><a:ea typeface="Pptx CJK"/></a:rPr><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
            ),
        );
    }
    zip.finish().unwrap();
}
fn options() -> ConvertOptions {
    ConvertOptions {
        no_system_fonts: true,
        font_file: vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cjk.ttf")],
        ..Default::default()
    }
}
fn run(input: &Path, out: Option<&Path>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rhodopsin"));
    command
        .arg(input)
        .args(["--format", "pdf", "--no-system-fonts", "--font-file"])
        .arg(&options().font_file[0]);
    if let Some(out) = out {
        command.arg("--out").arg(out);
    }
    command.output().unwrap()
}
#[test]
fn pdf_pages_text_order_hidden_policy_and_common_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.pptx");
    fixture(&input, "", true);
    let mut opts = options();
    opts.width = 0; // The library's PNG pixel width does not constrain PDF.
    let out = dir.path().join("out.pdf");
    let pdf = convert_pdf(&input, &out, &opts).unwrap();
    assert_eq!((pdf.pages, pdf.width_pt, pdf.height_pt), (1, 150.0, 75.0));
    assert_eq!(pdf.slides[0].slide, 1);
    assert_eq!(pdf.skipped_hidden_slides, vec![2]);
    let bytes = fs::read(&out).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with("%PDF-1.4"));
    assert!(text.contains("/Subtype /Type3"));
    assert!(text.contains("/ToUnicode"));
    assert!(text.contains("<01> <3042>"));
    assert!(text.contains("/ActualText <FEFF3042>"));
    assert!(!text.contains("/ActualText <FEFF0041>"));
    opts.width = 1600;
    let png = convert(&input, dir.path().join("png"), &opts).unwrap();
    assert_eq!(pdf.slides[0].diagnostics, png.slides[0].diagnostics);
    assert_eq!(
        serde_json::to_value(pdf.fonts).unwrap(),
        serde_json::to_value(png.fonts).unwrap()
    );
    opts.include_hidden = true;
    let all = convert_pdf(&input, dir.path().join("all.pdf"), &opts).unwrap();
    assert_eq!(all.pages, 2);
    assert!(all.slides[1].hidden);
    assert_eq!(all.slides[1].page, 2);
}
#[test]
fn cli_defaults_to_one_file_reports_diagnostics_and_refuses_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.pptx");
    fixture(&input, "", false);
    let result = run(&input, None);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.starts_with("status: completed\n"));
    assert!(stdout.contains("  pages: 2\n"));
    assert!(stdout.contains("  file: "));
    let report: serde_json::Value =
        serde_json::from_str(stdout.split("report: ").nth(1).unwrap()).unwrap();
    assert_eq!(report["slides"][0]["page"], 1);
    assert!(report["slides"][0]["diagnostics"].is_array());
    let out = input.with_extension("pdf");
    let original = fs::read(&out).unwrap();
    let again = run(&input, Some(&out));
    assert_eq!(again.status.code(), Some(1));
    assert!(again.stdout.is_empty());
    assert!(String::from_utf8_lossy(&again.stderr).contains("created: false"));
    assert_eq!(fs::read(out).unwrap(), original);
}
/// Convert a fixture whose shape properties contain `props` and return the PDF text and the
/// per-slide warnings.
fn pdf_of(props: &str) -> (String, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.pptx");
    fixture(&input, props, false);
    let out = dir.path().join("out.pdf");
    let report = convert_pdf(&input, &out, &options()).unwrap();
    let warnings = report
        .slides
        .iter()
        .flat_map(|s| s.warnings.clone())
        .collect();
    (
        String::from_utf8_lossy(&fs::read(&out).unwrap()).into_owned(),
        warnings,
    )
}
#[test]
fn gradients_become_shadings_and_alpha_becomes_graphics_state() {
    let gradient = r#"<a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs><a:gs pos="50000"><a:srgbClr val="00FF00"/></a:gs><a:gs pos="100000"><a:srgbClr val="0000FF"/></a:gs></a:gsLst><a:lin ang="0" scaled="1"/></a:gradFill>"#;
    let (pdf, warnings) = pdf_of(gradient);
    assert!(pdf.contains("/ShadingType 2"), "{pdf}");
    assert!(pdf.contains("/FunctionType 3"));
    assert!(pdf.contains("/Pattern cs"));
    assert!(
        !warnings.iter().any(|w| w.contains("PDF output")),
        "{warnings:?}"
    );
    let radial = r#"<a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs><a:gs pos="100000"><a:srgbClr val="0000FF"/></a:gs></a:gsLst><a:path path="circle"><a:fillToRect l="50000" t="50000" r="50000" b="50000"/></a:path></a:gradFill>"#;
    assert!(pdf_of(radial).0.contains("/ShadingType 3"));
    let alpha = r#"<a:solidFill><a:srgbClr val="FF0000"><a:alpha val="60000"/></a:srgbClr></a:solidFill><a:ln w="12700"><a:solidFill><a:srgbClr val="0000FF"><a:alpha val="50000"/></a:srgbClr></a:solidFill></a:ln>"#;
    let (pdf, warnings) = pdf_of(alpha);
    assert!(pdf.contains("/ca 0.6") && pdf.contains("/CA 0.5"), "{pdf}");
    assert!(
        !warnings.iter().any(|w| w.contains("PDF output")),
        "{warnings:?}"
    );
}
#[test]
fn shadows_are_rasterized_under_vector_text_and_reported() {
    let shadow = r#"<a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:effectLst><a:outerShdw blurRad="38100" dist="38100" dir="2700000"><a:srgbClr val="000000"><a:alpha val="40000"/></a:srgbClr></a:outerShdw></a:effectLst>"#;
    let (pdf, warnings) = pdf_of(shadow);
    assert!(pdf.contains("/SMask"), "{pdf}");
    assert!(pdf.contains("/FlateDecode"));
    // The shape and its text are still drawn as vectors on top of the shadow raster.
    assert!(pdf.contains("/ActualText <FEFF3042>"));
    assert!(
        warnings.contains(&"Filter effects are rasterized in PDF output".to_string()),
        "{warnings:?}"
    );
    let pattern = r#"<a:pattFill prst="dkUpDiag"><a:fgClr><a:srgbClr val="000000"/></a:fgClr><a:bgClr><a:srgbClr val="FFFFFF"/></a:bgClr></a:pattFill>"#;
    let (pdf, warnings) = pdf_of(pattern);
    assert!(pdf.contains("/Subtype /Image"));
    assert!(
        warnings.iter().any(|w| w.contains("Pattern fills")),
        "{warnings:?}"
    );
}
#[test]
fn strict_mode_leaves_no_pdf_or_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.pptx");
    let out = dir.path().join("out.pdf");
    fixture(&input, "", false);
    let mut opts = options();
    opts.strict = true;
    let error = convert_pdf(&input, &out, &opts).unwrap_err();
    assert!(error.downcast_ref::<rhodopsin::StrictModeError>().is_some());
    assert!(!out.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn all_embedding_restrictions_are_rejected_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.pptx");
    fixture(&input, "", false);
    let original = fs::read(&options().font_file[0]).unwrap();
    let count = u16::from_be_bytes(original[4..6].try_into().unwrap()) as usize;
    let entry = (0..count)
        .map(|i| 12 + i * 16)
        .find(|i| &original[*i..*i + 4] == b"OS/2")
        .unwrap();
    let offset = u32::from_be_bytes(original[entry + 8..entry + 12].try_into().unwrap()) as usize;
    for bits in [2u16, 256, 512] {
        let mut data = original.clone();
        data[offset + 8..offset + 10].copy_from_slice(&bits.to_be_bytes());
        let font = dir.path().join(format!("font-{bits}.ttf"));
        fs::write(&font, data).unwrap();
        let mut opts = options();
        opts.font_file = vec![font];
        let out = dir.path().join(format!("out-{bits}.pdf"));
        let error = convert_pdf(&input, &out, &opts).unwrap_err();
        assert!(format!("{error:#}").contains("embedding flags"));
        assert!(!out.exists());
    }
}
#[cfg(unix)]
#[test]
fn dangling_output_symlink_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.pptx");
    fixture(&input, "", false);
    let out = dir.path().join("out.pdf");
    std::os::unix::fs::symlink("missing", &out).unwrap();
    assert!(convert_pdf(&input, &out, &options()).is_err());
    assert_eq!(fs::read_link(out).unwrap(), Path::new("missing"));
}
