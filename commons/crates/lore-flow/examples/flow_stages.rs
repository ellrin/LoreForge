//! A figure built on the plate (`plate`, docs/flow-blocks.md §10), one picture per layer added.
//!
//! `cargo run -p lore-flow --example flow_stages -- <spec.json | figure.dsl> <width_pt> <w:h> <out dir> <flow dir> [name]`
//! writes `<name>-stage1.svg` … `<name>-stage5.svg` (plate with its numbers, + base areas,
//! + blocks in their cells, + lines, + words = the finished figure), `<name>-stages.html` (the five
//! side by side, labelled, for a screenshot) and prints the numbers and the look scores.
//!
//! `… --example flow_stages -- --score <spec.json> <width_pt> <w:h> <out.svg> <flow dir>` draws the
//! finished figure only and prints one line of scores in `flow_hug`'s words (the loop's runner).
//!
//! `… --example flow_stages -- --options <spec.json> <width_pt> <w:h> <out dir> <flow dir> <name>` draws
//! every option `place` offers: `<name>-option<i>.svg` (the plate's outline in thin grey),
//! `<name>-options.html` (the sheet) and `<name>-options.tsv` (one row of numbers per option).
//!
//! A block's hand-given `w=` / `h=` / `size=` is stripped: the plate decides sizes.

use lore_canvas::{Kind, Object};
use lore_flow::figure::{Catalogue, FlowSpec};
use lore_flow::plate::{self, CELL, Format, Measure, Placement};
use lore_flow::{Figure, Graph, Layout, NoMath, Rect, Vars, build, look, modules, route};

struct Drawn {
    p: Placement,
    binding: plate::Binding,
    g: Graph,
    all: Vec<Placement>,
    fig: Figure,
    stripped: Vec<String>,
    f: Format,
    fills: Vec<plate::FrameFill>,
}

fn ratio(s: &str) -> (u32, u32) {
    let (a, b) = s.split_once(':').expect("a ratio w:h");
    (a.trim().parse().expect("w"), b.trim().parse().expect("h"))
}

fn load_spec(path: &str) -> FlowSpec {
    let text = std::fs::read_to_string(path).expect("read the figure");
    if path.ends_with(".json") {
        FlowSpec::from_value(serde_json::from_str(&text).expect("json")).expect("spec")
    } else {
        FlowSpec::from_value(serde_json::json!({ "lore_flow": 1, "dsl": text, "theme": "lore-macaron" })).expect("spec")
    }
}

/// The options `place` offers (or the one `pick` names first), each drawn finished.
fn draw(spec: &FlowSpec, cat: &Catalogue, f: Format, pick: Option<&str>) -> Vec<Drawn> {
    let (mut g, theme, _opt) = spec.prepare(cat, spec.frame_rect()).expect("prepare");
    let mut stripped = Vec::new();
    for n in &mut g.nodes {
        let gone: Vec<String> = ["w", "h", "size", "grow"].iter().filter_map(|k| n.params.remove(*k).map(|v| format!("{k}={v}"))).collect();
        if !gone.is_empty() {
            stripped.push(format!("{}: {}", n.id, gone.join(" ")));
        }
    }
    g.validate(&cat.library.kinds()).expect("valid graph");
    let mut vars = Vars::default();
    vars.add_new(&g.vars_new);
    let m = Measure { theme: &theme, vars: &vars, library: &cat.library, image_size: &modules::no_images };
    let mut all = plate::place(&g, &f, &m).expect("a figure with blocks");
    if let Some(name) = pick {
        let p = plate::arrangement(&g, &f, &m, name).unwrap_or_else(|| panic!("no arrangement {name}"));
        all.retain(|q| q.arrangement != p.arrangement);
        all.insert(0, p);
        all.truncate(plate::OPTIONS);
    }
    all.iter()
        .map(|p| {
            let p = p.clone();
            let lay = plate::to_layout(&g, &f, &p, &m);
            let g2 = plate::grown(&g, &f, &p, &m);
            let fills = plate::fills(&g, &f, &p, &m);
            let binding = plate::binding(&g, &f, &p, &m);
            let routes = route::route(&g2, &lay);
            let built = build::build_with(&g2, &theme, &vars, &cat.library, &NoMath, &lay, &routes, 2, &modules::no_images);
            Drawn { p, binding, g: g2.clone(), all: all.clone(), fig: Figure { layout: lay, routes, built }, stripped: stripped.clone(), f: f.clone(), fills }
        })
        .collect()
}

