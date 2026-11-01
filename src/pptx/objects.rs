//! Render group coordinate systems and explicitly formatted tables.
use super::*;

pub(super) struct Shape<'a> {
    pub chain: Vec<Node<'a, 'a>>,
    pub master_style: Option<Node<'a, 'a>>,
    /// Whether ancestor groups mirror the coordinate system (an odd number of flips).
    pub mirrored: bool,
    /// Group properties whose fill a child's `a:grpFill` uses (the nearest filled group).
    pub group_fill: Option<Node<'a, 'a>>,
}
pub(super) struct Renderer<'a, 'd> {
    pub parts: &'a Parts,
    pub rels: &'a Relationships,
    pub theme: &'a Theme,
    pub options: &'a RenderOptions<'a>,
    pub svg: &'a mut String,
    pub warnings: &'a mut Warnings,
    pub part: &'a str,
    pub ancestors: &'a [Node<'d, 'd>],
    pub master_style: Option<Node<'d, 'd>>,
    /// Whether ancestor groups mirror the coordinate system (an odd number of flips).
    pub mirrored: bool,
    /// Properties of the enclosing group whose fill `a:grpFill` children use.
    pub group_fill: Option<Node<'d, 'd>>,
}
/// Whether the element's non-visual properties (`cNvPr`) mark it as hidden.
fn is_hidden(node: Node<'_, '_>) -> bool {
    node.children()
        .find_map(|n| child(n, P, "cNvPr"))
        .is_some_and(|n| theme::bool_attr(n, "hidden") == Some(true))
}
fn ole_object<'d>(frame: Node<'d, 'd>) -> Option<Node<'d, 'd>> {
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    let data = child(frame, A, "graphic").and_then(|n| child(n, A, "graphicData"))?;
    child(data, P, "oleObj").or_else(|| {
        child(data, MC, "AlternateContent")
            .and_then(|n| child(n, MC, "Fallback"))
            .and_then(|n| child(n, P, "oleObj"))
    })
}
impl<'d> Renderer<'_, 'd> {
    pub fn node(&mut self, node: Node<'d, 'd>, location: &str, depth: usize) -> Result<()> {
        if depth > 64 {
            bail!("Group nesting is limited to 64 levels");
        }
        let previous = self.warnings.target.clone();
        let identity = node.children().find_map(|n| child(n, P, "cNvPr"));
        self.warnings.target = Target {
            part: self.part.into(),
            location: location.into(),
            object_id: identity.and_then(|n| n.attribute("id")).map(str::to_owned),
            object_name: identity
                .and_then(|n| n.attribute("name"))
                .map(str::to_owned),
        };
        let is_frame = node.has_tag_name((P, "graphicFrame"));
        if node.tag_name().namespace() != Some(P) {
            self.warnings.push(
                Code::Omitted,
                format!("Omitted unsupported element: {}", node.tag_name().name()),
            );
        } else if is_hidden(node) && !(is_frame && ole_object(node).is_some()) {
            // Hidden elements are not drawn (as in Office); this is normal, so no diagnostic.
        } else {
            match node.tag_name().name() {
                "nvGrpSpPr" | "grpSpPr" => {}
                "sp" | "pic" | "cxnSp" => render_shape(
                    Shape {
                        chain: layout::inherited(node, self.ancestors),
                        master_style: self.master_style,
                        mirrored: self.mirrored,
                        group_fill: self.group_fill,
                    },
                    self.parts,
                    self.rels,
                    self.theme,
                    self.options,
                    self.svg,
                    self.warnings,
                )?,
                "grpSp" => self.group(node, location, depth)?,
                "graphicFrame" => {
                    if let Some(ole) = ole_object(node) {
                        self.ole(node, ole)?;
                    } else if let Some(id) = chart::reference(node) {
                        chart::render(self, node, id, location, depth)?;
                    } else {
                        self.table(node, location)?;
                    }
                }
                other => self.warnings.push(
                    Code::Omitted,
                    format!("Omitted unsupported element: {other}"),
                ),
            }
        }
        self.warnings.target = previous;
        Ok(())
    }
    fn group(&mut self, node: Node<'d, 'd>, location: &str, depth: usize) -> Result<()> {
        let props = child(node, P, "grpSpPr").context("Missing group properties")?;
        let t = child(props, A, "xfrm").context("Missing group transform")?;
        let (w, h, mirrored) = transform(t, self.svg)?;
        let off = child(t, A, "chOff").context("Missing group child origin")?;
        let ext = child(t, A, "chExt").context("Missing group child extent")?;
        let cw = number(ext, "cx", 0.0) / EMU;
        let ch = number(ext, "cy", 0.0) / EMU;
        if cw <= 0.0 || ch <= 0.0 {
            bail!("Group child extents must be positive");
        }
        for prop in props.children().filter(|n| n.is_element()) {
            if prop.has_tag_name((A, "extLst")) {
                warn_extensions(prop, self.warnings);
            } else if !prop.has_tag_name((A, "xfrm")) && !layout::is_fill(prop) {
                warn(self.warnings, "Group effect inheritance is unsupported");
            }
        }
        // A group's fill is drawn only by children that ask for it with `a:grpFill`; a group
        // that itself declares `a:grpFill` passes its parent's fill on.
        let group_fill = if child(props, A, "grpFill").is_some() {
            self.group_fill
        } else if layout::has_fill(props) {
            Some(props)
        } else {
            None
        };
        let outer_fill = std::mem::replace(&mut self.group_fill, group_fill);
        self.svg.push_str(&format!(
            "<g transform=\"scale({} {}) translate({} {})\">",
            w / cw,
            h / ch,
            -number(off, "x", 0.0) / EMU,
            -number(off, "y", 0.0) / EMU
        ));
        // An error aborts the whole slide, so the flag needs no restoring on that path.
        self.mirrored ^= mirrored;
        for (i, n) in node.children().filter(|n| n.is_element()).enumerate() {
            self.node(n, &format!("{location}/{i}"), depth + 1)?;
        }
        self.mirrored ^= mirrored;
        self.group_fill = outer_fill;
        self.svg.push_str("</g></g>");
        Ok(())
    }
    /// Display only a static image from the same package, without accessing the OLE payload.
    fn ole(&mut self, frame: Node<'d, 'd>, ole: Node<'d, 'd>) -> Result<()> {
        let kind = ole.attribute("progId").unwrap_or("unknown type");
        let preview = child(ole, P, "pic");
        // OLE is hidden when either the frame or its preview picture is hidden.
        let hidden = is_hidden(frame) || preview.is_some_and(is_hidden);
        if hidden {
            self.warnings.push(
                Code::Omitted,
                format!("OLE ({kind}) is hidden; OLE is never executed"),
            );
            return Ok(());
        }
        let reason = (|| {
            let pic = preview.ok_or("Missing preview image")?;
            let blip = pic
                .descendants()
                .find(|n| n.has_tag_name((A, "blip")))
                .ok_or("Missing preview image reference")?;
            let id =
                blip.attribute((R, "embed"))
                    .ok_or(if blip.attribute((R, "link")).is_some() {
                        "External preview references are not fetched"
                    } else {
                        "Missing preview image reference"
                    })?;
            let rel = self.rels.get(id).ok_or("Missing preview relationship")?;
            if rel.external {
                return Err("External preview references are not fetched");
            }
            if !rel.kind.ends_with("/image") {
                return Err("Preview relationship is not an image");
            }
            let data = self
                .parts
                .get(&rel.target)
                .ok_or("Missing preview image part")?;
            if metafile::is_metafile(data) {
                // Decoded by `render_image`, which reports its own omission.
                return Ok("EMF/WMF");
            }
            if !data.starts_with(b"\x89PNG\r\n\x1a\n") && !data.starts_with(b"\xff\xd8\xff") {
                return Err("Unsupported preview format; only PNG/JPEG/EMF/WMF are supported");
            }
            if imagesize::blob_size(data).is_err() {
                return Err("Invalid preview image");
            }
            Ok("PNG/JPEG")
        })();
        let format = match reason {
            Ok(format) => format,
            Err(reason) => {
                self.warnings.push(
                    Code::Omitted,
                    format!("Omitted OLE ({kind}): {reason}. OLE is never executed"),
                );
                return Ok(());
            }
        };
        let Some(t) = child(frame, P, "xfrm") else {
            self.warnings.push(
                Code::Omitted,
                format!("Missing display transform for OLE ({kind})"),
            );
            return Ok(());
        };
        let (w, h, _) = transform(t, self.svg)?;
        // Do not reapply the inner pic transform; graphicFrame defines the display area.
        let drawn = render_image(
            preview.unwrap(),
            self.parts,
            self.rels,
            self.theme,
            (w, h),
            self.svg,
            self.warnings,
        )?;
        self.svg.push_str("</g>");
        // render_image records its own omission; claim a rendered preview only when drawn.
        if drawn {
            self.warnings.push(
                Code::Approximation,
                format!(
                    "Rendered embedded {format} preview for OLE ({kind}) without executing OLE"
                ),
            );
        }
        Ok(())
    }
    fn table(&mut self, node: Node<'d, 'd>, location: &str) -> Result<()> {
        let table = child(node, A, "graphic")
            .and_then(|n| child(n, A, "graphicData"))
            .and_then(|n| child(n, A, "tbl"));
        let Some(table) = table else {
            self.warnings.push(
                Code::Omitted,
                "Omitted unsupported element: non-table graphicFrame",
            );
            return Ok(());
        };
        let t = child(node, P, "xfrm").context("Missing table transform")?;
        // The frame height only sets the rotation centre: rows keep their stored minimum
        // heights and grow to fit their content, so the table may end above or below it.
        let (w, _, frame_mirrored) = transform(t, self.svg)?;
        // Cell text is un-mirrored like shape text when the net transform is a mirror.
        let mirrored = frame_mirrored != self.mirrored;
        let grid = child(table, A, "tblGrid").context("Missing table column widths")?;
        let widths: Vec<_> = grid
            .children()
            .filter(|n| n.has_tag_name((A, "gridCol")))
            .map(|n| number(n, "w", 0.0) / EMU)
            .collect();
        let rows: Vec<_> = table
            .children()
            .filter(|n| n.has_tag_name((A, "tr")))
            .collect();
        let mut heights: Vec<_> = rows.iter().map(|n| number(*n, "h", 0.0) / EMU).collect();
        // `a:tr/@h` is a minimum height, so zero is valid; negative values are not.
        if widths.is_empty()
            || heights.is_empty()
            || widths.iter().any(|v| *v <= 0.0)
            || heights.iter().any(|v| *v < 0.0)
        {
            bail!("Invalid table row or column dimensions");
        }
        let style_source = table_styles::load(self.parts, table, self.warnings)?;
        let style_doc = style_source.as_deref().map(Document::parse).transpose()?;
        let cell_count = widths
            .len()
            .checked_mul(rows.len())
            .context("Table cell count exceeds the limit")?;
        if cell_count > 100_000 {
            bail!("Tables are limited to 100000 cells");
        }
        // Pass 1: validate merges and record each drawn cell's geometry. Styled cell
        // documents are rebuilt per cell in later passes and dropped right away, so a
        // large table never holds every styled cell at once.
        struct Placed<'n> {
            row: usize,
            col: usize,
            rows: usize,
            x: f64,
            width: f64,
            source: Node<'n, 'n>,
        }
        let mut covered = vec![false; cell_count];
        let mut placed = Vec::new();
        for (row_index, row) in rows.iter().enumerate() {
            let cells: Vec<_> = row
                .children()
                .filter(|n| n.has_tag_name((A, "tc")))
                .collect();
            if cells.len() != widths.len() {
                bail!("Table cell count does not match column count");
            }
            let mut x = 0.0;
            for (col, cell) in cells.iter().enumerate() {
                let left = x;
                x += widths[col];
                self.warnings.target.location =
                    format!("{location}/row/{}/cell/{}", row_index + 1, col + 1);
                let merged = theme::bool_attr(*cell, "hMerge") == Some(true)
                    || theme::bool_attr(*cell, "vMerge") == Some(true);
                if merged != covered[row_index * widths.len() + col] {
                    bail!("Merged cell span does not match continuation cells");
                }
                if merged {
                    continue;
                }
                let span = |key: &str| -> Result<usize> {
                    let value = cell.attribute(key).unwrap_or("1").parse::<usize>()?;
                    if value == 0 {
                        bail!("Merged cell spans must be positive");
                    }
                    Ok(value)
                };
                let cols = span("gridSpan")?;
                let rs = span("rowSpan")?;
                let column_end = col
                    .checked_add(cols)
                    .context("Invalid merged column count")?;
                let row_end = row_index
                    .checked_add(rs)
                    .context("Invalid merged row count")?;
                if column_end > widths.len() {
                    bail!("Merged cell exceeds column bounds");
                }
                if row_end > heights.len() {
                    bail!("Merged cell exceeds row bounds");
                }
                for r in row_index..row_end {
                    for c in col..column_end {
                        let index = r * widths.len() + c;
                        if covered[index] {
                            bail!("Merged cell spans overlap");
                        }
                        if (r, c) != (row_index, col) {
                            covered[index] = true;
                        }
                    }
                }
                placed.push(Placed {
                    row: row_index,
                    col,
                    rows: rs,
                    x: left,
                    width: widths[col..column_end].iter().sum(),
                    source: *cell,
                });
            }
        }
        let styled = |cell: &Placed<'_>, warnings: &mut Warnings| {
            style_doc.as_ref().map(|doc| {
                table_styles::cell(
                    cell.source,
                    doc.root_element(),
                    child(table, A, "tblPr"),
                    (cell.row, cell.col, rows.len(), widths.len()),
                    warnings,
                )
            })
        };
        fn text_body<'a>(cell: Node<'a, 'a>) -> Option<text::TextBody<'a>> {
            child(cell, A, "txBody").map(|body| text::TextBody {
                body,
                inherited: vec![],
                master_style: None,
                cell_props: child(cell, A, "tcPr"),
                style_ref: None,
            })
        }
        // Pass 2: measure cell text and grow rows to fit it. Measurement diagnostics are
        // discarded; the final render below publishes the diagnostics for the final size.
        let mut required = vec![0.0; placed.len()];
        let mut scratch = String::new();
        for (i, cell) in placed.iter().enumerate() {
            let mut scratch_warnings = Warnings::default();
            let source = styled(cell, &mut scratch_warnings);
            let doc = source.as_deref().map(Document::parse).transpose()?;
            let node = doc
                .as_ref()
                .map(Document::root_element)
                .unwrap_or(cell.source);
            if let Some(body) = text_body(node) {
                scratch.clear();
                required[i] = text::render(
                    body,
                    cell.width,
                    heights[cell.row..cell.row + cell.rows].iter().sum(),
                    self.theme,
                    self.options,
                    &mut scratch,
                    &mut scratch_warnings,
                )
                .required_height;
            }
        }
        for (cell, need) in placed.iter().zip(&required) {
            if cell.rows == 1 {
                heights[cell.row] = heights[cell.row].max(*need);
            }
        }
        // Row-spanning cells add any remaining shortfall to their last row.
        for (cell, need) in placed.iter().zip(&required) {
            let end = cell.row + cell.rows;
            let shortfall = need - heights[cell.row..end].iter().sum::<f64>();
            if cell.rows > 1 && shortfall > 0.0 {
                heights[end - 1] += shortfall;
            }
        }
        let tops: Vec<f64> = heights
            .iter()
            .scan(0.0, |y, h| {
                let top = *y;
                *y += h;
                Some(top)
            })
            .collect();
        // Columns still scale to the frame width; rows are not scaled to the frame height.
        self.svg.push_str(&format!(
            "<g transform=\"scale({} 1)\">",
            w / widths.iter().sum::<f64>()
        ));
        // Pass 3: draw backgrounds, borders, and text at the final row heights.
        for placement in &placed {
            self.warnings.target.location = format!(
                "{location}/row/{}/cell/{}",
                placement.row + 1,
                placement.col + 1
            );
            let source = styled(placement, self.warnings);
            let doc = source.as_deref().map(Document::parse).transpose()?;
            let cell = doc
                .as_ref()
                .map(Document::root_element)
                .unwrap_or(placement.source);
            let cw = placement.width;
            let ch: f64 = heights[placement.row..placement.row + placement.rows]
                .iter()
                .sum();
            self.svg.push_str(&format!(
                "<g transform=\"translate({} {})\">",
                placement.x, tops[placement.row]
            ));
            let props = child(cell, A, "tcPr");
            let background = props
                .map(|n| fill(n, "none", self.theme, self.warnings))
                .unwrap_or("none".into());
            self.svg.push_str(&format!(
                "<rect width=\"{cw}\" height=\"{ch}\" fill=\"{background}\"/>"
            ));
            if let Some(props) = props {
                for (edge, x1, y1, x2, y2) in [
                    ("lnL", 0.0, 0.0, 0.0, ch),
                    ("lnR", cw, 0.0, cw, ch),
                    ("lnT", 0.0, 0.0, cw, 0.0),
                    ("lnB", 0.0, ch, cw, ch),
                ] {
                    if let Some(line) = child(props, A, edge) {
                        let color = fill_line(line, self.theme, self.warnings);
                        let width = number(line, "w", 9525.0) / EMU;
                        let decorations =
                            lines::decorations(line, &color, width, self.svg, self.warnings);
                        self.svg.push_str(&format!("<line {decorations} x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"{color}\" stroke-width=\"{width}\"/>"));
                    }
                }
                for n in props.children().filter(|n| n.is_element()) {
                    if !matches!(
                        n.tag_name().name(),
                        "solidFill" | "noFill" | "lnL" | "lnR" | "lnT" | "lnB"
                    ) {
                        warn(
                            self.warnings,
                            format!("Unsupported cell property: {}", n.tag_name().name()),
                        );
                    }
                }
                if props.attribute("vert").is_some_and(|s| s != "horz") {
                    warn(self.warnings, "Vertical table text is unsupported");
                }
            }
            if let Some(body) = text_body(cell) {
                let mut text_svg = String::new();
                text::render(
                    body,
                    cw,
                    ch,
                    self.theme,
                    self.options,
                    &mut text_svg,
                    self.warnings,
                );
                self.svg
                    .push_str(&unmirror_text(text_svg, mirrored, cw / 2.0));
            }
            self.svg.push_str("</g>");
        }
        self.svg.push_str("</g></g>");
        Ok(())
    }
}
/// Open the transform group and return the extent and whether the transform mirrors.
pub(super) fn transform(t: Node<'_, '_>, svg: &mut String) -> Result<(f64, f64, bool)> {
    let off = child(t, A, "off").context("Missing transform origin")?;
    let ext = child(t, A, "ext").context("Missing transform extent")?;
    let (x, y, w, h) = (
        number(off, "x", 0.0) / EMU,
        number(off, "y", 0.0) / EMU,
        number(ext, "cx", 0.0) / EMU,
        number(ext, "cy", 0.0) / EMU,
    );
    if w < 0.0 || h < 0.0 {
        bail!("Transform extent is negative");
    }
    let rotation = number(t, "rot", 0.0) / 60000.0;
    let fx = if theme::bool_attr(t, "flipH") == Some(true) {
        -1
    } else {
        1
    };
    let fy = if theme::bool_attr(t, "flipV") == Some(true) {
        -1
    } else {
        1
    };
    svg.push_str(&format!(
        "<g transform=\"translate({} {}) rotate({rotation}) scale({fx} {fy}) translate({} {})\">",
        x + w / 2.0,
        y + h / 2.0,
        -w / 2.0,
        -h / 2.0
    ));
    Ok((w, h, fx != fy))
}
