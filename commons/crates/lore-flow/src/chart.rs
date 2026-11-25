//! A small chart inside a flow figure (kind `chart`): a distribution, a few curves, a bar
//! chart, clusters, a surface. Drawn as plain canvas shapes (lines and filled paths) so it
//! stays vector on a slide; what the curves are comes from the plotting tool's own functions
//! (`lore-plot`), never from a picture.

use std::collections::BTreeMap;

use lore_canvas::Object;
use serde_json::Value;

/// The chart kinds a block may ask for (`chart=`).
pub const KINDS: [&str; 6] = ["bell", "bells", "membership", "bars", "scatter", "surface"];

/// The shapes of one chart in a `w` × `h` box (pt, origin top-left), drawn in `ink` (`RRGGBB`)
/// with the series in the document palette. `params` are the block's (`chart`, `n`, `labels`,
/// …). Object ids are 0 and names `chart`, `chart-2` …. Empty = no such chart (the block then
/// draws a dashed slot and says so).
pub fn objects(params: &BTreeMap<String, Value>, w: f64, h: f64, ink: &str) -> Vec<Object> {
    let _ = (params, w, h, ink);
    Vec::new()
}
