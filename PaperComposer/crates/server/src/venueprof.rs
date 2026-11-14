//! 期刊寫法 (docs/long-paper.md §D): a journal template's habits, learned from about five of the
//! journal's own recent open-access articles. The profile itself is pure code in
//! `pc_core::venueprofile`; this module finds the journal (OpenAlex source, confirmed once), runs
//! the background job (lore-jobs: `find → fetch → label → profile`), and serves the panel.
//!
//! Privacy: only the journal's name / ISSN and its OpenAlex id leave the machine. The paper's own
//! keywords rank the candidates here, locally; no key value, title, author name or e-mail is ever
//! sent (OpenAlex `mailto` stays off unless the author sets it). Downloads and extracted text live
//! in the cache tier (`<OS cache>/PaperComposer/venue/`, `PAPERCOMPOSER_CACHE` overrides); the
//! profile (no full text: titles, counts, shares, short phrases) goes to the private workspace's
//! `venues/<template>/`.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/venue/summaries | `{<template>: {made, stale, n, summary}}` for the template picker |
//! | GET | /api/venue/sources?q= | OpenAlex sources for a name or ISSN → `{sources}` |
//! | GET | /api/venue/t/{tpl} | the panel: `{template, identity, guess, submission, submission_stale, profile, stale, job, demo}` |
//! | PUT | /api/venue/t/{tpl}/identity | `{source}` (one of `/sources`) → the panel; written into template.toml `[venue]` |
//! | POST | /api/venue/t/{tpl}/make | `{paper?, kind?, engine?}` → the job object (`job` = id) |
//! | GET | /api/venue/t/{tpl}/skeleton | `{outline}`: sections in the journal's naming (for a new paper) |
//! | GET | /api/venue/t/{tpl}/prompt | `?section=` → `{section, outline, emphasis, checklist}` (the drafting hooks, for inspection) |
//! | GET | /api/venue/jobs/{id} | `?after=<seq>` long poll → job object |
//! | POST | /api/venue/jobs/{id}/stop · /resume | |
//! | POST | /api/venue/journals | `{source, base?}` → `{id, job}`: a journal template of its own on its base format, then its profile; 422 `{error, bases}` when no base is known |
//! | POST | /api/venue/paper/{name}/journal | `{template}`: move a paper onto a journal template (選期刊) |
//! | POST | /api/venue/requirements/apply | `[submission]` from `venues/requirements.toml` into the journal templates it lists → `{updated}` |

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use lore_jobs::{Job, Jobs, Pipeline, StageCx, StageError, StageFuture, StageSpec, status};
use lore_research::venue::{SourceInfo, VenueQuery, Work};
use pc_core::venueprofile::{self as vp, ArticleText, Identity, Labels, Submission};
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;
use crate::book::model::{Ask, BookModel, LlmModel};
use crate::book::source::Extract;

pub const TASK: &str = "pc-venue-profile";
/// Candidates asked from OpenAlex (many publisher PDF links answer with a web page).
const CANDIDATES: usize = 40;

// ── Dependencies (real or test) ──

pub struct Deps {
    /// This feature's cache root (`jobs/`, `pdf/`).
    pub cache: PathBuf,
    pub http: Arc<dyn lore_research::Http>,
    pub research: lore_research::Config,
    pub model: Arc<dyn BookModel>,
    pub extract: Arc<dyn Extract>,
}

/// PDF → sections without crops (the profile needs the text, headings and captions only).
pub struct TextExtract;

impl Extract for TextExtract {
    fn extract(&self, pdf: &Path, out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        let mut o = lore_docsrc::pdf::PdfOptions::new(out);
        o.render_crops = false;
        // Springer journals' headings are unnumbered, only larger
        o.unnumbered_headings = true;
        o.save = false;
        lore_docsrc::pdf::extract(pdf, &o).map_err(|e| format!("PDF 無法分析：{e}"))
    }
}

pub fn cache_root() -> PathBuf {
    std::env::var_os("PAPERCOMPOSER_CACHE").map(PathBuf::from).unwrap_or_else(|| dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer")).join("venue")
}

impl Deps {
    fn for_server(s: &Shared) -> Deps {
        let cfg = lore_research::Config::default();
        Deps {
            cache: cache_root(),
            http: Arc::new(lore_research::ReqwestHttp::new(&cfg.user_agent, cfg.timeout)),
            research: cfg,
            model: Arc::new(LlmModel(s.llm.clone())),
            extract: Arc::new(TextExtract),
        }
    }
    fn client(&self) -> lore_research::Client {
        let mut cfg = self.research.clone();
        cfg.retries = cfg.retries.min(2);
        cfg.pace.max_wait = cfg.pace.max_wait.min(Duration::from_secs(10));
        lore_research::Client::with_http(cfg, self.http.clone())
    }
}

static TEST_DEPS: OnceLock<Arc<Deps>> = OnceLock::new();

/// Tests of the HTTP routes: the jobs and the source search run on these (set before the first call).
pub fn use_deps(d: Arc<Deps>) {
    let _ = TEST_DEPS.set(d);
}

async fn jobs(s: &Shared) -> Arc<Jobs> {
    static JOBS: OnceLock<Arc<Jobs>> = OnceLock::new();
    if let Some(j) = JOBS.get() {
        return j.clone();
    }
    let deps = TEST_DEPS.get().cloned().unwrap_or_else(|| Arc::new(Deps::for_server(s)));
    JOBS.get_or_init(|| Jobs::open(deps.cache.join("jobs"), Arc::new(VenueJob { deps: deps.clone() }))).clone()
}

// ── The job ──

/// What a job is given (in its `job.json`; no session text).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Params {
    pub template: String,
    /// The private workspace (where the profile goes).
    pub workspace: PathBuf,
    pub identity: Identity,
    /// `article` or `review`.
    pub kind: String,
    /// The paper's own keywords: rank the candidates here, never sent.
    pub topic: Vec<String>,
    pub today: String,
    pub engine: Option<String>,
    /// The author's own PDFs of this journal (用手邊的 PDF): no OpenAlex, no download.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub local: Vec<LocalPdf>,
}

