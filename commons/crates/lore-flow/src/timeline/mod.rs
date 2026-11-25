//! Timelines (`docs/flow-figures.md` §7): a figure of points in time hung on an axis, not a
//! data flow. It rides inside a [`crate::figure::FlowSpec`] (`"timeline": {…}`), so everything
//! that carries a flow figure (an L-Slide group, L-Paper's `<stem>.flow.json`, L-Brain's made
//! figures) carries a timeline, and [`crate::figure::render`] draws it into the same
//! [`crate::Figure`] of `lore_canvas` objects.
//!
//! ```json
//! { "lore_flow": 1,
//!   "timeline": { "id": "history", "title": "Five years", "style": "ring-alternate",
//!     "points": [ { "time": "2021", "title": "Pilot", "body": "Two sites, one protocol." } ] } }
//! ```
//!
//! How it looks is a **style**: one TOML file per style ([`Style`]), the neutral ones in this
//! crate (`timelines/*.toml`), the author's own in `<private>/shared/flow/timelines/*.toml`
//! and an app's `flow/timelines/`. A new file is a new style with no change to the program. A
//! style that asks for something the program cannot draw yet is still loaded, and says so in
//! [`Style::cannot`]: it is listed, never silently drawn as something else.
//!
//! Three steps, three files:
//! - [`style`]: the file → [`Style`], with what cannot be drawn.
//! - [`place`]: spec + style + frame → [`Plan`] (every rectangle and line, in pt; no objects).
//! - [`draw`]: the plan → `lore_canvas` objects.
//!
//! Colours come from the style file only (a figure is a document: it never takes an app's
//! theme). The words `point` and `paper` in a colour field mean the point's own colour and the
//! page (`FFFFFF`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::build::Built;
use crate::layout::Rect;

pub mod draw;
#[cfg(feature = "llm")]
pub mod llm;
pub mod place;
pub mod style;

// ── what is in it ────────────────────────────────────────────────────────────

/// A timeline as it is stored in a flow spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TimelineSpec {
    /// The figure id (`flow:<id>/…` names its objects); empty = `timeline`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// A [`Style::id`]; empty or unknown = the first neutral style.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub style: String,
    pub points: Vec<Point>,
    /// The colours of this figure where it is used (a slide, a paper), `RRGGBB` in point
    /// order, over the style's and Lore's own. Empty = not changed. The style gives the
    /// structure; the colours are the author's to set in each use.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<String>,
    /// Keys a newer editor wrote: kept.
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// One point in time. Texts go through the flow tool's text path (`$…$` is math).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Point {
    /// What is written as the time: a year, a month, a date, "Phase 2".
    pub time: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// A few lines; `\n` breaks a line, long lines are wrapped by [`place`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// What the picture of this point should show (a word or a file); empty = none. Where the
    /// style has an icon place and there is no picture, an empty slot is drawn.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// `RRGGBB` over the style's colour for this point; empty = the style's.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// An ISO date (`2026-11-25`), for styles that space by date; unused by equal spacing.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
}

impl TimelineSpec {
    pub fn fig_id(&self) -> &str {
        if self.id.is_empty() { "timeline" } else { &self.id }
    }
}

// ── how it looks: the style file ─────────────────────────────────────────────

/// The values each field takes today. [`style::check`] lists anything else in [`Style::cannot`].
pub mod can {
    pub const AXIS: [&str; 4] = ["line", "band", "joins", "none"];
    pub const AXIS_AT: [&str; 3] = ["middle", "top", "bottom"];
    pub const MARKER: [&str; 5] = ["dot", "ring", "square", "circle", "block"];
    pub const INSIDE: [&str; 4] = ["none", "time", "icon", "number"];
    pub const TIME_AT: [&str; 5] = ["opposite", "with-text", "in-marker", "in-axis", "none"];
    pub const TEXT: [&str; 2] = ["plain", "card"];
    pub const SIDE: [&str; 3] = ["above", "below", "alternate"];
    pub const ALIGN: [&str; 2] = ["left", "center"];
    pub const RULE: [&str; 4] = ["none", "under", "over", "far"];
    pub const STEM: [&str; 3] = ["none", "line", "drop"];
    pub const STEM_END: [&str; 2] = ["none", "dot"];
    pub const ICON_AT: [&str; 4] = ["none", "in-marker", "opposite", "with-text"];
    pub const COLORS: [&str; 3] = ["per-point", "single", "cycle"];
    pub const SPACING: [&str; 1] = ["equal"];
}

