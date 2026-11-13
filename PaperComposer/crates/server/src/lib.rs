//! PaperComposer local server, used both by `cargo run -p pc-server` (browser
//! dev mode) and by the desktop app, which starts it in-process and points
//! its window at it.
//!
//! Binds to 127.0.0.1 only. Finds the writing profile in this order:
//! 1. `PAPERCOMPOSER_WORKSPACE` (private repo root; profile in `profile/`)
//! 2. `PAPERCOMPOSER_PROFILE` (a profile folder on its own)
//! 3. `workspace_path` or `profile_path` in `<config dir>/PaperComposer/config.toml`
//! 4. the bundled demo workspace (`../private-example/PaperComposer`; `workspace-template/` inside the app bundle)
//!
//! The config file lives in the OS config folder, never inside a repository,
//! so a personal profile path is never committed by accident.

pub mod citecheck;
pub mod convert;
mod draft;
pub mod floats; // figures and tables, one path for the editor and 寫書
pub mod book; // 寫書: the job that writes a book from a paper (docs/book.md)
mod books; // 寫書: books.toml and 新書
mod home;
mod notation;
mod notes; // notes.js: 筆記 (lore-notes files in the workspace's notes/)
mod plot; // figplot.js: 繪圖 (lore-plot editor, figures/<stem>.pdf + .plot.json)
mod newtemplate; // templates.js: 新增期刊 from a LaTeX kit or a Word sample, rules from the guidelines
mod sections;
mod shell; // the shared Lore shell: its files, 列印, 另存新檔
pub mod export;
pub mod import;
pub mod opened;
pub mod llm;
pub mod preview;
pub mod proc;
mod kb; // kbcards.js: 知識卡 (KnowledgeBase cards, read-only)
mod reader;
pub mod models;
pub mod refimport;
pub mod settings;
pub mod texfonts;
pub mod texmeasure;
pub mod thumb;
mod todeck; // todeck.js: 做成簡報 (SlideComposer makes the deck)
pub mod wizard;
pub mod word;

use axum::{
    extract::{Path as AxPath, Query as AxQuery, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post, put},
    Router,
};
use llm::{EngineChoice, EngineConfig, EngineStatus, Llm};
use pc_core::compose::{self, ComposeRequest, Draft, DraftCheck, Prompt};
use pc_core::authors::{self, Affiliation, Author, PaperAuthor, Person};
use pc_core::meta::{self, OutlineSection, PaperMeta};
use pc_core::paper::{self, Section};
use pc_core::table::{self, Table};
use pc_core::bib;
use pc_core::softcheck::{self, SoftResult};
use pc_core::{check_text, decompose, search, Hit, Issue, Profile, Query};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;
use tower_http::services::ServeDir;

/// Where the UI and the demo profile live. The desktop app passes its bundled
/// resource folder; dev mode uses the repository.
#[derive(Clone)]
pub struct Paths {
    pub ui: PathBuf,
    pub demo_workspace: PathBuf,
}

impl Paths {
    pub fn from_repo() -> Paths {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = root.canonicalize().unwrap_or(root);
        Paths { ui: root.join("ui"), demo_workspace: root.join("../private-example/PaperComposer") }
    }
}

struct Loaded {
    profile: Option<Profile>,
    issues: Vec<Issue>,
    error: Option<String>,
    /// Profile folder.
    path: PathBuf,
    /// Workspace root (the private repository), when one is used.
    workspace: Option<PathBuf>,
    demo: bool,
}

struct AppState {
    loaded: RwLock<Loaded>,
    /// Engines and task routing (lore-llm).
    llm: Arc<Llm>,
    models: Arc<models::Manager>,
    paths: Paths,
    /// Soft-check results by prompt hash: an unchanged paragraph is not re-sent.
    soft_cache: tokio::sync::Mutex<std::collections::HashMap<u64, Vec<SoftResult>>>,
}

type Shared = Arc<AppState>;

#[derive(Deserialize, Default)]
struct Config {
    /// Root of the private repository: profile/, publishers/, papers/.
    workspace_path: Option<PathBuf>,
    /// Profile folder on its own (when not using a workspace).
    profile_path: Option<PathBuf>,
    engine: Option<EngineConfig>,
    claude: Option<ClaudeConfig>,
    model: Option<ModelConfig>,
    /// This machine's folder of reference PDFs (never synced).
    pdf_dir: Option<PathBuf>,
    /// Shared local-model store used by every project on this machine
    /// (fallback for the LOCAL_LLM_HOME environment variable).
    local_llm_home: Option<PathBuf>,
}

#[derive(Deserialize, Default)]
struct ClaudeConfig {
    model: Option<String>,
}

#[derive(Deserialize, Default)]
struct ModelConfig {
    /// Catalog id of the local model to start with the app.
    active: Option<String>,
}

/// Set a top-level `key = value` in config.toml.
fn write_config_root(key: &str, value: &str) -> Result<(), String> {
    let path = config_path().ok_or("no config folder on this system")?;
    let mut doc: toml::Table = std::fs::read_to_string(&path).ok().and_then(|raw| raw.parse().ok()).unwrap_or_default();
    doc.insert(key.to_string(), toml::Value::String(value.to_string()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, toml::to_string(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("PaperComposer/config.toml"))
}

fn read_config() -> Config {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Set one `[table] key = value` in config.toml, keeping everything else.
fn write_config_value(table: &str, key: &str, value: &str) -> Result<(), String> {
    let path = config_path().ok_or("no config folder on this system")?;
    let mut doc: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or_default();
    let section = doc
        .entry(table.to_string())
        .or_insert_with(|| toml::Value::Table(Default::default()));
    if let toml::Value::Table(t) = section {
        t.insert(key.to_string(), toml::Value::String(value.to_string()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, toml::to_string(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Copy files that do not exist yet; never overwrite the user's files.
fn copy_missing(from: &Path, to: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut n = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            n += copy_missing(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), target)?;
            n += 1;
        }
    }
    Ok(n)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

// ── theme: the one every LoreForge app shows (commons/ui, lore-home) ──────
async fn theme_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::CSS)
}

/// theme.js with the shared choice in front, so the first paint has the theme.
async fn theme_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], lore_home::theme::script(&loreforge_home()))
}

fn theme_value(home: &lore_home::Home) -> serde_json::Value {
    serde_json::json!({ "theme": home.theme() })
}

fn theme_set(home: &lore_home::Home, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    home.set_theme(body["theme"].as_str().unwrap_or_default()).map(|t| serde_json::json!({ "theme": t })).map_err(|e| e.to_string())
}

async fn theme_get() -> Json<serde_json::Value> {
    Json(theme_value(&loreforge_home()))
}

async fn theme_put(Json(b): Json<serde_json::Value>) -> axum::response::Response {
    match theme_set(&loreforge_home(), &b) {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}

/// The demo workspace is copied into the app's data folder on first use, so
/// trying the app never edits the bundled (or repository) copy.
fn demo_workspace(paths: &Paths) -> PathBuf {
    let Some(target) = dirs::data_dir().map(|d| d.join("PaperComposer/demo-workspace")) else {
        return paths.demo_workspace.clone();
    };
    if !target.exists() && copy_dir(&paths.demo_workspace, &target).is_err() {
        return paths.demo_workspace.clone();
    }
    target
}

pub(crate) fn loreforge_home() -> lore_home::Home {
    // unit tests never read the author's private folders (registry, prefs): an empty per-process home
    #[cfg(test)]
    if std::env::var_os("LOREFORGE_PRIVATE").is_none() {
        return lore_home::Home::at(std::env::temp_dir().join(format!("pc-unit-home-{}", std::process::id())));
    }
    lore_home::Home::discover(Path::new(env!("CARGO_MANIFEST_DIR")))
}

/// Interface fonts from LoreForge `private/shared/fonts.toml` (CSS variables; linked after styles.css).
async fn prefs_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], loreforge_home().prefs().fonts.css())
}

/// Resolution order: PAPERCOMPOSER_WORKSPACE, PAPERCOMPOSER_PROFILE,
/// config workspace_path, config profile_path, LoreForge private/PaperComposer,
/// bundled demo workspace.
fn locate(paths: &Paths) -> (PathBuf, Option<PathBuf>, bool) {
    let env = |k| std::env::var(k).ok().map(PathBuf::from);
    if let Some(ws) = env("PAPERCOMPOSER_WORKSPACE") {
        return (ws.join("profile"), Some(ws), false);
    }
    if let Some(p) = env("PAPERCOMPOSER_PROFILE") {
        return (p, None, false);
    }
    let cfg = read_config();
    if let Some(ws) = cfg.workspace_path {
        return (ws.join("profile"), Some(ws), false);
    }
    if let Some(p) = cfg.profile_path {
        return (p, None, false);
    }
    // LoreForge: private/PaperComposer when it exists (the setup page creates it otherwise).
    if let Some(ws) = loreforge_home().app_dir("PaperComposer").filter(|d| d.join("profile").is_dir()) {
        return (ws.join("profile"), Some(ws), false);
    }
    let ws = demo_workspace(paths);
    (ws.join("profile"), Some(ws), true)
}

fn load(paths: &Paths) -> Loaded {
    let (path, workspace, demo) = locate(paths);
    match Profile::load(&path) {
        Ok((profile, issues)) => Loaded { profile: Some(profile), issues, error: None, path, workspace, demo },
        Err(e) => Loaded { profile: None, issues: vec![], error: Some(e), path, workspace, demo },
    }
}

#[derive(Serialize)]
struct WorkspaceInfo {
    root: Option<String>,
    demo: bool,
    papers: Vec<String>,
    publishers: Vec<String>,
    /// Journal name from each template's README heading, when it has one.
    publisher_titles: std::collections::BTreeMap<String, String>,
    /// "desktop" when native file dialogs are there (opened.rs), else "browser".
    shell: &'static str,
}

fn subdirs(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    out.sort();
    out
}

async fn workspace_info(State(s): State<Shared>) -> Json<WorkspaceInfo> {
    let l = s.loaded.read().await;
    let ws = l.workspace.as_deref();
    let publishers = ws.map(|w| subdirs(&w.join("publishers"))).unwrap_or_default();
    Json(WorkspaceInfo {
        root: ws.map(|w| w.display().to_string()),
        demo: l.demo,
        // names a new paper cannot take: the workspace's and those of papers opened elsewhere
        papers: ws.map(|w| subdirs(&w.join("papers"))).unwrap_or_default().into_iter().chain(opened::registry().papers.into_iter().map(|e| e.id)).collect(),
        publisher_titles: ws
            .map(|w| pc_core::journal::list(&w.join("publishers")).into_iter().map(|j| (j.id, j.meta.name)).collect())
            .unwrap_or_default(),
        publishers,
        shell: if opened::DIALOG.get().is_some() { "desktop" } else { "browser" },
    })
}

#[derive(Serialize)]
struct ProfileInfo {
    name: Option<String>,
    path: String,
    demo: bool,
    error: Option<String>,
    phrases: usize,
    vocabulary: usize,
    rules: usize,
    sections: Vec<String>,
    functions: Vec<String>,
    issues: Vec<Issue>,
}

async fn profile_info(State(s): State<Shared>) -> Json<ProfileInfo> {
    let l = s.loaded.read().await;
    let (mut sections, mut functions) = (Vec::new(), Vec::new());
    if let Some(p) = &l.profile {
        for ph in &p.phrases {
            if !sections.contains(&ph.section) {
                sections.push(ph.section.clone());
            }
            if !functions.contains(&ph.function) {
                functions.push(ph.function.clone());
            }
        }
    }
    functions.sort();
    Json(ProfileInfo {
        name: l.profile.as_ref().map(|p| p.manifest.name.clone()),
        path: l.path.display().to_string(),
        demo: l.demo,
        error: l.error.clone(),
        phrases: l.profile.as_ref().map_or(0, |p| p.phrases.len()),
        vocabulary: l.profile.as_ref().map_or(0, |p| p.vocabulary.len()),
        rules: l.profile.as_ref().map_or(0, |p| p.rules.len()),
        sections,
        functions,
        issues: l.issues.clone(),
    })
}

/// Rule id -> message, so the UI can name soft rules that are still pending.
async fn rule_messages(State(s): State<Shared>) -> Json<std::collections::BTreeMap<String, String>> {
    let l = s.loaded.read().await;
    Json(
        l.profile
            .iter()
            .flat_map(|p| &p.rules)
            .map(|r| (r.id.clone(), if r.message.is_empty() { r.check.clone() } else { r.message.clone() }))
            .collect(),
    )
}

async fn reload(State(s): State<Shared>) -> impl IntoResponse {
    *s.loaded.write().await = load(&s.paths);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct SoftCheckBody {
    /// One paragraph.
    text: String,
    section: Option<String>,
    before: Option<String>,
    after: Option<String>,
    #[serde(default)]
    engine: Option<EngineChoice>,
    /// The paper's key value, for the rule that checks the paragraph stays on topic.
    #[serde(default)]
    key_value: Option<String>,
}

/// Judge one paragraph against the soft rules in rules.csv with a model.
async fn soft_check(State(s): State<Shared>, Json(b): Json<SoftCheckBody>) -> impl IntoResponse {
    let (prompt, rules) = {
        let l = s.loaded.read().await;
        let Some(p) = &l.profile else { return bad("profile not loaded") };
        let kv = b.key_value.as_deref().map(str::trim).filter(|k| !k.is_empty());
        let mut rules = softcheck::soft_rules(p, b.section.as_deref());
        if kv.is_none() {
            rules.retain(|r| r.check != softcheck::KEY_VALUE_CHECK);
        }
        let prompt = softcheck::build_prompt(&rules, &b.text, b.section.as_deref(), b.before.as_deref(), b.after.as_deref(), kv);
        (prompt, rules.into_iter().cloned().collect::<Vec<_>>())
    };
    if rules.is_empty() || b.text.trim().is_empty() {
        return Json(serde_json::json!({ "results": [], "cached": false })).into_response();
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&prompt.system, &prompt.user).hash(&mut h);
        h.finish()
    };
    if let Some(hit) = s.soft_cache.lock().await.get(&key) {
        return Json(serde_json::json!({ "results": hit, "cached": true })).into_response();
    }
    if !has_writing_engine(&s, b.engine).await {
        return Json(serde_json::json!({ "results": [], "error": "no engine" })).into_response();
    }
    let refs: Vec<&pc_core::Rule> = rules.iter().collect();
    // One retry when the reply cannot be read.
    let mut extra = None;
    let mut last = String::new();
    for _ in 0..2 {
        match writing_complete(&s, "pc-soft-check", b.engine, &prompt, extra, Some(softcheck::reply_schema())).await {
            Ok(reply) => match softcheck::parse_reply(&reply, &refs, &b.text) {
                Ok(results) => {
                    s.soft_cache.lock().await.insert(key, results.clone());
                    return Json(serde_json::json!({ "results": results, "cached": false })).into_response();
                }
                Err(e) => {
                    last = e;
                    extra = Some("Reply with the JSON object only, one result per listed rule.");
                }
            },
            Err(e) => return Json(serde_json::json!({ "results": [], "error": e })).into_response(),
        }
    }
    Json(serde_json::json!({ "results": [], "error": format!("unreadable reply: {last}") })).into_response()
}

#[derive(Deserialize)]
struct TableText {
    text: String,
}

/// LaTeX table → editable grid, or why it has to be edited as code.
async fn table_parse(Json(b): Json<TableText>) -> impl IntoResponse {
    match pc_core::textable::parse(&b.text) {
        Ok(t) => Json(serde_json::json!({ "table": t })),
        Err(e) => Json(serde_json::json!({ "error": e })),
    }
}

#[derive(Deserialize)]
struct FitBody {
    table: pc_core::textable::TexTable,
}

/// Measure a table with the paper's class and advise how to fit it, using
/// its style's limits (or general ones for an unstyled table).
async fn table_fit(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<FitBody>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let style = b.table.style.as_deref().and_then(find_style).or_else(|| find_style("booktabs")).expect("booktabs style");
    let base_env = if b.table.style.is_some() { style.look.env.clone() } else { "table".into() };
    match texmeasure::measure(&dir, &b.table).await {
        Ok(m) => {
            let advice = pc_core::textable::advise(&b.table, &style.limits, &base_env, &m);
            Json(serde_json::json!({ "advice": advice, "measured": m })).into_response()
        }
        Err(e) => Json(serde_json::json!({ "error": e })).into_response(),
    }
}

async fn table_styles() -> impl IntoResponse {
    Json(pc_core::textable::builtin_styles())
}

#[derive(Deserialize)]
struct ApplyStyle {
    table: pc_core::textable::TexTable,
    style: String,
}

fn find_style(id: &str) -> Option<pc_core::textable::Style> {
    pc_core::textable::builtin_styles().into_iter().find(|s| s.id == id)
}

/// Restyle a table; the reply also lists the packages the style needs.
async fn table_apply(Json(b): Json<ApplyStyle>) -> impl IntoResponse {
    let Some(style) = find_style(&b.style) else { return bad("unknown style") };
    let mut t = b.table;
    pc_core::textable::apply_style(&mut t, &style);
    Json(serde_json::json!({ "table": t, "packages": style.packages })).into_response()
}

#[derive(Deserialize)]
struct NewTable {
    style: String,
}

async fn table_new(Json(b): Json<NewTable>) -> impl IntoResponse {
    let Some(style) = find_style(&b.style) else { return bad("unknown style") };
    match pc_core::textable::new_table(&style) {
        Ok(t) => Json(serde_json::json!({ "text": pc_core::textable::render(&t), "table": t, "packages": style.packages })).into_response(),
        Err(e) => bad(&e),
    }
}

/// Editable grid → LaTeX.
async fn table_render(Json(t): Json<pc_core::textable::TexTable>) -> impl IntoResponse {
    Json(serde_json::json!({ "text": pc_core::textable::render(&t) }))
}

