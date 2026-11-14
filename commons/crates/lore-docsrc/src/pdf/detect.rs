//! Region detection on text geometry + page ink: captions, figure and table
//! regions, numbered display equations, headings, body text, references,
//! front matter. Pure functions (no poppler), so they are unit-tested on
//! synthetic pages.

use super::layout::{Columns, Ink, Line, TextPage, Word};
use crate::model::*;
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Other,
    Body,
    Furniture,
    Heading,
    Caption,
    /// Inside a detected figure/table/equation region.
    InAsset,
    /// Front matter on page 1 (title, authors, affiliations).
    Front,
}

static FIG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:Fig\.|Figure|FIGURE|FIG\.|Fig)\s*(\d{1,2})\s*[.:|]").unwrap()
});
static TAB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:Table|TABLE)\s+(\d{1,2}|[IVXL]{1,6})\s*(?:[.:|]|$)").unwrap()
});
static NUM_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,2}(?:\.\d{1,2}){0,3})\.?\s+(\p{Lu}.{1,90})$").unwrap());
static ROMAN_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([IVX]{1,5})\.\s+(\p{Lu}[\p{Lu}0-9 ,:\-–&/()]+)$").unwrap());
static LETTER_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-H])\.\s+(\p{Lu}.{1,80})$").unwrap());
static NAMED_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?i)(abstract|references|bibliography|acknowledge?ments?|appendix|appendices|declaration of competing interest|credit authorship contribution statement|data availability|funding|conflict of interest)$").unwrap()
});
static NUM_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\d{1,2}(?:\.\d{1,2}){0,3}\.?|[IVX]{1,5}\.)$").unwrap());
static EQ_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\((\d{1,3}[a-z]?|[A-Z]\.?\d{1,2})\)$").unwrap());
static BRACKET_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[(\d{1,3})\]").unwrap());

/// Per-document analysis state.
pub struct Doc {
    /// Pages after splitting lines that pdftotext merged across the gutter.
    pub pages: Vec<TextPage>,
    pub cols: Columns,
    /// Ink per page (same order as `pages`); `None` when no render is available.
    pub inks: Vec<Option<Ink>>,
    pub roles: Vec<Vec<Role>>,
    /// Reading order of line indices per page (columns, full-width blocks).
    pub order: Vec<Vec<usize>>,
    /// Larger-size unnumbered lines count as headings ([`super::PdfOptions::unnumbered_headings`]).
    pub unnumbered_headings: bool,
}

#[derive(Debug, Clone)]
pub struct CaptionHit {
    pub kind: AssetKind,
    pub number: String,
    pub page: usize,
    /// Line indices of the caption.
    pub lines: Vec<usize>,
    pub bbox: BBox,
    pub text: String,
}

/// A numbered display equation.
#[derive(Debug, Clone)]
pub struct EqHit {
    pub page: usize,
    pub number: String,
    /// The equation without its `(n)` tag.
    pub bbox: BBox,
    pub tag: BBox,
    pub text: String,
    pub confidence: f32,
    /// Running text inside the box to paint white.
    pub masks: Vec<BBox>,
}

#[derive(Debug, Clone)]
pub struct HeadingHit {
    pub page: usize,
    pub line: usize,
    pub level: u8,
    pub number: Option<String>,
    pub title: String,
}

/// Join line texts, removing end-of-line hyphenation before a lower-case letter.
pub fn join_lines<'b>(lines: impl Iterator<Item = &'b Line>) -> String {
    let mut out = String::new();
    for l in lines {
        let t = l.text();
        if out.ends_with('-') && t.chars().next().is_some_and(|c| c.is_lowercase()) {
            let before = out[..out.len() - 1].chars().last();
            if before.is_some_and(|c| c.is_alphabetic()) {
                out.pop();
                out.push_str(&t);
                continue;
            }
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&t);
    }
    out
}

/// One Unicode math character (Word's 𝐶 𝑝 𝔁 ℒ …) as LaTeX; `None` for anything else.
fn math_char(c: char) -> Option<String> {
    let u = c as u32;
    if (0x1D400..0x1D6A4).contains(&u) {
        let i = u - 0x1D400;
        let (style, k) = (i / 52, i % 52);
        let small = k >= 26;
        let l = if small { (b'a' + (k - 26) as u8) as char } else { (b'A' + k as u8) as char };
        return Some(match (style, small) {
            (0, _) => format!("\\mathbf{{{l}}}"),
            (1, _) => l.to_string(),
            (2, _) => format!("\\boldsymbol{{{l}}}"),
            (3 | 4, false) => format!("\\mathcal{{{l}}}"),
            (3, true) => l.to_string(),
            (4, true) => format!("\\boldsymbol{{{l}}}"),
            (6, false) => format!("\\mathbb{{{l}}}"),
            _ => l.to_string(),
        });
    }
    Some(
        match c {
            'ℎ' => "h",
            'ℒ' => "\\mathcal{L}",
            'ℛ' => "\\mathcal{R}",
            'ℬ' => "\\mathcal{B}",
            'ℰ' => "\\mathcal{E}",
            'ℱ' => "\\mathcal{F}",
            'ℋ' => "\\mathcal{H}",
            'ℐ' => "\\mathcal{I}",
            'ℳ' => "\\mathcal{M}",
            'ℝ' => "\\mathbb{R}",
            'ℕ' => "\\mathbb{N}",
            'ℤ' => "\\mathbb{Z}",
            _ => return None,
        }
        .to_string(),
    )
}

/// A word made only of math characters, as LaTeX (`𝐶` → `C`).
fn math_word(w: &str) -> Option<String> {
    if w.is_empty() {
        return None;
    }
    w.chars().map(math_char).collect::<Option<Vec<_>>>().map(|v| v.concat())
}

/// A line's text with Word's math words as `$…$`: a math word right after another, smaller
/// and lowered (raised), is its subscript (superscript) — `𝐶 𝑒𝑛𝑐` → `$C_{\mathrm{enc}}$`
/// (three or more letters in a script are a name). Punctuation right after math sticks.
pub fn line_text_math(l: &Line) -> String {
    line_math(l, None).0
}

/// [`line_text_math`] with an accent carried over from the line before (Word may end a
/// line with the combining tilde of the letter that starts the next); returns the accent
/// left at the end of this line.
fn line_math(l: &Line, carry: Option<&'static str>) -> (String, Option<&'static str>) {
    // trailing punctuation of a math word is its own (glued) word: `𝑛.` → `𝑛` `.`
    let mut ws: Vec<Word> = vec![];
    for w in &l.words {
        let core = w.text.trim_end_matches([',', '.', ';', ':', ')']);
        if core.len() < w.text.len() && math_word(core).is_some() {
            let f = core.chars().count() as f64 / w.text.chars().count() as f64;
            let cut = w.bbox.x0 + (w.bbox.x1 - w.bbox.x0) * f;
            ws.push(Word { bbox: BBox::new(w.bbox.x0, w.bbox.y0, cut, w.bbox.y1), text: core.to_string() });
            ws.push(Word { bbox: BBox::new(cut, w.bbox.y0, w.bbox.x1, w.bbox.y1), text: w.text[core.len()..].to_string() });
        } else {
            ws.push(w.clone());
        }
    }
    let accent = |w: &str| -> Option<&'static str> {
        let mut cs = w.chars();
        let c = cs.next()?;
        if cs.next().is_some() {
            return None;
        }
        Some(match c {
            '\u{0303}' => "tilde",
            '\u{0302}' => "hat",
            '\u{0304}' | '\u{0305}' => "bar",
            '\u{0307}' => "dot",
            '\u{0308}' => "ddot",
            '\u{20D7}' => "vec",
            _ => return None,
        })
    };
    let mut out = String::new();
    let mut i = 0;
    let mut pending: Option<&'static str> = carry;
    while i < ws.len() {
        let w = &ws[i];
        // a combining accent set as its own word goes on the math letter next to it
        if let Some(a) = accent(&w.text) {
            if i + 1 == ws.len() || ws.get(i + 1).is_some_and(|n| math_word(&n.text).is_some()) {
                pending = Some(a);
            } else if out.ends_with('$') && let Some(open) = out[..out.len() - 1].rfind('$') {
                let inner = out[open + 1..out.len() - 1].to_string();
                out.truncate(open);
                out.push_str(&format!("$\\{a}{{{inner}}}$"));
            }
            i += 1;
            continue;
        }
        let Some(mut base) = math_word(&w.text) else {
            if !out.is_empty() {
                let glued = out.ends_with('$') && w.text.chars().all(|c| ",.;:)".contains(c)) && i > 0 && w.bbox.x0 - ws[i - 1].bbox.x1 < 0.5 * l.height();
                if !glued {
                    out.push(' ');
                }
            }
            out.push_str(&w.text);
            i += 1;
            continue;
        };
        if let Some(a) = pending.take() {
            base = format!("\\{a}{{{base}}}");
        }
        let mut tex = base;
        let mut j = i + 1;
        while j < ws.len() {
            let (a, b) = (&ws[j - 1], &ws[j]);
            let Some(next) = math_word(&b.text) else { break };
            let touching = b.bbox.x0 - a.bbox.x1 < 0.15 * w.bbox.height();
            let smaller = b.bbox.height() < w.bbox.height() * 0.85;
            if !(touching && smaller) {
                break;
            }
            let name = if b.text.chars().count() >= 3 && b.text.chars().all(|c| c.is_alphabetic()) && next.chars().all(|c| c.is_ascii_alphabetic()) { format!("\\mathrm{{{next}}}") } else { next };
            if b.bbox.y1 > w.bbox.y1 + 0.1 * w.bbox.height() {
                tex.push_str(&format!("_{{{name}}}"));
            } else if b.bbox.y0 < w.bbox.y0 - 0.1 * w.bbox.height() {
                tex.push_str(&format!("^{{{name}}}"));
            } else {
                break;
            }
            j += 1;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("${tex}$"));
        i = j;
    }
    (out, pending)
}

