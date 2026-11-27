//! The base draft: a figure's blocks packed on a page, as large as they go, without lines.
//! `cargo run -p lore-flow --example flow_pack -- <spec.json|.flow> <width_pt> <w:h> <out.svg> <flow dir> [arrangement] [gap] [pad] [largest text pt]`
use lore_flow::Figure;
use lore_flow::figure::{Catalogue, FlowSpec};
use lore_flow::pack::{self, Air};
use lore_flow::plate::{Format, Measure};
use lore_flow::text::Vars;
use lore_flow::{NoMath, build, modules};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let text = std::fs::read_to_string(&a[0]).expect("read the figure");
    let spec = if a[0].ends_with(".json") {
        FlowSpec::from_value(serde_json::from_str(&text).expect("json")).expect("spec")
    } else {
        FlowSpec::from_value(serde_json::json!({ "lore_flow": 1, "dsl": text, "theme": "lore-macaron" })).expect("spec")
    };
    let (x, y) = a[2].split_once(':').expect("ratio w:h");
    let f = Format::new(a[1].parse().expect("width"), (x.parse().unwrap(), y.parse().unwrap()));
    let cat = Catalogue::load(Some(std::path::Path::new(&a[4])));
    let (mut g, theme, _) = spec.prepare(&cat, spec.frame_rect()).expect("prepare");
    for n in &mut g.nodes {
        for k in ["w", "h", "size", "grow"] {
            n.params.remove(k);
        }
    }
    let mut vars = Vars::default();
    vars.add_new(&g.vars_new);
    let m = Measure { theme: &theme, vars: &vars, library: &cat.library, image_size: &modules::no_images };
    let mut air = Air::default();
    if let Some(v) = a.get(6).and_then(|v| v.parse().ok()) {
        air.gap = v;
    }
    if let Some(v) = a.get(7).and_then(|v| v.parse().ok()) {
        air.pad = v;
    }
    if let Some(v) = a.get(8).and_then(|v| v.parse().ok()) {
        air.text_max_pt = v;
    }
    if let Some(v) = std::env::var("PATHS").ok().and_then(|v| v.parse().ok()) {
        air.paths = v;
    }
    // `GRID=1`: the pick of the one who draws, every stated grid kept
    air.stated = std::env::var("GRID").is_ok();
    // `OUTLINE=1`: the opening statement (areas, modules, frames, marks, paths) and what each
    // shape of the whole figure gives, before anything is placed
    if std::env::var("OUTLINE").is_ok() {
        print!("{}", lore_flow::outline::outline(&g).text());
        println!("整張圖的比例（寬 {} pt）：", f.width_pt);
        for s in lore_flow::outline::shapes(&g, f.width_pt, &m, a.get(5).map(String::as_str).unwrap_or(""), &air, &lore_flow::outline::SHAPES) {
            println!("  {}:{}  字 {:.1} pt，圖佔 {:.0}%{}", s.ratio.0, s.ratio.1, s.text_pt, s.cover * 100.0, if s.small { "（字太小）" } else { "" });
        }
        return;
    }
    let p = pack::pack(&g, &f, &m, a.get(5).map(String::as_str).unwrap_or(""), &air).expect("a figure with blocks");
    // the base draft has no lines; `LINES=1` draws them with the router as it is
    let mut p = p;
    let lines = std::env::var("LINES").is_ok();
    if !lines {
        p.graph.edges.clear();
    }
    let routes = match std::env::var("LINES").as_deref() {
        Ok("old") => lore_flow::route::route(&p.graph, &p.layout),
        Ok(_) => lore_flow::lines::lines(&p.graph, &p.layout),
        Err(_) => Vec::new(),
    };
    let built = build::build_with(&p.graph, &theme, &vars, &cat.library, &NoMath, &p.layout, &routes, 2, &modules::no_images);
    if lines {
        let bends: usize = routes.iter().map(|r| r.bends()).sum();
        let forced = routes.iter().filter(|r| r.forced).count();
        // lines over pictures they do not belong to, and lines that leave the page
        let (mut over, mut off) = (0, 0);
        for (e, r) in p.graph.edges.iter().zip(&routes) {
            let ends = [p.graph.node_index(&e.from), p.graph.node_index(&e.to)];
            let mut hit = false;
            for (i, n) in p.layout.nodes.iter().enumerate() {
                if ends.contains(&Some(i)) {
                    continue;
                }
                hit |= r.points.windows(2).any(|w| lore_flow::route::segment_hits(w[0], w[1], &n.art.inflate(-0.5)));
            }
            over += hit as usize;
            off += r.points.iter().any(|q| !p.layout.bounds.inflate(0.5).contains(&lore_flow::layout::Rect::new(q.0, q.1, 0.0, 0.0))) as usize;
        }
        // where placement leaves each flow line: its ends next to each other in a row, in a
        // column, or neither (which no routing can make straight)
        let (mut row, mut col, mut neither) = (0, 0, 0);
        for e in p.graph.edges.iter().filter(|e| e.kind == lore_flow::spec::EdgeKind::Data) {
            let (Some(a), Some(b)) = (p.graph.node_index(&e.from), p.graph.node_index(&e.to)) else { continue };
            let (a, b) = (&p.layout.nodes[a], &p.layout.nodes[b]);
            if (a.port_y - b.port_y).abs() < 1.0 {
                row += 1
            } else if (a.art.cx() - b.art.cx()).abs() < 1.0 {
                col += 1
            } else {
                neither += 1
            }
        }
        let broken = routes.iter().filter(|r| !lore_flow::lines::keeps_the_rule(r)).count();
        let under: usize = routes.iter().map(|r| lore_flow::lines::beneath(&p.graph, &p.layout, r)).sum();
        println!("lines against the rule (over two bends or doubling back) {broken}; blocks passed beneath {under}");
        println!("lines {}; bends {bends}; forced {forced}; hugging {}; over a picture {over}; off the page {off}; flow lines in a row {row}, in a column {col}, neither {neither}", routes.len(), lore_flow::route::hugging(&p.layout, &routes));
    }
    // what could not be drawn as asked (a line that does not show, a side that has no shape)
    for w in &built.warnings {
        println!("  warning: {w}");
    }
    let fig = Figure { layout: p.layout.clone(), routes, built };
    std::fs::write(&a[3], lore_flow::figure::svg_mm(&fig, None)).expect("write");
    // checks: nothing over anything, everything inside its area
    let lay = &p.layout;
    let mut over = 0;
    let mut out = 0;
    for i in 0..lay.nodes.len() {
        let r = lay.nodes[i].art;
        if !lay.panels[lay.nodes[i].panel].rect.inflate(0.5).contains(&r) {
            out += 1;
        }
        for j in i + 1..lay.nodes.len() {
            if r.inflate(-0.5).overlaps(&lay.nodes[j].art.inflate(-0.5)) {
                over += 1;
            }
        }
    }
    if p.small {
        println!("the text is under the least size");
    }
    println!("text {:.1} pt; page cover {:.0}%; slack across {:.0}% down {:.0}%; pictures over pictures {over}; pictures out of their area {out}", p.text_pt, 100.0 * p.cover, 100.0 * p.slack.0, 100.0 * p.slack.1);
    if !p.notes.is_empty() {
        println!("  notes (no room taken): {}; without free room: {}", p.notes.join(" "), if p.crowded.is_empty() { "none".to_string() } else { p.crowded.join(" ") });
    }
    for ar in &p.areas {
        println!("  {}{}: pictures x{:.2}, cover {:.0}%, rows {}", ar.id, if ar.grid { " (paths)" } else { "" }, ar.grow, 100.0 * ar.cover, ar.rows.iter().map(|r| r.join(" ")).collect::<Vec<_>>().join(" | "));
    }
}
