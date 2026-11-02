use rhodopsin::xlsx::{
    package::{Limits, Package},
    workbook,
};
use std::{fs::File, io::Write, path::Path};
use tempfile::TempDir;
use zip::{ZipWriter, write::SimpleFileOptions};

fn archive(path: &Path, parts: &[(&str, &str)]) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    for (name, data) in parts {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn fixture(dir: &TempDir, body: &str) -> std::path::PathBuf {
    fixture_with_book(dir, body, "", "")
}

fn fixture_with_book(
    dir: &TempDir,
    body: &str,
    extra: &str,
    more_sheets: &str,
) -> std::path::PathBuf {
    fixture_with_styles(dir, body, extra, more_sheets, None)
}

fn fixture_with_styles(
    dir: &TempDir,
    body: &str,
    extra: &str,
    more_sheets: &str,
    styles: Option<&str>,
) -> std::path::PathBuf {
    let path = dir.path().join("機密の顧客名.xlsx");
    let style_rel = if styles.is_some() {
        r#"<Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#
    } else {
        ""
    };
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>{style_rel}</Relationships>"#
    );
    let book = format!(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="機密の案件名" sheetId="1" r:id="rId1"/>{more_sheets}</sheets>{extra}</workbook>"#
    );
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</worksheet>"#
    );
    archive(
        &path,
        &[
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            ("xl/workbook.xml", &book),
            ("xl/_rels/workbook.xml.rels", &rels),
            ("xl/worksheets/sheet1.xml", &sheet),
            ("xl/styles.xml", styles.unwrap_or("")),
            (
                "xl/worksheets/sheet2.xml",
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
            ),
        ],
    );
    path
}

#[test]
fn スライサー既定名があってもシート上のスライサーは拒否する() {
    let dir = TempDir::new().unwrap();
    let path = fixture_with_styles(
        &dir,
        r#"<sheetData/><extLst><ext uri="{A8765BA9-456A-4DAB-B4F3-ACF838C121DE}"><x:slicerList xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"/></ext></extLst>"#,
        "",
        "",
        Some(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><extLst><ext uri="{EB79DEF2-80B8-43e5-95BD-54CBDDF9020C}"><x:slicerStyles xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" defaultSlicerStyle="SlicerStyleLight1"/></ext></extLst></styleSheet>"#,
        ),
    );
    let limits = Limits::default();
    let package = Package::open(&path, &limits).unwrap();
    let error = workbook::inspect(&package, &limits).unwrap_err();
    assert_eq!(error.reason, "worksheet_extension");
    assert_eq!(error.location.unwrap().sheet_index, 1);
}

#[test]
fn 保存済みの数式結果を読む() {
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="A1"><f>2+3</f><v>5</v></c><c r="B1" t="inlineStr"><is><t>見積書</t></is></c></row></sheetData>"#,
    );
    let limits = Limits::default();
    let package = Package::open(&path, &limits).unwrap();
    let book = workbook::inspect(&package, &limits).unwrap();
    assert_eq!(
        book.sheets[0].cells[0].value,
        workbook::CellValue::Number(5.0)
    );
    assert_eq!(
        book.sheets[0].cells[1].value,
        workbook::CellValue::Text("見積書".into())
    );
}

#[test]
fn 空の数値キャッシュと空文字列の計算結果を区別する() {
    for cell_type in ["", r#" t="n""#] {
        let dir = TempDir::new().unwrap();
        let path = fixture(
            &dir,
            &format!(
                r#"<sheetData><row r="1"><c r="A1"{cell_type}><f>機密の数式</f><v/></c></row></sheetData>"#
            ),
        );
        let out = dir.path().join("out.pdf");
        let error = rhodopsin::xlsx::convert(&path, &out, &Limits::default()).unwrap_err();
        assert_eq!(error.category.exit_code(), 2);
        assert_eq!(error.reason, "formula_cached_value_missing");
        let location = error.location.clone().unwrap();
        assert_eq!(location.sheet_index, 1);
        assert_eq!(location.cell.as_deref(), Some("A1"));
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
        assert!(!out.exists());
    }
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="A1" t="str"><f>機密の数式</f><v/></c></row></sheetData>"#,
    );
    let limits = Limits::default();
    let package = Package::open(&path, &limits).unwrap();
    let book = workbook::inspect(&package, &limits).unwrap();
    assert_eq!(
        book.sheets[0].cells[0].value,
        workbook::CellValue::Text("".into())
    );
}

