#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::{
    fonts::{FontBook, FontLimits},
    pdf::{self, Item, Limits, Page, Text},
};
fn book() -> FontBook {
    let mut book = FontBook::default();
    book.load_bytes(
        support::font("試験", false, false, 0),
        &FontLimits::default(),
    )
    .unwrap();
    book
}
fn pages() -> Vec<Page<'static>> {
    vec![Page {
        width: 595.0,
        height: 842.0,
        items: vec![Item::Text(Text {
            text: "見 123",
            font: 0,
            points: 12.0,
            x: 30.0,
            baseline: 40.0,
            color: [0, 0, 0],
            clip: None,
        })],
    }]
}
#[test]
fn 文字抽出と埋め込みと決定性() {
    let fonts = book();
    let pages = pages();
    let bytes = pdf::render(&pages, &fonts, &Limits::default(), &FontLimits::default()).unwrap();
    assert_eq!(
        bytes,
        pdf::render(&pages, &fonts, &Limits::default(), &FontLimits::default()).unwrap()
    );
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.get_pages().len(), 1);
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "見 123");
    assert!(
        doc.objects
            .values()
            .any(|o| o.as_dict().is_ok_and(|d| d.has(b"FontFile2")))
    );
    assert!(
        doc.objects
            .values()
            .any(|o| o.as_dict().is_ok_and(|d| d.has(b"ToUnicode")))
    );
}
#[test]
fn 欠落文字と上限を拒否する() {
    let fonts = book();
    let mut pages = pages();
    let limits = Limits {
        glyphs: 1,
        ..Limits::default()
    };
    assert_eq!(
        pdf::render(&pages, &fonts, &limits, &FontLimits::default())
            .unwrap_err()
            .reason,
        "pdf_glyph_limit"
    );
    let limits = Limits {
        output_bytes: 1,
        ..Limits::default()
    };
    assert_eq!(
        pdf::render(&pages, &fonts, &limits, &FontLimits::default())
            .unwrap_err()
            .reason,
        "pdf_output_limit"
    );
    if let Item::Text(text) = &mut pages[0].items[0] {
        text.text = "未";
    }
    assert_eq!(
        pdf::render(&pages, &fonts, &Limits::default(), &FontLimits::default())
            .unwrap_err()
            .reason,
        "font_glyph_missing"
    );
}

#[test]
fn 塗り文字罫線画像の順序と透明度を保持する() {
    use rhodopsin::xlsx::{
        drawing::Raster,
        pdf::graphics::{BoxRect, Paint},
    };
    let raster = Raster {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 64],
    };
    let rect = BoxRect {
        x: 10.0,
        y: 20.0,
        width: 40.0,
        height: 30.0,
    };
    let mut pages = pages();
    pages[0].items.insert(
        0,
        Item::Paint(Paint::Fill {
            rect,
            color: [250, 250, 200],
        }),
    );
    pages[0].items.push(Item::Paint(Paint::Line {
        from: [10.0, 50.0],
        to: [50.0, 50.0],
        width: 0.5,
        color: [0, 0, 0],
        dash: vec![3.0, 1.0],
    }));
    pages[0].items.push(Item::Paint(Paint::Image {
        rect,
        raster: &raster,
        clip: None,
    }));
    let bytes = pdf::render(&pages, &book(), &Limits::default(), &FontLimits::default()).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let page_id = doc.get_pages()[&1];
    let ops = lopdf::content::Content::decode(&doc.get_page_content(page_id).unwrap())
        .unwrap()
        .operations;
    let operators: Vec<_> = ops.iter().map(|o| o.operator.as_str()).collect();
    let pos = |op| operators.iter().position(|v| *v == op).unwrap();
    assert!(pos("f") < pos("BT") && pos("ET") < pos("S") && pos("S") < pos("Do"));
    let rect_op = &ops[pos("re")];
    assert_eq!(
        rect_op
            .operands
            .iter()
            .map(|v| v.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![10.0, 792.0, 40.0, 30.0]
    );
    let image = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| s.dict.has(b"SMask"))
        .unwrap();
    assert_eq!(
        image.decompressed_content().unwrap(),
        vec![255, 0, 0, 0, 255, 0]
    );
    let mask = doc
        .get_object(image.dict.get(b"SMask").unwrap().as_reference().unwrap())
        .unwrap()
        .as_stream()
        .unwrap();
    assert_eq!(mask.decompressed_content().unwrap(), vec![255, 64]);
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "見 123");
}

