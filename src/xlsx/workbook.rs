use crate::xlsx::{
    diagnostic::{Failure, Result},
    package::{Limits, Package, resolve},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const P: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

#[derive(Debug)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub date_1904: bool,
    pub styles: Option<crate::xlsx::styles::Styles>,
    pub theme: Option<crate::xlsx::theme::Theme>,
}

#[derive(Debug)]
pub struct Sheet {
    pub index: usize,
    pub hidden: bool,
    pub cells: Vec<Cell>,
    pub grid: crate::xlsx::grid::Grid,
    pub print: crate::xlsx::print::PrintSettings,
    pub pictures: Vec<crate::xlsx::drawing::Picture>,
}

#[derive(Debug)]
pub struct Cell {
    pub address: String,
    pub style: usize,
    pub value: CellValue,
    pub numeric_lexeme: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    Blank,
    Number(f64),
    Text(Arc<str>),
    RichText(Arc<[crate::xlsx::rich_text::Run]>),
    Boolean(bool),
    Error(String),
}

/// 自由文をセル番地として診断に流さないため、Excel のグリッド範囲まで検証する。
pub fn cell_address(value: &str) -> Option<(u32, u32)> {
    let split = value.find(|c: char| !c.is_ascii_uppercase())?;
    let (col, row) = value.split_at(split);
    if col.is_empty()
        || col.len() > 3
        || row.is_empty()
        || row.starts_with('0')
        || !row.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let column = col
        .bytes()
        .fold(0u32, |n, b| n * 26 + u32::from(b - b'A' + 1));
    let row: u32 = row.parse().ok()?;
    (column <= 16384 && (1..=1_048_576).contains(&row)).then_some((column, row))
}

pub(crate) fn relationships(
    package: &Package,
    path: &str,
    source: &str,
) -> Result<BTreeMap<String, (String, String)>> {
    let doc = package.xml(path)?;
    if !doc.root_element().has_tag_name((P, "Relationships")) {
        return Err(Failure::input("invalid_relationships"));
    }
    let mut result = BTreeMap::new();
    for n in doc.root_element().children().filter(|n| n.is_element()) {
        if !n.has_tag_name((P, "Relationship")) {
            return Err(Failure::unsupported("relationship_extension"));
        }
        if n.attribute("TargetMode").is_some_and(|v| v != "Internal") {
            return Err(Failure::unsupported("external_relationship"));
        }
        let id = n
            .attribute("Id")
            .ok_or_else(|| Failure::input("relationship_id_missing"))?;
        let kind = n
            .attribute("Type")
            .ok_or_else(|| Failure::input("relationship_type_missing"))?;
        let target = resolve(
            source,
            n.attribute("Target")
                .ok_or_else(|| Failure::input("relationship_target_missing"))?,
        )?;
        if result
            .insert(id.to_string(), (kind.to_string(), target))
            .is_some()
        {
            return Err(Failure::input("duplicate_relationship_id"));
        }
    }
    Ok(result)
}

