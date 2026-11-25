//! Icons for figures: the two open sets in `commons/icons` (Tabler Icons, Health Icons), built
//! into the program, so nothing is fetched and an installed app has them. This crate is the
//! only place that knows where the sets are and how they are written (`commons/icons/README.md`).
//!
//! An icon's id is `<set>:<name>`: `tabler:server`, `healthicons:doctor-outline`.
//!
//! Two looks. **Line**: Tabler's plain names (`server`) and Health Icons' `-outline` names.
//! **Filled**: Tabler's `-filled` names and Health Icons' plain names. Health Icons'
//! `-negative` and `-24px` spellings are other drawings of the same thing: they can be asked
//! for by id but a search never offers them.
//!
//! An icon becomes shapes a slide can hold ([`Icon`]): each shape is one path in a 0..1 box
//! written with `M`, `L`, `C`, `Z` only (what `lore_canvas::Geom::path` takes), either drawn as
//! a line of a given width with round ends or filled. A filled shape's holes are wound against
//! its outline, so a renderer that fills by the non-zero rule (PowerPoint's shapes have no
//! even-odd) shows them as holes.

use lore_canvas::model::{Frame, Geom, Kind, Object, Paint, SpObj, Stroke};
use serde::Serialize;

mod convert;
mod path;
mod sets;

/// One set on offer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetInfo {
    /// `tabler` | `healthicons`.
    pub id: &'static str,
    pub name: &'static str,
    /// The licence in a few words (`MIT`, `CC0 1.0`).
    pub license: &'static str,
    /// Icons a search can offer (both looks).
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Look {
    Line,
    Filled,
}

/// One find of a [`search`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    /// `<set>:<name>`.
    pub id: String,
    pub set: &'static str,
    /// The name without its look's ending (`doctor`, `server`): what a person reads.
    pub title: String,
    pub look: Look,
}

/// One path of an icon.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Shape {
    /// `M` / `L` / `C` / `Z` in a 0..1 box.
    pub path: String,
    /// Drawn as a line this wide (a fraction of the box's side, round ends and corners), not
    /// filled; `None` = filled, no line.
    pub stroke: Option<f64>,
}

/// An icon as shapes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Icon {
    pub id: String,
    pub shapes: Vec<Shape>,
}

/// The sets, in the order a search lists their finds.
pub fn sets() -> Vec<SetInfo> {
    sets::all()
        .iter()
        .map(|s| SetInfo { id: s.id, name: s.name, license: s.license, count: s.entries.len() })
        .collect()
}

/// Does `id` name an icon of one of the sets?
pub fn exists(id: &str) -> bool {
    sets::find(id).is_some()
}

/// The icons whose name has every word of `query` as a whole word of the name (names are
/// split at `-`; `lan` finds `lan` and never `plane`), an English plural matching its singular.
/// The best first: a name that is exactly the query, then the fewest other words, then by set
/// order and name. `look` = only that look. At most `limit`. An empty query finds nothing.
pub fn search(query: &str, look: Option<Look>, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    if words.is_empty() || limit == 0 {
        return Vec::new();
    }
    let qn: Vec<String> = words.iter().map(|w| sets::norm(w)).collect();
    let joined = words.join("-");
    let off = words.iter().any(|w| w == "off");
    let mut found: Vec<((bool, usize, usize), &'static str, &sets::Entry)> = Vec::new();
    for (si, set) in sets::all().iter().enumerate() {
        for e in &set.entries {
            if look.is_some_and(|l| l != e.look) || (e.has_off && !off) {
                continue;
            }
            if !qn.iter().all(|w| e.tokens.contains(w)) {
                continue;
            }
            let exact = e.title == joined || e.name == joined;
            found.push(((!exact, e.tokens.len(), si), set.id, e));
        }
    }
    // entries are in name order inside a set and the sort is stable
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
        .into_iter()
        .take(limit)
        .map(|(_, set, e)| Hit { id: format!("{set}:{}", e.name), set, title: e.title.clone(), look: e.look })
        .collect()
}

/// The icon as a page shows it: one `<svg viewBox=…>` drawn in `currentColor`, the set's own
/// drawing untouched. `None` when there is no such icon.
pub fn svg(id: &str) -> Option<String> {
    let f = sets::find(id)?;
    Some(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\">{}</svg>", f.w, f.h, f.body))
}