#[test]
fn 不正な描画データは出力しない() {
    use rhodopsin::xlsx::{
        drawing::Raster,
        pdf::graphics::{BoxRect, Paint},
    };
    let raster = Raster {
        width: 2,
        height: 1,
        rgba: vec![0; 4],
    };
    let rect = BoxRect {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    };
    let pages = vec![Page {
        width: 100.0,
        height: 100.0,
        items: vec![Item::Paint(Paint::Image {
            rect,
            raster: &raster,
            clip: None,
        })],
    }];
    assert_eq!(
        pdf::render(&pages, &book(), &Limits::default(), &FontLimits::default())
            .unwrap_err()
            .reason,
        "invalid_pdf_image"
    );
    let pages = vec![Page {
        width: 100.0,
        height: 100.0,
        items: vec![Item::Paint(Paint::Line {
            from: [0.0, 0.0],
            to: [10.0, 10.0],
            width: 0.0,
            color: [0; 3],
            dash: vec![],
        })],
    }];
    assert_eq!(
        pdf::render(&pages, &book(), &Limits::default(), &FontLimits::default())
            .unwrap_err()
            .reason,
        "invalid_pdf_line"
    );
}

#[test]
fn 文字の切断領域が後続の描画へ漏れない() {
    use rhodopsin::xlsx::pdf::graphics::BoxRect;
    let mut pages = pages();
    if let Item::Text(text) = &mut pages[0].items[0] {
        text.clip = Some(BoxRect {
            x: 30.0,
            y: 20.0,
            width: 10.0,
            height: 30.0,
        });
    }
    let bytes = pdf::render(&pages, &book(), &Limits::default(), &FontLimits::default()).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let ops = lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
        .unwrap()
        .operations;
    let operators: Vec<_> = ops.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(&operators[..4], &["q", "re", "W", "n"]);
    assert_eq!(&operators[operators.len() - 2..], &["ET", "Q"]);
}

#[test]
fn 同じ画像を全ページで共有して可逆圧縮する() {
    use rhodopsin::xlsx::{
        drawing::Raster,
        pdf::graphics::{BoxRect, Paint},
    };
    let raster = Raster {
        width: 100,
        height: 100,
        rgba: [12, 34, 56, 128].repeat(10_000),
    };
    let rect = BoxRect {
        x: 10.0,
        y: 20.0,
        width: 100.0,
        height: 100.0,
    };
    let make_pages = |image| {
        (0..3)
            .map(|_| Page {
                width: 200.0,
                height: 200.0,
                items: vec![Item::Paint(Paint::Image {
                    rect,
                    raster: image,
                    clip: None,
                })],
            })
            .collect::<Vec<_>>()
    };
    let pages = make_pages(&raster);
    let limits = Limits {
        image_bytes: raster.rgba.len(),
        ..Limits::default()
    };
    let bytes = pdf::render(&pages, &book(), &limits, &FontLimits::default()).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let streams: Vec<_> = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| {
            s.dict
                .get(b"Subtype")
                .is_ok_and(|v| v.as_name().is_ok_and(|v| v == b"Image"))
        })
        .collect();
    assert_eq!(streams.len(), 2); // RGB と透明度。ページごとに増えない。
    for stream in streams {
        assert_eq!(
            stream.dict.get(b"Filter").unwrap().as_name().unwrap(),
            b"FlateDecode"
        );
        let raw = stream.decompressed_content().unwrap();
        assert!(stream.content.len() < raw.len() / 10);
        if stream.dict.has(b"SMask") {
            assert_eq!(raw, [12, 34, 56].repeat(10_000));
        } else {
            assert_eq!(raw, vec![128; 10_000]);
        }
    }
    // 別アドレスの同内容入力でも出力バイト列は変わらない。
    let copy = Raster {
        width: raster.width,
        height: raster.height,
        rgba: raster.rgba.clone(),
    };
    assert_eq!(
        bytes,
        pdf::render(&make_pages(&copy), &book(), &limits, &FontLimits::default()).unwrap()
    );
}

