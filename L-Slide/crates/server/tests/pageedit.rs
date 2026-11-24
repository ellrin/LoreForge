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
    let t = with_plans(|u, id| vec![json!({ "edits": [{ "edit": "set_text", "slide": u, "obj": id, "paras": [long] }, { "edit": "make_it_pop", "slide": u }] })], &[json!({ "findings": [] })]).await;
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [t.slides[1]], "instruction": "rewrite" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(t.llm.asked(ASK_EDIT).len(), 2);
    assert!(t.llm.asked(ASK_EDIT)[1].flatten().contains("最多"), "the reason went to the corrective round");
    let w = v["warnings"].to_string();
    assert!(w.contains("不認得") && w.contains("沒有做"), "{w}");
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
    let plan = json!({ "edits": [{ "edit": "split_slide", "slide": uid, "groups": [[body], [90, 91]], "titles": ["First half", "Second half"], "notes": notes }] });
    // then an inserted page of the same base: where a freshly built page starts its text
    let insert = json!({ "edits": [{ "edit": "insert_slide", "after": uid, "kind": "takeaway", "title": "Fresh page", "blocks": [{ "role": "bullet", "text": "A fresh line" }], "notes": notes[0] }] });
    // (the synthetic title runs over its box on the new page: the corrective round gets the same plan)
    let t = setup(&[plan.clone(), plan, insert], &[json!({ "findings": [] })], opus).await;
    // a second body box (id 90) under the first, and a shape the author drew (id 91) beside it
    let mut extra = text_obj("內文 2", "text-2", 3_600_000, "body", &["A third synthetic line"]);
    extra["id"] = json!(90);
    let drawn = json!({ "id": 91, "name": "drawn mark", "frame": { "x": 9_000_000, "y": 3_700_000, "w": 600_000, "h": 400_000 }, "type": "shape", "geom": { "prst": "ellipse" }, "fill": { "kind": "none" } });
    let (st, e) = call(&t.app, "POST", "/api/decks/talk/ops", Some(json!({ "ops": [{ "op": "insert_obj", "slide": uid, "index": 99, "obj": extra }, { "op": "insert_obj", "slide": uid, "index": 99, "obj": drawn }], "label": "add" }))).await;
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
    assert!(a.contains("First half") && a.contains("A decoder grades them") && !a.contains("A third synthetic line"), "{a}\n{}", v["warnings"]);
    assert!(b.contains("Second half") && b.contains("A third synthetic line") && !b.contains("A decoder grades them"));
    assert_eq!(d["status"]["undo"], LABEL);
    // reflow: the second page's text starts at the top of 內文, where a freshly built page starts
    // it; the drawn shape kept its place and is said
    let (st, ins) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "add a page" }))).await;
    assert_eq!(st, StatusCode::OK, "{ins}");
    let (_, pv) = call(&t.pe, "GET", &format!("/api/decks/talk/page-edit/{}/deck", ins["id"].as_str().unwrap()), None).await;
    let fresh = pv["deck"]["slides"].as_array().unwrap().iter().find(|s| s.to_string().contains("Fresh page")).unwrap().clone();
    let top = obj(&fresh, |o| o["name"] == "內文")["frame"]["y"].as_i64().unwrap();
    let (first, second) = (&slides[i], &slides[i + 1]);
    let moved = obj(second, |o| o["id"] == 90);
    assert_eq!(moved["frame"]["y"].as_i64().unwrap(), top, "{}", moved["frame"]);
    assert_eq!(obj(first, |o| o["id"] == body)["frame"]["y"].as_i64().unwrap(), top);
    assert_eq!(obj(second, |o| o["id"] == 91)["frame"]["y"], 3_700_000);
    assert!(v["warnings"].to_string().contains("drawn mark") && v["warnings"].to_string().contains("留在原本的位置"), "{}", v["warnings"]);
}

fn obj(slide: &Value, f: impl Fn(&Value) -> bool) -> Value {
    slide["objects"].as_array().unwrap().iter().find(|o| f(o)).cloned().unwrap_or_else(|| panic!("no such object on {}", slide["uid"]))
}

fn fr(o: &Value, k: &str) -> i64 {
    o["frame"][k].as_i64().unwrap()
}

fn slide_of(deck: &Value, uid: &str) -> Value {
    deck["slides"].as_array().unwrap().iter().find(|s| s["uid"] == uid).cloned().unwrap()
}

