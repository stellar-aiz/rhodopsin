//! Horizontal text layout using nominal font advances and line metrics.
//! Handle hanging punctuation and fixed line spacing; complex shaping is unsupported.
mod bullets;
use super::fonts::Resolver;
use super::*;
#[cfg(test)]
use resvg::usvg::fontdb::Database;
use skrifa::raw::TableProvider;
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, Size},
};

/// Diagnostic prefixes for fields whose saved text is drawn; see [`field_text`].
pub(super) const DATE_FIELD_NOTE: &str = "Date and time fields";
pub(super) const OTHER_FIELD_NOTE: &str = "Field type ";

/// The text drawn for an `a:fld` run. `slidenum` shows the slide's number; every other type
/// keeps its saved text so output stays deterministic, and says so.
fn field_text<'a>(
    field: Node<'a, '_>,
    options: &RenderOptions<'_>,
    warnings: &mut Warnings,
) -> std::borrow::Cow<'a, str> {
    let saved = child(field, A, "t").and_then(|n| n.text()).unwrap_or("");
    let kind = field.attribute("type").unwrap_or("");
    if kind == "slidenum" {
        if let Some(number) = options.slide_number.get() {
            return number.to_string().into();
        }
    } else if kind.starts_with("datetime") {
        warnings.push(
            Code::Approximation,
            format!(
                "{DATE_FIELD_NOTE} are drawn with their saved text; the conversion date is not substituted"
            ),
        );
    } else {
        let name = if kind.is_empty() {
            "(unspecified)"
        } else {
            kind
        };
        warnings.push(
            Code::Approximation,
            format!("{OTHER_FIELD_NOTE}{name} is not resolved; its saved text is drawn"),
        );
    }
    saved.into()
}

#[derive(Clone, Debug, PartialEq)]
struct Style {
    face: String,
    size: f64,
    color: String,
    bold: bool,
    italic: bool,
    spacing: f64,
}
#[derive(Clone)]
struct Glyph {
    ch: char,
    advance: f64,
    /// Advance along a vertical column from the font's `vmtx`, when it has one.
    vertical_advance: Option<f64>,
    /// Drawn upright in an `eaVert` column instead of turned with the line.
    upright: bool,
    offset: f64,
    compressed: bool,
    ascent: f64,
    descent: f64,
    leading: f64,
    style: Style,
}
struct Line {
    glyphs: Vec<Glyph>,
    x: f64,
    baseline: f64,
}

fn attr<'a>(nodes: &[Node<'a, '_>], key: &str) -> Option<&'a str> {
    nodes.iter().find_map(|n| n.attribute(key))
}
fn prop_number(nodes: &[Node<'_, '_>], key: &str, default: f64) -> f64 {
    attr(nodes, key)
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}
fn face(nodes: &[Node<'_, '_>], kind: &str) -> Option<String> {
    nodes.iter().find_map(|n| {
        child(*n, A, kind)
            .and_then(|n| n.attribute("typeface"))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })
}
fn first_fill<'a, 'i>(nodes: &[Node<'a, 'i>]) -> Option<Node<'a, 'i>> {
    nodes.iter().find_map(|n| child(*n, A, "solidFill"))
}
/// Resolve a run's style. `nodes` are the run's own and shape-level properties, most specific
/// first; `fallback` are the master and presentation defaults. A shape style's `a:fontRef`
/// (`reference`) sits between them: PowerPoint shows its colour on a default shape even though
/// the master's `otherStyle` names the text colour.
fn style(
    nodes: &[Node<'_, '_>],
    fallback: &[Node<'_, '_>],
    east_asian: bool,
    theme: &Theme,
    reference: Option<&StyleRef>,
    warnings: &mut Warnings,
) -> Style {
    let all: Vec<Node<'_, '_>> = nodes.iter().chain(fallback).copied().collect();
    // The shape style's theme font is the default Latin face (unresolvable references are
    // reported by `Theme::font`); its East Asian face only when the theme declares one, so a
    // theme without one does not produce a diagnostic per run.
    let theme_face = |script: &str| {
        reference
            .and_then(|r| r.font)
            .map(|prefix| format!("{prefix}-{script}"))
    };
    let latin = face(nodes, "latin")
        .or_else(|| theme_face("lt"))
        .or_else(|| face(fallback, "latin"))
        .unwrap_or_else(|| "sans-serif".into());
    let requested = if east_asian {
        face(nodes, "ea")
            .or_else(|| theme_face("ea").filter(|key| theme.has_font(key)))
            .or_else(|| face(fallback, "ea"))
            .unwrap_or(latin)
    } else {
        latin
    };
    Style {
        face: theme.font(&requested, warnings),
        size: prop_number(&all, "sz", 1800.0).clamp(100.0, 40000.0) / 75.0,
        color: first_fill(nodes)
            .map(|n| color(n, "#000000", theme, warnings))
            .or_else(|| reference.and_then(|r| r.color.clone()))
            .or_else(|| first_fill(fallback).map(|n| color(n, "#000000", theme, warnings)))
            .unwrap_or_else(|| "#000000".into()),
        bold: matches!(attr(&all, "b"), Some("1" | "true")),
        italic: matches!(attr(&all, "i"), Some("1" | "true")),
        spacing: prop_number(&all, "spc", 0.0).clamp(-400000.0, 400000.0) / 75.0,
    }
}
/// PowerPoint's measured natural line height for substituted families, in em.
/// See docs/rendering.md#substituted-fonts.
const SUBSTITUTED_LINE_HEIGHT: f64 = 1.2;
/// Ascent and descent for a substituted family: the fixed height split by the
/// substitute's ascent:descent ratio.
fn substituted_metrics(size: f64, ascent: f64, descent: f64) -> Option<(f64, f64)> {
    (ascent > 0.0 && descent >= 0.0 && ascent.is_finite() && descent.is_finite()).then(|| {
        let height = SUBSTITUTED_LINE_HEIGHT * size;
        let ascent = height * ascent / (ascent + descent);
        (ascent, height - ascent)
    })
}
fn is_east_asian(ch: char) -> bool {
    matches!(ch as u32, 0x3000..=0x9fff | 0xf900..=0xfaff | 0xff00..=0xffef | 0x20000..=0x323af)
}
// Measurement size matching Impress's 600 DPI reference device.
// Keep glyph rendering size unchanged. See docs/history/20261015_text-measurement.md for evidence and scope.
fn measurement_size(css_px: f64) -> f64 {
    let hundredth_mm = (css_px * 2540.0 / 96.0).round();
    (hundredth_mm * 600.0 / 2540.0).round().max(1.0) * 96.0 / 600.0
}