/// 空の字形を持つ独自 CFF。商用フォントのデータを含まない。
fn cff_font(alias: bool) -> Vec<u8> {
    let mut original = support::font("CFF試験", false, false, 0);
    if alias {
        let face = rustybuzz::ttf_parser::RawFace::parse(&original, 0).unwrap();
        let offset = face
            .table_records
            .into_iter()
            .find(|r| r.tag == rustybuzz::ttf_parser::Tag::from_bytes(b"cmap"))
            .unwrap()
            .offset as usize;
        for (i, value) in [65u32, 65, 34, 66, 66, 34].into_iter().enumerate() {
            original[offset + 28 + i * 4..offset + 32 + i * 4]
                .copy_from_slice(&value.to_be_bytes());
        }
    }
    let raw = rustybuzz::ttf_parser::RawFace::parse(&original, 0).unwrap();
    let mut cff = vec![1, 0, 4, 4];
    cff.extend([0, 1, 1, 1, 5]);
    cff.extend(b"Test");
    cff.extend([0, 1, 1, 1, 7, 29, 0, 0, 0, 28, 17]);
    cff.extend([0, 0, 0, 0]);
    assert_eq!(cff.len(), 28);
    cff.extend([0, 97, 1]);
    cff.extend(1..=98);
    cff.extend(vec![14; 97]);
    let mut tables: Vec<([u8; 4], Vec<u8>)> = raw
        .table_records
        .into_iter()
        .filter(|r| {
            r.tag != rustybuzz::ttf_parser::Tag::from_bytes(b"glyf")
                && r.tag != rustybuzz::ttf_parser::Tag::from_bytes(b"loca")
        })
        .map(|r| (r.tag.to_bytes(), raw.table(r.tag).unwrap().to_vec()))
        .collect();
    tables.push((*b"CFF ", cff));
    tables.sort_by_key(|v| v.0);
    let mut out = vec![0; 12 + tables.len() * 16];
    out[..4].copy_from_slice(b"OTTO");
    out[4..6].copy_from_slice(&(tables.len() as u16).to_be_bytes());
    for (i, (tag, data)) in tables.into_iter().enumerate() {
        let at = 12 + i * 16;
        let offset = out.len() as u32;
        out[at..at + 4].copy_from_slice(&tag);
        out[at + 8..at + 12].copy_from_slice(&offset.to_be_bytes());
        out[at + 12..at + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    out
}
#[test]
fn cffの埋め込みと文字抽出を検証する() {
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(cff_font(false), &FontLimits::default())
        .unwrap();
    let bytes = pdf::render(&pages(), &fonts, &Limits::default(), &FontLimits::default()).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "見 123");
    let embedded = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| {
            s.dict
                .get(b"Subtype")
                .is_ok_and(|v| v.as_name().is_ok_and(|n| n == b"CIDFontType0C"))
        })
        .unwrap();
    let cff = rustybuzz::ttf_parser::cff::Table::parse(&embedded.content).unwrap();
    assert_eq!(cff.number_of_glyphs(), 6); // 使用した 5 字形と .notdef。
    assert_eq!(
        bytes,
        pdf::render(&pages(), &fonts, &Limits::default(), &FontLimits::default()).unwrap()
    );
}

#[test]
fn cffで同一字形に対応する異なる文字も復元する() {
    let mut fonts = FontBook::default();
    fonts
        .load_bytes(cff_font(true), &FontLimits::default())
        .unwrap();
    let mut pages = pages();
    if let Item::Text(text) = &mut pages[0].items[0] {
        text.text = "AB";
    }
    let bytes = pdf::render(&pages, &fonts, &Limits::default(), &FontLimits::default()).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), "AB");
    let encoding = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| String::from_utf8_lossy(&s.content).contains("begincidchar"))
        .unwrap();
    let source = String::from_utf8_lossy(&encoding.content);
    assert!(source.contains("<0001> 1\n<0002> 1\n"));
}

