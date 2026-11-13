//! `source`: the starting paper as material — its text, figures and references from the PDF
//! (lore-docsrc, read-only on LoreReader's library), and what the author did while reading it,
//! from LoreReader's files on disk (read-only): 懂了 paragraphs (`reading.json`), 問這篇 questions
//! (`chat.json`), quotes and highlights, the symbol table; plus the card's `extras`.

use std::path::Path;

use lore_docsrc::AssetKind;
use lore_learn::reader::{Question, Reader};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What LoreReader's 寫 review card sends along (docs/book.md `POST /api/book`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Extras {
    pub terms: Vec<ExtraTerm>,
    /// LoreReader paragraph ids without 懂了.
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
    pub sha: String,
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
    /// LoreReader's `symbols.json`, as it is.
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
}

pub struct DocsrcExtract;

impl Extract for DocsrcExtract {
    fn extract(&self, pdf: &Path, out: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        let mut o = lore_docsrc::pdf::PdfOptions::new(out);
        o.crop_dpi = 200;
        lore_docsrc::pdf::extract(pdf, &o).map_err(|e| format!("PDF 無法分析：{e}"))
    }
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// The material of LoreReader paper `sha` (blocking). Crops land in `img/`; `scratch` is the
/// extractor's own output folder (the cache tier).
pub fn read(reader: &Reader, sha: &str, extract: &dyn Extract, scratch: &Path, img: &Path, extras: Extras) -> Result<Material, String> {
    let meta = reader.paper_json(sha).ok_or("LoreReader 沒有這篇論文")?;
    let pdf = reader.pdf(sha).ok_or("LoreReader 的書庫裡沒有這篇的 PDF")?;
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
        let id = format!("fig{}", figures.len() + 1);
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
        sha: sha.to_string(),
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
