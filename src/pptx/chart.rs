//! Draw bar, line, and pie charts from the values cached in a chart part.
//!
//! The chart is translated into ordinary DrawingML shapes (rectangles, freeform lines and
//! wedges, and text boxes) that the shape renderer draws inside the frame's transform, so PNG
//! and PDF output, colours, and text all follow the existing paths. Positions that Office
//! computes itself (plot area, legend, labels, automatic axis scale) follow the rules here and
//! are reported as approximations. Nothing outside the chart part is read.
use super::objects::Renderer;
use super::*;

const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const MAX_SERIES: usize = 64;
/// A 0.75 pt line in CSS pixels.
const HAIRLINE_PX: f64 = 1.0;
const MAX_POINTS: usize = 10_000;
/// Longest text taken from the chart part for one name, category, or separator.
const MAX_TEXT: usize = 255;
/// Largest synthetic shape markup drawn for one chart.
const MAX_MARKUP: usize = 16 * 1024 * 1024;
/// Office's default accent cycle, used when the theme does not define an accent colour.
const PALETTE: [&str; 6] = ["4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47"];
const LAYOUT_NOTE: &str = "Chart layout is approximated: plot area, legend, and label positions follow rhodopsin rules, not Office's";
const SCALE_NOTE: &str = "Chart axis scale is approximated: missing minimum, maximum, or major unit is computed by rhodopsin";

/// The chart relationship ID when the frame holds a chart.
pub(super) fn reference<'d>(frame: Node<'d, 'd>) -> Option<&'d str> {
    let data = child(frame, A, "graphic").and_then(|n| child(n, A, "graphicData"))?;
    if data.attribute("uri") != Some(C) {
        return None;
    }
    child(data, C, "chart").and_then(|n| n.attribute((R, "id")))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Column,
    Bar,
    Line,
    Pie,
}
#[derive(Clone, Copy, PartialEq)]
enum Grouping {
    Clustered,
    Stacked,
    Percent,
}
struct Series<'a> {
    name: String,
    values: Vec<Option<f64>>,
    format: Option<String>,
    props: Option<Node<'a, 'a>>,
    points: HashMap<usize, Node<'a, 'a>>,
    labels: Option<Node<'a, 'a>>,
    marker: Option<Node<'a, 'a>>,
    smooth: bool,
}
/// Run properties shared by the synthetic text boxes.
#[derive(Clone)]
struct Font {
    size: f64,
    bold: bool,
    color: String,
    faces: String,
}
impl Font {
    fn px(&self) -> f64 {
        self.size / 75.0
    }
    fn with(&self, props: Option<Node<'_, '_>>, theme: &Theme, warnings: &mut Warnings) -> Font {
        let mut font = self.clone();
        let Some(props) = props else { return font };
        if let Some(size) = props.attribute("sz").and_then(|v| v.parse::<f64>().ok()) {
            font.size = size.clamp(100.0, 40000.0);
        }
        if let Some(bold) = theme::bool_attr(props, "b") {
            font.bold = bold;
        }
        if let Some(fill) = child(props, A, "solidFill") {
            font.color = color(fill, &font.color, theme, warnings);
        }
        let faces: String = ["latin", "ea", "cs"]
            .iter()
            .filter_map(|kind| {
                child(props, A, kind)
                    .and_then(|n| n.attribute("typeface"))
                    .map(|face| format!("<a:{kind} typeface=\"{}\"/>", escape(face)))
            })
            .collect();
        if !faces.is_empty() {
            font.faces = faces;
        }
        font
    }
}
/// Default run properties of a `c:txPr` or `c:rich` body.
fn text_props<'a>(node: Option<Node<'a, 'a>>) -> Option<Node<'a, 'a>> {
    let body = node.and_then(|n| {
        child(n, C, "txPr").or_else(|| child(n, C, "tx").and_then(|t| child(t, C, "rich")))
    })?;
    body.descendants()
        .find(|n| n.has_tag_name((A, "defRPr")) || n.has_tag_name((A, "rPr")))
}

struct Chart<'a> {
    kind: Kind,
    grouping: Grouping,
    plot: Node<'a, 'a>,
    series: Vec<Series<'a>>,
    categories: Vec<String>,
    vary: bool,
}

/// Draw the chart in `frame` or record why it is omitted. Errors inside the chart part never
/// abort the slide.
pub(super) fn render<'d>(
    renderer: &mut Renderer<'_, 'd>,
    frame: Node<'d, 'd>,
    id: &str,
    location: &str,
    depth: usize,
) -> Result<()> {
    let Some(rel) = renderer.rels.get(id) else {
        renderer
            .warnings
            .push(Code::Omitted, "Omitted chart: missing chart relationship");
        return Ok(());
    };
    if rel.external || !rel.kind.ends_with("/chart") {
        renderer.warnings.push(
            Code::Omitted,
            "Omitted chart: the relationship is not an internal chart part",
        );
        return Ok(());
    }
    let Some(doc) = renderer
        .parts
        .get(&rel.target)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(|text| Document::parse(text).ok())
    else {
        renderer.warnings.push(
            Code::Omitted,
            "Omitted chart: the chart part is missing or is not valid XML",
        );
        return Ok(());
    };
    let Some(t) = child(frame, P, "xfrm") else {
        renderer
            .warnings
            .push(Code::Omitted, "Omitted chart: missing frame transform");
        return Ok(());
    };
    let root = doc.root_element();
    let chart = match model(root, renderer.warnings) {
        Ok(chart) => chart,
        Err(reason) => {
            renderer.warnings.push(Code::Unsupported, reason);
            return Ok(());
        }
    };
    let (w, h, mirrored) = objects::transform(t, renderer.svg)?;
    let identity = frame.children().find_map(|n| child(n, P, "cNvPr"));
    let id_attr = identity.and_then(|n| n.attribute("id")).unwrap_or("0");
    let name = identity.and_then(|n| n.attribute("name")).unwrap_or("");
    let mut out = Shapes {
        xml: String::new(),
        id: escape(id_attr),
        name: escape(name),
    };
    draw(
        &chart,
        root,
        w,
        h,
        renderer.theme,
        renderer.warnings,
        &mut out,
    );
    if out.xml.len() > MAX_MARKUP {
        renderer.svg.push_str("</g>");
        renderer.warnings.push(
            Code::Omitted,
            "Omitted chart: its drawing exceeds the size limit",
        );
        return Ok(());
    }
    let xml = format!(
        "<p:spTree xmlns:p=\"{P}\" xmlns:a=\"{A}\">{}</p:spTree>",
        out.xml
    );
    let synthetic = Document::parse(&xml).context("Cannot build chart shapes")?;
    let mut inner = Renderer {
        parts: renderer.parts,
        rels: renderer.rels,
        theme: renderer.theme,
        options: renderer.options,
        svg: &mut *renderer.svg,
        warnings: &mut *renderer.warnings,
        part: renderer.part,
        ancestors: &[],
        master_style: None,
        mirrored: renderer.mirrored ^ mirrored,
        group_fill: None,
    };
    for (i, node) in synthetic
        .root_element()
        .children()
        .filter(|n| n.is_element())
        .enumerate()
    {
        inner.node(node, &format!("{location}/chart/{i}"), depth + 1)?;
    }
    renderer.svg.push_str("</g>");
    Ok(())
}

