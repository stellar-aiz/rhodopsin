//! Verify inheritance, composite shapes, and diagnostics with independently generated PPTX files and pixels.
use resvg::tiny_skia::{Color, Pixmap};
use std::{fs, io::Write, path::Path, process::Command};
use zip::{ZipWriter, write::SimpleFileOptions};
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PR: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
fn rel(kind: &str, target: &str) -> String {
    format!(
        r#"<Relationships xmlns="{PR}"><Relationship Id="r1" Type="{R}/{kind}" Target="{target}"/></Relationships>"#
    )
}
fn page(tag: &str, attrs: &str, background: &str, shapes: &str) -> String {
    format!(
        r#"<p:{tag} xmlns:p="{P}" xmlns:a="{A}" xmlns:r="{R}" {attrs}><p:cSld>{background}<p:spTree>{shapes}</p:spTree></p:cSld></p:{tag}>"#
    )
}
fn transform(x: i32, y: i32, w: i32, h: i32) -> String {
    format!(
        r#"<a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm>"#,
        x * 9525,
        y * 9525,
        w * 9525,
        h * 9525
    )
}
fn rect(x: i32, y: i32, w: i32, h: i32, color: &str) -> String {
    format!(
        r#"<p:sp><p:spPr>{}<a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="{color}"/></a:solidFill></p:spPr></p:sp>"#,
        transform(x, y, w, h)
    )
}
fn placeholder(index: u32, kind: &str, props: &str, text: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{index}" name="タイトル"/><p:nvPr><p:ph idx="{index}" type="{kind}"/></p:nvPr></p:nvSpPr><p:spPr>{props}</p:spPr>{text}</p:sp>"#
    )
}
fn write_deck(path: &Path, slide: String, extra: Vec<(&str, Vec<u8>)>) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    let mut parts=vec![
        ("_rels/.rels",rel("officeDocument","ppt/presentation.xml").into_bytes()),
        ("ppt/presentation.xml",format!(r#"<p:presentation xmlns:p="{P}" xmlns:r="{R}"><p:sldIdLst><p:sldId id="256" r:id="r1"/></p:sldIdLst><p:sldSz cx="952500" cy="952500"/></p:presentation>"#).into_bytes()),
        ("ppt/_rels/presentation.xml.rels",rel("slide","slides/slide1.xml").into_bytes()),
        ("ppt/slides/slide1.xml",slide.into_bytes()),
    ];
    parts.extend(extra);
    for (name, data) in parts {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(&data).unwrap();
    }
    zip.finish().unwrap();
}
fn run(path: &Path, out: &Path, strict: bool) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rhodopsin"));
    cmd.arg(path)
        .arg("--out")
        .arg(out)
        .args(["--width", "100", "--no-system-fonts", "--font-fallback"])
        .arg("--font-file")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/metrics.ttf"
        ));
    if strict {
        cmd.arg("--strict");
    }
    cmd.output().unwrap()
}
fn image(out: &Path) -> Pixmap {
    Pixmap::load_png(out.join("slide-001.png")).unwrap()
}
fn rgb(image: &Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let p = image.pixel(x, y).unwrap();
    (p.red(), p.green(), p.blue())
}
fn check(output: std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn inherits_background_placeholder_and_stacks_master_layout_slide() {
    for hide in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.pptx");
        let out = dir.path().join("out");
        let master = page(
            "sldMaster",
            "",
            r#"<p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFF00"/></a:solidFill></p:bgPr></p:bg>"#,
            &(rect(0, 0, 10, 10, "0000FF")
                + &placeholder(
                    99,
                    "body",
                    &(transform(20, 20, 20, 20)
                        + r#"<a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#),
                    "",
                )),
        );
        let layout = page(
            "sldLayout",
            "",
            "",
            &(rect(0, 10, 10, 10, "FF0000") + &placeholder(7, "body", "", "")),
        );
        let slide = page(
            "sld",
            if hide { "showMasterSp=\"0\"" } else { "" },
            "",
            &placeholder(
                7,
                "body",
                r#"<a:solidFill><a:srgbClr val="00FF00"/></a:solidFill>"#,
                "",
            ),
        );
        write_deck(
            &input,
            slide,
            vec![
                (
                    "ppt/slides/_rels/slide1.xml.rels",
                    rel("slideLayout", "../layouts/layout.xml").into_bytes(),
                ),
                ("ppt/layouts/layout.xml", layout.into_bytes()),
                (
                    "ppt/layouts/_rels/layout.xml.rels",
                    rel("slideMaster", "../masters/master.xml").into_bytes(),
                ),
                ("ppt/masters/master.xml", master.into_bytes()),
            ],
        );
        check(run(&input, &out, true));
        let img = image(&out);
        assert_eq!(rgb(&img, 90, 90), (255, 255, 0));
        assert_eq!(rgb(&img, 25, 25), (0, 255, 0));
        assert_eq!(
            rgb(&img, 5, 5),
            if hide { (255, 255, 0) } else { (0, 0, 255) }
        );
        assert_eq!(
            rgb(&img, 5, 15),
            if hide { (255, 255, 0) } else { (255, 0, 0) }
        );
    }
}
#[test]
fn group_scales_child_coordinates_and_keeps_warning_identity() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.pptx");
    let out = dir.path().join("out");
    let group = format!(
        r#"<p:grpSp><p:grpSpPr><a:xfrm flipH="1"><a:off x="190500" y="190500"/><a:ext cx="571500" cy="571500"/><a:chOff x="95250" y="95250"/><a:chExt cx="285750" cy="285750"/></a:xfrm></p:grpSpPr>{}<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="42" name="未対応グラフ"/></p:nvGraphicFramePr></p:graphicFrame></p:grpSp>"#,
        rect(10, 10, 10, 10, "FF0000")
    );
    write_deck(&input, page("sld", "", "", &group), vec![]);
    check(run(&input, &out, false));
    let img = image(&out);
    assert_eq!(rgb(&img, 70, 30), (255, 0, 0));
    assert_eq!(rgb(&img, 30, 30), (255, 255, 255));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    let d = &manifest["slides"][0]["diagnostics"][0];
    assert_eq!(d["code"], "omitted");
    assert_eq!(d["object_id"], "42");
    assert_eq!(d["object_name"], "未対応グラフ");
    assert_eq!(d["part"], "ppt/slides/slide1.xml");
    assert!(d["location"].as_str().unwrap().starts_with("spTree/0/"));
    let strict = dir.path().join("strict");
    assert!(!run(&input, &strict, true).status.success());
    assert!(!strict.exists());
}
#[test]
fn renders_table_fills_and_merged_cell_without_drawing_covered_cell() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.pptx");
    let out = dir.path().join("out");
    let frame = r#"<p:graphicFrame><p:xfrm><a:off x="95250" y="95250"/><a:ext cx="762000" cy="381000"/></p:xfrm><a:graphic><a:graphicData><a:tbl><a:tblGrid><a:gridCol w="381000"/><a:gridCol w="381000"/></a:tblGrid><a:tr h="190500"><a:tc gridSpan="2"><a:tcPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:tcPr></a:tc><a:tc hMerge="1"><a:tcPr><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a:tcPr></a:tc></a:tr><a:tr h="190500"><a:tc><a:tcPr><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:tcPr></a:tc><a:tc><a:tcPr><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a:tcPr></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#;
    write_deck(&input, page("sld", "", "", frame), vec![]);
    check(run(&input, &out, true));
    let img = image(&out);
    assert_eq!(rgb(&img, 20, 20), (255, 0, 0));
    assert_eq!(rgb(&img, 70, 20), (255, 0, 0));
    assert_eq!(rgb(&img, 20, 40), (0, 255, 0));
    assert_eq!(rgb(&img, 70, 40), (0, 0, 255));
}
#[test]
fn image_crop_clips_pixels_and_invalid_crop_leaves_no_output() {
    let mut png = Pixmap::new(20, 10).unwrap();
    png.fill(Color::from_rgba8(255, 0, 0, 255));
    for y in 0..10 {
        for x in 10..20 {
            let index = (y * 20 + x) * 4;
            png.data_mut()[index..index + 4].copy_from_slice(&[0, 0, 255, 255]);
        }
    }
    for (left, valid) in [(50000, true), (100000, false)] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.pptx");
        let out = dir.path().join("out");
        let picture = format!(
            r#"<p:pic><p:blipFill><a:blip r:embed="r1"/><a:srcRect l="{left}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}</p:spPr></p:pic>"#,
            transform(20, 20, 60, 60)
        );
        write_deck(
            &input,
            page("sld", "", "", &picture),
            vec![
                (
                    "ppt/slides/_rels/slide1.xml.rels",
                    rel("image", "../media/a.png").into_bytes(),
                ),
                ("ppt/media/a.png", png.encode_png().unwrap()),
            ],
        );
        let result = run(&input, &out, true);
        if valid {
            check(result);
            let img = image(&out);
            assert_eq!(rgb(&img, 30, 30), (0, 0, 255));
            assert_eq!(rgb(&img, 10, 30), (255, 255, 255));
        } else {
            assert!(!result.status.success());
            assert!(!out.exists());
        }
    }
}
#[test]
fn explicit_font_renders_with_no_system_fonts_and_records_actual_face() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.pptx");
    let out = dir.path().join("out");
    let body = r#"<p:txBody><a:bodyPr lIns="0" tIns="0"/><a:p><a:r><a:rPr sz="2400"><a:latin typeface="Rhodopsin Metrics Test"/></a:rPr><a:t>AA</a:t></a:r></a:p></p:txBody>"#;
    let shape = format!(
        "<p:sp><p:spPr>{}</p:spPr>{body}</p:sp>",
        transform(0, 0, 100, 100)
    );
    write_deck(&input, page("sld", "", "", &shape), vec![]);
    check(run(&input, &out, false));
    let img = image(&out);
    assert!(
        img.pixels().iter().filter(|p| p.red() < 128).count() > 100,
        "文字の画素がありません"
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema_version"], 4);
    assert_eq!(manifest["system_fonts"], false);
    assert_eq!(manifest["fonts"][0]["family"], "Rhodopsin Metrics Test");
    // PowerPoint's line height for this family is not measured, and that is recorded.
    assert_eq!(
        manifest["line_heights"],
        serde_json::json!([{"requested_family": "Rhodopsin Metrics Test", "basis": "font"}])
    );
}

