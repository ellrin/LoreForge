//! The main figure, explained in parts: a vision model LOOKS at the paper's figures
//! (lore-llm task `sc-figure-parts`: Opus, then Sonnet, never the local model), says which
//! one is the method's big picture, whether it should be walked through in parts and where
//! to cut it. The cuts are straight lines across the picture, moved to the blank gutter
//! nearest to where the model put them (`lore_docsrc::crop::snap_cut`). The author's 微調
//! answer 方法拆幾個模組 (2–5) fixes the number of parts.
//!
//! The slides (`build.rs`) show the figure across the body width: the part being explained
//! clear, the rest faint (the style's ghost alpha), its lines appearing one by one over the
//! faint side. A call that fails is asked once more, then the figure is cut into equal
//! parts — with a warning, never silently.

use lore_docsrc::crop::{Axis, fit_png, ink_profile, snap_cut};
use lore_llm::{Image, LlmRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::source::{Fig, Material};
use crate::llm::Llm;

pub const TASK: &str = "sc-figure-parts";
/// Long side of a figure as the model sees it.
const SIDE: u32 = 1568;
pub const MAX_PARTS: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    /// A few words, as printed in the figure where possible.
    pub label: String,
    /// One sentence: what this part does.
    pub about: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FigureParts {
    /// The main figure's id (`""`: the paper has no figure to walk through).
    pub figure: String,
    /// Explain it in parts (else the figure is shown whole, once).
    pub split: bool,
    /// `x`: vertical cuts (parts left → right); `y`: horizontal cuts (top → bottom).
    pub axis: String,
    /// `parts.len() - 1` ascending fractions of the picture's width (or height).
    pub cuts: Vec<f64>,
    pub parts: Vec<Part>,
    /// For each method slide the request listed: its part (0-based).
    pub slides: Vec<usize>,
    /// `model` (a model looked at the figure) or `equal` (the fallback split).
    pub by: String,
    /// The model's one-line reason.
    pub why: String,
}

impl FigureParts {
    pub fn walk(&self) -> bool {
        self.split && !self.figure.is_empty() && self.parts.len() >= 2 && self.cuts.len() + 1 == self.parts.len()
    }

    /// `[from, to]` of part `i` along the axis (fractions).
    pub fn span(&self, i: usize) -> (f64, f64) {
        let a = if i == 0 { 0.0 } else { self.cuts.get(i - 1).copied().unwrap_or(0.0) };
        let b = self.cuts.get(i).copied().unwrap_or(1.0);
        (a, b.max(a))
    }

    /// `n` equal parts of `figure`, left to right (wide picture) or top to bottom.
    pub fn equal(f: &Fig, n: usize) -> FigureParts {
        let n = n.clamp(2, MAX_PARTS);
        let wide = f.image.as_ref().is_none_or(|im| im.w >= im.h);
        FigureParts {
            figure: f.id.clone(),
            split: true,
            axis: if wide { "x" } else { "y" }.into(),
            cuts: (1..n).map(|i| i as f64 / n as f64).collect(),
            parts: (1..=n).map(|i| Part { label: format!("Part {i}"), about: String::new() }).collect(),
            slides: vec![],
            by: "equal".into(),
            why: String::new(),
        }
    }
}

const SYSTEM: &str = "You look at the figures of a research paper for the person who will present the paper in a seminar. \
Find the main method figure: the architecture, pipeline or flow diagram that shows how the whole method works (not a result plot, not a photo, not a table). \
Then decide whether the presenter should explain it in parts: a diagram with several stages or modules is walked through part by part, \
each part shown clearly while the rest is faint; a simple figure is shown whole.\n\n\
The parts are cut by straight lines across the whole picture: axis \"x\" = vertical cut lines (parts from left to right), axis \"y\" = horizontal cut lines (parts from top to bottom). \
Pick the axis the diagram flows along. cuts = the positions of the cut lines as fractions between 0 and 1 of the picture's width (axis x) or height (axis y), ascending, one fewer than the parts. \
Put every cut in the blank gap between neighbouring blocks, never through a block, an image or its label; arrows may cross a cut. \
Look carefully at where each block starts and ends before you answer.\n\
parts = the parts in reading order, each {label: 2–5 words, the words printed in the figure where possible; about: one sentence saying what happens in this part}. \
Use 2 or 3 parts unless the diagram clearly has more stages; never more than 5. A part must be wide enough to read (at least 15 % of the picture).\n\
split = false (cuts = [], parts = []) when the figure is simple, or when straight lines cannot separate its stages.\n\
main = the id of the main method figure, or \"\" when none of the figures is one.\n\
why = one short line.\n\
Reply with JSON {main, split, axis, cuts, parts, slides, why}; slides = [] unless slides are listed.";

fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["main", "split", "axis", "cuts", "parts"],
        "properties": {
            "main": { "type": "string" },
            "split": { "type": "boolean" },
            "axis": { "type": "string", "enum": ["x", "y"] },
            "cuts": { "type": "array", "items": { "type": "number" } },
            "parts": { "type": "array", "items": { "type": "object", "required": ["label"], "properties": { "label": { "type": "string" }, "about": { "type": "string" } } } },
            "slides": { "type": "array", "items": { "type": "integer" } },
            "why": { "type": "string" }
        }
    })
}

