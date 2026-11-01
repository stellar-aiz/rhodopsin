use rhodopsin::xlsx::{
    grid::{Column, Grid, Row},
    layout::{LayoutLimits, Paper, column_points, paginate},
    print::{Margins, Orientation, PageBreak, PageOrder, PrintSettings, Scaling},
    range::CellRange,
};
use std::collections::BTreeMap;
fn grid() -> Grid {
    Grid {
        default_row_height: Some(25.0),
        default_column_width: Some(10.0),
        base_column_width: 8,
        default_rows_hidden: false,
        columns: Vec::new(),
        rows: BTreeMap::new(),
        merges: Vec::new(),
    }
}
fn print() -> PrintSettings {
    PrintSettings {
        paper_size: Some(1),
        orientation: Orientation::Portrait,
        order: PageOrder::DownThenOver,
        scaling: Scaling::Percent(100),
        margins: Some(Margins {
            left: 10.0,
            right: 502.0,
            top: 20.0,
            bottom: 672.0,
            header: 0.0,
            footer: 0.0,
        }),
        horizontal_center: false,
        vertical_center: false,
        areas: vec![CellRange::parse("A1:G10").unwrap()],
        repeat_rows: None,
        repeat_columns: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
    }
}

#[test]
fn 列幅換算を仕様の数値例と照合する() {
    assert_eq!(column_points(8.7109375, 7.0).unwrap(), 61.0 * 0.75);
    assert_eq!(column_points(10.0, 4.0).unwrap(), 30.0);
    assert!(column_points(10.0, 0.0).is_err());
    assert!(column_points(f64::NAN, 7.0).is_err());
    let a4 = Paper::from_id(9, Orientation::Landscape).unwrap();
    assert!((a4.width - 297.0 * 72.0 / 25.4).abs() < 1e-8);
}