/// Read and validate the chart before anything is drawn; `Err` is the omission diagnostic.
fn model<'a>(root: Node<'a, 'a>, warnings: &mut Warnings) -> Result<Chart<'a>, String> {
    let plot = child(root, C, "chart")
        .and_then(|n| child(n, C, "plotArea"))
        .ok_or("Omitted chart: missing plot area")?;
    let plots: Vec<_> = plot
        .children()
        .filter(|n| n.is_element() && n.tag_name().name().ends_with("Chart"))
        .collect();
    let names: Vec<_> = plots
        .iter()
        .map(|n| format!("c:{}", n.tag_name().name()))
        .collect();
    let [element] = plots.as_slice() else {
        return Err(if plots.is_empty() {
            "Omitted chart: the plot area has no chart type".into()
        } else {
            format!(
                "Unsupported chart type: {}; combined chart types are omitted",
                names.join(" + ")
            )
        });
    };
    let element = *element;
    let value = |tag: &str| child(element, C, tag).and_then(|n| n.attribute("val"));
    let grouping = value("grouping").unwrap_or("clustered");
    let (kind, grouping) = match (element.tag_name().name(), grouping) {
        ("barChart", g) => {
            let kind = if value("barDir") == Some("bar") {
                Kind::Bar
            } else {
                Kind::Column
            };
            let grouping = match g {
                "clustered" | "standard" => Grouping::Clustered,
                "stacked" => Grouping::Stacked,
                "percentStacked" => Grouping::Percent,
                other => {
                    return Err(format!(
                        "Unsupported chart grouping: {other} in c:barChart; the chart is omitted"
                    ));
                }
            };
            (kind, grouping)
        }
        ("lineChart", "standard") => (Kind::Line, Grouping::Clustered),
        ("lineChart", other) => {
            return Err(format!(
                "Unsupported chart grouping: {other} in c:lineChart; the chart is omitted"
            ));
        }
        ("pieChart", _) => (Kind::Pie, Grouping::Clustered),
        _ => {
            return Err(format!(
                "Unsupported chart type: {}; the chart is omitted",
                names[0]
            ));
        }
    };
    let nodes: Vec<_> = element
        .children()
        .filter(|n| n.has_tag_name((C, "ser")))
        .collect();
    if nodes.len() > MAX_SERIES {
        return Err(format!("Omitted chart: more than {MAX_SERIES} series"));
    }
    let mut series = Vec::new();
    let mut total = 0usize;
    let mut categories = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let (values, format) = child(*node, C, "val").map(numbers).unwrap_or_default();
        total += values.len();
        if total > MAX_POINTS {
            return Err(format!("Omitted chart: more than {MAX_POINTS} data points"));
        }
        if categories.is_empty() {
            categories = child(*node, C, "cat")
                .map(|n| labels(n, warnings))
                .unwrap_or_default();
        }
        let name = child(*node, C, "tx")
            .and_then(|tx| {
                tx.descendants()
                    .find(|n| n.has_tag_name((C, "v")))
                    .and_then(|n| n.text())
                    .map(clip)
            })
            .unwrap_or_else(|| format!("Series{}", i + 1));
        let points = node
            .children()
            .filter(|n| n.has_tag_name((C, "dPt")))
            .filter_map(|n| {
                let idx = child(n, C, "idx")?.attribute("val")?.parse().ok()?;
                Some((idx, child(n, C, "spPr")?))
            })
            .collect();
        if node
            .children()
            .any(|n| matches!(n.tag_name().name(), "trendline" | "errBars"))
        {
            warn(warnings, "Chart trendlines and error bars are unsupported");
        }
        series.push(Series {
            name,
            values,
            format,
            props: child(*node, C, "spPr"),
            points,
            labels: child(*node, C, "dLbls").or_else(|| child(element, C, "dLbls")),
            marker: child(*node, C, "marker"),
            smooth: child(*node, C, "smooth")
                .and_then(|n| n.attribute("val"))
                .is_some_and(|v| v == "1" || v == "true"),
        });
    }
    let count = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if categories.len() < count {
        categories.extend((categories.len()..count).map(|i| (i + 1).to_string()));
    }
    for axis in plot
        .children()
        .filter(|n| matches!(n.tag_name().name(), "catAx" | "valAx" | "dateAx" | "serAx"))
    {
        if axis
            .descendants()
            .any(|n| n.has_tag_name((C, "orientation")) && n.attribute("val") == Some("maxMin"))
        {
            warn(warnings, "Reversed chart axis order is unsupported");
        }
    }
    if plot
        .children()
        .filter(|n| n.has_tag_name((C, "valAx")))
        .count()
        > 1
    {
        warn(warnings, "Secondary chart axes are unsupported");
    }
    let vary = child(element, C, "varyColors")
        .map(|n| n.attribute("val").is_none_or(|v| v == "1" || v == "true"))
        .unwrap_or(kind == Kind::Pie);
    Ok(Chart {
        kind,
        grouping,
        plot,
        series,
        categories,
        vary,
    })
}

/// Cached numbers of a `c:val`/`c:cat` reference or literal, by `c:pt/@idx`.
fn numbers(node: Node<'_, '_>) -> (Vec<Option<f64>>, Option<String>) {
    let Some(cache) = node
        .descendants()
        .find(|n| n.has_tag_name((C, "numCache")) || n.has_tag_name((C, "numLit")))
    else {
        return (Vec::new(), None);
    };
    let points: Vec<_> = cache
        .children()
        .filter(|n| n.has_tag_name((C, "pt")))
        .collect();
    let count = child(cache, C, "ptCount")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_POINTS + 1);
    let mut values = vec![None; count];
    for pt in points {
        let Some(idx) = pt.attribute("idx").and_then(|v| v.parse::<usize>().ok()) else {
            continue;
        };
        if idx > MAX_POINTS {
            continue;
        }
        if idx >= values.len() {
            values.resize(idx + 1, None);
        }
        values[idx] = child(pt, C, "v")
            .and_then(|n| n.text())
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite());
    }
    let format = child(cache, C, "formatCode")
        .and_then(|n| n.text())
        .map(str::to_owned);
    (values, format)
}
/// Category labels as text: strings as cached, numbers through their format code.
fn labels(node: Node<'_, '_>, warnings: &mut Warnings) -> Vec<String> {
    // A multi-level cache lists its innermost level (the one next to the axis) first.
    let multi = node
        .descendants()
        .find(|n| n.has_tag_name((C, "multiLvlStrCache")));
    if multi.is_some_and(|m| m.children().filter(|n| n.has_tag_name((C, "lvl"))).count() > 1) {
        warn(
            warnings,
            "Multi-level chart categories show the innermost level only",
        );
    }
    let cache = multi.and_then(|m| child(m, C, "lvl")).or_else(|| {
        node.descendants()
            .find(|n| n.has_tag_name((C, "strCache")) || n.has_tag_name((C, "strLit")))
    });
    if let Some(cache) = cache {
        let mut result = Vec::new();
        for pt in cache.children().filter(|n| n.has_tag_name((C, "pt"))) {
            let Some(idx) = pt.attribute("idx").and_then(|v| v.parse::<usize>().ok()) else {
                continue;
            };
            if idx > MAX_POINTS {
                continue;
            }
            if idx >= result.len() {
                result.resize(idx + 1, String::new());
            }
            result[idx] = child(pt, C, "v")
                .and_then(|n| n.text())
                .map(clip)
                .unwrap_or_default();
        }
        return result;
    }
    let (values, format) = numbers(node);
    values
        .iter()
        .map(|v| {
            v.map(|v| number_text(v, format.as_deref(), warnings))
                .unwrap_or_default()
        })
        .collect()
}

