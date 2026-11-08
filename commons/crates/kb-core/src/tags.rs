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
}
