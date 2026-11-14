//! 待補 over HTTP on a temp workspace with synthetic data: mark a result as
//! 等資料 with a table column, put the table in (pending cells, pending.tex),
//! the 待補清單 with dependents, the submission export refused with every mark
//! listed, then 回填: the result as expected becomes a 修訂 whose plan the code
//! makes (numbers only, no model), applied paragraph by paragraph.
//! Own process (the workspace comes from the environment); models isolated.

use serde_json::{json, Value};

async fn call(http: &reqwest::Client, method: reqwest::Method, url: String, body: Value) -> (u16, Value) {
    let r = http.request(method, url).json(&body).send().await.unwrap();
    let code = r.status().as_u16();
    let text = r.text().await.unwrap();
    (code, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

#[tokio::test]
async fn pending_results_from_record_to_fill_in() {
    let root = std::env::temp_dir().join(format!("pc-pending-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws/profile", "ws/papers/demo/sections", "ws/papers/demo/draft", "home", "private/shared"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let paper = root.join("ws/papers/demo");
    std::fs::write(
        paper.join("main.tex"),
        "\\documentclass{article}\n\\usepackage{amsmath,amssymb}\n\\begin{document}\n\\begin{abstract}\nWeighting holds on a second site\\pendingdep{P1}.\n\\end{abstract}\n\n\\input{sections/results}\n\\input{sections/conclusion}\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(paper.join("sections/results.tex"), "\\section{Results}\nInternal Dice is 0.72.\n\nExternal Dice is \\pending{P1}{external Dice}, which keeps the ranking\\pendingdep{P1}.\n").unwrap();
    std::fs::write(paper.join("sections/conclusion.tex"), "\\section{Conclusion}\nThe gain carries over\\pendingdep{P1}.\n").unwrap();
    // a session from the record step: one results table (written before 待補 existed)
    std::fs::write(
        paper.join("draft/session.json"),
        r#"{"phase":"plan","drafts":[{"section":"Results","paras":[{"para":"p3","text":"External Dice is \\pending{P1}{external Dice}, which keeps the ranking\\pendingdep{P1}."}]}],"tables":[{"id":"R1","caption":"Dice per site","columns":["Method","Internal","External"],"rows":[["Base","0.69",""],["Ours","0.72",""]],"from":["E1"]}]}"#,
    )
    .unwrap();
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", root.join("ws"));
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("HOME", root.join("home"));
    }
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let http = reqwest::Client::new();
    let url = |p: &str| format!("http://{addr}/api/papers/demo{p}");
    use reqwest::Method as M;

    // 等資料: one result, its table column
    let (code, v) = call(&http, M::PUT, url("/draft/pending"), json!({ "items": [
        { "what": "external Dice", "waits_for": "site B labels", "expected": "略低於內部，排名不變", "due": "2026-12", "table": "R1", "cells": [{ "row": null, "col": 2 }] }
    ] }))
    .await;
    assert_eq!(code, 200, "{v}");
    let items = &v["session"]["pending"];
    assert_eq!(items[0]["id"], "P1");
    assert_eq!(items[0]["table_caption"], "Dice per site");
    assert!(v["session"]["groups"].as_array().unwrap().is_empty(), "no plan yet: the 待補項目 group comes with the plan");

    // the table goes in with pending cells; the paper compiles with them (pending.tex)
    let (code, v) = call(&http, M::POST, url("/draft/tables/R1/insert"), json!({ "section": "Results" })).await;
    assert_eq!(code, 200, "{v}");
    let res = std::fs::read_to_string(paper.join("sections/results.tex")).unwrap();
    assert!(res.contains("\\pendingcell{P1}{Base / External}") && res.contains("\\pendingcell{P1}{Ours / External}"), "{res}");
    let main = std::fs::read_to_string(paper.join("main.tex")).unwrap();
    assert!(main.contains("\\input{pending}\n\\begin{document}"), "{main}");
    assert!(std::fs::read_to_string(paper.join("pending.tex")).unwrap().contains("\\DeclareRobustCommand\\pendingcell"));

    // the 待補清單: places and dependents (abstract, results, conclusion)
    let (_, v) = call(&http, M::GET, url("/draft"), Value::Null).await;
    let rep = &v["pending_report"][0];
    assert_eq!(rep["id"], "P1");
    assert_eq!(rep["places"].as_array().unwrap().len(), 3, "the number and two cells (the session's draft is in the paper already, not counted twice): {rep}");
    let deps: Vec<&str> = rep["dependents"].as_array().unwrap().iter().map(|d| d["section"].as_str().unwrap()).collect();
    assert_eq!(deps, ["Abstract", "Results", "Conclusion"]);

    // 投稿版 refused, every mark listed with where it is; nothing compiled
    for body in [json!({ "kind": "latex" }), json!({ "kind": "pdf", "final": true })] {
        let (code, v) = call(&http, M::POST, url("/export"), body).await;
        assert_eq!(code, 200);
        assert_eq!(v["ok"], false);
        let errs = v["errors"].as_array().unwrap();
        assert!(errs[0]["message"].as_str().unwrap().contains("投稿版不能有待補"), "{v}");
        assert_eq!(errs.len(), 1 + 4, "a heading and one line per paragraph: abstract, results ¶1, the table, conclusion");
        assert!(errs.iter().any(|e| e["file"] == "results.tex" && e["para"] == 1 && e["message"] == "待補 P1（數字・依賴句）：external Dice"), "{v}");
        assert!(errs.iter().any(|e| e["message"] == "待補 P1（表格）：Base / External、Ours / External"), "{v}");
    }
    assert!(!paper.join("export").exists(), "a refused export writes nothing");

    // 草稿版 Word: the marks as bracketed text (pandoc never sees pending.tex)
    let has_pandoc = std::process::Command::new("pandoc").arg("--version").output().is_ok_and(|o| o.status.success());
    if has_pandoc {
        let (code, v) = call(&http, M::POST, url("/export"), json!({ "kind": "docx" })).await;
        assert_eq!((code, &v["ok"]), (200, &json!(true)), "{v}");
        let out = std::process::Command::new("pandoc").arg(paper.join("export/demo.docx")).args(["-t", "plain"]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(text.contains("External Dice is [P1: external Dice], which keeps the ranking [P1].") && text.contains("[P1]"), "{text}");
    }

    // a blank number keeps its mark (never another field's value)
    let (code, v) = call(&http, M::POST, url("/draft/pending/P1/fill"), json!({ "result": "", "values": { "Base / External": "0.66", "Ours / External": "0.70" }, "direction": "matches" })).await;
    assert_eq!(code, 200, "{v}");
    let (_, v) = call(&http, M::POST, url("/draft/revisions/v1/plan"), json!({})).await;
    let plan = &v["session"]["revisions"][0]["plan"];
    assert!(plan["effects"].as_array().unwrap().iter().any(|e| e.as_str().unwrap().contains("external Dice") && e.as_str().unwrap().contains("還沒有數值")), "{plan}");
    let results = plan["changes"].as_array().unwrap().iter().find(|c| c["index"] == 1).unwrap();
    assert!(results["text"].as_str().unwrap().contains("\\pending{P1}{external Dice}") && !results["text"].as_str().unwrap().contains("0.66"), "{results}");

    // 回填: as expected → a 修訂 carrying the item, its plan made by the code
    let (code, v) = call(&http, M::POST, url("/draft/pending/P1/fill"), json!({ "result": "0.70", "values": { "Base / External": "0.66", "Ours / External": "0.70" }, "direction": "matches" })).await;
    assert_eq!(code, 200, "{v}");
    let rev = &v["session"]["revisions"][0];
    assert_eq!(rev["pending"], "P1");
    assert!(rev["request"].as_str().unwrap().starts_with("回填 P1：external Dice。結果：0.70（符合預期）"), "{rev}");
    assert_eq!(v["session"]["pending"][0]["status"], "filled");
    assert_eq!(v["session"]["pending"][0]["revision"], "v1");
    let (code, v) = call(&http, M::POST, url("/draft/revisions/v1/plan"), json!({})).await;
    assert_eq!(code, 200, "{v}");
    let plan = &v["session"]["revisions"][0]["plan"];
    let changes = plan["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 4, "abstract, the results paragraph, the table, the conclusion: {plan}");
    assert!(changes.iter().all(|c| c["rewrite"].is_null()), "as expected: nothing to rewrite");
    assert!(plan["summary"].as_str().unwrap().contains("只換數字"));
    for c in changes {
        let (code, v) = call(&http, M::POST, url(&format!("/draft/revisions/v1/changes/{}", c["id"].as_str().unwrap())), json!({ "apply": true })).await;
        assert_eq!(code, 200, "{v}");
    }
    let res = std::fs::read_to_string(paper.join("sections/results.tex")).unwrap();
    assert!(res.contains("External Dice is 0.70, which keeps the ranking.") && res.contains("Ours & 0.72 & 0.70"), "{res}");
    assert!(!res.contains("\\pending") && !std::fs::read_to_string(paper.join("main.tex")).unwrap().contains("\\pendingdep"));
    let (_, v) = call(&http, M::GET, url("/draft"), Value::Null).await;
    assert!(v["pending_report"][0]["places"].as_array().unwrap().is_empty() && v["session"]["revisions"][0]["status"] == "done");

    let _ = std::fs::remove_dir_all(&root);
}
