//! フォント名レコードを、プラットフォーム・符号化・言語に従って復号する。
use rustybuzz::ttf_parser::{PlatformId, name::Name};

pub fn decode(name: &Name<'_>) -> Option<String> {
    if name.is_unicode() || (name.platform_id == PlatformId::Windows && name.encoding_id == 10) {
        if !name.name.len().is_multiple_of(2) {
            return None;
        }
        let units: Vec<_> = name
            .name
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        return String::from_utf16(&units).ok();
    }
    // Roman の言語別変種を混同しない。まず英語レコードだけを対象とする。
    if name.platform_id == PlatformId::Macintosh && name.encoding_id == 0 && name.language_id == 0 {
        return Some(
            name.name
                .iter()
                .map(|byte| {
                    if *byte < 128 {
                        char::from(*byte)
                    } else {
                        MAC_ROMAN[usize::from(*byte - 128)]
                    }
                })
                .collect(),
        );
    }
    None
}

// Mac OS Roman の 0x80..0xFF。現行の 0xDB はユーロ記号。
const MAC_ROMAN: [char; 128] = [
    '\u{c4}', '\u{c5}', '\u{c7}', '\u{c9}', '\u{d1}', '\u{d6}', '\u{dc}', '\u{e1}', '\u{e0}',
    '\u{e2}', '\u{e4}', '\u{e3}', '\u{e5}', '\u{e7}', '\u{e9}', '\u{e8}', '\u{ea}', '\u{eb}',
    '\u{ed}', '\u{ec}', '\u{ee}', '\u{ef}', '\u{f1}', '\u{f3}', '\u{f2}', '\u{f4}', '\u{f6}',
    '\u{f5}', '\u{fa}', '\u{f9}', '\u{fb}', '\u{fc}', '\u{2020}', '\u{b0}', '\u{a2}', '\u{a3}',
    '\u{a7}', '\u{2022}', '\u{b6}', '\u{df}', '\u{ae}', '\u{a9}', '\u{2122}', '\u{b4}', '\u{a8}',
    '\u{2260}', '\u{c6}', '\u{d8}', '\u{221e}', '\u{b1}', '\u{2264}', '\u{2265}', '\u{a5}',
    '\u{b5}', '\u{2202}', '\u{2211}', '\u{220f}', '\u{3c0}', '\u{222b}', '\u{aa}', '\u{ba}',
    '\u{3a9}', '\u{e6}', '\u{f8}', '\u{bf}', '\u{a1}', '\u{ac}', '\u{221a}', '\u{192}', '\u{2248}',
    '\u{2206}', '\u{ab}', '\u{bb}', '\u{2026}', '\u{a0}', '\u{c0}', '\u{c3}', '\u{d5}', '\u{152}',
    '\u{153}', '\u{2013}', '\u{2014}', '\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}', '\u{f7}',
    '\u{25ca}', '\u{ff}', '\u{178}', '\u{2044}', '\u{20ac}', '\u{2039}', '\u{203a}', '\u{fb01}',
    '\u{fb02}', '\u{2021}', '\u{b7}', '\u{201a}', '\u{201e}', '\u{2030}', '\u{c2}', '\u{ca}',
    '\u{c1}', '\u{cb}', '\u{c8}', '\u{cd}', '\u{ce}', '\u{cf}', '\u{cc}', '\u{d3}', '\u{d4}',
    '\u{f8ff}', '\u{d2}', '\u{da}', '\u{db}', '\u{d9}', '\u{131}', '\u{2c6}', '\u{2dc}', '\u{af}',
    '\u{2d8}', '\u{2d9}', '\u{2da}', '\u{b8}', '\u{2dd}', '\u{2db}', '\u{2c7}',
];

#[cfg(test)]
mod tests {
    use super::*;
    fn record(
        platform_id: PlatformId,
        encoding_id: u16,
        language_id: u16,
        bytes: &[u8],
    ) -> Name<'_> {
        Name {
            platform_id,
            encoding_id,
            language_id,
            name_id: 1,
            name: bytes,
        }
    }
    #[test]
    fn 英語macromanのasciiと拡張文字を復号する() {
        assert_eq!(
            decode(&record(PlatformId::Macintosh, 0, 0, b"Caf\x8e")),
            Some("Café".into())
        );
        assert_eq!(
            decode(&record(PlatformId::Macintosh, 0, 0, &[0xdb])),
            Some("€".into())
        );
        assert!(decode(&record(PlatformId::Macintosh, 0, 15, b"Caf\x8e")).is_none());
        assert!(decode(&record(PlatformId::Macintosh, 1, 0, b"Font")).is_none());
    }
    #[test]
    fn unicode名を厳密に復号し不正な末尾を捨てない() {
        let bytes: Vec<_> = "試験𠮷".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(
            decode(&record(PlatformId::Windows, 10, 0x409, &bytes)),
            Some("試験𠮷".into())
        );
        for bytes in [&[0, 65, 0][..], &[0xd8, 0][..]] {
            assert!(decode(&record(PlatformId::Windows, 1, 0x409, bytes)).is_none());
        }
    }
}
