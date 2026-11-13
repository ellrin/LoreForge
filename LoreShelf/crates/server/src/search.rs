//! Guided search: every term of the query must match somewhere — the name, the
//! first pages' text (PDFs), a category label (論文, 作業, PDF, 圖片…, or one the
//! author made up), the download host, a folder on the path, or the start of one
//! of its times (`2022`, `2022-11`). So a word that is in no file name still
//! finds the paper that talks about it.
//!
//! The answer also says how to go on: [`Facet`]s are the categories, years and
//! sources that would narrow what was found (or, with an empty query, what the
//! catalog holds) — the page shows them as chips.

use crate::index::{Index, Item, kind_label};
use lore_catalog::category::{self, Filter};
use std::collections::BTreeMap;

/// One result: where the terms matched and, for a text match, the words around it and the part
/// they are in (第 30 頁, 第 4 張投影片).
pub struct Found<'a> {
    pub item: &'a Item,
    pub score: f64,
    pub via: Vec<&'static str>,
    pub snippet: Option<String>,
    pub at: Option<String>,
}

/// A way to narrow: a category (`cat`) or a term to add (`term`: a year, a host).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Facet {
    pub kind: &'static str,
    pub value: String,
    pub label: String,
    pub count: usize,
}

/// Results kept at most, whatever the request says.
pub const MAX_RESULTS: usize = 60;
/// Characters of text shown around a match.
const AROUND: usize = 28;

fn cat_label(id: &str) -> String {
    category::label(id).map(str::to_string).or_else(|| category::custom_name(id).map(str::to_string)).unwrap_or_default()
}

/// The time an item is filed under: downloaded, else the document's date, else created, else modified.
fn filed(item: &Item) -> Option<&str> {
    let t = &item.times;
    [&t.downloaded, &t.doc_created, &t.created, &t.modified].into_iter().find_map(|x| x.as_deref())
}

pub fn snippet(text: &str, lower: &str, at: usize, len: usize) -> String {
    // the lower-cased text has the same characters in the same places (nearly always): cut by characters
    let start = lower[..at].chars().count();
    let n = lower[at..at + len].chars().count();
    let from = start.saturating_sub(AROUND);
    let s: String = text.chars().skip(from).take(AROUND + n + AROUND).map(|c| if c.is_whitespace() { ' ' } else { c }).collect();
    format!("{}{}…", if from > 0 { "…" } else { "" }, s.trim())
}

