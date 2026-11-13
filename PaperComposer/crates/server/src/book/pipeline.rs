//! The 寫書 job on lore-jobs (docs/book.md §流程):
//! `source → terms → plan → research → review → write → check`. Every stage reads what it needs
//! from the earlier checkpoints in the job folder, so any one can be redone alone.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use lore_jobs::{Job, Pipeline, StageCx, StageError, StageFuture, StageSpec, status};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Deps;
use super::model::{Ask, TASK_PLAN, TASK_SUGGEST};
use super::plan::{self, Plan, Teach};
use super::research::{self, Research};
use super::source::{self, Extras, Material};
use super::write::{self, WriteOut};

pub const STAGES: &[StageSpec] = &[
    StageSpec { id: "source", label: "讀論文", model: false, wait: false },
    StageSpec { id: "terms", label: "名詞", model: false, wait: false },
    StageSpec { id: "plan", label: "大綱", model: true, wait: false },
    StageSpec { id: "research", label: "找資料", model: false, wait: false },
    StageSpec { id: "review", label: "微調", model: false, wait: true },
    StageSpec { id: "write", label: "撰寫", model: true, wait: false },
    StageSpec { id: "check", label: "檢查", model: false, wait: false },
];

/// `job.json` → `params`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Params {
    pub sha: String,
    /// `new` or the opened.toml id of a book.
    pub book: String,
    /// The existing book's folder (resolved when the job was made).
    pub book_path: Option<PathBuf>,
    /// Where a new book goes.
    pub folder: Option<PathBuf>,
    pub length: String,
    pub lang: String,
    /// `b5` · `a4` (a new book; the review window can change it).
    pub page: String,
    pub engine: Option<String>,
    pub extras: Extras,
}

pub struct BookJob {
    pub deps: Arc<Deps>,
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

fn join_err(e: tokio::task::JoinError) -> StageError {
    StageError::new(500, e.to_string())
}

/// `terms.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TermsOut {
    pub terms: lore_learn::TermsResult,
    /// Not `known`: what the book explains.
    pub teach: Vec<Teach>,
    /// Term pairs (A needs B) from the KnowledgeBase's prerequisite links.
    pub prereq: Vec<(String, String)>,
    /// Known terms (skipped).
    pub known: usize,
}

/// `plan.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlanOut {
    pub plan: Plan,
    pub suggestions: Vec<Value>,
    /// Terms to teach that no chapter took (shown in the review window).
    pub left_out: Vec<String>,
    pub engine: String,
}

/// `check.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CheckOut {
    pub problems: Vec<Problem>,
    pub compiled: super::tools::Compiled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Problem {
    /// notation · cite · figure · compile · chapter
    pub kind: String,
    pub message: String,
}

impl Pipeline for BookJob {
    fn stages(&self) -> &'static [StageSpec] {
        STAGES
    }

    fn run<'a>(&'a self, cx: &'a StageCx, stage: &'static str) -> StageFuture<'a> {
        Box::pin(async move {
            match stage {
                "source" => self.source(cx).await,
                "terms" => self.terms(cx).await,
                "plan" => self.plan(cx).await,
                "research" => self.research(cx).await,
                "write" => self.write(cx).await,
                "check" => self.check(cx).await,
                other => Err(StageError::new(500, format!("沒有這個階段：{other}"))),
            }
        })
    }

    fn decorate(&self, job: &Job, snap: &mut Map<String, Value>) {
        let p: Params = serde_json::from_value(job.meta().params).unwrap_or_default();
        snap.insert("options".into(), json!({ "length": p.length, "lang": p.lang, "page": p.page, "book": p.book, "folder": p.folder }));
        let research: Option<Research> = job.output("research").and_then(|v| serde_json::from_value(v).ok());
        if let Some(r) = &research {
            snap.insert("pending_imports".into(), r.pending_imports.len().into());
        }
        let st = job.status();
        if st == status::REVIEW {
            snap.insert("review".into(), review_view(job, research.as_ref()));
        }
        if st == status::DONE
            && let Some(w) = job.output("write").and_then(|v| serde_json::from_value::<WriteOut>(v).ok())
        {
            let c: CheckOut = job.output("check").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            snap.insert("book".into(), w.book.clone().into());
            snap.insert(
                "result".into(),
                json!({
                    "book": w.book, "title": w.title, "new": w.new_book,
                    "chapters": w.chapters.iter().map(|c| json!({ "title": c.title, "figures": c.figures.len(), "tables": c.tables, "cites": c.cites, "problems": c.problems })).collect::<Vec<_>>(),
                    "notation": w.notation, "notes": w.notes.iter().chain(research.iter().flat_map(|r| r.notes.iter())).collect::<Vec<_>>(),
                    "problems": c.problems, "pdf": c.compiled.pdf, "pages": c.compiled.pages, "compiled": c.compiled.ok, "compileNote": c.compiled.skipped,
                }),
            );
        }
    }
}

