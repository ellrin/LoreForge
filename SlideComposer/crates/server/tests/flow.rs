//! 流程圖 on a slide (lore-flow's shared editor), offline: a synthetic two-stage pipeline,
//! no model, no browser.

#[path = "../../../../commons/crates/lore-ooxml/tests/common/mod.rs"]
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

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
[ghost]
picture_alpha = 0.2
line_alpha = 0.2
text_color = "A6A6A6"
"#;

const SEG2: &str = "fig seg2\npanel a \"a. Coarse localisation\"\n  x: image \"$\\mathbf{x}$\"\n  det: encoder \"Localiser\"\n  roi: box \"ROI crop\"\n  x -> det -> roi\nend\npanel b \"b. Fine segmentation\"\n  crop: image \"@xr\"\n  seg: unet \"Segmenter\"\n  yhat: image \"$\\hat{\\mathbf{y}}$\" look=mask\n  loss: loss \"$\\mathcal{L}_{seg}$\"\n  y: image \"$\\mathbf{y}$\" role=gt look=mask\n  crop -> seg -> yhat -> loss\n  y -> loss : supervision\n  loss -> seg : feedback\nend\nroi -> crop : cross\nvar xr = \"\\mathbf{x}_{r}\" \"cropped region\"\n";

fn setup() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("styles/test")).unwrap();
    std::fs::write(dir.path().join("styles/test/style.toml"), STYLE).unwrap();
    let ws = sc_core::Workspace::new(dir.path(), None);
    let web = sc_server::repo_dir().join("web");
    // a LoreForge folder of its own: nothing of the author's is read
    let home = lore_home::Home::at(dir.path().join("lf"));
    let app = sc_server::router_isolated(sc_server::state_at(ws, web, home));
    (dir, app)
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value, Vec<u8>) {
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
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null), bytes)
}

fn group(d: &Value, slide: usize) -> Option<Value> {
    d["deck"]["slides"][slide]["objects"].as_array().unwrap().iter().find(|o| o["name"].as_str().is_some_and(|n| n.starts_with("flow:"))).cloned()
}

fn sizes(o: &Value, out: &mut Vec<f64>) {
    if let Some(s) = o["look"]["size"].as_f64() {
        out.push(s);
    }
    if let Some(w) = o["width_pt"].as_f64() {
        out.push(w);
    }
    match o {
        Value::Object(m) => m.values().for_each(|v| sizes(v, out)),
        Value::Array(a) => a.iter().for_each(|v| sizes(v, out)),
        _ => {}
    }
}

