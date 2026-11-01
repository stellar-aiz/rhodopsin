#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::{
    fonts::{FontBook, FontLimits},
    format::{DisplayValue, Fragment, NumberFormat},
    text_layout::measure,
};
fn fonts() -> FontBook {
    let mut book = FontBook::default();
    book.load_bytes(
        support::font("計測", false, false, 0),
        &FontLimits::default(),
    )
    .unwrap();
    book
}
#[test]
fn 会計形式の空白と埋め文字を実幅で配置する() {
    let book = fonts();
    let display = NumberFormat::parse("_($* 0.00_)")
        .unwrap()
        .number("12.3")
        .unwrap();
    let line = measure(
        &display,
        &book.faces[0],
        10.0,
        100.0,
        &FontLimits::default(),
    )
    .unwrap();
    assert!(!line.overflow);
    assert_eq!(line.runs[0].text, "$");
    assert_eq!(line.runs[0].x, 6.0);
    assert_eq!(line.runs[1].text, " ".repeat(9));
    assert_eq!(line.runs[2].text, "12.30");
    assert_eq!(line.runs[2].x, 66.0);
    assert!((line.advance - 98.6).abs() < 1e-10);
}
#[test]
fn 幅不足でも文字を削らず超過を返す() {
    let book = fonts();
    let display = NumberFormat::parse("0.00").unwrap().number("123").unwrap();
    let line = measure(&display, &book.faces[0], 10.0, 1.0, &FontLimits::default()).unwrap();
    assert!(line.overflow);
    assert_eq!(line.runs[0].text, "123.00");
}
#[test]
fn 埋め文字の展開と計測に上限を適用する() {
    let book = fonts();
    let display = DisplayValue {
        fragments: vec![Fragment::Fill(' ')],
        color: None,
    };
    let limits = FontLimits {
        run_bytes: 10,
        ..FontLimits::default()
    };
    assert_eq!(
        measure(&display, &book.faces[0], 10.0, 1000.0, &limits)
            .unwrap_err()
            .reason,
        "text_fill_limit"
    );
    assert!(measure(&display, &book.faces[0], 10.0, f64::NAN, &limits).is_err());
    let display = DisplayValue {
        fragments: vec![Fragment::Space('未')],
        color: None,
    };
    assert_eq!(
        measure(&display, &book.faces[0], 10.0, 100.0, &limits)
            .unwrap_err()
            .reason,
        "font_glyph_missing"
    );
}