#[test]
fn ページ境界の画像を倍率を変えずに切り取る() {
    use rhodopsin::xlsx::{
        drawing::Raster,
        pdf::graphics::{BoxRect, Paint},
    };
    let raster = Raster {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
    };
    let clip = BoxRect {
        x: 10.0,
        y: 20.0,
        width: 80.0,
        height: 40.0,
    };
    let pages: Vec<_> = [10.0, -70.0]
        .into_iter()
        .map(|x| Page {
            width: 100.0,
            height: 100.0,
            items: vec![
                Item::Paint(Paint::Image {
                    rect: BoxRect {
                        x,
                        y: 20.0,
                        width: 160.0,
                        height: 40.0,
                    },
                    raster: &raster,
                    clip: Some(clip),
                }),
                Item::Paint(Paint::Fill {
                    rect: clip,
                    color: [255; 3],
                }),
            ],
        })
        .collect();
    let bytes = pdf::render(&pages, &book(), &Limits::default(), &FontLimits::default()).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    for ((_, id), x) in doc.get_pages().into_iter().zip([10.0, -70.0]) {
        let content = lopdf::content::Content::decode(&doc.get_page_content(id).unwrap()).unwrap();
        let ops = &content.operations;
        let names: Vec<_> = ops.iter().map(|op| op.operator.as_str()).collect();
        assert_eq!(&names[..7], ["q", "re", "W", "n", "cm", "Do", "Q"]);
        assert_eq!(
            ops[1]
                .operands
                .iter()
                .map(|n| n.as_float().unwrap())
                .collect::<Vec<_>>(),
            [10.0, 40.0, 80.0, 40.0]
        );
        assert_eq!(
            ops[4]
                .operands
                .iter()
                .map(|n| n.as_float().unwrap())
                .collect::<Vec<_>>(),
            [160.0, 0.0, 0.0, 40.0, x, 40.0]
        );
        assert_eq!(names.iter().filter(|name| **name == "W").count(), 1);
    }
    // 切り取りが異なっても同じ画素データを再埋め込みしない。
    let images = doc
        .objects
        .values()
        .filter(|object| {
            object.as_stream().ok().is_some_and(|stream| {
                stream
                    .dict
                    .get(b"Subtype")
                    .ok()
                    .and_then(|value| value.as_name().ok())
                    == Some(b"Image".as_slice())
            })
        })
        .count();
    assert_eq!(images, 1);
}

#[test]
fn 画像の共有があっても不正な切り取り矩形を拒否する() {
    use rhodopsin::xlsx::{
        drawing::Raster,
        pdf::graphics::{BoxRect, Paint},
    };
    let raster = Raster {
        width: 1,
        height: 1,
        rgba: vec![0, 0, 0, 255],
    };
    let rect = BoxRect {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    };
    let pages = [Page {
        width: 100.0,
        height: 100.0,
        items: vec![
            Item::Paint(Paint::Image {
                rect,
                raster: &raster,
                clip: None,
            }),
            Item::Paint(Paint::Image {
                rect,
                raster: &raster,
                clip: Some(BoxRect { width: 0.0, ..rect }),
            }),
        ],
    }];
    assert_eq!(
        pdf::render(&pages, &book(), &Limits::default(), &FontLimits::default())
            .unwrap_err()
            .reason,
        "invalid_pdf_rectangle"
    );
}

#[test]
fn pdf文字量は全ページのutf8合計を整形前に検査する() {
    let fonts = book();
    let mut pages = pages();
    let bytes = "見 123".len();
    assert!(
        pdf::render(
            &pages,
            &fonts,
            &Limits {
                text_bytes: bytes,
                ..Default::default()
            },
            &FontLimits::default()
        )
        .is_ok()
    );
    assert_eq!(
        pdf::render(
            &pages,
            &fonts,
            &Limits {
                text_bytes: bytes - 1,
                ..Default::default()
            },
            &FontLimits::default()
        )
        .unwrap_err()
        .reason,
        "pdf_text_limit"
    );
    pages.push(Page {
        width: 595.0,
        height: 842.0,
        items: vec![Item::Text(Text {
            text: "見 123",
            font: 999,
            points: 12.0,
            x: 30.0,
            baseline: 40.0,
            color: [0; 3],
            clip: None,
        })],
    });
    // 上限超過後のフォント参照・整形へ進まない。ページごとに予算をリセットしない。
    assert_eq!(
        pdf::render(
            &pages,
            &fonts,
            &Limits {
                text_bytes: bytes,
                ..Default::default()
            },
            &FontLimits::default()
        )
        .unwrap_err()
        .reason,
        "pdf_text_limit"
    );
    if let Item::Text(text) = &mut pages[1].items[0] {
        text.font = 0;
    }
    assert!(
        pdf::render(
            &pages,
            &fonts,
            &Limits {
                text_bytes: bytes * 2,
                ..Default::default()
            },
            &FontLimits::default()
        )
        .is_ok()
    );
}

