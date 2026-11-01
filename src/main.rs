//! The `rhodopsin` binary: a thin wrapper around [`rhodopsin::cli`].
fn main() -> std::process::ExitCode {
    rhodopsin::cli::run()
}