/// The options sheet: every arrangement offered, finished, captioned as the user would choose.
fn options_html(ds: &[Drawn], out: &str, name: &str) -> (String, u32, u32) {
    let col = 760u32;
    let mut cells = String::new();
    let mut tallest: f64 = 0.0;
    for (i, d) in ds.iter().enumerate() {
        let (w, h) = (d.p.plate.0 as f64 * CELL, d.p.plate.1 as f64 * CELL);
        tallest = tallest.max(h / w);
        let file = format!("{name}-option{}.svg", i + 1);
        std::fs::write(format!("{out}/{file}"), page(d.fig.objects(), w, h, "", &outline(w, h))).expect("write option");
        let fills: Vec<String> = d.fills.iter().filter(|x| x.kind == "panel").map(|x| format!("{} {:.0}%", x.id, 100.0 * x.fill_after())).collect();
        let bends: usize = d.fig.routes.iter().map(|r| r.bends()).sum();
        let look = look::check(d.fig.objects());
        cells.push_str(&format!(
            "<div class=c><div class=l>{}. {}{}</div><div class=n>plate {} x {} cells · smallest text {:.1} pt · cover {:.0}% · frame faults {} · fill {} · bends {bends} · loose ends {}</div><img src=\"{file}\"></div>",
            i + 1,
            esc(&d.p.arrangement),
            if d.p.stated { " (stated)" } else { "" },
            d.p.plate.0,
            d.p.plate.1,
            look::smallest_text(d.fig.objects(), d.fig.layout.bounds.w, d.f.width_pt),
            100.0 * d.p.cover,
            own_faults(&d.g, &d.fig.layout).count(),
            esc(&fills.join(", ")),
            look.loose_ends.len()
        ));
    }
    let cols = ds.len().clamp(1, 4) as u32;
    let html = format!(
        "<!doctype html><meta charset=utf-8><style>body{{margin:0;background:#fff;font:15px Helvetica,Arial,sans-serif;color:#1F2A3A}}.g{{display:grid;grid-template-columns:repeat({cols},{col}px);gap:16px;padding:16px;align-items:start}}.c{{border:1px solid #D8DEE7;padding:8px}}.l{{font-weight:bold}}.n{{margin:4px 0 8px;color:#4A5568}}img{{width:100%;display:block}}</style><div class=g>{cells}</div>"
    );
    let win_w = cols * (col + 16) + 32;
    let win_h = (col as f64 * tallest + 120.0).ceil() as u32 + 40;
    (html, win_w, win_h)
}

/// The look scores in `flow_hug`'s words, one line.
fn scores(d: &Drawn, name: &str) -> String {
    let fig = &d.fig;
    let look = look::check(fig.objects());
    let (uneven, sparse) = look::frames(&fig.layout);
    let small = look::smallest_text(fig.objects(), fig.layout.bounds.w, d.f.width_pt);
    let hug = route::hugging(&fig.layout, &fig.routes);
    let bends: usize = fig.routes.iter().map(|r| r.bends()).sum();
    let used: f64 = fig.layout.nodes.iter().map(|n| n.rect.w * n.rect.h).sum::<f64>() / (fig.layout.bounds.w * fig.layout.bounds.h);
    let out_of_plate = fig.routes.iter().flat_map(|r| r.points.iter()).filter(|p| p.0 < -0.5 || p.1 < -0.5 || p.0 > fig.layout.bounds.right() + 0.5 || p.1 > fig.layout.bounds.bottom() + 0.5).count();
    format!(
        "line over words {}, line through an open shape {}, words cut by a shape or frame {}, loose ends {}, line on line {}, line behind a block {}, line over a module {}, hidden arrowheads {}, uneven frames {}, sparse panels {}, smallest text at page width {:.1} pt; {name}: on a frame {hug}, bends {bends}, blocks cover {:.0}% of the figure; line points off the plate {out_of_plate}",
        look.over_text.len(),
        look.through_open.len(),
        look.words_cut.len(),
        look.loose_ends.len(),
        look.on_line.len(),
        look.behind_block.len(),
        look.over_module.len(),
        look.hidden_heads.len(),
        uneven,
        sparse,
        small,
        100.0 * used
    )
}

