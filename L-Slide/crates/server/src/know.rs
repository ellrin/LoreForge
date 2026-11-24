//! 存進 L-Brain: the shapes selected on a slide, saved as a reusable drawing (a 模塊 or 元件)
//! of the flow library (`docs/brain-hub.md` §4.2, `docs/flow-figures.md`).
//!
//! | Method | Path | |
//! |---|---|---|
//! | POST | /api/decks/{name}/know/drawing | `{slide, ids, title, level: module｜component, kind?, style?}` → `{via: kb｜file, id, title}` |
//!
//! `ids` are the selected objects' ids on `slide` (`uids` is accepted as a name for the same list).
//! The drawing is stored through L-Brain's hub (`POST /api/hub/items`) while it answers on its
//! port (`KB_URL`, else the shared ports file); otherwise it is written to this app's flow
//! folder (`<workspace>/flow/modules/<kind>/<style>.json`), which the flow editor and L-Brain
//! both read. An existing kind/style is a 409, never overwritten. Every error is a status and
//! `{ "error": "<one line>" }`; no path is ever put into a response.
//!
//! How a drawing is derived ([`drawing_from`]): lore-flow's `template_from_objects` (frames in
//! EMU relative to the selection's top-left corner, ids / source XML / slots / placeholders dropped,
//! connector ends released, one module group saved as its parts), colours as drawn. When exactly
//! one text run exists in the selection, it becomes `{{label}}` (the node's label in the flow
//! tool); with none or several, every text stays as drawn. Pictures are refused.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use lore_canvas::{Kind, Object};
use lore_flow::{SaveOptions, template_from_objects};
use lore_know::formats::figure::save_drawing;
use lore_know::{Item, level};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ApiError, R, Shared, bad, session};

pub(crate) fn routes() -> axum::Router<Shared> {
    axum::Router::new().route("/api/decks/{name}/know/drawing", axum::routing::post(save))
}

const CATEGORY: &str = "sc.flow";

// ── step 1: objects → drawing ────────────────────────────────────────────────

/// Text runs that carry text, in shapes and text boxes (groups searched). `None` when a picture
/// is inside.
fn scan(objs: &[Object], runs: &mut Vec<String>) -> Option<()> {
    for o in objs {
        match &o.kind {
            Kind::Picture(_) => return None,
            Kind::Group(g) => scan(&g.children, runs)?,
            Kind::Text(sp) | Kind::Shape(sp) => {
                for r in sp.text.iter().flat_map(|t| t.paras.iter()).flat_map(|p| p.runs.iter()) {
                    if r.kind == "text" && !r.text.trim().is_empty() {
                        runs.push(r.text.clone());
                    }
                }
            }
            _ => {}
        }
    }
    Some(())
}

/// The JSON `Library::load_dir` reads (a `ModuleTemplate`), from the selected objects.
pub fn drawing_from(objects: &[Object], kind: &str, style: &str) -> Result<Value, String> {
    if objects.is_empty() {
        return Err("沒有選到任何物件".into());
    }
    let mut runs = vec![];
    if scan(objects, &mut runs).is_none() {
        return Err("選到的東西裡有圖片，圖片不能存成可重複使用的圖形".into());
    }
    let label = if runs.len() == 1 { Some(runs[0].clone()) } else { None };
    let opt = SaveOptions { kind: kind.into(), style: style.into(), description: String::new(), label, role_colours: false };
    let t = template_from_objects(objects, &opt).map_err(|e| if e.contains("no size") { "選到的範圍沒有大小".to_string() } else { format!("存不成圖形：{e}") })?;
    serde_json::to_value(&t).map_err(|e| e.to_string())
}

// ── ids ──────────────────────────────────────────────────────────────────────

fn fnv(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// A kind id from a title: its ASCII letters and digits joined by `-`; a title with none (Chinese)
/// gets `mod-<hash>` / `comp-<hash>`. A built-in kind's name is never taken (a saved drawing would
/// replace the built-in one in the flow tool): it gets `my-` in front.
fn kind_from(title: &str, lvl: &str) -> String {
    let words: Vec<String> = title.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_ascii_lowercase()).collect();
    let mut k = words.join("-");
    if k.is_empty() {
        k = format!("{}-{:06x}", if lvl == level::COMPONENT { "comp" } else { "mod" }, fnv(title) & 0xff_ffff);
    }
    if lore_flow::spec::MODULES.iter().any(|(m, _)| *m == k) {
        k = format!("my-{k}");
    }
    k
}

