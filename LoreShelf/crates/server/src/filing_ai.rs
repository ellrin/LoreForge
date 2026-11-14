//! 歸位 phase D: where a file the rules cannot place goes, guessed by a model, with 3–5 English
//! labels for LoreShelf's own search (never Finder tags, never the KnowledgeBase).
//!
//! Two routes (author, 2026-11-13):
//! - a paper or a slide deck (`Item::cloud_ok`) → Claude Haiku (`shelf-filing-papers`) with its
//!   first-pages text only — never its file name, path or folder names;
//! - everything else → the local model only (`shelf-filing-files`), with its name and first text.
//!
//! A local answer must name one of [`TARGETS`] word for word, else it is dropped. A research file
//! gets a short English topic; it joins an existing `Research/<Topic>` when the names agree
//! ([`match_topic`]), else it is a new topic the author sees (`new-topic`). Answers are kept by
//! content id in `<catalog>/filing-ai.json` (local only, like the plan), so a moved file keeps its
//! answer; a model-placed row never moves without the author's look (`filing::certain` is false).

use crate::filing::{OPEN, Plan, Row};
use lore_llm::{LlmProvider, LlmRequest, Message, complete_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "filing-ai.json";
const PAPER_BATCH: usize = 10;
const FILE_BATCH: usize = 8;
const PAPER_TEXT: usize = 1200;
const FILE_TEXT: usize = 600;

/// The places a local answer may name (a research one also names a topic).
pub const TARGETS: [&str; 22] = [
    "Research/Papers",
    "Research/Manuscripts",
    "Research/Slides",
    "Research/Datasets",
    "Research/Code",
    "Courses",
    "School/Travel",
    "School/Funding",
    "School/Forms",
    "School/IRB",
    "Career",
    "Templates",
    "Mentoring",
    "Learning/Languages",
    "Library/Books",
    "Software/Manuals",
    "Personal/IDs",
    "Personal/Finance",
    "Personal/Medical",
    "Personal/Contracts",
    "Media/Photos",
    "Archive",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Guess {
    /// One of [`TARGETS`].
    pub target: String,
    /// The research topic, in English (research targets only).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub topic: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// `cloud` or `local`.
    pub by: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Guesses {
    pub version: u32,
    /// content id → guess
    pub guesses: BTreeMap<String, Guess>,
}

impl Guesses {
    pub fn file(catalog_dir: &Path) -> PathBuf {
        catalog_dir.join(FILE)
    }
    pub fn load(catalog_dir: &Path) -> Guesses {
        std::fs::read(Guesses::file(catalog_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Guesses { version: 1, ..Guesses::default() })
    }
    pub fn save(&self, catalog_dir: &Path) -> Result<(), String> {
        let b = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        lore_catalog::store::write_atomic(&Guesses::file(catalog_dir), &b).map_err(|e| e.to_string())
    }
}

/// One file to ask about.
#[derive(Debug, Clone)]
pub struct Ask {
    pub id: String,
    pub name: String,
    pub text: String,
    /// A paper or slide deck with text: may go to the cloud (without its name).
    pub cloud: bool,
}

fn clip(s: &str, n: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(n).collect()
}

const PAPER_SYSTEM: &str = "You sort a researcher's papers and slide decks into research topics. Each item is the first text of one document (PDF extraction, may be broken). For each, give:\n- topic: the research area it belongs to, 1–3 English words, Title Case, broad enough that several papers share it (e.g. \"Medical Image Segmentation\", \"Fuzzy Systems\", \"Generative Models\"). Reuse a topic from the list given when one fits.\n- labels: 3–5 short English keywords for search (method names, datasets, venue, year).\nAnswer every item; copy its k.";

const FILE_SYSTEM: &str = "你幫一位研究者（博士生）把散落的檔案歸到他的新資料夾。每個檔案有檔名和開頭的文字（可能沒有）。\n對每個檔案回答：\n- target：只能從下面清單照抄一個；真的判斷不了就寫 unknown。\n- topic：target 是 Research/ 開頭時，給研究主題，1–3 個英文字、Title Case；其他留空。\n- labels：3–5 個英文關鍵字（給搜尋用：主題、文件種類、年份、場合）。\n清單：\n";

pub fn paper_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "items": { "type": "array", "items": {
            "type": "object",
            "properties": { "k": { "type": "string" }, "topic": { "type": "string" }, "labels": { "type": "array", "items": { "type": "string" } } },
            "required": ["k", "topic", "labels"],
            "additionalProperties": false
        } } },
        "required": ["items"],
        "additionalProperties": false
    })
}

