//! 開発専用。三者のページ幾何、抽出文字、文字・罫線・塗りの位置を比較する。
//! 紙面全体の合否は判定しない。
#[path = "xlsx-compare-pages/layout.rs"]
mod layout;
use lopdf::{Document, Object, ObjectId};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};
type Result<T> = std::result::Result<T, &'static str>;
#[derive(Clone, Debug, PartialEq, Serialize)]
struct Geometry {
    media_box_pt: [f64; 4],
    crop_box_pt: [f64; 4],
    rotation: i64,
}
#[derive(Serialize)]
struct Observation {
    sha256: String,
    pages: Vec<Geometry>,
    /// 康熙部首・部首補助だけを統合漢字へ戻した抽出文字。
    extracted_text: Vec<TextFingerprint>,
    raw_extracted_text: Vec<TextFingerprint>,
    /// 文字列を含むため出力しない。
    #[serde(skip)]
    layout: Vec<layout::PageLayout>,
}
impl Geometry {
    /// Excel for Mac は MediaBox を整数ポイントで書く（A4 は 595×842）。
    fn rounded(&self) -> Self {
        let round = |r: [f64; 4]| r.map(f64::round);
        Self {
            media_box_pt: round(self.media_box_pt),
            crop_box_pt: round(self.crop_box_pt),
            rotation: self.rotation,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
struct TextFingerprint {
    sha256: String,
    characters: usize,
    utf8_bytes: usize,
}
fn fingerprint(text: &str) -> TextFingerprint {
    TextFingerprint {
        sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
        characters: text.chars().count(),
        utf8_bytes: text.len(),
    }
}
#[derive(Serialize)]
struct Difference<T> {
    page: usize,
    reference: Option<T>,
    candidate: Option<T>,
}
#[derive(Serialize)]
struct Comparison<T> {
    matches: bool,
    differences: Vec<Difference<T>>,
}
fn inherited<'a>(doc: &'a Document, mut id: ObjectId, key: &[u8]) -> Result<Option<&'a Object>> {
    let mut seen = BTreeSet::new();
    for _ in 0..100 {
        if !seen.insert(id) {
            return Err("cyclic_page_tree");
        }
        let dict = doc
            .get_dictionary(id)
            .map_err(|_| "invalid_page_dictionary")?;
        if let Ok(value) = dict.get(key) {
            return Ok(Some(value));
        }
        match dict.get(b"Parent") {
            Ok(parent) => id = parent.as_reference().map_err(|_| "invalid_page_parent")?,
            Err(_) => return Ok(None),
        }
    }
    Err("page_tree_depth_limit")
}
fn numeric(doc: &Document, object: &Object) -> Result<f64> {
    let (_, object) = doc
        .dereference(object)
        .map_err(|_| "invalid_pdf_reference")?;
    let value = match object {
        Object::Integer(n) => *n as f64,
        Object::Real(n) => f64::from(*n),
        _ => return Err("invalid_pdf_number"),
    };
    if !value.is_finite() {
        return Err("invalid_pdf_number");
    }
    Ok(value)
}
fn rectangle(doc: &Document, object: &Object, unit: f64) -> Result<[f64; 4]> {
    let (_, object) = doc
        .dereference(object)
        .map_err(|_| "invalid_pdf_reference")?;
    let values = object.as_array().map_err(|_| "invalid_pdf_rectangle")?;
    if values.len() != 4 {
        return Err("invalid_pdf_rectangle");
    }
    let mut result = [0.0; 4];
    for (out, value) in result.iter_mut().zip(values) {
        *out = numeric(doc, value)? * unit;
    }
    if result.iter().any(|n| !n.is_finite()) || result[0] >= result[2] || result[1] >= result[3] {
        return Err("invalid_pdf_rectangle");
    }
    Ok(result)
}
fn geometry(doc: &Document) -> Result<Vec<Geometry>> {
    if doc.is_encrypted() {
        return Err("encrypted_pdf");
    }
    let pages = doc.get_pages();
    if pages.is_empty() || pages.len() > 10000 {
        return Err("pdf_page_limit");
    }
    pages
        .values()
        .map(|id| {
            let dict = doc
                .get_dictionary(*id)
                .map_err(|_| "invalid_page_dictionary")?;
            // UserUnit は継承属性ではない。
            let unit = dict
                .get(b"UserUnit")
                .ok()
                .map(|v| numeric(doc, v))
                .transpose()?
                .unwrap_or(1.0);
            if unit <= 0.0 || unit > 75000.0 {
                return Err("invalid_pdf_user_unit");
            }
            let media = rectangle(
                doc,
                inherited(doc, *id, b"MediaBox")?.ok_or("media_box_missing")?,
                unit,
            )?;
            let crop = inherited(doc, *id, b"CropBox")?
                .map(|v| rectangle(doc, v, unit))
                .transpose()?
                .unwrap_or(media);
            let rotate = inherited(doc, *id, b"Rotate")?
                .map(|v| numeric(doc, v))
                .transpose()?
                .unwrap_or(0.0);
            if rotate.fract() != 0.0 || rotate.abs() > 360000.0 || rotate % 90.0 != 0.0 {
                return Err("invalid_page_rotation");
            }
            Ok(Geometry {
                media_box_pt: media,
                crop_box_pt: crop,
                rotation: (rotate as i64).rem_euclid(360),
            })
        })
        .collect()
}
fn observe(path: &Path) -> Result<Observation> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "pdf_unreadable")?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "pdf_unreadable")?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err("pdf_size_limit");
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let doc = Document::load_mem(&bytes).map_err(|_| "invalid_pdf")?;
    let pages = geometry(&doc)?;
    let mut extracted_text = Vec::new();
    let mut raw_extracted_text = Vec::new();
    let mut layouts = Vec::new();
    for ((page, id), geometry) in doc.get_pages().iter().zip(&pages) {
        check_text_extraction(&doc, *id)?;
        let text = doc
            .extract_text(&[*page])
            .map_err(|_| "pdf_text_extraction_failed")?;
        raw_extracted_text.push(fingerprint(&text));
        extracted_text.push(fingerprint(&layout::normalize_radicals(&text)));
        layouts.push(layout::read(&doc, *id, geometry.media_box_pt[3])?);
    }
    Ok(Observation {
        sha256,
        pages,
        extracted_text,
        raw_extracted_text,
        layout: layouts,
    })
}

