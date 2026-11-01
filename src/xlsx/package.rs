use crate::xlsx::diagnostic::{Failure, Result};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// XML の DOM とモデル用の余裕を残して、入力・展開・ノード数を個別に制限する。
#[derive(Clone, Debug)]
pub struct Limits {
    pub input_bytes: u64,
    pub expanded_bytes: u64,
    pub part_bytes: u64,
    pub entries: usize,
    pub xml_nodes: u32,
    pub cells: usize,
    pub image_dimension: u32,
    pub image_bytes: u64,
    pub total_image_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: 64 * 1024 * 1024,
            expanded_bytes: 128 * 1024 * 1024,
            part_bytes: 32 * 1024 * 1024,
            entries: 4096,
            xml_nodes: 500_000,
            cells: 250_000,
            image_dimension: 16_384,
            image_bytes: 32 * 1024 * 1024,
            total_image_bytes: 64 * 1024 * 1024,
        }
    }
}

pub struct Package {
    parts: BTreeMap<String, Vec<u8>>,
    nodes_limit: u32,
}

impl Package {
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        if !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("xlsx"))
        {
            return Err(Failure::input("input_extension_not_xlsx"));
        }
        let mut file = File::open(path).map_err(|_| Failure::input("input_unreadable"))?;
        let metadata = file
            .metadata()
            .map_err(|_| Failure::input("input_unreadable"))?;
        if !metadata.is_file() || metadata.len() > limits.input_bytes {
            return Err(Failure::input("input_size_limit"));
        }
        let mut signature = [0; 8];
        file.read_exact(&mut signature)
            .map_err(|_| Failure::input("invalid_package"))?;
        if signature == [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1] {
            return Err(Failure::input("encrypted_or_legacy_workbook"));
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|_| Failure::input("input_unreadable"))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|_| Failure::input("invalid_zip"))?;
        if archive.len() > limits.entries {
            return Err(Failure::input("zip_entry_limit"));
        }
        let mut parts = BTreeMap::new();
        let mut total = 0u64;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|_| Failure::input("unreadable_zip_entry"))?;
            let name = entry.name().to_owned();
            if name.starts_with('/')
                || name.contains('\\')
                || name.split('/').any(|s| s == ".." || s == ".")
            {
                return Err(Failure::input("invalid_part_path"));
            }
            if entry.is_dir() {
                continue;
            }
            if parts.contains_key(&name) {
                return Err(Failure::input("duplicate_part"));
            }
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| Failure::input("expanded_size_limit"))?;
            if entry.size() > limits.part_bytes || total > limits.expanded_bytes {
                return Err(Failure::input("expanded_size_limit"));
            }
            let mut bytes = Vec::new();
            // 宣言サイズだけでなく、実際に読み出すバイト数も制限する。
            (&mut entry)
                .take(limits.part_bytes.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|_| Failure::input("corrupt_zip_entry"))?;
            if bytes.len() as u64 != entry.size() {
                return Err(Failure::input("zip_size_mismatch"));
            }
            parts.insert(name, bytes);
        }
        Ok(Self {
            parts,
            nodes_limit: limits.xml_nodes,
        })
    }

    pub fn xml(&self, name: &str) -> Result<roxmltree::Document<'_>> {
        let bytes = self
            .parts
            .get(name)
            .ok_or_else(|| Failure::input("required_part_missing"))?;
        let xml =
            std::str::from_utf8(bytes).map_err(|_| Failure::input("xml_encoding_unsupported"))?;
        roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                allow_dtd: false,
                nodes_limit: self.nodes_limit,
                ..Default::default()
            },
        )
        .map_err(|_| Failure::input("invalid_xml"))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }

    pub fn bytes(&self, name: &str) -> Result<&[u8]> {
        self.parts
            .get(name)
            .map(Vec::as_slice)
            .ok_or_else(|| Failure::input("required_part_missing"))
    }
}

pub fn relationship_path(source: &str) -> String {
    match source.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{source}.rels"),
    }
}

/// OPC の内部参照を正規化する。実ファイルシステムにはアクセスしない。
pub fn resolve(source: &str, target: &str) -> Result<String> {
    if target.contains(['\\', ':', '?', '#', '%']) {
        return Err(Failure::input("invalid_relationship_target"));
    }
    let mut segments: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        source
            .rsplit_once('/')
            .map_or(Vec::new(), |(dir, _)| dir.split('/').collect())
    };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments
                    .pop()
                    .ok_or_else(|| Failure::input("invalid_relationship_target"))?;
            }
            s => segments.push(s),
        }
    }
    Ok(segments.join("/"))
}