/// Is this object (or part) words: a title, a label, a variable, a caption, a line's label, a legend?
fn is_words(name: &str) -> bool {
    let last = name.rsplit('/').next().unwrap_or("");
    matches!(last, "title" | "label" | "var" | "caption") || name.contains("/legend:") || name.contains("/colours:")
}

fn without_words(objs: &[Object]) -> Vec<Object> {
    objs.iter()
        .filter(|o| !is_words(&o.name))
        .cloned()
        .map(|mut o| {
            if let Kind::Group(g) = &mut o.kind {
                g.children = without_words(&g.children);
            }
            o
        })
        .collect()
}

const M: f64 = 6.0;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// An SVG of `objs` on the plate, with `under` drawn below them and `over` above them (both in
/// plate points).
fn page(objs: &[Object], w: f64, h: f64, under: &str, over: &str) -> String {
    let mut objs = objs.to_vec();
    for o in &mut objs {
        o.frame.x += modules::emu(M);
        o.frame.y += modules::emu(M);
    }
    let svg = lore_flow::preview::svg(&objs, w + 2.0 * M, h + 2.0 * M, 1.0);
    let bg = "<rect width=\"100%\" height=\"100%\" fill=\"#FFFFFF\"/>";
    let under = format!("<g transform=\"translate({M},{M})\">{under}</g>");
    let over = format!("<g transform=\"translate({M},{M})\">{over}</g>");
    let svg = svg.replacen(bg, &format!("{bg}{under}"), 1);
    match svg.rfind("</svg>") {
        Some(i) => format!("{}{over}{}", &svg[..i], &svg[i..]),
        None => svg,
    }
}

/// The plate's outline: a thin grey rectangle, so the page is visible round the figure.
fn outline(w: f64, h: f64) -> String {
    format!("<rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"none\" stroke=\"#B8C0CC\" stroke-width=\"0.6\"/>")
}

/// Frame faults counted here, on the layout (the same three kinds as `look::frame_faults`, kept
/// while that is not merged): frames that overlap with neither inside the other, a title touching
/// another title or crossed by a frame not its own, a block outside its panel or a region it is a
/// member of, or inside a region it is not a member of.
#[derive(Default, Debug)]
struct Faults {
    overlaps: Vec<(String, String)>,
    titles: Vec<(String, String)>,
    strays: Vec<(String, String)>,
}

impl Faults {
    fn count(&self) -> usize {
        self.overlaps.len() + self.titles.len() + self.strays.len()
    }
}

fn own_faults(g: &Graph, lay: &Layout) -> Faults {
    let mut out = Faults::default();
    let meet = |a: &Rect, b: &Rect| a.x < b.right() - 0.5 && b.x < a.right() - 0.5 && a.y < b.bottom() - 0.5 && b.y < a.bottom() - 0.5;
    let holds = |a: &Rect, b: &Rect| a.x <= b.x + 0.5 && a.y <= b.y + 0.5 && a.right() >= b.right() - 0.5 && a.bottom() >= b.bottom() - 0.5;
    let mut frames: Vec<(String, Rect, Option<Rect>)> = lay.panels.iter().filter(|p| !p.bare).map(|p| (format!("panel {}", p.id), p.rect, p.title_rect)).collect();
    frames.extend(lay.regions.iter().map(|r| (format!("region {}", r.id), r.rect, r.title_rect)));
    for i in 0..frames.len() {
        for j in i + 1..frames.len() {
            let (a, b) = (&frames[i], &frames[j]);
            if meet(&a.1, &b.1) && !holds(&a.1, &b.1) && !holds(&b.1, &a.1) {
                out.overlaps.push((a.0.clone(), b.0.clone()));
            }
            if let (Some(x), Some(y)) = (a.2, b.2)
                && meet(&x, &y)
            {
                out.titles.push((a.0.clone(), b.0.clone()));
            }
        }
    }
    for (a, _, t) in &frames {
        let Some(t) = t else { continue };
        for (b, r, _) in &frames {
            if a != b && meet(t, r) && !holds(r, t) {
                out.titles.push((a.clone(), b.clone()));
            }
        }
    }
    // every node a region frames, its sub-regions' included
    let members = |id: &str| -> Vec<String> {
        let mut ids = vec![id.to_string()];
        let mut i = 0;
        while i < ids.len() {
            let kids: Vec<String> = g.regions.iter().filter(|r| r.parent == ids[i] && !ids.contains(&r.id)).map(|r| r.id.clone()).collect();
            ids.extend(kids);
            i += 1;
        }
        g.regions.iter().filter(|r| ids.contains(&r.id)).flat_map(|r| r.members.clone()).collect()
    };
    for (ni, n) in lay.nodes.iter().enumerate() {
        let id = &g.nodes[ni].id;
        let room = n.rect.union(&n.reserve);
        if let Some(p) = lay.panels.get(n.panel)
            && !p.bare
            && !holds(&p.rect, &room)
        {
            out.strays.push((id.clone(), format!("panel {}", p.id)));
        }
        for r in &lay.regions {
            let own = members(&r.id).contains(id);
            if (own && !holds(&r.rect, &room)) || (!own && meet(&r.rect, &room)) {
                out.strays.push((id.clone(), format!("region {}", r.id)));
            }
        }
    }
    out
}

