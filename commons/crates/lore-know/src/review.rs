//! 確認 (review): what a model wrote for the author's own sentences (another way to say a
//! pattern, a word that may be swapped in it) waits here, one candidate at a time. Nothing is in
//! his data until he accepts that one candidate; a candidate he refused is remembered and never
//! offered again; either decision can be taken back on its own.
//!
//! ```text
//! <hub>/review/<set id>.json     one Set: its candidates with their state
//! ```
//!
//! Accepting writes one item of the set's category (format `variant`) through [`Sink`]; taking
//! it back removes that item. A proposal (`proposal.rs`) is a batch decided once; a set is
//! decided candidate by candidate over many sittings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::*;

/// `Candidate::state`.
pub const OPEN: &str = "open";
pub const ACCEPTED: &str = "accepted";
pub const REJECTED: &str = "rejected";

/// One thing the author did not write, waiting for his decision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Candidate {
    /// Unique in its set: `c<n>` in the order given (set by [`Reviews::create`]).
    pub id: String,
    /// The item id of the author's own item it belongs to (`pc.patterns:P001`).
    pub of: String,
    /// `variant` (the whole sentence said another way) or `swap` (one word for `slot`).
    pub kind: String,
    pub text: String,
    /// `swap` only: the word of the author's sentence that `text` may replace.
    pub slot: String,
    /// One short line: when this one fits (emphasis, length, context).
    pub note: String,
    /// [`OPEN`], [`ACCEPTED`] or [`REJECTED`].
    pub state: String,
    /// The stored item's id while accepted, "" otherwise.
    pub item: String,
    /// RFC 3339, "" while open.
    pub decided_at: String,
}

/// A batch of candidates from one run of a tool.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Set {
    /// `r` + a time-sortable id (set by [`Reviews::create`]).
    pub id: String,
    /// Who made it: a Lore app id or a tool's name (`relabel`).
    pub by: String,
    pub title: String,
    /// The category accepted candidates are written to (`pc.variants`); its format is `variant`.
    pub category: String,
    pub created_at: String,
    pub candidates: Vec<Candidate>,
}

/// How many candidates of a set are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub open: usize,
    pub accepted: usize,
    pub rejected: usize,
}

/// The author's own item with the candidates that belong to it, in the order they were given.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// `Candidate::of`.
    pub of: String,
    /// The stored item now; `None` when it no longer exists (its candidates cannot be accepted).
    pub own: Option<Item>,
    pub candidates: Vec<Candidate>,
}

/// The box of sets: a folder with `<id>.json` per set.
#[derive(Debug, Clone)]
pub struct Reviews {
    dir: PathBuf,
}

impl Reviews {
    /// The box in `dir` (made when missing).
    pub fn open(dir: impl Into<PathBuf>) -> Result<Reviews> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Reviews { dir })
    }

    /// The folder (for the implementation and tests).
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// A new set, every candidate [`OPEN`] with ids `c1`, `c2` … Refused: no candidate, an empty
    /// `category`, a candidate without `of` or `text`, a `kind` other than `variant` / `swap`, a
    /// `swap` without `slot`. A candidate whose (`of`, `kind`, `slot`, `text`) was already
    /// [`REJECTED`] or is already waiting or accepted in any earlier set is left out (he is never
    /// asked twice); when nothing is left the set is not made and `Ok(None)` comes back.
    pub fn create(&self, _by: &str, _title: &str, _category: &str, _candidates: Vec<Candidate>) -> Result<Option<Set>> {
        todo!("review package")
    }

    /// [`KnowError::NotFound`] for an unknown or malformed id.
    pub fn get(&self, _id: &str) -> Result<Set> {
        todo!("review package")
    }

    /// Every set, oldest first. A file that cannot be read is skipped.
    pub fn list(&self) -> Vec<Set> {
        todo!("review package")
    }

    /// Candidates still open, over every set.
    pub fn count_open(&self) -> usize {
        todo!("review package")
    }
}

/// The counts of one set.
pub fn counts(_set: &Set) -> Counts {
    todo!("review package")
}

/// The candidates of `set` grouped by the item they belong to, groups in the order of their
/// first candidate, each with that item as stored now.
pub fn groups(_sink: &dyn Sink, _set: &Set) -> Result<Vec<Group>> {
    todo!("review package")
}

/// Accept one open candidate: one new item of `set.category` (format `variant`; title = `text`;
/// fields `pattern` = the local id of `of`, `kind`, `text`, `slot`, `note`; source
/// `Source::lore(set.by)`) is written through `sink`, the candidate becomes [`ACCEPTED`] with
/// the stored id in `item`. Refused when the candidate is not open, or the item `of` no longer
/// exists. The set file is saved only after the item was stored. Returns the candidate.
pub fn accept(_sink: &mut dyn Sink, _box: &Reviews, _set: &str, _candidate: &str) -> Result<Candidate> {
    todo!("review package")
}

/// Refuse one open candidate: it becomes [`REJECTED`]; nothing is written to the data.
pub fn reject(_box: &Reviews, _set: &str, _candidate: &str) -> Result<Candidate> {
    todo!("review package")
}

/// Take a decision back: an accepted candidate's item is removed through `sink` (already gone:
/// fine) and a rejected one is simply reopened; the candidate is [`OPEN`] again with `item` and
/// `decided_at` empty. Refused when the candidate is open.
pub fn undo(_sink: &mut dyn Sink, _box: &Reviews, _set: &str, _candidate: &str) -> Result<Candidate> {
    todo!("review package")
}
