//! 印刷されるセルの罫線候補。競合解決・線幅・交点の描画は別途検証する。
use crate::xlsx::{
    diagnostic::{Failure, Result},
    layout::Page,
    placement::range_on_page,
    styles::Styles,
    workbook::{Sheet, cell_address},
};
use std::collections::{BTreeMap, BTreeSet};
mod merged;
mod paint;
pub use paint::{Stroke, paint};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// 一本の辺の指定。共有辺の両側に指定があれば二件を保持する。
/// これ自体は PDF に描ける確定済みの罫線ではない。
#[derive(Clone, Debug)]
pub struct Edge {
    pub sheet_index: usize,
    pub cell: String,
    pub side: Side,
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub style: crate::xlsx::styles::LineStyle,
    pub color: [u8; 3],
}

/// 同じ線分・線種・色の候補を一本にする。隣接する線分は連結しない。
/// 破線の位相や交点の描画を変えないため、ここでは同一線分だけを扱う。
/// 同じ色の細線と中線・太線が共有辺で重なる場合は、Excel for Mac 16.105 の PDF と同じく
/// 太い方を残す（docs/history/20261102_xlsx-excel-mac-layout.md）。
/// それ以外の異なる指定の共有辺と部分的な重なりは、優先規則の検証まで位置付きで拒否する。
/// 一ページの候補を渡す。limit は入力候補数の上限。
pub fn coalesce(edges: &[Edge], limit: usize) -> Result<Vec<Edge>> {
    if edges.len() > limit {
        return Err(Failure::input("page_border_work_limit"));
    }
    let mut ordered = Vec::with_capacity(edges.len());
    for edge in edges {
        // 公開 API へ外部構築された候補が渡っても、自由文を診断へコピーしない。
        if edge.sheet_index == 0 || cell_address(&edge.cell).is_none() {
            return Err(Failure::input("invalid_border_location"));
        }
        if edges
            .first()
            .is_some_and(|first| first.sheet_index != edge.sheet_index)
        {
            return Err(Failure::input("page_sheet_mismatch"));
        }
        if edge
            .from
            .iter()
            .chain(&edge.to)
            .any(|value| !value.is_finite())
        {
            return Err(Failure::input("invalid_border_geometry")
                .at(edge.sheet_index, Some(edge.cell.clone())));
        }
        let vertical = matches!(edge.side, Side::Left | Side::Right);
        let (fixed, along) = if vertical { (0, 1) } else { (1, 0) };
        if edge.from[fixed] != edge.to[fixed] || edge.from[along] >= edge.to[along] {
            return Err(Failure::input("invalid_border_geometry")
                .at(edge.sheet_index, Some(edge.cell.clone())));
        }
        // -0 と +0 は同じ紙面座標。
        let canonical = |value: f64| if value == 0.0 { 0.0 } else { value };
        ordered.push((
            vertical,
            canonical(edge.from[fixed]),
            canonical(edge.from[along]),
            canonical(edge.to[along]),
            edge,
        ));
    }
    ordered.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.total_cmp(&b.3))
    });
    let mut result = Vec::new();
    let mut prior: Option<(bool, f64, f64, f64, &Edge)> = None;
    for (vertical, fixed, start, end, edge) in ordered {
        if let Some((pv, pf, ps, pe, previous)) = prior
            && vertical == pv
            && fixed == pf
            && start < pe
        {
            let same_segment = start == ps && end == pe;
            if same_segment && edge.style == previous.style && edge.color == previous.color {
                continue;
            }
            if same_segment && edge.color == previous.color {
                use crate::xlsx::styles::LineStyle::{Medium, Thick, Thin};
                match (previous.style, edge.style) {
                    (Medium | Thick, Thin) => continue,
                    (Thin, Medium | Thick) => {
                        result.pop();
                        result.push(edge.clone());
                        prior = Some((vertical, fixed, start, end, edge));
                        continue;
                    }
                    _ => {}
                }
            }
            return Err(Failure::unsupported(if same_segment {
                "border_conflict"
            } else {
                "border_partial_overlap"
            })
            .at(edge.sheet_index, Some(edge.cell.clone())));
        }
        result.push(edge.clone());
        prior = Some((vertical, fixed, start, end, edge));
    }
    Ok(result)
}

