//! K4, the figure format (docs/brain-hub.md §4.2): what the flow tool can draw, and what is
//! still only a picture and a description.
//!
//! An item is an **entity** (`entity = true`: the flow tool draws it now) or a **reference**
//! (`entity = false`: a picture plus a description, waiting to be made). Three sizes
//! ([`level`]): component, module, figure. Items link by `contains` (figure → modules →
//! components); [`parts_of`] reads the links of a whole figure, [`used_by`] the other way.
//!
//! | `what` | where it comes from | drawn by |
//! |---|---|---|
//! | `generator` | a node kind × art style the flow tool builds from parameters ([`from_catalogue`]) | one node of that kind |
//! | `drawing` | a saved drawing, `<flow>/modules/<kind>/<style>.json` ([`Drawings`], [`save_drawing`]) | one node of that kind and style |
//! | `spec` | a whole figure (the flow editor's spec) ([`spec_item`]) | the whole figure |
//! | `reference` | a folder with `desc.md` and a picture ([`ReferenceDirs`]) | not drawn: its picture is served as a file |
//!
//! # Level table
//!
//! Generators have no level of their own; this table (the author may change it) decides:
//!
//! * **component** (one shape): `box`, `op`, `loss`, `label`, `image`, `brace`.
//! * **module** (a composite, one named part of a network): `image-stack`, `feature-stack`,
//!   `vector-bar`, `matrix`, `layer-bars`, `cuboid-seq`, `unet`, `encoder`, `decoder`,
//!   `autoencoder`, `nn`.
//! * **figure**: only specs and references.
//! * A kind that is not built in (it exists only as a saved drawing) is a **module** unless
//!   the drawing's JSON carries `"level"`.
//!
//! The kinds that name a network structure (`unet`, `encoder`, `decoder`, `autoencoder`, `nn`,
//! `layer-bars`, `cuboid-seq`) are tagged with their kind under [`facet::KIND`]; the single
//! shapes are not.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lore_flow::figure::{Catalogue, FlowSpec};
use lore_flow::knowledge::VAR_KINDS;
use lore_flow::modules::{Library, ModuleTemplate, generator_styles};
use lore_flow::spec::{Graph, MODULES, Node, valid_id};
use lore_flow::text::NoMath;
use serde_json::{Map, Value};

use crate::util::write_atomic;
use crate::types::{
    Adapter, FieldDef, Format, FormatDef, Item, KnowError, Problem, Result, Source, facet, format, item_id, level, safe_local, split_id,
};

/// `what` of a figure item.
pub mod what {
    pub const GENERATOR: &str = "generator";
    pub const DRAWING: &str = "drawing";
    pub const SPEC: &str = "spec";
    pub const REFERENCE: &str = "reference";
    pub const ALL: [&str; 4] = [GENERATOR, DRAWING, SPEC, REFERENCE];
}

const LEVELS: [&str; 3] = [level::COMPONENT, level::MODULE, level::FIGURE];

/// Traditional Chinese name of a level.
pub fn level_label(l: &str) -> &'static str {
    match l {
        level::COMPONENT => "元件",
        level::MODULE => "模塊",
        level::FIGURE => "流程圖",
        _ => "",
    }
}

// ── step 1: the format ───────────────────────────────────────────────────────

/// The figure format.
pub struct Figure;

impl Format for Figure {
    fn def(&self) -> FormatDef {
        let mut what_f = FieldDef::new("what", "種類", "choice", true);
        what_f.choices = what::ALL.iter().map(|s| s.to_string()).collect();
        FormatDef {
            id: format::FIGURE.into(),
            label: "圖形".into(),
            fields: vec![
                what_f,
                FieldDef::new("kind", "節點種類", "text", false),
                FieldDef::new("style", "畫法", "text", false),
                FieldDef::new("description", "說明", "markdown", false),
                FieldDef::new("prompt", "用法", "markdown", false),
                FieldDef::new("inputs", "輸入變數", "text", false),
                FieldDef::new("outputs", "輸出變數", "text", false),
                FieldDef::new("connects", "連線", "text", false),
                FieldDef::new("params", "參數", "text", false),
                FieldDef::new("image", "參考圖", "text", false),
            ],
            shape: "figure".into(),
            form: false,
        }
    }

    fn validate(&self, item: &Item) -> Vec<Problem> {
        let mut out = vec![];
        if item.title.trim().is_empty() {
            out.push(Problem { at: item.id.clone(), field: "title".into(), message: "沒有名稱".into() });
        }
        let at = if item.title.is_empty() { item.id.clone() } else { item.title.clone() };
        let mut bad = |field: &str, message: String| out.push(Problem { at: at.clone(), field: field.into(), message });
        let w = item.field("what");
        if w.is_empty() {
            bad("what", "缺「種類」".into());
        } else if !what::ALL.contains(&w) {
            bad("what", format!("「種類」只能是 {}", what::ALL.join("、")));
        }
        if !LEVELS.contains(&item.level.as_str()) {
            bad("level", "階層只能是元件、模塊或流程圖".into());
        }
        if !w.is_empty() && item.entity != (w != what::REFERENCE) {
            bad("entity", if w == what::REFERENCE { "參考圖不是實體".into() } else { "這一種是實體，不能標成待製作".into() });
        }
        if matches!(w, what::GENERATOR | what::DRAWING) && item.field("kind").trim().is_empty() {
            bad("kind", "缺「節點種類」".into());
        }
        for (field, label) in [("inputs", "輸入變數"), ("outputs", "輸出變數")] {
            for line in crate::flowknow::var_lines(item.field(field)) {
                let kind = line.kind.trim();
                if !kind.is_empty() && !VAR_KINDS.contains(&kind) {
                    bad(field, format!("「{label}」的這一行種類不對：{}", line.raw));
                }
            }
        }
        for line in item.field("connects").lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if crate::flowknow::parse_connect(line).is_none() {
                bad("connects", format!("「連線」要寫成「甲 -> 乙」或「甲 -> 乙 : 種類」：{line}"));
            }
        }
        out
    }

    fn line(&self, item: &Item) -> String {
        if item.field("what") == what::REFERENCE {
            return first_line(item.field("description"));
        }
        let (k, s) = (item.field("kind"), item.field("style"));
        let how = match (k.is_empty(), s.is_empty()) {
            (true, _) => String::new(),
            (false, true) => k.to_string(),
            (false, false) => format!("{k}/{s}"),
        };
        let l = level_label(&item.level);
        if how.is_empty() { l.to_string() } else { format!("{l} · {how}") }
    }

    fn prompt(&self, item: &Item) -> String {
        let mut s = format!("{} [{}]", item.title, item.level);
        let d = first_line(item.field("description"));
        if !d.is_empty() {
            s.push_str(": ");
            s.push_str(&d);
        }
        let u = item.field("prompt").trim();
        if !u.is_empty() {
            s.push_str("\nuse: ");
            s.push_str(u);
        }
        for (label, field) in [("in", "inputs"), ("out", "outputs")] {
            let vars: Vec<String> = crate::flowknow::vars_of(item.field(field)).iter().map(|v| format!("{} ({})", v.id, v.kind)).collect();
            if !vars.is_empty() {
                s.push_str(&format!("\n{label}: {}", vars.join(", ")));
            }
        }
        let flow: Vec<&str> = item.field("connects").lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        if !flow.is_empty() {
            s.push_str("\nflow: ");
            s.push_str(&flow.iter().take(12).copied().collect::<Vec<_>>().join(" ; "));
            if flow.len() > 12 {
                s.push_str(" ; …");
            }
        }
        if item.entity && !item.field("kind").is_empty() {
            s.push_str(&format!("\nkind={} style={}", item.field("kind"), item.field("style")));
        }
        s
    }
}