/// A PDF the author gave for the profile (kept in the cache's `pdf/local/`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LocalPdf {
    pub path: PathBuf,
    pub title: String,
}

pub const STAGES: &[StageSpec] = &[
    StageSpec { id: "find", label: "找文章", model: false, wait: false },
    StageSpec { id: "fetch", label: "下載與讀取", model: false, wait: false },
    StageSpec { id: "label", label: "標記段落", model: true, wait: false },
    StageSpec { id: "profile", label: "整理寫法", model: false, wait: false },
];

pub struct VenueJob {
    pub deps: Arc<Deps>,
}

fn params(cx: &StageCx) -> Result<Params, StageError> {
    serde_json::from_value(cx.params()).map_err(|e| StageError::new(400, format!("工作參數不正確：{e}")))
}

/// The copy's kind for the profile (`publisher`, `pmc`, `repository`, `arxiv`).
fn copy_kind(l: &lore_research::venue::Location) -> &'static str {
    if l.is_arxiv() {
        "arxiv"
    } else if l.is_pmc() {
        "pmc"
    } else if l.host_kind.as_deref() == Some("journal") {
        "publisher"
    } else {
        "repository"
    }
}

/// Candidates in order: the requested type, a copy to read, closest to the topic, newest.
pub fn choose(works: Vec<Work>, kind: &str, topic: &[String]) -> (Vec<Work>, HashMap<String, usize>) {
    let mut skipped: HashMap<String, usize> = HashMap::new();
    let mut keep = Vec::new();
    for w in works {
        if !w.kind.is_empty() && w.kind != kind {
            *skipped.entry("不是研究文章".into()).or_default() += 1;
        } else if w.pdf_copies().is_empty() {
            *skipped.entry("沒有開放的 PDF".into()).or_default() += 1;
        } else if !w.pdf_copies().iter().any(|l| l.license.is_some() || l.is_arxiv()) {
            *skipped.entry("沒有標明授權".into()).or_default() += 1;
        } else {
            keep.push(w);
        }
    }
    let keys: Vec<(String, Vec<String>, String)> = keep.iter().map(|w| (w.record.title.clone(), w.keywords.clone(), w.date.clone())).collect();
    let order = vp::rank(&keys, topic);
    let mut slots: Vec<Option<Work>> = keep.into_iter().map(Some).collect();
    (order.into_iter().filter_map(|i| slots[i].take()).collect(), skipped)
}

/// The extractor's catalog as the profile reads it.
pub fn article_of(cat: &lore_docsrc::AssetCatalog) -> Vec<vp::SecText> {
    cat.sections.iter().filter(|s| !s.paragraphs.is_empty() || s.level <= 1).map(|s| vp::SecText { level: s.level.max(1), title: s.title.clone(), paragraphs: s.paragraphs.clone() }).collect()
}

fn fill_assets(a: &mut ArticleText, cat: &lore_docsrc::AssetCatalog) {
    use lore_docsrc::AssetKind;
    for x in &cat.assets {
        let Some(num) = x.number.clone() else { continue };
        match x.kind {
            AssetKind::Figure | AssetKind::Table => a.captions.push(vp::Caption { kind: if x.kind == AssetKind::Figure { "figure" } else { "table" }.into(), number: num, text: x.caption.clone().unwrap_or_default() }),
            AssetKind::Equation => a.equations.push(x.section.as_ref().and_then(|sid| cat.sections.iter().filter(|s| !s.paragraphs.is_empty() || s.level <= 1).position(|s| &s.id == sid))),
            _ => {}
        }
    }
}

impl Pipeline for VenueJob {
    fn stages(&self) -> &'static [StageSpec] {
        STAGES
    }

    fn run<'a>(&'a self, cx: &'a StageCx, stage: &'static str) -> StageFuture<'a> {
        Box::pin(async move {
            let p = params(cx)?;
            match stage {
                "find" => self.find(cx, &p).await,
                "fetch" => self.fetch(cx, &p).await,
                "label" => self.label(cx, &p).await,
                _ => self.profile(cx, &p).await,
            }
        })
    }
}

fn net_err(e: lore_research::Error) -> StageError {
    StageError::new(if e.is_offline() { 503 } else { 502 }, e.to_string())
}

