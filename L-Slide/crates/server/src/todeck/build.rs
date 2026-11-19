//! `slides`: the deck from a plan — page kinds from the base, slot geometry from the
//! author's academic arrangements, text in the style's tags, native tables and OMML
//! equations, `Ref:` lines, the module crops of the main figure.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lore_canvas::StyleSystem;
use lore_canvas::{Bullet, Cell, Crop, Frame, Geom, Kind, Look, Object, Para, Paint, PicObj, Row, Run, Slide, SpObj, Stroke, TableObj, TextBody};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::plan::{Plan, clip, strip_typed_refs};
use super::source::{Img, Material};

const EMU_IN: i64 = 914_400;
/// "No Style, Table Grid" — the author's table look (design §7.1).
const TABLE_GRID: &str = "{5940675A-B579-460E-94D1-54222C63F5DA}";

/// A block of an arrangement (fractions of the slide).
#[derive(Debug, Clone)]
pub struct Block {
    pub role: String,
    pub rect: [f64; 4],
}

#[derive(Debug, Clone)]
pub struct Arrangement {
    pub id: String,
    pub blocks: Vec<Block>,
}

/// The author's academic arrangements (`arrangements/*.json`).
pub fn academic(ws: &sc_core::Workspace) -> Vec<Arrangement> {
    ws.arrangements()
        .into_iter()
        .filter(|a| a.get("class").and_then(Value::as_str).unwrap_or("academic") == "academic")
        .filter_map(|a| {
            let blocks: Vec<Block> = a["blocks"]
                .as_array()?
                .iter()
                .filter_map(|b| {
                    let r = &b["rect"];
                    let rect = if let Some(v) = r.as_array() {
                        [v.first()?.as_f64()?, v.get(1)?.as_f64()?, v.get(2)?.as_f64()?, v.get(3)?.as_f64()?]
                    } else {
                        [r["x"].as_f64()?, r["y"].as_f64()?, r["w"].as_f64()?, r["h"].as_f64()?]
                    };
                    Some(Block { role: b["role"].as_str().unwrap_or("").to_string(), rect })
                })
                .collect();
            Some(Arrangement { id: a["id"].as_str()?.to_string(), blocks })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default)]
struct Need {
    image: bool,
    table: bool,
    text: bool,
}

/// The arrangement whose blocks fit what the slide shows best (none: the base's content area).
fn pick<'a>(arrs: &'a [Arrangement], n: Need) -> Option<&'a Arrangement> {
    let score = |a: &Arrangement| {
        let has = |r: &str| a.blocks.iter().any(|b| b.role == r);
        let mut s = 0i32;
        let visual = n.image || n.table;
        if n.image {
            s += if has("image") { 4 } else { -4 };
        }
        if n.table {
            s += if has("table") { 4 } else if has("image") && !n.image { 2 } else { -3 };
        }
        if n.text {
            s += if has("text") { 3 } else { -3 };
        } else if has("text") && !visual {
            s -= 1;
        }
        if !visual && has("image") {
            s -= 3;
        }
        if !n.text && has("text") {
            s -= 1;
        }
        s += if has("title") { 1 } else { 0 };
        s
    };
    arrs.iter().filter(|a| a.blocks.iter().any(|b| b.role != "title")).max_by_key(|a| (score(a), -(a.blocks.len() as i32)))
}

struct Ctx<'a> {
    st: &'a StyleSystem,
    pandoc: &'a lore_math::Pandoc,
    macros: BTreeMap<String, String>,
    warnings: Vec<String>,
    deck_dir: PathBuf,
    /// LaTeX → OMML already converted (the same equation on several slides).
    omml: BTreeMap<(String, bool), Option<String>>,
}

impl Ctx<'_> {
    fn tag(&self, want: &[&str]) -> String {
        want.iter().find(|t| self.st.tag(t).is_some()).map(|t| t.to_string()).or_else(|| self.st.tag("body").map(|_| "body".to_string())).or_else(|| self.st.pack.tags.keys().next().cloned()).unwrap_or_else(|| "body".into())
    }
    fn look(&self, tag: &str, em: Option<&str>) -> Look {
        self.st.look(tag, em).unwrap_or_default()
    }
    fn color(&self, names: &[&str]) -> String {
        names.iter().find_map(|n| self.st.color(n)).unwrap_or_else(|| "000000".into())
    }
    fn run(&self, text: &str, tag: &str, em: Option<&str>) -> Run {
        let mut r = Run::text(text, tag, self.look(tag, em));
        r.em = em.map(String::from);
        r
    }
    fn para(&self, runs: Vec<Run>, tag: &str) -> Para {
        let mut p = Para::new(runs);
        p.end = Some(self.look(tag, None));
        p
    }

    fn expand(&self, latex: &str) -> String {
        expand_macros(latex, &self.macros)
    }

    fn omml(&mut self, latex: &str, display: bool, size: f64) -> Option<String> {
        let key = (latex.to_string(), display);
        if let Some(v) = self.omml.get(&key) {
            return v.clone();
        }
        let v = if self.pandoc.available() {
            match sc_core::latex_to_omml(self.pandoc, latex, display, size) {
                Ok(x) => Some(x),
                Err(e) => {
                    self.warnings.push(format!("方程式無法轉換（{}）：{e}", clip(latex, 40)));
                    None
                }
            }
        } else {
            None
        };
        self.omml.insert(key, v.clone());
        v
    }

    /// Text with `$…$` as native math runs (plain text when pandoc is missing).
    fn rich(&mut self, text: &str, tag: &str, em: Option<&str>) -> Vec<Run> {
        let mut out = vec![];
        let parts: Vec<&str> = text.split('$').collect();
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            let math = i % 2 == 1 && i + 1 < parts.len();
            if !math {
                out.push(self.run(part, tag, em));
                continue;
            }
            let latex = self.expand(part);
            let size = self.look(tag, None).size;
            match self.omml(&latex, false, size) {
                Some(omml) => {
                    let mut look = self.look(tag, em);
                    look.font = "Cambria Math".into();
                    look.ea_font = "Cambria Math".into();
                    out.push(Run { kind: "math".into(), text: latex.clone(), tag: None, em: None, look, field: None, omml: Some(omml), latex: Some(latex), src: None, k: 0 });
                }
                // no OMML (pandoc missing): readable text, not LaTeX source
                None => out.push(self.run(&math_plain(part), tag, em)),
            }
        }
        out
    }
}

