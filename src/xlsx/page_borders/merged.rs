//! 結合範囲の外周。左上書式と矛盾する外周指定を推測で上書きしない。
use super::*;
use crate::xlsx::{color::ColorContext, range::CellRange, workbook::Cell};

fn visible_bounds(sheet: &Sheet, range: CellRange) -> Option<CellRange> {
    let mut bounds = range;
    for column in &sheet.grid.columns {
        if column.hidden
            && column.first <= bounds.first_column
            && bounds.first_column <= column.last
        {
            bounds.first_column = column.last.saturating_add(1);
        }
    }
    for column in sheet.grid.columns.iter().rev() {
        if column.hidden && column.first <= bounds.last_column && bounds.last_column <= column.last
        {
            bounds.last_column = column.first.saturating_sub(1);
        }
    }
    if sheet.grid.default_rows_hidden {
        let mut visible = sheet
            .grid
            .rows
            .range(range.first_row..=range.last_row)
            .filter(|(_, row)| !row.hidden)
            .map(|(&index, _)| index);
        bounds.first_row = visible.next()?;
        bounds.last_row = visible.next_back().unwrap_or(bounds.first_row);
    } else {
        for (&index, row) in sheet.grid.rows.range(range.first_row..=range.last_row) {
            if index != bounds.first_row || !row.hidden {
                break;
            }
            bounds.first_row += 1;
        }
        for (&index, row) in sheet
            .grid
            .rows
            .range(range.first_row..=range.last_row)
            .rev()
        {
            if index != bounds.last_row || !row.hidden {
                break;
            }
            bounds.last_row -= 1;
        }
    }
    (bounds.first_column <= bounds.last_column && bounds.first_row <= bounds.last_row)
        .then_some(bounds)
}

pub(super) fn edges(
    sheet: &Sheet,
    page: &Page,
    styles: &Styles,
    colors: &ColorContext<'_>,
    cells: &BTreeMap<(u32, u32), &Cell>,
    charge: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<(Vec<Edge>, Vec<CellRange>)> {
    let mut result = Vec::new();
    let mut ranges = Vec::new();
    for (index, range) in sheet.grid.merges.iter().enumerate() {
        charge(1)?;
        for prior in &sheet.grid.merges[..index] {
            charge(1)?;
            if range.intersects(*prior) {
                return Err(Failure::input("overlapping_merged_cells").at(sheet.index, None));
            }
        }
        charge(page.columns.len())?;
        charge(page.rows.len())?;
        let placements = range_on_page(page, *range)?;
        if placements.is_empty() {
            continue;
        }
        let address = crate::xlsx::drawing::Marker {
            column: range.first_column,
            row: range.first_row,
            column_offset: 0,
            row_offset: 0,
        }
        .address();
        let at = |error: Failure| error.at(sheet.index, Some(address.clone()));
        let mut border_at = |column, row| {
            charge(1)?;
            charge(sheet.grid.columns.len())?;
            let style = cells
                .get(&(row, column))
                .map_or_else(|| sheet.grid.style_for(column, row), |cell| cell.style);
            let style = styles.validate_cell(style).map_err(at)?;
            styles.borders[style.border]
                .as_ref()
                .map_err(|e| at(e.clone()))
        };
        let border = border_at(range.first_column, range.first_row)?;
        let equal = |candidate: &Option<crate::xlsx::styles::BorderLine>,
                     base: &Option<crate::xlsx::styles::BorderLine>|
         -> Result<()> {
            if let Some(candidate) = candidate {
                let Some(base) = base else {
                    return Err(at(Failure::unsupported("merged_border_style_conflict")));
                };
                if candidate.style != base.style
                    || candidate.color.resolve(colors).map_err(at)?
                        != base.color.resolve(colors).map_err(at)?
                {
                    return Err(at(Failure::unsupported("merged_border_style_conflict")));
                }
            }
            Ok(())
        };
        // 空白セルの行列書式も含めて外周を確認する。内部の格子辺は描かない。
        for column in range.first_column..=range.last_column {
            equal(&border_at(column, range.first_row)?.top, &border.top)?;
            equal(&border_at(column, range.last_row)?.bottom, &border.bottom)?;
        }
        for row in range.first_row..=range.last_row {
            equal(&border_at(range.first_column, row)?.left, &border.left)?;
            equal(&border_at(range.last_column, row)?.right, &border.right)?;
        }
        charge(sheet.grid.columns.len())?;
        charge(sheet.grid.rows.len())?;
        let Some(bounds) = visible_bounds(sheet, *range) else {
            continue;
        };
        ranges.push(*range);
        for placement in placements {
            let r = placement.rect;
            let visible = placement.visible;
            let right = r.x + r.width;
            let bottom = r.y + r.height;
            if !right.is_finite() || !bottom.is_finite() {
                return Err(at(Failure::input("invalid_page_transform")));
            }
            for (side, line, outer, from, to) in [
                (
                    Side::Left,
                    &border.left,
                    visible.first_column == bounds.first_column,
                    [r.x, r.y],
                    [r.x, bottom],
                ),
                (
                    Side::Right,
                    &border.right,
                    visible.last_column == bounds.last_column,
                    [right, r.y],
                    [right, bottom],
                ),
                (
                    Side::Top,
                    &border.top,
                    visible.first_row == bounds.first_row,
                    [r.x, r.y],
                    [right, r.y],
                ),
                (
                    Side::Bottom,
                    &border.bottom,
                    visible.last_row == bounds.last_row,
                    [r.x, bottom],
                    [right, bottom],
                ),
            ] {
                if !outer {
                    continue;
                }
                let Some(line) = line else { continue };
                charge(1)?;
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
    Ok((result, ranges))
}