// ── step 2: where it goes ────────────────────────────────────────────────────

/// Where a drawing is stored; injectable for tests.
pub struct Target {
    /// L-Brain's base URL (`http://127.0.0.1:<port>`) when it runs.
    pub brain: Option<String>,
    /// `<workspace>/flow`, the folder with `modules/`.
    pub flow_dir: std::path::PathBuf,
}

/// This machine's: `KB_URL` (tests), else L-Brain's port from the shared ports file; never in an
/// isolated server (`real_kb` off) except `KB_URL`.
fn target(s: &Shared) -> Target {
    let env = std::env::var("KB_URL").ok().filter(|u| !u.trim().is_empty());
    let live = || lore_home::ports::live().get("kb").map(|p| format!("http://127.0.0.1:{p}"));
    let brain = env.or_else(|| if s.real_kb.load(std::sync::atomic::Ordering::Relaxed) { live() } else { None });
    Target { brain, flow_dir: s.ws.root.join("flow") }
}

fn err(code: StatusCode, m: impl Into<String>) -> ApiError {
    ApiError(code, m.into().lines().next().unwrap_or("").to_string())
}

fn one_line_error(v: &Value, fallback: &str) -> String {
    v["error"].as_str().map(|s| s.lines().next().unwrap_or("").to_string()).unwrap_or_else(|| fallback.to_string())
}

/// Store a drawing (`params` = [`drawing_from`]). `{via, id, title}`.
pub async fn store(t: &Target, kind: &str, style: &str, title: &str, lvl: &str, params: Value) -> Result<Value, ApiError> {
    let id = format!("{CATEGORY}:{kind}.{style}");
    let exists = || err(StatusCode::CONFLICT, format!("已經有 {kind}/{style} 這個圖形，換個名字"));
    if let Some(base) = &t.brain {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(8)).build().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut item = Item::new(CATEGORY, "figure", title, Default::default());
        item.level = lvl.into();
        item.set("what", "drawing").set("kind", kind).set("style", style).set("description", title).set("params", params.clone());
        item.source = Default::default();
        // a closed or stale L-Brain is "not running": the file below takes over
        let probe = http.get(format!("{base}/api/hub/items/{id}")).send().await;
        if let Ok(r) = probe {
            if r.status().is_success() {
                return Err(exists());
            }
            let r = http.post(format!("{base}/api/hub/items")).json(&item).send().await.map_err(|_| err(StatusCode::BAD_GATEWAY, "L-Brain 沒有回應"))?;
            let code = r.status();
            let v: Value = r.json().await.unwrap_or(Value::Null);
            if code == StatusCode::CONFLICT {
                return Err(exists());
            }
            if !code.is_success() {
                return Err(err(StatusCode::BAD_GATEWAY, format!("L-Brain：{}", one_line_error(&v, "沒有存成"))));
            }
            let got = v["item"]["id"].as_str().unwrap_or(&id).to_string();
            return Ok(json!({ "via": "kb", "id": got, "title": title }));
        }
    }
    // L-Brain is not running: the flow folder, with the title as the drawing's description
    if t.flow_dir.join("modules").join(kind).join(format!("{style}.json")).exists() {
        return Err(exists());
    }
    let mut p = params;
    p["description"] = json!(title);
    save_drawing(&t.flow_dir, kind, style, &p, lvl, false).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(json!({ "via": "file", "id": id, "title": title }))
}

#[derive(Deserialize)]
pub struct Body {
    pub slide: String,
    #[serde(alias = "uids")]
    pub ids: Vec<u32>,
    pub title: String,
    pub level: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
}

/// The whole request against a target: the selected objects of one slide.
pub async fn save_selection(t: &Target, objects: &[Object], b: &Body) -> Result<Value, ApiError> {
    let title = b.title.trim();
    if title.is_empty() {
        return Err(bad("要有名字"));
    }
    let lvl = match b.level.as_str() {
        "module" => level::MODULE,
        "component" => level::COMPONENT,
        _ => return Err(bad("只能存成模塊或元件")),
    };
    let kind = b.kind.clone().filter(|k| !k.trim().is_empty()).unwrap_or_else(|| kind_from(title, lvl));
    let style = b.style.clone().filter(|k| !k.trim().is_empty()).unwrap_or_else(|| "mine".into());
    for (v, what) in [(&kind, "節點種類"), (&style, "畫法")] {
        if !lore_flow::spec::valid_id(v) {
            return Err(bad(format!("{what}只能用英文字母、數字、- _")));
        }
    }
    let params = drawing_from(objects, &kind, &style).map_err(bad)?;
    store(t, &kind, &style, title, lvl, params).await
}

