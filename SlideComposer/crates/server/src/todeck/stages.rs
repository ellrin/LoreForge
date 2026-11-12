//! The paper → deck pipeline on the job runner (`crate::jobs`):
//! `source → notation → terms → profile → outline → review → slides → polish`
//! (docs/paper-deck.md, the stage table). Every stage reads its inputs from the earlier
//! stages' checkpoints in the job folder, never from memory, so any one can be redone alone.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use lore_canvas::reveal::{RevealSpec, reveal};
use lore_canvas::{Look, Op};
use lore_llm::LlmRequest;
use lore_notation::Notation;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::build::{BuildInput, ModuleBox, build};
use super::plan::{Plan, PlanSlide, ROLES, clean_plan, prompt, schema};
use super::profile::{ProfileInput, StyleProfile};
use super::source::{Material, from_latex, from_lorereader, is_paper};
use super::suggest::point_of;
use super::{outline, polish, profile, suggest, terms};
use crate::AppState;
use crate::jobs::{Job, Pipeline, StageCx, StageError, StageFuture, StageSpec, status};
use crate::llm::TASK_PAPER_DECK;
use crate::reader::{Reader, valid_sha};

pub const STAGES: &[StageSpec] = &[
    StageSpec { id: "source", label: "讀論文", model: false, wait: false },
    StageSpec { id: "notation", label: "對照表", model: false, wait: false },
    StageSpec { id: "terms", label: "名詞", model: false, wait: false },
    StageSpec { id: "profile", label: "風格", model: false, wait: false },
    StageSpec { id: "outline", label: "大綱", model: true, wait: false },
    StageSpec { id: "review", label: "微調", model: false, wait: true },
    StageSpec { id: "slides", label: "投影片", model: false, wait: false },
    StageSpec { id: "polish", label: "排版用字", model: false, wait: false },
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    pub kind: String,
    pub path: Option<String>,
    pub sha: Option<String>,
}

/// A job's parameters (`job.json` → `params`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub source: Source,
    pub length: String,
    pub lang: String,
    pub style: String,
    pub engine: Option<String>,
}

pub struct PaperDeck {
    s: Weak<AppState>,
    /// The review material per job (read once from the checkpoints).
    review_view: Mutex<HashMap<String, (Option<std::time::SystemTime>, Value)>>,
}

impl PaperDeck {
    pub fn new(s: Weak<AppState>) -> PaperDeck {
        PaperDeck { s, review_view: Mutex::new(HashMap::new()) }
    }

    fn shared(&self) -> Result<Arc<AppState>, StageError> {
        self.s.upgrade().ok_or_else(|| StageError::new(500, "伺服器已關閉"))
    }
}

fn join_err(e: tokio::task::JoinError) -> StageError {
    StageError::new(500, e.to_string())
}

fn params(cx: &StageCx) -> Result<Params, StageError> {
    serde_json::from_value(cx.params()).map_err(|e| StageError::new(400, format!("工作參數無法讀取：{e}")))
}

fn typed<T: DeserializeOwned>(cx: &StageCx, stage: &str) -> Result<T, StageError> {
    serde_json::from_value(cx.output(stage)?).map_err(|e| StageError::new(500, format!("「{stage}」的結果無法讀取：{e}")))
}

fn to_value(v: &impl Serialize) -> Result<Value, StageError> {
    serde_json::to_value(v).map_err(|e| StageError::new(500, e.to_string()))
}

/// The material from `source.json`, with its pictures read back from `img/`.
fn material(cx: &StageCx, images: bool) -> Result<Material, StageError> {
    let mut m: Material = typed(cx, "source")?;
    if images {
        m.load_images(&cx.dir().join("img")).map_err(|e| StageError::new(500, format!("論文的圖不見了（{e}）")))?;
    }
    Ok(m)
}

