//! The `polish` request: the locked passage, its surroundings, the document's language, the
//! author's phrase library, his liked rewrites and the conversation so far. The reply is one
//! JSON object `{ "rewrite": "…" }` — only the rewritten passage.

use lore_llm::LlmRequest;
use serde_json::{Value, json};

use crate::liked::Liked;

/// Context and phrase limits (characters).
pub const CONTEXT_CHARS: usize = 1500;
pub const PHRASES: usize = 25;
pub const TONE_CHARS: usize = 1500;
pub const STYLE_CHARS: usize = 24_000;
pub const GUIDANCE_CHARS: usize = 8000;

/// One earlier turn of the conversation: what was asked and what came back (locked again).
#[derive(Debug, Clone, Default)]
pub struct TurnText {
    pub instruction: String,
    pub rewrite: String,
}

#[derive(Debug, Clone, Default)]
pub struct Input<'a> {
    /// The passage, locked.
    pub passage: &'a str,
    /// Text right before and after it (the rest of its paragraph), and nearby paragraphs.
    pub before: &'a str,
    pub after: &'a str,
    pub context: &'a str,
    /// `en`, `zh-TW`, … (empty: the passage's own language).
    pub language: &'a str,
    pub phrases: &'a [String],
    /// A ready block about the author's tone (bounded by the app).
    pub tone: &'a str,
    /// The author's style rules (bounded by the app; empty: none).
    pub style: &'a str,
    pub liked: &'a [Liked],
    pub turns: &'a [TurnText],
    pub instruction: &'a str,
    /// A preset's label (empty: a free instruction) and the app's block for it.
    pub preset: &'a str,
    pub guidance: &'a str,
}

pub fn schema() -> Value {
    json!({ "type": "object", "properties": { "rewrite": { "type": "string" } }, "required": ["rewrite"], "additionalProperties": false })
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n { s.to_string() } else { format!("…{}", c[c.len() - n..].iter().collect::<String>()) }
}

fn head(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n { s.to_string() } else { format!("{}…", c[..n].iter().collect::<String>()) }
}

pub const SYSTEM: &str = "You are the author's writing assistant. You rewrite ONLY the passage he selected, following his instructions \
(which words to keep, which connectives to add, drop or replace, what is too wordy). Keep the meaning and the facts; change nothing he did not ask for \
beyond what reads naturally. Tokens like ⟦1⟧, ⟦2⟧ stand for citations, cross-references, labels, mathematics and registered symbols: keep every one exactly \
once, unchanged, where it fits the new sentence; never write LaTeX citations, references, labels or math yourself. Keep other LaTeX markup (\\emph{…}, ~) as it is. \
Write in the passage's language unless told otherwise; match the author's own phrases and the rewrites he liked. \
Answer with one JSON object {\"rewrite\": \"…\"} holding only the rewritten passage — no comments, no surrounding text.";

pub fn request(i: &Input) -> LlmRequest {
    let mut u = String::new();
    if !i.language.trim().is_empty() {
        u += &format!("Document language: {}\n\n", i.language.trim());
    }
    if !i.context.trim().is_empty() {
        u += &format!("Nearby text (context only, do not rewrite):\n{}\n\n", head(i.context.trim(), CONTEXT_CHARS));
    }
    if !i.phrases.is_empty() {
        u += "The author's own phrases (use where they fit):\n";
        for p in i.phrases.iter().take(PHRASES) {
            u += &format!("- {}\n", head(p, 200));
        }
        u += "\n";
    }
    if !i.tone.trim().is_empty() {
        u += &format!("{}\n\n", head(i.tone.trim(), TONE_CHARS));
    }
    if !i.style.trim().is_empty() {
        u += &format!("The author's style rules (follow them in the rewrite; rules, not content):\n{}\n\n", head(i.style.trim(), STYLE_CHARS));
    }
    if !i.liked.is_empty() {
        u += "Rewrites the author liked before (his taste):\n";
        for l in i.liked {
            u += &format!("- asked: {}\n  before: {}\n  after: {}\n", l.instruction.trim(), l.before.trim(), l.after.trim());
        }
        u += "\n";
    }
    if !i.guidance.trim().is_empty() {
        u += &format!("For this rewrite ({}):\n{}\n\n", if i.preset.trim().is_empty() { "the app" } else { i.preset.trim() }, head(i.guidance.trim(), GUIDANCE_CHARS));
    }
    u += "Passage";
    if !i.before.trim().is_empty() || !i.after.trim().is_empty() {
        u += &format!(" (inside its paragraph: «{}[PASSAGE]{}»)", tail(i.before, 400), head(i.after, 400));
    }
    u += &format!(":\n{}\n\n", i.passage);
    for (k, t) in i.turns.iter().enumerate() {
        u += &format!("Instruction {}: {}\nYour rewrite {}: {}\n\n", k + 1, t.instruction.trim(), k + 1, t.rewrite);
    }
    if i.turns.is_empty() {
        u += &format!("Instruction: {}\n", i.instruction.trim());
    } else {
        u += &format!("Instruction {} (applies to your last rewrite): {}\n", i.turns.len() + 1, i.instruction.trim());
    }
    LlmRequest::prompt(SYSTEM, u).json(schema())
}

/// The rewrite in a reply: `{ "rewrite": … }`, or a bare string.
pub fn rewrite_of(v: &Value) -> Option<String> {
    v.get("rewrite").and_then(Value::as_str).or_else(|| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_carries_everything_and_asks_for_json() {
        let phrases = vec!["In contrast to X, we …".to_string()];
        let liked = vec![Liked { before: "a b".into(), after: "b a".into(), instruction: "swap".into(), ..Default::default() }];
        let turns = vec![TurnText { instruction: "shorter".into(), rewrite: "Short ⟦1⟧.".into() }];
        let r = request(&Input { passage: "Long text ⟦1⟧.", before: "Start. ", after: " End.", context: "Prev para.", language: "en", phrases: &phrases, tone: "Write like the presenter.", style: "No contractions.", liked: &liked, turns: &turns, instruction: "keep 'long'", preset: "用字", guidance: "wordy: in order to → to" });
        let u = r.flatten();
        for want in ["For this rewrite (用字):\nwordy: in order to → to", "Document language: en", "Write like the presenter.", "style rules (follow them in the rewrite; rules, not content):\nNo contractions.", "Prev para.", "In contrast to X", "before: a b", "Long text ⟦1⟧.", "Your rewrite 1: Short ⟦1⟧.", "Instruction 2 (applies to your last rewrite): keep 'long'", "[PASSAGE]"] {
            assert!(u.contains(want), "{want}: {u}");
        }
        assert!(r.json_schema.is_some());
        assert_eq!(rewrite_of(&json!({ "rewrite": " x " })).as_deref(), Some("x"));
        assert_eq!(rewrite_of(&json!({ "rewrite": "" })), None);
        let plain = request(&Input { passage: "x", instruction: "y", ..Default::default() }).flatten();
        assert!(!plain.contains("style rules"), "no style, nothing added");
    }
}
