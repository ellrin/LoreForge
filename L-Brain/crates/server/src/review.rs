//! 確認: the routes over `lore_know::review` (the author decides, one candidate at a time, what a
//! model wrote for his own sentences). Merged into `hub::routes`; errors and the hub guard are
//! the hub's (`hub::hub`, `hub::ok`, `hub::HubError`, `hub::row`).
//!
//! | | | body → answer |
//! |---|---|---|
//! | GET | /api/hub/review | → `{sets: [{id, title, by, created_at, counts, groups}], open}` (`groups` = how many of his own items the set covers; sets with nothing open come last) |
//! | POST | /api/hub/review | `{by, title, category, candidates: [{of, kind, text, slot?, note?}]}` → 201 `{set}` or 200 `{set: null}` when every candidate was already decided or waiting |
//! | GET | /api/hub/review/{set} | → `{set: {id, title, by, category, created_at}, counts, groups: [{of, own, candidates}]}` (`own` = the item as a list row, `hub::row`, or null) |
//! | POST | /api/hub/review/{set}/{candidate}/accept | → `{candidate, counts}` |
//! | POST | /api/hub/review/{set}/{candidate}/reject | → `{candidate, counts}` |
//! | POST | /api/hub/review/{set}/{candidate}/undo | → `{candidate, counts}` |
//!
//! Accept and undo go through `Hub::accept_review` / `Hub::undo_review` (they reindex).

use axum::Router;

use crate::Shared;

pub fn routes() -> Router<Shared> {
    Router::new()
}
