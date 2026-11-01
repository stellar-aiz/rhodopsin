//! 開発専用。一ページ分の不透明 PNG 三者を、補正・しきい値なしで比較する。
use image::ImageEncoder;
use rhodopsin::xlsx::drawing::{Raster, decode};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};
type Result<T> = std::result::Result<T, &'static str>;
const FILE_LIMIT: u64 = 32 * 1024 * 1024;
const PIXEL_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Serialize)]
struct Observation {
    sha256: String,
    width: u32,
    height: u32,
    #[serde(skip)]
    raster: Raster,
}
fn observe(path: &Path) -> Result<Observation> {
    let file = File::open(path).map_err(|_| "image_unreadable")?;
    let metadata = file.metadata().map_err(|_| "image_unreadable")?;
    if !metadata.is_file() {
        return Err("image_not_regular_file");
    }
    if metadata.len() > FILE_LIMIT {
        return Err("image_file_limit");
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "image_unreadable")?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err("image_file_limit");
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("expected_png");
    }
    let raster = decode(&bytes, 16384, PIXEL_LIMIT).map_err(|failure| failure.reason)?;
    if raster.rgba.chunks_exact(4).any(|pixel| pixel[3] != 255) {
        return Err("transparent_raster");
    }
    Ok(Observation {
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        width: raster.width,
        height: raster.height,
        raster,
    })
}
#[derive(Debug, Serialize)]
struct Difference {
    matches: bool,
    dimensions_match: bool,
    /// 寸法が違えば画素の切り落としや補間を行わず、以下は未評価とする。
    differing_pixels: Option<u64>,
    total_pixels: Option<u64>,
    maximum_channel_difference: Option<u8>,
    absolute_channel_difference_sum: Option<u64>,
    mean_absolute_channel_difference: Option<f64>,
    /// 左上原点、右端・下端を含まない [left, top, right, bottom]。
    difference_bounds: Option<[u32; 4]>,
}
fn compare(reference: &Observation, candidate: &Observation) -> Difference {
    let dimensions_match =
        (reference.width, reference.height) == (candidate.width, candidate.height);
    let mut result = Difference {
        matches: false,
        dimensions_match,
        differing_pixels: None,
        total_pixels: None,
        maximum_channel_difference: None,
        absolute_channel_difference_sum: None,
        mean_absolute_channel_difference: None,
        difference_bounds: None,
    };
    if !dimensions_match {
        return result;
    }
    let mut count = 0u64;
    let mut maximum = 0;
    let mut sum = 0u64;
    let mut bounds = [reference.width, reference.height, 0, 0];
    for (index, (a, b)) in reference
        .raster
        .rgba
        .chunks_exact(4)
        .zip(candidate.raster.rgba.chunks_exact(4))
        .enumerate()
    {
        let mut different = false;
        for channel in 0..3 {
            let delta = a[channel].abs_diff(b[channel]);
            maximum = maximum.max(delta);
            sum += u64::from(delta);
            different |= delta != 0;
        }
        if different {
            count += 1;
            let x = (index as u64 % u64::from(reference.width)) as u32;
            let y = (index as u64 / u64::from(reference.width)) as u32;
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x + 1);
            bounds[3] = bounds[3].max(y + 1);
        }
    }
    let total = u64::from(reference.width) * u64::from(reference.height);
    result.matches = count == 0;
    result.differing_pixels = Some(count);
    result.total_pixels = Some(total);
    result.maximum_channel_difference = Some(maximum);
    result.absolute_channel_difference_sum = Some(sum);
    result.mean_absolute_channel_difference = Some(sum as f64 / (total * 3) as f64);
    result.difference_bounds = (count != 0).then_some(bounds);
    result
}
fn observation_json(value: &Result<Observation>) -> serde_json::Value {
    match value {
        Ok(value) => serde_json::json!({"status":"observed","data":value}),
        Err(reason) => serde_json::json!({"status":"unobserved","reason":reason}),
    }
}
fn pair_json(a: &Result<Observation>, b: &Result<Observation>) -> serde_json::Value {
    match (a, b) {
        (Ok(a), Ok(b)) => serde_json::json!({"status":"compared","pixels":compare(a,b)}),
        _ => {
            serde_json::json!({"status":"unobserved","reference_observed":a.is_ok(),"candidate_observed":b.is_ok()})
        }
    }
}
fn report(
    excel: &Result<Observation>,
    ours: &Result<Observation>,
    lo: &Result<Observation>,
) -> (serde_json::Value, u8) {
    let code = match (excel, ours) {
        (Ok(a), Ok(b)) => u8::from(!compare(a, b).matches),
        _ => 2,
    };
    (
        serde_json::json!({"schema_version":1,"scope":"single_page_rgb8_raster","overall_verdict":"not_evaluated","alignment":"none","threshold":0,
        "observations":{"excel":observation_json(excel),"rhodopsin_xlsx":observation_json(ours),"libreoffice":observation_json(lo)},
        "comparisons":{"rhodopsin_xlsx_vs_excel":pair_json(excel,ours),"libreoffice_vs_excel":pair_json(excel,lo),"rhodopsin_xlsx_vs_libreoffice":pair_json(lo,ours)},
        "unchecked":["rasterization_conditions","page_count","physical_page_dimensions","text_positions","expected_cell_text","font_embedding"]}),
        code,
    )
}

