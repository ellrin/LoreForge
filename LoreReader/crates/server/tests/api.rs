//! API tests, offline: a temp library and workspace, synthetic PDFs, a fake
//! KnowledgeBase and a fake OpenAI-compatible local engine (both axum on 127.0.0.1:0).

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use http_body_util::BodyExt;
use lr_core::config::{Config, EngineConfig};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Env {
    _dir: tempfile::TempDir,
    app: axum::Router,
    root: std::path::PathBuf,
}

fn config(root: &std::path::Path, kb_url: &str, engine: Option<String>) -> Config {
    Config {
        library: root.join("library"),
        exports: root.join("exports"),
        kb_url: kb_url.into(),
        claude_model: None,
        engine: EngineConfig { url: engine, model: None },
        app_dir: None,
    }
}

fn setup(kb_url: &str, engine: Option<String>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let shared = lr_server::state(config(&root, kb_url, engine), root.join("ws"), lr_server::repo_dir().join("web"), true);
    Env { app: lr_server::router(shared), _dir: dir, root }
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>, raw: Option<Vec<u8>>) -> (StatusCode, Value, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = if let Some(b) = body {
        req = req.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&b).unwrap())
    } else if let Some(r) = raw {
        Body::from(r)
    } else {
        Body::empty()
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let st = res.status();
    let loc = res.headers().get("location").map(|v| v.to_str().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    let mut v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if let Some(l) = loc {
        v = json!({ "location": l });
    }
    (st, v, bytes)
}

async fn import(app: &axum::Router, bytes: Vec<u8>, name: &str) -> String {
    let (s, v, _) = call(app, "POST", &format!("/api/papers/import?name={name}"), None, Some(bytes)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["paper"]["sha256"].as_str().unwrap().to_string()
}

/// A fake kb-server: spaces, a scan that knows "Dice coefficient", and node creation.
async fn fake_kb() -> (String, Arc<Mutex<Vec<Value>>>) {
    let created: Arc<Mutex<Vec<Value>>> = Arc::default();
    let c2 = created.clone();
    let app = axum::Router::new()
        .route("/api/spaces", get(|| async { axum::Json(json!({ "member": "ellrin", "spaces": [{ "id": "home", "kind": "household", "is_member": true }, { "id": "me", "kind": "personal", "is_member": true }] })) }))
        .route(
            "/api/scan",
            post(|axum::Json(b): axum::Json<Value>| async move {
                let text = b["text"].as_str().unwrap_or("");
                let mut spans = vec![];
                if let Some(p) = text.find("Dice coefficient") {
                    let s = text[..p].encode_utf16().count();
                    spans.push(json!({ "start": s, "end": s + 16, "text": "Dice coefficient", "node_id": "n1", "state": "learning", "title": "Dice coefficient" }));
                }
                assert_eq!(b["record"], false, "LoreReader never records encounters");
                axum::Json(json!({ "spans": spans, "candidates": ["Inception Distance"], "assistant": "mock" }))
            }),
        )
        .route(
            "/api/nodes",
            get(|| async { axum::Json(json!({ "nodes": [
                { "id": "n1", "title": "Dice coefficient", "aliases": ["Sørensen–Dice index"], "state": "learning" },
                { "id": "n3", "title": "Hausdorff distance", "aliases": [], "state": "known" },
                { "id": "n4", "title": "U-Net", "aliases": ["UNet"], "state": null } ] })) })
            .post(move |axum::Json(b): axum::Json<Value>| {
                let c = c2.clone();
                async move {
                    c.lock().unwrap().push(b.clone());
                    (StatusCode::CREATED, axum::Json(json!({ "node": { "id": "n2", "title": b["title"] }, "summary": { "state": b["initial_state"] } })))
                }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), created)
}

/// A fake llama-server (OpenAI-compatible) that answers every request with one text.
async fn fake_engine(reply: &'static str) -> String {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        post(move || async move { axum::Json(json!({ "choices": [{ "index": 0, "message": { "role": "assistant", "content": reply }, "finish_reason": "stop" }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } })) }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}/v1")
}

fn dead_url() -> String {
    // A port nothing listens on.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    drop(l);
    format!("http://{a}")
}

/// The demo paper's text as the page would send it (one pdf.js item per line).
fn demo_text() -> Value {
    serde_json::to_value(lr_core::synth::paper_text(&lr_core::synth::demo_lines())).unwrap()
}

fn text_of_page(t: &Value, page: usize) -> String {
    t["pages"][page - 1]["text"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn library_open_link_annotations_quotes() {
    let e = setup(&dead_url(), None);
    let pdf = lr_core::synth::demo_paper();
    let sha = import(&e.app, pdf.clone(), "demo.pdf").await;
    // Content-addressed in the library, metadata in the workspace.
    assert!(e.root.join("library").join(&sha[..2]).join(format!("{sha}.pdf")).is_file());
    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{sha}"), None, None).await;
    assert_eq!(v["paper"]["title"], "A Synthetic Study of Overlap Metrics");
    assert_eq!(v["paper"]["doi"], "10.5555/lorereader.demo.2026");
    assert_eq!((v["paper"]["pages"].as_u64(), v["paper"]["original_name"].as_str(), v["paper"]["present"].as_bool()), (Some(3), Some("demo.pdf"), Some(true)));
    let (_, _, bytes) = call(&e.app, "GET", &format!("/api/papers/{sha}/pdf"), None, None).await;
    assert_eq!(bytes, pdf);
    // Importing again keeps the first paper.json.
    call(&e.app, "PUT", &format!("/api/papers/{sha}"), Some(json!({ "title": "Mine" })), None).await;
    import(&e.app, pdf.clone(), "again.pdf").await;
    let (_, v, _) = call(&e.app, "GET", "/api/papers", None, None).await;
    assert_eq!(v["papers"][0]["title"], "Mine");
    // Not a PDF.
    let (s, _, _) = call(&e.app, "POST", "/api/papers/import?name=x.pdf", None, Some(b"hello".to_vec())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // /open: in the library → redirect; otherwise a page that says so.
    let (s, v, _) = call(&e.app, "GET", &format!("/open?sha={sha}&page=2"), None, None).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(v["location"], format!("/?paper={sha}&page=2"));
    let (s, _, body) = call(&e.app, "GET", &format!("/open?sha={}&page=4", "cd".repeat(32)), None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(String::from_utf8(body).unwrap().contains("不在這台電腦的書庫裡"));

    // Annotations.
    let a = json!({ "page": 2, "kind": "highlight", "color": "green", "quote": { "exact": "Spectral Boundary Loss", "prefix": "The ", "suffix": " weights" }, "rects": [[0.1, 0.2, 0.4, 0.22]] });
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/annotations"), Some(a), None).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["annotation"]["id"].as_str().unwrap().to_string();
    call(&e.app, "PUT", &format!("/api/papers/{sha}/annotations/{id}"), Some(json!({ "note": "邊界權重" })), None).await;
    let file: Value = serde_json::from_slice(&std::fs::read(e.root.join("ws/papers").join(&sha).join("annotations.json")).unwrap()).unwrap();
    assert_eq!(file["annotations"][0]["note"], "邊界權重");
    assert_eq!(file["annotations"][0]["quote"]["exact"], "Spectral Boundary Loss");

    // Citable sentences, per paper and across papers.
    let (s, _, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/quotes"), Some(json!({ "text": "Overlap metrics hide boundary errors.", "page": 3, "note": "intro" })), None).await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, v, _) = call(&e.app, "GET", "/api/quotes", None, None).await;
    assert_eq!(v["schema"], "lorereader.quotes/1");
    assert_eq!(v["quotes"][0]["page"], 3);
    assert_eq!(v["quotes"][0]["sha256"], sha);
    assert_eq!(v["quotes"][0]["paper_title"], "Mine");
}

#[tokio::test]
async fn page_operations_make_new_pdfs() {
    let e = setup(&dead_url(), None);
    let a = import(&e.app, lr_core::synth::numbered_pdf(4, "A"), "a.pdf").await;
    let b = import(&e.app, lr_core::synth::numbered_pdf(2, "B"), "b.pdf").await;
    let before = std::fs::read(e.root.join("library").join(&a[..2]).join(format!("{a}.pdf"))).unwrap();
    // reorder + duplicate + insert from b, in one compose
    let pages = json!([{ "sha": a, "page": 3 }, { "sha": b, "page": 2 }, { "sha": a, "page": 1 }, { "sha": a, "page": 1 }]);
    let (s, v, _) = call(&e.app, "POST", "/api/compose", Some(json!({ "pages": pages, "op": "edit" })), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let new = v["paper"]["sha256"].as_str().unwrap().to_string();
    assert_eq!(v["paper"]["pages"], 4);
    assert_eq!(v["paper"]["derived_from"]["from"], json!([a, b]));
    let (_, _, out) = call(&e.app, "GET", &format!("/api/papers/{new}/pdf"), None, None).await;
    let labels: Vec<String> = (1..=4).map(|p| lr_core::pdfops::page_text(&out, p).unwrap().split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    assert_eq!(labels, ["A Page 3", "B Page 2", "A Page 1", "A Page 1"]);
    // The export is a free name and the source is untouched.
    let export = std::path::PathBuf::from(v["export"].as_str().unwrap());
    assert!(export.starts_with(e.root.join("exports")) && export.is_file());
    let (_, v2, _) = call(&e.app, "POST", "/api/compose", Some(json!({ "pages": [{ "sha": a, "page": 2 }], "title": "a（編輯）" })), None).await;
    assert_ne!(v2["export"], v["export"]);
    assert_eq!(before, std::fs::read(e.root.join("library").join(&a[..2]).join(format!("{a}.pdf"))).unwrap());
    let (s, _, _) = call(&e.app, "POST", "/api/compose", Some(json!({ "pages": [{ "sha": a, "page": 9 }] })), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn library_inside_git_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("repo/.git")).unwrap();
    let mut c = config(dir.path(), &dead_url(), None);
    c.library = dir.path().join("repo/library");
    let app = lr_server::router(lr_server::state(c, dir.path().join("ws"), lr_server::repo_dir().join("web"), true));
    let (_, v, _) = call(&app, "GET", "/api/info", None, None).await;
    assert!(v["library_error"].as_str().unwrap().contains("git"));
    let (s, _, _) = call(&app, "POST", "/api/papers/import?name=a.pdf", None, Some(lr_core::synth::numbered_pdf(1, "A"))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(!dir.path().join("repo/library").exists());
}

#[tokio::test]
async fn knowledge_base_terms_and_cards() {
    // KB not running: said clearly, colouring still gets offline candidates.
    let e = setup(&dead_url(), None);
    let sha = import(&e.app, lr_core::synth::demo_paper(), "demo.pdf").await;
    let (_, v, _) = call(&e.app, "GET", "/api/kb/status", None, None).await;
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("KnowledgeBase"));
    let (s, v, _) = call(&e.app, "POST", "/api/kb/cards", Some(json!({ "sha": sha, "page": 1, "title": "SBL" })), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(v["kb_down"], true);
    // The noun list still works without the KB: every unit is "no card".
    let text = demo_text();
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/read"), Some(text.clone()), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["nouns"]["kb"]["ok"], false);
    assert!(v["nouns"]["entries"].as_array().unwrap().iter().all(|n| n["state"] == "new"));

    // KB running: known spans win over candidates; the mock's candidates are not used.
    let (url, created) = fake_kb().await;
    let e = setup(&url, None);
    let sha = import(&e.app, lr_core::synth::demo_paper(), "demo.pdf").await;
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/read"), Some(demo_text()), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let nouns = &v["nouns"];
    assert_eq!(nouns["kb"]["ok"], true);
    let entry = |key: &str| nouns["entries"].as_array().unwrap().iter().find(|n| n["key"] == key).cloned().unwrap_or_else(|| panic!("no {key}: {nouns}"));
    let dice = entry("dice coefficient");
    assert_eq!((dice["state"].as_str(), dice["source"].as_str(), dice["kb"]["node_id"].as_str()), (Some("learning"), Some("kb"), Some("n1")));
    assert!(dice["count"].as_u64().unwrap() >= 2, "{dice}");
    assert_eq!(entry("hausdorff distance")["state"], "known");
    assert_eq!(entry("u net")["state"], "none", "a card without learning state");
    let sbl = entry("spectral boundary loss");
    // no card, but not classified yet: never red before the model has judged it
    assert_eq!((sbl["state"].as_str(), sbl["short"].as_str(), sbl["red"].as_bool()), (Some("new"), Some("SBL"), Some(false)));
    assert_eq!(nouns["classification"]["pending"].as_u64(), Some(nouns["stats"]["no_card"].as_u64().unwrap()));
    // Occurrences are [page, start, end] in UTF-16 of the page text that was sent.
    let o = &dice["occ"][0];
    let page_text = text_of_page(&demo_text(), o[0].as_u64().unwrap() as usize);
    let u: Vec<u16> = page_text.encode_utf16().collect();
    assert!(String::from_utf16(&u[o[1].as_u64().unwrap() as usize..o[2].as_u64().unwrap() as usize]).unwrap().starts_with("Dice coefficient"));

    // 「這不是術語」 writes to the ignore list and the list changes; undo removes it.
    assert!(nouns["entries"].as_array().unwrap().iter().any(|n| n["key"] == "anisotropic"));
    let (s, _, _) = call(&e.app, "POST", "/api/ignore", Some(json!({ "term": "anisotropic" })), None).await;
    assert_eq!(s, StatusCode::OK);
    let file = std::fs::read_to_string(e.root.join("ws/ignore-words.txt")).unwrap();
    assert!(file.lines().any(|l| l == "anisotropic"));
    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{sha}/nouns"), None, None).await;
    assert!(!v["entries"].as_array().unwrap().iter().any(|n| n["key"] == "anisotropic"));
    call(&e.app, "DELETE", "/api/ignore", Some(json!({ "term": "Anisotropic" })), None).await;
    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{sha}/nouns"), None, None).await;
    assert!(v["entries"].as_array().unwrap().iter().any(|n| n["key"] == "anisotropic"));
    // No text sent for a paper: a clear 409.
    let (s, _, _) = call(&e.app, "GET", &format!("/api/papers/{}/nouns", "ab".repeat(32)), None, None).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Add to KB: explicit, initial state chosen, source points at the PDF page.
    let (s, v, _) = call(&e.app, "POST", "/api/kb/cards", Some(json!({ "sha": sha, "page": 2, "title": "Spectral Boundary Loss", "summary": "依曲率加權邊界像素的損失", "initial_state": "known", "context": "The Spectral Boundary Loss weights pixels near edges." })), None).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (_, _, _) = call(&e.app, "POST", "/api/kb/cards", Some(json!({ "sha": sha, "page": 1, "title": "SBL" })), None).await;
    let c = created.lock().unwrap().clone();
    assert_eq!(c.len(), 2);
    assert_eq!(c[0]["space"], "me", "the personal space");
    assert_eq!(c[0]["initial_state"], "known");
    assert_eq!(c[1]["initial_state"], "learning", "default");
    assert_eq!(c[0]["via"], "lorereader");
    assert_eq!(c[0]["sources"], json!([{ "type": "pdf", "ref": sha, "where": "p.2", "title": "A Synthetic Study of Overlap Metrics" }]));
    assert!(c[0]["body"].as_str().unwrap().starts_with("> The Spectral Boundary Loss"));
    let (s, _, _) = call(&e.app, "POST", "/api/kb/cards", Some(json!({ "sha": sha, "page": 1, "title": "x", "initial_state": "stale" })), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn translate_and_chat_through_lore_llm() {
    // No engine at all: a clear error.
    let e = setup(&dead_url(), None);
    let (s, v, _) = call(&e.app, "POST", "/api/translate", Some(json!({ "text": "anisotropic" })), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(v["llm_unavailable"], true);

    // A local engine: lr-translate runs there first.
    let e = setup(&dead_url(), Some(fake_engine("各向異性（anisotropic）").await));
    let (s, v, _) = call(&e.app, "POST", "/api/translate", Some(json!({ "text": "anisotropic", "context": "anisotropic diffusion" })), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["translation"].as_str().unwrap().contains("各向異性"));
    assert_eq!(v["fallback_used"], false);

    // lr-chat is Claude only: with no Claude it fails and stores nothing.
    let sha = import(&e.app, lr_core::synth::demo_paper(), "demo.pdf").await;
    let (s, _, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/chat"), Some(json!({ "text": "這是什麼？", "attachments": [{ "text": "SBL", "page": 2, "context": "The SBL term matters." }] })), None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{sha}/chat"), None, None).await;
    assert_eq!(v["messages"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn classification_is_one_cached_call_and_the_model_menu_picks_the_engine() {
    // A fake local engine that answers every request with a classification.
    let reply = r#"{"方法": ["anisotropic", "diffusion", "Spectral Boundary Loss"], "指標": [], "資料集": [], "統計": [], "臨床／解剖": [], "一般學術詞": ["codebook"], "人名": ["Hausdorff", "Sobel"], "其他術語": []}"#;
    let (url, hits) = counting_engine(reply).await;
    let e = setup(&dead_url(), Some(url));
    let sha = import(&e.app, lr_core::synth::demo_paper(), "demo.pdf").await;
    call(&e.app, "POST", &format!("/api/papers/{sha}/read"), Some(demo_text()), None).await;
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/nouns/classify"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(*hits.lock().unwrap(), 1, "one call for the whole paper");
    let entries = v["nouns"]["entries"].as_array().unwrap();
    let get = |k: &str| entries.iter().find(|n| n["key"] == k).cloned().unwrap();
    assert_eq!(get("hausdorff")["category"], "人名");
    assert_eq!(get("hausdorff")["red"], false, "names are listed, not red");
    assert_eq!(get("anisotropic")["category"], "方法");
    assert_eq!(get("anisotropic")["red"], true);
    assert_eq!(v["nouns"]["classification"]["pending"], 0);
    // Cached per paper: asking again sends nothing.
    let cache: Value = serde_json::from_slice(&std::fs::read(e.root.join("ws/papers").join(&sha).join("nouns.json")).unwrap()).unwrap();
    assert_eq!(cache["schema"], "lorereader.nouns/1");
    assert_eq!(cache["categories"]["hausdorff"], "人名");
    let (_, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/nouns/classify"), None, None).await;
    assert_eq!((v["asked"].as_u64(), *hits.lock().unwrap()), (Some(0), 1));

    // The model menu: Haiku for classification is that engine or an error (no Claude
    // here, and no fallback to the local engine).
    let (s, v, _) = call(&e.app, "PUT", "/api/models", Some(json!({ "task": "lr-classify-terms", "model": "haiku" })), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tasks"][0]["choice"], "haiku");
    assert_eq!(v["tasks"].as_array().unwrap().len(), 4);
    let (s, _, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/nouns/classify"), Some(json!({ "again": true })), None).await;
    assert!(s == StatusCode::OK || s == StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(*hits.lock().unwrap(), 1, "the local engine was not used");
    let (s, _, _) = call(&e.app, "PUT", "/api/models", Some(json!({ "task": "lr-classify-terms", "model": "gpt" })), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Without any model the list still works: unclassified.
    let e = setup(&dead_url(), None);
    let sha = import(&e.app, lr_core::synth::demo_paper(), "demo.pdf").await;
    let (_, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/read"), Some(demo_text()), None).await;
    assert!(v["nouns"]["entries"].as_array().unwrap().iter().all(|n| n["category"].is_null()));
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/nouns/classify"), None, None).await;
    assert_eq!((s, v["llm_unavailable"].as_bool()), (StatusCode::SERVICE_UNAVAILABLE, Some(true)));
}

/// A fake OpenAI-compatible engine that counts the classification requests.
async fn counting_engine(reply: &'static str) -> (String, Arc<Mutex<usize>>) {
    let hits: Arc<Mutex<usize>> = Arc::default();
    let h2 = hits.clone();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        post(move |axum::Json(b): axum::Json<Value>| {
            let h = h2.clone();
            async move {
                // count the classification calls only (the paragraph run also asks)
                if b["messages"][0]["content"].as_str().unwrap_or("").contains("術語分類器") {
                    *h.lock().unwrap() += 1;
                }
                axum::Json(json!({ "choices": [{ "index": 0, "message": { "role": "assistant", "content": reply }, "finish_reason": "stop" }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } }))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}/v1"), hits)
}

#[tokio::test]
async fn symbol_table_detected_edited_and_kept() {
    let e = setup(&dead_url(), None);
    let sha = import(&e.app, lr_core::synth::demo_paper(), "demo.pdf").await;
    // The pdf.js items of a synthetic LaTeX paper (lr-core's test fixture).
    let items: Value = serde_json::from_slice(&std::fs::read(lr_server::repo_dir().join("crates/core/tests/math/paper1.items.json")).unwrap()).unwrap();
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/read"), Some(items.clone()), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let syms = &v["symbols"];
    assert_eq!(syms["schema"], "lorereader.symbols/1");
    let find = |v: &Value, latex: &str| v["entries"].as_array().unwrap().iter().find(|x| x["latex"] == latex).cloned().unwrap_or_else(|| panic!("{latex}"));
    let w = find(syms, "w");
    assert_eq!((w["grade"].as_str(), w["resolved_by"].as_str(), w["kind"].as_str()), (Some("defined"), Some("program"), Some("symbol")));
    assert!(w["evidence"][0]["quote"].as_str().unwrap().contains("weight of pixel"));
    assert_eq!(w["scope"], sha);
    assert!(syms["equations"].as_array().unwrap().iter().any(|q| q["label"] == "(1)"));
    let file = e.root.join("ws/papers").join(&sha).join("symbols.json");
    assert!(file.is_file());

    // The author edits an entry: it wins over a re-run.
    let f = find(syms, "\\eta");
    let id = f["id"].as_str().unwrap();
    let (s, v, _) = call(&e.app, "PUT", &format!("/api/papers/{sha}/symbols/{id}"), Some(json!({ "meaning": "分割網路", "grade": "certain" })), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["entry"]["resolved_by"], "author");
    let (s, _, _) = call(&e.app, "PUT", &format!("/api/papers/{sha}/symbols/{id}"), Some(json!({ "grade": "great" })), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    call(&e.app, "POST", &format!("/api/papers/{sha}/read"), Some(items), None).await;
    let (_, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/symbols/rebuild"), None, None).await;
    let f = find(&v, "\\eta");
    assert_eq!((f["meaning"].as_str(), f["grade"].as_str(), f["resolved_by"].as_str()), (Some("分割網路"), Some("certain"), Some("author")));
    assert!(!f["occurrences"].as_array().unwrap().is_empty());
    // Claude's answer written back: grade up, resolved_by claude, the evidence kept.
    let k = find(&v, "\\kappa");
    let (_, v, _) = call(&e.app, "PUT", &format!("/api/papers/{sha}/symbols/{}", k["id"].as_str().unwrap()), Some(json!({ "meaning": "輪廓的曲率", "grade": "inferred", "by": "claude", "evidence": "κ_i 是第 i 個像素所在輪廓的曲率（Claude 的回答）", "page": 1 })), None).await;
    assert_eq!((v["entry"]["resolved_by"].as_str(), v["entry"]["evidence"][0]["by"].as_str()), (Some("claude"), Some("claude")));
    // The author adds a symbol.
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/symbols"), Some(json!({ "latex": "\\bm{v}", "meaning": "速度" })), None).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["entry"]["latex"], "\\boldsymbol{v}");

    // The glyph override: invalid text is refused, valid text is saved.
    let (s, _, _) = call(&e.app, "PUT", "/api/glyphs", Some(json!({ "text": "[glyphs\n" })), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(&e.app, "PUT", "/api/glyphs", Some(json!({ "text": "[glyphs]\n\"ϰ\" = \"\\\\kappa\"\n" })), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(e.root.join("ws/glyphs.toml").is_file());
    let (_, v, _) = call(&e.app, "GET", "/api/glyphs", None, None).await;
    assert!(v["text"].as_str().unwrap().contains("kappa"));
}

/// A fake local engine that answers by task (classification, paragraph analysis, chat)
/// and keeps every request it got.
async fn task_engine() -> (String, Arc<Mutex<Vec<Value>>>) {
    let got: Arc<Mutex<Vec<Value>>> = Arc::default();
    let g2 = got.clone();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        post(move |axum::Json(b): axum::Json<Value>| {
            let g = g2.clone();
            async move {
                g.lock().unwrap().push(b.clone());
                let system = b["messages"][0]["content"].as_str().unwrap_or("").to_string();
                let reply = if system.contains("術語分類器") {
                    json!({ "方法": ["anisotropic", "Spectral Boundary Loss"], "人名": ["Hausdorff"], "基礎科學詞": ["pixels", "curvature"] }).to_string()
                } else if system.contains("段落") && system.contains("role") {
                    json!({ "role": "method", "equations": [{ "ref": "SBL", "inputs": "像素與邊緣距離", "outputs": "權重", "target": "邊界上的點", "effect": "邊界錯誤罰得更重" }], "motive": "細結構容易被忽略", "results": [] }).to_string()
                } else {
                    "Spectral Boundary Loss 依曲率加權邊界像素（測試回覆）".to_string()
                };
                axum::Json(json!({ "choices": [{ "index": 0, "message": { "role": "assistant", "content": reply }, "finish_reason": "stop" }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } }))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}/v1"), got)
}

#[tokio::test]
async fn paragraphs_run_in_the_background_and_ask_claude_gets_the_context() {
    let (kb, _) = fake_kb().await;
    let (engine, got) = task_engine().await;
    let e = setup(&kb, Some(engine));
    let sha = import(&e.app, lr_core::synth::demo_paper(), "demo.pdf").await;
    let (s, _, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/read"), Some(demo_text()), None).await;
    assert_eq!(s, StatusCode::OK);
    // classification decides which terms are chips (red): the paragraphs follow it
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/nouns/classify"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let pixel = v["nouns"]["entries"].as_array().unwrap().iter().find(|n| n["key"] == "pixel").cloned().unwrap();
    assert_eq!((pixel["category"].as_str(), pixel["red"].as_bool()), (Some("基礎科學詞"), Some(false)));
    // the run streams in; wait until it is done
    let mut v = Value::Null;
    for _ in 0..100 {
        v = call(&e.app, "GET", &format!("/api/papers/{sha}/paragraphs"), None, None).await.1;
        if v["status"]["running"] == false {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(v["status"]["running"], false, "{}", v["status"]);
    assert!(v["status"]["error"].is_null(), "{}", v["status"]);
    let paras = v["paragraphs"].as_array().unwrap().clone();
    assert!(paras.len() >= 4, "{v}");
    let method = paras.iter().find(|p| p["section_kind"] == "method").unwrap_or_else(|| panic!("{v}"));
    assert_eq!(method["analysis"]["role"], "method");
    assert_eq!(method["analysis"]["equations"][0]["target"], "邊界上的點");
    assert!(method["chips"]["terms"].as_array().unwrap().iter().any(|t| t == "spectral boundary loss"), "{}", method["chips"]);
    // methods and results first; no Chinese summary asked for
    let reqs = got.lock().unwrap().clone();
    let para_reqs: Vec<&Value> = reqs.iter().filter(|r| r["messages"][0]["content"].as_str().unwrap().contains("role")).collect();
    assert!(para_reqs[0]["messages"][1]["content"].as_str().unwrap().contains("章節：2. Method"), "{}", para_reqs[0]["messages"][1]);
    assert!(para_reqs[0]["messages"][0]["content"].as_str().unwrap().contains("不要翻譯"));
    // cached: a new run asks nothing more
    let before = got.lock().unwrap().len();
    call(&e.app, "POST", &format!("/api/papers/{sha}/paragraphs/run"), None, None).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(got.lock().unwrap().iter().skip(before).filter(|r| r["messages"][0]["content"].as_str().unwrap().contains("role")).count(), 0);
    assert!(e.root.join("ws/papers").join(&sha).join("paragraphs.json").is_file());

    // 懂了: per paper, shown as progress, separate from KB state
    let pid = method["id"].as_str().unwrap();
    let (s, _, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/paragraphs/{pid}/understood"), Some(json!({ "on": true })), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{sha}/paragraphs"), None, None).await;
    assert_eq!(v["progress"]["understood"], 1);
    assert!(v["progress"]["total"].as_u64().unwrap() >= 4);
    let reading: Value = serde_json::from_slice(&std::fs::read(e.root.join("ws/papers").join(&sha).join("reading.json")).unwrap()).unwrap();
    assert_eq!(reading["understood"][0], pid);

    // Ask Claude about a chip (here routed to the local engine with the model menu):
    // title, section, paragraph, the chip and its definition, the author's KB states.
    call(&e.app, "PUT", "/api/models", Some(json!({ "task": "lr-chat", "model": "local" })), None).await;
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/chat"), Some(json!({ "text": "", "paragraph": pid, "chip": { "kind": "term", "id": "spectral boundary loss" } })), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["reply"]["paragraph"], pid);
    let sent = got.lock().unwrap().last().unwrap().clone();
    let user = sent["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_string();
    for part in ["〔論文〕A Synthetic Study of Overlap Metrics", "〔章節〕2. Method", "〔段落，p.2〕", "〔卡住的地方〕Spectral Boundary Loss", "〔作者的知識庫〕", "U-Net", "〔問題〕只解釋我卡住的點，不要翻譯整段"] {
        assert!(user.contains(part), "{part} not in {user}");
    }
    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{sha}/chat"), None, None).await;
    let m = &v["messages"][0];
    assert_eq!((m["paragraph"].as_str(), m["chip"]["kind"].as_str(), m["text"].as_str()), (Some(pid), Some("term"), Some("只解釋我卡住的點，不要翻譯整段")));
    let (s, _, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/chat"), Some(json!({ "text": "?", "paragraph": "pdeadbeef" })), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // No model at all: the run stops with the reason, the paragraphs are still listed.
    let e = setup(&dead_url(), None);
    let sha = import(&e.app, lr_core::synth::demo_paper(), "demo.pdf").await;
    call(&e.app, "POST", &format!("/api/papers/{sha}/read"), Some(demo_text()), None).await;
    let mut v = Value::Null;
    for _ in 0..100 {
        v = call(&e.app, "GET", &format!("/api/papers/{sha}/paragraphs"), None, None).await.1;
        if v["status"]["running"] == false {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(v["status"]["error"].as_str().unwrap().contains("沒有可用的模型"), "{}", v["status"]);
    assert!(!v["paragraphs"].as_array().unwrap().is_empty());
}

/// A fake engine that classifies every asked term as 方法 (so everything is red unless
/// it has a card or is in my-vocab).
async fn all_method_engine() -> String {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        post(|axum::Json(b): axum::Json<Value>| async move {
            let user = b["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap_or("").to_string();
            let terms: Vec<&str> = user.split_once("個）：\n").map(|x| x.1.lines().collect()).unwrap_or_default();
            let reply = if b["messages"][0]["content"].as_str().unwrap_or("").contains("術語分類器") { json!({ "方法": terms }).to_string() } else { "{}".to_string() };
            axum::Json(json!({ "choices": [{ "index": 0, "message": { "role": "assistant", "content": reply }, "finish_reason": "stop" }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } }))
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}/v1")
}

fn lines_text(pages: &[&[&str]]) -> Value {
    let p: Vec<Vec<String>> = pages.iter().map(|l| l.iter().map(|s| s.to_string()).collect()).collect();
    serde_json::to_value(lr_core::synth::paper_text(&p)).unwrap()
}

#[tokio::test]
async fn my_papers_vocabulary_is_never_red_and_kb_wins() {
    let (kb, _) = fake_kb().await;
    let e = setup(&kb, Some(all_method_engine().await));
    // two synthetic "own" papers and a third one to read
    let own1 = lines_text(&[&["# 1. Method", "We measure vessel tortuosity after skeletonization of the mask.", "The Dice coefficient is learning material here.", "# References", "[1] Quaternion Wavelets."]]);
    let own2 = lines_text(&[&["# 2. Results", "Vessel tortuosity and centerline extraction work.", "Centerline extraction again with skeletonization."]]);
    let third = lines_text(&[&["# 1. Introduction", "Vessel tortuosity, skeletonization and centerline extraction meet kriging and homotopy.", "We also report the Dice coefficient."]]);
    let a = import(&e.app, lr_core::synth::numbered_pdf(1, "Own1"), "own1.pdf").await;
    let b = import(&e.app, lr_core::synth::numbered_pdf(1, "Own2"), "own2.pdf").await;
    let c = import(&e.app, lr_core::synth::numbered_pdf(1, "Third"), "third.pdf").await;

    let red_of = |v: &Value| -> Vec<String> { v["entries"].as_array().unwrap().iter().filter(|n| n["red"] == true).map(|n| n["key"].as_str().unwrap().to_string()).collect() };
    call(&e.app, "POST", &format!("/api/papers/{c}/read"), Some(third.clone()), None).await;
    let (_, v, _) = call(&e.app, "POST", &format!("/api/papers/{c}/nouns/classify"), None, None).await;
    let before = red_of(&v["nouns"]);
    for k in ["tortuosity", "skeletonization", "centerline", "kriging", "homotopy"] {
        assert!(before.contains(&k.to_string()), "{k} red before: {before:?}");
    }

    // flag the two own papers (their text comes with the request, as from the library)
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{a}/mine"), Some(json!({ "on": true, "pages": own1["pages"] })), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    call(&e.app, "POST", &format!("/api/papers/{b}/mine"), Some(json!({ "on": true, "pages": own2["pages"] })), None).await;
    let meta: Value = serde_json::from_slice(&std::fs::read(e.root.join("ws/papers").join(&a).join("paper.json")).unwrap()).unwrap();
    assert_eq!(meta["mine"], true);
    let vocab: Value = serde_json::from_slice(&std::fs::read(e.root.join("ws/shared/my-vocab.json")).unwrap()).unwrap();
    assert_eq!(vocab["sources"].as_array().unwrap().len(), 2);
    assert_eq!(vocab["schema"], "lore.my-vocab/1");
    let entry = |k: &str| vocab["entries"].as_array().unwrap().iter().find(|x| x["key"] == k).cloned();
    assert_eq!(entry("tortuosity").unwrap()["sources"].as_array().unwrap().len(), 2, "both papers are its sources");
    assert!(entry("centerline extraction").is_some(), "a recurring 2-gram");
    assert!(entry("quaternion").is_none(), "references are not read");

    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{c}/nouns"), None, None).await;
    let after = red_of(&v);
    for k in ["tortuosity", "skeletonization", "centerline"] {
        assert!(!after.contains(&k.to_string()), "{k} is in my papers: {after:?}");
        let n = v["entries"].as_array().unwrap().iter().find(|n| n["key"] == k).unwrap();
        assert_eq!((n["used"].as_bool(), n["state"].as_str()), (Some(true), Some("new")));
    }
    assert!(after.contains(&"kriging".to_string()) && after.len() < before.len());
    // they count as 記得 (the stats), and Ask Claude lists them as known
    assert!(v["stats"]["known"].as_u64().unwrap() >= 3, "{}", v["stats"]);
    assert_eq!(v["used"].as_u64().unwrap() as usize, before.len() - after.len());
    // a KB card wins over my-vocab: Dice coefficient (learning) stays learning
    let dice = v["entries"].as_array().unwrap().iter().find(|n| n["key"] == "dice coefficient").unwrap();
    assert_eq!((dice["state"].as_str(), dice["used"].as_bool()), (Some("learning"), Some(false)));

    // unflag one: rebuilt from the other; unflag both: empty
    call(&e.app, "POST", &format!("/api/papers/{a}/mine"), Some(json!({ "on": false })), None).await;
    let vocab: Value = serde_json::from_slice(&std::fs::read(e.root.join("ws/shared/my-vocab.json")).unwrap()).unwrap();
    assert_eq!(vocab["sources"], json!([b]));
    assert!(vocab["entries"].as_array().unwrap().iter().all(|x| x["sources"] == json!([b])));
    assert!(!vocab["entries"].as_array().unwrap().iter().any(|x| x["key"] == "mask"), "only in the unflagged paper");
    call(&e.app, "POST", &format!("/api/papers/{b}/mine"), Some(json!({ "on": false })), None).await;
    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{c}/nouns"), None, None).await;
    assert_eq!(red_of(&v).len(), before.len());
    let meta: Value = serde_json::from_slice(&std::fs::read(e.root.join("ws/papers").join(&a).join("paper.json")).unwrap()).unwrap();
    assert!(meta.get("mine").is_none());
    // my-vocab is not the ignore list
    assert!(!e.root.join("ws/ignore-words.txt").exists() || !std::fs::read_to_string(e.root.join("ws/ignore-words.txt")).unwrap().contains("tortuosity"));
    // the classification prompt carries the reader's field and level anchors
    let (_, v, _) = call(&e.app, "PUT", "/api/reader", Some(json!({ "field": "radiologist, deep learning for CT" })), None).await;
    assert_eq!(v["field"], "radiologist, deep learning for CT");
    assert!(std::fs::read_to_string(e.root.join("ws/shared/profile.toml")).unwrap().contains("field = \"radiologist, deep learning for CT\""));
    // categories made for another field (or prompt version) are not used
    let (_, v, _) = call(&e.app, "GET", &format!("/api/papers/{c}/nouns"), None, None).await;
    assert!(v["entries"].as_array().unwrap().iter().all(|n| n["category"].is_null()), "the cache was made for the old field");
    assert!(red_of(&v).is_empty());
    let (_, v, _) = call(&e.app, "POST", &format!("/api/papers/{c}/nouns/classify"), None, None).await;
    assert_eq!(red_of(&v["nouns"]).len(), before.len());
}

#[tokio::test]
async fn my_vocab_is_shared_keeps_imports_and_migrates_old_files() {
    let e = setup(&dead_url(), None);
    let ws = e.root.join("ws");
    std::fs::create_dir_all(ws.join("shared")).unwrap();
    // an imported list in the shared file, an older per-app file, an older field
    std::fs::write(ws.join("shared/my-vocab.json"), json!({ "schema": "lore.my-vocab/1", "sources": ["import:thesis"], "entries": [{ "key": "tortuosity", "text": "tortuosity", "sources": ["import:thesis"] }] }).to_string()).unwrap();
    let old_sha = "ab".repeat(32);
    std::fs::write(ws.join("my-vocab.json"), json!({ "schema": "lorereader.my-vocab/1", "papers": [old_sha], "entries": [{ "key": "centerline", "text": "centerline", "sources": [old_sha] }] }).to_string()).unwrap();
    std::fs::write(ws.join("reading.toml"), "field = \"ophthalmology and deep learning\"\n").unwrap();
    std::fs::write(ws.join("shared/profile.toml"), "# who\nid = \"ellrin\"\n").unwrap();

    let (_, v, _) = call(&e.app, "GET", "/api/reader", None, None).await;
    assert_eq!(v["field"], "ophthalmology and deep learning", "moved from reading.toml");
    let profile = std::fs::read_to_string(ws.join("shared/profile.toml")).unwrap();
    assert!(profile.contains("# who") && profile.contains("id = \"ellrin\"") && profile.contains("field = \"ophthalmology and deep learning\""), "{profile}");
    assert!(!ws.join("reading.toml").exists());

    let sha = import(&e.app, lr_core::synth::numbered_pdf(1, "Own"), "own.pdf").await;
    let pages = lines_text(&[&["# 1. Method", "Skeletonization of vessels and skeletonization again."]]);
    let (s, v, _) = call(&e.app, "POST", &format!("/api/papers/{sha}/mine"), Some(json!({ "on": true, "pages": pages["pages"] })), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let vocab: Value = serde_json::from_slice(&std::fs::read(ws.join("shared/my-vocab.json")).unwrap()).unwrap();
    let keys: Vec<&str> = vocab["entries"].as_array().unwrap().iter().map(|x| x["key"].as_str().unwrap()).collect();
    assert!(keys.contains(&"tortuosity") && keys.contains(&"centerline") && keys.contains(&"skeletonization"), "{keys:?}");
    assert!(!ws.join("my-vocab.json").exists(), "the old per-app file was merged once");
    // unflagging replaces only this paper's source
    call(&e.app, "POST", &format!("/api/papers/{sha}/mine"), Some(json!({ "on": false })), None).await;
    let vocab: Value = serde_json::from_slice(&std::fs::read(ws.join("shared/my-vocab.json")).unwrap()).unwrap();
    let srcs: Vec<&str> = vocab["sources"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(srcs, [old_sha.as_str(), "import:thesis"]);
}
