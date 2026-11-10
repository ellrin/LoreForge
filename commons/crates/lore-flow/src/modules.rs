//! The module library: every node kind becomes canvas objects.
//!
//! * Generators (Rust): geometry that depends on parameters — box, op, loss,
//!   label, image, image-stack, feature-stack, vector-bar, matrix, layer-bars,
//!   cuboid-seq, unet.
//! * Templates (JSON, [`ModuleTemplate`]): fixed drawings an author saved,
//!   with `role:*` / `theme:*` colours and a `{{label}}` run.
//!
//! A module is drawn at a scale `k` (1 = natural size, in pt) into a frame
//! whose origin is (0, 0). A module of one shape is inserted as that shape;
//! anything else becomes a group whose first child, `port`, is an invisible
//! rectangle over the whole frame (PowerPoint glues connectors to shapes, not
//! to groups). Connection sites are the mid-points of the frame's sides, as
//! lore-canvas and PowerPoint compute them: in a left-to-right panel the
//! frame has equal room above and below the body so the side sites are on
//! the body's centre line; in a top-down panel the frame is tight.

use std::collections::BTreeMap;

use lore_canvas::{ConnObj, Emu, Frame, Geom, GroupObj, Kind, Object, Paint, SpObj, Stroke};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::spec::Node;
use crate::text::{self, MathSource, Type, Vars};
use crate::theme::{Theme, shade};

pub const EMU_PT: f64 = 12_700.0;

pub fn emu(pt: f64) -> Emu {
    (pt * EMU_PT).round() as Emu
}

pub fn frame(x: f64, y: f64, w: f64, h: f64) -> Frame {
    Frame::new(emu(x), emu(y), emu(w).max(0), emu(h).max(0))
}

/// Where lines attach on a saved drawing: a rect in the drawing's own
/// coordinates (EMU). Parts outside it (a shadow, a side note) do not move
/// the connection sites.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PortRect {
    pub x: Emu,
    pub y: Emu,
    pub w: Emu,
    pub h: Emu,
}

/// A saved drawing: one art style of a module kind
/// (`modules/<kind>/<style>.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleTemplate {
    pub kind: String,
    /// Style name; a file `modules/<kind>/<style>.json` is named after it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub style: String,
    /// One line for the LLM's module list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Natural size in pt (of the port rect when there is one).
    pub w: f64,
    pub h: f64,
    /// The rect lines attach to; the whole drawing when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<PortRect>,
    /// Canvas objects, frames in EMU relative to the drawing's top-left corner.
    /// Colours may be `role:fill`, `role:line`, `role:text`, `theme:ink`,
    /// `theme:muted`, `theme:paper`, `theme:image`, `theme:tensor`; a run whose
    /// text is `{{label}}` receives the node's label.
    pub objects: Value,
}

/// The module library: kind → style → drawing.
#[derive(Debug, Clone, Default)]
pub struct Library {
    pub templates: BTreeMap<String, BTreeMap<String, ModuleTemplate>>,
}

pub const DEFAULT_STYLE: &str = "default";

impl Library {
    pub fn add(&mut self, mut t: ModuleTemplate) -> Result<(), String> {
        if !crate::spec::valid_id(&t.kind) {
            return Err(format!("module: bad kind {:?}", t.kind));
        }
        if t.style.is_empty() {
            t.style = DEFAULT_STYLE.into();
        }
        if !crate::spec::valid_id(&t.style) {
            return Err(format!("module {}: bad style name {:?}", t.kind, t.style));
        }
        if let Some(p) = &t.port {
            t.w = p.w as f64 / EMU_PT;
            t.h = p.h as f64 / EMU_PT;
        }
        if t.w <= 0.0 || t.h <= 0.0 {
            return Err(format!("module {}/{}: w and h must be positive", t.kind, t.style));
        }
        if serde_json::from_value::<Vec<Object>>(placeholder_colours(&t.objects)).is_err() {
            return Err(format!("module {}/{}: objects are not canvas objects", t.kind, t.style));
        }
        self.templates.entry(t.kind.clone()).or_default().insert(t.style.clone(), t);
        Ok(())
    }
    pub fn add_json(&mut self, text: &str) -> Result<(), String> {
        self.add(serde_json::from_str(text).map_err(|e| format!("module: {e}"))?)
    }
    /// Load a library folder: `<kind>/<style>.json` (the style defaults to the
    /// file name) and, directly in the folder, `<kind>.json` (style `default`).
    /// A missing folder is an empty library.
    pub fn load_dir(dir: &std::path::Path) -> Result<Library, String> {
        let mut lib = Library::default();
        let Ok(rd) = std::fs::read_dir(dir) else { return Ok(lib) };
        let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        let load = |f: &std::path::Path, kind: Option<&str>, lib: &mut Library| -> Result<(), String> {
            let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
            let mut t: ModuleTemplate = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
            if let Some(k) = kind {
                if t.kind != k {
                    return Err(format!("{}: kind {:?} does not match its folder {k:?}", f.display(), t.kind));
                }
                if t.style.is_empty() {
                    t.style = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                }
            }
            lib.add(t).map_err(|e| format!("{}: {e}", f.display()))
        };
        for e in entries {
            if e.is_dir() {
                let kind = e.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let mut files: Vec<_> = std::fs::read_dir(&e).map_err(|x| format!("{}: {x}", e.display()))?.filter_map(|x| x.ok()).map(|x| x.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
                files.sort();
                for f in files {
                    load(&f, Some(&kind), &mut lib)?;
                }
            } else if e.extension().is_some_and(|x| x == "json") {
                load(&e, None, &mut lib)?;
            }
        }
        Ok(lib)
    }
    /// Write a drawing to `<dir>/<kind>/<style>.json`. An existing file is
    /// never overwritten: the style gets a free name (`flat-2`), which is returned.
    pub fn save(dir: &std::path::Path, t: &ModuleTemplate) -> Result<std::path::PathBuf, String> {
        let mut t = t.clone();
        if t.style.is_empty() {
            t.style = DEFAULT_STYLE.into();
        }
        let folder = dir.join(&t.kind);
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let base = t.style.clone();
        let mut n = 1;
        let path = loop {
            let p = folder.join(format!("{}.json", t.style));
            if !p.exists() {
                break p;
            }
            n += 1;
            t.style = format!("{base}-{n}");
        };
        let text = serde_json::to_string_pretty(&t).map_err(|e| e.to_string())? + "\n";
        lore_notation::write_atomic(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }
    /// Kinds the library adds to the built-in ones.
    pub fn kinds(&self) -> Vec<String> {
        self.templates.keys().filter(|k| !crate::spec::MODULES.iter().any(|(m, _)| m == k)).cloned().collect()
    }
    /// Saved styles of a kind.
    pub fn styles(&self, kind: &str) -> Vec<String> {
        self.templates.get(kind).map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }
    /// The drawing of `kind` in `style`; with no style, `default` or the first one.
    pub fn get(&self, kind: &str, style: &str) -> Option<&ModuleTemplate> {
        let m = self.templates.get(kind)?;
        if style.is_empty() { m.get(DEFAULT_STYLE).or_else(|| m.values().next()) } else { m.get(style) }
    }
    pub fn description(&self, kind: &str) -> String {
        self.templates.get(kind).and_then(|m| m.values().find(|t| !t.description.is_empty())).map(|t| t.description.clone()).unwrap_or_default()
    }
}

/// `role:*` / `theme:*` replaced by a real colour so the objects parse.
fn placeholder_colours(v: &Value) -> Value {
    match v {
        Value::String(s) if s.starts_with("role:") || s.starts_with("theme:") => Value::String("000000".into()),
        Value::Array(a) => Value::Array(a.iter().map(placeholder_colours).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), placeholder_colours(x))).collect()),
        other => other.clone(),
    }
}

/// How a selection becomes a module file.
#[derive(Debug, Clone, Default)]
pub struct SaveOptions {
    pub kind: String,
    pub style: String,
    pub description: String,
    /// Text on the drawing that stands for the node's label (becomes `{{label}}`).
    pub label: Option<String>,
    /// Replace the most used fill, outline and text colour by the role's
    /// colours, so the drawing follows the theme.
    pub role_colours: bool,
}

