//! Reading aids over the text the page sends once per opened paper
//! (`POST /api/papers/{sha}/read`, kept in memory, never written to the workspace):
//!
//! - the noun list (track A): `GET …/nouns`, `POST …/nouns/classify` (one batched
//!   `lr-classify-terms` call, cached in `papers/<sha>/nouns.json`), the ignore list
//!   (`private/LoreReader/ignore-words.txt`, formerly `known-words.txt`);
//! - the symbol table (variables and equations, step 1, no model):
//!   `GET …/symbols`, `PUT …/symbols/{id}` (author edits win), `POST …/symbols` (add),
//!   `POST …/symbols/rebuild`, kept in `papers/<sha>/symbols.json`; the glyph → LaTeX
//!   override `GET/PUT /api/glyphs` (`private/LoreReader/glyphs.toml`);
//! - paragraphs (track B): `GET …/paragraphs`, the background `lr-paragraph` run
//!   (methods and results first, cached per paragraph in `papers/<sha>/paragraphs.json`),
//!   懂了 in `papers/<sha>/reading.json`, and the context Ask Claude gets for a paragraph;
//! - the model menu: `GET/PUT /api/models` (`private/LoreReader/models.json`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use lore_llm::LlmRequest;
use lr_core::nouns::{self, Filters};
use lr_core::paragraphs::{self, Analysis, Paragraph};
use lr_core::symbols::{self, Evidence, Glyphs, SymbolEntry, SymbolFile};
use lr_core::text::PaperText;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{ApiError, R, Shared, bad, llm, sha_ok};

pub const NOUNS_SCHEMA: &str = "lorereader.nouns/1";
pub const IGNORE_FILE: &str = "ignore-words.txt";
const OLD_IGNORE_FILE: &str = "known-words.txt";
const IGNORE_HEADER: &str = "# 不要列進「名詞」、也不要標紅的字（一行一個字或詞組，不分大小寫；複數、英式拼法會自動對到）。\n# 懂的專有名詞不要放這裡：加入知識庫並標「已掌握」。右鍵「這不是術語」會寫進這裡。\n";

/// `ignore-words.txt` in the workspace. The first time, the old `known-words.txt`
/// is moved there (or appended when both exist), so it is read only once.
pub fn ignore_file(ws: &FsPath) -> PathBuf {
    let new = ws.join(IGNORE_FILE);
    let old = ws.join(OLD_IGNORE_FILE);
    if old.is_file() {
        let moved = if new.exists() {
            let add = std::fs::read_to_string(&old).unwrap_or_default();
            let cur = std::fs::read_to_string(&new).unwrap_or_default();
            let mut out = cur.trim_end().to_string();
            out += "\n# 從 known-words.txt 搬過來的\n";
            out += add.trim();
            out.push('\n');
            lr_core::write_atomic(&new, out.as_bytes()).and_then(|_| std::fs::remove_file(&old))
        } else {
            std::fs::rename(&old, &new)
        };
        if let Err(e) = moved {
            eprintln!("lr-server: known-words.txt → ignore-words.txt: {e}");
            return old;
        }
    }
    new
}

/// The noun list's cache: the model's categories and which terms were already asked.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NounsCache {
    #[serde(default = "nouns_schema")]
    pub schema: String,
    /// key → category
    #[serde(default)]
    pub categories: BTreeMap<String, String>,
    /// Keys already sent to the model (not sent again automatically).
    #[serde(default)]
    pub asked: BTreeSet<String>,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub classified_at: Option<String>,
    /// The classification version and the reader's field the categories were made for;
    /// another one drops the cached categories.
    #[serde(default)]
    pub key: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn nouns_schema() -> String {
    NOUNS_SCHEMA.into()
}

pub const READER_FILE: &str = "reading.toml";
pub const VOCAB_FILE: &str = "my-vocab.json";

/// One word or term of the author's own papers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VocabEntry {
    pub key: String,
    pub text: String,
    /// The papers (sha256) it comes from: the vocabulary is rebuilt from these.
    pub sources: BTreeSet<String>,
}

/// `private/LoreReader/my-vocab.json`: the vocabulary of the papers marked 「我的論文」.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MyVocab {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub built_at: String,
    #[serde(default)]
    pub papers: BTreeSet<String>,
    #[serde(default)]
    pub entries: Vec<VocabEntry>,
}

