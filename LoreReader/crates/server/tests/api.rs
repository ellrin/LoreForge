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

/// The demo paper's text as the page would send it (one string per page; lopdf's
/// text extraction stands in for pdf.js here).
fn demo_text() -> Value {
    let pdf = lr_core::synth::demo_paper();
    let pages: Vec<Value> = (1..=3).map(|p| json!({ "page": p, "text": lr_core::pdfops::page_text(&pdf, p).unwrap() })).collect();
    json!({ "pages": pages })
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
    assert_eq!((sbl["state"].as_str(), sbl["short"].as_str(), sbl["red"].as_bool()), (Some("new"), Some("SBL"), Some(true)));
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

/// A fake OpenAI-compatible engine that counts its requests.
async fn counting_engine(reply: &'static str) -> (String, Arc<Mutex<usize>>) {
    let hits: Arc<Mutex<usize>> = Arc::default();
    let h2 = hits.clone();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let h = h2.clone();
            async move {
                *h.lock().unwrap() += 1;
                axum::Json(json!({ "choices": [{ "index": 0, "message": { "role": "assistant", "content": reply }, "finish_reason": "stop" }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } }))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}/v1"), hits)
}
