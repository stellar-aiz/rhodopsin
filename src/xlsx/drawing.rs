use crate::xlsx::{
    diagnostic::{Failure, Result},
    package::{Limits, Package, relationship_path},
    workbook::relationships,
    xml::{boolean, number},
};
use image::{ImageDecoder, ImageReader};
use roxmltree::Node;
use std::{collections::BTreeMap, io::Cursor, sync::Arc};

const X: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    pub column: u32,
    pub row: u32,
    pub column_offset: u64,
    pub row_offset: u64,
}
impl Marker {
    pub fn address(self) -> String {
        let mut n = self.column;
        let mut letters = Vec::new();
        while n > 0 {
            n -= 1;
            letters.push(char::from(b'A' + (n % 26) as u8));
            n /= 26;
        }
        format!("{}{}", letters.iter().rev().collect::<String>(), self.row)
    }
}
#[derive(Clone, Debug)]
pub enum Anchor {
    OneCell {
        from: Marker,
        width_emu: u64,
        height_emu: u64,
    },
    TwoCell {
        from: Marker,
        to: Marker,
        resize_with_cells: bool,
    },
}
#[derive(Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
/// 元画像の各辺からのオフセット。100000 が 100%、負数は外側への拡張。
/// 百分率表記の小数部を整数へ丸めず保持する。
/// ラスター自体は変更せず、元画像を描く矩形とクリップへ変換する。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SourceCrop {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}
impl SourceCrop {
    pub(crate) fn fractions(self) -> Result<(f64, f64, f64, f64)> {
        let width = 100000.0 - self.left - self.right;
        let height = 100000.0 - self.top - self.bottom;
        if [self.left, self.top, self.right, self.bottom, width, height]
            .iter()
            .any(|value| !value.is_finite())
            || width <= 0.0
            || height <= 0.0
        {
            return Err(Failure::input("invalid_picture_crop"));
        }
        Ok((
            self.left / 100000.0,
            self.top / 100000.0,
            width / 100000.0,
            height / 100000.0,
        ))
    }
}
// ECMA-376 §22.9.2.9 の -?[0-9]+(\.[0-9]+)?% と、
// Transitional / Office の符号付き32ビット整数（1/1000%）を受け取る。
fn crop_percentage(value: Option<&str>) -> Result<f64> {
    let Some(value) = value else { return Ok(0.0) };
    let invalid = || Failure::input("invalid_picture_crop");
    let Some(percent) = value.strip_suffix('%') else {
        return value.parse::<i32>().map(f64::from).map_err(|_| invalid());
    };
    let unsigned = percent.strip_prefix('-').unwrap_or(percent);
    let (whole, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(w, f)| (w, Some(f)));
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|part| !digits(part)) {
        return Err(invalid());
    }
    let scaled = percent.parse::<f64>().map_err(|_| invalid())? * 1000.0;
    if !scaled.is_finite() {
        return Err(invalid());
    }
    Ok(scaled)
}

#[derive(Debug)]
pub struct Picture {
    pub anchor: Anchor,
    pub crop: SourceCrop,
    pub raster: Arc<Raster>,
    pub print: bool,
    pub hidden: bool,
}
#[derive(Default)]
pub struct ImageCache<'a> {
    entries: BTreeMap<&'a [u8], Arc<Raster>>,
    total: u64,
}

