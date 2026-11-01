---
kind: guide
---

# Development and provenance

## Architecture

One crate, `rhodopsin`, converts PPTX and (behind the default `xlsx` feature) XLSX; the spreadsheet code is the `src/xlsx/` module ([XLSX](xlsx.md), [record](history/20261101_xlsx-single-crate.md)). `cargo build --lib --no-default-features` builds the library without the spreadsheet dependencies.

```text
PPTX (ZIP/XML)
  -> OOXML parsing, slide order, transforms, text layout   src/pptx.rs, src/pptx/
  -> internal SVG scene                                    resvg / usvg (via resvg) / skrifa
  -> PNG + manifest.json                                   src/lib.rs
  -> or PDF (Type 3 text, same scene)                      src/pdf.rs, src/pdf_output.rs
CLI: argument parsing src/main.rs; YAML result documents src/cli_output.rs; .xlsx routing src/cli_xlsx.rs
```

The rendering and font dependencies are `resvg` (which brings usvg and tiny-skia) and `skrifa` on the PPTX side, and pdf-writer, subsetter, rustybuzz (which brings ttf-parser), and image in the xlsx crate; `Cargo.toml` lists the rest. Independent implementation does not mean avoiding libraries; their licences and attribution apply. The project's public licence is MIT for both crates ([record](history/20261030_licence.md)); the `license` fields in both `Cargo.toml` files and the two `LICENSE` files must agree with it. Releases also ship a NOTICE file with the third-party attributions.

[ibis-office](https://github.com/ibisjp/ibis-office) inspired the idea of an Office-independent renderer; its public README and crate entry points were consulted, and its code, tests, and assets were not copied, ported, or translated. The rules for this are in [AGENTS.md](../AGENTS.md). Keep private inputs and experimental output in the ignored `playground/`.

## Tests

```sh
cargo fmt --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
```

Tests use ZIP/XML inputs, PNG pixels, and generated fonts to check slide order, images, output protection, malformed archives, strict mode, inheritance, font selection, layout, and diagnostics. `examples/basic.pptx` and the other `examples/*.pptx` are synthetic. Generated rectangle fonts (`tests/fixtures/metrics.ttf`, `tests/xlsx_support/`) are structural fixtures, not evidence of glyph fidelity. No Office or Python runtime is needed for ordinary Rust tests.

CI (`.github/workflows/ci.yml`) runs on ubuntu-24.04, macos-15, and windows-2025 with a pinned Rust 1.97.0 toolchain: the three commands above, a release build with `cargo test --release --locked --test pdf`, and the Python unit tests in `tests/visual` and `tools`. It runs once per change: on pull requests and on pushes to `main`, skipping Markdown-only changes; a new push to a pull-request branch cancels the superseded run, and runs on `main` are never cancelled. Cargo and pip dependencies are cached. The scheduled/manual visual workflow (`.github/workflows/visual.yml`, Sundays and `workflow_dispatch`) uses LibreOffice 26.2.5.2 with a verified download SHA-256 and checks the Hiragino Sans W3/W6 hashes before comparing; host OS images are labels, not pins, so environment information is retained in reports. Do not compensate for environment differences by relaxing thresholds.

## Visual comparisons

LibreOffice is a development reference only, never a CLI runtime dependency. Comparison tests are `#[ignore]` by default.

```sh
python3 -m venv .venv-visual
.venv-visual/bin/python -m pip install -r tests/visual/requirements.txt
# Install LibreOffice separately; use PATH or SOFFICE to select its executable.
RHODOPSIN_TEST_PYTHON="$PWD/.venv-visual/bin/python" \
  cargo test --test libreoffice -- --ignored --nocapture
.venv-visual/bin/python -m unittest discover -s tests/visual -p 'test_*.py'
```

On Windows use `.venv-visual/Scripts/python.exe`. Recorded comparisons were performed on macOS; Windows/Linux visual equivalence has not been established. Each comparison uses a new temporary LibreOffice profile to produce a PDF, which PDFium rasterizes; on macOS `SAL_USE_VCLPLUGIN=osx` keeps CoreText font resolution. No user profile or Office automation is used.

```sh
cargo build --release --locked
.venv-visual/bin/python tests/visual/compare.py \
  --binary target/release/rhodopsin --fixture examples/basic.pptx \
  --out target/visual-tests/basic-001
```

Artifacts: input PPTX, commands, stdout/stderr, `actual/`, `reference/` (PDF and PNG), `diff/` (red differences on black), and `report.json`; failures retain evidence, and each run needs a new output directory. Default thresholds: normalized RGB MAE <= 0.02 and changed-pixel ratio <= 0.05, where a changed pixel differs in any channel by more than 16; default width 1200 px. Only a 1 px rasterization-rounding discrepancy is corrected; page-count and aspect-ratio mismatches fail. Exit codes: 0 within thresholds, 1 exceeded, 2 execution error. Threshold overrides are recorded in the report and never used to hide a mismatch. Regions, font pinning, the per-OS CI legs, and every fixture generator are described in the [visual comparison guide](visual-comparison.md). LibreOffice can differ from PowerPoint too; passing thresholds is not proof of display compatibility, and same-font PowerPoint reference comparisons remain future work.

## Spreadsheet module

`src/xlsx/` was imported from the former xlsx2pdf repository as a snapshot without history ([record](history/20261022_xlsx-import.md)) and later merged into this crate ([record](history/20261101_xlsx-single-crate.md)). The CLI routes `.xlsx` input to it; its state, design rules, and open items are in [xlsx.md](xlsx.md). Its tests are `tests/xlsx_*.rs`, its probes and comparators the `examples/xlsx-*.rs` examples, and its Python tools (SBOM/NOTICE, distribution bundle, document comparison) are in `tools/`:

```sh
cargo test --locked --test xlsx_cli
python3 -m unittest discover -s tools -p 'test_*.py'
```

## Implementation references

- [Slide masters](https://learn.microsoft.com/en-us/office/open-xml/presentation/working-with-slide-masters)
- [Group transforms](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.transformgroup)
- [usvg 0.48.1 FontResolver](https://docs.rs/usvg/0.48.1/usvg/struct.FontResolver.html)
- [GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)

Decisions and their reasons, and the measurements that motivated them, are in [docs/history](history/).