fn combining_book() -> FontBook {
    let mut data = support::font("結合文字試験", false, false, 0);
    // 試験フォントの第二 cmap 群を結合文字範囲へ変更する。字形は空のまま。
    let table = data[12..]
        .chunks_exact(16)
        .find(|entry| &entry[..4] == b"cmap")
        .unwrap();
    let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
    data[offset + 40..offset + 44].copy_from_slice(&0x300u32.to_be_bytes());
    data[offset + 44..offset + 48].copy_from_slice(&0x327u32.to_be_bytes());
    data[offset + 48..offset + 52].copy_from_slice(&57u32.to_be_bytes());
    let mut fonts = FontBook::default();
    fonts.load_bytes(data, &FontLimits::default()).unwrap();
    fonts
}

#[test]
fn 結合文字の同一クラスタを字形位置と抽出文字列を保って保存する() {
    let fonts = combining_book();
    let text = "x\u{301} 12";
    let run = fonts.faces[0]
        .shape_run(text, 12.0, &FontLimits::default())
        .unwrap();
    assert_eq!(run.glyphs[0].cluster, 0);
    assert_eq!(run.glyphs[1].cluster, 0);
    assert_eq!(run.glyphs[1].advance_x, 0.0);
    let mut pages = pages();
    if let Item::Text(item) = &mut pages[0].items[0] {
        item.text = text;
    }
    let bytes = pdf::render(&pages, &fonts, &Limits::default(), &FontLimits::default()).unwrap();
    assert_eq!(
        bytes,
        pdf::render(&pages, &fonts, &Limits::default(), &FontLimits::default()).unwrap()
    );
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), text);
    let page_id = doc.get_pages()[&1];
    let content = lopdf::content::Content::decode(&doc.get_page_content(page_id).unwrap()).unwrap();
    let matrices: Vec<_> = content
        .operations
        .iter()
        .filter(|op| op.operator == "Tm")
        .collect();
    assert_eq!(matrices.len(), run.glyphs.len());
    let mut x = 30.0;
    let mut y = 842.0 - 40.0;
    for (op, glyph) in matrices.iter().zip(&run.glyphs) {
        assert_eq!(
            op.operands[4].as_float().unwrap(),
            (x + glyph.offset_x) as f32
        );
        assert_eq!(
            op.operands[5].as_float().unwrap(),
            (y + glyph.offset_y) as f32
        );
        x += glyph.advance_x;
        y += glyph.advance_y;
    }
}

#[test]
fn 並べ替えられた結合文字の位置と元文字列の順序を保持する() {
    let fonts = combining_book();
    let text = "x\u{301}\u{327}";
    let run = fonts.faces[0]
        .shape_run(text, 12.0, &FontLimits::default())
        .unwrap();
    let face = rustybuzz::ttf_parser::Face::parse(fonts.faces[0].data(), 0).unwrap();
    assert_eq!(run.glyphs.len(), 3);
    // canonical combining class によりアクセントとセディーユの字形順が交換される。
    assert_eq!(
        run.glyphs[1].id,
        u32::from(face.glyph_index('\u{327}').unwrap().0)
    );
    assert_eq!(
        run.glyphs[2].id,
        u32::from(face.glyph_index('\u{301}').unwrap().0)
    );
    let mut pages = pages();
    if let Item::Text(item) = &mut pages[0].items[0] {
        item.text = text;
    }
    let bytes = pdf::render(&pages, &fonts, &Limits::default(), &FontLimits::default()).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), text);
    let content =
        lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
            .unwrap();
    let matrices: Vec<_> = content
        .operations
        .iter()
        .filter(|op| op.operator == "Tm")
        .collect();
    let mut x = 30.0;
    let mut y = 842.0 - 40.0;
    let mut positions = Vec::new();
    for glyph in &run.glyphs {
        positions.push(((x + glyph.offset_x) as f32, (y + glyph.offset_y) as f32));
        x += glyph.advance_x;
        y += glyph.advance_y;
    }
    assert_eq!(matrices.len(), 3);
    for (matrix, index) in matrices.iter().zip([0, 2, 1]) {
        assert_eq!(matrix.operands[4].as_float().unwrap(), positions[index].0);
        assert_eq!(matrix.operands[5].as_float().unwrap(), positions[index].1);
    }
    assert_eq!(
        bytes,
        pdf::render(&pages, &fonts, &Limits::default(), &FontLimits::default()).unwrap()
    );
}