#[test]
fn inherited_text_settings_match_explicit_settings_and_local_values_win() {
    let dir = tempfile::tempdir().unwrap();
    let body = |props: &str, run: &str| {
        format!(r#"<p:txBody><a:bodyPr {props}/><a:p>{run}</a:p></p:txBody>"#)
    };
    let text = r#"<a:r><a:rPr sz="1200"/><a:t>AA</a:t></a:r>"#;
    let defaults = r#"<a:pPr algn="r"><a:defRPr sz="2400"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr>"#;
    let inherited = placeholder(7, "title", "", &body("", text));
    let explicit = placeholder(
        7,
        "title",
        &transform(10, 10, 80, 80),
        &body(
            "lIns=\"0\" rIns=\"0\" tIns=\"0\" bIns=\"0\" anchor=\"b\"",
            &(defaults.to_owned() + text),
        ),
    );
    for (name, shapes, extras) in [
        (
            "inherited",
            inherited,
            vec![
                (
                    "ppt/slides/_rels/slide1.xml.rels",
                    rel("slideLayout", "../layouts/layout.xml").into_bytes(),
                ),
                (
                    "ppt/layouts/layout.xml",
                    page(
                        "sldLayout",
                        "",
                        "",
                        &placeholder(
                            7,
                            "title",
                            &transform(10, 10, 80, 80),
                            &body(
                                "lIns=\"0\" rIns=\"0\" tIns=\"0\" bIns=\"0\" anchor=\"b\"",
                                defaults,
                            ),
                        ),
                    )
                    .into_bytes(),
                ),
            ],
        ),
        ("explicit", explicit, vec![]),
    ] {
        let input = dir.path().join(format!("{name}.pptx"));
        let out = dir.path().join(name);
        write_deck(&input, page("sld", "", "", &shapes), extras);
        check(run(&input, &out, false));
    }
    assert_eq!(
        image(&dir.path().join("inherited")).data(),
        image(&dir.path().join("explicit")).data()
    );
    let img = image(&dir.path().join("inherited"));
    assert!(img.pixels().iter().any(|p| p.red() > 200 && p.green() < 50));
}

#[test]
fn identical_warnings_on_two_objects_keep_two_diagnostic_targets() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.pptx");
    let out = dir.path().join("out");
    let objects=(1..=2).map(|id|format!(r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="グラフ{id}"/></p:nvGraphicFramePr></p:graphicFrame>"#)).collect::<String>();
    write_deck(&input, page("sld", "", "", &objects), vec![]);
    check(run(&input, &out, false));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    let slide = &manifest["slides"][0];
    assert_eq!(slide["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(slide["diagnostics"].as_array().unwrap().len(), 2);
    assert_ne!(
        slide["diagnostics"][0]["object_id"],
        slide["diagnostics"][1]["object_id"]
    );
}

#[test]
fn invalid_table_merge_is_rejected_before_output() {
    for attrs in ["hMerge=\"1\"", "gridSpan=\"2\"", "rowSpan=\"0\""] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.pptx");
        let out = dir.path().join("out");
        let shape = format!(
            r#"<p:graphicFrame><p:xfrm><a:off x="0" y="0"/><a:ext cx="952500" cy="952500"/></p:xfrm><a:graphic><a:graphicData><a:tbl><a:tblGrid><a:gridCol w="952500"/></a:tblGrid><a:tr h="952500"><a:tc {attrs}/></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#
        );
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        let result = run(&input, &out, false);
        assert!(!result.status.success());
        assert!(!out.exists());
        assert!(String::from_utf8_lossy(&result.stderr).contains("Merged cell"));
    }
}

#[test]
fn nested_group_limit_keeps_valid_input_and_rejects_excess_before_output() {
    for depth in [64, 65, 66] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.pptx");
        let out = dir.path().join("out");
        let mut shape = rect(0, 0, 10, 10, "000000");
        for _ in 0..depth {
            shape = format!(
                r#"<p:grpSp><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="952500" cy="952500"/><a:chOff x="0" y="0"/><a:chExt cx="952500" cy="952500"/></a:xfrm></p:grpSpPr>{shape}</p:grpSp>"#
            );
        }
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        let result = run(&input, &out, false);
        if depth == 64 {
            check(result);
            assert_eq!(rgb(&image(&out), 5, 5), (0, 0, 0));
        } else {
            // Require a normal error exit; a crash is not input rejection.
            assert_eq!(
                result.status.code(),
                Some(1),
                "終了状態: {:?}, 標準エラー: {}",
                result.status,
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(!out.exists());
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("64 levels"),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn missing_font_and_glyph_are_reported_separately() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.pptx");
    let out = dir.path().join("out");
    let body = r#"<p:txBody><a:bodyPr/><a:p><a:r><a:rPr><a:latin typeface="存在しない書体"/><a:ea typeface="存在しない書体"/></a:rPr><a:t>A未</a:t></a:r></a:p></p:txBody>"#;
    let shape = placeholder(7, "title", &transform(0, 0, 100, 100), body);
    write_deck(&input, page("sld", "", "", &shape), vec![]);
    let result = run(&input, &out, false);
    assert!(!result.status.success());
    assert!(!out.exists());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("code: FONT_GLYPH_MISSING"), "{stderr}");
    assert!(stderr.contains("codepoint: \"U+672A\""), "{stderr}");
}

#[test]
fn rounded_rectangle_adjustment_and_empty_outline() {
    for (adjustment, corner_is_red) in [("val 0", true), ("val 50000", false)] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("rounded.pptx");
        let out = dir.path().join("out");
        let shapes = format!(
            r#"<p:sp><p:spPr>{}<a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="{adjustment}"/></a:avLst></a:prstGeom><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln/></p:spPr></p:sp>"#,
            transform(10, 10, 80, 40)
        );
        write_deck(&input, page("sld", "", "", &shapes), vec![]);
        check(run(&input, &out, true));
        let rendered = image(&out);
        assert_eq!(rgb(&rendered, 50, 10), (255, 0, 0));
        assert_eq!(rgb(&rendered, 50, 30), (255, 0, 0));
        assert_eq!(rgb(&rendered, 11, 11) == (255, 0, 0), corner_is_red);
    }
}

#[test]
fn font_mapping_matches_api_and_cli_and_aggregates_targets() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("mapping.pptx");
    let body = r#"<p:txBody><a:bodyPr/><a:p><a:r><a:rPr><a:latin typeface="要求書体"/><a:ea typeface="要求書体"/></a:rPr><a:t>Aあ</a:t></a:r></a:p></p:txBody>"#;
    write_deck(
        &input,
        page(
            "sld",
            "",
            "",
            &(placeholder(7, "title", &transform(0, 0, 100, 40), body)
                + &placeholder(8, "title", &transform(0, 50, 100, 40), body)),
        ),
        vec![],
    );
    let font = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cjk.ttf");
    let cli_out = dir.path().join("cli");
    let cli = Command::new(env!("CARGO_BIN_EXE_rhodopsin"))
        .arg("--font-fallback")
        .arg(&input)
        .arg("--out")
        .arg(&cli_out)
        .args([
            "--width",
            "100",
            "--no-system-fonts",
            "--font-map",
            "要求書体=Pptx CJK",
            "--font-file",
        ])
        .arg(&font)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert_eq!(
        stderr.matches("Font substitution: 要求書体").count(),
        1,
        "{stderr}"
    );
    check(cli);
    let api_out = dir.path().join("api");
    let report = rhodopsin::convert(
        &input,
        &api_out,
        &rhodopsin::ConvertOptions {
            font_fallback: true,
            width: 100,
            no_system_fonts: true,
            font_file: vec![font],
            font_mapping: std::collections::HashMap::from([("要求書体".into(), "Pptx CJK".into())]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        fs::read(cli_out.join("slide-001.png")).unwrap(),
        fs::read(api_out.join("slide-001.png")).unwrap()
    );
    let diagnostics: Vec<_> = report.slides[0]
        .diagnostics
        .iter()
        .filter(|d| d.font.is_some())
        .collect();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(
        diagnostics[0].font.as_ref().unwrap().reason,
        "explicit_mapping"
    );
    assert_eq!(diagnostics[0].object_id.as_deref(), Some("7"));
    assert_eq!(diagnostics[1].object_id.as_deref(), Some("8"));
}

#[test]
fn missing_glyph_strict_leaves_no_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("missing.pptx");
    let out = dir.path().join("strict");
    write_deck(
        &input,
        page(
            "sld",
            "",
            "",
            &placeholder(
                1,
                "title",
                &transform(0, 0, 100, 100),
                r#"<p:txBody><a:bodyPr/><a:p><a:r><a:rPr><a:ea typeface="Rhodopsin Metrics Test"/></a:rPr><a:t>未</a:t></a:r></a:p></p:txBody>"#,
            ),
        ),
        vec![],
    );
    let output = run(&input, &out, true);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("U+672A"));
    assert!(!out.exists());
}

#[test]
fn shape_autofit_expands_fill_and_text_around_the_anchor() {
    for (anchor, y, top, bottom) in [("t", 20, 20, 68), ("ctr", 30, 11, 59), ("b", 60, 22, 70)] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("autofit.pptx");
        let out = dir.path().join("out");
        let shape = format!(
            r#"<p:sp><p:spPr>{}<a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></p:spPr><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0" anchor="{anchor}"><a:spAutoFit/></a:bodyPr><a:p><a:pPr><a:defRPr sz="1200"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r></a:p></p:txBody></p:sp>"#,
            transform(0, y, 100, 10)
        );
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        check(run(&input, &out, false));
        let img = image(&out);
        assert_eq!(rgb(&img, 80, top), (0, 255, 0));
        assert_eq!(rgb(&img, 80, bottom - 1), (0, 255, 0));
        assert_eq!(rgb(&img, 80, top - 1), (255, 255, 255));
        assert_eq!(rgb(&img, 80, bottom), (255, 255, 255));
        let m: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        assert!(
            !m["slides"][0]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "text_overflow")
        );
    }
}

