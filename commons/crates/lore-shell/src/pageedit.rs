//! One or two pages of a deck through L-Echo: the author tells L-Echo what to change on them
//! ("split this page", "insert a page about …"); the app's own server makes the edit (its
//! `/api/decks/{deck}/page-edit…` routes, which L-Echo proxies), and when the author applies it the
//! app queues its operations for the page, which applies them as one undo step and says whether it
//! worked. A sibling of [`crate::selection`]: the payload (slides, opaque operations) and the
//! lifecycle differ, the transport is the same.
//!
//! The page hands the slides to the app's server (`POST /api/lore/page`), which mints a request id
//! and forwards it to L-Echo (`POST /api/page` on L-Echo, with `request`, `app` and `port` added).
//! `launch: false` only reaches an L-Echo that is already running. An apply reaches the receive
//! route (`POST /api/lore/paged {request, edit, ops}`, from the app's own apply handler or from
//! L-Echo through [`send_paged`]); it is queued for the page and answered only when the page has
//! applied it (or refused it, or never answered). One open request per deck: a new one ends the old.
//!
//! | method | path | who | what |
//! |---|---|---|---|
//! | POST | /api/lore/page | the app's page | `{deck, source, slides: [{id, n, title}] (1 or 2), launch}` → `{request}`; 503 when L-Echo is not reached (nothing kept) |
//! | GET | /api/lore/page/{request} | the app's page | `{open, pending: [{seq, edit, ops}], error}` (polled while open) |
//! | POST | /api/lore/page/{request}/done | the app's page | `{seq, ok, error?}`: the page applied (or refused) item `seq` |
//! | POST | /api/lore/paged | L-Echo / the app | `{request, edit, ops}` → `{ok, message}`, 409 refused by the page (its reason), 410 closed, 504 no answer; `{request, end: true}` closes it |
//!
//! An app mounts it with one line: `.merge(lore_shell::pageedit::router(PageEdits::new("sc")))`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::findfile::{ECHO, Finds, echo_port, fail, live, port_of, post_json};
use crate::selection::Echo;

/// At most this many slides in one request.
pub const MAX_SLIDES: usize = 2;
/// Longest deck name, source or slide title (characters).
pub const MAX_NAME: usize = 1_000;
/// Longest slide id or page-edit id (characters).
pub const MAX_ID: usize = 200;
/// Largest operations value (bytes of JSON).
pub const MAX_OPS: usize = 1_000_000;
/// How long `/api/lore/paged` waits for the page.
pub const WAIT: Duration = Duration::from_secs(15);

const CLOSED: &str = "這個頁面要求已經結束";

/// One page of the deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    /// The app's own slide id.
    pub id: String,
    /// The page number the author sees.
    #[serde(default)]
    pub n: u32,
    #[serde(default)]
    pub title: String,
}

/// What L-Echo is handed (`POST /api/page` on L-Echo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageRequest {
    pub request: String,
    /// The asking app's short name (ports.json).
    pub app: String,
    /// Where the app's server answers (else ports.json `app`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// The deck (the app's own id or name, used in its `/api/decks/{deck}` routes).
    pub deck: String,
    /// The deck's title.
    #[serde(default)]
    pub source: String,
    pub slides: Vec<Slide>,
}

impl PageRequest {
    /// Why this request cannot be taken (both sides check it), or `None`.
    pub fn invalid(&self) -> Option<&'static str> {
        invalid_parts(&self.deck, &self.source, &self.slides).or(if self.request.trim().is_empty() || self.request.len() > MAX_ID { Some("需要 request") } else { None })
    }
}

fn invalid_parts(deck: &str, source: &str, slides: &[Slide]) -> Option<&'static str> {
    let n = |s: &str| s.chars().count();
    if deck.trim().is_empty() {
        return Some("需要 deck");
    }
    if slides.is_empty() || slides.len() > MAX_SLIDES {
        return Some("一次只能交一到兩頁");
    }
    if slides.iter().any(|s| s.id.trim().is_empty()) {
        return Some("需要頁面的 id");
    }
    if n(deck) > MAX_NAME || n(source) > MAX_NAME || slides.iter().any(|s| n(&s.id) > MAX_ID || n(&s.title) > MAX_NAME) {
        return Some("內容太長");
    }
    None
}

/// What reaches the receive route (`POST /api/lore/paged` on the app).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paged {
    pub request: String,
    /// The app's own page-edit id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<String>,
    /// The operations for the page (opaque here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ops: Option<Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
}

