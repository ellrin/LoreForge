//! PDF-only mode: poppler text geometry + page ink → asset catalog with crops.

pub mod detect;
pub mod layout;
pub mod poppler;

use crate::crop::Renderer;
use crate::error::Result;
use crate::model::*;
use detect::{Doc, HeadingHit, Role, join_lines, split_references};
use layout::{Ink, Line, TextPage, parse_bbox_layout};
use poppler::Poppler;
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PdfOptions {
    /// Catalog folder: `catalog.json`, `pages/`, `crops/`.
    pub out_dir: PathBuf,
    /// Gray render used for region detection.
    pub analysis_dpi: u32,
    /// Crop resolution.
    pub crop_dpi: u32,
    /// Write PNG crops for every asset.
    pub render_crops: bool,
    /// Write `catalog.json`.
    pub save: bool,
    pub poppler: Poppler,
    /// Read unnumbered headings set in a larger size (Springer journals) as sections. Off by
    /// default: only callers that need the section structure (PaperComposer's 期刊寫法) turn it on.
    pub unnumbered_headings: bool,
}

impl PdfOptions {
    pub fn new(out_dir: &Path) -> PdfOptions {
        PdfOptions {
            out_dir: out_dir.to_path_buf(),
            analysis_dpi: 100,
            crop_dpi: 200,
            render_crops: true,
            save: true,
            poppler: Poppler::discover(),
            unnumbered_headings: false,
        }
    }
}

/// Extract a PDF: renders pages (cached in `out_dir/pages`), detects regions,
/// writes crops to `out_dir/crops` and the catalog to `out_dir/catalog.json`.
pub fn extract(pdf: &Path, o: &PdfOptions) -> Result<AssetCatalog> {
    o.poppler.require()?;
    let info = o.poppler.info(pdf)?;
    let pages = parse_bbox_layout(&o.poppler.bbox_layout(pdf)?)?;
    let r = Renderer::new(o.poppler.clone(), pdf, &o.out_dir);
    let mut inks = vec![];
    let mut renders = vec![];
    for p in &pages {
        let png = r.page_png(p.number, o.analysis_dpi, true)?;
        inks.push(Some(Ink::from_png(&png, o.analysis_dpi as f64 / 72.0)?));
        renders.push(Renderer::page_rel(p.number, o.analysis_dpi, true));
    }
    let mut cat = analyze_with(&pages, inks, &info, o.unnumbered_headings);
    cat.source = DocSource::Pdf {
        path: pdf.display().to_string(),
    };
    for (pi, pg) in cat.pages.iter_mut().enumerate() {
        pg.render = renders.get(pi).cloned();
    }
    if o.render_crops {
        for a in cat.assets.iter_mut() {
            let (Some(page), Some(bbox)) = (a.page, a.bbox) else {
                continue;
            };
            let rel = format!("crops/{}.png", a.id);
            r.crop_png(page, &bbox, o.crop_dpi, &a.masks, &o.out_dir.join(&rel))?;
            a.crop = Some(rel);
        }
    }
    if o.save {
        cat.save(&o.out_dir.join("catalog.json"))?;
    }
    Ok(cat)
}

fn id_number(n: &str) -> String {
    normalize_number(n)
}

/// The detection pipeline on parsed pages (no poppler; `inks` may be `None`).
pub fn analyze(
    pages: &[TextPage],
    inks: Vec<Option<Ink>>,
    info: &poppler::PdfInfo,
) -> AssetCatalog {
    analyze_with(pages, inks, info, false)
}

