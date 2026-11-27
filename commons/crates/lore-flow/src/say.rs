//! A sentence about a figure's placement → commands. The model is shown the figure in words
//! (its objects by name, kind, words and cell; its lines; its ties) and the closed set of
//! [`crate::command::Command`]s, and answers with a list of them, or says what it could not
//! tell from the sentence. It never writes a coordinate: [`crate::command::apply`] works out
//! every place. Commands that cannot be done go back to the model once with their reasons.
//!
//! FROZEN SEAM: the signatures below are fixed; the bodies are to be written.

use lore_llm::LlmRequest;
use serde_json::Value;

use crate::command::{Clash, Command};
use crate::llm::Llm;
use crate::spec::Graph;

/// Task id in lore-llm's router (`tasks.toml`).
pub const TASK: &str = "flow-command";

/// What a sentence came to.
#[derive(Debug, Clone)]
pub struct Said {
    /// The figure after the commands that could be done.
    pub graph: Graph,
    /// The commands the model gave (after the one correction, if there was one).
    pub commands: Vec<Command>,
    /// What changed, in words, one line per change.
    pub done: Vec<String>,
    /// The commands that still could not be done, with their reasons.
    pub cannot: Vec<Clash>,
    /// What the model could not tell from the sentence (empty when it could), in the
    /// sentence's language.
    pub unclear: String,
    /// The engine that answered.
    pub provider: String,
    /// The commands were sent back once with the reasons they could not be done.
    pub retried: bool,
}

/// The reply's JSON schema: `{commands: [Command…], unclear: string}`, every command one of the
/// closed set with exactly its own fields.
pub fn schema() -> Value {
    unimplemented!()
}

/// The figure in words, as the model is shown it: per area its objects (name, kind, the words
/// on it, column and row, what it spans, what it is tied to), then the lines (from → to, kind).
pub fn figure_words(_g: &Graph) -> String {
    unimplemented!()
}

/// The request for one sentence.
pub fn request(_g: &Graph, _sentence: &str) -> LlmRequest {
    unimplemented!()
}

/// The commands and the `unclear` words of a reply; `Err` when it is not of the schema.
pub fn read(_reply: &Value) -> Result<(Vec<Command>, String), String> {
    unimplemented!()
}

/// One sentence through the model (task [`TASK`]) and the commands through
/// [`crate::command::apply_all`]. When some cannot be done, the model is asked once more with
/// the reasons (and the names nearest to an unknown one) and its second list is taken when it
/// leaves fewer undone.
pub async fn say(_llm: &dyn Llm, _g: &Graph, _sentence: &str) -> Result<Said, String> {
    unimplemented!()
}