/// [`join_lines`] with math words as `$…$` ([`line_text_math`]), for captions.
pub fn join_lines_math<'b>(lines: impl Iterator<Item = &'b Line>) -> String {
    let mut out = String::new();
    let mut carry = None;
    for l in lines {
        let (t, c) = line_math(l, carry);
        carry = c;
        if out.ends_with('-') && t.chars().next().is_some_and(|c| c.is_lowercase()) && out[..out.len() - 1].chars().last().is_some_and(|c| c.is_alphabetic()) {
            out.pop();
            out.push_str(&t);
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&t);
    }
    out
}

impl Doc {
    pub fn new(pages: &[TextPage], inks: Vec<Option<Ink>>) -> Doc {
        let cols = Columns::detect(pages);
        let pages: Vec<TextPage> = pages.iter().map(|p| split_gutter(p, &cols)).collect();
        let roles = pages
            .iter()
            .map(|p| vec![Role::Other; p.lines.len()])
            .collect();
        let order = pages.iter().map(|p| reading_order(p, &cols)).collect();
        let mut d = Doc {
            unnumbered_headings: false,
            pages,
            cols,
            inks,
            roles,
            order,
        };
        d.classify();
        d
    }

    fn h(&self) -> f64 {
        self.cols.line_height
    }

    /// Prose line: enough words, mostly words, no wide gaps, wide enough.
    pub fn is_prose(&self, l: &Line) -> bool {
        let (x0, x1) = self.cols.span_of(&l.bbox);
        let width_ok = l.bbox.width() >= 0.55 * (x1 - x0);
        l.words.len() >= 4
            && l.wordy_ratio() >= 0.6
            && l.max_gap() <= 0.9 * l.height().max(self.h() * 0.8)
            && width_ok
    }

    fn classify(&mut self) {
        for (pi, p) in self.pages.iter().enumerate() {
            // furniture: blocks entirely in the top / bottom band
            for (li, l) in p.lines.iter().enumerate() {
                let blk = p.blocks.get(l.block).copied().unwrap_or(l.bbox);
                if blk.y1 < 0.07 * p.height || blk.y0 > 0.945 * p.height {
                    self.roles[pi][li] = Role::Furniture;
                }
            }
            // prose lines, then whole prose blocks
            let mut prose = vec![false; p.lines.len()];
            for (li, l) in p.lines.iter().enumerate() {
                prose[li] = self.roles[pi][li] == Role::Other && self.is_prose(l);
            }
            let nblocks = p.blocks.len().max(1);
            let mut stats = vec![(0usize, 0usize); nblocks];
            for (li, l) in p.lines.iter().enumerate() {
                if let Some(s) = stats.get_mut(l.block) {
                    s.0 += 1;
                    s.1 += prose[li] as usize;
                }
            }
            for (li, l) in p.lines.iter().enumerate() {
                let (n, k) = stats.get(l.block).copied().unwrap_or((1, 0));
                let block_prose = k >= 1 && (n >= 2 && k * 2 >= n || n == 1);
                if self.roles[pi][li] == Role::Other && (prose[li] || block_prose) {
                    self.roles[pi][li] = Role::Body;
                }
            }
        }
    }

    fn first_in_block(p: &TextPage, li: usize) -> bool {
        li == 0 || p.lines[li - 1].block != p.lines[li].block
    }

    /// Caption starts ("Fig. 3.", "Table 2", "TABLE IV") and their full text.
    pub fn captions(&mut self) -> Vec<CaptionHit> {
        let mut out = vec![];
        let mut refs_started = false;
        for (pi, p) in self.pages.iter().enumerate() {
            let mut li = 0;
            while li < p.lines.len() {
                let l = &p.lines[li];
                let t = l.text();
                if NAMED_HEAD.is_match(l.squeezed().trim())
                    && l.squeezed().to_lowercase().starts_with("ref")
                {
                    refs_started = true;
                }
                let starts = Self::first_in_block(p, li)
                    || l.bbox.y0 - p.lines[li - 1].bbox.y1 > 0.5 * self.h();
                let hit = if !starts || refs_started {
                    None
                } else if let Some(c) = FIG_RE.captures(&t) {
                    Some((AssetKind::Figure, c[1].to_string()))
                } else {
                    TAB_RE
                        .captures(&t)
                        .map(|c| (AssetKind::Table, c[1].to_string()))
                };
                let Some((kind, number)) = hit else {
                    li += 1;
                    continue;
                };
                let mut lines = vec![li];
                let mut j = li + 1;
                while j < p.lines.len() {
                    let prev = &p.lines[j - 1];
                    let n = &p.lines[j];
                    let same_block = n.block == prev.block;
                    // a sentence cut at the line end goes on in the next block (Word's math
                    // words split a caption into blocks): same left edge, right below
                    let ends = prev.text().trim_end().ends_with(['.', ':', '!', '?']);
                    let goes_on = !ends
                        && (n.bbox.x0 - p.lines[lines[0]].bbox.x0).abs() < self.h()
                        && n.bbox.y0 - prev.bbox.y1 < 0.6 * self.h()
                        && n.bbox.y0 - prev.bbox.y1 > -0.5 * self.h();
                    let gap = n.bbox.y0 - prev.bbox.y1;
                    // a "Table 1" line alone: the title is the next line(s) even across blocks
                    let short_head = lines.len() == 1
                        && kind == AssetKind::Table
                        && t.split_whitespace().count() <= 3;
                    let close = gap < 0.6 * self.h()
                        && gap > -0.5 * self.h()
                        && n.bbox.x_overlap(&l.bbox.union(&prev.bbox)) > 0.0;
                    if !(same_block || short_head && close || goes_on)
                        || gap > 0.9 * self.h()
                        || n.max_gap() > 2.5 * self.h()
                    {
                        break;
                    }
                    if FIG_RE.is_match(&n.text()) || TAB_RE.is_match(&n.text()) {
                        break;
                    }
                    // table cells right under a one-sentence caption
                    let first = &p.lines[lines[0]];
                    if n.max_gap() > 1.2 * n.height().max(self.h())
                        || prev.text().trim_end().ends_with('.')
                            && n.bbox.width() < 0.6 * first.bbox.width()
                            && (n.bbox.x0 - first.bbox.x0).abs() > self.h()
                    {
                        break;
                    }
                    lines.push(j);
                    j += 1;
                }
                let bbox = lines
                    .iter()
                    .map(|&k| p.lines[k].bbox)
                    .reduce(|a, b| a.union(&b))
                    .unwrap();
                let text = join_lines_math(lines.iter().map(|&k| &p.lines[k]));
                for &k in &lines {
                    self.roles[pi][k] = Role::Caption;
                }
                out.push(CaptionHit {
                    kind,
                    number,
                    page: pi,
                    lines,
                    bbox,
                    text,
                });
                li = j;
            }
        }
        out
    }

