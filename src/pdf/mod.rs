//! PDF rendering and document output.
//!
//! [`convert_pdf`] converts PPTX into PDF. With the `pdf-input` feature,
//! `render` converts the supported PDF subset into PNG page images.
//!
//! # Example
//!
//! ```no_run
//! # #[cfg(feature = "pdf-input")]
//! # fn example() -> anyhow::Result<()> {
//! let report = rhodopsin::pdf::render(
//!     "input.pdf", "pages", &rhodopsin::pdf::Options::default(),
//! )?;
//! assert!(!report.pages.is_empty());
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "pdf-input")]
mod interpreter;
mod output;
#[cfg(feature = "pdf-input")]
mod reader;
#[cfg(feature = "pdf-input")]
mod render;
mod writer;

pub use output::{PdfConversionReport, PdfSlideReport, convert_pdf};
#[cfg(feature = "pdf-input")]
pub use render::{Options, Page, Report, render};
