//! 流程圖 in a paper or a book (lore-flow's shared editor, `docs/flow-figures.md` §6): the same
//! path as 新增圖… and 繪圖….
//!
//! | Method | Path | |
//! |---|---|---|
//! | * | /flow/… | `lore_flow::web::router`: the editor, steps, layouts, previews, 助手 (`doc` = the paper → its notation) |
//! | GET | /api/papers/{name}/flow?file=figures/x.pdf | the spec saved next to that figure (404: not made here) |
//! | GET | /api/papers/{name}/flow/source | `{text}`: the starting material — title, abstract and the method section(s) |
//! | POST | /api/papers/{name}/flow | `{stem, replace, spec, svg}` → `figures/<stem>.pdf` + `<stem>.flow.json` → `{file, stem, registered}` |
//!
//! The SVG comes from the page (`LoreFlow.paperSvg`: the figure at print size with TeX glyphs);
//! the PDF is printed from it like 繪圖's. A PDF in figures/ without a `.flow.json` next to it is the
//! author's own (or 繪圖's) and is never overwritten. New variables of the figure join the paper's
//! own `notation.json` (tag `flow-draft`), never the global registry or `notation.tex`. The LaTeX
//! block is inserted by the page through `figure-block` (floats.rs): label, `\ref`, checks.

use super::*;

use lore_flow::figure::FlowSpec;

/// The model calls of the discussion and 助手, through this app's routing.
struct FlowLlm(Arc<Llm>);

#[async_trait::async_trait]
impl lore_flow::llm::Llm for FlowLlm {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(serde_json::Value, String), String> {
        let route = self.0.route(task, None).await?;
        let req = route.apply(req);
        let v = lore_llm::complete_json(route.provider.as_ref(), &req).await.map_err(|e| format!("模型沒有回答：{e}"))?;
        let id = route.provider.id();
        lore_llm::shared::record_used(task, &id);
        Ok((v, id))
    }
}

/// SVG → PDF bytes. Chrome through lore-plot by default; tests put their own in.
pub type PdfMaker = dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync;
static PDF: std::sync::OnceLock<Arc<PdfMaker>> = std::sync::OnceLock::new();

/// Tests: make PDFs without a browser.
pub fn use_pdf_maker(f: Arc<PdfMaker>) {
    let _ = PDF.set(f);
}

fn make_pdf(svg: &str) -> Result<Vec<u8>, String> {
    match PDF.get() {
        Some(f) => f(svg),
        None => lore_plot::export::export(&lore_plot::Tools::find(), svg, None, lore_plot::Format::Pdf, 0).map_err(|e| e.to_string()),
    }
}

/// The paper folder for a name, without awaiting (the notation provider of `/flow` is sync).
fn paper_dir_now(s: &Shared, name: &str) -> Option<PathBuf> {
    if !paper::safe_name(name) {
        return None;
    }
    let ws = s.loaded.try_read().ok().and_then(|l| l.workspace.clone());
    ws.map(|w| w.join("papers").join(name)).filter(|d| d.is_dir()).or_else(|| opened::external_dir(name).filter(|d| d.is_dir()))
}

fn paper_notation(s: &Shared, name: &str) -> Option<lore_notation::Notation> {
    let dir = paper_dir_now(s, name)?;
    let ws = s.loaded.try_read().ok().and_then(|l| l.workspace.clone())?;
    Some(crate::notation::load(&ws, &dir))
}

pub(crate) fn routes(state: &Shared) -> Router<Shared> {
    let home = loreforge_home();
    let s = state.clone();
    let mut flow = lore_flow::web::Flow::new(home.clone()).with_llm(Arc::new(FlowLlm(state.llm.clone()))).with_notation(Arc::new(move |doc: &str| paper_notation(&s, doc)));
    if let Some(d) = home.app_dir("PaperComposer") {
        flow = flow.with_app_dir(d.join("flow"));
    }
    Router::new()
        .route("/api/papers/{name}/flow", get(spec_get).post(save).layer(axum::extract::DefaultBodyLimit::max(60 * 1024 * 1024)))
        .route("/api/papers/{name}/flow/source", get(source))
        .merge(lore_flow::web::router(flow))
}

#[derive(Deserialize)]
struct FileQ {
    file: String,
}

/// `figures/x.pdf` inside the paper folder.
fn figure_path(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() || rel.starts_with('/') || rel.contains("..") || rel.contains('\\') {
        return None;
    }
    let p = dir.join(rel);
    Some(if p.extension().is_none() { p.with_extension("pdf") } else { p })
}

/// `a/x.pdf` → `a/x.flow.json`.
pub fn sidecar(figure: &Path) -> PathBuf {
    let stem = figure.file_stem().and_then(|s| s.to_str()).unwrap_or("figure");
    figure.with_file_name(format!("{stem}.flow.json"))
}

