/// このプロジェクトのテスト用に組み立てる最小 sfnt。
/// 字形は空であり、実フォントの描画品質や shaping の一致を検証するものではない。
pub fn font(family: &str, bold: bool, italic: bool, embedding: u16) -> Vec<u8> {
    font_with_underline(family, bold, italic, embedding, None)
}
pub fn font_with_underline(
    family: &str,
    bold: bool,
    italic: bool,
    embedding: u16,
    underline: Option<(i16, i16)>,
) -> Vec<u8> {
    fn u16_at(out: &mut [u8], offset: usize, value: u16) {
        out[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    }
    fn u32_at(out: &mut [u8], offset: usize, value: u32) {
        out[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    let mut head = vec![0; 54];
    u32_at(&mut head, 0, 0x00010000);
    u32_at(&mut head, 12, 0x5f0f3cf5);
    u16_at(&mut head, 18, 1000);
    u16_at(&mut head, 44, u16::from(bold) | (u16::from(italic) << 1));
    let mut hhea = vec![0; 36];
    u32_at(&mut hhea, 0, 0x00010000);
    u16_at(&mut hhea, 4, 800);
    u16_at(&mut hhea, 6, (-200i16) as u16);
    u16_at(&mut hhea, 34, 97);
    let mut maxp = vec![0; 32];
    u32_at(&mut maxp, 0, 0x00010000);
    u16_at(&mut maxp, 4, 97);
    let mut os2 = vec![0; 96];
    u16_at(&mut os2, 0, 4);
    u16_at(&mut os2, 4, if bold { 700 } else { 400 });
    u16_at(&mut os2, 6, 5);
    u16_at(&mut os2, 8, embedding);
    u16_at(&mut os2, 62, (u16::from(bold) << 5) | u16::from(italic));
    let mut hmtx = vec![0; 97 * 4];
    for index in 0..97 {
        u16_at(&mut hmtx, index * 4, 600);
    }
    for digit in 0..10 {
        u16_at(&mut hmtx, (17 + digit) * 4, 500 + digit as u16 * 10);
    }
    let mut cmap = vec![0; 52];
    u16_at(&mut cmap, 2, 1);
    u16_at(&mut cmap, 4, 3);
    u16_at(&mut cmap, 6, 10);
    u32_at(&mut cmap, 8, 12);
    u16_at(&mut cmap, 12, 12);
    u32_at(&mut cmap, 16, 40);
    u32_at(&mut cmap, 24, 2);
    u32_at(&mut cmap, 28, 32);
    u32_at(&mut cmap, 32, 126);
    u32_at(&mut cmap, 36, 1);
    u32_at(&mut cmap, 40, 0x898b);
    u32_at(&mut cmap, 44, 0x898b);
    u32_at(&mut cmap, 48, 96);
    let names = [
        (1u16, family.to_owned()),
        (
            6u16,
            format!(
                "Test-{}-{}",
                if bold { "Bold" } else { "Regular" },
                if italic { "Italic" } else { "Upright" }
            ),
        ),
    ];
    let mut name = vec![0; 6 + names.len() * 12];
    u16_at(&mut name, 2, names.len() as u16);
    u16_at(&mut name, 4, (6 + names.len() * 12) as u16);
    let mut storage = Vec::new();
    for (index, (id, value)) in names.iter().enumerate() {
        let offset = 6 + index * 12;
        let encoded: Vec<_> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        u16_at(&mut name, offset, 3);
        u16_at(&mut name, offset + 2, 1);
        u16_at(&mut name, offset + 4, 0x409);
        u16_at(&mut name, offset + 6, *id);
        u16_at(&mut name, offset + 8, encoded.len() as u16);
        u16_at(&mut name, offset + 10, storage.len() as u16);
        storage.extend(encoded);
    }
    name.extend(storage);
    let mut tables = vec![
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", vec![0; 4]),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", vec![0; 98 * 2]),
        (b"maxp", maxp),
        (b"name", name),
    ];
    if let Some((position, thickness)) = underline {
        let mut post = vec![0; 32];
        u32_at(&mut post, 0, 0x00030000);
        u16_at(&mut post, 8, position as u16);
        u16_at(&mut post, 10, thickness as u16);
        tables.push((b"post", post));
    }
    let mut out = vec![0; 12 + tables.len() * 16];
    u32_at(&mut out, 0, 0x00010000);
    u16_at(&mut out, 4, tables.len() as u16);
    for (i, (tag, data)) in tables.into_iter().enumerate() {
        let offset = 12 + i * 16;
        out[offset..offset + 4].copy_from_slice(tag);
        let position = out.len() as u32;
        u32_at(&mut out, offset + 8, position);
        u32_at(&mut out, offset + 12, data.len() as u32);
        out.extend(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    out
}

/// 空輪郭の試験書体へ format 14 を追加する。実字形の見た目を検証するものではない。
#[allow(dead_code)]
pub fn font_with_variation(selector: u32, default: bool, glyph: u16) -> Vec<u8> {
    let source = font("異体字試験", false, false, 0);
    let count = u16::from_be_bytes(source[4..6].try_into().unwrap()) as usize;
    let records = &source[12..12 + count * 16];
    let mut result = source[..12 + count * 16].to_vec();
    for (i, record) in records.chunks_exact(16).enumerate() {
        let offset = u32::from_be_bytes(record[8..12].try_into().unwrap()) as usize;
        let length = u32::from_be_bytes(record[12..16].try_into().unwrap()) as usize;
        let mut table = source[offset..offset + length].to_vec();
        if &record[..4] == b"cmap" {
            // format 12 を20バイト目へ移し、Unicode variation の符号化レコードを加える。
            let mut cmap = vec![0; 20];
            cmap[2..4].copy_from_slice(&2u16.to_be_bytes());
            cmap[4..6].copy_from_slice(&3u16.to_be_bytes());
            cmap[6..8].copy_from_slice(&10u16.to_be_bytes());
            cmap[8..12].copy_from_slice(&20u32.to_be_bytes());
            cmap[14..16].copy_from_slice(&5u16.to_be_bytes());
            cmap.extend_from_slice(&table[12..]);
            let variation_offset = cmap.len() as u32;
            cmap[16..20].copy_from_slice(&variation_offset.to_be_bytes());
            let mut variation = vec![0; if default { 29 } else { 30 }];
            variation[..2].copy_from_slice(&14u16.to_be_bytes());
            let len = variation.len() as u32;
            variation[2..6].copy_from_slice(&len.to_be_bytes());
            variation[6..10].copy_from_slice(&1u32.to_be_bytes());
            variation[10..13].copy_from_slice(&selector.to_be_bytes()[1..]);
            let map_offset = if default { 13 } else { 17 };
            variation[map_offset..map_offset + 4].copy_from_slice(&21u32.to_be_bytes());
            variation[21..25].copy_from_slice(&1u32.to_be_bytes());
            variation[25..28].copy_from_slice(&0x898bu32.to_be_bytes()[1..]); // 見
            if !default {
                variation[28..30].copy_from_slice(&glyph.to_be_bytes());
            }
            cmap.extend(variation);
            table = cmap;
        }
        let position = result.len() as u32;
        result[12 + i * 16 + 8..12 + i * 16 + 12].copy_from_slice(&position.to_be_bytes());
        result[12 + i * 16 + 12..12 + i * 16 + 16]
            .copy_from_slice(&(table.len() as u32).to_be_bytes());
        result.extend(table);
        while !result.len().is_multiple_of(4) {
            result.push(0);
        }
    }
    result
}

#[allow(dead_code)]
pub fn font_with_svg_variation() -> Vec<u8> {
    let mut data = font_with_variation(0xe0100, false, 34);
    let count = u16::from_be_bytes(data[4..6].try_into().unwrap()) as usize;
    let record = 12 + count * 16;
    for i in 0..count {
        let offset = 12 + i * 16 + 8;
        let value = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) + 16;
        data[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
    data.splice(record..record, [0; 16]);
    data[4..6].copy_from_slice(&((count + 1) as u16).to_be_bytes());
    data[record..record + 4].copy_from_slice(b"SVG ");
    let offset = data.len() as u32;
    data[record + 8..record + 12].copy_from_slice(&offset.to_be_bytes());
    let mut svg = vec![0; 24];
    svg[2..6].copy_from_slice(&10u32.to_be_bytes());
    svg[10..12].copy_from_slice(&1u16.to_be_bytes());
    svg[12..14].copy_from_slice(&34u16.to_be_bytes());
    svg[14..16].copy_from_slice(&34u16.to_be_bytes());
    svg[16..20].copy_from_slice(&14u32.to_be_bytes());
    svg[20..24].copy_from_slice(&6u32.to_be_bytes());
    svg.extend_from_slice(b"<svg/>");
    data[record + 12..record + 16].copy_from_slice(&(svg.len() as u32).to_be_bytes());
    data.extend(svg);
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    data
}

/// 欧文とヘブライ文字を持つ非機密の合成書体。輪郭は空のまま。
#[allow(dead_code)]
pub fn bidi_font() -> Vec<u8> {
    let mut data = font("方向試験", false, false, 0);
    let table = data[12..]
        .chunks_exact(16)
        .find(|entry| &entry[..4] == b"cmap")
        .unwrap();
    let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
    data[offset + 40..offset + 44].copy_from_slice(&0x5d0u32.to_be_bytes());
    data[offset + 44..offset + 48].copy_from_slice(&0x5eau32.to_be_bytes());
    data[offset + 48..offset + 52].copy_from_slice(&57u32.to_be_bytes());
    data
}
