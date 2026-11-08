//! Abbreviations in plain text ("long form (ABC)" and later uses of ABC),
//! and spelling variants of the same word (`nonlinear` / `non-linear`,
//! `ResNet-T` / `ResNet-Tiny`).

use crate::scan::{Area, ItemKind, Scan};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Use {
    pub off: usize,
    pub area: Area,
}

#[derive(Debug, Clone)]
pub struct Expansion {
    pub off: usize,
    pub area: Area,
    pub long: String,
}

#[derive(Debug, Clone, Default)]
pub struct AbbrevInfo {
    pub short: String,
    /// Every occurrence, expansions included.
    pub uses: Vec<Use>,
    pub expansions: Vec<Expansion>,
}

/// Words that start a hyphenated name without being part of the acronym
/// (`No-GAN`, `Non-CNN`): the acronym is the part after the hyphen.
const COMMON_PREFIX: &[&str] = &["No", "Non", "Pre", "Post", "Full", "Multi", "Semi", "Anti", "Self", "Sub", "Super", "Over", "Under", "Cross", "Co", "Re", "De", "Un", "In", "Out", "All", "Mid"];

fn is_roman(w: &str) -> bool {
    w.chars().all(|c| matches!(c, 'I' | 'V' | 'X'))
}

fn plain_acronym(w: &str) -> Option<String> {
    let w = if w.len() > 2 && w.ends_with('s') && w[..w.len() - 1].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) { &w[..w.len() - 1] } else { w };
    let upper = w.chars().filter(|c| c.is_ascii_uppercase()).count();
    if upper >= 2 && w.len() <= 10 && w.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && w.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) && !is_roman(w) {
        Some(w.to_string())
    } else {
        None
    }
}

/// Acronyms in one word (`CNNs` → CNN, `GAN-based` → GAN, `Grad-CAM` → Grad-CAM).
pub fn acronyms(w: &str) -> Vec<String> {
    if !w.contains('-') {
        return plain_acronym(w).into_iter().collect();
    }
    let parts: Vec<&str> = w.split('-').collect();
    let caps: Vec<String> = parts.iter().filter_map(|p| plain_acronym(p)).collect();
    if caps.is_empty() {
        return vec![];
    }
    let first = parts[0];
    let mixed_name = parts.iter().all(|p| p.chars().next().is_some_and(|c| c.is_uppercase()))
        && plain_acronym(first).is_none()
        && first.len() > 1
        && !COMMON_PREFIX.contains(&first);
    if mixed_name && parts.len() == 2 {
        let whole = w.trim_end_matches('s');
        return vec![whole.to_string()];
    }
    caps
}