/// [`analyze`], with [`PdfOptions::unnumbered_headings`].
pub fn analyze_with(
    pages: &[TextPage],
    inks: Vec<Option<Ink>>,
    info: &poppler::PdfInfo,
    unnumbered_headings: bool,
) -> AssetCatalog {
    let mut d = Doc::new(pages, inks);
    d.unnumbered_headings = unnumbered_headings;
    let mut cat = AssetCatalog {
        schema: SCHEMA,
        ..Default::default()
    };
    cat.meta.page_count = Some(if info.pages > 0 {
        info.pages
    } else {
        pages.len() as u32
    });

    let mut caps = d.captions();
    caps.sort_by_key(|a| (a.page, a.bbox.y0 as i64));
    let heads = d.headings();

    // figures and tables
    let mut taken: Vec<(usize, BBox)> = vec![];
    let mut seen: HashSet<(AssetKind, String)> = HashSet::new();
    let mut cap_pos: Vec<(String, usize, usize)> = vec![]; // (asset id, page, first line)
    for c in &caps {
        let key = (c.kind, id_number(&c.number));
        if seen.contains(&key) {
            continue;
        }
        let same_page: Vec<BBox> = taken
            .iter()
            .filter(|(p, _)| *p == c.page)
            .map(|(_, b)| *b)
            .collect();
        let found = d.region_for(c, &caps, &same_page);
        let Some((bbox, conf, notes)) = found else {
            continue;
        };
        seen.insert(key);
        let id = format!("{}{}", c.kind.prefix(), id_number(&c.number));
        let mut a = Asset::new(id.clone(), c.kind);
        a.number = Some(c.number.clone());
        a.caption = Some(strip_caption_head(&layout::fix_small_caps(&c.text)));
        a.page = Some(c.page as u32 + 1);
        a.bbox = Some(bbox.rounded());
        a.caption_bbox = Some(c.bbox.rounded());
        a.confidence = conf;
        a.notes = notes;
        let t = d.text_in(c.page, &bbox);
        if !t.is_empty() {
            a.text = Some(t);
        }
        a.source_ref = SourceRef {
            page: a.page,
            ..Default::default()
        };
        d.claim(c.page, &bbox);
        taken.push((c.page, bbox));
        cap_pos.push((id, c.page, c.lines[0]));
        cat.assets.push(a);
    }
    // captions whose region was not found are still useful (caption only)
    for c in &caps {
        let key = (c.kind, id_number(&c.number));
        if seen.insert(key) {
            let id = format!("{}{}", c.kind.prefix(), id_number(&c.number));
            let mut a = Asset::new(id.clone(), c.kind);
            a.number = Some(c.number.clone());
            a.caption = Some(strip_caption_head(&layout::fix_small_caps(&c.text)));
            a.page = Some(c.page as u32 + 1);
            a.caption_bbox = Some(c.bbox.rounded());
            a.confidence = 0.2;
            a.notes.push("region not found".into());
            a.source_ref = SourceRef {
                page: a.page,
                ..Default::default()
            };
            cap_pos.push((id, c.page, c.lines[0]));
            cat.assets.push(a);
        }
    }

    // equations
    let eqs = d.equations(&taken);
    let mut eq_seen = HashSet::new();
    for e in eqs {
        let (pi, num, bbox) = (e.page, e.number.clone(), e.bbox);
        if !eq_seen.insert(num.clone()) {
            continue;
        }
        let mut a = Asset::new(format!("eq{num}"), AssetKind::Equation);
        a.number = Some(num);
        a.page = Some(pi as u32 + 1);
        a.bbox = Some(bbox.rounded());
        a.text = Some(e.text.clone());
        a.masks = e.masks.iter().map(|m| m.rounded()).collect();
        a.confidence = e.confidence;
        a.source_ref = SourceRef {
            page: a.page,
            ..Default::default()
        };
        d.claim(pi, &bbox.union(&e.tag));
        let first = d.pages[pi]
            .lines
            .iter()
            .position(|l| l.bbox.intersect(&bbox).is_some())
            .unwrap_or(0);
        cap_pos.push((a.id.clone(), pi, first));
        cat.assets.push(a);
    }
    cat.assets.sort_by(|a, b| {
        (a.page, a.kind, a.bbox.map(|b| b.y0 as i64)).cmp(&(
            b.page,
            b.kind,
            b.bbox.map(|b| b.y0 as i64),
        ))
    });

    sections_and_refs(&d, &heads, &mut cat, &cap_pos);
    front_matter(&d, &heads, info, &mut cat);
    cat.pages = pages
        .iter()
        .map(|p| PageInfo {
            number: p.number,
            width: p.width,
            height: p.height,
            columns: d.cols.cols.clone(),
            render: None,
        })
        .collect();
    cat
}

