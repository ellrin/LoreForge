//! `source`: one material per starting point (docs/book.md §統一入口). An L-Reader paper: its
//! text, figures and references from the PDF (lore-docsrc, read-only on L-Reader's library), and
//! what the author did while reading it, from L-Reader's files on disk (read-only): 懂了
//! paragraphs (`reading.json`), 問這篇 questions (`chat.json`), quotes and highlights, the symbol
//! table; plus the card's `extras`. A PDF file, and every PDF in a folder: its text only (no
//! figure crops), and only when it is a paper ([`paper_rule`]; L-Shelf's `cloud_ok` decides while
//! it runs). A material never holds a path: files are named by their file name.

use std::path::Path;

use lore_docsrc::AssetKind;
use lore_learn::reader::{Question, Reader};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What L-Reader's 寫 review card sends along (docs/book.md `POST /api/book`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Extras {
    pub terms: Vec<ExtraTerm>,
    /// L-Reader paragraph ids without 懂了.
    pub stuck: Vec<String>,
    pub questions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExtraTerm {
    pub term: String,
    pub n: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Fig {
    /// `fig1` …
    pub id: String,
    pub number: String,
    pub caption: String,
    /// The crop in the job's `img/` folder, when there is one.
    pub file: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Quote {
    pub text: String,
    pub note: String,
    pub page: u64,
}

/// `source.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Material {
    /// `lr` (an L-Reader paper; empty in older jobs) · `file` (a PDF file or one of a folder's)
    pub kind: String,
    /// L-Reader's sha (`lr`); a file's own sha256 (`file`): the same paper is read once.
    pub sha: String,
    /// The file name (`file`): what the page shows when there is no title.
    pub label: String,
    pub arxiv: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: String,
    pub venue: String,
    pub doi: String,
    pub abstract_text: String,
    pub keywords: Vec<String>,
    /// `(heading, plain text)` in reading order.
    pub sections: Vec<(String, String)>,
    pub figures: Vec<Fig>,
    pub refs: usize,
    /// L-Reader's `symbols.json`, as it is.
    pub symbols: Option<Value>,
    /// Paragraph ids marked 懂了.
    pub understood: Vec<String>,
    pub questions: Vec<Question>,
    pub quotes: Vec<Quote>,
    pub extras: Extras,
}

impl Material {
    /// Text the term finder reads: the sections, then the questions (as `問這篇`).
    pub fn term_items(&self) -> Vec<(String, String)> {
        let mut v = self.sections.clone();
        if !self.abstract_text.is_empty() {
            v.insert(0, ("Abstract".into(), self.abstract_text.clone()));
        }
        for q in self.questions.iter().map(|q| q.text.clone()).chain(self.extras.questions.iter().cloned()) {
            v.push(("問這篇".into(), q));
        }
        v
    }

    /// The parts of the paper that mention any of `words` (case-insensitive), at most `budget` characters.
    pub fn passages(&self, words: &[String], budget: usize) -> String {
        let lw: Vec<String> = words.iter().map(|w| w.to_lowercase()).filter(|w| w.len() > 1).collect();
        let mut out = String::new();
        for (h, text) in &self.sections {
            for p in text.split("\n\n") {
                let l = p.to_lowercase();
                if lw.iter().any(|w| l.contains(w)) {
                    let piece = format!("[{h}] {}\n", p.trim());
                    if out.chars().count() + piece.chars().count() > budget {
                        return out;
                    }
                    out += &piece;
                }
            }
        }
        out
    }
}

/// How the PDF becomes a catalog (lore-docsrc + poppler; a synthetic one in tests).
pub trait Extract: Send + Sync {
    fn extract(&self, pdf: &Path, out: &Path) -> Result<lore_docsrc::AssetCatalog, String>;
    /// Text only, no figure crops (files and folders).
    fn text(&self, pdf: &Path, out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        self.extract(pdf, out)
    }
}

pub struct DocsrcExtract;

impl Extract for DocsrcExtract {
    fn extract(&self, pdf: &Path, out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        let mut o = lore_docsrc::pdf::PdfOptions::new(out);
        o.crop_dpi = 200;
        lore_docsrc::pdf::extract(pdf, &o).map_err(|e| format!("PDF 無法分析：{e}"))
    }

    fn text(&self, pdf: &Path, out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        let mut o = lore_docsrc::pdf::PdfOptions::new(out);
        o.render_crops = false;
        o.save = false;
        lore_docsrc::pdf::extract(pdf, &o).map_err(|e| format!("PDF 無法分析：{e}"))
    }
}

/// `source.json`: one material per starting point (an older job's file is one [`Material`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Sources {
    pub materials: Vec<Material>,
    /// PDF files read so far, of how many (files and folders).
    pub files: usize,
    pub of: usize,
    /// Files left out (not a paper, unreadable): counted, never named in prompts.
    pub skipped: usize,
    pub notes: Vec<String>,
}

