//! Unicode の段落方向を解決し、書字方向・文字体系ごとの整形範囲を視覚順で返す。
use crate::xlsx::diagnostic::{Failure, Result};
use rustybuzz::{Direction, Script};
use std::ops::Range;
use unicode_script::{Script as UnicodeScript, UnicodeScript as _};

type Segment = (Range<usize>, Direction, Script);

pub(crate) fn has_rtl(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            unicode_bidi::bidi_class(ch),
            unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL | unicode_bidi::BidiClass::AN
        )
    })
}

pub(crate) fn segments(text: &str) -> Result<Vec<Segment>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let bidi = unicode_bidi::BidiInfo::new(text, None);
    if bidi.paragraphs.len() != 1 {
        return Err(Failure::unsupported("text_multiple_paragraphs"));
    }
    let (levels, ranges) = bidi.visual_runs(&bidi.paragraphs[0], 0..text.len());
    let mut result = Vec::new();
    for range in ranges {
        let rtl = levels[range.start].is_rtl();
        let direction = if rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        };
        let mut parts: Vec<(Range<usize>, UnicodeScript)> = Vec::new();
        let mut script = UnicodeScript::Common;
        let mut start = range.start;
        for (offset, ch) in text[range.clone()].char_indices() {
            let current = ch.script();
            if matches!(current, UnicodeScript::Common | UnicodeScript::Inherited) {
                continue;
            }
            if script != UnicodeScript::Common && script != current {
                parts.push((start..range.start + offset, script));
                start = range.start + offset;
            }
            script = current;
        }
        parts.push((start..range.end, script));
        if rtl {
            parts.reverse();
        }
        for (range, script) in parts {
            let tag = rustybuzz::ttf_parser::Tag::from_bytes(
                script.short_name().as_bytes().try_into().unwrap(),
            );
            let script = Script::from_iso15924_tag(tag)
                .ok_or_else(|| Failure::unsupported("text_script"))?;
            result.push((range, direction, script));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 同じ右から左方向でも文字体系を混同しない() {
        let text = "אבاب";
        let segments = segments(text).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments[0],
            (4..8, Direction::RightToLeft, rustybuzz::script::ARABIC)
        );
        assert_eq!(
            segments[1],
            (0..4, Direction::RightToLeft, rustybuzz::script::HEBREW)
        );
    }

    #[test]
    fn 結合文字と異体字セレクタを基底文字から分離しない() {
        let text = "x\u{301}見\u{e0100}A";
        let segments = segments(text).unwrap();
        assert_eq!(
            segments
                .iter()
                .map(|s| &text[s.0.clone()])
                .collect::<Vec<_>>(),
            ["x\u{301}", "見\u{e0100}", "A"]
        );
    }
}
