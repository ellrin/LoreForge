//! The asset catalog: what both sources (LaTeX, PDF) produce.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Catalog format version (bump on incompatible changes).
pub const SCHEMA: u32 = 1;

/// A rectangle in PDF points, origin at the page's top-left, y grows
/// downward (the convention of `pdftotext -bbox` and of image crops).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl BBox {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> BBox {
        BBox {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn area(&self) -> f64 {
        self.width().max(0.0) * self.height().max(0.0)
    }
    pub fn center_x(&self) -> f64 {
        (self.x0 + self.x1) / 2.0
    }
    pub fn center_y(&self) -> f64 {
        (self.y0 + self.y1) / 2.0
    }
    pub fn union(&self, o: &BBox) -> BBox {
        BBox {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
    pub fn intersect(&self, o: &BBox) -> Option<BBox> {
        let b = BBox {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        };
        (b.x1 > b.x0 && b.y1 > b.y0).then_some(b)
    }
    /// Horizontal overlap length.
    pub fn x_overlap(&self, o: &BBox) -> f64 {
        (self.x1.min(o.x1) - self.x0.max(o.x0)).max(0.0)
    }
    pub fn contains(&self, o: &BBox) -> bool {
        o.x0 >= self.x0 - 0.5
            && o.y0 >= self.y0 - 0.5
            && o.x1 <= self.x1 + 0.5
            && o.y1 <= self.y1 + 0.5
    }
    pub fn pad(&self, p: f64) -> BBox {
        BBox {
            x0: self.x0 - p,
            y0: self.y0 - p,
            x1: self.x1 + p,
            y1: self.y1 + p,
        }
    }
    /// A sub-rectangle given relative to this box's top-left corner (points).
    pub fn sub(&self, rel: &BBox) -> BBox {
        BBox::new(
            self.x0 + rel.x0,
            self.y0 + rel.y0,
            self.x0 + rel.x1,
            self.y0 + rel.y1,
        )
        .intersect(self)
        .unwrap_or(BBox {
            x0: self.x0,
            y0: self.y0,
            x1: self.x0,
            y1: self.y0,
        })
    }
    /// A sub-rectangle given as fractions (0..1) of this box.
    pub fn sub_fraction(&self, fx0: f64, fy0: f64, fx1: f64, fy1: f64) -> BBox {
        let (w, h) = (self.width(), self.height());
        self.sub(&BBox::new(fx0 * w, fy0 * h, fx1 * w, fy1 * h))
    }
    pub fn rounded(&self) -> BBox {
        let r = |v: f64| (v * 100.0).round() / 100.0;
        BBox {
            x0: r(self.x0),
            y0: r(self.y0),
            x1: r(self.x1),
            y1: r(self.y1),
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    #[default]
    Figure,
    Subfigure,
    Table,
    Equation,
    /// A region cut out of another asset (re-crop, split, semantic module box).
    Region,
}

impl AssetKind {
    pub fn prefix(self) -> &'static str {
        match self {
            AssetKind::Figure => "fig",
            AssetKind::Subfigure => "subfig",
            AssetKind::Table => "tab",
            AssetKind::Equation => "eq",
            AssetKind::Region => "reg",
        }
    }
}

/// Where an asset came from: a source file and line (LaTeX) and/or a page (PDF).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

/// One reusable piece of the paper.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    /// Stable within a catalog: `fig1`, `fig1a`, `tab2`, `eq3`, `eq-u4` (unnumbered), `fig1.r1` (region).
    pub id: String,
    pub kind: AssetKind,
    /// Printed number: `1`, `II`, `3a`. None for unnumbered equations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// `\label{…}` key (LaTeX), or the sub-module name for semantic regions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Caption as plain text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// Caption as written (LaTeX mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_latex: Option<String>,
    /// Equation LaTeX (stand-alone), table cells as tab-separated text, or the
    /// text found inside a PDF region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The whole environment as written (LaTeX mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Table cells (LaTeX mode, best effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Vec<Vec<String>>>,
    /// 1-based page number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<BBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_bbox: Option<BBox>,
    /// Regions painted white when the asset is rendered (absolute page coordinates).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub masks: Vec<BBox>,
    /// Graphics files (LaTeX mode), relative to the source root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The asset this one was cut from or belongs to (sub-figure, region).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Section id the asset sits in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default)]
    pub source_ref: SourceRef,
    /// Rendered PNG, relative to the catalog directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<String>,
    /// PDF detection confidence 0..1 (1 for LaTeX-mode assets).
    #[serde(default = "one")]
    pub confidence: f32,
    /// Detection notes (why a region looks the way it does).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

fn one() -> f32 {
    1.0
}

impl Asset {
    pub fn new(id: impl Into<String>, kind: AssetKind) -> Asset {
        Asset {
            id: id.into(),
            kind,
            confidence: 1.0,
            ..Default::default()
        }
    }
}

/// A section with its paragraphs as plain text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Section {
    /// `s1`, `s2.3`, `abstract`, `appendix-a` … unique within the catalog.
    pub id: String,
    /// 1 = section, 2 = subsection, 3 = subsubsection.
    pub level: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Plain-text paragraphs (inline math kept as `$…$`, citations dropped).
    pub paragraphs: Vec<String>,
    #[serde(default)]
    pub source_ref: SourceRef,
}

/// One bibliography entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    /// BibTeX key, or `[n]` / `ref<n>` in PDF mode.
    pub key: String,
    /// Formatted entry as plain text.
    pub text: String,
    /// BibTeX fields (lower-case names), LaTeX mode only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<String>,
    /// Cited in the text (LaTeX mode: `\cite` keys).
    #[serde(default)]
    pub cited: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arxiv_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
}

/// Page geometry (PDF mode).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub number: u32,
    pub width: f64,
    pub height: f64,
    /// Text columns as (x0, x1); one entry for single-column pages.
    pub columns: Vec<(f64, f64)>,
    /// Cached analysis render, relative to the catalog directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocSource {
    Latex { root: String, main: String },
    Pdf { path: String },
}