fn line_text(value: &'static str, x: f64) -> Text<'static> {
    Text {
        text: value,
        font: 0,
        points: 12.0,
        x,
        baseline: 40.0,
        color: [0, 0, 0],
        clip: None,
    }
}
#[test]
fn 同一行の書式境界は抽出改行にせず字形座標を保つ() {
    let fonts = book();
    let mut b = line_text("B", 45.0);
    b.color = [255, 0, 0];
    b.points = 16.0;
    let page = Page {
        width: 100.0,
        height: 100.0,
        items: vec![Item::TextLine(vec![line_text("A", 30.0), b])],
    };
    let bytes = pdf::render(&[page], &fonts, &Limits::default(), &FontLimits::default()).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap(), "AB\n");
    let id = *doc.get_pages().get(&1).unwrap();
    let content = lopdf::content::Content::decode(&doc.get_page_content(id).unwrap()).unwrap();
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|op| op.operator == "BT")
            .count(),
        1
    );
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|op| op.operator == "ET")
            .count(),
        1
    );
    let positions: Vec<_> = content
        .operations
        .iter()
        .filter(|op| op.operator == "Tm")
        .map(|op| {
            (
                op.operands[4].as_float().unwrap(),
                op.operands[5].as_float().unwrap(),
            )
        })
        .collect();
    assert_eq!(positions, vec![(30.0, 60.0), (45.0, 60.0)]);
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|op| op.operator == "Tf")
            .count(),
        2
    );
}

#[test]
fn 文字行の内側にも項目文字量字形上限と書体検査を適用する() {
    let fonts = book();
    let page = Page {
        width: 100.0,
        height: 100.0,
        items: vec![Item::TextLine(vec![
            line_text("A", 30.0),
            line_text("B", 40.0),
        ])],
    };
    for (limits, reason) in [
        (
            Limits {
                items: 1,
                ..Default::default()
            },
            "pdf_item_limit",
        ),
        (
            Limits {
                text_bytes: 1,
                ..Default::default()
            },
            "pdf_text_limit",
        ),
        (
            Limits {
                glyphs: 1,
                ..Default::default()
            },
            "pdf_glyph_limit",
        ),
    ] {
        assert_eq!(
            pdf::render(
                std::slice::from_ref(&page),
                &fonts,
                &limits,
                &FontLimits::default()
            )
            .unwrap_err()
            .reason,
            reason
        );
    }
    let mut bad = line_text("A", 30.0);
    bad.font = 1;
    let page = Page {
        width: 100.0,
        height: 100.0,
        items: vec![Item::TextLine(vec![bad])],
    };
    assert_eq!(
        pdf::render(&[page], &fonts, &Limits::default(), &FontLimits::default())
            .unwrap_err()
            .reason,
        "invalid_pdf_font_index"
    );
}

#[test]
fn 空の文字行と行内の異なる基線やクリップを拒否する() {
    let fonts = book();
    let mut shifted = line_text("B", 40.0);
    shifted.baseline = 50.0;
    let mut clipped = line_text("B", 40.0);
    clipped.clip = Some(pdf::graphics::BoxRect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    });
    for (texts, reason) in [
        (vec![], "empty_pdf_text_line"),
        (
            vec![line_text("A", 30.0), shifted],
            "inconsistent_pdf_text_line",
        ),
        (
            vec![line_text("A", 30.0), clipped],
            "inconsistent_pdf_text_line",
        ),
    ] {
        let page = Page {
            width: 100.0,
            height: 100.0,
            items: vec![Item::TextLine(texts)],
        };
        assert_eq!(
            pdf::render(&[page], &fonts, &Limits::default(), &FontLimits::default())
                .unwrap_err()
                .reason,
            reason
        );
    }
}