#[test]
fn 計測結果をpdfの文字位置へ渡せる() {
    use rhodopsin::xlsx::pdf::{self, Item, Page, Text};
    use rhodopsin::xlsx::{
        pdf::graphics::BoxRect,
        styles::{Horizontal, Vertical},
        text_layout::position_line,
    };
    let book = fonts();
    let display = NumberFormat::parse("_($* 0.00_)")
        .unwrap()
        .number("12.3")
        .unwrap();
    let line = measure(
        &display,
        &book.faces[0],
        10.0,
        100.0,
        &FontLimits::default(),
    )
    .unwrap();
    let position = position_line(
        &line,
        BoxRect {
            x: 20.0,
            y: 12.0,
            width: 100.0,
            height: 20.0,
        },
        Horizontal::Left,
        Vertical::Bottom,
        line.runs[0].shaped.ascender,
        line.runs[0].shaped.descender,
    )
    .unwrap();
    let items = line
        .runs
        .iter()
        .map(|run| {
            Item::Text(Text {
                text: &run.text,
                font: 0,
                points: 10.0,
                x: position.x + run.x,
                baseline: position.baseline,
                color: [0; 3],
                clip: None,
            })
        })
        .collect();
    let bytes = pdf::render(
        &[Page {
            width: 200.0,
            height: 100.0,
            items,
        }],
        &book,
        &pdf::Limits::default(),
        &FontLimits::default(),
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let page_id = doc.get_pages()[&1];
    let content = lopdf::content::Content::decode(&doc.get_page_content(page_id).unwrap()).unwrap();
    let matrices: Vec<_> = content
        .operations
        .iter()
        .filter(|op| op.operator == "Tm")
        .collect();
    // 非描画の括弧幅を含む x と、上端原点から変換した PDF の y を確認する。
    assert_eq!(matrices[0].operands[4].as_float().unwrap(), 26.0);
    assert!(
        matrices
            .iter()
            .all(|m| m.operands[5].as_float().unwrap() == 70.0)
    );
    let extracted = doc.extract_text(&[1]).unwrap();
    assert!(extracted.contains('$'));
    assert!(extracted.contains("12.30"));
    assert!(!extracted.contains('('));
    assert!(!extracted.contains(')'));
}

#[test]
fn 一行の左右中央と上下中央を内容矩形へ配置する() {
    use rhodopsin::xlsx::{
        pdf::graphics::BoxRect,
        styles::{Horizontal, Vertical},
        text_layout::position_line,
    };
    let book = fonts();
    let display = DisplayValue {
        fragments: vec![Fragment::Text("AB".into())],
        color: None,
    };
    let line = measure(&display, &book.faces[0], 10.0, 50.0, &FontLimits::default()).unwrap();
    let content = BoxRect {
        x: 10.0,
        y: 20.0,
        width: 50.0,
        height: 30.0,
    };
    let run = &line.runs[0].shaped;
    for (horizontal, x) in [
        (Horizontal::Left, 10.0),
        (Horizontal::Center, 29.0),
        (Horizontal::Right, 48.0),
    ] {
        for (vertical, baseline) in [
            (Vertical::Top, 28.0),
            (Vertical::Center, 38.0),
            (Vertical::Bottom, 48.0),
        ] {
            let position = position_line(
                &line,
                content,
                horizontal,
                vertical,
                run.ascender,
                run.descender,
            )
            .unwrap();
            assert_eq!((position.x, position.baseline), (x, baseline));
        }
    }
    // 超過の切り取りを配置層で行わず、右端を保存する。
    let narrow = BoxRect {
        width: 2.0,
        ..content
    };
    let position = position_line(
        &line,
        narrow,
        Horizontal::Right,
        Vertical::Bottom,
        8.0,
        -2.0,
    )
    .unwrap();
    assert_eq!(position.x, 0.0);
}

#[test]
fn 標準配置をキャッシュの型で解決する() {
    use rhodopsin::xlsx::{
        styles::Horizontal, text_layout::horizontal_for_value, workbook::CellValue,
    };
    for (value, expected) in [
        (CellValue::Blank, Horizontal::Left),
        (CellValue::Text("123".into()), Horizontal::Left),
        (CellValue::Number(123.0), Horizontal::Right),
        (CellValue::Boolean(true), Horizontal::Center),
        (CellValue::Error("#N/A".into()), Horizontal::Center),
    ] {
        assert_eq!(horizontal_for_value(Horizontal::General, &value), expected);
        assert_eq!(
            horizontal_for_value(Horizontal::Left, &value),
            Horizontal::Left
        );
    }
}

#[test]
fn 一行配置の不正寸法と未解決整列を拒否する() {
    use rhodopsin::xlsx::{
        pdf::graphics::BoxRect,
        styles::{Horizontal, Vertical},
        text_layout::{Line, position_line},
    };
    let line = Line {
        runs: Vec::new(),
        advance: 0.0,
        overflow: false,
    };
    let content = BoxRect {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    };
    for horizontal in [
        Horizontal::General,
        Horizontal::Fill,
        Horizontal::Justify,
        Horizontal::CenterContinuous,
        Horizontal::Distributed,
    ] {
        assert!(position_line(&line, content, horizontal, Vertical::Bottom, 8.0, -2.0).is_err());
    }
    for vertical in [Vertical::Justify, Vertical::Distributed] {
        assert!(position_line(&line, content, Horizontal::Left, vertical, 8.0, -2.0).is_err());
    }
    for (ascender, descender) in [(f64::NAN, -2.0), (8.0, 2.0), (0.0, 0.0), (-8.0, -2.0)] {
        assert!(
            position_line(
                &line,
                content,
                Horizontal::Left,
                Vertical::Bottom,
                ascender,
                descender
            )
            .is_err()
        );
    }
    for width in [-1.0, f64::INFINITY] {
        assert!(
            position_line(
                &line,
                BoxRect { width, ..content },
                Horizontal::Left,
                Vertical::Bottom,
                8.0,
                -2.0
            )
            .is_err()
        );
    }
}

#[test]
fn 明示改行と空行を保持して複数行を配置する() {
    use rhodopsin::xlsx::{
        pdf::graphics::BoxRect,
        styles::{Horizontal, Vertical},
        text_layout::{measure_lines, position_lines},
    };
    let book = fonts();
    let lines = measure_lines(
        "AB\n\nA\n",
        &book.faces[0],
        10.0,
        50.0,
        &FontLimits::default(),
    )
    .unwrap();
    assert_eq!(lines.len(), 4);
    assert_eq!(
        lines.iter().map(|l| l.advance).collect::<Vec<_>>(),
        [12.0, 0.0, 6.0, 0.0]
    );
    let content = BoxRect {
        x: 10.0,
        y: 20.0,
        width: 50.0,
        height: 60.0,
    };
    for (vertical, first) in [
        (Vertical::Top, 28.0),
        (Vertical::Center, 35.0),
        (Vertical::Bottom, 42.0),
    ] {
        let positions = position_lines(
            &lines,
            content,
            Horizontal::Right,
            vertical,
            8.0,
            -2.0,
            12.0,
        )
        .unwrap();
        assert_eq!(
            positions.iter().map(|p| p.x).collect::<Vec<_>>(),
            [48.0, 60.0, 54.0, 60.0]
        );
        assert_eq!(
            positions.iter().map(|p| p.baseline).collect::<Vec<_>>(),
            [first, first + 12.0, first + 24.0, first + 36.0]
        );
    }
}

#[test]
fn 改行による上限回避と不正な複数行寸法を拒否する() {
    use rhodopsin::xlsx::{
        pdf::graphics::BoxRect,
        styles::{Horizontal, Vertical},
        text_layout::{measure_lines, position_lines},
    };
    let book = fonts();
    let limits = FontLimits {
        run_bytes: 4,
        ..FontLimits::default()
    };
    assert_eq!(
        measure_lines("AB\nAB", &book.faces[0], 10.0, 50.0, &limits)
            .unwrap_err()
            .reason,
        "text_run_limit"
    );
    assert_eq!(
        measure_lines(
            &"\n".repeat(4096),
            &book.faces[0],
            10.0,
            50.0,
            &FontLimits::default()
        )
        .unwrap_err()
        .reason,
        "text_line_limit"
    );
    assert!(measure_lines("\n", &book.faces[0], f64::NAN, 50.0, &limits).is_err());
    assert!(measure_lines("A\r\n", &book.faces[0], 10.0, 50.0, &limits).is_err());
    let lines = measure_lines("A\nB", &book.faces[0], 10.0, 50.0, &limits).unwrap();
    let content = BoxRect {
        x: 0.0,
        y: 0.0,
        width: 50.0,
        height: 10.0,
    };
    for spacing in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        assert!(
            position_lines(
                &lines,
                content,
                Horizontal::Left,
                Vertical::Bottom,
                8.0,
                -2.0,
                spacing
            )
            .is_err()
        );
    }
    assert!(
        position_lines(
            &lines,
            content,
            Horizontal::Left,
            Vertical::Bottom,
            8.0,
            2.0,
            12.0
        )
        .is_err()
    );
    assert!(
        position_lines(
            &[],
            content,
            Horizontal::Left,
            Vertical::Bottom,
            8.0,
            -2.0,
            12.0
        )
        .is_err()
    );
    // 高さ不足も切り捨てず、最終行の下端を保った配置を返す。
    let positions = position_lines(
        &lines,
        content,
        Horizontal::Left,
        Vertical::Bottom,
        8.0,
        -2.0,
        12.0,
    )
    .unwrap();
    assert_eq!(positions[0].baseline, -4.0);
    assert_eq!(positions[1].baseline, 8.0);
}

