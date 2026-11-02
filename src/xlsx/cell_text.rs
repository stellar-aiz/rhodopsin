//! セルの表示形式から配置済みの PDF 文字までを接続する。
use crate::xlsx::{
    color::ColorContext,
    diagnostic::{Failure, Result},
    fonts::{FontBook, FontLimits},
    format::{self, FormatContext, Fragment},
    pdf::{self, graphics::BoxRect},
    styles::{Horizontal, Styles, Underline},
    text_layout::{
        horizontal_for_value, measure, measure_fixed_number, measure_lines, measure_wrapped,
        position_line, position_line_in_box, position_lines, snap_advances,
    },
    theme::{TextScript, resolve_font},
    warnings::{Kind, Warnings},
    workbook::{Cell, CellValue, cell_address},
};

mod rich;

pub struct Context<'a> {
    pub styles: &'a Styles,
    pub fonts: &'a FontBook,
    pub colors: ColorContext<'a>,
    pub format: FormatContext,
    pub script: TextScript<'a>,
    /// 列幅と字下げに使う標準書体の文字体系。本文用の script とは独立。
    pub normal_script: TextScript<'a>,
    pub font_limits: &'a FontLimits,
    pub warning_limit: usize,
    /// None なら代用しない。太字・斜体の一致と埋め込み可否は代用時にも検査する。
    pub fallback_family: Option<&'a str>,
}

/// 内容矩形はセル余白を除いたページ座標。字下げは plan が適用する。
/// clip は隣接セルを考慮して決める。
pub struct Frame {
    pub content: BoxRect,
    pub clip: BoxRect,
    pub scale: f64,
    /// 複数行の場合に必要な、倍率適用済みのベースライン間隔（ポイント）。
    pub baseline_step: Option<f64>,
    /// 一行の文字を置く行箱。None なら内容矩形とフォントの ascender/descender で置く。
    pub line_box: Option<LineBox>,
}

/// 一行の文字を置く行箱と字送りの格子。値は倍率適用済みのポイント。
/// 縦揃えは行箱を内容矩形の上端・中央・下端に置き、ベースラインは行箱の上端から測る。
#[derive(Clone, Copy, Debug)]
pub struct LineBox {
    pub height: f64,
    pub baseline: f64,
    /// 各字形の送り幅をこの単位の整数倍へ丸める。
    pub advance_unit: f64,
}

pub struct Text {
    pub(crate) origin: (usize, u32, u32),
    text: String,
    font: usize,
    points: f64,
    x: f64,
    baseline: f64,
    color: [u8; 3],
    clip: BoxRect,
    underline: Option<BoxRect>,
}
impl Text {
    pub fn decoration(&self) -> Option<pdf::graphics::Paint<'static>> {
        self.underline.map(|rect| pdf::graphics::Paint::Fill {
            rect,
            color: self.color,
        })
    }
    pub fn pdf(&self) -> pdf::Text<'_> {
        pdf::Text {
            text: &self.text,
            font: self.font,
            points: self.points,
            x: self.x,
            baseline: self.baseline,
            color: self.color,
            clip: Some(self.clip),
        }
    }
}

/// 同一セルの同一行だけをまとめ、行の文字、下線の順で描画する。
/// 別セルを座標だけで結合しない。複数行の明示境界も保持する。
pub fn pdf_items(texts: &[Text]) -> Vec<pdf::Item<'_>> {
    let mut items = Vec::new();
    for line in
        texts.chunk_by(|a, b| a.origin == b.origin && a.baseline == b.baseline && a.clip == b.clip)
    {
        if line.len() == 1 {
            items.push(pdf::Item::Text(line[0].pdf()));
        } else {
            items.push(pdf::Item::TextLine(line.iter().map(Text::pdf).collect()));
        }
        for paint in line.iter().filter_map(Text::decoration) {
            items.push(pdf::Item::Paint(paint));
        }
    }
    items
}