fn grid_lines(pw: u32, ph: u32, strong: bool) -> String {
    let (w, h) = (pw as f64 * CELL, ph as f64 * CELL);
    let mut s = String::new();
    let (thin, every) = if strong { ("#C9D3E0", "#93A4BC") } else { ("#EEF1F5", "#D8DEE7") };
    for i in 0..=pw {
        let x = i as f64 * CELL;
        let c = if i % 10 == 0 { every } else { thin };
        s.push_str(&format!("<line x1=\"{x}\" y1=\"0\" x2=\"{x}\" y2=\"{h}\" stroke=\"{c}\" stroke-width=\"{}\"/>", if i % 10 == 0 { 0.5 } else { 0.25 }));
    }
    for j in 0..=ph {
        let y = j as f64 * CELL;
        let c = if j % 10 == 0 { every } else { thin };
        s.push_str(&format!("<line x1=\"0\" y1=\"{y}\" x2=\"{w}\" y2=\"{y}\" stroke=\"{c}\" stroke-width=\"{}\"/>", if j % 10 == 0 { 0.5 } else { 0.25 }));
    }
    s.push_str(&format!("<rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"none\" stroke=\"#5B6B82\" stroke-width=\"1\"/>"));
    s
}

fn fill_line(fl: &plate::FrameFill) -> String {
    let ((fw, fh), (cw, ch)) = fl.after;
    let ((bw, bh), (bcw, bch)) = fl.before;
    let (mb, ma) = fl.margin();
    format!(
        "{} {}: content {cw}x{ch} / frame {fw}x{fh} = {:.0}%, margin {ma} cells (least: {bcw}x{bch} / {bw}x{bh} = {:.0}%, margin {mb})",
        fl.kind,
        fl.id,
        100.0 * fl.fill_after(),
        100.0 * fl.fill_before()
    )
}

fn numbers(d: &Drawn) -> Vec<String> {
    let p = &d.p;
    let f = &d.f;
    let b = &p.budget;
    let k = p.plate.0 / f.ratio.0;
    let least = b.least.0 as u64 * b.least.1 as u64;
    let mut v = vec![
        format!("width {:.0} pt, ratio {}:{}", f.width_pt, f.ratio.0, f.ratio.1),
        format!("multiplier {k}: plate {} x {} cells (one cell {:.2} pt on the page, {CELL} pt natural)", p.plate.0, p.plate.1, f.cell_pt(p.plate.0)),
        format!("budget: blocks {} + margins {} + channels {} = least {} x {} = {least} cells", b.blocks, b.margins, b.channels, b.least.0, b.least.1),
        format!("plate {} cells, empty share {:.0}%", p.plate.0 as u64 * p.plate.1 as u64, 100.0 * p.empty),
        format!("smallest text on the page {:.1} pt (limit {:.0} pt)", p.text_pt, f.min_text_pt),
        format!("arrangement {}", p.arrangement),
    ];
    for w in &p.ways {
        v.push(format!("way: {w}"));
    }
    for fl in &d.fills {
        v.push(fill_line(fl));
    }
    v
}

