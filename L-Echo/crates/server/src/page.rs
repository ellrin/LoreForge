//! One or two pages of a deck from another Lore app (`lore_shell::pageedit`): the app's server
//! hands L-Echo the pages (`POST /api/page`), the window opens in page mode, the page polls it and
//! sends the author's instruction; everything else is the app's (the model call, the preview, the
//! render, the check), reached through the proxies below on 127.0.0.1. At most one request, in
//! memory only: a new one replaces the old (whose app is told `end`), and a page request and a text
//! selection ([`crate::selection`]) end each other. Nothing is written anywhere.
//!
//! | method | path | what |
//! |---|---|---|
//! | POST | /api/page | a `PageRequest` from an app's server → kept, the window opens → `{ok}` |
//! | GET | /api/page | `{open: false}` or `{open: true, request, app, deck, source, slides: [{id, n, title}]}` (the page polls) |
//! | POST | /api/page/end | the app is told, the request is dropped |
//! | POST | /api/page/edit | → the app's `POST /api/decks/{deck}/page-edit` (`slides: [ids]` added when absent) |
//! | POST | /api/page/edit/{id}/revise, /apply, /undo | → the app's `POST /api/decks/{deck}/page-edit/{id}/…`; a 410 from `apply` ends the request here too |
//! | GET | /api/page/edit/{id}/image/{which} | → the app's `GET /api/decks/{deck}/page-edit/{id}/image/{which}` (a PNG, its content type kept) |
//!
//! The proxies pass the body, the query, the status and the answer unchanged, only while a request
//! is open (else 409).

use std::sync::Mutex;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_shell::pageedit::{PageRequest, end_page};
use serde_json::{Value, json};

use crate::{DesktopEvent, Shared};

/// A model call on the app's side (the edit, a revision).
const SLOW: Duration = Duration::from_secs(300);
/// Everything else: the apply waits for the app's page (up to 15 s), a render may take a while.
const NORMAL: Duration = Duration::from_secs(60);

const NONE: &str = "沒有交給 L-Echo 的頁面";

#[derive(Default)]
pub struct Pager {
    current: Mutex<Option<PageRequest>>,
}

impl Pager {
    fn current(&self) -> Option<PageRequest> {
        self.current.lock().unwrap().clone()
    }

    /// Drop the open page request (a text selection replaces it), telling its app.
    pub(crate) fn end_now(&self) {
        if let Some(old) = self.current.lock().unwrap().take() {
            tokio::task::spawn_blocking(move || end_page(&old));
        }
    }

    /// Drop `request` if it is still the current one.
    fn forget(&self, request: &str) {
        let mut c = self.current.lock().unwrap();
        if c.as_ref().is_some_and(|x| x.request == request) {
            *c = None;
        }
    }
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/page", get(page_get).post(page_post))
        .route("/api/page/end", post(end))
        .route("/api/page/edit", post(edit))
        .route("/api/page/edit/{id}/revise", post(|s: State<Shared>, Path(id): Path<String>, q: RawQuery, b: Bytes| step(s, id, "revise", SLOW, q, b)))
        .route("/api/page/edit/{id}/apply", post(|s: State<Shared>, Path(id): Path<String>, q: RawQuery, b: Bytes| step(s, id, "apply", NORMAL, q, b)))
        .route("/api/page/edit/{id}/undo", post(|s: State<Shared>, Path(id): Path<String>, q: RawQuery, b: Bytes| step(s, id, "undo", NORMAL, q, b)))
        .route("/api/page/edit/{id}/image/{which}", get(image))
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

fn invalid(r: &PageRequest) -> Option<&'static str> {
    if !crate::selection::short_name(&r.app) {
        return Some("不認得要求的 App");
    }
    if r.port == Some(0) {
        return Some("port 不對");
    }
    r.invalid()
}