pub fn file_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "items": { "type": "array", "items": {
            "type": "object",
            "properties": { "k": { "type": "string" }, "target": { "type": "string" }, "topic": { "type": "string" }, "labels": { "type": "array", "items": { "type": "string" } } },
            "required": ["k", "target", "labels"],
            "additionalProperties": false
        } } },
        "required": ["items"],
        "additionalProperties": false
    })
}

/// A cloud request: the text only, keyed `d1`, `d2`, … — no name, no path, no folder.
pub fn paper_request(batch: &[&Ask], topics: &[String]) -> LlmRequest {
    let list = batch.iter().enumerate().map(|(i, a)| format!("- k: d{}\n  text: {}", i + 1, clip(&a.text, PAPER_TEXT))).collect::<Vec<_>>().join("\n");
    let known = if topics.is_empty() { String::new() } else { format!("Topics so far: {}\n\n", topics.join("; ")) };
    let mut req = LlmRequest::chat(PAPER_SYSTEM, vec![Message::user(format!("{known}Documents ({}):\n{list}", batch.len()))]);
    req.json_schema = Some(paper_schema());
    req.max_tokens = 4000;
    req
}

/// A local request: name and first text.
pub fn file_request(batch: &[&Ask], topics: &[String]) -> LlmRequest {
    let list = batch.iter().enumerate().map(|(i, a)| format!("- k: f{}\n  檔名: {}\n  開頭: {}", i + 1, a.name, clip(&a.text, FILE_TEXT))).collect::<Vec<_>>().join("\n");
    let known = if topics.is_empty() { String::new() } else { format!("已有的研究主題（合適就沿用）：{}\n\n", topics.join("; ")) };
    let mut req = LlmRequest::chat(format!("{FILE_SYSTEM}{}", TARGETS.join("\n")), vec![Message::user(format!("{known}檔案（{} 個）：\n{list}", batch.len()))]);
    req.json_schema = Some(file_schema());
    req.max_tokens = 3000;
    req
}

fn labels_of(x: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in x["labels"].as_array().into_iter().flatten().filter_map(|l| l.as_str()) {
        let l = l.split_whitespace().collect::<Vec<_>>().join(" ");
        if !l.is_empty() && l.chars().count() <= 40 && l.is_ascii() && !out.iter().any(|o| o.eq_ignore_ascii_case(&l)) {
            out.push(l);
        }
    }
    out.truncate(5);
    out
}

/// A topic usable as a folder name: ASCII words, at most 4, at most 40 characters.
pub fn clean_topic(t: &str) -> String {
    let t: String = t.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { ' ' }).collect();
    let words: Vec<&str> = t.split_whitespace().take(4).collect();
    let t = words.join(" ").chars().take(40).collect::<String>().trim().to_string();
    if matches!(t.to_lowercase().as_str(), "unknown" | "none" | "misc" | "other" | "general" | "n a" | "na") { String::new() } else { t }
}

/// The answers to one batch, by the asked file's id; a local answer naming no listed place, or a
/// research place without a topic, is dropped.
pub fn read_answer(v: &Value, batch: &[&Ask], cloud: bool) -> BTreeMap<String, Guess> {
    let prefix = if cloud { "d" } else { "f" };
    let mut out = BTreeMap::new();
    for x in v["items"].as_array().into_iter().flatten() {
        let Some(i) = x["k"].as_str().and_then(|k| k.trim().strip_prefix(prefix)).and_then(|n| n.parse::<usize>().ok()) else { continue };
        let Some(a) = i.checked_sub(1).and_then(|i| batch.get(i)) else { continue };
        let topic = clean_topic(x["topic"].as_str().unwrap_or(""));
        let target = if cloud { "Research/Papers".to_string() } else { x["target"].as_str().unwrap_or("").trim().to_string() };
        if !TARGETS.contains(&target.as_str()) || target.starts_with("Research/") && topic.is_empty() {
            continue;
        }
        let topic = if target.starts_with("Research/") { topic } else { String::new() };
        out.insert(a.id.clone(), Guess { target, topic, labels: labels_of(x), by: if cloud { "cloud" } else { "local" }.into() });
    }
    out
}