/// セルを配置する。一重下線は OpenType の寸法で描く検証用実装。
/// 内容矩形・切り取り矩形の Excel 固有規則はこの関数の呼び出し側で確定する。
pub fn plan(
    cell: &Cell,
    sheet_index: usize,
    frame: Frame,
    context: &Context<'_>,
    warnings: &mut Warnings,
) -> Result<Vec<Text>> {
    if sheet_index == 0 || cell_address(&cell.address).is_none() {
        return Err(Failure::input("invalid_cell_location"));
    }
    let at = |error: Failure| error.at(sheet_index, Some(cell.address.clone()));
    let work = || -> Result<(Vec<Text>, bool, bool)> {
        if !frame.scale.is_finite() || !(0.1..=4.0).contains(&frame.scale) {
            return Err(Failure::input("invalid_page_transform"));
        }
        for rect in [frame.content, frame.clip] {
            if [
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                rect.x + rect.width,
                rect.y + rect.height,
            ]
            .iter()
            .any(|v| !v.is_finite())
                || rect.width <= 0.0
                || rect.height <= 0.0
            {
                return Err(Failure::input("invalid_text_rectangle"));
            }
        }
        let style = context.styles.validate_cell(cell.style)?;
        if matches!(cell.value, CellValue::Blank) {
            return Ok((Vec::new(), false, false));
        }
        let mut content = frame.content;
        let mut indent_substituted = false;
        if style.alignment.indent != 0 {
            if !matches!(
                style.alignment.horizontal,
                Horizontal::Left | Horizontal::Right
            ) {
                return Err(Failure::unsupported("indented_horizontal_alignment"));
            }
            let index = context
                .styles
                .normal_font
                .ok_or_else(|| Failure::unsupported("normal_style_font_unresolved"))?;
            let normal = context
                .styles
                .fonts
                .get(index)
                .ok_or_else(|| Failure::input("font_style_index"))?
                .as_ref()
                .map_err(Clone::clone)?;
            let request = resolve_font(normal, context.colors.theme, context.normal_script)?;
            let selection = context
                .fonts
                .select_request(&request, context.fallback_family)?;
            let space = context.fonts.faces[selection.index]
                .shape_run(" ", request.points * frame.scale, context.font_limits)?
                .advance;
            let indent = space * 3.0 * f64::from(style.alignment.indent);
            if !indent.is_finite() || indent <= 0.0 || indent >= content.width {
                return Err(Failure::unsupported("indent_exceeds_content_width"));
            }
            if style.alignment.horizontal == Horizontal::Left {
                content.x += indent;
            }
            content.width -= indent;
            indent_substituted = selection.substituted;
        }
        if let CellValue::RichText(runs) = &cell.value {
            // 書式の異なる run を一つの行箱へ置く規則は測っていない。
            if frame.line_box.is_some() {
                return Err(Failure::unsupported("rich_text_line_box"));
            }
            let (texts, substituted) = rich::plan(runs, style, &frame, content, context)?;
            return Ok((texts, false, substituted || indent_substituted));
        }
        let font = context.styles.fonts[style.font]
            .as_ref()
            .map_err(Clone::clone)?;
        if !matches!(font.underline, Underline::None | Underline::Single) {
            return Err(Failure::unsupported("underlined_cell_text"));
        }
        let request = resolve_font(font, context.colors.theme, context.script)?;
        let selection = context
            .fonts
            .select_request(&request, context.fallback_family)?;
        let face = &context.fonts.faces[selection.index];
        let points = request.points * frame.scale;
        let general = format::general_numeric_lexeme(cell, context.styles)?
            .map(|raw| {
                crate::xlsx::text_layout::measure_general_number(
                    raw,
                    face,
                    points,
                    content.width,
                    context.font_limits,
                )
            })
            .transpose()?;
        let display = if let Some(result) = &general {
            format::DisplayValue {
                fragments: vec![Fragment::Text(
                    result
                        .line
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect(),
                )],
                color: None,
            }
        } else {
            format::cell_with_context(cell, Some(context.styles), context.format)?
        };
        let underline = if font.underline == Underline::Single {
            if !matches!(display.fragments.as_slice(), [Fragment::Text(_)]) {
                return Err(Failure::unsupported("formatted_text_underline"));
            }
            Some(face.underline_metrics(points)?)
        } else {
            None
        };
        let color = display
            .color
            .as_ref()
            .unwrap_or(&font.color)
            .resolve(&context.colors)?;
        let explicit_lines = match display.fragments.as_slice() {
            [Fragment::Text(text)]
                if matches!(cell.value, CellValue::Text(_))
                    && style.alignment.wrap
                    && text.contains('\n') =>
            {
                Some(text.as_str())
            }
            _ => None,
        };
        if style.alignment.wrap
            && !(matches!(cell.value, CellValue::Text(_))
                && matches!(display.fragments.as_slice(), [Fragment::Text(_)]))
        {
            return Err(Failure::unsupported("wrapped_cell_text"));
        }
        let automatic = if style.alignment.wrap && frame.baseline_step.is_some() {
            match display.fragments.as_slice() {
                [Fragment::Text(text)] => Some(measure_wrapped(
                    text,
                    face,
                    points,
                    content.width,
                    context.font_limits,
                    context.font_limits.run_bytes.saturating_mul(16),
                )?),
                _ => None,
            }
        } else {
            None
        };
        let (lines, hashes) = if let Some(lines) = automatic {
            if !frame
                .baseline_step
                .is_some_and(|step| step.is_finite() && step > 0.0)
            {
                return Err(Failure::unsupported("cell_line_spacing_unresolved"));
            }
            (lines, false)
        } else if let Some(text) = explicit_lines {
            let lines = measure_lines(text, face, points, content.width, context.font_limits)?;
            if lines.iter().any(|line| line.overflow) {
                return Err(Failure::unsupported("automatic_text_wrap"));
            }
            // 空行だけでも未確定・不正な行送りを成功扱いにしない。
            if !frame
                .baseline_step
                .is_some_and(|step| step.is_finite() && step > 0.0)
            {
                return Err(Failure::unsupported("cell_line_spacing_unresolved"));
            }
            (lines, false)
        } else if let Some(result) = general {
            (vec![result.line], result.hashes)
        } else if matches!(cell.value, CellValue::Number(_)) {
            let result =
                measure_fixed_number(&display, face, points, content.width, context.font_limits)?;
            (vec![result.line], result.hashes)
        } else {
            (
                vec![measure(
                    &display,
                    face,
                    points,
                    content.width,
                    context.font_limits,
                )?],
                false,
            )
        };
        if style.alignment.wrap && lines.iter().any(|line| line.overflow) {
            return Err(Failure::unsupported("automatic_text_wrap"));
        }
        let mut lines = lines;
        if let Some(line_box) = frame.line_box {
            match lines.as_mut_slice() {
                [] => {}
                [line] => snap_advances(line, line_box.advance_unit)?,
                _ => return Err(Failure::unsupported("cell_line_spacing_unresolved")),
            }
        }
        let Some(first) = lines.iter().flat_map(|line| &line.runs).next() else {
            return Ok((Vec::new(), hashes, false));
        };
        let horizontal = horizontal_for_value(style.alignment.horizontal, &cell.value);
        let positions = if lines.len() > 1 {
            position_lines(
                &lines,
                content,
                horizontal,
                style.alignment.vertical,
                first.shaped.ascender,
                first.shaped.descender,
                frame
                    .baseline_step
                    .ok_or_else(|| Failure::unsupported("cell_line_spacing_unresolved"))?,
            )?
        } else {
            vec![match frame.line_box {
                Some(line_box) => position_line_in_box(
                    &lines[0],
                    content,
                    horizontal,
                    style.alignment.vertical,
                    line_box.height,
                    line_box.baseline,
                )?,
                None => position_line(
                    &lines[0],
                    content,
                    horizontal,
                    style.alignment.vertical,
                    first.shaped.ascender,
                    first.shaped.descender,
                )?,
            }]
        };
        let mut result = Vec::new();
        for (line, position) in lines.into_iter().zip(positions) {
            for run in line.runs {
                let x = position.x + run.x;
                if !x.is_finite() {
                    return Err(Failure::input("invalid_text_rectangle"));
                }
                result.push(Text {
                    origin: (0, 0, 0),
                    underline: underline.and_then(|(offset, thickness)| {
                        let left = x.max(frame.clip.x);
                        let right = (x + run.shaped.advance).min(frame.clip.x + frame.clip.width);
                        let top = (position.baseline + offset).max(frame.clip.y);
                        let bottom = (position.baseline + offset + thickness)
                            .min(frame.clip.y + frame.clip.height);
                        (right > left && bottom > top).then_some(BoxRect {
                            x: left,
                            y: top,
                            width: right - left,
                            height: bottom - top,
                        })
                    }),
                    text: run.text,
                    font: selection.index,
                    points,
                    x,
                    baseline: position.baseline,
                    color,
                    clip: frame.clip,
                });
            }
        }
        Ok((result, hashes, selection.substituted || indent_substituted))
    };
    let (mut texts, hashes, substituted) = work().map_err(at)?;
    let (column, row) =
        cell_address(&cell.address).ok_or_else(|| Failure::input("invalid_cell_location"))?;
    for text in &mut texts {
        text.origin = (sheet_index, column, row);
    }
    let mut kinds = Vec::new();
    if hashes {
        kinds.push(Kind::NumericOverflow);
    }
    if substituted {
        kinds.push(Kind::FontSubstitution);
    }
    warnings
        .record_kinds(&kinds, sheet_index, &cell.address, context.warning_limit)
        .map_err(at)?;
    Ok(texts)
}

