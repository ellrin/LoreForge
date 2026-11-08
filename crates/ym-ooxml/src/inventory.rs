//! Per-slide object inventory: what was parsed into typed objects, what is
//! locked (kept raw, not editable), and which raw-kept features appear inside
//! otherwise editable objects.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::deck::Deck;
use crate::model::{FrameContent, Shape, SlideDoc};
use crate::xml::Element;

#[derive(Debug, Clone, Default, Serialize)]
pub struct SlideInventory {
    pub index: usize,
    pub part: String,
    pub hidden: bool,
    pub layout: String,
    /// Object counts by kind (sp, textbox, pic, cxn, grp, table, math, alt, frame, locked).
    pub counts: BTreeMap<String, usize>,
    pub objects: usize,
    /// Locked objects (kind strings, e.g. `graphicFrame:chart`).
    pub locked: Vec<String>,
    /// Raw-kept features inside editable objects (custGeom, gradFill, a14:backgroundRemoval…).
    pub raw_features: BTreeMap<String, usize>,
    pub has_notes: bool,
    /// Slide has `p:timing` (kept raw unless it is one of the 5 supported entrance effects).
    pub has_timing: bool,
    pub has_transition: bool,
}

impl SlideInventory {
    pub fn fully_editable(&self) -> bool {
        self.locked.is_empty()
    }
}

/// Pieces kept as raw XML inside otherwise editable objects (preserved on save,
/// edited as a whole or not at all).
const FEATURES: &[&str] = &[
    "a:custGeom",
    "a:gradFill",
    "a:pattFill",
    "a:blipFill",
    "a:effectLst",
    "a:scene3d",
    "a:sp3d",
    "a14:backgroundRemoval",
    "a14:imgProps",
    "a14:hiddenFill",
    "a14:m",
    "a:hlinkClick",
    "a:prstTxWarp",
];

fn count_features(el: &Element, out: &mut BTreeMap<String, usize>) {
    if FEATURES.contains(&el.name.as_str()) {
        // empty effect lists are just "no effect"
        if !(el.name == "a:effectLst" && el.children.is_empty()) {
            *out.entry(el.name.clone()).or_default() += 1;
        }
    }
    for c in el.elements() {
        count_features(c, out);
    }
}

pub fn inventory_doc(
    doc: &SlideDoc,
) -> (
    BTreeMap<String, usize>,
    Vec<String>,
    BTreeMap<String, usize>,
) {
    let mut counts = BTreeMap::new();
    let mut locked = Vec::new();
    let mut feats = BTreeMap::new();
    doc.tree.walk(&mut |s, _| {
        *counts.entry(s.kind_name().to_string()).or_default() += 1;
        match s {
            Shape::Locked(l) => locked.push(l.kind.clone()),
            Shape::Frame(f) => {
                if let FrameContent::Other(_) = f.content {
                    let uri = f.uri();
                    let tail = uri.rsplit('/').next().unwrap_or("unknown").to_string();
                    locked.push(format!("graphicFrame:{tail}"));
                }
            }
            _ => {}
        }
        if !matches!(s, Shape::Group(_) | Shape::Alt(_)) {
            count_features(&s.to_element(), &mut feats);
        }
    });
    if let Some(bg) = &doc.background {
        count_features(bg, &mut feats);
    }
    (counts, locked, feats)
}

pub fn inventory(deck: &Deck) -> Vec<SlideInventory> {
    deck.slides
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (counts, locked, raw_features) = inventory_doc(&s.doc);
            let objects = counts
                .iter()
                .filter(|(k, _)| k.as_str() != "alt")
                .map(|(_, v)| v)
                .sum();
            SlideInventory {
                index: i + 1,
                part: s.part.clone(),
                hidden: s.doc.hidden(),
                layout: deck
                    .slide_layout(i)
                    .map(|l| l.name().to_string())
                    .unwrap_or_default(),
                counts,
                objects,
                locked,
                raw_features,
                has_notes: s.notes.is_some(),
                has_timing: s.doc.timing.is_some(),
                has_transition: s.doc.transition.is_some(),
            }
        })
        .collect()
}
