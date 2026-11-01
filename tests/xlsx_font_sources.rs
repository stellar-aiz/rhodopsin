#[path = "xlsx_support/mod.rs"]
mod support;
use rhodopsin::xlsx::{
    font_sources::{SearchLimits, Sources},
    fonts::{FontBook, FontLimits},
};
#[test]
fn 明示先だけを決まった順序で読み込む() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(
        dir.path().join("z.ttf"),
        support::font("末尾", false, false, 0),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("a.OTF"),
        support::font("先頭", false, false, 0),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("sub/b.ttf"),
        support::font("途中", false, false, 0),
    )
    .unwrap();
    std::fs::write(dir.path().join("ignored.txt"), "フォントではない").unwrap();
    let sources = Sources {
        directories: vec![dir.path().into()],
        ..Sources::default()
    };
    let mut book = FontBook::default();
    sources
        .load(&mut book, &SearchLimits::default(), &FontLimits::default())
        .unwrap();
    assert_eq!(book.select("先頭", false, false, None).unwrap().index, 0);
    assert_eq!(book.select("途中", false, false, None).unwrap().index, 1);
    assert_eq!(book.select("末尾", false, false, None).unwrap().index, 2);
}
#[test]
fn 探索の上限と失敗時の原状維持() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.ttf"),
        support::font("試験", false, false, 0),
    )
    .unwrap();
    std::fs::write(dir.path().join("z.ttf"), "機密の壊れたフォント").unwrap();
    let sources = Sources {
        directories: vec![dir.path().into()],
        ..Sources::default()
    };
    let mut book = FontBook::default();
    let error = sources
        .load(&mut book, &SearchLimits::default(), &FontLimits::default())
        .unwrap_err();
    assert!(book.faces.is_empty());
    assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
    assert_eq!(
        sources
            .load(
                &mut book,
                &SearchLimits {
                    entries: 1,
                    depth: 1
                },
                &FontLimits::default()
            )
            .unwrap_err()
            .reason,
        "font_search_entry_limit"
    );
    std::fs::remove_file(dir.path().join("z.ttf")).unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    assert_eq!(
        sources
            .load(
                &mut book,
                &SearchLimits {
                    entries: 20,
                    depth: 0
                },
                &FontLimits::default()
            )
            .unwrap_err()
            .reason,
        "font_search_depth_limit"
    );
}
#[cfg(unix)]
#[test]
fn 探索先外へのシンボリックリンクをたどらない() {
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("/存在しない機密パス", dir.path().join("link.ttf")).unwrap();
    let sources = Sources {
        directories: vec![dir.path().into()],
        ..Sources::default()
    };
    assert_eq!(
        sources
            .load(
                &mut FontBook::default(),
                &SearchLimits::default(),
                &FontLimits::default()
            )
            .unwrap_err()
            .reason,
        "font_directory_symlink"
    );
}

#[cfg(unix)]
#[test]
fn 許可した別の探索先へのフォントリンクは実体と重複せず読む() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let font = second.join("font.ttf");
    std::fs::write(&font, support::font("試験", false, false, 0)).unwrap();
    std::os::unix::fs::symlink(&font, first.join("alias.ttf")).unwrap();
    let sources = Sources {
        files: vec![font],
        directories: vec![first, second],
    };
    let mut book = FontBook::default();
    sources
        .load(&mut book, &SearchLimits::default(), &FontLimits::default())
        .unwrap();
    assert_eq!(book.faces.len(), 1);
    assert!(book.select("試験", false, false, None).is_ok());
}

