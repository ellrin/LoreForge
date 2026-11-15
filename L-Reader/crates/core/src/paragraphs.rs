//! Track B, 「讀懂段落」: the paper as paragraphs with section headings, each with its
//! "stuck" chips (no-card terms from the noun list, symbols and display equations from the
//! symbol table), and the per-paragraph analysis a model fills in (`lr-paragraph`): the
//! paragraph's role, equation "translations" (inputs, outputs, target, effect) and the
//! step's motive for method paragraphs, and a metric / comparison / values / difference /
//! test / significance row for result paragraphs. **No Chinese summary or translation of
//! the paragraph** — only where the reader may get stuck.
//!
//! Splitting uses the pdf.js lines (an item that ends a line): a heading (bold or larger
//! than the body, short, often numbered) starts a section; a vertical gap, a first-line
//! indent, or a short line ending a sentence starts a paragraph; display equations stay in
//! the paragraph around them; a paragraph that does not end a sentence continues on the
//! next page. Without items (plain text) blank lines and heading-like lines are used.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::nouns::Inventory;
use crate::symbols::SymbolFile;
use crate::text::{PaperText, body_end};

pub const PARAGRAPHS_SCHEMA: &str = "lorereader.paragraphs/1";
pub const ROLES: [&str; 6] = ["background", "gap", "method", "result", "discussion", "other"];

/// One place of a paragraph's text on one page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub page: u32,
    /// UTF-16 offsets in that page's text.
    pub s: usize,
    pub e: usize,
    /// `[x0, y0, x1, y1]` page fractions, top-left origin (0s without geometry).
    pub bbox: [f32; 4],
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chips {
    /// Noun-list keys of the no-card specialist terms in the paragraph.
    pub terms: Vec<String>,
    /// Symbol-table ids.
    pub symbols: Vec<String>,
    /// Display-equation ids.
    pub equations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    /// Stable across re-runs: a hash of the text (with a suffix for repeats).
    pub id: String,
    pub index: usize,
    pub section: String,
    /// front | abstract | intro | method | result | discussion | other — from the heading
    pub section_kind: String,
    pub ranges: Vec<Range>,
    pub text: String,
    pub words: usize,
    #[serde(default)]
    pub chips: Chips,
}

impl Paragraph {
    pub fn page(&self) -> u32 {
        self.ranges.first().map(|r| r.page).unwrap_or(1)
    }
}

/// What kind of section a heading opens.
pub fn section_kind(heading: &str) -> &'static str {
    let h = heading.to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| h.contains(w));
    if has(&["abstract"]) {
        "abstract"
    } else if has(&["introduction", "background", "related work", "motivation", "preliminar", "prior work", "literature"]) {
        "intro"
    } else if has(&["method", "approach", "model", "framework", "algorithm", "proposed", "architecture", "implementation", "training", "setup", "materials", "formulation", "theory", "analysis of"]) {
        "method"
    } else if has(&["result", "experiment", "evaluation", "ablation", "performance", "comparison", "empirical", "benchmark"]) {
        "result"
    } else if has(&["discussion", "conclusion", "limitation", "future", "summary"]) {
        "discussion"
    } else {
        "other"
    }
}

