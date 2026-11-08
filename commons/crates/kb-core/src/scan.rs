//! Term scanner: find known cards (titles + aliases) in a text with Aho-Corasick.
//!
//! - Case-insensitive (Unicode lowercase, `Sørensen–Dice` matches `sørensen–dice`).
//! - English plurals: a name ending in a Latin letter also matches `+s`, `+es`,
//!   `y → ies` (`U-Nets`, `metrics`, `ontologies`).
//! - Latin names need word boundaries (`DSC` does not match inside `DSCX`); CJK names
//!   match anywhere (no spaces in Chinese / Japanese).
//! - Leftmost-longest: `Dice coefficient` wins over `Dice`.
//!
//! Spans carry UTF-16 offsets (`start`/`end`, what JavaScript uses) and byte offsets.

use std::collections::{HashMap, HashSet};

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ScanOptions {
    /// Only cards in these spaces (empty = all mounted spaces).
    pub spaces: Vec<String>,
    /// Ask the assistant for candidate terms that have no card yet (ym-llm later).
    pub candidates: bool,
    /// Record an `encountered` event for every matched card (first occurrence).
    pub record: bool,
    /// Who is reading (`reader`, `slides`, …) for the encounter log.
    pub via: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScanSpan {
    /// UTF-16 offsets.
    pub start: usize,
    pub end: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    /// The text as written.
    pub text: String,
    pub node_id: String,
    pub space: String,
    pub title: String,
    /// learning | known | stale | none (no learning state for this member)
    pub state: String,
    /// First occurrence of this card in the text.
    pub first: bool,
    /// Other cards with the same name (another space).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<String>,
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF | 0x20000..=0x2FFFF | 0x3000..=0x303F | 0xFF00..=0xFFEF)
}

/// A letter or digit that is part of a Latin-script word.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() && !is_cjk(c)
}

fn plural_forms(lower: &str) -> Vec<String> {
    let mut out = vec![lower.to_string()];
    let Some(last) = lower.chars().last() else { return out };
    if !last.is_ascii_alphabetic() || lower.chars().count() < 3 {
        return out;
    }
    let before = lower.chars().rev().nth(1).unwrap_or(' ');
    if lower.ends_with("ss") || lower.ends_with('x') || lower.ends_with('z') || lower.ends_with("ch") || lower.ends_with("sh") {
        out.push(format!("{lower}es"));
    } else if last == 'y' && !"aeiou".contains(before) {
        out.push(format!("{}ies", &lower[..lower.len() - 1]));
    } else if last != 's' {
        out.push(format!("{lower}s"));
    }
    out
}

struct Entry {
    space: String,
    id: String,
    title: String,
}

/// Lowercased text plus a map from each lowercased byte back to the original char.
struct Folded {
    text: String,
    /// For every byte of `text`: (original byte start, original byte end) of its char.
    map: Vec<(usize, usize)>,
}

fn fold(s: &str) -> Folded {
    let mut text = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        let end = i + c.len_utf8();
        for l in c.to_lowercase() {
            let mut buf = [0u8; 4];
            let enc = l.encode_utf8(&mut buf);
            text.push_str(enc);
            map.extend(std::iter::repeat_n((i, end), enc.len()));
        }
    }
    Folded { text, map }
}

pub struct Scanner {
    ac: Option<AhoCorasick>,
    /// pattern index → entries (several cards may share a name).
    targets: Vec<Vec<usize>>,
    /// pattern needs Latin word boundaries at (start, end).
    bounds: Vec<(bool, bool)>,
    entries: Vec<Entry>,
}

impl Scanner {
    /// `names`: `(id, space, title, name)` rows (see `Index::names`), in preference
    /// order (earlier wins when two cards share a name).
    pub fn build(names: &[(String, String, String, String)], spaces: &[String]) -> Result<Scanner> {
        let mut patterns: Vec<String> = Vec::new();
        let mut by_pattern: HashMap<String, usize> = HashMap::new();
        let mut targets: Vec<Vec<usize>> = Vec::new();
        let mut bounds = Vec::new();
        let mut entries: Vec<Entry> = Vec::new();
        let mut entry_of: HashMap<String, usize> = HashMap::new();
        for (id, space, title, name) in names {
            if !spaces.is_empty() && !spaces.contains(space) {
                continue;
            }
            let lower = name.trim().to_lowercase();
            if lower.chars().count() < 2 && !lower.chars().any(is_cjk) {
                continue;
            }
            let e = *entry_of.entry(id.clone()).or_insert_with(|| {
                entries.push(Entry { space: space.clone(), id: id.clone(), title: title.clone() });
                entries.len() - 1
            });
            let first = lower.chars().next().unwrap_or(' ');
            let last = lower.chars().last().unwrap_or(' ');
            for p in plural_forms(&lower) {
                let k = *by_pattern.entry(p.clone()).or_insert_with(|| {
                    patterns.push(p.clone());
                    targets.push(Vec::new());
                    bounds.push((is_word(first), is_word(last)));
                    patterns.len() - 1
                });
                if !targets[k].contains(&e) {
                    targets[k].push(e);
                }
            }
        }
        let ac = if patterns.is_empty() {
            None
        } else {
            Some(AhoCorasickBuilder::new().match_kind(MatchKind::LeftmostLongest).build(&patterns).map_err(|e| crate::KbError::Invalid(e.to_string()))?)
        };
        Ok(Scanner { ac, targets, bounds, entries })
    }