pub fn inspect(package: &Package, limits: &Limits) -> Result<Workbook> {
    let types = package.xml("[Content_Types].xml")?;
    if types
        .descendants()
        .filter_map(|n| n.attribute("ContentType"))
        .any(|s| s.contains("macroEnabled") || s.contains("vbaProject"))
    {
        return Err(Failure::input("macro_enabled_workbook"));
    }
    let roots = relationships(package, "_rels/.rels", "")?;
    let candidates: Vec<_> = roots
        .values()
        .filter(|(kind, _)| kind == &format!("{R}/officeDocument"))
        .collect();
    if candidates.len() != 1 {
        return Err(Failure::input("workbook_relationship_missing"));
    }
    let workbook_path = &candidates[0].1;
    let (dir, file) = workbook_path
        .rsplit_once('/')
        .unwrap_or(("", workbook_path));
    let relpath = if dir.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{dir}/_rels/{file}.rels")
    };
    let rels = relationships(package, &relpath, workbook_path)?;
    let mut styles = None;
    let mut theme = None;
    for (kind, path) in rels.values() {
        if kind == &format!("{R}/styles") {
            if styles.is_some() {
                return Err(Failure::input("duplicate_styles_part"));
            }
            styles = Some(crate::xlsx::styles::Styles::parse(
                package.xml(path)?.root_element(),
            )?);
        } else if kind == &format!("{R}/theme") {
            if theme.is_some() {
                return Err(Failure::input("duplicate_theme_part"));
            }
            theme = Some(crate::xlsx::theme::Theme::parse(
                package.xml(path)?.root_element(),
            )?);
        }
    }
    let doc = package.xml(workbook_path)?;
    if !doc.root_element().has_tag_name((S, "workbook")) {
        return Err(Failure::unsupported("workbook_namespace"));
    }
    let date_1904 = crate::xlsx::xml::singleton(doc.root_element(), "workbookPr")?
        .map(|n| crate::xlsx::xml::boolean(n, "date1904", false))
        .transpose()?
        .unwrap_or(false);
    for name in package.names() {
        let reason = if name.ends_with("vbaProject.bin") {
            Some("macro_enabled_workbook")
        } else if name.starts_with("xl/charts/") {
            Some("chart")
        } else if name.starts_with("xl/activeX/") || name.starts_with("xl/embeddings/") {
            Some("embedded_object")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(Failure::unsupported(reason));
        }
    }
    let mut strings: Vec<CellValue> = Vec::new();
    for (kind, path) in rels.values() {
        if kind == &format!("{R}/sharedStrings") {
            let sst = package.xml(path)?;
            for item in sst
                .root_element()
                .children()
                .filter(|n| n.has_tag_name((S, "si")))
            {
                strings.push(crate::xlsx::rich_text::parse(item)?);
                if strings.len() > limits.cells {
                    return Err(Failure::input("shared_string_limit"));
                }
            }
        }
    }
    let mut sheets = Vec::new();
    let mut image_cache = crate::xlsx::drawing::ImageCache::default();
    let mut total_cells = 0usize;
    let mut orientation = None;
    for n in doc.descendants().filter(|n| n.has_tag_name((S, "sheet"))) {
        let index = sheets.len() + 1;
        let id = n
            .attribute((R, "id"))
            .ok_or_else(|| Failure::input("sheet_relationship_missing").at(index, None))?;
        let (kind, path) = rels
            .get(id)
            .ok_or_else(|| Failure::input("sheet_relationship_missing").at(index, None))?;
        if kind != &format!("{R}/worksheet") {
            return Err(Failure::unsupported("sheet_type").at(index, None));
        }
        let hidden = match n.attribute("state").unwrap_or("visible") {
            "hidden" | "veryHidden" => true,
            "visible" => false,
            _ => return Err(Failure::input("invalid_sheet_state").at(index, None)),
        };
        let sheet = package.xml(path)?;
        if !sheet.root_element().has_tag_name((S, "worksheet")) {
            return Err(Failure::input("invalid_worksheet").at(index, None));
        }
        let grid = crate::xlsx::grid::Grid::parse(sheet.root_element()).map_err(|mut e| {
            if let Some(location) = &mut e.location {
                location.sheet_index = index;
                e
            } else {
                e.at(index, None)
            }
        })?;
        let mut print = crate::xlsx::print::PrintSettings::parse(sheet.root_element())
            .map_err(|e| e.at(index, None))?;
        apply_print_names(
            doc.root_element(),
            index,
            n.attribute("name")
                .ok_or_else(|| Failure::input("sheet_name_missing"))?,
            &mut print,
        )
        .map_err(|e| e.at(index, None))?;
        if !hidden {
            if orientation.is_some_and(|previous| previous != print.orientation) {
                return Err(Failure::unsupported("mixed_page_orientation").at(index, None));
            }
            orientation = Some(print.orientation);
        }
        let mut cells = Vec::new();
        let pictures = crate::xlsx::drawing::read_sheet(
            package,
            path,
            sheet.root_element(),
            limits,
            &mut image_cache,
        )
        .map_err(|mut e| {
            if let Some(location) = &mut e.location {
                location.sheet_index = index;
                e
            } else {
                e.at(index, None)
            }
        })?;
        let mut addresses = BTreeSet::new();
        for node in sheet.descendants().filter(|n| n.is_element()) {
            let reason = match node.tag_name().name() {
                "conditionalFormatting" => Some("conditional_formatting"),
                "tableParts" => Some("table_style"),
                "headerFooter" => Some("header_footer"),
                "legacyDrawing" | "legacyDrawingHF" => Some("legacy_drawing"),
                "extLst" | "AlternateContent" => Some("worksheet_extension"),
                _ => None,
            };
            if let Some(reason) = reason {
                return Err(Failure::unsupported(reason).at(index, None));
            }
            if !node.has_tag_name((S, "c")) {
                continue;
            }
            let address = node
                .attribute("r")
                .filter(|s| cell_address(s).is_some())
                .ok_or_else(|| Failure::input("invalid_cell_address").at(index, None))?;
            crate::xlsx::xml::validate_cell(node).map_err(|e| e.at(index, Some(address.into())))?;
            let parent = node.parent().filter(|n| n.has_tag_name((S, "row")));
            if parent.is_none_or(|row| {
                row.parent()
                    .is_none_or(|n| !n.has_tag_name((S, "sheetData")))
                    || row.attribute("r").and_then(|r| r.parse::<u32>().ok())
                        != cell_address(address).map(|(_, row)| row)
            }) {
                return Err(Failure::input("cell_row_mismatch").at(index, Some(address.into())));
            }
            if !addresses.insert(address) {
                return Err(Failure::input("duplicate_cell").at(index, Some(address.into())));
            }
            total_cells += 1;
            if total_cells > limits.cells {
                return Err(Failure::input("cell_limit").at(index, None));
            }
            let v = node.children().find(|n| n.has_tag_name((S, "v")));
            let formula = node.children().any(|n| n.has_tag_name((S, "f")));
            let raw = v.and_then(|n| n.text()).unwrap_or("");
            // 空の文字列キャッシュは有効だが、数値型の空要素は計算結果を持たない。
            if formula
                && (v.is_none() || (node.attribute("t").unwrap_or("n") == "n" && raw.is_empty()))
            {
                return Err(
                    Failure::input("formula_cached_value_missing").at(index, Some(address.into()))
                );
            }
            let bad = || Failure::input("invalid_cell_value").at(index, Some(address.into()));
            let value = match node.attribute("t").unwrap_or("n") {
                "n" if v.is_none() => CellValue::Blank,
                "n" => {
                    let number: f64 = raw.parse().map_err(|_| bad())?;
                    if !number.is_finite() {
                        return Err(bad());
                    }
                    CellValue::Number(number)
                }
                "s" => strings
                    .get(raw.parse::<usize>().map_err(|_| bad())?)
                    .ok_or_else(bad)?
                    .clone(),
                "str" => CellValue::Text(
                    crate::xlsx::xstring::decode(raw)
                        .map_err(|e| e.at(index, Some(address.into())))?
                        .into(),
                ),
                "inlineStr" => {
                    if formula {
                        return Err(bad());
                    }
                    match crate::xlsx::xml::singleton(node, "is")? {
                        Some(inline) => crate::xlsx::rich_text::parse(inline)
                            .map_err(|e| e.at(index, Some(address.into())))?,
                        None => CellValue::Text("".into()),
                    }
                }
                "b" => CellValue::Boolean(match raw {
                    "0" => false,
                    "1" => true,
                    _ => return Err(bad()),
                }),
                "e" => CellValue::Error(raw.into()),
                "d" => {
                    return Err(
                        Failure::unsupported("iso_date_cell").at(index, Some(address.into()))
                    );
                }
                _ => return Err(bad()),
            };
            let (column, row) = cell_address(address).ok_or_else(bad)?;
            let style = node
                .attribute("s")
                .map(|s| s.parse().map_err(|_| bad()))
                .transpose()?
                .unwrap_or_else(|| grid.style_for(column, row));
            if let Some(styles) = &styles {
                styles
                    .validate_cell(style)
                    .map_err(|e| e.at(index, Some(address.into())))?;
            }
            cells.push(Cell {
                address: address.into(),
                style,
                numeric_lexeme: if matches!(value, CellValue::Number(_)) {
                    Some(raw.into())
                } else {
                    None
                },
                value,
            });
        }
        sheets.push(Sheet {
            index,
            hidden,
            cells,
            grid,
            print,
            pictures,
        });
    }
    if sheets.is_empty() {
        return Err(Failure::input("no_worksheets"));
    }
    if let Some(styles) = &styles {
        styles.validate_all()?;
    }
    Ok(Workbook {
        sheets,
        date_1904,
        styles,
        theme,
    })
}