impl VenueJob {
    async fn find(&self, cx: &StageCx, p: &Params) -> Result<Value, StageError> {
        if !p.local.is_empty() {
            cx.counts(json!({ "found": p.local.len(), "candidates": p.local.len() }));
            return Ok(json!({ "candidates": [], "skipped": {}, "found": p.local.len(), "local": true }));
        }
        let c = self.deps.client();
        let q = VenueQuery { source_id: p.identity.openalex.clone(), from_date: vp::from_date(&p.today), kind: p.kind.clone(), per_page: CANDIDATES };
        let works = c.venue_works(&q).await.map_err(net_err)?;
        let found = works.len();
        let (cands, skipped) = choose(works, &p.kind, &p.topic);
        cx.counts(json!({ "found": found, "candidates": cands.len() }));
        if cands.is_empty() {
            let mut msg = if found == 0 { "OpenAlex 沒有這本期刊近兩年的開放取用文章".to_string() } else { "近兩年的開放取用文章都沒有可讀的 PDF 或授權".to_string() };
            if p.identity.kind == "conference" {
                msg.push_str("（研討會論文集多半不在 OpenAlex 的開放取用標示裡）");
            }
            return Err(StageError::new(404, msg));
        }
        Ok(json!({ "candidates": cands, "skipped": skipped, "found": found }))
    }

    async fn fetch(&self, cx: &StageCx, p: &Params) -> Result<Value, StageError> {
        if !p.local.is_empty() {
            return self.fetch_local(cx, p).await;
        }
        let cands: Vec<Work> = serde_json::from_value(cx.output("find")?["candidates"].clone()).unwrap_or_default();
        let mut skipped: HashMap<String, usize> = serde_json::from_value(cx.output("find")?["skipped"].clone()).unwrap_or_default();
        let c = self.deps.client();
        let pdf_dir = self.deps.cache.join("pdf");
        let art_dir = cx.dir().join("articles");
        std::fs::create_dir_all(&art_dir).map_err(|e| e.to_string())?;
        let mut kept: Vec<Value> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let mut tried = 0;
        'works: for w in &cands {
            if kept.len() >= vp::ARTICLES {
                break;
            }
            tried += 1;
            let mut last = "下載失敗";
            for loc in w.pdf_copies() {
                // the licence of the copy read; arXiv states its own
                let licence = if loc.is_arxiv() {
                    let id = w.record.arxiv.clone().or_else(|| loc.pdf_url.as_deref().and_then(lore_research::arxiv_from_url));
                    match id {
                        Some(id) => c.arxiv_license(&id).await.ok().flatten(),
                        None => None,
                    }
                } else {
                    loc.license.clone()
                };
                let Some(licence) = licence else {
                    last = "沒有標明授權";
                    continue;
                };
                let d = match c.download_copy(loc, &pdf_dir).await {
                    Ok(d) => d,
                    Err(e) if e.is_offline() => return Err(net_err(e)),
                    Err(_) => {
                        last = "下載的不是 PDF";
                        continue;
                    }
                };
                let ex = self.deps.extract.clone();
                let out = cx.dir().join("extract").join(&d.sha256);
                let pdf = d.path.clone();
                let cat = match tokio::task::spawn_blocking(move || ex.extract(&pdf, &out)).await {
                    Ok(Ok(cat)) => cat,
                    // no poppler: every PDF would fail the same way; say why and stop
                    Ok(Err(e)) if e.contains("找不到") || e.to_lowercase().contains("poppler") => return Err(StageError::new(500, e)),
                    Ok(Err(e)) => {
                        notes.push(format!("{}：{e}", w.record.doi.as_deref().unwrap_or(&w.record.title)));
                        last = "PDF 讀不出文字";
                        continue;
                    }
                    Err(e) => return Err(StageError::new(500, e.to_string())),
                };
                let mut a = ArticleText {
                    doi: w.record.doi.clone().unwrap_or_default(),
                    title: w.record.title.clone(),
                    year: w.record.year,
                    licence,
                    url: d.url.clone(),
                    copy: copy_kind(loc).into(),
                    sections: article_of(&cat),
                    ..Default::default()
                };
                fill_assets(&mut a, &cat);
                if !a.usable() {
                    last = "讀不出章節";
                    continue;
                }
                let file = art_dir.join(format!("{}.json", kept.len()));
                lore_jobs::write_json(&file, &a).map_err(|e| e.to_string())?;
                kept.push(json!({ "file": file.file_name().unwrap().to_string_lossy(), "doi": a.doi, "title": a.title, "licence": a.licence, "copy": a.copy }));
                cx.counts(json!({ "articles": kept.len(), "tried": tried }));
                continue 'works;
            }
            *skipped.entry(last.into()).or_default() += 1;
        }
        cx.counts(json!({ "articles": kept.len(), "tried": tried }));
        if kept.is_empty() {
            let why = notes.first().map(|n| format!("（{n}）")).unwrap_or_default();
            return Err(StageError::new(404, format!("沒有一篇開放取用文章能下載並讀出章節{why}")));
        }
        Ok(json!({ "articles": kept, "skipped": skipped, "notes": notes }))
    }

    /// 用手邊的 PDF: read the author's own copies (their licence is the author's to know).
    async fn fetch_local(&self, cx: &StageCx, p: &Params) -> Result<Value, StageError> {
        let art_dir = cx.dir().join("articles");
        std::fs::create_dir_all(&art_dir).map_err(|e| e.to_string())?;
        let mut kept: Vec<Value> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for (i, f) in p.local.iter().enumerate() {
            if kept.len() >= vp::ARTICLES + 1 {
                break;
            }
            let ex = self.deps.extract.clone();
            let out = cx.dir().join("extract").join(format!("local-{i}"));
            let pdf = f.path.clone();
            let cat = match tokio::task::spawn_blocking(move || ex.extract(&pdf, &out)).await {
                Ok(Ok(cat)) => cat,
                Ok(Err(e)) if e.contains("找不到") || e.to_lowercase().contains("poppler") => return Err(StageError::new(500, e)),
                Ok(Err(e)) => {
                    notes.push(format!("{}：{e}", f.title));
                    continue;
                }
                Err(e) => return Err(StageError::new(500, e.to_string())),
            };
            let mut a = ArticleText { title: f.title.clone(), licence: "author's copy".into(), copy: "local".into(), sections: article_of(&cat), ..Default::default() };
            fill_assets(&mut a, &cat);
            if !a.usable() {
                notes.push(format!("{}：讀不出章節", f.title));
                continue;
            }
            let file = art_dir.join(format!("{}.json", kept.len()));
            lore_jobs::write_json(&file, &a).map_err(|e| e.to_string())?;
            kept.push(json!({ "file": file.file_name().unwrap().to_string_lossy(), "doi": "", "title": a.title, "licence": a.licence, "copy": a.copy }));
            cx.counts(json!({ "articles": kept.len(), "tried": i + 1 }));
        }
        if kept.is_empty() {
            let why = notes.first().map(|n| format!("（{n}）")).unwrap_or_default();
            return Err(StageError::new(404, format!("這些 PDF 都讀不出章節{why}")));
        }
        Ok(json!({ "articles": kept, "skipped": {}, "notes": notes }))
    }

    fn articles(cx: &StageCx) -> Result<Vec<ArticleText>, StageError> {
        let list = cx.output("fetch")?;
        let mut v = Vec::new();
        for a in list["articles"].as_array().into_iter().flatten() {
            let f = cx.dir().join("articles").join(a["file"].as_str().unwrap_or(""));
            let t = std::fs::read_to_string(&f).map_err(|e| format!("讀不到下載的文章：{e}"))?;
            v.push(serde_json::from_str(&t).map_err(|e| e.to_string())?);
        }
        Ok(v)
    }

    async fn label(&self, cx: &StageCx, p: &Params) -> Result<Value, StageError> {
        let arts = Self::articles(cx)?;
        let mut known: Vec<vp::Category> = Vec::new();
        let mut engine = String::new();
        let mut base_chars = 0u64;
        for (i, a) in arts.iter().enumerate() {
            let file = cx.dir().join("articles").join(format!("{i}.labels.json"));
            // a labelled article is not asked again after a resume
            if let Some(l) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Labels>(&t).ok()) {
                known.extend(l.extra.iter().filter(|e| !known.iter().any(|k| k.id == e.id)).cloned().collect::<Vec<_>>());
                continue;
            }
            let (system, user, schema) = vp::label_request(a, &known);
            let on = |n: u64| cx.chars(base_chars + n);
            let (v, who) = self.deps.model.json(Ask { task: TASK, system, user, schema, max_tokens: 8000, engine: p.engine.as_deref() }, &on).await?;
            base_chars += v.to_string().chars().count() as u64;
            let l = vp::parse_labels(&v, a, &known);
            known.extend(l.extra.iter().cloned());
            engine = who;
            let _ = std::fs::write(cx.dir().join("articles").join("engine"), &engine);
            lore_jobs::write_json(&file, &l).map_err(|e| e.to_string())?;
            cx.counts(json!({ "labelled": i + 1, "of": arts.len() }));
        }
        if engine.is_empty() {
            engine = std::fs::read_to_string(cx.dir().join("articles").join("engine")).unwrap_or_default();
        }
        Ok(json!({ "engine": engine, "articles": arts.len() }))
    }

    async fn profile(&self, cx: &StageCx, p: &Params) -> Result<Value, StageError> {
        let arts = Self::articles(cx)?;
        let mut pairs = Vec::new();
        for (i, a) in arts.into_iter().enumerate() {
            let t = std::fs::read_to_string(cx.dir().join("articles").join(format!("{i}.labels.json"))).map_err(|e| e.to_string())?;
            pairs.push((a, serde_json::from_str::<Labels>(&t).map_err(|e| e.to_string())?));
        }
        let engine = cx.output("label")?["engine"].as_str().unwrap_or("").to_string();
        let mut prof = vp::build(&p.template, &p.identity, &p.kind, &p.today, &engine, &pairs);
        // the hand-made figure and table habits stay (same journal)
        if let Some(old) = vp::load(&p.workspace, &p.template) {
            prof.keep_floats_from(&old);
        }
        let skipped: HashMap<String, usize> = serde_json::from_value(cx.output("fetch")?["skipped"].clone()).unwrap_or_default();
        prof.skipped = skipped.into_iter().collect();
        vp::save(&p.workspace, &prof)?;
        cx.counts(json!({ "articles": prof.articles.len(), "sections": prof.sections.len() }));
        Ok(json!({ "made": prof.made, "articles": prof.articles.len(), "summary": vp::summary(&prof) }))
    }
}

