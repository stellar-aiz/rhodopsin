//! 隣接セルの占有状況から、文字列の横方向の可視範囲を求める。
//! セル余白と Excel の紙面一致は別途検証する。CLI の既定組版にはまだ接続しない。
use crate::xlsx::{
    diagnostic::{Failure, Result},
    layout::Page,
    pdf::graphics::BoxRect,
    placement::{RangePlacement, range_on_page},
    range::CellRange,
    styles::Horizontal,
    workbook::{Cell, CellValue, Sheet, cell_address},
};
use std::collections::{BTreeMap, BTreeSet};

pub struct Occupancy {
    sheet_index: usize,
    rows: BTreeMap<u32, BTreeSet<u32>>,
    merges: Vec<CellRange>,
}
impl Occupancy {
    pub fn new(sheet: &Sheet, limit: usize) -> Result<Self> {
        if sheet
            .cells
            .len()
            .checked_add(sheet.grid.merges.len())
            .is_none_or(|count| count > limit)
        {
            return Err(Failure::input("text_overflow_work_limit"));
        }
        let mut rows = BTreeMap::<_, BTreeSet<_>>::new();
        let mut seen = BTreeSet::new();
        for cell in &sheet.cells {
            let (column, row) = cell_address(&cell.address)
                .ok_or_else(|| Failure::input("invalid_cell_address"))?;
            if !seen.insert((row, column)) {
                return Err(
                    Failure::input("duplicate_cell").at(sheet.index, Some(cell.address.clone()))
                );
            }
            // 空文字列の計算結果は空セルと区別する。
            if !matches!(cell.value, CellValue::Blank) {
                rows.entry(row).or_default().insert(column);
            }
        }
        for range in &sheet.grid.merges {
            if range.first_column == 0
                || range.first_row == 0
                || range.first_column > range.last_column
                || range.first_row > range.last_row
                || range.last_column > 16384
                || range.last_row > 1048576
            {
                return Err(Failure::input("invalid_cell_range"));
            }
        }
        Ok(Self {
            sheet_index: sheet.index,
            rows,
            merges: sheet.grid.merges.clone(),
        })
    }

    /// 数値・折り返し・結合セルは元の矩形。左揃えは右、右揃えは左、中央揃えは両側へ広げる。
    /// 空セルでも結合範囲なら境界とする。繰り返し列と本文の間は越えない。
    pub fn clip(
        &self,
        page: &Page,
        cell: &Cell,
        placement: &RangePlacement,
        horizontal: Horizontal,
        wrap: bool,
        limit: usize,
    ) -> Result<BoxRect> {
        let mut remaining = limit;
        self.clip_with_budget(page, cell, placement, horizontal, wrap, &mut remaining)
    }

    /// 同じページの複数セルで共有する走査予算。失敗しても消費済み作業は戻さない。
    pub fn clip_with_budget(
        &self,
        page: &Page,
        cell: &Cell,
        placement: &RangePlacement,
        horizontal: Horizontal,
        wrap: bool,
        remaining: &mut usize,
    ) -> Result<BoxRect> {
        let at = |error: Failure| error.at(self.sheet_index, Some(cell.address.clone()));
        let (column, row) =
            cell_address(&cell.address).ok_or_else(|| Failure::input("invalid_cell_address"))?;
        if self.sheet_index != page.sheet_index {
            return Err(Failure::input("page_sheet_mismatch"));
        }
        let mut charge = |count: usize| {
            *remaining = remaining
                .checked_sub(count)
                .ok_or_else(|| at(Failure::input("text_overflow_work_limit")))?;
            Ok::<_, Failure>(())
        };
        charge(page.columns.len())?;
        charge(page.rows.len())?;
        let resolved = range_on_page(page, placement.visible).map_err(at)?;
        if resolved.len() != 1
            || placement.visible.first_column != column
            || placement.visible.first_row != row
        {
            return Err(at(Failure::input("text_overflow_placement_mismatch")));
        }
        let rect = resolved[0].rect;
        if [rect.x, rect.y, rect.width, rect.height]
            != [
                placement.rect.x,
                placement.rect.y,
                placement.rect.width,
                placement.rect.height,
            ]
        {
            return Err(at(Failure::input("text_overflow_placement_mismatch")));
        }
        charge(self.merges.len())?;
        let merged = self.merges.iter().any(|range| {
            range.first_column <= column
                && column <= range.last_column
                && range.first_row <= row
                && row <= range.last_row
        });
        if !merged && (placement.visible.last_column != column || placement.visible.last_row != row)
        {
            return Err(at(Failure::input("text_overflow_placement_mismatch")));
        }
        if !matches!(cell.value, CellValue::Text(_) | CellValue::RichText(_)) || wrap || merged {
            return Ok(rect);
        }
        let (left, right) = match horizontal {
            Horizontal::General | Horizontal::Left => (false, true),
            Horizontal::Right => (true, false),
            Horizontal::Center => (true, true),
            _ => return Err(at(Failure::unsupported("text_overflow_alignment"))),
        };
        let start = page
            .columns
            .binary_search_by_key(&column, |item| item.index)
            .map_err(|_| at(Failure::input("text_overflow_placement_mismatch")))?;
        let mut first = start;
        let mut last = start;
        let mut available = |index: usize| -> Result<bool> {
            charge(self.merges.len() + 1)?;
            let candidate = page.columns[index].index;
            Ok(!self
                .rows
                .get(&row)
                .is_some_and(|cells| cells.contains(&candidate))
                && !self.merges.iter().any(|range| {
                    range.first_column <= candidate
                        && candidate <= range.last_column
                        && range.first_row <= row
                        && row <= range.last_row
                }))
        };
        if left {
            while first > 0 {
                let previous = &page.columns[first - 1];
                let current = &page.columns[first];
                if previous.repeated != current.repeated
                    || previous.offset + previous.size != current.offset
                    || !available(first - 1)?
                {
                    break;
                }
                first -= 1;
            }
        }
        if right {
            while last + 1 < page.columns.len() {
                let next = &page.columns[last + 1];
                let current = &page.columns[last];
                if next.repeated != current.repeated
                    || current.offset + current.size != next.offset
                    || !available(last + 1)?
                {
                    break;
                }
                last += 1;
            }
        }
        let x = page.origin_x + page.columns[first].offset * page.scale;
        let end =
            page.origin_x + (page.columns[last].offset + page.columns[last].size) * page.scale;
        Ok(BoxRect {
            x,
            width: end - x,
            ..rect
        })
    }
}
