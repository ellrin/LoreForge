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
    /// `theme:muted`, `theme:paper`, `theme:image`, `theme:tensor`,
    /// `layer:<type>` (the theme's colour of a layer type); a run whose
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
        Value::String(s) if s.starts_with("role:") || s.starts_with("theme:") || s.starts_with("layer:") => Value::String("000000".into()),
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
                    if let Some(Value::String(c)) = m.get("color").filter(|c| c.as_str().is_some_and(|c| !c.contains(':'))) {
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
    /// Layer types it coloured, for the figure's colour legend.
    pub legend: Vec<crate::art::Swatch>,
}

#[derive(Default)]
pub(crate) struct Core {
    pub w: f64,
    pub h: f64,
    pub objs: Vec<Object>,
    pub single: bool,
    pub ellipse: bool,
    /// Where the node's label goes when the core did not draw it.
    pub label: LabelPos,
    /// Layer types the drawing coloured (for the figure's colour legend).
    pub legend: Vec<crate::art::Swatch>,
    /// Named points (scaled, in the core's frame) a matched pair joins up.
    pub anchors: Vec<(String, (f64, f64))>,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum LabelPos {
    #[default]
    Drawn,
    Above,
    Below,
}

pub(crate) fn obj(name: &str, fr: Frame, kind: Kind) -> Object {
    Object { id: 0, name: name.into(), frame: fr, hidden: false, ph: None, slot: None, kind, src: None }
}

pub(crate) fn paint(hex: &str, alpha: f64) -> Paint {
    if hex == "none" || hex.is_empty() { Paint::None } else { Paint::Solid { color: hex.to_uppercase(), alpha } }
}

pub(crate) fn stroke(hex: &str, w: f64) -> Option<Stroke> {
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
    pub(crate) fn put_text_pub(&self, o: &mut Object, s: &str, ty: &Type, vert: bool) {
        self.put_text(o, s, ty, vert)
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

pub(crate) fn hex_param(n: &Node, k: &str) -> Option<String> {
    let v = n.str(k)?;
    let v = v.trim_start_matches('#');
    (v == "none" || (v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()))).then(|| v.to_uppercase())
}

struct Style {
    fill: String,
    line: String,
    text: String,
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
    }
}

/// The art style of a node: its own `style`, else the theme's for its kind.
pub fn style_name(n: &Node, theme: &Theme) -> String {
    if !n.style.is_empty() {
        return n.style.clone();
    }
    theme.modules.get(&n.module).and_then(|m| m.style.clone()).unwrap_or_default()
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

pub(crate) fn round_adj(sp: &mut Object, radius: f64) {
    if let Some(s) = sp.kind.sp_mut() {
        s.geom.adj.insert("adj".into(), (radius * 100_000.0).round().clamp(0.0, 50_000.0));
    }
}

fn gen_box(n: &Node, cx: &Ctx) -> Core {
    let mut f = crate::finish::Finish::of(n, cx.theme);
    let ty = cx.ty(1.0, n.flag("bold"), &f.text);
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
    let kind = n.str("shape").unwrap_or(if named.is_empty() { "rect" } else { named.as_str() });
    // round and pill are corner radii (a finish) of the rectangle
    match kind {
        "round" if f.radius <= 0.0 => f.radius = 0.18,
        "pill" => f.radius = 0.5,
        _ => {}
    }
    let prst = match kind {
        "trapezoid" => "trapezoid",
        "parallelogram" => "parallelogram",
        "hexagon" => "hexagon",
        "ellipse" => "ellipse",
        "can" | "cylinder" => "can",
        _ if f.radius > 0.0 => "roundRect",
        _ => "rect",
    };
    if prst == "ellipse" {
        w *= 1.2;
        h *= 1.15;
    }
    let (fill, line) = f.part(&f.fill, cx.k);
    let mut o = shape("", prst, 0.0, 0.0, w, h, fill, line);
    if prst == "roundRect" {
        round_adj(&mut o, f.radius);
    }
    cx.put_text(&mut o, &n.label, &ty, vertical);
    // rectangles have PowerPoint's four mid-side sites; other presets do not, so they get a port frame
    let single = matches!(prst, "rect" | "roundRect" | "ellipse");
    Core { w, h, objs: vec![o], single, ellipse: prst == "ellipse", label: LabelPos::Drawn, ..Default::default() }
}

fn gen_op(n: &Node, cx: &Ctx) -> Core {
    let f = crate::finish::Finish::of(n, cx.theme);
    let sym = op_symbol(&n.label);
    let named = style_name(n, cx.theme);
    let ty = cx.ty(if sym == "C" || sym.chars().count() > 1 { 0.95 } else { 1.25 }, sym.chars().count() == 1, &f.text);
    let d = cx.s(n.num("size").unwrap_or(20.0));
    let w = d.max(text::width(&sym, ty.size) + cx.s(8.0));
    let (fill, line) = f.part(&f.fill, cx.k);
    match named.as_str() {
        "box" | "square" => {
            let mut o = shape("", "rect", 0.0, 0.0, w, d, fill, line);
            cx.put_text(&mut o, &sym, &ty, false);
            Core { w, h: d, objs: vec![o], single: true, label: LabelPos::Drawn, ..Default::default() }
        }
        "plain" => {
            let mut o = obj("", frame(0.0, 0.0, w, d), Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: None, equation: false }));
            cx.put_text(&mut o, &sym, &ty, false);
            Core { w, h: d, objs: vec![o], single: true, label: LabelPos::Drawn, ..Default::default() }
        }
        _ => {
            let mut o = shape("", "ellipse", 0.0, 0.0, w, d, fill, line);
            cx.put_text(&mut o, &sym, &ty, false);
            Core { w, h: d, objs: vec![o], single: true, ellipse: true, label: LabelPos::Drawn, ..Default::default() }
        }
    }
}