/// One edit waiting for the page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pending {
    pub seq: u64,
    pub edit: String,
    pub ops: Value,
}

struct Open {
    deck: String,
    open: bool,
    next: u64,
    pending: Vec<Pending>,
    waiters: HashMap<u64, oneshot::Sender<Result<(), String>>>,
    error: Option<String>,
}

/// The app side: its open page requests.
#[derive(Clone)]
pub struct PageEdits {
    app: String,
    echo: Echo,
    wait: Duration,
    open: Arc<Mutex<HashMap<String, Open>>>,
}

impl PageEdits {
    /// `app` = the short name in ports.json; L-Echo is the real one.
    pub fn new(app: &str) -> PageEdits {
        let echo: Echo = Arc::new(|launch: bool| if launch { echo_port() } else { live(ECHO).ok_or_else(|| "L-Echo 沒有開".to_string()) });
        PageEdits { app: app.into(), echo, wait: WAIT, open: Arc::default() }
    }

    /// Find L-Echo some other way (tests: a temp ports.json, never a launch).
    pub fn with_echo(mut self, echo: Echo) -> PageEdits {
        self.echo = echo;
        self
    }

    /// Only L-Echo entries of this ports.json, never a launch (tests).
    pub fn with_ports(self, path: std::path::PathBuf) -> PageEdits {
        self.with_echo(Arc::new(move |_| lore_home::ports::live_at(&path).get(ECHO).copied().ok_or_else(|| "L-Echo 沒有開".to_string())))
    }

    /// How long an edit waits for the page (tests: short).
    pub fn with_wait(mut self, wait: Duration) -> PageEdits {
        self.wait = wait;
        self
    }

    /// Whether `request` is open (an app's apply handler may check before its own work).
    pub fn is_open(&self, request: &str) -> bool {
        self.open.lock().unwrap().get(request).is_some_and(|o| o.open)
    }

    /// An edit to apply: queued for the page, answered when the page has applied it.
    /// `Ok(message)`, or `(status, error)`: 409 refused by the page, 410 closed, 504 no answer.
    pub async fn paged(&self, p: Paged) -> Result<String, (u16, String)> {
        let mut out = Ok(String::new());
        if let Some(ops) = p.ops {
            let edit = p.edit.unwrap_or_default();
            if edit.trim().is_empty() || edit.chars().count() > MAX_ID {
                return Err((400, "需要 edit".into()));
            }
            if ops.to_string().len() > MAX_OPS {
                return Err((400, "修改太大".into()));
            }
            let (seq, rx) = {
                let mut all = self.open.lock().unwrap();
                let Some(o) = all.get_mut(&p.request).filter(|o| o.open) else { return Err((410, CLOSED.into())) };
                o.next += 1;
                let seq = o.next;
                let (tx, rx) = oneshot::channel();
                o.pending.push(Pending { seq, edit, ops });
                o.waiters.insert(seq, tx);
                (seq, rx)
            };
            out = match tokio::time::timeout(self.wait, rx).await {
                Ok(Ok(Ok(()))) => Ok("已套用".to_string()),
                Ok(Ok(Err(e))) => Err((409, e)),
                Ok(Err(_)) => Err((410, CLOSED.into())),
                Err(_) => {
                    if let Some(o) = self.open.lock().unwrap().get_mut(&p.request) {
                        o.pending.retain(|x| x.seq != seq);
                        o.waiters.remove(&seq);
                    }
                    Err((504, "頁面沒有回應".into()))
                }
            };
        } else if !p.end {
            return Err((400, "需要 ops 或 end".into()));
        }
        if p.end {
            let mut all = self.open.lock().unwrap();
            let Some(o) = all.get_mut(&p.request).filter(|o| o.open) else { return out.and(Err((410, CLOSED.into()))) };
            close(o);
        }
        out
    }

    /// The page's answer for item `seq`.
    fn done(&self, request: &str, d: Done) -> Result<(), (u16, String)> {
        let mut all = self.open.lock().unwrap();
        let Some(o) = all.get_mut(request) else { return Err((404, "沒有這個頁面要求".into())) };
        o.pending.retain(|p| p.seq != d.seq);
        let Some(tx) = o.waiters.remove(&d.seq) else { return Err((410, "已經不等這個修改".into())) };
        let r = if d.ok { Ok(()) } else { Err(d.error.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| "頁面沒有套用".into())) };
        if let Err(e) = &r {
            o.error = Some(e.clone());
        }
        let _ = tx.send(r);
        Ok(())
    }
}