fn save_difference(
    reference: &Result<Observation>,
    candidate: &Result<Observation>,
    path: &Path,
) -> Result<serde_json::Value> {
    let (Ok(a), Ok(b)) = (reference, candidate) else {
        return Ok(serde_json::json!({"status":"not_written","reason":"unobserved_input"}));
    };
    if (a.width, a.height) != (b.width, b.height) {
        return Ok(serde_json::json!({"status":"not_written","reason":"dimensions_differ"}));
    }
    rhodopsin::xlsx::output::check_available(path).map_err(|failure| failure.reason)?;
    let mut pixels = Vec::with_capacity(a.raster.rgba.len() / 4 * 3);
    for (left, right) in a
        .raster
        .rgba
        .chunks_exact(4)
        .zip(b.raster.rgba.chunks_exact(4))
    {
        for channel in 0..3 {
            pixels.push(left[channel].abs_diff(right[channel]));
        }
    }
    let mut encoded = Vec::new();
    image::codecs::png::PngEncoder::new(&mut encoded)
        .write_image(&pixels, a.width, a.height, image::ExtendedColorType::Rgb8)
        .map_err(|_| "difference_encode_failed")?;
    rhodopsin::xlsx::output::publish(path, &encoded).map_err(|failure| failure.reason)?;
    Ok(
        serde_json::json!({"status":"written","sha256":format!("{:x}",Sha256::digest(&encoded)),
        "width":a.width,"height":a.height,"encoding":"rgb8_absolute_difference","gain":1}),
    )
}

fn save_differences(
    output: &mut serde_json::Value,
    excel: &Result<Observation>,
    ours: &Result<Observation>,
    lo: &Result<Observation>,
    directory: &Path,
) -> bool {
    let mut failed = false;
    let mut artifacts = serde_json::Map::new();
    for (key, a, b) in [
        ("rhodopsin_xlsx_vs_excel", excel, ours),
        ("libreoffice_vs_excel", excel, lo),
        ("rhodopsin_xlsx_vs_libreoffice", lo, ours),
    ] {
        let file = format!("{key}.png");
        let mut value = match save_difference(a, b, &directory.join(&file)) {
            Ok(value) => value,
            Err(reason) => {
                failed = true;
                serde_json::json!({"status":"failed","reason":reason})
            }
        };
        // 固定の相対名だけを報告し、指定ディレクトリはログへ出さない。
        value["file"] = file.into();
        artifacts.insert(key.into(), value);
    }
    output["difference_images"] = artifacts.into();
    failed
}

