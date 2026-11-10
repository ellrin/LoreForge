//! 讀過的論文: LoreReader's library while writing.
//!
//! LoreReader keeps one folder per paper in its workspace
//! (`private/LoreReader/papers/<sha256>/`, see `LoreReader/crates/core/src/papers.rs`):
//! `paper.json` (title, DOI, pages), `annotations.json` (highlights and notes with their
//! quoted text and page), `quotes.json` (citable sentences) and `symbols.json` (the
//! paper's symbol table: lore-notation entries plus scope, grade and evidence).
//! This module reads those files **read-only**, straight from disk, so it works when
//! LoreReader is not running, and never writes into LoreReader's folder. The folder is
//! `LOREREADER_WORKSPACE` (LoreReader's own override), else lore-home's
//! `private/LoreReader`; it is never created or seeded from here.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/reader/papers?paper= | papers with counts and the library key each one already has (`?paper=` also looks in that paper's own .bib) |
//! | GET | /api/reader/papers/{sha} | its quotes, highlights and notes (by page) and its symbols |
//! | POST | /api/reader/papers/{sha}/cite | `{paper?, doi?, minimal?}` → `{key, added}`: the library entry (Crossref BibTeX from the DOI; deduped by DOI, title and key), or `{need_doi}` / `{error, can_minimal}` |
//! | POST | /api/reader/open | `{sha, page}` → LoreReader's `/open?sha=&page=` link |
//! | POST | /api/reader/paraphrase | `{text, note?, language?, engine?}` → `{text}` (task `pc-compose-paragraph`) |

use super::{bad, bib_files, load_library, paper_dir, refs_root, save_library, writing_complete, Shared, BIB_MARK, LIBRARY_LOCK};
use crate::llm::EngineChoice;
use crate::{proc, refimport};
use axum::{
    extract::{Path as AxPath, Query as AxQuery, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use lore_notation::Entry as NotationEntry;
use pc_core::bib;
use pc_core::compose::Prompt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

const APP: &str = "LoreReader";
const READER_URL: &str = "http://127.0.0.1:7890";

/// LoreReader's workspace, when it exists. Read-only: never created here.
pub fn reader_dir() -> Option<PathBuf> {
    std::env::var_os("LOREREADER_WORKSPACE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| super::loreforge_home().app_dir(APP))
        .filter(|d| d.join("papers").is_dir())
}

fn reader_url() -> String {
    std::env::var("LOREREADER_URL").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| READER_URL.into()).trim_end_matches('/').to_string()
}

fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.iter().map(text_of).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" and "),
        _ => String::new(),
    }
}

pub fn norm_doi(d: &str) -> String {
    let d = d.trim();
    let d = d.strip_prefix("https://doi.org/").or_else(|| d.strip_prefix("http://doi.org/")).or_else(|| d.strip_prefix("doi:")).unwrap_or(d);
    d.trim().to_lowercase()
}

fn norm_title(t: &str) -> String {
    bib::clean(t).chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// One paper of LoreReader's library, as the cite popup lists it.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ReaderPaper {
    pub sha: String,
    pub title: String,
    /// BibTeX-style "A and B" when LoreReader has them (paper.json `authors`), else "".
    pub authors: String,
    pub year: String,
    pub venue: String,
    pub doi: String,
    pub pages: u64,
    pub added_at: String,
    /// Highlights and notes (annotations.json).
    pub marks: usize,
    /// Citable sentences (quotes.json).
    pub quotes: usize,
    pub symbols: usize,
    /// The library (or this paper's .bib) key it already has.
    pub key: Option<String>,
    /// The title is only the file name (paper.json `title_source: "file"`): not
    /// good enough to match a library entry by.
    #[serde(skip)]
    pub title_from_file: bool,
}

fn count(v: Option<Value>, field: &str) -> usize {
    v.and_then(|v| v.get(field).and_then(Value::as_array).map(Vec::len)).unwrap_or(0)
}

