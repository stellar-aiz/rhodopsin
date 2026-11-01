//! 背景・文字・画像を同一ページへ組み立てる。Excel 固有の文字余白は呼び出し側で確定する。
use crate::xlsx::{
    cell_paint,
    cell_text::{self, Context, Frame, Text},
    diagnostic::{Failure, Result},
    image_geometry::GridGeometry,
    layout::Page,
    pdf::{self, graphics::Paint},
    placement::RangePlacement,
    warnings::Warnings,
    workbook::{Cell, Sheet},
};

pub struct Limits {
    pub work: usize,
    pub items: usize,
    pub text_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            work: 2_000_000,
            items: 1_000_000,
            text_bytes: 32 * 1024 * 1024,
        }
    }
}

pub struct Scene<'a> {
    width: f64,
    height: f64,
    backgrounds: Vec<Paint<'static>>,
    texts: Vec<Text>,
    borders: Vec<Paint<'static>>,
    pictures: Vec<Paint<'a>>,
}
impl Scene<'_> {
    // plan で加算のオーバーフローと上限を検査済み。
    pub(crate) fn usage(&self) -> (usize, usize) {
        (
            self.backgrounds.len()
                + self.borders.len()
                + self.texts.len()
                + self
                    .texts
                    .iter()
                    .filter(|text| text.decoration().is_some())
                    .count()
                + self.pictures.len(),
            self.texts.iter().map(|text| text.pdf().text.len()).sum(),
        )
    }

    /// 背景、罫線、文字、画像の順で PDF 描画へ渡す。文字はパスへ変換しない。
    pub fn pdf_page(&self) -> pdf::Page<'_> {
        let items = self
            .backgrounds
            .iter()
            .cloned()
            .map(pdf::Item::Paint)
            .chain(self.borders.iter().cloned().map(pdf::Item::Paint))
            .chain(cell_text::pdf_items(&self.texts))
            .chain(self.pictures.iter().cloned().map(pdf::Item::Paint))
            .collect();
        pdf::Page {
            width: self.width,
            height: self.height,
            items,
        }
    }
}

/// ページ内の全レイヤーが成功したときだけ、セル警告を確定する。
/// 既定の罫線描画規則が未確定のため、印刷セルで使う罫線を位置付きで拒否する。
/// 本 API は CLI の完成済み変換経路ではなく、検証用の組版接続。
pub fn plan<'a>(
    sheet: &'a Sheet,
    page: &Page,
    context: &Context<'_>,
    geometry: Option<&GridGeometry>,
    warnings: &mut Warnings,
    limits: &Limits,
    frame: impl FnMut(&Cell, &RangePlacement) -> Result<Frame>,
) -> Result<Scene<'a>> {
    plan_impl(
        sheet,
        page,
        context,
        geometry,
        warnings,
        limits,
        (frame, None),
    )
}

/// 配置規則と罫線規則を明示する検証用接続。既定CLIへはまだ接続しない。
/// 罫線規則は印刷倍率適用後のポイント値を返す。
pub struct Policies<F, B> {
    pub frame: F,
    pub border: B,
}

pub fn plan_with_borders<'a>(
    sheet: &'a Sheet,
    page: &Page,
    context: &Context<'_>,
    geometry: Option<&GridGeometry>,
    warnings: &mut Warnings,
    limits: &Limits,
    mut policies: Policies<
        impl FnMut(&Cell, &RangePlacement) -> Result<Frame>,
        impl FnMut(&crate::xlsx::page_borders::Edge) -> Result<crate::xlsx::page_borders::Stroke>,
    >,
) -> Result<Scene<'a>> {
    plan_impl(
        sheet,
        page,
        context,
        geometry,
        warnings,
        limits,
        (policies.frame, Some(&mut policies.border)),
    )
}

type BorderPolicy<'a> = &'a mut dyn FnMut(
    &crate::xlsx::page_borders::Edge,
) -> Result<crate::xlsx::page_borders::Stroke>;

fn plan_impl<'a>(
    sheet: &'a Sheet,
    page: &Page,
    context: &Context<'_>,
    geometry: Option<&GridGeometry>,
    warnings: &mut Warnings,
    limits: &Limits,
    (frame, border): (
        impl FnMut(&Cell, &RangePlacement) -> Result<Frame>,
        Option<BorderPolicy<'_>>,
    ),
) -> Result<Scene<'a>> {
    if sheet.index != page.sheet_index {
        return Err(Failure::input("page_sheet_mismatch"));
    }
    if sheet.hidden {
        return Err(Failure::input("hidden_sheet_page").at(sheet.index, None));
    }
    if sheet.pictures.len() > limits.work {
        return Err(Failure::input("page_scene_work_limit"));
    }
    let borders = if let Some(policy) = border {
        let edges = crate::xlsx::page_borders::edges(
            sheet,
            page,
            context.styles,
            &context.colors,
            limits.work,
        )?;
        crate::xlsx::page_borders::paint(&edges, limits.work, limits.items, policy)?
    } else {
        crate::xlsx::page_borders::reject_used(sheet, page, context.styles, limits.work)?;
        Vec::new()
    };
    let backgrounds =
        cell_paint::page_fills(sheet, page, context.styles, &context.colors, limits.work)?;
    let mut pending = warnings.clone();
    let texts = cell_text::plan_page_limited(
        sheet,
        page,
        context,
        &mut pending,
        limits.work,
        limits.text_bytes,
        frame,
    )?;
    let mut item_count = backgrounds
        .len()
        .checked_add(borders.len())
        .and_then(|count| count.checked_add(texts.len()))
        .and_then(|count| {
            count.checked_add(
                texts
                    .iter()
                    .filter(|text| text.decoration().is_some())
                    .count(),
            )
        })
        .filter(|count| *count <= limits.items)
        .ok_or_else(|| Failure::input("page_scene_item_limit"))?;
    let mut pictures = Vec::new();
    let mut picture_work = 0usize;
    for picture in &sheet.pictures {
        if picture.hidden || !picture.print {
            continue;
        }
        picture_work = picture_work
            .checked_add(page.columns.len())
            .and_then(|count| count.checked_add(page.rows.len()))
            .filter(|count| *count <= limits.work)
            .ok_or_else(|| Failure::input("page_scene_work_limit"))?;
        let from = match picture.anchor {
            crate::xlsx::drawing::Anchor::OneCell { from, .. }
            | crate::xlsx::drawing::Anchor::TwoCell { from, .. } => from,
        };
        let address = from.address();
        let location = crate::xlsx::workbook::cell_address(&address).map(|_| address);
        let geometry = geometry.ok_or_else(|| {
            Failure::unsupported("drawing_geometry_unresolved").at(sheet.index, location.clone())
        })?;
        let paints = geometry
            .picture_on_page(picture, page)
            .map_err(|error| error.at(sheet.index, location))?;
        item_count = item_count
            .checked_add(paints.len())
            .filter(|count| *count <= limits.items)
            .ok_or_else(|| Failure::input("page_scene_item_limit"))?;
        pictures.extend(paints);
    }
    *warnings = pending;
    Ok(Scene {
        width: page.paper.width,
        height: page.paper.height,
        backgrounds,
        borders,
        texts,
        pictures,
    })
}
