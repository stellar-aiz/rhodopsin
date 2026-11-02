//! 開発専用。ページ上の文字 run の原点、罫線、塗りの位置を取り出して比較する。
//! 文字列そのものは出力しない。比較の定義は docs/xlsx-validation.md に従う。
use lopdf::{Document, Object, ObjectId, content::Content};
use std::collections::BTreeMap;

pub type Result<T> = std::result::Result<T, &'static str>;
/// 位置の許容差（ポイント）。比較の前に固定した判断値で、差を隠すために広げない。
pub const TOLERANCE: f64 = 1.0;
/// 罫線とみなす矩形の短辺の上限（ポイント）。
const RULE_THICKNESS: f64 = 4.0;

#[derive(Clone, Debug)]
pub struct Run {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub size: f64,
}
#[derive(Clone, Debug)]
pub struct Rule {
    pub horizontal: bool,
    pub center: f64,
    pub thickness: f64,
    pub from: f64,
    pub to: f64,
}
#[derive(Clone, Debug, Default)]
pub struct PageLayout {
    /// 空白だけでない文字表示命令。内容の順。
    pub runs: Vec<Run>,
    pub rules: Vec<Rule>,
    /// 塗りの色（0〜255 に丸めた RGB）ごとの外接矩形。白は紙と区別しない。
    pub areas: BTreeMap<[u8; 3], [f64; 4]>,
}

/// Quartz は共有字形の ToUnicode に康熙部首・部首補助を割り当てる（例: 見 → U+2F92）。
/// その二つのブロックだけを互換分解して統合漢字へ戻し、他の文字は変えない。
pub fn normalize_radicals(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut result = String::with_capacity(text.len());
    for ch in text.chars() {
        if ('\u{2E80}'..='\u{2EFF}').contains(&ch) || ('\u{2F00}'..='\u{2FDF}').contains(&ch) {
            result.extend(std::iter::once(ch).nfkc());
        } else {
            result.push(ch);
        }
    }
    result
}

type Matrix = [f64; 6];
fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}
fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}
fn number(operand: &Object) -> Result<f64> {
    match operand {
        Object::Integer(v) => Ok(*v as f64),
        Object::Real(v) => Ok(f64::from(*v)),
        _ => Err("invalid_content_operands"),
    }
}
fn numbers<const N: usize>(operands: &[Object]) -> Result<[f64; N]> {
    let operands: &[Object; N] = operands
        .try_into()
        .map_err(|_| "invalid_content_operands")?;
    let mut values = [0.0; N];
    for (value, operand) in values.iter_mut().zip(operands) {
        *value = number(operand)?;
    }
    Ok(values)
}
/// 灰色または RGB の塗り・線の色を 0〜255 に丸める。
fn rgb(operands: &[Object]) -> Result<[u8; 3]> {
    let level = |o: &Object| number(o).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    match operands {
        [g] => Ok([level(g)?; 3]),
        [r, g, b] => Ok([level(r)?, level(g)?, level(b)?]),
        _ => Err("color_space_unobserved"),
    }
}
/// 0〜255 の各成分が 1 以内なら同じ色とみなす（色空間の丸めの違い）。
fn same_color(a: &[u8; 3], b: &[u8; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 1)
}

