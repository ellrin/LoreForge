//! Frame faults: frames lying over each other, titles touching or cut by another frame's
//! border, blocks outside their frame or inside one they do not belong to. Synthetic layouts
//! and one small real figure.

use lore_flow::layout::{Layout, LayoutOptions, PanelBox, Placed, RegionBox, Rect};
use lore_flow::look::frame_faults;
use lore_flow::spec::{Graph, Node, Region};
use lore_flow::*;

fn panel(id: &str, r: Rect, title: Option<Rect>) -> PanelBox {
    PanelBox { id: id.into(), title: id.into(), rect: r, title_rect: title, bare: false, down: false }
}

fn region(id: &str, r: Rect, title: Option<Rect>) -> RegionBox {
    RegionBox { id: id.into(), title: id.into(), rect: r, title_rect: title, ..Default::default() }
}

fn placed(r: Rect, panel: usize) -> Placed {
    Placed { rect: r, reserve: r, port_y: r.cy(), art: r, words: Default::default(), panel, layer: 0, lane: 0, single: false, ellipse: false, ends: Default::default(), origin: (0.0, 0.0) }
}

fn lay(panels: Vec<PanelBox>, regions: Vec<RegionBox>, nodes: Vec<Placed>) -> Layout {
    Layout { nodes, panels, regions, legend: Vec::new(), swatches: Vec::new(), bounds: Rect::new(0.0, 0.0, 400.0, 300.0), scale: 1.0 }
}

fn graph(ids: &[&str], regions: Vec<Region>) -> Graph {
    Graph { nodes: ids.iter().map(|i| Node { id: i.to_string(), ..Default::default() }).collect(), regions, ..Default::default() }
}

fn reg(id: &str, parent: &str, members: &[&str]) -> Region {
    Region { id: id.into(), parent: parent.into(), members: members.iter().map(|m| m.to_string()).collect(), ..Default::default() }
}

fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect::new(x, y, w, h)
}