/// The paper's material (blocking: files, poppler). `cache`: rendered figure PDFs, LoreReader crops.
pub fn read_source(src: &Source, home: &lore_home::Home, cache: &Path) -> Result<Material, String> {
    match src.kind.as_str() {
        "pc" => {
            let p = PathBuf::from(src.path.clone().ok_or("缺少論文資料夾")?);
            if !is_paper(&p) {
                return Err("這個資料夾不是 LaTeX 論文".into());
            }
            from_latex(&p, cache)
        }
        "lr" => {
            let sha = src.sha.clone().filter(|x| valid_sha(x)).ok_or("缺少論文")?;
            let r = Reader::discover(home).ok_or("找不到 LoreReader 的資料夾")?;
            from_lorereader(&r, &sha, cache)
        }
        _ => Err("不認得的來源".into()),
    }
}

impl Pipeline for PaperDeck {
    fn stages(&self) -> &'static [StageSpec] {
        STAGES
    }

    fn run<'a>(&'a self, cx: &'a StageCx, stage: &'static str) -> StageFuture<'a> {
        Box::pin(async move {
            let s = self.shared()?;
            match stage {
                "source" => source(&s, cx).await,
                "notation" => notation(&s, cx),
                "terms" => terms_stage(&s, cx).await,
                "profile" => profile_stage(&s, cx).await,
                "outline" => outline_stage(&s, cx).await,
                "slides" => slides(&s, cx).await,
                "polish" => polish_stage(&s, cx).await,
                other => Err(StageError::new(500, format!("沒有這個階段：{other}"))),
            }
        })
    }

    fn decorate(&self, job: &Job, snap: &mut Map<String, Value>) {
        let st = job.status();
        let p = job.meta().params;
        snap.insert("options".into(), json!({ "length": p["length"], "lang": p["lang"], "style": p["style"] }));
        if st == status::DONE
            && let Some(out) = job.output("slides")
        {
            snap.insert("deck".into(), out["name"].clone());
            let mut result = out.clone();
            if let Some(p) = job.output("polish") {
                result["polish"] = json!({ "checks": p["checks"], "fixes": p["fixes"].as_array().map_or(0, Vec::len) });
            }
            snap.insert("result".into(), result);
        }
        if st == status::REVIEW {
            // keyed by the outline file's time: a redone outline (語言／長度) is read again
            let stamp = std::fs::metadata(job.stage_file("outline")).and_then(|m| m.modified()).ok();
            let cached = self.review_view.lock().unwrap_or_else(|e| e.into_inner()).get(&job.id).filter(|(t, _)| *t == stamp).map(|(_, v)| v.clone());
            let v = match cached {
                Some(v) => v,
                None => {
                    let v = review_view(job);
                    self.review_view.lock().unwrap_or_else(|e| e.into_inner()).insert(job.id.clone(), (stamp, v.clone()));
                    v
                }
            };
            snap.insert("review".into(), v);
        }
    }
}

// ── stages ───────────────────────────────────────────────────────────────────

async fn source(s: &Arc<AppState>, cx: &StageCx) -> Result<Value, StageError> {
    let p = params(cx)?;
    let (home, cache, img) = (s.home.clone(), s.ws.root.join("cache/todeck"), cx.dir().join("img"));
    let src = p.source.clone();
    let (h2, c2) = (home.clone(), cache.clone());
    let mut m = tokio::task::spawn_blocking(move || read_source(&src, &h2, &c2).map_err(|e| StageError::new(422, e))).await.map_err(join_err)??;
    // equations whose text LaTeX would not typeset: read from the page crop, else the crop
    if p.source.kind == "lr" && m.equations.iter().any(super::eqread::unreliable) {
        let sha = p.source.sha.clone().unwrap_or_default();
        let crops: Vec<(String, Option<super::source::Img>)> = {
            let (eqs, home, cache, sha) = (m.equations.clone(), home.clone(), cache.clone(), sha.clone());
            tokio::task::spawn_blocking(move || {
                let r = Reader::discover(&home);
                eqs.iter().filter(|e| super::eqread::unreliable(e)).map(|e| (e.id.clone(), r.as_ref().and_then(|r| equation_crop(r, &sha, e, &cache)))).collect()
            })
            .await
            .map_err(join_err)?
        };
        let notes = super::eqread::read_equations(super::llm(s), &mut m, &|e| crops.iter().find(|(id, _)| *id == e.id).and_then(|(_, c)| c.clone())).await;
        cx.counts(json!({ "eqNotes": notes }));
    }
    let m = tokio::task::spawn_blocking(move || -> Result<Material, StageError> {
        m.save_images(&img).map_err(|e| StageError::new(500, format!("無法寫入快取：{e}")))?;
        Ok(m)
    })
    .await
    .map_err(join_err)??;
    cx.title(&m.title);
    cx.counts(json!({
        "fig": m.figures.iter().filter(|f| f.image.is_some()).count(),
        "figTotal": m.figures.len(),
        "table": m.tables.len(),
        "eq": m.equations.len(),
        "eqVision": m.equations.iter().filter(|e| e.read == "vision").count(),
        "eqPicture": m.equations.iter().filter(|e| e.read == "picture").count(),
        "sections": m.sections.len(),
        "refs": m.refs.len(),
    }));
    to_value(&m)
}

