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

use std::collections::BTreeSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_know::review::{self, Candidate};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;
use crate::hub::{HubError, R, hub, ok, row};

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/hub/review", get(sets).post(set_add))
        .route("/api/hub/review/{set}", get(set_view))
        .route("/api/hub/review/{set}/{candidate}/accept", post(accept))
        .route("/api/hub/review/{set}/{candidate}/reject", post(reject))
        .route("/api/hub/review/{set}/{candidate}/undo", post(undo))
}

async fn sets(State(s): State<Shared>) -> R {
    let h = hub(&s)?;
    let all = h.reviews.list();
    let mut open = 0;
    let mut waiting = vec![];
    let mut finished = vec![];
    for set in &all {
        let counts = review::counts(set);
        open += counts.open;
        let groups = set.candidates.iter().map(|c| c.of.as_str()).collect::<BTreeSet<_>>().len();
        let v = json!({ "id": set.id, "title": set.title, "by": set.by, "created_at": set.created_at, "counts": counts, "groups": groups });
        if counts.open > 0 { waiting.push(v) } else { finished.push(v) }
    }
    waiting.extend(finished);
    ok(json!({ "sets": waiting, "open": open }))
}

#[derive(Deserialize)]
struct NewCandidate {
    #[serde(default)]
    of: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    slot: String,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize)]
struct NewSet {
    #[serde(default)]
    by: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    candidates: Vec<NewCandidate>,
}

async fn set_add(State(s): State<Shared>, Json(b): Json<NewSet>) -> Result<(StatusCode, Json<Value>), HubError> {
    let h = hub(&s)?;
    let candidates = b.candidates.into_iter().map(|c| Candidate { of: c.of, kind: c.kind, text: c.text, slot: c.slot, note: c.note, ..Default::default() }).collect();
    match h.reviews.create(&b.by, &b.title, &b.category, candidates)? {
        Some(set) => Ok((StatusCode::CREATED, Json(json!({ "set": set })))),
        None => Ok((StatusCode::OK, Json(json!({ "set": null })))),
    }
}

async fn set_view(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let h = hub(&s)?;
    let set = h.reviews.get(&id)?;
    let groups: Vec<Value> = review::groups(&h.store, &set)?
        .into_iter()
        .map(|g| json!({ "of": g.of, "own": g.own.as_ref().map(|i| row(&h, i)), "candidates": g.candidates }))
        .collect();
    ok(json!({
        "set": { "id": set.id, "title": set.title, "by": set.by, "category": set.category, "created_at": set.created_at },
        "counts": review::counts(&set),
        "groups": groups,
    }))
}

fn answer(h: &lore_know::hub::Hub, set: &str, c: Candidate) -> R {
    let counts = review::counts(&h.reviews.get(set)?);
    ok(json!({ "candidate": c, "counts": counts }))
}

async fn accept(State(s): State<Shared>, Path((set, cand)): Path<(String, String)>) -> R {
    let mut h = hub(&s)?;
    let c = h.accept_review(&set, &cand)?;
    answer(&h, &set, c)
}

async fn reject(State(s): State<Shared>, Path((set, cand)): Path<(String, String)>) -> R {
    let h = hub(&s)?;
    let c = review::reject(&h.reviews, &set, &cand)?;
    answer(&h, &set, c)
}

