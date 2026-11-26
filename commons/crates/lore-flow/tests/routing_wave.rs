//! Where lines leave and enter blocks: the words side, an author's side, forks, branch points,
//! junctions, skips above and routes that keep to their panel.

use lore_canvas::Kind;
use lore_flow::layout::{Placed, Rect};
use lore_flow::modules::{Library, ModuleTemplate};
use lore_flow::route::{Route, Side, segment_hits};
use lore_flow::*;

fn drawn(src: &str) -> (Graph, Figure) {
    drawn_with(src, Drawing::default())
}

fn drawn_with(src: &str, d: Drawing) -> (Graph, Figure) {
    let names: Vec<String> = d.library.kinds();
    let g = dsl::parse_with(src, &names).unwrap_or_else(|e| panic!("{e:?}"));
    let f = draw(&g, &d).unwrap();
    (g, f)
}

fn node<'a>(g: &Graph, f: &'a Figure, id: &str) -> &'a Placed {
    &f.layout.nodes[g.node_index(id).unwrap()]
}

fn line(g: &Graph, f: &Figure, from: &str, to: &str) -> Route {
    f.routes.iter().find(|r| g.edges[r.edge].from == from && g.edges[r.edge].to == to).unwrap_or_else(|| panic!("no line {from} -> {to}")).clone()
}

/// The strip of a block that holds its words below (or above) the picture.
fn words_below(n: &Placed) -> Rect {
    Rect::new(n.reserve.x, n.art.bottom(), n.reserve.w, n.reserve.bottom() - n.art.bottom())
}
fn words_above(n: &Placed) -> Rect {
    Rect::new(n.reserve.x, n.reserve.y, n.reserve.w, n.art.y - n.reserve.y)
}

