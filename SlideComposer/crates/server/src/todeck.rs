//! 論文報告: a paper (a PaperComposer LaTeX folder or a LoreReader PDF) becomes a
//! paper-report deck in one action. Read-only on the paper; the deck is a normal deck.
//!
//! 1. **Material** (`lore-docsrc`): PaperComposer → `latex::extract_dir` (sections, figure
//!    files, table grids, equations with labels, bibliography) and the paper's own
//!    `notation.json`; LoreReader → `pdf::extract` on the library PDF (figure / table crops,
//!    sections, references; pages cached in this workspace's `cache/todeck/`) and
//!    LoreReader's `symbols.json`. Nothing is written into the paper or LoreReader.
//! 2. **Plan** (lore-llm task `sc-paper-deck`, class `create`): one JSON reply with the
//!    slides in the author's paper-report order (design §7.1: cover → background →
//!    challenge / big picture → one method module per slide → loss → training details →
//!    results → ablation → visualisation → opinions → take home → thanks), referring to
//!    the material by id; the model never places anything.
//! 3. **Module boxes** of the main architecture figure (lore-llm `sc-crop-modules`, a vision
//!    engine) when one is available: method slides show the whole figure dimmed with the
//!    current module cropped back on top, framed; without boxes the figure is shown whole.
//! 4. **Build**: page kinds from the base (cover / content / end), the slot geometry of the
//!    author's arrangements (`arrangements/*.json`, `class: academic`; the base's content
//!    area when none fits), text in the style's tags, native tables (training details as
//!    key-value, results from the LaTeX grid), equations as native OMML (lore-math), `Ref:`
//!    lines from the cited bibliography, and the big-picture slide expanded with the reveal
//!    generator (one slide per numbered step). The paper's notation goes into the deck's
//!    `notation.json` with the paper's ids.
//!
//! | Method | Path | |
//! |---|---|---|
//! | GET | /api/todeck/papers | `{pc: [{path, title}], lr: [{sha, title, pdf}]}` |
//! | POST | /api/todeck | `{source: {kind: pc, path} \| {kind: lr, sha}, length, lang, style, engine?}` → `{name, slides, engine, warnings}` |
//!
//! Both answer cross-origin to pages on 127.0.0.1 / localhost (PaperComposer's 做成簡報).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lore_canvas::reveal::{RevealSpec, reveal};
use lore_canvas::{Bullet, Cell, Crop, Frame, Geom, Kind, Look, Object, Op, Para, Paint, PicObj, Row, Run, Slide, SpObj, Stroke, TableObj, TextBody};
use lore_docsrc::{AssetCatalog, AssetKind};
use lore_llm::LlmRequest;
use lore_notation::Notation;
use lore_canvas::StyleSystem;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::llm::{Llm, TASK_PAPER_DECK};
use crate::reader::{Reader, png_size, valid_sha};
use crate::{ApiError, R, Shared, bad};

const EMU_IN: i64 = 914_400;
/// "No Style, Table Grid" — the author's table look (design §7.1).
const TABLE_GRID: &str = "{5940675A-B579-460E-94D1-54222C63F5DA}";

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/todeck/papers", get(papers)).route("/api/todeck", post(make))
}

// ── Where the papers are ─────────────────────────────────────────────────────

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

/// The OS settings folder (what `dirs::config_dir` returns).
fn config_dir() -> Option<PathBuf> {
    let home = env("HOME").or_else(|| env("USERPROFILE")).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        env("APPDATA").map(PathBuf::from)
    } else {
        env("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")))
    }
}

/// `key = "…"` at the top level of a small TOML file.
fn toml_top(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            break;
        }
        let Some(rest) = l.strip_prefix(key) else { continue };
        let Some(v) = rest.trim_start().strip_prefix('=') else { continue };
        let v = v.trim();
        let q = v.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let end = v[1..].find(q)?;
        return Some(v[1..1 + end].to_string()).filter(|s| !s.is_empty());
    }
    None
}

/// PaperComposer's workspace, found like PaperComposer does (read-only here):
/// `PAPERCOMPOSER_WORKSPACE` → `workspace_path` in its config.toml → LoreForge `private/PaperComposer`.
fn pc_workspace(home: &lore_home::Home) -> Option<PathBuf> {
    if let Some(w) = env("PAPERCOMPOSER_WORKSPACE") {
        return Some(PathBuf::from(w));
    }
    let cfg = config_dir().map(|d| d.join("PaperComposer/config.toml"));
    if let Some(w) = cfg.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml_top(&t, "workspace_path")) {
        return Some(PathBuf::from(w));
    }
    home.app_dir("PaperComposer")
}

/// Folders PaperComposer opened where they live (`opened.toml`: `[[paper]] path = "…"`).
fn pc_opened() -> Vec<PathBuf> {
    let file = env("PAPERCOMPOSER_OPENED").map(PathBuf::from).or_else(|| config_dir().map(|d| d.join("PaperComposer/opened.toml")));
    let Some(text) = file.and_then(|f| std::fs::read_to_string(f).ok()) else { return vec![] };
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("path").map(str::trim))
        .filter_map(|v| v.strip_prefix('=').map(str::trim))
        .filter_map(|v| v.strip_prefix('"').and_then(|v| v.strip_suffix('"')))
        .map(|v| PathBuf::from(v.replace("\\\\", "\\")))
        .collect()
}

/// A LaTeX paper folder: has a main .tex (`lore_docsrc::latex::find_main`).
fn is_paper(dir: &Path) -> bool {
    dir.is_dir() && lore_docsrc::latex::find_main(dir).is_ok()
}

/// `\title{…}` of the main file, else the folder name.
fn quick_title(dir: &Path) -> String {
    let main = lore_docsrc::latex::find_main(dir).ok().and_then(|m| std::fs::read_to_string(dir.join(m)).ok()).unwrap_or_default();
    let t = main.find("\\title").and_then(|i| {
        let s = &main[i + 6..];
        let s = &s[s.find('{')? + 1..];
        let mut depth = 1;
        let end = s.char_indices().find(|&(_, c)| {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
            }
            depth == 0
        })?;
        Some(lore_docsrc::latex::plain::tidy(&s[..end.0]))
    });
    t.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

/// PaperComposer's papers: the workspace's `papers/*` and the folders it opened.
pub fn pc_papers(home: &lore_home::Home) -> Vec<Value> {
    let mut dirs: Vec<PathBuf> = vec![];
    if let Some(ws) = pc_workspace(home) {
        let mut v: Vec<PathBuf> = std::fs::read_dir(ws.join("papers")).into_iter().flatten().flatten().map(|e| e.path()).collect();
        v.sort();
        dirs.extend(v);
    }
    dirs.extend(pc_opened());
    let mut seen = BTreeSet::new();
    dirs.into_iter()
        .filter(|d| is_paper(d) && seen.insert(d.clone()))
        .map(|d| json!({ "path": d, "title": quick_title(&d), "name": d.file_name().map(|n| n.to_string_lossy().to_string()) }))
        .collect()
}

async fn papers(State(s): State<Shared>) -> R {
    let home = s.home.clone();
    let (pc, lr) = tokio::task::spawn_blocking(move || {
        let pc = pc_papers(&home);
        let lr: Vec<Value> = Reader::discover(&home)
            .map(|r| r.list().iter().map(|m| json!({ "sha": m["sha256"], "title": crate::reader::title_of(m), "pdf": r.pdf(m["sha256"].as_str().unwrap_or("")).is_some() })).collect())
            .unwrap_or_default();
        (pc, lr)
    })
    .await
    .map_err(|e| bad(e.to_string()))?;
    Ok(Json(json!({ "pc": pc, "lr": lr })))
}

// ── Material ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Img {
    pub bytes: Vec<u8>,
    pub ext: String,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Fig {
    pub id: String,
    pub caption: String,
    pub image: Option<Img>,
    /// Table cells (LaTeX tables).
    pub grid: Option<Vec<Vec<String>>>,
}

#[derive(Debug, Clone, Default)]
pub struct Eq {
    pub id: String,
    pub latex: String,
    pub label: Option<String>,
}

