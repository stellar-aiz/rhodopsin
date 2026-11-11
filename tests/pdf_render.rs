#![cfg(feature = "pdf-input")]
use rhodopsin::pdf::{Options, render};
use std::fs;
fn pdf(content: &str, extra: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R {extra} >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut data = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(data.len());
        data.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
    }
    let xref = data.len();
    data.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for off in &offsets[1..] {
        data.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    data.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    data
}
fn run(content: &str, extra: &str) -> Result<resvg::tiny_skia::Pixmap, String> {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("in.pdf"), pdf(content, extra)).unwrap();
    render(
        dir.path().join("in.pdf"),
        dir.path().join("out"),
        &Options {
            width: 100,
            strict: false,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(resvg::tiny_skia::Pixmap::load_png(dir.path().join("out/page-001.png")).unwrap())
}
#[test]
fn independent_path_clip_transform_and_state_oracle() {
    let p=run("q 1 0 0 1 10 10 cm 0 0 m 40 0 l 40 40 l 0 40 l h W n 1 0 0 rg 0 0 m 90 0 l 90 90 l 0 90 l h f Q 0 0 1 rg 70 70 m 90 70 l 90 90 l 70 90 l h f", "").unwrap();
    assert_eq!(p.pixel(20, 70).unwrap().red(), 255);
    assert_eq!(p.pixel(20, 70).unwrap().green(), 0);
    assert_eq!(p.pixel(60, 60).unwrap().green(), 255);
    assert_eq!(p.pixel(80, 20).unwrap().blue(), 255);
    assert_eq!(p.pixel(80, 20).unwrap().red(), 0);
}
#[test]
fn unsupported_geometry_and_annotations_fail() {
    for extra in [
        "/Rotate 90",
        "/CropBox [0 0 50 50]",
        "/UserUnit 2",
        "/Annots [9 0 R]",
        "/Group << /S /Transparency >>",
    ] {
        assert!(run("", extra).is_err(), "{extra}");
    }
    assert!(run("", "/Rotate 0 /UserUnit 1 /CropBox [0 0 100 100]").is_ok());
}
#[test]
fn unknown_ops_and_unbalanced_programs_fail_without_outputs() {
    for content in [
        "secret_operator",
        "q",
        "Q",
        "BT",
        "EMC",
        "1 2 m",
        "0 0 m 2 2 l S Q",
    ] {
        assert!(run(content, "").is_err(), "{content}");
    }
}
#[test]
fn output_exists_and_late_failure_preserve_destination() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.pdf");
    let out = dir.path().join("out");
    fs::write(&input, pdf("", "")).unwrap();
    fs::create_dir(&out).unwrap();
    fs::write(out.join("keep"), "keep").unwrap();
    assert!(render(&input, &out, &Default::default()).is_err());
    assert_eq!(fs::read_to_string(out.join("keep")).unwrap(), "keep");
    let failed = dir.path().join("failed");
    fs::write(&input, pdf("q", "")).unwrap();
    assert!(render(&input, &failed, &Default::default()).is_err());
    assert!(!failed.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn nesting_and_page_pixel_limits_fail() {
    assert!(run(&"q ".repeat(65), "").is_err());
    assert!(run("", "/MediaBox [0 0 0 100]").is_err());
    assert!(run("", "/MediaBox [0 0 1 14400]").is_err());
}
#[test]
fn cli_routes_pdf_and_rejects_pptx_options() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.PDF");
    fs::write(&input, pdf("", "")).unwrap();
    let binary = env!("CARGO_BIN_EXE_rhodopsin");
    let status = std::process::Command::new(binary)
        .arg(&input)
        .args(["--width", "100"])
        .output()
        .unwrap();
    assert!(status.status.success(), "{:?}", status);
    assert!(dir.path().join("input.rhodopsin/page-001.png").exists());
    for option in ["--include-hidden", "--no-system-fonts"] {
        assert!(
            !std::process::Command::new(binary)
                .arg(&input)
                .arg(option)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

fn assemble(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut data = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(data.len());
        data.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        data.extend_from_slice(obj);
        data.extend_from_slice(b"\nendobj\n");
    }
    let xref = data.len();
    data.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for off in &offsets[1..] {
        data.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    data.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    data
}
fn stream(bytes: &[u8], dict: &str) -> Vec<u8> {
    let mut s = format!("<< /Length {} {dict} >>\nstream\n", bytes.len()).into_bytes();
    s.extend_from_slice(bytes);
    s.extend_from_slice(b"\nendstream");
    s
}
fn image_pdf(bytes: &[u8], extra: &str) -> Vec<u8> {
    assemble(vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /I 5 0 R >> >> /Contents 4 0 R >>".to_vec(),stream(b"q 100 0 0 100 0 0 cm /I Do Q",""),stream(bytes,&format!("/Type /XObject /Subtype /Image /Width 1 /Height 2 /BitsPerComponent 8 {} {} {extra}", if extra.contains("/ColorSpace") {""} else {"/ColorSpace /DeviceRGB"}, if extra.contains("/Filter") {""} else {"/Filter /FlateDecode"}))])
}
#[test]
fn independent_image_orientation_and_decompression_limit() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("image.pdf");
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&[255, 0, 0, 0, 0, 255], 6);
    fs::write(&input, image_pdf(&compressed, "")).unwrap();
    render(
        &input,
        dir.path().join("out"),
        &Options {
            width: 100,
            strict: false,
        },
    )
    .unwrap();
    let p = resvg::tiny_skia::Pixmap::load_png(dir.path().join("out/page-001.png")).unwrap();
    assert!(p.pixel(50, 10).unwrap().red() > 240);
    assert!(p.pixel(50, 90).unwrap().blue() > 240);
    let bomb = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0; 16 * 1024 * 1024 + 1], 1);
    fs::write(&input, image_pdf(&bomb, "")).unwrap();
    let error = render(&input, dir.path().join("bomb"), &Default::default()).unwrap_err();
    assert!(error.to_string().contains("oversized"));
    assert!(!dir.path().join("bomb").exists());
}
#[test]
fn masks_and_unsupported_filters_fail_in_both_modes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.pdf");
    for strict in [false, true] {
        for extra in [
            "/SMask 9 0 R",
            "/Filter /SecretFilter",
            "/ColorSpace /DeviceCMYK",
        ] {
            fs::write(&input, image_pdf(&[1, 2, 3], extra)).unwrap();
            assert!(
                render(
                    &input,
                    dir.path().join("out"),
                    &Options { width: 100, strict }
                )
                .is_err()
            );
            assert!(!dir.path().join("out").exists());
        }
    }
}
#[test]
fn page_tree_cycles_and_missing_objects_are_rejected() {
    for pages in [
        "<< /Type /Pages /Kids [2 0 R] >>",
        "<< /Type /Pages /Kids [99 0 R] >>",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("bad.pdf");
        fs::write(
            &input,
            assemble(vec![
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                pages.as_bytes().to_vec(),
            ]),
        )
        .unwrap();
        assert!(render(input, dir.path().join("out"), &Default::default()).is_err());
    }
}