#[test]
fn 複数書式の一行で字形並べ替えと抽出改行抑制を両立する() {
    let fonts = combining_book();
    let text = "x\u{301}\u{327}";
    let mut second = line_text(text, 60.0);
    second.points = 16.0;
    second.color = [255, 0, 0];
    let page = Page {
        width: 200.0,
        height: 100.0,
        items: vec![Item::TextLine(vec![line_text(text, 30.0), second])],
    };
    let render = || {
        pdf::render(
            std::slice::from_ref(&page),
            &fonts,
            &Limits::default(),
            &FontLimits::default(),
        )
        .unwrap()
    };
    let bytes = render();
    assert_eq!(bytes, render());
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap(), format!("{text}{text}\n"));
    let content =
        lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
            .unwrap();
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|op| op.operator == "BT")
            .count(),
        1
    );
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|op| op.operator == "ET")
            .count(),
        1
    );
    let matrices: Vec<_> = content
        .operations
        .iter()
        .filter(|op| op.operator == "Tm")
        .collect();
    assert_eq!(matrices.len(), 6);
    for (part, (points, origin)) in [(12.0, 30.0), (16.0, 60.0)].into_iter().enumerate() {
        let run = fonts.faces[0]
            .shape_run(text, points, &FontLimits::default())
            .unwrap();
        let mut x = origin;
        let mut y = 60.0;
        let mut positions = Vec::new();
        for glyph in run.glyphs {
            positions.push(((x + glyph.offset_x) as f32, (y + glyph.offset_y) as f32));
            x += glyph.advance_x;
            y += glyph.advance_y;
        }
        for (matrix, index) in matrices[part * 3..part * 3 + 3].iter().zip([0, 2, 1]) {
            assert_eq!(matrix.operands[4].as_float().unwrap(), positions[index].0);
            assert_eq!(matrix.operands[5].as_float().unwrap(), positions[index].1);
        }
    }
}

#[test]
fn 異体字と基底文字を別のunicode対応で抽出し選択字形を埋め込む() {
    for selector in [0xfe0f, 0xe0100] {
        for default in [false, true] {
            let mut fonts = FontBook::default();
            fonts
                .load_bytes(
                    support::font_with_variation(selector, default, 34),
                    &FontLimits::default(),
                )
                .unwrap();
            let text = format!("見見{}見", char::from_u32(selector).unwrap());
            let page = Page {
                width: 200.0,
                height: 100.0,
                items: vec![Item::Text(Text {
                    text: &text,
                    font: 0,
                    points: 12.0,
                    x: 30.0,
                    baseline: 40.0,
                    color: [0, 0, 0],
                    clip: None,
                })],
            };
            let render = || {
                pdf::render(
                    std::slice::from_ref(&page),
                    &fonts,
                    &Limits::default(),
                    &FontLimits::default(),
                )
                .unwrap()
            };
            let bytes = render();
            assert_eq!(bytes, render());
            let doc = lopdf::Document::load_mem(&bytes).unwrap();
            assert_eq!(doc.extract_text(&[1]).unwrap().trim(), text);
            let content = lopdf::content::Content::decode(
                &doc.get_page_content(doc.get_pages()[&1]).unwrap(),
            )
            .unwrap();
            let shows: Vec<_> = content
                .operations
                .iter()
                .filter(|op| op.operator == "Tj")
                .collect();
            assert_eq!(shows.len(), 3);
            assert_eq!(shows[0].operands, shows[2].operands);
            assert_ne!(shows[0].operands, shows[1].operands);
            let mapping = doc
                .objects
                .values()
                .find_map(|object| {
                    object
                        .as_dict()
                        .ok()?
                        .get(b"CIDToGIDMap")
                        .ok()?
                        .as_reference()
                        .ok()
                })
                .unwrap();
            let bytes = doc
                .get_object(mapping)
                .unwrap()
                .as_stream()
                .unwrap()
                .get_plain_content()
                .unwrap();
            let ids: std::collections::BTreeSet<_> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .filter(|id| *id != 0)
                .collect();
            assert_eq!(ids.len(), if default { 1 } else { 2 });
            let limits = Limits {
                text_bytes: text.len() - 1,
                ..Default::default()
            };
            assert_eq!(
                pdf::render(&[page], &fonts, &limits, &FontLimits::default())
                    .unwrap_err()
                    .reason,
                "pdf_text_limit"
            );
        }
    }
}

