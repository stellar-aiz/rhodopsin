#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::{
    ConversionOptions, PreparedWorkbook,
    cell_text::Frame,
    drawing::{Anchor, Marker, Picture, Raster},
    fonts::{FontBook, FontLimits},
    format::date::Locale,
    grid::Grid,
    image_geometry::GridGeometry,
    layout::{AxisItem, Page, Paper},
    page_scene::{self, Limits},
    pdf,
    print::PrintSettings,
    range::CellRange,
    styles::Styles,
    theme::TextScript,
    warnings::Warnings,
    workbook::{Cell, CellValue, Sheet, Workbook},
};
use std::sync::Arc;
fn fixture() -> (PreparedWorkbook, Page, GridGeometry) {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr defaultRowHeight="20" defaultColWidth="10"/></worksheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let grid = Grid::parse(doc.root_element()).unwrap();
    let geometry = GridGeometry::new(&grid, 1, 1, 4.0, 10).unwrap();
    let sheet = Sheet {
        index: 1,
        hidden: false,
        grid,
        print: PrintSettings::parse(doc.root_element()).unwrap(),
        cells: vec![Cell {
            address: "A1".into(),
            style: 0,
            value: CellValue::Number(1234.0),
            numeric_lexeme: Some("1234".into()),
        }],
        pictures: vec![Picture {
            crop: Default::default(),
            anchor: Anchor::OneCell {
                from: Marker {
                    column: 1,
                    row: 1,
                    column_offset: 0,
                    row_offset: 0,
                },
                width_emu: 127000,
                height_emu: 127000,
            },
            raster: Arc::new(Raster {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 255, 255],
            }),
            print: true,
            hidden: false,
        }],
    };
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font><name val="試験"/><sz val="10"/><color rgb="FF000000"/></font></fonts><fills><fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/></patternFill></fill></fills><borders><border/></borders><cellXfs><xf numFmtId="3"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let styles = Styles::parse(doc.root_element()).unwrap();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(
            support::font("試験", false, false, 0),
            &FontLimits::default(),
        )
        .unwrap();
    let prepared = PreparedWorkbook {
        workbook: Workbook {
            sheets: vec![sheet],
            styles: Some(styles),
            theme: None,
            date_1904: false,
        },
        fonts,
    };
    let page = Page {
        sheet_index: 1,
        area_index: 1,
        paper: Paper {
            width: 200.0,
            height: 100.0,
        },
        scale: 1.0,
        origin_x: 10.0,
        origin_y: 20.0,
        columns: vec![AxisItem {
            index: 1,
            offset: 0.0,
            size: 30.0,
            repeated: false,
        }]
        .into(),
        rows: vec![AxisItem {
            index: 1,
            offset: 0.0,
            size: 20.0,
            repeated: false,
        }]
        .into(),
        body: CellRange::parse("A1").unwrap(),
    };
    (prepared, page, geometry)
}