fn run() -> Result<u8> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !(args.len() == 3 || args.len() == 5 && args[3] == "--diff-dir") {
        return Err("expected_excel_rhodopsin_xlsx_libreoffice_png_paths");
    }
    let excel = observe(Path::new(&args[0]));
    let ours = observe(Path::new(&args[1]));
    let lo = observe(Path::new(&args[2]));
    let (mut output, mut code) = report(&excel, &ours, &lo);
    if args.len() == 5 && save_differences(&mut output, &excel, &ours, &lo, Path::new(&args[4])) {
        code = 2;
    }
    serde_json::to_writer(std::io::stdout().lock(), &output).map_err(|_| "report_write_failed")?;
    Ok(code)
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
    fn observation(width: u32, height: u32, rgba: Vec<u8>) -> Observation {
        Observation {
            sha256: "試験用".into(),
            width,
            height,
            raster: Raster {
                width,
                height,
                rgba,
            },
        }
    }
    #[test]
    fn 一画素内の複数チャンネル差を一画素と数え境界を記録する() {
        let a = observation(3, 2, vec![255; 24]);
        let mut bytes = vec![255; 24];
        bytes[4] = 245;
        bytes[5] = 235;
        bytes[20] = 0;
        let b = observation(3, 2, bytes);
        let result = compare(&a, &b);
        assert!(!result.matches);
        assert_eq!(result.differing_pixels, Some(2));
        assert_eq!(result.total_pixels, Some(6));
        assert_eq!(result.maximum_channel_difference, Some(255));
        assert_eq!(result.absolute_channel_difference_sum, Some(285));
        assert_eq!(result.mean_absolute_channel_difference, Some(285.0 / 18.0));
        assert_eq!(result.difference_bounds, Some([1, 0, 3, 2]));
        let same = compare(&a, &a);
        assert!(same.matches);
        assert_eq!(same.differing_pixels, Some(0));
        assert_eq!(same.difference_bounds, None);
    }
    #[test]
    fn 寸法不一致で重なる部分だけを比較しない() {
        let a = observation(2, 1, vec![255; 8]);
        let b = observation(1, 2, vec![255; 8]);
        let result = compare(&a, &b);
        assert!(!result.matches);
        assert!(!result.dimensions_match);
        assert_eq!(result.differing_pixels, None);
        assert_eq!(result.mean_absolute_channel_difference, None);
    }
    #[test]
    fn 三者比較を多数決にせず欠落を一致と扱わない() {
        let excel = Ok(observation(1, 1, vec![255; 4]));
        let ours = Ok(observation(1, 1, vec![0, 0, 0, 255]));
        let lo = Ok(observation(1, 1, vec![0, 0, 0, 255]));
        let (value, code) = report(&excel, &ours, &lo);
        assert_eq!(code, 1);
        assert_eq!(
            value["comparisons"]["rhodopsin_xlsx_vs_libreoffice"]["pixels"]["matches"],
            true
        );
        assert_eq!(
            value["comparisons"]["rhodopsin_xlsx_vs_excel"]["pixels"]["matches"],
            false
        );
        assert_eq!(value["overall_verdict"], "not_evaluated");
        assert_eq!(report(&excel, &excel, &Err("image_unreadable")).1, 0);
        let (value, code) = report(&Err("image_unreadable"), &ours, &lo);
        assert_eq!(code, 2);
        assert_eq!(
            value["comparisons"]["rhodopsin_xlsx_vs_excel"]["status"],
            "unobserved"
        );
        assert_eq!(
            value["comparisons"]["rhodopsin_xlsx_vs_libreoffice"]["pixels"]["matches"],
            true
        );
    }
    #[test]
    fn pngの観測と入力制限の診断にパスを含めない() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("機密の名前.png");
        image::RgbaImage::from_raw(1, 1, vec![12, 34, 56, 255])
            .unwrap()
            .save(&path)
            .unwrap();
        let result = observe(&path).unwrap();
        assert_eq!((result.width, result.height), (1, 1));
        assert_eq!(result.raster.rgba, vec![12, 34, 56, 255]);
        assert_eq!(result.sha256.len(), 64);
        assert!(!observation_json(&Ok(result)).to_string().contains("機密"));
        image::RgbaImage::from_raw(1, 1, vec![12, 34, 56, 254])
            .unwrap()
            .save(&path)
            .unwrap();
        assert!(matches!(observe(&path), Err("transparent_raster")));
        std::fs::write(&path, b"invalid").unwrap();
        assert!(matches!(observe(&path), Err("expected_png")));
        File::create(&path)
            .unwrap()
            .set_len(FILE_LIMIT + 1)
            .unwrap();
        assert!(matches!(observe(&path), Err("image_file_limit")));
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(observe(&path), Err("image_unreadable")));
    }
}