    pub fn is_empty(&self) -> bool {
        self.ac.is_none()
    }

    /// Spans of known cards. `states`: this member's state per card id.
    pub fn scan(&self, text: &str, states: &HashMap<String, String>) -> Vec<ScanSpan> {
        let Some(ac) = &self.ac else { return vec![] };
        let folded = fold(text);
        let mut seen: HashSet<usize> = HashSet::new();
        let mut out = Vec::new();
        // UTF-16 offset of every byte boundary, computed lazily in order.
        let mut u16_at = vec![0usize; text.len() + 1];
        {
            let mut u = 0;
            for (i, c) in text.char_indices() {
                for b in 0..c.len_utf8() {
                    u16_at[i + b] = u;
                }
                u += c.len_utf16();
            }
            u16_at[text.len()] = u;
        }
        for m in ac.find_iter(&folded.text) {
            let (bs, _) = folded.map[m.start()];
            let (_, be) = folded.map[m.end() - 1];
            let (need_l, need_r) = self.bounds[m.pattern().as_usize()];
            if need_l && text[..bs].chars().next_back().is_some_and(is_word) {
                continue;
            }
            if need_r && text[be..].chars().next().is_some_and(is_word) {
                continue;
            }
            let es = &self.targets[m.pattern().as_usize()];
            let e = &self.entries[es[0]];
            out.push(ScanSpan {
                start: u16_at[bs],
                end: u16_at[be],
                byte_start: bs,
                byte_end: be,
                text: text[bs..be].to_string(),
                node_id: e.id.clone(),
                space: e.space.clone(),
                title: e.title.clone(),
                state: states.get(&e.id).cloned().unwrap_or_else(|| "none".into()),
                first: seen.insert(es[0]),
                also: es[1..].iter().map(|i| self.entries[*i].id.clone()).collect(),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<(String, String, String, String)> {
        let mut v = Vec::new();
        let mut add = |id: &str, title: &str, names: &[&str]| {
            for n in std::iter::once(&title).chain(names) {
                v.push((id.to_string(), "me".to_string(), title.to_string(), n.to_string()));
            }
        };
        add("n1", "Dice coefficient", &["DSC", "Dice score", "Sørensen–Dice"]);
        add("n2", "U-Net", &["UNet"]);
        add("n3", "膝關節", &["knee joint"]);
        add("n4", "ontology", &[]);
        add("n5", "Dice", &[]);
        add("n6", "box", &[]);
        v
    }

    #[test]
    fn finds_aliases_plurals_cjk() {
        let s = Scanner::build(&rows(), &[]).unwrap();
        let states: HashMap<String, String> = [("n1".to_string(), "learning".to_string())].into();
        let text = "We report DSC and the dice coefficient of two U-Nets on 膝關節 MRI; ontologies, boxes. DSCX no. Sørensen–Dice again, dsc.";
        let spans = s.scan(text, &states);
        let got: Vec<(&str, &str, bool)> = spans.iter().map(|s| (s.text.as_str(), s.node_id.as_str(), s.first)).collect();
        assert_eq!(
            got,
            vec![
                ("DSC", "n1", true),
                ("dice coefficient", "n1", false),
                ("U-Nets", "n2", true),
                ("膝關節", "n3", true),
                ("ontologies", "n4", true),
                ("boxes", "n6", true),
                ("Sørensen–Dice", "n1", false),
                ("dsc", "n1", false),
            ]
        );
        assert_eq!(spans[0].state, "learning");
        assert_eq!(spans[2].state, "none");
        // UTF-16 offsets line up with JavaScript string indices.
        let u: Vec<u16> = text.encode_utf16().collect();
        for sp in &spans {
            assert_eq!(String::from_utf16(&u[sp.start..sp.end]).unwrap(), sp.text);
        }
    }

    #[test]
    fn cjk_inside_sentence_and_space_filter() {
        let s = Scanner::build(&rows(), &["other".to_string()]).unwrap();
        assert!(s.is_empty());
        let s = Scanner::build(&rows(), &[]).unwrap();
        let spans = s.scan("右膝關節疼痛", &HashMap::new());
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (1, 4));
    }
}
