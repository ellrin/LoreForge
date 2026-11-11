//! 功能列表 (design §4, contract §4a): 整理成筆記, 做成知識卡, 記下靈感. Every button drafts a
//! preview first; nothing is written until the author confirms it (the `…/save` calls).
//!
//! | method | path | what |
//! |---|---|---|
//! | POST | /api/actions/note/draft | `{ messages, character?, engine? }` → `{ title, body, tags, kb, findings, engine, label }` (task `le-note`) |
//! | POST | /api/actions/note/check | `{ body }` → `{ findings, blocking }` (lore-notation, while the author edits) |
//! | POST | /api/actions/note/save | `{ title, body, kind: note\|idea, tags, kb }` → `{ via: pc\|file, id, path, findings }`; 422 `{ error, findings }` when a finding blocks |
//! | POST | /api/actions/cards/draft | `{ messages, character?, engine? }` → `{ cards: [{ title, summary, body, tags, links: [{ to, rel, title }] }], engine, label }` (task `le-cards`) |
//! | POST | /api/actions/cards/save | `{ cards, topic? }` → `{ via: kb\|file, ids, space? }` (KnowledgeBase's 待審 only, never its cards) |
//!
//! Where it goes: PaperComposer's `POST /api/notes` while it runs (its port in ports.json, else
//! 7878; it counts as running when `GET /api/notes` answers), else the note file in
//! `private/PaperComposer/notes/` ([`write_note_file`]); KnowledgeBase's `POST /api/pending`
//! while it runs, else kb-core's `pending::add` in the member's personal space ([`write_pending`]). Once the other
//! app answered, its refusal is shown and nothing is written here (no second copy).
//! The conversation itself is never written: a pending card keeps only a one-line topic.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use kb_core::{Clock, SystemClock};
use lore_llm::{LlmRequest, Message};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Shared;
use crate::chat::{Turn, clip};

pub const TASK_NOTE: &str = "le-note";
pub const TASK_CARDS: &str = "le-cards";
/// PaperComposer's folder under `private/`, and its port in browser mode.
const PC_APP: &str = "PaperComposer";
const PC_DEFAULT_PORT: u16 = 7878;
/// The conversation given to the model (characters, the newest kept). Chinese runs about one
/// token a character: this leaves room for the rules and the reply in a 16k local model.
pub const MAX_CONVERSATION: usize = 10_000;
/// Existing cards offered to the model as links / related cards.
const CANDIDATES: usize = 8;
/// Cards per draft.
pub const MAX_CARDS: usize = 6;
/// A title (characters).
const TITLE_CHARS: usize = 80;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/actions/note/draft", post(note_draft))
        .route("/api/actions/note/check", post(note_check))
        .route("/api/actions/note/save", post(note_save))
        .route("/api/actions/cards/draft", post(cards_draft))
        .route("/api/actions/cards/save", post(cards_save))
}

// ── PaperComposer (like kbctx's KnowledgeBase: pinned in tests, the machine's in `system()`) ──

#[derive(Debug, Default)]
pub struct Pc {
    system: AtomicBool,
    /// `Some(url)` = exactly this server (`None` inside = not running).
    pin: RwLock<Option<Option<String>>>,
}

impl Pc {
    pub fn use_system(&self) {
        self.system.store(true, Ordering::Relaxed);
    }

    /// Tests: PaperComposer at `url` (`None` = not running).
    pub fn pin(&self, url: Option<String>) {
        *self.pin.write().unwrap_or_else(|e| e.into_inner()) = Some(url);
    }

    /// PaperComposer's server when it runs and has notes (`GET /api/notes` answers).
    pub async fn server(&self) -> Option<String> {
        if let Some(p) = self.pin.read().unwrap_or_else(|e| e.into_inner()).clone() {
            return p;
        }
        if !self.system.load(Ordering::Relaxed) {
            return None;
        }
        let mut ports = vec![];
        if let Some(p) = lore_home::ports::live().get("pc") {
            ports.push(*p);
        }
        ports.push(PC_DEFAULT_PORT);
        for port in ports {
            let url = format!("http://127.0.0.1:{port}");
            if let Ok(r) = client().get(format!("{url}/api/notes")).timeout(Duration::from_millis(800)).send().await
                && r.status().is_success()
            {
                return Some(url);
            }
        }
        None
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).no_proxy().build().unwrap_or_default()
}

