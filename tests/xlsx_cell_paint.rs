use rhodopsin::xlsx::{
    cell_paint::page_fills,
    color::ColorContext,
    grid::Grid,
    layout::{AxisItem, Page, Paper},
    pdf::graphics::Paint,
    print::PrintSettings,
    range::CellRange,
    styles::Styles,
    workbook::{Cell, CellValue, Sheet},
};
fn fixture() -> (Sheet, Page, Styles) {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cols><col min="1" max="4" style="1"/></cols><sheetData/></worksheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let sheet = Sheet {
        index: 1,
        hidden: false,
        cells: vec![Cell {
            address: "B1".into(),
            style: 0,
            value: CellValue::Blank,
            numeric_lexeme: None,
        }],
        grid: Grid::parse(doc.root_element()).unwrap(),
        print: PrintSettings::parse(doc.root_element()).unwrap(),
        pictures: vec![],
    };
    // C 列は非表示。A 列は繰り返し見出しで、B・D 列は本文。
    let page = Page {
        sheet_index: 1,
        area_index: 0,
        paper: Paper {
            width: 200.0,
            height: 200.0,
        },
        scale: 0.5,
        origin_x: 10.0,
        origin_y: 20.0,
        columns: vec![
            AxisItem {
                index: 1,
                offset: 0.0,
                size: 20.0,
                repeated: true,
            },
            AxisItem {
                index: 2,
                offset: 20.0,
                size: 20.0,
                repeated: false,
            },
            AxisItem {
                index: 4,
                offset: 40.0,
                size: 20.0,
                repeated: false,
            },
        ]
        .into(),
        rows: vec![AxisItem {
            index: 1,
            offset: 0.0,
            size: 30.0,
            repeated: false,
        }]
        .into(),
        body: CellRange::parse("B1:D1").unwrap(),
    };
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/></fonts><fills><fill/><fill><patternFill patternType="solid"><fgColor rgb="FF123456"/></patternFill></fill></fills><borders><border/></borders><cellXfs><xf/><xf fillId="1"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    (sheet, page, Styles::parse(doc.root_element()).unwrap())
}
fn colors() -> ColorContext<'static> {
    ColorContext {
        theme: None,
        indexed: &[],
        automatic: None,
        foreground: None,
        background: None,
    }
}
#[test]
fn 空白背景と明示セルの無塗りをページへ接続する() {
    let (sheet, page, styles) = fixture();
    let fills = page_fills(&sheet, &page, &styles, &colors(), 100).unwrap();
    let boxes: Vec<_> = fills
        .iter()
        .map(|fill| match fill {
            Paint::Fill { rect, color } => {
                assert_eq!(*color, [0x12, 0x34, 0x56]);
                (rect.x, rect.y, rect.width, rect.height)
            }
            _ => panic!("塗り以外の描画命令"),
        })
        .collect();
    assert_eq!(boxes, [(10.0, 20.0, 10.0, 15.0), (30.0, 20.0, 10.0, 15.0)]);
    let items = fills
        .into_iter()
        .map(rhodopsin::xlsx::pdf::Item::Paint)
        .collect();
    let bytes = rhodopsin::xlsx::pdf::render(
        &[rhodopsin::xlsx::pdf::Page {
            width: 200.0,
            height: 200.0,
            items,
        }],
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let pdf = lopdf::Document::load_mem(&bytes).unwrap();
    let content =
        lopdf::content::Content::decode(&pdf.get_page_content(pdf.get_pages()[&1]).unwrap())
            .unwrap();
    let rectangles: Vec<_> = content
        .operations
        .iter()
        .filter(|op| op.operator == "re")
        .collect();
    assert_eq!(rectangles.len(), 2);
    assert_eq!(rectangles[0].operands[1].as_float().unwrap(), 165.0);
}
#[test]
fn 資源上限と非表示シートを検査する() {
    let (mut sheet, page, styles) = fixture();
    assert!(page_fills(&sheet, &page, &styles, &colors(), 1).is_err());
    sheet.grid.merges.push(CellRange::parse("A1:B1").unwrap());
    assert!(page_fills(&sheet, &page, &styles, &colors(), 100).is_ok());
    sheet.hidden = true;
    assert!(
        page_fills(&sheet, &page, &styles, &colors(), 100)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn 結合左上の無塗りで内部セルの背景を取り除く() {
    let (mut sheet, page, styles) = fixture();
    sheet.grid.merges.push(CellRange::parse("B1:D1").unwrap());
    // B1 は明示的な無塗り。D1 の列既定の塗りも結合内では消える。
    let fills = page_fills(&sheet, &page, &styles, &colors(), 200).unwrap();
    assert_eq!(fills.len(), 1);
    match &fills[0] {
        Paint::Fill { rect, .. } => {
            assert_eq!(rect.x, 10.0);
            assert_eq!(rect.width, 10.0);
        }
        _ => panic!("塗り以外の描画命令"),
    }
}

#[test]
fn 結合左上が未保存でも行列の書式から背景を伝播する() {
    let (mut sheet, page, styles) = fixture();
    sheet.grid.merges.push(CellRange::parse("A1:D1").unwrap());
    let fills = page_fills(&sheet, &page, &styles, &colors(), 200).unwrap();
    let mut intervals = Vec::new();
    for paint in fills {
        match paint {
            Paint::Fill { rect, color } => {
                assert_eq!(color, [0x12, 0x34, 0x56]);
                assert_eq!(rect.height, 15.0);
                intervals.push((rect.x, rect.x + rect.width));
            }
            _ => panic!("塗り以外の描画命令"),
        }
    }
    // 非表示 C 列、繰り返し A 列を含む範囲。B1 の無塗りは結合左上で上書き。
    assert_eq!(intervals, [(10.0, 20.0), (20.0, 30.0), (30.0, 40.0)]);
}

#[test]
fn ページ外の結合左上の書式を使い診断も左上へ戻す() {
    let (mut sheet, mut page, styles) = fixture();
    sheet.grid.merges.push(CellRange::parse("A1:D1").unwrap());
    let mut columns = page.columns[1..].to_vec();
    for column in &mut columns {
        column.offset -= 20.0;
    }
    page.columns = columns.into();
    sheet.cells.push(Cell {
        address: "A1".into(),
        style: 0,
        value: CellValue::Blank,
        numeric_lexeme: None,
    });
    assert!(
        page_fills(&sheet, &page, &styles, &colors(), 200)
            .unwrap()
            .is_empty()
    );
    sheet.cells.last_mut().unwrap().style = 999;
    let error = page_fills(&sheet, &page, &styles, &colors(), 200)
        .err()
        .unwrap();
    assert_eq!(error.reason, "cell_style_index");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("A1"));
}

#[test]
fn 重複結合と結合検査の資源上限を拒否する() {
    let (mut sheet, page, styles) = fixture();
    sheet.grid.merges = vec![
        CellRange::parse("A1:B1").unwrap(),
        CellRange::parse("B1:D1").unwrap(),
    ];
    assert_eq!(
        page_fills(&sheet, &page, &styles, &colors(), 200)
            .err()
            .unwrap()
            .reason,
        "overlapping_merged_cells"
    );
    sheet.grid.merges.pop();
    assert_eq!(
        page_fills(&sheet, &page, &styles, &colors(), 8)
            .err()
            .unwrap()
            .reason,
        "cell_paint_limit"
    );
}
