//! Write synthetic PDFs (never a real paper) for demos and UI checks.
//!
//! cargo run -p lr-core --example synth_pdf -- demo out.pdf      # the demo paper (3 pages)
//! cargo run -p lr-core --example synth_pdf -- long out.pdf      # ≈8,000 words (noun-list scenario)
//! cargo run -p lr-core --example synth_pdf -- numbered 5 out.pdf # "Page 1" … "Page 5"
//! cargo run -p lr-core --example synth_pdf -- notes out.pdf     # demo, page 2 turned 90°, notes written in

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (bytes, out) = match a.first().map(String::as_str) {
        Some("demo") => (lr_core::synth::demo_paper(), a.get(1)),
        Some("long") => (lr_core::synth::long_paper(7), a.get(1)),
        Some("numbered") => (lr_core::synth::numbered_pdf(a.get(1).and_then(|n| n.parse().ok()).unwrap_or(3), "Extra"), a.get(2)),
        Some("notes") => (notes(), a.get(1)),
        _ => {
            eprintln!("usage: synth_pdf demo <out.pdf> | long <out.pdf> | numbered <n> <out.pdf> | notes <out.pdf>");
            std::process::exit(2);
        }
    };
    let out = out.expect("output path");
    std::fs::write(out, bytes).expect("write");
    eprintln!("wrote {out}");
}

/// The demo paper with page 2 rotated by 90° and a few highlights / notes written in as
/// PDF annotations (checks placement in Preview / PDFKit).
fn notes() -> Vec<u8> {
    use lr_core::annotate::{PageGeom, write};
    let mut doc = lopdf::Document::load_mem(&lr_core::synth::demo_paper()).expect("demo");
    let p2 = doc.get_pages()[&2];
    doc.get_dictionary_mut(p2).expect("page").set("Rotate", 90);
    let mut src = vec![];
    doc.save_to(&mut src).expect("save");
    // A text line of the synthetic paper (11 pt at baseline `y`), `x0..x1` in points,
    // as display fractions of a page turned by `rotate`.
    let line = |rotate: i64, y: f64, x0: f64, x1: f64| PageGeom { bx: [0.0, 0.0, 612.0, 792.0], rotate }.rect_of(&[(x0, y - 2.5), (x1, y - 2.5), (x0, y + 8.5), (x1, y + 8.5)]);
    let ann = |id: &str, page: u32, kind: &str, color: &str, rects: Vec<[f64; 4]>, note: &str| -> lr_core::papers::Annotation {
        serde_json::from_value(serde_json::json!({ "id": id, "page": page, "kind": kind, "color": color, "rects": rects, "note": note, "quote": { "exact": "x" } })).expect("annotation")
    };
    let list = vec![
        // page 1: "We compare the Dice coefficient …" (baseline 659) and the next line
        ann("a1", 1, "highlight", "yellow", vec![line(0, 659.0, 72.0, 440.0), line(0, 643.5, 72.0, 300.0)], ""),
        ann("a2", 1, "note", "green", vec![line(0, 547.0, 72.0, 260.0)], "要再查：Dice 與 Jaccard"),
        // page 2 (rotated): "The network is a U-Net …" (baseline 724.5)
        ann("a3", 2, "highlight", "blue", vec![line(90, 724.5, 72.0, 400.0)], "螢光的註記"),
        ann("a4", 2, "note", "pink", vec![line(90, 597.0, 72.0, 380.0)], "rotated note"),
    ];
    write(&src, &list, &"ab".repeat(32)).expect("write")
}
