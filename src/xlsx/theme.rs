use crate::xlsx::diagnostic::{Failure, Result};
use roxmltree::Node;
use std::collections::BTreeMap;

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

#[derive(Clone, Debug, Default)]
pub struct ThemeFonts {
    pub latin: String,
    pub east_asian: String,
    pub complex: String,
    pub scripts: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct Theme {
    /// SpreadsheetML の theme インデックス順（lt1, dk1, lt2, dk2）。
    pub colors: [[u8; 3]; 12],
    pub major: ThemeFonts,
    pub minor: ThemeFonts,
}

pub fn rgb(value: &str) -> Result<[u8; 3]> {
    if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Failure::input("invalid_rgb_color"));
    }
    let channel = |offset| {
        u8::from_str_radix(&value[offset..offset + 2], 16)
            .map_err(|_| Failure::input("invalid_rgb_color"))
    };
    Ok([channel(0)?, channel(2)?, channel(4)?])
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Node<'a, 'i>> {
    let mut matches = node.children().filter(|n| n.has_tag_name((A, name)));
    let result = matches
        .next()
        .ok_or_else(|| Failure::input("theme_element_missing"))?;
    if matches.next().is_some() {
        return Err(Failure::input("duplicate_theme_element"));
    }
    Ok(result)
}

impl Theme {
    pub fn parse(root: Node<'_, '_>) -> Result<Self> {
        if !root.has_tag_name((A, "theme")) {
            return Err(Failure::input("invalid_theme"));
        }
        let elements = child(root, "themeElements")?;
        let scheme = child(elements, "clrScheme")?;
        let mut colors = [[0; 3]; 12];
        for (index, name) in [
            "lt1", "dk1", "lt2", "dk2", "accent1", "accent2", "accent3", "accent4", "accent5",
            "accent6", "hlink", "folHlink",
        ]
        .iter()
        .enumerate()
        {
            let wrapper = child(scheme, name)?;
            let mut nodes = wrapper.children().filter(|n| n.is_element());
            let color = nodes
                .next()
                .ok_or_else(|| Failure::input("theme_color_missing"))?;
            if nodes.next().is_some() || color.children().any(|n| n.is_element()) {
                return Err(Failure::unsupported("theme_color_transform"));
            }
            let value = if color.has_tag_name((A, "srgbClr")) {
                color.attribute("val")
            } else if color.has_tag_name((A, "sysClr")) {
                color.attribute("lastClr")
            } else {
                return Err(Failure::unsupported("theme_color_type"));
            };
            colors[index] = rgb(value.ok_or_else(|| Failure::input("theme_color_missing"))?)?;
        }
        let fonts = child(elements, "fontScheme")?;
        Ok(Self {
            colors,
            major: ThemeFonts::parse(child(fonts, "majorFont")?)?,
            minor: ThemeFonts::parse(child(fonts, "minorFont")?)?,
        })
    }
}

impl ThemeFonts {
    fn parse(root: Node<'_, '_>) -> Result<Self> {
        let mut result = Self::default();
        for (name, destination) in [
            ("latin", &mut result.latin),
            ("ea", &mut result.east_asian),
            ("cs", &mut result.complex),
        ] {
            *destination = child(root, name)?
                .attribute("typeface")
                .ok_or_else(|| Failure::input("theme_font_missing"))?
                .to_string();
        }
        for node in root.children().filter(|n| n.has_tag_name((A, "font"))) {
            let script = node
                .attribute("script")
                .ok_or_else(|| Failure::input("theme_script_missing"))?;
            let family = node
                .attribute("typeface")
                .ok_or_else(|| Failure::input("theme_font_missing"))?;
            if result
                .scripts
                .insert(script.into(), family.into())
                .is_some()
            {
                return Err(Failure::input("duplicate_theme_script"));
            }
        }
        Ok(result)
    }
}

/// 文字体系の判定は呼び出し元が行う。漢字だけでは Jpan/Hans/Hant を決めない。
#[derive(Clone, Copy, Debug)]
pub enum TextScript<'a> {
    Latin,
    EastAsian(&'a str),
    Complex(&'a str),
    Supplemental(&'a str),
}

impl ThemeFonts {
    pub fn family(&self, script: TextScript<'_>) -> Result<&str> {
        let (primary, code) = match script {
            TextScript::Latin => (Some(self.latin.as_str()), None),
            TextScript::EastAsian(code) => (Some(self.east_asian.as_str()), Some(code)),
            TextScript::Complex(code) => (Some(self.complex.as_str()), Some(code)),
            TextScript::Supplemental(code) => (None, Some(code)),
        };
        if let Some(code) = code
            && (code.len() != 4 || !code.bytes().all(|c| c.is_ascii_alphabetic()))
        {
            return Err(Failure::input("invalid_theme_script"));
        }
        if let Some(primary) = primary.filter(|name| !name.is_empty()) {
            return Ok(primary);
        }
        code.and_then(|code| self.scripts.get(code))
            .filter(|name| !name.is_empty())
            .map(String::as_str)
            .ok_or_else(|| {
                Failure::new(
                    crate::xlsx::diagnostic::Category::MissingFont,
                    "theme_font_unresolved",
                )
            })
    }
}

pub struct FontRequest<'a> {
    pub family: &'a str,
    pub points: f64,
    pub bold: bool,
    pub italic: bool,
}

/// テーマ参照を解決する。指定された書体の有無は FontBook で別途検査する。
pub fn resolve_font<'a>(
    font: &'a crate::xlsx::styles::Font,
    theme: Option<&'a Theme>,
    script: TextScript<'_>,
) -> Result<FontRequest<'a>> {
    use crate::xlsx::styles::FontScheme;
    let family = match font.scheme {
        FontScheme::None => font
            .family
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Failure::new(
                    crate::xlsx::diagnostic::Category::MissingFont,
                    "font_family_missing",
                )
            })?,
        scheme => {
            let theme = theme.ok_or_else(|| Failure::input("theme_missing"))?;
            match scheme {
                FontScheme::Major => &theme.major,
                FontScheme::Minor => &theme.minor,
                FontScheme::None => unreachable!(),
            }
            .family(script)?
        }
    };
    let points = font
        .size
        .filter(|s| s.is_finite() && (1.0..=409.0).contains(s))
        .ok_or_else(|| Failure::input("font_size_missing_or_invalid"))?;
    Ok(FontRequest {
        family,
        points,
        bold: font.bold,
        italic: font.italic,
    })
}
