//! 寫書's routes over HTTP (docs/book.md §API): CORS for LoreReader's page, /api/info, the job
//! object and its refusals. Own process (environment set before the server starts); the job runs on
//! test dependencies (a mock model, no network), never on the server's own engines.

use std::sync::Arc;

use serde_json::{Value, json};

#[tokio::test]
async fn book_routes_answer_lorereader_and_refuse_what_they_should() {
    let root = std::env::temp_dir().join(format!("pc-book-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws/profile", "home", "private", "cache"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", root.join("ws"));
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("PAPERCOMPOSER_BOOKS", root.join("home/books.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("HOME", root.join("home"));
    }
    let deps = pc_server::book::Deps {
        cache: root.join("cache"),
        workspace: Some(root.join("ws")),
        global_file: None,
        reader: None,
        reader_url: "http://127.0.0.1:9".into(),
        kb_spaces: vec![],
        member: "ellrin".into(),
        filters: Default::default(),
        model: Arc::new(pc_server::book::model::MockModel::default()),
        http: Arc::new(lore_research::http::MockHttp::new()),
        research: lore_research::Config::default(),
        web: false,
        extract: Arc::new(pc_server::book::source::DocsrcExtract),
        tools: Arc::new(pc_server::book::tools::MockTools { compile: false, made: Default::default() }),
        plot_styles: json!([]),
        books_file: root.join("home/books.toml"),
    };
    pc_server::book::use_deps(Arc::new(deps));
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let http = reqwest::Client::new();
    let url = |p: &str| format!("http://{addr}{p}");

    // LoreReader's page (another port) may ask: preflight and the answer carry its origin
    let pre = http.request(reqwest::Method::OPTIONS, url("/api/book")).header("Origin", "http://127.0.0.1:7890").header("Access-Control-Request-Method", "POST").header("Access-Control-Request-Headers", "content-type").send().await.unwrap();
    assert_eq!(pre.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()), Some("http://127.0.0.1:7890"));
    let info = http.get(url("/api/info")).header("Origin", "http://localhost:7890").send().await.unwrap();
    assert_eq!(info.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()), Some("http://localhost:7890"));
    let v: Value = info.json().await.unwrap();
    assert_eq!(v["app"], "pc");
    assert!(v["books"].is_array());
    let other = http.get(url("/api/info")).header("Origin", "https://example.org").send().await.unwrap();
    assert!(other.headers().get("access-control-allow-origin").is_none(), "only local pages");
    let books = http.get(url("/api/books")).header("Origin", "http://127.0.0.1:7890").send().await.unwrap();
    assert!(books.headers().get("access-control-allow-origin").is_some());

    // refusals: one line of Chinese with a status
    let bad = |b: Value| http.post(url("/api/book")).json(&b).send();
    let r = bad(json!({ "source": { "kind": "pc", "sha": "x" } })).await.unwrap();
    assert_eq!(r.status().as_u16(), 422);
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    let sha = "a".repeat(64);
    assert_eq!(bad(json!({ "source": { "kind": "lr", "sha": sha }, "length": "huge" })).await.unwrap().status().as_u16(), 400);
    assert_eq!(bad(json!({ "source": { "kind": "lr", "sha": sha }, "book": "nope" })).await.unwrap().status().as_u16(), 404);
    assert_eq!(http.get(url("/api/book/jobs/j123")).send().await.unwrap().status().as_u16(), 404);

    // a job: LoreReader's folder is not here, so the source stage fails — a job object all the same
    let r = bad(json!({ "source": { "kind": "lr", "sha": sha }, "folder": root.join("Books"), "mode": "review" })).await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.json().await.unwrap();
    let id = v["job"].as_str().unwrap().to_string();
    assert_eq!(v["stages"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>(), ["source", "terms", "plan", "research", "review", "write", "check"]);
    let mut state = String::new();
    for _ in 0..100 {
        let v: Value = http.get(url(&format!("/api/book/jobs/{id}?after=0"))).send().await.unwrap().json().await.unwrap();
        state = v["state"].as_str().unwrap_or("").to_string();
        if state == "failed" {
            assert!(v["error"].as_str().unwrap().contains("LoreReader"), "{v}");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    assert_eq!(state, "failed");
    let r = http.post(url(&format!("/api/book/jobs/{id}/review"))).json(&json!({ "skip": true })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 409, "not waiting for the review");
    let r = http.post(url(&format!("/api/book/jobs/{id}/params"))).json(&json!({ "length": "long" })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 409);
    let jobs: Value = http.get(url("/api/book/jobs?unfinished=1")).send().await.unwrap().json().await.unwrap();
    assert_eq!(jobs["jobs"].as_array().unwrap().len(), 1);
    assert_eq!(http.get(url(&format!("/api/book/jobs/{id}/open"))).send().await.unwrap().status().as_u16(), 404);
    assert_eq!(http.delete(url(&format!("/api/book/jobs/{id}"))).send().await.unwrap().status().as_u16(), 204);
    // the page loads the job view
    let index = std::fs::read_to_string(pc_server::Paths::from_repo().ui.join("index.html")).unwrap();
    assert!(index.contains("<script src=\"bookjob.js\"></script>") && index.contains("bookjob.css"));
    let _ = std::fs::remove_dir_all(&root);
}