fn read_paper(dir: &Path, sha: &str) -> Option<(ReaderPaper, Value)> {
    if !valid_sha(sha) {
        return None;
    }
    let folder = dir.join("papers").join(sha);
    let m = read_json(&folder.join("paper.json"))?;
    let g = |k: &str| m.get(k).map(text_of).unwrap_or_default();
    let p = ReaderPaper {
        sha: sha.to_string(),
        title: g("title"),
        authors: g("authors"),
        year: g("year"),
        venue: g("venue"),
        doi: m.get("doi").and_then(Value::as_str).map(norm_doi).unwrap_or_default(),
        pages: m.get("pages").and_then(Value::as_u64).unwrap_or(0),
        added_at: g("added_at"),
        marks: count(read_json(&folder.join("annotations.json")), "annotations"),
        quotes: count(read_json(&folder.join("quotes.json")), "quotes"),
        symbols: count(read_json(&folder.join("symbols.json")), "entries"),
        key: None,
        title_from_file: m.get("title_source").and_then(Value::as_str) == Some("file"),
    };
    Some((p, m))
}

/// Every paper with a paper.json, newest first. A LoreReader export of a paper that is
/// itself in the library (`source_sha`) is left out: its notes are the original's.
pub fn list_papers(dir: &Path) -> Vec<ReaderPaper> {
    let Ok(rd) = std::fs::read_dir(dir.join("papers")) else { return vec![] };
    let all: Vec<(ReaderPaper, Value)> = rd.flatten().filter_map(|e| read_paper(dir, &e.file_name().to_string_lossy())).collect();
    let shas: Vec<String> = all.iter().map(|(p, _)| p.sha.clone()).collect();
    let mut out: Vec<ReaderPaper> = all
        .into_iter()
        .filter(|(_, m)| !m.get("source_sha").and_then(Value::as_str).is_some_and(|s| shas.iter().any(|x| x == s)))
        .map(|(p, _)| p)
        .collect();
    out.sort_by(|a, b| b.added_at.cmp(&a.added_at));
    out
}

/// A quote, highlight or note, as side material.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Material {
    pub id: String,
    /// quote | highlight | note
    pub kind: String,
    pub text: String,
    pub note: String,
    pub page: u64,
    pub color: String,
    #[serde(skip)]
    at: String,
}

/// Quotes first on each page, then highlights and notes, in reading order.
pub fn material(dir: &Path, sha: &str) -> Vec<Material> {
    if !valid_sha(sha) {
        return vec![];
    }
    let folder = dir.join("papers").join(sha);
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let mut out: Vec<Material> = vec![];
    if let Some(q) = read_json(&folder.join("quotes.json")) {
        for v in q.get("quotes").and_then(Value::as_array).into_iter().flatten() {
            let text = s(v, "text");
            if text.is_empty() {
                continue;
            }
            out.push(Material { id: s(v, "id"), kind: "quote".into(), text, note: s(v, "note"), page: v.get("page").and_then(Value::as_u64).unwrap_or(0), color: String::new(), at: s(v, "created_at") });
        }
    }
    if let Some(a) = read_json(&folder.join("annotations.json")) {
        for v in a.get("annotations").and_then(Value::as_array).into_iter().flatten() {
            let text = v.get("quote").map(|q| s(q, "exact")).unwrap_or_default();
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let note = s(v, "note");
            if text.is_empty() && note.is_empty() {
                continue;
            }
            // the same sentence kept as a citable quote is listed once
            if !text.is_empty() && out.iter().any(|m| m.kind == "quote" && m.text == text) {
                continue;
            }
            let kind = if text.is_empty() || s(v, "kind") == "note" { "note" } else { "highlight" };
            out.push(Material { id: s(v, "id"), kind: kind.into(), text, note, page: v.get("page").and_then(Value::as_u64).unwrap_or(0), color: s(v, "color"), at: s(v, "created_at") });
        }
    }
    let rank = |k: &str| match k { "quote" => 0, "highlight" => 1, _ => 2 };
    out.sort_by(|a, b| (a.page, rank(&a.kind), &a.at).cmp(&(b.page, rank(&b.kind), &b.at)));
    out
}

