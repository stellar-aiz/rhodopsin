use crate::xlsx::diagnostic::{Failure, Result};
use roxmltree::Node;
use std::str::FromStr;

pub const S: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

pub fn boolean(node: Node<'_, '_>, key: &str, default: bool) -> Result<bool> {
    match node.attribute(key) {
        None => Ok(default),
        Some("1" | "true") => Ok(true),
        Some("0" | "false") => Ok(false),
        _ => Err(Failure::input("invalid_boolean_attribute")),
    }
}

pub fn number<T: FromStr>(node: Node<'_, '_>, key: &str, default: T) -> Result<T> {
    match node.attribute(key) {
        None => Ok(default),
        Some(value) => value
            .parse()
            .map_err(|_| Failure::input("invalid_numeric_attribute")),
    }
}

pub fn finite(node: Node<'_, '_>, key: &str, default: f64, min: f64, max: f64) -> Result<f64> {
    let value = number(node, key, default)?;
    if !value.is_finite() || value < min || value > max {
        return Err(Failure::input("numeric_attribute_out_of_range"));
    }
    Ok(value)
}

pub fn singleton<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Option<Node<'a, 'i>>> {
    let mut children = node.children().filter(|n| n.has_tag_name((S, name)));
    let result = children.next();
    if children.next().is_some() {
        return Err(Failure::input("duplicate_xml_element"));
    }
    Ok(result)
}

/// セル内の未対応情報を読み捨てず、値を取得する前に検査する。
pub fn validate_cell(node: Node<'_, '_>) -> Result<()> {
    for attribute in node.attributes() {
        if attribute.namespace().is_some()
            || !["r", "s", "t", "cm", "vm", "ph"].contains(&attribute.name())
        {
            return Err(Failure::unsupported("cell_attribute"));
        }
    }
    for key in ["cm", "vm"] {
        if number::<u32>(node, key, 0)? != 0 {
            return Err(Failure::unsupported("cell_metadata"));
        }
    }
    if boolean(node, "ph", false)? {
        return Err(Failure::unsupported("phonetic_display"));
    }
    for child in node.children().filter(|n| n.is_element()) {
        if child.tag_name().namespace() != Some(S)
            || !["f", "v", "is"].contains(&child.tag_name().name())
        {
            return Err(Failure::unsupported("cell_element"));
        }
    }
    let value = singleton(node, "v")?;
    let formula = singleton(node, "f")?;
    let inline = singleton(node, "is")?;
    let inline_type = node.attribute("t") == Some("inlineStr");
    if inline_type && (value.is_some() || formula.is_some()) || !inline_type && inline.is_some() {
        return Err(Failure::input("conflicting_cell_content"));
    }
    if let Some(value) = value
        && (value.attributes().len() != 0 || value.children().any(|n| n.is_element()))
    {
        return Err(Failure::unsupported("cell_value_structure"));
    }
    if let Some(formula) = formula
        && (formula.children().any(|n| n.is_element())
            || formula.attributes().any(|a| {
                a.namespace().is_some()
                    || ![
                        "t", "ref", "si", "ca", "aca", "bx", "dt2D", "dtr", "r1", "r2", "del1",
                        "del2",
                    ]
                    .contains(&a.name())
            }))
    {
        return Err(Failure::unsupported("formula_structure"));
    }
    if let Some(inline) = inline {
        if inline.attributes().len() != 0 {
            return Err(Failure::unsupported("inline_string_attribute"));
        }
        for child in inline.children().filter(|n| n.is_element()) {
            if !child.has_tag_name((S, "t")) && !child.has_tag_name((S, "r")) {
                return Err(Failure::unsupported("inline_string_element"));
            }
        }
        if let Some(text) = singleton(inline, "t")?
            && (text.children().any(|n| n.is_element())
                || text.attributes().any(|a| {
                    a.namespace() != Some("http://www.w3.org/XML/1998/namespace")
                        || a.name() != "space"
                        || !["preserve", "default"].contains(&a.value())
                }))
        {
            return Err(Failure::unsupported("inline_text_structure"));
        }
    }
    Ok(())
}
