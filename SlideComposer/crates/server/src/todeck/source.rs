//! `source`: the paper's material (sections, figures, tables, equations with labels,
//! references) from a PaperComposer LaTeX folder or a LoreReader PDF, read-only, and where
//! PaperComposer's papers are.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lore_docsrc::{AssetCatalog, AssetKind};
use lore_notation::Notation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::reader::{Reader, png_size};

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
pub(crate) fn is_paper(dir: &Path) -> bool {
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

// ── Material ─────────────────────────────────────────────────────────────────

/// A picture of the paper. In a checkpoint the bytes are a file of the job folder
/// (`img/<file>`), written by [`Material::save_images`] and read back by [`Material::load_images`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Img {
    #[serde(skip)]
    pub bytes: Vec<u8>,
    #[serde(default)]
    pub file: String,
    pub ext: String,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Fig {
    pub id: String,
    pub caption: String,
    pub image: Option<Img>,
    /// Table cells (LaTeX tables).
    pub grid: Option<Vec<Vec<String>>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Eq {
    pub id: String,
    pub latex: String,
    pub label: Option<String>,
    /// Where it is in the PDF (LoreReader): page and `[x0, y0, x1, y1]` page fractions.
    pub page: Option<u32>,
    pub bbox: Option<[f64; 4]>,
    /// How the LaTeX was read: `text` (the PDF's text), `vision` (a model read the crop),
    /// `picture` (no reliable LaTeX: the slide shows `image`, the crop).
    pub read: String,
    pub image: Option<Img>,
}

impl Eq {
    /// LaTeX a slide may show as an equation (not a picture-only one).
    pub fn usable(&self) -> bool {
        self.read != "picture" && !self.latex.trim().is_empty()
    }
}

/// A symbol of the paper for the notes under an equation: its LaTeX and meaning, in the
/// paper's registry language (`meaning`, the author's) and in English (`meaning_en`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SymbolNote {
    pub latex: String,
    pub meaning: Option<String>,
    pub meaning_en: Option<String>,
}

impl<'de> Deserialize<'de> for SymbolNote {
    /// Also the older `[latex, meaning]` pairs of a saved job.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum In {
            Pair(String, String),
            Full {
                latex: String,
                #[serde(default)]
                meaning: Option<String>,
                #[serde(default)]
                meaning_en: Option<String>,
            },
        }
        Ok(match In::deserialize(d)? {
            In::Pair(latex, m) => {
                let zh = m.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
                SymbolNote { latex, meaning: zh.then(|| m.clone()), meaning_en: (!zh).then_some(m) }
            }
            In::Full { latex, meaning, meaning_en } => SymbolNote { latex, meaning, meaning_en },
        })
    }
}

/// Everything the plan and the build use, from either source.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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
    /// Symbols of the paper's registry, for the notes under an equation.
    pub symbols: Vec<SymbolNote>,
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