impl Default for DocSource {
    fn default() -> Self {
        DocSource::Pdf {
            path: String::new(),
        }
    }
}

/// Everything extracted from one paper.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AssetCatalog {
    pub schema: u32,
    pub source: DocSource,
    pub meta: Meta,
    pub sections: Vec<Section>,
    pub assets: Vec<Asset>,
    pub references: Vec<Reference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<PageInfo>,
}

impl AssetCatalog {
    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }
    pub fn asset_mut(&mut self, id: &str) -> Option<&mut Asset> {
        self.assets.iter_mut().find(|a| a.id == id)
    }
    pub fn require(&self, id: &str) -> Result<&Asset> {
        self.asset(id).ok_or_else(|| Error::NoAsset(id.into()))
    }
    pub fn of_kind(&self, kind: AssetKind) -> impl Iterator<Item = &Asset> {
        self.assets.iter().filter(move |a| a.kind == kind)
    }
    /// Count of numbered assets of one kind (each equation number counts once).
    pub fn count_numbered(&self, kind: AssetKind) -> usize {
        self.of_kind(kind).filter(|a| a.number.is_some()).count()
    }
    /// Look up by kind and printed number; roman (`IV`) and arabic (`4`) match.
    pub fn by_number(&self, kind: AssetKind, number: &str) -> Option<&Asset> {
        let want = normalize_number(number);
        self.of_kind(kind)
            .find(|a| a.number.as_deref().map(normalize_number).as_deref() == Some(want.as_str()))
    }
    /// An id not used yet: `base`, then `base-2`, `base-3` …
    pub fn free_id(&self, base: &str) -> String {
        if self.asset(base).is_none() {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|c| self.asset(c).is_none())
            .unwrap()
    }
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(s: &str) -> Result<AssetCatalog> {
        Ok(serde_json::from_str(s)?)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }
    pub fn load(path: &Path) -> Result<AssetCatalog> {
        AssetCatalog::from_json(&std::fs::read_to_string(path)?)
    }

    /// Copy page positions (page, bbox, caption bbox, crop) from a PDF-mode
    /// catalog of the same paper into this (LaTeX-mode) catalog, matching
    /// figures, tables and equations by kind and printed number. Returns the
    /// ids that got a position.
    pub fn attach_pdf(&mut self, pdf: &AssetCatalog) -> Vec<String> {
        let mut done = vec![];
        for a in self.assets.iter_mut() {
            let Some(n) = a.number.clone() else { continue };
            if !matches!(
                a.kind,
                AssetKind::Figure | AssetKind::Table | AssetKind::Equation
            ) {
                continue;
            }
            if let Some(p) = pdf.by_number(a.kind, &n)
                && p.bbox.is_some()
            {
                a.page = p.page;
                a.bbox = p.bbox;
                a.caption_bbox = p.caption_bbox;
                a.crop = p.crop.clone();
                a.source_ref.page = p.page;
                done.push(a.id.clone());
            }
        }
        if self.pages.is_empty() {
            self.pages = pdf.pages.clone();
        }
        if self.meta.page_count.is_none() {
            self.meta.page_count = pdf.meta.page_count;
        }
        done
    }
}