/// Inline LaTeX as plain text when it cannot become OMML: font and accent commands and
/// braces dropped, Greek letters and a few operators as characters, scripts as `_` / `^`
/// (`C_{\mathrm{enc}}` → `C_enc`).
pub fn math_plain(latex: &str) -> String {
    const SYM: &[(&str, &str)] = &[
        ("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ε"), ("varepsilon", "ε"), ("theta", "θ"), ("lambda", "λ"), ("mu", "μ"),
        ("sigma", "σ"), ("phi", "φ"), ("varphi", "φ"), ("psi", "ψ"), ("omega", "ω"), ("pi", "π"), ("rho", "ρ"), ("tau", "τ"), ("eta", "η"), ("kappa", "κ"),
        ("Gamma", "Γ"), ("Delta", "Δ"), ("Theta", "Θ"), ("Lambda", "Λ"), ("Sigma", "Σ"), ("Phi", "Φ"), ("Psi", "Ψ"), ("Omega", "Ω"), ("sim", "∼"),
        ("in", "∈"), ("leq", "≤"), ("geq", "≥"), ("neq", "≠"), ("approx", "≈"), ("times", "×"), ("cdot", "·"), ("sum", "Σ"), ("infty", "∞"), ("to", "→"),
        ("ell", "ℓ"), ("partial", "∂"), ("nabla", "∇"), ("prime", "′"), ("|", "‖"),
    ];
    let cs: Vec<char> = latex.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        match c {
            '\\' => {
                let start = i;
                while i < cs.len() && cs[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i == start && i < cs.len() {
                    i += 1;
                }
                let name: String = cs[start..i].iter().collect();
                if let Some((_, u)) = SYM.iter().find(|(n, _)| *n == name) {
                    out.push_str(u);
                } else if matches!(name.as_str(), "quad" | "qquad" | "," | ";" | " ") {
                    out.push(' ');
                }
            }
            '{' | '}' => {}
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A caption for the slide: whole sentences up to `n` characters (the first sentence when
/// it alone is longer, cut at a word with `…`), never inside `$…$`.
pub fn caption_clip(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        return s.to_string();
    }
    // sentence ends outside math
    let mut ends = vec![];
    let mut math = false;
    let cs: Vec<char> = s.chars().collect();
    for (i, &c) in cs.iter().enumerate() {
        if c == '$' {
            math = !math;
        } else if !math && c == '.' && cs.get(i + 1).is_none_or(|x| x.is_whitespace()) {
            ends.push(i + 1);
        }
    }
    if let Some(&e) = ends.iter().rev().find(|&&e| e <= n) {
        return cs[..e].iter().collect();
    }
    let mut cut: String = cs[..n].iter().collect();
    if let Some(sp) = cut.rfind(' ') {
        cut.truncate(sp);
    }
    if cut.matches('$').count() % 2 == 1 {
        let open = cut.rfind('$').unwrap_or(0);
        cut.truncate(open);
    }
    cut.trim_end().to_string() + "…"
}

/// Symbol keys (lore-notation canonical) an equation uses: its symbols and those in its
/// index scripts (`x` and `P` of `\mathbb{E}_{x\sim P}`), not the letters of a name
/// subscript (`r` of `\mathcal{L}_{rec}` is no symbol of the equation).
pub fn equation_keys(latex: &str) -> BTreeSet<String> {
    use lore_notation::math;
    fn walk(s: &math::Sym, out: &mut BTreeSet<String>) {
        out.insert(s.key());
        for u in &s.script_uses {
            walk(u, out);
        }
    }
    let idx: Vec<String> = math::DEFAULT_INDEX.iter().map(|s| s.to_string()).collect();
    let atoms = math::parse(&math::tokenize(latex, 0), &math::Options { index_letters: &idx });
    let mut syms = vec![];
    math::collect_syms(&atoms, &mut syms);
    let mut out = BTreeSet::new();
    for s in &syms {
        walk(s, &mut out);
    }
    out
}

/// The note lines under a slide's equations: each registry symbol the equations use, once,
/// in the deck's language — the author's meaning (`meaning`, the registry language) for a
/// Chinese deck, the English one for an English deck; a meaning only in the other language
/// is written `$x$：meaning`.
pub fn symbol_notes(symbols: &[super::source::SymbolNote], used: &BTreeSet<String>, lang: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    let zh = |t: &str| t.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
    for n in symbols {
        let key = lore_notation::math::canonical(&n.latex);
        if !used.contains(&key) || !seen.insert(key) {
            continue;
        }
        let (mine, en) = (n.meaning.as_deref().map(str::trim).filter(|m| !m.is_empty()), n.meaning_en.as_deref().map(str::trim).filter(|m| !m.is_empty()));
        let sym = n.latex.trim();
        let line = if lang == "en" {
            match en.or(mine.filter(|m| !zh(m))) {
                Some(m) => format!("where ${sym}$ denotes {m}"),
                None => format!("${sym}$: {}", mine.unwrap_or_default()),
            }
        } else {
            match mine.filter(|m| zh(m)) {
                Some(m) => format!("${sym}$ 為{m}"),
                None => format!("${sym}$：{}", mine.or(en).unwrap_or_default()),
            }
        };
        out.push(line);
    }
    out
}

/// Registry macros (`\vx` → `\mathbf{x}`) expanded so pandoc and KaTeX read the equation;
/// the registry still recognises the expanded form (lore-notation matches LaTeX and macros).
pub fn expand_macros(latex: &str, macros: &BTreeMap<String, String>) -> String {
    let mut s = latex.to_string();
    for _ in 0..3 {
        let mut changed = false;
        let mut out = String::with_capacity(s.len());
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                if j > i + 1
                    && let Some(rep) = macros.get(&s[i..j]).or_else(|| macros.get(&s[i + 1..j]))
                {
                    out.push_str(rep);
                    changed = true;
                    i = j;
                    continue;
                }
                out.push_str(&s[i..j.max(i + 1)]);
                i = j.max(i + 1);
                continue;
            }
            let c = s[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
        s = out;
        if !changed {
            break;
        }
    }
    s
}

fn text_obj(name: &str, frame: Frame, body: TextBody, slot: Option<&str>) -> Object {
    Object {
        id: 0,
        name: name.into(),
        frame,
        hidden: false,
        ph: None,
        slot: slot.map(String::from),
        kind: Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(body), equation: false }),
        src: None,
    }
}

fn body(paras: Vec<Para>, anchor: &str) -> TextBody {
    TextBody { insets: [0, 0, 0, 0], anchor: anchor.into(), wrap: true, autofit: "none".into(), paras, ..Default::default() }
}

fn pic(name: &str, frame: Frame, asset: &str, crop: Crop, alpha: f64, slot: Option<&str>) -> Object {
    Object {
        id: 0,
        name: name.into(),
        frame,
        hidden: false,
        ph: None,
        slot: slot.map(String::from),
        kind: Kind::Picture(PicObj { asset: asset.into(), crop, alpha, line: None, clip: "rect".into(), masks: vec![], effects: vec![] }),
        src: None,
    }
}

/// Fit `iw × ih` inside `f` (contain, centred across, at the top).
fn contain(f: &Frame, iw: u32, ih: u32) -> Frame {
    let (iw, ih) = (iw.max(1) as f64, ih.max(1) as f64);
    let s = (f.w as f64 / iw).min(f.h as f64 / ih);
    let (w, h) = ((iw * s) as i64, (ih * s) as i64);
    Frame::new(f.x + (f.w - w) / 2, f.y, w, h)
}

/// Module box of the main figure, in pixels of its image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleBox {
    pub label: String,
    pub bbox: [f64; 4],
}

