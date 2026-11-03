use crate::xlsx::diagnostic::{Category, Failure, Result};
use rustybuzz::ttf_parser::{self, Face, Permissions};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct FontLimits {
    pub file_bytes: u64,
    pub total_bytes: u64,
    pub faces: usize,
    pub run_bytes: usize,
}
impl Default for FontLimits {
    fn default() -> Self {
        Self {
            file_bytes: 64 * 1024 * 1024,
            total_bytes: 256 * 1024 * 1024,
            faces: 1024,
            run_bytes: 128 * 1024,
        }
    }
}
#[derive(Clone, Debug)]
pub struct FontFace {
    pub families: BTreeSet<String>,
    pub postscript_name: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub collection_index: u32,
    data: Arc<Vec<u8>>,
}
impl FontFace {
    /// OpenType post の下線上端と厚さをポイントへ換算する。
    /// 上端はベースラインから下向きを正とする。Excel の採用規則とは別の計測値。
    pub fn underline_metrics(&self, points: f64) -> Result<(f64, f64)> {
        validate_print_size(points)?;
        let face = self.face()?;
        let metrics = face
            .underline_metrics()
            .filter(|metrics| metrics.thickness > 0)
            .ok_or_else(|| Failure::unsupported("font_underline_metrics_missing"))?;
        let factor = points / f64::from(face.units_per_em());
        Ok((
            -f64::from(metrics.position) * factor,
            f64::from(metrics.thickness) * factor,
        ))
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    fn face(&self) -> Result<Face<'_>> {
        Face::parse(&self.data, self.collection_index)
            .map_err(|_| Failure::input("invalid_font_data"))
    }

    pub fn check_embedding(&self) -> Result<()> {
        let face = self.face()?;
        if !matches!(
            face.permissions(),
            Some(Permissions::Installable | Permissions::PreviewAndPrint | Permissions::Editable)
        ) || !face.is_subsetting_allowed()
            || !face.is_outline_embedding_allowed()
        {
            return Err(Failure::unsupported("font_embedding_restricted"));
        }
        if face.is_variable() {
            return Err(Failure::unsupported("variable_font"));
        }
        Ok(())
    }

    /// 列幅の換算に使う、標準書体の数字 0〜9 の最大送り幅（ポイント）。
    pub fn maximum_digit_width(&self, points: f64) -> Result<f64> {
        validate_size(points)?;
        let face = self.face()?;
        let mut maximum = 0;
        for ch in '0'..='9' {
            let glyph = face
                .glyph_index(ch)
                .filter(|id| id.0 != 0)
                .ok_or_else(|| missing("font_glyph_missing"))?;
            maximum = maximum.max(
                face.glyph_hor_advance(glyph)
                    .ok_or_else(|| missing("font_metrics_missing"))?,
            );
        }
        Ok(f64::from(maximum) * points / f64::from(face.units_per_em()))
    }