/// Closed: nothing pending, waiting edits answered 410 (their senders drop).
fn close(o: &mut Open) {
    o.open = false;
    o.pending.clear();
    o.waiters.clear();
}

#[derive(Deserialize)]
struct PageBody {
    #[serde(default)]
    deck: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    slides: Vec<Slide>,
    #[serde(default)]
    launch: bool,
}

#[derive(Deserialize)]
struct Done {
    seq: u64,
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// The four routes above, for any app state.
pub fn router<S: Clone + Send + Sync + 'static>(pe: PageEdits) -> axum::Router<S> {
    use axum::extract::Path as UrlPath;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    let (s1, s2, s3, s4) = (pe.clone(), pe.clone(), pe.clone(), pe);
    axum::Router::new()
        .route(
            "/api/lore/page",
            post(move |headers: axum::http::HeaderMap, axum::Json(b): axum::Json<PageBody>| {
                let s = s1.clone();
                async move {
                    if let Some(e) = invalid_parts(&b.deck, &b.source, &b.slides) {
                        return fail(400, e);
                    }
                    let req = PageRequest { request: Finds::mint(), app: s.app.clone(), port: port_of(&headers), deck: b.deck, source: b.source, slides: b.slides };
                    {
                        // one open request per deck: the new one ends the old one
                        let mut all = s.open.lock().unwrap();
                        all.retain(|_, o| o.deck != req.deck);
                        all.insert(req.request.clone(), Open { deck: req.deck.clone(), open: true, next: 0, pending: vec![], waiters: HashMap::new(), error: None });
                    }
                    let (echo, launch, r2) = (s.echo.clone(), b.launch, req.clone());
                    let res = tokio::task::spawn_blocking(move || {
                        let port = echo(launch)?;
                        post_json(port, "/api/page", &json!(r2)).map(|_| ()).map_err(|(_, e)| e)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    match res {
                        Ok(()) => axum::Json(json!({ "request": req.request })).into_response(),
                        Err(e) => {
                            s.open.lock().unwrap().remove(&req.request);
                            fail(503, if e.contains("L-Echo") { &e } else { "L-Echo 沒有回應" })
                        }
                    }
                }
            }),
        )
        .route(
            "/api/lore/page/{request}",
            get(move |UrlPath(id): UrlPath<String>| {
                let s = s2.clone();
                async move {
                    let all = s.open.lock().unwrap();
                    match all.get(&id) {
                        Some(o) => axum::Json(json!({ "open": o.open, "pending": o.pending, "error": o.error })).into_response(),
                        None => fail(404, "沒有這個頁面要求"),
                    }
                }
            }),
        )
        .route(
            "/api/lore/page/{request}/done",
            post(move |UrlPath(id): UrlPath<String>, axum::Json(d): axum::Json<Done>| {
                let s = s3.clone();
                async move {
                    match s.done(&id, d) {
                        Ok(()) => axum::Json(json!({ "ok": true })).into_response(),
                        Err((code, e)) => fail(code, &e),
                    }
                }
            }),
        )
        .route(
            "/api/lore/paged",
            post(move |axum::Json(p): axum::Json<Paged>| {
                let s = s4.clone();
                async move {
                    match s.paged(p).await {
                        Ok(m) => axum::Json(json!({ "ok": true, "message": m })).into_response(),
                        Err((code, e)) => fail(code, &e),
                    }
                }
            }),
        )
}

// ── L-Echo's side ──────────────────────────────────

/// Send an edit's operations to the asking app and wait until its page applied them (blocking,
/// up to [`WAIT`]). `Ok(message)`, or `(status, error)`: 409 the page refused (its reason), 410
/// the request ended, 504 the page did not answer, 0 the app was not reached.
pub fn send_paged(req: &PageRequest, edit: &str, ops: &Value) -> Result<String, (u16, String)> {
    let port = req.port.or_else(|| live(&req.app)).ok_or_else(|| (0, "要求的 App 已經關了".to_string()))?;
    let body = Paged { request: req.request.clone(), edit: Some(edit.to_string()), ops: Some(ops.clone()), end: false };
    post_json(port, "/api/lore/paged", &json!(body)).map(|v| v["message"].as_str().unwrap_or_default().to_string())
}

/// L-Echo: the author is done with these pages.
pub fn end_page(req: &PageRequest) -> Result<(), (u16, String)> {
    let port = req.port.or_else(|| live(&req.app)).ok_or_else(|| (0, "要求的 App 已經關了".to_string()))?;
    post_json(port, "/api/lore/paged", &json!(Paged { request: req.request.clone(), edit: None, ops: None, end: true })).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn call(app: &axum::Router, method: &str, uri: &str, body: Value) -> (u16, Value) {
        let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:7878").header("content-type", "application/json").body(if method == "GET" { Body::empty() } else { Body::from(body.to_string()) }).unwrap();
        let r = app.clone().oneshot(req).await.unwrap();
        let code = r.status().as_u16();
        let b = http_body_util::BodyExt::collect(r.into_body()).await.unwrap().to_bytes();
        (code, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    /// A fake L-Echo on a free port, recording what it is handed; and a temp ports.json naming it.
    async fn fake_echo(name: &str) -> (std::path::PathBuf, Arc<Mutex<Vec<Value>>>) {
        let got: Arc<Mutex<Vec<Value>>> = Arc::default();
        let g = got.clone();
        let echo: axum::Router = axum::Router::new().route(
            "/api/page",
            axum::routing::post(move |axum::Json(v): axum::Json<Value>| {
                let g = g.clone();
                async move {
                    g.lock().unwrap().push(v);
                    axum::Json(json!({ "ok": true }))
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, echo).await.unwrap() });
        let ports = ports_file(name);
        lore_home::ports::register_at(&ports, ECHO, port, std::process::id()).unwrap();
        (ports, got)
    }

    fn ports_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lore-shell-pageedit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("ports.json")
    }

    fn body(deck: &str, n: usize, launch: bool) -> Value {
        let slides: Vec<Value> = (0..n).map(|i| json!({ "id": format!("s{i}"), "n": i + 3, "title": format!("第 {} 頁", i + 3) })).collect();
        json!({ "deck": deck, "source": "A synthetic deck", "slides": slides, "launch": launch })
    }

    async fn first_pending(app: &axum::Router, id: &str) -> Value {
        for _ in 0..100 {
            let v = call(app, "GET", &format!("/api/lore/page/{id}"), Value::Null).await.1;
            if !v["pending"].as_array().unwrap().is_empty() {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("nothing pending")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn page_forward_apply_refuse_and_end() {
        let (ports, got) = fake_echo("flow").await;
        let app: axum::Router = router(PageEdits::new("sc").with_ports(ports));
        let (c, v) = call(&app, "POST", "/api/lore/page", body("deck a", 2, false)).await;
        assert_eq!(c, 200, "{v}");
        let id = v["request"].as_str().unwrap().to_string();
        let sent = got.lock().unwrap()[0].clone();
        assert!(sent.get("launch").is_none());
        let r: PageRequest = serde_json::from_value(sent).unwrap();
        assert_eq!((r.request.as_str(), r.app.as_str(), r.port, r.deck.as_str(), r.source.as_str()), (id.as_str(), "sc", Some(7878), "deck a", "A synthetic deck"));
        assert_eq!(r.slides, vec![Slide { id: "s0".into(), n: 3, title: "第 3 頁".into() }, Slide { id: "s1".into(), n: 4, title: "第 4 頁".into() }]);

        // an edit arrives; the page polls, applies, says done; the sender hears ok
        let ops = json!([{ "op": "split", "slide": "s0", "at": 2 }]);
        let (a2, id2, o2) = (app.clone(), id.clone(), ops.clone());
        let sender = tokio::spawn(async move { call(&a2, "POST", "/api/lore/paged", json!({ "request": id2, "edit": "e1", "ops": o2 })).await });
        let v = first_pending(&app, &id).await;
        assert_eq!(v, json!({ "open": true, "pending": [{ "seq": 1, "edit": "e1", "ops": ops }], "error": null }));
        assert_eq!(call(&app, "POST", &format!("/api/lore/page/{id}/done"), json!({ "seq": 1, "ok": true })).await.0, 200);
        let (c, v) = sender.await.unwrap();
        assert_eq!((c, v), (200, json!({ "ok": true, "message": "已套用" })));
        assert_eq!(call(&app, "GET", &format!("/api/lore/page/{id}"), Value::Null).await.1["pending"], json!([]));

        // the page refuses (the slides changed): 409 with its words, kept as the error
        let (a2, id2) = (app.clone(), id.clone());
        let sender = tokio::spawn(async move { call(&a2, "POST", "/api/lore/paged", json!({ "request": id2, "edit": "e2", "ops": [] })).await });
        assert_eq!(first_pending(&app, &id).await["pending"][0]["seq"], json!(2));
        call(&app, "POST", &format!("/api/lore/page/{id}/done"), json!({ "seq": 2, "ok": false, "error": "頁面已經改過" })).await;
        let (c, v) = sender.await.unwrap();
        assert_eq!((c, v["error"].as_str()), (409, Some("頁面已經改過")));
        assert_eq!(call(&app, "GET", &format!("/api/lore/page/{id}"), Value::Null).await.1["error"], json!("頁面已經改過"));

        // ops without an edit id, or neither ops nor end: 400
        assert_eq!(call(&app, "POST", "/api/lore/paged", json!({ "request": id, "ops": [] })).await.0, 400);
        assert_eq!(call(&app, "POST", "/api/lore/paged", json!({ "request": id })).await.0, 400);

        // end: closed; later edits 410; the page sees open false
        assert_eq!(call(&app, "POST", "/api/lore/paged", json!({ "request": id, "end": true })).await.0, 200);
        assert_eq!(call(&app, "GET", &format!("/api/lore/page/{id}"), Value::Null).await.1["open"], json!(false));
        assert_eq!(call(&app, "POST", "/api/lore/paged", json!({ "request": id, "edit": "e3", "ops": [] })).await.0, 410);
        assert_eq!(call(&app, "POST", "/api/lore/paged", json!({ "request": "nope", "edit": "e3", "ops": [] })).await.0, 410);
        assert_eq!(call(&app, "POST", "/api/lore/paged", json!({ "request": id, "end": true })).await.0, 410);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn page_one_request_per_deck() {
        let (ports, got) = fake_echo("replace").await;
        let pe = PageEdits::new("sc").with_ports(ports);
        let app: axum::Router = router(pe.clone());
        let old = call(&app, "POST", "/api/lore/page", body("deck-a", 1, false)).await.1["request"].as_str().unwrap().to_string();
        // an edit waiting on the old request hears 410 when a new one for the same deck comes
        let (a2, o2) = (app.clone(), old.clone());
        let waiting = tokio::spawn(async move { call(&a2, "POST", "/api/lore/paged", json!({ "request": o2, "edit": "e1", "ops": [] })).await });
        first_pending(&app, &old).await;
        let other = call(&app, "POST", "/api/lore/page", body("deck-b", 1, false)).await.1["request"].as_str().unwrap().to_string();
        let new = call(&app, "POST", "/api/lore/page", body("deck-a", 2, true)).await.1["request"].as_str().unwrap().to_string();
        assert_eq!(waiting.await.unwrap().0, 410);
        assert_eq!(got.lock().unwrap().len(), 3);
        assert_eq!(call(&app, "GET", &format!("/api/lore/page/{old}"), Value::Null).await.0, 404);
        assert_eq!(call(&app, "POST", "/api/lore/paged", json!({ "request": old, "edit": "e", "ops": [] })).await.0, 410);
        assert!(pe.is_open(&other) && pe.is_open(&new) && !pe.is_open(&old));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn page_never_answers_504() {
        let (ports, _) = fake_echo("timeout").await;
        let app: axum::Router = router(PageEdits::new("sc").with_ports(ports).with_wait(Duration::from_millis(100)));
        let id = call(&app, "POST", "/api/lore/page", body("d", 1, false)).await.1["request"].as_str().unwrap().to_string();
        let (c, v) = call(&app, "POST", "/api/lore/paged", json!({ "request": id, "edit": "e1", "ops": [] })).await;
        assert_eq!((c, v["error"].as_str()), (504, Some("頁面沒有回應")));
        assert_eq!(call(&app, "GET", &format!("/api/lore/page/{id}"), Value::Null).await.1["pending"], json!([]));
        assert_eq!(call(&app, "POST", &format!("/api/lore/page/{id}/done"), json!({ "seq": 1, "ok": true })).await.0, 410);
    }

    #[tokio::test]
    async fn page_echo_absent_or_unreachable_drops_the_request() {
        let pe = PageEdits::new("sc").with_ports(ports_file("absent"));
        let app: axum::Router = router(pe.clone());
        let (c, v) = call(&app, "POST", "/api/lore/page", body("d", 1, false)).await;
        assert_eq!(c, 503, "{v}");
        assert!(v["error"].as_str().unwrap().contains("L-Echo"));
        assert!(pe.open.lock().unwrap().is_empty());
        let pe = PageEdits::new("sc").with_echo(Arc::new(|_| {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            Ok(l.local_addr().unwrap().port())
        }));
        let app: axum::Router = router(pe.clone());
        assert_eq!(call(&app, "POST", "/api/lore/page", body("d", 1, true)).await.0, 503);
        assert!(pe.open.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn page_launch_flag_reaches_the_echo_lookup() {
        let seen: Arc<Mutex<Vec<bool>>> = Arc::default();
        let s = seen.clone();
        let app: axum::Router = router(PageEdits::new("sc").with_echo(Arc::new(move |launch| {
            s.lock().unwrap().push(launch);
            Err("L-Echo 沒有開".into())
        })));
        let (c, v) = call(&app, "POST", "/api/lore/page", body("d", 1, false)).await;
        assert_eq!((c, v["error"].as_str()), (503, Some("L-Echo 沒有開")));
        call(&app, "POST", "/api/lore/page", body("d", 1, true)).await;
        assert_eq!(*seen.lock().unwrap(), vec![false, true]);
    }

    #[tokio::test]
    async fn page_bad_payloads_refused() {
        let asked: Arc<Mutex<u32>> = Arc::default();
        let a = asked.clone();
        let app: axum::Router = router(PageEdits::new("sc").with_echo(Arc::new(move |_| {
            *a.lock().unwrap() += 1;
            Err("no".into())
        })));
        assert_eq!(call(&app, "POST", "/api/lore/page", body("d", 0, false)).await.0, 400);
        assert_eq!(call(&app, "POST", "/api/lore/page", body("d", 3, false)).await.0, 400, "at most two slides");
        assert_eq!(call(&app, "POST", "/api/lore/page", body(" ", 1, false)).await.0, 400);
        let mut b = body("d", 1, false);
        b["slides"][0]["id"] = json!(" ");
        assert_eq!(call(&app, "POST", "/api/lore/page", b).await.0, 400);
        let mut b = body("d", 1, false);
        b["slides"][0]["title"] = json!("字".repeat(MAX_NAME + 1));
        assert_eq!(call(&app, "POST", "/api/lore/page", b).await.0, 400);
        let mut b = body("d", 1, false);
        b["slides"][0]["id"] = json!("x".repeat(MAX_ID + 1));
        assert_eq!(call(&app, "POST", "/api/lore/page", b).await.0, 400);
        assert_eq!(call(&app, "POST", "/api/lore/page", body(&"字".repeat(MAX_NAME + 1), 1, false)).await.0, 400);
        assert_eq!(*asked.lock().unwrap(), 0, "nothing forwarded");
        // two slides at the limits reach the echo step
        let mut b = body(&"字".repeat(MAX_NAME), 2, false);
        b["slides"][1]["title"] = json!("字".repeat(MAX_NAME));
        assert_eq!(call(&app, "POST", "/api/lore/page", b).await.0, 503);
        assert_eq!(*asked.lock().unwrap(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn page_echo_helpers_reach_the_app_over_http() {
        let (ports, _) = fake_echo("helpers").await;
        let pe = PageEdits::new("sc").with_ports(ports);
        let app: axum::Router = router(pe.clone());
        let id = call(&app, "POST", "/api/lore/page", body("d", 1, false)).await.1["request"].as_str().unwrap().to_string();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        // the page: apply whatever arrives, remember what it got
        let got: Arc<Mutex<Vec<Pending>>> = Arc::default();
        let (page, rid, g) = (pe.clone(), id.clone(), got.clone());
        tokio::spawn(async move {
            loop {
                let first = page.open.lock().unwrap().get(&rid).and_then(|o| o.pending.first().cloned());
                if let Some(p) = first {
                    g.lock().unwrap().push(p.clone());
                    let _ = page.done(&rid, Done { seq: p.seq, ok: true, error: None });
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let req = PageRequest { request: id, app: "sc".into(), port: Some(port), deck: "d".into(), source: String::new(), slides: vec![Slide { id: "s0".into(), n: 1, title: String::new() }] };
        assert_eq!(req.invalid(), None);
        let ops = json!({ "insert": [{ "after": "s0" }] });
        let o2 = ops.clone();
        let r = tokio::task::spawn_blocking(move || (send_paged(&req, "e1", &o2), end_page(&req), send_paged(&req, "e2", &o2))).await.unwrap();
        assert_eq!(r.0, Ok("已套用".to_string()));
        assert_eq!(r.1, Ok(()));
        assert_eq!(r.2.unwrap_err().0, 410);
        assert_eq!(got.lock().unwrap().clone(), vec![Pending { seq: 1, edit: "e1".into(), ops }]);
    }
}
