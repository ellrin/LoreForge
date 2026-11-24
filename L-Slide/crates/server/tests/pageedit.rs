//! 改頁 through L-Echo (pageedit.rs) on a synthetic deck of the public blank base: the propose →
//! check → corrective loop, apply as one undo step, undo, the 409s, engines and material. Mock
//! models, a fake page shot, a fake page queue; never a real model, Chrome or the network.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use lore_llm::router::MockFactory;
use serde_json::{Value, json};
use sc_server::pageedit::{self, Deps, LABEL};
use tower::ServiceExt;

const ASK_EDIT: &str = "You change one or two slides";
const ASK_CHECK: &str = "You look at slides of a talk";

/// Answers by the system prompt: each rule's replies in turn (the last one again and again).
struct Keyed {
    rules: Mutex<Vec<(String, Vec<String>)>>,
    seen: Mutex<Vec<lore_llm::LlmRequest>>,
}

impl Keyed {
    fn new(edit: &[Value], check: &[Value]) -> Arc<Keyed> {
        let r = |v: &[Value]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        Arc::new(Keyed { rules: Mutex::new(vec![(ASK_EDIT.into(), r(edit)), (ASK_CHECK.into(), r(check))]), seen: Default::default() })
    }
    fn asked(&self, words: &str) -> Vec<lore_llm::LlmRequest> {
        self.seen.lock().unwrap().iter().filter(|r| r.system.contains(words)).cloned().collect()
    }
}

#[async_trait::async_trait]
impl lore_llm::LlmProvider for Keyed {
    fn id(&self) -> String {
        "mock".into()
    }
    fn capabilities(&self) -> lore_llm::Capabilities {
        lore_llm::Capabilities { json: true, vision: true, web: false, streaming: true, context: 1_000_000, needs_network: false }
    }
    async fn send(&self, req: &lore_llm::LlmRequest) -> Result<lore_llm::LlmResponse, lore_llm::LlmError> {
        self.seen.lock().unwrap().push(req.clone());
        let mut rules = self.rules.lock().unwrap();
        let text = match rules.iter_mut().find(|(w, _)| req.system.contains(w.as_str())) {
            Some((_, r)) if r.len() > 1 => r.remove(0),
            Some((_, r)) => r.first().cloned().unwrap_or_default(),
            None => String::new(),
        };
        Ok(lore_llm::LlmResponse { text, provider: self.id(), sources: vec![], usage: None })
    }
}

struct Shot(Mutex<Vec<(String, Option<String>)>>);

#[async_trait::async_trait]
impl sc_server::PageShot for Shot {
    async fn png(&self, _deck: &str, slide: &str, preview: Option<&str>, _w: u32) -> Result<Vec<u8>, String> {
        self.0.lock().unwrap().push((slide.into(), preview.map(String::from)));
        Ok(b"\x89PNG\r\n\x1a\nfake".to_vec())
    }
}

/// The page of an open L-Echo request: applies the ops through the deck's own /ops route with the label.
struct Page(axum::Router, Mutex<u32>);

#[async_trait::async_trait]
impl pageedit::PageQueue for Page {
    fn is_open(&self, request: &str) -> bool {
        request == "r1"
    }
    async fn apply(&self, request: &str, _edit: &str, ops: &[lore_canvas::Op]) -> Result<String, (u16, String)> {
        if request != "r1" {
            return Err((410, "closed".into()));
        }
        *self.1.lock().unwrap() += 1;
        let (st, v) = call(&self.0, "POST", "/api/decks/talk/ops", Some(json!({ "ops": ops, "label": LABEL }))).await;
        if st == StatusCode::OK { Ok("已套用".into()) } else { Err((409, v["error"].as_str().unwrap_or("").into())) }
    }
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&b).unwrap())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

struct T {
    _dir: tempfile::TempDir,
    app: axum::Router,
    pe: axum::Router,
    llm: Arc<Keyed>,
    shots: Arc<Shot>,
    page: Arc<Page>,
    slides: Vec<String>,
}

fn look(tag: &str) -> Value {
    let size = if tag == "title" { 32.0 } else { 18.0 };
    json!({ "font": "Arial", "ea_font": "Noto Sans TC", "size": size, "color": "1D1D20", "bold": tag == "title" })
}

