//! The shared flow-figure editor's routes (`docs/flow-figures.md`), mounted by an app
//! like lore-plot's `/plot/*`:
//!
//! ```ignore
//! let app = app.merge(lore_flow::web::router(lore_flow::web::Flow::new(home).with_llm(llm).with_notation(notation)));
//! ```
//!
//! | Route | |
//! |---|---|
//! | `GET /flow/`, `/flow/editor.js`, `/flow/editor.css`, `/flow/katex/*` | the editor (`commons/ui/flow/`, compiled in) |
//! | `GET /flow/api/catalogue` | steps, layout templates, themes, line kinds, block colours |
//! | `POST /flow/api/notation` | `{doc}` → the registry entries (id, LaTeX, meaning) |
//! | `POST /flow/api/preview` | `{spec, doc, width}` → `{svg, linear, blocks, nodes, lines, suggestions, problems, error}` |
//! | `POST /flow/api/layouts` | `{spec, doc, width}` → `{layouts: [{id, name, private, svg}]}` (the figure in every template that fits) |
//! | `POST /flow/api/discuss` | `{step, source, answers, doc}` → `{say, options, pick, question, rows?, engine}` |
//! | `POST /flow/api/graph` | `{source, answers, doc}` → `{dsl, linear, vars_new, findings, engine}` |
//! | `POST /flow/api/assist` | `{mode: adjust｜alternatives, spec, request, doc}` → `{spec, changes, note, question}` ｜ `{alternatives}` |
//! | `POST /flow/api/svg` | `{spec, doc}` → the figure as an SVG file |
//!
//! `doc` names the document whose notation registry is in force; the app resolves it
//! ([`Flow::with_notation`]). The author's own templates, themes, block colours,
//! examples and habits live in `private/shared/flow/` (lore-home).

use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use lore_notation::Notation;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::figure::{Catalogue, FlowSpec, PALETTE, linear, preview_svg, render};
use crate::llm::Llm;
use crate::spec::{EdgeKind, Graph};
use crate::text::NoMath;

/// Resolves a document id to its notation registry (global + document).
pub type NotationFn = dyn Fn(&str) -> Option<Notation> + Send + Sync;

/// What the `/flow` routes need from the app.
#[derive(Clone)]
pub struct Flow {
    pub home: Option<lore_home::Home>,
    pub llm: Option<Arc<dyn Llm>>,
    pub notation: Option<Arc<NotationFn>>,
    /// Overrides `private/shared/flow` (tests).
    pub dir: Option<PathBuf>,
    /// The app's own `flow/` folder (its templates, themes, drawings), read after the shared one.
    pub app_dir: Option<PathBuf>,
}

impl Flow {
    pub fn new(home: lore_home::Home) -> Flow {
        Flow { home: Some(home), llm: None, notation: None, dir: None, app_dir: None }
    }
    /// No private folder (tests, demos).
    pub fn bare() -> Flow {
        Flow { home: None, llm: None, notation: None, dir: None, app_dir: None }
    }
    pub fn with_llm(mut self, llm: Arc<dyn Llm>) -> Flow {
        self.llm = Some(llm);
        self
    }
    pub fn with_notation(mut self, f: Arc<NotationFn>) -> Flow {
        self.notation = Some(f);
        self
    }
    pub fn with_dir(mut self, dir: PathBuf) -> Flow {
        self.dir = Some(dir);
        self
    }
    pub fn with_app_dir(mut self, dir: PathBuf) -> Flow {
        self.app_dir = Some(dir);
        self
    }

