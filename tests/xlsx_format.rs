use rhodopsin::xlsx::{
    format::{DisplayValue, Fragment, NumberFormat, builtin},
    styles::ColorSource,
};

fn text(display: DisplayValue) -> String {
    display
        .fragments
        .into_iter()
        .map(|f| match f {
            Fragment::Text(s) => s,
            _ => panic!("配置指示を文字列に変換しない"),
        })
        .collect()
}
fn number(code: &str, value: &str) -> String {
    text(NumberFormat::parse(code).unwrap().number(value).unwrap())
}

#[test]
fn 通貨と桁区切りと十進丸めを適用する() {
    assert_eq!(number("¥#,##0", "1234567.5"), "¥1,234,568");
    assert_eq!(number("$#,##0.00", "1234.565"), "$1,234.57");
    assert_eq!(number("0.00", "2.675"), "2.68");
    assert_eq!(number("#,##0.00", "999.995"), "1,000.00");
    assert_eq!(number("0.00", "-2.675"), "-2.68");
    assert_eq!(number("0.00", "0000.001"), "0.00");
    assert_eq!(number("00000", "12"), "00012");
}

#[test]
fn 指数表記の保存値とパーセントと千単位を扱う() {
    assert_eq!(number("0.00%", "0.12345"), "12.35%");
    assert_eq!(number("0.0%", "1.235E-2"), "1.2%");
    assert_eq!(number("#,##0,", "1234567"), "1,235");
    assert_eq!(number("0.00,,", "1234567"), "1.23");
    assert_eq!(number("0.00", "1e-100"), "0.00");
    assert_eq!(number("0", "5e-1"), "1");
    assert_eq!(number("0", "4.999e-1"), "0");
    assert_eq!(number("0", "1e20"), "100000000000000000000");
}

