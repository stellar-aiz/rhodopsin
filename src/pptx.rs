//! Render OOXML shapes, text, and images with explicit coordinates; warn on unsupported features.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use roxmltree::{Document, Node};
use std::{
    collections::{BTreeSet, HashMap},
    fs::File,
    io::Read,
    path::Path,
};

mod chart;
mod colors;
mod diagnostics;
mod effects;
mod fonts;
mod geometry;
mod gradients;
pub(crate) use fonts::Resolver as FontResolver;
mod layout;
mod lines;
mod metafile;
mod objects;
mod patterns;
mod picture_effects;
mod raster;
mod table_styles;
mod text;
mod vector;
pub use diagnostics::{Code as DiagnosticCode, Diagnostic, FontResolution};
use diagnostics::{Code, Target, Warnings};
mod theme;
use theme::Theme;

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_R: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const ASVG: &str = "http://schemas.microsoft.com/office/drawing/2016/SVG/main";
/// Office 2010 drawing extensions (`a14:hidden*`, `a14:imgProps`, `a14:useLocalDpi`).
const A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";
const EMU: f64 = 9525.0;
const SAVED_CONNECTOR_NOTE: &str = "Connectors with connection-site references use saved geometry; live rerouting is not evaluated";
pub struct Deck {
    pub width: f64,
    pub height: f64,
    pub slides: Vec<Slide>,
    /// One-based deck positions of hidden slides that were not rendered.
    pub skipped_hidden: Vec<usize>,
}
pub struct Slide {
    /// One-based position in the deck, counting hidden slides that were skipped.
    pub number: usize,
    pub hidden: bool,
    /// Package part name of the slide, for diagnostics added after rendering.
    pub part: String,
    pub svg: String,
    pub warnings: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}
struct Part {
    target: String,
    kind: String,
    external: bool,
}
type Parts = HashMap<String, Vec<u8>>;
type Relationships = HashMap<String, Part>;

fn warn(warnings: &mut Warnings, value: impl Into<String>) {
    warnings.push(Code::Unsupported, value);
}
fn xml<'a>(parts: &'a Parts, name: &str) -> Result<Document<'a>> {
    let bytes = parts
        .get(name)
        .with_context(|| format!("Missing PPTX part: {name}"))?;
    Document::parse(std::str::from_utf8(bytes)?)
        .with_context(|| format!("Cannot parse XML: {name}"))
}
fn resolve(base: &str, target: &str) -> Result<String> {
    if target.contains('\\') || target.contains(':') || target.contains('%') {
        bail!("Unsupported part URI: {target}");
    }
    let mut components: Vec<&str> = if target.starts_with('/') {
        vec![]
    } else {
        base.rsplit_once('/')
            .map(|(dir, _)| dir.split('/').collect())
            .unwrap_or_default()
    };
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    bail!("Part URI points outside the package");
                }
            }
            other => components.push(other),
        }
    }
    Ok(components.join("/"))
}
fn relationships(parts: &Parts, base: &str) -> Result<Relationships> {
    let path = match base.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{base}.rels"),
    };
    if !parts.contains_key(&path) {
        return Ok(HashMap::new());
    }
    let doc = xml(parts, &path)?;
    let mut result = HashMap::new();
    for node in doc
        .descendants()
        .filter(|n| n.has_tag_name((PKG_R, "Relationship")))
    {
        let id = node.attribute("Id").context("Relationship is missing Id")?;
        let external = node.attribute("TargetMode") == Some("External");
        let target = node
            .attribute("Target")
            .context("Relationship is missing Target")?;
        result.insert(
            id.into(),
            Part {
                target: if external {
                    target.into()
                } else {
                    resolve(base, target)?
                },
                kind: node.attribute("Type").unwrap_or("").into(),
                external,
            },
        );
    }
    Ok(result)
}
fn child<'a, 'i>(node: Node<'a, 'i>, ns: &str, tag: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name((ns, tag)))
}
fn number(node: Node<'_, '_>, attr: &str, default: f64) -> f64 {
    node.attribute(attr)
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub struct RenderOptions<'a> {
    pub fonts: FontResolver<'a>,
    pub punctuation_compression: bool,
    /// The number the slide being rendered shows for `slidenum` fields, set by [`read`] before
    /// each slide. `None` keeps the saved field text.
    pub slide_number: std::cell::Cell<Option<i64>>,
}