/// The icon as shapes, or `None` when there is no such icon or it uses something that cannot
/// be turned into paths (a handful do).
pub fn icon(id: &str) -> Option<Icon> {
    let f = sets::find(id)?;
    let shapes = convert::shapes(f.body, f.w, f.h)?;
    Some(Icon { id: id.to_string(), shapes })
}

/// The icon's shapes as slide objects inside `frame` (kept square, centred), in `color`
/// (`RRGGBB`), ids from `first_id`, each named `name`: a filled shape has the colour as its
/// fill and no line, a line shape no fill and a line with `cap = "round"`.
pub fn objects(icon: &Icon, frame: Frame, color: &str, first_id: u32, name: &str) -> Vec<Object> {
    let side = frame.w.min(frame.h).max(0);
    let sq = Frame {
        x: frame.x + (frame.w - side) / 2,
        y: frame.y + (frame.h - side) / 2,
        w: side,
        h: side,
        rot: 0.0,
        flip_h: false,
        flip_v: false,
    };
    // EMU -> pt: 12700 per point
    let side_pt = side as f64 / 12700.0;
    let color = color.trim_start_matches('#').to_uppercase();
    icon.shapes
        .iter()
        .enumerate()
        .map(|(i, sh)| {
            let (fill, line) = match sh.stroke {
                Some(w) => {
                    let mut s = Stroke::new(&color, w * side_pt);
                    s.cap = "round".into();
                    (Paint::None, Some(s))
                }
                None => (Paint::Solid { color: color.clone(), alpha: 1.0 }, None),
            };
            Object {
                id: first_id + i as u32,
                name: name.to_string(),
                frame: sq.clone(),
                hidden: false,
                ph: None,
                slot: None,
                kind: Kind::Shape(SpObj {
                    geom: Geom { prst: "custom".into(), adj: Default::default(), path: Some(sh.path.clone()) },
                    fill,
                    line,
                    text: None,
                    equation: false,
                }),
                src: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(q: &str, look: Option<Look>) -> Vec<String> {
        search(q, look, 100000).into_iter().map(|h| h.id).collect()
    }

    #[test]
    fn a_word_matches_whole_tokens_only() {
        // `lan` is not a word of lane / language / plane
        let r = ids("lan", None);
        assert!(r.iter().all(|i| i.split([':', '-']).any(|t| t == "lan")), "{r:?}");
        assert!(!ids("server", None).contains(&"tabler:serverless".to_string()));
        assert!(ids("serverless", None).contains(&"tabler:serverless".to_string()));
        assert!(!ids("lane", None).iter().any(|i| i.contains("plane")));
        let r = ids("arrow up", None);
        assert!(r.contains(&"tabler:arrow-up".to_string()));
        assert!(r.iter().all(|i| i.contains("arrow") && i.contains("up")));
    }

    #[test]
    fn the_exact_name_comes_first() {
        let r = search("server", None, 10);
        assert_eq!(r[0].id, "tabler:server");
        assert_eq!(r[0].title, "server");
        assert_eq!(r[0].look, Look::Line);
        let r = search("doctor", None, 10);
        assert_eq!(r[0].id, "healthicons:doctor");
        assert_eq!(search("server", None, 3).len(), 3);
        assert!(search("server", None, 0).is_empty());
    }

    #[test]
    fn a_look_filters() {
        let line = search("doctor", Some(Look::Line), 100);
        assert!(!line.is_empty() && line.iter().all(|h| h.look == Look::Line));
        assert!(line.iter().any(|h| h.id == "healthicons:doctor-outline" && h.title == "doctor"));
        let filled = search("accessible", Some(Look::Filled), 100);
        assert!(filled.iter().any(|h| h.id == "tabler:accessible-filled" && h.title == "accessible"));
        assert!(filled.iter().all(|h| h.look == Look::Filled));
    }

    #[test]
    fn negative_and_24px_are_never_offered() {
        let r = ids("crutches", None);
        assert!(r.contains(&"healthicons:crutches".to_string()));
        assert!(r.iter().all(|i| !i.ends_with("-negative") && !i.contains("24px")), "{r:?}");
        assert!(ids("negative", None).iter().all(|i| !i.ends_with("-negative")));
        assert!(exists("healthicons:crutches-negative"));
        assert!(svg("healthicons:crutches-negative").is_some());
        // tabler -off names only when asked for
        assert!(!ids("wifi", None).iter().any(|i| i.ends_with("-off")));
        assert!(ids("wifi off", None).iter().any(|i| i.ends_with("-off")));
    }

    #[test]
    fn a_plural_finds_its_singular() {
        let a = ids("servers", None);
        assert!(a.contains(&"tabler:server".to_string()));
        assert_eq!(a, ids("server", None));
    }

    #[test]
    fn an_empty_query_finds_nothing() {
        assert!(search("", None, 10).is_empty());
        assert!(search("  - ", None, 10).is_empty());
    }

    fn frame(w: i64, h: i64) -> Frame {
        Frame { x: 1000, y: 2000, w, h, ..Default::default() }
    }

    #[test]
    fn a_line_icon_is_unfilled_shapes_with_round_ends() {
        let ic = icon("tabler:server").unwrap();
        let o = objects(&ic, frame(24 * 12700, 24 * 12700), "1a2b3c", 5, "icon");
        assert_eq!(o.len(), ic.shapes.len());
        let Kind::Shape(sp) = &o[0].kind else { panic!() };
        assert_eq!(sp.fill, Paint::None);
        let l = sp.line.as_ref().unwrap();
        assert_eq!(l.cap, "round");
        assert_eq!(l.color, "1A2B3C");
        // stroke 2/24 of a 24 pt box
        assert!((l.width_pt - 2.0).abs() < 1e-9, "{}", l.width_pt);
        assert_eq!(sp.geom.prst, "custom");
        assert!(sp.geom.path.as_deref().unwrap().starts_with("M "));
        assert!(sp.text.is_none() && !sp.equation);
    }

    #[test]
    fn a_filled_icon_is_filled_shapes_without_a_line() {
        let ic = icon("healthicons:doctor").unwrap();
        let o = objects(&ic, frame(100, 100), "FF0000", 1, "x");
        assert!(!o.is_empty());
        for ob in &o {
            let Kind::Shape(sp) = &ob.kind else { panic!() };
            assert_eq!(sp.fill, Paint::Solid { color: "FF0000".into(), alpha: 1.0 });
            assert!(sp.line.is_none());
        }
    }

    #[test]
    fn the_icon_is_square_and_centred_in_a_wide_frame() {
        let ic = icon("tabler:server").unwrap();
        let o = objects(&ic, frame(1000, 400), "000000", 1, "x");
        let f = &o[0].frame;
        assert_eq!((f.w, f.h), (400, 400));
        assert_eq!((f.x, f.y), (1000 + 300, 2000));
        assert_eq!((f.rot, f.flip_h, f.flip_v), (0.0, false, false));
        let o = objects(&ic, frame(400, 1000), "000000", 1, "x");
        assert_eq!((o[0].frame.x, o[0].frame.y, o[0].frame.w), (1000, 2000 + 300, 400));
    }

    #[test]
    fn ids_count_up_from_the_first() {
        let ic = icon("healthicons:doctor-outline").unwrap();
        let mut ic2 = ic.clone();
        ic2.shapes.extend(ic.shapes.clone());
        let o = objects(&ic2, frame(100, 100), "000000", 40, "pic");
        assert_eq!(o.iter().map(|x| x.id).collect::<Vec<_>>(), (40..40 + o.len() as u32).collect::<Vec<_>>());
        assert!(o.len() >= 2 && o.iter().all(|x| x.name == "pic"));
    }
}