/// ページの可視セルを行・列順に配置する。背景・罫線・画像は別レイヤー。
/// frame は余白、隣接セル、行送りを考慮して内容矩形を確定する配置ポリシー。
/// その Excel 固有規則を既定値で推測せず、呼び出し側に明示させる。
/// limit は入力セル数・ページ軸数・生成文字 run 数・結合検査作業数それぞれの上限。
pub fn plan_page(
    sheet: &crate::xlsx::workbook::Sheet,
    page: &crate::xlsx::layout::Page,
    context: &Context<'_>,
    warnings: &mut Warnings,
    limit: usize,
    frame: impl FnMut(&Cell, &crate::xlsx::placement::RangePlacement) -> Result<Frame>,
) -> Result<Vec<Text>> {
    plan_page_limited(
        sheet,
        page,
        context,
        warnings,
        limit,
        32 * 1024 * 1024,
        frame,
    )
}

/// セルを結果へ追加する前に、生成文字列の合計 UTF-8 バイト数を検査する。
/// 途中で失敗した場合は、それまでの文字・警告を返さない。
pub fn plan_page_limited(
    sheet: &crate::xlsx::workbook::Sheet,
    page: &crate::xlsx::layout::Page,
    context: &Context<'_>,
    warnings: &mut Warnings,
    limit: usize,
    text_bytes: usize,
    mut frame: impl FnMut(&Cell, &crate::xlsx::placement::RangePlacement) -> Result<Frame>,
) -> Result<Vec<Text>> {
    use crate::xlsx::{
        placement::{RangePlacement, range_on_page},
        range::CellRange,
    };
    use std::collections::BTreeMap;
    if sheet.index != page.sheet_index {
        return Err(Failure::input("page_sheet_mismatch"));
    }
    if sheet.hidden {
        return Ok(Vec::new());
    }
    if sheet.cells.len() > limit
        || page
            .columns
            .len()
            .checked_add(page.rows.len())
            .is_none_or(|n| n > limit)
    {
        return Err(Failure::input("cell_text_limit"));
    }
    range_on_page(
        page,
        CellRange {
            first_column: 1,
            last_column: 16384,
            first_row: 1,
            last_row: 1048576,
        },
    )?;
    let columns: BTreeMap<_, _> = page.columns.iter().map(|item| (item.index, item)).collect();
    let rows: BTreeMap<_, _> = page.rows.iter().map(|item| (item.index, item)).collect();
    let mut cells = BTreeMap::new();
    for cell in &sheet.cells {
        let (column, row) =
            cell_address(&cell.address).ok_or_else(|| Failure::input("invalid_cell_location"))?;
        if cells.insert((row, column), cell).is_some() {
            return Err(
                Failure::input("duplicate_cell").at(sheet.index, Some(cell.address.clone()))
            );
        }
    }
    let mut merged = BTreeMap::new();
    let mut work = 0usize;
    let mut charge = |count: usize| -> Result<()> {
        work = work
            .checked_add(count)
            .filter(|n| *n <= limit)
            .ok_or_else(|| Failure::input("merged_text_work_limit").at(sheet.index, None))?;
        Ok(())
    };
    for (index, range) in sheet.grid.merges.iter().enumerate() {
        charge(1)?;
        for prior in &sheet.grid.merges[..index] {
            charge(1)?;
            if range.intersects(*prior) {
                return Err(Failure::input("overlapping_merged_cells").at(sheet.index, None));
            }
        }
        charge(page.columns.len() + page.rows.len())?;
        let mut placements =
            range_on_page(page, *range).map_err(|error| error.at(sheet.index, None))?;
        if placements.is_empty() {
            continue;
        }
        // ページ境界・非表示行列・見出し境界で分断された結合は原点規則が未確定。
        let complete = placements.len() == 1
            && placements[0].visible == *range
            && columns
                .range(range.first_column..=range.last_column)
                .count()
                == (range.last_column - range.first_column + 1) as usize
            && rows.range(range.first_row..=range.last_row).count()
                == (range.last_row - range.first_row + 1) as usize;
        if !complete {
            // range_on_page が番地の範囲を検査済み。入力文字列を診断へ転写しない。
            let mut column = range.first_column;
            let mut letters = Vec::new();
            while column > 0 {
                letters.push(char::from(b'A' + ((column - 1) % 26) as u8));
                column = (column - 1) / 26;
            }
            let anchor = format!(
                "{}{}",
                letters.into_iter().rev().collect::<String>(),
                range.first_row
            );
            return Err(
                Failure::unsupported("split_merged_cell_text").at(sheet.index, Some(anchor))
            );
        }
        let anchor = (range.first_row, range.first_column);
        for (&position, cell) in cells.range((range.first_row, 0)..=(range.last_row, 16384)) {
            charge(1)?;
            if position.1 < range.first_column
                || position.1 > range.last_column
                || position == anchor
            {
                continue;
            }
            if !matches!(cell.value, CellValue::Blank) {
                return Err(Failure::unsupported("merged_nonanchor_value")
                    .at(sheet.index, Some(cell.address.clone())));
            }
        }
        merged.insert(anchor, placements.remove(0));
    }
    let mut pending = warnings.clone();
    let mut result = Vec::new();
    let mut remaining_bytes = text_bytes;
    for ((row, column), cell) in cells {
        let (Some(x), Some(y)) = (columns.get(&column), rows.get(&row)) else {
            continue;
        };
        if matches!(cell.value, CellValue::Blank) {
            continue;
        }
        let placement = merged.remove(&(row, column)).unwrap_or(RangePlacement {
            visible: CellRange {
                first_column: column,
                last_column: column,
                first_row: row,
                last_row: row,
            },
            rect: BoxRect {
                x: page.origin_x + x.offset * page.scale,
                y: page.origin_y + y.offset * page.scale,
                width: x.size * page.scale,
                height: y.size * page.scale,
            },
            repeated_rows: y.repeated,
            repeated_columns: x.repeated,
        });
        let cell_frame = frame(cell, &placement)
            .map_err(|error| error.at(sheet.index, Some(cell.address.clone())))?;
        if cell_frame.scale != page.scale {
            return Err(Failure::input("cell_page_scale_mismatch")
                .at(sheet.index, Some(cell.address.clone())));
        }
        let texts = plan(cell, sheet.index, cell_frame, context, &mut pending)?;
        for text in &texts {
            remaining_bytes = remaining_bytes
                .checked_sub(text.text.len())
                .ok_or_else(|| {
                    Failure::input("page_scene_text_limit")
                        .at(sheet.index, Some(cell.address.clone()))
                })?;
        }
        if result
            .len()
            .checked_add(texts.len())
            .is_none_or(|n| n > limit)
        {
            return Err(Failure::input("cell_text_limit"));
        }
        result.extend(texts);
    }
    *warnings = pending;
    Ok(result)
}