/// Where everything of one page goes.
struct Areas {
    title: Option<Frame>,
    kicker: Option<Frame>,
    content: Frame,
    takeaway: Option<Frame>,
    subtitle: Option<Frame>,
}

fn areas(ws: &sc_core::Workspace, st: &StyleSystem, source: &lore_ooxml::Deck, kind: &str, w: i64, h: i64) -> Areas {
    let base = ws.base(&st.pack.pack.id).ok().flatten();
    let page = base.as_ref().and_then(|b| b.pages.get(kind).or_else(|| b.pages.get("content")));
    let pa = ws.page_areas(Some(st), source);
    let v = pa.iter().find(|p| p["kind"] == kind).or_else(|| pa.iter().find(|p| p["kind"] == "content"));
    let fr = |k: &str| v.and_then(|v| serde_json::from_value::<Frame>(v[k].clone()).ok());
    Areas {
        title: fr("title"),
        kicker: fr("kicker"),
        content: fr("content").unwrap_or_else(|| Frame::new(w / 25, h / 5, w - 2 * w / 25, h * 3 / 5)),
        takeaway: fr("takeaway"),
        subtitle: page.and_then(|p| p.subtitle.clone()),
    }
}

/// The frames every content page of one deck shares, so the same role sits at the same place
/// on every page: the body area (clear of what the layout draws at the bottom: logo, marks,
/// the page number, a bar) and the `Ref:` line (beside the logo, where the author's own decks
/// put it; never over it).
struct PageFrames {
    content: Frame,
    refs: Frame,
    /// A dark takeaway band the base itself has (`None`: the takeaway is a line under the content).
    band: Option<Frame>,
    kicker: Option<Frame>,
    title: Frame,
}

/// How many lines a `Ref:` box holds.
const REF_LINES: i64 = 3;

fn page_frames(ws: &sc_core::Workspace, st: &StyleSystem, source: &lore_ooxml::Deck, arrs: &[Arrangement], w: i64, h: i64) -> PageFrames {
    let ar = areas(ws, st, source, "content", w, h);
    let layout = st.pack.layouts.get("content").cloned().unwrap_or_default();
    // what the layout draws in the bottom band
    let drawn: Vec<Frame> = lore_canvas::pptx_in::backdrops(source).into_iter().filter(|b| b.layout == layout).flat_map(|b| b.objects).filter(|o| !o.hidden).map(|o| o.frame).filter(|f| f.y + f.h / 2 > h * 86 / 100 && f.y < h).collect();
    let bar_top = drawn.iter().filter(|f| f.w > w * 8 / 10).map(|f| f.y).min().unwrap_or(h * 98 / 100);
    let small: Vec<&Frame> = drawn.iter().filter(|f| f.w <= w * 8 / 10).collect();
    let left = small.iter().filter(|f| f.x + f.w / 2 < w * 35 / 100).map(|f| f.x + f.w).max();
    let right = small.iter().filter(|f| f.x + f.w / 2 > w * 65 / 100).map(|f| f.x).min();
    let ref_size = st.tag("source").or_else(|| st.tag("reference")).or_else(|| st.tag("caption")).map(|t| t.size).unwrap_or(9.0);
    let ref_h = (ref_size * 1.2 * REF_LINES as f64 * 12_700.0) as i64 + h / 200;
    let rx = left.map(|l| l + w / 150).unwrap_or(ar.content.x);
    let rw = (right.unwrap_or(w - ar.content.x) - w / 150 - rx).min(w * 6 / 10).max(w / 4);
    let ry = bar_top - h / 200 - ref_h;
    let refs = Frame::new(rx, ry, rw, ref_h);
    let band = ar.takeaway.clone().filter(|_| st.color("inverse").is_some());
    let mut bottom = (ar.content.y + ar.content.h).min(ry - h / 100);
    if let Some(top) = small.iter().map(|f| f.y).min() {
        bottom = bottom.min(top - h / 100);
    }
    if let Some(b) = &band {
        bottom = bottom.min(b.y - h / 60);
    }
    // the author's arrangements start the body at one line; the base's area when it has one
    let from_arr = arrs.iter().flat_map(|a| a.blocks.iter()).find(|b| b.role == "text").map(|b| ((b.rect[0] * w as f64) as i64, (b.rect[1] * h as f64) as i64, (b.rect[2] * w as f64) as i64));
    let (cx, cy, cw) = match (from_arr, ws.base(&st.pack.pack.id).ok().flatten().is_some()) {
        (Some(a), false) => (a.0, a.1.max(ar.content.y.min(a.1)), a.2),
        _ => (ar.content.x, ar.content.y, ar.content.w),
    };
    let content = Frame::new(cx, cy, cw, (bottom - cy).max(h / 4));
    let title = ar.title.clone().unwrap_or_else(|| {
        arrs.iter().flat_map(|a| a.blocks.iter()).find(|b| b.role == "title").map(|b| Frame::new((b.rect[0] * w as f64) as i64, (b.rect[1] * h as f64) as i64, (b.rect[2] * w as f64) as i64, (b.rect[3] * h as f64) as i64)).unwrap_or_else(|| Frame::new(w / 25, h / 12, w * 23 / 25, h / 10))
    });
    PageFrames { content, refs, band, kicker: ar.kicker, title }
}

