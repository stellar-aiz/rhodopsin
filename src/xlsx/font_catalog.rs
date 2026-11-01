//! sfnt の名前テーブルだけを読み、不要な書体の本体をメモリへ保持しない。
use crate::xlsx::diagnostic::{Failure, Result};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
};

fn bad() -> Failure {
    Failure::input("invalid_font_catalog")
}
fn read(
    file: &mut File,
    length: u64,
    offset: u64,
    count: usize,
    remaining: &mut usize,
) -> Result<Vec<u8>> {
    if offset
        .checked_add(count as u64)
        .is_none_or(|end| end > length)
    {
        return Err(bad());
    }
    *remaining = remaining
        .checked_sub(count)
        .ok_or_else(|| Failure::input("font_catalog_limit"))?;
    file.seek(SeekFrom::Start(offset)).map_err(|_| bad())?;
    let mut bytes = vec![0; count];
    file.read_exact(&mut bytes).map_err(|_| bad())?;
    Ok(bytes)
}
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("固定長ヘッダー検査済み"),
    )
}

pub fn matches(
    file: &mut File,
    families: &BTreeSet<String>,
    remaining: &mut usize,
) -> Result<bool> {
    let metadata = file.metadata().map_err(|_| bad())?;
    if !metadata.is_file() {
        return Err(bad());
    }
    let length = metadata.len();
    let header = read(file, length, 0, 12, remaining)?;
    let offsets = if &header[..4] == b"ttcf" {
        if !matches!(u32_at(&header, 4), 0x10000 | 0x20000) {
            return Err(bad());
        }
        let count = u32_at(&header, 8) as usize;
        if count == 0 || count > 1024 {
            return Err(Failure::input("font_catalog_face_limit"));
        }
        let bytes = read(file, length, 12, count * 4, remaining)?;
        (0..count)
            .map(|i| u64::from(u32_at(&bytes, i * 4)))
            .collect::<Vec<_>>()
    } else {
        vec![0]
    };
    let mut matched = false;
    for offset in offsets {
        let header = read(file, length, offset, 12, remaining)?;
        if !matches!(&header[..4], b"\0\x01\0\0" | b"OTTO" | b"true") {
            return Err(bad());
        }
        let count = usize::from(u16_at(&header, 4));
        if count > 4096 {
            return Err(Failure::input("font_catalog_table_limit"));
        }
        let tables = read(file, length, offset + 12, count * 16, remaining)?;
        let mut seen_name = false;
        for record in tables.chunks_exact(16) {
            if &record[..4] != b"name" {
                continue;
            }
            if seen_name {
                return Err(bad());
            }
            seen_name = true;
            let size = u32_at(record, 12) as usize;
            if size > 1024 * 1024 {
                return Err(Failure::input("font_catalog_name_limit"));
            }
            let bytes = read(file, length, u64::from(u32_at(record, 8)), size, remaining)?;
            let table = rustybuzz::ttf_parser::name::Table::parse(&bytes).ok_or_else(bad)?;
            for name in table.names {
                if matches!(name.name_id, 1 | 16)
                    && let Some(name) = crate::xlsx::font_names::decode(&name)
                    && families.contains(&name.to_ascii_lowercase())
                {
                    matched = true;
                }
            }
        }
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xlsx::fonts::{FontBook, FontLimits};
    mod support {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/xlsx_support/mod.rs"
        ));
    }

    #[test]
    fn 索引確認後にパスが差し替わっても元のハンドルから読む() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("font.ttf");
        let original = support::font("確認済み", false, false, 0);
        std::fs::write(&path, &original).unwrap();
        let mut file = File::open(&path).unwrap();
        assert!(
            matches(
                &mut file,
                &BTreeSet::from(["確認済み".into()]),
                &mut 100_000
            )
            .unwrap()
        );
        std::fs::rename(&path, dir.path().join("original.ttf")).unwrap();
        std::fs::write(&path, support::font("差し替え", false, false, 0)).unwrap();
        let mut book = FontBook::default();
        book.load_opened(&mut file, &FontLimits::default()).unwrap();
        assert_eq!(book.faces[0].data(), original);
        assert!(book.select("確認済み", false, false, None).is_ok());
        assert!(book.select("差し替え", false, false, None).is_err());
    }

    #[test]
    fn 索引を読んだ後も本体のサイズ上限を検査する() {
        let mut file = tempfile::tempfile().unwrap();
        use std::io::Write;
        file.write_all(&support::font("試験", false, false, 0))
            .unwrap();
        assert!(matches(&mut file, &BTreeSet::from(["試験".into()]), &mut 100_000).unwrap());
        let mut book = FontBook::default();
        let limits = FontLimits {
            file_bytes: 10,
            ..Default::default()
        };
        assert_eq!(
            book.load_opened(&mut file, &limits).unwrap_err().reason,
            "font_file_limit"
        );
        assert!(book.faces.is_empty());
    }
}