#[test]
fn bullet_font_mapping_color_and_local_none_reach_pixels() {
    for disabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("bullet.pptx");
        let out = dir.path().join("out");
        let none = if disabled { "<a:buNone/>" } else { "" };
        let body = format!(
            r#"<p:txBody><a:bodyPr lIns="0" tIns="0" rIns="0" bIns="0"/><a:lstStyle><a:lvl1pPr marL="190500" indent="-190500"><a:buChar char="•"/><a:buFont typeface="要求記号書体"/><a:buSzPct val="150000"/><a:buClr><a:srgbClr val="0000FF"/></a:buClr><a:defRPr sz="1200"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:lvl1pPr></a:lstStyle><a:p><a:pPr>{none}</a:pPr><a:r><a:t>AAAA AAAA AAAA</a:t></a:r></a:p></p:txBody>"#
        );
        write_deck(
            &input,
            page(
                "sld",
                "",
                "",
                &placeholder(1, "title", &transform(0, 0, 100, 100), &body),
            ),
            vec![],
        );
        let report = rhodopsin::convert(
            &input,
            &out,
            &rhodopsin::ConvertOptions {
                font_fallback: true,
                width: 100,
                no_system_fonts: true,
                font_file: vec![
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metrics.ttf"),
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bullets.ttf"),
                ],
                font_mapping: std::collections::HashMap::from([(
                    "要求記号書体".into(),
                    "Pptx Bullets".into(),
                )]),
                ..Default::default()
            },
        )
        .unwrap();
        let img = image(&out);
        let mut blue = 0;
        for y in 0..100 {
            for x in 0..100 {
                let (r, g, b) = rgb(&img, x, y);
                if b > 200 && r < 100 && g < 100 {
                    blue += 1;
                    assert!(x < 20);
                }
            }
        }
        assert_eq!(blue > 0, !disabled);
        assert_eq!(
            report.slides[0].diagnostics.iter().any(|d| d
                .font
                .as_ref()
                .is_some_and(
                    |f| f.requested_family == "要求記号書体" && f.reason == "explicit_mapping"
                )),
            !disabled
        );
    }
}