async fn spec_get(State(s): State<Shared>, AxPath(name): AxPath<String>, AxQuery(q): AxQuery<FileQ>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let Some(p) = figure_path(&dir, &q.file) else { return bad("圖檔路徑不對") };
    match std::fs::read_to_string(sidecar(&p)).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        Some(spec) => Json(serde_json::json!({ "spec": spec, "file": q.file })).into_response(),
        None => (StatusCode::NOT_FOUND, "這張圖不是流程圖工具做的").into_response(),
    }
}

/// Title, abstract and the method section(s) of a paper (or a book's chapters on the method):
/// what the discussion starts from. Bounded; plain text with `$…$` maths.
pub fn starting_material(dir: &Path) -> String {
    let from_docsrc = docsrc_material(dir);
    if !from_docsrc.trim().is_empty() {
        return from_docsrc;
    }
    // the paper's own fields and sections when lore-docsrc cannot read the project (a journal class)
    let main = std::fs::read_to_string(pc_core::open::main_file(dir)).unwrap_or_default();
    let fields = pc_core::front::fields_in(dir, &main);
    let field = |k: &str| fields.iter().find(|f| f.field == k).map(|f| pc_core::text::strip_latex(&f.value)).unwrap_or_default();
    let mut out = String::new();
    if !field("title").trim().is_empty() {
        out += &format!("Title: {}\n\n", field("title").trim());
    }
    if !field("abstract").trim().is_empty() {
        out += &format!("Abstract: {}\n\n", field("abstract").trim());
    }
    for sec in pc_core::open::sections(dir).iter().filter(|s| {
        let t = s.title.to_lowercase();
        ["method", "approach", "model", "framework", "pipeline", "materials", "方法"].iter().any(|w| t.contains(w))
    }) {
        out += &format!("{}\n{}\n\n", pc_core::text::strip_latex(&sec.title), sec.paragraphs.join("\n"));
    }
    out.chars().take(9000).collect::<String>().trim().to_string()
}

fn docsrc_material(dir: &Path) -> String {
    let Ok(cat) = lore_docsrc::latex::extract_dir(dir) else { return String::new() };
    let mut out = String::new();
    if !cat.meta.title.trim().is_empty() {
        out += &format!("Title: {}\n\n", cat.meta.title.trim());
    }
    if let Some(a) = cat.meta.abstract_text.as_deref().filter(|a| !a.trim().is_empty()) {
        out += &format!("Abstract: {}\n\n", a.trim());
    }
    let method = |t: &str| {
        let t = t.to_lowercase();
        ["method", "approach", "model", "framework", "pipeline", "architecture", "proposed", "方法"].iter().any(|w| t.contains(w))
    };
    let mut picked: Vec<&lore_docsrc::Section> = cat.sections.iter().filter(|s| s.level == 1 && method(&s.title)).collect();
    if let Some(top) = picked.first().map(|s| s.id.clone()) {
        // the method section's own subsections come with it
        picked.extend(cat.sections.iter().filter(|s| s.level > 1 && s.id.starts_with(&format!("{top}."))));
    }
    for s in picked {
        let body: String = s.paragraphs.join("\n");
        if body.trim().is_empty() {
            continue;
        }
        out += &format!("{}\n{}\n\n", s.title.trim(), body.trim());
        if out.chars().count() > 9000 {
            break;
        }
    }
    if !out.contains('\n') || !picked_any(&out, &cat) {
        // a book: its chapters (lore-docsrc reads sections only)
        out += &chapters_on_method(dir, &method);
    }
    let out: String = out.chars().take(9000).collect();
    out.trim().to_string()
}

fn picked_any(out: &str, cat: &lore_docsrc::AssetCatalog) -> bool {
    cat.sections.iter().any(|s| s.level == 1 && out.contains(&format!("{}\n", s.title.trim())))
}

/// `\chapter{…}` blocks whose title is about the method, as plain text (books).
fn chapters_on_method(dir: &Path, method: &dyn Fn(&str) -> bool) -> String {
    let text: String = pc_core::open::source_files(dir).iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect::<Vec<_>>().join("\n");
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("\\chapter") {
        let after = &rest[i..];
        let Some(open) = after.find('{') else { break };
        let Some(close) = after[open..].find('}').map(|c| open + c) else { break };
        let title = after[open + 1..close].to_string();
        let body_start = close + 1;
        let body_end = after[body_start..].find("\\chapter").map(|e| body_start + e).unwrap_or(after.len());
        let body = after[body_start..body_end].replace("\\end{document}", "");
        if method(&title) {
            out += &format!("{}\n{}\n\n", title.trim(), lore_docsrc::latex::plain::tidy(&body).trim());
        }
        rest = &after[body_end..];
        if out.chars().count() > 9000 {
            break;
        }
    }
    out
}

async fn source(State(s): State<Shared>, AxPath(name): AxPath<String>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let text = tokio::task::spawn_blocking(move || starting_material(&dir)).await.unwrap_or_default();
    Json(serde_json::json!({ "text": text })).into_response()
}

