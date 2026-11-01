//! 内容を含まない、元セル単位の警告集計。表示ページ数では件数を増やさない。
use crate::xlsx::{
    diagnostic::{Failure, Location, Result},
    workbook::cell_address,
};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    NumericOverflow,
    FontSubstitution,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    kind: Kind,
    count: usize,
    locations: Vec<Location>,
}

#[derive(Clone, Default)]
pub struct Warnings {
    // 行・列を数値で並べ、A10 が A2 より前に出る辞書順を避ける。
    cells: BTreeSet<(Kind, usize, u32, u32)>,
}
impl Warnings {
    /// 同種・同シート・同セルは一件。上限超過時は既存集計を変更しない。
    pub fn record(
        &mut self,
        kind: Kind,
        sheet_index: usize,
        address: &str,
        limit: usize,
    ) -> Result<()> {
        self.record_kinds(&[kind], sheet_index, address, limit)
    }

    /// 一つのセルに生じた複数種類をまとめて登録する。上限不足なら全件を拒否する。
    pub fn record_kinds(
        &mut self,
        kinds: &[Kind],
        sheet_index: usize,
        address: &str,
        limit: usize,
    ) -> Result<()> {
        let (column, row) = cell_address(address)
            .filter(|_| sheet_index > 0)
            .ok_or_else(|| Failure::input("invalid_warning_location"))?;
        let additions: BTreeSet<_> = kinds
            .iter()
            .map(|kind| (*kind, sheet_index, row, column))
            .filter(|key| !self.cells.contains(key))
            .collect();
        if !additions.is_empty()
            && self
                .cells
                .len()
                .checked_add(additions.len())
                .is_none_or(|size| size > limit)
        {
            return Err(Failure::input("warning_limit"));
        }
        self.cells.extend(additions);
        Ok(())
    }

    /// 種類、シート順、行、列の順で決定的に返す。値・シート名・書体名は保持しない。
    pub fn into_summaries(self) -> Vec<Summary> {
        let mut result: Vec<Summary> = Vec::new();
        for (kind, sheet_index, row, mut column) in self.cells {
            let mut letters = Vec::new();
            while column > 0 {
                letters.push(char::from(b'A' + ((column - 1) % 26) as u8));
                column = (column - 1) / 26;
            }
            let cell = format!("{}{}", letters.into_iter().rev().collect::<String>(), row);
            if result.last().is_none_or(|summary| summary.kind != kind) {
                result.push(Summary {
                    kind,
                    count: 0,
                    locations: Vec::new(),
                });
            }
            let summary = result.last_mut().expect("警告種別の集計は作成済み");
            summary.locations.push(Location {
                sheet_index,
                cell: Some(cell),
            });
            summary.count += 1;
        }
        result
    }
}
