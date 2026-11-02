//! PaperComposer local server, used both by `cargo run -p pc-server` (browser
//! dev mode) and by the desktop app, which starts it in-process and points
//! its window at it.
//!
//! Binds to 127.0.0.1 only. Finds the writing profile in this order:
//! 1. `PAPERCOMPOSER_WORKSPACE` (private repo root; profile in `profile/`)
//! 2. `PAPERCOMPOSER_PROFILE` (a profile folder on its own)
//! 3. `workspace_path` or `profile_path` in `<config dir>/PaperComposer/config.toml`
//! 4. the bundled `workspace-template/` (demo mode)
//!
//! The config file lives in the OS config folder, never inside a repository,
//! so a personal profile path is never committed by accident.

pub mod claude;
pub mod claude_code;
pub mod convert;
pub mod engine;
pub mod export;
pub mod import;
pub mod preview;
pub mod proc;
pub mod git;
pub mod models;
pub mod refimport;
pub mod texmeasure;

use axum::{
    extract::{Path as AxPath, Query as AxQuery, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post, put},
    Router,
};
use claude::Claude;
use engine::{Engine, EngineConfig, EngineStatus};
use pc_core::compose::{self, ComposeRequest, Draft, DraftCheck, Prompt};
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
        Paths { ui: root.join("ui"), demo_workspace: root.join("workspace-template") }
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
    engine: RwLock<Engine>,
    models: Arc<models::Manager>,
    claude: RwLock<Claude>,
    paths: Paths,
    /// Soft-check results by prompt hash: an unchanged paragraph is not re-sent.
    soft_cache: tokio::sync::Mutex<std::collections::HashMap<u64, Vec<SoftResult>>>,
    /// Claude Code CLI status (installed / logged in), refreshed at most once a minute.
    cc: claude_code::Cache,
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

/// Resolution order: PAPERCOMPOSER_WORKSPACE, PAPERCOMPOSER_PROFILE,
/// config workspace_path, config profile_path, bundled demo workspace.
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
        papers: ws.map(|w| subdirs(&w.join("papers"))).unwrap_or_default(),
        publisher_titles: ws
            .map(|w| pc_core::journal::list(&w.join("publishers")).into_iter().map(|j| (j.id, j.meta.name)).collect())
            .unwrap_or_default(),
        publishers,
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
}

