//! 搜尋檔案 through LoreEcho: the one way a Lore app finds one of the author's files.
//!
//! The app never searches itself. Its page's 「搜尋檔案」 button (`commons/ui/findfile.js`,
//! `/findfile.js`) asks the app's own server (`POST /api/lore/find`), which mints a request id,
//! brings LoreEcho up (`open -g -a LoreEcho` when it is not in ports.json as `le`) and hands it
//! the request (`POST /api/find` on LoreEcho). LoreEcho finds candidates through LoreShelf, the
//! author chooses, and LoreEcho sends each chosen file back to the app's standard receive route
//! (`POST /api/lore/picked {request, path}`; `{request, end: true}` when find mode ends). The
//! app's callback decides what to do with the file. Names and paths stay on 127.0.0.1.
//!
//! | method | path | who | what |
//! |---|---|---|---|
//! | POST | /api/lore/find | the app's page | `{purpose, exts, context}` → `{request}` (LoreEcho is asked) |
//! | GET | /api/lore/find/{request} | the app's page | `{open, added: [name], error}` (polled while open) |
//! | POST | /api/lore/picked | LoreEcho | `{request, path}` → the callback → `{ok, message}`; `{request, end: true}` closes it |
//!
//! An app mounts it with one line: `.merge(lore_shell::findfile::router(Finds::new("pc", on_pick)))`.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// LoreEcho's short name in ports.json.
pub const ECHO: &str = "le";
/// LoreEcho's application name (`open -a`).
pub const ECHO_APP: &str = "LoreEcho";
/// How long to wait for LoreEcho after launching it.
const ECHO_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// What LoreEcho is asked to find (`POST /api/find` on LoreEcho).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindRequest {
    pub request: String,
    /// The asking app's short name (ports.json).
    pub app: String,
    /// What the file is for, as the author says it (「實驗紀錄」).
    pub purpose: String,
    /// Accepted extensions, lowercase, no dot (empty = any).
    #[serde(default)]
    pub exts: Vec<String>,
    /// Where the app's server answers (else ports.json `app`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

impl FindRequest {
    /// Does `name` have one of the accepted extensions?
    pub fn accepts(&self, name: &str) -> bool {
        accepts(&self.exts, name)
    }
}

/// `name`'s extension is in `exts` (empty = any).
pub fn accepts(exts: &[String], name: &str) -> bool {
    if exts.is_empty() {
        return true;
    }
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    exts.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
}

/// What LoreEcho sends back (`POST /api/lore/picked` on the app).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Picked {
    pub request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
}

/// A chosen file, handed to the app's callback.
#[derive(Debug, Clone)]
pub struct Pick {
    pub purpose: String,
    /// What the page sent with the request (PaperComposer: `{paper}`).
    pub context: Value,
    pub path: PathBuf,
}

/// The app's callback: `Ok(message)` = taken, `Err(why)` = refused (LoreEcho shows it).
pub type OnPick = Arc<dyn Fn(Pick) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync>;
/// How the app reaches LoreEcho: hands it a request (tests: a fake).
pub type Ask = Arc<dyn Fn(&FindRequest) -> Result<(), String> + Send + Sync>;

struct Open {
    purpose: String,
    exts: Vec<String>,
    context: Value,
    added: Vec<String>,
    open: bool,
    error: Option<String>,
}

/// The app side: its open requests and what to do with a chosen file.
#[derive(Clone)]
pub struct Finds {
    app: String,
    on_pick: OnPick,
    ask: Ask,
    open: Arc<Mutex<HashMap<String, Open>>>,
}

impl Finds {
    /// `app` = the short name in ports.json; LoreEcho is the real one (launched when needed).
    pub fn new(app: &str, on_pick: OnPick) -> Finds {
        Finds { app: app.into(), on_pick, ask: Arc::new(|r: &FindRequest| ask_echo(r)), open: Arc::default() }
    }

    /// Reach LoreEcho some other way (tests).
    pub fn with_ask(mut self, ask: Ask) -> Finds {
        self.ask = ask;
        self
    }