/// Selected canvas objects → a module drawing ("存成模組"): coordinates
/// relative to the selection's corner, ids and source XML dropped, attachments
/// to objects outside the selection released.
pub fn template_from_objects(objects: &[Object], opt: &SaveOptions) -> Result<ModuleTemplate, String> {
    if objects.is_empty() {
        return Err("nothing is selected".into());
    }
    let b = lore_canvas::geom::bounds(objects.iter().map(|o| &o.frame));
    if b.w <= 0 || b.h <= 0 {
        return Err("the selection has no size".into());
    }
    fn clean(o: &mut Object) {
        o.id = 0;
        o.src = None;
        o.slot = None;
        o.ph = None;
        o.name = o.name.rsplit('/').next().unwrap_or("").to_string();
        match &mut o.kind {
            Kind::Connector(c) => {
                // ids are given anew every time the module is placed
                c.start = None;
                c.end = None;
            }
            Kind::Group(g) => g.children.iter_mut().for_each(clean),
            Kind::Text(sp) | Kind::Shape(sp) => {
                if let Some(tb) = &mut sp.text {
                    for r in tb.paras.iter_mut().flat_map(|p| p.runs.iter_mut()) {
                        r.src = None;
                        r.k = 0;
                    }
                    for p in &mut tb.paras {
                        p.src = None;
                        p.k = 0;
                    }
                }
            }
            _ => {}
        }
    }
    let mut objs: Vec<Object> = objects.to_vec();
    // a selection that is one module group is saved as its parts
    if objs.len() == 1
        && let Kind::Group(g) = &objs[0].kind
        && g.ch.w == objs[0].frame.w
        && g.ch.h == objs[0].frame.h
    {
        let (gx, gy) = (objs[0].frame.x - g.ch.x, objs[0].frame.y - g.ch.y);
        let mut kids = g.children.clone();
        for k in &mut kids {
            k.frame.x += gx;
            k.frame.y += gy;
        }
        kids.retain(|k| !k.name.ends_with("/port") && k.name != "port");
        if !kids.is_empty() {
            objs = kids;
        }
    }
    let b = if objs.len() == objects.len() { b } else { lore_canvas::geom::bounds(objs.iter().map(|o| &o.frame)) };
    for o in &mut objs {
        clean(o);
        o.frame.x -= b.x;
        o.frame.y -= b.y;
    }
    let mut v = serde_json::to_value(&objs).map_err(|e| e.to_string())?;
    if let Some(label) = opt.label.as_deref().filter(|l| !l.trim().is_empty()) {
        fn mark(v: &mut Value, label: &str) {
            match v {
                Value::Object(m) => {
                    if m.contains_key("look") && m.get("text").and_then(|t| t.as_str()).map(str::trim) == Some(label.trim()) {
                        m.insert("text".into(), Value::String("{{label}}".into()));
                        m.insert("kind".into(), Value::String("text".into()));
                        m.remove("omml");
                        m.remove("latex");
                    }
                    m.values_mut().for_each(|x| mark(x, label));
                }
                Value::Array(a) => a.iter_mut().for_each(|x| mark(x, label)),
                _ => {}
            }
        }
        mark(&mut v, label);
    }
    if opt.role_colours {
        // count colours by where they are used
        fn count(v: &Value, key: Option<&str>, in_look: bool, out: &mut [BTreeMap<String, usize>; 3]) {
            match v {
                Value::Object(m) => {
                    if let Some(Value::String(c)) = m.get("color") {
                        let slot = if in_look {
                            Some(2)
                        } else if key == Some("fill") {
                            Some(0)
                        } else if key == Some("line") {
                            Some(1)
                        } else {
                            None
                        };
                        if let Some(i) = slot {
                            *out[i].entry(c.clone()).or_default() += 1;
                        }
                    }
                    for (k, x) in m {
                        count(x, Some(k), k == "look" || (in_look && k != "runs"), out);
                    }
                }
                Value::Array(a) => a.iter().for_each(|x| count(x, key, in_look, out)),
                _ => {}
            }
        }
        let mut tallies: [BTreeMap<String, usize>; 3] = Default::default();
        count(&v, None, false, &mut tallies);
        let top = |m: &BTreeMap<String, usize>| m.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(c, _)| c.clone());
        let picks = [top(&tallies[0]), top(&tallies[1]), top(&tallies[2])];
        fn swap(v: &mut Value, key: Option<&str>, in_look: bool, picks: &[Option<String>; 3]) {
            match v {
                Value::Object(m) => {
                    let slot = if in_look {
                        Some((2, "role:text"))
                    } else if key == Some("fill") {
                        Some((0, "role:fill"))
                    } else if key == Some("line") {
                        Some((1, "role:line"))
                    } else {
                        None
                    };
                    if let (Some((i, role)), Some(Value::String(c))) = (slot, m.get("color"))
                        && picks[i].as_deref() == Some(c.as_str())
                    {
                        m.insert("color".into(), Value::String(role.into()));
                    }
                    for (k, x) in m.iter_mut() {
                        let look = k == "look" || (in_look && k != "runs");
                        swap(x, Some(k.as_str()), look, picks);
                    }
                }
                Value::Array(a) => a.iter_mut().for_each(|x| swap(x, key, in_look, picks)),
                _ => {}
            }
        }
        swap(&mut v, None, false, &picks);
    }
    let t = ModuleTemplate { kind: opt.kind.clone(), style: opt.style.clone(), description: opt.description.clone(), w: b.w as f64 / EMU_PT, h: b.h as f64 / EMU_PT, port: None, objects: v };
    // must load back
    Library::default().add(t.clone())?;
    Ok(t)
}

pub struct Ctx<'a> {
    pub theme: &'a Theme,
    pub vars: &'a Vars,
    pub math: &'a dyn MathSource,
    pub lib: &'a Library,
    /// Scale: 1 = natural size.
    pub k: f64,
    /// Lines enter this module from the left and right (a left-to-right
    /// panel): the frame gets equal room above and below the body, so the
    /// side sites sit on the body's centre line. Otherwise (top-down flow)
    /// the frame is tight and the top and bottom sites touch the content.
    pub symmetric: bool,
    /// Pixel size of a picture file (`src=` of image nodes), when known.
    pub image_size: &'a dyn Fn(&str) -> Option<(u32, u32)>,
}

/// No picture files are known.
pub fn no_images(_: &str) -> Option<(u32, u32)> {
    None
}

/// A drawn module, origin at its frame's top-left corner.
#[derive(Debug, Clone)]
pub struct Body {
    /// Attach frame (pt, already scaled).
    pub w: f64,
    pub h: f64,
    /// Height at which lines enter the left and right sides (the frame's middle).
    pub cy: f64,
    /// Width the module needs including text that overflows the frame.
    pub reserve_w: f64,
    /// Objects with ids 0 and part names (`""` = the body itself).
    pub objs: Vec<Object>,
    /// One shape, inserted without a group.
    pub single: bool,
    /// The single shape is an ellipse (8 connection sites).
    pub ellipse: bool,
}

struct Core {
    w: f64,
    h: f64,
    objs: Vec<Object>,
    single: bool,
    ellipse: bool,
    /// Where the node's label goes when the core did not draw it.
    label: LabelPos,
}

#[derive(Clone, Copy, PartialEq)]
enum LabelPos {
    Drawn,
    Above,
    Below,
}

fn obj(name: &str, fr: Frame, kind: Kind) -> Object {
    Object { id: 0, name: name.into(), frame: fr, hidden: false, ph: None, slot: None, kind, src: None }
}

fn paint(hex: &str, alpha: f64) -> Paint {
    if hex == "none" || hex.is_empty() { Paint::None } else { Paint::Solid { color: hex.to_uppercase(), alpha } }
}

fn stroke(hex: &str, w: f64) -> Option<Stroke> {
    if hex == "none" || hex.is_empty() || w <= 0.0 { None } else { Some(Stroke::new(hex, w)) }
}

pub(crate) fn shape(name: &str, prst: &str, x: f64, y: f64, w: f64, h: f64, fill: Paint, line: Option<Stroke>) -> Object {
    obj(name, frame(x, y, w, h), Kind::Shape(SpObj { geom: Geom::preset(prst), fill, line, text: None, equation: false }))
}

