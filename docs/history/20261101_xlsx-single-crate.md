---
date: 2026-11-01
kind: decision
principles: [lightweightness, transparency]
---

# Merge the spreadsheet crate into the rhodopsin crate

**Date**: 2026-11-01
**Status**: Decided by the owner; applied in the same change.

## Decision

- There is no separate `rhodopsin-xlsx` crate. The spreadsheet code is the `rhodopsin::xlsx` module (`src/xlsx/`), its tests are `tests/xlsx_*.rs` with `tests/xlsx_support/` and `tests/fixtures/xlsx/`, its development probes are the `examples/xlsx-*.rs` examples, and its Python tools moved to `tools/`. The package is a single-package workspace.
- The module sits behind an `xlsx` feature that is on by default and pulls in its own dependencies (image, pdf-writer, rustybuzz, subsetter, tempfile, the unicode crates, rustix on Unix). PPTX-only library users can build with `--no-default-features`. The binary requires the feature.
- The public API of `rhodopsin::xlsx` is `convert`, `convert_with_options`, `convert_with_policy`, `prepare`, `ConversionOptions`, `PreparedWorkbook`, and the `diagnostic`, `font_sources`, `package`, and `report` modules. The other modules stay `pub` only so the integration tests and probes can reach them, and are `#[doc(hidden)]`; this settles the public-surface question of #118 for the spreadsheet code.
- This supersedes the provisional crate name and the workspace layout in the [import record](20261022_xlsx-import.md); its other decisions (snapshot import, single CLI and result format, convergence order) stand. Paths in older records refer to the layout of their date.

## Why

- The owner does not want a separate crate: one CLI with one contract was already the goal of the import, and a second crate only added a second manifest, licence file, and package identity to keep in step (the licence and SBOM work of #113 and the CLI embedding of #125 both had to special-case it).
- A feature keeps the lightweightness principle without a second crate: what a library user does not need is not compiled.
- Hiding rather than privatising the internal modules keeps all 563 tests unchanged in this move; making them private would require moving the integration tests into the modules, a larger diff with no behaviour change.

Canonical code: `Cargo.toml` (`[features]`), `src/lib.rs`, `src/xlsx/mod.rs`.