fn kicker_label(role: &str, lang: &str) -> &'static str {
    let zh = lang != "en";
    match role {
        "background" => if zh { "背景" } else { "Background" },
        "challenge" | "overview" => if zh { "問題與架構" } else { "Big picture" },
        "method" | "loss" => if zh { "方法" } else { "Method" },
        "training" => if zh { "訓練" } else { "Training" },
        "results" | "ablation" | "visual" => if zh { "結果" } else { "Results" },
        "opinion" | "takeaway" => if zh { "討論" } else { "Discussion" },
        _ => "",
    }
}

/// What the build produced besides the slides.
pub struct Built {
    pub slides: Vec<Slide>,
    /// Index of the big-picture slide and the object ids of each numbered step.
    pub reveal: Option<(usize, Vec<Vec<u32>>)>,
    pub warnings: Vec<String>,
    /// The body area every content page shares.
    pub content: Frame,
}

pub struct BuildInput<'a> {
    pub ws: &'a sc_core::Workspace,
    pub st: &'a StyleSystem,
    pub source: &'a lore_ooxml::Deck,
    pub deck_dir: &'a Path,
    pub pandoc: &'a lore_math::Pandoc,
    pub macros: BTreeMap<String, String>,
    pub lang: &'a str,
    pub boxes: Vec<ModuleBox>,
    /// The cover's Speaker and Supervisor.
    pub presenter: Option<String>,
    pub supervisor: Option<String>,
    pub next_uid: &'a mut dyn FnMut() -> String,
    /// Told after each planned slide: (done, total).
    pub progress: &'a mut dyn FnMut(usize, usize),
}

/// Estimated height of a body `w` wide (EMU), with a little air.
fn text_h(b: &TextBody, w: i64) -> i64 {
    (crate::deckpolish::text_size(b, w).0 * 1.04) as i64 + 12_700
}