    /// Section headings (numbered arabic / roman / letter, or named).
    pub fn headings(&mut self) -> Vec<HeadingHit> {
        let mut out: Vec<HeadingHit> = vec![];
        let roman_doc = self
            .pages
            .iter()
            .flat_map(|p| p.lines.iter())
            .any(|l| ROMAN_HEAD.is_match(&l.text()));
        // Springer-style articles: headings are not numbered, only set in a larger
        // font. Without numbered headings, a short Title-Case line in a larger size is one.
        let h0 = self.h();
        // numbered: a "1 …" and a "2 …" heading (or a "1" / "2" set apart from its
        // Title-Case title by a quad), in body size or larger
        let set_apart = |p: &TextPage, l: &Line| {
            p.lines.iter().any(|o| {
                (o.bbox.y0 - l.bbox.y0).abs() < 2.0
                    && o.bbox.x0 > l.bbox.x1
                    && o.bbox.x0 - l.bbox.x1 < 4.0 * h0
                    && (1..=6).contains(&o.words.len())
                    && o.text().chars().next().is_some_and(char::is_uppercase)
                    && !o.text().ends_with('.')
            })
        };
        let top_numbers: std::collections::HashSet<String> = self
            .pages
            .iter()
            .flat_map(|p| p.lines.iter().map(move |l| (p, l)))
            .filter(|(_, l)| l.words.len() <= 8 && l.height() >= 0.95 * h0)
            .filter_map(|(p, l)| {
                let t = l.squeezed().trim().to_string();
                if t.ends_with('.') || t.ends_with(',') || t.ends_with('-') {
                    return None;
                }
                if ROMAN_HEAD.is_match(&t) {
                    return Some("I".to_string());
                }
                let n = t.trim_end_matches('.');
                if (n == "1" || n == "2") && set_apart(p, l) {
                    return Some(format!("apart{n}"));
                }
                NUM_HEAD.captures(&t).map(|c| c[1].trim_end_matches('.').to_string()).filter(|n| n == "1")
            })
            .collect();
        let numbered_doc = !self.unnumbered_headings || top_numbers.contains("I") || top_numbers.contains("1") || (top_numbers.contains("apart1") && top_numbers.contains("apart2"));
        let mut seen_abstract = false;
        let mut last_top = 0u32;
        let mut in_refs = false;
        for (pi, p) in self.pages.iter().enumerate() {
            for &li in &self.order[pi] {
                let l = &p.lines[li];
                let after_heading = li > 0
                    && p.lines[li - 1].block == l.block
                    && self.roles[pi][li - 1] == Role::Heading;
                if !matches!(self.roles[pi][li], Role::Other | Role::Body)
                    || !(Self::first_in_block(p, li) || after_heading)
                {
                    continue;
                }
                let mut t = l.squeezed();
                // "1" and "Introduction" set apart by a quad end up as two blocks
                let mut partner = None;
                if NUM_ONLY.is_match(t.trim()) {
                    partner = p.lines.iter().position(|o| {
                        (o.bbox.y0 - l.bbox.y0).abs() < 2.0
                            && o.bbox.x0 > l.bbox.x1
                            && o.bbox.x0 - l.bbox.x1 < 4.0 * self.h()
                            && o.words.len() <= 12
                    });
                    if let Some(k) = partner {
                        t = format!("{} {}", t.trim(), p.lines[k].squeezed());
                    }
                }
                let t = t.trim();
                let block_lines = p.lines.iter().filter(|x| x.block == l.block).count();
                let (x0, x1) = self.cols.span_of(&l.bbox);
                let title_case = |s: &str| {
                    let long: Vec<&str> = s
                        .split_whitespace()
                        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 4)
                        .collect();
                    !long.is_empty()
                        && long
                            .iter()
                            .filter(|w| w.chars().next().is_some_and(char::is_uppercase))
                            .count()
                            * 10
                            >= long.len() * 6
                };
                let numbered = LETTER_HEAD.is_match(t) || NUM_HEAD.is_match(t);
                let short = l.bbox.width() < 0.75 * (x1 - x0)
                    || block_lines == 1
                    || numbered && title_case(t) && !t.ends_with('.') && l.words.len() <= 12;
                let spaced =
                    l.words.len() >= 4 && l.words.iter().all(|w| w.text.chars().count() == 1);
                if !spaced && (!short || l.words.len() > 14 || l.height() < 0.7 * self.h()) {
                    continue;
                }
                let hit = if let Some(c) = NAMED_HEAD.captures(t) {
                    Some((1u8, None, c[1].to_string()))
                } else if spaced && t.to_lowercase().ends_with("abstract") {
                    Some((1u8, None, "Abstract".to_string()))
                } else if in_refs {
                    None
                } else if let Some(c) = ROMAN_HEAD.captures(t) {
                    Some((1, Some(c[1].to_string()), c[2].trim().to_string()))
                } else if roman_doc && let Some(c) = LETTER_HEAD.captures(t) {
                    if t.ends_with('.') || t.ends_with(',') {
                        None
                    } else {
                        Some((2, Some(c[1].to_string()), c[2].trim().to_string()))
                    }
                } else if !roman_doc && numbered_doc && let Some(c) = NUM_HEAD.captures(t) {
                    let num = c[1].to_string();
                    let title = c[2].trim().to_string();
                    let top: u32 = num.split('.').next().unwrap().parse().unwrap_or(99);
                    let level = num.split('.').count() as u8;
                    let ok_num = if level == 1 {
                        top == last_top + 1
                    } else {
                        top == last_top || top == last_top + 1 && last_top == 0
                    };
                    if ok_num && !title.ends_with('.') && !title.ends_with(',') {
                        Some((level, Some(num), title))
                    } else {
                        None
                    }
                } else if !numbered_doc
                    && seen_abstract
                    && l.height() >= 1.08 * h0
                    && l.words.len() <= 10
                    && title_case(t)
                    && !t.ends_with('.')
                    && !t.ends_with(',')
                    && !t.ends_with(':')
                    && t.chars().next().is_some_and(char::is_uppercase)
                    && t.chars().filter(|c| c.is_alphabetic()).count() >= 4
                {
                    // two sizes: the larger is a section, the smaller a subsection
                    Some((if l.height() >= 1.3 * h0 { 1 } else { 2 }, None, t.to_string()))
                } else {
                    None
                };
                let Some((level, number, mut title)) = hit else {
                    continue;
                };
                if title.eq_ignore_ascii_case("abstract") {
                    seen_abstract = true;
                }
                // a heading wrapped onto a second short Title-Case line
                let mut cont = None;
                let unnumbered_wrap = number.is_none()
                    && !numbered_doc
                    && p.lines.get(li + 1).is_some_and(|n| (n.height() - l.height()).abs() < 0.3 && n.words.len() <= 6 && title_case(&n.text()) && !n.text().ends_with('.'));
                if (number.is_some() || unnumbered_wrap)
                    && let Some(n) = p.lines.get(li + 1)
                    && n.block == l.block
                    && n.bbox.width() < 0.6 * (x1 - x0)
                    && n.words.len() <= 6
                    && !n.text().ends_with('.')
                    && title_case(&n.text())
                    && l.bbox.width() > 0.6 * (x1 - x0)
                {
                    title = format!("{title} {}", n.text());
                    cont = Some(li + 1);
                }
                if let Some(n) = &number
                    && let Ok(top) = n.split('.').next().unwrap().parse::<u32>()
                {
                    last_top = last_top.max(top);
                }
                if title.to_lowercase().starts_with("reference")
                    || title.to_lowercase() == "bibliography"
                {
                    in_refs = true;
                }
                self.roles[pi][li] = Role::Heading;
                if let Some(k) = partner.or(cont) {
                    self.roles[pi][k] = Role::Heading;
                }
                out.push(HeadingHit {
                    page: pi,
                    line: li,
                    level,
                    number,
                    title,
                });
            }
        }
        out
    }

    /// Lines that bound a region search (prose, furniture, headings, captions).
    fn is_stopper(&self, pi: usize, li: usize, skip: &[usize]) -> bool {
        !skip.contains(&li)
            && matches!(
                self.roles[pi][li],
                Role::Body | Role::Furniture | Role::Heading | Role::Caption
            )
    }

    /// The free vertical interval above (`up`) or below a caption within span `(x0, x1)`.
    fn free_interval(
        &self,
        pi: usize,
        span: (f64, f64),
        cap: &BBox,
        skip: &[usize],
        up: bool,
        taken: &[BBox],
    ) -> (f64, f64) {
        let p = &self.pages[pi];
        let probe = BBox::new(span.0, 0.0, span.1, p.height);
        let mut lo = 0.03 * p.height;
        let mut hi = 0.97 * p.height;
        let mut consider = |b: &BBox| {
            let ov = b.x_overlap(&probe);
            if ov < 0.3 * b.width().min(probe.width()) && ov < 20.0 {
                return;
            }
            if up && b.y1 <= cap.y0 + 1.0 {
                lo = lo.max(b.y1);
            }
            if !up && b.y0 >= cap.y1 - 1.0 {
                hi = hi.min(b.y0);
            }
        };
        for (li, l) in p.lines.iter().enumerate() {
            if self.is_stopper(pi, li, skip) {
                consider(&l.bbox);
            }
        }
        for t in taken {
            consider(t);
        }
        if up { (lo, cap.y0) } else { (cap.y1, hi) }
    }

    fn ink_box(&self, pi: usize, region: &BBox) -> Option<BBox> {
        let ink = self.inks.get(pi)?.as_ref()?;
        let b = ink.bbox_in(region, 1)?;
        (b.height() >= 4.0 && b.width() >= 8.0).then_some(b)
    }

    /// Ink box, or (no render) the union of words inside the region.
    fn content_box(&self, pi: usize, region: &BBox) -> Option<BBox> {
        if self.inks.get(pi).is_some_and(|i| i.is_some()) {
            return self.ink_box(pi, region);
        }
        self.pages[pi]
            .lines
            .iter()
            .flat_map(|l| l.words.iter())
            .filter(|w| region.contains(&w.bbox))
            .map(|w| w.bbox)
            .reduce(|a, b| a.union(&b))
    }

    /// Region of a figure or table caption. Returns (bbox, confidence, notes).
    pub fn region_for(
        &self,
        c: &CaptionHit,
        others: &[CaptionHit],
        taken: &[BBox],
    ) -> Option<(BBox, f32, Vec<String>)> {
        let pi = c.page;
        let p = &self.pages[pi];
        let mut span = self.cols.span_of(&c.bbox);
        let mut notes: Vec<String> = vec![];
        let prefer_up = c.kind == AssetKind::Figure;
        let attempt = |span: (f64, f64), up: bool| -> Option<BBox> {
            let (mut y0, mut y1) = self.free_interval(pi, span, &c.bbox, &c.lines, up, taken);
            if y1 - y0 < 8.0 {
                return None;
            }
            // a table (caption above) and a figure (caption below) stacked between
            // their captions share the interval: cut at the widest blank gap
            let facing = others.iter().any(|o| {
                o.page == pi
                    && o.bbox.x_overlap(&BBox::new(span.0, 0.0, span.1, 1.0)) > 10.0
                    && if up {
                        o.kind == AssetKind::Table && (o.bbox.y1 - y0).abs() < 2.0
                    } else {
                        o.kind == AssetKind::Figure && (o.bbox.y0 - y1).abs() < 2.0
                    }
            });
            if facing && let Some(ink) = self.inks.get(pi).and_then(|i| i.as_ref()) {
                let bands = ink.bands(&BBox::new(span.0, y0 + 0.5, span.1, y1 - 0.5), 0.0);
                if let Some(w) = bands
                    .windows(2)
                    .max_by(|a, b| (a[1].0 - a[0].1).partial_cmp(&(b[1].0 - b[0].1)).unwrap())
                {
                    if up {
                        y0 = w[0].1;
                    } else {
                        y1 = w[1].0;
                    }
                }
            }
            // keep clear of the bounding text lines (glyph slivers)
            let m = 0.25 * self.h();
            let region = BBox::new(span.0 - 4.0, y0 + m, span.1 + 4.0, y1 - m);
            self.content_box(pi, &region).filter(|b| b.height() >= 12.0)
        };
        let mut up = prefer_up;
        let mut found = attempt(span, up);
        if found.is_none() {
            up = !up;
            found = attempt(span, up);
            if found.is_some() {
                notes.push(if up {
                    "caption below the region".into()
                } else {
                    "caption above the region".into()
                });
            }
        }
        let mut b = found?;
        // a column-wide caption under a full-width float: widen when the other
        // column has no prose next to the region but does have ink
        if self.cols.two() && self.cols.column_of(&c.bbox).is_some() {
            let full = self.cols.full();
            let other = if self.cols.column_of(&c.bbox) == Some(0) {
                self.cols.cols[1]
            } else {
                self.cols.cols[0]
            };
            let side = BBox::new(other.0, b.y0, other.1, b.y1);
            let prose_beside = p.lines.iter().enumerate().any(|(li, l)| {
                self.is_stopper(pi, li, &c.lines) && l.bbox.intersect(&side).is_some()
            });
            let ink_beside = self
                .content_box(pi, &side)
                .is_some_and(|x| x.height() > 0.3 * b.height());
            let near = BBox::new(
                other.0,
                b.y0.min(c.bbox.y0) - 2.0 * self.h(),
                other.1,
                b.y1.max(c.bbox.y1) + 2.0 * self.h(),
            );
            let taken_beside = taken.iter().any(|t| t.intersect(&side).is_some());
            let caption_beside = p.lines.iter().enumerate().any(|(li, l)| {
                !c.lines.contains(&li)
                    && self.roles[pi][li] == Role::Caption
                    && l.bbox.intersect(&near).is_some()
            });
            if !prose_beside
                && ink_beside
                && !caption_beside
                && !taken_beside
                && let Some(w) = attempt(full, up)
            {
                b = w;
                span = full;
                notes.push("widened to full width".into());
            }
        }
        let _ = span;
        // a lone thin rule far above/below the content (page decoration) is not part of it
        if let Some(ink) = self.inks.get(pi).and_then(|i| i.as_ref()) {
            let bands = ink.bands(&b, 0.0);
            if bands.len() >= 2 {
                let (mut y0, mut y1) = (b.y0, b.y1);
                if bands[0].1 - bands[0].0 < 2.5 && bands[1].0 - bands[0].1 > 1.5 * self.h() {
                    y0 = bands[1].0;
                }
                let n = bands.len();
                if bands[n - 1].1 - bands[n - 1].0 < 2.5
                    && bands[n - 1].0 - bands[n - 2].1 > 1.5 * self.h()
                {
                    y1 = bands[n - 2].1;
                }
                if (y0, y1) != (b.y0, b.y1)
                    && let Some(t) = ink.bbox_in(&BBox::new(b.x0, y0, b.x1, y1), 1)
                {
                    b = t;
                    notes.push("dropped a stray rule".into());
                }
            }
        }
        let mut conf: f32 = 0.9;
        if notes.iter().any(|n| !n.starts_with("dropped")) {
            conf -= 0.1;
        }
        if b.height() < 30.0 {
            conf -= 0.3;
        }
        Some((
            b.pad(1.5)
                .intersect(&BBox::new(0.0, 0.0, p.width, p.height))
                .unwrap_or(b),
            (conf.max(0.1) * 100.0).round() / 100.0,
            notes,
        ))
    }

    /// Words inside a box as text rows (words regrouped by baseline, since
    /// pdftotext may split table cells into separate lines); wide gaps become tabs.
    pub fn text_in(&self, pi: usize, b: &BBox) -> String {
        let mut words: Vec<&super::layout::Word> = self.pages[pi]
            .lines
            .iter()
            .flat_map(|l| l.words.iter())
            .filter(|w| mostly_inside(&w.bbox, b))
            .collect();
        words.sort_by(|x, y| x.bbox.center_y().partial_cmp(&y.bbox.center_y()).unwrap());
        let mut rows: Vec<Vec<&super::layout::Word>> = vec![];
        for w in words {
            match rows.last_mut() {
                Some(r)
                    if (w.bbox.center_y() - r[0].bbox.center_y()).abs()
                        < 0.4 * r[0].bbox.height().max(w.bbox.height()) =>
                {
                    r.push(w)
                }
                _ => rows.push(vec![w]),
            }
        }
        let mut out = vec![];
        for mut r in rows {
            r.sort_by(|x, y| x.bbox.x0.partial_cmp(&y.bbox.x0).unwrap());
            let h = r.iter().map(|w| w.bbox.height()).fold(0.0, f64::max);
            let mut s = String::new();
            for (k, w) in r.iter().enumerate() {
                if k > 0 {
                    s.push(if w.bbox.x0 - r[k - 1].bbox.x1 > 0.8 * h {
                        '\t'
                    } else {
                        ' '
                    });
                }
                s.push_str(&w.text);
            }
            out.push(s);
        }
        out.join("\n")
    }

    /// Mark lines inside a region.
    pub fn claim(&mut self, pi: usize, b: &BBox) {
        for (li, l) in self.pages[pi].lines.iter().enumerate() {
            if self.roles[pi][li] != Role::Caption && mostly_inside(&l.bbox, b) {
                self.roles[pi][li] = Role::InAsset;
            }
        }
    }

    /// Numbered display equations: `(n)` at the right edge of a column.
    pub fn equations(&self, regions: &[(usize, BBox)]) -> Vec<EqHit> {
        let mut out = vec![];
        let h = self.h();
        for (pi, p) in self.pages.iter().enumerate() {
            let in_refs = |_: usize| false;
            // tag candidates
            let mut tags: Vec<(usize, String, BBox)> = vec![];
            for (li, l) in p.lines.iter().enumerate() {
                if matches!(
                    self.roles[pi][li],
                    Role::Body | Role::Furniture | Role::Caption | Role::Heading | Role::InAsset
                ) || in_refs(li)
                {
                    continue;
                }
                let Some(last) = l.words.last() else { continue };
                let Some(c) = EQ_TAG.captures(&last.text) else {
                    continue;
                };
                if regions
                    .iter()
                    .any(|(rp, b)| *rp == pi && b.contains(&last.bbox))
                {
                    continue;
                }
                let alone = l.words.len() == 1;
                let gap = if l.words.len() >= 2 {
                    last.bbox.x0 - l.words[l.words.len() - 2].bbox.x1
                } else {
                    f64::MAX
                };
                let (_, cx1) = self.cols.span_of(&last.bbox);
                let col = self.column_for_x(last.bbox.center_x());
                let col_x1 = self.cols.cols[col].1.max(cx1.min(self.cols.cols[col].1));
                let at_edge =
                    last.bbox.x1 >= col_x1 - 0.1 * (self.cols.cols[col].1 - self.cols.cols[col].0);
                if at_edge && (alone || gap >= 1.0 * h) {
                    tags.push((li, c[1].to_string(), last.bbox));
                }
            }
            // group tags by the free slot between stoppers in their column
            for (k, (li, num, tb)) in tags.iter().enumerate() {
                let col = self.cols.cols[self.column_for_x(tb.center_x())];
                let probe = BBox::new(col.0, 0.0, col.1, p.height);
                let mut lo = 0.0f64;
                let mut hi = p.height;
                let cy = tb.center_y();
                let mut bounds: Vec<BBox> = p
                    .lines
                    .iter()
                    .enumerate()
                    .filter(|(lj, _)| {
                        *lj != *li
                            && matches!(
                                self.roles[pi][*lj],
                                Role::Body | Role::Furniture | Role::Heading | Role::Caption
                            )
                    })
                    .map(|(_, l)| l.bbox)
                    .collect();
                bounds.extend(regions.iter().filter(|(rp, _)| *rp == pi).map(|(_, b)| *b));
                for b in &bounds {
                    if b.x_overlap(&probe) < 10.0 {
                        continue;
                    }
                    if b.y1 <= cy && b.y1 > lo && !(b.y0 < tb.y0 && b.y1 > tb.y1) {
                        lo = b.y1;
                    }
                    if b.y0 >= cy && b.y0 < hi {
                        hi = b.y0;
                    }
                }
                // neighbouring tags in the same slot split it at midpoints
                for (k2, (_, _, ob)) in tags.iter().enumerate() {
                    if k2 == k
                        || (self.column_for_x(ob.center_x()) != self.column_for_x(tb.center_x()))
                    {
                        continue;
                    }
                    let oy = ob.center_y();
                    // split between two tags at the widest blank gap (else the midpoint)
                    let cut = |a: f64, b: f64| -> f64 {
                        let mid = (a + b) / 2.0;
                        let Some(ink) = self.inks.get(pi).and_then(|i| i.as_ref()) else {
                            return mid;
                        };
                        let bands =
                            ink.bands(&BBox::new(col.0 - 2.0, a, tb.x0.min(ob.x0) - 2.0, b), 0.0);
                        bands
                            .windows(2)
                            .max_by(|x, y| {
                                (x[1].0 - x[0].1).partial_cmp(&(y[1].0 - y[0].1)).unwrap()
                            })
                            .filter(|w| w[1].0 - w[0].1 >= 1.0)
                            .map(|w| (w[0].1 + w[1].0) / 2.0)
                            .unwrap_or(mid)
                    };
                    if oy < cy && oy > lo {
                        lo = lo.max(cut(oy, cy));
                    }
                    if oy > cy && oy < hi {
                        hi = hi.min(cut(cy, oy));
                    }
                }
                // keep the slot near the tag (at most ~12 lines each way)
                lo = lo.max(cy - 12.0 * h);
                hi = hi.min(cy + 12.0 * h);
                let region = BBox::new(col.0 - 2.0, lo + 0.3, tb.x0 - 2.0, hi - 0.3);
                let mut masks: Vec<BBox> = vec![];
                // running text: a text-like line, or a justified full-width line with a few words
                let text_like = |l: &Line| {
                    is_text_line(l)
                        || (l.text().trim_end().ends_with(':') && l.words.len() <= 4)
                        || (l.bbox.width() >= 0.9 * (col.1 - col.0)
                            && (l.bbox.x0 - col.0).abs() < 3.0
                            && l.max_gap() < h
                            && l.words
                                .iter()
                                .filter(|w| {
                                    w.text.chars().filter(|c| c.is_alphabetic()).count() >= 3
                                })
                                .count()
                                >= 3)
                };
                let bbox = match self.inks.get(pi).and_then(|i| i.as_ref()) {
                    Some(ink) => {
                        let bands = ink.bands(&region, 0.25 * h);
                        // bands that are plain left-aligned prose lines are not math
                        let mut keep: Vec<(f64, f64)> = vec![];
                        for (mut y0, mut y1) in bands {
                            // a text line glued to the top/bottom of a band ("as the loss:") is cut off
                            let mut ls: Vec<&Line> = p
                                .lines
                                .iter()
                                .filter(|l| {
                                    l.bbox
                                        .intersect(&BBox::new(region.x0, y0, region.x1, y1))
                                        .is_some()
                                        && l.bbox.x0 < tb.x0
                                })
                                .collect();
                            ls.sort_by(|a, b| a.bbox.y0.partial_cmp(&b.bbox.y0).unwrap());
                            // peel text lines off the top, then the bottom
                            while ls.len() >= 2 {
                                let first = ls[0];
                                if !(text_like(first)
                                    && ls[1..]
                                        .iter()
                                        .all(|o| o.bbox.y0 >= first.bbox.y0 + 0.3 * first.height()))
                                {
                                    break;
                                }
                                let below =
                                    ls[1..].iter().map(|o| o.bbox.y0).fold(f64::MAX, f64::min);
                                if below < first.bbox.y1 - 0.5 {
                                    // tall math overlaps the line's rows: keep them, paint the words white
                                    masks.push(first.bbox.pad(0.5));
                                    y0 = y0.max(below);
                                } else {
                                    y0 = y0.max(first.bbox.y1);
                                }
                                ls.remove(0);
                            }
                            while ls.len() >= 2 {
                                let last = ls[ls.len() - 1];
                                if !(text_like(last)
                                    && ls[..ls.len() - 1]
                                        .iter()
                                        .all(|o| o.bbox.y1 <= last.bbox.y0 + 1.0))
                                {
                                    break;
                                }
                                y1 = y1.min(last.bbox.y0);
                                ls.pop();
                            }
                            if y1 - y0 < 1.0 {
                                continue;
                            }
                            let band = BBox::new(region.x0, y0, region.x1, y1);
                            let lines: Vec<&Line> = p
                                .lines
                                .iter()
                                .filter(|l| {
                                    l.bbox
                                        .intersect(&band)
                                        .is_some_and(|i| i.height() > 0.3 * l.height())
                                        && l.bbox.x0 < tb.x0
                                        && !masks.iter().any(|m| m.contains(&l.bbox))
                                })
                                .collect();
                            let widest = lines.iter().max_by(|a, b| {
                                a.bbox.width().partial_cmp(&b.bbox.width()).unwrap()
                            });
                            let prose = !lines.is_empty()
                                && (lines.iter().all(|l| text_like(l))
                                    || lines
                                        .iter()
                                        .any(|l| (l.bbox.x0 - col.0).abs() < 4.0 && text_like(l))
                                    || widest.is_some_and(|l| {
                                        text_like(l) && l.bbox.width() > 0.5 * (col.1 - col.0)
                                    }));
                            if std::env::var_os("LORE_DOCSRC_DEBUG").is_some() {
                                eprintln!(
                                    "  band {y0:.1}..{y1:.1} prose={prose} lines={:?}",
                                    lines.iter().map(|l| l.text()).collect::<Vec<_>>()
                                );
                            }
                            if !prose {
                                keep.push((y0, y1));
                            }
                        }
                        // contiguous bands around the tag's row
                        let mut sel: Vec<(f64, f64)> = vec![];
                        if let Some(center) = keep
                            .iter()
                            .position(|(a, b)| *a <= tb.y1 + 2.0 && *b >= tb.y0 - 2.0)
                            .or_else(|| {
                                keep.iter()
                                    .enumerate()
                                    .min_by(|a, b| {
                                        let da = ((a.1.0 + a.1.1) / 2.0 - cy).abs();
                                        let db = ((b.1.0 + b.1.1) / 2.0 - cy).abs();
                                        da.partial_cmp(&db).unwrap()
                                    })
                                    .map(|x| x.0)
                            })
                        {
                            sel.push(keep[center]);
                            let mut i = center;
                            while i > 0 && keep[i].0 - keep[i - 1].1 < 0.9 * h {
                                i -= 1;
                                sel.push(keep[i]);
                            }
                            let mut i = center;
                            while i + 1 < keep.len() && keep[i + 1].0 - keep[i].1 < 0.9 * h {
                                i += 1;
                                sel.push(keep[i]);
                            }
                        }
                        let y0 = sel.iter().map(|x| x.0).fold(f64::MAX, f64::min);
                        let y1 = sel.iter().map(|x| x.1).fold(f64::MIN, f64::max);
                        if sel.is_empty() {
                            None
                        } else {
                            ink.bbox_in(&BBox::new(region.x0, y0, region.x1, y1), 1)
                        }
                    }
                    None => p
                        .lines
                        .iter()
                        .filter(|l| {
                            l.bbox.intersect(&region).is_some()
                                && (l.bbox.center_y() - cy).abs() < 1.5 * h
                        })
                        .flat_map(|l| l.words.iter())
                        .filter(|w| w.bbox.x1 <= tb.x0)
                        .map(|w| w.bbox)
                        .reduce(|a, b| a.union(&b)),
                };
                if std::env::var_os("LORE_DOCSRC_DEBUG").is_some() {
                    eprintln!(
                        "eq tag ({num}) p{} slot {lo:.1}..{hi:.1} region {region:?} -> {bbox:?}",
                        pi + 1
                    );
                }
                let Some(bbox) = bbox else { continue };
                let conf = if bbox.height() > 5.0 * h { 0.7 } else { 0.85 };
                let bbox = bbox.pad(1.0);
                masks.retain(|m| m.intersect(&bbox).is_some());
                let mut text = self.text_in(pi, &bbox).replace(['\t', '\n'], " ");
                for m in &masks {
                    for w in p
                        .lines
                        .iter()
                        .flat_map(|l| l.words.iter())
                        .filter(|w| mostly_inside(&w.bbox, m))
                    {
                        text = text.replacen(&w.text, "", 1);
                    }
                }
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                out.push(EqHit {
                    page: pi,
                    number: num.clone(),
                    bbox,
                    tag: *tb,
                    text,
                    confidence: conf,
                    masks,
                });
            }
        }
        out
    }

    fn column_for_x(&self, x: f64) -> usize {
        match self.cols.gutter_mid() {
            Some(g) if x >= g => 1,
            _ => 0,
        }
    }
}