fn pair(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

#[test]
fn two_frames_over_each_other_are_an_overlap() {
    let l = lay(vec![], vec![region("a", r(0.0, 0.0, 100.0, 80.0), None), region("b", r(60.0, 40.0, 100.0, 80.0), None), region("c", r(300.0, 0.0, 50.0, 50.0), None)], vec![]);
    let f = frame_faults(&graph(&[], vec![]), &l);
    assert_eq!(f.overlaps, vec![pair("a", "b")]);
    // frames that only touch along an edge are not over each other
    let t = lay(vec![], vec![region("a", r(0.0, 0.0, 100.0, 80.0), None), region("b", r(100.0, 0.0, 100.0, 80.0), None)], vec![]);
    assert!(frame_faults(&graph(&[], vec![]), &t).overlaps.is_empty());
}

#[test]
fn a_frame_inside_another_is_not() {
    let l = lay(vec![panel("p", r(0.0, 0.0, 300.0, 200.0), None)], vec![region("outer", r(10.0, 10.0, 200.0, 150.0), None), region("inner", r(20.0, 30.0, 100.0, 80.0), None)], vec![]);
    let f = frame_faults(&graph(&[], vec![]), &l);
    assert!(f.overlaps.is_empty(), "{:?}", f.overlaps);
    // a bare panel is no frame
    let mut bare = panel("q", r(0.0, 0.0, 50.0, 50.0), None);
    bare.bare = true;
    let l = lay(vec![bare], vec![region("a", r(30.0, 30.0, 100.0, 100.0), None)], vec![]);
    assert!(frame_faults(&graph(&[], vec![]), &l).overlaps.is_empty());
}

#[test]
fn a_region_sticking_out_of_its_panel_is_an_overlap() {
    let l = lay(vec![panel("p", r(0.0, 0.0, 200.0, 150.0), None)], vec![region("g", r(120.0, 50.0, 150.0, 60.0), None)], vec![]);
    let f = frame_faults(&graph(&[], vec![]), &l);
    assert_eq!(f.overlaps, vec![pair("p", "g")]);
}

#[test]
fn two_titles_touching_are_reported() {
    let l = lay(vec![], vec![region("a", r(0.0, 0.0, 100.0, 80.0), Some(r(10.0, 2.0, 60.0, 12.0))), region("b", r(110.0, 0.0, 100.0, 80.0), Some(r(60.0, 2.0, 60.0, 12.0)))], vec![]);
    let f = frame_faults(&graph(&[], vec![]), &l);
    assert!(f.titles.contains(&pair("a", "b")), "{:?}", f.titles);
    // titles that are apart are fine
    let l = lay(vec![], vec![region("a", r(0.0, 0.0, 100.0, 80.0), Some(r(10.0, 2.0, 60.0, 12.0))), region("b", r(110.0, 0.0, 100.0, 80.0), Some(r(120.0, 2.0, 60.0, 12.0)))], vec![]);
    assert!(frame_faults(&graph(&[], vec![]), &l).titles.is_empty());
}

#[test]
fn a_title_cut_by_another_frames_border_is_reported() {
    // the title of `a` runs out over the left border of `b`
    let l = lay(vec![], vec![region("a", r(0.0, 0.0, 100.0, 80.0), Some(r(30.0, 2.0, 100.0, 12.0))), region("b", r(110.0, 0.0, 100.0, 80.0), None)], vec![]);
    let f = frame_faults(&graph(&[], vec![]), &l);
    assert_eq!(f.titles, vec![pair("a", "b")]);
    // a title standing just above another frame's top edge is not cut
    let l = lay(vec![], vec![region("a", r(0.0, 0.0, 100.0, 40.0), Some(r(10.0, 2.0, 60.0, 12.0))), region("b", r(0.0, 40.0, 100.0, 40.0), None)], vec![]);
    assert!(frame_faults(&graph(&[], vec![]), &l).titles.is_empty());
}

#[test]
fn a_title_inside_its_own_frame_is_fine() {
    // the title is in its region, which is itself inside a panel that holds it wholly
    let l = lay(vec![panel("p", r(0.0, 0.0, 300.0, 200.0), Some(r(100.0, 2.0, 60.0, 12.0)))], vec![region("g", r(20.0, 30.0, 150.0, 100.0), Some(r(30.0, 32.0, 60.0, 12.0)))], vec![]);
    let f = frame_faults(&graph(&[], vec![]), &l);
    assert!(f.titles.is_empty(), "{:?}", f.titles);
}

fn two_in_a_region() -> (Graph, Vec<RegionBox>) {
    (graph(&["a", "b", "c"], vec![reg("g", "", &["a", "b"])]), vec![region("g", r(10.0, 10.0, 120.0, 60.0), None)])
}

#[test]
fn a_block_outside_its_region_is_a_stray() {
    let (g, regions) = two_in_a_region();
    let nodes = vec![placed(r(20.0, 20.0, 40.0, 30.0), 0), placed(r(150.0, 20.0, 40.0, 30.0), 0), placed(r(150.0, 100.0, 40.0, 30.0), 0)];
    let f = frame_faults(&g, &lay(vec![], regions, nodes));
    assert_eq!(f.strays, vec![pair("b", "g")]);
    // a member of a sub-region counts as a member of the outer one
    let g2 = graph(&["a", "b", "c"], vec![reg("g", "", &["a"]), reg("h", "g", &["b"])]);
    let regions = vec![region("g", r(10.0, 10.0, 120.0, 60.0), None), region("h", r(70.0, 20.0, 50.0, 40.0), None)];
    let nodes = vec![placed(r(20.0, 20.0, 40.0, 30.0), 0), placed(r(150.0, 20.0, 30.0, 30.0), 0), placed(r(150.0, 100.0, 40.0, 30.0), 0)];
    let f = frame_faults(&g2, &lay(vec![], regions, nodes));
    assert_eq!(f.strays, vec![pair("b", "g"), pair("b", "h")]);
}

#[test]
fn a_stranger_inside_a_region_is_a_stray() {
    let (g, regions) = two_in_a_region();
    let nodes = vec![placed(r(20.0, 20.0, 40.0, 30.0), 0), placed(r(70.0, 20.0, 40.0, 30.0), 0), placed(r(100.0, 30.0, 40.0, 30.0), 0)];
    let f = frame_faults(&g, &lay(vec![], regions, nodes));
    assert_eq!(f.strays, vec![pair("c", "g")]);
}

#[test]
fn a_block_outside_its_panel_is_a_stray() {
    let g = graph(&["a", "b"], vec![]);
    let nodes = vec![placed(r(20.0, 20.0, 40.0, 30.0), 0), placed(r(180.0, 20.0, 40.0, 30.0), 0)];
    let f = frame_faults(&g, &lay(vec![panel("p", r(0.0, 0.0, 200.0, 100.0), None)], vec![], nodes.clone()));
    assert_eq!(f.strays, vec![pair("b", "p")]);
    // a bare panel has no frame to be outside of
    let mut bare = panel("p", r(0.0, 0.0, 200.0, 100.0), None);
    bare.bare = true;
    assert!(frame_faults(&g, &lay(vec![bare], vec![], nodes)).strays.is_empty());
}

#[test]
fn a_clean_layout_has_no_faults() {
    let src = "fig f\npanel p \"Stage\"\nregion g \"Pair\"\na: box \"A\"\nb: box \"B\"\nend\nc: box \"C\"\na -> b\nb -> c\n";
    let g = dsl::parse(src).unwrap_or_else(|e| panic!("{e:?}"));
    let opt = LayoutOptions { frame: Rect::new(0.0, 0.0, 600.0, 300.0), ..Default::default() };
    let lay = layout::layout(&g, &Theme::classic_bw(), &Vars::default(), &Library::default(), &opt);
    assert_eq!(lay.regions.len(), 1, "the figure should have its region");
    assert_eq!(g.regions[0].members.len(), 2);
    let f = frame_faults(&g, &lay);
    assert_eq!(f.count(), 0, "{f:?}");
}
