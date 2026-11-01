# Software Design Charter

rhodopsin converts PPTX slides into PNG images or a PDF, and is gaining XLSX-to-PDF conversion through its `rhodopsin-xlsx` crate (under development), according to four design principles: lightweightness, fidelity, transparency, and observability. Every design decision is derived from these four; the records under [docs/history](history/) are their applications, and when a document and the code disagree, the code is right. Development procedures are in [AGENTS.md](../AGENTS.md) and the [development guide](development.md).

## Lightweightness

Keep resource consumption and runtime dependencies small. Conversion runs standalone: no Office, LibreOffice, browser, Python, Node, network, or child process at runtime. Third-party Rust libraries are compatible with this principle; their runtime cost, deployment requirements, and licences are what matter.

- LibreOffice and Excel are development-time comparison references only; the comparators live under `tests/visual/`, `tools/`, and the `examples/xlsx-*` comparators, never in the binary.
- Input and output are bounded by explicit limits per layer (archive size, parts, slides, images, PDF objects), and an oversized picture degrades per picture rather than failing the document.
- A dependency is adopted only when it covers the needed subset; otherwise the subset is implemented from the specification (see [the EMF/WMF survey](history/20261020_emf-wmf-survey.md)).

Canonical code: `Cargo.toml`, `src/pptx.rs` and `src/pptx/raster.rs` (limits), `tests/libreoffice.rs` (`#[ignore]` comparisons).

## Fidelity

Reproduce the original document's content, appearance, and layout as faithfully as possible: text, shapes, their placement, and the formatting the file expresses.

- The authoring application's saved state is trusted over live recomputation: saved connector routes, saved autofit ratios, saved date-field text, cached formula results.
- Measurement follows the specification and observed behaviour of the authoring application, not of the comparison reference; matching LibreOffice is not the goal and is not proof.
- Thresholds are never relaxed to hide a difference. A known difference stays reproducible, with its fixture, until it is fixed.
- When Excel's rule is unknown (border precedence, underline geometry, General rounding), the renderer stops instead of guessing.

Canonical code: `src/pptx/geometry/`, `src/pptx/text.rs`, `tests/visual/compare.py` (fixed thresholds), `src/xlsx/format/`, `src/xlsx/page_borders.rs`.

## Transparency

Make substitutions, approximations, omissions, and unsupported features visible to the user, alongside the output, so they can judge whether the result is fit for purpose.

- Every approximation or omission is a diagnostic (`unsupported`, `omitted`, `approximation`, `font_substitution`, `missing_glyph`, `text_overflow`); `--strict` turns any of them into a failure before output is written.
- Hiding is silent only when it is the document's own instruction (hidden shapes, hidden slides); an executable or external object that is not run keeps its diagnostic even when hidden.
- Diagnostics never carry document content: no cell values, sheet names, paths, or raw library errors.
- Nothing is described as verified against Office when it was not.

Canonical code: `src/lib.rs` (`DiagnosticCode`), `src/cli_output.rs` and `src/cli_xlsx.rs` (result documents), `src/xlsx/diagnostic.rs`.

## Observability

Make rendering decisions and their output traceable, with enough diagnostic information for people and agents to find the cause of a difference, correct it, and verify the correction. Observability enables autonomous improvement; it does not mean the renderer modifies itself.

- Diagnostics name the part, object, and location; the manifest and the PDF result document carry the fonts actually used.
- Comparison reports record the environment (renderer, reference, fonts and their hashes, thresholds), and recording an environment does not pin it.
- Regression checks go below whole-page metrics when those hid a defect (line-level and ink-distribution checks for text).

Canonical code: `src/lib.rs` (manifest schema), `tests/visual/compare.py`, `tests/visual/check_environment.py`, `src/xlsx/report.rs`.