#[test]
fn preset_text_rectangle_applies_body_and_paragraph_margins_once() {
    let dir = tempfile::tempdir().unwrap();
    let body = r#"<p:txBody><a:bodyPr lIns="19050" rIns="28575" tIns="0" bIns="0"><a:normAutofit fontScale="55000"/></a:bodyPr><a:p><a:pPr marL="47625" marR="66675"><a:defRPr sz="1200"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr><a:r><a:t>AAAA AAAA AAAA AAAA AAAA AAAA</a:t></a:r></a:p></p:txBody>"#;
    for (name, geometry, x, y, w, h) in [
        ("arrow", "rightArrow", 0, 0, 100, 40),
        ("rectangle", "rect", 0, 10, 90, 20),
    ] {
        let input = dir.path().join(format!("{name}.pptx"));
        let shape = format!(
            r#"<p:sp><p:spPr>{}<a:prstGeom prst="{geometry}"/><a:noFill/></p:spPr>{body}</p:sp>"#,
            transform(x, y, w, h)
        );
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        check(run(&input, &dir.path().join(name), false));
    }
    assert_eq!(
        image(&dir.path().join("arrow")).data(),
        image(&dir.path().join("rectangle")).data()
    );
}

#[test]
fn preset_autofit_matches_explicit_expanded_geometry_for_each_anchor() {
    let dir = tempfile::tempdir().unwrap();
    for (anchor, y, final_y) in [("t", 10, 10), ("ctr", 35, 16), ("b", 65, 27)] {
        for fitted in [false, true] {
            let mode = if fitted { "spAutoFit" } else { "noAutofit" };
            let (top, height) = if fitted { (y, 10) } else { (final_y, 48) };
            let shape = format!(
                r#"<p:sp><p:spPr>{}<a:prstGeom prst="rightArrow"/><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></p:spPr><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0" anchor="{anchor}"><a:{mode}/></a:bodyPr><a:p><a:pPr><a:defRPr sz="600"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r></a:p></p:txBody></p:sp>"#,
                transform(0, top, 100, height)
            );
            let input = dir.path().join(format!("{anchor}-{fitted}.pptx"));
            let out = dir.path().join(format!("{anchor}-{fitted}"));
            write_deck(&input, page("sld", "", "", &shape), vec![]);
            check(run(&input, &out, false));
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
            assert!(
                !manifest["slides"][0]["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["code"] == "text_overflow")
            );
        }
        assert_eq!(
            image(&dir.path().join(format!("{anchor}-true"))).data(),
            image(&dir.path().join(format!("{anchor}-false"))).data()
        );
    }
}

#[test]
fn preset_autofit_bounds_degenerate_areas_and_keeps_final_diagnostics_once() {
    for (height, adjustment, limited) in [(0, 50000, false), (10, 0, true), (10, 1, true)] {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("bounded.pptx");
        let out = d.path().join("out");
        let shape = format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="境界検証"/></p:nvSpPr><p:spPr>{}<a:prstGeom prst="rightArrow"><a:avLst><a:gd name="adj1" fmla="val {adjustment}"/></a:avLst></a:prstGeom><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></p:spPr><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0"><a:spAutoFit/></a:bodyPr><a:p><a:pPr><a:defRPr sz="600"><a:latin typeface="要求不在"/></a:defRPr></a:pPr><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r></a:p></p:txBody></p:sp>"#,
            transform(0, 10, 100, height)
        );
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        check(run(&input, &out, false));
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        let records = manifest["slides"][0]["diagnostics"].as_array().unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|d| d["message"]
                    .as_str()
                    .unwrap()
                    .contains("autofit reached its limit"))
                .count(),
            usize::from(limited)
        );
        assert_eq!(
            records
                .iter()
                .filter(|d| d["code"] == "font_substitution")
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|d| d["message"]
                    .as_str()
                    .unwrap()
                    .contains("Text layout is approximate"))
                .count(),
            1
        );
        assert!(records.iter().all(|d| d["object_id"] == "2"));
        if !limited {
            // Even from zero height, expand to a 57.6px arrow accommodating 28.8px text
            // (three lines of the substituted family at 1.2 em).
            let img = image(&out);
            assert_eq!(rgb(&img, 10, 25), (0, 255, 0));
            assert_eq!(rgb(&img, 10, 51), (0, 255, 0));
            assert_eq!(rgb(&img, 10, 55), (255, 255, 255));
            assert!(!records.iter().any(|d| d["code"] == "text_overflow"));
        }
    }
}

const ROW_COLORS: [&str; 3] = ["FFD0D0", "D0FFD0", "D0D0FF"];
const ROW_RGB: [(u8, u8, u8); 3] = [(255, 208, 208), (208, 255, 208), (208, 208, 255)];

/// A table at the slide origin. Row `i` is filled with `ROW_COLORS[i % 3]`; `texts[i]`
/// (if any) is placed in every cell of that row. `frame_h` is the graphicFrame height.
fn table_frame(heights: &[i32], widths: &[i32], frame_h: i32, texts: &[Option<&str>]) -> String {
    let grid: String = widths
        .iter()
        .map(|w| format!(r#"<a:gridCol w="{}"/>"#, w * 9525))
        .collect();
    let rows: String = heights
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let text = texts.get(i).copied().flatten().map(|t| {
                format!(
                    r#"<a:txBody><a:bodyPr/><a:p><a:r><a:rPr sz="1200"><a:latin typeface="Rhodopsin Metrics Test"/></a:rPr><a:t>{t}</a:t></a:r></a:p></a:txBody>"#
                )
            });
            let cells: String = widths
                .iter()
                .map(|_| {
                    format!(
                        r#"<a:tc>{}<a:tcPr><a:solidFill><a:srgbClr val="{}"/></a:solidFill></a:tcPr></a:tc>"#,
                        text.as_deref().unwrap_or(""),
                        ROW_COLORS[i % 3]
                    )
                })
                .collect();
            format!(r#"<a:tr h="{}">{cells}</a:tr>"#, h * 9525)
        })
        .collect();
    wrap_table(widths, frame_h, &grid, &rows)
}
fn wrap_table(widths: &[i32], frame_h: i32, grid: &str, rows: &str) -> String {
    format!(
        r#"<p:graphicFrame><p:xfrm><a:off x="0" y="0"/><a:ext cx="{}" cy="{}"/></p:xfrm><a:graphic><a:graphicData><a:tbl><a:tblGrid>{grid}</a:tblGrid>{rows}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
        widths.iter().sum::<i32>() * 9525,
        frame_h * 9525
    )
}
fn render_table(table: &str) -> (Pixmap, String) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.pptx");
    let out = dir.path().join("out");
    write_deck(&input, page("sld", "", "", table), vec![]);
    check(run(&input, &out, false));
    (
        image(&out),
        fs::read_to_string(out.join("manifest.json")).unwrap(),
    )
}
/// The `[top, bottom)` pixel span of each row color along column `x`, in row order.
fn row_spans(img: &Pixmap, x: u32, rows: usize) -> Vec<(u32, u32)> {
    (0..rows)
        .map(|i| {
            let ys: Vec<_> = (0..img.height())
                .filter(|&y| rgb(img, x, y) == ROW_RGB[i % 3])
                .collect();
            assert!(!ys.is_empty(), "row {i} is not drawn at x={x}");
            (ys[0], ys[ys.len() - 1] + 1)
        })
        .collect()
}
/// Pixel positions agree within one anti-aliased edge pixel.
fn near(a: u32, b: u32) -> bool {
    a.abs_diff(b) <= 1
}
/// The lowest pixel row containing dark text pixels within the columns `xs`.
fn text_bottom(img: &Pixmap, xs: std::ops::Range<u32>) -> u32 {
    xs.flat_map(|x| (0..img.height()).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let (r, g, b) = rgb(img, x, y);
            r < 100 && g < 100 && b < 100
        })
        .map(|(_, y)| y)
        .max()
        .expect("no text pixels")
}

