//! Selected text from another Lore app (`lore_shell::selection`): the app's server hands L-Echo
//! the author's selection (`POST /api/selection`), the window opens, the page polls it and works
//! on it (改寫 through the app's own 語句潤飾 routes, proxied here; 翻譯/解釋/問他 through the chat
//! with a `selection` block; 加入知識庫 through the app's `/api/lore/card`), and a result goes back
//! to the app (`send_selected`), whose page applies it as one undo step. At most one request, in
//! memory only: a new one replaces the old (whose app is told `end`); nothing is written anywhere.
//!
//! | method | path | what |
//! |---|---|---|
//! | POST | /api/selection | a `SelectionRequest` from an app's server → kept, the window opens → `{ok}` |
//! | GET | /api/selection | `{open, request?, app?, doc?, source?, text?, scope?, language?, kind?, readonly?}` (the page polls) |
//! | POST | /api/selection/end | the app is told, the request is dropped |
//! | POST | /api/selection/apply `{after, polish?}` | → the app's page; ok → `{ok, message}` (and the app's `/api/polish/applied` when `polish`); the app's 409/410/504 pass through |
//! | POST/GET | /api/selection/polish, /polish/{applied,like,history,presets}, /lore/card | forwarded unchanged to the app's `/api/polish…`, `/api/lore/card` while a request is open (else 409) |
//! (`/lore/card` from a read-only selection carries `{title, source, doc, scope, context}`: L-Reader's `/api/lore/card` is its 一鍵加卡.)

use std::sync::Mutex;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_shell::selection::{MAX_CONTEXT, MAX_TEXT, SelectionRequest, end_selection, send_selected};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{DesktopEvent, Shared};

/// A model call on the app's side (語句潤飾, 加入知識庫 with its web search).
const SLOW: Duration = Duration::from_secs(180);
/// Everything else the app answers at once.
const QUICK: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct Selector {
    current: Mutex<Option<SelectionRequest>>,
}

impl Selector {
    fn current(&self) -> Option<SelectionRequest> {
        self.current.lock().unwrap().clone()
    }
}

pub fn routes() -> Router<Shared> {
    // POST: the body as it came; GET: the query
    macro_rules! fwd_post {
        ($path:expr, $t:expr) => {
            post(|s: State<Shared>, q: RawQuery, b: Bytes| proxy(s, Method::POST, $path, $t, q, b))
        };
    }
    macro_rules! fwd_get {
        ($path:expr) => {
            get(|s: State<Shared>, q: RawQuery| proxy(s, Method::GET, $path, QUICK, q, Bytes::new()))
        };
    }
    Router::new()
        .route("/api/selection", get(sel_get).post(sel_post))
        .route("/api/selection/end", post(end))
        .route("/api/selection/apply", post(apply))
        .route("/api/selection/polish", fwd_post!("/api/polish", SLOW))
        .route("/api/selection/polish/applied", fwd_post!("/api/polish/applied", QUICK))
        .route("/api/selection/polish/like", fwd_post!("/api/polish/like", QUICK))
        .route("/api/selection/polish/history", fwd_get!("/api/polish/history"))
        .route("/api/selection/polish/presets", fwd_get!("/api/polish/presets"))
        .route("/api/selection/lore/card", fwd_post!("/api/lore/card", SLOW))
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

/// `term` (a word or a short name: at most 12 CJK characters and at most 4 words, no sentence
/// punctuation) or `passage` (everything else).
pub fn kind(text: &str) -> &'static str {
    let t = text.trim();
    let cjk = t.chars().filter(|c| is_cjk(*c)).count();
    let words = t.split_whitespace().count();
    let punct = t.chars().any(|c| matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | ';' | '；' | '\n' | '\r'));
    if !t.is_empty() && cjk <= 12 && words <= 4 && !punct { "term" } else { "passage" }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