/// `Fig. 3. The model …` → `The model …`; `Table 2 Results` → `Results`.
pub fn strip_caption_head(s: &str) -> String {
    let re = Regex::new(
        r"^(?:Fig\.|Figure|FIGURE|FIG\.|Fig|Table|TABLE)\s*(?:\d{1,2}|[IVXL]{1,6})\s*[.:|]?\s*",
    )
    .unwrap();
    re.replace(s, "").trim().to_string()
}

fn is_reference_heading(h: &HeadingHit) -> bool {
    let t = h.title.to_lowercase();
    t.starts_with("reference") || t == "bibliography"
}

fn sections_and_refs(
    d: &Doc,
    heads: &[HeadingHit],
    cat: &mut AssetCatalog,
    cap_pos: &[(String, usize, usize)],
) {
    // walk lines in pdftotext reading order
    let mut order: Vec<(usize, usize)> = vec![];
    for pi in 0..d.pages.len() {
        for &li in &d.order[pi] {
            order.push((pi, li));
        }
    }
    let head_at = |pi: usize, li: usize| heads.iter().position(|h| h.page == pi && h.line == li);
    let mut cur: Option<usize> = None; // index into cat.sections
    let mut in_refs = false;
    let mut ref_lines: Vec<(Line, f64)> = vec![];
    let mut para: Vec<&Line> = vec![];
    let mut para_block: Option<(usize, usize)> = None;
    let mut parent_num: Vec<String> = vec![];
    let flush = |para: &mut Vec<&Line>, cat: &mut AssetCatalog, cur: Option<usize>| {
        if para.is_empty() {
            return;
        }
        let text = join_lines(para.drain(..));
        let Some(si) = cur else { return };
        let ps = &mut cat.sections[si].paragraphs;
        // continuation across a column/page break or around an equation
        if let Some(last) = ps.last_mut()
            && !last.ends_with(['.', '?', '!', ':'])
            && text
                .chars()
                .next()
                .is_some_and(|c| c.is_lowercase() || c.is_ascii_digit() || c == '(')
        {
            last.push(' ');
            last.push_str(&text);
            return;
        }
        if text.chars().filter(|c| c.is_alphabetic()).count() >= 3 {
            ps.push(text);
        }
    };
    let mut sec_of_pos: Vec<((usize, usize), usize)> = vec![];
    for &(pi, li) in &order {
        if let Some(hi) = head_at(pi, li) {
            flush(&mut para, cat, cur);
            para_block = None;
            let h = &heads[hi];
            if in_refs && !is_reference_heading(h) {
                in_refs = false;
            }
            if is_reference_heading(h) {
                in_refs = true;
                cur = None;
                continue;
            }
            let number = h.number.as_ref().map(|n| {
                if h.level == 1 {
                    parent_num = vec![n.clone()];
                    n.clone()
                } else if n.contains('.') {
                    n.clone()
                } else {
                    // IEEE "A." under "II."
                    parent_num.truncate(1);
                    format!("{}.{n}", parent_num.first().cloned().unwrap_or_default())
                }
            });
            let base = match &number {
                Some(n) => {
                    // top level may be roman (IEEE); deeper levels are digits or letters
                    let mut parts = n.split('.');
                    let head = normalize_number(parts.next().unwrap_or(""));
                    let rest: Vec<String> = parts.map(|p| p.to_lowercase()).collect();
                    format!(
                        "s{}",
                        std::iter::once(head)
                            .chain(rest)
                            .collect::<Vec<_>>()
                            .join(".")
                    )
                }
                None => format!(
                    "s-{}",
                    h.title
                        .to_lowercase()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join("-")
                ),
            };
            let id = if h.title.eq_ignore_ascii_case("abstract") {
                "abstract".to_string()
            } else {
                base
            };
            let id = if cat.sections.iter().any(|s| s.id == id) {
                format!("{id}-{}", cat.sections.len())
            } else {
                id
            };
            cat.sections.push(Section {
                id,
                level: h.level,
                number,
                title: h.title.clone(),
                label: None,
                paragraphs: vec![],
                source_ref: SourceRef {
                    page: Some(pi as u32 + 1),
                    ..Default::default()
                },
            });
            cur = Some(cat.sections.len() - 1);
            continue;
        }
        if let Some(si) = cur {
            sec_of_pos.push(((pi, li), si));
        }
        let role = d.roles[pi][li];
        let l = &d.pages[pi].lines[li];
        if in_refs {
            if matches!(role, Role::Body | Role::Other) {
                let col = d.cols.span_of(&l.bbox).0;
                let col_x0 = if d.cols.column_of(&l.bbox).is_none() {
                    d.cols.cols[0].0
                } else {
                    col
                };
                ref_lines.push((l.clone(), col_x0));
            }
            continue;
        }
        if !matches!(role, Role::Body | Role::Other) {
            continue;
        }
        let blk = (pi, l.block);
        if para_block != Some(blk) {
            flush(&mut para, cat, cur);
            para_block = Some(blk);
        } else if let Some(prev) = para.last()
            && new_paragraph(d, prev, l)
        {
            flush(&mut para, cat, cur);
        }
        para.push(l);
    }
    flush(&mut para, cat, cur);

    // assets → section (the section that was open at the caption / region)
    let rank: std::collections::HashMap<(usize, usize), usize> =
        order.iter().enumerate().map(|(k, pos)| (*pos, k)).collect();
    for (id, pi, li) in cap_pos {
        let Some(&at) = rank.get(&(*pi, *li)) else {
            continue;
        };
        let sec = sec_of_pos
            .iter()
            .rfind(|(pos, _)| rank.get(pos).is_some_and(|r| *r <= at))
            .map(|x| x.1);
        if let (Some(si), Some(a)) = (sec, cat.assets.iter_mut().find(|a| &a.id == id)) {
            a.section = Some(cat.sections[si].id.clone());
        }
    }

    let bracket = Regex::new(r"^\[(\d{1,3})\]\s*").unwrap();
    for (k, text) in split_references(&ref_lines).into_iter().enumerate() {
        let (key, text) = match bracket.captures(&text) {
            Some(c) => (
                format!("[{}]", &c[1]),
                bracket.replace(&text, "").to_string(),
            ),
            None => (format!("ref{}", k + 1), text),
        };
        cat.references.push(Reference {
            key,
            text,
            ..Default::default()
        });
    }
}

