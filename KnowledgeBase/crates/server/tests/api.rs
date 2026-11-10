//! API tests over temp spaces (offline; nothing touches iCloud or the real private space).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use kb_core::{FixedClock, KbOptions};
use kb_server::reminders::Reminders;
use kb_server::{AppState, Config};
use serde_json::{Value, json};
use tower::ServiceExt;

fn setup() -> (tempfile::TempDir, axum::Router, Arc<FixedClock>) {
    setup_with(kb_server::llm::Llm::none())
}

fn setup_with(llm: kb_server::llm::Llm) -> (tempfile::TempDir, axum::Router, Arc<FixedClock>) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("personal");
    std::fs::create_dir_all(p.join("nodes")).unwrap();
    std::fs::write(
        p.join("space.toml"),
        format!("id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n[assets]\nbackend = \"icloud\"\nroot = \"{}\"\n", dir.path().join("icloud").display()),
    )
    .unwrap();
    let h = dir.path().join("household");
    std::fs::create_dir_all(h.join("nodes")).unwrap();
    std::fs::write(h.join("space.toml"), "id = \"home\"\nname = \"家\"\nkind = \"household\"\nmembers = [\"ellrin\", \"daidai\"]\n").unwrap();
    let config = Config { spaces: vec![p, h], member: "ellrin".into(), port: 0, web: dir.path().join("web"), intervals: Default::default(), app_dir: None };
    let clock = Arc::new(FixedClock::at("2026-11-08T21:10:00+08:00"));
    let kb = kb_server::open(&config, KbOptions { clock: clock.clone(), ..Default::default() }).unwrap();
    let rem = Reminders::isolated(&dir.path().join("app"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let app = kb_server::router(AppState::with_llm(kb, config.web.clone(), rem, llm));
    (dir, app, clock)
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn create_with_initial_state_and_a_pdf_source() {
    let (_d, app, _clock) = setup();
    let src = json!([{ "type": "pdf", "ref": "ab".repeat(32), "where": "p.3", "title": "A synthetic paper" }]);
    let (s, v) = call(&app, "POST", "/api/nodes", Some(json!({ "space": "me", "title": "Softmax", "initial_state": "known", "via": "reader", "sources": src }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["summary"]["state"], "known");
    let id = v["node"]["id"].as_str().unwrap().to_string();
    let (_, v) = call(&app, "GET", &format!("/api/nodes/{id}"), None).await;
    assert_eq!(v["progress"]["state"], "known");
    assert_eq!(v["node"]["sources"], src);
    assert!(v["node"].get("initial_state").is_none(), "not stored on the card");
    // Default stays learning; a new card cannot start stale.
    let (_, v) = call(&app, "POST", "/api/nodes", Some(json!({ "space": "me", "title": "Sigmoid" }))).await;
    assert_eq!(v["summary"]["state"], "learning");
    let (s, _) = call(&app, "POST", "/api/nodes", Some(json!({ "space": "me", "title": "Tanh", "initial_state": "stale" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v) = call(&app, "GET", "/api/health", None).await;
    assert!(v["reader_url"].as_str().unwrap().starts_with("http"));
}

#[tokio::test]
async fn card_lifecycle_scan_recall() {
    let (_d, app, clock) = setup();
    let (s, v) = call(&app, "GET", "/api/spaces", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["spaces"].as_array().unwrap().len(), 2);

    // A write without a space is refused.
    let (s, _) = call(&app, "POST", "/api/nodes", Some(json!({ "title": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, v) = call(&app, "POST", "/api/nodes", Some(json!({ "space": "me", "title": "Dice coefficient", "aliases": ["DSC"], "tags": ["metric"] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["node"]["id"].as_str().unwrap().to_string();

    let (s, v) = call(&app, "POST", "/api/scan", Some(json!({ "text": "Our DSC was 0.91.", "candidates": true }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["spans"][0]["node_id"], id);
    assert_eq!(v["spans"][0]["state"], "learning");
    assert_eq!((v["spans"][0]["start"].as_u64(), v["spans"][0]["end"].as_u64()), (Some(4), Some(7)));

    let (s, v) = call(&app, "PUT", &format!("/api/nodes/{id}"), Some(json!({ "revision": 1, "summary": "重疊指標" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["node"]["revision"], 2);
    let (s, _) = call(&app, "PUT", &format!("/api/nodes/{id}"), Some(json!({ "revision": 1, "summary": "x" }))).await;
    assert_eq!(s, StatusCode::CONFLICT);

    clock.advance_days(1);
    let (_, v) = call(&app, "GET", "/api/recall/due", None).await;
    assert_eq!(v["nodes"][0]["id"], id);
    let (s, v) = call(&app, "POST", &format!("/api/recall/{id}"), Some(json!({ "outcome": "remembered", "via": "test" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["progress"]["streak"], 1);
    clock.advance_days(3);
    let (_, v) = call(&app, "POST", &format!("/api/recall/{id}"), Some(json!({ "outcome": "remembered" }))).await;
    assert_eq!(v["progress"]["state"], "known");
    let (_, v) = call(&app, "GET", "/api/nodes?state=known", None).await;
    assert_eq!(v["nodes"].as_array().unwrap().len(), 1);

    let (_, v) = call(&app, "GET", "/api/timeline?days=30", None).await;
    assert!(!v["days"].as_array().unwrap().is_empty());
    let (_, v) = call(&app, "GET", "/api/graph?tags=true", None).await;
    assert_eq!(v["nodes"].as_array().unwrap().len(), 2);
    // Full graph with placeholders for unresolved [[links]]; local graph carries hops.
    let (s, v) = call(&app, "POST", "/api/nodes", Some(json!({ "space": "me", "title": "IoU", "body": "比較 [[Dice coefficient]] 與 [[Hausdorff distance]]", "links": [{ "to": id, "rel": "contrast" }] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let iou = v["node"]["id"].as_str().unwrap().to_string();
    let (_, v) = call(&app, "GET", "/api/graph?full=true&missing=true", None).await;
    let nodes = v["nodes"].as_array().unwrap();
    assert_eq!((nodes.len(), v["total"].as_u64(), v["truncated"].as_bool()), (3, Some(2), Some(false)));
    let ph = nodes.iter().find(|n| n["kind"] == "missing").unwrap();
    assert_eq!((ph["title"].as_str(), ph["id"].as_str()), (Some("Hausdorff distance"), Some("missing:me:hausdorff distance")));
    let dice = nodes.iter().find(|n| n["id"] == id.as_str()).unwrap();
    assert_eq!((dice["degree"].as_u64(), dice["state"].as_str(), dice["summary"].as_str()), (Some(2), Some("known"), Some("重疊指標")));
    assert!(dice["created_at"].as_str().unwrap().starts_with("2026-11-08"));
    assert!(v["edges"].as_array().unwrap().iter().any(|e| e["rel"] == "contrast" && e["origin"] == "link"));
    let (_, v) = call(&app, "GET", &format!("/api/graph?center={iou}&depth=1"), None).await;
    assert_eq!(v["center"], iou.as_str());
    assert!(v["nodes"].as_array().unwrap().iter().all(|n| n["hop"].as_u64().is_some()));
    let (_, v) = call(&app, "GET", "/api/graph?limit=1", None).await;
    assert_eq!((v["nodes"].as_array().unwrap().len(), v["truncated"].as_bool()), (1, Some(true)));
    let (_, v) = call(&app, "GET", "/api/registry?space=me", None).await;
    assert!(v["kinds"].as_array().unwrap().iter().any(|k| k["id"] == "concept"));
    // No engine connected: one-line message, nothing invented.
    let (s, v) = call(&app, "POST", "/api/draft", Some(json!({ "term": "IoU", "context": "IoU is intersection over union." }))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!v["error"].as_str().unwrap().contains('\n'));

    // Share into the household, then trash the personal card.
    let (s, v) = call(&app, "POST", &format!("/api/nodes/{id}/share"), Some(json!({ "to_space": "home" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_ne!(v["node"]["id"], id);
    let (s, _) = call(&app, "DELETE", &format!("/api/nodes/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, "GET", &format!("/api/nodes/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Undo the delete (the web UI's 復原): the card and its learning state come back.
    let (s, v) = call(&app, "POST", &format!("/api/nodes/{id}/restore"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["space"].as_str(), v["node"]["id"].as_str()), (Some("me"), Some(id.as_str())));
    let (s, v) = call(&app, "GET", &format!("/api/nodes/{id}"), None).await;
    assert_eq!((s, v["progress"]["state"].as_str()), (StatusCode::OK, Some("known")));
    let (s, _) = call(&app, "POST", &format!("/api/nodes/{id}/restore"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&app, "POST", "/api/nodes/n01K7QHM527JNK6KJFYY0WPJ28S/restore", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn assets_roundtrip_and_cors() {
    let (_d, app, _) = setup();
    let req = Request::builder().method("POST").uri("/api/assets?space=me&ext=svg").body(Body::from("<svg xmlns=\"http://www.w3.org/2000/svg\"/>")).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let url = v["url"].as_str().unwrap().to_string();
    let res = app.clone().oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "image/svg+xml");

    // NeonDeck's WKWebView (file:// page) sends `Origin: null`.
    let req = Request::builder().method("OPTIONS").uri("/api/recall/due").header("origin", "null").header("access-control-request-method", "GET").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.headers().get("access-control-allow-origin").unwrap(), "null");
    let req = Request::builder().uri("/api/health").header("origin", "https://example.com").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(res.headers().get("access-control-allow-origin").is_none());
    // NeonDeck's own `deck://` scheme.
    let req = Request::builder().uri("/api/stats").header("origin", "deck://desktop").body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.headers().get("access-control-allow-origin").unwrap(), "deck://desktop");
}

#[test]
fn workspace_template_opens() {
    let dir = tempfile::tempdir().unwrap();
    let tpl = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/KnowledgeBase");
    lore_store::copy_missing(&tpl, &dir.path().join("kb")).unwrap();
    let config = Config { spaces: vec![dir.path().join("kb")], member: "ellrin".into(), port: 0, web: dir.path().into(), intervals: Default::default(), app_dir: None };
    let kb = kb_server::open(&config, KbOptions::default()).unwrap();
    let reg = kb.registry("personal").unwrap();
    assert!(reg.has_link_type("vocab-of") && reg.has_kind("equation"));
    assert!(reg.kinds.iter().find(|k| k.id == "paper").unwrap().fields.iter().any(|f| f.name == "doi"));
    assert_eq!(kb.tags("personal").unwrap().domains.len(), 3);
}

#[tokio::test]
async fn web_files_are_revalidated() {
    // The desktop WebView must not keep showing an old app.js / styles.css after an update.
    let (dir, app, _) = setup();
    std::fs::create_dir_all(dir.path().join("web")).unwrap();
    std::fs::write(dir.path().join("web/app.js"), "// app").unwrap();
    let res = app.clone().oneshot(Request::builder().uri("/app.js").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers().get("cache-control").unwrap(), "no-cache");
}

/// One theme for every LoreForge app: `/theme.js` carries the shared choice,
/// `/api/theme` reads and sets it; the page has no palette of its own.
#[tokio::test]
async fn theme_is_the_shared_choice() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("personal");
    std::fs::create_dir_all(p.join("nodes")).unwrap();
    std::fs::write(p.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
    let config = Config { spaces: vec![p], member: "ellrin".into(), port: 0, web: kb_server::config::repo_dir().join("web"), intervals: Default::default(), app_dir: None };
    let kb = kb_server::open(&config, KbOptions::default()).unwrap();
    let rem = Reminders::isolated(&dir.path().join("app"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let home = lore_home::Home::at(dir.path().join("lore"));
    let app = kb_server::router(AppState::with_home(kb, config.web.clone(), rem, home.clone()));

    let get = |uri: &'static str| {
        let app = app.clone();
        async move {
            let res = app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
            String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
        }
    };
    let put = |id: &'static str| {
        let app = app.clone();
        async move {
            let req = Request::builder().method("PUT").uri("/api/theme").header("content-type", "application/json").body(Body::from(json!({ "theme": id }).to_string())).unwrap();
            app.oneshot(req).await.unwrap().status()
        }
    };
    assert_eq!(serde_json::from_str::<Value>(&get("/api/theme").await).unwrap()["theme"], Value::Null);
    assert!(get("/theme.js").await.starts_with("window.LORE_THEME = null;"));
    assert_eq!(put("orange").await, StatusCode::BAD_REQUEST);
    assert_eq!(put("mint").await, StatusCode::OK);
    assert_eq!(home.theme(), Some("mint"));
    assert!(get("/theme.js").await.starts_with("window.LORE_THEME = \"mint\";"));
    assert!(get("/theme.css").await.contains(":root[data-theme=\"mint\"]"));

    let index = get("/").await;
    assert!(index.contains("href=\"theme.css\"") && index.contains("<script src=\"theme.js\"></script>"));
    let styles = get("/styles.css").await;
    assert!(!styles.contains("--bg: #"), "palettes live in commons/ui/theme.css only");
}

#[tokio::test]
async fn draft_goes_through_the_engine_with_cleaned_source() {
    use lore_llm::mock::MockProvider;
    use lore_llm::router::MockFactory;
    let mock = Arc::new(MockProvider::new(vec![json!({ "summary": "  延後給標籤。 ", "supported": true }).to_string()]));
    let f = MockFactory::default().with("local:current", mock.clone());
    let (_d, app, _c) = setup_with(kb_server::llm::Llm::with_factory(f));
    let body = json!({ "term": "deferral", "context": "The model de- fers the label.", "page": 7, "source": "A synthetic paper" });
    let (s, v) = call(&app, "POST", "/api/draft", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["draft"]["summary"], "延後給標籤。");
    assert_eq!(v["draft"]["body"], "> The model defers the label.\n\n（A synthetic paper，p.7）");
    assert_eq!(v["assistant"], "mock");
    let seen = mock.seen.lock().unwrap();
    assert!(seen[0].flatten().contains("defers") && !seen[0].flatten().contains("de- fers"));
    drop(seen);
    // Nothing was saved.
    let (_, v) = call(&app, "GET", "/api/nodes", None).await;
    assert!(v.to_string().find("deferral").is_none());
}

#[tokio::test]
async fn draft_refuses_when_the_source_does_not_say() {
    use lore_llm::mock::MockProvider;
    use lore_llm::router::MockFactory;
    let mock = Arc::new(MockProvider::new(vec![json!({ "summary": "", "supported": false }).to_string()]));
    let (_d, app, _c) = setup_with(kb_server::llm::Llm::with_factory(MockFactory::default().with("local:current", mock)));
    let (s, v) = call(&app, "POST", "/api/draft", Some(json!({ "term": "X", "context": "Unrelated sentence." }))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(v["error"].as_str().unwrap().contains("沒有起草"));
}

/// The shared kit is served from the binary (the page has no icons.js of its own) and the page
/// loads it in the documented order.
#[tokio::test]
async fn shared_shell_files_are_served() {
    let (_d, app, _c) = setup();
    for (path, needle) in [("/icons.js", "const ICONS"), ("/base.css", "--top-h"), ("/shell.js", "LoreShell"), ("/notation-panel.js", "notation-panel")] {
        let res = app.clone().oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(text.contains(needle), "{path}");
    }
    let web = kb_server::config::repo_dir().join("web");
    assert!(!web.join("icons.js").exists(), "the kit's icons.js replaces the app's own");
    let index = std::fs::read_to_string(web.join("index.html")).unwrap();
    let at = |s: &str| index.find(s).unwrap_or_else(|| panic!("{s}"));
    assert!(at("theme.css") < at("base.css") && at("base.css") < at("styles.css"));
    assert!(at("theme.js") < at("icons.js") && at("icons.js") < at("shell.js") && at("shell.js") < at("app.js"));
}

/// 對照表 is the global registry in `private/shared/notation.json`: read, edit, saved there.
#[tokio::test]
async fn notation_is_the_global_registry() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("personal");
    std::fs::create_dir_all(p.join("nodes")).unwrap();
    std::fs::write(p.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
    let config = Config { spaces: vec![p], member: "ellrin".into(), port: 0, web: dir.path().join("web"), intervals: Default::default(), app_dir: None };
    let kb = kb_server::open(&config, KbOptions::default()).unwrap();
    let rem = Reminders::isolated(&dir.path().join("app"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let home = lore_home::Home::at(dir.path().join("lore"));
    let app = kb_server::router(AppState::with_home(kb, config.web.clone(), rem, home.clone()));

    let (s, v) = call(&app, "GET", "/api/notation", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["entries"], json!([]));
    let body = json!({ "version": 1, "entries": [{ "id": "speed", "kind": "symbol", "latex": "v", "name": "widget speed", "origin": "global" }] });
    let (s, v) = call(&app, "PUT", "/api/notation", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["entries"][0]["id"], "speed");
    let file = home.shared_dir().unwrap().join("notation.json");
    let saved = lore_notation::Notation::load(&file).unwrap();
    assert_eq!(saved.entries.len(), 1);
    assert!(!std::fs::read_to_string(&file).unwrap().contains("origin"), "display-only fields stay out of the file");
    let (_, v) = call(&app, "GET", "/api/notation", None).await;
    assert_eq!(v["entries"][0]["latex"], "v");
    let (s, v) = call(&app, "PUT", "/api/notation", Some(json!({ "entries": "nope" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}