pub fn read(path: &Path, options: &RenderOptions<'_>, include_hidden: bool) -> Result<Deck> {
    let _images = raster::CacheScope::new();
    let file = File::open(path).context("Cannot open input PPTX")?;
    if file.metadata()?.len() > 128 * 1024 * 1024 {
        bail!("Input must not exceed 128 MiB");
    }
    let mut archive = zip::ZipArchive::new(file).context("Cannot read PPTX ZIP archive")?;
    if archive.len() > 10_000 {
        bail!("Part count exceeds the limit");
    }
    // stellar:debt(scope) Skipped entries are never decompressed, so their CRCs are not checked. upgrade: stream-verify skipped entries if archive integrity must be reported.
    let mut names = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index)?;
        if !entry.is_dir() && !names.insert(entry.name().to_string()) {
            bail!("Duplicate part name in ZIP archive");
        }
    }
    // Load relationship parts first, then only the parts a renderer can reach through them.
    // Caps apply to every loaded part; parts that are never read do not count.
    let mut parts = HashMap::new();
    let mut total = 0usize;
    let mut sources = Vec::new();
    for name in &names {
        if let Some(source) = relationship_source(name) {
            load_part(&mut archive, name, &mut parts, &mut total)?;
            sources.push(source);
        }
    }
    let mut reachable = BTreeSet::new();
    // Per source, the internal targets whose relationship type no renderer follows.
    let mut skipped = Vec::new();
    for source in sources {
        // Malformed relationship parts are ignored here; they still fail when a renderer reads them.
        let mut unrendered = HashMap::new();
        for (id, part) in relationships(&parts, &source).unwrap_or_default() {
            if part.external || !names.contains(&part.target) {
                continue;
            }
            if is_unrendered(&part.kind) {
                unrendered.insert(id, part.target);
            } else {
                reachable.insert(part.target);
            }
        }
        if !unrendered.is_empty() {
            skipped.push((source, unrendered));
        }
    }
    for name in reachable {
        if !parts.contains_key(&name) {
            load_part(&mut archive, &name, &mut parts, &mut total)?;
        }
    }
    // Some writers give picture data a relationship type the filter above skips; load what a
    // picture actually references so the picture is rendered or omitted on its own.
    // stellar:debt(scope) Only `a:blip`/`asvg:svgBlip` references override the type filter. upgrade: load parts on demand where a renderer reads them.
    for name in picture_targets(&parts, &skipped) {
        if !parts.contains_key(&name) {
            load_part(&mut archive, &name, &mut parts, &mut total)?;
        }
    }
    let roots = relationships(&parts, "")?;
    let root = roots
        .values()
        .find(|r| r.kind.ends_with("/officeDocument") && !r.external)
        .context("Missing presentation relationship")?;
    let doc = xml(&parts, &root.target)?;
    if !doc.root_element().has_tag_name((P, "presentation")) {
        bail!("Only Transitional PPTX is supported");
    }
    let size = child(doc.root_element(), P, "sldSz").context("Missing slide dimensions")?;
    let width = number(size, "cx", 0.0) / EMU;
    let height = number(size, "cy", 0.0) / EMU;
    if width <= 0.0 || height <= 0.0 || width > 100_000.0 || height > 100_000.0 {
        bail!("Invalid slide dimensions");
    }
    // `firstSlideNum` is an xsd:int that defaults to 1.
    let first_slide_number = doc
        .root_element()
        .attribute("firstSlideNum")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(1);
    let rels = relationships(&parts, &root.target)?;
    let mut slides = Vec::new();
    let mut skipped_hidden = Vec::new();
    for (position, id) in doc
        .descendants()
        .filter(|n| n.has_tag_name((P, "sldId")))
        .enumerate()
    {
        // Count every slide entry, hidden or not: skipped slides are still parsed.
        if position >= 200 {
            bail!("This release supports up to 200 slides");
        }
        let rid = id
            .attribute((R, "id"))
            .context("Missing slide relationship ID")?;
        let part = rels.get(rid).context("Cannot resolve slide relationship")?;
        if part.external || !part.kind.ends_with("/slide") {
            bail!("Invalid slide relationship");
        }
        // Hidden slides keep their place in the numbering even when they are not rendered.
        options
            .slide_number
            .set(Some(first_slide_number + position as i64));
        let Some(mut slide) =
            render_slide(&parts, &part.target, width, height, options, include_hidden)?
        else {
            skipped_hidden.push(position + 1);
            continue;
        };
        slide.number = position + 1;
        slides.push(slide);
    }
    if slides.is_empty() {
        if skipped_hidden.is_empty() {
            bail!("No slides found");
        }
        bail!("All slides are hidden; use --include-hidden to render them");
    }
    Ok(Deck {
        width,
        height,
        slides,
        skipped_hidden,
    })
}

const PART_LIMIT: usize = 32 * 1024 * 1024;
const TOTAL_LIMIT: usize = 256 * 1024 * 1024;
fn load_part(
    archive: &mut zip::ZipArchive<File>,
    name: &str,
    parts: &mut Parts,
    total: &mut usize,
) -> Result<()> {
    let entry = archive.by_name(name)?;
    let mut data = Vec::new();
    entry.take(PART_LIMIT as u64 + 1).read_to_end(&mut data)?;
    *total += data.len();
    if data.len() > PART_LIMIT || *total > TOTAL_LIMIT {
        bail!("Uncompressed PPTX exceeds the limit");
    }
    parts.insert(name.into(), data);
    Ok(())
}
/// Return the source part of a relationship part (`dir/_rels/file.rels` -> `dir/file`).
fn relationship_source(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".rels")?;
    let (dir, file) = match stem.rsplit_once("/_rels/") {
        Some((dir, file)) => (Some(dir), file),
        None => (None, stem.strip_prefix("_rels/")?),
    };
    if file.contains('/') {
        return None;
    }
    Some(dir.map_or_else(|| file.into(), |dir| format!("{dir}/{file}")))
}
/// Skipped targets that their source part references from `a:blip` or `asvg:svgBlip`.
fn picture_targets(
    parts: &Parts,
    skipped: &[(String, HashMap<String, String>)],
) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for (source, unrendered) in skipped {
        // Unloaded or malformed sources are not rendered, so nothing they reference is needed.
        let Ok(doc) = xml(parts, source) else {
            continue;
        };
        for node in doc
            .descendants()
            .filter(|n| n.has_tag_name((A, "blip")) || n.has_tag_name((ASVG, "svgBlip")))
        {
            if let Some(target) = node
                .attribute((R, "embed"))
                .and_then(|id| unrendered.get(id))
            {
                targets.insert(target.clone());
            }
        }
    }
    targets
}
/// Relationship types whose targets no renderer path reads (playback, embeddings, metadata).
fn is_unrendered(kind: &str) -> bool {
    is_media(kind)
        || [
            "/oleObject",
            "/package",
            "/metadata/thumbnail",
            "/font",
            "/hdphoto",
        ]
        .iter()
        .any(|suffix| kind.ends_with(suffix))
}
fn is_media(kind: &str) -> bool {
    ["/video", "/audio", "/media"]
        .iter()
        .any(|suffix| kind.ends_with(suffix))
}