/// Ask about every file not answered yet, papers and decks in the cloud, the rest locally; saved
/// after each batch. `cloud` may be `None` (then papers go locally too). Returns (asked, answered).
pub async fn run(catalog_dir: &Path, asks: &[Ask], cloud: Option<&dyn LlmProvider>, local: &dyn LlmProvider, mut progress: impl FnMut(usize, usize)) -> Result<(usize, usize), String> {
    let mut g = Guesses::load(catalog_dir);
    let todo: Vec<&Ask> = asks.iter().filter(|a| !g.guesses.contains_key(&a.id)).collect();
    let (papers, files): (Vec<&Ask>, Vec<&Ask>) = todo.iter().partition(|a| a.cloud && cloud.is_some());
    let (total, mut done, mut answered) = (todo.len(), 0, 0);
    for (list, size, on_cloud) in [(papers, PAPER_BATCH, true), (files, FILE_BATCH, false)] {
        let p: &dyn LlmProvider = if on_cloud { cloud.unwrap() } else { local };
        let mut failed = 0;
        for chunk in list.chunks(size) {
            let mut ask: Vec<&Ask> = chunk.to_vec();
            for _ in 0..2 {
                if ask.is_empty() {
                    break;
                }
                let mut topics: Vec<String> = g.guesses.values().filter(|x| !x.topic.is_empty()).map(|x| x.topic.clone()).collect();
                topics.sort();
                topics.dedup();
                let req = if on_cloud { paper_request(&ask, &topics) } else { file_request(&ask, &topics) };
                match complete_json(p, &req).await {
                    Ok(v) => {
                        let got = read_answer(&v, &ask, on_cloud);
                        answered += got.len();
                        ask.retain(|a| !got.contains_key(&a.id));
                        g.guesses.extend(got);
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("一批沒有結果：{e}");
                        if answered == 0 && failed >= 2 {
                            return Err(format!("模型沒有回答：{e}"));
                        }
                        break;
                    }
                }
            }
            g.save(catalog_dir)?;
            done += chunk.len();
            progress(done, total);
        }
    }
    Ok((total, answered))
}

/// Words that say nothing about what a file is about (its format, "a document", a bare year).
const EMPTY_LABELS: [&str; 38] = [
    "null", "none", "na", "untitled", "pdf", "document", "documents", "file", "files", "doc", "docx", "csv", "xlsx", "pptx", "text", "data", "code", "script", "programming", "research", "paper", "papers", "report", "notes", "note", "misc", "other", "unknown", "general", "metadata", "sample", "test", "draft", "version", "final", "academic", "study", "slides",
];

/// What the page shows and the neighbours are weighed by: id → the research topic (after the
/// author's merges) then the labels, without the empty ones and bare numbers; at most 5.
pub fn shown_labels(catalog_dir: &Path) -> std::collections::HashMap<String, Vec<String>> {
    let merges = crate::filing::Settings::load(catalog_dir).map(|s| s.topics).unwrap_or_default();
    let merged: BTreeMap<String, &String> = merges.iter().map(|(k, v)| (norm(k), v)).collect();
    let mut out = std::collections::HashMap::new();
    for (id, g) in Guesses::load(catalog_dir).guesses {
        let topic = clean_topic(&g.topic);
        let topic = merged.get(&norm(&topic)).map(|t| (*t).clone()).unwrap_or(topic);
        let mut labels: Vec<String> = Vec::new();
        for l in std::iter::once(topic).chain(g.labels) {
            let k = norm(&l);
            if k.is_empty() || k.chars().all(|c| c.is_ascii_digit()) || EMPTY_LABELS.contains(&k.as_str()) || labels.iter().any(|x| norm(x) == k) {
                continue;
            }
            labels.push(l);
        }
        labels.truncate(5);
        if !labels.is_empty() {
            out.insert(id, labels);
        }
    }
    out
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

/// An existing topic folder with the same name (ignoring case, spaces, `_` and `-`), or one whose
/// name contains the other's (three letters at least); else `None` (a new topic).
pub fn match_topic<'a>(topic: &str, existing: &'a [String]) -> Option<&'a String> {
    let t = norm(topic);
    if t.is_empty() {
        return None;
    }
    existing.iter().find(|e| norm(e) == t).or_else(|| {
        existing.iter().filter(|e| {
            let n = norm(e);
            n.len() >= 3 && (t.contains(&n) || n.contains(&t) && t.len() >= 3)
        })
        .min_by_key(|e| e.len())
    })
}

