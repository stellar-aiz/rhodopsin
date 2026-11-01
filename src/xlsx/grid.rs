use crate::xlsx::{
    diagnostic::{Failure, Result},
    range::{CellRange, MAX_COLUMN, MAX_ROW},
    xml::{S, boolean, finite, number, singleton},
};
use roxmltree::Node;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Column {
    pub first: u32,
    pub last: u32,
    pub width: Option<f64>,
    pub hidden: bool,
    pub style: Option<usize>,
}
#[derive(Clone, Debug)]
pub struct Row {
    pub height: Option<f64>,
    pub hidden: bool,
    pub style: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleSpan {
    pub first: u32,
    pub last: u32,
    pub style: usize,
}
#[derive(Clone, Debug)]
pub struct Grid {
    pub default_row_height: Option<f64>,
    pub default_column_width: Option<f64>,
    pub base_column_width: u32,
    pub default_rows_hidden: bool,
    pub columns: Vec<Column>,
    pub rows: BTreeMap<u32, Row>,
    pub merges: Vec<CellRange>,
}

impl Grid {
    /// セル要素が存在しない空白にも適用する、行・列の既定スタイル区間。
    /// 明示セルの s 属性による上書きは呼び出し側で扱う。
    /// 幅に比例してセルを生成せず、列定義の境界だけで区切る。
    pub fn style_spans(&self, row: u32, first: u32, last: u32) -> Result<Vec<StyleSpan>> {
        if row == 0 || row > MAX_ROW || first == 0 || first > last || last > MAX_COLUMN {
            return Err(Failure::input("invalid_cell_range"));
        }
        // Grid は公開構造体なので、外部構築された不正な列定義も拒否する。
        let mut previous = 0;
        for column in &self.columns {
            if column.first <= previous || column.first > column.last || column.last > MAX_COLUMN {
                return Err(Failure::input("invalid_column_range"));
            }
            previous = column.last;
        }
        if let Some(style) = self.rows.get(&row).and_then(|r| r.style) {
            return Ok(vec![StyleSpan { first, last, style }]);
        }
        let mut result: Vec<StyleSpan> = Vec::new();
        let mut push = |first, last, style| {
            if let Some(prior) = result.last_mut()
                && prior.last + 1 == first
                && prior.style == style
            {
                prior.last = last;
            } else {
                result.push(StyleSpan { first, last, style });
            }
        };
        let mut cursor = first;
        for column in &self.columns {
            if column.last < first {
                continue;
            }
            if column.first > last {
                break;
            }
            let start = column.first.max(first);
            let end = column.last.min(last);
            if cursor < start {
                push(cursor, start - 1, 0);
            }
            push(start, end, column.style.unwrap_or(0));
            cursor = end + 1;
        }
        if cursor <= last {
            push(cursor, last, 0);
        }
        Ok(result)
    }

