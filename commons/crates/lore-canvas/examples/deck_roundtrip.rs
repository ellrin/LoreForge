//! Import real decks into the canvas model and export them unedited:
//! every slide / notes part must come back byte-identical.
//!
//! ```sh
//! LORE_DECKS="a.pptx:b.pptx" [LORE_STYLE_DIR=styles/x] cargo run --release -p lore-canvas --example deck_roundtrip
//! ```
//! Paths come from the environment; no deck content is printed (only counts).

use std::collections::BTreeMap;

use lore_canvas::{Kind, StyleSystem, pptx_in, pptx_out};
use lore_ooxml::Deck;

fn main() {
    let decks = std::env::var("LORE_DECKS").expect("LORE_DECKS=a.pptx:b.pptx");
    let style = std::env::var("LORE_STYLE_DIR")
        .ok()
        .map(|d| StyleSystem::new(lore_ooxml::style::StylePack::load_dir(std::path::Path::new(&d)).expect("style")));
    for path in decks.split(':') {
        let od = Deck::open(path).expect("open");
        let t = std::time::Instant::now();
        let cd = pptx_in::import(&od, style.as_ref(), None);
        let t_import = t.elapsed();
        // JSON round trip (what deck.json goes through)
        let json = serde_json::to_string(&cd).unwrap();
        let cd2: lore_canvas::Deck = serde_json::from_str(&json).unwrap();
        assert_eq!(cd, cd2, "json round trip");
        let none = |_: &str| None;
        let out = pptx_out::export(&cd2, &od, style.as_ref(), &none).expect("export");
        let bytes = out.to_bytes().unwrap();
        let back = Deck::from_bytes(&bytes).unwrap();
        let mut same = 0;
        let mut diff = Vec::new();
        for p in od.pkg.parts.iter() {
            match back.pkg.get(&p.name) {
                Some(b) if b == p.data.as_slice() => same += 1,
                _ => diff.push(p.name.clone()),
            }
        }
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        let mut runs = 0;
        let mut untagged = 0;
        let mut untagged_looks: BTreeMap<String, usize> = BTreeMap::new();
        for s in &cd.slides {
            s.walk(&mut |o| {
                *kinds.entry(o.kind.name().to_string()).or_default() += 1;
                let mut tbs = Vec::new();
                match &o.kind {
                    Kind::Text(sp) | Kind::Shape(sp) => tbs.extend(sp.text.iter()),
                    Kind::Table(t) => tbs.extend(t.rows.iter().flat_map(|r| r.cells.iter().map(|c| &c.text))),
                    _ => {}
                }
                for tb in tbs {
                    for p in &tb.paras {
                        for r in &p.runs {
                            if r.kind == "text" && !r.text.trim().is_empty() {
                                runs += 1;
                                if r.tag.is_none() {
                                    untagged += 1;
                                    let l = &r.look;
                                    *untagged_looks
                                        .entry(format!("{}/{} {} {}{}", l.font, l.ea_font, l.size, if l.bold { "b " } else { "" }, l.color))
                                        .or_default() += 1;
                                }
                            }
                        }
                    }
                }
            });
        }
        let name = std::path::Path::new(path).file_name().unwrap().to_string_lossy();
        println!(
            "{name}: {} slides, import {:?}, json {} KB, parts identical {same}/{} {:?}",
            cd.slides.len(),
            t_import,
            json.len() / 1024,
            od.pkg.parts.len(),
            diff
        );
        println!("  objects {kinds:?}; text runs {runs}, untagged {untagged}");
        let mut v: Vec<_> = untagged_looks.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, n) in v.iter().take(8) {
            println!("    untagged look {k}: {n}");
        }
    }
}
