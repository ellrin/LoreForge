//! 修訂: adding to a paper that already exists — new data after a review,
//! a figure a supervisor asked for, two more citations. Each request gets a
//! conversation (an interview group) about where it belongs and what it
//! changes around it, then a concrete plan: paragraph-level changes the
//! author accepts or rejects one by one. A change is applied only when the
//! paragraph it targets still reads as it did when the plan was made.

use super::session::Session;
use crate::compose::{json_array, json_object, json_string, json_string_or_null, Prompt};
use crate::paper::{self, Section};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RevisionStatus {
    /// Being discussed.
    #[default]
    Open,
    /// A plan exists.
    Planned,
    /// Finished (changes applied or rejected).
    Done,
    Dropped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Change {
    pub id: String,
    /// File under `sections/`.
    pub file: String,
    /// insert-after | insert-before | replace | append
    pub action: String,
    /// Paragraph index in the section (not used by append).
    #[serde(default)]
    pub index: Option<usize>,
    /// The target paragraph as it read when the plan was made.
    #[serde(default)]
    pub before: String,
    /// New LaTeX (a paragraph, or a figure/table block).
    pub text: String,
    /// Why (for the author).
    #[serde(default)]
    pub reason: String,
    /// None = undecided, Some(true) = applied, Some(false) = rejected.
    #[serde(default)]
    pub applied: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RevisionPlan {
    /// What the revision does, for the author.
    pub summary: String,
    pub changes: Vec<Change>,
    /// Knock-on effects elsewhere that the plan does not change itself.
    #[serde(default)]
    pub effects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Revision {
    pub id: String,
    /// What was asked for, in the author's words (or the reviewer's).
    pub request: String,
    /// The interview group that discusses it.
    pub group: String,
    /// Reference keys the request adds.
    #[serde(default)]
    pub refs: Vec<String>,
    /// A new figure file under `figures/`.
    #[serde(default)]
    pub figure: Option<String>,
    #[serde(default)]
    pub status: RevisionStatus,
    #[serde(default)]
    pub plan: Option<RevisionPlan>,
}

/// The paper as numbered paragraphs, per section file. `full` = whole
/// paragraphs (for planning); otherwise the first sentence of each (for the
/// conversation). Cut at `max_chars`.
pub fn paper_map(paper_dir: &Path, full: bool, max_chars: usize) -> String {
    let mut out = String::new();
    for s in paper::load_paper(paper_dir) {
        out += &format!("\n### {} — {}\n", s.file, s.title);
        for (i, p) in s.paragraphs.iter().enumerate() {
            let p = p.trim();
            let shown = if full {
                p.to_string()
            } else {
                let first = p.split_inclusive(". ").next().unwrap_or(p);
                let first: String = first.chars().take(220).collect();
                if first.len() < p.len() { format!("{first}…") } else { first }
            };
            out += &format!("[{i}] {}\n", shown.replace('\n', " "));
        }
    }
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>() + "\n[… paper cut …]\n";
    }
    out
}

/// The revision block put into a conversation turn about it.
pub fn turn_context(r: &Revision, paper_dir: &Path) -> String {
    let mut s = format!(
        "\n## This group is a REVISION of the existing paper\nRequest: {}\n{}{}\
Ask, one at a time: what the new material shows and why it matters to the paper's argument; where it belongs \
(which section, before or after which paragraph); what it changes around it (claims, numbers, the abstract or the \
conclusion that must follow); for a figure, what the reader should see in it and its caption; for new references, \
what exactly each supports. Settle placement with the author — offer the likely spots as options, naming the \
paragraph by its number and first words. Placement and format are the author's decisions, not motives: once \
the author settles them, grade those items `comply` and do not ask why.\n\
The paper as it stands (section file, [paragraph number] first sentence):\n",
        r.request,
        if r.refs.is_empty() { String::new() } else { format!("References to add: {}\n", r.refs.join(", ")) },
        r.figure.as_deref().map(|f| format!("New figure file: figures/{f}\n")).unwrap_or_default()
    );
    s += &paper_map(paper_dir, false, 20_000);
    s
}

pub fn plan_schema() -> Value {
    let change = json_object(&[
        ("file", json_string()),
        ("action", json_string()),
        ("index", json!({ "anyOf": [{ "type": "integer" }, { "type": "null" }] })),
        ("text", json_string()),
        ("reason", json_string()),
    ]);
    json_object(&[("summary", json_string()), ("changes", json_array(change)), ("effects", json_array(json_string())), ("caption", json_string_or_null())])
}

pub fn plan_prompt(key_value: &str, s: &Session, r: &Revision, paper_dir: &Path, material: &str, style: &str, conversation: &str) -> Prompt {
    let system = "You revise an existing paper to take in something new, in the author's own voice. Produce the \
smallest set of paragraph-level changes that does it well.\n\
Rules:\n\
1. Each change names a section `file`, an `action` (insert-after, insert-before or replace a paragraph by its \
`index`; append to the end of the section with index null) and the new LaTeX `text`. A replace keeps what the old \
paragraph said unless the revision changes it.\n\
2. Put new material where the conversation settled it. When it changes something elsewhere (a number, a claim, the \
abstract, the conclusion, a sentence that now contradicts it), change that too — or, when it needs the author, list \
it in `effects`.\n\
3. A new figure gets a figure block with \\\\includegraphics[width=\\\\linewidth]{figures/<file>}, a caption that says \
what the reader should see (no numbers it does not show), a \\\\label{fig:...}, and a sentence in the text that \
points to it.\n\
4. Cite with \\\\cite{key}, only keys in the paper's references or the ones the request adds, within the purpose the \
author noted for each.\n\
5. New numbers come only from the request, the record or the conversation. Never invent results or references.\n\
6. Keep the author's style (paragraph length, crisp reasons, no generic closing). Write English for the paper; \
`reason`, `summary` and `effects` in Traditional Chinese with full-width punctuation.\n\
Reply with JSON only.";
    let mut user = super::prompts::context(key_value, s, material);
    user += &format!("\n## Revision request\n{}\n", r.request);
    if !r.refs.is_empty() {
        user += &format!("References to add: {}\n", r.refs.join(", "));
    }
    if let Some(f) = &r.figure {
        user += &format!("New figure file: figures/{f}\n");
    }
    user += "\n## The conversation about it\n";
    user += conversation;
    if !style.trim().is_empty() {
        user += "\n## The author's style\n";
        user += style;
    }
    user += "\n## The paper (section file, [paragraph index] full text)\n";
    user += &paper_map(paper_dir, true, 90_000);
    Prompt { system: system.into(), user }
}

#[derive(Debug, Deserialize)]
struct RawPlan {
    summary: String,
    changes: Vec<RawChange>,
    effects: Vec<String>,
    #[allow(dead_code)]
    caption: Option<String>,
}
#[derive(Debug, Deserialize)]
struct RawChange {
    file: String,
    action: String,
    index: Option<usize>,
    text: String,
    reason: String,
}

/// Parse a plan and pin each change to the paragraph it targets now:
/// changes to files or paragraphs that do not exist are dropped.
pub fn parse_plan(reply: &str, paper_dir: &Path) -> Result<RevisionPlan, String> {
    let r: RawPlan = super::prompts::json_in(reply)?;
    let sections = paper::load_paper(paper_dir);
    let stem = |f: &str| f.trim().trim_end_matches(".tex").trim_end_matches('s').to_lowercase();
    let mut changes = vec![];
    let mut effects: Vec<String> = r.effects.into_iter().filter(|e| !e.trim().is_empty()).collect();
    for c in r.changes {
        if c.text.trim().is_empty() {
            continue;
        }
        // Tolerate "results.tex" for "result.tex", or a section title for its file.
        let sec = sections
            .iter()
            .find(|s| s.file == c.file.trim())
            .or_else(|| sections.iter().find(|s| stem(&s.file) == stem(&c.file)))
            .or_else(|| sections.iter().find(|s| stem(&crate::text::strip_latex(&s.title)) == stem(&c.file)));
        let action = c.action.trim().to_lowercase().replace(['_', ' '], "-");
        let target = sec.and_then(|sec| match action.as_str() {
            "append" => Some(String::new()),
            "insert-after" | "insert-before" | "replace" => c.index.and_then(|i| sec.paragraphs.get(i)).cloned(),
            _ => None,
        });
        let (Some(sec), Some(before)) = (sec, target) else {
            let head: String = c.text.chars().take(60).collect();
            effects.push(format!("有一項修改對不上論文（{} ¶{}，{}）：「{head}…」，沒有列入，請重新產生或自己處理", c.file, c.index.map(|i| i.to_string()).unwrap_or("-".into()), c.action));
            continue;
        };
        changes.push(Change {
            id: format!("x{}", changes.len() + 1),
            file: sec.file.clone(),
            index: if action == "append" { None } else { c.index },
            action,
            before,
            text: c.text.trim().to_string(),
            reason: c.reason,
            applied: None,
        });
    }
    if changes.is_empty() {
        return Err("修改方案裡沒有可以套用的修改".into());
    }
    Ok(RevisionPlan { summary: r.summary, changes, effects })
}

/// Apply one change to its section file. The target paragraph is found by
/// its text (it may have moved if earlier changes inserted paragraphs); if it
/// no longer exists, nothing is written.
pub fn apply(paper_dir: &Path, c: &Change) -> Result<(), String> {
    if !paper::safe_name(&c.file) {
        return Err("invalid file".into());
    }
    let path = paper_dir.join("sections").join(&c.file);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", c.file))?;
    let mut sec: Section = paper::parse_section(&c.file, &text);
    let new = c.text.trim().to_string();
    if c.action == "append" {
        sec.paragraphs.push(new);
    } else {
        // Same words, whatever the line breaks (saving rewrites one sentence per line).
        let norm = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
        let want = norm(&c.before);
        let at = sec
            .paragraphs
            .iter()
            .position(|p| norm(p) == want)
            .ok_or("那一段已經改過了，請重新產生修改方案")?;
        match c.action.as_str() {
            "insert-after" => sec.paragraphs.insert(at + 1, new),
            "insert-before" => sec.paragraphs.insert(at, new),
            "replace" => sec.paragraphs[at] = new,
            _ => return Err("unknown action".into()),
        }
    }
    paper::write_atomic(&path, &paper::render_section(&sec)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paper() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("pc-revise-{}-{}", std::process::id(), rand_tag()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sections")).unwrap();
        std::fs::write(d.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
        std::fs::write(d.join("sections/results.tex"), "\\section{Results}\nFirst paragraph. It has two sentences.\n\nSecond paragraph about Table 1.\n").unwrap();
        d
    }
    fn rand_tag() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    #[test]
    fn maps_the_paper_by_paragraph() {
        let p = paper();
        let short = paper_map(&p, false, 10_000);
        assert!(short.contains("### results.tex — Results") && short.contains("[0] First paragraph. …") && short.contains("[1] Second paragraph"));
        assert!(paper_map(&p, true, 10_000).contains("[0] First paragraph. It has two sentences."));
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn plans_pin_their_targets_and_apply_safely() {
        let p = paper();
        let plan = parse_plan(
            r#"{"summary":"加入新的外部驗證","changes":[
              {"file":"results.tex","action":"insert-after","index":1,"text":"External validation on 120 frames gave Dice 0.71.","reason":"新數據放在主表之後"},
              {"file":"results.tex","action":"replace","index":0,"text":"First paragraph, revised.","reason":"更新數字"},
              {"file":"nope.tex","action":"append","index":null,"text":"x","reason":""},
              {"file":"results.tex","action":"replace","index":9,"text":"x","reason":""},
              {"file":"result","action":"Insert_After","index":0,"text":"Tolerated.","reason":""}
            ],"effects":["摘要的數字要更新"],"caption":null}"#,
            &p,
        )
        .unwrap();
        assert_eq!(plan.changes.len(), 3, "unknown files and paragraphs are dropped");
        assert_eq!((plan.changes[2].file.as_str(), plan.changes[2].action.as_str()), ("results.tex", "insert-after"), "near-miss names and actions are understood");
        assert_eq!(plan.effects.iter().filter(|e| e.contains("對不上")).count(), 2, "dropped changes are reported, never silent");
        assert_eq!(plan.changes[0].before, "Second paragraph about Table 1.");
        // Saving rewrites the file one sentence per line; targets still match.
        let reflowed = "\\section{Results}\nFirst paragraph.\nIt has two sentences.\n\nSecond paragraph about Table 1.\n";
        std::fs::write(p.join("sections/results.tex"), reflowed).unwrap();
        // Apply the replace first: the insert still finds its paragraph by text.
        apply(&p, &plan.changes[1]).unwrap();
        apply(&p, &plan.changes[0]).unwrap();
        let s = paper::load_paper(&p).remove(0);
        assert_eq!(s.paragraphs, ["First paragraph, revised.", "Second paragraph about Table 1.", "External validation on 120 frames gave Dice 0.71."]);
        // Applying the replace again fails: its paragraph is gone.
        assert!(apply(&p, &plan.changes[1]).is_err());
        let _ = std::fs::remove_dir_all(&p);
    }
}