async fn phrases(State(s): State<Shared>, AxQuery(q): AxQuery<Query>) -> impl IntoResponse {
    let l = s.loaded.read().await;
    match &l.profile {
        Some(p) => Json(search(&p.phrases, &q)).into_response(),
        None => (StatusCode::SERVICE_UNAVAILABLE, "profile not loaded").into_response(),
    }
}

#[derive(Deserialize)]
struct CheckBody {
    text: String,
    section: Option<String>,
}

async fn check(State(s): State<Shared>, Json(b): Json<CheckBody>) -> impl IntoResponse {
    let l = s.loaded.read().await;
    match &l.profile {
        Some(p) => Json(check_text(p, &b.text, b.section.as_deref())).into_response(),
        None => (StatusCode::SERVICE_UNAVAILABLE, "profile not loaded").into_response(),
    }
}

async fn engine_status(State(s): State<Shared>) -> impl IntoResponse {
    if !s.llm.local_configured().await {
        let c = s.llm.claude_status();
        if c.configured {
            return Json(EngineStatus { connected: true, name: format!("Claude · {}", c.model) });
        }
    }
    Json(s.llm.local_status().await)
}

/// Both writing engines, for the engine switch in the compose panel.
#[derive(Deserialize, Default)]
struct Fresh {
    #[serde(default)]
    fresh: Option<u8>,
}

/// The three writing engines, for the engine switch and the setup dialog.
/// `?fresh=1` re-reads the Claude Code login instead of the cached answer.
async fn engines_status(State(s): State<Shared>, AxQuery(f): AxQuery<Fresh>) -> impl IntoResponse {
    let c = s.llm.claude_status();
    let cc = s.llm.cc.get(&s.llm.cli, f.fresh.is_some()).await;
    // Downloaded models, in catalog order, for the local engine's menu.
    let have = models::installed(&s.llm.catalog);
    let local_models: Vec<serde_json::Value> = s
        .llm
        .catalog
        .brains(true)
        .filter(|m| have.contains(&m.id))
        .map(|m| serde_json::json!({ "id": m.id, "name": m.name, "variant": m.variant, "summary": m.summary, "uncensored": m.uncensored }))
        .collect();
    let running = s.models.running().await.map(|r| r.0);
    Json(serde_json::json!({
        "local": { "configured": s.llm.local_configured().await, "status": s.llm.local_status().await, "models": local_models, "running": running },
        "subscription": { "configured": cc.ready, "status": cc },
        "api": { "configured": c.configured, "masked": c.masked, "source": c.source },
        "model": c.model,
        "models": c.models,
    }))
}

async fn claude_code_login(State(s): State<Shared>) -> impl IntoResponse {
    match lore_llm::claude_code::start_login(&s.llm.cli) {
        Ok(()) => {
            s.llm.cc.get(&s.llm.cli, true).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct TestBody {
    engine: EngineChoice,
}

/// Round-trip a tiny prompt through one engine, to prove the binding works.
async fn test_engine(State(s): State<Shared>, Json(b): Json<TestBody>) -> impl IntoResponse {
    let prompt = Prompt { system: "Reply with exactly: OK".into(), user: "ping".into() };
    let t = std::time::Instant::now();
    let r = writing_complete(&s, "pc-engine-test", Some(b.engine), &prompt, None, None).await;
    let ms = t.elapsed().as_millis() as u64;
    match r {
        Ok(reply) => Json(serde_json::json!({ "ok": true, "ms": ms, "reply": reply.trim() })),
        Err(e) => Json(serde_json::json!({ "ok": false, "ms": ms, "error": if e == "no engine" { "尚未完成設定".to_string() } else { e } })),
    }
}

async fn engine_ready(s: &Shared, e: EngineChoice) -> bool {
    s.llm.ready(e).await
}

/// The engine that runs: the chosen one, or the first ready of local,
/// subscription, API.
async fn resolve_engine(s: &Shared, choice: Option<EngineChoice>) -> Option<EngineChoice> {
    match choice {
        Some(e) => engine_ready(s, e).await.then_some(e),
        None => {
            for e in [EngineChoice::Local, EngineChoice::Subscription, EngineChoice::Api] {
                if engine_ready(s, e).await {
                    return Some(e);
                }
            }
            None
        }
    }
}

/// Run `task` through the router: the engine switch's choice is an override
/// for this call; without one the task's order (local, then Claude) applies.
/// "no engine" when nothing is set up.
async fn writing_complete(s: &Shared, task: &str, choice: Option<EngineChoice>, prompt: &Prompt, extra: Option<&str>, schema: Option<serde_json::Value>) -> Result<String, String> {
    resolve_engine(s, choice).await.ok_or("no engine")?;
    s.llm.complete(task, choice, prompt, extra, schema).await
}

async fn has_writing_engine(s: &Shared, choice: Option<EngineChoice>) -> bool {
    resolve_engine(s, choice).await.is_some()
}

#[derive(Deserialize)]
struct ComposeBody {
    #[serde(flatten)]
    req: ComposeRequest,
    #[serde(default)]
    engine: Option<EngineChoice>,
}

#[derive(Serialize)]
struct ComposeReply {
    /// Patterns offered to the engine (shown even when no engine is connected).
    references: Vec<Hit>,
    draft: Option<Draft>,
    check: Option<DraftCheck>,
    /// Set when no engine ran, or it failed.
    error: Option<String>,
    /// The exact prompt, for inspection.
    prompt: Prompt,
}

async fn compose_paragraph(State(s): State<Shared>, Json(body): Json<ComposeBody>) -> impl IntoResponse {
    let (req, choice) = (body.req, body.engine);
    let (points, refs, prompt) = {
        let l = s.loaded.read().await;
        let Some(p) = &l.profile else {
            return (StatusCode::SERVICE_UNAVAILABLE, "profile not loaded").into_response();
        };
        let points = compose::parse_points(&req.points);
        let refs = compose::reference_phrases(p, &req, &points);
        let prompt = compose::build_prompt(p, &req, &points, &refs);
        (points, refs, prompt)
    };
    // 生成時參考知識卡 (off by default): the author's cards for the terms in the points and the paragraph
    let prompt = kb::with_cards(&s, prompt, &[req.points.as_str(), req.before.as_deref().unwrap_or("")].join("\n")).await;
    if points.is_empty() {
        return (StatusCode::BAD_REQUEST, "no points given").into_response();
    }
    if !has_writing_engine(&s, choice).await {
        return Json(ComposeReply {
            references: refs,
            draft: None,
            check: None,
            error: Some("no engine".into()),
            prompt,
        })
        .into_response();
    }

    // One retry when the reply is not valid JSON or fails validation.
    let mut feedback: Option<String> = None;
    let mut last_err = None;
    for _ in 0..2 {
        let reply = match writing_complete(&s, "pc-compose-paragraph", choice, &prompt, feedback.as_deref(), Some(compose::reply_schema())).await {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(e);
                break;
            }
        };
        match compose::parse_draft(&reply) {
            Ok(mut draft) => {
                let l = s.loaded.read().await;
                let Some(p) = &l.profile else { break };
                let check = compose::check_draft(p, &req, &points, &refs, &mut draft);
                if check.ok() || feedback.is_some() {
                    return Json(ComposeReply { references: refs, draft: Some(draft), check: Some(check), error: None, prompt })
                        .into_response();
                }
                feedback = Some(format!("Your previous answer had problems: {}. Fix them.", check.problems.join("; ")));
            }
            Err(e) => {
                last_err = Some(format!("unreadable reply: {e}"));
                feedback = Some("Reply with the JSON object only.".into());
            }
        }
    }
    Json(ComposeReply { references: refs, draft: None, check: None, error: last_err, prompt }).into_response()
}

// ── Papers ─────────────────────────────────────────────

fn bad(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, msg.to_string()).into_response()
}

async fn paper_dir(s: &Shared, name: &str) -> Result<PathBuf, axum::response::Response> {
    if !paper::safe_name(name) {
        return Err(bad("invalid paper name"));
    }
    // the workspace's papers/<name>/, else a folder opened where it is (opened.rs)
    let ws = s.loaded.read().await.workspace.clone();
    if let Some(dir) = ws.map(|w| w.join("papers").join(name)).filter(|d| d.is_dir()) {
        return Ok(dir);
    }
    match opened::external_dir(name) {
        Some(dir) if dir.is_dir() => Ok(dir),
        Some(dir) => Err((StatusCode::NOT_FOUND, format!("找不到 {}", dir.display())).into_response()),
        None => Err((StatusCode::NOT_FOUND, "paper not found").into_response()),
    }
}

/// A new workspace paper may not take the id of a paper opened elsewhere.
fn name_taken_outside(name: &str) -> Option<axum::response::Response> {
    opened::registry().get(name.trim()).map(|_| bad(&format!("已經有一篇叫 {} 的論文", name.trim())))
}

#[derive(Deserialize)]
struct NewPaper {
    name: String,
    /// A folder under publishers/; none = blank article.
    #[serde(default)]
    template: Option<String>,
    /// The one thing the whole paper argues; empty = asked later.
    #[serde(default)]
    key_value: String,
    #[serde(default)]
    title: Option<String>,
    /// From the profile's authors.csv, in order.
    #[serde(default)]
    authors: Vec<PaperAuthor>,
    /// Section skeleton from the wizard; empty = the template's sections.
    #[serde(default)]
    outline: Vec<OutlineSection>,
    /// Results the user already has, placed in their sections in this order.
    #[serde(default)]
    tables: Vec<TableSpec>,
}

#[derive(Deserialize)]
struct TableSpec {
    /// "table" (the default) or "figure".
    #[serde(default)]
    kind: String,
    #[serde(default)]
    caption: String,
    /// Section title to put it in.
    #[serde(default)]
    section: String,
    /// Pasted cells: spreadsheet copy, Markdown table or CSV; first row = header.
    #[serde(default)]
    data: String,
    /// Figure: its file name under figures/, uploaded once the paper exists.
    #[serde(default)]
    file: String,
    /// Table style id; empty = the journal's own.
    #[serde(default)]
    style: Option<String>,
}

/// The table style that matches a paper's class.
fn journal_table_style(main: &str, template: Option<&str>) -> &'static str {
    use authors::Style as S;
    match authors::detect(main) {
        S::Springer => "springer-nature",
        S::Elsevier | S::Cas => "elsevier",
        S::IeeeJournal | S::IeeeConference => "ieee",
        S::Lncs => "lncs",
        S::Blocks { .. } if template.is_some_and(|t| t.contains("cvpr") || t.contains("iccv")) => "cvpr",
        S::Blocks { .. } => "booktabs",
    }
}

/// `tab:results-test-set` (or `fig:...`) from a caption.
fn float_label(prefix: &str, caption: &str, i: usize, taken: &[String]) -> String {
    let words: Vec<String> = caption
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() > 2)
        .take(3)
        .map(|w| w.to_lowercase())
        .collect();
    let base = if words.is_empty() { format!("{prefix}:{}{}", &prefix[..1], i + 1) } else { format!("{prefix}:{}", words.join("-")) };
    let mut label = base.clone();
    let mut n = 2;
    while taken.contains(&label) {
        label = format!("{base}-{n}");
        n += 1;
    }
    label
}

/// Put the user's tables at the end of their sections, in order, and load
/// the packages their style needs.
fn add_tables(dir: &Path, template: Option<&str>, specs: &[TableSpec]) -> Result<(), String> {
    if specs.is_empty() {
        return Ok(());
    }
    let main = std::fs::read_to_string(dir.join("main.tex")).map_err(|e| e.to_string())?;
    let default_style = journal_table_style(&main, template);
    let mut sections = paper::load_paper(dir);
    if sections.is_empty() {
        return Err("這篇論文沒有章節可以放表格".into());
    }
    let mut packages: Vec<String> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut touched = vec![false; sections.len()];
    let (mut nt, mut nf) = (0, 0);
    for spec in specs {
        let block = if spec.kind == "figure" {
            if !figure_name_ok(&spec.file) {
                return Err(format!("圖檔名稱不合法：{}", spec.file));
            }
            let caption = if spec.caption.trim().is_empty() { format!("Figure {}", nf + 1) } else { meta::tex_text(&spec.caption) };
            let label = float_label("fig", &spec.caption, nf, &labels);
            labels.push(label.clone());
            nf += 1;
            packages.push("graphicx".into());
            format!("\\begin{{figure}}[t]\n\\centering\n\\includegraphics[width=\\linewidth]{{figures/{}}}\n\\caption{{{caption}}}\n\\label{{{label}}}\n\\end{{figure}}", spec.file)
        } else {
            let grid = pc_core::textable::parse_grid(&spec.data);
            if grid.is_empty() {
                return Err(format!("第 {} 個表格沒有資料", nt + 1));
            }
            let id = spec.style.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(default_style);
            let style = find_style(id).or_else(|| find_style("booktabs")).expect("booktabs style");
            let caption = if spec.caption.trim().is_empty() { format!("Table {}", nt + 1) } else { meta::tex_text(&spec.caption) };
            let label = float_label("tab", &spec.caption, nt, &labels);
            labels.push(label.clone());
            nt += 1;
            let table = pc_core::textable::from_grid(&grid, &caption, &label, &style);
            packages.extend(style.packages.iter().cloned());
            packages.extend(pc_core::textable::needed_packages(&table));
            pc_core::textable::render(&table).trim_end().to_string()
        };
        let want = spec.section.trim();
        let si = sections
            .iter()
            .position(|s| s.title.eq_ignore_ascii_case(want) || s.title.eq_ignore_ascii_case(&meta::tex_text(want)))
            .or_else(|| sections.iter().position(|s| s.title.to_lowercase().contains("result")))
            .unwrap_or(sections.len() - 1);
        sections[si].paragraphs.push(block);
        touched[si] = true;
    }
    for (s, t) in sections.iter().zip(&touched) {
        if *t {
            paper::write_atomic(&dir.join("sections").join(&s.file), &paper::render_section(s)).map_err(|e| e.to_string())?;
        }
    }
    packages.sort();
    packages.dedup();
    paper::ensure_packages(dir, &packages).map(|_| ())
}

/// A figure file name the wizard may write: plain characters and an image type.
fn figure_name_ok(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "pdf" | "eps")
}

#[derive(Deserialize)]
struct FigureUpload {
    file: String,
}

