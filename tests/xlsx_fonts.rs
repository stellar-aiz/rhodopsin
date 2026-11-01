#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::diagnostic::Category;
use rhodopsin::xlsx::fonts::{FontBook, FontLimits};

#[test]
fn 書体とスタイルを厳密に選択する() {
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(support::font("試験書体", false, false, 0), &limits)
        .unwrap();
    fonts
        .load_bytes(support::font("試験書体", true, false, 0), &limits)
        .unwrap();
    assert_eq!(
        fonts.select("試験書体", false, false, None).unwrap().index,
        0
    );
    assert_eq!(
        fonts.select("試験書体", true, false, None).unwrap().index,
        1
    );
    let error = fonts.select("試験書体", false, true, None).unwrap_err();
    assert_eq!(error.category, Category::MissingFont);
    assert_eq!(error.reason, "font_style_missing");
    assert_eq!(
        fonts
            .select("機密書体", false, false, None)
            .unwrap_err()
            .reason,
        "font_family_missing"
    );
    assert!(
        fonts
            .select("機密書体", false, false, Some("試験書体"))
            .unwrap()
            .substituted
    );
}

#[test]
fn 文字幅と位置とクラスタを計測する() {
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(support::font("試験", false, false, 0), &limits)
        .unwrap();
    let font = &fonts.faces[0];
    assert!((font.maximum_digit_width(10.0).unwrap() - 5.9).abs() < 1e-10);
    let run = font.shape_run("A見B", 10.0, &limits).unwrap();
    assert_eq!(
        run.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(),
        [0, 1, 4]
    );
    assert_eq!(run.advance, 18.0);
    assert_eq!((run.ascender, run.descender), (8.0, -2.0));
    assert_eq!(run.glyphs.len(), 3);
}

#[test]
fn 欠落文字を豆腐や別書体に置き換えない() {
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(support::font("試験", false, false, 0), &limits)
        .unwrap();
    let error = fonts.faces[0].shape_run("機密", 10.0, &limits).unwrap_err();
    assert_eq!(error.reason, "font_glyph_missing");
    assert_eq!(error.category, Category::MissingFont);
    assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
}

#[test]
fn 埋め込み制限とサブセット禁止を守る() {
    for flags in [2, 0x100, 0x200] {
        let mut fonts = FontBook::default();
        fonts
            .load_bytes(
                support::font("制限書体", false, false, flags),
                &FontLimits::default(),
            )
            .unwrap();
        assert_eq!(
            fonts
                .select("制限書体", false, false, None)
                .unwrap_err()
                .reason,
            "font_embedding_restricted"
        );
    }
}

#[test]
fn フォント資源上限と入力不正を検出する() {
    let mut fonts = FontBook::default();
    let limits = FontLimits {
        file_bytes: 1,
        ..FontLimits::default()
    };
    assert_eq!(
        fonts
            .load_bytes(support::font("試験", false, false, 0), &limits)
            .unwrap_err()
            .reason,
        "font_file_limit"
    );
    assert!(
        fonts
            .load_bytes(vec![0; 100], &FontLimits::default())
            .is_err()
    );
    assert!(fonts.faces.is_empty());
    fonts
        .load_bytes(
            support::font("試験", false, false, 0),
            &FontLimits::default(),
        )
        .unwrap();
    fonts
        .load_bytes(
            support::font("試験", false, false, 0),
            &FontLimits::default(),
        )
        .unwrap();
    assert_eq!(fonts.faces.len(), 1);
    assert!(
        fonts.faces[0]
            .shape_run(
                "AB",
                10.0,
                &FontLimits {
                    run_bytes: 1,
                    ..FontLimits::default()
                }
            )
            .is_err()
    );
    assert!(fonts.faces[0].maximum_digit_width(f64::NAN).is_err());
}

#[test]
fn 明示ファイルだけを読み込む() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("font.ttf");
    std::fs::write(&path, support::font("試験", false, false, 0)).unwrap();
    let mut fonts = FontBook::default();
    fonts.load_file(&path, &FontLimits::default()).unwrap();
    assert_eq!(fonts.faces.len(), 1);
    assert!(fonts.select("試験", false, false, None).is_ok());
}

#[test]
fn macromanだけの書体名を選択に使いファイル名から推測しない() {
    let mut bytes = support::font("AAAA", false, false, 0);
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let record = (0..count)
        .map(|i| 12 + i * 16)
        .find(|offset| &bytes[*offset..*offset + 4] == b"name")
        .unwrap();
    let table = u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize;
    let storage =
        table + u16::from_be_bytes(bytes[table + 4..table + 6].try_into().unwrap()) as usize;
    let record = table + 6;
    bytes[record..record + 2].copy_from_slice(&1u16.to_be_bytes());
    bytes[record + 2..record + 4].copy_from_slice(&0u16.to_be_bytes());
    bytes[record + 4..record + 6].copy_from_slice(&0u16.to_be_bytes());
    bytes[record + 8..record + 10].copy_from_slice(&4u16.to_be_bytes());
    bytes[storage..storage + 4].copy_from_slice(b"Caf\x8e");
    let mut book = FontBook::default();
    book.load_bytes(bytes, &FontLimits::default()).unwrap();
    assert!(book.select("Café", false, false, None).is_ok());
    assert!(book.select("AAAA", false, false, None).is_err());
}

