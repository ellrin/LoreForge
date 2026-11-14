//! 搜尋檔案 (find mode): another Lore app asks LoreEcho to find a file for something
//! (`lore_shell::findfile`: `{request, app, purpose, exts, port}`). The window opens in find mode;
//! the author describes the file by voice or typing, LoreEcho lists 3–5 candidates from LoreShelf
//! (filtered by the accepted extensions, never chosen for him), and each file he chooses is sent
//! back to the asking app (`POST /api/lore/picked`), which decides what to do with it. Nothing is
//! opened here. Generic: nothing in this file knows which app asks or why.
//!
//! | method | path | what |
//! |---|---|---|
//! | POST | /api/find | a request from an app → find mode (the window opens) |
//! | GET | /api/find | `{seq, find}`: the open request (the page polls) |
//! | GET | /api/find/search?q= | `{items: [{id, name, folder, date, ext, thumb}]}` (≤ 5) |
//! | GET | /api/find/thumb/{id} | LoreShelf's thumbnail |
//! | POST | /api/find/pick `{id}` | the path from LoreShelf → the asking app → `{ok, name, message}` |
//! | POST | /api/find/end | find mode ends (the app is told) |

use std::sync::Mutex;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_shell::findfile::{FindRequest, accepts, send_back};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{DesktopEvent, Shared};

/// Candidates shown at once.
pub const MAX: usize = 5;
/// Asked from LoreShelf before the extension filter.
const ASK: &str = "30";

#[derive(Default)]
pub struct Finder {
    current: Mutex<(u64, Option<FindRequest>)>,
    /// LoreShelf's address, pinned (tests, the isolated server); else ports.json while the real app runs.
    pinned: Mutex<Option<String>>,
}

impl Finder {
    pub fn pin_shelf(&self, base: &str) {
        *self.pinned.lock().unwrap() = Some(base.trim_end_matches('/').to_string());
    }

    fn current(&self) -> Option<FindRequest> {
        self.current.lock().unwrap().1.clone()
    }
}

fn shelf(s: &Shared) -> Option<String> {
    if let Some(b) = s.find.pinned.lock().unwrap().clone() {
        return Some(b);
    }
    if s.shelf.load(std::sync::atomic::Ordering::Relaxed) { crate::shelfctx::shelf_url() } else { None }
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/find", get(find_get).post(find_post))
        .route("/api/find/search", get(search))
        .route("/api/find/thumb/{id}", get(thumb))
        .route("/api/find/pick", post(pick))
        .route("/api/find/end", post(end))
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

async fn find_post(State(s): State<Shared>, Json(r): Json<FindRequest>) -> Response {
    if r.request.trim().is_empty() || r.purpose.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "需要 request 和 purpose");
    }
    let old = {
        let mut c = s.find.current.lock().unwrap();
        c.0 += 1;
        c.1.replace(r)
    };
    if let Some(old) = old {
        tokio::task::spawn_blocking(move || send_back(&old, None, true));
    }
    s.tell(DesktopEvent::Expanded(true));
    let seq = s.find.current.lock().unwrap().0;
    Json(json!({ "ok": true, "seq": seq })).into_response()
}

async fn find_get(State(s): State<Shared>) -> Json<Value> {
    let c = s.find.current.lock().unwrap();
    Json(json!({ "seq": c.0, "find": c.1.as_ref().map(|r| json!({ "request": r.request, "purpose": r.purpose, "exts": r.exts })) }))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    /// The folder it is in (`~/…`).
    pub folder: String,
    /// `YYYY-MM-DD` (downloaded, else modified, else created).
    pub date: String,
    pub ext: String,
    /// A thumbnail may be shown (not for sensitive items).
    pub thumb: bool,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(8)).no_proxy().build().unwrap_or_default()
}

/// LoreShelf's search rows that may be offered: files (no projects, packages or datasets) with an accepted extension.
pub fn rows(v: &Value, exts: &[String]) -> Vec<Candidate> {
    v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let it = &x["item"];
                    let name = it["name"].as_str()?;
                    if matches!(it["kind"].as_str(), Some("project" | "package" | "dataset")) || !accepts(exts, name) {
                        return None;
                    }
                    let t = &it["times"];
                    let date = ["downloaded", "modified", "created"].iter().find_map(|k| t[*k].as_str()).map(|s| s.chars().take(10).collect()).unwrap_or_default();
                    let ext = std::path::Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    Some(Candidate { id: it["id"].as_str()?.to_string(), name: name.to_string(), folder: String::new(), date, ext, thumb: !it["sensitive"].as_bool().unwrap_or(false) })
                })
                .take(MAX)
                .collect()
        })
        .unwrap_or_default()
}

