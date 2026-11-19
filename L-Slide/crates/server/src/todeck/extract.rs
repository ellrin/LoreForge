//! The material of an L-Reader PDF, seen and confirmed by a model (the author, 2026-11-19:
//! quality first — the model really looks at the pages, nothing is guessed by rules and
//! dropped silently). All tasks are Opus, then Sonnet, never the local model.
//!
//! 1. **Locate** (`sc-locate-material`, one call per page): the page's picture → its figures
//!    and tables (number, box, caption), its display equations (number, box, LaTeX) and, on
//!    the first page, the authors. The rule-based detector's boxes and L-Reader's text-layer
//!    LaTeX go along as proposals to check, never as the answer.
//! 2. **Confirm** (`sc-check-crop`): every figure / table crop is shown back to the model:
//!    exactly that item, whole, nothing else? A crop that is cut off or shows more is cut
//!    again from the model's tight box and confirmed once more; one that still fails is kept
//!    with a warning.
//! 3. **Tables**: the cells come from the PDF's own words inside the confirmed box
//!    (`lore_docsrc::pdf::table`), so no number is a model's reading; the model's row and
//!    column counts only cross-check the grid. No grid: the picture, said so.
//! 4. **Equations** (`sc-read-equation`): the LaTeX is typeset (KaTeX through headless
//!    Chrome) and the model compares the crop with the rendering symbol by symbol; a
//!    difference is corrected and compared again. "It typesets" is not enough. No agreement:
//!    the crop is shown as a picture, said so.
//!
//! Every answer is kept in the paper's cache folder (page number or crop hash + prompt
//! version), so a stopped or repeated job never pays for the same look twice. Every skipped
//! or degraded item is a line of the returned notes (the job's warnings).

use std::path::{Path, PathBuf};

use lore_docsrc::crop::{crop_png_fraction, fit_png};
use lore_llm::{Image, LlmRequest};
use serde_json::{Value, json};

use super::source::{Eq, Fig, Material, img_from};
use crate::llm::Llm;

pub const TASK_LOCATE: &str = "sc-locate-material";
pub const TASK_CHECK: &str = "sc-check-crop";
pub const TASK_COMPARE: &str = super::eqread::TASK;
/// Prompt version: part of every cache file's name.
const V: &str = "v1";
/// Model calls running at once.
const PARALLEL: usize = 4;
/// Long side of a page / crop as the model sees it.
const SIDE: u32 = 1568;
const PAGE_DPI: u32 = 150;
const CROP_DPI: u32 = 200;

/// A LaTeX equation as a picture (to compare with the paper's crop).
pub trait Typeset: Send + Sync {
    fn png(&self, latex: &str) -> Result<Vec<u8>, String>;
}

/// The real one: KaTeX (the editor's own copy) drawn by headless Chrome.
/// **Not tested on real hardware in the offline tests.**
pub struct ChromeTypeset {
    pub web: PathBuf,
}

impl Typeset for ChromeTypeset {
    fn png(&self, latex: &str) -> Result<Vec<u8>, String> {
        let chrome = sc_core::find_chrome().ok_or("找不到 Chrome")?;
        let dir = std::env::temp_dir().join(format!("sc-typeset-{}-{:016x}", std::process::id(), lore_canvas::fnv(latex.as_bytes())));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let katex = self.web.join("vendor/katex");
        let html = format!(
            "<!doctype html><meta charset=\"utf-8\"><link rel=\"stylesheet\" href=\"file://{k}/katex.min.css\"><script src=\"file://{k}/katex.min.js\"></script>\
<body style=\"margin:0;background:#fff\"><div id=\"e\" style=\"font-size:30px;padding:28px 24px\"></div>\
<script>try{{katex.render({src},document.getElementById('e'),{{displayMode:true,throwOnError:false}})}}catch(x){{document.getElementById('e').textContent=String(x)}}</script>",
            k = katex.display(),
            src = serde_json::to_string(latex).map_err(|e| e.to_string())?.replace("</", "<\\/")
        );
        let (page, out) = (dir.join("eq.html"), dir.join("eq.png"));
        std::fs::write(&page, html).map_err(|e| e.to_string())?;
        let r = sc_core::screenshot_png(&chrome, &format!("file://{}", page.display()), 1500, 380, &out).map_err(|e| e.to_string()).and_then(|_| std::fs::read(&out).map_err(|e| e.to_string()));
        let _ = std::fs::remove_dir_all(&dir);
        r
    }
}

/// What the rule-based detector (or L-Reader) proposes for a page.
#[derive(Debug, Clone, Default)]
pub struct Hint {
    pub page: u32,
    /// `figure`, `table` or `equation`.
    pub kind: &'static str,
    pub number: String,
    pub bbox: Option<[f64; 4]>,
    /// Equation: the text-layer LaTeX (only when it has no control characters).
    pub latex: String,
}

pub struct Input<'a> {
    pub pages: u32,
    /// Page `n` (1-based) as PNG at a dpi.
    pub page_png: &'a (dyn Fn(u32, u32) -> Option<Vec<u8>> + Send + Sync),
    /// The cells of the PDF's own text inside a box of a page.
    pub table_grid: &'a (dyn Fn(u32, [f64; 4]) -> Option<Vec<Vec<String>>> + Send + Sync),
    pub typeset: Option<&'a dyn Typeset>,
    /// Where the answers are kept.
    pub cache: PathBuf,
    pub hints: Vec<Hint>,
}

