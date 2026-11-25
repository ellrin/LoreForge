//! Where everything goes (package T-place). Signatures are frozen; see `timeline/mod.rs`.

use super::{AxisGeom, Plan, Style, TimelineSpec};
use crate::layout::Rect;

/// Place `spec` in `style` inside `frame` (pt). Pure geometry: no objects, no colours but the
/// resolved colour of each point. Values the program cannot draw ([`Style::cannot`]) fall back
/// to the field's default, so a plan always comes back.
pub fn plan(spec: &TimelineSpec, style: &Style, frame: Rect) -> Plan {
    let _ = (spec, style);
    Plan { bounds: frame, title: None, legend: vec![], axis: AxisGeom { x0: frame.x, x1: frame.right(), y: frame.cy(), thickness: 0.0 }, points: vec![], scale: 1.0 }
}
