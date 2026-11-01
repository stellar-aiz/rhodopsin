//! Hidden-slide policy: skipped by default, rendered with --include-hidden, recorded in the manifest.
use std::{fs, io::Write, path::Path, process::Command};
use zip::{ZipWriter, write::SimpleFileOptions};

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PR: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// One slide per `show` attribute value; every slide is a distinct solid color.
fn deck(path: &Path, shows: &[&str]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    let mut add = |name: &str, value: String| {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    };
    add(
        "_rels/.rels",
        format!(
            r#"<Relationships xmlns="{PR}"><Relationship Id="r1" Type="{R}/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#
        ),
    );
    let ids: String = (1..=shows.len())
        .map(|i| format!(r#"<p:sldId id="{}" r:id="s{i}"/>"#, 255 + i))
        .collect();
    add(
        "ppt/presentation.xml",
        format!(
            r#"<p:presentation xmlns:p="{P}" xmlns:r="{R}"><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="952500" cy="952500"/></p:presentation>"#
        ),
    );
    let rels: String = (1..=shows.len())
        .map(|i| {
            format!(r#"<Relationship Id="s{i}" Type="{R}/slide" Target="slides/slide{i}.xml"/>"#)
        })
        .collect();
    add(
        "ppt/_rels/presentation.xml.rels",
        format!(r#"<Relationships xmlns="{PR}">{rels}</Relationships>"#),
    );
    for (index, show) in shows.iter().enumerate() {
        let attribute = if show.is_empty() {
            String::new()
        } else {
            format!(r#" show="{show}""#)
        };
        add(
            &format!("ppt/slides/slide{}.xml", index + 1),
            format!(
                r#"<p:sld xmlns:p="{P}" xmlns:a="{A}"{attribute}><p:cSld><p:spTree><p:sp><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="952500" cy="952500"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="{:02X}0000"/></a:solidFill></p:spPr></p:sp></p:spTree></p:cSld></p:sld>"#,
                (index + 1) * 40
            ),
        );
    }
    zip.finish().unwrap();
}

fn convert(
    shows: &[&str],
    extra: &[&str],
) -> (std::process::Output, serde_json::Value, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("deck.pptx");
    let out = dir.path().join("out");
    deck(&input, shows);
    let output = Command::new(env!("CARGO_BIN_EXE_rhodopsin"))
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--width", "64"])
        .args(extra)
        .output()
        .unwrap();
    let manifest = fs::read(out.join("manifest.json"))
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
        .unwrap_or(serde_json::Value::Null);
    (output, manifest, dir)
}

fn numbers(manifest: &serde_json::Value, key: &str) -> Vec<u64> {
    manifest[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect()
}

#[test]
fn hidden_slides_are_skipped_by_default_and_recorded() {
    let (output, manifest, dir) = convert(&["", "0", "false", "1", "true"], &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(manifest["schema_version"], 4);
    assert_eq!(manifest["include_hidden"], false);
    assert_eq!(numbers(&manifest, "skipped_hidden_slides"), [2, 3]);
    let slides = manifest["slides"].as_array().unwrap();
    let listed: Vec<_> = slides
        .iter()
        .map(|s| s["slide"].as_u64().unwrap())
        .collect();
    assert_eq!(listed, [1, 4, 5]);
    assert!(slides.iter().all(|s| s["hidden"] == false));
    // Images keep the deck position so they can be traced back to the slide.
    let images: Vec<_> = slides
        .iter()
        .map(|s| s["image"].as_str().unwrap())
        .collect();
    assert_eq!(images, ["slide-001.png", "slide-004.png", "slide-005.png"]);
    assert!(!dir.path().join("out/slide-002.png").exists());
}

#[test]
fn include_hidden_renders_and_marks_hidden_slides() {
    let (output, manifest, _dir) = convert(&["", "0", "1"], &["--include-hidden"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(manifest["include_hidden"], true);
    assert!(numbers(&manifest, "skipped_hidden_slides").is_empty());
    let hidden: Vec<_> = manifest["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["slide"].as_u64().unwrap(), s["hidden"].as_bool().unwrap()))
        .collect();
    assert_eq!(hidden, [(1, false), (2, true), (3, false)]);
}

#[test]
fn deck_without_hidden_slides_is_unchanged() {
    let (output, manifest, _dir) = convert(&["", "1"], &[]);
    assert!(output.status.success());
    assert_eq!(manifest["slides"].as_array().unwrap().len(), 2);
    assert!(numbers(&manifest, "skipped_hidden_slides").is_empty());
}

#[test]
fn all_hidden_deck_fails_with_a_pointer_to_the_flag() {
    let (output, _, _dir) = convert(&["0", "0"], &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--include-hidden"));
    let (output, manifest, _dir) = convert(&["0", "0"], &["--include-hidden"]);
    assert!(output.status.success());
    assert_eq!(manifest["slides"].as_array().unwrap().len(), 2);
}

#[test]
fn skipped_hidden_slides_count_toward_the_slide_limit() {
    let mut shows = vec!["0"; 201];
    shows[0] = "";
    for extra in [&[][..], &["--include-hidden"][..]] {
        let (output, _, _dir) = convert(&shows, extra);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("200 slides"));
    }
    // Exactly 200 entries stay within the limit.
    let (output, manifest, _dir) = convert(&shows[..200], &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(numbers(&manifest, "skipped_hidden_slides").len(), 199);
}
