//! The Key value check: one small model call judges the Key value as a research paper's thesis
//! and answers which problems from a fixed list apply, plus one or two rewrites. Advice only:
//! the page shows it next to the field and never waits for it (保存、下一步、匯入 go on).
//!
//! The criteria come from the workspace's `style/_thesis-statement.md` when it exists (the
//! author's notes on thesis statements, read for this one task, bounded by
//! [`CRITERIA_MAX_CHARS`]); otherwise a neutral built-in list ([`FALLBACK`], no examples).
//! A research paper's thesis is *supportable by evidence*: the prompt says so instead of
//! asking for a debatable opinion.

use crate::compose::{json_array, json_object, json_string, Prompt};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;

/// The author's notes on thesis statements, in `style/` (`_`: never sent with the style notes).
pub const FILE: &str = "_thesis-statement.md";
pub const CRITERIA_MAX_CHARS: usize = 6000;
pub const MAX_REWRITES: usize = 2;

/// The problems the model may name, with the label the page shows.
pub const CODES: &[(&str, &str)] = &[
    ("fragment", "不是完整句子"),
    ("question", "是問句，沒有立場"),
    ("bare_fact", "只是事實，沒有主張"),
    ("topic_only", "只宣告主題"),
    ("too_broad", "太廣"),
    ("too_narrow", "太窄"),
    ("two_ideas", "不只一個主要想法"),
    ("not_supportable", "無法用證據支持"),
    ("hedge", "弱化語氣（I think、我認為）"),
    ("vague", "用語模糊"),
    ("no_reason", "沒有理由（because …）"),
];

pub fn label(code: &str) -> Option<&'static str> {
    CODES.iter().find(|(c, _)| *c == code).map(|(_, l)| *l)
}

/// Generic criteria, used when the workspace has no notes (public, neutral, no examples).
pub const FALLBACK: &str = "A research paper's thesis (its key value) is:\n\
- one complete sentence (or two at most), not a fragment and not a question;\n\
- a claim the paper's evidence can support (results, analysis, data) — not a bare fact everyone accepts and not an announcement of the topic (\"This paper is about …\");\n\
- specific: names what is studied and what is claimed about it; not so broad that one paper cannot support it, not so narrow that nothing is left to show;\n\
- one controlling idea, not two or three loosely joined;\n\
- direct: no hedges such as \"I think\", \"I believe\", \"in my opinion\", and no vague words (things, good, interesting, nice);\n\
- ideally topic + claim + reason (\"…, because …\" / \"… by …\"), which also previews how the paper argues.";

