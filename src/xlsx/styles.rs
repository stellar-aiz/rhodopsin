use crate::xlsx::{
    diagnostic::{Failure, Result},
    theme,
    xml::{S, boolean, finite, number, singleton},
};
use roxmltree::Node;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum ColorSource {
    Rgb([u8; 3]),
    Theme(u8),
    Indexed(u8),
    Automatic,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Color {
    pub source: ColorSource,
    pub tint: f64,
}
impl Default for Color {
    fn default() -> Self {
        Self {
            source: ColorSource::Automatic,
            tint: 0.0,
        }
    }
}

impl Color {
    pub fn parse(n: Node<'_, '_>) -> Result<Self> {
        let count = ["rgb", "theme", "indexed", "auto"]
            .iter()
            .filter(|k| n.attribute(**k).is_some())
            .count();
        if count != 1 {
            return Err(Failure::input("ambiguous_color"));
        }
        let source = if let Some(rgb) = n.attribute("rgb") {
            if rgb.len() != 8 || !rgb.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Failure::input("invalid_argb_color"));
            }
            // セル色の alpha は描画の透明度ではない。RGB の下位 24 bit を保持する。
            ColorSource::Rgb(theme::rgb(&rgb[2..])?)
        } else if n.attribute("theme").is_some() {
            let index: u8 = number(n, "theme", 0)?;
            if index > 11 {
                return Err(Failure::input("theme_color_index"));
            }
            ColorSource::Theme(index)
        } else if n.attribute("indexed").is_some() {
            let index: u8 = number(n, "indexed", 0)?;
            if index > 65 {
                return Err(Failure::input("indexed_color_index"));
            }
            ColorSource::Indexed(index)
        } else {
            if !boolean(n, "auto", false)? {
                return Err(Failure::input("invalid_automatic_color"));
            }
            ColorSource::Automatic
        };
        Ok(Self {
            source,
            tint: finite(n, "tint", 0.0, -1.0, 1.0)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontScheme {
    None,
    Major,
    Minor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Underline {
    None,
    Single,
    Double,
    SingleAccounting,
    DoubleAccounting,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: Option<String>,
    pub size: Option<f64>,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub color: Color,
    pub scheme: FontScheme,
}

fn children_allowed(n: Node<'_, '_>, allowed: &[&str], reason: &'static str) -> Result<()> {
    if n.children()
        .filter(|n| n.is_element())
        .any(|n| n.tag_name().namespace() != Some(S) || !allowed.contains(&n.tag_name().name()))
    {
        return Err(Failure::unsupported(reason));
    }
    Ok(())
}
fn flag(n: Node<'_, '_>, key: &str) -> Result<bool> {
    singleton(n, key)?
        .map(|n| boolean(n, "val", true))
        .transpose()
        .map(|v| v.unwrap_or(false))
}

// スライサー・タイムライン本体はシート拡張・描画の検査で拒否する。
// ここではセルの書式を変更しない既定名だけの宣言に限って受理する。
// Excel for Mac 16.105 は両方の宣言を保存するブックすべてに書く。
const DEFAULT_STYLE_EXTENSIONS: [(&str, &str, &str, &str); 2] = [
    (
        "{EB79DEF2-80B8-43e5-95BD-54CBDDF9020C}",
        "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main",
        "slicerStyles",
        "defaultSlicerStyle",
    ),
    (
        "{9260A510-F301-46a8-8635-F512D64BE5F5}",
        "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main",
        "timelineStyles",
        "defaultTimelineStyle",
    ),
];

fn validate_extensions(root: Node<'_, '_>) -> Result<()> {
    let Some(list) = singleton(root, "extLst")? else {
        return Ok(());
    };
    let unsupported = || Failure::unsupported("stylesheet_extension");
    if list.attributes().len() != 0 {
        return Err(unsupported());
    }
    let mut seen = [false; DEFAULT_STYLE_EXTENSIONS.len()];
    for ext in list.children().filter(|n| n.is_element()) {
        if !ext.has_tag_name((S, "ext")) || ext.attributes().len() != 1 {
            return Err(unsupported());
        }
        let index = DEFAULT_STYLE_EXTENSIONS
            .iter()
            .position(|(uri, ..)| ext.attribute("uri") == Some(*uri))
            .ok_or_else(unsupported)?;
        if std::mem::replace(&mut seen[index], true) {
            return Err(unsupported());
        }
        let (_, namespace, tag, attribute) = DEFAULT_STYLE_EXTENSIONS[index];
        let mut children = ext.children().filter(|n| n.is_element());
        let styles = children.next().ok_or_else(unsupported)?;
        if children.next().is_some()
            || !styles.has_tag_name((namespace, tag))
            || styles.attributes().len() != 1
            || styles.children().any(|n| n.is_element())
            || styles.attribute(attribute).is_none_or(|name| {
                let len = name.chars().count();
                !(1..=255).contains(&len)
            })
        {
            return Err(unsupported());
        }
    }
    // 未知のデータをテキストノードとして読み飛ばさない。
    if list
        .descendants()
        .filter(|n| n.is_text())
        .any(|n| n.text().is_some_and(|s| !s.trim().is_empty()))
    {
        return Err(unsupported());
    }
    Ok(())
}

impl Font {
    fn parse(n: Node<'_, '_>) -> Result<Self> {
        Self::parse_named(n, "name")
    }

    pub(crate) fn parse_run(n: Node<'_, '_>) -> Result<Self> {
        Self::parse_named(n, "rFont")
    }

    fn parse_named(n: Node<'_, '_>, name: &str) -> Result<Self> {
        children_allowed(
            n,
            &[
                name,
                "sz",
                "b",
                "i",
                "u",
                "color",
                "scheme",
                "family",
                "charset",
                "strike",
                "outline",
                "shadow",
                "condense",
                "extend",
                "vertAlign",
            ],
            "font_extension",
        )?;
        for key in ["strike", "outline", "shadow", "condense", "extend"] {
            if flag(n, key)? {
                return Err(Failure::unsupported("font_effect"));
            }
        }
        if let Some(v) = singleton(n, "vertAlign")?
            && v.attribute("val") != Some("baseline")
        {
            return Err(Failure::unsupported("font_vertical_alignment"));
        }
        let family = singleton(n, name)?
            .map(|n| {
                n.attribute("val")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| Failure::input("font_name_missing"))
            })
            .transpose()?;
        let size = singleton(n, "sz")?
            .map(|n| finite(n, "val", 0.0, 1.0, 409.0))
            .transpose()?;
        let scheme = match singleton(n, "scheme")?
            .and_then(|n| n.attribute("val"))
            .unwrap_or("none")
        {
            "none" => FontScheme::None,
            "major" => FontScheme::Major,
            "minor" => FontScheme::Minor,
            _ => return Err(Failure::input("invalid_font_scheme")),
        };
        let underline = match singleton(n, "u")? {
            None => Underline::None,
            Some(u) => match u.attribute("val").unwrap_or("single") {
                "none" => Underline::None,
                "single" => Underline::Single,
                "double" => Underline::Double,
                "singleAccounting" => Underline::SingleAccounting,
                "doubleAccounting" => Underline::DoubleAccounting,
                _ => return Err(Failure::input("invalid_underline")),
            },
        };
        Ok(Self {
            family,
            size,
            bold: flag(n, "b")?,
            italic: flag(n, "i")?,
            underline,
            color: singleton(n, "color")?
                .map(Color::parse)
                .transpose()?
                .unwrap_or_default(),
            scheme,
        })
    }
}

#[derive(Clone, Debug)]
pub enum Fill {
    None,
    Gray125,
    Solid(Color),
}
impl Fill {
    fn parse(n: Node<'_, '_>) -> Result<Self> {
        if singleton(n, "gradientFill")?.is_some() {
            return Err(Failure::unsupported("gradient_fill"));
        }
        children_allowed(n, &["patternFill"], "fill_extension")?;
        let Some(pattern) = singleton(n, "patternFill")? else {
            return Ok(Self::None);
        };
        match pattern.attribute("patternType").unwrap_or("none") {
            "none" => Ok(Self::None),
            "gray125" => Ok(Self::Gray125),
            "solid" => Ok(Self::Solid(Color::parse(
                singleton(pattern, "fgColor")?
                    .ok_or_else(|| Failure::input("fill_color_missing"))?,
            )?)),
            _ => Err(Failure::unsupported("pattern_fill")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    Thin,
    Medium,
    Dashed,
    Dotted,
    Thick,
    Double,
    Hair,
    MediumDashed,
    DashDot,
    MediumDashDot,
    DashDotDot,
    MediumDashDotDot,
    SlantDashDot,
}
#[derive(Clone, Debug)]
pub struct BorderLine {
    pub style: LineStyle,
    pub color: Color,
}
#[derive(Clone, Debug)]
pub struct Border {
    pub left: Option<BorderLine>,
    pub right: Option<BorderLine>,
    pub top: Option<BorderLine>,
    pub bottom: Option<BorderLine>,
}

fn line(n: Node<'_, '_>) -> Result<Option<BorderLine>> {
    let style = match n.attribute("style").unwrap_or("none") {
        "none" => return Ok(None),
        "thin" => LineStyle::Thin,
        "medium" => LineStyle::Medium,
        "dashed" => LineStyle::Dashed,
        "dotted" => LineStyle::Dotted,
        "thick" => LineStyle::Thick,
        "double" => LineStyle::Double,
        "hair" => LineStyle::Hair,
        "mediumDashed" => LineStyle::MediumDashed,
        "dashDot" => LineStyle::DashDot,
        "mediumDashDot" => LineStyle::MediumDashDot,
        "dashDotDot" => LineStyle::DashDotDot,
        "mediumDashDotDot" => LineStyle::MediumDashDotDot,
        "slantDashDot" => LineStyle::SlantDashDot,
        _ => return Err(Failure::input("invalid_border_style")),
    };
    Ok(Some(BorderLine {
        style,
        color: singleton(n, "color")?
            .map(Color::parse)
            .transpose()?
            .unwrap_or_default(),
    }))
}
impl Border {
    fn parse(n: Node<'_, '_>) -> Result<Self> {
        children_allowed(
            n,
            &[
                "left",
                "right",
                "top",
                "bottom",
                "diagonal",
                "vertical",
                "horizontal",
                "start",
                "end",
            ],
            "border_extension",
        )?;
        if boolean(n, "diagonalUp", false)? || boolean(n, "diagonalDown", false)? {
            return Err(Failure::unsupported("diagonal_border"));
        }
        for key in ["diagonal", "vertical", "horizontal", "start", "end"] {
            if singleton(n, key)?
                .map(line)
                .transpose()?
                .flatten()
                .is_some()
            {
                return Err(Failure::unsupported("non_edge_border"));
            }
        }
        let side = |key| {
            singleton(n, key)?
                .map(line)
                .transpose()
                .map(Option::flatten)
        };
        Ok(Self {
            left: side("left")?,
            right: side("right")?,
            top: side("top")?,
            bottom: side("bottom")?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Horizontal {
    General,
    Left,
    Center,
    Right,
    Fill,
    Justify,
    CenterContinuous,
    Distributed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vertical {
    Top,
    Center,
    Bottom,
    Justify,
    Distributed,
}
#[derive(Clone, Debug)]
pub struct Alignment {
    pub horizontal: Horizontal,
    pub vertical: Vertical,
    pub wrap: bool,
    pub indent: u16,
}
impl Default for Alignment {
    fn default() -> Self {
        Self {
            horizontal: Horizontal::General,
            vertical: Vertical::Bottom,
            wrap: false,
            indent: 0,
        }
    }
}
impl Alignment {
    fn parse(n: Node<'_, '_>) -> Result<Self> {
        let rotation: u16 = number(n, "textRotation", 0)?;
        if rotation != 0 {
            return Err(Failure::unsupported(if rotation == 255 {
                "vertical_text"
            } else {
                "rotated_text"
            }));
        }
        if boolean(n, "shrinkToFit", false)? {
            return Err(Failure::unsupported("shrink_to_fit"));
        }
        if number(n, "readingOrder", 0u8)? != 0 {
            return Err(Failure::unsupported("explicit_reading_order"));
        }
        if number(n, "relativeIndent", 0i32)? != 0 || boolean(n, "justifyLastLine", false)? {
            return Err(Failure::unsupported("extended_alignment"));
        }
        let horizontal = match n.attribute("horizontal").unwrap_or("general") {
            "general" => Horizontal::General,
            "left" => Horizontal::Left,
            "center" => Horizontal::Center,
            "right" => Horizontal::Right,
            "fill" => Horizontal::Fill,
            "justify" => Horizontal::Justify,
            "centerContinuous" => Horizontal::CenterContinuous,
            "distributed" => Horizontal::Distributed,
            _ => return Err(Failure::input("invalid_horizontal_alignment")),
        };
        let vertical = match n.attribute("vertical").unwrap_or("bottom") {
            "top" => Vertical::Top,
            "center" => Vertical::Center,
            "bottom" => Vertical::Bottom,
            "justify" => Vertical::Justify,
            "distributed" => Vertical::Distributed,
            _ => return Err(Failure::input("invalid_vertical_alignment")),
        };
        let indent = number(n, "indent", 0u16)?;
        if indent > 250 {
            return Err(Failure::input("invalid_indent"));
        }
        Ok(Self {
            horizontal,
            vertical,
            wrap: boolean(n, "wrapText", false)?,
            indent,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CellStyle {
    pub font: usize,
    pub fill: usize,
    pub border: usize,
    pub number_format: u32,
    pub alignment: Alignment,
}
#[derive(Clone, Debug, Default)]
pub struct Styles {
    pub fonts: Vec<Result<Font>>,
    pub fills: Vec<Result<Fill>>,
    pub borders: Vec<Result<Border>>,
    pub cells: Vec<Result<CellStyle>>,
    pub number_formats: BTreeMap<u32, String>,
    pub indexed_colors: Vec<[u8; 3]>,
    /// builtinId=0 の名前付きスタイルが参照する書体。表示名には依存しない。
    pub normal_font: Option<usize>,
}

impl Styles {
    pub fn parse(root: Node<'_, '_>) -> Result<Self> {
        if !root.has_tag_name((S, "styleSheet")) {
            return Err(Failure::input("invalid_stylesheet"));
        }
        children_allowed(
            root,
            &[
                "numFmts",
                "fonts",
                "fills",
                "borders",
                "cellStyleXfs",
                "cellXfs",
                "cellStyles",
                "dxfs",
                "tableStyles",
                "colors",
                "extLst",
            ],
            "stylesheet_extension",
        )?;
        validate_extensions(root)?;
        let mut result = Self::default();
        for (container, item) in [("fonts", "font"), ("fills", "fill"), ("borders", "border")] {
            if let Some(parent) = singleton(root, container)? {
                children_allowed(parent, &[item], "style_collection_extension")?;
                for node in parent.children().filter(|n| n.is_element()) {
                    match item {
                        "font" => result.fonts.push(Font::parse(node)),
                        "fill" => result.fills.push(Fill::parse(node)),
                        _ => result.borders.push(Border::parse(node)),
                    }
                }
            }
        }
        if let Some(formats) = singleton(root, "numFmts")? {
            for n in formats.children().filter(|n| n.is_element()) {
                if !n.has_tag_name((S, "numFmt")) {
                    return Err(Failure::unsupported("number_format_extension"));
                }
                let id = number(n, "numFmtId", 0u32)?;
                let code = n
                    .attribute("formatCode")
                    .ok_or_else(|| Failure::input("number_format_code_missing"))?;
                // formatCode は ST_Xstring。XML 属性の復号後に一度だけ適用する。
                // 一文字が _xHHHH_ の 7 バイトになる場合も、復号前に割当を制限する。
                if code.len() > 4096 * 7 {
                    return Err(Failure::input("number_format_length"));
                }
                let code = crate::xlsx::xstring::decode(code)?;
                if code.len() > 4096 {
                    return Err(Failure::input("number_format_length"));
                }
                if result.number_formats.insert(id, code).is_some() {
                    return Err(Failure::input("duplicate_number_format"));
                }
            }
        }
        let base: Vec<_> = singleton(root, "cellStyleXfs")?
            .into_iter()
            .flat_map(|n| n.children())
            .filter(|n| n.has_tag_name((S, "xf")))
            .collect();
        if let Some(named) = singleton(root, "cellStyles")? {
            children_allowed(named, &["cellStyle"], "named_style_extension")?;
            for node in named.children().filter(|n| n.is_element()) {
                if node.attribute("builtinId").is_some() && number(node, "builtinId", 0u32)? == 0 {
                    if result.normal_font.is_some() {
                        return Err(Failure::input("duplicate_normal_style"));
                    }
                    let index = node
                        .attribute("xfId")
                        .ok_or_else(|| Failure::input("normal_style_xf_missing"))?
                        .parse::<usize>()
                        .map_err(|_| Failure::input("base_style_index"))?;
                    let xf = *base
                        .get(index)
                        .ok_or_else(|| Failure::input("base_style_index"))?;
                    let font = number(xf, "fontId", 0usize)?;
                    if font >= result.fonts.len() {
                        return Err(Failure::input("font_style_index"));
                    }
                    result.normal_font = Some(font);
                }
            }
        }
        if let Some(xfs) = singleton(root, "cellXfs")? {
            children_allowed(xfs, &["xf"], "cell_format_extension")?;
            for n in xfs.children().filter(|n| n.is_element()) {
                result.cells.push(parse_xf(n, &base));
            }
        }
        if let Some(colors) = singleton(root, "colors")?
            && let Some(indexed) = singleton(colors, "indexedColors")?
        {
            for n in indexed.children().filter(|n| n.is_element()) {
                if !n.has_tag_name((S, "rgbColor")) {
                    return Err(Failure::unsupported("indexed_color_extension"));
                }
                match Color::parse(n)?.source {
                    ColorSource::Rgb(rgb) => result.indexed_colors.push(rgb),
                    _ => return Err(Failure::input("invalid_indexed_color")),
                }
            }
            if result.indexed_colors.len() > 64 {
                return Err(Failure::input("indexed_color_count"));
            }
        }
        Ok(result)
    }

    pub fn validate_cell(&self, index: usize) -> Result<&CellStyle> {
        let style = self
            .cells
            .get(index)
            .ok_or_else(|| Failure::input("cell_style_index"))?
            .as_ref()
            .map_err(Clone::clone)?;
        self.fonts
            .get(style.font)
            .ok_or_else(|| Failure::input("font_style_index"))?
            .as_ref()
            .map_err(Clone::clone)?;
        let fill = self
            .fills
            .get(style.fill)
            .ok_or_else(|| Failure::input("fill_style_index"))?
            .as_ref()
            .map_err(Clone::clone)?;
        if matches!(fill, Fill::Gray125) {
            return Err(Failure::unsupported("pattern_fill"));
        }
        self.borders
            .get(style.border)
            .ok_or_else(|| Failure::input("border_style_index"))?
            .as_ref()
            .map_err(Clone::clone)?;
        // 通貨・会計などの地域依存番号は、保存された書式コードを必要とする。
        if matches!(style.number_format, 5..=8 | 23..=26 | 41..=44 | 63..=66)
            && !self.number_formats.contains_key(&style.number_format)
        {
            return Err(Failure::unsupported("locale_number_format_missing"));
        }
        if style.number_format >= 164 && !self.number_formats.contains_key(&style.number_format) {
            return Err(Failure::input("number_format_missing"));
        }
        Ok(style)
    }

    pub fn validate_all(&self) -> Result<()> {
        for item in &self.fonts {
            item.as_ref().map_err(Clone::clone)?;
        }
        for item in &self.fills {
            item.as_ref().map_err(Clone::clone)?;
        }
        for item in &self.borders {
            item.as_ref().map_err(Clone::clone)?;
        }
        for index in 0..self.cells.len() {
            self.validate_cell(index)?;
        }
        Ok(())
    }
}

fn parse_xf(n: Node<'_, '_>, bases: &[Node<'_, '_>]) -> Result<CellStyle> {
    children_allowed(n, &["alignment", "protection"], "cell_format_extension")?;
    let base = if n.attribute("xfId").is_some() {
        let index = number(n, "xfId", 0usize)?;
        Some(
            *bases
                .get(index)
                .ok_or_else(|| Failure::input("base_style_index"))?,
        )
    } else {
        None
    };
    let component = |key: &str, apply: &str| -> Result<usize> {
        let source = if n.attribute(apply).is_some() && !boolean(n, apply, true)? {
            base.unwrap_or(n)
        } else {
            n
        };
        number(source, key, 0)
    };
    let alignment_source =
        if n.attribute("applyAlignment").is_some() && !boolean(n, "applyAlignment", true)? {
            base.unwrap_or(n)
        } else {
            n
        };
    Ok(CellStyle {
        font: component("fontId", "applyFont")?,
        fill: component("fillId", "applyFill")?,
        border: component("borderId", "applyBorder")?,
        number_format: component("numFmtId", "applyNumberFormat")?
            .try_into()
            .map_err(|_| Failure::input("number_format_id"))?,
        alignment: singleton(alignment_source, "alignment")?
            .map(Alignment::parse)
            .transpose()?
            .unwrap_or_default(),
    })
}