#[test]
fn 欧文で始まる双方向文字列も方向区間に分けて整形する() {
    let limits = FontLimits::default();
    for ch in ['א', 'ا', '\u{0661}', '\u{1e900}'] {
        let mut data = support::font("混在方向試験", false, false, 0);
        let table = data[12..]
            .chunks_exact(16)
            .find(|entry| &entry[..4] == b"cmap")
            .unwrap();
        let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
        data[offset + 40..offset + 44].copy_from_slice(&(ch as u32).to_be_bytes());
        data[offset + 44..offset + 48].copy_from_slice(&(ch as u32).to_be_bytes());
        let mut fonts = FontBook::default();
        fonts.load_bytes(data, &limits).unwrap();
        let face = rustybuzz::ttf_parser::Face::parse(fonts.faces[0].data(), 0).unwrap();
        assert!(face.glyph_index(ch).is_some());
        let text = format!("A{ch}B");
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(&text);
        buffer.guess_segment_properties();
        assert_eq!(buffer.direction(), rustybuzz::Direction::LeftToRight);
        let shaped = fonts.faces[0].shape_run(&text, 10.0, &limits).unwrap();
        assert_eq!(shaped.glyphs.len(), 3);
        assert_eq!(
            shaped.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(),
            [0, 1, 1 + ch.len_utf8()]
        );
    }
}

#[test]
fn 双方向制御文字も方向推定や字形欠落に依存せず停止する() {
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(support::font("試験", false, false, 0), &limits)
        .unwrap();
    for control in [
        '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}',
        '\u{2068}', '\u{2069}', '\u{200e}', '\u{200f}', '\u{061c}',
    ] {
        assert_eq!(
            fonts.faces[0]
                .shape_run(&format!("A{control}B"), 10.0, &limits)
                .unwrap_err()
                .reason,
            "text_direction"
        );
    }
    assert!(fonts.faces[0].shape_run("A見123", 10.0, &limits).is_ok());
}

#[test]
fn 異体字の既定と非既定を対応表どおりに選ぶ() {
    let limits = FontLimits::default();
    for selector in [0xfe00, 0xfe0f, 0xe0100, 0xe01ef] {
        for default in [false, true] {
            let mut fonts = FontBook::default();
            fonts
                .load_bytes(support::font_with_variation(selector, default, 34), &limits)
                .unwrap();
            let text = format!("見{}B", char::from_u32(selector).unwrap());
            let run = fonts.faces[0].shape_run(&text, 10.0, &limits).unwrap();
            assert_eq!(run.glyphs.len(), 2);
            assert_eq!(run.glyphs[0].id, if default { 96 } else { 34 });
            assert_eq!(run.glyphs[0].cluster, 0);
            assert_eq!(run.glyphs[1].cluster, text.len() - 1);
            assert_eq!(run.advance, 12.0);
        }
    }
}
#[test]
fn 未対応異体字と単独セレクタを基底字形で代用しない() {
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(support::font_with_variation(0xe0100, false, 34), &limits)
        .unwrap();
    for text in ["見\u{e0101}", "A\u{e0100}"] {
        let error = fonts.faces[0].shape_run(text, 10.0, &limits).unwrap_err();
        assert_eq!(error.reason, "font_variation_sequence_missing");
        assert_eq!(error.category, Category::MissingFont);
    }
    for text in ["\u{e0100}", "見\u{e0100}\u{e0100}"] {
        assert_eq!(
            fonts.faces[0]
                .shape_run(text, 10.0, &limits)
                .unwrap_err()
                .reason,
            "variation_selector_without_base"
        );
    }
    for glyph in [0, 97] {
        let mut invalid = FontBook::default();
        invalid
            .load_bytes(support::font_with_variation(0xe0100, false, glyph), &limits)
            .unwrap();
        assert!(
            invalid.faces[0]
                .shape_run("見\u{e0100}", 10.0, &limits)
                .is_err()
        );
    }
    let mut plain = FontBook::default();
    plain
        .load_bytes(support::font("通常", false, false, 0), &limits)
        .unwrap();
    assert_eq!(
        plain.faces[0]
            .shape_run("見\u{e0100}", 10.0, &limits)
            .unwrap_err()
            .reason,
        "font_variation_sequence_missing"
    );
}

#[test]
fn 画像字形の異体字を輪郭字形へ黙って置き換えない() {
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(support::font_with_svg_variation(), &limits)
        .unwrap();
    assert_eq!(
        fonts.faces[0]
            .shape_run("見\u{e0100}", 10.0, &limits)
            .unwrap_err()
            .reason,
        "variation_glyph_image"
    );
    // 同じ書体の通常の基底文字は画像字形ではない。
    assert!(fonts.faces[0].shape_run("見", 10.0, &limits).is_ok());
}

#[test]
fn 右から左の括弧に必要な鏡像字形が欠落したら停止する() {
    let limits = FontLimits::default();
    let mut data = support::bidi_font();
    let table = data[12..]
        .chunks_exact(16)
        .find(|entry| &entry[..4] == b"cmap")
        .unwrap();
    let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
    // 欧文範囲を ( までに限定し、) を欠落させる。
    data[offset + 32..offset + 36].copy_from_slice(&0x28u32.to_be_bytes());
    let mut fonts = FontBook::default();
    fonts.load_bytes(data, &limits).unwrap();
    let error = fonts.faces[0].shape_run("אב(", 10.0, &limits).unwrap_err();
    assert_eq!(error.reason, "font_mirrored_glyph_missing");
}