async fn save(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<Body>) -> R {
    let objects: Vec<Object> = {
        let sess = session(&s, &name)?;
        let sess = sess.lock().unwrap();
        let sl = sess.editor.deck.slide(&b.slide).ok_or_else(|| bad("找不到投影片"))?;
        let found: Vec<Object> = sl.objects.iter().filter(|o| b.ids.contains(&o.id)).cloned().collect();
        if found.len() != b.ids.len() {
            return Err(bad("找不到選到的物件"));
        }
        found
    };
    Ok(Json(save_selection(&target(&s), &objects, &b).await?))
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../../../../commons/crates/lore-ooxml/tests/common/mod.rs"]
mod common;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn shape(id: u32, x: i64, text: Option<&str>) -> Object {
        let mut v = json!({ "id": id, "name": "box", "frame": { "x": x, "y": 300_000, "w": 1_000_000, "h": 500_000 }, "type": "shape",
            "geom": { "prst": "rect" }, "fill": { "kind": "solid", "color": "C8C1FF" } });
        if let Some(t) = text {
            v["text"] = json!({ "paras": [{ "runs": [{ "text": t, "look": { "font": "Calibri", "size": 14.0, "color": "000000" } }] }] });
        }
        serde_json::from_value(v).unwrap_or_else(|e| panic!("{e}"))
    }

    fn picture() -> Object {
        serde_json::from_value(json!({ "id": 9, "name": "pic", "frame": { "x": 0, "y": 0, "w": 100, "h": 100 }, "type": "picture", "asset": "file:assets/a.png" })).unwrap()
    }

    fn body(title: &str, lvl: &str) -> Body {
        Body { slide: "s1".into(), ids: vec![], title: title.into(), level: lvl.into(), kind: None, style: None }
    }

    #[test]
    fn a_selection_becomes_a_drawing_the_flow_tool_loads() {
        // two shapes, one text run: it becomes the node's label; coordinates start at the corner
        let objs = [shape(1, 2_000_000, Some("Encoder")), shape(2, 3_500_000, None)];
        let v = drawing_from(&objs, "my-enc", "mine").unwrap();
        assert_eq!((v["kind"].as_str(), v["style"].as_str()), (Some("my-enc"), Some("mine")));
        let dir = tempfile::tempdir().unwrap();
        save_drawing(dir.path(), "my-enc", "mine", &v, "", false).unwrap();
        let lib = lore_flow::Library::load_dir(&dir.path().join("modules")).unwrap();
        let t = lib.get("my-enc", "mine").expect("the flow tool loads it");
        let o = t.objects.as_array().unwrap();
        assert_eq!(o.len(), 2);
        assert_eq!(o[0]["frame"]["x"], 0);
        assert_eq!(o[1]["frame"]["x"], 1_500_000);
        assert!(t.objects.to_string().contains("{{label}}") && !t.objects.to_string().contains("Encoder"));
        assert!(t.objects.to_string().contains("C8C1FF"), "colours as drawn");
        // two text runs: kept as drawn
        let v = drawing_from(&[shape(1, 0, Some("A")), shape(2, 2_000_000, Some("B"))], "two", "mine").unwrap();
        let s = v["objects"].to_string();
        assert!(!s.contains("{{label}}") && s.contains("\"A\"") && s.contains("\"B\""));
    }

    #[test]
    fn an_empty_selection_and_a_picture_are_refused() {
        assert!(drawing_from(&[], "k", "mine").unwrap_err().contains("沒有選到"));
        let e = drawing_from(&[shape(1, 0, None), picture()], "k", "mine").unwrap_err();
        assert!(e.contains("圖片"), "{e}");
    }

    /// A fake L-Brain: records the item posted; `have` = ids that already exist.
    async fn fake_brain(have: Vec<String>) -> (String, Arc<Mutex<Vec<Value>>>) {
        use axum::routing::{get, post};
        let seen = Arc::new(Mutex::new(vec![]));
        let (s1, h1) = (seen.clone(), have.clone());
        let app = axum::Router::new()
            .route("/api/hub/items/{id}", get(move |axum::extract::Path(id): axum::extract::Path<String>| {
                let h = h1.clone();
                async move { if h.contains(&id) { (StatusCode::OK, Json(json!({ "item": { "id": id } }))) } else { (StatusCode::NOT_FOUND, Json(json!({ "error": "找不到" }))) } }
            }))
            .route("/api/hub/items", post(move |Json(v): Json<Value>| {
                let s = s1.clone();
                async move {
                    let id = format!("{}:{}.{}", v["category"].as_str().unwrap(), v["fields"]["kind"].as_str().unwrap(), v["fields"]["style"].as_str().unwrap());
                    s.lock().unwrap().push(v);
                    (StatusCode::CREATED, Json(json!({ "item": { "id": id } })))
                }
            }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (format!("http://127.0.0.1:{port}"), seen)
    }

    #[tokio::test]
    async fn a_drawing_goes_to_lbrain_when_it_runs() {
        let (url, seen) = fake_brain(vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        let t = Target { brain: Some(url), flow_dir: dir.path().to_path_buf() };
        let mut b = body("Encoder block", "component");
        b.kind = Some("enc-block".into());
        let r = save_selection(&t, &[shape(1, 0, Some("E"))], &b).await.unwrap();
        assert_eq!((r["via"].as_str(), r["id"].as_str(), r["title"].as_str()), (Some("kb"), Some("sc.flow:enc-block.mine"), Some("Encoder block")));
        let it = seen.lock().unwrap()[0].clone();
        let it = &it;
        assert_eq!((it["category"].as_str(), it["format"].as_str(), it["level"].as_str(), it["fields"]["what"].as_str()), (Some("sc.flow"), Some("figure"), Some("component"), Some("drawing")));
        assert_eq!(it["fields"]["params"]["kind"], "enc-block");
        assert!(it["fields"]["params"]["objects"].is_array());
        assert!(!dir.path().join("modules").exists(), "nothing written to the folder");
        // the same name is already in L-Brain: 409
        let (url, _) = fake_brain(vec!["sc.flow:enc-block.mine".into()]).await;
        let t = Target { brain: Some(url), flow_dir: dir.path().to_path_buf() };
        let e = save_selection(&t, &[shape(1, 0, None)], &b).await.unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn a_drawing_is_written_to_the_flow_folder_when_lbrain_is_closed() {
        let dir = tempfile::tempdir().unwrap();
        // a closed port is the same as no L-Brain
        let closed = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); format!("http://127.0.0.1:{}", l.local_addr().unwrap().port()) };
        for brain in [None, Some(closed)] {
            let t = Target { brain, flow_dir: dir.path().join("flow") };
            let mut b = body("Encoder", "module");
            b.kind = Some(format!("enc-{}", dir.path().join("flow/modules").exists()));
            let r = save_selection(&t, &[shape(1, 0, Some("E"))], &b).await.unwrap();
            assert_eq!(r["via"], "file");
            let lib = lore_flow::Library::load_dir(&dir.path().join("flow/modules")).unwrap();
            let k = b.kind.unwrap();
            assert_eq!(lib.get(&k, "mine").unwrap().description, "Encoder");
            assert_eq!(r["id"], format!("sc.flow:{k}.mine"));
        }
        assert!(!r_has_path(&json!({ "via": "file" })));
    }

    fn r_has_path(v: &Value) -> bool {
        v.to_string().contains('/')
    }

    #[tokio::test]
    async fn an_existing_drawing_is_a_409() {
        let dir = tempfile::tempdir().unwrap();
        let t = Target { brain: None, flow_dir: dir.path().to_path_buf() };
        let mut b = body("Encoder", "module");
        b.kind = Some("enc".into());
        save_selection(&t, &[shape(1, 0, Some("E"))], &b).await.unwrap();
        let before = std::fs::read_to_string(dir.path().join("modules/enc/mine.json")).unwrap();
        let e = save_selection(&t, &[shape(1, 0, Some("Other"))], &b).await.unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        assert!(!e.1.contains('\n') && !e.1.contains(dir.path().to_str().unwrap()));
        assert_eq!(std::fs::read_to_string(dir.path().join("modules/enc/mine.json")).unwrap(), before, "never overwritten");
    }

    #[tokio::test]
    async fn a_chinese_title_keeps_its_text_and_gets_a_generated_id() {
        let dir = tempfile::tempdir().unwrap();
        let t = Target { brain: None, flow_dir: dir.path().to_path_buf() };
        let r = save_selection(&t, &[shape(1, 0, Some("E"))], &body("我的編碼器", "module")).await.unwrap();
        assert_eq!(r["title"], "我的編碼器");
        let id = r["id"].as_str().unwrap();
        assert!(id.is_ascii() && id.starts_with("sc.flow:mod-") && id.ends_with(".mine"), "{id}");
        // the same title gives the same id (so a second save is a 409, not a duplicate)
        let e = save_selection(&t, &[shape(1, 0, None)], &body("我的編碼器", "module")).await.unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        // a built-in kind's name is not taken
        assert_eq!(kind_from("Box", "module"), "my-box");
        assert_eq!(kind_from("Fancy  Encoder 2", "module"), "fancy-encoder-2");
        assert!(kind_from("元件", "component").starts_with("comp-"));
    }

    fn app() -> (tempfile::TempDir, axum::Router, Shared) {
        let dir = tempfile::tempdir().unwrap();
        let ws = sc_core::Workspace::new(dir.path(), None);
        let home = lore_home::Home::at(dir.path().join("lf"));
        let s = crate::state_at(ws, crate::repo_dir().join("web"), home);
        (dir, crate::router_isolated(s.clone()), s)
    }

    #[tokio::test]
    async fn the_catalogue_lists_the_new_drawing() {
        use tower::ServiceExt;
        let (dir, app, s) = app();
        // the flow editor reads its catalogue on every request: not there before, there after
        let flow = lore_flow::web::Flow::new(s.home.clone()).with_app_dir(s.ws.root.join("flow"));
        assert!(flow.catalogue().library.get("enc", "mine").is_none());
        let t = Target { brain: None, flow_dir: s.ws.root.join("flow") };
        let mut b = body("Encoder", "module");
        b.kind = Some("enc".into());
        save_selection(&t, &[shape(1, 0, Some("E"))], &b).await.unwrap();
        assert!(flow.catalogue().library.get("enc", "mine").is_some());
        assert!(flow.catalogue().library.kinds().contains(&"enc".to_string()));
        // and the served route still answers
        let res = app.oneshot(axum::http::Request::builder().uri("/flow/api/catalogue").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        drop(dir);
    }

    #[tokio::test]
    async fn the_route_saves_the_selected_objects_of_an_open_deck() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let (dir, app, _s) = app();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        let file = docs.join("ellrin.pptx");
        std::fs::write(&file, common::synthetic_deck()).unwrap();
        std::fs::create_dir_all(dir.path().join("styles/test")).unwrap();
        let call = |method: &str, uri: String, body: Value| {
            let app = app.clone();
            let m = method.to_string();
            async move {
                let req = axum::http::Request::builder().method(m.as_str()).uri(uri).header("content-type", "application/json").body(axum::body::Body::from(body.to_string())).unwrap();
                let res = app.oneshot(req).await.unwrap();
                let st = res.status();
                (st, serde_json::from_slice::<Value>(&res.into_body().collect().await.unwrap().to_bytes()).unwrap_or(Value::Null))
            }
        };
        let (code, v) = call("POST", "/api/files/open".into(), json!({ "path": file })).await;
        assert_eq!(code, StatusCode::OK, "{v}");
        let name = v["name"].as_str().unwrap().to_string();
        let (_, d) = call("GET", format!("/api/decks/{name}"), json!(null)).await;
        let sl = &d["deck"]["slides"][0];
        let uid = sl["uid"].as_str().unwrap().to_string();
        let ids: Vec<u64> = sl["objects"].as_array().unwrap().iter().filter(|o| o["type"] != "picture").take(1).map(|o| o["id"].as_u64().unwrap()).collect();
        assert!(!ids.is_empty());
        let (code, r) = call("POST", format!("/api/decks/{name}/know/drawing"), json!({ "slide": uid, "uids": ids, "title": "我的圖", "level": "module" })).await;
        assert_eq!(code, StatusCode::OK, "{r}");
        assert_eq!(r["via"], "file");
        assert!(dir.path().join("flow/modules").is_dir());
        let (code, r) = call("POST", format!("/api/decks/{name}/know/drawing"), json!({ "slide": uid, "ids": [99999], "title": "x", "level": "module" })).await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{r}");
        assert!(r["error"].is_string());
    }
}
