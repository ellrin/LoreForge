use super::*;
use axum::body::Body;
use axum::http::Request;
use std::sync::Mutex;
use tower::ServiceExt;

const TASKS: &str = r#"
[task.pc-draft-turn]
label = "草稿問答"
class = "ask"
default = "claude:haiku"
fallback = "local:current"

[task.pc-compose-paragraph]
label = "撰寫段落"
class = "create"
default = "local:current"
fallback = "claude:selected"

[task.pc-engine-test]
default = "local:current"
"#;

/// `router()` installs process-wide routing: one test at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn engines(dir: &std::path::Path) -> Engines {
    Engines::new(Home::at(dir), "PaperComposer", TASKS).isolated()
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let req = req.body(body.map_or_else(Body::empty, |b| Body::from(b.to_string()))).unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn get_follows_the_contract() {
    let _g = SERIAL.lock().await;
    let d = tempfile::tempdir().unwrap();
    let app: axum::Router = router(engines(d.path()));
    let (st, v) = call(&app, "GET", "/api/engines", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["local"]["state"], "missing", "no engine, no models: {v}");
    assert_eq!(v["local"]["models"], json!([]));
    let c = &v["claude"];
    assert_eq!((c["state"].as_str(), c["via"].as_str()), (Some("missing"), Some("subscription")), "{c}");
    assert_eq!(c["vias"], json!({ "subscription": false, "api": false }));
    assert_eq!(c["model"], "sonnet", "the catalog default, as its alias");
    assert_eq!(c["vision"], false);
    let ids: Vec<&str> = c["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["opus", "sonnet", "haiku"]);
    assert_eq!(c["models"][0]["label"], "Opus");
    assert_eq!(
        v["classes"],
        json!([
            { "id": "create", "label": "創作與審稿", "route": "auto", "auto": ["claude", "local"] },
            { "id": "tidy", "label": "整理與格式", "route": "auto", "auto": ["local", "claude"] },
            { "id": "ask", "label": "問答", "route": "auto", "auto": ["local", "claude"] },
        ])
    );
    let turn = v["tasks"].as_array().unwrap().iter().find(|t| t["id"] == "pc-draft-turn").unwrap();
    assert_eq!((turn["label"].as_str(), turn["class"].as_str()), (Some("草稿問答"), Some("ask")));
    let test = v["tasks"].as_array().unwrap().iter().find(|t| t["id"] == "pc-engine-test").unwrap();
    assert!(test["class"].is_null() && test["last"].is_null());
    assert_eq!(v["overrides"], json!({ "pc-compose-paragraph": "auto", "pc-draft-turn": "auto", "pc-engine-test": "auto" }));

    // the shared engine (not probed in tests): off, with its model and catalog label
    Home::at(d.path()).set_engine(Some("http://127.0.0.1:9/v1"), Some("qwen3.5-9b-q8")).unwrap();
    let (_, v) = call(&app, "GET", "/api/engines", None).await;
    let l = &v["local"];
    assert_eq!((l["state"].as_str(), l["url"].as_str(), l["model"].as_str()), (Some("off"), Some("http://127.0.0.1:9/v1"), Some("qwen3.5-9b-q8")));
    assert_eq!(l["label"], "Qwen3.5 9B");
    assert_eq!(l["vision"], true);
    assert_eq!(l["models"][0]["id"], "qwen3.5-9b-q8");
    shared::uninstall();
}

#[tokio::test]
async fn put_writes_the_shared_file_and_routes_follow() {
    let _g = SERIAL.lock().await;
    let d = tempfile::tempdir().unwrap();
    let app: axum::Router = router(engines(d.path()));
    let patch = json!({
        "claude": { "via": "api", "model": "opus" },
        "routes": { "create": "cloud", "ask": "local" },
        "overrides": { "pc-draft-turn": "cloud" },
    });
    let (st, v) = call(&app, "PUT", "/api/engines", Some(patch)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!((v["claude"]["via"].as_str(), v["claude"]["model"].as_str(), v["claude"]["state"].as_str()), (Some("api"), Some("opus"), Some("off")));
    assert_eq!(v["classes"][0]["route"], "cloud");
    assert_eq!(v["classes"][2]["route"], "local");
    assert_eq!(v["overrides"]["pc-draft-turn"], "cloud");
    let file = std::fs::read_to_string(d.path().join("private/shared/engine.toml")).unwrap();
    for line in ["via = \"api\"", "model = \"opus\"", "create = \"cloud\"", "pc-draft-turn = \"cloud\""] {
        assert!(file.contains(line), "{file}");
    }
    assert!(!file.to_lowercase().contains("key"), "never a key in the file");

    // the installed routing now reads the file
    let s = shared::installed().unwrap();
    assert_eq!(s.claude_via, Some(lore_llm::router::ClaudeVia::Api));
    assert_eq!(s.mode_for("pc-draft-turn", Some(TaskClass::Ask)), lore_llm::RouteMode::Cloud);
    let t = TaskTable::parse(TASKS).unwrap();
    assert_eq!(t.candidates_for("pc-compose-paragraph", &Default::default()), vec!["claude-api:opus"]);

    // auto / null clear an override
    let (_, v) = call(&app, "PUT", "/api/engines", Some(json!({ "overrides": { "pc-draft-turn": null } }))).await;
    assert_eq!(v["overrides"]["pc-draft-turn"], "auto");
    assert!(!std::fs::read_to_string(d.path().join("private/shared/engine.toml")).unwrap().contains("pc-draft-turn"));
    shared::uninstall();
}

#[tokio::test]
async fn bad_values_are_400_and_nothing_is_written() {
    let _g = SERIAL.lock().await;
    let d = tempfile::tempdir().unwrap();
    let app: axum::Router = router(engines(d.path()));
    for patch in [
        json!({ "routes": { "create": "sky" } }),
        json!({ "routes": { "draw": "auto" } }),
        json!({ "overrides": { "lr-chat": "local" } }),
        json!({ "overrides": { "pc-draft-turn": "nowhere" } }),
        json!({ "claude": { "via": "pigeon" } }),
        json!({ "claude": { "model": "gpt" } }),
        json!({ "local": { "model": "qwen3.5-9b-q8" } }),
        // a good part with a bad one: neither is written
        json!({ "routes": { "tidy": "local" }, "claude": { "model": 3 } }),
        json!([1, 2]),
    ] {
        let (st, v) = call(&app, "PUT", "/api/engines", Some(patch.clone())).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{patch}");
        assert!(v["error"].as_str().is_some_and(|e| !e.is_empty() && !e.contains('\n')), "{v}");
    }
    let (st, _) = call(&app, "PUT", "/api/engines", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(!d.path().join("private/shared/engine.toml").exists());
    shared::uninstall();
}

#[tokio::test]
async fn an_app_with_a_runtime_checks_the_model_before_starting_it() {
    let _g = SERIAL.lock().await;
    let d = tempfile::tempdir().unwrap();
    let layout = Layout::new(d.path().join("models"), d.path().join("llama.cpp"));
    let e = engines(d.path()).runtime(Arc::new(LocalRuntime::new()), layout);
    let app: axum::Router = router(e);
    let (_, v) = call(&app, "GET", "/api/engines", None).await;
    assert_eq!(v["local"]["state"], "missing");
    let (st, v) = call(&app, "PUT", "/api/engines", Some(json!({ "local": { "model": "qwen3.5-9b-q8" } }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("llama.cpp"), "not downloaded: {v}");
    let (st, _) = call(&app, "PUT", "/api/engines", Some(json!({ "local": { "model": "no-such-model" } }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    shared::uninstall();
}

#[test]
fn only_loreforge_and_local_links_open() {
    for ok in [
        "lore-pc://open?path=a.tex",
        "lore-sc://deck/x",
        "lore-lr://paper/abc?page=2",
        "lore-kb://node/12",
        "lore-kb://",
        "http://127.0.0.1",
        "http://127.0.0.1/",
        "http://127.0.0.1:7878/?open=a",
        "http://127.0.0.1:7700/#/node/1",
    ] {
        assert!(link_allowed(ok), "{ok}");
    }
    for bad in [
        "",
        "https://example.com",
        "http://127.0.0.1.evil.example/",
        "http://127.0.0.1@evil.example/",
        "http://127.0.0.1:/x",
        "http://127.0.0.1:123456/",
        "http://127.0.0.1:80x",
        "http://localhost:7878/",
        "file:///etc/passwd",
        "lore-xx://a",
        "LORE-PC://open",
        "javascript:alert(1)",
        "lore-pc://open?path=a b",
        "lore-pc://a\n-b",
    ] {
        assert!(!link_allowed(bad), "{bad}");
    }
}

#[tokio::test]
async fn links_go_to_the_opener_only_when_allowed() {
    let _g = SERIAL.lock().await;
    let d = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut e = engines(d.path());
    let s = seen.clone();
    e.opener = Arc::new(move |u| {
        s.lock().unwrap().push(u.to_string());
        Ok(())
    });
    let app: axum::Router = router(e);
    let (st, _) = call(&app, "POST", "/api/links/open", Some(json!({ "url": "lore-lr://paper/abc?page=3" }))).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, v) = call(&app, "POST", "/api/links/open", Some(json!({ "url": "https://example.com" }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());
    let (st, _) = call(&app, "POST", "/api/links/open", Some(json!({}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(*seen.lock().unwrap(), vec!["lore-lr://paper/abc?page=3".to_string()]);
    shared::uninstall();
}

#[tokio::test]
async fn merges_into_an_app_router_with_its_own_state() {
    let _g = SERIAL.lock().await;
    #[derive(Clone)]
    struct AppState {
        name: &'static str,
    }
    let d = tempfile::tempdir().unwrap();
    let app = axum::Router::new()
        .route("/api/name", get(|axum::extract::State(s): axum::extract::State<Arc<AppState>>| async move { s.name }))
        .merge(router(engines(d.path())))
        .with_state(Arc::new(AppState { name: "app" }));
    let (st, v) = call(&app, "GET", "/api/engines", None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["classes"].is_array());
    shared::uninstall();
}

#[tokio::test]
async fn links_ports_lists_the_live_apps() {
    let _g = SERIAL.lock().await;
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("ports.json");
    let mut e = engines(d.path());
    // isolated: no ports file, nothing live
    let app: axum::Router = router(e.clone());
    let (st, v) = call(&app, "GET", "/api/links/ports", None).await;
    assert_eq!((st, v), (StatusCode::OK, json!({})));
    lore_home::ports::register_at(&f, "sc", 51234, std::process::id()).unwrap();
    lore_home::ports::register_at(&f, "pc", 51235, u32::MAX - 1).unwrap();
    e.ports_file = Some(f);
    let app: axum::Router = router(e);
    let (st, v) = call(&app, "GET", "/api/links/ports", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v, json!({ "sc": 51234 }), "the dead pc entry is ignored");
    shared::uninstall();
}

#[test]
fn every_shared_task_has_a_class() {
    for (name, toml) in [("lore-llm", include_str!("../../lore-llm/tasks.toml")), ("lore-flow", include_str!("../../lore-flow/tasks.toml"))] {
        for (id, t) in TaskTable::parse(toml).unwrap().task {
            assert!(t.class.is_some(), "{name}: {id} has no class");
        }
    }
}