/// lopdf 0.38 の抽出器が無視する命令を、空文字として比較させない。
fn check_text_extraction(doc: &Document, page: ObjectId) -> Result<()> {
    let data = doc
        .get_page_content(page)
        .map_err(|_| "invalid_page_content")?;
    let content = lopdf::content::Content::decode(&data).map_err(|_| "invalid_page_content")?;
    let fonts = doc.get_page_fonts(page).map_err(|_| "invalid_page_fonts")?;
    let mut font_selected = false;
    for operation in content.operations {
        match operation.operator.as_str() {
            "Tf" => {
                let name = operation
                    .operands
                    .first()
                    .and_then(|value| value.as_name().ok())
                    .ok_or("invalid_text_font")?;
                if !fonts.contains_key(name) {
                    return Err("text_font_unresolved");
                }
                font_selected = true;
            }
            "Tj" | "TJ" if !font_selected => return Err("text_font_state_unobserved"),
            // 抽出器は Q による書体復元を扱わない。次の Tf まで安全に抽出できない。
            "Q" => font_selected = false,
            "'" | "\"" => return Err("text_show_operator_unobserved"),
            // ActualText / オプショナルコンテンツの置換・可視性は別の解析が必要。
            "BDC" => return Err("marked_text_unobserved"),
            "Do" => {
                let name = operation
                    .operands
                    .first()
                    .and_then(|value| value.as_name().ok())
                    .ok_or("invalid_xobject_name")?;
                let resource =
                    inherited(doc, page, b"Resources")?.ok_or("page_resources_missing")?;
                let (_, resource) = doc
                    .dereference(resource)
                    .map_err(|_| "invalid_pdf_reference")?;
                let resources = resource.as_dict().map_err(|_| "invalid_page_resources")?;
                let xobjects = doc
                    .get_dict_in_dict(resources, b"XObject")
                    .map_err(|_| "invalid_xobjects")?;
                let object = xobjects.get(name).map_err(|_| "xobject_missing")?;
                let (_, object) = doc
                    .dereference(object)
                    .map_err(|_| "invalid_pdf_reference")?;
                let stream = object.as_stream().map_err(|_| "invalid_xobject")?;
                let subtype = stream
                    .dict
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .map_err(|_| "invalid_xobject")?;
                if subtype != b"Image" {
                    return Err("form_text_unobserved");
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn rounded(pages: &[Geometry]) -> Vec<Geometry> {
    pages.iter().map(Geometry::rounded).collect()
}
fn compare<T: Clone + PartialEq>(reference: &[T], candidate: &[T]) -> Comparison<T> {
    let differences: Vec<_> = (0..reference.len().max(candidate.len()))
        .filter_map(|index| {
            let left = reference.get(index);
            let right = candidate.get(index);
            (left != right).then(|| Difference {
                page: index + 1,
                reference: left.cloned(),
                candidate: right.cloned(),
            })
        })
        .collect();
    Comparison {
        matches: differences.is_empty(),
        differences,
    }
}
fn observation_json(value: &Result<Observation>) -> serde_json::Value {
    match value {
        Ok(observation) => serde_json::json!({"status":"observed","data":observation}),
        Err(reason) => serde_json::json!({"status":"unobserved","reason":reason}),
    }
}
fn pair_json(
    reference: &Result<Observation>,
    candidate: &Result<Observation>,
) -> serde_json::Value {
    match (reference, candidate) {
        (Ok(reference), Ok(candidate)) => serde_json::json!({"status":"compared",
            "geometry":compare(&rounded(&reference.pages),&rounded(&candidate.pages)),
            "extracted_text":compare(&reference.extracted_text,&candidate.extracted_text),
            "raw_extracted_text":compare(&reference.raw_extracted_text,&candidate.raw_extracted_text),
            "positions":layout::compare(&reference.layout,&candidate.layout)}),
        _ => {
            serde_json::json!({"status":"unobserved","reference_observed":reference.is_ok(),"candidate_observed":candidate.is_ok()})
        }
    }
}
fn report(
    excel: &Result<Observation>,
    rhodopsin_xlsx: &Result<Observation>,
    libreoffice: &Result<Observation>,
) -> (serde_json::Value, u8) {
    let exit_code = match (excel, rhodopsin_xlsx) {
        // ページ全体の抽出文字は描画順と分割に依存するため参考値とし、文字列の一致は
        // positions の文字表示ごとの照合で判定する。
        (Ok(excel), Ok(ours)) => u8::from(
            !(compare(&rounded(&excel.pages), &rounded(&ours.pages)).matches
                && layout::compare(&excel.layout, &ours.layout).matches),
        ),
        _ => 2,
    };
    let report = serde_json::json!({"schema_version":4,"scope":"page_geometry_extracted_text_and_positions","overall_verdict":"not_evaluated",
        "text_extractor":"lopdf-0.38.0","text_normalization":"kangxi_radicals_to_unified_ideographs",
        "page_size_comparison":"rounded_to_whole_points","position_tolerance_pt":layout::TOLERANCE,
        "observations":{"excel":observation_json(excel),"rhodopsin_xlsx":observation_json(rhodopsin_xlsx),"libreoffice":observation_json(libreoffice)},
        "comparisons":{"rhodopsin_xlsx_vs_excel":pair_json(excel,rhodopsin_xlsx),"libreoffice_vs_excel":pair_json(excel,libreoffice),"rhodopsin_xlsx_vs_libreoffice":pair_json(libreoffice,rhodopsin_xlsx)},
        "unchecked":["cell_page_boundaries","expected_cell_text","amounts","glyph_positions_within_runs","font_embedding","raster_difference"]});
    (report, exit_code)
}
fn run() -> Result<u8> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("expected_excel_rhodopsin_xlsx_libreoffice_paths");
    }
    // 一つの観測失敗で残りの比較証拠を失わない。
    let excel = observe(Path::new(&args[0]));
    let rhodopsin_xlsx = observe(Path::new(&args[1]));
    let libreoffice = observe(Path::new(&args[2]));
    let (report, exit_code) = report(&excel, &rhodopsin_xlsx, &libreoffice);
    serde_json::to_writer(std::io::stdout().lock(), &report).map_err(|_| "report_write_failed")?;
    Ok(exit_code)
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(code) => code.into(),
        Err(reason) => {
            eprintln!("{}", serde_json::json!({"status":"failed","reason":reason}));
            2.into()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn page(width: f64) -> Geometry {
        Geometry {
            media_box_pt: [0.0, 0.0, width, 842.0],
            crop_box_pt: [0.0, 0.0, width, 842.0],
            rotation: 0,
        }
    }
    #[test]
    fn 二者が一致してもexcelとの差を残す() {
        let excel = [page(595.0)];
        let ours = [page(594.0)];
        let lo = ours.clone();
        assert!(!compare(&excel, &ours).matches);
        assert!(!compare(&excel, &lo).matches);
        assert!(compare(&lo, &ours).matches);
        let missing = compare(&excel, &[]);
        assert_eq!(missing.differences[0].page, 1);
        assert!(missing.differences[0].candidate.is_none());
    }
    #[test]
    fn 継承矩形とページ単位と回転を読む() {
        use lopdf::dictionary;
        let mut doc = Document::with_version("1.7");
        let parent = doc.new_object_id();
        let page = doc.add_object(
            dictionary! {"Type"=>"Page","Parent"=>parent,"UserUnit"=>2,"Contents"=>Object::Null},
        );
        doc.objects.insert(parent,Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),100.into(),200.into()],"Rotate"=>-90}));
        let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>parent});
        doc.trailer.set("Root", catalog);
        let result = geometry(&doc).unwrap();
        assert_eq!(result[0].media_box_pt, [0.0, 0.0, 200.0, 400.0]);
        assert_eq!(result[0].rotation, 270);
    }
}