/// Save an image into a paper's figures/ folder.
async fn put_figure(State(s): State<Shared>, AxPath(name): AxPath<String>, AxQuery(q): AxQuery<FigureUpload>, body: axum::body::Bytes) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    if !figure_name_ok(&q.file) {
        return bad("圖檔要是 PNG、JPG、PDF 或 EPS，名稱只用英數字與 - _ .");
    }
    match floats::save_figure_file(&dir, &q.file, &body) {
        Ok(path) => Json(serde_json::json!({ "path": path })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// The authors picked for a paper, resolved against the profile.
async fn resolve_people(s: &Shared, chosen: &[PaperAuthor]) -> Result<Option<(Vec<Person>, Vec<Affiliation>)>, String> {
    if chosen.is_empty() {
        return Ok(None);
    }
    let l = s.loaded.read().await;
    let p = l.profile.as_ref().ok_or("profile not loaded")?;
    authors::resolve(chosen, &p.authors, &p.affiliations).map(Some)
}

/// paper.toml, the title and the author block of a paper just made.
fn finish_paper(dir: &Path, key_value: &str, title: Option<&str>, chosen: &[PaperAuthor], people: Option<(Vec<Person>, Vec<Affiliation>)>) -> Result<(), String> {
    meta::write_meta(dir, &PaperMeta { key_value: key_value.to_string(), authors: chosen.to_vec(), ..Default::default() })?;
    let path = dir.join("main.tex");
    let mut main = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
        main = pc_core::front::set_field(&main, "title", 0, &meta::tex_text(t))?;
    }
    if let Some((p, a)) = people {
        main = authors::apply(&main, &p, &a);
    }
    paper::write_atomic(&path, &main).map_err(|e| e.to_string())
}

/// `Cache-Control: no-cache` on the static UI files (like SlideComposer and KnowledgeBase).
async fn no_cache(mut res: axum::response::Response) -> axum::response::Response {
    res.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
    res
}

async fn create_paper(State(s): State<Shared>, Json(b): Json<NewPaper>) -> impl IntoResponse {
    let Some(ws) = s.loaded.read().await.workspace.clone() else {
        return bad("no workspace");
    };
    // asked in 討論草稿 step 1 (or in the editor's key value bar) when left empty here
    let key_value = if b.key_value.trim().is_empty() { String::new() } else { match meta::check_key_value(&b.key_value) { Ok(k) => k, Err(e) => return bad(&e) } };
    let people = match resolve_people(&s, &b.authors).await { Ok(p) => p, Err(e) => return bad(&e) };
    // 空白 on a journal template: the template's class and front matter with the usual
    // empty sections, never the sample's showcase text (same start as the 討論草稿 route)
    let blank: Vec<OutlineSection>;
    let outline = if !b.outline.is_empty() {
        Some(b.outline.as_slice())
    } else if b.template.as_deref().is_some_and(|t| !t.is_empty()) {
        blank = ["Introduction", "Related work", "Method", "Result", "Discussion", "Conclusion"].iter().map(|t| OutlineSection { title: (*t).into(), ..Default::default() }).collect();
        Some(blank.as_slice())
    } else {
        None
    };
    if let Some(r) = name_taken_outside(&b.name) {
        return r;
    }
    let made = match b.template.as_deref().filter(|t| !t.is_empty()) {
        Some(t) if !paper::safe_name(t) => return bad("invalid template"),
        Some(t) => paper::create_paper_from_template_with(&ws.join("papers"), &b.name, &ws.join("publishers").join(t), outline),
        None => paper::create_paper_with(&ws.join("papers"), &b.name, outline),
    };
    let dir = match made { Ok(d) => d, Err(e) => return bad(&e) };
    if let Err(e) = finish_paper(&dir, &key_value, b.title.as_deref(), &b.authors, people).and_then(|_| add_tables(&dir, b.template.as_deref(), &b.tables)) {
        let _ = std::fs::remove_dir_all(&dir);
        return bad(&e);
    }
    if let Some(t) = b.template.as_deref().filter(|t| !t.is_empty()) {
        let mut m = meta::read_meta(&dir);
        m.journal = Some(t.to_string());
        let _ = meta::write_meta(&dir, &m);
    }
    Json(serde_json::json!({ "name": b.name.trim() })).into_response()
}

#[derive(Deserialize)]
struct MetaEdit {
    key_value: String,
}

/// Change a paper's key value.
async fn put_meta(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<MetaEdit>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let key_value = match meta::check_key_value(&b.key_value) { Ok(k) => k, Err(e) => return bad(&e) };
    let mut m = meta::read_meta(&dir);
    m.key_value = key_value;
    match meta::write_meta(&dir, &m) {
        Ok(()) => Json(m).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// The people and institutions in the profile, for picking authors.
async fn people(State(s): State<Shared>) -> impl IntoResponse {
    let l = s.loaded.read().await;
    let (a, f) = l.profile.as_ref().map(|p| (p.authors.clone(), p.affiliations.clone())).unwrap_or_default();
    Json(serde_json::json!({ "authors": a, "affiliations": f }))
}

#[derive(Deserialize)]
struct NewPerson {
    author: Author,
    /// A new institution for this person, added to affiliations.csv first.
    #[serde(default)]
    affiliation: Option<Affiliation>,
}

/// Next free id with a prefix in a table's `id` column: A001, F012, ...
fn next_id(t: &Table, prefix: &str) -> String {
    let col = t.headers.iter().position(|h| h == "id").unwrap_or(0);
    let max = t.rows.iter().filter_map(|r| r.get(col)?.strip_prefix(prefix)?.parse::<u32>().ok()).max().unwrap_or(0);
    format!("{prefix}{:03}", max + 1)
}

fn put_row(t: &mut Table, values: &[(&str, String)]) {
    let row = t.headers.iter().map(|h| values.iter().find(|(k, _)| k == h).map(|(_, v)| v.clone()).unwrap_or_default()).collect();
    t.rows.push(row);
}

/// Add a person (and optionally a new institution) to the profile.
async fn add_person(State(s): State<Shared>, Json(b): Json<NewPerson>) -> impl IntoResponse {
    let dir = s.loaded.read().await.path.clone();
    let mut a = b.author;
    if a.family.trim().is_empty() {
        return bad("請填姓（family name）");
    }
    let mut new_affil = None;
    if let Some(f) = b.affiliation {
        if f.organization.trim().is_empty() {
            return bad("請填機構名稱");
        }
        let mut t = match table::read_table(&dir, "affiliations.csv") { Ok(t) => t, Err(e) => return bad(&e) };
        let id = next_id(&t, "F");
        put_row(&mut t, &[("id", id.clone()), ("department", f.department), ("organization", f.organization), ("city", f.city), ("country", f.country)]);
        if let Err(e) = table::write_table(&dir, "affiliations.csv", &t) { return bad(&e) }
        a.affiliations.push(id.clone());
        new_affil = Some(id);
    }
    if a.affiliations.is_empty() {
        return bad("請選擇或新增一個機構");
    }
    let mut t = match table::read_table(&dir, "authors.csv") { Ok(t) => t, Err(e) => return bad(&e) };
    let id = next_id(&t, "A");
    put_row(&mut t, &[("id", id.clone()), ("given", a.given), ("family", a.family), ("email", a.email), ("orcid", a.orcid), ("affiliations", a.affiliations.join(";")), ("role", a.role), ("note", a.note)]);
    if let Err(e) = table::write_table(&dir, "authors.csv", &t) { return bad(&e) }
    *s.loaded.write().await = load(&s.paths);
    Json(serde_json::json!({ "id": id, "affiliation": new_affil })).into_response()
}

#[derive(Deserialize)]
struct OutlineBody {
    #[serde(default)]
    template: Option<String>,
    outline: String,
    key_value: String,
    engine: EngineChoice,
}

/// Wizard: titles and a section skeleton from the outline (Claude, then the local model).
async fn wizard_outline(State(s): State<Shared>, Json(b): Json<OutlineBody>) -> impl IntoResponse {
    let key_value = match meta::check_key_value(&b.key_value) { Ok(k) => k, Err(e) => return bad(&e) };
    if b.outline.trim().is_empty() {
        return bad("請輸入大綱");
    }
    let (ws, language) = {
        let l = s.loaded.read().await;
        (l.workspace.clone(), l.profile.as_ref().map(|p| p.manifest.language.clone()).unwrap_or_else(|| "en".into()))
    };
    let sections = match (ws, b.template.as_deref().filter(|t| paper::safe_name(t))) {
        (Some(ws), Some(t)) => std::fs::read_to_string(ws.join("publishers").join(t).join("sample.tex")).map(|x| wizard::sample_sections(&x)).unwrap_or_default(),
        _ => vec![],
    };
    let prompt = wizard::outline_prompt(&b.outline, &key_value, &sections, &language);
    match draft::claude_then_local(&s, "pc-outline", b.engine, &prompt, &wizard::plan_schema(), wizard::parse_plan).await {
        Ok(plan) => Json(plan).into_response(),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct CaptionBody {
    /// "table" or "figure".
    kind: String,
    /// Table cells as pasted.
    #[serde(default)]
    data: String,
    /// Figure file name.
    #[serde(default)]
    file: String,
    #[serde(default)]
    section: String,
    #[serde(default)]
    key_value: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    engine: Option<EngineChoice>,
}

/// A caption for a table or figure the wizard is about to place.
async fn wizard_caption(State(s): State<Shared>, Json(b): Json<CaptionBody>) -> impl IntoResponse {
    if !has_writing_engine(&s, b.engine).await {
        return bad("自動產生標題需要寫作引擎");
    }
    let language = s.loaded.read().await.profile.as_ref().map(|p| p.manifest.language.clone()).unwrap_or_else(|| "en".into());
    let grid = pc_core::textable::parse_grid(&b.data);
    let prompt = wizard::caption_prompt(&b.kind, &grid, &b.file, &b.section, &b.key_value, &b.title, &language);
    match writing_complete(&s, "pc-caption", b.engine, &prompt, None, None).await {
        // empty: nothing to say from what was given (a figure named img1.png)
        Ok(reply) => Json(serde_json::json!({ "caption": wizard::clean_caption(&reply).unwrap_or_default() })).into_response(),
        Err(e) => bad(&e),
    }
}

/// The workspace's own settings (config.toml): look and choices the UI
/// remembers, kept with the user's data so every device opens the same way.
async fn get_settings(State(s): State<Shared>) -> impl IntoResponse {
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return Json(serde_json::json!({})).into_response() };
    Json(settings::read(&ws)).into_response()
}

async fn put_settings(State(s): State<Shared>, Json(b): Json<serde_json::Map<String, serde_json::Value>>) -> impl IntoResponse {
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return bad("no workspace") };
    match settings::write(&ws, &b) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ExtractBody {
    text: String,
    engine: EngineChoice,
}

/// Split pasted text into the papers it lists (Claude, then the local model).
async fn extract_refs(State(s): State<Shared>, Json(b): Json<ExtractBody>) -> impl IntoResponse {
    if b.text.trim().is_empty() {
        return bad("請貼上想引用的文章");
    }
    let prompt = citecheck::extract_prompt(&b.text);
    match draft::claude_then_local(&s, "pc-cite-extract", b.engine, &prompt, &citecheck::extract_schema(), citecheck::parse_extract).await {
        Ok(papers) => Json(serde_json::json!({ "papers": papers })).into_response(),
        Err(e) => bad(&e),
    }
}

/// Is this paper real? Crossref decides; a paper already in the library says so.
async fn verify_ref(State(s): State<Shared>, Json(g): Json<citecheck::Guess>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let g = citecheck::clean_guess(g);
    let v = match citecheck::verify(&g).await { Ok(v) => v, Err(e) => return bad(&e) };
    let (entries, _) = load_library(&ws);
    let exists = v.work.as_ref().and_then(|w| library_has(&entries, w)).map(|e| e.key.clone());
    Json(serde_json::json!({ "verdict": v, "exists": exists })).into_response()
}

/// The library entry for a work: same DOI, or same title and year.
fn library_has<'a>(entries: &'a [bib::Entry], w: &citecheck::Work) -> Option<&'a bib::Entry> {
    entries.iter().find(|e| {
        (!w.doi.is_empty() && e.get("doi").is_some_and(|d| d.eq_ignore_ascii_case(&w.doi)))
            || (citecheck::title_similarity(&e.title(), &w.title) > 0.95 && e.year() == w.year)
    })
}

#[derive(Deserialize)]
struct ProposeRecord {
    /// A database record (PubMed, arXiv) ...
    #[serde(default)]
    work: Option<citecheck::Work>,
    /// ... or the user's own entry, when no database has the paper.
    #[serde(default)]
    guess: Option<citecheck::Guess>,
    #[serde(default)]
    engine: Option<EngineChoice>,
}

/// A library entry for a work with no DOI record to fetch: from PubMed's
/// record, or from what the user typed (marked so in the entry).
async fn propose_record(State(s): State<Shared>, Json(b): Json<ProposeRecord>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let Some(w) = b.work.or_else(|| b.guess.map(|g| citecheck::from_text(&citecheck::clean_guess(g)))) else {
        return bad("缺少書目");
    };
    if w.title.trim().is_empty() || w.names.is_empty() || w.year.trim().is_empty() {
        return Json(serde_json::json!({ "error": "書目不完整：至少要有標題、作者與年份" })).into_response();
    }
    let (entries, _) = load_library(&ws);
    if let Some(e) = library_has(&entries, &w) {
        return Json(serde_json::json!({ "exists": e.key })).into_response();
    }
    let mut entry = bib::Entry { kind: "article".into(), key: String::new(), fields: citecheck::entry_fields(&w) };
    let taken: Vec<String> = entries.iter().map(|e| e.key.clone()).collect();
    entry.key = refimport::make_key(&entry, &taken);
    let (summary, one_liner, tags, note_error) = ref_notes(&s, b.engine, &entry, &w.venue, "article", "").await;
    Json(serde_json::json!({
        "key": entry.key, "bibtex": bib::write_entry(&entry), "title": entry.title(), "authors": entry.authors_short(),
        "year": entry.year(), "venue": w.venue, "doi": w.doi, "summary": summary, "one_liner": one_liner,
        "tags": tags, "note_error": note_error, "source": w.source,
    }))
    .into_response()
}

/// Summary, one-liner and tags for a library entry, from the model (venue
/// tags from the profile's list), and why the notes are missing if they are.
async fn ref_notes(s: &Shared, engine: Option<EngineChoice>, entry: &bib::Entry, venue: &str, kind: &str, abstract_text: &str) -> (String, String, Vec<String>, Option<String>) {
    let profile = s.loaded.read().await.path.clone();
    let tag_list = bib::read_tags(&profile.join("tags.csv"));
    let mut tags = refimport::venue_tags(&profile, venue);
    if kind == "review" && !tags.contains(&"review".to_string()) {
        tags.push("review".into());
    }
    let (mut summary, mut one_liner, mut note_error) = (String::new(), String::new(), None);
    if has_writing_engine(s, engine).await {
        let prompt = refimport::notes_prompt(&entry.title(), venue, &entry.year(), kind, abstract_text, &tag_list);
        match writing_complete(s, "pc-ref-notes", engine, &prompt, None, Some(refimport::notes_schema())).await {
            Ok(reply) => match refimport::parse_notes(&reply, &tag_list) {
                Some(n) => {
                    summary = n.summary;
                    one_liner = n.one_liner;
                    for t in n.tags {
                        if !tags.contains(&t) {
                            tags.push(t);
                        }
                    }
                }
                None => note_error = Some("模型的回覆無法讀取".to_string()),
            },
            Err(e) => note_error = Some(e),
        }
    }
    (summary, one_liner, tags, note_error)
}

#[derive(Deserialize)]
struct ImportQuery {
    name: String,
    #[serde(default)]
    key_value: String,
    /// Also tidy the import (see `paper::tidy_plan`); otherwise it is only offered.
    #[serde(default)]
    tidy: Option<u8>,
}

/// Import an uploaded LaTeX project (.zip, .tar.gz or .tex as the raw body).
async fn import_upload(State(s): State<Shared>, AxQuery(q): AxQuery<ImportQuery>, body: axum::body::Bytes) -> impl IntoResponse {
    import_bytes(&s, &q.name, &q.key_value, &body, q.tidy.is_some_and(|t| t != 0)).await
}

#[derive(Deserialize)]
struct ArxivImport {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    key_value: String,
    #[serde(default)]
    tidy: bool,
}

/// Import a paper's LaTeX source from arXiv by id or URL.
async fn import_arxiv(State(s): State<Shared>, Json(b): Json<ArxivImport>) -> impl IntoResponse {
    if let Err(e) = meta::check_key_value(&b.key_value) {
        return bad(&e);
    }
    let Some(id) = import::arxiv_id(&b.id) else { return bad("看不懂這個 arXiv 編號") };
    let bytes = match import::fetch_arxiv(&id).await { Ok(b) => b, Err(e) => return bad(&e) };
    let name = b.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("arXiv {}", id.replace('/', "-")));
    import_bytes(&s, &name, &b.key_value, &bytes, b.tidy).await
}

/// Import, recognise the journal template, and either tidy the paper or say
/// what tidying and merging its .bib into the library would do.
async fn import_bytes(s: &Shared, name: &str, key_value: &str, bytes: &[u8], tidy: bool) -> axum::response::Response {
    let key_value = match meta::check_key_value(key_value) { Ok(k) => k, Err(e) => return bad(&e) };
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return bad("no workspace") };
    if let Some(r) = name_taken_outside(name) {
        return r;
    }
    let profile_authors = s.loaded.read().await.profile.as_ref().map(|p| p.authors.clone()).unwrap_or_default();
    let tmp = std::env::temp_dir().join(format!("papercomposer-import-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    let res = std::fs::create_dir_all(&tmp)
        .map_err(|e| e.to_string())
        .and_then(|_| import::unpack(bytes, &tmp))
        .and_then(|_| paper::import_paper(&ws.join("papers"), name, &tmp));
    let _ = std::fs::remove_dir_all(&tmp);
    let res = res.and_then(|dir| {
        let journal = pc_core::journal::detect(&ws.join("publishers"), &dir);
        // authors named in the paper's own \author block who are in the profile (none guessed)
        let main = std::fs::read_to_string(pc_core::open::main_file(&dir)).unwrap_or_default();
        let authors = pc_core::authors::match_block(&main, &profile_authors);
        meta::write_meta(&dir, &PaperMeta { key_value, journal: journal.clone(), authors: authors.clone(), ..Default::default() }).map(|_| (dir, journal, authors))
    });
    let (dir, journal, authors) = match res {
        Ok(x) => x,
        Err(e) => return bad(&e),
    };
    let journal_name = journal.as_deref().and_then(|j| pc_core::journal::load(&ws.join("publishers"), j)).map(|j| j.meta.name).filter(|n| !n.is_empty());
    let author_names: Vec<String> = authors.iter().filter_map(|a| profile_authors.iter().find(|p| p.id == a.id)).map(|p| p.name()).collect();
    let plan = paper::tidy_plan(&dir);
    // Tidying is offered (`tidy`) unless it was asked for (`tidied`).
    let tidied = tidy.then(|| paper::apply_tidy(&dir, &plan));
    let bib = merge_paper_bib(&ws, &dir, true).await.ok();
    Json(serde_json::json!({
        "name": name.trim(),
        "journal": journal,
        "journal_name": journal_name,
        "authors": author_names,
        "tidied": tidied.as_ref().and_then(|r| r.as_ref().ok()),
        "tidy_error": tidied.as_ref().and_then(|r| r.as_ref().err()),
        "tidy": (!tidy && !plan.is_empty()).then_some(&plan),
        "bib": bib,
    }))
    .into_response()
}

/// What the import card shows for a paper already in the workspace: the
/// recognised journal, the tidy plan, and a dry run of merging its .bib.
async fn import_summary(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return bad("no workspace") };
    let m = meta::read_meta(&dir);
    let journal_name = m.journal.as_deref().and_then(|j| pc_core::journal::load(&ws.join("publishers"), j)).map(|j| j.meta.name).filter(|n| !n.is_empty());
    let plan = paper::tidy_plan(&dir);
    let bib = merge_paper_bib(&ws, &dir, true).await.ok();
    Json(serde_json::json!({
        "name": name,
        "journal": m.journal,
        "journal_name": journal_name,
        "tidy": (!plan.is_empty()).then_some(&plan),
        "bib": bib,
    }))
    .into_response()
}

/// GET: what tidying the paper would do. POST: do it.
async fn get_tidy(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    Json(paper::tidy_plan(&dir)).into_response()
}

async fn post_tidy(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    match paper::apply_tidy(&dir, &paper::tidy_plan(&dir)) {
        Ok(done) => Json(done).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize, Default)]
struct MergeBody {
    #[serde(default)]
    dry_run: bool,
}

/// Copy the entries of the paper's own .bib files into refs/library.bib;
/// an entry whose key or DOI the library already has is skipped.
async fn merge_bib(State(s): State<Shared>, AxPath(name): AxPath<String>, body: Option<Json<MergeBody>>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let dry_run = body.is_some_and(|Json(b)| b.dry_run);
    match merge_paper_bib(&ws, &dir, dry_run).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Serialize)]
struct MergeReport {
    /// The paper's own .bib files that were read.
    files: Vec<String>,
    added: Vec<String>,
    /// Already in the library (same key or same DOI).
    skipped: Vec<String>,
    errors: Vec<String>,
    dry_run: bool,
}

