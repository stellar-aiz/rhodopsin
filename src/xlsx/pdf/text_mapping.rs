//! クラスタから字形ごとの ToUnicode 対応を作る。推測した文字の割当はしない。
use crate::xlsx::{
    diagnostic::{Failure, Result},
    fonts::Glyph,
};
use rustybuzz::ttf_parser::Face;

/// 一意な字形対応を優先し、置換を伴うクラスタには元文字列をまとめて対応させる。
/// 空文字列の字形はToUnicode項目を作らず、呼び出し元がActualTextを必ず付ける。
pub(super) fn mapped_values<'a>(
    text: &'a str,
    glyphs: &[Glyph],
    face: &Face<'_>,
) -> Result<(Vec<(usize, &'a str)>, bool)> {
    match unicode_values(text, glyphs, face) {
        Ok(values) => return Ok((values, false)),
        Err(error) if error.reason == "pdf_multi_glyph_cluster" => {}
        Err(error) => return Err(error),
    }
    let clusters: std::collections::BTreeSet<_> = glyphs.iter().map(|g| g.cluster).collect();
    if clusters.first() != Some(&0)
        || clusters
            .iter()
            .any(|offset| *offset >= text.len() || !text.is_char_boundary(*offset))
    {
        return Err(Failure::input("invalid_pdf_cluster"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut values = Vec::with_capacity(glyphs.len());
    let mut start = 0;
    while start < glyphs.len() {
        let cluster = glyphs[start].cluster;
        if !seen.insert(cluster) {
            return Err(Failure::input("invalid_pdf_cluster"));
        }
        let mut next = start + 1;
        while next < glyphs.len() && glyphs[next].cluster == cluster {
            next += 1;
        }
        let end = clusters
            .range((
                std::ops::Bound::Excluded(cluster),
                std::ops::Bound::Unbounded,
            ))
            .next()
            .copied()
            .unwrap_or(text.len());
        let value = text
            .get(cluster..end)
            .ok_or_else(|| Failure::input("invalid_pdf_cluster"))?;
        let local: Vec<_> = glyphs[start..next]
            .iter()
            .cloned()
            .map(|mut glyph| {
                glyph.cluster = 0;
                glyph
            })
            .collect();
        match unicode_values(value, &local, face) {
            Ok(mapped) => values.extend(
                mapped
                    .into_iter()
                    .map(|(offset, value)| (cluster + offset, value)),
            ),
            Err(error) if error.reason == "pdf_multi_glyph_cluster" => {
                values.push((cluster, value));
                values.extend((start + 1..next).map(|_| (cluster, "")));
            }
            Err(error) => return Err(error),
        }
        start = next;
    }
    Ok((values, true))
}

pub(super) fn unicode_values<'a>(
    text: &'a str,
    glyphs: &[Glyph],
    face: &Face<'_>,
) -> Result<Vec<(usize, &'a str)>> {
    if glyphs.is_empty() {
        return if text.is_empty() {
            Ok(Vec::new())
        } else {
            Err(Failure::input("invalid_pdf_cluster"))
        };
    }
    let clusters: std::collections::BTreeSet<_> =
        glyphs.iter().map(|glyph| glyph.cluster).collect();
    if clusters.first() != Some(&0) {
        return Err(Failure::input("invalid_pdf_cluster"));
    }
    let mut result = Vec::with_capacity(glyphs.len());
    let mut seen = std::collections::BTreeSet::new();
    let mut start = 0;
    while start < glyphs.len() {
        let cluster = glyphs[start].cluster;
        if !seen.insert(cluster) {
            return Err(Failure::input("invalid_pdf_cluster"));
        }
        let mut next = start + 1;
        while next < glyphs.len() && glyphs[next].cluster == cluster {
            next += 1;
        }
        let end = clusters
            .range((
                std::ops::Bound::Excluded(cluster),
                std::ops::Bound::Unbounded,
            ))
            .next()
            .copied()
            .unwrap_or(text.len());
        let value = text
            .get(cluster..end)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Failure::input("invalid_pdf_cluster"))?;
        if next == start + 1 {
            // 合字を含め、一字形に複数 Unicode 文字を対応させられる。
            result.push((cluster, value));
        } else {
            // 字形位置は shaping の結果を使い、対応が確定する文字だけ元順へ戻す。
            let characters: Vec<_> = value.char_indices().collect();
            let group = &glyphs[start..next];
            if characters.len() != group.len() {
                return Err(Failure::unsupported("pdf_multi_glyph_cluster"));
            }
            let direct = characters.iter().zip(group).all(|((_, ch), glyph)| {
                face.glyph_index(*ch)
                    .is_some_and(|id| u32::from(id.0) == glyph.id)
            });
            if direct {
                result.extend(characters.iter().map(|(offset, ch)| {
                    (cluster + offset, &value[*offset..offset + ch.len_utf8()])
                }));
            } else {
                // 異なる Unicode が同じ字形を指す場合は、並べ替え後の対応を推測しない。
                // 同一文字の繰り返しだけは元順のキューで対応する。
                let mut by_glyph = std::collections::BTreeMap::new();
                for (offset, ch) in characters {
                    let id = face
                        .glyph_index(ch)
                        .ok_or_else(|| Failure::unsupported("pdf_multi_glyph_cluster"))?;
                    let entry = by_glyph
                        .entry(u32::from(id.0))
                        .or_insert_with(|| (ch, std::collections::VecDeque::new()));
                    if entry.0 != ch {
                        return Err(Failure::unsupported("pdf_multi_glyph_cluster"));
                    }
                    entry.1.push_back(offset);
                }
                for glyph in group {
                    let (ch, offsets) = by_glyph
                        .get_mut(&glyph.id)
                        .ok_or_else(|| Failure::unsupported("pdf_multi_glyph_cluster"))?;
                    let offset = offsets
                        .pop_front()
                        .ok_or_else(|| Failure::unsupported("pdf_multi_glyph_cluster"))?;
                    result.push((cluster + offset, &value[offset..offset + ch.len_utf8()]));
                }
            }
        }
        start = next;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    mod support {
        include!("../../../tests/xlsx_support/mod.rs");
    }

    fn glyph(id: u32, cluster: usize) -> Glyph {
        Glyph {
            id,
            cluster,
            advance_x: 1.0,
            advance_y: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    #[test]
    fn 同一文字の繰り返しと複数クラスタの元位置を保持する() {
        let data = support::font("試験", false, false, 0);
        let face = Face::parse(&data, 0).unwrap();
        let id = |ch| u32::from(face.glyph_index(ch).unwrap().0);
        let glyphs = [
            glyph(id('b'), 0),
            glyph(id('a'), 0),
            glyph(id('a'), 0),
            glyph(id('c'), 3),
        ];
        assert_eq!(
            unicode_values("aabc", &glyphs, &face).unwrap(),
            [(2, "b"), (0, "a"), (1, "a"), (3, "c")]
        );
    }

    #[test]
    fn 字形置換や件数違いを誤対応させない() {
        let data = support::font("試験", false, false, 0);
        let face = Face::parse(&data, 0).unwrap();
        let id = |ch| u32::from(face.glyph_index(ch).unwrap().0);
        for glyphs in [
            vec![glyph(id('b'), 0), glyph(id('c'), 0)],
            vec![glyph(id('a'), 0), glyph(id('a'), 0)],
            vec![glyph(id('a'), 0), glyph(id('b'), 0), glyph(id('b'), 0)],
        ] {
            assert_eq!(
                unicode_values("ab", &glyphs, &face).unwrap_err().reason,
                "pdf_multi_glyph_cluster"
            );
        }
        assert_eq!(
            unicode_values("ab", &[glyph(id('a'), 1)], &face)
                .unwrap_err()
                .reason,
            "invalid_pdf_cluster"
        );
    }

    #[test]
    fn 異なる文字が同じ字形なら並べ替え後の対応を推測しない() {
        let mut data = support::font("試験", false, false, 0);
        let entry = data[12..]
            .chunks_exact(16)
            .find(|entry| &entry[..4] == b"cmap")
            .unwrap();
        let offset = u32::from_be_bytes(entry[8..12].try_into().unwrap()) as usize;
        // 見と A を同じ字形へ対応させる。
        data[offset + 48..offset + 52].copy_from_slice(&34u32.to_be_bytes());
        let face = Face::parse(&data, 0).unwrap();
        let b = u32::from(face.glyph_index('b').unwrap().0);
        assert_eq!(
            unicode_values("A見b", &[glyph(b, 0), glyph(34, 0), glyph(34, 0)], &face)
                .unwrap_err()
                .reason,
            "pdf_multi_glyph_cluster"
        );
    }
    #[test]
    fn 未分解クラスタだけを元文字列とactualtextへ委ねる() {
        let data = support::font("試験", false, false, 0);
        let face = Face::parse(&data, 0).unwrap();
        let id = |ch| u32::from(face.glyph_index(ch).unwrap().0);
        // abは字形置換を仮定。cdは一意な並べ替えなので個別対応を保持する。
        let glyphs = [
            glyph(id('x'), 0),
            glyph(id('y'), 0),
            glyph(id('d'), 2),
            glyph(id('c'), 2),
        ];
        let (values, actual) = mapped_values("abcd", &glyphs, &face).unwrap();
        assert!(actual);
        assert_eq!(values, [(0, "ab"), (0, ""), (3, "d"), (2, "c")]);
        let direct = [glyph(id('a'), 0), glyph(id('b'), 1)];
        assert_eq!(
            mapped_values("ab", &direct, &face).unwrap(),
            (vec![(0, "a"), (1, "b")], false)
        );
    }

    #[test]
    fn 未分解クラスタの後ろの不正位置も拒否する() {
        let data = support::font("試験", false, false, 0);
        let face = Face::parse(&data, 0).unwrap();
        for glyphs in [
            vec![glyph(10, 0), glyph(11, 0), glyph(12, 3), glyph(13, 0)],
            vec![glyph(10, 0), glyph(11, 0), glyph(12, 99)],
            vec![glyph(10, 0), glyph(11, 0), glyph(12, 1)],
        ] {
            assert_eq!(
                mapped_values("見ab", &glyphs, &face).unwrap_err().reason,
                "invalid_pdf_cluster"
            );
        }
    }
}
