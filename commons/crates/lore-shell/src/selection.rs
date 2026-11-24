//! Selected text through L-Echo: the author selects text in a Lore app, L-Echo works on it and
//! sends a rewrite back, the page applies it as one undo step and says whether it worked.
//!
//! The page (`commons/ui/selection.js`, `/selection.js`, `LoreSelection.watch({gather, apply})`)
//! hands each settled selection to the app's own server (`POST /api/lore/selection`), which mints
//! a request id and forwards it to L-Echo (`POST /api/selection` on L-Echo, with `request`, `app`
//! and `port` added). `launch: false` only reaches an L-Echo that is already running (ports.json
//! `le`); `launch: true` brings it up like 搜尋檔案. L-Echo sends a result to the app's receive
//! route (`POST /api/lore/selected {request, after, polish}`); the server queues it for the page
//! and answers L-Echo only when the page has applied it (or refused it, or never answered).
//! One open request per document: a new selection ends the old one. Everything stays on 127.0.0.1.
//!
//! | method | path | who | what |
//! |---|---|---|---|
//! | POST | /api/lore/selection | the app's page | `{doc, source, text, prefix, suffix, context, scope, keep, language?, readonly?, launch}` → `{request}`; 503 when L-Echo is not reached; `readonly` (a PDF) = L-Echo offers no rewrite |
//! | GET | /api/lore/selection/{request} | the app's page | `{open, pending: [{seq, after, polish}], error}` (polled while open) |
//! | POST | /api/lore/selection/{request}/done | the app's page | `{seq, ok, error?}`: the page applied (or refused) item `seq` |
//! | POST | /api/lore/selected | L-Echo | `{request, after, polish?}` → `{ok, message}`, 409 refused by the page, 410 closed, 504 no answer; `{request, end: true}` closes it |
//!
//! An app mounts it with one line: `.merge(lore_shell::selection::router(Selections::new("sc")))`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot;

use crate::findfile::{ECHO, Finds, echo_port, fail, live, port_of, post_json};

/// Longest selected text (characters); also the longest result.
pub const MAX_TEXT: usize = 20_000;
/// Longest context, prefix or suffix (characters).
pub const MAX_CONTEXT: usize = 8_000;
/// How long L-Echo's `/api/lore/selected` waits for the page.
pub const WAIT: Duration = Duration::from_secs(15);

/// What L-Echo is handed (`POST /api/selection` on L-Echo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionRequest {
    pub request: String,
    /// The asking app's short name (ports.json).
    pub app: String,
    /// Where the app's server answers (else ports.json `app`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// The document the text is in (the app's own id or name).
    #[serde(default)]
    pub doc: String,
    /// Where in the document (a slide, a section, a field).
    #[serde(default)]
    pub source: String,
    pub text: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub context: String,
    /// What kind of text it is (the app's word, e.g. a paragraph, a bullet).
    #[serde(default)]
    pub scope: String,
    /// Pieces the rewrite must keep as they are.
    #[serde(default)]
    pub keep: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The document cannot be changed (a PDF): L-Echo offers no rewrite and sends nothing back.
    #[serde(default)]
    pub readonly: bool,
}

/// What L-Echo sends back (`POST /api/lore/selected` on the app).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selected {
    pub request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polish: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
}

/// One result waiting for the page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pending {
    pub seq: u64,
    pub after: String,
    pub polish: Option<String>,
}

/// L-Echo's port: `true` = launch it when needed (blocking), `false` = only a running one.
pub type Echo = Arc<dyn Fn(bool) -> Result<u16, String> + Send + Sync>;

struct Open {
    doc: String,
    open: bool,
    next: u64,
    pending: Vec<Pending>,
    waiters: HashMap<u64, oneshot::Sender<Result<(), String>>>,
    error: Option<String>,
}

