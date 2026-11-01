//! 共有文字列とインライン文字列の書式境界を保持する。
use crate::xlsx::{
    diagnostic::{Failure, Result},
    styles::Font,
    workbook::CellValue,
    xml::{S, singleton},
};
use roxmltree::Node;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: Arc<str>,
    /// rPr がなければセルの書体。ある場合は独立した書体指定として保持する。
    /// 名前・サイズ不足をセル書体から推測して埋めない。
    pub font: Option<Font>,
}

fn text(node: Node<'_, '_>) -> Result<Arc<str>> {
    if node.children().any(|n| n.is_element())
        || node.attributes().any(|a| {
            a.namespace() != Some("http://www.w3.org/XML/1998/namespace")
                || a.name() != "space"
                || !["preserve", "default"].contains(&a.value())
        })
    {
        return Err(Failure::unsupported("string_text_structure"));
    }
    Ok(crate::xlsx::xstring::decode(node.text().unwrap_or_default())?.into())
}

/// 文字を連結してから解析し直すと失われる書式・空白・エスケープ境界を保存する。
pub fn parse(node: Node<'_, '_>) -> Result<CellValue> {
    if node.attributes().len() != 0 {
        return Err(Failure::unsupported("string_attribute"));
    }
    let plain = singleton(node, "t")?;
    singleton(node, "phoneticPr")?;
    let mut runs = Vec::new();
    for child in node.children().filter(|n| n.is_element()) {
        // セル ph=true は validate_cell で拒否する。共有文字列の読み情報は
        // 表示対象ではなく、基底文字列へ連結しない。インラインの対応範囲は維持する。
        if node.has_tag_name((S, "si")) && child.has_tag_name((S, "rPh")) {
            validate_phonetic(child)?;
            continue;
        }
        if node.has_tag_name((S, "si")) && child.has_tag_name((S, "phoneticPr")) {
            validate_phonetic_properties(child)?;
            continue;
        }
        if child.has_tag_name((S, "t")) {
            continue;
        }
        if !child.has_tag_name((S, "r")) || child.attributes().len() != 0 {
            return Err(Failure::unsupported("string_element"));
        }
        if plain.is_some() {
            return Err(Failure::input("conflicting_string_content"));
        }
        for field in child.children().filter(|n| n.is_element()) {
            if !field.has_tag_name((S, "rPr")) && !field.has_tag_name((S, "t")) {
                return Err(Failure::unsupported("rich_text_run_element"));
            }
        }
        let properties = singleton(child, "rPr")?;
        if let Some(properties) = properties {
            if properties.attributes().len() != 0 {
                return Err(Failure::unsupported("rich_text_property_attribute"));
            }
            // 既存の書体パーサーが読み取らない追加属性・入れ子も拒否する。
            for property in properties.children().filter(|n| n.is_element()) {
                singleton(properties, property.tag_name().name())?;
                if property.children().any(|n| n.is_element())
                    || property.attributes().any(|a| {
                        a.namespace().is_some()
                            || if property.has_tag_name((S, "color")) {
                                !["rgb", "indexed", "theme", "auto", "tint"].contains(&a.name())
                            } else {
                                a.name() != "val"
                            }
                    })
                {
                    return Err(Failure::unsupported("rich_text_property_structure"));
                }
            }
        }
        runs.push(Run {
            text: text(
                singleton(child, "t")?.ok_or_else(|| Failure::input("rich_text_missing_text"))?,
            )?,
            font: properties.map(Font::parse_run).transpose()?,
        });
        if runs.len() > 4096 {
            return Err(Failure::input("text_fragment_limit"));
        }
    }
    if runs.is_empty() {
        Ok(CellValue::Text(
            plain.map(text).transpose()?.unwrap_or_else(|| "".into()),
        ))
    } else {
        Ok(CellValue::RichText(runs.into()))
    }
}

fn validate_phonetic(node: Node<'_, '_>) -> Result<()> {
    if node
        .attributes()
        .any(|a| a.namespace().is_some() || !["sb", "eb"].contains(&a.name()))
        || node
            .children()
            .filter(|n| n.is_element())
            .any(|n| !n.has_tag_name((S, "t")))
    {
        return Err(Failure::unsupported("phonetic_structure"));
    }
    let start = node.attribute("sb").and_then(|v| v.parse::<u32>().ok());
    let end = node.attribute("eb").and_then(|v| v.parse::<u32>().ok());
    if !matches!((start, end), (Some(start), Some(end)) if start < end) {
        return Err(Failure::input("invalid_phonetic_range"));
    }
    text(singleton(node, "t")?.ok_or_else(|| Failure::input("phonetic_text_missing"))?)?;
    Ok(())
}

fn validate_phonetic_properties(node: Node<'_, '_>) -> Result<()> {
    if node.children().any(|n| n.is_element())
        || node.attributes().any(|a| {
            a.namespace().is_some() || !["fontId", "type", "alignment"].contains(&a.name())
        })
    {
        return Err(Failure::unsupported("phonetic_properties_structure"));
    }
    if node
        .attribute("fontId")
        .and_then(|s| s.parse::<u32>().ok())
        .is_none()
        || node.attribute("type").is_some_and(|s| {
            ![
                "halfwidthKatakana",
                "fullwidthKatakana",
                "Hiragana",
                "noConversion",
            ]
            .contains(&s)
        })
        || node
            .attribute("alignment")
            .is_some_and(|s| !["noControl", "left", "center", "distributed"].contains(&s))
    {
        return Err(Failure::input("invalid_phonetic_properties"));
    }
    Ok(())
}
