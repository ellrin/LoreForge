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
    // Not in the first third of the paper (a "References" line in a table of contents
    // or an early heading should not cut the body); a submission PDF (response letter,
    // manuscript, then a marked-up copy) has its first list just before the middle.
    let n = paper.pages.len();
    for (i, p) in paper.pages.iter().enumerate() {
        if n > 2 && i < n / 3 {
            continue;
        }
        if let Some(b) = references_start(&p.text) {
            return Some((i, U16Map::new(&p.text).u16(b)));
        }
    }
    None
}

/// The paper's title from its first page: the block of lines in the largest type of the
/// upper part of the page (what a reader sees as the headline). `None` when the page has
/// no clearly larger type, or the block does not look like a title.
pub fn title_from_first_page(p: &PageText) -> Option<String> {
    let top = if p.height > 0.0 { p.height * 0.3 } else { f32::MIN };
    let words: Vec<&Item> = p.items.iter().filter(|i| !i.str.trim().is_empty() && i.size > 0.0 && i.y >= top).collect();
    if words.len() < 2 {
        return None;
    }
    let mut sizes: Vec<f32> = p.items.iter().filter(|i| !i.str.trim().is_empty() && i.size > 0.0).map(|i| i.size).collect();
    sizes.sort_by(|a, b| a.total_cmp(b));
    let body = sizes[sizes.len() / 2];
    let big = words.iter().map(|i| i.size).fold(0.0f32, f32::max);
    if big < body * 1.2 || big < 11.0 {
        return None;
    }
    // items in that type, top line first
    let mut hit: Vec<&Item> = words.into_iter().filter(|i| i.size >= big - 0.8).collect();
    hit.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
    // lines: items whose baselines are within half a line of each other
    let mut lines: Vec<(f32, Vec<&Item>)> = vec![];
    for it in hit {
        match lines.last_mut() {
            Some((y, l)) if (*y - it.y).abs() < big * 0.5 => l.push(it),
            _ => lines.push((it.y, vec![it])),
        }
    }
    // the first block: lines about one line-height apart
    let mut out: Vec<String> = vec![];
    let mut prev: Option<f32> = None;
    for (y, mut l) in lines {
        if prev.is_some_and(|p| p - y > big * 2.2) {
            break;
        }
        l.sort_by(|a, b| a.x.total_cmp(&b.x));
        out.push(l.iter().map(|i| i.str.as_str()).collect::<Vec<_>>().join(" "));
        prev = Some(y);
    }
    let mut t = String::new();
    for line in out {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.ends_with('-') && line.chars().next().is_some_and(|c| c.is_lowercase()) {
            t.pop();
        } else if !t.is_empty() {
            t.push(' ');
        }
        t += &line;
    }
    let t = t.trim().to_string();
    let n = t.chars().count();
    ((6..=250).contains(&n) && t.split_whitespace().count() >= 2).then_some(t)
}

// ── margin line numbers and manuscript boilerplate ───────────────────────

/// The items of one page that are margin line numbers of a line-numbered manuscript: an
/// integer alone (1–5 digits), in one column at a fixed x (left- or right-aligned) in the
/// outer fifth of the page, at least five of them, counting up down the page. pdf.js puts
/// such a number right after the line's last word ("the" + "275"), so they glue to words.
pub fn line_number_items(p: &PageText) -> Vec<usize> {
    let num = |it: &Item| -> Option<u32> {
        let t = it.str.trim();
        if !t.is_empty() && t.len() <= 5 && t.bytes().all(|b| b.is_ascii_digit()) { t.parse().ok() } else { None }
    };
    let width = if p.width > 0.0 { p.width } else { p.items.iter().map(|i| i.x + i.w).fold(0.0f32, f32::max) };
    if width <= 0.0 {
        return vec![];
    }
    let cands: Vec<(usize, u32)> = p
        .items
        .iter()
        .enumerate()
        .filter_map(|(k, it)| {
            let n = num(it)?;
            (it.x + it.w <= width * 0.2 || it.x >= width * 0.8).then_some((k, n))
        })
        .collect();
    if cands.len() < 5 {
        return vec![];
    }
    // columns by left edge and by right edge (2-pt buckets); the largest one wins
    let mut best: Vec<(usize, u32)> = vec![];
    for edge in [0, 1] {
        let mut cols: std::collections::BTreeMap<i32, Vec<(usize, u32)>> = Default::default();
        for &(k, n) in &cands {
            let it = &p.items[k];
            let x = if edge == 0 { it.x } else { it.x + it.w };
            cols.entry((x / 2.0).round() as i32).or_default().push((k, n));
        }
        // neighbouring buckets belong together (a value on a bucket edge)
        let keys: Vec<i32> = cols.keys().copied().collect();
        for &c in &keys {
            let mut col: Vec<(usize, u32)> = (c - 1..=c + 1).filter_map(|b| cols.get(&b)).flatten().copied().collect();
            col.sort();
            col.dedup();
            if col.len() > best.len() {
                best = col;
            }
        }
    }
    if best.len() < 5 {
        return vec![];
    }
    best.sort_by(|a, b| p.items[b.0].y.total_cmp(&p.items[a.0].y));
    let up = best.windows(2).filter(|w| w[1].1 > w[0].1).count();
    if (up as f32) < (best.len() - 1) as f32 * 0.8 {
        return vec![];
    }
    let mut out: Vec<usize> = best.into_iter().map(|(k, _)| k).collect();
    out.sort();
    out
}