fn gen_loss(n: &Node, cx: &Ctx) -> Core {
    let mut f = crate::finish::Finish::of(n, cx.theme);
    let ty = cx.ty(1.0, false, &f.text);
    let label = cx.vars.resolve(&n.label);
    let (tw, lines) = text::block(&label, ty.size);
    let named = style_name(n, cx.theme);
    let mut look = n.str("look").map(str::to_string).unwrap_or(if named.is_empty() { cx.theme.node.loss.clone() } else { named });
    if look == "pill" {
        // a pill is a box with round ends (a finish)
        look = "box".into();
        f.radius = 0.5;
    }
    let pad = if look == "plain" { cx.s(6.0) } else { cx.s(16.0) };
    let (mut w, mut h) = (tw + pad, lines as f64 * ty.size * 1.25 + if look == "plain" { cx.s(4.0) } else { cx.s(10.0) });
    let (fill, line) = f.part(&f.fill, cx.k);
    let mut o = match look.as_str() {
        "box" => {
            let mut o = shape("", if f.radius > 0.0 { "roundRect" } else { "rect" }, 0.0, 0.0, w, h, fill, line);
            if f.radius > 0.0 {
                round_adj(&mut o, f.radius);
            }
            o
        }
        "circle" => {
            w = w.max(h) * 1.1;
            h = h.max(w * 0.7);
            shape("", "ellipse", 0.0, 0.0, w, h, fill, line)
        }
        _ => {
            let mut o = shape("", "rect", 0.0, 0.0, w, h, Paint::None, None);
            if let Kind::Shape(sp) = o.kind {
                o.kind = Kind::Text(sp);
            }
            o
        }
    };
    cx.put_text(&mut o, &n.label, &ty, false);
    Core { w, h, objs: vec![o], single: true, ellipse: look == "circle", label: LabelPos::Drawn, ..Default::default() }
}

