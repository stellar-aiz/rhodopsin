//! 明示した実フォントで非機密の双方向文字列を描画し、抽出と決定性を検査する。
use rhodopsin::xlsx::{
    fonts::{FontBook, FontLimits},
    pdf::{self, Item, Page, Text},
};
use sha2::{Digest, Sha256};
use std::path::Path;

fn run() -> Result<serde_json::Value, &'static str> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err("expected_font_face_script_output");
    }
    let index: usize = args[1]
        .to_str()
        .and_then(|s| s.parse().ok())
        .ok_or("invalid_face_index")?;
    let text = match args[2].to_str() {
        Some("hebrew") => "AB אבג 123 (דהו)",
        Some("arabic") => "AB سلام 123 (مرحبا)",
        _ => return Err("invalid_script"),
    };
    let output = Path::new(&args[3]);
    rhodopsin::xlsx::output::check_available(output).map_err(|e| e.reason)?;
    let limits = FontLimits::default();
    let mut fonts = FontBook::default();
    fonts
        .load_file(Path::new(&args[0]), &limits)
        .map_err(|e| e.reason)?;
    let font = fonts.faces.get(index).ok_or("invalid_face_index")?;
    let face = rustybuzz::ttf_parser::Face::parse(font.data(), font.collection_index)
        .map_err(|_| "invalid_font")?;
    let run = font.shape_run(text, 20.0, &limits).map_err(|e| e.reason)?;
    // スペース以外は、使用字形の輪郭が実在することを確認する。
    for glyph in &run.glyphs {
        if !text[glyph.cluster..].starts_with(' ')
            && face
                .glyph_bounding_box(rustybuzz::ttf_parser::GlyphId(glyph.id as u16))
                .is_none()
        {
            return Err("glyph_outline_missing");
        }
    }
    let page = Page {
        width: 500.0,
        height: 100.0,
        items: vec![Item::Text(Text {
            text,
            font: index,
            points: 20.0,
            x: 30.0,
            baseline: 45.0,
            color: [0, 0, 0],
            clip: None,
            advance_unit: None,
        })],
    };
    let bytes = pdf::render(
        std::slice::from_ref(&page),
        &fonts,
        &Default::default(),
        &limits,
    )
    .map_err(|e| e.reason)?;
    let again = pdf::render(&[page], &fonts, &Default::default(), &limits).map_err(|e| e.reason)?;
    if bytes != again {
        return Err("pdf_not_deterministic");
    }
    let doc = lopdf::Document::load_mem(&bytes).map_err(|_| "invalid_pdf")?;
    if doc
        .extract_text(&[1])
        .map_err(|_| "pdf_extraction_failed")?
        .trim()
        != text
    {
        return Err("pdf_text_mismatch");
    }
    rhodopsin::xlsx::output::publish(output, &bytes).map_err(|e| e.reason)?;
    Ok(serde_json::json!({
        "font_sha256": format!("{:x}", Sha256::digest(font.data())),
        "pdf_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "collection_index": font.collection_index,
        "glyphs": run.glyphs.len(),
        "clusters": run.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>(),
        "nonspace_outlines": true, "extraction": true, "deterministic": true,
        "excel_comparison": "not_evaluated"
    }))
}
fn main() {
    match run() {
        Ok(result) => println!("{result}"),
        Err(reason) => {
            eprintln!("{}", serde_json::json!({"reason": reason}));
            std::process::exit(1);
        }
    }
}