/// Replace every margin line number with spaces (same UTF-16 length, so every offset
/// stays valid) in the text and items. Returns `(page, start, end)` of each.
pub fn blank_line_numbers(t: &mut PaperText) -> Vec<(u32, usize, usize)> {
    let mut out = vec![];
    for p in &mut t.pages {
        let hits = line_number_items(p);
        if hits.is_empty() {
            continue;
        }
        let map = U16Map::new(&p.text);
        let mut bytes = std::mem::take(&mut p.text).into_bytes();
        for k in hits {
            let it = &mut p.items[k];
            let (b0, b1) = (map.byte(it.s), map.byte(it.e));
            if b1 > bytes.len() || !bytes[b0..b1].iter().all(|b| b.is_ascii_digit() || *b == b' ') {
                continue;
            }
            bytes[b0..b1].iter_mut().for_each(|b| *b = b' ');
            it.str = " ".repeat(it.str.len());
            out.push((p.page, it.s, it.e));
        }
        p.text = String::from_utf8(bytes).expect("ASCII replaced by ASCII");
    }
    out
}

/// Lines no reader reads as the paper: submission-system boilerplate ("Click here to
/// view linked References", "--Manuscript Draft--", "Powered by Editorial Manager…") and
/// running headers / footers (the same line, digits ignored, on at least three pages and
/// a third of them). `(page, start, end)`, UTF-16.
pub fn boilerplate_lines(t: &PaperText) -> Vec<(u32, usize, usize)> {
    const PATTERNS: &[&str] = &["click here to", "manuscript draft", "editorial manager", "produxion manager", "aries systems", "journal pre-proof"];
    let shape = |l: &str| -> String { l.chars().filter(|c| c.is_alphabetic()).flat_map(char::to_lowercase).collect() };
    let mut seen: std::collections::HashMap<String, std::collections::HashSet<u32>> = Default::default();
    for p in &t.pages {
        for l in p.text.split('\n') {
            let k = shape(l);
            if k.len() >= 6 {
                seen.entry(k).or_default().insert(p.page);
            }
        }
    }
    let n = t.pages.len();
    let mut out = vec![];
    for p in &t.pages {
        let mut u = 0;
        for l in p.text.split('\n') {
            let len = l.encode_utf16().count();
            let low = l.to_lowercase();
            let repeated = seen.get(&shape(l)).is_some_and(|pages| n >= 3 && pages.len() >= 3 && pages.len() * 3 >= n);
            if !l.trim().is_empty() && (PATTERNS.iter().any(|q| low.contains(q)) || repeated) {
                out.push((p.page, u, u + len));
            }
            u += len + 1;
        }
    }
    out
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

    fn it(str: &str, x: f32, y: f32, size: f32) -> Item {
        Item { str: str.into(), x, y, size, ..Default::default() }
    }

    #[test]
    fn title_is_the_largest_block_on_the_first_page() {
        let mut items = vec![it("Deep Learning for Retinal", 72.0, 720.0, 20.0), it("Image Triage", 72.0, 695.0, 20.0), it("A. Author, B. Author", 72.0, 660.0, 11.0), it("Abstract", 72.0, 620.0, 12.0)];
        for k in 0..30 {
            items.push(it("body text of the paper goes here", 72.0, 580.0 - k as f32 * 12.0, 10.0));
        }
        let p = PageText { page: 1, text: String::new(), items, width: 612.0, height: 792.0 };
        assert_eq!(title_from_first_page(&p).as_deref(), Some("Deep Learning for Retinal Image Triage"));
        // no larger type at all: no guess
        let flat = PageText { items: (0..20).map(|k| it("same size text here", 72.0, 700.0 - k as f32 * 12.0, 10.0)).collect(), ..p.clone() };
        assert_eq!(title_from_first_page(&flat), None);
        // a hyphenated line break is joined
        let hy = PageText { items: [vec![it("Segmen-", 72.0, 720.0, 20.0), it("tation of Vessels", 72.0, 695.0, 20.0)], p.items[2..].to_vec()].concat(), ..p };
        assert_eq!(title_from_first_page(&hy).as_deref(), Some("Segmentation of Vessels"));
    }


    /// A line-numbered manuscript page: body lines with the number pdf.js puts right
    /// after each line's last word, in a right-aligned column in the left margin.
    fn numbered_page(first: u32, lines: &[&str]) -> PageText {
        let mut text = String::new();
        let mut items = vec![];
        let mut push = |text: &mut String, s: &str, x: f32, y: f32, w: f32, eol: bool| {
            let a = text.encode_utf16().count();
            text.push_str(s);
            items.push(Item { s: a, e: text.encode_utf16().count(), str: s.into(), x, y, w, size: 10.0, eol, ..Default::default() });
            if eol {
                text.push('\n');
            }
        };
        for (k, l) in lines.iter().enumerate() {
            let y = 740.0 - k as f32 * 17.0;
            let n = (first + k as u32).to_string();
            push(&mut text, l, 77.0, y, 400.0, false);
            push(&mut text, &n, 59.0 - 6.0 * n.len() as f32, y, 6.0 * n.len() as f32, true);
        }
        push(&mut text, "5", 514.0, 43.0, 5.0, false); // the page number: not a line number
        PageText { page: 1, text, items, width: 612.0, height: 792.0 }
    }

    #[test]
    fn margin_line_numbers_are_blanked_with_offsets_kept() {
        let lines = ["the method is similar", "to the one we transpose", "into a new shape and", "keep the rest of the", "pipeline as it was", "before this change"];
        let p = numbered_page(98, &lines);
        let before = p.text.encode_utf16().count();
        let mut t = PaperText { pages: vec![p] };
        let hits = blank_line_numbers(&mut t);
        assert_eq!(hits.len(), 6);
        let txt = &t.pages[0].text;
        assert_eq!(txt.encode_utf16().count(), before);
        assert!(txt.starts_with("the method is similar  \n"), "{txt:?}");
        assert!(!txt.contains("similar98") && !txt.contains("99") && !txt.contains("103"));
        assert!(txt.ends_with('5'), "the page number stays");
        // numbers in the body (a table column) are not a margin column
        let mut q = numbered_page(1, &lines);
        for it in q.items.iter_mut().filter(|i| i.x < 70.0) {
            it.x += 250.0;
        }
        assert!(line_number_items(&q).is_empty());
        // numbers that do not count up are not line numbers
        let mut r = numbered_page(1, &lines);
        let nums: Vec<usize> = (0..r.items.len()).filter(|&k| r.items[k].x < 70.0).collect();
        let vals = ["7", "2", "9", "1", "8", "3"];
        for (k, v) in nums.iter().zip(vals) {
            r.items[*k].str = v.into();
        }
        assert!(line_number_items(&r).is_empty());
    }

    #[test]
    fn manuscript_boilerplate_and_running_heads() {
        let p = |b: &str| format!("Journal of Synthetic Studies\n{b}\nClick here to view linked References\n");
        let t = PaperText::from_strings(&[&format!("--Manuscript Draft--\n{}", p("Intro text")), &p("More text"), &p("Even more"), &p("Last")]);
        let b = boilerplate_lines(&t);
        let on = |page: u32, needle: &str| {
            let txt = &t.pages[page as usize - 1].text;
            let at = txt[..txt.find(needle).unwrap()].encode_utf16().count();
            b.iter().any(|&(pg, s, e)| pg == page && s <= at && at < e)
        };
        assert!(on(1, "Manuscript Draft") && on(1, "Click here") && on(2, "Journal of Synthetic") && on(4, "Click here"));
        assert!(!on(1, "Intro text") && !on(3, "Even more"));
    }

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