fn text_obj(name: &str, slot: &str, y: i64, tag: &str, paras: &[&str]) -> Value {
    let paras: Vec<Value> = paras.iter().map(|t| json!({ "runs": [{ "kind": "text", "text": t, "tag": tag, "look": look(tag) }] })).collect();
    json!({ "id": 0, "name": name, "slot": slot, "frame": { "x": 487680, "y": y, "w": 11216640, "h": if tag == "title" { 534924 } else { 1800000 } }, "type": "text", "geom": { "prst": "rect" }, "fill": { "kind": "none" }, "text": { "paras": paras } })
}

/// Three content slides of the blank base, each a title and two body lines; notes in the shape.
async fn setup(edit: &[Value], check: &[Value], factory: impl FnOnce(Arc<Keyed>) -> MockFactory) -> T {
    let dir = tempfile::tempdir().unwrap();
    let example = sc_server::repo_dir().join("../private-example/SlideComposer");
    lore_home::copy_missing(&example, &dir.path().join("sc")).unwrap();
    let ws = sc_core::Workspace::new(dir.path().join("sc"), None);
    let home = lore_home::Home::at(dir.path());
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    let llm = Keyed::new(edit, check);
    assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home, factory(llm.clone()))).is_ok());
    let shots = Arc::new(Shot(Mutex::new(vec![])));
    assert!(shared.pageshot.set(shots.clone()).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let (st, v) = call(&app, "POST", "/api/decks", Some(json!({ "name": "talk", "style": "blank" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let mut slides = vec![];
    for (i, t) in ["Synthetic Grading Problem", "Method of the Synthetic Grader", "Results on Synthetic Lesions"].iter().enumerate() {
        let objects = vec![text_obj("Title", "title", 630936, "title", &[t]), text_obj("內文", "text-1", 1447038, "body", &["An encoder finds the lesions", "A decoder grades them"])];
        let (st, v) = call(&app, "POST", "/api/decks/talk/new-slide", Some(json!({ "kind": "content", "objects": objects, "after": slides.last() }))).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let uid = v["order"][i].as_str().unwrap().to_string();
        let notes = format!("This slide covers step {}.\n1. One point about it.", i + 1);
        let (st, _) = call(&app, "POST", "/api/decks/talk/ops", Some(json!({ "ops": [{ "op": "set_slide", "slide": uid, "path": "/notes", "value": notes }], "label": "notes" }))).await;
        assert_eq!(st, StatusCode::OK);
        slides.push(uid);
    }
    let page = Arc::new(Page(app.clone(), Mutex::new(0)));
    let deps = Deps {
        queue: page.clone(),
        math: Arc::new(|l: &str, _: bool, _: f64| if l.contains("\\bad") { Err("unknown command".into()) } else { Ok("<m:oMath/>".into()) }),
        paper: Arc::new(|sha: &str, pages: &[u32]| Ok(format!("[synthetic paper {sha} pages {pages:?}] The encoder uses three stages."))),
    };
    let pe = pageedit::router_with(&shared, deps);
    T { _dir: dir, app, pe, llm, shots, page, slides }
}

fn opus(k: Arc<Keyed>) -> MockFactory {
    MockFactory::default().with("claude:opus", k)
}

/// The body object's id on a slide of the deck JSON.
fn body_id(deck: &Value, uid: &str) -> u64 {
    let s = deck["slides"].as_array().unwrap().iter().find(|s| s["uid"] == uid).unwrap();
    s["objects"].as_array().unwrap().iter().find(|o| o["slot"] == "text-1").unwrap()["id"].as_u64().unwrap()
}

fn texts(deck: &Value, uid: &str) -> String {
    deck["slides"].as_array().unwrap().iter().find(|s| s["uid"] == uid).map(|s| s.to_string()).unwrap_or_default()
}

async fn deck(t: &T) -> Value {
    call(&t.app, "GET", "/api/decks/talk", None).await.1
}

fn set_text(uid: &str, obj: u64, paras: &[&str]) -> Value {
    json!({ "edits": [{ "edit": "set_text", "slide": uid, "obj": obj, "paras": paras }] })
}

/// A setup whose slide ids are known before the replies are written (they are s1… in order).
async fn with_plans(edit: impl Fn(&str, u64) -> Vec<Value>, check: &[Value]) -> T {
    // the deck is the same every time: probe once to learn the ids
    let probe = setup(&[json!({})], &[json!({ "findings": [] })], opus).await;
    let d = deck(&probe).await;
    let uid = probe.slides[1].clone();
    let id = body_id(&d["deck"], &uid);
    drop(probe);
    setup(&edit(&uid, id), check, opus).await
}

#[tokio::test]
async fn check_finds_a_problem_the_corrective_round_fixes_it() {
    let bad = json!({ "findings": [{ "slide": "", "kind": "wrapped", "text": "標題換行了" }] });
    let t = with_plans(|u, id| vec![set_text(u, id, &["Encoder finds lesions first", "Decoder grades every one"]), set_text(u, id, &["Encoder finds lesions", "Decoder grades them"])], &[bad, json!({ "findings": [] })]).await;
    let uid = t.slides[1].clone();
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "make the lines shorter" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["findings"], json!([]), "fixed: {v}");
    assert_eq!((t.llm.asked(ASK_EDIT).len(), t.llm.asked(ASK_CHECK).len()), (2, 2));
    let fix = &t.llm.asked(ASK_EDIT)[1];
    assert!(fix.flatten().contains("What failed") && fix.flatten().contains("標題換行了"), "the corrective request carries the findings");
    assert!(t.llm.asked(ASK_CHECK)[0].has_images() && t.llm.asked(ASK_EDIT)[0].has_images());
    assert_eq!(v["before"].as_array().unwrap().len(), 1);
    assert_eq!(v["after"].as_array().unwrap().len(), 1);
    assert_eq!(v["plan"]["edits"][0]["edit"], "set_text");
    // the after pictures were taken of the preview, through its deck route
    let id = v["id"].as_str().unwrap();
    let preview = format!("/api/decks/talk/page-edit/{id}/deck");
    assert!(t.shots.0.lock().unwrap().iter().any(|(s, p)| s == &uid && p.as_deref() == Some(preview.as_str())));
    let (st, pv) = call(&t.pe, "GET", &preview, None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(texts(&pv["deck"], &uid).contains("Decoder grades them") && pv["font_stacks"].is_object());
    let res = t.pe.clone().oneshot(Request::get(format!("/api/decks/talk/page-edit/{id}/image/after-0")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    // the live deck is untouched until 套用
    assert!(!texts(&deck(&t).await["deck"], &uid).contains("Decoder grades them"));
    let log = std::fs::read_to_string(t._dir.path().join("sc/decks/talk/page-edit-log.jsonl")).unwrap();
    assert!(log.contains("make the lines shorter") && log.contains("\"applied\":false"));
}

#[tokio::test]
async fn findings_that_remain_are_returned() {
    let bad = json!({ "findings": [{ "slide": "", "kind": "overlap", "text": "文字壓到圖" }] });
    let t = with_plans(|u, id| vec![set_text(u, id, &["Encoder finds lesions"])], &[bad]).await;
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [t.slides[1]], "instruction": "shorter" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["findings"][0]["text"], "文字壓到圖");
    assert_eq!(t.llm.asked(ASK_EDIT).len(), 2, "one corrective round only");
}

#[tokio::test]
async fn a_refused_edit_is_corrected_once_then_said() {
    let long = "x".repeat(200);
    let t = with_plans(|u, id| vec![json!({ "edits": [{ "edit": "set_text", "slide": u, "obj": id, "paras": [long] }, { "edit": "highlight_crop", "slide": u }] })], &[json!({ "findings": [] })]).await;
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [t.slides[1]], "instruction": "rewrite" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(t.llm.asked(ASK_EDIT).len(), 2);
    assert!(t.llm.asked(ASK_EDIT)[1].flatten().contains("最多"), "the reason went to the corrective round");
    let w = v["warnings"].to_string();
    assert!(w.contains("還不支援") && w.contains("沒有做"), "{w}");
    assert_eq!(v["plan"]["edits"], json!([]));
    let (st, e) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{}/apply", v["id"].as_str().unwrap()), None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{e}");
}

#[tokio::test]
async fn apply_is_one_undoable_step_and_undo_only_while_it_is_next() {
    let t = with_plans(|u, id| vec![json!({ "edits": [{ "edit": "set_text", "slide": u, "obj": id, "paras": ["Encoder finds lesions"] }, { "edit": "set_notes", "slide": u, "text": "This slide covers the encoder.\n1. It finds the lesions." }] })], &[json!({ "findings": [] })]).await;
    let uid = t.slides[1].clone();
    let (_, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "one line" }))).await;
    let id = v["id"].as_str().unwrap().to_string();
    let rev0 = deck(&t).await["status"]["rev"].as_u64().unwrap();
    let (st, a) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/apply"), Some(json!({}))).await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(a["applied"], "server", "no open L-Echo request: the server applies");
    let d = deck(&t).await;
    assert_eq!(d["status"]["undo"], LABEL);
    assert_eq!(d["status"]["rev"].as_u64().unwrap(), rev0 + 1, "one transaction");
    assert!(texts(&d["deck"], &uid).contains("It finds the lesions") && !texts(&d["deck"], &uid).contains("A decoder grades them"));
    let (st, _) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/apply"), None).await;
    assert_eq!(st, StatusCode::CONFLICT, "applied once");
    let (st, u) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/undo"), None).await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert!(texts(&deck(&t).await["deck"], &uid).contains("A decoder grades them"));
    let (st, _) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/undo"), None).await;
    assert_eq!(st, StatusCode::CONFLICT, "nothing applied any more");
    // applied again, then another edit: undo is no longer this one's
    let (st, _) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/apply"), None).await;
    assert_eq!(st, StatusCode::OK);
    let other = t.slides[2].clone();
    call(&t.app, "POST", "/api/decks/talk/ops", Some(json!({ "ops": [{ "op": "set_slide", "slide": other, "path": "/hidden", "value": true }], "label": "hide" }))).await;
    let (st, e) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/undo"), None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{e}");
}