/// The app side: its open selection requests.
#[derive(Clone)]
pub struct Selections {
    app: String,
    echo: Echo,
    wait: Duration,
    open: Arc<Mutex<HashMap<String, Open>>>,
}

impl Selections {
    /// `app` = the short name in ports.json; L-Echo is the real one.
    pub fn new(app: &str) -> Selections {
        let echo: Echo = Arc::new(|launch: bool| if launch { echo_port() } else { live(ECHO).ok_or_else(|| "L-Echo 沒有開".to_string()) });
        Selections { app: app.into(), echo, wait: WAIT, open: Arc::default() }
    }

    /// Find L-Echo some other way (tests: a temp ports.json, never a launch).
    pub fn with_echo(mut self, echo: Echo) -> Selections {
        self.echo = echo;
        self
    }

    /// Only L-Echo entries of this ports.json, never a launch (tests).
    pub fn with_ports(self, path: std::path::PathBuf) -> Selections {
        self.with_echo(Arc::new(move |_| lore_home::ports::live_at(&path).get(ECHO).copied().ok_or_else(|| "L-Echo 沒有開".to_string())))
    }

    /// How long a result waits for the page (tests: short).
    pub fn with_wait(mut self, wait: Duration) -> Selections {
        self.wait = wait;
        self
    }

    /// A result from L-Echo: queued for the page, answered when the page has applied it.
    /// `Ok(message)`, or `(status, error)`: 409 refused by the page, 410 closed, 504 no answer.
    pub async fn selected(&self, s: Selected) -> Result<String, (u16, String)> {
        let mut out = Ok(String::new());
        if let Some(after) = s.after {
            if after.chars().count() > MAX_TEXT {
                return Err((400, "結果太長".into()));
            }
            let (seq, rx) = {
                let mut all = self.open.lock().unwrap();
                let Some(o) = all.get_mut(&s.request).filter(|o| o.open) else { return Err((410, "這個選取已經結束".into())) };
                o.next += 1;
                let seq = o.next;
                let (tx, rx) = oneshot::channel();
                o.pending.push(Pending { seq, after, polish: s.polish });
                o.waiters.insert(seq, tx);
                (seq, rx)
            };
            out = match tokio::time::timeout(self.wait, rx).await {
                Ok(Ok(Ok(()))) => Ok("已套用".to_string()),
                Ok(Ok(Err(e))) => Err((409, e)),
                Ok(Err(_)) => Err((410, "這個選取已經結束".into())),
                Err(_) => {
                    if let Some(o) = self.open.lock().unwrap().get_mut(&s.request) {
                        o.pending.retain(|p| p.seq != seq);
                        o.waiters.remove(&seq);
                    }
                    Err((504, "頁面沒有回應".into()))
                }
            };
        } else if !s.end {
            return Err((400, "需要 after 或 end".into()));
        }
        if s.end {
            let mut all = self.open.lock().unwrap();
            let Some(o) = all.get_mut(&s.request).filter(|o| o.open) else { return out.and(Err((410, "這個選取已經結束".into()))) };
            close(o);
        }
        out
    }

    /// The page's answer for item `seq`.
    fn done(&self, request: &str, d: Done) -> Result<(), (u16, String)> {
        let mut all = self.open.lock().unwrap();
        let Some(o) = all.get_mut(request) else { return Err((404, "沒有這個選取".into())) };
        o.pending.retain(|p| p.seq != d.seq);
        let Some(tx) = o.waiters.remove(&d.seq) else { return Err((410, "已經不等這個結果".into())) };
        let r = if d.ok { Ok(()) } else { Err(d.error.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| "頁面沒有套用".into())) };
        if let Err(e) = &r {
            o.error = Some(e.clone());
        }
        let _ = tx.send(r);
        Ok(())
    }
}

/// Closed: nothing pending, waiting results answered 410 (their senders drop).
fn close(o: &mut Open) {
    o.open = false;
    o.pending.clear();
    o.waiters.clear();
}

