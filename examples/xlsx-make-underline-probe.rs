//! 開発専用。下線の Excel 観測入力を生成する。正解の描画寸法は含めない。
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Write},
    path::Path,
};
use zip::{ZipWriter, write::SimpleFileOptions};
type Result<T> = std::result::Result<T, &'static str>;
const KINDS: [&str; 5] = [
    "none",
    "single",
    "double",
    "singleAccounting",
    "doubleAccounting",
];
const LABELS: [&str; 8] = [
    "文字 11pt",
    "文字 20pt",
    "数値",
    "会計 正数",
    "会計 負数",
    "会計 ゼロ",
    "明示改行",
    "複数書式",
];
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn build(font: &str) -> Result<Vec<u8>> {
    if font.trim().is_empty() || font.len() > 1024 || font.chars().any(char::is_control) {
        return Err("invalid_probe_font");
    }
    let family = xml(font);
    let mut fonts = String::new();
    for points in [11, 20] {
        for kind in KINDS {
            fonts.push_str(&format!(r#"<font><sz val="{points}"/><color rgb="FF000000"/><name val="{family}"/><u val="{kind}"/></font>"#));
        }
    }
    let mut xfs = String::from(r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>"#);
    let mut rows = String::from(
        r#"<row r="1" ht="32" customHeight="1"><c r="A1" t="inlineStr"><is><t>下線観測</t></is></c>"#,
    );
    for (i, label) in ["なし", "一重", "二重", "会計一重", "会計二重"]
        .iter()
        .enumerate()
    {
        let col = char::from(b'B' + i as u8);
        rows.push_str(&format!(
            r#"<c r="{col}1" t="inlineStr"><is><t>{label}</t></is></c>"#
        ));
    }
    rows.push_str("</row>");
    for (i, label) in LABELS.iter().enumerate() {
        let row = i + 2;
        let height = if i == 6 { 60 } else { 44 };
        rows.push_str(&format!(r#"<row r="{row}" ht="{height}" customHeight="1"><c r="A{row}" t="inlineStr"><is><t>{label}</t></is></c>"#));
        for (j, kind) in KINDS.iter().enumerate() {
            let col = char::from(b'B' + j as u8);
            let style = 1 + i * 5 + j;
            let font_id = j + if i == 1 { 5 } else { 0 };
            let num_fmt = if (3..=5).contains(&i) {
                164
            } else if i == 2 {
                4
            } else {
                0
            };
            let wrap = if i == 6 { 1 } else { 0 };
            xfs.push_str(&format!(r#"<xf numFmtId="{num_fmt}" fontId="{font_id}" fillId="0" borderId="0" xfId="0" applyFont="1" applyNumberFormat="1" applyAlignment="1"><alignment vertical="bottom" wrapText="{wrap}"/></xf>"#));
            let value = match i {
                2..=5 => {
                    let raw = if i == 4 {
                        "-1234.5"
                    } else if i == 5 {
                        "0"
                    } else {
                        "1234.5"
                    };
                    format!("<v>{raw}</v>")
                }
                7 => format!(
                    r#"<is><r><rPr><rFont val="{family}"/><sz val="11"/><color rgb="FF000000"/><u val="{kind}"/></rPr><t>Ag</t></r><r><rPr><rFont val="{family}"/><sz val="20"/><color rgb="FF0000FF"/><u val="{kind}"/></rPr><t>jp</t></r></is>"#
                ),
                6 => "<is><t>Ag&#10;jp</t></is>".into(),
                _ => "<is><t>Agjp</t></is>".into(),
            };
            let typ = if (2..=5).contains(&i) {
                ""
            } else {
                " t=\"inlineStr\""
            };
            rows.push_str(&format!(
                r#"<c r="{col}{row}" s="{style}"{typ}>{value}</c>"#
            ));
        }
        rows.push_str("</row>");
    }
    let account = xml(r#"_("$"* #,##0.00_);_("$"* (#,##0.00);_("$"* "-"??_);_(@_)"#);
    let styles = format!(
        r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="{account}"/></numFmts><fonts count="10">{fonts}</fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="41">{xfs}</cellXfs><cellStyles count="1"><cellStyle name="Normal" builtinId="0" xfId="0"/></cellStyles></styleSheet>"#
    );
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:F9"/><sheetViews><sheetView workbookViewId="0" showGridLines="0"/></sheetViews><sheetFormatPr defaultRowHeight="44" defaultColWidth="16"/><cols><col min="1" max="6" width="16" customWidth="1"/></cols><sheetData>{rows}</sheetData><printOptions gridLines="0"/><pageMargins left="0.5" right="0.5" top="0.5" bottom="0.5" header="0" footer="0"/><pageSetup paperSize="9" orientation="landscape" scale="100"/></worksheet>"#
    );
    let book = String::from(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr date1904="0"/><sheets><sheet name="下線観測" sheetId="1" r:id="sheet"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">'下線観測'!$A$1:$F$9</definedName></definedNames></workbook>"#,
    );
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="book" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="sheet" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        ),
        ("xl/workbook.xml", &book),
        ("xl/styles.xml", &styles),
        ("xl/worksheets/sheet1.xml", &sheet),
    ];
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, value) in parts {
        zip.start_file(
            name,
            SimpleFileOptions::default().last_modified_time(zip::DateTime::default()),
        )
        .map_err(|_| "probe_archive_failed")?;
        zip.write_all(value.as_bytes())
            .map_err(|_| "probe_archive_failed")?;
    }
    Ok(zip
        .finish()
        .map_err(|_| "probe_archive_failed")?
        .into_inner())
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected_output_xlsx_and_font_family");
    }
    let output = Path::new(&args[0]);
    if output.extension().is_none_or(|x| x != "xlsx") {
        return Err("expected_xlsx_output");
    }
    rhodopsin::xlsx::output::check_available(output).map_err(|e| e.reason)?;
    let font = args[1].to_str().ok_or("invalid_probe_font")?;
    let bytes = build(font)?;
    rhodopsin::xlsx::output::publish(output, &bytes).map_err(|e| e.reason)?;
    println!(
        "{}",
        serde_json::json!({"schema_version":1,"status":"created","kind":"synthetic_probe","cases":40,"input_sha256":format!("{:x}",Sha256::digest(&bytes)),"font_requested":font,"font_actual":null,"underline_kinds":KINDS,"row_cases":LABELS,"print_area":"A1:F9","paper":"A4","orientation":"landscape","scale":100,"excel_reference":"unobserved","libreoffice_reference":"unobserved","reference_verified":false})
    );
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
    use std::io::Read;
    #[test]
    fn 四十条件の下線と書体と印刷条件を保存する() {
        let bytes = build("試験 & 書体").unwrap();
        assert_eq!(bytes, build("試験 & 書体").unwrap());
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut read = |name: &str| {
            let mut text = String::new();
            archive
                .by_name(name)
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            text
        };
        let text = read("xl/styles.xml");
        let doc = roxmltree::Document::parse(&text).unwrap();
        let parsed = rhodopsin::xlsx::styles::Styles::parse(doc.root_element()).unwrap();
        assert_eq!(parsed.fonts.len(), 10);
        for (index, font) in parsed.fonts.iter().enumerate() {
            let font = font.as_ref().unwrap();
            assert_eq!(font.family.as_deref(), Some("試験 & 書体"));
            assert_eq!(
                font.underline,
                [
                    rhodopsin::xlsx::styles::Underline::None,
                    rhodopsin::xlsx::styles::Underline::Single,
                    rhodopsin::xlsx::styles::Underline::Double,
                    rhodopsin::xlsx::styles::Underline::SingleAccounting,
                    rhodopsin::xlsx::styles::Underline::DoubleAccounting
                ][index % 5]
            );
        }
        let text = read("xl/worksheets/sheet1.xml");
        let doc = roxmltree::Document::parse(&text).unwrap();
        let cells: Vec<_> = doc
            .descendants()
            .filter(|n| {
                n.has_tag_name((
                    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                    "c",
                ))
            })
            .collect();
        assert_eq!(cells.len(), 54);
        for i in 0..8 {
            for j in 0..5 {
                let cell = cells[(i + 1) * 6 + j + 1];
                assert_eq!(
                    cell.attribute("s"),
                    Some((1 + i * 5 + j).to_string().as_str())
                );
                if i == 7 {
                    let underlines: Vec<_> = cell
                        .descendants()
                        .filter(|n| n.tag_name().name() == "u")
                        .map(|n| n.attribute("val").unwrap())
                        .collect();
                    assert_eq!(underlines, [KINDS[j], KINDS[j]]);
                }
            }
        }
        let setup = doc
            .descendants()
            .find(|n| n.tag_name().name() == "pageSetup")
            .unwrap();
        assert_eq!(setup.attribute("scale"), Some("100"));
        assert_eq!(setup.attribute("orientation"), Some("landscape"));
        let text = read("xl/workbook.xml");
        assert!(text.contains("'下線観測'!$A$1:$F$9"));
    }
    #[test]
    fn 空と制御文字を含む書体指定を拒否する() {
        for name in ["", " ", "A\nB"] {
            assert_eq!(build(name).unwrap_err(), "invalid_probe_font");
        }
    }
}
