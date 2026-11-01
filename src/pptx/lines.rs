//! Convert DrawingML line styles and ends into SVG stroke attributes and markers.
use super::*;
use std::collections::BTreeMap;

/// Small XML representation for line inheritance, independent of part namespace prefixes.
#[derive(Clone)]
pub(super) struct Element {
    name: String,
    attrs: BTreeMap<String, String>,
    children: Vec<Element>,
}
impl Element {
    pub(super) fn empty(name: &str) -> Self {
        Self {
            name: name.into(),
            attrs: BTreeMap::new(),
            children: vec![],
        }
    }
    /// An `a:srgbClr` element.
    pub(super) fn rgb(hex: &str) -> Self {
        Self {
            name: "srgbClr".into(),
            attrs: BTreeMap::from([("val".into(), hex.into())]),
            children: vec![],
        }
    }
    pub(super) fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
    pub(super) fn from(node: Node<'_, '_>) -> Self {
        Self {
            name: node.tag_name().name().into(),
            attrs: node
                .attributes()
                .map(|a| (a.name().into(), a.value().into()))
                .collect(),
            children: node
                .children()
                .filter(|n| n.is_element())
                .map(Self::from)
                .collect(),
        }
    }
    fn group(&self) -> &str {
        match self.name.as_str() {
            "noFill" | "solidFill" | "gradFill" | "pattFill" | "blipFill" | "grpFill" => "fill",
            "prstDash" | "custDash" => "dash",
            "round" | "bevel" | "miter" => "join",
            name => name,
        }
    }
    fn overlay(&mut self, other: Self) {
        self.attrs.extend(other.attrs);
        for child in other.children {
            if let Some(index) = self
                .children
                .iter()
                .position(|c| c.group() == child.group())
            {
                if matches!(child.name.as_str(), "headEnd" | "tailEnd") {
                    self.children[index].attrs.extend(child.attrs);
                } else {
                    self.children[index] = child;
                }
            } else {
                self.children.push(child);
            }
        }
    }
    /// Replace every `phClr` scheme colour with `color`, keeping the transforms applied to it.
    pub(super) fn placeholder(&mut self, color: &Self) {
        if self.name == "schemeClr" && self.attrs.get("val").is_some_and(|v| v == "phClr") {
            let transforms = std::mem::take(&mut self.children);
            *self = color.clone();
            self.children.extend(transforms);
        } else {
            for child in &mut self.children {
                child.placeholder(color);
            }
        }
    }
    pub(super) fn xml(&self) -> String {
        let attrs = self
            .attrs
            .iter()
            .map(|(k, v)| format!(" {k}=\"{}\"", escape(v)))
            .collect::<String>();
        format!(
            "<a:{}{attrs}>{}</a:{}>",
            self.name,
            self.children.iter().map(Self::xml).collect::<String>(),
            self.name
        )
    }
}

