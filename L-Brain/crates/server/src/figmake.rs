//! 製作: a 待製作 reference (a picture and a description of a flow figure the flow tool cannot draw
//! yet) is made in the shared flow editor, and saving turns it into a drawable figure item linked
//! to the reference (`docs/brain-hub.md` §2 rule 2, §4.2). Also the model adapter the editor at
//! `/flow` uses (L-Brain's lore-llm routing behind lore-flow's `Llm` trait).
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/hub/make/{id} | `{id, title, source, image, spec, made}`: what the editor starts from (409: not a reference) |
//! | POST | /api/hub/make/{id} | `{spec, title?}` → `{figure, reference}`; the spec is drawn once first (422 when it cannot be) |
//! | POST | /api/hub/make/{id}/draft | `{spec, engine, warnings}`: a first draft from the description (and the picture), written by the model as figure text and drawn once; stored nowhere |
//! | GET | /api/hub/make/{id}/pptx | the made figure as a one-slide .pptx of native objects (409: none made yet) |
//! | POST | /api/hub/make/{id}/open | writes that .pptx into the app's cache folder and asks the system to open it with L-Slide → `{path, opened, how}` |
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
        .route("/api/hub/make/{id}/pptx", get(pptx))
        .route("/api/hub/make/{id}/open", post(open_in_slide))
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
    /// Write 指揮文字 and 口語說明 again from the spec even when the figure has them (unless
    /// they come with this save).
    #[serde(default)]
    rederive: bool,
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
    // what was derived from the spec it had goes with that spec: a changed figure gets its
    // variables, connections, commands and spoken text afresh; what the author wrote stays
    lore_know::flowknow::forget_derived(&mut fig, &h.catalogue().library, if b.rederive { &["commands", "spoken"] } else { &[] });
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

/// The page a draft is made for, in the draft route's query: `?width=500&ratio=3:2`.
#[derive(Debug, Default, serde::Deserialize)]
struct DraftPage {
    width: Option<f64>,
    ratio: Option<String>,
}

impl DraftPage {
    fn format(&self) -> Option<lore_flow::plate::Format> {
        let (a, b) = self.ratio.as_deref()?.split_once(':')?;
        let (a, b): (u32, u32) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
        (a > 0 && b > 0).then(|| lore_flow::plate::Format::new(self.width.filter(|w| *w > 0.0).unwrap_or(500.0), (a, b)))
    }
}

async fn draft(State(s): State<Shared>, Path(id): Path<String>, axum::extract::Query(page): axum::extract::Query<DraftPage>) -> R {
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
    // the author's own registry (the file `/api/notation` serves); none when there is no file
    let registry = s.home.shared_file("notation.json").and_then(|f| lore_notation::Notation::load(&f).ok()).filter(|n| !n.entries.is_empty());
    let prompt = lore_flow::llm::FlowPrompt { description: &description, image, notation: registry.as_ref(), library: &cat.library, examples: &examples, page: page.format() };
    // the model writes the figure language itself; the program checked and drew it once
    let (p, shown) = lore_flow::llm::propose_text_with(&llm, &prompt, &know).await.map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &e))?;
    let spec = lore_flow::figure::FlowSpec { lore_flow: lore_flow::figure::VERSION, dsl: p.dsl, ..Default::default() };
    // what the model was told from his knowledge, each with the one line on why it was picked
    let told = json!({
        "modules": shown.modules.iter().map(|m| json!({ "kind": m.item.kind, "style": m.item.style, "title": m.item.title, "why": m.why })).collect::<Vec<_>>(),
        "examples": shown.examples.iter().map(|x| json!({ "id": x.item.id, "title": x.item.title, "why": x.why })).collect::<Vec<_>>(),
    });
    // how the draft sits on its page, and whether it was sent back once and the second kept
    let on_page = json!({ "text_pt": p.page_text_pt, "shape": p.shape });
    ok(json!({ "spec": spec.to_value(), "engine": p.provider, "told": told, "warnings": p.warnings, "looks": p.looks, "polished": p.polished, "page": on_page, "kind": "flow", "why": why }))
}

// ── the made figure as a slide ───────────────────────────────────────────────

const SLIDE_MARGIN_PT: f64 = 24.0;
const EMU_PER_PT: f64 = 12_700.0;

