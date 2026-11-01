//! 生成と検証が済んだバイト列を、既存出力を置き換えずに確定する。
use crate::xlsx::diagnostic::{Category, Failure, Result};
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};

pub fn check_available(output: &Path) -> Result<()> {
    match output.symlink_metadata() {
        Ok(_) => Err(Failure::input("output_already_exists")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Failure::input("output_inaccessible")),
    }
}
fn internal(reason: &'static str) -> Failure {
    Failure::new(Category::InternalError, reason)
}

pub fn publish(output: &Path, bytes: &[u8]) -> Result<()> {
    publish_with(output, |file| file.write_all(bytes))
}

fn publish_with(output: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> Result<()> {
    check_available(output)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".rhodopsin_xlsx-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|_| Failure::input("output_directory_unwritable"))?;
    let (mut file, path) = temporary.into_parts();
    let written = write(&mut file)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all());
    drop(file);
    if written.is_err() {
        path.close()
            .map_err(|_| internal("output_cleanup_failed"))?;
        return Err(internal("output_write_failed"));
    }
    commit(path, output)
}

#[cfg(unix)]
fn commit(mut path: tempfile::TempPath, output: &Path) -> Result<()> {
    // link + unlink へのフォールバックはしない。一時名の削除失敗を成功扱いしないため。
    let result = rustix::fs::renameat_with(
        rustix::fs::CWD,
        &*path,
        rustix::fs::CWD,
        output,
        rustix::fs::RenameFlags::NOREPLACE,
    );
    match result {
        Ok(()) => {
            // rename で元の名前は消えている。同名で後から作られた別ファイルを消さない。
            path.disable_cleanup(true);
            Ok(())
        }
        Err(error) => {
            path.close()
                .map_err(|_| internal("output_cleanup_failed"))?;
            if error == rustix::io::Errno::EXIST {
                Err(Failure::input("output_already_exists"))
            } else {
                Err(internal("output_commit_failed"))
            }
        }
    }
}
#[cfg(windows)]
fn commit(path: tempfile::TempPath, output: &Path) -> Result<()> {
    // Windows の実装は置換フラグなしの MoveFileExW を使う。
    path.persist_noclobber(output).map_err(|error| {
        let exists = error.error.kind() == io::ErrorKind::AlreadyExists;
        if error.path.close().is_err() {
            internal("output_cleanup_failed")
        } else if exists {
            Failure::input("output_already_exists")
        } else {
            internal("output_commit_failed")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 書き込み途中の失敗で一時ファイルを消す() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.pdf");
        let error = publish_with(&output, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("機密の内容"))
        })
        .unwrap_err();
        assert_eq!(error.reason, "output_write_failed");
        assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
    #[test]
    fn 事前確認後に作られた出力も上書きしない() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.pdf");
        let error = publish_with(&output, |file| {
            std::fs::write(&output, b"existing")?;
            file.write_all(b"new")
        })
        .unwrap_err();
        assert_eq!(error.reason, "output_already_exists");
        assert_eq!(std::fs::read(&output).unwrap(), b"existing");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