/// セルと結合外周の辺をページ上端原点のポイント座標へ移す。
/// 行列既定書式・空白セル・繰り返し行列も対象にする。
/// 結合は左上書式を外周へ広げ、ページ切断位置には辺を追加しない。
/// 外周セルの指定が左上書式と矛盾する場合は停止する。
/// limit は走査作業と返却する辺の合計を制限する。
pub fn edges(
    sheet: &Sheet,
    page: &Page,
    styles: &Styles,
    colors: &crate::xlsx::color::ColorContext<'_>,
    limit: usize,
) -> Result<Vec<Edge>> {
    if sheet.index != page.sheet_index {
        return Err(Failure::input("page_sheet_mismatch"));
    }
    if sheet.hidden {
        return Ok(Vec::new());
    }
    let mut remaining = limit;
    let mut charge = |count: usize| -> Result<()> {
        remaining = remaining
            .checked_sub(count)
            .ok_or_else(|| Failure::input("page_border_work_limit"))?;
        Ok(())
    };
    let axis_work = page
        .columns
        .len()
        .checked_add(page.rows.len())
        .ok_or_else(|| Failure::input("page_border_work_limit"))?;
    charge(axis_work)?;
    range_on_page(
        page,
        crate::xlsx::range::CellRange {
            first_column: 1,
            last_column: crate::xlsx::range::MAX_COLUMN,
            first_row: 1,
            last_row: crate::xlsx::range::MAX_ROW,
        },
    )?;
    let (Some(first), Some(last)) = (page.columns.first(), page.columns.last()) else {
        return Err(Failure::input("empty_page_axis"));
    };
    if page.rows.is_empty() {
        return Err(Failure::input("empty_page_axis"));
    }
    let mut cells = BTreeMap::new();
    for cell in &sheet.cells {
        charge(1)?;
        let (column, row) =
            cell_address(&cell.address).ok_or_else(|| Failure::input("invalid_cell_address"))?;
        if cells.insert((row, column), cell).is_some() {
            return Err(
                Failure::input("duplicate_cell").at(sheet.index, Some(cell.address.clone()))
            );
        }
    }
    let (mut result, merges) = merged::edges(sheet, page, styles, colors, &cells, &mut charge)?;
    for row in page.rows.iter() {
        charge(sheet.grid.columns.len())?;
        let spans = sheet.grid.style_spans(row.index, first.index, last.index)?;
        let mut span_index = 0;
        for column in page.columns.iter() {
            charge(1)?;
            charge(merges.len())?;
            if merges.iter().any(|range| {
                range.first_column <= column.index
                    && column.index <= range.last_column
                    && range.first_row <= row.index
                    && row.index <= range.last_row
            }) {
                continue;
            }
            while spans[span_index].last < column.index {
                charge(1)?;
                span_index += 1;
            }
            let style = cells
                .get(&(row.index, column.index))
                .map_or(spans[span_index].style, |cell| cell.style);
            let address = crate::xlsx::drawing::Marker {
                column: column.index,
                row: row.index,
                column_offset: 0,
                row_offset: 0,
            }
            .address();
            let at = |error: Failure| error.at(sheet.index, Some(address.clone()));
            let style_ref = styles.validate_cell(style).map_err(at)?;
            let border = styles.borders[style_ref.border]
                .as_ref()
                .map_err(|e| at(e.clone()))?;
            let x = page.origin_x + column.offset * page.scale;
            let y = page.origin_y + row.offset * page.scale;
            let right = page.origin_x + (column.offset + column.size) * page.scale;
            let bottom = page.origin_y + (row.offset + row.size) * page.scale;
            if [x, y, right, bottom].iter().any(|value| !value.is_finite()) {
                return Err(at(Failure::input("invalid_page_transform")));
            }
            for (side, line, from, to) in [
                (Side::Left, &border.left, [x, y], [x, bottom]),
                (Side::Right, &border.right, [right, y], [right, bottom]),
                (Side::Top, &border.top, [x, y], [right, y]),
                (Side::Bottom, &border.bottom, [x, bottom], [right, bottom]),
            ] {
                let Some(line) = line else { continue };
                charge(1).map_err(at)?;
                result.push(Edge {
                    sheet_index: sheet.index,
                    cell: address.clone(),
                    side,
                    from,
                    to,
                    style: line.style,
                    color: line.color.resolve(colors).map_err(at)?,
                });
            }
        }
    }
    Ok(result)
}