/// Format a value with one of the supported number format codes.
fn number_text(value: f64, code: Option<&str>, warnings: &mut Warnings) -> String {
    let code = code.unwrap_or("General").trim();
    if code.eq_ignore_ascii_case("General") || code.is_empty() {
        return general(value);
    }
    let (body, percent) = match code.strip_suffix('%') {
        Some(body) => (body, true),
        None => (code, false),
    };
    let (integer, decimals) = match body.split_once('.') {
        Some((integer, decimals)) => (integer, Some(decimals)),
        None => (body, None),
    };
    let grouped = match integer {
        "0" => false,
        "#,##0" => true,
        _ => {
            warnings.push(
                Code::Approximation,
                "Unsupported chart number format; General is used",
            );
            return general(value);
        }
    };
    let places = match decimals {
        None => 0,
        Some(d) if (1..=30).contains(&d.len()) && d.chars().all(|c| c == '0') => d.len(),
        Some(_) => {
            warnings.push(
                Code::Approximation,
                "Unsupported chart number format; General is used",
            );
            return general(value);
        }
    };
    let scaled = if percent { value * 100.0 } else { value };
    // Office rounds halves away from zero; `format!` alone rounds them to even.
    let factor = 10f64.powi(places as i32);
    let mut text = format!("{:.*}", places, (scaled * factor).round() / factor);
    if text.starts_with('-') && text[1..].chars().all(|c| c == '0' || c == '.') {
        text.remove(0);
    }
    if grouped {
        text = group_thousands(&text);
    }
    if percent {
        text.push('%');
    }
    text
}
/// Integers as they are; other values rounded to at most 9 significant digits, no exponent.
fn general(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let magnitude = value.abs().log10().floor() as i32;
    let places = (8 - magnitude).clamp(0, 15) as usize;
    let text = format!("{:.*}", places, value);
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    if text == "-0" { "0".into() } else { text }
}
/// Text from the chart part, cut to `MAX_TEXT` characters so labels stay bounded.
fn clip(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}
fn group_thousands(text: &str) -> String {
    let (sign, rest) = text.strip_prefix('-').map_or(("", text), |r| ("-", r));
    let (integer, fraction) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (i, ch) in integer.chars().enumerate() {
        if i > 0 && (integer.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    match fraction {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

/// Synthetic DrawingML shapes in frame coordinates (CSS pixels).
struct Shapes {
    xml: String,
    id: String,
    name: String,
}
fn emu(px: f64) -> i64 {
    (px * EMU).round() as i64
}
fn fill_xml(color: &str) -> String {
    if color == "none" {
        return "<a:noFill/>".into();
    }
    let hex = color.trim_start_matches('#');
    let (rgb, alpha) = hex.split_at(hex.len().min(6));
    let alpha = u8::from_str_radix(alpha, 16)
        .map(|a| {
            format!(
                "<a:alpha val=\"{}\"/>",
                (f64::from(a) / 255.0 * 100_000.0).round()
            )
        })
        .unwrap_or_default();
    format!("<a:solidFill><a:srgbClr val=\"{rgb}\">{alpha}</a:srgbClr></a:solidFill>")
}
fn line_xml(color: Option<&str>, width_px: f64) -> String {
    match color {
        Some(color) if color != "none" => {
            format!("<a:ln w=\"{}\">{}</a:ln>", emu(width_px), fill_xml(color))
        }
        _ => "<a:ln><a:noFill/></a:ln>".into(),
    }
}
#[allow(clippy::too_many_arguments)]
impl Shapes {
    fn open(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.xml.push_str(&format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{}\" name=\"{}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
            self.id,
            self.name,
            emu(x),
            emu(y),
            emu(w.max(0.0)),
            emu(h.max(0.0))
        ));
    }
    fn preset(
        &mut self,
        prst: &str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        fill: &str,
        line: Option<&str>,
        line_px: f64,
    ) {
        self.open(x, y, w, h);
        self.xml.push_str(&format!(
            "<a:prstGeom prst=\"{prst}\"><a:avLst/></a:prstGeom>{}{}</p:spPr></p:sp>",
            fill_xml(fill),
            line_xml(line, line_px)
        ));
    }
    /// An open polyline or closed polygon in frame coordinates.
    fn path(
        &mut self,
        points: &[(f64, f64)],
        closed: bool,
        fill: &str,
        line: Option<&str>,
        line_px: f64,
    ) {
        if points.len() < 2 {
            return;
        }
        let (x0, y0) = points
            .iter()
            .fold((f64::MAX, f64::MAX), |a, p| (a.0.min(p.0), a.1.min(p.1)));
        let (x1, y1) = points
            .iter()
            .fold((f64::MIN, f64::MIN), |a, p| (a.0.max(p.0), a.1.max(p.1)));
        let (w, h) = ((x1 - x0).max(1.0 / EMU), (y1 - y0).max(1.0 / EMU));
        self.open(x0, y0, w, h);
        let mut d = String::new();
        for (i, (x, y)) in points.iter().enumerate() {
            let tag = if i == 0 { "moveTo" } else { "lnTo" };
            d.push_str(&format!(
                "<a:{tag}><a:pt x=\"{}\" y=\"{}\"/></a:{tag}>",
                emu(x - x0),
                emu(y - y0)
            ));
        }
        if closed {
            d.push_str("<a:close/>");
        }
        let mode = if closed { "" } else { " fill=\"none\"" };
        self.xml.push_str(&format!(
            "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"l\" t=\"t\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"{}\" h=\"{}\"{mode}>{d}</a:path></a:pathLst></a:custGeom>{}{}</p:spPr></p:sp>",
            emu(w),
            emu(h),
            if closed { fill_xml(fill) } else { "<a:noFill/>".into() },
            line_xml(line, line_px)
        ));
    }
    /// A pie wedge from `start` sweeping `sweep` degrees clockwise (0 = 3 o'clock).
    fn wedge(
        &mut self,
        cx: f64,
        cy: f64,
        r: f64,
        start: f64,
        sweep: f64,
        fill: &str,
        line: Option<&str>,
        line_px: f64,
    ) {
        self.open(cx - r, cy - r, 2.0 * r, 2.0 * r);
        let size = emu(2.0 * r);
        let (sx, sy) = (
            r + r * start.to_radians().cos(),
            r + r * start.to_radians().sin(),
        );
        let body = if sweep >= 359.999 {
            format!(
                "<a:moveTo><a:pt x=\"{size}\" y=\"{}\"/></a:moveTo><a:arcTo wR=\"{}\" hR=\"{}\" stAng=\"0\" swAng=\"21600000\"/><a:close/>",
                emu(r),
                emu(r),
                emu(r)
            )
        } else {
            format!(
                "<a:moveTo><a:pt x=\"{}\" y=\"{}\"/></a:moveTo><a:lnTo><a:pt x=\"{}\" y=\"{}\"/></a:lnTo><a:arcTo wR=\"{}\" hR=\"{}\" stAng=\"{}\" swAng=\"{}\"/><a:close/>",
                emu(r),
                emu(r),
                emu(sx),
                emu(sy),
                emu(r),
                emu(r),
                (start * 60000.0).round() as i64,
                (sweep * 60000.0).round() as i64
            )
        };
        self.xml.push_str(&format!(
            "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"l\" t=\"t\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"{size}\" h=\"{size}\">{body}</a:path></a:pathLst></a:custGeom>{}{}</p:spPr></p:sp>",
            fill_xml(fill),
            line_xml(line, line_px)
        ));
    }
    /// One line of unwrapped text aligned within the box.
    fn text(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        text: &str,
        font: &Font,
        align: &str,
        anchor: &str,
    ) {
        if text.is_empty() {
            return;
        }
        self.open(x, y, w, h);
        let paragraphs: String = text
            .split('\n')
            .map(|line| {
                format!(
                    "<a:p><a:pPr algn=\"{align}\"/><a:r><a:rPr lang=\"en-US\" sz=\"{}\" b=\"{}\">{}{}</a:rPr><a:t>{}</a:t></a:r></a:p>",
                    font.size.round(),
                    u8::from(font.bold),
                    fill_xml(&font.color),
                    font.faces,
                    escape(line)
                )
            })
            .collect();
        self.xml.push_str(&format!(
            "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap=\"none\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"{anchor}\"><a:noAutofit/></a:bodyPr><a:lstStyle/>{paragraphs}</p:txBody></p:sp>"
        ));
    }
}
/// Rough advance of `text` for layout reservations only; drawing measures real glyphs.
fn text_width(text: &str, px: f64) -> f64 {
    text.lines()
        .map(|line| {
            line.chars()
                .map(|c| if (c as u32) >= 0x2E80 { px } else { px * 0.55 })
                .sum::<f64>()
        })
        .fold(0.0, f64::max)
}

/// Saved scale values of the value axis.
fn saved_scale(plot: Node<'_, '_>) -> (Option<f64>, Option<f64>, Option<f64>) {
    let axis = child(plot, C, "valAx");
    let read = |node: Option<Node<'_, '_>>| {
        node.and_then(|n| n.attribute("val"))
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    let scaling = axis.and_then(|a| child(a, C, "scaling"));
    (
        read(scaling.and_then(|s| child(s, C, "min"))),
        read(scaling.and_then(|s| child(s, C, "max"))),
        read(axis.and_then(|a| child(a, C, "majorUnit"))).filter(|v| *v > 0.0),
    )
}
fn nice_step(range: f64) -> f64 {
    let rough = (range / 10.0).max(f64::MIN_POSITIVE);
    let magnitude = 10f64.powf(rough.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * magnitude)
        .find(|step| *step >= rough)
        .unwrap_or(10.0 * magnitude)
}
/// Axis minimum, maximum, and major unit: saved values win; the rest is computed.
fn scale(
    data_min: f64,
    data_max: f64,
    saved: (Option<f64>, Option<f64>, Option<f64>),
    warnings: &mut Warnings,
) -> (f64, f64, f64) {
    if saved.0.is_none() || saved.1.is_none() || saved.2.is_none() {
        warnings.push(Code::Approximation, SCALE_NOTE);
    }
    let low = saved
        .0
        .unwrap_or(if data_min >= 0.0 { 0.0 } else { data_min });
    let high_guess = saved.1.unwrap_or(if data_max <= 0.0 && data_min < 0.0 {
        0.0
    } else {
        data_max
    });
    let step = saved
        .2
        .unwrap_or_else(|| nice_step((high_guess - low).abs().max(1e-9)));
    let low = if saved.0.is_some() {
        low
    } else {
        (low / step).floor() * step
    };
    // Leave headroom above the largest value (5% of the range), as Office's axes do.
    let mut high = saved.1.unwrap_or_else(|| {
        ((high_guess + 0.05 * (high_guess - low).max(0.0)) / step).ceil() * step
    });
    if high <= low {
        high = low + step;
    }
    (low, high, step)
}

fn accent(index: usize, theme: &Theme) -> String {
    let key = format!("accent{}", index % 6 + 1);
    theme
        .color(&key)
        .map(|c| format!("#{}", c.trim_start_matches('#')))
        .unwrap_or_else(|| format!("#{}", PALETTE[index % 6]))
}
/// Fill and outline colour of a series or point from its `c:spPr`, else the accent colour.
fn paint_of(
    props: Option<Node<'_, '_>>,
    fallback: &str,
    theme: &Theme,
    warnings: &mut Warnings,
) -> (String, Option<String>, f64) {
    let Some(props) = props else {
        return (fallback.into(), None, HAIRLINE_PX);
    };
    for n in props.children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "solidFill" | "noFill" | "ln" | "xfrm" | "prstGeom" => {}
            "gradFill" | "pattFill" | "blipFill" => warn(
                warnings,
                "Chart fills other than solid colours are unsupported",
            ),
            "effectLst" | "effectDag" | "scene3d" | "sp3d" if !n.has_children() => {}
            "effectLst" | "effectDag" | "scene3d" | "sp3d" => {
                warn(warnings, "Chart effects are unsupported")
            }
            other => warn(
                warnings,
                format!("Unsupported chart shape property: {other}"),
            ),
        }
    }
    let fill = if child(props, A, "noFill").is_some() {
        "none".into()
    } else if let Some(solid) = child(props, A, "solidFill") {
        color(solid, fallback, theme, warnings)
    } else {
        fallback.into()
    };
    let (line, width) = match child(props, A, "ln") {
        Some(ln) => {
            let width = number(ln, "w", 9525.0) / EMU;
            if child(ln, A, "noFill").is_some() {
                (Some("none".to_string()), width)
            } else if let Some(solid) = child(ln, A, "solidFill") {
                (Some(color(solid, "#000000", theme, warnings)), width)
            } else {
                (None, width)
            }
        }
        None => (None, HAIRLINE_PX),
    };
    (fill, line, width)
}
/// Text of a data label for one point, or `None` when nothing is shown.
#[allow(clippy::too_many_arguments)]
fn label_text(
    series: &Series<'_>,
    category: &str,
    value: f64,
    total: f64,
    kind: Kind,
    warnings: &mut Warnings,
) -> Option<String> {
    let labels = series.labels?;
    let shown = |tag: &str| {
        child(labels, C, tag)
            .and_then(|n| n.attribute("val"))
            .is_some_and(|v| v == "1" || v == "true")
    };
    if shown("delete") {
        return None;
    }
    let format = child(labels, C, "numFmt")
        .filter(|n| {
            n.attribute("sourceLinked")
                .is_none_or(|v| v == "0" || v == "false")
        })
        .and_then(|n| n.attribute("formatCode"))
        .map(str::to_owned)
        .or_else(|| series.format.clone());
    let mut parts = Vec::new();
    if shown("showSerName") {
        parts.push(series.name.clone());
    }
    if shown("showCatName") {
        parts.push(category.to_owned());
    }
    if shown("showVal") {
        parts.push(number_text(value, format.as_deref(), warnings));
    }
    if shown("showPercent") && kind == Kind::Pie && total > 0.0 {
        parts.push(number_text(value / total, Some("0%"), warnings));
    }
    if parts.is_empty() {
        return None;
    }
    let separator = child(labels, C, "separator")
        .and_then(|n| n.text())
        .unwrap_or(", ")
        .chars()
        .take(MAX_TEXT)
        .collect::<String>();
    Some(parts.join(&separator))
}

#[allow(clippy::too_many_lines)]
fn draw(
    chart: &Chart<'_>,
    root: Node<'_, '_>,
    w: f64,
    h: f64,
    theme: &Theme,
    warnings: &mut Warnings,
    out: &mut Shapes,
) {
    warnings.push(Code::Approximation, LAYOUT_NOTE);
    let chart_node = child(root, C, "chart");
    let base = Font {
        size: 1000.0,
        bold: false,
        color: theme
            .color("tx1")
            .map(|c| format!("#{}", c.trim_start_matches('#')))
            .unwrap_or_else(|| "#000000".into()),
        faces: if theme.has_font("+mn-lt") {
            "<a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/>".into()
        } else {
            String::new()
        },
    }
    .with(text_props(Some(root)), theme, warnings);
    let pad = 7.0;
    let mut area = (pad, pad, w - 2.0 * pad, h - 2.0 * pad);
    // Chart area fill and outline.
    if let Some(props) = child(root, C, "spPr") {
        let (fill, line, line_px) = paint_of(Some(props), "none", theme, warnings);
        out.preset("rect", 0.0, 0.0, w, h, &fill, line.as_deref(), line_px);
    }
    // Title.
    let deleted = chart_node
        .and_then(|n| child(n, C, "autoTitleDeleted"))
        .and_then(|n| n.attribute("val"))
        .is_some_and(|v| v == "1" || v == "true");
    let title_node = chart_node.and_then(|n| child(n, C, "title"));
    let title_text = title_node.map(|t| {
        let rich: Vec<String> = t
            .descendants()
            .filter(|n| n.has_tag_name((A, "p")))
            .map(|p| {
                p.descendants()
                    .filter(|n| n.has_tag_name((A, "t")))
                    .filter_map(|n| n.text())
                    .collect()
            })
            .collect();
        let rich = rich.join("\n");
        if !rich.trim().is_empty() {
            rich
        } else if let Some(v) = t
            .descendants()
            .find(|n| n.has_tag_name((C, "v")))
            .and_then(|n| n.text())
        {
            v.to_owned()
        } else if chart.series.len() == 1 {
            chart.series[0].name.clone()
        } else {
            "Chart Title".into()
        }
    });
    if let (Some(text), false) = (title_text, deleted && title_node.is_none()) {
        let mut title_font = base.clone();
        title_font.size = base.size * 1.2;
        title_font.bold = true;
        let title_font = title_font.with(text_props(title_node), theme, warnings);
        let height = title_font.px() * 1.3 * text.lines().count().max(1) as f64;
        out.text(
            area.0,
            area.1,
            area.2,
            height,
            &text,
            &title_font,
            "ctr",
            "t",
        );
        area.1 += height + pad;
        area.3 -= height + pad;
    }
    // Legend.
    let legend = chart_node.and_then(|n| child(n, C, "legend"));
    let per_point = chart.kind == Kind::Pie || (chart.vary && chart.series.len() == 1);
    let entries: Vec<(String, String)> = if per_point {
        let series = chart.series.first();
        chart
            .categories
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let props = series.and_then(|s| s.points.get(&i).copied().or(s.props));
                (
                    name.clone(),
                    paint_of(props, &accent(i, theme), theme, &mut Warnings::default()).0,
                )
            })
            .collect()
    } else {
        chart
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (fill, line, _) =
                    paint_of(s.props, &accent(i, theme), theme, &mut Warnings::default());
                let swatch = if chart.kind == Kind::Line {
                    line.filter(|l| l != "none").unwrap_or(fill)
                } else {
                    fill
                };
                (s.name.clone(), swatch)
            })
            .collect()
    };
    if let Some(legend) = legend {
        let font = base.with(text_props(Some(legend)), theme, warnings);
        let px = font.px();
        let line_h = px * 1.4;
        let swatch = px * 0.7;
        let position = child(legend, C, "legendPos")
            .and_then(|n| n.attribute("val"))
            .unwrap_or("r");
        let widths: Vec<f64> = entries
            .iter()
            .map(|(n, _)| swatch + px * 0.4 + text_width(n, px))
            .collect();
        let draw_entry =
            |out: &mut Shapes, x: f64, y: f64, (name, fill): &(String, String), width: f64| {
                if chart.kind == Kind::Line && !per_point {
                    out.path(
                        &[(x, y + line_h / 2.0), (x + swatch, y + line_h / 2.0)],
                        false,
                        "none",
                        Some(fill),
                        2.0,
                    );
                } else {
                    out.preset(
                        "rect",
                        x,
                        y + (line_h - swatch) / 2.0,
                        swatch,
                        swatch,
                        fill,
                        None,
                        0.0,
                    );
                }
                out.text(
                    x + swatch + px * 0.4,
                    y,
                    width,
                    line_h,
                    name,
                    &font,
                    "l",
                    "ctr",
                );
            };
        match position {
            "t" | "b" => {
                let gap = px;
                let total: f64 =
                    widths.iter().sum::<f64>() + gap * entries.len().saturating_sub(1) as f64;
                let rows = (total / area.2.max(1.0)).ceil().max(1.0);
                let height = rows * line_h;
                let y0 = if position == "t" {
                    area.1
                } else {
                    area.1 + area.3 - height
                };
                let mut x = area.0 + ((area.2 - total).max(0.0)) / 2.0;
                let mut y = y0;
                for (entry, width) in entries.iter().zip(&widths) {
                    if x + width > area.0 + area.2 + 0.5 && x > area.0 {
                        x = area.0;
                        y += line_h;
                    }
                    draw_entry(out, x, y, entry, *width);
                    x += width + gap;
                }
                if position == "t" {
                    area.1 += height + pad;
                }
                area.3 -= height + pad;
            }
            _ => {
                let width = widths.iter().copied().fold(0.0, f64::max).min(area.2 * 0.4);
                let height = line_h * entries.len() as f64;
                let x = if position == "l" {
                    area.0
                } else {
                    area.0 + area.2 - width
                };
                let y = if position == "tr" {
                    area.1
                } else {
                    area.1 + (area.3 - height).max(0.0) / 2.0
                };
                for (i, entry) in entries.iter().enumerate() {
                    draw_entry(out, x, y + i as f64 * line_h, entry, width);
                }
                if position == "l" {
                    area.0 += width + pad;
                }
                area.2 -= width + pad;
            }
        }
    }
    // Manually laid out plot area (edge mode only), in fractions of the chart.
    let manual = child(chart.plot, C, "layout")
        .and_then(|n| child(n, C, "manualLayout"))
        .and_then(|m| {
            let v = |tag: &str| {
                child(m, C, tag)
                    .and_then(|n| n.attribute("val"))
                    .and_then(|v| v.parse::<f64>().ok())
            };
            let edge =
                |tag: &str| child(m, C, tag).and_then(|n| n.attribute("val")) == Some("edge");
            let inner =
                child(m, C, "layoutTarget").and_then(|n| n.attribute("val")) == Some("inner");
            (edge("xMode") && edge("yMode")).then_some(())?;
            Some(((v("x")? * w, v("y")? * h, v("w")? * w, v("h")? * h), inner))
        });
    if let Some((rect, _)) = manual {
        area = rect;
    }
    if area.2 <= 1.0 || area.3 <= 1.0 {
        return;
    }
    let inner_manual = manual.is_some_and(|(_, inner)| inner);
    match chart.kind {
        Kind::Pie => pie(chart, area, &base, theme, warnings, out),
        _ => axes_chart(chart, area, inner_manual, &base, theme, warnings, out),
    }
}