/// A free stem in `figs`: not a PDF, `.plot.json` or `.flow.json` there — or `stem` itself when
/// `replace` and the flow editor made it (its `.flow.json` is next to it).
pub fn free_stem(figs: &Path, stem: &str, replace: bool) -> String {
    let stem = lore_plot::figure::stem_of(stem);
    let ours = |s: &str| figs.join(format!("{s}.flow.json")).is_file();
    let taken = |s: &str| ["pdf", "plot.json", "flow.json"].iter().any(|e| figs.join(format!("{s}.{e}")).exists());
    if replace && ours(&stem) || !taken(&stem) {
        return stem;
    }
    (2..).map(|k| format!("{stem}-{k}")).find(|s| !taken(s)).expect("a free name")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("f"), std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Templates, themes and drawings: the public ones, `private/shared/flow`, `private/PaperComposer/flow`.
fn catalogue() -> lore_flow::figure::Catalogue {
    let home = loreforge_home();
    let dirs: Vec<PathBuf> = home.shared_dir().map(|d| d.join("flow")).into_iter().chain(home.app_dir("PaperComposer").map(|d| d.join("flow"))).collect();
    lore_flow::figure::Catalogue::load_all(&dirs.iter().map(|d| d.as_path()).collect::<Vec<_>>())
}

/// The figure's new variables into the paper's own registry (tag `flow-draft`); their ids.
pub fn register_vars(paper: &Path, spec: &FlowSpec) -> Result<Vec<String>, String> {
    let g = spec.graph(&catalogue().library)?;
    if g.vars_new.is_empty() {
        return Ok(vec![]);
    }
    let path = pc_core::paper::app_dir(paper).join("notation.json");
    let mut doc = lore_notation::Notation::load(&path).unwrap_or_default();
    let added = lore_flow::notation::register_new(&mut doc, &g.vars_new);
    if !added.is_empty() {
        doc.save(&path)?;
    }
    Ok(added)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Saved {
    pub file: String,
    pub stem: String,
    pub registered: Vec<String>,
}

/// Write `figures/<stem>.pdf` and `<stem>.flow.json`, register the new variables.
pub fn save_figure(paper: &Path, stem: &str, replace: Option<&str>, spec: &FlowSpec, svg: &str) -> Result<Saved, String> {
    let g = spec.graph(&catalogue().library)?;
    let p = spec.problems(&g);
    if !p.is_empty() {
        return Err(p.join("；"));
    }
    let figs = paper.join("figures");
    // re-edit: the same file when it lives in figures/ and this tool made it (else a new one)
    let (want, replace) = match replace.and_then(|r| figure_path(paper, r)) {
        Some(p) if p.parent() == Some(figs.as_path()) => (p.file_stem().and_then(|x| x.to_str()).unwrap_or("figure").to_string(), true),
        _ => (if stem.trim().is_empty() { g.fig.clone() } else { stem.to_string() }, false),
    };
    let stem = free_stem(&figs, &want, replace);
    let pdf = make_pdf(svg)?;
    write_atomic(&figs.join(format!("{stem}.pdf")), &pdf).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&spec.to_value()).map_err(|e| e.to_string())?;
    write_atomic(&figs.join(format!("{stem}.flow.json")), json.as_bytes()).map_err(|e| e.to_string())?;
    let registered = register_vars(paper, spec)?;
    Ok(Saved { file: format!("figures/{stem}.pdf"), stem, registered })
}

#[derive(Deserialize)]
struct SaveBody {
    #[serde(default)]
    stem: String,
    #[serde(default)]
    replace: Option<String>,
    spec: serde_json::Value,
    svg: String,
}

async fn save(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<SaveBody>) -> axum::response::Response {
    let dir = match paper_dir(&s, &name).await { Ok(d) => d, Err(r) => return r };
    let spec = match FlowSpec::from_value(b.spec) {
        Ok(sp) => sp,
        Err(e) => return bad(&e),
    };
    if lore_plot::export::size_mm(&b.svg).is_none() {
        return bad("圖沒有 mm 大小（要用 LoreFlow.paperSvg）");
    }
    let res = tokio::task::spawn_blocking(move || save_figure(&dir, &b.stem, b.replace.as_deref(), &spec, &b.svg)).await;
    match res {
        Ok(Ok(saved)) => Json(saved).into_response(),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_never_take_the_authors_or_the_plotting_tools_files() {
        let d = std::env::temp_dir().join(format!("pc-flow-stem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("method.pdf"), b"%PDF mine").unwrap();
        assert_eq!(free_stem(&d, "Method", true), "method-2", "the author's PDF");
        std::fs::write(d.join("bars.pdf"), b"%PDF").unwrap();
        std::fs::write(d.join("bars.plot.json"), b"{}").unwrap();
        assert_eq!(free_stem(&d, "bars", true), "bars-2", "繪圖's figure");
        std::fs::write(d.join("seg.pdf"), b"%PDF").unwrap();
        std::fs::write(d.join("seg.flow.json"), b"{}").unwrap();
        assert_eq!(free_stem(&d, "seg", true), "seg", "ours: replaced in place");
        assert_eq!(free_stem(&d, "seg", false), "seg-2");
        assert_eq!(sidecar(Path::new("/p/figures/seg.pdf")), PathBuf::from("/p/figures/seg.flow.json"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
