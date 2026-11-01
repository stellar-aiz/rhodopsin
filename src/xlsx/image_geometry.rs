//! 保存行列寸法から、画像アンカーのシート上のポイント座標を求める。
use crate::xlsx::{
    diagnostic::{Failure, Result},
    drawing::{Anchor, Marker, Picture},
    grid::Grid,
    layout::{AxisItem, Page, axis_size},
    pdf::graphics::{BoxRect, Paint},
    range::{MAX_COLUMN, MAX_ROW},
};

pub struct GridGeometry {
    columns: Vec<f64>,
    rows: Vec<f64>,
}
impl GridGeometry {
    /// 指定行列の左上までの累積寸法。行列数合計を割当前に制限する。
    pub fn new(
        grid: &Grid,
        last_column: u32,
        last_row: u32,
        mdw: f64,
        limit: usize,
    ) -> Result<Self> {
        if last_column == 0 || last_column > MAX_COLUMN || last_row == 0 || last_row > MAX_ROW {
            return Err(Failure::input("invalid_drawing_grid"));
        }
        if u64::from(last_column) + u64::from(last_row) > limit as u64 {
            return Err(Failure::input("drawing_axis_limit"));
        }
        if !mdw.is_finite() || mdw <= 0.0 {
            return Err(Failure::input("invalid_column_metrics"));
        }
        // 公開 Grid の範囲がソート済み・非重複であることも確認する。
        grid.style_spans(1, 1, MAX_COLUMN)?;
        let prefix = |last, metric| -> Result<Vec<f64>> {
            let mut positions = Vec::with_capacity(last as usize + 1);
            positions.push(0.0);
            for index in 1..=last {
                let end = positions[index as usize - 1] + axis_size(grid, index, metric)?;
                if !end.is_finite() {
                    return Err(Failure::input("invalid_drawing_grid"));
                }
                positions.push(end);
            }
            Ok(positions)
        };
        Ok(Self {
            columns: prefix(last_column, Some(mdw))?,
            rows: prefix(last_row, None)?,
        })
    }

    fn point(&self, marker: Marker) -> Result<(f64, f64)> {
        if marker.column as usize >= self.columns.len() || marker.row as usize >= self.rows.len() {
            return Err(Failure::input("drawing_marker_out_of_range"));
        }
        let x = marker
            .column
            .checked_sub(1)
            .and_then(|index| self.columns.get(index as usize));
        let y = marker
            .row
            .checked_sub(1)
            .and_then(|index| self.rows.get(index as usize));
        let (Some(x), Some(y)) = (x, y) else {
            return Err(Failure::input("drawing_marker_out_of_range"));
        };
        Ok((x + emu(marker.column_offset)?, y + emu(marker.row_offset)?))
    }

    /// 印刷倍率・余白は未適用。ページごとの切り取りと見出しの再配置は別層で扱う。
    pub fn anchor(&self, anchor: &Anchor) -> Result<BoxRect> {
        let (from, width, height) = match *anchor {
            Anchor::OneCell {
                from,
                width_emu,
                height_emu,
            } => (from, emu(width_emu)?, emu(height_emu)?),
            Anchor::TwoCell { from, to, .. } => {
                let (x, y) = self.point(from)?;
                let (end_x, end_y) = self.point(to)?;
                (from, end_x - x, end_y - y)
            }
        };
        let (x, y) = self.point(from)?;
        if width <= 0.0 || height <= 0.0 {
            return Err(Failure::input("invalid_drawing_extent"));
        }
        Ok(BoxRect {
            x,
            y,
            width,
            height,
        })
    }

    /// 元画像の矩形を保ち、見出し・本文それぞれの可視領域で切り取る。
    pub fn picture_on_page<'a>(&self, picture: &'a Picture, page: &Page) -> Result<Vec<Paint<'a>>> {
        if picture.hidden || !picture.print {
            return Ok(Vec::new());
        }
        if !page.scale.is_finite()
            || !(0.1..=4.0).contains(&page.scale)
            || !page.origin_x.is_finite()
            || !page.origin_y.is_finite()
        {
            return Err(Failure::input("invalid_page_transform"));
        }
        let image = self.anchor(&picture.anchor)?;
        let (left_crop, top_crop, width_fraction, height_fraction) = picture.crop.fractions()?;
        let source_width = image.width / width_fraction;
        let source_height = image.height / height_fraction;
        let source_x = image.x - left_crop * source_width;
        let source_y = image.y - top_crop * source_height;
        let columns = segments(&self.columns, &page.columns)?;
        let rows = segments(&self.rows, &page.rows)?;
        let mut result = Vec::new();
        for row in &rows {
            for column in &columns {
                let left = image.x.max(column.start);
                let right = (image.x + image.width).min(column.end);
                let top = image.y.max(row.start);
                let bottom = (image.y + image.height).min(row.end);
                if left >= right || top >= bottom {
                    continue;
                }
                let map_x = |x| page.origin_x + (column.offset + x - column.start) * page.scale;
                let map_y = |y| page.origin_y + (row.offset + y - row.start) * page.scale;
                let rect = BoxRect {
                    x: map_x(source_x),
                    y: map_y(source_y),
                    width: source_width * page.scale,
                    height: source_height * page.scale,
                };
                let clip = BoxRect {
                    x: map_x(left),
                    y: map_y(top),
                    width: (right - left) * page.scale,
                    height: (bottom - top) * page.scale,
                };
                if [
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    clip.x,
                    clip.y,
                    clip.width,
                    clip.height,
                ]
                .iter()
                .any(|value| !value.is_finite())
                {
                    return Err(Failure::input("invalid_page_transform"));
                }
                result.push(Paint::Image {
                    rect,
                    raster: &picture.raster,
                    clip: Some(clip),
                });
            }
        }
        Ok(result)
    }
}

struct Segment {
    start: f64,
    end: f64,
    offset: f64,
    repeated: bool,
}
fn segments(prefix: &[f64], axis: &[AxisItem]) -> Result<Vec<Segment>> {
    let mut result: Vec<Segment> = Vec::new();
    let mut previous = 0;
    let mut page_end = 0.0;
    for item in axis {
        if item.index <= previous
            || item.index as usize >= prefix.len()
            || !item.offset.is_finite()
            || !item.size.is_finite()
            || item.size <= 0.0
            || item.offset != page_end
        {
            return Err(Failure::input("invalid_image_page_axis"));
        }
        let start = prefix[item.index as usize - 1];
        let end = prefix[item.index as usize];
        // 累積値の差に伴う浮動小数点誤差だけを許す。紙面照合の許容値ではない。
        if (end - start - item.size).abs() > f64::EPSILON * end.max(1.0) * 4.0 {
            return Err(Failure::input("image_page_metrics_mismatch"));
        }
        if let Some(prior) = result.last_mut()
            && prior.repeated == item.repeated
            && prior.end == start
        {
            prior.end = end;
        } else {
            if result.len() == 2 {
                return Err(Failure::input("invalid_image_page_axis"));
            }
            result.push(Segment {
                start,
                end,
                offset: item.offset,
                repeated: item.repeated,
            });
        }
        previous = item.index;
        page_end = item.offset + item.size;
        if !page_end.is_finite() {
            return Err(Failure::input("invalid_image_page_axis"));
        }
    }
    if result.is_empty() {
        return Err(Failure::input("empty_page_axis"));
    }
    Ok(result)
}
fn emu(value: u64) -> Result<f64> {
    if value > 27_273_042_316_900 {
        return Err(Failure::input("drawing_coordinate_out_of_range"));
    }
    Ok(value as f64 / 12700.0)
}
