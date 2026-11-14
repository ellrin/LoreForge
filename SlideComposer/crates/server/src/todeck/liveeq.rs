//! 會動的方程式 in a paper report (`../docs/living-equations.md`): the paper's main loss on the
//! method slides gets an animated GIF beside it (one per deck; 微調 → 會動的方程式 不要 turns it off).
//!
//! Only when it is safe with nobody looking: the main loss (a `\mathcal{L}` equation on a
//! loss / method slide) goes to the shared `equation-plot` task with the paper's symbols and the
//! text around it (method, ablation and implementation details); the answer must be a catalogue
//! template, animate the parameter the paper itself tunes (its value from the paper) and pass
//! every check (`lore_plot::equation::propose::fit_for_auto`). Anything else is skipped with the
//! reason in the deck's warnings — never a generic curve. Best-effort: nothing here fails the job.
//! The GIF carries its spec (SlideComposer's 編輯 re-opens it), frames are drawn by the same
//! `plot.js` in headless Chrome ([`ChromeRender`]; tests use a fake [`Render`]).

use std::sync::Arc;

use lore_canvas::{Crop, Frame, Kind, Object, PicObj, Slide};
use lore_plot::equation::{self, propose};
use serde_json::{Value, json};

use super::plan::Plan;
use super::source::Material;
use crate::AppState;

/// Frames of a deck's GIF.
pub const FRAMES: usize = 24;
/// The deck GIF's figure size (mm): wide, for the text slot under a method slide's equation.
pub const DECK_MM: (f64, f64) = (220.0, 95.0);

/// Draws an equation figure's animation.
pub trait Render: Send + Sync {
    /// The GIF (spec inside) of `spec_json`, `macros` = the deck's registry macros for KaTeX.
    fn gif(&self, spec_json: &str, macros: &Value) -> Result<Vec<u8>, String>;
}

/// The real one: headless Chrome draws the frames (`lore_plot::export::render_equation_frames`),
/// poppler or Chrome rasterises them. **Not tested on real hardware in the offline tests.**
pub struct ChromeRender {
    pub home: lore_home::Home,
}

impl Render for ChromeRender {
    fn gif(&self, spec_json: &str, macros: &Value) -> Result<Vec<u8>, String> {
        let tools = lore_plot::Tools::find();
        let styles = lore_plot::style::list(&self.home)["styles"].clone();
        // paced by visual change, the turning points held (the same frames as the editor's GIF)
        let f = lore_plot::export::render_equation_gif_frames(&tools, spec_json, &styles, macros).map_err(|e| e.to_string())?;
        let ms = serde_json::from_str::<Value>(spec_json).ok().and_then(|v| v["panels"][0]["eq"]["animate"]["ms"].as_u64()).unwrap_or(80) as u32;
        let opts = equation::gif::Opts { delays: f.delays, dither: f.dither };
        equation::raster::gif_from_svgs_with(&tools, &f.frames, 200, f.max_w.unwrap_or(equation::raster::MAX_W_PX), ms, Some(spec_json), &opts).map_err(|e| e.to_string())
    }
}

/// The GIF for one slide of the plan.
#[derive(Debug, Clone)]
pub struct EqGif {
    /// Index in `plan.slides` (= the built slide before the big picture is expanded).
    pub slide: usize,
    pub eq_id: String,
    pub gif: Vec<u8>,
    /// The figure's size in mm (the picture keeps its aspect).
    pub mm: (f64, f64),
    /// Each variable's LaTeX and colour (its role on the deck style's palette, as in the GIF),
    /// and the function's colour (the equation's left-hand side).
    pub inks: Vec<(String, String)>,
    pub out: String,
}

