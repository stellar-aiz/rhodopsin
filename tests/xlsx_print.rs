use rhodopsin::xlsx::{
    grid::Grid,
    print::{Orientation, PageOrder, PrintSettings, Scaling},
    range::{CellRange, references, title_range},
};

fn sheet(body: &str) -> String {
    format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</worksheet>"#
    )
}

#[test]
fn 倍率とページ収容を区別して保持する() {
    for (fit, expected) in [
        (false, Scaling::Percent(80)),
        (
            true,
            Scaling::Fit {
                width: 1,
                height: 0,
            },
        ),
    ] {
        let xml = sheet(&format!(
            r#"<sheetPr><pageSetUpPr fitToPage="{fit}"/></sheetPr><pageSetup paperSize="9" orientation="landscape" scale="80" fitToWidth="1" fitToHeight="0" pageOrder="overThenDown"/>"#
        ));
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let print = PrintSettings::parse(doc.root_element()).unwrap();
        assert_eq!(print.scaling, expected);
        assert_eq!(print.paper_size, Some(9));
        assert_eq!(print.orientation, Orientation::Landscape);
        assert_eq!(print.order, PageOrder::OverThenDown);
    }
}

#[test]
fn 余白はポイントへ変換し中央寄せを保持する() {
    let xml = sheet(
        r#"<pageMargins left="0.5" right="0.25" top="1" bottom="0.75" header="0.1" footer="0.2"/><printOptions horizontalCentered="true" verticalCentered="1"/>"#,
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let print = PrintSettings::parse(doc.root_element()).unwrap();
    assert!(print.horizontal_center && print.vertical_center);
    let margins = print.margins.unwrap();
    assert_eq!(
        (margins.left, margins.right, margins.top, margins.bottom),
        (36.0, 18.0, 72.0, 54.0)
    );
}

#[test]
fn 自動改ページを再利用せず手動改ページの範囲を保持する() {
    let xml = sheet(
        r#"<rowBreaks><brk id="20" min="0" max="16383" man="1"/><brk id="10" man="0"/></rowBreaks><colBreaks><brk id="5" min="2" max="100" man="true"/></colBreaks>"#,
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let print = PrintSettings::parse(doc.root_element()).unwrap();
    assert_eq!(print.row_breaks.len(), 1);
    assert_eq!(print.row_breaks[0].after, 20);
    assert_eq!(print.column_breaks[0].min, 2);
    assert_eq!(print.column_breaks[0].max, 100);
}

#[test]
fn 印刷設定の欠落を推測で埋めない() {
    let xml = sheet("");
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let print = PrintSettings::parse(doc.root_element()).unwrap();
    assert!(print.paper_size.is_none());
    assert!(print.margins.is_none());
}

#[test]
fn 印刷設定の不正値を拒否する() {
    for body in [
        r#"<pageSetup scale="0"/>"#,
        r#"<pageSetup scale="401"/>"#,
        r#"<pageSetup fitToWidth="-1"/><sheetPr><pageSetUpPr fitToPage="true"/></sheetPr>"#,
        r#"<pageMargins left="NaN" right="0" top="0" bottom="0" header="0" footer="0"/>"#,
        r#"<printOptions horizontalCentered="yes"/>"#,
        r#"<rowBreaks><brk id="5" min="10" max="1" man="1"/></rowBreaks>"#,
        r#"<pageSetup/><pageSetup/>"#,
    ] {
        let xml = sheet(body);
        let doc = roxmltree::Document::parse(&xml).unwrap();
        assert!(PrintSettings::parse(doc.root_element()).is_err(), "{body}");
    }
}

#[test]
fn 列幅と行高と非表示とスタイルを保持する() {
    let xml = sheet(
        r#"<sheetFormatPr defaultRowHeight="15" defaultColWidth="9.25" zeroHeight="1"/><cols><col min="1" max="3" width="12.5" hidden="1" style="2"/></cols><sheetData><row r="1" ht="30" hidden="0" customFormat="1" s="3"/><row r="2"/></sheetData><mergeCells><mergeCell ref="A1:C1"/></mergeCells>"#,
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let grid = Grid::parse(doc.root_element()).unwrap();
    assert_eq!(grid.default_column_width, Some(9.25));
    assert_eq!(grid.columns[0].width, Some(12.5));
    assert!(grid.columns[0].hidden);
    assert_eq!(grid.rows[&1].height, Some(30.0));
    assert!(!grid.rows[&1].hidden);
    assert!(grid.rows[&2].hidden);
    assert_eq!(grid.style_for(1, 1), 3);
    assert_eq!(grid.style_for(1, 2), 2);
    assert_eq!(grid.style_for(4, 2), 0);
    assert_eq!(grid.merges, vec![CellRange::parse("A1:C1").unwrap()]);
}

#[test]
fn 結合の交差だけを拒否する() {
    for (ranges, valid) in [
        (vec!["A1:C3", "D1:E3"], true),
        (vec!["A1:C3", "A4:C4"], true),
        (vec!["A1:C3", "B2:D5"], false),
        (vec!["A1:C3", "A1:C3"], false),
        (vec!["A1:C3", "B1:B2"], false),
    ] {
        let body = format!(
            "<mergeCells>{}</mergeCells>",
            ranges
                .iter()
                .map(|r| format!(r#"<mergeCell ref="{r}"/>"#))
                .collect::<String>()
        );
        let xml = sheet(&body);
        let doc = roxmltree::Document::parse(&xml).unwrap();
        assert_eq!(Grid::parse(doc.root_element()).is_ok(), valid);
    }
}

#[test]
fn 範囲と印刷タイトルを検証する() {
    assert_eq!(
        CellRange::parse("$A$1:$XFD$1048576").unwrap().last_column,
        16384
    );
    for bad in ["A1:B0", "B2:A1", "A1:XFE1", "A1:B2,C3", "$$A1", "A$$1"] {
        assert!(CellRange::parse(bad).is_err());
    }
    assert_eq!(title_range("$1:$3").unwrap().last_row, 3);
    assert_eq!(title_range("$A:$B").unwrap().last_column, 2);
    for bad in ["$3:$1", "A1:B2", ":", "1:B", "A:XFE"] {
        assert!(title_range(bad).is_err());
    }
}

#[test]
fn 引用されたシート名のカンマと引用符を扱う() {
    assert_eq!(
        references(
            "'案件, O''Brien'!$A$1:$C$20,'案件, O''Brien'!$E$1:$F$20",
            "案件, O'Brien"
        )
        .unwrap(),
        vec!["$A$1:$C$20", "$E$1:$F$20"]
    );
    for value in [
        "'機密!A1",
        "別シート!A1",
        "OFFSET(A1,0,0)",
        "[外部.xlsx]機密!A1",
    ] {
        let error = references(value, "機密").unwrap_err();
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
}

#[test]
fn 印刷設定の未知属性と子要素を黙って無視しない() {
    for body in [
        r#"<pageSetup secretSetting="機密"/>"#,
        r#"<pageSetup xmlns:x="urn:unknown" x:scale="80"/>"#,
        r#"<pageMargins left="0" right="0" top="0" bottom="0" header="0" footer="0"><extra/></pageMargins>"#,
        r#"<printOptions><extra/></printOptions>"#,
        r#"<sheetPr><pageSetUpPr unknown="機密"/></sheetPr>"#,
        r#"<rowBreaks><brk id="1" man="1" unknown="機密"/></rowBreaks>"#,
        r#"<pageSetup>機密</pageSetup>"#,
    ] {
        let xml = sheet(body);
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let error = PrintSettings::parse(doc.root_element()).unwrap_err();
        assert_eq!(
            error.category,
            rhodopsin::xlsx::diagnostic::Category::UnsupportedFeature
        );
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
}

#[test]
fn 既知の印刷メタデータと正しい名前空間付き参照を受け入れる() {
    let xml = sheet(
        r#"<sheetPr><pageSetUpPr fitToPage="0" autoPageBreaks="1"/></sheetPr><pageSetup xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="printer" paperSize="9" horizontalDpi="600" verticalDpi="600" copies="1" firstPageNumber="1" useFirstPageNumber="0"/><printOptions gridLinesSet="1"/>"#,
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    assert_eq!(
        PrintSettings::parse(doc.root_element()).unwrap().paper_size,
        Some(9)
    );
    for body in [
        r#"<printOptions gridLinesSet="invalid"/>"#,
        r#"<sheetPr><pageSetUpPr autoPageBreaks="invalid"/></sheetPr>"#,
    ] {
        let xml = sheet(body);
        let doc = roxmltree::Document::parse(&xml).unwrap();
        assert!(PrintSettings::parse(doc.root_element()).is_err());
    }
}
