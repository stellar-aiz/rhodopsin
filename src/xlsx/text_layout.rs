//! 表示形式の文字列・空白・埋め文字を一行の相対位置へ変換する。
use crate::xlsx::{
    diagnostic::{Category, Failure, Result},
    fonts::{FontFace, FontLimits, ShapedRun},
    format::{DisplayValue, Fragment},
    pdf::graphics::BoxRect,
    styles::{Horizontal, Vertical},
    workbook::CellValue,
};

#[derive(Debug)]
pub struct Run {
    pub text: String,
    pub x: f64,
    pub shaped: ShapedRun,
}
#[derive(Debug)]
pub struct Line {
    pub runs: Vec<Run>,
    /// 非描画の空白も含めた送り幅。
    pub advance: f64,
    pub overflow: bool,
}

#[derive(Debug)]
pub struct NumericLine {
    pub line: Line,
    /// true なら、呼び出し側は元セルの位置を添えて幅不足警告を記録する。
    pub hashes: bool,
}

impl NumericLine {
    /// 元セルの番地を指定して警告へ接続する。値や表示文字列は集計へ渡さない。
    pub fn record_warning(
        &self,
        warnings: &mut crate::xlsx::warnings::Warnings,
        sheet_index: usize,
        address: &str,
        limit: usize,
    ) -> Result<()> {
        if self.hashes {
            warnings.record(
                crate::xlsx::warnings::Kind::NumericOverflow,
                sheet_index,
                address,
                limit,
            )?;
        }
        Ok(())
    }
}

/// 表示内容が確定した数値を計測し、幅不足なら # の繰り返しに置換する。
/// 標準書式の桁数・指数表記の選択や、日付エラーの判定は呼び出し前に行う。
/// 色は元の DisplayValue のまま使う。文字列セルには使用しない。
pub fn measure_fixed_number(
    display: &DisplayValue,
    font: &FontFace,
    points: f64,
    width: f64,
    limits: &FontLimits,
) -> Result<NumericLine> {
    let line = measure(display, font, points, width, limits)?;
    if !line.overflow {
        return Ok(NumericLine {
            line,
            hashes: false,
        });
    }
    numeric_hashes(font, points, width, limits)
}

fn numeric_hashes(
    font: &FontFace,
    points: f64,
    width: f64,
    limits: &FontLimits,
) -> Result<NumericLine> {
    let line = measure(
        &DisplayValue {
            fragments: vec![Fragment::Fill('#')],
            color: None,
        },
        font,
        points,
        width,
        limits,
    )?;
    // 1 文字も入らない幅の Excel の切り取り規則は未検証。空白の成功にしない。
    if line.runs.is_empty() {
        return Err(Failure::unsupported("numeric_hash_clipping"));
    }
    Ok(NumericLine { line, hashes: true })
}