pub fn load_vocab(ws: &FsPath) -> MyVocab {
    std::fs::read(ws.join(VOCAB_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Rebuild with `sha`'s words replaced by `words` (`None` = the paper is no longer mine).
pub fn rebuild_vocab(ws: &FsPath, sha: &str, words: Option<BTreeMap<String, String>>) -> std::io::Result<MyVocab> {
    let mut v = load_vocab(ws);
    let mut map: BTreeMap<String, VocabEntry> = v.entries.drain(..).map(|e| (e.key.clone(), e)).collect();
    for e in map.values_mut() {
        e.sources.remove(sha);
    }
    v.papers.remove(sha);
    if let Some(w) = words {
        v.papers.insert(sha.to_string());
        for (k, t) in w {
            map.entry(k.clone()).or_insert_with(|| VocabEntry { key: k, text: t, sources: BTreeSet::new() }).sources.insert(sha.to_string());
        }
    }
    v.entries = map.into_values().filter(|e| !e.sources.is_empty()).collect();
    v.schema = "lorereader.my-vocab/1".into();
    v.built_at = lr_core::now();
    let mut b = serde_json::to_vec_pretty(&v).unwrap_or_default();
    b.push(b'\n');
    lr_core::write_atomic(&ws.join(VOCAB_FILE), &b)?;
    Ok(v)
}

#[derive(Deserialize)]
pub struct MineBody {
    pub on: bool,
    /// The paper's text (as for `/read`); not needed when the paper is open.
    #[serde(default)]
    pub pages: Option<Vec<lr_core::text::PageText>>,
}

/// 「我的論文」 on / off: the flag in paper.json and the rebuilt my-vocab.
pub async fn mine(State(s): State<Shared>, Path(sha): Path<String>, Json(b): Json<MineBody>) -> R {
    sha_ok(&sha)?;
    let words = if b.on {
        let text = match b.pages {
            Some(p) if !p.is_empty() => Arc::new(PaperText { pages: p }.normalise()),
            _ => text_of(&s, &sha)?,
        };
        let kb = s.kb.lexicon().await.unwrap_or_default();
        Some(nouns::vocabulary(&text, &kb, &Filters::default()))
    } else {
        None
    };
    let _g = s.write.lock().unwrap();
    let m = s.papers.set_mine(&sha, b.on)?;
    let v = rebuild_vocab(&s.workspace, &sha, words).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?;
    Ok(Json(json!({ "paper": m, "mine": b.on, "vocab": { "papers": v.papers.len(), "entries": v.entries.len() } })))
}

/// The reader's field and level for the classification prompt: `field` in
/// `private/LoreReader/reading.toml`, else `field` in `private/shared/profile.toml`,
/// else a neutral default.
pub fn reader_field(s: &crate::AppState) -> String {
    let from = |p: Option<std::path::PathBuf>| -> Option<String> {
        let t = std::fs::read_to_string(p?).ok()?;
        let v: toml::Value = toml::from_str(&t).ok()?;
        v.get("field")?.as_str().map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
    };
    from(Some(s.workspace.join(READER_FILE))).or_else(|| from(s.home.shared_file("profile.toml"))).unwrap_or_else(|| nouns::DEFAULT_FIELD.to_string())
}

/// What the cached categories must have been made for.
fn cache_key(s: &crate::AppState) -> String {
    format!("v{}|{}", nouns::CLASSIFY_VERSION, reader_field(s))
}

/// nouns.json, empty when it was made for another classification version or field.
fn load_cache(s: &crate::AppState, sha: &str) -> Result<NounsCache, ApiError> {
    let c: NounsCache = s.papers.load(sha, "nouns.json")?.unwrap_or_default();
    Ok(if c.key == cache_key(s) { c } else { NounsCache { extra: c.extra, ..Default::default() } })
}

pub async fn reader_get(State(s): State<Shared>) -> R {
    Ok(Json(json!({ "field": reader_field(&s), "default": nouns::DEFAULT_FIELD })))
}

#[derive(Deserialize)]
pub struct FieldBody {
    pub field: String,
}

/// The reader's field (the 模型 dialog). Empty = back to the default.
pub async fn reader_put(State(s): State<Shared>, Json(b): Json<FieldBody>) -> R {
    let f = b.field.split_whitespace().collect::<Vec<_>>().join(" ");
    if f.chars().count() > 200 {
        return Err(bad("太長了"));
    }
    let _g = s.write.lock().unwrap();
    let p = s.workspace.join(READER_FILE);
    let mut v: toml::Table = std::fs::read_to_string(&p).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
    if f.is_empty() {
        v.remove("field");
    } else {
        v.insert("field".into(), toml::Value::String(f));
    }
    let text = format!("# 讀論文的設定。field = 你的領域與程度（名詞分類用來判斷什麼是「基礎」）。\n{}", toml::to_string(&v).unwrap_or_default());
    lr_core::write_atomic(&p, text.as_bytes()).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?;
    drop(_g);
    reader_get(State(s)).await
}

/// The text of an opened paper, or 409 when the page has not sent it.
pub fn text_of(s: &crate::AppState, sha: &str) -> Result<Arc<PaperText>, ApiError> {
    s.texts.lock().unwrap().get(sha).cloned().ok_or_else(|| ApiError(StatusCode::CONFLICT, json!({ "error": "這篇論文的文字還沒讀進來（重新開啟論文）", "no_text": true })))
}

/// `POST /api/papers/{sha}/read`: the page's pdf.js text (all pages of the paper's own
/// PDF). Returns the noun list.
pub async fn read_paper(State(s): State<Shared>, Path(sha): Path<String>, Json(t): Json<PaperText>) -> R {
    sha_ok(&sha)?;
    if t.pages.is_empty() {
        return Err(bad("沒有頁面文字"));
    }
    let t = Arc::new(t.normalise());
    s.texts.lock().unwrap().insert(sha.clone(), t.clone());
    let symbols = rebuild_symbols(&s, &sha, &t)?;
    let nouns = nouns_view(&s, &sha).await?;
    build_paragraphs(&s, &sha).await?;
    spawn_run(s.clone(), sha.clone(), false);
    Ok(Json(json!({ "words": t.words(), "pages": t.pages.len(), "nouns": nouns, "symbols": symbols })))
}

/// Inventory × KB × ignore list × cached categories, and the KB's status.
async fn inventory_of(s: &crate::AppState, sha: &str) -> Result<(nouns::Inventory, Value, NounsCache), ApiError> {
    let text = text_of(s, sha)?;
    let (kb, kb_status) = match s.kb.lexicon().await {
        Ok(l) => (l, json!({ "ok": true })),
        Err(e) => (vec![], json!({ "ok": false, "down": e.down, "error": e.message })),
    };
    let cache = load_cache(s, sha)?;
    let cats = cache.categories.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let syms: SymbolFile = s.papers.load(sha, "symbols.json")?.unwrap_or_default();
    let mut inv = nouns::inventory_masked(&text, &kb, &Filters::with_file(&ignore_file(&s.workspace)), &cats, &math_mask(&syms));
    inv.mark_used(&load_vocab(&s.workspace).entries.iter().map(|e| e.key.clone()).collect());
    Ok((inv, kb_status, cache))
}

/// The page ranges of every math run and display equation (not words).
pub fn math_mask(syms: &SymbolFile) -> Vec<(u32, usize, usize)> {
    let mut m: Vec<(u32, usize, usize)> = syms.entries.iter().flat_map(|e| e.occurrences.iter().map(|o| (o.page, o.s, o.e))).collect();
    m.extend(syms.equations.iter().map(|q| (q.page, q.s, q.e)));
    m.sort();
    m.dedup();
    m
}

/// The noun list now: inventory × KB × ignore list × cached categories.
pub async fn nouns_view(s: &crate::AppState, sha: &str) -> Result<Value, ApiError> {
    let (inv, kb_status, cache) = inventory_of(s, sha).await?;
    let pending = inv.unclassified().iter().filter(|n| !cache.asked.contains(&n.key)).count();
    let entries: Vec<Value> = inv
        .entries
        .iter()
        .map(|n| {
            let mut v = serde_json::to_value(n).unwrap_or(Value::Null);
            v["red"] = n.red().into();
            v
        })
        .collect();
    Ok(json!({
        "entries": entries,
        "stats": inv.stats,
        "used": inv.entries.iter().filter(|n| n.used).count(),
        "kb": kb_status,
        "categories": nouns::CATEGORIES,
        "classification": { "engine": cache.engine, "at": cache.classified_at, "unclassified": inv.unclassified().len(), "pending": pending },
    }))
}

pub async fn nouns_get(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    Ok(Json(nouns_view(&s, &sha).await?))
}

#[derive(Deserialize, Default)]
pub struct ClassifyBody {
    /// Also ask for terms that were asked before and came back without a category.
    #[serde(default)]
    pub again: bool,
}

/// One batched `lr-classify-terms` call for every no-card term without a category.
pub async fn classify(State(s): State<Shared>, Path(sha): Path<String>, body: Option<Json<ClassifyBody>>) -> R {
    sha_ok(&sha)?;
    let again = body.map(|b| b.again).unwrap_or(false);
    let text = text_of(&s, &sha)?;
    let _ = &text;
    let (inv, _, mut cache) = inventory_of(&s, &sha).await?;
    let todo: Vec<&nouns::Noun> = inv.unclassified().into_iter().filter(|n| again || !cache.asked.contains(&n.key)).collect();
    if todo.is_empty() {
        return Ok(Json(json!({ "asked": 0, "nouns": nouns_view(&s, &sha).await? })));
    }
    let terms: Vec<&str> = todo.iter().map(|n| n.text.as_str()).collect();
    let title = s.papers.meta(&sha)?.map(|m| m.title).unwrap_or_default();
    let choice = model_choice(&s.workspace, "lr-classify-terms");
    // the reader's level: their field and up to 40 terms they already know
    let mut known: Vec<String> = s.kb.lexicon().await.unwrap_or_default().into_iter().filter(|n| n.state.as_deref() == Some("known")).map(|n| n.title).collect();
    known.sort();
    known.dedup();
    known.truncate(40);
    // and terms from the author's own papers
    let vocab: BTreeMap<String, String> = load_vocab(&s.workspace).entries.into_iter().map(|e| (e.key, e.text)).collect();
    for t in nouns::vocab_anchors(&vocab, &Filters::default(), 30) {
        if !known.contains(&t) {
            known.push(t);
        }
    }
    let req = LlmRequest::prompt(nouns::CLASSIFY_SYSTEM, nouns::classify_prompt(&title, &reader_field(&s), &known, &terms)).json(nouns::classify_schema()).max_tokens(8000);
    let a = s.llm.complete_on("lr-classify-terms", choice.as_deref(), req).await.map_err(crate::llm_error)?;
    let v = lore_llm::json::extract_json(&a.text).ok_or_else(|| ApiError(StatusCode::BAD_GATEWAY, json!({ "error": format!("模型（{}）沒有回 JSON", a.engine) })))?;
    let got = nouns::parse_classification(&v, &terms);
    {
        let _g = s.write.lock().unwrap();
        let mut fresh = load_cache(&s, &sha)?;
        fresh.key = cache_key(&s);
        fresh.categories.extend(got.clone());
        fresh.asked.extend(todo.iter().map(|n| n.key.clone()));
        fresh.engine = Some(a.engine.clone());
        fresh.classified_at = Some(lr_core::now());
        fresh.schema = NOUNS_SCHEMA.into();
        s.papers.save(&sha, "nouns.json", &fresh)?;
        cache = fresh;
    }
    let _ = cache;
    // the red terms changed: so did the paragraphs' chips
    build_paragraphs(&s, &sha).await?;
    Ok(Json(json!({ "asked": terms.len(), "classified": got.len(), "engine": a.engine, "nouns": nouns_view(&s, &sha).await? })))
}

#[derive(Deserialize)]
pub struct TermBody {
    pub term: String,
}

/// 「這不是術語」: add a word or phrase to the ignore list.
pub async fn ignore_add(State(s): State<Shared>, Json(b): Json<TermBody>) -> R {
    let t = b.term.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() || t.contains('#') || t.chars().count() > 80 {
        return Err(bad("要忽略的字不對"));
    }
    let _g = s.write.lock().unwrap();
    let p = ignore_file(&s.workspace);
    let cur = std::fs::read_to_string(&p).unwrap_or_else(|_| IGNORE_HEADER.into());
    let key = nouns::phrase_key(&t);
    if cur.lines().any(|l| nouns::phrase_key(l.split('#').next().unwrap_or("")) == key) {
        return Ok(Json(json!({ "ignored": t, "new": false })));
    }
    let mut out = cur.trim_end().to_string();
    out.push('\n');
    out += &t;
    out.push('\n');
    lr_core::write_atomic(&p, out.as_bytes()).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?;
    Ok(Json(json!({ "ignored": t, "new": true })))
}

/// Undo 「這不是術語」: remove the lines that are this word or phrase.
pub async fn ignore_remove(State(s): State<Shared>, Json(b): Json<TermBody>) -> R {
    let _g = s.write.lock().unwrap();
    let p = ignore_file(&s.workspace);
    let cur = std::fs::read_to_string(&p).unwrap_or_default();
    let key = nouns::phrase_key(&b.term);
    let kept: Vec<&str> = cur.lines().filter(|l| nouns::phrase_key(l.split('#').next().unwrap_or("")) != key || l.trim_start().starts_with('#')).collect();
    let removed = cur.lines().count() - kept.len();
    if removed > 0 {
        lr_core::write_atomic(&p, (kept.join("\n") + "\n").as_bytes()).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?;
    }
    Ok(Json(json!({ "removed": removed })))
}

// ── symbols ──────────────────────────────────────────────────────────────

pub const GLYPHS_FILE: &str = "glyphs.toml";

/// The glyph table: default, then `private/shared/glyphs.toml`, then the app's own file.
pub fn glyphs(s: &crate::AppState) -> (Glyphs, Vec<String>) {
    let shared = s.home.shared_dir().map(|d| d.join(GLYPHS_FILE));
    let mine = s.workspace.join(GLYPHS_FILE);
    let mut files: Vec<&FsPath> = vec![];
    if let Some(p) = shared.as_deref() {
        files.push(p);
    }
    files.push(&mine);
    Glyphs::load(&files)
}

fn symbols_view(f: &SymbolFile, glyph_errors: &[String]) -> Value {
    let mut v = serde_json::to_value(f).unwrap_or(Value::Null);
    v["grades"] = json!(symbols::GRADES);
    v["glyph_errors"] = json!(glyph_errors);
    v
}

/// Detect, build and merge with what is stored (author / Claude decisions are kept).
fn rebuild_symbols(s: &crate::AppState, sha: &str, t: &PaperText) -> Result<Value, ApiError> {
    let (g, errs) = glyphs(s);
    let det = symbols::detect(t, &g);
    let new = symbols::build(t, &det, sha);
    let _g = s.write.lock().unwrap();
    let old: Option<SymbolFile> = s.papers.load(sha, "symbols.json")?;
    let merged = symbols::merge(old, new);
    s.papers.save(sha, "symbols.json", &merged)?;
    Ok(symbols_view(&merged, &errs))
}

pub async fn symbols_get(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    let f: SymbolFile = s.papers.load(&sha, "symbols.json")?.unwrap_or_default();
    Ok(Json(symbols_view(&f, &glyphs(&s).1)))
}

pub async fn symbols_rebuild(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    let t = text_of(&s, &sha)?;
    Ok(Json(rebuild_symbols(&s, &sha, &t)?))
}

/// An author's change to one entry (only the fields that are sent).
#[derive(Deserialize, Default)]
pub struct SymbolPatch {
    pub latex: Option<String>,
    pub meaning: Option<String>,
    pub meaning_en: Option<String>,
    pub grade: Option<String>,
    pub note: Option<String>,
    /// author (default) | claude | local — who resolved it
    pub by: Option<String>,
    /// The evidence for the change (Claude's answer, a quote) and its page.
    pub evidence: Option<String>,
    pub page: Option<u32>,
}

fn apply_patch(e: &mut SymbolEntry, p: SymbolPatch) -> Result<(), ApiError> {
    if let Some(g) = &p.grade
        && !symbols::GRADES.contains(&g.as_str())
    {
        return Err(bad("等級只能是 certain、defined、inferred、conflict、unresolved"));
    }
    let by = p.by.clone().unwrap_or_else(|| "author".into());
    if !symbols::RESOLVERS.contains(&by.as_str()) || by == "program" {
        return Err(bad("by 只能是 author、claude 或 local"));
    }
    let clean = |x: String| Some(x.trim().to_string()).filter(|x| !x.is_empty());
    if let Some(l) = p.latex {
        e.entry.latex = clean(l).or(e.entry.latex.take());
    }
    if let Some(m) = p.meaning {
        e.entry.meaning = clean(m);
    }
    if let Some(m) = p.meaning_en {
        e.entry.meaning_en = clean(m);
    }
    if let Some(n) = p.note {
        e.note = clean(n);
    }
    if let Some(g) = p.grade {
        e.grade = g;
    }
    if let Some(ev) = p.evidence.and_then(clean) {
        e.evidence.push(Evidence { quote: ev, page: p.page.unwrap_or(0), by: by.clone(), at: Some(lr_core::now()), meaning: e.entry.meaning.clone().or(e.entry.meaning_en.clone()) });
    }
    e.resolved_by = by;
    Ok(())
}

pub async fn symbol_put(State(s): State<Shared>, Path((sha, id)): Path<(String, String)>, Json(p): Json<SymbolPatch>) -> R {
    sha_ok(&sha)?;
    let _g = s.write.lock().unwrap();
    let mut f: SymbolFile = s.papers.load(&sha, "symbols.json")?.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, json!({ "error": "這篇論文還沒有符號表" })))?;
    let e = f.entries.iter_mut().find(|e| e.entry.id == id).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, json!({ "error": format!("沒有符號 {id}") })))?;
    apply_patch(e, p)?;
    let out = e.clone();
    s.papers.save(&sha, "symbols.json", &f)?;
    Ok(Json(json!({ "entry": out })))
}

