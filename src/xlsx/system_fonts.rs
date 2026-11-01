//! 明示オプションで許可された場合だけ、OS 標準の探索先を追加する。
use crate::xlsx::diagnostic::{Category, Failure, Result};
use std::path::PathBuf;

fn candidates(os: &str, env: impl Fn(&str) -> Option<PathBuf>) -> Result<Vec<PathBuf>> {
    let absolute = |key| -> Result<Option<PathBuf>> {
        match env(key) {
            Some(path) if !path.is_absolute() => {
                Err(Failure::input("invalid_system_font_directory"))
            }
            path => Ok(path),
        }
    };
    let mut paths = match os {
        "macos" => {
            let mut paths = vec![
                PathBuf::from("/System/Library/Fonts"),
                PathBuf::from("/Library/Fonts"),
            ];
            if let Some(home) = absolute("HOME")? {
                paths.push(home.join("Library/Fonts"));
            }
            paths
        }
        "windows" => {
            let root = absolute("WINDIR")?.ok_or_else(|| {
                Failure::new(Category::MissingFont, "system_font_directory_unresolved")
            })?;
            let mut paths = vec![root.join("Fonts")];
            if let Some(local) = absolute("LOCALAPPDATA")? {
                paths.push(local.join("Microsoft/Windows/Fonts"));
            }
            paths
        }
        "linux" => {
            let mut paths = vec![
                PathBuf::from("/usr/share/fonts"),
                PathBuf::from("/usr/local/share/fonts"),
            ];
            let home = absolute("HOME")?;
            let data = absolute("XDG_DATA_HOME")?
                .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
            if let Some(data) = data {
                paths.push(data.join("fonts"));
            }
            if let Some(home) = home {
                paths.push(home.join(".fonts"));
            }
            paths
        }
        _ => return Err(Failure::unsupported("system_font_platform")),
    };
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn existing(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for path in paths {
        match path.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Failure::unsupported("font_directory_symlink"));
            }
            Ok(metadata) if metadata.is_dir() => result.push(path),
            Ok(_) => return Err(Failure::input("font_search_not_directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(Failure::new(
                    Category::MissingFont,
                    "font_directory_unreadable",
                ));
            }
        }
    }
    Ok(result)
}

/// 設定ファイル・フォントキャッシュ・レジストリは読まない。
/// 標準パス以外のインストール先は --font-dir で明示する。
pub fn directories() -> Result<Vec<PathBuf>> {
    existing(candidates(std::env::consts::OS, |key| {
        std::env::var_os(key).map(PathBuf::from)
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 各osの探索先を環境変数を書き換えず検査する() {
        let root = std::env::temp_dir().join("rhodopsin-xlsx-test");
        let env = |key: &str| Some(root.join(key));
        let mac = candidates("macos", env).unwrap();
        assert!(mac.contains(&root.join("HOME/Library/Fonts")));
        let win = candidates("windows", env).unwrap();
        assert!(win.contains(&root.join("WINDIR/Fonts")));
        assert!(win.contains(&root.join("LOCALAPPDATA/Microsoft/Windows/Fonts")));
        let linux = candidates("linux", env).unwrap();
        assert!(linux.contains(&root.join("XDG_DATA_HOME/fonts")));
        assert!(!linux.contains(&root.join("HOME/.local/share/fonts")));
        let linux = candidates("linux", |key| (key == "HOME").then(|| root.clone())).unwrap();
        assert!(linux.contains(&root.join(".local/share/fonts")));
        assert!(linux.contains(&root.join(".fonts")));
        assert!(candidates("linux", |_| Some("relative-secret".into())).is_err());
        assert!(candidates("windows", |_| None).is_err());
    }
    #[test]
    fn 存在しない標準先だけを除外しファイルを無視しない() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert_eq!(
            existing(vec![missing, dir.path().into()]).unwrap(),
            vec![dir.path().to_path_buf()]
        );
        let file = dir.path().join("file");
        std::fs::write(&file, []).unwrap();
        assert_eq!(
            existing(vec![file]).unwrap_err().reason,
            "font_search_not_directory"
        );
    }
}