fn bad(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

// ── the conversation for the model ──

#[derive(Debug, Clone, Deserialize)]
pub struct DraftBody {
    #[serde(default)]
    pub messages: Vec<Turn>,
    pub character: Option<String>,
    pub engine: Option<String>,
}

/// The page's turns as one labelled text (「我：…」「<name>：…」), the newest
/// [`MAX_CONVERSATION`] characters. `Err` when nobody has said anything yet.
pub fn conversation(turns: &[Turn], name: &str) -> Result<String, String> {
    let mut lines: Vec<String> = vec![];
    for t in turns {
        let text = t.text.trim();
        if text.is_empty() {
            continue;
        }
        let who = match t.role.as_str() {
            "user" => "我",
            "assistant" => name,
            _ => continue,
        };
        lines.push(format!("{who}：{text}"));
    }
    if !turns.iter().any(|t| t.role == "user" && !t.text.trim().is_empty()) {
        return Err("還沒有對話可以整理".into());
    }
    let mut out: Vec<String> = vec![];
    let mut n = 0;
    for l in lines.into_iter().rev() {
        let c = l.chars().count();
        if n + c > MAX_CONVERSATION && !out.is_empty() {
            break;
        }
        n += c;
        out.push(if c > MAX_CONVERSATION { clip(&l, MAX_CONVERSATION) } else { l });
    }
    out.reverse();
    Ok(out.join("\n\n"))
}

/// The author's cards this conversation touches (titles, aliases, key words), read from disk.
async fn candidates(s: &Shared, turns: &[Turn]) -> Vec<crate::kbctx::Card> {
    let (spaces, member) = s.kb.locate(&s.home);
    if spaces.is_empty() {
        return vec![];
    }
    let all: String = turns.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join("\n");
    let one = vec![Turn { role: "user".into(), text: all }];
    tokio::task::spawn_blocking(move || {
        let cards = crate::kbctx::load(&spaces, &member, &SystemClock);
        crate::kbctx::retrieve(&cards, &one, CANDIDATES)
    })
    .await
    .unwrap_or_default()
}

fn candidate_lines(cards: &[crate::kbctx::Card]) -> String {
    cards.iter().map(|c| format!("- {}：{}（{}）", c.id, c.title, clip(&c.summary, 80))).collect::<Vec<_>>().join("\n")
}

/// Run `task` for one JSON value. `Err` = the response for the page (503 with the
/// alternative engine when none can answer, 502 when the call failed).
async fn ask_json(s: &Shared, task: &str, engine: Option<&str>, system: String, user: String, schema: Value) -> Result<(Value, String), Response> {
    let route = match s.llm.route(task, engine).await {
        Ok(r) => r,
        Err(payload) => return Err((StatusCode::SERVICE_UNAVAILABLE, Json(payload)).into_response()),
    };
    let id = route.provider.id();
    let req = route.apply(LlmRequest::chat(system, vec![Message::user(user)]).json(schema).temperature(0.3));
    match lore_llm::complete_json(route.provider.as_ref(), &req).await {
        Ok(v) => Ok((v, id)),
        Err(e) => {
            let msg = format!("模型沒有整理好：{e}");
            Err(bad(StatusCode::BAD_GATEWAY, msg.lines().next().unwrap_or("").trim()))
        }
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

// ── lore-notation ──

/// One finding for the preview.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub code: String,
    /// error | warning | info; `error` blocks the save.
    pub severity: String,
    pub message: String,
}

/// The note's LaTeX through lore-notation's checks (an empty registry: a note has no paper of
/// its own). Errors (a `\ref` to a missing label, a label twice) block the save.
pub fn check(body: &str) -> Vec<Finding> {
    use lore_notation::{CheckOptions, Document, Notation, check_document};
    let r = check_document(Document::from_str("note.tex", body), &Notation::default(), &CheckOptions::default());
    r.issues
        .into_iter()
        // a short note uses words and symbols without the paper's definitions: those are not
        // worth the author's attention here
        .filter(|i| !matches!(i.category.as_str(), "registry") && i.code != "unused_definition")
        .map(|i| Finding { code: i.code, severity: format!("{:?}", i.severity).to_lowercase(), message: i.message })
        .collect()
}

pub fn blocking(f: &[Finding]) -> bool {
    f.iter().any(|x| x.severity == "error")
}

// ── 整理成筆記 ──

const NOTE_RULES: &str = "你把一段對話整理成一篇 LaTeX 筆記，給使用者之後放進論文或自己複習。
內文用台灣繁體中文，結構固定四節，依序：
\\section*{問題}（這段對話想弄清楚什麼）
\\section*{討論過程}（重點、推理、例子，條列可以用 itemize）
\\section*{結論}（目前的答案）
\\section*{待查}（還沒解決或要查證的事，itemize；沒有就寫「無」）
規則：只寫內文，不要 \\documentclass、\\begin{document} 或前言；數學用 $…$ 或 \\[…\\]；
要提到筆記裡的方程式時先 \\label{eq:…} 再用 \\ref{eq:…}，絕不打「式 1」「Table 1」「圖 2」這種字；
不要 \\label 用不到的東西；不要 Markdown。
回一個 JSON：{\"title\": 簡短標題, \"body\": LaTeX 內文, \"tags\": [幾個標籤], \"kb\": [用到的知識卡 id]}。";

fn note_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "body": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "kb": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["title", "body"]
    })
}

