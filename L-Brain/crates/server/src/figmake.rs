//! 製作: a 待製作 reference (a picture and a description of a flow figure the flow tool cannot draw
//! yet) is made in the shared flow editor, and saving turns it into a drawable figure item linked
//! to the reference (`docs/brain-hub.md` §2 rule 2, §4.2). Also the model adapter the editor at
//! `/flow` uses (L-Brain's lore-llm routing behind lore-flow's `Llm` trait).
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/hub/make/{id} | `{id, title, source, image, spec, made}`: what the editor starts from (409: not a reference) |
//! | POST | /api/hub/make/{id} | `{spec, title?}` → `{figure, reference}`; the spec is drawn once first (422 when it cannot be) |
//! | POST | /api/hub/make/{id}/draft | `{spec, engine}`: a first draft from the description (and the picture); stored nowhere |
//!
//! A reference that describes a time axis is made as a timeline: its draft comes from the
//! timeline task (`sc-flow-timeline`, with a picture `sc-flow-timeline-vision`) and is a spec with
//! `timeline` set and no `dsl`; saving draws it once like any other spec and stores it the same way.
//!
//! The made figure is an item of the category whose id ends in `.flowspecs` (`what = spec`,
//! `level = figure`, `entity`, `fields.params` = the spec, `fields.made_from` = the reference id,
//! the reference's domain / kind tags). One figure per reference: saving again replaces it. The
//! reference lists the figure in its `contains` (kept in the hub's overlay); the reference's own
//! files (desc.md) have no place for a `made` field, so the figure side (`made_from`) is the
//! authority and the dashboard's 待製作 counts both signals.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_know::formats::figure::{self, what};
use lore_know::{Item, Sink, Source, facet, format};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Shared;
use crate::hub::{HubError, R, hub, not_found, ok, row};
use crate::llm::Llm;

/// The flow editor's model calls, through this app's routing (the same shape as L-Paper's).
/// A task with a picture (`sc-flow-graph-vision`) is Claude only: its task table says so.
pub struct FlowLlm(pub Arc<Llm>);

#[async_trait::async_trait]
impl lore_flow::llm::Llm for FlowLlm {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(Value, String), String> {
        let a = self.0.complete_json(task, req, None).await.map_err(|(line, _)| line)?;
        Ok((a.value, a.engine))
    }
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/hub/make/{id}", get(start).post(save).layer(axum::extract::DefaultBodyLimit::max(20 * 1024 * 1024)))
        .route("/api/hub/make/{id}/draft", post(draft))
}

fn err(status: StatusCode, msg: &str) -> HubError {
    HubError::new(status, msg)
}

/// The reference `id` names, or 404 / 409.
fn reference(h: &lore_know::hub::Hub, id: &str) -> Result<Item, HubError> {
    let item = h.store.get(id)?.ok_or_else(|| not_found(id))?;
    if item.format != format::FIGURE || item.field("what") != what::REFERENCE || item.entity {
        return Err(err(StatusCode::CONFLICT, "這一條已經是畫得出來的圖，不用再製作。"));
    }
    Ok(item)
}

/// The category whole figures are stored in.
fn spec_category(h: &lore_know::hub::Hub) -> Result<String, HubError> {
    h.store
        .registry()
        .all()
        .iter()
        .find(|c| c.id.ends_with(".flowspecs") && c.format == format::FIGURE && c.writable && c.native.kind.is_empty())
        .map(|c| c.id.clone())
        .ok_or_else(|| err(StatusCode::CONFLICT, "這個空間沒有放做好的流程圖的類別。"))
}

/// The figure already made from `reference`, if any.
fn made_figure(h: &lore_know::hub::Hub, cat: &str, reference: &str) -> Result<Option<Item>, HubError> {
    Ok(h.store.list(cat)?.into_iter().find(|i| i.field("made_from") == reference))
}

fn spec_value(item: &Item) -> Value {
    match item.fields.get("params") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    }
}