#[test]
fn 背景文字画像を順番に描画して文字抽出と決定性を保つ() {
    let (prepared, page, geometry) = fixture();
    let options = ConversionOptions::default();
    let context = prepared
        .text_context(&options, Locale::Japanese, TextScript::Latin, 100)
        .unwrap();
    let mut warnings = Warnings::default();
    let scene = page_scene::plan(
        &prepared.workbook.sheets[0],
        &page,
        &context,
        Some(&geometry),
        &mut warnings,
        &Limits::default(),
        |_, placement| {
            Ok(Frame {
                content: placement.rect,
                clip: placement.rect,
                scale: page.scale,
                baseline_step: None,
                line_box: None,
            })
        },
    )
    .unwrap();
    let render = || {
        pdf::render(
            &[scene.pdf_page()],
            &prepared.fonts,
            &Default::default(),
            &options.font_limits,
        )
        .unwrap()
    };
    let bytes = render();
    assert_eq!(bytes, render());
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "1,234");
    let content =
        lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
            .unwrap();
    let operators: Vec<_> = content
        .operations
        .iter()
        .map(|op| op.operator.as_str())
        .collect();
    assert!(
        operators.iter().position(|op| *op == "f").unwrap()
            < operators.iter().position(|op| *op == "BT").unwrap()
    );
    assert!(
        operators.iter().position(|op| *op == "ET").unwrap()
            < operators.iter().position(|op| *op == "Do").unwrap()
    );
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn 画像と総量の失敗で文字段の警告を確定しない() {
    let (prepared, page, geometry) = fixture();
    let options = ConversionOptions::default();
    let context = prepared
        .text_context(&options, Locale::Japanese, TextScript::Latin, 100)
        .unwrap();
    for (geometry, limits, expected) in [
        (None, Limits::default(), "drawing_geometry_unresolved"),
        (
            Some(&geometry),
            Limits {
                items: 2,
                ..Default::default()
            },
            "page_scene_item_limit",
        ),
        (
            Some(&geometry),
            Limits {
                text_bytes: 1,
                ..Default::default()
            },
            "page_scene_text_limit",
        ),
    ] {
        let mut warnings = Warnings::default();
        let error = page_scene::plan(
            &prepared.workbook.sheets[0],
            &page,
            &context,
            geometry,
            &mut warnings,
            &limits,
            |_, placement| {
                let mut rect = placement.rect;
                rect.width = 18.0;
                Ok(Frame {
                    content: rect,
                    clip: rect,
                    scale: page.scale,
                    baseline_step: None,
                    line_box: None,
                })
            },
        )
        .err()
        .unwrap();
        assert_eq!(error.reason, expected);
        assert!(warnings.into_summaries().is_empty());
    }
}

#[test]
fn 未実装の罫線を黙って落とさない() {
    let (mut prepared, page, geometry) = fixture();
    prepared.workbook.styles.as_mut().unwrap().borders[0]
        .as_mut()
        .unwrap()
        .left = Some(rhodopsin::xlsx::styles::BorderLine {
        style: rhodopsin::xlsx::styles::LineStyle::Thin,
        color: Default::default(),
    });
    let options = ConversionOptions::default();
    let context = prepared
        .text_context(&options, Locale::Japanese, TextScript::Latin, 100)
        .unwrap();
    let error = page_scene::plan(
        &prepared.workbook.sheets[0],
        &page,
        &context,
        Some(&geometry),
        &mut Warnings::default(),
        &Limits::default(),
        |_, _| panic!("罫線検査で停止する"),
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, "page_border_layout_unimplemented");
}

#[test]
fn 下線を文字と画像の間へ追加し描画数の上限にも含める() {
    let (mut prepared, page, geometry) = fixture();
    prepared.workbook.styles.as_mut().unwrap().fonts[0]
        .as_mut()
        .unwrap()
        .underline = rhodopsin::xlsx::styles::Underline::Single;
    prepared.fonts = FontBook::default();
    let options = ConversionOptions::default();
    prepared
        .fonts
        .load_bytes(
            support::font_with_underline("試験", false, false, 0, Some((-100, 50))),
            &options.font_limits,
        )
        .unwrap();
    let context = prepared
        .text_context(&options, Locale::Japanese, TextScript::Latin, 100)
        .unwrap();
    let make = |limit| {
        page_scene::plan(
            &prepared.workbook.sheets[0],
            &page,
            &context,
            Some(&geometry),
            &mut Warnings::default(),
            &Limits {
                items: limit,
                ..Default::default()
            },
            |_, placement| {
                Ok(Frame {
                    content: placement.rect,
                    clip: placement.rect,
                    scale: page.scale,
                    baseline_step: None,
                    line_box: None,
                })
            },
        )
    };
    let scene = make(4).unwrap();
    let page = scene.pdf_page();
    assert_eq!(page.items.len(), 4);
    assert!(matches!(page.items[1], pdf::Item::Text(_)));
    assert!(matches!(
        page.items[2],
        pdf::Item::Paint(pdf::graphics::Paint::Fill { .. })
    ));
    assert!(matches!(
        page.items[3],
        pdf::Item::Paint(pdf::graphics::Paint::Image { .. })
    ));
    assert_eq!(make(3).err().unwrap().reason, "page_scene_item_limit");
}