#[derive(Debug, Default)]
pub struct Report {
    /// Model calls made now / answers taken from the cache.
    pub calls: u32,
    pub cached: u32,
    pub notes: Vec<String>,
    /// No page could be looked at (no engine): the material is the detector's, unconfirmed.
    pub unverified: bool,
}

// ── asking, with the answers kept ────────────────────────────────────────────

/// LaTeX with a control character in it (L-Reader maps some PDF glyphs to U+0008 and the
/// like): never shown to anybody as it is.
pub fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

struct Asker<'a> {
    llm: &'a Llm,
    cache: &'a Path,
    calls: std::sync::atomic::AtomicU32,
    cached: std::sync::atomic::AtomicU32,
}

impl Asker<'_> {
    /// The answer for `key` (a cache file name), asked when it is not there yet.
    async fn ask(&self, key: &str, task: &str, req: LlmRequest) -> Result<Value, String> {
        use std::sync::atomic::Ordering::Relaxed;
        let file = self.cache.join(format!("{key}.json"));
        if let Some(v) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            self.cached.fetch_add(1, Relaxed);
            return Ok(v);
        }
        self.calls.fetch_add(1, Relaxed);
        let a = self.llm.complete_json(task, req, None).await.map_err(|(m, _)| m)?;
        let _ = std::fs::create_dir_all(self.cache);
        let _ = crate::jobs::write_atomic(&file, &serde_json::to_vec(&a.value).unwrap_or_default());
        Ok(a.value)
    }
}

fn hash(parts: &[&[u8]]) -> String {
    let mut all = vec![];
    for p in parts {
        all.extend_from_slice(p);
        all.push(0);
    }
    format!("{:016x}", lore_canvas::fnv(&all))
}

fn frac4(v: &Value) -> Option<[f64; 4]> {
    let a: Vec<f64> = v.as_array()?.iter().filter_map(Value::as_f64).collect();
    if a.len() != 4 {
        return None;
    }
    let b = [a[0].min(a[2]).clamp(0.0, 1.0), a[1].min(a[3]).clamp(0.0, 1.0), a[0].max(a[2]).clamp(0.0, 1.0), a[1].max(a[3]).clamp(0.0, 1.0)];
    (b[2] - b[0] > 0.01 && b[3] - b[1] > 0.004).then_some(b)
}

// ── 1. locate ────────────────────────────────────────────────────────────────

const LOCATE: &str = "You read one page of a research paper, given as an image, and list what is on it for someone who will build seminar slides from the paper. \
A program cuts the items out of the page with your boxes, so be exact: look where the ink of each item starts and ends.\n\
Boxes are [x0, y0, x1, y1] as fractions of the page image: x from the left edge, y from the top edge, each between 0 and 1.\n\n\
- figures: every figure on this page. number = as printed (\"1\", \"2\"); box = the whole picture with all its panels, axes, tick labels, legends and panel letters, \
but NOT the caption (\"Fig. 1. …\"), NOT body text and NOT a table or another figure next to it; caption = the caption text without the \"Fig. N.\" label.\n\
- tables: every table on this page. number = as printed, Roman numerals as they are (\"I\", \"II\"); box = the table from its top rule to its bottom rule with the header row, \
but NOT its title or caption (above or below), NOT footnotes and NOT a figure next to it; caption = the title text without the \"TABLE N\" label, in sentence case; \
rows = how many rows of text it has, the header included; cols = how many columns.\n\
- equations: every DISPLAYED equation (set on its own line or lines, usually with a number at the right margin); never inline math. number = the printed number without parentheses (\"\" when it has none); \
box = the equation without its number; latex = the equation transcribed exactly, as LaTeX that KaTeX can typeset: \\frac for stacked fractions, \\sum_{i=1}^{N} with its limits, \
\\tilde / \\hat / \\bar accents, \\mathrm{…} for words in subscripts, \\mathcal for calligraphic capitals, \\mathbb for blackboard letters (\\mathbb{1} for an indicator), \
several rows in \\begin{aligned} … \\end{aligned} with & before the relation. Leave out the number and trailing punctuation.\n\
- authors: only on the first page of the paper, the authors' names in order, without affiliations, degrees or marks; otherwise [].\n\n\
Do not invent anything: a list is empty when the page has none. A program's proposals may be listed; they are often wrong (a box that runs into the next item, a formula read from broken text): check each against the image.\n\
Reply with JSON {figures, tables, equations, authors}.";

fn locate_schema() -> Value {
    let s = json!({ "type": "string" });
    let bx = json!({ "type": "array", "items": { "type": "number" } });
    json!({
        "type": "object",
        "required": ["figures", "tables", "equations"],
        "properties": {
            "figures": { "type": "array", "items": { "type": "object", "required": ["box"], "properties": { "number": s, "box": bx, "caption": s } } },
            "tables": { "type": "array", "items": { "type": "object", "required": ["box"], "properties": { "number": s, "box": bx, "caption": s, "rows": { "type": "integer" }, "cols": { "type": "integer" } } } },
            "equations": { "type": "array", "items": { "type": "object", "required": ["box", "latex"], "properties": { "number": s, "box": bx, "latex": s } } },
            "authors": { "type": "array", "items": s }
        }
    })
}