#[test]
fn 数式結果なしは位置だけを返す() {
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="A1"><f>機密の数式</f></c></row></sheetData>"#,
    );
    let out = dir.path().join("result.pdf");
    let error = rhodopsin::xlsx::convert(&path, &out, &Limits::default()).unwrap_err();
    assert_eq!(error.category.exit_code(), 2);
    assert_eq!(error.reason, "formula_cached_value_missing");
    let location = error.location.clone().unwrap();
    assert_eq!(location.sheet_index, 1);
    assert_eq!(location.cell.as_deref(), Some("A1"));
    assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    assert!(!out.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn 不正な番地を診断へコピーしない() {
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="機密の文字列"><v>1</v></c></row></sheetData>"#,
    );
    let err = rhodopsin::xlsx::convert(&path, &dir.path().join("out.pdf"), &Limits::default())
        .unwrap_err();
    assert_eq!(err.reason, "invalid_cell_address");
    assert!(!serde_json::to_string(&err).unwrap().contains("機密"));
}

#[test]
fn 既存ファイルを保持する() {
    let dir = TempDir::new().unwrap();
    let path = fixture(&dir, "<sheetData/>");
    let out = dir.path().join("out.pdf");
    std::fs::write(&out, b"original").unwrap();
    let err = rhodopsin::xlsx::convert(&path, &out, &Limits::default()).unwrap_err();
    assert_eq!(err.reason, "output_already_exists");
    assert_eq!(std::fs::read(&out).unwrap(), b"original");
}

#[test]
fn 展開上限とセル数上限を適用する() {
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>"#,
    );
    let limits = Limits {
        expanded_bytes: 100,
        ..Limits::default()
    };
    assert_eq!(
        Package::open(&path, &limits).err().unwrap().reason,
        "expanded_size_limit"
    );
    let limits = Limits {
        cells: 0,
        ..Limits::default()
    };
    let package = Package::open(&path, &limits).unwrap();
    assert_eq!(
        workbook::inspect(&package, &limits).unwrap_err().reason,
        "cell_limit"
    );
}

#[test]
fn 対応外の条件付き書式で止まる() {
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData/><conditionalFormatting sqref="A1"><cfRule type="expression"/></conditionalFormatting>"#,
    );
    let err = rhodopsin::xlsx::convert(&path, &dir.path().join("out.pdf"), &Limits::default())
        .unwrap_err();
    assert_eq!(err.category.exit_code(), 3);
    assert_eq!(err.reason, "conditional_formatting");
}

#[test]
fn 不正なパッケージと暗号化形式を拒否する() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.xlsx");
    archive(&path, &[("../private.xml", "機密")]);
    assert_eq!(
        Package::open(&path, &Limits::default())
            .err()
            .unwrap()
            .reason,
        "invalid_part_path"
    );
    std::fs::write(&path, [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]).unwrap();
    assert_eq!(
        Package::open(&path, &Limits::default())
            .err()
            .unwrap()
            .reason,
        "encrypted_or_legacy_workbook"
    );
}

#[test]
fn 番地の境界を確認する() {
    for valid in ["A1", "XFD1048576", "ZZ3"] {
        assert!(workbook::cell_address(valid).is_some());
    }
    for invalid in [
        "XFE1", "A1048577", "A0", "A01", "a1", "A1機密", "機密", "1", "A", "",
    ] {
        assert!(workbook::cell_address(invalid).is_none());
    }
}