/// A line of running text next to a display equation ("where …", "as:").
pub fn is_text_line(l: &Line) -> bool {
    let wordy = l
        .words
        .iter()
        .filter(|w| {
            let t = w.text.trim_matches(|c: char| !c.is_alphanumeric());
            let letters = t.chars().filter(|c| c.is_alphabetic()).count();
            letters >= 2 && letters + t.matches('-').count() == t.chars().count()
        })
        .count();
    let ratio = wordy as f64 / l.words.len().max(1) as f64;
    let first = l
        .words
        .first()
        .map(|w| {
            w.text
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        })
        .unwrap_or_default();
    let first_lower =
        first.chars().count() >= 2 && first.chars().all(|c| c.is_lowercase() || c == '-');
    const CONNECTIVES: &[&str] = &[
        "where", "with", "and", "as", "for", "which", "here", "then", "subject", "since", "while",
        "when", "if", "given", "s.t", "so", "thus", "hence",
    ];
    let connective = first_lower && CONNECTIVES.contains(&first.as_str()) && l.words.len() <= 6;
    connective
        || (ratio >= 0.5 && (first_lower || wordy >= 3))
        || (first_lower && ratio >= 0.3 && wordy >= 3)
}

/// pdftotext sometimes joins two columns' lines that share a baseline (and
/// their blocks). Split such lines at the gutter; right-hand parts get new blocks.
pub fn split_gutter(p: &TextPage, cols: &Columns) -> TextPage {
    let Some(g) = cols.gutter_mid() else {
        return p.clone();
    };
    let gap_min = (cols.cols[1].0 - cols.cols[0].1).max(4.0) * 0.6;
    let mut out = TextPage {
        lines: vec![],
        blocks: p.blocks.clone(),
        ..p.clone()
    };
    let base = p.blocks.len();
    let mut used_right = std::collections::BTreeSet::new();
    for l in &p.lines {
        let cut = l.words.windows(2).position(|w| {
            w[0].bbox.x1 <= g + 1.0
                && w[1].bbox.x0 >= g - 1.0
                && w[1].bbox.x0 - w[0].bbox.x1 >= gap_min
        });
        match cut {
            Some(k)
                if l.words[..=k].iter().all(|w| w.bbox.x1 <= g + 1.0)
                    && l.words[k + 1..].iter().all(|w| w.bbox.x0 >= g - 1.0) =>
            {
                let (a, b) = l.words.split_at(k + 1);
                let bb = |ws: &[Word]| {
                    ws.iter()
                        .map(|w| w.bbox)
                        .reduce(|x, y| x.union(&y))
                        .unwrap()
                };
                out.lines.push(Line {
                    bbox: bb(a),
                    words: a.to_vec(),
                    block: l.block,
                });
                out.lines.push(Line {
                    bbox: bb(b),
                    words: b.to_vec(),
                    block: base + l.block,
                });
                used_right.insert(l.block);
            }
            _ => out.lines.push(l.clone()),
        }
    }
    // blocks holding lines of both columns (pdftotext joins paragraphs that
    // share baselines): right-column lines move to a new block
    for b in 0..base {
        let ls: Vec<&Line> = out.lines.iter().filter(|l| l.block == b).collect();
        let left = ls.iter().any(|l| cols.column_of(&l.bbox) == Some(0));
        let right = ls.iter().any(|l| cols.column_of(&l.bbox) == Some(1));
        if left && right {
            used_right.insert(b);
        }
    }
    if used_right.is_empty() {
        return p.clone();
    }
    for l in out.lines.iter_mut() {
        if used_right.contains(&l.block) && l.bbox.x0 >= g - 1.0 && l.block < base {
            l.block += base;
        }
    }
    // keep lines of one block together (stable by block, then original order)
    let mut idx: Vec<usize> = (0..out.lines.len()).collect();
    idx.sort_by_key(|&i| (out.lines[i].block, i));
    out.lines = idx.into_iter().map(|i| out.lines[i].clone()).collect();
    out.blocks.resize(base * 2, BBox::new(0.0, 0.0, 0.0, 0.0));
    for b in 0..out.blocks.len() {
        if let Some(u) = out
            .lines
            .iter()
            .filter(|l| l.block == b)
            .map(|l| l.bbox)
            .reduce(|x, y| x.union(&y))
        {
            out.blocks[b] = u;
        }
    }
    out
}

