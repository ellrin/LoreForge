//! `write`: the chapters, one model call each (`pc-book-chapter`), into the book's LaTeX folder
//! through the paper's own paths — sections (`paper::add_section`, `render_section`), figures and
//! tables (`crate::floats`, the editor's 新增圖… / 繪圖… / 新增表… path), references (the
//! workspace library, `reader::file_entry`, then `write_paper_bib` as 文獻 does), symbols
//! (`reader::register_into` into the book's notation.json; notation.tex regenerated only by
//! lore-notation and only while it is the generated file) — and the registry's checks per chapter
//! (`notation::check`: a typed "Figure 1", an unknown `\ref`… asked once more, then listed).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::model::{Ask, BookModel, TASK_CHAPTER};
use super::plan::{Chapter, Plan, Teach};
use super::research::Research;
use super::source::Material;

/// `write.state.json` in the job folder: what is made already, so a resumed stage goes on.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    pub book: Option<PathBuf>,
    pub new_book: bool,
    /// chapter id → (section file, label)
    pub sections: BTreeMap<String, (String, String)>,
    /// chapter id → its report
    pub done: BTreeMap<String, ChapterOut>,
    /// source key in research → library key
    pub keys: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ChapterOut {
    pub id: String,
    pub title: String,
    pub file: String,
    pub label: String,
    pub figures: Vec<String>,
    pub tables: usize,
    pub cites: usize,
    pub engine: String,
    /// What the registry checks still say, and what was left out.
    pub problems: Vec<String>,
}

/// `write.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WriteOut {
    pub book: String,
    pub title: String,
    pub new_book: bool,
    pub chapters: Vec<ChapterOut>,
    pub notation: usize,
    pub notes: Vec<String>,
}

pub struct Ctx<'a> {
    pub model: &'a dyn BookModel,
    pub tools: &'a dyn super::tools::Tools,
    pub ws: Option<&'a Path>,
    pub global: &'a lore_notation::Notation,
    pub plot_styles: &'a Value,
    pub engine: Option<&'a str>,
    pub length: &'a str,
    pub lang: &'a str,
    pub job_dir: &'a Path,
    pub on_chars: &'a (dyn Fn(u64) + Send + Sync),
    pub progress: &'a (dyn Fn(Value) + Send + Sync),
}

pub fn words_for(depth: &str, length: &str) -> (usize, usize) {
    let (a, b) = match depth {
        "brief" => (400, 700),
        "deep" => (1500, 2200),
        _ => (900, 1400),
    };
    let f = match length {
        "short" => 0.7,
        "long" => 1.3,
        _ => 1.0,
    };
    ((a as f64 * f) as usize, (b as f64 * f) as usize)
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["body"],
        "properties": {
            "body": { "type": "string" },
            "figures": { "type": "array", "items": {
                "type": "object", "required": ["id", "kind", "caption"],
                "properties": {
                    "id": { "type": "string" },
                    "kind": { "type": "string", "enum": ["plot", "paper"] },
                    "caption": { "type": "string" },
                    "paper_fig": { "type": "string" },
                    "plot": { "type": "object", "properties": {
                        "chart": { "type": "string", "enum": ["line", "bar", "scatter"] },
                        "x_label": { "type": "string" }, "y_label": { "type": "string" },
                        "x": { "type": "array" },
                        "series": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "y": { "type": "array" } } } }
                    } }
                }
            } },
            "tables": { "type": "array", "items": {
                "type": "object", "required": ["id", "caption", "rows"],
                "properties": { "id": { "type": "string" }, "caption": { "type": "string" }, "rows": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } } }
            } }
        }
    })
}

fn lang_rules(lang: &str) -> &'static str {
    match lang {
        "en" => "Write in English. Give each key term with \\term{English term}{} at its first mention. No margin notes.",
        "zh" => "Write the text in Traditional Chinese (Taiwan usage). Give each key term with \\term{English term}{中文} at its first mention. \
Keep symbols and equations in math mode.",
        _ => "Write the main text in English. Chinese is a help for the reader: at the first mention of each key term write \\term{English term}{中文名稱} \
(Traditional Chinese, Taiwan usage); for 1–3 key points of the chapter add \\sidenote{…} with a short Traditional Chinese explanation, \
placed right after the sentence it explains and never inside a box.",
    }
}