    /// `private/shared/flow` (the author's templates, themes, colours, examples, habits).
    pub fn private_dir(&self) -> Option<PathBuf> {
        self.dir.clone().or_else(|| self.home.as_ref().and_then(|h| h.shared_dir()).map(|d| d.join("flow")))
    }
    pub fn catalogue(&self) -> Catalogue {
        let dirs: Vec<PathBuf> = self.private_dir().into_iter().chain(self.app_dir.clone()).collect();
        Catalogue::load_all(&dirs.iter().map(|d| d.as_path()).collect::<Vec<_>>())
    }
    pub fn preferences(&self) -> String {
        self.private_dir().and_then(|d| std::fs::read_to_string(d.join("assist.md")).ok()).unwrap_or_default()
    }
    /// Worked examples for the graph task: `examples/*.json` = `{ "description", "dsl" }`.
    pub fn examples(&self, lib: &crate::Library) -> Vec<(String, Graph)> {
        let Some(dir) = self.private_dir() else { return vec![] };
        let mut v: Vec<_> = std::fs::read_dir(dir.join("examples")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
        v.sort();
        v.iter()
            .filter_map(|p| {
                let j: Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
                let g = crate::dsl::parse_with(j["dsl"].as_str()?, &lib.kinds()).ok()?;
                Some((j["description"].as_str()?.to_string(), g))
            })
            .take(4)
            .collect()
    }
    /// Block colours: the author's `palette.json` (`[[name, fill, line], …]`) or the neutral list.
    pub fn palette(&self) -> Value {
        if let Some(v) = self.private_dir().and_then(|d| std::fs::read_to_string(d.join("palette.json")).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_array) {
            return v;
        }
        json!(PALETTE.iter().map(|(n, f, l)| json!([n, f, l])).collect::<Vec<_>>())
    }
    fn notation_of(&self, doc: &str) -> Option<Notation> {
        self.notation.as_ref().and_then(|f| f(doc))
    }
}

// ── the editor's files ───────────────────────────────────────────────

macro_rules! katex_fonts {
    ($($f:literal),* $(,)?) => { &[$(($f, include_bytes!(concat!("../../../ui/notation-panel/vendor/katex/fonts/", $f)) as &[u8])),*] };
}

/// `(path under /flow/, content type, bytes)`.
pub const FILES: &[(&str, &str, &[u8])] = &[
    ("index.html", "text/html; charset=utf-8", include_bytes!("../../../ui/flow/index.html")),
    ("editor.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/flow/editor.js")),
    ("editor.css", "text/css; charset=utf-8", include_bytes!("../../../ui/flow/editor.css")),
    ("katex/katex.min.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/notation-panel/vendor/katex/katex.min.js")),
    ("katex/katex.min.css", "text/css; charset=utf-8", include_bytes!("../../../ui/notation-panel/vendor/katex/katex.min.css")),
];

pub const KATEX_FONTS: &[(&str, &[u8])] = katex_fonts!(
    "KaTeX_AMS-Regular.woff2",
    "KaTeX_Caligraphic-Regular.woff2",
    "KaTeX_Fraktur-Regular.woff2",
    "KaTeX_Main-Bold.woff2",
    "KaTeX_Main-BoldItalic.woff2",
    "KaTeX_Main-Italic.woff2",
    "KaTeX_Main-Regular.woff2",
    "KaTeX_Math-BoldItalic.woff2",
    "KaTeX_Math-Italic.woff2",
    "KaTeX_SansSerif-Regular.woff2",
    "KaTeX_Script-Regular.woff2",
    "KaTeX_Size1-Regular.woff2",
    "KaTeX_Size2-Regular.woff2",
    "KaTeX_Size3-Regular.woff2",
    "KaTeX_Size4-Regular.woff2",
    "KaTeX_Typewriter-Regular.woff2",
);

/// A file by its path under `/flow/`.
pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(f) = path.strip_prefix("katex/fonts/") {
        return KATEX_FONTS.iter().find(|(n, _)| *n == f).map(|(_, b)| ("font/woff2", *b));
    }
    FILES.iter().find(|(p, _, _)| *p == path).map(|(_, m, b)| (*m, *b))
}