/// The made figure of reference `id` drawn on one slide: its title and the .pptx bytes. The slide
/// is the spec's `frame` (16:9, 960 × 540 pt when it has none); the figure is laid out inside
/// the frame less a margin, so it is scaled to fit, and centred.
fn figure_pptx(s: &Shared, id: &str) -> Result<(String, Vec<u8>), HubError> {
    let h = hub(s)?;
    let r = reference(&h, id)?;
    let cat = spec_category(&h)?;
    let fig = made_figure(&h, &cat, &r.id)?.ok_or_else(|| err(StatusCode::CONFLICT, "這一條還沒有做好的圖。"))?;
    let spec = lore_flow::figure::FlowSpec::from_value(spec_value(&fig)).map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &format!("圖的規格讀不了：{e}")))?;
    let (w, hh) = match spec.frame {
        Some([w, hh]) if w > 10.0 && hh > 10.0 => (w, hh),
        _ => (960.0, 540.0),
    };
    let inner = lore_flow::Rect::new(0.0, 0.0, w - 2.0 * SLIDE_MARGIN_PT, hh - 2.0 * SLIDE_MARGIN_PT);
    let (drawn, _) = lore_flow::figure::render(&spec, &h.catalogue(), None, &lore_flow::NoMath, inner, 2).map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &format!("畫不出來：{e}")))?;
    let b = drawn.layout.bounds;
    let dx = ((SLIDE_MARGIN_PT + (inner.w - b.w) / 2.0 - b.x) * EMU_PER_PT).round() as i64;
    let dy = ((SLIDE_MARGIN_PT + (inner.h - b.h) / 2.0 - b.y) * EMU_PER_PT).round() as i64;
    let mut objects = drawn.built.objects.clone();
    // top-level objects (connectors too) are in slide coordinates; group children are relative
    for o in &mut objects {
        o.frame.x += dx;
        o.frame.y += dy;
    }
    let pictures = drawn.built.pictures.clone();
    let bytes = lore_flow::pptx::single_slide_with(&objects, w, hh, &|p: &str| if pictures.iter().any(|x| x == p) { std::fs::read(p).ok() } else { None }).map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &format!("做不出 .pptx：{e}")))?;
    let title = if fig.title.trim().is_empty() { r.title.clone() } else { fig.title.clone() };
    Ok((title, bytes))
}

/// A file stem from a title: no path separators or control characters, never empty.
fn file_stem(title: &str, id: &str) -> String {
    let clean = |t: &str| t.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    let t = clean(title);
    let t = t.trim_matches('.').to_string();
    if t.is_empty() { clean(&id.replace(':', "-")) } else { t.chars().take(80).collect() }
}

/// `attachment; filename="ascii.pptx"; filename*=UTF-8''percent.pptx`
fn disposition(stem: &str) -> String {
    let ascii: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let ascii = if ascii.trim_matches(|c| c == '_' || c == ' ').is_empty() { "figure".to_string() } else { ascii };
    let mut enc = String::new();
    for b in format!("{stem}.pptx").bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { enc.push(b as char) } else { enc.push_str(&format!("%{b:02X}")) }
    }
    format!("attachment; filename=\"{ascii}.pptx\"; filename*=UTF-8''{enc}")
}

async fn pptx(State(s): State<Shared>, Path(id): Path<String>) -> Result<axum::response::Response, HubError> {
    use axum::http::{HeaderValue, header};
    let (title, bytes) = figure_pptx(&s, &id)?;
    let mut res = axum::response::Response::new(axum::body::Body::from(bytes));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/vnd.openxmlformats-officedocument.presentationml.presentation"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&disposition(&file_stem(&title, &id))).map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "檔名不能用。"))?);
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}