/// The review window's material (docs/book.md `review`).
fn review_view(job: &Job, research: Option<&Research>) -> Value {
    let po: PlanOut = job.output("plan").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let tr: TermsOut = job.output("terms").and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let state = |t: &str| tr.teach.iter().find(|x| x.term.eq_ignore_ascii_case(t)).map(|x| x.state.clone()).unwrap_or_else(|| "unknown".into());
    let chapters: Vec<Value> = po
        .plan
        .chapters
        .iter()
        .map(|c| {
            let sources = research.map(|r| r.for_chapter(&c.id).len()).unwrap_or(0);
            json!({ "id": c.id, "title": c.title, "depth": c.depth, "terms": c.terms.iter().map(|t| json!({ "term": t, "state": state(t) })).collect::<Vec<_>>(), "figures": c.figures.len(), "sources": sources })
        })
        .collect();
    json!({
        "chapters": chapters,
        "suggestions": po.suggestions,
        "leftOut": po.left_out,
        "notes": research.map(|r| r.notes.clone()).unwrap_or_default(),
        "license": research.and_then(|r| r.license.clone()),
    })
}

impl BookJob {
    async fn source(&self, cx: &StageCx) -> Result<Value, StageError> {
        let p = params(cx)?;
        let d = self.deps.clone();
        let (img, scratch) = (cx.dir().join("img"), d.cache.join("docsrc").join(&p.sha));
        let m = tokio::task::spawn_blocking(move || {
            let r = d.reader.as_ref().ok_or("找不到 LoreReader 的資料夾")?;
            source::read(r, &p.sha, d.extract.as_ref(), &scratch, &img, p.extras.clone())
        })
        .await
        .map_err(join_err)?
        .map_err(|e: String| StageError::new(422, e))?;
        cx.title(&m.title);
        cx.counts(json!({ "sections": m.sections.len(), "fig": m.figures.len(), "refs": m.refs, "questions": m.questions.len() + m.extras.questions.len(), "read": m.understood.len() }));
        to_value(&m)
    }

    async fn terms(&self, cx: &StageCx) -> Result<Value, StageError> {
        let p = params(cx)?;
        let m: Material = typed(cx, "source")?;
        let d = self.deps.clone();
        let out = tokio::task::spawn_blocking(move || terms_of(&d, &m, p.book_path.as_deref())).await.map_err(join_err)?;
        cx.counts(json!({ "terms": out.terms.total, "teach": out.teach.len(), "known": out.known }));
        to_value(&out)
    }