/// Search the index. Returns the best `limit` results, how many matched in all, and the facets.
pub fn search<'a>(idx: &'a Index, q: &str, filter: &Filter, limit: usize) -> (Vec<Found<'a>>, usize, Vec<Facet>) {
    let terms: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    let mut found: Vec<Found> = Vec::new();
    let mut matched: Vec<&Item> = Vec::new();
    // the whole text: where each term may be (entry → parts), checked against the text when shown
    let in_text: Vec<std::collections::HashMap<u32, Vec<u32>>> = terms.iter().map(|t| idx.ft.candidates(t)).collect();
    let mut text_parts: std::collections::HashMap<usize, (usize, Vec<u32>)> = std::collections::HashMap::new();
    for (ei, e) in idx.entries().iter().enumerate() {
        let item = &e.item;
        if !filter.accepts(&item.categories) {
            continue;
        }
        if terms.is_empty() {
            matched.push(item);
            // only a category chosen: what is in it (newest first); nothing asked at all: only the ways in
            if !filter.is_empty() {
                found.push(Found { item, score: 0.0, via: Vec::new(), snippet: None, at: None });
            }
            continue;
        }
        let name = item.name.to_lowercase();
        let labels: Vec<String> = item.categories.iter().map(|c| cat_label(c).to_lowercase()).chain([kind_label(&item.kind).to_lowercase()]).collect();
        let host = item.download_host.as_deref().unwrap_or_default().to_lowercase();
        let folders = e.paths.iter().map(|p| std::path::Path::new(p).parent().map(|d| d.to_string_lossy().to_lowercase()).unwrap_or_default()).collect::<Vec<_>>();
        let t = &item.times;
        let times = [&t.created, &t.modified, &t.downloaded, &t.opened, &t.doc_created];
        let text = e.text.to_lowercase();
        let (mut score, mut via, mut snip, mut all) = (0.0, Vec::new(), None, true);
        for term in &terms {
            let datey = term.len() >= 4 && term.chars().all(|c| c.is_ascii_digit() || c == '-');
            let mut best: Option<(f64, &'static str)> = None;
            let mut hit = |s: f64, w: &'static str| {
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, w));
                }
            };
            if name.contains(term.as_str()) {
                hit(3.0, "name");
            }
            if labels.iter().any(|l| !l.is_empty() && l.contains(term.as_str())) {
                hit(2.0, "category");
            }
            if !host.is_empty() && host.contains(term.as_str()) {
                hit(1.5, "host");
            }
            if folders.iter().any(|f| f.contains(term.as_str())) {
                hit(1.2, "folder");
            }
            if datey && times.iter().any(|x| x.as_deref().is_some_and(|x| x.starts_with(term.as_str()))) {
                hit(1.0, "time");
            }
            let ti = terms.iter().position(|x| x == term).unwrap_or(0);
            if idx.ft.has(ei) {
                if let Some(parts) = in_text[ti].get(&(ei as u32)) {
                    hit(1.0, "text");
                    text_parts.entry(ei).or_insert((ti, parts.clone()));
                }
            } else if let Some(at) = text.find(term.as_str()) {
                hit(1.0, "text");
                if snip.is_none() && !item.sensitive {
                    snip = Some(snippet(&e.text, &text, at, term.len()));
                }
            }
            match best {
                Some((s, w)) => {
                    score += s;
                    if !via.contains(&w) {
                        via.push(w);
                    }
                }
                None => {
                    all = false;
                    break;
                }
            }
        }
        if all {
            matched.push(item);
            let snippet = if via.contains(&"text") { snip } else { None };
            found.push(Found { item, score, via, snippet, at: None });
        }
    }
    // best first; among equals the most recently filed
    found.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| filed(b.item).cmp(&filed(a.item))));
    let total = matched.len();
    found.truncate(limit.min(MAX_RESULTS));
    // the words around a match in the whole text, and the part it is in (read only for what is shown)
    for f in found.iter_mut().filter(|f| f.via.contains(&"text") && f.snippet.is_none() && !f.item.sensitive) {
        let Some(&ei) = idx.by_id.get(&f.item.id) else { continue };
        let Some((ti, parts)) = text_parts.get(&ei) else { continue };
        if let Some((unit, n, s)) = idx.ft.find(ei, parts, &terms[*ti]) {
            f.snippet = Some(s);
            f.at = Some(crate::fulltext::where_label(&unit, n));
        }
    }
    (found, total, facets(&matched, &terms, filter))
}

/// What would narrow `items`: the use categories, types, years and sources most of them share
/// (never one already asked for, never one that every item has).
fn facets(items: &[&Item], terms: &[String], filter: &Filter) -> Vec<Facet> {
    let total = items.len();
    let (mut uses, mut types, mut years, mut hosts) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for i in items {
        for c in &i.categories {
            let m = if category::group(c) == Some(category::GROUP_USE) { &mut uses } else { &mut types };
            *m.entry(c.clone()).or_insert(0usize) += 1;
        }
        if let Some(y) = filed(i).and_then(|t| t.get(..4)) {
            *years.entry(y.to_string()).or_insert(0usize) += 1;
        }
        if let Some(h) = &i.download_host {
            *hosts.entry(h.clone()).or_insert(0usize) += 1;
        }
    }
    let top = |m: BTreeMap<String, usize>, n: usize, newest: bool| {
        let mut v: Vec<(String, usize)> = m.into_iter().filter(|(_, c)| *c < total || terms.is_empty()).collect();
        if newest {
            v.sort_by(|a, b| b.0.cmp(&a.0));
        } else {
            v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        }
        v.truncate(n);
        v
    };
    let mut out = Vec::new();
    for (kind, m, n) in [("cat", uses, 4), ("cat", types, 4)] {
        for (id, count) in top(m, n, false) {
            if filter.types.contains(&id) || filter.uses.contains(&id) {
                continue;
            }
            out.push(Facet { kind, label: cat_label(&id), value: id, count });
        }
    }
    for (y, count) in top(years, 4, true) {
        if !terms.contains(&y) {
            out.push(Facet { kind: "term", label: format!("{y} 年"), value: y, count });
        }
    }
    for (h, count) in top(hosts, 3, false) {
        if !terms.contains(&h.to_lowercase()) {
            out.push(Facet { kind: "term", label: h.clone(), value: h, count });
        }
    }
    out
}

/// What a sentence must reach to count as "about this file" in [`loose`].
pub const LOOSE_MIN: f64 = 8.0;