#[cfg(unix)]
#[test]
fn 実在する許可外ファイルとディレクトリへのリンクを拒否する() {
    let root = tempfile::tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let outside = root.path().join("outside");
    std::fs::create_dir(&allowed).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let font = outside.join("font.ttf");
    std::fs::write(&font, support::font("試験", false, false, 0)).unwrap();
    let link = allowed.join("alias.ttf");
    for target in [&font, &outside] {
        std::os::unix::fs::symlink(target, &link).unwrap();
        let mut book = FontBook::default();
        let sources = Sources {
            files: vec![],
            directories: vec![allowed.clone()],
        };
        assert_eq!(
            sources
                .load(&mut book, &SearchLimits::default(), &FontLimits::default())
                .unwrap_err()
                .reason,
            "font_directory_symlink"
        );
        assert!(book.faces.is_empty());
        std::fs::remove_file(&link).unwrap();
    }
}

#[test]
fn 要求されない巨大フォントの本体を読み込まない() {
    let dir = tempfile::tempdir().unwrap();
    let wanted = support::font("Wanted", false, false, 0);
    std::fs::write(dir.path().join("wanted.ttf"), &wanted).unwrap();
    let unwanted = dir.path().join("unwanted.ttf");
    std::fs::write(&unwanted, support::font("Other", false, false, 0)).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&unwanted)
        .unwrap()
        .set_len(1024 * 1024)
        .unwrap();
    let sources = Sources {
        files: vec![],
        directories: vec![dir.path().into()],
    };
    let limits = FontLimits {
        file_bytes: wanted.len() as u64 + 100,
        ..Default::default()
    };
    let mut book = FontBook::default();
    sources
        .load_matching(
            &mut book,
            &SearchLimits::default(),
            &limits,
            &std::collections::BTreeSet::from(["wanted".into()]),
            1024 * 1024,
        )
        .unwrap();
    assert_eq!(book.faces.len(), 1);
    assert!(book.select("Wanted", false, false, None).is_ok());
    assert!(book.select("Other", false, false, None).is_err());
    let mut unchanged = FontBook::default();
    assert_eq!(
        sources
            .load_matching(
                &mut unchanged,
                &SearchLimits::default(),
                &limits,
                &std::collections::BTreeSet::from(["Other".into()]),
                1024 * 1024
            )
            .unwrap_err()
            .reason,
        "font_file_limit"
    );
    assert!(unchanged.faces.is_empty());
}

#[test]
fn 名前テーブル探索の読取量を制限し失敗時は書体を追加しない() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("font.ttf");
    std::fs::write(&path, support::font("Wanted", false, false, 0)).unwrap();
    let sources = Sources {
        files: vec![path],
        directories: vec![],
    };
    let mut book = FontBook::default();
    let error = sources
        .load_matching(
            &mut book,
            &SearchLimits::default(),
            &FontLimits::default(),
            &std::collections::BTreeSet::from(["Wanted".into()]),
            1,
        )
        .unwrap_err();
    assert_eq!(error.reason, "font_catalog_limit");
    assert!(book.faces.is_empty());
}

#[test]
fn 不正なフォント索引の長さと範囲を割当前に拒否する() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("font.ttf");
    let sources = Sources {
        files: vec![path.clone()],
        directories: vec![],
    };
    let mut collection = b"ttcf\0\x01\0\0".to_vec();
    collection.extend(1025u32.to_be_bytes());
    let mut tables = vec![0; 12];
    tables[..4].copy_from_slice(b"\0\x01\0\0");
    tables[4..6].copy_from_slice(&4097u16.to_be_bytes());
    for (bytes, reason) in [
        (vec![0; 4], "invalid_font_catalog"),
        (collection, "font_catalog_face_limit"),
        (tables, "font_catalog_table_limit"),
    ] {
        std::fs::write(&path, bytes).unwrap();
        let mut book = FontBook::default();
        let error = sources
            .load_matching(
                &mut book,
                &SearchLimits::default(),
                &FontLimits::default(),
                &std::collections::BTreeSet::from(["Wanted".into()]),
                1024 * 1024,
            )
            .unwrap_err();
        assert_eq!(error.reason, reason);
        assert!(book.faces.is_empty());
    }
}