/// The author adds a symbol the detection missed.
pub async fn symbol_add(State(s): State<Shared>, Path(sha): Path<String>, Json(p): Json<SymbolPatch>) -> Result<(StatusCode, Json<Value>), ApiError> {
    sha_ok(&sha)?;
    let latex = p.latex.clone().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).ok_or_else(|| bad("需要 LaTeX"))?;
    let _g = s.write.lock().unwrap();
    let mut f: SymbolFile = s.papers.load(&sha, "symbols.json")?.unwrap_or_else(|| SymbolFile { schema: symbols::SYMBOLS_SCHEMA.into(), sha256: sha.clone(), ..Default::default() });
    let key = lore_notation::math::canonical(&latex);
    if f.entries.iter().any(|e| e.entry.latex.as_deref() == Some(key.as_str())) {
        return Err(bad("符號表裡已經有這個符號"));
    }
    let mut id = format!("a-{}", key.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>());
    while f.entries.iter().any(|e| e.entry.id == id) {
        id.push('x');
    }
    let mut entry = lore_notation::Entry::new(&id, lore_notation::Kind::Symbol);
    entry.latex = Some(key);
    let mut e = SymbolEntry { entry, scope: sha.clone(), grade: "inferred".into(), resolved_by: "author".into(), evidence: vec![], occurrences: vec![], first: None, equations: vec![], note: None };
    apply_patch(&mut e, SymbolPatch { latex: None, ..p })?;
    f.entries.push(e.clone());
    s.papers.save(&sha, "symbols.json", &f)?;
    Ok((StatusCode::CREATED, Json(json!({ "entry": e }))))
}

