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
//! - the model menu: `GET/PUT /api/models` (`private/LoreReader/models.json`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use lore_llm::LlmRequest;
use lr_core::nouns::{self, Filters};
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
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn nouns_schema() -> String {
    NOUNS_SCHEMA.into()
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
    Ok(Json(json!({ "words": t.words(), "pages": t.pages.len(), "nouns": nouns, "symbols": symbols })))
}

/// The noun list now: inventory × KB × ignore list × cached categories.
pub async fn nouns_view(s: &crate::AppState, sha: &str) -> Result<Value, ApiError> {
    let text = text_of(s, sha)?;
    let (kb, kb_status) = match s.kb.lexicon().await {
        Ok(l) => (l, json!({ "ok": true })),
        Err(e) => (vec![], json!({ "ok": false, "down": e.down, "error": e.message })),
    };
    let cache: NounsCache = s.papers.load(sha, "nouns.json")?.unwrap_or_default();
    let cats = cache.categories.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let inv = nouns::inventory(&text, &kb, &Filters::with_file(&ignore_file(&s.workspace)), &cats);
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
    let kb = s.kb.lexicon().await.unwrap_or_default();
    let mut cache: NounsCache = s.papers.load(&sha, "nouns.json")?.unwrap_or_default();
    let cats = cache.categories.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let inv = nouns::inventory(&text, &kb, &Filters::with_file(&ignore_file(&s.workspace)), &cats);
    let todo: Vec<&nouns::Noun> = inv.unclassified().into_iter().filter(|n| again || !cache.asked.contains(&n.key)).collect();
    if todo.is_empty() {
        return Ok(Json(json!({ "asked": 0, "nouns": nouns_view(&s, &sha).await? })));
    }
    let terms: Vec<&str> = todo.iter().map(|n| n.text.as_str()).collect();
    let title = s.papers.meta(&sha)?.map(|m| m.title).unwrap_or_default();
    let choice = model_choice(&s.workspace, "lr-classify-terms");
    let req = LlmRequest::prompt(nouns::CLASSIFY_SYSTEM, nouns::classify_prompt(&title, &terms)).json(nouns::classify_schema()).max_tokens(8000);
    let a = s.llm.complete_on("lr-classify-terms", choice.as_deref(), req).await.map_err(crate::llm_error)?;
    let v = lore_llm::json::extract_json(&a.text).ok_or_else(|| ApiError(StatusCode::BAD_GATEWAY, json!({ "error": format!("模型（{}）沒有回 JSON", a.engine) })))?;
    let got = nouns::parse_classification(&v, &terms);
    {
        let _g = s.write.lock().unwrap();
        let mut fresh: NounsCache = s.papers.load(&sha, "nouns.json")?.unwrap_or_default();
        fresh.categories.extend(got.clone());
        fresh.asked.extend(todo.iter().map(|n| n.key.clone()));
        fresh.engine = Some(a.engine.clone());
        fresh.classified_at = Some(lr_core::now());
        fresh.schema = NOUNS_SCHEMA.into();
        s.papers.save(&sha, "nouns.json", &fresh)?;
        cache = fresh;
    }
    let _ = cache;
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

const GLYPHS_TEMPLATE: &str = "# 你自己的字形 → LaTeX 對照（蓋過內建的；格式同 LoreReader/crates/core/src/glyphs.toml）。\n# 例：\n# [glyphs]\n# \"ε\" = \"\\\\epsilon\"\n# [fonts]\n# math = [\"MyMathFont\"]\n";

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
