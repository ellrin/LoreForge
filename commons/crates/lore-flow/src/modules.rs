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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModuleTemplate {
    pub kind: String,
    /// Style name; a file `modules/<kind>/<style>.json` is named after it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub style: String,
    /// One line for the LLM's module list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// What size of building block this is: `component` (the smallest: one thing that is not
    /// made of other blocks) or `module` (components and plain objects put together). Empty =
    /// a module, as every saved drawing was before there was a level.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    /// The blocks a module is made of, each `<kind>/<style>` (a saved drawing or a built-in
    /// kind and its style). The drawing itself is whole in `objects`; this is what it says
    /// about itself, for the hub's 包含.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub made_of: Vec<String>,
    /// Other names the thing is called by, in any language (`U-Net`, `U 形`, `跳接`): a
    /// description that uses one of them is about this block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// The built-in kinds or looks this block stands in for (`unet`, `chart=membership`,
    /// `image look=mask`): a draft that uses one of those is told about this block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    /// Kinds this block already draws as parts of itself (a classifier head draws its
    /// `layer-bars`, its `nn` and its `brace`): a neighbouring block of one of those kinds,
    /// joined to it by a line, draws the same thing a second time, and a draft is told so.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
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

impl ModuleTemplate {
    /// The words the drawing prints by itself (not the `{{label}}` slot), in drawing order.
    pub fn fixed_texts(&self) -> Vec<String> {
        fn walk(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    if let Some(Value::Array(runs)) = m.get("runs") {
                        let t: String = runs.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect();
                        let t = t.trim();
                        if !t.is_empty() && !t.contains("{{label}}") {
                            out.push(t.to_string());
                        }
                    }
                    m.values().for_each(|x| walk(x, out));
                }
                Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.objects, &mut out);
        out
    }
    /// Does a node use what this block replaces? An entry is a kind (`unet`), a kind with
    /// parameters (`image look=mask`) or parameters alone (`chart=membership`).
    pub fn stands_in_for(&self, n: &Node) -> bool {
        self.replaces.iter().any(|e| {
            let mut ok = true;
            let mut any = false;
            for tok in e.split_whitespace() {
                any = true;
                match tok.split_once('=') {
                    Some((k, v)) => ok &= n.str(k) == Some(v),
                    None => ok &= n.module == tok,
                }
            }
            any && ok
        })
    }
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
    let t = ModuleTemplate { kind: opt.kind.clone(), style: opt.style.clone(), description: opt.description.clone(), w: b.w as f64 / EMU_PT, h: b.h as f64 / EMU_PT, level: String::new(), made_of: Vec::new(), names: Vec::new(), replaces: Vec::new(), includes: Vec::new(), port: None, objects: v };
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
    /// The drawn picture without its words, in the body's frame (x, y, w, h): where lines
    /// connect (the `port` child). The whole body for a block that is one shape.
    pub art: (f64, f64, f64, f64),
    /// Words lie above the picture, and below it (a label, a variable, a caption).
    pub words: (bool, bool),
    /// The height of the words below the picture with their gap to it (0 without them), not
    /// counting `label_drop`.
    pub below: f64,
    /// The height of the words above the picture with their gap to it (0 without them), not
    /// counting `label_rise`.
    pub above: f64,
    /// The height of the line its neighbours in a row are aligned on: the middle of the
    /// picture's large pieces (see `main_middle`), which an attachment on one side (a loop
    /// arrow) does not move. Lines enter on it too (`cy` is the same height; the `port` child
    /// lies evenly about it).
    pub axis: f64,
    /// Objects with ids 0 and part names (`""` = the body itself).
    pub objs: Vec<Object>,
    /// One shape, inserted without a group.
    pub single: bool,
    /// The single shape is an ellipse (8 connection sites).
    pub ellipse: bool,
    /// Layer types it coloured, for the figure's colour legend.
    pub legend: Vec<crate::art::Swatch>,
    /// Problems of this node (an icon that does not exist); `build` should pass them on.
    pub warn: Vec<String>,
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
    /// The part of the core's box the drawing really fills (x, y, w, h in the core's frame),
    /// when it is smaller than the box; `None` = the whole box.
    pub art: Option<(f64, f64, f64, f64)>,
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
    f.fit(w, h, cx.k);
    match kind {
        // with a corner scheme the scheme decides how round a rectangle is
        "round" if f.radius <= 0.0 && f.corner.is_none() => f.radius = 0.18,
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
    f.fit(w, h, cx.k);
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

/// The problem of an `icon` node that cannot draw its picture.
fn icon_problem(n: &Node) -> Vec<String> {
    if n.module != "icon" || n.str("icon").is_some_and(|id| lore_icons::icon(id.trim()).is_some()) {
        return vec![];
    }
    let id = n.str("icon").unwrap_or("").trim();
    vec![if id.is_empty() { format!("{}: icon 方塊沒有 icon=", n.id) } else { format!("{}: 找不到圖示 {id}", n.id) }]
}

/// A small picture from the icon sets in the line colour; without a usable `icon=` a dashed
/// rounded square of the same size (and a warning).
fn gen_icon(n: &Node, cx: &Ctx) -> Core {
    let ink = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    let (w, h) = pic_size(n, cx, 44.0);
    let id = n.str("icon").unwrap_or("").trim();
    if let Some(ic) = lore_icons::icon(id) {
        let fr = frame(0.0, 0.0, w, h);
        let mut objs = lore_icons::objects(&ic, fr, &ink, 0, "");
        for (j, o) in objs.iter_mut().enumerate() {
            o.name = if j == 0 { "icon".into() } else { format!("icon-{}", j + 1) };
        }
        return Core { w, h, objs, single: false, ellipse: false, label: LabelPos::Below, ..Default::default() };
    }
    let mut st = Stroke::new(&ink, 0.75 * cx.k);
    st.dash = "sysDash".into();
    let mut o = shape("icon", "roundRect", 0.0, 0.0, w, h, Paint::None, Some(st));
    round_adj(&mut o, 0.15);
    Core { w, h, objs: vec![o], single: false, ellipse: false, label: LabelPos::Below, ..Default::default() }
}

/// A chart from the plotting tool ([`crate::chart`]); a dashed slot when it has none to draw.
fn gen_chart(n: &Node, cx: &Ctx) -> Core {
    let ink = hex_param(n, "line").unwrap_or_else(|| cx.theme.colors.ink.clone());
    let (w, h) = (cx.s(n.num("w").unwrap_or(84.0)), cx.s(n.num("h").unwrap_or(56.0)));
    let mut objs = crate::chart::objects(&n.params, w, h, &ink);
    if objs.is_empty() {
        let mut st = Stroke::new(&ink, 0.75 * cx.k);
        st.dash = "sysDash".into();
        objs.push(shape("chart", "rect", 0.0, 0.0, w, h, Paint::None, Some(st)));
    }
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

fn gen_layer_bars(n: &Node, cx: &Ctx, warn: &mut Vec<String>) -> Core {
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
    // the bars' own colours, in order; a shorter list repeats its last
    let own: Vec<String> = n.list("colors");
    let filled = n.flag("filled");
    let through = match n.params.get("through") {
        Some(Value::Bool(true)) => Some(cx.theme.colors.ink.clone()),
        Some(Value::Bool(false)) | None => None,
        Some(v) => {
            let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            if matches!(s.as_str(), "true" | "yes" | "on") {
                Some(cx.theme.colors.ink.clone())
            } else {
                match cx.theme.tint(&s) {
                    Some((_, line)) => Some(line),
                    None => {
                        warn.push(format!("{}: 不認得的顏色 {}", n.id, s));
                        None
                    }
                }
            }
        }
    };
    let mut bad: Vec<String> = Vec::new();
    let mut texts: Vec<Object> = Vec::new();
    for (i, name) in layers.iter().enumerate() {
        let mine: Option<(String, String)> = own.get(i).or(own.last()).map(String::as_str).filter(|c| c.trim() != "_").and_then(|c| match cx.theme.tint(c) {
            Some(p) => Some(p),
            None => {
                if !bad.iter().any(|b| b == c) {
                    bad.push(c.to_string());
                    warn.push(format!("{}: 不認得的顏色 {}", n.id, c));
                }
                None
            }
        });
        let cls = layer_class(name);
        let ci = classes.iter().position(|c| *c == cls).unwrap_or_else(|| {
            classes.push(cls.clone());
            classes.len() - 1
        });
        // past the end of the cycle the colours come round lighter, so no two kinds share one
        let len = cx.theme.colors.cycle.len().max(1);
        let named = cls.chars().any(char::is_alphanumeric);
        let colour = if !named { cx.theme.colors.muted.clone() } else if ci < len || len == 1 { cx.theme.cycle(ci).to_string() } else { shade(cx.theme.cycle(ci), 0.45 * (ci / len) as f64) };
        if mine.is_none() && typed && named && !legend.iter().any(|s: &crate::art::Swatch| s.key == format!("bars:{cls}")) {
            legend.push(crate::art::Swatch { key: format!("bars:{cls}"), label: cls.clone(), color: colour.clone() });
        }
        // tapering keeps room for the name
        let h = if taper { (full * (1.0 - 0.4 * i as f64 / (count.max(2) - 1) as f64)).max(text::width(&cx.vars.resolve(name), ty.size) + cx.s(6.0)) } else { full };
        let x = i as f64 * (bw + gap);
        let mut fill = if solid { paint(&shade(&colour, 0.7), 1.0) } else { paint(&cx.theme.colors.paper, 1.0) };
        let mut edge = colour.clone();
        if let Some((pale, strong)) = &mine {
            edge = strong.clone();
            if filled {
                fill = paint(pale, 1.0);
            }
        }
        let mut o = shape(&format!("bar{i}"), "rect", x, (full - h) / 2.0, bw, h, fill, stroke(&edge, (cx.theme.node.line_w * 1.5) * cx.k));
        if through.is_some() {
            // the text goes over the line, in a shape of its own
            let mut t = shape(&format!("bartext{i}"), "rect", x, (full - h) / 2.0, bw, h, Paint::None, None);
            cx.put_text(&mut t, name, &ty, true);
            texts.push(t);
        } else {
            cx.put_text(&mut o, name, &ty, true);
        }
        objs.push(o);
    }
    let width = count as f64 * (bw + gap) - gap;
    if let Some(c) = through {
        let mut st = Stroke::new(&c, (cx.theme.node.line_w * cx.k).max(0.5));
        st.dash = "dash".into();
        // beneath the bars (they are opaque): it shows in the gaps between them and never runs
        // over a bar's name
        objs.insert(0, seg("through", (0.0, full / 2.0), (width, full / 2.0), st));
        objs.extend(texts);
    }
    Core { w: width, h: full, objs, single: false, ellipse: false, label: LabelPos::Above, legend, ..Default::default() }
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

/// A saved block in the theme's style: its strokes at the nearest of the theme's line levels,
/// its rounded rectangles at the corner scheme's step radius (natural size, before scaling).
fn restyle(o: &mut Object, theme: &Theme) {
    let levels = &theme.node.levels;
    let snap = |l: &mut Stroke| {
        if l.width_pt > 0.0
            && let Some(best) = levels.iter().copied().min_by(|a, b| (a - l.width_pt).abs().total_cmp(&(b - l.width_pt).abs()))
        {
            l.width_pt = best;
        }
    };
    let (w, h) = (o.frame.w as f64 / EMU_PT, o.frame.h as f64 / EMU_PT);
    match &mut o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => {
            if let Some(l) = &mut sp.line {
                snap(l);
            }
            if let Some(c) = theme.corners
                && sp.geom.prst == "roundRect"
            {
                // a pill stays a pill; any other rounding follows the scheme
                let pill = sp.geom.adj.get("adj").is_some_and(|a| *a >= 45_000.0);
                if !pill {
                    sp.geom.adj.insert("adj".into(), crate::theme::Corners::adj(c.step, 1.0, w, h));
                }
            }
        }
        Kind::Connector(c) => {
            if let Some(l) = &mut c.line {
                snap(l);
            }
        }
        Kind::Group(g) => g.children.iter_mut().for_each(|c| restyle(c, theme)),
        _ => {}
    }
}

fn gen_template(n: &Node, t: &ModuleTemplate, cx: &Ctx, warn: &mut Vec<String>) -> Core {
    let st = style(n, cx);
    // the layer types a saved drawing names go into the colour legend, as when it was drawn
    fn layer_keys(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                if let Some(k) = s.strip_prefix("layer:")
                    && !out.iter().any(|x| x == k)
                {
                    out.push(k.to_string());
                }
            }
            Value::Array(a) => a.iter().for_each(|x| layer_keys(x, out)),
            Value::Object(m) => m.values().for_each(|x| layer_keys(x, out)),
            _ => {}
        }
    }
    let mut keys = Vec::new();
    layer_keys(&t.objects, &mut keys);
    let legend: Vec<crate::art::Swatch> = keys.iter().map(|k| crate::art::Swatch { key: k.clone(), label: cx.theme.layer_label(k), color: cx.theme.layer(k).to_string() }).collect();
    let mut v = t.objects.clone();
    substitute(&mut v, &st, cx.theme);
    let mut objs: Vec<Object> = serde_json::from_value(v).unwrap_or_default();
    let mut drawn = false;
    let label = cx.vars.resolve(&n.label);
    // the label goes in once the drawing has its final size, so it can break to fit its part
    fn fill_label(o: &mut Object, sx: f64, label: &str, cx: &Ctx, drawn: &mut bool) {
        let room = part_room(o, sx);
        match &mut o.kind {
            Kind::Text(sp) | Kind::Shape(sp) => {
                let mut math = false;
                if let Some(tb) = &mut sp.text {
                    let room = if tb.vert.is_some() { None } else { room };
                    let mut pi = 0;
                    while pi < tb.paras.len() {
                        if let Some(i) = tb.paras[pi].runs.iter().position(|r| r.text.trim() == "{{label}}") {
                            let l = &tb.paras[pi].runs[i].look;
                            let ty = Type { font: l.font.clone(), ea_font: l.ea_font.clone(), math_font: cx.theme.font.math.clone(), size: l.size, bold: l.bold, italic: l.italic, color: l.color.clone() };
                            let lines: Vec<Vec<lore_canvas::Run>> = word_lines(label, ty.size, room)
                                .iter()
                                .map(|line| {
                                    let (rs, m) = text::runs(line, &ty, cx.vars, cx.math);
                                    math |= m;
                                    rs
                                })
                                .collect();
                            let (np, _) = splice_lines(&mut tb.paras, pi, i, lines);
                            pi = np;
                            *drawn = true;
                        }
                        pi += 1;
                    }
                }
                sp.equation |= math;
            }
            Kind::Group(g) => {
                let k = group_scale(o.frame.w, g.ch.w, sx);
                g.children.iter_mut().for_each(|c| fill_label(c, k, label, cx, drawn))
            }
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
        restyle(o, cx.theme);
        scale_obj(o, cx.k);
    }
    // a size given to the block: the drawing stretched to it (its words and line weights stay)
    let (mut w, mut h) = (cx.s(t.w), cx.s(t.h));
    let (gx, gy) = match (n.num("w"), n.num("h"), n.num("size")) {
        (Some(a), Some(b), _) => (a / t.w, b / t.h),
        (Some(a), None, _) => (a / t.w, a / t.w),
        (None, Some(b), _) => (b / t.h, b / t.h),
        (None, None, Some(z)) => (z / t.w.max(t.h), z / t.w.max(t.h)),
        _ => (1.0, 1.0),
    };
    if (gx - 1.0).abs() > 1e-6 || (gy - 1.0).abs() > 1e-6 {
        let (gx, gy) = (gx.clamp(0.2, 8.0), gy.clamp(0.2, 8.0));
        for o in objs.iter_mut() {
            grow_obj_xy(o, gx, gy);
        }
        w *= gx;
        h *= gy;
    }
    join_arrowheads(&mut objs);
    // the block's own words in place of the drawing's fixed ones
    let texts: Vec<String> = n.list("texts");
    let named: Vec<(String, String)> = n.params.iter().filter_map(|(k, v)| k.strip_prefix("text.").map(|name| (name.to_string(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))).collect();
    if !texts.is_empty() || !named.is_empty() {
        let mut seen = 0usize;
        for o in objs.iter_mut() {
            replace_fixed(o, 1.0, &texts, &named, &mut seen, &[], cx);
        }
        if texts.len() > seen {
            warn.push(format!("{}: 這個積木只有 {} 處文字", n.id, seen));
        }
    }
    for o in objs.iter_mut() {
        fill_label(o, 1.0, &label, cx, &mut drawn);
    }
    // where lines connect: what the drawing really draws, when the box keeps room beside it (words
    // under a picture, empty margin); a drawing with its own port keeps the port the author set
    let art = if t.port.is_some() {
        None
    } else {
        drawn_bounds(&objs).and_then(|(x0, y0, x1, y1)| {
            let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(w), y1.min(h));
            let spare = x0 > ART_SLACK || y0 > ART_SLACK || w - x1 > ART_SLACK || h - y1 > ART_SLACK;
            (x1 - x0 > 0.0 && y1 - y0 > 0.0 && spare).then_some((x0, y0, x1 - x0, y1 - y0))
        })
    };
    Core { w, h, objs, single: false, ellipse: false, label: if drawn { LabelPos::Drawn } else { LabelPos::Below }, legend, art, ..Default::default() }
}

/// An arrow drawn as two parts, a line and a small triangle at one of its ends, becomes one
/// object: the line with an arrowhead of its own, the triangle gone. Two parts drift apart (a
/// triangle a point beside the line's end, more when the drawing is enlarged) and are two things
/// to move in an editor. Only open straight-run lines and triangles no larger than
/// [`HEAD_MAX`] pt that hold the line's end count.
fn join_arrowheads(objs: &mut Vec<Object>) {
    let pt = |v: Emu| v as f64 / EMU_PT;
    // the triangles: (index, its box)
    let is_head = |o: &Object| -> bool {
        let Kind::Shape(sp) = &o.kind else { return false };
        if sp.text.is_some() || pt(o.frame.w).max(pt(o.frame.h)) > HEAD_MAX {
            return false;
        }
        match sp.geom.prst.as_str() {
            "triangle" => true,
            "custom" => {
                let path = sp.geom.path.clone().unwrap_or_default();
                path.trim_end().ends_with(['Z', 'z']) && !path.contains(['C', 'Q']) && path.matches('M').count() == 1 && lore_canvas::geom::path_vertices(&path).len() == 3
            }
            _ => false,
        }
    };
    let heads: Vec<usize> = (0..objs.len()).filter(|i| is_head(&objs[*i])).collect();
    if heads.is_empty() {
        return;
    }
    let mut gone: Vec<usize> = Vec::new();
    for li in 0..objs.len() {
        let f = objs[li].frame.clone();
        let Kind::Shape(sp) = &objs[li].kind else { continue };
        let (Some(path), Some(_)) = (sp.geom.path.clone().filter(|_| sp.geom.prst == "custom"), sp.line.as_ref()) else { continue };
        if path.trim_end().ends_with(['Z', 'z']) || path.contains(['C', 'Q']) || path.matches('M').count() != 1 {
            continue;
        }
        let v = lore_canvas::geom::path_vertices(&path);
        if v.len() < 2 {
            continue;
        }
        let at = |(u, w): (f64, f64)| (pt(f.x) + if f.flip_h { 1.0 - u } else { u } * pt(f.w), pt(f.y) + if f.flip_v { 1.0 - w } else { w } * pt(f.h));
        let ends = [at(v[0]), at(v[v.len() - 1])];
        for (k, e) in ends.iter().enumerate() {
            let hit = heads.iter().copied().find(|h| {
                let hf = objs[*h].frame.clone();
                !gone.contains(h) && e.0 >= pt(hf.x) - 1.5 && e.0 <= pt(hf.x + hf.w) + 1.5 && e.1 >= pt(hf.y) - 1.5 && e.1 <= pt(hf.y + hf.h) + 1.5
            });
            if let Some(h) = hit {
                gone.push(h);
                if let Kind::Shape(sp) = &mut objs[li].kind {
                    let st = sp.line.as_mut().unwrap();
                    if k == 0 { st.head = "triangle".into() } else { st.tail = "triangle".into() }
                }
            }
        }
    }
    let mut i = 0;
    objs.retain(|_| {
        i += 1;
        !gone.contains(&(i - 1))
    });
}

/// The largest side (pt) of a triangle that is taken for a line's arrowhead.
const HEAD_MAX: f64 = 10.0;

/// Room (pt) a saved drawing's box may keep beside what it draws before lines connect at the drawing.
const ART_SLACK: f64 = 1.0;

/// How many pt one unit of a group's children is: `sx` (the group's own) times its frame over its
/// child space.
fn group_scale(frame_w: Emu, ch_w: Emu, sx: f64) -> f64 {
    if ch_w > 0 && frame_w > 0 { sx * frame_w as f64 / ch_w as f64 } else { sx }
}

/// The width (pt) words have inside a part: its frame without its side insets.
fn part_room(o: &Object, sx: f64) -> Option<f64> {
    let tb = o.kind.sp()?.text.as_ref()?;
    let w = (o.frame.w - tb.insets[0] - tb.insets[2]) as f64 / EMU_PT * sx;
    (w > 0.0).then_some(w)
}

/// The box (x0, y0, x1, y1, pt) of the parts a drawing shows: a fill, an outline, a line, a
/// picture; not words alone, not hidden parts, not a `port`. `None` = nothing is drawn.
fn drawn_bounds(objs: &[Object]) -> Option<(f64, f64, f64, f64)> {
    // child → parent: parent = a * child + b, across and down
    #[derive(Clone, Copy)]
    struct Map {
        ax: f64,
        bx: f64,
        ay: f64,
        by: f64,
    }
    fn walk(o: &Object, m: Map, b: &mut Option<(f64, f64, f64, f64)>) {
        if o.hidden || o.name == "port" || o.name.ends_with("/port") {
            return;
        }
        let f = &o.frame;
        let (mut x, mut y, mut w, mut h) = (f.x as f64, f.y as f64, f.w as f64, f.h as f64);
        // a quarter turn swaps the box's sides about its centre
        let turn = f.rot.rem_euclid(180.0);
        if (turn - 90.0).abs() < 1.0 {
            (x, y, w, h) = (x + w / 2.0 - h / 2.0, y + h / 2.0 - w / 2.0, h, w);
        }
        let seen = |p: &Paint| match p {
            Paint::None => false,
            Paint::Solid { alpha, .. } => *alpha > 0.0,
            _ => true,
        };
        let stroked = |l: &Option<Stroke>| l.as_ref().is_some_and(|l| l.width_pt > 0.0 && l.alpha > 0.0 && !l.color.is_empty() && l.color != "none");
        let drawn = match &o.kind {
            Kind::Text(sp) | Kind::Shape(sp) => seen(&sp.fill) || stroked(&sp.line),
            Kind::Connector(c) => stroked(&c.line),
            Kind::Picture(_) | Kind::Table(_) | Kind::Locked(_) => true,
            Kind::Group(g) => {
                let (sx, sy) = (if g.ch.w > 0 { w / g.ch.w as f64 } else { 1.0 }, if g.ch.h > 0 { h / g.ch.h as f64 } else { 1.0 });
                let inner = Map { ax: m.ax * sx, bx: m.ax * (x - g.ch.x as f64 * sx) + m.bx, ay: m.ay * sy, by: m.ay * (y - g.ch.y as f64 * sy) + m.by };
                g.children.iter().for_each(|c| walk(c, inner, b));
                false
            }
        };
        if drawn {
            let (x0, y0) = (m.ax * x + m.bx, m.ay * y + m.by);
            let (x1, y1) = (m.ax * (x + w) + m.bx, m.ay * (y + h) + m.by);
            *b = Some(match *b {
                None => (x0, y0, x1, y1),
                Some((a, c, d, e)) => (a.min(x0), c.min(y0), d.max(x1), e.max(y1)),
            });
        }
    }
    let mut b = None;
    let pt = Map { ax: 1.0 / EMU_PT, bx: 0.0, ay: 1.0 / EMU_PT, by: 0.0 };
    for o in objs {
        walk(o, pt, &mut b);
    }
    b
}

/// The lines words are written in inside a part: `\\` breaks the line, and a line wider than
/// `room` (pt) breaks at its spaces (never inside `$…$`) so each line fits; a word wider than the
/// room alone stays whole and overflows as before.
fn word_lines(s: &str, size: f64, room: Option<f64>) -> Vec<String> {
    let mut out = Vec::new();
    for line in text::lines(s) {
        let room = match room {
            Some(r) if text::width(&line, size) > r + 0.5 => r,
            _ => {
                out.push(line);
                continue;
            }
        };
        // words: split at spaces outside math
        let mut words: Vec<String> = Vec::new();
        let (mut cur, mut math, mut prev) = (String::new(), false, ' ');
        for c in line.chars() {
            if c == '$' && prev != '\\' {
                math = !math;
            }
            if c == ' ' && !math {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(c);
            }
            prev = c;
        }
        if !cur.is_empty() {
            words.push(cur);
        }
        let mut row = String::new();
        for w in words {
            if row.is_empty() {
                row = w;
                continue;
            }
            let wider = format!("{row} {w}");
            if text::width(&wider, size) <= room + 0.5 {
                row = wider;
            } else {
                out.push(std::mem::replace(&mut row, w));
            }
        }
        out.push(row);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Put `lines` (runs per line) in place of run `ri` of paragraph `pi`: the first line there, each
/// further line in a paragraph of its own made like `pi`; runs before the placeholder stay in the
/// first line, runs after it follow the last. Returns (paragraph, run) just after what was put in.
fn splice_lines(paras: &mut Vec<lore_canvas::Para>, pi: usize, ri: usize, lines: Vec<Vec<lore_canvas::Run>>) -> (usize, usize) {
    let after: Vec<lore_canvas::Run> = paras[pi].runs.split_off(ri + 1);
    paras[pi].runs.pop();
    let mut like = paras[pi].clone();
    like.runs.clear();
    like.src = None;
    like.k = 0;
    let mut idx = pi;
    for (j, l) in lines.into_iter().enumerate() {
        if j == 0 {
            paras[pi].runs.extend(l);
        } else {
            idx += 1;
            let mut q = like.clone();
            q.runs = l;
            paras.insert(idx, q);
        }
    }
    let end = paras[idx].runs.len();
    paras[idx].runs.extend(after);
    (idx, end)
}

/// The words a block gives for one fixed text of its drawing: `_` keeps the drawing's word.
fn own_words(w: &str) -> Option<&str> {
    let w = w.trim();
    if w == "_" {
        None
    } else if w == "\"\"" || w == "\"" {
        Some("")
    } else {
        Some(w)
    }
}

/// Walk an object (groups depth-first, paragraphs and runs in order): the `seen`-th fixed text
/// (a run that is not `{{label}}` and not blank) takes `texts[seen]`, and the first fixed text
/// of an object (or group) named in `named` takes that name's words.
/// Words given with `\\` or wider than their part break into lines as a label does.
fn replace_fixed(o: &mut Object, sx: f64, texts: &[String], named: &[(String, String)], seen: &mut usize, above: &[String], cx: &Ctx) {
    let mut path: Vec<String> = above.to_vec();
    path.push(o.name.clone());
    let room = part_room(o, sx);
    match &mut o.kind {
        Kind::Text(sp) | Kind::Shape(sp) => {
            let mut first_here = true;
            let mut math = false;
            if let Some(tb) = &mut sp.text {
                let room = if tb.vert.is_some() { None } else { room };
                let mut pi = 0;
                while pi < tb.paras.len() {
                    let mut i = 0;
                    while i < tb.paras[pi].runs.len() {
                        let r = &tb.paras[pi].runs[i];
                        if r.kind != "text" || r.text.trim().is_empty() || r.text.trim() == "{{label}}" {
                            i += 1;
                            continue;
                        }
                        let mut word = texts.get(*seen).and_then(|w| own_words(w)).map(str::to_string);
                        if first_here && let Some((_, w)) = named.iter().find(|(k, _)| path.iter().any(|nm| nm == k)) {
                            word = own_words(w).map(str::to_string);
                        }
                        first_here = false;
                        *seen += 1;
                        let l = r.look.clone();
                        match word {
                            None => i += 1,
                            Some(w) if w.is_empty() => {
                                tb.paras[pi].runs[i].text = String::new();
                                i += 1;
                            }
                            Some(w) => {
                                let ty = Type { font: l.font.clone(), ea_font: l.ea_font.clone(), math_font: cx.theme.font.math.clone(), size: l.size, bold: l.bold, italic: l.italic, color: l.color.clone() };
                                let lines: Vec<Vec<lore_canvas::Run>> = word_lines(&cx.vars.resolve(&w), ty.size, room)
                                    .iter()
                                    .map(|line| {
                                        let (rs, m) = text::runs(line, &ty, cx.vars, cx.math);
                                        math |= m;
                                        rs
                                    })
                                    .collect();
                                let (np, nr) = splice_lines(&mut tb.paras, pi, i, lines);
                                // go on after the words put in (the run they replaced is gone)
                                pi = np;
                                i = nr;
                            }
                        }
                    }
                    pi += 1;
                }
            }
            sp.equation |= math;
        }
        Kind::Group(g) => {
            let k = group_scale(o.frame.w, g.ch.w, sx);
            for c in &mut g.children {
                replace_fixed(c, k, texts, named, seen, &path, cx);
            }
        }
        _ => {}
    }
}

struct Own {
    /// The text colour the block names, as a hex.
    text: Option<String>,
    /// `fill=` / `line=` / `text=` words that are no colour.
    warn: Vec<String>,
}

/// A colour param as written (name or hex) -> (fill, line) of its name, or itself.
fn own_pair(n: &Node, k: &str, theme: &Theme) -> Option<(String, String)> {
    let v = n.str(k)?.trim();
    if v == "none" { Some(("none".into(), "none".into())) } else { theme.tint(v) }
}

/// Turn the `fill=` / `line=` / `text=` params of a block into hex colours, once, where the
/// role's colours are resolved: `fill=blue` alone is the pale blue fill and the strong blue
/// line, `line=blue` only the line, a hex only what it names. The generators and the saved
/// drawings (`role:fill` …) then read the block's own colour first. A multi-colour kind
/// (a layer legend, `classes=`) keeps its own per-class colours.
fn own_colours(n: &mut Node, theme: &Theme) -> Own {
    // a word that is no colour (a misspelt name) is said, not silently dropped
    let mut warn = Vec::new();
    for k in ["fill", "line", "text"] {
        if let Some(v) = n.str(k).map(str::trim)
            && !v.is_empty()
            && v != "none"
            && !matches!(v, "true" | "false" | "yes" | "no" | "on" | "off")
            && theme.tint(v).is_none()
        {
            warn.push(format!("{}: 不認得的顏色 {}", n.id, v));
        }
    }
    let by_name = |v: Option<&str>| v.is_some_and(|v| {
        let h = v.trim().trim_start_matches('#');
        !(h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())) && h != "none"
    });
    let fill_named = by_name(n.str("fill"));
    let line_given = n.str("line").is_some();
    if let Some((fill, line)) = own_pair(n, "fill", theme) {
        n.params.insert("fill".into(), fill.into());
        if fill_named && !line_given {
            n.params.insert("line".into(), line.into());
        }
    }
    if let Some((_, line)) = own_pair(n, "line", theme) {
        n.params.insert("line".into(), line.into());
    }
    let text = own_pair(n, "text", theme).map(|(_, c)| c).filter(|c| c != "none");
    Own { text, warn }
}

/// Every node parameter some kind reads (the generators here and in `art`, `finish`, `chart`,
/// the layout's `col` / `lane`, a saved drawing's `texts` / `text.<name>`), for
/// `notation::check_graph`'s `unknown_param`. Names starting with `_` are the crate's own.
/// A new parameter read anywhere belongs here, else it is reported as unknown.
pub const PARAMS: &[&str] = &[
    // size, colour, finish (every kind)
    "w", "h", "size", "grow", "fill", "line", "text", "line_w", "radius", "finish", "alpha", "bold", "small",
    // layout
    "col", "lane",
    // box, op, loss, image, icon, chart
    "shape", "vertical", "look", "src", "srcs", "icon", "chart", "axes", "mu", "sigma", "values", "ellipse",
    // stacks, bars, grids
    "n", "offset", "cell", "dir", "rows", "cols", "highlight",
    // networks
    "layers", "colors", "through", "filled", "bar_w", "bar_h", "gap", "taper", "solid", "dims", "classes", "fills", "labels", "depth", "latent", "names", "sizes", "inputs", "hidden",
    // brace
    "thick", "ends", "heads",
    // saved drawings
    "texts",
];

/// Whether some kind reads node parameter `k` ([`PARAMS`], `text.<name>`, the crate's `_…`).
pub fn known_param(k: &str) -> bool {
    k.starts_with('_') || k.starts_with("text.") || PARAMS.contains(&k)
}

/// A block that may be drawn larger than its natural size to fill its panel: pictures, charts,
/// icons, networks and saved drawings, not the blocks whose size is their words (box, op, loss,
/// label, brace), and not a block given an exact `w=` / `h=`. A `size=` is the smallest it is
/// drawn, so such a block still grows.
pub fn growable(n: &Node) -> bool {
    !matches!(n.module.as_str(), "box" | "op" | "loss" | "label" | "brace") && n.num("w").is_none() && n.num("h").is_none()
}

/// Scale an object's frame (and a group's children) by `g` about the origin, leaving line
/// weights and text sizes as they are.
pub fn grow_obj(o: &mut Object, g: f64) {
    grow_obj_xy(o, g, g);
}

/// As [`grow_obj`], with its own factor across (`gx`) and down (`gy`).
pub fn grow_obj_xy(o: &mut Object, gx: f64, gy: f64) {
    let sx = |v: Emu| (v as f64 * gx).round() as Emu;
    let sy = |v: Emu| (v as f64 * gy).round() as Emu;
    o.frame.x = sx(o.frame.x);
    o.frame.y = sy(o.frame.y);
    o.frame.w = sx(o.frame.w);
    o.frame.h = sy(o.frame.h);
    if let Kind::Group(gr) = &mut o.kind {
        gr.ch.x = sx(gr.ch.x);
        gr.ch.y = sy(gr.ch.y);
        gr.ch.w = sx(gr.ch.w);
        gr.ch.h = sy(gr.ch.h);
        for c in &mut gr.children {
            grow_obj_xy(c, gx, gy);
        }
    }
}

/// Every plain run of `o` still in the canvas's default font (`unset`) set in `family`.
fn own_font(o: &mut Object, unset: &str, family: &str) {
    match &mut o.kind {
        Kind::Group(g) => g.children.iter_mut().for_each(|c| own_font(c, unset, family)),
        Kind::Text(sp) | Kind::Shape(sp) => {
            if let Some(tb) = &mut sp.text {
                for r in tb.paras.iter_mut().flat_map(|p| p.runs.iter_mut()) {
                    if r.kind != "math" && r.look.font == unset {
                        r.look.font = family.to_string();
                    }
                }
            }
        }
        _ => {}
    }
}

/// A label of one line that is much wider than its picture, on as few lines as keep it within
/// the wider of the picture and [`LABEL_WIDE`] text sizes, the lines about equally long. A label
/// the author broke (`\\\\`) is kept.
pub fn label_lines(s: &str, size: f64, picture_w: f64) -> String {
    let room = picture_w.max(LABEL_WIDE * size);
    let w = text::width(s, size);
    if text::lines(s).len() != 1 || w <= room + 0.5 {
        return s.to_string();
    }
    let greedy = word_lines(s, size, Some(room));
    let n = greedy.len();
    // the narrowest room that still gives that many lines: lines about equally long
    let mut best = greedy;
    let mut r = w / n as f64;
    while r < room {
        let v = word_lines(s, size, Some(r));
        if v.len() == n {
            best = v;
            break;
        }
        r += size * 0.5;
    }
    best.join("\\\\")
}

/// The widest a label under or over a picture is written on one line, in text sizes (a wider
/// picture allows a wider label).
pub const LABEL_WIDE: f64 = 10.0;

/// The height of a drawing's middle line: the middle of its large pieces (every piece at least
/// half as large as the largest), so an attachment (a loop arrow, a badge, a small mark) on one
/// side does not move the line its neighbours are aligned on. The picture's own middle when the
/// pieces say nothing.
fn main_middle(objs: &[Object], art: (f64, f64, f64, f64)) -> f64 {
    let own = art.1 + art.3 / 2.0;
    let boxes: Vec<(f64, f64, f64)> = objs
        .iter()
        .filter(|o| !matches!(o.kind, Kind::Text(_)))
        .map(|o| (o.frame.y as f64 / EMU_PT, o.frame.h as f64 / EMU_PT, o.frame.w as f64 / EMU_PT * o.frame.h as f64 / EMU_PT))
        .collect();
    let largest = boxes.iter().map(|b| b.2).fold(0.0, f64::max);
    if largest <= 0.0 {
        return own;
    }
    let big: Vec<&(f64, f64, f64)> = boxes.iter().filter(|b| b.2 >= largest * 0.5).collect();
    let (lo, hi) = (big.iter().map(|b| b.0).fold(f64::MAX, f64::min), big.iter().map(|b| b.0 + b.1).fold(f64::MIN, f64::max));
    let mid = (lo + hi) / 2.0;
    // within the picture, and only an attachment counts: a difference under [`MIDDLE_MOVES`] of
    // the picture's height (the steps of a stack, a thick outline) leaves the middle where it is
    if (mid - own).abs() < MIDDLE_MOVES * art.3 || mid < art.1 || mid > art.1 + art.3 { own } else { mid }
}

/// The share of a picture's height its large pieces' middle must lie off its own middle before
/// the middle line moves to them.
const MIDDLE_MOVES: f64 = 0.08;

/// How wide a drawing's own words make it: the width, centred on the drawing, that holds every
/// text of its parts (words wider than their part spill over both sides of it).
fn words_width(objs: &[Object], w: f64) -> f64 {
    fn walk(o: &Object, ox: f64, lo: &mut f64, hi: &mut f64) {
        let x = ox + o.frame.x as f64 / EMU_PT;
        let fw = o.frame.w as f64 / EMU_PT;
        match &o.kind {
            Kind::Text(sp) | Kind::Shape(sp) => {
                let Some(tb) = &sp.text else { return };
                if tb.vert.is_some() {
                    return;
                }
                for p in &tb.paras {
                    let words: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                    let size = p.runs.iter().map(|r| r.look.size).fold(0.0, f64::max);
                    if words.trim().is_empty() || size <= 0.0 {
                        continue;
                    }
                    let tw = text::block(&words, size).0;
                    if tw > fw {
                        let cx = x + fw / 2.0;
                        *lo = lo.min(cx - tw / 2.0);
                        *hi = hi.max(cx + tw / 2.0);
                    }
                }
            }
            // a group's children are in its own frame here (parts are placed unscaled)
            Kind::Group(g) => g.children.iter().for_each(|c| walk(c, x - g.ch.x as f64 / EMU_PT, lo, hi)),
            _ => {}
        }
    }
    let (mut lo, mut hi) = (0.0f64, w);
    for o in objs {
        walk(o, 0.0, &mut lo, &mut hi);
    }
    2.0 * (w / 2.0 - lo).max(hi - w / 2.0)
}

pub fn make(node: &Node, cx: &Ctx) -> Body {
    // a node that only names its variable shows the variable as its label
    let mut n = node.clone();
    if n.label.is_empty() && !n.var.is_empty() {
        n.label = format!("@{}", std::mem::take(&mut n.var));
    }
    // a label that is the variable itself says it once (a model wrote both, and it was drawn twice)
    if !n.var.is_empty() && n.label.trim() == format!("@{}", n.var) {
        n.var.clear();
    }
    // the block's own colours (a name or a hex) replace the theme's, for every kind
    let own = own_colours(&mut n, cx.theme);
    let themed;
    let own_cx;
    let cx = if let Some(text) = own.text {
        let mut t = cx.theme.clone();
        t.modules.entry(n.module.clone()).or_default().text = Some(text);
        themed = t;
        own_cx = Ctx { theme: &themed, vars: cx.vars, math: cx.math, lib: cx.lib, k: cx.k, symmetric: cx.symmetric, image_size: cx.image_size };
        &own_cx
    } else {
        cx
    };
    let n = &n;
    // a saved drawing of this kind and style wins over the built-in drawing
    let which = style_name(n, cx.theme);
    let saved = match cx.lib.templates.get(&n.module) {
        Some(styles) if !which.is_empty() => styles.get(&which),
        Some(_) if !crate::spec::MODULES.iter().any(|(m, _)| *m == n.module) => cx.lib.get(&n.module, ""),
        _ => None,
    };
    let mut warn: Vec<String> = own.warn;
    // `size` of a picture-like block (a growable kind, a saved drawing) is a length in pt; a small
    // number was meant as a factor
    let picture_like = saved.is_some() || !matches!(n.module.as_str(), "box" | "op" | "loss" | "label" | "brace");
    if picture_like && n.num("size").is_some_and(|s| s < 8.0) {
        warn.push(format!("{}: size 是點數（pt），不是倍數", n.id));
    }
    let core = match (saved, n.module.as_str()) {
        (Some(t), _) => gen_template(n, t, cx, &mut warn),
        (None, "box") => gen_box(n, cx),
        (None, "op") => gen_op(n, cx),
        (None, "loss") => gen_loss(n, cx),
        (None, "label") => gen_label(n, cx),
        (None, "image") => gen_image(n, cx),
        (None, "chart") => gen_chart(n, cx),
        (None, "icon") => gen_icon(n, cx),
        (None, "image-stack") => gen_image_stack(n, cx),
        (None, "feature-stack") => gen_feature_stack(n, cx),
        (None, "vector-bar") => gen_vector_bar(n, cx),
        (None, "matrix") => gen_matrix(n, cx),
        (None, "layer-bars") => gen_layer_bars(n, cx, &mut warn),
        (None, "cuboid-seq") => gen_cuboid_seq(n, cx),
        (None, "unet") => gen_unet(n, cx),
        (None, "encoder") => crate::art::coder(n, cx, false),
        (None, "decoder") => crate::art::coder(n, cx, true),
        (None, "autoencoder") => crate::art::pair(n, cx),
        (None, "nn") => crate::art::nn(n, cx),
        (None, "brace") => gen_brace(n, cx),
        // a library kind whose named style is missing falls back to its default drawing
        (None, other) => match cx.lib.get(other, "") {
            Some(t) => gen_template(n, t, cx, &mut warn),
            None => gen_box(n, cx),
        },
    };
    // a picture-like block grown to fill its panel (`grow=`, set by `crate::draw`): its shapes
    // are larger, its line weights and its words stay as the style has them
    let mut core = core;
    if let Some(gr) = n.num("grow").filter(|g| *g > 1.0 && growable(n)) {
        core.w *= gr;
        core.h *= gr;
        for o in &mut core.objs {
            grow_obj(o, gr);
        }
        if let Some((x, y, w, h)) = core.art {
            core.art = Some((x * gr, y * gr, w * gr, h * gr));
        }
    }
    // words a drawing carries itself (a saved drawing's own texts, a legend's rows) were written
    // without a font: they take the theme's, like every other word of the figure
    for o in &mut core.objs {
        own_font(o, &lore_canvas::Look::default().font, &cx.theme.font.family);
    }
    // text around the body: the label (above or below) and, below, the variable and caption
    let ink = cx.theme.colors.ink.clone();
    let ty = cx.ty(1.0, false, &ink);
    let small = cx.small(&cx.theme.colors.muted);
    let mut above: Vec<(String, &'static str, Type)> = Vec::new();
    let mut below: Vec<(String, &'static str, Type)> = Vec::new();
    match core.label {
        LabelPos::Above if !n.label.is_empty() => above.push((label_lines(&cx.vars.resolve(&n.label), ty.size, core.w), "label", ty.clone())),
        LabelPos::Below if !n.label.is_empty() => below.push((label_lines(&cx.vars.resolve(&n.label), ty.size, core.w), "label", ty.clone())),
        _ => {}
    }
    if !n.var.is_empty() {
        below.push((format!("@{}", n.var), "var", ty.clone()));
    }
    if !n.caption.is_empty() {
        below.push((n.caption.clone(), "caption", small));
    }
    // `label_at=above`: the words written under the picture are written above it, in their
    // order, so a line can leave the picture's bottom without crossing them
    if n.str("label_at") == Some("above") {
        above.append(&mut below);
    }
    // words of the drawing itself that are wider than it: the room it needs is wider too, so a
    // region's frame and the next column keep clear of them
    let spill = words_width(&core.objs, core.w);
    if above.is_empty() && below.is_empty() && core.single {
        let (w, h) = (core.w, core.h);
        return Body { w, h, cy: h / 2.0, reserve_w: w.max(spill), art: (0.0, 0.0, w, h), words: (false, false), below: 0.0, above: 0.0, axis: h / 2.0, objs: core.objs, single: true, ellipse: core.ellipse, legend: core.legend, warn: { warn.extend(icon_problem(n)); warn } };
    }
    let legend = core.legend;
    warn.extend(icon_problem(n));
    let pad = cx.s(2.0);
    let measure = |v: &[(String, &str, Type)]| -> f64 { v.iter().map(|(s, _, t)| text::block(&cx.vars.resolve(s), t.size).1 as f64 * t.size * 1.25).sum::<f64>() + if v.is_empty() { 0.0 } else { pad } };
    let (top, bottom) = (measure(&above), measure(&below));
    // the words below start `label_drop` lower, so the labels of a row stand on one line
    // under its tallest picture (set by `crate::pack`)
    let drop = if below.is_empty() { 0.0 } else { n.num("label_drop").unwrap_or(0.0).max(0.0) };
    // and the words above start `label_rise` higher, so they stand on one line over the row's
    // tallest picture
    let rise = if above.is_empty() { 0.0 } else { n.num("label_rise").unwrap_or(0.0).max(0.0) };
    let strip = if cx.symmetric { (top + rise).max(bottom + drop) } else { top + rise };
    let mut objs = Vec::new();
    let (w, h) = (core.w, core.h + strip + if cx.symmetric { strip } else { bottom + drop });
    let mid = main_middle(&core.objs, core.art.unwrap_or((0.0, 0.0, core.w, core.h)));
    // the port: the drawn picture, without the words above and below it, centred on the body's
    // middle line so lines on the left and right meet the picture's middle
    let (ax, ay, aw, ah) = core.art.unwrap_or((0.0, 0.0, core.w, core.h));
    let art = (ax, strip + ay, aw, ah);
    // an attachment on one side (a loop arrow) moves the picture's middle off its body's: the
    // port is then the part of the picture that lies evenly about the body's middle line, so
    // lines enter on that line and the connection sites are where they end
    let half = (mid - core.art.map(|a| a.1).unwrap_or(0.0)).min(core.art.map(|a| a.1 + a.3).unwrap_or(core.h) - mid);
    let port = if ((strip + mid) - (art.1 + art.3 / 2.0)).abs() > 0.01 && half > 0.0 { (art.0, strip + mid - half, art.2, 2.0 * half) } else { art };
    objs.push(shape("port", "rect", port.0, port.1, port.2, port.3, Paint::None, None));
    for mut o in core.objs {
        o.frame.y += emu(strip);
        if o.name.is_empty() {
            o.name = "body".into();
        }
        objs.push(o);
    }
    let mut reserve_w = w.max(spill);
    let mut y = strip - rise - pad;
    for (s, name, t) in above.iter().rev() {
        let lines = text::block(&cx.vars.resolve(s), t.size).1 as f64;
        y -= lines * t.size * 1.25;
        let (o, tw, _) = cx.text(name, s, t, w / 2.0, y);
        reserve_w = reserve_w.max(tw);
        objs.push(o);
    }
    let mut y = strip + core.h + pad + drop;
    for (s, name, t) in &below {
        let (o, tw, th) = cx.text(name, s, t, w / 2.0, y);
        y += th;
        reserve_w = reserve_w.max(tw);
        objs.push(o);
    }
    Body { w, h, cy: strip + mid, reserve_w, art, words: (!above.is_empty(), !below.is_empty()), below: bottom, above: top, axis: strip + mid, objs, single: false, ellipse: false, legend, warn }
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
    let mut group = obj(name, fr, Kind::Group(GroupObj { ch, children, lock_aspect: false, data: None }));
    group.id = gid;
    (group, id)
}

#[cfg(test)]
mod align_tests {
    use super::*;

    #[test]
    fn a_line_and_its_triangle_become_one_arrow() {
        // a loop under a body: down, across, up, with a triangle at its last end, a point beside it
        let mut objs = vec![
            shape("body", "rect", 0.0, 0.0, 80.0, 60.0, Paint::None, stroke("000000", 1.0)),
            poly("loop", &[(10.0, 62.0), (10.0, 72.0), (66.0, 72.0), (66.0, 62.0)], false, Paint::None, stroke("000000", 1.0)),
            poly("head", &[(65.0, 62.0), (68.0, 68.0), (62.0, 68.0)], true, paint("000000", 1.0), None),
            // a triangle that is a picture of its own (large) stays
            poly("roof", &[(20.0, 10.0), (60.0, 50.0), (0.0, 50.0)], true, Paint::None, stroke("000000", 1.0)),
        ];
        join_arrowheads(&mut objs);
        let names: Vec<&str> = objs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["body", "loop", "roof"], "the head is gone, the rest stays");
        let Kind::Shape(sp) = &objs[1].kind else { panic!() };
        let st = sp.line.as_ref().unwrap();
        assert_eq!((st.head.as_str(), st.tail.as_str()), ("none", "triangle"), "the line ends in its own arrowhead, at the end the triangle was");
        // a triangle away from every line's end stays
        let mut far = vec![
            poly("loop", &[(10.0, 62.0), (10.0, 72.0), (66.0, 72.0)], false, Paint::None, stroke("000000", 1.0)),
            poly("mark", &[(30.0, 10.0), (33.0, 16.0), (27.0, 16.0)], true, paint("000000", 1.0), None),
        ];
        join_arrowheads(&mut far);
        assert_eq!(far.len(), 2);
    }

    #[test]
    fn an_attachment_does_not_move_the_middle_line() {
        let body = shape("body", "rect", 0.0, 0.0, 80.0, 60.0, Paint::None, stroke("000000", 1.0));
        let under = poly("loop", &[(10.0, 62.0), (10.0, 84.0), (66.0, 84.0), (66.0, 62.0)], false, Paint::None, stroke("000000", 1.0));
        let over = poly("loop", &[(10.0, 22.0), (10.0, 0.0), (66.0, 0.0), (66.0, 22.0)], false, Paint::None, stroke("000000", 1.0));
        let low = shape("body", "rect", 0.0, 24.0, 80.0, 60.0, Paint::None, stroke("000000", 1.0));
        // the loop under the body: the middle is the body's, not the whole picture's
        assert!((main_middle(&[body.clone(), under], (0.0, 0.0, 80.0, 84.0)) - 30.0).abs() < 0.01);
        // the loop over it: again the body's
        assert!((main_middle(&[low, over], (0.0, 0.0, 80.0, 84.0)) - 54.0).abs() < 0.01);
        // pieces of about one size (a stack): the picture's own middle
        let back = shape("back", "rect", 6.0, 0.0, 80.0, 60.0, Paint::None, stroke("000000", 1.0));
        let front = shape("front", "rect", 0.0, 6.0, 80.0, 60.0, Paint::None, stroke("000000", 1.0));
        assert!((main_middle(&[back, front], (0.0, 0.0, 86.0, 66.0)) - 33.0).abs() < 0.01);
    }

    #[test]
    fn a_long_label_is_written_on_lines_of_about_one_length() {
        let size = 10.0;
        let long = "Corrupted local neighbor representation";
        let got = label_lines(long, size, 30.0);
        let lines = text::lines(&got);
        assert!(lines.len() >= 2, "{got}");
        assert!(lines.iter().all(|l| text::width(l, size) <= LABEL_WIDE * size + 0.5), "{got}");
        let (short, wide) = (lines.iter().map(|l| text::width(l, size)).fold(f64::MAX, f64::min), lines.iter().map(|l| text::width(l, size)).fold(0.0, f64::max));
        assert!(short > wide * 0.5, "no line is a leftover word: {got}");
        // a short label, a label under a picture as wide as it, and a label the author broke stay
        assert_eq!(label_lines("Graph", size, 30.0), "Graph");
        assert_eq!(label_lines(long, size, 400.0), long);
        assert_eq!(label_lines("Corrupted local\\\\neighbor representation and more words", size, 30.0), "Corrupted local\\\\neighbor representation and more words");
    }
}
