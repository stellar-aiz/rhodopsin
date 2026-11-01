//! 呼び出し元が許可したフォントの探索先だけを列挙する。
use crate::xlsx::{
    diagnostic::{Category, Failure, Result},
    fonts::{FontBook, FontLimits},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Default)]
pub struct Sources {
    pub files: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
}
pub struct SearchLimits {
    pub entries: usize,
    pub depth: usize,
}
impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            entries: 20_000,
            depth: 16,
        }
    }
}
fn unreadable() -> Failure {
    Failure::new(Category::MissingFont, "font_directory_unreadable")
}
fn walk(
    path: &Path,
    depth: usize,
    count: &mut usize,
    limits: &SearchLimits,
    files: &mut BTreeSet<PathBuf>,
    allowed: &[PathBuf],
) -> Result<()> {
    if depth > limits.depth {
        return Err(Failure::input("font_search_depth_limit"));
    }
    let metadata = path.symlink_metadata().map_err(|_| unreadable())?;
    if metadata.file_type().is_symlink() {
        return Err(Failure::unsupported("font_directory_symlink"));
    }
    if !metadata.is_dir() {
        return Err(Failure::input("font_search_not_directory"));
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|_| unreadable())? {
        *count = count
            .checked_add(1)
            .filter(|n| *n <= limits.entries)
            .ok_or_else(|| Failure::input("font_search_entry_limit"))?;
        entries.push(entry.map_err(|_| unreadable())?);
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type().map_err(|_| unreadable())?;
        let path = entry.path();
        let is_font = path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
            ["ttf", "ttc", "otf", "otc"]
                .iter()
                .any(|ext| s.eq_ignore_ascii_case(ext))
        });
        if kind.is_symlink() {
            let target = path
                .canonicalize()
                .map_err(|_| Failure::unsupported("font_directory_symlink"))?;
            if !is_font
                || !allowed.iter().any(|root| target.starts_with(root))
                || !target.metadata().map_err(|_| unreadable())?.is_file()
            {
                return Err(Failure::unsupported("font_directory_symlink"));
            }
            files.insert(target);
        } else if kind.is_dir() {
            walk(&path, depth + 1, count, limits, files, allowed)?;
        } else if kind.is_file() && is_font {
            files.insert(path);
        }
    }
    Ok(())
}

impl Sources {
    fn paths(&self, search: &SearchLimits) -> Result<BTreeSet<PathBuf>> {
        if self.files.len().saturating_add(self.directories.len()) > search.entries {
            return Err(Failure::input("font_search_entry_limit"));
        }
        let mut files: BTreeSet<_> = self.files.iter().cloned().collect();
        let mut count = self.files.len() + self.directories.len();
        let allowed: Vec<_> = self
            .directories
            .iter()
            .map(|path| path.canonicalize().map_err(|_| unreadable()))
            .collect::<Result<_>>()?;
        for path in self.directories.iter().collect::<BTreeSet<_>>() {
            walk(path, 0, &mut count, search, &mut files, &allowed)?;
        }
        // 同じファイルのリンク名・実体名・明示指定が重なっても一度だけ読み込む。
        let files: BTreeSet<_> = files
            .into_iter()
            .map(|path| {
                path.canonicalize()
                    .map_err(|_| Failure::new(Category::MissingFont, "font_file_unreadable"))
            })
            .collect::<Result<_>>()?;
        Ok(files)
    }

    /// 要求書体を含むファイルだけ本体を読む。探索順や候補の曖昧さは維持する。
    pub fn load_matching(
        &self,
        book: &mut FontBook,
        search: &SearchLimits,
        font_limits: &FontLimits,
        families: &BTreeSet<String>,
        catalog_bytes: usize,
    ) -> Result<()> {
        let files = self.paths(search)?;
        let families: BTreeSet<_> = families
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect();
        let mut remaining = catalog_bytes;
        let mut pending = book.clone();
        for file in files {
            if families.is_empty() {
                break;
            }
            let mut file = std::fs::File::open(&file)
                .map_err(|_| Failure::new(Category::MissingFont, "font_file_unreadable"))?;
            if crate::xlsx::font_catalog::matches(&mut file, &families, &mut remaining)? {
                pending.load_opened(&mut file, font_limits)?;
            }
        }
        *book = pending;
        Ok(())
    }

    /// 指定順によらずパス順で読む。探索・読み込みに失敗した場合、book は変更しない。
    pub fn load(
        &self,
        book: &mut FontBook,
        search: &SearchLimits,
        font_limits: &FontLimits,
    ) -> Result<()> {
        let files = self.paths(search)?;
        let mut pending = book.clone();
        for file in files {
            pending.load_file(&file, font_limits)?;
        }
        *book = pending;
        Ok(())
    }
}