fn file(path: &str) -> Response {
    match asset(path) {
        Some((mime, body)) => ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache")], body).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub fn bad(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

// ── answers ──────────────────────────────────────────────────────────

/// Everything the page shows about a spec: the drawing, the per-block list, the
/// blocks / nodes / lines it has (for the colour and legend blocks), what is wrong.
pub fn preview(flow: &Flow, spec: &FlowSpec, doc: &str, width: f64) -> Value {
    let cat = flow.catalogue();
    let g = match spec.graph(&cat.library) {
        Ok(g) => g,
        Err(e) => return json!({ "error": e }),
    };
    let notation = flow.notation_of(doc);
    let (fig, g2) = match render(spec, &cat, notation.as_ref(), &NoMath, spec.frame_rect(), 2) {
        Ok(x) => x,
        Err(e) => return json!({ "error": e, "linear": linear(&g) }),
    };
    let (_, theme, _) = spec.prepare(&cat, spec.frame_rect()).unwrap_or_else(|_| (g.clone(), cat.theme(&spec.theme), Default::default()));
    let lines: Vec<Value> = g
        .line_kinds()
        .into_iter()
        .map(|k| {
            let e = theme.edge(k);
            json!({ "id": k.as_str(), "label": k.label(), "color": e.color, "dash": e.dash, "width": e.width, "meaning": spec.lines.get(k.as_str()).map(|l| l.meaning.clone()).unwrap_or_default() })
        })
        .collect();
    json!({
        "svg": preview_svg(&fig, width),
        "linear": linear(&g),
        "blocks": g2.panels.iter().map(|p| json!({ "id": p.id, "title": p.title, "dir": p.dir })).collect::<Vec<_>>(),
        "nodes": g.nodes.iter().map(|n| json!({ "id": n.id, "label": n.label, "module": n.module, "panel": n.panel })).collect::<Vec<_>>(),
        "lines": lines,
        "suggestions": spec.legend_suggestions(&g),
        "problems": spec.problems(&g),
        "warnings": fig.built.warnings,
        "size": [fig.layout.bounds.w, fig.layout.bounds.h],
    })
}

/// The figure drawn in every layout template that fits its block count.
pub fn layouts(flow: &Flow, spec: &FlowSpec, doc: &str, width: f64) -> Value {
    let cat = flow.catalogue();
    let Ok(g) = spec.graph(&cat.library) else { return json!({ "layouts": [] }) };
    let notation = flow.notation_of(doc);
    let out: Vec<Value> = cat
        .layouts
        .iter()
        .filter(|l| l.fits(g.panels.len()))
        .filter_map(|l| {
            let mut s = spec.clone();
            s.layout = l.id.clone();
            // the template decides the directions here
            s.dirs.clear();
            let (fig, _) = render(&s, &cat, notation.as_ref(), &NoMath, s.frame_rect(), 2).ok()?;
            Some(json!({ "id": l.id, "name": l.name, "private": l.private, "svg": preview_svg(&fig, width) }))
        })
        .collect();
    json!({ "layouts": out })
}

#[derive(Deserialize)]
struct SpecBody {
    spec: Value,
    #[serde(default)]
    doc: String,
    #[serde(default)]
    width: Option<f64>,
}

#[derive(Deserialize)]
struct DiscussBody {
    #[serde(default)]
    step: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    answers: Value,
    #[serde(default)]
    doc: String,
}

#[derive(Deserialize)]
struct AssistBody {
    #[serde(default)]
    mode: String,
    spec: Value,
    #[serde(default)]
    request: String,
    #[serde(default)]
    doc: String,
}

#[derive(Deserialize)]
struct DocBody {
    #[serde(default)]
    doc: String,
}

fn spec_of(v: Value) -> Result<FlowSpec, Response> {
    FlowSpec::from_value(v).map_err(|e| bad(StatusCode::BAD_REQUEST, e))
}

fn catalogue_json(flow: &Flow) -> Value {
    let cat = flow.catalogue();
    let th = crate::theme::Theme::classic_bw();
    json!({
        "steps": crate::discuss::STEPS.iter().map(|s| json!({ "id": s.0, "title": s.1 })).collect::<Vec<_>>(),
        "layouts": cat.layouts,
        "themes": cat.themes.iter().map(|t| json!({ "id": t.0, "name": t.1 })).collect::<Vec<_>>(),
        "lines": EdgeKind::ALL.iter().map(|k| { let e = th.edge(*k); json!({ "id": k.as_str(), "label": k.label(), "color": e.color, "dash": e.dash, "width": e.width }) }).collect::<Vec<_>>(),
        "palette": flow.palette(),
        "llm": flow.llm.is_some(),
    })
}

/// `/flow/` (the editor and its files) and `/flow/api/*`.
pub fn router<S: Clone + Send + Sync + 'static>(flow: Flow) -> axum::Router<S> {
    let f = Arc::new(flow);
    let (f1, f2, f3, f4, f5, f6, f7, f8) = (f.clone(), f.clone(), f.clone(), f.clone(), f.clone(), f.clone(), f.clone(), f.clone());
    axum::Router::new()
        .route("/flow", get(|| async { axum::response::Redirect::permanent("/flow/") }))
        .route("/flow/", get(|| async { file("index.html") }))
        .route("/flow/{*path}", get(|Path(path): Path<String>| async move { file(&path) }))
        .route("/flow/api/catalogue", get(move || { let f = f1.clone(); async move { Json(catalogue_json(&f)) } }))
        .route(
            "/flow/api/notation",
            post(move |Json(b): Json<DocBody>| {
                let f = f2.clone();
                async move {
                    let entries: Vec<Value> = f.notation_of(&b.doc).map(|n| n.entries.iter().filter(|e| e.latex.is_some()).map(|e| json!({ "id": e.id, "latex": e.latex, "meaning": e.meaning.clone().or(e.meaning_en.clone()).unwrap_or_default() })).collect()).unwrap_or_default();
                    Json(json!({ "entries": entries }))
                }
            }),
        )
        .route(
            "/flow/api/preview",
            post(move |Json(b): Json<SpecBody>| {
                let f = f3.clone();
                async move {
                    let spec = match spec_of(b.spec) { Ok(s) => s, Err(r) => return r };
                    let w = b.width.unwrap_or(900.0).clamp(100.0, 4000.0);
                    match tokio::task::spawn_blocking(move || preview(&f, &spec, &b.doc, w)).await {
                        Ok(v) => Json(v).into_response(),
                        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                    }
                }
            }),
        )
        .route(
            "/flow/api/layouts",
            post(move |Json(b): Json<SpecBody>| {
                let f = f4.clone();
                async move {
                    let spec = match spec_of(b.spec) { Ok(s) => s, Err(r) => return r };
                    let w = b.width.unwrap_or(180.0).clamp(60.0, 800.0);
                    match tokio::task::spawn_blocking(move || layouts(&f, &spec, &b.doc, w)).await {
                        Ok(v) => Json(v).into_response(),
                        Err(e) => bad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                    }
                }
            }),
        )
        .route(
            "/flow/api/discuss",
            post(move |Json(b): Json<DiscussBody>| {
                let f = f5.clone();
                async move {
                    let Some(llm) = f.llm.clone() else { return bad(StatusCode::SERVICE_UNAVAILABLE, "這個 app 沒有接模型") };
                    let n = f.notation_of(&b.doc);
                    match crate::discuss::turn(llm.as_ref(), &b.step, &b.source, &b.answers, n.as_ref(), &f.preferences()).await {
                        Ok((mut v, engine)) => {
                            v["engine"] = json!(engine);
                            Json(v).into_response()
                        }
                        Err(e) => bad(StatusCode::UNPROCESSABLE_ENTITY, e),
                    }
                }
            }),
        )
        .route(
            "/flow/api/graph",
            post(move |Json(b): Json<DiscussBody>| {
                let f = f6.clone();
                async move {
                    let Some(llm) = f.llm.clone() else { return bad(StatusCode::SERVICE_UNAVAILABLE, "這個 app 沒有接模型") };
                    let n = f.notation_of(&b.doc);
                    let cat = f.catalogue();
                    let examples = f.examples(&cat.library);
                    match crate::discuss::graph(llm.as_ref(), &b.source, &b.answers, n.as_ref(), &cat.library, &examples).await {
                        Ok(p) => Json(json!({
                            "dsl": p.dsl,
                            "linear": linear(&p.graph),
                            "vars_new": p.graph.vars_new,
                            "findings": p.notation.findings.iter().map(|x| json!({ "code": x.code, "message": x.message })).collect::<Vec<_>>(),
                            "engine": p.provider,
                        }))
                        .into_response(),
                        Err(e) => bad(StatusCode::UNPROCESSABLE_ENTITY, e),
                    }
                }
            }),
        )
        .route(
            "/flow/api/assist",
            post(move |Json(b): Json<AssistBody>| {
                let f = f7.clone();
                async move {
                    let Some(llm) = f.llm.clone() else { return bad(StatusCode::SERVICE_UNAVAILABLE, "這個 app 沒有接模型") };
                    let spec = match spec_of(b.spec) { Ok(s) => s, Err(r) => return r };
                    let cat = f.catalogue();
                    let r = if b.mode == "alternatives" {
                        let n = f.notation_of(&b.doc);
                        crate::assist::alternatives(llm.as_ref(), &cat, &spec, &b.request, &f.preferences(), n.as_ref(), 260.0).await
                    } else {
                        if b.request.trim().is_empty() {
                            return bad(StatusCode::BAD_REQUEST, "要改什麼？");
                        }
                        crate::assist::adjust(llm.as_ref(), &cat, &spec, &b.request, &f.preferences()).await
                    };
                    match r {
                        Ok(v) => Json(v).into_response(),
                        Err(e) => bad(StatusCode::UNPROCESSABLE_ENTITY, e),
                    }
                }
            }),
        )
        .route(
            "/flow/api/svg",
            post(move |Json(b): Json<SpecBody>| {
                let f = f8.clone();
                async move {
                    let spec = match spec_of(b.spec) { Ok(s) => s, Err(r) => return r };
                    let cat = f.catalogue();
                    let n = f.notation_of(&b.doc);
                    match render(&spec, &cat, n.as_ref(), &NoMath, spec.frame_rect(), 2) {
                        Ok((fig, _)) => {
                            let b = fig.layout.bounds;
                            ([(header::CONTENT_TYPE, "image/svg+xml")], preview_svg(&fig, b.w.max(1.0) * 4.0 / 3.0)).into_response()
                        }
                        Err(e) => bad(StatusCode::UNPROCESSABLE_ENTITY, e),
                    }
                }
            }),
        )
}