async fn note_draft(State(s): State<Shared>, body: Option<Json<DraftBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { messages }") };
    let ch = s.character(b.character.as_deref());
    let text = match conversation(&b.messages, &ch.name) {
        Ok(t) => t,
        Err(e) => return bad(StatusCode::BAD_REQUEST, e),
    };
    let cands = candidates(&s, &b.messages).await;
    let mut system = NOTE_RULES.to_string();
    if !cands.is_empty() {
        system.push_str(&format!("\n\n使用者自己的知識卡裡和這段對話有關的（kb 只能填這些 id）：\n{}", candidate_lines(&cands)));
    }
    let (v, engine) = match ask_json(&s, TASK_NOTE, b.engine.as_deref(), system, format!("對話：\n\n{text}"), note_schema()).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let body = v["body"].as_str().unwrap_or_default().trim().to_string();
    let kb: Vec<String> = strings(&v["kb"]).into_iter().filter(|id| cands.iter().any(|c| &c.id == id)).collect();
    let findings = check(&body);
    Json(json!({
        "title": clip(v["title"].as_str().unwrap_or_default(), TITLE_CHARS),
        "body": body,
        "tags": strings(&v["tags"]),
        "kb": kb,
        "kb_titles": kb.iter().filter_map(|id| cands.iter().find(|c| &c.id == id).map(|c| c.title.clone())).collect::<Vec<_>>(),
        "blocking": blocking(&findings),
        "findings": findings,
        "label": crate::llm::engine_label(&engine),
        "engine": engine,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct CheckBody {
    #[serde(default)]
    body: String,
}

async fn note_check(Json(b): Json<CheckBody>) -> Json<Value> {
    let f = check(&b.body);
    Json(json!({ "blocking": blocking(&f), "findings": f }))
}

/// A note or an idea as the author confirmed it (contract §4a).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Note {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// note | idea
    #[serde(default = "note_kind")]
    pub kind: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub kb: Vec<String>,
    #[serde(default = "loreecho")]
    pub source: String,
}

fn note_kind() -> String {
    "note".into()
}
fn loreecho() -> String {
    "loreecho".into()
}

impl Note {
    /// Trimmed, one-line title (an idea without one: its first words), known kind,
    /// tags without commas.
    pub fn clean(mut self) -> Result<Note, String> {
        self.kind = if self.kind == "idea" { "idea".into() } else { "note".into() };
        self.body = self.body.trim().to_string();
        if self.body.is_empty() {
            return Err("內容是空的".into());
        }
        let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        self.title = clip(&one_line(&self.title), TITLE_CHARS);
        if self.title.is_empty() {
            self.title = clip(&one_line(&self.body), 30);
        }
        self.tags = self.tags.iter().map(|t| one_line(&t.replace(',', " "))).filter(|t| !t.is_empty()).collect();
        self.kb.retain(|id| kb_core::id::valid_node_id(id));
        self.source = "loreecho".into();
        Ok(self)
    }
}

/// 記下靈感 is typed as plain text: the characters that would break LaTeX are escaped
/// (`$…$` math stays as typed).
pub fn idea_tex(line: &str) -> String {
    let mut out = String::new();
    let mut prev = '\0';
    for c in line.trim().chars() {
        if matches!(c, '%' | '&' | '#') && prev != '\\' {
            out.push('\\');
        }
        out.push(c);
        prev = c;
    }
    out
}

/// `% ---` … `% ---` (contract §4a: source, created, kind, kb, tags, title).
pub fn front_matter(n: &Note, created: &str) -> String {
    format!(
        "% ---\n% source: {}\n% created: {created}\n% kind: {}\n% kb: {}\n% tags: {}\n% title: {}\n% ---\n",
        n.source,
        n.kind,
        n.kb.join(", "),
        n.tags.join(", "),
        n.title
    )
}

/// The file name's slug: letters and digits of the title (any script), the rest as `-`.
pub fn slug(title: &str, kind: &str) -> String {
    let mut s = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s: String = s.trim_matches('-').chars().take(40).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { kind.to_string() } else { s }
}

/// The note as a file in PaperComposer's notes folder, through `lore-notes` (the format
/// PaperComposer reads: `<notes>/<YYYY-MM-DD>-<slug>.tex`, never over another file: `-2`, `-3`, …),
/// checked against the author's registry first. Returns `(id, path)`.
pub fn write_note_file(notes: &Path, n: &Note, now: &kb_core::clock::Time, notation: &lore_notation::Notation) -> Result<(String, PathBuf), String> {
    let new = lore_notes::NewNote {
        title: n.title.clone(),
        body: n.body.clone(),
        kind: if n.kind == "idea" { lore_notes::Kind::Idea } else { lore_notes::Kind::Note },
        tags: n.tags.clone(),
        kb: n.kb.clone(),
        source: Some(n.source.clone()),
    };
    match lore_notes::Notes::new(notes).create_at(new, notation, *now) {
        Ok(saved) => Ok((saved.id, saved.path)),
        Err(lore_notes::Error::Blocked(f)) => {
            let why: Vec<&str> = f.iter().filter(|x| x.blocking).map(|x| x.message.as_str()).collect();
            Err(format!("記號檢查沒過，沒有存：{}", why.join("；")))
        }
        Err(e) => Err(format!("筆記沒有寫好：{e}")),
    }
}

async fn note_save(State(s): State<Shared>, body: Option<Json<Note>>) -> Response {
    let Some(Json(n)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { title, body, kind }") };
    let mut n = match n.clean() {
        Ok(n) => n,
        Err(e) => return bad(StatusCode::BAD_REQUEST, e),
    };
    if n.kind == "idea" {
        n.body = idea_tex(&n.body);
    }
    if let Some(url) = s.pc.server().await {
        return match client().post(format!("{url}/api/notes")).json(&n).send().await {
            Ok(r) => {
                let status = r.status();
                let v: Value = r.json().await.unwrap_or(Value::Null);
                if status.is_success() {
                    Json(json!({ "via": "pc", "id": v["id"], "path": v["path"], "findings": v.get("findings").cloned().unwrap_or(json!([])) })).into_response()
                } else if status == StatusCode::UNPROCESSABLE_ENTITY {
                    let findings = v.get("findings").or(v.get("issues")).cloned().unwrap_or(json!([]));
                    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": "PaperComposer 的記號檢查沒過，沒有存", "findings": findings }))).into_response()
                } else {
                    bad(StatusCode::BAD_GATEWAY, format!("PaperComposer 沒有存：{}", v["error"].as_str().unwrap_or(status.as_str())))
                }
            }
            Err(e) => bad(StatusCode::BAD_GATEWAY, format!("PaperComposer 沒有回應：{e}")),
        };
    }
    let findings = check(&n.body);
    if blocking(&findings) {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": "記號檢查沒過，沒有存", "findings": findings }))).into_response();
    }
    let Some(notes) = s.home.app_dir(PC_APP).map(|d| d.join("notes")) else { return bad(StatusCode::SERVICE_UNAVAILABLE, "沒有 LoreForge 的 private 資料夾，無法存筆記") };
    let now = SystemClock.now();
    let notation = s.home.shared_file("notation.json").and_then(|p| lore_notation::Notation::load(&p).ok()).unwrap_or_default();
    match tokio::task::spawn_blocking(move || write_note_file(&notes, &n, &now, &notation)).await {
        Ok(Ok((id, path))) => Json(json!({ "via": "file", "id": id, "path": path, "findings": findings })).into_response(),
        Ok(Err(e)) => bad(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── 做成知識卡 ──

const CARDS_RULES: &str = "你從一段對話裡挑出值得記成知識卡的概念，給使用者審過再收進他的知識庫。
每張卡一個概念，1 到 5 張，只挑對話裡真的講清楚的；使用者已經有的卡（下面列出）不要重做，可以連過去。
欄位：title（概念名稱，短）、summary（一句話）、body（幾句說明，可以用 $…$ 寫數學，不要 Markdown 標題）、
tags（幾個標籤）、links（相關的既有卡 id，只能用下面列出的）。用台灣繁體中文（術語可以保留英文）。
回一個 JSON：{\"cards\": [ … ]}。";

fn cards_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cards": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "summary": { "type": "string" },
                        "body": { "type": "string" },
                        "tags": { "type": "array", "items": { "type": "string" } },
                        "links": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["title"]
                }
            }
        },
        "required": ["cards"]
    })
}

/// A link to an existing card (kb-core's `{to, rel}`; `title` only for the preview).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardLink {
    pub to: String,
    #[serde(default = "related")]
    pub rel: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
}

fn related() -> String {
    "related".into()
}

/// A candidate card (contract §4a: `{title, summary, body, tags, links, kind, source}`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardDraft {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<CardLink>,
    #[serde(default = "concept")]
    pub kind: String,
    #[serde(default = "loreecho")]
    pub source: String,
}