#[derive(Deserialize)]
struct SelectionBody {
    #[serde(default)]
    doc: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    suffix: String,
    #[serde(default)]
    context: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    keep: Vec<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    readonly: bool,
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

fn too_long(b: &SelectionBody) -> Option<&'static str> {
    let n = |s: &str| s.chars().count();
    if n(&b.text) > MAX_TEXT {
        return Some("選取的文字太長");
    }
    if n(&b.context) > MAX_CONTEXT || n(&b.prefix) > MAX_CONTEXT || n(&b.suffix) > MAX_CONTEXT {
        return Some("前後文太長");
    }
    if n(&b.doc) > 1_000 || n(&b.source) > 1_000 || n(&b.scope) > 1_000 || b.keep.len() > 500 || b.keep.iter().map(|k| n(k)).sum::<usize>() > MAX_TEXT {
        return Some("內容太長");
    }
    None
}

/// The four routes above, for any app state.
pub fn router<S: Clone + Send + Sync + 'static>(sel: Selections) -> axum::Router<S> {
    use axum::extract::Path as UrlPath;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    let (s1, s2, s3, s4) = (sel.clone(), sel.clone(), sel.clone(), sel);
    axum::Router::new()
        .route(
            "/api/lore/selection",
            post(move |headers: axum::http::HeaderMap, axum::Json(b): axum::Json<SelectionBody>| {
                let s = s1.clone();
                async move {
                    if b.text.trim().is_empty() {
                        return fail(400, "需要 text");
                    }
                    if let Some(e) = too_long(&b) {
                        return fail(400, e);
                    }
                    let req = SelectionRequest {
                        request: Finds::mint(),
                        app: s.app.clone(),
                        port: port_of(&headers),
                        doc: b.doc,
                        source: b.source,
                        text: b.text,
                        prefix: b.prefix,
                        suffix: b.suffix,
                        context: b.context,
                        scope: b.scope,
                        keep: b.keep,
                        language: b.language.filter(|l| !l.trim().is_empty()),
                        readonly: b.readonly,
                    };
                    {
                        // one open request per document: the new selection ends the old one
                        let mut all = s.open.lock().unwrap();
                        all.retain(|_, o| o.doc != req.doc);
                        all.insert(req.request.clone(), Open { doc: req.doc.clone(), open: true, next: 0, pending: vec![], waiters: HashMap::new(), error: None });
                    }
                    let (echo, launch, r2) = (s.echo.clone(), b.launch, req.clone());
                    let res = tokio::task::spawn_blocking(move || {
                        let port = echo(launch)?;
                        post_json(port, "/api/selection", &json!(r2)).map(|_| ()).map_err(|(_, e)| e)
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
            "/api/lore/selection/{request}",
            get(move |UrlPath(id): UrlPath<String>| {
                let s = s2.clone();
                async move {
                    let all = s.open.lock().unwrap();
                    match all.get(&id) {
                        Some(o) => axum::Json(json!({ "open": o.open, "pending": o.pending, "error": o.error })).into_response(),
                        None => fail(404, "沒有這個選取"),
                    }
                }
            }),
        )
        .route(
            "/api/lore/selection/{request}/done",
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
            "/api/lore/selected",
            post(move |axum::Json(p): axum::Json<Selected>| {
                let s = s4.clone();
                async move {
                    match s.selected(p).await {
                        Ok(m) => axum::Json(json!({ "ok": true, "message": m })).into_response(),
                        Err((code, e)) => fail(code, &e),
                    }
                }
            }),
        )
}

// ── L-Echo's side ──────────────────────────────────

/// L-Echo: send a rewrite back to the asking app and wait until its page applied it (blocking,
/// up to [`WAIT`]). `Ok(message)`, or `(status, error)`: 409 the page refused (its reason), 410
/// the request ended, 504 the page did not answer, 0 the app was not reached.
pub fn send_selected(req: &SelectionRequest, after: &str, polish: Option<&str>) -> Result<String, (u16, String)> {
    let port = req.port.or_else(|| live(&req.app)).ok_or_else(|| (0, "要求的 App 已經關了".to_string()))?;
    let body = Selected { request: req.request.clone(), after: Some(after.to_string()), polish: polish.map(str::to_string), end: false };
    post_json(port, "/api/lore/selected", &json!(body)).map(|v| v["message"].as_str().unwrap_or_default().to_string())
}

/// L-Echo: the author is done with this selection.
pub fn end_selection(req: &SelectionRequest) -> Result<(), (u16, String)> {
    let port = req.port.or_else(|| live(&req.app)).ok_or_else(|| (0, "要求的 App 已經關了".to_string()))?;
    post_json(port, "/api/lore/selected", &json!(Selected { request: req.request.clone(), after: None, polish: None, end: true })).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use serde_json::Value;
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
            "/api/selection",
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
        let dir = std::env::temp_dir().join(format!("lore-shell-selection-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("ports.json")
    }

    fn body(doc: &str, text: &str, launch: bool) -> Value {
        json!({ "doc": doc, "source": "slide 3", "text": text, "prefix": "前文", "suffix": "後文", "context": "整段", "scope": "bullet", "keep": ["\\ref{fig:a}"], "language": "zh", "launch": launch })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forward_apply_refuse_and_end() {
        let (ports, got) = fake_echo("flow").await;
        let sel = Selections::new("sc").with_ports(ports);
        let app: axum::Router = router(sel.clone());
        let (c, v) = call(&app, "POST", "/api/lore/selection", body("deck-a", "舊的句子", false)).await;
        assert_eq!(c, 200, "{v}");
        let id = v["request"].as_str().unwrap().to_string();
        let r: SelectionRequest = serde_json::from_value(got.lock().unwrap()[0].clone()).unwrap();
        assert_eq!((r.request.as_str(), r.app.as_str(), r.port, r.doc.as_str(), r.text.as_str(), r.keep.clone(), r.language.as_deref()), (id.as_str(), "sc", Some(7878), "deck-a", "舊的句子", vec!["\\ref{fig:a}".to_string()], Some("zh")));
        assert!(got.lock().unwrap()[0].get("launch").is_none());
        assert_eq!(got.lock().unwrap()[0]["readonly"], json!(false), "readonly defaults to false");

        // a read-only document (a PDF): readonly reaches L-Echo
        let mut ro = body("paper-a", "一段", false);
        ro["readonly"] = json!(true);
        let (c, v) = call(&app, "POST", "/api/lore/selection", ro).await;
        assert_eq!(c, 200, "{v}");
        let r: SelectionRequest = serde_json::from_value(got.lock().unwrap()[1].clone()).unwrap();
        assert!(r.readonly);

        // L-Echo sends a result; the page polls, applies, says done; L-Echo hears ok
        let a2 = app.clone();
        let id2 = id.clone();
        let echo_side = tokio::spawn(async move { call(&a2, "POST", "/api/lore/selected", json!({ "request": id2, "after": "新的句子", "polish": "簡潔" })).await });
        let mut v = Value::Null;
        for _ in 0..100 {
            v = call(&app, "GET", &format!("/api/lore/selection/{id}"), Value::Null).await.1;
            if !v["pending"].as_array().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(v, json!({ "open": true, "pending": [{ "seq": 1, "after": "新的句子", "polish": "簡潔" }], "error": null }));
        assert_eq!(call(&app, "POST", &format!("/api/lore/selection/{id}/done"), json!({ "seq": 1, "ok": true })).await.0, 200);
        let (c, v) = echo_side.await.unwrap();
        assert_eq!((c, v["ok"].as_bool()), (200, Some(true)), "{v}");
        assert_eq!(call(&app, "GET", &format!("/api/lore/selection/{id}"), Value::Null).await.1["pending"], json!([]));

        // the page refuses (the selection went stale): 409 with its words
        let a2 = app.clone();
        let id2 = id.clone();
        let echo_side = tokio::spawn(async move { call(&a2, "POST", "/api/lore/selected", json!({ "request": id2, "after": "再改" })).await });
        let mut seq = 0;
        for _ in 0..100 {
            let v = call(&app, "GET", &format!("/api/lore/selection/{id}"), Value::Null).await.1;
            if let Some(p) = v["pending"].as_array().unwrap().first() {
                seq = p["seq"].as_u64().unwrap();
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(seq, 2);
        call(&app, "POST", &format!("/api/lore/selection/{id}/done"), json!({ "seq": 2, "ok": false, "error": "選取已經改過" })).await;
        let (c, v) = echo_side.await.unwrap();
        assert_eq!((c, v["error"].as_str()), (409, Some("選取已經改過")));

        // end: closed; later results 410; the page sees open false
        assert_eq!(call(&app, "POST", "/api/lore/selected", json!({ "request": id, "end": true })).await.0, 200);
        assert_eq!(call(&app, "GET", &format!("/api/lore/selection/{id}"), Value::Null).await.1["open"], json!(false));
        assert_eq!(call(&app, "POST", "/api/lore/selected", json!({ "request": id, "after": "x" })).await.0, 410);
        assert_eq!(call(&app, "POST", "/api/lore/selected", json!({ "request": "nope", "after": "x" })).await.0, 410);
        assert_eq!(call(&app, "POST", "/api/lore/selected", json!({ "request": id })).await.0, 400);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_selection_ends_the_old_one() {
        let (ports, got) = fake_echo("replace").await;
        let app: axum::Router = router(Selections::new("sc").with_ports(ports));
        let old = call(&app, "POST", "/api/lore/selection", body("deck-a", "一", false)).await.1["request"].as_str().unwrap().to_string();
        let other = call(&app, "POST", "/api/lore/selection", body("deck-b", "二", false)).await.1["request"].as_str().unwrap().to_string();
        let new = call(&app, "POST", "/api/lore/selection", body("deck-a", "三", true)).await.1["request"].as_str().unwrap().to_string();
        assert_eq!(got.lock().unwrap().len(), 3);
        assert_eq!(call(&app, "GET", &format!("/api/lore/selection/{old}"), Value::Null).await.0, 404);
        assert_eq!(call(&app, "POST", "/api/lore/selected", json!({ "request": old, "after": "x" })).await.0, 410);
        assert_eq!(call(&app, "GET", &format!("/api/lore/selection/{other}"), Value::Null).await.1["open"], json!(true));
        assert_eq!(call(&app, "GET", &format!("/api/lore/selection/{new}"), Value::Null).await.1["open"], json!(true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn page_never_answers_504() {
        let (ports, _) = fake_echo("timeout").await;
        let app: axum::Router = router(Selections::new("sc").with_ports(ports).with_wait(Duration::from_millis(100)));
        let id = call(&app, "POST", "/api/lore/selection", body("d", "一句", false)).await.1["request"].as_str().unwrap().to_string();
        let (c, v) = call(&app, "POST", "/api/lore/selected", json!({ "request": id, "after": "改" })).await;
        assert_eq!((c, v["error"].as_str()), (504, Some("頁面沒有回應")));
        // the stale item is gone; a late done is refused
        assert_eq!(call(&app, "GET", &format!("/api/lore/selection/{id}"), Value::Null).await.1["pending"], json!([]));
        assert_eq!(call(&app, "POST", &format!("/api/lore/selection/{id}/done"), json!({ "seq": 1, "ok": true })).await.0, 410);
    }

    #[tokio::test]
    async fn echo_absent_or_unreachable_drops_the_request() {
        // launch: false and no L-Echo in ports.json: 503, nothing kept
        let sel = Selections::new("sc").with_ports(ports_file("absent"));
        let app: axum::Router = router(sel.clone());
        let (c, v) = call(&app, "POST", "/api/lore/selection", body("d", "一句", false)).await;
        assert_eq!(c, 503, "{v}");
        assert!(v["error"].as_str().unwrap().contains("L-Echo"));
        assert!(sel.open.lock().unwrap().is_empty());
        // reached nothing (a port with no listener)
        let sel = Selections::new("sc").with_echo(Arc::new(|_| {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            Ok(l.local_addr().unwrap().port())
        }));
        let app: axum::Router = router(sel.clone());
        assert_eq!(call(&app, "POST", "/api/lore/selection", body("d", "一句", true)).await.0, 503);
        assert!(sel.open.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn absurd_payloads_refused() {
        let asked: Arc<Mutex<u32>> = Arc::default();
        let a = asked.clone();
        let app: axum::Router = router(Selections::new("sc").with_echo(Arc::new(move |_| {
            *a.lock().unwrap() += 1;
            Err("no".into())
        })));
        let long = |n: usize| "字".repeat(n);
        assert_eq!(call(&app, "POST", "/api/lore/selection", body("d", &long(MAX_TEXT + 1), false)).await.0, 400);
        let mut b = body("d", "一句", false);
        b["context"] = json!(long(MAX_CONTEXT + 1));
        assert_eq!(call(&app, "POST", "/api/lore/selection", b).await.0, 400);
        assert_eq!(call(&app, "POST", "/api/lore/selection", body("d", "  ", false)).await.0, 400);
        assert_eq!(*asked.lock().unwrap(), 0, "nothing forwarded");
        // at the limit is fine (reaches the echo step)
        assert_eq!(call(&app, "POST", "/api/lore/selection", body("d", &long(MAX_TEXT), false)).await.0, 503);
        assert_eq!(*asked.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn launch_flag_reaches_the_echo_lookup() {
        let seen: Arc<Mutex<Vec<bool>>> = Arc::default();
        let s = seen.clone();
        let app: axum::Router = router(Selections::new("sc").with_echo(Arc::new(move |launch| {
            s.lock().unwrap().push(launch);
            Err("L-Echo 沒有開".into())
        })));
        let (c, v) = call(&app, "POST", "/api/lore/selection", body("d", "一句", false)).await;
        assert_eq!((c, v["error"].as_str()), (503, Some("L-Echo 沒有開")));
        call(&app, "POST", "/api/lore/selection", body("d", "一句", true)).await;
        assert_eq!(*seen.lock().unwrap(), vec![false, true]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn echo_helpers_reach_the_app_over_http() {
        let (ports, _) = fake_echo("helpers").await;
        let sel = Selections::new("sc").with_ports(ports);
        let app: axum::Router = router(sel.clone());
        let id = call(&app, "POST", "/api/lore/selection", body("d", "一句", false)).await.1["request"].as_str().unwrap().to_string();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        // the page: apply whatever arrives
        let page = sel.clone();
        let rid = id.clone();
        tokio::spawn(async move {
            loop {
                let seq = page.open.lock().unwrap().get(&rid).and_then(|o| o.pending.first().map(|p| p.seq));
                if let Some(seq) = seq {
                    let _ = page.done(&rid, Done { seq, ok: true, error: None });
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let req = SelectionRequest { request: id, app: "sc".into(), port: Some(port), doc: "d".into(), source: String::new(), text: "一句".into(), prefix: String::new(), suffix: String::new(), context: String::new(), scope: String::new(), keep: vec![], language: None, readonly: false };
        let r = tokio::task::spawn_blocking(move || (send_selected(&req, "改好", None), end_selection(&req), send_selected(&req, "再改", None))).await.unwrap();
        assert_eq!(r.0, Ok("已套用".to_string()));
        assert_eq!(r.1, Ok(()));
        assert_eq!(r.2.unwrap_err().0, 410);
    }
}