fn add_border_style(prepared: &mut PreparedWorkbook) {
    let styles = prepared.workbook.styles.as_mut().unwrap();
    let mut border = styles.borders[0].as_ref().unwrap().clone();
    border.left = Some(rhodopsin::xlsx::styles::BorderLine {
        style: rhodopsin::xlsx::styles::LineStyle::Thin,
        color: Default::default(),
    });
    styles.borders.push(Ok(border));
    let mut style = styles.cells[0].as_ref().unwrap().clone();
    style.border = 1;
    styles.cells.push(Ok(style));
}
fn check_scene(
    prepared: &PreparedWorkbook,
    page: &Page,
    geometry: &GridGeometry,
) -> rhodopsin::xlsx::diagnostic::Result<()> {
    let options = ConversionOptions::default();
    let context = prepared.text_context(&options, Locale::Japanese, TextScript::Latin, 100)?;
    page_scene::plan(
        &prepared.workbook.sheets[0],
        page,
        &context,
        Some(geometry),
        &mut Warnings::default(),
        &Limits::default(),
        |_, placement| {
            Ok(Frame {
                content: placement.rect,
                clip: placement.rect,
                scale: page.scale,
                baseline_step: None,
                line_box: None,
            })
        },
    )
    .map(|_| ())
}
#[test]
fn 未使用の罫線定義と印刷範囲外のセルだけでは停止しない() {
    let (mut prepared, page, geometry) = fixture();
    add_border_style(&mut prepared);
    assert!(check_scene(&prepared, &page, &geometry).is_ok());
    prepared.workbook.sheets[0].cells.push(Cell {
        address: "Z100".into(),
        style: 1,
        value: CellValue::Blank,
        numeric_lexeme: None,
    });
    assert!(check_scene(&prepared, &page, &geometry).is_ok());
    prepared.workbook.sheets[0].cells[0].style = 1;
    let error = check_scene(&prepared, &page, &geometry).unwrap_err();
    assert_eq!(error.reason, "page_border_layout_unimplemented");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("A1"));
}
#[test]
fn 空白セルの行列罫線と明示セルの上書きを判定する() {
    let (mut prepared, page, geometry) = fixture();
    add_border_style(&mut prepared);
    prepared.workbook.sheets[0].grid.rows.insert(
        1,
        rhodopsin::xlsx::grid::Row {
            height: None,
            hidden: false,
            style: Some(1),
        },
    );
    assert!(check_scene(&prepared, &page, &geometry).is_ok());
    prepared.workbook.sheets[0].cells.clear();
    assert_eq!(
        check_scene(&prepared, &page, &geometry).unwrap_err().reason,
        "page_border_layout_unimplemented"
    );
    prepared.workbook.sheets[0].grid.rows.clear();
    prepared.workbook.sheets[0]
        .grid
        .columns
        .push(rhodopsin::xlsx::grid::Column {
            first: 1,
            last: 1,
            width: None,
            hidden: false,
            style: Some(1),
        });
    assert_eq!(
        check_scene(&prepared, &page, &geometry)
            .unwrap_err()
            .location
            .unwrap()
            .cell
            .as_deref(),
        Some("A1")
    );
}
#[test]
fn 結合左上がページ外でも罫線を黙って捨てない() {
    let (mut prepared, mut page, geometry) = fixture();
    add_border_style(&mut prepared);
    prepared.workbook.sheets[0].pictures.clear();
    prepared.workbook.sheets[0].cells[0].style = 1;
    page.columns = vec![AxisItem {
        index: 2,
        offset: 0.0,
        size: 30.0,
        repeated: false,
    }]
    .into();
    page.body = CellRange::parse("B1").unwrap();
    assert!(check_scene(&prepared, &page, &geometry).is_ok());
    prepared.workbook.sheets[0]
        .grid
        .merges
        .push(CellRange::parse("A1:B1").unwrap());
    let error = check_scene(&prepared, &page, &geometry).unwrap_err();
    assert_eq!(error.reason, "page_border_layout_unimplemented");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("A1"));
}

