//! A whole 骨架 run over HTTP, the models answered from `tests/fixtures/skeleton_mock.json` (the
//! file the isolated example's `--mock` takes) on the synthetic `skeleton-demo` paper:
//! inventory → plan → three answers → write → refine → style. Temp workspace; never the network,
//! the `claude` CLI or a local model.

use serde_json::{json, Value};
use std::path::Path;

async fn call(http: &reqwest::Client, method: reqwest::Method, url: String, body: Value) -> (u16, Value) {
    let mut r = http.request(method, url);
    if !body.is_null() {
        r = r.json(&body);
    }
    let r = r.send().await.unwrap();
    let code = r.status().as_u16();
    let text = r.text().await.unwrap();
    (code, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

async fn until(http: &reqwest::Client, url: String, ok: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..300 {
        let (_, v) = call(http, reqwest::Method::GET, url.clone(), Value::Null).await;
        if ok(&v) {
            return v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("timed out waiting on {url}");
}

fn paras(v: &Value) -> Vec<Value> {
    v["sections"].as_array().unwrap().iter().flat_map(|s| s["blocks"].as_array().unwrap().clone()).flat_map(|b| b["paras"].as_array().unwrap().clone()).collect()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

#[tokio::test]
async fn skeleton_flow_with_scripted_models() {
    let root = std::env::temp_dir().join(format!("pc-skeleton-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws", "home", "private/shared"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let ws = root.join("ws");
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    copy_dir(&here.join("../../../private-example/PaperComposer/profile"), &ws.join("profile"));
    pc_server::seed_skeleton_demo(&ws).unwrap();
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", &ws);
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("PAPERCOMPOSER_LLM_MOCK", here.join("tests/fixtures/skeleton_mock.json"));
    }
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let http = reqwest::Client::new();
    let url = |p: &str| format!("http://{addr}/api/papers/skeleton-demo/draft/skeleton{p}");
    use reqwest::Method as M;

    // 素材清單: everything but the flow figure
    let (code, v) = call(&http, M::GET, url(""), Value::Null).await;
    assert_eq!(code, 200, "{v}");
    let keep: Vec<Value> = v["inventory"].as_array().unwrap().iter().filter(|i| i["label"] != "fig:overview").map(|i| i["id"].clone()).collect();
    assert!(keep.len() >= 6, "{v}");
    let (code, v) = call(&http, M::POST, url("/inventory"), json!({ "keep": keep, "add": [{ "kind": "equation", "title": "Loss of Method A", "note": "" }] })).await;
    assert_eq!(code, 200, "{v}");

    // 排骨架
    let (code, _) = call(&http, M::POST, url("/plan"), json!({})).await;
    assert_eq!(code, 200);
    let v = until(&http, url(""), |v| v["job"].is_null() && v["exists"] == true).await;
    let ps = paras(&v);
    assert_eq!(ps.len(), 10);
    assert!(ps.iter().all(|p| p["floats"].as_array().unwrap().iter().all(|f| f != "fig:overview")), "a label not kept is dropped");
    assert_eq!(ps.iter().filter(|p| p["state"] == "points").count(), 5, "the record's points seeded");
    assert_eq!(v["threads"].as_array().unwrap().len(), 4);
    assert_eq!(v["next"], "q1");

    // three answers, each folded in before the next
    for (k, (thread, body)) in [("q1", json!({ "thread": "q1", "picked": ["o1"] })), ("q2", json!({ "thread": "q2", "picked": ["o1"], "text": "外部只當驗證" })), ("q5", json!({ "thread": "q5", "picked": ["o1"] }))].into_iter().enumerate() {
        let (code, v) = call(&http, M::POST, url("/answer"), body).await;
        assert_eq!(code, 200, "{k}: {v}");
        let v = until(&http, url(""), |v| v["threads"].as_array().unwrap().iter().any(|t| t["id"] == thread && t["status"] == "done")).await;
        let t = v["threads"].as_array().unwrap().iter().find(|t| t["id"] == thread).unwrap().clone();
        let anchor = t["anchor"].as_str().unwrap();
        let p = paras(&v).into_iter().find(|p| p["id"] == anchor).unwrap();
        assert!(p["points"].as_array().unwrap().iter().any(|x| x["from"] == thread), "{thread}: {p}");
        if thread == "q2" {
            // the follow-up the answer opened comes after the question shown now
            let ids: Vec<&str> = v["threads"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
            assert_eq!(ids, ["q1", "q2", "q3", "q5", "q4"], "{v}");
            assert_eq!(v["next"], "q3");
        }
    }
    let (_, v) = call(&http, M::POST, url("/answer"), json!({ "thread": "q3", "skip": true })).await;
    assert_eq!(v["next"], "q4");

    // 文字
    let (code, v) = call(&http, M::POST, url("/write"), json!({ "id": "all" })).await;
    assert_eq!(code, 200, "{v}");
    let v = until(&http, url(""), |v| v["job"].is_null()).await;
    let ps = paras(&v);
    assert!(ps.iter().all(|p| p["state"] == "text" && p["text"].as_str().unwrap().starts_with("Method A reached")), "{v}");
    let p7 = ps.iter().find(|p| p["id"] == "p7").unwrap();
    assert!(!p7["problems"].as_array().unwrap().iter().any(|x| x.as_str().unwrap().contains("novel")), "the text has no banned word");

    // 細修 and 風格對照
    let (code, _) = call(&http, M::POST, url("/refine"), json!({ "para": "p4", "preset": "wording" })).await;
    assert_eq!(code, 200);
    let v = until(&http, url(""), |v| v["threads"].as_array().unwrap().iter().any(|t| t["kind"] == "refine")).await;
    let r = v["threads"].as_array().unwrap().iter().find(|t| t["kind"] == "refine").unwrap().clone();
    assert_eq!(r["changes"].as_array().unwrap().len(), 1, "{r}");
    let (code, v) = call(&http, M::POST, url("/change"), json!({ "thread": r["id"], "change": "c1", "accept": true })).await;
    assert_eq!(code, 200, "{v}");
    let (code, st) = call(&http, M::GET, url("/style"), Value::Null).await;
    assert_eq!(code, 200, "{st}");
    assert_eq!(st["sections"].as_array().unwrap().len(), 5, "every section has text");

    let _ = std::fs::remove_dir_all(&root);
}