#[tokio::test]
async fn an_open_request_gets_the_ops_through_the_page() {
    let t = with_plans(|u, id| vec![set_text(u, id, &["Encoder finds lesions"])], &[json!({ "findings": [] })]).await;
    let (_, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [t.slides[1]], "instruction": "one line" }))).await;
    let id = v["id"].as_str().unwrap();
    let (st, a) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/apply"), Some(json!({ "request": "r1" }))).await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!((a["applied"].as_str(), *t.page.1.lock().unwrap()), (Some("page"), 1));
    assert_eq!(deck(&t).await["status"]["undo"], LABEL);
    let (st, _) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/undo"), None).await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn a_slide_changed_since_the_proposal_is_409() {
    let t = with_plans(|u, id| vec![set_text(u, id, &["Encoder finds lesions"])], &[json!({ "findings": [] })]).await;
    let uid = t.slides[1].clone();
    let (_, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "one line" }))).await;
    call(&t.app, "POST", "/api/decks/talk/ops", Some(json!({ "ops": [{ "op": "set_slide", "slide": uid, "path": "/notes", "value": "Changed by hand.\n1. Yes." }], "label": "notes" }))).await;
    let (st, e) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{}/apply", v["id"].as_str().unwrap()), None).await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert!(e["error"].as_str().unwrap().contains("改過了"), "{e}");
}