    fn mint() -> String {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        format!("{:x}-{}", t ^ (std::process::id() as u128) << 64, N.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    /// Hand a chosen file to the callback (only for an open request, with an accepted extension).
    pub async fn picked(&self, p: Picked) -> Result<String, (u16, String)> {
        let (purpose, context, exts) = {
            let mut all = self.open.lock().unwrap();
            let Some(o) = all.get_mut(&p.request).filter(|o| o.open) else { return Err((410, "這個搜尋已經結束".into())) };
            if p.end {
                o.open = false;
                return Ok(String::new());
            }
            (o.purpose.clone(), o.context.clone(), o.exts.clone())
        };
        let Some(path) = p.path.filter(|x| !x.trim().is_empty()) else { return Err((400, "需要 path".into())) };
        let path = PathBuf::from(path);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if !accepts(&exts, &name) {
            return Err((422, format!("只接受 {}", exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(" "))));
        }
        match (self.on_pick)(Pick { purpose, context, path }).await {
            Ok(m) => {
                if let Some(o) = self.open.lock().unwrap().get_mut(&p.request) {
                    o.added.push(name);
                }
                Ok(m)
            }
            Err(e) => Err((422, e)),
        }
    }
}

#[derive(Deserialize)]
struct FindBody {
    purpose: String,
    #[serde(default)]
    exts: Vec<String>,
    #[serde(default)]
    context: Value,
}

fn port_of(headers: &axum::http::HeaderMap) -> Option<u16> {
    headers.get(axum::http::header::HOST)?.to_str().ok()?.rsplit_once(':')?.1.parse().ok()
}

fn fail(code: u16, msg: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    (axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::BAD_REQUEST), axum::Json(json!({ "error": msg }))).into_response()
}

/// The three routes above, for any app state.
pub fn router<S: Clone + Send + Sync + 'static>(finds: Finds) -> axum::Router<S> {
    use axum::extract::Path as UrlPath;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    let (f1, f2, f3) = (finds.clone(), finds.clone(), finds);
    axum::Router::new()
        .route(
            "/api/lore/find",
            post(move |headers: axum::http::HeaderMap, axum::Json(b): axum::Json<FindBody>| {
                let f = f1.clone();
                async move {
                    let exts: Vec<String> = b.exts.iter().map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect();
                    let req = FindRequest { request: Finds::mint(), app: f.app.clone(), purpose: b.purpose.trim().to_string(), exts: exts.clone(), port: port_of(&headers) };
                    if req.purpose.is_empty() {
                        return fail(400, "需要 purpose");
                    }
                    f.open.lock().unwrap().insert(req.request.clone(), Open { purpose: req.purpose.clone(), exts, context: b.context, added: vec![], open: true, error: None });
                    let ask = f.ask.clone();
                    let r2 = req.clone();
                    let res = tokio::task::spawn_blocking(move || ask(&r2)).await.unwrap_or_else(|e| Err(e.to_string()));
                    match res {
                        Ok(()) => axum::Json(json!({ "request": req.request })).into_response(),
                        Err(e) => {
                            f.open.lock().unwrap().remove(&req.request);
                            fail(503, &e)
                        }
                    }
                }
            }),
        )
        .route(
            "/api/lore/find/{request}",
            get(move |UrlPath(id): UrlPath<String>| {
                let f = f2.clone();
                async move {
                    let all = f.open.lock().unwrap();
                    match all.get(&id) {
                        Some(o) => axum::Json(json!({ "open": o.open, "added": o.added, "error": o.error })).into_response(),
                        None => fail(404, "沒有這個搜尋"),
                    }
                }
            }),
        )
        .route(
            "/api/lore/picked",
            post(move |axum::Json(p): axum::Json<Picked>| {
                let f = f3.clone();
                async move {
                    match f.picked(p).await {
                        Ok(m) => axum::Json(json!({ "ok": true, "message": m })).into_response(),
                        Err((code, e)) => fail(code, &e),
                    }
                }
            }),
        )
}

// ── LoreEcho, from either side ──────────────────────────────────

