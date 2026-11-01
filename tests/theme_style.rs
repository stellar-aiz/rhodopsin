//! Shape style references (`p:style`): theme fills, effects, and text defaults (#130).
mod support;
use support::*;

/// A theme whose third fill style is a vertical gradient of the placeholder colour, whose
/// second effect style is an opaque 8 px shadow to the right, and whose minor font is the
/// test metrics font.
const THEME: &str = r#"<a:themeElements><a:clrScheme name="t"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:accent1><a:srgbClr val="0000FF"/></a:accent1></a:clrScheme><a:fontScheme name="t"><a:majorFont><a:latin typeface="Rhodopsin Metrics Test"/><a:ea typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Rhodopsin Metrics Test"/><a:ea typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="t"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="20000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"/></a:gs></a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill></a:fillStyleLst><a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw blurRad="0" dist="76200" dir="0" rotWithShape="0"><a:srgbClr val="000000"/></a:outerShdw></a:effectLst></a:effectStyle><a:effectStyle><a:effectLst/><a:scene3d><a:camera prst="orthographicFront"/><a:lightRig rig="threePt" dir="t"/></a:scene3d><a:sp3d/></a:effectStyle></a:effectStyleLst></a:fmtScheme></a:themeElements>"#;

/// A 60x40 px shape at (10, 10) with the given style references and text body.
fn styled(style: &str, body: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst="rect"/></p:spPr><p:style>{style}</p:style>{body}</p:sp>"#,
        transform(10, 10, 60, 40)
    )
}
/// Text "A" (a full-width rectangle glyph in the metrics font) at 24 pt with no inset.
fn text(run_props: &str) -> String {
    format!(
        r#"<p:txBody><a:bodyPr lIns="0" tIns="0" rIns="0" bIns="0" anchor="t"/><a:lstStyle/><a:p><a:r><a:rPr sz="2400">{run_props}</a:rPr><a:t>A</a:t></a:r></a:p></p:txBody>"#
    )
}
const DEFAULT_STYLE: &str = r#"<a:lnRef idx="0"><a:schemeClr val="accent1"/></a:lnRef><a:fillRef idx="3"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="2"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef>"#;

