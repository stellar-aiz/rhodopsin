use crate::xlsx::{
    diagnostic::{Failure, Result},
    workbook::cell_address,
};

pub const MAX_COLUMN: u32 = 16_384;
pub const MAX_ROW: u32 = 1_048_576;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRange {
    pub first_column: u32,
    pub first_row: u32,
    pub last_column: u32,
    pub last_row: u32,
}

impl CellRange {
    pub fn parse(value: &str) -> Result<Self> {
        let (first, last) = value.split_once(':').unwrap_or((value, value));
        let parse = |s: &str| -> Option<(u32, u32)> {
            let s = s.strip_prefix('$').unwrap_or(s);
            let split = s.find(|c: char| !c.is_ascii_uppercase())?;
            let (col, row) = s.split_at(split);
            if col.is_empty() {
                return None;
            }
            let row = row.strip_prefix('$').unwrap_or(row);
            cell_address(&format!("{col}{row}"))
        };
        let (first_column, first_row) =
            parse(first).ok_or_else(|| Failure::input("invalid_cell_range"))?;
        let (last_column, last_row) =
            parse(last).ok_or_else(|| Failure::input("invalid_cell_range"))?;
        if first_column > last_column || first_row > last_row {
            return Err(Failure::input("reversed_cell_range"));
        }
        Ok(Self {
            first_column,
            first_row,
            last_column,
            last_row,
        })
    }

    pub fn intersects(self, other: Self) -> bool {
        self.first_column <= other.last_column
            && other.first_column <= self.last_column
            && self.first_row <= other.last_row
            && other.first_row <= self.last_row
    }
}

/// 名前定義の和集合を分割する。引用されたシート名内のカンマは区切らない。
pub fn references(value: &str, expected_sheet: &str) -> Result<Vec<String>> {
    let mut quoted = false;
    let mut start = 0;
    let mut parts = Vec::new();
    let mut chars = value.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' {
            if quoted && chars.peek().is_some_and(|(_, c)| *c == '\'') {
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if ch == ',' && !quoted {
            parts.push(&value[start..index]);
            start = index + 1;
        }
    }
    if quoted {
        return Err(Failure::input("invalid_defined_name"));
    }
    parts.push(&value[start..]);
    parts
        .into_iter()
        .map(|part| {
            let (name, reference) = part
                .trim()
                .rsplit_once('!')
                .ok_or_else(|| Failure::unsupported("dynamic_print_range"))?;
            let name =
                if let Some(inner) = name.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                    inner.replace("''", "'")
                } else {
                    name.to_string()
                };
            if name != expected_sheet {
                return Err(Failure::unsupported("cross_sheet_print_range"));
            }
            Ok(reference.to_string())
        })
        .collect()
}

pub fn title_range(value: &str) -> Result<CellRange> {
    let (first, last) = value
        .split_once(':')
        .ok_or_else(|| Failure::input("invalid_print_title"))?;
    let first = first.strip_prefix('$').unwrap_or(first);
    let last = last.strip_prefix('$').unwrap_or(last);
    if first.bytes().all(|b| b.is_ascii_digit()) && last.bytes().all(|b| b.is_ascii_digit()) {
        let range = CellRange::parse(&format!("A{first}:XFD{last}"))?;
        Ok(range)
    } else if first.bytes().all(|b| b.is_ascii_uppercase())
        && last.bytes().all(|b| b.is_ascii_uppercase())
    {
        CellRange::parse(&format!("{first}1:{last}1048576"))
    } else {
        Err(Failure::input("invalid_print_title"))
    }
}