fn pie(
    chart: &Chart<'_>,
    area: (f64, f64, f64, f64),
    base: &Font,
    theme: &Theme,
    warnings: &mut Warnings,
    out: &mut Shapes,
) {
    let Some(series) = chart.series.first() else {
        return;
    };
    if chart.series.len() > 1 {
        warn(warnings, "Pie charts draw the first series only");
    }
    let element = chart
        .plot
        .children()
        .find(|n| n.has_tag_name((C, "pieChart")));
    let first = element
        .and_then(|e| child(e, C, "firstSliceAng"))
        .map(|n| number(n, "val", 0.0))
        .unwrap_or(0.0);
    if series.points.values().chain(series.props.iter()).any(|n| {
        n.parent()
            .is_some_and(|p| child(p, C, "explosion").is_some())
    }) {
        warn(warnings, "Exploded pie slices are drawn in place");
    }
    let total: f64 = series.values.iter().flatten().filter(|v| **v > 0.0).sum();
    if total <= 0.0 {
        return;
    }
    let r = area.2.min(area.3) / 2.0;
    let (cx, cy) = (area.0 + area.2 / 2.0, area.1 + area.3 / 2.0);
    let font = base.with(text_props(series.labels), theme, warnings);
    let mut angle = first - 90.0;
    let mut labels = Vec::new();
    for (i, value) in series.values.iter().enumerate() {
        let Some(value) = value.filter(|v| *v > 0.0) else {
            continue;
        };
        let sweep = value / total * 360.0;
        let props = series.points.get(&i).copied().or(series.props);
        let fallback = if chart.vary {
            accent(i, theme)
        } else {
            accent(0, theme)
        };
        let (fill, line, line_px) = paint_of(props, &fallback, theme, warnings);
        let line = line.or_else(|| Some("#FFFFFF".into()));
        out.wedge(cx, cy, r, angle, sweep, &fill, line.as_deref(), line_px);
        let category = chart.categories.get(i).map(String::as_str).unwrap_or("");
        if let Some(text) = label_text(series, category, value, total, Kind::Pie, warnings) {
            let middle = (angle + sweep / 2.0).to_radians();
            labels.push((
                cx + r * 0.65 * middle.cos(),
                cy + r * 0.65 * middle.sin(),
                text,
            ));
        }
        angle += sweep;
    }
    for (x, y, text) in labels {
        let width = text_width(&text, font.px()) * 2.0 + 20.0;
        let height = font.px() * 1.4 * text.lines().count().max(1) as f64;
        out.text(
            x - width / 2.0,
            y - height / 2.0,
            width,
            height,
            &text,
            &font,
            "ctr",
            "ctr",
        );
    }
}

