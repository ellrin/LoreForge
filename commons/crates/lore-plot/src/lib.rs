//! lore-plot — the shared plotting tool (繪圖, `docs/plot.md`).
//!
//! The figure is a JSON spec ([`Spec`]); `commons/ui/plot/plot.js` draws it as SVG in the
//! browser (the editor, `editor.js`, is the same drawing). This crate keeps the spec model,
//! data import, style presets (public + the author's private ones through lore-home), the
//! export pipeline (headless Chrome → PDF, poppler → SVG/PNG), the `plot-propose` LLM task
//! and the `/plot` routes an app mounts:
//!
//! ```ignore
//! let app = app.merge(lore_plot::router(lore_plot::Plot::new(home).with_llm(Arc::new(my_llm))));
//! ```
//!
//! Documents are the app's business: PaperComposer saves `figures/<stem>.pdf` + `.plot.json`
//! with [`figure::save_paper_figure`]; SlideComposer stores [`figure::slide_png`] (the spec
//! inside the PNG) as a deck asset.

pub mod assets;
pub mod data;
pub mod export;
pub mod figure;
pub mod propose;
pub mod spec;
pub mod style;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

pub use export::{Format, Tools};
pub use spec::{Panel, Series, Size, Spec, SpecError, Table};

/// What the `/plot` routes need from the app.
#[derive(Clone)]
pub struct Plot {
    pub home: lore_home::Home,
    pub tools: Arc<Tools>,
    pub llm: Option<Arc<dyn propose::Llm>>,
}

impl Plot {
    pub fn new(home: lore_home::Home) -> Plot {
        Plot { home, tools: Arc::new(Tools::find()), llm: None }
    }
    pub fn with_llm(mut self, llm: Arc<dyn propose::Llm>) -> Plot {
        self.llm = Some(llm);
        self
    }
    pub fn with_tools(mut self, tools: Tools) -> Plot {
        self.tools = Arc::new(tools);
        self
    }
}

pub fn bad(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

fn file(path: &str) -> Response {
    match assets::get(path) {
        Some((mime, body)) => ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache")], body).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct NameQ {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct ExportBody {
    svg: String,
    #[serde(default)]
    spec: Option<Value>,
    format: String,
    #[serde(default)]
    dpi: Option<u32>,
}

#[derive(Deserialize)]
struct ProposeBody {
    /// A list of tables, or the spec's `data` map.
    tables: Value,
    sentence: String,
    #[serde(default)]
    style: Option<String>,
    #[serde(default)]
    target: Option<String>,
}

/// Tables from a list or an id → table map.
pub fn tables_of(v: &Value) -> BTreeMap<String, Table> {
    match v {
        Value::Array(a) => a.iter().enumerate().filter_map(|(i, t)| Some((format!("t{}", i + 1), serde_json::from_value(t.clone()).ok()?))).collect(),
        Value::Object(o) => o.iter().filter_map(|(k, t)| Some((k.clone(), serde_json::from_value(t.clone()).ok()?))).collect(),
        _ => BTreeMap::new(),
    }
}

/// Run an export off the async threads.
pub async fn export_blocking(tools: Arc<Tools>, svg: String, spec_json: Option<String>, format: Format, dpi: u32) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || export::export(&tools, &svg, spec_json.as_deref(), format, dpi).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}

/// `/plot/` (the editor page and its files), `/plot/katex/*`, `/plot/api/{styles,import,export,propose}`.
pub fn router<S: Clone + Send + Sync + 'static>(p: Plot) -> axum::Router<S> {
    let p = Arc::new(p);
    let (ps, pe, pp) = (p.clone(), p.clone(), p.clone());
    axum::Router::new()
        .route("/plot", get(|| async { axum::response::Redirect::permanent("/plot/") }))
        .route("/plot/", get(|| async { file("index.html") }))
        .route("/plot/{*path}", get(|Path(path): Path<String>| async move { file(&path) }))
        .route(
            "/plot/api/styles",
            get(move || {
                let p = ps.clone();
                async move { Json(style::list(&p.home)) }
            }),
        )
        .route(
            "/plot/api/import",
            post(|Query(q): Query<NameQ>, body: Bytes| async move {
                let name = if q.name.is_empty() { "pasted.txt".to_string() } else { q.name };
                match tokio::task::spawn_blocking(move || data::import(&name, &body)).await {
                    Ok(Ok(tables)) => Json(json!({ "tables": tables })).into_response(),
                    Ok(Err(e)) => bad(StatusCode::BAD_REQUEST, e.to_string()),
                    Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                }
            })
            .layer(DefaultBodyLimit::max(60 * 1024 * 1024)),
        )
        .route(
            "/plot/api/export",
            post(move |Json(b): Json<ExportBody>| {
                let p = pe.clone();
                async move {
                    let Some(format) = Format::parse(&b.format) else { return bad(StatusCode::BAD_REQUEST, "格式只能是 pdf、svg、png") };
                    let spec_json = b.spec.map(|s| serde_json::to_string_pretty(&s).unwrap_or_default());
                    match export_blocking(p.tools.clone(), b.svg, spec_json, format, b.dpi.unwrap_or(300)).await {
                        Ok(bytes) => ([(header::CONTENT_TYPE, format.mime())], bytes).into_response(),
                        Err(e) => bad(StatusCode::UNPROCESSABLE_ENTITY, e),
                    }
                }
            })
            .layer(DefaultBodyLimit::max(60 * 1024 * 1024)),
        )
        .route(
            "/plot/api/propose",
            post(move |Json(b): Json<ProposeBody>| {
                let p = pp.clone();
                async move {
                    let Some(llm) = p.llm.clone() else { return bad(StatusCode::SERVICE_UNAVAILABLE, "這個 app 沒有接模型") };
                    let tables = tables_of(&b.tables);
                    if tables.is_empty() {
                        return bad(StatusCode::BAD_REQUEST, "先放資料");
                    }
                    let list = style::list(&p.home);
                    let styles: Vec<String> = list["styles"].as_array().into_iter().flatten().filter_map(|s| s["id"].as_str().map(str::to_string)).collect();
                    let current = b.style.filter(|s| styles.contains(s)).unwrap_or_else(|| list["default"].as_str().unwrap_or("clean").to_string());
                    let examples = style::private_dir(&p.home).map(|d| propose::examples(&d.join("examples"))).unwrap_or_default();
                    let ask = propose::Ask { tables: &tables, sentence: &b.sentence, styles: &styles, style: &current, target: b.target.as_deref(), examples: &examples };
                    match propose::propose(llm.as_ref(), &ask).await {
                        Ok((spec, engine)) => Json(json!({ "spec": spec, "engine": engine })).into_response(),
                        Err(e) => bad(StatusCode::UNPROCESSABLE_ENTITY, e),
                    }
                }
            }),
        )
}

#[cfg(test)]
mod tests;