/// One style file. Every field has a default, so a file states only what differs. Sizes are pt
/// on a 672 × 357 frame (a 16:9 slide's content box) and scale with the frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    /// The file's stem when the file gives none.
    pub id: String,
    /// What the author sees in the list (Traditional Chinese is fine).
    pub name: String,
    /// One line on when to use it.
    pub note: String,
    pub axis: Axis,
    pub marker: Marker,
    pub time: TimeLabel,
    pub text: TextBlock,
    pub card: Card,
    pub stem: Stem,
    pub icon: Icon,
    pub colors: Colors,
    pub title: Title,
    /// `equal` (wave 1). `date` = by [`Point::date`] (not drawn yet).
    pub spacing: String,
    /// Tables this program does not know (`[[track]]`, `[today]` …), kept and reported.
    #[serde(flatten)]
    pub other: BTreeMap<String, toml::Value>,
    /// Set by the loader: the file came from the author's folders.
    #[serde(skip)]
    pub private: bool,
    /// Set by [`style::check`]: what this style asks for that cannot be drawn yet, one short
    /// line each (`axis.kind = "path"`). Empty = it draws as written.
    #[serde(skip)]
    pub cannot: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Axis {
    /// `line` (thin) · `band` (thick, may hold the times) · `joins` (no axis: dashed joins
    /// between the markers) · `none`. Later: `chevrons`, `path`.
    pub kind: String,
    /// Line weight, or the band's height (pt).
    pub weight: f64,
    /// `RRGGBB`.
    pub color: String,
    /// Where in the height the axis runs: `middle` · `top` · `bottom`.
    pub at: String,
    /// The axis runs to the frame's left and right edges instead of stopping at the end points.
    pub bleed: bool,
    /// `solid` · `dash` · `sysDot` (the canvas's dash names).
    pub dash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Marker {
    /// `dot` · `ring` (a dot with rings round it) · `square` · `circle` (large, holds
    /// something) · `block` (a rounded block that holds the time).
    pub kind: String,
    /// Diameter or side (pt); a `block` is as wide as its text asks.
    pub size: f64,
    /// `point` · `paper` · `RRGGBB`.
    pub fill: String,
    /// Outline: `point` · `none` · `RRGGBB`.
    pub line: String,
    pub line_weight: f64,
    /// `ring` only: how many rings round the dot.
    pub rings: u32,
    /// What is written inside a `circle` or `block`: `none` · `time` · `icon` · `number`.
    pub inside: String,
    /// Corner radius of a `square` or `block` (pt).
    pub radius: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeLabel {
    /// `opposite` (across the axis from the text) · `with-text` (over the title, same side) ·
    /// `in-marker` · `in-axis` (inside a band) · `none`.
    pub at: String,
    pub size: f64,
    pub bold: bool,
    /// `point` · `RRGGBB`.
    pub color: String,
    /// `in-axis` only: the times sit in two rows, odd and even points apart.
    pub stagger: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextBlock {
    /// `plain` · `card` (a filled box with a header strip, see [`Card`]).
    pub kind: String,
    /// Where the title and body go: `above` · `below` · `alternate`.
    pub side: String,
    /// `alternate` only: the side of the first point (`above` · `below`).
    pub first: String,
    pub title_size: f64,
    pub body_size: f64,
    /// `point` · `RRGGBB`.
    pub title_color: String,
    pub body_color: String,
    /// `left` (from the stem) · `center` (on the point).
    pub align: String,
    /// A short rule in the point's colour: `none` · `under` · `over` the text · `far` (beyond
    /// the text on the side away from the axis: under it below the axis, over it above).
    pub rule: String,
    /// Widest text column (pt); 0 = what the spacing allows.
    pub width: f64,
    pub caps: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    /// `point` · `RRGGBB`.
    pub fill: String,
    /// Text colour on the card.
    pub text: String,
    /// The title sits in a strip on the card's top edge.
    pub header: bool,
    pub radius: f64,
    pub pad: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stem {
    /// From the marker to the text: `none` · `line` · `drop` (a pointer on the marker).
    pub kind: String,
    /// `none` · `dot` at the text end.
    pub end: String,
    /// `point` · `RRGGBB`.
    pub color: String,
    pub weight: f64,
    /// Shortest stem (pt).
    pub length: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Icon {
    /// `none` · `in-marker` · `opposite` (across the axis from the text) · `with-text`.
    pub at: String,
    pub size: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Colors {
    /// `per-point` (the palette in order, round again when it runs out) · `single` (the first)
    /// · `cycle` (the same as per-point; kept as its own word for files that say it).
    pub mode: String,
    /// `RRGGBB`. Empty (the default) = Lore's own figure colours: the flow tool's palette
    /// ([`lore_colors`]), or the author's `palette.json`. A style states a palette only when
    /// it must differ.
    pub palette: Vec<String>,
    /// Text that is not coloured.
    pub ink: String,
    pub muted: String,
    /// Markers, cards and blocks are filled with a left-to-right gradient of their colour.
    pub gradient: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Title {
    pub size: f64,
    pub color: String,
    /// `left` · `center`.
    pub align: String,
    pub caps: bool,
    /// A row of dots in the points' colours under the title.
    pub legend_dots: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            id: String::new(),
            name: String::new(),
            note: String::new(),
            axis: Axis::default(),
            marker: Marker::default(),
            time: TimeLabel::default(),
            text: TextBlock::default(),
            card: Card::default(),
            stem: Stem::default(),
            icon: Icon::default(),
            colors: Colors::default(),
            title: Title::default(),
            spacing: "equal".into(),
            other: BTreeMap::new(),
            private: false,
            cannot: vec![],
        }
    }
}
impl Default for Axis {
    fn default() -> Self {
        Axis { kind: "line".into(), weight: 1.0, color: "9AA0A6".into(), at: "middle".into(), bleed: false, dash: "solid".into() }
    }
}
impl Default for Marker {
    fn default() -> Self {
        Marker { kind: "dot".into(), size: 10.0, fill: "point".into(), line: "none".into(), line_weight: 1.0, rings: 1, inside: "none".into(), radius: 2.0 }
    }
}
impl Default for TimeLabel {
    fn default() -> Self {
        TimeLabel { at: "opposite".into(), size: 16.0, bold: true, color: "point".into(), stagger: false }
    }
}
impl Default for TextBlock {
    fn default() -> Self {
        TextBlock { kind: "plain".into(), side: "below".into(), first: "below".into(), title_size: 11.0, body_size: 9.0, title_color: "point".into(), body_color: "5F6368".into(), align: "center".into(), rule: "none".into(), width: 0.0, caps: false }
    }
}
impl Default for Card {
    fn default() -> Self {
        Card { fill: "point".into(), text: "FFFFFF".into(), header: true, radius: 4.0, pad: 6.0 }
    }
}
impl Default for Stem {
    fn default() -> Self {
        Stem { kind: "line".into(), end: "none".into(), color: "point".into(), weight: 0.75, length: 18.0 }
    }
}
impl Default for Icon {
    fn default() -> Self {
        Icon { at: "none".into(), size: 28.0 }
    }
}
impl Default for Colors {
    fn default() -> Self {
        Colors { mode: "per-point".into(), palette: vec![], ink: "202124".into(), muted: "5F6368".into(), gradient: false }
    }
}
impl Default for Title {
    fn default() -> Self {
        Title { size: 16.0, color: "202124".into(), align: "center".into(), caps: false, legend_dots: false }
    }
}

// ── where everything goes: the plan ──────────────────────────────────────────

/// The side of the axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Side {
    Above,
    Below,
}

/// A text with its box: `text` is already wrapped (`\n`), `size` is the type size it was
/// measured at (pt), `rect` holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Slot {
    pub rect: Rect,
    pub text: String,
    pub size: f64,
}

/// A straight line from `a` to `b` (pt).
pub type Line = [(f64, f64); 2];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AxisGeom {
    pub x0: f64,
    pub x1: f64,
    /// The axis's centre line.
    pub y: f64,
    /// Its height (a line's weight, a band's height); 0 = no axis is drawn.
    pub thickness: f64,
}

/// Everything of one point, in pt inside the frame. `None` = the style has no such part.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PointGeom {
    /// The point's place on the axis.
    pub x: f64,
    /// The side its title and body are on.
    pub side: Side,
    /// `RRGGBB`, resolved (the style's palette or the point's own).
    pub color: String,
    pub marker: Rect,
    pub time: Option<Slot>,
    pub title: Option<Slot>,
    pub body: Option<Slot>,
    /// `text.kind = card`: the whole card and, with `card.header`, its strip (the title's slot
    /// lies inside the strip, the body's inside the card).
    pub card: Option<Rect>,
    pub card_header: Option<Rect>,
    pub stem: Option<Line>,
    /// Where the picture goes (an empty slot when the point has none).
    pub icon: Option<Rect>,
    /// The short rule of `text.rule`.
    pub rule: Option<Line>,
}

/// The whole figure placed: what [`draw`] turns into objects.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    /// The box everything lies in (inside the frame given to [`place::plan`]).
    pub bounds: Rect,
    pub title: Option<Slot>,
    /// The centres of the title's legend dots, in point order (empty = none).
    pub legend: Vec<(f64, f64)>,
    pub axis: AxisGeom,
    pub points: Vec<PointGeom>,
    /// Sizes were multiplied by this to fit the frame (1 = as the style says).
    pub scale: f64,
}

/// Lore's figure colours as point colours: the strong colour of each entry of the flow tool's
/// palette ([`crate::figure::PALETTE`], never an app theme), without the grey and the white
/// (a point in black or grey reads as "no colour", not as one of the points).
pub fn lore_colors() -> Vec<String> {
    crate::figure::PALETTE.iter().filter(|(_, fill, _)| !matches!(*fill, "EDEDED" | "FFFFFF")).map(|(_, _, line)| line.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lores_point_colours_are_hues_only() {
        let c = lore_colors();
        assert!(c.len() >= 5);
        for x in &c {
            let v: Vec<i32> = (0..3).map(|i| i32::from_str_radix(&x[i * 2..i * 2 + 2], 16).unwrap()).collect();
            assert!(v.iter().max().unwrap() - v.iter().min().unwrap() > 40, "{x} is a grey");
        }
    }

    #[test]
    fn a_figures_own_colours_win_then_the_styles_then_lores() {
        let lore = lore_colors();
        let mut spec = TimelineSpec::default();
        let mut st = Style::default();
        assert_eq!(palette_for(&spec, &st, &lore), lore);
        st.colors.mode = "single".into();
        assert_eq!(palette_for(&spec, &st, &lore), vec![SINGLE.to_string()]);
        st.colors.palette = vec!["111111".into()];
        assert_eq!(palette_for(&spec, &st, &lore), vec!["111111".to_string()]);
        spec.colors = vec!["222222".into(), "not a colour".into()];
        assert_eq!(palette_for(&spec, &st, &lore), vec!["222222".to_string()]);
    }
}

/// The colour of a style that uses one colour and states none: Lore's blue.
pub const SINGLE: &str = "2F6DB0";

/// The palette a figure is drawn with: the figure's own colours ([`TimelineSpec::colors`]),
/// else the style's, else `lore` (Lore's figure colours; a one-colour style takes [`SINGLE`]).
pub fn palette_for(spec: &TimelineSpec, style: &Style, lore: &[String]) -> Vec<String> {
    let hex = |s: &&String| s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit());
    let own: Vec<String> = spec.colors.iter().filter(hex).cloned().collect();
    if !own.is_empty() {
        own
    } else if !style.colors.palette.is_empty() {
        style.colors.palette.clone()
    } else if style.colors.mode == "single" {
        vec![SINGLE.to_string()]
    } else {
        lore.to_vec()
    }
}

// ── the one entry ────────────────────────────────────────────────────────────

/// Draw `spec` in `style` into `frame` (pt): [`place::plan`], then [`draw::objects`].
/// `fonts` = the text faces (the flow theme's). An empty timeline is an error, one line.
pub fn render(spec: &TimelineSpec, style: &Style, fonts: &crate::theme::Font, frame: Rect, first_id: u32) -> Result<(Plan, Built), String> {
    if spec.points.is_empty() {
        return Err("時間軸沒有任何時間點".into());
    }
    let plan = place::plan(spec, style, frame);
    let built = draw::objects(spec, style, &plan, fonts, first_id);
    Ok((plan, built))
}

/// The neutral styles of this crate, in list order.
pub const BUILTIN: [(&str, &str); 3] = [
    ("line-below", include_str!("../../timelines/line-below.toml")),
    ("ring-alternate", include_str!("../../timelines/ring-alternate.toml")),
    ("cards", include_str!("../../timelines/cards.toml")),
];
