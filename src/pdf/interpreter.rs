use super::reader::{Document, Reader, Value};
use anyhow::{Result, bail, ensure};
use base64::Engine;
use resvg::tiny_skia::{self as sk, Transform};
use std::{collections::BTreeMap, fmt::Write};

type Dict = BTreeMap<String, Value>;
pub type PageDict<'a> = BTreeMap<String, &'a Value>;
fn page_get<'a>(d: &PageDict<'a>, key: &str) -> Result<&'a Value> {
    d.get(key)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("Missing required page field"))
}
fn get<'a>(d: &'a Dict, k: &str) -> Result<&'a Value> {
    d.get(k)
        .ok_or_else(|| anyhow::anyhow!("Missing required PDF field"))
}
fn nums(v: &Value, n: usize) -> Result<Vec<f64>> {
    let a = v.array()?;
    ensure!(a.len() == n, "Invalid numeric array");
    a.iter().map(Value::number).collect()
}
fn matrix(v: &[f64]) -> Transform {
    Transform::from_row(
        v[0] as f32,
        v[1] as f32,
        v[2] as f32,
        v[3] as f32,
        v[4] as f32,
        v[5] as f32,
    )
}
fn mt(t: Transform) -> String {
    format!(
        "matrix({} {} {} {} {} {})",
        t.sx, t.ky, t.kx, t.sy, t.tx, t.ty
    )
}
#[derive(Clone)]
struct State {
    ctm: Transform,
    fill: String,
    stroke: String,
    width: f64,
    cap: usize,
    join: usize,
    miter: f64,
    dash: Vec<f64>,
    offset: f64,
    clips: Vec<usize>,
    font: Option<Value>,
    size: f64,
    tm: Transform,
}
impl Default for State {
    fn default() -> Self {
        Self {
            ctm: Transform::identity(),
            fill: "rgb(0,0,0)".into(),
            stroke: "rgb(0,0,0)".into(),
            width: 1.,
            cap: 0,
            join: 0,
            miter: 10.,
            dash: Vec::new(),
            offset: 0.,
            clips: Vec::new(),
            font: None,
            size: 1.,
            tm: Transform::identity(),
        }
    }
}
pub struct Renderer<'a> {
    doc: &'a Document,
    svg: String,
    defs: String,
    commands: usize,
    decoded: usize,
    clip_id: usize,
    pixels: u64,
    page_pixels: u64,
    paint_work: u64,
}
impl<'a> Renderer<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            svg: String::new(),
            defs: String::new(),
            commands: 0,
            decoded: 0,
            clip_id: 0,
            pixels: 0,
            page_pixels: 0,
            paint_work: 0,
        }
    }
    fn resource(&self, res: &Dict, kind: &str, name: &str) -> Result<Value> {
        let d = self.doc.dict(get(res, kind)?)?;
        let value = get(d, name)?;
        ensure!(
            matches!(value, Value::Ref(_)),
            "Inline drawing resources unsupported"
        );
        Ok(value.clone())
    }
    fn emit(&mut self, s: &State, item: &str) -> Result<()> {
        // Conservatively charge a full-page paint for each primitive and clip.
        self.paint_work += self.page_pixels * (1 + s.clips.len() as u64);
        ensure!(self.paint_work <= 1_073_741_824, "Raster work limit");
        ensure!(
            self.svg.len() + self.defs.len() + item.len() < 32 * 1024 * 1024,
            "Scene byte limit"
        );
        for c in &s.clips {
            write!(self.svg, "<g clip-path=\"url(#c{c})\">")?;
        }
        self.svg.push_str(item);
        for _ in &s.clips {
            self.svg.push_str("</g>");
        }
        Ok(())
    }
    fn stream(&mut self, v: &Value, image: bool) -> Result<Vec<u8>> {
        let o = self.doc.object(v)?;
        let d = o.value.dict()?;
        let raw = o
            .stream
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing stream"))?;
        ensure!(
            !d.contains_key("DecodeParms"),
            "Decode parameters unsupported"
        );
        let bytes = if let Some(filter) = d.get("Filter") {
            ensure!(
                image && filter.name()? == "FlateDecode",
                "Unsupported stream filter"
            );
            miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
                raw,
                (16 * 1024 * 1024).min(64 * 1024 * 1024 - self.decoded),
            )
            .map_err(|_| anyhow::anyhow!("Invalid or oversized compressed stream"))?
        } else {
            raw.clone()
        };
        self.decoded += bytes.len();
        ensure!(self.decoded <= 64 * 1024 * 1024, "Decoded byte limit");
        Ok(bytes)
    }
    fn interpret(
        &mut self,
        bytes: &[u8],
        res: &Dict,
        mut s: State,
        depth: usize,
        glyph: bool,
    ) -> Result<()> {
        ensure!(depth < 32, "Drawing recursion limit");
        let mut r = Reader::new(bytes);
        let mut args: Vec<Value> = Vec::new();
        let mut stack = Vec::new();
        let mut path = String::new();
        let mut path_matrix = None;
        let mut clip = None;
        let mut text = false;
        let mut marked = 0;
        loop {
            r.space();
            if r.pos == bytes.len() {
                break;
            }
            let v = r.value(0, false)?;
            let Value::Word(op) = v else {
                ensure!(args.len() < 128, "Operand limit");
                args.push(v);
                continue;
            };
            self.commands += 1;
            ensure!(self.commands <= 200_000, "Drawing command limit");
            let count = match op.as_str() {
                "q" | "Q" | "h" | "f" | "f*" | "S" | "n" | "W" | "W*" | "BT" | "ET" | "EMC" => 0,
                "cm" | "c" | "Tm" => 6,
                "m" | "l" | "Tf" | "d0" | "d" | "BDC" => 2,
                "rg" | "RG" => 3,
                "w" | "J" | "j" | "M" | "g" | "G" | "Do" | "gs" | "Tj" | "BMC" => 1,
                _ => bail!("Unsupported PDF drawing operator"),
            };
            ensure!(args.len() == count, "Invalid operand count");
            match op.as_str() {
                "q" => {
                    ensure!(stack.len() < 64, "Graphics-state depth limit");
                    stack.push(s.clone());
                }
                "Q" => {
                    s = stack
                        .pop()
                        .ok_or_else(|| anyhow::anyhow!("Unbalanced graphics state"))?;
                }
                "cm" => {
                    ensure!(path.is_empty(), "Transform inside a path unsupported");
                    let n = args.iter().map(Value::number).collect::<Result<Vec<_>>>()?;
                    s.ctm = s.ctm.pre_concat(matrix(&n));
                    ensure!(s.ctm.is_finite(), "Invalid transform");
                }
                "m" | "l" | "c" => {
                    if path_matrix.is_none() {
                        path_matrix = Some(s.ctm);
                    }
                    let letter = match op.as_str() {
                        "m" => "M",
                        "l" => "L",
                        _ => "C",
                    };
                    path.push_str(letter);
                    for a in &args {
                        write!(path, "{} ", a.number()?)?;
                    }
                    ensure!(path.len() < 1024 * 1024, "Path limit");
                }
                "h" => path.push('Z'),
                "W" | "W*" => clip = Some(op == "W*"),
                "f" | "f*" | "S" | "n" => {
                    let t = path_matrix.unwrap_or(s.ctm);
                    let even = op == "f*";
                    let rule = if even { "evenodd" } else { "nonzero" };
                    if op != "n" && !path.is_empty() {
                        let attrs = if op == "S" {
                            ensure!(s.width > 0., "Hairline strokes unsupported");
                            format!(
                                "fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"{}\" stroke-linejoin=\"{}\" stroke-miterlimit=\"{}\" stroke-dasharray=\"{}\" stroke-dashoffset=\"{}\"",
                                s.stroke,
                                s.width,
                                ["butt", "round", "square"][s.cap],
                                ["miter", "round", "bevel"][s.join],
                                s.miter,
                                if s.dash.is_empty() {
                                    "none".into()
                                } else {
                                    s.dash
                                        .iter()
                                        .map(ToString::to_string)
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                },
                                s.offset
                            )
                        } else {
                            format!("fill=\"{}\" fill-rule=\"{rule}\"", s.fill)
                        };
                        self.emit(
                            &s,
                            &format!("<path transform=\"{}\" d=\"{path}\" {attrs}/>", mt(t)),
                        )?;
                    }
                    if let Some(even) = clip.take() {
                        ensure!(s.clips.len() < 64, "Clip depth limit");
                        let id = self.clip_id;
                        self.clip_id += 1;
                        write!(
                            self.defs,
                            "<clipPath id=\"c{id}\" clipPathUnits=\"userSpaceOnUse\"><path transform=\"{}\" d=\"{path}\" clip-rule=\"{}\"/></clipPath>",
                            mt(t),
                            if even { "evenodd" } else { "nonzero" }
                        )?;
                        s.clips.push(id);
                    }
                    path.clear();
                    path_matrix = None;
                }
                "rg" | "RG" | "g" | "G" => {
                    let n = args.iter().map(Value::number).collect::<Result<Vec<_>>>()?;
                    ensure!(
                        n.iter().all(|x| (0.0..=1.0).contains(x)),
                        "Color component out of range"
                    );
                    let n = if n.len() == 1 { vec![n[0]; 3] } else { n };
                    let color = format!("rgb({},{},{})", n[0] * 255., n[1] * 255., n[2] * 255.);
                    if op == "rg" || op == "g" {
                        s.fill = color;
                    } else {
                        s.stroke = color;
                    }
                }
                "w" => {
                    s.width = args[0].number()?;
                    ensure!(s.width >= 0., "Invalid stroke width");
                }
                "J" | "j" => {
                    let n = args[0].integer()?;
                    ensure!(n <= 2, "Invalid line style");
                    if op == "J" { s.cap = n } else { s.join = n }
                }
                "M" => {
                    s.miter = args[0].number()?;
                    ensure!(s.miter >= 1., "Invalid miter limit");
                }
                "d" => {
                    s.dash = args[0]
                        .array()?
                        .iter()
                        .map(Value::number)
                        .collect::<Result<Vec<_>>>()?;
                    ensure!(
                        s.dash.len() <= 128
                            && s.dash.iter().all(|n| *n >= 0.)
                            && (s.dash.is_empty() || s.dash.iter().any(|n| *n > 0.)),
                        "Invalid dash pattern"
                    );
                    s.offset = args[1].number()?;
                }
                "BT" => {
                    ensure!(!text, "Nested text object");
                    text = true;
                    s.tm = Transform::identity();
                }
                "ET" => {
                    ensure!(text, "Unbalanced text object");
                    text = false;
                }
                "Tf" => {
                    ensure!(text, "Font outside text object");
                    let font = self.resource(res, "Font", args[0].name()?)?;
                    ensure!(
                        get(self.doc.dict(&font)?, "Subtype")?.name()? == "Type3",
                        "Unsupported font subtype"
                    );
                    s.font = Some(font);
                    s.size = args[1].number()?;
                    ensure!(s.size > 0., "Invalid font size");
                }
                "Tm" => {
                    ensure!(text, "Text matrix outside text object");
                    s.tm = matrix(&args.iter().map(Value::number).collect::<Result<Vec<_>>>()?);
                }
                "Tj" => {
                    ensure!(text, "Text outside text object");
                    let Value::Bytes(codes) = &args[0] else {
                        bail!("Expected text bytes")
                    };
                    let font = s
                        .font
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Missing font"))?;
                    let fd = self.doc.dict(font)?;
                    let fm = matrix(&nums(get(fd, "FontMatrix")?, 6)?);
                    let encoding = self.doc.dict(get(fd, "Encoding")?)?;
                    ensure!(
                        !encoding.contains_key("BaseEncoding"),
                        "Base font encoding unsupported"
                    );
                    let differences = get(encoding, "Differences")?.array()?;
                    let mut map = BTreeMap::new();
                    let mut code = 0usize;
                    for v in differences {
                        match v {
                            Value::Number(_) => code = v.integer()?,
                            Value::Name(name) => {
                                ensure!(code < 256, "Invalid font encoding");
                                map.insert(code, name.clone());
                                code += 1;
                            }
                            _ => bail!("Invalid font encoding"),
                        }
                    }
                    let procs = self.doc.dict(get(fd, "CharProcs")?)?;
                    let first = get(fd, "FirstChar")?.integer()?;
                    let widths = get(fd, "Widths")?.array()?;
                    let empty = Dict::new();
                    let fres = if let Some(v) = fd.get("Resources") {
                        self.doc.dict(v)?
                    } else {
                        &empty
                    };
                    for c in codes {
                        self.commands += 1;
                        ensure!(self.commands <= 200_000, "Drawing command limit");
                        let name = map
                            .get(&(*c as usize))
                            .ok_or_else(|| anyhow::anyhow!("Missing character encoding"))?;
                        let proc = get(procs, name)?;
                        let bytes = self.stream(proc, false)?;
                        let mut gs = s.clone();
                        gs.ctm = s
                            .ctm
                            .pre_concat(s.tm)
                            .pre_scale(s.size as f32, s.size as f32)
                            .pre_concat(fm);
                        self.interpret(&bytes, fres, gs, depth + 1, true)?;
                        let idx = (*c as usize)
                            .checked_sub(first)
                            .ok_or_else(|| anyhow::anyhow!("Invalid character width"))?;
                        let width = widths
                            .get(idx)
                            .ok_or_else(|| anyhow::anyhow!("Missing character width"))?
                            .number()?;
                        s.tm = s.tm.pre_translate(
                            (width * fm.sx as f64 * s.size) as f32,
                            (width * fm.ky as f64 * s.size) as f32,
                        );
                    }
                }
                "d0" => {
                    ensure!(glyph, "Glyph width outside glyph");
                    for v in &args {
                        v.number()?;
                    }
                }
                "BMC" => {
                    args[0].name()?;
                    marked += 1;
                    ensure!(marked < 64, "Marked-content depth limit");
                }
                "BDC" => {
                    args[0].name()?;
                    let d = args[1].dict()?;
                    ensure!(
                        d.keys().all(|k| k == "ActualText"),
                        "Unsupported marked-content property"
                    );
                    marked += 1;
                    ensure!(marked < 64, "Marked-content depth limit");
                }
                "EMC" => {
                    ensure!(marked > 0, "Unbalanced marked content");
                    marked -= 1;
                }
                "gs" => {
                    let gs = self.resource(res, "ExtGState", args[0].name()?)?;
                    let d = self.doc.dict(&gs)?;
                    ensure!(
                        d.keys()
                            .all(|k| matches!(k.as_str(), "Type" | "ca" | "CA" | "BM")),
                        "Unsupported graphics state"
                    );
                    for key in ["ca", "CA"] {
                        if let Some(v) = d.get(key) {
                            ensure!(v.number()? == 1., "Transparency unsupported");
                        }
                    }
                    if let Some(v) = d.get("BM") {
                        ensure!(v.name()? == "Normal", "Blend mode unsupported");
                    }
                }
                "Do" => {
                    let image = self.resource(res, "XObject", args[0].name()?)?;
                    let d = self.doc.dict(&image)?;
                    ensure!(
                        get(d, "Subtype")?.name()? == "Image",
                        "Form XObject unsupported"
                    );
                    ensure!(
                        !d.contains_key("SMask")
                            && !d.contains_key("Mask")
                            && !d.contains_key("Decode")
                            && !d.contains_key("ImageMask")
                            && !d.contains_key("Interpolate"),
                        "Image mask or decode unsupported"
                    );
                    ensure!(
                        get(d, "ColorSpace")?.name()? == "DeviceRGB"
                            && get(d, "BitsPerComponent")?.integer()? == 8,
                        "Unsupported image format"
                    );
                    let w = get(d, "Width")?.integer()?;
                    let h = get(d, "Height")?.integer()?;
                    ensure!(
                        w > 0 && h > 0 && w <= 4096 && h <= 4096 && w * h <= 4_194_304,
                        "Image pixel limit"
                    );
                    let bytes = self.stream(&image, true)?;
                    ensure!(bytes.len() == w * h * 3, "Invalid image bytes");
                    let mut png = Vec::new();
                    {
                        let mut e = png::Encoder::new(&mut png, w as u32, h as u32);
                        e.set_color(png::ColorType::Rgb);
                        e.write_header()?.write_image_data(&bytes)?;
                    }
                    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
                    let t = s.ctm.pre_concat(Transform::from_row(
                        1.0 / w as f32,
                        0.,
                        0.,
                        -1.0 / h as f32,
                        0.,
                        1.,
                    ));
                    self.emit(&s,&format!("<image transform=\"{}\" width=\"{w}\" height=\"{h}\" preserveAspectRatio=\"none\" image-rendering=\"optimizeSpeed\" href=\"data:image/png;base64,{encoded}\"/>",mt(t)))?;
                }
                _ => unreachable!(),
            }
            args.clear();
        }
        ensure!(
            args.is_empty()
                && stack.is_empty()
                && !text
                && marked == 0
                && path.is_empty()
                && clip.is_none(),
            "Unbalanced drawing program"
        );
        Ok(())
    }
    pub fn page(&mut self, page: &PageDict<'_>, width: u32) -> Result<sk::Pixmap> {
        ensure!((64..=4096).contains(&width), "Output width out of range");
        ensure!(!page.contains_key("AA"), "Page actions unsupported");
        let b = nums(self.doc.resolve(page_get(page, "MediaBox")?)?, 4)?;
        ensure!(
            b[0] == 0. && b[1] == 0. && b[2] > 0. && b[3] > 0. && b[2] <= 14400. && b[3] <= 14400.,
            "Unsupported page bounds"
        );
        for (k, default) in [("Rotate", 0.), ("UserUnit", 1.)] {
            if let Some(v) = page.get(k) {
                ensure!(
                    self.doc.resolve(v)?.number()? == default,
                    "Unsupported page geometry"
                );
            }
        }
        if let Some(v) = page.get("CropBox") {
            ensure!(
                nums(self.doc.resolve(v)?, 4)? == b,
                "Page cropping unsupported"
            );
        }
        if let Some(v) = page.get("Annots") {
            ensure!(
                self.doc.resolve(v)?.array()?.is_empty(),
                "Annotations unsupported"
            );
        }
        ensure!(
            !page.contains_key("Group"),
            "Page transparency group unsupported"
        );
        let height = (width as f64 * b[3] / b[2]).round();
        ensure!(
            (1.0..=16384.).contains(&height) && height * width as f64 <= 16_777_216.,
            "Page pixel limit"
        );
        let height = height as u32;
        let pixels = width as u64 * height as u64;
        ensure!(self.pixels + pixels <= 67_108_864, "Document pixel limit");
        self.pixels += pixels;
        self.page_pixels = pixels;
        self.svg.clear();
        self.defs.clear();
        let res = self.doc.dict(page_get(page, "Resources")?)?;
        let content = page_get(page, "Contents")?;
        let bytes = self.stream(content, false)?;
        let s = State {
            ctm: Transform::from_row(1., 0., 0., -1., 0., b[3] as f32),
            ..State::default()
        };
        self.interpret(&bytes, res, s, 0, false)?;
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\"><defs>{}</defs>{}</svg>",
            b[2], b[3], self.defs, self.svg
        );
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
            .map_err(|_| anyhow::anyhow!("Cannot construct PDF scene"))?;
        let mut pix = sk::Pixmap::new(width, height)
            .ok_or_else(|| anyhow::anyhow!("Cannot allocate page"))?;
        pix.fill(sk::Color::WHITE);
        resvg::render(
            &tree,
            Transform::from_scale(width as f32 / b[2] as f32, height as f32 / b[3] as f32),
            &mut pix.as_mut(),
        );
        Ok(pix)
    }
}
pub fn pages(doc: &Document) -> Result<Vec<PageDict<'_>>> {
    fn walk<'a>(
        doc: &'a Document,
        v: &'a Value,
        inherit: &PageDict<'a>,
        active: &mut Vec<u32>,
        out: &mut Vec<PageDict<'a>>,
        visits: &mut usize,
    ) -> Result<()> {
        *visits += 1;
        ensure!(*visits <= 100_000 && active.len() < 64, "Page tree limit");
        let Value::Ref(id) = v else {
            bail!("Page must be indirect")
        };
        ensure!(!active.contains(id), "Page tree cycle");
        active.push(*id);
        let d = doc.dict(v)?;
        let mut inherited = inherit.clone();
        for k in ["MediaBox", "CropBox", "Rotate", "Resources"] {
            if let Some(v) = d.get(k) {
                inherited.insert(k.into(), v);
            }
        }
        match get(d, "Type")?.name()? {
            "Pages" => {
                for v in doc.resolve(get(d, "Kids")?)?.array()? {
                    walk(doc, v, &inherited, active, out, visits)?;
                }
            }
            "Page" => {
                ensure!(out.len() < 100, "Page count limit");
                let mut p: PageDict<'_> = d.iter().map(|(k, v)| (k.clone(), v)).collect();
                for (k, v) in inherited {
                    p.entry(k).or_insert(v);
                }
                out.push(p);
            }
            _ => bail!("Invalid page tree"),
        };
        active.pop();
        Ok(())
    }
    let root = doc.dict(&doc.root)?;
    ensure!(get(root, "Type")?.name()? == "Catalog", "Invalid catalog");
    ensure!(
        !root.contains_key("OpenAction")
            && !root.contains_key("AA")
            && !root.contains_key("AcroForm")
            && !root.contains_key("Names")
            && !root.contains_key("OCProperties"),
        "Document actions or forms unsupported"
    );
    let mut pages = Vec::new();
    walk(
        doc,
        get(root, "Pages")?,
        &PageDict::new(),
        &mut Vec::new(),
        &mut pages,
        &mut 0,
    )?;
    ensure!(!pages.is_empty(), "Empty PDF");
    Ok(pages)
}
