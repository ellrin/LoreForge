//! `research`: what each chapter can stand on. The starting paper (always citable), the
//! author's L-Brain cards (context: what the reader wrote, not citable), other papers in
//! L-Reader's library whose title names a chapter term (with the author's quotes), and
//! open-access papers found on the web (lore-research: OpenAlex, arXiv, Semantic Scholar),
//! downloaded and added to L-Reader's library through its `POST /api/papers/import` — from this
//! server, never from the page. L-Reader not running: the files wait in `pending_imports`.
//! No network: KB cards and L-Reader only, noted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lore_learn::reader::Reader;
use lore_research::{Client, Record, bibtex};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::plan::Plan;
use super::source::{Material, Sources};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Src {
    /// The BibTeX key (`doi:…`, `arXiv:…`, `name2024word`).
    pub key: String,
    /// `paper` (the starting paper) · `lr` (in L-Reader's library) · `web` (found and downloaded)
    pub kind: String,
    pub record: Record,
    /// Chapter ids it is for (empty = every chapter: the starting paper).
    pub chapters: Vec<String>,
    /// The author's quotes and highlights in it (L-Reader).
    pub quotes: Vec<String>,
    /// The BibTeX entry.
    pub bib: String,
    /// The downloaded PDF (cache tier), web sources only.
    pub file: Option<String>,
    /// `paper`: which starting material it is (`source.json`'s `materials`).
    pub material: Option<usize>,
    /// `paper`: its licence lets its figures be reproduced.
    pub reuse: bool,
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
    /// Downloaded PDFs L-Reader has not taken yet (it was not running).
    pub pending_imports: Vec<String>,
    pub imported: usize,
    /// 追一層: terms the found papers use that the author does not know yet and the book does
    /// not teach, per chapter id (offered in the review window, unticked).
    pub found: std::collections::BTreeMap<String, Vec<Found>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Found {
    pub term: String,
    /// `unknown` · `learning`
    pub state: String,
    /// The found paper's title.
    pub from: String,
}

/// One more layer of terms (blocking: no network): the term finder over what research kept for
/// each found paper (its abstract and the author's quotes), against the knowledge base. Terms the
/// book already teaches (`taught`), known ones and repeats are left out; at most `per` a chapter.
pub fn found_terms(r: &Research, kb: &[lore_learn::KbName], filters: &lore_words::Filters, taught: &[String], per: usize) -> std::collections::BTreeMap<String, Vec<Found>> {
    let key = |s: &str| lore_words::phrase_key(s);
    let taught: Vec<String> = taught.iter().map(|t| key(t)).collect();
    let mut out: std::collections::BTreeMap<String, Vec<Found>> = Default::default();
    let mut seen: Vec<String> = vec![];
    for s in r.sources.iter().filter(|s| matches!(s.kind.as_str(), "web" | "lr")) {
        let mut items = vec![];
        if let Some(a) = s.record.abstract_text.as_deref().filter(|a| !a.trim().is_empty()) {
            items.push(("Abstract".to_string(), a.to_string()));
        }
        for q in &s.quotes {
            items.push(("Quote".to_string(), q.clone()));
        }
        if items.is_empty() {
            continue;
        }
        let tr = lore_learn::find_terms(&items, kb, filters);
        for t in tr.terms.iter().filter(|t| t.state != lore_learn::Familiar::Known) {
            let k = key(&t.text);
            let l = t.text.to_lowercase();
            if k.is_empty() || taught.contains(&k) || seen.contains(&k) || l.ends_with(" et") || l.contains(" et al") || l.ends_with(" al") {
                continue;
            }
            let Some(ch) = s.chapters.first() else { continue };
            let list = out.entry(ch.clone()).or_default();
            if list.len() >= per {
                continue;
            }
            seen.push(k);
            let state = if t.state == lore_learn::Familiar::Learning { "learning" } else { "unknown" };
            list.push(Found { term: t.text.clone(), state: state.into(), from: s.record.title.clone() });
        }
    }
    out
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
        title: if m.title.is_empty() { m.label.trim_end_matches(".pdf").trim_end_matches(".PDF").to_string() } else { m.title.clone() },
        arxiv: Some(m.arxiv.clone()).filter(|a| !a.is_empty()),
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
    let t = lore_research::norm_title(&a.title);
    (a.doi.is_some() && a.doi == b.doi) || (a.arxiv.is_some() && a.arxiv == b.arxiv) || (!t.is_empty() && t == lore_research::norm_title(&b.title))
}

/// Which chapters each starting paper serves: one paper serves every chapter (empty); several are
/// given to the chapters whose terms they use most (at most `per` a chapter). One that no chapter
/// takes stays empty and is not a source (its terms were read; a folder of a hundred PDFs never
/// fills one prompt or the reference library).
pub fn assign(src: &Sources, plan: &Plan, per: usize) -> Vec<Vec<String>> {
    let n = src.materials.len();
    if n <= 1 {
        return vec![vec![]; n];
    }
    let texts: Vec<String> = src.materials.iter().map(|m| format!("{} {} {}", m.title, m.abstract_text, m.sections.iter().map(|(h, t)| format!("{h} {t}")).collect::<Vec<_>>().join(" ")).to_lowercase()).collect();
    let mut out = vec![vec![]; n];
    for c in &plan.chapters {
        let mut scored: Vec<(usize, usize)> = texts.iter().enumerate().map(|(i, t)| (c.terms.iter().filter(|w| w.chars().count() > 2 && t.contains(&w.to_lowercase())).count(), i)).filter(|(k, _)| *k > 0).collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, i) in scored.into_iter().take(per) {
            out[i].push(c.id.clone());
        }
    }
    out
}

