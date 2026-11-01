---
kind: guide
---

# Rendering support and limitations

This is an early, approximate renderer, not a complete PowerPoint implementation. Unsupported content is reported where detected; detection itself is incomplete. Strict mode gates known warnings, not all incompatibilities. Measurements against LibreOffice that motivated individual rules are kept in git history and in [docs/history](history/); this page states the rules as they stand.

## PDF output

`--format pdf` uses the same usvg scene, font faces, and glyph positions as PNG output (`src/pdf.rs`, `src/pdf_output.rs`). No PDF library is used. The backend writes PDF 1.4 with Type 3 outline fonts, ToUnicode maps, and ActualText, so the text is real PDF text rather than a bitmap or an embedded OpenType subset. Font hinting is not preserved, and extraction order, whitespace, and punctuation can differ across readers.

Supported as vectors: solid fills and strokes with any opacity (`ExtGState` `/ca` and `/CA`), linear and radial gradients (`ShadingType` 2 and 3 with stitched functions), group opacity, blending, and isolation (transparency-group form XObjects with `/BM`), curves, ordinary caps/joins/dashes, transforms, single-path clipping, outline text with its own fill opacity, and PNG (with an alpha soft mask) and JPEG images. Tables and charts are their rendered paths and text.

Rasterized with an `approximation` diagnostic (the slide's diagnostics carry the scene location under `pdf/`): filters, masks, pattern fills, gradients with translucent stops, and text filled with a gradient or pattern. The element is drawn by resvg at 2 pixels per CSS pixel (192 dpi) into a Flate-compressed RGB image with an alpha soft mask. A filter that ends by merging its result under the source graphic (outer shadows, `src/pptx/effects.rs`) is split: the raster holds only the filter result (the source is rendered separately and removed from the composite), and the source is drawn as vectors on top, so a shadowed shape's text stays text and translucent shapes are not composited twice. Any other filter, a filter combined with group opacity or blending, and any mask is shown by the raster alone, with the group's blend mode applied to the raster; text inside it is not extractable. Masks are not written as PDF soft masks. A raster that would exceed the image pixel limit even at 1 pixel per CSS pixel, or that would take a document past four limit-sized rasters in total, is omitted with an `omitted` diagnostic and nothing is drawn for that element. Gradient spread methods other than pad are drawn as pad with an approximation diagnostic. `--strict` fails on all of these before any PDF is published.

The following stop conversion with a slide-level error before any PDF is published: nested or multiple clipping paths, miter-clip joins, text strokes and decorations, variable/color/bitmap fonts, non-outline glyphs, unhandled glyph clusters, and embedded SVG/GIF/WebP image nodes. Fonts whose OS/2 flags forbid embedding or subsetting, or that are bitmap-only, are rejected, as are fonts without embedding metadata. These checks do not replace the user's responsibility to hold a licence for the supplied fonts.

Known difference, kept reproducible: fine hatch patterns are resolution dependent once rasterized, so `examples/fills.pptx` page 1 differs from the PNG output pixel by pixel (MAE 0.164, 40% changed at 1600 px; MAE 0.004 and 0% after a 3 px blur of both). The other example decks with effects match the PNG output within the comparison thresholds (`examples/image-effects.pptx` at most MAE 0.0046 with 1.8% changed).

Limits: 200-inch page dimensions, 16,777,216 decoded pixels per image, 100,000 PDF objects, and 128 MiB of accumulated content. Images and rasters are Flate-compressed; content streams are not, and a font resource is cached per face/glyph/Unicode combination, so large decks can reach the limit. Slides are converted sequentially, but PDF resources and final bytes are held in memory. Failures name the affected slide and tree node or feature.

`tests/visual/pdf_output.py` generates a Japanese text/table/PNG/JPEG deck and compares PDFium rasterization with the PNG output under the unchanged thresholds. It does not validate PowerPoint compatibility or any particular consumer's text extraction.

## Supported subset

- Transitional OOXML PPTX, using presentation-defined slide order.
- Explicit positions, sizes, rotation, and flips for basic shapes and the [presets and connectors](preset-shapes.md) evaluated from DrawingML shape guides, and [freeform shapes](#freeform-shapes) (`a:custGeom`).
- Solid backgrounds, fills, and lines using RGB, system `lastClr`, and theme colors; the [color transforms and theme fills](colors.md) listed there, including translucent (`alpha`) fills, lines, and text; linear and radial [gradient fills](colors.md#gradient-fills); theme background references, approximate pattern fills, stretched and tiled picture fills, and group fills ([details](colors.md#theme-backgrounds-pattern-picture-and-group-fills)).
- Slide -> layout -> master -> theme relationships, color mappings, and major/minor Latin and Japanese theme fonts, including the Jpan supplemental font.
- Explicit font, size, color, bold/italic, and left/center/right paragraph alignment.
- Rotated [vertical text](#vertical-text) (`vert`, `vert270`) and upright East Asian columns (`eaVert`).
- Nominal font-advance wrapping, basic Japanese prohibited breaks, explicit line breaks and empty lines, paragraph spacing, point/percentage line spacing, and top/center/bottom vertical anchoring with overflow warnings.
- `hangingPunct` inherited from paragraph, list, master, and presentation defaults; one trailing punctuation character (`、。，．`) may hang outside the box.
- Explicit character spacing `spc` (negative, positive, inherited, explicit zero) and optional automatic punctuation compression (`--punctuation-compression`, experimental; see [text measurement](history/20261015_text-measurement.md)).
- Property-by-property inheritance from runs, paragraph defaults, and list levels.
- Embedded PNG/JPEG placement and `srcRect` cropping, and EMF and placeable WMF pictures translated to SVG ([subset and limits](#emf-and-wmf-pictures)).
- Basic placeholder inheritance and shared master/layout shapes and solid backgrounds.
- Nested group transforms, explicit table cell formatting, rectangular merged cells, [table styles](table-styles.md), [outer shadows](shadows.md), and [static OLE previews](ole-preview.md).

Connectors use their saved geometry and transforms even when `stCxn`/`endCxn` references are present. Live rerouting is not evaluated; one `approximation` note per slide retains the first affected connector target. See [saved bent routes](preset-shapes.md#saved-bent-routes-and-connection-sites) for the policy and its evidence.

## Inheritance

Placeholders match by `idx` from slide to layout and by type from layout to master. Template sample text is not rendered; inherited properties supplement slide text, and explicit slide properties win. `showMasterSp="0"` hides shared ancestor shapes. This covers transforms, solid fills, and basic text styles, not every property.

Ordinary text boxes (`txBox=1`) use `presentation/defaultTextStyle`; placeholders inherit layout/master styles; text inside shapes uses `otherStyle`. Run, paragraph, and list values override defaults. Empty paragraphs use the size and font metrics in `endParaRPr`. Fixture: `examples/text-inheritance.pptx`.

## Line height

A run's natural line height follows one priority order, keyed on what is known about the requested family, and the result records it per requested family as `line_heights[].basis` (see [manifest diagnostics](usage.md#manifest-diagnostics)):

| Requested family | Natural line height | `basis` |
|---|---|---|
| Present, and one of the families measured in PowerPoint: Meiryo UI, Yu Gothic, MS PGothic | 1.2 em | `powerpoint` |
| Present, any other family | the font's own `usWin` cell (typographic or `hhea` metrics when OS/2 is absent) | `font` |
| Absent; with `--font-fallback` a face of another family stands in (automatic selection or `--font-map`) | 1.2 em | `estimate` |

The 1.2 em line is split into ascent and descent by the drawn face's `usWinAscent : usWinDescent` ratio, with no extra leading. A family counts as measured when the selected face exposes one of the three names, so Japanese requests such as `游ゴシック` or `ＭＳ Ｐゴシック` match too, as do other weights of those families (only the regular weight was measured); `Meiryo`, `Yu Gothic UI`, and `MS Gothic` are other families. Percentage spacing, `lnSpcReduction`, and fixed-point spacing apply on top as for any font. A face reached through per-glyph fallback keeps its own cell height.

`font` means PowerPoint's line height for the family has not been observed: the run is laid out with the font's metrics, which can differ from PowerPoint (Calibri's own cell is 1.22 em). `estimate` means the requested font's metrics are unknown, and PowerPoint's measured 1.2 em stands in for them; PowerPoint's own missing-font fallback has not been observed (PowerPoint for Mac bundles these families), and the rule applies to every script. Neither is a diagnostic, so `--strict` does not stop on them; counting `font` families over a corpus shows which families to measure next.

Measurements on the fixture `tests/visual/make_cjk_metrics.py` (five lines per box at 12, 18, and 24 pt, default spacing), 2026-11-01:

| Source | Face | Line pitch | First baseline below the text top |
|---|---|---|---|
| PowerPoint for Mac 16.105.3, PDF export | Meiryo UI, Yu Gothic, MS PGothic (real) | 1.19 to 1.21 em, about 1.2 em on average for all three | 1.00, 0.92, 1.00 to 1.05 em |
| LibreOffice 26.2.5.2 | Hiragino Sans W3 (pinned) | 1.200 em | not measured |
| This renderer, substituted | Hiragino Sans W4 | 1.200 em (1.257 em before #89) | 0.969 em (1.015 em before) |
| This renderer, real fonts supplied | Meiryo UI, Yu Gothic, MS PGothic | 1.200 em (1.270, 1.287, 1.000 em `usWin` before #133) | 1.00, 0.92, 1.03 em (`usWinAscent` before) |

PowerPoint's pitch is about 1.2 em although the three fonts' `usWin` heights are 1.270, 1.287, and 1.000 em, and 1.2 em times each font's `usWinAscent` share predicts its first baseline (1.00, 0.92, 1.03 em). PowerPoint's baselines are quantized in about 1.2 pt steps, so single intervals alternate; the renderer does not quantize.

### Substituted faces

The substituted face is best-effort. Selection is unchanged (coverage, style, weight nearest the request, then the family priority in [usage](usage.md#font-discovery-and-fallback)), so regular text on macOS gets `HiraginoSans-W4`, while LibreOffice embeds W3. Neither weight is consistently closer to the originals: kanji ink coverage relative to the real font is Meiryo UI +8% (W4) / −9% (W3), Yu Gothic +65% / +40%, and MS PGothic −2% / −17%. The chosen face is named in the `font_substitution` diagnostic (`post_script_name`), and `--strict` stops on it. A `--font-map` target must be a family name the font database exposes; Hiragino's weight-specific names such as "Hiragino Sans W3" are not, so W3 cannot be selected explicitly.

## Theme style references

A shape style (`p:style`) supplies theme defaults that explicit properties anywhere in the inheritance chain override: `fillRef` selects a `fillStyleLst` entry (`bgFillStyleLst` for `bgRef` and indices above 1000), `lnRef` a `lnStyleLst` entry, `effectRef` an `effectStyleLst` entry, and `fontRef` a theme font with a text colour. The reference's colour replaces `phClr` in the entry, keeping the entry's colour transforms, and the result goes through the ordinary fill, line, effect, and text code, so gradient and pattern theme fills and theme shadows render like explicit ones. `fontRef` ranks above the master text styles and presentation defaults and below the shape's own run, paragraph, and list properties: PowerPoint shows a default shape's text in the style's `lt1` colour even though the master's `otherStyle` names `tx1`. Placeholders keep their inherited master styles above the reference (only ordinary shapes were measured). Its theme font (`+mn`/`+mj`) is requested when no `latin` typeface is set; the theme's East Asian font only when the theme declares one (an `ea` typeface or a `Jpan` script font). Index 0 (and 1000 for fills) means none; an unresolved index, a `blipFill` theme entry, effect styles other than outer shadows, and 3-D members of an effect style (`scene3d`, `sp3d`) are reported. Fixture: `examples/theme-style.pptx` (`tests/visual/make_theme_style.py`), within the comparison thresholds against LibreOffice (MAE 0.008, 2.3% changed).

## Saved autofit

`normAutofit/fontScale` (default 100%) applies to measurement, wrapping, and drawing of all runs. `lnSpcReduction` (default 0%) subtracts from percentage line spacing (120% - 20% = 100%); fixed-point line spacing is unchanged. Child `noAutofit` cancels inherited autofit. The renderer never searches for a new shrink ratio.

`spAutoFit` expands height for fixed-width horizontal text, preserving anchoring and margins. Table-cell and vertical autofit are unsupported and warned about. When the body also has `wrap="none"`, the shape width becomes the widest line plus insets and paragraph margins, so outline, fill, and effects follow the text; an oversized saved width shrinks too. The edge on the first paragraph's alignment side stays fixed (left for left/justified, centre for `ctr`, right for `r`), measured along the text's own direction so rotation and flips keep the edge on the alignment side as the text reads. Bodies without visible text keep their saved width. This follows the Office interpretation of `spAutoFit`; LibreOffice keeps saved widths and wraps instead, so the fixture page for this rule is a known difference. Fixture: `examples/text-autofit.pptx` (`tests/visual/make_text_autofit.py`).

- [NormalAutoFit](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.drawing.normalautofit)
- [Office spAutoFit interpretation](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/19d31037-28bf-4693-85be-3a37bfe82d51)

## Vertical text

`a:bodyPr@vert="vert"` and `"vert270"` are rotated horizontal text: lines are laid out with the box's width and height swapped, then turned 90 degrees clockwise (`vert`) or counter-clockwise (`vert270`). Wrapping, alignment, anchoring, spacing, and insets follow the turned lines, so `algn="l"` starts at the top for `vert` and at the bottom for `vert270`. Shape autofit is unsupported for vertical bodies and reported.

`eaVert` lays the body out as clockwise lines, so columns run top to bottom and from right to left, then turns East Asian characters (the CJK, kana, and full-width ranges) back upright. An upright character advances along the column by the font's `vmtx` advance, or by the em size when the font has none, which adds a `Font has no vertical metrics` approximation. Other characters, including ASCII letters and digits, stay turned with the line; there is no tate-chu-yoko. The font's vertical forms (OpenType `vert`/`vrt2`) are not applied: brackets, dashes, and the long vowel mark are drawn turned, and `、。，．` are moved to the upper right of their cell, which every `eaVert` body reports as an approximation. Character spacing continues across a change between upright and turned characters. Table-cell vertical text (`a:tcPr@vert`), `wordArtVert`, `wordArtVertRtl`, and `mongolianVert` are laid out horizontally with an unsupported diagnostic. Fixture: `examples/vertical-text.pptx` (`tests/visual/make_vertical_text.py`).

Against LibreOffice 26.2.5.2 at width 1600 with the default thresholds (2026-11-01), the `eaVert` page measures MAE 0.0180 and 4.08% changed (0.0605 and 8.55% when it was laid out horizontally), and the rotated page 0.0063 and 1.49%. The `eaVert` result depends on the 1.2 em pitch of [substituted fonts](#line-height); the remaining differences are glyph forms of punctuation and the Hiragino weight. PowerPoint's rendering of this fixture has not been measured.

## Table row heights

`a:tr/@h` is a minimum. Each cell's text is laid out at its merged width with its margins (`marT`/`marB`, default 0.05in), and a row grows to `max(stored h, margins + content)`. A row-spanning cell adds any shortfall to its last spanned row. Rows stack from the top of the frame; the frame height does not stretch or squash them, so a grown table can extend below its frame (LibreOffice does the same). Columns still scale to the frame width. A zero-height row is ordinary input: it grows to its text and emits no diagnostic. Fixtures: `examples/table-row-growth.pptx`, `examples/table-zero-row-height.pptx`.

## Bullets and indentation

`buChar` and `buAutoNum` are placed separately from text at `marL+indent`; body and wrapped lines start at `marL`; `marR` sets the right margin. Paragraph/list/master settings, bullet font/size/color, and `buNone` are supported, and bullet fonts resolve like body text. Automatic numbering supports Arabic, alphabetic, and Roman sequences with period, closing parenthesis, paired parentheses, or no decoration; counters are per level; a direct `startAt` restarts, an inherited one applies only at the first entry. Picture bullets are unsupported; other numbering formats and non-left bullet positions produce `approximation` warnings.

A paragraph without a text run (`a:r` or `a:fld`) draws no bullet marker, because Office shows none; the empty line keeps its height and still advances the numbering counter. This is normal input and emits no diagnostic (PowerPoint observation recorded in issue #53). Fixtures: `examples/text-bullets.pptx`, `examples/empty-bullets.pptx`.

## Hidden slides

`p:sld/@show` is read for every slide. Hidden slides are not rendered unless `--include-hidden` is passed, and the manifest records them (`hidden`, `skipped_hidden_slides`). LibreOffice's PDF export also skips them, so comparison pages pair one to one. See [usage](usage.md#hidden-slides).

## Text fields

`a:fld` runs are resolved by `@type` during layout, including text boxes that a layout or master draws onto a slide.

| Field type | Drawn text | Diagnostic |
|---|---|---|
| `slidenum` | The slide's 1-based position in `p:sldIdLst` plus `firstSlideNum` minus 1; hidden slides count even when skipped | none |
| `datetime*` | The saved text, so output is deterministic | one `approximation` per slide |
| `title`, `footer`, other or missing `@type` | The saved text | one `approximation` per slide and type |

Numbers are never formatted as roman numerals or letters, since `a:fld` carries no number format. Generator: `tests/visual/make_fields.py`.

## Hidden elements

Shapes, pictures, connectors, groups, and non-OLE graphic frames with `p:cNvPr/@hidden` are not drawn, on slides, layouts, and masters. A hidden group hides its children; a hidden child inside a visible group hides only itself. This is silent, because hiding is normal in real decks. A hidden OLE object is the exception and keeps its `omitted` diagnostic; see [OLE previews](ole-preview.md).

## Shape property extensions

`a:extLst` in shape properties is inspected element by element. The Office 2010 extensions `a14:hiddenFill`, `a14:hiddenLine`, `a14:hiddenEffects`, `a14:hiddenScene3d`, and `a14:hiddenSp3d` only remember settings a shape had before they were switched off, so they are ignored by design without a diagnostic. Any other extension is reported once per shape as `Unsupported shape property extension: <prefix:name>`.

## Flipped shapes and groups

`flipH`/`flipV` on a shape or any ancestor group mirror the geometry (outline, fill, text rectangle, pictures) but never the text. When the net transform is a mirror, the text is mirrored once more around the centre of its final text rectangle, so `flipH` text reads normally, `flipV` text is turned 180 degrees, and `flipH`+`flipV` is a plain rotation. Table cells follow the same rule, so a table inside a flipped group stays readable. LibreOffice does not mirror such a table at all, so that case has no matching reference and is covered by `tests/flips.rs` instead. Fixture: `examples/flipped-text.pptx` with regions `examples/flipped-text.json`.

## Freeform shapes

Custom geometry (`a:custGeom`) uses the same guide evaluator as the presets. Guides in `a:avLst` and `a:gdLst` are evaluated in document order in shape coordinates (EMU) and may use built-in guides and earlier guides. Each `a:path` is scaled from its own `w`/`h` space to the shape extent; a missing or zero `w`/`h` means shape coordinates on that axis. Supported commands: `moveTo`, `lnTo`, `cubicBezTo`, `quadBezTo`, `arcTo`, `close`; a path not starting with `moveTo` starts at the origin. `fill="none"` and `stroke="0"` are honoured per path; an open filled path is filled as if closed; line-end arrowheads are kept on open paths only. `extrusionOk`, `a:ahLst`, and `a:cxnLst` do not affect the image. The text area is `a:rect` or the whole shape when absent.

Reported approximations: `lighten`/`lightenLess`/`darken`/`darkenLess` fill modes draw as a normal fill (`approximation`); an unknown command is skipped (`unsupported`); an unresolvable guide that makes a path or text area non-finite omits the outline (`unsupported`) and the text uses the whole shape. Fixture: `examples/custom-geometry.pptx` with regions `examples/custom-geometry.json`.

## Picture effects and fills

Children of a picture's `a:blip` are applied as one SVG filter in document order:

| Element | Rendering | Diagnostic |
|---|---|---|
| `a:alphaModFix` | Alpha multiplied by `amt` | none |
| `a:grayscl` | Saturation 0 (Rec. 709 luma) | none |
| `a:biLevel` | Black or white by luma against `thresh` | none |
| `a:lum` | Linear brightness/contrast around mid-grey | `approximation` |
| `a:duotone` | Luma mapped between the two colors | `approximation` |
| `a:clrChange` | Pixels exactly equal to `clrFrom` become `clrTo`; no tolerance | `approximation` |
| `a14:saturation` (in `a14:imgProps`) | Saturation scaled by `sat` | none |
| `a14:brightnessContrast` | Same mapping as `a:lum` | `approximation` |
| `a14:backgroundRemoval`, `a14:sharpenSoften`, other `a14:imgEffect` | Picture drawn unchanged | `unsupported`, `Unsupported picture effect: <name>` |
| Other `a:blip` children (`a:alphaBiLevel`, `a:hsl`, `a:tint`, ...) | Ignored | `unsupported`, `Unsupported picture effect: <name>` |

`asvg:svgBlip` and `a14:useLocalDpi` produce no diagnostic; any other extension is named in `Unsupported picture extension: <name>`. Effects that leave pixels unchanged add no filter.

`a:tile` repeats the cropped picture over the frame. The tile size is the natural size (PNG `pHYs` or JPEG JFIF density, else 96 dpi; EXIF is not read) scaled by `sx`/`sy`; `algn` places the first tile, `tx`/`ty` offset it, and `flip` mirrors alternate tiles. A tiled SVG picture uses the frame as the tile (`approximation`). More than 256 tiles along an axis, or a pattern cell above 16 times the frame area, is not tiled: the picture is drawn stretched with `Picture tile size is out of range; drawn stretched: <part>`, and densities below 1 dpi are ignored, so a deck cannot inflate the rasterized pattern. The 96 dpi fallback is a known difference from PowerPoint, which showed an effective 144 dpi on a `pHYs`-less PNG in one measurement (issue #88); one observation does not establish a universal density, so the fallback is unchanged. `a:stretch/a:fillRect` insets shrink or, when negative, extend the stretched picture inside the frame; an inset that leaves no area omits the picture. Picture fills of shapes and backgrounds go through the same code and record no tiling diagnostic. LibreOffice draws `p:pic` tiles and fill rectangles as one stretched picture and ignores `a14:imgProps`, so those fixture slides differ from it by design. Fixture: `examples/image-effects.pptx` with regions `examples/image-effects.json`; tile generator `tests/visual/make_shape_tile.py`.

## EMF and WMF pictures

EMF and placeable WMF are decoded independently from MS-EMF and MS-WMF with no new dependency (`src/pptx/metafile.rs`). The two crates surveyed before implementation were not adopted; the reasons are recorded in [the survey](history/20261020_emf-wmf-survey.md). A picture part or OLE preview is a metafile when it starts with `EMR_HEADER` and the `" EMF"` signature at byte 40, or with the placeable WMF key `0x9AC6CDD7`. Records are translated to an SVG document that passes the same gate as SVG pictures (node limit, no external references, render check), then cropped, stretched, and placed like any other picture. A metafile that cannot be drawn is omitted once, naming the part.

Coordinates are mapped per record: world transform, then the map mode's window/viewport mapping, onto the reference device. The EMF picture is `rclFrame` converted with `szlDevice`/`szlMillimeters`, or `rclBounds` when the frame is empty; all three header sizes are read. A placeable WMF is its bounding box; viewport records are ignored there. Non-placeable WMF is not detected and stays `Only PNG/JPEG images are supported`.

| Record group | Translated |
|---|---|
| Shapes | `Rectangle`, `RoundRect`, `Ellipse`, `Polygon`, `Polyline`, `PolyPolygon`, `PolyPolyline`, `PolyBezier`, `PolyBezierTo`, `PolylineTo`, `MoveToEx`, `LineTo` (32- and 16-bit); WMF `META_RECTANGLE`, `META_ROUNDRECT`, `META_ELLIPSE`, `META_POLYGON`, `META_POLYLINE`, `META_POLYPOLYGON`, `META_MOVETO`, `META_LINETO`, `META_PATBLT` |
| Objects | `CreatePen`, `ExtCreatePen` (dashes, caps, joins), `CreateBrushIndirect`, stock objects, `SelectObject`, `DeleteObject`; WMF object table; `SetPolyFillMode` |
| Paths | `BeginPath`, `EndPath`, `CloseFigure`, `AbortPath`, `FillPath`, `StrokePath`, `StrokeAndFillPath`, `SelectClipPath` |
| Coordinates | `SetMapMode` (all eight), window/viewport origin, extent and scale records, `SetWorldTransform`, `ModifyWorldTransform`, `SaveDC`, `RestoreDC` |
| Clipping | `IntersectClipRect` (chained clip paths), `ExcludeClipRect` (mask), `ExtSelectClipRgn` with rectangle regions; modes copy, and, diff |
| Bitmaps | `StretchDIBits`, `BitBlt`, `StretchBlt`, `AlphaBlend`; WMF `META_STRETCHDIB`, `META_DIBSTRETCHBLT`, `META_DIBBITBLT`; 1/4/8/16/24/32-bit uncompressed or bit-field DIBs, stretched or mirrored, picked like GDI's default stretch modes and smoothed after `HALFTONE` or a downscale |

Diagnostics, suffixed with the part name and recorded once per message:

- `unsupported` `Text in metafile pictures is not rendered (<record>)` for the `ExtTextOut`, `PolyTextOut`, `SmallTextOut`, `META_TEXTOUT`, and `META_EXTTEXTOUT` families; the text is not drawn (issue #78).
- `unsupported` `Unsupported metafile record ignored (<record>)` once per record type outside the subset; state-only records the subset does not draw are skipped silently.
- `unsupported` for pattern brushes, compressed DIBs, and palette-index DIBs; `approximation` for hatched brushes (drawn solid), raster operations other than copy, mix modes other than `R2_COPYPEN`, clip modes or/xor, `WidenPath`, and EMF+ comments (a dual picture is drawn from its EMF records; EMF+ only is `unsupported`).
- `omitted` `Malformed bitmap in metafile record omitted (...)` for one unreadable DIB; the rest renders.

Bitmaps go through the PNG/JPEG size policy under [resource limits](#resource-limits), encoded as PNG, never embedded as BMP. A cosmetic pen is one reference-device pixel wide. Per-picture limits, each omitting the picture: 500,000 records, 2,000,000 points, 50,000 SVG elements, 67,108,864 decoded bitmap pixels, `SaveDC` nesting 1,024, 65,536 live objects, 32 MiB translated SVG, 100,000 XML nodes, and 32 chained clip links (further clip records are reported once as `approximation`). At most 64 distinct diagnostics are kept per picture. Truncated or malformed records, a missing end-of-file record, and non-finite world transforms omit the picture. `ExcludeClipRect` holes follow MS-EMF; LibreOffice does not draw them, so that fixture rests on the specification, not the reference. Fixtures: `tests/visual/make_metafile_pictures.py OUT_DIR [--pptx]` (nine metafiles under `tests/fixtures/metafile/`).

## Charts

A chart frame (`a:graphicData` with the DrawingML chart URI) is drawn from the values cached in its chart part (`c:numCache`, `c:strCache`, literals) when the plot area holds exactly one of: `c:barChart` (columns or bars; clustered, stacked, or percent-stacked), `c:lineChart` (standard grouping), or `c:pieChart` (first series). The chart is translated into ordinary shapes and text boxes (`src/pptx/chart.rs`), so PNG and PDF output, colours, and fonts follow the shape renderer. Drawn: series and point (`c:dPt`) solid fills and lines or the theme accent cycle (`c:varyColors`), title, legend at `c:legendPos`, value and category axes with tick labels unless deleted, major gridlines, line markers, and data labels for values, percentages (pie), category and series names.

- Saved state wins: an edge-mode `c:manualLayout` of the plot area and each saved `c:min`, `c:max`, and `c:majorUnit` are used as written. Everything Office computes itself is computed here and reported: every drawn chart gets `Chart layout is approximated`, and a value axis missing any of the three scale values gets `Chart axis scale is approximated` (saved values are kept). Automatic scales aim at about ten intervals with 5% headroom; text defaults are 10 pt in the theme text colour with a bold 12 pt title.
- Number formats: `General` and exactly `0`, `0.0…`, `#,##0`, `#,##0.0…`, `0%`, `0.0…%`; a label or axis `c:numFmt` with `sourceLinked="0"` wins over the cache's `c:formatCode`. Other codes fall back to General with an approximation diagnostic. General prints integers as they are and other values rounded to at most nine significant digits.
- Any other chart type, more than one plot element (even two supported ones), unsupported grouping, more than 64 series, or more than 10,000 points omits the chart before anything is drawn, with one diagnostic naming the element or limit; the slide still renders. `c:externalData` is never followed.
- Not supported (diagnosed): non-solid series fills and effects, reversed axes, secondary axes, trendlines and error bars, exploded slices (drawn in place), smoothed lines (drawn straight), marker symbols other than circle, square, diamond, and triangle (drawn as circles), multi-level categories (innermost level shown).
- Known differences, kept reproducible: against LibreOffice 26.2 on macOS, `examples/chart-line.pptx` passes the fixed comparison thresholds, while `examples/chart-bar.pptx` (slide 1 MAE 0.037 with 8.8% of pixels changed, slide 2 MAE 0.040 with 9.3%) and `examples/chart-pie.pptx` (MAE 0.031, 7.3%) exceed them (limits: MAE 0.02, 5%) because the automatic plot-area size and pie radius differ. LibreOffice is not the reference for Office layout, so the thresholds stay and the fixtures stay out of the gating comparison until Office measurements decide the layout. Reproduce with `python tests/visual/compare.py --binary target/release/rhodopsin --fixture examples/chart-bar.pptx --out target/visual-tests/chart-bar`; regenerate the fixtures with `python tests/visual/make_charts.py`.

## Known limitations

- Complete layout/master property and text-style inheritance; `themeOverride` parts; `blipFill` entries and 3-D effect styles in theme style tables.
- Full Japanese line-breaking rules, punctuation compression, all hanging symbols, complex shaping/ligatures/kerning, and dynamic autofit ratio calculation.
- Identical baselines and fallback behavior across all fonts and Office renderers.
- Chart types other than those under [Charts](#charts), SmartArt, freeform path fill modes other than normal and none, and many presets.
- Full table theme-style coverage, theme/line/table-cell/text gradients, inner shadows/glow, theme effects, and the picture effects listed as unsupported under [picture effects and fills](#picture-effects-and-fills).
- Text, arcs, regions, and EMF+ records inside EMF/WMF pictures, non-placeable WMF, SVG referenced directly from `a:blip/@r:embed`, text inside SVG pictures, external links, animations, Strict OOXML, legacy `.ppt`, and encrypted files.

SVG pictures stored the Office 2016 way (`asvg:svgBlip`) are rendered as vector data, stretched and cropped like other pictures. When `a:blip/@r:embed` also names a PNG/JPEG fallback, the fallback is drawn first; the SVG is used when there is no fallback or it cannot be drawn; if neither can, the picture is omitted once with both reasons. An SVG picture is rendered only when it is at most 4 MiB, has at most 100,000 XML nodes, parses, and references nothing outside itself (`href` other than `#id`, `url()` other than `url(#id)`, CSS `@import`, and `data:` images are rejected because they would bypass the size policy). `<text>` inside an SVG picture is `unsupported` because document fonts are not resolved for it. A blip with no `r:embed` is omitted with `Missing image reference`; `Externally linked images are not loaded` is reserved for `r:link` and external relationships.

Audio and video are never played. A slide, layout, or master with a `video`, `audio`, or `media` relationship gets one `unsupported` diagnostic (`Audio and video playback is unsupported`); media parts are not loaded and do not count toward the size limits, so an oversized media payload does not fail the deck, and the poster frame renders as an ordinary picture. Generator: `tests/visual/make_embedded_media.py` (writes about 40 MiB; keep it outside the repository).

Pictures that are not PNG/JPEG, EMF, or placeable WMF, malformed metafiles, or pictures whose dimensions cannot be read are omitted with a diagnostic naming the part, and the rest of the slide renders. The same applies to a missing relationship (`Missing image relationship: <id>`), a missing part (`Missing image part: <part>`), or a crop that leaves no area (`Image crop leaves no visible area: <part>`). A part referenced by `a:blip` or `asvg:svgBlip` is loaded even when its relationship type is one the loader otherwise skips (`oleObject`, `hdphoto`), and counts toward the size limits.

Visual output depends on available fonts. Ordinary CI covers Windows, Linux, and macOS; recorded visual comparisons are from macOS.

## Resource limits

Input size 128 MiB; total uncompressed data 256 MiB; per part 32 MiB; 200 slides; group nesting 64 levels; tables 100,000 cells (`src/pptx.rs`, `src/pptx/objects.rs`). Embedded PNG/JPEG pictures are handled per picture and never abort the deck (`src/pptx/raster.rs`):

- Up to 8192 px per side and 16,777,216 pixels: embedded unchanged.
- Above that, up to 32768 px per side and 67,108,864 pixels: decoded and box-filtered so the longest side is at most 2048 px, then embedded with an `approximation` diagnostic recording both sizes. The target bounds memory and is not derived from the output width, so a large or heavily cropped picture can look softer than in Office. Non-interlaced PNG is decoded row by row; JPEG is decoded at full size first.
- Above the ceiling, interlaced PNG above the first limit, or undecodable data: omitted with a diagnostic naming the part.

ZIP entries are read without extracting to the filesystem, and external images are never fetched. Conversion runs on an 8 MiB thread stack so OS main-stack differences do not cause parser failures; tests require a normal error exit, not a crash, for groups deeper than 64. Additional robustness validation is needed before exposing a service to arbitrary untrusted input.

For font requirements and diagnostics, see [usage](usage.md).