fn gen_label(n: &Node, cx: &Ctx) -> Core {
    let st = style(n, cx);
    let ty = cx.ty(if n.flag("small") { cx.theme.font.small_scale } else { 1.0 }, n.flag("bold"), &st.text);
    let label = cx.vars.resolve(&n.label);
    let (tw, lines) = text::block(&label, ty.size);
    let (w, h) = (tw + cx.s(6.0), lines as f64 * ty.size * 1.25 + cx.s(2.0));
    let mut o = obj("", frame(0.0, 0.0, w, h), Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: None, equation: false }));
    cx.put_text(&mut o, &n.label, &ty, false);
    Core { w, h, objs: vec![o], single: true, ellipse: false, label: LabelPos::Drawn, ..Default::default() }
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
        return Core { w, h, objs: vec![photo("picture", src, 0.0, 0.0, w, h, &outline, lw, cx)], single: false, ellipse: false, label: LabelPos::Below, ..Default::default() };
    }
    let (w, h) = pic_size(n, cx, 44.0);
    let objs = picture("", n.str("look").unwrap_or("photo"), 0.0, 0.0, w, h, &outline, lw, cx, true);
    Core { w, h, objs, single: false, ellipse: false, label: LabelPos::Below, ..Default::default() }
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
    let named = style_name(n, cx.theme);
    let one = |i: usize, x: f64, y: f64, w: f64, h: f64, detail: bool, objs: &mut Vec<Object>| match srcs.get(i) {
        Some(src) => objs.push(photo(&format!("p{i}"), src, x, y, w, h, &outline, lw, cx)),
        None => objs.extend(picture(&format!("p{i}"), look, x, y, w, h, &outline, lw, cx, detail)),
    };
    let mut objs = Vec::new();
    match named.as_str() {
        "film" => {
            // frames on a strip with sprocket holes: a video
            let (fw, fh, gap, rim) = (w * 0.62, h * 0.62, cx.s(3.0), cx.s(6.0));
            let total = count as f64 * fw + (count + 1) as f64 * gap;
            objs.push(shape("strip", "rect", 0.0, 0.0, total, fh + 2.0 * rim, paint(&cx.theme.colors.ink, 1.0), None));
            let holes = ((total / cx.s(6.0)) as usize).max(4);
            for i in 0..holes {
                let x = (i as f64 + 0.5) * total / holes as f64 - cx.s(1.2);
                for (j, y) in [rim * 0.3, fh + rim * 1.25].iter().enumerate() {
                    objs.push(shape(&format!("hole{i}-{j}"), "rect", x, *y, cx.s(2.4), rim * 0.45, paint(&cx.theme.colors.paper, 1.0), None));
                }
            }
            for i in 0..count {
                one(i, gap + i as f64 * (fw + gap), rim, fw, fh, true, &mut objs);
            }
            Core { w: total, h: fh + 2.0 * rim, objs, label: LabelPos::Below, ..Default::default() }
        }
        "mosaic" => {
            // a 2 × 2 grid of pictures: a batch
            let (cw, ch, gap) = (w * 0.55, h * 0.55, cx.s(2.0));
            for i in 0..4 {
                one(i, (i % 2) as f64 * (cw + gap), (i / 2) as f64 * (ch + gap), cw, ch, true, &mut objs);
            }
            Core { w: 2.0 * cw + gap, h: 2.0 * ch + gap, objs, label: LabelPos::Below, ..Default::default() }
        }
        "row" => {
            // a few frames side by side and an ellipsis: a sequence
            let (cw, ch, gap) = (w * 0.6, h * 0.6, cx.s(3.0));
            let shown = count.min(3);
            for i in 0..shown {
                one(i, i as f64 * (cw + gap), 0.0, cw, ch, true, &mut objs);
            }
            let ty = cx.ty(1.0, true, &cx.theme.colors.ink);
            let (o, tw, th) = cx.text("more", "…", &ty, shown as f64 * (cw + gap) + cx.s(6.0), (ch - ty.size * 1.25) / 2.0);
            let _ = th;
            objs.push(o);
            Core { w: shown as f64 * (cw + gap) + tw.max(cx.s(12.0)), h: ch, objs, label: LabelPos::Below, ..Default::default() }
        }
        _ => {
            let total = off * (count - 1) as f64;
            // back to front: the front picture is bottom-left
            for i in 0..count {
                let back = (count - 1 - i) as f64;
                one(i, back * off, total - back * off, w, h, i == count - 1, &mut objs);
            }
            Core { w: w + total, h: h + total, objs, label: LabelPos::Below, ..Default::default() }
        }
    }
}