#[test]
fn zero_height_table_row_grows_to_fit_its_text() {
    let (img, manifest) = render_table(&table_frame(
        &[20, 0, 20],
        &[100],
        60,
        &[None, Some("AA"), None],
    ));
    let spans = row_spans(&img, 95, 3);
    assert_eq!(spans[0], (0, 20));
    assert_eq!(spans[1].0, 20);
    // The zero-height row grows to hold its text; the next row starts below the text.
    let bottom = text_bottom(&img, 0..100);
    assert!(bottom > 20 && bottom < spans[1].1, "{spans:?} {bottom}");
    assert!(near(spans[2].0, spans[1].1), "{spans:?}");
    assert!(near(spans[2].1 - spans[2].0, 20), "{spans:?}");
    // The #45 fallback (a share of the frame height) and its diagnostic are gone.
    assert!(!manifest.contains("Table rows with zero height"));
}

#[test]
fn wrapped_cell_text_grows_its_row_and_pushes_later_rows_down() {
    let text = "AA AA AA AA AA AA";
    let (img, manifest) = render_table(&table_frame(&[20, 20], &[60], 40, &[Some(text), None]));
    let spans = row_spans(&img, 57, 2);
    assert_eq!(spans[0].0, 0);
    // Several wrapped lines need far more than the stored 20 px.
    assert!(spans[0].1 > 40, "{spans:?}");
    // No text pixels from row 1 reach row 2's background region.
    let bottom = text_bottom(&img, 0..60);
    assert!(bottom < spans[1].0, "{spans:?} {bottom}");
    // A row whose content fits keeps its stored height.
    assert!(near(spans[1].0, spans[0].1), "{spans:?}");
    assert!(near(spans[1].1 - spans[1].0, 20), "{spans:?}");
    assert!(!manifest.contains("Text exceeds the text box height"));
}

#[test]
fn rows_that_fit_keep_stored_heights_whatever_the_frame_height() {
    for frame_h in [30, 60, 120] {
        let (img, _) = render_table(&table_frame(
            &[30, 30],
            &[100],
            frame_h,
            &[Some("AA"), None],
        ));
        assert_eq!(row_spans(&img, 95, 2), vec![(0, 30), (30, 60)], "{frame_h}");
        // The table is not stretched to a taller frame.
        assert_eq!(rgb(&img, 95, 70), (255, 255, 255), "{frame_h}");
    }
}

#[test]
fn all_zero_height_table_rows_grow_to_their_text() {
    let (img, _) = render_table(&table_frame(&[0, 0], &[100], 60, &[Some("AA"), Some("AA")]));
    let spans = row_spans(&img, 95, 2);
    assert_eq!(spans[0].0, 0);
    assert!(near(spans[1].0, spans[0].1), "{spans:?}");
    // Both rows hold one line of the same text, so they grow to the same height.
    assert!(
        near(spans[0].1 - spans[0].0, spans[1].1 - spans[1].0),
        "{spans:?}"
    );
    assert!(spans[0].1 > 16, "{spans:?}");
}

#[test]
fn row_spanning_cell_adds_its_shortfall_to_the_last_spanned_row() {
    let cell = |attrs: &str, text: &str, color: &str| {
        let body = if text.is_empty() {
            String::new()
        } else {
            format!(
                r#"<a:txBody><a:bodyPr/><a:p><a:r><a:rPr sz="1200"><a:latin typeface="Rhodopsin Metrics Test"/></a:rPr><a:t>{text}</a:t></a:r></a:p></a:txBody>"#
            )
        };
        format!(
            r#"<a:tc {attrs}>{body}<a:tcPr><a:solidFill><a:srgbClr val="{color}"/></a:solidFill></a:tcPr></a:tc>"#
        )
    };
    let rows = format!(
        r#"<a:tr h="{h}">{}{}</a:tr><a:tr h="{h}">{}{}</a:tr>"#,
        cell(r#"rowSpan="2""#, "AA AA AA AA AA AA", "FFFFFF"),
        cell("", "", ROW_COLORS[0]),
        cell(r#"vMerge="1""#, "", "FFFFFF"),
        cell("", "", ROW_COLORS[1]),
        h = 10 * 9525
    );
    let grid = format!(r#"<a:gridCol w="{0}"/><a:gridCol w="{0}"/>"#, 50 * 9525);
    let (img, _) = render_table(&wrap_table(&[50, 50], 20, &grid, &rows));
    let spans = row_spans(&img, 95, 2);
    // The first row keeps its stored height; the second absorbs the shortfall.
    assert_eq!(spans[0], (0, 10));
    assert_eq!(spans[1].0, 10);
    let bottom = text_bottom(&img, 0..50);
    assert!(bottom > 20 && bottom < spans[1].1, "{spans:?} {bottom}");
}

#[test]
fn negative_row_height_and_zero_column_width_are_still_rejected() {
    for (heights, widths) in [(vec![-20, 20], vec![100]), (vec![20, 20], vec![50, 0])] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.pptx");
        let out = dir.path().join("out");
        write_deck(
            &input,
            page("sld", "", "", &table_frame(&heights, &widths, 60, &[])),
            vec![],
        );
        let result = run(&input, &out, false);
        assert!(!result.status.success());
        assert!(!out.exists());
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("Invalid table row or column dimensions")
        );
    }
}
const A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";
/// Render one unfilled, unstroked rectangle whose `a:extLst` holds a red `a14:hiddenFill`
/// plus `extra` extensions; return the output directory and the shape-property messages.
fn render_extensions(dir: &Path, extra: &str, strict: bool) -> (std::path::PathBuf, Vec<String>) {
    let input = dir.join("a.pptx");
    let out = dir.join("out");
    let shape = format!(
        r#"<p:sp><p:spPr>{}<a:prstGeom prst="rect"/><a:noFill/><a:ln><a:noFill/></a:ln><a:extLst><a:ext uri="{{909E8E84-426E-40DD-AFC4-6F175D3DCCD1}}"><a14:hiddenFill xmlns:a14="{A14}"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a14:hiddenFill></a:ext>{extra}</a:extLst></p:spPr></p:sp>"#,
        transform(10, 10, 80, 80)
    );
    write_deck(&input, page("sld", "", "", &shape), vec![]);
    check(run(&input, &out, strict));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    let messages = manifest["slides"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["message"].as_str().unwrap().to_string())
        .filter(|m| m.contains("shape property"))
        .collect();
    (out, messages)
}
#[test]
fn hidden_office_extensions_are_ignored_without_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let extra = format!(
        r#"<a:ext uri="{{91240B29-F687-4F45-9708-019B960494DF}}"><a14:hiddenLine xmlns:a14="{A14}" w="9525"><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a14:hiddenLine></a:ext><a:ext uri="{{AF507438-7753-43E0-B8FC-AC1667EBCBE1}}"><a14:hiddenEffects xmlns:a14="{A14}"><a:effectLst/></a14:hiddenEffects></a:ext>"#
    );
    let (out, messages) = render_extensions(dir.path(), &extra, true);
    let img = image(&out);
    assert_eq!(rgb(&img, 50, 50), (255, 255, 255));
    assert_eq!(rgb(&img, 10, 50), (255, 255, 255));
    assert!(messages.is_empty());
}
#[test]
fn other_shape_property_extensions_are_reported_by_element_name() {
    let dir = tempfile::tempdir().unwrap();
    let extra = r#"<a:ext uri="{00000000-0000-0000-0000-000000000000}"><x:future xmlns:x="urn:example:future"/></a:ext>"#;
    let (_, messages) = render_extensions(dir.path(), extra, false);
    assert_eq!(messages, ["Unsupported shape property extension: x:future"]);
    let strict = dir.path().join("strict");
    assert!(
        !run(&dir.path().join("a.pptx"), &strict, true)
            .status
            .success()
    );
}