/// The slides of a plan.
pub fn build(plan: &Plan, m: &Material, inp: BuildInput) -> Result<Built, String> {
    let (w, h) = inp.source.slide_size;
    let mut cx = Ctx { st: inp.st, pandoc: inp.pandoc, macros: inp.macros, warnings: vec![], deck_dir: inp.deck_dir.to_path_buf(), omml: BTreeMap::new() };
    let arrs = academic(inp.ws);
    let pf = page_frames(inp.ws, inp.st, inp.source, &arrs, w, h);
    let gap = h / 60;
    let mut assets: BTreeMap<String, String> = BTreeMap::new();
    let mut out = vec![];
    let mut reveal_steps = None;
    let main_fig = plan.slides.iter().find(|s| s.role == "overview").map(|s| s.figure.clone()).unwrap_or_default();

    for (page, ps) in plan.slides.iter().enumerate() {
        let kind = match ps.role.as_str() {
            "cover" => "cover",
            "thanks" if inp.st.pack.layouts.contains_key("end") => "end",
            _ => "content",
        };
        let layout = inp.st.pack.layouts.get(kind).or_else(|| inp.st.pack.layouts.get("content")).cloned();
        let mut slide = Slide::new((inp.next_uid)(), layout.clone());
        let mut objs = match &layout {
            Some(l) => inp.ws.new_slide_objects(Some(inp.st), inp.source, l, inp.deck_dir).map_err(|e| e.to_string())?,
            None => vec![],
        };
        slide.notes = ps.notes.clone();
        let mut extra: Vec<Object> = vec![];
        let warn_at = format!("第 {} 頁「{}」", page + 1, clip(&ps.title, 24));

        let fig = m.figures.iter().find(|f| f.id == ps.figure && f.image.is_some()).or_else(|| m.tables.iter().find(|t| t.id == ps.figure && t.image.is_some()));
        let tab = m.tables.iter().find(|t| t.id == ps.table);
        let tab_native = tab.and_then(|t| t.grid.clone()).filter(|g| !g.is_empty());
        let tab_pic = if tab_native.is_none() { tab.filter(|t| t.image.is_some()) } else { None };
        let image = fig.or(tab_pic);
        let kv = (!ps.kv.is_empty()).then(|| ps.kv.iter().map(|r| vec![r[0].clone(), r[1..].join(" ")]).collect::<Vec<_>>());
        let table_grid = kv.clone().or(tab_native.clone());

        match kind {
            "cover" => {
                let ar = areas(inp.ws, inp.st, inp.source, kind, w, h);
                let title_tag = cx.tag(&["cover-title", "title"]);
                let title = if m.title.is_empty() { ps.title.clone() } else { m.title.clone() };
                let tf = ar.title.clone().unwrap_or_else(|| Frame::new(w / 16, h * 3 / 10, w * 7 / 8, h / 6));
                fill_title(&mut objs, &cx, &title, &title_tag, tf.clone());
                // a long title grows upwards from where the layout ends it: the block below stays put
                if let Some(o) = objs.iter_mut().find(|o| matches!(o.ph.as_deref(), Some("title") | Some("ctrTitle")) || o.slot.as_deref() == Some("title")) {
                    let need = o.kind.sp().and_then(|s| s.text.as_ref()).map(|t| text_h(t, o.frame.w)).unwrap_or(0);
                    if need > o.frame.h {
                        let bottom = o.frame.y + o.frame.h;
                        o.frame = Frame::new(o.frame.x, (bottom - need).max(h / 12), o.frame.w, need);
                    }
                }
                let venue = [plan.venue.trim(), m.venue.trim()].into_iter().find(|v| !v.is_empty()).unwrap_or("").to_string();
                let venue = if !m.year.is_empty() && !venue.contains(&m.year) && !venue.is_empty() { format!("{venue}, {}", m.year) } else { venue };
                // journal line, then who presents (the base's `subtitle` = the Speaker / Supervisor block)
                let mut paras = vec![];
                if !venue.is_empty() {
                    let t = cx.tag(&["venue", "label", "subtitle"]);
                    let mut p = cx.para(vec![cx.run(&venue, &t, None)], &t);
                    p.after_pt = Some(6.0);
                    paras.push(p);
                }
                let zh = inp.lang != "en";
                let who = [(inp.presenter.as_deref(), if zh { "報告者：" } else { "Speaker: " }), (inp.supervisor.as_deref(), if zh { "指導教授：" } else { "Supervisor: " })];
                for (name, label) in who {
                    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
                        let t = cx.tag(&["subtitle", "body"]);
                        paras.push(cx.para(vec![cx.run(&format!("{label}{n}"), &t, None)], &t));
                    }
                }
                if !paras.is_empty() {
                    let tb = body(paras, "t");
                    match objs.iter_mut().find(|o| o.ph.as_deref() == Some("subTitle")) {
                        Some(o) => {
                            let need = text_h(&tb, o.frame.w);
                            o.frame.h = o.frame.h.max(need);
                            if let Some(sp) = o.kind.sp_mut() {
                                sp.text = Some(tb);
                            }
                        }
                        None => {
                            let f = ar.subtitle.clone().unwrap_or_else(|| Frame::new(tf.x, tf.y + tf.h + h / 40, tf.w, h / 4));
                            let need = text_h(&tb, f.w);
                            extra.push(text_obj("講者", Frame::new(f.x, f.y, f.w, need.max(f.h.min(h / 5))), tb, None));
                        }
                    }
                }
            }
            "end" => {
                let ar = areas(inp.ws, inp.st, inp.source, kind, w, h);
                let t = cx.tag(&["cover-title", "title"]);
                fill_title(&mut objs, &cx, &ps.title, &t, ar.title.clone().unwrap_or_else(|| Frame::new(w / 16, h * 2 / 5, w * 7 / 8, h / 6)));
            }
            _ if ps.role == "thanks" => {
                // a closing page on the content layout: the title alone, in the middle
                let t = cx.tag(&["title"]);
                fill_title(&mut objs, &cx, &ps.title, &t, pf.title.clone());
                if let Some(o) = objs.iter_mut().find(|o| matches!(o.ph.as_deref(), Some("title") | Some("ctrTitle")) || o.slot.as_deref() == Some("title")) {
                    o.frame = Frame::new(o.frame.x, (h - o.frame.h) / 2, o.frame.w, o.frame.h);
                    if let Some(tb) = o.kind.sp_mut().and_then(|s| s.text.as_mut()) {
                        tb.anchor = "ctr".into();
                        tb.paras.iter_mut().for_each(|p| p.align = "ctr".into());
                    }
                }
            }
            _ => {
                let title_tag = cx.tag(&["title"]);
                let has_text = !ps.bullets.is_empty() || !ps.equations.is_empty() || !ps.steps.is_empty();
                let need = Need { image: image.is_some(), table: table_grid.is_some(), text: has_text };
                let arr = pick(&arrs, need);
                slide.arrangement = arr.map(|a| a.id.clone());
                let frac = |b: &Block| Frame::new((b.rect[0] * w as f64) as i64, (b.rect[1] * h as f64) as i64, (b.rect[2] * w as f64) as i64, (b.rect[3] * h as f64) as i64);
                let block = |role: &str| arr.and_then(|a| a.blocks.iter().find(|b| b.role == role)).map(frac);
                // the title: always the layout's own title (one frame on every page)
                fill_title(&mut objs, &cx, &ps.title, &title_tag, pf.title.clone());
                if let Some(kf) = &pf.kicker {
                    let label = kicker_label(&ps.role, inp.lang);
                    if !label.is_empty() {
                        let t = cx.tag(&["kicker", "label"]);
                        extra.push(text_obj("kicker", kf.clone(), body(vec![cx.para(vec![cx.run(label, &t, None)], &t)], "b"), None));
                    }
                }
                let c = pf.content.clone();
                // the takeaway: the base's dark band when it has one, else a key line right under the content
                let take = (!ps.takeaway.trim().is_empty()).then(|| takeaway_body(&mut cx, &ps.takeaway, pf.band.is_some()));
                let take_h = match (&take, &pf.band) {
                    (Some(tb), None) => text_h(tb, c.w),
                    _ => 0,
                };
                let bottom = c.y + c.h - if take_h > 0 { take_h + gap } else { 0 };

                // ── the text of the page (one size per role: never a denser tag) ──
                let t = if ps.role == "takeaway" { cx.tag(&["takeaway", "body"]) } else { cx.tag(&["body"]) };
                let mut paras = vec![];
                let num = ps.role == "takeaway";
                for b in &ps.bullets {
                    let runs = cx.rich(b, &t, None);
                    let mut p = cx.para(runs, &t);
                    p.bullet = if num { Bullet::Num { scheme: "arabicPeriod".into(), start: 1 } } else { Bullet::Char { ch: "•".into() } };
                    p.mar_l = EMU_IN / 4;
                    p.indent = -EMU_IN / 4;
                    p.after_pt = Some(6.0);
                    paras.push(p);
                }
                // equations of the slide: native OMML, one per line; one the paper's text
                // could not give is its crop (a picture above the text), never LaTeX as text
                let eq_tag = cx.tag(&["equation", "body"]);
                let mut eq_pics: Vec<&Img> = vec![];
                let mut used_keys: BTreeSet<String> = BTreeSet::new();
                for id in &ps.equations {
                    let Some(e) = m.equations.iter().find(|e| &e.id == id) else { continue };
                    let latex = cx.expand(&e.latex);
                    let size = cx.look(&eq_tag, None).size;
                    let omml = if e.usable() { cx.omml(&latex, true, size) } else { None };
                    let mut p = match omml {
                        Some(omml) => {
                            let look = cx.look(&eq_tag, None);
                            let tag = cx.st.tag("equation").map(|_| "equation".to_string());
                            used_keys.extend(equation_keys(&latex));
                            cx.para(vec![Run { kind: "math".into(), text: latex.clone(), tag, em: None, look, field: None, omml: Some(omml), latex: Some(latex), src: None, k: 0 }], &eq_tag)
                        }
                        None => {
                            match e.image.as_ref() {
                                Some(im) => eq_pics.push(im),
                                None => cx.warnings.push(format!("{warn_at}：{} 無法排版，沒有放", e.label.clone().unwrap_or_else(|| e.id.clone()))),
                            }
                            continue;
                        }
                    };
                    p.align = "ctr".into();
                    p.before_pt = Some(6.0);
                    p.after_pt = Some(6.0);
                    paras.push(p);
                }
                // the symbols the slide's equations use, once each, from the paper's
                // registry, as notes under them (§7.1: 符號說明)
                let note_tag = cx.tag(&["caption", "body"]);
                let em = cx.st.pack.emphasis.contains_key("note").then_some("note");
                for line in symbol_notes(&m.symbols, &used_keys, inp.lang) {
                    let runs = cx.rich(&line, &note_tag, em);
                    let mut p = cx.para(runs, &note_tag);
                    p.align = "ctr".into();
                    paras.push(p);
                }
                let text_body = (!paras.is_empty()).then(|| body(paras, "t"));

                // ── the picture or table and where the text goes ──
                let mut step_ids: Vec<Vec<u32>> = vec![];
                // top of what is still free in the body area, and the text's frame
                let mut y = c.y;
                let mut text_f = Frame::new(c.x, c.y, c.w, (bottom - c.y).max(h / 10));
                let mut low = c.y; // the lowest edge of what was placed
                if let Some(grid) = &table_grid {
                    let o = table_obj(&mut cx, &Frame::new(c.x, c.y, c.w, bottom - c.y), grid, kv.is_some());
                    y = o.frame.y + o.frame.h + gap;
                    low = o.frame.y + o.frame.h;
                    if low > bottom {
                        cx.warnings.push(format!("{warn_at}：表格有 {} 列，超出版面，請刪列或拆頁", grid.len()));
                    }
                    extra.push(o);
                    text_f = Frame::new(c.x, y, c.w, (bottom - y).max(h / 10));
                } else if let Some(img) = image {
                    let im = img.image.as_ref().unwrap();
                    let asset = match assets.get(&img.id) {
                        Some(a) => a.clone(),
                        None => {
                            let a = sc_core::store_asset(&cx.deck_dir, &im.bytes, &im.ext).map_err(|e| e.to_string())?;
                            assets.insert(img.id.clone(), a.clone());
                            a
                        }
                    };
                    let cap_tag = cx.tag(&["caption"]);
                    let cap = strip_typed_refs(&caption_clip(&img.caption, 140));
                    let aspect = im.w.max(1) as f64 / im.h.max(1) as f64;
                    let steps = !ps.steps.is_empty();
                    // a wide picture gets the width (text under it); a tall one sits beside the text
                    let side = steps || (has_text && aspect < 1.4);
                    let area = if side {
                        let b = block("image").unwrap_or_else(|| Frame::new(c.x, c.y, c.w * 48 / 100, c.h));
                        Frame::new(b.x, c.y, b.w, bottom - c.y)
                    } else {
                        Frame::new(c.x, c.y, c.w, bottom - c.y)
                    };
                    let cap_body = (!cap.is_empty()).then(|| {
                        let runs = cx.rich(&cap, &cap_tag, None);
                        body(vec![cx.para(runs, &cap_tag)], "t")
                    });
                    let cap_h = cap_body.as_ref().map(|b| text_h(b, area.w) + h / 120).unwrap_or(0);
                    let below = if side { 0 } else { text_body.as_ref().map(|b| text_h(b, c.w) + gap).unwrap_or(0) };
                    // the picture: as wide as the room under it allows, never under 80 % of the width when it is on top
                    let room = (area.h - cap_h - below).max(h / 10);
                    let mut pw = area.w.min((room as f64 * aspect) as i64);
                    if !side && has_text {
                        pw = pw.max(area.w * 8 / 10);
                    }
                    let ph = (pw as f64 / aspect) as i64;
                    let pframe = Frame::new(area.x + (area.w - pw) / 2, area.y, pw, ph);
                    let is_main = img.id == main_fig && !inp.boxes.is_empty();
                    let module = ps.module as usize;
                    if is_main && ps.role == "method" && module >= 1 && module <= inp.boxes.len() {
                        // the whole figure dimmed, the current module cropped back on top, framed
                        let ghost = inp.st.pack.ghost.picture_alpha.clamp(0.1, 1.0);
                        extra.push(pic("架構圖", pframe.clone(), &asset, Crop::default(), ghost, Some("image-1")));
                        let (sub, crop) = module_rect(&pframe, im, &inp.boxes[module - 1].bbox);
                        extra.push(pic(&format!("模組 {module}"), sub.clone(), &asset, crop, 1.0, None));
                        extra.push(frame_obj(&cx, &sub, &format!("框 {module}")));
                    } else {
                        extra.push(pic(if fig.is_some() { "圖" } else { "表" }, pframe.clone(), &asset, Crop::default(), 1.0, Some("image-1")));
                        if is_main && ps.role == "overview" {
                            for (i, b) in inp.boxes.iter().enumerate() {
                                let (sub, _) = module_rect(&pframe, im, &b.bbox);
                                let mut o = frame_obj(&cx, &sub, &format!("框 {}", i + 1));
                                o.name = format!("step-frame-{}", i + 1);
                                extra.push(o);
                            }
                        }
                    }
                    low = pframe.y + pframe.h;
                    if let Some(cb) = cap_body {
                        // the caption: right under the picture, as wide as the picture's column
                        let cf = Frame::new(area.x, pframe.y + pframe.h + h / 120, area.w, cap_h - h / 120);
                        low = cf.y + cf.h;
                        extra.push(text_obj("圖說", cf, cb, Some("caption-1")));
                    }
                    if side {
                        let b = block("text").unwrap_or_else(|| Frame::new(c.x + c.w * 52 / 100, c.y, c.w * 48 / 100, c.h));
                        text_f = Frame::new(b.x, c.y, b.w, (bottom - c.y).max(h / 10));
                    } else {
                        y = low + gap;
                        text_f = Frame::new(c.x, y, c.w, (bottom - y).max(h / 10));
                    }
                }
                let _ = y;
                if has_text {
                    let t = cx.tag(&["body"]);
                    let mut tf = text_f.clone();
                    if !ps.steps.is_empty() {
                        // one text box per numbered step (the reveal steps)
                        let n = ps.steps.len() as i64;
                        let sh = ((tf.h - gap * (n - 1)) / n).min(h / 7);
                        for (i, s) in ps.steps.iter().enumerate() {
                            let label = cx.tag(&["label"]);
                            let mut runs = vec![cx.run(&format!("{}  ", i + 1), &label, None)];
                            runs.extend(cx.rich(s, &t, None));
                            let mut o = text_obj(&format!("步驟 {}", i + 1), Frame::new(tf.x, tf.y + i as i64 * (sh + gap), tf.w, sh), body(vec![cx.para(runs, &t)], "t"), None);
                            o.name = format!("step-{}", i + 1);
                            low = low.max(o.frame.y + o.frame.h);
                            extra.push(o);
                        }
                        step_ids = (0..ps.steps.len()).map(|_| vec![]).collect();
                        let used = tf.y + n * (sh + gap);
                        tf = Frame::new(tf.x, used, tf.w, (tf.y + tf.h - used).max(h / 10));
                    }
                    // equation crops: stacked at the top of the text area, the text below
                    if !eq_pics.is_empty() {
                        let share = if text_body.is_none() { tf.h } else { tf.h * 3 / 5 };
                        let each = ((share - gap * (eq_pics.len() as i64 - 1)) / eq_pics.len() as i64).max(h / 12);
                        let mut ey = tf.y;
                        for (k, im) in eq_pics.iter().enumerate() {
                            let asset = sc_core::store_asset(&cx.deck_dir, &im.bytes, &im.ext).map_err(|e| e.to_string())?;
                            let ef = contain(&Frame::new(tf.x, ey, tf.w, each), im.w, im.h);
                            extra.push(pic(&format!("方程式 {}", k + 1), Frame::new(ef.x, ey, ef.w, ef.h), &asset, Crop::default(), 1.0, None));
                            ey += ef.h + gap;
                        }
                        low = low.max(ey - gap);
                        tf = Frame::new(tf.x, ey, tf.w, (tf.y + tf.h - ey).max(h / 10));
                    }
                    if let Some(tb) = text_body {
                        let need = text_h(&tb, tf.w);
                        if need > tf.h + h / 100 {
                            cx.warnings.push(format!("{warn_at}：文字放不下（沒有縮字），請刪短或拆頁"));
                        }
                        let f = Frame::new(tf.x, tf.y, tf.w, need.min(tf.h).max(h / 20));
                        low = low.max(f.y + f.h);
                        extra.push(text_obj("內文", f, tb, Some("text-1")));
                    }
                }
                if let Some(tb) = take {
                    let f = match &pf.band {
                        Some(b) => b.clone(),
                        None => Frame::new(c.x, (low + gap).min(c.y + c.h - take_h).max(c.y), c.w, take_h),
                    };
                    extra.push(takeaway_obj(&cx, &f, tb, pf.band.is_some()));
                }
                if !ps.cites.is_empty() {
                    let refs: Vec<String> = ps.cites.iter().filter_map(|c| m.refs.get(c)).cloned().collect();
                    if let Some(o) = ref_obj(&mut cx, &refs, &pf.refs, &warn_at) {
                        extra.push(o);
                    }
                }
                if !step_ids.is_empty() {
                    reveal_steps = Some((out.len(), step_ids));
                }
            }
        }
        // ids after the base's objects
        let mut next = objs.iter().map(|o| o.id).max().unwrap_or(1) + 1;
        for mut o in extra {
            o.id = next;
            next += 1;
            objs.push(o);
        }
        // the reveal steps: step text i and its frame on the figure
        if let Some((idx, steps)) = &mut reveal_steps
            && *idx == out.len()
        {
            for (i, s) in steps.iter_mut().enumerate() {
                for o in &objs {
                    if o.name == format!("step-{}", i + 1) || o.name == format!("step-frame-{}", i + 1) {
                        s.push(o.id);
                    }
                }
            }
        }
        slide.objects = objs;
        out.push(slide);
        (inp.progress)(out.len(), plan.slides.len());
    }
    Ok(Built { slides: out, reveal: reveal_steps.filter(|(_, s)| s.len() >= 2), warnings: cx.warnings, content: pf.content })
}