fn first_line(s: &str) -> String {
    s.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

// ── step 2: entities from the flow tool's catalogue ──────────────────────────

/// Readable name (Traditional Chinese) of a built-in node kind.
fn kind_name(kind: &str) -> &str {
    match kind {
        "box" => "方塊",
        "op" => "運算符",
        "loss" => "損失",
        "label" => "文字",
        "image" => "圖片",
        "image-stack" => "圖片堆",
        "feature-stack" => "特徵堆",
        "vector-bar" => "向量條",
        "matrix" => "矩陣",
        "layer-bars" => "層柱",
        "cuboid-seq" => "立方體序列",
        "unet" => "U-Net",
        "encoder" => "編碼器",
        "decoder" => "解碼器",
        "autoencoder" => "自編碼器",
        "nn" => "神經網路",
        "brace" => "括號",
        other => other,
    }
}

/// The level of a built-in generator kind (see the module's level table). `None` = not built in.
pub fn generator_level(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "box" | "op" | "loss" | "label" | "image" | "brace" => level::COMPONENT,
        "image-stack" | "feature-stack" | "vector-bar" | "matrix" | "layer-bars" | "cuboid-seq" | "unet" | "encoder" | "decoder" | "autoencoder" | "nn" => level::MODULE,
        _ => return None,
    })
}

/// Kinds that name a structure: their kind is also a [`facet::KIND`] tag.
const STRUCTURE_KINDS: [&str; 7] = ["unet", "encoder", "decoder", "autoencoder", "nn", "layer-bars", "cuboid-seq"];

fn title_of(kind: &str, style: &str) -> String {
    if style == "default" || style.is_empty() { kind_name(kind).to_string() } else { format!("{} · {style}", kind_name(kind)) }
}

#[allow(clippy::too_many_arguments)]
fn entity_item(category: &str, what_: &str, kind: &str, style: &str, level_: &str, description: &str, params: Value, source: &Source) -> Item {
    let mut it = Item::new(category, format::FIGURE, &title_of(kind, style), source.clone());
    it.id = item_id(category, &format!("{kind}.{style}"));
    it.level = level_.into();
    it.entity = true;
    it.set("what", what_).set("kind", kind).set("style", style).set("description", description).set("params", params);
    if STRUCTURE_KINDS.contains(&kind) {
        it.tag(facet::KIND, kind);
    }
    it
}

fn drawing_item(category: &str, t: &ModuleTemplate, level_: &str, source: &Source) -> Item {
    let style = if t.style.is_empty() { lore_flow::modules::DEFAULT_STYLE } else { &t.style };
    let params = serde_json::to_value(t).unwrap_or(Value::Null);
    entity_item(category, what::DRAWING, &t.kind, style, level_, &t.description, params, source)
}

/// One entity per generator kind × style, then one per saved drawing of the catalogue's
/// library. A kind without art styles has the one style `default`. A saved drawing of the same
/// kind and style replaces the generator (the flow tool does the same: a saved drawing wins).
/// Order: the flow tool's kind list, then drawings by kind and style.
///
/// The catalogue keeps drawings as [`ModuleTemplate`], which has no `level`, so drawings are
/// modules here; [`Drawings`] reads the files and honours `"level"`.
pub fn from_catalogue(cat: &Catalogue, category: &str, source: Source) -> Vec<Item> {
    let mut out: Vec<Item> = vec![];
    for (kind, desc) in MODULES.iter() {
        let styles = generator_styles(kind);
        let styles: Vec<&str> = if styles.is_empty() { vec!["default"] } else { styles.to_vec() };
        for s in styles {
            let l = generator_level(kind).unwrap_or(level::MODULE);
            out.push(entity_item(category, what::GENERATOR, kind, s, l, desc, Value::Object(Map::new()), &source));
        }
    }
    for styles in cat.library.templates.values() {
        for t in styles.values() {
            put_replacing(&mut out, drawing_item(category, t, level::MODULE, &source));
        }
    }
    out
}

fn put_replacing(out: &mut Vec<Item>, it: Item) {
    match out.iter().position(|x| x.id == it.id) {
        Some(i) => out[i] = it,
        None => out.push(it),
    }
}

// ── step 3: thumbnails ───────────────────────────────────────────────────────

fn json_value(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    }
}

/// The SVG of an entity as the flow tool draws it: a generator or a drawing as one node of its
/// kind and style with a short sample label, a spec as the whole figure. A reference is not
/// drawn (its picture is a file the caller serves).
pub fn thumbnail(item: &Item, cat: &Catalogue) -> Result<String> {
    let w = item.field("what");
    if w == what::REFERENCE {
        return Err(KnowError::Invalid("參考圖沒有可畫的實體".into()));
    }
    let svg = if w == what::SPEC {
        let spec = FlowSpec::from_value(json_value(item.fields.get("params"))).map_err(|e| KnowError::Invalid(format!("流程圖的規格讀不了：{e}")))?;
        let (fig, _) = lore_flow::figure::render(&spec, cat, None, &NoMath, spec.frame_rect(), 2).map_err(|e| KnowError::Invalid(format!("畫不出來：{e}")))?;
        fitted_svg(&fig, 320.0)
    } else {
        let kind = item.field("kind");
        let style = item.field("style");
        let mut lib = cat.library.clone();
        if w == what::DRAWING
            && let Ok(t) = serde_json::from_value::<ModuleTemplate>(json_value(item.fields.get("params")))
        {
            // the item carries its own drawing: draw that, whatever the catalogue holds
            let _ = lib.add(t);
        }
        let params: BTreeMap<String, Value> = match (w, json_value(item.fields.get("params"))) {
            (what::GENERATOR, Value::Object(m)) => m.into_iter().collect(),
            _ => BTreeMap::new(),
        };
        let node = Node {
            id: "n".into(),
            module: kind.into(),
            label: if kind == "op" { "+".into() } else { "Sample".into() },
            style: if style == "default" { String::new() } else { style.into() },
            params,
            ..Default::default()
        };
        let g = Graph { fig: "thumb".into(), nodes: vec![node], ..Default::default() };
        let d = lore_flow::Drawing { theme: cat.theme(""), library: lib, ..Default::default() };
        let fig = lore_flow::draw(&g, &d).map_err(|e| KnowError::Invalid(format!("畫不出來：{e}")))?;
        fitted_svg(&fig, 160.0)
    };
    if !has_shape(&svg) {
        return Err(KnowError::Invalid("畫出來是空白的".into()));
    }
    Ok(svg)
}

/// The figure as an SVG whose viewBox holds every object, plus a small margin.
///
/// `lore_flow::figure::preview_svg` frames the drawing by the layout's own bounds, which leave out
/// what the module draws around itself: a legend above a wide module is centred on it and sticks
/// out at both sides, and a module can be taller than its frame. The frame here is the layout's
/// bounds united with the box of every visible object (a rotated one by its rotated box).
fn fitted_svg(fig: &lore_flow::Figure, width_px: f64) -> String {
    use lore_flow::layout::Rect;
    use lore_flow::modules::{emu, EMU_PT};
    /// Grow the box by an object and, in a group, by its children (they may stick out of the
    /// group's own frame: the `N×` of a decoder sits beside it). `(sx, sy, ox, oy)` maps the
    /// object's own coordinates (EMU) to the figure's (pt).
    fn grow(o: &lore_canvas::Object, xf: (f64, f64, f64, f64), bb: &mut [f64; 4]) {
        if o.hidden {
            return;
        }
        let f = &o.frame;
        let (x, y) = (xf.2 + f.x as f64 / EMU_PT * xf.0, xf.3 + f.y as f64 / EMU_PT * xf.1);
        let (w, h) = (f.w as f64 / EMU_PT * xf.0, f.h as f64 / EMU_PT * xf.1);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (s, c) = f.rot.to_radians().sin_cos();
        let (hw, hh) = ((w * c.abs() + h * s.abs()) / 2.0, (w * s.abs() + h * c.abs()) / 2.0);
        bb[0] = bb[0].min(cx - hw);
        bb[1] = bb[1].min(cy - hh);
        bb[2] = bb[2].max(cx + hw);
        bb[3] = bb[3].max(cy + hh);
        if let lore_canvas::Kind::Group(g) = &o.kind {
            let sx = if g.ch.w == 0 { 1.0 } else { f.w as f64 / g.ch.w as f64 };
            let sy = if g.ch.h == 0 { 1.0 } else { f.h as f64 / g.ch.h as f64 };
            // the same mapping the SVG drawing uses for a group's children
            let inner = (xf.0 * sx, xf.1 * sy, x - g.ch.x as f64 / EMU_PT * xf.0 * sx, y - g.ch.y as f64 / EMU_PT * xf.1 * sy);
            for ch in &g.children {
                grow(ch, inner, bb);
            }
        }
    }
    let mut b = fig.layout.bounds;
    let mut bb = [b.x, b.y, b.x + b.w, b.y + b.h];
    for o in fig.objects() {
        grow(o, (1.0, 1.0, 0.0, 0.0), &mut bb);
    }
    let [x0, y0, x1, y1] = bb;
    b = Rect::new(x0, y0, x1 - x0, y1 - y0).inflate(6.0);
    let mut objs = fig.objects().to_vec();
    let (dx, dy) = (emu(-b.x), emu(-b.y));
    for o in &mut objs {
        o.frame.x += dx;
        o.frame.y += dy;
    }
    let zoom = if b.w > 0.0 { width_px / b.w } else { 1.0 };
    lore_flow::preview::svg(&objs, b.w, b.h, zoom)
}

