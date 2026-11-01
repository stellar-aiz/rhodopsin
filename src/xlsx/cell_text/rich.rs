//! 書体境界を維持した計測と改行。Excel 固有の余白・行送りは Frame から受け取る。
use super::*;
use crate::xlsx::{rich_text::Run, styles::CellStyle, text_layout::Line};
use std::ops::Range;

struct Span {
    range: Range<usize>,
    font: usize,
    points: f64,
    color: [u8; 3],
    underline: Option<(f64, f64)>,
}
struct Measured {
    line: Line,
    spans: Vec<usize>,
}

pub(super) fn plan(
    runs: &[Run],
    style: &CellStyle,
    frame: &Frame,
    content: BoxRect,
    context: &Context<'_>,
) -> Result<(Vec<Text>, bool)> {
    let (formatted_runs, format_color) =
        crate::xlsx::format::rich_runs(runs, style, context.styles, context.format)?;
    let runs = formatted_runs.as_slice();
    if runs
        .iter()
        .any(|run| crate::xlsx::text_direction::has_rtl(&run.text))
    {
        return Err(Failure::unsupported("rich_text_bidi_layout"));
    }
    if runs.len() > 4096 {
        return Err(Failure::input("text_fragment_limit"));
    }
    let total = runs.iter().try_fold(0usize, |sum, run| {
        sum.checked_add(run.text.len())
            .filter(|n| *n <= context.font_limits.run_bytes)
            .ok_or_else(|| Failure::input("text_run_limit"))
    })?;
    let base = context.styles.fonts[style.font]
        .as_ref()
        .map_err(Clone::clone)?;
    let mut text = String::with_capacity(total);
    let mut spans = Vec::new();
    let mut substituted = false;
    let mut ascent: f64 = 0.0;
    let mut descent: f64 = 0.0;
    for run in runs {
        let font = run.font.as_ref().unwrap_or(base);
        if !matches!(font.underline, Underline::None | Underline::Single) {
            return Err(Failure::unsupported("underlined_cell_text"));
        }
        let request = resolve_font(font, context.colors.theme, context.script)?;
        let selection = context
            .fonts
            .select_request(&request, context.fallback_family)?;
        substituted |= selection.substituted;
        let points = request.points * frame.scale;
        let face = &context.fonts.faces[selection.index];
        // 空行の高さもこの書体の寸法から求める。
        let metrics = face.shape_run("", points, context.font_limits)?;
        ascent = ascent.max(metrics.ascender);
        descent = descent.min(metrics.descender);
        let start = text.len();
        text.push_str(&run.text);
        spans.push(Span {
            range: start..text.len(),
            font: selection.index,
            points,
            color: format_color
                .as_ref()
                .unwrap_or(&font.color)
                .resolve(&context.colors)?,
            underline: if font.underline == Underline::Single {
                Some(face.underline_metrics(points)?)
            } else {
                None
            },
        });
    }
    if spans.is_empty() {
        return Ok((Vec::new(), false));
    }
    let mut remaining = context.font_limits.run_bytes.saturating_mul(16);
    let mut measure = |range: Range<usize>| -> Result<Measured> {
        remaining = remaining
            .checked_sub(range.len() + spans.len() + 1)
            .ok_or_else(|| Failure::input("text_wrap_work_limit"))?;
        let mut measured = Measured {
            line: Line {
                runs: Vec::new(),
                advance: 0.0,
                overflow: false,
            },
            spans: Vec::new(),
        };
        for (index, span) in spans.iter().enumerate() {
            let start = range.start.max(span.range.start);
            let end = range.end.min(span.range.end);
            if start >= end {
                continue;
            }
            let part = &text[start..end];
            let shaped =
                context.fonts.faces[span.font].shape_run(part, span.points, context.font_limits)?;
            let x = measured.line.advance;
            measured.line.advance += shaped.advance;
            measured.line.runs.push(crate::xlsx::text_layout::Run {
                text: part.into(),
                x,
                shaped,
            });
            measured.spans.push(index);
        }
        measured.line.overflow = measured.line.advance > content.width;
        Ok(measured)
    };
    let mut lines = Vec::new();
    let mut offset = 0;
    // 非 wrap の LF は既存の文字セルと同じく制御文字として拒否する。
    let paragraphs: Vec<&str> = if style.alignment.wrap {
        text.split('\n').collect()
    } else {
        vec![&text]
    };
    if paragraphs.len() > 4096 {
        return Err(Failure::input("text_line_limit"));
    }
    for paragraph in paragraphs {
        let end = offset + paragraph.len();
        let full = measure(offset..end)?;
        if !style.alignment.wrap || !full.line.overflow {
            lines.push(full);
        } else {
            if frame.baseline_step.is_none() {
                return Err(Failure::unsupported("cell_line_spacing_unresolved"));
            }
            let candidates = unicode_linebreak::linebreaks(paragraph)
                .map(|(end, opportunity)| {
                    if opportunity == unicode_linebreak::BreakOpportunity::Mandatory
                        && end != paragraph.len()
                    {
                        Err(Failure::unsupported("text_line_separator"))
                    } else {
                        Ok(offset + end)
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            let mut start = offset;
            while start < end {
                let mut best = None;
                for &candidate in &candidates {
                    if candidate <= start {
                        continue;
                    }
                    let next = measure(start..candidate)?;
                    if !next.line.overflow {
                        best = Some((candidate, next));
                    }
                }
                let (next, line) =
                    best.ok_or_else(|| Failure::unsupported("automatic_text_wrap"))?;
                lines.push(line);
                start = next;
                if lines.len() > 4096 {
                    return Err(Failure::input("text_line_limit"));
                }
            }
        }
        offset = end + 1;
        if lines.len() > 4096 {
            return Err(Failure::input("text_line_limit"));
        }
    }
    let indices: Vec<_> = lines
        .iter_mut()
        .map(|l| std::mem::take(&mut l.spans))
        .collect();
    let lines: Vec<_> = lines.into_iter().map(|l| l.line).collect();
    let horizontal = if style.alignment.horizontal == Horizontal::General {
        Horizontal::Left
    } else {
        style.alignment.horizontal
    };
    let positions = if lines.len() > 1 {
        position_lines(
            &lines,
            content,
            horizontal,
            style.alignment.vertical,
            ascent,
            descent,
            frame
                .baseline_step
                .ok_or_else(|| Failure::unsupported("cell_line_spacing_unresolved"))?,
        )?
    } else {
        vec![position_line(
            &lines[0],
            content,
            horizontal,
            style.alignment.vertical,
            ascent,
            descent,
        )?]
    };
    let mut result = Vec::new();
    for ((line, indices), position) in lines.into_iter().zip(indices).zip(positions) {
        for (run, index) in line.runs.into_iter().zip(indices) {
            let span = &spans[index];
            let x = position.x + run.x;
            if !x.is_finite() {
                return Err(Failure::input("invalid_text_rectangle"));
            }
            let underline = span.underline.and_then(|(offset, thickness)| {
                let left = x.max(frame.clip.x);
                let right = (x + run.shaped.advance).min(frame.clip.x + frame.clip.width);
                let top = (position.baseline + offset).max(frame.clip.y);
                let bottom =
                    (position.baseline + offset + thickness).min(frame.clip.y + frame.clip.height);
                (right > left && bottom > top).then_some(BoxRect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                })
            });
            result.push(Text {
                origin: (0, 0, 0),
                text: run.text,
                font: span.font,
                points: span.points,
                x,
                baseline: position.baseline,
                color: span.color,
                clip: frame.clip,
                underline,
            });
        }
    }
    Ok((result, substituted))
}