/// The module's rectangle on the slide and the crop that shows only it.
fn module_rect(pf: &Frame, im: &Img, b: &[f64; 4]) -> (Frame, Crop) {
    let (iw, ih) = (im.w.max(1) as f64, im.h.max(1) as f64);
    let (x0, y0, x1, y1) = (b[0].clamp(0.0, iw), b[1].clamp(0.0, ih), b[2].clamp(0.0, iw), b[3].clamp(0.0, ih));
    let sub = Frame::new(pf.x + (pf.w as f64 * x0 / iw) as i64, pf.y + (pf.h as f64 * y0 / ih) as i64, (pf.w as f64 * (x1 - x0) / iw) as i64, (pf.h as f64 * (y1 - y0) / ih) as i64);
    (sub, Crop { l: x0 / iw, t: y0 / ih, r: 1.0 - x1 / iw, b: 1.0 - y1 / ih })
}

/// The emphasis frame around the current part (design §7.1: a red 2.2 pt frame).
fn frame_obj(cx: &Ctx, f: &Frame, name: &str) -> Object {
    let c = cx.st.pack.emphasis.get("key").and_then(|e| e.color.clone()).map(|c| c.to_uppercase()).unwrap_or_else(|| cx.color(&["emphasis", "accent-1", "red", "ink"]));
    Object {
        id: 0,
        name: name.into(),
        frame: f.clone(),
        hidden: false,
        ph: None,
        slot: None,
        kind: Kind::Shape(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: Some(Stroke::new(&c, 2.25)), text: None, equation: false }),
        src: None,
    }
}