/// Next shape size along one axis when its text area must change by `delta`, or
/// `None` when growing the shape no longer helps. A preset's text area need not
/// scale with the shape (it is often inset by a fraction of the shorter side), so the
/// first step changes the shape by `delta` itself, and later steps divide by the rate
/// the text area actually followed over the previous step. When the text area
/// follows at less than half the shape's change (or shrinks), the step stops
/// instead of driving the shape towards the size limit.
fn autofit_step(size: f64, area: f64, delta: f64, trail: &mut Option<(f64, f64)>) -> Option<f64> {
    let rate = match *trail {
        Some((previous_size, previous_area)) => {
            let rate = (area - previous_area) / (size - previous_size);
            if !rate.is_finite() || rate < 0.5 {
                return None;
            }
            rate.min(1.0)
        }
        None => 1.0,
    };
    *trail = Some((size, area));
    Some((size + delta / rate).clamp(0.0, 100_000.0))
}
/// Cancel a net mirror for text. Flips mirror the geometry, never the glyphs: when the net
/// transform (the element and its ancestor groups) is a mirror, mirror the text once more
/// around the vertical line `center` (the centre of its final text rectangle, in the
/// element's own coordinates). flipH then reads normally and flipV turns the text 180
/// degrees, as LibreOffice does.
fn unmirror_text(text_svg: String, mirrored: bool, center: f64) -> String {
    if mirrored && !text_svg.is_empty() {
        format!(
            "<g transform=\"translate({} 0) scale(-1 1)\">{text_svg}</g>",
            2.0 * center
        )
    } else {
        text_svg
    }
}
fn color(node: Node<'_, '_>, default: &str, theme: &Theme, warnings: &mut Warnings) -> String {
    colors::resolve(node, default, theme, None, warnings)
}
fn fill(node: Node<'_, '_>, default: &str, theme: &Theme, warnings: &mut Warnings) -> String {
    if child(node, A, "noFill").is_some() {
        return "none".into();
    }
    if let Some(solid) = child(node, A, "solidFill") {
        return color(solid, default, theme, warnings);
    }
    if node.children().any(|n| {
        matches!(
            n.tag_name().name(),
            "gradFill" | "pattFill" | "blipFill" | "grpFill"
        )
    }) {
        warn(warnings, "Non-solid fills are unsupported");
    }
    default.into()
}
/// Like `fill`, but also paints `gradFill` and `pattFill` by adding a definition to `svg`.
fn paint(
    node: Node<'_, '_>,
    default: &str,
    theme: &Theme,
    size: (f64, f64),
    rotated: bool,
    svg: &mut String,
    warnings: &mut Warnings,
) -> String {
    if let Some(gradient) = child(node, A, "gradFill") {
        gradients::define(gradient, theme, size, rotated, svg, warnings)
            .unwrap_or_else(|| default.into())
    } else if let Some(pattern) = child(node, A, "pattFill") {
        patterns::define(pattern, theme, svg, warnings)
    } else {
        fill(node, default, theme, warnings)
    }
}
fn render_slide(
    parts: &Parts,
    name: &str,
    width: f64,
    height: f64,
    options: &RenderOptions<'_>,
    include_hidden: bool,
) -> Result<Option<Slide>> {
    let paths = layout::chain(parts, name)?;
    let docs = paths
        .iter()
        .map(|p| xml(parts, p))
        .collect::<Result<Vec<_>>>()?;
    let roots: Vec<_> = docs.iter().map(Document::root_element).collect();
    // `show` is an xsd:boolean attribute on p:sld that defaults to true. The slide is the first
    // part of the chain and is parsed once, so a skipped slide costs no further rendering work.
    let hidden = roots[0]
        .attribute("show")
        .is_some_and(|value| matches!(value.trim(), "0" | "false"));
    if hidden && !include_hidden {
        return Ok(None);
    }
    let mut warnings = Warnings::default();
    warnings.target = Target {
        part: name.into(),
        location: "slide".into(),
        ..Target::default()
    };
    // Layouts and masters draw onto the slide, so their media counts too.
    for path in &paths {
        if relationships(parts, path)?
            .values()
            .any(|part| is_media(&part.kind))
        {
            warn(&mut warnings, "Audio and video playback is unsupported");
            break;
        }
    }
    let theme = Theme::load(parts, name, &mut warnings)?;
    // The nearest part (slide, layout, master) with a background supplies it.
    let background_owner = roots.iter().enumerate().find_map(|(i, r)| {
        child(*r, P, "cSld")
            .and_then(|n| child(n, P, "bg"))
            .map(|n| (i, n))
    });
    let mut background_defs = String::new();
    let mut background = "#ffffff".to_string();
    if let Some((owner, node)) = background_owner {
        if let Some(props) = child(node, P, "bgPr") {
            background = if child(props, A, "blipFill").is_some() {
                let rels = relationships(parts, &paths[owner])?;
                picture_fill(
                    props,
                    parts,
                    &rels,
                    &theme,
                    (width, height),
                    &mut background_defs,
                    &mut warnings,
                )?
            } else {
                paint(
                    props,
                    "#ffffff",
                    &theme,
                    (width, height),
                    false,
                    &mut background_defs,
                    &mut warnings,
                )
            };
        } else if let Some(reference) = child(node, P, "bgRef") {
            // `idx` 1001 and above select `bgFillStyleLst`; the reference supplies `phClr`.
            background = theme.fill_reference(reference, &mut warnings);
        }
        if background == "none" {
            background = "#ffffff".into();
        }
    }
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">{background_defs}<rect width=\"100%\" height=\"100%\" fill=\"{background}\"/>"
    );
    // Paint lower layers first and the slide last. Do not paint template sample text.
    for index in (0..roots.len()).rev() {
        if index > 0 && theme::bool_attr(roots[0], "showMasterSp") == Some(false) {
            continue;
        }
        if index == 2 && theme::bool_attr(roots[1], "showMasterSp") == Some(false) {
            continue;
        }
        let tree = layout::tree(roots[index]).context("Missing shape tree")?;
        let rels = relationships(parts, &paths[index])?;
        let mut renderer = objects::Renderer {
            parts,
            rels: &rels,
            theme: &theme,
            options,
            svg: &mut svg,
            warnings: &mut warnings,
            part: &paths[index],
            ancestors: &roots[index + 1..],
            master_style: roots
                .iter()
                .find(|n| n.has_tag_name((P, "sldMaster")))
                .and_then(|n| child(*n, P, "txStyles")),
            mirrored: false,
            group_fill: None,
        };
        for (position, node) in tree.children().filter(|n| n.is_element()).enumerate() {
            if index > 0 && layout::placeholder(node).is_some() {
                continue;
            }
            renderer.node(node, &format!("spTree/{position}"), 0)?;
        }
    }
    warnings.target = Target {
        part: name.into(),
        location: "slide".into(),
        ..Target::default()
    };
    for n in roots[0].children().filter(|n| n.is_element()) {
        if !matches!(n.tag_name().name(), "cSld" | "clrMapOvr") {
            warn(
                &mut warnings,
                format!("Unsupported slide property: {}", n.tag_name().name()),
            );
        }
    }
    svg.push_str("</svg>");
    // Field notes come from every text box on the slide, layout and master included; report each
    // once per slide.
    warnings.keep_first_per_message(text::DATE_FIELD_NOTE);
    warnings.keep_first_per_message(text::OTHER_FIELD_NOTE);
    // Keep the first connector target while reporting the saved-route policy once per slide.
    warnings.keep_first_per_message(SAVED_CONNECTOR_NOTE);
    let (warnings, diagnostics) = warnings.into_parts();
    Ok(Some(Slide {
        number: 0,
        hidden,
        part: name.into(),
        svg,
        warnings,
        diagnostics,
    }))
}
fn render_shape(
    shape: objects::Shape<'_>,
    parts: &Parts,
    rels: &Relationships,
    theme: &Theme,
    options: &RenderOptions<'_>,
    svg: &mut String,
    warnings: &mut Warnings,
) -> Result<()> {
    let node = shape.chain[0];
    if node.has_tag_name((P, "cxnSp"))
        && node
            .descendants()
            .any(|n| n.has_tag_name((A, "stCxn")) || n.has_tag_name((A, "endCxn")))
    {
        warnings.push(Code::Approximation, SAVED_CONNECTOR_NOTE);
    }
    let props_chain: Vec<_> = shape
        .chain
        .iter()
        .filter_map(|n| child(*n, P, "spPr"))
        .collect();
    let Some(_) = props_chain.first() else {
        warnings.push(Code::Omitted, "Omitted element without shape properties");
        return Ok(());
    };
    let transforms: Vec<_> = props_chain
        .iter()
        .filter_map(|n| child(*n, A, "xfrm"))
        .collect();
    let Some(transform) = transforms.first().copied() else {
        warnings.push(
            Code::Omitted,
            "Omitted element whose inherited transform cannot be resolved",
        );
        return Ok(());
    };
    let (Some(offset), Some(extent)) = (
        transforms.iter().find_map(|n| child(*n, A, "off")),
        transforms.iter().find_map(|n| child(*n, A, "ext")),
    ) else {
        bail!("Missing shape position or dimensions");
    };
    let mut x = number(offset, "x", 0.0) / EMU;
    let mut y = number(offset, "y", 0.0) / EMU;
    let mut w = number(extent, "cx", 0.0) / EMU;
    let mut h = number(extent, "cy", 0.0) / EMU;
    if w < 0.0 || h < 0.0 {
        bail!("Shape dimensions are negative");
    }
    let mut text_svg = String::new();
    // Horizontal centre (in shape coordinates) of the final text rectangle.
    let mut text_center = w / 2.0;
    let rotation = number(transform, "rot", 0.0) / 60_000.0;
    let fx = if matches!(transform.attribute("flipH"), Some("1" | "true")) {
        -1
    } else {
        1
    };
    let fy = if matches!(transform.attribute("flipV"), Some("1" | "true")) {
        -1
    } else {
        1
    };
    let text_mirrored = (fx != fy) != shape.mirrored;
    if let Some(body) = child(node, P, "txBody") {
        // Measure within the shape's text area. Height changes may also change its width,
        // so measure again at the final size. Do not search for a new saved fontScale.
        let geometry_node = props_chain.iter().find_map(|n| geometry_of(*n));
        // Previous (shape size, text area size) per axis, for the autofit step rate.
        let (mut height_trail, mut width_trail) = (None, None);
        for attempt in 0..16 {
            let mut measured_warnings = Warnings::default();
            measured_warnings.target = warnings.target.clone();
            let rect = geometry_node
                .and_then(|n| outline(n, w, h, "", &mut measured_warnings))
                .map(|g| g.text)
                .unwrap_or((0.0, 0.0, w, h));
            text_svg.clear();
            text_center = rect.0 + rect.2 / 2.0;
            text_svg.push_str(&format!(
                "<g transform=\"translate({} {})\">",
                rect.0, rect.1
            ));
            let extent = text::render(
                text::TextBody {
                    body,
                    inherited: shape
                        .chain
                        .iter()
                        .skip(1)
                        .filter_map(|n| child(*n, P, "txBody"))
                        .collect(),
                    master_style: shape.master_style,
                    cell_props: None,
                },
                rect.2,
                rect.3,
                theme,
                options,
                &mut text_svg,
                &mut measured_warnings,
            );
            text_svg.push_str("</g>");
            let growth = extent.height - rect.3;
            let widening = extent.width - rect.2;
            let grow_height = growth > 0.01 && (rect.3 > 0.0 || h == 0.0) && h < 100_000.0;
            let resize_width = widening.abs() > 0.01 && (rect.2 > 0.0 || w == 0.0) && w < 100_000.0;
            let new_height = grow_height
                .then(|| autofit_step(h, rect.3, growth, &mut height_trail))
                .flatten();
            let new_width = resize_width
                .then(|| autofit_step(w, rect.2, widening, &mut width_trail))
                .flatten();
            if (new_height.is_some() || new_width.is_some()) && attempt < 15 {
                if let Some(new_height) = new_height {
                    y += extent.offset_y / growth * (new_height - h);
                    h = new_height;
                }
                if let Some(new_width) = new_width {
                    let change = new_width - w;
                    // Move the box centre along the text's own horizontal axis so the
                    // alignment-side edge stays put, through the text's flips and rotation.
                    let shift = (extent.offset_x / widening + 0.5) * change;
                    let direction = f64::from(fx) * if text_mirrored { -1.0 } else { 1.0 };
                    let (sin, cos) = rotation.to_radians().sin_cos();
                    x += direction * shift * cos - change / 2.0;
                    y += direction * shift * sin;
                    w = new_width;
                }
                continue;
            }
            if growth > 0.01 || widening.abs() > 0.01 {
                warn(
                    warnings,
                    "Shape text autofit reached its limit; text may not fit",
                );
            }
            // Publish diagnostics only for the selected layout, not intermediate sizes.
            for record in measured_warnings.records {
                warnings.measured(record);
            }
            break;
        }
    }
    // Flips mirror the geometry, never the glyphs.
    text_svg = unmirror_text(text_svg, text_mirrored, text_center);
    svg.push_str(&format!(
        "<g transform=\"translate({} {}) rotate({rotation}) scale({fx} {fy}) translate({} {})\">",
        x + w / 2.0,
        y + h / 2.0,
        -w / 2.0,
        -h / 2.0
    ));
    for attr in props_chain
        .iter()
        .flat_map(|n| n.children())
        .filter(|n| n.is_element())
    {
        if attr.has_tag_name((A, "extLst")) {
            warn_extensions(attr, warnings);
        } else if !layout::is_fill(attr)
            && !matches!(
                attr.tag_name().name(),
                "xfrm" | "prstGeom" | "custGeom" | "ln" | "effectLst"
            )
        {
            warn(
                warnings,
                format!("Unsupported shape property: {}", attr.tag_name().name()),
            );
        }
    }
    let style_node = shape.chain.iter().find_map(|n| child(*n, P, "style"));
    if style_node.is_some_and(|n| {
        n.children()
            .any(|c| c.is_element() && !matches!(c.tag_name().name(), "fillRef" | "lnRef"))
    }) {
        warn(
            warnings,
            "Theme effect and text style references are unsupported",
        );
    }
    let effect_count = props_chain
        .iter()
        .find_map(|n| child(*n, A, "effectLst"))
        .map(|n| {
            effects::begin(
                n,
                (w, h, rotation, fx as f64, fy as f64),
                theme,
                svg,
                warnings,
            )
        })
        .unwrap_or(0);
    if node.has_tag_name((P, "pic")) {
        render_image(node, parts, rels, theme, (w, h), svg, warnings)?;
    } else {
        let explicit_fill = props_chain.iter().copied().find(|n| layout::has_fill(*n));
        let fill = if let Some(explicit) = explicit_fill {
            // A shape's own `grpFill` takes the enclosing group's fill (or none).
            let inherited = child(explicit, A, "grpFill").is_some();
            match if inherited {
                shape.group_fill
            } else {
                Some(explicit)
            } {
                Some(props) if child(props, A, "blipFill").is_some() => {
                    picture_fill(props, parts, rels, theme, (w, h), svg, warnings)?
                }
                Some(props) => {
                    if inherited && child(props, A, "gradFill").is_some() {
                        warnings.push(
                            Code::Approximation,
                            "Group gradient fills are drawn per shape, not across the group",
                        );
                    }
                    paint(props, "none", theme, (w, h), rotation != 0.0, svg, warnings)
                }
                None => "none".into(),
            }
        } else if let Some(reference) = style_node.and_then(|n| child(n, A, "fillRef")) {
            theme.fill_reference(reference, warnings)
        } else {
            "none".into()
        };
        let direct_lines: Vec<_> = props_chain
            .iter()
            .filter_map(|n| child(*n, A, "ln"))
            .collect();
        let resolved_line = lines::resolve(
            &direct_lines,
            style_node.and_then(|n| child(n, A, "lnRef")),
            theme,
            warnings,
        );
        let line_doc = resolved_line.as_deref().map(Document::parse).transpose()?;
        let line = line_doc
            .as_ref()
            .and_then(|doc| child(doc.root_element(), A, "ln"));

        let stroke = line
            .map(|n| fill_line(n, theme, warnings))
            .unwrap_or_else(|| "none".into());
        let line_width = line.map(|n| number(n, "w", 9525.0) / EMU).unwrap_or(1.0);
        let decorations = line
            .map(|n| lines::decorations(n, &stroke, line_width, svg, warnings))
            .unwrap_or_default();
        let style = format!(
            "{decorations} fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{line_width}\""
        );

        if let Some(geometry) = props_chain.iter().find_map(|n| geometry_of(*n)) {
            if let Some(preset) = outline(geometry, w, h, &style, warnings) {
                svg.push_str(&preset.svg);
            } else {
                if geometry.descendants().any(|n| n.has_tag_name((A, "gd"))) {
                    warn(warnings, "Shape adjustments are unsupported");
                }
                match geometry.attribute("prst").unwrap_or("") {
                    "bentConnector2" => {
                        svg.push_str(&format!("<path d=\"M0 0 H{w} V{h}\" {style}/>"))
                    }
                    "line" | "straightConnector1" => {
                        svg.push_str(&format!("<line x2=\"{w}\" y2=\"{h}\" {style}/>"))
                    }
                    other => {
                        warnings.push(Code::Omitted, format!("Unsupported preset shape: {other}"))
                    }
                }
            }
        }
        svg.push_str(&text_svg);
    }
    for _ in 0..effect_count {
        svg.push_str("</g>");
    }
    svg.push_str("</g>");
    Ok(())
}
/// The first geometry element (`a:prstGeom` or `a:custGeom`) of shape properties.
fn geometry_of<'a, 'i>(props: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    props
        .children()
        .find(|n| n.has_tag_name((A, "prstGeom")) || n.has_tag_name((A, "custGeom")))
}
/// Evaluated outline of a geometry element, or `None` for presets drawn below.
fn outline(
    node: Node<'_, '_>,
    w: f64,
    h: f64,
    style: &str,
    warnings: &mut Warnings,
) -> Option<geometry::Geometry> {
    if node.has_tag_name((A, "custGeom")) {
        Some(geometry::custom(node, w, h, style, warnings))
    } else {
        geometry::preset(node, w, h, style, warnings)
    }
}
fn fill_line(node: Node<'_, '_>, theme: &Theme, warnings: &mut Warnings) -> String {
    for n in node.children().filter(|n| n.is_element()) {
        if !matches!(
            n.tag_name().name(),
            "solidFill"
                | "noFill"
                | "prstDash"
                | "headEnd"
                | "tailEnd"
                | "round"
                | "bevel"
                | "miter"
        ) {
            warn(
                warnings,
                format!("Unsupported line property: {}", n.tag_name().name()),
            );
        }
    }
    fill(node, "none", theme, warnings)
}
/// Paint the `a:blipFill` of `props` over a `size` box, as an SVG pattern that the shape
/// outline (or background rectangle) clips. The picture is stretched, or repeated through
/// the shared tile pattern when the fill has `a:tile` (with the tile-size bounds of
/// `picture_effects::tile_pattern`). Returns `none` if the picture is omitted.
fn picture_fill(
    props: Node<'_, '_>,
    parts: &Parts,
    rels: &Relationships,
    theme: &Theme,
    size: (f64, f64),
    svg: &mut String,
    warnings: &mut Warnings,
) -> Result<String> {
    // stellar:debt(perf) Each use embeds the picture data again (for example a background on every slide). upgrade: share one definition per part and size.
    let (w, h) = size;
    let mut picture = String::new();
    let fill = child(props, A, "blipFill").context("Missing picture fill")?;
    if !render_image(fill, parts, rels, theme, (w, h), &mut picture, warnings)? {
        return Ok("none".into());
    }
    let id = format!("picture-{}", svg.len());
    svg.push_str(&format!(
        "<defs><pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"{w}\" height=\"{h}\">{picture}</pattern></defs>"
    ));
    Ok(format!("url(#{id})"))
}
/// An image part ready to embed: its `href`, natural size in px (raster only), and
/// diagnostics to record if it is drawn.
struct Picture {
    href: String,
    size: Option<(f64, f64)>,
    notes: Vec<(Code, String)>,
}
/// Data of the internal part that relationship `id` names, or why it cannot be used.
fn embedded<'p>(
    id: &str,
    parts: &'p Parts,
    rels: &'p Relationships,
) -> Result<(&'p str, &'p [u8]), String> {
    let part = rels
        .get(id)
        .ok_or_else(|| format!("Missing image relationship: {id}"))?;
    if part.external {
        return Err("Externally linked images are not loaded".into());
    }
    let data = parts
        .get(&part.target)
        .ok_or_else(|| format!("Missing image part: {}", part.target))?;
    Ok((&part.target, data))
}
fn raster_picture<'p>(
    id: &str,
    parts: &'p Parts,
    rels: &'p Relationships,
) -> Result<Picture, String> {
    let (target, data) = embedded(id, parts, rels)?;
    // Sniff the format before probing dimensions: the probe cannot read EMF/WMF/SVG/WDP,
    // and one undecodable picture must not abort the whole deck.
    let mime = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if data.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if metafile::is_metafile(data) {
        return metafile_picture(target, data);
    } else {
        return Err(format!("Only PNG/JPEG images are supported: {target}"));
    };
    let dimensions = imagesize::blob_size(data)
        .map_err(|_| format!("Cannot read image dimensions: {target}"))?;
    let (width, height) = (dimensions.width, dimensions.height);
    let size = Some(raster::natural_size(data, width as f64, height as f64));
    match raster::prepare(target, data, mime, width, height) {
        Ok(raster::Prepared::Embed) => Ok(Picture {
            href: format!("data:{mime};base64,{}", STANDARD.encode(data)),
            size,
            notes: vec![],
        }),
        Ok(raster::Prepared::Downscaled {
            href,
            width: tw,
            height: th,
        }) => Ok(Picture {
            href: href.to_string(),
            size,
            notes: vec![(
                Code::Approximation,
                format!(
                    "{}: {target}",
                    raster::downscaled((width, height), (tw, th))
                ),
            )],
        }),
        Err(reason) => Err(format!("{reason}: {target}")),
    }
}
/// An EMF or placeable WMF picture, translated to SVG and checked by the SVG picture gate.
/// Its bitmaps were encoded under the PNG/JPEG size policy during translation.
// stellar:debt(perf) a layout or master metafile is translated again on every slide. upgrade: cache by part like raster downscales.
fn metafile_picture(target: &str, data: &[u8]) -> Result<Picture, String> {
    let translated = metafile::translate(data).map_err(|reason| format!("{reason}: {target}"))?;
    // Text records are reported by the translator, which writes no `<text>`.
    vector::check_generated(&translated.svg).map_err(|reason| format!("{reason}: {target}"))?;
    Ok(Picture {
        href: svg_href(&translated.svg),
        size: Some(translated.size),
        notes: translated
            .notes
            .into_iter()
            .map(|(code, message)| (code, format!("{message}: {target}")))
            .collect(),
    })
}
fn svg_href(svg: &[u8]) -> String {
    format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg))
}
/// An SVG picture; vector data bypasses the PNG/JPEG pixel policy and uses its own gate.
// stellar:debt(perf) a layout or master SVG is checked and encoded again on every slide. upgrade: cache by part like raster downscales.
fn svg_picture<'p>(id: &str, parts: &'p Parts, rels: &'p Relationships) -> Result<Picture, String> {
    let (target, data) = embedded(id, parts, rels)?;
    let has_text = vector::check(data).map_err(|reason| format!("{reason}: {target}"))?;
    let mut notes = vec![];
    if has_text {
        notes.push((
            Code::Unsupported,
            format!("Text inside SVG pictures may not render: {target}"),
        ));
    }
    Ok(Picture {
        href: svg_href(data),
        size: None,
        notes,
    })
}
/// Report each extension element of `list` that changes rendering.
fn warn_extensions(list: Node<'_, '_>, warnings: &mut Warnings) {
    for name in unsupported_shape_extensions(list) {
        warn(
            warnings,
            format!("Unsupported shape property extension: {name}"),
        );
    }
}
/// Extension elements of `list` (an `a:extLst`) whose `(namespace, local name)` is not in
/// `allow`. `a:extLst` holds only `a:ext` wrappers (ECMA-376 Part 1 §20.1.2.2.15).
fn unsupported_extensions<'a, 'i>(list: Node<'a, 'i>, allow: &[(&str, &str)]) -> Vec<Node<'a, 'i>> {
    list.children()
        .filter(|n| n.has_tag_name((A, "ext")))
        .flat_map(|ext| ext.children().filter(|n| n.is_element()))
        .filter(|n| {
            let name = n.tag_name();
            !allow
                .iter()
                .any(|(ns, local)| name.namespace() == Some(ns) && name.name() == *local)
        })
        .collect()
}
/// Shape-property extensions that change rendering, by qualified name.
/// The Office 2010 `a14:hidden*` extensions keep the fill, line, effects, or 3-D settings
/// a shape had before they were switched off; a renderer that honours the visible
/// properties ignores them, so they are not reported.
fn unsupported_shape_extensions(list: Node<'_, '_>) -> Vec<String> {
    let allow = [
        (A14, "hiddenFill"),
        (A14, "hiddenLine"),
        (A14, "hiddenEffects"),
        (A14, "hiddenScene3d"),
        (A14, "hiddenSp3d"),
    ];
    let mut names = Vec::new();
    for element in unsupported_extensions(list, &allow) {
        let name = element.tag_name();
        let qualified = match name.namespace().and_then(|ns| element.lookup_prefix(ns)) {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}:{}", name.name()),
            _ => name.name().to_string(),
        };
        if !names.contains(&qualified) {
            names.push(qualified);
        }
    }
    names
}
/// `asvg:svgBlip` children of `a:blip/a:extLst/a:ext` (Office 2016 SVG pictures).
fn svg_blips<'a, 'i>(blip: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    child(blip, A, "extLst")
        .into_iter()
        .flat_map(|list| list.children().filter(|n| n.has_tag_name((A, "ext"))))
        .filter_map(|ext| child(ext, ASVG, "svgBlip"))
}
fn render_image(
    node: Node<'_, '_>,
    parts: &Parts,
    rels: &Relationships,
    theme: &Theme,
    (w, h): (f64, f64),
    svg: &mut String,
    warnings: &mut Warnings,
) -> Result<bool> {
    let Some(blip) = node.descendants().find(|n| n.has_tag_name((A, "blip"))) else {
        warnings.push(Code::Omitted, "Missing image reference");
        return Ok(false);
    };
    let svg_blip = svg_blips(blip).next();
    let raster_id = blip.attribute((R, "embed"));
    let svg_id = svg_blip.and_then(|n| n.attribute((R, "embed")));
    let Some(first) = raster_id.or(svg_id) else {
        let linked = [Some(blip), svg_blip]
            .into_iter()
            .flatten()
            .any(|n| n.attribute((R, "link")).is_some());
        warnings.push(
            Code::Omitted,
            if linked {
                "Externally linked images are not loaded"
            } else {
                "Missing image reference"
            },
        );
        return Ok(false);
    };
    let crop = node.descendants().find(|n| n.has_tag_name((A, "srcRect")));
    let values: Vec<_> = ["l", "t", "r", "b"]
        .iter()
        .map(|key| crop.map(|n| number(n, key, 0.0) / 100000.0).unwrap_or(0.0))
        .collect();
    let (left, top, right, bottom) = (values[0], values[1], values[2], values[3]);
    let name = rels.get(first).map_or(first, |part| part.target.as_str());
    if left + right >= 1.0 || top + bottom >= 1.0 {
        warnings.push(
            Code::Omitted,
            format!("Image crop leaves no visible area: {name}"),
        );
        return Ok(false);
    }
    let blip_fill = blip.parent().filter(|n| n.tag_name().name() == "blipFill");
    let tile = blip_fill.and_then(|n| child(n, A, "tile"));
    let rect = picture_effects::fill_rect(blip_fill, w, h);
    if tile.is_none() && (rect.2 <= 0.0 || rect.3 <= 0.0) {
        warnings.push(
            Code::Omitted,
            format!("Image fill rectangle leaves no visible area: {name}"),
        );
        return Ok(false);
    }
    // Exactly one outcome per picture: draw the first usable candidate, or omit once.
    // The raster reference comes first: it is what Office writes as the compatible fallback,
    // and keeps already-rendered pictures unchanged. The SVG is used when it is absent or fails.
    type Load = for<'p> fn(&str, &'p Parts, &'p Relationships) -> Result<Picture, String>;
    let candidates: [(Option<&str>, Load); 2] =
        [(raster_id, raster_picture), (svg_id, svg_picture)];
    let mut reasons = Vec::new();
    let mut picture = None;
    for (id, load) in candidates {
        let Some(id) = id else { continue };
        match load(id, parts, rels) {
            Ok(loaded) => {
                picture = Some(loaded);
                break;
            }
            Err(reason) => reasons.push(reason),
        }
    }
    let Some(picture) = picture else {
        warnings.push(Code::Omitted, reasons.join("; "));
        return Ok(false);
    };
    let seed = svg.len();
    let mut defs = String::new();
    let filter = picture_effects::filter(blip, theme, (w, h), seed, &mut defs, warnings);
    for (code, message) in picture.notes {
        warnings.push(code, message);
    }
    let crop = [left, top, right, bottom];
    let full = rect == (0.0, 0.0, w, h);
    // A full-frame picture keeps the single clipped `<image>` it always had.
    let stretched = || picture_effects::image(&picture.href, crop, rect, !full);
    let body = match tile {
        Some(tile) => {
            let natural = picture.size.unwrap_or_else(|| {
                warnings.push(
                    Code::Approximation,
                    "Tiled SVG pictures use the picture frame as the tile size",
                );
                (w, h)
            });
            match picture_effects::tile_pattern(
                tile,
                &picture.href,
                natural,
                crop,
                (w, h),
                seed,
                &mut defs,
            ) {
                Some(id) => format!("<rect width=\"{w}\" height=\"{h}\" fill=\"url(#{id})\"/>"),
                None => {
                    warnings.push(
                        Code::Approximation,
                        format!("Picture tile size is out of range; drawn stretched: {name}"),
                    );
                    stretched()
                }
            }
        }
        None => stretched(),
    };
    if !defs.is_empty() {
        svg.push_str("<defs>");
        svg.push_str(&defs);
        svg.push_str("</defs>");
    }
    if let Some(id) = &filter {
        svg.push_str(&format!("<g filter=\"url(#{id})\">"));
    }
    // Clip to the nested SVG viewport; rotate and flip in the outer shape coordinates.
    svg.push_str(&format!(
        "<svg width=\"{w}\" height=\"{h}\" overflow=\"hidden\">{body}</svg>"
    ));
    if filter.is_some() {
        svg.push_str("</g>");
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_parts_stay_in_package() {
        assert_eq!(
            resolve("ppt/slides/slide1.xml", "../media/image1.png").unwrap(),
            "ppt/media/image1.png"
        );
        assert!(resolve("ppt/slides/slide1.xml", "../../../outside").is_err());
        assert!(resolve("ppt/slides/slide1.xml", "https://example.com/a").is_err());
    }
    #[test]
    fn xml_text_is_escaped() {
        assert_eq!(escape("<&\""), "&lt;&amp;&quot;");
    }
}