/// Run the stage. `progress(cards, read, found, downloaded)`.
pub async fn research(src: &Sources, plan: &Plan, env: Env<'_>, progress: &(dyn Fn(Value) + Send + Sync)) -> Research {
    let mut out = Research::default();
    // a source that answers "too many requests" is left out for the rest of this job (one retry,
    // short waits): a book job does not sit for minutes behind a rate limit
    let mut cfg = env.cfg.clone();
    cfg.retries = cfg.retries.min(1);
    cfg.pace.max_wait = cfg.pace.max_wait.min(std::time::Duration::from_secs(8));
    let mut client = Client::with_http(cfg.clone(), env.http.clone());

    // the starting papers: one reference each among the found ones; the licence is asked only for
    // a paper whose figures could be reproduced (an L-Reader paper with figure crops)
    let chapters_of = assign(src, plan, 6);
    let mut records: Vec<(Src, Record)> = vec![];
    let mut first_licence: Option<(Option<String>, bool)> = None;
    let mut unused = 0;
    for (i, m) in src.materials.iter().enumerate() {
        if src.materials.len() > 1 && chapters_of[i].is_empty() {
            unused += 1;
            continue;
        }
        let mut start = record_of(m);
        let has_figures = m.figures.iter().any(|f| f.file.is_some());
        if env.web && !out.offline && has_figures {
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
            // on arXiv the authors' licence is arXiv's to state (OpenAlex says cc-by for every arXiv copy)
            let arxiv_id = start.arxiv.clone().or_else(|| m.doi.to_lowercase().strip_prefix("10.48550/arxiv.").map(String::from));
            if let Some(id) = arxiv_id.filter(|_| !out.offline) {
                start.license = match client.arxiv_license(&id).await {
                    Ok(Some(l)) => Some(l),
                    Ok(None) | Err(_) => {
                        out.notes.push("arXiv 沒有回答這篇的授權：不放論文的圖".into());
                        None
                    }
                };
            }
        }
        if start.abstract_text.as_deref().is_none_or(str::is_empty) && !m.abstract_text.is_empty() {
            start.abstract_text = Some(m.abstract_text.clone());
        }
        let reuse = has_figures && lore_research::figure_reuse_allowed(start.license.as_deref());
        if has_figures && first_licence.is_none() {
            first_licence = Some((start.license.clone(), reuse));
        }
        let quotes = m.quotes.iter().map(|q| q.text.clone()).filter(|t| !t.is_empty()).collect();
        records.push((Src { kind: "paper".into(), quotes, chapters: chapters_of[i].clone(), material: Some(i), reuse, ..Default::default() }, start));
    }
    if let Some((l, r)) = first_licence {
        out.license = l;
        out.reuse_figures = r;
    }
    if unused > 0 {
        out.notes.push(format!("{unused} 篇起始論文沒有章節用到，不列為文獻（名詞已經讀過）"));
    }
    let starts: Vec<Record> = records.iter().map(|(_, r)| r.clone()).collect();

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

    // papers the author has read in L-Reader
    if let Some(r) = env.reader {
        let ws = r.papers.parent().unwrap_or(Path::new("")).to_path_buf();
        for p in r.list() {
            let sha = p["sha256"].as_str().unwrap_or("").to_string();
            if src.materials.iter().any(|m| m.sha == sha) {
                continue;
            }
            let rec = lr_record(&p);
            if rec.title.is_empty() || starts.iter().any(|s| same(&rec, s)) {
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
    progress(serde_json::json!({ "read": records.iter().filter(|(s, _)| s.kind == "lr").count() }));

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
                    if matches!(e, lore_research::Error::RateLimited(_) | lore_research::Error::Quota(..)) {
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
        out.notes.push("沒有網路：只用了知識卡與 L-Reader 的論文".into());
    }

    // download the web papers, then hand them to L-Reader
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
            out.notes.push(format!("L-Reader 沒有收下 {}：{e}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        }
        if !out.pending_imports.is_empty() {
            out.notes.push(format!("L-Reader 沒有開啟：{} 篇等開啟後加進書庫", out.pending_imports.len()));
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

/// Hand the waiting PDFs to L-Reader again (the page opened it). Returns what is still waiting.
pub async fn retry_imports(http: &dyn lore_research::Http, reader_url: &str, pending: &[String]) -> (usize, Vec<String>, Vec<String>) {
    let files: Vec<PathBuf> = pending.iter().map(PathBuf::from).filter(|p| p.is_file()).collect();
    let rep = lore_research::import_into_reader(http, reader_url, &files).await;
    let failed = rep.failed.iter().map(|(p, e)| format!("{}：{e}", p.display())).collect();
    (rep.imported.len(), rep.pending.iter().map(|p| p.to_string_lossy().to_string()).collect(), failed)
}
