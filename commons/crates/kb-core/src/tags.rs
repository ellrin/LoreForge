//! `tags.json`: domains and the tag hierarchy (`deep-learning/cnn` is a child of
//! `deep-learning`). A tag can be promoted to a card by naming it in `node`.
//!
//! ```json
//! { "version": 1,
//!   "domains": { "medical-image-analysis": { "label": "醫學影像分析" } },
//!   "tags": { "deep-learning/cnn": { "label": "CNN" } } }
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TagInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The card this tag was promoted to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TagsFile {
    #[serde(default = "v1")]
    pub version: u32,
    #[serde(default)]
    pub domains: BTreeMap<String, TagInfo>,
    #[serde(default)]
    pub tags: BTreeMap<String, TagInfo>,
}

fn v1() -> u32 {
    1
}

impl TagsFile {
    pub fn load(space_root: &Path) -> Result<TagsFile> {
        let p = space_root.join("tags.json");
        if !p.is_file() {
            return Ok(TagsFile { version: 1, ..Default::default() });
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?)
    }

    /// Written atomically to `<space>/tags.json` (pretty, newline at the end).
    pub fn save(&self, space_root: &Path) -> Result<()> {
        let mut s = serde_json::to_string_pretty(self)?;
        s.push('\n');
        lore_store::write_atomic(&space_root.join("tags.json"), s.as_bytes())?;
        Ok(())
    }

    /// A new vocabulary: `domains` and `tags` replace the old ones (labels from the new
    /// entries); a tag that stays keeps the card it was promoted to.
    pub fn replace_vocabulary(&mut self, domains: BTreeMap<String, TagInfo>, tags: BTreeMap<String, TagInfo>) {
        let old = std::mem::take(&mut self.tags);
        self.domains = domains;
        self.tags = tags
            .into_iter()
            .map(|(k, mut v)| {
                if v.node.is_none() {
                    v.node = old.get(&k).and_then(|o| o.node.clone());
                }
                (k, v)
            })
            .collect();
    }
}

/// A domain or tag path: lowercase ASCII words joined by `-`, levels by `/`.
pub fn valid_path(p: &str) -> bool {
    !p.is_empty() && p.split('/').all(|s| !s.is_empty() && !s.starts_with('-') && !s.ends_with('-') && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
}

/// `a/b/c` → `["a", "a/b"]`.
pub fn ancestors(tag: &str) -> Vec<String> {
    let parts: Vec<&str> = tag.split('/').collect();
    (1..parts.len()).map(|i| parts[..i].join("/")).collect()
}

/// `tag` is `query` or below it.
pub fn under(tag: &str, query: &str) -> bool {
    tag == query || (tag.len() > query.len() && tag.starts_with(query) && tag.as_bytes()[query.len()] == b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy() {
        assert_eq!(ancestors("a/b/c"), vec!["a", "a/b"]);
        assert!(ancestors("a").is_empty());
        assert!(under("deep-learning/cnn", "deep-learning"));
        assert!(under("deep-learning", "deep-learning"));
        assert!(!under("deep-learning-x", "deep-learning"));
    }

    #[test]
    fn a_new_vocabulary_keeps_promoted_tags_and_saves() {
        let t = tempfile::tempdir().unwrap();
        let mut f = TagsFile { version: 1, ..Default::default() };
        f.tags.insert("a/x".into(), TagInfo { label: Some("舊".into()), node: Some("n01".into()) });
        f.tags.insert("a/gone".into(), TagInfo::default());
        let info = |l: &str| TagInfo { label: Some(l.into()), node: None };
        f.replace_vocabulary([("a".to_string(), info("甲"))].into(), [("a/x".to_string(), info("新")), ("a/y".to_string(), info("乙"))].into());
        assert_eq!(f.tags["a/x"], TagInfo { label: Some("新".into()), node: Some("n01".into()) });
        assert!(!f.tags.contains_key("a/gone"));
        f.save(t.path()).unwrap();
        assert_eq!(TagsFile::load(t.path()).unwrap(), f);
        assert!(valid_path("deep-learning/loss") && !valid_path("深度學習") && !valid_path("a//b") && !valid_path("A"));
    }
}