// ── Helpers over the workspace ──

fn err(code: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

async fn ws_or_err(s: &Shared) -> Result<PathBuf, axum::response::Response> {
    workspace_root(s).await.ok_or_else(|| err(StatusCode::CONFLICT, "示範資料夾不做期刊寫法：請先設定自己的工作區"))
}

fn publishers(ws: &Path) -> PathBuf {
    ws.join("publishers")
}

fn template_or_err(ws: &Path, tpl: &str) -> Result<pc_core::journal::Journal, axum::response::Response> {
    pc_core::journal::list(&publishers(ws)).into_iter().find(|j| j.id == tpl).ok_or_else(|| err(StatusCode::NOT_FOUND, "找不到這個模板"))
}

/// The paper's own keywords (its `keywords` field), never sent anywhere.
fn paper_topic(dir: &Path) -> Vec<String> {
    let Ok(main) = std::fs::read_to_string(dir.join("main.tex")) else { return vec![] };
    pc_core::front::fields(&main)
        .into_iter()
        .filter(|f| f.field == "keywords")
        .flat_map(|f| f.value.replace("\\sep", ",").split([',', ';', '\n']).map(|k| k.trim().to_string()).collect::<Vec<_>>())
        .filter(|k| !k.is_empty() && k.len() < 60)
        .take(10)
        .collect()
}

/// The newest job for a template (running first).
async fn job_for(s: &Shared, tpl: &str) -> Option<(Arc<Jobs>, Arc<Job>)> {
    let js = jobs(s).await;
    let list = js.list();
    let mine: Vec<Arc<Job>> = list.into_iter().filter(|j| j.meta().params["template"].as_str() == Some(tpl)).collect();
    let pick = mine.iter().find(|j| j.is_running()).cloned().or_else(|| mine.first().cloned())?;
    Some((js, pick))
}

fn view(js: &Jobs, job: &Job) -> Value {
    let mut v = js.view(job);
    v["job"] = job.id.clone().into();
    v
}

async fn panel(s: &Shared, ws: &Path, tpl: &str) -> Result<Value, axum::response::Response> {
    let j = template_or_err(ws, tpl)?;
    let identity = Identity::from_table(j.meta.venue.as_ref());
    let submission = Submission::from_table(j.meta.submission.as_ref());
    let today = vp::today();
    let profile = vp::current(ws, tpl, j.meta.venue.as_ref());
    let job = match job_for(s, tpl).await {
        Some((js, job)) if job.is_running() || matches!(job.status().as_str(), status::FAILED | status::STOPPED | status::INTERRUPTED) => {
            // a job older than the profile is of no interest
            let newer = profile.as_ref().is_none_or(|p| vp::day_number(&p.made).unwrap_or(0) * 86_400_000 <= job.meta().created as i64);
            if newer { Some(view(&js, &job)) } else { None }
        }
        _ => None,
    };
    Ok(json!({
        "template": { "id": j.id, "name": j.meta.name, "kind": j.meta.kind, "hidden": j.hidden },
        "identity": identity,
        "guess": vp::search_name(&j.meta.name),
        "submission": submission,
        "submission_stale": submission.as_ref().is_some_and(|x| vp::is_stale(&x.checked, &today)),
        "stale": profile.as_ref().is_some_and(|p| vp::is_stale(&p.made, &today)),
        "profile": profile,
        "job": job,
        "today": today,
    }))
}

// ── Handlers ──

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/venue/summaries", get(summaries))
        .route("/api/venue/sources", get(sources))
        .route("/api/venue/t/{tpl}", get(get_panel))
        .route("/api/venue/t/{tpl}/identity", put(put_identity))
        .route("/api/venue/t/{tpl}/make", post(make))
        .route("/api/venue/t/{tpl}/pdf", post(put_pdf).layer(axum::extract::DefaultBodyLimit::max(80 * 1024 * 1024)))
        .route("/api/venue/t/{tpl}/skeleton", get(skeleton))
        .route("/api/venue/t/{tpl}/prompt", get(prompt))
        .route("/api/venue/jobs/{id}", get(one))
        .route("/api/venue/jobs/{id}/stop", post(stop))
        .route("/api/venue/jobs/{id}/resume", post(resume))
        .route("/api/venue/journals", post(new_journal))
        .route("/api/venue/paper/{name}/journal", post(paper_journal))
        .route("/api/venue/requirements/apply", post(apply_requirements_route))
}