/// General の固定小数・指数候補を実書体で計測する。
/// 負の指数が -3 未満で固定小数の有効桁を失う場合は、より多くの桁を残す指数を選ぶ。
/// どちらも入らない場合は既存の # 表示と位置付き警告へ接続する。
pub fn measure_general_number(
    raw: &str,
    font: &FontFace,
    points: f64,
    width: f64,
    limits: &FontLimits,
) -> Result<NumericLine> {
    let candidates = crate::xlsx::format::general::candidates(raw)?;
    let fit =
        |values: &[crate::xlsx::format::general::Candidate]| -> Result<Option<(Line, usize)>> {
            for candidate in values {
                let line = measure(
                    &DisplayValue {
                        fragments: vec![Fragment::Text(candidate.text.clone())],
                        color: None,
                    },
                    font,
                    points,
                    width,
                    limits,
                )?;
                if !line.overflow {
                    return Ok(Some((line, candidate.precision)));
                }
            }
            Ok(None)
        };
    let fixed = fit(&candidates.fixed)?;
    let try_scientific = fixed.as_ref().is_none_or(|(_, precision)| {
        candidates.order < -3 && *precision < candidates.significant_digits
    });
    if try_scientific
        && let Some((line, precision)) = fit(&candidates.scientific)?
        && fixed
            .as_ref()
            .is_none_or(|(_, fixed_precision)| precision > *fixed_precision)
    {
        return Ok(NumericLine {
            line,
            hashes: false,
        });
    }
    if let Some((line, _)) = fixed {
        return Ok(NumericLine {
            line,
            hashes: false,
        });
    }
    numeric_hashes(font, points, width, limits)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinePosition {
    /// 行内の Run.x に加える、ページ左端からの距離。
    pub x: f64,
    /// ページ上端を原点とするベースライン。
    pub baseline: f64,
}

/// 標準配置の型依存を解決する。数式も保存済み結果の型を渡す。
pub fn horizontal_for_value(alignment: Horizontal, value: &CellValue) -> Horizontal {
    if alignment != Horizontal::General {
        return alignment;
    }
    match value {
        CellValue::Blank | CellValue::Text(_) | CellValue::RichText(_) => Horizontal::Left,
        CellValue::Number(_) => Horizontal::Right,
        CellValue::Boolean(_) | CellValue::Error(_) => Horizontal::Center,
    }
}

/// 一行を内容矩形へ配置する。内容矩形の余白・字下げは呼び出し側で確定する。
/// 幅超過でも整列原点を保持し、切り取り・はみ出し・数値の ### 化は別層で扱う。
/// ascender/descender はベースラインからの符号付き距離（ポイント）。
pub fn position_line(
    line: &Line,
    content: BoxRect,
    horizontal: Horizontal,
    vertical: Vertical,
    ascender: f64,
    descender: f64,
) -> Result<LinePosition> {
    if !ascender.is_finite()
        || !descender.is_finite()
        || ascender < 0.0
        || descender > 0.0
        || ascender <= descender
    {
        return Err(layout_error("invalid_line_geometry"));
    }
    let height = ascender - descender;
    position_line_in_box(line, content, horizontal, vertical, height, ascender)
}

/// 行箱を内容矩形の上端・中央・下端に置き、行箱の上端から baseline の位置を返す。
pub fn position_line_in_box(
    line: &Line,
    content: BoxRect,
    horizontal: Horizontal,
    vertical: Vertical,
    height: f64,
    baseline: f64,
) -> Result<LinePosition> {
    if [
        content.x,
        content.y,
        content.width,
        content.height,
        line.advance,
        height,
        baseline,
    ]
    .iter()
    .any(|n| !n.is_finite())
        || content.width < 0.0
        || content.height < 0.0
        || line.advance < 0.0
        || height <= 0.0
        || !(0.0..=height).contains(&baseline)
    {
        return Err(layout_error("invalid_line_geometry"));
    }
    let x = content.x
        + match horizontal {
            Horizontal::Left => 0.0,
            Horizontal::Center => (content.width - line.advance) / 2.0,
            Horizontal::Right => content.width - line.advance,
            Horizontal::General => return Err(layout_error("unresolved_general_alignment")),
            _ => return Err(Failure::unsupported("horizontal_text_layout")),
        };
    let baseline = content.y
        + baseline
        + match vertical {
            Vertical::Top => 0.0,
            Vertical::Center => (content.height - height) / 2.0,
            Vertical::Bottom => content.height - height,
            _ => return Err(Failure::unsupported("vertical_text_layout")),
        };
    if !x.is_finite() || !baseline.is_finite() {
        return Err(layout_error("invalid_line_geometry"));
    }
    Ok(LinePosition { x, baseline })
}

/// 各字形の送り幅を unit の整数倍へ丸め、後続の run と行の送り幅を詰め直す。
pub fn snap_advances(line: &mut Line, unit: f64) -> Result<()> {
    let mut shift = 0.0;
    for run in &mut line.runs {
        run.x += shift;
        let before = run.shaped.advance;
        run.shaped.snap_advances(unit)?;
        shift += run.shaped.advance - before;
    }
    line.advance += shift;
    if !line.advance.is_finite() || line.advance < 0.0 {
        return Err(layout_error("invalid_line_geometry"));
    }
    Ok(())
}
fn layout_error(reason: &'static str) -> Failure {
    Failure::new(Category::LayoutFailure, reason)
}

const MAX_LINES: usize = 4096;

/// Unicode の改行候補を実フォントで計測する検証用の折り返し。
/// 空白を削除せず、候補のない長い単語を任意の文字位置で分割しない。
/// work_limit は再計測する UTF-8 バイト数の合計上限。
pub fn measure_wrapped(
    text: &str,
    font: &FontFace,
    points: f64,
    width: f64,
    limits: &FontLimits,
    work_limit: usize,
) -> Result<Vec<Line>> {
    // 折り返し後も段落の基底方向を保持する必要がある。行ごとの推定で置換しない。
    if crate::xlsx::text_direction::has_rtl(text) {
        return Err(Failure::unsupported("wrapped_bidi_layout"));
    }

    if !width.is_finite() || width <= 0.0 {
        return Err(layout_error("invalid_text_width"));
    }
    if text.len() > limits.run_bytes {
        return Err(Failure::input("text_run_limit"));
    }
    let mut remaining = work_limit;
    let mut result = Vec::new();
    for paragraph in text.split('\n') {
        if result.len() >= MAX_LINES {
            return Err(Failure::input("text_line_limit"));
        }
        remaining = remaining
            .checked_sub(paragraph.len() + 1)
            .ok_or_else(|| Failure::input("text_wrap_work_limit"))?;
        let full = measure(
            &DisplayValue {
                fragments: vec![Fragment::Text(paragraph.into())],
                color: None,
            },
            font,
            points,
            width,
            limits,
        )?;
        if !full.overflow {
            result.push(full);
            continue;
        }
        let mut candidates = Vec::new();
        for (end, opportunity) in unicode_linebreak::linebreaks(paragraph) {
            if opportunity == unicode_linebreak::BreakOpportunity::Mandatory
                && end != paragraph.len()
            {
                return Err(Failure::unsupported("text_line_separator"));
            }
            candidates.push(end);
        }
        if paragraph.is_empty() {
            candidates.push(0);
        }
        let mut start = 0;
        loop {
            if result.len() >= MAX_LINES {
                return Err(Failure::input("text_line_limit"));
            }
            let mut best = None;
            for &end in &candidates {
                remaining = remaining
                    .checked_sub(1)
                    .ok_or_else(|| Failure::input("text_wrap_work_limit"))?;
                if end <= start && !paragraph.is_empty() {
                    continue;
                }
                remaining = remaining
                    .checked_sub(end - start + 1)
                    .ok_or_else(|| Failure::input("text_wrap_work_limit"))?;
                let line = measure(
                    &DisplayValue {
                        fragments: vec![Fragment::Text(paragraph[start..end].into())],
                        color: None,
                    },
                    font,
                    points,
                    width,
                    limits,
                )?;
                if !line.overflow {
                    best = Some((end, line));
                }
            }
            let (end, line) = best.ok_or_else(|| Failure::unsupported("automatic_text_wrap"))?;
            result.push(line);
            start = end;
            if start == paragraph.len() {
                break;
            }
        }
    }
    Ok(result)
}

/// 明示された LF 改行を分割して計測する。空行・末尾の改行も行として保持する。
/// 自動折り返しは行わない。CR 等の別の制御文字は従来どおり拒否する。
pub fn measure_lines(
    text: &str,
    font: &FontFace,
    points: f64,
    width: f64,
    limits: &FontLimits,
) -> Result<Vec<Line>> {
    // 行ごとの制限だけでは、改行によって総量制限を回避できてしまう。
    if text.len() > limits.run_bytes {
        return Err(Failure::input("text_run_limit"));
    }
    if text.bytes().filter(|b| *b == b'\n').count() >= MAX_LINES {
        return Err(Failure::input("text_line_limit"));
    }
    text.split('\n')
        .map(|line| {
            measure(
                &DisplayValue {
                    fragments: vec![Fragment::Text(line.into())],
                    color: None,
                },
                font,
                points,
                width,
                limits,
            )
        })
        .collect()
}

/// 同じフォント寸法を持つ複数行を配置する。baseline_step は明示された行送り。
/// 最初の昇部から最後の降部までを上下整列し、各行を個別に左右整列する。
/// Excel の行送りをフォントサイズから推測する処理はここには含めない。
pub fn position_lines(
    lines: &[Line],
    content: BoxRect,
    horizontal: Horizontal,
    vertical: Vertical,
    ascender: f64,
    descender: f64,
    baseline_step: f64,
) -> Result<Vec<LinePosition>> {
    if lines.is_empty() || lines.len() > MAX_LINES {
        return Err(Failure::input("text_line_limit"));
    }
    if !baseline_step.is_finite() || baseline_step <= 0.0 {
        return Err(layout_error("invalid_line_spacing"));
    }
    // 元の昇部・降部も検査し、複数行化で不正な降部が隠れないようにする。
    position_line(
        &lines[0], content, horizontal, vertical, ascender, descender,
    )?;
    let first = position_line(
        &lines[0],
        content,
        horizontal,
        vertical,
        ascender,
        descender - (lines.len() - 1) as f64 * baseline_step,
    )?;
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let position = position_line(
                line,
                content,
                horizontal,
                Vertical::Top,
                ascender,
                descender,
            )?;
            let baseline = first.baseline + index as f64 * baseline_step;
            if !baseline.is_finite() {
                return Err(layout_error("invalid_line_geometry"));
            }
            Ok(LinePosition {
                x: position.x,
                baseline,
            })
        })
        .collect()
}