fn child<'a, 'i>(n: Node<'a, 'i>, ns: &str, name: &str) -> Result<Node<'a, 'i>> {
    let mut found = n.children().filter(|c| c.has_tag_name((ns, name)));
    let value = found
        .next()
        .ok_or_else(|| Failure::input("drawing_element_missing"))?;
    if found.next().is_some() {
        return Err(Failure::input("duplicate_drawing_element"));
    }
    Ok(value)
}
fn optional<'a, 'i>(n: Node<'a, 'i>, ns: &str, name: &str) -> Result<Option<Node<'a, 'i>>> {
    if n.children().any(|n| n.has_tag_name((ns, name))) {
        child(n, ns, name).map(Some)
    } else {
        Ok(None)
    }
}
fn allowed(n: Node<'_, '_>, ns: &str, names: &[&str], reason: &'static str) -> Result<()> {
    if n.children()
        .filter(|c| c.is_element())
        .any(|c| c.tag_name().namespace() != Some(ns) || !names.contains(&c.tag_name().name()))
    {
        return Err(Failure::unsupported(reason));
    }
    Ok(())
}
fn marker(n: Node<'_, '_>) -> Result<Marker> {
    allowed(
        n,
        X,
        &["col", "row", "colOff", "rowOff"],
        "drawing_marker_extension",
    )?;
    let value = |name| -> Result<u64> {
        child(n, X, name)?
            .text()
            .ok_or_else(|| Failure::input("drawing_marker_missing"))?
            .parse()
            .map_err(|_| Failure::input("invalid_drawing_marker"))
    };
    let column = value("col")?;
    let row = value("row")?;
    let column_offset = value("colOff")?;
    let row_offset = value("rowOff")?;
    if column >= 16384
        || row >= 1048576
        || column_offset > 27_273_042_316_900
        || row_offset > 27_273_042_316_900
    {
        return Err(Failure::input("drawing_marker_out_of_range"));
    }
    Ok(Marker {
        column: column as u32 + 1,
        row: row as u32 + 1,
        column_offset,
        row_offset,
    })
}

pub fn read_sheet<'a>(
    package: &'a Package,
    source: &str,
    root: Node<'_, '_>,
    limits: &Limits,
    cache: &mut ImageCache<'a>,
) -> Result<Vec<Picture>> {
    let Some(drawing) = crate::xlsx::xml::singleton(root, "drawing")? else {
        return Ok(Vec::new());
    };
    let rels = relationships(package, &relationship_path(source), source)?;
    let id = drawing
        .attribute((R, "id"))
        .ok_or_else(|| Failure::input("drawing_relationship_missing"))?;
    let (kind, target) = rels
        .get(id)
        .ok_or_else(|| Failure::input("drawing_relationship_missing"))?;
    if kind != &format!("{R}/drawing") {
        return Err(Failure::input("invalid_drawing_relationship"));
    }
    let doc = package.xml(target)?;
    if !doc.root_element().has_tag_name((X, "wsDr")) {
        return Err(Failure::input("invalid_drawing_root"));
    }
    // 図形だけの drawing は画像参照を持たないことがある。先に種類を検査する。
    for n in doc.descendants().filter(|n| n.is_element()) {
        let reason = match (n.tag_name().namespace(), n.tag_name().name()) {
            (Some(X), "sp") => Some("shape_or_text_box"),
            (Some(X), "graphicFrame") => Some("chart_or_graphic_frame"),
            (Some(X), "grpSp") => Some("group_shape"),
            (Some(X), "cxnSp") => Some("connector_shape"),
            _ => None,
        };
        if let Some(reason) = reason {
            let position = n
                .ancestors()
                .find(|n| {
                    n.has_tag_name((X, "oneCellAnchor")) || n.has_tag_name((X, "twoCellAnchor"))
                })
                .and_then(|n| child(n, X, "from").ok())
                .and_then(|n| marker(n).ok())
                .map(Marker::address);
            return Err(Failure::unsupported(reason).at(0, position));
        }
    }
    let picture_rels = relationships(package, &relationship_path(target), target)?;
    let mut pictures = Vec::new();
    for n in doc.root_element().children().filter(|n| n.is_element()) {
        let from = optional(n, X, "from")?.map(marker).transpose()?;
        let position = from.map(Marker::address);
        let picture = parse_picture(n, &picture_rels, package, cache, limits)
            .map_err(|e| e.at(0, position))?;
        pictures.push(picture);
    }
    Ok(pictures)
}

