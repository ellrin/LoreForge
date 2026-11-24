//! The formats: fixed shapes of knowledge. Each implements [`crate::Format`]; a format that
//! reads an app's existing data also gives [`crate::Adapter`]s.
//!
//! * `table` (K2): pattern, expression, wording, rule, exemplar.
//! * `text` (K3): note, bundle, layout.
//! * `figure` (K4): figure (component / module / figure; entity or reference).

pub mod figure;
pub mod table;
pub mod text;