fn gen_feature_stack(n: &Node, cx: &Ctx) -> Core {
    let size = cx.s(n.num("size").unwrap_or(34.0));
    let count = n.num("n").unwrap_or(4.0).clamp(2.0, 10.0) as usize;
    let off = cx.s(n.num("offset").unwrap_or(4.0));
    let mut f = crate::finish::Finish::of(n, cx.theme);
    if hex_param(n, "fill").is_none() && cx.theme.modules.get("feature-stack").and_then(|m| m.fill.as_ref()).is_none() {
        f.fill = cx.theme.colors.tensor.clone();
    }
    if hex_param(n, "line").is_none() && cx.theme.modules.get("feature-stack").and_then(|m| m.line.as_ref()).is_none() {
        f.line = cx.theme.colors.ink.clone();
    }
    f.lw = n.num("line_w").unwrap_or(0.5);
    f.alpha = n.num("alpha").unwrap_or(if f.alpha < 1.0 { f.alpha } else { 0.85 });
    let total = off * (count - 1) as f64;
    let mut objs = Vec::new();
    let named = style_name(n, cx.theme);
    let style = if n.str("look") == Some("3d") { "3d" } else { named.as_str() };
    let col = f.base();
    let k = cx.k;
    match style {
        "3d" => {
            // upright sheets seen from the side: parallelograms
            let (sw, skew) = (size * 0.45, size * 0.28);
            for i in 0..count {
                let x = i as f64 * off * 1.6;
                let pts = [(x, skew), (x + sw, 0.0), (x + sw, size), (x, size + skew)];
                let (p, s) = f.part(&col, k);
                objs.push(poly(&format!("sheet{i}"), &pts, true, p, s));
            }
            Core { w: sw + (count - 1) as f64 * off * 1.6, h: size + skew, objs, label: LabelPos::Below, ..Default::default() }
        }
        "cube" => {
            // one tensor as a block with H, W and C on its edges
            let (fw, fh, d) = (size * 0.75, size * 0.75, size * 0.55);
            let (dx, dy) = (d * 0.6, d * 0.45);
            let fa = |t: f64| f.face(&col, t, k);
            let (p, s) = fa(cx.theme.shade.top);
            objs.push(poly("top", &[(0.0, dy), (dx, 0.0), (dx + fw, 0.0), (fw, dy)], true, p, s));
            let (p, s) = fa(cx.theme.shade.side);
            objs.push(poly("side", &[(fw, dy), (fw + dx, 0.0), (fw + dx, fh), (fw, fh + dy)], true, p, s));
            let (p, s) = fa(0.0);
            objs.push(shape("front", "rect", 0.0, dy, fw, fh, p, s));
            let ty = cx.small(&cx.theme.colors.muted);
            let mut ty = ty;
            ty.size *= 0.85;
            let (o, _, th) = cx.text("H", "$H$", &ty, -cx.s(4.0), dy + fh / 2.0 - ty.size * 0.62);
            objs.push(o);
            let (o, _, _) = cx.text("W", "$W$", &ty, fw / 2.0, dy + fh + cx.s(0.5));
            objs.push(o);
            let (o, _, _) = cx.text("C", "$C$", &ty, fw + dx * 0.5 + cx.s(5.0), fh + dy * 0.3);
            objs.push(o);
            let _ = th;
            Core { w: fw + dx, h: fh + dy, objs, label: LabelPos::Below, ..Default::default() }
        }
        "grid" => {
            // the front map shows its pixels
            for i in 0..count {
                let back = (count - 1 - i) as f64;
                let (x, y) = (back * off, total - back * off);
                let (p, s) = f.part(&col, k);
                objs.push(shape(&format!("map{i}"), "rect", x, y, size, size, p, s));
                if i == count - 1 {
                    let cells = 5;
                    let c = size / cells as f64;
                    let st = Stroke::new(&f.line, 0.35 * k);
                    for j in 1..cells {
                        objs.push(seg(&format!("v{j}"), (x + j as f64 * c, y), (x + j as f64 * c, y + size), st.clone()));
                        objs.push(seg(&format!("h{j}"), (x, y + j as f64 * c), (x + size, y + j as f64 * c), st.clone()));
                    }
                }
            }
            Core { w: size + total, h: size + total, objs, label: LabelPos::Below, ..Default::default() }
        }
        "row" => {
            // channels apart, side by side
            let (sw, skew, gap) = (size * 0.38, size * 0.22, size * 0.2);
            for i in 0..count.min(4) {
                let x = i as f64 * (sw + gap);
                let pts = [(x, skew), (x + sw, 0.0), (x + sw, size), (x, size + skew)];
                let c = cx.theme.cycle(i).to_string();
                let c = if f.typed && cx.theme.colors.cycle.len() > 1 { shade(&c, 0.45) } else { col.clone() };
                let (p, s) = f.part(&c, k);
                objs.push(poly(&format!("sheet{i}"), &pts, true, p, s));
            }
            let m = count.min(4) as f64;
            Core { w: m * sw + (m - 1.0) * gap, h: size + skew, objs, label: LabelPos::Below, ..Default::default() }
        }
        _ => {
            for i in 0..count {
                let back = (count - 1 - i) as f64;
                let front = i == count - 1;
                let (p, s) = f.part(&col, k);
                let s = s.map(|mut s| {
                    if front {
                        s.width_pt *= 2.0;
                    }
                    s
                });
                objs.push(shape(&format!("map{i}"), "rect", back * off, total - back * off, size, size, p, s));
            }
            Core { w: size + total, h: size + total, objs, label: LabelPos::Below, ..Default::default() }
        }
    }
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
    Core { w, h, objs, single: false, ellipse: false, label: LabelPos::Below, ..Default::default() }
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
    Core { w: cell * cols as f64, h: cell * rows as f64, objs, single: false, ellipse: false, label: LabelPos::Below, ..Default::default() }
}

