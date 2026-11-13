//! `research`: what each chapter can stand on. The starting paper (always citable), the
//! author's KnowledgeBase cards (context: what the reader wrote, not citable), other papers in
//! LoreReader's library whose title names a chapter term (with the author's quotes), and
//! open-access papers found on the web (lore-research: OpenAlex, arXiv, Semantic Scholar),
//! downloaded and added to LoreReader's library through its `POST /api/papers/import` — from this
//! server, never from the page. LoreReader not running: the files wait in `pending_imports`.
//! No network: KB cards and LoreReader only, noted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lore_learn::reader::Reader;
use lore_research::{Client, Record, bibtex};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::plan::Plan;
use super::source::Material;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Src {
    /// The BibTeX key (`doi:…`, `arXiv:…`, `name2024word`).
    pub key: String,
    /// `paper` (the starting paper) · `lr` (in LoreReader's library) · `web` (found and downloaded)
    pub kind: String,
    pub record: Record,
    /// Chapter ids it is for (empty = every chapter: the starting paper).
    pub chapters: Vec<String>,
    /// The author's quotes and highlights in it (LoreReader).
    pub quotes: Vec<String>,
    /// The BibTeX entry.
    pub bib: String,
    /// The downloaded PDF (cache tier), web sources only.
    pub file: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CardNote {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub chapters: Vec<String>,
}

/// `research.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Research {
    pub sources: Vec<Src>,
    pub cards: Vec<CardNote>,
    /// The starting paper's key.
    pub start_key: String,
    /// The starting paper's open-access licence and whether its figures may be reproduced.
    pub license: Option<String>,
    pub reuse_figures: bool,
    pub offline: bool,
    pub notes: Vec<String>,
    /// Downloaded PDFs LoreReader has not taken yet (it was not running).
    pub pending_imports: Vec<String>,
    pub imported: usize,
}

impl Research {
    /// Sources a chapter may cite, the starting paper first.
    pub fn for_chapter(&self, id: &str) -> Vec<&Src> {
        self.sources.iter().filter(|s| s.chapters.is_empty() || s.chapters.iter().any(|c| c == id)).collect()
    }
}

/// What the stage reads and talks to.
pub struct Env<'a> {
    pub http: Arc<dyn lore_research::Http>,
    pub cfg: lore_research::Config,
    pub reader: Option<&'a Reader>,
    pub kb_cards: Vec<crate::kb::Card>,
    pub reader_url: &'a str,
    /// Where downloads go (the job folder's `pdf/`).
    pub pdf_dir: PathBuf,
    /// Web search on (off in an offline run).
    pub web: bool,
}

fn record_of(m: &Material) -> Record {
    Record {
        title: m.title.clone(),
        authors: m.authors.clone(),
        year: m.year.parse().ok(),
        venue: Some(m.venue.clone()).filter(|v| !v.is_empty()),
        doi: Some(m.doi.clone()).filter(|d| !d.is_empty()),
        ..Default::default()
    }
}

fn lr_record(v: &Value) -> Record {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(String::from);
    let authors = match v.get("authors") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        Some(Value::String(x)) => x.split([';', '\n']).flat_map(|p| p.split(" and ")).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
        _ => vec![],
    };
    Record {
        title: s("title").unwrap_or_default(),
        authors,
        year: v.get("year").and_then(|y| y.as_i64().or_else(|| y.as_str().and_then(|s| s.parse().ok()))).map(|y| y as i32),
        venue: s("venue"),
        doi: s("doi").map(|d| lore_research::norm_doi(&d)),
        ..Default::default()
    }
}

fn same(a: &Record, b: &Record) -> bool {
    (a.doi.is_some() && a.doi == b.doi) || (a.arxiv.is_some() && a.arxiv == b.arxiv) || lore_research::norm_title(&a.title) == lore_research::norm_title(&b.title)
}

