//! 寫作三階段 over HTTP on a temp workspace with no model connected: the Key value check
//! answers 200 with no advice (it never blocks), guide sentences of `style/patterns.toml` are
//! refused by 收藏句型 (`/api/phrases/add`) while the author's own pattern is added, and the
//! 潤飾 presets are 用字 and 我的風格. Own process (the workspace comes from the environment).

use serde_json::{json, Value};

const GUIDE: &str = "Although trains and buses differ in speed, both reduce traffic in city centres.";

#[tokio::test]
async fn key_value_check_never_blocks_and_guide_sentences_stay_out_of_the_bank() {
    let root = std::env::temp_dir().join(format!("pc-style-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws/profile", "ws/style", "home", "private/shared"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let p = root.join("ws/profile");
    std::fs::write(p.join("profile.toml"), "name = \"t\"\nschema_version = 1\nlanguage = \"en\"\n").unwrap();
    std::fs::write(p.join("phrases.csv"), "id,section,function,position,template,example,tags,priority\nP001,Result,finding,body,\"{method} improved {metric}.\",,paper1,3\n").unwrap();
    std::fs::write(p.join("expressions.csv"), "id,kind,text,usage,section,example,tags\n").unwrap();
    std::fs::write(p.join("vocabulary.csv"), "avoid,prefer,note\n").unwrap();
    std::fs::write(p.join("rules.csv"), "id,type,scope,check,threshold,severity,message\n").unwrap();
    std::fs::write(
        root.join("ws/style/patterns.toml"),
        format!("[[pattern]]\nid = \"g\"\nsituation = \"compare_two\"\nstructure = \"Although {{A}} and {{B}} differ in {{x}}, both {{y}}.\"\nexample = \"{GUIDE}\"\nsource = \"guide\"\n"),
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
    let url = |p: &str| format!("http://{addr}{p}");

    // no engine: still 200, nothing to show, the page goes on
    let r = http.post(url("/api/keyvalue/check")).json(&json!({ "key_value": "Should models use more data?" })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!((v["problems"].clone(), v["rewrites"].clone()), (json!([]), json!([])));
    let r = http.post(url("/api/keyvalue/check")).json(&json!({ "key_value": "  " })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);

    // 收藏句型: the guide's sentence and structure are refused, the author's own is added
    let body = json!({
        "phrases": [
            { "section": "Discussion", "function": "comparison", "position": "opening", "template": "Although {A} and {B} differ in {x}, both {y}.", "example": GUIDE, "tags": [] },
            { "section": "Result", "function": "finding", "position": "closing", "template": "These results indicate that {claim}.", "example": "", "tags": ["paper1"] }
        ],
        "expressions": [{ "kind": "phrase", "text": GUIDE, "usage": "", "section": "", "example": "" }]
    });
    let v: Value = http.post(url("/api/phrases/add")).json(&body).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["guide"], json!(2), "{v}");
    assert_eq!(v["phrases"].as_array().unwrap().len(), 1);
    let bank = std::fs::read_to_string(p.join("phrases.csv")).unwrap() + &std::fs::read_to_string(p.join("expressions.csv")).unwrap();
    assert!(bank.contains("These results indicate that {claim}."));
    assert!(!bank.contains("trains and buses") && !bank.contains("differ in {x}"), "{bank}");

    // the presets, labels only
    let v: Value = http.get(url("/api/polish/presets")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v, json!([{ "id": "diction", "label": "用字" }, { "id": "voice", "label": "我的風格" }]));
    let _ = std::fs::remove_dir_all(&root);
}