/// The palette the GIF used: the plot style's line colours, else its palette (a list or a name
/// in palettes.json), following `base`; the root palette when nothing says.
pub fn style_palette(home: &lore_home::Home, id: &str) -> Vec<String> {
    let list = lore_plot::style::list(home);
    let styles = list["styles"].as_array().cloned().unwrap_or_default();
    let pals: Value = lore_plot::assets::get("palettes.json").and_then(|(_, b)| serde_json::from_slice(b).ok()).unwrap_or_default();
    let colours = |v: &Value| -> Option<Vec<String>> {
        match v {
            Value::Array(a) if !a.is_empty() => Some(a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()),
            Value::String(n) => pals[n.as_str()]["colors"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()),
            _ => None,
        }
    };
    let mut cur = styles.iter().find(|s| s["id"] == id);
    for _ in 0..8 {
        let Some(s) = cur else { break };
        if let Some(c) = colours(&s["colors"]["lines"]).or_else(|| colours(&s["colors"]["palette"])) {
            return c;
        }
        cur = s["base"].as_str().and_then(|b| styles.iter().find(|x| x["id"] == b));
    }
    ["#0072B2", "#E69F00", "#009E73", "#D55E00", "#CC79A7", "#56B4E9"].map(String::from).to_vec()
}

/// A symbol's spellings in LaTeX (`p_t` ⇄ `p_{t}`).
fn spellings(tex: &str) -> Vec<String> {
    let t: String = tex.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = vec![t.clone()];
    if let Some((a, b)) = t.split_once('_') {
        let inner = b.trim_start_matches('{').trim_end_matches('}');
        for s in [format!("{a}_{inner}"), format!("{a}_{{{inner}}}")] {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// The equation's LaTeX with each variable (and the left-hand side) in its colour (`\textcolor`).
pub fn colour_latex(latex: &str, inks: &[(String, String)], out: &str) -> String {
    let (lhs, rhs) = match latex.split_once('=') {
        Some((l, r)) if !l.contains("\\begin") => (Some(l), r),
        _ => (None, latex),
    };
    let mut body: String = rhs.to_string();
    let mut keep: Vec<String> = vec![];
    let mut list: Vec<(String, &str)> = inks.iter().flat_map(|(t, c)| spellings(t).into_iter().map(move |s| (s, c.as_str()))).filter(|(s, _)| !s.is_empty()).collect();
    list.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (sp, c) in list {
        let mut res = String::new();
        let mut rest = body.as_str();
        while let Some(i) = rest.find(&sp) {
            let after = &rest[i + sp.len()..];
            // `\gamma` must not match inside `\gammax`
            if after.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic()) && sp.chars().last().is_some_and(|ch| ch.is_ascii_alphabetic()) {
                res.push_str(&rest[..i + sp.len()]);
                rest = after;
                continue;
            }
            res.push_str(&rest[..i]);
            keep.push(format!("\\textcolor{{{c}}}{{{sp}}}"));
            res.push_str(&format!("\u{1}{}\u{2}", keep.len() - 1));
            rest = after;
        }
        res.push_str(rest);
        body = res;
    }
    for (k, v) in keep.iter().enumerate().rev() {
        body = body.replace(&format!("\u{1}{k}\u{2}"), v);
    }
    match lhs {
        Some(l) => format!("\\textcolor{{{out}}}{{{}}}={body}", l.trim_end()),
        None => body,
    }
}

/// The glyphs a symbol's base letter becomes in OMML (plain and math italic).
fn glyphs(tex: &str) -> Vec<String> {
    let t = tex.trim();
    if t.starts_with("\\mathcal{L}") || t.starts_with("\\mathcal L") {
        return vec!["ℒ".into(), "𝓛".into()];
    }
    let base = t.split(['_', '^']).next().unwrap_or("").trim_matches(|c| c == '{' || c == '}');
    const GREEK: &[(&str, char)] = &[("alpha", 'α'), ("beta", 'β'), ("gamma", 'γ'), ("delta", 'δ'), ("epsilon", 'ε'), ("varepsilon", 'ε'), ("zeta", 'ζ'), ("eta", 'η'), ("theta", 'θ'), ("lambda", 'λ'), ("mu", 'μ'), ("nu", 'ν'), ("xi", 'ξ'), ("pi", 'π'), ("rho", 'ρ'), ("sigma", 'σ'), ("tau", 'τ'), ("phi", 'φ'), ("chi", 'χ'), ("psi", 'ψ'), ("omega", 'ω')];
    if let Some(name) = base.strip_prefix('\\') {
        return GREEK.iter().find(|g| g.0 == name).map(|(_, c)| {
            let it = char::from_u32(0x1D6FC + (*c as u32 - 0x3B1)).unwrap_or(*c);
            vec![c.to_string(), it.to_string()]
        }).unwrap_or_default();
    }
    let mut cs = base.chars();
    match (cs.next(), cs.next()) {
        (Some(c), None) if c.is_ascii_lowercase() => {
            let it = if c == 'h' { '\u{210E}' } else { char::from_u32(0x1D44E + (c as u32 - 'a' as u32)).unwrap_or(c) };
            vec![c.to_string(), it.to_string()]
        }
        (Some(c), None) if c.is_ascii_uppercase() => vec![c.to_string(), char::from_u32(0x1D434 + (c as u32 - 'A' as u32)).unwrap_or(c).to_string()],
        _ => vec![],
    }
}

/// PowerPoint OMML with the runs of these glyphs coloured (`a:solidFill` first in their `a:rPr`).
pub fn colour_omml(omml: &str, inks: &[(Vec<String>, String)]) -> String {
    let mut out = String::with_capacity(omml.len() + 256);
    let mut rest = omml;
    while let Some(i) = rest.find("<m:r>") {
        let Some(j) = rest[i..].find("</m:r>").map(|j| i + j + "</m:r>".len()) else { break };
        out.push_str(&rest[..i]);
        let mut seg = rest[i..j].to_string();
        let text = seg.split_once("<m:t>").and_then(|(_, t)| t.split_once("</m:t>")).map(|(t, _)| t.to_string()).unwrap_or_default();
        if let Some((_, hex)) = inks.iter().find(|(g, _)| g.iter().any(|x| *x == text)) {
            let fill = format!("<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>", hex.trim_start_matches('#').to_uppercase());
            if let Some(a) = seg.find("<a:rPr") {
                let end = a + seg[a..].find('>').unwrap_or(0);
                if seg[..end].ends_with('/') {
                    seg.replace_range(end - 1..=end, &format!(">{fill}</a:rPr>"));
                } else {
                    seg.insert_str(end + 1, &fill);
                }
            }
        }
        out.push_str(&seg);
        rest = &rest[j..];
    }
    out.push_str(rest);
    out
}

/// The slide's equations (the text box's maths) coloured like the GIF: LaTeX for the editor
/// (KaTeX), OMML run colours for the .pptx.
pub fn colour_slide_equations(slide: &mut Slide, inks: &[(String, String)], out: &str) {
    let mut g: Vec<(Vec<String>, String)> = inks.iter().map(|(t, c)| (glyphs(t), c.clone())).filter(|(g, _)| !g.is_empty()).collect();
    g.push((vec!["ℒ".into(), "𝓛".into()], out.to_string()));
    for o in slide.objects.iter_mut().filter(|o| o.name == "內文") {
        let Kind::Text(sp) = &mut o.kind else { continue };
        let Some(body) = sp.text.as_mut() else { continue };
        for p in &mut body.paras {
            for r in p.runs.iter_mut().filter(|r| r.kind == "math") {
                if let Some(l) = r.latex.as_mut() {
                    *l = colour_latex(l, inks, out);
                }
                if let Some(m) = r.omml.as_mut() {
                    *m = colour_omml(m, &g);
                }
            }
        }
    }
}

/// How loss-like an equation is: 0 = not a loss.
pub fn loss_score(latex: &str) -> u32 {
    let lhs = latex.split_once('=').map(|(l, _)| l).unwrap_or("");
    let rhs = latex.split_once('=').map(|(_, r)| r).unwrap_or(latex);
    let low = lhs.to_lowercase();
    let mut s = 0;
    if lhs.contains("\\mathcal{L}") || lhs.contains("\\mathcal L") || low.contains("loss") || lhs.trim_start().starts_with("L_") || lhs.trim() == "L" {
        s += 2;
        let sub = low.contains("total") || low.contains("all") || low.contains("overall") || !lhs.contains('_');
        if sub {
            s += 1;
        }
    }
    if s > 0 && rhs.contains("\\log") {
        s += 1;
    }
    s
}

/// The paper's main loss on the planned slides: (plan slide index, equation id). Loss slides
/// first, then method slides; the most loss-like equation wins; none = no loss shown.
pub fn main_loss(plan: &Plan, m: &Material) -> Option<(usize, String)> {
    let mut best: Option<(u32, usize, String)> = None;
    for (i, ps) in plan.slides.iter().enumerate() {
        let bonus = match ps.role.as_str() {
            "loss" => 2,
            "method" | "overview" => 0,
            _ => continue,
        };
        for id in &ps.equations {
            let Some(e) = m.equations.iter().find(|e| &e.id == id) else { continue };
            if !e.usable() {
                continue;
            }
            let sc = loss_score(&e.latex);
            if sc == 0 {
                continue;
            }
            let sc = sc + bonus;
            if best.as_ref().is_none_or(|b| sc > b.0) {
                best = Some((sc, i, id.clone()));
            }
        }
    }
    best.map(|(_, i, id)| (i, id))
}

/// What the model reads: the paper's symbols and the text that says how the loss is used and tuned.
pub fn ask_for(m: &Material, eq_id: &str) -> propose::Ask {
    let e = m.equations.iter().find(|e| e.id == eq_id);
    let latex = e.map(|e| e.latex.clone()).unwrap_or_default();
    let mut symbols: Vec<String> = m.symbols.iter().map(|s| format!("${}$: {}", s.latex, s.meaning_en.clone().or_else(|| s.meaning.clone()).unwrap_or_default())).collect();
    for n in &m.notation.entries {
        let Some(tex) = n.latex.as_deref().filter(|t| !t.is_empty()) else { continue };
        let row = format!("${tex}$: {}", n.meaning.as_deref().unwrap_or(""));
        if !symbols.contains(&row) {
            symbols.push(row);
        }
    }
    // the section with the equation, then the ones about tuning (ablation, settings)
    let label = e.and_then(|e| e.label.clone()).unwrap_or_default();
    let mut context = String::new();
    let tuning = |t: &str| {
        let t = t.to_lowercase();
        ["ablation", "hyper", "sensitiv", "implementation", "setting", "training detail", "parameter"].iter().any(|k| t.contains(k))
    };
    for (title, text) in &m.sections {
        let has_eq = (!label.is_empty() && text.contains(&label)) || text.contains(latex.get(..latex.len().min(24)).unwrap_or(""));
        if has_eq {
            context += &format!("[{title}] {}\n", text.chars().take(1500).collect::<String>());
        }
    }
    for (title, text) in &m.sections {
        if tuning(title) || tuning(text.get(..text.len().min(400)).unwrap_or("")) {
            context += &format!("[{title}] {}\n", text.chars().take(1200).collect::<String>());
        }
    }
    propose::Ask { latex, symbols, context: context.chars().take(4000).collect() }
}

/// Propose, gate, draw. `Ok(None)` + a reason when skipped.
pub async fn prepare(s: &Arc<AppState>, plan: &Plan, m: &Material, macros: &Value, engine: Option<&str>, lang: &str) -> (Option<EqGif>, Option<String>) {
    let Some((slide, eq_id)) = main_loss(plan, m) else { return (None, None) };
    let ask = ask_for(m, &eq_id);
    let llm = EqLlm { s: s.clone(), engine: engine.map(String::from) };
    let p = match propose::propose(&llm, &ask).await {
        Ok(p) => p,
        Err(f) => return (None, Some(format!("會動的方程式沒有放：{}", f.reason))),
    };
    if let Err(why) = propose::fit_for_auto(&p, &ask.context) {
        return (None, Some(format!("會動的方程式沒有放：{why}")));
    }
    let style = lore_plot::style::list(&s.home)["slide"].as_str().unwrap_or("slide").to_string();
    // colour roles fixed now (templates < the author's global notation < the paper's), so 編輯 later keeps them
    let mut eq = p.eq.clone();
    let global = s.home.shared_file("notation.json").and_then(|f| lore_notation::Notation::load(&f).ok());
    let layers: Vec<&lore_notation::Notation> = global.iter().chain(std::iter::once(&m.notation)).collect();
    equation::assign_roles(&mut eq, &layers);
    let mut spec = equation::spec_for(&eq, &style);
    spec["panels"][0]["eq"]["animate"]["frames"] = json!(FRAMES);
    // a method slide's text slot is wide and short: a wide figure fills its width (type sizes stay in pt)
    spec["size"] = json!({ "w": DECK_MM.0, "h": DECK_MM.1 });
    // the legend's 「（論文）」／"(paper)" follows the deck's language
    spec["panels"][0]["eq"]["lang"] = json!(if lang.starts_with("en") { "en" } else { "zh" });
    let spec_json = spec.to_string();
    let mm = spec["size"]["w"].as_f64().zip(spec["size"]["h"].as_f64()).unwrap_or(equation::SLIDE_MM);
    // the same roles on the same palette as the GIF, for the slide's own equation
    let pal = style_palette(&s.home, &style);
    let pick = |r: Option<u8>| pal[r.unwrap_or(0) as usize % pal.len()].clone();
    let mut inks: Vec<(String, String)> = vec![];
    if let Some(x) = &eq.x {
        inks.push((equation::label_tex(&x.label), pick(x.color)));
    }
    for v in eq.vars.iter().filter(|v| v.role == "param") {
        inks.push((equation::label_tex(&v.label), pick(v.color)));
    }
    inks.retain(|(t, _)| !t.is_empty());
    let out = pick(Some(0));
    let render: Arc<dyn Render> = s.liveeq.get().cloned().unwrap_or_else(|| Arc::new(ChromeRender { home: s.home.clone() }));
    let macros = macros.clone();
    match tokio::task::spawn_blocking(move || render.gif(&spec_json, &macros)).await {
        Ok(Ok(gif)) => (Some(EqGif { slide, eq_id, gif, mm, inks, out }), None),
        Ok(Err(e)) => (None, Some(format!("會動的方程式沒有放：{e}"))),
        Err(e) => (None, Some(format!("會動的方程式沒有放：{e}"))),
    }
}

struct EqLlm {
    s: Arc<AppState>,
    engine: Option<String>,
}

#[async_trait::async_trait]
impl lore_plot::propose::Llm for EqLlm {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(Value, String), String> {
        let a = super::llm(&self.s).complete_json(task, req, self.engine.as_deref()).await.map_err(|(m, _)| m)?;
        Ok((a.value, a.engine))
    }
}

/// Put the GIF on its slide: below the slide's text (the equation), the text box made shorter;
/// without a text box, the lower right of the slide. The picture keeps the figure's aspect.
pub fn place(slide: &mut Slide, asset: &str, mm: (f64, f64), size: (i64, i64), id: u32) {
    let (w, h) = size;
    let aspect = mm.1 / mm.0;
    let gap = h / 60;
    let text = slide.objects.iter_mut().find(|o| o.name == "內文" && matches!(o.kind, Kind::Text(_)));
    let frame = match text {
        Some(t) => {
            let tf = t.frame.clone();
            let mut gw = tf.w;
            let mut gh = (gw as f64 * aspect) as i64;
            let max_h = tf.h * 2 / 3;
            if gh > max_h {
                gh = max_h;
                gw = (gh as f64 / aspect) as i64;
            }
            t.frame.h = (tf.h - gh - gap).max(h / 10);
            Frame::new(tf.x + (tf.w - gw) / 2, tf.y + tf.h - gh, gw, gh)
        }
        None => {
            let gw = w * 2 / 5;
            let gh = (gw as f64 * aspect) as i64;
            Frame::new(w - gw - w / 25, h - gh - h / 10, gw, gh)
        }
    };
    slide.objects.push(Object {
        id,
        name: "方程式動畫".into(),
        frame,
        hidden: false,
        ph: None,
        slot: None,
        kind: Kind::Picture(PicObj { asset: asset.into(), crop: Crop::default(), alpha: 1.0, line: None, clip: "rect".into(), masks: vec![], effects: vec![] }),
        src: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slide_equation_takes_the_gifs_colours() {
        let inks = vec![("p_t".to_string(), "#E69F00".to_string()), ("\\gamma".to_string(), "#009E73".to_string()), ("\\alpha_t".to_string(), "#D55E00".to_string())];
        let l = colour_latex("\\mathcal{L}_{\\mathrm{FL}} = -\\alpha_{t} (1 - p_{t})^{\\gamma} \\log p_t \\gammax", &inks, "#0072B2");
        assert!(l.starts_with("\\textcolor{#0072B2}{\\mathcal{L}_{\\mathrm{FL}}}="), "{l}");
        assert!(l.contains("\\textcolor{#E69F00}{p_{t}}") && l.contains("\\textcolor{#E69F00}{p_t}") && l.contains("\\textcolor{#009E73}{\\gamma}") && l.contains("\\textcolor{#D55E00}{\\alpha_{t}}"), "{l}");
        assert!(l.contains("\\gammax"), "no match inside a longer command");
        assert_eq!(glyphs("p_t"), vec!["p", "𝑝"]);
        assert_eq!(glyphs("\\gamma"), vec!["γ", "𝛾"]);
        let omml = "<m:oMath><m:r><a:rPr lang=\"en-US\" i=\"1\"><a:latin typeface=\"Cambria Math\"/></a:rPr><m:t>𝛾</m:t></m:r><m:r><a:rPr sz=\"1800\"/><m:t>𝑥</m:t></m:r><m:r><a:rPr sz=\"1800\"/><m:t>𝑝</m:t></m:r></m:oMath>";
        let c = colour_omml(omml, &[(glyphs("\\gamma"), "#009E73".into()), (glyphs("p"), "#e69f00".into())]);
        assert!(c.contains("<a:rPr lang=\"en-US\" i=\"1\"><a:solidFill><a:srgbClr val=\"009E73\"/></a:solidFill><a:latin"), "{c}");
        assert!(c.contains("<a:rPr sz=\"1800\"><a:solidFill><a:srgbClr val=\"E69F00\"/></a:solidFill></a:rPr><m:t>𝑝"), "{c}");
        assert!(c.contains("<a:rPr sz=\"1800\"/><m:t>𝑥"), "other runs untouched");
    }

    #[test]
    fn losses_are_told_from_other_equations() {
        assert!(loss_score(r"\mathcal{L} = -\alpha_t (1-p_t)^\gamma \log p_t") >= 3);
        assert!(loss_score(r"\mathcal{L}_{total} = \mathcal{L}_{ce} + \lambda \mathcal{L}_{dice}") > loss_score(r"\mathcal{L}_{dice} = 1 - x"));
        assert_eq!(loss_score(r"h = \sigma(Wx + b)"), 0);
        assert_eq!(loss_score(r"y = \mathcal{L}"), 0, "the loss is on the left");
    }
}
