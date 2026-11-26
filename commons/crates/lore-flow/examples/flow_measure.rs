//! Measure the spacing of drawn figures as ratios of the text size: the gap between neighbouring
//! blocks of a row, the gap between rows, the padding inside a frame, the height of pictures, and
//! how much of each framed area its blocks cover.
//! `cargo run -p lore-flow --example flow_measure -- <flow dir> [--page <width_pt> <w:h>] <spec.json>...`
//! With `--page` every spec is drawn on the baseplate for that page instead of as it is.
use lore_flow::NoMath;
use lore_flow::figure::{Catalogue, FlowSpec, PageSpec, render};

fn med(v: &mut Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let dir = a.remove(0);
    let page = if a.first().map(String::as_str) == Some("--page") {
        let w: f64 = a[1].parse().unwrap();
        let (x, y) = a[2].split_once(':').unwrap();
        let p = PageSpec { width_pt: w, ratio: [x.parse().unwrap(), y.parse().unwrap()], arrangement: String::new() };
        a.drain(0..3);
        Some(p)
    } else {
        None
    };
    let cat = Catalogue::load(Some(std::path::Path::new(&dir)));
    let (mut gx, mut gy, mut pad, mut pic, mut cov) = (vec![], vec![], vec![], vec![], vec![]);
    println!("figure\tcol gap/text\trow gap/text\tframe pad/text\tpicture height/text\tarea cover %");
    for f in &a {
        let mut spec = FlowSpec::from_value(serde_json::from_str(&std::fs::read_to_string(f).expect("read")).expect("json")).expect("spec");
        if page.is_some() {
            spec.page = page.clone();
        }
        let Ok((fig, g)) = render(&spec, &cat, None, &NoMath, spec.frame_rect(), 2) else { continue };
        let lay = &fig.layout;
        let fs = cat.theme(&spec.theme).font.size * lay.scale;
        let (mut x, mut y, mut p, mut h, mut c) = (vec![], vec![], vec![], vec![], vec![]);
        for (pi, pb) in lay.panels.iter().enumerate() {
            let ns: Vec<usize> = (0..lay.nodes.len()).filter(|i| lay.nodes[*i].panel == pi).collect();
            if ns.is_empty() {
                continue;
            }
            for &i in &ns {
                let r = lay.nodes[i].rect.union(&lay.nodes[i].reserve);
                // the nearest block to the right on the same row, and the nearest below in the same column
                let right = ns.iter().filter(|j| **j != i).map(|j| lay.nodes[*j].rect.union(&lay.nodes[*j].reserve)).filter(|o| o.x >= r.right() - 0.5 && o.y < r.bottom() && r.y < o.bottom()).map(|o| o.x - r.right()).fold(f64::MAX, f64::min);
                let below = ns.iter().filter(|j| **j != i).map(|j| lay.nodes[*j].rect.union(&lay.nodes[*j].reserve)).filter(|o| o.y >= r.bottom() - 0.5 && o.x < r.right() && r.x < o.right()).map(|o| o.y - r.bottom()).fold(f64::MAX, f64::min);
                if right < f64::MAX {
                    x.push(right / fs);
                }
                if below < f64::MAX {
                    y.push(below / fs);
                }
                if lore_flow::modules::growable(&g.nodes[i]) {
                    h.push(lay.nodes[i].art.h / fs);
                }
            }
            if !pb.bare {
                let u = ns.iter().map(|i| lay.nodes[*i].rect.union(&lay.nodes[*i].reserve)).reduce(|a, b| a.union(&b)).unwrap();
                p.push(((u.x - pb.rect.x).min(pb.rect.right() - u.right()).min(pb.rect.bottom() - u.bottom())) / fs);
                c.push(100.0 * ns.iter().map(|i| lay.nodes[*i].rect.w * lay.nodes[*i].rect.h).sum::<f64>() / (pb.rect.w * pb.rect.h));
            }
        }
        let name = std::path::Path::new(f).parent().and_then(|p| p.parent()).and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        println!("{name}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.0}", med(&mut x.clone()), med(&mut y.clone()), med(&mut p.clone()), med(&mut h.clone()), med(&mut c.clone()));
        gx.extend(x);
        gy.extend(y);
        pad.extend(p);
        pic.extend(h);
        cov.extend(c);
    }
    println!("MEDIAN\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.0}", med(&mut gx), med(&mut gy), med(&mut pad), med(&mut pic), med(&mut cov));
}