async fn summaries(State(s): State<Shared>) -> axum::response::Response {
    let Some(ws) = workspace_root(&s).await else { return Json(json!({})).into_response() };
    let today = vp::today();
    let mut out = serde_json::Map::new();
    for j in pc_core::journal::list(&publishers(&ws)) {
        if let Some(p) = vp::current(&ws, &j.id, j.meta.venue.as_ref()) {
            out.insert(j.id.clone(), json!({ "made": p.made, "stale": vp::is_stale(&p.made, &today), "n": p.articles.len(), "summary": vp::summary(&p) }));
        }
    }
    Json(Value::Object(out)).into_response()
}

async fn sources(State(s): State<Shared>, AxQuery(q): AxQuery<HashMap<String, String>>) -> axum::response::Response {
    let query = q.get("q").map(|x| vp::search_name(x)).unwrap_or_default();
    if query.trim().len() < 2 {
        return Json(json!({ "sources": [] })).into_response();
    }
    let deps = TEST_DEPS.get().cloned().unwrap_or_else(|| Arc::new(Deps::for_server(&s)));
    match deps.client().find_sources(&query, 8).await {
        Ok(v) => Json(json!({ "sources": v })).into_response(),
        Err(e) => err(if e.is_offline() { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::BAD_GATEWAY }, e.to_string()),
    }
}

