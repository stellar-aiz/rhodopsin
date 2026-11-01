use rhodopsin::xlsx::grid::{Grid, StyleSpan};

fn grid(body: &str) -> Grid {
    let xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</worksheet>"#
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    Grid::parse(doc.root_element()).unwrap()
}

#[test]
fn 空白セルを展開せず列のスタイル区間を取得する() {
    let grid = grid(
        r#"<cols><col min="2" max="3" style="1"/><col min="4" max="5" style="1"/><col min="7" max="9" style="2" hidden="1"/><col min="10" max="11"/></cols>"#,
    );
    assert_eq!(
        grid.style_spans(1, 1, 16384).unwrap(),
        vec![
            StyleSpan {
                first: 1,
                last: 1,
                style: 0
            },
            StyleSpan {
                first: 2,
                last: 5,
                style: 1
            },
            StyleSpan {
                first: 6,
                last: 6,
                style: 0
            },
            StyleSpan {
                first: 7,
                last: 9,
                style: 2
            },
            StyleSpan {
                first: 10,
                last: 16384,
                style: 0
            },
        ]
    );
    // 非表示列を消すのはページ配置層。ここでは番地を変えずに保持する。
    assert_eq!(
        grid.style_spans(1048576, 3, 8).unwrap(),
        vec![
            StyleSpan {
                first: 3,
                last: 5,
                style: 1
            },
            StyleSpan {
                first: 6,
                last: 6,
                style: 0
            },
            StyleSpan {
                first: 7,
                last: 8,
                style: 2
            },
        ]
    );
    for first in 1..=12 {
        for last in first..=12 {
            let spans = grid.style_spans(1, first, last).unwrap();
            assert_eq!(spans.first().unwrap().first, first);
            assert_eq!(spans.last().unwrap().last, last);
            for span in spans {
                for column in span.first..=span.last {
                    assert_eq!(span.style, grid.style_for(column, 1));
                }
            }
        }
    }
}

#[test]
fn 行の明示ゼロスタイルと無効な行書式指定を区別する() {
    let grid = grid(
        r#"<cols><col min="1" max="16384" style="2"/></cols><sheetData><row r="1" s="0" customFormat="1"/><row r="2" s="1" customFormat="0"/><row r="3" s="3" customFormat="1"/></sheetData>"#,
    );
    for (row, style) in [(1, 0), (2, 2), (3, 3), (4, 2)] {
        assert_eq!(
            grid.style_spans(row, 1, 16384).unwrap(),
            vec![StyleSpan {
                first: 1,
                last: 16384,
                style
            }]
        );
    }
}

#[test]
fn 不正な範囲と外部構築された重複列を拒否する() {
    let mut grid = grid(r#"<cols><col min="1" max="2"/><col min="3" max="4"/></cols>"#);
    for (row, first, last) in [
        (0, 1, 1),
        (1048577, 1, 1),
        (1, 0, 1),
        (1, 3, 2),
        (1, 1, 16385),
    ] {
        assert!(grid.style_spans(row, first, last).is_err());
    }
    grid.columns[1].first = 2;
    assert!(grid.style_spans(1, 1, 4).is_err());
}

#[test]
fn 罫線補正済みの保存行高を二重に増やさない() {
    for custom in ["0", "1"] {
        for flags in [
            r#"thickTop="1""#,
            r#"thickBot="1""#,
            r#"thickTop="1" thickBot="1""#,
        ] {
            let grid = grid(&format!(
                r#"<sheetFormatPr defaultRowHeight="15" defaultColWidth="10"/><sheetData><row r="1" ht="16.5" customHeight="{custom}" {flags}/></sheetData>"#
            ));
            assert_eq!(grid.rows[&1].height, Some(16.5));
            let geometry =
                rhodopsin::xlsx::image_geometry::GridGeometry::new(&grid, 1, 3, 7.0, 10).unwrap();
            let rect = geometry
                .anchor(&rhodopsin::xlsx::drawing::Anchor::OneCell {
                    from: rhodopsin::xlsx::drawing::Marker {
                        column: 1,
                        row: 2,
                        column_offset: 0,
                        row_offset: 0,
                    },
                    width_emu: 12700,
                    height_emu: 12700,
                })
                .unwrap();
            assert_eq!(rect.y, 16.5);
        }
    }
}

#[test]
fn 保存行高のない罫線補正や不正値を推測しない() {
    for attrs in [
        r#"thickTop="1""#,
        r#"thickBot="1""#,
        r#"thickTop="1" ht="NaN""#,
        r#"thickBot="1" ht="410""#,
        r#"thickTop="機密" ht="16.5""#,
        r#"customHeight="機密" ht="16.5""#,
    ] {
        let xml = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr defaultRowHeight="15"/><sheetData><row r="1" {attrs}/></sheetData></worksheet>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let error = Grid::parse(doc.root_element()).unwrap_err();
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
}