/// Run the stage. `progress(cards, read, found, downloaded)`.
pub async fn research(m: &Material, plan: &Plan, env: Env<'_>, progress: &(dyn Fn(Value) + Send + Sync)) -> Research {
    let mut out = Research::default();
    // a source that answers "too many requests" is left out for the rest of this job (one retry,
    // short waits): a book job does not sit for minutes behind a rate limit
    let mut cfg = env.cfg.clone();
    cfg.retries = cfg.retries.min(1);
    cfg.pace.max_wait = cfg.pace.max_wait.min(std::time::Duration::from_secs(8));
    let mut client = Client::with_http(cfg.clone(), env.http.clone());

    // the starting paper: its licence decides whether its figures may be reproduced
    let mut start = record_of(m);
    if env.web {
        let reference = if m.doi.is_empty() { m.title.clone() } else { m.doi.clone() };
        match client.resolve(&reference).await {
            Ok(Some(r)) if same(&r, &start) || !m.doi.is_empty() => {
                start.license = r.license.clone();
                start.arxiv = start.arxiv.or(r.arxiv);
                start.abstract_text = r.abstract_text;
                start.year = start.year.or(r.year);
            }
            Ok(_) => {}
            Err(e) if e.is_offline() => out.offline = true,
            Err(e) => out.notes.push(format!("查論文的授權沒有成功：{e}")),
        }
    }
    // on arXiv the authors' licence is arXiv's to state (OpenAlex says cc-by for every arXiv copy)
    let arxiv_id = start.arxiv.clone().or_else(|| m.doi.to_lowercase().strip_prefix("10.48550/arxiv.").map(String::from));
    if let Some(id) = arxiv_id.filter(|_| env.web && !out.offline) {
        start.license = match client.arxiv_license(&id).await {
            Ok(Some(l)) => Some(l),
            Ok(None) | Err(_) => {
                out.notes.push("arXiv 沒有回答這篇的授權：不放論文的圖".into());
                None
            }
        };
    }
    out.license = start.license.clone();
    out.reuse_figures = lore_research::figure_reuse_allowed(start.license.as_deref());
    let mut records: Vec<(Src, Record)> = vec![(Src { kind: "paper".into(), quotes: m.quotes.iter().map(|q| q.text.clone()).filter(|t| !t.is_empty()).collect(), ..Default::default() }, start.clone())];

    // the author's cards (context)
    for c in &plan.chapters {
        for t in &c.terms {
            for card in crate::kb::search_in(&env.kb_cards, t, 1) {
                match out.cards.iter_mut().find(|x| x.id == card.id) {
                    Some(x) if !x.chapters.contains(&c.id) => x.chapters.push(c.id.clone()),
                    Some(_) => {}
                    None => out.cards.push(CardNote { id: card.id.clone(), title: card.title.clone(), summary: card.summary.clone(), chapters: vec![c.id.clone()] }),
                }
            }
        }
    }
    progress(serde_json::json!({ "cards": out.cards.len() }));

    // papers the author has read in LoreReader
    if let Some(r) = env.reader {
        let ws = r.papers.parent().unwrap_or(Path::new("")).to_path_buf();
        for p in r.list() {
            let sha = p["sha256"].as_str().unwrap_or("").to_string();
            if sha == m.sha {
                continue;
            }
            let rec = lr_record(&p);
            if rec.title.is_empty() || same(&rec, &start) {
                continue;
            }
            let lt = rec.title.to_lowercase();
            let chapters: Vec<String> = plan.chapters.iter().filter(|c| c.terms.iter().any(|t| t.chars().count() > 2 && lt.contains(&t.to_lowercase()))).map(|c| c.id.clone()).collect();
            if chapters.is_empty() {
                continue;
            }
            let quotes = crate::reader::material(&ws, &sha).into_iter().map(|q| q.text).filter(|t| !t.is_empty()).take(8).collect();
            records.push((Src { kind: "lr".into(), chapters, quotes, ..Default::default() }, rec));
        }
    }
    progress(serde_json::json!({ "read": records.len() - 1 }));

    // the web: open-access papers only
    if env.web && !out.offline {
        'chapters: for c in &plan.chapters {
            let mut found: Vec<Record> = vec![];
            for q in c.queries.iter().take(2) {
                let f = client.search(q, 5).await;
                if f.offline() {
                    out.offline = true;
                    break 'chapters;
                }
                let mut dropped = false;
                for (src, e) in &f.errors {
                    let line = format!("{}：{e}", src.label());
                    if !out.notes.contains(&line) {
                        out.notes.push(line);
                    }
                    if matches!(e, lore_research::Error::RateLimited(_)) {
                        cfg.sources.retain(|s| s != src);
                        dropped = true;
                    }
                }
                if dropped {
                    client = Client::with_http(cfg.clone(), env.http.clone());
                }
                if cfg.sources.is_empty() {
                    break 'chapters;
                }
                found.extend(f.records.into_iter().filter(|r| r.open_access()));
            }
            let mut found = lore_research::dedupe(found);
            found.sort_by(|a, b| b.citations.unwrap_or(0).cmp(&a.citations.unwrap_or(0)));
            let mut taken = 0;
            for r in found {
                if taken == 2 {
                    break;
                }
                match records.iter_mut().find(|(_, x)| same(x, &r)) {
                    Some((s, _)) if s.kind == "paper" => {}
                    Some((s, _)) => {
                        if !s.chapters.is_empty() && !s.chapters.contains(&c.id) {
                            s.chapters.push(c.id.clone());
                        }
                    }
                    None => {
                        records.push((Src { kind: "web".into(), chapters: vec![c.id.clone()], ..Default::default() }, r));
                        taken += 1;
                    }
                }
            }
            progress(serde_json::json!({ "found": records.iter().filter(|(s, _)| s.kind == "web").count() }));
        }
    }
    if out.offline {
        out.notes.push("沒有網路：只用了知識卡與 LoreReader 的論文".into());
    }

    // download the web papers, then hand them to LoreReader
    let mut files = vec![];
    for (s, r) in records.iter_mut().filter(|(s, _)| s.kind == "web") {
        match client.download(r, &env.pdf_dir).await {
            Ok(d) => {
                s.file = Some(d.path.to_string_lossy().to_string());
                files.push(d.path);
            }
            Err(e) if e.is_offline() => {
                out.offline = true;
                break;
            }
            Err(e) => out.notes.push(format!("{}：{e}", r.title.chars().take(60).collect::<String>())),
        }
        progress(serde_json::json!({ "downloaded": files.len() }));
    }
    if !files.is_empty() {
        let rep = lore_research::import_into_reader(&*env.http, env.reader_url, &files).await;
        out.imported = rep.imported.len();
        out.pending_imports = rep.pending.iter().map(|p| p.to_string_lossy().to_string()).collect();
        for (p, e) in rep.failed {
            out.notes.push(format!("LoreReader 沒有收下 {}：{e}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        }
        if !out.pending_imports.is_empty() {
            out.notes.push(format!("LoreReader 沒有開啟：{} 篇等開啟後加進書庫", out.pending_imports.len()));
        }
    }

    // keys and BibTeX entries, unique in the set
    let recs: Vec<Record> = records.iter().map(|(_, r)| r.clone()).collect();
    let keys = bibtex::keys(&recs);
    for ((mut s, r), k) in records.into_iter().zip(keys) {
        s.bib = bibtex::entry(&r, &k);
        s.key = k;
        s.record = r;
        out.sources.push(s);
    }
    out.start_key = out.sources.first().map(|s| s.key.clone()).unwrap_or_default();
    out
}

/// Hand the waiting PDFs to LoreReader again (the page opened it). Returns what is still waiting.
pub async fn retry_imports(http: &dyn lore_research::Http, reader_url: &str, pending: &[String]) -> (usize, Vec<String>, Vec<String>) {
    let files: Vec<PathBuf> = pending.iter().map(PathBuf::from).filter(|p| p.is_file()).collect();
    let rep = lore_research::import_into_reader(http, reader_url, &files).await;
    let failed = rep.failed.iter().map(|(p, e)| format!("{}：{e}", p.display())).collect();
    (rep.imported.len(), rep.pending.iter().map(|p| p.to_string_lossy().to_string()).collect(), failed)
}