/// A polygon (closed) or polyline through absolute points.
pub(crate) fn poly(name: &str, pts: &[(f64, f64)], closed: bool, fill: Paint, line: Option<Stroke>) -> Object {
    let x0 = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let y0 = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let x1 = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
    let y1 = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let (w, h) = ((x1 - x0).max(1e-6), (y1 - y0).max(1e-6));
    let n = |v: f64| {
        let s = format!("{:.5}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    let mut path = String::new();
    for (i, p) in pts.iter().enumerate() {
        path.push_str(&format!("{}{} {} ", if i == 0 { "M" } else { "L" }, n((p.0 - x0) / w), n((p.1 - y0) / h)));
    }
    if closed {
        path.push('Z');
    }
    let geom = Geom { prst: "custom".into(), adj: BTreeMap::new(), path: Some(path.trim().to_string()) };
    obj(name, frame(x0, y0, x1 - x0, y1 - y0), Kind::Shape(SpObj { geom, fill, line, text: None, equation: false }))
}

/// A straight, unattached line.
pub(crate) fn seg(name: &str, a: (f64, f64), b: (f64, f64), st: Stroke) -> Object {
    let fr = lore_canvas::geom::connector_frame((a.0 * EMU_PT, a.1 * EMU_PT), (b.0 * EMU_PT, b.1 * EMU_PT));
    obj(name, fr, Kind::Connector(ConnObj { geom: "straightConnector1".into(), adj: BTreeMap::new(), line: Some(st), start: None, end: None, text: None }))
}

impl Ctx<'_> {
    fn s(&self, v: f64) -> f64 {
        v * self.k
    }
    pub(crate) fn ty(&self, scale: f64, bold: bool, color: &str) -> Type {
        let f = &self.theme.font;
        Type { font: f.family.clone(), ea_font: f.ea.clone(), math_font: f.math.clone(), size: f.size * scale * self.k, bold, italic: false, color: color.to_uppercase() }
    }
    /// Captions, sizes, line labels, legend.
    pub(crate) fn small(&self, color: &str) -> Type {
        let f = &self.theme.font;
        Type { font: if f.caption.is_empty() { f.family.clone() } else { f.caption.clone() }, size: self.theme.small_size() * self.k, ..self.ty(1.0, false, color) }
    }
    /// Panel titles.
    pub(crate) fn title(&self) -> Type {
        let f = &self.theme.font;
        Type { font: if f.title.is_empty() { f.family.clone() } else { f.title.clone() }, size: self.theme.title_size() * self.k, bold: f.title_bold, ..self.ty(1.0, false, &self.theme.colors.ink) }
    }
    /// A text box of one or more centred lines; (object, width, height).
    pub(crate) fn text(&self, name: &str, s: &str, ty: &Type, cx: f64, y: f64) -> (Object, f64, f64) {
        let s = self.vars.resolve(s);
        let (tw, lines) = text::block(&s, ty.size);
        let (w, h) = (tw + ty.size * 0.5, lines as f64 * ty.size * 1.25);
        let (tb, has_math) = text::body(&s, ty, self.vars, self.math, "ctr", "ctr");
        let o = obj(name, frame(cx - w / 2.0, y, w, h), Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(tb), equation: has_math }));
        (o, w, h)
    }
    fn put_text(&self, o: &mut Object, s: &str, ty: &Type, vert: bool) {
        let s = self.vars.resolve(s);
        let (mut tb, has_math) = text::body(&s, ty, self.vars, self.math, "ctr", "ctr");
        if vert {
            tb.vert = Some("vert270".into());
        }
        if let Some(sp) = o.kind.sp_mut() {
            sp.text = Some(tb);
            sp.equation = has_math;
        }
    }
}

fn hex_param(n: &Node, k: &str) -> Option<String> {
    let v = n.str(k)?;
    let v = v.trim_start_matches('#');
    (v == "none" || (v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()))).then(|| v.to_uppercase())
}

struct Style {
    fill: String,
    line: String,
    text: String,
    lw: f64,
}

fn style(n: &Node, cx: &Ctx) -> Style {
    let r = cx.theme.role(n.role_or_default());
    // a node's own colour, then the theme's colour for this kind of module, then the role's
    let m = cx.theme.modules.get(&n.module);
    let pick = |param: &str, kind: Option<&String>, role: &String| hex_param(n, param).or_else(|| kind.cloned()).unwrap_or_else(|| role.clone());
    Style {
        fill: pick("fill", m.and_then(|m| m.fill.as_ref()), &r.fill),
        line: pick("line", m.and_then(|m| m.line.as_ref()), &r.line),
        text: m.and_then(|m| m.text.clone()).unwrap_or_else(|| r.text.clone()),
        lw: n.num("line_w").unwrap_or(cx.theme.node.line_w) * cx.k,
    }
}

/// The art style of a node: its own `style`, else the theme's for its kind.
pub fn style_name(n: &Node, theme: &Theme) -> String {
    if !n.style.is_empty() {
        return n.style.clone();
    }
    theme.modules.get(&n.module).and_then(|m| m.style.clone()).unwrap_or_default()
}

/// A fill for parts that must show on paper: the role's fill unless it is
/// none or the paper colour, then the tensor colour.
fn solid_fill(st: &Style, cx: &Ctx) -> String {
    if st.fill == "none" || st.fill.eq_ignore_ascii_case(&cx.theme.colors.paper) { cx.theme.colors.tensor.clone() } else { st.fill.clone() }
}

fn op_symbol(label: &str) -> String {
    match label.trim() {
        "+" | "⊕" | "add" | "sum" => "+".into(),
        "x" | "×" | "⊗" | "mul" | "*" => "×".into(),
        "." | "·" | "⊙" | "dot" | "odot" | "o" => "•".into(),
        "c" | "C" | "concat" | "cat" | "©" => "C".into(),
        "" => "+".into(),
        other => other.to_string(),
    }
}

fn round_adj(sp: &mut Object, radius: f64) {
    if let Some(s) = sp.kind.sp_mut() {
        s.geom.adj.insert("adj".into(), (radius * 100_000.0).round().clamp(0.0, 50_000.0));
    }
}

fn gen_box(n: &Node, cx: &Ctx) -> Core {
    let st = style(n, cx);
    let ty = cx.ty(1.0, n.flag("bold"), &st.text);
    let label = cx.vars.resolve(&n.label);
    let (tw, lines) = text::block(&label, ty.size);
    let lh = ty.size * 1.25;
    let vertical = n.flag("vertical");
    let (mut w, mut h) = if vertical { (lines as f64 * lh + cx.s(8.0), (tw + cx.s(14.0)).max(cx.s(48.0))) } else { ((tw + cx.s(18.0)).max(cx.s(40.0)), (lines as f64 * lh + cx.s(12.0)).max(cx.s(26.0))) };
    if let Some(v) = n.num("w") {
        w = cx.s(v);
    }
    if let Some(v) = n.num("h") {
        h = cx.s(v);
    }
    let named = style_name(n, cx.theme);
    let kind = n.str("shape").unwrap_or(if !named.is_empty() { named.as_str() } else if cx.theme.node.radius > 0.0 { "round" } else { "rect" });
    let prst = match kind {
        "round" => "roundRect",
        "rect" => "rect",
        "pill" => "roundRect",
        "trapezoid" => "trapezoid",
        "parallelogram" => "parallelogram",
        "diamond" => "diamond",
        "hexagon" => "hexagon",
        "ellipse" => "ellipse",
        _ => "rect",
    };
    let mut o = shape("", prst, 0.0, 0.0, w, h, paint(&st.fill, 1.0), stroke(&st.line, st.lw));
    match kind {
        "round" => round_adj(&mut o, if cx.theme.node.radius > 0.0 { cx.theme.node.radius } else { 0.18 }),
        "pill" => round_adj(&mut o, 0.5),
        _ => {}
    }
    cx.put_text(&mut o, &n.label, &ty, vertical);
    // rectangles have PowerPoint's four mid-side sites; other presets do not, so they get a port frame
    let single = matches!(prst, "rect" | "roundRect" | "ellipse");
    Core { w, h, objs: vec![o], single, ellipse: prst == "ellipse", label: LabelPos::Drawn }
}

fn gen_op(n: &Node, cx: &Ctx) -> Core {
    let st = style(n, cx);
    let sym = op_symbol(&n.label);
    let ty = cx.ty(if sym == "C" || sym.chars().count() > 1 { 0.95 } else { 1.25 }, sym.chars().count() == 1, &st.text);
    let d = cx.s(n.num("size").unwrap_or(20.0));
    let w = d.max(text::width(&sym, ty.size) + cx.s(8.0));
    let mut o = shape("", "ellipse", 0.0, 0.0, w, d, paint(&st.fill, 1.0), stroke(&st.line, st.lw));
    cx.put_text(&mut o, &sym, &ty, false);
    Core { w, h: d, objs: vec![o], single: true, ellipse: true, label: LabelPos::Drawn }
}

fn gen_loss(n: &Node, cx: &Ctx) -> Core {
    let st = style(n, cx);
    let ty = cx.ty(1.0, false, &st.text);
    let label = cx.vars.resolve(&n.label);
    let (tw, lines) = text::block(&label, ty.size);
    let named = style_name(n, cx.theme);
    let look = n.str("look").map(str::to_string).unwrap_or(if named.is_empty() { cx.theme.node.loss.clone() } else { named });
    let pad = if look == "plain" { cx.s(6.0) } else { cx.s(16.0) };
    let (w, h) = (tw + pad, lines as f64 * ty.size * 1.25 + if look == "plain" { cx.s(4.0) } else { cx.s(10.0) });
    let mut o = match look.as_str() {
        "pill" => {
            let mut o = shape("", "roundRect", 0.0, 0.0, w, h, paint(&st.fill, 1.0), stroke(&st.line, st.lw));
            round_adj(&mut o, 0.5);
            o
        }
        "box" => shape("", "rect", 0.0, 0.0, w, h, paint(&st.fill, 1.0), stroke(&st.line, st.lw)),
        _ => {
            let mut o = shape("", "rect", 0.0, 0.0, w, h, Paint::None, None);
            if let Kind::Shape(sp) = o.kind {
                o.kind = Kind::Text(sp);
            }
            o
        }
    };
    cx.put_text(&mut o, &n.label, &ty, false);
    Core { w, h, objs: vec![o], single: true, ellipse: false, label: LabelPos::Drawn }
}

