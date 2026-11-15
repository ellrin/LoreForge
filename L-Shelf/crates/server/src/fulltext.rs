//! Search over the whole text of the author's files (`lore_catalog::fulltext`, filled by
//! `loreshelf extract`). Only an inverted list is kept in memory — token → (entry, part) — never
//! the text itself; the text of a found part is read from the cache when a snippet is shown.
//!
//! Tokens: Latin words (letters and digits, lower case, 2+ characters) and every pair of
//! neighbouring CJK characters. A query word matches a Latin token it starts (`segment` →
//! `segmentation`); a CJK query matches where all its pairs are in the same part, checked against
//! the text before it is shown.

use lore_catalog::fulltext::FullText;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub fn is_cjk(c: char) -> bool {
    ('\u{3400}'..='\u{9fff}').contains(&c) || ('\u{f900}'..='\u{faff}').contains(&c) || ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{ac00}'..='\u{d7af}').contains(&c)
}

/// The tokens of a text, in order, repeats included.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut word, mut cjk) = (String::new(), Vec::<char>::new());
    let flush_word = |w: &mut String, out: &mut Vec<String>| {
        if w.chars().count() >= 2 {
            out.push(std::mem::take(w));
        }
        w.clear();
    };
    for c in text.chars().flat_map(char::to_lowercase).chain([' ']) {
        if is_cjk(c) {
            flush_word(&mut word, &mut out);
            cjk.push(c);
            continue;
        }
        for w in cjk.windows(2) {
            out.push(w.iter().collect());
        }
        cjk.clear();
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            flush_word(&mut word, &mut out);
        }
    }
    out
}

/// Where a token occurs: (entry index, part number).
type Postings = Vec<(u32, u32)>;

#[derive(Debug, Default)]
pub struct FullIndex {
    dir: PathBuf,
    /// token → postings, sorted by token (prefix lookups).
    map: BTreeMap<String, Postings>,
    /// entry index → content id (the cache file's name) and its unit (`page`, `slide`, `block`).
    files: HashMap<u32, (String, String)>,
}

impl FullIndex {
    /// Read the cache for these (entry index, content id) pairs.
    pub fn load(dir: &Path, ids: impl Iterator<Item = (usize, String)>) -> FullIndex {
        let mut ix = FullIndex { dir: dir.to_path_buf(), ..FullIndex::default() };
        for (i, id) in ids {
            let Some(ft) = FullText::load(dir, &id) else { continue };
            if ft.parts.is_empty() {
                continue;
            }
            ix.files.insert(i as u32, (id, ft.unit.clone()));
            for p in &ft.parts {
                let mut seen: Vec<String> = tokens(&p.text);
                seen.sort_unstable();
                seen.dedup();
                for t in seen {
                    ix.map.entry(t).or_default().push((i as u32, p.n));
                }
            }
        }
        ix
    }

    pub fn has(&self, entry: usize) -> bool {
        self.files.contains_key(&(entry as u32))
    }

    /// In how many files a token occurs.
    pub fn files_with(&self, token: &str) -> usize {
        self.map.get(token).map(|p| distinct_entries(p)).unwrap_or(0)
    }

    /// Parts that may hold the query word: entry → part numbers (ascending).
    pub fn candidates(&self, word: &str) -> HashMap<u32, Vec<u32>> {
        let w = word.to_lowercase();
        let toks = tokens(&w);
        let mut out: HashMap<u32, Vec<u32>> = HashMap::new();
        if toks.is_empty() {
            return out;
        }
        if toks.len() == 1 && !toks[0].chars().any(is_cjk) {
            // a Latin word: every token it starts
            for (_, posts) in self.map.range(toks[0].clone()..).take_while(|(k, _)| k.starts_with(&toks[0])) {
                for &(e, n) in posts {
                    out.entry(e).or_default().push(n);
                }
            }
        } else {
            // all its tokens in the same part
            let mut sets: Vec<HashMap<u32, Vec<u32>>> = Vec::new();
            for t in &toks {
                let mut m: HashMap<u32, Vec<u32>> = HashMap::new();
                for &(e, n) in self.map.get(t).map(Vec::as_slice).unwrap_or_default() {
                    m.entry(e).or_default().push(n);
                }
                sets.push(m);
            }
            sets.sort_by_key(|m| m.values().map(Vec::len).sum::<usize>());
            let (first, rest) = sets.split_first().unwrap();
            for (e, parts) in first {
                let keep: Vec<u32> = parts.iter().copied().filter(|n| rest.iter().all(|m| m.get(e).is_some_and(|v| v.contains(n)))).collect();
                if !keep.is_empty() {
                    out.insert(*e, keep);
                }
            }
        }
        for v in out.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        out
    }

