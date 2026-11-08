//! Style system on top of a style pack (`ym_ooxml::style::StylePack`): text tags,
//! emphasis tags, palette (named colours, semantic tints, sequences, alpha
//! levels), components and ghost rules — plus the rules the editor enforces:
//!
//! * text runs use defined tags only (a new font/size needs a new tag first);
//! * colours come from the palette.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;
pub use ym_ooxml::style::{ArrowStyle, Emphasis, GhostRules, StylePack, TagStyle};

use crate::model::*;

/// Reserved emphasis applied by the progressive-reveal generator.
pub const GHOST_EM: &str = "ghost";

#[derive(Debug, Clone, Default, Serialize)]
pub struct StyleSystem {
    pub pack: StylePack,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuleError {
    #[error("未定義的文字標籤「{0}」：請先建立標籤")]
    UnknownTag(String),
    #[error("未定義的強調標籤「{0}」")]
    UnknownEmphasis(String),
    #[error("顏色 {0} 不在色票裡")]
    OffPalette(String),
    #[error("字級 {size} 超出標籤「{tag}」的範圍")]
    SizeOutOfRange { tag: String, size: f64 },
}

fn norm(h: &str) -> String {
    h.trim_start_matches('#').to_uppercase()
}

impl StyleSystem {
    pub fn new(pack: StylePack) -> Self {
        StyleSystem { pack }
    }

    pub fn tag(&self, name: &str) -> Option<&TagStyle> {
        self.pack.tags.get(name)
    }

    /// Every colour the palette allows (hex, upper case).
    pub fn palette_hex(&self) -> BTreeSet<String> {
        let p = &self.pack.palette;
        let mut s: BTreeSet<String> = p.colors.values().map(|h| norm(h)).collect();
        s.extend(p.tints.values().map(|t| norm(&t.hex)));
        for seq in p.sequences.values() {
            s.extend(seq.iter().map(|h| norm(h)));
        }
        s.extend(self.pack.emphasis.values().filter_map(|e| e.color.as_deref()).map(norm));
        s.extend(self.pack.tags.values().filter_map(|t| t.color.as_deref()).map(norm));
        s.extend(self.pack.arrows.values().map(|a| norm(&a.color)));
        if !self.pack.ghost.text_color.is_empty() {
            s.insert(norm(&self.pack.ghost.text_color));
        }
        s
    }