#[test]
fn 正負ゼロと文字列の区分を選ぶ() {
    let fmt = NumberFormat::parse(r#"#,##0;[Red](#,##0);"-";"品名："@"#).unwrap();
    assert_eq!(text(fmt.number("1234").unwrap()), "1,234");
    let negative = fmt.number("-1234").unwrap();
    assert_eq!(
        negative.color.as_ref().unwrap().source,
        ColorSource::Rgb([255, 0, 0])
    );
    assert_eq!(text(negative), "(1,234)");
    assert_eq!(text(fmt.number("-0.00").unwrap()), "-");
    assert_eq!(text(fmt.text("見積書").unwrap()), "品名：見積書");
    assert_eq!(number("0;(0)", "0"), "0");
    assert_eq!(number("0;;", "-3"), "");
    assert_eq!(number("0;;", "0"), "");
}

#[test]
fn 会計の空白と埋め文字を配置指示として保持する() {
    let fmt = NumberFormat::parse(r#"_($* #,##0.00_);_($* (#,##0.00);_($* "-"??_);_(@_)"#).unwrap();
    assert_eq!(
        fmt.number("12.3").unwrap().fragments,
        vec![
            Fragment::Space('('),
            Fragment::Text("$".into()),
            Fragment::Fill(' '),
            Fragment::Text("12.30".into()),
            Fragment::Space(')')
        ]
    );
    let zero = fmt.number("0").unwrap();
    assert_eq!(
        zero.fragments,
        vec![
            Fragment::Space('('),
            Fragment::Text("$".into()),
            Fragment::Fill(' '),
            Fragment::Text("-".into()),
            Fragment::Space('0'),
            Fragment::Space('0'),
            Fragment::Space(')')
        ]
    );
}

#[test]
fn 引用とエスケープ内の区切りを解釈しない() {
    assert_eq!(number(r#"";#0%"0"#, "2"), ";#0%2");
    assert_eq!(number(r#"0\;"#, "2"), "2;");
    assert_eq!(number(r#"0\%"#, "2"), "2%");
    assert_eq!(
        text(NumberFormat::parse("0").unwrap().text("未加工").unwrap()),
        "未加工"
    );
    assert_eq!(
        text(
            NumberFormat::parse("0;0;0;\"固定\"")
                .unwrap()
                .text("機密値")
                .unwrap()
        ),
        "固定"
    );
}

#[test]
fn 不正な形式と未実装形式を明示的に拒否する() {
    for code in [
        "0;0;0;@;0",
        "\"機密",
        "[Red",
        "0\\",
        "0_",
        "0*",
        "General",
        "yyyy/mm/dd",
        "[DBNum1]0",
        "[>機密]0",
        "0.00E00",
        "[$¥-407]0",
    ] {
        let error = NumberFormat::parse(code).unwrap_err();
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
    assert!(NumberFormat::parse("@").unwrap().number("123").is_err());
    assert!(NumberFormat::parse("0-0").unwrap().number("123").is_err());
    for raw in [
        "NaN",
        "inf",
        "1.2.3",
        "--2",
        "1e9999999",
        "",
        ".",
        "機密の値",
    ] {
        let e = NumberFormat::parse("0").unwrap().number(raw).unwrap_err();
        assert!(!serde_json::to_string(&e).unwrap().contains("機密"));
    }
}

#[test]
fn 組み込み表示形式を解決する() {
    assert_eq!(number(builtin(4).unwrap(), "1234.56"), "1,234.56");
    assert_eq!(number(builtin(10).unwrap(), "0.12"), "12.00%");
    assert_eq!(
        text(
            NumberFormat::parse(builtin(49).unwrap())
                .unwrap()
                .text("見積")
                .unwrap()
        ),
        "見積"
    );
    assert!(builtin(999).is_err());
}

#[test]
fn セルの表示形式参照と保存済み十進値を使う() {
    use rhodopsin::xlsx::{
        styles::Styles,
        workbook::{Cell, CellValue},
    };
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="164" formatCode="¥#,##0.00"/></numFmts><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="164"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let styles = Styles::parse(doc.root_element()).unwrap();
    let cell = Cell {
        address: "A1".into(),
        style: 0,
        value: CellValue::Number(1234.565),
        numeric_lexeme: Some("1234.565".into()),
    };
    assert_eq!(
        text(rhodopsin::xlsx::format::cell(&cell, Some(&styles)).unwrap()),
        "¥1,234.57"
    );
    let cell = Cell {
        address: "A1".into(),
        style: 0,
        value: CellValue::Text("見積書".into()),
        numeric_lexeme: None,
    };
    assert_eq!(
        text(rhodopsin::xlsx::format::cell(&cell, None).unwrap()),
        "見積書"
    );
}

#[test]
fn 標準形式の真偽値とエラー値を数値に変換しない() {
    use rhodopsin::xlsx::workbook::{Cell, CellValue};
    for (value, expected) in [
        (CellValue::Boolean(true), "TRUE"),
        (CellValue::Boolean(false), "FALSE"),
        (CellValue::Error("#DIV/0!".into()), "#DIV/0!"),
        (CellValue::Error("#N/A".into()), "#N/A"),
        (CellValue::Error("#GETTING_DATA".into()), "#GETTING_DATA"),
    ] {
        let cell = Cell {
            address: "A1".into(),
            style: 0,
            value,
            numeric_lexeme: None,
        };
        assert_eq!(
            text(rhodopsin::xlsx::format::cell(&cell, None).unwrap()),
            expected
        );
    }
    let cell = Cell {
        address: "A1".into(),
        style: 0,
        value: CellValue::Error("機密の未定義エラー".into()),
        numeric_lexeme: None,
    };
    let error = rhodopsin::xlsx::format::cell(&cell, None).unwrap_err();
    assert_eq!(error.reason, "unknown_cell_error");
    assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
}

#[test]
fn 真偽値にもスタイル検査を適用し未対応の区分は拒否する() {
    use rhodopsin::xlsx::{
        styles::Styles,
        workbook::{Cell, CellValue},
    };
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="1"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let styles = Styles::parse(doc.root_element()).unwrap();
    let mut cell = Cell {
        address: "A1".into(),
        style: 0,
        value: CellValue::Boolean(true),
        numeric_lexeme: None,
    };
    assert_eq!(
        rhodopsin::xlsx::format::cell(&cell, Some(&styles))
            .unwrap_err()
            .reason,
        "boolean_custom_format"
    );
    cell.style = 999;
    assert!(rhodopsin::xlsx::format::cell(&cell, Some(&styles)).is_err());
}

#[test]
fn 数値パターンの先頭と末尾の小数点を保持する() {
    assert_eq!(number(".00", "0.125"), ".13");
    assert_eq!(number(".00", "12.125"), "12.13");
    assert_eq!(number(".00", "-0.125"), "-.13");
    assert_eq!(number("0.", "12.5"), "13.");
    assert_eq!(number("0.%", "0.125"), "13.%");
    assert_eq!(number(r#""."0"#, "12.5"), ".13");
    assert!(NumberFormat::parse("0..00").unwrap().number("1.2").is_err());
}

#[test]
fn 組み込み負数書式の括弧幅と赤色を保持する() {
    for (id, positive, negative) in [
        (37, "1,235", "(1,235)"),
        (38, "1,235", "(1,235)"),
        (39, "1,234.50", "(1,234.50)"),
        (40, "1,234.50", "(1,234.50)"),
    ] {
        let formatter = NumberFormat::parse(builtin(id).unwrap()).unwrap();
        assert_eq!(
            formatter.number("1234.5").unwrap().fragments,
            vec![Fragment::Text(positive.into()), Fragment::Space(')')]
        );
        let result = formatter.number("-1234.5").unwrap();
        assert_eq!(result.color.is_some(), id == 38 || id == 40);
        assert_eq!(text(result), negative);
    }
}

#[test]
fn 小数点直前のカンマを桁区切りと混同しない() {
    assert_eq!(number("0,.00", "1234567"), "1234.57");
    assert_eq!(number("#,##0,.00", "1234567"), "1,234.57");
    assert_eq!(number("0,,.00", "1234567"), "1.23");
    assert_eq!(number("0,.00%", "1234"), "123.40%");
}

#[test]
fn 通貨とロケール指定を数値区分へ適用する() {
    assert_eq!(number("[$¥-411]#,##0", "1234.5"), "¥1,235");
    assert_eq!(number("[$$-409]#,##0.00", "1234.5"), "$1,234.50");
    assert_eq!(number("[$USD-0409]0.00", "12"), "USD12.00");
    assert_eq!(number("[$¥-ja-JP]0", "12"), "¥12");
    assert_eq!(number("[$-409]0.00", "12"), "12.00");
    assert_eq!(number("[$€]0.00", "12"), "€12.00");
    assert_eq!(number("[$¥-411]0;[Red]([$¥-411]0)", "-12"), "(¥12)");
    assert!(NumberFormat::parse("[$¥-411][$¥-411]0").is_err());
    assert!(NumberFormat::parse("[$機密-F800]0").is_err());
    let error = NumberFormat::parse("[$機密-zzzz]0").unwrap_err();
    assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
}

#[test]
fn 条件を丸め前の十進数で左から評価する() {
    let fmt = NumberFormat::parse("[>=100]0.0;[>=10]0.00;0.000").unwrap();
    assert_eq!(text(fmt.number("100").unwrap()), "100.0");
    assert_eq!(text(fmt.number("99.999").unwrap()), "100.00");
    assert_eq!(text(fmt.number("9.9999").unwrap()), "10.000");
    assert_eq!(number("[=0.1]0.00;0.000", "1e-1"), "0.10");
    assert_eq!(
        number("[>0.10000000000000001]0.00;0.000", "0.10000000000000002"),
        "0.10"
    );
    assert_eq!(number("[<>0]0.0;0.00", "-0.00"), "0.00");
    assert_eq!(number("[>1e-20]0.0;0.00", "1e-21"), "0.00");
}
#[test]
fn 条件付き負数の区分と不正な条件を処理する() {
    assert_eq!(number("[>=100]0;[<0](0);0", "-2"), "(2)");
    assert_eq!(number("[<0]0;0", "-2"), "-2");
    assert_eq!(number("[>100]0;[>10]0;0", "-2"), "-2");
    for code in ["[>1][<2]0", "0[>1]", "0;0;[>0]0", "[>NaN]0"] {
        assert!(NumberFormat::parse(code).is_err());
    }
    assert!(NumberFormat::parse("[>100]0").unwrap().number("1").is_err());
}

#[test]
fn 指数表示の符号と桁数と丸めによる繰り上がりを扱う() {
    for (code, raw, expected) in [
        ("0.00E+00", "12200000", "1.22E+07"),
        ("0.00E+00", "0.001234", "1.23E-03"),
        ("0.00E+00", "9.995", "1.00E+01"),
        ("0.00E+00", "0.009995", "1.00E-02"),
        ("0.00E+00", "9.994", "9.99E+00"),
        ("0.00E+00", "-12200000", "-1.22E+07"),
        ("0.00E+00", "0", "0.00E+00"),
        ("0.0e-00", "1200", "1.2e03"),
        ("0.0e-00", "0.012", "1.2e-02"),
        ("0E+0", "99", "1E+2"),
        ("0.00E+00", "1e308", "1.00E+308"),
        ("0.00E+00", "1e-308", "1.00E-308"),
        ("0.##E+00", "12500", "1.25E+04"),
        ("0.00E+00;[Red](0.00E+00);\"zero\";@", "-9995", "(1.00E+04)"),
    ] {
        assert_eq!(number(code, raw), expected, "{code} / {raw}");
    }
    assert_eq!(number(builtin(11).unwrap(), "12200000"), "1.22E+07");
}

#[test]
fn 未対応の指数パターンを近似しない() {
    for code in [
        "00.0E+00",
        "0.0E+#",
        "0.0E+00%",
        "0.0E+00E+00",
        "0.0E+",
        "0,0E+00",
    ] {
        assert!(NumberFormat::parse(code).unwrap().number("1234").is_err());
    }
    assert_eq!(number(r#"0.00"E+00""#, "12.3"), "12.30E+00");
    let code = format!("0.{}E+00", "0".repeat(31));
    assert!(NumberFormat::parse(&code).unwrap().number("12.3").is_err());
}

#[test]
fn 日本語dbnumの整数桁を変換して引用文字は保持する() {
    assert_eq!(
        number("[DBNum1][$-411]0", "1234567890"),
        "一二三四五六七八九〇"
    );
    assert_eq!(
        number("[DBNum2][$-411]0", "1234567890"),
        "壱弐参四伍六七八九零"
    );
    assert_eq!(
        number("[DBNum3][$-411]0", "1234567890"),
        "１２３４５６７８９０"
    );
    assert_eq!(
        number(r#"[DBNum3][$-ja-JP]"ID2:"0000"号3""#, "12.5"),
        "ID2:００１３号3"
    );
    assert_eq!(number("[$-0411][dbnum1]000", "12"), "〇一二");
    assert_eq!(number("[DBNum3][$-411]#", "0"), "");
    let formatter = NumberFormat::parse("[Red][DBNum1][$-411]0;0;\"零\";@").unwrap();
    assert_eq!(text(formatter.number("0").unwrap()), "零");
    assert_eq!(text(formatter.text("商品123").unwrap()), "商品123");
    let value = formatter.number("12").unwrap();
    assert_eq!(
        value.color.as_ref().unwrap().source,
        ColorSource::Rgb([255, 0, 0])
    );
    assert_eq!(text(value), "一二");
}

#[test]
fn dbnumの未確認規則と不正な指定を数字に近似しない() {
    for code in [
        "[DBNum1]0",
        "[DBNum2][$-409]0",
        "[DBNum3][$-411]0.0",
        "[DBNum1][$-411]#,##0",
        "[DBNum2][$-411]General",
        "[DBNum3][$-411]0%",
        "[DBNum1][$-411]0E+00",
        "[DBNum1][$-411]?",
        "[DBNum1][$-411]@",
        "[DBNum4][$-411]0",
        "[DBNum1][DBNum2][$-411]0",
        "0[DBNum1][$-411]",
    ] {
        assert!(NumberFormat::parse(code).is_err(), "{code}");
    }
    assert_eq!(
        NumberFormat::parse("[DBNum1][$-411]0")
            .unwrap()
            .number("-1")
            .unwrap_err()
            .reason,
        "dbnum_negative_format"
    );
}

#[test]
fn dbnumは明示コンテキストを継承し区分のロケールで上書きする() {
    use rhodopsin::xlsx::format::date::Locale;
    for (code, expected) in [
        ("[DBNum1]0", "一二三"),
        ("[DBNum2]0", "壱弐参"),
        ("[DBNum3]0", "１２３"),
    ] {
        let format = NumberFormat::parse_with_locale(code, Locale::Japanese).unwrap();
        assert_eq!(text(format.number("123").unwrap()), expected);
        assert_eq!(
            NumberFormat::parse_with_locale(code, Locale::English)
                .unwrap_err()
                .reason,
            "dbnum_locale"
        );
        assert_eq!(
            NumberFormat::parse(code).unwrap_err().reason,
            "dbnum_locale"
        );
    }
    let explicit = NumberFormat::parse_with_locale("[DBNum3][$-411]0", Locale::English).unwrap();
    assert_eq!(text(explicit.number("123").unwrap()), "１２３");
    assert_eq!(
        NumberFormat::parse_with_locale("[DBNum3][$-409]0", Locale::Japanese)
            .unwrap_err()
            .reason,
        "dbnum_locale"
    );
    // ある区分の指定を、別区分の暗黙ロケールとして使わない。
    assert!(
        NumberFormat::parse_with_locale("[DBNum3][$-411]0;[DBNum3]0", Locale::English).is_err()
    );
}

#[test]
fn 工学指数の組み込み形式と三桁境界を扱う() {
    assert_eq!(builtin(48).unwrap(), "##0.0E+0");
    for (raw, expected) in [
        ("0", "0.0E+0"),
        ("1", "1.0E+0"),
        ("12", "12.0E+0"),
        ("123", "123.0E+0"),
        ("1234", "1.2E+3"),
        ("12345", "12.3E+3"),
        ("123456", "123.5E+3"),
        ("0.1", "100.0E-3"),
        ("0.01", "10.0E-3"),
        ("0.001", "1.0E-3"),
        ("0.0001", "100.0E-6"),
        ("999.94", "999.9E+0"),
        ("999.95", "1.0E+3"),
        ("0.99995", "1.0E+0"),
        ("-999.95", "-1.0E+3"),
        ("1e308", "100.0E+306"),
        ("1e-308", "10.0E-309"),
    ] {
        assert_eq!(number(builtin(48).unwrap(), raw), expected, "{raw}");
    }
    assert_eq!(number("##0.##e-00", "1234"), "1.23e03");
    assert_eq!(number("##0E+0", "999.5"), "1E+3");
    assert_eq!(number("##0.0E+0;[Red](##0.0E+0)", "-1234"), "(1.2E+3)");
}

#[test]
fn dbnumと通貨のゼロ埋めlcidを数値として解釈する() {
    for width in 3..=8 {
        let id = format!("{:0width$x}", 0x411);
        for (kind, expected) in [(1, "一二三"), (2, "壱弐参"), (3, "１２３")] {
            assert_eq!(number(&format!("[DBNum{kind}][$-{id}]0"), "123"), expected);
        }
        assert_eq!(number(&format!("[$¥-{id}]0"), "123"), "¥123");
        let english = format!("{:0width$x}", 0x409);
        assert!(NumberFormat::parse(&format!("[DBNum3][$-{english}]0")).is_err());
        assert_eq!(number(&format!("[$USD-{english}]0"), "123"), "USD123");
    }
    for id in ["40411", "FFFF0411", "000000411", "F800", "407"] {
        assert!(NumberFormat::parse(&format!("[DBNum3][$-{id}]0")).is_err());
        assert!(NumberFormat::parse(&format!("[$¥-{id}]0")).is_err());
    }
}

fn fraction_visible(display: DisplayValue) -> String {
    display
        .fragments
        .into_iter()
        .map(|fragment| match fragment {
            Fragment::Text(text) => text,
            Fragment::Space(_) => "_".into(),
            Fragment::Fill(_) => panic!("分数は埋め文字を生成しない"),
        })
        .collect()
}
#[test]
fn 組み込み分数の分母制限と桁合わせを保持する() {
    assert_eq!(builtin(12).unwrap(), "# ?/?");
    assert_eq!(builtin(13).unwrap(), "# ??/??");
    for (id, raw, expected) in [
        (12, "0.5", " 1/2"),
        (13, "0.5", " _1/2_"),
        (12, "1.5625", "1 4/7"),
        (13, "1.5625", "1 _9/16"),
        (12, "2.0", "2 ___"),
        (13, "2.0", "2 _____"),
        (12, "0", "0 ___"),
        (12, "-0.5", "- 1/2"),
        (12, "0.999", "1 ___"),
        (13, "0.999", "1 _____"),
        (13, "0.125", " _1/8_"),
        (13, "0.3333333333333333333", " _1/3_"),
        (12, "1e-1000", "0 ___"),
        (
            12,
            "9999999999999999999999999999.999",
            "10000000000000000000000000000 ___",
        ),
    ] {
        let display = NumberFormat::parse(builtin(id).unwrap())
            .unwrap()
            .number(raw)
            .unwrap();
        assert_eq!(fraction_visible(display), expected, "番号{id} 入力{raw}");
    }
    let display = NumberFormat::parse("# ??/??")
        .unwrap()
        .number("0.5")
        .unwrap();
    assert_eq!(
        display.fragments,
        vec![
            Fragment::Text(" ".into()),
            Fragment::Space('0'),
            Fragment::Text("1/2".into()),
            Fragment::Space('0')
        ]
    );
}
#[test]
fn 二進浮動小数点では同値になる分数丸め境界を十進で区別する() {
    for (raw, expected) in [
        ("0.09999999999999999999999999999999999999999999", "0 ___"),
        ("0.10000000000000000000000000000000000000000001", " 1/9"),
    ] {
        assert_eq!(
            fraction_visible(NumberFormat::parse("# ?/?").unwrap().number(raw).unwrap()),
            expected
        );
    }
}
#[test]
fn 分数の区分と色と必須整数部を保持する() {
    let display = NumberFormat::parse("# ?/?;[Red](# ?/?)")
        .unwrap()
        .number("-1.5")
        .unwrap();
    assert_eq!(
        display.color.as_ref().unwrap().source,
        ColorSource::Rgb([255, 0, 0])
    );
    assert_eq!(fraction_visible(display), "(1 1/2)");
    assert_eq!(
        fraction_visible(NumberFormat::parse("0 ?/?").unwrap().number("0.5").unwrap()),
        "0 1/2"
    );
    for code in ["# ???/???", "?/?", "# ?/8", "# ?/?%", "# 0/0"] {
        assert!(
            NumberFormat::parse(code)
                .and_then(|f| f.number("1.5"))
                .is_err()
        );
    }
}