/// The page's title into the layout's title placeholder (else a box at `fallback`). The text
/// keeps its tag's size: nothing shrinks it (`autofit` off).
fn fill_title(objs: &mut Vec<Object>, cx: &Ctx, text: &str, tag: &str, fallback: Frame) {
    let para = cx.para(vec![cx.run(text, tag, None)], tag);
    let slot = objs.iter_mut().find(|o| o.slot.as_deref() == Some("title") || matches!(o.ph.as_deref(), Some("title") | Some("ctrTitle")));
    match slot.and_then(|o| o.kind.sp_mut()) {
        Some(sp) => {
            let tb = sp.text.get_or_insert_with(TextBody::default);
            tb.paras = vec![para];
            tb.autofit = "none".into();
        }
        None => objs.push(text_obj("Title", fallback, body(vec![para], "ctr"), Some("title"))),
    }
}

/// `Ref:` small, beside the logo (tag `source`, else `reference`, else `caption`): whole
/// entries only, as many as the box's lines hold — an entry that does not fit is left out
/// and reported, never cut in the middle.
fn ref_obj(cx: &mut Ctx, refs: &[String], f: &Frame, at: &str) -> Option<Object> {
    let t = cx.tag(&["source", "reference", "caption"]);
    let make = |cx: &Ctx, list: &[&str]| body(vec![cx.para(vec![cx.run(&format!("Ref: {}", list.join("; ")), &t, None)], &t)], "b");
    let mut kept: Vec<&str> = vec![];
    for r in refs {
        let r = r.trim().trim_end_matches(';');
        if r.is_empty() {
            continue;
        }
        let mut next = kept.clone();
        next.push(r);
        if crate::deckpolish::text_size(&make(cx, &next), f.w).1 <= REF_LINES as f64 {
            kept = next;
        }
    }
    let total = refs.iter().filter(|r| !r.trim().is_empty()).count();
    if kept.len() < total {
        cx.warnings.push(format!("{at}：參考文獻放得下 {} 筆（共 {total} 筆），其餘沒有放", kept.len()));
    }
    if kept.is_empty() {
        return None;
    }
    Some(text_obj("Ref", f.clone(), make(cx, &kept), None))
}