/// The research topic folders already in the plan or in the new tree.
pub fn topics(plan: &Plan) -> Vec<String> {
    let mut out: Vec<String> = plan.rows.iter().filter_map(|r| r.target.as_deref()).filter_map(|t| t.strip_prefix("Research/")).map(|t| t.split('/').next().unwrap_or("").to_string()).filter(|t| !t.is_empty() && t != OPEN).collect();
    if let Ok(rd) = std::fs::read_dir(plan.root.join("Research")) {
        out.extend(rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()));
    }
    out.sort();
    out.dedup();
    out
}

/// Fills the rows the rules left open (no place, or `Research/?/…`) from the answers: flags
/// `by-model` (and `new-topic`), the reason says which model.
pub fn apply(plan: &mut Plan, g: &Guesses, merges: &BTreeMap<String, String>) {
    let existing = topics(plan);
    let merged: BTreeMap<String, &String> = merges.iter().map(|(k, v)| (norm(k), v)).collect();
    for r in plan.rows.iter_mut() {
        let open = r.target.as_deref().is_none_or(|t| t.contains(OPEN));
        if !open || r.id.is_empty() {
            continue;
        }
        let Some(x) = g.guesses.get(&r.id) else { continue };
        if x.target.starts_with("Research/") && clean_topic(&x.topic).is_empty() {
            continue; // an old answer without a topic: still open
        }
        let x = &match merged.get(&norm(&x.topic)) {
            Some(to) => Guess { topic: (*to).clone(), ..x.clone() },
            None => x.clone(),
        };
        if x.target == "Archive" || x.target == "Courses" {
            continue; // never by default (Archive) / needs its term (Courses)
        }
        place_row(r, x, &existing);
    }
}