fn diagnostics(out: &Path) -> Vec<serde_json::Value> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    manifest["slides"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn unwrapped_shape_autofit_sets_width_and_keeps_the_alignment_edge() {
    // "AAAA" is 32px wide in the metrics font at 12pt; with 4px insets the box is 40px.
    // Under rot="180" or flipV the text reads from the page's right edge, so the
    // left-aligned edge that stays fixed is the page-right one.
    for (align, x, w, left, xfrm) in [
        ("l", 40, 10, 40, ""),
        ("ctr", 40, 10, 25, ""),
        ("r", 40, 10, 10, ""),
        ("l", 20, 60, 20, ""),
        ("ctr", 20, 60, 30, ""),
        ("r", 20, 60, 40, ""),
        ("l", 40, 10, 40, r#"flipH="1""#),
        ("l", 40, 10, 10, r#"rot="10800000""#),
        ("l", 40, 10, 10, r#"flipV="1""#),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("autofit-width.pptx");
        let out = dir.path().join("out");
        let shape = format!(
            r#"<p:sp><p:spPr>{}<a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill><a:ln w="0"><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap="none" lIns="38100" rIns="38100" tIns="0" bIns="0"><a:spAutoFit/></a:bodyPr><a:p><a:pPr algn="{align}"><a:defRPr sz="1200"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr><a:r><a:t>AAAA</a:t></a:r></a:p></p:txBody></p:sp>"#,
            transform(x, 10, w, 60).replace("<a:xfrm>", &format!("<a:xfrm {xfrm}>"))
        );
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        check(run(&input, &out, false));
        let img = image(&out);
        let row = 65;
        assert_eq!(rgb(&img, left, row), (0, 255, 0), "{align} {x} {w} {xfrm}");
        assert_eq!(
            rgb(&img, left + 39, row),
            (0, 255, 0),
            "{align} {x} {w} {xfrm}"
        );
        if left > 0 {
            assert_eq!(
                rgb(&img, left - 1, row),
                (255, 255, 255),
                "{align} {x} {w} {xfrm}"
            );
        }
        assert_eq!(
            rgb(&img, left + 40, row),
            (255, 255, 255),
            "{align} {x} {w} {xfrm}"
        );
        let records = diagnostics(&out);
        assert!(
            !records.iter().any(|d| {
                let message = d["message"].as_str().unwrap();
                message.contains("Horizontal shape autofit") || d["code"] == "text_overflow"
            }),
            "{records:?}"
        );
    }
}

#[test]
fn unwrapped_shape_autofit_keeps_width_for_empty_and_vertical_bodies() {
    for (vert, text, warned) in [
        ("horz", "", false),
        ("horz", "   ", false),
        ("vert270", "AAAA", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("autofit-kept.pptx");
        let out = dir.path().join("out");
        let shape = format!(
            r#"<p:sp><p:spPr>{}<a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill><a:ln w="0"><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap="none" vert="{vert}" lIns="0" rIns="0" tIns="0" bIns="0"><a:spAutoFit/></a:bodyPr><a:p><a:pPr><a:defRPr sz="600"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#,
            transform(40, 10, 10, 80)
        );
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        check(run(&input, &out, false));
        let img = image(&out);
        assert_eq!(rgb(&img, 40, 15), (0, 255, 0), "{vert} {text:?}");
        assert_eq!(rgb(&img, 49, 15), (0, 255, 0), "{vert} {text:?}");
        assert_eq!(rgb(&img, 39, 15), (255, 255, 255), "{vert} {text:?}");
        assert_eq!(rgb(&img, 50, 15), (255, 255, 255), "{vert} {text:?}");
        let records = diagnostics(&out);
        assert_eq!(
            records.iter().any(|d| d["message"]
                .as_str()
                .unwrap()
                .contains("Shape autofit for table cells or vertical text is unsupported")),
            warned,
            "{records:?}"
        );
        assert!(
            !records.iter().any(|d| d["message"]
                .as_str()
                .unwrap()
                .contains("Horizontal shape autofit")),
            "{records:?}"
        );
    }
}

#[test]
fn autofit_in_a_small_inset_text_area_grows_only_by_the_needed_height() {
    // A custom geometry with the `plus` preset's text rectangle: inset on every side
    // by 47.769% of the shorter side, so the text area is under 1px of a 20px square.
    // It does not scale with the shape, which used to drive the height to the limit.
    let geometry = r#"<a:custGeom><a:avLst/><a:gdLst><a:gd name="x1" fmla="*/ ss 47769 100000"/><a:gd name="x2" fmla="+- r 0 x1"/><a:gd name="y2" fmla="+- b 0 x1"/></a:gdLst><a:rect l="x1" t="x1" r="x2" b="y2"/><a:pathLst><a:path w="100" h="100"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="100" y="0"/></a:lnTo><a:lnTo><a:pt x="100" y="100"/></a:lnTo><a:lnTo><a:pt x="0" y="100"/></a:lnTo><a:close/></a:path></a:pathLst></a:custGeom>"#;
    for (paragraph, grows) in [
        ("<a:p><a:endParaRPr sz=\"600\"/></a:p>", false),
        (
            r#"<a:p><a:pPr><a:defRPr sz="600"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr><a:r><a:t>A</a:t></a:r></a:p>"#,
            true,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("inset.pptx");
        let out = dir.path().join("out");
        let shape = format!(
            r#"<p:sp><p:spPr>{}{geometry}<a:solidFill><a:srgbClr val="00FF00"/></a:solidFill><a:ln w="0"><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap="square" lIns="54000" tIns="54000" rIns="54000" bIns="54000" anchor="ctr"><a:spAutoFit/></a:bodyPr>{paragraph}</p:txBody></p:sp>"#,
            transform(40, 40, 20, 20)
        );
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        check(run(&input, &out, false));
        let img = image(&out);
        let filled: Vec<u32> = (0..100)
            .filter(|&y| rgb(&img, 41, y) == (0, 255, 0))
            .collect();
        let (top, bottom) = (filled[0], *filled.last().unwrap());
        if grows {
            // About 20px of text area are needed (8px line plus two 5.7px insets),
            // so the square grows to roughly 39px around its centre, not to the limit.
            assert!(top < 38 && bottom > 61, "{top}..{bottom}");
            assert!(top > 25 && bottom < 75, "{top}..{bottom}");
        } else {
            assert_eq!((top, bottom), (40, 59));
        }
        let records = diagnostics(&out);
        assert!(
            !records.iter().any(|d| {
                let message = d["message"].as_str().unwrap();
                // The one-letter case still overflows its sub-pixel width, which
                // autofit does not change for wrapped text.
                message.contains("autofit reached its limit")
                    || (!grows && d["code"] == "text_overflow")
            }),
            "{records:?}"
        );
    }
    // Unwrapped text sizes the width through the same small text area: "AAAA" needs
    // 16px plus two 5.7px insets, so the left-aligned square grows to about 46px.
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("inset-width.pptx");
    let out = dir.path().join("out");
    let shape = format!(
        r#"<p:sp><p:spPr>{}{geometry}<a:solidFill><a:srgbClr val="00FF00"/></a:solidFill><a:ln w="0"><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap="none" lIns="54000" tIns="54000" rIns="54000" bIns="54000" anchor="ctr"><a:spAutoFit/></a:bodyPr><a:p><a:pPr><a:defRPr sz="600"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr><a:r><a:t>AAAA</a:t></a:r></a:p></p:txBody></p:sp>"#,
        transform(40, 40, 20, 20)
    );
    write_deck(&input, page("sld", "", "", &shape), vec![]);
    check(run(&input, &out, false));
    let img = image(&out);
    let filled: Vec<u32> = (0..100)
        .filter(|&x| rgb(&img, x, 50) == (0, 255, 0))
        .collect();
    let (left, right) = (filled[0], *filled.last().unwrap());
    assert_eq!(left, 40);
    assert!(right > 80 && right < 92, "{left}..{right}");
    // Once the square is wider than tall, the inset follows the height, so the text
    // area cannot catch up: the growth stops after one step and says so.
    assert!(diagnostics(&out).iter().any(|d| {
        d["message"]
            .as_str()
            .unwrap()
            .contains("autofit reached its limit")
    }));
}

#[test]
fn rotated_vertical_text_runs_along_the_box_and_stays_inside() {
    // "AAAA" is 32px long in the metrics font at 12pt. With lIns/tIns of 0, vert270
    // starts at the bottom edge with its line on the left; vert starts at the top
    // with its line on the right.
    for (vert, starts_low, line_left) in [("vert270", true, true), ("vert", false, false)] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("vertical.pptx");
        let out = dir.path().join("out");
        let shape = format!(
            r#"<p:sp><p:spPr>{}<a:prstGeom prst="rect"/><a:noFill/></p:spPr><p:txBody><a:bodyPr vert="{vert}" lIns="0" rIns="0" tIns="0" bIns="0" anchor="t"/><a:p><a:pPr><a:defRPr sz="1200"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr><a:r><a:t>AAAA</a:t></a:r></a:p></p:txBody></p:sp>"#,
            transform(40, 5, 20, 90)
        );
        write_deck(&input, page("sld", "", "", &shape), vec![]);
        check(run(&input, &out, false));
        let img = image(&out);
        let ink: Vec<(u32, u32)> = (0..100)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .filter(|&(x, y)| rgb(&img, x, y).0 < 128)
            .collect();
        assert!(!ink.is_empty(), "{vert}");
        let (xs, ys): (Vec<_>, Vec<_>) = ink.iter().copied().unzip();
        let (left, right) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
        let (top, bottom) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
        assert!(
            left >= 40 && right < 60 && top >= 5 && bottom < 95,
            "{vert}: {ink:?}"
        );
        // The line runs along the box: much taller than wide.
        assert!(
            bottom - top > 2 * (right - left),
            "{vert}: {left}..{right} x {top}..{bottom}"
        );
        assert_eq!(top > 50, starts_low, "{vert}: {top}..{bottom}");
        assert_eq!(right < 58, line_left, "{vert}: {left}..{right}");
        let records = diagnostics(&out);
        assert!(
            !records.iter().any(|d| {
                let message = d["message"].as_str().unwrap();
                message.contains("Vertical text is unsupported") || d["code"] == "text_overflow"
            }),
            "{records:?}"
        );
    }
}

type TextBounds = (u32, u32, u32, u32);

/// Render one text-bearing preset shape without fill or outline, so only text pixels are blue.
fn blue_text_bounds(
    kind: &str,
    guides: &str,
    frame: (i32, i32, i32, i32),
    anchor: &str,
    paragraph: &str,
) -> (Option<TextBounds>, Vec<serde_json::Value>) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("basic.pptx");
    let out = dir.path().join("out");
    let shape = format!(
        r#"<p:sp><p:spPr>{}<a:prstGeom prst="{kind}"><a:avLst>{guides}</a:avLst></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0" anchor="{anchor}"><a:noAutofit/></a:bodyPr><a:p><a:pPr algn="l"><a:defRPr sz="900"><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:pPr>{paragraph}</a:p></p:txBody></p:sp>"#,
        transform(frame.0, frame.1, frame.2, frame.3)
    );
    write_deck(&input, page("sld", "", "", &shape), vec![]);
    check(run(&input, &out, false));
    let img = image(&out);
    let mut bounds: Option<TextBounds> = None;
    for y in 0..img.height() {
        for x in 0..img.width() {
            if rgb(&img, x, y) == (0, 0, 255) {
                let b = bounds.get_or_insert((x, y, x, y));
                *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    let records = manifest["slides"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .clone();
    (bounds, records)
}

#[test]
fn ellipse_text_starts_inside_the_inscribed_rectangle_and_wraps_there() {
    let words = "<a:r><a:t>AAAA AAAA AAAA AAAA AAAA AAAA</a:t></a:r>";
    let (bounds, records) = blue_text_bounds("ellipse", "", (10, 10, 80, 80), "t", words);
    let (left, top, right, _) = bounds.expect("text is drawn");
    // Inset is 80 * (1 - cos 45deg) / 2 = 11.7px on each side.
    assert!(left >= 21, "left {left}");
    assert!(top >= 21, "top {top}");
    assert!(right <= 78, "right {right}");
    assert!(!records.iter().any(|d| d["code"] == "text_overflow"));
}

#[test]
fn round_rect_text_is_inset_by_the_corner_radius() {
    let text = "<a:r><a:t>AAAA AAAA AAAA AAAA AAAA AAAA</a:t></a:r>";
    // adj 50000 on a 100x40 box gives radius 20 and an inset of 0.29289 * 20 = 5.86px.
    let adj = r#"<a:gd name="adj" fmla="val 50000"/>"#;
    let (bounds, records) = blue_text_bounds("roundRect", adj, (0, 20, 100, 40), "t", text);
    let (left, _, right, _) = bounds.expect("text is drawn");
    // Left-aligned text starts at the inset (5.86px); the full box would start it at 0.
    assert!((5..=8).contains(&left) && right <= 94, "{left} {right}");
    assert!(!records.iter().any(|d| d["code"] == "text_overflow"));
    // Rounded rectangles keep the adjustment-formula diagnostic.
    let formula = r#"<a:gd name="adj" fmla="*/ w 1 4"/>"#;
    let (_, records) = blue_text_bounds("roundRect", formula, (0, 20, 100, 40), "ctr", text);
    assert!(records.iter().any(|d| {
        d["message"]
            .as_str()
            .unwrap()
            .contains("Rounded rectangle adjustment formulas are unsupported")
    }));
}

#[test]
fn small_ellipse_records_text_overflow_when_text_exceeds_the_inscribed_rectangle() {
    // Three 9pt lines are 36px tall: they fit the 40px box but not its 28.3px inscribed height.
    let lines =
        "<a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r>";
    let (_, records) = blue_text_bounds("ellipse", "", (30, 30, 40, 40), "t", lines);
    let overflow = records
        .iter()
        .filter(|d| d["code"] == "text_overflow")
        .count();
    assert_eq!(overflow, 1, "{records:?}");
    let (_, records) = blue_text_bounds("rect", "", (30, 30, 40, 40), "t", lines);
    assert!(!records.iter().any(|d| d["code"] == "text_overflow"));
}
/// A deck of `count` slides sharing one layout whose text box holds the given runs. The test font
/// has digits but not `‹#›`, so drawing the saved placeholder fails the conversion with a
/// missing-glyph error.
fn field_deck(
    path: &Path,
    count: usize,
    hidden: &[usize],
    first_slide_num: Option<&str>,
    field: &str,
) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    let ids: String = (1..=count)
        .map(|i| format!(r#"<p:sldId id="{}" r:id="s{i}"/>"#, 255 + i))
        .collect();
    let first = first_slide_num
        .map(|v| format!(r#" firstSlideNum="{v}""#))
        .unwrap_or_default();
    let slide_rels: String = (1..=count)
        .map(|i| {
            format!(r#"<Relationship Id="s{i}" Type="{R}/slide" Target="slides/slide{i}.xml"/>"#)
        })
        .collect();
    let font = r#"<a:rPr><a:latin typeface="Pptx Bullets"/></a:rPr>"#;
    let layout_box = format!(
        r#"<p:sp><p:spPr>{}<a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:p>{field}</a:p></p:txBody></p:sp>"#,
        transform(0, 0, 100, 50)
    )
    .replace("<a:rPr/>", font);
    let mut parts = vec![
        (
            "_rels/.rels".to_string(),
            rel("officeDocument", "ppt/presentation.xml"),
        ),
        (
            "ppt/presentation.xml".to_string(),
            format!(
                r#"<p:presentation xmlns:p="{P}" xmlns:r="{R}"{first}><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="952500" cy="952500"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels".to_string(),
            format!(r#"<Relationships xmlns="{PR}">{slide_rels}</Relationships>"#),
        ),
        (
            "ppt/layouts/layout.xml".to_string(),
            page("sldLayout", "", "", &layout_box),
        ),
        (
            "ppt/layouts/_rels/layout.xml.rels".to_string(),
            rel("slideMaster", "../masters/master.xml"),
        ),
        (
            "ppt/masters/master.xml".to_string(),
            page("sldMaster", "", "", ""),
        ),
    ];
    for i in 1..=count {
        let attrs = if hidden.contains(&i) {
            r#"show="0""#
        } else {
            ""
        };
        parts.push((
            format!("ppt/slides/slide{i}.xml"),
            page("sld", attrs, "", ""),
        ));
        parts.push((
            format!("ppt/slides/_rels/slide{i}.xml.rels"),
            rel("slideLayout", "../layouts/layout.xml"),
        ));
    }
    for (name, data) in parts {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}
/// Converts a field deck and returns the manifest.
fn convert_fields(
    count: usize,
    hidden: &[usize],
    first_slide_num: Option<&str>,
    field: &str,
) -> (serde_json::Value, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("fields.pptx");
    let out = dir.path().join("out");
    field_deck(&input, count, hidden, first_slide_num, field);
    let mut command = Command::new(env!("CARGO_BIN_EXE_rhodopsin"));
    // Hidden slides stay skipped, which is the case under test.
    command
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--width", "100", "--no-system-fonts", "--font-fallback"])
        .arg("--font-file")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/bullets.ttf"
        ));
    let output = command.output().unwrap();
    assert!(
        out.join("manifest.json").exists(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
    (manifest, dir)
}
/// How many glyphs were drawn on each rendered slide, keyed by slide number. Every glyph of the
/// test font is a separate box, so groups of dark columns count glyphs.
fn glyph_counts(manifest: &serde_json::Value, out: &Path) -> Vec<(u64, usize)> {
    manifest["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slide| {
            let number = slide["slide"].as_u64().unwrap();
            let img = Pixmap::load_png(out.join(format!("slide-{number:03}.png"))).unwrap();
            let mut groups = 0;
            let mut inside = false;
            for x in 0..img.width() {
                let dark = (0..img.height()).any(|y| rgb(&img, x, y).0 < 128);
                groups += usize::from(dark && !inside);
                inside = dark;
            }
            (number, groups)
        })
        .collect()
}
const SLIDE_NUMBER: &str = r#"<a:fld id="{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}" type="slidenum"><a:rPr/><a:t>‹#›</a:t></a:fld>"#;
#[test]
fn slide_number_field_is_resolved_and_the_saved_placeholder_is_not_drawn() {
    // Drawing `‹#›` would fail the conversion: the font has no such glyphs. Slides 9 and 10
    // tell apart one-digit and two-digit numbers.
    let (manifest, dir) = convert_fields(3, &[], Some("9"), SLIDE_NUMBER);
    assert_eq!(
        glyph_counts(&manifest, &dir.path().join("out")),
        [(1, 1), (2, 2), (3, 2)]
    );
    let (manifest, dir) = convert_fields(3, &[], None, SLIDE_NUMBER);
    assert_eq!(
        glyph_counts(&manifest, &dir.path().join("out")),
        [(1, 1), (2, 1), (3, 1)]
    );
}
#[test]
fn hidden_slides_still_count_in_the_numbering_of_later_slides() {
    // Slide 3 is number 10 when hidden slide 2 is counted; it would be 9 otherwise.
    let (manifest, dir) = convert_fields(3, &[2], Some("8"), SLIDE_NUMBER);
    assert_eq!(
        glyph_counts(&manifest, &dir.path().join("out")),
        [(1, 1), (3, 2)]
    );
}
#[test]
fn date_field_keeps_its_saved_text_with_one_diagnostic_per_slide() {
    let field = r#"<a:fld id="{1}" type="datetime1"><a:rPr/><a:t>AAA</a:t></a:fld><a:fld id="{2}" type="datetime4"><a:rPr/><a:t>AA</a:t></a:fld>"#;
    let (manifest, _dir) = convert_fields(2, &[], None, field);
    for slide in manifest["slides"].as_array().unwrap() {
        let notes: Vec<_> = slide["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| {
                d["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("Date and time fields")
            })
            .collect();
        assert_eq!(notes.len(), 1, "{slide}");
        assert_eq!(notes[0]["code"], "approximation");
    }
}
#[test]
fn other_field_types_keep_saved_text_and_name_the_type() {
    let field = r#"<a:fld id="{1}" type="slidetitle"><a:rPr/><a:t>AAA</a:t></a:fld>"#;
    let (manifest, _dir) = convert_fields(1, &[], None, field);
    let diagnostics = manifest["slides"][0]["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics.iter().any(|d| d["message"]
            .as_str()
            .unwrap()
            .starts_with("Field type slidetitle is not resolved")),
        "{diagnostics:?}"
    );
}
