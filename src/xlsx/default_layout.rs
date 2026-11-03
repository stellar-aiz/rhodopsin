//! XLSX CLI の既定配置規則。Excel for Mac 16.105.3 が書き出した PDF から測った値で、
//! 測った条件（文字と標準スタイルが Noto Sans JP、標準スタイル 11 pt、文字 6〜40 pt の整数）
//! にだけ適用し、それ以外は推測せず cell_layout_unmeasured で止める。
//! 測定の記録は docs/history/20261102_xlsx-excel-mac-layout.md。
use crate::xlsx::{
    ConversionOptions, PreparedWorkbook,
    cell_text::{Frame, LineBox},
    diagnostic::{Failure, Result},
    page_borders::{Edge, Stroke},
    pdf::graphics::BoxRect,
    render::{Environment, Limits, Rendered},
    styles::{Font, FontScheme, LineStyle, Styles},
    theme::TextScript,
    workbook::{Cell, CellValue},
};

pub const MEASURED_FAMILY: &str = "Noto Sans JP";
const NORMAL_POINTS: u8 = 11;
const FIRST_POINTS: u8 = 6;
const LAST_POINTS: u8 = FIRST_POINTS + LINE_BOXES.len() as u8 - 1;
/// 文字サイズ 6〜40 pt の行箱（高さ、上端からベースラインまで）。単位は倍率適用前のポイント。
/// 高さは Excel が保存した自然な行の高さと下揃えの位置、ベースラインは上揃えの位置から測った。
const LINE_BOXES: [(u8, u8); 35] = [
    (13, 8),
    (14, 9),
    (15, 10),
    (16, 11),
    (18, 13),
    (19, 14),
    (20, 15),
    (22, 16),
    (23, 17),
    (24, 18),
    (28, 20),
    (29, 21),
    (30, 22),
    (31, 23),
    (33, 24),
    (34, 25),
    (37, 27),
    (39, 28),
    (40, 29),
    (41, 30),
    (42, 31),
    (44, 32),
    (45, 33),
    (48, 35),
    (50, 36),
    (51, 37),
    (52, 38),
    (54, 39),
    (55, 40),
    (58, 42),
    (59, 43),
    (61, 44),
    (62, 45),
    (63, 46),
    (65, 47),
];

fn unmeasured() -> Failure {
    Failure::unsupported("cell_layout_unmeasured")
}

/// 測った書体・文字サイズなら倍率適用前の文字サイズを返す。
fn measured_points(font: &Font) -> Result<u8> {
    let family = font.family.as_deref().unwrap_or_default();
    let points = font.size.unwrap_or_default();
    if font.scheme != FontScheme::None
        || !family.eq_ignore_ascii_case(MEASURED_FAMILY)
        || points.fract() != 0.0
        || !(f64::from(FIRST_POINTS)..=f64::from(LAST_POINTS)).contains(&points)
    {
        return Err(unmeasured());
    }
    Ok(points as u8)
}

fn font(styles: &Styles, index: usize) -> Result<&Font> {
    styles
        .fonts
        .get(index)
        .ok_or_else(|| Failure::input("font_style_index"))?
        .as_ref()
        .map_err(Clone::clone)
}

fn check_normal(styles: &Styles) -> Result<()> {
    let index = styles
        .normal_font
        .ok_or_else(|| Failure::unsupported("normal_style_font_unresolved"))?;
    if measured_points(font(styles, index)?)? != NORMAL_POINTS {
        return Err(unmeasured());
    }
    Ok(())
}

/// セルの文字サイズで決まる左右の余白と行箱を、印刷倍率を掛けて内容矩形へ適用する。
/// 左余白は 9・16・23・30・37 pt で 1 ずつ増え、右余白は左より 1 小さい。
fn frame(styles: &Styles, scale: f64, cell: &Cell, rect: BoxRect) -> Result<Frame> {
    let mut frame = Frame {
        content: rect,
        clip: rect,
        scale,
        baseline_step: None,
        line_box: None,
    };
    if matches!(cell.value, CellValue::Blank) {
        return Ok(frame);
    }
    // 書式付き文字列は cell_text が行箱のある frame で止める（rich_text_line_box）。
    let points = measured_points(font(styles, styles.validate_cell(cell.style)?.font)?)?;
    let left = f64::from((points + 5) / 7 + 1);
    let right = left - 1.0;
    let (height, baseline) = LINE_BOXES[usize::from(points - FIRST_POINTS)];
    frame.content.x += left * scale;
    frame.content.width -= (left + right) * scale;
    if frame.content.width <= 0.0 {
        return Err(unmeasured());
    }
    frame.line_box = Some(LineBox {
        height: f64::from(height) * scale,
        baseline: f64::from(baseline) * scale,
    });
    Ok(frame)
}

/// 細線・中線・太線の幅は 1・2・3 pt。その他の線種は測っていない。
fn stroke(scale: f64, edge: &Edge) -> Result<Stroke> {
    let width = match edge.style {
        LineStyle::Thin => 1.0,
        LineStyle::Medium => 2.0,
        LineStyle::Thick => 3.0,
        _ => {
            return Err(Failure::unsupported("border_style_unmeasured")
                .at(edge.sheet_index, Some(edge.cell.clone())));
        }
    };
    Ok(Stroke {
        width: width * scale,
        dash: Vec::new(),
        separation: None,
    })
}

/// 既定の配置規則で全ページを描画する。自動色は Excel の PDF と同じ黒。
pub fn render(
    prepared: &PreparedWorkbook,
    options: &ConversionOptions,
    limits: &Limits,
) -> Result<Rendered> {
    let styles = prepared
        .workbook
        .styles
        .as_ref()
        .ok_or_else(|| Failure::unsupported("cell_styles_unresolved"))?;
    check_normal(styles)?;
    let environment = Environment {
        locale: options.locale,
        // テーマの書体は measured_points が止めるため、文字体系は選択に使われない。
        script: TextScript::Latin,
        normal_script: TextScript::Latin,
        automatic_color: Some([0, 0, 0]),
        foreground: None,
        background: None,
    };
    prepared.render_with_borders(
        options,
        &environment,
        limits,
        |sheet, page, cell, placement| {
            frame(styles, page.scale, cell, placement.rect)
                .map_err(|error| error.at(sheet.index, Some(cell.address.clone())))
        },
        |_, page, edge| stroke(page.scale, edge),
    )
}