async fn get_panel(State(s): State<Shared>, AxPath(tpl): AxPath<String>) -> axum::response::Response {
    let ws = match ws_or_err(&s).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    // venues/requirements.toml edited by hand: its [submission] reaches the templates when the
    // panel is looked at (only what changed is written; no button to remember)
    let _ = apply_requirements(&ws);
    match panel(&s, &ws, &tpl).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
struct IdentityBody {
    source: SourceInfo,
}

fn identity_of(src: &SourceInfo) -> Identity {
    Identity {
        openalex: lore_research::venue::short_id(&src.id),
        name: src.name.clone(),
        issn_l: src.issn_l.clone().unwrap_or_default(),
        issn: src.issn.clone(),
        publisher: src.publisher.clone().unwrap_or_default(),
        kind: src.kind.clone(),
        confirmed: vp::today(),
    }
}

async fn put_identity(State(s): State<Shared>, AxPath(tpl): AxPath<String>, Json(b): Json<IdentityBody>) -> axum::response::Response {
    let ws = match ws_or_err(&s).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    if b.source.id.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "請選一本期刊");
    }
    let id = identity_of(&b.source);
    if let Err(e) = pc_core::journal::edit_meta(&publishers(&ws), &tpl, |m| m.venue = Some(id.to_table())) {
        return err(StatusCode::NOT_FOUND, e);
    }
    match panel(&s, &ws, &tpl).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize, Default)]
struct MakeBody {
    #[serde(default)]
    paper: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    engine: Option<String>,
    /// Files from `POST /api/venue/t/{tpl}/pdf` (用手邊的 PDF).
    #[serde(default)]
    local: Vec<LocalPdf>,
}

/// Start (or resume) the profile job of a template.
pub(crate) async fn start(s: &Shared, ws: &Path, tpl: &str, topic: Vec<String>, kind: &str, engine: Option<String>) -> Result<Value, axum::response::Response> {
    start_with(s, ws, tpl, topic, kind, engine, vec![]).await
}

pub(crate) async fn start_with(s: &Shared, ws: &Path, tpl: &str, topic: Vec<String>, kind: &str, engine: Option<String>, local: Vec<LocalPdf>) -> Result<Value, axum::response::Response> {
    let j = template_or_err(ws, tpl)?;
    if j.hidden {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "這是基礎格式，不是一本期刊"));
    }
    let identity = match Identity::from_table(j.meta.venue.as_ref()) {
        Some(i) => i,
        // the author's own PDFs need no OpenAlex: the journal is the template itself until confirmed
        None if !local.is_empty() => {
            let id = Identity { openalex: format!("local:{tpl}"), name: j.meta.name.clone(), kind: "journal".into(), ..Default::default() };
            pc_core::journal::edit_meta(&publishers(ws), tpl, |m| m.venue = Some(id.to_table())).map_err(|e| err(StatusCode::NOT_FOUND, e))?;
            id
        }
        None => return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "請先確認是哪一本期刊")),
    };
    if let Some((js, job)) = job_for(s, tpl).await
        && job.is_running()
    {
        return Ok(view(&js, &job));
    }
    let p = Params { template: tpl.into(), workspace: ws.to_path_buf(), identity: identity.clone(), kind: kind.into(), topic, today: vp::today(), engine, local };
    let js = jobs(s).await;
    let job = js.create(serde_json::to_value(&p).unwrap_or_default(), true, &identity.name).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("無法建立工作：{e}")))?;
    Ok(view(&js, &job))
}

async fn make(State(s): State<Shared>, AxPath(tpl): AxPath<String>, body: Option<Json<MakeBody>>) -> axum::response::Response {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let ws = match ws_or_err(&s).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let mut topic = vec![];
    if let Some(name) = b.paper.as_deref().filter(|n| !n.is_empty())
        && let Ok(dir) = paper_dir(&s, name).await
    {
        topic = paper_topic(&dir);
    }
    let kind = if b.kind.as_deref() == Some("review") { "review" } else { "article" };
    let engine = b.engine.filter(|e| matches!(e.as_str(), "local" | "subscription" | "api"));
    let local: Vec<LocalPdf> = b.local.into_iter().filter(|f| f.path.starts_with(local_dir()) && f.path.is_file()).collect();
    match start_with(&s, &ws, &tpl, topic, kind, engine, local).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

fn local_dir() -> PathBuf {
    cache_root().join("pdf").join("local")
}

#[derive(Deserialize)]
struct PdfQuery {
    file: String,
}

/// 用手邊的 PDF: one PDF of the journal into the cache → `{path, title}` for `make`.
async fn put_pdf(AxQuery(q): AxQuery<PdfQuery>, body: axum::body::Bytes) -> axum::response::Response {
    if !body.starts_with(b"%PDF") {
        return err(StatusCode::BAD_REQUEST, "不是 PDF");
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut h);
    let sha = format!("{:016x}{:08x}", h.finish(), body.len());
    let dir = local_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    let path = dir.join(format!("{}.pdf", &sha[..16]));
    if let Err(e) = std::fs::write(&path, &body) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    let title = Path::new(&q.file).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    Json(json!({ "path": path, "title": title })).into_response()
}

async fn skeleton(State(s): State<Shared>, AxPath(tpl): AxPath<String>) -> axum::response::Response {
    let Some(ws) = workspace_root(&s).await else { return Json(json!({ "outline": [] })).into_response() };
    let venue = pc_core::journal::load(&publishers(&ws), &tpl).and_then(|j| j.meta.venue);
    let outline = vp::current(&ws, &tpl, venue.as_ref()).map(|p| vp::skeleton(&p)).unwrap_or_default();
    Json(json!({ "outline": outline })).into_response()
}

async fn prompt(State(s): State<Shared>, AxPath(tpl): AxPath<String>, AxQuery(q): AxQuery<HashMap<String, String>>) -> axum::response::Response {
    let ws = match ws_or_err(&s).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let j = match template_or_err(&ws, &tpl) {
        Ok(j) => j,
        Err(r) => return r,
    };
    let sub = Submission::from_table(j.meta.submission.as_ref());
    let p = vp::current(&ws, &tpl, j.meta.venue.as_ref());
    let section = q.get("section").and_then(|t| p.as_ref().and_then(|p| vp::prompt_block(p, t, sub.as_ref())));
    Json(json!({
        "section": section,
        "outline": p.as_ref().map(|p| vp::outline_block(p, sub.as_ref())),
        "emphasis": p.as_ref().map(vp::emphasis_block),
        "checklist": vp::audit_checklist(p.as_ref(), sub.as_ref()),
    }))
    .into_response()
}

async fn find(s: &Shared, id: &str) -> Result<(Arc<Jobs>, Arc<Job>), axum::response::Response> {
    let js = jobs(s).await;
    if !lore_jobs::valid_id(id) {
        return Err(err(StatusCode::NOT_FOUND, "沒有這個工作"));
    }
    let job = js.get(id).ok_or_else(|| err(StatusCode::NOT_FOUND, "沒有這個工作"))?;
    Ok((js, job))
}

async fn one(State(s): State<Shared>, AxPath(id): AxPath<String>, AxQuery(q): AxQuery<HashMap<String, String>>) -> axum::response::Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if let Some(after) = q.get("after").and_then(|v| v.parse::<u64>().ok()) {
        job.changed_after(after, Duration::from_secs(25)).await;
    }
    Json(view(&js, &job)).into_response()
}

