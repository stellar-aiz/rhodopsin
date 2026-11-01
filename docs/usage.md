---
kind: guide
---

# CLI and Rust API

## Installation and output

Install from a [GitHub release](#install-from-a-release) or build from a checkout.

Build with `cargo build --release --locked`, or install from a checkout with
`cargo install --path . --locked`. The initial dependency download needs network
access; conversion itself is local and does not start Office or access the network.

```sh
rhodopsin slides.pptx --out images --width 1600
rhodopsin slides.pptx --out verified --strict
rhodopsin slides.pptx --out images --font-file /path/to/font.ttf
```

Without `--out`, replace the input's final extension with `.rhodopsin` beside the
input: `slides.pptx` becomes `slides.rhodopsin/`. An explicit output path wins.
Existing output directories are never overwritten. Output consists of numbered
`slide-001.png` files and `manifest.json`. An I/O failure can leave partial images;
the manifest is written only after successful completion.

Output width must be 64–4096 pixels (default 1600). See `rhodopsin --help` for all arguments.

## Install from a release

Tagged versions are published as GitHub pre-releases with one ZIP per target
(`aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`) and a
`SHA256SUMS` file. Each ZIP holds the executable, `LICENSE`, `NOTICE.txt`, the
dependency licence texts, an SBOM, and a manifest whose status is `development`.

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing   # Linux: sha256sum -c
unzip rhodopsin-<version>-<target>.zip
```

On Windows, compare the output of `Get-FileHash <zip> -Algorithm SHA256` with the
matching line of `SHA256SUMS`.

The checksum confirms that the download is intact; it does not prove who published it.
The binaries are not signed. On macOS, a ZIP downloaded with a browser is quarantined
and the executable is blocked on first run; clear the flag for that one file:

```sh
xattr -d com.apple.quarantine rhodopsin-<version>-aarch64-apple-darwin/rhodopsin
```

## PDF output

```sh
rhodopsin slides.pptx --format pdf --out slides.pdf
rhodopsin slides.pptx --format pdf --font-file /path/to/font.ttf
```

PNG remains the default. PDF mode writes one file, with one page per rendered
slide, in presentation order. Without `--out`, `slides.pptx` becomes `slides.pdf`
beside the input. Pages retain the original physical slide dimensions; `--width`
is ignored for PDF output (the CLI still validates its numeric range).
`--include-hidden`, font selection, and `--strict` retain their existing meanings.

The output is published only after it is complete. Existing files and symlinks
are never replaced, even if another process creates the destination during
conversion. Publication requires a filesystem supporting same-directory hard
links (including typical NTFS, APFS, and ext4 filesystems); otherwise it fails
explicitly. Ordinary errors clean up the temporary file. A killed process may
leave a `.rhodopsin-<pid>-<counter>.tmp` file, but no partial final PDF.

Success YAML includes `output.file`, `output.pages`, and `report`, a JSON flow
mapping valid in YAML 1.2. `report` contains `fonts`, per-slide `warnings` and
`diagnostics`, physical page dimensions, and skipped hidden slide numbers.
`report.slides[].slide` identifies the original deck slide and `.page` the PDF
page. Save stdout if you need these diagnostics; no sidecar manifest is written.
PDF-specific failures identify the slide and relevant unsupported feature/font.

The library exposes `convert_pdf(input, out, &ConvertOptions)`, returning
`PdfConversionReport`. It ignores `ConvertOptions.width`, has no stdout/stderr
side effects, and does not change the existing `convert` API or PNG report.

```rust,no_run
use rhodopsin::{ConvertOptions, convert_pdf};
let report = convert_pdf("slides.pptx", "slides.pdf", &ConvertOptions::default())?;
assert_eq!(report.pages, report.slides.len());
# Ok::<(), anyhow::Error>(())
```

PDF support is deliberately limited initially; see
[PDF rendering limits](rendering.md#pdf-output). A successfully displayed PDF
does not guarantee identical text extraction in every reader or downstream
service. Check representative text, numbers, and tables in the consuming service.

## XLSX input

`.xlsx` workbooks use the same command and are converted to PDF only.
The spreadsheet renderer is not connected end to end yet, so no workbook is converted: unsupported content and invalid input are reported with their own reason codes, and a workbook that passes validation and font loading stops with `UNSUPPORTED_FEATURE` / `rendering_not_implemented`.
Its design rules and open items are in [xlsx.md](xlsx.md).

```sh
rhodopsin book.xlsx --format pdf
rhodopsin book.xlsx --format pdf --out printed.pdf --font-file /path/to/font.ttf
```

| Option | XLSX input |
| --- | --- |
| `--format pdf`, `--out` | Supported. Without `--out`, `book.xlsx` becomes `book.pdf`. |
| `--font-file` | Supported; repeatable. |
| `--no-system-fonts` | Supported. By default the OS font directories are searched for the families the workbook uses. |
| `--strict` | Accepted. XLSX input always behaves as strict: unsupported content stops the conversion instead of being approximated with a warning. |
| `--width` | Ignored, as for PDF output. |
| `--format png`, `--font-fallback`, `--font-map`, `--include-hidden`, `--punctuation-compression` | Rejected with exit code 2 (`FORMAT_NOT_SUPPORTED` or `OPTION_NOT_SUPPORTED`). |

Font substitution for XLSX input is not exposed; the renderer's own font directory and fallback-family options were not carried over to the CLI.
When the OS font search is enabled and the home or data directory environment variables point to relative paths, the conversion fails with `invalid_system_font_directory`.
The `.xlsx`, `.xlsm`, and `.xls` extensions (case-insensitive) are routed to the spreadsheet renderer; other extensions are treated as PPTX. `.xlsm` and `.xls` are refused there with `INVALID_INPUT` and the reason `input_extension_not_xlsx` (macro-enabled and legacy workbooks are not converted), and a legacy or encrypted file renamed to `.xlsx` is refused with `encrypted_or_legacy_workbook`.

Failures use the same YAML layout as PPTX input and never include cell contents, sheet names, or paths:

```yaml
status: failed
output:
  created: false
errors:
  - code: INVALID_INPUT
    reason: formula_cached_value_missing
    location:
      sheet: 1
      cell: "A1"
    suggestion: "..."
```

`code` is one of `INVALID_INPUT`, `UNSUPPORTED_FEATURE`, `FONT_NOT_FOUND`, `LAYOUT_FAILED`, and `INTERNAL_ERROR`.
`reason` is the renderer's fixed reason code, and `location` appears when the sheet (one-based, in workbook order) or cell is known.
Conversion failures exit with 1, as for PPTX input.
A successful conversion prints `status: completed` with `file`, `pages`, and a `report` object listing pages, fonts, and warnings.

## Hidden slides

Slides marked hidden in PowerPoint (`p:sld/@show="0"`) are skipped by default, like
PowerPoint's PDF export and slide show; the images show what the audience sees.
Pass `--include-hidden` to render them too (`ConvertOptions.include_hidden` in the
library). Image names and `slides[].slide` use the one-based position in the deck,
counting hidden slides, so skipped slides leave gaps: a deck whose slide 2 is hidden
produces `slide-001.png` and `slide-003.png`. A deck whose slides are all hidden fails
unless `--include-hidden` is given. A skipped slide is parsed once to read its `show`
attribute and is not rendered, so its content produces no warnings. Skipped slides
count toward the 200-slide limit.

## Font discovery and fallback

System fonts are loaded by default. This is not an exhaustive search of the
filesystem: fonts inside application bundles or inaccessible directories may not
be found. Missing-family errors mean a family is unavailable in the loaded font
database, not necessarily absent from the machine.

Use repeatable `--font-file` arguments for fonts you can access and are entitled
to use. To isolate conversion from system fonts:

```sh
rhodopsin slides.pptx --out images \
  --no-system-fonts --font-file /path/to/font.ttf
```

No fonts are downloaded or redistributed automatically. Use the same font files
when reproducing results; equal family names do not guarantee identical versions.

| Condition | Default behavior |
|---|---|
| Requested family unavailable | Fail before creating output |
| Requested weight or italic style unavailable | Fail before creating output |
| Explicit family mapping | Requires `--font-fallback` |
| Individual glyph missing, another font contains it | Continue with a warning |
| No loaded font contains the glyph | Fail even with `--font-fallback` |
| Any warning with `--strict` | Fail before creating output |

`--font-fallback` permits family substitution and style approximation. An explicit
mapping such as `--font-map 'Meiryo UI=Noto Sans CJK JP'` also requires that flag.
Mappings are repeatable, last-wins for the same requested name, and applied once;
cycles do not trigger recursive lookup. If a mapped target is unavailable,
diagnostics record it and selection returns to automatic fallback.

Selection considers character coverage, italic match, weight distance, then
family priority: Noto Sans CJK JP, Noto Sans JP, Yu Gothic, Hiragino Sans, Meiryo,
Arial Unicode MS, Arial, then other families. Ties use PostScript name, family
name, and collection face index. Coverage matters within a requested family too.
Measurement and rendering use the same face; fallback can still change widths
and line heights relative to the original font.

## CLI result documents

Success writes one YAML document to stdout. Failure writes one YAML document to
stderr. Successful conversions also write human-readable warnings to stderr;
do not merge the streams when parsing YAML. `--help` and `--version` retain their
usual text format. Exit codes are 0 for success, 1 for conversion failure, and 2
for invalid arguments.

```yaml
status: completed
output:
  created: true
  folder: images
  slides: 20
warnings:
  - code: FONT_GLYPH_MISSING
    family: Arial
    substitute_family: Hiragino Sans
    slides: [13, 15, 16, 17]
    characters:
      - character: "①"
        codepoint: U+2460
```

The optional `warnings` array summarizes successful glyph fallback by requested
and replacement family. Other rendering warnings remain on stderr and in the
manifest. Actual replacement families depend on the loaded fonts.

```yaml
status: failed
output:
  created: false
errors:
  - code: FONT_NOT_FOUND
    family: Calibri
    slides: [6, 14]
    suggestion: "Add the required font with --font-file <path>. If unavailable, ask the requester to provide it."
```

`errors` groups font problems by code and family, listing distinct slide numbers.
`FONT_GLYPH_MISSING` includes `characters` with `character` and `codepoint`.
Other codes are `FONT_STYLE_MISSING`, `FONT_MAPPING_TARGET_NOT_FOUND`,
`FONT_SUBSTITUTION_REQUIRED`, `STRICT_MODE_REJECTED`, `CONVERSION_FAILED`,
`INVALID_ARGUMENTS`, and, for XLSX input, `FORMAT_NOT_SUPPORTED` and
`OPTION_NOT_SUPPORTED` (exit code 2). Missing-font errors do not recommend bypassing the problem
with fallback. Treat codes and fields as structured data, not message text.

`output.created` says whether this invocation created the output directory.
A write failure may return `status: failed` with `created: true` and `folder`.
Refusing an existing directory returns `created: false`. Dynamic strings are
quoted and escaped, including newlines and punctuation in paths and font names.

## Manifest diagnostics

The manifest uses `schema_version: 3`. Version 3 adds `include_hidden`,
`skipped_hidden_slides` (deck positions of hidden slides that were not rendered), and
`slides[].hidden`; consumers that pair images with another export should use
`slides[].slide` rather than the array index. `slides[].warnings` contains deduplicated
messages; `slides[].diagnostics` retains separate records for different targets,
even when their warning text is the same. The slide number is on the parent entry.

```json
{
  "code": "omitted",
  "part": "ppt/slides/slide1.xml",
  "object_id": "42",
  "object_name": "Chart",
  "location": "spTree/0/2",
  "message": "Omitted unsupported element: non-table graphicFrame"
}
```

Categories are `unsupported`, `omitted`, `approximation`, `font_substitution`,
`missing_glyph`, and `text_overflow`. Missing object IDs or names are `null`.
Table locations include row and cell indices.

`fonts` records family, PostScript name, and collection face index.
`slides[].diagnostics[].font` records requested and actual families, PostScript
name, face index, and reason. Reasons are `family_missing`, `explicit_mapping`,
`mapped_family_missing`, `glyph_fallback`, and `style_approximation`.
`required_font` and `missing_character` appear where applicable. Use part,
object, location, and slide metadata to identify affected content.

Warnings do not guarantee exhaustive detection. Text currently always produces
an approximation warning; strict mode is not a PowerPoint compatibility test.

## Rust library

The package also provides a library crate. This local-path example does not
assume a crates.io release:

```toml
[dependencies]
rhodopsin = { path = "../rhodopsin" }
```

```rust,no_run
use rhodopsin::{convert, ConvertOptions, FontFallbackError, StrictModeError};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = ConvertOptions { width: 1600, ..Default::default() };
    match convert("slides.pptx", "images", &options) {
        Ok(report) => {
            for slide in &report.slides {
                for warning in &slide.warnings {
                    eprintln!("Slide {}: {warning}", slide.slide);
                }
            }
        }
        Err(error) => {
            if let Some(fonts) = error.downcast_ref::<FontFallbackError>() {
                eprintln!("Font requirements: {:?}", fonts.slides);
            }
            if let Some(strict) = error.downcast_ref::<StrictModeError>() {
                eprintln!("Strict-mode diagnostics: {:?}", strict.slides);
            }
            return Err(error.into());
        }
    }
    Ok(())
}
```

`convert` is synchronous. It writes PNG files and a manifest and returns the same
content as `ConversionReport`. The library writes no messages to stdout/stderr.
Inspect `report.slides` and the public `DiagnosticCode` enum for diagnostics.
`ConvertOptions.font_fallback` defaults to `false`; `font_mapping` holds mappings.

`StrictModeError.slides` and `FontFallbackError.slides` describe pre-output
failures; their `image` values are planned filenames, not existing files.
Other errors also use `anyhow::Error`. The public API may change during this early
implementation stage.
