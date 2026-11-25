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

use lore_canvas::model::{Frame, Object};
use serde::Serialize;

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
    Vec::new()
}

/// Does `id` name an icon of one of the sets?
pub fn exists(id: &str) -> bool {
    let _ = id;
    false
}

/// The icons whose name has every word of `query` as a whole word of the name (names are
/// split at `-`; `lan` finds `lan` and never `plane`), an English plural matching its singular.
/// The best first: a name that is exactly the query, then the fewest other words, then by set
/// order and name. `look` = only that look. At most `limit`. An empty query finds nothing.
pub fn search(query: &str, look: Option<Look>, limit: usize) -> Vec<Hit> {
    let _ = (query, look, limit);
    Vec::new()
}

/// The icon as a page shows it: one `<svg viewBox=…>` drawn in `currentColor`, the set's own
/// drawing untouched. `None` when there is no such icon.
pub fn svg(id: &str) -> Option<String> {
    let _ = id;
    None
}

/// The icon as shapes, or `None` when there is no such icon or it uses something that cannot
/// be turned into paths (a handful do).
pub fn icon(id: &str) -> Option<Icon> {
    let _ = id;
    None
}

/// The icon's shapes as slide objects inside `frame` (kept square, centred), in `color`
/// (`RRGGBB`), ids from `first_id`, each named `name`: a filled shape has the colour as its
/// fill and no line, a line shape no fill and a line with `cap = "round"`.
pub fn objects(icon: &Icon, frame: Frame, color: &str, first_id: u32, name: &str) -> Vec<Object> {
    let _ = (icon, frame, color, first_id, name);
    Vec::new()
}