/// The chapter prompt.
#[allow(clippy::too_many_arguments)]
pub fn prompt(book_title: &str, c: &Chapter, plan: &Plan, labels: &BTreeMap<String, (String, String)>, teach: &[Teach], m: &Material, r: &Research, keys: &BTreeMap<String, String>, notation: &str, length: &str, lang: &str) -> (String, String) {
    let (lo, hi) = words_for(&c.depth, length);
    let mut system = format!(
        "You write one chapter of a set of course lecture notes for one reader, in the manner of university lecture notes such as \
Stanford CS224n's: a textbook tone, from the basics to depth, intuition before equations, worked reasoning rather than lists.\n\
Rules:\n\
- Explain every listed term. A term with state `unknown` gets a full explanation (what it is, why it is needed, how it works, an example); \
a term with state `learning` gets a short reminder, never skipped. Start from what the reader already knows.\n\
- Every factual claim carries a citation \\cite{{key}} with a key from the given sources only. Never invent a key, a number or a result. \
If no given source supports a statement, leave the statement out.\n\
- LaTeX for the lecture-notes template: \\subsection{{…}} for the parts of the chapter (no \\section: the chapter heading is written for you); \
\\begin{{definition}}[name]…\\end{{definition}} for definitions, \\begin{{intuition}}…\\end{{intuition}} before an equation, \
\\begin{{keyidea}}…\\end{{keyidea}} for the take-away, \\begin{{remark}}…\\end{{remark}} for asides. Boxes hold a few sentences and never a \\sidenote.\n\
- Variables and equations in TeX math: inline $…$, displayed \\begin{{equation}}…\\label{{eq:…}}\\end{{equation}}.\n\
- {lang}\n\
- Length: {lo}–{hi} words of text.\n\
- Label an equation (\\label{{eq:…}}) only when the text refers to it with \\eqref{{…}}.\n\
- Figures: draw each planned figure unless it would not help; put a line `%%FIGURE:<id>` where the figure goes and refer to it once as \
(Fig.~\\ref{{fig:<id>}}). `plot` figures draw a mathematical function or numbers that a source states (cite it in the caption): give \
`plot` = {{chart, x_label, y_label, x: [...], series: [{{name, y: [...]}}]}}, every y as long as x, labels may use $…$. `paper` figures \
reproduce the paper's figure `paper_fig` (its licence allows it); the caption says what to look at.\n\
- Tables only for a small comparison the sources give: a line `%%TABLE:<id>`, referred to as (Table~\\ref{{tab:<id>}}), rows of strings, \
the first row the header.\n\
- Other chapters are referred to only with \\ref{{label}} (Section~\\ref{{sec:…}}); never type a number such as \"Section 2\" or \"Figure 1\".\n\
Reply with JSON only: {{\"body\": LaTeX of the chapter, \"figures\": [...], \"tables\": [...]}}.\n",
        lang = lang_rules(lang)
    );
    system += notation;
    let mut user = format!("Notes: {book_title}\nThese notes teach the paper \"{}\".\n\nChapters (label):\n", m.title);
    for x in &plan.chapters {
        let l = labels.get(&x.id).map(|(_, l)| l.as_str()).unwrap_or("");
        user += &format!("- {}{} ({l})\n", x.title, if x.id == c.id { "  ← this chapter" } else { "" });
    }
    user += &format!("\nThis chapter: {}\nDepth: {}\n", c.title, c.depth);
    if !c.goal.is_empty() {
        user += &format!("Goal: {}\n", c.goal);
    }
    user += "Terms to explain (state):\n";
    for t in &c.terms {
        let st = teach.iter().find(|x| x.term.eq_ignore_ascii_case(t)).map(|x| x.state.as_str()).unwrap_or("unknown");
        user += &format!("- {t} ({st})\n");
    }
    user += "\nSources you may cite (key — what it is):\n";
    for s in r.for_chapter(&c.id) {
        let key = keys.get(&s.key).unwrap_or(&s.key);
        let rec = &s.record;
        let who = rec.authors.first().map(|a| format!("{a}{}", if rec.authors.len() > 1 { " et al." } else { "" })).unwrap_or_default();
        user += &format!("\n[{key}] {} — {who} {}\n", rec.title, rec.year.map(|y| y.to_string()).unwrap_or_default());
        if let Some(a) = rec.abstract_text.as_deref().filter(|a| !a.is_empty()) {
            user += &format!("Abstract: {}\n", a.chars().take(1200).collect::<String>());
        }
        for q in s.quotes.iter().take(6) {
            user += &format!("Quote: {q}\n");
        }
        if s.kind == "paper" {
            let mut words = c.terms.clone();
            words.extend(c.terms.iter().flat_map(|t| t.split_whitespace().map(String::from)).filter(|w| w.len() > 4));
            let p = m.passages(&words, 6000);
            if !p.is_empty() {
                user += &format!("Passages of this paper about the chapter's terms:\n{p}\n");
            }
        }
    }
    let cards: Vec<_> = r.cards.iter().filter(|k| k.chapters.contains(&c.id)).collect();
    if !cards.is_empty() {
        user += "\nThe reader's own knowledge-base notes (context only, not citable):\n";
        for k in cards {
            user += &format!("- {}: {}\n", k.title, k.summary.chars().take(400).collect::<String>());
        }
    }
    let qs: Vec<&str> = m.questions.iter().map(|q| q.text.as_str()).chain(m.extras.questions.iter().map(String::as_str)).filter(|q| c.terms.iter().any(|t| q.to_lowercase().contains(&t.to_lowercase()))).collect();
    if !qs.is_empty() {
        user += "\nQuestions the reader asked about these terms (answer them in the text):\n";
        for q in qs {
            user += &format!("- {q}\n");
        }
    }
    if !c.figures.is_empty() {
        user += "\nPlanned figures:\n";
        for (i, f) in c.figures.iter().enumerate() {
            match f.kind.as_str() {
                "paper" => {
                    let cap = m.figures.iter().find(|x| x.id == f.paper_fig).map(|x| x.caption.as_str()).unwrap_or("");
                    user += &format!("- id f{} kind paper, paper_fig {}: {} (the paper's caption: {cap})\n", i + 1, f.paper_fig, f.idea);
                }
                k => user += &format!("- id f{} kind {k}: {}\n", i + 1, f.idea),
            }
        }
    }
    (system, user)
}