fn place_row(r: &mut Row, x: &Guess, existing: &[String]) {
    let target = if let Some(leaf) = x.target.strip_prefix("Research/") {
        // a rule's own leaf wins (`Research/?/Manuscripts` stays a manuscript)
        let leaf = r.target.as_deref().and_then(|t| t.strip_prefix(&format!("Research/{OPEN}/"))).unwrap_or(leaf).to_string();
        let topic = match match_topic(&x.topic, existing) {
            Some(e) => e.clone(),
            None => {
                r.flags.push("new-topic".into());
                x.topic.clone()
            }
        };
        format!("Research/{topic}/{leaf}")
    } else {
        x.target.clone()
    };
    r.target = Some(target);
    r.flags.retain(|f| f != "need-topic");
    r.flags.push("by-model".into());
    r.reason = format!("{}：{}", if x.by == "cloud" { "Claude 看內容" } else { "本機模型" }, x.labels.join(", "));
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_llm::types::{Capabilities, LlmResponse};

    /// Answers every item; records what it was sent.
    struct Mock(std::sync::Mutex<Vec<String>>, &'static str);

    #[async_trait::async_trait]
    impl LlmProvider for Mock {
        fn id(&self) -> String {
            "mock".into()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { json: true, vision: false, web: false, streaming: false, context: 100_000, needs_network: false }
        }
        async fn send(&self, req: &LlmRequest) -> Result<LlmResponse, lore_llm::LlmError> {
            let u = req.messages[0].text();
            self.0.lock().unwrap().push(u.clone());
            // (k, the item's lines)
            let mut ks: Vec<(String, String)> = Vec::new();
            for l in u.lines() {
                match l.trim().strip_prefix("- k: ") {
                    Some(k) => ks.push((k.to_string(), String::new())),
                    None => {
                        if let Some(last) = ks.last_mut() {
                            last.1.push_str(l);
                        }
                    }
                }
            }
            let items: Vec<Value> = ks
                .iter()
                .map(|(k, body)| match self.1 {
                    "cloud" => json!({ "k": k, "topic": "fuzzy systems!", "labels": ["fuzzy", "loss", "2025"] }),
                    // the odd file always gets a place that is not on the list
                    _ => json!({ "k": k, "target": if body.contains("odd") { "School/Misc" } else { "School/Travel" }, "labels": ["travel"] }),
                })
                .collect();
            Ok(LlmResponse { text: json!({ "items": items }).to_string(), provider: "mock".into(), sources: vec![], usage: None })
        }
    }

    fn ask(id: &str, name: &str, cloud: bool) -> Ask {
        Ask { id: id.into(), name: name.into(), text: format!("text of {id}"), cloud }
    }

    #[tokio::test]
    async fn papers_go_without_their_names_and_local_answers_must_name_a_listed_place() {
        let t = tempfile::tempdir().unwrap();
        let (c, l) = (Mock(Default::default(), "cloud"), Mock(Default::default(), "local"));
        let asks = vec![ask("p1", "ellrin_fuzzy_draft.pdf", true), ask("a", "travel form daidai.docx", false), ask("b", "odd.docx", false)];
        let (asked, answered) = run(t.path(), &asks, Some(&c), &l, |_, _| {}).await.unwrap();
        assert_eq!((asked, answered), (3, 2));
        let sent = c.0.lock().unwrap().join("\n");
        assert!(sent.contains("text of p1") && !sent.contains("ellrin") && !sent.contains("daidai") && !sent.contains(".pdf"), "{sent}");
        assert!(l.0.lock().unwrap().join("\n").contains("travel form daidai.docx"));
        let g = Guesses::load(t.path());
        assert_eq!(g.guesses["p1"], Guess { target: "Research/Papers".into(), topic: "fuzzy systems".into(), labels: vec!["fuzzy".into(), "loss".into(), "2025".into()], by: "cloud".into() });
        assert_eq!(g.guesses["a"].target, "School/Travel");
        assert!(!g.guesses.contains_key("b"), "a place off the list is dropped");
        // asked again only for what has no answer
        let (asked, _) = run(t.path(), &asks, Some(&c), &l, |_, _| {}).await.unwrap();
        assert_eq!(asked, 1);
    }

    #[test]
    fn topics_join_an_existing_folder_when_the_names_agree() {
        let ex = vec!["Fuzzy_Systems".to_string(), "seg".to_string(), "cv".to_string()];
        assert_eq!(match_topic("Fuzzy Systems", &ex).map(String::as_str), Some("Fuzzy_Systems"));
        assert_eq!(match_topic("Medical Image Segmentation", &ex).map(String::as_str), Some("seg"));
        assert_eq!(match_topic("Generative Models", &ex), None);
        let mut plan = Plan { made: "x".into(), root: "/nowhere".into(), rows: vec![] };
        plan.rows.push(Row { from: "/h/Downloads/a.pdf".into(), kind: "file".into(), files: 1, id: "a".into(), target: Some(format!("Research/{OPEN}/Manuscripts")), flags: vec!["need-topic".into()], ..Row::default() });
        plan.rows.push(Row { from: "/h/Downloads/b.pdf".into(), kind: "file".into(), files: 1, id: "b".into(), target: None, ..Row::default() });
        plan.rows.push(Row { from: "/h/Downloads/rib".into(), kind: "folder".into(), files: 3, target: Some("Research/rib".into()), ..Row::default() });
        let mut g = Guesses::default();
        g.guesses.insert("a".into(), Guess { target: "Research/Papers".into(), topic: "Rib".into(), labels: vec![], by: "cloud".into() });
        g.guesses.insert("b".into(), Guess { target: "Research/Papers".into(), topic: "Generative Models".into(), labels: vec![], by: "local".into() });
        let mut merges = BTreeMap::new();
        merges.insert("generative  models".to_string(), "GAN".to_string());
        g.guesses.insert("c".into(), Guess { target: "Research/Papers".into(), topic: "Unknown".into(), labels: vec![], by: "local".into() });
        plan.rows.push(Row { from: "/h/Downloads/c.pdf".into(), kind: "file".into(), files: 1, id: "c".into(), target: None, ..Row::default() });
        apply(&mut plan, &g, &merges);
        assert_eq!(plan.rows[3].target, None, "no topic, no place");
        assert_eq!(plan.rows[0].target.as_deref(), Some("Research/rib/Manuscripts"));
        assert_eq!(plan.rows[0].flags, vec!["by-model"]);
        assert_eq!(plan.rows[1].target.as_deref(), Some("Research/GAN/Papers"), "the author's merge");
        assert!(plan.rows[1].flags.contains(&"new-topic".to_string()));
        assert!(!crate::filing::certain(&plan.rows[1]));
    }
}
