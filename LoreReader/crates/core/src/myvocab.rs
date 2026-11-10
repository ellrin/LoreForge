//! "My vocabulary": the words and terms of the author's own papers, cross-app personal
//! data in `private/shared/my-vocab.json` (found through lore-home; SlideComposer and
//! PaperComposer can read it too). Every entry names its sources so it can be rebuilt:
//! a LoreReader paper's sha256, or a named source such as `import:<name>` for a list
//! built elsewhere. Rebuilding one source never touches the others.
//!
//! ```json
//! { "schema": "lore.my-vocab/1", "updated_at": "…",
//!   "sources": ["<sha256>", "import:<name>"],
//!   "entries": [ { "key": "vessel tortuosity", "text": "vessel tortuosity",
//!                  "sources": ["<sha256>", "import:<name>"] } ] }
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::nouns::{self, Filters, KbName};
use crate::text::{PageText, PaperText};

pub const SCHEMA: &str = "lore.my-vocab/1";
pub const FILE: &str = "my-vocab.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VocabEntry {
    /// The noun list's normalised key (`word_key` / `phrase_key`).
    pub key: String,
    pub text: String,
    pub sources: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MyVocab {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub sources: BTreeSet<String>,
    #[serde(default)]
    pub entries: Vec<VocabEntry>,
}

impl MyVocab {
    pub fn load(path: &Path) -> MyVocab {
        let mut v: MyVocab = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // the first LoreReader version kept the source list as `papers`
        if v.sources.is_empty()
            && let Some(old) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|x| x["papers"].as_array().cloned())
        {
            v.sources = old.iter().filter_map(|s| s.as_str().map(str::to_string)).collect();
        }
        v
    }

    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        self.schema = SCHEMA.into();
        self.updated_at = crate::now();
        let mut b = serde_json::to_vec_pretty(self).unwrap_or_default();
        b.push(b'\n');
        crate::write_atomic(path, &b)
    }

    /// Replace one source's words (`None` removes the source). Entries of other sources
    /// are kept; entries left without a source are dropped.
    pub fn set_source(&mut self, source: &str, words: Option<BTreeMap<String, String>>) {
        let mut map: BTreeMap<String, VocabEntry> = std::mem::take(&mut self.entries).into_iter().map(|e| (e.key.clone(), e)).collect();
        for e in map.values_mut() {
            e.sources.remove(source);
        }
        self.sources.remove(source);
        if let Some(w) = words {
            self.sources.insert(source.to_string());
            for (k, t) in w {
                map.entry(k.clone()).or_insert_with(|| VocabEntry { key: k, text: t, sources: BTreeSet::new() }).sources.insert(source.to_string());
            }
        }
        self.entries = map.into_values().filter(|e| !e.sources.is_empty()).collect();
    }

    /// Merge another file (migration): every source of `other` is set here.
    pub fn merge(&mut self, other: &MyVocab) {
        for src in &other.sources {
            let words: BTreeMap<String, String> = other.entries.iter().filter(|e| e.sources.contains(src)).map(|e| (e.key.clone(), e.text.clone())).collect();
            if !self.sources.contains(src) {
                self.set_source(src, Some(words));
            }
        }
    }

    pub fn keys(&self) -> std::collections::HashSet<String> {
        self.entries.iter().map(|e| e.key.clone()).collect()
    }

    pub fn texts(&self) -> BTreeMap<String, String> {
        self.entries.iter().map(|e| (e.key.clone(), e.text.clone())).collect()
    }
}

/// The vocabulary of plain paragraphs (an import, no PDF): the same normalisation and
/// n-grams as for a paper, each paragraph on its own lines.
pub fn vocabulary_of_paragraphs(paragraphs: &[String], kb: &[KbName]) -> BTreeMap<String, String> {
    let text = paragraphs.join("\n\n");
    let paper = PaperText { pages: vec![PageText { page: 1, text, ..Default::default() }] };
    nouns::vocabulary(&paper, kb, &Filters::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_rebuilt_independently() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(FILE);
        let mut v = MyVocab::default();
        let w = |ks: &[&str]| ks.iter().map(|k| (k.to_string(), k.to_string())).collect::<BTreeMap<_, _>>();
        v.set_source("import:a", Some(w(&["tortuosity", "vessel tortuosity"])));
        v.set_source(&"ab".repeat(32), Some(w(&["tortuosity", "centerline"])));
        v.save(&p).unwrap();
        let mut v = MyVocab::load(&p);
        assert_eq!(v.schema, SCHEMA);
        assert_eq!(v.entries.iter().find(|e| e.key == "tortuosity").unwrap().sources.len(), 2);
        // rebuilding the paper keeps the import
        v.set_source(&"ab".repeat(32), None);
        assert!(v.keys().contains("vessel tortuosity") && v.keys().contains("tortuosity") && !v.keys().contains("centerline"));
        assert_eq!(v.sources, ["import:a".to_string()].into_iter().collect());
        // merging an old file adds its sources once
        let mut old = MyVocab::default();
        old.set_source("cd".repeat(32).as_str(), Some(w(&["skeletonization"])));
        v.merge(&old);
        v.merge(&old);
        assert!(v.keys().contains("skeletonization") && v.sources.len() == 2);
        let para = vocabulary_of_paragraphs(&["Vessel tortuosity is measured. Vessel tortuosity again.".into()], &[]);
        assert!(para.contains_key("vessel tortuosity"));
    }
}