const GLYPHS_TEMPLATE: &str = "# 只寫要覆蓋的條目。例：\n# [glyphs]\n# \"ε\" = \"\\\\epsilon\"\n# [fonts]\n# math = [\"MyMathFont\"]\n";

pub async fn glyphs_get(State(s): State<Shared>) -> R {
    let p = s.workspace.join(GLYPHS_FILE);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|_| GLYPHS_TEMPLATE.into());
    Ok(Json(json!({ "text": text, "path": p, "errors": glyphs(&s).1 })))
}

#[derive(Deserialize)]
pub struct GlyphsBody {
    pub text: String,
}

pub async fn glyphs_put(State(s): State<Shared>, Json(b): Json<GlyphsBody>) -> R {
    let n = Glyphs::validate(&b.text).map_err(|e| bad(format!("字形對照表有錯：{e}")))?;
    let _g = s.write.lock().unwrap();
    lr_core::write_atomic(&s.workspace.join(GLYPHS_FILE), b.text.as_bytes()).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?;
    Ok(Json(json!({ "entries": n })))
}

// ── paragraphs (track B) ─────────────────────────────────────────────────

pub const PARAGRAPHS_FILE: &str = "paragraphs.json";
pub const READING_FILE: &str = "reading.json";
/// The question Ask Claude sends for a paragraph unless the author writes another.
pub const DEFAULT_QUESTION: &str = "只解釋我卡住的點，不要翻譯整段";

