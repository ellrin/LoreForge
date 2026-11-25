//! 待製作 in one go: every reference of the hub is drafted through the app's own routes (the
//! same calls the 製作 page makes), drawn once, and written to a folder to be looked at. Nothing
//! is saved into the hub: saving stays the author's step in the editor.
//!
//! Written into `out`, one set per reference (`<n>` = the id with every character that is not
//! a letter, a digit, `-` or `_` replaced by `_`):
//! `<n>.spec.json` (the draft's spec), `<n>.svg` (drawn by `/flow/api/preview`),
//! `<n>.ref.<ext>` (the reference's picture, when it has one), `<n>.told.json` (what the model
//! was shown, as the draft answered it), and `index.html`: one row per reference, its picture
//! beside the drawing, the error line where a draft or a drawing failed.

use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

/// What became of one reference.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Made {
    pub id: String,
    pub title: String,
    /// Drafted by the timeline task.
    pub timeline: bool,
    /// The engine that answered; empty when the draft failed.
    pub engine: String,
    /// Empty = drafted and drawn; else the one line of what went wrong.
    pub error: String,
}

/// One call through the router: status, content type, body bytes.
async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String, Vec<u8>) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty));
    let Ok(req) = req else { return (StatusCode::BAD_REQUEST, String::new(), Vec::new()) };
    match app.clone().oneshot(req).await {
        Ok(res) => {
            let status = res.status();
            let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.map(|b| b.to_vec()).unwrap_or_default();
            (status, ctype, bytes)
        }
        Err(e) => match e {},
    }
}

/// A JSON call: the reply, or the one line of what went wrong.
async fn json_call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> Result<Value, String> {
    let (status, _, bytes) = call(app, method, uri, body).await;
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status.is_success() && v.get("error").is_none_or(|e| e.is_null()) {
        return Ok(v);
    }
    Err(one_line(v["error"].as_str().filter(|e| !e.trim().is_empty()).map(str::to_string).unwrap_or_else(|| format!("HTTP {}", status.as_u16()))))
}