fn heading_like(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > 90 || t.ends_with(['.', ',', ';']) && !t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    let numbered = regex::Regex::new(r"^(?:\d+(?:\.\d+)*\.?|[IVX]+\.|[A-Z]\.)\s+[A-Z]").unwrap().is_match(t);
    let known = matches!(t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ').to_lowercase().as_str(), "abstract" | "introduction" | "related work" | "background" | "method" | "methods" | "results" | "discussion" | "conclusion" | "conclusions" | "experiments" | "references" | "acknowledgments" | "acknowledgements" | "appendix");
    numbered && t.split_whitespace().count() <= 12 || known
}

struct Line {
    page: usize,
    s: usize,
    e: usize,
    text: String,
    x0: f32,
    x1: f32,
    y: f32,
    size: f32,
    bold: bool,
}

fn lines_of(paper: &PaperText, end: Option<(usize, usize)>) -> Vec<Line> {
    let mut out = vec![];
    for (pi, p) in paper.pages.iter().enumerate() {
        let limit = match end {
            Some((ep, u)) if ep == pi => u,
            Some((ep, _)) if pi > ep => 0,
            _ => usize::MAX,
        };
        let u: Vec<u16> = p.text.encode_utf16().collect();
        let slice = |s: usize, e: usize| String::from_utf16_lossy(&u[s.min(u.len())..e.min(u.len())]);
        if p.items.is_empty() {
            // plain text: one line per text line
            let mut s = 0;
            for l in p.text.split('\n') {
                let n = l.encode_utf16().count();
                if s < limit {
                    out.push(Line { page: pi, s, e: s + n, text: l.to_string(), x0: 0.0, x1: 0.0, y: 0.0, size: 0.0, bold: false });
                }
                s += n + 1;
            }
            continue;
        }
        let mut cur: Vec<usize> = vec![];
        let flush = |cur: &mut Vec<usize>, out: &mut Vec<Line>| {
            let items: Vec<&crate::text::Item> = cur.iter().map(|&i| &p.items[i]).filter(|it| !it.str.trim().is_empty()).collect();
            if let (Some(f), Some(l)) = (items.first(), items.last()) {
                let (s, e) = (f.s, l.e);
                if s < limit {
                    let chars: usize = items.iter().map(|i| i.str.chars().count()).sum();
                    let bold_chars: usize = items.iter().filter(|i| i.bold || i.font.to_uppercase().contains("BOLD") || crate::text::base_font(&i.font).to_uppercase().starts_with("CMBX") || crate::text::base_font(&i.font).to_uppercase().ends_with("-BD")).map(|i| i.str.chars().count()).sum();
                    let size = items.iter().map(|i| (i.size, i.str.chars().count())).max_by_key(|x| x.1).map(|x| x.0).unwrap_or(0.0);
                    out.push(Line {
                        page: pi,
                        s,
                        e: e.min(limit),
                        text: slice(s, e.min(limit)),
                        x0: items.iter().map(|i| i.x).fold(f32::MAX, f32::min),
                        x1: items.iter().map(|i| i.x + i.w).fold(f32::MIN, f32::max),
                        y: f.y,
                        size,
                        bold: bold_chars * 2 > chars,
                    });
                }
            }
            cur.clear();
        };
        for (i, it) in p.items.iter().enumerate() {
            cur.push(i);
            if it.eol {
                flush(&mut cur, &mut out);
            }
        }
        flush(&mut cur, &mut out);
    }
    out
}

fn ends_sentence(t: &str) -> bool {
    let t = t.trim_end();
    t.ends_with(['.', '?', '!', ':']) || t.ends_with(".)") || t.ends_with(".”")
}

/// Split the body (up to the references) into paragraphs.
pub fn split(paper: &PaperText) -> Vec<Paragraph> {
    split_with(paper, &[])
}

/// [`split`] knowing the display equations (`(page, start, end, label)`): their lines stay
/// in the paragraph around them, read as ［式 (n)］, and the line after one continues it.
pub fn split_with(paper: &PaperText, equations: &[(u32, usize, usize, String)]) -> Vec<Paragraph> {
    let end = body_end(paper);
    let mut lines = lines_of(paper, end);
    // equation lines: one placeholder line per equation
    let eq_of = |l: &Line| equations.iter().position(|q| q.0 == paper.pages[l.page].page.max(1) && l.s < q.2 && q.1 < l.e && (l.e.min(q.2) - l.s.max(q.1)) * 2 >= (l.e - l.s).max(1));
    let mut is_eq = vec![false; lines.len()];
    let mut last_eq: Option<usize> = None;
    let mut keep = vec![true; lines.len()];
    for i in 0..lines.len() {
        if let Some(q) = eq_of(&lines[i]) {
            is_eq[i] = true;
            if last_eq == Some(q) {
                keep[i] = false;
                continue;
            }
            last_eq = Some(q);
            let label = &equations[q].3;
            lines[i].text = if label.is_empty() { "［式］".into() } else { format!("［式 {label}］") };
        } else {
            last_eq = None;
        }
    }
    let lines: Vec<Line> = lines.into_iter().zip(keep.iter()).filter(|(_, k)| **k).map(|(l, _)| l).collect();
    let is_eq: Vec<bool> = is_eq.into_iter().zip(keep.iter()).filter(|(_, k)| **k).map(|(e, _)| e).collect();
    let geometry = paper.pages.iter().any(|p| !p.items.is_empty());
    // body size and line spacing
    let mut sizes: BTreeMap<i32, usize> = BTreeMap::new();
    for l in &lines {
        *sizes.entry((l.size * 2.0).round() as i32).or_default() += l.text.chars().count();
    }
    let body = sizes.iter().max_by_key(|(_, n)| **n).map(|(k, _)| *k as f32 / 2.0).unwrap_or(10.0);
    let mut gaps: Vec<f32> = lines.windows(2).filter(|w| w[0].page == w[1].page && w[0].y > w[1].y).map(|w| w[0].y - w[1].y).filter(|g| *g > body * 0.8 && *g < body * 3.0).collect();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let spacing = gaps.get(gaps.len() / 3).copied().unwrap_or(body * 1.2);
    // column left edges per page (most common x0, rounded)
    let left_of = |pi: usize, x: f32| -> f32 {
        let mut c: BTreeMap<i32, usize> = BTreeMap::new();
        for l in lines.iter().filter(|l| l.page == pi && (l.x0 - x).abs() < 200.0 && (l.x0 < x + 1.0)) {
            *c.entry(l.x0.round() as i32).or_default() += 1;
        }
        c.iter().max_by_key(|(_, n)| **n).map(|(k, _)| *k as f32).unwrap_or(x)
    };
    let mut paras: Vec<Paragraph> = vec![];
    let mut section = String::new();
    let mut kind = "front".to_string();
    let mut cur: Vec<usize> = vec![];
    let mut seen_heading = false;
    let push = |cur: &mut Vec<usize>, paras: &mut Vec<Paragraph>, section: &str, kind: &str| {
        if cur.is_empty() {
            return;
        }
        let mut ranges: Vec<Range> = vec![];
        let mut text = String::new();
        for &i in cur.iter() {
            let l = &lines[i];
            let p = &paper.pages[l.page];
            let (w, h) = (p.width.max(1.0), p.height.max(1.0));
            let bb = if geometry { [l.x0 / w, 1.0 - (l.y + l.size) / h, l.x1 / w, 1.0 - (l.y - l.size * 0.3) / h] } else { [0.0; 4] };
            match ranges.last_mut() {
                Some(r) if r.page == p.page.max(1) && geometry => {
                    r.e = r.e.max(l.e);
                    r.bbox = [r.bbox[0].min(bb[0]), r.bbox[1].min(bb[1]), r.bbox[2].max(bb[2]), r.bbox[3].max(bb[3])];
                }
                Some(r) if r.page == p.page.max(1) => r.e = r.e.max(l.e),
                _ => ranges.push(Range { page: p.page.max(1), s: l.s, e: l.e, bbox: bb.map(|x| x.clamp(0.0, 1.0)) }),
            }
            let t = l.text.trim();
            if text.ends_with('-') && t.chars().next().is_some_and(char::is_lowercase) {
                text.pop();
            } else if !text.is_empty() {
                text.push(' ');
            }
            text += t;
        }
        let words = text.split_whitespace().count();
        // page numbers, running heads and stray symbols are not paragraphs
        if words == 0 || (words < 3 && text.chars().filter(|c| c.is_alphabetic()).count() < 12) {
            cur.clear();
            return;
        }
        paras.push(Paragraph { id: String::new(), index: paras.len(), section: section.to_string(), section_kind: kind.to_string(), ranges, text, words, chips: Chips::default() });
        cur.clear();
    };
    for i in 0..lines.len() {
        let l = &lines[i];
        let t = l.text.trim();
        if t.is_empty() {
            if !geometry {
                push(&mut cur, &mut paras, &section, &kind);
            }
            continue;
        }
        let is_heading = heading_like(t) && (!geometry || l.bold || l.size > body * 1.12) || (geometry && (l.bold || l.size > body * 1.15) && t.chars().count() <= 70 && t.split_whitespace().count() <= 10 && !ends_sentence(t) && cur.is_empty() && seen_heading);
        if is_heading {
            push(&mut cur, &mut paras, &section, &kind);
            section = t.to_string();
            kind = section_kind(t).to_string();
            seen_heading = true;
            continue;
        }
        if let Some(&prev) = cur.last() {
            let p = &lines[prev];
            let new_para = if is_eq[i] || is_eq[prev] || !geometry {
                false
            } else if p.page == l.page && p.y > l.y {
                let gap = p.y - l.y;
                let left = left_of(l.page, l.x0);
                let indent = l.x0 > left + body * 0.6 && l.x0 < left + body * 4.0 && ends_sentence(&p.text);
                let short_end = p.x1 < left_of(p.page, p.x0) + (l.x1 - left) * 0.75 && ends_sentence(&p.text) && l.x1 - left > 0.0;
                gap > spacing * 1.45 || indent || (short_end && gap > spacing * 0.9)
            } else {
                // a new column or page: continue unless the previous line ended a sentence
                // and this one is indented
                let left = left_of(l.page, l.x0);
                ends_sentence(&p.text) && l.x0 > left + body * 0.6 && l.x0 < left + body * 4.0
            };
            if new_para {
                push(&mut cur, &mut paras, &section, &kind);
            }
        }
        cur.push(i);
    }
    push(&mut cur, &mut paras, &section, &kind);
    // ids
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for p in &mut paras {
        let norm: String = p.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(400).collect();
        let h = hex::encode(&Sha256::digest(norm.as_bytes())[..6]);
        let n = seen.entry(h.clone()).or_default();
        *n += 1;
        p.id = if *n == 1 { format!("p{h}") } else { format!("p{h}-{n}") };
    }
    paras
}

/// Fill each paragraph's chips: red terms, symbols and display equations inside it.
pub fn attach_chips(paras: &mut [Paragraph], nouns: &Inventory, symbols: &SymbolFile) {
    let inside = |p: &Paragraph, page: u32, s: usize, e: usize| p.ranges.iter().any(|r| r.page == page && s >= r.s && e <= r.e.max(r.s));
    for p in paras.iter_mut() {
        let mut c = Chips::default();
        for n in nouns.entries.iter().filter(|n| n.red()) {
            if n.occ.iter().any(|o| inside(p, o[0], o[1] as usize, o[2] as usize)) {
                c.terms.push(n.key.clone());
            }
        }
        for e in &symbols.entries {
            if e.entry.tags.iter().any(|t| t == "conventional") {
                continue;
            }
            if e.occurrences.iter().any(|o| inside(p, o.page, o.s, o.e)) {
                c.symbols.push(e.entry.id.clone());
            }
        }
        for q in &symbols.equations {
            let overlaps = p.ranges.iter().any(|r| r.page == q.page && q.s < r.e && r.s < q.e);
            if overlaps {
                c.equations.push(q.entry.id.clone());
            }
        }
        p.chips = c;
    }
}

/// The order of the background run: method and result paragraphs first, then the rest;
/// very short paragraphs (captions, affiliations) are not analysed.
pub fn run_order(paras: &[Paragraph]) -> Vec<usize> {
    let rank = |p: &Paragraph| match p.section_kind.as_str() {
        "method" => 0,
        "result" => 1,
        "abstract" | "intro" => 2,
        "discussion" => 3,
        "front" => 5,
        _ => 4,
    };
    let mut idx: Vec<usize> = (0..paras.len()).filter(|&i| paras[i].words >= 15 && paras[i].section_kind != "front").collect();
    idx.sort_by_key(|&i| (rank(&paras[i]), i));
    idx
}

// ── the model's analysis ─────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EquationNote {
    /// "(3)" or the inline formula
    #[serde(default)]
    pub r#ref: String,
    #[serde(default)]
    pub inputs: String,
    #[serde(default)]
    pub outputs: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub effect: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResultRow {
    #[serde(default)]
    pub metric: String,
    #[serde(default)]
    pub comparison: String,
    #[serde(default)]
    pub values: String,
    #[serde(default)]
    pub difference: String,
    #[serde(default)]
    pub test: String,
    #[serde(default)]
    pub significance: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    pub role: String,
    #[serde(default)]
    pub equations: Vec<EquationNote>,
    #[serde(default)]
    pub motive: String,
    #[serde(default)]
    pub results: Vec<ResultRow>,
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub at: String,
}

pub const PARAGRAPH_SYSTEM: &str = "你是作者讀研究論文時的助手。作者會給你一段論文原文與它所在的章節。\
你的工作只有指出這段的結構，讓作者知道卡住的地方在哪裡：**不要翻譯、不要用中文摘要或改寫整段**。\
role：這段在論文裡的角色，只能是 background（背景）、gap（指出缺口或問題）、method（方法步驟）、result（結果）、discussion（討論）、other。\
method 段：equations 對每條方程式（含行內的重要式子）寫 ref（式號或式子）、inputs（輸入）、outputs（輸出）、target（作用在什麼上）、effect（作用結果），每格一短句；motive 寫這一步的動機，原文沒寫就留空。\
result 段：results 每個比較一列：metric（指標）、comparison（和誰比）、values（數值）、difference（差多少）、test（統計檢定）、significance（p 值、信賴區間或是否顯著），原文沒有的格子留空，不要推論。\
其他段落 equations、results 留空陣列。短句用台灣繁體中文，術語、符號、數值保留原文。只輸出 JSON。";

/// The user prompt for one paragraph.
pub fn paragraph_prompt(title: &str, p: &Paragraph, symbols: &SymbolFile) -> String {
    let mut out = format!("論文：{title}\n章節：{}\n頁碼：p.{}\n\n段落原文：\n{}\n", if p.section.is_empty() { "（未知）" } else { &p.section }, p.page(), p.text);
    let eqs: Vec<String> = symbols.equations.iter().filter(|q| p.chips.equations.contains(&q.entry.id)).map(|q| format!("{} {}", q.entry.label.clone().unwrap_or_default(), q.entry.latex.clone().unwrap_or_default())).collect();
    if !eqs.is_empty() {
        out += &format!("\n這段的獨立方程式（PDF 讀出的 LaTeX，可能不完整）：\n{}\n", eqs.join("\n"));
    }
    out
}

pub fn paragraph_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "role": { "type": "string", "enum": ROLES },
            "equations": { "type": "array", "items": { "type": "object", "properties": {
                "ref": { "type": "string" }, "inputs": { "type": "string" }, "outputs": { "type": "string" }, "target": { "type": "string" }, "effect": { "type": "string" } },
                "required": ["ref", "inputs", "outputs", "target", "effect"] } },
            "motive": { "type": "string" },
            "results": { "type": "array", "items": { "type": "object", "properties": {
                "metric": { "type": "string" }, "comparison": { "type": "string" }, "values": { "type": "string" }, "difference": { "type": "string" }, "test": { "type": "string" }, "significance": { "type": "string" } },
                "required": ["metric", "comparison", "values", "difference", "test", "significance"] } }
        },
        "required": ["role", "equations", "motive", "results"]
    })
}