/// A ports.json short name: a few lowercase letters or digits.
fn short_name(app: &str) -> bool {
    (1..=8).contains(&app.len()) && app.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn invalid(r: &SelectionRequest) -> Option<&'static str> {
    let n = |s: &str| s.chars().count();
    if r.request.trim().is_empty() || r.request.len() > 200 {
        return Some("需要 request");
    }
    if !short_name(&r.app) {
        return Some("不認得要求的 App");
    }
    if r.port == Some(0) {
        return Some("port 不對");
    }
    if r.text.trim().is_empty() {
        return Some("需要 text");
    }
    if n(&r.text) > MAX_TEXT {
        return Some("選取的文字太長");
    }
    if n(&r.context) > MAX_CONTEXT || n(&r.prefix) > MAX_CONTEXT || n(&r.suffix) > MAX_CONTEXT {
        return Some("前後文太長");
    }
    if n(&r.doc) > 1_000 || n(&r.source) > 1_000 || n(&r.scope) > 1_000 || r.keep.len() > 500 || r.keep.iter().map(|k| n(k)).sum::<usize>() > MAX_TEXT {
        return Some("內容太長");
    }
    None
}

async fn sel_post(State(s): State<Shared>, Json(r): Json<SelectionRequest>) -> Response {
    if let Some(e) = invalid(&r) {
        return err(StatusCode::BAD_REQUEST, e);
    }
    let id = r.request.clone();
    let old = s.selection.current.lock().unwrap().replace(r);
    if let Some(old) = old.filter(|o| o.request != id) {
        tokio::task::spawn_blocking(move || end_selection(&old));
    }
    s.tell(DesktopEvent::Expanded(true));
    Json(json!({ "ok": true })).into_response()
}

async fn sel_get(State(s): State<Shared>) -> Json<Value> {
    Json(match s.selection.current() {
        Some(r) => json!({
            "open": true, "request": r.request, "app": r.app, "doc": r.doc, "source": r.source,
            "text": r.text, "scope": r.scope, "language": r.language, "kind": kind(&r.text),
            // a document that cannot be changed (a PDF): no rewrite, nothing sent back
            "readonly": r.readonly,
            // what a rewrite or a question about it needs: the rest of its paragraph and the page
            "prefix": r.prefix, "suffix": r.suffix, "context": r.context, "keep": r.keep,
        }),
        None => json!({ "open": false }),
    })
}