/// The kind of a layer: its name without index, subscripts or math.
pub(crate) fn layer_class(name: &str) -> String {
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
    // the figure's classes in order (draw() hands them in), so a kind of layer has one colour everywhere
    let mut classes: Vec<String> = n.list("_classes");
    let mut legend = Vec::new();
    let typed = crate::finish::Finish::of(n, cx.theme).typed && cx.theme.colors.cycle.len() > 1;
    let mut objs = Vec::new();
    let count = layers.len();
    for (i, name) in layers.iter().enumerate() {
        let cls = layer_class(name);
        let ci = classes.iter().position(|c| *c == cls).unwrap_or_else(|| {
            classes.push(cls.clone());
            classes.len() - 1
        });
        // past the end of the cycle the colours come round lighter, so no two kinds share one
        let len = cx.theme.colors.cycle.len().max(1);
        let named = cls.chars().any(char::is_alphanumeric);
        let colour = if !named { cx.theme.colors.muted.clone() } else if ci < len { cx.theme.cycle(ci).to_string() } else { shade(cx.theme.cycle(ci), 0.45 * (ci / len) as f64) };
        if typed && named && !legend.iter().any(|s: &crate::art::Swatch| s.key == format!("bars:{cls}")) {
            legend.push(crate::art::Swatch { key: format!("bars:{cls}"), label: cls.clone(), color: colour.clone() });
        }
        // tapering keeps room for the name
        let h = if taper { (full * (1.0 - 0.4 * i as f64 / (count.max(2) - 1) as f64)).max(text::width(&cx.vars.resolve(name), ty.size) + cx.s(6.0)) } else { full };
        let x = i as f64 * (bw + gap);
        let fill = if solid { paint(&shade(&colour, 0.7), 1.0) } else { paint(&cx.theme.colors.paper, 1.0) };
        let mut o = shape(&format!("bar{i}"), "rect", x, (full - h) / 2.0, bw, h, fill, stroke(&colour, (cx.theme.node.line_w * 1.5) * cx.k));
        cx.put_text(&mut o, name, &ty, true);
        objs.push(o);
    }
    Core { w: count as f64 * (bw + gap) - gap, h: full, objs, single: false, ellipse: false, label: LabelPos::Above, legend, ..Default::default() }
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
    // classes=[conv,pool,…]: each block takes its layer type's colour and name from the theme
    let classes = n.list("classes");
    let mut legend: Vec<crate::art::Swatch> = Vec::new();
    // painted by the finish: outline = white blocks, translucent, gradient …
    let mut f = crate::finish::Finish::of(n, cx.theme);
    f.fill = fill.clone();
    f.line = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    f.lw = n.num("line_w").unwrap_or(0.5) / 0.6;
    if let Some(a) = n.num("alpha") {
        f.alpha = a;
    }
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
        let mut c = fills.get(i).map(|f| f.trim_start_matches('#').to_uppercase()).filter(|f| f.len() == 6).unwrap_or_else(|| fill.clone());
        if let Some(key) = classes.get(i).filter(|k| !k.is_empty()) {
            c = cx.theme.layer(key).to_string();
            if !legend.iter().any(|s| s.key == *key) {
                legend.push(crate::art::Swatch { key: key.clone(), label: cx.theme.layer_label(key), color: c.clone() });
            }
        }
        let (p, s) = f.face(&c, cx.theme.shade.top, cx.k);
        objs.push(poly(&format!("b{i}-top"), &[(x, y), (x + dx, y - dy), (x + t + dx, y - dy), (x + t, y)], true, p, s));
        let (p, s) = f.face(&c, cx.theme.shade.side, cx.k);
        objs.push(poly(&format!("b{i}-side"), &[(x + t, y), (x + t + dx, y - dy), (x + t + dx, y + h - dy), (x + t, y + h)], true, p, s));
        let (p, s) = f.face(&c, 0.0, cx.k);
        objs.push(shape(&format!("b{i}-front"), "rect", x, y, t, h, p, s));
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
    Core { w, h: 2.0 * half, objs, single: false, ellipse: false, label: LabelPos::Above, legend, ..Default::default() }
}