fn gen_label(n: &Node, cx: &Ctx) -> Core {
    let st = style(n, cx);
    let ty = cx.ty(if n.flag("small") { cx.theme.font.small_scale } else { 1.0 }, n.flag("bold"), &st.text);
    let label = cx.vars.resolve(&n.label);
    let (tw, lines) = text::block(&label, ty.size);
    let (w, h) = (tw + cx.s(6.0), lines as f64 * ty.size * 1.25 + cx.s(2.0));
    let mut o = obj("", frame(0.0, 0.0, w, h), Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: None, equation: false }));
    cx.put_text(&mut o, &n.label, &ty, false);
    Core { w, h, objs: vec![o], single: true, ellipse: false, label: LabelPos::Drawn }
}

/// A picture placeholder drawn with shapes (photo, mask, heat map, plot).
fn picture(name: &str, look: &str, x: f64, y: f64, w: f64, h: f64, outline: &str, lw: f64, cx: &Ctx, detail: bool) -> Vec<Object> {
    let t = cx.theme;
    let mut v = Vec::new();
    let part = |p: &str| if name.is_empty() { p.to_string() } else { format!("{name}-{p}") };
    match look {
        "mask" => {
            v.push(shape(&part("bg"), "rect", x, y, w, h, paint("000000", 1.0), stroke(outline, lw)));
            if detail {
                v.push(shape(&part("region"), "ellipse", x + w * 0.28, y + h * 0.3, w * 0.42, h * 0.36, paint("FFFFFF", 1.0), None));
            }
        }
        "heatmap" => {
            v.push(shape(&part("bg"), "rect", x, y, w, h, paint(&shade(&t.colors.tensor, 0.6), 1.0), stroke(outline, lw)));
            if detail {
                for (i, r) in [0.34, 0.22, 0.1].iter().enumerate() {
                    let c = shade(&t.colors.tensor, 0.2 - 0.35 * i as f64);
                    v.push(shape(&part(&format!("ring{i}")), "ellipse", x + w * (0.55 - r), y + h * (0.45 - r), w * r * 2.0, h * r * 2.0, paint(&c, 1.0), None));
                }
            }
        }
        "plot" => {
            v.push(shape(&part("bg"), "rect", x, y, w, h, paint(&t.colors.paper, 1.0), stroke(outline, lw)));
            if detail {
                let p = |u: f64, q: f64| (x + w * u, y + h * q);
                v.push(poly(&part("axes"), &[p(0.12, 0.1), p(0.12, 0.88), p(0.92, 0.88)], false, Paint::None, stroke(&t.colors.ink, lw.max(0.5 * cx.k))));
                v.push(poly(&part("curve-a"), &[p(0.12, 0.3), p(0.3, 0.3), p(0.5, 0.86)], false, Paint::None, stroke(&t.colors.muted, cx.k)));
                v.push(poly(&part("curve-b"), &[p(0.3, 0.86), p(0.52, 0.3), p(0.74, 0.86)], false, Paint::None, stroke(&t.colors.muted, cx.k)));
                v.push(poly(&part("curve-c"), &[p(0.54, 0.86), p(0.74, 0.3), p(0.92, 0.3)], false, Paint::None, stroke(&t.colors.muted, cx.k)));
            }
        }
        "blank" => v.push(shape(&part("bg"), "rect", x, y, w, h, paint(&t.colors.paper, 1.0), stroke(outline, lw))),
        _ => {
            v.push(shape(&part("bg"), "rect", x, y, w, h, paint(&t.colors.image, 1.0), stroke(outline, lw)));
            if detail {
                let dark = shade(&t.colors.image, -0.28);
                let p = |u: f64, q: f64| (x + w * u, y + h * q);
                v.push(poly(&part("hill"), &[p(0.08, 0.9), p(0.36, 0.48), p(0.54, 0.72), p(0.68, 0.56), p(0.92, 0.9)], true, paint(&dark, 1.0), None));
                v.push(shape(&part("sun"), "ellipse", x + w * 0.64, y + h * 0.16, w * 0.18, h * 0.18, paint(&dark, 1.0), None));
            }
        }
    }
    v
}

fn pic_size(n: &Node, cx: &Ctx, default: f64) -> (f64, f64) {
    let s = n.num("size").unwrap_or(default);
    (cx.s(n.num("w").unwrap_or(s)), cx.s(n.num("h").unwrap_or(s)))
}

/// A picture file on the slide. With a known pixel size the picture keeps its
/// proportions: it is cropped to fill a box whose width and height are both
/// given, else the box follows the picture.
fn photo(name: &str, src: &str, x: f64, y: f64, w: f64, h: f64, outline: &str, lw: f64, cx: &Ctx) -> Object {
    let mut crop = lore_canvas::Crop::default();
    if let Some((pw, ph)) = (cx.image_size)(src).filter(|d| d.0 > 0 && d.1 > 0) {
        let (pic, boxr) = (pw as f64 / ph as f64, w / h);
        if pic > boxr * 1.001 {
            let cut = (1.0 - boxr / pic) / 2.0;
            crop.l = cut;
            crop.r = cut;
        } else if pic < boxr / 1.001 {
            let cut = (1.0 - pic / boxr) / 2.0;
            crop.t = cut;
            crop.b = cut;
        }
    }
    obj(name, frame(x, y, w, h), Kind::Picture(lore_canvas::PicObj { asset: format!("file:{src}"), crop, alpha: 1.0, line: stroke(outline, lw), clip: "rect".into(), masks: vec![], effects: vec![] }))
}

/// Box of a picture node: `w`/`h` (cropping), or `size` as the longer side of
/// the picture's own proportions.
fn photo_size(n: &Node, cx: &Ctx, src: &str, default: f64) -> (f64, f64) {
    let s = n.num("size").unwrap_or(default);
    match ((cx.image_size)(src), n.num("w"), n.num("h")) {
        (Some((pw, ph)), None, None) if pw > 0 && ph > 0 => {
            let r = (pw as f64 / ph as f64).clamp(0.4, 2.5);
            if r >= 1.0 { (cx.s(s), cx.s(s / r)) } else { (cx.s(s * r), cx.s(s)) }
        }
        _ => pic_size(n, cx, default),
    }
}

fn gen_image(n: &Node, cx: &Ctx) -> Core {
    let outline = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    let lw = n.num("line_w").unwrap_or(0.75) * cx.k;
    if let Some(src) = n.str("src").filter(|s| !s.is_empty()) {
        let (w, h) = photo_size(n, cx, src, 44.0);
        return Core { w, h, objs: vec![photo("picture", src, 0.0, 0.0, w, h, &outline, lw, cx)], single: false, ellipse: false, label: LabelPos::Below };
    }
    let (w, h) = pic_size(n, cx, 44.0);
    let objs = picture("", n.str("look").unwrap_or("photo"), 0.0, 0.0, w, h, &outline, lw, cx, true);
    Core { w, h, objs, single: false, ellipse: false, label: LabelPos::Below }
}

fn gen_image_stack(n: &Node, cx: &Ctx) -> Core {
    // real pictures: `srcs=[a.png,b.png]` back to front, or one `src` repeated
    let mut srcs = n.list("srcs");
    if srcs.is_empty()
        && let Some(s) = n.str("src").filter(|s| !s.is_empty())
    {
        srcs = vec![s.to_string(); n.num("n").unwrap_or(3.0).clamp(2.0, 8.0) as usize];
    }
    let count = if srcs.is_empty() { n.num("n").unwrap_or(3.0).clamp(2.0, 8.0) as usize } else { srcs.len().clamp(1, 8) };
    let (w, h) = match srcs.last() {
        Some(front) => photo_size(n, cx, front, 40.0),
        None => pic_size(n, cx, 40.0),
    };
    let off = cx.s(n.num("offset").unwrap_or(5.0));
    let outline = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    let lw = n.num("line_w").unwrap_or(0.75) * cx.k;
    let look = n.str("look").unwrap_or("photo");
    let total = off * (count - 1) as f64;
    let mut objs = Vec::new();
    // back to front: the front picture is bottom-left
    for i in 0..count {
        let back = (count - 1 - i) as f64;
        match srcs.get(i) {
            Some(src) => objs.push(photo(&format!("p{i}"), src, back * off, total - back * off, w, h, &outline, lw, cx)),
            None => objs.extend(picture(&format!("p{i}"), look, back * off, total - back * off, w, h, &outline, lw, cx, i == count - 1)),
        }
    }
    Core { w: w + total, h: h + total, objs, single: false, ellipse: false, label: LabelPos::Below }
}