async fn merge_paper_bib(ws: &Path, dir: &Path, dry_run: bool) -> Result<MergeReport, String> {
    let main = std::fs::read_to_string(pc_core::open::main_file(&dir)).unwrap_or_default();
    let mut files: Vec<String> = bib_files(&main).into_iter().filter(|f| f != PC_BIB).collect();
    if files.is_empty() {
        files = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".bib")).collect();
        files.sort();
    }
    let mut new = Vec::new();
    let mut errors = Vec::new();
    let mut read = Vec::new();
    for f in files {
        let Ok(bytes) = std::fs::read(dir.join(&f)) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        if text.starts_with(BIB_MARK) {
            continue; // written by the app from the library
        }
        let (entries, errs) = bib::parse(&text);
        new.extend(entries);
        errors.extend(errs);
        read.push(f);
    }
    let _library = LIBRARY_LOCK.lock().await;
    let (mut entries, mut notes) = load_library(ws);
    let (added, skipped) = add_to_library(&mut entries, &mut notes, new);
    if !dry_run && !added.is_empty() {
        save_library(ws, &entries, &notes)?;
    }
    Ok(MergeReport { files: read, added, skipped, errors, dry_run })
}

/// Append entries the library does not have (by key or DOI); returns the
/// keys added and the keys skipped.
fn add_to_library(entries: &mut Vec<bib::Entry>, notes: &mut Vec<bib::Note>, new: Vec<bib::Entry>) -> (Vec<String>, Vec<String>) {
    let (mut added, mut skipped) = (Vec::new(), Vec::new());
    for e in new {
        let doi = e.get("doi").unwrap_or("").trim().to_lowercase();
        let dup = entries.iter().any(|x| x.key == e.key || (!doi.is_empty() && x.get("doi").is_some_and(|d| d.trim().to_lowercase() == doi)));
        if dup {
            skipped.push(e.key.clone());
            continue;
        }
        notes.push(bib::Note { key: e.key.clone(), doi: e.get("doi").unwrap_or("").into(), ..Default::default() });
        added.push(e.key.clone());
        entries.push(e);
    }
    (added, skipped)
}

#[derive(Deserialize)]
struct Packages {
    packages: Vec<String>,
}

/// Load the packages a table style needs in this paper's main.tex.
async fn paper_packages(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<Packages>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    if b.packages.iter().any(|p| !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
        return bad("invalid package name");
    }
    match paper::ensure_packages(&dir, &b.packages) {
        Ok(added) => Json(serde_json::json!({ "added": added })).into_response(),
        Err(e) => bad(&e),
    }
}

// ── References ─────────────────────────────────────────

fn pdf_dir() -> Option<PathBuf> {
    std::env::var("PAPERCOMPOSER_PDF_DIR").ok().map(PathBuf::from).or_else(|| read_config().pdf_dir).filter(|p| p.is_dir())
}

async fn refs_root(s: &Shared) -> Result<PathBuf, axum::response::Response> {
    s.loaded.read().await.workspace.clone().ok_or_else(|| bad("no workspace"))
}

fn load_library(ws: &Path) -> (Vec<bib::Entry>, Vec<bib::Note>) {
    let src = std::fs::read_to_string(ws.join("refs/library.bib")).unwrap_or_default();
    (bib::parse(&src).0, bib::read_notes(&ws.join("refs/notes.csv")))
}

/// Handlers that change the library hold this across their read and write, so
/// papers filed at the same time do not overwrite each other.
static LIBRARY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn save_library(ws: &Path, entries: &[bib::Entry], notes: &[bib::Note]) -> Result<(), String> {
    std::fs::create_dir_all(ws.join("refs")).map_err(|e| e.to_string())?;
    paper::write_atomic(&ws.join("refs/library.bib"), &bib::write(entries)).map_err(|e| e.to_string())?;
    bib::write_notes(&ws.join("refs/notes.csv"), notes).map_err(|e| e.to_string())
}

/// Every reference with its notes and whether this machine has its PDF.
async fn list_refs(State(s): State<Shared>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let (entries, notes) = load_library(&ws);
    let dir = pdf_dir();
    let rows: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            let n = notes.iter().find(|n| n.key == e.key).cloned().unwrap_or_else(|| bib::Note { key: e.key.clone(), doi: e.get("doi").unwrap_or("").into(), ..Default::default() });
            let found = dir.as_deref().and_then(|d| bib::find_pdf(d, &n)).is_some();
            serde_json::json!({
                "key": e.key, "kind": e.kind, "title": e.title(), "authors": e.authors_short(),
                "year": e.year(), "venue": e.venue(), "doi": n.doi, "summary": n.summary,
                "one_liner": n.one_liner, "tags": n.tags, "pdf": n.pdf, "pdf_found": found,
            })
        })
        .collect();
    Json(serde_json::json!({ "refs": rows, "pdf_dir": dir })).into_response()
}

async fn ref_tags(State(s): State<Shared>) -> impl IntoResponse {
    let path = s.loaded.read().await.path.join("tags.csv");
    Json(bib::read_tags(&path))
}

#[derive(Deserialize)]
struct ImportBib {
    bibtex: String,
}

/// Add BibTeX entries to the library; existing keys or DOIs are skipped.
async fn import_refs(State(s): State<Shared>, Json(b): Json<ImportBib>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let _library = LIBRARY_LOCK.lock().await;
    let (mut entries, mut notes) = load_library(&ws);
    let (new, errors) = bib::parse(&b.bibtex);
    let (added, skipped) = add_to_library(&mut entries, &mut notes, new);
    if let Err(e) = save_library(&ws, &entries, &notes) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    Json(serde_json::json!({ "added": added, "skipped": skipped, "errors": errors })).into_response()
}

#[derive(Deserialize)]
struct NoteBody {
    summary: Option<String>,
    one_liner: Option<String>,
    tags: Option<Vec<String>>,
    pdf: Option<String>,
}

