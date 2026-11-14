//! 搜尋檔案: find a file through LoreShelf's catalog (its own search, ports.json `ls`) instead
//! of a folder dialog. LoreShelf is brought up in the background when it is not running.
//! Names and paths stay on this machine (local calls only).
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/shelf/search?q= | → `{items: [{id, name, ext, size, modified, labels}]}` (LoreShelf's guided search, then its sentence search when nothing matched) |

use super::*;
use axum::extract::Query;
use serde_json::{json, Value};
use axum::response::Response;

pub(crate) fn routes() -> Router<Shared> {
    Router::new().route("/api/shelf/search", get(search))
}

const SHELF_APP: &str = "LoreShelf";
/// How long to wait for LoreShelf after launching it.
const WAIT: std::time::Duration = std::time::Duration::from_secs(15);

fn live() -> Option<String> {
    lore_home::ports::live().get("ls").map(|p| format!("http://127.0.0.1:{p}"))
}

/// LoreShelf's address, launching it in the background (`open -g -a LoreShelf`) when needed.
async fn base() -> Result<String, String> {
    if let Some(b) = live() {
        return Ok(b);
    }
    if !cfg!(target_os = "macos") {
        return Err("LoreShelf 沒有開".into());
    }
    let _ = std::process::Command::new("open").args(["-g", "-a", SHELF_APP]).status();
    let start = std::time::Instant::now();
    while start.elapsed() < WAIT {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        if let Some(b) = live() {
            return Ok(b);
        }
    }
    Err("LoreShelf 沒有回應（打開 LoreShelf 再試一次）".into())
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().unwrap_or_default()
}

/// One result row for the page (no path: the path is fetched only for the chosen file).
fn row(item: &Value) -> Value {
    let modified = item["times"]["modified"].clone();
    json!({ "id": item["id"], "name": item["name"], "ext": item["ext"], "size": item["size"], "modified": modified, "labels": item["labels"], "kind": item["kind"] })
}

#[derive(Deserialize)]
struct Q {
    q: Option<String>,
}

async fn search(Query(q): Query<Q>) -> Response {
    let text = q.q.unwrap_or_default();
    if text.trim().is_empty() {
        return Json(json!({ "items": [] })).into_response();
    }
    let b = match base().await {
        Ok(b) => b,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, e).into_response(),
    };
    let c = client();
    let mut items: Vec<Value> = vec![];
    for loose in [false, true] {
        let mut req = c.get(format!("{b}/api/search")).query(&[("q", text.as_str()), ("limit", "12")]);
        if loose {
            req = req.query(&[("loose", "1")]);
        }
        let v: Value = match req.send().await {
            Ok(r) => r.json().await.unwrap_or(Value::Null),
            Err(e) => return (StatusCode::BAD_GATEWAY, format!("LoreShelf：{e}")).into_response(),
        };
        items = v["items"].as_array().cloned().unwrap_or_default().iter().map(|x| row(&x["item"])).filter(|x| !matches!(x["kind"].as_str(), Some("project" | "package" | "dataset"))).collect();
        if !items.is_empty() {
            break;
        }
    }
    Json(json!({ "items": items })).into_response()
}

/// The local path of a LoreShelf item (its first path that still exists).
pub(crate) async fn path_of(id: &str) -> Result<String, String> {
    let b = base().await?;
    let v: Value = client().get(format!("{b}/api/files/{}", urlencode(id))).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let paths = v["paths"].as_array().cloned().or_else(|| v["entry"]["paths"].as_array().cloned()).unwrap_or_default();
    paths.iter().filter_map(|p| p.as_str()).find(|p| Path::new(p).exists()).map(str::to_string).ok_or_else(|| "這個檔案已經不在原處".into())
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