fn gen_feature_stack(n: &Node, cx: &Ctx) -> Core {
    let size = cx.s(n.num("size").unwrap_or(34.0));
    let count = n.num("n").unwrap_or(4.0).clamp(2.0, 10.0) as usize;
    let off = cx.s(n.num("offset").unwrap_or(4.0));
    let fill = hex_param(n, "fill").unwrap_or_else(|| cx.theme.colors.tensor.clone());
    let line = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    let lw = 0.5 * cx.k;
    let alpha = n.num("alpha").unwrap_or(0.85);
    let total = off * (count - 1) as f64;
    let mut objs = Vec::new();
    if n.str("look") == Some("3d") || style_name(n, cx.theme) == "3d" {
        // upright sheets seen from the side: parallelograms
        let (sw, skew) = (size * 0.45, size * 0.28);
        for i in 0..count {
            let x = i as f64 * off * 1.6;
            let pts = [(x, skew), (x + sw, 0.0), (x + sw, size), (x, size + skew)];
            objs.push(poly(&format!("sheet{i}"), &pts, true, paint(&fill, alpha), stroke(&line, lw)));
        }
        return Core { w: sw + (count - 1) as f64 * off * 1.6, h: size + skew, objs, single: false, ellipse: false, label: LabelPos::Below };
    }
    for i in 0..count {
        let back = (count - 1 - i) as f64;
        let front = i == count - 1;
        objs.push(shape(&format!("map{i}"), "rect", back * off, total - back * off, size, size, paint(&fill, alpha), stroke(&line, if front { lw * 2.0 } else { lw })));
    }
    Core { w: size + total, h: size + total, objs, single: false, ellipse: false, label: LabelPos::Below }
}

fn gen_vector_bar(n: &Node, cx: &Ctx) -> Core {
    let count = n.num("n").unwrap_or(5.0).clamp(1.0, 24.0) as usize;
    let cell = cx.s(n.num("cell").unwrap_or(10.0));
    let horiz = n.str("dir") == Some("h");
    let line = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    let fill = hex_param(n, "fill");
    let mut objs = Vec::new();
    for i in 0..count {
        let (x, y) = if horiz { (i as f64 * cell, 0.0) } else { (0.0, i as f64 * cell) };
        let f = match &fill {
            Some(f) => f.clone(),
            // a grey ramp reads as "values" without any colour
            None => shade(&cx.theme.colors.tensor, 0.75 - 0.75 * ((i * 7) % count) as f64 / count as f64),
        };
        objs.push(shape(&format!("cell{i}"), "rect", x, y, cell, cell, paint(&f, 1.0), stroke(&line, 0.5 * cx.k)));
    }
    let (w, h) = if horiz { (cell * count as f64, cell) } else { (cell, cell * count as f64) };
    Core { w, h, objs, single: false, ellipse: false, label: LabelPos::Below }
}

fn gen_matrix(n: &Node, cx: &Ctx) -> Core {
    let rows = n.num("rows").unwrap_or(4.0).clamp(1.0, 16.0) as usize;
    let cols = n.num("cols").unwrap_or(4.0).clamp(1.0, 16.0) as usize;
    let cell = cx.s(n.num("cell").unwrap_or(9.0));
    let line = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    let base = hex_param(n, "fill").unwrap_or_else(|| cx.theme.colors.paper.clone());
    let hi = n.num("highlight").map(|v| v as usize);
    let mut objs = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let f = if hi == Some(r) { cx.theme.colors.tensor.clone() } else { base.clone() };
            objs.push(shape(&format!("r{r}c{c}"), "rect", c as f64 * cell, r as f64 * cell, cell, cell, paint(&f, 1.0), stroke(&line, 0.5 * cx.k)));
        }
    }
    Core { w: cell * cols as f64, h: cell * rows as f64, objs, single: false, ellipse: false, label: LabelPos::Below }
}

/// The kind of a layer: its name without index, subscripts or math.
fn layer_class(name: &str) -> String {
    let p = text::plain(name);
    let s: String = p.chars().take_while(|c| c.is_alphabetic() || *c == ' ').collect();
    let s = s.trim();
    if s.is_empty() { p } else { s.to_string() }
}

fn gen_layer_bars(n: &Node, cx: &Ctx) -> Core {
    let mut layers = n.list("layers");
    if layers.is_empty() {
        layers = vec!["Conv".into(), "Conv".into(), "Conv".into()];
    }
    let st = style(n, cx);
    let ty = cx.small(&st.text);
    let bw = n.num("bar_w").map(|v| cx.s(v)).unwrap_or(ty.size * 1.25 + cx.s(5.0));
    let gap = cx.s(n.num("gap").unwrap_or(4.0));
    let longest = layers.iter().map(|l| text::width(&cx.vars.resolve(l), ty.size)).fold(0.0, f64::max);
    let full = n.num("bar_h").map(|v| cx.s(v)).unwrap_or((longest + cx.s(12.0)).max(cx.s(52.0)));
    let taper = n.flag("taper");
    let solid = n.flag("solid");
    let mut classes: Vec<String> = Vec::new();
    let mut objs = Vec::new();
    let count = layers.len();
    for (i, name) in layers.iter().enumerate() {
        let cls = layer_class(name);
        let ci = classes.iter().position(|c| *c == cls).unwrap_or_else(|| {
            classes.push(cls.clone());
            classes.len() - 1
        });
        let colour = cx.theme.cycle(ci).to_string();
        // tapering keeps room for the name
        let h = if taper { (full * (1.0 - 0.4 * i as f64 / (count.max(2) - 1) as f64)).max(text::width(&cx.vars.resolve(name), ty.size) + cx.s(6.0)) } else { full };
        let x = i as f64 * (bw + gap);
        let fill = if solid { paint(&shade(&colour, 0.7), 1.0) } else { paint(&cx.theme.colors.paper, 1.0) };
        let mut o = shape(&format!("bar{i}"), "rect", x, (full - h) / 2.0, bw, h, fill, stroke(&colour, (cx.theme.node.line_w * 1.5) * cx.k));
        cx.put_text(&mut o, name, &ty, true);
        objs.push(o);
    }
    Core { w: count as f64 * (bw + gap) - gap, h: full, objs, single: false, ellipse: false, label: LabelPos::Above }
}

