//! 明示された描画規則で罫線候補をPDFの線へ変換する。
use super::{Edge, Side, coalesce};
use crate::xlsx::{
    diagnostic::{Failure, Result},
    pdf::graphics::Paint,
    styles::LineStyle,
};

/// 値はすべて印刷倍率適用後のポイント。Excelの既定値ではない。
#[derive(Clone, Debug)]
pub struct Stroke {
    pub width: f64,
    pub dash: Vec<f64>,
    /// 二重線の中心間隔。Double以外はNone、Doubleでは幅より大きい値が必要。
    pub separation: Option<f64>,
}

/// 共有辺を整理し、各辺を欠落させず線に変換する。
/// 線端はPDF既定のbutt、破線位相は0。交点を延長・短縮しない。
/// 異なる共有辺・部分重複・斜め鎖線の規則は未確定のため拒否する。
pub fn paint(
    edges: &[Edge],
    work_limit: usize,
    item_limit: usize,
    mut policy: impl FnMut(&Edge) -> Result<Stroke>,
) -> Result<Vec<Paint<'static>>> {
    let edges = coalesce(edges, work_limit)?;
    let mut result = Vec::new();
    for edge in edges {
        let at = |error: Failure| error.at(edge.sheet_index, Some(edge.cell.clone()));
        if edge.style == LineStyle::SlantDashDot {
            return Err(at(Failure::unsupported("slanted_border_unresolved")));
        }
        let stroke = policy(&edge).map_err(at)?;
        let valid = |n: f64| n.is_finite() && n > 0.0 && n <= 1_000_000.0;
        if !valid(stroke.width)
            || stroke.dash.len() > 32
            || stroke.dash.iter().any(|n| !valid(*n))
            || (edge.style == LineStyle::Double) != stroke.separation.is_some()
            || stroke
                .separation
                .is_some_and(|n| !valid(n) || n <= stroke.width)
            || stroke.separation.is_some() && !stroke.dash.is_empty()
        {
            return Err(at(Failure::input("invalid_border_stroke")));
        }
        let offsets = match stroke.separation {
            Some(distance) => vec![-distance / 2.0, distance / 2.0],
            None => vec![0.0],
        };
        if result
            .len()
            .checked_add(offsets.len())
            .is_none_or(|n| n > item_limit)
        {
            return Err(at(Failure::input("page_scene_item_limit")));
        }
        for offset in offsets {
            let mut from = edge.from;
            let mut to = edge.to;
            let perpendicular = if matches!(edge.side, Side::Left | Side::Right) {
                0
            } else {
                1
            };
            from[perpendicular] += offset;
            to[perpendicular] += offset;
            if from
                .iter()
                .chain(&to)
                .any(|n| !n.is_finite() || n.abs() > 1_000_000.0)
            {
                return Err(at(Failure::input("invalid_border_geometry")));
            }
            result.push(Paint::Line {
                from,
                to,
                width: stroke.width,
                color: edge.color,
                dash: stroke.dash.clone(),
            });
        }
    }
    Ok(result)
}