/// Writes the figure's .pptx into `<app dir>/figures/` (the app's own cache, never the space or
/// `private/`; a name from the title, never over a file) and asks the system to open it. L-Slide
/// registers .pptx as a document type (`apps/desktop/tauri.conf.json`, `RunEvent::Opened` in its
/// `main.rs`), so `open -a L-Slide <file>` opens it there as a deck bound to that file. When that
/// fails the file is revealed in Finder instead.
async fn open_in_slide(State(s): State<Shared>, Path(id): Path<String>) -> R {
    let (title, bytes) = figure_pptx(&s, &id)?;
    let dir = s.rem.dir.clone().ok_or_else(|| err(StatusCode::CONFLICT, "找不到這個 App 的資料夾，沒有地方放檔案。"))?.join("figures");
    std::fs::create_dir_all(&dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("資料夾建不起來：{e}")))?;
    let path = lore_store::free_path(&dir, &file_stem(&title, &id), "pptx");
    lore_store::write_atomic(&path, &bytes).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &format!("檔案寫不出來：{e}")))?;
    let p = path.to_string_lossy().to_string();
    let tried = s.rem.runner.run("open", &["-a", "L-Slide", &p]);
    let (opened, how) = match tried {
        Ok(o) if o.status == 0 => (true, "L-Slide".to_string()),
        _ => match s.rem.runner.run("open", &["-R", &p]) {
            Ok(o) if o.status == 0 => (false, "Finder".to_string()),
            _ => (false, "none".to_string()),
        },
    };
    ok(json!({ "path": p, "opened": opened, "how": how }))
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

    use super::EMU_PER_PT;
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
        json!({ "dsl": "fig pipe\nx: image \"Input\" col=0 lane=0\nf: box \"Model\" col=1 lane=0\ny: label \"Output\" col=2 lane=0\nx -> f : data color=blue\nf -> y : data color=blue\n", "vars_new": [] }).to_string()
    }

    struct Env {
        _dir: tempfile::TempDir,
        app: axum::Router,
        mock: Arc<MockProvider>,
        /// The LoreForge folder of the test (`private/shared/notation.json` goes under it).
        home: std::path::PathBuf,
    }

    fn write(p: &std::path::Path, text: impl AsRef<[u8]>) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn setup(replies: Vec<String>) -> Env {
        setup_with(replies, Arc::new(lore_store::SystemRunner))
    }

    fn setup_with(replies: Vec<String>, runner: Arc<dyn lore_store::CommandRunner>) -> Env {
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
        let rem = Reminders::isolated(&dir.path().join("rem"), runner, "http://127.0.0.1:7700");
        let mock = Arc::new(MockProvider::new(replies));
        let f = MockFactory::default().with("claude:sonnet", mock.clone());
        let state = HubState::isolated(&hubdir, MANIFEST).unwrap();
        // the same as AppState::with_hub, but with a home inside the temp dir: no real registry is read
        let home = lore_home::Home::at(dir.path().join("home"));
        let shared = Arc::new(AppState { kb: std::sync::Mutex::new(kb), web: config.web.clone(), rem, home, llm: Arc::new(Llm::with_factory(f)), hub: state.hub.map(std::sync::Mutex::new), inbox: None, hub_dirs: state.app_dirs, flow: state.flow });
        let app = crate::router_isolated(shared);
        Env { home: dir.path().join("home"), _dir: dir, app, mock }
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
        assert!(schema.contains("dsl") && !schema.contains("points"), "{schema}");
    }
    #[tokio::test]
    async fn a_flow_draft_is_written_as_text() {
        let dsl = "fig f\nx: image \"In\" col=0 lane=0\nm: box \"Model\" col=1 lane=0\nx -> m : data color=notacolour\n";
        let e = setup(vec![kind("flow"), json!({ "dsl": dsl, "vars_new": [] }).to_string()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["spec"]["dsl"], dsl);
        assert_eq!(v["kind"], "flow");
        assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("notacolour")), "{v}");
        let seen = e.mock.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].system.contains("align row") && seen[1].system.contains("texts=["));
    }

    #[tokio::test]
    async fn a_flow_draft_gets_the_notation_registry() {
        let e = setup(vec![kind("flow"), graph_reply()]);
        let shared = e.home.join("private/shared");
        std::fs::create_dir_all(&shared).unwrap();
        let reg = json!({ "version": 1, "entries": [ { "id": "w_daidai", "kind": "symbol", "latex": "\\omega_d", "meaning": "weight of the daidai run" } ] });
        std::fs::write(shared.join("notation.json"), reg.to_string()).unwrap();
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let seen = e.mock.seen.lock().unwrap();
        assert!(seen[1].system.contains("- w_daidai — $\\omega_d$ — weight of the daidai run"), "{}", seen[1].system);
        // with no registry file the table is empty
        let e = setup(vec![kind("flow"), graph_reply()]);
        call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/draft", None).await;
        assert!(e.mock.seen.lock().unwrap()[1].system.contains("(empty: every variable goes into vars_new)"));
    }

    #[tokio::test]
    async fn a_timeline_draft_is_unchanged() {
        let e = setup(vec![kind("timeline"), timeline_reply()]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f02/draft", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["kind"], "timeline");
        assert!(v["spec"].get("dsl").is_none_or(|d| d.is_null() || d == ""), "{v}");
        assert!(v.get("warnings").is_none(), "{v}");
        assert_eq!(e.mock.seen.lock().unwrap().len(), 2);
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
        assert!(e.mock.seen.lock().unwrap().iter().all(|r| !serde_json::to_string(&r.json_schema).unwrap().contains("dsl")));
    }
    /// Found by a review: a second save with a changed figure kept the first figure's commands,
    /// and the next draft was shown them as its example.
    #[tokio::test]
    async fn a_changed_figure_gets_its_commands_afresh() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC) }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["figure"]["fields"]["commands"].as_str().unwrap().contains("Model"));
        let other = "fig demo\nx: image \"Input\"\ng: box \"Other\"\nx -> g\n";
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(other) }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let c = v["figure"]["fields"]["commands"].as_str().unwrap();
        assert!(c.contains("Other") && !c.contains("Model"), "{c}");
        assert!(v["figure"]["fields"]["spoken"].as_str().unwrap().contains("Other"));
    }

    // ── the figure as a slide ──

    struct FakeRunner(std::sync::Mutex<Vec<Vec<String>>>, i32);
    impl lore_store::CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> std::io::Result<lore_store::CommandOutput> {
            let mut v = vec![program.to_string()];
            v.extend(args.iter().map(|a| a.to_string()));
            self.0.lock().unwrap().push(v);
            Ok(lore_store::CommandOutput { status: self.1, stdout: String::new(), stderr: String::new() })
        }
    }

    async fn get_raw(app: &axum::Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let res = app.clone().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let (s, h) = (res.status(), res.headers().clone());
        (s, h, res.into_body().collect().await.unwrap().to_bytes().to_vec())
    }

    async fn made(e: &Env, frame: Option<[f64; 2]>) {
        let mut sp = spec(SPEC);
        if let Some(f) = frame {
            sp["frame"] = json!(f);
        }
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": sp }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }

    fn read_slide(bytes: &[u8]) -> lore_canvas::Deck {
        lore_flow::pptx::read(bytes).unwrap()
    }

    fn conns(sl: &lore_canvas::Slide) -> Vec<&lore_canvas::Object> {
        sl.objects.iter().filter(|o| matches!(o.kind, lore_canvas::Kind::Connector(_))).collect()
    }

    #[tokio::test]
    async fn a_made_figure_downloads_as_a_pptx() {
        let e = setup(vec![]);
        made(&e, None).await;
        let (s, h, b) = get_raw(&e.app, "/api/hub/make/t.flowrefs:f01/pptx").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(h["content-type"], "application/vnd.openxmlformats-officedocument.presentationml.presentation");
        assert!(b.starts_with(b"PK"));
        let d = read_slide(&b);
        assert_eq!(d.slides.len(), 1);
        assert!(d.slides[0].objects.len() >= 5, "{}", d.slides[0].objects.len());
    }

    #[tokio::test]
    async fn a_reference_without_a_figure_has_no_pptx() {
        let e = setup(vec![]);
        let (s, _, b) = get_raw(&e.app, "/api/hub/make/t.flowrefs:f01/pptx").await;
        assert_eq!(s, StatusCode::CONFLICT);
        let v: Value = serde_json::from_slice(&b).unwrap();
        println!("{v}");
        assert!(!v.to_string().contains('\n'));
        assert_eq!(get_raw(&e.app, "/api/hub/make/t.flowrefs:nope/pptx").await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_file_name_follows_the_title() {
        let e = setup(vec![]);
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01", Some(json!({ "spec": spec(SPEC), "title": "訓練 loop/一" }))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let (_, h, _) = get_raw(&e.app, "/api/hub/make/t.flowrefs:f01/pptx").await;
        let d = h["content-disposition"].to_str().unwrap();
        assert!(d.starts_with("attachment; filename=\"____ loop_.pptx\"") || d.contains("filename=\""), "{d}");
        assert!(d.contains("filename*=UTF-8''%E8%A8%93%E7%B7%B4%20loop%20%E4%B8%80.pptx"), "{d}");
        assert!(d.is_ascii());
        assert_eq!(super::file_stem("", "t.x:f01"), "t.x-f01");
        assert_eq!(super::file_stem("Plain", "x"), "Plain");
    }

    #[tokio::test]
    async fn every_line_is_a_connector_attached_at_both_ends() {
        let e = setup(vec![]);
        made(&e, None).await;
        let (_, _, b) = get_raw(&e.app, "/api/hub/make/t.flowrefs:f01/pptx").await;
        let d = read_slide(&b);
        let sl = &d.slides[0];
        let cs = conns(sl);
        assert_eq!(cs.len(), 2, "x -> f -> y");
        fn all(os: &[lore_canvas::Object], out: &mut Vec<u32>) {
            for o in os {
                out.push(o.id);
                if let lore_canvas::Kind::Group(g) = &o.kind {
                    all(&g.children, out);
                }
            }
        }
        let mut ids = vec![];
        all(&sl.objects, &mut ids);
        for c in cs {
            let lore_canvas::Kind::Connector(k) = &c.kind else { unreachable!() };
            for a in [k.start.as_ref(), k.end.as_ref()] {
                let a = a.expect("attached");
                assert!(ids.contains(&a.id), "{a:?}");
            }
        }
    }

    #[tokio::test]
    async fn moving_a_block_moves_its_lines_end() {
        let e = setup(vec![]);
        made(&e, None).await;
        let (_, _, b) = get_raw(&e.app, "/api/hub/make/t.flowrefs:f01/pptx").await;
        let mut d = read_slide(&b);
        let sl = &mut d.slides[0];
        let c0 = conns(sl)[0].clone();
        let lore_canvas::Kind::Connector(k) = &c0.kind else { unreachable!() };
        let target = k.end.clone().unwrap();
        let before = lore_canvas::geom::connector_points(&c0.frame).1;
        let dy = (30.0 * EMU_PER_PT) as i64;
        sl.objects.iter_mut().find(|o| o.id == target.id).unwrap().frame.y += dy;
        let moved = std::collections::BTreeSet::from([target.id]);
        let fixed = lore_canvas::geom::reroute(sl, &moved);
        let nf = fixed.iter().find(|(id, _)| *id == c0.id).expect("the line follows").1.clone();
        let after = lore_canvas::geom::connector_points(&nf).1;
        assert!(((after.1 - before.1) - dy as f64).abs() < 2.0, "{before:?} {after:?}");
    }

    #[tokio::test]
    async fn the_slide_is_sized_to_the_figure() {
        let e = setup(vec![]);
        made(&e, Some([800.0, 300.0])).await;
        let (_, _, b) = get_raw(&e.app, "/api/hub/make/t.flowrefs:f01/pptx").await;
        let d = read_slide(&b);
        assert_eq!((d.size.w, d.size.h), ((800.0 * EMU_PER_PT) as i64, (300.0 * EMU_PER_PT) as i64));
        for o in &d.slides[0].objects {
            assert!(o.frame.x >= 0 && o.frame.y >= 0 && o.frame.x + o.frame.w <= d.size.w + 12700 && o.frame.y + o.frame.h <= d.size.h + 12700, "{} {:?}", o.id, o.frame);
        }
        let e2 = setup(vec![]);
        made(&e2, None).await;
        let (_, _, b2) = get_raw(&e2.app, "/api/hub/make/t.flowrefs:f01/pptx").await;
        let d2 = read_slide(&b2);
        assert_eq!((d2.size.w, d2.size.h), ((960.0 * EMU_PER_PT) as i64, (540.0 * EMU_PER_PT) as i64));
    }

    #[tokio::test]
    async fn open_writes_the_file_and_asks_the_system() {
        let fake = Arc::new(FakeRunner(Default::default(), 0));
        let e = setup_with(vec![], fake.clone());
        made(&e, None).await;
        let (s, v) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/open", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!((v["opened"].clone(), v["how"].clone()), (json!(true), json!("L-Slide")));
        let p = std::path::PathBuf::from(v["path"].as_str().unwrap());
        assert!(p.is_file() && std::fs::read(&p).unwrap().starts_with(b"PK"));
        assert!(p.starts_with(e._dir.path().join("rem")), "{p:?}");
        assert!(!p.starts_with(e._dir.path().join("personal")) && !p.starts_with(e._dir.path().join("hubdata")));
        assert_eq!(fake.0.lock().unwrap().clone(), vec![vec!["open".to_string(), "-a".into(), "L-Slide".into(), p.to_string_lossy().to_string()]]);
        // a command that fails: the file is shown in Finder instead
        let bad = Arc::new(FakeRunner(Default::default(), 1));
        let e2 = setup_with(vec![], bad.clone());
        made(&e2, None).await;
        let (_, v2) = call(&e2.app, "POST", "/api/hub/make/t.flowrefs:f01/open", None).await;
        assert_eq!((v2["opened"].clone(), v2["how"].clone()), (json!(false), json!("none")));
        assert_eq!(bad.0.lock().unwrap().len(), 2);
        assert_eq!(bad.0.lock().unwrap()[1][1], "-R");
    }

    #[tokio::test]
    async fn open_never_overwrites() {
        let fake = Arc::new(FakeRunner(Default::default(), 0));
        let e = setup_with(vec![], fake);
        made(&e, None).await;
        let (_, a) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/open", None).await;
        let (_, b) = call(&e.app, "POST", "/api/hub/make/t.flowrefs:f01/open", None).await;
        assert_ne!(a["path"], b["path"]);
        assert!(std::path::Path::new(a["path"].as_str().unwrap()).is_file() && std::path::Path::new(b["path"].as_str().unwrap()).is_file());
    }
}
