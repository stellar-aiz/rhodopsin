---
kind: guide
---

# Experimental PDF input

`rhodopsin input.pdf --out pages --width 1600` writes `page-001.png` etc. and
`manifest.json`. The `pdf-input` feature is enabled by default. Library callers
use `rhodopsin::pdf_input::convert` with `pdf_input::Options`. Width is 64–4096.
Unsupported content fails in normal and strict modes. The destination must not
exist, even as a dangling symlink. Pages and the manifest are staged beside the
output and published together using a no-replacement rename. Handled failures
leave no final directory. Crash recovery is not provided. Existing PPTX/XLSX
publication behavior is unchanged. Filesystems must support exclusive rename.

This is an independently implemented Rust PDF syntax reader and interpreter.
It uses the existing resvg/tiny-skia drawing facilities, miniz_oxide, png,
tempfile, rustix and Windows-only windows-sys OS bindings; no native PDF engine or runtime child process is added.
No new dependency versions or third-party source/assets are introduced.
Existing dependency licenses and NOTICE attributions continue to apply.

## Supported subset

Support depends on structure, not Producer metadata. This does not cover all
rhodopsin output, XLSX PDFs, or general PDFs.

- PDF 1.x with classic cross-reference tables, generation-zero indirect objects,
  direct stream lengths, and a single revision. Content streams are uncompressed.
- Page trees with inherited MediaBox, CropBox, Rotate and Resources. MediaBox
  starts at (0,0), with positive dimensions up to 14,400 points. CropBox must
  equal MediaBox, Rotate must be zero, UserUnit must be one. White opaque output.
- Paths: `q Q cm m l c h f f* S n W W*`, `w J j M d`, `rg RG g G`.
  Transform changes within an unfinished path and zero-width hairlines fail.
- Type 3 text: `BT ET Tf Tm Tj`, Encoding Differences, FontMatrix, Widths and
  CharProcs using `d0`. Glyphs are interpreted as paths without Unicode shaping.
  `BMC BDC EMC` wrappers and ActualText are structural/extraction metadata.
- Indirect opaque 8-bit DeviceRGB image XObjects through `Do`, unfiltered or
  FlateDecode, without DecodeParms, Decode, masks or Interpolate overrides.
  Images follow PDF's default noninterpolated sampling.
- `gs` only for neutral alpha 1 and Normal blend mode; other graphics state
  settings fail. Used drawing resources must be indirect objects.

Unknown drawing operators, unsupported fonts, gradients, transparency, forms,
annotations, encryption, incremental updates and unsupported filters fail.
Unused drawing resources and ToUnicode metadata do not affect rendering.
Actions are never executed; catalog/page actions, forms, name trees and optional
content properties are rejected. Errors use fixed descriptions with page/content
object context where available, never copied document text or raw I/O errors.

## Limits

These are fixed initial implementation budgets, checked during processing:

| Resource | Limit |
|---|---:|
| Input | 32 MiB |
| Xref objects / page-tree visits | 100,000 each |
| Syntax tokens across objects | 1,000,000 |
| Allocated strings / copied streams | 64 MiB each |
| Syntax/reference/page-tree/graphics/clip depth | 64 (strictly smaller for syntax/reference/tree) |
| Glyph drawing recursion | 32 |
| Drawing operators | 200,000 per document |
| Operands / dash elements | 128 each |
| Path data / scene data | 1 MiB / 32 MiB |
| Decoded stream / cumulative decoded bytes | 16 MiB / 64 MiB |
| Image dimensions / pixels | 4096 per axis / 4,194,304 |
| Pages | 100 |
| Page pixels / cumulative page pixels | 16,777,216 / 67,108,864 |
| Page height | 16,384 pixels |
| Conservative raster work | 1,073,741,824 pixel-paints |

Raster work charges each primitive and its clips the full page area; this can
reject complex otherwise supported pages. These limits are not a hard bound on
process RSS or wall-clock time. They bound parser, decoder and interpreter work.

## Verification and known differences

Generate a new evidence directory and compare the **same** PDF with PDFium:

```sh
RHODOPSIN_PDF_INPUT_EVIDENCE=/tmp/pdf-input-check \
  cargo test --locked --test pdf self_produced
.venv-visual/bin/python tools/compare_pdf_input.py /tmp/pdf-input-check
```

PDFium (`pypdfium2`) and Pillow are development-only dependencies from the
existing visual-test environment, never part of the shipped converter.
Whole-page and fixed text/path/image regions use MAE <= 0.02 and changed-pixel
ratio <= 0.05 with per-channel delta > 16. Independent Rust fixtures also test
geometry, clipping, state restoration, image orientation, failures and limits.

Direct PPTX PNG comparison is **diagnostic only**. The tiny two-color image in
the round-trip fixture is interpolated by the PPTX renderer, while the PDF writer
omits Interpolate and PDFium uses discrete sampling. The new PDF reader matches
PDFium: the image region has zero difference, but differs from direct PPTX by
MAE 0.13046 / changed ratio 0.48333. Input documents, both comparisons and diffs
are retained; no threshold is relaxed and the PDF writer is unchanged.
Passing these fixtures does not establish general PDF or Office compatibility.