/// Block sizes (thickness, height, depth) of a cuboid sequence.
fn cuboid_dims(n: &Node) -> Vec<[f64; 3]> {
    if let Some(Value::Array(a)) = n.params.get("dims") {
        let v: Vec<[f64; 3]> = a
            .iter()
            .filter_map(|d| {
                let d = d.as_array()?;
                Some([d.first()?.as_f64()?, d.get(1)?.as_f64()?, d.get(2).and_then(|x| x.as_f64()).unwrap_or(10.0)])
            })
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    let count = n.num("n").unwrap_or(5.0).clamp(1.0, 12.0) as usize;
    let shape = n.str("shape").unwrap_or("down");
    (0..count)
        .map(|i| {
            let t = if count == 1 { 0.0 } else { i as f64 / (count - 1) as f64 };
            // 0 = large and thin, 1 = small and thick
            let level = match shape {
                "up" => 1.0 - t,
                "hourglass" => 1.0 - (2.0 * t - 1.0).abs(),
                "flat" => 0.0,
                _ => t,
            };
            [5.0 + 9.0 * level, 54.0 - 32.0 * level, 12.0 - 4.0 * level]
        })
        .collect()
}

fn gen_cuboid_seq(n: &Node, cx: &Ctx) -> Core {
    let dims = cuboid_dims(n);
    let fill = hex_param(n, "fill").unwrap_or_else(|| cx.theme.colors.tensor.clone());
    let fills = n.list("fills");
    let line = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    let alpha = n.num("alpha").unwrap_or(1.0);
    let lw = 0.5 * cx.k;
    let gap = cx.s(n.num("gap").unwrap_or(7.0));
    let labels = n.list("labels");
    let small = cx.ty(cx.theme.font.small_scale * 0.9, false, &cx.theme.colors.ink);
    let max_h = dims.iter().map(|d| d[1]).fold(0.0, f64::max);
    let max_d = dims.iter().map(|d| d[2]).fold(0.0, f64::max);
    let (ox, oy) = (0.5, 0.42); // depth direction on the page
    let top_pad = cx.s(max_d * oy);
    let mid = top_pad + cx.s(max_h) / 2.0;
    let mut x = 0.0;
    let mut objs = Vec::new();
    let mut bottom = top_pad + cx.s(max_h);
    for (i, d) in dims.iter().enumerate() {
        let (t, h, dep) = (cx.s(d[0]), cx.s(d[1]), cx.s(d[2]));
        let (dx, dy) = (dep * ox, dep * oy);
        let y = mid - h / 2.0;
        let c = fills.get(i).map(|f| f.trim_start_matches('#').to_uppercase()).filter(|f| f.len() == 6).unwrap_or_else(|| fill.clone());
        objs.push(poly(&format!("b{i}-top"), &[(x, y), (x + dx, y - dy), (x + t + dx, y - dy), (x + t, y)], true, paint(&shade(&c, cx.theme.shade.top), alpha), stroke(&line, lw)));
        objs.push(poly(&format!("b{i}-side"), &[(x + t, y), (x + t + dx, y - dy), (x + t + dx, y + h - dy), (x + t, y + h)], true, paint(&shade(&c, cx.theme.shade.side), alpha), stroke(&line, lw)));
        objs.push(shape(&format!("b{i}-front"), "rect", x, y, t, h, paint(&c, alpha), stroke(&line, lw)));
        if let Some(l) = labels.get(i).filter(|l| !l.is_empty()) {
            let (o, _, th) = cx.text(&format!("b{i}-dim"), l, &small, x + t / 2.0, mid + cx.s(max_h) / 2.0 + cx.s(2.0));
            bottom = bottom.max(mid + cx.s(max_h) / 2.0 + cx.s(2.0) + th);
            objs.push(o);
        }
        x += t + dx.max(cx.s(2.0)) + gap;
    }
    let last = dims.last().map(|d| cx.s(d[2]) * ox).unwrap_or(0.0);
    let w = x - gap - (last.max(cx.s(2.0)) - last);
    // keep the frame symmetric about the front faces' centre line
    let half = (mid).max(bottom - mid);
    for o in &mut objs {
        o.frame.y += emu(half - mid);
    }
    Core { w, h: 2.0 * half, objs, single: false, ellipse: false, label: LabelPos::Above }
}

fn gen_unet(n: &Node, cx: &Ctx) -> Core {
    let depth = n.num("depth").unwrap_or(3.0).clamp(1.0, 6.0) as usize;
    let st = style(n, cx);
    let fill = solid_fill(&st, cx);
    let (bw, gap, h0, step) = (cx.s(8.0), cx.s(8.0), cx.s(42.0), cx.s(11.0));
    let hl = |l: usize| h0 * 0.68f64.powi(l as i32);
    let count = 2 * depth + 1;
    let level = |i: usize| if i <= depth { i } else { 2 * depth - i };
    let mut objs = Vec::new();
    let total_h = (0..=depth).map(|l| l as f64 * step + hl(l)).fold(0.0, f64::max);
    // skip lines first so the bars cover their ends
    let dash = {
        let mut s = Stroke::new(&cx.theme.colors.muted, 0.75 * cx.k);
        s.dash = "dash".into();
        s
    };
    for l in 0..depth {
        let (a, b) = (l, 2 * depth - l);
        let y = l as f64 * step + cx.s(5.0);
        objs.push(seg(&format!("skip{l}"), (a as f64 * (bw + gap) + bw, y), (b as f64 * (bw + gap), y), dash.clone()));
    }
    for i in 0..count {
        let l = level(i);
        let x = i as f64 * (bw + gap);
        objs.push(shape(&format!("bar{i}"), "rect", x, l as f64 * step, bw, hl(l), paint(&fill, 1.0), stroke(&st.line, st.lw)));
    }
    Core { w: count as f64 * (bw + gap) - gap, h: total_h, objs, single: false, ellipse: false, label: LabelPos::Above }
}

/// Built-in art styles of the generator kinds (the first is the default).
pub fn generator_styles(kind: &str) -> &'static [&'static str] {
    match kind {
        "encoder" | "decoder" => &["trapezoid", "outline", "rounded", "gradient", "bars", "3d", "stacked"],
        "nn" => &["circles", "filled", "dots", "pills", "squares"],
        "brace" => &["curly", "square", "line", "arrow", "angle"],
        "feature-stack" => &["flat", "3d"],
        "loss" => &["plain", "pill", "box"],
        "box" => &["rect", "round", "pill"],
        _ => &[],
    }
}

/// encoder / decoder: a block that narrows (encoder) or widens (decoder) along the flow.
fn gen_coder(n: &Node, cx: &Ctx, decoder: bool) -> Core {
    let st = style(n, cx);
    let which = style_name(n, cx.theme);
    let (w, h) = (cx.s(n.num("w").unwrap_or(58.0)), cx.s(n.num("h").unwrap_or(46.0)));
    let ty = cx.ty(1.0, false, &st.text);
    // heights at the wide and the narrow end
    let narrow = 0.5;
    let frac = |t: f64| if decoder { narrow + (1.0 - narrow) * t } else { 1.0 - (1.0 - narrow) * t };
    let outline = |fill: Paint, lw: f64| {
        let (a, b) = (frac(0.0), frac(1.0));
        let pts = [(0.0, h * (1.0 - a) / 2.0), (w, h * (1.0 - b) / 2.0), (w, h * (1.0 + b) / 2.0), (0.0, h * (1.0 + a) / 2.0)];
        poly("", &pts, true, fill, stroke(&st.line, lw))
    };
    let steps = n.num("n").unwrap_or(4.0).clamp(2.0, 8.0) as usize;
    match which.as_str() {
        "rounded" => {
            let mut o = shape("", "roundRect", 0.0, 0.0, w, h * 0.8, paint(&st.fill, 1.0), stroke(&st.line, st.lw));
            round_adj(&mut o, 0.28);
            cx.put_text(&mut o, &n.label, &ty, false);
            Core { w, h: h * 0.8, objs: vec![o], single: true, ellipse: false, label: LabelPos::Drawn }
        }
        "outline" => {
            let mut o = outline(Paint::None, st.lw * 1.5);
            cx.put_text(&mut o, &n.label, &ty, false);
            Core { w, h, objs: vec![o], single: false, ellipse: false, label: LabelPos::Drawn }
        }
        "gradient" => {
            let base = solid_fill(&st, cx);
            let (from, to) = if decoder { (shade(&base, 0.75), base.clone()) } else { (base.clone(), shade(&base, 0.75)) };
            let stops = vec![lore_canvas::Stop { pos: 0.0, color: from, alpha: 1.0 }, lore_canvas::Stop { pos: 1.0, color: to, alpha: 1.0 }];
            let mut o = outline(Paint::Gradient { stops, angle: 0.0, path: false }, st.lw);
            cx.put_text(&mut o, &n.label, &ty, false);
            Core { w, h, objs: vec![o], single: false, ellipse: false, label: LabelPos::Drawn }
        }
        "bars" => {
            let fill = solid_fill(&st, cx);
            let (bw, gap) = (cx.s(9.0), cx.s(5.0));
            let mut objs = Vec::new();
            for i in 0..steps {
                let bh = h * frac(i as f64 / (steps - 1) as f64);
                let mut o = shape(&format!("bar{i}"), "roundRect", i as f64 * (bw + gap), (h - bh) / 2.0, bw, bh, paint(&fill, 1.0), stroke(&st.line, st.lw));
                round_adj(&mut o, 0.3);
                objs.push(o);
            }
            Core { w: steps as f64 * (bw + gap) - gap, h, objs, single: false, ellipse: false, label: LabelPos::Above }
        }
        "3d" => {
            let mut m = n.clone();
            let dims: Vec<Value> = (0..steps)
                .map(|i| {
                    let f = frac(i as f64 / (steps - 1) as f64);
                    serde_json::json!([5.0 + 8.0 * (1.0 - f) / (1.0 - narrow), 46.0 * f, 14.0 * f])
                })
                .collect();
            m.params.insert("dims".into(), Value::Array(dims));
            if hex_param(n, "fill").is_none() {
                m.params.insert("fill".into(), Value::String(solid_fill(&st, cx)));
            }
            gen_cuboid_seq(&m, cx)
        }
        "stacked" => {
            // translucent sheets one behind the other
            let fill = solid_fill(&st, cx);
            let (sw, off, skew) = (cx.s(16.0), cx.s(9.0), cx.s(7.0));
            let mut objs = Vec::new();
            for i in 0..steps {
                let sh = (h - skew) * frac(i as f64 / (steps - 1) as f64);
                let x = i as f64 * off;
                let y0 = (h - skew - sh) / 2.0;
                let pts = [(x, y0 + skew), (x + sw, y0), (x + sw, y0 + sh), (x, y0 + sh + skew)];
                objs.push(poly(&format!("sheet{i}"), &pts, true, paint(&fill, 0.55), stroke(&st.line, 0.75 * cx.k)));
            }
            Core { w: (steps - 1) as f64 * off + sw, h, objs, single: false, ellipse: false, label: LabelPos::Above }
        }
        _ => {
            let mut o = outline(paint(&st.fill, 1.0), st.lw);
            cx.put_text(&mut o, &n.label, &ty, false);
            Core { w, h, objs: vec![o], single: false, ellipse: false, label: LabelPos::Drawn }
        }
    }
}

