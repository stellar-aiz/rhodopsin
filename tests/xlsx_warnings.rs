#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::{
    fonts::{FontBook, FontLimits},
    format::NumberFormat,
    text_layout::measure_fixed_number,
    warnings::{Kind, Warnings},
};
#[test]
fn ハッシュ表示を位置だけの警告に接続し重複を除く() {
    let mut fonts = FontBook::default();
    let limits = FontLimits::default();
    fonts
        .load_bytes(support::font("検証", false, false, 0), &limits)
        .unwrap();
    let display = NumberFormat::parse("0.00")
        .unwrap()
        .number("987654.32")
        .unwrap();
    let narrow = measure_fixed_number(&display, &fonts.faces[0], 10.0, 18.0, &limits).unwrap();
    let wide = measure_fixed_number(&display, &fonts.faces[0], 10.0, 100.0, &limits).unwrap();
    let mut warnings = Warnings::default();
    for (sheet, cell) in [(2, "A1"), (1, "A10"), (1, "B2"), (1, "A2"), (1, "A2")] {
        narrow
            .record_warning(&mut warnings, sheet, cell, 4)
            .unwrap();
    }
    wide.record_warning(&mut warnings, 1, "A1", 4).unwrap();
    let json = serde_json::to_value(warnings.into_summaries()).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{"kind":"numeric_overflow","count":4,"locations":[
            {"sheet_index":1,"cell":"A2"},{"sheet_index":1,"cell":"B2"},{"sheet_index":1,"cell":"A10"},{"sheet_index":2,"cell":"A1"}
        ]}])
    );
    assert!(!json.to_string().contains("987654"));
}
#[test]
fn 不正な番地と上限超過を集計へ残さない() {
    let mut warnings = Warnings::default();
    for (sheet, cell) in [(0, "A1"), (1, "機密"), (1, "XFE1"), (1, "A0"), (1, "A01")] {
        let error = warnings
            .record(Kind::NumericOverflow, sheet, cell, 1)
            .unwrap_err();
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
    warnings
        .record(Kind::NumericOverflow, 1, "XFD1048576", 1)
        .unwrap();
    assert!(warnings.record(Kind::NumericOverflow, 1, "A1", 1).is_err());
    warnings
        .record(Kind::NumericOverflow, 1, "XFD1048576", 1)
        .unwrap();
    let result = serde_json::to_value(warnings.into_summaries()).unwrap();
    assert_eq!(result[0]["count"], 1);
    assert_eq!(result[0]["locations"][0]["cell"], "XFD1048576");
}