/// Judge one paragraph against the soft rules in rules.csv with a model.
async fn soft_check(State(s): State<Shared>, Json(b): Json<SoftCheckBody>) -> impl IntoResponse {
    let (prompt, rules) = {
        let l = s.loaded.read().await;
        let Some(p) = &l.profile else { return bad("profile not loaded") };
        let rules = softcheck::soft_rules(p, b.section.as_deref());
        let prompt = softcheck::build_prompt(&rules, &b.text, b.section.as_deref(), b.before.as_deref(), b.after.as_deref());
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
        match writing_complete(&s, b.engine, &prompt, extra).await {
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
    if !s.engine.read().await.configured() {
        let c = s.claude.read().await;
        if c.status().configured {
            return Json(EngineStatus { connected: true, name: format!("Claude · {}", c.model) });
        }
    }
    Json(s.engine.read().await.status().await)
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
    let local = s.engine.read().await;
    let c = s.claude.read().await.status();
    let cc = s.cc.get(f.fresh.is_some()).await;
    Json(serde_json::json!({
        "local": { "configured": local.configured(), "status": local.status().await },
        "subscription": { "configured": cc.ready, "status": cc },
        "api": { "configured": c.configured, "masked": c.masked, "source": c.source },
        "model": c.model,
        "models": c.models,
    }))
}

async fn claude_code_login(State(s): State<Shared>) -> impl IntoResponse {
    match claude_code::start_login() {
        Ok(()) => {
            s.cc.get(true).await;
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
    let r = writing_complete(&s, Some(b.engine), &prompt, None).await;
    let ms = t.elapsed().as_millis() as u64;
    match r {
        Ok(reply) => Json(serde_json::json!({ "ok": true, "ms": ms, "reply": reply.trim() })),
        Err(e) => Json(serde_json::json!({ "ok": false, "ms": ms, "error": if e == "no engine" { "尚未完成設定".to_string() } else { e } })),
    }
}

/// Which engine writes. None = automatic: the local model when configured,
/// otherwise Claude. Both get the same retrieved patterns and prompt, so the
/// voice rules hold whichever runs.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum EngineChoice {
    Local,
    /// Claude Code CLI on the claude.ai subscription.
    Subscription,
    /// Anthropic API with a key (billed per use).
    #[serde(alias = "claude")]
    Api,
}

async fn engine_ready(s: &Shared, e: EngineChoice) -> bool {
    match e {
        EngineChoice::Local => s.engine.read().await.configured(),
        EngineChoice::Subscription => s.cc.get(false).await.ready,
        EngineChoice::Api => s.claude.read().await.status().configured,
    }
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

async fn writing_complete(s: &Shared, choice: Option<EngineChoice>, prompt: &Prompt, extra: Option<&str>) -> Result<String, String> {
    match resolve_engine(s, choice).await.ok_or("no engine")? {
        EngineChoice::Local => s.engine.read().await.complete(prompt, extra).await,
        EngineChoice::Subscription => {
            let model = s.claude.read().await.model.clone();
            claude_code::complete(prompt, &model, extra).await
        }
        EngineChoice::Api => {
            let c = s.claude.read().await;
            match extra {
                Some(e) => c.complete(&Prompt { system: prompt.system.clone(), user: format!("{}\n{e}", prompt.user) }).await,
                None => c.complete(prompt).await,
            }
        }
    }
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
        let reply = match writing_complete(&s, choice, &prompt, feedback.as_deref()).await {
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
    let l = s.loaded.read().await;
    let ws = l.workspace.clone().ok_or_else(|| bad("no workspace"))?;
    let dir = ws.join("papers").join(name);
    if !dir.is_dir() {
        return Err((StatusCode::NOT_FOUND, "paper not found").into_response());
    }
    Ok(dir)
}

#[derive(Deserialize)]
struct NewPaper {
    name: String,
    /// A folder under publishers/; none = blank article.
    #[serde(default)]
    template: Option<String>,
}

async fn create_paper(State(s): State<Shared>, Json(b): Json<NewPaper>) -> impl IntoResponse {
    let Some(ws) = s.loaded.read().await.workspace.clone() else {
        return bad("no workspace");
    };
    let made = match b.template.as_deref().filter(|t| !t.is_empty()) {
        Some(t) if !paper::safe_name(t) => return bad("invalid template"),
        Some(t) => paper::create_paper_from_template(&ws.join("papers"), &b.name, &ws.join("publishers").join(t)),
        None => paper::create_paper(&ws.join("papers"), &b.name),
    };
    match made {
        Ok(_) => Json(serde_json::json!({ "name": b.name.trim() })).into_response(),
        Err(e) => bad(&e),
    }
}

#[derive(Deserialize)]
struct ImportQuery {
    name: String,
}

/// Import an uploaded LaTeX project (.zip, .tar.gz or .tex as the raw body).
async fn import_upload(State(s): State<Shared>, AxQuery(q): AxQuery<ImportQuery>, body: axum::body::Bytes) -> impl IntoResponse {
    import_bytes(&s, &q.name, &body).await
}

#[derive(Deserialize)]
struct ArxivImport {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

/// Import a paper's LaTeX source from arXiv by id or URL.
async fn import_arxiv(State(s): State<Shared>, Json(b): Json<ArxivImport>) -> impl IntoResponse {
    let Some(id) = import::arxiv_id(&b.id) else { return bad("看不懂這個 arXiv 編號") };
    let bytes = match import::fetch_arxiv(&id).await { Ok(b) => b, Err(e) => return bad(&e) };
    let name = b.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("arXiv {}", id.replace('/', "-")));
    import_bytes(&s, &name, &bytes).await
}

async fn import_bytes(s: &Shared, name: &str, bytes: &[u8]) -> axum::response::Response {
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return bad("no workspace") };
    let tmp = std::env::temp_dir().join(format!("papercomposer-import-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    let res = std::fs::create_dir_all(&tmp)
        .map_err(|e| e.to_string())
        .and_then(|_| import::unpack(bytes, &tmp))
        .and_then(|_| paper::import_paper(&ws.join("papers"), name, &tmp));
    let _ = std::fs::remove_dir_all(&tmp);
    match res {
        Ok(_) => Json(serde_json::json!({ "name": name.trim() })).into_response(),
        Err(e) => bad(&e),
    }
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
    let (mut entries, mut notes) = load_library(&ws);
    let (new, errors) = bib::parse(&b.bibtex);
    let (mut added, mut skipped) = (Vec::new(), Vec::new());
    for e in new {
        let doi = e.get("doi").unwrap_or("").to_lowercase();
        let dup = entries.iter().any(|x| x.key == e.key || (!doi.is_empty() && x.get("doi").is_some_and(|d| d.to_lowercase() == doi)));
        if dup {
            skipped.push(e.key.clone());
            continue;
        }
        notes.push(bib::Note { key: e.key.clone(), doi: e.get("doi").unwrap_or("").into(), ..Default::default() });
        added.push(e.key.clone());
        entries.push(e);
    }
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
        Some(p) => match claude::open_in_browser(&p.to_string_lossy()) {
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
        match writing_complete(&s, b.engine, &prompt, None).await {
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

/// Find papers: Claude writes the queries, OpenAlex returns real works (all
/// with DOIs, no preprint-only ones), Claude ranks them and says why.
async fn search_refs(State(s): State<Shared>, Json(b): Json<SearchBody>) -> impl IntoResponse {
    if b.engine == EngineChoice::Local || !engine_ready(&s, b.engine).await {
        return bad("搜尋需要連接 Claude");
    }
    if b.request.trim().is_empty() {
        return bad("寫下要找什麼");
    }
    let ws = match refs_root(&s).await { Ok(w) => w, Err(r) => return r };
    let profile = s.loaded.read().await.path.clone();
    let tag_list = bib::read_tags(&profile.join("tags.csv"));
    let labels: Vec<String> = b.tags.iter().map(|t| tag_list.iter().find(|x| &x.id == t).map(|x| format!("{} ({})", x.label, x.id)).unwrap_or(t.clone())).collect();
    let steer = refimport::Steer::from_tags(&b.tags);

    let queries = match writing_complete(&s, Some(b.engine), &refimport::queries_prompt(&b.request, &labels), None).await {
        Ok(r) => refimport::parse_queries(&r),
        Err(e) => return Json(serde_json::json!({ "error": e })).into_response(),
    };
    if queries.is_empty() {
        return Json(serde_json::json!({ "error": "Claude 沒有產生搜尋關鍵字" })).into_response();
    }
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
    let ranked = match writing_complete(&s, Some(b.engine), &refimport::rank_prompt(&b.request, &cands), None).await {
        Ok(r) => refimport::apply_ranking(&r, &cands).unwrap_or_else(|| cands.iter().take(8).cloned().collect()),
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
    match claude::open_in_browser(&url) {
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
    let Some(e) = bib::parse(&b.bibtex).0.into_iter().next() else { return bad("BibTeX 無法讀取") };
    let (mut entries, mut notes) = load_library(&ws);
    let doi = e.get("doi").unwrap_or("").to_string();
    if entries.iter().any(|x| x.key == e.key || (!doi.is_empty() && x.get("doi").is_some_and(|d| d.eq_ignore_ascii_case(&doi)))) {
        return bad("文獻庫已經有這篇");
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
    let tex: String = std::iter::once(dir.join("main.tex"))
        .chain(paper::section_files(&dir).into_iter().map(|f| dir.join("sections").join(f)))
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect::<Vec<_>>()
        .join("\n");
    // entries in the paper's own .bib files, for keys the library lacks
    let main = std::fs::read_to_string(dir.join("main.tex")).unwrap_or_default();
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
    Json(serde_json::json!({ "cited": bib::cited_keys(&tex), "purposes": bib::read_citations(&dir.join("citations.csv")), "local": local })).into_response()
}

#[derive(Deserialize)]
struct Purpose {
    key: String,
    purpose: String,
}

async fn set_purpose(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<Purpose>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let path = dir.join("citations.csv");
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
    let main = std::fs::read_to_string(dir.join("main.tex")).unwrap_or_default();
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
    let tex: String = paper::section_files(dir)
        .into_iter()
        .filter_map(|f| std::fs::read_to_string(dir.join("sections").join(f)).ok())
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
    paper::write_atomic(&dir.join("main.tex"), &(lines.join("\n") + "\n")).map_err(|e| e.to_string())
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
    let main = std::fs::read_to_string(dir.join("main.tex")).unwrap_or_default();
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
            let main = std::fs::read_to_string(dir.join("main.tex")).unwrap_or_default();
            let (front, back) = preview::front_back(&main);
            Json(serde_json::json!({
            "sections": paper::load_paper(&dir),
            "macros": paper::paper_macros(&dir),
            "numbers": preview::numbers(&dir, &export::cache_dir(&name)),
            "front": front,
            "back": back,
            "fields": pc_core::front::fields(&main),
            "bibliography": preview::bibliography(&dir, &export::cache_dir(&name)),
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
    } else if let Some((k, _)) = claude::key() {
        convert::Auth::ApiKey(k)
    } else {
        return bad("模板轉換需要 Claude：在「設定 → Claude」登入帳號或設定 API key");
    };
    let model = b.model.filter(|m| m.starts_with("claude-")).unwrap_or_else(|| "claude-sonnet-5-5".into());
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
    let path = dir.join("main.tex");
    let main = match std::fs::read_to_string(&path) { Ok(m) => m, Err(e) => return bad(&e.to_string()) };
    let out = match pc_core::front::set_field(&main, &b.field, b.index, &b.value) { Ok(o) => o, Err(e) => return bad(&e) };
    if let Err(e) = paper::write_atomic(&path, &out) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let (front, back) = preview::front_back(&out);
    Json(serde_json::json!({ "front": front, "back": back, "fields": pc_core::front::fields(&out) })).into_response()
}

#[derive(Deserialize)]
struct FigureQuery {
    path: String,
}

/// A figure as an image the editor can show (PDF and EPS figures converted).
async fn paper_figure(State(s): State<Shared>, AxPath(name): AxPath<String>, AxQuery(q): AxQuery<FigureQuery>) -> impl IntoResponse {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(file) = preview::resolve_figure(&dir, &q.path) else { return (StatusCode::NOT_FOUND, "no such figure").into_response() };
    let cache = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer/figures");
    match preview::figure_image(&file, &cache).await {
        Some((img, mime)) => match std::fs::read(img) {
            Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, mime), (axum::http::header::CACHE_CONTROL, "no-cache")], bytes).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        None => (StatusCode::UNSUPPORTED_MEDIA_TYPE, "cannot show this figure").into_response(),
    }
}

#[derive(Deserialize)]
struct SectionBody {
    title: String,
    paragraphs: Vec<String>,
}

async fn put_section(
    State(s): State<Shared>,
    AxPath((name, file)): AxPath<(String, String)>,
    Json(b): Json<SectionBody>,
) -> impl IntoResponse {
    if !paper::safe_name(&file) || !file.ends_with(".tex") {
        return bad("invalid section file");
    }
    let dir = match paper_dir(&s, &name).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let path = dir.join("sections").join(&file);
    // Keep what follows \section{} (e.g. \label) from the file on disk.
    let tail = std::fs::read_to_string(&path).map(|t| paper::parse_section(&file, &t).tail).unwrap_or_default();
    let section = Section { file: file.clone(), title: b.title, paragraphs: b.paragraphs, tail };
    if let Err(e) = std::fs::create_dir_all(dir.join("sections")).and_then(|_| paper::write_atomic(&path, &paper::render_section(&section))) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    // Return what is now on disk, re-read, so the UI shows the saved form.
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    Json(paper::parse_section(&file, &text)).into_response()
}

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
    Json(s.claude.read().await.status())
}

/// Pages the UI may open in the system browser. Fixed URLs only, so the
/// local API cannot be used to open arbitrary links.
const PAGES: &[(&str, &str)] = &[
    ("anthropic-keys", claude::KEYS_PAGE),
    // GitHub pre-fills the form; the user still confirms on the page.
    ("github-new-repo", "https://github.com/new?name=my-paper-workspace&visibility=private"),
];

async fn open_page(AxPath(name): AxPath<String>) -> impl IntoResponse {
    let Some((_, url)) = PAGES.iter().find(|(n, _)| *n == name) else {
        return (StatusCode::NOT_FOUND, "unknown page").into_response();
    };
    match claude::open_in_browser(url) {
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
        if let Err(e) = claude::save_key(k) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("無法存入系統金鑰圈：{e}")).into_response();
        }
    }
    if let Some(m) = b.model.filter(|m| claude::MODELS.contains(&m.as_str())) {
        if let Err(e) = write_config_value("claude", "model", &m) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
        s.claude.write().await.model = m;
    }
    Json(s.claude.read().await.status()).into_response()
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
    let reply = match writing_complete(&s, b.engine, &prompt, None).await {
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

// ── Git ────────────────────────────────────────────────

async fn workspace_root(s: &Shared) -> Option<PathBuf> {
    let l = s.loaded.read().await;
    if l.demo { None } else { l.workspace.clone() }
}

async fn git_status(State(s): State<Shared>) -> impl IntoResponse {
    match workspace_root(&s).await {
        Some(ws) => Json(git::status(&ws).await).into_response(),
        None => Json(git::GitStatus::default()).into_response(),
    }
}

async fn git_pull(State(s): State<Shared>) -> impl IntoResponse {
    let Some(ws) = workspace_root(&s).await else { return Json(serde_json::json!({ "pulled": false })).into_response() };
    match git::pull(&ws).await {
        Ok(pulled) => {
            if pulled {
                *s.loaded.write().await = load(&s.paths);
            }
            Json(serde_json::json!({ "pulled": pulled })).into_response()
        }
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

#[derive(Deserialize, Default)]
struct SyncBody {
    message: Option<String>,
}

async fn git_sync(State(s): State<Shared>, body: Option<Json<SyncBody>>) -> impl IntoResponse {
    let Some(ws) = workspace_root(&s).await else { return bad("示範工作區不做同步") };
    let msg = body.and_then(|Json(b)| b.message);
    match git::sync(&ws, msg).await {
        Ok(r) => {
            if r.pulled {
                *s.loaded.write().await = load(&s.paths);
            }
            Json(r).into_response()
        }
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
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
        (Some(r), false) => Some(git::status(r).await),
        _ => None,
    };
    let sys = models::system_info().await;
    let cat = models::catalog();
    let recommended = models::recommend(&cat, &sys);
    let pool = sys.memory_gb.max(sys.gpu_memory_gb.unwrap_or(0.0));
    let installed: Vec<String> = models::installed().into_iter().map(|m| m.id).collect();
    let running = s.models.runtime.lock().await.as_ref().map(|r| r.model_id.clone());
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
        "models": cat.models.iter().map(|m| serde_json::json!({
            "entry": m,
            "fits": pool + 0.5 >= m.min_memory_gb as f32,
            "installed": installed.contains(&m.id),
        })).collect::<Vec<_>>(),
        "recommended": recommended,
        "runtime_installed": models::runtime_binary().is_some(),
        "running": running,
        "phase": s.models.phase.lock().await.clone(),
        "engine": s.engine.read().await.status().await,
        "claude": s.claude.read().await.status(),
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

async fn model_plan(Json(b): Json<ModelBody>) -> impl IntoResponse {
    match models::plan(&b.id).await {
        Ok(p) => Json(p).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

async fn activate(s: &Shared, id: &str) -> Result<(), String> {
    let url = models::start(&s.models, id).await?;
    let name = models::catalog().models.into_iter().find(|m| m.id == id).map(|m| format!("{} {}", m.name, m.variant)).unwrap_or(id.into());
    *s.engine.write().await = Engine::new(EngineConfig { url: Some(url), model: Some(name) });
    write_config_value("model", "active", id)
}

/// Re-plans on the server (the client never supplies URLs), downloads, then starts.
async fn model_install(State(s): State<Shared>, Json(b): Json<ModelBody>) -> impl IntoResponse {
    if s.models.phase.lock().await.as_str() == "downloading" {
        return (StatusCode::CONFLICT, "已有下載進行中").into_response();
    }
    let plan = match models::plan(&b.id).await {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
    };
    s.models.progress.lock().await.clear();
    let s2 = s.clone();
    tokio::spawn(async move {
        if models::install(s2.models.clone(), plan).await.is_ok() {
            if let Err(e) = activate(&s2, &b.id).await {
                *s2.models.phase.lock().await = format!("error: {e}");
            }
        }
    });
    StatusCode::ACCEPTED.into_response()
}

async fn model_progress(State(s): State<Shared>) -> impl IntoResponse {
    let files: Vec<models::Progress> = s.models.progress.lock().await.values().cloned().collect();
    let running = s.models.runtime.lock().await.as_ref().map(|r| (r.model_id.clone(), r.url.clone()));
    Json(serde_json::json!({ "phase": s.models.phase.lock().await.clone(), "files": files, "running": running }))
}

async fn model_start(State(s): State<Shared>, Json(b): Json<ModelBody>) -> impl IntoResponse {
    match activate(&s, &b.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

async fn model_stop(State(s): State<Shared>) -> impl IntoResponse {
    models::stop(&s.models).await;
    *s.models.phase.lock().await = "idle".into();
    *s.engine.write().await = Engine::new(EngineConfig::resolve(read_config().engine));
    StatusCode::NO_CONTENT
}

pub fn router(paths: Paths) -> Router {
    let cfg = read_config();
    let engine_cfg = EngineConfig::resolve(cfg.engine);
    let external = engine_cfg.url.is_some();
    let engine = Engine::new(engine_cfg);
    let claude = Claude::new(cfg.claude.and_then(|c| c.model));
    let state: Shared = Arc::new(AppState {
        loaded: RwLock::new(load(&paths)),
        engine: RwLock::new(engine),
        models: Arc::new(models::Manager::default()),
        claude: RwLock::new(claude),
        paths: paths.clone(),
        soft_cache: Default::default(),
        cc: Default::default(),
    });
    models::register(state.models.clone());
    // Start the chosen local model with the app, unless an external engine is set.
    if let (false, Some(id)) = (external, cfg.model.and_then(|m| m.active)) {
        if models::installed().iter().any(|m| m.id == id) {
            let s = state.clone();
            tokio::spawn(async move {
                if let Err(e) = activate(&s, &id).await {
                    eprintln!("local model: {e}");
                }
            });
        }
    }
    Router::new()
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
        .route("/api/models/catalog", get(|| async { Json(models::catalog()) }))
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
        .route("/api/papers/{name}/export", post(export_paper))
        .route("/api/papers/{name}/figure", get(paper_figure))
        .route("/api/papers/{name}/front", put(put_front))
        .route("/api/templates", get(list_templates).post(create_template))
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
        .route("/api/settings/pdf-dir", post(set_pdf_dir))
        .route("/api/papers/{name}/sections/{file}", put(put_section))
        .route("/api/tables/{name}", get(get_table).put(put_table))
        .route("/api/claude", get(claude_status))
        .route("/api/open/{page}", post(open_page))
        .route("/api/settings/claude", post(claude_settings))
        .route("/api/decompose", post(decompose_text))
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
        .route("/api/engines", get(engines_status))
        .route("/api/engines/test", post(test_engine))
        .route("/api/claude-code/login", post(claude_code_login))
        .route("/api/compose", post(compose_paragraph))
        .fallback_service(ServeDir::new(paths.ui))
        .with_state(state)
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