    async fn plan(&self, cx: &StageCx) -> Result<Value, StageError> {
        let p = params(cx)?;
        let m: Material = typed(cx, "source")?;
        let tr: TermsOut = typed(cx, "terms")?;
        if tr.teach.is_empty() {
            return Err(StageError::new(422, "這篇論文的名詞你都記得了，沒有要寫的"));
        }
        let existing: Vec<String> = p.book_path.as_ref().map(|b| pc_core::open::sections(b).into_iter().map(|s| s.title).collect()).unwrap_or_default();
        // a paper figure is only offered once research knows the licence; the plan may name one, write checks again
        let (system, user) = plan::prompt(&m, &tr.teach, &tr.prereq, &existing, &p.length, &p.lang, true);
        let on = |n: u64| cx.chars(n);
        let (v, engine) = self.deps.model.json(Ask { task: TASK_PLAN, system, user, schema: plan::schema(), max_tokens: 8000, engine: p.engine.as_deref() }, &on).await?;
        let mut pl = plan::clean(&v, &m, true);
        pl.chapters = plan::order(pl.chapters, &tr.prereq);
        if pl.chapters.is_empty() {
            return Err(StageError::new(502, "模型的大綱沒有章節"));
        }
        let taken: Vec<String> = pl.chapters.iter().flat_map(|c| c.terms.iter().map(|t| t.to_lowercase())).collect();
        let left_out: Vec<String> = tr.teach.iter().filter(|t| !taken.contains(&t.term.to_lowercase())).map(|t| t.term.clone()).collect();
        // 建議: best effort, the characters keep counting
        let base = cx.job.meta().stages.get("plan").and_then(|s| s.chars).unwrap_or(0);
        let on2 = |n: u64| cx.chars(base + n);
        let (ss, us) = plan::suggest_prompt(&pl, &tr.teach, &p.lang);
        let suggestions = match self.deps.model.json(Ask { task: TASK_SUGGEST, system: ss, user: us, schema: plan::suggest_schema(), max_tokens: 3000, engine: p.engine.as_deref() }, &on2).await {
            Ok((v, _)) => plan::clean_suggestions(&v, &pl),
            Err(_) => vec![],
        };
        cx.counts(json!({ "chapters": pl.chapters.len(), "suggestions": suggestions.len() }));
        to_value(&PlanOut { plan: pl, suggestions, left_out, engine })
    }

    async fn research(&self, cx: &StageCx) -> Result<Value, StageError> {
        let m: Material = typed(cx, "source")?;
        let po: PlanOut = typed(cx, "plan")?;
        let d = self.deps.clone();
        let spaces = d.kb_spaces.clone();
        let cards = tokio::task::spawn_blocking(move || crate::kb::load_cards(&spaces)).await.map_err(join_err)?;
        let env = research::Env {
            http: d.http.clone(),
            cfg: d.research.clone(),
            reader: d.reader.as_ref(),
            kb_cards: cards,
            reader_url: &d.reader_url,
            pdf_dir: cx.dir().join("pdf"),
            web: d.web,
        };
        let counter = cx.counter();
        let r = research::research(&m, &po.plan, env, &counter).await;
        cx.counts(json!({ "cards": r.cards.len(), "sources": r.sources.len(), "imported": r.imported, "pending": r.pending_imports.len(), "offline": r.offline }));
        to_value(&r)
    }