/// Schwartz–Hearst: the shortest tail of `words` whose letters spell `short`.
pub fn long_form(short: &str, words: &[&str]) -> Option<String> {
    let s: Vec<char> = short.chars().filter(|c| c.is_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
    if s.is_empty() || words.is_empty() {
        return None;
    }
    let max = (s.len() + 5).min(s.len() * 2).min(words.len());
    for n in 1..=max {
        let cand = &words[words.len() - n..];
        let text = cand.join(" ");
        if matches_sh(&s, &text.to_lowercase()) {
            // first letter must start the first word
            if cand[0].to_lowercase().starts_with(s[0]) {
                return Some(text);
            }
        }
    }
    None
}

fn matches_sh(s: &[char], long: &str) -> bool {
    let l: Vec<char> = long.chars().collect();
    let mut li = l.len() as isize - 1;
    for (k, &c) in s.iter().enumerate().rev() {
        loop {
            if li < 0 {
                return false;
            }
            let lc = l[li as usize];
            let at_word_start = li == 0 || !l[li as usize - 1].is_alphanumeric();
            li -= 1;
            if lc == c && (k != 0 || at_word_start) {
                break;
            }
        }
    }
    true
}

pub fn analyze(scan: &Scan) -> BTreeMap<String, AbbrevInfo> {
    let mut out: BTreeMap<String, AbbrevInfo> = BTreeMap::new();
    let items = &scan.items;
    for (i, it) in items.iter().enumerate() {
        let ItemKind::Word(w) = &it.kind else { continue };
        for a in acronyms(w) {
            let e = out.entry(a.clone()).or_insert_with(|| AbbrevInfo { short: a.clone(), ..Default::default() });
            e.uses.push(Use { off: it.start, area: it.ctx.area });
            // "(ABC)" right after words: an expansion
            let paren = matches!(items.get(i.wrapping_sub(1)).map(|x| &x.kind), Some(ItemKind::Punct('(')))
                && matches!(items.get(i + 1).map(|x| &x.kind), Some(ItemKind::Punct(')' | ';' | ',')));
            if paren && plain_acronym(w).is_some() || paren && w.contains('-') {
                let mut ws: Vec<&str> = Vec::new();
                let mut j = i.wrapping_sub(2);
                while j < items.len() && items[j].sentence == it.sentence {
                    match &items[j].kind {
                        ItemKind::Word(x) => ws.push(x),
                        _ => break,
                    }
                    j = j.wrapping_sub(1);
                    if ws.len() > 16 {
                        break;
                    }
                }
                ws.reverse();
                if let Some(long) = long_form(&a, &ws) {
                    e.expansions.push(Expansion { off: it.start, area: it.ctx.area, long });
                }
            }
        }
    }
    out
}

/// Groups of different spellings of one word: hyphenation (`non-linear` /
/// `nonlinear`) and truncated name parts (`Net-T` / `Net-Tiny`).
pub fn spelling_variants(scan: &Scan) -> Vec<Vec<(String, usize)>> {
    // surface → first offset and count
    let mut surfaces: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for it in &scan.items {
        if let ItemKind::Word(w) = &it.kind {
            if w.chars().count() < 3 || w.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let e = surfaces.entry(w.clone()).or_insert((it.start, 0));
            e.1 += 1;
        }
    }
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in surfaces.keys() {
        groups.entry(s.to_lowercase().replace('-', "")).or_default().push(s.clone());
    }
    let mut out = Vec::new();
    for (_, v) in groups {
        // distinct lower-case spellings (sentence-initial capitals are not variants)
        let mut lower: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for s in &v {
            let l = s.to_lowercase();
            let off = surfaces[s].0;
            let e = lower.entry(l).or_insert((s.clone(), off));
            if off < e.1 {
                *e = (s.clone(), off);
            }
        }
        if lower.len() >= 2 {
            let mut g: Vec<(String, usize)> = lower.into_values().collect();
            g.sort_by_key(|x| x.1);
            out.push(g);
        }
    }
    // truncated hyphen parts: Name-T vs Name-Tiny
    let hy: Vec<&String> = surfaces.keys().filter(|s| s.contains('-')).collect();
    for (ai, a) in hy.iter().enumerate() {
        for b in &hy[ai + 1..] {
            let pa: Vec<&str> = a.split('-').collect();
            let pb: Vec<&str> = b.split('-').collect();
            if pa.len() != pb.len() {
                continue;
            }
            let diff: Vec<usize> = (0..pa.len()).filter(|k| pa[*k] != pb[*k]).collect();
            if diff.len() != 1 {
                continue;
            }
            let (x, y) = (pa[diff[0]], pb[diff[0]]);
            let (short, long) = if x.len() < y.len() { (x, y) } else { (y, x) };
            if !short.is_empty() && long.starts_with(short) && short.chars().next().is_some_and(|c| c.is_uppercase()) && short.len() <= 2 {
                let mut g = vec![((*a).clone(), surfaces[*a].0), ((*b).clone(), surfaces[*b].0)];
                g.sort_by_key(|x| x.1);
                out.push(g);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acronym_shapes() {
        assert_eq!(acronyms("CNNs"), vec!["CNN"]);
        assert_eq!(acronyms("GAN-based"), vec!["GAN"]);
        assert_eq!(acronyms("No-GAN"), vec!["GAN"]);
        assert_eq!(acronyms("Grad-CAM"), vec!["Grad-CAM"]);
        assert!(acronyms("III").is_empty());
        assert!(acronyms("PadNet").is_empty());
        assert!(acronyms("U-Net").is_empty());
    }

    #[test]
    fn long_forms() {
        assert_eq!(long_form("EMA", &["an", "exponential", "moving", "average"]).as_deref(), Some("exponential moving average"));
        assert_eq!(long_form("SSIM", &["and", "structural", "similarity", "index"]).as_deref(), Some("structural similarity index"));
        assert_eq!(long_form("BCE", &["binary", "cross-entropy"]).as_deref(), Some("binary cross-entropy"));
        assert_eq!(long_form("XYZ", &["foo", "bar"]), None);
    }

    #[test]
    fn finds_expansions_and_variants() {
        let s = Scan::from_str(
            "a.tex",
            "We use a widget network (WN) on non-widget data. The WN and WNs are fast; nonwidget samples help. Net-S beats Net-Small.",
        );
        let a = analyze(&s);
        assert_eq!(a["WN"].expansions.len(), 1);
        assert_eq!(a["WN"].expansions[0].long, "widget network");
        assert_eq!(a["WN"].uses.len(), 3);
        let v = spelling_variants(&s);
        let flat: Vec<Vec<String>> = v.iter().map(|g| g.iter().map(|x| x.0.clone()).collect()).collect();
        assert!(flat.contains(&vec!["non-widget".to_string(), "nonwidget".to_string()]), "{flat:?}");
        assert!(flat.contains(&vec!["Net-S".to_string(), "Net-Small".to_string()]), "{flat:?}");
    }
}