async fn page_post(State(s): State<Shared>, Json(r): Json<PageRequest>) -> Response {
    if let Some(e) = invalid(&r) {
        return err(StatusCode::BAD_REQUEST, e);
    }
    let id = r.request.clone();
    let old = s.page.current.lock().unwrap().replace(r);
    if let Some(old) = old.filter(|o| o.request != id) {
        tokio::task::spawn_blocking(move || end_page(&old));
    }
    // one thing handed to L-Echo at a time: an open text selection ends
    s.selection.end_now();
    s.tell(DesktopEvent::Expanded(true));
    Json(json!({ "ok": true })).into_response()
}

async fn page_get(State(s): State<Shared>) -> Json<Value> {
    Json(match s.page.current() {
        Some(r) => json!({ "open": true, "request": r.request, "app": r.app, "deck": r.deck, "source": r.source, "slides": r.slides }),
        None => json!({ "open": false }),
    })
}

async fn end(State(s): State<Shared>) -> Json<Value> {
    let old = s.page.current.lock().unwrap().take();
    if let Some(r) = old {
        let _ = tokio::task::spawn_blocking(move || end_page(&r)).await;
    }
    Json(json!({ "ok": true }))
}

/// One path segment, percent-encoded (everything but unreserved characters).
fn seg(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn base(r: &PageRequest) -> String {
    format!("/api/decks/{}/page-edit", seg(&r.deck))
}

/// The edit: the page's `{instruction, material?, engine?}`, with the request's slide ids when absent.
async fn edit(State(s): State<Shared>, RawQuery(q): RawQuery, body: Bytes) -> Response {
    let Some(r) = s.page.current() else { return err(StatusCode::CONFLICT, NONE) };
    let Ok(Value::Object(mut b)) = serde_json::from_slice::<Value>(&body) else { return err(StatusCode::BAD_REQUEST, "需要 JSON") };
    if !b.contains_key("slides") {
        b.insert("slides".into(), json!(r.slides.iter().map(|x| x.id.clone()).collect::<Vec<_>>()));
    }
    let path = base(&r);
    proxy(&r, Method::POST, &path, SLOW, q, Bytes::from(Value::Object(b).to_string())).await
}

async fn step(State(s): State<Shared>, id: String, what: &str, timeout: Duration, RawQuery(q): RawQuery, body: Bytes) -> Response {
    let Some(r) = s.page.current() else { return err(StatusCode::CONFLICT, NONE) };
    let path = format!("{}/{}/{what}", base(&r), seg(&id));
    let res = proxy(&r, Method::POST, &path, timeout, q, body).await;
    if what == "apply" && res.status() == StatusCode::GONE {
        // the app ended it: so does L-Echo
        s.page.forget(&r.request);
    }
    res
}

async fn image(State(s): State<Shared>, Path((id, which)): Path<(String, String)>, RawQuery(q): RawQuery) -> Response {
    let Some(r) = s.page.current() else { return err(StatusCode::CONFLICT, NONE) };
    let path = format!("{}/{}/image/{}", base(&r), seg(&id), seg(&which));
    proxy(&r, Method::GET, &path, NORMAL, q, Bytes::new()).await
}

/// `path` on the asking app (127.0.0.1), the body and query as they came; its status, content type and answer as they are.
async fn proxy(r: &PageRequest, method: Method, path: &str, timeout: Duration, q: Option<String>, body: Bytes) -> Response {
    let Some(port) = r.port.or_else(|| lore_home::ports::live().get(&r.app).copied()) else { return err(StatusCode::BAD_GATEWAY, "要求的 App 已經關了") };
    let url = match q.filter(|q| !q.is_empty()) {
        Some(q) => format!("http://127.0.0.1:{port}{path}?{q}"),
        None => format!("http://127.0.0.1:{port}{path}"),
    };
    let client = reqwest::Client::builder().timeout(timeout).no_proxy().build().unwrap_or_default();
    let mut req = client.request(method.clone(), url);
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
    fn page_segments_encoded() {
        assert_eq!(seg("deck a"), "deck%20a");
        assert_eq!(seg("a/b?c#d"), "a%2Fb%3Fc%23d");
        assert_eq!(seg("簡報"), "%E7%B0%A1%E5%A0%B1");
        assert_eq!(seg("after-1"), "after-1");
    }
}
