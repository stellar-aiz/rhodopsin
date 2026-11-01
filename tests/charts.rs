//! Charts drawn from the values cached in independently written chart parts (#115).
mod support;
use support::*;

const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

/// A 100x100 px frame holding the chart part `ppt/charts/chart1.xml`.
fn frame() -> String {
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="5" name="Chart 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>{}<a:graphic><a:graphicData uri="{C}"><c:chart xmlns:c="{C}" r:id="r1"/></a:graphicData></a:graphic></p:graphicFrame>"#,
        transform(0, 0, 100, 100).replace("a:xfrm", "p:xfrm")
    )
}
/// A chart part whose plot area is pinned to (10, 10, 80, 80) px with both axes hidden.
fn chart(plot: &str, scale: &str, extra: &str) -> String {
    format!(
        r#"<c:chartSpace xmlns:c="{C}" xmlns:a="{A}" xmlns:r="{R}"><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout><c:manualLayout><c:layoutTarget val="inner"/><c:xMode val="edge"/><c:yMode val="edge"/><c:x val="0.1"/><c:y val="0.1"/><c:w val="0.8"/><c:h val="0.8"/></c:manualLayout></c:layout>{plot}<c:catAx><c:axId val="1"/><c:delete val="1"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling>{scale}</c:scaling><c:delete val="1"/><c:crossAx val="1"/></c:valAx></c:plotArea></c:chart>{extra}</c:chartSpace>"#
    )
}
fn series(color: &str, values: &[f64], extra: &str) -> String {
    let points: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#))
        .collect();
    let categories: String = (0..values.len())
        .map(|i| format!(r#"<c:pt idx="{i}"><c:v>C{i}</c:v></c:pt>"#))
        .collect();
    format!(
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>Secret series</c:v></c:tx><c:spPr><a:solidFill><a:srgbClr val="{color}"/></a:solidFill><a:ln w="57150"><a:solidFill><a:srgbClr val="{color}"/></a:solidFill></a:ln></c:spPr>{extra}<c:cat><c:strLit><c:ptCount val="{n}"/>{categories}</c:strLit></c:cat><c:val><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="{n}"/>{points}</c:numLit></c:val></c:ser>"#,
        n = values.len()
    )
}
const SAVED: &str = r#"<c:orientation val="minMax"/><c:max val="100"/><c:min val="0"/>"#;
const UNIT: &str = r#"<c:majorUnit val="50"/>"#;

fn render(part: String, slide_rels: &str) -> (resvg::tiny_skia::Pixmap, String, String) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("chart.pptx");
    let out = d.path().join("out");
    write_deck(
        &p,
        page("sld", "", "", &frame()),
        vec![
            (
                "ppt/slides/_rels/slide1.xml.rels",
                slide_rels.as_bytes().to_vec(),
            ),
            ("ppt/charts/chart1.xml", part.into_bytes()),
        ],
    );
    let output = run(&p, &out, false);
    check(output);
    (
        image(&out),
        std::fs::read_to_string(out.join("manifest.json")).unwrap(),
        String::new(),
    )
}
fn chart_rels() -> String {
    rel("chart", "../charts/chart1.xml")
}
fn red(pixel: (u8, u8, u8)) -> bool {
    pixel.0 > 200 && pixel.1 < 60 && pixel.2 < 60
}
fn blue(pixel: (u8, u8, u8)) -> bool {
    pixel.2 > 200 && pixel.0 < 60 && pixel.1 < 60
}
const WHITE: (u8, u8, u8) = (255, 255, 255);
/// The plot area's axis scale with the saved major unit inserted where Office writes it.
fn with_unit(part: String) -> String {
    part.replace(
        "<c:delete val=\"1\"/><c:crossAx val=\"1\"/>",
        &format!("<c:delete val=\"1\"/><c:crossAx val=\"1\"/>{UNIT}"),
    )
}

#[test]
fn clustered_columns_follow_the_saved_layout_and_scale() {
    // Two categories in an 80 px plot: 40 px bands, gap 100% gives 20 px columns centred
    // in each band. 50 reaches y=50 and 100 reaches y=10 on the saved 0..100 scale.
    let plot = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>{}<c:gapWidth val="100"/><c:axId val="1"/><c:axId val="2"/></c:barChart>"#,
        series("FF0000", &[50.0, 100.0], "")
    );
    let (pixels, manifest, _) = render(with_unit(chart(&plot, SAVED, "")), &chart_rels());
    assert!(red(rgb(&pixels, 30, 70)), "{:?}", rgb(&pixels, 30, 70));
    assert_eq!(rgb(&pixels, 30, 40), WHITE);
    assert_eq!(rgb(&pixels, 15, 70), WHITE);
    assert!(red(rgb(&pixels, 70, 20)), "{:?}", rgb(&pixels, 70, 20));
    assert_eq!(rgb(&pixels, 50, 70), WHITE);
    assert!(!manifest.contains("non-table graphicFrame"), "{manifest}");
    assert!(
        manifest.contains("Chart layout is approximated"),
        "{manifest}"
    );
    assert!(
        !manifest.contains("axis scale is approximated"),
        "{manifest}"
    );
    assert!(!manifest.contains("Secret series"), "{manifest}");
}

#[test]
fn a_missing_saved_scale_is_reported() {
    let plot = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart>"#,
        series("FF0000", &[50.0, 100.0], "")
    );
    let (_, manifest, _) = render(
        chart(&plot, r#"<c:orientation val="minMax"/>"#, ""),
        &chart_rels(),
    );
    assert!(
        manifest.contains("Chart axis scale is approximated"),
        "{manifest}"
    );
}