// ── turning the reply into the chapter ───────────────────────────────────────

/// `\cite`-like commands keep only `allowed` keys (mapped to the library's keys); one without any
/// key is removed with the space before it. Returns the text, the keys kept, the keys dropped.
pub fn filter_cites(text: &str, allowed: &BTreeMap<String, String>) -> (String, BTreeSet<String>, BTreeSet<String>) {
    let (mut kept, mut dropped) = (BTreeSet::new(), BTreeSet::new());
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("\\cite") {
        let name_end = i + 1 + rest[i + 1..].find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len() - i - 1);
        let mut j = name_end;
        if rest[j..].starts_with('*') {
            j += 1;
        }
        while rest[j..].starts_with('[') {
            match rest[j..].find(']') {
                Some(e) => j += e + 1,
                None => break,
            }
        }
        if !rest[j..].starts_with('{') {
            out += &rest[..name_end];
            rest = &rest[name_end..];
            continue;
        }
        let Some(close) = rest[j..].find('}') else { break };
        let keys: Vec<String> = rest[j + 1..j + close].split(',').map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect();
        let good: Vec<String> = keys.iter().filter_map(|k| allowed.get(k).cloned()).collect();
        for k in &keys {
            if !allowed.contains_key(k) {
                dropped.insert(k.clone());
            }
        }
        let before = &rest[..i];
        if good.is_empty() {
            out += before.trim_end_matches([' ', '~']);
        } else {
            out += before;
            out += &rest[i..j + 1];
            out += &good.join(",");
            out += "}";
            kept.extend(good);
        }
        rest = &rest[j + close + 1..];
    }
    out += rest;
    (out, kept, dropped)
}

/// Lines starting with `\section` dropped (the heading is written for the chapter).
fn without_sections(body: &str) -> String {
    body.lines().filter(|l| !l.trim_start().starts_with("\\section")).collect::<Vec<_>>().join("\n")
}

/// Put `block` where the marker line is (else at the end) and point `\ref{<prefix>:<id>}` at `label`.
fn place(body: &str, marker: &str, block: &str, ph: &str, label: &str) -> String {
    let mut done = false;
    let mut lines: Vec<String> = vec![];
    for l in body.lines() {
        if !done && l.trim() == marker {
            lines.push(String::new());
            lines.push(block.to_string());
            lines.push(String::new());
            done = true;
        } else if l.trim() != marker {
            lines.push(l.to_string());
        }
    }
    let mut text = lines.join("\n");
    if !done {
        text = format!("{}\n\n{block}\n", text.trim_end());
    }
    text.replace(&format!("\\ref{{{ph}}}"), &format!("\\ref{{{label}}}"))
}

/// The marker and the reference to a figure that could not be made are taken out.
fn drop_float(body: &str, marker: &str, ph: &str) -> String {
    let mut text: String = body.lines().filter(|l| l.trim() != marker).collect::<Vec<_>>().join("\n");
    for form in ["Fig.~", "Figure~", "Fig. ", "Figure ", "Table~", "Table "] {
        for (open, close) in [(" (", ")"), ("(", ")")] {
            for see in ["", "see "] {
                text = text.replace(&format!("{open}{see}{form}\\ref{{{ph}}}{close}"), "");
            }
        }
    }
    text
}