/// nn: a small fully connected network, neurons and the lines between layers.
fn gen_nn(n: &Node, cx: &Ctx) -> Core {
    let st = style(n, cx);
    let which = style_name(n, cx.theme);
    let mut layers: Vec<usize> = n.list("layers").iter().filter_map(|l| l.trim().parse::<f64>().ok()).map(|v| v.clamp(1.0, 8.0) as usize).collect();
    if layers.is_empty() {
        layers = vec![3, 4, 2];
    }
    let (d, vgap, lgap) = match which.as_str() {
        "dots" => (cx.s(5.0), cx.s(6.0), cx.s(22.0)),
        "pills" => (cx.s(8.0), cx.s(3.0), cx.s(26.0)),
        _ => (cx.s(9.0), cx.s(5.0), cx.s(24.0)),
    };
    let pad = if which == "pills" { cx.s(3.0) } else { 0.0 };
    let tallest = *layers.iter().max().unwrap() as f64;
    let h = tallest * d + (tallest - 1.0) * vgap + 2.0 * pad;
    let centre = |li: usize, ni: usize| {
        let count = layers[li] as f64;
        let col_h = count * d + (count - 1.0) * vgap;
        (pad + li as f64 * (d + lgap + 2.0 * pad) + d / 2.0, (h - col_h) / 2.0 + ni as f64 * (d + vgap) + d / 2.0)
    };
    let mut objs = Vec::new();
    let wire = Stroke::new(if which == "dots" { &cx.theme.colors.muted } else { &st.line }, (if which == "dots" { 0.35 } else { 0.5 }) * cx.k);
    for li in 0..layers.len().saturating_sub(1) {
        for a in 0..layers[li] {
            for b in 0..layers[li + 1] {
                let (p, q) = (centre(li, a), centre(li + 1, b));
                // from edge to edge of the neurons, so outlined neurons stay clean
                objs.push(seg(&format!("w{li}-{a}-{b}"), (p.0 + d / 2.0 + pad, p.1), (q.0 - d / 2.0 - pad, q.1), wire.clone()));
            }
        }
    }
    for (li, count) in layers.iter().enumerate() {
        if which == "pills" {
            let (x, _) = centre(li, 0);
            let col_h = *count as f64 * d + (*count as f64 - 1.0) * vgap + 2.0 * pad;
            let mut o = shape(&format!("layer{li}"), "roundRect", x - d / 2.0 - pad, (h - col_h) / 2.0, d + 2.0 * pad, col_h, paint(&cx.theme.colors.paper, 1.0), stroke(&st.line, st.lw));
            round_adj(&mut o, 0.5);
            objs.push(o);
        }
        for ni in 0..*count {
            let (x, y) = centre(li, ni);
            let (prst, fill, line) = match which.as_str() {
                "filled" | "pills" => ("ellipse", paint(&solid_fill(&st, cx), 1.0), stroke(&st.line, 0.5 * cx.k)),
                "dots" => ("ellipse", paint(&st.line, 1.0), None),
                "squares" => ("rect", paint(&cx.theme.colors.paper, 1.0), stroke(&st.line, st.lw)),
                _ => ("ellipse", paint(&cx.theme.colors.paper, 1.0), stroke(&st.line, st.lw)),
            };
            objs.push(shape(&format!("n{li}-{ni}"), prst, x - d / 2.0, y - d / 2.0, d, d, fill, line));
        }
    }
    let w = layers.len() as f64 * (d + 2.0 * pad) + (layers.len() - 1) as f64 * lgap;
    Core { w, h, objs, single: false, ellipse: false, label: LabelPos::Above }
}

/// brace: gathers what is on its open side (`dir` = the side it points to).
fn gen_brace(n: &Node, cx: &Ctx) -> Core {
    let st = style(n, cx);
    let which = style_name(n, cx.theme);
    let len = cx.s(n.num("size").unwrap_or(56.0));
    let thick = cx.s(n.num("thick").unwrap_or(if which == "line" || which == "arrow" { 8.0 } else { 10.0 }));
    let dir = n.str("dir").unwrap_or("right").to_string();
    let vertical = dir == "left" || dir == "right";
    let line = Stroke::new(&st.line, st.lw);
    // drawn pointing right in a (thick × len) box, then turned
    let (w, h) = if vertical { (thick, len) } else { (len, thick) };
    let turn = |p: (f64, f64)| -> (f64, f64) {
        match dir.as_str() {
            "left" => (thick - p.0, p.1),
            "down" => (p.1, p.0),
            "up" => (p.1, thick - p.0),
            _ => p,
        }
    };
    let path = |pts: &[(f64, f64)], name: &str, st: Stroke| poly(name, &pts.iter().map(|p| turn(*p)).collect::<Vec<_>>(), false, Paint::None, Some(st));
    let objs = match which.as_str() {
        "square" => vec![path(&[(0.0, 0.0), (thick, 0.0), (thick, len), (0.0, len)], "", line)],
        "angle" => vec![path(&[(0.0, 0.0), (thick, len / 2.0), (0.0, len)], "", line)],
        "line" => vec![path(&[(0.0, 0.0), (thick, 0.0)], "tick0", line.clone()), path(&[(thick / 2.0, 0.0), (thick / 2.0, len)], "bar", line.clone()), path(&[(0.0, len), (thick, len)], "tick1", line)],
        "arrow" => {
            let mut both = line.clone();
            both.head = "triangle".into();
            both.tail = "triangle".into();
            vec![path(&[(0.0, 0.0), (thick, 0.0)], "tick0", line.clone()), path(&[(thick / 2.0, 0.0), (thick / 2.0, len)], "bar", both), path(&[(0.0, len), (thick, len)], "tick1", line)]
        }
        _ => {
            // PowerPoint's own brace, turned for the up/down directions
            let prst = if dir == "left" || dir == "up" { "leftBrace" } else { "rightBrace" };
            let mut o = shape("", prst, (w - thick) / 2.0, (h - len) / 2.0, thick, len, Paint::None, Some(line));
            if !vertical {
                o.frame.rot = 90.0;
            }
            vec![o]
        }
    };
    Core { w, h, objs, single: false, ellipse: false, label: LabelPos::Below }
}

fn resolve_colour(s: &str, st: &Style, t: &Theme) -> Option<String> {
    Some(match s {
        "role:fill" => st.fill.clone(),
        "role:line" => st.line.clone(),
        "role:text" => st.text.clone(),
        "theme:ink" => t.colors.ink.clone(),
        "theme:muted" => t.colors.muted.clone(),
        "theme:paper" => t.colors.paper.clone(),
        "theme:image" => t.colors.image.clone(),
        "theme:tensor" => t.colors.tensor.clone(),
        _ => return None,
    })
}

fn substitute(v: &mut Value, st: &Style, t: &Theme) {
    match v {
        Value::String(s) => {
            if let Some(c) = resolve_colour(s, st, t) {
                *s = c;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| substitute(x, st, t)),
        Value::Object(m) => {
            // a fill of "none" in a role becomes no paint
            if m.get("kind").and_then(|k| k.as_str()) == Some("solid")
                && let Some(Value::String(c)) = m.get("color")
                && resolve_colour(c, st, t).as_deref() == Some("none")
            {
                *v = serde_json::json!({ "kind": "none" });
                return;
            }
            m.values_mut().for_each(|x| substitute(x, st, t));
        }
        _ => {}
    }
}

/// Scale an object's frame, strokes, text sizes and children by `k`.
pub fn scale_obj(o: &mut Object, k: f64) {
    let sc = |v: Emu| (v as f64 * k).round() as Emu;
    o.frame.x = sc(o.frame.x);
    o.frame.y = sc(o.frame.y);
    o.frame.w = sc(o.frame.w);
    o.frame.h = sc(o.frame.h);
    let text = |tb: &mut lore_canvas::TextBody| {
        for i in &mut tb.insets {
            *i = (*i as f64 * k).round() as Emu;
        }
        for p in &mut tb.paras {
            for r in &mut p.runs {
                r.look.size = (r.look.size * k * 10.0).round() / 10.0;
            }
            if let Some(e) = &mut p.end {
                e.size = (e.size * k * 10.0).round() / 10.0;
            }
        }
    };
    match &mut o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => {
            if let Some(l) = &mut sp.line {
                l.width_pt *= k;
            }
            if let Some(tb) = &mut sp.text {
                text(tb);
            }
        }
        Kind::Connector(c) => {
            if let Some(l) = &mut c.line {
                l.width_pt *= k;
            }
        }
        Kind::Group(g) => {
            g.ch.x = sc(g.ch.x);
            g.ch.y = sc(g.ch.y);
            g.ch.w = sc(g.ch.w);
            g.ch.h = sc(g.ch.h);
            for c in &mut g.children {
                scale_obj(c, k);
            }
        }
        _ => {}
    }
}

