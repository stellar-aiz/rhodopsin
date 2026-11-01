use rhodopsin::xlsx::output::publish;
#[test]
fn 出力を確定して一時ファイルを残さない() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.pdf");
    let bytes = b"%PDF-1.7\ncomplete";
    publish(&output, bytes).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(
        publish(&output, b"replacement").unwrap_err().reason,
        "output_already_exists"
    );
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
}
#[test]
fn 存在しない出力ディレクトリを作成しない() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("missing/out.pdf");
    assert_eq!(
        publish(&output, b"data").unwrap_err().reason,
        "output_directory_unwritable"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[cfg(unix)]
#[test]
fn 壊れたシンボリックリンクも既存出力として保護する() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.pdf");
    std::os::unix::fs::symlink("missing", &output).unwrap();
    assert_eq!(
        publish(&output, b"data").unwrap_err().reason,
        "output_already_exists"
    );
    assert!(output.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(!dir.path().join("missing").exists());
}

#[test]
fn 同時に確定しても成功は一つだけになる() {
    use std::sync::{Arc, Barrier};
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.pdf");
    let gate = Arc::new(Barrier::new(4));
    let threads: Vec<_> = (0u8..4)
        .map(|value| {
            let gate = gate.clone();
            let output = output.clone();
            std::thread::spawn(move || {
                gate.wait();
                (value, publish(&output, &vec![value; 8192]))
            })
        })
        .collect();
    let mut winner = None;
    for thread in threads {
        let (value, result) = thread.join().unwrap();
        match result {
            Ok(()) => {
                assert!(winner.replace(value).is_none());
            }
            Err(error) => assert_eq!(error.reason, "output_already_exists"),
        }
    }
    assert_eq!(std::fs::read(&output).unwrap(), vec![winner.unwrap(); 8192]);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