/// LoreEcho's port, launching it in the background when it is not running (macOS).
pub fn echo_port() -> Result<u16, String> {
    if let Some(p) = crate::findfile::live(ECHO) {
        return Ok(p);
    }
    if !cfg!(target_os = "macos") {
        return Err("LoreEcho 沒有開".into());
    }
    std::process::Command::new("open").args(["-g", "-a", ECHO_APP]).status().map_err(|e| format!("打不開 LoreEcho：{e}"))?;
    let until = std::time::Instant::now() + ECHO_WAIT;
    while std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(300));
        if let Some(p) = live(ECHO) {
            return Ok(p);
        }
    }
    Err("LoreEcho 沒有回應（打開 LoreEcho 再試一次）".into())
}

fn live(app: &str) -> Option<u16> {
    lore_home::ports::live().get(app).copied()
}

/// Hand a request to LoreEcho (launched when needed).
pub fn ask_echo(req: &FindRequest) -> Result<(), String> {
    let port = echo_port()?;
    post_json(port, "/api/find", &json!(req)).map(|_| ()).map_err(|(_, e)| e)
}

/// LoreEcho: send a chosen file (or `end`) back to the asking app. `Ok(message)`, or the app's refusal.
pub fn send_back(req: &FindRequest, path: Option<&str>, end: bool) -> Result<String, String> {
    let port = req.port.or_else(|| live(&req.app)).ok_or_else(|| "要求的 App 已經關了".to_string())?;
    let body = Picked { request: req.request.clone(), path: path.map(str::to_string), end };
    post_json(port, "/api/lore/picked", &json!(body)).map(|v| v["message"].as_str().unwrap_or_default().to_string()).map_err(|(_, e)| e)
}

/// A small `POST` with a JSON body to a Lore app on this machine: the JSON answer, or
/// `(status, error)` (0 = not reached).
pub fn post_json(port: u16, path: &str, body: &Value) -> Result<Value, (u16, String)> {
    use std::io::{Read, Write};
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).map_err(|e| (0, e.to_string()))?;
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(60)));
    let body = body.to_string();
    let req = format!("POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).map_err(|e| (0, e.to_string()))?;
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let out = String::from_utf8_lossy(&out);
    let (head, rest) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
    let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let text = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { unchunk(rest) } else { rest.to_string() };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if (200..300).contains(&code) {
        Ok(v)
    } else {
        Err((code, v["error"].as_str().map(str::to_string).unwrap_or_else(|| if text.trim().is_empty() { format!("HTTP {code}") } else { text.trim().to_string() })))
    }
}