/// Line indices in reading order: blocks that cross the gutter cut the page
/// into bands; within a band the left column is read before the right one.
pub fn reading_order(p: &TextPage, cols: &Columns) -> Vec<usize> {
    let nb = p.blocks.len();
    let mut lines_of: Vec<Vec<usize>> = vec![vec![]; nb.max(1)];
    for (li, l) in p.lines.iter().enumerate() {
        lines_of[l.block.min(nb.saturating_sub(1))].push(li);
    }
    let bbox_of = |b: usize| {
        lines_of[b]
            .iter()
            .map(|&li| p.lines[li].bbox)
            .reduce(|a, c| a.union(&c))
    };
    let mut spanning: Vec<f64> = vec![];
    if cols.two() {
        for b in 0..lines_of.len() {
            if let Some(bb) = bbox_of(b)
                && cols.spans(&bb)
            {
                spanning.push(bb.y0);
            }
        }
    }
    spanning.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut keyed: Vec<((usize, usize, i64, i64), usize)> = vec![];
    for b in 0..lines_of.len() {
        let Some(bb) = bbox_of(b) else { continue };
        let key = if cols.two() && cols.spans(&bb) {
            let j = spanning
                .iter()
                .position(|y| *y >= bb.y0 - 0.01)
                .unwrap_or(0);
            (2 * j + 1, 0, (bb.y0 * 10.0) as i64, (bb.x0 * 10.0) as i64)
        } else {
            let seg = spanning.iter().filter(|y| **y <= bb.y0).count();
            (
                2 * seg,
                cols.column_of(&bb).unwrap_or(0),
                (bb.y0 * 10.0) as i64,
                (bb.x0 * 10.0) as i64,
            )
        };
        keyed.push((key, b));
    }
    keyed.sort();
    keyed
        .into_iter()
        .flat_map(|(_, b)| lines_of[b].clone())
        .collect()
}

