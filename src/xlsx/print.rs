use crate::xlsx::{
    diagnostic::{Failure, Result},
    range::{CellRange, MAX_COLUMN, MAX_ROW},
    xml::{S, boolean, finite, number, singleton},
};
use roxmltree::Node;
use std::collections::BTreeSet;

fn leaf(n: Node<'_, '_>, attributes: &[&str], printer_relationship: bool) -> Result<()> {
    for attribute in n.attributes() {
        let known = match attribute.namespace() {
            None => attributes.contains(&attribute.name()),
            Some("http://schemas.openxmlformats.org/officeDocument/2006/relationships") => {
                printer_relationship && attribute.name() == "id"
            }
            _ => false,
        };
        if !known {
            return Err(Failure::unsupported("print_setting_attribute"));
        }
    }
    if n.children().any(|child| {
        child.is_element() || child.is_text() && child.text().is_some_and(|s| !s.trim().is_empty())
    }) {
        return Err(Failure::unsupported("print_setting_content"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageOrder {
    DownThenOver,
    OverThenDown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scaling {
    Percent(u16),
    Fit { width: u16, height: u16 },
}

#[derive(Clone, Debug)]
pub struct Margins {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub header: f64,
    pub footer: f64,
}

#[derive(Clone, Debug)]
pub struct PageBreak {
    pub after: u32,
    pub min: u32,
    pub max: u32,
}

#[derive(Clone, Debug)]
pub struct PrintSettings {
    /// 用紙が未指定の場合はプリンター依存。レイアウト時に補完せず拒否する。
    pub paper_size: Option<u16>,
    pub orientation: Orientation,
    pub order: PageOrder,
    pub scaling: Scaling,
    /// 保存されていない余白を推定しない。
    pub margins: Option<Margins>,
    pub horizontal_center: bool,
    pub vertical_center: bool,
    pub areas: Vec<CellRange>,
    pub repeat_rows: Option<CellRange>,
    pub repeat_columns: Option<CellRange>,
    pub row_breaks: Vec<PageBreak>,
    pub column_breaks: Vec<PageBreak>,
}

impl PrintSettings {
    pub fn parse(root: Node<'_, '_>) -> Result<Self> {
        let setup = singleton(root, "pageSetup")?;
        let mut result = Self {
            paper_size: None,
            orientation: Orientation::Portrait,
            order: PageOrder::DownThenOver,
            scaling: Scaling::Percent(100),
            margins: None,
            horizontal_center: false,
            vertical_center: false,
            areas: Vec::new(),
            repeat_rows: None,
            repeat_columns: None,
            row_breaks: Vec::new(),
            column_breaks: Vec::new(),
        };
        let fit = singleton(root, "sheetPr")?
            .map(|n| singleton(n, "pageSetUpPr"))
            .transpose()?
            .flatten()
            .map(|n| {
                leaf(n, &["autoPageBreaks", "fitToPage"], false)?;
                boolean(n, "autoPageBreaks", true)?;
                boolean(n, "fitToPage", false)
            })
            .transpose()?
            .unwrap_or(false);
        if let Some(n) = setup {
            leaf(
                n,
                &[
                    "paperSize",
                    "paperHeight",
                    "paperWidth",
                    "paperUnits",
                    "scale",
                    "firstPageNumber",
                    "fitToWidth",
                    "fitToHeight",
                    "pageOrder",
                    "orientation",
                    "usePrinterDefaults",
                    "blackAndWhite",
                    "draft",
                    "cellComments",
                    "useFirstPageNumber",
                    "errors",
                    "horizontalDpi",
                    "verticalDpi",
                    "copies",
                ],
                true,
            )?;
            result.orientation = match n.attribute("orientation").unwrap_or("portrait") {
                "portrait" => Orientation::Portrait,
                "landscape" => Orientation::Landscape,
                _ => return Err(Failure::unsupported("printer_default_orientation")),
            };
            result.order = match n.attribute("pageOrder").unwrap_or("downThenOver") {
                "downThenOver" => PageOrder::DownThenOver,
                "overThenDown" => PageOrder::OverThenDown,
                _ => return Err(Failure::input("invalid_page_order")),
            };
            if n.attribute("paperSize").is_some() {
                let id = number(n, "paperSize", 0u16)?;
                if id == 0 {
                    return Err(Failure::unsupported("printer_default_paper"));
                }
                result.paper_size = Some(id);
            }
            for attr in ["paperWidth", "paperHeight", "paperUnits"] {
                if n.attribute(attr).is_some() {
                    return Err(Failure::unsupported("custom_paper_size"));
                }
            }
            for (attr, reason) in [
                ("blackAndWhite", "black_and_white_print"),
                ("draft", "draft_print"),
                ("usePrinterDefaults", "printer_defaults"),
            ] {
                if boolean(n, attr, false)? {
                    return Err(Failure::unsupported(reason));
                }
            }
            if n.attribute("cellComments").is_some_and(|s| s != "none") {
                return Err(Failure::unsupported("printed_comments"));
            }
            if n.attribute("errors").is_some_and(|s| s != "displayed") {
                return Err(Failure::unsupported("print_error_replacement"));
            }
            let percent = number(n, "scale", 100u16)?;
            if !(10..=400).contains(&percent) {
                return Err(Failure::input("invalid_print_scale"));
            }
            result.scaling = if fit {
                Scaling::Fit {
                    width: number(n, "fitToWidth", 1u16)?,
                    height: number(n, "fitToHeight", 1u16)?,
                }
            } else {
                Scaling::Percent(percent)
            };
        } else if fit {
            result.scaling = Scaling::Fit {
                width: 1,
                height: 1,
            };
        }
        if let Some(n) = singleton(root, "pageMargins")? {
            leaf(
                n,
                &["left", "right", "top", "bottom", "header", "footer"],
                false,
            )?;
            for attr in ["left", "right", "top", "bottom", "header", "footer"] {
                if n.attribute(attr).is_none() {
                    return Err(Failure::input("page_margin_missing"));
                }
            }
            // OOXML はインチ。以後のモデルでは PDF と同じポイントに揃える。
            result.margins = Some(Margins {
                left: finite(n, "left", 0.0, 0.0, 100.0)? * 72.0,
                right: finite(n, "right", 0.0, 0.0, 100.0)? * 72.0,
                top: finite(n, "top", 0.0, 0.0, 100.0)? * 72.0,
                bottom: finite(n, "bottom", 0.0, 0.0, 100.0)? * 72.0,
                header: finite(n, "header", 0.0, 0.0, 100.0)? * 72.0,
                footer: finite(n, "footer", 0.0, 0.0, 100.0)? * 72.0,
            });
        }
        if let Some(n) = singleton(root, "printOptions")? {
            leaf(
                n,
                &[
                    "horizontalCentered",
                    "verticalCentered",
                    "headings",
                    "gridLines",
                    "gridLinesSet",
                ],
                false,
            )?;
            boolean(n, "gridLinesSet", true)?;
            result.horizontal_center = boolean(n, "horizontalCentered", false)?;
            result.vertical_center = boolean(n, "verticalCentered", false)?;
            if boolean(n, "headings", false)? {
                return Err(Failure::unsupported("printed_headings"));
            }
            if boolean(n, "gridLines", false)? {
                return Err(Failure::unsupported("printed_grid_lines"));
            }
        }
        result.row_breaks = parse_breaks(root, "rowBreaks", MAX_ROW, MAX_COLUMN)?;
        result.column_breaks = parse_breaks(root, "colBreaks", MAX_COLUMN, MAX_ROW)?;
        Ok(result)
    }
}

fn parse_breaks(
    root: Node<'_, '_>,
    name: &str,
    limit: u32,
    cross_limit: u32,
) -> Result<Vec<PageBreak>> {
    let mut result = Vec::new();
    let Some(parent) = singleton(root, name)? else {
        return Ok(result);
    };
    let mut seen = BTreeSet::new();
    for n in parent.children().filter(|n| n.is_element()) {
        if !n.has_tag_name((S, "brk")) {
            return Err(Failure::unsupported("page_break_extension"));
        }
        leaf(n, &["id", "min", "max", "man", "pt"], false)?;
        let after = number(n, "id", 0u32)?;
        let min = number(n, "min", 0u32)?;
        let max = number(n, "max", cross_limit - 1)?;
        if after == 0 || after > limit || min > max || max >= cross_limit || !seen.insert(after) {
            return Err(Failure::input("invalid_page_break"));
        }
        if boolean(n, "pt", false)? {
            return Err(Failure::unsupported("pivot_page_break"));
        }
        if boolean(n, "man", false)? {
            result.push(PageBreak { after, min, max });
        }
    }
    result.sort_by_key(|b| b.after);
    Ok(result)
}
