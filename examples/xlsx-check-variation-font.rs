//! 開発専用。明示フォントの異体字を描画し、埋め込み輪郭と抽出を検証する。
use rhodopsin::xlsx::{
    fonts::{FontBook, FontLimits},
    pdf::{self, Item, Page, Text},
};
use rustybuzz::ttf_parser::{Face, GlyphId, OutlineBuilder};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
type Result<T> = std::result::Result<T, &'static str>;

#[derive(Default)]
struct Outline {
    hash: Sha256,
    commands: usize,
}
impl Outline {
    fn add(&mut self, command: u8, values: &[f32]) {
        self.commands += 1;
        self.hash.update([command]);
        for value in values {
            // 符号付きゼロは同じ座標。
            let value = if *value == 0.0 { 0.0 } else { *value };
            self.hash.update(value.to_bits().to_be_bytes());
        }
    }
    fn digest(self) -> Result<String> {
        if self.commands == 0 {
            return Err("glyph_outline_missing");
        }
        Ok(format!("{:x}", self.hash.finalize()))
    }
}
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(0, &[x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(1, &[x, y]);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.add(2, &[x1, y1, x, y]);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.add(3, &[x1, y1, x2, y2, x, y]);
    }
    fn close(&mut self) {
        self.add(4, &[]);
    }
}
fn font_reference(doc: &lopdf::Document, key: &[u8]) -> Result<lopdf::ObjectId> {
    doc.objects
        .values()
        .find_map(|object| object.as_dict().ok()?.get(key).ok()?.as_reference().ok())
        .ok_or("pdf_font_reference_missing")
}
fn stream(doc: &lopdf::Document, id: lopdf::ObjectId) -> Result<Vec<u8>> {
    doc.get_object(id)
        .map_err(|_| "pdf_stream_missing")?
        .as_stream()
        .map_err(|_| "pdf_stream_missing")?
        .get_plain_content()
        .map_err(|_| "pdf_stream_invalid")
}
fn run() -> Result<serde_json::Value> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 5 {
        return Err("expected_font_face_base_selector_output");
    }
    let face_index: usize = args[1]
        .to_str()
        .ok_or("invalid_face_index")?
        .parse()
        .map_err(|_| "invalid_face_index")?;
    let codepoint = |index: usize| -> Result<char> {
        char::from_u32(
            u32::from_str_radix(args[index].to_str().ok_or("invalid_codepoint")?, 16)
                .map_err(|_| "invalid_codepoint")?,
        )
        .ok_or("invalid_codepoint")
    };
    let base = codepoint(2)?;
    let selector = codepoint(3)?;
    if !matches!(selector,'\u{fe00}'..='\u{fe0f}'|'\u{e0100}'..='\u{e01ef}') {
        return Err("invalid_selector");
    }
    let output = Path::new(&args[4]);
    rhodopsin::xlsx::output::check_available(output).map_err(|e| e.reason)?;
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_file(Path::new(&args[0]), &limits)
        .map_err(|e| e.reason)?;
    let selected = fonts.faces.get(face_index).ok_or("invalid_face_index")?;
    selected.check_embedding().map_err(|e| e.reason)?;
    let face =
        Face::parse(selected.data(), selected.collection_index).map_err(|_| "invalid_font_data")?;
    let base_gid = face.glyph_index(base).ok_or("font_glyph_missing")?;
    let variation_gid = face
        .glyph_variation_index(base, selector)
        .ok_or("font_variation_sequence_missing")?;
    let text = format!("{base}{base}{selector}{base}");
    let shaped = selected
        .shape_run(&text, 24.0, &limits)
        .map_err(|e| e.reason)?;
    let expected = [base_gid, variation_gid, base_gid];
    if shaped.glyphs.len() != 3
        || shaped
            .glyphs
            .iter()
            .zip(expected)
            .any(|(a, b)| a.id != u32::from(b.0))
    {
        return Err("unexpected_variation_shaping");
    }
    let page = Page {
        width: 300.0,
        height: 100.0,
        items: vec![Item::Text(Text {
            text: &text,
            font: face_index,
            points: 24.0,
            x: 30.0,
            baseline: 50.0,
            color: [0, 0, 0],
            clip: None,
        })],
    };
    let render = || {
        pdf::render(
            std::slice::from_ref(&page),
            &fonts,
            &Default::default(),
            &limits,
        )
        .map_err(|e| e.reason)
    };
    let bytes = render()?;
    if bytes != render()? {
        return Err("nondeterministic_pdf");
    }
    let doc = lopdf::Document::load_mem(&bytes).map_err(|_| "pdf_parse_failed")?;
    if doc
        .extract_text(&[1])
        .map_err(|_| "pdf_text_failed")?
        .trim()
        != text
    {
        return Err("pdf_text_mismatch");
    }
    let page_id = *doc.get_pages().get(&1).ok_or("pdf_page_missing")?;
    let content = lopdf::content::Content::decode(
        &doc.get_page_content(page_id)
            .map_err(|_| "pdf_content_failed")?,
    )
    .map_err(|_| "pdf_content_failed")?;
    let mut codes = Vec::new();
    for op in &content.operations {
        if op.operator != "Tj" {
            continue;
        }
        let data = op
            .operands
            .first()
            .ok_or("invalid_pdf_text")?
            .as_str()
            .map_err(|_| "invalid_pdf_text")?;
        if data.len() != 2 {
            return Err("invalid_pdf_text");
        }
        codes.push(u16::from_be_bytes([data[0], data[1]]));
    }
    if codes.len() != 3 {
        return Err("invalid_pdf_text");
    }
    let mut embedded = Vec::new();
    if let Ok(id) = font_reference(&doc, b"FontFile2") {
        let font = stream(&doc, id)?;
        let subset = Face::parse(&font, 0).map_err(|_| "invalid_font_subset")?;
        let map = stream(&doc, font_reference(&doc, b"CIDToGIDMap")?)?;
        for code in &codes {
            let index = usize::from(*code) * 2;
            let pair = map.get(index..index + 2).ok_or("invalid_pdf_mapping")?;
            let mut outline = Outline::default();
            subset
                .outline_glyph(
                    GlyphId(u16::from_be_bytes([pair[0], pair[1]])),
                    &mut outline,
                )
                .ok_or("glyph_outline_missing")?;
            embedded.push(outline.digest()?);
        }
    } else {
        let font = stream(&doc, font_reference(&doc, b"FontFile3")?)?;
        let subset =
            rustybuzz::ttf_parser::cff::Table::parse(&font).ok_or("invalid_font_subset")?;
        let data = stream(&doc, font_reference(&doc, b"Encoding")?)?;
        let mut map = BTreeMap::new();
        for line in std::str::from_utf8(&data)
            .map_err(|_| "invalid_pdf_mapping")?
            .lines()
        {
            let parts: Vec<_> = line.split_whitespace().collect();
            if let [code, cid] = parts.as_slice()
                && let Some(hex) = code.strip_prefix('<').and_then(|s| s.strip_suffix('>'))
                && let (Ok(code), Ok(cid)) = (u16::from_str_radix(hex, 16), cid.parse::<u16>())
            {
                map.insert(code, cid);
            }
        }
        for code in &codes {
            let cid = *map.get(code).ok_or("invalid_pdf_mapping")?;
            let glyph = (0..subset.number_of_glyphs())
                .map(GlyphId)
                .find(|id| subset.glyph_cid(*id) == Some(cid))
                .ok_or("invalid_pdf_mapping")?;
            let mut outline = Outline::default();
            subset
                .outline(glyph, &mut outline)
                .map_err(|_| "glyph_outline_missing")?;
            embedded.push(outline.digest()?);
        }
    }
    let mut original = Vec::new();
    for glyph in expected {
        let mut outline = Outline::default();
        face.outline_glyph(glyph, &mut outline)
            .ok_or("glyph_outline_missing")?;
        original.push(outline.digest()?);
    }
    if original != embedded {
        return Err("subset_outline_mismatch");
    }
    let version = face
        .names()
        .into_iter()
        .filter(|name| name.name_id == rustybuzz::ttf_parser::name_id::VERSION)
        .find_map(|name| name.to_string());
    rhodopsin::xlsx::output::publish(output, &bytes).map_err(|e| e.reason)?;
    Ok(
        serde_json::json!({"schema_version":1,"status":"font_probe_verified",
        "font_sha256":format!("{:x}",Sha256::digest(selected.data())),
        "font_families":selected.families,"font_version":version,"collection_index":selected.collection_index,
        "base":format!("U+{:04X}",u32::from(base)),"selector":format!("U+{:04X}",u32::from(selector)),
        "source_glyphs":expected.map(|g|g.0),"outline_sha256":original,
        "pdf_sha256":format!("{:x}",Sha256::digest(&bytes)),
        "pdf_bytes":bytes.len(),"extraction_exact":true,"deterministic":true,
        "embedded_outline_exact":true,"excel_reference":"unobserved"}),
    )
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(record) => {
            println!("{record}");
            0.into()
        }
        Err(reason) => {
            eprintln!("{}", serde_json::json!({"status":"failed","reason":reason}));
            2.into()
        }
    }
}