/// A synthetic architecture figure: three blocks with blank gutters at 1/3 and 2/3, arrows across.
fn figure_png() -> Vec<u8> {
    let mut img = image::RgbaImage::from_pixel(900, 300, image::Rgba([255, 255, 255, 255]));
    for (k, x0) in [30u32, 330, 630].into_iter().enumerate() {
        let c = [[52u8, 101, 164], [230, 145, 30], [60, 150, 90]][k];
        for y in 40..260u32 {
            for x in x0..x0 + 240 {
                img.put_pixel(x, y, image::Rgba([c[0], c[1], c[2], 255]));
            }
        }
    }
    for x in (270..330u32).chain(570..630) {
        for y in 149..152 {
            img.put_pixel(x, y, image::Rgba([60, 60, 60, 255]));
        }
    }
    let mut buf = std::io::Cursor::new(vec![]);
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

/// A setup like [`with_plans`] whose middle slide is changed by `prep` (ops) before anything is
/// proposed; `prep` gets the slide, its body id and the deck's folder (for assets).
async fn prepared(edit: impl Fn(&str, u64) -> Vec<Value>, check: &[Value], prep: impl Fn(&str, u64, &std::path::Path) -> Vec<Value>) -> (T, String, u64) {
    let probe = setup(&[json!({})], &[json!({ "findings": [] })], opus).await;
    let uid = probe.slides[1].clone();
    let body = body_id(&deck(&probe).await["deck"], &uid);
    drop(probe);
    let t = setup(&edit(&uid, body), check, opus).await;
    let ops = prep(&uid, body, &t._dir.path().join("sc/decks/talk"));
    if !ops.is_empty() {
        let (st, e) = call(&t.app, "POST", "/api/decks/talk/ops", Some(json!({ "ops": ops, "label": "prep" }))).await;
        assert_eq!(st, StatusCode::OK, "{e}");
    }
    (t, uid, body)
}

async fn propose(t: &T, uid: &str) -> Value {
    let (st, v) = call(&t.pe, "POST", "/api/decks/talk/page-edit", Some(json!({ "slides": [uid], "instruction": "change it" }))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    v
}

async fn preview(t: &T, v: &Value) -> Value {
    call(&t.pe, "GET", &format!("/api/decks/talk/page-edit/{}/deck", v["id"].as_str().unwrap()), None).await.1["deck"].clone()
}

async fn apply(t: &T, v: &Value) {
    let (st, a) = call(&t.pe, "POST", &format!("/api/decks/talk/page-edit/{}/apply", v["id"].as_str().unwrap()), None).await;
    assert_eq!(st, StatusCode::OK, "{a}");
}

/// The exported .pptx's slide XML of the slide at `uid`.
async fn slide_xml(t: &T, uid: &str) -> String {
    let d = deck(t).await;
    let n = d["deck"]["slides"].as_array().unwrap().iter().position(|s| s["uid"] == uid).unwrap() + 1;
    let res = t.app.clone().oneshot(Request::builder().uri("/api/decks/talk/pptx").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut x = String::new();
    std::io::Read::read_to_string(&mut zip.by_name(&format!("ppt/slides/slide{n}.xml")).unwrap(), &mut x).unwrap();
    x
}

/// 排版用字's checks on the deck as it is (no model): the items about `uid`'s size and overflow.
async fn polish_items(t: &T, uid: &str) -> Vec<Value> {
    let (st, rep) = call(&t.app, "POST", "/api/decks/talk/polish", Some(json!({ "terms": false }))).await;
    assert_eq!(st, StatusCode::OK, "{rep}");
    rep["items"].as_array().unwrap().iter().filter(|i| i["slide"] == uid && i["check"] == 1).cloned().collect()
}

const CRAMMED: &str = "Recall: found over all lesions; Precision: found that are real; F1: both in one; AUC: area under the curve";
const QUARTERS: [&str; 4] = ["Recall: found over all lesions;", "Precision: found that are real;", "F1: both in one;", "AUC: area under the curve"];

#[tokio::test]
async fn split_object_into_a_2x2_grid_in_its_frame() {
    let plan = |u: &str, id: u64| vec![json!({ "edits": [{ "edit": "split_object", "slide": u, "obj": id, "parts": QUARTERS, "arrange": "2x2" }] })];
    let (t, uid, body) = prepared(plan, &[json!({ "findings": [] })], |u, id, _| vec![json!({ "op": "set_obj", "slide": u, "id": id, "path": "/text/paras/0/runs/0/text", "value": CRAMMED }), json!({ "op": "set_obj", "slide": u, "id": id, "path": "/text/paras", "value": [deck_para(CRAMMED)] })]).await;
    let before = slide_of(&deck(&t).await["deck"], &uid);
    let was = obj(&before, |o| o["id"] == body);
    let v = propose(&t, &uid).await;
    assert_eq!(v["plan"]["edits"].as_array().unwrap().len(), 1, "{v}");
    let s = slide_of(&preview(&t, &v).await, &uid);
    let cells: Vec<Value> = (1..=4).map(|k| obj(&s, |o| o["name"] == format!("內文 {k}"))).collect();
    assert_eq!(cells[0]["id"], body, "the first part keeps the object");
    let gap = 6_858_000 / 60;
    for (k, c) in cells.iter().enumerate() {
        assert_eq!(c["text"]["paras"][0]["runs"][0]["text"], QUARTERS[k].trim_end_matches(';'), "no separator left at the end of a part");
        assert_eq!(c["text"]["paras"][0]["runs"][0]["look"]["size"], 18.0, "the role's size");
        assert_eq!(c["text"]["paras"][0]["runs"][0]["tag"], "body");
        assert!(fr(c, "x") >= fr(&was, "x") && fr(c, "x") + fr(c, "w") <= fr(&was, "x") + fr(&was, "w") + 1, "inside the original width");
        assert_eq!(fr(c, "y") >= fr(&was, "y"), true);
    }
    // the cells: two columns with the deck's gutter, two rows
    assert_eq!((fr(&cells[0], "x"), fr(&cells[2], "x")), (fr(&was, "x"), fr(&was, "x")));
    assert_eq!(fr(&cells[1], "x"), fr(&cells[0], "x") + fr(&cells[0], "w") + 2 * gap);
    assert_eq!((fr(&cells[0], "y"), fr(&cells[1], "y")), (fr(&was, "y"), fr(&was, "y")));
    assert_eq!(fr(&cells[2], "y"), fr(&cells[0], "y") + fr(&cells[0], "h") + gap);
    assert_eq!(fr(&cells[0], "w"), fr(&cells[1], "w"));
    // applied: ordinary text boxes in the .pptx, nothing for 排版用字 to fix
    apply(&t, &v).await;
    let x = slide_xml(&t, &uid).await;
    for q in QUARTERS {
        assert!(x.contains(q.trim_end_matches(';')), "{q}");
    }
    assert!(x.matches("<p:txBody>").count() >= 5, "four boxes and the title");
    let items = polish_items(&t, &uid).await;
    assert!(items.is_empty(), "{items:?}");
}

fn deck_para(t: &str) -> Value {
    json!({ "runs": [{ "kind": "text", "text": t, "tag": "body", "look": look("body") }] })
}

fn all(v: &Value) -> String {
    v.to_string()
}

#[tokio::test]
async fn split_object_refused_when_a_part_does_not_fit() {
    // a box right under the line leaves it one line of room: a row of four narrow cells needs more
    let row = |u: &str, id: u64| json!({ "edit": "split_object", "slide": u, "obj": id, "parts": QUARTERS, "arrange": "row" });
    let plan = move |u: &str, id: u64| vec![json!({ "edits": [row(u, id)] })];
    let (t, uid, _) = prepared(plan, &[json!({ "findings": [] })], |u, id, _| {
        let mut below = text_obj("內文 2", "text-2", 1_447_038 + 520_000, "body", &["A line under it"]);
        below["id"] = json!(90);
        vec![json!({ "op": "set_obj", "slide": u, "id": id, "path": "/text/paras", "value": [deck_para(CRAMMED)] }), json!({ "op": "set_obj", "slide": u, "id": id, "path": "/frame/h", "value": 400_000 }), json!({ "op": "insert_obj", "slide": u, "index": 99, "obj": below })]
    })
    .await;
    let v = propose(&t, &uid).await;
    assert_eq!(v["plan"]["edits"], json!([]));
    let w = v["warnings"].to_string();
    assert!(w.contains("排成 row") && w.contains("不縮字") && w.contains("改用"), "{w}");
    assert!(t.llm.asked(ASK_EDIT)[1].flatten().contains("排成 row"), "the reason went to the corrective round");
}

fn picture(id: u32, asset: &str, x: i64, y: i64, w: i64, h: i64) -> Value {
    json!({ "id": id, "name": "圖", "slot": "image-1", "frame": { "x": x, "y": y, "w": w, "h": h }, "type": "picture", "asset": asset, "crop": { "l": 0.0, "t": 0.0, "r": 0.0, "b": 0.0 }, "alpha": 1.0, "clip": "rect" })
}

/// The middle slide as a built figure page: the figure across the body (3:1), the two lines under it.
fn figure_page(u: &str, id: u64, dir: &std::path::Path) -> Vec<Value> {
    let asset = sc_core::store_asset(dir, &figure_png(), "png").unwrap();
    vec![
        json!({ "op": "insert_obj", "slide": u, "index": 99, "obj": picture(80, &asset, 487_680, 1_447_038, 7_200_000, 2_400_000) }),
        json!({ "op": "set_obj", "slide": u, "id": id, "path": "/frame", "value": { "x": 487_680, "y": 4_000_000, "w": 11_216_640, "h": 900_000 } }),
    ]
}

#[tokio::test]
async fn crop_shows_the_box_snapped_to_the_gutters_beside_the_lines() {
    // the model's box is a little off the middle block: its edges land in the blank gutters
    let plan = |u: &str, _| vec![json!({ "edits": [{ "edit": "highlight_crop", "slide": u, "picture": 80, "mode": "crop", "box": [0.35, 0.1, 0.65, 0.9] }] })];
    let (t, uid, body) = prepared(plan, &[json!({ "findings": [] })], figure_page).await;
    let v = propose(&t, &uid).await;
    assert_eq!(v["plan"]["edits"].as_array().unwrap().len(), 1, "{v}");
    let s = slide_of(&preview(&t, &v).await, &uid);
    let p = obj(&s, |o| o["id"] == 80);
    let c = |k: &str| p["crop"][k].as_f64().unwrap();
    assert!((c("l") - 1.0 / 3.0).abs() < 0.01 && (c("r") - 1.0 / 3.0).abs() < 0.01, "snapped to the gutters at 1/3 and 2/3: {}", p["crop"]);
    assert!(c("t") > 0.03 && c("t") < 0.134 && c("b") > 0.03 && c("b") < 0.134, "{}", p["crop"]);
    assert_eq!(p["alpha"], 1.0);
    // the region's aspect kept, beside the lines (the part-beside-text frames)
    let aspect = (7_200_000.0 * (1.0 - c("l") - c("r"))) / (2_400_000.0 * (1.0 - c("t") - c("b")));
    assert!((fr(&p, "w") as f64 / fr(&p, "h") as f64 - aspect).abs() < 0.02, "{} {aspect}", p["frame"]);
    let text = obj(&s, |o| o["id"] == body);
    assert!(fr(&text, "x") >= fr(&p, "x") + fr(&p, "w") && fr(&text, "y") == fr(&p, "y"), "{} {}", text["frame"], p["frame"]);
    assert!(t.llm.asked(ASK_CHECK)[0].flatten().contains("Look closely") && t.llm.asked(ASK_CHECK)[0].flatten().contains("box"), "the check looks at the box");
    apply(&t, &v).await;
    let x = slide_xml(&t, &uid).await;
    let l = x.split("<a:srcRect l=\"").nth(1).and_then(|r| r.split('"').next()).and_then(|n| n.parse::<i64>().ok()).unwrap_or(0);
    assert!((33_000..34_000).contains(&l), "srcRect: {x}");
    let items = polish_items(&t, &uid).await;
    assert!(items.is_empty(), "{items:?}");
}

#[tokio::test]
async fn highlight_fades_the_picture_and_lays_the_box_clear_on_it() {
    let plan = |u: &str, _| vec![json!({ "edits": [{ "edit": "highlight_crop", "slide": u, "picture": 80, "mode": "highlight", "box": [0.0, 0.0, 0.64, 1.0] }] })];
    let (t, uid, _) = prepared(plan, &[json!({ "findings": [] })], figure_page).await;
    let v = propose(&t, &uid).await;
    assert_eq!(v["plan"]["edits"].as_array().unwrap().len(), 1, "{v}");
    let s = slide_of(&preview(&t, &v).await, &uid);
    let whole = obj(&s, |o| o["id"] == 80);
    let piece = obj(&s, |o| o["name"] == "清楚的一段");
    assert!((whole["alpha"].as_f64().unwrap() - 0.25).abs() < 1e-9, "the style's ghost alpha: {}", whole["alpha"]);
    assert_eq!((whole["frame"].clone(), whole["asset"].clone()), (json!({ "x": 487_680, "y": 1_447_038, "w": 7_200_000, "h": 2_400_000 }), piece["asset"].clone()), "the picture stays where it was");
    assert_eq!(piece["alpha"], 1.0);
    let r = piece["crop"]["r"].as_f64().unwrap();
    assert!((r - 1.0 / 3.0).abs() < 0.01, "the right edge snapped to the gutter at 2/3: {}", piece["crop"]);
    assert_eq!((fr(&piece, "x"), fr(&piece, "y"), fr(&piece, "h")), (fr(&whole, "x"), fr(&whole, "y"), fr(&whole, "h")));
    assert!((fr(&piece, "w") as f64 / fr(&whole, "w") as f64 - (1.0 - r)).abs() < 0.001);
    // over the faint one, in z-order
    let ids: Vec<u64> = s["objects"].as_array().unwrap().iter().map(|o| o["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.iter().position(|i| *i == piece["id"].as_u64().unwrap()), ids.iter().position(|i| *i == 80).map(|k| k + 1));
    apply(&t, &v).await;
    let x = slide_xml(&t, &uid).await;
    assert!(x.contains("<a:alphaModFix amt=\"25000\"") && x.contains("<a:srcRect r=\""), "{x}");
}

#[tokio::test]
async fn equations_under_their_lines_then_as_two_columns() {
    let notes = "This slide covers the loss.\n1. $L$ denotes the loss.\n2. $R$ denotes the reconstruction term.\n3. $\\lambda$ denotes its weight.";
    let plan = move |u: &str, id: u64| {
        vec![json!({ "edits": [
            { "edit": "add_equation", "slide": u, "latex": "L = \\lambda R", "explains": id, "line": 2, "display": true },
            { "edit": "set_notes", "slide": u, "text": notes },
        ] }), json!({ "edits": [
            { "edit": "add_equation", "slide": u, "latex": "L = \\lambda R", "explains": id, "line": 2, "display": true },
            { "edit": "add_equation", "slide": u, "latex": "R = \\lambda L", "explains": id, "line": 1, "display": true },
            { "edit": "add_equation", "slide": u, "latex": "L = R", "explains": id, "line": 1, "display": true },
            { "edit": "set_notes", "slide": u, "text": notes },
        ] })]
    };
    let (t, uid, body) = prepared(plan, &[json!({ "findings": [] })], |u, id, _| vec![json!({ "op": "set_obj", "slide": u, "id": id, "path": "/frame/h", "value": 700_000 })]).await;
    let kinds = |o: &Value| o["text"]["paras"].as_array().unwrap().iter().map(|p| p["runs"][0]["kind"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    // one: right under the line it explains, in the same box, grown to its text
    let v = propose(&t, &uid).await;
    assert_eq!(v["plan"]["edits"].as_array().unwrap().len(), 2, "{v}");
    let s = slide_of(&preview(&t, &v).await, &uid);
    let b = obj(&s, |o| o["id"] == body);
    assert_eq!(kinds(&b), ["text", "text", "math"]);
    let eq = &b["text"]["paras"][2];
    assert_eq!((eq["runs"][0]["latex"].as_str(), eq["align"].as_str()), (Some("L = \\lambda R"), Some("ctr")));
    assert!(fr(&b, "h") > 700_000, "the box grew to its text: {}", b["frame"]);
    // two: the paired columns, the third refused (two per slide)
    let v = propose(&t, &uid).await;
    let s = slide_of(&preview(&t, &v).await, &uid);
    let (l1, e1, l2, e2) = (obj(&s, |o| o["name"] == "內文 1"), obj(&s, |o| o["name"] == "配對方程式 1"), obj(&s, |o| o["name"] == "內文 2"), obj(&s, |o| o["name"] == "配對方程式 2"));
    assert!(all(&l1).contains("An encoder") && all(&e1).contains("R = \\\\lambda L") && all(&l2).contains("A decoder") && all(&e2).contains("L = \\\\lambda R"), "{l1} {e1}");
    for (l, e) in [(&l1, &e1), (&l2, &e2)] {
        assert_eq!((fr(e, "x"), fr(e, "w"), fr(e, "y")), (fr(l, "x"), fr(l, "w"), fr(l, "y") + fr(l, "h")));
    }
    assert!(fr(&l1, "y") == fr(&l2, "y") && fr(&e1, "y") == fr(&e2, "y") && fr(&l1, "x") == 487_680 && fr(&l2, "x") + fr(&l2, "w") <= 487_680 + 11_216_640);
    assert!(!s.to_string().contains(&format!("\"id\":{body},")), "the one box became the columns");
    let w = v["warnings"].to_string();
    assert!(w.contains("一頁最多 2 條"), "{w}");
    apply(&t, &v).await;
    let x = slide_xml(&t, &uid).await;
    assert_eq!(x.matches("<m:oMath").count(), 2, "{x}");
    assert!(polish_items(&t, &uid).await.is_empty());
}

/// Plans by the slide they are for (the visual fixture: one proposal per slide, a corrective
/// round gets the same plan again).
struct ByUid(Vec<(String, String)>);

#[async_trait::async_trait]
impl lore_llm::LlmProvider for ByUid {
    fn id(&self) -> String {
        "mock".into()
    }
    fn capabilities(&self) -> lore_llm::Capabilities {
        lore_llm::Capabilities { json: true, vision: true, web: false, streaming: true, context: 1_000_000, needs_network: false }
    }
    async fn send(&self, req: &lore_llm::LlmRequest) -> Result<lore_llm::LlmResponse, lore_llm::LlmError> {
        let text = if req.system.contains(ASK_CHECK) {
            json!({ "findings": [] }).to_string()
        } else {
            let f = req.flatten();
            self.0.iter().find(|(u, _)| f.contains(&format!("\"slide\":\"{u}\""))).map(|(_, p)| p.clone()).unwrap_or_else(|| json!({ "edits": [] }).to_string())
        };
        Ok(lore_llm::LlmResponse { text, provider: self.id(), sources: vec![], usage: None })
    }
}

/// Visual check only (never in CI): in `SC_UI_DIR`, decks `before` and `after` of one slide per
/// new kind (2×2 split, crop, highlight, two equations with a conclusion line, a split page), the
/// after deck through the real 改頁 path (mock model, real pandoc, fake page shot); `pages.json`
/// lists the slides. Render them with the isolated server
/// (`SLIDECOMPOSER_WORKSPACE=<dir>/sc … isolated_server -- <dir> <port>`, `GET …/slides/{uid}/png`).
#[tokio::test]
#[ignore]
async fn kinds_fixture() {
    let Ok(d) = std::env::var("SC_UI_DIR") else { return };
    let dir = std::path::PathBuf::from(d);
    let example = sc_server::repo_dir().join("../private-example/SlideComposer");
    lore_home::copy_missing(&example, &dir.join("sc")).unwrap();
    let ws = sc_core::Workspace::new(dir.join("sc"), None);
    let home = lore_home::Home::at(dir.join("home"));
    let shared = sc_server::state_at(ws, sc_server::repo_dir().join("web"), home.clone());
    assert!(shared.pageshot.set(Arc::new(Shot(Mutex::new(vec![])))).is_ok());
    let app = sc_server::router_isolated(shared.clone());
    let notes = "This slide covers the loss.\n1. $L$ denotes the loss.\n2. $R$ denotes the reconstruction term.\n3. $\\lambda$ denotes its weight.";
    let titles = ["Four measures in one line", "Fusion weighs the lesions", "Where fusion sits", "Two terms of the loss", "Encoder and decoder", "The grader's blocks", "The grader at a glance"];
    let mut pages = vec![];
    let mut plans: Vec<(String, String)> = vec![];
    for name in ["before", "after"] {
        let (st, v) = call(&app, "POST", "/api/decks", Some(json!({ "name": name, "style": "blank" }))).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let ddir = dir.join("sc/decks").join(name);
        let asset = sc_core::store_asset(&ddir, &figure_png(), "png").unwrap();
        let mut uids: Vec<String> = vec![];
        for (k, t) in titles.iter().enumerate() {
            let mut objects = vec![text_obj("Title", "title", 630936, "title", &[t])];
            let mut body = |paras: &[&str], y: i64, hh: i64| {
                let mut b = text_obj("內文", "text-1", y, "body", paras);
                b["id"] = json!(70);
                b["frame"]["h"] = json!(hh);
                objects.push(b);
            };
            match k {
                0 => body(&[CRAMMED], 1_447_038, 500_000),
                5 | 6 => {
                    body(&["Each block hands its output to the next", "Only the middle block weighs lesions"], 4_400_000, 900_000);
                    objects.push(picture(80, &asset, 487_680, 1_447_038, 7_200_000, 2_400_000));
                    objects.push(caption(81, 3_900_000));
                }
                1 | 2 => {
                    body(&["Each lesion gets a score from its size", "The three highest scores are kept"], 4_000_000, 900_000);
                    objects.push(picture(80, &asset, 487_680, 1_447_038, 7_200_000, 2_400_000));
                }
                3 => {
                    body(&["Reconstruction compares the image with its copy", "Grading compares the grade scores with the label"], 1_447_038, 800_000);
                    let mut tk = text_obj("takeaway", "takeaway", 2_400_000, "takeaway", &["One weight balances both terms"]);
                    tk["id"] = json!(75);
                    tk["frame"]["h"] = json!(450_000);
                    objects.push(tk);
                }
                _ => {
                    body(&["An encoder finds the lesions", "A decoder grades them"], 1_447_038, 800_000);
                    let mut b2 = text_obj("內文 2", "text-2", 3_600_000, "body", &["The decoder sees only the lesions the encoder kept"]);
                    b2["id"] = json!(90);
                    b2["frame"]["h"] = json!(600_000);
                    objects.push(b2);
                    objects.push(json!({ "id": 91, "name": "drawn mark", "frame": { "x": 9_000_000, "y": 4_400_000, "w": 600_000, "h": 400_000 }, "type": "shape", "geom": { "prst": "ellipse" }, "fill": { "kind": "solid", "color": "1D1D20" } }));
                }
            }
            let (st, v) = call(&app, "POST", &format!("/api/decks/{name}/new-slide"), Some(json!({ "kind": "content", "objects": objects, "after": uids.last() }))).await;
            assert_eq!(st, StatusCode::OK, "{v}");
            let d = call(&app, "GET", &format!("/api/decks/{name}"), None).await.1;
            let uid = d["deck"]["slides"].as_array().unwrap().iter().map(|s| s["uid"].as_str().unwrap().to_string()).find(|u| !uids.contains(u) && s_has(&d, u, t)).unwrap();
            call(&app, "POST", &format!("/api/decks/{name}/ops"), Some(json!({ "ops": [{ "op": "set_slide", "slide": uid, "path": "/notes", "value": "This slide covers one step.\n1. One point." }], "label": "notes" }))).await;
            uids.push(uid);
        }
        if name == "before" {
            pages = uids.clone();
            continue;
        }
        let u = &uids;
        let dd = call(&app, "GET", "/api/decks/after", None).await.1;
        let id = |k: usize, f: &dyn Fn(&Value) -> bool| obj(&slide_of(&dd["deck"], &u[k]), f)["id"].as_u64().unwrap();
        let body = |k: usize| id(k, &|o| o["slot"] == "text-1");
        let pic = |k: usize| id(k, &|o| o["type"] == "picture");
        let (b2, mark) = (id(4, &|o| o["slot"] == "text-2"), id(4, &|o| o["name"] == "drawn mark"));
        plans = vec![
            (u[0].clone(), json!({ "edits": [{ "edit": "split_object", "slide": u[0], "obj": body(0), "parts": QUARTERS, "arrange": "2x2" }] }).to_string()),
            (u[1].clone(), json!({ "edits": [{ "edit": "highlight_crop", "slide": u[1], "picture": pic(1), "mode": "crop", "box": [0.35, 0.1, 0.65, 0.9] }] }).to_string()),
            (u[2].clone(), json!({ "edits": [{ "edit": "highlight_crop", "slide": u[2], "picture": pic(2), "mode": "highlight", "box": [0.35, 0.1, 0.65, 0.9] }] }).to_string()),
            (u[3].clone(), json!({ "edits": [
                { "edit": "add_equation", "slide": u[3], "latex": "R = \\lambda L", "explains": body(3), "line": 1, "display": true },
                { "edit": "add_equation", "slide": u[3], "latex": "L = \\lambda R", "explains": body(3), "line": 2, "display": true },
                { "edit": "set_notes", "slide": u[3], "text": notes },
            ] }).to_string()),
            (u[5].clone(), json!({ "edits": [{ "edit": "highlight_crop", "slide": u[5], "picture": pic(5), "mode": "crop", "box": [0.35, 0.1, 0.65, 0.9] }] }).to_string()),
            (u[6].clone(), json!({ "edits": [{ "edit": "split_slide", "slide": u[6], "groups": [[pic(6), id(6, &|o| o["slot"] == "caption-1")], [body(6)]], "titles": ["The grader at a glance", "What the blocks do"], "notes": ["This part shows the figure.\n1. One point.", "This part says what it means.\n1. One point."] }] }).to_string()),
            (u[4].clone(), json!({ "edits": [{ "edit": "split_slide", "slide": u[4], "groups": [[body(4)], [b2, mark]], "titles": ["Encoder and decoder", "What the decoder sees"], "notes": ["This part covers the encoder.\n1. One point.", "This part covers the decoder.\n1. One point."] }] }).to_string()),
        ];
        assert!(shared.llm.set(sc_server::llm::Llm::with_factory(home.clone(), MockFactory::default().with("claude:opus", Arc::new(ByUid(plans.clone()))))).is_ok());
        let pe = pageedit::router(&shared, None);
        let mut out = vec![];
        for (k, uid) in u.iter().enumerate() {
            let (st, v) = call(&pe, "POST", "/api/decks/after/page-edit", Some(json!({ "slides": [uid], "instruction": "the author's change" }))).await;
            assert_eq!(st, StatusCode::OK, "{v}");
            assert_eq!(v["plan"]["edits"].as_array().map(Vec::len), Some(if k == 3 { 3 } else { 1 }), "{v}");
            let (st, a) = call(&pe, "POST", &format!("/api/decks/after/page-edit/{}/apply", v["id"].as_str().unwrap()), None).await;
            assert_eq!(st, StatusCode::OK, "{a}");
            out.push(json!({ "title": titles[k], "before": pages[k], "slide": uid, "warnings": v["warnings"] }));
        }
        let d = call(&app, "GET", "/api/decks/after", None).await.1;
        let order: Vec<String> = d["deck"]["slides"].as_array().unwrap().iter().map(|s| s["uid"].as_str().unwrap().to_string()).collect();
        for k in [4, 6] {
            out[k]["second"] = json!(order[order.iter().position(|x| *x == u[k]).unwrap() + 1].clone());
        }
        std::fs::write(dir.join("pages.json"), serde_json::to_vec_pretty(&out).unwrap()).unwrap();
    }
}

fn s_has(d: &Value, uid: &str, title: &str) -> bool {
    d["deck"]["slides"].as_array().unwrap().iter().any(|s| s["uid"] == uid && s.to_string().contains(title))
}

#[tokio::test]
async fn an_equation_with_an_undefined_symbol_is_refused_by_the_notation_check() {
    let plan = |u: &str, id: u64| vec![json!({ "edits": [{ "edit": "add_equation", "slide": u, "latex": "Q = \\mu P", "explains": id, "line": 1, "display": true }] })];
    let (t, uid, _) = prepared(plan, &[json!({ "findings": [] })], |_, _, _| vec![]).await;
    let v = propose(&t, &uid).await;
    assert_eq!(v["plan"]["edits"], json!([]));
    assert!(v["warnings"].to_string().contains("對照表"), "{}", v["warnings"]);
}


#[tokio::test]
async fn split_keeps_each_paired_equation_under_its_line() {
    let notes = "This slide covers the loss.\n1. $L$ denotes the loss.\n2. $R$ denotes the reconstruction term.\n3. $\\lambda$ denotes its weight.";
    let plan = move |u: &str, id: u64| {
        let eqs = json!({ "edits": [
            { "edit": "add_equation", "slide": u, "latex": "R = \\lambda L", "explains": id, "line": 1, "display": true },
            { "edit": "add_equation", "slide": u, "latex": "L = \\lambda R", "explains": id, "line": 2, "display": true },
            { "edit": "set_notes", "slide": u, "text": notes },
        ] });
        // the columns get the ids after the slide's own (2, 3, 4): 內文 1 = 5, 配對方程式 1 = 6, …
        let split = json!({ "edits": [{ "edit": "split_slide", "slide": u, "groups": [[5, 6], [7, 8]], "titles": ["The reconstruction term", "The grading term"], "notes": [notes, notes] }] });
        vec![eqs, split.clone(), split]
    };
    let (t, uid, _) = prepared(plan, &[json!({ "findings": [] })], |u, id, _| vec![json!({ "op": "set_obj", "slide": u, "id": id, "path": "/frame/h", "value": 700_000 })]).await;
    let v = propose(&t, &uid).await;
    apply(&t, &v).await;
    let s = slide_of(&deck(&t).await["deck"], &uid);
    assert_eq!((obj(&s, |o| o["name"] == "內文 1")["id"].as_u64(), obj(&s, |o| o["name"] == "配對方程式 2")["id"].as_u64()), (Some(5), Some(8)));
    let v = propose(&t, &uid).await;
    assert_eq!(v["plan"]["edits"].as_array().unwrap().len(), 1, "{v}");
    assert!(!v["warnings"].to_string().contains("自己畫"), "an equation box is content: {}", v["warnings"]);
    let d = preview(&t, &v).await;
    let order: Vec<String> = d["slides"].as_array().unwrap().iter().map(|s| s["uid"].as_str().unwrap().to_string()).collect();
    let second = slide_of(&d, &order[order.iter().position(|x| *x == uid).unwrap() + 1]);
    let first = slide_of(&d, &uid);
    for (s, l, e) in [(&first, 5, 6), (&second, 7, 8)] {
        let (l, e) = (obj(s, |o| o["id"] == l), obj(s, |o| o["id"] == e));
        assert_eq!((fr(&e, "x"), fr(&e, "y")), (fr(&l, "x"), fr(&l, "y") + fr(&l, "h")), "the equation stays under its line: {} {}", l["frame"], e["frame"]);
    }
    assert_eq!(fr(&obj(&second, |o| o["id"] == 7), "y"), fr(&obj(&first, |o| o["id"] == 5), "y"), "both pages start their text at the top of 內文");
    assert_eq!(fr(&obj(&second, |o| o["id"] == 7), "x"), 487_680);
}

fn caption(id: u32, y: i64) -> Value {
    let mut c = text_obj("圖說", "caption-1", y, "caption", &["The three blocks of the synthetic grader"]);
    c["id"] = json!(id);
    c["text"]["paras"][0]["runs"][0]["look"] = json!({ "font": "Arial", "ea_font": "Noto Sans TC", "size": 12.0, "color": "5D5D64" });
    c["frame"]["h"] = json!(400_000);
    c
}

/// A figure page with a caption under the picture and the lines under that.
fn captioned_page(u: &str, id: u64, dir: &std::path::Path) -> Vec<Value> {
    let mut ops = figure_page(u, id, dir);
    ops.push(json!({ "op": "insert_obj", "slide": u, "index": 99, "obj": caption(81, 3_900_000) }));
    ops.push(json!({ "op": "set_obj", "slide": u, "id": id, "path": "/frame/y", "value": 4_400_000 }));
    ops
}

#[tokio::test]
async fn a_captioned_figure_cropped_keeps_its_caption_under_the_part() {
    let plan = |u: &str, _| vec![json!({ "edits": [{ "edit": "highlight_crop", "slide": u, "picture": 80, "mode": "crop", "box": [0.35, 0.1, 0.65, 0.9] }] })];
    let (t, uid, body) = prepared(plan, &[json!({ "findings": [] })], captioned_page).await;
    let v = propose(&t, &uid).await;
    let s = slide_of(&preview(&t, &v).await, &uid);
    let (p, c, text) = (obj(&s, |o| o["id"] == 80), obj(&s, |o| o["id"] == 81), obj(&s, |o| o["id"] == body));
    assert_eq!((fr(&c, "x"), fr(&c, "w")), (fr(&p, "x"), fr(&p, "w")), "the caption under the part, as wide: {}", c["frame"]);
    assert_eq!(fr(&c, "y"), fr(&p, "y") + fr(&p, "h") + 6_858_000 / 120);
    assert!(fr(&text, "x") >= fr(&p, "x") + fr(&p, "w") && fr(&text, "y") == fr(&p, "y"), "the lines beside: {} {}", text["frame"], p["frame"]);
    apply(&t, &v).await;
    let items = polish_items(&t, &uid).await;
    assert!(items.is_empty(), "{items:?}");
}

#[tokio::test]
async fn split_lays_a_figure_page_out_again() {
    let plan = |u: &str, id: u64| {
        let notes = ["This part shows the figure.\n1. One point.", "This part says what it means.\n1. One point."];
        let p = json!({ "edits": [{ "edit": "split_slide", "slide": u, "groups": [[80, 81], [id]], "titles": ["The figure", "What it means"], "notes": notes }] });
        vec![p.clone(), p]
    };
    let (t, uid, body) = prepared(plan, &[json!({ "findings": [] })], captioned_page).await;
    let v = propose(&t, &uid).await;
    assert_eq!(v["plan"]["edits"].as_array().unwrap().len(), 1, "{v}");
    let d = preview(&t, &v).await;
    let order: Vec<String> = d["slides"].as_array().unwrap().iter().map(|s| s["uid"].as_str().unwrap().to_string()).collect();
    let (first, second) = (slide_of(&d, &uid), slide_of(&d, &order[order.iter().position(|x| *x == uid).unwrap() + 1]));
    let (p, c) = (obj(&first, |o| o["id"] == 80), obj(&first, |o| o["id"] == 81));
    let text = obj(&second, |o| o["id"] == body);
    assert_eq!(fr(&p, "y"), fr(&text, "y"), "both pages start at the top of 內文");
    assert!((fr(&p, "w") as f64 / fr(&p, "h") as f64 - 3.0).abs() < 0.01, "the figure keeps its aspect: {}", p["frame"]);
    assert!(fr(&p, "w") > 7_200_000, "alone on its page the figure takes the room: {}", p["frame"]);
    assert_eq!(fr(&c, "y"), fr(&p, "y") + fr(&p, "h") + 6_858_000 / 120, "the caption right under it");
    assert!(!v["warnings"].to_string().contains("自己畫"), "{}", v["warnings"]);
}
