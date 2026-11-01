#!/usr/bin/env python3
"""PDF 三者の幾何・抽出文字と、全ページ PNG の比較を集約する開発ツール。"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

SOURCES = ("excel", "rhodopsin_xlsx", "libreoffice")


def invoke(binary, arguments):
    try:
        result = subprocess.run(
            [str(binary), *map(str, arguments)], capture_output=True, timeout=120,
            check=False,
        )
        if result.returncode not in (0, 1, 2) or len(result.stdout) > 16 * 1024 * 1024:
            raise ValueError()
        report = json.loads(result.stdout)
        if not isinstance(report, dict) or "observations" not in report:
            raise ValueError()
        return report, result.returncode
    except (OSError, subprocess.TimeoutExpired, ValueError):
        return {"status": "unobserved", "reason": "comparison_failed"}, 2


def compare(manifest, root, binaries, output):
    if manifest.get("schema_version") != 1:
        raise ValueError("invalid_manifest")
    inputs = manifest.get("inputs", {})
    if not isinstance(inputs, dict):
        raise ValueError("invalid_manifest")
    for source in SOURCES:
        entry = inputs.get(source)
        if not isinstance(entry, dict) or not isinstance(entry.get("pdf"), str):
            raise ValueError("invalid_manifest")
        rasters = entry.get("pages")
        if not isinstance(rasters, list) or len(rasters) > 10000:
            raise ValueError("invalid_manifest")
        if any(not isinstance(p, str) or not p for p in rasters):
            raise ValueError("invalid_manifest")
    output.mkdir()  # 既存の証跡ディレクトリを上書きしない。
    pdf, code = invoke(binaries[0], [root / inputs[s]["pdf"] for s in SOURCES])
    coverage = {}
    counts = []
    for source in SOURCES:
        observation = pdf.get("observations", {}).get(source, {})
        pages = observation.get("data", {}).get("pages")
        count = len(pages) if observation.get("status") == "observed" and isinstance(pages, list) else None
        listed = len(inputs[source]["pages"])
        complete = count is not None and count > 0 and count == listed
        coverage[source] = {"pdf_pages": count, "listed_rasters": listed, "complete": complete}
        counts.append(max(count or 0, listed))
        if source != "libreoffice" and not complete:
            code = 2
    pages = []
    # 一度に保持する画像は三枚。欠落ページは存在しない固定パスで未観測にする。
    with tempfile.TemporaryDirectory() as temporary:
        missing = Path(temporary) / "missing.png"
        for index in range(max(counts, default=0)):
            directory = output / f"page-{index + 1:05d}"
            directory.mkdir()
            paths = [root / inputs[s]["pages"][index] if index < len(inputs[s]["pages"]) else missing for s in SOURCES]
            report, page_code = invoke(binaries[1], [*paths, "--diff-dir", directory])
            if index < max(counts[:2]):
                code = max(code, page_code)
            pages.append({"page_index": index + 1, "report": report})
    result = {
        "schema_version": 1, "scope": "all_page_geometry_text_and_rasters",
        "overall_verdict": "not_evaluated", "comparison_exit_code": code,
        "pdf": pdf, "coverage": coverage, "pages": pages,
        "unchecked": ["rasterization_conditions", "raster_page_correspondence",
                      "cell_page_boundaries", "expected_cell_text", "amounts",
                      "text_positions", "font_embedding", "reference_provenance"],
    }
    with (output / "comparison.json").open("x") as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    return result, code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--pdf-comparator", type=Path, required=True)
    parser.add_argument("--raster-comparator", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        with args.manifest.open("rb") as stream:
            data = stream.read(1024 * 1024 + 1)
        if len(data) > 1024 * 1024:
            raise ValueError()
        manifest = json.loads(data)
        if not isinstance(manifest, dict):
            raise ValueError()
        result, code = compare(manifest, args.manifest.resolve().parent,
                               (args.pdf_comparator.resolve(), args.raster_comparator.resolve()),
                               args.out_dir)
        print(json.dumps(result, ensure_ascii=False))
        return code
    except (OSError, ValueError, TypeError):
        print(json.dumps({"status": "failed", "reason": "comparison_unavailable"}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