/// Upper-case roman numerals → arabic; everything else trimmed and lower-cased.
pub fn normalize_number(s: &str) -> String {
    let t = s.trim().trim_end_matches('.');
    if let Some(n) = roman_to_int(t) {
        return n.to_string();
    }
    t.to_lowercase()
}

pub fn roman_to_int(s: &str) -> Option<u32> {
    if s.is_empty() || !s.chars().all(|c| "IVXLC".contains(c)) {
        return None;
    }
    let v = |c| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        _ => 0,
    };
    let cs: Vec<u32> = s.chars().map(v).collect();
    let mut total = 0i64;
    for i in 0..cs.len() {
        let c = cs[i] as i64;
        if i + 1 < cs.len() && (cs[i + 1] as i64) > c {
            total -= c;
        } else {
            total += c;
        }
    }
    (total > 0).then_some(total as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roman_numbers_match_arabic() {
        assert_eq!(roman_to_int("IV"), Some(4));
        assert_eq!(roman_to_int("VIII"), Some(8));
        assert_eq!(roman_to_int("XII"), Some(12));
        assert_eq!(roman_to_int("4"), None);
        assert_eq!(normalize_number("III"), "3");
        assert_eq!(normalize_number("3a"), "3a");
    }

    #[test]
    fn bbox_sub_and_fraction() {
        let b = BBox::new(100.0, 200.0, 300.0, 400.0);
        assert_eq!(
            b.sub(&BBox::new(10.0, 10.0, 50.0, 60.0)),
            BBox::new(110.0, 210.0, 150.0, 260.0)
        );
        assert_eq!(
            b.sub_fraction(0.5, 0.0, 1.0, 0.5),
            BBox::new(200.0, 200.0, 300.0, 300.0)
        );
        // clipped to the parent
        assert_eq!(
            b.sub(&BBox::new(150.0, 0.0, 400.0, 50.0)),
            BBox::new(250.0, 200.0, 300.0, 250.0)
        );
    }

    #[test]
    fn catalog_roundtrip_and_attach() {
        let mut tex = AssetCatalog {
            schema: SCHEMA,
            ..Default::default()
        };
        let mut t = Asset::new("tab1", AssetKind::Table);
        t.number = Some("1".into());
        tex.assets.push(t);
        let mut pdf = AssetCatalog::default();
        let mut p = Asset::new("tab1", AssetKind::Table);
        p.number = Some("I".into());
        p.page = Some(3);
        p.bbox = Some(BBox::new(1.0, 2.0, 3.0, 4.0));
        pdf.assets.push(p);
        assert_eq!(tex.attach_pdf(&pdf), vec!["tab1".to_string()]);
        assert_eq!(tex.assets[0].page, Some(3));
        let back = AssetCatalog::from_json(&tex.to_json().unwrap()).unwrap();
        assert_eq!(back, tex);
        assert_eq!(tex.free_id("tab1"), "tab1-2");
    }
}
