//! The paper's text as the reader page sees it: pdf.js text items per page, in
//! order, with the page text built from them exactly like `pageText()` in
//! `web/app.js` (items' `str` concatenated, `\n` after an item that ends a line).
//! Offsets are UTF-16 code units into that page text, so every range the server
//! computes (term occurrences, math spans, paragraphs) can be marked in the text
//! layer without any re-alignment.
//!
//! The page sends this once per opened paper (`POST /api/papers/{sha}/read`); the
//! server keeps it in memory and never writes the paper's text to the workspace.

use serde::{Deserialize, Serialize};

/// One pdf.js text item.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// UTF-16 range of `str` in the page text.
    pub s: usize,
    pub e: usize,
    #[serde(default)]
    pub str: String,
    /// The PDF font's name (`ABCDEF+CMMI10`, `Times-Italic`, …) — empty when unknown.
    #[serde(default)]
    pub font: String,
    /// Font size in PDF points (the larger of the matrix's scale terms).
    #[serde(default)]
    pub size: f32,
    /// Baseline origin in PDF user space (y grows upwards).
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub h: f32,
    /// The item ends a line (pdf.js `hasEOL`).
    #[serde(default)]
    pub eol: bool,
    /// pdf.js says the font is italic / bold (from the font object, when loaded).
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub bold: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PageText {
    /// 1-based page number in the paper's own PDF.
    #[serde(default)]
    pub page: u32,
    pub text: String,
    #[serde(default)]
    pub items: Vec<Item>,
    /// Page size in PDF points.
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub height: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaperText {
    pub pages: Vec<PageText>,
}

impl PaperText {
    /// Pages from plain strings (tests; no items, no geometry).
    pub fn from_strings(pages: &[&str]) -> PaperText {
        PaperText { pages: pages.iter().enumerate().map(|(i, t)| PageText { page: i as u32 + 1, text: t.to_string(), ..Default::default() }).collect() }
    }

    /// Fill in page numbers that were not sent and drop items whose ranges do not fit.
    pub fn normalise(mut self) -> PaperText {
        for (i, p) in self.pages.iter_mut().enumerate() {
            if p.page == 0 {
                p.page = i as u32 + 1;
            }
            let n = p.text.encode_utf16().count();
            p.items.retain(|it| it.s <= it.e && it.e <= n);
        }
        self
    }

    pub fn words(&self) -> usize {
        self.pages.iter().map(|p| p.text.split_whitespace().count()).sum()
    }
}

/// Byte ↔ UTF-16 offsets of one string.
pub struct U16Map {
    /// UTF-16 offset at every byte boundary (len + 1 entries).
    at: Vec<usize>,
}

impl U16Map {
    pub fn new(s: &str) -> U16Map {
        let mut at = vec![0usize; s.len() + 1];
        let mut u = 0;
        for (i, c) in s.char_indices() {
            for k in 0..c.len_utf8() {
                at[i + k] = u;
            }
            u += c.len_utf16();
        }
        at[s.len()] = u;
        U16Map { at }
    }
    /// UTF-16 offset of a byte offset.
    pub fn u16(&self, byte: usize) -> usize {
        self.at[byte.min(self.at.len() - 1)]
    }
    /// The byte offset of a UTF-16 offset (first byte at or after it).
    pub fn byte(&self, u: usize) -> usize {
        self.at.partition_point(|&x| x < u)
    }
}

/// Byte offset of the line that starts the references ("References", "Bibliography",
/// "參考文獻", optionally numbered).
pub fn references_start(text: &str) -> Option<usize> {
    let mut off = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c.is_whitespace()).to_lowercase();
        if matches!(t.as_str(), "references" | "bibliography" | "literature cited" | "參考文獻" | "reference") {
            return Some(off);
        }
        off += line.len();
    }
    None
}

/// Where the body ends: (page index, UTF-16 offset in that page), or `None` when the
/// paper has no references heading.
pub fn body_end(paper: &PaperText) -> Option<(usize, usize)> {
    // Only look in the second half of the paper (a "References" line in a table of
    // contents or an early heading should not cut the body).
    let n = paper.pages.len();
    for (i, p) in paper.pages.iter().enumerate() {
        if n > 2 && i < n / 2 {
            continue;
        }
        if let Some(b) = references_start(&p.text) {
            return Some((i, U16Map::new(&p.text).u16(b)));
        }
    }
    None
}

/// Strip a subset tag: `ABCDEF+CMMI10` → `CMMI10`.
pub fn base_font(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_and_references() {
        let m = U16Map::new("a中𝑥b");
        assert_eq!((m.u16(0), m.u16(1), m.u16(4), m.u16(8), m.u16(9)), (0, 1, 2, 4, 5));
        assert_eq!((m.byte(2), m.byte(4)), (4, 8));
        let p = PaperText::from_strings(&["Intro\nreferences are cited", "Body", "x\n7. References\n[1] A."]);
        assert_eq!(body_end(&p), Some((2, 2)));
        assert_eq!(base_font("ABCDEF+CMMI10"), "CMMI10");
        assert_eq!(base_font("Times-Italic"), "Times-Italic");
    }
}
