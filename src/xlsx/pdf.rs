//! 配置済み文字の PDF 出力。セル配置と CLI への接続は別層で行う。
use crate::xlsx::{
    diagnostic::{Failure, Result},
    fonts::{FontBook, FontLimits},
};
use pdf_writer::{
    Content, Finish, Name, Pdf, Rect, Ref, Str, TextStr,
    types::{CidFontType, FontFlags, SystemInfo},
};
use rustybuzz::ttf_parser::{Face, GlyphId, Tag};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

pub struct Text<'a> {
    pub text: &'a str,
    pub font: usize,
    pub points: f64,
    pub x: f64,
    /// 上端からベースラインまでの距離。
    pub baseline: f64,
    pub color: [u8; 3],
    /// 隣接セルの占有状況から配置層が決める可視範囲。
    pub clip: Option<graphics::BoxRect>,
    /// Some なら各字形の送り幅をこの単位の整数倍へ丸めて並べる（[`crate::xlsx::cell_text::LineBox`]）。
    pub advance_unit: Option<f64>,
}
pub mod graphics;
use graphics::Paint;
mod text_mapping;

pub enum Item<'a> {
    Text(Text<'a>),
    /// 同じセルの同じ可視行。書式境界を PDF の改行境界にしない。
    TextLine(Vec<Text<'a>>),
    Paint(Paint<'a>),
}
pub struct Page<'a> {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item<'a>>,
}
impl<'a> Page<'a> {
    pub(crate) fn texts(&self) -> impl Iterator<Item = &Text<'a>> {
        self.items.iter().flat_map(|item| match item {
            Item::Text(text) => std::slice::from_ref(text),
            Item::TextLine(texts) => texts.as_slice(),
            Item::Paint(_) => &[],
        })
    }
}
pub struct Limits {
    pub pages: usize,
    pub glyphs: usize,
    pub items: usize,
    pub text_bytes: usize,
    pub image_bytes: usize,
    pub output_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            pages: 10_000,
            glyphs: 1_000_000,
            items: 1_000_000,
            text_bytes: 32 * 1024 * 1024,
            image_bytes: 64 * 1024 * 1024,
            output_bytes: 128 * 1024 * 1024,
        }
    }
}
struct Prepared {
    keys: BTreeMap<(u16, String), u16>,
}
fn info() -> SystemInfo<'static> {
    SystemInfo {
        registry: Str(b"Adobe"),
        ordering: Str(b"Identity"),
        supplement: 0,
    }
}
fn number(value: f64) -> Result<f32> {
    if !value.is_finite() || value.abs() > 1_000_000.0 {
        return Err(Failure::input("invalid_pdf_coordinate"));
    }
    Ok(value as f32)
}

