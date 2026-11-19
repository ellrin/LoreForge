//! Synthetic spaces in temp dirs and fake L-Brain servers; never the author's KB.

use std::sync::{Arc, Mutex};

use axum::routing::get;
use http_body_util::BodyExt;
use kb_core::lore_store::{Space, SpaceConfig, SpaceKind};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;

fn space(root: &std::path::Path) -> PathBuf {
    let d = root.join("personal");
    Space::create(&d, SpaceConfig { id: "me".into(), name: "我的知識庫".into(), kind: SpaceKind::Personal, members: vec!["ellrin".into()], assets: Default::default() }).unwrap();
    d
}

fn cards_on_disk(space: &std::path::Path) -> usize {
    std::fs::read_dir(space.join("nodes")).map(|r| r.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).count()).unwrap_or(0)
}

/// A port nothing listens on.
fn dead_url() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    format!("http://127.0.0.1:{p}")
}

async fn serve(app: axum::Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    url
}

#[tokio::test]
async fn closed_l_brain_cards_go_into_the_files() {
    let d = tempfile::tempdir().unwrap();
    let sp = space(d.path());
    let b = Brain::at(Some(&dead_url()), vec![sp.clone()], "ellrin");
    assert_eq!(b.place().await, Ok(Place::Disk));
    let v = b.create(json!({ "via": "papercomposer", "title": "Momentum queue", "summary": "存放舊特徵的佇列", "initial_state": "known", "sources": [{ "type": "document", "ref": "Draft" }] })).await.unwrap();
    assert_eq!(v["via"], "file");
    let id = v["node"]["id"].as_str().unwrap().to_string();
    assert_eq!(cards_on_disk(&sp), 1);
    // read back as L-Brain would: the member's state and the source
    let view = b.view(&id).await.unwrap();
    assert_eq!((view["node"]["title"].as_str(), view["space"].as_str()), (Some("Momentum queue"), Some("me")));
    assert_eq!(view["node"]["sources"][0]["ref"], "Draft");
    assert_eq!(view["progress"]["state"], "known");
    // a review answer and an edit, also without L-Brain
    b.recall(&id, "unfamiliar", "loreecho").await.unwrap();
    let rev = view["node"]["revision"].as_u64();
    let n = b.update(&id, json!({ "revision": rev, "summary": "新的摘要" })).await.unwrap();
    assert_eq!(n["node"]["summary"], "新的摘要");
    assert_eq!(b.update(&id, json!({ "revision": rev, "summary": "舊版" })).await.unwrap_err().status, 409, "stale revision refused");
    assert_eq!(b.view("nope").await.unwrap_err().status, 404);
    // one card per term, also in the files: any case or spacing
    let e = b.create(json!({ "title": "momentum  QUEUE" })).await.unwrap_err();
    assert_eq!((e.status, e.card.as_deref()), (409, Some(id.as_str())));
    assert_eq!(cards_on_disk(&sp), 1);
}

#[tokio::test]
async fn a_running_l_brain_writes_and_the_files_are_left_alone() {
    let d = tempfile::tempdir().unwrap();
    let sp = space(d.path());
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let s = seen.clone();
    let app = axum::Router::new()
        .route("/api/spaces", get(|| async { axum::Json(json!({ "spaces": [{ "id": "home", "kind": "household", "is_member": true }, { "id": "me", "kind": "personal", "is_member": true }] })) }))
        .route(
            "/api/nodes",
            get(|| async { axum::Json(json!({ "nodes": [{ "id": "n0", "title": "Dice coefficient", "aliases": ["Dice score"] }] })) }).post(move |axum::Json(b): axum::Json<Value>| {
                let s = s.clone();
                async move {
                    s.lock().unwrap().push(b.clone());
                    (axum::http::StatusCode::CREATED, axum::Json(json!({ "node": { "id": "n1", "title": b["title"] } })))
                }
            }),
        );
    let url = serve(app).await;
    let b = Brain::at(Some(&url), vec![sp.clone()], "ellrin");
    let v = b.create(json!({ "via": "slidecomposer", "title": "Warmup schedule" })).await.unwrap();
    assert_eq!((v["via"].as_str(), v["node"]["id"].as_str()), (Some("kb"), Some("n1")));
    assert_eq!(seen.lock().unwrap()[0]["space"], "me", "the personal space");
    assert_eq!(cards_on_disk(&sp), 0, "one writer: never the files while L-Brain runs");
    // a term L-Brain already has (title or alias): refused, its id given
    let e = b.create(json!({ "title": "Dice  Score" })).await.unwrap_err();
    assert_eq!((e.status, e.card.as_deref()), (409, Some("n0")));
    assert_eq!(seen.lock().unwrap().len(), 1, "nothing posted");
}

#[tokio::test]
async fn an_l_brain_that_answers_wrongly_is_an_error_not_a_reason_to_write_files() {
    let d = tempfile::tempdir().unwrap();
    let sp = space(d.path());
    let url = serve(axum::Router::new().route("/api/spaces", get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }))).await;
    let b = Brain::at(Some(&url), vec![sp.clone()], "ellrin");
    let e = b.create(json!({ "title": "x" })).await.unwrap_err();
    assert!(!e.down && e.status == 502, "{e:?}");
    assert_eq!(cards_on_disk(&sp), 0);
    // nothing anywhere: down
    let e = Brain::at(Some(&dead_url()), vec![], "ellrin").create(json!({ "title": "x" })).await.unwrap_err();
    assert!(e.down && e.status == 503);
    assert!(Brain::none().place().await.unwrap_err().down);
}