#[tokio::test]
async fn a_flow_figure_is_one_picture_that_reopens_in_its_editor() {
    let (dir, app) = setup();
    let docs = dir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let file = docs.join("ellrin.pptx");
    std::fs::write(&file, common::synthetic_deck()).unwrap();
    let (code, v, _) = call(&app, "POST", "/api/files/open", Some(json!({ "path": file }))).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let name = v["name"].as_str().unwrap().to_string();
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None).await;
    let s2 = d["deck"]["slides"][1]["uid"].as_str().unwrap().to_string();

    // insert: the server draws, the page commits the ops (one undo step)
    let spec = json!({ "lore_flow": 1, "dsl": SEG2, "layout": "small-big", "frame": [600, 300],
        "types": [{ "id": "conv", "label": "conv block", "fill": "F9D5E5", "line": "B03A6E" }], "blocks": { "b": "conv" },
        "legend": { "show": "on", "rows": [{ "key": "type:conv", "label": "conv block" }, { "key": "line:supervision", "label": "ground truth" }] },
        "discussion": { "source": "synthetic", "answers": { "focus": "process" } } });
    let (code, r, _) = call(&app, "POST", &format!("/api/decks/{name}/flow"), Some(json!({ "slide": s2, "spec": spec }))).await;
    assert_eq!(code, StatusCode::OK, "{r}");
    let (code, v, _) = call(&app, "POST", &format!("/api/decks/{name}/ops"), Some(json!({ "ops": r["ops"], "label": "插入流程圖" }))).await;
    assert_eq!(code, StatusCode::OK, "off-palette figure colours are allowed inside one picture: {v}");
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None).await;
    let g = group(&d, 1).expect("the figure");
    assert_eq!(g["id"], r["id"]);
    assert_eq!(g["type"], "group");
    assert_eq!(g["lock_aspect"], true);
    assert_eq!(g["data"]["layout"], "small-big");
    assert_eq!(g["data"]["discussion"]["answers"]["focus"], "process");
    let w = d["deck"]["size"]["w"].as_i64().unwrap();
    assert!(g["frame"]["w"].as_i64().unwrap() <= w * 9 / 10 + 1);
    // the figure's new variable joins the deck's registry
    let (_, n, _) = call(&app, "GET", &format!("/api/decks/{name}/notation"), None).await;
    assert!(n["notation"]["entries"].as_array().unwrap().iter().any(|e| e["id"] == "xr"), "{n}");
    // one undo removes the whole figure
    call(&app, "POST", &format!("/api/decks/{name}/undo"), None).await;
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None).await;
    assert!(group(&d, 1).is_none());
    let (code, v, _) = call(&app, "POST", &format!("/api/decks/{name}/redo"), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None).await;
    let g0 = group(&d, 1).unwrap_or_else(|| panic!("{:?} / {v}", d["deck"]["slides"][1]["objects"].as_array().unwrap().iter().map(|o| o["name"].clone()).collect::<Vec<_>>()));

    // resized as one picture: text sizes and line widths scale with it
    let id = g0["id"].as_u64().unwrap();
    let (x, y, w0) = (g0["frame"]["x"].as_i64().unwrap(), g0["frame"]["y"].as_i64().unwrap(), g0["frame"]["w"].as_i64().unwrap());
    let (code, v, _) = call(&app, "POST", &format!("/api/decks/{name}/ops"), Some(json!({ "ops": [{ "op": "scale", "slide": s2, "id": id, "x": x, "y": y, "w": w0 / 2 }], "label": "縮放" }))).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None).await;
    let g1 = group(&d, 1).unwrap();
    let (mut a, mut b) = (vec![], vec![]);
    sizes(&g0, &mut a);
    sizes(&g1, &mut b);
    assert!(!a.is_empty() && a.len() == b.len());
    assert!(a.iter().zip(&b).all(|(p, q)| (q - p / 2.0).abs() <= 0.06 + p * 0.002), "{a:?} → {b:?}");
    let ar = |g: &Value| g["frame"]["w"].as_f64().unwrap() / g["frame"]["h"].as_f64().unwrap();
    assert!((ar(&g0) - ar(&g1)).abs() < 0.01);

    // re-edit: «block 2 改成直的» → the same place and width, the new spec; one undo step back
    let mut spec2 = g1["data"].clone();
    spec2["dirs"] = json!({ "b": "down" });
    let (code, r, _) = call(&app, "POST", &format!("/api/decks/{name}/flow"), Some(json!({ "slide": s2, "spec": spec2, "replace": id }))).await;
    assert_eq!(code, StatusCode::OK, "{r}");
    call(&app, "POST", &format!("/api/decks/{name}/ops"), Some(json!({ "ops": r["ops"], "label": "更新流程圖" }))).await;
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None).await;
    let g2 = group(&d, 1).unwrap();
    assert_eq!((g2["frame"]["x"].clone(), g2["frame"]["y"].clone(), g2["frame"]["w"].clone()), (g1["frame"]["x"].clone(), g1["frame"]["y"].clone(), g1["frame"]["w"].clone()));
    assert_eq!(g2["data"]["dirs"]["b"], "down");
    let figures = d["deck"]["slides"][1]["objects"].as_array().unwrap().iter().filter(|o| o["name"].as_str().is_some_and(|n| n.starts_with("flow:"))).count();
    assert_eq!(figures, 1, "replaced, not added");
    call(&app, "POST", &format!("/api/decks/{name}/undo"), None).await;
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None).await;
    assert!(group(&d, 1).unwrap()["data"]["dirs"].is_null());
    call(&app, "POST", &format!("/api/decks/{name}/redo"), None).await;

    // saved into the .pptx and read back from it: still one picture, still re-opens
    let (code, v, _) = call(&app, "POST", &format!("/api/decks/{name}/file/write"), Some(json!({}))).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let back = lore_ooxml::Deck::from_bytes(&std::fs::read(&file).unwrap()).unwrap();
    let cd = lore_canvas::pptx_in::import(&back, None, None);
    let g = cd.slides[1].objects.iter().find(|o| o.name.starts_with("flow:")).expect("in the file");
    assert!(matches!(&g.kind, lore_canvas::Kind::Group(x) if x.lock_aspect));
    std::fs::remove_dir_all(dir.path().join("decks")).unwrap();
    let (code, v, _) = call(&app, "POST", "/api/files/open", Some(json!({ "path": file }))).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let name2 = v["name"].as_str().unwrap().to_string();
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name2}"), None).await;
    let g = group(&d, 1).expect("the figure after a rebuild from the file");
    assert_eq!(g["data"]["dirs"]["b"], "down");
    assert_eq!(g["lock_aspect"], true);

    // 另存圖片: the format is checked before any browser runs
    let (code, _, _) = call(&app, "POST", &format!("/api/decks/{name2}/image"), Some(json!({ "slide": s2, "id": g["id"], "format": "gif" }))).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    // a spec that does not parse never reaches the slide
    let (code, _, _) = call(&app, "POST", &format!("/api/decks/{name2}/flow"), Some(json!({ "slide": s2, "spec": { "lore_flow": 1, "dsl": "x -> " } }))).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_shared_editor_is_mounted() {
    let (_dir, app) = setup();
    let (st, _, page) = call(&app, "GET", "/flow/", None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(String::from_utf8_lossy(&page).contains("LoreFlow.edit"));
    let (st, _, js) = call(&app, "GET", "/flow/editor.js", None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(String::from_utf8_lossy(&js).contains("LF.edit"));
    let (st, v, _) = call(&app, "GET", "/flow/api/catalogue", None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["layouts"].as_array().unwrap().iter().any(|l| l["id"] == "two-left-big-right"));
    assert_eq!(v["steps"][0]["id"], "start");
    let (st, v, _) = call(&app, "POST", "/flow/api/preview", Some(json!({ "spec": { "lore_flow": 1, "dsl": SEG2 }, "doc": "nope" }))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["linear"][0], "block 1 (a. Coarse localisation): x -> det -> roi");
    // the tasks are in the 模型 tab
    let t = sc_server::llm::app_tasks();
    assert!(t.task.contains_key("flow-discuss") && t.task.contains_key("flow-assist"));
}
