//! 座標・線種は配置層で確定させる。ここでは上端原点から PDF 座標へ変換する。
use super::{Limits, number};
use crate::xlsx::{
    diagnostic::{Failure, Result},
    drawing::Raster,
};
use miniz_oxide::deflate::compress_to_vec_zlib;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Ref};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq)]
pub struct BoxRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl BoxRect {
    pub(super) fn validate(&self) -> Result<()> {
        for value in [
            self.x,
            self.y,
            self.width,
            self.height,
            self.x + self.width,
            self.y + self.height,
        ] {
            number(value)?;
        }
        if self.width <= 0.0 || self.height <= 0.0 {
            return Err(Failure::input("invalid_pdf_rectangle"));
        }
        Ok(())
    }
}
#[derive(Clone)]
pub enum Paint<'a> {
    Fill {
        rect: BoxRect,
        color: [u8; 3],
    },
    Line {
        from: [f64; 2],
        to: [f64; 2],
        width: f64,
        color: [u8; 3],
        dash: Vec<f64>,
    },
    Image {
        rect: BoxRect,
        raster: &'a Raster,
        /// ページ上の可視領域。画像自体の倍率・原点には影響させない。
        clip: Option<BoxRect>,
    },
}
impl Paint<'_> {
    pub(super) fn validate(
        &self,
        image_bytes: &mut usize,
        seen: &mut BTreeSet<usize>,
        limits: &Limits,
    ) -> Result<()> {
        match self {
            Self::Fill { rect, .. } => rect.validate(),
            Self::Line {
                from,
                to,
                width,
                dash,
                ..
            } => {
                for value in from
                    .iter()
                    .chain(to)
                    .chain(std::iter::once(width))
                    .chain(dash)
                {
                    number(*value)?;
                }
                if *width <= 0.0 || dash.len() > 32 || dash.iter().any(|n| *n <= 0.0) {
                    return Err(Failure::input("invalid_pdf_line"));
                }
                Ok(())
            }
            Self::Image { rect, raster, clip } => {
                rect.validate()?;
                if let Some(clip) = clip {
                    clip.validate()?;
                }
                if raster.width == 0
                    || raster.height == 0
                    || raster.width > 16384
                    || raster.height > 16384
                {
                    return Err(Failure::input("invalid_pdf_image"));
                }
                let expected = u64::from(raster.width) * u64::from(raster.height) * 4;
                if expected != raster.rgba.len() as u64 {
                    return Err(Failure::input("invalid_pdf_image"));
                }
                if !seen.insert(*raster as *const Raster as usize) {
                    return Ok(());
                }
                *image_bytes = image_bytes
                    .checked_add(raster.rgba.len())
                    .filter(|n| *n <= limits.image_bytes)
                    .ok_or_else(|| Failure::input("pdf_image_limit"))?;
                Ok(())
            }
        }
    }
    pub(super) fn draw(
        &self,
        content: &mut Content,
        pdf: &mut Pdf,
        next: &mut i32,
        images: &mut Vec<(String, Ref)>,
        height: f64,
        cache: &mut BTreeMap<usize, Ref>,
    ) -> Result<()> {
        content.save_state();
        match self {
            Self::Fill { rect, color } => {
                content.set_fill_rgb(
                    f32::from(color[0]) / 255.0,
                    f32::from(color[1]) / 255.0,
                    f32::from(color[2]) / 255.0,
                );
                content
                    .rect(
                        number(rect.x)?,
                        number(height - rect.y - rect.height)?,
                        number(rect.width)?,
                        number(rect.height)?,
                    )
                    .fill_nonzero();
            }
            Self::Line {
                from,
                to,
                width,
                color,
                dash,
            } => {
                content.set_stroke_rgb(
                    f32::from(color[0]) / 255.0,
                    f32::from(color[1]) / 255.0,
                    f32::from(color[2]) / 255.0,
                );
                content
                    .set_line_width(number(*width)?)
                    .set_dash_pattern(dash.iter().map(|n| *n as f32), 0.0);
                content
                    .move_to(number(from[0])?, number(height - from[1])?)
                    .line_to(number(to[0])?, number(height - to[1])?)
                    .stroke();
            }
            Self::Image { rect, raster, clip } => {
                if let Some(clip) = clip {
                    content
                        .rect(
                            number(clip.x)?,
                            number(height - clip.y - clip.height)?,
                            number(clip.width)?,
                            number(clip.height)?,
                        )
                        .clip_nonzero()
                        .end_path();
                }
                // ポインタは同じ共有画像の識別だけに使い、PDF の順序や ID に含めない。
                let key = *raster as *const Raster as usize;
                let id = if let Some(id) = cache.get(&key) {
                    *id
                } else {
                    let id = Ref::new(*next);
                    *next += 1;
                    let transparent = raster.rgba.chunks_exact(4).any(|p| p[3] != 255);
                    let mask = if transparent {
                        let id = Ref::new(*next);
                        *next += 1;
                        Some(id)
                    } else {
                        None
                    };
                    let rgb: Vec<_> = raster
                        .rgba
                        .chunks_exact(4)
                        .flat_map(|p| p[..3].iter().copied())
                        .collect();
                    let rgb = compress_to_vec_zlib(&rgb, 6);
                    let mut image = pdf.image_xobject(id, &rgb);
                    image.filter(Filter::FlateDecode);
                    image
                        .width(raster.width as i32)
                        .height(raster.height as i32)
                        .bits_per_component(8)
                        .interpolate(false);
                    image.color_space().device_rgb();
                    if let Some(mask) = mask {
                        image.s_mask(mask);
                    }
                    image.finish();
                    if let Some(mask) = mask {
                        let alpha: Vec<_> = raster.rgba.chunks_exact(4).map(|p| p[3]).collect();
                        let alpha = compress_to_vec_zlib(&alpha, 6);
                        let mut image = pdf.image_xobject(mask, &alpha);
                        image.filter(Filter::FlateDecode);
                        image
                            .width(raster.width as i32)
                            .height(raster.height as i32)
                            .bits_per_component(8)
                            .interpolate(false);
                        image.color_space().device_gray();
                        image.finish();
                    }
                    cache.insert(key, id);
                    id
                };
                let name = format!("Im{}", images.len());
                content
                    .transform([
                        number(rect.width)?,
                        0.0,
                        0.0,
                        number(rect.height)?,
                        number(rect.x)?,
                        number(height - rect.y - rect.height)?,
                    ])
                    .x_object(Name(name.as_bytes()));
                images.push((name, id));
            }
        }
        content.restore_state();
        Ok(())
    }
}
