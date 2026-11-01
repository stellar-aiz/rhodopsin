use image::{DynamicImage, ImageFormat, RgbImage, RgbaImage};
use rhodopsin::xlsx::{
    drawing::{Anchor, ImageCache, decode, read_sheet},
    package::{Limits, Package},
};
use std::{
    fs::File,
    io::{Cursor, Write},
    sync::Arc,
};
use tempfile::TempDir;
use zip::{ZipWriter, write::SimpleFileOptions};

fn png() -> Vec<u8> {
    let image = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

const ROOT: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><drawing r:id="rId1"/></worksheet>"#;
const FROM: &str = "<xdr:from><xdr:col>2</xdr:col><xdr:colOff>100</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>200</xdr:rowOff></xdr:from>";
const PIC: &str = r#"<xdr:pic><xdr:nvPicPr><xdr:cNvPr id="1" name="機密画像名"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic>"#;

fn package(body: &str, image: &[u8]) -> (TempDir, Package) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("input.xlsx");
    let drawing = format!(
        r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">{body}</xdr:wsDr>"#
    );
    let sheet_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#;
    let image_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>"#;
    let mut zip = ZipWriter::new(File::create(&path).unwrap());
    for (name, bytes) in [
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/drawings/drawing1.xml", drawing.as_bytes()),
        ("xl/drawings/_rels/drawing1.xml.rels", image_rels.as_bytes()),
        ("xl/media/image1.png", image),
    ] {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    let pkg = Package::open(&path, &Limits::default()).unwrap();
    (dir, pkg)
}

fn one_cell(pic: &str) -> String {
    format!(
        r#"<xdr:oneCellAnchor>{FROM}<xdr:ext cx="914400" cy="457200"/>{pic}<xdr:clientData/></xdr:oneCellAnchor>"#
    )
}

#[test]
fn セル位置と画像の透明度を保持し重複画像を共有する() {
    let anchor = one_cell(PIC);
    let (_dir, package) = package(&format!("{anchor}{anchor}"), &png());
    let root = roxmltree::Document::parse(ROOT).unwrap();
    let pictures = read_sheet(
        &package,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &Limits::default(),
        &mut ImageCache::default(),
    )
    .unwrap();
    assert_eq!(pictures.len(), 2);
    assert!(Arc::ptr_eq(&pictures[0].raster, &pictures[1].raster));
    assert_eq!(pictures[0].raster.rgba, [255, 0, 0, 255, 0, 0, 255, 128]);
    assert!(pictures[0].print && !pictures[0].hidden);
    match pictures[0].anchor {
        Anchor::OneCell {
            from,
            width_emu,
            height_emu,
        } => {
            assert_eq!(from.address(), "C7");
            assert_eq!((from.column_offset, from.row_offset), (100, 200));
            assert_eq!((width_emu, height_emu), (914400, 457200));
        }
        _ => panic!("固定位置が異なる"),
    }
}

#[test]
fn 二セル固定と印刷除外を保持する() {
    let anchor = format!(
        r#"<xdr:twoCellAnchor editAs="oneCell">{FROM}<xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>8</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>{PIC}<xdr:clientData fPrintsWithSheet="0"/></xdr:twoCellAnchor>"#
    );
    let (_dir, package) = package(&anchor, &png());
    let root = roxmltree::Document::parse(ROOT).unwrap();
    let pictures = read_sheet(
        &package,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &Limits::default(),
        &mut ImageCache::default(),
    )
    .unwrap();
    assert!(!pictures[0].print);
    assert!(matches!(
        pictures[0].anchor,
        Anchor::TwoCell {
            resize_with_cells: false,
            ..
        }
    ));
}

#[test]
fn 画像効果と図形の診断に顧客情報を含めない() {
    for (pic, reason) in [
        (
            PIC.replace("<xdr:spPr>", r#"<xdr:spPr><a:xfrm rot="60000"/>"#),
            "picture_transform",
        ),
        (
            PIC.replace("<a:stretch>", r#"<a:srcRect l="100000"/><a:stretch>"#),
            "invalid_picture_crop",
        ),
        (
            "<xdr:sp><xdr:txBody>機密文字列</xdr:txBody></xdr:sp>".into(),
            "shape_or_text_box",
        ),
    ] {
        let (_dir, package) = package(&one_cell(&pic), &png());
        let root = roxmltree::Document::parse(ROOT).unwrap();
        let error = read_sheet(
            &package,
            "xl/worksheets/sheet1.xml",
            root.root_element(),
            &Limits::default(),
            &mut ImageCache::default(),
        )
        .unwrap_err();
        assert_eq!(error.reason, reason);
        assert_eq!(error.location.as_ref().unwrap().cell.as_deref(), Some("C7"));
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
}

#[test]
fn 画像の寸法と総展開量を制限する() {
    assert_eq!(
        decode(&png(), 1, 10000).unwrap_err().reason,
        "image_expansion_limit"
    );
    assert_eq!(
        decode(&png(), 100, 7).unwrap_err().reason,
        "image_expansion_limit"
    );
    let (_dir, package) = package(&one_cell(PIC), &png());
    let root = roxmltree::Document::parse(ROOT).unwrap();
    let limits = Limits {
        total_image_bytes: 7,
        ..Limits::default()
    };
    let error = read_sheet(
        &package,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &limits,
        &mut ImageCache::default(),
    )
    .unwrap_err();
    assert_eq!(error.reason, "image_expansion_limit");
}

#[test]
fn 通常のjpegを読み破損画像を拒否する() {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, image::Rgb([255, 255, 255])))
        .write_to(&mut bytes, ImageFormat::Jpeg)
        .unwrap();
    let image = decode(bytes.get_ref(), 100, 100000).unwrap();
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(image.rgba.len(), 16);
    assert!(decode("機密".as_bytes(), 100, 10000).is_err());
    let mut truncated = png();
    truncated.truncate(40);
    assert!(decode(&truncated, 100, 10000).is_err());
}

#[test]
fn 同じ部品パスでも別の入力画像を取り違えない() {
    let (_first_dir, first) = package(&one_cell(PIC), &png());
    let mut alternate = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 1, image::Rgb([10, 20, 30])))
        .write_to(&mut alternate, ImageFormat::Png)
        .unwrap();
    let (_second_dir, second) = package(&one_cell(PIC), alternate.get_ref());
    let root = roxmltree::Document::parse(ROOT).unwrap();
    let mut cache = ImageCache::default();
    let first_picture = read_sheet(
        &first,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &Limits::default(),
        &mut cache,
    )
    .unwrap();
    let second_picture = read_sheet(
        &second,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &Limits::default(),
        &mut cache,
    )
    .unwrap();
    assert!(!Arc::ptr_eq(
        &first_picture[0].raster,
        &second_picture[0].raster
    ));
    assert_eq!(
        second_picture[0].raster.rgba,
        [10, 20, 30, 255, 10, 20, 30, 255]
    );
    assert_ne!(first_picture[0].raster.rgba, second_picture[0].raster.rgba);
}