/// The criteria text for the prompt: the workspace's notes (bounded) or [`FALLBACK`].
pub fn criteria(workspace: Option<&Path>) -> String {
    let notes = workspace.and_then(|w| std::fs::read_to_string(w.join(crate::draft::STYLE_DIR).join(FILE)).ok()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    match notes {
        Some(t) if t.chars().count() > CRITERIA_MAX_CHARS => t.chars().take(CRITERIA_MAX_CHARS).collect::<String>() + "\n[…]",
        Some(t) => t,
        None => FALLBACK.to_string(),
    }
}

pub fn schema() -> Value {
    let codes: Vec<&str> = CODES.iter().map(|(c, _)| *c).collect();
    json_object(&[("problems", json_array(json!({ "type": "string", "enum": codes }))), ("rewrites", json_array(json_string()))])
}

/// The request: the key value, its paper's title when known, and the criteria.
pub fn prompt(key_value: &str, title: &str, criteria: &str) -> Prompt {
    let (_, max) = crate::meta::key_value_length(key_value);
    let lang = if max == crate::meta::KEY_VALUE_MAX_CJK { "the same language as the key value (Traditional Chinese where it is Chinese)" } else { "English" };
    let codes = CODES.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(", ");
    let system = format!(
        "You check a research paper's key value: the one sentence the whole paper argues, used as its thesis. \
Judge it against the criteria below, for a research paper: the claim must be supportable by the paper's evidence, \
not merely debatable opinion.\n\
Answer with JSON: {{\"problems\": [codes], \"rewrites\": [text]}}.\n\
- problems: only codes from this list that clearly apply: {codes}. An empty list when the key value is fine.\n\
- rewrites: at most {MAX_REWRITES} better versions that keep the author's meaning, field and facts (never invent results, \
numbers or methods); written in {lang}; each at most {max} {unit}. No rewrites when there are no problems.\n\
The criteria (the author's notes on thesis statements; their examples illustrate problems, never copy their subjects):\n{criteria}",
        unit = if max == crate::meta::KEY_VALUE_MAX_CJK { "characters" } else { "words" },
    );
    let mut user = format!("Key value: {}\n", key_value.trim());
    if !title.trim().is_empty() {
        user += &format!("Paper title: {}\n", title.trim());
    }
    Prompt { system, user }
}

/// What the page gets.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Verdict {
    /// `{ code, label }` of every problem named.
    pub problems: Vec<Problem>,
    pub rewrites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Problem {
    pub code: String,
    pub label: String,
}

/// Read the reply; unknown codes, repeats, empty or over-long rewrites and the key value
/// itself are dropped. A reply that is not JSON is an empty verdict (advice, never an error).
pub fn parse(reply: &str, key_value: &str) -> Verdict {
    let v: Value = match (reply.find('{'), reply.rfind('}')) {
        (Some(a), Some(b)) if b > a => serde_json::from_str(&reply[a..=b]).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let strings = |k: &str| v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).collect::<Vec<_>>()).unwrap_or_default();
    let mut out = Verdict::default();
    for c in strings("problems") {
        if let Some(l) = label(&c) {
            if !out.problems.iter().any(|p| p.code == c) {
                out.problems.push(Problem { code: c, label: l.to_string() });
            }
        }
    }
    let same = |a: &str, b: &str| a.split_whitespace().collect::<Vec<_>>() == b.split_whitespace().collect::<Vec<_>>();
    for r in strings("rewrites") {
        let (n, max) = crate::meta::key_value_length(&r);
        if r.is_empty() || n > max || same(&r, key_value) || out.rewrites.iter().any(|x| same(x, &r)) {
            continue;
        }
        out.rewrites.push(r);
        if out.rewrites.len() == MAX_REWRITES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criteria_from_the_notes_or_the_neutral_list() {
        let ws = std::env::temp_dir().join(format!("pc-thesis-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        assert_eq!(criteria(None), FALLBACK);
        assert_eq!(criteria(Some(&ws)), FALLBACK, "no notes: the built-in list");
        std::fs::create_dir_all(ws.join("style")).unwrap();
        std::fs::write(ws.join("style").join(FILE), "Bad: a topic announcement.\n".repeat(400)).unwrap();
        let c = criteria(Some(&ws));
        assert!(c.starts_with("Bad: a topic announcement.") && c.chars().count() <= CRITERIA_MAX_CHARS + 4, "bounded");
        let p = prompt("Improve fracture detection", "", &c);
        assert!(p.system.contains("Bad: a topic announcement.") && p.system.contains("supportable by the paper's evidence"));
        assert!(p.user.contains("Key value: Improve fracture detection"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn the_fallback_has_no_examples() {
        // generic criteria only: nothing copied from anyone's notes
        assert!(!FALLBACK.contains("Bad") && !FALLBACK.contains("Better") && !FALLBACK.contains('壞'));
    }

    #[test]
    fn the_request_asks_for_codes_and_rewrites_in_the_key_values_language() {
        let p = prompt("以大量資料提升判讀準確率是否可行", "A title", FALLBACK);
        assert!(p.system.contains("Traditional Chinese") && p.system.contains("50 characters"));
        assert!(p.user.contains("Paper title: A title"));
        let en = prompt("A loss that keeps attention on the lesion", "", FALLBACK);
        assert!(en.system.contains("written in English") && en.system.contains("30 words"));
        let s = schema();
        assert_eq!(s["properties"]["problems"]["items"]["enum"].as_array().unwrap().len(), CODES.len());
        assert_eq!(s["required"], json!(["problems", "rewrites"]));
    }

    #[test]
    fn replies_are_cleaned() {
        let kv = "Should models use more data?";
        let v = parse(r#"```json
{"problems":["question","question","made_up","vague"],"rewrites":["Should models use more data?","Training on a larger chest radiograph set improves trauma finding detection, because the findings share visual features.","x x", "Training on a larger chest radiograph set improves trauma finding detection, because the findings share visual features.", "third"]}
```"#, kv);
        assert_eq!(v.problems.iter().map(|p| p.code.as_str()).collect::<Vec<_>>(), ["question", "vague"]);
        assert_eq!(v.problems[0].label, "是問句，沒有立場");
        assert_eq!(v.rewrites.len(), MAX_REWRITES);
        assert!(!v.rewrites.iter().any(|r| r == kv), "the key value itself is not a suggestion");
        let long = "word ".repeat(40);
        assert!(parse(&format!(r#"{{"problems":[],"rewrites":["{long}"]}}"#), kv).rewrites.is_empty(), "over the limit");
        assert_eq!(parse("not json", kv), Verdict::default(), "garbage: no advice, no error");
    }
}