/// Everything the plan and the build use, from either source.
#[derive(Debug, Clone, Default)]
pub struct Material {
    pub title: String,
    pub authors: Vec<String>,
    pub venue: String,
    pub year: String,
    pub keywords: Vec<String>,
    pub abstract_text: String,
    pub sections: Vec<(String, String)>,
    pub figures: Vec<Fig>,
    pub tables: Vec<Fig>,
    pub equations: Vec<Eq>,
    /// Bibliography key → short `Ref:` text.
    pub refs: BTreeMap<String, String>,
    /// `Ref:` line of the paper itself (cover).
    pub self_ref: String,
    /// The paper's own notation file (PaperComposer) — registered into the deck with its ids.
    pub notation: Notation,
    /// LoreReader's symbols.json (its rows are registered through `reader::register_into`).
    pub lr_symbols: Option<Value>,
    /// Symbols of the paper's registry (LaTeX, meaning), for the notes under an equation.
    pub symbols: Vec<(String, String)>,
    /// Name for the deck folder.
    pub slug: String,
}

/// Width and height of a JPEG (SOFn marker).
pub fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

fn img_from(bytes: Vec<u8>, ext: &str) -> Option<Img> {
    let (w, h) = match ext {
        "png" => png_size(&bytes)?,
        "jpg" | "jpeg" => jpeg_size(&bytes)?,
        _ => return None,
    };
    Some(Img { bytes, ext: if ext == "jpeg" { "jpg".into() } else { ext.into() }, w, h })
}

/// A figure file of the paper as a picture: PNG / JPEG as they are, a PDF figure's first
/// page through `pdftoppm` (rendered into `cache`, never next to the paper).
fn figure_file(path: &Path, cache: &Path) -> Option<Img> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" => img_from(std::fs::read(path).ok()?, &ext),
        "pdf" => {
            let tool = crate::reader::pdftoppm()?;
            let bytes = std::fs::read(path).ok()?;
            let key = format!("{:016x}", lore_canvas::fnv(&bytes));
            let png = crate::reader::render_page(&tool, path, &key, 1, 200, cache).ok()?;
            img_from(png, "png")
        }
        _ => None,
    }
}

/// `Surname et al., Title, Venue, Year.` from BibTeX fields.
fn short_ref(fields: &BTreeMap<String, String>, text: &str) -> String {
    let f = |k: &str| fields.get(k).map(|v| lore_docsrc::latex::plain::tidy(&v.replace(['{', '}'], ""))).unwrap_or_default();
    let authors = f("author");
    if authors.is_empty() {
        return text.trim().to_string();
    }
    let list: Vec<&str> = authors.split(" and ").map(str::trim).filter(|a| !a.is_empty()).collect();
    let surname = |a: &str| a.split(',').next().map(str::trim).filter(|_| a.contains(',')).map(String::from).unwrap_or_else(|| a.split_whitespace().last().unwrap_or(a).to_string());
    let who = match list.len() {
        0 => String::new(),
        1 => surname(list[0]),
        2 => format!("{} and {}", surname(list[0]), surname(list[1])),
        _ => format!("{} et al.", surname(list[0])),
    };
    let venue = [f("journal"), f("booktitle")].into_iter().find(|v| !v.is_empty()).unwrap_or_default();
    [who, f("title"), venue, f("year")].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ")
}

/// A folder name from a title: letters, digits and dashes, at most ~6 words.
pub fn slug(title: &str) -> String {
    let s: String = title.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let words: Vec<&str> = s.split_whitespace().take(6).collect();
    let out = words.join("-");
    if out.is_empty() { "paper-report".into() } else { out }
}

fn paragraphs_of(cat: &AssetCatalog) -> Vec<(String, String)> {
    cat.sections
        .iter()
        .filter(|s| !s.paragraphs.is_empty())
        .map(|s| {
            let head = match &s.number {
                Some(n) => format!("{n} {}", s.title),
                None => s.title.clone(),
            };
            (head, s.paragraphs.join("\n"))
        })
        .collect()
}

/// A PaperComposer paper folder (LaTeX). `cache`: where rendered figure PDFs go.
pub fn from_latex(dir: &Path, cache: &Path) -> Result<Material, String> {
    let cat = lore_docsrc::latex::extract_dir(dir).map_err(|e| format!("讀不到論文：{e}"))?;
    let mut m = Material {
        title: cat.meta.title.clone(),
        authors: cat.meta.authors.clone(),
        keywords: cat.meta.keywords.clone(),
        abstract_text: cat.meta.abstract_text.clone().unwrap_or_default(),
        sections: paragraphs_of(&cat),
        ..Default::default()
    };
    if m.title.trim().is_empty() {
        m.title = quick_title(dir);
    }
    for a in &cat.assets {
        match a.kind {
            AssetKind::Figure => {
                let image = a.files.iter().find_map(|f| figure_file(&dir.join(f), cache));
                m.figures.push(Fig { id: a.id.clone(), caption: a.caption.clone().unwrap_or_default(), image, grid: None });
            }
            AssetKind::Table => m.tables.push(Fig { id: a.id.clone(), caption: a.caption.clone().unwrap_or_default(), image: None, grid: a.grid.clone().filter(|g| !g.is_empty()) }),
            AssetKind::Equation => {
                if let Some(t) = a.text.as_deref().filter(|t| !t.trim().is_empty()) {
                    m.equations.push(Eq { id: a.id.clone(), latex: t.trim().to_string(), label: a.label.clone() });
                }
            }
            _ => {}
        }
    }
    for r in &cat.references {
        m.refs.insert(r.key.clone(), short_ref(&r.fields, &r.text));
    }
    m.notation = Notation::load(&dir.join("notation.json")).unwrap_or_default();
    m.symbols = m
        .notation
        .entries
        .iter()
        .filter(|e| matches!(e.kind(), lore_notation::Kind::Symbol | lore_notation::Kind::Operator))
        .filter_map(|e| Some((e.latex.clone()?, e.meaning.clone().or_else(|| e.meaning_en.clone())?)))
        .collect();
    let who = match m.authors.len() {
        0 => String::new(),
        1 => m.authors[0].clone(),
        _ => format!("{} et al.", m.authors[0]),
    };
    m.self_ref = format!("Ref: {}", [who, m.title.clone()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", "));
    m.slug = slug(&m.title);
    Ok(m)
}

/// A LoreReader paper (its library PDF): crops through lore-docsrc's PDF extraction.
pub fn from_lorereader(r: &Reader, sha: &str, cache: &Path) -> Result<Material, String> {
    let meta = r.paper_json(sha).ok_or("LoreReader 沒有這篇論文")?;
    let pdf = r.pdf(sha).ok_or("LoreReader 的書庫裡沒有這篇的 PDF")?;
    let out = cache.join(sha);
    let mut o = lore_docsrc::pdf::PdfOptions::new(&out);
    o.crop_dpi = 200;
    let cat = lore_docsrc::pdf::extract(&pdf, &o).map_err(|e| format!("PDF 無法分析：{e}"))?;
    let s = |k: &str| meta.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let authors: Vec<String> = match meta.get("authors") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        Some(Value::String(x)) => x.split([';', '\n']).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
        _ => cat.meta.authors.clone(),
    };
    let year = match meta.get("year") {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(x)) => x.clone(),
        _ => String::new(),
    };
    let mut m = Material {
        title: crate::reader::title_of(&meta),
        authors,
        venue: s("venue"),
        year,
        keywords: cat.meta.keywords.clone(),
        abstract_text: cat.meta.abstract_text.clone().unwrap_or_default(),
        sections: paragraphs_of(&cat),
        self_ref: crate::reader::ref_line(&meta),
        lr_symbols: r.symbols_json(sha),
        ..Default::default()
    };
    let crop = |a: &lore_docsrc::Asset| a.crop.as_ref().and_then(|c| std::fs::read(out.join(c)).ok()).and_then(|b| img_from(b, "png"));
    for a in &cat.assets {
        match a.kind {
            AssetKind::Figure => m.figures.push(Fig { id: a.id.clone(), caption: a.caption.clone().unwrap_or_default(), image: crop(a), grid: None }),
            AssetKind::Table => m.tables.push(Fig { id: a.id.clone(), caption: a.caption.clone().unwrap_or_default(), image: crop(a), grid: None }),
            _ => {}
        }
    }
    // equations: LoreReader's own LaTeX (symbols.json), ids kept
    for e in m.lr_symbols.as_ref().and_then(|v| v["equations"].as_array().cloned()).unwrap_or_default() {
        let (Some(id), Some(latex)) = (e["id"].as_str(), e["latex"].as_str()) else { continue };
        if !latex.trim().is_empty() {
            m.equations.push(Eq { id: id.to_string(), latex: latex.trim().to_string(), label: None });
        }
    }
    for r in &cat.references {
        m.refs.insert(r.key.clone(), r.text.trim().to_string());
    }
    m.symbols = m
        .lr_symbols
        .as_ref()
        .and_then(|v| v["entries"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| Some((e["latex"].as_str()?.to_string(), e["meaning"].as_str().or_else(|| e["meaning_en"].as_str())?.to_string())))
        .filter(|(l, m)| !l.trim().is_empty() && !m.trim().is_empty())
        .collect();
    m.slug = slug(&m.title);
    Ok(m)
}