impl Sources {
    /// The `source` checkpoint, old (one material) or new.
    pub fn from_value(v: Value) -> Result<Sources, String> {
        if v.get("materials").is_some() {
            return serde_json::from_value(v).map_err(|e| e.to_string());
        }
        let m: Material = serde_json::from_value(v).map_err(|e| e.to_string())?;
        Ok(Sources { materials: vec![Material { kind: if m.kind.is_empty() { "lr".into() } else { m.kind }, ..m }], ..Default::default() })
    }

    /// The papers among the materials (all of them are papers; the name says what they are for).
    pub fn papers(&self) -> &[Material] {
        &self.materials
    }

    /// The L-Reader shas (books.toml `papers`).
    pub fn shas(&self) -> Vec<String> {
        self.materials.iter().filter(|m| m.kind == "lr" && !m.sha.is_empty()).map(|m| m.sha.clone()).collect()
    }

    /// Questions the author asked while reading, every paper's and the card's.
    pub fn questions(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for m in &self.materials {
            for q in m.questions.iter().map(|q| q.text.clone()).chain(m.extras.questions.iter().cloned()) {
                if !out.contains(&q) {
                    out.push(q);
                }
            }
        }
        out
    }

    /// The material holding figure `id` (ids are unique across materials).
    pub fn figure(&self, id: &str) -> Option<(usize, &Fig)> {
        self.materials.iter().enumerate().find_map(|(i, m)| m.figures.iter().find(|f| f.id == id).map(|f| (i, f)))
    }
}

/// A PDF counts as a paper (without L-Shelf): lore-docsrc found a title and references.
pub fn paper_rule(cat: &lore_docsrc::AssetCatalog) -> bool {
    !cat.meta.title.trim().is_empty() && !cat.references.is_empty()
}

/// A file's material from its catalog (text only). `label` = the file name.
pub fn file_material(cat: &lore_docsrc::AssetCatalog, label: &str) -> Material {
    let sections: Vec<(String, String)> = cat
        .section_texts(lore_docsrc::text::TextOptions { captions: false, math: true })
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| (s.title, s.text))
        .collect();
    Material {
        kind: "file".into(),
        label: label.to_string(),
        title: cat.meta.title.trim().to_string(),
        authors: cat.meta.authors.clone(),
        arxiv: cat.meta.arxiv_id.clone().unwrap_or_default(),
        abstract_text: cat.meta.abstract_text.clone().unwrap_or_default(),
        keywords: cat.meta.keywords.clone(),
        sections,
        refs: cat.references.len(),
        ..Default::default()
    }
}