    /// Resolve a palette name (`red`, `supervised`, `numbered-circle[2]`) or hex to hex.
    pub fn color(&self, name: &str) -> Option<String> {
        let p = &self.pack.palette;
        if let Some(h) = p.colors.get(name) {
            return Some(norm(h));
        }
        if let Some(t) = p.tints.get(name) {
            return Some(norm(&t.hex));
        }
        if let Some((seq, idx)) = name.strip_suffix(']').and_then(|n| n.split_once('[')) {
            let i: usize = idx.parse().ok()?;
            let s = p.sequences.get(seq)?;
            return s.get(i % s.len().max(1)).map(|h| norm(h));
        }
        let h = norm(name);
        (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
    }

    /// Look of a run with this tag and emphasis.
    pub fn look(&self, tag: &str, em: Option<&str>) -> Option<Look> {
        let t = self.tag(tag)?;
        let mut l = Look {
            font: t.font.clone(),
            ea_font: t.ea_font.clone().unwrap_or_else(|| t.font.clone()),
            size: t.size,
            bold: t.bold,
            color: norm(t.color.as_deref().unwrap_or("000000")),
            ..Look::default()
        };
        match em {
            Some(GHOST_EM) => {
                l.color = norm(&self.pack.ghost.text_color);
            }
            Some(e) => {
                if let Some(e) = self.pack.emphasis.get(e) {
                    if let Some(c) = &e.color {
                        l.color = norm(c);
                    }
                    l.bold |= e.bold;
                }
            }
            None => {}
        }
        Some(l)
    }

    /// Find the tag (+ emphasis) that reproduces an imported look exactly.
    /// `role` hints the context: `title`, `ctrTitle`, `subTitle`, `table`, `math`, ``.
    pub fn match_look(&self, look: &Look, role: &str) -> Option<(String, Option<String>)> {
        let prefer: &[&str] = match role {
            "title" => &["title"],
            "ctrTitle" => &["cover-title"],
            "subTitle" => &["subtitle", "venue"],
            "table" => &["table"],
            "math" => &["equation"],
            _ => &[],
        };
        let order = [
            "body",
            "body-dense",
            "caption",
            "label",
            "takeaway",
            "reference",
            "table",
            "title",
            "cover-title",
            "subtitle",
            "venue",
            "equation",
        ];
        let mut names: Vec<&str> = prefer.to_vec();
        names.extend(order.iter().filter(|n| !prefer.contains(n)));
        let rest: Vec<&str> = self
            .pack
            .tags
            .keys()
            .map(String::as_str)
            .filter(|n| !names.contains(n))
            .collect();
        names.extend(rest);
        let fonts_match = |t: &TagStyle| {
            let cands = [
                Some(t.font.as_str()),
                t.ea_font.as_deref(),
                t.fallback.as_deref(),
            ];
            let ok = |f: &str| f.is_empty() || cands.contains(&Some(f));
            ok(&look.font) || (!look.ea_font.is_empty() && cands.contains(&Some(look.ea_font.as_str())))
        };
        let size_ok = |t: &TagStyle| {
            (look.size - t.size).abs() < 0.05
                || t
                    .size_range
                    .map(|[a, b]| look.size >= a - 0.05 && look.size <= b + 0.05)
                    .unwrap_or(false)
        };
        let col = norm(&look.color);
        for n in names {
            let Some(t) = self.tag(n) else { continue };
            if !size_ok(t) || !fonts_match(t) || look.italic {
                continue;
            }
            let tcol = norm(t.color.as_deref().unwrap_or("000000"));
            if col == tcol && look.bold == t.bold {
                return Some((n.to_string(), None));
            }
            for (en, e) in &self.pack.emphasis {
                let ecol = e.color.as_deref().map(norm).unwrap_or_else(|| tcol.clone());
                if ecol == col && (t.bold || e.bold) == look.bold {
                    return Some((n.to_string(), Some(en.clone())));
                }
            }
            if col == norm(&self.pack.ghost.text_color) && look.bold == t.bold {
                return Some((n.to_string(), Some(GHOST_EM.to_string())));
            }
        }
        None
    }

    /// Check a run against the rules (tag/emphasis exist, size within the tag's range).
    pub fn check_run(&self, r: &Run) -> Result<(), RuleError> {
        if r.kind == "math" || r.kind == "br" {
            return Ok(());
        }
        if let Some(t) = &r.tag {
            let ts = self.tag(t).ok_or_else(|| RuleError::UnknownTag(t.clone()))?;
            let (lo, hi) = ts
                .size_range
                .map(|[a, b]| (a, b))
                .unwrap_or((ts.size, ts.size));
            if r.look.size < lo - 0.05 || r.look.size > hi + 0.05 {
                return Err(RuleError::SizeOutOfRange {
                    tag: t.clone(),
                    size: r.look.size,
                });
            }
        }
        if let Some(e) = &r.em
            && e != GHOST_EM
            && !self.pack.emphasis.contains_key(e)
        {
            return Err(RuleError::UnknownEmphasis(e.clone()));
        }
        Ok(())
    }

    /// Validate a change `old → new` (JSON of an object, text body, run …):
    /// every run in `new` must pass [`check_run`] unless an identical run already
    /// existed in `old` (imported text may stay untagged until edited), and every
    /// colour in `new` must be in the palette or already present in `old`.
    pub fn check_change(&self, old: Option<&Value>, new: &Value) -> Result<(), RuleError> {
        let mut old_runs = Vec::new();
        let mut old_cols = BTreeSet::new();
        if let Some(o) = old {
            collect(o, &mut old_runs, &mut old_cols, false);
        }
        let mut new_runs = Vec::new();
        let mut new_cols = BTreeSet::new();
        // objects that still carry their source XML are imported content: their
        // own formatting stays legal until edited (edits are checked via `old`)
        collect(new, &mut new_runs, &mut new_cols, old.is_none());
        for r in new_runs {
            let Ok(run) = serde_json::from_value::<Run>(r.clone()) else {
                continue;
            };
            if self.check_run(&run).is_err() || run.tag.is_none() {
                // allowed only if the same formatting existed before
                // compare formatting without emphasis/colour: re-emphasising an
                // imported untagged run (e.g. ghosting it) keeps it legal
                let strip = |r: &Run| {
                    let mut r = r.clone();
                    r.em = None;
                    r.look.color.clear();
                    r.look.bold = false;
                    run_k(&r)
                };
                let same = old_runs.iter().any(|o| {
                    serde_json::from_value::<Run>(o.clone())
                        .map(|o| strip(&o) == strip(&run))
                        .unwrap_or(false)
                });
                if let Some(e) = &run.em
                    && e != GHOST_EM
                    && !self.pack.emphasis.contains_key(e)
                {
                    return Err(RuleError::UnknownEmphasis(e.clone()));
                }
                if !same {
                    self.check_run(&run)?;
                    if run.tag.is_none() && run.kind == "text" && !run.text.is_empty() {
                        return Err(RuleError::UnknownTag("（未指定）".into()));
                    }
                }
            }
        }
        let pal = self.palette_hex();
        for c in new_cols {
            if !pal.contains(&c) && !old_cols.contains(&c) {
                return Err(RuleError::OffPalette(c));
            }
        }
        Ok(())
    }
}

/// Collect runs (objects with `look`) and colours (`color` strings outside
/// runs' looks, since run colours follow tags) from a JSON value.
fn collect(v: &Value, runs: &mut Vec<Value>, cols: &mut BTreeSet<String>, skip_src: bool) {
    match v {
        Value::Object(m) => {
            if skip_src && m.contains_key("frame") && m.get("src").is_some_and(|s| !s.is_null()) {
                return;
            }
            if m.contains_key("look") && m.contains_key("kind") {
                runs.push(v.clone());
                return;
            }
            for (k, x) in m {
                if (k == "color" || k == "fill") && x.is_string() {
                    let h = norm(x.as_str().unwrap_or(""));
                    if h.len() == 6 {
                        cols.insert(h);
                    }
                } else if k != "src" {
                    collect(x, runs, cols, skip_src);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect(x, runs, cols, skip_src)),
        _ => {}
    }
}