fn apply_print_names(
    root: roxmltree::Node<'_, '_>,
    index: usize,
    sheet_name: &str,
    print: &mut crate::xlsx::print::PrintSettings,
) -> Result<()> {
    use crate::xlsx::range::{CellRange, MAX_COLUMN, references, title_range};
    let Some(names) = crate::xlsx::xml::singleton(root, "definedNames")? else {
        return Ok(());
    };
    let mut seen = BTreeSet::new();
    for n in names
        .children()
        .filter(|n| n.has_tag_name((S, "definedName")))
    {
        let name = n.attribute("name").unwrap_or("");
        if !matches!(name, "_xlnm.Print_Area" | "_xlnm.Print_Titles") {
            continue;
        }
        let scope: usize = n
            .attribute("localSheetId")
            .ok_or_else(|| Failure::unsupported("global_print_range"))?
            .parse()
            .map_err(|_| Failure::input("invalid_print_scope"))?;
        if scope != index - 1 {
            continue;
        }
        if !seen.insert(name) {
            return Err(Failure::input("duplicate_print_name"));
        }
        for reference in references(n.text().unwrap_or(""), sheet_name)? {
            if name == "_xlnm.Print_Area" {
                print.areas.push(CellRange::parse(&reference)?);
            } else {
                let range = title_range(&reference)?;
                let destination = if range.first_column == 1 && range.last_column == MAX_COLUMN {
                    &mut print.repeat_rows
                } else {
                    &mut print.repeat_columns
                };
                if destination.replace(range).is_some() {
                    return Err(Failure::input("duplicate_print_title"));
                }
            }
        }
    }
    Ok(())
}
