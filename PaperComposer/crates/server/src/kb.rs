//! 知識卡: the author's KnowledgeBase cards while writing.
//!
//! Cards are `nodes/<id>.json` in KnowledgeBase's spaces (`kb_core::Node`). They are read
//! **read-only**, straight from disk, so this works when KnowledgeBase is not running; nothing is
//! ever written there, created or seeded. Spaces, in order: `KB_SPACES` (dirs separated by `:`),
//! `spaces` in KnowledgeBase's `config.toml` (`$KB_HOME` or `<config dir>/KnowledgeBase`), lore-home's
//! `private/KnowledgeBase`.
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | /api/kb/cards | `{paragraph?, q?, limit?}` → `{available, cards}`: cards whose title or alias occurs in the paragraph, or that match the search words |
//! | GET/PUT | /api/kb/settings | `{context}`: 生成時參考知識卡 (off by default), kept in the workspace's config.toml `[kb]` |
//! | POST | /api/kb/paraphrase | `{id, language?, engine?}` → `{text, paper?}`: the card's definition restated as a draft sentence (task `pc-kb-paraphrase`, class tidy) |
//! | POST | /api/papers/{name}/kb/notation | `{id, index}` → like reader's register: the card's symbol or equation into **this paper's** 對照表 (the card id in `kb`; the global registry is read, never written) |
//!
//! `relevant_cards(paragraph)` is the private-retrieval entry: the paragraph-generation prompt
//! (`pc-compose-paragraph`) gets the matching cards as context through `with_cards`, only when the
//! setting is on.

use super::{bad, paper_dir, writing_complete, Shared};
use crate::llm::EngineChoice;
use axum::{
    extract::{Path as AxPath, State},
    response::{IntoResponse, Json},
};
use kb_core::Node;
use pc_core::compose::Prompt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const APP: &str = "KnowledgeBase";

/// One source of a card; a `pdf` source with a sha is a paper in LoreReader.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CardSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub location: String,
    /// LoreReader's id for the PDF (the 64-hex sha256), when the source is one.
    pub sha: Option<String>,
    pub page: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CardSymbol {
    pub latex: String,
    pub meaning: String,
    pub equation: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Card {
    pub id: String,
    pub space: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub kind: String,
    pub summary: String,
    pub symbols: Vec<CardSymbol>,
    pub sources: Vec<CardSource>,
    /// The title or alias found in the paragraph ("" for a search hit).
    pub matched: String,
}

// ── where the spaces are ──

fn kb_app_dir() -> Option<PathBuf> {
    std::env::var_os("KB_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| dirs::config_dir().map(|d| d.join(APP)))
}

/// KnowledgeBase's spaces: the ones its server would mount, minus the seeding.
pub fn space_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = match std::env::var("KB_SPACES") {
        Ok(s) if !s.is_empty() => s.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect(),
        _ => vec![],
    };
    if v.is_empty() {
        let file = kb_app_dir().map(|d| d.join("config.toml")).filter(|p| p.is_file());
        if let Some(t) = file.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| t.parse::<toml::Table>().ok()) {
            v = t.get("spaces").and_then(|s| s.as_array()).into_iter().flatten().filter_map(|s| s.as_str()).map(lore_store::expand_home).collect();
        }
    }
    if v.is_empty() && let Some(d) = super::loreforge_home().app_dir(APP) {
        v.push(d);
    }
    v.retain(|d| d.join("space.toml").is_file());
    v
}

fn node_card(space: &str, n: Node) -> Card {
    let summary = if n.summary.trim().is_empty() { first_para(&n.body) } else { n.summary.trim().to_string() };
    let sources = n
        .sources
        .iter()
        .map(|s| {
            let sha = (s.kind == "pdf" && s.reference.len() == 64 && s.reference.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.reference.to_lowercase());
            let page = sha.as_ref().and_then(|_| s.location.chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).collect::<String>().parse().ok());
            let title = s.extra.get("title").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| if sha.is_some() { String::new() } else { s.reference.clone() });
            CardSource { kind: s.kind.clone(), title, location: s.location.clone(), sha, page }
        })
        .collect();
    let symbols = n.symbols.iter().filter(|s| !s.latex.trim().is_empty()).map(|s| CardSymbol { latex: s.latex.trim().to_string(), meaning: s.meaning.trim().to_string(), equation: is_equation(&s.latex) }).collect();
    Card { id: n.id, space: space.to_string(), title: n.title, aliases: n.aliases, kind: n.kind, summary, symbols, sources, matched: String::new() }
}

