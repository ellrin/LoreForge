//! 繪圖 (lore-plot, `docs/plot.md` §8) on a slide.
//!
//! | Method | Path | |
//! |---|---|---|
//! | * | /plot/… | `lore_plot::router`: the editor's files, styles, data import, export, 自動, 助手 |
//! | POST | /api/decks/{name}/plot | `{spec, svg, dpi}` → a PNG (spec inside, `iTXt`) stored as a deck asset → `{asset, px: [w, h], mm: [w, h]}` |
//! | GET | /api/decks/{name}/plot?asset=… | the spec a picture carries (404: not a figure made here) |
//!
//! The picture is a PNG because the PPTX export has no SVG picture (no `svgBlip` + PNG fallback);
//! the spec rides inside the PNG, so it survives saving to and reopening from the .pptx.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::json;

use crate::{Shared, session};

/// 自動／助手 through this app's routing (the shared engine settings).
struct PlotLlm(Shared);

#[async_trait::async_trait]
impl lore_plot::propose::Llm for PlotLlm {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(serde_json::Value, String), String> {
        let a = crate::todeck::llm(&self.0).complete_json(task, req, None).await.map_err(|(m, _)| m)?;
        Ok((a.value, a.engine))
    }
}

/// The deck routes (state: the app's).
pub(crate) fn routes() -> axum::Router<Shared> {
    axum::Router::new().route("/api/decks/{name}/plot", get(spec_get).post(save).layer(axum::extract::DefaultBodyLimit::max(60 * 1024 * 1024)))
}

/// `/plot/*` (no app state needed).
pub(crate) fn plot_router(shared: &Shared) -> axum::Router {
    lore_plot::router(lore_plot::Plot::new(shared.home.clone()).with_llm(Arc::new(PlotLlm(shared.clone()))))
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

#[derive(Deserialize)]
struct AssetQ {
    asset: String,
}

async fn spec_get(State(s): State<Shared>, Path(name): Path<String>, Query(q): Query<AssetQ>) -> Response {
    let sess = match session(&s, &name) {
        Ok(x) => x,
        Err(_) => return err(StatusCode::NOT_FOUND, "找不到簡報"),
    };
    let bytes = sess.lock().unwrap().asset(&q.asset).map(|b| b.0);
    match bytes.as_deref().and_then(lore_plot::figure::spec_in_png) {
        Some(spec) => Json(json!({ "spec": spec })).into_response(),
        None => err(StatusCode::NOT_FOUND, "這張圖不是繪圖工具做的"),
    }
}

#[derive(Deserialize)]
struct SaveBody {
    spec: serde_json::Value,
    svg: String,
    #[serde(default)]
    dpi: Option<u32>,
}

async fn save(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<SaveBody>) -> Response {
    let sess = match session(&s, &name) {
        Ok(x) => x,
        Err(_) => return err(StatusCode::NOT_FOUND, "找不到簡報"),
    };
    let spec = match lore_plot::Spec::from_value(b.spec) {
        Ok(x) => x,
        Err(e) => return err(StatusCode::BAD_REQUEST, e.to_string()),
    };
    if let Err(e) = spec.validate() {
        return err(StatusCode::BAD_REQUEST, e.to_string());
    }
    let mm = lore_plot::export::size_mm(&b.svg);
    let dpi = b.dpi.unwrap_or(300).clamp(96, 600);
    let tools = lore_plot::Tools::find();
    let svg = b.svg;
    let png = match tokio::task::spawn_blocking(move || lore_plot::figure::slide_png(&tools, &spec, &svg, dpi)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return err(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let px = lore_plot::export::png_size(&png).unwrap_or((0, 0));
    let asset = match sess.lock().unwrap().add_asset(&png, "png") {
        Ok(a) => a,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    Json(json!({ "asset": asset, "px": [px.0, px.1], "mm": mm.map(|(w, h)| vec![w, h]) })).into_response()
}