#[cfg(test)]
#[path = "../tests/xlsx_support/mod.rs"]
mod support;
#[cfg(test)]
mod text_tests {
    use super::*;
    #[test]
    fn 同じ紙面寸法でも抽出文字の差を検出する() {
        use rhodopsin::xlsx::{
            fonts::{FontBook, FontLimits},
            pdf::{self, Item, Page, Text},
        };
        let limits = FontLimits::default();
        let mut fonts = FontBook::default();
        fonts
            .load_bytes(super::support::font("検証", false, false, 0), &limits)
            .unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let observations: Vec<_> = ["見 123", "見 124"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| {
                let bytes = pdf::render(
                    &[Page {
                        width: 100.0,
                        height: 100.0,
                        items: vec![Item::Text(Text {
                            text,
                            font: 0,
                            points: 10.0,
                            x: 10.0,
                            baseline: 20.0,
                            color: [0; 3],
                            clip: None,
                            advance_unit: None,
                        })],
                    }],
                    &fonts,
                    &Default::default(),
                    &limits,
                )
                .unwrap();
                let path = dir.path().join(format!("{index}.pdf"));
                std::fs::write(&path, bytes).unwrap();
                observe(&path).unwrap()
            })
            .collect();
        assert!(compare(&observations[0].pages, &observations[1].pages).matches);
        assert!(
            !compare(
                &observations[0].extracted_text,
                &observations[1].extracted_text
            )
            .matches
        );
        let json = serde_json::to_string(&observations[0]).unwrap();
        assert!(!json.contains("見 123"));
    }
    #[test]
    fn 文字表示の原点を許容差で比べ文字列は出力しない() {
        use rhodopsin::xlsx::{
            fonts::{FontBook, FontLimits},
            pdf::{self, Item, Page, Text},
        };
        let limits = FontLimits::default();
        let mut fonts = FontBook::default();
        fonts
            .load_bytes(super::support::font("検証", false, false, 0), &limits)
            .unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let observe_at = |name: &str, text: &str, x: f64| {
            let bytes = pdf::render(
                &[Page {
                    width: 100.0,
                    height: 100.0,
                    items: vec![Item::Text(Text {
                        text,
                        font: 0,
                        points: 10.0,
                        x,
                        baseline: 20.0,
                        color: [0; 3],
                        clip: None,
                        advance_unit: None,
                    })],
                }],
                &fonts,
                &Default::default(),
                &limits,
            )
            .unwrap();
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            observe(&path).unwrap()
        };
        let reference = observe_at("reference.pdf", "見 123", 10.0);
        let near = observe_at("near.pdf", "見 123", 11.0);
        let far = observe_at("far.pdf", "見 123", 11.5);
        let other = observe_at("other.pdf", "見 124", 10.0);
        assert!(layout::compare(&reference.layout, &near.layout).matches);
        let result = layout::compare(&reference.layout, &far.layout);
        assert!(!result.matches);
        assert_eq!(result.exceeded[0].kind, "text");
        assert_eq!(result.exceeded[0].delta_pt, 1.5);
        let result = layout::compare(&reference.layout, &other.layout);
        assert_eq!(result.unmatched_reference_runs, 1);
        assert!(!serde_json::to_string(&result).unwrap().contains("見"));
    }
    #[test]
    fn 康熙部首だけを統合漢字へ戻す() {
        assert_eq!(layout::normalize_radicals("御\u{2F92}積書"), "御見積書");
        assert_eq!(layout::normalize_radicals("\u{2F00}\u{2E9F}"), "一母");
        // 全角記号や互換文字は変えない。
        assert_eq!(layout::normalize_radicals("：（①）"), "：（①）");
    }
    #[test]
    fn 空白と改行も正規化せず比較する() {
        assert_ne!(fingerprint("A B"), fingerprint("AB"));
        assert_ne!(fingerprint("A\nB"), fingerprint("A B"));
        assert_eq!(fingerprint("見").characters, 1);
        assert_eq!(fingerprint("見").utf8_bytes, 3);
    }
}

