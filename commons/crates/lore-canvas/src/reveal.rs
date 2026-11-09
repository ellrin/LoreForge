//! Progressive-reveal generator ("duplicate the slide and ghost what is not
//! current", design §7.3): one slide per step; objects of the current step stay
//! as they are, objects of other steps are ghosted by the style pack's rules
//! (pictures alpha, text grey, lines/arrows alpha, fills alpha) — or, in agenda
//! mode, one paragraph of a list is emphasised per slide.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::model::*;
use crate::style::{GHOST_EM, GhostRules};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RevealSpec {
    /// One slide per step; each step lists the object ids that are "current".
    Ghost {
        steps: Vec<Vec<u32>>,
        /// What happens to objects of *later* steps: `ghost` (default) or `hide`.
        #[serde(default = "ghost")]
        future: String,
    },
    /// One slide per paragraph of a text object: that paragraph gets `em`
    /// (default `strong` = bold), the others stay plain.
    Agenda {
        obj: u32,
        #[serde(default)]
        paras: Option<Vec<usize>>,
        #[serde(default = "strong")]
        em: String,
    },
}
fn ghost() -> String {
    "ghost".into()
}
fn strong() -> String {
    "strong".into()
}

/// Ghost one object (recursively for groups).
pub fn ghost_object(o: &mut Object, r: &GhostRules) {
    let fill_alpha = if r.fill_alpha > 0.0 { r.fill_alpha } else { r.line_alpha };
    match &mut o.kind {
        Kind::Picture(p) => {
            p.alpha = p.alpha.min(r.picture_alpha);
            if let Some(l) = &mut p.line {
                l.alpha = l.alpha.min(r.line_alpha);
            }
        }
        Kind::Connector(c) => {
            if let Some(l) = &mut c.line {
                l.alpha = l.alpha.min(r.line_alpha);
            }
            if let Some(t) = &mut c.text {
                ghost_text(t);
            }
        }
        Kind::Text(s) | Kind::Shape(s) => {
            match &mut s.fill {
                Paint::Solid { alpha, .. } => *alpha = alpha.min(fill_alpha),
                Paint::Gradient { stops, .. } => {
                    for st in stops {
                        st.alpha = st.alpha.min(fill_alpha);
                    }
                }
                _ => {}
            }
            if let Some(l) = &mut s.line {
                l.alpha = l.alpha.min(r.line_alpha);
            }
            if let Some(t) = &mut s.text {
                ghost_text(t);
            }
        }
        Kind::Table(t) => {
            for row in &mut t.rows {
                for c in &mut row.cells {
                    ghost_text(&mut c.text);
                }
            }
        }
        Kind::Group(g) => {
            for c in &mut g.children {
                ghost_object(c, r);
            }
        }
        Kind::Locked(_) => {}
    }
}

fn ghost_text(t: &mut TextBody) {
    for p in &mut t.paras {
        for r in &mut p.runs {
            if r.kind == "text" || r.kind == "field" || r.kind == "math" {
                r.em = Some(GHOST_EM.into());
            }
        }
    }
}

/// Produce the step slides for `slide`. The first output slide keeps the
/// source's uid and `src` (it replaces the original); the others get uids from
/// `new_uid` and keep `src` so their unedited objects still export verbatim.
/// `ghost_color` is the ghost text colour (applied to run looks).
pub fn reveal(
    slide: &Slide,
    spec: &RevealSpec,
    rules: &GhostRules,
    ghost_look: &dyn Fn(&Look) -> Look,
    new_uid: &mut dyn FnMut() -> String,
) -> Vec<Slide> {
    let mut out = Vec::new();
    match spec {
        RevealSpec::Ghost { steps, future } => {
            let all: BTreeSet<u32> = steps.iter().flatten().copied().collect();
            for (i, cur) in steps.iter().enumerate() {
                let cur: BTreeSet<u32> = cur.iter().copied().collect();
                let later: BTreeSet<u32> = steps[i + 1..]
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|id| !cur.contains(id) && !steps[..i].iter().flatten().any(|x| x == id))
                    .collect();
                let mut s = slide.clone();
                if i > 0 {
                    s.uid = new_uid();
                }
                s.anims.clear();
                s.walk_mut(&mut |o| {
                    if all.contains(&o.id) && !cur.contains(&o.id) {
                        if future == "hide" && later.contains(&o.id) {
                            o.hidden = true;
                        } else {
                            ghost_object(o, rules);
                            relook(o, ghost_look);
                        }
                    }
                });
                out.push(s);
            }
        }
        RevealSpec::Agenda { obj, paras, em } => {
            let list: Vec<usize> = paras.clone().unwrap_or_else(|| {
                slide
                    .find(*obj)
                    .and_then(|o| o.kind.sp())
                    .and_then(|s| s.text.as_ref())
                    .map(agenda_items)
                    .unwrap_or_default()
            });
            for (i, pi) in list.iter().enumerate() {
                let mut s = slide.clone();
                if i > 0 {
                    s.uid = new_uid();
                }
                if let Some(o) = s.find_mut(*obj)
                    && let Some(t) = o.kind.sp_mut().and_then(|s| s.text.as_mut())
                    && let Some(p) = t.paras.get_mut(*pi)
                {
                    for r in &mut p.runs {
                        if r.kind == "text" {
                            r.em = Some(em.clone());
                            r.look.bold = true;
                        }
                    }
                }
                out.push(s);
            }
        }
    }
    out
}

/// Paragraphs that make one agenda slide each: top-level (level 0) items with
/// visible content; empty spacer lines and sub-items are skipped.
pub fn agenda_items(t: &TextBody) -> Vec<usize> {
    t.paras
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.level == 0
                && p.runs.iter().any(|r| r.kind == "math" || r.kind == "raw" || !r.text.trim().is_empty())
        })
        .map(|(i, _)| i)
        .collect()
}

fn relook(o: &mut Object, f: &dyn Fn(&Look) -> Look) {
    let fix = |t: &mut TextBody| {
        for p in &mut t.paras {
            for r in &mut p.runs {
                if r.em.as_deref() == Some(GHOST_EM) {
                    r.look = f(&r.look);
                }
            }
        }
    };
    match &mut o.kind {
        Kind::Text(s) | Kind::Shape(s) => {
            if let Some(t) = &mut s.text {
                fix(t)
            }
        }
        Kind::Connector(c) => {
            if let Some(t) = &mut c.text {
                fix(t)
            }
        }
        Kind::Table(t) => {
            for row in &mut t.rows {
                for c in &mut row.cells {
                    fix(&mut c.text);
                }
            }
        }
        Kind::Group(g) => {
            for c in &mut g.children {
                relook(c, f);
            }
        }
        _ => {}
    }
}
