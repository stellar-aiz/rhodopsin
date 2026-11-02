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
    text.chars()
        .flat_map(|ch| {
            if ('\u{2E80}'..='\u{2EFF}').contains(&ch) || ('\u{2F00}'..='\u{2FDF}').contains(&ch) {
                ch.to_string().nfkc().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect()
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
fn numbers(operands: &[Object], count: usize) -> Result<Vec<f64>> {
    if operands.len() != count {
        return Err("invalid_content_operands");
    }
    operands
        .iter()
        .map(|o| match o {
            Object::Integer(v) => Ok(*v as f64),
            Object::Real(v) => Ok(f64::from(*v)),
            _ => Err("invalid_content_operands"),
        })
        .collect()
}
fn rgb(values: &[f64]) -> Result<[u8; 3]> {
    let level = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    match values {
        [g] => Ok([level(*g); 3]),
        [r, g, b] => Ok([level(*r), level(*g), level(*b)]),
        _ => Err("color_space_unobserved"),
    }
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
    let mut shows = Vec::new();
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
                layout.rules.push(if horizontal {
                    Rule {
                        horizontal,
                        center: (top + bottom) / 2.0,
                        thickness: bottom - top,
                        from: left,
                        to: right,
                    }
                } else {
                    Rule {
                        horizontal,
                        center: (left + right) / 2.0,
                        thickness: right - left,
                        from: top,
                        to: bottom,
                    }
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
            "cm" => {
                let m = numbers(operands, 6)?;
                ctm = multiply(&[m[0], m[1], m[2], m[3], m[4], m[5]], &ctm);
            }
            "g" | "rg" | "sc" | "scn" => fill = rgb(&numbers(operands, operands.len())?)?,
            "G" | "RG" | "SC" | "SCN" => stroke = rgb(&numbers(operands, operands.len())?)?,
            "cs" | "CS" | "d" | "J" | "j" | "M" | "gs" | "ri" | "i" | "BT" | "ET" | "W" | "W*" => {
                if operation.operator == "BT" {
                    (tm, line, positioned) = (identity, identity, true);
                }
            }
            "w" => width = numbers(operands, 1)?[0],
            "re" => {
                let v = numbers(operands, 4)?;
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
                let v = numbers(operands, 2)?;
                path.push(vec![apply(&ctm, v[0], v[1])]);
            }
            "l" => {
                let v = numbers(operands, 2)?;
                path.last_mut()
                    .ok_or("invalid_path")?
                    .push(apply(&ctm, v[0], v[1]));
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
                size = numbers(&operands[1..], 1)?[0];
            }
            "Tm" => {
                let m = numbers(operands, 6)?;
                tm = [m[0], m[1], m[2], m[3], m[4], m[5]];
                (line, positioned) = (tm, true);
            }
            "Td" | "TD" => {
                let v = numbers(operands, 2)?;
                line = multiply(&[1.0, 0.0, 0.0, 1.0, v[0], v[1]], &line);
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
                shows.push(Run {
                    text: normalize_radicals(&text),
                    x: m[4],
                    y: height - m[5],
                    size: size * m[0],
                });
            }
            "'" | "\"" | "T*" | "TL" | "Do" | "BDC" | "BMC" | "sh" | "BI" => {
                return Err("content_operator_unobserved");
            }
            _ => {}
        }
    }
    layout.runs = shows
        .into_iter()
        .filter(|show| !show.text.trim().is_empty())
        .collect();
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
        for (forward, (from, to)) in [(true, (expected, actual)), (false, (actual, expected))] {
            for (order, rule) in from.rules.iter().enumerate() {
                let near: Vec<_> = to
                    .rules
                    .iter()
                    .filter(|r| {
                        r.horizontal == rule.horizontal
                            && (r.center - rule.center).abs() <= TOLERANCE
                            && (r.thickness - rule.thickness).abs() <= TOLERANCE
                    })
                    .collect();
                // 線の中心線を 0.5 pt おきに調べ、相手側のいずれかの線（向きを問わない）を
                // TOLERANCE だけ広げた矩形に入るか。交点では縦横どちらの線で覆われてもよい。
                let inside = |along: f64, across: f64| {
                    to.rules.iter().any(|r| {
                        let (x, y) = if rule.horizontal {
                            (along, across)
                        } else {
                            (across, along)
                        };
                        let half = r.thickness / 2.0 + TOLERANCE;
                        let (x0, x1, y0, y1) = if r.horizontal {
                            (
                                r.from - TOLERANCE,
                                r.to + TOLERANCE,
                                r.center - half,
                                r.center + half,
                            )
                        } else {
                            (
                                r.center - half,
                                r.center + half,
                                r.from - TOLERANCE,
                                r.to + TOLERANCE,
                            )
                        };
                        (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
                    })
                };
                let steps = ((rule.to - rule.from) / 0.5).ceil().max(1.0) as usize;
                let covered = (0..=steps)
                    .all(|i| inside((rule.from + i as f64 * 0.5).min(rule.to), rule.center));
                if forward {
                    result.rules_compared += 1;
                }
                if !covered {
                    result.uncovered_rules += 1;
                    result.exceeded.push(Exceeded {
                        page,
                        kind: if forward {
                            "rule_missing"
                        } else {
                            "rule_extra"
                        },
                        index: order,
                        delta_pt: 0.0,
                    });
                    continue;
                }
                if forward {
                    let delta = near
                        .iter()
                        .map(|r| {
                            (r.center - rule.center)
                                .abs()
                                .max((r.thickness - rule.thickness).abs())
                        })
                        .fold(f64::MAX, f64::min);
                    // near は中心と太さが TOLERANCE 以内の線だけなので、空なら対応する線が無い。
                    if near.is_empty() {
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
            }
        }
        for (order, (color, area)) in expected.areas.iter().enumerate() {
            let near = actual
                .areas
                .iter()
                .find(|(c, _)| c.iter().zip(color.iter()).all(|(a, b)| a.abs_diff(*b) <= 1));
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
            .filter(|c| {
                !expected
                    .areas
                    .keys()
                    .any(|e| e.iter().zip(c.iter()).all(|(a, b)| a.abs_diff(*b) <= 1))
            })
            .count();
    }
    result.matches = reference.len() == candidate.len()
        && result.exceeded.is_empty()
        && result.unmatched_reference_runs == 0
        && result.unmatched_candidate_runs == 0
        && result.unmatched_area_colors == 0;
    result
}