/// Render one slide under a master whose `otherStyle` colours text green.
fn render(shapes: &str) -> (resvg::tiny_skia::Pixmap, serde_json::Value) {
    let d = tempfile::tempdir().unwrap();
    let input = d.path().join("style.pptx");
    let out = d.path().join("out");
    let theme = format!(r#"<a:theme xmlns:a="{A}">{THEME}</a:theme>"#);
    let master = format!(
        r#"<p:sldMaster xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}"><p:cSld><p:spTree/></p:cSld><p:txStyles><p:titleStyle/><p:bodyStyle><a:lvl1pPr><a:defRPr><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:defRPr></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:defRPr></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"#
    );
    write_deck(
        &input,
        page("sld", "", "", shapes),
        vec![
            (
                "ppt/slides/_rels/slide1.xml.rels",
                rel("slideLayout", "../layouts/layout.xml").into_bytes(),
            ),
            (
                "ppt/layouts/layout.xml",
                page("sldLayout", "", "", "").into_bytes(),
            ),
            (
                "ppt/layouts/_rels/layout.xml.rels",
                rel("slideMaster", "../masters/master.xml").into_bytes(),
            ),
            ("ppt/masters/master.xml", master.into_bytes()),
            (
                "ppt/masters/_rels/master.xml.rels",
                rel("theme", "../theme/theme.xml").into_bytes(),
            ),
            ("ppt/theme/theme.xml", theme.into_bytes()),
        ],
    );
    check(run(&input, &out, false));
    let manifest =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
    (image(&out), manifest)
}
fn messages(manifest: &serde_json::Value) -> Vec<String> {
    manifest["slides"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["message"].as_str().unwrap().to_owned())
        .collect()
}
const WHITE: (u8, u8, u8) = (255, 255, 255);

#[test]
fn gradient_fill_reference_is_drawn_with_the_reference_colour() {
    let (pixels, manifest) = render(&styled(DEFAULT_STYLE, ""));
    // Vertical gradient from a tint of blue (light) at the top to blue at the bottom.
    let top = rgb(&pixels, 60, 13);
    let bottom = rgb(&pixels, 60, 47);
    assert!(top.2 == 255 && top.0 > 150, "{top:?}");
    assert!(bottom.2 == 255 && bottom.0 < 30, "{bottom:?}");
    assert!(top.0 > bottom.0 + 100, "{top:?} {bottom:?}");
    let messages = messages(&manifest);
    assert!(
        !messages
            .iter()
            .any(|m| m.contains("unresolved") || m.contains("style references")),
        "{messages:?}"
    );
}

#[test]
fn font_reference_colour_ranks_above_the_master_and_below_the_run() {
    // No explicit colour: the style's `lt1` (white) wins over the master's green.
    let (pixels, manifest) = render(&styled(DEFAULT_STYLE, &text("")));
    assert_eq!(rgb(&pixels, 14, 20), WHITE, "{:?}", messages(&manifest));
    // An explicit run colour wins over the style.
    let (pixels, _) = render(&styled(
        DEFAULT_STYLE,
        &text(r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#),
    ));
    assert_eq!(rgb(&pixels, 14, 20), (255, 0, 0));
    // Without a style, the master's colour applies as before.
    let (pixels, _) = render(&styled("", &text("")).replace("<p:style></p:style>", ""));
    assert_eq!(rgb(&pixels, 14, 20), (0, 255, 0));
    // A placeholder keeps its inherited master style (bodyStyle: green) above the reference.
    let placeholder = styled(DEFAULT_STYLE, &text("")).replace(
        "<p:nvPr/>",
        r#"<p:nvPr><p:ph type="body" idx="1"/></p:nvPr>"#,
    );
    let (pixels, _) = render(&placeholder);
    assert_eq!(rgb(&pixels, 14, 20), (0, 255, 0));
}

#[test]
fn font_reference_selects_the_theme_font() {
    // The minor theme font is the loaded metrics font, so no substitution is reported for the
    // styled shape; a shape without a style requests sans-serif and is substituted.
    let (_, manifest) = render(&styled(DEFAULT_STYLE, &text("")));
    assert!(
        !messages(&manifest)
            .iter()
            .any(|m| m.contains("Font substitution")),
        "{:?}",
        messages(&manifest)
    );
    let (_, manifest) = render(&styled("", &text("")).replace("<p:style></p:style>", ""));
    assert!(
        messages(&manifest)
            .iter()
            .any(|m| m.contains("Font substitution"))
    );
}

#[test]
fn effect_reference_draws_the_theme_shadow_unless_the_shape_has_its_own() {
    let (pixels, manifest) = render(&styled(DEFAULT_STYLE, ""));
    // The theme's second effect style is an opaque black shadow 8 px to the right.
    assert_eq!(rgb(&pixels, 74, 30), (0, 0, 0), "{:?}", messages(&manifest));
    assert!(
        !messages(&manifest)
            .iter()
            .any(|m| m.contains("approximated"))
    );
    // Index 0 means no effects.
    let (pixels, _) = render(&styled(
        &DEFAULT_STYLE.replace("effectRef idx=\"2\"", "effectRef idx=\"0\""),
        "",
    ));
    assert_eq!(rgb(&pixels, 74, 30), WHITE);
    // A shape with its own effect list ignores the reference (shadow downwards instead).
    let own = styled(DEFAULT_STYLE, "").replace(
        "<a:prstGeom prst=\"rect\"/>",
        r#"<a:prstGeom prst="rect"/><a:effectLst><a:outerShdw blurRad="0" dist="76200" dir="5400000"><a:srgbClr val="000000"/></a:outerShdw></a:effectLst>"#,
    );
    let (pixels, _) = render(&own);
    assert_eq!(rgb(&pixels, 74, 30), WHITE);
    assert_eq!(rgb(&pixels, 40, 54), (0, 0, 0));
}

#[test]
fn unresolved_references_are_reported_and_leave_the_shape_unfilled() {
    let (pixels, manifest) = render(&styled(
        &DEFAULT_STYLE
            .replace("fillRef idx=\"3\"", "fillRef idx=\"9\"")
            .replace("effectRef idx=\"2\"", "effectRef idx=\"7\""),
        "",
    ));
    assert_eq!(rgb(&pixels, 40, 30), WHITE);
    let messages = messages(&manifest);
    assert!(
        messages
            .iter()
            .any(|m| m == "Theme fill reference 9 is unresolved"),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m == "Theme effect reference 7 is unresolved"),
        "{messages:?}"
    );
}
