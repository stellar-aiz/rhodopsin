use rhodopsin::xlsx::{
    styles::{Font, FontScheme, Underline},
    theme::{TextScript, Theme, ThemeFonts, resolve_font},
};
fn theme() -> Theme {
    Theme {
        colors: [[0; 3]; 12],
        major: ThemeFonts {
            latin: "見出し欧文".into(),
            scripts: [("Jpan".into(), "見出し和文".into())].into(),
            ..ThemeFonts::default()
        },
        minor: ThemeFonts {
            latin: "本文欧文".into(),
            scripts: [
                ("Jpan".into(), "本文和文".into()),
                ("Hans".into(), "簡体字".into()),
            ]
            .into(),
            ..ThemeFonts::default()
        },
    }
}
fn font() -> Font {
    Font {
        family: Some("保存時の書体".into()),
        size: Some(11.0),
        bold: true,
        italic: false,
        underline: Underline::None,
        color: Default::default(),
        scheme: FontScheme::Minor,
    }
}
#[test]
fn テーマの主要補助書体と文字体系を解決する() {
    let theme = theme();
    let mut font = font();
    let request = resolve_font(&font, Some(&theme), TextScript::Latin).unwrap();
    assert_eq!(
        (request.family, request.points, request.bold),
        ("本文欧文", 11.0, true)
    );
    assert_eq!(
        resolve_font(&font, Some(&theme), TextScript::EastAsian("Jpan"))
            .unwrap()
            .family,
        "本文和文"
    );
    assert_eq!(
        resolve_font(&font, Some(&theme), TextScript::EastAsian("Hans"))
            .unwrap()
            .family,
        "簡体字"
    );
    font.scheme = FontScheme::Major;
    assert_eq!(
        resolve_font(&font, Some(&theme), TextScript::EastAsian("Jpan"))
            .unwrap()
            .family,
        "見出し和文"
    );
    font.scheme = FontScheme::None;
    assert_eq!(
        resolve_font(&font, None, TextScript::Latin).unwrap().family,
        "保存時の書体"
    );
}
#[test]
fn 未解決のテーマを欧文や保存書体で代用しない() {
    let theme = theme();
    let font = font();
    assert!(resolve_font(&font, None, TextScript::Latin).is_err());
    let error = resolve_font(&font, Some(&theme), TextScript::EastAsian("Hant"))
        .err()
        .unwrap();
    assert_eq!(error.reason, "theme_font_unresolved");
    assert!(!serde_json::to_string(&error).unwrap().contains("保存時"));
    assert!(theme.minor.family(TextScript::EastAsian("機密")).is_err());
}
#[test]
fn 明示された東アジア書体を使う() {
    let mut theme = theme();
    theme.minor.east_asian = "明示書体".into();
    assert_eq!(
        theme.minor.family(TextScript::EastAsian("Jpan")).unwrap(),
        "明示書体"
    );
}

#[path = "xlsx_support/mod.rs"]
mod support;
#[test]
fn 解決した書体も実フォントの厳密選択を通す() {
    use rhodopsin::xlsx::fonts::{FontBook, FontLimits};
    let theme = theme();
    let font = font();
    let request = resolve_font(&font, Some(&theme), TextScript::EastAsian("Jpan")).unwrap();
    let mut book = FontBook::default();
    book.load_bytes(
        support::font("本文和文", true, false, 0),
        &FontLimits::default(),
    )
    .unwrap();
    let selection = book.select_request(&request, None).unwrap();
    assert_eq!(selection.index, 0);
    assert!(!selection.substituted);
    let latin = resolve_font(&font, Some(&theme), TextScript::Latin).unwrap();
    assert!(book.select_request(&latin, None).is_err());
    assert!(
        book.select_request(&latin, Some("本文和文"))
            .unwrap()
            .substituted
    );
}