fn stages(d: &Drawn, out: &str, name: &str) -> Vec<String> {
    let p = &d.p;
    let (w, h) = (p.plate.0 as f64 * CELL, p.plate.1 as f64 * CELL);
    let objs = d.fig.objects();
    let no_words = without_words(objs);
    let is_panel = |o: &Object| o.name.contains("/panel:");
    let is_edge = |o: &Object| o.name.contains("/edge:");
    let areas: Vec<Object> = no_words.iter().filter(|o| is_panel(o)).cloned().collect();
    let blocks: Vec<Object> = no_words.iter().filter(|o| !is_edge(o)).cloned().collect();
    // stage 1: the plate and its numbers
    let units = |l: &str| l.chars().map(|c| if c > '\u{2E80}' { 1.0 } else { 0.56 }).sum::<f64>();
    let longest = numbers(d).iter().map(|l| units(l)).fold(20.0, f64::max);
    let fs = (w / 48.0).min((w - 4.0 * (w / 48.0)) / longest).clamp(3.0, 40.0);
    let mut text = format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#FFFFFF\" fill-opacity=\"0.9\" stroke=\"#5B6B82\" stroke-width=\"0.8\"/>", fs, fs, w - 2.0 * fs, fs * 1.5 * (numbers(d).len() as f64 + 0.8));
    for (i, l) in numbers(d).iter().enumerate() {
        text.push_str(&format!("<text x=\"{}\" y=\"{}\" font-family=\"Helvetica, Arial, sans-serif\" font-size=\"{fs}\" fill=\"#1F2A3A\">{}</text>", fs * 1.6, fs * (2.6 + 1.5 * i as f64), esc(l)));
    }
    let footprints: String = p
        .blocks
        .iter()
        .map(|c| {
            let r = plate::rect_of(*c, CELL);
            format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#5B8DEF\" fill-opacity=\"0.12\" stroke=\"#5B8DEF\" stroke-opacity=\"0.6\" stroke-width=\"0.5\"/>", r.x, r.y, r.w, r.h)
        })
        .collect();
    // the arithmetic of every frame: content box against frame, before and after the spare room
    let mut footprints = footprints;
    let lay = &d.fig.layout;
    for fl in &d.fills {
        let r = match fl.kind.as_str() {
            "panel" => lay.panels.iter().find(|p| p.id == fl.id).map(|p| p.rect),
            _ => lay.regions.iter().find(|r| r.id == fl.id).map(|r| r.rect),
        };
        let Some(r) = r else { continue };
        // in the title's row (no words are drawn at this stage), from the frame's left
        let line = fill_line(fl);
        let fsz = (11.0f64).min(0.95 * r.w / (line.chars().count() as f64 * 0.55));
        let tw = line.chars().count() as f64 * fsz * 0.52;
        let (x, y) = (r.x + 3.0, r.y + 1.0);
        footprints.push_str(&format!("<rect x=\"{x}\" y=\"{y}\" width=\"{}\" height=\"{}\" fill=\"#FFF7D6\" stroke=\"#C9A227\" stroke-width=\"0.5\"/><text x=\"{}\" y=\"{}\" font-family=\"Helvetica, Arial, sans-serif\" font-size=\"{fsz}\" fill=\"#5A4500\">{}</text>", tw + fsz * 0.6, fsz * 1.5, x + fsz * 0.3, y + fsz * 1.1, esc(&line)));
    }
    let grid = grid_lines(p.plate.0, p.plate.1, true);
    let faint = grid_lines(p.plate.0, p.plate.1, false);
    let pages = [
        page(&[], w, h, &grid, &text),
        page(&areas, w, h, &faint, ""),
        page(&blocks, w, h, &faint, &footprints),
        page(&no_words, w, h, &faint, ""),
        page(objs, w, h, "", ""),
    ];
    let mut files = Vec::new();
    for (i, s) in pages.iter().enumerate() {
        let f = format!("{out}/{name}-stage{}.svg", i + 1);
        std::fs::write(&f, s).expect("write svg");
        files.push(f);
    }
    files
}

