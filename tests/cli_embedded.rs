//! The CLI entry point that a dependent crate can call to build the same binary (#125).
#![cfg(feature = "cli")]
use rhodopsin::cli::run_from_args;
use std::{path::Path, process::ExitCode};

#[test]
fn run_from_args_converts_in_process_and_reports_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = dir.path().join("out");
    let code = run_from_args([
        "rhodopsin".as_ref(),
        root.join("examples/basic.pptx").as_os_str(),
        "--out".as_ref(),
        out.as_os_str(),
        "--width".as_ref(),
        "100".as_ref(),
        "--font-fallback".as_ref(),
        "--no-system-fonts".as_ref(),
        "--font-file".as_ref(),
        root.join("tests/fixtures/samples.ttf").as_os_str(),
    ]);
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(out.join("manifest.json").exists());
    // Invalid arguments and a refused overwrite return their codes without exiting the test.
    assert_eq!(
        run_from_args(["rhodopsin", "--width", "1"]),
        ExitCode::from(2)
    );
    let refused = run_from_args([
        "rhodopsin".as_ref(),
        root.join("examples/basic.pptx").as_os_str(),
        "--out".as_ref(),
        out.as_os_str(),
    ]);
    assert_eq!(refused, ExitCode::from(1));
    assert_eq!(run_from_args(["rhodopsin", "--version"]), ExitCode::SUCCESS);
}