#[test]
fn キャッシュ再利用でも縮小された上限を回避しない() {
    let (_dir, package) = package(&one_cell(PIC), &png());
    let root = roxmltree::Document::parse(ROOT).unwrap();
    let mut cache = ImageCache::default();
    let original = read_sheet(
        &package,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &Limits::default(),
        &mut cache,
    )
    .unwrap();
    for limits in [
        Limits {
            image_bytes: 7,
            ..Limits::default()
        },
        Limits {
            total_image_bytes: 7,
            ..Limits::default()
        },
        Limits {
            image_dimension: 1,
            ..Limits::default()
        },
    ] {
        let error = read_sheet(
            &package,
            "xl/worksheets/sheet1.xml",
            root.root_element(),
            &limits,
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(error.reason, "image_expansion_limit");
        assert_eq!(error.location.unwrap().cell.as_deref(), Some("C7"));
    }
    let repeated = read_sheet(
        &package,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &Limits::default(),
        &mut cache,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&original[0].raster, &repeated[0].raster));
}

#[test]
fn 同じ画像データは一度だけ数え異なる画像は合計上限へ加える() {
    let (_first_dir, first) = package(&one_cell(PIC), &png());
    let (_same_dir, same) = package(&one_cell(PIC), &png());
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 1, image::Rgb([10, 20, 30])))
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    let (_other_dir, other) = package(&one_cell(PIC), bytes.get_ref());
    let root = roxmltree::Document::parse(ROOT).unwrap();
    let mut cache = ImageCache::default();
    let limits = Limits {
        total_image_bytes: 8,
        ..Limits::default()
    };
    let first_picture = read_sheet(
        &first,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &limits,
        &mut cache,
    )
    .unwrap();
    let same_picture = read_sheet(
        &same,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &limits,
        &mut cache,
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        &first_picture[0].raster,
        &same_picture[0].raster
    ));
    assert_eq!(
        read_sheet(
            &other,
            "xl/worksheets/sheet1.xml",
            root.root_element(),
            &limits,
            &mut cache
        )
        .unwrap_err()
        .reason,
        "image_expansion_limit"
    );
    let larger = Limits {
        total_image_bytes: 16,
        ..limits
    };
    assert!(
        read_sheet(
            &other,
            "xl/worksheets/sheet1.xml",
            root.root_element(),
            &larger,
            &mut cache
        )
        .is_ok()
    );
}