/// The crop of a LoreReader equation (its page at 200 dpi, the box with a margin).
fn equation_crop(r: &Reader, sha: &str, e: &super::source::Eq, cache: &Path) -> Option<super::source::Img> {
    let (page, b) = (e.page?, e.bbox?);
    let tool = crate::reader::pdftoppm()?;
    let pdf = r.pdf(sha)?;
    let png = crate::reader::render_page(&tool, &pdf, sha, page, 200, cache).ok()?;
    let crop = lore_docsrc::crop::crop_png_fraction(&png, b, 14).ok()?;
    super::source::img_from(crop, "png")
}

/// `notation.json` of the job: the entries the deck registers (the paper's ids kept) and the
/// macros the equations are expanded with (shared registry + the paper's).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NotationOut {
    pub doc: Notation,
    pub macros: BTreeMap<String, String>,
}

fn notation(s: &Arc<AppState>, cx: &StageCx) -> Result<Value, StageError> {
    let m = material(cx, false)?;
    let global = s.home.shared_dir().map(|d| d.join("notation.json")).and_then(|p| Notation::load(&p).ok()).unwrap_or_default();
    let macros = lore_notation::katex_macros(&Notation::effective(&global, &m.notation));
    let mut doc = Notation { entries: m.notation.entries.clone(), ..Default::default() };
    if let Some(table) = &m.lr_symbols {
        let ids: Vec<String> = table["entries"].as_array().into_iter().flatten().chain(table["equations"].as_array().into_iter().flatten()).filter_map(|e| e["id"].as_str().map(String::from)).collect();
        crate::reader::register_into(&mut doc, table, &ids);
    }
    cx.counts(json!({ "entries": doc.entries.len() }));
    to_value(&NotationOut { doc, macros })
}

async fn terms_stage(s: &Arc<AppState>, cx: &StageCx) -> Result<Value, StageError> {
    let m = material(cx, false)?;
    let n: NotationOut = typed(cx, "notation")?;
    let home = s.home.clone();
    let out = tokio::task::spawn_blocking(move || terms::terms(&terms::TermsInput { material: &m, notation: &n.doc, home: &home })).await.map_err(join_err)?;
    cx.counts(json!({ "terms": out.total, "unknown": out.unfamiliar }));
    to_value(&out)
}

async fn profile_stage(s: &Arc<AppState>, cx: &StageCx) -> Result<Value, StageError> {
    let p = params(cx)?;
    let s = s.clone();
    let out = tokio::task::spawn_blocking(move || profile::profile(&ProfileInput { home: &s.home, ws: &s.ws, style: &p.style, lang: &p.lang })).await.map_err(join_err)?;
    to_value(&out)
}

/// `outline.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OutlineOut {
    pub plan: Plan,
    /// Up to 4 questions for the review window (none yet: the page asks its own four).
    pub questions: Vec<Value>,
    pub boxes: Vec<ModuleBox>,
    /// Provider id that wrote the plan.
    pub engine: String,
    /// 微調's 建議 (`suggest.rs`): at most 5 edits on the review rows, often none.
    pub suggestions: Vec<suggest::Suggestion>,
}