#[test]
fn used_unsupported_resources_fail_but_unused_resources_do_not() {
    for (program, resource, expected) in [
        ("", "<< /Type /Font /Subtype /Type1 >>", true),
        ("BT /F 12 Tf ET", "<< /Type /Font /Subtype /Type1 >>", false),
        ("/G gs", "<< /Type /ExtGState /ca 0.5 >>", false),
        (
            "/G gs",
            "<< /Type /ExtGState /ca 1 /CA 1 /BM /Normal >>",
            true,
        ),
    ] {
        let bytes=assemble(vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] /Rotate 0 /Resources << /Font << /F 5 0 R >> /ExtGState << /G 5 0 R >> >> >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),stream(program.as_bytes(),""),resource.as_bytes().to_vec()]);
        for strict in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let input = dir.path().join("in.pdf");
            fs::write(&input, &bytes).unwrap();
            assert_eq!(
                render(
                    &input,
                    dir.path().join("out"),
                    &Options { width: 100, strict }
                )
                .is_ok(),
                expected
            );
        }
    }
}
#[test]
fn later_page_failure_cleans_previously_staged_page() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.pdf");
    fs::write(&input,assemble(vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 /MediaBox [0 0 100 100] /Resources << >> >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),stream(b"",""),
        b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".to_vec(),stream(b"secret_operator","")])).unwrap();
    let err = render(input, dir.path().join("out"), &Default::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("page 2"));
    assert!(!err.contains("secret_operator"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn input_width_boundaries_and_oversized_input() {
    for width in [63, 64, 4096, 4097] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.pdf");
        fs::write(&input, pdf("", "")).unwrap();
        assert_eq!(
            render(
                input,
                dir.path().join("out"),
                &Options {
                    width,
                    strict: false
                }
            )
            .is_ok(),
            (64..=4096).contains(&width)
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("large.pdf");
    fs::File::create(&input)
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    assert!(
        render(input, dir.path().join("out"), &Default::default())
            .unwrap_err()
            .to_string()
            .contains("Input byte limit")
    );
}

#[test]
fn even_odd_fill_and_stroke_have_independent_pixel_oracles() {
    let p=run("1 0 0 rg 10 10 m 90 10 l 90 90 l 10 90 l h 30 30 m 70 30 l 70 70 l 30 70 l h f* 0 0 0 RG 4 w 1 J 1 j 10 M [] 0 d 50 40 m 50 60 l S", "").unwrap();
    assert_eq!(p.pixel(20, 50).unwrap().green(), 0);
    assert_eq!(p.pixel(40, 50).unwrap().green(), 255);
    assert_eq!(p.pixel(50, 50).unwrap().red(), 0);
}
#[test]
fn encrypted_trailer_is_rejected_without_document_text() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.pdf");
    let bytes = String::from_utf8(pdf("", ""))
        .unwrap()
        .replace("/Size 5 /Root", "/Encrypt 9 0 R /Size 5 /Root");
    fs::write(&input, bytes).unwrap();
    assert!(
        render(input, dir.path().join("out"), &Default::default())
            .unwrap_err()
            .to_string()
            .contains("Encrypted")
    );
}

#[test]
fn independent_type3_font_matrix_and_character_positioning() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("font.pdf");
    fs::write(&input,assemble(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(b"1 0 0 rg BT /F 20 Tf 1 0 0 1 10 10 Tm <4141> Tj ET",""),
        b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 100 100] /FontMatrix [0.01 0 0 0.01 0 0] /Encoding << /Differences [65 /g] >> /FirstChar 65 /LastChar 65 /Widths [150] /Resources << >> /CharProcs << /g 6 0 R >> >>".to_vec(),
        stream(b"150 0 d0 0 0 m 100 0 l 100 100 l 0 100 l h f", "")])).unwrap();
    render(
        input,
        dir.path().join("out"),
        &Options {
            width: 100,
            strict: false,
        },
    )
    .unwrap();
    let p = resvg::tiny_skia::Pixmap::load_png(dir.path().join("out/page-001.png")).unwrap();
    assert_eq!(p.pixel(20, 80).unwrap().green(), 0);
    assert_eq!(p.pixel(35, 80).unwrap().green(), 255);
    assert_eq!(p.pixel(50, 80).unwrap().green(), 0);
}