async fn stop(State(s): State<Shared>, AxPath(id): AxPath<String>) -> axum::response::Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    js.stop(&job).await;
    Json(view(&js, &job)).into_response()
}

async fn resume(State(s): State<Shared>, AxPath(id): AxPath<String>) -> axum::response::Response {
    let (js, job) = match find(&s, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if !job.is_running() {
        js.start(&job);
    }
    Json(view(&js, &job)).into_response()
}

// ── A journal template of its own ──

/// The submission requirements the author's research gathered (the hook for that data), in the
/// workspace's `venues/requirements.toml`:
///
/// ```toml
/// [template."<template id>"]   # a journal template that exists
/// base = "<base template id>"  # optional: the format it is built on
/// [template."<template id>".submission]
/// abstract_words = 250
/// required_statements = ["Data availability"]
/// checked = "2026-11-14"
///
/// [template."<umbrella id>"]
/// role = "umbrella"            # or "base" / "journal": hides it from the picker (or shows it)
///
/// [journal."1234-5678"]        # a journal by ISSN-L, an ISSN or its OpenAlex id (for 新增期刊)
/// base = "<base template id>"
/// [journal."1234-5678".submission]
/// …
/// ```
///
/// `(base, submission)` for a template id and / or a journal identity; None = not listed.
pub fn requirements_for(ws: &Path, template: Option<&str>, id: Option<&Identity>) -> Option<(Option<String>, Option<toml::Table>)> {
    let t: toml::Table = toml::from_str(&std::fs::read_to_string(ws.join(vp::DIR).join("requirements.toml")).ok()?).ok()?;
    let entry = |e: &toml::Table| (e.get("base").and_then(|b| b.as_str()).map(String::from), e.get("submission").and_then(|b| b.as_table()).cloned());
    if let Some(e) = template.and_then(|tpl| t.get("template")?.as_table()?.get(tpl)?.as_table().cloned()) {
        return Some(entry(&e));
    }
    let journals = t.get("journal")?.as_table()?;
    let id = id?;
    let keys = std::iter::once(&id.issn_l).chain(id.issn.iter()).chain(std::iter::once(&id.openalex)).filter(|k| !k.is_empty());
    for k in keys {
        if let Some(e) = journals.get(k.as_str()).and_then(|v| v.as_table()) {
            return Some(entry(e));
        }
    }
    None
}

/// Write `[submission]` (and `role`) from `venues/requirements.toml` into every template it lists
/// (by template id, or by the template's confirmed identity); the ids changed.
pub fn apply_requirements(ws: &Path) -> Vec<String> {
    let pubs = publishers(ws);
    let mut done = Vec::new();
    for j in pc_core::journal::list(&pubs) {
        let id = Identity::from_table(j.meta.venue.as_ref());
        let role = template_role(ws, &j.id).filter(|r| *r != j.meta.role);
        let sub = requirements_for(ws, Some(&j.id), id.as_ref()).and_then(|r| r.1).filter(|x| j.meta.submission.as_ref() != Some(x));
        if role.is_none() && sub.is_none() {
            continue;
        }
        if pc_core::journal::edit_meta(&pubs, &j.id, |m| {
            if let Some(r) = role {
                m.role = r;
            }
            if let Some(x) = sub {
                m.submission = Some(x);
            }
        })
        .is_ok()
        {
            done.push(j.id.clone());
        }
    }
    done
}

/// `role` of `[template."<id>"]` in `venues/requirements.toml` (`journal`, `base`, `umbrella`).
fn template_role(ws: &Path, tpl: &str) -> Option<String> {
    let t: toml::Table = toml::from_str(&std::fs::read_to_string(ws.join(vp::DIR).join("requirements.toml")).ok()?).ok()?;
    let r = t.get("template")?.as_table()?.get(tpl)?.as_table()?.get("role")?.as_str()?.trim().to_string();
    matches!(r.as_str(), "journal" | "base" | "umbrella").then_some(r)
}

/// A base format for a journal from its publisher (the bases' own publisher and name words
/// against OpenAlex's host organisation, also as an acronym: "Institute of Electrical and
/// Electronics Engineers" → IEEE), the conference / journal kind breaking ties.
pub fn base_for(bases: &[pc_core::journal::Journal], publisher: &str, conference: bool) -> Option<String> {
    let low = publisher.to_lowercase();
    if low.trim().is_empty() {
        return None;
    }
    let acronym: String = publisher.split_whitespace().filter(|w| w.chars().next().is_some_and(char::is_uppercase)).filter_map(|w| w.chars().next()).collect::<String>().to_lowercase();
    let mut best: Option<(i32, String)> = None;
    for b in bases {
        let words: Vec<String> = format!("{} {} {}", b.meta.publisher, b.meta.name, b.id).to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 3).map(String::from).collect();
        let hit = words.iter().any(|w| low.split(|c: char| !c.is_alphanumeric()).any(|x| x == w) || (acronym.len() >= 3 && *w == acronym));
        if !hit {
            continue;
        }
        let kind_fit = (b.meta.kind == "conference") == conference;
        let score = 10 + if kind_fit { 5 } else { 0 };
        if best.as_ref().is_none_or(|(s, id)| score > *s || (score == *s && b.id < *id)) {
            best = Some((score, b.id.clone()));
        }
    }
    best.map(|(_, id)| id)
}