#[test]
fn 異なる書体の一行で既定と非既定の異体字を欠落なく抽出する() {
    let mut fonts = FontBook::default();
    for default in [true, false] {
        fonts
            .load_bytes(
                support::font_with_variation(0xe0100, default, 34),
                &FontLimits::default(),
            )
            .unwrap();
    }
    let text = "見\u{e0100}見";
    let first = line_text(text, 30.0);
    let mut second = line_text(text, 60.0);
    second.font = 1;
    second.points = 16.0;
    second.color = [255, 0, 0];
    let page = Page {
        width: 200.0,
        height: 100.0,
        items: vec![Item::TextLine(vec![first, second])],
    };
    let render = || {
        pdf::render(
            std::slice::from_ref(&page),
            &fonts,
            &Limits::default(),
            &FontLimits::default(),
        )
        .unwrap()
    };
    let bytes = render();
    assert_eq!(bytes, render());
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap(), format!("{text}{text}\n"));
    let content =
        lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
            .unwrap();
    for (operator, count) in [("BT", 1), ("ET", 1), ("Tj", 4), ("Tm", 4)] {
        assert_eq!(
            content
                .operations
                .iter()
                .filter(|op| op.operator == operator)
                .count(),
            count
        );
    }
    let mappings: Vec<_> = doc
        .objects
        .values()
        .filter_map(|object| {
            object
                .as_dict()
                .ok()?
                .get(b"CIDToGIDMap")
                .ok()?
                .as_reference()
                .ok()
        })
        .collect();
    assert_eq!(mappings.len(), 2);
    let mut glyph_counts: Vec<_> = mappings
        .into_iter()
        .map(|id| {
            let data = doc
                .get_object(id)
                .unwrap()
                .as_stream()
                .unwrap()
                .get_plain_content()
                .unwrap();
            data.chunks_exact(2)
                .map(|p| u16::from_be_bytes([p[0], p[1]]))
                .filter(|id| *id != 0)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        })
        .collect();
    glyph_counts.sort();
    assert_eq!(glyph_counts, [1, 2]);
    let limits = Limits {
        text_bytes: 2 * text.len() - 1,
        ..Default::default()
    };
    assert_eq!(
        pdf::render(&[page], &fonts, &limits, &FontLimits::default())
            .unwrap_err()
            .reason,
        "pdf_text_limit"
    );
}

#[test]
fn 双方向文字の視覚位置と論理順のpdf抽出を両立する() {
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts.load_bytes(support::bidi_font(), &limits).unwrap();
    for (text, order) in [
        ("Aאב12B", vec![0, 5, 6, 3, 1, 7]),
        ("אב12", vec![4, 5, 2, 0]),
        ("(אב)", vec![5, 3, 1, 0]),
    ] {
        let run = fonts.faces[0].shape_run(text, 12.0, &limits).unwrap();
        assert_eq!(
            run.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(),
            order,
            "{text}"
        );
        let page = Page {
            width: 300.0,
            height: 100.0,
            items: vec![Item::Text(line_text(text, 30.0))],
        };
        let bytes = pdf::render(
            std::slice::from_ref(&page),
            &fonts,
            &Limits::default(),
            &limits,
        )
        .unwrap();
        assert_eq!(
            bytes,
            pdf::render(&[page], &fonts, &Limits::default(), &limits).unwrap()
        );
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.extract_text(&[1]).unwrap().trim(), text);
        let content =
            lopdf::content::Content::decode(&doc.get_page_content(doc.get_pages()[&1]).unwrap())
                .unwrap();
        let marked: Vec<_> = content
            .operations
            .iter()
            .filter(|op| op.operator == "BDC")
            .collect();
        assert_eq!(marked.len(), 1);
        let actual = marked[0].operands[1]
            .as_dict()
            .unwrap()
            .get(b"ActualText")
            .unwrap()
            .as_str()
            .unwrap();
        let expected: Vec<_> = std::iter::once(0xfeffu16)
            .chain(text.encode_utf16())
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(
            content
                .operations
                .iter()
                .filter(|op| op.operator == "EMC")
                .count(),
            1
        );
        let matrices: Vec<_> = content
            .operations
            .iter()
            .filter(|op| op.operator == "Tm")
            .collect();
        let mut positions = Vec::new();
        let mut x = 30.0;
        for glyph in &run.glyphs {
            positions.push((glyph.cluster, (x + glyph.offset_x) as f32));
            x += glyph.advance_x;
        }
        positions.sort_by_key(|item| item.0);
        assert_eq!(matrices.len(), positions.len());
        for (matrix, (_, expected)) in matrices.iter().zip(positions) {
            assert_eq!(matrix.operands[4].as_float().unwrap(), expected);
        }
    }
}
