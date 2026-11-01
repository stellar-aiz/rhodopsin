use rhodopsin::xlsx::format::date::{DateFormat, DateSystem, Locale, from_serial};

fn formatted(code: &str, serial: &str) -> String {
    DateFormat::parse(code, Locale::English)
        .unwrap()
        .format(serial, DateSystem::Excel1900)
        .unwrap()
}

#[test]
fn 二つの日付基準と架空の閏日を扱う() {
    for (serial, expected) in [
        (0, "1900-01-00"),
        (1, "1900-01-01"),
        (59, "1900-02-28"),
        (60, "1900-02-29"),
        (61, "1900-03-01"),
        (1462, "1904-01-01"),
        (40729, "2011-07-05"),
        (2958465, "9999-12-31"),
    ] {
        assert_eq!(formatted("yyyy-mm-dd", &serial.to_string()), expected);
    }
    let format = DateFormat::parse("yyyy-mm-dd", Locale::English).unwrap();
    assert_eq!(
        format.format("0", DateSystem::Excel1904).unwrap(),
        "1904-01-01"
    );
    assert_eq!(
        format.format("39267", DateSystem::Excel1904).unwrap(),
        "2011-07-05"
    );
    assert_eq!(
        format.format("2957003", DateSystem::Excel1904).unwrap(),
        "9999-12-31"
    );
}

#[test]
fn 世紀の閏年と日付の連続性を確認する() {
    let mut previous = from_serial(61, DateSystem::Excel1900).unwrap();
    for serial in 62..=100000 {
        let current = from_serial(serial, DateSystem::Excel1900).unwrap();
        assert!(current.day >= 1 && current.day <= 31);
        assert!(current.month >= 1 && current.month <= 12);
        assert_eq!(current.weekday, (previous.weekday + 1) % 7);
        if current.month == previous.month {
            assert_eq!(current.day, previous.day + 1);
        } else {
            assert_eq!(current.day, 1);
        }
        if current.year == 1900 || current.year == 2100 {
            assert!(!(current.month == 2 && current.day == 29));
        }
        previous = current;
    }
    assert_eq!(formatted("yyyy-mm-dd", "36585"), "2000-02-29");
}

#[test]
fn 月と分を文脈で区別する() {
    assert_eq!(
        formatted("yyyy-mm-dd hh:mm:ss", "40729.5"),
        "2011-07-05 12:00:00"
    );
    assert_eq!(formatted("mm:ss", "0.0430555555555556"), "02:00");
    assert_eq!(formatted("mm", "40729.5"), "07");
    assert_eq!(formatted("h:mm AM/PM", "0.5"), "12:00 PM");
    assert_eq!(formatted("h:mm A/P", "0"), "12:00 A");
}

#[test]
fn 経過時間と秒の小数を表示する() {
    assert_eq!(formatted("[h]:mm:ss", "1.5"), "36:00:00");
    assert_eq!(formatted("[mm]:ss", "1.5"), "2160:00");
    assert_eq!(formatted("[ss].00", "0.5"), "43200.00");
    assert_eq!(
        formatted("hh:mm:ss.000", "0.00000578703703703704"),
        "00:00:00.500"
    );
    assert_eq!(
        formatted("yyyy-mm-dd hh:mm:ss", "40729.9999999"),
        "2011-07-06 00:00:00"
    );
    assert_eq!(formatted("yyyy-mm-dd", "40729.9999999"), "2011-07-05");
}

#[test]
fn 元号境界と日本語曜日を表示する() {
    assert_eq!(formatted("ggge年m月d日", "43585"), "平成31年4月30日");
    assert_eq!(formatted("ggge年m月d日", "43586"), "令和1年5月1日");
    assert_eq!(formatted("g ee/m/d", "43586"), "R 01/5/1");
    assert_eq!(formatted("ggge年m月d日", "32515"), "昭和64年1月7日");
    assert_eq!(formatted("ggge年m月d日", "32516"), "平成1年1月8日");
    assert_eq!(
        formatted("yyyy年m月d日(aaaa)", "43586"),
        "2019年5月1日(水曜日)"
    );
    assert_eq!(formatted("ddd, mmm d, yyyy", "43586"), "Wed, May 1, 2019");
}