async fn outline_stage(s: &Arc<AppState>, cx: &StageCx) -> Result<Value, StageError> {
    let p = params(cx)?;
    let m = material(cx, true)?;
    let prof: StyleProfile = typed(cx, "profile").unwrap_or_default();
    let (mut system, user) = prompt(&m, &p.length, &p.lang);
    if !prof.summary.trim().is_empty() {
        system += &format!("\n\nThe presenter's own decks, summarised — follow these habits:\n{}", prof.summary.trim());
    }
    let req = LlmRequest::prompt(system, user).json(schema()).max_tokens(12000);
    let llm = super::llm(s);
    cx.chars(0);
    let (v, engine) = outline::ask_json(llm, TASK_PAPER_DECK, req, p.engine.as_deref(), &|n| cx.chars(n)).await?;
    let plan: Plan = serde_json::from_value(v).map_err(|e| StageError::new(502, format!("模型的大綱無法使用：{e}")))?;
    let plan = clean_plan(plan, &m);
    if plan.slides.len() < 3 {
        return Err(StageError::new(502, "模型的大綱太短"));
    }
    let boxes = outline::module_boxes(llm, &m, &plan).await;
    // 建議: the progress keeps counting characters after the plan's
    let base = cx.job.meta().stages.get("outline").and_then(|s| s.chars).unwrap_or(0);
    let suggestions = suggest::suggest(llm, &plan, &m, &p.length, &p.lang, &prof.summary, "", p.engine.as_deref(), &|n| cx.chars(base + n)).await;
    cx.counts(json!({ "slides": plan.slides.len(), "boxes": boxes.len(), "suggestions": suggestions.len() }));
    to_value(&OutlineOut { plan, questions: vec![], boxes, engine, suggestions })
}

async fn slides(s: &Arc<AppState>, cx: &StageCx) -> Result<Value, StageError> {
    let p = params(cx)?;
    let m = material(cx, true)?;
    let n: NotationOut = typed(cx, "notation")?;
    let o: OutlineOut = typed(cx, "outline")?;
    // the terms as the review left them (我懂 already applied)
    let tr: terms::TermsResult = typed(cx, "terms").unwrap_or_default();
    let review = cx.output("review").unwrap_or(json!({ "skipped": true }));
    let main_before = o.plan.slides.iter().find(|s| s.role == "overview").map(|s| s.figure.clone());
    let plan = apply_review(o.plan, &review, &m);
    let main_after = plan.slides.iter().find(|s| s.role == "overview").map(|s| s.figure.clone());
    // the boxes belong to the figure and steps the model planned
    let boxes = if main_before == main_after { o.boxes } else { vec![] };
    let total = plan.slides.len();
    cx.counts(json!({ "slide": 0, "slides": total }));
    let count = cx.counter();
    let (s, dir, engine) = (s.clone(), cx.dir().to_path_buf(), o.engine);
    tokio::task::spawn_blocking(move || make_deck(&s, &dir, &p, &m, &n, &plan, &tr, boxes, &engine, &|i, t| count(json!({ "slide": i, "slides": t })))).await.map_err(join_err)?
}