/// Whether the SVG draws anything besides its white page.
fn has_shape(svg: &str) -> bool {
    let body = match svg.find("</defs>") {
        Some(i) => &svg[i + 7..],
        None => svg,
    };
    let body = body.replacen("<rect width=\"100%\" height=\"100%\" fill=\"#FFFFFF\"/>", "", 1);
    ["<path", "<rect", "<ellipse", "<polygon", "<polyline", "<line", "<circle", "<text", "<image", "<g"].iter().any(|t| body.contains(t))
}

// ── step 4: references ───────────────────────────────────────────────────────

const PICTURES: [&str; 4] = ["ref.png", "ref.jpg", "ref.jpeg", "ref.svg"];

/// One item per sub-folder of `dir` that holds `desc.md` (and perhaps `ref.png` / `ref.jpg` /
/// `ref.svg`): the author's reference pictures, not drawable yet. The folder's name is the local
/// id. `desc.md`: optional first-lines marker `level: component|module|figure` (default
/// figure), a `# heading` (a leading id such as `f01 ` is dropped) and the description.
pub struct ReferenceDirs {
    pub dir: PathBuf,
    pub category: String,
    pub source: Source,
}

/// `(level, title, description)` of a `desc.md`.
fn parse_desc(text: &str, fallback_title: &str) -> (String, String, String) {
    let mut lines: Vec<&str> = text.lines().collect();
    let mut lv = String::new();
    // the marker lives in the first lines, before or after the heading
    let mut i = 0;
    while i < lines.len().min(6) {
        let t = lines[i].trim();
        if let Some(v) = t.strip_prefix("level:").map(str::trim)
            && LEVELS.contains(&v)
        {
            lv = v.to_string();
            lines.remove(i);
            continue;
        }
        i += 1;
    }
    let mut title = String::new();
    if let Some(p) = lines.iter().position(|l| l.starts_with("# ")) {
        title = strip_id(lines[p][2..].trim()).to_string();
        lines.remove(p);
    }
    if title.is_empty() {
        title = fallback_title.to_string();
    }
    (if lv.is_empty() { level::FIGURE.into() } else { lv }, title, lines.join("\n").trim().to_string())
}

/// `f01 Overview` → `Overview` (letters then digits, then a space).
fn strip_id(t: &str) -> &str {
    if let Some((head, rest)) = t.split_once(char::is_whitespace) {
        let letters = head.chars().take_while(|c| c.is_ascii_alphabetic()).count();
        let digits = &head[letters..];
        if letters > 0 && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && !rest.trim().is_empty() {
            return rest.trim();
        }
    }
    t
}

fn picture_in(folder: &Path) -> Option<String> {
    PICTURES.iter().find(|p| folder.join(p).is_file()).map(|p| p.to_string())
}

impl ReferenceDirs {
    fn read(&self, folder: &Path, local: &str) -> Result<Item> {
        let text = std::fs::read_to_string(folder.join("desc.md"))?;
        let (lv, title, desc) = parse_desc(&text, local);
        let mut it = Item::new(&self.category, format::FIGURE, &title, self.source.clone());
        it.id = item_id(&self.category, local);
        it.level = lv;
        it.entity = false;
        it.set("what", what::REFERENCE).set("description", desc).set("image", picture_in(folder).unwrap_or_default());
        Ok(it)
    }

    fn folder_of(&self, id: &str) -> Result<(PathBuf, String)> {
        let (c, l) = split_id(id).ok_or_else(|| KnowError::Invalid(format!("不是這一類的編號：{id}")))?;
        if c != self.category || safe_local(l) != l {
            return Err(KnowError::Invalid(format!("不是這一類的編號：{id}")));
        }
        Ok((self.dir.join(l), l.to_string()))
    }
}

impl Adapter for ReferenceDirs {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        let mut folders: Vec<PathBuf> = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir() && p.join("desc.md").is_file()).collect(),
            Err(_) => return Ok((vec![], vec![])),
        };
        folders.sort();
        let (mut items, mut problems) = (vec![], vec![]);
        for f in folders {
            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if safe_local(&name) != name {
                problems.push(Problem { at: name, field: "id".into(), message: "資料夾名稱只能用英文字母、數字、- _ .".into() });
                continue;
            }
            match self.read(&f, &name) {
                Ok(i) => items.push(i),
                Err(e) => problems.push(Problem { at: name, field: "desc.md".into(), message: format!("讀不了：{e}") }),
            }
        }
        Ok((items, problems))
    }

    /// Writes `desc.md` (the folder is created for a new item). The picture is put there by
    /// the caller. A level other than `figure` is written as the first line.
    fn save(&self, item: &Item) -> Result<()> {
        if item.format != format::FIGURE || item.field("what") != what::REFERENCE || item.entity {
            return Err(KnowError::Invalid("這裡只存參考圖".into()));
        }
        if !LEVELS.contains(&item.level.as_str()) {
            return Err(KnowError::Invalid("階層只能是元件、模塊或流程圖".into()));
        }
        if item.title.trim().is_empty() {
            return Err(KnowError::Invalid("沒有名稱".into()));
        }
        let (folder, _) = self.folder_of(&item.id)?;
        let mut s = String::new();
        if item.level != level::FIGURE {
            s.push_str(&format!("level: {}\n\n", item.level));
        }
        s.push_str(&format!("# {}\n", item.title.trim()));
        let d = item.field("description").trim();
        if !d.is_empty() {
            s.push_str(&format!("\n{d}\n"));
        }
        write_atomic(&folder.join("desc.md"), s.as_bytes())
    }

    /// Deletes the folder's `desc.md` and picture (and the folder when nothing else is left),
    /// and returns the item. The store copies the picture into the trash first
    /// ([`Adapter::picture_of`]) and puts it back on restore ([`Adapter::restore_picture`]).
    fn remove(&self, id: &str) -> Result<Item> {
        let (folder, local) = self.folder_of(id)?;
        if !folder.join("desc.md").is_file() {
            return Err(KnowError::NotFound(id.into()));
        }
        let item = self.read(&folder, &local)?;
        std::fs::remove_file(folder.join("desc.md"))?;
        for p in PICTURES {
            let _ = std::fs::remove_file(folder.join(p));
        }
        let _ = std::fs::remove_dir(&folder);
        Ok(item)
    }

    fn picture_of(&self, id: &str) -> Option<PathBuf> {
        let (folder, _) = self.folder_of(id).ok()?;
        picture_in(&folder).map(|p| folder.join(p))
    }

    fn restore_picture(&self, id: &str, name: &str, from: &Path) -> Result<()> {
        let (folder, _) = self.folder_of(id)?;
        let name = if PICTURES.contains(&name) {
            name.to_string()
        } else {
            let ext = from.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            PICTURES.iter().find(|p| p.ends_with(&format!(".{ext}"))).unwrap_or(&PICTURES[0]).to_string()
        };
        std::fs::create_dir_all(&folder)?;
        std::fs::copy(from, folder.join(name))?;
        Ok(())
    }
}

// ── step 5: saving a drawing as a module the flow tool reads ─────────────────

fn read_template(v: &Value) -> Result<ModuleTemplate> {
    let v = match v {
        Value::String(s) => serde_json::from_str(s).map_err(|e| KnowError::Invalid(format!("圖形的 JSON 讀不了：{e}")))?,
        other => other.clone(),
    };
    serde_json::from_value(v).map_err(|e| KnowError::Invalid(format!("不是流程圖工具的圖形：{e}")))
}