async fn end(State(s): State<Shared>) -> Json<Value> {
    let old = s.selection.current.lock().unwrap().take();
    if let Some(r) = old {
        let _ = tokio::task::spawn_blocking(move || end_selection(&r)).await;
    }
    Json(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct Apply {
    after: String,
    #[serde(default)]
    polish: Option<String>,
}

/// The app's port: the one it sent, else ports.json while it runs.
fn port_of(r: &SelectionRequest) -> Option<u16> {
    r.port.or_else(|| lore_home::ports::live().get(&r.app).copied())
}

async fn apply(State(s): State<Shared>, Json(b): Json<Apply>) -> Response {
    let Some(r) = s.selection.current() else { return err(StatusCode::CONFLICT, "沒有選取的文字") };
    if b.after.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "需要 after");
    }
    if b.after.chars().count() > MAX_TEXT {
        return err(StatusCode::BAD_REQUEST, "結果太長");
    }
    let polish = b.polish.filter(|p| !p.trim().is_empty());
    let (r2, p2) = (r.clone(), polish.clone());
    let res = tokio::task::spawn_blocking(move || send_selected(&r2, &b.after, p2.as_deref())).await;
    match res {
        Ok(Ok(message)) => {
            if let (Some(id), Some(port)) = (polish, port_of(&r)) {
                // the proposal log: this one was used (its failure changes nothing)
                let _ = client(QUICK).post(format!("http://127.0.0.1:{port}/api/polish/applied")).json(&json!({ "doc": r.doc, "id": id })).send().await;
            }
            Json(json!({ "ok": true, "message": message })).into_response()
        }
        Ok(Err((code, e))) => {
            if code == 410 {
                // the app ended it: so does L-Echo
                let mut c = s.selection.current.lock().unwrap();
                if c.as_ref().is_some_and(|x| x.request == r.request) {
                    *c = None;
                }
            }
            let status = match code {
                0 => StatusCode::BAD_GATEWAY,
                c => StatusCode::from_u16(c).ok().filter(|c| c.is_client_error() || c.is_server_error()).unwrap_or(StatusCode::BAD_GATEWAY),
            };
            err(status, &e)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder().timeout(timeout).no_proxy().build().unwrap_or_default()
}

/// `path` on the asking app (127.0.0.1), the body and query as they came; its status and answer as they are.
async fn proxy(State(s): State<Shared>, method: Method, path: &'static str, timeout: Duration, RawQuery(q): RawQuery, body: Bytes) -> Response {
    let Some(r) = s.selection.current() else { return err(StatusCode::CONFLICT, "沒有選取的文字") };
    let Some(port) = port_of(&r) else { return err(StatusCode::BAD_GATEWAY, "要求的 App 已經關了") };
    let url = match q.filter(|q| !q.is_empty()) {
        Some(q) => format!("http://127.0.0.1:{port}{path}?{q}"),
        None => format!("http://127.0.0.1:{port}{path}"),
    };
    let mut req = client(timeout).request(method.clone(), url);
    if method == Method::POST {
        req = req.header(header::CONTENT_TYPE, "application/json").body(body);
    }
    match req.send().await {
        Ok(res) => {
            let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let ct = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/json").to_string();
            match res.bytes().await {
                Ok(b) => (status, [(header::CONTENT_TYPE, ct)], b).into_response(),
                Err(e) => err(StatusCode::BAD_GATEWAY, &e.to_string()),
            }
        }
        Err(e) if e.is_timeout() => err(StatusCode::GATEWAY_TIMEOUT, "要求的 App 沒有回應"),
        Err(_) => err(StatusCode::BAD_GATEWAY, "要求的 App 已經關了"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_kind_terms_and_passages() {
        for t in ["注意力機制", "  前饋神經網路 ", "softmax", "cross entropy", "Vision Transformer", "Transformer 的 attention", "批次正規化層的動量參數", "ResNet-50"] {
            assert_eq!(kind(t), "term", "{t}");
        }
        for t in [
            "這個方法在很多資料集上都表現得很好",
            "模型很好。",
            "真的嗎？",
            "attention is all you need",
            "It works.",
            "Why?",
            "first; second",
            "前饋\n神經網路",
            "",
            "   ",
        ] {
            assert_eq!(kind(t), "passage", "{t:?}");
        }
        assert_eq!(kind(&"字".repeat(12)), "term");
        assert_eq!(kind(&"字".repeat(13)), "passage");
    }

    #[test]
    fn selection_names_and_limits() {
        assert!(short_name("sc") && short_name("pc") && !short_name("") && !short_name("../x") && !short_name("SC") && !short_name("toolongname"));
        let ok = SelectionRequest { request: "r1".into(), app: "sc".into(), port: Some(7000), doc: "d".into(), source: String::new(), text: "一句".into(), prefix: String::new(), suffix: String::new(), context: String::new(), scope: String::new(), keep: vec![], language: None, readonly: false };
        assert_eq!(invalid(&ok), None);
        assert!(invalid(&SelectionRequest { port: Some(0), ..ok.clone() }).is_some());
        assert!(invalid(&SelectionRequest { text: " ".into(), ..ok.clone() }).is_some());
        assert!(invalid(&SelectionRequest { text: "字".repeat(MAX_TEXT + 1), ..ok.clone() }).is_some());
        assert!(invalid(&SelectionRequest { context: "字".repeat(MAX_CONTEXT + 1), ..ok.clone() }).is_some());
        assert_eq!(invalid(&SelectionRequest { text: "字".repeat(MAX_TEXT), context: "字".repeat(MAX_CONTEXT), ..ok }), None);
    }
}
