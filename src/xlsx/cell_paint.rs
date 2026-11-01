//! 保存済みのセル・行・列書式をページ上の背景矩形へ接続する。
use crate::xlsx::{
    color::ColorContext,
    diagnostic::{Failure, Result},
    grid::StyleSpan,
    layout::Page,
    pdf::graphics::Paint,
    placement::range_on_page,
    range::CellRange,
    styles::{Fill, Styles},
    workbook::{Sheet, cell_address},
};
use std::collections::BTreeMap;

/// 塗りだけを生成する。文字・罫線・画像より前に描画する。
/// limit は検査するスタイル区間・セルと生成する矩形それぞれの上限。
/// 結合範囲は左上セルの塗りで置き換え、内部セルの背景を残さない。
pub fn page_fills(
    sheet: &Sheet,
    page: &Page,
    styles: &Styles,
    colors: &ColorContext<'_>,
    limit: usize,
) -> Result<Vec<Paint<'static>>> {
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
            .ok_or_else(|| Failure::input("cell_paint_limit"))?;
        Ok(())
    };
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
    let (Some(first), Some(last)) = (page.columns.first(), page.columns.last()) else {
        return Err(Failure::input("empty_page_axis"));
    };
    // 共通の配置検査を先に通し、空塗りだけの場合も不正なページ軸を拒否する。
    let axis_work = page
        .columns
        .len()
        .checked_add(page.rows.len())
        .ok_or_else(|| Failure::input("cell_paint_limit"))?;
    charge(axis_work)?;
    range_on_page(
        page,
        CellRange {
            first_column: 1,
            last_column: 16384,
            first_row: 1,
            last_row: 1048576,
        },
    )?;
    if page.rows.is_empty() {
        return Err(Failure::input("empty_page_axis"));
    }
    let mut merges = Vec::new();
    for (index, range) in sheet.grid.merges.iter().enumerate() {
        charge(1)?;
        for prior in &sheet.grid.merges[..index] {
            charge(1)?;
            if range.intersects(*prior) {
                return Err(Failure::input("overlapping_merged_cells").at(sheet.index, None));
            }
        }
        charge(axis_work)?;
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
        merges.push((*range, style));
    }
    let mut result = Vec::new();
    for row in page.rows.iter() {
        // 列定義の検査・走査も計算量の上限へ含める。
        charge(sheet.grid.columns.len())?;
        let spans = sheet.grid.style_spans(row.index, first.index, last.index)?;
        let mut resolved = Vec::new();
        for span in spans {
            charge(1)?;
            let mut cursor = span.first;
            for (&(_, column), cell) in
                cells.range((row.index, span.first)..=(row.index, span.last))
            {
                charge(1)?;
                if cursor < column {
                    resolved.push(StyleSpan {
                        first: cursor,
                        last: column - 1,
                        style: span.style,
                    });
                }
                resolved.push(StyleSpan {
                    first: column,
                    last: column,
                    style: cell.style,
                });
                cursor = column + 1;
            }
            if cursor <= span.last {
                resolved.push(StyleSpan {
                    first: cursor,
                    last: span.last,
                    style: span.style,
                });
            }
        }
        // 元セルの位置を保持し、結合の途中ページでも診断を左上セルへ戻す。
        let mut resolved: Vec<_> = resolved
            .into_iter()
            .map(|span| (span, (span.first, row.index)))
            .collect();
        for (range, style) in &merges {
            charge(1)?;
            if row.index < range.first_row || row.index > range.last_row {
                continue;
            }
            let mut replaced = Vec::new();
            for (span, source) in resolved {
                charge(1)?;
                let first = span.first.max(range.first_column);
                let last = span.last.min(range.last_column);
                if first > last {
                    replaced.push((span, source));
                    continue;
                }
                if span.first < first {
                    replaced.push((
                        StyleSpan {
                            last: first - 1,
                            ..span
                        },
                        source,
                    ));
                }
                replaced.push((
                    StyleSpan {
                        first,
                        last,
                        style: *style,
                    },
                    (range.first_column, range.first_row),
                ));
                if last < span.last {
                    replaced.push((
                        StyleSpan {
                            first: last + 1,
                            ..span
                        },
                        source,
                    ));
                }
            }
            resolved = replaced;
        }
        for (span, source) in resolved {
            let range = CellRange {
                first_column: span.first,
                last_column: span.last,
                first_row: row.index,
                last_row: row.index,
            };
            charge(axis_work)?;
            let placements = range_on_page(page, range)?;
            if placements.is_empty() {
                continue;
            }
            let mut column = source.0;
            let mut letters = Vec::new();
            while column > 0 {
                letters.push((b'A' + ((column - 1) % 26) as u8) as char);
                column = (column - 1) / 26;
            }
            let address = format!(
                "{}{}",
                letters.into_iter().rev().collect::<String>(),
                source.1
            );
            let at = |error: Failure| error.at(sheet.index, Some(address.clone()));
            let style = styles.validate_cell(span.style).map_err(at)?;
            let fill = styles.fills[style.fill]
                .as_ref()
                .map_err(|e| at(e.clone()))?;
            let color = match fill {
                Fill::None => continue,
                Fill::Solid(color) => color.resolve(colors).map_err(at)?,
                Fill::Gray125 => return Err(Failure::unsupported("pattern_fill")),
            };
            for placement in placements {
                if result.len() >= limit {
                    return Err(Failure::input("cell_paint_limit"));
                }
                result.push(Paint::Fill {
                    rect: placement.rect,
                    color,
                });
            }
        }
    }
    Ok(result)
}