/// The deck from the plan (blocking: pandoc, files). A crash half-way leaves the deck's name
/// in `slides.pending`; the next try removes that half-made deck first.
#[allow(clippy::too_many_arguments)]
fn make_deck(s: &AppState, dir: &Path, p: &Params, m: &Material, n: &NotationOut, plan: &Plan, tr: &terms::TermsResult, boxes: Vec<ModuleBox>, engine: &str, progress: &dyn Fn(usize, usize)) -> Result<Value, StageError> {
    let pending = dir.join("slides.pending");
    if let Ok(old) = std::fs::read_to_string(&pending) {
        let old = old.trim();
        if !old.is_empty() && !old.contains(['/', '\\']) && old != "." && old != ".." {
            let _ = std::fs::remove_dir_all(s.ws.decks.join(old));
        }
    }
    let api = |e: crate::ApiError| StageError::new(e.0.as_u16(), e.1);
    let st = s.ws.style(&p.style).map_err(|e| api(e.into()))?;
    let name = s.ws.new_deck(&m.slug, &p.style).map_err(|e| api(e.into()))?;
    crate::jobs::write_atomic(&pending, name.as_bytes()).map_err(|e| StageError::new(500, e.to_string()))?;
    let out = (|| -> Result<Value, StageError> {
        let mut sess = s.ws.open(&name).map_err(|e| api(e.into()))?;
        let deck_dir = sess.dir.clone();
        let mut deck = sess.editor.deck.clone();
        let built = {
            let mut uid = || deck.new_uid();
            let mut prog = |i: usize, t: usize| progress(i, t);
            build(plan, m, BuildInput { ws: &s.ws, st: &st, source: &sess.source, deck_dir: &deck_dir, pandoc: &s.pandoc, macros: n.macros.clone(), lang: &p.lang, boxes: boxes.clone(), next_uid: &mut uid, progress: &mut prog })
                .map_err(|e| StageError::new(422, e))?
        };
        let mut slides = built.slides;
        // 不熟: learn marks on the unfamiliar terms (whole sentences in background / method
        // narration), before the big picture is split so its steps keep them
        let matcher = tr.matcher();
        if !matcher.is_empty() {
            for (sl, ps) in slides.iter_mut().zip(&plan.slides) {
                crate::kbterms::mark_slide(sl, &matcher, crate::kbterms::sentence_role(&ps.role));
            }
        }
        // the big picture, one slide per numbered step
        if let Some((idx, steps)) = &built.reveal {
            let rules = st.pack.ghost.clone();
            let grey = rules.text_color.clone();
            let ghost = move |l: &Look| Look { color: grey.clone(), ..l.clone() };
            let src = slides[*idx].clone();
            let more = reveal(&src, &RevealSpec::Ghost { steps: steps.clone(), future: "ghost".into() }, &rules, &ghost, &mut || deck.new_uid());
            slides.splice(*idx..=*idx, more);
        }
        let mut ops: Vec<Op> = sess.editor.deck.slides.iter().map(|sl| Op::DeleteSlide { slide: sl.uid.clone() }).collect();
        for (i, sl) in slides.into_iter().enumerate() {
            ops.push(Op::InsertSlide { index: i, slide: sl });
        }
        sess.editor.apply(ops, "論文報告").map_err(|e| api(e.into()))?;
        sess.save().map_err(|e| api(e.into()))?;
        crate::kbterms::save_terms(&sess.dir, tr).map_err(|e| StageError::new(500, e.to_string()))?;
        // the paper's notation, with its ids, into the deck's own file
        let (_, mut doc) = crate::notation::load(&sess.dir, None);
        let mut added = 0;
        for e in &n.doc.entries {
            if doc.get(&e.id).is_none() {
                doc.entries.push(e.clone());
                added += 1;
            }
        }
        if added > 0 {
            doc.save(&sess.dir.join("notation.json")).map_err(|e| StageError::new(500, e.to_string()))?;
        }
        Ok(json!({
            "name": name,
            "slides": sess.editor.deck.slides.len(),
            "engine": crate::llm::engine_label(engine),
            "boxes": boxes.len(),
            "notation": added,
            "unknown": tr.unfamiliar,
            "warnings": built.warnings,
        }))
    })();
    if out.is_err() {
        // a half-made deck is not left behind
        let _ = std::fs::remove_dir_all(s.ws.decks.join(&name));
    }
    let _ = std::fs::remove_file(&pending);
    out
}