struct Host(Arc<lore_llm::MockProvider>, lore_home::Home);

impl crate::search::Search for Host {
    async fn route(&self) -> Result<lore_llm::Route, String> {
        Ok(lore_llm::Route { task: crate::search::TASK.into(), model: lore_llm::ModelRef::Mock("m".into()), provider: self.0.clone(), fallback_used: false, skipped: vec![], effort: None, max_tokens: None, alternative: None })
    }
    fn home(&self) -> lore_home::Home {
        self.1.clone()
    }
}

fn host(d: &std::path::Path, reply: &str, sources: &[&str]) -> Arc<Host> {
    std::fs::create_dir_all(d.join("private/shared")).unwrap();
    std::fs::write(d.join("private/shared/profile.toml"), "field = \"computer vision\"\n").unwrap();
    Arc::new(Host(Arc::new(lore_llm::MockProvider::new(vec![reply.into()]).with_sources(sources)), lore_home::Home::at(d)))
}

async fn call(app: &axum::Router, body: Value) -> (u16, Value) {
    let req = axum::http::Request::builder().method("POST").uri("/api/lore/card").header("content-type", "application/json").body(axum::body::Body::from(body.to_string())).unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    let st = r.status().as_u16();
    let b = r.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

/// 加入知識庫 = 一鍵加卡: a web-searched summary, then the card; only the words and the field are sent.
#[tokio::test]
async fn the_selection_route_searches_then_adds() {
    let d = tempfile::tempdir().unwrap();
    let sp = space(d.path());
    let reply = json!({ "found": true, "summary": "兩個集合重疊程度的指標", "sources": ["https://a.org/dice"] }).to_string();
    let h = host(d.path(), &reply, &["https://a.org/dice", "https://b.org/x"]);
    let app: axum::Router = router(Arc::new(Brain::at(Some(&dead_url()), vec![sp.clone()], "ellrin")), h.clone(), "papercomposer");
    let (st, v) = call(&app, json!({ "title": "  dice   Coefficient ", "source": "Synthetic draft" })).await;
    assert_eq!((st, v["via"].as_str(), v["node"]["title"].as_str()), (201, Some("file"), Some("dice Coefficient")), "{v}");
    let id = v["node"]["id"].as_str().unwrap();
    let card: Value = serde_json::from_slice(&std::fs::read(sp.join("nodes").join(format!("{id}.json"))).unwrap()).unwrap();
    assert_eq!(card["summary"], "兩個集合重疊程度的指標");
    assert_eq!(card["sources"][0], json!({ "type": "document", "ref": "Synthetic draft" }));
    assert_eq!(card["sources"][1]["ref"], "https://a.org/dice", "the page the model named among those it read");
    let req = h.0.seen.lock().unwrap()[0].clone();
    assert!(req.web_search, "searches the web");
    let u = req.messages[0].text();
    assert!(u.contains("dice Coefficient") && u.contains("computer vision") && !u.contains("Synthetic"), "only the words and the field: {u}");
    assert_eq!(call(&app, json!({ "title": " " })).await.0, 400);
    // the same words again: 409 with the card, before any search
    let (st, v2) = call(&app, json!({ "title": "Dice Coefficient" })).await;
    assert_eq!((st, v2["id"].as_str()), (409, Some(id)));
    assert_eq!(h.0.seen.lock().unwrap().len(), 1, "no second search");
    assert_eq!(cards_on_disk(&sp), 1);
}

#[tokio::test]
async fn no_search_or_nothing_found_makes_no_card() {
    let d = tempfile::tempdir().unwrap();
    let sp = space(d.path());
    let brain = Arc::new(Brain::at(Some(&dead_url()), vec![sp.clone()], "ellrin"));
    let ok = json!({ "found": true, "summary": "x", "sources": [] }).to_string();
    let app: axum::Router = router(brain.clone(), host(d.path(), &ok, &[]), "slidecomposer");
    assert_eq!(call(&app, json!({ "title": "Warmup schedule" })).await.0, 422, "an answer without a search");
    let none = json!({ "found": false, "summary": "", "sources": [] }).to_string();
    let app: axum::Router = router(brain, host(d.path(), &none, &["https://a.org"]), "slidecomposer");
    assert_eq!(call(&app, json!({ "title": "Warmup schedule" })).await.0, 422);
    assert_eq!(cards_on_disk(&sp), 0);
    // nowhere to write: refused before any search
    let h = host(d.path(), &ok, &["https://a.org"]);
    let app: axum::Router = router(Arc::new(Brain::none()), h.clone(), "slidecomposer");
    let (st, v) = call(&app, json!({ "title": "x" })).await;
    assert_eq!((st, v["kb_down"].as_bool()), (503, Some(true)));
    assert!(h.0.seen.lock().unwrap().is_empty(), "no search spent");
}