#[test]
fn ブックから印刷範囲と繰り返しと日付基準を読む() {
    let dir = TempDir::new().unwrap();
    let path = fixture_with_book(
        &dir,
        r#"<sheetFormatPr defaultRowHeight="15"/><cols><col min="1" max="2" width="10" style="2"/></cols><sheetData><row r="1"><c r="A1"><v>3</v></c></row></sheetData><pageSetup paperSize="9"/>"#,
        r#"<workbookPr date1904="1"/><definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">'機密の案件名'!$A$1:$D$20,'機密の案件名'!$F$1:$G$20</definedName><definedName name="_xlnm.Print_Titles" localSheetId="0">'機密の案件名'!$1:$2,'機密の案件名'!$A:$B</definedName></definedNames>"#,
        "",
    );
    let limits = Limits::default();
    let package = Package::open(&path, &limits).unwrap();
    let book = workbook::inspect(&package, &limits).unwrap();
    assert!(book.date_1904);
    let sheet = &book.sheets[0];
    assert_eq!(sheet.print.areas.len(), 2);
    assert_eq!(sheet.print.repeat_rows.unwrap().last_row, 2);
    assert_eq!(sheet.print.repeat_columns.unwrap().last_column, 2);
    assert_eq!(sheet.cells[0].style, 2);
}

#[test]
fn 方向省略の可視シートも混在検査するが非表示は除く() {
    for hidden in [false, true] {
        let dir = TempDir::new().unwrap();
        let state = if hidden { "hidden" } else { "visible" };
        let path = fixture_with_book(
            &dir,
            r#"<sheetData/><pageSetup orientation="landscape"/>"#,
            "",
            &format!(r#"<sheet name="機密2" sheetId="2" r:id="rId2" state="{state}"/>"#),
        );
        let limits = Limits::default();
        let package = Package::open(&path, &limits).unwrap();
        let result = workbook::inspect(&package, &limits);
        if hidden {
            assert!(result.is_ok());
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.reason, "mixed_page_orientation");
            assert_eq!(error.location.unwrap().sheet_index, 2);
        }
    }
}

#[test]
fn 回転文字の診断には使用セルの位置だけを付ける() {
    let dir = TempDir::new().unwrap();
    let styles = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font><name val="機密書体"/></font></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf/><xf><alignment textRotation="90"/></xf></cellXfs></styleSheet>"#;
    let path = fixture_with_styles(
        &dir,
        r#"<sheetData><row r="7"><c r="C7" s="1" t="inlineStr"><is><t>機密の金額</t></is></c></row></sheetData>"#,
        "",
        "",
        Some(styles),
    );
    let out = dir.path().join("out.pdf");
    let error = rhodopsin::xlsx::convert(&path, &out, &Limits::default()).unwrap_err();
    assert_eq!(error.category.exit_code(), 3);
    assert_eq!(error.reason, "rotated_text");
    let location = error.location.clone().unwrap();
    assert_eq!(location.sheet_index, 1);
    assert_eq!(location.cell.as_deref(), Some("C7"));
    assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    assert!(!out.exists());
}