    /// 一段落・一行の文字を方向と文字体系で分割し、視覚順の字形を返す。
    /// 改行と書式境界を跨ぐ方向解決は呼び出し元で扱う。
    pub fn shape_run(&self, text: &str, points: f64, limits: &FontLimits) -> Result<ShapedRun> {
        validate_print_size(points)?;
        if text.len() > limits.run_bytes {
            return Err(Failure::input("text_run_limit"));
        }
        if text.chars().any(char::is_control) {
            return Err(Failure::unsupported("text_control_character"));
        }
        // 制御文字を抽出時に保持する経路は別途必要。黙って除去しない。
        if text.chars().any(|ch| {
            use unicode_bidi::BidiClass::*;
            matches!(ch, '\u{200e}' | '\u{200f}' | '\u{061c}')
                || matches!(
                    unicode_bidi::bidi_class(ch),
                    RLE | RLO | LRE | LRO | PDF | RLI | LRI | FSI | PDI
                )
        }) {
            return Err(Failure::unsupported("text_direction"));
        }
        self.check_embedding()?;
        let face = rustybuzz::Face::from_slice(&self.data, self.collection_index)
            .ok_or_else(|| Failure::input("invalid_font_data"))?;
        // cmap format 14 の指定を確認してから shaping へ渡す。
        // セレクタを通常文字へ落としたり、未対応の異体字を基底字形で代用しない。
        let selector = |ch: char| matches!(ch, '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}');
        let mut characters = text.chars().peekable();
        while let Some(base) = characters.next() {
            if selector(base) {
                return Err(Failure::unsupported("variation_selector_without_base"));
            }
            let glyph =
                if let Some(variation) = characters.peek().copied().filter(|ch| selector(*ch)) {
                    characters.next();
                    let glyph = face
                        .glyph_variation_index(base, variation)
                        .filter(|id| id.0 != 0)
                        .ok_or_else(|| missing("font_variation_sequence_missing"))?;
                    if face.is_color_glyph(glyph)
                        || face.glyph_svg_image(glyph).is_some()
                        || face.glyph_raster_image(glyph, u16::MAX).is_some()
                    {
                        return Err(Failure::unsupported("variation_glyph_image"));
                    }
                    glyph
                } else {
                    face.glyph_index(base)
                        .filter(|id| id.0 != 0)
                        .ok_or_else(|| missing("font_glyph_missing"))?
                };
            // 非既定異体字だけに対応するフォントでも、この字形自体が存在する必要がある。
            if glyph.0 >= face.number_of_glyphs() {
                return Err(missing("font_glyph_missing"));
            }
        }
        let factor = points / f64::from(face.units_per_em());
        let mut glyphs = Vec::new();
        for (range, direction, script) in crate::xlsx::text_direction::segments(text)? {
            if direction == rustybuzz::Direction::RightToLeft {
                for ch in text[range.clone()].chars() {
                    if let Some(mirrored) = unicode_bidi_mirroring::get_mirrored(ch) {
                        face.glyph_index(mirrored)
                            .filter(|id| id.0 != 0)
                            .ok_or_else(|| missing("font_mirrored_glyph_missing"))?;
                    }
                }
            }
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(&text[range.clone()]);
            buffer.set_direction(direction);
            buffer.set_script(script);
            let shaped = rustybuzz::shape(&face, &[], buffer);
            for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
                if info.glyph_id == 0 {
                    return Err(missing("font_glyph_missing"));
                }
                let cluster = range.start + info.cluster as usize;
                if cluster >= range.end || !text.is_char_boundary(cluster) {
                    return Err(Failure::new(
                        Category::InternalError,
                        "invalid_glyph_cluster",
                    ));
                }
                glyphs.push(Glyph {
                    id: info.glyph_id,
                    cluster,
                    advance_x: f64::from(position.x_advance) * factor,
                    advance_y: f64::from(position.y_advance) * factor,
                    offset_x: f64::from(position.x_offset) * factor,
                    offset_y: f64::from(position.y_offset) * factor,
                });
            }
        }
        Ok(ShapedRun {
            advance: glyphs.iter().map(|g| g.advance_x).sum(),
            glyphs,
            ascender: f64::from(face.ascender()) * factor,
            descender: f64::from(face.descender()) * factor,
            line_gap: f64::from(face.line_gap()) * factor,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Glyph {
    pub id: u32,
    pub cluster: usize,
    pub advance_x: f64,
    pub advance_y: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}
#[derive(Clone, Debug)]
pub struct ShapedRun {
    pub glyphs: Vec<Glyph>,
    pub advance: f64,
    pub ascender: f64,
    pub descender: f64,
    pub line_gap: f64,
}
impl ShapedRun {
    /// 各字形の送り幅を unit の整数倍へ丸め、run の送り幅を合計し直す。
    pub fn snap_advances(&mut self, unit: f64) -> Result<()> {
        if !unit.is_finite() || unit <= 0.0 {
            return Err(Failure::new(
                Category::LayoutFailure,
                "invalid_advance_unit",
            ));
        }
        for glyph in &mut self.glyphs {
            glyph.advance_x = (glyph.advance_x / unit).round() * unit;
        }
        self.advance = self.glyphs.iter().map(|glyph| glyph.advance_x).sum();
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub index: usize,
    pub substituted: bool,
}
#[derive(Clone, Default)]
pub struct FontBook {
    pub faces: Vec<FontFace>,
    total: u64,
}
fn missing(reason: &'static str) -> Failure {
    Failure::new(Category::MissingFont, reason)
}
pub(crate) fn validate_size(points: f64) -> Result<()> {
    if !points.is_finite() || !(1.0..=409.0).contains(&points) {
        return Err(Failure::input("invalid_font_size"));
    }
    Ok(())
}

/// 保存サイズ 1〜409 pt に、対応する印刷倍率 10〜400% を適用した範囲。
/// 保存サイズ自体の検査を緩めず、描画時のサイズだけ別に検査する。
pub(crate) fn validate_print_size(points: f64) -> Result<()> {
    if !points.is_finite() || !(0.1..=1636.0).contains(&points) {
        return Err(Failure::input("invalid_print_font_size"));
    }
    Ok(())
}

impl FontBook {
    /// 指定されたファイルだけを読む。OS のフォント探索は別途許可された入口から行う。
    pub fn load_file(&mut self, path: &Path, limits: &FontLimits) -> Result<()> {
        let mut file = File::open(path).map_err(|_| missing("font_file_unreadable"))?;
        self.load_opened(&mut file, limits)
    }

    /// 名前索引の検査に使ったハンドルから本体を読む。パスを再度開かない。
    pub(crate) fn load_opened(&mut self, file: &mut File, limits: &FontLimits) -> Result<()> {
        let metadata = file
            .metadata()
            .map_err(|_| missing("font_file_unreadable"))?;
        if !metadata.is_file() || metadata.len() > limits.file_bytes {
            return Err(Failure::input("font_file_limit"));
        }
        let allowed = limits
            .file_bytes
            .min(limits.total_bytes.saturating_sub(self.total));
        if metadata.len() > allowed {
            return Err(Failure::input("font_total_limit"));
        }
        let mut data = Vec::new();
        file.seek(SeekFrom::Start(0))
            .map_err(|_| missing("font_file_unreadable"))?;
        file.take(allowed.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|_| missing("font_file_unreadable"))?;
        self.load_bytes(data, limits)
    }

    pub fn load_bytes(&mut self, data: Vec<u8>, limits: &FontLimits) -> Result<()> {
        if data.len() as u64 > limits.file_bytes {
            return Err(Failure::input("font_file_limit"));
        }
        if self.faces.iter().any(|f| f.data.as_slice() == data) {
            return Ok(());
        }
        let total = self
            .total
            .checked_add(data.len() as u64)
            .filter(|n| *n <= limits.total_bytes)
            .ok_or_else(|| Failure::input("font_total_limit"))?;
        let count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        if self
            .faces
            .len()
            .checked_add(count as usize)
            .is_none_or(|n| n > limits.faces)
        {
            return Err(Failure::input("font_face_limit"));
        }
        let data = Arc::new(data);
        let mut faces = Vec::new();
        for collection_index in 0..count {
            let face = Face::parse(&data, collection_index)
                .map_err(|_| Failure::input("invalid_font_data"))?;
            let mut families = BTreeSet::new();
            let mut postscript_name = None;
            for name in face.names() {
                if !matches!(
                    name.name_id,
                    ttf_parser::name_id::FAMILY
                        | ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                        | ttf_parser::name_id::POST_SCRIPT_NAME
                ) {
                    continue;
                }
                if let Some(value) = crate::xlsx::font_names::decode(&name) {
                    match name.name_id {
                        ttf_parser::name_id::FAMILY | ttf_parser::name_id::TYPOGRAPHIC_FAMILY => {
                            if !value.is_empty() {
                                families.insert(value);
                            }
                        }
                        ttf_parser::name_id::POST_SCRIPT_NAME if postscript_name.is_none() => {
                            postscript_name = Some(value);
                        }
                        _ => {}
                    }
                }
            }
            if families.is_empty() {
                return Err(missing("font_family_name_missing"));
            }
            faces.push(FontFace {
                families,
                postscript_name,
                bold: face.is_bold(),
                italic: face.is_italic(),
                collection_index,
                data: data.clone(),
            });
        }
        self.faces.extend(faces);
        self.total = total;
        Ok(())
    }

    /// 同一書体の異なるスタイルを偽の太字・斜体で代用しない。
    /// 代用先は呼び出し元が明示した書体名だけに限定する。
    pub fn select(
        &self,
        family: &str,
        bold: bool,
        italic: bool,
        fallback: Option<&str>,
    ) -> Result<Selection> {
        match self.find(family, bold, italic) {
            Ok(index) => Ok(Selection {
                index,
                substituted: false,
            }),
            Err(error) if error.category == Category::MissingFont => {
                if let Some(replacement) = fallback {
                    Ok(Selection {
                        index: self.find(replacement, bold, italic)?,
                        substituted: true,
                    })
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    fn find(&self, family: &str, bold: bool, italic: bool) -> Result<usize> {
        let family_faces: Vec<_> = self
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.families
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(family))
            })
            .collect();
        if family_faces.is_empty() {
            return Err(missing("font_family_missing"));
        }
        let matches: Vec<_> = family_faces
            .into_iter()
            .filter(|(_, f)| f.bold == bold && f.italic == italic)
            .collect();
        match matches.as_slice() {
            [] => Err(missing("font_style_missing")),
            [(index, face)] => {
                face.check_embedding()?;
                Ok(*index)
            }
            _ => Err(Failure::unsupported("ambiguous_font_face")),
        }
    }
}

impl FontBook {
    /// テーマを解決した要求にも、通常書体と同じ厳密な選択規則を適用する。
    pub fn select_request(
        &self,
        request: &crate::xlsx::theme::FontRequest<'_>,
        fallback: Option<&str>,
    ) -> Result<Selection> {
        self.select(request.family, request.bold, request.italic, fallback)
    }
}
