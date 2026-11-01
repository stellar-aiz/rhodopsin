//! ページ上に現れるセル範囲の可視部分を、描画用のポイント座標へ変換する。
use crate::xlsx::{
    diagnostic::{Category, Failure, Result},
    layout::{AxisItem, Page},
    pdf::graphics::BoxRect,
    range::{CellRange, MAX_COLUMN, MAX_ROW},
};

pub struct RangePlacement {
    /// この矩形に含まれる最初と最後の可視行列。途中に非表示行列を含み得る。
    pub visible: CellRange,
    pub rect: BoxRect,
    pub repeated_rows: bool,
    pub repeated_columns: bool,
}
struct Span {
    first: u32,
    last: u32,
    offset: f64,
    end: f64,
    repeated: bool,
}
fn failure(reason: &'static str) -> Failure {
    Failure::new(Category::LayoutFailure, reason)
}
fn spans(axis: &[AxisItem], first: u32, last: u32) -> Result<Vec<Span>> {
    let mut spans: Vec<Span> = Vec::new();
    let mut previous_index = 0;
    let mut previous_end = 0.0;
    for item in axis {
        if item.index <= previous_index
            || !item.offset.is_finite()
            || !item.size.is_finite()
            || item.size <= 0.0
            || item.offset < previous_end
        {
            return Err(failure("invalid_page_axis"));
        }
        let end = item.offset + item.size;
        if !end.is_finite() {
            return Err(failure("invalid_page_axis"));
        }
        previous_index = item.index;
        previous_end = end;
        if item.index < first || item.index > last {
            continue;
        }
        if let Some(span) = spans.last_mut()
            && span.repeated == item.repeated
            && span.end == item.offset
        {
            span.last = item.index;
            span.end = end;
        } else {
            // paginate が生成する軸は見出しと本文の最大 2 区間。
            // 不正な外部構築データから直積の割当が膨張することも防ぐ。
            if spans.len() == 2 {
                return Err(failure("invalid_page_axis"));
            }
            spans.push(Span {
                first: item.index,
                last: item.index,
                offset: item.offset,
                end,
                repeated: item.repeated,
            });
        }
    }
    Ok(spans)
}

/// 繰り返し部分と本文は別矩形にする。非表示・ページ外だけの範囲は空になる。
/// 結合セルも渡せるが、返すのは可視部分であり、全文字配置の原点ではない。
pub fn range_on_page(page: &Page, range: CellRange) -> Result<Vec<RangePlacement>> {
    if range.first_column == 0
        || range.first_row == 0
        || range.last_column > MAX_COLUMN
        || range.last_row > MAX_ROW
        || range.first_column > range.last_column
        || range.first_row > range.last_row
    {
        return Err(Failure::input("invalid_cell_range"));
    }
    if !page.scale.is_finite()
        || page.scale <= 0.0
        || !page.origin_x.is_finite()
        || !page.origin_y.is_finite()
    {
        return Err(failure("invalid_page_transform"));
    }
    let columns = spans(&page.columns, range.first_column, range.last_column)?;
    let rows = spans(&page.rows, range.first_row, range.last_row)?;
    let mut result = Vec::new();
    for row in rows {
        for column in &columns {
            let rect = BoxRect {
                x: page.origin_x + column.offset * page.scale,
                y: page.origin_y + row.offset * page.scale,
                width: (column.end - column.offset) * page.scale,
                height: (row.end - row.offset) * page.scale,
            };
            if [rect.x, rect.y, rect.width, rect.height]
                .iter()
                .any(|n| !n.is_finite())
            {
                return Err(failure("invalid_page_transform"));
            }
            result.push(RangePlacement {
                visible: CellRange {
                    first_column: column.first,
                    last_column: column.last,
                    first_row: row.first,
                    last_row: row.last,
                },
                rect,
                repeated_rows: row.repeated,
                repeated_columns: column.repeated,
            });
        }
    }
    Ok(result)
}