fn locate_request(page: u32, pages: u32, png: &[u8], hints: &[Hint]) -> LlmRequest {
    let mut user = format!("Page {page} of {pages}.");
    let mine: Vec<&Hint> = hints.iter().filter(|h| h.page == page).collect();
    if !mine.is_empty() {
        user += "\nThe program's proposals for this page (check them, they may be wrong):\n";
        for h in mine {
            let at = h.bbox.map(|b| format!(" near [{:.3}, {:.3}, {:.3}, {:.3}]", b[0], b[1], b[2], b[3])).unwrap_or_default();
            let tex = if h.latex.is_empty() { String::new() } else { format!("; its text-layer reading (unreliable): {}", super::plan::clip(&h.latex, 300)) };
            user += &format!("- {} {}{at}{tex}\n", h.kind, h.number);
        }
    }
    LlmRequest::prompt(LOCATE, user).json(locate_schema()).max_tokens(6000).image(Image::from_bytes("image/png", png))
}

/// "IV" → 4 (table numbers); a number that is no Roman numeral comes back as it is.
fn arabic(n: &str) -> String {
    let t = n.trim().trim_end_matches('.').to_uppercase();
    if t.is_empty() || !t.chars().all(|c| "IVXLC".contains(c)) {
        return n.trim().trim_end_matches('.').to_lowercase();
    }
    let val = |c| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        _ => 100,
    };
    let cs: Vec<i32> = t.chars().map(val).collect();
    let mut sum = 0;
    for (i, v) in cs.iter().enumerate() {
        sum += if cs.get(i + 1).is_some_and(|n| n > v) { -v } else { *v };
    }
    sum.to_string()
}

