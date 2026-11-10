//! API tests on a synthetic deck (lore-ooxml's test fixture), offline.

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
red = "FF0000"
[emphasis.key]
color = "FF0000"
bold = true
[ghost]
picture_alpha = 0.2
line_alpha = 0.2
text_color = "A6A6A6"
"#;

fn setup() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("styles/test")).unwrap();
    std::fs::write(dir.path().join("styles/test/style.toml"), STYLE).unwrap();
    let ws = sc_core::Workspace::new(dir.path(), None);
    let web = sc_server::repo_dir().join("web");
    let app = sc_server::router(sc_server::state(ws, web));
    (dir, app)
}

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
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (st, v, bytes)
}

#[tokio::test]
async fn import_edit_undo_reveal_export() {
    let (dir, app) = setup();
    let (st, v, _) = call(&app, "POST", "/api/decks/import?name=demo&style=test", None, Some(common::synthetic_deck())).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "demo");
    let (_, d, _) = call(&app, "GET", "/api/decks/demo", None, None).await;
    assert_eq!(d["deck"]["slides"].as_array().unwrap().len(), 2);
    let s2 = d["deck"]["slides"][1]["uid"].as_str().unwrap().to_string();

    // unedited export keeps every part
    let (_, _, bytes) = call(&app, "GET", "/api/decks/demo/pptx", None, None).await;
    let back = lore_ooxml::Deck::from_bytes(&bytes).unwrap();
    let orig = lore_ooxml::Deck::from_bytes(&common::synthetic_deck()).unwrap();
    for p in &orig.pkg.parts {
        assert!(back.pkg.get(&p.name) == Some(p.data.as_slice()), "{}", p.name);
    }

    // rule: unknown tag rejected (422), nothing applied
    let (st, v, _) = call(&app, "POST", "/api/decks/demo/ops", Some(json!({"ops": [{"op": "set_obj", "slide": s2, "id": 3, "path": "/text/paras/0/runs/0/tag", "value": "giant"}], "label": "x"})), None).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    // move the picture, then undo / redo
    let pic = d["deck"]["slides"][1]["objects"].as_array().unwrap().iter().find(|o| o["id"] == 5).unwrap().clone();
    let mut f = pic["frame"].clone();
    f["x"] = json!(f["x"].as_i64().unwrap() + 100000);
    let (st, v, _) = call(&app, "POST", "/api/decks/demo/ops", Some(json!({"ops": [{"op": "set_obj", "slide": s2, "id": 5, "path": "/frame", "value": f}], "label": "移動"})), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"]["undo"], "移動");
    let (_, v, _) = call(&app, "POST", "/api/decks/demo/undo", Some(json!({})), None).await;
    assert_eq!(v["status"]["redo"], "移動");
    let (_, v, _) = call(&app, "POST", "/api/decks/demo/redo", Some(json!({})), None).await;
    let moved = v["slides"][0]["objects"].as_array().unwrap().iter().find(|o| o["id"] == 5).unwrap()["frame"]["x"].clone();
    assert_eq!(moved, f["x"]);

    // progressive reveal: 2 steps -> 3 slides
    let (st, v, _) = call(&app, "POST", "/api/decks/demo/reveal", Some(json!({"slide": s2, "spec": {"mode": "ghost", "steps": [[3], [5]]}})), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["order"].as_array().unwrap().len(), 3);

    // picture asset is served from the source package
    let (st, _, b) = call(&app, "GET", "/api/decks/demo/asset?id=pkg:ppt/media/image2.png", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(b, b"\x89PNG-figure");

    // export writes a new file, never overwriting
    let (_, a, _) = call(&app, "POST", "/api/decks/demo/export", Some(json!({})), None).await;
    let (_, b2, _) = call(&app, "POST", "/api/decks/demo/export", Some(json!({})), None).await;
    assert_ne!(a["path"], b2["path"]);
    let out = lore_ooxml::Deck::open(a["path"].as_str().unwrap()).unwrap();
    assert_eq!(out.slides.len(), 3);

    // deck.json on disk is the source of truth: a fresh server sees the edits
    let ws = sc_core::Workspace::new(dir.path(), None);
    let sess = ws.open("demo").unwrap();
    assert_eq!(sess.deck().slides.len(), 3);
}

#[tokio::test]
async fn new_deck_needs_a_template_and_names_are_checked() {
    let (_dir, app) = setup();
    let (st, _, _) = call(&app, "POST", "/api/decks", Some(json!({"name": "../evil", "style": "test"})), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v, _) = call(&app, "GET", "/api/info", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["styles"][0], "test");
}

#[tokio::test]
async fn start_page_lists_styles_with_their_previews() {
    let (dir, app) = setup();
    let preview = dir.path().join("styles/test/preview");
    std::fs::create_dir_all(&preview).unwrap();
    std::fs::write(preview.join("slide-1.png"), b"\x89PNG one").unwrap();
    std::fs::write(preview.join("slide-2.png"), b"\x89PNG two").unwrap();
    std::fs::write(preview.join("notes.txt"), b"not served").unwrap();

    let (_, v, _) = call(&app, "GET", "/api/info", None, None).await;
    assert_eq!(v["style_list"], json!([{ "id": "test", "name": "Test", "previews": 2 }]));

    let (st, _, bytes) = call(&app, "GET", "/api/styles/test/preview/2", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(bytes, b"\x89PNG two");
    let (st, _, _) = call(&app, "GET", "/api/styles/test/preview/3", None, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // only slide-<number>.png, only inside a known style
    let (st, _, _) = call(&app, "GET", "/api/styles/test/preview/notes.txt", None, None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _, _) = call(&app, "GET", "/api/styles/..%2Fstyles%2Ftest/preview/1", None, None).await;
    assert!(st.is_client_error(), "{st}");
}

const BASE: &str = r#"{
  "id": "test", "name": "封面＋內頁",
  "size": { "w": 9144000, "h": 5143500 },
  "pages": {
    "cover": { "layout": "封面", "title": { "x": 457200, "y": 1828800, "w": 8229600, "h": 914400 },
      "starter": [
        { "id": 2, "name": "photo", "frame": { "x": 4572000, "y": 0, "w": 4572000, "h": 5143500 }, "type": "picture",
          "asset": "style:cover.png", "crop": { "l": 0, "t": 0, "r": 0, "b": 0 }, "alpha": 1.0, "clip": "rect" },
        { "id": 3, "name": "block", "frame": { "x": 0, "y": 0, "w": 5029200, "h": 5143500 }, "type": "shape",
          "geom": { "prst": "rect" }, "fill": { "kind": "solid", "color": "@black" } } ] },
    "content": { "layout": "內頁", "title": { "x": 457200, "y": 228600, "w": 8229600, "h": 685800 },
      "fixed": [ { "id": 2, "name": "bar", "frame": { "x": 0, "y": 5029200, "w": 9144000, "h": 114300 }, "type": "shape",
        "geom": { "prst": "rect" }, "fill": { "kind": "gradient", "angle": 0, "stops": [ { "pos": 0, "color": "@black" }, { "pos": 1, "color": "@red" } ] } } ] }
  }
}"#;

#[tokio::test]
async fn a_canvas_base_makes_new_decks_and_previews() {
    let (dir, app) = setup();
    let st = dir.path().join("styles/test");
    std::fs::write(st.join("style.toml"), STYLE.replace("slide_cy = 5143500", "slide_cy = 5143500\n[layouts]\ncover = \"封面\"\ncontent = \"內頁\"")).unwrap();
    std::fs::write(st.join("base.json"), BASE).unwrap();
    std::fs::write(st.join("cover.png"), b"\x89PNG cover").unwrap();

    let (code, v, _) = call(&app, "POST", "/api/decks", Some(json!({"name": "ellrin-talk", "style": "test"})), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (_, d, _) = call(&app, "GET", "/api/decks/ellrin-talk", None, None).await;
    let slides = d["deck"]["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 2);
    assert_eq!(slides[0]["layout"], "封面");
    let cover: Vec<&str> = slides[0]["objects"].as_array().unwrap().iter().map(|o| o["name"].as_str().unwrap()).collect();
    assert_eq!(cover, ["photo", "block", "Title"], "starters under the title placeholder");
    let photo = &slides[0]["objects"][0];
    let asset = photo["asset"].as_str().unwrap();
    assert!(asset.starts_with("file:assets/"), "{asset}");
    assert_eq!(slides[0]["objects"][1]["fill"]["color"], "000000", "role resolved");
    // the layout's fixed bar is a backdrop of the content page, not a slide object
    let bd = d["backdrops"].as_array().unwrap().iter().find(|b| b["layout"] == "內頁").unwrap();
    assert_eq!(bd["objects"][0]["fill"]["kind"], "gradient");
    let (_, _, bytes) = call(&app, "GET", &format!("/api/decks/ellrin-talk/asset?id={}", asset), None, None).await;
    assert_eq!(bytes, b"\x89PNG cover");

    // a new content slide starts with the title only; exporting keeps the gradient bar in the layout
    let (code, v, _) = call(&app, "POST", "/api/decks/ellrin-talk/new-slide", Some(json!({})), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (_, _, pptx) = call(&app, "GET", "/api/decks/ellrin-talk/pptx", None, None).await;
    let od = lore_ooxml::Deck::from_bytes(&pptx).unwrap();
    assert_eq!(od.slides.len(), 3);
    let layout = od.layout_by_name("內頁").unwrap();
    let xml = String::from_utf8(od.pkg.get(&layout.part).unwrap().to_vec()).unwrap();
    assert!(xml.contains("a:gradFill") && xml.contains("FF0000"), "{xml}");

    // previews: the cover page as a small deck, its pictures served from the style folder
    let (_, smp, _) = call(&app, "GET", "/api/styles/test/sample", None, None).await;
    assert_eq!(smp["slides"].as_array().unwrap().len(), 2);
    assert_eq!(smp["slides"][0]["objects"][0]["asset"], "style:cover.png");
    let (code, _, bytes) = call(&app, "GET", "/api/styles/test/asset?id=style:cover.png", None, None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(bytes, b"\x89PNG cover");
    let (code, _, _) = call(&app, "GET", "/api/styles/test/asset?id=style:../style.toml", None, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND);

    // a colour role the palette lacks is an error, not black
    std::fs::write(st.join("base.json"), BASE.replace("@red", "@rd")).unwrap();
    let (code, v, _) = call(&app, "POST", "/api/decks", Some(json!({"name": "daidai", "style": "test"})), None).await;
    assert!(code.is_client_error(), "{code} {v}");
}

#[tokio::test]
async fn arranged_slides_keep_empty_slots_out_of_exports() {
    let (dir, app) = setup();
    let (_, v, _) = call(&app, "POST", "/api/decks/import?name=demo&style=test", None, Some(common::synthetic_deck())).await;
    assert_eq!(v["name"], "demo");
    let (_, d, _) = call(&app, "GET", "/api/decks/demo", None, None).await;
    let after = d["deck"]["slides"][0]["uid"].as_str().unwrap().to_string();
    let slot = |name: &str, x: i64, text: &str| json!({ "id": 0, "name": name, "slot": name, "frame": { "x": x, "y": 1000000, "w": 3000000, "h": 2000000 },
        "type": "text", "geom": { "prst": "rect" }, "fill": { "kind": "none" },
        "text": { "paras": [ { "runs": if text.is_empty() { json!([]) } else { json!([{ "kind": "text", "text": text, "tag": "body", "look": { "font": "Calibri", "size": 14.0, "color": "000000" } }]) } } ] } });
    let body = json!({ "after": after, "arrangement": "halves-v:text,text", "objects": [ slot("text-1", 457200, "filled ellrin"), slot("text-2", 4572000, "") ] });
    let (code, v, _) = call(&app, "POST", "/api/decks/demo/new-slide", Some(body), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (_, d, _) = call(&app, "GET", "/api/decks/demo", None, None).await;
    let new = &d["deck"]["slides"][1];
    assert_eq!(new["arrangement"], "halves-v:text,text");
    let slots: Vec<&str> = new["objects"].as_array().unwrap().iter().filter_map(|o| o["slot"].as_str()).collect();
    assert_eq!(slots, ["text-1", "text-2"]);
    // export: the filled slot is there, the empty one is not
    let (_, _, pptx) = call(&app, "GET", "/api/decks/demo/pptx", None, None).await;
    let od = lore_ooxml::Deck::from_bytes(&pptx).unwrap();
    let xml = String::from_utf8(od.pkg.get(&od.slides[1].part).unwrap().to_vec()).unwrap();
    assert!(xml.contains("filled ellrin"));
    assert!(xml.contains("name=\"text-1\"") && !xml.contains("name=\"text-2\""), "{xml}");
    // deck defaults are a deck op (undoable) and come back with the change
    let (code, v, _) = call(&app, "POST", "/api/decks/demo/ops", Some(json!({ "ops": [ { "op": "set_deck", "path": "/defaults/content", "value": "halves-v:text,text" } ], "label": "預設版型" })), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["defaults"]["content"], "halves-v:text,text");
    let (_, v, _) = call(&app, "POST", "/api/decks/demo/undo", Some(json!({})), None).await;
    assert!(v["defaults"].as_object().is_none_or(|m| m.is_empty()), "{v}");
    // named arrangements are files in the workspace
    let (code, _, _) = call(&app, "POST", "/api/arrangements", Some(json!({ "id": "ellrin-1", "name": "左圖右文", "split": "halves-v", "regions": ["image", "text"] })), None).await;
    assert_eq!(code, StatusCode::OK);
    assert!(dir.path().join("arrangements/ellrin-1.json").is_file());
    let (_, v, _) = call(&app, "GET", "/api/decks/demo/arrangements", None, None).await;
    assert_eq!(v["presets"][0]["id"], "ellrin-1");
    let (code, _, _) = call(&app, "POST", "/api/arrangements", Some(json!({ "id": "../x", "name": "bad" })), None).await;
    assert!(code.is_client_error());
}

#[tokio::test]
async fn a_pptx_opened_where_it_lives_saves_back_into_it() {
    let (dir, app) = setup();
    let docs = dir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let file = docs.join("daidai.pptx");
    std::fs::write(&file, common::synthetic_deck()).unwrap();
    // browser mode: no native dialog
    let (_, info, _) = call(&app, "GET", "/api/info", None, None).await;
    assert_eq!(info["shell"], "browser");
    let (code, _, _) = call(&app, "POST", "/api/shell/dialog", Some(json!({ "kind": "open" })), None).await;
    assert_eq!(code, StatusCode::NOT_IMPLEMENTED);

    let (code, v, _) = call(&app, "POST", "/api/files/open", Some(json!({ "path": file })), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let name = v["name"].as_str().unwrap().to_string();
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    assert_eq!(d["file"]["state"], "saved");
    assert_eq!(d["file"]["file"], "daidai.pptx");
    let s2 = d["deck"]["slides"][1]["uid"].as_str().unwrap().to_string();
    let mut f = d["deck"]["slides"][1]["objects"].as_array().unwrap().iter().find(|o| o["id"] == 5).unwrap()["frame"].clone();
    f["x"] = json!(f["x"].as_i64().unwrap() + 100000);
    let (_, v, _) = call(&app, "POST", &format!("/api/decks/{name}/ops"), Some(json!({"ops": [{"op": "set_obj", "slide": s2, "id": 5, "path": "/frame", "value": f}], "label": "移動"})), None).await;
    assert_eq!(v["file"]["state"], "pending");
    let (code, v, _) = call(&app, "POST", &format!("/api/decks/{name}/file/write"), Some(json!({})), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!((v["written"].as_bool(), v["file"]["state"].as_str()), (Some(true), Some("saved")));
    assert_ne!(std::fs::read(&file).unwrap(), common::synthetic_deck());
    assert!(docs.join(".daidai.original.pptx").is_file());

    // changed elsewhere: the write is refused until the author chooses
    std::fs::write(&file, common::synthetic_deck()).unwrap();
    let (_, v, _) = call(&app, "POST", &format!("/api/decks/{name}/ops"), Some(json!({"ops": [{"op": "set_obj", "slide": s2, "id": 5, "path": "/frame/y", "value": 0}], "label": "移動"})), None).await;
    assert_eq!(v["file"]["state"], "changed");
    let (_, v, _) = call(&app, "POST", &format!("/api/decks/{name}/file/write"), Some(json!({})), None).await;
    assert_eq!((v["written"].as_bool(), v["file"]["state"].as_str()), (Some(false), Some("changed")));
    assert_eq!(std::fs::read(&file).unwrap(), common::synthetic_deck());
    // 重新載入
    let (code, _, _) = call(&app, "POST", &format!("/api/decks/{name}/file/reload"), Some(json!({})), None).await;
    assert_eq!(code, StatusCode::OK);
    let (_, d, _) = call(&app, "GET", &format!("/api/decks/{name}"), None, None).await;
    assert_eq!(d["file"]["state"], "saved");
    assert_ne!(d["deck"]["slides"][1]["objects"].as_array().unwrap().iter().find(|o| o["id"] == 5).unwrap()["frame"]["y"], 0);

    // 另存新檔 binds to the new file
    let to = docs.join("copy.pptx");
    let (code, v, _) = call(&app, "POST", &format!("/api/decks/{name}/file/save-as"), Some(json!({ "path": to })), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["file"]["file"], "copy.pptx");
    assert!(to.is_file());
    let (code, _, _) = call(&app, "POST", &format!("/api/decks/{name}/file/save-as"), Some(json!({ "path": docs.join("x.txt") })), None).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}
