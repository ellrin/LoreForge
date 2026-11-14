//! 搜尋檔案 → 實驗紀錄 over HTTP on a temp workspace: the page's request (LoreEcho not asked:
//! `PAPERCOMPOSER_NO_ECHO`), LoreEcho's `POST /api/lore/picked` with a chosen file → a record of
//! that paper (a name with spaces, parentheses and Chinese); refusals for an unknown request, a
//! file outside the home folder and a scanned-looking binary; `end` closes the request.
//! Own process (the workspace comes from the environment).

use serde_json::{json, Value};

#[tokio::test]
async fn a_picked_file_becomes_a_record() {
    let root = std::env::temp_dir().join(format!("pc-picked-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws/profile", "ws/papers/demo", "home/Shelf/Run 1", "private/shared"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("ws/papers/demo/main.tex"), "\\documentclass{article}\n\\begin{document}\nx\n\\end{document}\n").unwrap();
    let log = root.join("home/Shelf/Run 1/看板 (Copy) - E1.md");
    std::fs::write(&log, "E1 lr 1e-4, AUC 0.81\n").unwrap();
    std::fs::write(root.join("home/Shelf/Run 1/blob.csv"), [0u8, 159, 146, 150]).unwrap();
    let outside = root.join("outside.md");
    std::fs::write(&outside, "x").unwrap();
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", root.join("ws"));
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("PAPERCOMPOSER_NO_ECHO", "1");
        std::env::set_var("HOME", root.join("home"));
    }
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let http = reqwest::Client::new();
    let url = |p: &str| format!("http://{addr}{p}");
    let picked = |b: Value| http.post(url("/api/lore/picked")).json(&b).send();

    // the page asks; LoreEcho would get {request, app: "pc", purpose, exts, port}
    let v: Value = http.post(url("/api/lore/find")).json(&json!({ "purpose": "實驗紀錄", "exts": ["md", "csv", "pdf"], "context": { "paper": "demo" } })).send().await.unwrap().json().await.unwrap();
    let id = v["request"].as_str().unwrap().to_string();

    // unknown request: nothing read
    assert_eq!(picked(json!({ "request": "nope", "path": log })).await.unwrap().status().as_u16(), 410);
    // the chosen file → a record of paper demo
    let r = picked(json!({ "request": id, "path": log })).await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "{}", r.text().await.unwrap());
    let d: Value = http.get(url("/api/papers/demo/draft")).send().await.unwrap().json().await.unwrap();
    let files: Vec<String> = d["session"]["records"].as_array().or(d["records"].as_array()).expect("records").iter().map(|r| r["file"].as_str().unwrap().to_string()).collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let status: Value = http.get(url(&format!("/api/lore/find/{id}"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(status["added"], json!(["看板 (Copy) - E1.md"]));
    // outside the home folder, or not text: refused with the reason
    let r = picked(json!({ "request": id, "path": outside })).await.unwrap();
    assert_eq!(r.status().as_u16(), 422);
    assert!(r.json::<Value>().await.unwrap()["error"].as_str().unwrap().contains("家目錄"));
    assert_eq!(picked(json!({ "request": id, "path": root.join("home/Shelf/Run 1/blob.csv") })).await.unwrap().status().as_u16(), 422);
    // 好了 in LoreEcho: closed, later picks refused
    assert_eq!(picked(json!({ "request": id, "end": true })).await.unwrap().status().as_u16(), 200);
    assert_eq!(picked(json!({ "request": id, "path": log })).await.unwrap().status().as_u16(), 410);
    let _ = std::fs::remove_dir_all(&root);
}
