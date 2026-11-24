//! Hub API tests over a temp dir and one synthetic manifest (never the real folders).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use kb_core::{FixedClock, KbOptions};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;
use crate::reminders::Reminders;
use crate::{AppState, Config};

const DRAWING: &str = include_str!("../../../../../private-example/SlideComposer/flow/modules/document/page.json");

const MANIFEST: &str = r#"
[[category]]
id = "t.words"
label = "用詞"
group = "寫作"
format = "wording"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "words.csv"

[[category]]
id = "t.exprs"
label = "常用詞"
group = "寫作"
format = "expression"
writable = true
[category.native]
kind = "csv"
app_dir = "App"
path = "exprs.csv"

[[category]]
id = "t.layouts"
label = "版型檔"
format = "layout"
[category.native]
kind = "files"
app_dir = "App"
path = "layouts"

[[category]]
id = "t.refs"
label = "參考圖"
format = "figure"
writable = true
[category.native]
kind = "dir"
app_dir = "App"
path = "refs"

[[category]]
id = "t.flow"
label = "流程圖"
format = "figure"
writable = true
[category.native]
kind = "flow"
app_dir = "App"
path = "flow"

[[category]]
id = "t.own"
label = "自己的"
format = "rule"
writable = true
tasks = ["draft"]
"#;

struct Env {
    dir: tempfile::TempDir,
    app: axum::Router,
}