#[tokio::test]
async fn engines_and_material() {
    let t = with_plans(|u, id| vec![set_text(u, id, &["Encoder finds lesions"])], &[json!({ "findings": [] })]).await;
    let uid = t.slides[1].clone();
    for e in ["agy:m-1", "local:qwen", "local"] {
        let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "x", "engine": e }))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
        assert!(!v["error"].as_str().unwrap().contains('\n'));
    }
    assert!(t.llm.asked(ASK_EDIT).is_empty(), "refused before any call");
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "x", "engine": "claude:opus" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "x", "material": { "kb_card": "c1" } }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("kb_card"), "{v}");
    let sha = "a".repeat(64);
    let (st, _) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "x", "material": { "reader_paper": sha, "pages": [3] } }))).await;
    assert_eq!(st, StatusCode::OK);
    assert!(t.llm.asked(ASK_EDIT).last().unwrap().flatten().contains("three stages"));
    // one or two slides
    let (st, _) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": t.slides, "instruction": "x" }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn insert_each_allowed_kind_and_split() {
    let notes = "This slide covers a step.\n1. One point.";
    let t = with_plans(
        |u, _| {
            let mut edits: Vec<Value> = pageedit::insert_kinds()
                .into_iter()
                .map(|k| {
                    let blocks = match k {
                        "thanks" => json!([]),
                        "takeaway" => json!([{ "role": "bullet", "text": "Grading is fast" }]),
                        _ => json!([{ "role": "bullet", "text": "The encoder finds lesions" }, { "role": "suggest", "text": "the encoder figure" }]),
                    };
                    json!({ "edit": "insert_slide", "after": u, "kind": k, "title": format!("New {k} page"), "blocks": blocks, "notes": notes })
                })
                .collect();
            edits.push(json!({ "edit": "insert_slide", "after": u, "kind": "cover", "title": "Second cover", "notes": notes }));
            edits.push(json!({ "edit": "insert_slide", "after": u, "kind": "method", "title": "No picture", "blocks": [{ "role": "bullet", "text": "Only text" }], "notes": notes }));
            vec![json!({ "edits": edits })]
        },
        &[json!({ "findings": [] })],
    )
    .await;
    let uid = t.slides[1].clone();
    let n0 = deck(&t).await["deck"]["slides"].as_array().unwrap().len();
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "add pages" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let kinds = pageedit::insert_kinds();
    assert_eq!(v["plan"]["edits"].as_array().unwrap().len(), kinds.len(), "{}", v["warnings"]);
    let w = v["warnings"].to_string();
    assert!(w.contains("cover") && w.contains("不能只有文字"), "{w}");
    let id = v["id"].as_str().unwrap();
    let (_, pv) = call(&t.pe, "GET", &format!("/api/decks/talk/page-edit/{id}/deck"), None).await;
    let slides = pv["deck"]["slides"].as_array().unwrap();
    assert_eq!(slides.len(), n0 + kinds.len());
    for k in kinds {
        let s = slides.iter().find(|s| s.to_string().contains(&format!("New {k} page"))).unwrap_or_else(|| panic!("{k}"));
        let slot = s["objects"].as_array().unwrap().iter().any(|o| o["name"].as_str().is_some_and(|n| n.starts_with(sc_server::todeck::build::SUGGEST_PREFIX)));
        assert_eq!(slot, !sc_server::todeck::plan::text_only_ok(k), "{k}: an empty picture slot unless text-only is allowed");
    }
    assert_eq!(v["after"].as_array().unwrap().len(), 1 + pageedit::insert_kinds().len(), "the new pages were looked at");
    let (st, a) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{id}/apply"), None).await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(deck(&t).await["deck"]["slides"].as_array().unwrap().len(), n0 + pageedit::insert_kinds().len());
}