/// 一ページの内容を読む。回転・傾き、位置を確定できない文字表示、未対応の色空間は未観測にする。
pub fn read(doc: &Document, page: ObjectId, height: f64) -> Result<PageLayout> {
    let data = doc
        .get_page_content(page)
        .map_err(|_| "invalid_page_content")?;
    let content = Content::decode(&data).map_err(|_| "invalid_page_content")?;
    let fonts = doc.get_page_fonts(page).map_err(|_| "invalid_page_fonts")?;
    let mut encodings = BTreeMap::new();
    for (name, font) in &fonts {
        encodings.insert(
            name.clone(),
            font.get_font_encoding(doc)
                .map_err(|_| "text_font_unresolved")?,
        );
    }
    let mut layout = PageLayout::default();
    let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut ctm = identity;
    let mut stack = Vec::new();
    let (mut fill, mut stroke, mut width) = ([0u8; 3], [0u8; 3], 1.0);
    let (mut tm, mut line, mut positioned) = (identity, identity, false);
    let (mut font, mut size): (Option<Vec<u8>>, f64) = (None, 0.0);
    let mut path: Vec<Vec<(f64, f64)>> = Vec::new();
    let push_rect =
        |layout: &mut PageLayout, color: [u8; 3], x0: f64, y0: f64, x1: f64, y1: f64| {
            let (left, right, top, bottom) = (
                x0.min(x1),
                x0.max(x1),
                height - y0.max(y1),
                height - y0.min(y1),
            );
            if (right - left).min(bottom - top) <= RULE_THICKNESS {
                let horizontal = right - left > bottom - top;
                let (center, thickness, from, to) = if horizontal {
                    ((top + bottom) / 2.0, bottom - top, left, right)
                } else {
                    ((left + right) / 2.0, right - left, top, bottom)
                };
                layout.rules.push(Rule {
                    horizontal,
                    center,
                    thickness,
                    from,
                    to,
                });
            } else if color != [255; 3] {
                let area =
                    layout
                        .areas
                        .entry(color)
                        .or_insert([f64::MAX, f64::MAX, f64::MIN, f64::MIN]);
                *area = [
                    area[0].min(left),
                    area[1].min(top),
                    area[2].max(right),
                    area[3].max(bottom),
                ];
            }
        };
    for operation in &content.operations {
        let operands = &operation.operands;
        match operation.operator.as_str() {
            "q" => stack.push((ctm, fill, stroke, width)),
            "Q" => (ctm, fill, stroke, width) = stack.pop().ok_or("unbalanced_graphics_state")?,
            "cm" => ctm = multiply(&numbers(operands)?, &ctm),
            "g" | "rg" | "sc" | "scn" => fill = rgb(operands)?,
            "G" | "RG" | "SC" | "SCN" => stroke = rgb(operands)?,
            "BT" => (tm, line, positioned) = (identity, identity, true),
            "w" => [width] = numbers(operands)?,
            "re" => {
                let v: [f64; 4] = numbers(operands)?;
                path.push(
                    vec![
                        (v[0], v[1]),
                        (v[0] + v[2], v[1]),
                        (v[0] + v[2], v[1] + v[3]),
                        (v[0], v[1] + v[3]),
                    ]
                    .into_iter()
                    .map(|(x, y)| apply(&ctm, x, y))
                    .collect(),
                );
            }
            "m" => {
                let [x, y] = numbers(operands)?;
                path.push(vec![apply(&ctm, x, y)]);
            }
            "l" => {
                let [x, y] = numbers(operands)?;
                path.last_mut()
                    .ok_or("invalid_path")?
                    .push(apply(&ctm, x, y));
            }
            "h" => {}
            "f" | "f*" | "F" => {
                for points in path.drain(..) {
                    let xs = points.iter().map(|p| p.0);
                    let ys = points.iter().map(|p| p.1);
                    let (x0, x1) = (
                        xs.clone().fold(f64::MAX, f64::min),
                        xs.fold(f64::MIN, f64::max),
                    );
                    let (y0, y1) = (
                        ys.clone().fold(f64::MAX, f64::min),
                        ys.fold(f64::MIN, f64::max),
                    );
                    push_rect(&mut layout, fill, x0, y0, x1, y1);
                }
            }
            "S" => {
                if ctm[1] != 0.0 || ctm[2] != 0.0 || ctm[0] != ctm[3] {
                    return Err("transformed_stroke_unobserved");
                }
                let half = width * ctm[0].abs() / 2.0;
                for points in path.drain(..) {
                    for pair in points.windows(2) {
                        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
                        if y0 == y1 {
                            push_rect(&mut layout, stroke, x0, y0 - half, x1, y0 + half);
                        } else if x0 == x1 {
                            push_rect(&mut layout, stroke, x0 - half, y0, x0 + half, y1);
                        } else {
                            return Err("diagonal_stroke_unobserved");
                        }
                    }
                }
            }
            "n" => path.clear(),
            "Tf" => {
                let name = operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .ok_or("invalid_text_font")?;
                if !encodings.contains_key(name) {
                    return Err("text_font_unresolved");
                }
                font = Some(name.to_vec());
                [size] = numbers(&operands[1..])?;
            }
            "Tm" => {
                tm = numbers(operands)?;
                (line, positioned) = (tm, true);
            }
            "Td" | "TD" => {
                let [x, y] = numbers(operands)?;
                line = multiply(&[1.0, 0.0, 0.0, 1.0, x, y], &line);
                (tm, positioned) = (line, true);
            }
            "Tj" | "TJ" => {
                if !positioned {
                    return Err("text_position_unobserved");
                }
                positioned = false;
                let encoding = encodings
                    .get(font.as_deref().ok_or("text_font_state_unobserved")?)
                    .ok_or("text_font_unresolved")?;
                let strings: Vec<&[u8]> = match operands.as_slice() {
                    [Object::String(bytes, _)] => vec![bytes.as_slice()],
                    [Object::Array(items)] => items
                        .iter()
                        .filter_map(|item| match item {
                            Object::String(bytes, _) => Some(bytes.as_slice()),
                            _ => None,
                        })
                        .collect(),
                    _ => return Err("invalid_content_operands"),
                };
                let mut text = String::new();
                for bytes in strings {
                    text.push_str(
                        &Document::decode_text(encoding, bytes)
                            .map_err(|_| "pdf_text_extraction_failed")?,
                    );
                }
                let m = multiply(&tm, &ctm);
                if m[1] != 0.0 || m[2] != 0.0 || m[0] <= 0.0 || m[0] != m[3] {
                    return Err("transformed_text_unobserved");
                }
                if !text.trim().is_empty() {
                    layout.runs.push(Run {
                        text: normalize_radicals(&text),
                        x: m[4],
                        y: height - m[5],
                        size: size * m[0],
                    });
                }
            }
            "'" | "\"" | "T*" | "TL" | "Do" | "BDC" | "BMC" | "sh" | "BI" => {
                return Err("content_operator_unobserved");
            }
            _ => {}
        }
    }
    Ok(layout)
}

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct Exceeded {
    pub page: usize,
    pub kind: &'static str,
    /// 正解側の出現順（0 起点）。文字列は出力しない。
    pub index: usize,
    pub delta_pt: f64,
}
#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct Comparison {
    pub matches: bool,
    pub runs_compared: usize,
    pub unmatched_reference_runs: usize,
    pub unmatched_candidate_runs: usize,
    pub max_text_delta_pt: f64,
    pub rules_compared: usize,
    pub uncovered_rules: usize,
    pub max_rule_delta_pt: f64,
    pub area_colors_compared: usize,
    pub unmatched_area_colors: usize,
    pub max_area_delta_pt: f64,
    pub exceeded: Vec<Exceeded>,
}

