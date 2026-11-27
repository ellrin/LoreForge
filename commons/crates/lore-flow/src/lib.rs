//! lore-flow — flow charts and architecture figures from a data-flow graph.
//!
//! Three layers:
//!
//! 1. **L1, the data flow** ([`spec::Graph`]): nodes (a module kind from a
//!    closed list, a role, a label), edges (a kind: data, supervision,
//!    feedback …), panels, variables by notation id. No coordinates, no
//!    colours. Written by hand in a short text form ([`dsl`]) or proposed by
//!    an LLM ([`llm`]).
//! 2. **L2, layout and lines** ([`layout`], [`route`]): the template arranges
//!    the panels, nodes are layered inside each panel, lines are routed at
//!    right angles around the modules.
//! 3. **L3, objects** ([`modules`], [`build`]): every module becomes
//!    `lore_canvas` objects, every line a connector attached to its modules;
//!    the batch from [`build::insert_ops`] is one undo step.
//!
//! ```
//! use lore_flow::*;
//! let g = dsl::parse("fig demo\nx: image \"$x$\"\nf: box \"Model\"\ny: label \"$\\hat y$\"\nx -> f -> y\n").unwrap();
//! let fig = draw(&g, &Drawing::default()).unwrap();
//! assert!(fig.objects().iter().any(|o| o.name == "flow:demo/f"));
//! ```

pub mod art;
#[cfg(feature = "web")]
pub mod assist;
pub mod build;
pub mod chart;
#[cfg(feature = "web")]
pub mod discuss;
pub mod dsl;
pub mod figure;
pub mod knowledge;
pub mod finish;
pub mod layout;
pub mod look;
#[cfg(feature = "llm")]
pub mod llm;
pub mod modules;
pub mod notation;
pub mod lines;
pub mod outline;
pub mod settle;
pub mod pack;
pub mod plate;
pub mod pptx;
pub mod preview;
pub mod route;
pub mod sheet;
pub mod spec;
pub mod text;
pub mod theme;
pub mod timeline;
#[cfg(feature = "web")]
pub mod web;

pub use build::{Built, insert_ops, retarget_for_pptx};
pub use layout::{Layout, LayoutOptions, PanelTemplate, Rect};
pub use modules::{Library, ModuleTemplate, SaveOptions, template_from_objects};
pub use route::Route;
pub use spec::{Edge, EdgeKind, Graph, Node, Panel, SpecError, VarNew};
pub use text::{MathSource, NoMath, PandocMath, TexMath, Vars};
pub use theme::Theme;

/// Everything besides the graph that decides how a figure is drawn.
pub struct Drawing<'a> {
    pub theme: Theme,
    pub library: Library,
    /// The notation registry in force (global + document), for `@id`
    /// references and registered macros.
    pub notation: Option<&'a lore_notation::Notation>,
    pub math: &'a dyn MathSource,
    pub options: LayoutOptions,
    /// Id of the first new object (the slide's largest id + 1).
    pub first_id: u32,
    /// Bytes of the picture files image nodes name (`src=`), so pictures keep
    /// their proportions. `None` = unknown (boxes are drawn as asked).
    pub pictures: Option<&'a dyn Fn(&str) -> Option<Vec<u8>>>,
}

impl Default for Drawing<'_> {
    fn default() -> Self {
        Drawing { theme: Theme::classic_bw(), library: Library::default(), notation: None, math: &NoMath, options: LayoutOptions::default(), first_id: 2, pictures: None }
    }
}

/// Layer bars colour a kind of layer the same in every module of a figure:
/// the figure's layer kinds, in order, go to each layer-bars node.
fn with_layer_classes(g: &Graph) -> Graph {
    let mut classes: Vec<String> = Vec::new();
    for n in g.nodes.iter().filter(|n| n.module == "layer-bars") {
        for l in n.list("layers") {
            let c = modules::layer_class(&l);
            // an ellipsis (…) stands for layers left out, not a kind of layer
            if c.chars().any(char::is_alphanumeric) && !classes.contains(&c) {
                classes.push(c);
            }
        }
    }
    let mut g = g.clone();
    if classes.len() > 1 {
        for n in g.nodes.iter_mut().filter(|n| n.module == "layer-bars") {
            n.params.insert("_classes".into(), serde_json::Value::Array(classes.iter().cloned().map(serde_json::Value::String).collect()));
        }
    }
    g
}

/// A drawn figure with the layout and routes it was built from.
#[derive(Debug, Clone)]
pub struct Figure {
    pub layout: Layout,
    pub routes: Vec<Route>,
    pub built: Built,
}

impl Figure {
    pub fn objects(&self) -> &[lore_canvas::Object] {
        &self.built.objects
    }
}

