//! Debug: roles of lines matching a pattern. `debug_lines <pdf> <regex>`
use ym_docsrc::pdf::{detect::Doc, layout::parse_bbox_layout, poppler::Poppler};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let p = Poppler::discover();
    let pages = parse_bbox_layout(&p.bbox_layout(std::path::Path::new(&a[1])).unwrap()).unwrap();
    let re = regex::Regex::new(&a[2]).unwrap();
    let inks = pages.iter().map(|_| None).collect();
    let mut d = Doc::new(&pages, inks);
    let _ = d.captions();
    let hs = d.headings();
    println!("cols {:?}", d.cols);
    for h in &hs {
        println!(
            "HEAD p{} l{} {:?} {}",
            h.page + 1,
            h.line,
            h.number,
            h.title
        );
    }
    if std::env::var_os("ORDER").is_some() {
        for (pi, o) in d.order.iter().enumerate() {
            for &li in o {
                let l = &d.pages[pi].lines[li];
                println!(
                    "ORD p{} l{} b{} y{:.0} x{:.0} {:?} {}",
                    pi + 1,
                    li,
                    l.block,
                    l.bbox.y0,
                    l.bbox.x0,
                    d.roles[pi][li],
                    l.text().chars().take(50).collect::<String>()
                );
            }
        }
    }
    for (pi, pg) in d.pages.iter().enumerate() {
        for (li, l) in pg.lines.iter().enumerate() {
            if re.is_match(&l.text()) {
                let h = hs.iter().find(|h| h.page == pi && h.line == li);
                println!(
                    "p{} l{} b{} {:?} h={:.1} w={:.0} gap={:.1} wordy={:.2} head={:?} | {}",
                    pi + 1,
                    li,
                    l.block,
                    d.roles[pi][li],
                    l.height(),
                    l.bbox.width(),
                    l.max_gap(),
                    l.wordy_ratio(),
                    h.map(|h| &h.title),
                    l.text()
                );
            }
        }
    }
}