/// Write `<flow_dir>/modules/<kind>/<style>.json`, the file [`Library::load_dir`] reads, and
/// return its path relative to `flow_dir`. `template_json` is a [`ModuleTemplate`] (its `kind`
/// and `style` must be absent, empty or equal to the arguments); `level` (`""` = module) is
/// kept in the file as an extra `"level"` key the flow tool ignores. An existing file is kept
/// unless `replace`.
pub fn save_drawing(flow_dir: &Path, kind: &str, style: &str, template_json: &Value, level_: &str, replace: bool) -> Result<PathBuf> {
    if !valid_id(kind) || !valid_id(style) {
        return Err(KnowError::Invalid("節點種類與畫法只能用英文字母、數字、- _".into()));
    }
    let level_ = if level_.is_empty() { level::MODULE } else { level_ };
    if !LEVELS.contains(&level_) {
        return Err(KnowError::Invalid("階層只能是元件、模塊或流程圖".into()));
    }
    let mut t = read_template(template_json)?;
    for (have, given, what_) in [(&t.kind, kind, "節點種類"), (&t.style, style, "畫法")] {
        if !have.is_empty() && have != given {
            return Err(KnowError::Invalid(format!("圖形裡的{what_}（{have}）和要存的（{given}）不同")));
        }
    }
    t.kind = kind.into();
    t.style = style.into();
    // exactly what the flow tool checks when it loads a drawing
    Library::default().add(t.clone()).map_err(KnowError::Invalid)?;
    let rel = PathBuf::from("modules").join(kind).join(format!("{style}.json"));
    let path = flow_dir.join(&rel);
    if path.exists() && !replace {
        return Err(KnowError::Invalid(format!("已經有 {kind}/{style} 這個圖形")));
    }
    let mut v = serde_json::to_value(&t).map_err(|e| KnowError::Other(e.to_string()))?;
    if level_ != level::MODULE
        && let Value::Object(m) = &mut v
    {
        m.insert("level".into(), level_.into());
    }
    let text = serde_json::to_string_pretty(&v).map_err(|e| KnowError::Other(e.to_string()))? + "\n";
    write_atomic(&path, text.as_bytes())?;
    Ok(rel)
}

/// The saved drawings of a flow folder (`<flow_dir>/modules`), as entities of `category`.
pub struct Drawings {
    pub flow_dir: PathBuf,
    pub category: String,
    pub source: Source,
}

struct Found {
    path: PathBuf,
    t: ModuleTemplate,
    level: String,
}

impl Drawings {
    fn scan(&self) -> (Vec<Found>, Vec<Problem>) {
        let root = self.flow_dir.join("modules");
        let (mut found, mut problems) = (vec![], vec![]);
        let json_files = |d: &Path| -> Vec<PathBuf> {
            let mut v: Vec<PathBuf> = std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json")).collect();
            v.sort();
            v
        };
        let mut read = |f: &Path, kind: Option<&str>| {
            let at = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let mut bad = |m: String| problems.push(Problem { at: at.clone(), field: "json".into(), message: m });
            let Ok(text) = std::fs::read_to_string(f) else { return bad("讀不了".into()) };
            let Ok(raw) = serde_json::from_str::<Value>(&text) else { return bad("不是 JSON".into()) };
            let mut t: ModuleTemplate = match serde_json::from_value(raw.clone()) {
                Ok(t) => t,
                Err(e) => return bad(format!("不是流程圖工具的圖形：{e}")),
            };
            if let Some(k) = kind {
                if t.kind != k {
                    return bad(format!("節點種類 {} 和資料夾 {k} 不同", t.kind));
                }
                if t.style.is_empty() {
                    t.style = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                }
            }
            if let Err(e) = Library::default().add(t.clone()) {
                return bad(e);
            }
            if t.style.is_empty() {
                t.style = lore_flow::modules::DEFAULT_STYLE.into();
            }
            let lv = raw.get("level").and_then(Value::as_str).filter(|l| LEVELS.contains(l)).unwrap_or(level::MODULE).to_string();
            found.push(Found { path: f.to_path_buf(), t, level: lv });
        };
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).collect();
        entries.sort();
        for e in entries {
            if e.is_dir() {
                let kind = e.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                for f in json_files(&e) {
                    read(&f, Some(&kind));
                }
            } else if e.extension().is_some_and(|x| x == "json") {
                read(&e, None);
            }
        }
        (found, problems)
    }
}

impl Adapter for Drawings {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        let (found, problems) = self.scan();
        Ok((found.iter().map(|f| drawing_item(&self.category, &f.t, &f.level, &self.source)).collect(), problems))
    }

    /// Add or replace the drawing: `fields.params` is its JSON, `kind` and `style` its names
    /// and the id must be `<category>:<kind>.<style>`.
    fn save(&self, item: &Item) -> Result<()> {
        if item.field("what") != what::DRAWING {
            return Err(KnowError::Invalid("這裡只存圖形的畫法".into()));
        }
        let (kind, style) = (item.field("kind"), item.field("style"));
        if !item.id.is_empty() && split_id(&item.id) != Some((self.category.as_str(), format!("{kind}.{style}").as_str())) {
            return Err(KnowError::Invalid("編號和節點種類、畫法對不上".into()));
        }
        let params = item.fields.get("params").ok_or_else(|| KnowError::Invalid("缺「參數」".into()))?;
        save_drawing(&self.flow_dir, kind, style, params, &item.level, true).map(|_| ())
    }

    fn remove(&self, id: &str) -> Result<Item> {
        let (c, l) = split_id(id).ok_or_else(|| KnowError::NotFound(id.into()))?;
        if c != self.category {
            return Err(KnowError::NotFound(id.into()));
        }
        let (found, _) = self.scan();
        let f = found.iter().find(|f| format!("{}.{}", f.t.kind, f.t.style) == l).ok_or_else(|| KnowError::NotFound(id.into()))?;
        let item = drawing_item(&self.category, &f.t, &f.level, &self.source);
        std::fs::remove_file(&f.path)?;
        if let Some(d) = f.path.parent() {
            let _ = std::fs::remove_dir(d);
        }
        Ok(item)
    }
}

// ── step 6: whole figures and links ──────────────────────────────────────────

/// A whole figure (the flow editor's spec, an object or its JSON text) as a figure-level
/// entity. The id is left empty (the store makes it); fill `contains` from [`parts_of`].
pub fn spec_item(category: &str, title: &str, spec_json: &Value, source: Source) -> Item {
    let mut it = Item::new(category, format::FIGURE, title, source);
    it.level = level::FIGURE.into();
    it.entity = true;
    it.set("what", what::SPEC).set("params", spec_json.clone());
    it
}

/// The ids of the entities (generators and drawings, by kind and style) the nodes of a spec use,
/// in node order, each once. A node with a saved drawing of its kind and style takes the
/// drawing (the flow tool prefers it); a node without a style takes `<kind>.default`, else the
/// kind's first built-in style, else its first saved drawing. A node that no entity covers, or a spec that cannot be read,
/// adds nothing.
pub fn parts_of(spec_json: &Value, entities: &[Item]) -> Vec<String> {
    let Ok(spec) = FlowSpec::from_value(json_value(Some(spec_json))) else { return vec![] };
    let extra: Vec<String> = entities.iter().filter(|e| e.field("what") == what::DRAWING).map(|e| e.field("kind").to_string()).collect();
    let Ok(g) = lore_flow::dsl::parse_with(&spec.dsl, &extra) else { return vec![] };
    let find = |kind: &str, style: &str| -> Option<&Item> {
        let ok = |e: &&Item| matches!(e.field("what"), what::GENERATOR | what::DRAWING) && e.field("kind") == kind && e.field("style") == style;
        entities.iter().filter(ok).find(|e| e.field("what") == what::DRAWING).or_else(|| entities.iter().find(ok))
    };
    let mut out: Vec<String> = vec![];
    for n in &g.nodes {
        let hit = if n.style.is_empty() {
            find(&n.module, "default").or_else(|| generator_styles(&n.module).first().and_then(|s| find(&n.module, s)))
                // a saved kind without a style: the tool takes its `default`, else its first drawing
                .or_else(|| entities.iter().find(|e| e.field("what") == what::DRAWING && e.field("kind") == n.module))
        } else {
            find(&n.module, &n.style)
        };
        if let Some(e) = hit
            && !out.contains(&e.id)
        {
            out.push(e.id.clone());
        }
    }
    out
}