#[test]
fn stacked_bars_stack_along_the_horizontal_value_axis() {
    // One category: the 80 px band holds a 40 px bar (gap 100%) between y=30 and y=70.
    // 40 then 60 on the 0..100 scale end at x=42 and x=90.
    let plot = format!(
        r#"<c:barChart><c:barDir val="bar"/><c:grouping val="stacked"/>{}{}<c:gapWidth val="100"/><c:overlap val="100"/><c:axId val="1"/><c:axId val="2"/></c:barChart>"#,
        series("FF0000", &[40.0], ""),
        series("0000FF", &[60.0], "").replace(
            "<c:idx val=\"0\"/><c:order val=\"0\"/>",
            "<c:idx val=\"1\"/><c:order val=\"1\"/>"
        )
    );
    let (pixels, _, _) = render(with_unit(chart(&plot, SAVED, "")), &chart_rels());
    assert!(red(rgb(&pixels, 30, 50)), "{:?}", rgb(&pixels, 30, 50));
    assert!(blue(rgb(&pixels, 70, 50)), "{:?}", rgb(&pixels, 70, 50));
    assert_eq!(rgb(&pixels, 30, 20), WHITE);
}

#[test]
fn lines_join_category_centres() {
    // Points at the band centres (30, 90) and (70, 10); the midpoint (50, 50) is on the line.
    let plot = format!(
        r#"<c:lineChart><c:grouping val="standard"/>{}<c:marker val="1"/><c:axId val="1"/><c:axId val="2"/></c:lineChart>"#,
        series(
            "0000FF",
            &[0.0, 100.0],
            r#"<c:marker><c:symbol val="none"/></c:marker>"#
        )
    );
    let (pixels, manifest, _) = render(with_unit(chart(&plot, SAVED, "")), &chart_rels());
    assert!(blue(rgb(&pixels, 50, 50)), "{:?}", rgb(&pixels, 50, 50));
    assert_eq!(rgb(&pixels, 70, 70), WHITE);
    assert!(!manifest.contains("marker"), "{manifest}");
}

#[test]
fn pie_wedges_start_at_twelve_and_run_clockwise() {
    let plot = format!(
        r#"<c:pieChart><c:varyColors val="1"/>{}<c:firstSliceAng val="0"/></c:pieChart>"#,
        series(
            "00FF00",
            &[1.0, 1.0],
            r#"<c:dPt><c:idx val="0"/><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr></c:dPt><c:dPt><c:idx val="1"/><c:spPr><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></c:spPr></c:dPt>"#
        )
    );
    let part = chart(&plot, "", "").replace(
        r#"<c:catAx><c:axId val="1"/><c:delete val="1"/><c:crossAx val="2"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling></c:scaling><c:delete val="1"/><c:crossAx val="1"/></c:valAx>"#,
        "",
    );
    let (pixels, manifest, _) = render(part, &chart_rels());
    assert!(red(rgb(&pixels, 70, 50)), "{:?}", rgb(&pixels, 70, 50));
    assert!(blue(rgb(&pixels, 30, 50)), "{:?}", rgb(&pixels, 30, 50));
    assert_eq!(rgb(&pixels, 5, 5), WHITE);
    assert!(!manifest.contains("axis scale"), "{manifest}");
}

#[test]
fn unsupported_and_combined_types_omit_the_chart_with_one_diagnostic() {
    for (plot, name) in [
        (
            format!(
                r#"<c:radarChart><c:radarStyle val="marker"/>{}<c:axId val="1"/><c:axId val="2"/></c:radarChart>"#,
                series("FF0000", &[50.0, 100.0], "")
            ),
            "c:radarChart",
        ),
        (
            format!(
                r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{0}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:lineChart><c:grouping val="standard"/>{0}<c:axId val="1"/><c:axId val="2"/></c:lineChart>"#,
                series("FF0000", &[50.0, 100.0], "")
            ),
            "c:barChart + c:lineChart",
        ),
    ] {
        let (pixels, manifest, _) = render(with_unit(chart(&plot, SAVED, "")), &chart_rels());
        assert_eq!(rgb(&pixels, 70, 20), WHITE, "{name}");
        let json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        let diagnostics: Vec<_> = json["slides"][0]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["object_name"] == "Chart 1")
            .collect();
        assert_eq!(diagnostics.len(), 1, "{manifest}");
        assert_eq!(diagnostics[0]["code"], "unsupported", "{manifest}");
        assert!(manifest.contains(name), "{manifest}");
        assert!(
            !manifest.contains("Chart layout is approximated"),
            "{manifest}"
        );
        assert!(!manifest.contains("non-table graphicFrame"), "{manifest}");
        assert!(!manifest.contains("Secret series"), "{manifest}");
    }
}

#[test]
fn external_data_is_not_followed_and_limits_omit_the_chart() {
    let plot = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}<c:gapWidth val="100"/><c:axId val="1"/><c:axId val="2"/></c:barChart>"#,
        series("FF0000", &[50.0, 100.0], "")
    );
    let external = with_unit(chart(
        &plot,
        SAVED,
        r#"<c:externalData r:id="rWorkbook"><c:autoUpdate val="0"/></c:externalData>"#,
    ));
    let (pixels, manifest, _) = render(external, &chart_rels());
    assert!(red(rgb(&pixels, 70, 20)), "{:?}", rgb(&pixels, 70, 20));
    assert!(!manifest.contains("Omitted chart"), "{manifest}");

    let many: String = (0..65).map(|_| series("FF0000", &[1.0], "")).collect();
    let plot = format!(
        r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{many}<c:axId val="1"/><c:axId val="2"/></c:barChart>"#
    );
    let (pixels, manifest, _) = render(with_unit(chart(&plot, SAVED, "")), &chart_rels());
    assert_eq!(rgb(&pixels, 50, 85), WHITE);
    assert!(manifest.contains("more than 64 series"), "{manifest}");
}
