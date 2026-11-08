//! ym-docsrc — paper material extraction (design §7.4, §6 learning flow).
//!
//! Two sources produce the same [`AssetCatalog`]:
//! - [`latex`]: an arXiv e-print or the author's own LaTeX folder → sections,
//!   figures (files, captions, labels, sub-figures), tables (source + grid),
//!   equations (LaTeX + labels + numbers), bibliography, title/authors/abstract.
//!   [`arxiv`] fetches e-prints behind the [`arxiv::ArxivFetcher`] trait.
//! - [`pdf`]: a PDF alone → poppler CLIs (`pdftotext -bbox-layout`,
//!   `pdftoppm`, `pdfinfo`) → column layout, captions, figure/table regions,
//!   numbered display equations, section headings, references, metadata,
//!   with page renders cached and PNG crops ([`crop`]).
//!
//! Plus [`boxer`] (semantic sub-module boxes on an architecture figure through
//! ym-llm task `sc-crop-modules`) and [`text`] (plain text per section for
//! term/notation scanners).

pub mod arxiv;
pub mod boxer;
pub mod crop;
pub mod error;
pub mod latex;
pub mod model;
pub mod pdf;
pub mod text;

pub use error::{Error, Result};
pub use model::{
    Asset, AssetCatalog, AssetKind, BBox, DocSource, Meta, PageInfo, Reference, SCHEMA, Section,
    SourceRef,
};
pub use text::{SectionText, TextOptions};