    /// The first of `parts` whose text really holds `word`, as (unit, part number, words around it).
    pub fn find(&self, entry: usize, parts: &[u32], word: &str) -> Option<(String, u32, String)> {
        let (id, unit) = self.files.get(&(entry as u32))?;
        let ft = FullText::load(&self.dir, id)?;
        let w = word.to_lowercase();
        for p in ft.parts.iter().filter(|p| parts.contains(&p.n)) {
            let lower = p.text.to_lowercase();
            if let Some(at) = lower.find(&w) {
                return Some((unit.clone(), p.n, crate::search::snippet(&p.text, &lower, at, w.len())));
            }
        }
        None
    }
}

fn distinct_entries(p: &Postings) -> usize {
    let mut n = 0;
    let mut last = u32::MAX;
    for &(e, _) in p {
        if e != last {
            n += 1;
            last = e;
        }
    }
    n
}

/// How a part is called in a sentence: 第 3 頁 / 第 3 張投影片 / 第 3 段.
pub fn where_label(unit: &str, n: u32) -> String {
    match unit {
        "page" => format!("第 {n} 頁"),
        "slide" => format!("第 {n} 張投影片"),
        _ => format!("第 {n} 段"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_catalog::fulltext::Part;

    fn save(dir: &Path, id: &str, unit: &str, parts: &[(u32, &str)]) {
        FullText { version: 1, unit: unit.into(), parts: parts.iter().map(|(n, t)| Part { n: *n, text: t.to_string() }).collect(), note: None }.save(dir, id).unwrap();
    }

    #[test]
    fn tokens_are_latin_words_and_cjk_pairs() {
        assert_eq!(tokens("Achilles 肌腱的力學, x y2"), vec!["achilles", "肌腱", "腱的", "的力", "力學", "y2"]);
    }

    #[test]
    fn a_word_on_page_30_is_found_with_its_page() {
        let t = tempfile::tempdir().unwrap();
        save(t.path(), "a", "page", &[(1, "Biomechanics notes"), (30, "The Achilles tendon (跟腱) stores elastic energy.")]);
        save(t.path(), "b", "slide", &[(2, "Gait cycle and 跟腱 loading")]);
        let ix = FullIndex::load(t.path(), [(0, "a".to_string()), (1, "b".to_string()), (2, "missing".to_string())].into_iter());
        assert!(ix.has(0) && ix.has(1) && !ix.has(2));
        let c = ix.candidates("tendon");
        assert_eq!(c.get(&0), Some(&vec![30]));
        assert_eq!(ix.candidates("achil").get(&0), Some(&vec![30]), "a word it starts");
        let c = ix.candidates("跟腱");
        assert_eq!((c.get(&0), c.get(&1)), (Some(&vec![30]), Some(&vec![2])));
        assert_eq!(ix.files_with("跟腱"), 2);
        let (unit, n, snip) = ix.find(0, &[30], "achilles tendon").unwrap();
        assert_eq!((unit.as_str(), n), ("page", 30));
        assert!(snip.contains("Achilles tendon"));
        assert_eq!(where_label("slide", 2), "第 2 張投影片");
        assert!(ix.candidates("nothing-here").is_empty());
    }
}