/// The paper's symbol table as lore-notation entries plus what LoreReader knows of each.
pub fn symbols(dir: &Path, sha: &str) -> Vec<Value> {
    if !valid_sha(sha) {
        return vec![];
    }
    let Some(f) = read_json(&dir.join("papers").join(sha).join("symbols.json")) else { return vec![] };
    f.get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let e: NotationEntry = serde_json::from_value(v.clone()).ok()?;
            e.latex.as_ref()?;
            let ev = v.get("evidence").and_then(Value::as_array).and_then(|a| a.first());
            Some(serde_json::json!({
                "id": e.id, "kind": e.kind, "latex": e.latex, "meaning": e.meaning, "meaning_en": e.meaning_en,
                "grade": v.get("grade").and_then(Value::as_str).unwrap_or(""),
                "quote": ev.and_then(|q| q.get("quote")).and_then(Value::as_str).unwrap_or(""),
                "page": ev.and_then(|q| q.get("page")).and_then(Value::as_u64)
                    .or_else(|| v.get("first").and_then(|f| f.get("page")).and_then(Value::as_u64)),
            }))
        })
        .collect()
}

/// The key `entries` already have for this paper: same DOI, else the same title.
pub fn find_key(p: &ReaderPaper, doi: &str, entries: &[bib::Entry]) -> Option<String> {
    let doi = norm_doi(doi);
    if !doi.is_empty()
        && let Some(e) = entries.iter().find(|e| e.get("doi").is_some_and(|d| norm_doi(d) == doi))
    {
        return Some(e.key.clone());
    }
    if p.title_from_file {
        return None;
    }
    let t = norm_title(&p.title);
    (t.len() >= 12).then(|| entries.iter().find(|e| norm_title(&e.title()) == t).map(|e| e.key.clone())).flatten()
}

/// Entries of the paper's own .bib files (not the one PaperComposer generates).
fn paper_entries(dir: &Path) -> Vec<bib::Entry> {
    let main = std::fs::read_to_string(dir.join("main.tex")).unwrap_or_default();
    bib_files(&main)
        .iter()
        .filter_map(|f| std::fs::read_to_string(dir.join(f)).ok())
        .filter(|t| !t.starts_with(BIB_MARK))
        .flat_map(|t| bib::parse(&t).0)
        .collect()
}

fn bib_value(s: &str) -> String {
    s.replace(['{', '}'], "")
}

/// A BibTeX entry from what LoreReader knows (the author's choice when Crossref has
/// nothing or cannot be reached).
pub fn minimal_entry(p: &ReaderPaper, doi: &str) -> bib::Entry {
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("title".to_string(), format!("{{{}}}", bib_value(&p.title)));
    for (k, v) in [("author", &p.authors), ("year", &p.year), ("journal", &p.venue)] {
        if !v.trim().is_empty() {
            fields.insert(k.to_string(), bib_value(v));
        }
    }
    if !doi.is_empty() {
        fields.insert("doi".into(), doi.to_string());
    }
    let kind = if p.venue.trim().is_empty() { "misc" } else { "article" };
    bib::Entry { kind: kind.into(), key: String::new(), fields }
}

/// Put `e` in the library unless it is already there (same DOI or title): the key
/// it is cited by, and whether it was added. `also_taken`: keys the paper's own .bib uses.
pub fn file_entry(ws: &Path, p: &ReaderPaper, mut e: bib::Entry, doi: &str, also_taken: &[String]) -> Result<(String, bool), String> {
    let (mut entries, mut notes) = load_library(ws);
    if let Some(k) = find_key(p, doi, &entries) {
        return Ok((k, false));
    }
    if !doi.is_empty() {
        e.fields.entry("doi".into()).or_insert_with(|| doi.to_string());
    }
    let taken: Vec<String> = entries.iter().map(|x| x.key.clone()).chain(also_taken.iter().cloned()).collect();
    e.key = refimport::make_key(&e, &taken);
    notes.push(bib::Note { key: e.key.clone(), doi: doi.to_string(), ..Default::default() });
    let key = e.key.clone();
    entries.push(e);
    save_library(ws, &entries, &notes)?;
    Ok((key, true))
}

/// Which library key each LoreReader paper was cited by (`refs/reader-links.json` in
/// PaperComposer's workspace, never in LoreReader's): a DOI typed here, or a title
/// LoreReader only knows as a file name, still finds the entry the next time.
const LINKS: &str = "refs/reader-links.json";

