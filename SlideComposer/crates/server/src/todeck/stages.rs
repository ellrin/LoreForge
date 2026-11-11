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
use super::{outline, polish, profile, terms};
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
    review_view: Mutex<HashMap<String, Value>>,
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
            let cached = self.review_view.lock().unwrap_or_else(|e| e.into_inner()).get(&job.id).cloned();
            let v = match cached {
                Some(v) => v,
                None => {
                    let v = review_view(job);
                    self.review_view.lock().unwrap_or_else(|e| e.into_inner()).insert(job.id.clone(), v.clone());
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
    let m = tokio::task::spawn_blocking(move || -> Result<Material, StageError> {
        let mut m = read_source(&p.source, &home, &cache).map_err(|e| StageError::new(422, e))?;
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
        "sections": m.sections.len(),
        "refs": m.refs.len(),
    }));
    to_value(&m)
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
    cx.counts(json!({ "terms": out.terms.len(), "unknown": out.terms.iter().filter(|t| t.unknown).count() }));
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
    cx.counts(json!({ "slides": plan.slides.len(), "boxes": boxes.len() }));
    to_value(&OutlineOut { plan, questions: vec![], boxes, engine })
}

async fn slides(s: &Arc<AppState>, cx: &StageCx) -> Result<Value, StageError> {
    let p = params(cx)?;
    let m = material(cx, true)?;
    let n: NotationOut = typed(cx, "notation")?;
    let o: OutlineOut = typed(cx, "outline")?;
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
    tokio::task::spawn_blocking(move || make_deck(&s, &dir, &p, &m, &n, &plan, boxes, &engine, &|i, t| count(json!({ "slide": i, "slides": t })))).await.map_err(join_err)?
}

/// The deck from the plan (blocking: pandoc, files). A crash half-way leaves the deck's name
/// in `slides.pending`; the next try removes that half-made deck first.
#[allow(clippy::too_many_arguments)]
fn make_deck(s: &AppState, dir: &Path, p: &Params, m: &Material, n: &NotationOut, plan: &Plan, boxes: Vec<ModuleBox>, engine: &str, progress: &dyn Fn(usize, usize)) -> Result<Value, StageError> {
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
    let s = s.clone();
    let report = tokio::task::spawn_blocking(move || -> Result<polish::PolishReport, StageError> {
        let api = |e: crate::ApiError| StageError::new(e.0.as_u16(), e.1);
        let st = s.ws.style(&p.style).map_err(|e| api(e.into()))?;
        let mut sess = s.ws.open(&name).map_err(|e| api(e.into()))?;
        let (_, doc) = crate::notation::load(&sess.dir, None);
        let mut slides = sess.editor.deck.slides.clone();
        let report = polish::polish(&mut slides, &polish::PolishInput { st: &st, profile: &prof, notation: &doc, lang: &p.lang });
        if !report.fixes.is_empty() {
            let mut ops: Vec<Op> = sess.editor.deck.slides.iter().map(|sl| Op::DeleteSlide { slide: sl.uid.clone() }).collect();
            for (i, sl) in slides.into_iter().enumerate() {
                ops.push(Op::InsertSlide { index: i, slide: sl });
            }
            sess.editor.apply(ops, "排版用字微調").map_err(|e| api(e.into()))?;
            sess.save().map_err(|e| api(e.into()))?;
        }
        Ok(report)
    })
    .await
    .map_err(join_err)??;
    cx.counts(json!({ "checks": report.checks, "fixes": report.fixes.len() }));
    to_value(&report)
}

// ── the review window ────────────────────────────────────────────────────────

/// The line a review row shows under its title: the take-away, else the first bullet.
fn point_of(s: &PlanSlide) -> String {
    if !s.takeaway.trim().is_empty() { s.takeaway.clone() } else { s.bullets.first().cloned().unwrap_or_default() }
}

fn figs_of(s: &PlanSlide) -> Vec<String> {
    [&s.figure, &s.table].into_iter().filter(|x| !x.is_empty()).cloned().collect()
}

/// `review` of the job object while it waits: the outline rows, the paper's figures and
/// tables (thumbnails from the job folder) and the unknown terms.
fn review_view(job: &Job) -> Value {
    let plan: Plan = job.output("outline").and_then(|o| serde_json::from_value(o["plan"].clone()).ok()).unwrap_or_default();
    let m: Material = job.output("source").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let outline: Vec<Value> = plan
        .slides
        .iter()
        .enumerate()
        .map(|(i, s)| json!({ "id": format!("s{}", i + 1), "kind": s.role, "title": s.title, "figs": figs_of(s), "point": point_of(s) }))
        .collect();
    let thumb = |f: &super::source::Fig| f.image.as_ref().filter(|im| !im.file.is_empty()).map(|im| format!("/api/todeck/jobs/{}/img/{}", job.id, im.file));
    let mut figures: Vec<Value> = m.figures.iter().enumerate().map(|(i, f)| json!({ "id": f.id, "label": format!("圖 {}", i + 1), "caption": f.caption, "thumb": thumb(f) })).collect();
    figures.extend(m.tables.iter().enumerate().map(|(i, f)| json!({ "id": f.id, "label": format!("表 {}", i + 1), "caption": f.caption, "thumb": thumb(f) })));
    let terms: terms::TermsOutput = job.output("terms").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let unknown: Vec<Value> = terms.terms.iter().filter(|t| t.unknown).map(|t| json!({ "term": t.term, "n": t.n })).collect();
    json!({ "outline": outline, "figures": figures, "unknown": unknown })
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