// ── The plan (one model call) ────────────────────────────────────────────────

/// Page roles of a paper report, in the author's order (design §7.1).
pub const ROLES: [&str; 13] = ["cover", "background", "challenge", "overview", "method", "loss", "training", "results", "ablation", "visual", "opinion", "takeaway", "thanks"];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PlanSlide {
    pub role: String,
    pub title: String,
    pub bullets: Vec<String>,
    pub figure: String,
    pub table: String,
    pub equations: Vec<String>,
    pub takeaway: String,
    pub steps: Vec<String>,
    pub module: u32,
    pub kv: Vec<Vec<String>>,
    pub cites: Vec<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub venue: String,
    pub keywords: Vec<String>,
    pub slides: Vec<PlanSlide>,
}

pub fn target_slides(length: &str) -> (usize, usize) {
    match length {
        "short" => (9, 11),
        "long" => (24, 30),
        _ => (15, 20),
    }
}

pub fn schema() -> Value {
    let s = json!({ "type": "string" });
    let list = json!({ "type": "array", "items": { "type": "string" } });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["venue", "keywords", "slides"],
        "properties": {
            "venue": s, "keywords": list,
            "slides": { "type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["role", "title", "bullets"],
                "properties": {
                    "role": { "type": "string", "enum": ROLES },
                    "title": s, "bullets": list, "figure": s, "table": s, "equations": list,
                    "takeaway": s, "steps": list, "module": { "type": "integer" },
                    "kv": { "type": "array", "items": list }, "cites": list, "notes": s
                }
            } }
        }
    })
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n).collect::<String>() + "…" }
}

pub fn prompt(m: &Material, length: &str, lang: &str) -> (String, String) {
    let (lo, hi) = target_slides(length);
    let language = if lang == "en" { "English" } else { "Traditional Chinese as used in Taiwan (keep technical terms, model names and metrics in English)" };
    let system = format!(
        "You plan a paper-report seminar deck about one research paper, the way the presenter always builds them. \
A program lays out the slides from your plan: you decide WHAT each slide says and which of the paper's figures, tables and equations it shows, never where things go.\n\n\
Reply with one JSON object {{venue, keywords, slides}}. {lo}–{hi} slides in this order (skip a role the paper has no material for, repeat method / results when needed):\n\
- cover: the paper title as title; venue = journal or conference and year if the text says so (else empty); keywords = 3–6.\n\
- background (1–2): the field or clinical context.\n\
- challenge: the problem and the intuition.\n\
- overview: the big picture — figure = the main architecture figure; steps = 3–6 short numbered steps of the method, in order (each later gets its own method slide).\n\
- method: one per step, module = the step number (1-based); figure = the same architecture figure; equations = ids of the equations of that module; bullets explain it.\n\
- loss: the training objective; equations = the loss equation ids.\n\
- training: training details; kv = [[key, value], …] rows (data, model, optimiser, learning rate, epochs, batch size, hardware…), only values the paper states.\n\
- results: table = the main results table id (or figure); takeaway = one sentence reading the result.\n\
- ablation: table or figure id; takeaway = one sentence.\n\
- visual: qualitative figure id; bullets about what it shows, including failure cases if any.\n\
- opinion: title \"Some opinions\"; the presenter's critical view (strengths, limits, questions).\n\
- takeaway: title \"Take home messages\"; 3–5 numbered messages in bullets.\n\
- thanks: title \"Thanks\".\n\n\
Rules:\n\
- Titles are short noun phrases (\"Results of ablation study\", \"Sampling method: …\"); no agenda slide.\n\
- Bullets: short, spoken, start with a verb; at most 5 per slide, at most 90 characters each. Inline math between $ signs as LaTeX.\n\
- Never write figure, table or equation numbers (no \"Fig. 3\", \"Table 2\", \"Eq. (1)\"): the slide shows the item itself.\n\
- figure / table / equations use ONLY ids from the material lists; \"\" or [] when none.\n\
- cites: bibliography keys from the list for background claims from other work.\n\
- notes: one or two sentences the presenter says (speaker notes).\n\
- Write all slide text in {language}."
    );
    let mut u = String::new();
    u += &format!("Title: {}\n", m.title);
    if !m.authors.is_empty() {
        u += &format!("Authors: {}\n", m.authors.join(", "));
    }
    if !m.venue.is_empty() {
        u += &format!("Venue: {} {}\n", m.venue, m.year);
    }
    if !m.keywords.is_empty() {
        u += &format!("Keywords: {}\n", m.keywords.join(", "));
    }
    if !m.abstract_text.is_empty() {
        u += &format!("\nAbstract:\n{}\n", clip(&m.abstract_text, 2500));
    }
    let budget = 22_000usize;
    let per = (budget / m.sections.len().max(1)).clamp(600, 4000);
    u += "\n## Sections\n";
    for (h, t) in &m.sections {
        u += &format!("\n### {h}\n{}\n", clip(t, per));
    }
    u += "\n## Figures (id: caption)\n";
    for f in &m.figures {
        u += &format!("- {}: {}\n", f.id, clip(&f.caption, 300));
    }
    u += "\n## Tables (id: caption)\n";
    for t in &m.tables {
        u += &format!("- {}: {}\n", t.id, clip(&t.caption, 300));
        if let Some(g) = &t.grid {
            for row in g.iter().take(8) {
                u += &format!("    | {} |\n", row.join(" | "));
            }
        }
    }
    u += "\n## Equations (id: LaTeX)\n";
    for e in &m.equations {
        u += &format!("- {}: {}\n", e.id, clip(&e.latex, 300));
    }
    if !m.refs.is_empty() {
        u += "\n## Bibliography (key: entry)\n";
        for (k, v) in m.refs.iter().take(80) {
            u += &format!("- {k}: {}\n", clip(v, 160));
        }
    }
    u += &format!("\nPlan {lo}–{hi} slides.");
    (system, u)
}

/// Typed cross-references ("Fig. 3", "Table 2", "Eq. (1)") out of slide text: a slide shows
/// the item itself, and the author's rule is `\ref`, never a typed number (lore-notation's
/// `hardcoded_reference`).
pub fn strip_typed_refs(s: &str) -> String {
    let re = regex::Regex::new(r"(?i)\s*\(?\b(?:fig(?:ure)?s?\.?|tab(?:le)?s?\.?|eqs?\.?|equations?|sec(?:tion)?s?\.?)\s*~?\(?\d+[a-z]?\)?\)?").unwrap();
    let out = re.replace_all(s, "");
    let out = out.replace(" ,", ",").replace(" .", ".").replace("()", "");
    out.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches([',', ';', ' ']).to_string()
}

/// Keep only ids the material has; enforce cover first and thanks last.
pub fn clean_plan(mut p: Plan, m: &Material) -> Plan {
    let figs: BTreeSet<&str> = m.figures.iter().map(|f| f.id.as_str()).collect();
    let tabs: BTreeSet<&str> = m.tables.iter().map(|f| f.id.as_str()).collect();
    let eqs: BTreeSet<&str> = m.equations.iter().map(|e| e.id.as_str()).collect();
    p.slides.retain(|s| ROLES.contains(&s.role.as_str()));
    p.slides.truncate(40);
    for s in &mut p.slides {
        if !figs.contains(s.figure.as_str()) {
            s.figure = if tabs.contains(s.figure.as_str()) && s.table.is_empty() { std::mem::take(&mut s.figure) } else { String::new() };
            if !s.figure.is_empty() {
                s.table = std::mem::take(&mut s.figure);
            }
        }
        if !tabs.contains(s.table.as_str()) {
            s.table.clear();
        }
        s.equations.retain(|e| eqs.contains(e.as_str()));
        s.cites.retain(|c| m.refs.contains_key(c));
        s.title = strip_typed_refs(&s.title);
        s.takeaway = strip_typed_refs(&s.takeaway);
        for b in s.bullets.iter_mut().chain(s.steps.iter_mut()) {
            *b = strip_typed_refs(b);
        }
        s.bullets.retain(|b| !b.is_empty());
        s.kv.retain(|r| r.len() >= 2 && !r[0].trim().is_empty());
    }
    if p.slides.first().map(|s| s.role.as_str()) != Some("cover") {
        p.slides.insert(0, PlanSlide { role: "cover".into(), title: m.title.clone(), ..Default::default() });
    }
    if p.slides.last().map(|s| s.role.as_str()) != Some("thanks") {
        p.slides.push(PlanSlide { role: "thanks".into(), title: "Thanks".into(), ..Default::default() });
    }
    // method slides without a figure use the big picture's
    let main = p.slides.iter().find(|s| s.role == "overview" && !s.figure.is_empty()).map(|s| s.figure.clone());
    if let Some(f) = main {
        for s in p.slides.iter_mut().filter(|s| s.role == "method" && s.figure.is_empty()) {
            s.figure = f.clone();
        }
    }
    p
}