fn is_equation(latex: &str) -> bool {
    latex.contains('=')
}

fn first_para(body: &str) -> String {
    body.split("\n\n").map(str::trim).find(|p| !p.is_empty() && !p.starts_with('#')).unwrap_or("").chars().take(300).collect()
}

/// Every card of the given spaces (read-only; unreadable files are skipped).
pub fn load_cards(spaces: &[PathBuf]) -> Vec<Card> {
    let mut out = vec![];
    for root in spaces {
        let Ok(space) = lore_store::Space::open(root) else { continue };
        let Ok(rd) = std::fs::read_dir(space.nodes_dir()) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
        files.sort();
        for f in files {
            if let Some(n) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<Node>(&b).ok()).filter(|n| !n.id.is_empty() && !n.title.trim().is_empty()) {
                out.push(node_card(space.id(), n));
            }
        }
    }
    out
}

// ── matching ──

/// The paragraph as words to match against: citations, labels, math and commands go.
fn plain(src: &str) -> String {
    let mut s = String::with_capacity(src.len());
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut math = false;
    while i < b.len() {
        match b[i] {
            '$' => { math = !math; s.push(' '); i += 1; }
            '\\' if i + 1 < b.len() => {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_alphabetic() { j += 1 }
                let name: String = b[i + 1..j].iter().collect();
                i = if j == i + 1 { i + 2 } else { j };
                if matches!(name.as_str(), "cite" | "citep" | "citet" | "ref" | "eqref" | "label" | "autoref" | "cref") {
                    // drop the optional and braced arguments
                    while i < b.len() && (b[i] == '[' || b[i] == '{') {
                        let close = if b[i] == '[' { ']' } else { '}' };
                        while i < b.len() && b[i] != close { i += 1 }
                        i += 1;
                    }
                }
                s.push(' ');
            }
            c if math => { let _ = c; i += 1; }
            '{' | '}' | '~' => { s.push(' '); i += 1; }
            c => { s.push(c); i += 1; }
        }
    }
    s.to_lowercase()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() && c.is_ascii()
}

fn cjk(c: char) -> bool {
    !c.is_ascii() && c.is_alphanumeric()
}

/// Does `name` (lowercase) occur in `text`: Latin names only as whole words (a plural s/es is fine), CJK anywhere.
fn occurs(text: &str, name: &str) -> bool {
    if name.chars().count() < 2 {
        return false;
    }
    let latin = name.chars().any(is_word) && !name.chars().any(cjk);
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let (a, z) = (from + at, from + at + name.len());
        if !latin {
            return true;
        }
        let before = text[..a].chars().next_back();
        let rest = &text[z..];
        let after = rest.strip_prefix("es").or_else(|| rest.strip_prefix('s')).filter(|r| !r.starts_with(is_word)).unwrap_or(rest);
        if !before.is_some_and(is_word) && !after.chars().next().is_some_and(is_word) {
            return true;
        }
        from = z;
    }
    false
}

/// Cards whose title or alias occurs in `paragraph`, longest match first (then by title).
pub fn relevant_in(cards: &[Card], paragraph: &str, limit: usize) -> Vec<Card> {
    let text = plain(paragraph);
    let mut hits: Vec<(usize, Card)> = vec![];
    for c in cards {
        let best = std::iter::once(c.title.as_str()).chain(c.aliases.iter().map(String::as_str)).map(str::trim).filter(|n| occurs(&text, &n.to_lowercase())).max_by_key(|n| n.chars().count());
        if let Some(n) = best {
            let mut c = c.clone();
            c.matched = n.to_string();
            hits.push((n.chars().count(), c));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));
    hits.into_iter().map(|(_, c)| c).take(limit).collect()
}

