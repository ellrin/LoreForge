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
    g.edges.clear();
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
    let p = pack::pack(&g, &f, &m, a.get(5).map(String::as_str).unwrap_or(""), &air).expect("a figure with blocks");
    let built = build::build_with(&p.graph, &theme, &vars, &cat.library, &NoMath, &p.layout, &[], 2, &modules::no_images);
    let fig = Figure { layout: p.layout.clone(), routes: Vec::new(), built };
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
    println!("text {:.1} pt; page cover {:.0}%; slack across {:.0}% down {:.0}%; pictures over pictures {over}; pictures out of their area {out}", p.text_pt, 100.0 * p.cover, 100.0 * p.slack.0, 100.0 * p.slack.1);
    for ar in &p.areas {
        println!("  {}: pictures x{:.2}, cover {:.0}%, rows {}", ar.id, ar.grow, 100.0 * ar.cover, ar.rows.iter().map(|r| r.join(" ")).collect::<Vec<_>>().join(" | "));
    }
}
