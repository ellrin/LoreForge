//! 對照表: the author's one global registry (`private/shared/notation.json`, the same file
//! PaperComposer and SlideComposer read) shown by the shared `<notation-panel>`.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /notation-panel.js | the Web Component (commons/ui), compiled in |
//! | GET | /api/notation | the registry (empty when there is no file yet) |
//! | PUT | /api/notation | replace it with what the panel sends; returns the saved registry |

use std::path::PathBuf;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use lore_notation::Notation;
use serde_json::{Value, json};

use crate::Shared;

pub fn routes() -> Router<Shared> {
    Router::new().route("/notation-panel.js", get(panel_js)).route("/api/notation", get(notation_get).put(notation_put))
}

/// The shared notation-panel Web Component (commons/ui), compiled in.
async fn panel_js() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], include_str!("../../../../commons/ui/notation-panel/notation-panel.js"))
}

fn bad(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg, "category": "invalid" }))).into_response()
}

/// Where edits go: `private/shared/notation.json`.
fn write_path(s: &Shared) -> Option<PathBuf> {
    s.home.shared_dir().map(|d| d.join("notation.json"))
}

fn load(s: &Shared) -> Result<Notation, String> {
    match s.home.shared_file("notation.json") {
        Some(p) => Notation::load(&p),
        None => Ok(Notation::default()),
    }
}

async fn notation_get(State(s): State<Shared>) -> Response {
    match load(&s) {
        Ok(n) => Json(serde_json::to_value(&n).unwrap_or(Value::Null)).into_response(),
        Err(_) => bad("對照表檔案讀不了。"),
    }
}

async fn notation_put(State(s): State<Shared>, Json(mut b): Json<Value>) -> Response {
    let Some(path) = write_path(&s) else { return bad("找不到共用資料夾，對照表沒有地方存。") };
    // Rows may carry a display-only `origin`; the file has none.
    if let Some(rows) = b.get_mut("entries").and_then(Value::as_array_mut) {
        for r in rows {
            if let Some(o) = r.as_object_mut() {
                o.remove("origin");
            }
        }
    }
    let Ok(n) = serde_json::from_value::<Notation>(b) else { return bad("對照表的內容不對。") };
    // Rows may still be half filled while being edited (the panel flags bad macros itself),
    // so there is no validation here; PaperComposer / SlideComposer check when they generate.
    if let Err(e) = n.save(&path) {
        eprintln!("kb-server: {e}");
        return bad("對照表存不了。");
    }
    Json(serde_json::to_value(&n).unwrap_or(Value::Null)).into_response()
}