#[test]
fn 印刷順序とページごとのセル範囲を保持する() {
    let mut settings = print();
    let pages = paginate(2, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap();
    assert_eq!(pages.len(), 9);
    assert_eq!(pages[0].body, CellRange::parse("A1:C4").unwrap());
    assert_eq!(pages[1].body, CellRange::parse("A5:C8").unwrap());
    assert_eq!(pages[3].body, CellRange::parse("D1:F4").unwrap());
    assert_eq!(pages[8].body, CellRange::parse("G9:G10").unwrap());
    assert_eq!(pages[0].sheet_index, 2);
    assert_eq!(pages[0].columns[1].offset, 30.0);
    settings.order = PageOrder::OverThenDown;
    let pages = paginate(2, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap();
    assert_eq!(pages[1].body, CellRange::parse("D1:F4").unwrap());
}

#[test]
fn 非表示行列を除外する() {
    let mut grid = grid();
    grid.rows.insert(
        2,
        Row {
            height: Some(300.0),
            hidden: true,
            style: None,
        },
    );
    grid.columns.push(Column {
        first: 2,
        last: 2,
        width: Some(255.0),
        hidden: true,
        style: None,
    });
    let pages = paginate(1, &grid, &print(), 4.0, &LayoutLimits::default()).unwrap();
    assert_eq!(pages.len(), 6);
    assert_eq!(
        pages[0].rows.iter().map(|r| r.index).collect::<Vec<_>>(),
        [1, 3, 4, 5]
    );
    assert_eq!(
        pages[0].columns.iter().map(|c| c.index).collect::<Vec<_>>(),
        [1, 3, 4]
    );
}

#[test]
fn 繰り返し行列を各ページに配置する() {
    let mut settings = print();
    settings.repeat_rows = Some(CellRange::parse("A1:XFD1").unwrap());
    settings.repeat_columns = Some(CellRange::parse("A1:A1048576").unwrap());
    let pages = paginate(1, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap();
    assert_eq!(pages.len(), 9);
    for page in &pages {
        assert_eq!(page.rows[0].index, 1);
        assert!(page.rows[0].repeated);
        assert_eq!(page.columns[0].index, 1);
        assert!(page.columns[0].repeated);
        assert_eq!(page.rows[1].offset, 25.0);
        assert_eq!(page.columns[1].offset, 30.0);
        assert!(page.rows[1..].iter().all(|r| r.index != 1));
    }
    assert_eq!(pages[0].body, CellRange::parse("B2:C4").unwrap());
}

#[test]
fn 倍率指定では手動改ページを守り収容指定では無視する() {
    let mut settings = print();
    settings.row_breaks.push(PageBreak {
        after: 2,
        min: 0,
        max: 16383,
    });
    let pages = paginate(1, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap();
    assert_eq!(pages[0].body.last_row, 2);
    assert_eq!(pages[1].body.first_row, 3);
    settings.scaling = Scaling::Fit {
        width: 1,
        height: 1,
    };
    let pages = paginate(1, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].scale, 0.4);
    assert_eq!(pages[0].body, CellRange::parse("A1:G10").unwrap());
    settings.row_breaks[0].min = 2;
    settings.row_breaks[0].max = 3;
    assert!(paginate(1, &grid(), &settings, 4.0, &LayoutLimits::default()).is_ok());
}

#[test]
fn 中央寄せはページ内の使用寸法から求める() {
    let mut settings = print();
    settings.areas = vec![CellRange::parse("A1:B2").unwrap()];
    settings.horizontal_center = true;
    settings.vertical_center = true;
    let pages = paginate(1, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap();
    assert_eq!((pages[0].origin_x, pages[0].origin_y), (30.0, 45.0));
}

#[test]
fn 配置上限と欠落設定を理由付きで拒否する() {
    let mut settings = print();
    let limits = LayoutLimits {
        pages: 1,
        ..LayoutLimits::default()
    };
    assert_eq!(
        paginate(3, &grid(), &settings, 4.0, &limits)
            .unwrap_err()
            .reason,
        "page_limit"
    );
    let limits = LayoutLimits {
        placements: 2,
        ..LayoutLimits::default()
    };
    assert_eq!(
        paginate(3, &grid(), &settings, 4.0, &limits)
            .unwrap_err()
            .reason,
        "layout_placement_limit"
    );
    settings.margins = None;
    let error = paginate(3, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap_err();
    assert_eq!(error.reason, "page_margins_missing");
    assert_eq!(error.location.unwrap().sheet_index, 3);
    let mut grid = grid();
    grid.default_column_width = None;
    assert_eq!(
        paginate(1, &grid, &print(), 4.0, &LayoutLimits::default())
            .unwrap_err()
            .reason,
        "default_column_width_missing"
    );
}

#[test]
fn セル矩形へ余白と拡大縮小を一度だけ適用する() {
    use rhodopsin::xlsx::placement::range_on_page;
    let mut settings = print();
    settings.scaling = Scaling::Percent(50);
    let pages = paginate(1, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap();
    let found = range_on_page(&pages[0], CellRange::parse("B2:C3").unwrap()).unwrap();
    assert_eq!(found.len(), 1);
    let rect = &found[0].rect;
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (25.0, 32.5, 30.0, 25.0)
    );
    assert!(
        range_on_page(&pages[0], CellRange::parse("Z100").unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn 範囲の可視矩形から非表示行列の幅を除外する() {
    use rhodopsin::xlsx::placement::range_on_page;
    let mut grid = grid();
    grid.rows.insert(
        2,
        Row {
            height: Some(500.0),
            hidden: true,
            style: None,
        },
    );
    grid.columns.push(Column {
        first: 2,
        last: 2,
        width: Some(100.0),
        hidden: true,
        style: None,
    });
    let pages = paginate(1, &grid, &print(), 4.0, &LayoutLimits::default()).unwrap();
    let found = range_on_page(&pages[0], CellRange::parse("A1:C3").unwrap()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].rect.width, found[0].rect.height), (60.0, 50.0));
    assert!(
        range_on_page(&pages[0], CellRange::parse("B2").unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn 繰り返しと本文を分離して改ページ後の範囲を配置する() {
    use rhodopsin::xlsx::placement::range_on_page;
    let mut settings = print();
    settings.repeat_rows = Some(CellRange::parse("A1:XFD1").unwrap());
    settings.repeat_columns = Some(CellRange::parse("A1:A1048576").unwrap());
    let pages = paginate(1, &grid(), &settings, 4.0, &LayoutLimits::default()).unwrap();
    let page = pages
        .iter()
        .find(|p| p.body.first_row > 2 && p.body.first_column > 2)
        .unwrap();
    let found = range_on_page(page, CellRange::parse("A1:G10").unwrap()).unwrap();
    assert_eq!(found.len(), 4);
    assert_eq!(
        (found[0].repeated_rows, found[0].repeated_columns),
        (true, true)
    );
    assert_eq!(
        (found[3].repeated_rows, found[3].repeated_columns),
        (false, false)
    );
    assert_eq!(found[3].visible, page.body);
    assert_eq!((found[3].rect.x, found[3].rect.y), (40.0, 45.0));
}

#[test]
fn 保存された罫線補正行高で改ページし非表示行を除外する() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr defaultRowHeight="15" defaultColWidth="10"/><sheetData><row r="1" ht="16.5" thickTop="1" thickBot="1"/><row r="2" ht="16.5" customHeight="1" thickBot="1"/><row r="3" ht="16.5" hidden="1" thickTop="1"/><row r="4" ht="16.5" thickBot="1"/></sheetData></worksheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let grid = Grid::parse(doc.root_element()).unwrap();
    let mut settings = print();
    settings.areas = vec![CellRange::parse("A1:A4").unwrap()];
    // 本文高さを保存行高二行分へ合わせる。補正を加算すると一行しか入らない。
    settings.margins.as_mut().unwrap().bottom = 739.0;
    let pages = paginate(1, &grid, &settings, 7.0, &LayoutLimits::default()).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(
        pages[0]
            .rows
            .iter()
            .map(|r| (r.index, r.size, r.offset))
            .collect::<Vec<_>>(),
        [(1, 16.5, 0.0), (2, 16.5, 16.5)]
    );
    assert_eq!(
        pages[1].rows.iter().map(|r| r.index).collect::<Vec<_>>(),
        [4]
    );
}
