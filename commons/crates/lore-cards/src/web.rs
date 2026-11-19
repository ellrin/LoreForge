//! `POST /api/lore/card` — 加入知識庫 for selected words (`commons/ui/addcard.js`): L-Reader's
//! 一鍵加卡 for any app. `{ title, source? }` → web-searched summary ([`crate::search`], task
//! `lore-card`) → a card in state learning with the document's title and the pages read as its
//! sources → 201 `{ node: {id, title}, summary, engine, via: kb | file }`. No search, no card;
//! a term some card already has (title or alias) → 409 `{ error, id, title }` before any search.
//! Only the words and the author's field go to the model; `source` is a title, never a path.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Brain;
use crate::search::{Search, summary};

#[derive(Debug, Deserialize)]
pub struct NewCard {
    pub title: String,
    /// The document the words came from (its title).
    #[serde(default)]
    pub source: String,
}

struct Ctx<H> {
    brain: Arc<Brain>,
    host: Arc<H>,
    via: &'static str,
}

impl<H> Clone for Ctx<H> {
    fn clone(&self) -> Self {
        Ctx { brain: self.brain.clone(), host: self.host.clone(), via: self.via }
    }
}

/// The route over `brain` and the app's model (`host`); `via` = the app in L-Brain's history.
pub fn router<S: Clone + Send + Sync + 'static, H: Search>(brain: Arc<Brain>, host: Arc<H>, via: &'static str) -> axum::Router<S> {
    axum::Router::new().route("/api/lore/card", post(create::<H>)).with_state(Ctx { brain, host, via })
}

fn fail(status: u16, v: Value) -> Response {
    (StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), Json(v)).into_response()
}

async fn create<H: Search>(State(c): State<Ctx<H>>, body: Option<Json<NewCard>>) -> Response {
    let Some(Json(b)) = body else { return fail(400, json!({ "error": "要送 { title }" })) };
    let title = b.title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || title.chars().count() > 120 {
        return fail(400, json!({ "error": "詞不對" }));
    }
    // the KB first: no search is spent on a card that cannot be saved or already exists
    match c.brain.existing(&title).await {
        Err(e) => return fail(e.status, json!({ "error": e.message, "kb_down": e.down })),
        Ok(Some((id, have))) => return fail(409, json!({ "error": format!("知識庫已經有「{have}」"), "id": id, "title": have })),
        Ok(None) => {}
    }
    let (sum, pages, engine) = match summary(c.host.as_ref(), &title).await {
        Ok(x) => x,
        Err(e) => return fail(e.status, json!({ "error": e.message })),
    };
    let mut sources: Vec<Value> = vec![];
    let source = b.source.trim();
    if !source.is_empty() {
        sources.push(json!({ "type": "document", "ref": source.chars().take(200).collect::<String>() }));
    }
    sources.extend(pages.iter().map(|w| json!({ "type": "url", "ref": w.url, "title": w.title })));
    let card = json!({ "via": c.via, "kind": "concept", "title": title, "summary": sum, "initial_state": "learning", "sources": sources });
    match c.brain.create(card).await {
        Ok(v) => (StatusCode::CREATED, Json(json!({ "node": { "id": v["node"]["id"], "title": v["node"]["title"] }, "summary": sum, "engine": engine, "via": v["via"] }))).into_response(),
        Err(e) => fail(e.status, json!({ "error": e.message, "kb_down": e.down, "id": e.card })),
    }
}
