//! 繪圖 (lore-plot, `docs/plot.md` §8) in a paper.
//!
//! | Method | Path | |
//! |---|---|---|
//! | * | /plot/… | `lore_plot::router`: the editor's files, styles, data import, export, 自動 |
//! | GET | /api/papers/{name}/plot?file=figures/x.pdf | the spec saved next to that figure (404: not made here) |
//! | GET | /api/papers/{name}/plot/list | `[{file, spec}]`: figures/*.plot.json (pulled into a composite) |
//! | GET | /api/papers/{name}/plot/notation | the paper's notation entries (equation figures take each variable's colour role from them) |
//! | GET | /api/papers/{name}/plot/macros | the paper's notation macros for KaTeX (`{"\\vw": "\\boldsymbol{w}"}`) |
//! | POST | /api/papers/{name}/plot | `{stem, replace, spec, svg}` → `figures/<stem>.pdf` + `.plot.json` → `{file, stem}` |
//!
//! A PDF in figures/ with no `.plot.json` next to it is the author's own file and is never
//! overwritten: a new figure, or `replace` of such a file, gets a free name. The LaTeX block
//! is inserted by the page through the same path as 新增圖… (label, `\ref`, notation checks).

use super::*;

use lore_plot::figure;

/// The model calls of 自動 (task `plot-propose`), through this app's routing.
struct PlotLlm(Shared);

#[async_trait::async_trait]
impl lore_plot::propose::Llm for PlotLlm {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(serde_json::Value, String), String> {
        let route = self.0.llm.route(task, None).await?;
        let req = route.apply(req);
        let mut v = lore_llm::complete_json(route.provider.as_ref(), &req).await.map_err(|e| format!("模型沒有回答：{e}"))?;
        // a drafted caption keeps to the author's punctuation rule (mechanical fixes; off by default)
        if let Some(c) = v.get("caption").and_then(|c| c.as_str()).map(str::to_string) {
            let g = crate::workspace_guard(&self.0).await;
            v["caption"] = serde_json::Value::String(g.fix(&c));
        }
        let id = route.provider.id();
        lore_llm::shared::record_used(task, &id);
        Ok((v, id))
    }
}

pub(crate) fn routes(state: &Shared) -> Router<Shared> {
    let plot = lore_plot::Plot::new(loreforge_home()).with_llm(Arc::new(PlotLlm(state.clone())));
    Router::new()
        .route("/api/papers/{name}/plot", get(spec_get).post(save).layer(axum::extract::DefaultBodyLimit::max(60 * 1024 * 1024)))
        .route("/api/papers/{name}/plot/macros", get(macros))
        .route("/api/papers/{name}/plot/notation", get(notation_entries))
        .route("/api/papers/{name}/plot/list", get(list))
        .merge(lore_plot::router(plot))
}

#[derive(Deserialize)]
struct FileQ {
    file: String,
}

/// `figures/x.pdf` inside the paper folder (no `..`, no absolute path).
fn figure_path(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() || rel.starts_with('/') || rel.contains("..") || rel.contains('\\') {
        return None;
    }
    let p = dir.join(rel);
    let p = if p.extension().is_none() { p.with_extension("pdf") } else { p };
    Some(p)
}

async fn spec_get(State(s): State<Shared>, AxPath(name): AxPath<String>, AxQuery(q): AxQuery<FileQ>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(p) = figure_path(&dir, &q.file) else { return bad("圖檔路徑不對") };
    match figure::spec_next_to(&p) {
        Some(spec) => Json(serde_json::json!({ "spec": spec, "file": q.file })).into_response(),
        None => (StatusCode::NOT_FOUND, "這張圖不是繪圖工具做的").into_response(),
    }
}

async fn macros(State(s): State<Shared>, AxPath(name): AxPath<String>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return Json(serde_json::json!({})).into_response() };
    let n = crate::notation::load(&ws, &dir);
    let m: serde_json::Map<String, serde_json::Value> = lore_notation::katex_macros(&n).into_iter().map(|(k, v)| (k, v.into())).collect();
    Json(serde_json::Value::Object(m)).into_response()
}

/// The paper's notation (the same registry as its macros): equation figures read colour roles from it.
async fn notation_entries(State(s): State<Shared>, AxPath(name): AxPath<String>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(ws) = s.loaded.read().await.workspace.clone() else { return Json(serde_json::json!([])).into_response() };
    let n = crate::notation::load(&ws, &dir);
    Json(serde_json::to_value(&n.entries).unwrap_or_default()).into_response()
}

async fn list(State(s): State<Shared>, AxPath(name): AxPath<String>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir.join("figures")) {
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(".plot.json")).collect();
        files.sort();
        for f in files {
            let Some(spec) = std::fs::read_to_string(&f).ok().and_then(|t| lore_plot::Spec::from_json(&t).ok()) else { continue };
            let stem = f.file_name().and_then(|n| n.to_str()).unwrap_or("").trim_end_matches(".plot.json").to_string();
            out.push(serde_json::json!({ "file": format!("figures/{stem}.pdf"), "spec": spec }));
        }
    }
    Json(serde_json::Value::Array(out)).into_response()
}

#[derive(Deserialize)]
struct SaveBody {
    /// Wanted file name (without extension), e.g. from the caption.
    #[serde(default)]
    stem: String,
    /// `figures/x.pdf` being re-edited: written in place when this tool made it.
    #[serde(default)]
    replace: Option<String>,
    spec: serde_json::Value,
    svg: String,
}

async fn save(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<SaveBody>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let spec = match lore_plot::Spec::from_value(b.spec) {
        Ok(sp) => sp,
        Err(e) => return bad(&e.to_string()),
    };
    if let Err(e) = spec.validate() {
        return bad(&e.to_string());
    }
    let figs = dir.join("figures");
    // re-edit: the same file when it lives in figures/ (else a new one there)
    let (stem, replace) = match b.replace.as_deref().and_then(|r| figure_path(&dir, r)) {
        Some(p) if p.parent() == Some(figs.as_path()) => (p.file_stem().and_then(|x| x.to_str()).unwrap_or("figure").to_string(), true),
        _ => (if b.stem.trim().is_empty() { "figure".to_string() } else { b.stem.clone() }, false),
    };
    let tools = lore_plot::Tools::find();
    let svg = b.svg;
    let res = tokio::task::spawn_blocking(move || figure::save_paper_figure(&tools, &figs, &stem, replace, &spec, &svg)).await;
    match res {
        Ok(Ok(saved)) => Json(serde_json::json!({ "file": format!("figures/{}", saved.file), "stem": saved.stem })).into_response(),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figure_paths_stay_inside_the_paper() {
        let d = Path::new("/p");
        assert_eq!(figure_path(d, "figures/a.pdf"), Some(PathBuf::from("/p/figures/a.pdf")));
        assert_eq!(figure_path(d, "figures/a"), Some(PathBuf::from("/p/figures/a.pdf")));
        assert_eq!(figure_path(d, "../x.pdf"), None);
        assert_eq!(figure_path(d, "/etc/x.pdf"), None);
        assert_eq!(figure_path(d, ""), None);
    }
}
