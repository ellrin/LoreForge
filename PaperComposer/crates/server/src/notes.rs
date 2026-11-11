//! 筆記: the workspace's `notes/` folder (`lore-notes` format; notes and 靈感 from LoreEcho or
//! written here). Every write is checked against the author's global 對照表 first; a blocking
//! finding answers 422 and writes nothing.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/notes | `{notes, dir}` newest first; `?q=` every word in title, tags or body |
//! | GET | /api/notes/macros | `{macros}`: KaTeX macros of the global registry (preview with no paper open) |
//! | GET | /api/notes/{id} | the note |
//! | POST | /api/notes | `{title, body, kind, tags, kb, source}` → `{id, path, findings}`; 422 `{error, findings}` |
//! | PUT | /api/notes/{id} | any of `{title, body, kind, tags, kb}` → `{id, path, findings}`; 422 as above |
//! | DELETE | /api/notes/{id} | → `{trash}`: moved to `notes/.trash/` |

use super::{bad, Shared};
use axum::{
    extract::{Path as AxPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use pc_core::notes::{self, Edit, Error, NewNote, Notes};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/notes", get(list).post(create))
        .route("/api/notes/macros", get(macros))
        .route("/api/notes/{id}", get(read).put(update).delete(remove))
}

async fn workspace(s: &Shared) -> Result<PathBuf, Response> {
    s.loaded.read().await.workspace.clone().ok_or_else(|| bad("no workspace"))
}

fn fail(e: Error) -> Response {
    match e {
        Error::Blocked(findings) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": e_text(&findings), "findings": findings }))).into_response(),
        Error::NotFound(_) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
        Error::BadId(_) | Error::Empty => bad(&e.to_string()),
        Error::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn e_text(findings: &[notes::Finding]) -> String {
    Error::Blocked(findings.to_vec()).to_string()
}

#[derive(Deserialize, Default)]
struct Search {
    q: Option<String>,
}

async fn list(State(s): State<Shared>, Query(q): Query<Search>) -> Response {
    let ws = match workspace(&s).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let n = notes::of(&ws);
    let q = q.q.unwrap_or_default();
    let all: Vec<_> = n.list().into_iter().filter(|x| notes::matches(x, &q)).collect();
    Json(json!({ "notes": all, "dir": n.dir() })).into_response()
}

async fn macros(State(s): State<Shared>) -> Response {
    let ws = match workspace(&s).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    Json(json!({ "macros": lore_notation::katex_macros(&crate::notation::load_global(&ws)) })).into_response()
}

async fn with_notes(s: &Shared) -> Result<(Notes, lore_notation::Notation), Response> {
    let ws = workspace(s).await?;
    Ok((notes::of(&ws), crate::notation::load_global(&ws)))
}

async fn read(State(s): State<Shared>, AxPath(id): AxPath<String>) -> Response {
    match with_notes(&s).await {
        Ok((n, _)) => n.get(&id).map(|x| Json(x).into_response()).unwrap_or_else(fail),
        Err(r) => r,
    }
}

async fn create(State(s): State<Shared>, Json(b): Json<NewNote>) -> Response {
    match with_notes(&s).await {
        Ok((n, reg)) => n.create(b, &reg).map(|x| Json(x).into_response()).unwrap_or_else(fail),
        Err(r) => r,
    }
}

async fn update(State(s): State<Shared>, AxPath(id): AxPath<String>, Json(b): Json<Edit>) -> Response {
    match with_notes(&s).await {
        Ok((n, reg)) => n.update(&id, b, &reg).map(|x| Json(x).into_response()).unwrap_or_else(fail),
        Err(r) => r,
    }
}

async fn remove(State(s): State<Shared>, AxPath(id): AxPath<String>) -> Response {
    match with_notes(&s).await {
        Ok((n, _)) => n.delete(&id).map(|to| Json(json!({ "trash": to })).into_response()).unwrap_or_else(fail),
        Err(r) => r,
    }
}