fn parse_picture<'a>(
    n: Node<'_, '_>,
    rels: &BTreeMap<String, (String, String)>,
    package: &'a Package,
    cache: &mut ImageCache<'a>,
    limits: &Limits,
) -> Result<Picture> {
    if n.tag_name().namespace() != Some(X) {
        return Err(Failure::unsupported("drawing_extension"));
    }
    let from = if matches!(n.tag_name().name(), "oneCellAnchor" | "twoCellAnchor") {
        marker(child(n, X, "from")?)?
    } else {
        return Err(Failure::unsupported("absolute_or_unknown_anchor"));
    };
    let anchor = match n.tag_name().name() {
        "oneCellAnchor" => {
            allowed(
                n,
                X,
                &["from", "ext", "pic", "clientData"],
                "drawing_object",
            )?;
            let ext = child(n, X, "ext")?;
            let width_emu = number(ext, "cx", 0u64)?;
            let height_emu = number(ext, "cy", 0u64)?;
            if width_emu == 0
                || height_emu == 0
                || width_emu > 27_273_042_316_900
                || height_emu > 27_273_042_316_900
            {
                return Err(Failure::input("invalid_picture_extent"));
            }
            Anchor::OneCell {
                from,
                width_emu,
                height_emu,
            }
        }
        "twoCellAnchor" => {
            allowed(n, X, &["from", "to", "pic", "clientData"], "drawing_object")?;
            let resize_with_cells = match n.attribute("editAs").unwrap_or("twoCell") {
                "twoCell" => true,
                "oneCell" => false,
                _ => return Err(Failure::unsupported("absolute_picture_anchor")),
            };
            let to = marker(child(n, X, "to")?)?;
            if (to.column, to.column_offset) <= (from.column, from.column_offset)
                || (to.row, to.row_offset) <= (from.row, from.row_offset)
            {
                return Err(Failure::input("invalid_picture_extent"));
            }
            Anchor::TwoCell {
                from,
                to,
                resize_with_cells,
            }
        }
        _ => return Err(Failure::unsupported("drawing_anchor")),
    };
    let pic = child(n, X, "pic")?;
    allowed(
        pic,
        X,
        &["nvPicPr", "blipFill", "spPr"],
        "picture_extension",
    )?;
    let props = child(pic, X, "nvPicPr")?;
    allowed(props, X, &["cNvPr", "cNvPicPr"], "picture_properties")?;
    let nv = child(props, X, "cNvPr")?;
    allowed(nv, A, &[], "picture_properties_extension")?;
    let hidden = boolean(nv, "hidden", false)?;
    let shape = child(pic, X, "spPr")?;
    allowed(shape, A, &["xfrm", "prstGeom"], "picture_shape_effect")?;
    if let Some(transform) = optional(shape, A, "xfrm")? {
        if number(transform, "rot", 0i64)? != 0
            || boolean(transform, "flipH", false)?
            || boolean(transform, "flipV", false)?
        {
            return Err(Failure::unsupported("picture_transform"));
        }
        allowed(transform, A, &["off", "ext"], "picture_transform")?;
    }
    if let Some(geometry) = optional(shape, A, "prstGeom")? {
        if geometry.attribute("prst") != Some("rect") {
            return Err(Failure::unsupported("picture_nonrectangular_geometry"));
        }
        allowed(geometry, A, &["avLst"], "picture_geometry")?;
        if geometry.descendants().any(|n| n.has_tag_name((A, "gd"))) {
            return Err(Failure::unsupported("picture_geometry_adjustment"));
        }
    }
    let fill = child(pic, X, "blipFill")?;
    allowed(
        fill,
        A,
        &["blip", "srcRect", "stretch"],
        "picture_fill_effect",
    )?;
    if boolean(fill, "rotWithShape", false)? {
        return Err(Failure::unsupported("picture_fill_rotation"));
    }
    let crop = if let Some(rect) = optional(fill, A, "srcRect")? {
        allowed(rect, A, &[], "picture_crop_extension")?;
        if rect
            .attributes()
            .any(|a| a.namespace().is_some() || !["l", "t", "r", "b"].contains(&a.name()))
        {
            return Err(Failure::unsupported("picture_crop_extension"));
        }
        SourceCrop {
            left: crop_percentage(rect.attribute("l"))?,
            top: crop_percentage(rect.attribute("t"))?,
            right: crop_percentage(rect.attribute("r"))?,
            bottom: crop_percentage(rect.attribute("b"))?,
        }
    } else {
        SourceCrop::default()
    };
    crop.fractions()?;
    let stretch = child(fill, A, "stretch")?;
    allowed(stretch, A, &["fillRect"], "picture_stretch_extension")?;
    if let Some(rect) = optional(stretch, A, "fillRect")? {
        for side in ["l", "r", "t", "b"] {
            if number(rect, side, 0i64)? != 0 {
                return Err(Failure::unsupported("picture_fill_rect"));
            }
        }
    }
    let blip = child(fill, A, "blip")?;
    allowed(blip, A, &[], "picture_image_effect")?;
    if blip.attribute((R, "link")).is_some() {
        return Err(Failure::unsupported("external_picture"));
    }
    let id = blip
        .attribute((R, "embed"))
        .ok_or_else(|| Failure::input("picture_relationship_missing"))?;
    let (kind, path) = rels
        .get(id)
        .ok_or_else(|| Failure::input("picture_relationship_missing"))?;
    if kind != &format!("{R}/image") {
        return Err(Failure::input("invalid_picture_relationship"));
    }
    let client = child(n, X, "clientData")?;
    let print = boolean(client, "fPrintsWithSheet", true)?;
    let raster = cache.load(package, path, limits)?;
    Ok(Picture {
        anchor,
        crop,
        raster,
        print,
        hidden,
    })
}