/// Does any run of the line touch (pass through or end inside) `r`?
fn touches(r: &Route, rect: &Rect) -> bool {
    let inside = |p: (f64, f64)| p.0 > rect.x + 1e-6 && p.0 < rect.right() - 1e-6 && p.1 > rect.y + 1e-6 && p.1 < rect.bottom() - 1e-6;
    r.points.windows(2).any(|w| segment_hits(w[0], w[1], rect)) || r.points.iter().any(|p| inside(*p))
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

// ── 1. words side ─────────────────────────────────────────────────────────────────

/// A picture with its words below it, in a top-down panel, feeding a block below it and to the
/// side (a line down cannot be straight): the natural way out is the bottom, through the words.
const CAPTION_DOWN: &str = "fig f\npanel p \"P\" dir=down\n  x: image \"A label under the picture\" caption=\"256 x 256\"\n  y: box \"Next\" lane=1\n  x -> y\nend\n";

/// The same with the block straight below the picture.
const CAPTION_STACKED: &str = "fig f\npanel p \"P\" dir=down\n  x: image \"A label under the picture\" caption=\"256 x 256\"\n  y: box \"Next\"\n  x -> y\nend\n";

#[test]
fn a_line_does_not_leave_through_a_caption() {
    let (g, f) = drawn(CAPTION_DOWN);
    let x = node(&g, &f, "x");
    assert!(x.words.1 && !x.words.0, "words below only: {:?}", x.words);
    let r = line(&g, &f, "x", "y");
    assert!(!r.forced);
    assert_ne!(r.from_side, Side::Bottom, "{:?}", r.points);
    assert!(!touches(&r, &words_below(x)), "{:?} touches the words {:?}", r.points, words_below(x));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert!(!f.built.warnings.iter().any(|w| w.contains("文字那一邊")), "{:?}", f.built.warnings);
}

/// Layer bars carry their label above the bars: fed from above and to the side in a top-down
/// panel.
const LABEL_ABOVE_DOWN: &str = "fig f\npanel p \"P\" dir=down\n  a: box \"Input\" lane=1\n  x: layer-bars \"Layers on top\" layers=[conv,conv,pool]\n  a -> x\nend\n";

#[test]
fn a_line_into_a_captioned_block_comes_from_a_free_side() {
    let (g, f) = drawn(LABEL_ABOVE_DOWN);
    let x = node(&g, &f, "x");
    assert!(x.words.0, "words above: {:?} art {:?} rect {:?}", x.words, x.art, x.rect);
    let r = line(&g, &f, "a", "x");
    assert!(!r.forced);
    assert_ne!(r.to_side, Side::Top, "{:?}", r.points);
    assert!(!touches(&r, &words_above(x)), "{:?} touches the words {:?}", r.points, words_above(x));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn when_only_the_words_side_is_left_the_line_stops_short_of_the_words() {
    // the author leaves no other side: the line leaves through the words' side, ends outside
    // the words with a gap of 2 pt (scaled), says so, and is not glued onto the picture
    let src = CAPTION_DOWN.replace("x -> y\n", "x -> y from=bottom\n");
    let (g, f) = drawn(&src);
    let x = node(&g, &f, "x");
    let k = f.layout.scale;
    let r = line(&g, &f, "x", "y");
    assert_eq!(r.from_side, Side::Bottom);
    let p = r.points[0];
    assert!(near(p.1, x.rect.bottom() + 2.0 * k), "{p:?} vs the words' bottom {}", x.rect.bottom());
    assert!(!touches(&r, &words_below(x)), "{:?}", r.points);
    assert!(r.warnings.iter().any(|w| w == "x → y: 線從文字那一邊進出"), "{:?}", r.warnings);
    let o = f.objects().iter().find(|o| o.name == "flow:f/edge:x-y").unwrap();
    let Kind::Connector(c) = &o.kind else { panic!("a connector: {:?}", o.kind) };
    assert!(c.start.is_none() && c.end.is_some(), "{:?} {:?}", c.start, c.end);
}

#[test]
fn a_straight_line_through_a_caption_side_stops_short_and_stays_straight() {
    let k = |f: &Figure| f.layout.scale;
    // down out of a picture with its words below, into the block straight under it
    let (g, f) = drawn(CAPTION_STACKED);
    let x = node(&g, &f, "x");
    assert!(x.words.1);
    let r = line(&g, &f, "x", "y");
    assert_eq!((r.from_side, r.to_side, r.bends()), (Side::Bottom, Side::Top, 0), "{:?}", r.points);
    assert!(near(r.points[0].1, x.rect.bottom() + 2.0 * k(&f)), "starts outside the words: {:?} vs {}", r.points, x.rect.bottom());
    assert!(near(r.points[1].1, node(&g, &f, "y").art.y), "{:?}", r.points);
    assert!(!touches(&r, &words_below(x)), "{:?}", r.points);
    assert!(r.warnings.is_empty(), "a straight line says nothing: {:?}", r.warnings);
    // down into layer bars with their words above: ends outside the words
    let (g, f) = drawn(&LABEL_ABOVE_DOWN.replace(" lane=1", ""));
    let x = node(&g, &f, "x");
    let r = line(&g, &f, "a", "x");
    assert_eq!((r.from_side, r.to_side, r.bends()), (Side::Bottom, Side::Top, 0), "{:?}", r.points);
    assert!(near(r.points.last().unwrap().1, x.rect.y - 2.0 * k(&f)), "ends outside the words: {:?} vs {}", r.points, x.rect.y);
    assert!(!touches(&r, &words_above(x)), "{:?}", r.points);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

// ── 2. the author's side ──────────────────────────────────────────────────────────

const CHAIN: &str = "fig m\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\na -> b -> c\n";

#[test]
fn from_and_to_sides_parse_and_round_trip() {
    let g = dsl::parse("fig m\na: box \"A\"\nb: box \"B\"\na -> b : skip from=top to=bottom\nb -> a : feedback \"back\" color=red to=left\n").unwrap();
    assert_eq!((g.edges[0].from_side.as_str(), g.edges[0].to_side.as_str()), ("top", "bottom"));
    assert_eq!((g.edges[1].from_side.as_str(), g.edges[1].to_side.as_str()), ("", "left"));
    let text = dsl::to_dsl(&g);
    assert!(text.contains("a -> b : skip from=top to=bottom\n"), "{text}");
    let back = dsl::parse(&text).unwrap();
    assert_eq!(back.edges, g.edges);
    // empty sides are not written to JSON
    let json = serde_json::to_string(&g.edges[1]).unwrap();
    assert!(!json.contains("from_side") && json.contains("\"to_side\":\"left\""), "{json}");
    let plain = serde_json::to_string(&dsl::parse(CHAIN).unwrap().edges[0]).unwrap();
    assert!(!plain.contains("_side"), "{plain}");
}

#[test]
fn a_chosen_side_is_kept() {
    let (g, f) = drawn(&CHAIN.replace("a -> b -> c\n", "a -> b from=top to=bottom\nb -> c to=top\n"));
    let r = line(&g, &f, "a", "b");
    assert!(!r.forced);
    assert_eq!((r.from_side, r.to_side), (Side::Top, Side::Bottom), "{:?}", r.points);
    let (a, b) = (node(&g, &f, "a"), node(&g, &f, "b"));
    assert!(near(r.points[0].1, a.art.y) && near(r.points.last().unwrap().1, b.art.bottom()), "{:?}", r.points);
    let r = line(&g, &f, "b", "c");
    assert_eq!(r.to_side, Side::Top);
    assert!(r.from_side != Side::Top || r.points.len() > 2);
    // without the choice the same lines go right to left
    let (g, f) = drawn(CHAIN);
    assert_eq!((line(&g, &f, "a", "b").from_side, line(&g, &f, "a", "b").to_side), (Side::Right, Side::Left));
}

#[test]
fn an_unknown_side_is_an_error() {
    let e = dsl::parse("fig m\na: box \"A\"\nb: box \"B\"\na -> b from=up\n").unwrap_err();
    assert_eq!(e.line, 4);
    assert!(e.message.contains("from=") && e.message.contains("up") && e.message.contains("top"), "{}", e.message);
    let e = dsl::parse("fig m\na: box \"A\"\nb: box \"B\"\na -> b to=\n").unwrap_err();
    assert_eq!(e.line, 4);
}

// ── 3. forks ──────────────────────────────────────────────────────────────────────

fn on_path(p: (f64, f64), path: &[(f64, f64)]) -> bool {
    path.windows(2).any(|w| p.0 >= w[0].0.min(w[1].0) - 1e-6 && p.0 <= w[0].0.max(w[1].0) + 1e-6 && p.1 >= w[0].1.min(w[1].1) - 1e-6 && p.1 <= w[0].1.max(w[1].1) + 1e-6)
}

/// `near` and `far` leave one point and run as one line until `near` turns off.
fn one_trunk(near: &Route, far: &Route) {
    assert_eq!(near.points[0], far.points[0], "one point: {:?} {:?}", near.points, far.points);
    assert_eq!(near.points[1], far.points[1], "one turn: {:?} {:?}", near.points, far.points);
    // the nearer line splits off the farther one
    assert!(on_path(near.points[2], &far.points), "{:?} splits off {:?}", near.points, far.points);
}

#[test]
fn two_lines_the_same_way_share_a_trunk() {
    let (g, f) = drawn("fig t\na: box \"A\"\nb: box \"B\" lane=1\nc: box \"C\" lane=2\na -> b\na -> c\n");
    let a = node(&g, &f, "a");
    let (rb, rc) = (line(&g, &f, "a", "b"), line(&g, &f, "a", "c"));
    assert_eq!((rb.from_side, rc.from_side), (Side::Right, Side::Right));
    assert!(near(rb.points[0].1, a.port_y), "from the middle: {:?}", rb.points);
    assert_eq!(rb.from_site, rc.from_site);
    one_trunk(&rb, &rc);
    // the farther target a column further on: on its own the search took the next column for
    // it, beside the first line; it turns where the nearer line turns
    let (g, f) = drawn("fig t\na: box \"A\"\nb: box \"B\" lane=1 col=1\nc: box \"C\" lane=2 col=2\na -> b\na -> c\n");
    one_trunk(&line(&g, &f, "a", "b"), &line(&g, &f, "a", "c"));
}

#[test]
fn a_fork_off_a_straight_main_line() {
    let (g, f) = drawn("fig t\na: box \"A\"\nm: box \"M\"\nb: box \"B\" lane=1\nc: box \"C\" lane=2 col=2\na -> m\na -> b\na -> c\n");
    let a = node(&g, &f, "a");
    let main = line(&g, &f, "a", "m");
    assert_eq!(main.points.len(), 2, "the main line is straight: {:?}", main.points);
    assert!(near(main.points[0].1, a.port_y));
    let (rb, rc) = (line(&g, &f, "a", "b"), line(&g, &f, "a", "c"));
    for r in [&rb, &rc] {
        assert_eq!(r.points[0], main.points[0], "off the main line's point: {:?}", r.points);
        assert!(on_path(r.points[1], &main.points), "turns off the main line: {:?} vs {:?}", r.points, main.points);
    }
    one_trunk(&rb, &rc);
}

// ── 4. branch point ───────────────────────────────────────────────────────────────

fn placed(src: &str, place: &[(&str, [f64; 2])]) -> (Graph, Figure) {
    let options = LayoutOptions { place: place.iter().map(|(k, v)| (k.to_string(), *v)).collect(), ..Default::default() };
    drawn_with(src, Drawing { options, ..Default::default() })
}

#[test]
fn a_branch_leaves_mid_way() {
    // d sits below and to the right: the old way left just before d, at the parent's far end
    let srcs = [
        ("fig m\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c\na>b -> d\n", vec![("d", [40.0, 80.0])]),
        ("fig m\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: box \"D\"\na -> b -> c\na>b -> d\n", vec![("d", [-40.0, 80.0]), ("b", [90.0, 0.0])]),
        ("fig m\na: box \"A\"\nb: box \"B\" lane=1\nd: box \"D\"\na -> b\na>b -> d\n", vec![("d", [60.0, -70.0])]),
    ];
    for (src, place) in srcs {
        let (g, f) = placed(src, &place);
        let k = f.layout.scale;
        let parent = line(&g, &f, "a", "b");
        let r = line(&g, &f, "a", "d");
        assert!(r.branch && !r.forced, "{r:?}");
        let p = r.points[0];
        // the parent's longest run, and the branch leaves its middle
        let runs: Vec<((f64, f64), (f64, f64))> = parent.points.windows(2).map(|w| (w[0], w[1])).collect();
        let long = runs.iter().copied().max_by(|x, y| {
            let l = |s: &((f64, f64), (f64, f64))| (s.0.0 - s.1.0).abs() + (s.0.1 - s.1.1).abs();
            l(x).partial_cmp(&l(y)).unwrap()
        });
        let (s, e) = long.unwrap();
        let mid = ((s.0 + e.0) / 2.0, (s.1 + e.1) / 2.0);
        assert!(near(p.0, mid.0) && near(p.1, mid.1), "{src}: leaves at {p:?}, the middle of the longest run is {mid:?} (parent {:?})", parent.points);
        let from_ends = ((p.0 - s.0).abs() + (p.1 - s.1).abs()).min((p.0 - e.0).abs() + (p.1 - e.1).abs());
        assert!(from_ends >= 8.0 * k - 1e-6, "{from_ends} from the run's ends");
    }
}

// ── 6. skips above, routes that keep to their panel ──────────────────────────────

#[test]
fn a_skip_goes_above() {
    // a block hangs under the row, so the row's bottom is busy; both skips still go over the top
    let (g, f) = drawn("fig s\na: box \"A\"\nb: box \"B\"\nc: box \"C\"\nd: box \"D\"\nu: box \"U\" lane=1 col=1\na -> b -> c -> d\nb -> u\nu -> d\na -> c : skip\nb -> d : skip\n");
    for (from, to) in [("a", "c"), ("b", "d")] {
        let r = line(&g, &f, from, to);
        assert!(!r.forced);
        assert_eq!((r.from_side, r.to_side), (Side::Top, Side::Top), "{from} -> {to}: {:?}", r.points);
        let top = node(&g, &f, from).rect.y.min(node(&g, &f, to).rect.y);
        assert!(r.points.iter().all(|p| p.1 <= top + 1e-6), "{from} -> {to} goes above the row: {:?}", r.points);
    }
    // a skip in a top-down panel still goes beside its column (unchanged)
    let (g, f) = drawn("fig s\npanel p \"P\" dir=down\n  a: box \"A\"\n  b: box \"B\"\n  c: box \"C\"\n  a -> b -> c\n  a -> c : skip\nend\n");
    let r = line(&g, &f, "a", "c");
    assert!(matches!(r.from_side, Side::Left | Side::Right), "{:?}", r.points);
}

/// Two panels, arranged as `arr` (`p q` side by side, `p / q` one above the other); `q`'s first
/// block is `w` × `h`.
fn two_panels(arr: &str, w: f64, h: f64) -> String {
    format!(
        "fig s\narrange {arr}\npanel p \"P\"\n  a: box \"A\"\n  b: box \"B\"\n  c: box \"C\"\n  u: box \"U\" lane=1 col=1\n  v: box \"V\" lane=2 col=2\n  a -> b -> c\n  a -> u\n  u -> c\n  a -> v\n  b -> c : skip\nend\npanel q \"Q\"\n  x: box \"X\" w={w} h={h}\n  y: box \"Y\"\n  z: box \"Z\" lane=1\n  x -> y\n  x -> z\n  y -> z : feedback\nend\n"
    )
}

#[test]
fn routes_in_one_panel_ignore_a_small_change_in_another() {
    // P's lines, measured from P's first block at the figure's natural size (the layout may move
    // the whole panel, and scale the figure to its frame)
    let p_lines = |src: &str| -> Vec<Vec<(f64, f64)>> {
        let (g, f) = drawn(src);
        let k = f.layout.scale;
        let o = node(&g, &f, "a").rect;
        let p = node(&g, &f, "a").panel;
        f.routes
            .iter()
            .filter(|r| f.layout.nodes[g.node_index(&g.edges[r.edge].from).unwrap()].panel == p)
            .map(|r| r.points.iter().map(|q| (((q.0 - o.x) / k * 1000.0).round() / 1000.0, ((q.1 - o.y) / k * 1000.0).round() / 1000.0)).collect())
            .collect()
    };
    // one above the other, Q's tracks used to run through P (before: 22 of 121 sizes moved a line)
    for arr in ["p q", "p / q", "q / p"] {
        let base = p_lines(&two_panels(arr, 60.0, 39.0));
        for w in (50..72).step_by(3) {
            for h in (30..52).step_by(3) {
                assert_eq!(p_lines(&two_panels(arr, w as f64, h as f64)), base, "{arr}: Q's block at {w} x {h}");
            }
        }
    }
}

// ── 5. junction ───────────────────────────────────────────────────────────────────

fn junction_library() -> Library {
    use lore_canvas::{Frame, Geom, Object, Paint, SpObj};
    let dot = Object { id: 0, name: "dot".into(), frame: Frame::new(0, 0, 76_200, 76_200), hidden: false, ph: None, slot: None, kind: Kind::Shape(SpObj { geom: Geom::preset("ellipse"), fill: Paint::solid("000000"), line: None, text: None, equation: false }), src: None };
    let t = ModuleTemplate { kind: "junction".into(), style: "dot".into(), w: 6.0, h: 6.0, objects: serde_json::to_value(vec![dot]).unwrap(), ..Default::default() };
    let mut lib = Library::default();
    lib.add(t).unwrap();
    lib
}

#[test]
fn no_arrowhead_into_a_junction() {
    let src = "fig j\na: box \"A\"\nb: box \"B\"\nj: junction\nc: box \"C\"\na -> j\nb -> j\nj -> c\n";
    let (g, f) = drawn_with(src, Drawing { library: junction_library(), ..Default::default() });
    let stroke = |name: &str| -> lore_canvas::Stroke {
        let o = f.objects().iter().find(|o| o.name == name).unwrap_or_else(|| panic!("{name}"));
        match &o.kind {
            Kind::Connector(c) => c.line.clone().unwrap(),
            Kind::Shape(s) => s.line.clone().unwrap(),
            k => panic!("{k:?}"),
        }
    };
    for from in ["a", "b"] {
        assert_eq!(stroke(&format!("flow:j/edge:{from}-j")).tail, "none");
    }
    // out of the junction the line keeps its head
    assert_ne!(stroke("flow:j/edge:j-c").tail, "none");
    let _ = line(&g, &f, "j", "c");
}