pub(super) fn resolve(
    direct: &[Node<'_, '_>],
    reference: Option<Node<'_, '_>>,
    theme: &Theme,
    warnings: &mut Warnings,
) -> Option<String> {
    let mut resolved = None;
    if let Some(reference) = reference {
        let index = reference
            .attribute("idx")
            .and_then(|s| s.parse::<usize>().ok());
        if index == Some(0) {
            resolved = Some(Element {
                name: "ln".into(),
                attrs: BTreeMap::new(),
                children: vec![Element {
                    name: "noFill".into(),
                    attrs: BTreeMap::new(),
                    children: vec![],
                }],
            });
        } else if let Some(mut line) = index.and_then(|i| theme.line_style(i)) {
            let color = reference
                .children()
                .find(|n| n.is_element())
                .map(Element::from)
                .unwrap_or_else(|| {
                    warn(
                        warnings,
                        "Theme line reference color is missing; using black",
                    );
                    Element::rgb("000000")
                });
            line.placeholder(&color);
            resolved = Some(line);
        } else {
            warn(
                warnings,
                format!(
                    "Cannot resolve theme line reference {}",
                    reference.attribute("idx").unwrap_or("unspecified")
                ),
            );
        }
    }
    // Apply distant ancestors first and the nearest explicit properties last.
    for node in direct.iter().rev() {
        let line = Element::from(*node);
        if let Some(current) = &mut resolved {
            current.overlay(line);
        } else {
            resolved = Some(line);
        }
    }
    resolved.map(|line| format!("<root xmlns:a=\"{A}\">{}</root>", line.xml()))
}
pub(super) fn decorations(
    line: Node<'_, '_>,
    stroke: &str,
    width: f64,
    svg: &mut String,
    warnings: &mut Warnings,
) -> String {
    for attr in line.attributes() {
        if !matches!(attr.name(), "w" | "cap" | "cmpd" | "algn") {
            warn(
                warnings,
                format!("Unsupported line property: {}", attr.name()),
            );
        }
    }
    let cap = match line.attribute("cap").unwrap_or("flat") {
        "flat" => "butt",
        "rnd" => "round",
        "sq" => "square",
        other => {
            warn(warnings, format!("Unsupported line cap: {other}"));
            "butt"
        }
    };
    let mut attrs = format!("stroke-linecap=\"{cap}\"");
    let join = if child(line, A, "round").is_some() {
        "round"
    } else if child(line, A, "bevel").is_some() {
        "bevel"
    } else {
        "miter"
    };
    attrs.push_str(&format!(" stroke-linejoin=\"{join}\""));
    if let Some(miter) = child(line, A, "miter") {
        attrs.push_str(&format!(
            " stroke-miterlimit=\"{}\"",
            (number(miter, "lim", 800000.0) / 100000.0).max(1.0)
        ));
    }
    if let Some(dash) = child(line, A, "prstDash") {
        let pattern: Option<&[f64]> = match dash.attribute("val").unwrap_or("solid") {
            "solid" => Some(&[]),
            "dot" => Some(&[1.0, 3.0]),
            "dash" => Some(&[4.0, 3.0]),
            "lgDash" => Some(&[8.0, 3.0]),
            "dashDot" => Some(&[4.0, 3.0, 1.0, 3.0]),
            "lgDashDot" => Some(&[8.0, 3.0, 1.0, 3.0]),
            "lgDashDotDot" => Some(&[8.0, 3.0, 1.0, 3.0, 1.0, 3.0]),
            "sysDash" => Some(&[3.0, 1.0]),
            "sysDot" => Some(&[1.0, 1.0]),
            "sysDashDot" => Some(&[3.0, 1.0, 1.0, 1.0]),
            "sysDashDotDot" => Some(&[3.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            _ => None,
        };
        if let Some(pattern) = pattern {
            if !pattern.is_empty() && width > 0.0 {
                attrs.push_str(&format!(
                    " stroke-dasharray=\"{}\"",
                    pattern
                        .iter()
                        .map(|v| (v * width).to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        } else {
            warn(warnings, "Unsupported preset dash pattern");
        }
    }
    if ["headEnd", "tailEnd"].iter().any(|tag| {
        child(line, A, tag).is_some_and(|n| {
            n.attribute("type")
                .is_some_and(|t| matches!(t, "triangle" | "stealth" | "arrow" | "diamond" | "oval"))
        })
    }) {
        // stellar:debt(scope) Arrowheads overlap the shaft. upgrade: shorten shafts for each head geometry
        warnings.push(
            Code::Approximation,
            "Arrowheads are approximate: lines thinner than 2pt use a 2pt reference width for heads, and joins overlap",
        );
    }
    for (tag, side) in [("headEnd", "start"), ("tailEnd", "end")] {
        let Some(end) = child(line, A, tag) else {
            continue;
        };
        let kind = end.attribute("type").unwrap_or("none");
        if kind == "none" || width <= 0.0 {
            continue;
        }
        let dimension = |key| match end.attribute(key).unwrap_or("med") {
            "sm" => 2.0,
            "lg" => 5.0,
            _ => 3.0,
        };
        // Our 0.25-6pt fixtures and LibreOffice comparisons show a constant head reference width below 2pt.
        // Approximate only head dimensions, without changing shaft width.
        let end_scale = (8.0 / 3.0 / width).max(1.0);
        let (length, height) = (dimension("len") * end_scale, dimension("w") * end_scale);
        let middle = height / 2.0;
        let drawing = match kind {
            "triangle" => format!("<path d=\"M0 0 L{length} {middle} L0 {height} Z\"/>"),
            "stealth" => format!(
                "<path d=\"M0 0 L{length} {middle} L0 {height} L{} {middle} Z\"/>",
                length / 3.0
            ),
            "arrow" => format!(
                "<path d=\"M0 0 L{length} {middle} L0 {height}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"1\"/>"
            ),
            "diamond" => format!(
                "<path d=\"M0 {middle} L{} 0 L{length} {middle} L{} {height} Z\"/>",
                length / 2.0,
                length / 2.0
            ),
            "oval" => format!(
                "<ellipse cx=\"{}\" cy=\"{middle}\" rx=\"{}\" ry=\"{middle}\"/>",
                length / 2.0,
                length / 2.0
            ),
            _ => {
                warn(warnings, format!("Unsupported arrowhead: {kind}"));
                continue;
            }
        };
        let id = format!("line-end-{}-{side}", svg.len());
        let reference = if matches!(kind, "diamond" | "oval") {
            length / 2.0
        } else {
            length
        };
        svg.push_str(&format!("<defs><marker id=\"{id}\" markerUnits=\"strokeWidth\" markerWidth=\"{}\" markerHeight=\"{}\" viewBox=\"-1 -1 {} {}\" refX=\"{reference}\" refY=\"{middle}\" orient=\"auto-start-reverse\" overflow=\"visible\"><g fill=\"{stroke}\">{drawing}</g></marker></defs>",length+2.0,height+2.0,length+2.0,height+2.0));
        attrs.push_str(&format!(" marker-{side}=\"url(#{id})\""));
    }
    if line.attribute("cmpd").is_some_and(|v| v != "sng") {
        warn(warnings, "Compound lines are unsupported");
    }
    if line.attribute("algn").is_some_and(|v| v != "ctr") {
        warn(warnings, "Inset lines are approximated as centered lines");
    }
    attrs
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearer_line_overlays_parent_attributes_and_exclusive_groups() {
        let source = format!(
            r#"<root xmlns:d="{A}"><d:ln cap="rnd"><d:noFill/><d:prstDash val="solid"/><d:bevel/><d:headEnd w="lg"/></d:ln><d:ln w="19050"><d:solidFill><d:srgbClr val="FF0000"/></d:solidFill><d:custDash/><d:miter/><d:headEnd type="triangle"/></d:ln></root>"#
        );
        let doc = Document::parse(&source).unwrap();
        let nodes: Vec<_> = doc
            .root_element()
            .children()
            .filter(|n| n.is_element())
            .collect();
        let xml = resolve(&nodes, None, &Theme::default(), &mut Warnings::default()).unwrap();
        let doc = Document::parse(&xml).unwrap();
        let line = child(doc.root_element(), A, "ln").unwrap();
        assert_eq!(line.attribute("w"), Some("19050"));
        assert_eq!(line.attribute("cap"), Some("rnd"));
        assert!(child(line, A, "noFill").is_some());
        assert!(child(line, A, "solidFill").is_none());
        assert!(child(line, A, "prstDash").is_some());
        assert!(child(line, A, "custDash").is_none());
        assert!(child(line, A, "bevel").is_some());
        assert!(child(line, A, "miter").is_none());
        let head = child(line, A, "headEnd").unwrap();
        assert_eq!(head.attribute("type"), Some("triangle"));
        assert_eq!(head.attribute("w"), Some("lg"));
    }
}