#[test]
fn 切り取り範囲を画像ごとに保持してラスターを共有する() {
    let cropped = PIC.replace(
        "<a:stretch>",
        r#"<a:srcRect l="25000" r="10000" t="-5000" b="20000"/><a:stretch>"#,
    );
    let body = format!("{}{}", one_cell(PIC), one_cell(&cropped));
    let (_dir, package) = package(&body, &png());
    let root = roxmltree::Document::parse(ROOT).unwrap();
    let pictures = read_sheet(
        &package,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &Limits::default(),
        &mut ImageCache::default(),
    )
    .unwrap();
    assert_eq!(pictures[0].crop, Default::default());
    assert_eq!(
        pictures[1].crop,
        rhodopsin::xlsx::drawing::SourceCrop {
            left: 25000.0,
            right: 10000.0,
            top: -5000.0,
            bottom: 20000.0,
        }
    );
    assert!(Arc::ptr_eq(&pictures[0].raster, &pictures[1].raster));
    assert_eq!(pictures[1].raster.rgba, [255, 0, 0, 255, 0, 0, 255, 128]);
}

#[test]
fn 不正な切り取りや拡張を位置付きで拒否する() {
    for rect in [
        r#"<a:srcRect l="50000" r="50000"/>"#,
        r#"<a:srcRect t="60000" b="50000"/>"#,
        r#"<a:srcRect l="2147483648"/>"#,
        r#"<a:srcRect l="機密"/>"#,
        r#"<a:srcRect unknown="機密"/>"#,
        r#"<a:srcRect a:l="25000"/>"#,
        r#"<a:srcRect><a:extLst/></a:srcRect>"#,
    ] {
        let pic = PIC.replace("<a:stretch>", &format!("{rect}<a:stretch>"));
        let (_dir, package) = package(&one_cell(&pic), &png());
        let root = roxmltree::Document::parse(ROOT).unwrap();
        let error = read_sheet(
            &package,
            "xl/worksheets/sheet1.xml",
            root.root_element(),
            &Limits::default(),
            &mut ImageCache::default(),
        )
        .unwrap_err();
        assert_eq!(error.location.as_ref().unwrap().cell.as_deref(), Some("C7"));
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
}

#[test]
fn 百分率表記と整数表記が同じ切り取りを表し小数を保持する() {
    let mut body = String::new();
    for rect in [
        r#"<a:srcRect l="25000" r="10000" t="-5000" b="20000"/>"#,
        r#"<a:srcRect l="25%" r="10.000%" t="-5%" b="20%"/>"#,
        r#"<a:srcRect l="12.5005%" r="37.4995%" t="-0.0005%"/>"#,
    ] {
        body.push_str(&one_cell(
            &PIC.replace("<a:stretch>", &format!("{rect}<a:stretch>")),
        ));
    }
    let (_dir, package) = package(&body, &png());
    let root = roxmltree::Document::parse(ROOT).unwrap();
    let pictures = read_sheet(
        &package,
        "xl/worksheets/sheet1.xml",
        root.root_element(),
        &Limits::default(),
        &mut ImageCache::default(),
    )
    .unwrap();
    assert_eq!(pictures[0].crop, pictures[1].crop);
    assert_eq!(pictures[2].crop.left, 12500.5);
    assert_eq!(pictures[2].crop.right, 37499.5);
    assert_eq!(pictures[2].crop.top, -0.5);
    assert_eq!(pictures[2].crop.bottom, 0.0);
    assert!(Arc::ptr_eq(&pictures[0].raster, &pictures[2].raster));
}

#[test]
fn 不正な百分率の文法と空になる切り取りを拒否する() {
    let huge = format!("-{}%", "9".repeat(400));
    for value in [
        "25.5", "+25%", ".5%", "25.%", "1e1%", "NaN%", "inf%", "25%%", " 25%", "25% ", "--25%",
        "機密%", "100%", &huge,
    ] {
        let pic = PIC.replace(
            "<a:stretch>",
            &format!(r#"<a:srcRect l="{value}"/><a:stretch>"#),
        );
        let (_dir, package) = package(&one_cell(&pic), &png());
        let root = roxmltree::Document::parse(ROOT).unwrap();
        let error = read_sheet(
            &package,
            "xl/worksheets/sheet1.xml",
            root.root_element(),
            &Limits::default(),
            &mut ImageCache::default(),
        )
        .unwrap_err();
        assert_eq!(error.reason, "invalid_picture_crop");
        assert_eq!(error.location.as_ref().unwrap().cell.as_deref(), Some("C7"));
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    }
}
