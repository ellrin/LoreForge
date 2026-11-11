//! 「不熟」: terms of a synthetic paper against a synthetic KnowledgeBase space, the `learn`
//! marks, 我懂 and its optional card (through a fake KnowledgeBase API). Offline.

#[path = "../../../../commons/crates/lore-ooxml/tests/common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use lore_canvas::style::LEARN_EM;
use lore_canvas::{Look, Para, Run, TextBody};
use lore_words::Filters;
use sc_server::kbterms::{self, Familiar, Matcher};
use serde_json::{Value, json};
use tower::ServiceExt;

// ── synthetic knowledge base ──

fn space(root: &Path) -> PathBuf {
    let sp = root.join("kb");
    std::fs::create_dir_all(sp.join("nodes")).unwrap();
    std::fs::create_dir_all(sp.join("progress/ellrin")).unwrap();
    std::fs::write(sp.join("space.toml"), "id = \"me\"\nname = \"Me\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
    let node = |id: &str, title: &str, aliases: &[&str], log: &[&str], state: Option<&str>| {
        let n = json!({ "id": id, "kind": "concept", "title": title, "aliases": aliases, "created_at": "2026-11-01T09:00:00+00:00", "updated_at": "2026-11-01T09:00:00+00:00" });
        std::fs::write(sp.join("nodes").join(format!("{id}.json")), serde_json::to_vec(&n).unwrap()).unwrap();
        if let Some(st) = state {
            let log: Vec<Value> = log.iter().map(|e| json!({ "at": "2026-11-02T09:00:00+00:00", "event": e })).collect();
            let p = json!({ "state": st, "streak": 0, "next_check_at": null, "log": log });
            std::fs::write(sp.join("progress/ellrin").join(format!("{id}.json")), serde_json::to_vec(&p).unwrap()).unwrap();
        }
    };
    node("n01", "Spectral Boundary Loss", &["SBL"], &["created", "remembered", "remembered", "known"], Some("known"));
    node("n02", "U-Net", &[], &["created", "encountered"], Some("learning"));
    node("n03", "Dice coefficient", &[], &["created", "unfamiliar", "encountered"], Some("learning"));
    node("n04", "segmentation", &[], &["created"], Some("stale"));
    // a card made only of common words never makes a term
    node("n05", "the results", &[], &["created"], Some("known"));
    std::fs::write(sp.join("nodes/broken.json"), "not json").unwrap();
    sp
}

/// A synthetic paper, as (section, text).
fn paper() -> Vec<(String, String)> {
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    vec![
        s("1 Introduction", "Medical image segmentation is often done with a U-Net. We propose a hybrid attention gate (HAG) and train it with the Spectral Boundary Loss (SBL). The results show that the proposed method is better."),
        s("2 Method", "The HAG weights skip connections. Contrastive pretraining of the encoder uses a momentum queue. The total loss adds SBL to the Dice coefficient. Contrastive pretraining needs no labels."),
        s("3 Results", "Our model improves the Dice coefficient by 3 points over the U-Net in Table~1. The momentum queue keeps 4096 keys. The results are shown in Section IV."),
    ]
}

#[test]
fn terms_against_the_knowledge_base() {
    let d = tempfile::tempdir().unwrap();
    let sp = space(d.path());
    let before = std::fs::read_dir(sp.join("nodes")).unwrap().count();
    let kb = kbterms::load_names(std::slice::from_ref(&sp), "ellrin");
    assert!(kb.iter().any(|n| n.name == "SBL" && n.state == Familiar::Known));
    assert!(kb.iter().any(|n| n.name == "Dice coefficient" && n.state == Familiar::Unknown), "last answer 不熟 = 要學");
    assert!(kb.iter().any(|n| n.name == "U-Net" && n.state == Familiar::Learning));
    // another member: no learning state → 學習中
    assert!(kbterms::load_names(std::slice::from_ref(&sp), "daidai").iter().all(|n| n.state == Familiar::Learning));

    let r = kbterms::find_terms(&paper(), &kb, &Filters::default());
    let get = |t: &str| r.terms.iter().find(|x| x.text == t).unwrap_or_else(|| panic!("{t} missing: {:?}", r.terms.iter().map(|x| &x.text).collect::<Vec<_>>()));
    let hag = get("hybrid attention gate");
    assert_eq!((hag.kind.as_str(), hag.short.as_deref(), hag.state), ("expansion", Some("HAG"), Familiar::Unknown));
    assert_eq!(hag.label, "不熟");
    assert!(hag.at.iter().any(|w| w.at == "2 Method"), "the acronym later counts for its expansion: {:?}", hag.at);
    let sbl = get("Spectral Boundary Loss");
    assert_eq!((sbl.state, sbl.card.as_deref()), (Familiar::Known, Some("n01")));
    assert_eq!(sbl.at.iter().map(|w| w.count).sum::<usize>(), 3, "title once + SBL twice: {:?}", sbl.at);
    assert_eq!(get("U-Net").state, Familiar::Learning);
    assert_eq!(get("U-Net").label, "學習中");
    let dice = get("Dice coefficient");
    assert_eq!((dice.state, dice.count), (Familiar::Unknown, 2));
    assert_eq!(get("Contrastive pretraining").kind, "phrase");
    assert_eq!(get("momentum queue").state, Familiar::Unknown);
    assert_eq!(get("segmentation").state, Familiar::Learning, "a KB single word");
    // common words and phrases never count
    for t in &r.terms {
        let k = t.key.as_str();
        assert!(!["the result", "proposed method", "medical image", "result", "iv", "table"].contains(&k), "{k} should not be a term");
    }
    assert_eq!(r.total, r.terms.len());
    assert_eq!(r.unfamiliar, r.terms.iter().filter(|t| t.state == Familiar::Unknown).count());
    assert!(r.unfamiliar >= 4);
    assert_eq!(std::fs::read_dir(sp.join("nodes")).unwrap().count(), before, "nothing written into the space");
}

fn look() -> Look {
    Look { font: "Calibri".into(), size: 14.0, color: "000000".into(), ..Look::default() }
}

fn run(text: &str, tag: &str, em: Option<&str>) -> Run {
    let mut r = Run::text(text, tag, look());
    r.em = em.map(str::to_string);
    r
}

fn body(paras: Vec<Vec<Run>>) -> TextBody {
    TextBody { paras: paras.into_iter().map(Para::new).collect(), ..Default::default() }
}

fn marked(t: &TextBody) -> Vec<String> {
    t.paras.iter().flat_map(|p| &p.runs).filter(|r| r.em.as_deref() == Some(LEARN_EM)).map(|r| r.text.clone()).collect()
}

#[test]
fn marks_words_and_sentences_never_key_results() {
    let m = Matcher::new(["hybrid attention gate".to_string(), "HAG".into(), "momentum queue".into()]);
    let src = || {
        body(vec![
            vec![run("Hybrid attention gates", "title", None)],
            vec![run("We add a HAG to each skip. It is cheap. The momentum queues stay.", "body", None)],
            vec![run("HAG: ", "body", None), run("+3.1 Dice with HAG", "body", Some("key"))],
        ])
    };
    let mut words = src();
    assert_eq!(kbterms::mark_body(&mut words, &m, false), 4);
    assert_eq!(marked(&words), ["Hybrid attention gates", "HAG", "momentum queues", "HAG"]);
    let p = &words.paras[1].runs;
    assert_eq!(p.iter().map(|r| r.text.as_str()).collect::<String>(), "We add a HAG to each skip. It is cheap. The momentum queues stay.", "text unchanged");
    assert!(words.paras[2].runs.iter().any(|r| r.text == "+3.1 Dice with HAG" && r.em.as_deref() == Some("key")), "a key result is never re-marked");

    let mut sent = src();
    kbterms::mark_body(&mut sent, &m, true);
    assert_eq!(marked(&sent), ["Hybrid attention gates", "We add a HAG to each skip.", "The momentum queues stay.", "HAG: "], "sentences, but titles keep word marks");

    // 我懂 for HAG: the momentum queue sentence stays
    let keep = Matcher::new(["momentum queue".to_string()]);
    assert!(kbterms::unmark_body(&mut sent, &keep));
    assert_eq!(marked(&sent), ["The momentum queues stay."]);
    assert_eq!(sent.paras[1].runs.len(), 2, "runs joined again");
    assert!(!kbterms::unmark_body(&mut sent, &keep), "nothing more to do");
}

// ── API: 我懂, export, card through KnowledgeBase's API ──

const STYLE: &str = r#"
[pack]
id = "test"
name = "Test"
slide_cx = 9144000
slide_cy = 5143500
[tags.title]
font = "Calibri"
size = 24.0
bold = true
[tags.body]
font = "Calibri"
size = 14.0
[palette.colors]
black = "000000"
red = "FF0000"
[emphasis.key]
color = "FF0000"
bold = true
[ghost]
picture_alpha = 0.2
line_alpha = 0.2
text_color = "A6A6A6"
"#;

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
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null), bytes)
}