/// What the request asks beyond "look at these figures".
#[derive(Debug, Clone, Default)]
pub struct Ask<'a> {
    /// Only this figure (the author's or the plan's main figure); else the model picks one.
    pub figure: Option<&'a str>,
    /// Exactly this many parts (微調: 方法拆幾個模組).
    pub parts: Option<usize>,
    /// The method slides to assign to parts: (title, first lines).
    pub slides: &'a [(String, String)],
}

fn request(figs: &[&Fig], ask: &Ask) -> Option<LlmRequest> {
    let mut user = String::from("The figures, in the order of the images:\n");
    let mut images = vec![];
    for f in figs {
        let im = f.image.as_ref()?;
        let (bytes, media) = match im.ext.as_str() {
            "png" => (fit_png(&im.bytes, SIDE).ok()?, "image/png"),
            _ => (im.bytes.clone(), "image/jpeg"),
        };
        user += &format!("- image {}: id {} ({} × {} px). Caption: {}\n", images.len() + 1, f.id, im.w, im.h, super::plan::clip(&f.caption, 500));
        images.push(Image::from_bytes(media, &bytes));
    }
    if let Some(id) = ask.figure {
        user += &format!("\nThe main method figure is {id}: answer main = \"{id}\" and decide its parts.\n");
    }
    if let Some(n) = ask.parts {
        user += &format!("\nThe presenter wants exactly {n} parts: split = true with {n} parts and {} cuts.\n", n - 1);
    }
    if !ask.slides.is_empty() {
        user += "\nThe method slides of the deck, in order. slides = for each of them the number (1-based) of the part it explains:\n";
        for (i, (title, lines)) in ask.slides.iter().enumerate() {
            user += &format!("{}. {title} — {}\n", i + 1, super::plan::clip(lines, 200));
        }
    }
    let mut req = LlmRequest::prompt(SYSTEM, user).json(schema()).max_tokens(1500);
    // `image` puts each new picture after the ones already there: the order of the list
    for im in images {
        req = req.image(im);
    }
    Some(req)
}