/// width は余白・字下げを除いた内容幅。色・縦位置・折り返しは別層で扱う。
pub fn measure(
    display: &DisplayValue,
    font: &FontFace,
    points: f64,
    width: f64,
    limits: &FontLimits,
) -> Result<Line> {
    if display.fragments.len() > 1 && display.fragments.iter().any(|fragment|
        matches!(fragment, Fragment::Text(text) if crate::xlsx::text_direction::has_rtl(text))) {
        return Err(Failure::unsupported("formatted_bidi_layout"));
    }

    crate::xlsx::fonts::validate_print_size(points)?;
    if !width.is_finite() || width < 0.0 {
        return Err(layout_error("invalid_text_width"));
    }
    if display.fragments.len() > 4096 {
        return Err(Failure::input("text_fragment_limit"));
    }
    let mut bytes = 0usize;
    let mut fills = 0;
    for fragment in &display.fragments {
        let length = match fragment {
            Fragment::Text(text) => text.len(),
            Fragment::Space(ch) => ch.len_utf8(),
            Fragment::Fill(ch) => {
                fills += 1;
                ch.len_utf8()
            }
        };
        bytes = bytes
            .checked_add(length)
            .filter(|n| *n <= limits.run_bytes)
            .ok_or_else(|| Failure::input("text_run_limit"))?;
    }
    if fills > 1 {
        return Err(Failure::unsupported("multiple_number_format_fills"));
    }
    let mut parts: Vec<(Option<String>, Option<ShapedRun>, f64)> = Vec::new();
    let mut fixed = 0.0;
    let mut fill = None;
    for fragment in &display.fragments {
        match fragment {
            Fragment::Text(text) if text.is_empty() => parts.push((None, None, 0.0)),
            Fragment::Text(text) => {
                let run = font.shape_run(text, points, limits)?;
                fixed += run.advance;
                let advance = run.advance;
                parts.push((Some(text.clone()), Some(run), advance));
            }
            Fragment::Space(ch) => {
                let run = font.shape_run(&ch.to_string(), points, limits)?;
                fixed += run.advance;
                parts.push((None, None, run.advance));
            }
            Fragment::Fill(ch) => {
                fill = Some((parts.len(), *ch));
                parts.push((None, None, 0.0));
            }
        }
    }
    if !fixed.is_finite() || fixed < 0.0 {
        return Err(layout_error("invalid_text_advance"));
    }
    if let Some((index, ch)) = fill {
        let single = font.shape_run(&ch.to_string(), points, limits)?;
        if single.glyphs.len() != 1 || single.advance <= 0.0 || single.glyphs[0].advance_y != 0.0 {
            return Err(Failure::unsupported("contextual_fill_character"));
        }
        let room = (width - fixed).max(0.0);
        let count = (room / single.advance).floor();
        let remaining = limits.run_bytes.saturating_sub(bytes - ch.len_utf8());
        if count > (remaining / ch.len_utf8()) as f64 {
            return Err(Failure::input("text_fill_limit"));
        }
        let mut count = count as usize;
        if count > 0 {
            let mut text = ch.to_string().repeat(count);
            let mut run = font.shape_run(&text, points, limits)?;
            // 文脈によって字形・送りが変わる埋め文字を単純な積算で配置しない。
            if run.glyphs.len() != count
                || run.glyphs.iter().any(|g| {
                    g.id != single.glyphs[0].id
                        || g.advance_x != single.advance
                        || g.advance_y != 0.0
                })
            {
                return Err(Failure::unsupported("contextual_fill_character"));
            }
            // 浮動小数点の積と和の差が境界をまたいだ場合も、幅を越えない。
            if run.advance > room {
                count -= 1;
                text = ch.to_string().repeat(count);
                if count == 0 {
                    run.glyphs.clear();
                    run.advance = 0.0;
                } else {
                    run = font.shape_run(&text, points, limits)?;
                }
            }
            if run.advance > room
                || run.glyphs.len() != count
                || run.glyphs.iter().any(|g| {
                    g.id != single.glyphs[0].id
                        || g.advance_x != single.advance
                        || g.advance_y != 0.0
                })
            {
                return Err(Failure::unsupported("contextual_fill_character"));
            }
            let advance = run.advance;
            parts[index] = (Some(text), Some(run), advance);
        }
    }
    let mut runs = Vec::new();
    let mut x = 0.0;
    for (text, shaped, advance) in parts {
        if let (Some(text), Some(shaped)) = (text, shaped)
            && !text.is_empty()
        {
            runs.push(Run { text, x, shaped });
        }
        x += advance;
    }
    Ok(Line {
        runs,
        advance: x,
        overflow: fixed > width,
    })
}
