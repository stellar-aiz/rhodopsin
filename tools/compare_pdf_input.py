#!/usr/bin/env python3
"""Compare the PDF-input round-trip fixture against its PDF and PPTX sources.

Generate the fixture with RHODOPSIN_PDF_INPUT_EVIDENCE=<new-directory>
cargo test --test pdf self_produced. Requires development-only pypdfium2/Pillow.
"""
import argparse
import importlib.metadata
import json
from pathlib import Path
import sys

from PIL import Image, ImageChops
import pypdfium2 as pdfium

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests" / "visual"))
from compare import compare_images, passes  # noqa: E402

# Fixed before measuring. The synthetic page is 200 by 100 scene units.
REGIONS = {"text": (0.04, 0.08, 0.35, 0.5), "path": (0.04, 0.08, 0.96, 0.92),
           "image": (0.725, 0.6, 0.875, 0.9)}


def white(image):
    rgba = image.convert("RGBA")
    result = Image.new("RGBA", rgba.size, "white")
    return Image.alpha_composite(result, rgba).convert("RGB")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    directory = args.directory
    results = []
    with pdfium.PdfDocument(directory / "input.pdf") as document:
        for index in range(len(document)):
            page = document[index]
            reference = white(page.render(scale=1600 / page.get_width()).to_pil())
            reference.save(directory / f"reference-{index+1:03}.png")
            actual = white(Image.open(directory / "raster" / f"page-{index+1:03}.png"))
            direct = white(Image.open(directory / "direct" / f"slide-{index+1:03}.png"))
            for name, expected in (("pdfium", reference), ("pptx", direct)):
                if actual.size != expected.size:
                    raise ValueError("Page dimensions differ")
                ImageChops.difference(actual, expected).save(directory / f"diff-{name}-{index+1:03}.png")
                regions = {"page": (0, 0, 1, 1), **REGIONS}
                for region, box in regions.items():
                    rect = tuple(round(v * (actual.width if i % 2 == 0 else actual.height)) for i, v in enumerate(box))
                    metrics, _ = compare_images(actual.crop(rect), expected.crop(rect), 16)
                    results.append({"page": index+1, "reference": name, "region": region,
                                    **metrics, "passed": passes(metrics, 0.02, 0.05)})
    report = {"pypdfium2": importlib.metadata.version("pypdfium2"),
              "pdfium": str(pdfium.version.PDFIUM_INFO),
              "thresholds": {"mae": 0.02, "changed_ratio": 0.05, "delta": 16},
              "regions": REGIONS, "results": results,
              "passed": all(row["passed"] for row in results if row["reference"] == "pdfium"),
              "pptx_comparison": "diagnostic_only"}
    (directory / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