/// The model's reply as parts of one of `figs` (`Err`: why it cannot be used).
fn parse(v: &Value, figs: &[&Fig], ask: &Ask) -> Result<FigureParts, String> {
    let main = v["main"].as_str().unwrap_or("").trim().to_string();
    if main.is_empty() && ask.figure.is_none() {
        return Ok(FigureParts { by: "model".into(), why: v["why"].as_str().unwrap_or("").to_string(), ..Default::default() });
    }
    let fig = figs.iter().find(|f| f.id == main || Some(f.id.as_str()) == ask.figure.filter(|_| main.is_empty())).ok_or_else(|| format!("回答的圖 {main} 不在清單裡"))?;
    if let Some(want) = ask.figure
        && fig.id != want
    {
        return Err(format!("回答的是 {}，不是指定的 {want}", fig.id));
    }
    let why = v["why"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
    let split = v["split"].as_bool().unwrap_or(false);
    if !split && ask.parts.is_none() {
        return Ok(FigureParts { figure: fig.id.clone(), by: "model".into(), why, ..Default::default() });
    }
    let axis = match v["axis"].as_str() {
        Some("y") => "y",
        _ => "x",
    };
    let cuts: Vec<f64> = v["cuts"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    let parts: Vec<Part> = v["parts"].as_array().into_iter().flatten().map(|p| Part { label: p["label"].as_str().unwrap_or("").trim().to_string(), about: p["about"].as_str().unwrap_or("").trim().to_string() }).collect();
    if parts.len() < 2 || parts.len() > MAX_PARTS || cuts.len() + 1 != parts.len() {
        return Err(format!("{} 個部分配 {} 條切線", parts.len(), cuts.len()));
    }
    if let Some(n) = ask.parts
        && parts.len() != n
    {
        return Err(format!("要 {n} 個部分，回答了 {} 個", parts.len()));
    }
    if cuts.windows(2).any(|w| w[1] <= w[0]) || cuts.iter().any(|c| !(0.02..=0.98).contains(c)) {
        return Err("切線的位置不合理".into());
    }
    let slides: Vec<usize> = v["slides"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|n| (n as usize).clamp(1, parts.len()) - 1).collect();
    let slides = if slides.len() == ask.slides.len() { slides } else { vec![] };
    Ok(FigureParts { figure: fig.id.clone(), split: true, axis: axis.into(), cuts, parts, slides, by: "model".into(), why })
}

/// Move each cut to the blank gutter nearest to it; a part may not get thinner than 12 %.
/// Returns how many cuts found a gutter.
pub fn snap(p: &mut FigureParts, f: &Fig) -> usize {
    let Some(im) = f.image.as_ref().filter(|i| i.ext == "png") else { return 0 };
    let axis = if p.axis == "y" { Axis::Horizontal } else { Axis::Vertical };
    let Ok(profile) = ink_profile(&im.bytes, axis) else { return 0 };
    let mut found = 0;
    let n = p.cuts.len();
    for i in 0..n {
        let lo = if i == 0 { 0.0 } else { p.cuts[i - 1] } + 0.12;
        let hi = if i + 1 < n { p.cuts[i + 1] } else { 1.0 } - 0.12;
        let (c, ok) = snap_cut(&profile, p.cuts[i], 0.07);
        if ok && c > lo && c < hi {
            p.cuts[i] = c;
            found += 1;
        }
    }
    found
}

/// The outcome of [`figure_parts`]: the parts (if any), model calls made, report lines.
pub struct Outcome {
    pub parts: Option<FigureParts>,
    pub calls: u32,
    pub notes: Vec<String>,
}

/// Ask the model (once more when its answer cannot be used); then the fallback: `ask.figure`
/// (or nothing when the model was to pick) in equal parts — `ask.parts`, else 2 — with a note.
pub async fn figure_parts(llm: &Llm, m: &Material, ask: &Ask<'_>) -> Outcome {
    let figs: Vec<&Fig> = m.figures.iter().filter(|f| f.image.is_some()).filter(|f| ask.figure.is_none_or(|id| f.id == id)).take(8).collect();
    let mut out = Outcome { parts: None, calls: 0, notes: vec![] };
    if figs.is_empty() {
        return out;
    }
    let mut why = String::new();
    for _ in 0..2 {
        let Some(req) = request(&figs, ask) else { break };
        out.calls += 1;
        match llm.complete_json(TASK, req, None).await {
            Ok(a) => match parse(&a.value, &figs, ask) {
                Ok(mut p) => {
                    if p.walk()
                        && let Some(f) = figs.iter().find(|f| f.id == p.figure)
                    {
                        let n = snap(&mut p, f);
                        if n < p.cuts.len() {
                            out.notes.push(format!("{}：{} 條切線裡有 {} 條附近沒有空白，照模型給的位置切", p.figure, p.cuts.len(), p.cuts.len() - n));
                        }
                    }
                    out.parts = Some(p);
                    return out;
                }
                Err(e) => why = e,
            },
            Err((e, _)) => why = e,
        }
    }
    // the fallback, never silent
    match ask.figure.and_then(|id| figs.iter().find(|f| f.id == id)) {
        Some(f) => {
            let n = ask.parts.unwrap_or(2);
            out.notes.push(format!("{}：模型沒有看圖決定怎麼分段（{why}），改成平均切 {n} 段", f.id));
            let mut p = FigureParts::equal(f, n);
            p.slides = (0..ask.slides.len()).map(|i| (i * n / ask.slides.len().max(1)).min(n - 1)).collect();
            out.parts = Some(p);
        }
        None => out.notes.push(format!("模型沒有看圖決定主架構圖與分段（{why}），方法頁不放分段的圖")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todeck::source::Img;
    use lore_llm::MockProvider;
    use lore_llm::router::MockFactory;
    use std::sync::Arc;

    /// White, with dark blocks at 5–30 %, 38–62 % and 70–95 % of the width.
    fn three_blocks() -> Img {
        use std::io::Cursor;
        let mut img = image::RgbaImage::from_pixel(400, 120, image::Rgba([255, 255, 255, 255]));
        for y in 20..100 {
            for x in (20..120).chain(152..248).chain(280..380) {
                img.put_pixel(x, y, image::Rgba([40, 60, 120, 255]));
            }
        }
        let mut buf = Cursor::new(vec![]);
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        Img { bytes: buf.into_inner(), file: String::new(), ext: "png".into(), w: 400, h: 120 }
    }

    fn material() -> Material {
        Material { figures: vec![Fig { id: "fig1".into(), caption: "Overview of the pipeline.".into(), image: Some(three_blocks()), ..Default::default() }, Fig { id: "fig2".into(), caption: "No picture.".into(), ..Default::default() }], ..Default::default() }
    }

    fn llm(replies: Vec<Value>) -> (tempfile::TempDir, Llm, Arc<MockProvider>) {
        let dir = tempfile::tempdir().unwrap();
        let model = Arc::new(MockProvider::new(replies.iter().map(Value::to_string).collect()));
        let llm = Llm::with_factory(lore_home::Home::at(dir.path()), MockFactory::default().with("claude:opus", model.clone()));
        (dir, llm, model)
    }

    #[tokio::test]
    async fn the_model_sees_the_figure_and_its_cuts_move_to_the_gutters() {
        let answer = json!({ "main": "fig1", "split": true, "axis": "x", "cuts": [0.31, 0.68], "why": "three stages",
            "parts": [{ "label": "Encoder", "about": "Finds lesions." }, { "label": "Fusion", "about": "Weighs them." }, { "label": "Decoder", "about": "Grades." }] });
        let (_d, llm, model) = llm(vec![answer]);
        let out = figure_parts(&llm, &material(), &Ask::default()).await;
        let p = out.parts.unwrap();
        assert_eq!((out.calls, p.figure.as_str(), p.parts.len(), p.by.as_str()), (1, "fig1", 3, "model"));
        assert!(p.walk());
        // the blank gutters are at 30–38 % and 62–70 %
        assert!((p.cuts[0] - 0.34).abs() < 0.012 && (p.cuts[1] - 0.66).abs() < 0.012, "{:?}", p.cuts);
        assert_eq!(p.span(0).0, 0.0);
        assert_eq!(p.span(2).1, 1.0);
        let seen = model.seen.lock().unwrap();
        assert!(seen[0].has_images() && seen[0].flatten().contains("id fig1") && !seen[0].flatten().contains("fig2"), "only figures that have a picture are shown");
    }

    #[tokio::test]
    async fn the_authors_number_of_parts_wins_and_a_failed_call_is_an_equal_split_with_a_note() {
        // asked for 2 parts: an answer with 3 is refused, the second answer is used
        let three = json!({ "main": "fig1", "split": true, "axis": "x", "cuts": [0.33, 0.66], "parts": [{ "label": "a" }, { "label": "b" }, { "label": "c" }] });
        let two = json!({ "main": "fig1", "split": true, "axis": "x", "cuts": [0.5], "parts": [{ "label": "a" }, { "label": "b" }], "slides": [1, 2, 2] });
        let (_d, llm, model) = llm(vec![three.clone(), two]);
        let slides = vec![("Encoder".to_string(), String::new()), ("Fusion".to_string(), String::new()), ("Decoder".to_string(), String::new())];
        let ask = Ask { figure: Some("fig1"), parts: Some(2), slides: &slides };
        let out = figure_parts(&llm, &material(), &ask).await;
        let p = out.parts.unwrap();
        assert_eq!((out.calls, p.parts.len(), p.slides.clone()), (2, 2, vec![0, 1, 1]));
        assert!(model.seen.lock().unwrap()[0].flatten().contains("exactly 2 parts"));
        // twice unusable: equal parts, said so
        let (_d, llm, _) = llm_same(three);
        let out = figure_parts(&llm, &material(), &ask).await;
        let p = out.parts.unwrap();
        assert_eq!((out.calls, p.by.as_str(), p.cuts.clone(), p.slides.clone()), (2, "equal", vec![0.5], vec![0, 0, 1]));
        assert!(out.notes.iter().any(|n| n.contains("平均切 2 段")), "{:?}", out.notes);
        // no engine and the model was to pick the figure: no parts, a note
        let none = Llm::none(lore_home::Home::at(_d.path()));
        let out = figure_parts(&none, &material(), &Ask::default()).await;
        assert!(out.parts.is_none() && out.notes.len() == 1);
        // a simple figure: shown whole
        let (_d, llm, _) = llm_same(json!({ "main": "fig1", "split": false, "axis": "x", "cuts": [], "parts": [] }));
        let p = figure_parts(&llm, &material(), &Ask::default()).await.parts.unwrap();
        assert!(!p.walk() && p.figure == "fig1");
    }

    fn llm_same(v: Value) -> (tempfile::TempDir, Llm, Arc<MockProvider>) {
        llm(vec![v])
    }
}
