//! LLM-dependent features behind a small trait.
//!
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │ INTEGRATION POINT (lore-llm): `lore-llm` is being built in parallel. When it    │
//! │ lands, add an implementation of [`CardAssistant`] that runs the tasks       │
//! │ `kb-summarize-card` (draft) and `kb-extract-terms` (candidates) through its  │
//! │ task router with a JSON schema, validates the result and retries once       │
//! │ (design §9). Until then apps use [`MockAssistant`]; tests always do.        │
//! └─────────────────────────────────────────────────────────────────────────────┘

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::node::NodeFields;

/// Draft a card for `term` from the text around it (a paper paragraph, a slide, a chat).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DraftRequest {
    pub term: String,
    pub context: String,
    /// Preferred card kind (`concept`, `method`, …).
    pub kind: Option<String>,
    /// Language of the card text (`zh-Hant` by default for summaries).
    pub lang: Option<String>,
}

pub trait CardAssistant: Send + Sync {
    /// A card draft (never saved by this call; the user edits it first).
    fn draft_card(&self, req: &DraftRequest) -> Result<NodeFields>;
    /// Candidate terms in `text` that are not in `known` (lowercased names).
    fn extract_terms(&self, text: &str, known: &[String]) -> Result<Vec<String>>;
    /// Shown in the UI ("mock", later the model id from lore-llm).
    fn name(&self) -> String;
}

/// Offline stand-in: the draft is the context sentence that contains the term;
/// candidates are acronyms and Capitalised Multi-Word terms.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockAssistant;

fn sentences(text: &str) -> Vec<&str> {
    text.split_inclusive(['.', '。', '!', '?', '！', '？', '\n']).map(str::trim).filter(|s| !s.is_empty()).collect()
}

impl CardAssistant for MockAssistant {
    fn draft_card(&self, req: &DraftRequest) -> Result<NodeFields> {
        let term = req.term.trim();
        if term.is_empty() {
            return Err(crate::KbError::Invalid("term is empty".into()));
        }
        let lower = term.to_lowercase();
        let summary = sentences(&req.context).into_iter().find(|s| s.to_lowercase().contains(&lower)).unwrap_or("").to_string();
        Ok(NodeFields {
            kind: Some(req.kind.clone().unwrap_or_else(|| "concept".into())),
            title: Some(term.to_string()),
            lang: req.lang.clone(),
            summary: Some(summary),
            body: Some(format!("（草稿，由 mock 產生）\n\n> {}", req.context.trim())),
            ..Default::default()
        })
    }

    fn extract_terms(&self, text: &str, known: &[String]) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        let words: Vec<&str> = text.split(|c: char| !(c.is_alphanumeric() || c == '-')).filter(|w| !w.is_empty()).collect();
        let mut push = |t: String| {
            let l = t.to_lowercase();
            if !known.contains(&l) && !out.iter().any(|o| o.to_lowercase() == l) {
                out.push(t);
            }
        };
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            let caps = w.chars().filter(|c| c.is_ascii_uppercase()).count();
            if caps >= 2 && w.len() <= 10 && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                push(w.to_string());
                i += 1;
                continue;
            }
            // Capitalised runs of 2–4 words (not sentence starts we cannot tell apart; good enough for a mock).
            let mut j = i;
            while j < words.len() && j - i < 4 && words[j].chars().next().is_some_and(|c| c.is_ascii_uppercase()) && words[j].chars().skip(1).all(|c| c.is_ascii_lowercase() || c == '-') {
                j += 1;
            }
            if j - i >= 2 {
                push(words[i..j].join(" "));
                i = j;
            } else {
                i += 1;
            }
        }
        Ok(out)
    }

    fn name(&self) -> String {
        "mock".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_drafts_and_extracts() {
        let m = MockAssistant;
        let f = m.draft_card(&DraftRequest { term: "Dice".into(), context: "We use IoU. The Dice score measures overlap. Done.".into(), ..Default::default() }).unwrap();
        assert_eq!(f.summary.as_deref(), Some("The Dice score measures overlap."));
        let t = m.extract_terms("The Fréchet Inception Distance (FID) and Structural Similarity Index are used with BLEU.", &["bleu".into()]).unwrap();
        assert_eq!(t, vec!["Inception Distance", "FID", "Structural Similarity Index"]);
    }
}