fn concept() -> String {
    "concept".into()
}

async fn cards_draft(State(s): State<Shared>, body: Option<Json<DraftBody>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { messages }") };
    let ch = s.character(b.character.as_deref());
    let text = match conversation(&b.messages, &ch.name) {
        Ok(t) => t,
        Err(e) => return bad(StatusCode::BAD_REQUEST, e),
    };
    let cands = candidates(&s, &b.messages).await;
    let mut system = CARDS_RULES.to_string();
    system.push_str(&if cands.is_empty() { "\n\n使用者的知識庫裡沒有相關的卡（links 留空）。".to_string() } else { format!("\n\n使用者已有的相關卡：\n{}", candidate_lines(&cands)) });
    let (v, engine) = match ask_json(&s, TASK_CARDS, b.engine.as_deref(), system, format!("對話：\n\n{text}"), cards_schema()).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let cards: Vec<CardDraft> = v["cards"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let title = clip(c["title"].as_str().unwrap_or_default(), TITLE_CHARS);
            if title.is_empty() || cands.iter().any(|k| std::iter::once(&k.title).chain(k.aliases.iter()).any(|n| n.trim().eq_ignore_ascii_case(&title))) {
                return None; // nameless, or a card he already has
            }
            let mut links: Vec<CardLink> = vec![];
            for id in strings(&c["links"]) {
                if let Some(k) = cands.iter().find(|k| k.id == id)
                    && !links.iter().any(|l| l.to == id)
                {
                    links.push(CardLink { to: id, rel: related(), title: k.title.clone() });
                }
            }
            Some(CardDraft {
                title,
                summary: c["summary"].as_str().unwrap_or_default().trim().to_string(),
                body: c["body"].as_str().unwrap_or_default().trim().to_string(),
                tags: strings(&c["tags"]),
                links,
                kind: concept(),
                source: loreecho(),
            })
        })
        .take(MAX_CARDS)
        .collect();
    Json(json!({ "cards": cards, "label": crate::llm::engine_label(&engine), "engine": engine })).into_response()
}