fn gen_template(n: &Node, t: &ModuleTemplate, cx: &Ctx) -> Core {
    let st = style(n, cx);
    let mut v = t.objects.clone();
    substitute(&mut v, &st, cx.theme);
    let mut objs: Vec<Object> = serde_json::from_value(v).unwrap_or_default();
    let mut drawn = false;
    let label = cx.vars.resolve(&n.label);
    fn fill_label(o: &mut Object, label: &str, cx: &Ctx, drawn: &mut bool) {
        match &mut o.kind {
            Kind::Text(sp) | Kind::Shape(sp) => {
                let mut math = false;
                if let Some(tb) = &mut sp.text {
                    for p in &mut tb.paras {
                        if let Some(i) = p.runs.iter().position(|r| r.text.trim() == "{{label}}") {
                            let l = &p.runs[i].look;
                            let ty = Type { font: l.font.clone(), ea_font: l.ea_font.clone(), math_font: cx.theme.font.math.clone(), size: l.size, bold: l.bold, italic: l.italic, color: l.color.clone() };
                            let (rs, m) = text::runs(label, &ty, cx.vars, cx.math);
                            math |= m;
                            p.runs.splice(i..=i, rs);
                            *drawn = true;
                        }
                    }
                }
                sp.equation |= math;
            }
            Kind::Group(g) => g.children.iter_mut().for_each(|c| fill_label(c, label, cx, drawn)),
            _ => {}
        }
    }
    for (i, o) in objs.iter_mut().enumerate() {
        o.id = 0;
        o.src = None;
        if o.name.is_empty() {
            o.name = format!("part{i}");
        }
        if let Some(p) = &t.port {
            o.frame.x -= p.x;
            o.frame.y -= p.y;
        }
        scale_obj(o, cx.k);
        fill_label(o, &label, cx, &mut drawn);
    }
    Core { w: cx.s(t.w), h: cx.s(t.h), objs, single: false, ellipse: false, label: if drawn { LabelPos::Drawn } else { LabelPos::Below } }
}

/// Draw a node at scale `cx.k`.
pub fn make(node: &Node, cx: &Ctx) -> Body {
    // a node that only names its variable shows the variable as its label
    let mut n = node.clone();
    if n.label.is_empty() && !n.var.is_empty() {
        n.label = format!("@{}", std::mem::take(&mut n.var));
    }
    let n = &n;
    // a saved drawing of this kind and style wins over the built-in drawing
    let which = style_name(n, cx.theme);
    let saved = match cx.lib.templates.get(&n.module) {
        Some(styles) if !which.is_empty() => styles.get(&which),
        Some(_) if !crate::spec::MODULES.iter().any(|(m, _)| *m == n.module) => cx.lib.get(&n.module, ""),
        _ => None,
    };
    let core = match (saved, n.module.as_str()) {
        (Some(t), _) => gen_template(n, t, cx),
        (None, "box") => gen_box(n, cx),
        (None, "op") => gen_op(n, cx),
        (None, "loss") => gen_loss(n, cx),
        (None, "label") => gen_label(n, cx),
        (None, "image") => gen_image(n, cx),
        (None, "image-stack") => gen_image_stack(n, cx),
        (None, "feature-stack") => gen_feature_stack(n, cx),
        (None, "vector-bar") => gen_vector_bar(n, cx),
        (None, "matrix") => gen_matrix(n, cx),
        (None, "layer-bars") => gen_layer_bars(n, cx),
        (None, "cuboid-seq") => gen_cuboid_seq(n, cx),
        (None, "unet") => gen_unet(n, cx),
        (None, "encoder") => gen_coder(n, cx, false),
        (None, "decoder") => gen_coder(n, cx, true),
        (None, "nn") => gen_nn(n, cx),
        (None, "brace") => gen_brace(n, cx),
        // a library kind whose named style is missing falls back to its default drawing
        (None, other) => match cx.lib.get(other, "") {
            Some(t) => gen_template(n, t, cx),
            None => gen_box(n, cx),
        },
    };
    // text around the body: the label (above or below) and, below, the variable and caption
    let ink = cx.theme.colors.ink.clone();
    let ty = cx.ty(1.0, false, &ink);
    let small = cx.small(&cx.theme.colors.muted);
    let mut above: Vec<(String, &'static str, Type)> = Vec::new();
    let mut below: Vec<(String, &'static str, Type)> = Vec::new();
    match core.label {
        LabelPos::Above if !n.label.is_empty() => above.push((n.label.clone(), "label", ty.clone())),
        LabelPos::Below if !n.label.is_empty() => below.push((n.label.clone(), "label", ty.clone())),
        _ => {}
    }
    if !n.var.is_empty() {
        below.push((format!("@{}", n.var), "var", ty.clone()));
    }
    if !n.caption.is_empty() {
        below.push((n.caption.clone(), "caption", small));
    }
    if above.is_empty() && below.is_empty() && core.single {
        let (w, h) = (core.w, core.h);
        return Body { w, h, cy: h / 2.0, reserve_w: w, objs: core.objs, single: true, ellipse: core.ellipse };
    }
    let pad = cx.s(2.0);
    let measure = |v: &[(String, &str, Type)]| -> f64 { v.iter().map(|(s, _, t)| text::block(&cx.vars.resolve(s), t.size).1 as f64 * t.size * 1.25).sum::<f64>() + if v.is_empty() { 0.0 } else { pad } };
    let (top, bottom) = (measure(&above), measure(&below));
    let strip = if cx.symmetric { top.max(bottom) } else { top };
    let mut objs = Vec::new();
    let (w, h) = (core.w, core.h + strip + if cx.symmetric { strip } else { bottom });
    objs.push(shape("port", "rect", 0.0, 0.0, w, h, Paint::None, None));
    for mut o in core.objs {
        o.frame.y += emu(strip);
        if o.name.is_empty() {
            o.name = "body".into();
        }
        objs.push(o);
    }
    let mut reserve_w = w;
    let mut y = strip - pad;
    for (s, name, t) in above.iter().rev() {
        let lines = text::block(&cx.vars.resolve(s), t.size).1 as f64;
        y -= lines * t.size * 1.25;
        let (o, tw, _) = cx.text(name, s, t, w / 2.0, y);
        reserve_w = reserve_w.max(tw);
        objs.push(o);
    }
    let mut y = strip + core.h + pad;
    for (s, name, t) in &below {
        let (o, tw, th) = cx.text(name, s, t, w / 2.0, y);
        y += th;
        reserve_w = reserve_w.max(tw);
        objs.push(o);
    }
    Body { w, h, cy: h / 2.0, reserve_w, objs, single: false, ellipse: false }
}

/// Wrap a body's objects into what goes on the slide: the single shape, or a
/// group named `name` with its parts named `name/<part>`. Ids start at
/// `first_id`; returns the object and the next free id.
pub fn place(body: Body, name: &str, x: f64, y: f64, first_id: u32) -> (Object, u32) {
    let mut id = first_id;
    if body.single && body.objs.len() == 1 {
        let mut o = body.objs.into_iter().next().unwrap();
        o.id = id;
        o.name = name.to_string();
        o.frame.x += emu(x);
        o.frame.y += emu(y);
        return (o, id + 1);
    }
    let gid = id;
    id += 1;
    fn number(o: &mut Object, id: &mut u32, prefix: &str) {
        o.id = *id;
        *id += 1;
        o.name = format!("{prefix}/{}", o.name);
        if let Kind::Group(g) = &mut o.kind {
            let p = o.name.clone();
            for c in &mut g.children {
                number(c, id, &p);
            }
        }
    }
    let mut children = body.objs;
    for c in &mut children {
        number(c, &mut id, name);
    }
    let fr = frame(x, y, body.w, body.h);
    let ch = Frame::new(0, 0, fr.w, fr.h);
    let mut group = obj(name, fr, Kind::Group(GroupObj { ch, children }));
    group.id = gid;
    (group, id)
}