fn write(p: &std::path::Path, text: impl AsRef<[u8]>) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn kb_in(dir: &std::path::Path) -> (kb_core::KnowledgeBase, Config) {
    let p = dir.join("personal");
    std::fs::create_dir_all(p.join("nodes")).unwrap();
    std::fs::write(p.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
    let config = Config { spaces: vec![p], member: "ellrin".into(), port: 0, web: dir.join("web"), intervals: Default::default(), app_dir: None };
    let clock = Arc::new(FixedClock::at("2026-11-08T21:10:00+08:00"));
    let kb = crate::open(&config, KbOptions { clock, ..Default::default() }).unwrap();
    (kb, config)
}

fn setup() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let hubdir = dir.path().join("hubdata");
    let a = hubdir.join("app");
    write(&a.join("words.csv"), "avoid,prefer,note\nutilize,use,\n");
    write(&a.join("exprs.csv"), "id,kind,text,usage,section,example,tags\nE1,phrase,in contrast,,,,\n");
    write(&a.join("layouts/grid.json"), r#"{"name":"Grid","group":"basic","description":"three columns"}"#);
    write(&a.join("refs/f01/desc.md"), "# f01 Overview\n\nTwo rows of boxes.\n");
    write(&a.join("refs/f01/ref.png"), b"PNGDATA");
    write(&a.join("refs/f02/desc.md"), "# f02 No picture\n\nNothing drawn yet.\n");
    write(&hubdir.join("flow/modules/document/page.json"), DRAWING);
    let (kb, config) = kb_in(dir.path());
    let rem = Reminders::isolated(&dir.path().join("rem"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let state = HubState::isolated(&hubdir, MANIFEST).unwrap();
    let app = crate::router_isolated(AppState::with_hub(kb, config.web.clone(), rem, crate::llm::Llm::none(), state));
    Env { dir, app }
}

async fn raw(app: &axum::Router, method: &str, uri: &str, body: Option<String>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(body.map(Body::from).unwrap_or_else(Body::empty)).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (s, _, b) = raw(app, method, uri, body.map(|b| b.to_string())).await;
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

fn rule(title: &str) -> Value {
    json!({ "category": "t.own", "title": title, "fields": { "message": format!("{title} says so") } })
}

async fn add_rule(app: &axum::Router, title: &str, tags: Value) -> Value {
    let mut b = rule(title);
    b["tags"] = tags;
    let (s, v) = call(app, "POST", "/api/hub/items", Some(b)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["item"].clone()
}

fn assert_no_path(v: &Value, dir: &std::path::Path) {
    let t = v.to_string();
    assert!(!t.contains(dir.to_str().unwrap()) && !t.contains("/private/") && !t.contains("/var/folders"), "{t}");
}

// ── health, stats ────────────────────────────────────────────────────────────

#[tokio::test]
async fn without_a_hub_every_route_answers_503() {
    let dir = tempfile::tempdir().unwrap();
    let (kb, config) = kb_in(dir.path());
    let rem = Reminders::isolated(&dir.path().join("rem"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let app = crate::router_isolated(AppState::with_llm(kb, config.web.clone(), rem, crate::llm::Llm::none()));
    let (s, v) = call(&app, "GET", "/api/hub/health", None).await;
    assert_eq!((s, v["hub"].as_bool()), (StatusCode::OK, Some(false)));
    let (s, v) = call(&app, "GET", "/api/hub/items", None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(v["error"].as_str().unwrap().contains("個人空間"));
}

#[tokio::test]
async fn stats_carry_the_cards_family_first() {
    let e = setup();
    let (_, v) = call(&e.app, "GET", "/api/hub/health", None).await;
    assert_eq!(v, json!({ "ok": true, "hub": true }));
    for t in ["Softmax", "Sigmoid"] {
        let (s, _) = call(&e.app, "POST", "/api/nodes", Some(json!({ "space": "me", "title": t }))).await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let (s, v) = call(&e.app, "GET", "/api/hub/stats", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    println!("STATS {v}");
    let f = &v["stats"]["families"][0];
    assert_eq!((f["id"].as_str(), f["label"].as_str(), f["app"].as_str(), f["count"].as_u64()), (Some("kb/cards"), Some("知識卡"), Some("kb"), Some(2)));
    assert_eq!((f["added_30d"].as_u64(), f["used_30d"].as_u64(), f["gap"].as_str()), (Some(2), Some(0), Some("")));
    let weeks: Vec<u64> = f["weeks"].as_array().unwrap().iter().map(|w| w.as_u64().unwrap()).collect();
    assert_eq!((weeks.len(), weeks[11], weeks.iter().sum::<u64>()), (12, 2, 2));
    assert!(f["parts"][0]["label"].as_str().is_some_and(|l| !l.is_empty()) && f["parts"][0]["count"] == 2);
    assert!(v["stats"]["total"].as_u64().unwrap() >= 2);
    let c = &v["cards"];
    assert_eq!((c["total"].as_u64(), c["learning"].as_u64(), c["known"].as_u64(), c["due"].as_u64()), (Some(2), Some(2), Some(0), Some(0)));
    assert_eq!(v["stats"]["week_starts"].as_array().unwrap().len(), 12);
    assert_no_path(&v, e.dir.path());
}

#[test]
fn week_starts_are_twelve_mondays() {
    use chrono::Datelike;
    for today in ["2026-11-08", "2026-11-09", "2026-01-01"] {
        let t = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").unwrap();
        let m = twelve_mondays(t);
        assert_eq!(m.len(), 12);
        assert!(m.iter().all(|d| d.weekday() == chrono::Weekday::Mon));
        assert!(m[11] <= t && (t - m[11]).num_days() < 7, "last is this week's Monday");
        assert!(m.windows(2).all(|w| (w[1] - w[0]).num_days() == 7));
    }
}

// ── categories ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn categories_list_with_counts_and_formats() {
    let e = setup();
    let (s, v) = call(&e.app, "GET", "/api/hub/categories", None).await;
    assert_eq!(s, StatusCode::OK);
    println!("CATEGORIES {v}");
    let cats = v["categories"].as_array().unwrap();
    let by = |id: &str| cats.iter().find(|c| c["id"] == id).unwrap_or_else(|| panic!("{id}")).clone();
    assert_eq!((by("t.words")["count"].as_u64(), by("t.refs")["count"].as_u64(), by("t.own")["count"].as_u64()), (Some(1), Some(2), Some(0)));
    assert_eq!((by("t.own")["connected"].as_bool(), by("t.words")["connected"].as_bool()), (Some(true), Some(false)));
    let formats: Vec<&str> = v["formats"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert!(["wording", "rule", "figure", "note"].iter().all(|f| formats.contains(f)));
    assert!(v["sorts"].as_array().unwrap().contains(&json!(["title", "名稱"])));
    assert_eq!(v["facets"], json!(["domain", "kind", "use"]));
    assert_no_path(&v, e.dir.path());
}

#[tokio::test]
async fn a_hand_made_category_is_added_renamed_and_removed() {
    let e = setup();
    let (s, v) = call(&e.app, "POST", "/api/hub/categories", Some(json!({ "label": "Mine", "format": "note" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["category"]["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("my.") && v["category"]["origin"] == "manual" && v["category"]["count"] == 0);
    let (s, v) = call(&e.app, "PUT", &format!("/api/hub/categories/{id}"), Some(json!({ "label": "Mine 2", "tasks": ["draft"] }))).await;
    assert_eq!((s, v["category"]["label"].as_str(), v["category"]["connected"].as_bool()), (StatusCode::OK, Some("Mine 2"), Some(true)));
    // an app's own category is not renamed from here
    let (s, _) = call(&e.app, "PUT", "/api/hub/categories/t.own", Some(json!({ "label": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v) = call(&e.app, "POST", "/api/hub/categories", Some(json!({ "label": "Bad", "format": "nope" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // with an item it stays
    let (s, v) = call(&e.app, "POST", "/api/hub/items", Some(json!({ "category": id, "title": "N", "fields": { "body": "text" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = call(&e.app, "DELETE", &format!("/api/hub/categories/{id}"), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let item_id = call(&e.app, "GET", &format!("/api/hub/items?category={id}"), None).await.1["items"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(call(&e.app, "DELETE", &format!("/api/hub/items/{item_id}"), None).await.0, StatusCode::OK);
    let (s, v) = call(&e.app, "DELETE", &format!("/api/hub/categories/{id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = call(&e.app, "GET", "/api/hub/categories", None).await;
    assert!(v["categories"].as_array().unwrap().iter().all(|c| c["id"] != id.as_str()));
    assert_eq!(call(&e.app, "DELETE", "/api/hub/categories/t.own", None).await.0, StatusCode::FORBIDDEN);
}

// ── items ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn items_filter_sort_and_page() {
    let e = setup();
    for (t, tags) in [("Alpha", json!({ "use": ["paper"] })), ("Bravo", json!({ "use": ["slide"] })), ("Charlie", json!({ "use": ["paper"], "kind": ["timeline"] })), ("Delta", json!({}))] {
        add_rule(&e.app, t, tags).await;
    }
    let titles = |v: &Value| v["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.own&sort=title", None).await;
    assert_eq!((titles(&v), v["total"].as_u64()), (vec!["Alpha".into(), "Bravo".into(), "Charlie".into(), "Delta".into()], Some(4)));
    assert_eq!(v["tags"]["use"], json!({ "paper": 2, "slide": 1 }));
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.own&sort=title&desc=1&limit=2&offset=1", None).await;
    assert_eq!((titles(&v), v["total"].as_u64()), (vec!["Charlie".into(), "Bravo".into()], Some(4)));
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.own&tag=use:paper&tag=kind:timeline", None).await;
    assert_eq!(titles(&v), vec!["Charlie".to_string()]);
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.own&tag=use:paper&tag=use:slide&sort=title", None).await;
    assert_eq!(titles(&v), vec!["Alpha".to_string(), "Bravo".into(), "Charlie".into()]);
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.own&q=Bravo+says", None).await;
    assert_eq!(titles(&v), vec!["Bravo".to_string()]);
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.own&source=author", None).await;
    assert_eq!(v["total"], 4);
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.own&source=external:nobody", None).await;
    assert_eq!(v["total"], 0);
    let (s, _) = call(&e.app, "GET", "/api/hub/items?sort=nonsense", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&e.app, "GET", "/api/hub/items?tag=nocolon", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // a family is its categories together: `<app>/<group>`
    let (_, v) = call(&e.app, "GET", "/api/hub/items?family=t/%E5%AF%AB%E4%BD%9C&sort=title", None).await;
    assert_eq!((v["total"].as_u64(), v["items"].as_array().unwrap().len()), (Some(2), 2));
    let (_, v) = call(&e.app, "GET", "/api/hub/items?family=t/%E5%AF%AB%E4%BD%9C&sort=title&limit=1&offset=1", None).await;
    assert_eq!((v["total"].as_u64(), v["items"].as_array().unwrap().len()), (Some(2), 1));
    let (_, v) = call(&e.app, "GET", "/api/hub/items?family=t.own", None).await;
    assert_eq!(v["total"], 4);
    let (_, v) = call(&e.app, "GET", "/api/hub/items?family=nothing", None).await;
    assert_eq!((v["total"].as_u64(), v["items"].as_array().unwrap().len()), (Some(0), 0));
    let (_, v) = call(&e.app, "GET", "/api/hub/items?limit=99999", None).await;
    assert!(v["items"].as_array().unwrap().len() <= 1000);
}

#[tokio::test]
async fn rows_carry_their_line_and_thumb() {
    let e = setup();
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.exprs", None).await;
    let r = &v["items"][0];
    assert_eq!((r["title"].as_str(), r["line"].as_str(), r["thumb"].as_str()), (Some("in contrast"), Some("phrase"), Some("")));
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.refs&sort=title", None).await;
    println!("ROW {}", v["items"][0]);
    let by = |t: &str| v["items"].as_array().unwrap().iter().find(|i| i["title"] == t).unwrap().clone();
    assert_eq!(by("Overview")["thumb"], "/api/hub/thumb/t.refs:f01");
    assert_eq!(by("No picture")["thumb"], "");
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.flow", None).await;
    let drawn = v["items"].as_array().unwrap().iter().filter(|i| i["entity"] == true).collect::<Vec<_>>();
    assert!(!drawn.is_empty() && drawn.iter().all(|i| i["thumb"] == format!("/api/hub/thumb/{}", i["id"].as_str().unwrap())));
}

#[tokio::test]
async fn an_item_is_added_changed_removed_and_restored() {
    let e = setup();
    let item = add_rule(&e.app, "Short", json!({ "use": ["paper"] })).await;
    let id = item["id"].as_str().unwrap().to_string();
    assert_eq!((item["revision"].as_u64(), item["source"]["kind"].as_str(), item["format"].as_str(), item["entity"].as_bool()), (Some(1), Some("author"), Some("rule"), Some(true)));
    // a new item cannot name its id; a missing required field is listed
    let (s, _) = call(&e.app, "POST", "/api/hub/items", Some(json!({ "id": "t.own:x", "category": "t.own", "title": "x", "fields": { "message": "m" } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = call(&e.app, "POST", "/api/hub/items", Some(json!({ "category": "t.own", "title": "No message" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["problems"][0]["field"], "message");
    let (s, _) = call(&e.app, "POST", "/api/hub/items", Some(json!({ "category": "t.nowhere", "title": "x" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let mut changed = item.clone();
    changed["title"] = json!("Shorter");
    let (s, v) = call(&e.app, "PUT", &format!("/api/hub/items/{id}"), Some(changed)).await;
    assert_eq!((s, v["item"]["revision"].as_u64(), v["item"]["title"].as_str()), (StatusCode::OK, Some(2), Some("Shorter")));
    let (s, v) = call(&e.app, "GET", &format!("/api/hub/items/{id}"), None).await;
    println!("ITEM {v}");
    assert_eq!((s, v["item"]["title"].as_str(), v["contains"].clone(), v["used_by"].clone(), v["problems"].clone()), (StatusCode::OK, Some("Shorter"), json!([]), json!([]), json!([])));
    let (s, v) = call(&e.app, "DELETE", &format!("/api/hub/items/{id}"), None).await;
    assert_eq!((s, v["removed"]["id"].as_str()), (StatusCode::OK, Some(id.as_str())));
    assert_eq!(call(&e.app, "GET", &format!("/api/hub/items/{id}"), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own", None).await.1["total"], 0);
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/items/{id}/restore"), None).await;
    assert_eq!((s, v["item"]["title"].as_str()), (StatusCode::OK, Some("Shorter")));
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own", None).await.1["total"], 1);
    // an item of an app's own csv is written through its own data
    let (s, v) = call(&e.app, "POST", "/api/hub/items", Some(json!({ "category": "t.words", "title": "leverage", "fields": { "avoid": "leverage", "prefer": "use" } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert!(std::fs::read_to_string(e.dir.path().join("hubdata/app/words.csv")).unwrap().contains("leverage"));
    assert_no_path(&v, e.dir.path());
}

#[tokio::test]
async fn a_stale_write_is_a_409() {
    let e = setup();
    let item = add_rule(&e.app, "Once", json!({})).await;
    let id = item["id"].as_str().unwrap();
    let mut first = item.clone();
    first["title"] = json!("Twice");
    assert_eq!(call(&e.app, "PUT", &format!("/api/hub/items/{id}"), Some(first)).await.0, StatusCode::OK);
    let mut stale = item.clone();
    stale["title"] = json!("Lost");
    let (s, v) = call(&e.app, "PUT", &format!("/api/hub/items/{id}"), Some(stale)).await;
    assert_eq!((s, v["revision"].as_u64()), (StatusCode::CONFLICT, Some(2)), "{v}");
    assert!(v["error"].as_str().unwrap().contains("改過"));
}

#[tokio::test]
async fn a_read_only_category_is_a_403() {
    let e = setup();
    let (s, v) = call(&e.app, "POST", "/api/hub/items", Some(json!({ "category": "t.layouts", "title": "x", "fields": {} }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.layouts", None).await;
    let item = v["items"][0].clone();
    let id = item["id"].as_str().unwrap();
    assert_eq!(call(&e.app, "PUT", &format!("/api/hub/items/{id}"), Some(item.clone())).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&e.app, "DELETE", &format!("/api/hub/items/{id}"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&e.app, "POST", &format!("/api/hub/items/{id}/restore"), None).await.0, StatusCode::FORBIDDEN);
    // the tool's own generators are not removed from here either (the adapter refuses)
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.flow", None).await;
    let gen_item = v["items"].as_array().unwrap().iter().find(|i| i["fields"]["what"] == "generator").expect("a generator").clone();
    let gid = gen_item["id"].as_str().unwrap();
    assert_eq!(call(&e.app, "DELETE", &format!("/api/hub/items/{gid}"), None).await.0, StatusCode::FORBIDDEN);
}

// ── thumbnails ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_entity_thumb_is_svg() {
    let e = setup();
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.flow", None).await;
    let id = v["items"].as_array().unwrap().iter().find(|i| i["entity"] == true).unwrap()["id"].as_str().unwrap().to_string();
    let (s, h, b) = raw(&e.app, "GET", &format!("/api/hub/thumb/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h[header::CONTENT_TYPE].to_str().unwrap().starts_with("image/svg+xml"));
    assert_eq!(h[header::CACHE_CONTROL], "no-cache");
    assert!(String::from_utf8(b).unwrap().contains("<svg"));
    // a rule has no thumbnail
    let rule = add_rule(&e.app, "R", json!({})).await;
    assert_eq!(raw(&e.app, "GET", &format!("/api/hub/thumb/{}", rule["id"].as_str().unwrap()), None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_reference_thumb_is_its_picture() {
    let e = setup();
    let (s, h, b) = raw(&e.app, "GET", "/api/hub/thumb/t.refs:f01", None).await;
    assert_eq!((s, b.as_slice()), (StatusCode::OK, b"PNGDATA".as_slice()));
    assert_eq!((h[header::CONTENT_TYPE].to_str().unwrap(), h[header::CACHE_CONTROL].to_str().unwrap()), ("image/png", "no-cache"));
    assert_eq!(raw(&e.app, "GET", "/api/hub/thumb/t.refs:f02", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(raw(&e.app, "GET", "/api/hub/thumb/t.refs:nope", None).await.0, StatusCode::NOT_FOUND);
}

#[cfg(unix)]
#[tokio::test]
async fn a_thumb_never_leaves_its_folder() {
    let e = setup();
    let a = e.dir.path().join("hubdata/app");
    write(&a.join("secret.png"), b"SECRET");
    write(&a.join("refs/f03/desc.md"), "# f03 Linked\n\nA picture that is a link out.\n");
    std::os::unix::fs::symlink(a.join("secret.png"), a.join("refs/f03/ref.png")).unwrap();
    let (s, v) = call(&e.app, "GET", "/api/hub/items/t.refs:f03", None).await;
    assert_eq!((s, v["item"]["fields"]["image"].as_str()), (StatusCode::OK, Some("ref.png")), "the item names the linked picture");
    let (s, _, b) = raw(&e.app, "GET", "/api/hub/thumb/t.refs:f03", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(!String::from_utf8_lossy(&b).contains("SECRET"));
    // names that are not plain picture names inside the folder
    let f = a.join("refs/f01");
    assert!(picture_in(&f, "ref.png").is_some());
    for bad in ["../f02/desc.md", "../../secret.png", "/etc/passwd", "..", "", "ref.txt", "sub/ref.png"] {
        assert!(picture_in(&f, bad).is_none(), "{bad}");
    }
}

// ── proposals ────────────────────────────────────────────────────────────────

fn add_change(title: &str) -> Value {
    json!({ "op": "add", "note": "why", "item": { "category": "t.own", "format": "rule", "title": title, "entity": true, "fields": { "message": "m" } } })
}

#[tokio::test]
async fn a_proposal_writes_nothing_until_applied() {
    let e = setup();
    let (s, v) = call(&e.app, "POST", "/api/hub/proposals", Some(json!({ "by": "le", "title": "Two rules", "changes": [add_change("P1"), add_change("P2")] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["proposal"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["proposal"]["state"], "open");
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own", None).await.1["total"], 0);
    let (_, v) = call(&e.app, "GET", "/api/hub/proposals?state=open", None).await;
    assert_eq!((v["open"].as_u64(), v["proposals"].as_array().unwrap().len()), (Some(1), 1));
    let (s, v) = call(&e.app, "GET", &format!("/api/hub/proposals/{id}"), None).await;
    println!("PROPOSAL {v}");
    assert_eq!((s, v["shown"].as_array().unwrap().len(), v["shown"][0]["op"].as_str()), (StatusCode::OK, 2, Some("add")));
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own", None).await.1["total"], 0);
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/proposals/{id}/apply"), None).await;
    assert_eq!((s, v["proposal"]["state"].as_str()), (StatusCode::OK, Some("applied")), "{v}");
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own", None).await.1["total"], 2);
    // dismissing another one writes nothing either
    let (_, v) = call(&e.app, "POST", "/api/hub/proposals", Some(json!({ "by": "le", "title": "No", "changes": [add_change("P3")] }))).await;
    let id2 = v["proposal"]["id"].as_str().unwrap().to_string();
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/proposals/{id2}/dismiss"), None).await;
    assert_eq!((s, v["proposal"]["state"].as_str()), (StatusCode::OK, Some("dismissed")));
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own", None).await.1["total"], 2);
    assert_eq!(call(&e.app, "GET", "/api/hub/proposals/nope", None).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn part_of_a_proposal_can_be_applied_and_undone() {
    let e = setup();
    let (_, v) = call(&e.app, "POST", "/api/hub/proposals", Some(json!({ "by": "le", "title": "Two", "changes": [add_change("P1"), add_change("P2")] }))).await;
    let id = v["proposal"]["id"].as_str().unwrap().to_string();
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/proposals/{id}/apply"), Some(json!({ "chosen": [1] }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let results: Vec<&str> = v["proposal"]["changes"].as_array().unwrap().iter().map(|c| c["result"].as_str().unwrap()).collect();
    assert_eq!(results, ["skipped", "applied"]);
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.own", None).await;
    assert_eq!((v["total"].as_u64(), v["items"][0]["title"].as_str()), (Some(1), Some("P2")));
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/proposals/{id}/undo"), None).await;
    assert_eq!((s, v["not_undone"].clone()), (StatusCode::OK, json!([])), "{v}");
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own", None).await.1["total"], 0);
}

// ── outside applications ─────────────────────────────────────────────────────

fn batch(source: &str, target: Value, titles: &[&str]) -> String {
    let items: Vec<Value> = titles.iter().map(|t| json!({ "id": t.to_lowercase(), "title": t, "fields": { "message": format!("{t} m"), "avoid": t, "prefer": "x" } })).collect();
    json!({ "lore_know": 1, "source": source, "target": target, "items": items }).to_string()
}

#[tokio::test]
async fn an_aligned_batch_joins_its_category() {
    let e = setup();
    let body = batch("toolx", json!({ "kind": "align", "category": "t.own" }), &["A", "B"]);
    let (s, _, b) = raw(&e.app, "POST", "/api/hub/import", Some(body.clone())).await;
    let v: Value = serde_json::from_slice(&b).unwrap();
    println!("IMPORT {v}");
    assert_eq!((s, v["category"].as_str(), v["added"].as_u64(), v["problems"].clone()), (StatusCode::OK, Some("t.own"), Some(2), json!([])));
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own&source=external:toolx", None).await.1["total"], 2);
    // again: the same two, nothing new
    let (_, _, b) = raw(&e.app, "POST", "/api/hub/import", Some(body)).await;
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!((v["added"].as_u64(), v["replaced"].as_u64()), (Some(0), Some(2)));
    let (_, v) = call(&e.app, "GET", "/api/hub/sources", None).await;
    println!("SOURCES {v}");
    assert_eq!((v[0]["name"].as_str(), v[0]["sets"][0]["category"].as_str(), v[0]["sets"][0]["count"].as_u64(), v[0]["sets"][0]["aligned"].as_bool()), (Some("toolx"), Some("t.own"), Some(2), Some(true)));
    // a file with no source is only taken when the author says it is his own
    let own = batch("", json!({ "kind": "align", "category": "t.own" }), &["Mine"]);
    let (s, _, b) = raw(&e.app, "POST", "/api/hub/import", Some(own.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&b));
    let (s, _, b) = raw(&e.app, "POST", "/api/hub/import?own=1", Some(own)).await;
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!((s, v["added"].as_u64()), (StatusCode::OK, Some(1)), "{v}");
    assert_eq!(call(&e.app, "GET", "/api/hub/items?category=t.own&source=author", None).await.1["total"], 1);
    // garbage is a 400 with one line
    let (s, _, b) = raw(&e.app, "POST", "/api/hub/import", Some("not json".into())).await;
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(!v["error"].as_str().unwrap().contains('\n'));
}

#[tokio::test]
async fn an_independent_batch_gets_its_own_category() {
    let e = setup();
    let body = batch("toolx", json!({ "kind": "independent", "name": "words", "format": "wording" }), &["Utilize"]);
    let (s, _, b) = raw(&e.app, "POST", "/api/hub/import", Some(body)).await;
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!((s, v["category"].as_str(), v["added"].as_u64()), (StatusCode::OK, Some("x.toolx.words"), Some(1)), "{v}");
    let (_, v) = call(&e.app, "GET", "/api/hub/categories", None).await;
    let c = v["categories"].as_array().unwrap().iter().find(|c| c["id"] == "x.toolx.words").unwrap().clone();
    assert_eq!((c["origin"].as_str(), c["count"].as_u64(), c["connected"].as_bool()), (Some("external"), Some(1), Some(false)));
    // an outside application cannot bring a format
    let bad = batch("toolx", json!({ "kind": "independent", "name": "w", "format": "invented" }), &["A"]);
    assert_eq!(raw(&e.app, "POST", "/api/hub/import", Some(bad)).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn what_is_exported_imports_back() {
    let e = setup();
    let body = batch("toolx", json!({ "kind": "independent", "name": "words", "format": "wording" }), &["Utilize", "Leverage"]);
    raw(&e.app, "POST", "/api/hub/import", Some(body)).await;
    let (s, h, b) = raw(&e.app, "GET", "/api/hub/export?category=x.toolx.words", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("attachment; filename=\"x.toolx.words"));
    let text = String::from_utf8(b).unwrap();
    let exported: Value = serde_json::from_str(&text).unwrap();
    assert_eq!((exported["source"].as_str(), exported["items"].as_array().unwrap().len()), (Some("toolx"), 2));
    let (s, _, b) = raw(&e.app, "POST", "/api/hub/import", Some(text)).await;
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!((s, v["added"].as_u64(), v["replaced"].as_u64(), v["removed"].as_u64()), (StatusCode::OK, Some(0), Some(2), Some(0)), "{v}");
    // one source's part, and the usual refusals
    assert_eq!(raw(&e.app, "GET", "/api/hub/export?category=x.toolx.words&source=external:toolx", None).await.0, StatusCode::OK);
    assert_eq!(raw(&e.app, "GET", "/api/hub/export", None).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(raw(&e.app, "GET", "/api/hub/export?category=nope", None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(raw(&e.app, "GET", "/api/hub/export?category=t.own&source=bogus", None).await.0, StatusCode::BAD_REQUEST);
}

// ── the flow editor ──────────────────────────────────────────────────────────

#[tokio::test]
async fn the_flow_editor_page_is_served() {
    let e = setup();
    let (s, _, b) = raw(&e.app, "GET", "/flow/", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!b.is_empty());
}

#[test]
fn errors_are_one_line_without_paths() {
    assert_eq!(one_line("讀不了 /Users/x/private/a.csv\n再試"), "讀不了 a.csv 再試");
    assert_eq!(one_line("找不到：t.own:k1"), "找不到：t.own:k1");
}

/// L-Echo's split of a figure: the picture waits beside the proposal and becomes the picture of
/// the reference once that reference is approved; a proposal applied without it leaves it waiting.
#[tokio::test]
async fn a_proposals_picture_becomes_the_approved_references_picture() {
    use base64::Engine;
    let e = setup();
    let png: Vec<u8> = [&[0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..], b"synthetic"].concat();
    let data_url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));
    let fig = json!({ "id": "new:1", "category": "t.refs", "format": "figure", "title": "Gated net", "level": "figure", "entity": false, "fields": { "what": "reference", "description": "A net with a gate.", "image": "le-u1.png" } });
    let (s, v) = call(&e.app, "POST", "/api/hub/proposals", Some(json!({ "by": "le", "title": "one figure", "changes": [{ "op": "add", "note": "new", "item": fig }] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["proposal"]["id"].as_str().unwrap().to_string();
    // not a picture: refused
    let (s, _) = call(&e.app, "POST", &format!("/api/hub/proposals/{id}/picture"), Some(json!({ "name": "le-u1.png", "data_url": "data:image/png;base64,QUJD" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/proposals/{id}/picture"), Some(json!({ "name": "le-u1.png", "data_url": data_url }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(&e.app, "POST", &format!("/api/hub/proposals/{id}/apply"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = call(&e.app, "GET", "/api/hub/items?category=t.refs&q=Gated", None).await;
    let row = &v["items"][0];
    assert_eq!(row["title"], "Gated net", "{v}");
    assert_eq!(row["fields"]["image"], "ref.png", "the reference's reader names the placed picture: {row}");
    let (s, h, b) = raw(&e.app, "GET", row["thumb"].as_str().unwrap(), None).await;
    assert_eq!((s, h[header::CONTENT_TYPE].to_str().unwrap()), (StatusCode::OK, "image/png"));
    assert_eq!(b, png);
}