#[derive(Deserialize)]
struct CardsSave {
    #[serde(default)]
    cards: Vec<CardDraft>,
    /// One line about the conversation (its first question), kept as `from_conversation`.
    #[serde(default)]
    topic: String,
}

/// The confirmed cards into `space`'s 待審 (kb-core `pending::add`: only `<space>/pending/` is
/// written; KnowledgeBase lists them under 待審 when it next opens). Returns the ids.
pub fn write_pending(space: &Path, cards: &[CardDraft], topic: &str, now: kb_core::clock::Time) -> Result<Vec<String>, String> {
    let space = lore_store::Space::open(space).map_err(|e| format!("打不開 KnowledgeBase 的空間：{e}"))?;
    let new: Vec<kb_core::pending::NewPending> = cards
        .iter()
        .map(|c| kb_core::pending::NewPending {
            title: c.title.clone(),
            kind: c.kind.clone(),
            tags: c.tags.clone(),
            summary: c.summary.clone(),
            body: c.body.clone(),
            links: c.links.iter().map(|l| kb_core::node::Link { to: l.to.clone(), rel: l.rel.clone(), extra: Default::default() }).collect(),
            source: c.source.clone(),
            from_conversation: (!topic.is_empty()).then(|| topic.to_string()),
            ..Default::default()
        })
        .collect();
    let added = kb_core::pending::add(&space, now, new).map_err(|e| format!("待審卡沒有寫好：{e}"))?;
    Ok(added.into_iter().map(|c| c.id).collect())
}