/// A lore-plot spec from the model's small plot form.
pub fn plot_spec(p: &Value, caption: &str) -> Result<lore_plot::Spec, String> {
    let x: Vec<Value> = p["x"].as_array().cloned().unwrap_or_default();
    let series: Vec<&Value> = p["series"].as_array().into_iter().flatten().collect();
    if x.len() < 2 || series.is_empty() || x.len() > 500 {
        return Err("繪圖的資料不完整".into());
    }
    let mut columns = vec![Value::from("x")];
    let mut ser = vec![];
    for (i, s) in series.iter().enumerate() {
        let y = s["y"].as_array().ok_or("缺少 y")?;
        if y.len() != x.len() || !y.iter().all(Value::is_number) {
            return Err("y 和 x 的長度不同".into());
        }
        let col = format!("y{}", i + 1);
        columns.push(col.clone().into());
        ser.push(json!({ "y": col, "label": s["name"].as_str().unwrap_or("") }));
    }
    let rows: Vec<Value> = (0..x.len()).map(|r| Value::Array(std::iter::once(x[r].clone()).chain(series.iter().map(|s| s["y"][r].clone())).collect())).collect();
    let chart = match p["chart"].as_str() {
        Some("bar") => "bar",
        Some("scatter") => "scatter",
        _ => "line",
    };
    let panel = if chart == "scatter" {
        json!({ "type": "scatter", "data": "t", "x": "x", "y": ser[0]["y"], "xaxis": { "label": p["x_label"] }, "yaxis": { "label": p["y_label"] } })
    } else {
        json!({ "type": chart, "data": "t", "x": "x", "series": ser, "xaxis": { "label": p["x_label"] }, "yaxis": { "label": p["y_label"] } })
    };
    let spec = json!({ "lore_plot": 1, "style": "clean", "size": { "w": 90, "h": 60 }, "data": { "t": { "name": "notes", "columns": columns, "rows": rows } }, "panels": [panel], "caption": caption });
    let s = lore_plot::Spec::from_value(spec).map_err(|e| e.to_string())?;
    s.validate().map_err(|e| e.to_string())?;
    Ok(s)
}

/// The chapter's text with its floats made (files the attempt wrote are listed for a retry).
#[allow(clippy::too_many_arguments)]
fn assemble(ctx: &Ctx<'_>, book: &Path, reply: &Value, c: &Chapter, m: &Material, r: &Research, keys: &BTreeMap<String, String>, made: &mut Vec<PathBuf>, out: &mut ChapterOut) -> String {
    let mut body = without_sections(reply["body"].as_str().unwrap_or(""));
    let mut new_labels: Vec<String> = vec![];
    for f in reply["figures"].as_array().into_iter().flatten() {
        let id: String = f["id"].as_str().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let (marker, ph) = (format!("%%FIGURE:{id}"), format!("fig:{id}"));
        let caption = f["caption"].as_str().unwrap_or("").trim().to_string();
        let made_file: Result<String, String> = match f["kind"].as_str() {
            Some("plot") => plot_spec(&f["plot"], &caption).and_then(|spec| {
                let stem = super::super::floats::free_figure_name(book, if caption.is_empty() { &id } else { &caption }, "pdf");
                let file = ctx.tools.plot(&book.join("figures"), stem.trim_end_matches(".pdf"), &spec, ctx.plot_styles)?;
                made.push(book.join("figures").join(&file));
                made.push(book.join("figures").join(file.replace(".pdf", ".plot.json")));
                Ok(format!("figures/{file}"))
            }),
            Some("paper") if r.reuse_figures => {
                let want = f["paper_fig"].as_str().unwrap_or("");
                match m.figures.iter().find(|x| x.id == want).and_then(|x| x.file.clone()).and_then(|file| std::fs::read(ctx.job_dir.join("img").join(file)).ok()) {
                    Some(bytes) => {
                        let name = super::super::floats::free_figure_name(book, &format!("paper {want}"), "png");
                        super::super::floats::save_figure_file(book, &name, &bytes).inspect(|_| made.push(book.join("figures").join(&name)))
                    }
                    None => Err("論文沒有這張圖".into()),
                }
            }
            _ => Err("不能用的圖".into()),
        };
        let caption = match f["kind"].as_str() {
            // attribution: the paper and its licence
            // «Reproduced from [n], CC BY-NC 4.0.» — always the citation and the licence
            Some("paper") => reused_caption(&caption, &r.start_key, r.license.as_deref()),
            _ => caption,
        };
        match made_file.and_then(|path| super::super::floats::figure_block(ctx.ws, book, &path, &caption, &caption, &new_labels)) {
            Ok((label, block, _)) => {
                body = place(&body, &marker, &block, &ph, &label);
                new_labels.push(label.clone());
                out.figures.push(label);
            }
            Err(e) => {
                body = drop_float(&body, &marker, &ph);
                out.problems.push(format!("圖 {id} 沒有做：{e}"));
            }
        }
    }
    for t in reply["tables"].as_array().into_iter().flatten() {
        let id: String = t["id"].as_str().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let (marker, ph) = (format!("%%TABLE:{id}"), format!("tab:{id}"));
        let grid: Vec<Vec<String>> = t["rows"].as_array().into_iter().flatten().map(|row| row.as_array().into_iter().flatten().map(|c| c.as_str().unwrap_or("").to_string()).collect()).filter(|r: &Vec<String>| !r.is_empty()).collect();
        let res = if grid.len() < 2 { Err("表格沒有內容".to_string()) } else { super::super::floats::table_block(ctx.ws, book, &grid, t["caption"].as_str().unwrap_or(""), &new_labels) };
        match res {
            Ok((label, block, _)) => {
                body = place(&body, &marker, &block, &ph, &label);
                new_labels.push(label);
                out.tables += 1;
            }
            Err(e) => {
                body = drop_float(&body, &marker, &ph);
                out.problems.push(format!("表 {id} 沒有做：{e}"));
            }
        }
    }
    // citations: only the sources of this chapter, under the library's keys
    let allowed: BTreeMap<String, String> = r.for_chapter(&c.id).iter().map(|s| (s.key.clone(), keys.get(&s.key).cloned().unwrap_or_else(|| s.key.clone()))).flat_map(|(a, b)| [(a, b.clone()), (b.clone(), b)]).collect();
    let (text, kept, dropped) = filter_cites(&body, &allowed);
    out.cites = kept.len();
    if !dropped.is_empty() {
        out.problems.push(format!("拿掉了沒有來源的引用：{}", dropped.into_iter().collect::<Vec<_>>().join(", ")));
    }
    text
}