fn id_part(n: &str) -> String {
    n.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

/// Mostly capitals (a small-caps title read from the text layer).
fn shouting(s: &str) -> bool {
    let letters: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 8 && letters.iter().filter(|c| c.is_uppercase()).count() * 10 >= letters.len() * 7
}

struct Found {
    kind: &'static str,
    id: String,
    number: String,
    page: u32,
    bbox: [f64; 4],
    caption: String,
    rows: usize,
    cols: usize,
    latex: String,
}

// ── 2. confirm a figure / table crop ─────────────────────────────────────────

const CHECK: &str = "You check a crop cut out of a page of a research paper for a slide. The crop should show exactly one item, whole, and nothing else. \
For a figure: the whole picture (all panels, axes, tick labels, legends, panel letters) without its caption line. \
For a table: the table from its top rule to its bottom rule with its header row, without its title, caption or footnotes.\n\
Answer with JSON {ok, cut_off, extra, tight, problem}:\n\
- cut_off = true when part of the item is missing at an edge of the image (a panel, an axis label, a row or a column is cut).\n\
- extra = true when the image also shows something that is not part of the item: caption or title text, body text of the paper, another figure or table, a page header.\n\
- ok = true only when cut_off and extra are both false.\n\
- tight = [x0, y0, x1, y1]: the box of the item alone inside THIS image, as fractions of the image (x from the left, y from the top; the whole image is [0, 0, 1, 1]).\n\
- problem = one short line saying what is wrong, \"\" when ok.";

fn check_schema() -> Value {
    json!({
        "type": "object",
        "required": ["ok"],
        "properties": {
            "ok": { "type": "boolean" }, "cut_off": { "type": "boolean" }, "extra": { "type": "boolean" },
            "tight": { "type": "array", "items": { "type": "number" } }, "problem": { "type": "string" }
        }
    })
}

/// The crop of `f` confirmed (at most three looks). Returns the final box, the crop and
/// `Err(problem)` when the last look still found something wrong.
async fn confirm(ask: &Asker<'_>, inp: &Input<'_>, f: &Found) -> (Option<([f64; 4], Vec<u8>)>, Result<(), String>) {
    let Some(page) = (inp.page_png)(f.page, CROP_DPI) else { return (None, Err("頁面無法轉成圖片".into())) };
    let what = if f.kind == "table" { format!("Table {}", f.number) } else { format!("Figure {}", f.number) };
    let mut bbox = f.bbox;
    let mut last: Option<([f64; 4], Vec<u8>)> = None;
    let mut problem = String::new();
    for round in 0..3 {
        let Ok(crop) = crop_png_fraction(&page, bbox, 6) else { return (last, Err("裁切失敗".into())) };
        let shown = fit_png(&crop, SIDE).unwrap_or_else(|_| crop.clone());
        let user = format!("The item: {what} of the paper. Its caption: {}", super::plan::clip(&f.caption, 400));
        let req = LlmRequest::prompt(CHECK, user).json(check_schema()).max_tokens(800).image(Image::from_bytes("image/png", &shown));
        let key = format!("check-{V}-{}", hash(&[&crop, what.as_bytes()]));
        let v = match ask.ask(&key, TASK_CHECK, req).await {
            Ok(v) => v,
            Err(e) => return (Some((bbox, crop)), Err(format!("模型沒有確認（{e}）"))),
        };
        let (cut, extra) = (v["cut_off"] == true, v["extra"] == true);
        if v["ok"] == true && !cut && !extra {
            return (Some((bbox, crop)), Ok(()));
        }
        problem = v["problem"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
        last = Some((bbox, crop));
        if round == 2 {
            break;
        }
        // cut again: wider when something is missing, else the model's tight box
        let (w, h) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
        bbox = if cut {
            [(bbox[0] - 0.04).max(0.0), (bbox[1] - 0.04).max(0.0), (bbox[2] + 0.04).min(1.0), (bbox[3] + 0.04).min(1.0)]
        } else {
            match frac4(&v["tight"]) {
                Some(t) => [bbox[0] + t[0] * w, bbox[1] + t[1] * h, bbox[0] + t[2] * w, bbox[1] + t[3] * h],
                None => break,
            }
        };
    }
    (last, Err(if problem.is_empty() { "裁切的範圍不對".into() } else { problem }))
}

// ── 4. an equation's reading against its crop ────────────────────────────────

const COMPARE: &str = "You check the LaTeX transcription of one display equation of a research paper. Image 1 is the equation cut from the paper. \
When there is an image 2, it is the transcription typeset by a program: compare the two pictures symbol by symbol — every letter, subscript and superscript, accent, fraction, \
sum or product with its limits, bracket, set and indicator symbol, and the order of the terms. Without an image 2, read the LaTeX yourself and compare it with image 1. \
Differences in spacing, font size or line breaking do not matter; the equation number and trailing punctuation are not part of the equation.\n\
Answer with JSON {same, latex, whole, difference}: same = true only when the transcription shows exactly the equation of image 1; \
latex = when not the same, the complete corrected transcription as LaTeX that KaTeX can typeset (\\frac, \\sum_{…}^{…}, \\mathrm{…} for words, \\mathbb{1} for an indicator; no number), else \"\"; \
whole = false when image 1 does not show the whole equation or is not an equation; difference = one short line, \"\" when the same.";

fn compare_schema() -> Value {
    json!({
        "type": "object",
        "required": ["same"],
        "properties": { "same": { "type": "boolean" }, "latex": { "type": "string" }, "whole": { "type": "boolean" }, "difference": { "type": "string" } }
    })
}

fn clean_latex(s: &str) -> String {
    let mut t = s.trim();
    for (a, b) in [("$$", "$$"), ("\\[", "\\]"), ("$", "$")] {
        if let Some(x) = t.strip_prefix(a).and_then(|x| x.strip_suffix(b)) {
            t = x.trim();
        }
    }
    let t = regex::Regex::new(r"\\(tag|label)\{[^}]*\}").unwrap().replace_all(t, "");
    t.trim().trim_end_matches([',', '.', ';']).trim().to_string()
}

/// The reading of `f` compared with its crop (twice at most). `Ok(latex)`: agreed.
async fn compare(ask: &Asker<'_>, inp: &Input<'_>, f: &Found, crop: &[u8]) -> Result<String, String> {
    let mut latex = clean_latex(&f.latex);
    let shown = fit_png(crop, SIDE).unwrap_or_else(|_| crop.to_vec());
    let mut why = String::new();
    for _ in 0..2 {
        let typesets = !latex.is_empty() && !has_control(&latex) && lore_math::check::check_reading(&latex).is_ok();
        let render = if typesets { inp.typeset.and_then(|t| t.png(&latex).ok()) } else { None };
        let mut user = format!("The equation {} of the paper. The transcription:\n{latex}", if f.number.is_empty() { String::new() } else { format!("({})", f.number) });
        if !typesets {
            user += "\n(This transcription cannot be typeset: write the correct one.)";
        }
        let mut req = LlmRequest::prompt(COMPARE, user).json(compare_schema()).max_tokens(2000).image(Image::from_bytes("image/png", &shown));
        if let Some(r) = &render {
            req = req.image(Image::from_bytes("image/png", r));
        }
        let key = format!("compare-{V}-{}", hash(&[crop, latex.as_bytes(), if render.is_some() { b"r" } else { b"t" }]));
        let v = ask.ask(&key, TASK_COMPARE, req).await.map_err(|e| format!("模型沒有比對（{e}）"))?;
        if v["whole"] == false {
            return Err("截圖不是完整的一條式子".into());
        }
        if v["same"] == true && typesets {
            return Ok(latex);
        }
        why = v["difference"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
        let fixed = clean_latex(v["latex"].as_str().unwrap_or(""));
        if fixed.is_empty() || fixed == latex {
            break;
        }
        latex = fixed;
    }
    Err(if why.is_empty() { "讀出的式子和截圖對不起來".into() } else { format!("讀出的式子和截圖對不起來（{why}）") })
}

// ── the whole pass ───────────────────────────────────────────────────────────

type Fut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Every future's result in order, [`PARALLEL`] of them running at a time.
async fn run_all<T>(mut futs: Vec<Fut<'_, T>>) -> Vec<T> {
    let mut out = Vec::with_capacity(futs.len());
    while !futs.is_empty() {
        let rest = futs.split_off(futs.len().min(PARALLEL));
        out.extend(futures_util::future::join_all(futs).await);
        futs = rest;
    }
    out
}

/// Look at every page, confirm every crop and reading, and put the result into `m` (its
/// figures, tables, equations, authors). The detector's material stays only where no model
/// could look, and the report says so.
pub async fn verify(llm: &Llm, m: &mut Material, inp: &Input<'_>) -> Report {
    let ask = Asker { llm, cache: &inp.cache, calls: Default::default(), cached: Default::default() };
    let mut notes: Vec<String> = vec![];

    // 1. every page
    let pages: Vec<u32> = (1..=inp.pages).collect();
    let mut looks: Vec<Fut<'_, (u32, Result<Value, String>)>> = vec![];
    for &n in &pages {
        let ask = &ask;
        looks.push(Box::pin(async move {
            let Some(png) = (inp.page_png)(n, PAGE_DPI).and_then(|p| fit_png(&p, SIDE).ok()) else { return (n, Err("頁面無法轉成圖片".to_string())) };
            (n, ask.ask(&format!("locate-{V}-p{n}"), TASK_LOCATE, locate_request(n, inp.pages, &png, &inp.hints)).await)
        }));
    }
    let located = run_all(looks).await;
    let count = |a: &Asker| (a.calls.load(std::sync::atomic::Ordering::Relaxed), a.cached.load(std::sync::atomic::Ordering::Relaxed));
    let failed: Vec<&(u32, Result<Value, String>)> = located.iter().filter(|(_, r)| r.is_err()).collect();
    if failed.len() == located.len() {
        let why = failed.first().and_then(|(_, r)| r.as_ref().err().cloned()).unwrap_or_default();
        notes.push(format!("沒有模型看過論文的頁面（{why}）：圖、表、方程式是程式偵測的結果，沒有經過確認"));
        let (calls, cached) = count(&ask);
        return Report { calls, cached, notes, unverified: true };
    }
    for (n, r) in &failed {
        notes.push(format!("第 {n} 頁模型沒有看（{}）：這一頁的圖表與方程式沒有放", r.as_ref().err().cloned().unwrap_or_default()));
    }
    let mut found: Vec<Found> = vec![];
    let mut authors: Vec<String> = vec![];
    for (n, r) in &located {
        let Ok(v) = r else { continue };
        if authors.is_empty() && *n <= 2 {
            authors = v["authors"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|a| a.trim().to_string()).filter(|a| !a.is_empty() && a.chars().count() < 60).collect();
        }
        let mut unnumbered = 0;
        for (kind, key) in [("figure", "figures"), ("table", "tables"), ("equation", "equations")] {
            let mut items: Vec<&Value> = v[key].as_array().into_iter().flatten().collect();
            items.sort_by(|a, b| a["box"][1].as_f64().partial_cmp(&b["box"][1].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
            for it in items {
                let number = it["number"].as_str().unwrap_or("").trim().trim_matches(['(', ')']).to_string();
                let Some(bbox) = frac4(&it["box"]) else {
                    notes.push(format!("第 {n} 頁的 {kind} {number}：模型給的範圍不能用，沒有放"));
                    continue;
                };
                let num = if kind == "table" { arabic(&number) } else { id_part(&number) };
                let id = match (kind, num.is_empty()) {
                    ("figure", false) => format!("fig{num}"),
                    ("table", false) => format!("tab{num}"),
                    ("equation", false) => format!("eq:{num}"),
                    _ => {
                        unnumbered += 1;
                        format!("{}p{n}-{unnumbered}", if kind == "equation" { "eq:" } else if kind == "table" { "tab" } else { "fig" })
                    }
                };
                if found.iter().any(|f| f.id == id) {
                    continue; // the same item seen on a second page (a paper bound twice)
                }
                found.push(Found {
                    kind,
                    id,
                    number,
                    page: *n,
                    bbox,
                    caption: it["caption"].as_str().unwrap_or("").trim().to_string(),
                    rows: it["rows"].as_u64().unwrap_or(0) as usize,
                    cols: it["cols"].as_u64().unwrap_or(0) as usize,
                    latex: it["latex"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }

    // 2. figures and tables: each crop shown back
    let visuals: Vec<&Found> = found.iter().filter(|f| f.kind != "equation").collect();
    let mut checks: Vec<Fut<'_, (Option<([f64; 4], Vec<u8>)>, Result<(), String>)>> = vec![];
    for f in &visuals {
        checks.push(Box::pin(confirm(&ask, inp, f)));
    }
    let confirmed = run_all(checks).await;
    let old_figs = std::mem::take(&mut m.figures);
    let old_tabs = std::mem::take(&mut m.tables);
    for (f, (got, ok)) in visuals.iter().zip(confirmed) {
        let label = if f.kind == "table" { format!("表 {}", f.number) } else { format!("圖 {}", f.number) };
        let Some((bbox, crop)) = got else {
            notes.push(format!("{label}（第 {} 頁）：{}，沒有圖檔", f.page, ok.err().unwrap_or_default()));
            continue;
        };
        if let Err(why) = &ok {
            notes.push(format!("{label}（第 {} 頁）：截圖給模型確認沒有通過（{why}），照目前的範圍放，請自己看一下", f.page));
        }
        // the caption: the PDF's own text when the detector has it (not a small-caps title), else the model's reading
        let old = if f.kind == "table" { &old_tabs } else { &old_figs }.iter().find(|o| o.id == f.id);
        let caption = match old.map(|o| o.caption.trim()).filter(|c| !c.is_empty() && !shouting(c)) {
            Some(c) => c.to_string(),
            None => f.caption.clone(),
        };
        let mut fig = Fig { id: f.id.clone(), caption, image: img_from(crop, "png"), grid: None, page: Some(f.page), bbox: Some(bbox), number: f.number.clone() };
        if f.kind == "table" {
            // 3. the cells: the PDF's own words inside the confirmed box
            match (inp.table_grid)(f.page, bbox) {
                Some(g) => {
                    let (rows, cols) = (g.len(), g.iter().map(Vec::len).max().unwrap_or(0));
                    if (f.cols > 0 && cols != f.cols) || (f.rows > 0 && rows.abs_diff(f.rows) > 1) {
                        notes.push(format!("{label}：PDF 文字排出 {rows} 列 {cols} 欄，模型看到的是 {} 列 {} 欄，對不起來：改放表格的截圖", f.rows, f.cols));
                    } else {
                        if f.rows > 0 && rows != f.rows {
                            notes.push(format!("{label}：PDF 文字排出 {rows} 列，模型看到 {} 列，請核對表格", f.rows));
                        }
                        fig.grid = Some(g);
                    }
                }
                None => notes.push(format!("{label}：PDF 的文字排不出表格，改放表格的截圖")),
            }
            m.tables.push(fig);
        } else {
            m.figures.push(fig);
        }
    }
    // what the detector had and no model confirmed is not used; say what differs
    for (old, new, name) in [(&old_figs, &m.figures, "圖"), (&old_tabs, &m.tables, "表")] {
        let gone: Vec<&str> = old.iter().filter(|o| !new.iter().any(|n| n.id == o.id)).map(|o| o.id.as_str()).collect();
        if !gone.is_empty() {
            notes.push(format!("程式偵測到的{name} {} 模型在頁面上沒有看到，沒有放", gone.join("、")));
        }
    }
    m.figures.sort_by(|a, b| natural(&a.id).cmp(&natural(&b.id)));
    m.tables.sort_by(|a, b| natural(&a.id).cmp(&natural(&b.id)));

    // 4. equations: the reading against the crop
    let eqs: Vec<&Found> = found.iter().filter(|f| f.kind == "equation").collect();
    let mut reads: Vec<Fut<'_, (Option<Vec<u8>>, Result<String, String>)>> = vec![];
    for f in &eqs {
        let ask = &ask;
        reads.push(Box::pin(async move {
            let Some(crop) = (inp.page_png)(f.page, CROP_DPI).and_then(|p| crop_png_fraction(&p, f.bbox, 10).ok()) else { return (None, Err("頁面無法轉成圖片".to_string())) };
            let r = compare(ask, inp, f, &crop).await;
            (Some(crop), r)
        }));
    }
    let read = run_all(reads).await;
    let old_eqs = std::mem::take(&mut m.equations);
    for (f, (crop, r)) in eqs.iter().zip(read) {
        let label = if f.number.is_empty() { f.id.clone() } else { format!("式 ({})", f.number) };
        let mut e = Eq { id: f.id.clone(), latex: String::new(), label: (!f.number.is_empty()).then(|| format!("({})", f.number)), page: Some(f.page), bbox: Some(f.bbox), read: "vision".into(), image: None };
        match r {
            Ok(latex) => e.latex = latex,
            Err(why) => {
                e.read = "picture".into();
                e.image = crop.and_then(|c| img_from(c, "png"));
                notes.push(format!("{label}（第 {} 頁）：{why}，{}", f.page, if e.image.is_some() { "改放論文的截圖" } else { "沒有放" }));
            }
        }
        m.equations.push(e);
    }
    let gone: Vec<&str> = old_eqs.iter().filter(|o| !m.equations.iter().any(|n| n.id == o.id)).map(|o| o.id.as_str()).collect();
    if !gone.is_empty() {
        notes.push(format!("L-Reader 文字層的 {} 條式子（{}）模型在頁面上沒有看到獨立成行的式子，沒有放", gone.len(), gone.join("、")));
    }
    if m.authors.is_empty() {
        if authors.is_empty() {
            notes.push("第一頁讀不出作者".to_string());
        }
        m.authors = authors;
    }
    let (calls, cached) = count(&ask);
    Report { calls, cached, notes, unverified: false }
}

/// `fig10` after `fig2`.
fn natural(id: &str) -> (String, u32, String) {
    let digits: String = id.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    let head: String = id.chars().take_while(|c| !c.is_ascii_digit()).collect();
    (head, digits.parse().unwrap_or(u32::MAX), id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_llm::router::MockFactory;
    use std::sync::{Arc, Mutex};

    /// Answers by what the request is about: the first rule whose words the system prompt and
    /// the user text contain gives its next reply (the last one again and again).
    struct Scripted {
        rules: Mutex<Vec<(&'static str, &'static str, Vec<Value>)>>,
        seen: Mutex<Vec<LlmRequest>>,
    }

    #[async_trait::async_trait]
    impl lore_llm::LlmProvider for Scripted {
        fn id(&self) -> String {
            "mock".into()
        }
        fn capabilities(&self) -> lore_llm::Capabilities {
            lore_llm::Capabilities { json: true, vision: true, web: false, streaming: true, context: 1_000_000, needs_network: false }
        }
        async fn send(&self, req: &LlmRequest) -> Result<lore_llm::LlmResponse, lore_llm::LlmError> {
            self.seen.lock().unwrap().push(req.clone());
            let user = req.flatten();
            let mut rules = self.rules.lock().unwrap();
            let text = match rules.iter_mut().find(|(s, u, _)| req.system.contains(s) && user.contains(u)) {
                Some((_, _, r)) if r.len() > 1 => r.remove(0).to_string(),
                Some((_, _, r)) => r[0].to_string(),
                None => return Err(lore_llm::LlmError::NotConfigured("no reply for this request".into())),
            };
            Ok(lore_llm::LlmResponse { text, provider: self.id(), sources: vec![], usage: None })
        }
    }

    fn page() -> Vec<u8> {
        use std::io::Cursor;
        let mut img = image::RgbaImage::from_pixel(600, 800, image::Rgba([255, 255, 255, 255]));
        for y in 100..300 {
            for x in 100..500 {
                img.put_pixel(x, y, image::Rgba([40, 60, 120, 255]));
            }
        }
        let mut buf = Cursor::new(vec![]);
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    struct FakeTypeset(Mutex<Vec<String>>);
    impl Typeset for FakeTypeset {
        fn png(&self, latex: &str) -> Result<Vec<u8>, String> {
            self.0.lock().unwrap().push(latex.to_string());
            Ok(page())
        }
    }

    fn material() -> Material {
        Material {
            figures: vec![Fig { id: "fig1".into(), caption: "Overview of the pipeline.".into(), ..Default::default() }, Fig { id: "fig7".into(), caption: "A ghost.".into(), ..Default::default() }],
            tables: vec![Fig { id: "tab1".into(), caption: "DICE PER SITE ON THE SYNTHETIC SET".into(), ..Default::default() }],
            equations: vec![Eq { id: "eq:1".into(), latex: "G(m \u{8}u".into(), read: "text".into(), ..Default::default() }, Eq { id: "eq:p2-3".into(), latex: "K\\sum".into(), read: "text".into(), ..Default::default() }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn pages_are_looked_at_crops_confirmed_cells_from_the_pdf_and_readings_compared() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = json!({ "authors": ["Ellrin Example", "Daidai Example"],
            "figures": [{ "number": "1", "box": [0.15, 0.12, 0.85, 0.50], "caption": "Overview of the pipeline." }],
            "tables": [], "equations": [{ "number": "1", "box": [0.2, 0.6, 0.8, 0.65], "latex": "G(m) = \\sum_{t} u_t" }] });
        let p2 = json!({ "authors": [],
            "figures": [{ "number": "2", "box": [0.1, 0.5, 0.9, 0.9], "caption": "Results per site." }],
            "tables": [{ "number": "I", "box": [0.1, 0.1, 0.9, 0.3], "caption": "Dice per site on the synthetic set", "rows": 3, "cols": 3 }],
            "equations": [{ "number": "2", "box": [0.2, 0.35, 0.8, 0.4], "latex": "S = \\frac{1}{K} \\sum_{k=1}^{K} z_k" }, { "number": "3", "box": [0.2, 0.42, 0.8, 0.46], "latex": "y = \\frac{a}{b" }] });
        let model = Arc::new(Scripted {
            rules: Mutex::new(vec![
                ("read one page", "Page 1 of 2", vec![p1]),
                ("read one page", "Page 2 of 2", vec![p2]),
                // figure 1's first crop shows caption text: cut again from the tight box, then fine
                ("check a crop", "Figure 1", vec![json!({ "ok": false, "cut_off": false, "extra": true, "tight": [0.0, 0.0, 1.0, 0.8], "problem": "caption text at the bottom" }), json!({ "ok": true })]),
                ("check a crop", "Figure 2", vec![json!({ "ok": false, "cut_off": false, "extra": true, "tight": [0.0, 0.0, 1.0, 0.9], "problem": "runs into the table" })]),
                ("check a crop", "Table I", vec![json!({ "ok": true, "cut_off": false, "extra": false })]),
                ("LaTeX transcription", "G(m) = \\sum_{t} u_t", vec![json!({ "same": true })]),
                // equation 2: a wrong limit is corrected, the corrected reading compared again
                ("LaTeX transcription", "\\sum_{k=1}^{K} z_k", vec![json!({ "same": false, "latex": "S = \\frac{1}{K} \\sum_{k=1}^{K} |z_k|", "difference": "absolute value missing" })]),
                ("LaTeX transcription", "|z_k|", vec![json!({ "same": true })]),
                // equation 3 does not typeset and the model cannot fix it
                ("LaTeX transcription", "y = \\frac{a}{b", vec![json!({ "same": false, "latex": "", "difference": "unreadable denominator" })]),
            ]),
            seen: Mutex::new(vec![]),
        });
        let llm = Llm::with_factory(lore_home::Home::at(dir.path()), MockFactory::default().with("claude:opus", model.clone()));
        let png = page();
        let grid = vec![vec!["Method".to_string(), "Site A".into(), "Site B".into()], vec!["Ours".into(), "0.91".into(), "0.84".into()], vec!["Baseline".into(), "0.85".into(), "0.79".into()]];
        let ts = FakeTypeset(Mutex::new(vec![]));
        let page_png = |_: u32, _: u32| Some(png.clone());
        let table_grid = |_: u32, _: [f64; 4]| Some(grid.clone());
        let hints = vec![Hint { page: 1, kind: "equation", number: "(1)".into(), bbox: None, latex: String::new() }, Hint { page: 2, kind: "table", number: "I".into(), bbox: Some([0.1, 0.1, 0.9, 0.9]), latex: String::new() }];
        let inp = Input { pages: 2, page_png: &page_png, table_grid: &table_grid, typeset: Some(&ts), cache: dir.path().join("cache"), hints };
        let mut m = material();
        let rep = verify(&llm, &mut m, &inp).await;
        assert!(!rep.unverified);

        // figures: confirmed crops; the detector's fig7 nobody saw is gone, said so
        assert_eq!(m.figures.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["fig1", "fig2"]);
        assert!(m.figures.iter().all(|f| f.image.is_some()));
        let f1 = m.figures[0].bbox.unwrap();
        assert!((f1[3] - (0.12 + 0.38 * 0.8)).abs() < 1e-9, "cut again from the tight box: {f1:?}");
        assert!(rep.notes.iter().any(|n| n.contains("圖 2") && n.contains("沒有通過") && n.contains("runs into the table")), "{:?}", rep.notes);
        assert!(rep.notes.iter().any(|n| n.contains("fig7") && n.contains("沒有看到")));
        // the table: cells from the PDF text, the caption read by the model when the text layer shouts
        assert_eq!(m.tables[0].id, "tab1");
        assert_eq!(m.tables[0].grid.as_ref().unwrap()[1], ["Ours", "0.91", "0.84"]);
        assert_eq!(m.tables[0].caption, "Dice per site on the synthetic set");
        // equations: agreed readings are LaTeX, the corrected one was compared again, the unreadable one is its crop
        let e: Vec<(&str, &str, &str)> = m.equations.iter().map(|e| (e.id.as_str(), e.read.as_str(), e.latex.as_str())).collect();
        assert_eq!(e[0], ("eq:1", "vision", "G(m) = \\sum_{t} u_t"));
        assert_eq!(e[1], ("eq:2", "vision", "S = \\frac{1}{K} \\sum_{k=1}^{K} |z_k|"));
        assert_eq!((e[2].0, e[2].1), ("eq:3", "picture"));
        assert!(m.equations[2].image.is_some() && !m.equations[2].usable());
        assert!(rep.notes.iter().any(|n| n.contains("式 (3)") && n.contains("改放論文的截圖")));
        assert!(rep.notes.iter().any(|n| n.contains("eq:p2-3")), "L-Reader's fragment nobody saw: {:?}", rep.notes);
        assert!(!m.equations.iter().any(|e| has_control(&e.latex)));
        assert_eq!(m.authors, ["Ellrin Example", "Daidai Example"]);
        // typeset pictures went along with the comparisons (not for the one that cannot be typeset)
        assert_eq!(ts.0.lock().unwrap().len(), 3);
        let seen = model.seen.lock().unwrap();
        let compares: Vec<&LlmRequest> = seen.iter().filter(|r| r.system.contains("LaTeX transcription")).collect();
        assert_eq!(compares.len(), 4);
        assert!(seen.iter().filter(|r| r.system.contains("read one page")).all(|r| r.has_images()));
        assert!(seen.iter().find(|r| r.flatten().contains("Page 2 of 2")).unwrap().flatten().contains("table I near [0.100"), "the detector's box is a proposal");
        // 2 pages + 2 + 3 + 1 crop checks + 4 comparisons
        assert_eq!((rep.calls, rep.cached), (12, 0));
        drop(seen);

        // again (a resumed or repeated job): every answer comes from the cache
        let mut m2 = material();
        let rep2 = verify(&llm, &mut m2, &inp).await;
        assert_eq!((rep2.calls, rep2.cached), (0, 12));
        assert_eq!(m2.equations[1].latex, m.equations[1].latex);

        // a table whose text does not give the columns the model saw: the picture, said so
        let bad = |_: u32, _: [f64; 4]| Some(vec![vec!["a".to_string(), "b".into()], vec!["c".into(), "d".into()]]);
        let inp3 = Input { pages: 2, page_png: &page_png, table_grid: &bad, typeset: Some(&ts), cache: dir.path().join("cache"), hints: vec![] };
        let mut m3 = material();
        let rep3 = verify(&llm, &mut m3, &inp3).await;
        assert!(m3.tables[0].grid.is_none() && m3.tables[0].image.is_some());
        assert!(rep3.notes.iter().any(|n| n.contains("改放表格的截圖")), "{:?}", rep3.notes);

        // no engine: nothing is confirmed, the detector's material stays, said so
        let none = Llm::none(lore_home::Home::at(dir.path()));
        let inp4 = Input { pages: 2, page_png: &page_png, table_grid: &table_grid, typeset: None, cache: dir.path().join("cache-none"), hints: vec![] };
        let mut m4 = material();
        let rep4 = verify(&none, &mut m4, &inp4).await;
        assert!(rep4.unverified && m4.figures.len() == 2 && rep4.notes.len() == 1 && rep4.notes[0].contains("沒有經過確認"));
    }

    /// Real Chrome (not run offline): `cargo test -p sc-server --lib chrome_typeset -- --ignored`
    /// writes the picture to `$SC_TYPESET_OUT` when set.
    #[test]
    #[ignore]
    fn chrome_typeset_draws_an_equation() {
        let t = ChromeTypeset { web: crate::repo_dir().join("web") };
        let png = t.png("S = \\frac{1}{K} \\sum_{k=1}^{K} \\hat{z}_k").unwrap();
        assert!(crate::reader::png_size(&png).is_some());
        if let Ok(out) = std::env::var("SC_TYPESET_OUT") {
            std::fs::write(out, &png).unwrap();
        }
    }

    #[test]
    fn numbers_and_shouting() {
        assert_eq!((arabic("I"), arabic("IV"), arabic("XII"), arabic("3"), arabic("A1")), ("1".into(), "4".into(), "12".into(), "3".into(), "a1".into()));
        assert!(shouting("DICE PER SITE ON THE SYNTHETIC SET") && !shouting("Dice per site (FFT, CNN)"));
        assert!(has_control("a \u{8} b") && !has_control("a\nb"));
        assert_eq!(clean_latex("$$ x = 1, \\tag{3}$$"), "x = 1");
    }
}