async fn polish_stage(s: &Arc<AppState>, cx: &StageCx) -> Result<Value, StageError> {
    let p = params(cx)?;
    let made = cx.output("slides")?;
    let name = made["name"].as_str().unwrap_or("").to_string();
    let prof: StyleProfile = typed(cx, "profile").unwrap_or_default();
    let api = |e: crate::ApiError| StageError::new(e.0.as_u16(), e.1);
    // the deck's shared session: a page that has it open sees the fixes and never saves over them
    let sess = crate::session(s, &name).map_err(api)?;
    let (s2, style, lang) = (s.clone(), p.style.clone(), p.lang.clone());
    let (mut report, deck, deck_dir) = tokio::task::spawn_blocking(move || -> Result<(polish::PolishReport, lore_canvas::Deck, PathBuf), StageError> {
        let st = s2.ws.style(&style).map_err(|e| api(e.into()))?;
        let mut sess = sess.lock().unwrap_or_else(|e| e.into_inner());
        let (_, doc) = crate::notation::load(&sess.dir, None);
        let mut slides = sess.editor.deck.slides.clone();
        let report = polish::polish(&mut slides, &polish::PolishInput { st: &st, profile: &prof, notation: &doc, lang: &lang });
        if !report.fixes.is_empty() {
            let mut ops: Vec<Op> = sess.editor.deck.slides.iter().map(|sl| Op::DeleteSlide { slide: sl.uid.clone() }).collect();
            for (i, sl) in slides.into_iter().enumerate() {
                ops.push(Op::InsertSlide { index: i, slide: sl });
            }
            sess.editor.apply(ops, "排版用字微調").map_err(|e| api(e.into()))?;
            sess.save().map_err(|e| api(e.into()))?;
        }
        Ok((report, sess.editor.deck.clone(), sess.dir.clone()))
    })
    .await
    .map_err(join_err)??;
    // rule 6's model step (one term per concept): suggestions only, never applied here
    cx.counts(json!({ "checks": report.checks, "fixes": report.fixes.len() }));
    let mut rep = crate::deckpolish::Report { items: std::mem::take(&mut report.items), ..Default::default() };
    match crate::deckpolish::suggest_terms(super::llm(s), &deck, p.engine.as_deref()).await {
        Ok((groups, terms, engine)) => {
            crate::deckpolish::term_items(&deck, &groups, &terms, &mut rep);
            rep.engine = engine;
        }
        Err(e) => rep.notes.push(format!("用詞一致的建議沒有做：{e}")),
    }
    // the deck's own report (`decks/<name>/polish.json`): the editor shows it, 復原 and 套用
    // go through `POST /api/decks/{name}/polish/{item}` (the editor's undo path)
    crate::deckpolish::save_report(&deck_dir, &rep).map_err(|e| StageError::new(500, e))?;
    report.items = rep.items;
    to_value(&report)
}

// ── the review window ────────────────────────────────────────────────────────

/// `review` of the job object while it waits: the outline rows, the paper's figures and
/// tables (thumbnails from the job folder) and the unknown terms.
fn review_view(job: &Job) -> Value {
    let out: OutlineOut = job.output("outline").and_then(|o| serde_json::from_value(o).ok()).unwrap_or_default();
    let plan = out.plan;
    let m: Material = job.output("source").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let outline = suggest::rows(&plan);
    let thumb = |f: &super::source::Fig| f.image.as_ref().filter(|im| !im.file.is_empty()).map(|im| format!("/api/todeck/jobs/{}/img/{}", job.id, im.file));
    let mut figures: Vec<Value> = m.figures.iter().enumerate().map(|(i, f)| json!({ "id": f.id, "label": format!("圖 {}", i + 1), "caption": f.caption, "thumb": thumb(f) })).collect();
    figures.extend(m.tables.iter().enumerate().map(|(i, f)| json!({ "id": f.id, "label": format!("表 {}", i + 1), "caption": f.caption, "thumb": thumb(f) })));
    let terms: terms::TermsResult = job.output("terms").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let unknown: Vec<Value> = terms.terms.iter().filter(|t| t.unfamiliar()).map(|t| json!({ "term": t.text, "n": t.count })).collect();
    json!({ "outline": outline, "figures": figures, "unknown": unknown, "suggestions": out.suggestions })
}