async fn start(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let h = hub(&s)?;
    let r = reference(&h, &id)?;
    let made = match spec_category(&h) {
        Ok(cat) => made_figure(&h, &cat, &r.id)?,
        Err(_) => None,
    };
    let image = if r.field("image").is_empty() { String::new() } else { format!("/api/hub/thumb/{}", r.id) };
    ok(json!({
        "id": r.id,
        "title": r.title,
        "source": r.field("description"),
        "image": image,
        "spec": made.as_ref().map(spec_value).unwrap_or(Value::Null),
        "made": made.map(|m| m.id).unwrap_or_default(),
    }))
}

#[derive(Deserialize)]
struct MakeBody {
    spec: Value,
    #[serde(default)]
    title: Option<String>,
    /// 指揮文字 / 口語說明 as the editor wrote them; absent = derived from the spec
    #[serde(default)]
    commands: Option<String>,
    #[serde(default)]
    spoken: Option<String>,
}

async fn save(State(s): State<Shared>, Path(id): Path<String>, Json(b): Json<MakeBody>) -> R {
    // the page may send the spec as an object or as its JSON text
    let raw = match b.spec {
        Value::String(t) => serde_json::from_str(&t).map_err(|_| err(StatusCode::BAD_REQUEST, "圖的規格不是 JSON。"))?,
        v => v,
    };
    if !raw.is_object() {
        return Err(err(StatusCode::BAD_REQUEST, "圖的規格要是一個物件。"));
    }
    // normalised through the flow tool's own type: `lore_flow: 1` is there, unknown keys stay
    let spec = lore_flow::figure::FlowSpec::from_value(raw).map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &format!("圖的規格讀不了：{e}")))?.to_value();
    let mut h = hub(&s)?;
    let reference_item = reference(&h, &id)?;
    let cat = spec_category(&h)?;
    let title = b.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&reference_item.title).to_string();

    // draw it once: a spec that cannot be drawn is not stored
    let probe = figure::spec_item(&cat, &title, &spec, Source::author());
    figure::thumbnail(&probe, &h.catalogue()).map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()))?;

    // the entities its nodes use, for `contains`
    let mut entities: Vec<Item> = vec![];
    for c in h.store.registry().all().iter().filter(|c| c.format == format::FIGURE) {
        entities.extend(h.store.list(&c.id)?.into_iter().filter(|i| i.entity && matches!(i.field("what"), what::GENERATOR | what::DRAWING)));
    }

    let existing = made_figure(&h, &cat, &reference_item.id)?;
    let mut fig = existing.unwrap_or(probe);
    fig.title = title;
    fig.set("params", spec.clone()).set("made_from", reference_item.id.clone());
    fig.contains = figure::parts_of(&spec, &entities);
    // the figure as knowledge: what it shows (the reference's words when the figure brought
    // none), the variables it takes and gives and its connections, read from the figure itself.
    // Only what is still empty is filled: what the author wrote stays, and 用法 is his alone.
    // what came with the save is kept as given; the rest is derived below
    for (k, v) in [("commands", &b.commands), ("spoken", &b.spoken)] {
        if let Some(t) = v.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            fig.set(k, t.to_string());
        }
    }
    lore_know::flowknow::fill_item(&mut fig, &h.catalogue().library);
    if fig.field("description").trim().is_empty() {
        fig.set("description", reference_item.field("description").to_string());
    }
    for f in [facet::DOMAIN, facet::KIND] {
        for t in reference_item.tags.get(f).cloned().unwrap_or_default() {
            fig.tag(f, &t);
        }
    }
    let figure_item = h.put(fig)?;

    // the reference lists the figure
    let mut r = reference_item;
    if !r.contains.contains(&figure_item.id) {
        r.contains.push(figure_item.id.clone());
    }
    let reference_item = h.put(r)?;
    ok(json!({ "figure": row(&h, &figure_item), "reference": row(&h, &reference_item) }))
}