#[derive(Deserialize)]
struct NewJournalBody {
    source: SourceInfo,
    #[serde(default)]
    base: Option<String>,
}

async fn new_journal(State(s): State<Shared>, Json(b): Json<NewJournalBody>) -> axum::response::Response {
    let ws = match ws_or_err(&s).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let pubs = publishers(&ws);
    let all = pc_core::journal::list(&pubs);
    let id = identity_of(&b.source);
    // the same journal already has its template (confirmed, or by its name): that one, confirmed now
    let same_name = |j: &pc_core::journal::Journal| !j.hidden && [&j.meta.name, &j.meta.short].iter().any(|n| lore_research::norm_title(&vp::search_name(n)) == lore_research::norm_title(&id.name));
    if let Some(j) = all.iter().find(|j| Identity::from_table(j.meta.venue.as_ref()).is_some_and(|x| x.openalex == id.openalex)) {
        return Json(json!({ "id": j.id, "existing": true })).into_response();
    }
    if let Some(j) = all.iter().find(|j| Identity::from_table(j.meta.venue.as_ref()).is_none() && same_name(j)) {
        if let Err(e) = pc_core::journal::edit_meta(&pubs, &j.id, |m| m.venue = Some(id.to_table())) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, e);
        }
        return Json(json!({ "id": j.id, "existing": true })).into_response();
    }
    let bases: Vec<pc_core::journal::Journal> = all.iter().filter(|j| j.hidden && j.meta.role != "umbrella" && j.meta.kind != "book").cloned().collect();
    let req = requirements_for(&ws, None, Some(&id));
    let conference = id.kind == "conference";
    let base = b.base.filter(|x| bases.iter().any(|j| &j.id == x)).or_else(|| req.as_ref().and_then(|r| r.0.clone()).filter(|x| all.iter().any(|j| &j.id == x))).or_else(|| base_for(&bases, &id.publisher, conference));
    let Some(base) = base else {
        let list: Vec<Value> = bases.iter().map(|j| json!({ "id": j.id, "name": j.meta.name, "publisher": j.meta.publisher })).collect();
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": "不知道這本期刊用哪個格式", "bases": list }))).into_response();
    };
    let meta = pc_core::journal::Meta { name: id.name.clone(), publisher: id.publisher.clone(), kind: if conference { "conference".into() } else { "journal".into() }, ..Default::default() };
    let tpl = match pc_core::journal::create(&pubs, meta, &base) {
        Ok(t) => t,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    let sub = req.and_then(|r| r.1);
    if let Err(e) = pc_core::journal::edit_meta(&pubs, &tpl, |m| {
        m.venue = Some(id.to_table());
        m.role = "journal".into();
        if sub.is_some() {
            m.submission = sub.clone();
        }
    }) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    let job = start(&s, &ws, &tpl, vec![], "article", None).await.ok();
    Json(json!({ "id": tpl, "base": base, "job": job })).into_response()
}

async fn apply_requirements_route(State(s): State<Shared>) -> axum::response::Response {
    match ws_or_err(&s).await {
        Ok(ws) => Json(json!({ "updated": apply_requirements(&ws) })).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
struct PaperJournalBody {
    template: String,
}

/// 選期刊: a paper on a base format (or none) onto a journal template; its section files keep
/// every word (the draft's own retemplating: the old main.tex goes to draft/archive/).
async fn paper_journal(State(s): State<Shared>, AxPath(name): AxPath<String>, Json(b): Json<PaperJournalBody>) -> axum::response::Response {
    let ws = match ws_or_err(&s).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let dir = match paper_dir(&s, &name).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    match template_or_err(&ws, &b.template) {
        Ok(j) if !j.hidden => {}
        Ok(_) => return err(StatusCode::UNPROCESSABLE_ENTITY, "請選一本期刊，不是基礎格式"),
        Err(r) => return r,
    }
    if let Err(r) = crate::draft::set_paper_template(&s, &ws, &dir, Some(&b.template)).await {
        return r;
    }
    Json(json!({ "ok": true, "template": b.template })).into_response()
}

#[cfg(test)]
#[path = "venueprof_tests.rs"]
mod tests;