pub(crate) fn img_from(bytes: Vec<u8>, ext: &str) -> Option<Img> {
    let (w, h) = match ext {
        "png" => png_size(&bytes)?,
        "jpg" | "jpeg" => jpeg_size(&bytes)?,
        _ => return None,
    };
    Some(Img { bytes, file: String::new(), ext: if ext == "jpeg" { "jpg".into() } else { ext.into() }, w, h })
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
pub(crate) fn short_ref(fields: &BTreeMap<String, String>, text: &str) -> String {
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
            // 待補 cells (`\pendingcell{P1}{…}`) stay marks: `[pending]`, never a number
            AssetKind::Table => m.tables.push(Fig {
                id: a.id.clone(),
                caption: a.caption.clone().unwrap_or_default(),
                image: None,
                grid: a.grid.clone().filter(|g| !g.is_empty()).map(|g| g.into_iter().map(|r| r.into_iter().map(|c| lore_notation::pending::to_plain(&c)).collect()).collect()),
            }),
            AssetKind::Equation => {
                if let Some(t) = a.text.as_deref().filter(|t| !t.trim().is_empty()) {
                    m.equations.push(Eq { id: a.id.clone(), latex: t.trim().to_string(), label: a.label.clone(), read: "text".into(), ..Default::default() });
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
        .filter_map(|e| Some(SymbolNote { latex: e.latex.clone()?, meaning: e.meaning.clone(), meaning_en: e.meaning_en.clone() }))
        .filter(|n| n.meaning.is_some() || n.meaning_en.is_some())
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
    // equations: LoreReader's own LaTeX (symbols.json), ids and numbers kept
    if let Some(v) = &m.lr_symbols {
        m.equations = lr_equations(v);
        m.symbols = lr_symbol_notes(v);
    }
    for r in &cat.references {
        m.refs.insert(r.key.clone(), r.text.trim().to_string());
    }
    m.slug = slug(&m.title);
    Ok(m)
}

/// LoreReader's display equations (symbols.json `equations`): the label is the paper's
/// number (`(1)`); a paper bound twice (a manuscript and its marked-up copy) gives every
/// equation twice — the second copy (same LaTeX, or the same number) is left out; an
/// equation without LaTeX is left out.
pub fn lr_equations(v: &Value) -> Vec<Eq> {
    let mut out: Vec<Eq> = vec![];
    let squeeze = |s: &str| s.split_whitespace().collect::<String>();
    for e in v["equations"].as_array().into_iter().flatten() {
        let (Some(id), Some(latex)) = (e["id"].as_str(), e["latex"].as_str()) else { continue };
        let latex = latex.trim();
        if latex.is_empty() {
            continue;
        }
        let label = e["label"].as_str().map(str::to_string).or_else(|| e["number"].as_str().map(|n| format!("({n})"))).filter(|l| !l.trim().is_empty());
        let dup = out.iter().any(|o| o.id == id || squeeze(&o.latex) == squeeze(latex) || (label.is_some() && o.label == label));
        if dup {
            continue;
        }
        let bbox = e["bbox"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<_>>()).filter(|b| b.len() == 4 && b[2] > b[0] && b[3] > b[1]).map(|b| [b[0], b[1], b[2], b[3]]);
        let page = e["page"].as_u64().map(|p| p as u32);
        out.push(Eq { id: id.to_string(), latex: latex.to_string(), label, page, bbox, read: "text".into(), image: None });
    }
    out
}

/// LoreReader's symbol rows with a meaning (the author's / Claude's `meaning`, or the
/// definition the program found, `meaning_en`).
pub fn lr_symbol_notes(v: &Value) -> Vec<SymbolNote> {
    let s = |e: &Value, k: &str| e[k].as_str().map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    v["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some(SymbolNote { latex: s(e, "latex")?, meaning: s(e, "meaning"), meaning_en: s(e, "meaning_en") }))
        .filter(|n| n.meaning.is_some() || n.meaning_en.is_some())
        .collect()
}

impl Material {
    fn pictures_mut(&mut self) -> impl Iterator<Item = (String, &mut Img)> {
        let figs = self.figures.iter_mut().chain(self.tables.iter_mut()).filter_map(|f| Some((f.id.clone(), f.image.as_mut()?)));
        let eqs = self.equations.iter_mut().filter_map(|e| Some((e.id.clone(), e.image.as_mut()?)));
        figs.chain(eqs)
    }

    /// Every picture's bytes into `dir` (`<id>.<ext>`), its file name kept in the picture.
    pub fn save_images(&mut self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        for (id, im) in self.pictures_mut() {
            let name = format!("{}.{}", safe_name(&id), im.ext);
            crate::jobs::write_atomic(&dir.join(&name), &im.bytes)?;
            im.file = name;
        }
        Ok(())
    }

    /// The pictures' bytes back from `dir`; a missing file is an error (the stage is redone).
    pub fn load_images(&mut self, dir: &Path) -> Result<(), String> {
        for (_, im) in self.pictures_mut() {
            im.bytes = std::fs::read(dir.join(&im.file)).map_err(|e| format!("{}: {e}", im.file))?;
        }
        Ok(())
    }
}

/// An id as a file name: letters, digits, `-`, `_`.
pub fn safe_name(id: &str) -> String {
    let s: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.is_empty() { "_".into() } else { s }
}
