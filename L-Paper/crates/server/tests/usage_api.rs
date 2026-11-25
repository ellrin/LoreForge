//! Section drafting remembers the sentences a section has said (`usage.json`) and sends a paragraph back once
//! when it repeats a sentence the author published. A temp workspace, the example profile with one
//! synthetic 說法, scripted replies (`PAPERCOMPOSER_LLM_MOCK`). One test: the environment is per process.

use serde_json::{json, Value};
use std::path::Path;

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

fn paper(ws: &Path, name: &str) {
    let dir = ws.join("papers").join(name);
    std::fs::create_dir_all(dir.join("sections")).unwrap();
    std::fs::create_dir_all(dir.join("draft")).unwrap();
    std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
    std::fs::write(dir.join("sections/results.tex"), "\\section{Results}\n").unwrap();
    std::fs::write(
        dir.join("draft/session.json"),
        r#"{"phase":"draft","groups":[{"id":"g1","section":"Results","title":"主要比較","purpose":"A 對 B","status":"done","turns":1,"summary":{"conclusion":"A 較好"}}],
"outline":[{"section":"Results","paras":[{"id":"p1","kind":"Results key result","zh":"A 對 B","en":["Compared with Method B, Method A improves accuracy by 0.04"],"sources":["g1"]},
{"id":"p2","kind":"Results secondary","zh":"A 對 C","en":["Compared with Method C, Method A improves recall by 0.07"],"sources":["g1"]}]}]}"#,
    )
    .unwrap();
}

fn reply(sentences: [&str; 3]) -> Value {
    json!({ "sentences": [
        { "text": sentences[0], "phrase_id": "P010" },
        { "text": sentences[1], "phrase_id": null },
        { "text": sentences[2], "phrase_id": null }
    ] })
}

/// The first sentence repeats the example of P010 in `private-example` (eight words in a row).
const COPIED: [&str; 3] = [
    "Compared with BM25, our retriever improves recall@10 by 12.4 points on the internal test set.",
    "The difference held in every fold of the cross validation, which suggests that the gain does not depend on a single split of the data.",
    "The same pattern appeared in all five folds and remained stable when the training set was reduced, so the advantage of Method A is consistent across the settings that we examined in this study.",
];
const FIRST: [&str; 3] = ["Method A reached an accuracy of 0.91 while Method B reached 0.87 on the internal test set.", COPIED[1], COPIED[2]];
const SECOND: [&str; 3] = [
    "Method A also recovered more positive cases than Method C, with a recall that was higher by 0.07.",
    "This margin was seen at each of the three operating points that were fixed before the evaluation began.",
    "Because the two methods share one backbone, the margin is attributed to the sampling scheme and not to a larger number of parameters in the network.",
];

async fn draft(http: &reqwest::Client, addr: &std::net::SocketAddr, name: &str) -> Value {
    let r = http.post(format!("http://{addr}/api/papers/{name}/draft/compose")).json(&json!({ "section": "Results" })).send().await.unwrap();
    let code = r.status().as_u16();
    let text = r.text().await.unwrap();
    assert_eq!(code, 200, "{text}");
    serde_json::from_str::<Value>(&text).unwrap()["session"]["drafts"][0]["paras"].clone()
}

fn used(ws: &Path, name: &str, para: &str) -> Vec<String> {
    let u = pc_core::usage::Usage::load(ws);
    let paper = u.papers.iter().find(|(k, _)| k.ends_with(name)).unwrap_or_else(|| panic!("{name} not in {u:?}")).1;
    paper[para].clone()
}

#[tokio::test]
async fn drafting_remembers_wordings_and_rewrites_a_repeated_sentence() {
    let root = std::env::temp_dir().join(format!("pc-usage-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws", "home", "private/shared"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let ws = root.join("ws");
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/PaperComposer/profile"), &ws.join("profile"));
    std::fs::write(ws.join("profile/variants.csv"), "id,pattern,kind,text,slot,note\nV001,P010,variant,\"{method} improves {metric} by {value} over {baseline}.\",,when the method is the subject\n").unwrap();
    paper(&ws, "one");
    // first run: p1 copied, then in his own words, p2 in his own words. second run: copied every time.
    let script = json!({ "pc-compose-paragraph": [reply(COPIED), reply(FIRST), reply(SECOND), reply(COPIED)] });
    std::fs::write(root.join("script.json"), script.to_string()).unwrap();
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

    // a paragraph that repeats a published sentence is sent back once, and the rewrite is kept
    let p = draft(&http, &addr, "one").await;
    assert!(p[0]["text"].as_str().unwrap().starts_with("Method A reached"), "{p}");
    assert!(p[1]["text"].as_str().unwrap().starts_with("Method A also"), "{p}");
    assert!(!p.to_string().contains("重複"), "{p}");
    // the second paragraph of the section does not get the sentence the first one said
    let (first, second) = (used(&ws, "one", "Results/p1"), used(&ws, "one", "Results/p2"));
    assert_eq!((first.len(), second.len()), (1, 1), "one sentence of each named a pattern: {first:?} {second:?}");
    assert!(first[0].starts_with("P010") && second[0].starts_with("P010"), "{first:?} {second:?}");
    assert_ne!(first, second, "the same pattern, another wording");

    // drafted again, and the model repeats the published sentence twice: it is flagged, never hidden
    let p = draft(&http, &addr, "one").await;
    assert!(p[0]["problems"].to_string().contains("和你發表過的句子重複"), "{p}");
    assert_eq!(pc_core::usage::Usage::load(&ws).papers.iter().find(|(k, _)| k.ends_with("one")).unwrap().1.len(), 2, "a redraft replaces its records");

    let _ = std::fs::remove_dir_all(&root);
}
