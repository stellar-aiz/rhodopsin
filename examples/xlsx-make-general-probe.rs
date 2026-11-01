//! 開発専用。General の Excel 観測用ブックを生成する。生成物は正解 fixture ではない。
use std::{
    io::{Cursor, Write},
    path::Path,
};
use zip::{ZipWriter, write::SimpleFileOptions};
type Result<T> = std::result::Result<T, &'static str>;
const VALUES: &[&str] = &[
    "0",
    "1",
    "-1",
    "1234567890",
    "12345678901",
    "123456789012",
    "99999999999",
    "99999999999.5",
    "0.1",
    "0.0001",
    "0.00001",
    "0.000001",
    "0.00000001",
    "0.000123456789",
    "1.23456789012345",
    "-12345678901",
    "1E20",
    "1E-20",
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
    let mut rows = String::new();
    for (index, value) in VALUES.iter().enumerate() {
        let row = index + 1;
        rows.push_str(&format!(r#"<row r="{row}" ht="22" customHeight="1"><c r="A{row}" t="inlineStr"><is><t>{value}</t></is></c>"#));
        for column in ['B', 'C', 'D', 'E'] {
            rows.push_str(&format!(r#"<c r="{column}{row}" s="0"><v>{value}</v></c>"#));
        }
        rows.push_str("</row>");
    }
    let styles = format!(
        r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><color rgb="FF000000"/><name val="{}"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#,
        xml(font)
    );
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:E{}"/><sheetFormatPr defaultRowHeight="22" defaultColWidth="8"/><cols><col min="1" max="1" width="24" customWidth="1"/><col min="2" max="2" width="3" customWidth="1"/><col min="3" max="3" width="8" customWidth="1"/><col min="4" max="4" width="16" customWidth="1"/><col min="5" max="5" width="28" customWidth="1"/></cols><sheetData>{rows}</sheetData><pageMargins left="0.5" right="0.5" top="0.5" bottom="0.5" header="0" footer="0"/><pageSetup paperSize="9" orientation="portrait" scale="100"/></worksheet>"#,
        VALUES.len()
    );
    let book = format!(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr date1904="0"/><sheets><sheet name="General検証" sheetId="1" r:id="sheet"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">'General検証'!$A$1:$E${}</definedName></definedNames></workbook>"#,
        VALUES.len()
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
    if output
        .extension()
        .is_none_or(|extension| extension != "xlsx")
    {
        return Err("expected_xlsx_output");
    }
    rhodopsin::xlsx::output::check_available(output).map_err(|e| e.reason)?;
    let bytes = build(args[1].to_str().ok_or("invalid_probe_font")?)?;
    rhodopsin::xlsx::output::publish(output, &bytes).map_err(|e| e.reason)?;
    println!(
        "{}",
        serde_json::json!({"status":"created","kind":"synthetic_probe","cases":VALUES.len(),"numeric_columns":4,"reference_verified":false})
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
    #[test]
    fn 観測ブックの印刷範囲と数値と列幅を読み戻す() {
        let bytes = build("試験 & 書体").unwrap();
        assert_eq!(bytes, build("試験 & 書体").unwrap());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.xlsx");
        rhodopsin::xlsx::output::publish(&path, &bytes).unwrap();
        let limits = rhodopsin::xlsx::package::Limits::default();
        let package = rhodopsin::xlsx::package::Package::open(&path, &limits).unwrap();
        let workbook = rhodopsin::xlsx::workbook::inspect(&package, &limits).unwrap();
        assert_eq!(workbook.sheets.len(), 1);
        let sheet = &workbook.sheets[0];
        assert_eq!(sheet.cells.len(), VALUES.len() * 5);
        assert_eq!(
            sheet.print.areas[0],
            rhodopsin::xlsx::range::CellRange::parse("A1:E18").unwrap()
        );
        assert_eq!(
            sheet
                .grid
                .columns
                .iter()
                .map(|column| column.width.unwrap())
                .collect::<Vec<_>>(),
            [24.0, 3.0, 8.0, 16.0, 28.0]
        );
        for (row, value) in sheet.cells.chunks_exact(5).zip(VALUES) {
            assert_eq!(
                row[0].value,
                rhodopsin::xlsx::workbook::CellValue::Text((*value).into())
            );
            for cell in &row[1..] {
                assert_eq!(cell.numeric_lexeme.as_deref(), Some(*value));
            }
        }
        assert_eq!(
            workbook.styles.unwrap().fonts[0]
                .as_ref()
                .unwrap()
                .family
                .as_deref(),
            Some("試験 & 書体")
        );
    }
}
