//! 開発専用。分数の Excel 観測用ブックを生成する。生成物は正解 fixture ではない。
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Write},
    path::Path,
};
use zip::{ZipWriter, write::SimpleFileOptions};
type Result<T> = std::result::Result<T, &'static str>;
const VALUES: &[&str] = &[
    "0", "1", "-1", "0.1", "-0.1", "0.3", "-0.3", "2.3", "-2.3", "12.3", "1.5625", "0.5", "0.999",
    "0.125", "1.999", "12.34",
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
        for (column, style) in [('B', 1), ('C', 2), ('D', 1), ('E', 2)] {
            rows.push_str(&format!(
                r#"<c r="{column}{row}" s="{style}"><v>{value}</v></c>"#
            ));
        }
        rows.push_str("</row>");
    }
    let styles = format!(
        r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><color rgb="FF000000"/><name val="{}"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="12" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="13" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#,
        xml(font)
    );
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:E{}"/><sheetFormatPr defaultRowHeight="22" defaultColWidth="8"/><cols><col min="1" max="1" width="24" customWidth="1"/><col min="2" max="2" width="16" customWidth="1"/><col min="3" max="3" width="16" customWidth="1"/><col min="4" max="4" width="4" customWidth="1"/><col min="5" max="5" width="4" customWidth="1"/></cols><sheetData>{rows}</sheetData><pageMargins left="0.5" right="0.5" top="0.5" bottom="0.5" header="0" footer="0"/><pageSetup paperSize="9" orientation="portrait" scale="100"/></worksheet>"#,
        VALUES.len()
    );
    let book = format!(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr date1904="0"/><sheets><sheet name="分数検証" sheetId="1" r:id="sheet"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="0">'分数検証'!$A$1:$E${}</definedName></definedNames></workbook>"#,
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
fn record(bytes: &[u8], font: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "status": "created",
        "kind": "synthetic_fraction_probe",
        "input_sha256": format!("{:x}", Sha256::digest(bytes)),
        "reference_verified": false,
        "font": {"requested_family":font,"points":11,"actual_version":null,"file_sha256":null,"substituted":null},
        "print": {"area":format!("A1:E{}", VALUES.len()),"paper":"A4","orientation":"portrait","scale_percent":100,"margin_inches":0.5},
        "columns": [
            {"column":"A","kind":"source_text","width":24},
            {"column":"B","num_fmt_id":12,"width":16},
            {"column":"C","num_fmt_id":13,"width":16},
            {"column":"D","num_fmt_id":12,"width":4},
            {"column":"E","num_fmt_id":13,"width":4}
        ],
        "cases": VALUES.iter().enumerate().map(|(index,value)| serde_json::json!({
            "row":index+1,"numeric_lexeme":value,
            "binary64_hex":format!("{:016x}",value.parse::<f64>().expect("固定観測値").to_bits()),
            "excel_display":null
        })).collect::<Vec<_>>(),
        "references": {
            "excel":{"status":"unobserved","pdf_sha256":null,"app_version":null,"os":null,"locale":null,"input_sha256_after":null},
            "libreoffice":{"status":"unobserved","pdf_sha256":null,"app_version":null,"os":null,"locale":null,"input_sha256_after":null}
        }
    })
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
    let font = args[1].to_str().ok_or("invalid_probe_font")?;
    let bytes = build(font)?;
    rhodopsin::xlsx::output::publish(output, &bytes).map_err(|e| e.reason)?;
    println!("{}", record(&bytes, font));
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
        let record = record(&bytes, "試験 & 書体");
        assert_eq!(record["reference_verified"], false);
        assert_eq!(record["references"]["excel"]["status"], "unobserved");
        assert_eq!(record["references"]["libreoffice"]["status"], "unobserved");
        assert!(
            record["cases"]
                .as_array()
                .unwrap()
                .iter()
                .all(|case| case["excel_display"].is_null())
        );
        assert_eq!(
            record["input_sha256"],
            format!("{:x}", Sha256::digest(&bytes))
        );
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
            rhodopsin::xlsx::range::CellRange::parse("A1:E16").unwrap()
        );
        assert_eq!(
            sheet
                .grid
                .columns
                .iter()
                .map(|column| column.width.unwrap())
                .collect::<Vec<_>>(),
            [24.0, 16.0, 16.0, 4.0, 4.0]
        );
        for (row, value) in sheet.cells.chunks_exact(5).zip(VALUES) {
            assert_eq!(
                row[0].value,
                rhodopsin::xlsx::workbook::CellValue::Text((*value).into())
            );
            for (cell, style) in row[1..].iter().zip([1, 2, 1, 2]) {
                assert_eq!(cell.style, style);
                assert_eq!(cell.numeric_lexeme.as_deref(), Some(*value));
            }
        }
        let styles = workbook.styles.unwrap();
        assert_eq!(styles.cells[1].as_ref().unwrap().number_format, 12);
        assert_eq!(styles.cells[2].as_ref().unwrap().number_format, 13);
        assert_eq!(
            styles.fonts[0].as_ref().unwrap().family.as_deref(),
            Some("試験 & 書体")
        );
    }
}