// ── Building the deck ────────────────────────────────────────────────────────

/// A block of an arrangement (fractions of the slide).
#[derive(Debug, Clone)]
pub struct Block {
    pub role: String,
    pub rect: [f64; 4],
}

#[derive(Debug, Clone)]
pub struct Arrangement {
    pub id: String,
    pub blocks: Vec<Block>,
}

/// The author's academic arrangements (`arrangements/*.json`).
pub fn academic(ws: &sc_core::Workspace) -> Vec<Arrangement> {
    ws.arrangements()
        .into_iter()
        .filter(|a| a.get("class").and_then(Value::as_str).unwrap_or("academic") == "academic")
        .filter_map(|a| {
            let blocks: Vec<Block> = a["blocks"]
                .as_array()?
                .iter()
                .filter_map(|b| {
                    let r = &b["rect"];
                    let rect = if let Some(v) = r.as_array() {
                        [v.first()?.as_f64()?, v.get(1)?.as_f64()?, v.get(2)?.as_f64()?, v.get(3)?.as_f64()?]
                    } else {
                        [r["x"].as_f64()?, r["y"].as_f64()?, r["w"].as_f64()?, r["h"].as_f64()?]
                    };
                    Some(Block { role: b["role"].as_str().unwrap_or("").to_string(), rect })
                })
                .collect();
            Some(Arrangement { id: a["id"].as_str()?.to_string(), blocks })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default)]
struct Need {
    image: bool,
    table: bool,
    text: bool,
}

/// The arrangement whose blocks fit what the slide shows best (none: the base's content area).
fn pick<'a>(arrs: &'a [Arrangement], n: Need) -> Option<&'a Arrangement> {
    let score = |a: &Arrangement| {
        let has = |r: &str| a.blocks.iter().any(|b| b.role == r);
        let mut s = 0i32;
        let visual = n.image || n.table;
        if n.image {
            s += if has("image") { 4 } else { -4 };
        }
        if n.table {
            s += if has("table") { 4 } else if has("image") && !n.image { 2 } else { -3 };
        }
        if n.text {
            s += if has("text") { 3 } else { -3 };
        } else if has("text") && !visual {
            s -= 1;
        }
        if !visual && has("image") {
            s -= 3;
        }
        if !n.text && has("text") {
            s -= 1;
        }
        s += if has("title") { 1 } else { 0 };
        s
    };
    arrs.iter().filter(|a| a.blocks.iter().any(|b| b.role != "title")).max_by_key(|a| (score(a), -(a.blocks.len() as i32)))
}

struct Ctx<'a> {
    st: &'a StyleSystem,
    w: i64,
    h: i64,
    pandoc: &'a lore_math::Pandoc,
    macros: BTreeMap<String, String>,
    warnings: Vec<String>,
    deck_dir: PathBuf,
    /// LaTeX → OMML already converted (the same equation on several slides).
    omml: BTreeMap<(String, bool), Option<String>>,
}

impl Ctx<'_> {
    fn tag(&self, want: &[&str]) -> String {
        want.iter().find(|t| self.st.tag(t).is_some()).map(|t| t.to_string()).or_else(|| self.st.tag("body").map(|_| "body".to_string())).or_else(|| self.st.pack.tags.keys().next().cloned()).unwrap_or_else(|| "body".into())
    }
    fn look(&self, tag: &str, em: Option<&str>) -> Look {
        self.st.look(tag, em).unwrap_or_default()
    }
    fn color(&self, names: &[&str]) -> String {
        names.iter().find_map(|n| self.st.color(n)).unwrap_or_else(|| "000000".into())
    }
    fn run(&self, text: &str, tag: &str, em: Option<&str>) -> Run {
        let mut r = Run::text(text, tag, self.look(tag, em));
        r.em = em.map(String::from);
        r
    }
    fn para(&self, runs: Vec<Run>, tag: &str) -> Para {
        let mut p = Para::new(runs);
        p.end = Some(self.look(tag, None));
        p
    }

    fn expand(&self, latex: &str) -> String {
        expand_macros(latex, &self.macros)
    }

    fn omml(&mut self, latex: &str, display: bool, size: f64) -> Option<String> {
        let key = (latex.to_string(), display);
        if let Some(v) = self.omml.get(&key) {
            return v.clone();
        }
        let v = if self.pandoc.available() {
            match sc_core::latex_to_omml(self.pandoc, latex, display, size) {
                Ok(x) => Some(x),
                Err(e) => {
                    self.warnings.push(format!("方程式無法轉換（{}）：{e}", clip(latex, 40)));
                    None
                }
            }
        } else {
            None
        };
        self.omml.insert(key, v.clone());
        v
    }

    /// Text with `$…$` as native math runs (plain text when pandoc is missing).
    fn rich(&mut self, text: &str, tag: &str, em: Option<&str>) -> Vec<Run> {
        let mut out = vec![];
        let parts: Vec<&str> = text.split('$').collect();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            let math = i % 2 == 1 && i + 1 < parts.len();
            if !math {
                out.push(self.run(part, tag, em));
                continue;
            }
            let latex = self.expand(part);
            let size = self.look(tag, None).size;
            match self.omml(&latex, false, size) {
                Some(omml) => {
                    let mut look = self.look(tag, em);
                    look.font = "Cambria Math".into();
                    look.ea_font = "Cambria Math".into();
                    out.push(Run { kind: "math".into(), text: latex.clone(), tag: None, em: None, look, field: None, omml: Some(omml), latex: Some(latex), src: None, k: 0 });
                }
                None => out.push(self.run(&format!("${part}$"), tag, em)),
            }
        }
        out
    }
}

/// Registry macros (`\vx` → `\mathbf{x}`) expanded so pandoc and KaTeX read the equation;
/// the registry still recognises the expanded form (lore-notation matches LaTeX and macros).
pub fn expand_macros(latex: &str, macros: &BTreeMap<String, String>) -> String {
    let mut s = latex.to_string();
    for _ in 0..3 {
        let mut changed = false;
        let mut out = String::with_capacity(s.len());
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                if j > i + 1
                    && let Some(rep) = macros.get(&s[i..j]).or_else(|| macros.get(&s[i + 1..j]))
                {
                    out.push_str(rep);
                    changed = true;
                    i = j;
                    continue;
                }
                out.push_str(&s[i..j.max(i + 1)]);
                i = j.max(i + 1);
                continue;
            }
            let c = s[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
        s = out;
        if !changed {
            break;
        }
    }
    s
}

fn text_obj(name: &str, frame: Frame, body: TextBody, slot: Option<&str>) -> Object {
    Object {
        id: 0,
        name: name.into(),
        frame,
        hidden: false,
        ph: None,
        slot: slot.map(String::from),
        kind: Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(body), equation: false }),
        src: None,
    }
}

fn body(paras: Vec<Para>, anchor: &str) -> TextBody {
    TextBody { insets: [0, 0, 0, 0], anchor: anchor.into(), wrap: true, autofit: "none".into(), paras, ..Default::default() }
}

fn pic(name: &str, frame: Frame, asset: &str, crop: Crop, alpha: f64, slot: Option<&str>) -> Object {
    Object {
        id: 0,
        name: name.into(),
        frame,
        hidden: false,
        ph: None,
        slot: slot.map(String::from),
        kind: Kind::Picture(PicObj { asset: asset.into(), crop, alpha, line: None, clip: "rect".into(), masks: vec![], effects: vec![] }),
        src: None,
    }
}

