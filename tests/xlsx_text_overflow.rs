use rhodopsin::xlsx::{
    grid::Grid,
    layout::{AxisItem, Page, Paper},
    placement::range_on_page,
    print::PrintSettings,
    range::CellRange,
    styles::Horizontal,
    text_overflow::Occupancy,
    workbook::{Cell, CellValue, Sheet},
};
fn fixture() -> (Sheet, Page) {
    let doc = roxmltree::Document::parse(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr defaultRowHeight="20" defaultColWidth="10"/></worksheet>"#).unwrap();
    let sheet = Sheet {
        index: 1,
        hidden: false,
        grid: Grid::parse(doc.root_element()).unwrap(),
        print: PrintSettings::parse(doc.root_element()).unwrap(),
        cells: vec![Cell {
            address: "C1".into(),
            style: 0,
            value: CellValue::Text("非機密の試験文字".into()),
            numeric_lexeme: None,
        }],
        pictures: vec![],
    };
    let page = Page {
        sheet_index: 1,
        area_index: 1,
        paper: Paper {
            width: 200.0,
            height: 100.0,
        },
        scale: 2.0,
        origin_x: 10.0,
        origin_y: 20.0,
        columns: (1..=5)
            .map(|index| AxisItem {
                index,
                offset: f64::from(index - 1) * 10.0,
                size: 10.0,
                repeated: false,
            })
            .collect::<Vec<_>>()
            .into(),
        rows: vec![AxisItem {
            index: 1,
            offset: 0.0,
            size: 20.0,
            repeated: false,
        }]
        .into(),
        body: CellRange::parse("A1:E1").unwrap(),
    };
    (sheet, page)
}
fn bounds(sheet: &Sheet, page: &Page, alignment: Horizontal, wrap: bool) -> (f64, f64) {
    let occupancy = Occupancy::new(sheet, 100).unwrap();
    let placement = range_on_page(page, CellRange::parse("C1").unwrap())
        .unwrap()
        .remove(0);
    let clip = occupancy
        .clip(page, &sheet.cells[0], &placement, alignment, wrap, 100)
        .unwrap();
    assert_eq!((clip.y, clip.height), (20.0, 40.0));
    (clip.x, clip.width)
}
#[test]
fn 揃えに応じて空セル側へ広げ印刷倍率を一度だけ適用する() {
    let (sheet, page) = fixture();
    assert_eq!(
        bounds(&sheet, &page, Horizontal::General, false),
        (50.0, 60.0)
    );
    assert_eq!(bounds(&sheet, &page, Horizontal::Left, false), (50.0, 60.0));
    assert_eq!(
        bounds(&sheet, &page, Horizontal::Right, false),
        (10.0, 60.0)
    );
    assert_eq!(
        bounds(&sheet, &page, Horizontal::Center, false),
        (10.0, 100.0)
    );
    assert_eq!(bounds(&sheet, &page, Horizontal::Left, true), (50.0, 20.0));
}
#[test]
fn 空文字列の計算結果を空セル扱いしない() {
    let (mut sheet, page) = fixture();
    sheet.cells.push(Cell {
        address: "D1".into(),
        style: 0,
        value: CellValue::Text("".into()),
        numeric_lexeme: None,
    });
    assert_eq!(bounds(&sheet, &page, Horizontal::Left, false), (50.0, 20.0));
    sheet.cells[1].value = CellValue::Blank;
    assert_eq!(bounds(&sheet, &page, Horizontal::Left, false), (50.0, 60.0));
    sheet.cells[0].value = CellValue::Number(1.0);
    assert_eq!(bounds(&sheet, &page, Horizontal::Left, false), (50.0, 20.0));
}
#[test]
fn 非表示列を飛ばし繰り返し列の境界は越えない() {
    let (mut sheet, mut page) = fixture();
    sheet.cells.push(Cell {
        address: "D1".into(),
        style: 0,
        value: CellValue::Text("非表示の値".into()),
        numeric_lexeme: None,
    });
    let mut columns = page.columns.to_vec();
    columns.remove(3);
    columns[3].offset = 30.0;
    columns[1].repeated = true;
    columns[0].repeated = true;
    page.columns = columns.into();
    assert_eq!(bounds(&sheet, &page, Horizontal::Left, false), (50.0, 40.0));
    assert_eq!(
        bounds(&sheet, &page, Horizontal::Right, false),
        (50.0, 20.0)
    );
}
#[test]
fn 空の結合セルにもはみ出さず結合元からも広げない() {
    let (mut sheet, page) = fixture();
    sheet.grid.merges.push(CellRange::parse("D1:E1").unwrap());
    assert_eq!(bounds(&sheet, &page, Horizontal::Left, false), (50.0, 20.0));
    sheet.grid.merges[0] = CellRange::parse("C1:D1").unwrap();
    let occupancy = Occupancy::new(&sheet, 100).unwrap();
    let placement = range_on_page(&page, sheet.grid.merges[0])
        .unwrap()
        .remove(0);
    let clip = occupancy
        .clip(
            &page,
            &sheet.cells[0],
            &placement,
            Horizontal::Left,
            false,
            100,
        )
        .unwrap();
    assert_eq!((clip.x, clip.width), (50.0, 40.0));
}
#[test]
fn 計算量と位置の不一致を拒否する() {
    let (sheet, page) = fixture();
    assert_eq!(
        Occupancy::new(&sheet, 0).err().unwrap().reason,
        "text_overflow_work_limit"
    );
    let occupancy = Occupancy::new(&sheet, 100).unwrap();
    let mut placement = range_on_page(&page, CellRange::parse("C1").unwrap())
        .unwrap()
        .remove(0);
    let error = occupancy
        .clip(
            &page,
            &sheet.cells[0],
            &placement,
            Horizontal::Left,
            false,
            7,
        )
        .err()
        .unwrap();
    assert_eq!(error.reason, "text_overflow_work_limit");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("C1"));
    placement.rect.width += 1.0;
    assert_eq!(
        occupancy
            .clip(
                &page,
                &sheet.cells[0],
                &placement,
                Horizontal::Left,
                false,
                100
            )
            .err()
            .unwrap()
            .reason,
        "text_overflow_placement_mismatch"
    );
}