/// 線の中心線を 0.5 pt おきに調べ、相手側のいずれかの線（向きを問わない）を TOLERANCE だけ
/// 広げた矩形に入るか。交点では縦横どちらの線で覆われてもよい。
fn covered(rule: &Rule, others: &[Rule]) -> bool {
    let inside = |along: f64| {
        let (x, y) = if rule.horizontal {
            (along, rule.center)
        } else {
            (rule.center, along)
        };
        others.iter().any(|r| {
            let half = r.thickness / 2.0 + TOLERANCE;
            let (across, along) = if r.horizontal { (y, x) } else { (x, y) };
            (r.center - half..=r.center + half).contains(&across)
                && (r.from - TOLERANCE..=r.to + TOLERANCE).contains(&along)
        })
    };
    let steps = ((rule.to - rule.from) / 0.5).ceil().max(1.0) as usize;
    (0..=steps).all(|i| inside((rule.from + i as f64 * 0.5).min(rule.to)))
}

fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// 正解の各要素に最も近い候補を対応付け、位置差を TOLERANCE と比べる。
pub fn compare(reference: &[PageLayout], candidate: &[PageLayout]) -> Comparison {
    let mut result = Comparison::default();
    for (index, (expected, actual)) in reference.iter().zip(candidate).enumerate() {
        let page = index + 1;
        // 正解の各文字表示に、同じベースライン付近で内容順に続く候補の文字表示の並びを
        // 対応付ける。Excel はセルごと、rhodopsin は字形ごとに書くため、分割の違いを吸収する。
        // 空白は描画されないことがあるので、空白を除いた文字列で照合する。
        let compact = |s: &str| s.split_whitespace().collect::<String>();
        let mut used = vec![false; actual.runs.len()];
        for (order, run) in expected.runs.iter().enumerate() {
            let target = compact(&run.text);
            let mut best: Option<(usize, usize, f64)> = None;
            for start in 0..actual.runs.len() {
                let first = &actual.runs[start];
                if used[start] || (first.y - run.y).abs() > 2.0 * TOLERANCE {
                    continue;
                }
                let mut text = String::new();
                for (end, (next, taken)) in actual.runs.iter().zip(&used).enumerate().skip(start) {
                    if *taken || (next.y - first.y).abs() > 0.01 {
                        break;
                    }
                    text.push_str(&compact(&next.text));
                    if !target.starts_with(&text) {
                        break;
                    }
                    if text == target {
                        let distance = (first.x - run.x).abs() + (first.y - run.y).abs();
                        if best.is_none_or(|(_, _, d)| distance < d) {
                            best = Some((start, end, distance));
                        }
                        break;
                    }
                }
            }
            let Some((start, end, _)) = best else {
                result.unmatched_reference_runs += 1;
                continue;
            };
            used[start..=end].iter_mut().for_each(|u| *u = true);
            result.runs_compared += 1;
            let found = &actual.runs[start];
            let delta = (found.x - run.x)
                .abs()
                .max((found.y - run.y).abs())
                .max((found.size - run.size).abs());
            result.max_text_delta_pt = result.max_text_delta_pt.max(round(delta));
            if delta > TOLERANCE {
                result.exceeded.push(Exceeded {
                    page,
                    kind: "text",
                    index: order,
                    delta_pt: round(delta),
                });
            }
        }
        result.unmatched_candidate_runs += used.iter().filter(|u| !**u).count();
        for (order, rule) in expected.rules.iter().enumerate() {
            result.rules_compared += 1;
            if !covered(rule, &actual.rules) {
                result.uncovered_rules += 1;
                result.exceeded.push(Exceeded {
                    page,
                    kind: "rule_missing",
                    index: order,
                    delta_pt: 0.0,
                });
                continue;
            }
            // 中心と太さが TOLERANCE 以内の線が無ければ、覆われていても対応する線が無い。
            let delta = actual
                .rules
                .iter()
                .filter(|r| r.horizontal == rule.horizontal)
                .map(|r| {
                    (r.center - rule.center)
                        .abs()
                        .max((r.thickness - rule.thickness).abs())
                })
                .fold(f64::MAX, f64::min);
            if delta > TOLERANCE {
                result.exceeded.push(Exceeded {
                    page,
                    kind: "rule_unmatched",
                    index: order,
                    delta_pt: 0.0,
                });
            } else {
                result.max_rule_delta_pt = result.max_rule_delta_pt.max(round(delta));
            }
        }
        for (order, rule) in actual.rules.iter().enumerate() {
            if !covered(rule, &expected.rules) {
                result.uncovered_rules += 1;
                result.exceeded.push(Exceeded {
                    page,
                    kind: "rule_extra",
                    index: order,
                    delta_pt: 0.0,
                });
            }
        }
        for (order, (color, area)) in expected.areas.iter().enumerate() {
            let near = actual.areas.iter().find(|(c, _)| same_color(c, color));
            let Some((_, found)) = near else {
                result.unmatched_area_colors += 1;
                result.exceeded.push(Exceeded {
                    page,
                    kind: "area_missing",
                    index: order,
                    delta_pt: 0.0,
                });
                continue;
            };
            result.area_colors_compared += 1;
            let delta = area
                .iter()
                .zip(found)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            result.max_area_delta_pt = result.max_area_delta_pt.max(round(delta));
            if delta > TOLERANCE {
                result.exceeded.push(Exceeded {
                    page,
                    kind: "area",
                    index: order,
                    delta_pt: round(delta),
                });
            }
        }
        result.unmatched_area_colors += actual
            .areas
            .keys()
            .filter(|c| !expected.areas.keys().any(|e| same_color(e, c)))
            .count();
    }
    result.matches = reference.len() == candidate.len()
        && result.exceeded.is_empty()
        && result.unmatched_reference_runs == 0
        && result.unmatched_candidate_runs == 0
        && result.unmatched_area_colors == 0;
    result
}