/// 我懂 from the review window, before the slides are made: the terms checkpoint marks them
/// `understood`; with `remember` each one is also saved in KnowledgeBase as 記得 (its own card
/// when it has one). Returns one line per term saving that failed.
pub async fn review_terms(job: &Job, review: &Value) -> Vec<String> {
    let known: Vec<String> = review["known"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
    if known.is_empty() {
        return vec![];
    }
    let Some(mut tr) = job.output("terms").and_then(|v| serde_json::from_value::<terms::TermsResult>(v).ok()) else { return vec![] };
    for k in &known {
        tr.understood(k);
    }
    if let Ok(v) = serde_json::to_value(&tr)
        && let Err(e) = job.replace_output("terms", v, json!({ "terms": tr.total, "unknown": tr.unfamiliar }))
    {
        return vec![format!("無法更新名詞：{e}")];
    }
    let mut notes = vec![];
    if review["remember"] == true {
        let base = crate::kbterms::kb_url();
        for k in &known {
            let card = tr.terms.iter().find(|t| t.understood && (t.text == *k || t.key == *k || t.forms.iter().any(|f| f == k))).and_then(|t| t.card.clone());
            if let Err(e) = crate::kbterms::save_known(&base, k, card.as_deref()).await {
                notes.push(e);
                break; // KnowledgeBase not open: once is enough
            }
        }
    }
    notes
}

/// The plan after the author's review (docs/paper-deck.md "API", 微調): rows reordered,
/// retitled, figures swapped, points rewritten, rows removed; the answers 主架構圖 and
/// 我的看法頁 applied. Then cleaned again (only ids the material has).
pub fn apply_review(mut plan: Plan, review: &Value, m: &Material) -> Plan {
    if review["skipped"] == true || review["skip"] == true {
        return plan;
    }
    let figs: BTreeSet<&str> = m.figures.iter().map(|f| f.id.as_str()).collect();
    let tabs: BTreeSet<&str> = m.tables.iter().map(|f| f.id.as_str()).collect();
    if let Some(rows) = review["outline"].as_array() {
        let mut out = vec![];
        for r in rows {
            let idx = r["id"].as_str().and_then(|x| x.strip_prefix('s')).and_then(|n| n.parse::<usize>().ok()).and_then(|n| n.checked_sub(1)).filter(|&i| i < plan.slides.len());
            let orig = idx.map(|i| plan.slides[i].clone());
            let mut sl = orig.clone().unwrap_or_else(|| PlanSlide { role: "background".into(), ..Default::default() });
            if let Some(k) = r["kind"].as_str().filter(|k| ROLES.contains(k)) {
                sl.role = k.to_string();
            }
            if let Some(t) = r["title"].as_str() {
                sl.title = t.trim().to_string();
            }
            if let Some(list) = r["figs"].as_array() {
                let ids: Vec<&str> = list.iter().filter_map(Value::as_str).collect();
                sl.figure = ids.iter().find(|i| figs.contains(**i)).map(|s| s.to_string()).unwrap_or_default();
                sl.table = ids.iter().find(|i| tabs.contains(**i)).map(|s| s.to_string()).unwrap_or_default();
            }
            if let Some(pt) = r["point"].as_str().map(str::trim) {
                let before = orig.as_ref().map(point_of).unwrap_or_default();
                if pt != before.trim() {
                    if !sl.takeaway.trim().is_empty() {
                        sl.takeaway = pt.to_string();
                    } else if let Some(b) = sl.bullets.first_mut() {
                        *b = pt.to_string();
                    } else if !pt.is_empty() {
                        sl.bullets.push(pt.to_string());
                    }
                }
            }
            out.push(sl);
        }
        plan.slides = out;
    }
    let a = &review["answers"];
    if let Some(f) = a["mainFig"].as_str().filter(|f| figs.contains(f)) {
        for s in plan.slides.iter_mut().filter(|s| s.role == "overview" || s.role == "method") {
            s.figure = f.to_string();
        }
    }
    match a["opinion"].as_bool() {
        Some(false) => plan.slides.retain(|s| s.role != "opinion"),
        Some(true) if !plan.slides.iter().any(|s| s.role == "opinion") => {
            let at = plan.slides.iter().position(|s| s.role == "takeaway" || s.role == "thanks").unwrap_or(plan.slides.len());
            plan.slides.insert(at, PlanSlide { role: "opinion".into(), title: "Some opinions".into(), ..Default::default() });
        }
        _ => {}
    }
    clean_plan(plan, m)
}