fn unchunk(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some((len, after)) = rest.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(len.trim(), 16) else { break };
        if n == 0 || after.len() < n {
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn finds(log: Arc<Mutex<Vec<String>>>, asked: Arc<Mutex<Vec<FindRequest>>>) -> Finds {
        let on_pick: OnPick = Arc::new(move |p: Pick| {
            let log = log.clone();
            Box::pin(async move {
                let name = p.path.file_name().unwrap().to_string_lossy().to_string();
                if name.starts_with("bad") {
                    return Err("讀不到".to_string());
                }
                log.lock().unwrap().push(format!("{} {} {}", p.purpose, p.context["paper"].as_str().unwrap_or_default(), name));
                Ok(format!("已加入{}：{name}", p.purpose))
            })
        });
        Finds::new("pc", on_pick).with_ask(Arc::new(move |r: &FindRequest| {
            asked.lock().unwrap().push(r.clone());
            Ok(())
        }))
    }

    async fn call(app: &axum::Router, method: &str, uri: &str, body: Value) -> (u16, Value) {
        let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:7878").header("content-type", "application/json").body(if method == "GET" { Body::empty() } else { Body::from(body.to_string()) }).unwrap();
        let r = app.clone().oneshot(req).await.unwrap();
        let code = r.status().as_u16();
        let b = http_body_util::BodyExt::collect(r.into_body()).await.unwrap().to_bytes();
        (code, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn request_pick_and_end() {
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        let asked: Arc<Mutex<Vec<FindRequest>>> = Arc::default();
        let app: axum::Router = router(finds(log.clone(), asked.clone()));
        let (c, v) = call(&app, "POST", "/api/lore/find", json!({ "purpose": "實驗紀錄", "exts": [".MD", "csv"], "context": { "paper": "demo" } })).await;
        assert_eq!(c, 200, "{v}");
        let id = v["request"].as_str().unwrap().to_string();
        let r = asked.lock().unwrap()[0].clone();
        assert_eq!((r.request.as_str(), r.app.as_str(), r.purpose.as_str(), r.exts.clone(), r.port), (id.as_str(), "pc", "實驗紀錄", vec!["md".to_string(), "csv".into()], Some(7878)));

        // an accepted file → the callback, with the page's context
        let (c, v) = call(&app, "POST", "/api/lore/picked", json!({ "request": id, "path": "/tmp/x/run-1.md" })).await;
        assert_eq!((c, v["message"].as_str()), (200, Some("已加入實驗紀錄：run-1.md")));
        // a refused extension and a refusing callback
        assert_eq!(call(&app, "POST", "/api/lore/picked", json!({ "request": id, "path": "/tmp/x/a.png" })).await.0, 422);
        let (c, v) = call(&app, "POST", "/api/lore/picked", json!({ "request": id, "path": "/tmp/x/bad.csv" })).await;
        assert_eq!((c, v["error"].as_str()), (422, Some("讀不到")));
        // unknown request: nothing is read
        assert_eq!(call(&app, "POST", "/api/lore/picked", json!({ "request": "nope", "path": "/tmp/x/run-2.md" })).await.0, 410);
        let (_, v) = call(&app, "GET", &format!("/api/lore/find/{id}"), Value::Null).await;
        assert_eq!(v, json!({ "open": true, "added": ["run-1.md"], "error": null }));
        // end: closed, later picks refused
        assert_eq!(call(&app, "POST", "/api/lore/picked", json!({ "request": id, "end": true })).await.0, 200);
        assert_eq!(call(&app, "GET", &format!("/api/lore/find/{id}"), Value::Null).await.1["open"], json!(false));
        assert_eq!(call(&app, "POST", "/api/lore/picked", json!({ "request": id, "path": "/tmp/x/run-3.md" })).await.0, 410);
        assert_eq!(*log.lock().unwrap(), vec!["實驗紀錄 demo run-1.md".to_string()]);
    }

    #[tokio::test]
    async fn echo_unreachable_drops_the_request() {
        let f = finds(Arc::default(), Arc::default()).with_ask(Arc::new(|_: &FindRequest| Err("LoreEcho 沒有回應".to_string())));
        let app: axum::Router = router(f.clone());
        let (c, v) = call(&app, "POST", "/api/lore/find", json!({ "purpose": "實驗紀錄" })).await;
        assert_eq!((c, v["error"].as_str()), (503, Some("LoreEcho 沒有回應")));
        assert!(f.open.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn send_back_reaches_the_app_over_http() {
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        let f = finds(log.clone(), Arc::default());
        f.open.lock().unwrap().insert("r1".into(), Open { purpose: "實驗紀錄".into(), exts: vec!["md".into()], context: json!({ "paper": "p" }), added: vec![], open: true, error: None });
        let app: axum::Router = router(f);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let req = FindRequest { request: "r1".into(), app: "pc".into(), purpose: "實驗紀錄".into(), exts: vec!["md".into()], port: Some(port) };
        let r = tokio::task::spawn_blocking(move || (send_back(&req, Some("/tmp/a b/紀錄 (1).md"), false), send_back(&req, Some("/tmp/bad.md"), false), send_back(&req, None, true), send_back(&req, Some("/tmp/c.md"), false))).await.unwrap();
        assert_eq!(r.0.as_deref(), Ok("已加入實驗紀錄：紀錄 (1).md"));
        assert_eq!(r.1, Err("讀不到".to_string()));
        assert_eq!(r.2, Ok(String::new()));
        assert!(r.3.is_err());
        assert!(accepts(&["pdf".into()], "x.PDF") && !accepts(&["pdf".into()], "x.pdf.txt") && accepts(&[], "any"));
    }
}
