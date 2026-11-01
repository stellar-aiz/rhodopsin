---
kind: guide
---

# XLSX to PDF: design rules and readiness

The `rhodopsin::xlsx` module (`src/xlsx/`, behind the default `xlsx` feature) will render the printed pages of an `.xlsx` workbook to PDF. The CLI routes `.xlsx` input to it (`rhodopsin book.xlsx --format pdf`, see [usage](usage.md#xlsx-input)). The requirements it works toward are in [xlsx-requirements.md](xlsx-requirements.md), and the comparison method in [xlsx-validation.md](xlsx-validation.md).

**Status: no workbook is converted yet.** `convert_with_options` validates the package, workbook, styles, and fonts, then stops with `UNSUPPORTED_FEATURE` / `rendering_not_implemented` (`src/xlsx/mod.rs`). Inputs that fail earlier report their own reason code. XLSX input therefore always behaves as `--strict`. The library's development API (`convert_with_policy`, `PreparedWorkbook::render_with_policy`, `render_with_cell_overflow`, `render_with_borders` in `src/xlsx/render.rs`) renders a workbook when the caller supplies the layout policy that Excel decides for itself (content rectangles, line pitch, clip rules); that policy is what remains to be derived and verified.

## Design rules

These are the decisions that cannot be read from the code. Each is enforced by a reason code and fixed by tests under `tests/xlsx_*.rs` (test names are Japanese).

- **Stop, never approximate silently.** Anything the renderer cannot reproduce stops with a located reason instead of a guess: a missing glyph is not patched from another font, a locale is not inferred, a font family is not inferred from a file name. Per-element approximate rendering with warnings is planned but not implemented.
- **Diagnostics carry position, count, and kind only.** No cell values, strings, formulas, sheet names, paths, or raw library errors reach a diagnostic (`src/xlsx/diagnostic.rs`; the CLI adds a silent panic hook for the same reason, `src/cli_xlsx.rs`).
- **Formulas are never recalculated.** The stored cached value and its decimal lexeme are used; a missing cache stops with `formula_cached_value_missing`.
- **Decimal arithmetic on stored lexemes.** Dates, times, exponents, fractions, and General candidates are computed from the decimal text, never through an `f64` round trip (`format/`, `text_layout.rs`).
- **Number format resolution.** An explicit `formatCode` beats `numFmtId`. Built-in date/time ids 14-22 and 45-47 resolve only when the context declares en-US (14, 22, 47 follow the MS-OI29500 Excel differences); the Japanese ids 27-36 and 50-58 resolve only under a declared ja locale; otherwise the id is rejected rather than guessed. Ids 5-8, 23-26, 41-44, and 63-66 require the stored `formatCode`, else `locale_number_format_missing`.
- **General is width dependent.** `text_layout::measure_general_number` chooses between fixed notation (10 down to 0 decimals, trailing zeros stripped, 11 characters excluding the sign) and scientific notation by measuring candidates in the real font; the width-less `format` API returns `general_number_layout` instead of fixing a string. A non-zero value is never rounded to zero; when nothing fits, the `#` fill and width warning apply. See [the fraction and General record](history/20261020_xlsx-fraction-general.md).
- **DBNum** converts only non-negative integer digits under a declared ja locale; General's 万/億 units are never substituted by digit conversion. `parse_with_locale` passes the caller's locale; an in-section `[$-409]` is not treated as Japanese.
- **Indent** is 3 spaces of the Normal font, found through `cellStyles builtinId=0`, scaled by the print scale.
- **Underline** uses the `post` table top and thickness; the top is not the line centre. Only single underline is supported; double and accounting underlines stop (`underlined_cell_text`, `formatted_text_underline`) until Excel is measured ([record](history/20261021_xlsx-underline.md)).
- **Bidi** text is rejected with `text_direction` when any R, AL, AN, or directional control character appears, so no run is drawn in a wrong single direction.
- **Multi-glyph clusters** are accepted only when each source character maps to one glyph in order; substitution or reordering stops with `pdf_multi_glyph_cluster` (`pdf/text_mapping.rs`).
- **Shared-edge borders** that disagree in style or color stop with `border_conflict`, and partial overlaps with `border_partial_overlap`; the thicker or later cell is never picked because Excel's precedence is unverified (`page_borders.rs`). A merged range whose outer edge disagrees stops with `merged_border_style_conflict`.
- **OS font probing** reads only sfnt directories and name tables (64 MiB total, 1 MiB per table) and loads the bodies of files that contain a requested family, through the same handle, so a path swapped after indexing still reads the original file (`fonts.rs`, `system_fonts.rs`). Relative `HOME`/data directories stop with `invalid_system_font_directory`.
- **Limits are best effort**, applied per layer (package, fonts, layout, scene, PDF). They are not a strict process peak-memory guarantee.
- **Output is never overwritten**; the PDF is written to a temporary file in the output directory and published exclusively (`output.rs`).
- **Nothing external at runtime**: no network, no child process, no Excel, LibreOffice, or Python.

## Evidence

Fixtures under `tests/fixtures/xlsx/` and the workbooks built inside tests are synthetic; `tests/xlsx_support/mod.rs` generates minimal sfnt fonts with empty outlines. These prove control flow (selection, measurement, rejection), not appearance. No Excel-generated reference workbook or PDF exists yet, so no claim of page-level agreement with Excel has been verified, and none should be stated. The probe generators `cargo run -p rhodopsin-xlsx --example make-fraction-probe|make-general-probe|make-underline-probe -- OUT.xlsx 'Font Name'` write deterministic observation workbooks for the three open measurement questions; they are inputs for an Excel observation, not fixtures.

## Not finished

| Requirement | Current evidence | Remaining |
|---|---|---|
| One CLI call produces a PDF and a success document | Every workbook stops with `rendering_not_implemented` | Connect verified default layout, PDF output, and the success report into one path |
| Same page as Excel | No Excel reference PDF | Obtain references under recorded conditions; compare per [xlsx-validation.md](xlsx-validation.md) |
| General number format | Width-dependent selection connected to cell layout | Verify selection, rounding, and exponent boundaries against Excel |
| DBNum1-3 | Non-negative integer digits with declared ja locale | Units, decimals, signs, dates |
| Borders | Candidates, shared-edge coalescing, merged outer edges, explicit-policy drawing (`render_with_borders`); the default path still rejects used borders | Line widths, dashes, double lines, junctions, precedence |
| Underline | Single underline only | Double and accounting underline geometry after measurement |
| Wrapping and alignment | `render_with_policy` takes content rectangles and line pitch from the caller | Excel's padding, line pitch, and break rules as the default |
| Text overflow | `render_with_cell_overflow` clips by occupancy | Verify Excel's boundary rules |
| Merged cells | Top-left cell content and fill | Merges across page breaks, split text, edge rules |
| Text shaping and extraction | TrueType/CFF subsetting with ToUnicode; one-to-one clusters | Substituting clusters, script splitting |
| Target OS and sandbox | CI runs natively on ubuntu, macOS, and Windows; one macOS Seatbelt denied-read test (`tests/xlsx_cli.rs`) | Strict read permissions; AppContainer and bwrap are not exercised anywhere in this repository |
| Three-way comparison | `compare-pages`, `compare-rasters`, `check-reference`, and `tools/compare-document.py` exist with synthetic tests | A real Excel reference; cell-level, position, amount, and embedding checks |
| Dependency licences, SBOM, NOTICE, distribution bundle | `tools/dependency-artifacts.py` and `tools/distribution-bundle.py` run against the workspace from the repository root: the SBOM is rooted at the `rhodopsin` binary, first-party crates are exempt from the permissive allow list, and the bundle carries the root `LICENSE` (MIT) ([dependencies](history/20261020_xlsx-dependencies.md), [distribution](history/20261020_xlsx-distribution.md), [licence](history/20261030_licence.md)) | No CI job produces or stores a bundle |

The CLI also differs from the requirements in two places that are tracked rather than hidden: `.xls` and `.xlsm` input falls through to the PPTX pipeline instead of a reasoned rejection ([#112](https://github.com/stellar-aiz/rhodopsin/issues/112); the library itself rejects them with `input_extension_not_xlsx` / `encrypted_or_legacy_workbook`), and font substitution (`fallback_family`) and font directories exist in `ConversionOptions` but are not exposed by the CLI.

## Working on the spreadsheet module

```sh
cargo test --locked xlsx
cargo test --locked --test xlsx_cli
cargo clippy --locked --all-targets -- -D warnings
python3 -m unittest discover -s tools -p 'test_*.py'
```

The spreadsheet integration tests are `tests/xlsx_*.rs` and its development probes are the `examples/xlsx-*.rs` examples. Primary sources consulted for the rules above; none of them substitutes for a comparison with Excel output:

- [NumberingFormat](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.numberingformat?view=openxml-2.8.1) and [Excel's built-in format differences (MS-OI29500)](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/17d11129-219b-4e2c-88db-45844d21e528)
- [Excel date systems](https://support.microsoft.com/en-us/excel/date-systems-in-excel) and [Japanese era boundaries](https://learn.microsoft.com/ja-jp/dotnet/api/system.globalization.japanesecalendar.getera?view=netframework-4.8.1)
- [Alignment indent](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.alignment?view=openxml-3.0.1), [PageSetup](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.pagesetup?view=openxml-3.0.1), [ST_Xstring](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/bd0aa042-434a-4ca7-b25f-4e1fd25a954d)
- [OpenType `name`](https://learn.microsoft.com/en-us/typography/opentype/spec/name) and [`post`](https://learn.microsoft.com/en-us/typography/opentype/spec/post) tables, [Mac Roman mapping](https://www.unicode.org/Public/MAPPINGS/VENDORS/APPLE/ROMAN.TXT)
- [ToUnicode CMaps](https://pdfa.org/norm-refs/5411.ToUnicode.pdf) and [Unicode in PDF 2.0](https://pdfa.org/understanding-utf-8-in-pdf-2-0/)
- [Apple font locations](https://support.apple.com/en-ie/122869), [Windows known folders](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid), [Fontconfig](https://fontconfig.pages.freedesktop.org/fontconfig/fontconfig-user.html)
 Convergence with the PPTX side (fonts, then the PDF writer) is recorded in [the import decision](history/20261022_xlsx-import.md).