#[test]
fn 日付の不正値と未対応書式を拒否する() {
    let f = DateFormat::parse("yyyy/mm/dd", Locale::Japanese).unwrap();
    for value in ["-1", "NaN", "inf", "2958466", "機密"] {
        let error = f.format(value, DateSystem::Excel1900).unwrap_err();
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
    for code in [
        "[DBNum1]yyyy",
        "yyyy;mm",
        "yyyy_)",
        "yyyy\"機密",
        "[hh",
        "hh:mm:ss.0000",
    ] {
        assert!(DateFormat::parse(code, Locale::Japanese).is_err());
    }
}

#[test]
fn セルの和暦表示と明示的な日付基準を接続する() {
    use rhodopsin::xlsx::{
        format::{FormatContext, Fragment, cell_with_context},
        styles::Styles,
        workbook::{Cell, CellValue},
    };
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="164" formatCode="ggge年m月d日"/></numFmts><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="164"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let styles = Styles::parse(doc.root_element()).unwrap();
    let cell = Cell {
        address: "A1".into(),
        style: 0,
        value: CellValue::Number(42124.0),
        numeric_lexeme: Some("42124".into()),
    };
    let display = cell_with_context(
        &cell,
        Some(&styles),
        FormatContext {
            date_system: DateSystem::Excel1904,
            locale: Locale::Japanese,
        },
    )
    .unwrap();
    assert_eq!(
        display.fragments,
        vec![Fragment::Text("令和1年5月1日".into())]
    );
}

#[test]
fn 英語の組み込み日付時刻形式をセルから解決する() {
    use rhodopsin::xlsx::{
        format::{FormatContext, Fragment, builtin, builtin_with_context, cell, cell_with_context},
        styles::Styles,
        workbook::{Cell, CellValue},
    };
    let context = FormatContext {
        date_system: DateSystem::Excel1900,
        locale: Locale::English,
    };
    for (id, expected) in [
        (14, "7/5/2011"),
        (15, "5-Jul-11"),
        (16, "5-Jul"),
        (17, "Jul-11"),
        (18, "12:00 PM"),
        (19, "12:00:00 PM"),
        (20, "12:00"),
        (21, "12:00:00"),
        (22, "7/5/2011 12:00"),
        (45, "00:00"),
        (46, "977508:00:00"),
        (47, "00:00.0"),
    ] {
        let xml = format!(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="{id}"/></cellXfs></styleSheet>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let styles = Styles::parse(doc.root_element()).unwrap();
        let value = Cell {
            address: "A1".into(),
            style: 0,
            value: CellValue::Number(40729.5),
            numeric_lexeme: Some("40729.5".into()),
        };
        assert_eq!(
            cell_with_context(&value, Some(&styles), context)
                .unwrap()
                .fragments,
            vec![Fragment::Text(expected.into())],
            "numFmtId={id}"
        );
        assert!(builtin(id).is_err());
        assert!(cell(&value, Some(&styles)).is_err());
        assert_eq!(
            builtin_with_context(
                id,
                FormatContext {
                    locale: Locale::Japanese,
                    ..context
                }
            )
            .unwrap_err()
            .reason,
            "builtin_date_locale"
        );
    }
    assert_eq!(builtin_with_context(4, context).unwrap(), "#,##0.00");
}

#[test]
fn 明示された書式コードを組み込み番号より優先する() {
    use rhodopsin::xlsx::{
        format::{FormatContext, Fragment, cell_with_context},
        styles::Styles,
        workbook::{Cell, CellValue},
    };
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="14" formatCode="yyyy年m月d日"/></numFmts><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="14"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let styles = Styles::parse(doc.root_element()).unwrap();
    let value = Cell {
        address: "A1".into(),
        style: 0,
        value: CellValue::Number(0.0),
        numeric_lexeme: Some("0".into()),
    };
    let result = cell_with_context(
        &value,
        Some(&styles),
        FormatContext {
            date_system: DateSystem::Excel1904,
            locale: Locale::Japanese,
        },
    )
    .unwrap();
    assert_eq!(
        result.fragments,
        vec![Fragment::Text("1904年1月1日".into())]
    );
}

#[test]
fn 日本語の組み込み形式を番号からセル表示へ接続する() {
    use rhodopsin::xlsx::{
        format::{FormatContext, Fragment, builtin, builtin_with_context, cell_with_context},
        styles::Styles,
        workbook::{Cell, CellValue},
    };
    let context = FormatContext {
        date_system: DateSystem::Excel1900,
        locale: Locale::Japanese,
    };
    for (id, expected) in [
        (27, "R1.5.1"),
        (28, "令和1年5月1日"),
        (29, "令和1年5月1日"),
        (30, "5/1/19"),
        (31, "2019年5月1日"),
        (32, "12時00分"),
        (33, "12時00分00秒"),
        (34, "2019年5月"),
        (35, "5月1日"),
        (36, "R1.5.1"),
        (50, "R1.5.1"),
        (51, "令和1年5月1日"),
        (52, "2019年5月"),
        (53, "5月1日"),
        (54, "令和1年5月1日"),
        (55, "2019年5月"),
        (56, "5月1日"),
        (57, "R1.5.1"),
        (58, "令和1年5月1日"),
    ] {
        let xml = format!(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="{id}"/></cellXfs></styleSheet>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let styles = Styles::parse(doc.root_element()).unwrap();
        for (raw, system) in [
            ("43586.5", DateSystem::Excel1900),
            ("42124.5", DateSystem::Excel1904),
        ] {
            let cell = Cell {
                address: "A1".into(),
                style: 0,
                value: CellValue::Number(raw.parse().unwrap()),
                numeric_lexeme: Some(raw.into()),
            };
            let display = cell_with_context(
                &cell,
                Some(&styles),
                FormatContext {
                    date_system: system,
                    ..context
                },
            )
            .unwrap();
            assert_eq!(
                display.fragments,
                vec![Fragment::Text(expected.into())],
                "numFmtId={id}"
            );
        }
        assert!(builtin(id).is_err());
        assert!(
            builtin_with_context(
                id,
                FormatContext {
                    locale: Locale::English,
                    ..context
                }
            )
            .is_err()
        );
    }
}

#[test]
fn 組み込み和暦でも元号境界を保持する() {
    use rhodopsin::xlsx::format::{FormatContext, builtin_with_context};
    let context = FormatContext {
        date_system: DateSystem::Excel1900,
        locale: Locale::Japanese,
    };
    for id in [28, 29, 51, 54, 58] {
        let format =
            DateFormat::parse(builtin_with_context(id, context).unwrap(), context.locale).unwrap();
        assert_eq!(
            format.format("43585", context.date_system).unwrap(),
            "平成31年4月30日"
        );
        assert_eq!(
            format.format("43586", context.date_system).unwrap(),
            "令和1年5月1日"
        );
    }
}

#[test]
fn 日付時刻の色指定を保持し重複と途中指定を拒否する() {
    use rhodopsin::xlsx::styles::ColorSource;
    for (code, rgb, expected) in [
        ("[Red]yyyy-mm-dd", [255, 0, 0], "2019-05-01"),
        ("[bLuE][$-411]ggge年m月d日", [0, 0, 255], "令和1年5月1日"),
        ("[Green][h]:mm", [0, 255, 0], "1046064:00"),
    ] {
        let format = DateFormat::parse(code, Locale::English).unwrap();
        assert_eq!(format.color().unwrap().source, ColorSource::Rgb(rgb));
        assert_eq!(
            format.format("43586", DateSystem::Excel1900).unwrap(),
            expected
        );
    }
    for code in ["[Red][Blue]yyyy", "yyyy[Red]", "\"文字\"[Red]yyyy"] {
        assert_eq!(
            DateFormat::parse(code, Locale::Japanese)
                .unwrap_err()
                .reason,
            "number_format_color_position"
        );
    }
    let literal = DateFormat::parse("\"[Red]\"yyyy", Locale::English).unwrap();
    assert!(literal.color().is_none());
    assert_eq!(
        literal.format("43586", DateSystem::Excel1900).unwrap(),
        "[Red]2019"
    );
}

#[test]
fn 大きい日付でも保存された小数の丸め境界を保持する() {
    for (raw, expected) in [
        ("2958464.000156249999999999", "9999-12-30 00:00:13"),
        ("2958464.00015625", "9999-12-30 00:00:14"),
        ("2958464.000156250000000001", "9999-12-30 00:00:14"),
        ("295846400015625e-8", "9999-12-30 00:00:14"),
    ] {
        assert_eq!(formatted("yyyy-mm-dd hh:mm:ss", raw), expected);
    }
    for (raw, expected) in [
        ("2958464.000000156249999999", "00:00:00.013"),
        ("2958464.00000015625", "00:00:00.014"),
        ("0.00000015625", "00:00:00.014"),
        ("1e-1024", "00:00:00.000"),
        ("-0.000", "00:00:00.000"),
    ] {
        assert_eq!(formatted("hh:mm:ss.000", raw), expected);
    }
}

#[test]
fn 時刻の繰り上がりと日付だけの切り捨てを分ける() {
    assert_eq!(
        formatted("yyyy-mm-dd hh:mm:ss", "59.9999999"),
        "1900-02-29 00:00:00"
    );
    assert_eq!(formatted("yyyy-mm-dd", "59.9999999"), "1900-02-28");
    assert_eq!(formatted("[h]:mm:ss", "59.9999999"), "1440:00:00");
    assert_eq!(
        formatted("yyyy-mm-dd", "2958465.999999999999999999"),
        "9999-12-31"
    );
    assert!(
        DateFormat::parse("yyyy-mm-dd hh:mm:ss", Locale::English)
            .unwrap()
            .format("2958465.9999999", DateSystem::Excel1900)
            .is_err()
    );
    for raw in ["2958466", "1e1024", "-1e-1024", "NaN"] {
        assert!(
            DateFormat::parse("yyyy-mm-dd", Locale::English)
                .unwrap()
                .format(raw, DateSystem::Excel1900)
                .is_err()
        );
    }
}

fn composite_display(
    code: &str,
    raw: Option<&str>,
) -> rhodopsin::xlsx::diagnostic::Result<rhodopsin::xlsx::format::DisplayValue> {
    use rhodopsin::xlsx::{
        format::{FormatContext, cell_with_context},
        styles::Styles,
        workbook::{Cell, CellValue},
    };
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="164" formatCode="0"/></numFmts><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="164"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let mut styles = Styles::parse(doc.root_element()).unwrap();
    styles.number_formats.insert(164, code.into());
    cell_with_context(
        &Cell {
            address: "A1".into(),
            style: 0,
            value: raw.map_or_else(
                || CellValue::Text("入力".into()),
                |raw| CellValue::Number(raw.parse().unwrap()),
            ),
            numeric_lexeme: raw.map(Into::into),
        },
        Some(&styles),
        FormatContext {
            date_system: DateSystem::Excel1900,
            locale: Locale::Japanese,
        },
    )
}

#[test]
fn 日付数値ゼロ文字列の区分を混在させる() {
    use rhodopsin::xlsx::{format::Fragment, styles::ColorSource};
    let code = "[Blue]yyyy-mm-dd;[Red](0);\"なし\";[Green]\"注記;\"@";
    for (raw, expected, color) in [
        (Some("43586"), "2019-05-01", Some([0, 0, 255])),
        (Some("-12"), "(12)", Some([255, 0, 0])),
        (Some("0"), "なし", None),
        (None, "注記;入力", Some([0, 255, 0])),
    ] {
        let display = composite_display(code, raw).unwrap();
        assert_eq!(display.fragments, vec![Fragment::Text(expected.into())]);
        assert_eq!(
            display.color.map(|color| color.source),
            color.map(ColorSource::Rgb)
        );
    }
    assert_eq!(
        composite_display("yyyy-mm-dd;;;", None).unwrap().fragments,
        vec![]
    );
    assert_eq!(
        composite_display("yyyy-mm-dd;0", None).unwrap().fragments,
        vec![Fragment::Text("入力".into())]
    );
    assert_eq!(
        composite_display("yyyy\\;mm;0", Some("43586"))
            .unwrap()
            .fragments,
        vec![Fragment::Text("2019;05".into())]
    );
}

#[test]
fn 日付区分の条件も丸め前の値で評価する() {
    use rhodopsin::xlsx::format::Fragment;
    let code = "[>=43586][Blue]yyyy-mm-dd;[<0][Red](0);0";
    for (raw, expected) in [
        ("43586", "2019-05-01"),
        ("43585.99", "43586"),
        ("-2", "(2)"),
    ] {
        assert_eq!(
            composite_display(code, Some(raw)).unwrap().fragments,
            vec![Fragment::Text(expected.into())]
        );
    }
    for code in [
        "[>0][<10]yyyy;0",
        "yyyy;[<0]0",
        "[>0]yyyy;0;0",
        "yyyy;0;[>0]0",
        "[h][>0];0",
        "yyyy;[DBNum4]0",
        "yyyy;0;0;yyyy",
        "yyyy;0;0;@;0",
        "yyyy\"未完",
    ] {
        assert!(composite_display(code, Some("43586")).is_err(), "{code}");
    }
    assert!(composite_display("yyyy;yyyy", Some("-1")).is_err());
}

#[test]
fn セルのロケールを日付とdbnumの混在区分へ渡す() {
    use rhodopsin::xlsx::format::Fragment;
    for (code, raw, expected) in [
        ("[DBNum1]0", "123", "一二三"),
        ("[>=40000]yyyy-mm-dd;[DBNum3]0000", "43586", "2019-05-01"),
        ("[>=40000]yyyy-mm-dd;[DBNum3]0000", "12", "００１２"),
        ("[>=40000][DBNum2]0;yyyy-mm-dd", "43586", "四参伍八六"),
        ("[>=40000][DBNum2]0;yyyy-mm-dd", "1", "1900-01-01"),
    ] {
        assert_eq!(
            composite_display(code, Some(raw)).unwrap().fragments,
            vec![Fragment::Text(expected.into())]
        );
    }
    assert!(composite_display("yyyy;[DBNum1][$-409]0", Some("43586")).is_err());
    // 非選択区分でも対応外の書式を黙って受け入れない。
    assert!(composite_display("yyyy;[DBNum1]0.0", Some("43586")).is_err());
}

#[test]
fn 日付のゼロ埋めlcidを同じ言語へ解決する() {
    for width in 3..=8 {
        let japanese = format!("{:0width$x}", 0x411);
        let english = format!("{:0width$x}", 0x409);
        assert_eq!(
            formatted(&format!("[$-{japanese}]ggge年m月d日"), "43586"),
            "令和1年5月1日"
        );
        assert_eq!(
            DateFormat::parse(&format!("[$-{english}]mmmm d yyyy"), Locale::Japanese)
                .unwrap()
                .format("43586", DateSystem::Excel1900)
                .unwrap(),
            "May 1 2019"
        );
        let code = format!("[>=40000][$-{japanese}]ggge年m月d日;[DBNum3][$-{japanese}]0000");
        for (raw, expected) in [("43586", "令和1年5月1日"), ("12", "００１２")] {
            assert_eq!(
                composite_display(&code, Some(raw)).unwrap().fragments,
                vec![rhodopsin::xlsx::format::Fragment::Text(expected.into())]
            );
        }
    }
}

#[test]
fn 日付ロケールの重複と未対応上位ビットを推測しない() {
    for code in ["[$-411][$-0411]yyyy", "[$-411]yyyy[$-409]"] {
        assert_eq!(
            DateFormat::parse(code, Locale::English).unwrap_err().reason,
            "duplicate_number_format_locale"
        );
    }
    for id in [
        "",
        "41",
        "000000411",
        "+411",
        "0x411",
        "４１１",
        "411 ",
        "40411",
        "10411",
        "F800",
        "FFFF0411",
        "407",
    ] {
        assert!(
            DateFormat::parse(&format!("[$-{id}]yyyy"), Locale::Japanese).is_err(),
            "{id}"
        );
    }
}

#[test]
fn dbnum3の日付数字を全角にして固定文字を保持する() {
    for (code, raw, expected) in [
        ("yyyy-mm-dd", "43586", "２０１９-０５-０１"),
        ("yy/m/d", "60", "００/２/２９"),
        ("yyyy-mm-dd", "0", "１９００-０１-００"),
        ("yyyy-mm-dd", "2958465", "９９９９-１２-３１"),
        ("ggge年m月d日(aaaa)", "43586", "令和１年５月１日(水曜日)"),
        ("g ee/m/d", "43585", "H ３１/４/３０"),
        ("mmmm d yyyy", "43586", "５月 １ ２０１９"),
        (
            "\"ID2: \"yyyy\\1/mm/dd",
            "43586",
            "ID2: ２０１９1/０５/０１",
        ),
        (
            "hh:mm:ss.000",
            "0.00000578703703703704",
            "００:００:００.５００",
        ),
        ("[h]:mm:ss", "1.5", "３６:００:００"),
        ("h:mm AM/PM", "0.5", "１２:００ PM"),
        (
            "yyyy-mm-dd hh:mm:ss",
            "59.9999999",
            "１９００-０２-２９ ００:００:００",
        ),
    ] {
        let format = DateFormat::parse(&format!("[DBNum3]{code}"), Locale::Japanese).unwrap();
        assert_eq!(
            format.format(raw, DateSystem::Excel1900).unwrap(),
            expected,
            "{code}"
        );
    }
    let format = DateFormat::parse("[dbnum3][$-00000411]yyyy-mm-dd", Locale::English).unwrap();
    assert_eq!(
        format.format("0", DateSystem::Excel1904).unwrap(),
        "１９０４-０１-０１"
    );
}

#[test]
fn dbnum3の日付と数値を条件で選び色と文字区分を保持する() {
    use rhodopsin::xlsx::{format::Fragment, styles::ColorSource};
    for prefix in ["[Red][>=40000][DBNum3]", "[DBNum3][>=40000][Red]"] {
        let code = format!("{prefix}yyyy-mm-dd;[>0][DBNum3]0000;;\"注記1:\"@");
        let display = composite_display(&code, Some("43586")).unwrap();
        assert_eq!(
            display.fragments,
            vec![Fragment::Text("２０１９-０５-０１".into())]
        );
        assert_eq!(display.color.unwrap().source, ColorSource::Rgb([255, 0, 0]));
        assert_eq!(
            composite_display(&code, Some("12")).unwrap().fragments,
            vec![Fragment::Text("００１２".into())]
        );
        assert_eq!(
            composite_display(&code, None).unwrap().fragments,
            vec![Fragment::Text("注記1:入力".into())]
        );
    }
}

#[test]
fn 日付dbnumの未対応指定と誤った位置を拒否する() {
    for code in [
        "[DBNum1]yyyy",
        "[DBNum2]yyyy",
        "[DBNum4]yyyy",
        "[DBNum3][DBNum3]yyyy",
        "yyyy[DBNum3]",
        "[DBNum3][$-409]yyyy",
        "[$-409][DBNum3]yyyy",
        "[DBNum3][$-411][$-411]yyyy",
    ] {
        assert!(DateFormat::parse(code, Locale::Japanese).is_err(), "{code}");
        // 選択されない区分にあっても不正・未対応を隠さない。
        assert!(
            composite_display(&format!("[>=0]0;{code}"), Some("1")).is_err(),
            "{code}"
        );
    }
    assert!(DateFormat::parse("[DBNum3]yyyy", Locale::English).is_err());
    assert!(composite_display("[DBNum3][>0][<10]yyyy;0", Some("1")).is_err());
    let format = DateFormat::parse("[DBNum3]yyyy", Locale::Japanese).unwrap();
    for raw in ["-1", "NaN", "2958466"] {
        assert!(format.format(raw, DateSystem::Excel1900).is_err());
    }
}