fn takeaway_body(cx: &mut Ctx, text: &str, band: bool) -> TextBody {
    if band {
        let t = cx.tag(&["takeaway", "label"]);
        let runs = cx.rich(text, &t, None);
        let mut tb = body(vec![cx.para(runs, &t)], "ctr");
        tb.insets = [228_600, 45_720, 228_600, 45_720];
        return tb;
    }
    // no band in the base: one line in the label tag with the key-result emphasis
    let t = cx.tag(&["label", "body"]);
    let em = cx.st.pack.emphasis.contains_key("key").then_some("key");
    let runs = cx.rich(text, &t, em);
    body(vec![cx.para(runs, &t)], "t")
}

fn takeaway_obj(cx: &Ctx, f: &Frame, tb: TextBody, band: bool) -> Object {
    if band {
        let fill = cx.color(&["inverse"]);
        let mut g = Geom::preset("roundRect");
        g.adj.insert("adj".into(), 12000.0);
        return Object {
            id: 0,
            name: "takeaway".into(),
            frame: f.clone(),
            hidden: false,
            ph: None,
            slot: Some("takeaway-1".into()),
            kind: Kind::Shape(SpObj { geom: g, fill: Paint::solid(&fill), line: None, text: Some(tb), equation: false }),
            src: None,
        };
    }
    text_obj("takeaway", f.clone(), tb, Some("takeaway-1"))
}

/// A native table (無樣式、表格格線) across the body width: first row bold as header;
/// key-value tables have no header and stay narrow. Columns share the width by how much
/// text they hold; every table uses the `table` tag's one size.
fn table_obj(cx: &mut Ctx, f: &Frame, grid: &[Vec<String>], kv: bool) -> Object {
    let t = cx.tag(&["table", "body"]);
    let size = cx.look(&t, None).size;
    let ncol = grid.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let nrow = grid.len().max(1) as i64;
    let rh = ((size * 1.2 * 12_700.0) as i64 + 2 * 45_720).max(EMU_IN / 4);
    let cell_text = |r: &Vec<String>, ci: usize| r.get(ci).map(|s| strip_typed_refs(&lore_docsrc::latex::plain::tidy(&lore_notation::pending::to_plain(s)))).unwrap_or_default();
    let tw = if kv { f.w.min(ncol as i64 * EMU_IN * 13 / 5) } else { f.w };
    let cols: Vec<i64> = if kv && ncol == 2 {
        vec![tw * 2 / 5, tw - tw * 2 / 5]
    } else {
        // by the longest cell of each column, within 0.6–2.4 × an even share
        let longest: Vec<f64> = (0..ncol).map(|ci| grid.iter().map(|r| cell_text(r, ci).chars().count()).max().unwrap_or(1).max(3) as f64).collect();
        let even = longest.iter().sum::<f64>() / ncol as f64;
        let wts: Vec<f64> = longest.iter().map(|l| l.clamp(even * 0.6, even * 2.4)).collect();
        let sum: f64 = wts.iter().sum();
        let mut v: Vec<i64> = wts.iter().map(|x| (tw as f64 * x / sum) as i64).collect();
        let used: i64 = v.iter().sum();
        if let Some(l) = v.last_mut() {
            *l += tw - used;
        }
        v
    };
    let mut rows: Vec<Row> = vec![];
    for (ri, r) in grid.iter().enumerate() {
        let mut cells = vec![];
        for ci in 0..ncol {
            let text = cell_text(r, ci);
            let head = (!kv && ri == 0) || (kv && ci == 0);
            let em = head.then_some("strong").filter(|e| cx.st.pack.emphasis.contains_key(*e));
            // `$…$` in a cell is native math, like in the bullets
            let mut runs = cx.rich(&text, &t, em);
            if runs.is_empty() {
                runs.push(cx.run("", &t, em));
            }
            if head && em.is_none() {
                runs.iter_mut().for_each(|r| r.look.bold = true);
            }
            let mut tb = body(vec![cx.para(runs, &t)], "ctr");
            tb.insets = [91_440, 45_720, 91_440, 45_720];
            cells.push(Cell { text: tb, span: 1, rspan: 1, hmerge: false, vmerge: false, fill: None, borders: [None, None, None, None], src: None });
        }
        rows.push(Row { h: rh, cells, src: None });
    }
    let height = rh * nrow;
    Object {
        id: 0,
        name: if kv { "訓練細節".into() } else { "表".into() },
        frame: Frame::new(f.x, f.y, cols.iter().sum(), height),
        hidden: false,
        ph: None,
        slot: Some("table-1".into()),
        kind: Kind::Table(TableObj { cols, rows, style_id: Some(TABLE_GRID.into()), first_row: !kv, band_row: false }),
        src: None,
    }
}
