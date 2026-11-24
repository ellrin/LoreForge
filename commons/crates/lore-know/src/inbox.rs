//! 收件: knowledge the author hands over faster than it can be worked on (docs/brain-hub.md §16).
//! He reads sentences aloud to L-Echo, or selects them in a paper, one after another; each lands
//! here as an [`Entry`] and waits. A worker takes them in order: a [`Processor`] turns the raw
//! words into an [`Item`] of the target category (a sentence becomes a pattern with its holes
//! cut, placed on the family's tree, with its tone). What is ready waits again, for him: he
//! reads it, changes it if he likes, and 加入 writes it; 略過 drops it. Nothing reaches the
//! knowledge without that.
//!
//! The shapes in this file are the FROZEN contract between the inbox store (package I1), the
//! processors (I2), L-Brain's server and worker (I3), L-Echo (I4) and the pages.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Item;

/// Where an entry is in its life.
pub mod state {
    /// Handed over, not worked on yet.
    pub const WAITING: &str = "waiting";
    /// The worker has it.
    pub const WORKING: &str = "working";
    /// Worked out: `result` holds the item the author may add.
    pub const READY: &str = "ready";
    /// The worker could not (`error` says why in one line); 重試 puts it back to waiting.
    pub const FAILED: &str = "failed";
    /// The author added it (`added` = the stored item's id).
    pub const ADDED: &str = "added";
    /// The author passed.
    pub const SKIPPED: &str = "skipped";
}

/// How the words came in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct From {
    /// `voice` (read aloud to L-Echo) · `selection` (selected in a Lore app) · `typed`.
    pub via: String,
    /// The Lore app the words were selected in (`lr`, `pc` …); "" for voice.
    pub app: String,
    /// The title of the document they were selected in, when the app gave one (never a path).
    pub doc: String,
}

/// One thing handed over.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// `i` + a sortable id (`new_local`-style); the file is `<hub>/inbox/<id>.json`.
    pub id: String,
    /// The category the result goes into (`pc.patterns`).
    pub category: String,
    /// The raw words, as heard or selected (one sentence for a pattern).
    pub text: String,
    /// The author says these are his own words: only then may the result keep them as an example
    /// (others' sentences give their pattern, never their wording).
    pub own: bool,
    pub from: From,
    /// [`state`].
    pub state: String,
    /// The item worked out (id empty), when `ready`.
    pub result: Option<Item>,
    /// Existing items the result looks like (ids), so the author does not add a near copy.
    pub similar: Vec<String>,
    /// One line, Traditional Chinese, when `failed`.
    pub error: String,
    /// The stored item's id, when `added`.
    pub added: String,
    /// The engine that worked on it (shown small beside the result).
    pub engine: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Counts for the pages (the cover's 待你同意, L-Echo's running line).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub waiting: usize,
    pub working: usize,
    pub ready: usize,
    pub failed: usize,
}

/// What a processor is told besides the entry: what the target family already holds.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The family's tree (`reader::tree`), or `Null`.
    pub tree: Value,
    /// A few existing items of the category, as examples of how a result should look and as
    /// candidates for `similar` (the caller picks them; at most a few dozen).
    pub existing: Vec<Item>,
}

/// One model request (the worker runs it through the app's own engines).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub system: String,
    pub user: String,
    /// JSON schema of the reply.
    pub schema: Value,
}

/// What a processor makes of a reply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Made {
    pub item: Item,
    pub similar: Vec<String>,
}

/// Turns raw words into an item of one format. Pure: it writes the request and reads the reply;
/// the worker makes the call. One per format (`pattern` first).
pub trait Processor: Send + Sync {
    /// The format id it makes (`crate::format::PATTERN`).
    fn format(&self) -> &'static str;
    /// The request for `entry`.
    fn request(&self, entry: &Entry, cx: &Context) -> Request;
    /// The item from the model's reply, checked in code (never trusted): `Err` = one line,
    /// Traditional Chinese, why this reply cannot be used.
    fn read(&self, entry: &Entry, cx: &Context, reply: &Value) -> Result<Made, String>;
}

/// The words of one hand-over cut into what becomes one entry each (a pattern takes one
/// sentence): split at sentence ends, abbreviations and decimals kept whole, fragments under
/// three words dropped. (Stub: filled by package I1.)
pub fn sentences(text: &str) -> Vec<String> {
    let t = text.trim();
    if t.is_empty() { vec![] } else { vec![t.to_string()] }
}