async fn update_ref(State(s): State<Shared>, AxPath(key): AxPath<String>, Json(b): Json<NoteBody>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let _library = LIBRARY_LOCK.lock().await;
    let (entries, mut notes) = load_library(&ws);
    let Some(e) = entries.iter().find(|e| e.key == key) else { return (StatusCode::NOT_FOUND, "no such reference").into_response() };
    if !notes.iter().any(|n| n.key == key) {
        notes.push(bib::Note { key: key.clone(), doi: e.get("doi").unwrap_or("").into(), ..Default::default() });
    }
    let n = notes.iter_mut().find(|n| n.key == key).unwrap();
    if let Some(v) = b.summary { n.summary = v; }
    if let Some(v) = b.one_liner { n.one_liner = v; }
    if let Some(v) = b.tags { n.tags = v; }
    if let Some(v) = b.pdf { n.pdf = v; }
    match save_library(&ws, &entries, &notes) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Open a reference's PDF with the system viewer (only files in the PDF folder).
async fn open_ref_pdf(State(s): State<Shared>, AxPath(key): AxPath<String>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let (_, notes) = load_library(&ws);
    let Some(dir) = pdf_dir() else { return bad("還沒設定 PDF 資料夾") };
    let note = notes.into_iter().find(|n| n.key == key).unwrap_or(bib::Note { key: key.clone(), ..Default::default() });
    match bib::find_pdf(&dir, &note) {
        Some(p) => match proc::open_in_browser(&p.to_string_lossy()) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
        None => (StatusCode::NOT_FOUND, "本機找不到這篇的 PDF").into_response(),
    }
}

/// PDFs in this machine's folder, and which library entry each belongs to.
async fn list_pdfs(State(s): State<Shared>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let Some(dir) = pdf_dir() else { return Json(serde_json::json!({ "dir": null, "files": [] })).into_response() };
    let (_, notes) = load_library(&ws);
    let linked: Vec<(String, PathBuf)> = notes.iter().filter_map(|n| Some((n.key.clone(), bib::find_pdf(&dir, n)?))).collect();
    let mut files: Vec<serde_json::Value> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pdf")))
        .map(|p| {
            let key = linked.iter().find(|(_, lp)| lp == &p).map(|(k, _)| k.clone());
            serde_json::json!({ "name": p.file_name().unwrap().to_string_lossy(), "key": key })
        })
        .collect();
    files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Json(serde_json::json!({ "dir": dir, "files": files })).into_response()
}

#[derive(Deserialize)]
struct Propose {
    #[serde(default)]
    pdf: Option<String>,
    #[serde(default)]
    doi: Option<String>,
    #[serde(default)]
    engine: Option<EngineChoice>,
}

/// A library entry for a PDF or DOI, for the user to confirm: BibTeX from
/// Crossref, abstract and type from OpenAlex, notes and tags from the model.
async fn propose_ref(State(s): State<Shared>, Json(b): Json<Propose>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let pdf = b.pdf.filter(|n| paper::safe_name(n) && n.to_lowercase().ends_with(".pdf"));
    let mut doi = b.doi.map(|d| d.trim().trim_start_matches("https://doi.org/").trim_start_matches("doi:").to_string()).filter(|d| !d.is_empty());
    let mut head = String::new();
    if let (Some(name), Some(dir)) = (&pdf, pdf_dir()) {
        let path = dir.join(name);
        let (h, d) = tokio::task::spawn_blocking(move || {
            let h = refimport::pdf_head(&path);
            let d = refimport::pdf_doi(&path, &h);
            (h, d)
        })
        .await
        .unwrap_or_default();
        head = h;
        if doi.is_none() {
            doi = d;
        }
    }
    let Some(doi) = doi else {
        return Json(serde_json::json!({ "need_doi": true, "pdf": pdf })).into_response();
    };
    let (entries, _) = load_library(&ws);
    if let Some(e) = entries.iter().find(|e| e.get("doi").is_some_and(|d| d.eq_ignore_ascii_case(&doi))) {
        return Json(serde_json::json!({ "exists": e.key, "doi": doi, "pdf": pdf })).into_response();
    }
    let bibtex = match refimport::crossref_bibtex(&doi).await {
        Ok(t) => t,
        Err(e) => return Json(serde_json::json!({ "error": e, "doi": doi, "pdf": pdf })).into_response(),
    };
    let Some(mut entry) = bib::parse(&bibtex).0.into_iter().next() else {
        return Json(serde_json::json!({ "error": "doi.org 回傳的 BibTeX 無法讀取", "doi": doi, "pdf": pdf })).into_response();
    };
    let taken: Vec<String> = entries.iter().map(|e| e.key.clone()).collect();
    entry.key = refimport::make_key(&entry, &taken);
    entry.fields.entry("doi".into()).or_insert_with(|| doi.clone());
    let mut work = refimport::openalex(&doi).await;
    if work.abstract_text.is_empty() {
        work.abstract_text = refimport::semantic_scholar_abstract(&doi).await;
    }
    if work.abstract_text.is_empty() {
        work.abstract_text = refimport::pdf_abstract(&head);
    }
    let venue = if entry.venue().is_empty() { work.venue.clone() } else { entry.venue() };
    let profile = s.loaded.read().await.path.clone();
    let tag_list = bib::read_tags(&profile.join("tags.csv"));
    let mut tags = refimport::venue_tags(&profile, &venue);
    if work.kind == "review" && !tags.contains(&"review".to_string()) {
        tags.push("review".into());
    }
    let (mut summary, mut one_liner, mut note_error) = (String::new(), String::new(), None);
    if has_writing_engine(&s, b.engine).await {
        let prompt = refimport::notes_prompt(&entry.title(), &venue, &entry.year(), &work.kind, &work.abstract_text, &tag_list);
        match writing_complete(&s, "pc-ref-notes", b.engine, &prompt, None, Some(refimport::notes_schema())).await {
            Ok(reply) => match refimport::parse_notes(&reply, &tag_list) {
                Some(n) => {
                    summary = n.summary;
                    one_liner = n.one_liner;
                    for t in n.tags {
                        if !tags.contains(&t) { tags.push(t); }
                    }
                }
                None => note_error = Some("模型的回覆無法讀取".to_string()),
            },
            Err(e) => note_error = Some(e),
        }
    }
    Json(serde_json::json!({
        "key": entry.key, "bibtex": bib::write_entry(&entry), "title": entry.title(), "authors": entry.authors_short(),
        "year": entry.year(), "venue": venue, "doi": doi, "pdf": pdf, "abstract": work.abstract_text,
        "preprint_only": work.venue_type == "repository", "summary": summary, "one_liner": one_liner,
        "title_mismatch": !refimport::title_matches(&entry.title(), &head),
        "tags": tags, "note_error": note_error,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SearchBody {
    request: String,
    #[serde(default)]
    tags: Vec<String>,
    /// "subscription" or "api": search is a Claude feature.
    engine: EngineChoice,
}

/// Find papers: Claude (or the local model) writes the queries, OpenAlex
/// returns real works (all with DOIs, no preprint-only ones), then the model ranks them and says why.
async fn search_refs(State(s): State<Shared>, Json(b): Json<SearchBody>) -> impl IntoResponse {
    if b.request.trim().is_empty() {
        return bad("寫下要找什麼");
    }
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let profile = s.loaded.read().await.path.clone();
    let tag_list = bib::read_tags(&profile.join("tags.csv"));
    let labels: Vec<String> = b.tags.iter().map(|t| tag_list.iter().find(|x| &x.id == t).map(|x| format!("{} ({})", x.label, x.id)).unwrap_or(t.clone())).collect();
    let steer = refimport::Steer::from_tags(&b.tags);

    let queries = match draft::claude_then_local(&s, "pc-ref-queries", b.engine, &refimport::queries_prompt(&b.request, &labels), &refimport::queries_schema(), |r| {
        Some(refimport::parse_queries(r)).filter(|q| !q.is_empty()).ok_or_else(|| "no queries".to_string())
    })
    .await
    {
        Ok(q) => q,
        Err(e) => return Json(serde_json::json!({ "error": e })).into_response(),
    };
    let this_year: u32 = {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        (1970 + secs / 31_556_952) as u32
    };
    let mut cands: Vec<refimport::Candidate> = Vec::new();
    let mut last_err = None;
    for q in &queries {
        match refimport::openalex_search(q, &steer, this_year).await {
            Ok(list) => {
                for c in list {
                    if !cands.iter().any(|x| x.doi.eq_ignore_ascii_case(&c.doi)) {
                        cands.push(c);
                    }
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    if cands.is_empty() {
        return Json(serde_json::json!({ "queries": queries, "results": [], "error": last_err })).into_response();
    }
    // Venue-based steering after retrieval.
    let wants_top: Vec<&str> = b.tags.iter().map(String::as_str).filter(|t| *t == "top_conference" || *t == "top_journal").collect();
    if !wants_top.is_empty() {
        let top: Vec<_> = cands.iter().filter(|c| refimport::venue_tags(&profile, &c.venue).iter().any(|t| wants_top.contains(&t.as_str()))).cloned().collect();
        if !top.is_empty() {
            cands = top;
        }
    }
    cands.truncate(30);
    let ranked = match draft::claude_then_local(&s, "pc-ref-rank", b.engine, &refimport::rank_prompt(&b.request, &cands), &refimport::rank_schema(), |r| {
        refimport::apply_ranking(r, &cands).ok_or_else(|| "no ranking".to_string())
    })
    .await
    {
        Ok(r) => r,
        Err(_) => cands.iter().take(8).cloned().collect(),
    };
    let (entries, _) = load_library(&ws);
    let results: Vec<serde_json::Value> = ranked
        .into_iter()
        .map(|c| {
            let have = entries.iter().find(|e| e.get("doi").is_some_and(|d| d.eq_ignore_ascii_case(&c.doi))).map(|e| e.key.clone());
            let mut v = serde_json::to_value(&c).unwrap_or_default();
            v["in_library"] = serde_json::json!(have);
            v
        })
        .collect();
    Json(serde_json::json!({ "queries": queries, "results": results })).into_response()
}

#[derive(Deserialize)]
struct OpenDoi {
    #[serde(default)]
    doi: Option<String>,
    /// An open-access PDF link returned by the search.
    #[serde(default)]
    url: Option<String>,
}

/// Open a DOI landing page, or an open-access https link, in the browser.
async fn open_doi(Json(b): Json<OpenDoi>) -> impl IntoResponse {
    let url = match (b.doi, b.url) {
        (Some(d), _) if d.starts_with("10.") && !d.contains(char::is_whitespace) => format!("https://doi.org/{d}"),
        (_, Some(u)) if u.starts_with("https://") && !u.contains(char::is_whitespace) => u,
        _ => return bad("invalid link"),
    };
    match proc::open_in_browser(&url) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct AddRef {
    bibtex: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    one_liner: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    pdf: Option<String>,
}

async fn add_ref(State(s): State<Shared>, Json(b): Json<AddRef>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let Some(mut e) = bib::parse(&b.bibtex).0.into_iter().next() else { return bad("BibTeX 無法讀取") };
    let _library = LIBRARY_LOCK.lock().await; // one read-modify-write of the library at a time
    let (mut entries, mut notes) = load_library(&ws);
    let doi = e.get("doi").unwrap_or("").to_string();
    if !doi.is_empty() && entries.iter().any(|x| x.get("doi").is_some_and(|d| d.eq_ignore_ascii_case(&doi))) {
        return bad("文獻庫已經有這篇");
    }
    // Another paper filed meanwhile may have taken the key it was proposed with.
    if entries.iter().any(|x| x.key == e.key) {
        let taken: Vec<String> = entries.iter().map(|x| x.key.clone()).collect();
        e.key = refimport::make_key(&e, &taken);
    }
    notes.push(bib::Note { key: e.key.clone(), summary: b.summary, one_liner: b.one_liner, tags: b.tags, doi, pdf: b.pdf.unwrap_or_default() });
    let key = e.key.clone();
    entries.push(e);
    match save_library(&ws, &entries, &notes) {
        Ok(()) => Json(serde_json::json!({ "key": key })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct PdfDir {
    path: String,
}

async fn set_pdf_dir(Json(b): Json<PdfDir>) -> impl IntoResponse {
    let p = PathBuf::from(b.path.trim());
    if !p.is_dir() {
        return bad("資料夾不存在");
    }
    match write_config_root("pdf_dir", &p.to_string_lossy()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Citations used in one paper, and what each is used for there.
async fn paper_citations(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let tex: String = pc_core::open::source_files(&dir)
        .into_iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect::<Vec<_>>()
        .join("\n");
    // entries in the paper's own .bib files, for keys the library lacks
    let main = std::fs::read_to_string(pc_core::open::main_file(&dir)).unwrap_or_default();
    let local: Vec<serde_json::Value> = bib_files(&main)
        .iter()
        .filter_map(|f| std::fs::read(dir.join(f)).ok())
        .flat_map(|b| bib::parse(&String::from_utf8_lossy(&b)).0)
        .map(|e| {
            let g = |k: &str| e.get(k).unwrap_or("").replace(['{', '}'], "");
            let venue = format!("{}{}", g("journal"), g("booktitle"));
            serde_json::json!({ "key": e.key, "title": g("title"), "authors": g("author"), "year": g("year"), "venue": venue })
        })
        .collect();
    Json(serde_json::json!({ "cited": bib::cited_keys(&tex), "purposes": bib::read_citations(&paper::citations_file(&dir)), "local": local })).into_response()
}

#[derive(Deserialize)]
struct Purpose {
    key: String,
    purpose: String,
}

async fn set_purpose(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<Purpose>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let path = paper::citations_file(&dir);
    let mut c = bib::read_citations(&path);
    if b.purpose.trim().is_empty() { c.remove(&b.key); } else { c.insert(b.key, b.purpose.trim().to_string()); }
    match bib::write_citations(&path, &c) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

const BIB_MARK: &str = "% Generated by PaperComposer from refs/library.bib: edit the library, not this file.";

/// Make sure every reference the paper cites reaches BibTeX. The author's
/// own .bib files are never rewritten: when the paper has them, references
/// cited from the library that they lack go to `pc-refs.bib`, which is added
/// to `\\bibliography{...}`. A paper without its own .bib gets one file with
/// exactly what it cites.
async fn paper_bib(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    match write_paper_bib(&ws, &dir) {
        Ok(b) => Json(b).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Serialize)]
struct BibOut {
    /// Every .bib the paper uses, its own first.
    files: Vec<String>,
    /// Cited, but in neither the paper's .bib files nor the library.
    missing: Vec<String>,
}

const PC_BIB: &str = "pc-refs.bib";

fn write_paper_bib(ws: &Path, dir: &Path) -> Result<BibOut, String> {
    let main = std::fs::read_to_string(pc_core::open::main_file(&dir)).unwrap_or_default();
    let listed = bib_files(&main);
    let own: Vec<String> = listed
        .iter()
        .filter(|f| std::fs::read_to_string(dir.join(f)).is_ok_and(|t| !t.starts_with(BIB_MARK)))
        .cloned()
        .collect();
    let own_keys: Vec<String> = own
        .iter()
        .filter_map(|f| std::fs::read(dir.join(f)).ok())
        .flat_map(|b| bib::parse(&String::from_utf8_lossy(&b)).0.into_iter().map(|e| e.key))
        .collect();
    let tex: String = pc_core::open::source_files(dir)
        .into_iter()
        .skip(1)
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .chain(std::iter::once(main.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    let need: Vec<String> = bib::cited_keys(&tex).into_iter().filter(|k| !own_keys.contains(k)).collect();
    let (entries, _) = load_library(ws);
    let (body, missing) = bib::paper_bib(&entries, &need);
    let mut files = own.clone();
    let target = if own.is_empty() { listed.first().cloned().unwrap_or_else(|| "refs.bib".into()) } else { PC_BIB.into() };
    if own.is_empty() || !body.trim().is_empty() {
        paper::write_atomic(&dir.join(&target), &format!("{BIB_MARK}\n\n{body}")).map_err(|e| e.to_string())?;
        if !listed.contains(&target) && !own.is_empty() {
            add_bib_file(dir, &main, &target)?;
        }
        files.push(target);
    }
    Ok(BibOut { files, missing })
}

/// Name `file` in main.tex's \\bibliography{...} (or add an \\addbibresource).
fn add_bib_file(dir: &Path, main: &str, file: &str) -> Result<(), String> {
    let stem = file.trim_end_matches(".bib");
    let mut done = false;
    let lines: Vec<String> = main
        .lines()
        .map(|l| {
            let code = l.split('%').next().unwrap_or("");
            if !done {
                if let Some(i) = code.find("\\bibliography{") {
                    if let Some(j) = code[i..].find('}') {
                        done = true;
                        return format!("{},{stem}{}", &l[..i + j], &l[i + j..]);
                    }
                }
                if code.contains("\\addbibresource{") {
                    done = true;
                    return format!("{l}\n\\addbibresource{{{file}}}");
                }
            }
            l.to_string()
        })
        .collect();
    if !done {
        return Ok(());
    }
    paper::write_atomic(&pc_core::open::main_file(&dir), &(lines.join("\n") + "\n")).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct ExportBody {
    /// "pdf" or "docx"
    kind: String,
}

/// Compile to PDF, or convert to Word; the paper's .bib is brought up to date first.
async fn export_paper(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ExportBody>) -> impl IntoResponse {
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let main = std::fs::read_to_string(pc_core::open::main_file(&dir)).unwrap_or_default();
    // An imported paper may come with its bibliography already built (arXiv
    // often ships the .bbl, not the .bib): use it as is.
    let prebuilt = dir.join("main.bbl").is_file() && !bib_files(&main).iter().any(|f| dir.join(f).is_file());
    let bib = export::bib_tool(&main).filter(|_| !prebuilt).and_then(|_| write_paper_bib(&ws, &dir).ok());
    let res = match b.kind.as_str() {
        "pdf" => export::build_pdf(&dir, &name, prebuilt).await,
        "docx" => export::build_docx(&dir, &name, bib.as_ref().map(|b| b.files.as_slice()).unwrap_or_default()).await,
        // compile first: the package must build, and carries the fresh .bbl
        "latex" => match export::build_pdf(&dir, &name, prebuilt).await {
            Ok(mut r) if r.ok => match export::build_latex_zip(&dir, &name) {
                Ok(f) => {
                    r.file = Some(f);
                    Ok(r)
                }
                Err(e) => Err(e),
            },
            other => other,
        },
        _ => return bad("unknown export kind"),
    };
    match res {
        Ok(mut r) => {
            // Cited but nowhere to be found: report them as undefined citations.
            for k in bib.map(|b| b.missing).unwrap_or_default() {
                if !r.undefined_cites.contains(&k) {
                    r.undefined_cites.push(k);
                }
            }
            Json(r).into_response()
        }
        Err(e) => Json(serde_json::json!({ "ok": false, "errors": [{ "message": e }] })).into_response(),
    }
}

#[derive(Deserialize)]
struct OpenExport {
    kind: String,
    #[serde(default)]
    reveal: bool,
}

async fn open_export(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<OpenExport>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let f = dir.join("export").join(match b.kind.as_str() {
        "pdf" | "docx" => format!("{name}.{}", b.kind),
        "latex" => format!("{name}-latex.zip"),
        _ => return bad("unknown export kind"),
    });
    if !f.is_file() {
        return (StatusCode::NOT_FOUND, "還沒有匯出").into_response();
    }
    match export::open(&f, b.reveal) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// The .bib files main.tex uses: \\bibliography{a,b} or \\addbibresource{x.bib}.
fn bib_files(main: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in main.lines() {
        let code = line.split('%').next().unwrap_or("");
        for cmd in ["\\bibliography{", "\\addbibresource{"] {
            if let Some(i) = code.find(cmd) {
                let rest = &code[i + cmd.len()..];
                for name in rest.split('}').next().unwrap_or("").split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    let name = name.rsplit('/').next().unwrap_or(name);
                    let f = if name.ends_with(".bib") { name.to_string() } else { format!("{name}.bib") };
                    if !out.contains(&f) {
                        out.push(f);
                    }
                }
            }
        }
    }
    out
}

async fn get_paper(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    match paper_dir(&s, &name).await {
        Ok(dir) => {
            let main = std::fs::read_to_string(pc_core::open::main_file(&dir)).unwrap_or_default();
            let (front, back) = preview::front_back_in(&main, pc_core::open::is_custom(&dir));
            let m = meta::read_meta(&dir);
            // the journal's house rules: word limits for the word count,
            // reference words and caption places for inserting, page limits
            let ws = s.loaded.read().await.workspace.clone();
            let journal = m.journal.as_deref().zip(ws).and_then(|(j, ws)| pc_core::journal::load(&ws.join("publishers"), j));
            let ref_words = journal.as_ref().map(|j| j.effective_ref_words()).unwrap_or_default();
            let (word_limits, captions, limits) = journal.map(|j| (j.meta.word_limits, j.meta.captions, j.meta.limits)).unwrap_or_default();
            Json(serde_json::json!({
            "sections": pc_core::open::sections(&dir),
            "macros": paper::paper_macros(&dir),
            "numbers": preview::numbers(&dir, &export::cache_dir(&name)),
            "front": front,
            "back": back,
            "fields": pc_core::front::fields(&main),
            "bibliography": preview::bibliography(&dir, &export::cache_dir(&name)),
            "meta": m,
            "word_limits": word_limits,
            "ref_words": ref_words,
            "captions": captions,
            "limits": limits,
            // what is on disk now (edits made elsewhere change it), and where
            "stamp": pc_core::open::stamp(&dir),
            "path": opened::external_dir(&name).filter(|d| *d == dir),
        }))
        .into_response()
        }
        Err(r) => r,
    }
}

// ── Journal templates ──────────────────────────────────

async fn publishers_dir(s: &Shared) -> Result<PathBuf, axum::response::Response> {
    let ws = refs_root(s).await?;
    let d = ws.join("publishers");
    std::fs::create_dir_all(&d).map_err(|e| bad(&e.to_string()))?;
    Ok(d)
}

async fn list_templates(State(s): State<Shared>) -> impl IntoResponse {
    match publishers_dir(&s).await {
        Ok(d) => Json(pc_core::journal::list(&d)).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
struct NewJournal {
    #[serde(flatten)]
    meta: pc_core::journal::Meta,
    from: String,
}

/// A new journal template, copied from an existing one.
async fn create_template(State(s): State<Shared>, Json(b): Json<NewJournal>) -> impl IntoResponse {
    let d = match publishers_dir(&s).await { Ok(d) => d, Err(r) => return r };
    match pc_core::journal::create(&d, b.meta, &b.from) {
        Ok(id) => Json(pc_core::journal::load(&d, &id)).into_response(),
        Err(e) => bad(&e),
    }
}

async fn update_template(State(s): State<Shared>, AxPath(id): AxPath<String>, Json(b): Json<pc_core::journal::Update>) -> impl IntoResponse {
    let d = match publishers_dir(&s).await { Ok(d) => d, Err(r) => return r };
    match pc_core::journal::update(&d, &id, b) {
        Ok(j) => Json(j).into_response(),
        Err(e) => bad(&e),
    }
}

async fn get_template_sample(State(s): State<Shared>, AxPath(id): AxPath<String>) -> impl IntoResponse {
    let d = match publishers_dir(&s).await { Ok(d) => d, Err(r) => return r };
    if !paper::safe_name(&id) {
        return bad("invalid template");
    }
    match pc_core::journal::read_sample(&d.join(&id)) {
        Some(t) => t.into_response(),
        None => (StatusCode::NOT_FOUND, "這個模板沒有 sample.tex").into_response(),
    }
}

#[derive(Deserialize)]
struct SampleBody {
    text: String,
}

async fn put_template_sample(State(s): State<Shared>, AxPath(id): AxPath<String>, Json(b): Json<SampleBody>) -> impl IntoResponse {
    let d = match publishers_dir(&s).await { Ok(d) => d, Err(r) => return r };
    match pc_core::journal::write_sample(&d, &id, &b.text) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => bad(&e),
    }
}

/// Build a paper from the template in a scratch folder and compile it.
async fn check_template(State(s): State<Shared>, AxPath(id): AxPath<String>) -> impl IntoResponse {
    let d = match publishers_dir(&s).await { Ok(d) => d, Err(r) => return r };
    if !paper::safe_name(&id) || !d.join(&id).is_dir() {
        return bad("找不到模板");
    }
    let tmp = std::env::temp_dir().join(format!("papercomposer-tplcheck-{}-{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&tmp);
    let res = match paper::create_paper_from_template(&tmp, "check", &d.join(&id)) {
        Ok(dir) => export::build_pdf(&dir, &format!("tpl-{id}"), false).await,
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_dir_all(&tmp);
    match res {
        Ok(mut r) => {
            r.file = None;
            Json(r).into_response()
        }
        Err(e) => Json(serde_json::json!({ "ok": false, "errors": [{ "message": e }] })).into_response(),
    }
}

// ── Template conversion with Claude ────────────────────

#[derive(Deserialize)]
struct NewConversion {
    name: String,
    #[serde(default)]
    kind: String,
}

async fn convert_create(Json(b): Json<NewConversion>) -> impl IntoResponse {
    match convert::create(&b.name, &b.kind) {
        Ok(j) => Json(j).into_response(),
        Err(e) => bad(&e),
    }
}

async fn convert_get(AxPath(id): AxPath<String>) -> impl IntoResponse {
    let Some(job) = convert::load(&id) else { return (StatusCode::NOT_FOUND, "找不到這次轉換").into_response() };
    Json(serde_json::json!({ "job": job, "analysis": convert::analyze(&id), "files": convert::keepable(&id) })).into_response()
}

#[derive(Deserialize)]
struct UploadQuery {
    role: String,
    name: String,
}

async fn convert_upload(AxPath(id): AxPath<String>, AxQuery(q): AxQuery<UploadQuery>, body: axum::body::Bytes) -> impl IntoResponse {
    match convert::upload(&id, &q.role, &q.name, &body) {
        Ok(()) => Json(convert::analyze(&id)).into_response(),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct RunBody {
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// Start the conversion, or tell Claude what to change next. Uses the
/// subscription login when it is ready, else the API key.
async fn convert_run(State(s): State<Shared>, AxPath(id): AxPath<String>, Json(b): Json<RunBody>) -> impl IntoResponse {
    let Some(mut job) = convert::load(&id) else { return (StatusCode::NOT_FOUND, "找不到這次轉換").into_response() };
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let auth = if engine_ready(&s, EngineChoice::Subscription).await {
        convert::Auth::Subscription
    } else if let Some((k, _)) = s.llm.key() {
        convert::Auth::ApiKey(k)
    } else {
        return bad("模板轉換需要 Claude：在「設定 → Claude」登入帳號或設定 API key");
    };
    let model = b.model.filter(|m| m.starts_with("claude-")).unwrap_or_else(|| s.llm.catalog.cloud.default.clone());
    let system = convert::prompt(&ws);
    let instruction = b.instruction.map(|i| i.trim().to_string()).filter(|i| !i.is_empty());
    match convert::run(&mut job, auth, &model, &system, instruction).await {
        Ok(()) => Json(job).into_response(),
        Err(e) => bad(&e),
    }
}

async fn convert_stop(AxPath(id): AxPath<String>) -> impl IntoResponse {
    convert::stop(&id);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct FromQuery {
    #[serde(default)]
    from: usize,
}

async fn convert_events(AxPath(id): AxPath<String>, AxQuery(q): AxQuery<FromQuery>) -> impl IntoResponse {
    let (events, next) = convert::events(&id, q.from);
    let job = convert::load(&id);
    Json(serde_json::json!({ "events": events, "next": next, "status": job.as_ref().map(|j| j.status.clone()), "error": job.and_then(|j| j.error) }))
}

/// Build a paper from the work in progress, the way PaperComposer will use
/// the template, and render its first pages for comparison.
async fn convert_check(AxPath(id): AxPath<String>) -> impl IntoResponse {
    let Some(work) = convert::work(&id) else { return bad("bad job") };
    if !work.join("sample.tex").is_file() {
        return bad("還沒有 sample.tex");
    }
    let tmp = std::env::temp_dir().join(format!("papercomposer-convcheck-{id}"));
    let _ = std::fs::remove_dir_all(&tmp);
    let name = format!("conv-{}", &id[..8]);
    let res = match paper::create_paper_from_template(&tmp, "check", &work) {
        Ok(dir) => export::build_pdf(&dir, &name, false).await,
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_dir_all(&tmp);
    let preview = convert::root().join(&id).join("preview");
    let _ = std::fs::create_dir_all(&preview);
    for e in std::fs::read_dir(&preview).into_iter().flatten().flatten() {
        let _ = std::fs::remove_file(e.path());
    }
    let pdf = export::cache_dir(&name).join("main.pdf");
    if matches!(&res, Ok(r) if r.ok) {
        convert::render_pages(&pdf, &preview, "output", 3).await;
    }
    match res {
        Ok(mut r) => {
            r.file = None;
            Json(r).into_response()
        }
        Err(e) => Json(serde_json::json!({ "ok": false, "errors": [{ "message": e }] })).into_response(),
    }
}

#[derive(Deserialize)]
struct PageQuery {
    which: String,
    n: u32,
}

async fn convert_page(AxPath(id): AxPath<String>, AxQuery(q): AxQuery<PageQuery>) -> impl IntoResponse {
    let Some(work) = convert::work(&id) else { return bad("bad job") };
    let path = match q.which.as_str() {
        "reference" => work.join(format!("_upload/prep/reference-{}.png", q.n.min(9))),
        "output" => convert::root().join(&id).join(format!("preview/output-{}.png", q.n.min(9))),
        _ => return bad("which"),
    };
    match std::fs::read(path) {
        Ok(b) => ([(axum::http::header::CONTENT_TYPE, "image/png"), (axum::http::header::CACHE_CONTROL, "no-cache")], b).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn convert_save(State(s): State<Shared>, AxPath(id): AxPath<String>) -> impl IntoResponse {
    let d = match publishers_dir(&s).await { Ok(d) => d, Err(r) => return r };
    match convert::save_template(&id, &d) {
        Ok(tid) => Json(serde_json::json!({ "id": tid })).into_response(),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct FrontEdit {
    field: String,
    #[serde(default)]
    index: usize,
    value: String,
}

/// Change one field of the title block (title, author, affil, abstract, keywords).
async fn put_front(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<FrontEdit>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let path = pc_core::open::main_file(&dir);
    let main = match std::fs::read_to_string(&path) { Ok(m) => m, Err(e) => return bad(&e.to_string()) };
    let out = match pc_core::front::set_field(&main, &b.field, b.index, &b.value) { Ok(o) => o, Err(e) => return bad(&e) };
    if let Err(e) = paper::write_atomic(&path, &out) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let (front, back) = preview::front_back_in(&out, pc_core::open::is_custom(&dir));
    Json(serde_json::json!({ "front": front, "back": back, "fields": pc_core::front::fields(&out) })).into_response()
}

#[derive(Deserialize)]
struct FigureQuery {
    path: String,
    /// Pixels wide the editor wants (about twice its display width); the
    /// original when absent, as for export and print.
    w: Option<u32>,
}

/// A figure as an image the editor can show (PDF and EPS figures converted),
/// shrunk to `w` pixels wide when asked (copies cached by file, mtime and width).
async fn paper_figure(State(s): State<Shared>, AxPath(name): AxPath<String>, AxQuery(q): AxQuery<FigureQuery>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(file) = preview::resolve_figure(&dir, &q.path) else { return (StatusCode::NOT_FOUND, "no such figure").into_response() };
    let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer/figures");
    let shown = match preview::figure_image(&file, &cache).await {
        // widths come in steps of 128 px, so a few cached copies serve every window size
        Some((img, mime)) => match q.w {
            Some(w) => Some(thumb::thumbnail(&img, mime, &cache.join("thumbs"), w.clamp(64, 8192).div_ceil(128) * 128).await),
            None => Some((img, mime)),
        },
        None => None,
    };
    match shown {
        Some((img, mime)) => match std::fs::read(img) {
            Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, mime), (axum::http::header::CACHE_CONTROL, "no-cache")], bytes).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        None => (StatusCode::UNSUPPORTED_MEDIA_TYPE, "cannot show this figure").into_response(),
    }
}

#[derive(Deserialize)]
struct FigureSizesBody {
    paths: Vec<String>,
}

/// Width and height of each figure (from file headers; PDF/EPS in points),
/// so the page can reserve a figure's shape before its image arrives.
async fn paper_figure_sizes(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<FigureSizesBody>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let sizes: serde_json::Map<String, serde_json::Value> = b
        .paths
        .iter()
        .take(2000)
        .filter_map(|p| {
            let (w, h) = thumb::image_size(&preview::resolve_figure(&dir, p)?)?;
            Some((p.clone(), serde_json::json!([w, h])))
        })
        .collect();
    Json(serde_json::Value::Object(sizes)).into_response()
}

#[derive(Deserialize)]
struct SectionBody {
    title: String,
    paragraphs: Vec<String>,
    /// The section as the page last had it from the server: when the file
    /// no longer reads so (edited elsewhere), the save is refused (409).
    /// None = write anyway (保留我的版本).
    #[serde(default)]
    base: Option<SectionBase>,
}

#[derive(Deserialize)]
struct SectionBase {
    title: String,
    paragraphs: Vec<String>,
}

async fn put_section(
    State(s): State<Shared>,
    AxPath((name, file)): AxPath<(String, String)>,
    Json(b): Json<SectionBody>,
) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    if !file.ends_with(".tex") || !pc_core::open::is_section_file(&dir, &file) {
        return bad("invalid section file");
    }
    // the review (review.json) sees every save: tracked changes, comment anchors
    let _review = REVIEW_LOCK.lock().await;
    // Keep what follows \section{} (e.g. \label) from the file on disk.
    let was = pc_core::open::read_section(&dir, &file);
    if let (Some(base), Some(now)) = (&b.base, &was) {
        let norm = |p: &[String]| p.iter().map(|x| pc_core::review::norm(x)).collect::<Vec<_>>();
        if base.title.trim() != now.title.trim() || norm(&base.paragraphs) != norm(&now.paragraphs) {
            return (StatusCode::CONFLICT, Json(serde_json::json!({ "conflict": true, "section": now }))).into_response();
        }
    }
    let tail = was.as_ref().map(|s| s.tail.clone()).unwrap_or_default();
    let section = Section { file: file.clone(), title: b.title, paragraphs: b.paragraphs, tail };
    if let Err(e) = pc_core::open::write_section(&dir, &section) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    // Return what is now on disk, re-read, so the UI shows the saved form.
    let saved = pc_core::open::read_section(&dir, &file).unwrap_or_else(|| paper::parse_section(&file, ""));
    review_saved(&dir, &file, &was.map(|s| s.paragraphs).unwrap_or_default(), &saved.paragraphs);
    let mut v = serde_json::to_value(&saved).unwrap_or_default();
    v["stamp"] = serde_json::Value::String(pc_core::open::stamp(&dir));
    Json(v).into_response()
}

// ── Find & Replace: notation check (find.js) ───────────
// Before a replacement is saved, the registry's checks run on the changed
// paragraphs: what the new text adds over the old, and which registered
// entries (abbreviation, macro, label, symbol) the searched text touches.

#[derive(Deserialize)]
struct ReplaceCheckBody {
    /// The changed paragraphs as they are now, and as they would be.
    #[serde(default)]
    before: Vec<String>,
    #[serde(default)]
    texts: Vec<String>,
    #[serde(default)]
    find: String,
}

async fn notation_check(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<ReplaceCheckBody>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return bad("no workspace") };
    Json(replace_check(&ws, &dir, &b.before, &b.texts, &b.find)).into_response()
}

/// Findings in `after` that `before` does not have (messages compared without
/// their numbers, which hold line positions), plus the entries `find` touches.
fn replace_check(ws: &Path, dir: &Path, before: &[String], after: &[String], find: &str) -> serde_json::Value {
    let n = crate::notation::load(ws, dir);
    let opt = crate::notation::options(ws, dir);
    let run = |texts: &[String]| crate::notation::check(dir, &n, &opt, &[], &texts.join("\n\n"));
    let (old, new) = (run(before), run(after));
    let key = |m: &String| m.chars().filter(|c| !c.is_ascii_digit()).collect::<String>();
    let added = |was: &[String], now: &[String]| -> Vec<String> {
        let mut left: Vec<String> = was.iter().map(key).collect();
        now.iter()
            .filter(|m| match left.iter().position(|k| *k == key(m)) {
                Some(i) => { left.swap_remove(i); false }
                None => true,
            })
            .cloned()
            .collect()
    };
    let blocking = added(&old.blocking, &new.blocking);
    let shown: Vec<String> = added(&old.shown, &new.shown).into_iter().filter(|m| !blocking.contains(m)).collect();
    let new_symbols: Vec<&String> = new.new_symbols.iter().filter(|x| !old.new_symbols.contains(x)).collect();
    let f = find.trim();
    let lf = f.to_lowercase();
    let mut entries = vec![];
    if f.chars().count() >= 2 {
        for e in &n.entries {
            let macro_name = e.macro_name().map(|m| format!("\\{m}"));
            // words compare without case; LaTeX, macros and labels exactly
            let words = [e.short.as_deref(), e.long.as_deref()].into_iter().flatten().any(|w| {
                let w = w.to_lowercase();
                w.len() >= 2 && (lf.contains(&w) || w.contains(&lf))
            });
            let exact = [macro_name.as_deref(), e.label.as_deref(), e.latex.as_deref()].into_iter().flatten().any(|w| w.len() >= 2 && (f.contains(w) || w.contains(f)));
            if words || exact {
                let what = match (e.short.as_deref(), e.long.as_deref()) {
                    (Some(sh), Some(l)) => format!("{sh} — {l}"),
                    _ => macro_name.or_else(|| e.label.clone()).or_else(|| e.latex.clone()).or_else(|| e.short.clone()).unwrap_or_else(|| e.id.clone()),
                };
                entries.push(what);
            }
        }
    }
    serde_json::json!({ "blocking": blocking, "shown": shown, "new_symbols": new_symbols, "entries": entries })
}
// ── end Find & Replace ─────────────────────────────────

// ── Review: tracked changes and comments (review.js) ───
// review.json beside the paper, never in the .tex. Every section save goes
// through review_saved(), tracked or not, so changes and comments keep
// their places; while 追蹤修訂 is on it also records what changed. The
// author is the LoreForge profile's name (private/shared/profile.toml).

/// One review.json read-modify-write at a time, together with its section file.
pub(crate) static REVIEW_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Who is reviewing: the profile's display name, else its id, else 「我」.
pub(crate) fn review_author() -> String {
    let p = loreforge_home().prefs().profile;
    [p.name, p.id].into_iter().flatten().map(|s| s.trim().to_string()).find(|s| !s.is_empty()).unwrap_or_else(|| "我".into())
}

/// A section file went from `old` to `new` paragraphs (hold REVIEW_LOCK).
pub(crate) fn review_saved(dir: &Path, file: &str, old: &[String], new: &[String]) {
    let mut r = pc_core::review::load(dir);
    if !r.tracking && !r.changes.iter().any(|c| c.file == file) && !r.comments.iter().any(|c| c.file == file) {
        return;
    }
    let before = r.clone();
    pc_core::review::record(&mut r, file, old, new, &review_author(), &pc_core::review::now());
    if r != before {
        if let Err(e) = pc_core::review::save(dir, &r) {
            eprintln!("review.json: {e}");
        }
    }
}

fn section_of(dir: &Path, file: &str) -> Option<Section> {
    pc_core::open::read_section(dir, file)
}

/// The review with each change's place in its section now, and who I am.
fn review_view(dir: &Path, r: &pc_core::review::Review) -> serde_json::Value {
    let mut cache: std::collections::HashMap<String, Vec<String>> = Default::default();
    let mut v = serde_json::to_value(r).unwrap_or_default();
    if let Some(list) = v["changes"].as_array_mut() {
        for (x, c) in list.iter_mut().zip(&r.changes) {
            let paras = cache.entry(c.file.clone()).or_insert_with(|| section_of(dir, &c.file).map(|s| s.paragraphs).unwrap_or_default());
            x["place"] = serde_json::to_value(pc_core::review::locate(paras, &r.changes, c)).unwrap_or_default();
        }
    }
    v["me"] = serde_json::Value::String(review_author());
    v
}

async fn get_review(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let _g = REVIEW_LOCK.lock().await;
    Json(review_view(&dir, &pc_core::review::load(&dir))).into_response()
}

#[derive(Deserialize)]
struct TrackingBody {
    on: bool,
}

async fn put_tracking(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<TrackingBody>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let _g = REVIEW_LOCK.lock().await;
    let mut r = pc_core::review::load(&dir);
    r.tracking = b.on;
    if let Err(e) = pc_core::review::save(&dir, &r) { return bad(&e) }
    Json(review_view(&dir, &r)).into_response()
}

#[derive(Deserialize)]
struct DecideBody {
    /// "accept" | "reject" (one change); for all: also "resolve" (comments)
    action: String,
}

/// Reject change `id`: write the section with `before` back. Returns the
/// section and the notation findings the restored text adds.
fn reject_one(ws: Option<&Path>, dir: &Path, r: &mut pc_core::review::Review, id: &str) -> Result<(Section, Vec<String>), String> {
    let c = r.changes.iter().find(|c| c.id == id).cloned().ok_or("沒有這項修改")?;
    let mut sec = section_of(dir, &c.file).ok_or("找不到這個章節檔")?;
    let paras = pc_core::review::reject(r, id, &sec.paragraphs)?;
    sec.paragraphs = paras;
    pc_core::open::write_section(dir, &sec)?;
    let saved = section_of(dir, &c.file).ok_or("找不到這個章節檔")?;
    // the registry's checks on what came back (shown, not blocking)
    let issues = match (ws, &c.before) {
        (Some(ws), Some(before)) => {
            let was: Vec<String> = c.after.iter().cloned().collect();
            let res = replace_check(ws, dir, &was, std::slice::from_ref(before), "");
            let mut out: Vec<String> = ["blocking", "shown"]
                .iter()
                .flat_map(|k| res[*k].as_array().cloned().unwrap_or_default())
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            let syms: Vec<&str> = res["new_symbols"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            if !syms.is_empty() {
                out.push(format!("對照表沒有的符號：{}", syms.join("、")));
            }
            out
        }
        _ => vec![],
    };
    Ok((saved, issues))
}

async fn decide_review_change(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, Json(b): Json<DecideBody>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let ws = s.loaded.read().await.workspace.clone();
    let _g = REVIEW_LOCK.lock().await;
    let mut r = pc_core::review::load(&dir);
    let (sections, issues) = match b.action.as_str() {
        "accept" => match pc_core::review::accept(&mut r, &id) { Ok(_) => (vec![], vec![]), Err(e) => return bad(&e) },
        "reject" => match reject_one(ws.as_deref(), &dir, &mut r, &id) { Ok((sec, issues)) => (vec![sec], issues), Err(e) => return bad(&e) },
        _ => return bad("unknown action"),
    };
    if let Err(e) = pc_core::review::save(&dir, &r) { return bad(&e) }
    Json(serde_json::json!({ "review": review_view(&dir, &r), "stamp": pc_core::open::stamp(&dir), "sections": sections, "issues": issues })).into_response()
}

async fn decide_review_all(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<DecideBody>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let ws = s.loaded.read().await.workspace.clone();
    let _g = REVIEW_LOCK.lock().await;
    let mut r = pc_core::review::load(&dir);
    let (mut sections, mut issues, mut skipped): (Vec<Section>, Vec<String>, usize) = (vec![], vec![], 0);
    match b.action.as_str() {
        "accept" => r.changes.clear(),
        "reject" => {
            // passes until nothing more goes: a change under another one can go once the upper one has
            loop {
                let ids: Vec<String> = r.changes.iter().map(|c| c.id.clone()).collect();
                let mut done = false;
                for id in &ids {
                    if let Ok((sec, found)) = reject_one(ws.as_deref(), &dir, &mut r, id) {
                        sections.retain(|x: &Section| x.file != sec.file);
                        sections.push(sec);
                        issues.extend(found);
                        done = true;
                    }
                }
                if !done {
                    skipped = r.changes.len();
                    break;
                }
            }
        }
        "resolve" => r.comments.iter_mut().for_each(|c| c.resolved = true),
        _ => return bad("unknown action"),
    }
    if let Err(e) = pc_core::review::save(&dir, &r) { return bad(&e) }
    issues.dedup();
    Json(serde_json::json!({ "review": review_view(&dir, &r), "stamp": pc_core::open::stamp(&dir), "sections": sections, "issues": issues, "skipped": skipped })).into_response()
}

#[derive(Deserialize)]
struct NewComment {
    file: String,
    #[serde(default)]
    index: usize,
    #[serde(default)]
    exact: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    suffix: String,
    text: String,
}

async fn add_review_comment(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<NewComment>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    if b.text.trim().is_empty() { return bad("註解是空的") }
    let Some(sec) = section_of(&dir, &b.file) else { return bad("找不到這個章節檔") };
    let _g = REVIEW_LOCK.lock().await;
    let mut r = pc_core::review::load(&dir);
    r.comments.push(pc_core::review::Comment {
        id: pc_core::review::new_id("m"),
        author: review_author(),
        at: pc_core::review::now(),
        file: b.file.clone(),
        index: b.index,
        exact: b.exact,
        prefix: b.prefix,
        suffix: b.suffix,
        text: b.text.trim().to_string(),
        replies: vec![],
        resolved: false,
        orphan: false,
        source: None,
        extra: Default::default(),
    });
    pc_core::review::reanchor(&mut r, &b.file, &sec.paragraphs);
    if let Err(e) = pc_core::review::save(&dir, &r) { return bad(&e) }
    Json(review_view(&dir, &r)).into_response()
}

#[derive(Deserialize)]
struct CommentEdit {
    #[serde(default)]
    reply: Option<String>,
    #[serde(default)]
    resolved: Option<bool>,
}

async fn edit_review_comment(State(s): State<Shared>, AxPath((name, id)): AxPath<(String, String)>, Json(b): Json<CommentEdit>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let _g = REVIEW_LOCK.lock().await;
    let mut r = pc_core::review::load(&dir);
    let Some(c) = r.comments.iter_mut().find(|c| c.id == id) else { return bad("沒有這則註解") };
    if let Some(text) = b.reply.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        c.replies.push(pc_core::review::Reply { id: pc_core::review::new_id("r"), author: review_author(), at: pc_core::review::now(), text: text.into(), extra: Default::default() });
    }
    if let Some(v) = b.resolved {
        c.resolved = v;
    }
    if let Err(e) = pc_core::review::save(&dir, &r) { return bad(&e) }
    Json(review_view(&dir, &r)).into_response()
}
// ── end Review ─────────────────────────────────────────

// ── Database tables ────────────────────────────────────

async fn get_table(State(s): State<Shared>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let dir = s.loaded.read().await.path.clone();
    match table::read_table(&dir, &name) {
        Ok(t) => Json(t).into_response(),
        Err(e) => bad(&e),
    }
}

async fn put_table(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(t): Json<Table>) -> impl IntoResponse {
    let dir = s.loaded.read().await.path.clone();
    if let Err(e) = table::write_table(&dir, &name, &t) {
        return bad(&e);
    }
    let fresh = load(&s.paths);
    let issues: Vec<Issue> = fresh.issues.iter().filter(|i| i.file == name).cloned().collect();
    *s.loaded.write().await = fresh;
    Json(issues).into_response()
}

// ── Claude ─────────────────────────────────────────────

async fn claude_status(State(s): State<Shared>) -> impl IntoResponse {
    Json(s.llm.claude_status())
}

/// Pages the UI may open in the system browser. Fixed URLs only, so the
/// local API cannot be used to open arbitrary links.
const PAGES: &[(&str, &str)] = &[
    ("anthropic-keys", lore_llm::secrets::KEYS_PAGE),
    // GitHub pre-fills the form; the user still confirms on the page.
    ("github-new-repo", "https://github.com/new?name=my-paper-workspace&visibility=private"),
];

async fn open_page(AxPath(name): AxPath<String>) -> impl IntoResponse {
    let Some((_, url)) = PAGES.iter().find(|(n, _)| *n == name) else {
        return (StatusCode::NOT_FOUND, "unknown page").into_response();
    };
    match proc::open_in_browser(url) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ClaudeSettings {
    /// Some("") removes the stored key; None leaves it.
    key: Option<String>,
    model: Option<String>,
}

async fn claude_settings(State(s): State<Shared>, Json(b): Json<ClaudeSettings>) -> impl IntoResponse {
    if let Some(k) = &b.key {
        if let Err(e) = s.llm.save_key(k) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("無法存入系統金鑰圈：{e}")).into_response();
        }
    }
    if let Some(m) = b.model.filter(|m| s.llm.cloud_models().contains(m)) {
        if let Err(e) = write_config_value("claude", "model", &m) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
        s.llm.set_model(m);
    }
    Json(s.llm.claude_status()).into_response()
}

#[derive(Deserialize)]
struct DecomposeBody {
    text: String,
    section: Option<String>,
    #[serde(default)]
    engine: Option<EngineChoice>,
}

async fn decompose_text(State(s): State<Shared>, Json(b): Json<DecomposeBody>) -> impl IntoResponse {
    if b.text.trim().is_empty() {
        return bad("no text");
    }
    let prompt = {
        let l = s.loaded.read().await;
        let Some(p) = &l.profile else { return bad("profile not loaded") };
        decompose::build_prompt(p, &b.text, b.section.as_deref())
    };
    let reply = match writing_complete(&s, "pc-decompose", b.engine, &prompt, None, Some(decompose::reply_schema())).await {
        Ok(r) => r,
        Err(e) if e == "no engine" => return (StatusCode::BAD_GATEWAY, "需要先在「AI 與同步」連接一個引擎".to_string()).into_response(),
        Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
    };
    let l = s.loaded.read().await;
    let Some(p) = &l.profile else { return bad("profile not loaded") };
    match decompose::parse_proposals(p, &reply) {
        Ok(props) => Json(props).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("回覆無法拆解：{e}")).into_response(),
    }
}

/// Save picked proposals straight into the phrase bank: one step from a
/// sentence on the page, no trip through the database editor.
async fn add_proposals(State(s): State<Shared>, Json(p): Json<decompose::Proposals>) -> impl IntoResponse {
    let dir = s.loaded.read().await.path.clone();
    let phrases: Vec<Vec<(&str, String)>> = p
        .phrases
        .iter()
        .map(|x| {
            vec![
                ("section", x.section.clone()),
                ("function", x.function.clone()),
                ("position", x.position.clone()),
                ("template", x.template.clone()),
                ("example", x.example.clone()),
                ("tags", x.tags.join(";")),
            ]
        })
        .collect();
    let exprs: Vec<Vec<(&str, String)>> = p
        .expressions
        .iter()
        .map(|x| vec![("kind", x.kind.clone()), ("text", x.text.clone()), ("usage", x.usage.clone()), ("section", x.section.clone()), ("example", x.example.clone())])
        .collect();
    let added = table::append_rows(&dir, "phrases.csv", "P", "template", &[("priority", "3")], &phrases)
        .and_then(|a| Ok((a, table::append_rows(&dir, "expressions.csv", "X", "text", &[], &exprs)?)));
    let (ph, ex) = match added {
        Ok(x) => x,
        Err(e) => return bad(&e),
    };
    let fresh = load(&s.paths);
    let issues: Vec<Issue> = fresh.issues.iter().filter(|i| i.file == "phrases.csv" || i.file == "expressions.csv").cloned().collect();
    *s.loaded.write().await = fresh;
    Json(serde_json::json!({ "phrases": ph, "expressions": ex, "issues": issues })).into_response()
}

// ── Git ────────────────────────────────────────────────

async fn workspace_root(s: &Shared) -> Option<PathBuf> {
    let l = s.loaded.read().await;
    if l.demo { None } else { l.workspace.clone() }
}

async fn git_status(State(s): State<Shared>) -> impl IntoResponse {
    match workspace_root(&s).await {
        Some(ws) => Json(lore_gitsync::Repo::new(ws).status().await).into_response(),
        None => Json(lore_gitsync::GitStatus::default()).into_response(),
    }
}

async fn git_pull(State(s): State<Shared>) -> impl IntoResponse {
    let Some(ws) = workspace_root(&s).await else { return Json(serde_json::json!({ "pulled": false })).into_response() };
    match lore_gitsync::Repo::new(ws).pull().await {
        Ok(pulled) => {
            if pulled {
                *s.loaded.write().await = load(&s.paths);
            }
            Json(serde_json::json!({ "pulled": pulled })).into_response()
        }
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

#[derive(Deserialize, Default)]
struct SyncBody {
    message: Option<String>,
}

async fn git_sync(State(s): State<Shared>, body: Option<Json<SyncBody>>) -> impl IntoResponse {
    let Some(ws) = workspace_root(&s).await else { return bad("示範工作區不做同步") };
    let msg = body.and_then(|Json(b)| b.message);
    let opts = lore_gitsync::SyncOptions { message: msg, group_roots: vec!["papers".into()], ..Default::default() };
    match lore_gitsync::Repo::new(ws).sync(&opts).await {
        Ok(r) => {
            if r.pulled {
                *s.loaded.write().await = load(&s.paths);
            }
            Json(r).into_response()
        }
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

// ── Setup wizard ───────────────────────────────────────

#[derive(Serialize)]
struct GitTool {
    installed: bool,
    version: Option<String>,
    name: Option<String>,
    email: Option<String>,
}

async fn cmd_out(args: &[&str]) -> Option<String> {
    let out = crate::proc::cmd(args[0]).args(&args[1..]).env("GIT_TERMINAL_PROMPT", "0").output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

async fn git_tool() -> GitTool {
    let version = cmd_out(&["git", "--version"]).await;
    GitTool {
        installed: version.is_some(),
        version,
        name: cmd_out(&["git", "config", "--global", "user.name"]).await,
        email: cmd_out(&["git", "config", "--global", "user.email"]).await,
    }
}

async fn setup_status(State(s): State<Shared>) -> impl IntoResponse {
    let cfg = read_config();
    let env_ws = std::env::var("PAPERCOMPOSER_WORKSPACE").is_ok() || std::env::var("PAPERCOMPOSER_PROFILE").is_ok();
    let (root, demo) = {
        let l = s.loaded.read().await;
        (l.workspace.clone(), l.demo)
    };
    let ws_git = match (&root, demo) {
        (Some(r), false) => Some(lore_gitsync::Repo::new(r).status().await),
        _ => None,
    };
    let sys = models::system_info().await;
    let cat = &s.llm.catalog;
    let recommended = models::recommend(cat, &sys);
    let pool = sys.memory_gb.max(sys.gpu_memory_gb.unwrap_or(0.0)) as f64;
    let installed = models::installed(cat);
    let running = s.models.running().await.map(|r| r.0);
    Json(serde_json::json!({
        "first_run": !env_ws && cfg.workspace_path.is_none() && cfg.profile_path.is_none(),
        "config_path": config_path().map(|p| p.display().to_string()),
        "workspace": {
            "root": root.as_ref().map(|r| r.display().to_string()),
            "demo": demo,
            "has_profile": root.as_ref().is_some_and(|r| r.join("profile/profile.toml").exists()),
            "papers": root.as_ref().map(|r| subdirs(&r.join("papers")).len()).unwrap_or(0),
            "git": ws_git,
        },
        "git_tool": git_tool().await,
        "system": sys,
        "models": cat.brains(true).map(|m| serde_json::json!({
            "entry": m,
            "fits": pool + 0.5 >= m.min_memory_gb,
            "installed": installed.contains(&m.id),
        })).collect::<Vec<_>>(),
        "recommended": recommended,
        "runtime_installed": models::runtime_binary().is_some(),
        "running": running,
        "phase": s.models.phase().await,
        "engine": s.llm.local_status().await,
        "claude": s.llm.claude_status(),
    }))
}

async fn use_workspace(s: &Shared, path: &Path, init: bool) -> Result<serde_json::Value, String> {
    if !path.is_dir() {
        return Err("資料夾不存在".into());
    }
    let added = if init { copy_missing(&s.paths.demo_workspace, path).map_err(|e| e.to_string())? } else { 0 };
    if !path.join("profile/profile.toml").exists() {
        return Err("這個資料夾沒有 profile/，請勾選「補上缺少的資料夾」".into());
    }
    let abs = path.canonicalize().unwrap_or(path.to_path_buf());
    write_config_root("workspace_path", &abs.display().to_string())?;
    *s.loaded.write().await = load(&s.paths);
    Ok(serde_json::json!({ "root": abs.display().to_string(), "added": added }))
}

#[derive(Deserialize)]
struct WorkspaceBody {
    path: String,
    #[serde(default)]
    init: bool,
}

async fn setup_workspace(State(s): State<Shared>, Json(b): Json<WorkspaceBody>) -> impl IntoResponse {
    match use_workspace(&s, Path::new(b.path.trim()), b.init).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct CloneBody {
    url: String,
    /// Folder that will contain the clone.
    parent: String,
}

fn repo_name(url: &str) -> Option<String> {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = last.trim_end_matches(".git");
    paper::safe_name(name).then(|| name.to_string())
}

async fn setup_clone(State(s): State<Shared>, Json(b): Json<CloneBody>) -> impl IntoResponse {
    let url = b.url.trim();
    let Some(name) = repo_name(url) else { return bad("無法從網址判斷 repo 名稱") };
    let parent = PathBuf::from(b.parent.trim());
    if !parent.is_dir() {
        return bad("存放位置不存在");
    }
    let target = parent.join(&name);
    if target.exists() {
        return bad(&format!("{} 已經存在；若已 clone 過，請改用「開啟資料夾」", target.display()));
    }
    let out = crate::proc::cmd("git")
        .args(["clone", "--quiet", url])
        .arg(&target)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => match use_workspace(&s, &target, true).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => bad(&e),
        },
        Ok(o) => bad(&format!("clone 失敗：{}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => bad(&format!("無法執行 git：{e}")),
    }
}

#[derive(Deserialize)]
struct NewBody {
    /// Folder to create (must not exist or be empty).
    path: String,
    /// Optional: the user's own empty private repository.
    remote: Option<String>,
}

async fn setup_new(State(s): State<Shared>, Json(b): Json<NewBody>) -> impl IntoResponse {
    let path = PathBuf::from(b.path.trim());
    if path.exists() && std::fs::read_dir(&path).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return bad("資料夾已存在且不是空的");
    }
    if let Err(e) = copy_missing(&s.paths.demo_workspace, &path) {
        return bad(&e.to_string());
    }
    let run = |args: Vec<String>| {
        let p = path.clone();
        async move {
            let o = crate::proc::cmd("git").args(&args).current_dir(&p).env("GIT_TERMINAL_PROMPT", "0").output().await;
            match o {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(e) => Err(e.to_string()),
            }
        }
    };
    let mut note = None;
    if run(vec!["init".into(), "-q".into(), "-b".into(), "main".into()]).await.is_ok() {
        let _ = run(vec!["add".into(), "-A".into()]).await;
        if let Err(e) = run(vec!["commit".into(), "-q".into(), "-m".into(), "Create workspace".into()]).await {
            note = Some(format!("已建立資料夾，但無法 commit：{e}"));
        } else if let Some(r) = b.remote.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
            let _ = run(vec!["remote".into(), "add".into(), "origin".into(), r.into()]).await;
            if let Err(e) = run(vec!["push".into(), "-q".into(), "-u".into(), "origin".into(), "main".into()]).await {
                note = Some(format!("已建立，但推送到遠端失敗：{e}"));
            }
        }
    }
    match use_workspace(&s, &path, false).await {
        Ok(mut v) => {
            v["note"] = serde_json::json!(note);
            Json(v).into_response()
        }
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct VisibilityQuery {
    url: String,
}

/// For GitHub URLs: is the repository public? Anonymous lookup, so a private
/// repository looks the same as a missing one (404) — both are fine here.
async fn setup_visibility(AxQuery(q): AxQuery<VisibilityQuery>) -> impl IntoResponse {
    let u = q.url.trim().trim_end_matches(".git");
    let path = u
        .split_once("github.com/")
        .or_else(|| u.split_once("github.com:"))
        .map(|(_, p)| p.trim_matches('/').to_string());
    let Some(path) = path.filter(|p| p.split('/').count() == 2) else {
        return Json(serde_json::json!({ "host": "other" }));
    };
    let api = std::env::var("PAPERCOMPOSER_GH_API").unwrap_or_else(|_| "https://api.github.com".into());
    let r = reqwest::Client::builder().user_agent("PaperComposer").build().unwrap().get(format!("{api}/repos/{path}")).send().await;
    let public = match r {
        Ok(r) if r.status().is_success() => r.json::<serde_json::Value>().await.ok().and_then(|v| v["private"].as_bool()).map(|p| !p),
        Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => Some(false),
        _ => None,
    };
    Json(serde_json::json!({ "host": "github", "public": public }))
}

#[derive(Deserialize)]
struct IdentityBody {
    name: String,
    email: String,
}

async fn setup_git_identity(Json(b): Json<IdentityBody>) -> impl IntoResponse {
    for (k, v) in [("user.name", b.name.trim()), ("user.email", b.email.trim())] {
        if v.is_empty() {
            return bad("名稱與 email 都要填");
        }
        if cmd_out(&["git", "config", "--global", k, v]).await.is_none() {
            // `git config` prints nothing on success; check it took.
            if cmd_out(&["git", "config", "--global", k]).await.as_deref() != Some(v) {
                return bad("無法寫入 git 設定");
            }
        }
    }
    Json(git_tool().await).into_response()
}

// ── Local models ───────────────────────────────────────

#[derive(Deserialize)]
struct ModelBody {
    id: String,
}

async fn model_plan(State(s): State<Shared>, Json(b): Json<ModelBody>) -> impl IntoResponse {
    match models::plan(&s.llm.catalog, &b.id).await {
        Ok(p) => Json(p).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

/// Pick a downloaded model: it becomes the shared default (`engine.toml` `[engine] model`,
/// every app's `local:current`) in place of any external server, and is loaded now on the
/// machine's shared llama-server.
async fn activate(s: &Shared, id: &str) -> Result<(), String> {
    loreforge_home().set_engine_model(Some(id)).map_err(|e| format!("無法儲存本機模型：{e}"))?;
    s.llm.set_external(EngineConfig::default());
    write_config_value("model", "active", id)?;
    models::start(&s.models, &s.llm.catalog, id).await.map(|_| ())
}

/// Re-plans on the server (the client never supplies URLs), downloads, then starts.
async fn model_install(State(s): State<Shared>, Json(b): Json<ModelBody>) -> impl IntoResponse {
    if s.models.phase().await == "downloading" {
        return (StatusCode::CONFLICT, "已有下載進行中").into_response();
    }
    let plan = match models::plan(&s.llm.catalog, &b.id).await {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
    };
    s.models.progress.lock().await.clear();
    let s2 = s.clone();
    tokio::spawn(async move {
        if models::install(s2.models.clone(), plan).await.is_ok() {
            if let Err(e) = activate(&s2, &b.id).await {
                s2.models.note(Some(format!("error: {e}"))).await;
            }
        }
    });
    StatusCode::ACCEPTED.into_response()
}

async fn model_progress(State(s): State<Shared>) -> impl IntoResponse {
    let files: Vec<models::Progress> = s.models.progress.lock().await.values().cloned().collect();
    let running = s.models.running().await;
    Json(serde_json::json!({ "phase": s.models.phase().await, "files": files, "running": running }))
}

async fn model_start(State(s): State<Shared>, Json(b): Json<ModelBody>) -> impl IntoResponse {
    match activate(&s, &b.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

async fn model_stop(State(s): State<Shared>) -> impl IntoResponse {
    if let Err(e) = models::stop(&s.models).await {
        return (StatusCode::CONFLICT, e).into_response();
    }
    s.llm.set_external(EngineConfig::resolve(read_config().engine));
    StatusCode::NO_CONTENT.into_response()
}

pub fn router(paths: Paths) -> Router {
    let cfg = read_config();
    let engine_cfg = EngineConfig::resolve(cfg.engine);
    let external = engine_cfg.url.is_some();
    // the machine's one llama-server, shared with every Lore app (lore-llm shared_runtime):
    // PaperComposer no longer owns it; tests and the isolated example never touch it
    let local_rt = if cfg!(test) || engines_isolated() { None } else { lore_llm::shared_runtime::SharedRuntime::process("pc") };
    let manager = Arc::new(models::Manager::with_runtime(local_rt.clone()));
    let catalog = models::catalog();
    // The app's tasks, and the user's overrides when their file is readable.
    let user_tasks = config_path().map(|p| p.with_file_name("tasks.toml"));
    let tasks = Llm::task_table(user_tasks.as_deref()).unwrap_or_else(|e| {
        eprintln!("{e}");
        Llm::task_table(None).expect("bundled tasks.toml is valid")
    });
    let llm = Llm::new(catalog, tasks, local_rt, models::layout(), engine_cfg, cfg.claude.and_then(|c| c.model));
    llm.share_engine(loreforge_home());
    // the model folder, for apps opened from Finder (no LOCAL_LLM_HOME there): engine.toml [local] home
    // (only where it is known; tests and the isolated example never write the real private/shared)
    if !cfg!(test) && !engines_isolated() && let Some(h) = models::llm_home() {
        if let Err(e) = loreforge_home().set_local_home(Some(&h)) {
            eprintln!("local model folder: {e}");
        }
    }
    let state: Shared = Arc::new(AppState {
        loaded: RwLock::new(load(&paths)),
        llm: Arc::new(llm),
        models: manager,
        paths: paths.clone(),
        soft_cache: Default::default(),
    });
    models::register(state.models.clone());
    // The model loads when a task needs it (any app may start the shared server). Once:
    // the model this app used to start at launch becomes the shared default when none is set.
    if let (false, Some(id), true) = (external, cfg.model.and_then(|m| m.active), state.llm.local_rt.is_some())
        && state.llm.shared_engine().model.is_none()
        && models::installed(&state.llm.catalog).contains(&id)
        && let Err(e) = loreforge_home().set_engine_model(Some(&id))
    {
        eprintln!("local model: {e}");
    }
    Router::new()
        .route("/prefs.css", get(prefs_css))
        .route("/theme.css", get(theme_css))
        .route("/theme.js", get(theme_js))
        .route("/api/theme", get(theme_get).put(theme_put))
        .route("/api/setup/status", get(setup_status))
        .route("/api/setup/workspace", post(setup_workspace))
        .route("/api/setup/clone", post(setup_clone))
        .route("/api/setup/new", post(setup_new))
        .route("/api/setup/visibility", get(setup_visibility))
        .route("/api/setup/git-identity", post(setup_git_identity))
        .route("/api/models/plan", post(model_plan))
        .route("/api/models/install", post(model_install))
        .route("/api/models/progress", get(model_progress))
        .route("/api/models/start", post(model_start))
        .route("/api/models/stop", post(model_stop))
        .route("/api/models/catalog", get(|State(s): State<Shared>| async move { Json(s.llm.catalog.clone()) }))
        .route("/api/git/status", get(git_status))
        .route("/api/git/pull", post(git_pull))
        .route("/api/git/sync", post(git_sync))
        .route("/api/papers", post(create_paper))
        .route("/api/papers/import", post(import_upload).layer(axum::extract::DefaultBodyLimit::max(300 * 1024 * 1024)))
        .route("/api/papers/import-arxiv", post(import_arxiv))
        .route("/api/papers/{name}", get(get_paper))
        .route("/api/papers/{name}/packages", post(paper_packages))
        .route("/api/papers/{name}/table-fit", post(table_fit))
        .route("/api/papers/{name}/citations", get(paper_citations).post(set_purpose))
        .route("/api/papers/{name}/bib", post(paper_bib))
        .route("/api/papers/{name}/bib/merge", post(merge_bib))
        .route("/api/papers/{name}/tidy", get(get_tidy).post(post_tidy))
        .route("/api/papers/{name}/import-summary", get(import_summary))
        .route("/api/papers/{name}/export", post(export_paper))
        .route("/api/papers/{name}/figure", get(paper_figure))
        .route("/api/papers/{name}/figure-sizes", post(paper_figure_sizes))
        .route("/api/papers/{name}/figure-block", post(floats::figure_block_route))
        .route("/api/papers/{name}/figures", post(put_figure).layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)))
        .route("/api/papers/{name}/front", put(put_front))
        .route("/api/papers/{name}/meta", put(put_meta))
        .route("/api/people", get(people).post(add_person))
        .route("/api/wizard/outline", post(wizard_outline))
        .route("/api/wizard/caption", post(wizard_caption))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/refs/extract", post(extract_refs))
        .route("/api/refs/verify", post(verify_ref))
        .route("/api/refs/propose-record", post(propose_record))
        .route("/api/templates", get(list_templates).post(create_template))
        .route("/api/newtemplate", post(newtemplate::start))
        .route("/api/newtemplate/{id}", get(newtemplate::get))
        .route("/api/newtemplate/{id}/files", post(newtemplate::upload).layer(axum::extract::DefaultBodyLimit::max(200 * 1024 * 1024)))
        .route("/api/newtemplate/{id}/rules", post(newtemplate::rules))
        .route("/api/newtemplate/{id}/create", post(newtemplate::create))
        .route("/api/convert", post(convert_create))
        .route("/api/convert/{id}", get(convert_get))
        .route("/api/convert/{id}/upload", post(convert_upload).layer(axum::extract::DefaultBodyLimit::max(200 * 1024 * 1024)))
        .route("/api/convert/{id}/run", post(convert_run))
        .route("/api/convert/{id}/stop", post(convert_stop))
        .route("/api/convert/{id}/events", get(convert_events))
        .route("/api/convert/{id}/check", post(convert_check))
        .route("/api/convert/{id}/page", get(convert_page))
        .route("/api/convert/{id}/save", post(convert_save))
        .route("/api/templates/{id}", put(update_template))
        .route("/api/templates/{id}/sample", get(get_template_sample).put(put_template_sample))
        .route("/api/templates/{id}/check", post(check_template))
        .route("/api/papers/{name}/export/open", post(open_export))
        .route("/api/refs", get(list_refs))
        .route("/api/refs/tags", get(ref_tags))
        .route("/api/refs/import", post(import_refs))
        .route("/api/refs/pdfs", get(list_pdfs))
        .route("/api/refs/propose", post(propose_ref))
        .route("/api/refs/add", post(add_ref))
        .route("/api/refs/search", post(search_refs))
        .route("/api/open-link", post(open_doi))
        .route("/api/refs/{key}", put(update_ref))
        .route("/api/refs/{key}/open", post(open_ref_pdf))
        .route("/api/reader/papers", get(reader::papers)) // reader.rs: LoreReader's library, read-only
        .route("/api/reader/papers/{sha}", get(reader::paper))
        .route("/api/reader/papers/{sha}/cite", post(reader::cite))
        .route("/api/reader/open", post(reader::open))
        .route("/api/reader/paraphrase", post(reader::paraphrase))
        .route("/api/papers/{name}/reader/notation", post(reader::register)) // reader.js: 插入 a symbol or equation
        .route("/api/kb/cards", post(kb::cards)) // kb.rs: 知識卡 for a paragraph or a search
        .route("/api/kb/settings", get(kb::settings_get).put(kb::settings_put))
        .route("/api/kb/paraphrase", post(kb::paraphrase))
        .route("/api/papers/{name}/kb/notation", post(kb::register))
        .route("/api/settings/pdf-dir", post(set_pdf_dir))
        .route("/api/papers/{name}/sections/{file}", put(put_section))
        .route("/api/papers/{name}/notation/check", post(notation_check)) // find.js
        .route("/api/papers/{name}/review", get(get_review)) // review.js
        .route("/api/papers/{name}/review/tracking", put(put_tracking))
        .route("/api/papers/{name}/review/all", post(decide_review_all))
        .route("/api/papers/{name}/review/changes/{id}", post(decide_review_change))
        .route("/api/papers/{name}/review/comments", post(add_review_comment))
        .route("/api/papers/{name}/review/comments/{id}", put(edit_review_comment))
        .route("/api/papers/{name}/word-refs", get(word::api::list)) // review.js
        .route("/api/papers/{name}/word-refs/confirm", post(word::api::confirm))
        .route("/api/tables/{name}", get(get_table).put(put_table))
        .route("/api/claude", get(claude_status))
        .route("/api/open/{page}", post(open_page))
        .route("/api/settings/claude", post(claude_settings))
        .route("/api/decompose", post(decompose_text))
        .route("/api/phrases/add", post(add_proposals))
        .route("/api/workspace", get(workspace_info))
        .route("/api/profile", get(profile_info))
        .route("/api/profile/rules", get(rule_messages))
        .route("/api/profile/reload", post(reload))
        .route("/api/phrases", get(phrases))
        .route("/api/check", post(check))
        .route("/api/textable/parse", post(table_parse))
        .route("/api/textable/render", post(table_render))
        .route("/api/textable/styles", get(table_styles))
        .route("/api/textable/apply", post(table_apply))
        .route("/api/textable/new", post(table_new))
        .route("/api/check/soft", post(soft_check))
        .route("/api/engine", get(engine_status))
        .route("/api/engines/status", get(engines_status)) // setup.js, the 寫作 engine switch; /api/engines is lore-engines (the gear's 模型)
        .route("/api/engines/test", post(test_engine))
        .route("/api/claude-code/login", post(claude_code_login))
        .route("/api/compose", post(compose_paragraph))
        .merge(draft::routes())
        .merge(home::routes())
        .merge(books::routes().layer(book::local_cors()))
        .merge(book::routes())
        .merge(sections::routes())
        .merge(opened::routes())
        .merge(notation::routes())
        .merge(notes::routes())
        .merge(shell::routes())
        .merge(todeck::routes())
        .merge(plot::routes(&state))
        // GET/PUT /api/engines (the gear's 模型) and POST /api/links/open, shared by every app
        .merge(lore_engines::router(engines_settings(&state)))
        // the UI is read fresh each time: a new server must not meet an old script
        .fallback_service(Router::new().fallback_service(ServeDir::new(paths.ui)).layer(axum::middleware::map_response(no_cache)))
        .with_state(state)
}

/// The shared 模型 settings (lore-engines) over this app's llama-server, Claude Code and tasks.
/// `PAPERCOMPOSER_ENGINES_ISOLATED` (tests, the isolated example) keeps them off the
/// real engine.toml routing.
fn engines_settings(state: &Shared) -> lore_engines::Engines {
    let mut e = lore_engines::Engines::new(loreforge_home(), "PaperComposer", include_str!("../tasks.toml"))
        .with_tasks(lore_plot::propose::task_table())
        .layout(state.llm.layout.clone())
        .cli(state.llm.cli.clone(), state.llm.cc.clone())
        .claude_model(Some(state.llm.model()));
    e.include_uncensored = true;
    if cfg!(test) || engines_isolated() {
        e.install_routing = false;
    } else if let Some(rt) = state.llm.local_rt.clone() {
        e = e.shared(rt); // 本機 status, 釋放, and this app's share of the idle checks
    }
    e
}

/// `PAPERCOMPOSER_ENGINES_ISOLATED` (tests, the isolated example): no shared settings
/// routing and no shared llama-server.
fn engines_isolated() -> bool {
    std::env::var_os("PAPERCOMPOSER_ENGINES_ISOLATED").is_some()
}

/// Bind on 127.0.0.1 (port 0 = any free port) and return the bound address
/// plus a future that serves until dropped.
pub async fn bind(
    paths: Paths,
    port: u16,
) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = std::io::Result<()>>)> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let addr = listener.local_addr()?;
    let app = router(paths);
    Ok((addr, async move { axum::serve(listener, app).await }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejecting_writes_the_old_paragraph_back_and_checks_it() {
        let root = std::env::temp_dir().join(format!("pc-review-reject-{}-{}", std::process::id(), pc_core::review::new_id("")));
        let (ws, dir) = (root.join("ws"), root.join("ws/papers/P"));
        std::fs::create_dir_all(dir.join("sections")).unwrap();
        std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/res}\n\\end{document}\n").unwrap();
        std::fs::write(dir.join("notation.json"), r#"{"version":1,"entries":[{"id":"t1","kind":"table","label":"tab:main","caption":"Main"}]}"#).unwrap();
        // the old text typed a table number; the tracked edit fixed it with \ref
        let old = vec!["Table 1 shows the gain.".to_string(), "Second.".to_string()];
        let new = vec!["Table~\\ref{tab:main} shows the gain.".to_string(), "Second.".to_string()];
        let sec = |p: &[String]| Section { file: "res.tex".into(), title: "Results".into(), paragraphs: p.to_vec(), tail: String::new() };
        std::fs::write(dir.join("sections/res.tex"), paper::render_section(&sec(&new))).unwrap();
        let mut r = pc_core::review::Review { tracking: true, ..Default::default() };
        pc_core::review::record(&mut r, "res.tex", &old, &new, "ellrin", "t1");
        let id = r.changes[0].id.clone();
        let (saved, issues) = reject_one(Some(&ws), &dir, &mut r, &id).unwrap();
        assert_eq!(saved.paragraphs, old);
        assert!(r.changes.is_empty());
        let on_disk = std::fs::read_to_string(dir.join("sections/res.tex")).unwrap();
        assert!(on_disk.contains("Table 1 shows the gain."));
        // the restored text is checked against the registry; shown, not blocking
        assert!(!issues.is_empty(), "a typed table number comes back and is reported");
        // tracked changes never reach the .tex
        assert!(!on_disk.contains("ellrin"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn merges_a_paper_bib_into_the_library_without_duplicates() {
        let root = std::env::temp_dir().join(format!("pc-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (ws, dir) = (root.join("ws"), root.join("ws/papers/P"));
        std::fs::create_dir_all(ws.join("refs")).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(ws.join("refs/library.bib"), "@article{kept2020,\n  title = {Kept},\n  doi = {10.1000/Kept},\n  year = {2020}\n}\n").unwrap();
        std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\bibliography{references}\n\\end{document}\n").unwrap();
        std::fs::write(
            dir.join("references.bib"),
            "@article{kept2020,\n  title = {Same key},\n  year = {2020}\n}\n@article{other2021,\n  title = {Same DOI},\n  doi = {10.1000/kept},\n  year = {2021}\n}\n@inproceedings{new2022,\n  title = {New},\n  year = {2022}\n}\n",
        )
        .unwrap();
        let preview = merge_paper_bib(&ws, &dir, true).await.unwrap();
        assert_eq!((preview.added.as_slice(), preview.skipped.len()), (&["new2022".to_string()][..], 2));
        assert!(!std::fs::read_to_string(ws.join("refs/library.bib")).unwrap().contains("new2022"), "a dry run writes nothing");
        let r = merge_paper_bib(&ws, &dir, false).await.unwrap();
        assert_eq!(r.files, ["references.bib"]);
        assert_eq!(r.added, ["new2022"]);
        assert_eq!(r.skipped, ["kept2020", "other2021"]);
        let (entries, notes) = load_library(&ws);
        assert_eq!(entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["kept2020", "new2022"]);
        assert!(notes.iter().any(|n| n.key == "new2022"));
        assert!(merge_paper_bib(&ws, &dir, false).await.unwrap().added.is_empty(), "merging twice adds nothing");
        let _ = std::fs::remove_dir_all(&root);
    }

    // Find & Replace (find.js): the registry's checks on a replacement.
    #[test]
    fn replace_check_reports_what_a_replacement_adds_and_the_entries_it_touches() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("pc-replace-{}-{nanos}", std::process::id()));
        let (ws, dir) = (root.join("ws"), root.join("ws/papers/P"));
        std::fs::create_dir_all(dir.join("sections")).unwrap();
        std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
        let para = "Table~\\ref{tab:main} compares the runs.".to_string();
        std::fs::write(
            dir.join("sections/results.tex"),
            format!("\\section{{Results}}\n{para}\n\n\\begin{{table}}\n\\caption{{Main comparison.}}\n\\label{{tab:main}}\n\\begin{{tabular}}{{l}}\nx\n\\end{{tabular}}\n\\end{{table}}\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("notation.json"),
            r#"{"version":1,"entries":[{"id":"t1","kind":"table","label":"tab:main","caption":"Main comparison"},{"id":"cv","kind":"abbrev","short":"CV","long":"cross-validation"}]}"#,
        )
        .unwrap();
        let typed = "Table 1 compares the runs.".to_string();
        let r = replace_check(&ws, &dir, std::slice::from_ref(&para), std::slice::from_ref(&typed), "Table~\\ref{tab:main}");
        assert!(!r["blocking"].as_array().unwrap().is_empty(), "a typed 'Table 1' is new: {r}");
        assert_eq!(r["entries"], serde_json::json!(["tab:main"]));
        let same = replace_check(&ws, &dir, std::slice::from_ref(&typed), std::slice::from_ref(&typed), "runs");
        assert!(same["blocking"].as_array().unwrap().is_empty() && same["shown"].as_array().unwrap().is_empty(), "nothing new: {same}");
        assert!(same["entries"].as_array().unwrap().is_empty());
        let cv = replace_check(&ws, &dir, &[], &[], "cross-validation");
        assert_eq!(cv["entries"], serde_json::json!(["CV — cross-validation"]));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The theme is the one all LoreForge apps share: read and set through
    /// lore-home, and the page takes its palettes from commons/ui only.
    #[test]
    fn theme_is_the_shared_choice_and_the_page_has_no_palette_of_its_own() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("pc-theme-{}-{nanos}", std::process::id()));
        let home = lore_home::Home::at(&root);
        assert_eq!(theme_value(&home), serde_json::json!({ "theme": null }));
        assert!(theme_set(&home, &serde_json::json!({ "theme": "night" })).is_err(), "old ids are not stored");
        assert_eq!(theme_set(&home, &serde_json::json!({ "theme": "aurora" })).unwrap(), serde_json::json!({ "theme": "aurora" }));
        assert_eq!(theme_value(&home), serde_json::json!({ "theme": "aurora" }));
        assert!(lore_home::theme::script(&home).starts_with("window.LORE_THEME = \"aurora\";"));
        let _ = std::fs::remove_dir_all(&root);

        let ui = Paths::from_repo().ui;
        let index = std::fs::read_to_string(ui.join("index.html")).unwrap();
        let (theme, styles) = (index.find("href=\"theme.css\"").unwrap(), index.find("href=\"styles.css\"").unwrap());
        assert!(theme < styles, "theme.css loads before styles.css (the page's white sheet overrides --paper)");
        assert!(index.contains("<script src=\"theme.js\"></script>"));
        let css = std::fs::read_to_string(ui.join("styles.css")).unwrap();
        assert!(!css.contains("[data-theme=\""), "palettes live in commons/ui/theme.css only");
        // accent as text or a thin line is --accent-text; --accent is a fill
        for bad in [" color: var(--accent)", "border-color: var(--accent)", "solid var(--accent);  ", "{ color: var(--accent)"] {
            assert!(!css.contains(bad), "{bad}");
        }
        let app = std::fs::read_to_string(ui.join("app.js")).unwrap();
        assert!(!app.contains("pc.theme") && !app.contains("pc.mode"));
    }
}