/// Inside one text block: an indented first line after a sentence end, or a
/// short last line ending a sentence, starts a new paragraph.
fn new_paragraph(d: &Doc, prev: &Line, l: &Line) -> bool {
    let (x0, x1) = d.cols.span_of(&l.bbox);
    let h = d.cols.line_height;
    let ends = prev.text().trim_end().ends_with(['.', '?', '!', ':']);
    let indented =
        l.bbox.x0 > x0 + 0.6 * h && l.bbox.x0 < x0 + 3.0 * h && (prev.bbox.x0 - x0).abs() < 0.6 * h;
    let short_prev = prev.bbox.x1 < x1 - 2.0 * h;
    ends && (indented || short_prev) || l.bbox.y0 - prev.bbox.y1 > 0.8 * h
}

fn front_matter(d: &Doc, heads: &[HeadingHit], info: &poppler::PdfInfo, cat: &mut AssetCatalog) {
    let Some(p) = d.pages.first() else { return };
    // title: pdfinfo when it looks like one, else the tallest lines near the top of page 1
    let plausible = |t: &str| {
        let lower = t.to_lowercase();
        t.split_whitespace().count() >= 3
            && !lower.ends_with(".pdf")
            && !lower.ends_with(".dvi")
            && !lower.starts_with("microsoft")
            && !lower.contains("untitled")
    };
    let first_head = heads
        .iter()
        .find(|h| h.page == 0 && !h.title.eq_ignore_ascii_case("abstract"))
        .map(|h| p.lines[h.line].bbox.y0)
        .unwrap_or(p.height);
    let top: Vec<(usize, &Line)> = p
        .lines
        .iter()
        .enumerate()
        .filter(|(li, l)| {
            l.bbox.y0 < 0.5 * p.height
                && l.bbox.y0 < first_head
                && d.roles[0][*li] != Role::Furniture
                && l.wordy_ratio() > 0.5
        })
        .collect();
    let max_h = top.iter().map(|(_, l)| l.height()).fold(0.0, f64::max);
    let tall: Vec<(usize, &Line)> = top
        .iter()
        .filter(|(_, l)| l.height() >= max_h * 0.85 && max_h > 1.25 * d.cols.line_height)
        .copied()
        .collect();
    // consecutive tall lines form groups (journal masthead vs title): keep the wordiest
    let mut groups: Vec<Vec<(usize, &Line)>> = vec![];
    for (li, l) in tall {
        match groups.last_mut() {
            Some(g) if l.bbox.y0 - g.last().unwrap().1.bbox.y1 < 0.8 * l.height() => {
                g.push((li, l))
            }
            _ => groups.push(vec![(li, l)]),
        }
    }
    let title_lines: Vec<(usize, &Line)> = groups
        .into_iter()
        .max_by_key(|g| g.iter().map(|(_, l)| l.words.len()).sum::<usize>())
        .unwrap_or_default();
    cat.meta.title = match &info.title {
        Some(t) if plausible(t) => t.clone(),
        _ => join_lines(title_lines.iter().map(|(_, l)| *l)),
    };
    // authors: pdfinfo, else the block right below the title
    let clean_name = |s: &str| -> Option<String> {
        let n: String = s
            .chars()
            .filter(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '.' | '\''))
            .collect();
        let n = n
            .split_whitespace()
            .filter(|w| w.chars().count() > 1 || w.ends_with('.'))
            .collect::<Vec<_>>()
            .join(" ");
        let words = n.split_whitespace().count();
        (2..=5).contains(&words).then_some(n)
    };
    let split_names = |s: &str| -> Vec<String> {
        s.replace(" and ", ",")
            .replace(';', ",")
            .split(',')
            .filter_map(clean_name)
            .collect()
    };
    if let Some((last_li, last)) = title_lines.last() {
        let below: Vec<&Line> = p
            .lines
            .iter()
            .enumerate()
            .filter(|(li, l)| {
                *li > *last_li
                    && l.bbox.y0 >= last.bbox.y1 - 1.0
                    && l.bbox.y0 < last.bbox.y1 + 4.0 * d.cols.line_height
            })
            .map(|(_, l)| l)
            .take(3)
            .collect();
        if let Some(first) = below.first() {
            let blk: Vec<&Line> = below
                .iter()
                .copied()
                .filter(|l| l.block == first.block)
                .collect();
            cat.meta.authors = split_names(&join_lines(blk.into_iter()));
        }
    }
    if let Some(a) = info
        .author
        .as_deref()
        .filter(|a| a.chars().any(char::is_alphabetic))
    {
        let from_info = split_names(a);
        if from_info.len() > cat.meta.authors.len() {
            cat.meta.authors = from_info;
        }
    }
    // abstract and keywords
    let abs_re = Regex::new(r"^(?i)abstract\s*[—–:.\-]\s*").unwrap();
    let index_terms = Regex::new(r"(?i)index terms\s*[—–:\-]").unwrap();
    let kw_re = Regex::new(r"^(?i)(index terms|keywords|key words)\s*[—–:.\-]?\s*").unwrap();
    let pages_front: Vec<(usize, usize)> = (0..d.pages.len().min(2))
        .flat_map(|pi| (0..d.pages[pi].lines.len()).map(move |li| (pi, li)))
        .collect();
    let block_text = |pi: usize, li: usize| -> String {
        let pg = &d.pages[pi];
        let b = pg.lines[li].block;
        join_lines(pg.lines.iter().skip(li).take_while(|l| l.block == b))
    };
    if cat.meta.abstract_text.is_none()
        && let Some(s) = cat.sections.iter().find(|s| s.id == "abstract")
    {
        let long: Vec<&String> = s
            .paragraphs
            .iter()
            .filter(|p| p.split_whitespace().count() >= 30)
            .collect();
        let mut t = if long.is_empty() {
            s.paragraphs.join(" ")
        } else {
            long.into_iter().cloned().collect::<Vec<_>>().join(" ")
        };
        if let Some(m) = kw_re.find(&t) {
            t.truncate(m.start());
        }
        if !t.is_empty() {
            cat.meta.abstract_text = Some(t);
        }
    }
    for &(pi, li) in &pages_front {
        let t = d.pages[pi].lines[li].text();
        if cat.meta.abstract_text.is_none() && abs_re.is_match(&t) {
            let whole = block_text(pi, li);
            let mut abs = abs_re.replace(&whole, "").to_string();
            if let Some(m) = index_terms.find(&abs) {
                abs.truncate(m.start());
            }
            cat.meta.abstract_text = Some(abs.trim().to_string());
            if !cat.sections.iter().any(|s| s.id == "abstract") {
                cat.sections.insert(
                    0,
                    Section {
                        id: "abstract".into(),
                        level: 1,
                        title: "Abstract".into(),
                        paragraphs: vec![abs.trim().to_string()],
                        source_ref: SourceRef {
                            page: Some(pi as u32 + 1),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                );
            }
        }
        if cat.meta.keywords.is_empty() && kw_re.is_match(&t) {
            let whole = block_text(pi, li);
            let rest = kw_re.replace(&whole, "").to_string();
            let mut kws: Vec<String> = rest
                .split([',', ';', '·'])
                .map(|k| k.trim().trim_end_matches('.').to_string())
                .filter(|k| !k.is_empty() && k.len() < 60)
                .collect();
            if kws.len() <= 1 {
                // Elsevier: one keyword per line below "Keywords:"
                let pg = &d.pages[pi];
                let b = pg.lines[li].block;
                kws = pg
                    .lines
                    .iter()
                    .skip(li)
                    .take_while(|l| l.block == b)
                    .map(|l| kw_re.replace(&l.text(), "").trim().to_string())
                    .filter(|k| !k.is_empty())
                    .collect();
            }
            cat.meta.keywords = kws;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_heads_are_stripped() {
        assert_eq!(strip_caption_head("Fig. 3. The model."), "The model.");
        assert_eq!(strip_caption_head("TABLE IV Ablation"), "Ablation");
        assert_eq!(strip_caption_head("Table 2 Results"), "Results");
    }

    #[test]
    fn synthetic_page_pipeline() {
        let (pages, inks) = detect::tests::synthetic();
        let c = analyze(&pages, inks, &poppler::PdfInfo::default());
        let ids: Vec<&str> = c.assets.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["fig1", "tab1", "eq1"]);
        let fig = c.asset("fig1").unwrap();
        assert_eq!(
            fig.caption.as_deref(),
            Some("Overview of the model with encoder")
        );
        assert_eq!(fig.section.as_deref(), Some("s1"));
        assert_eq!(
            c.asset("tab1").unwrap().caption.as_deref(),
            Some("Results on the test set.")
        );
        assert_eq!(c.sections.len(), 1);
        assert_eq!(c.sections[0].title, "Introduction");
        // prose blocks only (figure, table, equation and caption lines left out)
        assert_eq!(
            c.sections[0].paragraphs.len(),
            4,
            "{:?}",
            c.sections[0].paragraphs
        );
        assert!(
            c.sections[0]
                .paragraphs
                .iter()
                .all(|p| !p.contains("Method") && !p.contains("Overview") && !p.contains("Wx"))
        );
        assert_eq!(c.references.len(), 3);
        assert_eq!(c.references[1].key, "[2]");
        assert_eq!(c.references[1].text, "B. Two, Other things, Conf., 2021.");
    }
}