/// Fit `iw × ih` inside `f` (contain, centred across, at the top).
fn contain(f: &Frame, iw: u32, ih: u32) -> Frame {
    let (iw, ih) = (iw.max(1) as f64, ih.max(1) as f64);
    let s = (f.w as f64 / iw).min(f.h as f64 / ih);
    let (w, h) = ((iw * s) as i64, (ih * s) as i64);
    Frame::new(f.x + (f.w - w) / 2, f.y, w, h)
}

/// Module box of the main figure, in pixels of its image.
#[derive(Debug, Clone)]
pub struct ModuleBox {
    pub label: String,
    pub bbox: [f64; 4],
}

/// Where everything of one page goes.
struct Areas {
    title: Option<Frame>,
    kicker: Option<Frame>,
    content: Frame,
    takeaway: Option<Frame>,
    subtitle: Option<Frame>,
}

fn areas(ws: &sc_core::Workspace, st: &StyleSystem, source: &lore_ooxml::Deck, kind: &str, w: i64, h: i64) -> Areas {
    let base = ws.base(&st.pack.pack.id).ok().flatten();
    let page = base.as_ref().and_then(|b| b.pages.get(kind).or_else(|| b.pages.get("content")));
    let pa = ws.page_areas(Some(st), source);
    let v = pa.iter().find(|p| p["kind"] == kind).or_else(|| pa.iter().find(|p| p["kind"] == "content"));
    let fr = |k: &str| v.and_then(|v| serde_json::from_value::<Frame>(v[k].clone()).ok());
    Areas {
        title: fr("title"),
        kicker: fr("kicker"),
        content: fr("content").unwrap_or_else(|| Frame::new(w / 25, h / 5, w - 2 * w / 25, h * 3 / 5)),
        takeaway: fr("takeaway"),
        subtitle: page.and_then(|p| p.subtitle.clone()),
    }
}

fn kicker_label(role: &str, lang: &str) -> &'static str {
    let zh = lang != "en";
    match role {
        "background" => if zh { "背景" } else { "Background" },
        "challenge" | "overview" => if zh { "問題與架構" } else { "Big picture" },
        "method" | "loss" => if zh { "方法" } else { "Method" },
        "training" => if zh { "訓練" } else { "Training" },
        "results" | "ablation" | "visual" => if zh { "結果" } else { "Results" },
        "opinion" | "takeaway" => if zh { "討論" } else { "Discussion" },
        _ => "",
    }
}

/// What the build produced besides the slides.
pub struct Built {
    pub slides: Vec<Slide>,
    /// Index of the big-picture slide and the object ids of each numbered step.
    pub reveal: Option<(usize, Vec<Vec<u32>>)>,
    pub warnings: Vec<String>,
}

pub struct BuildInput<'a> {
    pub ws: &'a sc_core::Workspace,
    pub st: &'a StyleSystem,
    pub source: &'a lore_ooxml::Deck,
    pub deck_dir: &'a Path,
    pub pandoc: &'a lore_math::Pandoc,
    pub macros: BTreeMap<String, String>,
    pub lang: &'a str,
    pub boxes: Vec<ModuleBox>,
    pub next_uid: &'a mut dyn FnMut() -> String,
}