fn html(d: &Drawn, name: &str, files: &[String], score: &str, col_px: u32) -> String {
    let labels = ["1. plate (format, multiplier, budget)", "2. + base areas", "3. + blocks in their cells", "4. + lines", "5. + words = the figure"];
    let mut cells = String::new();
    for (f, l) in files.iter().zip(labels) {
        let file = std::path::Path::new(f).file_name().unwrap().to_string_lossy();
        cells.push_str(&format!("<div class=c><div class=l>{l}</div><img src=\"{file}\"></div>"));
    }
    let mut notes = numbers(d).iter().map(|l| format!("<div>{}</div>", esc(l))).collect::<String>();
    notes.push_str(&format!("<div style=\"margin-top:8px\"><b>scores</b> {}</div>", esc(score)));
    if !d.stripped.is_empty() {
        notes.push_str(&format!("<div style=\"margin-top:8px\"><b>stripped</b> {}</div>", esc(&d.stripped.join("; "))));
    }
    let others: Vec<String> = d.all.iter().skip(1).take(4).map(|p| format!("{} (plate {}x{}, text {:.1} pt, empty {:.0}%)", p.arrangement, p.plate.0, p.plate.1, p.text_pt, 100.0 * p.empty)).collect();
    if !others.is_empty() {
        notes.push_str(&format!("<div style=\"margin-top:8px\"><b>next arrangements</b> {}</div>", esc(&others.join("; "))));
    }
    cells.push_str(&format!("<div class=c><div class=l>{}</div><div class=n>{notes}</div></div>", esc(name)));
    format!(
        "<!doctype html><meta charset=utf-8><style>body{{margin:0;background:#fff;font:14px Helvetica,Arial,sans-serif;color:#1F2A3A}}.g{{display:grid;grid-template-columns:repeat(3,{col_px}px);gap:16px;padding:16px}}.c{{border:1px solid #D8DEE7;padding:8px}}.l{{font-weight:bold;margin-bottom:6px}}img{{width:100%;display:block}}.n div{{margin:3px 0}}</style><div class=g>{cells}</div>"
    )
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().map(String::as_str) == Some("--score") {
        let [_, spec, width, r, out, dir, rest @ ..] = &a[..] else { panic!("--score <spec.json> <width_pt> <w:h> <out.svg> <flow dir> [arrangement]") };
        let cat = Catalogue::load(Some(std::path::Path::new(dir)));
        let f = Format::new(width.parse().expect("width"), ratio(r));
        let ds = draw(&load_spec(spec), &cat, f, rest.first().map(String::as_str).filter(|s| !s.is_empty()));
        let d = &ds[0];
        let (w, h) = (d.p.plate.0 as f64 * CELL, d.p.plate.1 as f64 * CELL);
        std::fs::write(out, page(d.fig.objects(), w, h, "", "")).expect("write");
        for w in &d.fig.built.warnings {
            eprintln!("warning: {w}");
        }
        let fills: Vec<String> = d.fills.iter().map(|x| format!("{} {:.0}%", x.id, 100.0 * x.fill_after())).collect();
        let faults = look::frame_faults(&d.g, &d.fig.layout).count();
        println!("{}; arrangement {}; plate {}x{}; fill {}; page cover {:.0}%; frame faults {faults}", scores(d, spec), d.p.arrangement, d.p.plate.0, d.p.plate.1, fills.join(" "), 100.0 * d.p.cover);
        return;
    }
    if a.first().map(String::as_str) == Some("--options") {
        let [_, spec, width, r, out, dir, name, ..] = &a[..] else { panic!("--options <spec.json> <width_pt> <w:h> <out dir> <flow dir> <name>") };
        let cat = Catalogue::load(Some(std::path::Path::new(dir)));
        let f = Format::new(width.parse().expect("width"), ratio(r));
        let ds = draw(&load_spec(spec), &cat, f, None);
        std::fs::create_dir_all(out).expect("out dir");
        let (opt, ow, oh) = options_html(&ds, out, name);
        std::fs::write(format!("{out}/{name}-options.html"), opt).expect("write options");
        let mut tsv = String::from("option\tarrangement\tinner\tstated\tplate\ttext_pt\tcover_pct\tbends\tover_words\twords_cut\tloose_ends\tuneven_frames\tsparse_panels\tframe_faults\n");
        for (i, d) in ds.iter().enumerate() {
            let lk = look::check(d.fig.objects());
            let (uneven, sparse) = look::frames(&d.fig.layout);
            let bends: usize = d.fig.routes.iter().map(|r| r.bends()).sum();
            let text = look::smallest_text(d.fig.objects(), d.fig.layout.bounds.w, d.f.width_pt);
            let faults = look::frame_faults(&d.g, &d.fig.layout).count();
            let own = own_faults(&d.g, &d.fig.layout);
            tsv.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}x{}\t{text:.1}\t{:.0}\t{bends}\t{}\t{}\t{}\t{uneven}\t{sparse}\t{faults}\n",
                i + 1,
                d.p.arrangement,
                d.p.inner.join(" "),
                d.p.stated,
                d.p.plate.0,
                d.p.plate.1,
                100.0 * d.p.cover,
                lk.over_text.len(),
                lk.words_cut.len(),
                lk.loose_ends.len()
            ));
            let b = &d.binding;
            println!("  binding: {} binds, need {}x{}, plate {}x{}, spare {}, text {:.1} pt, if shaped {:.1} pt; areas {}", if b.tall { "height" } else { "width" }, b.need.0, b.need.1, b.plate.0, b.plate.1, b.spare, b.text_pt, b.text_if_shaped_pt, b.areas.iter().map(|x| format!("{} {}x{} blocks, {}x{} cells{}", x.id, x.cols, x.rows, x.w, x.h, if x.binds { " (binds)" } else { "" })).collect::<Vec<_>>().join("; "));
            println!("option {}: {}{}; plate {}x{}; text {text:.1} pt (plate {:.1} pt); cover {:.0}%; bends {bends}; loose ends {}; frame faults {faults} (own {}: {:?})", i + 1, d.p.arrangement, if d.p.stated { " (stated)" } else { "" }, d.p.plate.0, d.p.plate.1, d.p.text_pt, 100.0 * d.p.cover, lk.loose_ends.len(), own.count(), own);
        }
        std::fs::write(format!("{out}/{name}-options.tsv"), tsv).expect("write tsv");
        println!("options-window: {ow},{oh}");
        return;
    }
    let [spec, width, r, out, dir, rest @ ..] = &a[..] else { panic!("<spec.json | figure.dsl> <width_pt> <w:h> <out dir> <flow dir> [name] [arrangement]") };
    let cat = Catalogue::load(Some(std::path::Path::new(dir)));
    let f = Format::new(width.parse().expect("width"), ratio(r));
    let ds = draw(&load_spec(spec), &cat, f, rest.get(1).map(String::as_str).filter(|s| !s.is_empty()));
    let d = &ds[0];
    let name = rest.first().cloned().unwrap_or_else(|| "figure".into());
    std::fs::create_dir_all(out).expect("out dir");
    let files = stages(d, out, &name);
    let score = scores(d, &name);
    let ar = d.p.plate.1 as f64 / d.p.plate.0 as f64;
    let col = 800u32;
    std::fs::write(format!("{out}/{name}-stages.html"), html(d, &name, &files, &score, col)).expect("write html");
    let (opt, ow, oh) = options_html(&ds, out, &name);
    std::fs::write(format!("{out}/{name}-options.html"), opt).expect("write options");
    for l in numbers(d) {
        println!("{l}");
    }
    println!("stripped: {}", if d.stripped.is_empty() { "nothing".to_string() } else { d.stripped.join("; ") });
    for o in ds.iter().skip(1) {
        let fills: Vec<String> = o.fills.iter().map(|x| format!("{} {:.0}%", x.id, 100.0 * x.fill_after())).collect();
        println!("option: {} plate {}x{} text {:.1} pt empty {:.0}% fill {}", o.p.arrangement, o.p.plate.0, o.p.plate.1, o.p.text_pt, 100.0 * o.p.empty, fills.join(" "));
        println!("  scores: {}", scores(o, &o.p.arrangement));
    }
    println!("scores: {score}");
    for w in &d.fig.built.warnings {
        println!("warning: {w}");
    }
    // the sizes of the screenshot windows for the html pages
    println!("window: {},{}", 3 * col + 64, ((col as f64 * ar + 40.0) * 2.0 + 80.0).ceil() as u32 + 40);
    println!("options-window: {ow},{oh}");
}
