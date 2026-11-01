//! 準備済みブックの標準書体計測と、可視シートのページ分割。
use crate::xlsx::{
    ConversionOptions, PreparedWorkbook,
    diagnostic::{Category, Failure, Result},
    layout::{LayoutLimits, Page, paginate},
    theme::{TextScript, resolve_font},
};
use std::collections::BTreeSet;

#[derive(Debug)]
pub struct ColumnMetrics {
    pub font_index: usize,
    pub maximum_digit_width_pixels: f64,
    /// 列幅の基準書体を代用した場合。成功報告へ引き継ぐ必要がある。
    pub substituted: bool,
}

#[derive(Debug)]
pub struct BookLayout {
    pub pages: Vec<Page>,
    pub column_metrics: ColumnMetrics,
}

impl PreparedWorkbook {
    /// 保存された印刷設定で全可視シートを分割する。
    /// 字形送り幅の 96 dpi 換算を用いる。Excel の画素丸めとの照合は未完了。
    /// 本 API の結果だけでは、紙面の再現性・セル描画の可否を保証しない。
    pub fn paginate(
        &self,
        options: &ConversionOptions,
        normal_script: TextScript<'_>,
        limits: &LayoutLimits,
    ) -> Result<BookLayout> {
        options.validate()?;
        let mut previous = 0;
        for sheet in &self.workbook.sheets {
            if sheet.index <= previous {
                return Err(Failure::input("invalid_sheet_order"));
            }
            previous = sheet.index;
        }
        let first = self
            .workbook
            .sheets
            .iter()
            .find(|sheet| !sheet.hidden)
            .ok_or_else(|| Failure::new(Category::LayoutFailure, "no_visible_sheets"))?;
        let metrics = || -> Result<ColumnMetrics> {
            let styles = self
                .workbook
                .styles
                .as_ref()
                .ok_or_else(|| Failure::unsupported("cell_styles_unresolved"))?;
            let index = styles
                .normal_font
                .ok_or_else(|| Failure::unsupported("normal_style_font_unresolved"))?;
            let font = styles
                .fonts
                .get(index)
                .ok_or_else(|| Failure::input("font_style_index"))?
                .as_ref()
                .map_err(Clone::clone)?;
            let request = resolve_font(font, self.workbook.theme.as_ref(), normal_script)?;
            let selected = self
                .fonts
                .select_request(&request, options.fallback_family.as_deref())?;
            let mdw =
                self.fonts.faces[selected.index].maximum_digit_width(request.points)? * 96.0 / 72.0;
            // 印刷倍率は paginate が適用する。ここで二重に掛けない。
            crate::xlsx::layout::column_points(1.0, mdw)?;
            Ok(ColumnMetrics {
                font_index: selected.index,
                maximum_digit_width_pixels: mdw,
                substituted: selected.substituted,
            })
        };
        let column_metrics = metrics().map_err(|error| error.at(first.index, None))?;
        let mut pages = Vec::new();
        let mut placements = 0usize;
        let mut axes = BTreeSet::new();
        for sheet in self.workbook.sheets.iter().filter(|sheet| !sheet.hidden) {
            let remaining = LayoutLimits {
                pages: limits.pages.saturating_sub(pages.len()),
                placements: limits.placements.saturating_sub(placements),
                axis_items: limits.axis_items,
            };
            let next = paginate(
                sheet.index,
                &sheet.grid,
                &sheet.print,
                column_metrics.maximum_digit_width_pixels,
                &remaining,
            )?;
            // 行列の Arc は同一シートの複数ページで共有される。実割当だけを数える。
            for page in &next {
                for axis in [&page.columns, &page.rows] {
                    if axes.insert((axis.as_ptr() as usize, axis.len())) {
                        placements = placements
                            .checked_add(axis.len())
                            .filter(|count| *count <= limits.placements)
                            .ok_or_else(|| {
                                Failure::new(Category::LayoutFailure, "layout_placement_limit")
                                    .at(sheet.index, None)
                            })?;
                    }
                }
            }
            pages.extend(next);
        }
        if pages.is_empty() {
            return Err(Failure::new(Category::LayoutFailure, "no_printable_pages"));
        }
        Ok(BookLayout {
            pages,
            column_metrics,
        })
    }
}