/// `\label{eq:…}` that nothing in `all` (the whole book) refers to is taken out of `text`, with
/// its line when the label stood alone (a blank line inside an equation would not compile).
fn drop_unused_eq_labels(text: &str, all: &str) -> String {
    let mut lines: Vec<String> = vec![];
    for line in text.lines() {
        let mut l = line.to_string();
        let mut from = 0;
        while let Some(k) = l[from..].find("\\label{eq:") {
            let i = from + k;
            let Some(end) = l[i..].find('}') else { break };
            let label = l[i + 7..i + end].to_string();
            if all.contains(&format!("ref{{{label}}}")) {
                from = i + end;
                continue;
            }
            l.replace_range(i..=i + end, "");
            from = i;
        }
        if !(l.trim().is_empty() && !line.trim().is_empty()) {
            lines.push(l);
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// File every source into the workspace's reference library (the paper's path, deduplicated by
/// DOI and title), returning research key → library key. Without a workspace the research keys stay.
pub async fn file_sources(ws: Option<&Path>, r: &Research) -> Result<BTreeMap<String, String>, String> {
    let mut keys = BTreeMap::new();
    let Some(ws) = ws else {
        for s in &r.sources {
            keys.insert(s.key.clone(), s.key.clone());
        }
        return Ok(keys);
    };
    let _g = crate::LIBRARY_LOCK.lock().await;
    for s in &r.sources {
        let Some(e) = pc_core::bib::parse(&s.bib).0.into_iter().next() else { continue };
        let p = crate::reader::ReaderPaper { title: s.record.title.clone(), ..Default::default() };
        let (k, _) = crate::reader::file_entry(ws, &p, e, s.record.doi.as_deref().unwrap_or(""), &[])?;
        keys.insert(s.key.clone(), k);
    }
    Ok(keys)
}

/// The LaTeX of a field of main.tex, set when the template has it.
/// The caption of a figure reproduced from a paper: always its citation and its licence
/// («… Reproduced from \\cite{key}, CC BY-NC 4.0.»).
pub fn reused_caption(caption: &str, key: &str, license: Option<&str>) -> String {
    let lic = license.and_then(lore_research::licence_label).map(|l| format!(", {l}")).unwrap_or_default();
    let cap = caption.trim().trim_end_matches('.');
    let sep = if cap.is_empty() { "" } else { ". " };
    format!("{cap}{sep}Reproduced from \\cite{{{key}}}{lic}.")
}

/// What the back of the title page says: the plan's abstract, else one built from the plan
/// (the source paper and the chapters in order), so the page is never a lone heading.
pub fn about_text(plan: &Plan, source_title: &str) -> String {
    let a = plan.abstract_text.trim();
    if !a.is_empty() {
        return a.to_string();
    }
    let chapters: Vec<&str> = plan.chapters.iter().map(|c| c.title.trim()).filter(|t| !t.is_empty()).collect();
    let src = source_title.trim();
    let mut s = if src.is_empty() { "These notes explain, one chapter at a time, the terms a paper builds on".to_string() } else { format!("These notes explain, one chapter at a time, the terms behind {src}") };
    if chapters.is_empty() {
        s.push('.');
    } else {
        s += &format!(": {}.", chapters.join("; "));
    }
    pc_core::meta::tex_text(&s)
}

pub(crate) fn set_front(book: &Path, field: &str, value: &str) {
    let path = pc_core::open::main_file(book);
    let Ok(main) = std::fs::read_to_string(&path) else { return };
    let new = match field {
        "course" | "keywords" => {
            let cmd = format!("\\{field}{{");
            match main.find(&cmd) {
                Some(i) => {
                    let j = i + cmd.len();
                    let end = main[j..].find("}\n").map(|e| j + e).unwrap_or(j);
                    format!("{}{}{}", &main[..j], value, &main[end..])
                }
                None => return,
            }
        }
        f => match pc_core::front::set_field(&main, f, 0, value) {
            Ok(m) => m,
            Err(_) => return,
        },
    };
    let _ = pc_core::paper::write_atomic(&path, &new);
}

fn state_file(dir: &Path) -> PathBuf {
    dir.join("write.state.json")
}

fn save_state(dir: &Path, st: &State) {
    let _ = lore_jobs::write_json(&state_file(dir), st);
}

/// Where the book is: an existing one (`book_path`) or a new one made now in `folder`.
pub struct Target<'a> {
    pub existing: Option<&'a Path>,
    pub folder: Option<&'a Path>,
    pub template: &'a (dyn Fn() -> Result<PathBuf, String> + Send + Sync),
    pub page: Option<&'a str>,
    pub books_file: &'a Path,
}

pub async fn write(ctx: &Ctx<'_>, target: Target<'_>, m: &Material, plan: &Plan, r: &Research, teach: &[Teach]) -> Result<WriteOut, lore_jobs::StageError> {
    let err = |e: String| lore_jobs::StageError::new(422, e);
    let mut st: State = std::fs::read(state_file(ctx.job_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut notes = vec![];
    // the book folder
    let book = match (&st.book, target.existing) {
        (Some(b), _) if b.is_dir() => b.clone(),
        (_, Some(b)) => {
            if !pc_core::open::is_paper(b) {
                return Err(err(format!("找不到這本書：{}", b.display())));
            }
            b.to_path_buf()
        }
        (_, None) => {
            let folder = target.folder.ok_or_else(|| err("沒有存放新書的資料夾".into()))?;
            std::fs::create_dir_all(folder).map_err(|e| err(format!("無法建立資料夾 {}：{e}", folder.display())))?;
            let title = if plan.title.trim().is_empty() { format!("Notes on {}", m.title) } else { plan.title.trim().to_string() };
            let tpl = (target.template)().map_err(err)?;
            let dir = crate::books::new_book(folder, &title, &tpl, target.page).map_err(err)?;
            st.new_book = true;
            dir
        }
    };
    st.book = Some(book.clone());
    save_state(ctx.job_dir, &st);
    let title = std::fs::read_to_string(pc_core::open::main_file(&book)).ok().and_then(|main| {
        let i = main.find("\\title{")? + 7;
        Some(main[i..i + main[i..].find('}')?].to_string())
    }).unwrap_or_default();
    if st.new_book {
        if let Some(p) = target.page {
            let _ = pc_core::book::set_page(&book, p);
        }
        // the template's empty first section gives way to the chapters
        let secs = pc_core::open::sections(&book);
        if secs.len() == 1 && st.sections.is_empty() {
            let f = &secs[0].file;
            let text = std::fs::read_to_string(book.join("sections").join(f)).unwrap_or_default();
            if pc_core::paper::parse_section(f, &text).paragraphs.is_empty() {
                let _ = pc_core::paper::delete_section(&book, f);
            }
        }
        set_front(&book, "course", "Reading notes");
        // the back of the title page (About these notes) is never left with a heading and nothing under it
        set_front(&book, "abstract", &about_text(plan, &m.title));
        let kw: Vec<String> = plan.chapters.iter().flat_map(|c| c.terms.iter().take(2).cloned()).take(8).collect();
        set_front(&book, "keywords", &pc_core::meta::tex_text(&kw.join(", ")));
    }
    // the paper's symbols into the book's own registry (LoreReader's ids), as the editor's 插入 does
    let nfile = pc_core::paper::app_dir(&book).join("notation.json");
    let mut doc = lore_notation::Notation::load(&nfile).unwrap_or_default();
    let before = doc.entries.len();
    if let Some(table) = &m.symbols {
        let ids: Vec<String> = table["entries"].as_array().into_iter().flatten().chain(table["equations"].as_array().into_iter().flatten()).filter_map(|e| e["id"].as_str().map(String::from)).collect();
        crate::reader::register_into(&mut doc, ctx.global, table, &ids);
    }
    let notation_added = doc.entries.len() - before;
    if notation_added > 0 {
        doc.save(&nfile).map_err(err)?;
    }
    let eff = lore_notation::Notation::effective(ctx.global, &doc);
    // notation.tex: lore-notation rewrites it only while it is its own generated file
    if std::fs::read_to_string(book.join("notation.tex")).is_ok_and(|t| lore_notation::macros::is_generated(&t)) {
        lore_notation::write_notation_tex(&book, &eff, &Default::default()).map_err(|e| err(e.to_string()))?;
    }
    // references into the library
    if st.keys.is_empty() {
        st.keys = file_sources(ctx.ws, r).await.map_err(err)?;
        save_state(ctx.job_dir, &st);
    }
    // a section per chapter (label known before any chapter is written)
    for c in &plan.chapters {
        if st.sections.contains_key(&c.id) {
            continue;
        }
        let after = pc_core::paper::section_files(&book).last().cloned();
        let id = pc_core::paper::add_section(&book, &c.title, after.as_deref()).map_err(err)?;
        let text = std::fs::read_to_string(book.join("sections").join(&id)).unwrap_or_default();
        let tail = pc_core::paper::parse_section(&id, &text).tail;
        let label = tail.split("\\label{").nth(1).and_then(|x| x.split('}').next()).unwrap_or("").to_string();
        st.sections.insert(c.id.clone(), (id, label));
        save_state(ctx.job_dir, &st);
    }
    let notation_rules = crate::notation::describe(&eff, None);
    let opt = lore_notation::CheckOptions::default();
    let total = plan.chapters.len();
    for (i, c) in plan.chapters.iter().enumerate() {
        (ctx.progress)(json!({ "chapter": i, "chapters": total }));
        if st.done.contains_key(&c.id) {
            continue;
        }
        let (file, label) = st.sections[&c.id].clone();
        let (system, user) = prompt(&title, c, plan, &st.sections, teach, m, r, &st.keys, &notation_rules, ctx.length, ctx.lang);
        let mut extra = String::new();
        let mut co = ChapterOut::default();
        let mut text = String::new();
        let mut made: Vec<PathBuf> = vec![];
        for attempt in 0..2 {
            for f in made.drain(..) {
                let _ = std::fs::remove_file(f);
            }
            let ask = Ask { task: TASK_CHAPTER, system: system.clone(), user: format!("{user}{extra}"), schema: schema(), max_tokens: 16000, engine: ctx.engine };
            let (reply, engine) = ctx.model.json(ask, ctx.on_chars).await?;
            co = ChapterOut { id: c.id.clone(), title: c.title.clone(), file: file.clone(), label: label.clone(), engine, ..Default::default() };
            text = assemble(ctx, &book, &reply, c, m, r, &st.keys, &mut made, &mut co);
            let checked = crate::notation::check(&book, &eff, &opt, &[], &text);
            if checked.blocking.is_empty() || attempt == 1 {
                co.problems.extend(checked.blocking.iter().map(|b| format!("對照表：{b}")));
                if !checked.new_symbols.is_empty() {
                    let _ = crate::notation::register(&book, &checked.new_symbols);
                    co.problems.push(format!("新符號登記為待確認：{}", checked.new_symbols.join(" ")));
                }
                break;
            }
            extra = format!("\n\nYour last reply had these problems; write the chapter again without them:\n- {}\n", checked.blocking.join("\n- "));
        }
        let sec = pc_core::paper::Section { file: file.clone(), title: c.title.clone(), paragraphs: pc_core::text::paragraphs(&text), tail: format!("\\label{{{label}}}") };
        pc_core::paper::write_atomic(&book.join("sections").join(&file), &pc_core::paper::render_section(&sec)).map_err(|e| err(e.to_string()))?;
        st.done.insert(c.id.clone(), co);
        save_state(ctx.job_dir, &st);
        (ctx.progress)(json!({ "chapter": i + 1, "chapters": total, "figures": st.done.values().map(|d| d.figures.len()).sum::<usize>() }));
    }
    // equation labels nothing in the book refers to (the registry flags them): out, across chapters
    let all: String = pc_core::open::source_files(&book).iter().filter_map(|f| std::fs::read_to_string(f).ok()).collect::<Vec<_>>().join("\n");
    for (f, _) in st.sections.values() {
        let path = book.join("sections").join(f);
        if let Ok(t) = std::fs::read_to_string(&path) {
            let n = drop_unused_eq_labels(&t, &all);
            if n != t {
                pc_core::paper::write_atomic(&path, &n).map_err(|e| err(e.to_string()))?;
            }
        }
    }
    // the book's bibliography from what the chapters cite (文獻's own step)
    if let Some(ws) = ctx.ws {
        match crate::write_paper_bib(ws, &book) {
            Ok(b) if !b.missing.is_empty() => notes.push(format!("文獻庫沒有：{}", b.missing.join(", "))),
            Ok(_) => {}
            Err(e) => notes.push(format!("refs：{e}")),
        }
    } else {
        // no workspace (tests without one): the entries go into the book's refs.bib directly
        let bib: String = r.sources.iter().map(|s| s.bib.clone()).collect::<Vec<_>>().join("\n");
        let path = book.join("refs.bib");
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        let have: Vec<String> = pc_core::bib::parse(&old).0.into_iter().map(|e| e.key).collect();
        let add: String = pc_core::bib::parse(&bib).0.into_iter().filter(|e| !have.contains(&e.key)).map(|e| pc_core::bib::write_entry(&e)).collect::<Vec<_>>().join("\n");
        pc_core::paper::write_atomic(&path, &format!("{old}\n{add}")).map_err(|e| err(e.to_string()))?;
    }
    crate::books::remember_in(target.books_file, &book, &title, Some(&m.sha), if st.new_book { book.parent() } else { None }).map_err(err)?;
    let chapters = plan.chapters.iter().filter_map(|c| st.done.get(&c.id).cloned()).collect();
    Ok(WriteOut { book: book.to_string_lossy().to_string(), title, new_book: st.new_book, chapters, notation: notation_added, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cites_keep_only_given_sources() {
        let allowed: BTreeMap<String, String> = [("doi:10.5555/a".to_string(), "ellrin2024gates".to_string()), ("ellrin2024gates".to_string(), "ellrin2024gates".to_string())].into();
        let (t, kept, dropped) = filter_cites("A claim~\\cite{doi:10.5555/a,made-up}. Another \\citep[p.~3]{made-up}. Third \\cite{ellrin2024gates}.", &allowed);
        assert_eq!(t, "A claim~\\cite{ellrin2024gates}. Another. Third \\cite{ellrin2024gates}.");
        assert_eq!(kept.into_iter().collect::<Vec<_>>(), ["ellrin2024gates"]);
        assert_eq!(dropped.into_iter().collect::<Vec<_>>(), ["made-up"]);
    }

    #[test]
    fn floats_land_at_their_markers_or_leave_cleanly() {
        let body = "Text (Fig.~\\ref{fig:f1}) more.\n%%FIGURE:f1\nAfter.";
        let placed = place(body, "%%FIGURE:f1", "\\begin{figure}X\\end{figure}", "fig:f1", "fig:softmax-shape");
        assert!(placed.contains("(Fig.~\\ref{fig:softmax-shape})") && placed.contains("\n\\begin{figure}X\\end{figure}\n") && !placed.contains("%%FIGURE"), "{placed}");
        assert_eq!(drop_float(body, "%%FIGURE:f1", "fig:f1"), "Text more.\nAfter.");
        let spec = plot_spec(&json!({ "chart": "line", "x_label": "$z$", "y_label": "p", "x": [0, 1, 2], "series": [{ "name": "a", "y": [0.1, 0.3, 0.6] }] }), "Softmax").unwrap();
        assert!(spec.to_json().contains("\"type\":\"line\"") || spec.to_json().contains("\"type\": \"line\""));
        assert!(plot_spec(&json!({ "x": [0, 1], "series": [{ "y": [1] }] }), "").is_err());
        let eqs = "\\begin{equation}\na = 1\n\\label{eq:a}\n\\end{equation}\n\\begin{equation}b\\label{eq:b}\\end{equation}\n";
        let book = format!("{eqs} and later \\eqref{{eq:b}}");
        assert_eq!(drop_unused_eq_labels(eqs, &book), "\\begin{equation}\na = 1\n\\end{equation}\n\\begin{equation}b\\label{eq:b}\\end{equation}\n", "no blank line in the equation; eq:b is used in another chapter");
    }
}