/// How much larger a panel's growable blocks could be: the panel's inside against the box of
/// its blocks (with their words), the smaller of the two directions; 1 = full.
pub fn panel_room(g: &Graph, lay: &layout::Layout) -> Vec<f64> {
    lay.panels
        .iter()
        .enumerate()
        .map(|(pi, p)| {
            let rs: Vec<Rect> = lay.nodes.iter().filter(|n| n.panel == pi).map(|n| n.reserve.union(&n.rect)).collect();
            if p.bare || rs.is_empty() || !g.nodes.iter().zip(&lay.nodes).any(|(n, b)| b.panel == pi && modules::growable(n)) {
                return 1.0;
            }
            let (x0, y0) = rs.iter().fold((f64::MAX, f64::MAX), |m, r| (m.0.min(r.x), m.1.min(r.y)));
            let (x1, y1) = rs.iter().fold((f64::MIN, f64::MIN), |m, r| (m.0.max(r.right()), m.1.max(r.bottom())));
            let pad = 14.0 * lay.scale;
            let top = p.title_rect.map(|t| (t.bottom() - p.rect.y).max(0.0)).unwrap_or(0.0);
            let (iw, ih) = (p.rect.w - 2.0 * pad, p.rect.h - 2.0 * pad - top);
            ((iw / (x1 - x0).max(1.0)).max(ih / (y1 - y0).max(1.0))).max(1.0)
        })
        .collect()
}

/// Grow the pictures, charts and drawings of panels their blocks leave mostly empty (room
/// over 1.15, at most 1.6 times), as long as the figure as a whole is not drawn smaller for
/// it; `relayout` lays out a graph again. `None` = nothing to grow.
pub fn fill_panels(g: &Graph, lay: &layout::Layout, relayout: impl Fn(&Graph) -> layout::Layout) -> Option<(Graph, layout::Layout)> {
    let room = panel_room(g, lay);
    if room.iter().all(|r| *r < 1.15) {
        return None;
    }
    let mut share = 1.0;
    for _ in 0..3 {
        let mut gg = g.clone();
        let mut any = false;
        for (n, b) in gg.nodes.iter_mut().zip(&lay.nodes) {
            let r = room[b.panel];
            if r >= 1.15 && modules::growable(n) {
                let f = 1.0 + (r.min(1.6) - 1.0) * share;
                n.params.insert("grow".into(), serde_json::json!((f * 100.0).round() / 100.0));
                any = true;
            }
        }
        if !any {
            return None;
        }
        let l2 = relayout(&gg);
        if l2.scale >= lay.scale * 0.99 {
            return Some((gg, l2));
        }
        share /= 2.0;
    }
    None
}

/// A graph placed on the baseplate of a page and fitted into the drawing's frame: the graph as
/// it is drawn (hand-given sizes dropped, the plate's enlargements set) and its layout. The
/// arrangement is the one the options name, else the first the plate offers. `None` for a graph
/// the plate cannot place (no blocks).
fn on_page(g: &Graph, d: &Drawing, f: &plate::Format, vars: &Vars, size: &dyn Fn(&str) -> Option<(u32, u32)>) -> Option<(Graph, layout::Layout)> {
    let mut g = g.clone();
    for n in &mut g.nodes {
        for k in ["w", "h", "size", "grow"] {
            n.params.remove(k);
        }
    }
    let m = plate::Measure { theme: &d.theme, vars, library: &d.library, image_size: size };
    let name = d.options.arrangement.trim();
    let picked = if name.is_empty() { None } else { plate::arrangement(&g, f, &m, name) };
    let p = match picked {
        Some(p) => p,
        None => plate::place(&g, f, &m)?.into_iter().next()?,
    };
    let lay = plate::to_layout(&g, f, &p, &m);
    let g = plate::grown(&g, f, &p, &m);
    let mut lay = lay.fitted(&g, &d.options);
    layout::apply_place(&mut lay, &g, &d.options, &d.theme);
    Some((g, lay))
}

/// Graph → layout → routes → objects.
pub fn draw(g: &Graph, d: &Drawing) -> Result<Figure, SpecError> {
    g.validate(&d.library.kinds())?;
    let g = &with_layer_classes(g);
    let mut vars = d.notation.map(Vars::from_notation).unwrap_or_default();
    vars.add_new(&g.vars_new);
    let size = |src: &str| d.pictures.and_then(|f| f(src)).and_then(|b| pptx::image_size(&b));
    let grown;
    let mut layout;
    let g = if let Some((gg, lay)) = d.options.page.as_ref().and_then(|f| on_page(g, d, f, &vars, &size)) {
        // made for a page: measured and placed on the baseplate, which also sizes the pictures
        grown = gg;
        layout = lay;
        &grown
    } else {
        layout = layout::layout_with(g, &d.theme, &vars, &d.library, &d.options, &size);
        // a panel its blocks leave mostly empty: its pictures, charts and drawings are drawn larger
        match fill_panels(g, &layout, |gg| layout::layout_with(gg, &d.theme, &vars, &d.library, &d.options, &size)) {
            Some((gg, lay)) => {
                grown = gg;
                layout = lay;
                &grown
            }
            None => g,
        }
    };
    let routes = route::route(g, &layout);
    // a line may run outside the blocks' box (round a block that was moved to the edge): the
    // figure's bounds hold every line, so no preview or picture cuts one off
    let pad = 4.0 * layout.scale;
    let b = layout.bounds;
    let (mut x0, mut y0, mut x1, mut y1) = (b.x, b.y, b.right(), b.bottom());
    for p in routes.iter().flat_map(|r| r.points.iter()) {
        x0 = x0.min(p.0 - pad);
        y0 = y0.min(p.1 - pad);
        x1 = x1.max(p.0 + pad);
        y1 = y1.max(p.1 + pad);
    }
    layout.bounds = Rect::new(x0, y0, x1 - x0, y1 - y0);
    let built = build::build_with(g, &d.theme, &vars, &d.library, d.math, &layout, &routes, d.first_id, &size);
    Ok(Figure { layout, routes, built })
}