async fn undo(State(s): State<Shared>, Path((set, cand)): Path<(String, String)>) -> R {
    let mut h = hub(&s)?;
    let c = h.undo_review(&set, &cand)?;
    answer(&h, &set, c)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use kb_core::{FixedClock, KbOptions};
    use tower::ServiceExt;

    use super::*;
    use crate::hub::HubState;
    use crate::reminders::Reminders;
    use crate::{AppState, Config};

    const MANIFEST: &str = r#"
[[category]]
id = "t.patterns"
label = "句型"
format = "pattern"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "phrases.csv"

[[category]]
id = "t.variants"
label = "說法"
format = "variant"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "variants.csv"
"#;

    struct Env {
        dir: tempfile::TempDir,
        app: axum::Router,
    }

    fn setup() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let hubdir = dir.path().join("hubdata");
        let a = hubdir.join("app");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(
            a.join("phrases.csv"),
            "id,section,function,position,template,example,tags,priority\nP001,intro,gap,opening,The {thing} was measured with {tool}.,,,\n",
        )
        .unwrap();
        std::fs::write(a.join("variants.csv"), "id,pattern,kind,text,slot,note\n").unwrap();
        let p = dir.path().join("personal");
        std::fs::create_dir_all(p.join("nodes")).unwrap();
        std::fs::write(p.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
        let config = Config { spaces: vec![p], member: "ellrin".into(), port: 0, web: dir.path().join("web"), intervals: Default::default(), app_dir: None };
        let clock = Arc::new(FixedClock::at("2026-11-08T21:10:00+08:00"));
        let kb = crate::open(&config, KbOptions { clock, ..Default::default() }).unwrap();
        let rem = Reminders::isolated(&dir.path().join("rem"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
        let state = HubState::isolated(&hubdir, MANIFEST).unwrap();
        let app = crate::router_isolated(AppState::with_hub(kb, config.web.clone(), rem, crate::llm::Llm::none(), state));
        Env { dir, app }
    }

    async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    fn body(texts: &[&str]) -> Value {
        let cs: Vec<Value> = texts.iter().map(|t| json!({ "of": "t.patterns:P001", "kind": "variant", "text": t, "note": "when it fits" })).collect();
        json!({ "by": "relabel", "title": "variants", "category": "t.variants", "candidates": cs })
    }

    async fn make(app: &axum::Router, texts: &[&str]) -> String {
        let (s, v) = call(app, "POST", "/api/hub/review", Some(body(texts))).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["set"]["id"].as_str().unwrap().to_string()
    }

    fn csv(e: &Env) -> String {
        std::fs::read_to_string(e.dir.path().join("hubdata/app/variants.csv")).unwrap()
    }

    #[tokio::test]
    async fn a_set_is_created_listed_and_shown_in_groups() {
        let e = setup();
        let id = make(&e.app, &["We measured the {thing} with {tool}.", "The {thing} was assessed with {tool}."]).await;
        let (s, v) = call(&e.app, "GET", "/api/hub/review", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["open"], 2);
        assert_eq!(v["sets"][0]["id"], id.as_str());
        assert_eq!(v["sets"][0]["groups"], 1);
        assert_eq!(v["sets"][0]["counts"]["open"], 2);
        let (s, v) = call(&e.app, "GET", &format!("/api/hub/review/{id}"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["set"]["category"], "t.variants");
        assert_eq!(v["groups"].as_array().unwrap().len(), 1);
        assert_eq!(v["groups"][0]["of"], "t.patterns:P001");
        assert_eq!(v["groups"][0]["own"]["id"], "t.patterns:P001");
        assert_eq!(v["groups"][0]["candidates"].as_array().unwrap().len(), 2);
        assert_eq!(v["counts"]["open"], 2);
    }

    #[tokio::test]
    async fn accept_writes_the_row_into_the_variants_csv() {
        let e = setup();
        let id = make(&e.app, &["We measured the {thing} with {tool}."]).await;
        let (s, v) = call(&e.app, "POST", &format!("/api/hub/review/{id}/c1/accept"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["candidate"]["state"], "accepted");
        assert_eq!(v["counts"]["accepted"], 1);
        assert_eq!(v["counts"]["open"], 0);
        let text = csv(&e);
        let row = text.lines().nth(1).unwrap_or_else(|| panic!("no row in {text}"));
        assert!(row.contains("P001") && row.contains("variant") && row.contains("We measured the {thing} with {tool}."), "{row}");
        // accepting again is refused
        let (s, _) = call(&e.app, "POST", &format!("/api/hub/review/{id}/c1/accept"), None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(csv(&e).lines().count(), 2);
    }

    #[tokio::test]
    async fn reject_then_undo_reopens() {
        let e = setup();
        let id = make(&e.app, &["one", "two"]).await;
        let (s, v) = call(&e.app, "POST", &format!("/api/hub/review/{id}/c1/reject"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["candidate"]["state"], "rejected");
        assert_eq!(v["counts"]["rejected"], 1);
        assert_eq!(csv(&e).lines().count(), 1);
        // never offered again
        let (s, v) = call(&e.app, "POST", "/api/hub/review", Some(body(&["one"]))).await;
        assert_eq!((s, &v["set"]), (StatusCode::OK, &Value::Null));
        let (s, v) = call(&e.app, "POST", &format!("/api/hub/review/{id}/c1/undo"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["candidate"]["state"], "open");
        assert_eq!(v["counts"]["open"], 2);
        // undoing an open one is refused
        let (s, _) = call(&e.app, "POST", &format!("/api/hub/review/{id}/c1/undo"), None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn undo_after_accept_takes_the_row_out_of_the_csv() {
        let e = setup();
        let id = make(&e.app, &["one"]).await;
        call(&e.app, "POST", &format!("/api/hub/review/{id}/c1/accept"), None).await;
        assert_eq!(csv(&e).lines().count(), 2);
        let (s, v) = call(&e.app, "POST", &format!("/api/hub/review/{id}/c1/undo"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["candidate"]["state"], "open");
        assert_eq!(v["candidate"]["item"], "");
        assert_eq!(csv(&e).lines().count(), 1, "{}", csv(&e));
    }

    #[tokio::test]
    async fn unknown_ids_are_404() {
        let e = setup();
        let id = make(&e.app, &["one"]).await;
        let (s, _) = call(&e.app, "GET", "/api/hub/review/rnope", None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        for act in ["accept", "reject", "undo"] {
            let (s, _) = call(&e.app, "POST", &format!("/api/hub/review/rnope/c1/{act}"), None).await;
            assert_eq!(s, StatusCode::NOT_FOUND, "{act}");
            let (s, _) = call(&e.app, "POST", &format!("/api/hub/review/{id}/c9/{act}"), None).await;
            assert_eq!(s, StatusCode::NOT_FOUND, "{act}");
        }
    }

    #[tokio::test]
    async fn a_second_post_with_the_same_candidates_makes_no_set() {
        let e = setup();
        make(&e.app, &["one", "two"]).await;
        let (s, v) = call(&e.app, "POST", "/api/hub/review", Some(body(&["one", "two"]))).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["set"], Value::Null);
        let (_, v) = call(&e.app, "GET", "/api/hub/review", None).await;
        assert_eq!(v["sets"].as_array().unwrap().len(), 1);
        let (s, _) = call(&e.app, "POST", "/api/hub/review", Some(json!({ "by": "x", "title": "t", "category": "t.variants", "candidates": [] }))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
}