/// The member's personal space (else the first), as KnowledgeBase picks it for a write.
fn personal_space(s: &Shared) -> Option<PathBuf> {
    let (roots, member) = s.kb.locate(&s.home);
    let personal = roots.iter().find(|r| lore_store::Space::open(r.as_path()).is_ok_and(|sp| sp.kind() == lore_store::SpaceKind::Personal && sp.is_member(&member)));
    personal.or(roots.first()).cloned()
}

async fn cards_save(State(s): State<Shared>, body: Option<Json<CardsSave>>) -> Response {
    let Some(Json(b)) = body else { return bad(StatusCode::BAD_REQUEST, "要送 { cards }") };
    let cards: Vec<CardDraft> = b
        .cards
        .into_iter()
        .map(|mut c| {
            c.title = clip(&c.title.split_whitespace().collect::<Vec<_>>().join(" "), TITLE_CHARS);
            c.summary = c.summary.trim().to_string();
            c.body = c.body.trim().to_string();
            c.kind = if c.kind.trim().is_empty() { concept() } else { c.kind.trim().to_string() };
            c.source = loreecho();
            c.links.retain(|l| kb_core::id::valid_node_id(&l.to));
            c
        })
        .filter(|c| !c.title.is_empty())
        .collect();
    if cards.is_empty() {
        return bad(StatusCode::BAD_REQUEST, "沒有勾選的卡");
    }
    let topic = clip(&b.topic.split_whitespace().collect::<Vec<_>>().join(" "), 80);
    // what KnowledgeBase shows as where a pending card came from
    let source = if topic.is_empty() { "LoreEcho".to_string() } else { format!("LoreEcho：{topic}") };
    let cards: Vec<CardDraft> = cards.into_iter().map(|c| CardDraft { source: source.clone(), ..c }).collect();
    if let Some(url) = s.kb.server().await {
        let send: Vec<Value> = cards.iter().map(|c| json!({ "title": c.title, "summary": c.summary, "body": c.body, "tags": c.tags, "links": c.links.iter().map(|l| json!({ "to": l.to, "rel": l.rel })).collect::<Vec<_>>(), "kind": c.kind, "source": c.source, "from_conversation": topic })).collect();
        return match client().post(format!("{url}/api/pending")).json(&json!({ "cards": send })).send().await {
            Ok(r) => {
                let status = r.status();
                let v: Value = r.json().await.unwrap_or(Value::Null);
                if status.is_success() {
                    Json(json!({ "via": "kb", "ids": v["ids"] })).into_response()
                } else {
                    bad(StatusCode::BAD_GATEWAY, format!("KnowledgeBase 沒有收下：{}", v["error"].as_str().unwrap_or(status.as_str())))
                }
            }
            Err(e) => bad(StatusCode::BAD_GATEWAY, format!("KnowledgeBase 沒有回應：{e}")),
        };
    }
    let Some(space) = personal_space(&s) else { return bad(StatusCode::SERVICE_UNAVAILABLE, "找不到 KnowledgeBase") };
    let now = SystemClock.now();
    let sp = space.clone();
    match tokio::task::spawn_blocking(move || write_pending(&sp, &cards, &topic, now)).await {
        Ok(Ok(ids)) => Json(json!({ "via": "file", "ids": ids, "space": space })).into_response(),
        Ok(Err(e)) => bad(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(role: &str, text: &str) -> Turn {
        Turn { role: role.into(), text: text.into() }
    }

    #[test]
    fn the_conversation_is_labelled_and_keeps_the_newest() {
        let c = conversation(&[t("user", "什麼是梯度？"), t("assistant", "增加最快的方向。"), t("user", " ")], "Echo").unwrap();
        assert_eq!(c, "我：什麼是梯度？\n\nEcho：增加最快的方向。");
        assert!(conversation(&[t("assistant", "嗨")], "Echo").is_err());
        let long: Vec<Turn> = (0..40).map(|i| t("user", &format!("{i}{}", "字".repeat(1000)))).collect();
        let c = conversation(&long, "Echo").unwrap();
        assert!(c.chars().count() <= MAX_CONVERSATION + 100 && c.ends_with(&"字".repeat(10)) && c.contains("我：39"));
        assert!(!c.contains("我：0字"));
    }

    #[test]
    fn notation_checks_are_wired() {
        let f = check("如 Table 1 所示，$x$ 很大。");
        assert!(f.iter().any(|x| x.code == "hardcoded_reference"), "{f:?}");
        assert!(!blocking(&f));
        let f = check("見式 \\ref{eq:none}。");
        assert!(f.iter().any(|x| x.code == "ref_undefined" && x.severity == "error"), "{f:?}");
        assert!(blocking(&f));
        assert!(!blocking(&check("\\begin{equation}a=b\\label{eq:a}\\end{equation}\n由 \\ref{eq:a} 可知。")));
    }

    #[test]
    fn slugs_and_front_matter() {
        assert_eq!(slug("傅立葉轉換 與 Nyquist!", "note"), "傅立葉轉換-與-nyquist");
        assert_eq!(slug("??", "idea"), "idea");
        assert_eq!(slug("../etc/passwd", "note"), "etc-passwd");
        let n = Note { title: "標題".into(), body: "x".into(), kind: "idea".into(), tags: vec!["a".into(), "b".into()], kb: vec![], source: "loreecho".into() };
        assert_eq!(front_matter(&n, "T"), "% ---\n% source: loreecho\n% created: T\n% kind: idea\n% kb: \n% tags: a, b\n% title: 標題\n% ---\n");
    }

    #[test]
    fn clean_and_idea_escapes() {
        let n = Note { title: " ".into(), body: " 一句\n靈感 ".into(), kind: "weird".into(), tags: vec!["a,b".into(), " ".into()], kb: vec!["../x".into()], source: "x".into() }.clean().unwrap();
        assert_eq!((n.title.as_str(), n.kind.as_str(), n.tags.clone(), n.kb.len(), n.source.as_str()), ("一句 靈感", "note", vec!["a b".to_string()], 0, "loreecho"));
        assert_eq!(idea_tex("50% & #1 但 \\% 不變，$x^2$"), "50\\% \\& \\#1 但 \\% 不變，$x^2$");
        assert!(Note { title: "t".into(), body: "".into(), kind: "note".into(), tags: vec![], kb: vec![], source: String::new() }.clean().is_err());
    }

    #[test]
    fn note_files_never_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let now = SystemClock.now();
        let n = Note { title: "梯度".into(), body: "\\section*{問題}".into(), kind: "note".into(), tags: vec![], kb: vec![], source: "loreecho".into() };
        let reg = lore_notation::Notation::default();
        let (a, pa) = write_note_file(tmp.path(), &n, &now, &reg).unwrap();
        let (b, _) = write_note_file(tmp.path(), &n, &now, &reg).unwrap();
        assert_eq!(b, format!("{a}-2"));
        assert!(a.ends_with("-梯度") && a.starts_with(&now.format("%Y-%m-%d").to_string()));
        let text = std::fs::read_to_string(pa).unwrap();
        assert!(text.starts_with("% ---\n% source: loreecho\n% created: ") && text.ends_with("% ---\n\\section*{問題}\n"), "{text}");
    }
}