pub(crate) fn reject_used(sheet: &Sheet, page: &Page, styles: &Styles, limit: usize) -> Result<()> {
    let mut remaining = limit;
    let mut charge = |count: usize| {
        remaining = remaining
            .checked_sub(count)
            .ok_or_else(|| Failure::input("page_border_work_limit"))?;
        Ok::<_, Failure>(())
    };
    charge(page.columns.len())?;
    charge(page.rows.len())?;
    let columns: BTreeSet<_> = page.columns.iter().map(|column| column.index).collect();
    let (&first, &last) = columns
        .first()
        .zip(columns.last())
        .ok_or_else(|| Failure::input("empty_page_axis"))?;
    let mut cells = BTreeMap::new();
    for cell in &sheet.cells {
        charge(1)?;
        let (column, row) =
            cell_address(&cell.address).ok_or_else(|| Failure::input("invalid_cell_address"))?;
        if cells.insert((row, column), cell).is_some() {
            return Err(
                Failure::input("duplicate_cell").at(sheet.index, Some(cell.address.clone()))
            );
        }
    }
    let mut checked = BTreeSet::new();
    let mut check = |style: usize, column: u32, row: u32| -> Result<()> {
        if checked.contains(&style) {
            return Ok(());
        }
        let address = crate::xlsx::drawing::Marker {
            column,
            row,
            column_offset: 0,
            row_offset: 0,
        }
        .address();
        let at = |error: Failure| error.at(sheet.index, Some(address.clone()));
        let style_ref = styles.validate_cell(style).map_err(at)?;
        let border = styles.borders[style_ref.border]
            .as_ref()
            .map_err(|error| at(error.clone()))?;
        if border.left.is_some()
            || border.right.is_some()
            || border.top.is_some()
            || border.bottom.is_some()
        {
            return Err(at(Failure::unsupported("page_border_layout_unimplemented")));
        }
        checked.insert(style);
        Ok(())
    };
    for row in page.rows.iter() {
        charge(sheet.grid.columns.len() + 1)?;
        for span in sheet.grid.style_spans(row.index, first, last)? {
            charge(1)?;
            let mut cursor = span.first;
            for (&(_, column), cell) in
                cells.range((row.index, span.first)..=(row.index, span.last))
            {
                charge(1)?;
                if cursor < column
                    && let Some(&visible) = columns.range(cursor..column).next()
                {
                    check(span.style, visible, row.index)?;
                }
                if columns.contains(&column) {
                    check(cell.style, column, row.index)?;
                }
                cursor = column + 1;
            }
            if cursor <= span.last
                && let Some(&visible) = columns.range(cursor..=span.last).next()
            {
                check(span.style, visible, row.index)?;
            }
        }
    }
    // 結合左上セルがページ外でも、その罫線がページ内へ及ぶ可能性がある。
    // 内部セルの罫線も上記で保守的に検出し、結合辺の規則を推測しない。
    for range in &sheet.grid.merges {
        charge(1)?;
        charge(page.columns.len())?;
        charge(page.rows.len())?;
        if range_on_page(page, *range)?.is_empty() {
            continue;
        }
        let style = if let Some(cell) = cells.get(&(range.first_row, range.first_column)) {
            cell.style
        } else {
            charge(sheet.grid.columns.len() + 1)?;
            sheet
                .grid
                .style_spans(range.first_row, range.first_column, range.first_column)?[0]
                .style
        };
        check(style, range.first_column, range.first_row)?;
    }
    Ok(())
}
