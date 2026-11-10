//! LLM-dependent features behind a small trait.
//!
//! LLM-dependent features.
//!
//! The real draft runs in the app's server through `lore-llm` (kb-core stays free of the
//! HTTP stack and of async): this module owns the parts that do not need an engine, so every
//! app drafts the same way. [`clean_source`] joins PDF line-break hyphenation, [`draft_prompt`]
//! builds the request (system text, user text, JSON schema) and [`parse_draft`] turns the
//! engine's JSON into an editable [`NodeFields`] that is never saved by this crate.
//! [`MockAssistant`] is for tests (and the offline candidates scan) only.

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
    /// Page of the quoted source (kept in the draft body as `p.N`).
    pub page: Option<u32>,
    /// Title of the paper / slide the context comes from.
    pub source: Option<String>,
}

pub trait CardAssistant: Send + Sync {
    /// A card draft (never saved by this call; the user edits it first).
    fn draft_card(&self, req: &DraftRequest) -> Result<NodeFields>;
    /// Candidate terms in `text` that are not in `known` (lowercased names).
    fn extract_terms(&self, text: &str, known: &[String]) -> Result<Vec<String>>;
    /// Shown in the UI ("mock", later the model id from lore-llm).
    fn name(&self) -> String;
}

/// Offline stand-in for tests (it never calls a model): the draft is the context sentence that contains the term;
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

/// Join words a PDF broke across lines ("de- fers", "de-\nfers", soft hyphens) and collapse
/// whitespace. A hyphen is dropped only between a letter and a lowercase letter after a gap, so
/// "state-of-the-art" and "5- 10" stay.
pub fn clean_source(text: &str) -> String {
    let chars: Vec<char> = text.chars().filter(|c| *c != '\u{ad}').collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '-' && i > 0 && chars[i - 1].is_alphabetic() && chars.get(i + 1).is_some_and(|n| n.is_whitespace()) {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if chars.get(j).is_some_and(|n| n.is_lowercase()) {
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What to send to the engine for a [`DraftRequest`]: system text, user text, JSON schema.
pub struct DraftPrompt {
    pub system: String,
    pub user: String,
    pub schema: serde_json::Value,
    /// The cleaned source text that was sent (and goes into the body as the quote).
    pub quote: String,
}

fn lang_name(lang: Option<&str>) -> String {
    match lang.map(str::trim).filter(|l| !l.is_empty()) {
        None => "台灣繁體中文".into(),
        Some(l) if l.eq_ignore_ascii_case("zh-hant") || l.eq_ignore_ascii_case("zh-tw") => "台灣繁體中文".into(),
        Some(l) if l.to_lowercase().starts_with("en") => "English".into(),
        Some(l) => format!("語言代碼 {l}"),
    }
}

/// The request for a card summary of `req.term` from `req.context` (the source quote only).
pub fn draft_prompt(req: &DraftRequest) -> Result<DraftPrompt> {
    let term = req.term.trim();
    if term.is_empty() {
        return Err(crate::KbError::Invalid("term is empty".into()));
    }
    let quote = clean_source(&req.context);
    if quote.is_empty() {
        return Err(crate::KbError::Invalid("context is empty".into()));
    }
    let lang = lang_name(req.lang.as_deref());
    let system = format!(
        "你是知識卡片的摘要助手。只根據使用者附上的【原文】，為指定術語寫一句卡片摘要（一到兩句，不超過 80 字），用{lang}寫，術語保留原文寫法。\
不得加入原文沒有的事實、數字、人名、年份或定義，也不要用一般知識補充；原文沒有說明這個術語是什麼時，supported 設為 false，summary 留空字串。\
只輸出 JSON：{{\"summary\": 字串, \"supported\": 布林}}。"
    );
    let user = format!("術語：{term}\n\n【原文】\n{quote}");
    let schema = serde_json::json!({
        "type": "object",
        "properties": { "summary": { "type": "string" }, "supported": { "type": "boolean" } },
        "required": ["summary", "supported"],
        "additionalProperties": false
    });
    Ok(DraftPrompt { system, user, schema, quote })
}

/// The editable card draft from the engine's JSON. Nothing is saved.
pub fn parse_draft(req: &DraftRequest, quote: &str, v: &serde_json::Value) -> Result<NodeFields> {
    let term = req.term.trim();
    let summary = v["summary"].as_str().map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
    if v["supported"].as_bool() == Some(false) || summary.is_empty() {
        return Err(crate::KbError::Assistant(format!("原文沒有說明「{term}」，沒有起草")));
    }
    let mut body = format!("> {quote}");
    let from = match (req.source.as_deref().map(str::trim).filter(|s| !s.is_empty()), req.page) {
        (Some(s), Some(p)) => Some(format!("{s}，p.{p}")),
        (Some(s), None) => Some(s.to_string()),
        (None, Some(p)) => Some(format!("p.{p}")),
        (None, None) => None,
    };
    if let Some(f) = from {
        body.push_str(&format!("\n\n（{f}）"));
    }
    Ok(NodeFields {
        kind: Some(req.kind.clone().filter(|k| !k.is_empty()).unwrap_or_else(|| "concept".into())),
        title: Some(term.to_string()),
        lang: req.lang.clone(),
        summary: Some(summary),
        body: Some(body),
        ..Default::default()
    })
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

    #[test]
    fn hyphenated_breaks_are_joined() {
        assert_eq!(clean_source("The model de- fers the\nlabel to a state-of-the-art step; 5- 10 and in-\nference."), "The model defers the label to a state-of-the-art step; 5- 10 and inference.");
        assert_eq!(clean_source("co\u{ad}operate"), "cooperate");
    }

    #[test]
    fn prompt_and_parse() {
        let req = DraftRequest { term: "FID".into(), context: "FID com- pares features.".into(), page: Some(4), source: Some("Paper".into()), ..Default::default() };
        let p = draft_prompt(&req).unwrap();
        assert!(p.user.contains("compares") && !p.user.contains("com-"));
        assert!(p.system.contains("台灣繁體中文"));
        let f = parse_draft(&req, &p.quote, &serde_json::json!({ "summary": "比較特徵。", "supported": true })).unwrap();
        assert_eq!(f.summary.as_deref(), Some("比較特徵。"));
        assert_eq!(f.body.as_deref(), Some("> FID compares features.\n\n（Paper，p.4）"));
        assert!(parse_draft(&req, &p.quote, &serde_json::json!({ "summary": "", "supported": false })).is_err());
        assert!(draft_prompt(&DraftRequest { term: "x".into(), context: "  ".into(), ..Default::default() }).is_err());
        let en = draft_prompt(&DraftRequest { term: "x".into(), context: "y".into(), lang: Some("en".into()), ..Default::default() }).unwrap();
        assert!(en.system.contains("English"));
    }
}
