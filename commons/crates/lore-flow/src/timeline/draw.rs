//! The plan as objects (package T-draw). Signatures are frozen; see `timeline/mod.rs`.

use super::{Plan, Style, TimelineSpec};
use crate::build::Built;

/// Every part of `plan` as a `lore_canvas` object, back to front (axis, stems, markers, cards,
/// texts), ids from `first_id` up. Names are `flow:<fig>/<part>` ([`crate::build::node_name`]
/// with the parts `axis`, `title`, `legend-<i>`, and for point `i` (from 1) `p<i>`,
/// `p<i>-stem`, `p<i>-time`, `p<i>-title`, `p<i>-body`, `p<i>-card`, `p<i>-icon` …), so
/// [`crate::build::is_flow_object`] finds them when the figure is edited again.
pub fn objects(spec: &TimelineSpec, style: &Style, plan: &Plan, fonts: &crate::theme::Font, first_id: u32) -> Built {
    let _ = (spec, style, plan, fonts);
    Built { objects: vec![], node_ids: Default::default(), next_id: first_id, pictures: vec![], warnings: vec![] }
}
