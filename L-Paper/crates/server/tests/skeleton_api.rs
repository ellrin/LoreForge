//! 骨架 over HTTP on a temp workspace with synthetic data (Method A vs Method B), the models
//! replaced by scripted replies (`PAPERCOMPOSER_LLM_MOCK`, never the network, the `claude` CLI or a
//! local model). One test: the server's environment is per process.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

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

/// A paper whose record step has run: experiments, a thought, a results table, a figure plan.
fn paper(ws: &Path, name: &str) -> PathBuf {
    let dir = ws.join("papers").join(name);
    std::fs::create_dir_all(dir.join("sections")).unwrap();
    std::fs::create_dir_all(dir.join("draft")).unwrap();
    std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
    std::fs::write(dir.join("sections/results.tex"), "\\section{Results}\n").unwrap();
    std::fs::write(
        dir.join("draft/session.json"),
        r#"{"phase":"plan","experiments":[{"id":"E1","title":"Method A vs Method B","metrics":"accuracy","result":"0.91 vs 0.87"},{"id":"E2","title":"Training size","result":"gap largest below 500 images"}],
"thoughts":[{"id":"H1","text":"A 的先驗較強","about":["E2"]}],
"tables":[{"id":"R1","caption":"Main comparison","columns":["Method","Accuracy"],"rows":[["A","0.91"],["B","0.87"]],"from":["E1"]}],
"figures":{"items":[{"id":"T1","kind":"table","label":"tab:main","title":"主要比較","supports":"A 較好","evidence":["R1","E1"],"tool":"table","status":"kept"},{"id":"F1","kind":"figure","label":"fig:size","title":"資料量的影響","evidence":["E2"],"tool":"plot","status":"proposed"}]}}"#,
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn skeleton_routes_end_to_end() {
    let root = std::env::temp_dir().join(format!("pc-skeleton-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws", "home", "private/shared"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let ws = root.join("ws");
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/PaperComposer/profile"), &ws.join("profile"));
    let dir = paper(&ws, "demo");
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", &ws);
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("HOME", root.join("home"));
    }
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let http = reqwest::Client::new();
    let url = |p: &str| format!("http://{addr}/api/papers/demo/draft/skeleton{p}");
    use reqwest::Method as M;

    // ── 素材清單 ──
    let (code, v) = call(&http, M::GET, url(""), Value::Null).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!((v["exists"].clone(), v["can_convert"].clone(), v["job"].clone(), v["next"].clone(), v["targets"].clone()), (json!(false), json!(false), Value::Null, Value::Null, Value::Null));
    let inv = v["inventory"].as_array().unwrap();
    let pick = |kind: &str, title: &str| inv.iter().find(|i| i["kind"] == kind && i["title"] == title).unwrap_or_else(|| panic!("{kind} {title}: {v}"))["id"].as_str().unwrap().to_string();
    let main = pick("table", "主要比較");
    let size = pick("figure", "資料量的影響");
    let e1 = pick("finding", "Method A vs Method B");
    assert!(inv.iter().all(|i| i["kind"] != "table" || i["title"] != "Main comparison"), "R1 is the planned table already");
    assert!(!dir.join("draft/session.json").exists() || !std::fs::read_to_string(dir.join("draft/session.json")).unwrap().contains("skeleton"), "reading writes nothing");
    let (_, again) = call(&http, M::GET, url(""), Value::Null).await;
    assert_eq!(again, v, "two reads, the same view");

    let (code, v) = call(&http, M::POST, url("/inventory"), json!({ "keep": [main, e1], "add": [{ "kind": "algorithm", "title": "Training loop", "note": "可重現" }] })).await;
    assert_eq!(code, 200, "{v}");
    let inv = v["inventory"].as_array().unwrap();
    assert_eq!(inv.iter().find(|i| i["id"] == size.as_str()).unwrap()["keep"], false);
    let alg = inv.iter().find(|i| i["kind"] == "algorithm").unwrap();
    assert_eq!((alg["label"].as_str(), alg["by_author"].as_bool(), alg["keep"].as_bool()), (Some("alg:training-loop"), Some(true), Some(true)));
    let rev = v["rev"].as_u64().unwrap();
    assert!(rev >= 1);
    let (_, v2) = call(&http, M::GET, url(""), Value::Null).await;
    assert_eq!(v2["rev"].as_u64(), Some(rev), "a read changes nothing");
    let (code, _) = call(&http, M::POST, url("/inventory"), json!({ "keep": [], "add": [{ "kind": "poem", "title": "x" }] })).await;
    assert_eq!(code, 400);
    let (_, v3) = call(&http, M::GET, url(""), Value::Null).await;
    assert_eq!(v3["inventory"], v["inventory"], "a refused choice changes nothing");

    let _ = std::fs::remove_dir_all(&root);
}