    pub fn parse(root: Node<'_, '_>) -> Result<Self> {
        let mut result = Self {
            default_row_height: None,
            default_column_width: None,
            base_column_width: 8,
            default_rows_hidden: false,
            columns: Vec::new(),
            rows: BTreeMap::new(),
            merges: Vec::new(),
        };
        if let Some(n) = singleton(root, "sheetFormatPr")? {
            if n.attribute("defaultRowHeight").is_some() {
                result.default_row_height = Some(finite(n, "defaultRowHeight", 0.0, 0.0, 409.5)?);
            }
            if n.attribute("defaultColWidth").is_some() {
                result.default_column_width = Some(finite(n, "defaultColWidth", 0.0, 0.0, 255.0)?);
            }
            result.base_column_width = number(n, "baseColWidth", 8)?;
            if result.base_column_width > 255 {
                return Err(Failure::input("invalid_column_width"));
            }
            result.default_rows_hidden = boolean(n, "zeroHeight", false)?;
            for attr in ["thickTop", "thickBottom"] {
                if boolean(n, attr, false)? {
                    return Err(Failure::unsupported("row_border_height_adjustment"));
                }
            }
        }
        if let Some(cols) = singleton(root, "cols")? {
            for n in cols.children().filter(|n| n.is_element()) {
                if !n.has_tag_name((S, "col")) {
                    return Err(Failure::unsupported("column_extension"));
                }
                let first = number(n, "min", 0u32)?;
                let last = number(n, "max", 0u32)?;
                if first == 0 || first > last || last > MAX_COLUMN {
                    return Err(Failure::input("invalid_column_range"));
                }
                let width = n
                    .attribute("width")
                    .map(|_| finite(n, "width", 0.0, 0.0, 255.0))
                    .transpose()?;
                result.columns.push(Column {
                    first,
                    last,
                    width,
                    hidden: boolean(n, "hidden", false)?,
                    style: n
                        .attribute("style")
                        .map(|_| number(n, "style", 0usize))
                        .transpose()?,
                });
            }
            result.columns.sort_by_key(|c| c.first);
            if result.columns.windows(2).any(|c| c[0].last >= c[1].first) {
                return Err(Failure::input("overlapping_column_ranges"));
            }
        }
        if let Some(data) = singleton(root, "sheetData")? {
            for n in data.children().filter(|n| n.is_element()) {
                if !n.has_tag_name((S, "row")) {
                    return Err(Failure::unsupported("row_extension"));
                }
                let index = number(n, "r", 0u32)?;
                if index == 0 || index > MAX_ROW {
                    return Err(Failure::input("invalid_row_index"));
                }
                let thick_top = boolean(n, "thickTop", false)?;
                let thick_bottom = boolean(n, "thickBot", false)?;
                // 保存済み ht はポイント単位の行高。ECMA-376 §18.3.1.73 の
                // thickTop/thickBot は既に行われた補正を示すため再加算しない。
                // customHeight は自動再計算の区別であり、入力を変更しない変換では
                // どちらも保存された寸法を使う。不正な真偽値は受理しない。
                boolean(n, "customHeight", false)?;
                let height = n
                    .attribute("ht")
                    .map(|_| finite(n, "ht", 0.0, 0.0, 409.5))
                    .transpose()?;
                if (thick_top || thick_bottom) && height.is_none() {
                    return Err(Failure::unsupported("row_border_height_adjustment"));
                }
                let style = if boolean(n, "customFormat", false)? {
                    Some(number(n, "s", 0usize)?)
                } else {
                    None
                };
                let row = Row {
                    height,
                    hidden: boolean(n, "hidden", result.default_rows_hidden)?,
                    style,
                };
                if result.rows.insert(index, row).is_some() {
                    return Err(Failure::input("duplicate_row"));
                }
                for c in n.children().filter(|n| n.has_tag_name((S, "c"))) {
                    // 番地そのものの検査・診断は workbook 側で行う。
                    if let Some((_, cell_row)) = c
                        .attribute("r")
                        .and_then(crate::xlsx::workbook::cell_address)
                        && cell_row != index
                    {
                        return Err(Failure::input("cell_row_mismatch")
                            .at(0, c.attribute("r").map(str::to_owned)));
                    }
                }
            }
        }
        if let Some(merges) = singleton(root, "mergeCells")? {
            for n in merges.children().filter(|n| n.is_element()) {
                if !n.has_tag_name((S, "mergeCell")) {
                    return Err(Failure::unsupported("merge_extension"));
                }
                let range = CellRange::parse(
                    n.attribute("ref")
                        .ok_or_else(|| Failure::input("merge_range_missing"))?,
                )?;
                result.merges.push(range);
            }
            // 行スイープで全グリッド展開を避ける。アクティブ範囲は列数で制限される。
            result.merges.sort_by_key(|r| (r.first_row, r.first_column));
            let mut active = BTreeMap::<u32, CellRange>::new();
            for range in &result.merges {
                active.retain(|_, prior| prior.last_row >= range.first_row);
                if active
                    .range(..=range.last_column)
                    .next_back()
                    .is_some_and(|(_, prior)| prior.last_column >= range.first_column)
                {
                    return Err(Failure::input("overlapping_merged_cells"));
                }
                active.insert(range.first_column, *range);
            }
        }
        Ok(result)
    }

    pub fn style_for(&self, column: u32, row: u32) -> usize {
        self.rows
            .get(&row)
            .and_then(|r| r.style)
            .or_else(|| {
                self.columns
                    .iter()
                    .find(|c| c.first <= column && column <= c.last)
                    .and_then(|c| c.style)
            })
            .unwrap_or(0)
    }
}