    async fn write(&self, cx: &StageCx) -> Result<Value, StageError> {
        let p = params(cx)?;
        let m: Material = typed(cx, "source")?;
        let po: PlanOut = typed(cx, "plan")?;
        let tr: TermsOut = typed(cx, "terms")?;
        let r: Research = typed(cx, "research")?;
        let review = cx.output("review").unwrap_or(json!({ "skipped": true }));
        let mut pl = plan::apply_review(&po.plan, &review);
        // the paper's figures only when its licence allows reuse
        // the paper's figures only when its licence allows reuse; otherwise a figure of numbers the
        // paper states can still be redrawn as a plot (cited), a diagram is explained in words
        if !r.reuse_figures {
            for c in &mut pl.chapters {
                for f in c.figures.iter_mut().filter(|f| f.kind == "paper") {
                    f.kind = "plot".into();
                    f.idea = format!("redraw with lore-plot only if the paper's figure shows numbers it states (cite them; skip it if it is a diagram): {}", f.idea);
                    f.paper_fig.clear();
                }
            }
        }
        // only what the write stage can make (an older plan may name other kinds)
        for c in &mut pl.chapters {
            c.figures.retain(|f| matches!(f.kind.as_str(), "plot" | "paper"));
        }
        if pl.chapters.is_empty() {
            return Err(StageError::new(422, "沒有要寫的章節"));
        }
        let d = self.deps.clone();
        let global = d.global();
        let on = |n: u64| cx.chars(n);
        let counter = cx.counter();
        let ctx = write::Ctx {
            model: d.model.as_ref(),
            tools: d.tools.as_ref(),
            ws: d.workspace.as_deref(),
            global: &global,
            plot_styles: &d.plot_styles,
            engine: p.engine.as_deref(),
            length: &p.length,
            lang: &p.lang,
            job_dir: cx.dir(),
            on_chars: &on,
            progress: &counter,
        };
        let scratch = d.cache.join("template");
        let ws = d.workspace.clone();
        let template = move || crate::books::template(ws.as_deref(), &scratch);
        let target = write::Target {
            existing: p.book_path.as_deref(),
            folder: p.folder.as_deref(),
            template: &template,
            page: Some(p.page.as_str()).filter(|x| !x.is_empty()),
            books_file: &d.books_file,
        };
        let out = write::write(&ctx, target, &m, &pl, &r, &tr.teach).await?;
        cx.title(&out.title);
        to_value(&out)
    }

    async fn check(&self, cx: &StageCx) -> Result<Value, StageError> {
        let w: WriteOut = typed(cx, "write")?;
        let book = PathBuf::from(&w.book);
        let d = self.deps.clone();
        let b2 = book.clone();
        let mut problems = tokio::task::spawn_blocking(move || check_files(&b2, &d.global())).await.map_err(join_err)?;
        for c in &w.chapters {
            for pr in &c.problems {
                problems.push(Problem { kind: "chapter".into(), message: format!("{}：{pr}", c.title) });
            }
        }
        cx.counts(json!({ "problems": problems.len() }));
        let compiled = self.deps.tools.compile(&book).await;
        for e in compiled.errors.iter().take(10) {
            problems.push(Problem { kind: "compile".into(), message: e.clone() });
        }
        for k in &compiled.undefined_refs {
            problems.push(Problem { kind: "compile".into(), message: format!("未定義的參照：{k}") });
        }
        cx.counts(json!({ "problems": problems.len(), "pages": compiled.pages }));
        to_value(&CheckOut { problems, compiled })
    }
}

