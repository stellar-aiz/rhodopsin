---
kind: guide
---

# XLSX to PDF: three-way validation

From the same non-confidential input, obtain a PDF from Excel, from rhodopsin, and from LibreOffice. Excel's "Save as PDF" is the oracle; LibreOffice is an independent observation point for separating causes, not a vote. Excel and LibreOffice are used only in the development environment, never as runtime dependencies.

## Verdicts

Three pairs are compared with the same comparator and conditions: rhodopsin vs Excel (pass/fail), LibreOffice vs Excel (compatibility gap), rhodopsin vs LibreOffice (shared vs individual differences).

| Observation | Verdict and next step |
|---|---|
| rhodopsin matches Excel | The checked item passes; LibreOffice's difference is recorded separately |
| Only LibreOffice matches Excel | Investigate rhodopsin's interpretation, layout, or drawing |
| rhodopsin and LibreOffice agree but differ from Excel | Fail; investigate Excel-specific behaviour or a shared assumption |
| All three differ | Fail; check input, environment, and method, then decompose the difference |

"Match" refers to the item under test. A LibreOffice failure or missing output is recorded as `unobserved` and never counted as a match. Byte equality of PDFs is not the pass criterion; rhodopsin's own determinism is checked separately by regenerating from the same input and fonts. Tolerances for positions and raster differences are fixed with a recorded reason; alignment, cropping, or relaxation to hide a difference is not done, and known differences are tied to a reproducing fixture.

## Evidence to keep

Per fixture: the input `.xlsx`, the Excel reference PDF, the LibreOffice comparison PDF, and rhodopsin's PDF and comparison result tagged with the implementation revision. Reference PDFs are never overwritten by ordinary test runs. The generation record holds: hashes of the input and each PDF; application versions and OS; font family, version, and file hash (font files themselves are not stored); locale, date system, print settings, and PDF export procedure; comparator and rasterizer versions, resolution, and conditions; tolerances with their reasons, known differences, and investigation status. Each application opens the same input without resaving it; if recalculation or font substitution occurred, it is recorded as an environment difference and the comparison is not treated as same-input.

Items checked: page count and size, first and last cell of each page (repeated rows and columns recorded separately), all strings, amounts, text positions, font embedding, and rasterized image differences.

## Tools

The tools are the `examples/xlsx-*.rs` examples and `tools/compare-document.py`; they read existing PDFs or PNGs, and none of them rasterizes a PDF, produces a verdict, or is a runtime dependency. None of the tools copies file paths, PDF strings, or child-process stderr into its output.

```sh
cargo build --locked --example xlsx-compare-pages --example xlsx-compare-rasters --example xlsx-check-reference
target/debug/examples/xlsx-compare-pages excel.pdf rhodopsin.pdf libreoffice.pdf > comparison.json
target/debug/examples/xlsx-compare-rasters excel.png rhodopsin.png libreoffice.png --diff-dir raster-differences > raster-comparison.json
target/debug/examples/xlsx-check-reference path/to/fixture/reference.json
python3 tools/compare-document.py comparison-input.json \
  --pdf-comparator target/debug/examples/xlsx-compare-pages \
  --raster-comparator target/debug/examples/xlsx-compare-rasters \
  --out-dir target/comparison
```

