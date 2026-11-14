//! `/api/polish*`, the same in every app (`commons/ui/polish.js` calls them):
//!
//! | method | path | |
//! |---|---|---|
//! | POST | /api/polish | [`ProposeBody`] → [`Proposal`] `{ id, passage, after, ok, problems, engine }` |
//! | POST | /api/polish/applied | `{ doc, id }`: 套用 happened → the entry |
//! | POST | /api/polish/like | `{ doc, id }`: 👍 → the entry (tone library + log) |
//! | GET | /api/polish/history | `?doc=&passage=` or `?doc=&text=` → `{ passage, items }` |
//! | GET | /api/polish/presets | the app's presets `[{ id, label }]` (empty: none) |

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::{Error, Host, ProposeBody};

type H = Arc<dyn Host>;

pub fn router<S: Clone + Send + Sync + 'static>(host: H) -> Router<S> {
    Router::new()
        .route("/api/polish", post(propose))
        .route("/api/polish/applied", post(applied))
        .route("/api/polish/like", post(like))
        .route("/api/polish/history", get(history))
        .route("/api/polish/presets", get(presets))
        .with_state(host)
}

fn err(e: Error) -> Response {
    let code = match e {
        Error::Bad(_) => StatusCode::BAD_REQUEST,
        Error::NotFound(_) => StatusCode::NOT_FOUND,
        Error::Model(_) => StatusCode::BAD_GATEWAY,
        Error::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (code, Json(json!({ "error": e.message() }))).into_response()
}

async fn propose(State(h): State<H>, Json(b): Json<ProposeBody>) -> Response {
    match crate::propose(h.as_ref(), b).await {
        Ok(p) => Json(p).into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
struct IdBody {
    doc: String,
    id: String,
}

async fn applied(State(h): State<H>, Json(b): Json<IdBody>) -> Response {
    match crate::applied(h.as_ref(), &b.doc, &b.id).await {
        Ok(e) => Json(e).into_response(),
        Err(e) => err(e),
    }
}

async fn like(State(h): State<H>, Json(b): Json<IdBody>) -> Response {
    match crate::like(h.as_ref(), &b.doc, &b.id).await {
        Ok(e) => Json(e).into_response(),
        Err(e) => err(e),
    }
}

async fn presets(State(h): State<H>) -> Response {
    Json(h.presets()).into_response()
}

#[derive(Deserialize)]
struct HistoryQuery {
    doc: String,
    passage: Option<String>,
    text: Option<String>,
}

async fn history(State(h): State<H>, Query(q): Query<HistoryQuery>) -> Response {
    match crate::history(h.as_ref(), &q.doc, q.passage.as_deref(), q.text.as_deref()).await {
        Ok((passage, items)) => Json(json!({ "passage": passage, "items": items })).into_response(),
        Err(e) => err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testhost::TestHost;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let s = res.status();
        (s, serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn the_routes() {
        let dir = tempfile::tempdir().unwrap();
        let app: Router = router(Arc::new(TestHost::new(dir.path(), &["Fast ⟦1⟧.", "Quick."])));
        let (st, p) = call(&app, "POST", "/api/polish", Some(json!({ "doc": "P", "text": "Very fast $x$.", "instruction": "shorter" }))).await;
        assert_eq!(st, StatusCode::OK, "{p}");
        assert_eq!((p["after"].as_str(), p["ok"].as_bool()), (Some("Fast $x$."), Some(true)));
        let id = p["id"].as_str().unwrap();
        assert_eq!(call(&app, "POST", "/api/polish/applied", Some(json!({ "doc": "P", "id": id }))).await.1["applied"], json!(true));
        assert_eq!(call(&app, "POST", "/api/polish/like", Some(json!({ "doc": "P", "id": id }))).await.1["liked"], json!(true));
        let (_, h) = call(&app, "GET", &format!("/api/polish/history?doc=P&passage={}", p["passage"].as_str().unwrap()), None).await;
        assert_eq!(h["items"].as_array().unwrap().len(), 1);
        // the math went missing: shown, not applicable
        let (_, p) = call(&app, "POST", "/api/polish", Some(json!({ "doc": "P", "text": "Very fast $x$.", "instruction": "shorter" }))).await;
        assert_eq!((p["ok"].as_bool(), p["problems"][0].as_str()), (Some(false), Some("少了 $x$")));
        assert_eq!(call(&app, "POST", "/api/polish", Some(json!({ "doc": "P", "text": "", "instruction": "x" }))).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call(&app, "POST", "/api/polish/like", Some(json!({ "doc": "P", "id": "nope" }))).await.0, StatusCode::NOT_FOUND);
        assert_eq!(call(&app, "GET", "/api/polish/presets", None).await.1, json!([]), "no presets by default");
        let mut t = TestHost::new(dir.path(), &[]);
        t.presets = vec![crate::Preset { id: "diction".into(), label: "用字".into(), instruction: "secret block".into() }];
        let app: Router = router(Arc::new(t));
        assert_eq!(call(&app, "GET", "/api/polish/presets", None).await.1, json!([{ "id": "diction", "label": "用字" }]), "the instruction stays on the server");
    }
}
