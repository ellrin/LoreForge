//! A sheet of icons as slide shapes, to look at: what a word finds, drawn through lore-icons and
//! the canvas into one SVG and one .pptx.
//! `cargo run -p lore-flow --example icons_sheet -- <out dir> <word or set:name> …`
use lore_canvas::model::Frame;

fn main() {
    let mut a = std::env::args().skip(1);
    let out = a.next().expect("an out folder");
    let (cell, cols, pt) = (96.0, 8usize, 12700.0);
    let (mut objs, mut id, mut n) = (Vec::new(), 1u32, 0usize);
    for w in a {
        let ids: Vec<String> = if w.contains(':') { vec![w.clone()] } else { lore_icons::search(&w, None, 2).into_iter().map(|h| h.id).collect() };
        if ids.is_empty() {
            eprintln!("{w}: nothing found");
        }
        for i in ids {
            let Some(ic) = lore_icons::icon(&i) else { eprintln!("{i}: no such icon"); continue };
            let (x, y) = (20.0 + (n % cols) as f64 * cell, 20.0 + (n / cols) as f64 * cell);
            let f = Frame::new((x * pt) as i64, (y * pt) as i64, (64.0 * pt) as i64, (64.0 * pt) as i64);
            let o = lore_icons::objects(&ic, f, "1F4E79", id, &i);
            id += o.len() as u32;
            objs.extend(o);
            println!("{i}");
            n += 1;
        }
    }
    let (w, h) = (20.0 + cols as f64 * cell, 30.0 + n.div_ceil(cols) as f64 * cell);
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(format!("{out}/icons.pptx"), lore_flow::pptx::single_slide(&objs, w, h).unwrap()).unwrap();
    std::fs::write(format!("{out}/icons.svg"), lore_flow::preview::svg(&objs, w, h, 2.0)).unwrap();
}