fn gen_unet(n: &Node, cx: &Ctx) -> Core {
    if style_name(n, cx.theme) == "lineart" {
        return unet_lineart(n, cx);
    }
    let depth = n.num("depth").unwrap_or(3.0).clamp(1.0, 6.0) as usize;
    let f = crate::finish::Finish::of(n, cx.theme);
    let fill = f.base();
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
        let (p, s) = f.part(&fill, cx.k);
        let mut o = shape(&format!("bar{i}"), if f.radius > 0.0 { "roundRect" } else { "rect" }, x, l as f64 * step, bw, hl(l), p, s);
        if f.radius > 0.0 {
            round_adj(&mut o, f.radius);
        }
        objs.push(o);
    }
    Core { w: count as f64 * (bw + gap) - gap, h: total_h, objs, label: LabelPos::Above, ..Default::default() }
}

/// The U-Net paper's line art: input picture, the U of conv pairs, skips
/// across, the output mask.
fn unet_lineart(n: &Node, cx: &Ctx) -> Core {
    let mut m = n.clone();
    m.style = "unet".into();
    let mut c = crate::art::pair(&m, cx);
    // drop the Encoder / Decoder names: the U says it
    c.objs.retain(|o| !o.name.ends_with("-name"));
    let k = cx.k;
    let s = cx.s(18.0);
    let (w, h) = (c.w, c.h);
    let pad = cx.s(8.0);
    for o in &mut c.objs {
        o.frame.x += emu(s + pad);
    }
    let ink = cx.theme.colors.ink.clone();
    let mut objs = picture("input", "photo", 0.0, cx.s(4.0), s, s, &ink, 0.5 * k, cx, true);
    objs.extend(std::mem::take(&mut c.objs));
    objs.extend(picture("output", "mask", s + 2.0 * pad + w, cx.s(4.0), s, s, &ink, 0.5 * k, cx, true));
    let st = {
        let mut st = Stroke::new(&ink, 0.75 * k);
        st.tail = "triangle".into();
        st
    };
    objs.push(seg("in", (s + cx.s(1.0), cx.s(4.0) + s / 2.0), (s + pad - cx.s(1.0), cx.s(4.0) + s / 2.0), st.clone()));
    objs.push(seg("out", (s + pad + w + cx.s(1.0), cx.s(4.0) + s / 2.0), (s + 2.0 * pad + w - cx.s(1.0), cx.s(4.0) + s / 2.0), st));
    Core { w: 2.0 * (s + pad) + w, h, objs, label: LabelPos::Above, legend: c.legend, ..Default::default() }
}

