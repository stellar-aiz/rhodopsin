//! Focused PDF 1.4 backend using the existing scene and font libraries.
//! Reject unsupported nodes instead of silently changing their appearance.
use crate::DiagnosticCode as Code;
use anyhow::{Context, Result, bail, ensure};
use resvg::{
    tiny_skia::{PathSegment, Transform},
    usvg,
};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider,
};
use std::{collections::HashMap, fmt::Write};

#[derive(Default)]
struct Pen {
    data: String,
    x: f32,
    y: f32,
    start: (f32, f32),
}
impl Pen {
    fn from_path(path: &resvg::tiny_skia::Path) -> Self {
        let mut pen = Self::default();
        for seg in path.segments() {
            match seg {
                PathSegment::MoveTo(p) => pen.move_to(p.x, p.y),
                PathSegment::LineTo(p) => pen.line_to(p.x, p.y),
                PathSegment::QuadTo(c, p) => pen.quad_to(c.x, c.y, p.x, p.y),
                PathSegment::CubicTo(a, b, p) => pen.curve_to(a.x, a.y, b.x, b.y, p.x, p.y),
                PathSegment::Close => pen.close(),
            }
        }
        pen
    }
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        writeln!(self.data, "{x} {y} m").unwrap();
        self.start = (x, y);
        self.x = x;
        self.y = y;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        writeln!(self.data, "{x} {y} l").unwrap();
        self.x = x;
        self.y = y;
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.curve_to(
            self.x + (cx - self.x) * 2.0 / 3.0,
            self.y + (cy - self.y) * 2.0 / 3.0,
            x + (cx - x) * 2.0 / 3.0,
            y + (cy - y) * 2.0 / 3.0,
            x,
            y,
        );
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        writeln!(self.data, "{a} {b} {c} {d} {x} {y} c").unwrap();
        self.x = x;
        self.y = y;
    }
    fn close(&mut self) {
        self.data.push_str("h\n");
        (self.x, self.y) = self.start;
    }
}
fn matrix(t: Transform) -> String {
    format!("{} {} {} {} {} {}", t.sx, t.ky, t.kx, t.sy, t.tx, t.ty)
}
fn color(paint: &usvg::Paint, stroke: bool) -> Result<String> {
    let usvg::Paint::Color(c) = paint else {
        bail!("Gradient/pattern paint is not implemented")
    };
    Ok(format!(
        "{} {} {} {}\n",
        c.red as f32 / 255.0,
        c.green as f32 / 255.0,
        c.blue as f32 / 255.0,
        if stroke { "RG" } else { "rg" }
    ))
}
pub struct Pdf {
    objects: Vec<Vec<u8>>,
    pages: Vec<usize>,
    fonts: HashMap<String, (usize, f32)>,
    states: HashMap<String, usize>,
    object_bytes: usize,
    /// Pattern space of the content being written: the page's default space, or identity
    /// inside a transparency-group form.
    base: Transform,
    /// Scene location of the node being written, for diagnostics.
    location: String,
    notes: Vec<(Code, String, String)>,
    /// Pixels allocated by rasters so far, against `raster_budget`.
    raster_pixels: u64,
    raster_budget: u64,
}
/// Resources referenced by one content stream, by dictionary and name.
#[derive(Default)]
struct Res(std::collections::BTreeMap<(&'static str, String), usize>);
impl Res {
    fn add(&mut self, kind: &'static str, name: String, id: usize) {
        self.0.insert((kind, name), id);
    }
    fn dictionaries(&self) -> String {
        ["XObject", "ExtGState", "Pattern"]
            .iter()
            .map(|kind| {
                let entries: String = self
                    .0
                    .iter()
                    .filter(|((k, _), _)| k == kind)
                    .map(|((_, name), id)| format!("/{name} {id} 0 R "))
                    .collect();
                format!("/{kind} << {entries}>>")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}
/// Raster resolution for effects PDF cannot express: pixels per canvas unit (2 = 192 dpi).
const RASTER_SCALE: f32 = 2.0;
/// Largest raster, matching the decoded image pixel limit.
const RASTER_PIXELS: u64 = 16_777_216;
/// Pixels all rasters of one document may allocate, so many effects cannot multiply the cost.
const RASTER_TOTAL: u64 = 4 * RASTER_PIXELS;
const FILTER_NOTE: &str = "Filter effects are rasterized in PDF output";
fn blend_name(mode: usvg::BlendMode) -> &'static str {
    use usvg::BlendMode::*;
    match mode {
        Normal => "Normal",
        Multiply => "Multiply",
        Screen => "Screen",
        Overlay => "Overlay",
        Darken => "Darken",
        Lighten => "Lighten",
        ColorDodge => "ColorDodge",
        ColorBurn => "ColorBurn",
        HardLight => "HardLight",
        SoftLight => "SoftLight",
        Difference => "Difference",
        Exclusion => "Exclusion",
        Hue => "Hue",
        Saturation => "Saturation",
        Color => "Color",
        Luminosity => "Luminosity",
    }
}
/// Render `node` into `pixmap`, whose top-left corner is `origin` in canvas units, at `scale`
/// pixels per unit. `render_node` applies only the node's own transform after shifting by the
/// node's layer box, so the parent's transform is folded into the root transform around that
/// shift.
fn render_at(
    node: &usvg::Node,
    origin: resvg::tiny_skia::NonZeroRect,
    scale: f32,
    pixmap: &mut resvg::tiny_skia::Pixmap,
) -> Result<()> {
    let Some(bbox) = node.abs_layer_bounding_box() else {
        return Ok(());
    };
    let parent = match node {
        usvg::Node::Group(g) => g
            .abs_transform()
            .pre_concat(g.transform().invert().context("Singular group transform")?),
        usvg::Node::Path(p) => p.abs_transform(),
        usvg::Node::Image(i) => i.abs_transform(),
        usvg::Node::Text(t) => t.abs_transform(),
    };
    let root = Transform::from_scale(scale, scale)
        .pre_translate(-origin.x(), -origin.y())
        .pre_concat(parent)
        .pre_translate(bbox.x(), bbox.y());
    resvg::render_node(node, root, &mut pixmap.as_mut());
    Ok(())
}
/// Whether a filter's last step merges its result under the unfiltered source graphic.
fn keeps_source_on_top(filter: &usvg::filter::Filter) -> bool {
    filter.primitives().last().is_some_and(|p| match p.kind() {
        usvg::filter::Kind::Merge(m) => {
            m.inputs().last() == Some(&usvg::filter::Input::SourceGraphic)
        }
        _ => false,
    })
}
impl Pdf {
    pub fn new() -> Self {
        Self {
            objects: vec![vec![], vec![]],
            pages: vec![],
            fonts: HashMap::new(),
            states: HashMap::new(),
            object_bytes: 0,
            base: Transform::identity(),
            location: String::new(),
            notes: Vec::new(),
            raster_pixels: 0,
            raster_budget: RASTER_TOTAL,
        }
    }
    fn object(&mut self, data: impl Into<Vec<u8>>) -> usize {
        let data = data.into();
        self.object_bytes += data.len();
        self.objects.push(data);
        self.objects.len()
    }
    fn stream(&mut self, extra: &str, data: &[u8]) -> usize {
        let mut out = format!("<< {extra} /Length {} >>\nstream\n", data.len()).into_bytes();
        out.extend_from_slice(data);
        out.extend_from_slice(b"\nendstream");
        self.object(out)
    }
    fn budget(&self, content: usize) -> Result<()> {
        ensure!(
            self.object_bytes.saturating_add(content) <= 128 * 1024 * 1024,
            "PDF content exceeds 128 MiB limit"
        );
        ensure!(
            self.objects.len() <= 100_000,
            "PDF object count exceeds limit"
        );
        Ok(())
    }
    fn font(
        &mut self,
        tree: &usvg::Tree,
        face: usvg::fontdb::ID,
        gid: usvg::GlyphId,
        text: &str,
    ) -> Result<(usize, f32)> {
        ensure!(gid.0 != 0, "Missing PDF glyph for {text:?}");
        let key = format!("{}:{}:{}", face, gid.0, text);
        if let Some(value) = self.fonts.get(&key) {
            return Ok(*value);
        }
        ensure!(
            !text.is_empty(),
            "Empty glyph Unicode mapping (cluster needs dedicated handling)"
        );
        // stellar:debt(perf) one resource per glyph/text pair. upgrade: pack up to 255 codes per Type 3 font.
        let (path, upem, width, bounds) = tree
            .fontdb()
            .with_face_data(face, |data, index| -> Result<_> {
                let font = FontRef::from_index(data, index)?;
                ensure!(
                    ![b"COLR", b"CBDT", b"sbix", b"SVG "]
                        .iter()
                        .any(|tag| font.table_data(skrifa::Tag::new(tag)).is_some()),
                    "Color/bitmap font is unsupported for PDF text"
                );
                let flags = font
                    .os2()
                    .context("Cannot check embedding flags")?
                    .fs_type();
                // Conservative probe policy: also reject no-subsetting and bitmap-only fonts.
                ensure!(
                    flags & 0x0302 == 0,
                    "Font embedding flags unsupported: 0x{flags:04x}"
                );
                ensure!(
                    font.axes().is_empty(),
                    "Variable fonts require explicit instance handling"
                );
                let metrics = font.metrics(Size::unscaled(), LocationRef::default());
                let width = font
                    .glyph_metrics(Size::unscaled(), LocationRef::default())
                    .advance_width(skrifa::GlyphId::new(gid.0))
                    .context("No advance width")?;
                let mut pen = Pen::default();
                font.outline_glyphs()
                    .get(skrifa::GlyphId::new(gid.0))
                    .context("No outline glyph")?
                    .draw(
                        DrawSettings::unhinted(Size::unscaled(), LocationRef::default()),
                        &mut pen,
                    )?;
                Ok((pen.data, metrics.units_per_em as f32, width, metrics.bounds))
            })
            .context("Font data unavailable")??;
        let bounds = bounds.context("No font bounding box")?;
        let proc = self.stream("", format!("{width} 0 d0\n{path}f\n").as_bytes());
        let unicode = text
            .encode_utf16()
            .map(|u| format!("{u:04X}"))
            .collect::<String>();
        let cmap=self.stream("",format!("/CIDInit /ProcSet findresource begin\n12 dict begin begincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /ProbeUnicode def /CMapType 2 def\n1 begincodespacerange <00> <FF> endcodespacerange\n1 beginbfchar <01> <{unicode}> endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end").as_bytes());
        let id=self.object(format!("<< /Type /Font /Subtype /Type3 /FontBBox [{} {} {} {}] /FontMatrix [{} 0 0 {} 0 0] /CharProcs << /g {proc} 0 R >> /Encoding << /Type /Encoding /Differences [1 /g] >> /FirstChar 1 /LastChar 1 /Widths [{width}] /Resources << >> /ToUnicode {cmap} 0 R >>",bounds.x_min,bounds.y_min,bounds.x_max,bounds.y_max,1.0/upem,1.0/upem));
        self.fonts.insert(key, (id, upem));
        Ok((id, upem))
    }
    /// A cached ExtGState for fill alpha, stroke alpha, and blend mode.
    fn state(&mut self, fill: f32, stroke: f32, blend: &str, res: &mut Res) -> String {
        let key = format!("{fill}/{stroke}/{blend}");
        let id = match self.states.get(&key) {
            Some(id) => *id,
            None => {
                let id = self.object(format!(
                    "<< /Type /ExtGState /ca {fill} /CA {stroke} /BM /{blend} >>"
                ));
                self.states.insert(key, id);
                id
            }
        };
        res.add("ExtGState", format!("GS{id}"), id);
        format!("/GS{id} gs\n")
    }
    /// The fill (`rg`) or stroke (`RG`) operator for a paint, or `None` when only a raster can
    /// show it (patterns, translucent gradient stops).
    fn paint(
        &mut self,
        paint: &usvg::Paint,
        stroke: bool,
        transform: Transform,
        res: &mut Res,
    ) -> Result<Option<String>> {
        let (kind, coords, base, spread) = match paint {
            usvg::Paint::Color(_) => return color(paint, stroke).map(Some),
            usvg::Paint::Pattern(_) => return Ok(None),
            usvg::Paint::LinearGradient(g) => (
                2,
                format!("{} {} {} {}", g.x1(), g.y1(), g.x2(), g.y2()),
                &**g as &usvg::BaseGradient,
                g.spread_method(),
            ),
            usvg::Paint::RadialGradient(g) => (
                3,
                format!(
                    "{} {} {} {} {} {}",
                    g.fx(),
                    g.fy(),
                    g.fr().get(),
                    g.cx(),
                    g.cy(),
                    g.r().get()
                ),
                &**g as &usvg::BaseGradient,
                g.spread_method(),
            ),
        };
        let stops = base.stops();
        if stops.is_empty() || stops.iter().any(|s| s.opacity().get() < 1.0) {
            return Ok(None);
        }
        if spread != usvg::SpreadMethod::Pad {
            self.note(
                Code::Approximation,
                "Gradient spread methods other than pad are drawn as pad in PDF output",
            );
        }
        let rgb = |c: usvg::Color| {
            format!(
                "{} {} {}",
                c.red as f32 / 255.0,
                c.green as f32 / 255.0,
                c.blue as f32 / 255.0
            )
        };
        // Stops padded to cover 0..1, then one exponential segment per stop pair.
        let mut points: Vec<(f32, usvg::Color)> = stops
            .iter()
            .map(|s| (s.offset().get(), s.color()))
            .collect();
        if points[0].0 > 0.0 {
            points.insert(0, (0.0, points[0].1));
        }
        if points.last().unwrap().0 < 1.0 {
            points.push((1.0, points.last().unwrap().1));
        }
        let segments: Vec<String> = points
            .windows(2)
            .map(|pair| {
                format!(
                    "<< /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N 1 >>",
                    rgb(pair[0].1),
                    rgb(pair[1].1)
                )
            })
            .collect();
        let function = if segments.len() == 1 {
            segments[0].clone()
        } else {
            let bounds: Vec<String> = points[1..points.len() - 1]
                .iter()
                .map(|(offset, _)| offset.to_string())
                .collect();
            let encode = vec!["0 1"; segments.len()].join(" ");
            format!(
                "<< /FunctionType 3 /Domain [0 1] /Functions [{}] /Bounds [{}] /Encode [{encode}] >>",
                segments.join(" "),
                bounds.join(" ")
            )
        };
        let shading = self.object(format!(
            "<< /ShadingType {kind} /ColorSpace /DeviceRGB /Coords [{coords}] /Function {function} /Extend [true true] >>"
        ));
        let space = self.base.pre_concat(transform).pre_concat(base.transform());
        let pattern = self.object(format!(
            "<< /Type /Pattern /PatternType 2 /Shading {shading} 0 R /Matrix [{}] >>",
            matrix(space)
        ));
        res.add("Pattern", format!("P{pattern}"), pattern);
        Ok(Some(if stroke {
            format!("/Pattern CS /P{pattern} SCN\n")
        } else {
            format!("/Pattern cs /P{pattern} scn\n")
        }))
    }
    fn note(&mut self, code: Code, message: &str) {
        let location = self.location.clone();
        if !self.notes.iter().any(|n| n.1 == message && n.2 == location) {
            self.notes.push((code, message.to_owned(), location));
        }
    }
    /// Whether a path paints with something only a raster can show.
    fn needs_raster(path: &usvg::Path) -> bool {
        let raster_only = |paint: &usvg::Paint| match paint {
            usvg::Paint::Color(_) => false,
            usvg::Paint::Pattern(_) => true,
            usvg::Paint::LinearGradient(g) => g.stops().iter().any(|s| s.opacity().get() < 1.0),
            usvg::Paint::RadialGradient(g) => g.stops().iter().any(|s| s.opacity().get() < 1.0),
        };
        path.fill().is_some_and(|f| raster_only(f.paint()))
            || path.stroke().is_some_and(|s| raster_only(s.paint()))
    }
    /// Draw `node` as an RGBA image with a soft mask at up to `RASTER_SCALE` pixels per canvas
    /// unit. With `under_vectors`, the node is a group whose filters end by merging their result
    /// under the source graphic: the raster then holds only the filter result, recovered by
    /// removing the source graphic, and the caller draws the source as vectors on top. Returns
    /// whether anything was drawn (false when the raster is omitted for size).
    fn raster(
        &mut self,
        node: &usvg::Node,
        out: &mut String,
        res: &mut Res,
        message: &str,
        under_vectors: bool,
    ) -> Result<bool> {
        let Some(bbox) = node.abs_layer_bounding_box() else {
            return Ok(true);
        };
        let size = |scale: f32| {
            (
                (bbox.width() * scale).ceil().max(1.0) as u64,
                (bbox.height() * scale).ceil().max(1.0) as u64,
            )
        };
        let mut scale =
            RASTER_SCALE.min((RASTER_PIXELS as f32 / (bbox.width() * bbox.height())).sqrt());
        // Rounding up can push the area over the limit; shrink until the pixel size fits.
        while scale >= 1.0 && size(scale).0 * size(scale).1 > RASTER_PIXELS {
            scale *= 0.99;
        }
        if scale < 1.0 {
            self.note(
                Code::Omitted,
                "Omitted an element whose PDF raster would exceed the image pixel limit",
            );
            return Ok(false);
        }
        let (w, h) = size(scale);
        // The source pass of `under_vectors` allocates a second pixmap of the same size.
        let cost = w * h * if under_vectors { 2 } else { 1 };
        if self.raster_pixels + cost > self.raster_budget {
            self.note(
                Code::Omitted,
                "Omitted an element because the document's PDF raster budget is spent",
            );
            return Ok(false);
        }
        self.raster_pixels += cost;
        let (w, h) = (w as u32, h as u32);
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(w, h).context("Cannot allocate PDF raster")?;
        render_at(node, bbox, scale, &mut pixmap)?;
        if under_vectors {
            let usvg::Node::Group(group) = node else {
                bail!("Only groups keep their source as vectors")
            };
            let mut source =
                resvg::tiny_skia::Pixmap::new(w, h).context("Cannot allocate PDF raster")?;
            for child in group.children() {
                render_at(child, bbox, scale, &mut source)?;
            }
            // Premultiplied `result = source + rest * (1 - source alpha)`, so the rest (the
            // shadow) is `(result - source) / (1 - source alpha)` wherever the source lets it
            // show; under an opaque source it is never visible and is left empty.
            for (r, s) in pixmap.pixels_mut().iter_mut().zip(source.pixels()) {
                let keep = 255.0 - f32::from(s.alpha());
                let channel = |a: u8, b: u8| {
                    if keep < 1.0 {
                        0
                    } else {
                        ((f32::from(a) - f32::from(b)).max(0.0) * 255.0 / keep)
                            .round()
                            .min(255.0) as u8
                    }
                };
                let alpha = channel(r.alpha(), s.alpha());
                let c = |a: u8, b: u8| channel(a, b).min(alpha);
                *r = resvg::tiny_skia::PremultipliedColorU8::from_rgba(
                    c(r.red(), s.red()),
                    c(r.green(), s.green()),
                    c(r.blue(), s.blue()),
                    alpha,
                )
                .context("Invalid raster pixel")?;
            }
        }
        let (rgb, alpha): (Vec<[u8; 3]>, Vec<u8>) = pixmap
            .pixels()
            .iter()
            .map(|p| {
                let c = p.demultiply();
                ([c.red(), c.green(), c.blue()], c.alpha())
            })
            .unzip();
        let id = self.image(w as usize, h as usize, &rgb.concat(), Some(&alpha))?;
        res.add("XObject", format!("I{id}"), id);
        // A raster drawn in place of a whole group carries the group's blending.
        let blend = match node {
            usvg::Node::Group(g) if !under_vectors => blend_name(g.blend_mode()),
            _ => "Normal",
        };
        let state = if blend == "Normal" {
            String::new()
        } else {
            self.state(1.0, 1.0, blend, res).trim_end().to_owned() + " "
        };
        writeln!(
            out,
            "q {state}{} 0 0 {} {} {} cm /I{id} Do Q",
            w as f32 / scale,
            -(h as f32 / scale),
            bbox.x(),
            bbox.y() + h as f32 / scale
        )?;
        self.note(Code::Approximation, message);
        Ok(true)
    }
    /// A Flate-compressed RGB image XObject, with an alpha soft mask when given.
    fn image(&mut self, w: usize, h: usize, rgb: &[u8], alpha: Option<&[u8]>) -> Result<usize> {
        self.budget(rgb.len() / 4)?;
        let mask = match alpha.filter(|a| a.iter().any(|v| *v != 255)) {
            Some(alpha) => {
                let data = miniz_oxide::deflate::compress_to_vec_zlib(alpha, 6);
                let id = self.stream(&format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode"), &data);
                format!(" /SMask {id} 0 R")
            }
            None => String::new(),
        };
        let data = miniz_oxide::deflate::compress_to_vec_zlib(rgb, 6);
        Ok(self.stream(&format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode{mask}"), &data))
    }
    fn path(&mut self, path: &usvg::Path, out: &mut String, res: &mut Res) -> Result<()> {
        if !path.is_visible() {
            return Ok(());
        }
        let pen = Pen::from_path(path.data());
        writeln!(out, "q {} cm", matrix(path.abs_transform()))?;
        let fill_alpha = path.fill().map_or(1.0, |f| f.opacity().get());
        let stroke_alpha = path.stroke().map_or(1.0, |s| s.opacity().get());
        if fill_alpha < 1.0 || stroke_alpha < 1.0 {
            let state = self.state(fill_alpha, stroke_alpha, "Normal", res);
            out.push_str(&state);
        }
        let order = if path.paint_order() == usvg::PaintOrder::FillAndStroke {
            [false, true]
        } else {
            [true, false]
        };
        for stroke in order {
            if stroke {
                if let Some(s) = path.stroke() {
                    let paint = self
                        .paint(s.paint(), true, path.abs_transform(), res)?
                        .context("Stroke paint needs a raster")?;
                    out.push_str(&paint);
                    let cap = match s.linecap() {
                        usvg::LineCap::Butt => 0,
                        usvg::LineCap::Round => 1,
                        usvg::LineCap::Square => 2,
                    };
                    let join = match s.linejoin() {
                        usvg::LineJoin::Miter => 0,
                        usvg::LineJoin::Round => 1,
                        usvg::LineJoin::Bevel => 2,
                        _ => bail!("Miter-clip is not implemented"),
                    };
                    let dash = s
                        .dasharray()
                        .unwrap_or(&[])
                        .iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                    writeln!(
                        out,
                        "{} w {cap} J {join} j {} M [{dash}] {} d\n{}S",
                        s.width().get(),
                        s.miterlimit().get(),
                        s.dashoffset(),
                        pen.data
                    )?;
                }
            } else if let Some(f) = path.fill() {
                let paint = self
                    .paint(f.paint(), false, path.abs_transform(), res)?
                    .context("Fill paint needs a raster")?;
                out.push_str(&paint);
                out.push_str(&pen.data);
                out.push_str(if f.rule() == usvg::FillRule::EvenOdd {
                    "f*\n"
                } else {
                    "f\n"
                });
            }
        }
        out.push_str("Q\n");
        Ok(())
    }
    /// Draw a group: a transparency-group form for opacity, blending, or isolation, otherwise
    /// its clip and children in place.
    fn group(
        &mut self,
        group: &usvg::Group,
        tree: &usvg::Tree,
        out: &mut String,
        res: &mut Res,
        location: &str,
    ) -> Result<()> {
        let blend = blend_name(group.blend_mode());
        if group.opacity().get() == 1.0 && blend == "Normal" && !group.isolate() {
            return self.contents(group, tree, out, res, location);
        }
        let mut inner = String::new();
        let mut inner_res = Res::default();
        // Pattern matrices inside a form map to the form's space, which is the page's
        // drawing space because the form is invoked without its own transform.
        let page_base = std::mem::replace(&mut self.base, Transform::identity());
        let drawn = self.contents(group, tree, &mut inner, &mut inner_res, location);
        self.base = page_base;
        drawn?;
        self.budget(inner.len())?;
        let resources = self.resources(&inner_res);
        let b = group.abs_layer_bounding_box();
        let form = self.stream(
            &format!("/Type /XObject /Subtype /Form /BBox [{} {} {} {}] /Group << /S /Transparency /I true /K false >> /Resources {resources}", b.x() - 1.0, b.y() - 1.0, b.right() + 1.0, b.bottom() + 1.0),
            inner.as_bytes(),
        );
        res.add("XObject", format!("X{form}"), form);
        let alpha = group.opacity().get();
        let state = self.state(alpha, alpha, blend, res);
        writeln!(out, "q {}/X{form} Do Q", state.trim_end().to_owned() + " ")?;
        Ok(())
    }
    fn contents(
        &mut self,
        group: &usvg::Group,
        tree: &usvg::Tree,
        out: &mut String,
        res: &mut Res,
        location: &str,
    ) -> Result<()> {
        out.push_str("q\n");
        if let Some(clip) = group.clip_path() {
            ensure!(
                clip.clip_path().is_none() && clip.root().children().len() == 1,
                "Nested/multiple clip paths are not implemented"
            );
            let usvg::Node::Path(p) = &clip.root().children()[0] else {
                bail!("Non-path clip is not implemented")
            };
            let transform = group
                .abs_transform()
                .pre_concat(clip.transform())
                .pre_concat(p.abs_transform());
            let path = p
                .data()
                .clone()
                .transform(transform)
                .context("Invalid clip transform")?;
            ensure!(
                p.is_visible() && p.fill().is_some(),
                "Invisible clip path is unsupported"
            );
            let pen = Pen::from_path(&path);
            out.push_str(&pen.data);
            out.push_str(
                if p.fill()
                    .is_some_and(|f| f.rule() == usvg::FillRule::EvenOdd)
                {
                    "W* n\n"
                } else {
                    "W n\n"
                },
            );
        }
        for (index, node) in group.children().iter().enumerate() {
            self.budget(out.len())?;
            let loc = format!("{location}/{index}");
            self.location = loc.clone();
            let result: Result<()> = (|| {
                match node {
                    usvg::Node::Group(g) if g.mask().is_some() => {
                        self.raster(node, out, res, "Masks are rasterized in PDF output", false)?;
                    }
                    usvg::Node::Group(g) if !g.filters().is_empty() => {
                        // A filter that ends by merging its result under the source graphic
                        // (outer shadows) keeps the source as vectors on top of the raster, so
                        // its text stays text. Any other filter, or one combined with group
                        // opacity or blending, is shown by the raster alone.
                        let under_vectors = g.filters().iter().all(|f| keeps_source_on_top(f))
                            && g.opacity().get() == 1.0
                            && g.blend_mode() == usvg::BlendMode::Normal;
                        if self.raster(node, out, res, FILTER_NOTE, under_vectors)? && under_vectors
                        {
                            self.contents(g, tree, out, res, &loc)?;
                        }
                    }
                    usvg::Node::Group(g) => self.group(g, tree, out, res, &loc)?,
                    usvg::Node::Path(p) if Self::needs_raster(p) => {
                        self.raster(
                            node,
                            out,
                            res,
                            "Pattern fills and translucent gradient stops are rasterized in PDF output",
                            false,
                        )?;
                    }
                    usvg::Node::Path(p) => self.path(p, out, res)?,
                    usvg::Node::Text(t) => {
                        if t.layouted().iter().any(|span| {
                            span.visible
                                && span
                                    .fill
                                    .as_ref()
                                    .is_some_and(|f| !matches!(f.paint(), usvg::Paint::Color(_)))
                        }) {
                            self.raster(
                                node,
                                out,
                                res,
                                "Text with gradient or pattern fill is rasterized in PDF output",
                                false,
                            )?;
                            return Ok(());
                        }
                        let actual = t
                            .chunks()
                            .iter()
                            .flat_map(|c| {
                                c.spans()
                                    .iter()
                                    .filter(|s| s.is_visible() && s.fill().is_some())
                                    .map(|s| &c.text()[s.start()..s.end()])
                            })
                            .flat_map(str::encode_utf16)
                            .map(|u| format!("{u:04X}"))
                            .collect::<String>();
                        writeln!(
                            out,
                            "q {} cm /Span << /ActualText <FEFF{actual}> >> BDC",
                            matrix(t.abs_transform())
                        )?;
                        for span in t.layouted() {
                            if !span.visible {
                                continue;
                            }
                            ensure!(span.stroke.is_none(), "Text strokes are not implemented");
                            ensure!(
                                span.underline.is_none()
                                    && span.overline.is_none()
                                    && span.line_through.is_none(),
                                "Text decorations are not implemented"
                            );
                            let Some(fill) = span.fill.as_ref() else {
                                continue;
                            };
                            // Each span sets its own alpha, so a translucent span never leaks
                            // into the next one.
                            let alpha = fill.opacity().get();
                            let state = self.state(alpha, 1.0, "Normal", res);
                            out.push_str(&state);
                            out.push_str(&color(fill.paint(), false)?);
                            for glyph in &span.positioned_glyphs {
                                self.budget(out.len())?;
                                let (id, upem) =
                                    self.font(tree, glyph.font, glyph.id, &glyph.text)?;
                                let transform = glyph.outline_transform().pre_scale(upem, upem);
                                writeln!(
                                    out,
                                    "BT /F{id} 1 Tf {} Tm <01> Tj ET",
                                    matrix(transform)
                                )?;
                            }
                        }
                        out.push_str("EMC Q\n");
                    }
                    usvg::Node::Image(image) => {
                        if !image.is_visible() {
                            return Ok(());
                        }
                        let (width, height, rgb, alpha) = match image.kind() {
                            usvg::ImageKind::PNG(data) => {
                                let size = imagesize::blob_size(data)?;
                                ensure!(
                                    (size.width as u64) * (size.height as u64) <= 16_777_216,
                                    "PDF image exceeds pixel limit"
                                );
                                let pix = resvg::tiny_skia::Pixmap::decode_png(data)?;
                                let (rgb, alpha): (Vec<[u8; 3]>, Vec<u8>) = pix
                                    .pixels()
                                    .iter()
                                    .map(|p| {
                                        let c = p.demultiply();
                                        ([c.red(), c.green(), c.blue()], c.alpha())
                                    })
                                    .unzip();
                                (
                                    pix.width() as usize,
                                    pix.height() as usize,
                                    rgb.concat(),
                                    Some(alpha),
                                )
                            }
                            usvg::ImageKind::JPEG(data) => {
                                use zune_jpeg::{
                                    JpegDecoder,
                                    zune_core::{colorspace::ColorSpace, options::DecoderOptions},
                                };
                                let options = DecoderOptions::default()
                                    .jpeg_set_out_colorspace(ColorSpace::RGB);
                                let mut decoder = JpegDecoder::new_with_options(
                                    std::io::Cursor::new(data.as_slice()),
                                    options,
                                );
                                decoder
                                    .decode_headers()
                                    .map_err(|e| anyhow::anyhow!("Cannot decode JPEG: {e:?}"))?;
                                let info = decoder.info().context("JPEG info unavailable")?;
                                let (w, h) = (info.width as usize, info.height as usize);
                                ensure!(
                                    (w as u64) * (h as u64) <= 16_777_216,
                                    "PDF image exceeds pixel limit"
                                );
                                let rgb = decoder
                                    .decode()
                                    .map_err(|e| anyhow::anyhow!("Cannot decode JPEG: {e:?}"))?;
                                ensure!(rgb.len() == w * h * 3, "Unexpected JPEG pixel buffer");
                                (w, h, rgb, None)
                            }
                            _ => bail!("PDF supports PNG/JPEG images only"),
                        };
                        let id = self.image(width, height, &rgb, alpha.as_deref())?;
                        res.add("XObject", format!("I{id}"), id);
                        writeln!(
                            out,
                            "q {} cm {} 0 0 {} 0 {} cm /I{id} Do Q",
                            matrix(image.abs_transform()),
                            image.size().width(),
                            -image.size().height(),
                            image.size().height()
                        )?;
                    }
                }
                Ok(())
            })();
            result.with_context(|| format!("{loc} (SVG id {:?})", node.id()))?;
        }
        out.push_str("Q\n");
        Ok(())
    }
    /// A resource dictionary with every font so far and the entries in `res`.
    fn resources(&self, res: &Res) -> String {
        let mut fonts = self.fonts.values().map(|(id, _)| *id).collect::<Vec<_>>();
        fonts.sort_unstable();
        fonts.dedup();
        let fonts = fonts
            .into_iter()
            .map(|id| format!("/F{id} {id} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("<< /Font << {fonts} >> {} >>", res.dictionaries())
    }
    /// Write one page and return the approximations and omissions it needed, as
    /// (code, message, location in the slide scene).
    pub fn page(&mut self, tree: &usvg::Tree) -> Result<Vec<(Code, String, String)>> {
        ensure!(
            tree.size().width() * 0.75 <= 14_400.0 && tree.size().height() * 0.75 <= 14_400.0,
            "PDF page exceeds 200 inch limit"
        );
        self.base = Transform::from_row(0.75, 0.0, 0.0, -0.75, 0.0, tree.size().height() * 0.75);
        let mut content = format!("q 0.75 0 0 -0.75 0 {} cm\n", tree.size().height() * 0.75);
        let mut res = Res::default();
        self.group(tree.root(), tree, &mut content, &mut res, "root")?;
        content.push_str("Q\n");
        self.budget(content.len())?;
        let stream = self.stream("", content.as_bytes());
        let resources = self.resources(&res);
        let page=self.object(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources {resources} /Contents {stream} 0 R >>",tree.size().width()*0.75,tree.size().height()*0.75));
        self.pages.push(page);
        Ok(std::mem::take(&mut self.notes))
    }
    pub fn finish(&mut self) -> Result<Vec<u8>> {
        self.budget(0)?;
        self.objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
        self.objects[1] = format!(
            "<< /Type /Pages /Count {} /Kids [{}] >>",
            self.pages.len(),
            self.pages
                .iter()
                .map(|p| format!("{p} 0 R"))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .into_bytes();
        let mut pdf = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = vec![0];
        for (i, obj) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(obj);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in &offsets[1..] {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes())
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        Ok(pdf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf(svg: &str) -> (String, Vec<(Code, String, String)>) {
        let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
        let mut pdf = Pdf::new();
        let notes = pdf.page(&tree).unwrap();
        (
            String::from_utf8_lossy(&pdf.finish().unwrap()).into_owned(),
            notes,
        )
    }

    #[test]
    fn group_opacity_and_blending_become_transparency_groups() {
        let (text, notes) = pdf(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><g opacity="0.5"><rect width="50" height="50" fill="red"/></g><g style="mix-blend-mode:multiply"><rect x="40" width="50" height="50" fill="blue"/></g></svg>"#,
        );
        assert_eq!(text.matches("/S /Transparency").count(), 2, "{text}");
        assert!(text.contains("/ca 0.5 /CA 0.5 /BM /Normal"));
        assert!(text.contains("/BM /Multiply"));
        assert!(notes.is_empty());
    }

    #[test]
    fn reflected_gradients_are_padded_and_translucent_stops_rasterized() {
        let (text, notes) = pdf(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><defs><linearGradient id="g" x2="0.5" spreadMethod="reflect"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient><linearGradient id="t"><stop offset="0" stop-color="red" stop-opacity="0.5"/><stop offset="1" stop-color="blue"/></linearGradient></defs><rect width="50" height="50" fill="url(#g)"/><rect y="50" width="50" height="50" fill="url(#t)"/></svg>"##,
        );
        assert!(text.contains("/ShadingType 2"));
        assert!(text.contains("/SMask"));
        let messages: Vec<_> = notes.iter().map(|n| n.1.as_str()).collect();
        assert!(
            messages
                .contains(&"Gradient spread methods other than pad are drawn as pad in PDF output")
        );
        assert!(messages.contains(
            &"Pattern fills and translucent gradient stops are rasterized in PDF output"
        ));
    }

    #[test]
    fn the_document_raster_budget_stops_further_rasters() {
        // Three blurred 100 x 100 rectangles need about 210 x 210 x 4 pixels each at 2x; a
        // budget for two of them omits the third.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200"><defs><filter id="f"><feGaussianBlur stdDeviation="1"/></filter></defs><rect width="100" height="100" fill="red" filter="url(#f)"/><rect x="120" width="100" height="100" fill="red" filter="url(#f)"/><rect x="240" width="100" height="100" fill="red" filter="url(#f)"/></svg>"#;
        let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
        let mut pdf = Pdf::new();
        pdf.raster_budget = 2 * 260 * 260;
        let notes = pdf.page(&tree).unwrap();
        assert_eq!(
            notes
                .iter()
                .filter(|n| n.1.contains("raster budget"))
                .count(),
            1,
            "{notes:?}"
        );
    }

    #[test]
    fn rasters_above_the_pixel_limit_are_omitted() {
        let (text, notes) = pdf(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="5000" height="5000"><defs><filter id="f"><feGaussianBlur stdDeviation="2"/></filter></defs><rect width="5000" height="5000" fill="red" filter="url(#f)"/></svg>"#,
        );
        assert!(!text.contains("/Subtype /Image"));
        assert!(notes.iter().any(|n| n.0 == Code::Omitted), "{notes:?}");
    }
}
