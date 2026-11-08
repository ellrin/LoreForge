//! A card = one JSON file with content only (design §4.1). Learning state lives
//! elsewhere ([`crate::progress`]). Unknown fields (kind-specific extras from
//! `kinds.toml`, newer app fields) are kept in `extra` and written back unchanged.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::clock::Time;

fn is_empty_str(s: &str) -> bool {
    s.is_empty()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// `sha256:<hex>`
    pub asset: String,
    pub ext: String,
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub caption: String,
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub credit: String,
    /// Region marks (arrows, boxes) drawn over the image; stored as JSON, the image
    /// itself is never changed.
    #[serde(default)]
    pub marks: Vec<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub latex: String,
    #[serde(default)]
    pub meaning: String,
}

/// An explicit typed edge. `rel` is a built-in link type or one declared in `apps.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub to: String,
    pub rel: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "ref", default)]
    pub reference: String,
    #[serde(rename = "where", default, skip_serializing_if = "is_empty_str")]
    pub location: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A key in another app (`{"app": "jelearner", "key": "en|coefficient|"}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct External {
    pub app: String,
    pub key: String,
}

/// Where an explicit share copied this card from / to. Only ever stored in a
/// personal space (a household space never names personal ids).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyRef {
    pub space: String,
    pub id: String,
    /// `to` = this card was shared into `space`; `from` = this card is a copy of it.
    pub dir: String,
    pub at: Time,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub lang: String,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub images: Vec<Image>,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub external: Vec<External>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<CopyRef>,
    pub created_at: Time,
    pub updated_at: Time,
    #[serde(default = "one")]
    pub revision: u64,
    /// Kind-specific fields and anything this version does not know.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn one() -> u64 {
    1
}

impl Node {
    /// Title and aliases, trimmed, without empties.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str()).chain(self.aliases.iter().map(|s| s.as_str())).map(str::trim).filter(|s| !s.is_empty())
    }
}

/// Fields for creating or editing a card. `None` = leave unchanged (edit) or default
/// (create). `extra` keys are merged; a `null` value removes the key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeFields {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub lang: Option<String>,
    pub domains: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub summary: Option<String>,
    pub body: Option<String>,
    pub images: Option<Vec<Image>>,
    pub symbols: Option<Vec<Symbol>>,
    pub links: Option<Vec<Link>>,
    pub sources: Option<Vec<Source>>,
    pub external: Option<Vec<External>>,
    #[serde(default)]
    pub extra: Map<String, Value>,
}

/// Keys that are part of the schema and may not be set through `extra`.
const RESERVED: &[&str] = &[
    "id", "kind", "title", "aliases", "lang", "domains", "tags", "summary", "body", "images", "symbols", "links", "sources", "external", "copies", "created_at", "updated_at", "revision",
];

fn clean_list(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        let s = s.trim().to_string();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

fn clean_tags(v: Vec<String>) -> Vec<String> {
    clean_list(v.into_iter().map(|t| t.trim().trim_matches('/').split('/').map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/")).collect())
}

impl NodeFields {
    /// Apply to `node` (used by both create and edit).
    pub fn apply_to(self, node: &mut Node) {
        if let Some(v) = self.kind {
            node.kind = v.trim().to_string();
        }
        if let Some(v) = self.title {
            node.title = v.trim().to_string();
        }
        if let Some(v) = self.aliases {
            node.aliases = clean_list(v).into_iter().filter(|a| *a != node.title).collect();
        }
        if let Some(v) = self.lang {
            node.lang = v.trim().to_string();
        }
        if let Some(v) = self.domains {
            node.domains = clean_tags(v);
        }
        if let Some(v) = self.tags {
            node.tags = clean_tags(v);
        }
        if let Some(v) = self.summary {
            node.summary = v;
        }
        if let Some(v) = self.body {
            node.body = v;
        }
        if let Some(v) = self.images {
            node.images = v;
        }
        if let Some(v) = self.symbols {
            node.symbols = v;
        }
        if let Some(v) = self.links {
            let mut links: Vec<Link> = Vec::new();
            for l in v {
                if !links.iter().any(|x| x.to == l.to && x.rel == l.rel) {
                    links.push(l);
                }
            }
            node.links = links;
        }
        if let Some(v) = self.sources {
            node.sources = v;
        }
        if let Some(v) = self.external {
            node.external = v;
        }
        for (k, v) in self.extra {
            if RESERVED.contains(&k.as_str()) {
                continue;
            }
            if v.is_null() {
                node.extra.remove(&k);
            } else {
                node.extra.insert(k, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_design_example_keeps_unknown_fields() {
        let json = r#"{
  "id": "n01JCB7Q3T2W8XAAAAAAAAAAAA",
  "kind": "concept",
  "title": "Dice coefficient",
  "aliases": ["DSC", "Dice score", "Sørensen–Dice"],
  "lang": "en",
  "domains": ["medical-image-analysis"],
  "tags": ["metric", "segmentation"],
  "summary": "衡量兩個集合重疊程度的指標，常用於分割評估。",
  "body": "定義：$\\mathrm{DSC}=\\frac{2|A\\cap B|}{|A|+|B|}$，可用 [[IoU]]",
  "images": [{ "asset": "sha256:9f2c", "ext": "png", "caption": "Dice 與 IoU", "credit": "自繪", "marks": [] }],
  "symbols": [{ "latex": "A", "meaning": "預測區域" }],
  "links": [{ "to": "n01JCB8", "rel": "contrast" }],
  "sources": [{ "type": "paper", "ref": "doi:10.1007/x", "where": "Sec. 4.2" }],
  "external": [{ "app": "jelearner", "key": "en|coefficient|" }],
  "created_at": "2026-11-08T21:10:00+08:00",
  "updated_at": "2026-11-09T09:02:00+08:00",
  "revision": 4,
  "latex": "\\frac{2|A\\cap B|}{|A|+|B|}"
}"#;
        let n: Node = serde_json::from_str(json).unwrap();
        assert_eq!(n.revision, 4);
        assert_eq!(n.sources[0].location, "Sec. 4.2");
        assert!(n.extra.contains_key("latex"));
        let back: Node = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(back, n);
        assert_eq!(n.names().collect::<Vec<_>>(), vec!["Dice coefficient", "DSC", "Dice score", "Sørensen–Dice"]);
    }

    #[test]
    fn fields_clean_up() {
        let mut n: Node = serde_json::from_str(r#"{"id":"n1","kind":"concept","title":"x","created_at":"2026-11-08T21:10:00+08:00","updated_at":"2026-11-08T21:10:00+08:00"}"#).unwrap();
        let f = NodeFields {
            title: Some(" U-Net ".into()),
            aliases: Some(vec!["UNet".into(), "".into(), "UNet".into(), "U-Net".into()]),
            tags: Some(vec!["/deep-learning/ cnn/".into()]),
            extra: [("revision".to_string(), Value::from(99)), ("latex".to_string(), Value::from("x"))].into_iter().collect(),
            ..Default::default()
        };
        f.apply_to(&mut n);
        assert_eq!(n.title, "U-Net");
        assert_eq!(n.aliases, vec!["UNet"]);
        assert_eq!(n.tags, vec!["deep-learning/cnn"]);
        assert_eq!(n.revision, 1);
        assert_eq!(n.extra["latex"], "x");
    }
}
