//! What a paper is about, beside its LaTeX: `paper.toml` in the paper folder
//! holds the key value (the one thing the whole paper argues) and the
//! authors picked from the profile. Every paper needs a key value; a new
//! paper gets it in 討論草稿 step 1 (or the editor's key value bar).

use crate::authors::PaperAuthor;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const META_FILE: &str = "paper.toml";
/// Limits: Chinese counts characters, English counts words.
pub const KEY_VALUE_MAX_CJK: usize = 50;
pub const KEY_VALUE_MAX_WORDS: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PaperMeta {
    #[serde(default)]
    pub key_value: String,
    /// Profile authors (ids into authors.csv). Left out of paper.toml when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<PaperAuthor>,
    /// The template under `publishers/` the paper is on: chosen when it was
    /// made or in 討論草稿's 版型 step, or recognised by an import (its class
    /// and style files match byte for byte). None = the plain article.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal: Option<String>,
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF | 0x20000..=0x2FA1F)
}

/// Length of a key value and its limit: CJK characters plus Latin words
/// against 50 when it has any CJK, else words against 30.
pub fn key_value_length(s: &str) -> (usize, usize) {
    let cjk = s.chars().filter(|&c| is_cjk(c)).count();
    let words = s
        .split(|c: char| c.is_whitespace() || is_cjk(c) || "，。、；：！？（）「」,;:!?()".contains(c))
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .count();
    if cjk > 0 {
        (cjk + words, KEY_VALUE_MAX_CJK)
    } else {
        (words, KEY_VALUE_MAX_WORDS)
    }
}

/// The key value, trimmed, or why it is not acceptable.
pub fn check_key_value(s: &str) -> Result<String, String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        return Err("請填寫 Key value：這篇論文通篇要強調的一件事".into());
    }
    let (n, max) = key_value_length(&s);
    if n > max {
        let unit = if max == KEY_VALUE_MAX_CJK { "字" } else { "個英文單字" };
        return Err(format!("Key value 最多 {max} {unit}，目前 {n}"));
    }
    Ok(s)
}

pub fn read_meta(paper: &Path) -> PaperMeta {
    std::fs::read_to_string(paper.join(META_FILE)).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

pub fn write_meta(paper: &Path, meta: &PaperMeta) -> Result<(), String> {
    let text = toml::to_string(meta).map_err(|e| e.to_string())?;
    crate::paper::write_atomic(&paper.join(META_FILE), &text).map_err(|e| e.to_string())
}

/// A section planned before writing: its heading, one or two sentences on
/// what it will say, and its subsections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OutlineSection {
    pub title: String,
    #[serde(default)]
    pub intent: String,
    #[serde(default)]
    pub subsections: Vec<OutlineSub>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OutlineSub {
    pub title: String,
    #[serde(default)]
    pub intent: String,
}

/// Escape what breaks LaTeX in plain text from a model.
pub fn tex_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.trim().chars() {
        if matches!(c, '&' | '%' | '$' | '#' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl OutlineSection {
    /// Paragraphs of the section file: the intent, then each subsection
    /// heading followed by its intent.
    pub fn paragraphs(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.intent.trim().is_empty() {
            out.push(tex_text(&self.intent));
        }
        for s in &self.subsections {
            out.push(format!("\\subsection{{{}}}", tex_text(&s.title)));
            if !s.intent.trim().is_empty() {
                out.push(tex_text(&s.intent));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_value_counts_chinese_characters_and_english_words() {
        assert_eq!(key_value_length("提升髖部骨折判讀的可解釋性"), (13, 50));
        assert_eq!(key_value_length("Improve the explainability of fracture detection"), (6, 30));
        assert_eq!(key_value_length("提升 AI 模型在 PXR 的效能"), (10, 50));
        assert!(check_key_value("  ").is_err());
        assert!(check_key_value(&"字".repeat(51)).is_err());
        assert_eq!(check_key_value(&"字".repeat(50)).unwrap().chars().count(), 50);
        assert!(check_key_value(&"word ".repeat(31)).is_err());
        assert_eq!(check_key_value("  a   b ").unwrap(), "a b");
    }

    #[test]
    fn meta_round_trips() {
        let dir = std::env::temp_dir().join(format!("pc-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_meta(&dir), PaperMeta::default());
        let m = PaperMeta { key_value: "提升效能".into(), authors: vec![PaperAuthor { id: "A001".into(), corresponding: true, equal: false }], ..Default::default() };
        write_meta(&dir, &m).unwrap();
        assert_eq!(read_meta(&dir), m);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn outline_section_paragraphs() {
        let s = OutlineSection {
            title: "Method".into(),
            intent: "Describe the 3 steps & data.".into(),
            subsections: vec![OutlineSub { title: "Data_set".into(), intent: "Where the 100% data came from.".into() }],
        };
        assert_eq!(s.paragraphs(), ["Describe the 3 steps \\& data.", "\\subsection{Data\\_set}", "Where the 100\\% data came from."]);
    }
}
