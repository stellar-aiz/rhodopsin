use crate::xlsx::{
    diagnostic::{Failure, Result},
    styles::{Color, ColorSource},
    theme::Theme,
};

/// 自動色は用途（文字・罫線・背景）を知る呼び出し元が明示する。
/// OS のテーマや画面設定を読まず、入力と印刷コンテキストだけで解決する。
pub struct ColorContext<'a> {
    pub theme: Option<&'a Theme>,
    pub indexed: &'a [[u8; 3]],
    pub automatic: Option<[u8; 3]>,
    pub foreground: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
}

// ISO/IEC 29500 の既定インデックスパレット。
const PALETTE: [u32; 64] = [
    0x000000, 0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0xff00ff, 0x00ffff, 0x000000,
    0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0xff00ff, 0x00ffff, 0x800000, 0x008000,
    0x000080, 0x808000, 0x800080, 0x008080, 0xc0c0c0, 0x808080, 0x9999ff, 0x993366, 0xffffcc,
    0xccffff, 0x660066, 0xff8080, 0x0066cc, 0xccccff, 0x000080, 0xff00ff, 0xffff00, 0x00ffff,
    0x800080, 0x800000, 0x008080, 0x0000ff, 0x00ccff, 0xccffff, 0xccffcc, 0xffff99, 0x99ccff,
    0xff99cc, 0xcc99ff, 0xffcc99, 0x3366ff, 0x33cccc, 0x99cc00, 0xffcc00, 0xff9900, 0xff6600,
    0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399,
    0x333333,
];

impl Color {
    pub fn resolve(&self, context: &ColorContext<'_>) -> Result<[u8; 3]> {
        let rgb = match self.source {
            ColorSource::Rgb(rgb) => rgb,
            ColorSource::Theme(index) => *context
                .theme
                .ok_or_else(|| Failure::input("theme_missing"))?
                .colors
                .get(usize::from(index))
                .ok_or_else(|| Failure::input("theme_color_index"))?,
            ColorSource::Automatic => context
                .automatic
                .ok_or_else(|| Failure::unsupported("automatic_color_context"))?,
            ColorSource::Indexed(64) => context
                .foreground
                .ok_or_else(|| Failure::unsupported("system_foreground_color"))?,
            ColorSource::Indexed(65) => context
                .background
                .ok_or_else(|| Failure::unsupported("system_background_color"))?,
            ColorSource::Indexed(index) => {
                if context.indexed.is_empty() {
                    let value = *PALETTE
                        .get(usize::from(index))
                        .ok_or_else(|| Failure::input("indexed_color_index"))?;
                    [(value >> 16) as u8, (value >> 8) as u8, value as u8]
                } else {
                    *context
                        .indexed
                        .get(usize::from(index))
                        .ok_or_else(|| Failure::input("indexed_color_missing"))?
                }
            }
        };
        tint(rgb, self.tint)
    }
}

/// OOXML の tint は RGB の各チャンネルではなく HLS の明度に作用する。
/// 中間値は丸めず、最終 RGB の段階で 8 bit に量子化する。
pub fn tint(rgb: [u8; 3], amount: f64) -> Result<[u8; 3]> {
    if !amount.is_finite() || !(-1.0..=1.0).contains(&amount) {
        return Err(Failure::input("invalid_color_tint"));
    }
    if amount == 0.0 {
        return Ok(rgb);
    }
    let [r, g, b] = rgb.map(|c| f64::from(c) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let original_light = (max + min) / 2.0;
    let light = if amount < 0.0 {
        original_light * (1.0 + amount)
    } else {
        original_light * (1.0 - amount) + amount
    };
    let channel = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if delta == 0.0 {
        return Ok([channel(light); 3]);
    }
    let saturation = delta / (1.0 - (2.0 * original_light - 1.0).abs());
    let hue = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let chroma = (1.0 - (2.0 * light - 1.0).abs()) * saturation;
    let x = chroma * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
    let intermediate = match hue as u8 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    let offset = light - chroma / 2.0;
    Ok(intermediate.map(|c| channel(c + offset)))
}