/// The slides of a plan.
pub fn build(plan: &Plan, m: &Material, inp: BuildInput) -> Result<Built, String> {
    let (w, h) = inp.source.slide_size;
    let mut cx = Ctx { st: inp.st, w, h, pandoc: inp.pandoc, macros: inp.macros, warnings: vec![], deck_dir: inp.deck_dir.to_path_buf(), omml: BTreeMap::new() };
    let arrs = academic(inp.ws);
    let mut assets: BTreeMap<String, String> = BTreeMap::new();
    let mut out = vec![];
    let mut reveal_steps = None;
    let main_fig = plan.slides.iter().find(|s| s.role == "overview").map(|s| s.figure.clone()).unwrap_or_default();

    for ps in &plan.slides {
        let kind = match ps.role.as_str() {
            "cover" => "cover",
            "thanks" if inp.st.pack.layouts.contains_key("end") => "end",
            _ => "content",
        };
        let layout = inp.st.pack.layouts.get(kind).or_else(|| inp.st.pack.layouts.get("content")).cloned();
        let mut slide = Slide::new((inp.next_uid)(), layout.clone());
        let mut objs = match &layout {
            Some(l) => inp.ws.new_slide_objects(Some(inp.st), inp.source, l, inp.deck_dir).map_err(|e| e.to_string())?,
            None => vec![],
        };
        slide.notes = ps.notes.clone();
        let ar = areas(inp.ws, inp.st, inp.source, kind, w, h);
        let mut extra: Vec<Object> = vec![];

        let fig = m.figures.iter().find(|f| f.id == ps.figure && f.image.is_some()).or_else(|| m.tables.iter().find(|t| t.id == ps.figure && t.image.is_some()));
        let tab = m.tables.iter().find(|t| t.id == ps.table);
        let tab_native = tab.and_then(|t| t.grid.clone()).filter(|g| !g.is_empty());
        let tab_pic = if tab_native.is_none() { tab.filter(|t| t.image.is_some()) } else { None };
        let image = fig.or(tab_pic);
        let kv = (!ps.kv.is_empty()).then(|| ps.kv.iter().map(|r| vec![r[0].clone(), r[1..].join(" ")]).collect::<Vec<_>>());
        let table_grid = kv.clone().or(tab_native.clone());

        match kind {
            "cover" => {
                let title_tag = cx.tag(&["cover-title", "title"]);
                let title = if m.title.is_empty() { ps.title.clone() } else { m.title.clone() };
                fill_title(&mut objs, &cx, &title, &title_tag, ar.title.clone().unwrap_or_else(|| Frame::new(w / 16, h * 3 / 10, w * 7 / 8, h / 6)));
                let venue = [plan.venue.trim(), m.venue.trim()].into_iter().find(|v| !v.is_empty()).unwrap_or("").to_string();
                let venue = if !m.year.is_empty() && !venue.contains(&m.year) && !venue.is_empty() { format!("{venue}, {}", m.year) } else { venue };
                let keywords: Vec<String> = if plan.keywords.is_empty() { m.keywords.clone() } else { plan.keywords.clone() };
                let mut paras = vec![];
                if !venue.is_empty() {
                    let t = cx.tag(&["venue", "label", "subtitle"]);
                    paras.push(cx.para(vec![cx.run(&venue, &t, None)], &t));
                }
                if !keywords.is_empty() {
                    let t = cx.tag(&["subtitle", "body"]);
                    paras.push(cx.para(vec![cx.run(&format!("Keywords: {}", keywords.join(", ")), &t, None)], &t));
                }
                if !m.authors.is_empty() {
                    let t = cx.tag(&["caption", "subtitle"]);
                    paras.push(cx.para(vec![cx.run(&m.authors.join(", "), &t, None)], &t));
                }
                let f = ar.subtitle.clone().unwrap_or_else(|| {
                    let t = ar.title.clone().unwrap_or_else(|| Frame::new(w / 16, h * 3 / 10, w * 7 / 8, h / 6));
                    Frame::new(t.x, t.y + t.h + h / 40, t.w, h / 4)
                });
                if !paras.is_empty() {
                    extra.push(text_obj("期刊與關鍵字", Frame::new(f.x, f.y, f.w, f.h.max(h / 5)), body(paras, "t"), None));
                }
                if !m.self_ref.is_empty() {
                    extra.push(ref_obj(&cx, &m.self_ref, &ar));
                }
            }
            "end" => {
                let t = cx.tag(&["cover-title", "title"]);
                fill_title(&mut objs, &cx, &ps.title, &t, ar.title.clone().unwrap_or_else(|| Frame::new(w / 16, h * 2 / 5, w * 7 / 8, h / 6)));
            }
            _ => {
                let title_tag = cx.tag(&["title"]);
                let need = Need { image: image.is_some(), table: table_grid.is_some(), text: !ps.bullets.is_empty() || !ps.equations.is_empty() || !ps.steps.is_empty() };
                let arr = pick(&arrs, need);
                slide.arrangement = arr.map(|a| a.id.clone());
                let frac = |b: &Block| Frame::new((b.rect[0] * w as f64) as i64, (b.rect[1] * h as f64) as i64, (b.rect[2] * w as f64) as i64, (b.rect[3] * h as f64) as i64);
                let block = |role: &str| arr.and_then(|a| a.blocks.iter().find(|b| b.role == role)).map(frac);
                // title: the arrangement's block, else the base's title area
                if let Some(tf) = block("title") {
                    objs.retain(|o| o.slot.as_deref() != Some("title"));
                    let t = text_obj("Title", tf, body(vec![cx.para(vec![cx.run(&ps.title, &title_tag, None)], &title_tag)], "ctr"), Some("title-1"));
                    extra.push(t);
                } else {
                    fill_title(&mut objs, &cx, &ps.title, &title_tag, ar.title.clone().unwrap_or_else(|| Frame::new(w / 25, h / 12, w * 23 / 25, h / 10)));
                }
                if let Some(kf) = &ar.kicker {
                    let label = kicker_label(&ps.role, inp.lang);
                    if !label.is_empty() {
                        let t = cx.tag(&["kicker", "label"]);
                        extra.push(text_obj("kicker", kf.clone(), body(vec![cx.para(vec![cx.run(label, &t, None)], &t)], "b"), None));
                    }
                }
                // where the visual and the text go
                let content = ar.content.clone();
                let (vis_f, text_f) = match (image.is_some() || table_grid.is_some(), need.text) {
                    (true, true) => {
                        let v = block("image").or_else(|| block("table")).unwrap_or_else(|| Frame::new(content.x, content.y, content.w * 55 / 100, content.h));
                        let t = block("text").unwrap_or_else(|| Frame::new(content.x + content.w * 58 / 100, content.y, content.w * 42 / 100, content.h));
                        (Some(v), Some(t))
                    }
                    (true, false) => {
                        let lone_image = block("image").filter(|_| block("text").is_none());
                        (Some(block("table").filter(|_| table_grid.is_some()).or(lone_image).unwrap_or(content.clone())), None)
                    }
                    (false, _) => (None, Some(block("text").unwrap_or(content.clone()))),
                };
                // the takeaway: the base's band, the arrangement's block, else a line under the content
                let mut take_f = None;
                if !ps.takeaway.trim().is_empty() {
                    take_f = Some(ar.takeaway.clone().or_else(|| block("takeaway")).unwrap_or_else(|| Frame::new(content.x, content.y + content.h - h / 12, content.w, h / 12)));
                }
                let shrink = |f: Frame| -> Frame {
                    match &take_f {
                        Some(t) if f.y + f.h > t.y && t.y > f.y => Frame::new(f.x, f.y, f.w, (t.y - f.y - h / 60).max(h / 8)),
                        _ => f,
                    }
                };
                let vis_f = vis_f.map(shrink);
                let text_f = text_f.map(shrink);

                let mut step_ids: Vec<Vec<u32>> = vec![];
                if let (Some(vf), Some(grid)) = (&vis_f, &table_grid) {
                    extra.push(table_obj(&mut cx, vf, grid, kv.is_some()));
                } else if let (Some(vf), Some(img)) = (&vis_f, image) {
                    let im = img.image.as_ref().unwrap();
                    let asset = match assets.get(&img.id) {
                        Some(a) => a.clone(),
                        None => {
                            let a = sc_core::store_asset(&cx.deck_dir, &im.bytes, &im.ext).map_err(|e| e.to_string())?;
                            assets.insert(img.id.clone(), a.clone());
                            a
                        }
                    };
                    let cap_h = if img.caption.is_empty() { 0 } else { h / 20 };
                    let area = Frame::new(vf.x, vf.y, vf.w, vf.h - cap_h);
                    let pf = contain(&area, im.w, im.h);
                    let is_main = img.id == main_fig && !inp.boxes.is_empty();
                    let module = ps.module as usize;
                    if is_main && ps.role == "method" && module >= 1 && module <= inp.boxes.len() {
                        // the whole figure dimmed, the current module cropped back on top, framed
                        let ghost = inp.st.pack.ghost.picture_alpha.clamp(0.1, 1.0);
                        extra.push(pic("架構圖", pf.clone(), &asset, Crop::default(), ghost, Some("image-1")));
                        let (sub, crop) = module_rect(&pf, im, &inp.boxes[module - 1].bbox);
                        extra.push(pic(&format!("模組 {module}"), sub.clone(), &asset, crop, 1.0, None));
                        extra.push(frame_obj(&cx, &sub, &format!("框 {module}")));
                    } else {
                        extra.push(pic(if fig.is_some() { "圖" } else { "表" }, pf.clone(), &asset, Crop::default(), 1.0, Some("image-1")));
                        if is_main && ps.role == "overview" {
                            for (i, b) in inp.boxes.iter().enumerate() {
                                let (sub, _) = module_rect(&pf, im, &b.bbox);
                                let mut o = frame_obj(&cx, &sub, &format!("框 {}", i + 1));
                                o.name = format!("step-frame-{}", i + 1);
                                extra.push(o);
                            }
                        }
                    }
                    if cap_h > 0 {
                        let t = cx.tag(&["caption"]);
                        let cap = strip_typed_refs(&clip(&img.caption, 140));
                        extra.push(text_obj("圖說", Frame::new(vf.x, pf.y + pf.h + h / 120, vf.w, cap_h), body(vec![cx.para(vec![cx.run(&cap, &t, None)], &t)], "t"), Some("caption-1")));
                    }
                }
                if let Some(tf) = &text_f {
                    let dense = ps.bullets.iter().map(|b| b.chars().count()).sum::<usize>() > 260;
                    let t = if dense { cx.tag(&["body-dense", "body"]) } else { cx.tag(&["body"]) };
                    if !ps.steps.is_empty() {
                        // one text box per numbered step (the reveal steps)
                        let n = ps.steps.len() as i64;
                        let gap = h / 60;
                        let sh = ((tf.h - gap * (n - 1)) / n).min(h / 7);
                        for (i, s) in ps.steps.iter().enumerate() {
                            let label = cx.tag(&["label"]);
                            let mut runs = vec![cx.run(&format!("{}  ", i + 1), &label, None)];
                            runs.extend(cx.rich(s, &t, None));
                            let mut o = text_obj(&format!("步驟 {}", i + 1), Frame::new(tf.x, tf.y + i as i64 * (sh + gap), tf.w, sh), body(vec![cx.para(runs, &t)], "t"), None);
                            o.name = format!("step-{}", i + 1);
                            extra.push(o);
                        }
                        step_ids = (0..ps.steps.len()).map(|_| vec![]).collect();
                    }
                    let mut paras = vec![];
                    let num = ps.role == "takeaway";
                    for b in &ps.bullets {
                        let runs = cx.rich(b, &t, None);
                        let mut p = cx.para(runs, &t);
                        p.bullet = if num { Bullet::Num { scheme: "arabicPeriod".into(), start: 1 } } else { Bullet::Char { ch: "•".into() } };
                        p.mar_l = EMU_IN / 4;
                        p.indent = -EMU_IN / 4;
                        p.after_pt = Some(6.0);
                        paras.push(p);
                    }
                    // equations of the slide: native OMML, one per line
                    let eq_tag = cx.tag(&["equation", "body"]);
                    for id in &ps.equations {
                        let Some(e) = m.equations.iter().find(|e| &e.id == id) else { continue };
                        let latex = cx.expand(&e.latex);
                        let used = latex.clone();
                        let size = cx.look(&eq_tag, None).size;
                        let mut p = match cx.omml(&latex, true, size) {
                            Some(omml) => {
                                let look = cx.look(&eq_tag, None);
                                let tag = cx.st.tag("equation").map(|_| "equation".to_string());
                                cx.para(vec![Run { kind: "math".into(), text: latex.clone(), tag, em: None, look, field: None, omml: Some(omml), latex: Some(latex), src: None, k: 0 }], &eq_tag)
                            }
                            None => cx.para(vec![cx.run(&format!("$${latex}$$"), &t, None)], &t),
                        };
                        p.align = "ctr".into();
                        p.before_pt = Some(6.0);
                        p.after_pt = Some(6.0);
                        paras.push(p);
                        // the symbols it uses, from the paper's registry, as notes under it (§7.1: 符號說明)
                        let note_tag = cx.tag(&["caption", "body"]);
                        let em = cx.st.pack.emphasis.contains_key("note").then_some("note");
                        let mut seen = BTreeSet::new();
                        for (sym, meaning) in &m.symbols {
                            if !used.contains(sym.as_str()) || !seen.insert(sym.clone()) {
                                continue;
                            }
                            let line = if inp.lang == "en" { format!("where ${sym}$ denotes {meaning}") } else { format!("${sym}$ 為{meaning}") };
                            let runs = cx.rich(&line, &note_tag, em);
                            let mut p = cx.para(runs, &note_tag);
                            p.align = "ctr".into();
                            paras.push(p);
                        }
                    }
                    if !paras.is_empty() {
                        let f = if ps.steps.is_empty() {
                            tf.clone()
                        } else {
                            let used = extra.iter().filter(|o| o.name.starts_with("step-") && !o.name.starts_with("step-frame")).map(|o| o.frame.y + o.frame.h).max().unwrap_or(tf.y);
                            Frame::new(tf.x, used + h / 60, tf.w, (tf.y + tf.h - used).max(h / 8))
                        };
                        extra.push(text_obj("內文", f, body(paras, "t"), Some("text-1")));
                    }
                }
                if let Some(tf) = take_f {
                    extra.push(takeaway_obj(&cx, &tf, &ps.takeaway, ar.takeaway.is_some() || block("takeaway").is_some()));
                }
                if !ps.cites.is_empty() {
                    let refs: Vec<String> = ps.cites.iter().filter_map(|c| m.refs.get(c)).cloned().collect();
                    if !refs.is_empty() {
                        extra.push(ref_obj(&cx, &format!("Ref: {}", refs.join("; ")), &ar));
                    }
                }
                if !step_ids.is_empty() {
                    reveal_steps = Some((out.len(), step_ids));
                }
            }
        }
        // ids after the base's objects
        let mut next = objs.iter().map(|o| o.id).max().unwrap_or(1) + 1;
        for mut o in extra {
            o.id = next;
            next += 1;
            objs.push(o);
        }
        // the reveal steps: step text i and its frame on the figure
        if let Some((idx, steps)) = &mut reveal_steps
            && *idx == out.len()
        {
            for (i, s) in steps.iter_mut().enumerate() {
                for o in &objs {
                    if o.name == format!("step-{}", i + 1) || o.name == format!("step-frame-{}", i + 1) {
                        s.push(o.id);
                    }
                }
            }
        }
        slide.objects = objs;
        out.push(slide);
    }
    Ok(Built { slides: out, reveal: reveal_steps.filter(|(_, s)| s.len() >= 2), warnings: cx.warnings })
}

