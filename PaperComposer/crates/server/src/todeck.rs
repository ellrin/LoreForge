//! 做成簡報 (ui/todeck.js): SlideComposer makes the deck, from the paper's folder as it
//! is. This server only tells the page which folder that is; nothing is written.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/papers/{name}/todeck | `{path}`: the paper's folder (workspace or opened in place) |

use super::*;

pub(crate) fn routes() -> Router<Shared> {
    Router::new().route("/api/papers/{name}/todeck", get(source))
}

async fn source(State(s): State<Shared>, AxPath(name): AxPath<String>) -> axum::response::Response {
    match paper_dir(&s, &name).await {
        Ok(dir) => {
            let dir = dir.canonicalize().unwrap_or(dir);
            Json(serde_json::json!({ "path": dir })).into_response()
        }
        Err(r) => r,
    }
}