#[test]
fn 未対応のセル属性と要素を位置付きで拒否する() {
    for (cell, reason) in [
        (r#"<c r="A1" unknown="機密"><v>1</v></c>"#, "cell_attribute"),
        (r#"<c r="A1" cm="1"><v>1</v></c>"#, "cell_metadata"),
        (r#"<c r="A1" ph="1"><v>1</v></c>"#, "phonetic_display"),
        (
            r#"<c r="A1"><future>機密</future><v>1</v></c>"#,
            "cell_element",
        ),
        (r#"<c r="A1"><v>1</v><v>2</v></c>"#, "duplicate_xml_element"),
        (
            r#"<c r="A1" t="inlineStr"><v>1</v><is><t>機密</t></is></c>"#,
            "conflicting_cell_content",
        ),
        (
            r#"<c r="A1" t="inlineStr"><is><t>機密</t><rPh><t>秘密</t></rPh></is></c>"#,
            "inline_string_element",
        ),
        (r#"<c r="A2"><v>1</v></c>"#, "cell_row_mismatch"),
    ] {
        let dir = TempDir::new().unwrap();
        let path = fixture(
            &dir,
            &format!("<sheetData><row r=\"1\">{cell}</row></sheetData>"),
        );
        let limits = Limits::default();
        let package = Package::open(&path, &limits).unwrap();
        let error = workbook::inspect(&package, &limits).unwrap_err();
        assert_eq!(error.reason, reason);
        assert_eq!(error.location.as_ref().unwrap().sheet_index, 1);
        assert!(error.location.as_ref().unwrap().cell.is_some());
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
}

#[test]
fn インラインと数式文字列キャッシュのエスケープを復号する() {
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>_x005f_x0041_見積</t></is></c><c r="B1" t="str"><f>機密の数式</f><v>_xD83D__xDE00_</v></c></row></sheetData>"#,
    );
    let limits = Limits::default();
    let package = Package::open(&path, &limits).unwrap();
    let book = workbook::inspect(&package, &limits).unwrap();
    assert_eq!(
        book.sheets[0].cells[0].value,
        workbook::CellValue::Text("_x0041_見積".into())
    );
    assert_eq!(
        book.sheets[0].cells[1].value,
        workbook::CellValue::Text("😀".into())
    );
}

#[test]
fn 共有文字列も一度だけ復号して参照共有する() {
    use std::io::Read;
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>0</v></c></row></sheetData>"#,
    );
    let mut zip = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
    let mut parts = Vec::new();
    for i in 0..zip.len() {
        let mut part = zip.by_index(i).unwrap();
        let name = part.name().to_owned();
        let mut data = String::new();
        part.read_to_string(&mut data).unwrap();
        if name == "xl/_rels/workbook.xml.rels" {
            data = data.replace("</Relationships>",r#"<Relationship Id="shared" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#);
        }
        parts.push((name, data));
    }
    drop(zip);
    parts.push(("xl/sharedStrings.xml".into(),r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>_x005F_x0041_見積</t></si></sst>"#.into()));
    archive(
        &path,
        &parts
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect::<Vec<_>>(),
    );
    let limits = Limits::default();
    let package = Package::open(&path, &limits).unwrap();
    let book = workbook::inspect(&package, &limits).unwrap();
    match (
        &book.sheets[0].cells[0].value,
        &book.sheets[0].cells[1].value,
    ) {
        (workbook::CellValue::Text(a), workbook::CellValue::Text(b)) => {
            assert_eq!(&**a, "_x0041_見積");
            assert!(std::sync::Arc::ptr_eq(a, b));
        }
        _ => panic!("文字列であること"),
    }
}

#[path = "xlsx_support/mod.rs"]
mod support;
#[test]
fn 明示したフォントを読み込んで準備段へ渡す() {
    let dir = TempDir::new().unwrap();
    let path = fixture(&dir, r#"<sheetData/>"#);
    let font = dir.path().join("試験.ttf");
    std::fs::write(&font, support::font("試験", false, false, 0)).unwrap();
    let options = rhodopsin::xlsx::ConversionOptions {
        font_sources: rhodopsin::xlsx::font_sources::Sources {
            files: vec![font.clone()],
            ..Default::default()
        },
        ..Default::default()
    };
    let prepared = rhodopsin::xlsx::prepare(&path, &Limits::default(), &options).unwrap();
    assert_eq!(prepared.workbook.sheets.len(), 1);
    assert_eq!(
        prepared
            .fonts
            .select("試験", false, false, None)
            .unwrap()
            .index,
        0
    );
    let output = dir.path().join("out.pdf");
    let error = rhodopsin::xlsx::convert_with_options(&path, &output, &Limits::default(), &options)
        .unwrap_err();
    assert_eq!(error.reason, "cell_styles_unresolved");
    assert!(!output.exists());
}
#[test]
fn フォント読み込みエラーに指定パスを漏らさない() {
    let dir = TempDir::new().unwrap();
    let path = fixture(&dir, r#"<sheetData/>"#);
    let output = dir.path().join("out.pdf");
    let missing = dir.path().join("機密の書体");
    for sources in [
        rhodopsin::xlsx::font_sources::Sources {
            files: vec![missing.clone()],
            directories: Vec::new(),
        },
        rhodopsin::xlsx::font_sources::Sources {
            files: Vec::new(),
            directories: vec![missing.clone()],
        },
    ] {
        let options = rhodopsin::xlsx::ConversionOptions {
            font_sources: sources,
            ..Default::default()
        };
        let error =
            rhodopsin::xlsx::convert_with_options(&path, &output, &Limits::default(), &options)
                .unwrap_err();
        assert_eq!(
            error.category,
            rhodopsin::xlsx::diagnostic::Category::MissingFont
        );
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
        assert!(!output.exists());
    }
}

#[test]
fn セルの明示スタイルを行列の既定より優先する() {
    let dir = TempDir::new().unwrap();
    let styles = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf/><xf/><xf/></cellXfs></styleSheet>"#;
    let path = fixture_with_styles(
        &dir,
        r#"<cols><col min="1" max="3" style="2"/></cols><sheetData><row r="1" s="1" customFormat="1"><c r="A1" s="0"/><c r="B1"/></row><row r="2"><c r="A2"/><c r="D2"/></row></sheetData>"#,
        "",
        "",
        Some(styles),
    );
    let limits = Limits::default();
    let package = Package::open(&path, &limits).unwrap();
    let book = workbook::inspect(&package, &limits).unwrap();
    let sheet = &book.sheets[0];
    assert_eq!(
        sheet
            .cells
            .iter()
            .map(|cell| (cell.address.as_str(), cell.style))
            .collect::<Vec<_>>(),
        [("A1", 0), ("B1", 1), ("A2", 2), ("D2", 0)]
    );
    assert_eq!(sheet.grid.style_spans(1, 3, 4).unwrap()[0].style, 1);
    assert_eq!(sheet.grid.style_spans(2, 2, 3).unwrap()[0].style, 2);
}

#[test]
fn 変換設定の書体代用を準備済み入力からセル描画と警告まで渡す() {
    use rhodopsin::xlsx::{
        cell_text, format::date::Locale, pdf::graphics::BoxRect, theme::TextScript,
        warnings::Warnings,
    };
    let dir = TempDir::new().unwrap();
    let path = fixture_with_styles(
        &dir,
        r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>ABC</t></is></c></row></sheetData>"#,
        "",
        "",
        Some(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font><name val="不足書体"/><sz val="10"/><color rgb="FF000000"/></font></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf/></cellXfs></styleSheet>"#,
        ),
    );
    let font = dir.path().join("代用.ttf");
    std::fs::write(&font, support::font("代用書体", false, false, 0)).unwrap();
    for fallback in [None, Some("代用書体".to_string())] {
        let options = rhodopsin::xlsx::ConversionOptions {
            font_sources: rhodopsin::xlsx::font_sources::Sources {
                files: vec![font.clone()],
                ..Default::default()
            },
            fallback_family: fallback.clone(),
            ..Default::default()
        };
        let prepared = rhodopsin::xlsx::prepare(&path, &Limits::default(), &options).unwrap();
        let context = prepared
            .text_context(&options, Locale::Japanese, TextScript::Latin, 10)
            .unwrap();
        let rect = BoxRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 20.0,
        };
        let mut warnings = Warnings::default();
        let result = cell_text::plan(
            &prepared.workbook.sheets[0].cells[0],
            1,
            cell_text::Frame {
                content: rect,
                clip: rect,
                scale: 1.0,
                baseline_step: None,
                line_box: None,
            },
            &context,
            &mut warnings,
        );
        if fallback.is_some() {
            let text = result.unwrap();
            assert_eq!(text[0].pdf().text, "ABC");
            let report = serde_json::to_value(warnings.into_summaries()).unwrap();
            assert_eq!(report[0]["kind"], "font_substitution");
            assert_eq!(
                report[0]["locations"][0],
                serde_json::json!({"sheet_index":1,"cell":"A1"})
            );
        } else {
            assert_eq!(result.err().unwrap().reason, "font_family_missing");
            assert!(warnings.into_summaries().is_empty());
        }
    }
}

#[test]
fn 代用書体オプションを検証し値を診断へ漏らさない() {
    let dir = TempDir::new().unwrap();
    let path = fixture(&dir, "<sheetData/>");
    let output = dir.path().join("out.pdf");
    for (name, reason, code) in [
        ("明示した代用書体", "cell_styles_unresolved", 3),
        ("機密\n書体", "invalid_fallback_font", 2),
        (" ", "invalid_fallback_font", 2),
    ] {
        let options = rhodopsin::xlsx::ConversionOptions {
            fallback_family: Some(name.into()),
            ..Default::default()
        };
        let error =
            rhodopsin::xlsx::convert_with_options(&path, &output, &Limits::default(), &options)
                .unwrap_err();
        assert_eq!(error.category.exit_code(), code);
        assert_eq!(error.reason, reason);
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
        assert!(!output.exists());
    }
}

#[test]
fn xlsx入力から準備組版pdf生成と排他的保存まで接続する() {
    use rhodopsin::xlsx::{
        cell_text::Frame,
        format::date::Locale,
        render::{Environment, Limits as RenderLimits},
        theme::TextScript,
    };
    let dir = TempDir::new().unwrap();
    let input = fixture_with_styles(
        &dir,
        r#"<sheetFormatPr defaultRowHeight="20" defaultColWidth="10"/><sheetData><row r="1"><c r="A1"><v>1234</v></c></row></sheetData><pageMargins left="0.5" right="0.5" top="0.5" bottom="0.5" header="0" footer="0"/><pageSetup paperSize="9" orientation="portrait" scale="100"/>"#,
        r#"<definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">'機密の案件名'!$A$1:$B$2</definedName></definedNames>"#,
        "",
        Some(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font><name val="試験"/><sz val="10"/><color rgb="FF000000"/></font></fonts><fills><fill/></fills><borders><border/></borders><cellStyleXfs><xf/></cellStyleXfs><cellStyles><cellStyle builtinId="0" xfId="0"/></cellStyles><cellXfs><xf numFmtId="3"/></cellXfs></styleSheet>"#,
        ),
    );
    let font = dir.path().join("font.ttf");
    std::fs::write(&font, support::font("試験", false, false, 0)).unwrap();
    let options = rhodopsin::xlsx::ConversionOptions {
        font_sources: rhodopsin::xlsx::font_sources::Sources {
            files: vec![font],
            directories: vec![],
        },
        ..Default::default()
    };
    let environment = Environment {
        locale: Locale::Japanese,
        script: TextScript::Latin,
        normal_script: TextScript::Latin,
        automatic_color: None,
        foreground: None,
        background: None,
    };
    // テスト専用の余白なし配置。Excel の位置一致を示すものではない。
    let frame = |_: &rhodopsin::xlsx::workbook::Sheet,
                 page: &rhodopsin::xlsx::layout::Page,
                 _: &rhodopsin::xlsx::workbook::Cell,
                 placement: &rhodopsin::xlsx::placement::RangePlacement| {
        Ok(Frame {
            content: placement.rect,
            clip: placement.rect,
            scale: page.scale,
            baseline_step: None,
            line_box: None,
        })
    };
    let convert = |output: &Path| {
        rhodopsin::xlsx::convert_with_policy(
            &input,
            output,
            &Limits::default(),
            &options,
            &environment,
            &RenderLimits::default(),
            frame,
        )
    };
    let output = dir.path().join("output.pdf");
    let report = convert(&output).unwrap();
    assert_eq!(report.page_count, 1);
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("機密"));
    let pdf = lopdf::Document::load(&output).unwrap();
    assert_eq!(pdf.get_pages().len(), report.page_count);
    assert_eq!(pdf.extract_text(&[1]).unwrap().trim(), "1,234");
    let bytes = std::fs::read(&output).unwrap();
    let second = dir.path().join("second.pdf");
    convert(&second).unwrap();
    assert_eq!(bytes, std::fs::read(&second).unwrap());
    assert_eq!(
        convert(&output).unwrap_err().reason,
        "output_already_exists"
    );
    assert_eq!(std::fs::read(&output).unwrap(), bytes);

    let failed = dir.path().join("failed.pdf");
    let error = rhodopsin::xlsx::convert_with_policy(
        &input,
        &failed,
        &Limits::default(),
        &options,
        &environment,
        &RenderLimits::default(),
        |_, _, _, _| {
            Err(rhodopsin::xlsx::diagnostic::Failure::unsupported(
                "unresolved_test_policy",
            ))
        },
    )
    .unwrap_err();
    assert_eq!(error.reason, "unresolved_test_policy");
    assert!(!failed.exists());
    assert_eq!(
        convert(&dir.path().join("missing/output.pdf"))
            .unwrap_err()
            .reason,
        "output_directory_unwritable"
    );

    // 入力処理中に別の書き手が確定しても、その出力を保持して成功を返さない。
    let raced = dir.path().join("raced.pdf");
    let error = rhodopsin::xlsx::convert_with_policy(
        &input,
        &raced,
        &Limits::default(),
        &options,
        &environment,
        &RenderLimits::default(),
        |sheet, page, cell, placement| {
            std::fs::write(&raced, b"existing").unwrap();
            frame(sheet, page, cell, placement)
        },
    )
    .unwrap_err();
    assert_eq!(error.reason, "output_already_exists");
    assert_eq!(std::fs::read(&raced).unwrap(), b"existing");
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".rhodopsin_xlsx-")
    }));
}

#[test]
fn リッチ共有文字列は書式を保持して参照共有する() {
    use std::io::Read;
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>0</v></c></row></sheetData>"#,
    );
    let mut zip = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
    let mut parts = Vec::new();
    for i in 0..zip.len() {
        let mut part = zip.by_index(i).unwrap();
        let name = part.name().to_owned();
        let mut data = String::new();
        part.read_to_string(&mut data).unwrap();
        if name == "xl/_rels/workbook.xml.rels" {
            data = data.replace("</Relationships>",r#"<Relationship Id="shared" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#);
        }
        parts.push((name, data));
    }
    drop(zip);
    parts.push(("xl/sharedStrings.xml".into(),r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><r><t>_x005F_x0041_見積</t></r><r><rPr><rFont val="試験"/><sz val="14"/><b/><color rgb="FFFF0000"/></rPr><t xml:space="preserve"> A</t></r><rPh sb="0" eb="1"><t>ヨミ</t></rPh><phoneticPr fontId="0"/></si></sst>"#.into()));
    archive(
        &path,
        &parts
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect::<Vec<_>>(),
    );
    let limits = Limits::default();
    let package = Package::open(&path, &limits).unwrap();
    let book = workbook::inspect(&package, &limits).unwrap();
    match (
        &book.sheets[0].cells[0].value,
        &book.sheets[0].cells[1].value,
    ) {
        (workbook::CellValue::RichText(a), workbook::CellValue::RichText(b)) => {
            assert_eq!(&*a[0].text, "_x0041_見積");
            assert_eq!(&*a[1].text, " A");
            assert!(a[0].font.is_none());
            assert!(a[1].font.as_ref().unwrap().bold);
            assert!(std::sync::Arc::ptr_eq(a, b));
        }
        _ => panic!("文字列であること"),
    }
}

#[test]
fn インライン文字列でも書式と復号を保持し未知要素を拒否する() {
    let dir = TempDir::new().unwrap();
    let path = fixture(
        &dir,
        r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><r><t>_x0041_</t></r><r><rPr><rFont val="試験"/><sz val="12"/><i/></rPr><t xml:space="preserve"> B</t></r></is></c></row></sheetData>"#,
    );
    let limits = Limits::default();
    let book = workbook::inspect(&Package::open(&path, &limits).unwrap(), &limits).unwrap();
    let workbook::CellValue::RichText(runs) = &book.sheets[0].cells[0].value else {
        panic!("リッチテキストであること")
    };
    assert_eq!(&*runs[0].text, "A");
    assert_eq!(&*runs[1].text, " B");
    assert!(runs[1].font.as_ref().unwrap().italic);
    for body in [
        "<t>A</t><r><t>B</t></r>",
        "<r><t>A</t><t>B</t></r>",
        "<r><rPr><sz val=\"10\"/><sz val=\"12\"/></rPr><t>A</t></r>",
        "<r><rPr><strike/></rPr><t>A</t></r>",
        "<r><rPr><b secret=\"1\"/></rPr><t>A</t></r>",
        "<r><rPr/><unknown/><t>A</t></r>",
    ] {
        let path = fixture(
            &dir,
            &format!(
                r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is>{body}</is></c></row></sheetData>"#
            ),
        );
        let error =
            workbook::inspect(&Package::open(&path, &limits).unwrap(), &limits).unwrap_err();
        assert_eq!(error.location.unwrap().cell.as_deref(), Some("A1"));
    }
}