#[path = "xlsx_support/mod.rs"]
mod support;
#[test]
fn はみ出し範囲をpdfの切り取りへ渡し文字を保持する() {
    use rhodopsin::xlsx::{
        cell_text::{self, Context, Frame},
        color::ColorContext,
        fonts::{FontBook, FontLimits},
        format::{
            FormatContext,
            date::{DateSystem, Locale},
        },
        pdf,
        styles::Styles,
        theme::TextScript,
        warnings::Warnings,
    };
    let (mut sheet, page) = fixture();
    sheet.cells[0].value = CellValue::Text("AAAAAAAA".into());
    let occupancy = Occupancy::new(&sheet, 100).unwrap();
    let placement = range_on_page(&page, CellRange::parse("C1").unwrap())
        .unwrap()
        .remove(0);
    let clip = occupancy
        .clip(
            &page,
            &sheet.cells[0],
            &placement,
            Horizontal::Left,
            false,
            100,
        )
        .unwrap();
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font><name val="試験"/><sz val="5"/><color rgb="FF000000"/></font></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf><alignment horizontal="left"/></xf></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let styles = Styles::parse(doc.root_element()).unwrap();
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(support::font("試験", false, false, 0), &limits)
        .unwrap();
    let context = Context {
        styles: &styles,
        fonts: &fonts,
        colors: ColorContext {
            theme: None,
            indexed: &[],
            automatic: None,
            foreground: None,
            background: None,
        },
        format: FormatContext {
            date_system: DateSystem::Excel1900,
            locale: Locale::Japanese,
        },
        script: TextScript::Latin,
        normal_script: TextScript::Latin,
        font_limits: &limits,
        warning_limit: 10,
        fallback_family: None,
    };
    let texts = cell_text::plan(
        &sheet.cells[0],
        1,
        Frame {
            content: placement.rect,
            clip,
            scale: page.scale,
            baseline_step: None,
        },
        &context,
        &mut Warnings::default(),
    )
    .unwrap();
    assert_eq!(texts[0].pdf().x, 50.0);
    let bytes = pdf::render(
        &[pdf::Page {
            width: 200.0,
            height: 100.0,
            items: texts
                .iter()
                .map(|text| pdf::Item::Text(text.pdf()))
                .collect(),
        }],
        &fonts,
        &Default::default(),
        &limits,
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "AAAAAAAA");
    let content =
        lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
            .unwrap();
    assert!(
        content.operations.iter().any(|op| op.operator == "re"
            && op.operands == vec![50.into(), 40.into(), 60.into(), 40.into()])
    );
}

#[test]
fn ページ内で走査予算を共有しセルごとにリセットしない() {
    let (sheet, page) = fixture();
    let occupancy = Occupancy::new(&sheet, 100).unwrap();
    let placement = range_on_page(&page, CellRange::parse("C1").unwrap())
        .unwrap()
        .remove(0);
    let mut budget = 10;
    occupancy
        .clip_with_budget(
            &page,
            &sheet.cells[0],
            &placement,
            Horizontal::Left,
            false,
            &mut budget,
        )
        .unwrap();
    assert_eq!(budget, 2);
    let error = occupancy
        .clip_with_budget(
            &page,
            &sheet.cells[0],
            &placement,
            Horizontal::Left,
            false,
            &mut budget,
        )
        .err()
        .unwrap();
    assert_eq!(error.reason, "text_overflow_work_limit");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("C1"));
}