/// The module's rectangle on the slide and the crop that shows only it.
fn module_rect(pf: &Frame, im: &Img, b: &[f64; 4]) -> (Frame, Crop) {
    let (iw, ih) = (im.w.max(1) as f64, im.h.max(1) as f64);
    let (x0, y0, x1, y1) = (b[0].clamp(0.0, iw), b[1].clamp(0.0, ih), b[2].clamp(0.0, iw), b[3].clamp(0.0, ih));
    let sub = Frame::new(pf.x + (pf.w as f64 * x0 / iw) as i64, pf.y + (pf.h as f64 * y0 / ih) as i64, (pf.w as f64 * (x1 - x0) / iw) as i64, (pf.h as f64 * (y1 - y0) / ih) as i64);
    (sub, Crop { l: x0 / iw, t: y0 / ih, r: 1.0 - x1 / iw, b: 1.0 - y1 / ih })
}

/// The emphasis frame around the current part (design §7.1: a red 2.2 pt frame).
fn frame_obj(cx: &Ctx, f: &Frame, name: &str) -> Object {
    let c = cx.st.pack.emphasis.get("key").and_then(|e| e.color.clone()).map(|c| c.to_uppercase()).unwrap_or_else(|| cx.color(&["emphasis", "accent-1", "red", "ink"]));
    Object {
        id: 0,
        name: name.into(),
        frame: f.clone(),
        hidden: false,
        ph: None,
        slot: None,
        kind: Kind::Shape(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: Some(Stroke::new(&c, 2.25)), text: None, equation: false }),
        src: None,
    }
}

fn fill_title(objs: &mut Vec<Object>, cx: &Ctx, text: &str, tag: &str, fallback: Frame) {
    let para = cx.para(vec![cx.run(text, tag, None)], tag);
    let slot = objs.iter_mut().find(|o| o.slot.as_deref() == Some("title") || matches!(o.ph.as_deref(), Some("title") | Some("ctrTitle")));
    match slot.and_then(|o| o.kind.sp_mut()) {
        Some(sp) => {
            let tb = sp.text.get_or_insert_with(TextBody::default);
            tb.paras = vec![para];
        }
        None => objs.push(text_obj("Title", fallback, body(vec![para], "ctr"), Some("title"))),
    }
}

/// `Ref:` small, bottom left (tag `source`, else `reference`, else `caption`).
fn ref_obj(cx: &Ctx, text: &str, ar: &Areas) -> Object {
    let t = cx.tag(&["source", "reference", "caption"]);
    let x = ar.content.x;
    text_obj("Ref", Frame::new(x, cx.h * 905 / 1000, cx.w * 7 / 10, cx.h * 4 / 100), body(vec![cx.para(vec![cx.run(&clip(text, 260), &t, None)], &t)], "b"), None)
}

fn takeaway_obj(cx: &Ctx, f: &Frame, text: &str, band: bool) -> Object {
    if band {
        let t = cx.tag(&["takeaway", "label"]);
        let fill = cx.color(&["inverse", "ink"]);
        let mut tb = body(vec![cx.para(vec![cx.run(text, &t, None)], &t)], "ctr");
        tb.insets = [228_600, 45_720, 228_600, 45_720];
        let mut g = Geom::preset("roundRect");
        g.adj.insert("adj".into(), 12000.0);
        return Object {
            id: 0,
            name: "takeaway".into(),
            frame: f.clone(),
            hidden: false,
            ph: None,
            slot: Some("takeaway-1".into()),
            kind: Kind::Shape(SpObj { geom: g, fill: Paint::solid(&fill), line: None, text: Some(tb), equation: false }),
            src: None,
        };
    }
    // no band in the base: one line in the label tag, key result emphasis
    let t = cx.tag(&["label", "body"]);
    let em = cx.st.pack.emphasis.contains_key("key").then_some("key");
    text_obj("takeaway", f.clone(), body(vec![cx.para(vec![cx.run(text, &t, em)], &t)], "ctr"), Some("takeaway-1"))
}

/// A native table (無樣式、表格格線): first row bold as header; key-value tables have no header.
fn table_obj(cx: &mut Ctx, f: &Frame, grid: &[Vec<String>], kv: bool) -> Object {
    let t = cx.tag(&["table", "body-dense", "body"]);
    let ncol = grid.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let nrow = grid.len().max(1) as i64;
    let rh = (f.h / nrow).min(EMU_IN * 2 / 5).max(EMU_IN / 5);
    // narrow tables stay narrow (about 2.6 in a column)
    let tw = f.w.min(ncol as i64 * EMU_IN * 13 / 5);
    let cols: Vec<i64> = if kv && ncol == 2 { vec![tw * 2 / 5, tw - tw * 2 / 5] } else { vec![tw / ncol as i64; ncol] };
    let mut rows: Vec<Row> = vec![];
    for (ri, r) in grid.iter().enumerate() {
        let mut cells = vec![];
        for ci in 0..ncol {
            let text = r.get(ci).map(|s| strip_typed_refs(&lore_docsrc::latex::plain::tidy(s))).unwrap_or_default();
            let head = (!kv && ri == 0) || (kv && ci == 0);
            let em = head.then_some("strong").filter(|e| cx.st.pack.emphasis.contains_key(*e));
            // `$…$` in a cell is native math, like in the bullets
            let mut runs = cx.rich(&text, &t, em);
            if runs.is_empty() {
                runs.push(cx.run("", &t, em));
            }
            if head && em.is_none() {
                runs.iter_mut().for_each(|r| r.look.bold = true);
            }
            let mut tb = body(vec![cx.para(runs, &t)], "ctr");
            tb.insets = [91_440, 45_720, 91_440, 45_720];
            cells.push(Cell { text: tb, span: 1, rspan: 1, hmerge: false, vmerge: false, fill: None, borders: [None, None, None, None], src: None });
        }
        rows.push(Row { h: rh, cells, src: None });
    }
    let height = rh * nrow;
    Object {
        id: 0,
        name: if kv { "訓練細節".into() } else { "表".into() },
        frame: Frame::new(f.x, f.y, cols.iter().sum(), height),
        hidden: false,
        ph: None,
        slot: Some("table-1".into()),
        kind: Kind::Table(TableObj { cols, rows, style_id: Some(TABLE_GRID.into()), first_row: !kv, band_row: false }),
        src: None,
    }
}