fn border_edges(
    prepared: &PreparedWorkbook,
    page: &Page,
    limit: usize,
) -> rhodopsin::xlsx::diagnostic::Result<Vec<rhodopsin::xlsx::page_borders::Edge>> {
    rhodopsin::xlsx::page_borders::edges(
        &prepared.workbook.sheets[0],
        page,
        prepared.workbook.styles.as_ref().unwrap(),
        &rhodopsin::xlsx::color::ColorContext {
            theme: None,
            indexed: &[],
            automatic: Some([0, 0, 0]),
            foreground: None,
            background: None,
        },
        limit,
    )
}

#[test]
fn 罫線候補は空白の行書式と明示セルを区別して共有辺を保持する() {
    use rhodopsin::xlsx::{
        page_borders::Side,
        styles::{BorderLine, Color, ColorSource, LineStyle},
    };
    let (mut prepared, mut page, _) = fixture();
    add_border_style(&mut prepared);
    let styles = prepared.workbook.styles.as_mut().unwrap();
    styles.borders[1].as_mut().unwrap().right = Some(BorderLine {
        style: LineStyle::Double,
        color: Color {
            source: ColorSource::Rgb([255, 0, 0]),
            tint: 0.0,
        },
    });
    prepared.workbook.sheets[0].grid.rows.insert(
        1,
        rhodopsin::xlsx::grid::Row {
            height: None,
            hidden: false,
            style: Some(1),
        },
    );
    page.columns = vec![
        AxisItem {
            index: 1,
            offset: 0.0,
            size: 30.0,
            repeated: true,
        },
        AxisItem {
            index: 3,
            offset: 30.0,
            size: 40.0,
            repeated: false,
        },
    ]
    .into();
    page.scale = 0.5;
    // A1 の明示スタイルは罫線なし。非表示 B 列の候補を作らず、空白 C1 に行書式を適用する。
    let edges = border_edges(&prepared, &page, 100).unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0].cell, "C1");
    assert_eq!(edges[0].side, Side::Left);
    assert_eq!(edges[0].from, [25.0, 20.0]);
    assert_eq!(edges[0].to, [25.0, 30.0]);
    assert_eq!(edges[1].from, [45.0, 20.0]);
    assert_eq!(edges[1].style, LineStyle::Double);
    assert_eq!(edges[1].color, [255, 0, 0]);
    prepared.workbook.sheets[0].cells[0].style = 1;
    let edges = border_edges(&prepared, &page, 100).unwrap();
    assert_eq!(edges.len(), 4);
    assert_eq!(edges[1].from, edges[2].from);
    assert_eq!(edges[1].to, edges[2].to);
    assert_ne!(edges[1].style, edges[2].style);
    assert_eq!(edges[1].cell, "A1");
    assert_eq!(edges[2].cell, "C1");
}

#[test]
fn 罫線候補は不正軸と上限超過を停止し結合外周を保持する() {
    let (mut prepared, mut page, _) = fixture();
    add_border_style(&mut prepared);
    prepared.workbook.sheets[0].cells[0].style = 1;
    assert_eq!(
        border_edges(&prepared, &page, 0).unwrap_err().reason,
        "page_border_work_limit"
    );
    assert_eq!(
        border_edges(&prepared, &page, 4).unwrap_err().reason,
        "page_border_work_limit"
    );
    assert_eq!(border_edges(&prepared, &page, 5).unwrap().len(), 1);
    prepared.workbook.sheets[0]
        .grid
        .merges
        .push(CellRange::parse("A1:B1").unwrap());
    let edges = border_edges(&prepared, &page, 100).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].cell, "A1");
    prepared.workbook.sheets[0].grid.merges.clear();
    page.scale = f64::NAN;
    assert_eq!(
        border_edges(&prepared, &page, 100).unwrap_err().reason,
        "invalid_page_transform"
    );
}

