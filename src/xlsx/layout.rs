use crate::xlsx::{
    diagnostic::{Category, Failure, Result},
    grid::Grid,
    print::{Orientation, PageBreak, PageOrder, PrintSettings, Scaling},
    range::{CellRange, MAX_COLUMN, MAX_ROW},
};
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct LayoutLimits {
    pub axis_items: usize,
    pub pages: usize,
    pub placements: usize,
}
impl Default for LayoutLimits {
    fn default() -> Self {
        Self {
            axis_items: 1_100_000,
            pages: 10_000,
            placements: 2_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    pub width: f64,
    pub height: f64,
}
impl Paper {
    pub fn from_id(id: u16, orientation: Orientation) -> Result<Self> {
        let mm = |v: f64| v * 72.0 / 25.4;
        let (mut width, mut height) = match id {
            1 | 2 | 18 => (612.0, 792.0),
            3 | 17 => (792.0, 1224.0),
            4 => (1224.0, 792.0),
            5 => (612.0, 1008.0),
            8 => (mm(297.0), mm(420.0)),
            9 | 10 => (mm(210.0), mm(297.0)),
            11 => (mm(148.0), mm(210.0)),
            _ => return Err(Failure::unsupported("paper_size")),
        };
        if orientation == Orientation::Landscape {
            std::mem::swap(&mut width, &mut height);
        }
        Ok(Self { width, height })
    }
}

#[derive(Clone, Debug)]
pub struct AxisItem {
    pub index: u32,
    pub offset: f64,
    pub size: f64,
    pub repeated: bool,
}
#[derive(Clone, Debug)]
pub struct Page {
    pub sheet_index: usize,
    pub area_index: usize,
    pub paper: Paper,
    pub scale: f64,
    pub origin_x: f64,
    pub origin_y: f64,
    pub columns: Arc<[AxisItem]>,
    pub rows: Arc<[AxisItem]>,
    pub body: CellRange,
}
fn failure(reason: &'static str) -> Failure {
    Failure::new(Category::LayoutFailure, reason)
}

/// OOXML の保存幅からポイントへ。mdw は標準書体の数字最大幅（96 dpi のピクセル）。
pub fn column_points(width: f64, mdw: f64) -> Result<f64> {
    if !width.is_finite()
        || !(0.0..=255.0).contains(&width)
        || !mdw.is_finite()
        || mdw <= 0.0
        || mdw > 4096.0
    {
        return Err(failure("invalid_column_metrics"));
    }
    Ok((((256.0 * width + (128.0 / mdw).floor()) / 256.0) * mdw).floor() * 0.75)
}

#[derive(Clone)]
struct Axis {
    items: Vec<(u32, f64)>,
    titles: Vec<(u32, f64)>,
    breaks: BTreeSet<u32>,
}
#[derive(Clone)]
struct Slice {
    items: Arc<[AxisItem]>,
    size: f64,
    first: u32,
    last: u32,
}

impl Axis {
    fn split(&self, capacity: f64, limits: &LayoutLimits) -> Result<Vec<Slice>> {
        let title_size: f64 = self.titles.iter().map(|(_, size)| size).sum();
        if !capacity.is_finite()
            || capacity <= 0.0
            || title_size > capacity
            || (title_size == capacity && !self.items.is_empty())
        {
            return Err(failure("repeated_titles_exceed_page"));
        }
        let mut result = Vec::new();
        let mut cursor = 0;
        let mut placements = 0usize;
        while cursor < self.items.len() {
            let mut offset = 0.0;
            let mut items = Vec::new();
            placements = placements
                .checked_add(self.titles.len())
                .filter(|n| *n <= limits.placements)
                .ok_or_else(|| failure("layout_placement_limit"))?;
            for (index, size) in &self.titles {
                items.push(AxisItem {
                    index: *index,
                    offset,
                    size: *size,
                    repeated: true,
                });
                offset += size;
            }
            let start = cursor;
            while let Some((index, size)) = self.items.get(cursor) {
                if offset + size > capacity {
                    if cursor == start {
                        return Err(failure("cell_exceeds_page"));
                    }
                    break;
                }
                if cursor > start
                    && self
                        .breaks
                        .range(self.items[cursor - 1].0..*index)
                        .next()
                        .is_some()
                {
                    break;
                }
                placements += 1;
                if placements > limits.placements {
                    return Err(failure("layout_placement_limit"));
                }
                items.push(AxisItem {
                    index: *index,
                    offset,
                    size: *size,
                    repeated: false,
                });
                offset += size;
                cursor += 1;
            }
            if result.len() >= limits.pages {
                return Err(failure("page_limit"));
            }
            result.push(Slice {
                items: items.into(),
                size: offset,
                first: self.items[start].0,
                last: self.items[cursor - 1].0,
            });
        }
        // 印刷範囲の全行または全列がタイトルに含まれるケース。
        if result.is_empty() && !self.titles.is_empty() {
            if self.titles.len() > limits.placements {
                return Err(failure("layout_placement_limit"));
            }
            let mut offset = 0.0;
            let items: Vec<_> = self
                .titles
                .iter()
                .map(|(index, size)| {
                    let item = AxisItem {
                        index: *index,
                        offset,
                        size: *size,
                        repeated: true,
                    };
                    offset += size;
                    item
                })
                .collect();
            result.push(Slice {
                items: items.into(),
                size: offset,
                first: self.titles[0].0,
                last: self.titles[self.titles.len() - 1].0,
            });
        }
        Ok(result)
    }
}

fn axis(
    grid: &Grid,
    span: (u32, u32),
    titles: Option<(u32, u32)>,
    breaks: &[PageBreak],
    cross: (u32, u32),
    mdw: Option<f64>,
    limits: &LayoutLimits,
) -> Result<Axis> {
    let (first, last) = span;
    let columns = mdw.is_some();
    let max = if columns { MAX_COLUMN } else { MAX_ROW };
    if first == 0 || first > last || last > max {
        return Err(Failure::input("invalid_print_range"));
    }
    if let Some((a, b)) = titles {
        if a == 0 || a > b || b > max {
            return Err(Failure::input("invalid_print_title"));
        }
        if b >= first && a > first {
            return Err(Failure::unsupported("nonleading_print_title"));
        }
    }
    let title_count = titles.map_or(0, |(a, b)| b - a + 1);
    if u64::from(last - first + 1) + u64::from(title_count) > limits.axis_items as u64 {
        return Err(failure("layout_axis_limit"));
    }
    let mut items = Vec::new();
    let mut title_items = Vec::new();
    let size = |index| axis_size(grid, index, mdw);
    if let Some((a, b)) = titles {
        for index in a..=b {
            let size = size(index)?;
            if size > 0.0 {
                title_items.push((index, size));
            }
        }
    }
    for index in first..=last {
        if titles.is_some_and(|(a, b)| a <= index && index <= b) {
            continue;
        }
        let size = size(index)?;
        if size > 0.0 {
            items.push((index, size));
        }
    }
    let mut manual = BTreeSet::new();
    for b in breaks {
        // 改ページの min/max は 0 始まり。部分範囲の改ページは将来個別対応する。
        if b.max < cross.0 - 1 || b.min > cross.1 - 1 {
            continue;
        }
        if b.min > cross.0 - 1 || b.max < cross.1 - 1 {
            return Err(Failure::unsupported("partial_page_break"));
        }
        manual.insert(b.after);
    }
    Ok(Axis {
        items,
        titles: title_items,
        breaks: manual,
    })
}

/// 文字・図形の配置前に、保存済み行列寸法からページの領域を組み立てる。
/// Excel 本体との比較が済むまでは、この結果を互換性検証済みと扱わない。
pub fn paginate(
    sheet_index: usize,
    grid: &Grid,
    print: &PrintSettings,
    mdw_pixels: f64,
    limits: &LayoutLimits,
) -> Result<Vec<Page>> {
    let work = || -> Result<Vec<Page>> {
        if limits.pages == 0 {
            return Err(failure("page_limit"));
        }
        let paper = Paper::from_id(
            print
                .paper_size
                .ok_or_else(|| Failure::unsupported("paper_size_missing"))?,
            print.orientation,
        )?;
        let margin = print
            .margins
            .as_ref()
            .ok_or_else(|| Failure::unsupported("page_margins_missing"))?;
        if [margin.left, margin.right, margin.top, margin.bottom]
            .iter()
            .any(|n| !n.is_finite() || *n < 0.0)
        {
            return Err(failure("invalid_page_margins"));
        }
        let width = paper.width - margin.left - margin.right;
        let height = paper.height - margin.top - margin.bottom;
        if width <= 0.0 || height <= 0.0 {
            return Err(failure("page_margins_exceed_paper"));
        }
        if print.areas.is_empty() {
            return Err(Failure::unsupported("print_area_missing"));
        }
        let mut pages = Vec::new();
        let mut placements = 0usize;
        for (area_index, area) in print.areas.iter().enumerate() {
            let manual = matches!(print.scaling, Scaling::Percent(_));
            let mut columns = axis(
                grid,
                (area.first_column, area.last_column),
                print
                    .repeat_columns
                    .map(|r| (r.first_column, r.last_column)),
                if manual { &print.column_breaks } else { &[] },
                (area.first_row, area.last_row),
                Some(mdw_pixels),
                limits,
            )?;
            let mut rows = axis(
                grid,
                (area.first_row, area.last_row),
                print.repeat_rows.map(|r| (r.first_row, r.last_row)),
                if manual { &print.row_breaks } else { &[] },
                (area.first_column, area.last_column),
                None,
                limits,
            )?;
            let (scale, x, y) = match print.scaling {
                Scaling::Percent(percent) => {
                    if !(10..=400).contains(&percent) {
                        return Err(Failure::input("invalid_print_scale"));
                    }
                    let scale = f64::from(percent) / 100.0;
                    (
                        scale,
                        columns.split(width / scale, limits)?,
                        rows.split(height / scale, limits)?,
                    )
                }
                Scaling::Fit {
                    width: horizontal,
                    height: vertical,
                } => {
                    columns.breaks.clear();
                    rows.breaks.clear();
                    let mut chosen = None;
                    for percent in (10..=100).rev() {
                        let scale = f64::from(percent) / 100.0;
                        let (Ok(x), Ok(y)) = (
                            columns.split(width / scale, limits),
                            rows.split(height / scale, limits),
                        ) else {
                            continue;
                        };
                        if (horizontal == 0 || x.len() <= usize::from(horizontal))
                            && (vertical == 0 || y.len() <= usize::from(vertical))
                        {
                            chosen = Some((scale, x, y));
                            break;
                        }
                    }
                    chosen.ok_or_else(|| failure("fit_to_pages_failed"))?
                }
            };
            let count = x
                .len()
                .checked_mul(y.len())
                .and_then(|n| n.checked_add(pages.len()))
                .ok_or_else(|| failure("page_limit"))?;
            if count > limits.pages {
                return Err(failure("page_limit"));
            }
            placements += x.iter().map(|s| s.items.len()).sum::<usize>()
                + y.iter().map(|s| s.items.len()).sum::<usize>();
            if placements > limits.placements {
                return Err(failure("layout_placement_limit"));
            }
            let make = |x: &Slice, y: &Slice| Page {
                sheet_index,
                area_index: area_index + 1,
                paper,
                scale,
                origin_x: margin.left
                    + if print.horizontal_center {
                        (width - x.size * scale) / 2.0
                    } else {
                        0.0
                    },
                origin_y: margin.top
                    + if print.vertical_center {
                        (height - y.size * scale) / 2.0
                    } else {
                        0.0
                    },
                columns: x.items.clone(),
                rows: y.items.clone(),
                body: CellRange {
                    first_column: x.first,
                    last_column: x.last,
                    first_row: y.first,
                    last_row: y.last,
                },
            };
            match print.order {
                PageOrder::DownThenOver => {
                    for column in &x {
                        for row in &y {
                            pages.push(make(column, row));
                        }
                    }
                }
                PageOrder::OverThenDown => {
                    for row in &y {
                        for column in &x {
                            pages.push(make(column, row));
                        }
                    }
                }
            }
        }
        Ok(pages)
    };
    work().map_err(|e| e.at(sheet_index, None))
}

/// 印刷配置と画像アンカーで共有する、非表示を除いた行列寸法。
pub(crate) fn axis_size(grid: &Grid, index: u32, mdw: Option<f64>) -> Result<f64> {
    if let Some(mdw) = mdw {
        let position = grid.columns.partition_point(|c| c.last < index);
        let col = grid.columns.get(position).filter(|c| c.first <= index);
        if col.is_some_and(|c| c.hidden) {
            return Ok(0.0);
        }
        let width = col
            .and_then(|c| c.width)
            .or(grid.default_column_width)
            .ok_or_else(|| Failure::unsupported("default_column_width_missing"))?;
        column_points(width, mdw)
    } else {
        let row = grid.rows.get(&index);
        if row.map_or(grid.default_rows_hidden, |r| r.hidden) {
            return Ok(0.0);
        }
        let height = row
            .and_then(|r| r.height)
            .or(grid.default_row_height)
            .ok_or_else(|| Failure::unsupported("default_row_height_missing"))?;
        if !height.is_finite() || !(0.0..=409.5).contains(&height) {
            return Err(failure("invalid_row_metrics"));
        }
        Ok(height)
    }
}
