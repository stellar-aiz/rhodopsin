---
date: 2026-11-02
kind: decision
principles: [fidelity, transparency, observability]
---

# XLSX default layout measured from Excel for Mac

**Date**: 2026-11-02
**Status**: Accepted. The CLI renders workbooks inside the measured conditions; everything else stops with a located reason.
**Canonical code**: `src/xlsx/default_layout.rs` (the rule and its limits), `src/xlsx/text_layout.rs` (`position_line_in_box`, `snap_advances`), `src/xlsx/workbook_layout.rs` (whole-pixel digit width), `src/xlsx/page_borders.rs` and `page_borders/paint.rs` (thin-edge precedence, projecting solid ends), `examples/xlsx-compare-pages.rs` (position comparison), `tests/fixtures/xlsx/excel-reference/`.

## Decision

The default path (`convert_with_options`, and so `rhodopsin book.xlsx --format pdf`) lays out cells by the rules below. They were read from PDFs that Excel for Mac wrote, and they apply only where they were measured: every printed cell font and the Normal style are Noto Sans JP, the Normal style is 11 pt, and cell sizes are whole points from 6 to 40. A cell outside these conditions stops with `cell_layout_unmeasured`; rich text stops with `rich_text_line_box`; a border other than thin, medium, or thick stops with `border_style_unmeasured`; text that needs more than one line keeps stopping with its existing reason. Agreement is claimed only for the recorded environment.

| Rule | Value |
| --- | --- |
| Column width | The OOXML formula with the maximum digit width rounded to whole pixels (Noto Sans JP 11 pt: 8 px at 96 dpi, so width 30 → 180 pt) |
| Horizontal padding | Left `floor((size + 5) / 7) + 1` pt (2 pt at 6-8 pt, +1 at 9, 16, 23, 30, 37); right one point less |
| Line box | Single-line text sits in a box whose height is Excel's natural row height for the size (`LINE_BOXES`), with the baseline `round(1.16 × size) + 1` pt below its top; top, centre, and bottom alignment place the box in the cell |
| Glyph advances | Each glyph advance is rounded to whole points (at 100 %, then scaled), for alignment and for the glyph positions in the PDF |
| Borders | Thin, medium, thick: 1, 2, 3 pt; solid lines extend by half their width at both ends so box corners are closed |
| Shared edges | A thin edge yields to a medium or thick edge of the same colour on the same segment; medium against thick and different colours still stop |
| Colour | Automatic colour (no `<color>` or `auto="1"`) prints black |
| Fit to page | The existing search (100 % down in whole percent) chose 63 % for the landscape fixture, as Excel did |
| Locale | `--locale ja-JP|en-US`, default `ja-JP`; the OS region is not read |

## Why

Excel for Mac lays a sheet out on a whole-point grid at 100 % and scales the page afterwards: cell boundaries, text origins, and border rectangles in its PDF are integers before the print scale is applied. The rules above reproduce that grid closely enough for every compared text origin, border, and fill to land within 1 pt.

The line box explains all three vertical alignments with one table. Top-aligned baselines were `round(1.16 × size) + 1` pt below the row top for every size from 6 to 40. Bottom-aligned baselines were `height - round(1.16 × size) - 1` above the row bottom, where `height` equals the row height Excel saved for a row holding only that size (12-40 pt), and 5 pt for 6-12 pt, where the Normal style's 11 pt row (19 pt) is taller. Centre alignment places the box `(row - height) / 2` below the row top; Excel used the floor of that value, at most 0.5 pt higher. The table is font specific; no general formula from the font's metrics matched every size, so other families are not guessed.

Rounded glyph advances matter for amounts: a right-aligned `¥1,479,690` in bold is 56 pt wide with whole-point advances and 59 pt with exact ones, which moved the last digit under the border. The same rounding showed in Excel's text operators (`Tc` and `TJ` offsets that bring each glyph back to a whole point).

The digit-width rounding and the thin-edge precedence change shared code because the observation is not specific to the default path; the explicit-policy API follows the same rule. The previous design rule ("the thicker or later cell is never picked") is narrowed to the pairs Excel was not observed on.

## Evidence

Environment: Microsoft Excel for Mac 16.105.3, macOS 27.0.1 (26A434), English UI, region Japan; LibreOffice 26.2.5.2 for comparison; Noto Sans JP 2.004 Regular and Bold (static instances shipped by the downstream product; hashes in each `reference.json`) installed in `~/Library/Fonts`, with Excel restarted after installation.

Procedure: `make_fixtures.py` (openpyxl 3.1.5) generated three workbooks; Excel opened each and saved it as `.xlsx` (the committed `input.xlsx`, an Excel-saved file like real inputs); Excel then opened that input and saved it as PDF through AppleScript (`save workbook as … file format PDF file format`) and closed it without saving; the input hash was unchanged. Excel asks once per folder and file for sandbox access; a person granted it. LibreOffice converted the same inputs headless with a fresh profile. `xlsx-check-reference` accepts all three records.

Probes (`tests/fixtures/xlsx/excel-reference/probes/`): `alignment` (sizes 8-28 pt, top/centre/bottom, left/centre/right, 48 and 60 pt rows) and `line-box` (sizes 6-40 pt in natural-height rows, bottom and top; the Excel-saved copy records the row heights).

Comparison at revision `58c1d9d` (rhodopsin with `--no-system-fonts` and the two Noto files):

| Fixture | Pages | Text origins (max Δ) | Borders (max Δ) | Fills (max Δ) | Raster mean |Δ| vs Excel: rhodopsin / LibreOffice |
| --- | --- | --- | --- | --- | --- |
| quote | 1 | 26 runs, 1.0 pt | 14, 0.5 pt | 2 colours, 1.0 pt | 2.01 / 5.87 |
| form | 2 | 123 runs, 0.4 pt | 2, 0.5 pt | 2 colours, 1.0 pt | 1.83, 2.00 / 4.65, 5.20 |
| landscape | 1 | 420 runs, 0.63 pt | 1, 0.18 pt | none | 6.62 / 11.49 |

Rasters are pdftoppm 26.10.0 at 100 dpi (827 × 1170 px), unaligned, compared with `tools/compare-document.py`; pixel equality is not expected from different rasterizer inputs, and side-by-side crops are attached to the pull request for review. Page count, page size (in whole points), and first and last printed cells (marker strings) agree for all fixtures.

## Known differences and open items

- Excel rounds the page origin to a whole point at 100 % (50.4 pt margin → 50 pt); rhodopsin keeps 50.4, a 0.4 pt shift everywhere.
- Excel's fills and lines extend one point right and down of the cell edge; rhodopsin fills the cell rectangle (≤ 1 pt).
- A 16 pt title centred in a 30 pt merged row is 1.0 pt lower than Excel; the probe rows centred within 0.5 pt. Excel's centring for merged cells is not modelled separately.
- Quartz writes Kangxi radicals (U+2F92 for 見, and others) in Excel's ToUnicode, so Excel's extracted text differs from the visible text; rhodopsin writes the unified ideographs. The comparator maps the two radical blocks back before comparing.
- Whole-page extracted text differs in order and spacing (Excel draws the merged title last, one text object per cell); strings are compared per text run instead.
- LibreOffice's PDFs are tagged (`BDC`), so the comparator records them as `unobserved`; their rasters are compared.
- Not measured: other families (including the Normal style at other sizes, which may change the 5 pt floor below 12 pt), rich text, multi-line text, other border styles, medium against thick, different colours on one edge, Excel for Windows.