#[allow(clippy::too_many_lines)]
fn axes_chart(
    chart: &Chart<'_>,
    area: (f64, f64, f64, f64),
    inner_manual: bool,
    base: &Font,
    theme: &Theme,
    warnings: &mut Warnings,
    out: &mut Shapes,
) {
    let horizontal = chart.kind == Kind::Bar;
    let count = chart.categories.len().max(1);
    let stacked = chart.grouping != Grouping::Clustered;
    // Value range, stacked per category when the bars are stacked.
    let (mut data_min, mut data_max) = (0.0f64, 0.0f64);
    let mut totals = vec![(0.0f64, 0.0f64); count];
    for series in &chart.series {
        for (i, v) in series.values.iter().enumerate() {
            let Some(v) = *v else { continue };
            if stacked && i < count {
                if v >= 0.0 {
                    totals[i].0 += v
                } else {
                    totals[i].1 += v
                }
            } else {
                data_min = data_min.min(v);
                data_max = data_max.max(v);
            }
        }
    }
    if stacked {
        for (pos, neg) in &totals {
            data_max = data_max.max(*pos);
            data_min = data_min.min(*neg);
        }
    }
    if chart
        .series
        .iter()
        .all(|s| s.values.iter().all(Option::is_none))
    {
        data_max = 1.0;
    }
    let percent = chart.grouping == Grouping::Percent;
    let (low, high, step) = if percent {
        let saved = saved_scale(chart.plot);
        scale(
            if data_min < 0.0 { -1.0 } else { 0.0 },
            1.0,
            (
                saved.0.or(Some(if data_min < 0.0 { -1.0 } else { 0.0 })),
                saved.1.or(Some(1.0)),
                saved.2.or(Some(0.1)),
            ),
            warnings,
        )
    } else {
        scale(data_min, data_max, saved_scale(chart.plot), warnings)
    };
    let val_axis = child(chart.plot, C, "valAx");
    let cat_axis = child(chart.plot, C, "catAx").or_else(|| child(chart.plot, C, "dateAx"));
    let hidden = |axis: Option<Node<'_, '_>>| {
        axis.and_then(|a| child(a, C, "delete"))
            .and_then(|n| n.attribute("val"))
            .is_some_and(|v| v == "1" || v == "true")
    };
    let value_format = val_axis
        .and_then(|a| child(a, C, "numFmt"))
        .filter(|n| {
            n.attribute("sourceLinked")
                .is_none_or(|v| v == "0" || v == "false")
        })
        .and_then(|n| n.attribute("formatCode"))
        .map(str::to_owned)
        .or_else(|| {
            if percent {
                Some("0%".into())
            } else {
                chart.series.first().and_then(|s| s.format.clone())
            }
        });
    let ticks: Vec<f64> = {
        let n = (((high - low) / step).round() as i64).clamp(1, 1000);
        (0..=n).map(|i| low + i as f64 * step).collect()
    };
    let value_font = base.with(text_props(val_axis), theme, warnings);
    let cat_font = base.with(text_props(cat_axis), theme, warnings);
    let tick_texts: Vec<String> = ticks
        .iter()
        .map(|t| number_text(*t, value_format.as_deref(), warnings))
        .collect();
    let show_values = !hidden(val_axis) && val_axis.is_some();
    let show_categories = !hidden(cat_axis) && cat_axis.is_some();
    // Reserve room for tick labels unless the saved layout already is the inner plot area.
    let mut plot = area;
    if !inner_manual {
        let value_room = if show_values {
            if horizontal {
                value_font.px() * 1.6
            } else {
                tick_texts
                    .iter()
                    .map(|t| text_width(t, value_font.px()))
                    .fold(0.0, f64::max)
                    + 6.0
            }
        } else {
            0.0
        };
        let category_room = if show_categories {
            if horizontal {
                chart
                    .categories
                    .iter()
                    .map(|t| text_width(t, cat_font.px()))
                    .fold(0.0, f64::max)
                    + 6.0
            } else {
                cat_font.px() * 1.6
            }
        } else {
            0.0
        };
        let (left, bottom) = if horizontal {
            (category_room, value_room)
        } else {
            (value_room, category_room)
        };
        plot = (
            area.0 + left,
            area.1,
            (area.2 - left).max(1.0),
            (area.3 - bottom).max(1.0),
        );
    }
    let (px, py, pw, ph) = plot;
    // Map a value to the plot coordinate along the value axis.
    let along = |v: f64| -> f64 {
        let t = ((v - low) / (high - low)).clamp(-0.05, 1.05);
        if horizontal {
            px + t * pw
        } else {
            py + ph - t * ph
        }
    };
    // Gridlines and tick labels.
    let grid = val_axis.and_then(|a| child(a, C, "majorGridlines"));
    if let Some(grid) = grid {
        let (_, line, width) = paint_of(child(grid, C, "spPr"), "none", theme, warnings);
        let line = line.unwrap_or_else(|| "#D9D9D9".into());
        for t in &ticks {
            let p = along(*t);
            let points = if horizontal {
                [(p, py), (p, py + ph)]
            } else {
                [(px, p), (px + pw, p)]
            };
            out.path(&points, false, "none", Some(&line), width);
        }
    }
    if show_values {
        for (t, text) in ticks.iter().zip(&tick_texts) {
            let p = along(*t);
            let size = value_font.px() * 1.4;
            if horizontal {
                let width = text_width(text, value_font.px()) * 2.0 + 20.0;
                out.text(
                    p - width / 2.0,
                    py + ph + 3.0,
                    width,
                    size,
                    text,
                    &value_font,
                    "ctr",
                    "t",
                );
            } else {
                out.text(
                    area.0 - 50.0,
                    p - size / 2.0,
                    px - area.0 + 50.0 - 4.0,
                    size,
                    text,
                    &value_font,
                    "r",
                    "ctr",
                );
            }
        }
    }
    // Category axis line and labels.
    let band = if horizontal { ph } else { pw } / count as f64;
    let category_at = |i: usize| -> f64 {
        if horizontal {
            py + ph - (i as f64 + 0.5) * band
        } else {
            px + (i as f64 + 0.5) * band
        }
    };
    let zero = along(0.0f64.clamp(low, high));
    if show_categories {
        let (_, line, width) = paint_of(
            cat_axis.and_then(|a| child(a, C, "spPr")),
            "none",
            theme,
            warnings,
        );
        let line = line.unwrap_or_else(|| "#BFBFBF".into());
        let points = if horizontal {
            [(zero, py), (zero, py + ph)]
        } else {
            [(px, zero), (px + pw, zero)]
        };
        out.path(&points, false, "none", Some(&line), width);
    }
    if show_categories {
        for (i, text) in chart.categories.iter().enumerate() {
            let c = category_at(i);
            let size = cat_font.px() * 1.4;
            if horizontal {
                // Right-aligned, so a generous box only grows to the left.
                out.text(
                    area.0 - 50.0,
                    c - size / 2.0,
                    px - area.0 + 46.0,
                    size,
                    text,
                    &cat_font,
                    "r",
                    "ctr",
                );
            } else {
                out.text(
                    c - band / 2.0,
                    py + ph + 3.0,
                    band,
                    size,
                    text,
                    &cat_font,
                    "ctr",
                    "t",
                );
            }
        }
    }
    // Series.
    let element = chart
        .plot
        .children()
        .find(|n| n.has_tag_name((C, "barChart")) || n.has_tag_name((C, "lineChart")));
    let read = |tag: &str, default: f64| {
        element
            .and_then(|e| child(e, C, tag))
            .map(|n| number(n, "val", default))
            .unwrap_or(default)
    };
    let n = chart.series.len().max(1);
    let gap = read("gapWidth", 150.0).clamp(0.0, 500.0) / 100.0;
    let overlap = if stacked {
        1.0
    } else {
        read("overlap", 0.0).clamp(-100.0, 100.0) / 100.0
    };
    let slots = if stacked {
        1.0
    } else {
        n as f64 - (n as f64 - 1.0) * overlap
    };
    let bar = band / (slots + gap);
    let mut offsets = vec![(0.0f64, 0.0f64); count];
    let mut labels = Vec::new();
    let line_marker = element
        .and_then(|e| child(e, C, "marker"))
        .and_then(|n| n.attribute("val"))
        .is_none_or(|v| v == "1" || v == "true");
    for (s, series) in chart.series.iter().enumerate() {
        let fallback = accent(s, theme);
        if chart.kind == Kind::Line {
            let (fill, line, width) = paint_of(series.props, &fallback, theme, warnings);
            let line = line.filter(|l| l != "none").unwrap_or_else(|| {
                if fill == "none" {
                    fallback.clone()
                } else {
                    fill.clone()
                }
            });
            let width = if series.props.and_then(|p| child(p, A, "ln")).is_some() {
                width
            } else {
                2.25 * 96.0 / 72.0
            };
            if series.smooth {
                warnings.push(
                    Code::Approximation,
                    "Smoothed chart lines are drawn straight",
                );
            }
            let mut run = Vec::new();
            let mut points = Vec::new();
            for (i, v) in series.values.iter().enumerate().take(count) {
                match v {
                    Some(v) => {
                        let point = (category_at(i), along(*v));
                        run.push(point);
                        points.push((i, point, *v));
                    }
                    None => {
                        out.path(&run, false, "none", Some(&line), width);
                        run.clear();
                    }
                }
            }
            out.path(&run, false, "none", Some(&line), width);
            let symbol = series
                .marker
                .and_then(|m| child(m, C, "symbol"))
                .and_then(|n| n.attribute("val"))
                .unwrap_or(if line_marker { "auto" } else { "none" });
            if symbol != "none" {
                let size = series
                    .marker
                    .and_then(|m| child(m, C, "size"))
                    .map(|n| number(n, "val", 5.0))
                    .unwrap_or(5.0)
                    .clamp(2.0, 72.0)
                    * 96.0
                    / 72.0;
                let prst = match symbol {
                    "circle" => "ellipse",
                    "square" => "rect",
                    "diamond" => "diamond",
                    "triangle" => "triangle",
                    _ => {
                        warnings.push(Code::Approximation, "Chart marker shapes are approximated");
                        "ellipse"
                    }
                };
                let (marker_fill, marker_line, marker_width) = paint_of(
                    series.marker.and_then(|m| child(m, C, "spPr")),
                    &line,
                    theme,
                    warnings,
                );
                for (_, (x, y), _) in &points {
                    out.preset(
                        prst,
                        x - size / 2.0,
                        y - size / 2.0,
                        size,
                        size,
                        &marker_fill,
                        Some(marker_line.as_deref().unwrap_or(&line)),
                        marker_width,
                    );
                }
            }
            for (i, (x, y), v) in points {
                let category = chart.categories.get(i).map(String::as_str).unwrap_or("");
                if let Some(text) = label_text(series, category, v, 0.0, chart.kind, warnings) {
                    labels.push((x + 4.0, y, text, "l", series.labels));
                }
            }
            continue;
        }
        for (i, v) in series.values.iter().enumerate().take(count) {
            let Some(mut v) = *v else { continue };
            if percent {
                let sum = totals[i].0 - totals[i].1;
                v = if sum > 0.0 { v / sum } else { 0.0 };
            }
            let props = series.points.get(&i).copied().or(series.props);
            let fallback = if chart.vary && chart.series.len() == 1 {
                accent(i, theme)
            } else {
                fallback.clone()
            };
            let (fill, line, width) = paint_of(props, &fallback, theme, warnings);
            let (start, end) = if stacked {
                let slot = if v >= 0.0 {
                    &mut offsets[i].0
                } else {
                    &mut offsets[i].1
                };
                let start = *slot;
                *slot += v;
                (start, *slot)
            } else {
                (0.0f64.clamp(low, high), v)
            };
            let slot = if stacked {
                0.0
            } else {
                s as f64 * (1.0 - overlap)
            };
            let group = bar * slots;
            let along_band = if horizontal {
                py + ph - (i as f64 + 1.0) * band
            } else {
                px + i as f64 * band
            } + (band - group) / 2.0
                + slot * bar;
            let (a, b) = (along(start), along(end));
            let (x, y, bw, bh) = if horizontal {
                (a.min(b), along_band, (a - b).abs(), bar)
            } else {
                (along_band, a.min(b), bar, (a - b).abs())
            };
            out.preset("rect", x, y, bw, bh, &fill, line.as_deref(), width);
            let category = chart.categories.get(i).map(String::as_str).unwrap_or("");
            let shown = series.values[i].unwrap_or(v);
            if let Some(text) = label_text(series, category, shown, 0.0, chart.kind, warnings) {
                let position = series
                    .labels
                    .and_then(|l| child(l, C, "dLblPos"))
                    .and_then(|n| n.attribute("val"))
                    .unwrap_or(if stacked { "ctr" } else { "outEnd" });
                let (base, tip) = (along(start), along(end));
                // Unit step from the bar's base towards its tip, in plot coordinates.
                let outward = (tip - base).signum();
                let center = along_band + bar / 2.0;
                let anchor = match position {
                    "ctr" | "inBase" => (base + tip) / 2.0,
                    "inEnd" => tip - outward * 8.0,
                    _ => tip + outward * 8.0,
                };
                if horizontal {
                    labels.push((anchor, center, text, "c", series.labels));
                } else {
                    labels.push((center, anchor, text, "c", series.labels));
                }
            }
        }
    }
    for (x, y, text, align, source) in labels {
        let font = base.with(text_props(source), theme, warnings);
        let height = font.px() * 1.4;
        let width = text_width(&text, font.px()) * 2.0 + 20.0;
        if align == "l" {
            out.text(x, y - height / 2.0, width, height, &text, &font, "l", "ctr");
        } else {
            out.text(
                x - width / 2.0,
                y - height / 2.0,
                width,
                height,
                &text,
                &font,
                "ctr",
                "ctr",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats_match_exact_codes_only() {
        let mut w = Warnings::default();
        assert_eq!(number_text(1234.5, Some("#,##0"), &mut w), "1,235");
        assert_eq!(number_text(1234.5, Some("#,##0.00"), &mut w), "1,234.50");
        assert_eq!(number_text(0.256, Some("0%"), &mut w), "26%");
        assert_eq!(number_text(0.256, Some("0.0%"), &mut w), "25.6%");
        assert_eq!(number_text(-0.0001, Some("0.0"), &mut w), "0.0");
        assert_eq!(number_text(12.0, None, &mut w), "12");
        assert_eq!(number_text(0.1 + 0.2, Some("General"), &mut w), "0.3");
        assert_eq!(number_text(-1234567.0, Some("#,##0"), &mut w), "-1,234,567");
        assert!(w.into_parts().1.is_empty());
        let mut w = Warnings::default();
        assert_eq!(number_text(3.5, Some("yyyy-mm-dd"), &mut w), "3.5");
        assert_eq!(w.into_parts().1.len(), 1);
    }

    #[test]
    fn saved_scale_values_win_and_missing_ones_are_reported() {
        let mut w = Warnings::default();
        assert_eq!(
            scale(0.0, 170.0, (Some(0.0), Some(200.0), Some(50.0)), &mut w),
            (0.0, 200.0, 50.0)
        );
        assert!(w.into_parts().1.is_empty());
        let mut w = Warnings::default();
        let (low, high, step) = scale(0.0, 170.0, (None, Some(300.0), None), &mut w);
        assert_eq!((low, high), (0.0, 300.0));
        assert!(step > 0.0);
        assert_eq!(w.into_parts().0, vec![SCALE_NOTE.to_string()]);
    }
}