/// The background run of one paper.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RunStatus {
    pub id: u64,
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
    pub engine: Option<String>,
}

/// paragraphs.json: the model's analysis per paragraph id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParagraphsFile {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub analyses: BTreeMap<String, Analysis>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// reading.json: what the author marked 懂了 (separate from KB states).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadingFile {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub understood: BTreeSet<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Split the paper and attach chips; kept in memory.
async fn build_paragraphs(s: &crate::AppState, sha: &str) -> Result<Arc<Vec<Paragraph>>, ApiError> {
    let text = text_of(s, sha)?;
    let (inv, _, _) = inventory_of(s, sha).await?;
    let syms: SymbolFile = s.papers.load(sha, "symbols.json")?.unwrap_or_default();
    let eqs: Vec<(u32, usize, usize, String)> = syms.equations.iter().map(|q| (q.page, q.s, q.e, q.entry.label.clone().unwrap_or_default())).collect();
    let mut ps = paragraphs::split_with(&text, &eqs);
    paragraphs::attach_chips(&mut ps, &inv, &syms);
    let ps = Arc::new(ps);
    s.paras.lock().unwrap().insert(sha.to_string(), ps.clone());
    Ok(ps)
}

fn paras_of(s: &crate::AppState, sha: &str) -> Option<Arc<Vec<Paragraph>>> {
    s.paras.lock().unwrap().get(sha).cloned()
}