/// The first recorded path of a LoreShelf item that still exists.
async fn path_of(base: &str, id: &str) -> Option<String> {
    let v: Value = client().get(format!("{base}/api/files/{}", enc(id))).send().await.ok()?.json().await.ok()?;
    v["paths"].as_array()?.iter().filter_map(|p| p.as_str()).find(|p| std::path::Path::new(p).exists()).map(str::to_string)
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// `path`'s folder, the home folder as `~`; a deep one as `…/<last two>`.
pub fn folder_of(path: &str, home: Option<&std::path::Path>) -> String {
    let dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let full = match home.and_then(|h| dir.strip_prefix(h).ok()) {
        Some(r) if r.as_os_str().is_empty() => return "~".into(),
        Some(r) => format!("~/{}", r.display()),
        None => dir.display().to_string(),
    };
    // a deep folder: its last two names (the row has one line for it)
    let parts: Vec<&str> = full.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() > 3 { format!("…/{}", parts[parts.len() - 2..].join("/")) } else { full }
}

/// Candidates for `q`: LoreShelf's guided search, then its sentence search when nothing fits.
pub async fn candidates(base: &str, q: &str, exts: &[String]) -> Result<Vec<Candidate>, String> {
    let mut found = vec![];
    for loose in [false, true] {
        let mut req = client().get(format!("{base}/api/search")).query(&[("q", q), ("limit", ASK)]);
        if loose {
            req = req.query(&[("loose", "1")]);
        }
        let v: Value = req.send().await.map_err(|e| format!("LoreShelf：{e}"))?.json().await.unwrap_or(Value::Null);
        found = rows(&v, exts);
        if !found.is_empty() {
            break;
        }
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    for c in &mut found {
        if let Some(p) = path_of(base, &c.id).await {
            c.folder = folder_of(&p, home.as_deref());
        }
    }
    Ok(found)
}

#[derive(serde::Deserialize)]
struct SearchQ {
    q: Option<String>,
}

async fn search(State(s): State<Shared>, Query(q): Query<SearchQ>) -> Response {
    let Some(r) = s.find.current() else { return err(StatusCode::GONE, "沒有在找檔案") };
    let Some(base) = shelf(&s) else { return err(StatusCode::SERVICE_UNAVAILABLE, "LoreShelf 沒有開") };
    let q = q.q.unwrap_or_default();
    if q.trim().is_empty() {
        return Json(json!({ "items": [] })).into_response();
    }
    match candidates(&base, q.trim(), &r.exts).await {
        Ok(items) => Json(json!({ "items": items })).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, &e),
    }
}

async fn thumb(State(s): State<Shared>, UrlPath(id): UrlPath<String>) -> Response {
    let Some(base) = shelf(&s) else { return StatusCode::NOT_FOUND.into_response() };
    match client().get(format!("{base}/api/thumb/{}", enc(&id))).query(&[("s", "160")]).send().await {
        Ok(r) if r.status().is_success() => {
            let ct = r.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("image/png").to_string();
            match r.bytes().await {
                Ok(b) => ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "max-age=600".to_string())], b).into_response(),
                Err(_) => StatusCode::BAD_GATEWAY.into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pick(State(s): State<Shared>, Json(b): Json<Value>) -> Response {
    let Some(r) = s.find.current() else { return err(StatusCode::GONE, "沒有在找檔案") };
    let Some(id) = b["id"].as_str() else { return err(StatusCode::BAD_REQUEST, "需要 id") };
    let Some(base) = shelf(&s) else { return err(StatusCode::SERVICE_UNAVAILABLE, "LoreShelf 沒有開") };
    let Some(path) = path_of(&base, id).await else { return err(StatusCode::GONE, "這個檔案已經不在原處") };
    let name = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    if !r.accepts(&name) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "這種檔案不能用");
    }
    match tokio::task::spawn_blocking(move || send_back(&r, Some(&path), false)).await {
        Ok(Ok(message)) => Json(json!({ "ok": true, "name": name, "message": message })).into_response(),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, &e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn end(State(s): State<Shared>) -> Json<Value> {
    let old = s.find.current.lock().unwrap().1.take();
    if let Some(r) = old {
        let _ = tokio::task::spawn_blocking(move || send_back(&r, None, true)).await;
    }
    Json(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_filter_kinds_and_extensions() {
        let v = json!({ "items": [
            { "item": { "id": "a", "name": "run-1.CSV", "kind": "table", "times": { "modified": "2026-11-01T10:00:00+08:00" } } },
            { "item": { "id": "b", "name": "photo.png", "kind": "image", "times": {} } },
            { "item": { "id": "c", "name": "repo", "kind": "project", "times": {} } },
            { "item": { "id": "d", "name": "id-card.pdf", "kind": "pdf", "sensitive": true, "times": { "downloaded": "2026-10-02T00:00:00Z", "modified": "2026-11-02" } } },
        ] });
        let exts = vec!["csv".to_string(), "pdf".into()];
        let r = rows(&v, &exts);
        assert_eq!(r.iter().map(|c| (c.id.as_str(), c.date.as_str(), c.ext.as_str(), c.thumb)).collect::<Vec<_>>(), vec![("a", "2026-11-01", "csv", true), ("d", "2026-10-02", "pdf", false)]);
        assert_eq!(rows(&v, &[]).len(), 3, "any extension; never a project");
        let many = json!({ "items": (0..9).map(|i| json!({ "item": { "id": format!("f{i}"), "name": format!("r{i}.md"), "kind": "text", "times": {} } })).collect::<Vec<_>>() });
        assert_eq!(rows(&many, &["md".into()]).len(), MAX);
    }

    #[test]
    fn folders_are_shown_from_home() {
        let h = std::path::Path::new("/home/ellrin");
        assert_eq!(folder_of("/home/ellrin/Documents/exp/run.csv", Some(h)), "~/Documents/exp");
        assert_eq!(folder_of("/home/ellrin/run.csv", Some(h)), "~");
        assert_eq!(folder_of("/data/run.csv", Some(h)), "/data");
        assert_eq!(folder_of("/home/ellrin/Documents/lab/2026/exp/run.csv", Some(h)), "…/2026/exp");
    }
}