fn one_line(s: String) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A path segment: the id with `:` and the like escaped.
fn enc(id: &str) -> String {
    id.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The file stem of an id.
fn stem(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn ext_of(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        _ => "img",
    }
}

fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Draft and draw the references of `app` (a router as [`crate::router_isolated`] gives) into
/// `out`. `only` empty = all of them, else just these ids. One at a time, in the list's order;
/// a failure is recorded and the run goes on. `progress` is called after each one.
pub async fn run(app: axum::Router, out: &Path, only: &[String], progress: &mut (dyn FnMut(usize, usize, &Made) + Send)) -> std::io::Result<Vec<Made>> {
    std::fs::create_dir_all(out)?;
    // the list has no format filter: every non-entity row is read and the references kept
    // read page by page: the list answers at most 1000 rows at a time
    let mut refs: Vec<(String, String)> = Vec::new();
    let mut offset = 0usize;
    loop {
        let list = json_call(&app, "GET", &format!("/api/hub/items?entity=false&limit=1000&offset={offset}"), None).await.map_err(io_err)?;
        let rows = list["items"].as_array().cloned().unwrap_or_default();
        refs.extend(
            rows.iter()
                .filter(|i| i["format"] == "figure" && i["fields"]["what"] == "reference" && i["entity"] != json!(true))
                .filter_map(|i| Some((i["id"].as_str()?.to_string(), i["title"].as_str().unwrap_or("").to_string()))),
        );
        offset += rows.len();
        if rows.is_empty() || offset as u64 >= list["total"].as_u64().unwrap_or(0) {
            break;
        }
    }
    let refs: Vec<_> = refs.into_iter().filter(|(id, _)| only.is_empty() || only.contains(id)).collect();
    let total = refs.len();
    let (mut made, mut rows, mut used) = (Vec::new(), Vec::new(), std::collections::BTreeSet::new());
    for (n, (id, title)) in refs.into_iter().enumerate() {
        let mut name = stem(&id);
        while !used.insert(name.clone()) {
            name.push('_');
        }
        let (m, row) = one(&app, out, &id, &title, &name).await;
        progress(n + 1, total, &m);
        rows.push(row);
        made.push(m);
    }
    std::fs::write(out.join("index.html"), index_html(&rows))?;
    Ok(made)
}

/// What the index shows of one reference.
struct Row {
    made: Made,
    stem: String,
    picture: String,
}

async fn one(app: &axum::Router, out: &Path, id: &str, title: &str, n: &str) -> (Made, Row) {
    let mut m = Made { id: id.to_string(), title: title.to_string(), timeline: false, engine: String::new(), error: String::new() };
    let mut picture = String::new();
    match json_call(app, "GET", &format!("/api/hub/make/{}", enc(id)), None).await {
        Ok(v) => {
            if let Some(u) = v["image"].as_str().filter(|u| !u.is_empty()) {
                let (status, ctype, bytes) = call(app, "GET", u, None).await;
                if status.is_success() && !bytes.is_empty() {
                    let name = format!("{n}.ref.{}", ext_of(&ctype));
                    if std::fs::write(out.join(&name), bytes).is_ok() {
                        picture = name;
                    }
                }
            }
        }
        Err(e) => m.error = e,
    }
    if m.error.is_empty() {
        draft_and_draw(app, out, id, n, &mut m).await;
    }
    let row = Row { made: m.clone(), stem: n.to_string(), picture };
    (m, row)
}

async fn draft_and_draw(app: &axum::Router, out: &Path, id: &str, n: &str, m: &mut Made) {
    // the 製作 page sends no body to the draft
    let d = match json_call(app, "POST", &format!("/api/hub/make/{}/draft", enc(id)), None).await {
        Ok(d) => d,
        Err(e) => {
            m.error = e;
            return;
        }
    };
    m.engine = d["engine"].as_str().unwrap_or("").to_string();
    m.timeline = d["spec"]["timeline"].is_object();
    let spec = d["spec"].clone();
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
    if let Err(e) = std::fs::write(out.join(format!("{n}.spec.json")), pretty(&spec)).and_then(|_| std::fs::write(out.join(format!("{n}.told.json")), pretty(&d))) {
        m.error = one_line(e.to_string());
        return;
    }
    match json_call(app, "POST", "/flow/api/preview", Some(json!({ "spec": spec, "doc": "", "width": 900 }))).await {
        Ok(p) => match p["svg"].as_str().filter(|s| !s.is_empty()) {
            Some(svg) => {
                if let Err(e) = std::fs::write(out.join(format!("{n}.svg")), svg) {
                    m.error = one_line(e.to_string());
                }
            }
            None => m.error = "the drawing came back empty".into(),
        },
        Err(e) => m.error = e,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn index_html(rows: &[Row]) -> String {
    let mut h = String::from(
        "<!doctype html>\n<html lang=\"zh-Hant\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>待製作</title>\n<style>\nbody{margin:0;padding:24px;background:#f6f6f4;color:#222;font:15px/1.5 system-ui,sans-serif}\nh1{font-size:18px;margin:0 0 16px}\n.row{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,2fr);gap:16px;background:#fff;border:1px solid #ddd;border-radius:8px;padding:12px;margin-bottom:16px}\n.row h2{grid-column:1/-1;font-size:15px;margin:0}\n.row h2 small{font-weight:400;color:#777;margin-left:8px}\n.row img{max-width:100%;height:auto;display:block;background:#fafafa}\n.empty{color:#999;font-size:13px}\n.err{grid-column:1/-1;color:#a02020;font-size:13px}\n@media(max-width:700px){.row{grid-template-columns:1fr}}\n</style></head><body>\n",
    );
    h.push_str(&format!("<h1>待製作 {}</h1>\n", rows.len()));
    for r in rows {
        let m = &r.made;
        h.push_str("<section class=\"row\">");
        let tag = [if m.timeline { "時間軸" } else { "" }, m.engine.as_str()].iter().filter(|t| !t.is_empty()).map(|t| esc(t)).collect::<Vec<_>>().join(" · ");
        h.push_str(&format!("<h2>{}<small>{}</small></h2>", esc(&m.title), tag));
        if r.picture.is_empty() {
            h.push_str("<div class=\"empty\">無圖</div>");
        } else {
            h.push_str(&format!("<div><img src=\"{}\" alt=\"\"></div>", esc(&r.picture)));
        }
        if m.error.is_empty() {
            h.push_str(&format!("<div><img src=\"{}.svg\" alt=\"\"></div>", esc(&r.stem)));
        } else {
            h.push_str("<div class=\"empty\">未畫出</div>");
            h.push_str(&format!("<div class=\"err\">{}</div>", esc(&m.error)));
        }
        h.push_str("</section>\n");
    }
    h.push_str("</body></html>\n");
    h
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use kb_core::{FixedClock, KbOptions};
    use lore_llm::mock::MockProvider;
    use lore_llm::router::MockFactory;
    use serde_json::{Value, json};

    use super::*;
    use crate::hub::HubState;
    use crate::llm::Llm;
    use crate::reminders::Reminders;
    use crate::{AppState, Config};

    const MANIFEST: &str = r#"
[[category]]
id = "t.flow"
label = "流程圖"
app = "t"
group = "流程圖"
format = "figure"
writable = true
[category.native]
kind = "flow"
app_dir = "App"
path = "flow"

[[category]]
id = "t.flowrefs"
label = "參考圖"
app = "t"
group = "流程圖"
format = "figure"
writable = true
[category.native]
kind = "dir"
app_dir = "App"
path = "refs"

[[category]]
id = "t.flowspecs"
label = "流程圖（做好的）"
app = "t"
group = "流程圖"
format = "figure"
writable = true
"#;

    const SPEC: &str = "fig demo\nx: image \"Input\"\nf: box \"Model\"\ny: label \"Output\"\nx -> f -> y\n";
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, 1, 2, 3];

    fn graph_reply() -> String {
        json!({
            "fig": "pipe", "panels": [],
            "nodes": [
                { "id": "x", "module": "image", "label": "Input" },
                { "id": "f", "module": "box", "label": "Model" },
                { "id": "y", "module": "label", "label": "Output" }
            ],
            "edges": [ { "from": "x", "to": "f", "kind": "data" }, { "from": "f", "to": "y", "kind": "data" } ],
            "vars_new": []
        })
        .to_string()
    }

    /// The model's answer to "a timeline or a data flow?", asked before every draft.
    fn kind(k: &str) -> String {
        json!({ "kind": k, "why": "scripted" }).to_string()
    }

    fn timeline_reply() -> String {
        json!({
            "title": "Release history", "style": "line-below",
            "points": [
                { "time": "2021", "title": "Pilot", "body": "Two sites.", "icon": "clipboard" },
                { "time": "2022", "title": "Cohort", "body": "Two hundred cases.", "icon": "people" }
            ]
        })
        .to_string()
    }

    struct Env {
        dir: tempfile::TempDir,
        hubdir: PathBuf,
        app: axum::Router,
    }

    fn write(p: &Path, text: impl AsRef<[u8]>) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// Three references: f01 (picture), f02 (a timeline), f03 (plain, no picture); the model
    /// answers with `llm`.
    fn setup(llm: Llm) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let hubdir = dir.path().join("hubdata");
        let a = hubdir.join("app");
        write(&a.join("refs/f01/desc.md"), "# f01 Training loop\n\nInput goes through a model to an output.\n");
        write(&a.join("refs/f01/ref.png"), PNG);
        write(&a.join("refs/f02/desc.md"), "# f02 Release history\n\nA timeline with five points.\n");
        write(&a.join("refs/f03/desc.md"), "# f03 Plain <b>&\n\nNo picture here.\n");
        let p = dir.path().join("personal");
        std::fs::create_dir_all(p.join("nodes")).unwrap();
        std::fs::write(p.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
        let config = Config { spaces: vec![p], member: "ellrin".into(), port: 0, web: dir.path().join("web"), intervals: Default::default(), app_dir: None };
        let kb = crate::open(&config, KbOptions { clock: Arc::new(FixedClock::at("2026-11-08T21:10:00+08:00")), ..Default::default() }).unwrap();
        let rem = Reminders::isolated(&dir.path().join("rem"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
        let state = HubState::isolated(&hubdir, MANIFEST).unwrap();
        let app = crate::router_isolated(AppState::with_hub(kb, config.web.clone(), rem, llm, state));
        Env { dir, hubdir, app }
    }

    fn scripted(replies: Vec<String>) -> Llm {
        Llm::with_factory(MockFactory::default().with("claude:sonnet", Arc::new(MockProvider::new(replies))))
    }

    async fn go(e: &Env, out: &Path, only: &[String]) -> Vec<Made> {
        run(e.app.clone(), out, only, &mut |_, _, _| {}).await.unwrap()
    }

    fn ids(m: &[Made]) -> Vec<&str> {
        m.iter().map(|m| m.id.as_str()).collect()
    }

    #[tokio::test]
    async fn lists_every_reference_and_nothing_drawable() {
        let e = setup(Llm::none());
        // one reference made into a drawable figure: the figure is not a row, its reference still is
        let (s, _, b) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": { "lore_flow": 1, "dsl": SPEC } }))).await;
        assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
        let out = e.dir.path().join("out");
        let made = go(&e, &out, &[]).await;
        let mut got = ids(&made);
        got.sort();
        assert_eq!(got, ["t.flowrefs:f01", "t.flowrefs:f02", "t.flowrefs:f03"]);
        assert!(made.iter().all(|m| !m.id.starts_with("t.flowspecs:")));
    }

    #[tokio::test]
    async fn only_keeps_the_named_ids() {
        let e = setup(Llm::none());
        let out = e.dir.path().join("out");
        let made = go(&e, &out, &["t.flowrefs:f03".to_string(), "t.nothing:x".to_string()]).await;
        assert_eq!(ids(&made), ["t.flowrefs:f03"]);
        assert!(!out.join("t_flowrefs_f01.ref.png").exists());
    }

    #[tokio::test]
    async fn a_drafted_reference_leaves_its_spec_its_drawing_and_what_was_told() {
        let e = setup(scripted(vec![kind("flow"), graph_reply()]));
        let out = e.dir.path().join("out");
        let made = go(&e, &out, &["t.flowrefs:f03".to_string()]).await;
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].error, "");
        assert_eq!(made[0].engine, "mock");
        assert!(!made[0].timeline);
        let svg = std::fs::read_to_string(out.join("t_flowrefs_f03.svg")).unwrap();
        assert!(svg.trim_start().starts_with("<svg"), "{}", &svg[..svg.len().min(80)]);
        let spec: Value = serde_json::from_str(&std::fs::read_to_string(out.join("t_flowrefs_f03.spec.json")).unwrap()).unwrap();
        assert!(spec["dsl"].as_str().unwrap().contains("Model"));
        let told: Value = serde_json::from_str(&std::fs::read_to_string(out.join("t_flowrefs_f03.told.json")).unwrap()).unwrap();
        assert!(told["told"]["modules"].is_array(), "{told}");
        assert!(out.join("index.html").is_file());
    }

    #[tokio::test]
    async fn the_references_picture_is_copied_beside_it() {
        let e = setup(scripted(vec![kind("flow"), graph_reply()]));
        let out = e.dir.path().join("out");
        go(&e, &out, &["t.flowrefs:f01".to_string(), "t.flowrefs:f03".to_string()]).await;
        assert_eq!(std::fs::read(out.join("t_flowrefs_f01.ref.png")).unwrap(), PNG);
        assert!(!out.join("t_flowrefs_f03.ref.png").exists(), "no picture, no file");
    }

    #[tokio::test]
    async fn a_timeline_reference_is_marked_and_drawn() {
        let e = setup(scripted(vec![kind("timeline"), timeline_reply()]));
        let out = e.dir.path().join("out");
        let made = go(&e, &out, &["t.flowrefs:f02".to_string()]).await;
        assert!(made[0].timeline, "{made:?}");
        assert_eq!(made[0].error, "");
        assert!(std::fs::read_to_string(out.join("t_flowrefs_f02.svg")).unwrap().trim_start().starts_with("<svg"));
        let spec: Value = serde_json::from_str(&std::fs::read_to_string(out.join("t_flowrefs_f02.spec.json")).unwrap()).unwrap();
        assert_eq!(spec["timeline"]["points"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_failed_draft_is_recorded_and_the_run_goes_on() {
        // the first reference gets an answer that is no figure twice (the draft asks again once),
        // the next a good one
        let bad = json!({ "nodes": [{ "id": "a", "module": "box", "label": "A" }], "edges": [{ "from": "a", "to": "zz" }] }).to_string();
        let e = setup(scripted(vec![kind("flow"), bad.clone(), bad, kind("flow"), graph_reply()]));
        let out = e.dir.path().join("out");
        let mut seen = vec![];
        let made = run(e.app.clone(), &out, &["t.flowrefs:f01".to_string(), "t.flowrefs:f03".to_string()], &mut |n, t, m| seen.push((n, t, m.error.is_empty()))).await.unwrap();
        assert_eq!(made.len(), 2);
        let (bad, good): (Vec<_>, Vec<_>) = made.iter().partition(|m| !m.error.is_empty());
        assert_eq!((bad.len(), good.len()), (1, 1), "{made:?}");
        assert!(!bad[0].error.contains('\n') && bad[0].engine.is_empty());
        assert!(!out.join(format!("{}.svg", stem(&bad[0].id))).exists());
        assert!(out.join(format!("{}.svg", stem(&good[0].id))).exists());
        assert_eq!(seen.iter().map(|s| (s.0, s.1)).collect::<Vec<_>>(), [(1, 2), (2, 2)]);
        assert!(std::fs::read_to_string(out.join("index.html")).unwrap().contains(&esc(&bad[0].error)));
    }

    #[tokio::test]
    async fn with_no_model_every_row_says_why() {
        let e = setup(Llm::none());
        let out = e.dir.path().join("out");
        let made = go(&e, &out, &[]).await;
        assert_eq!(made.len(), 3);
        for m in &made {
            assert!(!m.error.is_empty() && !m.error.contains('\n'), "{m:?}");
        }
    }

    /// Every file under `dir` with its bytes.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut all = vec![];
        let mut todo = vec![dir.to_path_buf()];
        while let Some(d) = todo.pop() {
            for f in std::fs::read_dir(d).unwrap().flatten() {
                let p = f.path();
                if p.is_dir() {
                    todo.push(p);
                } else {
                    all.push((p.clone(), std::fs::read(&p).unwrap()));
                }
            }
        }
        all.sort();
        all
    }

    #[tokio::test]
    async fn the_hub_is_unchanged_after_a_run() {
        let e = setup(scripted(vec![kind("flow"), graph_reply(), kind("timeline"), timeline_reply()]));
        // one look first, so any lazily made file of the hub is there before the comparison
        let (_, _, _) = call(&e.app, "GET", "/api/hub/items?limit=5", None).await;
        let before = snapshot(&e.hubdir);
        let list = |app: axum::Router| async move { json_call(&app, "GET", "/api/hub/items?limit=1000", None).await.unwrap()["items"].clone() };
        let items_before = list(e.app.clone()).await;
        let out = e.dir.path().join("out");
        let made = go(&e, &out, &[]).await;
        assert_eq!(made.len(), 3);
        assert_eq!(list(e.app.clone()).await, items_before);
        assert_eq!(snapshot(&e.hubdir), before);
    }

    #[tokio::test]
    async fn the_index_lists_every_row_and_escapes_titles() {
        let e = setup(scripted(vec![kind("flow"), graph_reply()]));
        let out = e.dir.path().join("out");
        let made = go(&e, &out, &[]).await;
        let html = std::fs::read_to_string(out.join("index.html")).unwrap();
        assert_eq!(html.matches("<section class=\"row\">").count(), made.len());
        // the title "Plain <b>&" is text, never markup
        assert!(html.contains("Plain &lt;b&gt;&amp;"), "{html}");
        assert!(!html.contains("<b>"));
        assert!(html.contains("t_flowrefs_f01.ref.png"));
        assert!(!html.contains("http://") && !html.contains("https://"));
        // a row that failed shows its line, one that was drawn shows its drawing
        let failed = made.iter().find(|m| !m.error.is_empty()).expect("the scripted model answers once");
        assert!(html.contains(&esc(&failed.error)));
        let drawn = made.iter().find(|m| m.error.is_empty()).unwrap();
        assert!(html.contains(&format!("{}.svg", stem(&drawn.id))));
    }
}