/// The PDFs of a folder: sorted, hidden files and folders left out, at most `depth` levels down
/// and `max` files (the second value: more were there).
pub fn folder_pdfs(dir: &Path, depth: usize, max: usize) -> (Vec<std::path::PathBuf>, bool) {
    let mut out = vec![];
    let mut more = false;
    fn walk(d: &Path, depth: usize, max: usize, out: &mut Vec<std::path::PathBuf>, more: &mut bool) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut items: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
        items.sort();
        for p in items {
            if p.is_symlink() {
                continue;
            }
            if p.is_dir() {
                if depth > 0 {
                    walk(&p, depth - 1, max, out, more);
                }
            } else if super::setup::is_pdf(&p) {
                if out.len() >= max {
                    *more = true;
                    return;
                }
                out.push(p);
            }
        }
    }
    walk(dir, depth, max, &mut out, &mut more);
    (out, more)
}

/// sha256 of a file (lower-case hex): the checkpoint key and L-Shelf's item id.
pub fn file_sha(path: &Path) -> std::io::Result<String> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// The material of L-Reader paper `sha` (blocking). Crops land in `img/`; `scratch` is the
/// extractor's own output folder (the cache tier).
pub fn read(reader: &Reader, sha: &str, extract: &dyn Extract, scratch: &Path, img: &Path, extras: Extras) -> Result<Material, String> {
    read_as(reader, sha, extract, scratch, img, extras, "fig")
}

/// [`read`] with the figure ids' prefix (`fig` for the first paper, `p2fig` … after it, so ids
/// stay unique across materials).
pub fn read_as(reader: &Reader, sha: &str, extract: &dyn Extract, scratch: &Path, img: &Path, extras: Extras, fig_prefix: &str) -> Result<Material, String> {
    let meta = reader.paper_json(sha).ok_or("L-Reader 沒有這篇論文")?;
    let pdf = reader.pdf(sha).ok_or("L-Reader 的書庫裡沒有這篇的 PDF")?;
    let cat = extract.extract(&pdf, scratch)?;
    let authors: Vec<String> = match meta.get("authors") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
        Some(Value::String(x)) => x.split([';', '\n']).flat_map(|p| p.split(" and ")).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
        _ => cat.meta.authors.clone(),
    };
    let title = Some(text_of(meta.get("title"))).filter(|t| !t.is_empty()).unwrap_or_else(|| cat.meta.title.clone());
    let sections: Vec<(String, String)> = cat
        .section_texts(lore_docsrc::text::TextOptions { captions: false, math: true })
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| (s.title, s.text))
        .collect();
    std::fs::create_dir_all(img).map_err(|e| e.to_string())?;
    let mut figures = vec![];
    for a in cat.assets.iter().filter(|a| a.kind == AssetKind::Figure) {
        let id = format!("{fig_prefix}{}", figures.len() + 1);
        let file = a.crop.as_ref().and_then(|c| std::fs::read(scratch.join(c)).ok()).and_then(|b| {
            let name = format!("{id}.png");
            lore_jobs::write_atomic(&img.join(&name), &b).ok().map(|_| name)
        });
        figures.push(Fig { id, number: a.number.clone().unwrap_or_default(), caption: a.caption.clone().unwrap_or_default(), file });
    }
    let quotes = crate::reader::material(reader.papers.parent().unwrap_or(Path::new("")), sha)
        .into_iter()
        .filter(|m| !m.text.is_empty() || !m.note.is_empty())
        .map(|m| Quote { text: m.text, note: m.note, page: m.page })
        .collect();
    Ok(Material {
        kind: "lr".into(),
        sha: sha.to_string(),
        arxiv: cat.meta.arxiv_id.clone().unwrap_or_default(),
        label: title.clone(),
        title,
        authors,
        year: text_of(meta.get("year")),
        venue: text_of(meta.get("venue")),
        doi: meta.get("doi").and_then(Value::as_str).map(crate::reader::norm_doi).unwrap_or_default(),
        abstract_text: cat.meta.abstract_text.clone().unwrap_or_default(),
        keywords: cat.meta.keywords.clone(),
        sections,
        figures,
        refs: cat.references.len(),
        symbols: reader.symbols_json(sha),
        understood: reader.understood(sha),
        questions: reader.questions(sha),
        quotes,
        extras,
    })
}
