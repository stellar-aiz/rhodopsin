#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::{
    ConversionOptions, PreparedWorkbook,
    fonts::{FontBook, FontLimits},
    grid::Grid,
    layout::LayoutLimits,
    print::PrintSettings,
    range::CellRange,
    styles::Styles,
    theme::TextScript,
    workbook::{Sheet, Workbook},
};
fn prepared() -> PreparedWorkbook {
    let doc = roxmltree::Document::parse(r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
      <fonts><font><name val="試験"/><sz val="10"/></font><font><name val="試験"/><sz val="20"/></font></fonts>
      <cellStyleXfs><xf fontId="0"/><xf fontId="1"/></cellStyleXfs>
      <cellStyles><cellStyle name="表示名" builtinId="0" xfId="1"/></cellStyles></styleSheet>"#).unwrap();
    let styles = Styles::parse(doc.root_element()).unwrap();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(
            support::font("試験", false, false, 0),
            &FontLimits::default(),
        )
        .unwrap();
    let mut sheets = Vec::new();
    for index in 1..=3 {
        let doc = roxmltree::Document::parse(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
          <sheetFormatPr defaultRowHeight="10" defaultColWidth="8"/>
          <pageMargins left="0.1" right="0.1" top="0.1" bottom="0.1" header="0" footer="0"/>
          <pageSetup paperSize="9" orientation="portrait" scale="50"/></worksheet>"#,
        )
        .unwrap();
        let mut print = PrintSettings::parse(doc.root_element()).unwrap();
        print.areas = vec![CellRange::parse("A1:C2").unwrap()];
        if index == 2 {
            print.paper_size = None;
        }
        sheets.push(Sheet {
            index,
            hidden: index == 2,
            cells: Vec::new(),
            grid: Grid::parse(doc.root_element()).unwrap(),
            print,
            pictures: Vec::new(),
        });
    }
    PreparedWorkbook {
        workbook: Workbook {
            sheets,
            date_1904: false,
            styles: Some(styles),
            theme: None,
        },
        fonts,
    }
}

#[test]
fn 標準書体の数字幅で可視シートを順番に分割する() {
    let prepared = prepared();
    let result = prepared
        .paginate(
            &ConversionOptions::default(),
            TextScript::Latin,
            &LayoutLimits::default(),
        )
        .unwrap();
    assert_eq!(
        result
            .pages
            .iter()
            .map(|p| p.sheet_index)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    // Excel は数字最大幅を整数ピクセルへ丸める。
    let expected =
        (prepared.fonts.faces[0].maximum_digit_width(20.0).unwrap() * 96.0 / 72.0).round();
    assert_eq!(result.column_metrics.maximum_digit_width_pixels, expected);
    assert!(!result.column_metrics.substituted);
    assert_eq!(result.pages[0].scale, 0.5);
    assert_eq!(
        result.pages[0].columns[0].size,
        rhodopsin::xlsx::layout::column_points(8.0, expected).unwrap()
    );
    assert_eq!(result.pages[1].body, CellRange::parse("A1:C2").unwrap());
}

#[test]
fn ページと配置数の上限はシートごとにリセットしない() {
    for (pages, placements, reason) in [(1, 100, "page_limit"), (10, 9, "layout_placement_limit")] {
        let error = prepared()
            .paginate(
                &ConversionOptions::default(),
                TextScript::Latin,
                &LayoutLimits {
                    pages,
                    placements,
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(error.reason, reason);
        assert_eq!(error.location.unwrap().sheet_index, 3);
    }
    assert!(
        prepared()
            .paginate(
                &ConversionOptions::default(),
                TextScript::Latin,
                &LayoutLimits {
                    pages: 2,
                    placements: 10,
                    ..Default::default()
                }
            )
            .is_ok()
    );
}

#[test]
fn 標準書体の代用を黙って列幅へ適用しない() {
    let mut prepared = prepared();
    prepared.workbook.styles.as_mut().unwrap().fonts[1]
        .as_mut()
        .unwrap()
        .family = Some("不足書体".into());
    let error = prepared
        .paginate(
            &ConversionOptions::default(),
            TextScript::Latin,
            &LayoutLimits::default(),
        )
        .unwrap_err();
    assert_eq!(error.reason, "font_family_missing");
    assert_eq!(error.location.unwrap().sheet_index, 1);
    let result = prepared
        .paginate(
            &ConversionOptions {
                fallback_family: Some("試験".into()),
                ..Default::default()
            },
            TextScript::Latin,
            &LayoutLimits::default(),
        )
        .unwrap();
    assert!(result.column_metrics.substituted);
    assert_eq!(result.column_metrics.font_index, 0);
}

#[test]
fn 標準スタイル不明と可視シートなしを拒否する() {
    let mut prepared = prepared();
    prepared.workbook.styles.as_mut().unwrap().normal_font = None;
    assert_eq!(
        prepared
            .paginate(
                &ConversionOptions::default(),
                TextScript::Latin,
                &LayoutLimits::default()
            )
            .unwrap_err()
            .reason,
        "normal_style_font_unresolved"
    );
    for sheet in &mut prepared.workbook.sheets {
        sheet.hidden = true;
    }
    assert_eq!(
        prepared
            .paginate(
                &ConversionOptions::default(),
                TextScript::Latin,
                &LayoutLimits::default()
            )
            .unwrap_err()
            .reason,
        "no_visible_sheets"
    );
}

fn render_fixture() -> PreparedWorkbook {
    use rhodopsin::xlsx::{
        drawing::{Anchor, Marker, Picture, Raster},
        workbook::{Cell, CellValue},
    };
    let mut prepared = prepared();
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font><name val="試験"/><sz val="10"/><color rgb="FF000000"/></font></fonts><fills><fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/></patternFill></fill></fills><borders><border/></borders><cellStyleXfs><xf/></cellStyleXfs><cellStyles><cellStyle builtinId="0" xfId="0"/></cellStyles><cellXfs><xf/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    prepared.workbook.styles = Some(Styles::parse(doc.root_element()).unwrap());
    for sheet in &mut prepared.workbook.sheets {
        sheet.cells.push(Cell {
            address: "A1".into(),
            style: 0,
            value: CellValue::Text(if sheet.index == 1 { "AB" } else { "CD" }.into()),
            numeric_lexeme: None,
        });
    }
    prepared.workbook.sheets[0].pictures.push(Picture {
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
        raster: std::sync::Arc::new(Raster {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 255, 255],
        }),
        print: true,
        hidden: false,
    });
    prepared
}
fn environment() -> rhodopsin::xlsx::render::Environment<'static> {
    rhodopsin::xlsx::render::Environment {
        locale: rhodopsin::xlsx::format::date::Locale::Japanese,
        script: TextScript::Latin,
        normal_script: TextScript::Latin,
        automatic_color: None,
        foreground: None,
        background: None,
    }
}

#[test]
fn 全可視シートを画像と文字を含む単一pdfへ接続する() {
    let prepared = render_fixture();
    let render = || {
        prepared
            .render_with_policy(
                &ConversionOptions::default(),
                &environment(),
                &Default::default(),
                |_, page, _, placement| {
                    Ok(rhodopsin::xlsx::cell_text::Frame {
                        content: placement.rect,
                        clip: placement.rect,
                        scale: page.scale,
                        baseline_step: None,
                        line_box: None,
                    })
                },
            )
            .unwrap()
    };
    let result = render();
    assert_eq!(result.bytes, render().bytes);
    assert_eq!(
        result
            .layout
            .pages
            .iter()
            .map(|page| page.sheet_index)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    let pdf = lopdf::Document::load_mem(&result.bytes).unwrap();
    assert_eq!(pdf.get_pages().len(), 2);
    assert_eq!(pdf.extract_text(&[1]).unwrap().trim(), "AB");
    assert_eq!(pdf.extract_text(&[2]).unwrap().trim(), "CD");
    let operators =
        lopdf::content::Content::decode(&pdf.get_page_content(pdf.get_pages()[&1]).unwrap())
            .unwrap();
    assert!(operators.operations.iter().any(|op| op.operator == "Do"));
    assert!(result.report.warnings.is_empty());
    assert_eq!(result.report.schema_version, 1);
    assert_eq!(result.report.page_count, pdf.get_pages().len());
    assert_eq!(
        result
            .report
            .pages
            .iter()
            .map(|page| page.sheet_index)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    for (index, page) in result.report.pages.iter().enumerate() {
        assert_eq!(page.page_index, index + 1);
        assert_eq!(page.body.first_cell, "A1");
        assert_eq!(page.body.last_cell, "C2");
        assert!(page.width_points > 0.0 && page.height_points > 0.0);
    }
    assert!(!result.report.fonts.is_empty());
    assert!(
        result
            .report
            .fonts
            .iter()
            .all(|font| font.embedded && font.subset && font.to_unicode)
    );
    let report = serde_json::to_string(&result.report).unwrap();
    assert!(!report.contains("AB"));
    assert!(!report.contains("CD"));
}

#[test]
fn ブック全体の描画量を制限し後続ページの失敗を返す() {
    let prepared = render_fixture();
    let mut limits = rhodopsin::xlsx::render::Limits::default();
    limits.scene.text_bytes = 3; // 各ページ 2 バイト。単独では収まるが合計では超える。
    let result = prepared.render_with_policy(
        &ConversionOptions::default(),
        &environment(),
        &limits,
        |_, page, _, placement| {
            Ok(rhodopsin::xlsx::cell_text::Frame {
                content: placement.rect,
                clip: placement.rect,
                scale: page.scale,
                baseline_step: None,
                line_box: None,
            })
        },
    );
    assert_eq!(result.err().unwrap().reason, "page_scene_text_limit");
    let mut visited = Vec::new();
    let result = prepared.render_with_policy(
        &ConversionOptions::default(),
        &environment(),
        &Default::default(),
        |sheet, page, _, placement| {
            visited.push(sheet.index);
            if sheet.index == 3 {
                return Err(rhodopsin::xlsx::diagnostic::Failure::unsupported(
                    "test_layout_failure",
                ));
            }
            Ok(rhodopsin::xlsx::cell_text::Frame {
                content: placement.rect,
                clip: placement.rect,
                scale: page.scale,
                baseline_step: None,
                line_box: None,
            })
        },
    );
    assert_eq!(visited, [1, 3]);
    let error = result.err().unwrap();
    assert_eq!(error.reason, "test_layout_failure");
    assert_eq!(error.location.unwrap().sheet_index, 3);
}

#[test]
fn 成功レポートの書体を実際のpdf埋め込みと照合する() {
    let mut prepared = render_fixture();
    prepared
        .fonts
        .load_bytes(
            support::font("未使用の書体", false, false, 0),
            &FontLimits::default(),
        )
        .unwrap();
    let result = prepared
        .render_with_policy(
            &ConversionOptions::default(),
            &environment(),
            &Default::default(),
            |_, page, _, placement| {
                Ok(rhodopsin::xlsx::cell_text::Frame {
                    content: placement.rect,
                    clip: placement.rect,
                    scale: page.scale,
                    baseline_step: None,
                    line_box: None,
                })
            },
        )
        .unwrap();
    assert_eq!(result.report.fonts.len(), 1);
    assert_eq!(result.report.fonts[0].source_index, 0);
    assert!(
        !serde_json::to_string(&result.report)
            .unwrap()
            .contains("未使用の書体")
    );
    let doc = lopdf::Document::load_mem(&result.bytes).unwrap();
    let pdf_fonts: Vec<_> = doc
        .objects
        .values()
        .filter_map(|object| object.as_dict().ok())
        .filter(|dict| {
            dict.get(b"Subtype")
                .and_then(lopdf::Object::as_name)
                .is_ok_and(|name| name == b"Type0")
        })
        .collect();
    assert_eq!(pdf_fonts.len(), result.report.fonts.len());
    for font in &result.report.fonts {
        let expected = format!("AAAAAA+Font{}", font.source_index);
        let dictionary = pdf_fonts
            .iter()
            .find(|dict| dict.get(b"BaseFont").unwrap().as_name().unwrap() == expected.as_bytes())
            .unwrap();
        let cmap = doc
            .get_object(
                dictionary
                    .get(b"ToUnicode")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
            )
            .unwrap()
            .as_stream()
            .unwrap();
        assert_eq!(font.to_unicode, !cmap.content.is_empty());
        let cid = doc
            .get_object(
                dictionary
                    .get(b"DescendantFonts")
                    .unwrap()
                    .as_array()
                    .unwrap()[0]
                    .as_reference()
                    .unwrap(),
            )
            .unwrap()
            .as_dict()
            .unwrap();
        let descriptor = doc
            .get_object(cid.get(b"FontDescriptor").unwrap().as_reference().unwrap())
            .unwrap()
            .as_dict()
            .unwrap();
        let stream = doc
            .get_object(
                descriptor
                    .get(b"FontFile2")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
            )
            .unwrap()
            .as_stream()
            .unwrap();
        assert_eq!(font.embedded, !stream.content.is_empty());
        let embedded = rustybuzz::ttf_parser::Face::parse(&stream.content, 0).unwrap();
        let original =
            rustybuzz::ttf_parser::Face::parse(prepared.fonts.faces[font.source_index].data(), 0)
                .unwrap();
        assert!(font.subset);
        assert!(embedded.number_of_glyphs() < original.number_of_glyphs());
        assert_eq!(embedded.number_of_glyphs(), 5); // .notdef と A〜D。
    }
    for (index, id) in doc.get_pages() {
        let media = doc
            .get_dictionary(id)
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap();
        let page = &result.report.pages[index as usize - 1];
        assert_eq!(media[2].as_float().unwrap(), page.width_points as f32);
        assert_eq!(media[3].as_float().unwrap(), page.height_points as f32);
    }
}

#[test]
fn 描画しない標準書体を埋め込み済みとして報告しない() {
    let mut prepared = render_fixture();
    for sheet in &mut prepared.workbook.sheets {
        sheet.cells.clear();
    }
    let result = prepared
        .render_with_policy(
            &ConversionOptions::default(),
            &environment(),
            &Default::default(),
            |_, _, _, _| panic!("文字セルなし"),
        )
        .unwrap();
    assert!(result.report.fonts.is_empty());
    let doc = lopdf::Document::load_mem(&result.bytes).unwrap();
    assert!(
        !doc.objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .any(|dict| dict.has(b"FontFile2") || dict.has(b"ToUnicode"))
    );
    assert_eq!(result.report.page_count, doc.get_pages().len());
}

#[test]
fn pdf層の小さい文字量上限もページ組版へ先に渡す() {
    let prepared = render_fixture();
    let mut limits = rhodopsin::xlsx::render::Limits::default();
    limits.pdf.text_bytes = 1;
    let mut visited = Vec::new();
    let result = prepared.render_with_policy(
        &ConversionOptions::default(),
        &environment(),
        &limits,
        |sheet, page, _, placement| {
            visited.push(sheet.index);
            Ok(rhodopsin::xlsx::cell_text::Frame {
                content: placement.rect,
                clip: placement.rect,
                scale: page.scale,
                baseline_step: None,
                line_box: None,
            })
        },
    );
    assert_eq!(result.err().unwrap().reason, "page_scene_text_limit");
    assert_eq!(visited, [1]);
}

#[test]
fn ブック組版は列幅と字下げへ同じ標準文字体系を渡す() {
    let mut prepared = render_fixture();
    let styles = prepared.workbook.styles.as_mut().unwrap();
    let mut normal = styles.fonts[0].as_ref().unwrap().clone();
    normal.scheme = rhodopsin::xlsx::styles::FontScheme::Minor;
    styles.fonts.push(Ok(normal));
    styles.normal_font = Some(1);
    let alignment = &mut styles.cells[0].as_mut().unwrap().alignment;
    alignment.indent = 1;
    alignment.horizontal = rhodopsin::xlsx::styles::Horizontal::Left;
    prepared.workbook.theme = Some(rhodopsin::xlsx::theme::Theme {
        colors: [[0; 3]; 12],
        major: Default::default(),
        minor: rhodopsin::xlsx::theme::ThemeFonts {
            latin: "試験".into(),
            ..Default::default()
        },
    });
    let mut environment = environment();
    environment.script = TextScript::EastAsian("Jpan");
    // 本文の書体は明示名。標準書体はテーマ欧文を指定。和文テーマは意図的に未定義。
    let result = prepared
        .render_with_policy(
            &ConversionOptions::default(),
            &environment,
            &Default::default(),
            |_, page, _, placement| {
                Ok(rhodopsin::xlsx::cell_text::Frame {
                    content: placement.rect,
                    clip: placement.rect,
                    scale: page.scale,
                    baseline_step: None,
                    line_box: None,
                })
            },
        )
        .unwrap();
    let doc = lopdf::Document::load_mem(&result.bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "AB");
    assert_eq!(doc.extract_text(&[2]).unwrap().trim(), "CD");
}

#[test]
fn ブック組版では隣接セルに応じた文字の可視範囲をpdfへ渡す() {
    use rhodopsin::xlsx::workbook::{Cell, CellValue};
    let mut prepared = render_fixture();
    prepared.workbook.sheets[0].pictures.clear();
    prepared.workbook.sheets[0].cells[0].value =
        CellValue::Text("ABCDEFGHIJKLMNOPQRSTUVWXYZ".into());
    let render = |prepared: &PreparedWorkbook| {
        prepared
            .render_with_cell_overflow(
                &ConversionOptions::default(),
                &environment(),
                &Default::default(),
                |_, page, _, placement| {
                    Ok(rhodopsin::xlsx::cell_text::Frame {
                        content: placement.rect,
                        clip: placement.rect,
                        scale: page.scale,
                        baseline_step: None,
                        line_box: None,
                    })
                },
            )
            .unwrap()
    };
    let inspect = |bytes: &[u8]| {
        let pdf = lopdf::Document::load_mem(bytes).unwrap();
        let content =
            lopdf::content::Content::decode(&pdf.get_page_content(pdf.get_pages()[&1]).unwrap())
                .unwrap();
        let clip = content
            .operations
            .windows(2)
            .find(|ops| ops[0].operator == "re" && ops[1].operator == "W")
            .unwrap()[0]
            .operands
            .clone();
        let matrix = content
            .operations
            .iter()
            .find(|op| op.operator == "Tm")
            .unwrap()
            .operands
            .clone();
        assert_eq!(
            pdf.extract_text(&[1]).unwrap().trim(),
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
        );
        (clip, matrix)
    };
    let expanded = render(&prepared);
    assert_eq!(expanded.bytes, render(&prepared).bytes);
    let (expanded_clip, expanded_matrix) = inspect(&expanded.bytes);
    let page = &expanded.layout.pages[0];
    let column_width = page.columns[0].size * page.scale;
    assert_eq!(
        expanded_clip[2].as_float().unwrap(),
        (column_width * 3.0) as f32
    );
    prepared.workbook.sheets[0].cells.push(Cell {
        address: "B1".into(),
        style: 0,
        value: CellValue::Text("".into()),
        numeric_lexeme: None,
    });
    let clipped = render(&prepared);
    let (clip, matrix) = inspect(&clipped.bytes);
    assert_eq!(clip[2].as_float().unwrap(), column_width as f32);
    assert_eq!(matrix, expanded_matrix);
    // 第二可視シートに第一シートの占有状況を持ち越さない。
    let pdf = lopdf::Document::load_mem(&clipped.bytes).unwrap();
    let content =
        lopdf::content::Content::decode(&pdf.get_page_content(pdf.get_pages()[&2]).unwrap())
            .unwrap();
    let clip = &content
        .operations
        .windows(2)
        .find(|ops| ops[0].operator == "re" && ops[1].operator == "W")
        .unwrap()[0];
    assert_eq!(
        clip.operands[2].as_float().unwrap(),
        (column_width * 3.0) as f32
    );
}

#[test]
fn ブック描画が複数書式の抽出と全使用書体のレポートを保持する() {
    let mut prepared = render_fixture();
    prepared
        .fonts
        .load_bytes(
            support::font("追加書体", true, false, 0),
            &FontLimits::default(),
        )
        .unwrap();
    let xml = r#"<si xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><r><t>A</t></r><r><rPr><rFont val="追加書体"/><sz val="16"/><b/><color rgb="FF0000FF"/></rPr><t>B</t></r></si>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    prepared.workbook.sheets[0].cells[0].value =
        rhodopsin::xlsx::rich_text::parse(doc.root_element()).unwrap();
    let render = || {
        prepared
            .render_with_policy(
                &ConversionOptions::default(),
                &environment(),
                &Default::default(),
                |_, page, _, placement| {
                    Ok(rhodopsin::xlsx::cell_text::Frame {
                        content: placement.rect,
                        clip: placement.rect,
                        scale: page.scale,
                        baseline_step: None,
                        line_box: None,
                    })
                },
            )
            .unwrap()
    };
    let result = render();
    assert_eq!(result.bytes, render().bytes);
    let pdf = lopdf::Document::load_mem(&result.bytes).unwrap();
    assert_eq!(pdf.extract_text(&[1]).unwrap(), "AB\n");
    assert_eq!(pdf.extract_text(&[2]).unwrap(), "CD\n");
    assert_eq!(
        result
            .report
            .fonts
            .iter()
            .map(|font| font.source_index)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(
        result
            .report
            .fonts
            .iter()
            .all(|font| font.embedded && font.subset && font.to_unicode)
    );
    let (resources, _) = pdf.get_page_resources(pdf.get_pages()[&1]).unwrap();
    assert_eq!(
        resources
            .unwrap()
            .get(b"Font")
            .unwrap()
            .as_dict()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn 複数書式と標準数値と小数画像切り取りを同じレポートへ接続する() {
    use rhodopsin::xlsx::workbook::{Cell, CellValue};
    let mut prepared = render_fixture();
    let document = roxmltree::Document::parse(r#"<si xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><r><t>A</t></r><r><rPr><rFont val="試験"/><sz val="12"/><color rgb="FF0000FF"/></rPr><t>B</t></r></si>"#).unwrap();
    let sheet = &mut prepared.workbook.sheets[0];
    sheet.cells[0].value = rhodopsin::xlsx::rich_text::parse(document.root_element()).unwrap();
    for (address, raw) in [("B1", "123.4500"), ("C1", "123456789012")] {
        sheet.cells.push(Cell {
            address: address.into(),
            style: 0,
            value: CellValue::Number(raw.parse().unwrap()),
            numeric_lexeme: Some(raw.into()),
        });
    }
    sheet.pictures[0].crop = rhodopsin::xlsx::drawing::SourceCrop {
        left: 12500.5,
        right: 37499.5,
        top: 0.0,
        bottom: 0.0,
    };
    let render = |text_bytes| {
        let mut limits = rhodopsin::xlsx::render::Limits::default();
        limits.scene.text_bytes = text_bytes;
        prepared.render_with_policy(
            &ConversionOptions::default(),
            &environment(),
            &limits,
            |_, page, cell, placement| {
                let mut content = placement.rect;
                if cell.address == "C1" {
                    content.width = 9.0;
                }
                Ok(rhodopsin::xlsx::cell_text::Frame {
                    content,
                    clip: placement.rect,
                    scale: page.scale,
                    baseline_step: None,
                    line_box: None,
                })
            },
        )
    };
    let result = render(13).unwrap();
    assert_eq!(result.bytes, render(13).unwrap().bytes);
    let pdf = lopdf::Document::load_mem(&result.bytes).unwrap();
    assert_eq!(pdf.extract_text(&[1]).unwrap(), "AB\n123.45\n###\n");
    assert_eq!(pdf.extract_text(&[2]).unwrap(), "CD\n");
    let content =
        lopdf::content::Content::decode(&pdf.get_page_content(pdf.get_pages()[&1]).unwrap())
            .unwrap();
    let transform = content
        .operations
        .iter()
        .find(|op| op.operator == "cm")
        .unwrap();
    assert!((f64::from(transform.operands[0].as_float().unwrap()) - 10.0).abs() < 1e-5);
    assert!((f64::from(transform.operands[4].as_float().unwrap()) - 5.94995).abs() < 1e-5);
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|op| op.operator == "Do")
            .count(),
        1
    );
    assert_eq!(result.report.page_count, 2);
    assert!(
        result
            .report
            .fonts
            .iter()
            .all(|font| font.embedded && font.subset && font.to_unicode)
    );
    let warnings = serde_json::to_value(&result.report.warnings).unwrap();
    assert_eq!(warnings.as_array().unwrap().len(), 1);
    assert_eq!(warnings[0]["kind"], "numeric_overflow");
    assert_eq!(warnings[0]["count"], 1);
    assert_eq!(warnings[0]["locations"][0]["cell"], "C1");
    assert_eq!(warnings[0]["locations"][0]["sheet_index"], 1);
    assert!(render(12).is_err());
}

#[test]
fn 全ページの罫線を明示規則で描画しpdfの幅と破線を照合する() {
    use rhodopsin::xlsx::{
        page_borders::Stroke,
        styles::{BorderLine, Color, ColorSource, LineStyle},
    };
    let mut prepared = render_fixture();
    prepared.workbook.styles.as_mut().unwrap().borders[0]
        .as_mut()
        .unwrap()
        .left = Some(BorderLine {
        style: LineStyle::Dashed,
        color: Color {
            source: ColorSource::Rgb([0, 0, 0]),
            tint: 0.0,
        },
    });
    let render = |limits: &rhodopsin::xlsx::render::Limits| {
        prepared.render_with_borders(
            &ConversionOptions::default(),
            &environment(),
            limits,
            |_, page, _, placement| {
                Ok(rhodopsin::xlsx::cell_text::Frame {
                    content: placement.rect,
                    clip: placement.rect,
                    scale: page.scale,
                    baseline_step: None,
                    line_box: None,
                })
            },
            |_, page, edge| {
                assert_eq!(edge.style, LineStyle::Dashed);
                Ok(Stroke {
                    width: page.scale,
                    dash: vec![4.0 * page.scale, 2.0 * page.scale],
                    separation: None,
                })
            },
        )
    };
    let first = render(&Default::default()).unwrap();
    assert_eq!(first.bytes, render(&Default::default()).unwrap().bytes);
    let doc = lopdf::Document::load_mem(&first.bytes).unwrap();
    assert_eq!(doc.get_pages().len(), 2);
    for (number, id) in doc.get_pages() {
        let content = lopdf::content::Content::decode(&doc.get_page_content(id).unwrap()).unwrap();
        let strokes: Vec<_> = content
            .operations
            .iter()
            .filter(|op| op.operator == "S")
            .collect();
        assert_eq!(strokes.len(), 6);
        let widths: Vec<_> = content
            .operations
            .iter()
            .filter(|op| op.operator == "w")
            .collect();
        assert_eq!(widths.len(), 6);
        for op in widths {
            assert_eq!(op.operands[0].as_float().unwrap(), 0.5);
        }
        for op in content.operations.iter().filter(|op| op.operator == "d") {
            let values = op.operands[0].as_array().unwrap();
            assert_eq!(
                values
                    .iter()
                    .map(|n| n.as_float().unwrap())
                    .collect::<Vec<_>>(),
                [2.0, 1.0]
            );
        }
        let operators: Vec<_> = content
            .operations
            .iter()
            .map(|op| op.operator.as_str())
            .collect();
        assert!(
            operators.iter().position(|op| *op == "f").unwrap()
                < operators.iter().position(|op| *op == "S").unwrap()
        );
        assert!(
            operators.iter().rposition(|op| *op == "S").unwrap()
                < operators.iter().position(|op| *op == "BT").unwrap()
        );
        assert_eq!(
            doc.extract_text(&[number]).unwrap().trim(),
            if number == 1 { "AB" } else { "CD" }
        );
    }
    let mut limits = rhodopsin::xlsx::render::Limits::default();
    limits.scene.items = 20; // 一ページ分は収まるが二ページ合計では超える。
    assert_eq!(
        render(&limits).err().unwrap().reason,
        "page_scene_item_limit"
    );
}