/// Read the model's JSON (unknown roles become `other`).
pub fn parse_analysis(v: &serde_json::Value) -> Analysis {
    let mut a: Analysis = serde_json::from_value(v.clone()).unwrap_or_default();
    if !ROLES.contains(&a.role.as_str()) {
        a.role = "other".into();
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf_text(pages: &[Vec<String>]) -> PaperText {
        crate::synth::paper_text(pages)
    }

    #[test]
    fn paragraphs_sections_and_order() {
        let lines = crate::synth::long_paper_lines(7);
        let paper = pdf_text(&lines);
        let paras = split(&paper);
        assert!(paras.len() > 20, "{}", paras.len());
        let kinds: Vec<&str> = paras.iter().map(|p| p.section_kind.as_str()).collect();
        for k in ["abstract", "intro", "method", "result", "discussion"] {
            assert!(kinds.contains(&k), "{k} in {kinds:?}");
        }
        assert!(paras.iter().all(|p| !p.text.contains("Quaternion")), "references are not paragraphs");
        let m = paras.iter().find(|p| p.section_kind == "method").unwrap();
        assert_eq!(m.section, "3. Methods");
        // ids are stable and unique
        let again = split(&paper);
        assert_eq!(paras.iter().map(|p| &p.id).collect::<Vec<_>>(), again.iter().map(|p| &p.id).collect::<Vec<_>>());
        let mut ids: Vec<&String> = paras.iter().map(|p| &p.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), paras.len());
        // methods and results first
        let order = run_order(&paras);
        assert_eq!(paras[order[0]].section_kind, "method");
        let first_other = order.iter().position(|&i| !matches!(paras[i].section_kind.as_str(), "method" | "result")).unwrap();
        assert!(order[first_other..].iter().all(|&i| !matches!(paras[i].section_kind.as_str(), "method" | "result")));
        // ranges point at the text
        let r = &m.ranges[0];
        let u: Vec<u16> = paper.pages[r.page as usize - 1].text.encode_utf16().collect();
        assert!(m.text.starts_with(String::from_utf16_lossy(&u[r.s..r.e]).split_whitespace().next().unwrap()));
        assert!(r.bbox[3] > r.bbox[1]);
    }

    #[test]
    fn plain_text_paragraphs_and_chips() {
        let p = PaperText::from_strings(&["1. Introduction\nThe anisotropic prior helps.\nIt is new.\n\nSecond paragraph about $w$ here.\n2. Method\nWe use it."]);
        let paras = split(&p);
        let texts: Vec<&str> = paras.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, ["The anisotropic prior helps. It is new.", "Second paragraph about $w$ here.", "We use it."]);
        assert_eq!(paras[2].section_kind, "method");
        let cats: std::collections::HashMap<String, String> = [("anisotropic".to_string(), "方法".to_string())].into();
        let inv = crate::nouns::inventory(&p, &[], &crate::nouns::Filters::default(), &cats);
        let mut paras = paras;
        attach_chips(&mut paras, &inv, &SymbolFile::default());
        assert!(paras[0].chips.terms.contains(&"anisotropic".to_string()), "{:?}", paras[0].chips);
        assert!(!paras[1].chips.terms.contains(&"anisotropic".to_string()));
        let a = parse_analysis(&serde_json::json!({ "role": "method", "equations": [{ "ref": "(1)", "inputs": "x", "outputs": "y", "target": "權重", "effect": "變小" }], "motive": "", "results": [] }));
        assert_eq!((a.role.as_str(), a.equations[0].target.as_str()), ("method", "權重"));
        assert_eq!(parse_analysis(&serde_json::json!({ "role": "summary" })).role, "other");
        assert!(paragraph_schema()["properties"]["role"]["enum"].as_array().unwrap().len() == 6);
    }
}