/// Cards matching every word of `q` in title, aliases, summary or kind; title hits first.
pub fn search_in(cards: &[Card], q: &str, limit: usize) -> Vec<Card> {
    let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return vec![];
    }
    let mut hits: Vec<(u8, Card)> = vec![];
    for c in cards {
        let names = std::iter::once(c.title.as_str()).chain(c.aliases.iter().map(String::as_str)).collect::<Vec<_>>().join(" ").to_lowercase();
        let all = format!("{names} {} {}", c.summary.to_lowercase(), c.kind.to_lowercase());
        if words.iter().all(|w| all.contains(w)) {
            let rank = if words.iter().all(|w| names.contains(w)) { 0 } else { 1 };
            hits.push((rank, c.clone()));
        }
    }
    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
    hits.into_iter().map(|(_, c)| c).take(limit).collect()
}

/// The private-retrieval entry: the author's cards that the paragraph is about.
pub fn relevant_cards(paragraph: &str) -> Vec<Card> {
    relevant_in(&load_cards(&space_dirs()), paragraph, 8)
}

// ── routes ──

#[derive(Deserialize)]
pub(crate) struct CardsBody {
    #[serde(default)]
    paragraph: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

pub(crate) async fn cards(Json(b): Json<CardsBody>) -> impl IntoResponse {
    let r = tokio::task::spawn_blocking(move || {
        let spaces = space_dirs();
        let all = load_cards(&spaces);
        let limit = b.limit.unwrap_or(30).clamp(1, 100);
        let found = if b.q.trim().is_empty() { relevant_in(&all, &b.paragraph, limit) } else { search_in(&all, &b.q, limit) };
        json!({ "available": !spaces.is_empty(), "total": all.len(), "cards": found })
    })
    .await
    .unwrap_or_else(|_| json!({ "available": false, "cards": [] }));
    Json(r)
}

fn context_on(ws: &Path) -> bool {
    std::fs::read_to_string(ws.join("config.toml")).ok().and_then(|t| t.parse::<toml::Table>().ok()).and_then(|t| t.get("kb")?.get("context")?.as_bool()).unwrap_or(false)
}

fn set_context(ws: &Path, on: bool) -> Result<(), String> {
    let path = ws.join("config.toml");
    let mut doc = std::fs::read_to_string(&path).ok().and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default();
    let mut kb = doc.get("kb").and_then(|v| v.as_table().cloned()).unwrap_or_default();
    kb.insert("context".into(), toml::Value::Boolean(on));
    doc.insert("kb".into(), toml::Value::Table(kb));
    let text = format!("# PaperComposer remembers these between sessions. Safe to edit or delete.\n\n{}", toml::to_string(&doc).map_err(|e| e.to_string())?);
    pc_core::paper::write_atomic(&path, &text).map_err(|e| e.to_string())
}

pub(crate) async fn settings_get(State(s): State<Shared>) -> impl IntoResponse {
    let ws = s.loaded.read().await.workspace.clone();
    Json(json!({ "context": ws.is_some_and(|w| context_on(&w)) }))
}

#[derive(Deserialize)]
pub(crate) struct SettingsBody {
    context: bool,
}

pub(crate) async fn settings_put(State(s): State<Shared>, Json(b): Json<SettingsBody>) -> axum::response::Response {
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return bad("no workspace") };
    match set_context(&ws, b.context) {
        Ok(()) => Json(json!({ "context": b.context })).into_response(),
        Err(e) => bad(&e),
    }
}

/// The generation prompt with the author's cards for `text` added as context, when the setting is on.
pub(crate) async fn with_cards(s: &Shared, prompt: Prompt, text: &str) -> Prompt {
    let on = s.loaded.read().await.workspace.as_deref().is_some_and(context_on);
    if !on {
        return prompt;
    }
    let text = text.to_string();
    let found = tokio::task::spawn_blocking(move || relevant_cards(&text)).await.unwrap_or_default();
    add_context(prompt, &found)
}

