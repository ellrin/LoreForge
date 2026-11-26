//! Count the stretches of lines drawn on a frame, per figure spec.
//! `cargo run -p lore-flow --example flow_hug -- <flow dir> <spec.json>...`
use lore_flow::NoMath;
use lore_flow::figure::{Catalogue, FlowSpec, render};

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("a flow dir");
    let cat = Catalogue::load(Some(std::path::Path::new(&dir)));
    let mut total = 0;
    for f in a {
        let spec = FlowSpec::from_value(serde_json::from_str(&std::fs::read_to_string(&f).expect("read")).expect("json")).expect("spec");
        let (fig, graph) = render(&spec, &cat, None, &NoMath, spec.frame_rect(), 2).expect("render");
        let n = lore_flow::route::hugging(&fig.layout, &fig.routes);
        let bends: usize = fig.routes.iter().map(|r| r.bends()).sum();
        total += n;
        let look = lore_flow::look::check(fig.objects());
        if std::env::var("LOOK_DEBUG").is_ok() {
            for o in fig.objects().iter().filter(|o| o.name.ends_with("edge:nd-mold")) {
                println!("  {} frame {:.1},{:.1} {:.1}x{:.1} flip {} {} {:?}", o.name, o.frame.x as f64 / 12700.0, o.frame.y as f64 / 12700.0, o.frame.w as f64 / 12700.0, o.frame.h as f64 / 12700.0, o.frame.flip_h, o.frame.flip_v, o.kind.sp().map(|s| s.geom.path.clone()));
            }
            for o in fig.objects().iter().filter(|o| o.name.ends_with("/title") && o.name.contains("region:")) {
                let tb = o.kind.sp().and_then(|s| s.text.clone()).unwrap();
                println!("  {} frame {:.0},{:.0} {:.0}x{:.0} anchor {} align {:?} sizes {:?}", o.name, o.frame.x as f64 / 12700.0, o.frame.y as f64 / 12700.0, o.frame.w as f64 / 12700.0, o.frame.h as f64 / 12700.0, tb.anchor, tb.paras.iter().map(|p| p.align.clone()).collect::<Vec<_>>(), tb.paras.iter().flat_map(|p| p.runs.iter().map(|r| (r.look.size, r.text.chars().count()))).collect::<Vec<_>>());
            }
        }
        let short = |s: &str| s.split('/').skip(1).collect::<Vec<_>>().join("/");
        println!("  line over words {}, line through an open shape {}, words cut by a shape or frame {}", look.over_text.len(), look.through_open.len(), look.words_cut.len());
        let (uneven, sparse) = lore_flow::look::frames(&fig.layout);
        let small_text = lore_flow::look::smallest_text(fig.objects(), fig.layout.bounds.w, 500.0);
        println!("  loose ends {}, line on line {}, line behind a block {}, line over a module {}, hidden arrowheads {}, uneven frames {}, sparse panels {}, smallest text at page width {:.1} pt", look.loose_ends.len(), look.on_line.len(), look.behind_block.len(), look.over_module.len(), look.hidden_heads.len(), uneven, sparse, small_text);
        // frame faults, on the graph `render` prepared (its nodes are in the layout's order)
        let ff = lore_flow::look::frame_faults(&graph, &fig.layout);
        println!("  frame faults {} (overlaps {}, titles {}, strays {})", ff.count(), ff.overlaps.len(), ff.titles.len(), ff.strays.len());
        for list in [&ff.overlaps, &ff.titles, &ff.strays] {
            for (a, b) in list.iter().take(3) {
                println!("    frame: {a} × {b}");
            }
        }
        // FORMAT="<width_pt> <w>:<h>": how the figure sits on a page of that width and shape
        if let Ok(fmt) = std::env::var("FORMAT") {
            let mut it = fmt.split_whitespace();
            let width: f64 = it.next().and_then(|w| w.parse().ok()).expect("FORMAT: <width_pt> <w>:<h>");
            let (rw, rh): (u32, u32) = it.next().and_then(|r| r.split_once(':')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).expect("FORMAT: <width_pt> <w>:<h>");
            let cover = lore_flow::look::page_cover(&fig.layout, (rw, rh));
            let text = lore_flow::look::page_text(fig.objects(), &fig.layout, width, (rw, rh));
            println!("  on the page {width} pt {rw}:{rh}: cover {:.0}%, smallest text {:.1} pt", 100.0 * cover, text);
        }
        for h in look.loose_ends.iter().take(4) {
            println!("    loose: {} ({})", short(&h.line), h.other);
        }
        for h in look.behind_block.iter().take(3) {
            println!("    behind: {}  ×  {}", short(&h.line), short(&h.other));
        }
        for h in look.on_line.iter().take(3) {
            println!("    on line: {}  ×  {}", short(&h.line), short(&h.other));
        }
        for h in look.words_cut.iter().take(6) {
            println!("    cut:   {}  ×  {}", short(&h.line), short(&h.other));
        }
        for h in look.over_text.iter().take(6) {
            println!("    words: {}  ×  {}  at {:.0},{:.0}", short(&h.line), short(&h.other), h.at.0, h.at.1);
        }
        for h in look.through_open.iter().take(4) {
            println!("    open:  {}  ×  {}", short(&h.line), short(&h.other));
        }
        // growable blocks smaller than four lines of text
        let fs = cat.theme(&spec.theme).font.size * fig.layout.scale;
        let g = lore_flow::dsl::parse_with(&spec.dsl, &cat.library.kinds()).ok();
        if let Some(g) = g {
            let small: Vec<String> = g.nodes.iter().zip(&fig.layout.nodes).filter(|(nd, b)| lore_flow::modules::growable(nd) && b.rect.h.max(b.rect.w) < 4.0 * fs).map(|(nd, b)| format!("{}({}) {:.1}x{:.1}", nd.id, nd.module, b.rect.w / fs, b.rect.h / fs)).collect();
            if !small.is_empty() {
                println!("  small: {}", small.join(", "));
            }
        }
        // how much of each framed panel its blocks fill: the content's box against the panel's inside
        let mut slack = Vec::new();
        for (pi, p) in fig.layout.panels.iter().enumerate() {
            let rs: Vec<_> = fig.layout.nodes.iter().filter(|nd| nd.panel == pi).map(|nd| nd.rect).collect();
            if rs.is_empty() || p.bare {
                continue;
            }
            let (x0, y0) = rs.iter().fold((f64::MAX, f64::MAX), |m, r| (m.0.min(r.x), m.1.min(r.y)));
            let (x1, y1) = rs.iter().fold((f64::MIN, f64::MIN), |m, r| (m.0.max(r.right()), m.1.max(r.bottom())));
            let area: f64 = rs.iter().map(|r| r.w * r.h).sum();
            slack.push(format!("{}:{:.0}%/{:.0}%", p.id, 100.0 * (x1 - x0) * (y1 - y0) / (p.rect.w * p.rect.h), 100.0 * area / (p.rect.w * p.rect.h)));
        }
        let used: f64 = fig.layout.nodes.iter().map(|nd| nd.rect.w * nd.rect.h).sum::<f64>() / (fig.layout.bounds.w * fig.layout.bounds.h);
        println!("{f}: on a frame {n}, bends {bends}, blocks cover {:.0}% of the figure; panels (content box / blocks) {}", 100.0 * used, slack.join(" "));
    }
    println!("total on a frame {total}");
}