/// A stand-in for KnowledgeBase's `/api/spaces` + `POST /api/nodes`; records what it got.
async fn fake_kb() -> (String, Arc<Mutex<Vec<Value>>>) {
    use axum::routing::{get, post};
    let got = Arc::new(Mutex::new(vec![]));
    let g = got.clone();
    let g2 = got.clone();
    let app = axum::Router::new()
        .route("/api/spaces", get(|| async { axum::Json(json!({ "spaces": [{ "id": "family", "kind": "household", "is_member": true }, { "id": "me", "kind": "personal", "is_member": true }] })) }))
        .route(
            "/api/nodes",
            post(move |axum::Json(b): axum::Json<Value>| {
                let g = g.clone();
                async move {
                    g.lock().unwrap().push(b);
                    axum::Json(json!({ "node": { "id": "n99" } }))
                }
            }),
        )
        .route(
            "/api/recall/{id}",
            post(move |axum::extract::Path(id): axum::extract::Path<String>, axum::Json(mut b): axum::Json<Value>| {
                let g = g2.clone();
                async move {
                    b["recall"] = id.into();
                    g.lock().unwrap().push(b);
                    axum::Json(json!({ "ok": true }))
                }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, got)
}

#[tokio::test]
async fn understood_clears_marks_export_never_has_them_and_kb_card_goes_through_the_api() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("styles/test")).unwrap();
    std::fs::write(dir.path().join("styles/test/style.toml"), STYLE).unwrap();
    let ws = sc_core::Workspace::new(dir.path(), None);
    let app = sc_server::router_isolated(sc_server::state(ws, sc_server::repo_dir().join("web")));
    let (st, v, _) = call(&app, "POST", "/api/decks/import?name=demo&style=test", None, Some(common::synthetic_deck())).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, d, _) = call(&app, "GET", "/api/decks/demo", None, None).await;
    let s2 = d["deck"]["slides"][1]["uid"].as_str().unwrap().to_string();

    // generated text with marks, as the slides stage leaves it
    let kb = kbterms::load_names(&[space(dir.path())], "ellrin");
    let mut terms = kbterms::find_terms(&paper(), &kb, &Filters::default());
    let mut t = body(vec![vec![run("We add a HAG to each skip. The momentum queues stay. Plain words here.", "body", None), run(" +3.1 Dice", "body", Some("key"))]]);
    kbterms::mark_body(&mut t, &terms.matcher(), true);
    assert_eq!(marked(&t).len(), 2);
    let (st, v, _) = call(&app, "POST", "/api/decks/demo/ops", Some(json!({ "label": "x", "ops": [{ "op": "set_obj", "slide": s2, "id": 3, "path": "/text/paras", "value": t.paras }] })), None).await;
    assert_eq!(st, StatusCode::OK, "style rules accept learn: {v}");
    kbterms::save_terms(&dir.path().join("decks/demo"), &terms).unwrap();

    // export, print data aside: the pptx never carries the mark or its colour
    let (_, _, bytes) = call(&app, "GET", "/api/decks/demo/pptx", None, None).await;
    let back = lore_ooxml::Deck::from_bytes(&bytes).unwrap();
    let xml = String::from_utf8_lossy(back.pkg.get("ppt/slides/slide2.xml").unwrap()).to_string();
    assert!(!xml.contains("learn") && xml.contains("We add a HAG"));
    let at = xml.find("We add a HAG").unwrap();
    let rpr = &xml[xml[..at].rfind("<a:rPr").unwrap()..at];
    assert!(!rpr.contains("FF0000") && !rpr.contains(" u="), "{rpr}");

    // 我懂 HAG (with its expansion), no card
    let (st, v, _) = call(&app, "PUT", "/api/decks/demo/learn", Some(json!({ "term": "hybrid attention gate", "forms": ["HAG"] })), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["kb"].is_null());
    let obj = v["slides"][0]["objects"].as_array().unwrap().iter().find(|o| o["id"] == 3).unwrap().clone();
    let runs: Vec<&Value> = obj["text"]["paras"][0]["runs"].as_array().unwrap().iter().collect();
    let learn: Vec<&str> = runs.iter().filter(|r| r["em"] == LEARN_EM).map(|r| r["text"].as_str().unwrap()).collect();
    assert_eq!(learn, ["The momentum queues stay."]);
    assert!(runs.iter().any(|r| r["em"] == "key"));
    terms = kbterms::load_terms(&dir.path().join("decks/demo")).unwrap();
    assert!(terms.terms.iter().any(|t| t.short.as_deref() == Some("HAG") && t.understood), "terms.json remembers 我懂");
    // one undo brings the marks back
    let (st, _, _) = call(&app, "POST", "/api/decks/demo/undo", None, None).await;
    assert_eq!(st, StatusCode::OK);

    // 同時存進知識庫（記得）: KnowledgeBase not running → nothing saved, said so
    // SAFETY: tests in this binary that read KB_URL run after this point only
    unsafe { std::env::set_var("KB_URL", "http://127.0.0.1:9") };
    let (_, v, _) = call(&app, "PUT", "/api/decks/demo/learn", Some(json!({ "term": "momentum queue", "kb": true })), None).await;
    assert_eq!(v["kb"]["saved"], false);
    assert!(v["kb"]["message"].as_str().unwrap().contains("沒有開啟"), "{v}");
    // … running: a minimal card in state 記得 in the personal space, through its API
    let (url, rec) = fake_kb().await;
    unsafe { std::env::set_var("KB_URL", &url) };
    let (_, v, _) = call(&app, "PUT", "/api/decks/demo/learn", Some(json!({ "term": "momentum queue", "kb": true })), None).await;
    assert_eq!(v["kb"]["saved"], true, "{v}");
    let got = rec.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["space"], "me");
    assert_eq!(got[0]["title"], "momentum queue");
    assert_eq!(got[0]["initial_state"], "known");
    // a term whose card says 要學: the card gets the answer 記得, no second card
    let (_, v, _) = call(&app, "PUT", "/api/decks/demo/learn", Some(json!({ "term": "Dice coefficient", "kb": true })), None).await;
    assert_eq!((v["kb"]["saved"].clone(), v["kb"]["id"].clone()), (json!(true), json!("n03")), "{v}");
    let got = rec.lock().unwrap().clone();
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!((got[1]["recall"].clone(), got[1]["outcome"].clone()), (json!("n03"), json!("remembered")));
    unsafe { std::env::remove_var("KB_URL") };
    // a deck that is not there: refused before anything reaches KnowledgeBase
    let (st, _, _) = call(&app, "PUT", "/api/decks/nope/learn", Some(json!({ "term": "x", "kb": true })), None).await;
    assert!(!st.is_success());
}
