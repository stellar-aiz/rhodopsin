# rhodopsin - No more LibreOffice for AI

Convert PPTX slides to PNG or searchable PDF with a standalone CLI or Rust library.

**😎 No Office, LibreOffice, browser, Python, or Node runtime required.**

Early implementation: rendering is approximate, not fully PowerPoint-compatible.

## Quick start

```
rhodopsin <INPUT> [--format png|pdf] [--out PATH] [--width N] [...]
```

Download a prebuilt binary from the [releases page](https://github.com/stellar-aiz/rhodopsin/releases)
(pre-release, unsigned; see [installing from a release](docs/usage.md#install-from-a-release)),
or install from this checkout with Rust and Cargo (to call it from Rust instead, see the
[library guide](docs/usage.md#rust-library)):

```sh
cargo install --path . --locked
rhodopsin slides.pptx
rhodopsin --help
```

Writes `slides.rhodopsin/slide-001.png`, etc., and `manifest.json`.
Use `--out images` to choose a destination; existing directories are never overwritten.

For one PDF per deck, use `rhodopsin slides.pptx --format pdf --out slides.pdf`.
PDF output preserves text and supported vector shapes using the existing rendering
dependencies. Its initial [supported subset](docs/rendering.md#pdf-output) is narrower
than PNG; unsupported PDF features fail explicitly. Existing files are never replaced.

## XLSX to PDF (under development)

`.xlsx` input is accepted by the same CLI and converted to PDF only:

```sh
rhodopsin book.xlsx --format pdf
```

The spreadsheet renderer (`src/xlsx`) converts workbooks inside the layout measured against Excel for Mac (cells in Noto Sans JP, 6-40 pt, single-line text); anything else stops with a located diagnostic instead of an approximated PDF.
See [XLSX input](docs/usage.md#xlsx-input) and [docs/xlsx.md](docs/xlsx.md).

## Fonts and warnings

- Supply missing fonts with `--font-file /path/to/font.ttf` (repeatable).
- Font-family substitution requires `--font-fallback`.
- Individual missing glyphs use available fallback fonts and produce warnings.
  If no font can render a character, conversion fails.
- `--strict` stops on any warning before writing output.

Success returns YAML on stdout; failures return YAML on stderr.
Warnings and rendering diagnostics are retained in the PNG manifest or PDF result
document on stdout.

## License

Copyright (c) 2026 Stellar AIz and the rhodopsin contributors.

Both crates are released under the [MIT License](LICENSE). Third-party
dependencies keep their own permissive licenses; each release ships a NOTICE file
with their attributions. See [the decision record](docs/history/20261030_licence.md)
for the reasoning.

[CLI, fonts, and Rust API](docs/usage.md) ·
[Supported features and limitations](docs/rendering.md) ·
[Development and provenance](docs/development.md) ·
[Design charter](docs/constitution.md) ·
[Decision records](docs/history/)

### Experimental PDF input

Convert the supported basic PDF subset to PNG with
`rhodopsin input.pdf --out pages --width 1600`. The implementation is pure Rust
and rejects unsupported content explicitly. See [PDF input](docs/pdf-input.md)
for the supported subset, limits, API, and known image-interpolation differences.
