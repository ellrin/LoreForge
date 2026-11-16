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

/// The scripted replies of every model call in this test.
fn script() -> Value {
    json!({
        "pc-draft-skeleton": [{
            "sections": [
                { "title": "Introduction", "role": "introduction", "blocks": [
                    { "key": "k1", "title": "問題與缺口", "purpose": "讀者接受小資料是常態", "of": "", "paras": [
                        { "key": "x1", "kind": "Intro turn", "say": "小資料是常態", "points": [], "link_prev": "", "link_next": "引出缺口", "floats": [], "cites": [], "sources": [], "pending": [], "answers": [] } ] } ] },
                { "title": "Results", "role": "results", "blocks": [
                    { "key": "k2", "title": "解釋表 1", "purpose": "A 最好", "of": "", "paras": [
                        { "key": "x2", "kind": "Results key result", "say": "A 對 B", "points": ["A 0.91，B 0.87"], "link_prev": "", "link_next": "接 B 對 C", "floats": ["tab:main", "fig:size"], "cites": [], "sources": ["E1", "E99"], "pending": [], "answers": [] },
                        { "key": "x3", "kind": "Results secondary", "say": "B 對 C", "points": [], "link_prev": "承接 A 對 B", "link_next": "", "floats": ["tab:main"], "cites": [], "sources": ["R1"], "pending": [], "answers": [] } ] } ] },
                { "title": "Discussion", "role": "discussion", "blocks": [
                    { "key": "k3", "title": "為什麼 A 在小資料較好", "purpose": "展開表 1", "of": "k2", "paras": [
                        { "key": "x4", "kind": "Discussion interpretation", "say": "先驗較強", "points": [], "link_prev": "承接結果", "link_next": "", "floats": [], "cites": [], "sources": ["E2", "H1"], "pending": [], "answers": ["g1"] } ] } ] }
            ],
            "questions": [
                { "para": "x3", "kind": "missing", "question": "B 和 C 差在哪？", "options": ["資料不同", "標註不同", "還不確定"] },
                { "para": "x4", "kind": "choice", "question": "討論先講資料差異還是方法限制？", "options": ["先資料差異", "先方法限制"] },
                { "para": "x2", "kind": "conflict", "question": "記錄寫 A 最好，外部卻反轉，你要怎麼說？", "options": ["如實寫反轉", "放到限制"] }
            ]
        }],
        "pc-draft-skeleton-repair": [{
            "paras": [
                { "id": "p5", "say": "過渡", "link_prev": "承接 A 對 B", "link_next": "交給 B 對 C", "why": "新增的段落" },
                { "id": "p1", "say": "不該改", "link_prev": "", "link_next": "", "why": "範圍外" }
            ],
            "blocks": [], "jumps": []
        }]
    })
}

