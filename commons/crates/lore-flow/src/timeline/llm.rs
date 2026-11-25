//! The model task of a timeline (package T-llm): a description (optionally with a picture) →
//! the points of a [`TimelineSpec`] and one of the styles on offer. Signatures are frozen.
//! The model gives words only: no coordinates, no colours, no style of its own.

use lore_llm::{Image, LlmRequest};
use serde_json::Value;

use super::{Style, TimelineSpec};
use crate::llm::Llm;

/// Task ids in lore-llm's router (`tasks.toml` of this crate).
pub const TASK: &str = "sc-flow-timeline";
pub const TASK_VISION: &str = "sc-flow-timeline-vision";
/// More points than this do not read on one slide.
pub const MAX_POINTS: usize = 10;

pub fn task_for(has_image: bool) -> &'static str {
    if has_image { TASK_VISION } else { TASK }
}

/// What the model is asked.
pub struct TimelinePrompt<'a> {
    /// The author's description, or the text the points are taken from.
    pub description: &'a str,
    /// A picture of a timeline to follow (its points and their order, never its colours).
    pub image: Option<Image>,
    /// The styles on offer (id, name, note): the model picks one id; only styles that draw as
    /// written ([`Style::cannot`] empty) are offered.
    pub styles: &'a [Style],
    /// The language of the texts (`en`, `zh-TW` …): the model keeps the description's own
    /// unless this says otherwise.
    pub language: &'a str,
}

/// The JSON schema of the reply: `{ title, style, points: [{ time, title, body, icon }] }`,
/// every field required, at most [`MAX_POINTS`] points.
pub fn schema(styles: &[Style]) -> Value {
    let _ = styles;
    Value::Null
}

/// The request for `p` (system and user text, the picture when there is one, the schema).
pub fn request(p: &TimelinePrompt) -> LlmRequest {
    let _ = p;
    LlmRequest::prompt("", "")
}

/// The spec in a reply, checked in code and never trusted: at least two points, each with a
/// time; texts trimmed; a style that is not on offer becomes the first offered one; more than
/// [`MAX_POINTS`] points is an error. `Err` = one line the model can be sent back.
pub fn read(reply: &Value, styles: &[Style]) -> Result<TimelineSpec, String> {
    let _ = (reply, styles);
    Err("not built yet".into())
}

/// Ask through an app's [`Llm`]; a reply that [`read`] refuses is sent back once with the
/// reason. Returns the spec and the engine that answered.
pub async fn propose_via(llm: &dyn Llm, p: &TimelinePrompt<'_>) -> Result<(TimelineSpec, String), String> {
    let _ = (llm, p);
    Err("not built yet".into())
}
