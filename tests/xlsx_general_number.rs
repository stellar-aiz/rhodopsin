#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::{
    fonts::{FontBook, FontLimits},
    format::{DisplayValue, Fragment},
    text_layout::{measure, measure_general_number},
};
fn book() -> FontBook {
    let mut book = FontBook::default();
    book.load_bytes(
        support::font("計測", false, false, 0),
        &FontLimits::default(),
    )
    .unwrap();
    book
}
fn text(result: &rhodopsin::xlsx::text_layout::NumericLine) -> String {
    result
        .line
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect()
}
fn width(book: &FontBook, text: &str) -> f64 {
    measure(
        &DisplayValue {
            fragments: vec![Fragment::Text(text.into())],
            color: None,
        },
        &book.faces[0],
        10.0,
        1000.0,
        &FontLimits::default(),
    )
    .unwrap()
    .advance
}
#[test]
fn 十進丸めと固定小数の桁上限と指数境界を保持する() {
    let book = book();
    for (raw, expected) in [
        ("0", "0"),
        ("-0.000", "0"),
        ("123.4500", "123.45"),
        ("1.234567890123", "1.23456789"),
        ("-12345678901", "-12345678901"),
        ("123456789012", "1.23457E+11"),
        ("99999999999.5", "1E+11"),
        ("0.0000123456789", "1.23457E-05"),
        ("0.00123456789", "0.001234568"),
        ("1e-9", "0.000000001"),
        ("1e-10", "1E-10"),
        ("-1e-1024", "-1E-1024"),
        ("1e1024", "1E+1024"),
    ] {
        let result =
            measure_general_number(raw, &book.faces[0], 10.0, 1000.0, &FontLimits::default())
                .unwrap();
        assert_eq!(text(&result), expected, "入力: {raw}");
        assert!(!result.hashes);
    }
}
#[test]
fn 比例幅の実計測で桁数と指数への切り替えを選ぶ() {
    let book = book();
    for (raw, expected) in [
        ("1234.5678", "1234.57"),
        ("123456789", "1.2E+08"),
        ("-1234.5678", "-1234.57"),
        ("9.999", "10"),
    ] {
        let result = measure_general_number(
            raw,
            &book.faces[0],
            10.0,
            width(&book, expected),
            &FontLimits::default(),
        )
        .unwrap();
        assert_eq!(text(&result), expected);
        assert!(!result.line.overflow && !result.hashes);
    }
}
#[test]
fn 指数も収まらないときだけ幅不足警告へ渡す() {
    let book = book();
    let result = measure_general_number(
        "123456789",
        &book.faces[0],
        10.0,
        18.0,
        &FontLimits::default(),
    )
    .unwrap();
    assert_eq!(text(&result), "###");
    assert!(result.hashes);
    let mut warnings = rhodopsin::xlsx::warnings::Warnings::default();
    result.record_warning(&mut warnings, 1, "A1", 1).unwrap();
    assert_eq!(
        serde_json::to_value(warnings.into_summaries()).unwrap()[0]["count"],
        1
    );
    assert_eq!(
        measure_general_number(
            "123456789",
            &book.faces[0],
            10.0,
            1.0,
            &FontLimits::default()
        )
        .unwrap_err()
        .reason,
        "numeric_hash_clipping"
    );
}
#[test]
fn 不正値と幅と字形制限を近似で回避しない() {
    let book = book();
    for raw in ["NaN", "inf", "機密", "1e1025"] {
        assert!(
            measure_general_number(raw, &book.faces[0], 10.0, 100.0, &FontLimits::default())
                .is_err()
        );
    }
    for width in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            measure_general_number("1", &book.faces[0], 10.0, width, &FontLimits::default())
                .is_err()
        );
    }
    let limits = FontLimits {
        run_bytes: 1,
        ..Default::default()
    };
    assert!(measure_general_number("123456789", &book.faces[0], 10.0, 100.0, &limits).is_err());
}

#[test]
fn 同じ文字数でも数字の幅と印刷倍率を反映する() {
    let book = book();
    for (raw, expected, hashes) in [("11111", "11111", false), ("99999", "####", true)] {
        for scale in [0.5, 1.0, 2.0] {
            let result = measure_general_number(
                raw,
                &book.faces[0],
                10.0 * scale,
                26.0 * scale,
                &FontLimits::default(),
            )
            .unwrap();
            assert_eq!(text(&result), expected);
            assert_eq!(result.hashes, hashes);
        }
    }
}