/// Analyse every paragraph without a cached analysis, methods and results first, one at
/// a time; results are saved as they come. A newer run for the same paper stops this one.
pub fn spawn_run(s: Shared, sha: String, again: bool) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let Some(ps) = paras_of(&s, &sha) else { return };
    let order = paragraphs::run_order(&ps);
    s.runs.lock().unwrap().insert(sha.clone(), RunStatus { id, running: true, done: 0, total: order.len(), error: None, engine: None });
    tokio::spawn(async move {
        let title = s.papers.meta(&sha).ok().flatten().map(|m| m.title).unwrap_or_default();
        let syms: SymbolFile = s.papers.load(&sha, "symbols.json").ok().flatten().unwrap_or_default();
        let mine = |s: &crate::AppState| s.runs.lock().unwrap().get(&sha).is_some_and(|r| r.id == id);
        let mut done = 0;
        for i in order {
            if !mine(&s) {
                return;
            }
            let p = &ps[i];
            let cached = s.papers.load::<ParagraphsFile>(&sha, PARAGRAPHS_FILE).ok().flatten().is_some_and(|f| f.analyses.contains_key(&p.id));
            if cached && !again {
                done += 1;
                if let Some(r) = s.runs.lock().unwrap().get_mut(&sha).filter(|r| r.id == id) {
                    r.done = done;
                }
                continue;
            }
            let choice = model_choice(&s.workspace, "lr-paragraph");
            let req = LlmRequest::prompt(paragraphs::PARAGRAPH_SYSTEM, paragraphs::paragraph_prompt(&title, p, &syms)).json(paragraphs::paragraph_schema()).max_tokens(1500);
            let res = s.llm.complete_on("lr-paragraph", choice.as_deref(), req).await;
            let a = match res {
                Ok(a) => a,
                Err(e) => {
                    if let Some(r) = s.runs.lock().unwrap().get_mut(&sha).filter(|r| r.id == id) {
                        r.running = false;
                        r.error = Some(format!("沒有可用的模型：{e}"));
                    }
                    return;
                }
            };
            let Some(v) = lore_llm::json::extract_json(&a.text) else {
                done += 1;
                continue;
            };
            let mut an = paragraphs::parse_analysis(&v);
            an.engine = a.engine.clone();
            an.at = lr_core::now();
            {
                let _g = s.write.lock().unwrap();
                let mut f: ParagraphsFile = s.papers.load(&sha, PARAGRAPHS_FILE).ok().flatten().unwrap_or_default();
                f.schema = paragraphs::PARAGRAPHS_SCHEMA.into();
                f.analyses.insert(p.id.clone(), an);
                let _ = s.papers.save(&sha, PARAGRAPHS_FILE, &f);
            }
            done += 1;
            if let Some(r) = s.runs.lock().unwrap().get_mut(&sha).filter(|r| r.id == id) {
                r.done = done;
                r.engine = Some(a.engine.clone());
            }
        }
        if let Some(r) = s.runs.lock().unwrap().get_mut(&sha).filter(|r| r.id == id) {
            r.running = false;
        }
    });
}

pub async fn paragraphs_get(State(s): State<Shared>, Path(sha): Path<String>) -> R {
    sha_ok(&sha)?;
    let ps = match paras_of(&s, &sha) {
        Some(p) => p,
        None => build_paragraphs(&s, &sha).await?,
    };
    let f: ParagraphsFile = s.papers.load(&sha, PARAGRAPHS_FILE)?.unwrap_or_default();
    let r: ReadingFile = s.papers.load(&sha, READING_FILE)?.unwrap_or_default();
    let list: Vec<Value> = ps
        .iter()
        .map(|p| {
            let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
            v["analysis"] = f.analyses.get(&p.id).map(|a| serde_json::to_value(a).unwrap_or(Value::Null)).unwrap_or(Value::Null);
            v["understood"] = r.understood.contains(&p.id).into();
            v["page"] = p.page().into();
            v
        })
        .collect();
    let readable = ps.iter().filter(|p| p.section_kind != "front").count();
    let understood = ps.iter().filter(|p| p.section_kind != "front" && r.understood.contains(&p.id)).count();
    let status = s.runs.lock().unwrap().get(&sha).cloned().unwrap_or_default();
    Ok(Json(json!({ "paragraphs": list, "status": status, "progress": { "understood": understood, "total": readable }, "default_question": DEFAULT_QUESTION })))
}