impl<'a> ImageCache<'a> {
    fn load(&mut self, package: &'a Package, path: &str, limits: &Limits) -> Result<Arc<Raster>> {
        let bytes = package.bytes(path)?;
        if self.total > limits.total_image_bytes {
            return Err(Failure::input("image_expansion_limit"));
        }
        if let Some(image) = self.entries.get(bytes) {
            if image.rgba.len() as u64 > limits.image_bytes
                || image.width > limits.image_dimension
                || image.height > limits.image_dimension
            {
                return Err(Failure::input("image_expansion_limit"));
            }
            return Ok(image.clone());
        }
        let remaining = limits
            .total_image_bytes
            .saturating_sub(self.total)
            .min(limits.image_bytes);
        let raster = Arc::new(decode(bytes, limits.image_dimension, remaining)?);
        self.total = self
            .total
            .checked_add(raster.rgba.len() as u64)
            .ok_or_else(|| Failure::input("image_expansion_limit"))?;
        self.entries.insert(bytes, raster.clone());
        Ok(raster)
    }
}

pub fn decode(bytes: &[u8], max_dimension: u32, max_bytes: u64) -> Result<Raster> {
    let format =
        image::guess_format(bytes).map_err(|_| Failure::input("invalid_image_signature"))?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err(Failure::unsupported("image_format"));
    }
    validate_image_metadata(bytes, format)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_dimension);
    limits.max_image_height = Some(max_dimension);
    limits.max_alloc = Some(max_bytes);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(image_failure)?;
    let (width, height) = decoder.dimensions();
    let size = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|size| size.checked_mul(4));
    if width == 0
        || height == 0
        || width > max_dimension
        || height > max_dimension
        || size.is_none_or(|size| size > max_bytes)
    {
        return Err(Failure::input("image_expansion_limit"));
    }
    if !matches!(
        decoder.color_type(),
        image::ColorType::L8
            | image::ColorType::La8
            | image::ColorType::Rgb8
            | image::ColorType::Rgba8
    ) {
        return Err(Failure::unsupported("image_bit_depth"));
    }
    if decoder.icc_profile().map_err(image_failure)?.is_some() {
        return Err(Failure::unsupported("image_color_profile"));
    }
    if decoder.orientation().map_err(image_failure)? != image::metadata::Orientation::NoTransforms {
        return Err(Failure::unsupported("image_exif_orientation"));
    }
    let image = image::DynamicImage::from_decoder(decoder).map_err(image_failure)?;
    Ok(Raster {
        width,
        height,
        rgba: image.into_rgba8().into_raw(),
    })
}

