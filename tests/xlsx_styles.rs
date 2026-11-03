use rhodopsin::xlsx::styles::{
    Color, ColorSource, Fill, FontScheme, Horizontal, LineStyle, Styles, Underline, Vertical,
};

#[test]
fn 空のスライサー・タイムラインスタイル既定名だけを受理する() {
    let declaration = r#"<ext uri="{EB79DEF2-80B8-43e5-95BD-54CBDDF9020C}"><x:slicerStyles xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" defaultSlicerStyle="SlicerStyleLight1"/></ext>"#;
    // Excel for Mac 16.105 が保存するブックの宣言。
    let timeline = r#"<ext uri="{9260A510-F301-46a8-8635-F512D64BE5F5}"><x15:timelineStyles xmlns:x15="http://schemas.microsoft.com/office/spreadsheetml/2010/11/main" defaultTimelineStyle="TimeSlicerStyleLight1"/></ext>"#;
    let wrap = |body: &str| {
        format!(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><extLst>{body}</extLst></styleSheet>"#
        )
    };
    for body in [
        String::new(),
        declaration.into(),
        timeline.into(),
        format!("{declaration}{timeline}"),
    ] {
        let xml = wrap(&body);
        assert!(Styles::parse(roxmltree::Document::parse(&xml).unwrap().root_element()).is_ok());
    }
    for body in [
        declaration.replace("EB79DEF2", "00000000"),
        declaration.replace("2009/9/main", "2099/9/main"),
        declaration.replace(
            r#"defaultSlicerStyle="SlicerStyleLight1""#,
            r#"defaultSlicerStyle="" "#,
        ),
        declaration.replace(r#" defaultSlicerStyle="SlicerStyleLight1""#, ""),
        declaration.replace("/>", r#" unknown="機密"/>"#),
        declaration.replace("/>", "><x:slicerStyle/></x:slicerStyles>"),
        declaration.replace("/>", ">機密</x:slicerStyles>"),
        format!("{declaration}{declaration}"),
        declaration.replace("</ext>", "<unknown/></ext>"),
        format!("{timeline}{timeline}"),
        timeline.replace("timelineStyles", "slicerStyles"),
        timeline.replace("2010/11/main", "2009/9/main"),
        timeline.replace(r#" defaultTimelineStyle="TimeSlicerStyleLight1""#, ""),
    ] {
        let xml = wrap(&body);
        let error =
            Styles::parse(roxmltree::Document::parse(&xml).unwrap().root_element()).unwrap_err();
        assert_eq!(error.reason, "stylesheet_extension");
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
}

fn styles(font: &str, fill: &str, border: &str, xf: &str, extra: &str) -> Styles {
    let xml = format!(
        r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font>{font}</font></fonts><fills><fill>{fill}</fill></fills><borders><border>{border}</border></borders><cellXfs><xf fontId="0" fillId="0" borderId="0">{xf}</xf></cellXfs>{extra}</styleSheet>"#
    );
    Styles::parse(roxmltree::Document::parse(&xml).unwrap().root_element()).unwrap()
}

#[test]
fn 字体と塗りと罫線と配置を保持する() {
    let s = styles(
        r#"<name val="游ゴシック"/><sz val="11"/><b/><i val="true"/><u val="doubleAccounting"/><color theme="1" tint="0.25"/><scheme val="minor"/>"#,
        r#"<patternFill patternType="solid"><fgColor rgb="FF123456"/></patternFill>"#,
        r#"<left style="double"><color rgb="FF000000"/></left><bottom style="dashed"/>"#,
        r#"<alignment horizontal="right" vertical="center" wrapText="1" indent="2"/>"#,
        "",
    );
    let xf = s.validate_cell(0).unwrap();
    let font = s.fonts[0].as_ref().unwrap();
    assert_eq!(font.family.as_deref(), Some("游ゴシック"));
    assert_eq!(font.size, Some(11.0));
    assert!(font.bold && font.italic);
    assert_eq!(font.underline, Underline::DoubleAccounting);
    assert_eq!(font.scheme, FontScheme::Minor);
    assert_eq!(font.color.source, ColorSource::Theme(1));
    assert_eq!(font.color.tint, 0.25);
    assert!(matches!(
        &s.fills[0],
        Ok(Fill::Solid(Color {
            source: ColorSource::Rgb([0x12, 0x34, 0x56]),
            ..
        }))
    ));
    let border = s.borders[0].as_ref().unwrap();
    assert_eq!(border.left.as_ref().unwrap().style, LineStyle::Double);
    assert_eq!(border.bottom.as_ref().unwrap().style, LineStyle::Dashed);
    assert_eq!(xf.alignment.horizontal, Horizontal::Right);
    assert_eq!(xf.alignment.vertical, Vertical::Center);
    assert!(xf.alignment.wrap);
    assert_eq!(xf.alignment.indent, 2);
}

#[test]
fn 対応外のセル配置を検出する() {
    for (attr, reason) in [
        ("textRotation=\"45\"", "rotated_text"),
        ("textRotation=\"255\"", "vertical_text"),
        ("shrinkToFit=\"true\"", "shrink_to_fit"),
    ] {
        let s = styles("", "", "", &format!("<alignment {attr}/>"), "");
        assert_eq!(s.validate_cell(0).unwrap_err().reason, reason);
    }
}

#[test]
fn グラデーションと使用される網掛けを拒否する() {
    for (fill, reason) in [
        ("<gradientFill/>", "gradient_fill"),
        (r#"<patternFill patternType="gray125"/>"#, "pattern_fill"),
    ] {
        let s = styles("", fill, "", "", "");
        assert_eq!(s.validate_cell(0).unwrap_err().reason, reason);
    }
}

#[test]
fn 色指定の衝突と範囲外を拒否する() {
    for attr in [
        r#"theme="12""#,
        r#"theme="1" rgb="FF000000""#,
        r#"rgb="機密文字列""#,
        r#"indexed="66""#,
        r#"theme="1" tint="NaN""#,
        r#"auto="false""#,
    ] {
        let s = styles(&format!("<color {attr}/>"), "", "", "", "");
        let e = s.validate_cell(0).unwrap_err();
        assert!(!serde_json::to_string(&e).unwrap().contains("機密"));
    }
}

#[test]
fn 基底スタイルへの参照と適用フラグを読む() {
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/><font><b/></font></fonts><fills><fill/></fills><borders><border/></borders><cellStyleXfs><xf fontId="1"><alignment horizontal="center"/></xf></cellStyleXfs><cellXfs><xf xfId="0" fontId="0" applyFont="0" applyAlignment="0"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let s = Styles::parse(doc.root_element()).unwrap();
    let cell = s.validate_cell(0).unwrap();
    assert_eq!(cell.font, 1);
    assert_eq!(cell.alignment.horizontal, Horizontal::Center);
}

#[test]
fn 無効なスタイル参照を拒否する() {
    let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf fontId="9"/><xf numFmtId="164"/></cellXfs></styleSheet>"#;
    let doc = roxmltree::Document::parse(xml).unwrap();
    let s = Styles::parse(doc.root_element()).unwrap();
    assert_eq!(s.validate_cell(0).unwrap_err().reason, "font_style_index");
    assert_eq!(
        s.validate_cell(1).unwrap_err().reason,
        "number_format_missing"
    );
    assert_eq!(s.validate_cell(99).unwrap_err().reason, "cell_style_index");
}

#[test]
fn テーマの色順序と日本語書体を保持する() {
    let colors = [
        "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
        "accent6", "hlink", "folHlink",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!(r#"<a:{n}><a:srgbClr val="{i:02X}0000"/></a:{n}>"#))
    .collect::<String>();
    let fonts = r#"<a:latin typeface="Aptos"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Jpan" typeface="游ゴシック"/>"#;
    let xml = format!(
        r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme>{colors}</a:clrScheme><a:fontScheme><a:majorFont>{fonts}</a:majorFont><a:minorFont>{fonts}</a:minorFont></a:fontScheme></a:themeElements></a:theme>"#
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let theme = rhodopsin::xlsx::theme::Theme::parse(doc.root_element()).unwrap();
    assert_eq!(theme.colors[0], [1, 0, 0]);
    assert_eq!(theme.colors[1], [0, 0, 0]);
    assert_eq!(theme.colors[2], [3, 0, 0]);
    assert_eq!(theme.minor.scripts["Jpan"], "游ゴシック");
}

#[test]
fn 表示形式のエスケープを一度だけ復号してセル表示へ渡す() {
    use rhodopsin::xlsx::{
        format::{Fragment, cell},
        workbook::{Cell, CellValue},
    };
    for (code, expected) in [
        ("_x0022__x00A5__x0022_#,##0", "¥1,234"),
        ("&quot;_x005F_x0041_&quot;0", "_x0041_1234"),
        ("&quot;_xD83D__xDE00_&quot;0", "😀1234"),
        ("&quot;&amp;_x0024_&quot;0", "&$1234"),
    ] {
        let xml = format!(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="164" formatCode="{code}"/></numFmts><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="164"/></cellXfs></styleSheet>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let styles = Styles::parse(doc.root_element()).unwrap();
        let value = Cell {
            address: "A1".into(),
            style: 0,
            value: CellValue::Number(1234.0),
            numeric_lexeme: Some("1234".into()),
        };
        assert_eq!(
            cell(&value, Some(&styles)).unwrap().fragments,
            vec![Fragment::Text(expected.into())]
        );
    }
}

#[test]
fn 不正な表示形式文字列を内容を漏らさず拒否する() {
    for (code, reason) in [
        ("_xD800_機密".to_owned(), "invalid_string_surrogate"),
        ("0".repeat(4097), "number_format_length"),
        ("_x0030_".repeat(4097), "number_format_length"),
    ] {
        let xml = format!(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="164" formatCode="{code}"/></numFmts></styleSheet>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let error = Styles::parse(doc.root_element()).unwrap_err();
        assert_eq!(error.reason, reason);
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
    let code = "_x0030_".repeat(4096);
    let xml = format!(
        r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts><numFmt numFmtId="164" formatCode="{code}"/></numFmts></styleSheet>"#
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    assert_eq!(
        Styles::parse(doc.root_element()).unwrap().number_formats[&164],
        "0".repeat(4096)
    );
}

#[test]
fn 標準スタイルの重複と無効な参照を拒否する() {
    for (named, base, reason) in [
        (
            r#"<cellStyle builtinId="0" xfId="0"/><cellStyle builtinId="0" xfId="0"/>"#,
            r#"<xf/>"#,
            "duplicate_normal_style",
        ),
        (
            r#"<cellStyle builtinId="0"/>"#,
            r#"<xf/>"#,
            "normal_style_xf_missing",
        ),
        (
            r#"<cellStyle builtinId="0" xfId="2"/>"#,
            r#"<xf/>"#,
            "base_style_index",
        ),
        (
            r#"<cellStyle builtinId="0" xfId="0"/>"#,
            r#"<xf fontId="2"/>"#,
            "font_style_index",
        ),
    ] {
        let xml = format!(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/></fonts><cellStyleXfs>{base}</cellStyleXfs><cellStyles>{named}</cellStyles></styleSheet>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        assert_eq!(
            Styles::parse(doc.root_element()).unwrap_err().reason,
            reason
        );
    }
}

#[test]
fn 地域依存の番号で保存書式がない場合は理由を明示する() {
    for id in [5, 6, 7, 8, 23, 26, 41, 42, 43, 44, 63, 66] {
        let xml = format!(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font/></fonts><fills><fill/></fills><borders><border/></borders><cellXfs><xf numFmtId="{id}"/></cellXfs></styleSheet>"#
        );
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let mut styles = Styles::parse(doc.root_element()).unwrap();
        assert_eq!(
            styles.validate_cell(0).unwrap_err().reason,
            "locale_number_format_missing"
        );
        styles.number_formats.insert(id, "¥#,##0".into());
        assert!(styles.validate_cell(0).is_ok());
    }
}