/// ファイル操作をせず、検査済みの PDF バイト列を返す。
pub fn render(
    pages: &[Page<'_>],
    fonts: &FontBook,
    limits: &Limits,
    font_limits: &FontLimits,
) -> Result<Vec<u8>> {
    if pages.is_empty() || pages.len() > limits.pages || pages.len() > 10_000 {
        return Err(Failure::input("pdf_page_limit"));
    }
    let mut prepared = BTreeMap::<usize, Prepared>::new();
    let mut runs = Vec::new();
    let mut count = 0usize;
    let mut item_count = 0usize;
    let mut text_bytes = 0usize;
    let mut image_bytes = 0usize;
    let mut seen_images = BTreeSet::new();
    for page in pages {
        if page.width <= 0.0 || page.height <= 0.0 {
            return Err(Failure::input("invalid_pdf_page_size"));
        }
        number(page.width)?;
        number(page.height)?;
        for item in &page.items {
            let size = match item {
                Item::TextLine(texts) => texts.len(),
                _ => 1,
            };
            item_count = item_count
                .checked_add(size)
                .filter(|n| *n <= limits.items)
                .ok_or_else(|| Failure::input("pdf_item_limit"))?;
            if let Item::TextLine(texts) = item {
                if texts.len() > 1
                    && texts
                        .iter()
                        .any(|text| crate::xlsx::text_direction::has_rtl(text.text))
                {
                    return Err(Failure::unsupported("rich_text_bidi_layout"));
                }
                let first = texts
                    .first()
                    .ok_or_else(|| Failure::input("empty_pdf_text_line"))?;
                if texts
                    .iter()
                    .any(|text| text.baseline != first.baseline || text.clip != first.clip)
                {
                    return Err(Failure::input("inconsistent_pdf_text_line"));
                }
            }
            if let Item::Paint(paint) = item {
                paint.validate(&mut image_bytes, &mut seen_images, limits)?;
            }
        }
        let mut page_runs = Vec::new();
        for text in page.texts() {
            text_bytes = text_bytes
                .checked_add(text.text.len())
                .filter(|count| *count <= limits.text_bytes)
                .ok_or_else(|| Failure::input("pdf_text_limit"))?;
            number(text.x)?;
            number(text.baseline)?;
            if let Some(clip) = text.clip {
                clip.validate()?;
            }
            let font = fonts
                .faces
                .get(text.font)
                .ok_or_else(|| Failure::input("invalid_pdf_font_index"))?;
            let mut run = font.shape_run(text.text, text.points, font_limits)?;
            if let Some(unit) = text.advance_unit {
                run.snap_advances(unit)?;
            }
            count = count
                .checked_add(run.glyphs.len())
                .filter(|n| *n <= limits.glyphs)
                .ok_or_else(|| Failure::input("pdf_glyph_limit"))?;
            let entry = prepared.entry(text.font).or_insert_with(|| Prepared {
                keys: BTreeMap::new(),
            });
            let mut keys = Vec::new();
            let face = Face::parse(font.data(), font.collection_index)
                .map_err(|_| Failure::input("invalid_font_data"))?;
            let (values, cluster_actual_text) =
                text_mapping::mapped_values(text.text, &run.glyphs, &face)?;
            let mut order = Vec::with_capacity(values.len());
            for (glyph, (offset, value)) in run.glyphs.iter().zip(values) {
                order.push(offset);
                let gid =
                    u16::try_from(glyph.id).map_err(|_| Failure::input("invalid_pdf_glyph"))?;
                let key = (gid, value.to_owned());
                entry.keys.insert(key.clone(), 0);
                keys.push(key);
            }
            page_runs.push((run, keys, order, cluster_actual_text));
        }
        runs.push(page_runs);
    }
    let mut pdf = Pdf::new();
    pdf.set_file_id((vec![0; 16], vec![0; 16]));
    let root = Ref::new(1);
    let tree = Ref::new(2);
    pdf.catalog(root).pages(tree);
    let mut next = 3i32;
    let mut font_refs = BTreeMap::new();
    for (font_index, entry) in &mut prepared {
        if entry.keys.len() > 65535 {
            return Err(Failure::input("pdf_cid_limit"));
        }
        let font = &fonts.faces[*font_index];
        let face = Face::parse(font.data(), font.collection_index)
            .map_err(|_| Failure::input("invalid_font_data"))?;
        let cff = face.raw_face().table(Tag::from_bytes(b"CFF ")).is_some();
        if !cff && face.raw_face().table(Tag::from_bytes(b"glyf")).is_none() {
            return Err(Failure::unsupported("pdf_font_outline"));
        }
        let mut remapper = subsetter::GlyphRemapper::new();
        for gid in entry.keys.keys().map(|k| k.0).collect::<BTreeSet<_>>() {
            remapper.remap(gid);
        }
        let subset = subsetter::subset(font.data(), font.collection_index, &remapper)
            .map_err(|_| Failure::unsupported("font_subset_failed"))?;
        if subset.len() > limits.output_bytes {
            return Err(Failure::input("pdf_output_limit"));
        }
        let ids: Vec<_> = (next..next + 6).map(Ref::new).collect();
        next += 6;
        font_refs.insert(*font_index, ids[0]);
        let name = format!("AAAAAA+Font{font_index}");
        let factor = 1000.0 / f32::from(face.units_per_em());
        let mut mapping = vec![0, 0];
        let mut widths = Vec::new();
        for (i, ((gid, _text), cid)) in entry.keys.iter_mut().enumerate() {
            *cid = (i + 1) as u16;
            mapping.extend_from_slice(
                &remapper
                    .get(*gid)
                    .ok_or_else(|| Failure::input("invalid_pdf_glyph"))?
                    .to_be_bytes(),
            );
            widths.push(
                f32::from(
                    face.glyph_hor_advance(GlyphId(*gid))
                        .ok_or_else(|| Failure::input("font_metrics_missing"))?,
                ) * factor,
            );
        }
        let mut type0 = pdf.type0_font(ids[0]);
        type0
            .base_font(Name(name.as_bytes()))
            .descendant_font(ids[1])
            .to_unicode(ids[4]);
        if cff {
            type0.encoding_cmap(ids[5]);
        } else {
            type0.encoding_predefined(Name(b"Identity-H"));
        }
        type0.finish();
        let mut cid = pdf.cid_font(ids[1]);
        cid.subtype(if cff {
            CidFontType::Type0
        } else {
            CidFontType::Type2
        })
        .base_font(Name(name.as_bytes()))
        .system_info(info())
        .font_descriptor(ids[2]);
        if cff {
            let mut unique = BTreeMap::new();
            for (gid, width) in mapping.chunks_exact(2).skip(1).zip(widths) {
                unique.insert(u16::from_be_bytes([gid[0], gid[1]]), width);
            }
            let mut writer = cid.widths();
            for (gid, width) in unique {
                writer.same(gid, gid, width);
            }
            writer.finish();
        } else {
            cid.cid_to_gid_map_stream(ids[5]);
            cid.widths().consecutive(1, widths);
        }
        cid.finish();
        let bbox = face.global_bounding_box();
        let mut descriptor = pdf.font_descriptor(ids[2]);
        descriptor
            .name(Name(name.as_bytes()))
            .flags(FontFlags::SYMBOLIC)
            .bbox(Rect::new(
                f32::from(bbox.x_min) * factor,
                f32::from(bbox.y_min) * factor,
                f32::from(bbox.x_max) * factor,
                f32::from(bbox.y_max) * factor,
            ))
            .italic_angle(face.italic_angle())
            .ascent(f32::from(face.ascender()) * factor)
            .descent(f32::from(face.descender()) * factor)
            .cap_height(f32::from(face.capital_height().unwrap_or(face.ascender())) * factor)
            .stem_v(80.0);
        if cff {
            descriptor.font_file3(ids[3]);
        } else {
            descriptor.font_file2(ids[3]);
        }
        descriptor.finish();
        if cff {
            let raw = rustybuzz::ttf_parser::RawFace::parse(&subset, 0)
                .map_err(|_| Failure::input("invalid_font_subset"))?;
            let bytes = raw
                .table(Tag::from_bytes(b"CFF "))
                .ok_or_else(|| Failure::input("invalid_font_subset"))?;
            pdf.stream(ids[3], bytes)
                .pair(Name(b"Subtype"), Name(b"CIDFontType0C"));
        } else {
            pdf.stream(ids[3], &subset)
                .pair(Name(b"Length1"), subset.len() as i32);
        }
        pdf.stream(ids[4], &unicode_map(&entry.keys));
        if cff {
            pdf.cmap(ids[5], &encoding_map(&mapping))
                .name(Name(b"GlyphEncoding"))
                .system_info(info());
        } else {
            pdf.stream(ids[5], &mapping);
        }
        if pdf.len() > limits.output_bytes {
            return Err(Failure::input("pdf_output_limit"));
        }
    }
    let mut kids = Vec::new();
    let mut image_cache = BTreeMap::new();
    for (page, page_runs) in pages.iter().zip(runs) {
        let id = Ref::new(next);
        let stream = Ref::new(next + 1);
        next += 2;
        kids.push(id);
        let mut content = Content::new();
        let mut page_runs = page_runs.into_iter();
        let mut images = Vec::new();
        for item in &page.items {
            let texts = match item {
                Item::Text(text) => std::slice::from_ref(text),
                Item::TextLine(texts) => texts.as_slice(),
                Item::Paint(paint) => {
                    paint.draw(
                        &mut content,
                        &mut pdf,
                        &mut next,
                        &mut images,
                        page.height,
                        &mut image_cache,
                    )?;
                    if pdf.len() > limits.output_bytes {
                        return Err(Failure::input("pdf_output_limit"));
                    }
                    continue;
                }
            };
            content.save_state();
            if let Some(clip) = texts[0].clip {
                content
                    .rect(
                        number(clip.x)?,
                        number(page.height - clip.y - clip.height)?,
                        number(clip.width)?,
                        number(clip.height)?,
                    )
                    .clip_nonzero()
                    .end_path();
            }
            content.begin_text();
            for text in texts {
                let (run, keys, order, cluster_actual_text) = page_runs
                    .next()
                    .ok_or_else(|| Failure::input("pdf_run_missing"))?;
                let actual_text =
                    cluster_actual_text || crate::xlsx::text_direction::has_rtl(text.text);
                if actual_text {
                    content
                        .begin_marked_content_with_properties(Name(b"Span"))
                        .properties()
                        .actual_text(TextStr(text.text));
                }
                let resource = format!("F{}", text.font);
                content.set_fill_rgb(
                    f32::from(text.color[0]) / 255.0,
                    f32::from(text.color[1]) / 255.0,
                    f32::from(text.color[2]) / 255.0,
                );
                content.set_font(Name(resource.as_bytes()), number(text.points)?);
                let mut x = text.x;
                let mut y = page.height - text.baseline;
                // 字形位置は視覚順で確定し、PDF 内の文字列だけを元の順へ戻す。
                let mut positioned = Vec::with_capacity(keys.len());
                for ((glyph, key), offset) in run.glyphs.iter().zip(keys).zip(order) {
                    positioned.push((
                        offset,
                        key,
                        number(x + glyph.offset_x)?,
                        number(y + glyph.offset_y)?,
                    ));
                    x += glyph.advance_x;
                    y += glyph.advance_y;
                }
                positioned.sort_by_key(|item| item.0);
                for (_, key, x, y) in positioned {
                    content.set_text_matrix([1.0, 0.0, 0.0, 1.0, x, y]);
                    content.show(Str(&prepared[&text.font].keys[&key].to_be_bytes()));
                }
                if actual_text {
                    content.end_marked_content();
                }
            }
            content.end_text();
            content.restore_state();
        }
        let bytes = content.finish();
        if bytes.len() > limits.output_bytes {
            return Err(Failure::input("pdf_output_limit"));
        }
        let mut out = pdf.page(id);
        out.parent(tree)
            .media_box(Rect::new(
                0.0,
                0.0,
                number(page.width)?,
                number(page.height)?,
            ))
            .contents(stream);
        let mut resources = out.resources();
        let mut fs = resources.fonts();
        for index in page.texts().map(|t| t.font).collect::<BTreeSet<_>>() {
            let name = format!("F{index}");
            fs.pair(Name(name.as_bytes()), font_refs[&index]);
        }
        fs.finish();
        let mut xs = resources.x_objects();
        for (name, id) in images {
            xs.pair(Name(name.as_bytes()), id);
        }
        xs.finish();
        resources.finish();
        out.finish();
        pdf.stream(stream, &bytes);
        if pdf.len() > limits.output_bytes {
            return Err(Failure::input("pdf_output_limit"));
        }
    }
    pdf.pages(tree).kids(kids).count(pages.len() as i32);
    let bytes = pdf.finish();
    if bytes.len() > limits.output_bytes {
        return Err(Failure::input("pdf_output_limit"));
    }
    Ok(bytes)
}

// ToUnicode は CMapType 2、宛先は BOM を持たない UTF-16BE。
fn unicode_map(keys: &BTreeMap<(u16, String), u16>) -> Vec<u8> {
    let mut out = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    // 空の対応は不正な空文字列のCMapにせず、ActualText側で意味を表す。
    let entries: Vec<_> = keys
        .iter()
        .filter(|((_, text), _)| !text.is_empty())
        .collect();
    for chunk in entries.chunks(100) {
        writeln!(out, "{} beginbfchar", chunk.len()).unwrap();
        for ((_, text), cid) in chunk {
            write!(out, "<{cid:04X}> <").unwrap();
            for unit in text.encode_utf16() {
                write!(out, "{unit:04X}").unwrap();
            }
            out.push_str(">\n");
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out.into_bytes()
}

// 文字コードと CFF の CID を分離する。同一字形に異なる元文字列を割り当てられる。
fn encoding_map(mapping: &[u8]) -> Vec<u8> {
    let mut out = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n/CMapName /GlyphEncoding def\n/CMapType 1 def\n/WMode 0 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for (block, chunk) in mapping.chunks(200).enumerate() {
        writeln!(out, "{} begincidchar", chunk.len() / 2).unwrap();
        for (i, bytes) in chunk.chunks_exact(2).enumerate() {
            writeln!(
                out,
                "<{:04X}> {}",
                block * 100 + i,
                u16::from_be_bytes([bytes[0], bytes[1]])
            )
            .unwrap();
        }
        out.push_str("endcidchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out.into_bytes()
}