#[derive(Deserialize, Default)]
pub struct RunBody {
    #[serde(default)]
    pub again: bool,
}

pub async fn paragraphs_run(State(s): State<Shared>, Path(sha): Path<String>, body: Option<Json<RunBody>>) -> R {
    sha_ok(&sha)?;
    if paras_of(&s, &sha).is_none() {
        build_paragraphs(&s, &sha).await?;
    }
    spawn_run(s.clone(), sha.clone(), body.map(|b| b.again).unwrap_or(false));
    paragraphs_get(State(s), Path(sha)).await
}

#[derive(Deserialize)]
pub struct UnderstoodBody {
    pub on: bool,
}

pub async fn understood(State(s): State<Shared>, Path((sha, pid)): Path<(String, String)>, Json(b): Json<UnderstoodBody>) -> R {
    sha_ok(&sha)?;
    if !pid.starts_with('p') || pid.len() > 40 || !pid[1..].chars().all(|c| c.is_ascii_hexdigit() || c == '-' || c.is_ascii_digit()) {
        return Err(bad("段落 id"));
    }
    let _g = s.write.lock().unwrap();
    let mut r: ReadingFile = s.papers.load(&sha, READING_FILE)?.unwrap_or_default();
    r.schema = "lorereader.reading/1".into();
    if b.on {
        r.understood.insert(pid.clone());
    } else {
        r.understood.remove(&pid);
    }
    s.papers.save(&sha, READING_FILE, &r)?;
    Ok(Json(json!({ "paragraph": pid, "understood": b.on, "count": r.understood.len() })))
}

/// The stuck point a question is about.
#[derive(Debug, Clone, Deserialize)]
pub struct ChipRef {
    /// term | symbol | equation
    pub kind: String,
    pub id: String,
}

/// What Ask Claude gets for a paragraph: title, section, the paragraph (with its page),
/// the chip and its known definition in this paper, and the author's KB states for the
/// terms in the paragraph (known vs not), so the answer uses words the author knows.
pub async fn chat_context(s: &crate::AppState, sha: &str, title: &str, pid: &str, chip: Option<&ChipRef>) -> Result<(String, Option<Value>), ApiError> {
    let ps = match paras_of(s, sha) {
        Some(p) => p,
        None => build_paragraphs(s, sha).await?,
    };
    let p = ps.iter().find(|p| p.id == pid).ok_or_else(|| bad("沒有這個段落（重新開啟論文）"))?;
    let (inv, kb_status, _) = inventory_of(s, sha).await?;
    let syms: SymbolFile = s.papers.load(sha, "symbols.json")?.unwrap_or_default();
    let mut out = format!("〔論文〕{title}\n〔章節〕{}\n〔段落，p.{}〕{}\n", if p.section.is_empty() { "（未知）" } else { &p.section }, p.page(), p.text);
    let mut chip_json = None;
    if let Some(c) = chip {
        let (label, known) = match c.kind.as_str() {
            "symbol" => match syms.entries.iter().find(|e| e.entry.id == c.id) {
                Some(e) => {
                    let latex = e.entry.latex.clone().unwrap_or_default();
                    let def = match (e.entry.meaning.as_deref().or(e.entry.meaning_en.as_deref()), e.evidence.first()) {
                        (Some(m), Some(ev)) => format!("{m}（{}；原文 p.{}：「{}」）", grade_label(&e.grade), ev.page, ev.quote),
                        (Some(m), None) => format!("{m}（{}）", grade_label(&e.grade)),
                        (None, _) => format!("本篇沒找到定義（{}）", grade_label(&e.grade)),
                    };
                    (format!("${latex}$"), def)
                }
                None => (c.id.clone(), "（符號表裡沒有）".into()),
            },
            "equation" => match syms.equations.iter().find(|q| q.entry.id == c.id) {
                Some(q) => (format!("式 {}", q.entry.label.clone().unwrap_or_default()), format!("PDF 讀出的 LaTeX（可能不完整）：{}", q.entry.latex.clone().unwrap_or_default())),
                None => (c.id.clone(), String::new()),
            },
            _ => match inv.entries.iter().find(|n| n.key == c.id) {
                Some(n) => (n.text.clone(), format!("知識庫：{}；分類：{}", state_label(&n.state), n.category.as_deref().unwrap_or("未分類"))),
                None => (c.id.clone(), String::new()),
            },
        };
        out += &format!("〔卡住的地方〕{label}\n〔在本篇的已知定義〕{}\n", if known.is_empty() { "（無）" } else { &known });
        chip_json = Some(json!({ "kind": c.kind, "id": c.id, "label": label }));
    } else {
        // 「這段我卡住了」: every stuck point of the paragraph with what is known here
        let mut lines = vec![];
        for k in &p.chips.terms {
            if let Some(n) = inv.entries.iter().find(|n| &n.key == k) {
                lines.push(format!("- {}（{}；{}）", n.text, state_label(&n.state), n.category.as_deref().unwrap_or("未分類")));
            }
        }
        for id in &p.chips.symbols {
            if let Some(e) = syms.entries.iter().find(|e| &e.entry.id == id) {
                let m = e.entry.meaning.as_deref().or(e.entry.meaning_en.as_deref()).unwrap_or("本篇沒找到定義");
                lines.push(format!("- ${}$：{m}（{}）", e.entry.latex.clone().unwrap_or_default(), grade_label(&e.grade)));
            }
        }
        for id in &p.chips.equations {
            if let Some(q) = syms.equations.iter().find(|q| &q.entry.id == id) {
                lines.push(format!("- 式 {}：{}", q.entry.label.clone().unwrap_or_else(|| "（無編號）".into()), q.entry.latex.clone().unwrap_or_default()));
            }
        }
        if !lines.is_empty() {
            out += &format!("〔這段卡住的地方〕\n{}\n", lines.join("\n"));
        }
    }
    // the author's KB states for the terms in this paragraph
    let inside = |page: u32, st: u32, en: u32| p.ranges.iter().any(|r| r.page == page && st as usize >= r.s && en as usize <= r.e);
    let (mut known, mut learning, mut none) = (vec![], vec![], vec![]);
    for n in inv.entries.iter().filter(|n| n.occ.iter().any(|o| inside(o[0], o[1], o[2]))) {
        match n.state.as_str() {
            "known" => known.push(n.text.clone()),
            "new" if n.used => known.push(n.text.clone()),
            "learning" | "stale" | "none" => learning.push(n.text.clone()),
            _ if n.red() => none.push(n.text.clone()),
            _ => {}
        }
    }
    if kb_status["ok"] == true {
        out += &format!(
            "〔作者的知識庫〕已掌握：{}；學習中：{}；沒有卡片（作者不熟）：{}\n",
            if known.is_empty() { "（無）".into() } else { known.join("、") },
            if learning.is_empty() { "（無）".into() } else { learning.join("、") },
            if none.is_empty() { "（無）".into() } else { none.join("、") }
        );
    } else {
        out += &format!("〔作者的知識庫〕KnowledgeBase 沒有回應；這段沒有卡片的術語：{}\n", if none.is_empty() { "（無）".into() } else { none.join("、") });
    }
    Ok((out, chip_json))
}