/// GET until `ok` holds (a background job lands).
async fn until(http: &reqwest::Client, url: String, ok: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..200 {
        let (_, v) = call(http, reqwest::Method::GET, url.clone(), Value::Null).await;
        if ok(&v) {
            return v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("timed out waiting on {url}");
}

fn para<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["sections"].as_array().unwrap().iter().flat_map(|s| s["blocks"].as_array().unwrap()).flat_map(|b| b["paras"].as_array().unwrap()).find(|p| p["id"] == id).unwrap_or_else(|| panic!("{id}: {v}"))
}

fn order(v: &Value) -> Vec<String> {
    v["sections"].as_array().unwrap().iter().flat_map(|s| s["blocks"].as_array().unwrap()).flat_map(|b| b["paras"].as_array().unwrap()).map(|p| p["id"].as_str().unwrap().to_string()).collect()
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
    std::fs::write(root.join("script.json"), script().to_string()).unwrap();
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", &ws);
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("PAPERCOMPOSER_LLM_MOCK", root.join("script.json"));
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

    // ── 排骨架 ──
    let (code, v) = call(&http, M::POST, url("/plan"), json!({ "redo": false })).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["job"]["kind"], "plan");
    let v = until(&http, url(""), |v| v["job"].is_null()).await;
    assert_eq!(v["exists"], true, "{v}");
    assert_eq!(order(&v), ["p1", "p2", "p3", "p4"]);
    let p2 = para(&v, "p2");
    assert_eq!((p2["floats"].clone(), p2["sources"].clone(), p2["state"].clone()), (json!(["tab:main"]), json!(["E1"]), json!("points")), "a label not kept and an unknown id are dropped");
    assert_eq!(para(&v, "p1")["state"], "outline");
    assert_eq!(v["sections"][2]["blocks"][0]["of"], "b2");
    let threads: Vec<(&str, &str)> = v["threads"].as_array().unwrap().iter().map(|t| (t["id"].as_str().unwrap(), t["anchor"].as_str().unwrap())).collect();
    assert_eq!(threads, [("q1", "p3"), ("q2", "p4"), ("q3", "p2")]);
    assert_eq!(v["next"], "q1");
    let (code, _) = call(&http, M::POST, url("/plan"), json!({})).await;
    assert_eq!(code, 400, "a skeleton exists: only redo replaces it");

    // ── edits ──
    let rev = v["rev"].as_u64().unwrap();
    let (code, v) = call(&http, M::POST, url("/ops"), json!({ "ops": [{ "op": "add_para", "block": "b2", "at": 1, "say": "新段", "reason": "補過渡" }] })).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(order(&v), ["p1", "p2", "p5", "p3", "p4"]);
    assert_eq!((v["dirty"].clone(), v["can_undo"].clone()), (json!(true), json!(true)));
    assert!(v["rev"].as_u64().unwrap() > rev);
    let (code, bad_ops) = call(&http, M::POST, url("/ops"), json!({ "ops": [{ "op": "remove", "id": "p1" }, { "op": "remove", "id": "zz" }] })).await;
    assert_eq!(code, 400, "{bad_ops}");
    let (_, same) = call(&http, M::GET, url(""), Value::Null).await;
    assert_eq!(order(&same), order(&v), "all or none");
    assert_eq!(same["rev"], v["rev"]);

    // ── 暫存 ──
    let before = same.clone();
    let (code, v) = call(&http, M::POST, url("/save"), json!({})).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["job"]["kind"], "save");
    assert_eq!(para(&v, "p5")["busy"], "repair");
    let v = until(&http, url(""), |v| v["job"].is_null()).await;
    assert_eq!(v["dirty"], false);
    assert_eq!(para(&v, "p5")["say"], "過渡");
    assert_eq!(para(&v, "p5")["touched"], false);
    assert_eq!(para(&v, "p1"), para(&before, "p1"), "outside the scope: unchanged");
    assert_eq!(para(&v, "p4"), para(&before, "p4"));
    let edit = v["threads"].as_array().unwrap().iter().find(|t| t["kind"] == "edit").unwrap().clone();
    assert_eq!((edit["anchor"].as_str(), edit["changes"].as_array().unwrap().len()), (Some("p5"), 3));
    let (code, v) = call(&http, M::POST, url("/change"), json!({ "thread": edit["id"], "change": "c1", "accept": false })).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(para(&v, "p5")["say"], "新段", "reverted");

    // ── 改用骨架 ──
    let old = paper(&ws, "old");
    std::fs::write(
        old.join("draft/session.json"),
        r#"{"phase":"draft","groups":[{"id":"g1","section":"Results","title":"主要比較","purpose":"A 對 B","status":"done","turns":1,"summary":{"conclusion":"A 較好"}}],
"outline":[{"section":"Results","paras":[{"id":"p1","kind":"Results key result","zh":"A 對 B","en":["A 0.91"],"sources":["g1"]}]}],
"drafts":[{"section":"Results","paras":[{"para":"p1","text":"Method A was better."}]}]}"#,
    )
    .unwrap();
    std::fs::write(old.join("draft/transcript.jsonl"), "{\"n\":1,\"group\":\"g1\",\"who\":\"ai\",\"text\":\"A 為什麼好？\"}\n{\"n\":2,\"group\":\"g1\",\"who\":\"me\",\"text\":\"先驗較強\"}\n").unwrap();
    let ourl = |p: &str| format!("http://{addr}/api/papers/old/draft/skeleton{p}");
    let (_, v) = call(&http, M::GET, ourl(""), Value::Null).await;
    assert_eq!((v["exists"].clone(), v["can_convert"].clone()), (json!(false), json!(true)));
    let (code, v) = call(&http, M::POST, ourl("/convert"), json!({})).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["exists"], true);
    assert_eq!(para(&v, "p1")["text"], "Method A was better.");
    assert_eq!(v["threads"][0]["conclusion"], "A 較好");
    assert!(std::fs::read_dir(old.join("draft/archive")).unwrap().count() == 1, "a copy first");
    let (code, _) = call(&http, M::POST, ourl("/convert"), json!({})).await;
    assert_eq!(code, 400, "converted already");

    // mid-interview: planned afresh, the answers hang where the planner used them
    let mid = paper(&ws, "mid");
    std::fs::write(mid.join("draft/session.json"), r#"{"phase":"interview","experiments":[{"id":"E1","title":"Method A vs Method B","result":"0.91 vs 0.87"}],"groups":[{"id":"g1","section":"Discussion","title":"為什麼","status":"active","turns":1}]}"#).unwrap();
    std::fs::write(mid.join("draft/transcript.jsonl"), "{\"n\":1,\"group\":\"g1\",\"who\":\"ai\",\"text\":\"為什麼 A 好？\"}\n{\"n\":2,\"group\":\"g1\",\"who\":\"me\",\"text\":\"先驗較強\"}\n").unwrap();
    let murl = |p: &str| format!("http://{addr}/api/papers/mid/draft/skeleton{p}");
    let (code, v) = call(&http, M::POST, murl("/convert"), json!({})).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["job"]["kind"], "convert");
    let v = until(&http, murl(""), |v| v["job"].is_null()).await;
    let answered = v["threads"].as_array().unwrap().iter().find(|t| t["status"] == "done").unwrap();
    assert_eq!(answered["anchor"], "p4", "where the planner used g1's answers");
    assert_eq!(answered["messages"][1]["text"], "先驗較強");

    let _ = std::fs::remove_dir_all(&root);
}