#[cfg(test)]
mod artifact_tests {
    use super::*;
    fn input(pixels: Vec<u8>) -> Result<Observation> {
        Ok(Observation {
            sha256: "試験用".into(),
            width: 2,
            height: 1,
            raster: Raster {
                width: 2,
                height: 1,
                rgba: pixels,
            },
        })
    }
    #[test]
    fn 差分pngは増幅なしの絶対差を同じ座標に保存する() {
        let a = input(vec![255, 100, 30, 255, 10, 20, 30, 255]);
        let b = input(vec![254, 80, 50, 255, 10, 20, 30, 255]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.png");
        let result = save_difference(&a, &b, &path).unwrap();
        assert_eq!(result["status"], "written");
        assert_eq!(result["gain"], 1);
        let pixels = image::open(&path).unwrap().to_rgb8();
        assert_eq!(pixels.dimensions(), (2, 1));
        assert_eq!(pixels.as_raw(), &vec![1, 20, 20, 0, 0, 0]);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(result["sha256"], format!("{:x}", Sha256::digest(&bytes)));
        let again = dir.path().join("again.png");
        save_difference(&a, &b, &again).unwrap();
        assert_eq!(bytes, std::fs::read(&again).unwrap());
        assert_eq!(
            save_difference(&a, &b, &path).unwrap_err(),
            "output_already_exists"
        );
        assert_eq!(bytes, std::fs::read(&path).unwrap());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
    #[test]
    fn 未観測と寸法不一致には差分画像を作らない() {
        let a = input(vec![255; 8]);
        let b = Ok(Observation {
            sha256: "試験用".into(),
            width: 1,
            height: 1,
            raster: Raster {
                width: 1,
                height: 1,
                rgba: vec![255; 4],
            },
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff.png");
        assert_eq!(
            save_difference(&a, &b, &path).unwrap()["reason"],
            "dimensions_differ"
        );
        assert_eq!(
            save_difference(&a, &Err("image_unreadable"), &path).unwrap()["reason"],
            "unobserved_input"
        );
        assert!(!path.exists());
    }
    #[test]
    fn 保存失敗でも比較結果と完成済みの差分画像を記録する() {
        let a = input(vec![255; 8]);
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("rhodopsin_xlsx_vs_excel.png");
        std::fs::write(&existing, "既存ファイルは保持").unwrap();
        let (mut output, code) = report(&a, &a, &a);
        assert_eq!(code, 0);
        assert!(save_differences(&mut output, &a, &a, &a, dir.path()));
        assert_eq!(
            output["difference_images"]["rhodopsin_xlsx_vs_excel"]["reason"],
            "output_already_exists"
        );
        assert_eq!(
            output["difference_images"]["libreoffice_vs_excel"]["status"],
            "written"
        );
        assert_eq!(
            output["comparisons"]["rhodopsin_xlsx_vs_excel"]["pixels"]["matches"],
            true
        );
        assert_eq!(
            std::fs::read(existing).unwrap(),
            "既存ファイルは保持".as_bytes()
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }
}
