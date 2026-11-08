//! Card kinds and link types: built-ins + the space's `kinds.toml` + kinds and link
//! types declared by apps in `apps.toml` (design §2.1: a new app declares what it uses,
//! the knowledge base needs no code change).
//!
//! ```toml
//! # kinds.toml
//! [kinds.equation]
//! label = "方程式"
//! fields = [{ name = "latex", label = "LaTeX", type = "text" }]
//!
//! # apps.toml
//! [apps.slidecomposer]
//! label = "SlideComposer"
//! link_types = [{ id = "illustrates", label = "圖解" }]
//! [apps.slidecomposer.kinds.figure]
//! label = "圖"
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    #[serde(default)]
    pub label: String,
    /// text | markdown | latex | number | list | url
    #[serde(rename = "type", default = "text_type")]
    pub kind: String,
}

fn text_type() -> String {
    "text".into()
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct KindDef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
    /// Card layout hint for the UI (e.g. `image-first`, `equation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// `builtin`, `kinds.toml` or `app:<id>`.
    #[serde(default)]
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkType {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub origin: String,
}

#[derive(Debug, Deserialize, Default)]
struct KindsFile {
    #[serde(default)]
    kinds: BTreeMap<String, KindDef>,
}

#[derive(Debug, Deserialize, Default)]
struct AppDecl {
    #[serde(default)]
    #[allow(dead_code)]
    label: String,
    #[serde(default)]
    kinds: BTreeMap<String, KindDef>,
    #[serde(default)]
    link_types: Vec<LinkType>,
}

#[derive(Debug, Deserialize, Default)]
struct AppsFile {
    #[serde(default)]
    apps: BTreeMap<String, AppDecl>,
}

pub const BUILTIN_KINDS: &[(&str, &str)] = &[
    ("concept", "概念"),
    ("term", "術語"),
    ("method", "方法"),
    ("equation", "方程式"),
    ("anatomy", "解剖"),
    ("dataset", "資料集"),
    ("metric", "指標"),
    ("paper", "論文"),
    ("person", "人物"),
    ("vocab", "單字"),
];

pub const BUILTIN_LINK_TYPES: &[(&str, &str)] = &[
    ("is-a", "是一種"),
    ("part-of", "屬於"),
    ("prerequisite", "先備知識"),
    ("contrast", "對照"),
    ("example-of", "例子"),
    ("uses", "用到"),
    ("translation", "翻譯"),
    ("related", "相關"),
];

#[derive(Debug, Clone, Serialize)]
pub struct Registry {
    pub kinds: Vec<KindDef>,
    pub link_types: Vec<LinkType>,
}

impl Default for Registry {
    fn default() -> Registry {
        Registry {
            kinds: BUILTIN_KINDS.iter().map(|(id, label)| KindDef { id: id.to_string(), label: label.to_string(), origin: "builtin".into(), ..Default::default() }).collect(),
            link_types: BUILTIN_LINK_TYPES.iter().map(|(id, label)| LinkType { id: id.to_string(), label: label.to_string(), origin: "builtin".into() }).collect(),
        }
    }
}

impl Registry {
    /// Built-ins + `<space>/kinds.toml` + `<space>/apps.toml` (both optional).
    pub fn load(space_root: &Path) -> Result<Registry> {
        let mut r = Registry::default();
        let kinds = space_root.join("kinds.toml");
        if kinds.is_file() {
            let f: KindsFile = toml::from_str(&std::fs::read_to_string(kinds)?)?;
            for (id, k) in f.kinds {
                r.add_kind(id, k, "kinds.toml");
            }
        }
        let apps = space_root.join("apps.toml");
        if apps.is_file() {
            let f: AppsFile = toml::from_str(&std::fs::read_to_string(apps)?)?;
            for (app, decl) in f.apps {
                let origin = format!("app:{app}");
                for (id, k) in decl.kinds {
                    r.add_kind(id, k, &origin);
                }
                for mut lt in decl.link_types {
                    if !r.has_link_type(&lt.id) {
                        lt.origin = origin.clone();
                        r.link_types.push(lt);
                    }
                }
            }
        }
        Ok(r)
    }

    /// A later definition of the same id adds fields / overrides the label.
    fn add_kind(&mut self, id: String, mut k: KindDef, origin: &str) {
        k.id = id.clone();
        if let Some(existing) = self.kinds.iter_mut().find(|e| e.id == id) {
            if !k.label.is_empty() {
                existing.label = k.label;
            }
            if k.icon.is_some() {
                existing.icon = k.icon;
            }
            if k.layout.is_some() {
                existing.layout = k.layout;
            }
            for f in k.fields {
                if !existing.fields.iter().any(|e| e.name == f.name) {
                    existing.fields.push(f);
                }
            }
        } else {
            if k.label.is_empty() {
                k.label = id.clone();
            }
            k.origin = origin.to_string();
            self.kinds.push(k);
        }
    }

    pub fn has_kind(&self, id: &str) -> bool {
        self.kinds.iter().any(|k| k.id == id)
    }

    pub fn has_link_type(&self, id: &str) -> bool {
        self.link_types.iter().any(|l| l.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_plus_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("kinds.toml"),
            "[kinds.equation]\nfields = [{ name = \"latex\", label = \"LaTeX\" }]\n[kinds.theorem]\nlabel = \"定理\"\n",
        )
        .unwrap();
        std::fs::write(
            d.path().join("apps.toml"),
            "[apps.slidecomposer]\nlabel = \"SlideComposer\"\nlink_types = [{ id = \"illustrates\", label = \"圖解\" }, { id = \"related\" }]\n[apps.slidecomposer.kinds.figure]\nlabel = \"圖\"\n",
        )
        .unwrap();
        let r = Registry::load(d.path()).unwrap();
        assert!(r.has_kind("concept") && r.has_kind("theorem") && r.has_kind("figure"));
        let eq = r.kinds.iter().find(|k| k.id == "equation").unwrap();
        assert_eq!(eq.label, "方程式");
        assert_eq!(eq.fields[0].kind, "text");
        assert!(r.has_link_type("illustrates") && r.has_link_type("prerequisite"));
        assert_eq!(r.link_types.iter().filter(|l| l.id == "related").count(), 1);
        assert_eq!(r.kinds.iter().find(|k| k.id == "figure").unwrap().origin, "app:slidecomposer");
        assert!(Registry::load(&d.path().join("missing")).unwrap().has_kind("vocab"));
    }
}