#[test]
fn 数値だけを実幅に収まるハッシュへ置換する() {
    use rhodopsin::xlsx::text_layout::measure_fixed_number;
    let book = fonts();
    let limits = FontLimits::default();
    let display = NumberFormat::parse("0.00")
        .unwrap()
        .number("123.45")
        .unwrap();
    let full = measure(&display, &book.faces[0], 10.0, 100.0, &limits).unwrap();
    let exact =
        measure_fixed_number(&display, &book.faces[0], 10.0, full.advance, &limits).unwrap();
    assert!(!exact.hashes);
    assert_eq!(exact.line.runs[0].text, "123.45");
    for (width, hashes) in [(18.0, "###"), (17.9, "##"), (6.0, "#")] {
        let result = measure_fixed_number(&display, &book.faces[0], 10.0, width, &limits).unwrap();
        assert!(result.hashes);
        assert!(!result.line.overflow);
        assert!(result.line.advance <= width);
        assert_eq!(result.line.runs[0].text, hashes);
    }
    assert_eq!(
        measure_fixed_number(&display, &book.faces[0], 10.0, 5.9, &limits)
            .unwrap_err()
            .reason,
        "numeric_hash_clipping"
    );
    // 通常の文字計測は数値に見える文字列でも置換しない。
    let text = measure(&display, &book.faces[0], 10.0, 6.0, &limits).unwrap();
    assert_eq!(text.runs[0].text, "123.45");
    assert!(text.overflow);
}

