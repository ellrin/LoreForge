//! The baseplate (docs/flow-blocks.md §10): footprints, budget, the plate of the format's ratio,
//! arrangements, spare room, the layout on cell borders. Small synthetic figures only.

use lore_flow::plate::{self, CELL, FRAME_PAD, Format, Measure, Placement};
use lore_flow::*;

fn graph(src: &str) -> Graph {
    dsl::parse(src).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Run `f` with the measuring things of the default drawing.
fn with_measure<T>(f: impl FnOnce(&Measure) -> T) -> T {
    let theme = Theme::classic_bw();
    let vars = Vars::default();
    let lib = Library::default();
    let m = Measure { theme: &theme, vars: &vars, library: &lib, image_size: &modules::no_images };
    f(&m)
}

fn best(g: &Graph, f: &Format) -> Placement {
    with_measure(|m| plate::place(g, f, m).expect("placements")[0].clone())
}

fn layout_of(g: &Graph, f: &Format, p: &Placement) -> Layout {
    with_measure(|m| plate::to_layout(g, f, p, m))
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn on_border(v: f64) -> bool {
    near(v / CELL, (v / CELL).round())
}

const CHAIN: &str = "fig c\npanel p \"Chain\"\n  a: box \"Alpha\"\n  b: box \"Bravo\"\n  c: box \"Delta\"\n  d: box \"Gamma\"\nend\na -> b -> c -> d\n";

// ── step 1: footprints ────────────────────────────────────────────────────────

#[test]
fn a_footprint_holds_the_block_and_its_words() {
    let g = graph("fig f\npanel p \"P\"\n  x: image \"a rather long label under it\"\n  b: box \"Words\"\nend\nx -> b\n");
    let f = Format::new(500.0, (16, 9));
    let fps = with_measure(|m| plate::footprints(&g, &f, m));
    let p = best(&g, &f);
    let lay = layout_of(&g, &f, &p);
    for fp in &fps {
        let n = &lay.nodes[fp.node];
        let room = n.rect.union(&n.reserve);
        // the picture, its words and a margin of a cell on each side, in its cells (grown or not)
        let cells = plate::rect_of(p.blocks[fp.node], CELL).inflate(-CELL);
        assert!(cells.contains(&room), "{fp:?} {cells:?} {room:?}");
        assert!(p.blocks[fp.node].w >= fp.w && p.blocks[fp.node].h >= fp.h);
    }
    // a block not enlarged: its footprint is what holds it
    let plain = &lay.nodes[1];
    assert!(fps[1].w as f64 * CELL >= plain.reserve.w.max(plain.rect.w) + 2.0 * CELL - 1e-6);
    assert!(fps[1].h as f64 * CELL >= plain.rect.h + 2.0 * CELL - 1e-6);
    // the picture may grow, the box is its words
    assert!(fps[0].grow > 1.0);
    assert_eq!(fps[1].grow, 1.0);
}

#[test]
fn footprints_are_whole_cells() {
    let g = graph(CHAIN);
    let f = Format::new(500.0, (16, 9));
    let fps = with_measure(|m| plate::footprints(&g, &f, m));
    let p = best(&g, &f);
    let lay = layout_of(&g, &f, &p);
    for fp in &fps {
        let n = &lay.nodes[fp.node];
        let room = n.rect.union(&n.reserve);
        // rounded up, at most a cell more on each side than the margin needs
        assert!(fp.w as f64 * CELL <= room.w + 4.0 * CELL + 1e-6, "{fp:?} {room:?}");
        assert!(fp.h as f64 * CELL <= room.h + 4.0 * CELL + 1e-6, "{fp:?} {room:?}");
        // a box does not grow: its cells on the plate are its footprint
        assert_eq!((p.blocks[fp.node].w, p.blocks[fp.node].h), (fp.w, fp.h));
    }
}

// ── step 2: rows and columns ──────────────────────────────────────────────────

#[test]
fn the_main_chain_shares_a_row() {
    let g = graph("fig m\npanel p \"P\"\n  a: box \"Alpha\"\n  b: box \"Bravo\"\n  c: box \"Delta\"\n  d: box \"Gamma\"\n  s: box \"Side\"\nend\na -> b -> c -> d\ns -> c : side\n");
    let f = Format::new(500.0, (16, 9));
    let lay = layout_of(&g, &f, &best(&g, &f));
    let n = |id: &str| &lay.nodes[g.node_index(id).unwrap()];
    for id in ["b", "c", "d"] {
        assert!(near(n(id).port_y, n("a").port_y), "{id} is off the chain's line");
    }
    assert!(n("b").art.cx() < n("c").art.cx() && n("c").art.cx() < n("d").art.cx());
    // a block joined by a side line: in the joined block's column, a row above or below
    assert!(near(n("s").art.cx(), n("c").art.cx()));
    assert!(!near(n("s").port_y, n("c").port_y));
}

#[test]
fn a_gap_grows_with_the_lines_that_pass() {
    // 1:100 = a plate exactly as wide as the figure: no spare room across, the gaps are the need
    let f = Format::new(500.0, (1, 100));
    let one = graph("fig g\npanel p \"P\"\n  a: box \"Alpha\"\n  b: box \"Bravo\"\nend\na -> b\n");
    let many = graph(&("fig g\npanel p \"P\"\n  a: box \"Alpha\"\n  b: box \"Bravo\"\n".to_string() + &(1..8).map(|i| format!("  c{i}: box \"Delta\" col=0 lane={i}\n")).collect::<String>() + "end\na -> b\n" + &(1..8).map(|i| format!("c{i} -> b\n")).collect::<String>()));
    let (p1, p2) = (best(&one, &f), best(&many, &f));
    assert_eq!(p1.columns.len(), 3, "{:?}", p1.columns);
    assert_eq!(p2.columns.len(), 3, "{:?}", p2.columns);
    // three lines turn in the gap between the columns: it is wider than for one straight line
    assert!(p2.columns[1] > p1.columns[1], "{:?} vs {:?}", p2.columns, p1.columns);
    // and in whole cells, at least one
    assert!(p1.columns[1] >= 1);
}

// ── step 3: the budget and the plate ──────────────────────────────────────────

#[test]
fn the_budget_adds_up() {
    let g = graph("fig b\npanel a \"A\"\n  x: image \"x\"\n  n: box \"Net\"\nend\npanel b \"B\"\n  y: box \"Out\"\n  l: loss \"L\"\nend\nx -> n -> y -> l\n");
    let f = Format::new(500.0, (16, 9));
    let fps = with_measure(|m| plate::footprints(&g, &f, m));
    let all = with_measure(|m| plate::place(&g, &f, m).unwrap());
    for p in &all {
        let b = &p.budget;
        assert_eq!(b.blocks, fps.iter().map(|fp| fp.w * fp.h).sum::<u32>());
        assert_eq!(b.blocks + b.margins + b.channels, b.least.0 * b.least.1, "{}", p.arrangement);
        assert!(b.channels > 0);
        assert!(p.plate.0 >= b.least.0 && p.plate.1 >= b.least.1);
    }
}

#[test]
fn the_plate_is_the_smallest_multiple_of_the_ratio_that_fits() {
    let g = graph(CHAIN);
    for r in [(16, 9), (5, 6), (1, 1)] {
        let f = Format::new(500.0, r);
        let p = best(&g, &f);
        let k = f.multiplier(p.budget.least);
        assert_eq!(p.plate, f.plate(k));
        // one step smaller would not hold the least arrangement
        if k > 1 {
            let (w, h) = f.plate(k - 1);
            assert!(w < p.budget.least.0 || h < p.budget.least.1);
        }
        // nothing is shrunk: every block keeps at least its footprint
        let fps = with_measure(|m| plate::footprints(&g, &f, m));
        for fp in &fps {
            assert!(p.blocks[fp.node].w >= fp.w && p.blocks[fp.node].h >= fp.h);
        }
    }
}

#[test]
fn any_ratio_works() {
    let g = graph("fig r\npanel a \"A\"\n  x: image \"x\"\n  n: box \"Net\"\n  y: image \"y\"\nend\npanel b \"B\"\n  q: box \"Q\"\n  w: label \"w\"\nend\nx -> n -> y\nq -> w\nw -> n : cross\n");
    for r in [(16, 9), (5, 6), (1, 1), (3, 4)] {
        let f = Format::new(250.0, r);
        let p = best(&g, &f);
        assert_eq!(p.plate.0 as u64 * r.1 as u64, p.plate.1 as u64 * r.0 as u64, "{r:?}: plate {:?}", p.plate);
        for (i, a) in p.blocks.iter().enumerate() {
            assert!(a.x + a.w <= p.plate.0 && a.y + a.h <= p.plate.1, "{r:?}: block {i} off the plate");
            for b in &p.blocks[i + 1..] {
                let apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{r:?}: blocks overlap {a:?} {b:?}");
            }
        }
        let lay = layout_of(&g, &f, &p);
        assert!(near(lay.bounds.w / lay.bounds.h, r.0 as f64 / r.1 as f64));
    }
}

#[test]
fn small_text_on_the_page_is_a_warning_with_ways() {
    let src = "fig s\npanel p \"P\"\n".to_string() + &(0..12).map(|i| format!("  n{i}: box \"Block number {i}\" col={i}\n")).collect::<String>() + "end\n" + &(0..11).map(|i| format!("n{i} -> n{}\n", i + 1)).collect::<String>();
    let g = graph(&src);
    let narrow = best(&g, &Format::new(100.0, (16, 9)));
    assert!(narrow.text_pt < 6.0);
    assert!(!narrow.ways.is_empty());
    assert!(narrow.ways.iter().any(|w| w.contains("pt")), "{:?}", narrow.ways);
    // still placed: every block has its cells
    assert!(narrow.blocks.iter().all(|c| c.w > 0 && c.h > 0));
    let wide = best(&g, &Format::new(5000.0, (16, 9)));
    assert!(wide.text_pt >= 6.0 && wide.ways.is_empty(), "{} {:?}", wide.text_pt, wide.ways);
    // the text on the page follows the cell on the page
    assert!(near(wide.text_pt / narrow.text_pt, 50.0 * narrow.plate.0 as f64 / wide.plate.0 as f64));
}

// ── step 4: placing ───────────────────────────────────────────────────────────

/// Three areas, each a chain of five with a block on a side line (about twice as wide as tall),
/// the text saying nothing of their arrangement.
fn three() -> Graph {
    three_with("")
}

fn three_with(arrange: &str) -> Graph {
    let mut s = format!("fig t\n{arrange}");
    for p in ["a", "b", "c"] {
        s += &format!("panel {p} \"{}\"\n", p.to_uppercase());
        for i in 1..=6 {
            s += &format!("  {p}{i}: box \"Step\"\n");
        }
        s += "end\n";
    }
    for p in ["a", "b", "c"] {
        s += &format!("{p}1 -> {p}2 -> {p}3 -> {p}4 -> {p}5\n{p}6 -> {p}3 : side\n");
    }
    graph(&s)
}

#[test]
fn the_options_differ_and_the_stated_one_is_among_them() {
    let f = Format::new(500.0, (16, 9));
    let offered = |g: &Graph| with_measure(|m| plate::place(g, &f, m).unwrap());
    // nothing stated: up to four different ways of cutting the areas into rows
    let free = offered(&three());
    assert!(free.len() >= 3 && free.len() <= plate::OPTIONS, "{}", free.len());
    let rows = |p: &Placement| p.arrangement.split_once(": ").unwrap().1.replace("(folded)", "");
    for (i, a) in free.iter().enumerate() {
        for b in &free[i + 1..] {
            assert_ne!(rows(a), rows(b), "the same arrangement offered twice");
        }
        // each option is complete
        assert!(a.text_pt > 0.0 && a.plate.0 > 0 && a.blocks.iter().all(|c| c.w > 0));
    }
    // three areas and no `arrange` line: nothing is stated
    assert!(free.iter().all(|p| !p.stated));
    // the author's line is offered, the others too
    let stated = offered(&three_with("arrange a b / c\n"));
    assert!(stated.iter().any(|p| p.stated && rows(p) == "a b / c"), "{:?}", stated.iter().map(|p| &p.arrangement).collect::<Vec<_>>());
    assert!(stated.len() > 1);
    // the user's pick, by name, is what gets placed
    let g = three();
    let pick = with_measure(|m| plate::arrangement(&g, &f, m, "two-rows: a / b c")).unwrap();
    assert_eq!(rows(&pick), "a / b c");
    assert!(pick.areas[1].y == pick.areas[2].y && pick.areas[0].y < pick.areas[1].y);
    assert!(with_measure(|m| plate::arrangement(&g, &f, m, "a c / b")).is_none(), "areas keep their order");
}

#[test]
fn options_come_largest_on_the_page_first() {
    for (r, arrange) in [((16, 9), ""), ((5, 6), ""), ((32, 9), "arrange a / b / c\n"), ((1, 1), "arrange a b c\n")] {
        let g = three_with(arrange);
        let f = Format::new(500.0, r);
        let all = with_measure(|m| plate::place(&g, &f, m).unwrap());
        // the stated one may take the last place out of turn; the rest are in order
        let ranked: Vec<&Placement> = all.iter().enumerate().filter(|(i, p)| !(p.stated && *i + 1 == all.len() && *i > 0)).map(|(_, p)| p).collect();
        for w in ranked.windows(2) {
            let (a, b) = (w[0], w[1]);
            // the fewest plate columns = the largest text on the page
            assert!(a.plate.0 <= b.plate.0, "{r:?}: {} before {}", a.arrangement, b.arrangement);
            assert!(a.text_pt >= b.text_pt - 1e-9);
            if a.plate.0 == b.plate.0 {
                assert!(a.cover >= b.cover - 1e-9, "{r:?}: same text, the fuller first");
            }
        }
        // the first is the best any arrangement gets
        let best_cols = all.iter().map(|p| p.plate.0).min().unwrap();
        assert_eq!(all[0].plate.0, best_cols);
    }
}

#[test]
fn the_stated_arrangement_is_always_offered_and_marked() {
    // stacked on a very wide page is the worst way: still offered, marked, exactly once
    let g = three_with("arrange a / b / c\n");
    for r in [(32, 9), (16, 9), (5, 6)] {
        let f = Format::new(500.0, r);
        let all = with_measure(|m| plate::place(&g, &f, m).unwrap());
        assert!(all.len() <= plate::OPTIONS);
        let stated: Vec<&Placement> = all.iter().filter(|p| p.stated).collect();
        assert_eq!(stated.len(), 1, "{r:?}");
        let s = stated[0];
        // as written: those rows, nothing folded, every area as its pins say
        assert_eq!(s.arrangement, "top-to-bottom: a / b / c", "{r:?}");
        assert_eq!(s.inner, vec!["stated"; 3]);
        // the others say how they arrange their areas inside
        for p in &all {
            assert_eq!(p.inner.len(), 3);
            for (k, v) in p.inner.iter().enumerate() {
                let folded = p.arrangement.contains(&format!("{}(folded)", ["a", "b", "c"][k]));
                assert_eq!(v, if folded { "two-rows" } else { "stated" }, "{}", p.arrangement);
            }
        }
    }
    // a figure of one area states itself
    let one = with_measure(|m| plate::place(&graph(CHAIN), &Format::new(500.0, (16, 9)), m).unwrap());
    assert!(one.iter().any(|p| p.stated && !p.arrangement.contains("folded")));
    // a pick by name knows whether it is the stated one
    let f = Format::new(500.0, (16, 9));
    assert!(with_measure(|m| plate::arrangement(&g, &f, m, "a / b / c")).unwrap().stated);
    assert!(!with_measure(|m| plate::arrangement(&g, &f, m, "a b c")).unwrap().stated);
}

#[test]
fn cover_is_the_blocks_over_the_plate() {
    let g = three();
    for r in [(16, 9), (1, 1)] {
        let f = Format::new(500.0, r);
        for p in with_measure(|m| plate::place(&g, &f, m).unwrap()) {
            let grown = with_measure(|m| plate::grown(&g, &f, &p, m));
            let lay = layout_of(&grown, &f, &p);
            let blocks: f64 = lay.nodes.iter().map(|n| n.rect.w * n.rect.h).sum();
            let plate_area = p.plate.0 as f64 * p.plate.1 as f64 * CELL * CELL;
            assert!(near(p.cover, blocks / plate_area), "{} {} vs {}", p.arrangement, p.cover, blocks / plate_area);
            assert!(near(p.cover, look::page_cover(&lay, r)));
            assert!(p.cover > 0.0 && p.cover < 1.0);
        }
    }
}

#[test]
fn the_ratio_decides_the_arrangement() {
    // the text says nothing; the plate decides which option comes first
    let g = three();
    let tall = best(&g, &Format::new(250.0, (5, 6)));
    let a = &tall.areas;
    assert!(a[0].y < a[1].y && a[1].y < a[2].y, "5:6 stacks them: {} {a:?}", tall.arrangement);
    assert!(a[0].x == a[1].x && a[1].x == a[2].x);
    let wide = best(&g, &Format::new(500.0, (32, 9)));
    let a = &wide.areas;
    assert!(a[0].x < a[1].x && a[1].x < a[2].x, "32:9 puts them side by side: {} {a:?}", wide.arrangement);
    assert!(a[0].y == a[1].y && a[1].y == a[2].y);
}

#[test]
fn neighbouring_areas_share_edges() {
    let g = three();
    for r in [(1, 1), (5, 6), (32, 9), (16, 9)] {
        let f = Format::new(500.0, r);
        for p in with_measure(|m| plate::place(&g, &f, m).unwrap()) {
            let a = &p.areas;
            for i in 0..a.len() {
                for j in i + 1..a.len() {
                    let (x, y) = (&a[i], &a[j]);
                    let side = x.y < y.y + y.h && y.y < x.y + x.h;
                    let stacked = x.x < y.x + y.w && y.x < x.x + x.w;
                    assert!(!(side && stacked), "{r:?} {}: areas overlap", p.arrangement);
                    // side by side: the same top and bottom; stacked: the same left and right
                    if side {
                        assert_eq!((x.y, x.h), (y.y, y.h), "{r:?} {}", p.arrangement);
                    }
                }
            }
            // every row of areas has the same left and right edge, on the plate
            let left = a.iter().map(|y| y.x).min().unwrap();
            let right = a.iter().map(|y| y.x + y.w).max().unwrap();
            assert!(right <= p.plate.0);
            for x in a {
                let row: Vec<_> = a.iter().filter(|y| y.y == x.y).collect();
                assert_eq!(row.iter().map(|y| y.x).min(), Some(left), "{r:?} {}", p.arrangement);
                assert_eq!(row.iter().map(|y| y.x + y.w).max(), Some(right), "{r:?} {}", p.arrangement);
            }
        }
    }
}

#[test]
fn spare_room_grows_the_pictures() {
    // a tall box beside the picture: the row has room down, the square plate room across
    let g = graph("fig p\npanel p \"P\"\n  x: image \"x\"\n  b: box \"1 \\\\ 2 \\\\ 3 \\\\ 4 \\\\ 5 \\\\ 6 \\\\ 7 \\\\ 8 \\\\ 9 \\\\ 10 \\\\ 11 \\\\ 12\"\nend\nx -> b\n");
    let f = Format::new(500.0, (1, 1));
    let fps = with_measure(|m| plate::footprints(&g, &f, m));
    let p = best(&g, &f);
    // the square plate leaves room down; the picture takes it, the box keeps its size
    assert!(p.blocks[0].w > fps[0].w || p.blocks[0].h > fps[0].h, "{:?} vs {:?}", p.blocks[0], fps[0]);
    assert_eq!((p.blocks[1].w, p.blocks[1].h), (fps[1].w, fps[1].h));
    let grown = with_measure(|m| plate::grown(&g, &f, &p, m));
    assert!(grown.nodes[0].num("grow").unwrap_or(1.0) > 1.0);
    assert!(grown.nodes[1].num("grow").is_none());
    // whole cells: the drawn picture lies in its cells
    let lay = layout_of(&grown, &f, &p);
    let n = &lay.nodes[0];
    assert!(plate::rect_of(p.blocks[0], CELL).contains(&n.rect.union(&n.reserve)));
}

#[test]
fn a_long_chain_folds_instead_of_shrinking() {
    let src = "fig l\npanel p \"P\"\n".to_string() + &(0..10).map(|i| format!("  n{i}: box \"Step {i}\"\n")).collect::<String>() + "end\n" + &(0..9).map(|i| format!("n{i} -> n{}\n", i + 1)).collect::<String>();
    let g = graph(&src);
    let f = Format::new(250.0, (1, 1));
    let all = with_measure(|m| plate::place(&g, &f, m).unwrap());
    assert!(all[0].arrangement.contains("folded"), "{}", all[0].arrangement);
    // one area: one option; the unfolded row is still there when asked for by name
    let flat = with_measure(|m| plate::arrangement(&g, &f, m, "p")).expect("the row as it is");
    assert!(!flat.arrangement.contains("folded"));
    // folding keeps the text larger than one long row on the same ratio
    assert!(all[0].text_pt > flat.text_pt, "{} vs {}", all[0].text_pt, flat.text_pt);
    // the blocks keep their size: footprints are not shrunk
    let fps = with_measure(|m| plate::footprints(&g, &f, m));
    for fp in &fps {
        assert!(all[0].blocks[fp.node].w >= fp.w);
    }
}

// ── step 5: the layout ────────────────────────────────────────────────────────

#[test]
fn the_layout_sits_on_cell_borders() {
    let g = graph("fig o\npanel a \"A\"\n  x: image \"x\"\n  n: box \"Net\"\n  l: loss \"L\"\nend\npanel b \"B\"\n  q: box \"Q\"\nend\nx -> n -> l\nq -> l : cross\n");
    let f = Format::new(500.0, (16, 9));
    let p = best(&g, &f);
    let (lay, g2) = with_measure(|m| (plate::to_layout(&g, &f, &p, m), plate::grown(&g, &f, &p, m)));
    assert_eq!(lay.scale, 1.0);
    assert!(near(lay.bounds.w, p.plate.0 as f64 * CELL) && near(lay.bounds.h, p.plate.1 as f64 * CELL));
    for pb in &lay.panels {
        for v in [pb.rect.x, pb.rect.y, pb.rect.right(), pb.rect.bottom()] {
            assert!(on_border(v), "panel {} edge {v} is off the cells", pb.id);
        }
    }
    for (i, n) in lay.nodes.iter().enumerate() {
        // the picture's middle on its column's line, the port line on its row's line
        assert!(on_border(n.art.cx()) && on_border(n.port_y), "{}: {:?}", g.nodes[i].id, n.art);
        assert!(plate::rect_of(p.blocks[i], CELL).contains(&n.rect.union(&n.reserve)), "{} leaves its cells", g.nodes[i].id);
    }
    // routed and drawn with the existing code
    let routes = route::route(&g2, &lay);
    assert_eq!(routes.len(), g.edges.len());
    let built = with_measure(|m| build::build(&g2, m.theme, m.vars, m.library, &NoMath, &lay, &routes, 2));
    assert!(look::check(&built.objects).loose_ends.is_empty());
}

// ── frames are computed from their content ───────────────────────────────────

#[test]
fn a_frame_is_its_content_plus_a_fixed_margin() {
    let f = Format::new(500.0, (16, 9));
    let frame = |src: &str| {
        let g = graph(src);
        let fps = with_measure(|m| plate::footprints(&g, &f, m));
        let p = best(&g, &f);
        let fl = with_measure(|m| plate::fills(&g, &f, &p, m)).remove(0);
        (fps, fl)
    };
    // one block: at the least arrangement the frame is its footprint and the fixed margin all
    // round, nothing else
    let (fps, fl) = frame("fig k\npanel a\n  x: box \"Alpha\"\n  y: box \"Bravo\"\nend\nx -> y\n");
    let ((fw, fh), (cw, ch)) = fl.before;
    assert_eq!(fh, fps[0].h.max(fps[1].h) + 2 * FRAME_PAD, "{fl:?}");
    assert!(fw > fps[0].w + fps[1].w + 2 * FRAME_PAD, "and the channel between them: {fl:?}");
    // the content box is the frame less that margin and the footprints' own cell
    assert!(fw - cw <= 2 * (FRAME_PAD + 1) + 1 && fh - ch <= 2 * (FRAME_PAD + 1) + 1, "{fl:?}");
    // a title adds its row on top, in place of the top margin
    let (fps, fl) = frame("fig k\npanel a \"A title\"\n  x: box \"Alpha\"\nend\n");
    let ((_, fh), _) = fl.before;
    assert!(fh > fps[0].h + 2 * FRAME_PAD && fh <= fps[0].h + FRAME_PAD + 8, "{fl:?}");
}

#[test]
fn a_shorter_neighbour_grows_its_blocks_before_its_margins() {
    // a tall panel beside a short one: the short one's picture takes the height first
    let g = graph("fig s\npanel a \"A\"\n  t: box \"One \\\\ Two \\\\ Three \\\\ Four \\\\ Five \\\\ Six \\\\ Seven\"\n  u: box \"Next\"\nend\npanel b \"B\"\n  x: image \"x\"\nend\nt -> u\nu -> x : cross\n");
    let fps = with_measure(|m| plate::footprints(&g, &Format::new(500.0, (4, 1)), m));
    let f = Format::new(500.0, (4, 1));
    let all = with_measure(|m| plate::place(&g, &f, m).unwrap());
    let p = all.iter().find(|p| p.areas[0].y == p.areas[1].y).expect("an arrangement side by side");
    let xi = g.node_index("x").unwrap();
    let c = p.blocks[xi];
    assert!(c.h > fps[xi].h, "the picture grew: {c:?} vs {:?}", fps[xi]);
    // what is left is margin, equal above and below the one row
    let b = p.areas[1];
    let lay = layout_of(&g, &f, p);
    let frame = &lay.panels[1].rect;
    let top = c.y as f64 * CELL - (frame.y + 0.0);
    let bottom = frame.bottom() - (c.y + c.h) as f64 * CELL;
    let title = lay.panels[1].title_rect.map(|t| t.bottom() - frame.y).unwrap_or(0.0);
    assert!(top >= title - 1e-6, "the block stays under the title");
    assert!(b.h >= c.h);
    assert!((top - title - bottom).abs() <= 2.0 * CELL + 1e-6 || (top - bottom).abs() <= 2.0 * CELL, "top {top} bottom {bottom} title {title}");
}

#[test]
fn blocks_of_one_kind_in_a_row_grow_alike() {
    // two pictures with words of different length beside a tall box, and a picture of the same
    // kind in another area: one enlargement for all of them, so a row reads as one size
    let g = graph("fig a\npanel p \"P\"\n  x: image \"x\"\n  y: image \"a much longer label under this one\"\n  t: box \"1 \\\\ 2 \\\\ 3 \\\\ 4 \\\\ 5 \\\\ 6 \\\\ 7 \\\\ 8 \\\\ 9 \\\\ 10\"\nend\npanel q \"Q\"\n  z: image \"z\"\n  s: image-stack \"s\"\nend\nx -> y -> t\nz -> s\nt -> z : cross\n");
    let mut grew = false;
    for r in [(1, 1), (16, 9), (5, 6)] {
        let f = Format::new(500.0, r);
        for p in with_measure(|m| plate::place(&g, &f, m).unwrap()) {
            let grown = with_measure(|m| plate::grown(&g, &f, &p, m));
            let k = |id: &str| grown.node(id).unwrap().num("grow").unwrap_or(1.0);
            assert_eq!(k("x"), k("y"), "{r:?} {}: one row, one size", p.arrangement);
            assert_eq!(k("x"), k("z"), "{r:?} {}: one kind, one size", p.arrangement);
            // the row of z holds s too
            assert_eq!(k("z"), k("s"), "{r:?} {}", p.arrangement);
            grew |= k("x") > 1.0;
            // and the layout draws them as placed: each in its cells
            let lay = layout_of(&grown, &f, &p);
            for (i, n) in lay.nodes.iter().enumerate() {
                assert!(plate::rect_of(p.blocks[i], CELL).contains(&n.rect.union(&n.reserve)), "{}", g.nodes[i].id);
            }
        }
    }
    assert!(grew, "some arrangement had room to grow them");
}

// ── regions in the budget ─────────────────────────────────────────────────────

/// A chain through two module frames side by side, the second holding a frame of its own; titles
/// above, inside and below; one long title over two short blocks.
const FRAMED: &str = "fig r\npanel p \"P\"\n  x: box \"Input\"\n  region r1 \"A rather long module title over two blocks\" title=above\n    a: box \"Alpha\"\n    b: box \"Bravo\"\n  end\n  region r2 \"Second\" title=below\n    c: box \"Delta\"\n    region r3 \"Inner\"\n      d: box \"Gamma\"\n      e: box \"Echo\"\n    end\n  end\n  y: box \"Out\"\nend\nx -> a -> b -> c -> d -> e -> y\n";

/// As a model writes them: every block pinned, two frames stacked in the same columns, a block
/// outside them in the first column and one under them, a frame with its title at the left.
const PINNED: &str = "fig q\npanel p \"P\"\n  x: box \"Input\" col=0 lane=0\n  region top \"Upper module\" title=above\n    a: box \"Alpha\" col=1 lane=0\n    b: box \"Bravo\" col=2 lane=0\n  end\n  region low \"Lower module with a title\"\n    c: box \"Delta\" col=1 lane=1\n    d: box \"Gamma\" col=2 lane=1\n  end\n  region side \"Side\" title=left\n    e: box \"Echo\" col=3 lane=0\n  end\n  z: box \"Under\" col=2 lane=2\nend\nx -> a -> b -> e\nx -> c -> d -> e\nd -> z\n";

fn framed_layouts() -> Vec<(String, Graph, Layout)> {
    let mut out = Vec::new();
    for src in [FRAMED, PINNED] {
        let g = graph(src);
        for r in [(16, 9), (5, 2), (1, 1), (5, 6)] {
            let f = Format::new(500.0, r);
            for p in with_measure(|m| plate::place(&g, &f, m).unwrap()) {
                assert_eq!(p.regions.len(), g.regions.len());
                let grown = with_measure(|m| plate::grown(&g, &f, &p, m));
                let lay = layout_of(&grown, &f, &p);
                // the frames are the cells reserved for them, on cell borders
                for rb in &lay.regions {
                    let i = g.regions.iter().position(|x| x.id == rb.id).unwrap();
                    assert_eq!(rb.rect, plate::rect_of(p.regions[i], CELL), "{}", rb.id);
                    for v in [rb.rect.x, rb.rect.y, rb.rect.right(), rb.rect.bottom()] {
                        assert!(on_border(v), "{} off the cells", rb.id);
                    }
                }
                out.push((format!("{} {r:?} {}", g.nodes[0].id, p.arrangement), g.clone(), lay));
            }
        }
    }
    out
}

fn meets(a: &Rect, b: &Rect) -> bool {
    a.x < b.right() - 1e-6 && b.x < a.right() - 1e-6 && a.y < b.bottom() - 1e-6 && b.y < a.bottom() - 1e-6
}

fn holds(a: &Rect, b: &Rect, by: f64) -> bool {
    a.x + by <= b.x + 1e-6 && a.y + by <= b.y + 1e-6 && a.right() - by >= b.right() - 1e-6 && a.bottom() - by >= b.bottom() - 1e-6
}

/// Every node a region frames, its sub-regions' included.
fn framed(g: &Graph, id: &str) -> Vec<String> {
    let mut ids = vec![id.to_string()];
    let mut i = 0;
    while i < ids.len() {
        let kids: Vec<String> = g.regions.iter().filter(|r| r.parent == ids[i]).map(|r| r.id.clone()).collect();
        ids.extend(kids);
        i += 1;
    }
    g.regions.iter().filter(|r| ids.contains(&r.id)).flat_map(|r| r.members.clone()).collect()
}

fn nested(g: &Graph, inner: &str, outer: &str) -> bool {
    let mut at = inner.to_string();
    while let Some(r) = g.regions.iter().find(|r| r.id == at) {
        if r.parent == outer {
            return true;
        }
        at = r.parent.clone();
    }
    false
}

#[test]
fn region_frames_never_overlap() {
    for (what, g, lay) in framed_layouts() {
        for (i, a) in lay.regions.iter().enumerate() {
            // inside its panel, a padding clear of the panel's frame
            let panel = &lay.panels[0].rect;
            assert!(holds(panel, &a.rect, CELL), "{what}: {} touches its panel", a.id);
            for b in &lay.regions[i + 1..] {
                if nested(&g, &a.id, &b.id) || nested(&g, &b.id, &a.id) {
                    continue;
                }
                // siblings: apart, with a channel between them a line can run in clear of both
                let (ga, gb) = (a.rect.inflate(10.0 - 1e-6), b.rect.inflate(10.0 - 1e-6));
                assert!(!meets(&ga, &gb), "{what}: {} and {} closer than a channel: {:?} {:?}", a.id, b.id, a.rect, b.rect);
            }
        }
    }
}

#[test]
fn a_region_title_has_its_own_room() {
    for (what, g, lay) in framed_layouts() {
        let rooms: Vec<Rect> = lay.nodes.iter().map(|n| n.rect.union(&n.reserve)).collect();
        for a in &lay.regions {
            let t = a.title_rect.expect("every test frame has a title");
            // never wider than its frame (upright at the left: never taller)
            if a.title_at == "left" {
                assert!(holds(&a.rect, &t, 0.0), "{what}: {}", a.id);
            } else {
                assert!(t.x >= a.rect.x - 1e-6 && t.right() <= a.rect.right() + 1e-6, "{what}: {} title {t:?} frame {:?}", a.id, a.rect);
            }
            match a.title_at.as_str() {
                "above" => assert!(t.bottom() <= a.rect.y + 1e-6 && t.y >= lay.panels[0].rect.y, "{what}: {}", a.id),
                "below" => assert!(t.y >= a.rect.bottom() - 1e-6 && t.bottom() <= lay.panels[0].rect.bottom(), "{what}: {}", a.id),
                _ => assert!(holds(&a.rect, &t, 0.0), "{what}: {}", a.id),
            }
            // clear of every block, every other title, and of the frames it is not inside
            for (ni, r) in rooms.iter().enumerate() {
                assert!(!meets(&t, r), "{what}: the title of {} lies on {}", a.id, g.nodes[ni].id);
            }
            for b in lay.regions.iter().filter(|b| b.id != a.id) {
                if let Some(u) = b.title_rect {
                    assert!(!meets(&t, &u), "{what}: titles of {} and {} touch", a.id, b.id);
                }
                assert!(!meets(&t, &b.rect) || holds(&b.rect, &t, 0.0), "{what}: the frame of {} crosses the title of {}", b.id, a.id);
            }
        }
    }
}

#[test]
fn a_block_stays_inside_its_region_and_out_of_the_others() {
    for (what, g, lay) in framed_layouts() {
        for a in &lay.regions {
            let own = framed(&g, &a.id);
            for (ni, n) in lay.nodes.iter().enumerate() {
                let room = n.rect.union(&n.reserve);
                if own.contains(&g.nodes[ni].id) {
                    // inside, a padding from the frame
                    assert!(holds(&a.rect, &room, 4.0), "{what}: {} leaves {}", g.nodes[ni].id, a.id);
                } else {
                    assert!(!meets(&a.rect, &room), "{what}: {} lies in {}", g.nodes[ni].id, a.id);
                }
            }
        }
    }
}

#[test]
fn a_nested_region_lies_inside_its_parent() {
    let mut seen = false;
    for (what, g, lay) in framed_layouts() {
        for a in &lay.regions {
            let Some(parent) = g.regions.iter().find(|r| r.id == a.id).map(|r| r.parent.clone()).filter(|p| !p.is_empty()) else { continue };
            let p = lay.regions.iter().find(|r| r.id == parent).unwrap();
            // wholly inside, a padding from the parent's frame, under the parent's title
            assert!(holds(&p.rect, &a.rect, CELL), "{what}: {} out of {}: {:?} {:?}", a.id, p.id, a.rect, p.rect);
            if let (Some(t), Some(pt)) = (a.title_rect, p.title_rect) {
                assert!(!meets(&t, &pt), "{what}");
            }
            seen = true;
        }
    }
    assert!(seen);
}

// ── the figure is the plate ───────────────────────────────────────────────────

#[test]
fn the_frames_reach_the_edges_of_the_plate() {
    for (g, rs) in [(three(), vec![(16, 9), (5, 6), (32, 9), (1, 1)]), (graph(FRAMED), vec![(16, 9), (5, 2)]), (graph(CHAIN), vec![(1, 1), (16, 9)])] {
        for r in rs {
            let f = Format::new(500.0, r);
            for p in with_measure(|m| plate::place(&g, &f, m).unwrap()) {
                let a = &p.areas;
                // no band of the plate outside the areas: the left of every row of areas is the
                // plate's left, its right the plate's right; the first row at the top, the last
                // at the bottom
                let rows: std::collections::BTreeSet<u32> = a.iter().map(|x| x.y).collect();
                for y in &rows {
                    let row: Vec<_> = a.iter().filter(|x| x.y == *y).collect();
                    assert_eq!(row.iter().map(|x| x.x).min(), Some(0), "{r:?} {}", p.arrangement);
                    assert_eq!(row.iter().map(|x| x.x + x.w).max(), Some(p.plate.0), "{r:?} {}", p.arrangement);
                }
                assert_eq!(a.iter().map(|x| x.y).min(), Some(0), "{r:?} {}", p.arrangement);
                assert_eq!(a.iter().map(|x| x.y + x.h).max(), Some(p.plate.1), "{r:?} {}", p.arrangement);
                // the rows of areas stack without a band between them other than the gap
                let lay = layout_of(&g, &f, &p);
                for pb in &lay.panels {
                    // the frame is its area (a title written outside takes its band from the area)
                    let ar = plate::rect_of(a[lay.panels.iter().position(|x| x.id == pb.id).unwrap()], CELL);
                    assert!(near(pb.rect.x, ar.x) && near(pb.rect.right(), ar.right()), "{r:?} {}", p.arrangement);
                    assert!(near(pb.rect.bottom(), ar.bottom()) || pb.title_rect.is_some_and(|t| t.y >= pb.rect.bottom()));
                }
            }
        }
    }
}

#[test]
fn areas_of_one_row_are_equally_tall() {
    let g = three();
    for r in [(16, 9), (5, 6), (32, 9), (1, 1), (3, 1)] {
        let f = Format::new(500.0, r);
        let mut all = with_measure(|m| plate::place(&g, &f, m).unwrap());
        all.push(with_measure(|m| plate::arrangement(&g, &f, m, "a b / c")).unwrap());
        all.push(with_measure(|m| plate::arrangement(&g, &f, m, "a / b c")).unwrap());
        for p in &all {
            let a = &p.areas;
            for x in a {
                for y in a {
                    let side = x.y < y.y + y.h && y.y < x.y + x.h;
                    let stacked = x.x < y.x + y.w && y.x < x.x + x.w;
                    if side {
                        assert_eq!((x.y, x.h), (y.y, y.h), "{r:?} {}: one row, one height", p.arrangement);
                    }
                    // stacked alone in their rows: one width, the plate's
                    if stacked && !side && a.iter().filter(|z| z.y == x.y).count() == 1 && a.iter().filter(|z| z.y == y.y).count() == 1 {
                        assert_eq!((x.x, x.w), (y.x, y.w), "{r:?} {}", p.arrangement);
                    }
                }
            }
        }
    }
}

#[test]
fn spare_room_evens_the_gaps_and_keeps_the_lines() {
    // a wide page for a stacked figure: the spare room across goes between the columns, evenly,
    // and blocks of one row stay on one port line, blocks of one column on one middle line
    let g = graph(CHAIN);
    let f = Format::new(500.0, (5, 1));
    let p = with_measure(|m| plate::arrangement(&g, &f, m, "p")).unwrap();
    let gaps: Vec<u32> = p.columns.iter().skip(1).step_by(2).copied().collect();
    assert_eq!(gaps.len(), 3);
    assert!(gaps.iter().max().unwrap() - gaps.iter().min().unwrap() <= 1, "{gaps:?}");
    let lay = layout_of(&g, &f, &p);
    for n in &lay.nodes[1..] {
        assert!(near(n.port_y, lay.nodes[0].port_y));
    }
}

// ── what binds ────────────────────────────────────────────────────────────────

#[test]
fn binding_names_the_direction_and_the_areas_that_set_it() {
    let g = three();
    // stacked on a very wide page: the height binds, every area sets it, room is left across
    let f = Format::new(500.0, (32, 9));
    let p = with_measure(|m| plate::arrangement(&g, &f, m, "a / b / c")).unwrap();
    let b = with_measure(|m| plate::binding(&g, &f, &p, m));
    assert!(b.tall);
    assert_eq!((b.need, b.plate), (p.budget.least, p.plate));
    assert_eq!(b.spare, p.plate.0 - b.need.0);
    assert_eq!(b.areas.len(), 3);
    assert!(b.areas.iter().all(|x| x.binds && x.cols >= 2 && x.rows >= 1 && x.w > 0 && x.h > 0));
    assert!(b.areas.iter().map(|x| x.h).sum::<u32>() <= b.need.1);
    // the same cells in the plate's shape would give larger text
    assert!(b.text_if_shaped_pt > b.text_pt, "{} vs {}", b.text_if_shaped_pt, b.text_pt);
    assert!(near(b.text_pt, p.text_pt));
    // side by side on a tall page: the width binds, room is left down
    let f = Format::new(500.0, (5, 6));
    let p = with_measure(|m| plate::arrangement(&g, &f, m, "a b c")).unwrap();
    let b = with_measure(|m| plate::binding(&g, &f, &p, m));
    assert!(!b.tall);
    assert_eq!(b.spare, p.plate.1 - b.need.1);
    assert!(b.areas.iter().all(|x| x.binds));
    // two rows, the wider on top: across, only the areas of the widest row set the width
    let p = with_measure(|m| plate::arrangement(&g, &f, m, "a b / c")).unwrap();
    let b = with_measure(|m| plate::binding(&g, &f, &p, m));
    assert!(!b.tall, "a b / c on 5:6 is held by its width");
    let ids: Vec<(&str, bool)> = b.areas.iter().map(|x| (x.id.as_str(), x.binds)).collect();
    assert_eq!(ids, vec![("a", true), ("b", true), ("c", false)]);
    // exactly the plate's shape: no more to win
    let shaped = (b.need.0 as f64 * b.need.1 as f64 * 5.0 / 6.0).sqrt();
    assert!(near(b.text_if_shaped_pt / b.text_pt, p.plate.0 as f64 / shaped));
}