pub fn read_links(ws: &Path) -> std::collections::BTreeMap<String, String> {
    std::fs::read(ws.join(LINKS)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_link(ws: &Path, sha: &str, key: &str) -> Result<(), String> {
    let mut l = read_links(ws);
    if l.get(sha).map(String::as_str) == Some(key) {
        return Ok(());
    }
    l.insert(sha.to_string(), key.to_string());
    std::fs::create_dir_all(ws.join("refs")).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&l).map_err(|e| e.to_string())? + "\n";
    pc_core::paper::write_atomic(&ws.join(LINKS), &text).map_err(|e| e.to_string())
}

/// The key this paper already has: the paper's own .bib first, then the library;
/// a remembered link counts while its entry still exists.
pub fn known_key(p: &ReaderPaper, doi: &str, own: &[bib::Entry], lib: &[bib::Entry], links: &std::collections::BTreeMap<String, String>) -> Option<String> {
    let linked = links.get(&p.sha).filter(|k| own.iter().chain(lib).any(|e| &e.key == *k)).cloned();
    find_key(p, doi, own).or(linked).or_else(|| find_key(p, doi, lib))
}

// ── handlers ───────────────────────────────────────────

fn not_here() -> axum::response::Response {
    (StatusCode::NOT_FOUND, "找不到 LoreReader 的書庫").into_response()
}

#[derive(Deserialize)]
pub(crate) struct ListQ {
    #[serde(default)]
    paper: Option<String>,
}

pub(crate) async fn papers(State(s): State<Shared>, AxQuery(q): AxQuery<ListQ>) -> impl IntoResponse {
    let Some(dir) = reader_dir() else { return Json(serde_json::json!({ "available": false, "papers": [] })).into_response() };
    let mut list = list_papers(&dir);
    let own = match q.paper.filter(|n| !n.is_empty()) {
        Some(n) => paper_dir(&s, &n).await.map(|d| paper_entries(&d)).unwrap_or_default(),
        None => vec![],
    };
    let ws = refs_root(&s).await.ok();
    let lib = ws.as_deref().map(|w| load_library(w).0).unwrap_or_default();
    let links = ws.as_deref().map(read_links).unwrap_or_default();
    for p in &mut list {
        let doi = p.doi.clone();
        p.key = known_key(p, &doi, &own, &lib, &links);
        // authors and year from the library when LoreReader has none
        if let Some(e) = p.key.as_ref().and_then(|k| own.iter().chain(lib.iter()).find(|e| &e.key == k)) {
            if p.authors.is_empty() {
                p.authors = e.get("author").unwrap_or("").to_string();
            }
            if p.year.is_empty() {
                p.year = e.year();
            }
            if p.venue.is_empty() {
                p.venue = e.venue();
            }
        }
    }
    Json(serde_json::json!({ "available": true, "papers": list })).into_response()
}

pub(crate) async fn paper(AxPath(sha): AxPath<String>) -> impl IntoResponse {
    let Some(dir) = reader_dir() else { return not_here() };
    let Some((p, _)) = read_paper(&dir, &sha) else { return (StatusCode::NOT_FOUND, "LoreReader 沒有這篇").into_response() };
    Json(serde_json::json!({ "paper": p, "items": material(&dir, &sha), "symbols": symbols(&dir, &sha) })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct CiteBody {
    #[serde(default)]
    paper: Option<String>,
    #[serde(default)]
    doi: Option<String>,
    /// File it from LoreReader's own data (no Crossref).
    #[serde(default)]
    minimal: bool,
}

pub(crate) async fn cite(State(s): State<Shared>, AxPath(sha): AxPath<String>, Json(b): Json<CiteBody>) -> impl IntoResponse {
    let Some(dir) = reader_dir() else { return not_here() };
    let Some((p, _)) = read_paper(&dir, &sha) else { return (StatusCode::NOT_FOUND, "LoreReader 沒有這篇").into_response() };
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let doi = b.doi.as_deref().map(norm_doi).filter(|d| !d.is_empty()).unwrap_or_else(|| p.doi.clone());
    let own = match b.paper.filter(|n| !n.is_empty()) {
        Some(n) => match paper_dir(&s, &n).await { Ok(d) => paper_entries(&d), Err(r) => return r },
        None => vec![],
    };
    let lib = load_library(&ws).0;
    if let Some(k) = known_key(&p, &doi, &own, &lib, &read_links(&ws)) {
        let _ = save_link(&ws, &sha, &k);
        return Json(serde_json::json!({ "key": k, "added": false })).into_response();
    }
    let entry = if b.minimal {
        if p.title.trim().is_empty() {
            return bad("這篇沒有標題");
        }
        minimal_entry(&p, &doi)
    } else if doi.is_empty() {
        return Json(serde_json::json!({ "need_doi": true, "title": p.title })).into_response();
    } else {
        let fetched = refimport::crossref_bibtex(&doi).await.and_then(|t| bib::parse(&t).0.into_iter().next().ok_or_else(|| "doi.org 回傳的 BibTeX 無法讀取".to_string()));
        match fetched {
            Ok(e) => e,
            Err(e) => return Json(serde_json::json!({ "error": e, "can_minimal": !p.title.trim().is_empty(), "doi": doi })).into_response(),
        }
    };
    let taken: Vec<String> = own.iter().map(|e| e.key.clone()).collect();
    let _library = LIBRARY_LOCK.lock().await;
    let title = entry.title();
    match file_entry(&ws, &p, entry, &doi, &taken) {
        Ok((key, added)) => {
            if let Err(e) = save_link(&ws, &sha, &key) {
                eprintln!("reader-links: {e}");
            }
            Json(serde_json::json!({ "key": key, "added": added, "title": title })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct OpenBody {
    sha: String,
    #[serde(default)]
    page: Option<u64>,
}

fn listening(url: &str) -> bool {
    let host = url.trim_start_matches("http://").split('/').next().unwrap_or("");
    use std::net::ToSocketAddrs;
    host.to_socket_addrs().ok().and_then(|mut a| a.next()).is_some_and(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(300)).is_ok())
}

/// Open LoreReader at that page: its `/open?sha=&page=` link (the one KnowledgeBase
/// cards use). On macOS the app is started first when nothing answers.
pub(crate) async fn open(Json(b): Json<OpenBody>) -> impl IntoResponse {
    if !valid_sha(&b.sha) {
        return bad("invalid sha");
    }
    let base = reader_url();
    let url = format!("{base}/open?sha={}&page={}", b.sha.to_lowercase(), b.page.unwrap_or(1).max(1));
    let probe = base.clone();
    let up = tokio::task::spawn_blocking(move || listening(&probe)).await.unwrap_or(false);
    if !up {
        #[cfg(target_os = "macos")]
        {
            if proc::std_cmd("open").args(["-g", "-a", APP]).status().is_ok_and(|s| s.success()) {
                for _ in 0..40 {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    let probe = base.clone();
                    if tokio::task::spawn_blocking(move || listening(&probe)).await.unwrap_or(false) {
                        break;
                    }
                }
            }
        }
    }
    match proc::open_in_browser(&url) {
        Ok(()) => Json(serde_json::json!({ "url": url })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct ParaphraseBody {
    text: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    engine: Option<EngineChoice>,
}

pub fn paraphrase_prompt(text: &str, note: &str, language: &str) -> Prompt {
    let lang = if language.starts_with("zh") { "Traditional Chinese (Taiwan usage)" } else { "English" };
    let system = format!(
        "You help an author cite a passage from a paper he read. Restate the passage in {lang} as one or two sentences \
for the author's own paper, in an academic register.\n\
Rules:\n\
1. Keep the meaning and every number exactly; add no claim of your own.\n\
2. Do not copy more than four consecutive words from the passage, apart from technical terms and names.\n\
3. Plain prose for LaTeX: no citation command, no \\ref, no typed figure or table numbers; math only as $...$.\n\
4. Reply with JSON only: {{\"sentence\": \"...\"}}."
    );
    let mut user = format!("Passage:\n{}\n", text.trim());
    if !note.trim().is_empty() {
        user.push_str(&format!("\nThe author's note on it (what he wants it for):\n{}\n", note.trim()));
    }
    Prompt { system, user }
}

fn parse_sentence(reply: &str) -> Option<String> {
    let (a, b) = (reply.find('{')?, reply.rfind('}')?);
    let v: Value = serde_json::from_str(&reply[a..=b]).ok()?;
    let s = v.get("sentence")?.as_str()?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!s.is_empty()).then_some(s)
}

pub(crate) async fn paraphrase(State(s): State<Shared>, Json(b): Json<ParaphraseBody>) -> impl IntoResponse {
    if b.text.trim().is_empty() {
        return bad("沒有文字");
    }
    let schema = serde_json::json!({ "type": "object", "properties": { "sentence": { "type": "string" } }, "required": ["sentence"] });
    match writing_complete(&s, "pc-compose-paragraph", b.engine, &paraphrase_prompt(&b.text, &b.note, &b.language), None, Some(schema)).await {
        Ok(reply) => match parse_sentence(&reply) {
            Some(t) => Json(serde_json::json!({ "text": t })).into_response(),
            None => Json(serde_json::json!({ "error": "模型的回覆無法讀取" })).into_response(),
        },
        Err(e) => Json(serde_json::json!({ "error": e })).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-reader-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A synthetic LoreReader workspace: two read papers and an export of one of them.
    fn reader_ws(root: &Path) -> PathBuf {
        let lr = root.join("LoreReader");
        let put = |sha: &str, file: &str, v: Value| {
            let d = lr.join("papers").join(sha);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(file), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        };
        put(SHA_A, "paper.json", serde_json::json!({ "schema": "lorereader.paper/1", "sha256": SHA_A, "title": "Gated Flows for Synthetic Signals",
            "original_name": "flows.pdf", "doi": "https://doi.org/10.5555/Synth.2024.001", "pages": 9, "added_at": "2026-11-01T09:00:00+08:00" }));
        put(SHA_A, "annotations.json", serde_json::json!({ "schema": "lorereader.annotations/1", "annotations": [
            { "id": "a2", "page": 4, "kind": "highlight", "color": "yellow", "quote": { "exact": "The gate halves the error  on every split." }, "note": "", "created_at": "2026-11-02T10:00:00Z" },
            { "id": "a1", "page": 2, "kind": "highlight", "color": "green", "quote": { "exact": "We model each signal as a flow." }, "note": "定義", "created_at": "2026-11-02T09:00:00Z" },
            { "id": "a3", "page": 5, "kind": "note", "color": "yellow", "quote": { "exact": "" }, "note": "比較我們的方法", "created_at": "2026-11-02T11:00:00Z" } ] }));
        put(SHA_A, "quotes.json", serde_json::json!({ "schema": "lorereader.quotes/1", "quotes": [
            { "id": "q1", "text": "The gate halves the error on every split.", "page": 4, "note": "主要結果", "created_at": "2026-11-02T10:01:00Z" } ] }));
        put(SHA_A, "symbols.json", serde_json::json!({ "schema": "lorereader.symbols/1", "sha256": SHA_A, "entries": [
            { "id": "w", "kind": "symbol", "latex": "w_{i}", "meaning_en": "gate weight", "scope": SHA_A, "grade": "defined", "resolved_by": "program",
              "evidence": [{ "quote": "where w_i denotes the gate weight", "page": 3, "by": "program" }] },
            { "id": "x", "kind": "symbol", "latex": "\\mathbf{x}", "scope": SHA_A, "grade": "unresolved", "resolved_by": "program" } ] }));
        put(SHA_B, "paper.json", serde_json::json!({ "sha256": SHA_B, "title": "A Survey of Toy Benchmarks", "doi": null, "pages": 20,
            "added_at": "2026-11-05T09:00:00+08:00", "authors": ["Ellrin Lee", "Daidai Wu"], "year": 2025, "venue": "Journal of Examples" }));
        // an export of A with the notes written in: not listed again
        put(SHA_C, "paper.json", serde_json::json!({ "sha256": SHA_C, "title": "Gated Flows (筆記)", "added_at": "2026-11-06T09:00:00+08:00", "source_sha": SHA_A }));
        lr
    }

    #[test]
    fn lists_read_papers_with_counts_newest_first() {
        let root = tmp("list");
        let lr = reader_ws(&root);
        let list = list_papers(&lr);
        assert_eq!(list.iter().map(|p| p.sha.as_str()).collect::<Vec<_>>(), [SHA_B, SHA_A]);
        let a = &list[1];
        assert_eq!((a.marks, a.quotes, a.symbols, a.pages), (3, 1, 2, 9));
        assert_eq!(a.doi, "10.5555/synth.2024.001");
        assert_eq!((list[0].authors.as_str(), list[0].year.as_str()), ("Ellrin Lee and Daidai Wu", "2025"));
        // nothing was written into LoreReader's folder
        assert_eq!(std::fs::read_dir(lr.join("papers").join(SHA_A)).unwrap().count(), 4);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn material_is_by_page_with_quotes_once() {
        let root = tmp("material");
        let lr = reader_ws(&root);
        let m = material(&lr, SHA_A);
        let got: Vec<(u64, &str)> = m.iter().map(|x| (x.page, x.kind.as_str())).collect();
        assert_eq!(got, [(2, "highlight"), (4, "quote"), (5, "note")], "the highlight kept as a quote is listed once");
        assert_eq!(m[1].note, "主要結果");
        assert!(material(&lr, "../../etc").is_empty());
        let s = symbols(&lr, SHA_A);
        assert_eq!(s[0]["latex"], "w_{i}");
        assert_eq!(s[0]["page"], 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn files_one_library_entry_and_finds_it_again() {
        let root = tmp("file");
        let lr = reader_ws(&root);
        let ws = root.join("PaperComposer");
        std::fs::create_dir_all(&ws).unwrap();
        let list = list_papers(&lr);
        let b = list.iter().find(|p| p.sha == SHA_B).unwrap();
        // the paper's own .bib already uses the key the entry would get
        let taken = vec!["lee2025survey".to_string()];
        let (k, added) = file_entry(&ws, b, minimal_entry(b, ""), "", &taken).unwrap();
        assert!(added);
        assert_eq!(k, "lee2025surveya");
        let lib = std::fs::read_to_string(ws.join("refs/library.bib")).unwrap();
        assert!(lib.contains("A Survey of Toy Benchmarks") && lib.contains("Journal of Examples") && lib.contains("@article"));
        // again: found by title, nothing added
        assert_eq!(file_entry(&ws, b, minimal_entry(b, ""), "", &[]).unwrap(), (k.clone(), false));
        // a Crossref-style entry for A, then found by DOI whatever its case
        let a = list.iter().find(|p| p.sha == SHA_A).unwrap();
        let e = bib::parse("@article{x, title={Gated Flows for Synthetic Signals}, author={Ellrin Lee}, year={2024}, DOI={10.5555/SYNTH.2024.001}}").0.remove(0);
        let (ka, added) = file_entry(&ws, a, e, &a.doi, &[]).unwrap();
        assert!(added);
        let entries = load_library(&ws).0;
        assert_eq!(find_key(a, "10.5555/Synth.2024.001", &entries), Some(ka));
        assert_eq!(entries.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_remembered_link_finds_an_entry_by_another_title() {
        let root = tmp("links");
        let lr = reader_ws(&root);
        let ws = root.join("PaperComposer");
        let a = list_papers(&lr).into_iter().find(|p| p.sha == SHA_A).unwrap();
        // filed from a DOI typed in PaperComposer: Crossref's title, not LoreReader's
        let lib = bib::parse("@article{other2024, title={Something Else Entirely Named}, doi={10.1/zz}}").0;
        assert_eq!(known_key(&a, &a.doi, &[], &lib, &read_links(&ws)), None);
        save_link(&ws, SHA_A, "other2024").unwrap();
        assert_eq!(known_key(&a, &a.doi, &[], &lib, &read_links(&ws)).as_deref(), Some("other2024"));
        // the entry was removed from the library: the link no longer counts
        assert_eq!(known_key(&a, &a.doi, &[], &[], &read_links(&ws)), None);
        // the paper's own .bib wins over the link
        let own = bib::parse("@article{mine, title={Gated Flows for Synthetic Signals}}").0;
        assert_eq!(known_key(&a, "", &own, &lib, &read_links(&ws)).as_deref(), Some("mine"));
        // a title that is only the file name never matches by title
        let named = ReaderPaper { title_from_file: true, ..a.clone() };
        assert_eq!(find_key(&named, "", &own), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn paraphrase_reply_and_prompt() {
        assert_eq!(parse_sentence("ok {\"sentence\": \" Gating  halves the error. \"}").as_deref(), Some("Gating halves the error."));
        assert_eq!(parse_sentence("{\"sentence\": \"\"}"), None);
        let p = paraphrase_prompt("The gate halves the error.", "主要結果", "en");
        assert!(p.user.contains("The gate halves the error.") && p.user.contains("主要結果"));
        assert!(p.system.contains("no citation command"));
    }
}