fn image_failure(error: image::ImageError) -> Failure {
    match error {
        image::ImageError::Limits(_) => Failure::input("image_expansion_limit"),
        _ => Failure::input("invalid_image_data"),
    }
}

/// デコーダーが通常画像へ変換してしまう情報を先に検出する。
fn validate_image_metadata(bytes: &[u8], format: image::ImageFormat) -> Result<()> {
    let bad = || Failure::input("invalid_image_data");
    if format == image::ImageFormat::Png {
        let mut cursor = 8usize;
        let mut nonstandard_gamma = false;
        let mut custom_chromaticity = false;
        let mut srgb = false;
        while cursor < bytes.len() {
            let head = bytes
                .get(cursor..cursor.checked_add(8).ok_or_else(bad)?)
                .ok_or_else(bad)?;
            let length = u32::from_be_bytes(head[..4].try_into().map_err(|_| bad())?) as usize;
            let end = cursor
                .checked_add(12)
                .and_then(|p| p.checked_add(length))
                .ok_or_else(bad)?;
            if end > bytes.len() {
                return Err(bad());
            }
            let data = &bytes[cursor + 8..end - 4];
            match &head[4..8] {
                b"acTL" => return Err(Failure::unsupported("animated_image")),
                b"iCCP" => return Err(Failure::unsupported("image_color_profile")),
                b"cHRM" => custom_chromaticity = true,
                b"sRGB" => srgb = true,
                b"gAMA" => {
                    if data.len() != 4 {
                        return Err(bad());
                    }
                    nonstandard_gamma =
                        u32::from_be_bytes(data.try_into().map_err(|_| bad())?) != 45455;
                }
                b"IEND" => break,
                _ => {}
            }
            cursor = end;
        }
        if !srgb && (nonstandard_gamma || custom_chromaticity) {
            return Err(Failure::unsupported("image_color_space"));
        }
    } else {
        let mut cursor = 2usize;
        while cursor < bytes.len() {
            if bytes[cursor] != 0xff {
                return Err(bad());
            }
            while bytes.get(cursor) == Some(&0xff) {
                cursor += 1;
            }
            let marker = *bytes.get(cursor).ok_or_else(bad)?;
            cursor += 1;
            if marker == 0xda || marker == 0xd9 {
                break;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let length_bytes = bytes.get(cursor..cursor + 2).ok_or_else(bad)?;
            let length = u16::from_be_bytes(length_bytes.try_into().map_err(|_| bad())?) as usize;
            if length < 2 {
                return Err(bad());
            }
            let segment = bytes
                .get(cursor + 2..cursor.checked_add(length).ok_or_else(bad)?)
                .ok_or_else(bad)?;
            if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                let components = *segment.get(5).ok_or_else(bad)?;
                if !matches!(components, 1 | 3) {
                    return Err(Failure::unsupported("image_color_space"));
                }
            }
            cursor += length;
        }
    }
    Ok(())
}

#[cfg(test)]
mod crop_tests {
    use super::SourceCrop;

    #[test]
    fn 公開構造体の非有限値と範囲計算の桁あふれを拒否する() {
        for crop in [
            SourceCrop {
                left: f64::NAN,
                ..Default::default()
            },
            SourceCrop {
                right: f64::INFINITY,
                ..Default::default()
            },
            SourceCrop {
                top: f64::NEG_INFINITY,
                ..Default::default()
            },
            SourceCrop {
                bottom: f64::NAN,
                ..Default::default()
            },
            SourceCrop {
                left: -f64::MAX,
                right: -f64::MAX,
                ..Default::default()
            },
            SourceCrop {
                top: -f64::MAX,
                bottom: -f64::MAX,
                ..Default::default()
            },
        ] {
            assert_eq!(crop.fractions().unwrap_err().reason, "invalid_picture_crop");
        }
    }
}