fn merged_fixture() -> (PreparedWorkbook, Page) {
    let (mut prepared, mut page, _) = fixture();
    add_border_style(&mut prepared);
    let border = prepared.workbook.styles.as_mut().unwrap().borders[1]
        .as_mut()
        .unwrap();
    border.right = border.left.clone();
    border.top = border.left.clone();
    border.bottom = border.left.clone();
    prepared.workbook.sheets[0].cells[0].style = 1;
    prepared.workbook.sheets[0].grid.merges = vec![CellRange::parse("A1:B2").unwrap()];
    page.columns = vec![
        AxisItem {
            index: 1,
            offset: 0.0,
            size: 30.0,
            repeated: false,
        },
        AxisItem {
            index: 2,
            offset: 30.0,
            size: 40.0,
            repeated: false,
        },
    ]
    .into();
    page.rows = vec![
        AxisItem {
            index: 1,
            offset: 0.0,
            size: 20.0,
            repeated: false,
        },
        AxisItem {
            index: 2,
            offset: 20.0,
            size: 25.0,
            repeated: false,
        },
    ]
    .into();
    page.scale = 0.5;
    (prepared, page)
}

#[test]
fn 結合罫線は左上書式を外周へ広げ内部の格子を描かない() {
    use rhodopsin::xlsx::page_borders::Side;
    let (prepared, page) = merged_fixture();
    let edges = border_edges(&prepared, &page, 1000).unwrap();
    assert_eq!(edges.len(), 4);
    assert_eq!(
        edges
            .iter()
            .map(|e| (e.side, e.from, e.to))
            .collect::<Vec<_>>(),
        vec![
            (Side::Left, [10.0, 20.0], [10.0, 42.5]),
            (Side::Right, [45.0, 20.0], [45.0, 42.5]),
            (Side::Top, [10.0, 20.0], [45.0, 20.0]),
            (Side::Bottom, [10.0, 42.5], [45.0, 42.5]),
        ]
    );
    assert!(edges.iter().all(|e| e.cell == "A1"));
}

#[test]
fn 改ページで切れた結合には切断辺を追加しない() {
    use rhodopsin::xlsx::page_borders::Side;
    let (prepared, mut page) = merged_fixture();
    page.columns = vec![page.columns[0].clone()].into();
    page.rows = vec![page.rows[0].clone()].into();
    let edges = border_edges(&prepared, &page, 1000).unwrap();
    assert_eq!(
        edges.iter().map(|e| e.side).collect::<Vec<_>>(),
        vec![Side::Left, Side::Top]
    );
    assert_eq!(edges[1].to, [25.0, 20.0]);
    page.columns = vec![AxisItem {
        index: 2,
        offset: 0.0,
        size: 40.0,
        repeated: false,
    }]
    .into();
    page.rows = vec![AxisItem {
        index: 2,
        offset: 0.0,
        size: 25.0,
        repeated: false,
    }]
    .into();
    let edges = border_edges(&prepared, &page, 1000).unwrap();
    assert_eq!(
        edges.iter().map(|e| e.side).collect::<Vec<_>>(),
        vec![Side::Right, Side::Bottom]
    );
    assert_eq!(edges[0].from, [30.0, 20.0]);
    assert_eq!(edges[1].from, [10.0, 32.5]);
}

