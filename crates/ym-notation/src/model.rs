//! The registry itself: `notation.json`.
//!
//! One file per document (`<paper>/notation.json`) and one global file in the
//! profile (`profile/notation.json`) for the author's habitual symbols. The
//! document file wins; entries are copied from the global file explicitly
//! (`Notation::import`), never merged in silently on save.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Symbol,
    Operator,
    Equation,
    Figure,
    Table,
    Abbrev,
    Term,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Symbol, Kind::Operator, Kind::Equation, Kind::Figure, Kind::Table, Kind::Abbrev, Kind::Term];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Symbol => "symbol",
            Kind::Operator => "operator",
            Kind::Equation => "equation",
            Kind::Figure => "figure",
            Kind::Table => "table",
            Kind::Abbrev => "abbrev",
            Kind::Term => "term",
        }
    }
}

/// One row of the table. Every field but `id` and `kind` is optional; which
/// ones matter depends on the kind (see the README).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub kind: Option<Kind>,
    /// LaTeX of the symbol / operator / equation body, e.g. `\boldsymbol{m}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latex: Option<String>,
    /// Macro name without the backslash, e.g. `ribmask` → `\ribmask`.
    #[serde(default, rename = "macro", skip_serializing_if = "Option::is_none")]
    pub macro_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning_en: Option<String>,
    /// Short spoken-style note used next to equations on slides (design §5 v0.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gloss: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Where the text defines it: `file:line` or a label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defined_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Image file for figures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// Abbreviation: short form (`CXR`) and long form (`chest radiograph`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long: Option<String>,
    /// Ids of entries this one uses (an equation's variables).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
    /// Knowledge card id (kb-core).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb: Option<String>,
    /// Free tags. Recognised: `conventional` / `well-known` (no definition or
    /// expansion needed), `auto` (made by auto-extract), `needs-expansion`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Entry {
    pub fn new(id: &str, kind: Kind) -> Self {
        Entry { id: id.to_string(), kind: Some(kind), ..Default::default() }
    }

    pub fn kind(&self) -> Kind {
        self.kind.unwrap_or(Kind::Symbol)
    }

    /// Macro name without a leading backslash.
    pub fn macro_name(&self) -> Option<&str> {
        self.macro_name.as_deref().map(|m| m.trim().trim_start_matches('\\')).filter(|m| !m.is_empty())
    }

    pub fn has_tag(&self, t: &str) -> bool {
        self.tags.iter().any(|x| x.eq_ignore_ascii_case(t))
    }

    /// Needs no definition / expansion in the text.
    pub fn is_conventional(&self) -> bool {
        self.has_tag("conventional") || self.has_tag("well-known")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notation {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

fn version() -> u32 {
    FORMAT_VERSION
}

impl Default for Notation {
    fn default() -> Self {
        Notation { version: FORMAT_VERSION, entries: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub id: String,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Keep the document's entry (default).
    KeepDocument,
    /// Replace the document's entry by the global one.
    TakeGlobal,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    pub added: Vec<String>,
    pub replaced: Vec<String>,
    /// Same id or same LaTeX already in the document and kept.
    pub kept: Vec<String>,
}

/// Where an entry of the effective (merged) view comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Document,
    Global,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedEntry {
    pub origin: Origin,
    #[serde(flatten)]
    pub entry: Entry,
}

impl Notation {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("notation.json: {e}"))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into()) + "\n"
    }

    /// Missing file → empty registry.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_json(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Atomic write (temp file + rename). notation.json is the app's own file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::write_atomic(path, &self.to_json()).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// Insert or replace by id.
    pub fn upsert(&mut self, e: Entry) {
        match self.entries.iter_mut().find(|x| x.id == e.id) {
            Some(x) => *x = e,
            None => self.entries.push(e),
        }
    }

    pub fn remove(&mut self, id: &str) -> Option<Entry> {
        let i = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(i))
    }

    /// Entry whose LaTeX means the same symbol as `latex` (canonical compare).
    pub fn find_latex(&self, latex: &str) -> Option<&Entry> {
        let key = crate::math::canonical(latex);
        self.entries.iter().find(|e| e.latex.as_deref().map(crate::math::canonical).as_deref() == Some(key.as_str()))
    }

    /// A free id based on `base` (`base`, `base-2`, …).
    pub fn free_id(&self, base: &str) -> String {
        let base = if base.is_empty() { "entry" } else { base };
        if self.get(base).is_none() {
            return base.to_string();
        }
        (2..).map(|n| format!("{base}-{n}")).find(|c| self.get(c).is_none()).unwrap()
    }

    /// Copy entries from the global registry. `ids = None` copies all.
    /// An entry already present (same id, or same canonical LaTeX / same
    /// abbreviation) is kept unless `conflict` says otherwise.
    pub fn import(&mut self, global: &Notation, ids: Option<&[&str]>, conflict: Conflict) -> ImportReport {
        let mut rep = ImportReport::default();
        for g in &global.entries {
            if let Some(ids) = ids
                && !ids.contains(&g.id.as_str()) {
                    continue;
                }
            let existing = self.entries.iter().position(|d| same_thing(d, g));
            match (existing, conflict) {
                (None, _) => {
                    self.entries.push(g.clone());
                    rep.added.push(g.id.clone());
                }
                (Some(i), Conflict::TakeGlobal) => {
                    self.entries[i] = g.clone();
                    rep.replaced.push(g.id.clone());
                }
                (Some(_), Conflict::KeepDocument) => rep.kept.push(g.id.clone()),
            }
        }
        rep
    }

    /// The effective view used for checks and the panel: document entries,
    /// then global entries the document does not already cover.
    pub fn merged(global: &Notation, doc: &Notation) -> Vec<MergedEntry> {
        let mut out: Vec<MergedEntry> = doc.entries.iter().map(|e| MergedEntry { origin: Origin::Document, entry: e.clone() }).collect();
        for g in &global.entries {
            if !doc.entries.iter().any(|d| same_thing(d, g)) {
                out.push(MergedEntry { origin: Origin::Global, entry: g.clone() });
            }
        }
        out
    }

    /// The merged view as a plain registry (for checks).
    pub fn effective(global: &Notation, doc: &Notation) -> Notation {
        Notation { version: FORMAT_VERSION, entries: Self::merged(global, doc).into_iter().map(|m| m.entry).collect() }
    }

    /// Structural problems: ids, macro names, kind-specific fields.
    /// `existing_macros` are commands the document already defines
    /// (see [`crate::scan::document_macros`]).
    pub fn validate(&self, existing_macros: &BTreeSet<String>) -> Vec<Problem> {
        let mut out = Vec::new();
        let mut ids = BTreeMap::new();
        let mut macros: BTreeMap<String, String> = BTreeMap::new();
        let p = |id: &str, field: &str, message: String| Problem { id: id.into(), field: field.into(), message };
        for e in &self.entries {
            if e.id.trim().is_empty() {
                out.push(p(&e.id, "id", "id 不可空白".into()));
            }
            if ids.insert(e.id.clone(), ()).is_some() {
                out.push(p(&e.id, "id", format!("id「{}」重複", e.id)));
            }
            if let Some(m) = e.macro_name() {
                if let Err(msg) = crate::macros::check_macro_name(m, existing_macros) {
                    out.push(p(&e.id, "macro", msg));
                }
                if let Some(other) = macros.insert(m.to_string(), e.id.clone()) {
                    out.push(p(&e.id, "macro", format!("巨集 \\{m} 與「{other}」重複")));
                }
            }
            match e.kind() {
                Kind::Symbol | Kind::Operator | Kind::Equation => {
                    if e.latex.as_deref().unwrap_or("").trim().is_empty() {
                        out.push(p(&e.id, "latex", "缺少 LaTeX".into()));
                    } else if !crate::macros::balanced(e.latex.as_deref().unwrap_or("")) {
                        out.push(p(&e.id, "latex", "LaTeX 大括號不成對".into()));
                    }
                }
                Kind::Abbrev => {
                    if e.short.as_deref().unwrap_or("").trim().is_empty() {
                        out.push(p(&e.id, "short", "縮寫缺少 short".into()));
                    }
                }
                Kind::Figure | Kind::Table => {
                    if e.label.as_deref().unwrap_or("").trim().is_empty() {
                        out.push(p(&e.id, "label", "圖表缺少 label".into()));
                    }
                }
                Kind::Term => {}
            }
        }
        out
    }
}

/// Two entries describe the same thing: same id, same canonical LaTeX
/// (symbols/operators), or same short form (abbreviations).
fn same_thing(a: &Entry, b: &Entry) -> bool {
    if a.id == b.id {
        return true;
    }
    if a.kind() != b.kind() {
        return false;
    }
    match a.kind() {
        Kind::Symbol | Kind::Operator => match (&a.latex, &b.latex) {
            (Some(x), Some(y)) => crate::math::canonical(x) == crate::math::canonical(y),
            _ => false,
        },
        Kind::Abbrev => a.short.is_some() && a.short == b.short,
        _ => false,
    }
}