fn glyphs(
    value: &str,
    mut style: Style,
    resolver: &Resolver<'_>,
    warnings: &mut Warnings,
) -> Vec<Glyph> {
    let fonts = resolver.database;
    let requested = style.face.clone();
    let internal = fonts
        .faces()
        .find(|f| format!("rhodopsin-face-{}", f.id) == style.face)
        .map(|f| f.id);
    let id = internal
        .or_else(|| resolver.select(&style.face, value, style.bold, style.italic, warnings));
    // A face of another family stands in for the requested one. Per-glyph fallback faces
    // arrive as internal names and keep their own metrics.
    let substituted =
        internal.is_none() && id.is_some_and(|id| resolver.substitutes(id, &requested));
    if let Some(id) = id {
        style.face = format!("rhodopsin-face-{id}");
        if let Some(Some(result)) = fonts.with_face_data(id, |data, index| {
            let font = FontRef::from_index(data, index).ok()?;
            let size = Size::new(style.size as f32);
            let metrics = font.metrics(size, LocationRef::default());
            // Use Windows cell metrics for Office-style horizontal text.
            // Fall back to standard typographic/hhea metrics when OS/2 is absent.
            let (ascent, descent, leading) = font
                .os2()
                .ok()
                .filter(|t| t.us_win_ascent() > 0)
                .map(|t| {
                    let scale = style.size / f64::from(metrics.units_per_em);
                    (
                        f64::from(t.us_win_ascent()) * scale,
                        f64::from(t.us_win_descent()) * scale,
                        0.0,
                    )
                })
                .unwrap_or((
                    f64::from(metrics.ascent),
                    f64::from(-metrics.descent),
                    f64::from(metrics.leading.max(0.0)),
                ));
            let (ascent, descent, leading) = substituted
                .then(|| substituted_metrics(style.size, ascent, descent))
                .flatten()
                .map_or((ascent, descent, leading), |(a, d)| (a, d, 0.0));
            let gm = font.glyph_metrics(Size::unscaled(), LocationRef::default());
            if metrics.units_per_em == 0 {
                return None;
            }
            // Sized glyph_metrics include fixed-point rounding.
            // Convert font-unit advances to the measurement size with double precision.
            let advance_scale = measurement_size(style.size) / f64::from(metrics.units_per_em);
            let map = font.charmap();
            let vmtx = font.vmtx().ok();
            Some(
                value
                    .chars()
                    .map(|ch| {
                        let id = map.map(ch);
                        let advance = id.and_then(|id| gm.advance_width(id));
                        let vertical_advance = id
                            .zip(vmtx.as_ref())
                            .and_then(|(id, vmtx)| vmtx.advance(id))
                            .map(|value| f64::from(value) * advance_scale);
                        if advance.is_none() && ch != '\n' {
                            let fallback = resolver.fallback(
                                &requested,
                                ch,
                                style.bold,
                                style.italic,
                                warnings,
                            );
                            if let Some(fallback) = fallback {
                                let mut replacement = style.clone();
                                replacement.face = format!("rhodopsin-face-{fallback}");
                                return glyphs(&ch.to_string(), replacement, resolver, warnings)
                                    .remove(0);
                            }
                            warnings.missing_glyph(&requested, ch);
                        }
                        Glyph {
                            ch,
                            offset: 0.0,
                            compressed: false,
                            vertical_advance,
                            upright: false,
                            advance: advance
                                .map(|value| f64::from(value) * advance_scale)
                                .unwrap_or(style.size),
                            ascent,
                            descent,
                            leading,
                            style: style.clone(),
                        }
                    })
                    .collect(),
            )
        }) {
            return result;
        }
    }
    warnings.push(
        Code::Approximation,
        "Cannot read font metrics; character widths are approximate",
    );
    value
        .chars()
        .map(|ch| Glyph {
            ch,
            offset: 0.0,
            compressed: false,
            vertical_advance: None,
            upright: false,
            advance: style.size,
            ascent: style.size,
            descent: style.size * 0.2,
            leading: 0.0,
            style: style.clone(),
        })
        .collect()
}
fn break_allowed(left: char, right: char) -> bool {
    let prohibited_start = "、。，．・：；？！ー）］｝〉》」』】〕〗〙〛’”ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ々ゝゞヽヾ";
    let prohibited_end = "（［｛〈《「『【〔〖〘〚‘“";
    !prohibited_start.contains(right)
        && !prohibited_end.contains(left)
        && !(left.is_ascii_alphanumeric() && right.is_ascii_alphanumeric())
        && !matches!(right as u32, 0x0300..=0x036f | 0x3099..=0x309a | 0xfe00..=0xfe0f)
}
#[cfg(test)]
fn paragraph_hanging(nodes: [Option<Node<'_, '_>>; 3], inherited: bool) -> bool {
    nodes
        .into_iter()
        .flatten()
        .find_map(|n| theme::bool_attr(n, "hangingPunct"))
        .unwrap_or(inherited)
}
fn can_hang(ch: char) -> bool {
    "、。，．".contains(ch)
}
// Add spacing only between characters with the same style, never at line ends.
// LibreOffice also resets character spacing at style boundaries.
fn step(glyph: &Glyph, next: Option<&Glyph>) -> f64 {
    let spacing = next
        .filter(|next| glyph.style == next.style)
        .map(|_| glyph.style.spacing)
        .unwrap_or(0.0);
    (glyph.advance + spacing).max(0.0)
}
fn text_width(glyphs: &[Glyph]) -> f64 {
    glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| step(g, glyphs.get(i + 1)))
        .sum()
}
fn layout_width(glyphs: &[Glyph], available: f64, hanging: bool) -> f64 {
    let width = text_width(glyphs);
    if hanging
        && width > available
        && let Some(last) = glyphs.last()
        && can_hang(last.ch)
        && text_width(&glyphs[..glyphs.len() - 1]) <= available
    {
        text_width(&glyphs[..glyphs.len() - 1])
    } else {
        width
    }
}
fn baseline_offset(
    paragraph: Option<Node<'_, '_>>,
    ascent: f64,
    natural: f64,
    advance: f64,
) -> f64 {
    let fixed = paragraph
        .and_then(|n| child(n, A, "lnSpc"))
        .and_then(|n| child(n, A, "spcPts"))
        .is_some();
    // Fixed line spacing preserves the area below the baseline and adjusts space above it.
    // Percentage spacing changes distance to the next line, not the first line's ascent.
    if fixed {
        ascent + advance - natural
    } else {
        ascent
    }
}
// Layout units verified against LibreOffice (1/100 mm).
// Round boxes and style spans; truncate cumulative fit widths and per-character compression space.
const LAYOUT_UNITS_PER_PX: f64 = 2540.0 / 96.0;
fn layout_units(width: f64) -> f64 {
    (width * LAYOUT_UNITS_PER_PX).round()
}
fn fits_line(width: f64, compressed_units: f64, available: f64) -> bool {
    // Use the integer part of cumulative width for boundary fitting, not per-character truncation.
    (width * LAYOUT_UNITS_PER_PX).floor() - compressed_units <= layout_units(available)
}
fn wrap(glyphs: &[Glyph], available: f64, enabled: bool, hanging: bool) -> Vec<Vec<Glyph>> {
    let mut result = Vec::new();
    for hard_line in glyphs.split(|g| g.ch == '\n') {
        if !enabled || hard_line.is_empty() {
            result.push(hard_line.to_vec());
            continue;
        }
        let explicit_spacing = hard_line.iter().any(|g| g.style.spacing != 0.0);
        let mut remaining = hard_line;
        while !remaining.is_empty() {
            let mut sum = 0.0;
            let mut segment_width = 0.0;
            let mut completed_units = 0.0;
            let mut end = 0;
            while end < remaining.len() {
                let candidate = if end == 0 {
                    remaining[end].advance
                } else {
                    sum - remaining[end - 1].advance
                        + step(&remaining[end - 1], Some(&remaining[end]))
                        + remaining[end].advance
                };
                // With explicit character spacing, round widths per same-style span.
                if end > 0 && remaining[end - 1].style != remaining[end].style {
                    completed_units += layout_units(segment_width);
                    segment_width = remaining[end].advance;
                } else {
                    segment_width += candidate - sum;
                }
                let fits = if explicit_spacing {
                    completed_units + layout_units(segment_width) <= layout_units(available)
                } else {
                    fits_line(candidate, 0.0, available)
                };
                if end > 0 && !fits {
                    break;
                }
                sum = candidate;
                end += 1;
            }
            // hangingPunct allows one punctuation character outside the box, before prohibited-break handling.
            if hanging && end < remaining.len() && can_hang(remaining[end].ch) {
                end += 1;
            }
            if end < remaining.len() {
                let fit = (1..=end)
                    .rev()
                    .find(|&i| break_allowed(remaining[i - 1].ch, remaining[i].ch));
                end = fit.unwrap_or_else(|| {
                    (end..remaining.len())
                        .find(|&i| break_allowed(remaining[i - 1].ch, remaining[i].ch))
                        .unwrap_or(remaining.len())
                });
            }
            result.push(remaining[..end].to_vec());
            remaining = &remaining[end..];
        }
    }
    result
}
// Compress only as much as needed, up to half-width spacing for full-width punctuation.
// Do not compress middle dots, exclamation marks, question marks, or kana in this mode.
fn opening(ch: char) -> bool {
    "「『（［｛〈《【〔".contains(ch)
}
fn capacity(g: &Glyph) -> f64 {
    if (opening(g.ch) || "、。，．」』）］｝〉》】〕：；".contains(g.ch))
        && (g.advance - g.style.size).abs() < g.style.size * 0.05
    {
        g.advance / 2.0
    } else {
        0.0
    }
}
fn compress_line(glyphs: &[Glyph], ratio: f64) -> Vec<Glyph> {
    let leading = glyphs
        .first()
        .filter(|g| opening(g.ch))
        .map(|g| capacity(g) * ratio)
        .unwrap_or(0.0);
    glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let mut result = g.clone();
            let reduction = capacity(g) * ratio;
            result.advance -= reduction;
            result.compressed = ratio > 0.0;
            // Keep leading opening brackets inside the box and shift following positions back equally.
            result.offset = if i == 0 {
                0.0
            } else {
                leading - if opening(g.ch) { reduction } else { 0.0 }
            };
            result
        })
        .collect()
}
fn compress_wrap(glyphs: &[Glyph], available: f64, hanging: bool) -> Vec<Vec<Glyph>> {
    let mut result = Vec::new();
    for hard_line in glyphs.split(|g| g.ch == '\n') {
        if hard_line.is_empty() {
            result.push(vec![]);
            continue;
        }
        let mut remaining = hard_line;
        while !remaining.is_empty() {
            let mut natural = 0.0;
            let mut compressed_units = 0.0;
            let mut end = 0;
            while end < remaining.len() {
                let next = natural + remaining[end].advance;
                let next_compressed =
                    compressed_units + (capacity(&remaining[end]) * LAYOUT_UNITS_PER_PX).floor();
                if end > 0 && !fits_line(next, next_compressed, available) {
                    break;
                }
                natural = next;
                compressed_units = next_compressed;
                end += 1;
            }
            if hanging && end < remaining.len() && can_hang(remaining[end].ch) {
                end += 1;
            }
            let candidate = end;
            if end < remaining.len() {
                end = (1..=end)
                    .rev()
                    .find(|&i| break_allowed(remaining[i - 1].ch, remaining[i].ch))
                    .unwrap_or_else(|| {
                        (end..remaining.len())
                            .find(|&i| break_allowed(remaining[i - 1].ch, remaining[i].ch))
                            .unwrap_or(remaining.len())
                    });
            }
            let line = &remaining[..end];
            let total: f64 = line.iter().map(capacity).sum();
            let width = layout_width(line, available, hanging);
            // At integer boundaries where wrapped lines regain natural width, compressed glyph positions persist.
            // Apply the one-unit (1/100 mm) boundary allowance only to this retention decision.
            let ratio = if total == 0.0 {
                0.0
            } else if end < candidate
                || (end < remaining.len() && layout_units(width) <= layout_units(available) + 1.0)
            {
                1.0
            } else {
                ((width - available) / total).clamp(0.0, 1.0)
            };
            result.push(compress_line(line, ratio));
            remaining = &remaining[end..];
        }
    }
    result
}
fn spacing(node: Option<Node<'_, '_>>, tag: &str, natural: f64, default: f64) -> f64 {
    let Some(value) = node.and_then(|n| child(n, A, tag)) else {
        return default;
    };
    if let Some(points) = child(value, A, "spcPts") {
        return number(points, "val", 0.0) / 75.0;
    }
    child(value, A, "spcPct")
        .map(|n| natural * number(n, "val", 100000.0) / 100000.0)
        .unwrap_or(default)
}
/// Characters that stay turned with the line in `eaVert` columns: brackets, dashes,
/// and the long vowel mark, whose vertical forms are their turned shapes.
fn sideways_in_columns(ch: char) -> bool {
    "ー－～〜…‥―（）［］｛｝〈〉《》「」『』【】〔〕〖〗〘〙〚〛＝＜＞".contains(ch)
}
/// Turn East Asian characters upright for an `eaVert` column. Their advance along the
/// column comes from the font's vertical metrics, or the em size without them.
fn stand_upright(glyphs: &mut [Glyph], warnings: &mut Warnings) {
    let mut missing_metrics = false;
    for glyph in glyphs
        .iter_mut()
        .filter(|g| is_east_asian(g.ch) && !sideways_in_columns(g.ch))
    {
        glyph.upright = true;
        glyph.advance = glyph.vertical_advance.unwrap_or_else(|| {
            missing_metrics = true;
            measurement_size(glyph.style.size)
        });
    }
    if missing_metrics {
        warnings.push(
            Code::Approximation,
            "Font has no vertical metrics; upright characters in vertical text advance by the em size",
        );
    }
}
pub(super) struct TextBody<'a> {
    pub body: Node<'a, 'a>,
    pub inherited: Vec<Node<'a, 'a>>,
    pub master_style: Option<Node<'a, 'a>>,
    pub cell_props: Option<Node<'a, 'a>>,
    /// Defaults from the shape style's `a:fontRef`, used only where nothing else sets them.
    pub style_ref: Option<StyleRef>,
}
impl<'a> From<Node<'a, 'a>> for TextBody<'a> {
    fn from(body: Node<'a, 'a>) -> Self {
        Self {
            body,
            inherited: vec![],
            master_style: None,
            cell_props: None,
            style_ref: None,
        }
    }
}
/// Text defaults a shape style's `a:fontRef` supplies: the colour it carries and the theme
/// font family it selects (`+mn` or `+mj`).
#[derive(Clone, Debug, Default)]
pub(super) struct StyleRef {
    pub color: Option<String>,
    pub font: Option<&'static str>,
}
impl StyleRef {
    pub(super) fn from_font_reference(
        reference: Node<'_, '_>,
        theme: &Theme,
        warnings: &mut Warnings,
    ) -> Self {
        let font = match reference.attribute("idx") {
            Some("minor") => Some("+mn"),
            Some("major") => Some("+mj"),
            Some("none") | None => None,
            Some(other) => {
                warn(
                    warnings,
                    format!("Unsupported theme font reference: {other}"),
                );
                None
            }
        };
        let color = reference
            .children()
            .any(|n| n.is_element())
            .then(|| color(reference, "#000000", theme, warnings));
        Self { color, font }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TextExtent {
    pub height: f64,
    pub offset_y: f64,
    /// Height the text needs including the top and bottom insets, independent of the
    /// given box height and anchor. Tables use it to grow rows to fit their content.
    pub required_height: f64,
    /// Final box width. It differs from the given width only for `spAutoFit` bodies
    /// that do not wrap, which take the widest line plus the left and right insets.
    pub width: f64,
    /// Horizontal shift of the box's left edge that keeps the edge on the first
    /// paragraph's alignment side fixed (0 for left, half the change for centre).
    pub offset_x: f64,
}
#[derive(Clone, Copy, Debug)]
struct AutoFit {
    scale: f64,
    reduction: f64,
    expand: bool,
}
fn auto_fit(nodes: &[Node<'_, '_>], warnings: &mut Warnings) -> AutoFit {
    let mode = nodes.iter().find_map(|n| {
        n.children().find(|c| {
            c.tag_name().namespace() == Some(A)
                && matches!(
                    c.tag_name().name(),
                    "noAutofit" | "normAutofit" | "spAutoFit"
                )
        })
    });
    let mut result = AutoFit {
        scale: 1.0,
        reduction: 0.0,
        expand: false,
    };
    if let Some(mode) = mode {
        match mode.tag_name().name() {
            "normAutofit" => {
                let percentage =
                    |key: &str, default: f64, maximum: f64, warnings: &mut Warnings| {
                        let Some(raw) = mode.attribute(key) else {
                            return default;
                        };
                        let value = if let Some(percent) = raw.strip_suffix('%') {
                            percent.parse::<f64>().ok().map(|n| n / 100.0)
                        } else {
                            raw.parse::<f64>().ok().map(|n| n / 100000.0)
                        };
                        match value.filter(|n| {
                            n.is_finite()
                                && *n >= 0.0
                                && *n <= maximum
                                && (key != "fontScale" || *n > 0.0)
                        }) {
                            Some(value) => value,
                            None => {
                                warn(
                                    warnings,
                                    format!("Invalid autofit value {key}={raw}; using the default"),
                                );
                                default
                            }
                        }
                    };
                result.scale = percentage("fontScale", 1.0, 1.0, warnings);
                result.reduction = percentage("lnSpcReduction", 0.0, 1.0, warnings);
            }
            "spAutoFit" => result.expand = true,
            _ => {}
        }
    }
    result
}

/// How lines run inside the text box (`a:bodyPr@vert`).
#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Horizontal,
    /// `vert`: horizontal lines turned 90 degrees clockwise.
    Clockwise,
    /// `vert270`: horizontal lines turned 90 degrees counter-clockwise.
    CounterClockwise,
    /// `eaVert`: columns from right to left with upright East Asian characters,
    /// laid out as clockwise lines whose upright glyphs are turned back.
    EastAsian,
}
impl Flow {
    /// Inset keys (and table-cell margin keys and defaults) that act as the left,
    /// right, top, and bottom insets of the lines once they are turned.
    fn insets(self) -> [(&'static str, &'static str, f64); 4] {
        let l = ("lIns", "marL", 91440.0);
        let r = ("rIns", "marR", 91440.0);
        let t = ("tIns", "marT", 45720.0);
        let b = ("bIns", "marB", 45720.0);
        match self {
            Flow::Horizontal => [l, r, t, b],
            Flow::Clockwise | Flow::EastAsian => [t, b, r, l],
            Flow::CounterClockwise => [b, t, l, r],
        }
    }
}

pub(super) fn render<'a>(
    source: impl Into<TextBody<'a>>,
    w: f64,
    h: f64,
    theme: &Theme,
    options: &RenderOptions<'_>,
    svg: &mut String,
    warnings: &mut Warnings,
) -> TextExtent {
    let source = source.into();
    let vert = std::iter::once(source.body)
        .chain(source.inherited.iter().copied())
        .filter_map(|n| child(n, A, "bodyPr"))
        .find_map(|n| n.attribute("vert"));
    let flow = match vert {
        None | Some("horz") => Flow::Horizontal,
        Some("vert") => Flow::Clockwise,
        Some("vert270") => Flow::CounterClockwise,
        Some("eaVert") => Flow::EastAsian,
        Some(_) => {
            warn(warnings, "Vertical text is unsupported");
            Flow::Horizontal
        }
    };
    if flow == Flow::Horizontal {
        return layout(source, w, h, flow, theme, options, svg, warnings);
    }
    if flow == Flow::EastAsian {
        warnings.push(
            Code::Approximation,
            "Vertical East Asian text turns or shifts horizontal glyphs instead of using the font's vertical forms",
        );
    }
    // Lay the lines out in the box with its sides swapped, then turn them into place.
    svg.push_str(&if flow == Flow::CounterClockwise {
        format!("<g transform=\"translate(0 {h}) rotate(-90)\">")
    } else {
        format!("<g transform=\"translate({w} 0) rotate(90)\">")
    });
    layout(source, h, w, flow, theme, options, svg, warnings);
    svg.push_str("</g>");
    // Vertical bodies never autofit, so the box keeps its size.
    TextExtent {
        height: h,
        offset_y: 0.0,
        required_height: h,
        width: w,
        offset_x: 0.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn layout(
    source: TextBody<'_>,
    w: f64,
    h: f64,
    flow: Flow,
    theme: &Theme,
    options: &RenderOptions<'_>,
    svg: &mut String,
    warnings: &mut Warnings,
) -> TextExtent {
    let body = source.body;
    let bodies: Vec<_> = std::iter::once(body)
        .chain(source.inherited.iter().copied())
        .collect();
    warnings.push(
        Code::Approximation,
        "Text layout is approximate; complex shaping and full Office compatibility are unsupported",
    );
    if options.punctuation_compression {
        warn(
            warnings,
            "Automatic punctuation compression is an experimental rendering policy, not a setting in the PPTX",
        );
    }
    let resolver = &options.fonts;
    let body_props: Vec<_> = bodies
        .iter()
        .filter_map(|n| child(*n, A, "bodyPr"))
        .collect();
    let autofit = auto_fit(&body_props, warnings);
    let text_style =
        |nodes: &[Node<'_, '_>], fallback: &[Node<'_, '_>], east: bool, warnings: &mut Warnings| {
            let mut style = style(
                nodes,
                fallback,
                east,
                theme,
                source.style_ref.as_ref(),
                warnings,
            );
            style.size *= autofit.scale;
            style
        };
    let margin = |key, cell_key, default| {
        source
            .cell_props
            .and_then(|n| n.attribute(cell_key))
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|n| n.is_finite())
            .unwrap_or_else(|| prop_number(&body_props, key, default))
            / EMU
    };
    let [left, right, top, bottom] = flow
        .insets()
        .map(|(key, cell_key, default)| margin(key, cell_key, default));
    let wrap_enabled = attr(&body_props, "wrap") != Some("none");
    let available = (w - left - right).max(1.0);
    let placeholder_type = bodies
        .iter()
        .filter_map(|n| n.parent())
        .filter_map(layout::placeholder)
        .find_map(|n| n.attribute("type"));
    let style_kind = match placeholder_type {
        Some("title" | "ctrTitle") => "titleStyle",
        Some("body" | "obj" | "subTitle") => "bodyStyle",
        _ => "otherStyle",
    };
    // Ordinary text boxes use presentation defaults, not placeholder
    // master styles. Text inside shapes continues to use otherStyle.
    let textbox = body
        .parent()
        .and_then(|n| child(n, P, "nvSpPr"))
        .and_then(|n| child(n, P, "cNvSpPr"))
        .and_then(|n| theme::bool_attr(n, "txBox"))
        == Some(true);
    let presentation = theme
        .presentation
        .as_deref()
        .and_then(|s| Document::parse(s).ok());
    let presentation_style = presentation
        .as_ref()
        .and_then(|doc| child(doc.root_element(), P, "defaultTextStyle"));
    let mut counters = bullets::Counters::default();
    let mut lines = Vec::new();
    let mut cursor = 0.0;
    // Widest line, measured from the left inset to the end of its paragraph's right margin.
    let mut natural_width: f64 = 0.0;
    let mut has_visible_text = false;
    // Only unwrapped shape autofit needs the natural width.
    let fit_width = autofit.expand && !wrap_enabled;
    let mut first_alignment = None;
    for paragraph in body.children().filter(|n| n.has_tag_name((A, "p"))) {
        let pp = child(paragraph, A, "pPr");
        let level = pp
            .map(|n| number(n, "lvl", 0.0) as usize)
            .unwrap_or(0)
            .min(8);
        let level_tag = format!("lvl{}pPr", level + 1);
        let mut paragraph_props: Vec<_> = pp.into_iter().collect();
        for (i, body) in bodies.iter().enumerate() {
            if i > 0
                && let Some(inherited) = child(*body, A, "p").and_then(|p| child(p, A, "pPr"))
            {
                paragraph_props.push(inherited);
            }
            if let Some(list) = child(*body, A, "lstStyle") {
                paragraph_props.extend(child(list, A, &level_tag));
                paragraph_props.extend(child(list, A, "defPPr"));
            }
        }
        // Properties up to here belong to the shape; for an ordinary shape the master and
        // presentation defaults below rank under its style's `a:fontRef`. Placeholders keep
        // their inherited master styles above it (only ordinary shapes were measured).
        let mut shape_level = paragraph_props.len();
        if !textbox && let Some(style) = source.master_style.and_then(|n| child(n, P, style_kind)) {
            paragraph_props.extend(child(style, A, &level_tag));
            paragraph_props.extend(child(style, A, "defPPr"));
        }
        if let Some(style) = presentation_style {
            paragraph_props.extend(child(style, A, &level_tag));
            paragraph_props.extend(child(style, A, "defPPr"));
        }
        if placeholder_type.is_some() {
            shape_level = paragraph_props.len();
        }
        let hanging = paragraph_props
            .iter()
            .find_map(|n| theme::bool_attr(*n, "hangingPunct"))
            .unwrap_or_else(|| theme.hanging_punctuation(style_kind, level));
        let defaults: Vec<_> = paragraph_props[..shape_level]
            .iter()
            .filter_map(|n| child(*n, A, "defRPr"))
            .collect();
        let theme_defaults: Vec<_> = paragraph_props[shape_level..]
            .iter()
            .filter_map(|n| child(*n, A, "defRPr"))
            .collect();
        let spacing_node = |tag| {
            paragraph_props
                .iter()
                .copied()
                .find(|n| child(*n, A, tag).is_some())
        };
        let mut all = Vec::new();
        for run in paragraph.children().filter(|n| n.is_element()) {
            if run.has_tag_name((A, "br")) {
                all.extend(glyphs(
                    "\n",
                    text_style(
                        &child(run, A, "rPr")
                            .into_iter()
                            .chain(defaults.iter().copied())
                            .collect::<Vec<_>>(),
                        &theme_defaults,
                        false,
                        warnings,
                    ),
                    resolver,
                    warnings,
                ));
            } else if run.has_tag_name((A, "r")) || run.has_tag_name((A, "fld")) {
                let nodes: Vec<_> = child(run, A, "rPr")
                    .into_iter()
                    .chain(defaults.iter().copied())
                    .collect();
                let value = if run.has_tag_name((A, "fld")) {
                    field_text(run, options, warnings)
                } else {
                    child(run, A, "t")
                        .and_then(|n| n.text())
                        .unwrap_or("")
                        .into()
                };
                // Resolve Latin and East Asian font declarations separately.
                let mut segment = String::new();
                let mut last = None;
                for ch in value.chars() {
                    let east = is_east_asian(ch);
                    if last.is_some_and(|v| v != east) {
                        all.extend(glyphs(
                            &segment,
                            text_style(&nodes, &theme_defaults, last.unwrap(), warnings),
                            resolver,
                            warnings,
                        ));
                        segment.clear();
                    }
                    segment.push(ch);
                    last = Some(east);
                }
                if !segment.is_empty() {
                    all.extend(glyphs(
                        &segment,
                        text_style(&nodes, &theme_defaults, last.unwrap_or(false), warnings),
                        resolver,
                        warnings,
                    ));
                }
            }
        }
        if flow == Flow::EastAsian {
            stand_upright(&mut all, warnings);
        }
        // The counter is advanced for every paragraph so numbering stays as before;
        // a text run, including a space-only run, makes the marker visible.
        let first_run = paragraph
            .children()
            .find(|n| n.has_tag_name((A, "r")) || n.has_tag_name((A, "fld")));
        let marker = counters
            .label(&paragraph_props, level, warnings)
            .filter(|_| first_run.is_some());
        let marker_glyphs = marker
            .as_deref()
            .map(|text| {
                let marker_nodes: Vec<_> = first_run
                    .and_then(|n| child(n, A, "rPr"))
                    .into_iter()
                    .chain(defaults.iter().copied())
                    .collect();
                let marker_style = text_style(
                    &marker_nodes,
                    &theme_defaults,
                    all.first().is_some_and(|g| is_east_asian(g.ch)),
                    warnings,
                );
                let marker_style = bullets::style(
                    &paragraph_props,
                    marker_style,
                    autofit.scale,
                    theme,
                    warnings,
                );
                glyphs(text, marker_style, resolver, warnings)
            })
            .unwrap_or_default();
        let margin_left = prop_number(&paragraph_props, "marL", 0.0) / EMU;
        let margin_right = prop_number(&paragraph_props, "marR", 0.0) / EMU;
        let indent = prop_number(&paragraph_props, "indent", 0.0) / EMU;
        let paragraph_left = left + margin_left;
        let paragraph_width = (available - margin_left - margin_right).max(1.0);
        let first_indent = if marker.is_some() { 0.0 } else { indent };
        if first_alignment.is_none() {
            first_alignment = Some(attr(&paragraph_props, "algn").unwrap_or("l"));
        }
        has_visible_text |= autofit.expand && all.iter().any(|g| !g.ch.is_whitespace());
        if fit_width && !marker_glyphs.is_empty() {
            natural_width = natural_width.max(margin_left + indent + text_width(&marker_glyphs));
        }
        if marker.is_some() && text_width(&marker_glyphs) > -indent && !marker_glyphs.is_empty() {
            warn(
                warnings,
                "Insufficient space between bullet and text; check marL/indent",
            );
        }
        if marker.is_some() && attr(&paragraph_props, "algn").is_some_and(|a| a != "l") {
            warn(
                warnings,
                "Bullet positions for non-left-aligned paragraphs are approximate",
            );
        }
        if all.windows(2).any(|pair| {
            pair[0].style == pair[1].style && pair[0].advance + pair[0].style.spacing < 0.0
        }) {
            warn(
                warnings,
                "Negative character spacing exceeding character width is unsupported; advance is clamped to 0",
            );
        }
        let end_defaults: Vec<_> = child(paragraph, A, "endParaRPr")
            .into_iter()
            .chain(defaults.iter().copied())
            .collect();
        let empty_style = text_style(&end_defaults, &theme_defaults, false, warnings);
        let has_empty_line = all.is_empty()
            || all.first().is_some_and(|g| g.ch == '\n')
            || all.last().is_some_and(|g| g.ch == '\n')
            || all.windows(2).any(|p| p[0].ch == '\n' && p[1].ch == '\n');
        let empty = if has_empty_line {
            glyphs(" ", empty_style, resolver, warnings)
        } else {
            Vec::new()
        };
        let empty_ascent = empty.first().map(|g| g.ascent).unwrap_or(0.0);
        let empty_height = empty
            .first()
            .map(|g| g.ascent + g.descent + g.leading)
            .unwrap_or(0.0);
        let natural = all
            .iter()
            .map(|g| g.ascent + g.descent + g.leading)
            .fold(0.0, f64::max)
            .max(if all.is_empty() { empty_height } else { 0.0 });
        cursor += spacing(spacing_node("spcBef"), "spcBef", natural, 0.0);
        let compressed = options.punctuation_compression && wrap_enabled;
        if compressed && all.iter().any(|g| g.style.spacing != 0.0) {
            warn(
                warnings,
                "Automatic punctuation compression with explicit character spacing is unsupported; explicit spacing takes precedence",
            );
        }
        let wrap_lines = |glyphs: &[Glyph], width| {
            if compressed && all.iter().all(|g| g.style.spacing == 0.0) {
                compress_wrap(glyphs, width, hanging)
            } else {
                wrap(glyphs, width, wrap_enabled, hanging)
            }
        };
        let mut wrapped = wrap_lines(&all, (paragraph_width - first_indent).max(1.0));
        if first_indent != 0.0 && wrapped.len() > 1 {
            let first = wrapped.remove(0);
            let consumed = first.len();
            let rest = if all.get(consumed).is_some_and(|g| g.ch == '\n') {
                consumed + 1
            } else {
                consumed
            };
            wrapped = std::iter::once(first)
                .chain(wrap_lines(&all[rest..], paragraph_width))
                .collect();
        }
        for (line_index, glyph_line) in wrapped.into_iter().enumerate() {
            let line_indent = if line_index == 0 { first_indent } else { 0.0 };
            let line_available = paragraph_width - line_indent;
            let width = layout_width(&glyph_line, line_available, hanging);
            if fit_width {
                natural_width = natural_width
                    .max(margin_left + line_indent + text_width(&glyph_line) + margin_right);
            }
            if width > line_available + 0.01 {
                warnings.push(Code::TextOverflow, "Text exceeds the text box width");
            }
            let metrics = glyph_line
                .iter()
                .chain(marker_glyphs.iter().filter(|_| line_index == 0));
            let ascent = metrics.clone().map(|g| g.ascent).fold(
                if glyph_line.is_empty() {
                    empty_ascent
                } else {
                    0.0
                },
                f64::max,
            );
            let descent = metrics.clone().map(|g| g.descent).fold(0.0, f64::max);
            let leading = metrics.map(|g| g.leading).fold(0.0, f64::max);
            let line_height =
                if glyph_line.is_empty() && (line_index != 0 || marker_glyphs.is_empty()) {
                    empty_height
                } else {
                    ascent + descent + leading
                };
            let fixed = spacing_node("lnSpc")
                .and_then(|n| child(n, A, "lnSpc"))
                .and_then(|n| child(n, A, "spcPts"))
                .is_some();
            let advance = (spacing(spacing_node("lnSpc"), "lnSpc", line_height, line_height)
                - if fixed {
                    0.0
                } else {
                    line_height * autofit.reduction
                })
            .max(0.0);
            let x = match attr(&paragraph_props, "algn") {
                Some("ctr") => paragraph_left + line_indent + (line_available - width) / 2.0,
                Some("r") => paragraph_left + line_indent + line_available - width,
                _ => paragraph_left + line_indent,
            };
            let baseline =
                cursor + baseline_offset(spacing_node("lnSpc"), ascent, line_height, advance);
            if line_index == 0 && !marker_glyphs.is_empty() {
                lines.push(Line {
                    glyphs: marker_glyphs.clone(),
                    x: paragraph_left + indent,
                    baseline,
                });
            }
            lines.push(Line {
                glyphs: glyph_line,
                x,
                baseline,
            });
            cursor += advance;
        }
        cursor += spacing(spacing_node("spcAft"), "spcAft", natural, 0.0);
    }
    let content_height = lines.iter().fold(cursor, |bottom, line| {
        bottom.max(line.baseline + line.glyphs.iter().map(|g| g.descent).fold(0.0, f64::max))
    });
    let anchor = source
        .cell_props
        .and_then(|n| n.attribute("anchor"))
        .or_else(|| attr(&body_props, "anchor"));
    let can_expand = autofit.expand
        && source.cell_props.is_none()
        && attr(&body_props, "vert").is_none_or(|s| s == "horz");
    if autofit.expand && !can_expand {
        warn(
            warnings,
            "Shape autofit for table cells or vertical text is unsupported",
        );
    }
    // Without wrapping, shape autofit also sets the width to the widest line. A body
    // without visible text keeps its saved width rather than collapsing to its insets.
    let fitted_width = if can_expand && fit_width && has_visible_text {
        left + right + natural_width
    } else {
        w
    };
    // A body without visible text keeps its saved size: the saved box is what Office
    // displays, and an inset text area can be far smaller than the shape.
    let expanded_height = if can_expand && has_visible_text {
        h.max(top + content_height + bottom)
    } else {
        h
    };
    let extent = TextExtent {
        height: expanded_height,
        offset_y: -(expanded_height - h)
            * match anchor {
                Some("ctr") => 0.5,
                Some("b") => 1.0,
                _ => 0.0,
            },
        required_height: top + content_height + bottom,
        width: fitted_width,
        offset_x: -(fitted_width - w)
            * match first_alignment {
                Some("ctr") => 0.5,
                Some("r") => 1.0,
                _ => 0.0,
            },
    };
    let h = expanded_height;
    // An autofit body without visible text keeps its saved height; its empty line
    // overflowing that box draws nothing, so it is not reported.
    if content_height > h - top - bottom + 0.01 && (has_visible_text || !autofit.expand) {
        warnings.push(Code::TextOverflow, "Text exceeds the text box height");
    }
    let vertical = match anchor {
        Some("ctr") => top + (h - top - bottom - content_height) / 2.0,
        Some("b") => h - bottom - content_height,
        Some("t") | None => top,
        _ => {
            warn(warnings, "Distributed vertical alignment is unsupported");
            top
        }
    };
    for line in lines {
        let mut x = line.x;
        let y = line.baseline + vertical;
        svg.push_str(&format!("<text y=\"{y}\" xml:space=\"preserve\">"));
        // Upright glyphs are separate elements, each turned back about its own centre.
        let mut upright = String::new();
        // Centre of the column across its width: the middle of the line's cell height.
        let ascent = line.glyphs.iter().map(|g| g.ascent).fold(0.0, f64::max);
        let descent = line.glyphs.iter().map(|g| g.descent).fold(0.0, f64::max);
        let column_center = y - (ascent - descent) / 2.0;

        // Advance past a segment, keeping the spacing to a next glyph of the same style
        // (segments also split where upright and sideways glyphs meet).
        let advance = |start: usize, end: usize| -> f64 {
            (start..end)
                .map(|i| step(&line.glyphs[i], line.glyphs.get(i + 1)))
                .sum()
        };
        let mut start = 0;
        while start < line.glyphs.len() {
            let style = &line.glyphs[start].style;
            let turned_back = line.glyphs[start].upright;
            let mut end = start + 1;
            while end < line.glyphs.len()
                && line.glyphs[end].style == *style
                && line.glyphs[end].upright == turned_back
            {
                end += 1;
            }
            let font = format!(
                "font-family=\"{}\" font-size=\"{}\" fill=\"{}\" font-weight=\"{}\" font-style=\"{}\"",
                escape(&style.face),
                style.size,
                style.color,
                if style.bold { "bold" } else { "normal" },
                if style.italic { "italic" } else { "normal" }
            );
            if turned_back {
                // Ideographic em box: 0.88 em above and 0.12 em below the baseline, so
                // its centre is 0.38 em above the baseline.
                let baseline = column_center + 0.38 * style.size;
                let mut position = x;
                for (i, g) in line.glyphs[start..end].iter().enumerate() {
                    let center = position + g.offset + g.advance / 2.0;
                    // Vertical forms of these marks sit in the upper-right quadrant.
                    let shift = if can_hang(g.ch) {
                        format!(" translate({} {})", style.size / 2.0, -style.size / 2.0)
                    } else {
                        String::new()
                    };
                    upright.push_str(&format!(
                        "<text x=\"{center}\" y=\"{baseline}\" text-anchor=\"middle\" transform=\"rotate(-90 {center} {column_center}){shift}\" {font}>{}</text>",
                        escape(&g.ch.to_string())
                    ));
                    position += step(
                        g,
                        line.glyphs
                            .get(start + i + 1)
                            .filter(|_| start + i + 1 < end),
                    );
                }
                x += advance(start, end);
                start = end;
                continue;
            }
            let positions = {
                let mut position = x;
                line.glyphs[start..end]
                    .iter()
                    .enumerate()
                    .map(|(i, g)| {
                        let result = (position + g.offset).to_string();
                        position += step(
                            g,
                            line.glyphs
                                .get(start + i + 1)
                                .filter(|_| start + i + 1 < end),
                        );
                        result
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let value: String = line.glyphs[start..end].iter().map(|g| g.ch).collect();
            svg.push_str(&format!(
                "<tspan x=\"{positions}\" {font}>{}</tspan>",
                escape(&value)
            ));
            x += advance(start, end);
            start = end;
        }
        svg.push_str("</text>");
        svg.push_str(&upright);
    }
    extent
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(value: &str) -> Vec<Glyph> {
        value
            .chars()
            .map(|ch| Glyph {
                ch,
                offset: 0.0,
                compressed: false,
                vertical_advance: None,
                upright: false,
                advance: 10.0,
                ascent: 8.0,
                descent: 2.0,
                leading: 0.0,
                style: Style {
                    face: "sans-serif".into(),
                    size: 10.0,
                    color: "#000000".into(),
                    bold: false,
                    italic: false,
                    spacing: 0.0,
                },
            })
            .collect()
    }
    fn strings(lines: Vec<Vec<Glyph>>) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.iter().map(|g| g.ch).collect())
            .collect()
    }
    #[test]
    fn twelve_point_boundary_uses_cumulative_width_before_kinsoku() {
        for text in ["あいうえお", "あいうえおか", "あ、いう、", "あ、いう、え"]
        {
            let mut glyphs = sample(text);
            for glyph in &mut glyphs {
                glyph.advance = 16.0;
                glyph.style.size = 16.0;
            }
            let below = strings(wrap(&glyphs, 59.95 * 4.0 / 3.0, true, false));
            assert!(below.len() > 1);
            for width in [59.98, 59.99, 60.0] {
                let lines = strings(wrap(&glyphs, width * 4.0 / 3.0, true, false));
                assert_eq!(lines[0], text.chars().take(5).collect::<String>());
                assert_eq!(lines.concat(), text);
            }
        }
        // Truncating each character before summing incorrectly fits overly narrow boxes.
        let mut glyphs = sample("あいうえお");
        for glyph in &mut glyphs {
            glyph.advance = 16.0;
            glyph.style.size = 16.0;
        }
        assert_eq!(
            strings(wrap(&glyphs, 59.95 * 4.0 / 3.0, true, false)),
            ["あいうえ", "お"]
        );
    }
    #[test]
    fn rounded_line_width_changes_only_at_layout_unit_boundary() {
        let mut glyphs = sample("あいうえおか");
        for glyph in &mut glyphs {
            glyph.advance = 32.0;
            glyph.style.size = 32.0;
        }
        assert_eq!(
            strings(wrap(&glyphs, 143.98 * 4.0 / 3.0, true, false)),
            ["あいうえお", "か"]
        );
        assert_eq!(
            strings(wrap(&glyphs, 143.99 * 4.0 / 3.0, true, false)),
            ["あいうえおか"]
        );
    }
    #[test]
    fn explicit_spacing_rounds_style_segments_before_adding_widths() {
        let mut glyphs = sample("あいう");
        for (index, glyph) in glyphs.iter_mut().enumerate() {
            glyph.advance = 32.0;
            glyph.style.size = 32.0;
            glyph.style.spacing = index as f64;
        }
        // All spans have different styles: no character spacing, but span rounding accumulates.
        assert_eq!(strings(wrap(&glyphs, 96.0, true, false)), ["あい", "う"]);
        assert_eq!(strings(wrap(&glyphs, 96.04, true, false)), ["あいう"]);
    }
    #[test]
    fn substituted_families_use_a_fixed_line_height_split_by_the_substitute_ratio() {
        let mut fonts = Database::new();
        fonts.load_font_data(include_bytes!("../../tests/fixtures/metrics.ttf").to_vec());
        let same = HashMap::from([(
            "Rhodopsin Metrics Test".to_string(),
            "Rhodopsin Metrics Test".to_string(),
        )]);
        let mapped = HashMap::from([(
            "Missing Family".to_string(),
            "Rhodopsin Metrics Test".to_string(),
        )]);
        let none = HashMap::new();
        // The fixture font's usWin cell is 0.8 em ascent and 0.2 em descent; a substitute
        // keeps that 4:1 split of a 1.2 em line.
        for (family, mapping, substituted) in [
            ("Rhodopsin Metrics Test", &none, false),
            ("Rhodopsin Metrics Test", &same, false),
            ("Missing Family", &none, true),
            ("Missing Family", &mapped, true),
        ] {
            let (ascent, descent) = if substituted {
                (0.96, 0.24)
            } else {
                (0.8, 0.2)
            };
            let mut style = sample("あ")[0].style.clone();
            style.face = family.into();
            style.size = 24.0;
            let mut warnings = Warnings::default();
            let measured = glyphs("あA", style, &Resolver::new(&fonts, mapping), &mut warnings);
            for glyph in measured {
                assert!((glyph.ascent - ascent * 24.0).abs() < 1e-9, "{family}");
                assert!((glyph.descent - descent * 24.0).abs() < 1e-9, "{family}");
                assert_eq!(glyph.leading, 0.0);
            }
        }
    }
    #[test]
    fn font_units_preserve_proportions_at_fractional_sizes() {
        let mut fonts = Database::new();
        fonts.load_font_data(include_bytes!("../../tests/fixtures/metrics.ttf").to_vec());
        for (size_pt, full) in [
            (12.0, 16.0),
            (18.0, 24.0),
            (20.0, 26.72),
            (21.5, 28.64),
            (24.0, 32.0),
            (32.0, 42.72),
        ] {
            let mut style = sample("あ")[0].style.clone();
            style.face = "Rhodopsin Metrics Test".into();
            style.size = size_pt * 4.0 / 3.0;
            let mut warnings = Warnings::default();
            let measured = glyphs(
                "あA \u{301}",
                style.clone(),
                &Resolver::new(&fonts, &HashMap::new()),
                &mut warnings,
            );
            assert!(warnings.is_empty(), "{warnings:?}");
            for (glyph, expected) in measured.iter().zip([full, full / 2.0, full / 4.0, 0.0]) {
                assert!((glyph.advance - expected).abs() < 1e-10);
                assert_eq!(glyph.style.size, style.size);
            }
            let missing = glyphs(
                "未",
                style.clone(),
                &Resolver::new(&fonts, &HashMap::new()),
                &mut warnings,
            );
            assert_eq!(missing[0].advance, style.size);
            assert!(warnings.iter().any(|w| w.contains("U+672A")));
        }
    }
    #[test]
    fn compression_capacity_is_truncated_per_character() {
        for (size_pt, below, fits) in [(24.0, 120.0, 120.01), (18.0, 90.01, 90.02)] {
            let mut glyphs = sample("あ、いう、え");
            for glyph in &mut glyphs {
                glyph.advance = size_pt * 4.0 / 3.0;
                glyph.style.size = glyph.advance;
            }
            assert_eq!(
                strings(compress_wrap(&glyphs, below * 4.0 / 3.0, false)).len(),
                2
            );
            assert_eq!(
                strings(compress_wrap(&glyphs, fits * 4.0 / 3.0, false)),
                ["あ、いう、え"]
            );
        }
    }
    #[test]
    fn wrapped_line_keeps_maximum_compression_at_expansion_boundary() {
        let mut glyphs = sample("あ、いう、え");
        for glyph in &mut glyphs {
            glyph.advance = 32.0;
            glyph.style.size = 32.0;
        }
        let before = compress_wrap(&glyphs, 119.9 * 4.0 / 3.0, false);
        assert_eq!(strings(before.clone()), ["あ、いう、", "え"]);
        assert!(before[0][1].advance > 31.9);
        for width in [119.95, 119.98, 120.0] {
            let lines = compress_wrap(&glyphs, width * 4.0 / 3.0, false);
            assert_eq!(strings(lines.clone()), ["あ、いう、", "え"]);
            assert_eq!(lines[0][1].advance, 16.0);
            assert_eq!(lines[0][4].advance, 16.0);
            assert_eq!(lines[1][0].advance, 32.0);
        }
        assert_eq!(compress_wrap(&glyphs, 120.01 * 4.0 / 3.0, false).len(), 1);
        // For a final line, compress only enough to fit the box.
        let last = compress_wrap(&glyphs[..5], 119.95 * 4.0 / 3.0, false);
        assert!(last[0][1].advance > 31.9);
    }
    #[test]
    fn automatic_compression_uses_only_the_required_punctuation_space() {
        let line = sample("あ、いう、え");
        let result = compress_wrap(&line, 55.0, false);
        assert_eq!(strings(result.clone()), ["あ、いう、え"]);
        assert_eq!(text_width(&result[0]), 55.0);
        assert_eq!(result[0][1].advance, 7.5);
        assert_eq!(result[0][4].advance, 7.5);
        assert_eq!(result[0][0].advance, 10.0);
        assert_eq!(
            strings(wrap(&line, 55.0, true, false)),
            ["あ、いう、", "え"]
        );
    }
    #[test]
    fn spacious_lines_and_uncompressible_characters_are_unchanged() {
        let line = sample("あ、いう。え");
        let result = compress_wrap(&line, 80.0, false);
        assert_eq!(text_width(&result[0]), 60.0);
        assert!(result[0].iter().all(|g| !g.compressed && g.offset == 0.0));
        let line = sample("あいうえお・？！");
        assert!(line.iter().all(|g| capacity(g) == 0.0));
        assert_eq!(
            strings(compress_wrap(&line, 35.0, false)),
            strings(wrap(&line, 35.0, true, false))
        );
    }
    #[test]
    fn compression_has_a_half_width_limit_and_keeps_explicit_breaks() {
        let line = sample("あ、いう、え");
        let result = compress_wrap(&line, 45.0, false);
        assert!(
            result
                .iter()
                .all(|line| line.iter().all(|g| g.advance >= 5.0))
        );
        assert_eq!(
            strings(compress_wrap(&sample("あ、\n\nい。"), 30.0, false)),
            ["あ、", "", "い。"]
        );
        let mut narrow = sample("、");
        narrow[0].advance = 5.0;
        assert_eq!(capacity(&narrow[0]), 0.0);
    }
    #[test]
    fn opening_bracket_removes_leading_space_without_scaling_the_glyph() {
        let line = compress_line(&sample("あ「い」"), 0.5);
        assert_eq!(line[1].offset, -2.5);
        assert_eq!(line[1].advance, 7.5);
        assert_eq!(line[1].style.size, 10.0);
        let leading = compress_line(&sample("「あ」"), 0.5);
        assert_eq!(leading[0].offset, 0.0);
    }
    #[test]
    fn compressed_positions_reach_svg_and_explicit_spacing_takes_priority() {
        for (spc, wrap_mode, expected, warn_spacing) in [
            ("0", "square", "0 10 17.5 27.5 37.5 45", false),
            ("-75", "square", "0 9 18 27 36 45", true),
            ("0", "none", "0 10 20 30 40 50", false),
        ] {
            let source = format!(
                r#"<a:txBody xmlns:a="{A}"><a:bodyPr lIns="0" rIns="0" wrap="{wrap_mode}"/><a:p><a:r><a:rPr sz="750" spc="{spc}"/><a:t>あ、いう、え</a:t></a:r></a:p></a:txBody>"#
            );
            let doc = Document::parse(&source).unwrap();
            let mut svg = String::new();
            let mut warnings = Warnings::default();
            render(
                doc.root_element(),
                55.0,
                100.0,
                &Theme::default(),
                &RenderOptions {
                    fonts: Resolver::new(&Database::new(), &HashMap::new()),
                    punctuation_compression: true,
                    slide_number: Default::default(),
                },
                &mut svg,
                &mut warnings,
            );
            assert!(svg.contains(&format!("<tspan x=\"{expected}\"")), "{svg}");
            assert_eq!(
                warnings
                    .iter()
                    .any(|s| s.contains("explicit spacing takes precedence")),
                warn_spacing
            );
            assert!(
                warnings
                    .iter()
                    .any(|s| s.contains("experimental rendering policy"))
            );
        }
    }
    #[test]
    fn signed_spacing_changes_width_and_wrap_without_trailing_space() {
        let mut glyphs = sample("あいうえ");
        for g in &mut glyphs {
            g.style.spacing = -5.0;
        }
        assert_eq!(text_width(&glyphs), 25.0);
        assert_eq!(strings(wrap(&glyphs, 20.0, true, false)), ["あいう", "え"]);
        for g in &mut glyphs {
            g.style.spacing = 2.0;
        }
        assert_eq!(text_width(&glyphs), 46.0);
        assert_eq!(strings(wrap(&glyphs, 22.0, true, false)), ["あい", "うえ"]);
        assert_eq!(text_width(&glyphs[..1]), 10.0);
    }
    #[test]
    fn spacing_stops_at_style_boundary_and_explicit_break() {
        let mut glyphs = sample("あ、、い");
        glyphs[1].style.spacing = -5.0;
        glyphs[2].style.spacing = -5.0;
        assert_eq!(text_width(&glyphs), 35.0);
        glyphs[2].style.color = "#ff0000".into();
        assert_eq!(text_width(&glyphs), 40.0);
        let mut glyphs = sample("あい\nうえ");
        for g in &mut glyphs {
            g.style.spacing = -5.0;
        }
        let lines = wrap(&glyphs, 100.0, true, false);
        assert_eq!(strings(lines.clone()), ["あい", "うえ"]);
        assert_eq!(text_width(&lines[0]), 15.0);
        assert_eq!(text_width(&lines[1]), 15.0);
    }
    #[test]
    fn hanging_uses_compressed_width() {
        let mut glyphs = sample("あいう、え");
        for g in &mut glyphs {
            g.style.spacing = -5.0;
        }
        let lines = wrap(&glyphs, 20.0, true, true);
        assert_eq!(strings(lines.clone()), ["あいう、", "え"]);
        assert_eq!(layout_width(&lines[0], 20.0, true), 20.0);
    }
    #[test]
    fn spacing_is_inherited_and_zero_cancels_it() {
        let source = format!(
            r#"<a:p xmlns:a="{A}"><a:defRPr spc="-600"/><a:rPr spc="0"/><a:rPr spc="300"/></a:p>"#
        );
        let doc = Document::parse(&source).unwrap();
        let nodes: Vec<_> = doc
            .root_element()
            .children()
            .filter(|n| n.is_element())
            .collect();
        let resolve = |nodes: &[Node<'_, '_>]| {
            style(
                nodes,
                &[],
                true,
                &Theme::default(),
                None,
                &mut Warnings::default(),
            )
            .spacing
        };
        assert_eq!(resolve(&[nodes[0]]), -8.0);
        assert_eq!(resolve(&[nodes[1], nodes[0]]), 0.0);
        assert_eq!(resolve(&[nodes[2], nodes[0]]), 4.0);
        assert_eq!(resolve(&[]), 0.0);
    }
    #[test]
    fn svg_positions_use_spacing_and_alignment() {
        // Verify SVG output with a deterministic 10px width when no fonts are registered.
        for (align, expected) in [
            ("l", "0 5 10 15"),
            ("ctr", "12.5 17.5 22.5 27.5"),
            ("r", "25 30 35 40"),
        ] {
            let source = format!(
                r#"<a:txBody xmlns:a="{A}"><a:bodyPr lIns="0" rIns="0"/><a:p><a:pPr algn="{align}"/><a:r><a:rPr sz="750" spc="-375"/><a:t>あ、、い</a:t></a:r></a:p></a:txBody>"#
            );
            let doc = Document::parse(&source).unwrap();
            let mut svg = String::new();
            render(
                doc.root_element(),
                50.0,
                100.0,
                &Theme::default(),
                &RenderOptions {
                    fonts: Resolver::new(&Database::new(), &HashMap::new()),
                    punctuation_compression: false,
                    slide_number: Default::default(),
                },
                &mut svg,
                &mut Warnings::default(),
            );
            assert!(svg.contains(&format!("<tspan x=\"{expected}\"")), "{svg}");
        }
    }
    #[test]
    fn excessive_negative_spacing_warns_instead_of_reversing_text() {
        let source = format!(
            r#"<a:txBody xmlns:a="{A}"><a:p><a:r><a:rPr sz="750" spc="-1500"/><a:t>あい</a:t></a:r></a:p></a:txBody>"#
        );
        let doc = Document::parse(&source).unwrap();
        let mut warnings = Warnings::default();
        render(
            doc.root_element(),
            50.0,
            100.0,
            &Theme::default(),
            &RenderOptions {
                fonts: Resolver::new(&Database::new(), &HashMap::new()),
                punctuation_compression: false,
                slide_number: Default::default(),
            },
            &mut String::new(),
            &mut warnings,
        );
        assert!(
            warnings
                .iter()
                .any(|s| s.contains("Negative character spacing exceeding character width"))
        );
    }
    #[test]
    fn closing_punctuation_does_not_begin_line() {
        assert_eq!(
            strings(wrap(&sample("あいう、え"), 30.0, true, false)),
            ["あい", "う、え"]
        );
    }
    #[test]
    fn opening_punctuation_does_not_end_line() {
        assert_eq!(
            strings(wrap(&sample("あい「うえ」"), 30.0, true, false)),
            ["あい", "「う", "え」"]
        );
    }
    #[test]
    fn explicit_break_and_empty_line_are_preserved() {
        assert_eq!(
            strings(wrap(&sample("あ\n\nい"), 100.0, true, false)),
            ["あ", "", "い"]
        );
    }
    #[test]
    fn no_wrap_keeps_long_line() {
        assert_eq!(
            strings(wrap(&sample("あいうえ"), 10.0, false, false)),
            ["あいうえ"]
        );
    }
    #[test]
    fn enabled_hanging_keeps_comma_with_previous_line() {
        let lines = wrap(&sample("あいう、え"), 30.0, true, true);
        assert_eq!(strings(lines.clone()), ["あいう、", "え"]);
        assert_eq!(layout_width(&lines[0], 30.0, true), 30.0);
        assert_eq!(layout_width(&lines[0], 30.0, false), 40.0);
    }
    #[test]
    fn hanging_does_not_allow_arbitrary_letters_to_overflow() {
        assert_eq!(
            strings(wrap(&sample("あいうえお"), 30.0, true, true)),
            ["あいう", "えお"]
        );
        assert_eq!(
            strings(wrap(&sample("あいう、、え"), 30.0, true, true)),
            ["あい", "う、、", "え"]
        );
    }
    #[test]
    fn explicit_false_overrides_inherited_hanging() {
        let source = format!(
            r#"<a:p xmlns:a="{A}"><a:pPr hangingPunct="0"/><a:lvl1pPr hangingPunct="true"/><a:defPPr hangingPunct="0"/></a:p>"#
        );
        let doc = Document::parse(&source).unwrap();
        let root = doc.root_element();
        let pp = child(root, A, "pPr");
        let level = child(root, A, "lvl1pPr");
        let default = child(root, A, "defPPr");
        assert!(!paragraph_hanging([pp, level, default], true));
        assert!(paragraph_hanging([None, level, default], false));
        assert!(!paragraph_hanging([None, None, default], true));
        assert!(paragraph_hanging([None, None, None], true));
    }
    #[test]
    fn fixed_spacing_changes_first_baseline_but_percentage_does_not() {
        let source = format!(
            r#"<a:p xmlns:a="{A}"><a:pPr><a:lnSpc><a:spcPts val="1800"/></a:lnSpc></a:pPr><a:pPr><a:lnSpc><a:spcPct val="200000"/></a:lnSpc></a:pPr></a:p>"#
        );
        let doc = Document::parse(&source).unwrap();
        let props: Vec<_> = doc
            .root_element()
            .children()
            .filter(|n| n.is_element())
            .collect();
        // Place an ascent-20/descent-5 cell in a fixed 24px line; preserve 5px below the baseline.
        let fixed = spacing(Some(props[0]), "lnSpc", 25.0, 25.0);
        assert_eq!(fixed, 24.0);
        assert_eq!(baseline_offset(Some(props[0]), 20.0, 25.0, fixed), 19.0);
        let percentage = spacing(Some(props[1]), "lnSpc", 25.0, 25.0);
        assert_eq!(percentage, 50.0);
        assert_eq!(
            baseline_offset(Some(props[1]), 20.0, 25.0, percentage),
            20.0
        );
        assert_eq!(baseline_offset(None, 20.0, 25.0, 25.0), 20.0);
    }
    #[test]
    fn inherited_run_properties_are_merged_per_attribute() {
        let source = format!(
            r#"<a:p xmlns:a="{A}"><a:pPr><a:defRPr sz="1200" b="1"><a:latin typeface="Example"/></a:defRPr></a:pPr><a:r><a:rPr b="0"/></a:r></a:p>"#
        );
        let doc = Document::parse(&source).unwrap();
        let defaults = doc
            .descendants()
            .find(|n| n.has_tag_name((A, "defRPr")))
            .unwrap();
        let run = doc
            .descendants()
            .find(|n| n.has_tag_name((A, "rPr")))
            .unwrap();
        let s = style(
            &[run, defaults],
            &[],
            false,
            &Theme::default(),
            None,
            &mut Warnings::default(),
        );
        assert_eq!(s.size, 16.0);
        assert!(!s.bold);
        assert_eq!(s.face, "Example");
    }
    fn inherited_fixture(paragraphs: &str, size: &str) -> (String, Warnings) {
        let source = format!(
            r#"<p:sp xmlns:p="{P}" xmlns:a="{A}"><p:nvSpPr><p:cNvSpPr txBox="1"/></p:nvSpPr><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0"/><a:lstStyle/>{paragraphs}</p:txBody><p:txStyles><p:otherStyle><a:lvl1pPr><a:defRPr sz="2400"/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sp>"#
        );
        let doc = Document::parse(&source).unwrap();
        let mut theme = Theme::default();
        theme.presentation = Some(format!(
            r#"<p:presentation xmlns:p="{P}" xmlns:a="{A}"><p:defaultTextStyle><a:lvl1pPr><a:defRPr sz="{size}"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:lvl1pPr></p:defaultTextStyle></p:presentation>"#
        ));
        let mut fonts = Database::new();
        fonts.load_font_data(include_bytes!("../../tests/fixtures/metrics.ttf").to_vec());
        let mut svg = String::new();
        let mut warnings = Warnings::default();
        render(
            TextBody {
                body: child(doc.root_element(), P, "txBody").unwrap(),
                inherited: vec![],
                master_style: child(doc.root_element(), P, "txStyles"),
                cell_props: None,
                style_ref: None,
            },
            100.0,
            90.0,
            &theme,
            &RenderOptions {
                fonts: Resolver::new(&fonts, &HashMap::new()),
                punctuation_compression: false,
                slide_number: Default::default(),
            },
            &mut svg,
            &mut warnings,
        );
        (svg, warnings)
    }
    #[test]
    fn textbox_uses_presentation_defaults_and_matches_explicit_runs() {
        let inherited =
            inherited_fixture(r#"<a:p><a:r><a:t>AAAA AAAA AAAA</a:t></a:r></a:p>"#, "900").0;
        let explicit = inherited_fixture(
            r#"<a:p><a:r><a:rPr sz="900"/><a:t>AAAA AAAA AAAA</a:t></a:r></a:p>"#,
            "1800",
        )
        .0;
        assert_eq!(inherited, explicit);
        assert!(inherited.contains("font-size=\"12\""));
    }
    #[test]
    fn paragraph_positions_include_spacing_breaks_and_empty_end_style() {
        for (line_spacing, expected) in [
            (
                r#"<a:spcPct val="150000"/>"#,
                vec![13.6, 31.6, 58.666666666666664],
            ),
            (
                r#"<a:spcPts val="1500"/>"#,
                vec![21.6, 41.6, 62.666666666666664],
            ),
        ] {
            let paragraph = format!(
                r#"<a:p><a:pPr><a:lnSpc>{line_spacing}</a:lnSpc><a:spcBef><a:spcPts val="300"/></a:spcBef><a:spcAft><a:spcPts val="200"/></a:spcAft></a:pPr><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r></a:p><a:p><a:endParaRPr sz="1500"/></a:p>"#
            );
            let (svg, _) = inherited_fixture(&paragraph, "900");
            let root = format!("<svg>{svg}</svg>");
            let doc = Document::parse(&root).unwrap();
            let positions: Vec<f64> = doc
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .map(|n| n.attribute("y").unwrap().parse().unwrap())
                .collect();
            for (actual, expected) in positions.iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-5, "{positions:?}");
            }
            assert_eq!(positions.len(), 3);
        }
    }
    #[test]
    fn overflowing_text_keeps_diagnostic_without_clipping() {
        let (svg, warnings) = inherited_fixture(
            &format!("<a:p><a:r><a:t>{}</a:t></a:r></a:p>", "A ".repeat(100)),
            "1800",
        );
        assert!(
            warnings
                .records
                .iter()
                .any(|d| d.code == Code::TextOverflow)
        );
        assert_eq!(svg.matches("A ").count(), 100);
    }
    fn autofit_fixture(
        mode: &str,
        inherited_mode: &str,
        anchor: &str,
        paragraphs: &str,
    ) -> (String, TextExtent, Warnings) {
        autofit_fixture_with_wrap(mode, inherited_mode, anchor, "square", paragraphs)
    }
    fn autofit_fixture_with_wrap(
        mode: &str,
        inherited_mode: &str,
        anchor: &str,
        wrap: &str,
        paragraphs: &str,
    ) -> (String, TextExtent, Warnings) {
        let xml = format!(
            r#"<root xmlns:a="{A}" xmlns:p="{P}"><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0" anchor="{anchor}" wrap="{wrap}">{mode}</a:bodyPr><a:lstStyle><a:lvl1pPr><a:defRPr sz="1500"><a:latin typeface="Rhodopsin Metrics Test"/></a:defRPr></a:lvl1pPr></a:lstStyle>{paragraphs}</p:txBody><p:txBody><a:bodyPr>{inherited_mode}</a:bodyPr></p:txBody></root>"#
        );
        let doc = Document::parse(&xml).unwrap();
        let bodies: Vec<_> = doc
            .root_element()
            .children()
            .filter(|n| n.is_element())
            .collect();
        let mut fonts = Database::new();
        fonts.load_font_data(include_bytes!("../../tests/fixtures/metrics.ttf").to_vec());
        let mut svg = String::new();
        let mut warnings = Warnings::default();
        let extent = render(
            TextBody {
                body: bodies[0],
                inherited: vec![bodies[1]],
                master_style: None,
                cell_props: None,
                style_ref: None,
            },
            200.0,
            20.0,
            &Theme::default(),
            &RenderOptions {
                fonts: Resolver::new(&fonts, &HashMap::new()),
                punctuation_compression: false,
                slide_number: Default::default(),
            },
            &mut svg,
            &mut warnings,
        );
        (svg, extent, warnings)
    }
    #[test]
    fn normal_autofit_scales_measurement_and_svg_together() {
        for (scale, expected_size) in [(100000, 20.0), (55000, 11.0), (92500, 18.5)] {
            let (svg, _, _) = autofit_fixture(
                &format!(r#"<a:normAutofit fontScale="{scale}"/>"#),
                "",
                "t",
                "<a:p><a:r><a:t>AA</a:t></a:r></a:p>",
            );
            let root = format!("<svg>{svg}</svg>");
            let doc = Document::parse(&root).unwrap();
            let span = doc.descendants().find(|n| n.has_tag_name("tspan")).unwrap();
            assert_eq!(
                span.attribute("font-size").unwrap().parse::<f64>().unwrap(),
                expected_size
            );
            let positions: Vec<f64> = span
                .attribute("x")
                .unwrap()
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            assert!((positions[1] - measurement_size(expected_size) / 2.0).abs() < 1e-6);
        }
    }
    #[test]
    fn line_spacing_reduction_subtracts_percentage_points_not_fixed_spacing() {
        for (spacing_xml, expected) in [
            (r#"<a:spcPct val="120000"/>"#, 20.0),
            (r#"<a:spcPts val="2400"/>"#, 32.0),
        ] {
            let p = format!(
                "<a:p><a:pPr><a:lnSpc>{spacing_xml}</a:lnSpc></a:pPr><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r></a:p>"
            );
            let (svg, _, _) =
                autofit_fixture(r#"<a:normAutofit lnSpcReduction="20000"/>"#, "", "t", &p);
            let root = format!("<svg>{svg}</svg>");
            let doc = Document::parse(&root).unwrap();
            let y: Vec<f64> = doc
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .map(|n| n.attribute("y").unwrap().parse().unwrap())
                .collect();
            assert_eq!(y[1] - y[0], expected);
        }
    }
    #[test]
    fn local_no_autofit_cancels_parent_and_shape_fit_preserves_anchor() {
        let paragraph = "<a:p><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r><a:br/><a:r><a:t>A</a:t></a:r></a:p>";
        let (svg, extent, _) = autofit_fixture(
            "<a:noAutofit/>",
            r#"<a:normAutofit fontScale="55000"/>"#,
            "t",
            paragraph,
        );
        assert!(svg.contains("font-size=\"20\""));
        assert_eq!(extent.height, 20.0);
        for (anchor, offset) in [("t", 0.0), ("ctr", -20.0), ("b", -40.0)] {
            let (_, extent, warnings) = autofit_fixture("<a:spAutoFit/>", "", anchor, paragraph);
            assert_eq!(extent.height, 60.0);
            assert_eq!(extent.offset_y, offset);
            assert!(
                !warnings
                    .records
                    .iter()
                    .any(|d| d.code == Code::TextOverflow)
            );
        }
    }
    #[test]
    fn unwrapped_shape_fit_measures_the_widest_line_and_alignment_offset() {
        // Metrics-font "A" is 10px at 15pt; the fixture box is 200px wide with zero insets.
        for (align, offset) in [("l", 0.0), ("ctr", 70.0), ("r", 140.0)] {
            let paragraphs = format!(
                r#"<a:p><a:pPr algn="{align}"/><a:r><a:t>AA</a:t></a:r></a:p><a:p><a:pPr algn="l"/><a:r><a:t>AAAAAA</a:t></a:r></a:p>"#
            );
            let (_, extent, warnings) =
                autofit_fixture_with_wrap("<a:spAutoFit/>", "", "t", "none", &paragraphs);
            assert!(
                !warnings
                    .iter()
                    .any(|w| w.contains("Horizontal shape autofit"))
            );
            assert_eq!(extent.width, 60.0);
            assert_eq!(extent.offset_x, offset);
        }
        let (_, extent, _) = autofit_fixture(
            "<a:spAutoFit/>",
            "",
            "t",
            "<a:p><a:r><a:t>AA</a:t></a:r></a:p>",
        );
        assert_eq!((extent.width, extent.offset_x), (200.0, 0.0));
    }
    #[test]
    fn rotated_text_swaps_the_box_and_maps_insets_to_the_turned_lines() {
        // 10px left inset, 20px top inset. Turned clockwise, the shape's top inset
        // becomes the lines' left inset and the right inset (0) their top one.
        for (vert, transform, first_x, baseline_shift, warned) in [
            ("vert", "translate(50 0) rotate(90)", 20.0, 0.0, false),
            ("vert270", "translate(0 100) rotate(-90)", 0.0, 10.0, false),
            ("eaVert", "translate(50 0) rotate(90)", 20.0, 0.0, false),
        ] {
            let source = format!(
                r#"<a:txBody xmlns:a="{A}"><a:bodyPr vert="{vert}" lIns="95250" tIns="190500" rIns="0" bIns="0"/><a:p><a:r><a:rPr sz="750"/><a:t>AB</a:t></a:r></a:p></a:txBody>"#
            );
            let doc = Document::parse(&source).unwrap();
            let mut svg = String::new();
            let mut warnings = Warnings::default();
            let extent = render(
                doc.root_element(),
                50.0,
                100.0,
                &Theme::default(),
                &RenderOptions {
                    fonts: Resolver::new(&Database::new(), &HashMap::new()),
                    punctuation_compression: false,
                    slide_number: Default::default(),
                },
                &mut svg,
                &mut warnings,
            );
            assert_eq!((extent.width, extent.height), (50.0, 100.0));
            if transform.is_empty() {
                assert!(!svg.contains("rotate("), "{svg}");
            } else {
                assert!(svg.contains(transform), "{svg}");
            }
            assert!(
                svg.contains(&format!("<tspan x=\"{first_x} {}\"", first_x + 10.0)),
                "{vert}: {svg}"
            );
            // Without fonts the ascent equals the 10px size.
            assert!(
                svg.contains(&format!("<text y=\"{}\"", 10.0 + baseline_shift)),
                "{vert}: {svg}"
            );
            assert_eq!(
                warnings
                    .iter()
                    .any(|w| w.contains("Vertical text is unsupported")),
                warned
            );
        }
    }
    /// Render an `eaVert` body with zero insets in a 50x100 box without fonts.
    fn east_asian_vertical(run: &str) -> (String, Warnings) {
        let source = format!(
            r#"<a:txBody xmlns:a="{A}"><a:bodyPr vert="eaVert" lIns="0" tIns="0" rIns="0" bIns="0"/><a:p><a:r>{run}</a:r></a:p></a:txBody>"#
        );
        let doc = Document::parse(&source).unwrap();
        let mut svg = String::new();
        let mut warnings = Warnings::default();
        render(
            doc.root_element(),
            50.0,
            100.0,
            &Theme::default(),
            &RenderOptions {
                fonts: Resolver::new(&Database::new(), &HashMap::new()),
                punctuation_compression: false,
                slide_number: Default::default(),
            },
            &mut svg,
            &mut warnings,
        );
        (svg, warnings)
    }
    /// Numeric value of `name="..."` in each element that starts with `prefix`.
    fn attributes(svg: &str, prefix: &str, name: &str) -> Vec<f64> {
        svg.split(prefix)
            .skip(1)
            .map(|rest| {
                let element = format!("{prefix}{rest}");
                let value = element.split(&format!(" {name}=\"")).nth(1).unwrap();
                value.split(['"', ' ']).next().unwrap().parse().unwrap()
            })
            .collect()
    }
    #[test]
    fn east_asian_vertical_text_fills_upright_columns_from_right_to_left() {
        // Fifteen 10px characters (10.08px measured) overflow the 100px column: nine
        // fit in the first column and six move to the second.
        let (svg, warnings) =
            east_asian_vertical(r#"<a:rPr sz="750"/><a:t>あいうえおかきくけこさしすせそ</a:t>"#);
        // Columns are clockwise lines, so a later line lies further left on the slide.
        assert!(
            svg.starts_with("<g transform=\"translate(50 0) rotate(90)\">"),
            "{svg}"
        );
        let upright: Vec<_> = svg.split("<text x=").skip(1).collect();
        assert_eq!(upright.len(), 15, "{svg}");
        assert!(
            upright
                .iter()
                .all(|e| e.contains("transform=\"rotate(-90 "))
        );
        let baselines = attributes(&svg, "<text x=", "y");
        assert!(baselines[..9].iter().all(|y| *y == baselines[0]), "{svg}");
        assert!(baselines[9..].iter().all(|y| *y == baselines[9]), "{svg}");
        assert!(baselines[9] > baselines[0], "{svg}");
        let centers = attributes(&svg, "<text x=", "x");
        assert!(centers[..9].windows(2).all(|w| w[1] > w[0]), "{svg}");
        assert_eq!(centers[9], centers[0], "{svg}");
        assert!(
            !warnings
                .iter()
                .any(|w| w.contains("Vertical text is unsupported"))
        );
    }
    #[test]
    fn east_asian_vertical_text_turns_latin_sideways_and_reports_approximations() {
        let (svg, warnings) = east_asian_vertical(r#"<a:rPr sz="750"/><a:t>日本ABC</a:t>"#);
        // Latin runs stay in the turned line; East Asian characters are turned back.
        assert!(svg.contains(">ABC</tspan>"), "{svg}");
        assert_eq!(svg.matches("rotate(-90 ").count(), 2, "{svg}");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("instead of using the font's vertical forms"))
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("Font has no vertical metrics"))
        );
        // Without upright characters there are no vertical metrics to miss.
        let (_, warnings) = east_asian_vertical(r#"<a:rPr sz="750"/><a:t>ABC</a:t>"#);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("instead of using the font's vertical forms"))
        );
        assert!(
            !warnings
                .iter()
                .any(|w| w.contains("Font has no vertical metrics"))
        );
    }
    #[test]
    fn east_asian_vertical_text_keeps_spacing_where_orientation_changes() {
        // spc="750" adds 10px after each character followed by one of the same style.
        let (svg, _) = east_asian_vertical(r#"<a:rPr sz="750" spc="750"/><a:t>日A</a:t>"#);
        let center = attributes(&svg, "<text x=", "x")[0];
        let latin = attributes(&svg, "<tspan x=", "x")[0];
        // The upright character is centred in its 10.08px advance.
        assert!((latin - (center - 5.04) - 20.08).abs() < 1e-9, "{svg}");
    }
    #[test]
    fn bullets_and_wrapped_lines_have_separate_starts() {
        for (level, margin, marker_x) in [(0, 190500, 0.0), (1, 381000, 20.0)] {
            let paragraph = format!(
                r#"<a:p><a:pPr lvl="{level}" marL="{margin}" indent="-190500"><a:buFont typeface="Rhodopsin Metrics Test"/><a:buChar char="A"/></a:pPr><a:r><a:t>AAAA AAAA AAAA AAAA AAAA</a:t></a:r></a:p>"#
            );
            let (svg, _) = inherited_fixture(&paragraph, "900");
            let root = format!("<svg>{svg}</svg>");
            let doc = Document::parse(&root).unwrap();
            let spans: Vec<_> = doc
                .descendants()
                .filter(|n| n.has_tag_name("tspan"))
                .collect();
            let x = |n: Node<'_, '_>| {
                n.attribute("x")
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse::<f64>()
                    .unwrap()
            };
            assert_eq!(spans[0].text(), Some("A"));
            assert_eq!(x(spans[0]), marker_x);
            assert!(spans.len() > 2);
            for span in &spans[1..] {
                assert_eq!(x(*span), margin as f64 / EMU);
            }
        }
    }
    #[test]
    fn empty_paragraphs_hide_markers_but_space_runs_show_them() {
        let bullet = r#"<a:pPr marL="190500" indent="-190500"><a:buFont typeface="Rhodopsin Metrics Test"/><a:buChar char="A"/></a:pPr>"#;
        let tail = r#"<a:p><a:r><a:t>B</a:t></a:r></a:p>"#;
        let marker_count = |svg: &str| {
            let root = format!("<svg>{svg}</svg>");
            let doc = Document::parse(&root).unwrap();
            let n = doc
                .descendants()
                .filter(|n| n.has_tag_name("tspan") && n.text() == Some("A"))
                .count();
            let y = doc
                .descendants()
                .filter(|n| n.has_tag_name("text"))
                .filter_map(|n| n.attribute("y").and_then(|v| v.parse::<f64>().ok()))
                .next_back();
            (n, y)
        };
        let base = format!("<a:p>{bullet}<a:endParaRPr/></a:p>{tail}");
        let (svg, _) = inherited_fixture(&base, "900");
        let (markers, y_empty) = marker_count(&svg);
        assert_eq!(markers, 0);
        let spaced = format!(r#"<a:p>{bullet}<a:r><a:t> </a:t></a:r></a:p>{tail}"#);
        let (svg, _) = inherited_fixture(&spaced, "900");
        let (markers, y_spaced) = marker_count(&svg);
        assert_eq!(markers, 1);
        let filled = format!(r#"<a:p>{bullet}<a:r><a:t>B</a:t></a:r></a:p>{tail}"#);
        let (svg, _) = inherited_fixture(&filled, "900");
        let (markers, y_filled) = marker_count(&svg);
        assert_eq!(markers, 1);
        // The empty paragraph keeps its line height.
        assert!(y_empty.is_some());
        assert_eq!(y_empty, y_filled);
        assert_eq!(y_spaced, y_filled);
    }
    #[test]
    fn non_bullet_indent_only_moves_first_line_and_keeps_hard_breaks() {
        let paragraph = r#"<a:p><a:pPr marL="190500" indent="190500"><a:buNone/></a:pPr><a:r><a:t>AAAA AAAA AAAA AAAA</a:t></a:r><a:br/><a:r><a:t>AAAA</a:t></a:r></a:p>"#;
        let (svg, _) = inherited_fixture(paragraph, "900");
        let root = format!("<svg>{svg}</svg>");
        let doc = Document::parse(&root).unwrap();
        let spans: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("tspan"))
            .collect();
        assert!(spans.len() > 2);
        for (i, span) in spans.iter().enumerate() {
            let x: f64 = span
                .attribute("x")
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(x, if i == 0 { 40.0 } else { 20.0 });
        }
        assert_eq!(
            spans.iter().filter_map(|n| n.text()).collect::<String>(),
            "AAAA AAAA AAAA AAAAAAAA"
        );
        assert_eq!(spans.last().unwrap().text(), Some("AAAA"));
    }
    #[test]
    fn inherited_bullet_can_be_cancelled_and_size_color_follow_choice() {
        let xml = format!(
            r#"<a:root xmlns:a="{A}"><a:pPr><a:buNone/><a:buFontTx/><a:buSzTx/><a:buClrTx/></a:pPr><a:lvl1pPr><a:buChar char="A"/><a:buFont typeface="不在"/><a:buSzPct val="200000"/><a:buClr><a:srgbClr val="FF0000"/></a:buClr></a:lvl1pPr></a:root>"#
        );
        let doc = Document::parse(&xml).unwrap();
        let nodes: Vec<_> = doc
            .root_element()
            .children()
            .filter(|n| n.is_element())
            .collect();
        let base = sample("A")[0].style.clone();
        let mut warnings = Warnings::default();
        let unchanged =
            bullets::style(&nodes, base.clone(), 0.55, &Theme::default(), &mut warnings);
        assert_eq!(unchanged, base);
        let changed = bullets::style(
            &nodes[1..],
            base.clone(),
            0.55,
            &Theme::default(),
            &mut warnings,
        );
        assert_eq!(changed.size, base.size * 2.0);
        assert_eq!(changed.color, "#FF0000");
        assert_eq!(changed.face, "不在");
        assert_eq!(
            bullets::Counters::default().label(&nodes, 0, &mut warnings),
            None
        );
    }
}