// ── The request ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Source {
    pub kind: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MakeBody {
    pub source: Source,
    #[serde(default)]
    pub length: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
}

fn llm(s: &Shared) -> &Llm {
    s.llm.get_or_init(|| Llm::new(s.home.clone()))
}

/// Ask a vision engine for the modules of the main figure; nothing when there is none.
async fn module_boxes(s: &Shared, m: &Material, plan: &Plan) -> Vec<ModuleBox> {
    let Some(ov) = plan.slides.iter().find(|s| s.role == "overview") else { return vec![] };
    let n = ov.steps.len();
    if n < 2 {
        return vec![];
    }
    let Some(f) = m.figures.iter().find(|f| f.id == ov.figure) else { return vec![] };
    let Some(im) = f.image.as_ref().filter(|i| i.ext == "png") else { return vec![] };
    let Ok(route) = llm(s).route(lore_docsrc::boxer::TASK_ID, None).await else { return vec![] };
    let Ok(img) = lore_docsrc::boxer::FigureImage::from_png_bytes(im.bytes.clone()) else { return vec![] };
    let caption = format!("{}\nThe steps, in order: {}", f.caption, ov.steps.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect::<Vec<_>>().join(" "));
    let boxer = lore_docsrc::boxer::LlmModuleBoxer::from_route(route);
    use lore_docsrc::boxer::ModuleBoxer;
    match boxer.module_boxes(&img, &caption).await {
        // one box per step, in reading order; any other count is not trusted
        Ok(b) if b.len() == n => b.into_iter().map(|b| ModuleBox { label: b.label, bbox: b.bbox }).collect(),
        _ => vec![],
    }
}

/// A failed generation: the kit's error body (with 改用本機／改用 Claude when the route offers it).
pub struct MakeErr(StatusCode, Value);

impl axum::response::IntoResponse for MakeErr {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(self.1)).into_response()
    }
}
impl From<ApiError> for MakeErr {
    fn from(e: ApiError) -> Self {
        MakeErr(e.0, json!({ "error": e.1 }))
    }
}
impl From<sc_core::Error> for MakeErr {
    fn from(e: sc_core::Error) -> Self {
        ApiError::from(e).into()
    }
}

async fn make(State(s): State<Shared>, Json(b): Json<MakeBody>) -> Result<Json<Value>, MakeErr> {
    let length = b.length.clone().unwrap_or_else(|| "standard".into());
    let lang = b.lang.clone().unwrap_or_else(|| "zh".into());
    let style = match &b.style {
        Some(st) if !st.is_empty() => st.clone(),
        _ => s.ws.style_ids().into_iter().next().ok_or_else(|| bad("沒有底版"))?,
    };
    let st = s.ws.style(&style)?;
    // 1. material (blocking: files, poppler)
    let cache = s.ws.root.join("cache/todeck");
    let (home, src_kind, path, sha) = (s.home.clone(), b.source.kind.clone(), b.source.path.clone(), b.source.sha.clone());
    let m = tokio::task::spawn_blocking(move || -> Result<Material, String> {
        match src_kind.as_str() {
            "pc" => {
                let p = PathBuf::from(path.ok_or("缺少論文資料夾")?);
                if !is_paper(&p) {
                    return Err("這個資料夾不是 LaTeX 論文".into());
                }
                from_latex(&p, &cache)
            }
            "lr" => {
                let sha = sha.filter(|x| valid_sha(x)).ok_or("缺少論文")?;
                let r = Reader::discover(&home).ok_or("找不到 LoreReader 的資料夾")?;
                from_lorereader(&r, &sha, &cache)
            }
            _ => Err("不認得的來源".into()),
        }
    })
    .await
    .map_err(|e| bad(e.to_string()))?
    .map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    // 2. the plan
    let (system, user) = prompt(&m, &length, &lang);
    let req = LlmRequest::prompt(system, user).json(schema()).max_tokens(12000);
    let ans = llm(&s).complete_json(TASK_PAPER_DECK, req, b.engine.as_deref()).await.map_err(|(_, body)| MakeErr(StatusCode::SERVICE_UNAVAILABLE, body))?;
    let plan: Plan = serde_json::from_value(ans.value.clone()).map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("模型的大綱無法使用：{e}")))?;
    let plan = clean_plan(plan, &m);
    if plan.slides.len() < 3 {
        return Err(ApiError(StatusCode::BAD_GATEWAY, "模型的大綱太短".into()).into());
    }
    // 3. module boxes of the main figure (vision; optional)
    let boxes = module_boxes(&s, &m, &plan).await;

    // 4. the deck
    let name = s.ws.new_deck(&m.slug, &style)?;
    let out = (|| -> Result<Value, ApiError> {
        let mut sess = s.ws.open(&name)?;
        let mut global = s.home.shared_dir().map(|d| d.join("notation.json")).and_then(|p| Notation::load(&p).ok()).unwrap_or_default();
        let eff = Notation::effective(&global, &m.notation);
        let macros = lore_notation::katex_macros(&eff);
        global.entries.clear();
        let deck_dir = sess.dir.clone();
        let mut deck = sess.editor.deck.clone();
        let mut uid = || deck.new_uid();
        let built = build(&plan, &m, BuildInput { ws: &s.ws, st: &st, source: &sess.source, deck_dir: &deck_dir, pandoc: &s.pandoc, macros, lang: &lang, boxes: boxes.clone(), next_uid: &mut uid })?;
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
        sess.editor.apply(ops, "論文報告")?;
        sess.save()?;
        // the paper's notation, with its ids, into the deck's own file
        let (_, mut doc) = crate::notation::load(&sess.dir, None);
        let mut added = 0;
        for e in &m.notation.entries {
            if doc.get(&e.id).is_none() {
                doc.entries.push(e.clone());
                added += 1;
            }
        }
        if let Some(table) = &m.lr_symbols {
            let ids: Vec<String> = table["entries"].as_array().into_iter().flatten().chain(table["equations"].as_array().into_iter().flatten()).filter_map(|e| e["id"].as_str().map(String::from)).collect();
            added += crate::reader::register_into(&mut doc, table, &ids)["added"].as_array().map_or(0, Vec::len);
        }
        if added > 0 {
            doc.save(&sess.dir.join("notation.json")).map_err(bad)?;
        }
        Ok(json!({
            "name": name,
            "slides": sess.editor.deck.slides.len(),
            "engine": crate::llm::engine_label(&ans.engine),
            "boxes": boxes.len(),
            "notation": added,
            "warnings": built.warnings,
        }))
    })();
    if out.is_err() {
        // a half-made deck is not left behind
        let _ = std::fs::remove_dir_all(s.ws.decks.join(&name));
    }
    Ok(Json(out?))
}

impl From<String> for ApiError {
    fn from(e: String) -> Self {
        ApiError(StatusCode::UNPROCESSABLE_ENTITY, e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_references_are_dropped() {
        assert_eq!(strip_typed_refs("As shown in Fig. 3, the model wins"), "As shown in, the model wins");
        assert_eq!(strip_typed_refs("Results (Table 2) are better"), "Results are better");
        assert_eq!(strip_typed_refs("Minimise Eq. (1) jointly"), "Minimise jointly");
        assert_eq!(strip_typed_refs("A figure of merit"), "A figure of merit");
    }

    #[test]
    fn macros_expand_until_stable() {
        let m: BTreeMap<String, String> = [("\\vx".to_string(), "\\mathbf{x}".to_string()), ("\\loss".to_string(), "\\mathcal{L}(\\vx)".to_string())].into();
        assert_eq!(expand_macros("\\loss = \\vxy + \\vx", &m), "\\mathcal{L}(\\mathbf{x}) = \\vxy + \\mathbf{x}");
    }

    #[test]
    fn jpeg_size_reads_sof() {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0, 0, 0xFF, 0xC0, 0x00, 0x11, 8, 0x00, 0x20, 0x00, 0x40, 3];
        b.extend([0u8; 16]);
        assert_eq!(jpeg_size(&b), Some((64, 32)));
    }

    #[test]
    fn short_refs() {
        let f: BTreeMap<String, String> = [("author", "Ellrin, A. and Daidai, B. and Third, C."), ("title", "A {Synthetic} Study"), ("journal", "Journal of Tests"), ("year", "2025")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        assert_eq!(short_ref(&f, ""), "Ellrin et al., A Synthetic Study, Journal of Tests, 2025");
    }
}