/// The items that contain `id` (who uses it), in the order of `all`.
pub fn used_by<'a>(id: &str, all: &'a [Item]) -> Vec<&'a Item> {
    all.iter().filter(|i| i.contains.iter().any(|c| c == id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    const ENCODER_PLAIN: &str = include_str!("../../../../../private-example/SlideComposer/flow/modules/encoder/plain.json");
    // a kind the flow tool does not build in: `Catalogue::load_all` keeps only those (see the report)
    const DOCUMENT_PAGE: &str = include_str!("../../../../../private-example/SlideComposer/flow/modules/document/page.json");

    fn fig_item(w: &str, lv: &str, entity: bool) -> Item {
        let mut i = Item::new("my.figs", format::FIGURE, "Example", Source::author());
        i.level = lv.into();
        i.entity = entity;
        i.set("what", w);
        if w == what::GENERATOR || w == what::DRAWING {
            i.set("kind", "unet").set("style", "bars");
        }
        i
    }

    fn fields_of(p: &[Problem]) -> Vec<&str> {
        p.iter().map(|p| p.field.as_str()).collect()
    }

    // ── step 1 ──

    #[test]
    fn a_figure_item_needs_a_level_and_a_matching_entity_flag() {
        let f = Figure;
        assert_eq!(f.def().id, format::FIGURE);
        assert_eq!((f.def().label.as_str(), f.def().shape.as_str(), f.def().form), ("圖形", "figure", false));
        assert!(f.validate(&fig_item(what::GENERATOR, level::MODULE, true)).is_empty());
        assert!(f.validate(&fig_item(what::REFERENCE, level::FIGURE, false)).is_empty());
        // no level, a wrong level
        assert_eq!(fields_of(&f.validate(&fig_item(what::GENERATOR, "", true))), ["level"]);
        assert_eq!(fields_of(&f.validate(&fig_item(what::GENERATOR, "big", true))), ["level"]);
        // an entity that claims to be a todo, a reference that claims to be drawable
        assert_eq!(fields_of(&f.validate(&fig_item(what::DRAWING, level::MODULE, false))), ["entity"]);
        assert_eq!(fields_of(&f.validate(&fig_item(what::REFERENCE, level::MODULE, true))), ["entity"]);
        // what: missing and unknown
        let mut no_what = fig_item(what::SPEC, level::FIGURE, true);
        no_what.fields.clear();
        assert!(fields_of(&f.validate(&no_what)).contains(&"what"));
        assert_eq!(fields_of(&f.validate(&fig_item("photo", level::FIGURE, true))), ["what"]);
        // a generator needs its kind
        let mut nk = fig_item(what::GENERATOR, level::MODULE, true);
        nk.fields.remove("kind");
        assert_eq!(fields_of(&f.validate(&nk)), ["kind"]);
    }

    #[test]
    fn lines_and_prompts_read_well() {
        let f = Figure;
        let mut e = fig_item(what::GENERATOR, level::MODULE, true);
        e.title = "U-Net".into();
        e.set("description", "A flat U-shaped encoder-decoder");
        assert_eq!(f.line(&e), "模塊 · unet/bars");
        assert_eq!(f.prompt(&e), "U-Net [module]: A flat U-shaped encoder-decoder\nkind=unet style=bars");
        let mut c = fig_item(what::GENERATOR, level::COMPONENT, true);
        c.set("kind", "box").set("style", "rect");
        assert_eq!(f.line(&c), "元件 · box/rect");
        let mut s = fig_item(what::SPEC, level::FIGURE, true);
        s.title = "Whole method".into();
        assert_eq!(f.line(&s), "流程圖");
        assert_eq!(f.prompt(&s), "Whole method [figure]");
        let mut r = fig_item(what::REFERENCE, level::FIGURE, false);
        r.title = "Timeline".into();
        r.set("description", "\nTwo rows of dated boxes.\nMore.");
        assert_eq!(f.line(&r), "Two rows of dated boxes.");
        assert_eq!(f.prompt(&r), "Timeline [figure]: Two rows of dated boxes.");
    }

    // ── step 2 ──

    #[test]
    fn the_builtin_catalogue_lists_every_generator_style_once() {
        let items = from_catalogue(&Catalogue::builtin(), "pc.figs", Source::lore("sc"));
        let mut expect = 0;
        for (kind, _) in MODULES.iter() {
            let styles = generator_styles(kind);
            let styles: Vec<&str> = if styles.is_empty() { vec!["default"] } else { styles.to_vec() };
            for s in styles {
                expect += 1;
                let id = format!("pc.figs:{kind}.{s}");
                assert_eq!(items.iter().filter(|i| i.id == id).count(), 1, "{id}");
            }
        }
        assert_eq!(items.len(), expect);
        let ids: std::collections::BTreeSet<_> = items.iter().map(|i| &i.id).collect();
        assert_eq!(ids.len(), items.len());
        let fig = Figure;
        for i in &items {
            assert!(fig.validate(i).is_empty(), "{}: {:?}", i.id, fig.validate(i));
            assert_eq!((i.field("what"), i.entity, i.category.as_str(), i.source.name.as_str()), ("generator", true, "pc.figs", "sc"));
        }
        let get = |id: &str| items.iter().find(|i| i.id == id).unwrap();
        assert_eq!(get("pc.figs:box.rect").level, level::COMPONENT);
        assert_eq!(get("pc.figs:unet.bars").level, level::MODULE);
        assert_eq!(get("pc.figs:unet.bars").tags[facet::KIND], ["unet"]);
        assert!(get("pc.figs:box.rect").tags.is_empty());
        assert_eq!(get("pc.figs:label.default").field("style"), "default");
        // deterministic
        assert_eq!(items, from_catalogue(&Catalogue::builtin(), "pc.figs", Source::lore("sc")));
        println!("builtin entities: {}", items.len());
    }

    #[test]
    fn saved_drawings_from_a_folder_become_module_entities() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("modules/document");
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("page.json"), DOCUMENT_PAGE).unwrap();
        // loaded as the flow tool does
        let cat = Catalogue::load_all(&[dir.path()]);
        let items = from_catalogue(&cat, "sc.figs", Source::lore("sc"));
        let it = items.iter().find(|i| i.id == "sc.figs:document.page").expect("the drawing is listed");
        assert_eq!((it.field("what"), it.level.as_str(), it.entity), ("drawing", level::MODULE, true));
        assert!(!it.field("description").is_empty());
        assert_eq!((it.field("kind"), it.field("style")), ("document", "page"));
        assert!(it.tags.is_empty(), "a document is not a network structure");
        assert!(it.fields["params"].get("objects").is_some());
        // the built-in generators are still there, once each
        assert_eq!(items.iter().filter(|i| i.id == "sc.figs:document.page").count(), 1);
        assert!(items.iter().any(|i| i.id == "sc.figs:encoder.trapezoid"));
        assert_eq!(Figure.validate(it), vec![]);
        // the folder adapter sees the same drawing
        let d = Drawings { flow_dir: dir.path().into(), category: "sc.figs".into(), source: Source::lore("sc") };
        let (loaded, problems) = d.load().unwrap();
        assert!(problems.is_empty());
        assert_eq!(loaded.len(), 1);
        assert_eq!(&loaded[0], it);
        // a drawing of a built-in kind (the example's encoder) is read by the adapter and by Library
        let e = dir.path().join("modules/encoder");
        fs::create_dir_all(&e).unwrap();
        fs::write(e.join("plain.json"), ENCODER_PLAIN).unwrap();
        assert!(d.load().unwrap().0.iter().any(|i| i.id == "sc.figs:encoder.plain" && i.tags[facet::KIND] == ["encoder"]));
        assert!(Library::load_dir(&dir.path().join("modules")).unwrap().get("encoder", "plain").is_some());
    }

    // ── step 3 ──

    #[test]
    fn every_builtin_entity_draws_a_non_empty_thumbnail() {
        let cat = Catalogue::builtin();
        for i in from_catalogue(&cat, "c", Source::lore("sc")) {
            let svg = thumbnail(&i, &cat).unwrap_or_else(|e| panic!("{}: {e}", i.id));
            assert!(svg.starts_with("<svg"), "{}", i.id);
            assert!(has_shape(&svg), "{}: no shape", i.id);
            let vb = svg.split("viewBox=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("");
            let n: Vec<f64> = vb.split(' ').filter_map(|x| x.parse().ok()).collect();
            assert!(n.len() == 4 && n[2] > 1.0 && n[3] > 1.0 && n[2] < 5000.0 && n[3] < 5000.0, "{}: viewBox {vb:?}", i.id);
        }
    }

    /// The viewBox of an SVG as `[x, y, w, h]`.
    fn view_box(svg: &str) -> [f64; 4] {
        let vb = svg.split("viewBox=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("");
        let n: Vec<f64> = vb.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        [n[0], n[1], n[2], n[3]]
    }

    fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
        let pat = format!(" {name}=\"");
        let at = tag.find(&pat)? + pat.len();
        tag[at..].split('"').next()
    }

    fn num(tag: &str, name: &str) -> Option<f64> {
        attr(tag, name)?.trim().parse().ok()
    }

    /// Every coordinate pair of a path (absolute and relative M L H V Q C T S A Z).
    fn path_points(d: &str) -> Vec<(f64, f64)> {
        let mut toks: Vec<String> = vec![];
        let mut cur = String::new();
        for c in d.chars() {
            if c.is_ascii_alphabetic() {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
                toks.push(c.to_string());
            } else if c == ',' || c.is_whitespace() {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
            } else if c == '-' && !cur.is_empty() && !cur.ends_with('e') {
                toks.push(std::mem::take(&mut cur));
                cur.push(c);
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            toks.push(cur);
        }
        let (mut x, mut y) = (0.0, 0.0);
        let mut out = vec![];
        let mut i = 0;
        let mut cmd = 'M';
        let n = |t: &String| t.parse::<f64>().unwrap_or(0.0);
        while i < toks.len() {
            if toks[i].chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                cmd = toks[i].chars().next().unwrap();
                i += 1;
                if cmd == 'Z' || cmd == 'z' {
                    continue;
                }
            }
            let rel = cmd.is_ascii_lowercase();
            let take = match cmd.to_ascii_uppercase() {
                'H' | 'V' => 1,
                'M' | 'L' | 'T' => 2,
                'Q' | 'S' => 4,
                'C' => 6,
                'A' => 7,
                _ => 2,
            };
            if i + take > toks.len() {
                break;
            }
            let v: Vec<f64> = toks[i..i + take].iter().map(n).collect();
            i += take;
            match cmd.to_ascii_uppercase() {
                'H' => x = if rel { x + v[0] } else { v[0] },
                'V' => y = if rel { y + v[0] } else { v[0] },
                'A' => {
                    x = if rel { x + v[5] } else { v[5] };
                    y = if rel { y + v[6] } else { v[6] };
                }
                _ => {
                    let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
                    for p in v.chunks(2) {
                        out.push((ox + p[0], oy + p[1]));
                    }
                    x = ox + v[take - 2];
                    y = oy + v[take - 1];
                }
            }
            if matches!(cmd.to_ascii_uppercase(), 'H' | 'V' | 'A') {
                out.push((x, y));
            }
        }
        out
    }

    /// What of an SVG's shapes lies outside its viewBox (rect, circle, ellipse, line, polyline,
    /// polygon, path; text is checked by its anchor point). Shapes inside a transformed group are
    /// not followed. Returns the descriptions of the offenders and how many shapes were checked.
    fn outside_view_box(svg: &str) -> (Vec<String>, usize) {
        let [vx, vy, vw, vh] = view_box(svg);
        let eps = 0.01;
        let mut bad = vec![];
        let mut checked = 0;
        let mut groups: Vec<bool> = vec![]; // per open <g>: transformed?
        let mut rest = svg;
        while let Some(at) = rest.find('<') {
            rest = &rest[at..];
            let end = rest.find('>').unwrap_or(rest.len() - 1);
            let tag = &rest[..=end];
            rest = &rest[end + 1..];
            let name: String = tag[1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '/').collect();
            if name == "g" {
                if !tag.ends_with("/>") {
                    groups.push(tag.contains(" transform=\""));
                }
                continue;
            }
            if name == "/g" {
                groups.pop();
                continue;
            }
            if groups.iter().any(|t| *t) || tag.contains(" transform=\"") {
                continue;
            }
            let mut pts: Vec<(f64, f64)> = vec![];
            match name.as_str() {
                "rect" => {
                    if let (Some(x), Some(y), Some(w), Some(h)) = (num(tag, "x"), num(tag, "y"), num(tag, "width"), num(tag, "height")) {
                        // the page background is `width="100%"`, which does not parse
                        pts = vec![(x, y), (x + w, y + h)];
                    }
                }
                "circle" | "ellipse" => {
                    let (rx, ry) = (num(tag, "rx").or(num(tag, "r")), num(tag, "ry").or(num(tag, "r")));
                    if let (Some(cx), Some(cy), Some(rx), Some(ry)) = (num(tag, "cx"), num(tag, "cy"), rx, ry) {
                        pts = vec![(cx - rx, cy - ry), (cx + rx, cy + ry)];
                    }
                }
                "line" => {
                    if let (Some(a), Some(b), Some(c), Some(d)) = (num(tag, "x1"), num(tag, "y1"), num(tag, "x2"), num(tag, "y2")) {
                        pts = vec![(a, b), (c, d)];
                    }
                }
                "polyline" | "polygon" => {
                    let v: Vec<f64> = attr(tag, "points").unwrap_or("").split(|c: char| c == ',' || c.is_whitespace()).filter_map(|x| x.parse().ok()).collect();
                    pts = v.chunks(2).filter(|p| p.len() == 2).map(|p| (p[0], p[1])).collect();
                }
                "path" => pts = path_points(attr(tag, "d").unwrap_or("")),
                "text" => {
                    if let (Some(x), Some(y)) = (num(tag, "x"), num(tag, "y")) {
                        pts = vec![(x, y)];
                    }
                }
                _ => {}
            }
            if pts.is_empty() {
                continue;
            }
            checked += 1;
            for (x, y) in pts {
                if x < vx - eps || y < vy - eps || x > vx + vw + eps || y > vy + vh + eps {
                    bad.push(format!("<{name}> at ({x:.1}, {y:.1}) outside {vx} {vy} {vw} {vh}"));
                    break;
                }
            }
        }
        (bad, checked)
    }

    #[test]
    fn the_cropping_check_sees_a_shape_outside_the_frame() {
        let svg = "<svg viewBox=\"0 0 100 50\"><rect width=\"100%\" height=\"100%\" fill=\"#FFFFFF\"/><g><rect x=\"10\" y=\"10\" width=\"20\" height=\"20\"/><path d=\"M10 10 Q50 -20 90 10\"/><circle cx=\"95\" cy=\"25\" r=\"10\"/></g></svg>";
        let (bad, checked) = outside_view_box(svg);
        assert_eq!(checked, 3);
        assert_eq!(bad.len(), 2, "{bad:?}");
        assert!(bad[0].starts_with("<path") && bad[1].starts_with("<circle"));
    }

    #[test]
    fn no_builtin_thumbnail_is_cropped() {
        let cat = Catalogue::builtin();
        let mut cropped = vec![];
        let mut shapes = 0;
        for i in from_catalogue(&cat, "c", Source::lore("sc")) {
            let svg = thumbnail(&i, &cat).unwrap_or_else(|e| panic!("{}: {e}", i.id));
            let (bad, n) = outside_view_box(&svg);
            assert!(n > 0, "{}: nothing was checked", i.id);
            shapes += n;
            if !bad.is_empty() {
                cropped.push(format!("{}: {}", i.id, bad[0]));
            }
        }
        // a whole figure with wide modules (their legends stick out) is framed the same way
        let spec = json!({ "lore_flow": 1, "dsl": "fig wide\nx: image \"x\"\ne: encoder \"Encoder\" style=slabs\nd: decoder \"Decoder\" style=transformer\nx -> e -> d\n" });
        let whole = spec_item("c", "Wide", &spec, Source::author());
        let svg = thumbnail(&whole, &cat).unwrap();
        let (bad, n) = outside_view_box(&svg);
        assert!(n > 5 && bad.is_empty(), "{bad:?}");
        assert!(shapes > 500, "only {shapes} shapes were checked");
        assert!(cropped.is_empty(), "cropped thumbnails:\n{}", cropped.join("\n"));
    }

    #[test]
    fn a_saved_drawing_and_a_spec_draw_too() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("modules/document")).unwrap();
        fs::write(dir.path().join("modules/document/page.json"), DOCUMENT_PAGE).unwrap();
        let cat = Catalogue::load_all(&[dir.path()]);
        let items = from_catalogue(&cat, "c", Source::lore("sc"));
        let d = items.iter().find(|i| i.id == "c:document.page").unwrap();
        assert!(thumbnail(d, &cat).unwrap().contains("Sample"));
        // the item carries its drawing: the bare builtin catalogue draws it as well
        assert!(has_shape(&thumbnail(d, &Catalogue::builtin()).unwrap()));
        let spec = json!({ "lore_flow": 1, "dsl": "fig demo\nx: image \"x\"\nf: box \"Model\"\nx -> f\n" });
        let s = spec_item("c", "Tiny", &spec, Source::author());
        let svg = thumbnail(&s, &cat).unwrap();
        assert!(svg.contains("Model") && has_shape(&svg));
        let bad = spec_item("c", "Bad", &json!({ "dsl": "this is not a figure" }), Source::author());
        assert!(matches!(thumbnail(&bad, &cat), Err(KnowError::Invalid(_))));
    }

    #[test]
    fn a_reference_has_no_drawn_thumbnail() {
        let r = fig_item(what::REFERENCE, level::FIGURE, false);
        match thumbnail(&r, &Catalogue::builtin()) {
            Err(KnowError::Invalid(m)) => assert_eq!(m, "參考圖沒有可畫的實體"),
            other => panic!("{other:?}"),
        }
    }

    // ── step 4 ──

    fn refs(dir: &Path) -> ReferenceDirs {
        ReferenceDirs { dir: dir.into(), category: "my.refs".into(), source: Source::author() }
    }

    #[test]
    fn folders_with_a_description_become_references() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("f01");
        fs::create_dir_all(&a).unwrap();
        fs::write(a.join("desc.md"), "# f01 Method overview\n\nThree boxes in a row,\nthen a loss.\n").unwrap();
        fs::write(a.join("ref.png"), b"not really a png").unwrap();
        let b = dir.path().join("t2");
        fs::create_dir_all(&b).unwrap();
        fs::write(b.join("desc.md"), "level: module\n\n# Dated boxes\n\nOne row.\n").unwrap();
        fs::create_dir_all(dir.path().join("empty")).unwrap();
        fs::create_dir_all(dir.path().join("bad name")).unwrap();
        fs::write(dir.path().join("bad name/desc.md"), "# x\n").unwrap();
        let (items, problems) = refs(dir.path()).load().unwrap();
        assert_eq!(items.len(), 2, "{items:?}");
        assert_eq!(problems.len(), 1);
        let (f01, t2) = (&items[0], &items[1]);
        assert_eq!((f01.id.as_str(), f01.title.as_str(), f01.level.as_str()), ("my.refs:f01", "Method overview", level::FIGURE));
        assert_eq!(f01.field("description"), "Three boxes in a row,\nthen a loss.");
        assert_eq!(f01.field("image"), "ref.png");
        assert_eq!((t2.title.as_str(), t2.level.as_str(), t2.field("image")), ("Dated boxes", level::MODULE, ""));
        assert_eq!(t2.field("description"), "One row.");
        for i in &items {
            assert_eq!(Figure.validate(i), vec![]);
        }
    }

    #[test]
    fn a_reference_is_never_an_entity() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("r1")).unwrap();
        fs::write(dir.path().join("r1/desc.md"), "# R\n\nd\n").unwrap();
        let (items, _) = refs(dir.path()).load().unwrap();
        assert!(items.iter().all(|i| !i.entity && i.field("what") == "reference"));
        // saving an entity, or something that is not a figure, is refused
        let r = refs(dir.path());
        let mut e = items[0].clone();
        e.entity = true;
        assert!(r.save(&e).is_err());
        let mut g = items[0].clone();
        g.set("what", "generator");
        assert!(r.save(&g).is_err());
        let mut up = items[0].clone();
        up.id = "my.refs:../x".into();
        assert!(r.save(&up).is_err());
    }

    #[test]
    fn saving_writes_the_description_back() {
        let dir = tempfile::tempdir().unwrap();
        let r = refs(dir.path());
        let mut it = Item::new("my.refs", format::FIGURE, "New timeline", Source::author());
        it.id = "my.refs:tl1".into();
        it.level = level::MODULE.into();
        it.entity = false;
        it.set("what", "reference").set("description", "Row of dated boxes.");
        r.save(&it).unwrap();
        assert!(dir.path().join("tl1/desc.md").is_file());
        assert!(!dir.path().join("tl1/.desc.md.tmp").exists());
        let (items, _) = r.load().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].title.as_str(), items[0].level.as_str(), items[0].field("description")), ("New timeline", level::MODULE, "Row of dated boxes."));
        // change it, with a picture put there by the caller
        fs::write(dir.path().join("tl1/ref.jpg"), b"jpg").unwrap();
        let mut ch = items[0].clone();
        ch.title = "Renamed".into();
        ch.level = level::FIGURE.into();
        r.save(&ch).unwrap();
        let (items, _) = r.load().unwrap();
        assert_eq!((items[0].title.as_str(), items[0].level.as_str(), items[0].field("image")), ("Renamed", level::FIGURE, "ref.jpg"));
        // remove: files gone, item returned
        let gone = r.remove("my.refs:tl1").unwrap();
        assert_eq!(gone.title, "Renamed");
        assert!(!dir.path().join("tl1").exists());
        assert!(matches!(r.remove("my.refs:tl1"), Err(KnowError::NotFound(_))));
        assert!(r.load().unwrap().0.is_empty());
    }

    // ── step 5 ──

    fn template() -> Value {
        serde_json::from_str(ENCODER_PLAIN).unwrap()
    }

    #[test]
    fn a_saved_drawing_is_read_back_by_the_flow_tools_own_loader() {
        let dir = tempfile::tempdir().unwrap();
        let rel = save_drawing(dir.path(), "encoder", "wide", &template(), "", false).unwrap();
        assert_eq!(rel, PathBuf::from("modules/encoder/wide.json"));
        let lib = Library::load_dir(&dir.path().join("modules")).unwrap();
        assert!(lib.get("encoder", "wide").is_some());
        // a level survives for the adapter, and the tool still loads the file
        save_drawing(dir.path(), "chip", "flat", &template_of("chip"), level::COMPONENT, false).unwrap();
        let cat = Catalogue::load_all(&[dir.path()]);
        assert!(cat.library.get("chip", "flat").is_some());
        let d = Drawings { flow_dir: dir.path().into(), category: "sc.figs".into(), source: Source::lore("sc") };
        let (items, problems) = d.load().unwrap();
        assert!(problems.is_empty());
        let level_of = |id: &str| items.iter().find(|i| i.id == id).unwrap().level.clone();
        assert_eq!((level_of("sc.figs:encoder.wide").as_str(), level_of("sc.figs:chip.flat").as_str()), (level::MODULE, level::COMPONENT));
        // save through the adapter and remove again
        let mut it = items.iter().find(|i| i.id == "sc.figs:chip.flat").unwrap().clone();
        it.level = level::MODULE.into();
        d.save(&it).unwrap();
        assert_eq!(d.load().unwrap().0.iter().find(|i| i.id == it.id).unwrap().level, level::MODULE);
        let gone = d.remove("sc.figs:chip.flat").unwrap();
        assert_eq!(gone.field("kind"), "chip");
        assert!(!dir.path().join("modules/chip").exists());
        assert!(Catalogue::load_all(&[dir.path()]).library.get("chip", "flat").is_none());
    }

    fn template_of(kind: &str) -> Value {
        let mut t = template();
        t["kind"] = kind.into();
        t
    }

    #[test]
    fn an_existing_drawing_is_not_overwritten_silently() {
        let dir = tempfile::tempdir().unwrap();
        save_drawing(dir.path(), "encoder", "wide", &template(), "", false).unwrap();
        let mut other = template();
        other["description"] = "changed".into();
        let err = save_drawing(dir.path(), "encoder", "wide", &other, "", false).unwrap_err();
        assert!(matches!(err, KnowError::Invalid(_)));
        let path = dir.path().join("modules/encoder/wide.json");
        assert!(!fs::read_to_string(&path).unwrap().contains("changed"));
        save_drawing(dir.path(), "encoder", "wide", &other, "", true).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("changed"));
    }

    #[test]
    fn a_bad_drawing_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let t = template();
        for (k, s) in [("", "a"), ("a", ""), ("../x", "a"), ("a", "b/c"), ("a b", "c"), ("a", "b.c")] {
            assert!(save_drawing(dir.path(), k, s, &t, "", false).is_err(), "{k:?} {s:?}");
        }
        assert!(save_drawing(dir.path(), "encoder", "x", &t, "huge", false).is_err());
        // another kind than the file says
        assert!(save_drawing(dir.path(), "decoder", "x", &t, "", false).is_err());
        // not a drawing at all, no size, objects that are no canvas objects
        assert!(save_drawing(dir.path(), "encoder", "x", &json!({ "hello": 1 }), "", false).is_err());
        let mut zero = template();
        zero["w"] = 0.into();
        assert!(save_drawing(dir.path(), "encoder", "x", &zero, "", false).is_err());
        let mut junk = template();
        junk["objects"] = json!([{ "nonsense": true }]);
        assert!(save_drawing(dir.path(), "encoder", "x", &junk, "", false).is_err());
        assert!(!dir.path().join("modules").exists(), "nothing is written for a refused drawing");
        // a file in the folder that is not a drawing is listed, not fatal
        fs::create_dir_all(dir.path().join("modules/encoder")).unwrap();
        fs::write(dir.path().join("modules/encoder/broken.json"), "{ nope").unwrap();
        save_drawing(dir.path(), "encoder", "good", &t, "", false).unwrap();
        let d = Drawings { flow_dir: dir.path().into(), category: "c".into(), source: Source::author() };
        let (items, problems) = d.load().unwrap();
        assert_eq!((items.len(), problems.len()), (1, 1));
    }

    // ── step 6 ──

    fn spec_of(dsl: &str) -> Value {
        json!({ "lore_flow": 1, "dsl": dsl })
    }

    #[test]
    fn a_spec_lists_the_entities_it_uses() {
        let dir = tempfile::tempdir().unwrap();
        save_drawing(dir.path(), "chip", "flat", &template_of("chip"), level::COMPONENT, false).unwrap();
        save_drawing(dir.path(), "document", "page", &serde_json::from_str(DOCUMENT_PAGE).unwrap(), "", false).unwrap();
        let cat = Catalogue::load_all(&[dir.path()]);
        let entities = from_catalogue(&cat, "c", Source::lore("sc"));
        let spec = spec_of("fig demo\nx: image \"x\"\ndoc: document \"D\" style=page\nseg: unet \"S\" style=bars\nb: box \"B\"\nb2: box \"B2\"\nc: chip \"C\"\nx -> doc -> seg -> b -> b2 -> c\n");
        let parts = parts_of(&spec, &entities);
        // image/box without a style take their kind's first style; a saved drawing is found by its kind
        assert_eq!(parts, ["c:image.default", "c:document.page", "c:unet.bars", "c:box.rect", "c:chip.flat"]);
        // the spec as an item
        let mut it = spec_item("c", "Method", &spec, Source::author());
        it.contains = parts.clone();
        assert_eq!((it.level.as_str(), it.entity, it.field("what")), (level::FIGURE, true, "spec"));
        assert_eq!(Figure.validate(&it), vec![]);
        // an unreadable spec adds nothing
        assert!(parts_of(&spec_of("nonsense ((("), &entities).is_empty());
        assert!(parts_of(&json!(5), &entities).is_empty());
        // the spec given as JSON text works as well
        assert_eq!(parts_of(&Value::String(spec.to_string()), &entities), parts);
    }

    #[test]
    fn links_read_both_ways() {
        let mk = |id: &str, contains: &[&str]| {
            let mut i = Item::new("c", format::FIGURE, id, Source::author());
            i.id = format!("c:{id}");
            i.contains = contains.iter().map(|s| format!("c:{s}")).collect();
            i
        };
        let all = vec![mk("fig1", &["enc", "box"]), mk("fig2", &["enc"]), mk("enc", &["box"]), mk("box", &[])];
        let names = |v: Vec<&Item>| v.iter().map(|i| i.title.clone()).collect::<Vec<_>>();
        assert_eq!(names(used_by("c:enc", &all)), ["fig1", "fig2"]);
        assert_eq!(names(used_by("c:box", &all)), ["fig1", "enc"]);
        assert!(used_by("c:fig1", &all).is_empty());
        // and the other way: what an item contains is in its own list
        let fig1 = &all[0];
        assert!(fig1.contains.iter().all(|id| all.iter().any(|i| &i.id == id)));
    }

    // ── knowledge fields ──

    #[test]
    fn a_figure_has_the_four_knowledge_fields() {
        let d = Figure.def();
        let names: Vec<&str> = d.fields.iter().map(|f| f.name.as_str()).collect();
        let at = names.iter().position(|n| *n == "description").unwrap();
        assert_eq!(&names[at + 1..at + 5], ["prompt", "inputs", "outputs", "connects"]);
        let labels: Vec<&str> = d.fields[at + 1..at + 5].iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["用法", "輸入變數", "輸出變數", "連線"]);
        assert!(d.fields[at + 1..at + 5].iter().all(|f| !f.required));
        let mut e = fig_item(what::GENERATOR, level::MODULE, true);
        e.set("inputs", "x | image | scan\n# note\n\nz | | free").set("connects", "a -> b\nb -> c : feedback");
        assert!(Figure.validate(&e).is_empty());
    }

    #[test]
    fn a_variable_line_with_an_unknown_kind_is_a_problem() {
        let mut e = fig_item(what::GENERATOR, level::MODULE, true);
        e.set("inputs", "x | image | ok\ny | pixels | bad");
        e.set("outputs", "m | mask | ok");
        let p = Figure.validate(&e);
        assert_eq!(fields_of(&p), ["inputs"]);
        assert!(p[0].message.contains("y | pixels | bad"));
        e.set("outputs", "m | blob | bad");
        assert_eq!(fields_of(&Figure.validate(&e)), ["inputs", "outputs"]);
    }

    #[test]
    fn a_connection_line_must_say_from_and_to() {
        let mut e = fig_item(what::GENERATOR, level::MODULE, true);
        for bad in ["a", "a ->", "-> b", "a -> b -> c", "a - b"] {
            e.set("connects", bad);
            assert_eq!(fields_of(&Figure.validate(&e)), ["connects"], "{bad}");
        }
        e.set("connects", "a -> b\nb -> c : skip");
        assert!(Figure.validate(&e).is_empty());
    }

    #[test]
    fn a_figures_prompt_says_use_variables_and_flow() {
        let mut e = fig_item(what::GENERATOR, level::MODULE, true);
        e.title = "U-Net".into();
        e.set("description", "A flat U-shaped net\nsecond line");
        e.set("prompt", "Use it for segmentation.");
        e.set("inputs", "x | image | scan\nz | | free");
        e.set("outputs", "m | mask | result");
        e.set("connects", "x -> m\nm -> loss : supervision");
        assert_eq!(
            Figure.prompt(&e),
            "U-Net [module]: A flat U-shaped net\nuse: Use it for segmentation.\nin: x (image), z (other)\nout: m (mask)\nflow: x -> m ; m -> loss : supervision\nkind=unet style=bars"
        );
    }

    #[test]
    fn empty_parts_are_left_out() {
        let mut s = fig_item(what::SPEC, level::FIGURE, true);
        s.title = "Whole".into();
        s.set("inputs", "  \n").set("connects", "").set("prompt", "  ");
        assert_eq!(Figure.prompt(&s), "Whole [figure]");
        s.set("outputs", "y | loss | l");
        assert_eq!(Figure.prompt(&s), "Whole [figure]\nout: y (loss)");
        let many: Vec<String> = (0..14).map(|i| format!("n{i} -> n{}", i + 1)).collect();
        s.set("connects", many.join("\n"));
        let p = Figure.prompt(&s);
        assert!(p.contains("n11 -> n12 ; …") && !p.contains("n12 -> n13"), "{p}");
    }

    #[test]
    fn params_never_reach_the_prompt() {
        let mut s = fig_item(what::SPEC, level::FIGURE, true);
        s.title = "Whole".into();
        s.set("params", json!({"dsl": "SECRET-DSL-TEXT"}));
        assert!(!Figure.prompt(&s).contains("SECRET"));
        let mut g = fig_item(what::DRAWING, level::MODULE, true);
        g.set("params", json!({"kind": "unet", "extra": "PARAM-TEXT"}));
        assert!(!Figure.prompt(&g).contains("PARAM"));
    }
}