fn grade_label(g: &str) -> &'static str {
    match g {
        "certain" => "✅ 確定",
        "defined" => "🟢 有定義",
        "inferred" => "🟡 推測",
        "conflict" => "🔴 本篇有兩種定義",
        _ => "⚪ 查不到",
    }
}

fn state_label(s: &str) -> &'static str {
    match s {
        "known" => "已掌握",
        "learning" | "none" => "學習中",
        "stale" => "久未複習",
        _ => "沒有卡片",
    }
}

// ── model menu ───────────────────────────────────────────────────────────

fn models_file(ws: &FsPath) -> PathBuf {
    ws.join("models.json")
}

fn load_models(ws: &FsPath) -> BTreeMap<String, String> {
    std::fs::read(models_file(ws)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["choices"].as_object().cloned()).map(|m| m.into_iter().filter_map(|(k, v)| v.as_str().map(|x| (k, x.to_string()))).collect()).unwrap_or_default()
}

/// The engine picked for `task` in the model menu (`None` = auto).
pub fn model_choice(ws: &FsPath, task: &str) -> Option<String> {
    load_models(ws).remove(task).filter(|c| c != "auto")
}

pub async fn models_get(State(s): State<Shared>) -> R {
    let chosen = load_models(&s.workspace);
    let tasks: Vec<Value> = llm::MENU_TASKS
        .iter()
        .map(|t| {
            let (label, default) = s.llm.task_info(t);
            json!({ "id": t, "label": label, "default": default, "choice": chosen.get(*t).cloned().unwrap_or_else(|| "auto".into()) })
        })
        .collect();
    Ok(Json(json!({ "tasks": tasks, "choices": llm::CHOICES, "local_engine": s.llm.has_local() })))
}

#[derive(Deserialize)]
pub struct ModelBody {
    pub task: String,
    pub model: String,
}

pub async fn models_put(State(s): State<Shared>, Json(b): Json<ModelBody>) -> R {
    if !llm::MENU_TASKS.contains(&b.task.as_str()) {
        return Err(bad("沒有這個任務"));
    }
    if !llm::CHOICES.contains(&b.model.as_str()) {
        return Err(bad("模型只能是 auto、local、haiku、sonnet 或 opus"));
    }
    let _g = s.write.lock().unwrap();
    let mut m = load_models(&s.workspace);
    m.insert(b.task.clone(), b.model.clone());
    let v = json!({ "schema": "lorereader.models/1", "choices": m });
    let mut bytes = serde_json::to_vec_pretty(&v).unwrap_or_default();
    bytes.push(b'\n');
    lr_core::write_atomic(&models_file(&s.workspace), &bytes).map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?;
    drop(_g);
    models_get(State(s)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_words_migrate_once() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("known-words.txt"), "softmax dropout\n").unwrap();
        let p = ignore_file(d.path());
        assert_eq!(p, d.path().join("ignore-words.txt"));
        assert!(std::fs::read_to_string(&p).unwrap().contains("softmax"));
        assert!(!d.path().join("known-words.txt").exists());
        // both present: appended, the old file goes away
        std::fs::write(d.path().join("known-words.txt"), "logit\n").unwrap();
        let p = ignore_file(d.path());
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("softmax") && t.contains("logit"));
        assert!(!d.path().join("known-words.txt").exists());
    }
}
