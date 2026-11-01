//! Load-order-independent font selection shared by measurement and rendering.
use super::diagnostics::{FontResolution, Warnings};
use resvg::usvg::fontdb::{Database, ID, Style, Weight};
use skrifa::{FontRef, MetadataProvider};
use std::{cell::RefCell, collections::HashMap};

type SelectionKey = (String, bool, bool, String);
pub(crate) struct Resolver<'a> {
    pub database: &'a Database,
    mapping: &'a HashMap<String, String>,
    coverage: RefCell<HashMap<(ID, char), bool>>,
    selections: RefCell<HashMap<SelectionKey, Option<ID>>>,
}
impl<'a> Resolver<'a> {
    pub fn new(database: &'a Database, mapping: &'a HashMap<String, String>) -> Self {
        Self {
            database,
            mapping,
            coverage: RefCell::new(HashMap::new()),
            selections: RefCell::new(HashMap::new()),
        }
    }
    fn has(&self, id: ID, ch: char) -> bool {
        if ch == '\n' {
            return true;
        }
        if let Some(found) = self.coverage.borrow().get(&(id, ch)) {
            return *found;
        }
        let found = self
            .database
            .with_face_data(id, |data, index| {
                FontRef::from_index(data, index)
                    .ok()
                    .is_some_and(|f| f.charmap().map(ch).is_some())
            })
            .unwrap_or(false);
        self.coverage.borrow_mut().insert((id, ch), found);
        found
    }
    fn named(&self, id: ID, family: &str) -> bool {
        self.database.face(id).is_some_and(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(family))
        })
    }
    /// The face belongs to another family than the requested one. Drives both the
    /// `family_missing` diagnostic and the substituted line metrics in text layout.
    pub(super) fn substitutes(&self, id: ID, family: &str) -> bool {
        !self.named(id, family)
    }
    fn best(&self, family: Option<&str>, value: &str, bold: bool, italic: bool) -> Option<ID> {
        let weight = if bold { Weight::BOLD } else { Weight::NORMAL };
        let style = if italic { Style::Italic } else { Style::Normal };
        self.database
            .faces()
            .filter(|f| family.is_none_or(|name| self.named(f.id, name)))
            .min_by_key(|f| {
                let missing = value.chars().filter(|ch| !self.has(f.id, *ch)).count();
                let priority = [
                    "Noto Sans CJK JP",
                    "Noto Sans JP",
                    "Yu Gothic",
                    "Hiragino Sans",
                    "Meiryo",
                    "Arial Unicode MS",
                    "Arial",
                ]
                .iter()
                .position(|name| self.named(f.id, name))
                .unwrap_or(7);
                (
                    missing,
                    f.style != style,
                    f.weight.0.abs_diff(weight.0),
                    priority,
                    f.post_script_name.clone(),
                    f.families.first().map(|v| v.0.clone()),
                    f.index,
                )
            })
            .map(|f| f.id)
    }
    pub(super) fn select(
        &self,
        family: &str,
        value: &str,
        bold: bool,
        italic: bool,
        warnings: &mut Warnings,
    ) -> Option<ID> {
        let key = (family.to_owned(), bold, italic, value.to_owned());
        let mapped = self.mapping.get(family).map(String::as_str);
        let cached = self.selections.borrow().get(&key).copied();
        let id = cached.unwrap_or_else(|| {
            let id = mapped
                .and_then(|name| self.best(Some(name), value, bold, italic))
                .or_else(|| self.best(Some(family), value, bold, italic))
                .or_else(|| self.best(None, value, bold, italic));
            self.selections.borrow_mut().insert(key, id);
            id
        });
        if let Some(id) = id {
            let reason = if mapped.is_some_and(|name| self.named(id, name)) {
                Some("explicit_mapping")
            } else if self.substitutes(id, family) {
                Some("family_missing")
            } else {
                None
            };
            if let Some(reason) = reason {
                self.record(family, id, reason, warnings);
            }
            let f = self.database.face(id).unwrap();
            if f.style != if italic { Style::Italic } else { Style::Normal }
                || f.weight != if bold { Weight::BOLD } else { Weight::NORMAL }
            {
                self.record(family, id, "style_approximation", warnings);
            }
            if mapped.is_some() && !mapped.is_some_and(|name| self.named(id, name)) {
                self.record(family, id, "mapped_family_missing", warnings);
            }
        }
        if id.is_none() {
            warnings.missing_font(family);
        }
        id
    }
    pub(super) fn fallback(
        &self,
        family: &str,
        ch: char,
        bold: bool,
        italic: bool,
        warnings: &mut Warnings,
    ) -> Option<ID> {
        let key = (String::new(), bold, italic, ch.to_string());
        let cached = self.selections.borrow().get(&key).copied();
        let id = cached.unwrap_or_else(|| {
            let id = self
                .best(None, &ch.to_string(), bold, italic)
                .filter(|id| self.has(*id, ch));
            self.selections.borrow_mut().insert(key, id);
            id
        });
        if let Some(id) = id {
            self.record_character(family, id, "glyph_fallback", Some(ch), warnings);
            let f = self.database.face(id).unwrap();
            if f.style != if italic { Style::Italic } else { Style::Normal }
                || f.weight != if bold { Weight::BOLD } else { Weight::NORMAL }
            {
                self.record_character(family, id, "style_approximation", Some(ch), warnings);
            }
        }
        id
    }
    fn record(&self, requested: &str, id: ID, reason: &str, warnings: &mut Warnings) {
        self.record_character(requested, id, reason, None, warnings);
    }
    fn record_character(
        &self,
        requested: &str,
        id: ID,
        reason: &str,
        character: Option<char>,
        warnings: &mut Warnings,
    ) {
        let face = self.database.face(id).unwrap();
        let resolution = FontResolution {
            requested_family: requested.into(),
            actual_family: face
                .families
                .first()
                .map(|f| f.0.clone())
                .unwrap_or_default(),
            post_script_name: face.post_script_name.clone(),
            face_index: face.index,
            reason: reason.into(),
        };
        if character.is_some() {
            warnings.font_character(resolution, character);
        } else {
            warnings.font(resolution);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FONTS: [&[u8]; 5] = [
        include_bytes!("../../tests/fixtures/latin.ttf"),
        include_bytes!("../../tests/fixtures/cjk.ttf"),
        include_bytes!("../../tests/fixtures/cjk-bold.ttf"),
        include_bytes!("../../tests/fixtures/cjk-italic.ttf"),
        include_bytes!("../../tests/fixtures/other.ttf"),
    ];
    #[test]
    fn coverage_style_and_ties_are_independent_of_load_order() {
        for reverse in [false, true] {
            let mut db = Database::new();
            let mut data = FONTS.to_vec();
            if reverse {
                data.reverse();
            }
            for font in data {
                db.load_font_data(font.to_vec());
            }
            let mapping = HashMap::new();
            let resolver = Resolver::new(&db, &mapping);
            for (bold, italic, suffix) in [
                (false, false, "Regular"),
                (true, false, "Bold"),
                (false, true, "Italic"),
            ] {
                let mut warnings = Warnings::default();
                let id = resolver
                    .select("不在", "Aあ", bold, italic, &mut warnings)
                    .unwrap();
                assert_eq!(
                    db.face(id).unwrap().post_script_name,
                    format!("PptxCJK-{suffix}")
                );
                assert_eq!(
                    warnings.records[0].font.as_ref().unwrap().reason,
                    "family_missing"
                );
                assert!(
                    !warnings
                        .records
                        .iter()
                        .any(|d| d.font.as_ref().unwrap().reason == "style_approximation")
                );
            }
            let mut warnings = Warnings::default();
            let id = resolver
                .select("Pptx Latin", "Aあ", false, false, &mut warnings)
                .unwrap();
            assert_eq!(db.face(id).unwrap().post_script_name, "PptxLatin-Regular");
            assert!(warnings.is_empty());
            assert!(
                resolver
                    .fallback("Pptx Latin", 'あ', false, false, &mut warnings)
                    .is_some()
            );
            assert_eq!(
                warnings.records[0].font.as_ref().unwrap().reason,
                "glyph_fallback"
            );
            assert!(
                resolver
                    .fallback("Pptx Latin", '未', false, false, &mut warnings)
                    .is_none()
            );
        }
    }
    #[test]
    fn explicit_mapping_wins_and_missing_style_is_visible() {
        let mut db = Database::new();
        for font in FONTS {
            db.load_font_data(font.to_vec());
        }
        let mapping = HashMap::from([("Pptx CJK".into(), "Pptx Other".into())]);
        let resolver = Resolver::new(&db, &mapping);
        let mut warnings = Warnings::default();
        let id = resolver
            .select("Pptx CJK", "Aあ", true, false, &mut warnings)
            .unwrap();
        assert_eq!(db.face(id).unwrap().post_script_name, "PptxOther-Regular");
        assert_eq!(warnings.records.len(), 2);
        assert_eq!(
            warnings.records[1].font.as_ref().unwrap().reason,
            "style_approximation"
        );
        let size = resolver.coverage.borrow().len();
        resolver.select("Pptx CJK", "Aあ", true, false, &mut warnings);
        assert_eq!(size, resolver.coverage.borrow().len());
        assert_eq!(warnings.records.len(), 2);
    }
}
