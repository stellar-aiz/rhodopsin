---
kind: guide
---

# Visual comparison against LibreOffice

`tests/visual/compare.py` renders a deck with rhodopsin and with LibreOffice (PDF, then PDFium), and compares the pages. LibreOffice is a development-time comparison reference, never a runtime dependency and not PowerPoint ground truth; its rendering can itself contain overlaps or differences, so passing thresholds does not prove compatibility. Inspect both images when investigating a difference. Environment setup and the whole-page metrics are in [development](development.md#visual-comparisons). Publish only independently created fixtures, never private source documents or images.

## Regions

`--regions regions.json` adds per-page local comparisons. `slide` is one-based, `name` is unique within a page, and `rect` is normalized `[x, y, width, height]` in 0-1. Out-of-bounds rectangles, zero area, nonexistent pages, and duplicate names are execution errors.

```json
{
  "regions": [
    {"slide": 1, "name": "small-shape", "rect": [0.1, 0.1, 0.02, 0.02]}
  ]
}
```

The pixel region floors the top-left and ceils the bottom-right; `pixel_rect` records left/top/right/bottom with right/bottom exclusive. Local regions use the same thresholds as whole pages (MAE 0.02, changed pixels 0.05, channel tolerance 16); no per-region threshold hides a difference.

| Report field | Meaning |
|---|---|
| `slides[].passed` / `global_passed` | Whole-page pixel verdict |
| `slides[].regions` | Metrics and verdict per requested region |
| `regions_passed` | Combined region verdict; `null` if none requested |
| `diagnostic_summary` | Counts by diagnostic category and `has_omissions` |
| `slides[].renderer_diagnostics` | Original diagnostics with part/object/location |
| `hidden_slides` | Manifest entries dropped before pairing (LibreOffice skips hidden slides too) |
| `status` | `difference_exceeded` if global or local thresholds are exceeded; `unpinned_families` under `--strict-font-pinning` |

Exit codes: 0 when global and local pass, 1 when either exceeds, 2 for execution errors. Diagnostics are independent of pixel verdicts: `global_passed: true` can coexist with `has_omissions: true`, and an absence of omissions does not prove compatibility. Old manifests without diagnostic fields report `available: false`. In the report, `slides[].slide` is the PDF page number and `slides[].deck_slide` the position in the deck.

## Deterministic reference fonts

LibreOffice resolves families missing from the machine (Meiryo UI, Yu Gothic, Calibri, ...) through CoreText or fontconfig, and the result was observed to differ between runs, so reference images fluctuated independently of rhodopsin. The comparator therefore writes a `Font/Substitution` replacement table into the generated LibreOffice profile. The default table `tests/visual/font-substitution.json` maps those families to Hiragino Sans, the face rhodopsin resolves to on macOS, so the comparison measures layout rather than font choice. Override it with `--font-substitution-map map.json` or `RHODOPSIN_FONT_SUBSTITUTION_MAP` (`{"families": {"Family": "Substitute"}}`).

| Report field | Meaning |
|---|---|
| `environment.libreoffice_font_substitution` | The table pinned in the profile |
| `environment.font_substitution_sha256` | Hash of the copied table |
| `reference_pdf_fonts` | Fonts used by text in the reference PDF (`name`, `embedded`, `pages`) |
| `requested_families` | `a:latin` / `a:ea` typefaces declared by the deck, including theme-declared faces that no text uses |
| `unpinned_families` | Requested families the table does not list; their reference font is left to LibreOffice's fallback and can drift |

Unpinned families are printed as a warning and recorded; `--strict-font-pinning` turns a non-empty list into exit 1. The tables also pin families that fell back differently between runs in a local survey (Japanese gothic variants, Microsoft YaHei, Aptos, Century, Century Gothic). The macOS table maps shipped faces (Arial, Georgia, Times New Roman, ...) to themselves so they count as pinned; the Linux and Windows tables leave those out, so Arial is reported as unpinned there. Add a row to all three tables when a new family shows up in `unpinned_families`. The substitution replaces font choice, not metrics: pinned references still differ from rhodopsin where layout differs, and rhodopsin resolves families through its own system lookup on each OS.

| OS | Table | Substitute | Source |
|---|---|---|---|
| macOS | `font-substitution.json` | Hiragino Sans | Part of macOS; `check_environment.py` pins the SHA-256 of the two Hiragino Sans files of the runner image and reports every differing file with its found hash |
| Linux | `font-substitution.linux.json` | Noto Sans CJK JP | `NotoSansCJKjp-Regular.otf` and `-Bold.otf` from the `notofonts/noto-cjk` release, installed by the job with pinned SHA-256 values |
| Windows | `font-substitution.windows.json` | Noto Sans CJK JP | The same two files in the per-user font directory |

The scheduled/manual workflow `.github/workflows/visual.yml` runs these three legs with the pinned LibreOffice and sets `RHODOPSIN_FONT_SUBSTITUTION_MAP` to the OS table. `check_determinism.py` compares twice, requires byte-identical reference PNGs, and appends the pinned table, the PDF fonts used, and the reference hashes to the job summary. The comparison step gates on macOS only; on Linux and Windows it runs with `continue-on-error` because rhodopsin resolves system fonts per OS while the reference pins Noto, so line widths differ. The determinism step gates on every OS, with `--strict-font-pinning` on macOS. A comparison that gets worse than the [recorded baseline](history/20261020_visual-ci-baseline.md) is the signal to investigate; the Linux and Windows numbers are not a statement that those OSes render acceptably. When the macOS image changes, update the Hiragino hashes in `check_environment.py` from the failure message.

## Reproducibility

Output retains the input PPTX, commands, stdout/stderr, reference PDF, both PNG sets, global differences, regions JSON, and a copy of the comparator. Reports record hashes for input, renderer, LibreOffice executable, comparator, and regions; width, thresholds, and compression settings; Python, image-library, and OS versions; font paths and hashes; and renderer-resolved fonts. Reproduce with the same binaries, fonts, OS, dependencies, and settings, using a new output directory. The comparator passes `--font-fallback` so that approximate rendering is measured; the standalone CLI rejects family substitution by default.

```sh
.venv-visual/bin/python -m unittest discover -s tests/visual -p 'test_*.py'
```

## Fixture generators

Every `make_*.py` under `tests/visual/` writes an independently generated deck. Only generation needs python-pptx or Pillow. Unless noted, compare a generated deck with:

```sh
cargo build --release --locked
.venv-visual/bin/python tests/visual/compare.py --binary target/release/rhodopsin \
  --fixture <deck.pptx> [--regions <regions.json>] --width 1600 --out target/visual-tests/<name>
```

| Generator | Deck and what it checks | Documented in |
|---|---|---|
| `make_local_omission.py OUT` | A small chart in whitespace; also writes `OUT`'s sibling `.json` regions. Global passes, the region fails with omission diagnostics (exit 1); chart omission is a known limitation | here |
| `make_unsupported_images.py` | A minimal EMF picture on the slide and on the shared layout; checks that a layout picture renders on every slide | [rendering](rendering.md#emf-and-wmf-pictures) |
| `make_oversized_images.py` | A 4400x4400 PNG and a 5800x3456 JPEG plus a picture-less baseline deck for peak-memory comparison (`/usr/bin/time -l`); both pictures downscale with `approximation` | [rendering](rendering.md#resource-limits) |
| `make_hidden_slides.py` | Hidden slides; pages pair one to one with the LibreOffice PDF | [rendering](rendering.md#hidden-slides) |
| `make_font_substitution.py`, `make_unpinned_font.py` | Families the table pins, and one it does not (`unpinned_families`) | above |
| `make_table_row_growth.py`, `make_table_zero_row_height.py` | Rows shorter than their text, a frame taller than its rows, an `h="0"` row | [rendering](rendering.md#table-row-heights) |
| `make_flipped_text.py` (+ `examples/flipped-text.json`) | Flipped shapes, groups, and tables with labels; group-flipped tables differ from LibreOffice by design | [rendering](rendering.md#flipped-shapes-and-groups) |
| `make_custom_geometry.py` (+ `.json`) | Six `a:custGeom` freeforms | [rendering](rendering.md#freeform-shapes) |
| `make_image_effects.py` (+ `.json`), `make_shape_tile.py` | Thirteen `a:blip` effect, tile, and `fillRect` pages; a tiled right triangle without `pHYs` | [rendering](rendering.md#picture-effects-and-fills) |
| `make_image_references.py` | A control, an `oleObject`-typed picture, a missing relationship, and an empty crop; the last two are omitted, not fatal | [rendering](rendering.md#known-limitations) |
| `make_svg_pictures.py` | SVG only, SVG with a PNG fallback (the fallback is drawn), SVG stretched | [rendering](rendering.md#known-limitations) |
| `make_hidden.py` | Hidden shapes, groups, children, and a layout shape; no diagnostic | [rendering](rendering.md#hidden-elements) |
| `make_cjk_metrics.py` | Japanese families at 12/18/24 pt measured in PowerPoint; line pitch of measured and substituted families | [rendering](rendering.md#line-height) |
| `make_empty_bullets.py` | Empty and whitespace-only paragraphs draw no marker | [rendering](rendering.md#bullets-and-indentation) |
| `make_embedded_media.py [size_mib]` | An embedded 40 MiB video and a poster frame; write it outside the repository | [rendering](rendering.md#known-limitations) |
| `make_text_inheritance.py` | Inherited versus explicit 18 pt; the master `otherStyle` is 24 pt so a wrongly applied style shows | [rendering](rendering.md#inheritance) |
| `make_text_autofit.py`, `make_text_bullets.py`, `make_vertical_text.py`, `make_fields.py` | Autofit, bullets, vertical text, text fields | [rendering](rendering.md) |
| `make_metafile_pictures.py OUT_DIR [--pptx]` | Nine EMF/WMF files isolating record groups; `tests/fixtures/metafile/` holds them for `tests/metafile_pictures.rs` | [rendering](rendering.md#emf-and-wmf-pictures), [survey](history/20261020_emf-wmf-survey.md) |
| `make_arrow_sizes.py`, `make_connector_routing.py`, `make_ellipse_text.py`, `make_lines.py`, `make_preset_shapes.py`, `make_theme_lines.py` | Presets, lines, connectors | [preset shapes](preset-shapes.md) |
| `make_auto_punctuation.py`, `make_boundary_widths.py`, `make_character_spacing.py`, `make_composite_shapes.py`, `make_punctuation_positions.py` | Text measurement fixtures; `compare_boundary_widths.py` and `compare_punctuation_positions.py` add line-level and ink-distribution checks | [text measurement](history/20261015_text-measurement.md) |
| `make_colors.py`, `make_fills.py`, `make_gradients.py` | Colors and fills | [colors](colors.md) |
| `make_ole.py`, `audit_ole.py` | OLE previews; structural audit of a deck | [OLE previews](ole-preview.md) |
| `make_shadows.py` | Outer shadows | [shadows](shadows.md) |
| `make_table_styles.py` | Table styles | [table styles](table-styles.md) |
| `pdf_output.py` | A text/table/picture deck rendered to PDF and rasterized with PDFium against the PNG output | [rendering](rendering.md#pdf-output) |