#[test]
fn 非表示の先頭行列と繰り返し見出しを外周座標へ反映する() {
    use rhodopsin::xlsx::{
        grid::{Column, Row},
        page_borders::Side,
    };
    let (mut prepared, mut page) = merged_fixture();
    prepared.workbook.sheets[0].grid.columns.push(Column {
        first: 1,
        last: 1,
        width: None,
        hidden: true,
        style: None,
    });
    prepared.workbook.sheets[0].grid.rows.insert(
        1,
        Row {
            height: None,
            hidden: true,
            style: None,
        },
    );
    page.columns = vec![AxisItem {
        index: 2,
        offset: 0.0,
        size: 40.0,
        repeated: false,
    }]
    .into();
    page.rows = vec![AxisItem {
        index: 2,
        offset: 0.0,
        size: 25.0,
        repeated: false,
    }]
    .into();
    let edges = border_edges(&prepared, &page, 1000).unwrap();
    assert_eq!(edges.len(), 4);
    assert_eq!(edges[0].from, [10.0, 20.0]);
    assert_eq!(edges[1].to, [30.0, 32.5]);
    prepared.workbook.sheets[0].grid.default_rows_hidden = true;
    prepared.workbook.sheets[0].grid.rows.insert(
        2,
        Row {
            height: None,
            hidden: false,
            style: None,
        },
    );
    assert_eq!(border_edges(&prepared, &page, 1000).unwrap().len(), 4);
    let (prepared, mut page) = merged_fixture();
    let mut columns = page.columns.to_vec();
    columns[0].repeated = true;
    page.columns = columns.into();
    let edges = border_edges(&prepared, &page, 1000).unwrap();
    assert_eq!(edges.len(), 6);
    assert_eq!(edges.iter().filter(|e| e.side == Side::Left).count(), 1);
    assert_eq!(edges.iter().filter(|e| e.side == Side::Right).count(), 1);
    assert_eq!(edges.iter().filter(|e| e.side == Side::Top).count(), 2);
}

#[test]
fn 結合外周の矛盾と重複と過大な検査を拒否する() {
    let (mut prepared, page) = merged_fixture();
    let styles = prepared.workbook.styles.as_mut().unwrap();
    let mut border = styles.borders[1].as_ref().unwrap().clone();
    border.right.as_mut().unwrap().style = rhodopsin::xlsx::styles::LineStyle::Double;
    styles.borders.push(Ok(border));
    let mut style = styles.cells[1].as_ref().unwrap().clone();
    style.border = 2;
    styles.cells.push(Ok(style));
    prepared.workbook.sheets[0].cells.push(Cell {
        address: "B2".into(),
        style: 2,
        value: CellValue::Blank,
        numeric_lexeme: None,
    });
    let error = border_edges(&prepared, &page, 1000).unwrap_err();
    assert_eq!(error.reason, "merged_border_style_conflict");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("A1"));
    prepared.workbook.sheets[0].cells.pop();
    prepared.workbook.sheets[0]
        .grid
        .merges
        .push(CellRange::parse("B2:C3").unwrap());
    assert_eq!(
        border_edges(&prepared, &page, 1000).unwrap_err().reason,
        "overlapping_merged_cells"
    );
    prepared.workbook.sheets[0].grid.merges = vec![CellRange::parse("A1:XFD1048576").unwrap()];
    assert_eq!(
        border_edges(&prepared, &page, 100).unwrap_err().reason,
        "page_border_work_limit"
    );
}

#[test]
fn 結合罫線の右下座標の桁あふれを候補生成時に拒否する() {
    let (prepared, mut page) = merged_fixture();
    page.origin_x = f64::MAX;
    page.scale = 1.0;
    page.columns = vec![
        AxisItem {
            index: 1,
            offset: 0.0,
            size: f64::MAX / 4.0,
            repeated: false,
        },
        AxisItem {
            index: 2,
            offset: f64::MAX / 4.0,
            size: f64::MAX / 4.0,
            repeated: false,
        },
    ]
    .into();
    assert_eq!(
        border_edges(&prepared, &page, 1000).unwrap_err().reason,
        "invalid_page_transform"
    );
}