#[test]
fn ハッシュ表示で欠落文字や制限違反を隠さない() {
    use rhodopsin::xlsx::text_layout::measure_fixed_number;
    let book = fonts();
    let display = DisplayValue {
        fragments: vec![Fragment::Text("未".into())],
        color: None,
    };
    assert_eq!(
        measure_fixed_number(&display, &book.faces[0], 10.0, 6.0, &FontLimits::default())
            .unwrap_err()
            .reason,
        "font_glyph_missing"
    );
    let display = NumberFormat::parse("0.00")
        .unwrap()
        .number("123.45")
        .unwrap();
    let limits = FontLimits {
        run_bytes: 2,
        ..FontLimits::default()
    };
    assert_eq!(
        measure_fixed_number(&display, &book.faces[0], 10.0, 6.0, &limits)
            .unwrap_err()
            .reason,
        "text_run_limit"
    );
    let hidden = NumberFormat::parse(";;;")
        .unwrap()
        .number("123.45")
        .unwrap();
    let result =
        measure_fixed_number(&hidden, &book.faces[0], 10.0, 0.0, &FontLimits::default()).unwrap();
    assert!(!result.hashes);
    assert!(result.line.runs.is_empty());
}

#[test]
fn 自動折り返しで空白と日本語文字と空行を失わない() {
    use rhodopsin::xlsx::text_layout::measure_wrapped;
    let book = fonts();
    for (text, width, expected) in [
        ("AB CD EF", 18.0, vec!["AB ", "CD ", "EF"]),
        ("見見見見", 12.0, vec!["見見", "見見"]),
        ("AB\n\nCD\n", 18.0, vec!["AB", "", "CD", ""]),
    ] {
        let lines = measure_wrapped(
            text,
            &book.faces[0],
            10.0,
            width,
            &FontLimits::default(),
            10000,
        )
        .unwrap();
        let actual: Vec<String> = lines
            .iter()
            .map(|line| line.runs.iter().map(|run| run.text.as_str()).collect())
            .collect();
        assert_eq!(actual, expected);
        assert!(lines.iter().all(|line| !line.overflow));
    }
}
#[test]
fn 自動折り返しの計測量と行数と分割不能を検査する() {
    use rhodopsin::xlsx::text_layout::measure_wrapped;
    let book = fonts();
    let limits = FontLimits::default();
    for (text, width, work, reason) in [
        ("AB CD EF", 18.0, 10, "text_wrap_work_limit"),
        ("ABCDEFG", 18.0, 1000, "automatic_text_wrap"),
        ("AB", 0.0, 1000, "invalid_text_width"),
    ] {
        assert_eq!(
            measure_wrapped(text, &book.faces[0], 10.0, width, &limits, work)
                .err()
                .unwrap()
                .reason,
            reason
        );
    }
    assert_eq!(
        measure_wrapped(
            &"\n".repeat(4096),
            &book.faces[0],
            10.0,
            20.0,
            &limits,
            10000
        )
        .err()
        .unwrap()
        .reason,
        "text_line_limit"
    );
}
