//! Route `.xlsx` input to the spreadsheet crate and render its results as YAML.
//!
//! The spreadsheet renderer currently behaves as if `--strict` were always set:
//! unsupported content stops the conversion instead of being approximated with a
//! warning. Per-element approximate rendering is planned; see docs/xlsx.md.
use super::output::quoted;
use crate::xlsx::{
    ConversionOptions,
    diagnostic::{Category, Failure},
    font_sources::Sources,
    package::Limits,
    report::Report,
};
use std::path::{Path, PathBuf};

/// Shared CLI options that the spreadsheet path accepts.
pub struct Options {
    pub font_file: Vec<PathBuf>,
    pub no_system_fonts: bool,
    pub locale: crate::xlsx::format::date::Locale,
}

/// Whether the input is a spreadsheet. `.xls` and `.xlsm` are routed here too, so the
/// spreadsheet renderer refuses them with its own reason instead of the PPTX pipeline
/// failing on missing presentation parts.
pub fn is_spreadsheet(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        ["xlsx", "xlsm", "xls"]
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    })
}

/// Result document for `--format png` with XLSX input.
pub fn unsupported_format() -> String {
    "status: failed\noutput:\n  created: false\nerrors:\n  - code: FORMAT_NOT_SUPPORTED\n    message: XLSX input can only be converted to PDF.\n    suggestion: Use --format pdf.\n".into()
}

/// Result document for an option the spreadsheet path does not accept yet.
/// Only the option name is echoed, never its value.
pub fn unsupported_option(option: &str) -> String {
    format!(
        "status: failed\noutput:\n  created: false\nerrors:\n  - code: OPTION_NOT_SUPPORTED\n    option: {}\n    message: This option is not available for XLSX input yet.\n    suggestion: Remove the option. XLSX input accepts --format pdf, --out, --font-file, --no-system-fonts, --strict, and --locale.\n",
        quoted(option)
    )
}

/// Convert one workbook to PDF. Return the stdout document on success and the
/// stderr document on failure; nothing is written on failure.
pub fn convert(input: &Path, out: &Path, options: Options) -> Result<String, String> {
    let settings = ConversionOptions {
        font_sources: Sources {
            files: options.font_file,
            directories: Vec::new(),
        },
        system_fonts: !options.no_system_fonts,
        locale: options.locale,
        ..Default::default()
    };
    // Panic messages may quote workbook content; keep them off stderr.
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::xlsx::convert_with_options(input, out, &Limits::default(), &settings)
    }));
    match result {
        Ok(Ok(report)) => Ok(completed(out, &report)),
        Ok(Err(failure)) => Err(failed(&failure)),
        Err(_) => Err(failed(&Failure::new(
            Category::InternalError,
            "internal_panic",
        ))),
    }
}

fn completed(out: &Path, report: &Report) -> String {
    let details = serde_json::to_string(report)
        .expect("report serialization")
        .replace('\u{0085}', "\\u0085")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!(
        "status: completed\noutput:\n  created: true\n  file: {}\n  pages: {}\nreport: {details}\n",
        quoted(&out.to_string_lossy()),
        report.page_count
    )
}

fn failed(failure: &Failure) -> String {
    let (code, suggestion) = match failure.category {
        Category::InvalidInput => (
            "INVALID_INPUT",
            "Check that the input is an unencrypted .xlsx workbook with saved formula results and that the output path is unused.",
        ),
        Category::UnsupportedFeature => (
            "UNSUPPORTED_FEATURE",
            "The workbook uses a feature the XLSX renderer does not support. XLSX input always stops on unsupported content; see docs/xlsx.md.",
        ),
        Category::MissingFont => (
            "FONT_NOT_FOUND",
            "Add the required font with --font-file <path>. Font substitution is not available for XLSX input yet.",
        ),
        Category::LayoutFailure => (
            "LAYOUT_FAILED",
            "The page could not be assembled within the renderer's limits. Reduce the print area or report the workbook shape.",
        ),
        Category::InternalError => (
            "INTERNAL_ERROR",
            "Check the execution environment and report the failure with its reason code.",
        ),
    };
    let mut result = format!(
        "status: failed\noutput:\n  created: false\nerrors:\n  - code: {code}\n    reason: {}\n",
        failure.reason
    );
    if let Some(location) = &failure.location {
        result.push_str(&format!(
            "    location:\n      sheet: {}\n",
            location.sheet_index
        ));
        if let Some(cell) = &location.cell {
            result.push_str(&format!("      cell: {}\n", quoted(cell)));
        }
    }
    result.push_str(&format!("    suggestion: {}\n", quoted(suggestion)));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_documents_carry_code_reason_and_location_only() {
        let failure = Failure::input("formula_cached_value_missing").at(2, Some("B7".into()));
        let text = failed(&failure);
        assert!(text.starts_with("status: failed\noutput:\n  created: false\nerrors:\n"));
        assert!(
            text.contains("  - code: INVALID_INPUT\n    reason: formula_cached_value_missing\n")
        );
        assert!(text.contains("    location:\n      sheet: 2\n      cell: \"B7\"\n"));
        assert!(text.contains("    suggestion: "));
        let without = failed(&Failure::unsupported("cell_layout_unmeasured"));
        assert!(!without.contains("location:"));
        assert!(without.contains("code: UNSUPPORTED_FEATURE"));
        assert!(without.contains("see docs/xlsx.md"));
    }

    #[test]
    fn success_document_matches_the_pdf_layout() {
        let report = Report {
            schema_version: 1,
            page_count: 0,
            pages: vec![],
            fonts: vec![],
            warnings: vec![],
            layout_font_substituted: false,
        };
        let text = completed(Path::new("out: fake.pdf"), &report);
        assert_eq!(
            text,
            "status: completed\noutput:\n  created: true\n  file: \"out: fake.pdf\"\n  pages: 0\nreport: {\"schema_version\":1,\"page_count\":0,\"pages\":[],\"fonts\":[],\"warnings\":[],\"layout_font_substituted\":false}\n"
        );
    }

    #[test]
    fn option_documents_echo_the_option_name_without_values() {
        assert!(unsupported_option("--font-map").contains("option: \"--font-map\"\n"));
        assert!(unsupported_format().contains("code: FORMAT_NOT_SUPPORTED"));
        assert!(is_spreadsheet(Path::new("book.XLSX")));
        assert!(is_spreadsheet(Path::new("book.xlsm")));
        assert!(is_spreadsheet(Path::new("book.Xls")));
        assert!(!is_spreadsheet(Path::new("book.pptx")));
        assert!(!is_spreadsheet(Path::new("book.xlsx.pptx")));
    }
}