- **compare-pages** (`schema_version` 4): SHA-256 of each PDF, per-page MediaBox/CropBox in points with inherited values and `UserUnit` resolved, rotation, the extracted text of each page by SHA-256, character count, and UTF-8 length (lopdf 0.38; the text itself is not output), and the positions of text runs, borders, and fills. Form XObjects, quoted text operators, BDC, unresolved fonts, text shown after a font restore, rotated text, and text shown without a fresh position are reported as `unobserved` with a reason rather than silently extracted. These comparison rules are fixed before comparing and are not relaxed to hide a difference:
  - Page sizes are compared after rounding to whole points; Excel for Mac writes 595×842 for A4.
  - Kangxi radicals and CJK radical supplements (U+2F00-U+2FDF, U+2E80-U+2EFF) are mapped to unified ideographs before text is compared, because Quartz writes them in Excel's ToUnicode (見 as U+2F92); no other character is normalized. `raw_extracted_text` keeps the unmapped comparison.
  - Each text-show operation of the reference is paired with the nearest sequence of the candidate's text-show operations on the same baseline whose text, without whitespace, is equal; Excel writes one run per cell and rhodopsin one position per glyph. The run origin, baseline, and size must agree within 1 pt (`position_tolerance_pt`). Unpaired runs on either side are differences. The whole-page extracted text depends on drawing order and is reported but not part of the exit code.
  - Filled rectangles and stroked horizontal or vertical lines at most 4 pt thick are borders: each must have a counterpart of the same orientation within 1 pt in centre and thickness, and every point of its centre line must lie within 1 pt of the other side's borders. Thicker fills are compared per color (±1 of 255) by their bounding box within 1 pt.

  Exit 0 means rhodopsin and Excel agree on page geometry and on every text, border, and fill position; 1 any difference; 2 when Excel or rhodopsin is unobserved or the run fails. Glyph positions within a run, expected cell strings, amounts, embedding, and images are listed under `unchecked`, and `overall_verdict` stays `not_evaluated`. The tool has no PDF-expansion memory limit comparable to the CLI and is for managed fixtures only.
- **compare-rasters**: for one page rendered to opaque 8-bit PNG by the same rasterizer, version, resolution, background, and antialiasing: SHA-256 and dimensions of each input, and per pair the differing pixel count, maximum channel difference, per-channel absolute sums and means, and the bounding box of the difference (top-left origin, exclusive right/bottom). No alignment, color correction, scaling, or thresholding. Transparent images and unsupported metadata (ICC profiles) are rejected; inputs are limited to 32 MiB each, 64 MiB decoded, 16384 px per side. With `--diff-dir` on an existing directory it also writes `rhodopsin_xlsx_vs_excel.png`, `libreoffice_vs_excel.png`, and `rhodopsin_xlsx_vs_libreoffice.png` (unamplified RGB absolute differences, lossless, never overwriting; a save failure gives exit 2 but the numbers are still printed). Exit 0 only for pixel-identical Excel and rhodopsin.
- **check-reference**: verifies a fixture's `reference.json` (`schema_version` 1: `input` and `excel` with file, SHA-256, application, version, OS, procedure, `recalculated`, `fonts_substituted`; optional `libreoffice` in the same shape or `null`; `locale`, `date_system`, `fonts` with family, version, and hash). Paths are relative to the record, files are read-only and limited to 128 MiB (the record to 64 KiB), font files are not read. A recorded recalculation or substitution is not accepted as same-condition. Exit 0 means record and hashes are consistent, 2 otherwise; the result states `provenance: declared_not_authenticated` and `overall_verdict: not_evaluated`. The records under `tests/fixtures/xlsx/excel-reference/` satisfy it.
- **compare-document.py** (standard library only): runs the two comparators over every page listed in `comparison-input.json` (`schema_version` 1; `inputs.excel|rhodopsin_xlsx|libreoffice` each with `pdf` and ordered `pages` PNGs, relative to the record) and writes `comparison.json` plus per-page difference PNGs to a new directory. The PDF's page count is checked against the PNG list; missing or extra pages and unobserved PDFs are not a complete comparison. Differences on any page propagate to the exit code (0 agree, 1 differ, 2 unobserved or save failure); LibreOffice gaps are recorded independently. Record limited to 1 MiB, 10,000 pages per input, 120 s per comparator process, one page at a time. `overall_verdict` is always `not_evaluated`.

The comparators' synthetic tests pass. The three fixtures in `tests/fixtures/xlsx/excel-reference/` were compared with PDFs written by Excel for Mac 16.105.3; the result is in [the layout record](history/20261102_xlsx-excel-mac-layout.md). LibreOffice's tagged PDFs are `unobserved` by `compare-pages`, and their rasters are compared.