/// Words of a spoken sentence worth looking for: Latin words of 3+ letters, and every pair of
/// neighbouring CJK characters (「生物力學的講義」 → 生物, 物力, 力學, …).
pub fn sentence_tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: String| {
        if !out.contains(&t) {
            out.push(t);
        }
    };
    let (mut word, mut cjk) = (String::new(), Vec::<char>::new());
    let is_cjk = |c: char| ('\u{3400}'..='\u{9fff}').contains(&c) || ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{ac00}'..='\u{d7af}').contains(&c);
    for c in text.to_lowercase().chars().chain([' ']) {
        if is_cjk(c) {
            if word.chars().count() >= 3 {
                push(std::mem::take(&mut word));
            }
            word.clear();
            cjk.push(c);
            continue;
        }
        for w in cjk.windows(2) {
            push(w.iter().collect());
        }
        cjk.clear();
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            if word.chars().count() >= 3 {
                push(std::mem::take(&mut word));
            }
            word.clear();
        }
    }
    out
}

/// Files a sentence is probably about (for LoreEcho: the sentence is what the author said, not a
/// query). Each token counts by how rare it is among the names; a hit in the name counts most,
/// then a category label, then the first pages' text. Only files reaching [`LOOSE_MIN`].
pub fn loose<'a>(idx: &'a Index, sentence: &str, filter: &Filter, limit: usize) -> Vec<Found<'a>> {
    let tokens = sentence_tokens(sentence);
    if tokens.is_empty() {
        return Vec::new();
    }
    let entries = idx.entries();
    let names: Vec<String> = entries.iter().map(|e| e.item.name.to_lowercase()).collect();
    let n = names.len() as f64;
    let weight: Vec<f64> = tokens.iter().map(|t| ((n + 1.0) / (1.0 + names.iter().filter(|x| x.contains(t.as_str())).count() as f64)).ln()).collect();
    let in_text: Vec<std::collections::HashMap<u32, Vec<u32>>> = tokens.iter().map(|t| idx.ft.candidates(t)).collect();
    let text_weight: Vec<f64> = in_text.iter().map(|m| ((n + 1.0) / (1.0 + m.len() as f64)).ln()).collect();
    let mut found: Vec<Found> = Vec::new();
    for (e, name) in entries.iter().zip(&names) {
        if !filter.accepts(&e.item.categories) {
            continue;
        }
        let labels: Vec<String> = e.item.categories.iter().map(|c| cat_label(c).to_lowercase()).collect();
        let ei = idx.by_id.get(&e.item.id).copied().unwrap_or(usize::MAX);
        let mut text: Option<String> = None;
        let (mut score, mut via) = (0.0, Vec::new());
        for (ti, (t, w)) in tokens.iter().zip(&weight).enumerate() {
            let (k, where_) = if name.contains(t.as_str()) {
                (3.0 * w, "name")
            } else if labels.iter().any(|l| !l.is_empty() && l.contains(t.as_str())) {
                (1.5 * w, "category")
            } else if idx.ft.has(ei) {
                // in the whole text: weighed by how rare it is among the texts, so one rare word
                // (a tissue in page 30 of the notes) is enough on its own
                if !in_text[ti].contains_key(&(ei as u32)) {
                    continue;
                }
                (text_weight[ti], "text")
            } else if !e.text.is_empty() && text.get_or_insert_with(|| e.text.to_lowercase()).contains(t.as_str()) {
                (0.6 * w, "text")
            } else {
                continue;
            };
            score += k;
            if !via.contains(&where_) {
                via.push(where_);
            }
        }
        if score >= LOOSE_MIN {
            found.push(Found { item: &e.item, score, via, snippet: None, at: None });
        }
    }
    found.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| filed(b.item).cmp(&filed(a.item))));
    found.truncate(limit.min(MAX_RESULTS));
    // where in the text: the part holding the rarest of the sentence's words that are there
    for f in found.iter_mut().filter(|f| f.via.contains(&"text") && !f.item.sensitive) {
        let Some(&ei) = idx.by_id.get(&f.item.id) else { continue };
        let mut best: Option<(f64, usize)> = None;
        for (ti, m) in in_text.iter().enumerate() {
            if m.contains_key(&(ei as u32)) && best.is_none_or(|(w, _)| text_weight[ti] > w) {
                best = Some((text_weight[ti], ti));
            }
        }
        if let Some((_, ti)) = best
            && let Some((unit, n, s)) = idx.ft.find(ei, &in_text[ti][&(ei as u32)], &tokens[ti])
        {
            f.at = Some(crate::fulltext::where_label(&unit, n));
            f.snippet = Some(s);
        }
    }
    found
}
