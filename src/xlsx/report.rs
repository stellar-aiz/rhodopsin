//! レンダリング結果の機械可読メタデータ。セル内容・シート名・パスは保持しない。
use crate::xlsx::{
    fonts::FontBook, layout::AxisItem, pdf, range::CellRange, warnings::Summary,
    workbook_layout::BookLayout,
};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub page_count: usize,
    pub pages: Vec<Page>,
    pub fonts: Vec<Font>,
    pub warnings: Vec<Summary>,
    pub layout_font_substituted: bool,
}
#[derive(Debug, Serialize)]
pub struct Page {
    pub page_index: usize,
    pub width_points: f64,
    pub height_points: f64,
    pub sheet_index: usize,
    pub area_index: usize,
    pub body: Range,
    pub repeated_rows: Option<AxisRange>,
    pub repeated_columns: Option<AxisRange>,
}
#[derive(Debug, Serialize)]
pub struct Range {
    pub first_cell: String,
    pub last_cell: String,
}
#[derive(Debug, Serialize)]
pub struct AxisRange {
    pub first: u32,
    pub last: u32,
}
#[derive(Debug, Serialize)]
pub struct Font {
    pub source_index: usize,
    pub families: Vec<String>,
    pub postscript_name: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub embedded: bool,
    pub subset: bool,
    pub to_unicode: bool,
}
fn address(column: u32, row: u32) -> String {
    crate::xlsx::drawing::Marker {
        column,
        row,
        column_offset: 0,
        row_offset: 0,
    }
    .address()
}
impl From<CellRange> for Range {
    fn from(range: CellRange) -> Self {
        Self {
            first_cell: address(range.first_column, range.first_row),
            last_cell: address(range.last_column, range.last_row),
        }
    }
}
fn repeated(axis: &[AxisItem]) -> Option<AxisRange> {
    let mut items = axis.iter().filter(|item| item.repeated);
    let first = items.next()?.index;
    Some(AxisRange {
        first,
        last: items.next_back().map_or(first, |item| item.index),
    })
}
impl Report {
    /// pdf::render の成功後にだけ呼ぶ。フォントは成功した描画命令から列挙する。
    pub(crate) fn rendered(
        layout: &BookLayout,
        pages: &[pdf::Page<'_>],
        fonts: &FontBook,
        warnings: Vec<Summary>,
    ) -> Self {
        let used: BTreeSet<_> = pages
            .iter()
            .flat_map(|page| page.texts())
            .map(|text| text.font)
            .collect();
        Self {
            schema_version: 1,
            page_count: pages.len(),
            pages: layout
                .pages
                .iter()
                .enumerate()
                .map(|(index, page)| Page {
                    page_index: index + 1,
                    width_points: page.paper.width,
                    height_points: page.paper.height,
                    sheet_index: page.sheet_index,
                    area_index: page.area_index,
                    body: page.body.into(),
                    repeated_rows: repeated(&page.rows),
                    repeated_columns: repeated(&page.columns),
                })
                .collect(),
            fonts: used
                .into_iter()
                .map(|index| {
                    let font = &fonts.faces[index];
                    Font {
                        source_index: index,
                        families: font.families.iter().cloned().collect(),
                        postscript_name: font.postscript_name.clone(),
                        bold: font.bold,
                        italic: font.italic,
                        embedded: true,
                        subset: true,
                        to_unicode: true,
                    }
                })
                .collect(),
            warnings,
            layout_font_substituted: layout.column_metrics.substituted,
        }
    }
}