#[cfg(test)]
mod missing_tests {
    use super::*;
    fn observed() -> Result<Observation> {
        Ok(Observation {
            sha256: "test".into(),
            pages: vec![Geometry {
                media_box_pt: [0.0, 0.0, 100.0, 100.0],
                crop_box_pt: [0.0, 0.0, 100.0, 100.0],
                rotation: 0,
            }],
            extracted_text: vec![fingerprint("test")],
            raw_extracted_text: vec![fingerprint("test")],
            layout: vec![layout::PageLayout::default()],
        })
    }
    #[test]
    fn libreoffice未観測でもexcelとの比較を残す() {
        let (result, code) = report(&observed(), &observed(), &Err("pdf_unreadable"));
        assert_eq!(code, 0);
        assert_eq!(
            result["comparisons"]["rhodopsin_xlsx_vs_excel"]["geometry"]["matches"],
            true
        );
        assert_eq!(
            result["comparisons"]["libreoffice_vs_excel"]["status"],
            "unobserved"
        );
        assert!(
            result["comparisons"]["libreoffice_vs_excel"]
                .get("matches")
                .is_none()
        );
        assert_eq!(
            result["observations"]["libreoffice"]["reason"],
            "pdf_unreadable"
        );
    }
    #[test]
    fn 正解未観測と両者未観測を一致扱いしない() {
        for (excel, ours) in [
            (Err("pdf_unreadable"), observed()),
            (Err("pdf_unreadable"), Err("invalid_pdf")),
            (observed(), Err("pdf_text_extraction_failed")),
        ] {
            let (result, code) = report(&excel, &ours, &observed());
            assert_eq!(code, 2);
            assert_eq!(
                result["comparisons"]["rhodopsin_xlsx_vs_excel"]["status"],
                "unobserved"
            );
            assert_eq!(result["overall_verdict"], "not_evaluated");
        }
    }
}

