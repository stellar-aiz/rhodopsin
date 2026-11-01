//! ブック全体の組版と PDF 生成。未確定の文字配置規則は呼び出し側が明示する。
use crate::xlsx::{
    ConversionOptions, PreparedWorkbook,
    cell_text::Frame,
    diagnostic::{Failure, Result},
    drawing::Anchor,
    format::date::Locale,
    image_geometry::GridGeometry,
    layout::{LayoutLimits, Page},
    page_scene, pdf,
    placement::RangePlacement,
    theme::TextScript,
    warnings::Warnings,
    workbook::{Cell, Sheet},
    workbook_layout::BookLayout,
};

pub struct Environment<'a> {
    pub locale: Locale,
    pub script: TextScript<'a>,
    pub normal_script: TextScript<'a>,
    pub automatic_color: Option<[u8; 3]>,
    pub foreground: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
}
pub struct Limits {
    pub layout: LayoutLimits,
    /// items と text_bytes はブック全体の上限。work はページ単位。
    pub scene: page_scene::Limits,
    pub pdf: pdf::Limits,
    pub warnings: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            layout: Default::default(),
            scene: Default::default(),
            pdf: Default::default(),
            warnings: 250_000,
        }
    }
}
pub struct Rendered {
    pub bytes: Vec<u8>,
    pub layout: BookLayout,
    pub report: crate::xlsx::report::Report,
}

type BorderPolicy<'a> = &'a mut dyn FnMut(
    &Sheet,
    &Page,
    &crate::xlsx::page_borders::Edge,
) -> Result<crate::xlsx::page_borders::Stroke>;

impl PreparedWorkbook {
    /// 全ページ成功後にだけ PDF と配置情報を返す。ファイルは書かない。
    /// Excel 固有の余白・文字はみ出し・行送りのポリシーはまだ既定提供しない。
    pub fn render_with_policy(
        &self,
        options: &ConversionOptions,
        environment: &Environment<'_>,
        limits: &Limits,
        frame: impl FnMut(&Sheet, &Page, &Cell, &RangePlacement) -> Result<Frame>,
    ) -> Result<Rendered> {
        self.render_impl(options, environment, limits, false, frame, None)
    }

    /// 隣接セルの占有状況によるはみ出し・切り取りをページ組版へ接続する。
    /// frame が返す clip は置き換える。内容矩形・倍率・行送りは呼び出し元が決める。
    /// Excel の印刷結果との照合は未完了で、CLI の既定変換にはまだ使用しない。
    pub fn render_with_cell_overflow(
        &self,
        options: &ConversionOptions,
        environment: &Environment<'_>,
        limits: &Limits,
        frame: impl FnMut(&Sheet, &Page, &Cell, &RangePlacement) -> Result<Frame>,
    ) -> Result<Rendered> {
        self.render_impl(options, environment, limits, true, frame, None)
    }

    /// 文字配置と罫線の描画規則を明示して全ページを描画する。
    /// はみ出しのclipはセル占有状況で決める。罫線の値は印刷倍率適用後のポイント。
    /// Excelの既定罫線規則・交点・競合優先順位を提供するものではない。
    pub fn render_with_borders(
        &self,
        options: &ConversionOptions,
        environment: &Environment<'_>,
        limits: &Limits,
        frame: impl FnMut(&Sheet, &Page, &Cell, &RangePlacement) -> Result<Frame>,
        mut border: impl FnMut(
            &Sheet,
            &Page,
            &crate::xlsx::page_borders::Edge,
        ) -> Result<crate::xlsx::page_borders::Stroke>,
    ) -> Result<Rendered> {
        self.render_impl(options, environment, limits, true, frame, Some(&mut border))
    }

