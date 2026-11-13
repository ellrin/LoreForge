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
#[cfg(feature = "web")]
pub mod discuss;
pub mod dsl;
pub mod figure;
pub mod finish;
pub mod layout;
#[cfg(feature = "llm")]
pub mod llm;
pub mod modules;
pub mod notation;
pub mod pptx;
pub mod preview;
pub mod route;
pub mod sheet;
pub mod spec;
pub mod text;
pub mod theme;
#[cfg(feature = "web")]
pub mod web;

pub use build::{Built, insert_ops, retarget_for_pptx};
pub use layout::{Layout, LayoutOptions, PanelTemplate, Rect};
pub use modules::{Library, ModuleTemplate, SaveOptions, template_from_objects};
pub use route::Route;
pub use spec::{Edge, EdgeKind, Graph, Node, Panel, SpecError, VarNew};
pub use text::{MathSource, NoMath, PandocMath, Vars};
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

/// Graph → layout → routes → objects.
pub fn draw(g: &Graph, d: &Drawing) -> Result<Figure, SpecError> {
    g.validate(&d.library.kinds())?;
    let g = &with_layer_classes(g);
    let mut vars = d.notation.map(Vars::from_notation).unwrap_or_default();
    vars.add_new(&g.vars_new);
    let size = |src: &str| d.pictures.and_then(|f| f(src)).and_then(|b| pptx::image_size(&b));
    let layout = layout::layout_with(g, &d.theme, &vars, &d.library, &d.options, &size);
    let routes = route::route(g, &layout);
    let built = build::build_with(g, &d.theme, &vars, &d.library, d.math, &layout, &routes, d.first_id, &size);
    Ok(Figure { layout, routes, built })
}
