//! Resolve slide -> layout -> master -> theme relationships, colors, and fonts.
use super::*;

#[derive(Default)]
pub(super) struct Theme {
    colors: HashMap<String, String>,
    source: Option<String>,
    line_styles: Vec<lines::Element>,
    /// Each `a:effectStyle` of the theme, in table order.
    effect_styles: Vec<lines::Element>,
    mapping: HashMap<String, String>,
    fonts: HashMap<String, String>,
    hanging: HashMap<String, [Option<bool>; 9]>,
    pub(super) presentation: Option<String>,
}
impl Theme {
    pub(super) fn load(parts: &Parts, slide: &str, warnings: &mut Warnings) -> Result<Self> {
        let mut result = Self::default();
        let roots = relationships(parts, "")?;
        if let Some(presentation) = roots
            .values()
            .find(|r| r.kind.ends_with("/officeDocument") && !r.external)
        {
            let doc = xml(parts, &presentation.target)?;
            if let Some(defaults) = child(doc.root_element(), P, "defaultTextStyle") {
                result.read_hanging("default", defaults);
                result.presentation = Some(doc.input_text().to_owned());
            }
        }
        let mut chain = vec![slide.to_string()];
        for kind in ["/slideLayout", "/slideMaster"] {
            let rels = relationships(parts, chain.last().unwrap())?;
            if let Some(part) = rels
                .values()
                .find(|p| p.kind.ends_with(kind) && !p.external)
            {
                chain.push(part.target.clone());
            } else {
                break;
            }
        }
        let mut theme_path = None;
        let mut master_mapping = HashMap::new();
        // More specific explicit overrides take precedence.
        for name in chain.iter().rev() {
            let doc = xml(parts, name)?;
            if let Some(map) = child(doc.root_element(), P, "clrMap") {
                for attr in map.attributes() {
                    result
                        .mapping
                        .insert(attr.name().into(), attr.value().into());
                }
            }
            if doc.root_element().has_tag_name((P, "sldMaster")) {
                master_mapping = result.mapping.clone();
                if let Some(styles) = child(doc.root_element(), P, "txStyles") {
                    for kind in ["titleStyle", "bodyStyle", "otherStyle"] {
                        if let Some(style) = child(styles, P, kind) {
                            result.read_hanging(kind, style);
                        }
                    }
                }
            }
            if child(doc.root_element(), P, "clrMapOvr")
                .and_then(|n| child(n, A, "masterClrMapping"))
                .is_some()
            {
                result.mapping = master_mapping.clone();
            }
            if let Some(map) = child(doc.root_element(), P, "clrMapOvr")
                .and_then(|n| child(n, A, "overrideClrMapping"))
            {
                for attr in map.attributes() {
                    result
                        .mapping
                        .insert(attr.name().into(), attr.value().into());
                }
            }
            let rels = relationships(parts, name)?;
            if let Some(part) = rels
                .values()
                .find(|p| p.kind.ends_with("/theme") && !p.external)
            {
                theme_path = Some(part.target.clone());
            }
            if rels.values().any(|p| p.kind.ends_with("/themeOverride")) {
                warn(warnings, "themeOverride parts are unsupported");
            }
        }
        if let Some(path) = theme_path {
            let doc = xml(parts, &path)?;
            result.read_theme(doc.root_element());
            result.source = Some(doc.input_text().into());
        }
        Ok(result)
    }
    fn read_hanging(&mut self, key: &str, style: Node<'_, '_>) {
        let fallback = child(style, A, "defPPr").and_then(|n| bool_attr(n, "hangingPunct"));
        let values = std::array::from_fn(|level| {
            child(style, A, &format!("lvl{}pPr", level + 1))
                .and_then(|n| bool_attr(n, "hangingPunct"))
                .or(fallback)
        });
        self.hanging.insert(key.into(), values);
    }
    pub(super) fn hanging_punctuation(&self, kind: &str, level: usize) -> bool {
        self.hanging
            .get(kind)
            .and_then(|levels| levels[level.min(8)])
            .or_else(|| {
                self.hanging
                    .get("default")
                    .and_then(|levels| levels[level.min(8)])
            })
            .unwrap_or(false)
    }
    pub(super) fn line_style(&self, index: usize) -> Option<lines::Element> {
        self.line_styles.get(index.checked_sub(1)?).cloned()
    }
    fn read_theme(&mut self, root: Node<'_, '_>) {
        self.line_styles = root
            .descendants()
            .find(|n| n.has_tag_name((A, "lnStyleLst")))
            .map(|list| {
                list.children()
                    .filter(|n| n.has_tag_name((A, "ln")))
                    .map(lines::Element::from)
                    .collect()
            })
            .unwrap_or_default();
        self.effect_styles = root
            .descendants()
            .find(|n| n.has_tag_name((A, "effectStyleLst")))
            .map(|list| {
                list.children()
                    .filter(|n| n.has_tag_name((A, "effectStyle")))
                    .map(lines::Element::from)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(scheme) = root
            .descendants()
            .find(|n| n.has_tag_name((A, "clrScheme")))
        {
            for entry in scheme.children().filter(|n| n.is_element()) {
                if let Some(c) = entry.children().find(|n| n.is_element()) {
                    let value = if c.has_tag_name((A, "sysClr")) {
                        c.attribute("lastClr")
                    } else if c.has_tag_name((A, "srgbClr")) {
                        c.attribute("val")
                    } else {
                        None
                    };
                    if let Some(rgb) = value.filter(|s| valid_rgb(s)) {
                        self.colors
                            .insert(entry.tag_name().name().into(), format!("#{rgb}"));
                    }
                }
            }
        }
        for (tag, key) in [("majorFont", "+mj"), ("minorFont", "+mn")] {
            if let Some(group) = root.descendants().find(|n| n.has_tag_name((A, tag))) {
                for (script, suffix) in [("latin", "lt"), ("ea", "ea"), ("cs", "cs")] {
                    let direct = child(group, A, script)
                        .and_then(|n| n.attribute("typeface"))
                        .filter(|s| !s.is_empty());
                    let value = direct.or_else(|| {
                        if script == "ea" {
                            group
                                .children()
                                .find(|n| {
                                    n.has_tag_name((A, "font"))
                                        && n.attribute("script") == Some("Jpan")
                                })
                                .and_then(|n| n.attribute("typeface"))
                        } else {
                            None
                        }
                    });
                    if let Some(value) = value {
                        self.fonts.insert(format!("{key}-{suffix}"), value.into());
                    }
                }
            }
        }
    }
    /// The colour element a style reference supplies for `phClr`, or black with a diagnostic.
    fn reference_color(reference: Node<'_, '_>, warnings: &mut Warnings) -> lines::Element {
        reference
            .children()
            .find(|n| n.is_element())
            .map(lines::Element::from)
            .unwrap_or_else(|| {
                warn(
                    warnings,
                    "Theme style reference color is missing; using black",
                );
                lines::Element::rgb("000000")
            })
    }
    /// The theme fill a `fillRef` or `bgRef` selects, as a `<root>` document holding the fill
    /// element with the reference colour substituted for `phClr`, ready for `paint`. `None`
    /// means no fill (index 0 or 1000) or an entry that could not be resolved (reported).
    pub(super) fn fill_reference_xml(
        &self,
        reference: Node<'_, '_>,
        warnings: &mut Warnings,
    ) -> Option<String> {
        let index = number(reference, "idx", 0.0) as usize;
        if index == 0 || index == 1000 {
            return None;
        }
        let Some(source) = &self.source else {
            warn(warnings, "Cannot resolve theme fill: missing theme part");
            return None;
        };
        let doc = Document::parse(source).expect("previously parsed theme XML");
        let (list, position) = if index > 1000 {
            ("bgFillStyleLst", index - 1001)
        } else {
            ("fillStyleLst", index - 1)
        };
        let entry = doc
            .descendants()
            .find(|n| n.has_tag_name((A, list)))
            .and_then(|n| n.children().filter(|n| n.is_element()).nth(position));
        match entry {
            Some(n)
                if matches!(
                    n.tag_name().name(),
                    "noFill" | "solidFill" | "gradFill" | "pattFill"
                ) =>
            {
                let mut fill = lines::Element::from(n);
                fill.placeholder(&Self::reference_color(reference, warnings));
                Some(format!("<root xmlns:a=\"{A}\">{}</root>", fill.xml()))
            }
            Some(n) => {
                warn(
                    warnings,
                    format!(
                        "Theme fill reference {index} is a {} fill, which is unsupported",
                        n.tag_name().name()
                    ),
                );
                None
            }
            None => {
                warn(
                    warnings,
                    format!("Theme fill reference {index} is unresolved"),
                );
                None
            }
        }
    }
    /// The theme effect style an `effectRef` selects, as a `<root>` document holding its
    /// `a:effectLst` with the reference colour substituted, or `None` for index 0, an empty
    /// style, or an unresolved index (reported).
    pub(super) fn effect_reference_xml(
        &self,
        reference: Node<'_, '_>,
        warnings: &mut Warnings,
    ) -> Option<String> {
        let index = number(reference, "idx", 0.0) as usize;
        if index == 0 {
            return None;
        }
        let Some(style) = index.checked_sub(1).and_then(|i| self.effect_styles.get(i)) else {
            warn(
                warnings,
                format!("Theme effect reference {index} is unresolved"),
            );
            return None;
        };
        // 3-D (`scene3d`, `sp3d`) and other members of the style are not drawn; say so.
        for name in style.child_names().filter(|name| *name != "effectLst") {
            warn(
                warnings,
                format!("Theme effect style {index} member is unsupported: {name}"),
            );
        }
        let mut effects = style.find("effectLst")?.clone();
        if !effects.has_children() {
            return None;
        }
        effects.placeholder(&Self::reference_color(reference, warnings));
        Some(format!("<root xmlns:a=\"{A}\">{}</root>", effects.xml()))
    }
    pub(super) fn font(&self, value: &str, warnings: &mut Warnings) -> String {
        if !value.starts_with('+') {
            return value.into();
        }
        match self.fonts.get(value) {
            Some(face) => face.clone(),
            None => {
                warn(warnings, format!("Cannot resolve theme font: {value}"));
                "sans-serif".into()
            }
        }
    }
    /// Whether a theme font reference such as `+mn-lt` resolves.
    pub(super) fn has_font(&self, value: &str) -> bool {
        self.fonts.contains_key(value)
    }
    pub(super) fn color(&self, value: &str) -> Option<&str> {
        let mapped = self
            .mapping
            .get(value)
            .map(String::as_str)
            .unwrap_or(match value {
                "tx1" => "dk1",
                "tx2" => "dk2",
                "bg1" => "lt1",
                "bg2" => "lt2",
                _ => value,
            });
        self.colors.get(mapped).map(String::as_str)
    }
}
pub(super) fn bool_attr(node: Node<'_, '_>, key: &str) -> Option<bool> {
    match node.attribute(key) {
        Some("1" | "true") => Some(true),
        Some("0" | "false") => Some(false),
        _ => None,
    }
}
pub(super) fn valid_rgb(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn master_levels_override_presentation_defaults_without_losing_false() {
        let source = format!(
            r#"<a:root xmlns:a="{A}"><a:defaultTextStyle><a:defPPr hangingPunct="0"/><a:lvl1pPr hangingPunct="1"/></a:defaultTextStyle><a:otherStyle><a:lvl1pPr hangingPunct="0"/><a:lvl2pPr hangingPunct="1"/></a:otherStyle></a:root>"#
        );
        let doc = Document::parse(&source).unwrap();
        let root = doc.root_element();
        let mut theme = Theme::default();
        theme.read_hanging("default", child(root, A, "defaultTextStyle").unwrap());
        theme.read_hanging("otherStyle", child(root, A, "otherStyle").unwrap());
        assert!(!theme.hanging_punctuation("otherStyle", 0));
        assert!(theme.hanging_punctuation("otherStyle", 1));
        assert!(!theme.hanging_punctuation("otherStyle", 2));
        assert!(theme.hanging_punctuation("titleStyle", 0));
    }
    #[test]
    fn theme_resolves_system_color_japanese_font_and_mapping() {
        let source = format!(
            r#"<a:theme xmlns:a="{A}"><a:clrScheme><a:dk1><a:sysClr val="windowText" lastClr="112233"/></a:dk1><a:accent1><a:srgbClr val="ABCDEF"/></a:accent1></a:clrScheme><a:minorFont><a:latin typeface="Example"/><a:ea typeface=""/><a:font script="Jpan" typeface="日本語書体"/></a:minorFont></a:theme>"#
        );
        let doc = Document::parse(&source).unwrap();
        let mut theme = Theme::default();
        theme.read_theme(doc.root_element());
        assert_eq!(theme.color("tx1"), Some("#112233"));
        theme.mapping.insert("tx1".into(), "accent1".into());
        assert_eq!(theme.color("tx1"), Some("#ABCDEF"));
        assert_eq!(theme.font("+mn-ea", &mut Warnings::default()), "日本語書体");
    }
}
