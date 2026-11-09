//! PaperComposer core: loading a writing profile (phrase bank, vocabulary,
//! rules), validating it, retrieving phrases, and running the "hard" rule checks
//! that need no model.
//!
//! A profile is a folder of plain files that lives in the user's own private
//! repository. Nothing in this crate writes to it except through explicit calls.

pub mod authors;
pub mod bib;
pub mod check;
pub mod compose;
pub mod decompose;
pub mod draft;
pub mod front;
pub mod journal;
pub mod meta;
pub mod paper;
pub mod table;
pub mod profile;
pub mod retrieve;
pub mod softcheck;
pub mod text;
pub mod textable;

pub use check::{check_text, ParagraphReport, Severity, Finding};
pub use profile::{Expression, Issue, Manifest, Phrase, Profile, Rule, RuleKind, VocabEntry, SCHEMA_VERSION};
pub use retrieve::{search, Hit, Query};