    fn render_impl(
        &self,
        options: &ConversionOptions,
        environment: &Environment<'_>,
        limits: &Limits,
        cell_overflow: bool,
        mut frame: impl FnMut(&Sheet, &Page, &Cell, &RangePlacement) -> Result<Frame>,
        mut border: Option<BorderPolicy<'_>>,
    ) -> Result<Rendered> {
        let mut layout_limits = limits.layout.clone();
        layout_limits.pages = layout_limits.pages.min(limits.pdf.pages);
        let layout = self.paginate(options, environment.normal_script, &layout_limits)?;
        let mut context = self.text_context(
            options,
            environment.locale,
            environment.script,
            limits.warnings,
        )?;
        context.colors.automatic = environment.automatic_color;
        context.normal_script = environment.normal_script;
        context.colors.foreground = environment.foreground;
        context.colors.background = environment.background;
        let mut warnings = Warnings::default();
        let mut scenes = Vec::new();
        let mut remaining_items = limits.scene.items.min(limits.pdf.items);
        let mut remaining_bytes = limits.scene.text_bytes.min(limits.pdf.text_bytes);
        for pages in layout.pages.chunk_by(|a, b| a.sheet_index == b.sheet_index) {
            let index = self
                .workbook
                .sheets
                .binary_search_by_key(&pages[0].sheet_index, |sheet| sheet.index)
                .map_err(|_| Failure::input("page_sheet_mismatch"))?;
            let sheet = &self.workbook.sheets[index];
            let occupancy = cell_overflow
                .then(|| crate::xlsx::text_overflow::Occupancy::new(sheet, limits.scene.work))
                .transpose()?;
            if sheet.pictures.len() > limits.scene.work {
                return Err(Failure::input("page_scene_work_limit"));
            }
            let pictures: Vec<_> = sheet
                .pictures
                .iter()
                .filter(|p| p.print && !p.hidden)
                .collect();
            let geometry = if pictures.is_empty() {
                None
            } else {
                let mut last_column = pages
                    .iter()
                    .filter_map(|p| p.columns.last())
                    .map(|item| item.index)
                    .max()
                    .unwrap_or(1);
                let mut last_row = pages
                    .iter()
                    .filter_map(|p| p.rows.last())
                    .map(|item| item.index)
                    .max()
                    .unwrap_or(1);
                for picture in pictures {
                    let (from, to) = match picture.anchor {
                        Anchor::OneCell { from, .. } => (from, from),
                        Anchor::TwoCell { from, to, .. } => (from, to),
                    };
                    last_column = last_column.max(from.column).max(to.column);
                    last_row = last_row.max(from.row).max(to.row);
                }
                Some(
                    GridGeometry::new(
                        &sheet.grid,
                        last_column,
                        last_row,
                        layout.column_metrics.maximum_digit_width_pixels,
                        limits.layout.axis_items,
                    )
                    .map_err(|error| error.at(sheet.index, None))?,
                )
            };
            for page in pages {
                let mut overflow_work = limits.scene.work;
                let scene_limits = page_scene::Limits {
                    work: limits.scene.work,
                    items: remaining_items,
                    text_bytes: remaining_bytes,
                };
                let build_frame = |cell: &Cell, placement: &RangePlacement| {
                    let mut result = frame(sheet, page, cell, placement)?;
                    if let Some(occupancy) = &occupancy {
                        let alignment = &context
                            .styles
                            .validate_cell(cell.style)
                            .map_err(|error| error.at(sheet.index, Some(cell.address.clone())))?
                            .alignment;
                        result.clip = occupancy.clip_with_budget(
                            page,
                            cell,
                            placement,
                            alignment.horizontal,
                            alignment.wrap,
                            &mut overflow_work,
                        )?;
                    }
                    Ok(result)
                };
                let scene = if let Some(policy) = border.as_deref_mut() {
                    page_scene::plan_with_borders(
                        sheet,
                        page,
                        &context,
                        geometry.as_ref(),
                        &mut warnings,
                        &scene_limits,
                        page_scene::Policies {
                            frame: build_frame,
                            border: |edge: &crate::xlsx::page_borders::Edge| {
                                policy(sheet, page, edge)
                            },
                        },
                    )?
                } else {
                    page_scene::plan(
                        sheet,
                        page,
                        &context,
                        geometry.as_ref(),
                        &mut warnings,
                        &scene_limits,
                        build_frame,
                    )?
                };
                let (items, bytes) = scene.usage();
                remaining_items = remaining_items
                    .checked_sub(items)
                    .ok_or_else(|| Failure::input("page_scene_item_limit"))?;
                remaining_bytes = remaining_bytes
                    .checked_sub(bytes)
                    .ok_or_else(|| Failure::input("page_scene_text_limit"))?;
                scenes.push(scene);
            }
        }
        let pages: Vec<_> = scenes.iter().map(|scene| scene.pdf_page()).collect();
        let bytes = pdf::render(&pages, &self.fonts, &limits.pdf, &options.font_limits)?;
        let report = crate::xlsx::report::Report::rendered(
            &layout,
            &pages,
            &self.fonts,
            warnings.into_summaries(),
        );
        Ok(Rendered {
            bytes,
            layout,
            report,
        })
    }
}