/// At least half of `a`'s area lies inside `b` (glyph ink is tighter than word boxes).
pub fn mostly_inside(a: &BBox, b: &BBox) -> bool {
    a.intersect(&b.pad(1.0))
        .is_some_and(|i| i.area() >= 0.5 * a.area().max(0.01))
}

/// Split a reference list into entries: `[n]` starts, `n.` starts, or
/// hanging indentation (first line flush with the column, rest indented).
pub fn split_references(lines: &[(Line, f64)]) -> Vec<String> {
    if lines.is_empty() {
        return vec![];
    }
    let bracket = lines
        .iter()
        .filter(|(l, _)| BRACKET_REF.is_match(&l.text()))
        .count();
    let numbered = lines
        .iter()
        .filter(|(l, _)| Regex::new(r"^\d{1,3}\.\s").unwrap().is_match(&l.text()))
        .count();
    let mut starts: Vec<bool> = if bracket >= 3 {
        lines
            .iter()
            .map(|(l, _)| BRACKET_REF.is_match(&l.text()))
            .collect()
    } else if numbered * 4 >= lines.len() && numbered >= 3 {
        let re = Regex::new(r"^\d{1,3}\.\s").unwrap();
        lines.iter().map(|(l, _)| re.is_match(&l.text())).collect()
    } else {
        // hanging indent relative to the column's left edge
        lines
            .iter()
            .map(|(l, col_x0)| l.bbox.x0 - col_x0 < 3.0)
            .collect()
    };
    starts[0] = true;
    if starts.iter().all(|s| *s) && bracket < 3 {
        // no indentation to go by: fall back to one entry per line group ending with a period
        starts = vec![false; lines.len()];
        starts[0] = true;
        for i in 1..lines.len() {
            starts[i] = lines[i - 1].0.text().trim_end().ends_with('.');
        }
    }
    let mut out: Vec<Vec<&Line>> = vec![];
    for (i, (l, _)) in lines.iter().enumerate() {
        if starts[i] || out.is_empty() {
            out.push(vec![l]);
        } else {
            out.last_mut().unwrap().push(l);
        }
    }
    out.into_iter()
        .map(|ls| join_lines(ls.into_iter()))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A line of words starting at x, each `ww` wide with `gap` between.
    pub fn line(text: &str, x: f64, y: f64, gap: f64, block: usize) -> Line {
        let mut words = vec![];
        let mut cx = x;
        for t in text.split(' ') {
            let w = t.chars().count() as f64 * 4.5;
            words.push(Word {
                bbox: BBox::new(cx, y, cx + w, y + 9.0),
                text: t.to_string(),
            });
            cx += w + gap;
        }
        let bbox = words
            .iter()
            .map(|w| w.bbox)
            .reduce(|a, b| a.union(&b))
            .unwrap();
        Line { bbox, words, block }
    }

    pub const PROSE: &str = "the proposed method aligns visual features with report knowledge.";

    /// One-column 600×800 page: prose, a figure (ink only) with a caption
    /// below, a numbered equation, more prose, a table caption + table text.
    pub fn synthetic() -> (Vec<TextPage>, Vec<Option<Ink>>) {
        let mut lines = vec![];
        let mut blocks = vec![];
        let add = |ls: Vec<Line>, blocks: &mut Vec<BBox>, lines: &mut Vec<Line>| {
            let b = ls
                .iter()
                .map(|l| l.bbox)
                .reduce(|a, b| a.union(&b))
                .unwrap();
            blocks.push(b);
            lines.extend(ls);
        };
        add(
            vec![line("1. Introduction", 60.0, 80.0, 3.0, 0)],
            &mut blocks,
            &mut lines,
        );
        add(
            (0..3)
                .map(|k| line(PROSE, 60.0, 100.0 + 12.0 * k as f64, 3.0, 1))
                .collect(),
            &mut blocks,
            &mut lines,
        );
        // figure ink 60..400 × 150..300 (no words), caption at 310
        add(
            vec![line(
                "Fig. 1. Overview of the model with encoder",
                60.0,
                310.0,
                3.0,
                2,
            )],
            &mut blocks,
            &mut lines,
        );
        add(
            (0..2)
                .map(|k| line(PROSE, 60.0, 330.0 + 12.0 * k as f64, 3.0, 3))
                .collect(),
            &mut blocks,
            &mut lines,
        );
        // equation: "y = Wx + b" centered, tag (1) at right edge
        let mut eq = line("y = Wx + b", 180.0, 365.0, 3.0, 4);
        let tag = line("(1)", 335.0, 365.0, 3.0, 4);
        eq.words.extend(tag.words.clone());
        eq.bbox = eq.bbox.union(&tag.bbox);
        add(vec![eq], &mut blocks, &mut lines);
        add(
            (0..2)
                .map(|k| line(PROSE, 60.0, 390.0 + 12.0 * k as f64, 3.0, 5))
                .collect(),
            &mut blocks,
            &mut lines,
        );
        add(
            vec![
                line("Table 1", 60.0, 430.0, 3.0, 6),
                line("Results on the test set.", 60.0, 441.0, 3.0, 6),
            ],
            &mut blocks,
            &mut lines,
        );
        add(
            vec![
                line("Method BLEU ROUGE", 60.0, 460.0, 40.0, 7),
                line("Ours 0.31 0.42", 60.0, 472.0, 60.0, 7),
                line("Base 0.25 0.38", 60.0, 484.0, 60.0, 7),
            ],
            &mut blocks,
            &mut lines,
        );
        add(
            (0..2)
                .map(|k| line(PROSE, 60.0, 510.0 + 12.0 * k as f64, 3.0, 8))
                .collect(),
            &mut blocks,
            &mut lines,
        );
        add(
            vec![line("References", 60.0, 560.0, 3.0, 9)],
            &mut blocks,
            &mut lines,
        );
        add(
            vec![
                line("[1] A. One, Deep things, J., 2020.", 60.0, 575.0, 3.0, 10),
                line("[2] B. Two, Other things,", 60.0, 587.0, 3.0, 10),
                line("Conf., 2021.", 72.0, 599.0, 3.0, 10),
                line("[3] C. Three, More, 2022.", 60.0, 611.0, 3.0, 10),
            ],
            &mut blocks,
            &mut lines,
        );
        let page = TextPage {
            number: 1,
            width: 600.0,
            height: 800.0,
            lines: lines.clone(),
            blocks,
        };
        // ink: every word box + the figure drawing + a rule above/below the table
        let mut px = vec![255u8; 600 * 800];
        let mut fill = |b: BBox| {
            for y in b.y0 as usize..(b.y1 as usize).min(800) {
                for x in b.x0 as usize..(b.x1 as usize).min(600) {
                    px[y * 600 + x] = 0;
                }
            }
        };
        for l in &lines {
            for w in &l.words {
                fill(BBox::new(
                    w.bbox.x0,
                    w.bbox.y0 + 2.0,
                    w.bbox.x1,
                    w.bbox.y1 - 1.0,
                ));
            }
        }
        fill(BBox::new(80.0, 150.0, 330.0, 152.0));
        fill(BBox::new(80.0, 150.0, 82.0, 300.0));
        fill(BBox::new(328.0, 150.0, 330.0, 300.0));
        fill(BBox::new(80.0, 298.0, 330.0, 300.0));
        fill(BBox::new(60.0, 456.0, 330.0, 457.0));
        fill(BBox::new(60.0, 496.0, 330.0, 497.0));
        let ink = Ink::from_luma(600, 800, &px, 1.0, 200);
        (vec![page], vec![Some(ink)])
    }

    /// The Springer rule is opt-in: a document read the default way gives the headings it always gave.
    #[test]
    fn unnumbered_headings_are_off_by_default() {
        let (pages, inks) = synthetic();
        let d = Doc::new(&pages, inks);
        assert!(!d.unnumbered_headings);
        assert!(!super::super::PdfOptions::new(std::path::Path::new("/tmp")).unnumbered_headings);
    }

    #[test]
    fn classifies_and_finds_regions() {
        let (pages, inks) = synthetic();
        let mut d = Doc::new(&pages, inks);
        assert_eq!(d.cols.cols.len(), 1);
        let caps = d.captions();
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].kind, AssetKind::Figure);
        assert_eq!(caps[1].kind, AssetKind::Table);
        assert_eq!(caps[1].text, "Table 1 Results on the test set.");
        let heads = d.headings();
        let titles: Vec<&str> = heads.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, vec!["Introduction", "References"]);

        let (fb, _, _) = d.region_for(&caps[0], &caps, &[]).unwrap();
        assert!(
            (fb.y0 - 148.5).abs() < 1.0 && (fb.y1 - 301.5).abs() < 1.0,
            "{fb:?}"
        );
        assert!(
            (fb.x0 - 78.5).abs() < 1.0 && (fb.x1 - 331.5).abs() < 1.0,
            "{fb:?}"
        );
        let (tb, _, notes) = d.region_for(&caps[1], &caps, &[]).unwrap();
        assert!(notes.is_empty());
        assert!(
            tb.y0 > 450.0 && tb.y0 < 457.0 && tb.y1 > 496.0 && tb.y1 < 500.0,
            "{tb:?}"
        );
        assert_eq!(
            d.text_in(0, &tb),
            "Method\tBLEU\tROUGE\nOurs\t0.31\t0.42\nBase\t0.25\t0.38"
        );

        d.claim(0, &fb);
        d.claim(0, &tb);
        let eqs = d.equations(&[(0, fb), (0, tb)]);
        assert_eq!(eqs.len(), 1);
        let EqHit {
            number: num,
            bbox: eb,
            text,
            ..
        } = &eqs[0];
        assert_eq!(num, "1");
        assert!(
            eb.x0 > 175.0 && eb.x1 < 335.0 && eb.y0 > 360.0 && eb.y1 < 380.0,
            "{eb:?}"
        );
        assert_eq!(text, "y = Wx + b");
    }

    #[test]
    fn reference_splitting() {
        let (pages, _) = synthetic();
        let refs: Vec<(Line, f64)> = pages[0]
            .lines
            .iter()
            .filter(|l| l.block == 10)
            .map(|l| (l.clone(), 60.0))
            .collect();
        assert_eq!(
            split_references(&refs),
            vec![
                "[1] A. One, Deep things, J., 2020.",
                "[2] B. Two, Other things, Conf., 2021.",
                "[3] C. Three, More, 2022."
            ]
        );
        // hanging indent, author-year
        let hang = vec![
            (
                line("Anderson, P., Fernando, B., 2016.", 40.0, 0.0, 3.0, 0),
                40.0,
            ),
            (
                line("Spice: semantic evaluation.", 48.0, 10.0, 3.0, 0),
                40.0,
            ),
            (
                line("Brown, T., 2020. Language models.", 40.0, 20.0, 3.0, 0),
                40.0,
            ),
        ];
        assert_eq!(
            split_references(&hang),
            vec![
                "Anderson, P., Fernando, B., 2016. Spice: semantic evaluation.",
                "Brown, T., 2020. Language models."
            ]
        );
    }

    /// Word's math words in a caption (synthetic): `𝐶` + lowered smaller `𝑒𝑛𝑐` is a
    /// subscripted symbol, a combining tilde as its own word goes on the next letter,
    /// trailing punctuation sticks.
    #[test]
    fn caption_math_words() {
        let w = |t: &str, x0: f64, y0: f64, x1: f64, y1: f64| Word { bbox: BBox::new(x0, y0, x1, y1), text: t.to_string() };
        let words = vec![
            w("The", 300.0, 340.0, 320.0, 352.0),
            w("classifier,", 323.0, 340.0, 375.0, 352.0),
            w("\u{1D436}", 430.0, 339.0, 437.0, 352.0),
            w("\u{1D452}\u{1D45B}\u{1D450}", 437.0, 344.0, 449.0, 354.0),
            w(",", 452.0, 340.0, 455.0, 352.0),
            w("patch", 287.2, 289.1, 311.0, 299.1),
            w("\u{303}", 315.5, 287.3, 322.3, 299.3),
            w("\u{1D501}", 315.4, 288.1, 322.5, 299.1),
            w("\u{1D491}", 322.5, 292.8, 327.5, 300.9),
            w("\u{1D45B}.", 330.0, 289.1, 340.0, 299.1),
        ];
        let bbox = words.iter().map(|w| w.bbox).reduce(|a, b| a.union(&b)).unwrap();
        let l = Line { bbox, words, block: 0 };
        assert_eq!(line_text_math(&l), "The classifier, $C_{\\mathrm{enc}}$, patch $\\tilde{\\boldsymbol{x}}_{\\boldsymbol{p}}$ $n$.");
        // the tilde ends a line, its letter starts the next
        let a = Line { bbox: BBox::new(0.0, 0.0, 60.0, 12.0), words: vec![w("a", 0.0, 0.0, 6.0, 12.0), w("patch", 9.0, 0.0, 40.0, 12.0), w("\u{303}", 44.0, -1.0, 50.0, 12.0)], block: 0 };
        let b = Line { bbox: BBox::new(0.0, 14.0, 60.0, 27.0), words: vec![w("\u{1D501}", 0.0, 14.0, 7.0, 26.0), w("\u{1D491}.", 7.0, 20.0, 15.0, 29.0)], block: 0 };
        assert_eq!(join_lines_math([a, b].iter()), "a patch $\\tilde{\\boldsymbol{x}}_{\\boldsymbol{p}}$.");
    }

    #[test]
    fn dehyphenation() {
        let a = line("knowledge en-", 0.0, 0.0, 3.0, 0);
        let b = line("hanced report", 0.0, 10.0, 3.0, 0);
        assert_eq!(join_lines([a, b].iter()), "knowledge enhanced report");
    }
}