/// Built-in art styles of the generator kinds (the first is the default).
/// A style is a construction; fill, outline, gradient, radius and colours
/// are the finish ([`crate::finish`]).
pub fn generator_styles(kind: &str) -> &'static [&'static str] {
    match kind {
        "encoder" | "decoder" | "autoencoder" | "nn" => crate::art::styles(kind),
        "brace" => &["curly", "bracket", "paren", "angle", "dimension"],
        "feature-stack" => &["offset", "3d", "cube", "grid", "row"],
        "image-stack" => &["stack", "film", "mosaic", "row"],
        "loss" => &["plain", "box", "circle"],
        "op" => &["circle", "box", "plain"],
        "box" => &["rect", "trapezoid", "parallelogram", "hexagon", "ellipse", "can"],
        "unet" => &["bars", "lineart"],
        _ => &[],
    }
}

/// brace: gathers what is on its open side (`dir` = the side it points to).
fn gen_brace(n: &Node, cx: &Ctx) -> Core {
    let f = crate::finish::Finish::of(n, cx.theme);
    let which = style_name(n, cx.theme);
    let which = match which.as_str() {
        "line" | "arrow" | "span" => "dimension".to_string(),
        "square" => "bracket".to_string(),
        _ => which,
    };
    let len = cx.s(n.num("size").unwrap_or(56.0));
    // dashed ends (`ends=dash`, or the old style name span) mark a section of a network
    let dashed = n.str("ends") == Some("dash") || style_name(n, cx.theme) == "span";
    let thick = cx.s(n.num("thick").unwrap_or(match which.as_str() {
        "dimension" if dashed => 14.0,
        "dimension" => 8.0,
        _ => 10.0,
    }));
    let dir = n.str("dir").unwrap_or("right").to_string();
    let vertical = dir == "left" || dir == "right";
    let line = Stroke::new(&f.line, f.lw * cx.k);
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
    let heads = |mut s: Stroke| {
        s.head = "triangle".into();
        s.tail = "triangle".into();
        s
    };
    let objs = match which.as_str() {
        "bracket" => vec![path(&[(0.0, 0.0), (thick, 0.0), (thick, len), (0.0, len)], "", line)],
        "angle" => vec![path(&[(0.0, 0.0), (thick, len / 2.0), (0.0, len)], "", line)],
        "paren" => {
            // a parenthesis: one quadratic curve
            let pts = [turn((0.0, 0.0)), turn((thick * 2.0, len / 2.0)), turn((0.0, len))];
            let (bw, bh) = (w.max(0.01), h.max(0.01));
            let u = |p: (f64, f64)| format!("{:.4} {:.4}", p.0 / bw, p.1 / bh);
            let geom = Geom { prst: "custom".into(), adj: BTreeMap::new(), path: Some(format!("M{} Q{} {}", u(pts[0]), u(pts[1]), u(pts[2]))) };
            vec![obj("", frame(0.0, 0.0, w, h), Kind::Shape(SpObj { geom, fill: Paint::None, line: Some(line), text: None, equation: false }))]
        }
        "dimension" => {
            // a dimension line: end ticks (dashed: a section) and a double arrow (heads=none for a plain bar)
            let bar = if n.str("heads") == Some("none") { line.clone() } else { heads(line.clone()) };
            let mut end = line.clone();
            if dashed {
                end.dash = "sysDash".into();
            }
            let inset = if dashed { cx.s(1.0) } else { 0.0 };
            vec![path(&[(0.0, 0.0), (thick, 0.0)], "tick0", end.clone()), path(&[(thick / 2.0, inset), (thick / 2.0, len - inset)], "bar", bar), path(&[(0.0, len), (thick, len)], "tick1", end)]
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
    Core { w, h, objs, label: LabelPos::Below, ..Default::default() }
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
        s if s.starts_with("layer:") => t.layer(&s["layer:".len()..]).to_string(),
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
    Core { w: cx.s(t.w), h: cx.s(t.h), objs, single: false, ellipse: false, label: if drawn { LabelPos::Drawn } else { LabelPos::Below }, ..Default::default() }
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
        (None, "encoder") => crate::art::coder(n, cx, false),
        (None, "decoder") => crate::art::coder(n, cx, true),
        (None, "autoencoder") => crate::art::pair(n, cx),
        (None, "nn") => crate::art::nn(n, cx),
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
        return Body { w, h, cy: h / 2.0, reserve_w: w, objs: core.objs, single: true, ellipse: core.ellipse, legend: core.legend };
    }
    let legend = core.legend;
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
    Body { w, h, cy: h / 2.0, reserve_w, objs, single: false, ellipse: false, legend }
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
