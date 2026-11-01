#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::{
    cell_text::{self, Context, Frame},
    color::ColorContext,
    fonts::{FontBook, FontLimits},
    format::{
        FormatContext,
        date::{DateSystem, Locale},
    },
    pdf::{self, graphics::BoxRect},
    styles::Styles,
    theme::TextScript,
    warnings::Warnings,
    workbook::{Cell, CellValue},
};
fn styles(alignment: &str) -> Styles {
    let xml = format!(
        r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="164" formatCode="[Red]#,##0"/></numFmts><fonts><font><name val="試験"/><sz val="10"/><color rgb="FF000000"/></font></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="164">{alignment}</xf></cellXfs></styleSheet>"#
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    Styles::parse(doc.root_element()).unwrap()
}
fn context<'a>(styles: &'a Styles, fonts: &'a FontBook, limits: &'a FontLimits) -> Context<'a> {
    Context {
        styles,
        fonts,
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
        font_limits: limits,
        warning_limit: 10,
        fallback_family: None,
    }
}
fn cell() -> Cell {
    Cell {
        address: "C5".into(),
        style: 0,
        value: CellValue::Number(1234.0),
        numeric_lexeme: Some("1234".into()),
    }
}
fn frame(width: f64) -> Frame {
    let rect = BoxRect {
        x: 20.0,
        y: 20.0,
        width,
        height: 20.0,
    };
    Frame {
        content: rect,
        clip: rect,
        scale: 1.0,
        baseline_step: None,
    }
}
fn fonts() -> FontBook {
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(
            support::font("試験", false, false, 0),
            &FontLimits::default(),
        )
        .unwrap();
    fonts
}
#[test]
fn セル書式からpdf文字と色と位置まで接続する() {
    let styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut warnings = Warnings::default();
    let texts = cell_text::plan(&cell(), 1, frame(100.0), &context, &mut warnings).unwrap();
    let text = texts[0].pdf();
    assert_eq!(text.text, "1,234");
    assert_eq!(text.color, [255, 0, 0]);
    assert!((text.x - 93.0).abs() < 1e-10);
    assert_eq!(text.baseline, 38.0);
    let bytes = pdf::render(
        &[pdf::Page {
            width: 200.0,
            height: 100.0,
            items: texts.iter().map(|t| pdf::Item::Text(t.pdf())).collect(),
        }],
        &fonts,
        &Default::default(),
        &limits,
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert!(doc.extract_text(&[1]).unwrap().contains("1,234"));
    assert_eq!(
        serde_json::to_value(warnings.into_summaries()).unwrap(),
        serde_json::json!([])
    );
}
#[test]
fn 幅不足を元セルの警告へ接続する() {
    let styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut warnings = Warnings::default();
    for _ in 0..2 {
        let texts = cell_text::plan(&cell(), 2, frame(18.0), &context, &mut warnings).unwrap();
        assert_eq!(texts[0].pdf().text, "###");
        assert_eq!(texts[0].pdf().color, [255, 0, 0]);
    }
    let report = serde_json::to_value(warnings.into_summaries()).unwrap();
    assert_eq!(report[0]["count"], 1);
    assert_eq!(
        report[0]["locations"][0],
        serde_json::json!({"sheet_index":2,"cell":"C5"})
    );
}
#[test]
fn 未対応配置と不足書体の診断に位置だけを付ける() {
    let limits = FontLimits::default();
    let fonts = fonts();
    let styles = styles(r#"<alignment wrapText="1"/>"#);
    let mut warnings = Warnings::default();
    let error = cell_text::plan(
        &cell(),
        1,
        frame(100.0),
        &context(&styles, &fonts, &limits),
        &mut warnings,
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, "wrapped_cell_text");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("C5"));
    let styles = crate::styles("");
    let empty = FontBook::default();
    let error = cell_text::plan(
        &cell(),
        1,
        frame(100.0),
        &context(&styles, &empty, &limits),
        &mut warnings,
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, "font_family_missing");
    let json = serde_json::to_string(&error).unwrap();
    assert!(!json.contains("試験"));
    assert!(!json.contains("1234"));
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn 明示された代用だけを使い位置付き警告を返す() {
    let styles = styles("");
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(support::font("代用", false, false, 0), &limits)
        .unwrap();
    let mut context = context(&styles, &fonts, &limits);
    let mut warnings = Warnings::default();
    assert!(cell_text::plan(&cell(), 1, frame(18.0), &context, &mut warnings).is_err());
    context.fallback_family = Some("代用");
    // 代用と幅不足の二種類。容量不足なら片方だけを残さない。
    context.warning_limit = 1;
    let error = cell_text::plan(&cell(), 1, frame(18.0), &context, &mut warnings)
        .err()
        .unwrap();
    assert_eq!(error.reason, "warning_limit");
    assert!(warnings.into_summaries().is_empty());
    let mut warnings = Warnings::default();
    context.warning_limit = 2;
    for _ in 0..2 {
        let texts = cell_text::plan(&cell(), 1, frame(18.0), &context, &mut warnings).unwrap();
        assert_eq!(texts[0].pdf().text, "###");
    }
    let report = serde_json::to_value(warnings.into_summaries()).unwrap();
    assert_eq!(report[0]["kind"], "numeric_overflow");
    assert_eq!(report[1]["kind"], "font_substitution");
    assert_eq!(report[1]["count"], 1);
    assert!(!report.to_string().contains("代用"));
}

#[test]
fn 本来の書体がある場合は代用警告を出さない() {
    let styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut context = context(&styles, &fonts, &limits);
    context.fallback_family = Some("存在しない代用書体");
    let mut warnings = Warnings::default();
    cell_text::plan(&cell(), 1, frame(100.0), &context, &mut warnings).unwrap();
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn 明示改行セルの空行を保ってpdfへ配置する() {
    let styles = styles(r#"<alignment wrapText="1"/>"#);
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut value = cell();
    value.value = CellValue::Text("AB\n\nCD".into());
    value.numeric_lexeme = None;
    let mut frame = frame(100.0);
    frame.content.height = 60.0;
    frame.clip.height = 60.0;
    frame.baseline_step = Some(12.0);
    let mut warnings = Warnings::default();
    let texts = cell_text::plan(&value, 1, frame, &context, &mut warnings).unwrap();
    assert_eq!(texts.len(), 2);
    assert_eq!(
        (
            texts[0].pdf().text,
            texts[0].pdf().x,
            texts[0].pdf().baseline
        ),
        ("AB", 20.0, 54.0)
    );
    assert_eq!((texts[1].pdf().text, texts[1].pdf().baseline), ("CD", 78.0));
    let bytes = pdf::render(
        &[pdf::Page {
            width: 200.0,
            height: 100.0,
            items: texts.iter().map(|t| pdf::Item::Text(t.pdf())).collect(),
        }],
        &fonts,
        &Default::default(),
        &limits,
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let extracted = doc.extract_text(&[1]).unwrap();
    assert!(extracted.contains("AB"));
    assert!(extracted.contains("CD"));
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn 行送り未確定と自動折り返しが必要なセルを拒否する() {
    let styles = styles(r#"<alignment wrapText="1"/>"#);
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut value = cell();
    value.value = CellValue::Text("AB\nCD".into());
    value.numeric_lexeme = None;
    let mut warnings = Warnings::default();
    let error = cell_text::plan(&value, 1, frame(100.0), &context, &mut warnings)
        .err()
        .unwrap();
    assert_eq!(error.reason, "cell_line_spacing_unresolved");
    let mut narrow = frame(6.0);
    narrow.baseline_step = Some(12.0);
    let error = cell_text::plan(&value, 1, narrow, &context, &mut warnings)
        .err()
        .unwrap();
    assert_eq!(error.reason, "automatic_text_wrap");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("C5"));
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn 保存サイズの範囲外になる印刷倍率もpdfへ反映する() {
    let mut styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    for (saved, scale, printed) in [(1.0, 0.1, 0.1), (409.0, 4.0, 1636.0)] {
        styles.fonts[0].as_mut().unwrap().size = Some(saved);
        let context = context(&styles, &fonts, &limits);
        let rect = BoxRect {
            x: 20.0,
            y: 20.0,
            width: 10000.0,
            height: 3000.0,
        };
        let frame = Frame {
            content: rect,
            clip: rect,
            scale,
            baseline_step: None,
        };
        let mut warnings = Warnings::default();
        let texts = cell_text::plan(&cell(), 1, frame, &context, &mut warnings).unwrap();
        assert_eq!(texts[0].pdf().points, printed);
        let bytes = pdf::render(
            &[pdf::Page {
                width: 11000.0,
                height: 4000.0,
                items: texts.iter().map(|t| pdf::Item::Text(t.pdf())).collect(),
            }],
            &fonts,
            &Default::default(),
            &limits,
        )
        .unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert!(doc.extract_text(&[1]).unwrap().contains("1,234"));
        let content =
            lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
                .unwrap();
        let size = content
            .operations
            .iter()
            .find(|op| op.operator == "Tf")
            .unwrap()
            .operands[1]
            .as_float()
            .unwrap();
        assert!((f64::from(size) - printed).abs() < 1e-6);
    }
}

#[test]
fn 印刷倍率対応で保存サイズの制限を緩めない() {
    let mut styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut warnings = Warnings::default();
    for saved in [0.5, 410.0] {
        styles.fonts[0].as_mut().unwrap().size = Some(saved);
        let error = cell_text::plan(
            &cell(),
            1,
            frame(100.0),
            &context(&styles, &fonts, &limits),
            &mut warnings,
        )
        .err()
        .unwrap();
        assert_eq!(error.reason, "font_size_missing_or_invalid");
    }
    styles.fonts[0].as_mut().unwrap().size = Some(10.0);
    for scale in [0.09, 4.01, f64::NAN] {
        let mut frame = frame(100.0);
        frame.scale = scale;
        assert!(
            cell_text::plan(
                &cell(),
                1,
                frame,
                &context(&styles, &fonts, &limits),
                &mut warnings
            )
            .is_err()
        );
    }
}

fn page_fixture() -> (
    rhodopsin::xlsx::workbook::Sheet,
    rhodopsin::xlsx::layout::Page,
) {
    use rhodopsin::xlsx::{
        grid::Grid,
        layout::{AxisItem, Page, Paper},
        print::PrintSettings,
        range::CellRange,
        workbook::Sheet,
    };
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let mut a = cell();
    a.address = "A1".into();
    let mut b = cell();
    b.address = "B1".into();
    b.style = 999;
    let mut c = cell();
    c.address = "C1".into();
    let sheet = Sheet {
        index: 1,
        hidden: false,
        cells: vec![c, b, a],
        grid: Grid::parse(doc.root_element()).unwrap(),
        print: PrintSettings::parse(doc.root_element()).unwrap(),
        pictures: vec![],
    };
    let page = Page {
        sheet_index: 1,
        area_index: 0,
        paper: Paper {
            width: 200.0,
            height: 100.0,
        },
        scale: 1.0,
        origin_x: 10.0,
        origin_y: 20.0,
        columns: vec![
            AxisItem {
                index: 1,
                offset: 0.0,
                size: 18.0,
                repeated: true,
            },
            AxisItem {
                index: 3,
                offset: 18.0,
                size: 18.0,
                repeated: false,
            },
        ]
        .into(),
        rows: vec![AxisItem {
            index: 1,
            offset: 0.0,
            size: 20.0,
            repeated: false,
        }]
        .into(),
        body: CellRange::parse("C1").unwrap(),
    };
    (sheet, page)
}
#[test]
fn 可視セルをページ順に配置し警告をまとめる() {
    let (sheet, page) = page_fixture();
    let styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut warnings = Warnings::default();
    let mut visited = Vec::new();
    let texts = cell_text::plan_page(
        &sheet,
        &page,
        &context(&styles, &fonts, &limits),
        &mut warnings,
        10,
        |cell, placement| {
            visited.push((cell.address.clone(), placement.repeated_columns));
            Ok(Frame {
                content: placement.rect,
                clip: placement.rect,
                scale: page.scale,
                baseline_step: None,
            })
        },
    )
    .unwrap();
    assert_eq!(visited, [("A1".into(), true), ("C1".into(), false)]);
    assert_eq!(texts.len(), 2);
    assert_eq!((texts[0].pdf().x, texts[1].pdf().x), (10.0, 28.0));
    assert_eq!(
        serde_json::to_value(warnings.into_summaries()).unwrap()[0]["count"],
        2
    );
}
#[test]
fn ページ途中の失敗で警告を部分的に確定しない() {
    let (sheet, page) = page_fixture();
    let styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut warnings = Warnings::default();
    let result = cell_text::plan_page(
        &sheet,
        &page,
        &context(&styles, &fonts, &limits),
        &mut warnings,
        10,
        |cell, placement| {
            if cell.address == "C1" {
                return Err(rhodopsin::xlsx::diagnostic::Failure::unsupported(
                    "test_frame_unresolved",
                ));
            }
            Ok(Frame {
                content: placement.rect,
                clip: placement.rect,
                scale: page.scale,
                baseline_step: None,
            })
        },
    );
    let error = result.err().unwrap();
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("C1"));
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn 字下げはセル書体ではなく標準書体の空白幅と印刷倍率を使う() {
    let fonts = fonts();
    let limits = FontLimits::default();
    for horizontal in ["left", "right"] {
        for scale in [0.5, 1.0, 2.0] {
            let xml = format!(
                r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
              <fonts><font><name val="試験"/><sz val="10"/><color rgb="FF000000"/></font><font><name val="試験"/><sz val="20"/></font></fonts>
              <fills><fill/></fills><borders><border/></borders>
              <cellStyleXfs><xf fontId="0"/><xf fontId="1"/></cellStyleXfs>
              <cellStyles><cellStyle name="任意の表示名" builtinId="0" xfId="1"/></cellStyles>
              <cellXfs><xf fontId="0"><alignment horizontal="{horizontal}" indent="2"/></xf></cellXfs>
              </styleSheet>"#
            );
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let styles = Styles::parse(doc.root_element()).unwrap();
            assert_eq!(styles.normal_font, Some(1));
            let context = context(&styles, &fonts, &limits);
            let mut cell = cell();
            cell.value = CellValue::Text("AB".into());
            let mut frame = frame(200.0);
            frame.scale = scale;
            let mut warnings = Warnings::default();
            let texts = cell_text::plan(&cell, 1, frame, &context, &mut warnings).unwrap();
            let indent = fonts.faces[0]
                .shape_run(" ", 20.0 * scale, &limits)
                .unwrap()
                .advance
                * 6.0;
            let advance = fonts.faces[0]
                .shape_run("AB", 10.0 * scale, &limits)
                .unwrap()
                .advance;
            let expected = if horizontal == "left" {
                20.0 + indent
            } else {
                220.0 - indent - advance
            };
            assert!((texts[0].pdf().x - expected).abs() < 1e-10);
            assert!(warnings.into_summaries().is_empty());
        }
    }
}

#[test]
fn 字下げの標準書体不明と幅不足を近似せず停止する() {
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut styles = styles(r#"<alignment horizontal="left" indent="2"/>"#);
    for (normal, width, reason) in [
        (None, 100.0, "normal_style_font_unresolved"),
        (Some(0), 1.0, "indent_exceeds_content_width"),
    ] {
        styles.normal_font = normal;
        let mut warnings = Warnings::default();
        let error = cell_text::plan(
            &cell(),
            2,
            frame(width),
            &context(&styles, &fonts, &limits),
            &mut warnings,
        )
        .err()
        .unwrap();
        assert_eq!(error.reason, reason);
        assert_eq!(error.location.unwrap().cell.as_deref(), Some("C5"));
        assert!(warnings.into_summaries().is_empty());
    }
}

fn merged_fixture() -> (
    rhodopsin::xlsx::workbook::Sheet,
    rhodopsin::xlsx::layout::Page,
) {
    use rhodopsin::xlsx::{layout::AxisItem, range::CellRange};
    let (mut sheet, mut page) = page_fixture();
    sheet.grid.merges = vec![CellRange::parse("A1:C2").unwrap()];
    for cell in &mut sheet.cells {
        if cell.address != "A1" {
            cell.value = CellValue::Blank;
        }
    }
    page.columns = (1..=3)
        .map(|index| AxisItem {
            index,
            offset: f64::from(index - 1) * 18.0,
            size: 18.0,
            repeated: false,
        })
        .collect::<Vec<_>>()
        .into();
    page.rows = (1..=2)
        .map(|index| AxisItem {
            index,
            offset: f64::from(index - 1) * 20.0,
            size: 20.0,
            repeated: false,
        })
        .collect::<Vec<_>>()
        .into();
    page.body = CellRange::parse("A1:C2").unwrap();
    (sheet, page)
}

#[test]
fn 結合範囲全体へ左上セルの文字を一度だけ配置する() {
    let (sheet, page) = merged_fixture();
    let styles = styles(r#"<alignment horizontal="center" vertical="center"/>"#);
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut warnings = Warnings::default();
    let mut visited = 0;
    let texts = cell_text::plan_page(
        &sheet,
        &page,
        &context(&styles, &fonts, &limits),
        &mut warnings,
        100,
        |cell, placement| {
            visited += 1;
            assert_eq!(cell.address, "A1");
            assert_eq!(placement.visible, sheet.grid.merges[0]);
            assert_eq!(placement.rect.width, 54.0);
            assert_eq!(placement.rect.height, 40.0);
            Ok(Frame {
                content: placement.rect,
                clip: placement.rect,
                scale: page.scale,
                baseline_step: None,
            })
        },
    )
    .unwrap();
    assert_eq!(visited, 1);
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].pdf().text, "1,234");
    assert_eq!(texts[0].pdf().x, 23.5);
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn 結合の分断と左上以外の値を黙って描画しない() {
    let styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    for mode in 0..5 {
        let (mut sheet, mut page) = merged_fixture();
        let expected = match mode {
            0 => {
                // 途中の列が非表示
                page.columns = vec![page.columns[0].clone(), page.columns[2].clone()].into();
                "split_merged_cell_text"
            }
            1 => {
                // 結合の下半分がページ外
                page.rows = vec![page.rows[0].clone()].into();
                "split_merged_cell_text"
            }
            2 => {
                // 繰り返し見出しと本文をまたぐ
                let mut columns = page.columns.to_vec();
                columns[0].repeated = true;
                page.columns = columns.into();
                "split_merged_cell_text"
            }
            3 => {
                sheet.cells[0].value = CellValue::Text("機密".into());
                "merged_nonanchor_value"
            }
            _ => {
                sheet.grid.merges.push(sheet.grid.merges[0]);
                "overlapping_merged_cells"
            }
        };
        let mut warnings = Warnings::default();
        let error = cell_text::plan_page(
            &sheet,
            &page,
            &context(&styles, &fonts, &limits),
            &mut warnings,
            100,
            |_, _| panic!("結合検査が失敗する場合は配置しない"),
        )
        .err()
        .unwrap();
        assert_eq!(error.reason, expected);
        assert_eq!(error.location.as_ref().unwrap().sheet_index, 1);
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
        assert!(warnings.into_summaries().is_empty());
    }
}

#[test]
fn 組み込み指数書式をセル文字と幅不足の警告へ接続する() {
    let mut styles = styles("");
    styles.cells[0].as_mut().unwrap().number_format = 11;
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut cell = cell();
    cell.value = CellValue::Number(12200000.0);
    cell.numeric_lexeme = Some("12200000".into());
    let mut warnings = Warnings::default();
    let context = context(&styles, &fonts, &limits);
    let texts = cell_text::plan(&cell, 1, frame(100.0), &context, &mut warnings).unwrap();
    assert_eq!(texts[0].pdf().text, "1.22E+07");
    let texts = cell_text::plan(&cell, 1, frame(18.0), &context, &mut warnings).unwrap();
    assert_eq!(texts[0].pdf().text, "###");
    assert_eq!(
        serde_json::to_value(warnings.into_summaries()).unwrap()[0]["kind"],
        "numeric_overflow"
    );
}

#[test]
fn 地域依存の会計番号は保存書式を使って記号と数字を配置する() {
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut styles = styles("");
    styles.cells[0].as_mut().unwrap().number_format = 44;
    styles.number_formats.insert(
        44,
        r#"_($* #,##0.00_);_($* (#,##0.00);_($* "-"??_);_(@_)"#.into(),
    );
    let context = context(&styles, &fonts, &limits);
    // 日本語ロケールでも、保存されたドルの書式を円へ置き換えない。
    for (raw, expected) in [
        ("1234.5", "1,234.50"),
        ("-1234.5", "(1,234.50)"),
        ("0", "-"),
    ] {
        let mut cell = cell();
        cell.value = CellValue::Number(raw.parse().unwrap());
        cell.numeric_lexeme = Some(raw.into());
        let texts =
            cell_text::plan(&cell, 1, frame(150.0), &context, &mut Warnings::default()).unwrap();
        let text: String = texts.iter().map(|item| item.pdf().text).collect();
        assert!(text.contains('$'));
        assert!(text.contains(expected));
        let currency = texts
            .iter()
            .find(|text| text.pdf().text.contains('$'))
            .unwrap()
            .pdf();
        let number = texts
            .iter()
            .find(|text| text.pdf().text.contains(expected))
            .unwrap()
            .pdf();
        assert!(currency.x < number.x);
    }
}

#[test]
fn 折り返し指定でも一行に収まる文字は実幅で配置する() {
    let styles = styles(r#"<alignment wrapText="1" horizontal="center"/>"#);
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut cell = cell();
    cell.value = CellValue::Text("AB見".into());
    let width = fonts.faces[0]
        .shape_run("AB見", 10.0, &limits)
        .unwrap()
        .advance;
    for scale in [0.5, 1.0, 2.0] {
        for extra in [0.0, 10.0] {
            let mut frame = frame(width * scale + extra);
            frame.scale = scale;
            let mut warnings = Warnings::default();
            let texts = cell_text::plan(&cell, 1, frame, &context, &mut warnings).unwrap();
            assert_eq!(texts.len(), 1);
            assert_eq!(texts[0].pdf().text, "AB見");
            assert_eq!(texts[0].pdf().x, 20.0 + extra / 2.0);
            assert!(warnings.into_summaries().is_empty());
        }
    }
}

#[test]
fn 折り返しが必要な実幅では切り取りで代用しない() {
    let mut styles = styles(r#"<alignment wrapText="1" horizontal="left" indent="1"/>"#);
    styles.normal_font = Some(0);
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut cell = cell();
    cell.value = CellValue::Text("AB見".into());
    let width = fonts.faces[0]
        .shape_run("AB見", 10.0, &limits)
        .unwrap()
        .advance;
    let indent = fonts.faces[0]
        .shape_run(" ", 10.0, &limits)
        .unwrap()
        .advance
        * 3.0;
    let mut warnings = Warnings::default();
    let error = cell_text::plan(
        &cell,
        3,
        frame(width + indent - 0.01),
        &context,
        &mut warnings,
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, "automatic_text_wrap");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("C5"));
    assert!(warnings.clone().into_summaries().is_empty());
    assert!(cell_text::plan(&cell, 3, frame(width + indent), &context, &mut warnings).is_ok());
}

#[test]
fn 一重下線の上端と厚さを計測し文字と同じ色と切取範囲で描く() {
    let mut styles = styles("");
    styles.fonts[0].as_mut().unwrap().underline = rhodopsin::xlsx::styles::Underline::Single;
    let mut fonts = FontBook::default();
    let limits = FontLimits::default();
    fonts
        .load_bytes(
            support::font_with_underline("試験", false, false, 0, Some((-100, 50))),
            &limits,
        )
        .unwrap();
    let context = context(&styles, &fonts, &limits);
    let mut warnings = Warnings::default();
    let texts = cell_text::plan(&cell(), 1, frame(100.0), &context, &mut warnings).unwrap();
    let decoration = texts[0].decoration().unwrap();
    match &decoration {
        pdf::graphics::Paint::Fill { rect, color } => {
            assert_eq!(rect.y, texts[0].pdf().baseline + 1.0);
            assert_eq!(rect.height, 0.5);
            assert_eq!(rect.x, texts[0].pdf().x);
            assert_eq!(*color, [255, 0, 0]);
        }
        _ => panic!("下線は矩形で描画する"),
    }
    let bytes = pdf::render(
        &[pdf::Page {
            width: 200.0,
            height: 100.0,
            items: vec![
                pdf::Item::Text(texts[0].pdf()),
                pdf::Item::Paint(decoration),
            ],
        }],
        &fonts,
        &Default::default(),
        &limits,
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "1,234");
    let mut clipped = frame(100.0);
    clipped.clip.height = 19.25;
    let texts = cell_text::plan(&cell(), 1, clipped, &context, &mut warnings).unwrap();
    match texts[0].decoration().unwrap() {
        pdf::graphics::Paint::Fill { rect, .. } => assert_eq!(rect.height, 0.25),
        _ => unreachable!(),
    }
}

#[test]
fn 下線寸法の欠落と未対応の下線種を近似しない() {
    let mut styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    for (underline, reason) in [
        (
            rhodopsin::xlsx::styles::Underline::Single,
            "font_underline_metrics_missing",
        ),
        (
            rhodopsin::xlsx::styles::Underline::Double,
            "underlined_cell_text",
        ),
        (
            rhodopsin::xlsx::styles::Underline::SingleAccounting,
            "underlined_cell_text",
        ),
    ] {
        styles.fonts[0].as_mut().unwrap().underline = underline;
        let mut warnings = Warnings::default();
        let error = cell_text::plan(
            &cell(),
            1,
            frame(100.0),
            &context(&styles, &fonts, &limits),
            &mut warnings,
        )
        .err()
        .unwrap();
        assert_eq!(error.reason, reason);
        assert_eq!(error.location.unwrap().cell.as_deref(), Some("C5"));
    }
}

#[test]
fn 日付書式の指定色をpdfの文字色へ渡す() {
    let mut styles = styles("");
    styles.number_formats.insert(164, "[Blue]yyyy-mm-dd".into());
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut value = cell();
    value.value = CellValue::Number(43586.0);
    value.numeric_lexeme = Some("43586".into());
    let texts =
        cell_text::plan(&value, 1, frame(100.0), &context, &mut Warnings::default()).unwrap();
    assert_eq!(texts[0].pdf().text, "2019-05-01");
    assert_eq!(texts[0].pdf().color, [0, 0, 255]);
    let bytes = pdf::render(
        &[pdf::Page {
            width: 200.0,
            height: 100.0,
            items: vec![pdf::Item::Text(texts[0].pdf())],
        }],
        &fonts,
        &Default::default(),
        &limits,
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "2019-05-01");
    let content =
        lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
            .unwrap();
    assert!(
        content
            .operations
            .iter()
            .any(|op| op.operator == "rg" && op.operands == vec![0.into(), 0.into(), 1.into()])
    );
}

#[test]
fn 自動折り返しの各行を指定行送りでpdfへ配置する() {
    let styles = styles(r#"<alignment horizontal="left" vertical="top" wrapText="1"/>"#);
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut cell = cell();
    cell.value = CellValue::Text("AB CD".into());
    cell.numeric_lexeme = None;
    let mut frame = frame(18.0);
    frame.content.height = 40.0;
    frame.clip.height = 40.0;
    frame.baseline_step = Some(12.0);
    let texts = cell_text::plan(&cell, 1, frame, &context, &mut Warnings::default()).unwrap();
    assert_eq!(
        texts.iter().map(|text| text.pdf().text).collect::<Vec<_>>(),
        ["AB ", "CD"]
    );
    assert_eq!(texts[1].pdf().baseline - texts[0].pdf().baseline, 12.0);
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
    let extracted = doc.extract_text(&[1]).unwrap();
    assert!(extracted.contains("AB "));
    assert!(extracted.contains("CD"));
}

#[test]
fn ページ文字量を各セル追加時に検査し後続セルと警告確定を止める() {
    let (sheet, page) = page_fixture();
    let styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    for (budget, expected_visited, succeeds) in [
        (2, vec!["A1"], false),
        (5, vec!["A1", "C1"], false),
        (6, vec!["A1", "C1"], true),
    ] {
        let mut visited = Vec::new();
        let mut warnings = Warnings::default();
        let result = cell_text::plan_page_limited(
            &sheet,
            &page,
            &context,
            &mut warnings,
            100,
            budget,
            |cell, placement| {
                visited.push(cell.address.clone());
                Ok(Frame {
                    content: placement.rect,
                    clip: placement.rect,
                    scale: page.scale,
                    baseline_step: None,
                })
            },
        );
        assert_eq!(visited, expected_visited);
        if succeeds {
            let texts = result.unwrap();
            assert_eq!(
                texts
                    .iter()
                    .map(|text| text.pdf().text.len())
                    .sum::<usize>(),
                6
            );
            assert!(!warnings.into_summaries().is_empty());
        } else {
            let error = result.err().unwrap();
            assert_eq!(error.reason, "page_scene_text_limit");
            assert_eq!(
                error.location.unwrap().cell.as_deref(),
                visited.last().map(String::as_str)
            );
            assert!(warnings.into_summaries().is_empty());
        }
    }
}

#[test]
fn ページ文字量は文字数ではなく生成utf8バイト数で数える() {
    let (mut sheet, page) = page_fixture();
    sheet.cells.retain(|cell| cell.address == "A1");
    sheet.cells[0].value = CellValue::Text("見見".into());
    sheet.cells[0].numeric_lexeme = None;
    let styles = styles("");
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let render = |budget| {
        cell_text::plan_page_limited(
            &sheet,
            &page,
            &context,
            &mut Warnings::default(),
            100,
            budget,
            |_, placement| {
                Ok(Frame {
                    content: placement.rect,
                    clip: placement.rect,
                    scale: page.scale,
                    baseline_step: None,
                })
            },
        )
    };
    assert_eq!(render(5).err().unwrap().reason, "page_scene_text_limit");
    assert_eq!(render(6).unwrap()[0].pdf().text, "見見");
}

#[test]
fn テーマ本文と標準書体の文字体系を字下げで混同しない() {
    use rhodopsin::xlsx::{
        styles::FontScheme,
        theme::{Theme, ThemeFonts},
    };
    let mut styles = styles(r#"<alignment horizontal="left" indent="1"/>"#);
    styles.normal_font = Some(0);
    styles.fonts[0].as_mut().unwrap().scheme = FontScheme::Minor;
    let theme = Theme {
        colors: [[0; 3]; 12],
        major: ThemeFonts::default(),
        minor: ThemeFonts {
            latin: "試験".into(),
            east_asian: "和文試験".into(),
            ..Default::default()
        },
    };
    let mut fonts = fonts();
    let limits = FontLimits::default();
    let mut japanese = support::font("和文試験", false, false, 0);
    let count = u16::from_be_bytes(japanese[4..6].try_into().unwrap()) as usize;
    let record = (0..count)
        .map(|index| 12 + index * 16)
        .find(|offset| &japanese[*offset..*offset + 4] == b"hmtx")
        .unwrap();
    let offset = u32::from_be_bytes(japanese[record + 8..record + 12].try_into().unwrap()) as usize;
    // 試験フォントの空白は glyph 1。欧文の 600 units に対し和文を 200 にする。
    japanese[offset + 4..offset + 6].copy_from_slice(&200u16.to_be_bytes());
    fonts.load_bytes(japanese, &limits).unwrap();
    let mut context = context(&styles, &fonts, &limits);
    context.colors.theme = Some(&theme);
    context.script = TextScript::EastAsian("Jpan");
    context.normal_script = TextScript::Latin;
    let mut cell = cell();
    cell.value = CellValue::Text("AB".into());
    cell.numeric_lexeme = None;
    let mut warnings = Warnings::default();
    let texts = cell_text::plan(&cell, 1, frame(100.0), &context, &mut warnings).unwrap();
    assert_eq!(texts[0].pdf().font, 1); // 本文は和文テーマ。
    assert_eq!(texts[0].pdf().x, 38.0); // 標準欧文 6 pt × 3 + 左端 20 pt。
    context.normal_script = TextScript::EastAsian("Jpan");
    let texts = cell_text::plan(&cell, 1, frame(100.0), &context, &mut warnings).unwrap();
    assert_eq!(texts[0].pdf().font, 1);
    assert_eq!(texts[0].pdf().x, 26.0); // 標準和文 2 pt × 3 + 左端。
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn 組み込み工学書式をpdf抽出と幅不足警告へ接続する() {
    let mut styles = styles("");
    styles.cells[0].as_mut().unwrap().number_format = 48;
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    for (width, expected, overflow) in [(100.0, "1.2E+3", false), (18.0, "###", true)] {
        let mut warnings = Warnings::default();
        let texts = cell_text::plan(&cell(), 1, frame(width), &context, &mut warnings).unwrap();
        let bytes = pdf::render(
            &[pdf::Page {
                width: 200.0,
                height: 100.0,
                items: texts.iter().map(|t| pdf::Item::Text(t.pdf())).collect(),
            }],
            &fonts,
            &Default::default(),
            &limits,
        )
        .unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.extract_text(&[1]).unwrap().trim(), expected);
        let warnings = warnings.into_summaries();
        assert_eq!(!warnings.is_empty(), overflow);
        if overflow {
            let json = serde_json::to_value(warnings).unwrap();
            assert_eq!(json[0]["kind"], "numeric_overflow");
            assert_eq!(json[0]["locations"][0]["cell"], "C5");
        }
    }
}

fn rich_cell(body: &str) -> Cell {
    let xml = format!(
        r#"<si xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{body}</si>"#
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    Cell {
        address: "C5".into(),
        style: 0,
        value: rhodopsin::xlsx::rich_text::parse(doc.root_element()).unwrap(),
        numeric_lexeme: None,
    }
}
fn rich_styles(alignment: &str) -> Styles {
    let mut styles = styles(alignment);
    styles.cells[0].as_mut().unwrap().number_format = 0;
    styles
}

#[test]
fn リッチテキストの書体色サイズと抽出順をpdfまで保存する() {
    let cell = rich_cell(
        r#"<r><t>A</t></r><r><rPr><rFont val="別書体"/><sz val="16"/><b/><i/><u/><color rgb="FFFF0000"/></rPr><t>B</t></r><r><t>C</t></r>"#,
    );
    let styles = rich_styles("");
    let mut fonts = fonts();
    fonts
        .load_bytes(
            support::font_with_underline("別書体", true, true, 0, Some((-100, 50))),
            &FontLimits::default(),
        )
        .unwrap();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let texts =
        cell_text::plan(&cell, 1, frame(100.0), &context, &mut Warnings::default()).unwrap();
    assert_eq!(texts.len(), 3);
    let a = texts[0].pdf();
    let b = texts[1].pdf();
    let c = texts[2].pdf();
    assert_eq!((a.text, b.text, c.text), ("A", "B", "C"));
    assert_eq!((a.points, b.points, c.points), (10.0, 16.0, 10.0));
    assert_eq!(b.color, [255, 0, 0]);
    assert_ne!(a.font, b.font);
    assert_eq!(a.font, c.font);
    assert_eq!(a.baseline, b.baseline);
    assert_eq!(b.baseline, c.baseline);
    assert!(a.x < b.x && b.x < c.x);
    assert!(texts[0].decoration().is_none());
    assert!(texts[1].decoration().is_some());
    let render = || {
        pdf::render(
            &[pdf::Page {
                width: 200.0,
                height: 100.0,
                items: cell_text::pdf_items(&texts),
            }],
            &fonts,
            &Default::default(),
            &limits,
        )
        .unwrap()
    };
    let bytes = render();
    assert_eq!(bytes, render());
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "ABC");
    assert_eq!(
        rhodopsin::xlsx::format::cell_with_context(&cell, Some(&styles), context.format)
            .unwrap_err()
            .reason,
        "rich_text_requires_run_layout"
    );
}

#[test]
fn リッチテキストの書式境界を跨いで折り返し空行も保持する() {
    let cell = rich_cell(
        r#"<r><t xml:space="preserve">A </t></r><r><rPr><rFont val="試験"/><sz val="10"/><color rgb="FF0000FF"/></rPr><t xml:space="preserve">B C&#10;&#10;D&#10;</t></r>"#,
    );
    let styles = rich_styles(r#"<alignment wrapText="1" vertical="top"/>"#);
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let width = fonts.faces[0]
        .shape_run("A B ", 10.0, &limits)
        .unwrap()
        .advance
        + 0.01;
    let mut frame = frame(width);
    frame.baseline_step = Some(12.0);
    let texts = cell_text::plan(&cell, 1, frame, &context, &mut Warnings::default()).unwrap();
    assert_eq!(
        texts.iter().map(|t| t.pdf().text).collect::<String>(),
        "A B CD"
    );
    assert_eq!(texts[0].pdf().baseline, texts[1].pdf().baseline);
    assert_eq!(texts[2].pdf().baseline - texts[0].pdf().baseline, 12.0);
    assert_eq!(texts[3].pdf().baseline - texts[0].pdf().baseline, 36.0);
    assert_eq!(texts[1].pdf().color, [0, 0, 255]);
    assert_eq!(texts[2].pdf().color, [0, 0, 255]);
}

#[test]
fn リッチテキストで未対応書式と資源超過を成功扱いしない() {
    let cell = rich_cell(r#"<r><t>A</t></r><r><t>B</t></r>"#);
    let fonts = fonts();
    let limits = FontLimits::default();
    let formatted = styles("");
    let texts = cell_text::plan(
        &cell,
        1,
        frame(100.0),
        &context(&formatted, &fonts, &limits),
        &mut Warnings::default(),
    )
    .unwrap();
    assert_eq!(
        texts.iter().map(|text| text.pdf().text).collect::<String>(),
        "AB"
    );
    assert!(texts.iter().all(|text| text.pdf().color == [0, 0, 0]));
    let styles = rich_styles("");
    let small = FontLimits {
        run_bytes: 1,
        ..limits
    };
    assert_eq!(
        cell_text::plan(
            &cell,
            1,
            frame(100.0),
            &context(&styles, &fonts, &small),
            &mut Warnings::default()
        )
        .err()
        .unwrap()
        .reason,
        "text_run_limit"
    );
    let partial = rich_cell(r#"<r><rPr><b/></rPr><t>A</t></r>"#);
    assert!(
        cell_text::plan(
            &partial,
            1,
            frame(100.0),
            &context(&styles, &fonts, &FontLimits::default()),
            &mut Warnings::default()
        )
        .is_err()
    );
}

#[test]
fn 書式境界だけを結合し別セルと明示改行を区別する() {
    let mut first = rich_cell(r#"<r><t>A</t></r><r><t>B&#10;C</t></r>"#);
    first.address = "A1".into();
    let mut second = rich_cell(r#"<r><t>D</t></r><r><t>E</t></r>"#);
    second.address = "B1".into();
    let styles = rich_styles(r#"<alignment wrapText="1"/>"#);
    let fonts = fonts();
    let limits = FontLimits::default();
    let context = context(&styles, &fonts, &limits);
    let mut f = frame(100.0);
    f.baseline_step = Some(12.0);
    let mut texts = cell_text::plan(&first, 1, f, &context, &mut Warnings::default()).unwrap();
    let mut f = frame(100.0);
    f.content.y = texts.last().unwrap().pdf().baseline - 18.0;
    texts.extend(cell_text::plan(&second, 1, f, &context, &mut Warnings::default()).unwrap());
    let items = cell_text::pdf_items(&texts);
    assert_eq!(items.len(), 3);
    assert!(matches!(&items[0],pdf::Item::TextLine(line) if line.len()==2));
    assert!(matches!(&items[1], pdf::Item::Text(_)));
    assert!(matches!(&items[2],pdf::Item::TextLine(line) if line.len()==2));
    let bytes = pdf::render(
        &[pdf::Page {
            width: 200.0,
            height: 100.0,
            items,
        }],
        &fonts,
        &Default::default(),
        &limits,
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap(), "AB\nC\nDE\n");
}

#[test]
fn 標準数値を幅で選びセル配置とpdf抽出へ接続する() {
    let fonts = fonts();
    let limits = FontLimits::default();
    for custom in [false, true] {
        let mut styles = styles("");
        if custom {
            styles.number_formats.insert(164, "gEnErAl".into());
        } else {
            styles.cells[0].as_mut().unwrap().number_format = 0;
        }
        let context = context(&styles, &fonts, &limits);
        for (raw, width, expected, hashes) in [
            ("123.4500", 100.0, "123.45", false),
            ("123456789012", 100.0, "1.23457E+11", false),
            ("123456789012", 18.0, "###", true),
        ] {
            let mut cell = cell();
            cell.numeric_lexeme = Some(raw.into());
            cell.value = CellValue::Number(raw.parse().unwrap());
            let mut warnings = Warnings::default();
            let texts = cell_text::plan(&cell, 1, frame(width), &context, &mut warnings).unwrap();
            assert_eq!(
                texts.iter().map(|text| text.pdf().text).collect::<String>(),
                expected
            );
            assert_eq!(texts[0].pdf().color, [0, 0, 0]);
            let page = pdf::Page {
                width: 200.0,
                height: 200.0,
                items: texts
                    .iter()
                    .map(|text| pdf::Item::Text(text.pdf()))
                    .collect(),
            };
            let bytes = pdf::render(
                std::slice::from_ref(&page),
                &fonts,
                &Default::default(),
                &limits,
            )
            .unwrap();
            assert_eq!(
                bytes,
                pdf::render(&[page], &fonts, &Default::default(), &limits).unwrap()
            );
            let doc = lopdf::Document::load_mem(&bytes).unwrap();
            assert_eq!(doc.extract_text(&[1]).unwrap().trim(), expected);
            let summaries = serde_json::to_value(warnings.into_summaries()).unwrap();
            assert_eq!(summaries.as_array().unwrap().len(), usize::from(hashes));
            if hashes {
                assert_eq!(summaries[0]["locations"][0]["cell"], "C5");
                assert_eq!(summaries[0]["kind"], "numeric_overflow");
            }
        }
    }
}

#[test]
fn 標準数値でも保存字句の欠落と幅不足警告の上限を守る() {
    let mut styles = styles("");
    styles.cells[0].as_mut().unwrap().number_format = 0;
    let fonts = fonts();
    let limits = FontLimits::default();
    let mut context = context(&styles, &fonts, &limits);
    let mut value = cell();
    value.numeric_lexeme = None;
    let mut warnings = Warnings::default();
    let error = cell_text::plan(&value, 1, frame(100.0), &context, &mut warnings)
        .err()
        .unwrap();
    assert_eq!(error.reason, "numeric_lexeme_missing");
    assert_eq!(error.location.unwrap().cell.as_deref(), Some("C5"));
    value.numeric_lexeme = Some("123456789012".into());
    context.warning_limit = 0;
    let error = cell_text::plan(&value, 1, frame(18.0), &context, &mut warnings)
        .err()
        .unwrap();
    assert_eq!(error.reason, "warning_limit");
    assert!(warnings.into_summaries().is_empty());
}

#[test]
fn リッチテキストの文字区分を展開して書体境界とpdf抽出を保存する() {
    let cell = rich_cell(
        r#"<r><t>A</t></r><r><rPr><rFont val="試験"/><sz val="16"/><color rgb="FF0000FF"/></rPr><t>B</t></r>"#,
    );
    let fonts = fonts();
    let limits = FontLimits::default();
    for (code, expected, color) in [
        (r#"0;0;0;"C"@"D""#, "CABD", None),
        ("0;0;0;[Red]@", "AB", Some([255, 0, 0])),
        ("0;0;0;@@", "ABAB", None),
        ("0;0;0;", "", None),
        ("yyyy;0;0;@", "AB", None),
    ] {
        let mut styles = rich_styles("");
        styles.cells[0].as_mut().unwrap().number_format = 164;
        styles.number_formats.insert(164, code.into());
        let texts = cell_text::plan(
            &cell,
            1,
            frame(200.0),
            &context(&styles, &fonts, &limits),
            &mut Warnings::default(),
        )
        .unwrap();
        assert_eq!(
            texts.iter().map(|text| text.pdf().text).collect::<String>(),
            expected
        );
        for text in &texts {
            let part = text.pdf();
            assert_eq!(part.points, if part.text == "B" { 16.0 } else { 10.0 });
            assert_eq!(
                part.color,
                color.unwrap_or(if part.text == "B" {
                    [0, 0, 255]
                } else {
                    [0, 0, 0]
                })
            );
        }
        let page = pdf::Page {
            width: 300.0,
            height: 100.0,
            items: cell_text::pdf_items(&texts),
        };
        let bytes = pdf::render(
            std::slice::from_ref(&page),
            &fonts,
            &Default::default(),
            &limits,
        )
        .unwrap();
        assert_eq!(
            bytes,
            pdf::render(&[page], &fonts, &Default::default(), &limits).unwrap()
        );
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.extract_text(&[1]).unwrap().trim(), expected);
    }
}

#[test]
fn リッチテキストの文字区分で展開上限と未対応空白を検査する() {
    let cell = rich_cell("<r><t>A</t></r><r><t>B</t></r>");
    let fonts = fonts();
    let limits = FontLimits::default();
    for (code, reason) in [
        ("@".repeat(2049), "text_fragment_limit"),
        ("0;0;0;_x@".into(), "rich_text_format_spacing"),
        ("0;0;0;*x@".into(), "rich_text_format_spacing"),
    ] {
        let mut styles = rich_styles("");
        styles.cells[0].as_mut().unwrap().number_format = 164;
        styles.number_formats.insert(164, code);
        let error = cell_text::plan(
            &cell,
            1,
            frame(200.0),
            &context(&styles, &fonts, &limits),
            &mut Warnings::default(),
        )
        .err()
        .unwrap();
        assert_eq!(error.reason, reason);
    }
}

#[test]
fn 分数の桁合わせと幅不足をpdf抽出へ接続する() {
    let fonts = fonts();
    let limits = FontLimits::default();
    for (id, width, expected, overflow) in [
        (12, 100.0, "1 4/7", false),
        (13, 100.0, "1 9/16", false),
        (13, 18.0, "###", true),
    ] {
        let mut styles = styles("");
        styles.cells[0].as_mut().unwrap().number_format = id;
        let context = context(&styles, &fonts, &limits);
        let mut cell = cell();
        cell.value = CellValue::Number(1.5625);
        cell.numeric_lexeme = Some("1.5625".into());
        let mut warnings = Warnings::default();
        let texts = cell_text::plan(&cell, 1, frame(width), &context, &mut warnings).unwrap();
        let page = pdf::Page {
            width: 200.0,
            height: 200.0,
            items: cell_text::pdf_items(&texts),
        };
        let bytes = pdf::render(
            std::slice::from_ref(&page),
            &fonts,
            &Default::default(),
            &limits,
        )
        .unwrap();
        assert_eq!(
            bytes,
            pdf::render(&[page], &fonts, &Default::default(), &limits).unwrap()
        );
        let pdf = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(pdf.extract_text(&[1]).unwrap().trim(), expected);
        let summaries = warnings.into_summaries();
        assert_eq!(summaries.len(), usize::from(overflow));
        if overflow {
            assert_eq!(
                serde_json::to_value(summaries).unwrap()[0]["locations"][0]["cell"],
                "C5"
            );
        }
    }
}

#[test]
fn セルの双方向一行を抽出し未接続の書式境界と折返しを拒否する() {
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts.load_bytes(support::bidi_font(), &limits).unwrap();
    let mut styles = rich_styles("");
    styles.fonts[0].as_mut().unwrap().family = Some("方向試験".into());
    let cell = Cell {
        address: "C5".into(),
        style: 0,
        value: CellValue::Text("Aאב12B".into()),
        numeric_lexeme: None,
    };
    let texts = cell_text::plan(
        &cell,
        1,
        frame(200.0),
        &context(&styles, &fonts, &limits),
        &mut Warnings::default(),
    )
    .unwrap();
    let bytes = pdf::render(
        &[pdf::Page {
            width: 300.0,
            height: 100.0,
            items: cell_text::pdf_items(&texts),
        }],
        &fonts,
        &Default::default(),
        &limits,
    )
    .unwrap();
    assert_eq!(
        lopdf::Document::load_mem(&bytes)
            .unwrap()
            .extract_text(&[1])
            .unwrap()
            .trim(),
        "Aאב12B"
    );
    let rich = rich_cell("<r><t>A</t></r><r><t>אב</t></r>");
    let error = cell_text::plan(
        &rich,
        1,
        frame(200.0),
        &context(&styles, &fonts, &limits),
        &mut Warnings::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, "rich_text_bidi_layout");
    styles.cells[0].as_mut().unwrap().alignment.wrap = true;
    let mut wrapped_frame = frame(10.0);
    wrapped_frame.baseline_step = Some(12.0);
    let error = cell_text::plan(
        &cell,
        1,
        wrapped_frame,
        &context(&styles, &fonts, &limits),
        &mut Warnings::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, "wrapped_bidi_layout");
}