pub fn add_context(mut prompt: Prompt, cards: &[Card]) -> Prompt {
    if cards.is_empty() {
        return prompt;
    }
    prompt.system.push_str("\nThe author's own knowledge cards for terms in the points follow the points. Use them only for what a term means and how he names it; they are not a source of claims, numbers or findings, so add nothing from them that the points do not say.");
    prompt.user.push_str("\nKnowledge cards (the author's own definitions):\n");
    for c in cards.iter().take(5) {
        let sum: String = c.summary.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(300).collect();
        let alias = if c.aliases.is_empty() { String::new() } else { format!(" (also: {})", c.aliases.join(", ")) };
        prompt.user.push_str(&format!("- {}{alias}: {sum}\n", c.title));
    }
    prompt
}

fn find_card(id: &str) -> Option<Card> {
    load_cards(&space_dirs()).into_iter().find(|c| c.id == id)
}

#[derive(Deserialize)]
pub(crate) struct ParaBody {
    id: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    engine: Option<EngineChoice>,
}

fn parse_sentence(reply: &str) -> Option<String> {
    let (a, b) = (reply.find('{')?, reply.rfind('}')?);
    let v: Value = serde_json::from_str(&reply[a..=b]).ok()?;
    let s = v.get("sentence")?.as_str()?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

/// The card's definition for the model: what to restate and which term it is for.
pub fn definition_prompt(c: &Card, language: &str) -> Prompt {
    crate::reader::paraphrase_prompt(&c.summary, &format!("This defines the term \"{}\"; the sentence should state what it is.", c.title), language)
}

pub(crate) async fn paraphrase(State(s): State<Shared>, Json(b): Json<ParaBody>) -> impl IntoResponse {
    let Some(card) = tokio::task::spawn_blocking(move || find_card(&b.id)).await.ok().flatten() else { return bad("找不到這張知識卡") };
    if card.summary.trim().is_empty() {
        return bad("這張卡沒有摘要");
    }
    let schema = json!({ "type": "object", "properties": { "sentence": { "type": "string" } }, "required": ["sentence"] });
    let paper = card.sources.iter().find(|s| s.sha.is_some()).map(|s| json!({ "sha": s.sha, "page": s.page }));
    match writing_complete(&s, "pc-kb-paraphrase", b.engine, &definition_prompt(&card, &b.language), None, Some(schema)).await {
        Ok(reply) => match parse_sentence(&reply) {
            Some(t) => Json(json!({ "text": t, "paper": paper })).into_response(),
            None => Json(json!({ "error": "模型的回覆無法讀取" })).into_response(),
        },
        Err(e) => Json(json!({ "error": e })).into_response(),
    }
}

/// A one-row table in LoreReader's `symbols.json` shape for `register_into`: the card's symbol `index`.
pub fn symbol_table(c: &Card, index: usize) -> Option<(String, Value)> {
    let s = c.symbols.get(index)?;
    let tail: String = c.id.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<String>().to_lowercase();
    let tid = format!("kb-{tail}-{}", index + 1);
    let mut row = json!({ "id": tid, "kind": if s.equation { "equation" } else { "symbol" }, "latex": s.latex, "kb": c.id });
    if !s.meaning.is_empty() {
        row["meaning"] = json!(s.meaning);
    }
    let key = if s.equation { "equations" } else { "entries" };
    Some((tid, json!({ key: [row] })))
}

#[derive(Deserialize)]
pub(crate) struct RegisterBody {
    id: String,
    index: usize,
}

/// POST /api/papers/{name}/kb/notation `{id, index}`: into the paper's own notation.json.
pub(crate) async fn register(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<RegisterBody>) -> axum::response::Response {
    let Some(card) = find_card(&b.id) else { return bad("找不到這張知識卡") };
    let Some((tid, table)) = symbol_table(&card, b.index) else { return bad("這張卡沒有這個符號") };
    let paper = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return bad("no workspace") };
    let glob = crate::notation::load_global(&ws);
    let file = pc_core::paper::app_dir(&paper).join("notation.json");
    let mut doc = lore_notation::Notation::load(&file).unwrap_or_default();
    let mut out = crate::reader::register_into(&mut doc, &glob, &table, &[tid.clone()]);
    if out["added"].as_array().is_some_and(|a| !a.is_empty()) && let Err(e) = doc.save(&file) {
        return bad(&e);
    }
    out["tid"] = json!(tid);
    Json(out).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// A synthetic KnowledgeBase space in a temp dir.
    pub(crate) fn space(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("pc-kb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nodes")).unwrap();
        std::fs::write(root.join("space.toml"), "id = \"me\"\nname = \"Me\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
        let node = |id: &str, v: Value| {
            let mut n = json!({ "id": id, "kind": "concept", "created_at": "2026-11-01T09:00:00Z", "updated_at": "2026-11-01T09:00:00Z" });
            n.as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
            std::fs::write(root.join("nodes").join(format!("{id}.json")), serde_json::to_vec_pretty(&n).unwrap()).unwrap();
        };
        node("nAAAAAAAAAAAAAAAAAAAAAAAA1", json!({ "title": "Gated flow", "aliases": ["gating"], "summary": "A flow whose edges are switched by a learned gate.",
            "symbols": [{ "latex": "g_{t}", "meaning": "gate value" }, { "latex": "y = g_{t} x", "meaning": "gated output" }],
            "sources": [{ "type": "pdf", "ref": SHA, "where": "p.4", "title": "Gated Flows for Synthetic Signals" }, { "type": "web", "ref": "https://example.org/gates" }] }));
        node("nAAAAAAAAAAAAAAAAAAAAAAAA2", json!({ "title": "梯度", "aliases": ["gradient"], "summary": "函數增加最快的方向。" }));
        node("nAAAAAAAAAAAAAAAAAAAAAAAA3", json!({ "title": "AI", "summary": "too short a name to match" }));
        node("nAAAAAAAAAAAAAAAAAAAAAAAA4", json!({ "title": "Flow", "summary": "Something that moves." }));
        std::fs::write(root.join("nodes/.hidden.json"), "{}").unwrap();
        std::fs::write(root.join("nodes/broken.json"), "not json").unwrap();
        root
    }

    #[test]
    fn finds_cards_for_the_terms_in_a_paragraph() {
        let root = space("match");
        let before: Vec<_> = std::fs::read_dir(root.join("nodes")).unwrap().flatten().map(|e| e.file_name()).collect();
        let all = load_cards(&[root.clone()]);
        assert_eq!(all.len(), 4, "hidden and broken files are skipped");
        let hit = relevant_in(&all, "Each \\textbf{gated flows} layer~\\cite{k} uses $g_{t}$ and the gradient of the loss; AI is hot.", 8);
        let titles: Vec<_> = hit.iter().map(|c| (c.title.as_str(), c.matched.as_str())).collect();
        assert_eq!(titles, [("Gated flow", "Gated flow"), ("梯度", "gradient"), ("Flow", "Flow"), ("AI", "AI")], "plural, alias, CJK card by alias, an acronym as a whole word");
        assert!(relevant_in(&all, "the said thing", 8).is_empty(), "AI inside a word does not match");
        assert!(relevant_in(&all, "Reflowing the data, \\ref{flow-fig}", 8).is_empty(), "inside a word, inside \\ref");
        assert_eq!(relevant_in(&all, "梯度下降", 8)[0].title, "梯度", "CJK matches inside text");
        assert_eq!(relevant_in(&all, "gating and flow", 1).len(), 1);
        let after: Vec<_> = std::fs::read_dir(root.join("nodes")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(before.len(), after.len(), "nothing was written into the space");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn card_carries_sources_with_reader_pages_and_symbols() {
        let root = space("card");
        let all = load_cards(&[root.clone()]);
        let c = all.iter().find(|c| c.title == "Gated flow").unwrap();
        assert_eq!(c.sources[0].sha.as_deref(), Some(SHA));
        assert_eq!((c.sources[0].page, c.sources[0].title.as_str()), (Some(4), "Gated Flows for Synthetic Signals"));
        assert_eq!((c.sources[1].sha.clone(), c.sources[1].title.as_str()), (None, "https://example.org/gates"));
        assert_eq!(c.symbols.iter().map(|s| s.equation).collect::<Vec<_>>(), [false, true]);
        let found = search_in(&all, "gate learned", 5);
        assert_eq!(found.len(), 1);
        assert_eq!(search_in(&all, "flow", 5)[0].title, "Flow", "a title hit comes first");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn spaces_come_from_the_environment_or_are_empty() {
        let root = space("env");
        // SAFETY: tests in this module that read KB_SPACES are this one only.
        unsafe { std::env::set_var("KB_SPACES", format!("{}:/nonexistent", root.display())) };
        assert_eq!(space_dirs(), [root.clone()]);
        unsafe { std::env::remove_var("KB_SPACES") };
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn context_goes_into_the_prompt_only_as_terms() {
        let root = space("ctx");
        let all = load_cards(&[root.clone()]);
        let p = Prompt { system: "S".into(), user: "U".into() };
        assert_eq!(add_context(p.clone(), &[]).user, "U");
        let hit = relevant_in(&all, "gated flow", 8);
        let q = add_context(p, &hit);
        assert!(q.user.contains("- Gated flow (also: gating): A flow whose edges"));
        assert!(q.system.contains("not a source of claims"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn setting_is_off_by_default_and_keeps_the_rest_of_config() {
        let root = space("set");
        assert!(!context_on(&root));
        std::fs::write(root.join("config.toml"), "[ui]\ndim = true\n").unwrap();
        set_context(&root, true).unwrap();
        assert!(context_on(&root));
        assert!(std::fs::read_to_string(root.join("config.toml")).unwrap().contains("dim = true"));
        set_context(&root, false).unwrap();
        assert!(!context_on(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn definition_prompt_and_reply() {
        let root = space("def");
        let c = load_cards(&[root.clone()]).into_iter().find(|c| c.title == "Gated flow").unwrap();
        let p = definition_prompt(&c, "en");
        assert!(p.user.contains("switched by a learned gate") && p.user.contains("\"Gated flow\""));
        assert!(p.system.contains("no citation command"), "the citation is added by the app, not the model");
        assert_eq!(parse_sentence("{\"sentence\": \"A gate switches edges.\"}").as_deref(), Some("A gate switches edges."));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn symbols_register_into_the_paper_with_the_card_id_and_pass_the_notation_check() {
        use lore_notation::{Entry, Kind, Notation};
        let root = space("reg");
        let c = load_cards(&[root.clone()]).into_iter().find(|c| c.title == "Gated flow").unwrap();
        let glob = Notation { entries: vec![Entry { id: "gate".into(), kind: Some(Kind::Symbol), latex: Some("g_{t}".into()), macro_name: Some("gt".into()), ..Default::default() }], ..Default::default() };
        let mut doc = Notation::default();
        // a symbol the global registry has keeps its row (and macro)
        let (t0, table0) = symbol_table(&c, 0).unwrap();
        let out = crate::reader::register_into(&mut doc, &glob, &table0, &[t0.clone()]);
        assert_eq!(out["use"][&t0]["macro"], "gt", "{out}");
        assert!(doc.entries.is_empty());
        // an equation is added with the card id and a label
        let (t1, table1) = symbol_table(&c, 1).unwrap();
        let out = crate::reader::register_into(&mut doc, &glob, &table1, &[t1.clone()]);
        assert_eq!(out["added"], json!([t1.clone()]), "{out}");
        let row = doc.get(&t1).unwrap();
        assert_eq!((row.kb.as_deref(), row.kind), (Some("nAAAAAAAAAAAAAAAAAAAAAAAA1"), Some(Kind::Equation)));
        assert_eq!(out["use"][&t1]["label"], format!("eq:{t1}"));
        assert!(symbol_table(&c, 2).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