#[tokio::test]
async fn split_through_the_route() {
    let probe = setup(&[json!({})], &[json!({ "findings": [] })], opus).await;
    let uid = probe.slides[1].clone();
    let body = body_id(&deck(&probe).await["deck"], &uid);
    drop(probe);
    let notes = ["This part covers the encoder.\n1. One point.", "This part covers the decoder.\n1. One point."];
    let plan = json!({ "edits": [{ "edit": "split_slide", "slide": uid, "groups": [[body], [90]], "titles": ["First half", "Second half"], "notes": notes }] });
    let t = setup(&[plan], &[json!({ "findings": [] })], opus).await;
    // a second body box (id 90) under the first
    let mut extra = text_obj("內文 2", "text-2", 3_600_000, "body", &["A third synthetic line"]);
    extra["id"] = json!(90);
    let (st, e) = call(&t.app, "POST", "/api/decks/talk/ops", Some(json!({ "ops": [{ "op": "insert_obj", "slide": uid, "index": 99, "obj": extra }], "label": "add" }))).await;
    assert_eq!(st, StatusCode::OK, "{e}");
    let n0 = deck(&t).await["deck"]["slides"].as_array().unwrap().len();
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "split" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["edits"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["after"].as_array().unwrap().len(), 2, "both parts looked at");
    let (st, _) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{}/apply", v["id"].as_str().unwrap()), None).await;
    assert_eq!(st, StatusCode::OK);
    let d = deck(&t).await;
    let slides = d["deck"]["slides"].as_array().unwrap();
    assert_eq!(slides.len(), n0 + 1);
    let i = slides.iter().position(|s| s["uid"] == uid).unwrap();
    let (a, b) = (slides[i].to_string(), slides[i + 1].to_string());
    assert!(a.contains("First half") && a.contains("A decoder grades them") && !a.contains("A third synthetic line"));
    assert!(b.contains("Second half") && b.contains("A third synthetic line") && !b.contains("A decoder grades them"));
    assert_eq!(d["status"]["undo"], LABEL);
}
