//! LaTeX source mode (arXiv e-prints, the author's own papers).

pub mod bib;
pub mod extract;
pub mod plain;
pub mod scan;
pub mod source;

pub use extract::{extract, extract_dir, tabular_grid};
pub use plain::{PlainOpts, to_plain};
pub use source::{LatexProject, find_main};