/// The terms stage (blocking: reads the KnowledgeBase).
pub fn terms_of(d: &Deps, m: &Material, book: Option<&std::path::Path>) -> TermsOut {
    let kb = lore_learn::load_names(&d.kb_spaces, &d.member);
    let mut tr = lore_learn::find_terms(&m.term_items(), &kb, &d.filters);
    // the card's red terms: added when the text did not give them
    let key = |s: &str| lore_words::phrase_key(s);
    for x in &m.extras.terms {
        let k = key(&x.term);
        if k.is_empty() || tr.terms.iter().any(|t| t.key == k || t.forms.iter().any(|f| key(f) == k)) {
            continue;
        }
        let card = kb.iter().find(|n| key(&n.name) == k);
        let state = card.map(|c| c.state).unwrap_or(lore_learn::Familiar::Unknown);
        tr.terms.push(lore_learn::Term {
            key: card.map(|c| format!("card:{}", c.card)).unwrap_or(k),
            text: x.term.clone(),
            kind: "reader".into(),
            short: None,
            card: card.map(|c| c.card.clone()),
            state,
            label: state.label().into(),
            count: x.n.max(1),
            at: vec![],
            forms: vec![x.term.clone()],
            understood: false,
        });
    }
    tr.total = tr.terms.len();
    tr.unfamiliar = tr.terms.iter().filter(|t| t.unfamiliar()).count();
    // explained already in this book (加進這本書: only new topics)
    let written: Vec<String> = book
        .map(|b| pc_core::open::source_files(b).iter().filter_map(|f| std::fs::read_to_string(f).ok()).flat_map(|t| t.match_indices("\\term{").map(|(i, _)| t[i + 6..].split('}').next().unwrap_or("").to_string()).collect::<Vec<_>>()).map(|t| key(&t)).collect())
        .unwrap_or_default();
    let known = tr.terms.iter().filter(|t| t.state == lore_learn::Familiar::Known).count();
    let mut teach: Vec<Teach> = tr
        .terms
        .iter()
        .filter(|t| t.state != lore_learn::Familiar::Known && !written.contains(&key(&t.text)))
        // citation fragments the PDF text leaves ("Chen et", "et al")
        .filter(|t| { let l = t.text.to_lowercase(); !(l.ends_with(" et") || l.contains(" et al") || l.ends_with(" al")) })
        .map(|t| Teach { term: t.text.clone(), state: if t.state == lore_learn::Familiar::Learning { "learning".into() } else { "unknown".into() }, n: t.count, card: t.card.clone() })
        .collect();
    teach.sort_by(|a, b| b.n.cmp(&a.n).then(a.term.cmp(&b.term)));
    teach.truncate(150);
    // prerequisite links between cards → between the terms they name
    let title_of: BTreeMap<&str, &str> = teach.iter().filter_map(|t| Some((t.card.as_deref()?, t.term.as_str()))).collect();
    let prereq = lore_learn::prerequisites(&d.kb_spaces)
        .into_iter()
        .filter_map(|(a, b)| Some((title_of.get(a.as_str())?.to_string(), title_of.get(b.as_str())?.to_string())))
        .collect();
    TermsOut { terms: tr, teach, prereq, known }
}

/// The registry's checks over the whole book, every `\cite` has an entry, every figure file is there.
pub fn check_files(book: &std::path::Path, global: &lore_notation::Notation) -> Vec<Problem> {
    let mut out = vec![];
    let rel = |p: &std::path::Path| p.strip_prefix(book).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let files: Vec<(String, String)> = pc_core::open::source_files(book).iter().filter_map(|p| Some((rel(p), std::fs::read_to_string(p).ok()?))).collect();
    let doc = lore_notation::Notation::load(&pc_core::paper::app_dir(book).join("notation.json")).unwrap_or_default();
    let eff = lore_notation::Notation::effective(global, &doc);
    let report = lore_notation::check_document(lore_notation::Document::from_files(&files), &eff, &lore_notation::CheckOptions::default());
    for i in report.issues.iter().filter(|i| ["hardcoded_reference", "ref_undefined", "label_"].iter().any(|c| i.code.starts_with(c))) {
        out.push(Problem { kind: "notation".into(), message: i.message.clone() });
    }
    let main = files.first().map(|(_, t)| t.clone()).unwrap_or_default();
    let keys: Vec<String> = crate::bib_files(&main).iter().filter_map(|f| std::fs::read_to_string(book.join(f)).ok()).flat_map(|t| pc_core::bib::parse(&t).0.into_iter().map(|e| e.key)).collect();
    let all: String = files.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
    for k in pc_core::bib::cited_keys(&all) {
        if !keys.contains(&k) {
            out.push(Problem { kind: "cite".into(), message: format!("引用沒有文獻條目：{k}") });
        }
    }
    let mut rest = all.as_str();
    while let Some(i) = rest.find("\\includegraphics") {
        rest = &rest[i + 16..];
        let Some(o) = rest.find('{') else { break };
        let Some(c) = rest[o..].find('}') else { break };
        let path = rest[o + 1..o + c].trim().to_string();
        let found = ["", ".pdf", ".png", ".jpg"].iter().any(|ext| book.join(format!("{path}{ext}")).is_file());
        if !found {
            out.push(Problem { kind: "figure".into(), message: format!("找不到圖檔：{path}") });
        }
        rest = &rest[o + c..];
    }
    out
}