async fn draft(State(s): State<Shared>, Path(id): Path<String>) -> R {
    // everything from the hub first: the lock is not held over the model call
    let description = {
        let h = hub(&s)?;
        let r = reference(&h, &id)?;
        // The reference's picture goes to the model with the description (task
        // `sc-flow-graph-vision`, Claude only): the author accepts that for figures from papers
        // and his own slides. A vector picture is not sent, a model cannot read it.
        let image = crate::hub::reference_picture(&h, &s.hub_dirs, &r).filter(|(_, mime)| *mime != "image/svg+xml").map(|(bytes, mime)| lore_llm::Image::from_bytes(mime, &bytes));
        // the author's knowledge of figures, read from the hub: every component and module
        // (with his 用法 and their variables) and every figure he made (as a worked example)
        let mut items: Vec<Item> = vec![];
        for c in h.store.registry().all().iter().filter(|c| c.format == format::FIGURE) {
            items.extend(h.store.list(&c.id)?);
        }
        // the hub's own catalogue: the drawings saved under every app's flow folder are offered,
        // not only the shared ones
        (format!("{}\n\n{}", r.title, r.field("description")), image, lore_know::flowknow::knowledge(&items), h.catalogue())
    };
    let (description, image, know, cat) = (description.0, description.1, description.2, description.3);
    let llm = FlowLlm(s.llm.clone());
    // a time axis or a data flow: the model looks at the picture and says (the words of the
    // title decided before, and seven of twelve timelines were drawn as boxes and arrows)
    let (timeline, why) = lore_flow::timeline::llm::is_timeline_via(&llm, &description, image.as_ref()).await.map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &e))?;
    if timeline {
        // only the styles that draw as written are offered
        let styles: Vec<lore_flow::timeline::Style> = cat.timelines.iter().filter(|st| st.cannot.is_empty()).cloned().collect();
        let prompt = lore_flow::timeline::llm::TimelinePrompt { description: &description, image, styles: &styles, language: "" };
        let (t, engine) = lore_flow::timeline::llm::propose_via(&llm, &prompt).await.map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &e))?;
        let spec = lore_flow::figure::FlowSpec { lore_flow: lore_flow::figure::VERSION, timeline: Some(t), ..Default::default() };
        return ok(json!({ "spec": spec.to_value(), "engine": engine, "kind": "timeline", "why": why }));
    }
    let examples = s.flow.examples(&cat.library);
    let prompt = lore_flow::llm::FlowPrompt { description: &description, image, notation: None, library: &cat.library, examples: &examples };
    let (p, shown) = lore_flow::llm::propose_with(&llm, &prompt, &know).await.map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &e))?;
    let spec = lore_flow::figure::FlowSpec { lore_flow: lore_flow::figure::VERSION, dsl: p.dsl, ..Default::default() };
    // what the model was told from his knowledge, each with the one line on why it was picked
    let told = json!({
        "modules": shown.modules.iter().map(|m| json!({ "kind": m.item.kind, "style": m.item.style, "title": m.item.title, "why": m.why })).collect::<Vec<_>>(),
        "examples": shown.examples.iter().map(|x| json!({ "id": x.item.id, "title": x.item.title, "why": x.why })).collect::<Vec<_>>(),
    });
    ok(json!({ "spec": spec.to_value(), "engine": p.provider, "told": told, "kind": "flow", "why": why }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use kb_core::{FixedClock, KbOptions};
    use lore_llm::mock::MockProvider;
    use lore_llm::router::MockFactory;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::hub::HubState;
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

    struct Env {
        _dir: tempfile::TempDir,
        app: axum::Router,
        mock: Arc<MockProvider>,
    }

    fn write(p: &std::path::Path, text: impl AsRef<[u8]>) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn setup(replies: Vec<String>) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let hubdir = dir.path().join("hubdata");
        let a = hubdir.join("app");
        write(&a.join("refs/f01/desc.md"), "# f01 Training loop\n\nInput goes through a model to an output.\n");
        write(&a.join("refs/f01/ref.png"), b"PNGDATA");
        write(&a.join("refs/f02/desc.md"), "# f02 Release history\n\nA timeline with five points.\n");
        write(&a.join("refs/f03/desc.md"), "# f03 Plain\n\nNo picture here.\n");
        let p = dir.path().join("personal");
        std::fs::create_dir_all(p.join("nodes")).unwrap();
        std::fs::write(p.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
        let config = Config { spaces: vec![p], member: "ellrin".into(), port: 0, web: dir.path().join("web"), intervals: Default::default(), app_dir: None };
        let kb = crate::open(&config, KbOptions { clock: Arc::new(FixedClock::at("2026-11-08T21:10:00+08:00")), ..Default::default() }).unwrap();
        let rem = Reminders::isolated(&dir.path().join("rem"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
        let mock = Arc::new(MockProvider::new(replies));
        let f = MockFactory::default().with("claude:sonnet", mock.clone());
        let state = HubState::isolated(&hubdir, MANIFEST).unwrap();
        let app = crate::router_isolated(AppState::with_hub(kb, config.web.clone(), rem, Llm::with_factory(f), state));
        Env { _dir: dir, app, mock }
    }

    use crate::llm::Llm;

    async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let s = res.status();
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    fn spec(dsl: &str) -> Value {
        json!({ "lore_flow": 1, "dsl": dsl })
    }

    #[test]
    fn the_flow_tasks_are_in_lbrains_task_table() {
        let t = Llm::task_table(None).unwrap();
        for k in ["sc-flow-graph", "sc-flow-graph-vision", "sc-flow-timeline", "sc-flow-timeline-vision", "flow-discuss", "flow-assist"] {
            assert!(t.task.contains_key(k), "{k}");
        }
        assert!(t.task.contains_key(crate::llm::TASK_DRAFT), "L-Brain's own task stays");
        assert!(t.task["sc-flow-graph-vision"].needs.vision);
    }

    #[tokio::test]
    async fn the_editor_reaches_the_model_through_lbrain() {
        let reply = json!({ "say": "從輸入到輸出。", "question": "", "options": [], "pick": -1 }).to_string();
        let e = setup(vec![reply]);
        let body = json!({ "step": "outline", "source": "A model.", "answers": {}, "doc": "" });
        let (s, v) = call(&e.app, "POST", "/flow/api/discuss", Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["say"], "從輸入到輸出。");
        assert_eq!(v["engine"], "mock");
        assert_eq!(e.mock.seen.lock().unwrap().len(), 1);
        // without any model the editor says so in one line and stays up
        let none = crate::router_isolated(AppState::with_llm(
            {
                let d = tempfile::tempdir().unwrap();
                let p = d.path().join("personal");
                std::fs::create_dir_all(p.join("nodes")).unwrap();
                std::fs::write(p.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
                let c = Config { spaces: vec![p], member: "ellrin".into(), port: 0, web: d.path().join("web"), intervals: Default::default(), app_dir: None };
                let kb = crate::open(&c, KbOptions::default()).unwrap();
                std::mem::forget(d);
                kb
            },
            std::env::temp_dir(),
            Reminders::isolated(&std::env::temp_dir().join("figmake-rem"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700"),
            Llm::none(),
        ));
        let (s, v) = call(&none, "POST", "/flow/api/discuss", Some(json!({ "step": "outline", "source": "A", "answers": {}, "doc": "" }))).await;
        assert_ne!(s, StatusCode::OK);
        assert!(!v["error"].as_str().unwrap_or("").contains('\n'), "{v}");
    }

    #[tokio::test]
    async fn a_reference_gives_the_editor_its_description_and_picture() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "GET", "/api/hub/make/t.flowrefs:f01", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        println!("GET /api/hub/make/t.flowrefs:f01 -> {v}");
        assert_eq!(v["title"], "Training loop");
        assert_eq!(v["source"], "Input goes through a model to an output.");
        assert_eq!(v["image"], "/api/hub/thumb/t.flowrefs:f01");
        assert_eq!((v["spec"].is_null(), v["made"].as_str()), (true, Some("")));
        let (_, v) = call(&e.app, "GET", "/api/hub/make/t.flowrefs:f03", None).await;
        assert_eq!(v["image"], "");
        assert_eq!(call(&e.app, "GET", "/api/hub/make/t.flowrefs:nope", None).await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn making_stores_a_drawable_figure_linked_to_its_reference() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        println!("POST /api/hub/make/t.flowrefs:f01 -> {v}");
        let fig = &v["figure"];
        let fid = fig["id"].as_str().unwrap().to_string();
        assert!(fid.starts_with("t.flowspecs:"), "{fid}");
        assert_eq!((fig["entity"].clone(), fig["level"].as_str(), fig["fields"]["what"].as_str()), (json!(true), Some("figure"), Some("spec")));
        assert_eq!(fig["title"], "Training loop");
        assert_eq!(fig["fields"]["made_from"], "t.flowrefs:f01");
        assert_eq!(fig["fields"]["params"]["lore_flow"], 1);
        assert_eq!(fig["thumb"], format!("/api/hub/thumb/{fid}"));
        assert!(v["reference"]["contains"].as_array().unwrap().contains(&json!(fid)));
        // drawn
        let req = Request::builder().uri(format!("/api/hub/thumb/{fid}")).body(Body::empty()).unwrap();
        let res = e.app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // the editor re-opens on it
        let (_, g) = call(&e.app, "GET", "/api/hub/make/t.flowrefs:f01", None).await;
        assert_eq!(g["made"], json!(fid));
        assert_eq!(g["spec"]["dsl"], SPEC);
        // it is listed as a figure
        let (_, l) = call(&e.app, "GET", "/api/hub/items?category=t.flowspecs", None).await;
        assert_eq!(l["items"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_spec_that_cannot_be_drawn_is_a_422() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec("this is not a figure ((("), "title": "X" }))).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        let line = v["error"].as_str().unwrap();
        assert!(!line.is_empty() && !line.contains('\n'), "{line}");
        // nothing was stored
        let (_, l) = call(&e.app, "GET", "/api/hub/items?category=t.flowspecs", None).await;
        assert!(l["items"].as_array().unwrap().is_empty());
        assert_eq!(call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": 5 }))).await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn making_again_replaces_the_same_figure() {
        let e = setup(vec![]);
        let (_, a) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        let more = format!("{SPEC}z: label \"Done\"\ny -> z\n");
        let (s, b) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(&more), "title": "Loop v2" }))).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(a["figure"]["id"], b["figure"]["id"]);
        assert_eq!(b["figure"]["revision"].as_u64().unwrap(), a["figure"]["revision"].as_u64().unwrap() + 1);
        assert_eq!(b["figure"]["title"], "Loop v2");
        assert_eq!(b["figure"]["fields"]["params"]["dsl"], more);
        let (_, l) = call(&e.app, "GET", "/api/hub/items?category=t.flowspecs", None).await;
        assert_eq!(l["items"].as_array().unwrap().len(), 1);
        assert_eq!(b["reference"]["contains"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_made_reference_is_no_longer_to_make() {
        let e = setup(vec![]);
        let to_make = |v: &Value| -> u64 {
            let fam = v["stats"]["families"].as_array().unwrap().iter().find(|f| f["label"] == "流程圖").unwrap().clone();
            fam["parts"].as_array().unwrap().iter().find(|p| p["label"] == "待製作").unwrap()["count"].as_u64().unwrap()
        };
        let (s, before) = call(&e.app, "GET", "/api/hub/stats", None).await;
        assert_eq!(s, StatusCode::OK, "{before}");
        assert_eq!(to_make(&before), 3);
        call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        let (_, after) = call(&e.app, "GET", "/api/hub/stats", None).await;
        assert_eq!(to_make(&after), 2);
    }

    #[tokio::test]
    async fn an_entity_is_not_makeable() {
        let e = setup(vec![]);
        let (_, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        let fid = v["figure"]["id"].as_str().unwrap().to_string();
        for (m, tail) in [("GET", ""), ("POST", ""), ("POST", "/draft")] {
            let body = (m == "POST" && tail.is_empty()).then(|| json!({ "spec": spec(SPEC) }));
            let (s, v) = call(&e.app, m, &format!("/api/hub/make/{fid}{tail}"), body).await;
            assert_eq!(s, StatusCode::CONFLICT, "{m}{tail} {v}");
            assert!(!v["error"].as_str().unwrap().contains('\n'));
        }
    }

    /// The model's answer to "a timeline or a data flow?", asked before every draft.
    fn kind(k: &str) -> String {
        json!({ "kind": k, "why": "scripted" }).to_string()
    }

    #[tokio::test]
    async fn a_draft_comes_from_the_description_and_the_picture() {
        let e = setup(vec![kind("flow"), graph_reply()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        println!("POST /api/hub/make/t.flowrefs:f01/draft -> {v}");
        let dsl = v["spec"]["dsl"].as_str().unwrap();
        assert!(dsl.contains("Model") && dsl.contains("->"), "{dsl}");
        assert_eq!(v["spec"]["lore_flow"], 1);
        {
            let seen = e.mock.seen.lock().unwrap();
            let text = seen[1].flatten();
            assert!(text.contains("Training loop") && text.contains("Input goes through a model"), "{text}");
            assert!(serde_json::to_string(&seen[1].messages).unwrap().contains("image/png"), "the picture is attached");
        }
        // nothing was stored
        let (_, l) = call(&e.app, "GET", "/api/hub/items?category=t.flowspecs", None).await;
        assert!(l["items"].as_array().unwrap().is_empty());
        // a reference without a picture is drafted from the description alone
        let e = setup(vec![kind("flow"), graph_reply()]);
        let (s, _) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f03/draft", None).await;
        assert_eq!(s, StatusCode::OK);
        assert!(!serde_json::to_string(&e.mock.seen.lock().unwrap()[1].messages).unwrap().contains("image/png"));
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

    fn timeline_spec(points: Value) -> Value {
        json!({ "lore_flow": 1, "timeline": { "title": "Release history", "style": "line-below", "points": points } })
    }

    fn two_points() -> Value {
        json!([
            { "time": "2021", "title": "Pilot", "body": "Two sites.", "icon": "clipboard" },
            { "time": "2022", "title": "Cohort", "body": "Two hundred cases.", "icon": "people" }
        ])
    }

    #[tokio::test]
    async fn a_timeline_reference_is_drafted_as_a_timeline() {
        let e = setup(vec![kind("timeline"), timeline_reply()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f02/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["spec"]["lore_flow"], 1);
        assert_eq!(v["spec"]["timeline"]["points"].as_array().unwrap().len(), 2, "{v}");
        assert_eq!(v["engine"], "mock");
        // nothing was stored
        let (_, l) = call(&e.app, "GET", "/api/hub/items?category=t.flowspecs", None).await;
        assert!(l["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_timeline_draft_uses_the_timeline_task() {
        let e = setup(vec![kind("timeline"), timeline_reply()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f02/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let seen = e.mock.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        // the mock does not record the task id; the timeline task is the one whose reply schema has `points`
        // (and the picture-less reference goes to the text task, `sc-flow-timeline`)
        let schema = serde_json::to_string(&seen[1].json_schema).unwrap();
        assert!(schema.contains("points") && !schema.contains("nodes"), "{schema}");
    }

    #[tokio::test]
    async fn a_figure_he_made_is_what_the_next_draft_is_shown() {
        let e = setup(vec![kind("flow"), graph_reply()]);
        // he makes one figure and writes when a box is the thing to use
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC), "title": "Training loop" }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let fid = v["figure"]["id"].as_str().unwrap().to_string();
        let (_, l) = call(&e.app, "GET", "/api/hub/items?q=box&limit=200", None).await;
        let mut boxed = l["items"].as_array().unwrap().iter().find(|i| i["fields"]["kind"] == "box" && i["fields"]["what"] == "generator").cloned().expect("a built-in box");
        boxed["fields"]["prompt"] = json!("Use a plain box for a model whose inside does not matter to the output.");
        let bid = boxed["id"].as_str().unwrap().to_string();
        let (s, v) = call(&e.app, "PUT", &format!("/api/hub/items/{}", bid.replace(':', "%3A").replace('/', "%2F")), Some(boxed)).await;
        assert_eq!(s, StatusCode::OK, "a built-in module takes his 用法: {v}");
        // the next draft of a figure about the same things is told both, and says so
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let told = &v["told"];
        assert_eq!(told["examples"][0]["id"], json!(fid), "{told}");
        assert!(!told["examples"][0]["why"].as_str().unwrap().is_empty());
        assert!(told["modules"].as_array().unwrap().iter().any(|m| m["kind"] == "box"), "{told}");
        let seen = e.mock.seen.lock().unwrap();
        let text = seen[1].system.clone();
        assert!(text.contains("the author's own figure: Training loop"), "{text}");
        assert!(text.contains("whose inside does not matter"), "{text}");
    }

    #[tokio::test]
    async fn a_made_figure_carries_its_variables_and_connections() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let f = &v["figure"]["fields"];
        assert!(f["inputs"].as_str().unwrap().contains("image"), "{f}");
        assert!(f["connects"].as_str().unwrap().lines().count() == 2, "{f}");
        assert!(!f["description"].as_str().unwrap().is_empty(), "what it shows comes from the reference: {f}");
        assert!(f["prompt"].as_str().unwrap_or("").is_empty(), "用法 is the author's alone");
        // saved again after the author wrote his own lines: they stay
        let fid = v["figure"]["id"].as_str().unwrap().to_string();
        let mut item = v["figure"].clone();
        item["fields"]["connects"] = json!("Input -> Model");
        item["fields"]["prompt"] = json!("Use for a plain pipeline.");
        let (s, v2) = call(&e.app, "PUT", &format!("/api/hub/items/{fid}"), Some(item)).await;
        assert_eq!(s, StatusCode::OK, "{v2}");
        let (_, v3) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        assert_eq!(v3["figure"]["fields"]["connects"], "Input -> Model");
        assert_eq!(v3["figure"]["fields"]["prompt"], "Use for a plain pipeline.");
    }

    #[tokio::test]
    async fn a_made_figure_carries_how_it_was_commanded() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let f = &v["figure"]["fields"];
        let c = f["commands"].as_str().unwrap();
        assert!(c.contains("# 1 積木") && c.contains("# 3 串起來\nx -> f -> y"), "{c}");
        assert!(f["spoken"].as_str().unwrap().contains("Input 接到 Model。"), "{f}");
        assert!(f["prompt"].as_str().unwrap_or("").is_empty());
    }

    #[tokio::test]
    async fn commands_given_with_the_save_are_kept() {
        let e = setup(vec![]);
        let body = json!({ "spec": spec(SPEC), "commands": "  first x -> f  ", "spoken": "先接起來。" });
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["figure"]["fields"]["commands"], "first x -> f");
        assert_eq!(v["figure"]["fields"]["spoken"], "先接起來。");
    }

    #[tokio::test]
    async fn a_resave_keeps_what_the_author_edited() {
        let e = setup(vec![]);
        let (_, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        let fid = v["figure"]["id"].as_str().unwrap().to_string();
        let mut item = v["figure"].clone();
        item["fields"]["commands"] = json!("# 1 積木\nmine");
        item["fields"]["spoken"] = json!("我的說法。");
        let (s, _) = call(&e.app, "PUT", &format!("/api/hub/items/{fid}"), Some(item)).await;
        assert_eq!(s, StatusCode::OK);
        let (_, v2) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        assert_eq!(v2["figure"]["fields"]["commands"], "# 1 積木\nmine");
        assert_eq!(v2["figure"]["fields"]["spoken"], "我的說法。");
    }

    #[tokio::test]
    async fn a_timeline_spec_is_saved_as_a_figure() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f02", Some(json!({ "spec": timeline_spec(two_points()) }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let fig = &v["figure"];
        let fid = fig["id"].as_str().unwrap().to_string();
        assert!(fid.starts_with("t.flowspecs:"), "{fid}");
        assert_eq!((fig["entity"].clone(), fig["level"].as_str(), fig["fields"]["what"].as_str()), (json!(true), Some("figure"), Some("spec")));
        assert_eq!(fig["fields"]["made_from"], "t.flowrefs:f02");
        assert_eq!(fig["fields"]["params"]["timeline"]["points"].as_array().unwrap().len(), 2);
        assert!(v["reference"]["contains"].as_array().unwrap().contains(&json!(fid)));
        // drawn
        let req = Request::builder().uri(format!("/api/hub/thumb/{fid}")).body(Body::empty()).unwrap();
        assert_eq!(e.app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
        // the editor re-opens on it
        let (_, g) = call(&e.app, "GET", "/api/hub/make/t.flowrefs:f02", None).await;
        assert_eq!(g["made"], json!(fid));
        assert_eq!(g["spec"]["timeline"]["title"], "Release history");
    }

    #[tokio::test]
    async fn an_empty_timeline_is_refused_with_one_line() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f02", Some(json!({ "spec": timeline_spec(json!([])) }))).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        let line = v["error"].as_str().unwrap();
        assert!(!line.is_empty() && !line.contains('\n'), "{line}");
        let (_, l) = call(&e.app, "GET", "/api/hub/items?category=t.flowspecs", None).await;
        assert!(l["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_flow_reference_is_still_drafted_as_before() {
        let e = setup(vec![kind("flow"), graph_reply()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["spec"]["dsl"].as_str().unwrap().contains("Model"));
        assert!(v["spec"].get("timeline").is_none_or(|t| t.is_null()), "{v}");
        let seen = e.mock.seen.lock().unwrap();
        let schema = serde_json::to_string(&seen[1].json_schema).unwrap();
        assert!(schema.contains("nodes") && !schema.contains("points"), "{schema}");
    }
    /// The regression this guards: seven of the author's twelve timeline references had no
    /// "時間軸" / "timeline" in their words and were drawn as boxes and arrows.
    #[tokio::test]
    async fn the_model_not_the_title_says_what_is_a_timeline() {
        // f01's words say nothing of time; the model looked at its picture and says timeline
        let e = setup(vec![kind("timeline"), timeline_reply()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["kind"], "timeline");
        assert_eq!(v["spec"]["timeline"]["points"].as_array().unwrap().len(), 2, "{v}");
        {
            let seen = e.mock.seen.lock().unwrap();
            assert!(serde_json::to_string(&seen[0].messages).unwrap().contains("image/png"), "the question carries the picture");
        }
        // f02's words say "timeline"; the model says it is a flow, and a flow it is
        let e = setup(vec![kind("flow"), graph_reply()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f02/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["kind"], "flow");
        assert!(v["spec"]["dsl"].as_str().unwrap().contains("Model"), "{v}");
    }

    #[tokio::test]
    async fn an_answer_that_is_neither_kind_is_one_line_and_no_draft() {
        let e = setup(vec![json!({ "kind": "poster", "why": "" }).to_string(), graph_reply()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/draft", None).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        // the question may be asked again once; no draft request (the one whose schema has `nodes`) was sent on a guess
        assert!(e.mock.seen.lock().unwrap().iter().all(|r| !serde_json::to_string(&r.json_schema).unwrap().contains("nodes")));
    }
}