#[cfg(test)]
mod extraction_tests {
    use super::*;
    fn shapes(content: &[u8]) -> layout::PageLayout {
        use lopdf::{Stream, dictionary};
        let mut doc = Document::with_version("1.7");
        let content = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let page = doc.add_object(
            dictionary! {"Type"=>"Page","Contents"=>content,"Resources"=>dictionary! {}},
        );
        layout::read(&doc, page, 100.0).unwrap()
    }
    #[test]
    fn 塗った細い矩形と線を同じ罫線として比べる() {
        let filled = shapes(b"q 0 0 0 rg 10 89 50 1 re f 0 0.5 0.5 rg 10 10 30 30 re f Q");
        let stroked =
            shapes(b"q 0 0 0 RG 1 w 10 89.5 m 60 89.5 l S 0 0.5 0.5 rg 10 10 30 30 re f Q");
        assert_eq!(filled.rules.len(), 1);
        assert_eq!(filled.areas.len(), 1);
        assert!(layout::compare(&[filled.clone()], &[stroked]).matches);
        let thicker =
            shapes(b"q 0 0 0 RG 3 w 10 89.5 m 60 89.5 l S 0 0.5 0.5 rg 10 10 30 30 re f Q");
        assert!(!layout::compare(&[filled.clone()], &[thicker]).matches);
        let shorter =
            shapes(b"q 0 0 0 RG 1 w 10 89.5 m 50 89.5 l S 0 0.5 0.5 rg 10 10 30 30 re f Q");
        assert!(!layout::compare(&[filled], &[shorter]).matches);
    }
    fn document(content: &[u8], subtype: &str) -> (Document, ObjectId) {
        use lopdf::{Stream, dictionary};
        let mut doc = Document::with_version("1.7");
        let object = doc.add_object(Stream::new(
            dictionary! {"Type"=>"XObject","Subtype"=>subtype},
            vec![],
        ));
        let content = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let page = doc.add_object(dictionary! {"Type"=>"Page","Contents"=>content,"Resources"=>dictionary! {"XObject"=>dictionary! {"X1"=>object}}});
        (doc, page)
    }
    #[test]
    fn 抽出器が無視する文字命令を未観測にする() {
        for (content, reason) in [
            (b"/X1 Do".as_slice(), "form_text_unobserved"),
            (b"(secret) Tj".as_slice(), "text_font_state_unobserved"),
            (b"(secret) '".as_slice(), "text_show_operator_unobserved"),
            (
                b"1 2 (secret) \"".as_slice(),
                "text_show_operator_unobserved",
            ),
            (
                b"/Span << /ActualText (secret) >> BDC".as_slice(),
                "marked_text_unobserved",
            ),
            (b"/Missing 12 Tf".as_slice(), "text_font_unresolved"),
        ] {
            let (doc, page) = document(content, "Form");
            assert_eq!(check_text_extraction(&doc, page).unwrap_err(), reason);
        }
    }
    #[test]
    fn 通常画像は文字の欠落として扱わない() {
        let (doc, page) = document(b"/X1 Do", "Image");
        assert!(check_text_extraction(&doc, page).is_ok());
    }
}
