//! The pandoc JSON tree of a Word paper, cleaned for LaTeX (see the module
//! above). Pandoc's JSON: every element is `{"t": kind, "c": content}`;
//! attributes are `[id, [classes], [[key, value]]]`.

use serde_json::{json, Value};
use std::collections::HashMap;

/// Markers written into the text as raw LaTeX and taken out after the
/// split into sections: private-use characters no paper contains.
pub const C_START: char = '\u{E000}';
pub const CLOSE: char = '\u{E001}';
pub const C_END: char = '\u{E002}';
pub const CHANGE: char = '\u{E003}';

#[derive(Debug, Default)]
pub struct Found {
    /// Labels made: `sec:N`, `eq:N`, `fig:N`, `tab:N`.
    pub labels: Vec<String>,
    /// Pictures: (path pandoc extracted, path in the paper).
    pub images: Vec<(String, String)>,
    /// The typed reference list, in order, numbers taken off.
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Who {
    pub author: String,
    pub date: String,
}

#[derive(Debug, Clone)]
pub struct CommentInfo {
    pub author: String,
    pub date: String,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Marks {
    /// Who made the first tracked change of each marked paragraph.
    pub changes: Vec<Who>,
    pub comments: HashMap<String, CommentInfo>,
}

#[derive(Debug, Clone, Copy)]
pub enum Mode {
    Accept,
    Reject,
}

fn t(v: &Value) -> &str {
    v.get("t").and_then(|t| t.as_str()).unwrap_or("")
}

fn attr_classes(attr: &Value) -> Vec<&str> {
    attr.get(1).and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default()
}

fn attr_get<'a>(attr: &'a Value, key: &str) -> Option<&'a str> {
    attr.get(2)?.as_array()?.iter().find(|kv| kv.get(0).and_then(|k| k.as_str()) == Some(key))?.get(1)?.as_str()
}

fn str_el(s: &str) -> Value {
    json!({"t": "Str", "c": s})
}

fn raw_inline(s: &str) -> Value {
    json!({"t": "RawInline", "c": ["latex", s]})
}

/// Plain text of inlines or blocks.
pub fn stringify(v: &Value) -> String {
    let mut out = String::new();
    fn go(v: &Value, out: &mut String) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| go(x, out)),
            Value::Object(_) => match t(v) {
                "Str" => out.push_str(v["c"].as_str().unwrap_or("")),
                "Space" | "SoftBreak" | "LineBreak" => out.push(' '),
                "Math" | "Code" => out.push_str(v["c"][1].as_str().unwrap_or("")),
                "RawInline" | "RawBlock" => {}
                "Para" | "Plain" | "Header" => {
                    if !out.is_empty() && !out.ends_with(' ') {
                        out.push(' ');
                    }
                    go(&v["c"], out);
                }
                _ => go(&v["c"], out),
            },
            _ => {}
        }
    }
    go(v, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Cleaning ────────────────────────────────────────────

/// A heading level from a Word style name: "Heading1", "heading 2", "Überschrift 1".
fn heading_style(style: &str) -> Option<u64> {
    let s = style.to_lowercase().replace(' ', "");
    let rest = s.strip_prefix("heading").or_else(|| s.strip_prefix("überschrift")).or_else(|| s.strip_prefix("titre"))?;
    rest.parse::<u64>().ok().filter(|n| (1..=6).contains(n))
}

/// A typed section number at the start of a heading: "2.", "2.1", "III.".
fn heading_number(text: &str) -> Option<(String, usize)> {
    let first = text.split_whitespace().next()?;
    let n = first.trim_end_matches('.');
    let arabic = !n.is_empty() && n.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let roman = !n.is_empty() && first.ends_with('.') && n.chars().all(|c| "IVXLC".contains(c));
    (arabic || roman).then(|| (n.to_string(), first.len()))
}

/// Drop the first `n` characters of text from the start of `inlines`
/// (and the space after them).
fn drop_lead(inlines: &mut Vec<Value>, mut n: usize) {
    while n > 0 && !inlines.is_empty() {
        match t(&inlines[0]) {
            "Str" => {
                let s = inlines[0]["c"].as_str().unwrap_or("").to_string();
                let len = s.chars().count();
                if len <= n {
                    inlines.remove(0);
                    n -= len;
                } else {
                    inlines[0] = str_el(&s.chars().skip(n).collect::<String>());
                    n = 0;
                }
            }
            "Space" | "SoftBreak" => {
                inlines.remove(0);
                n = n.saturating_sub(1);
            }
            _ => break,
        }
    }
    while inlines.first().is_some_and(|x| matches!(t(x), "Space" | "SoftBreak")) {
        inlines.remove(0);
    }
}

/// "Figure 1." / "Fig. 1:" / "Table 2" at the start of a caption: the kind
/// ("fig"/"tab"), the number, and how many characters to drop.
fn caption_number(text: &str) -> Option<(&'static str, String, usize)> {
    let lower = text.to_lowercase();
    let (kind, word) = ["figure", "fig.", "fig", "table", "tab."]
        .iter()
        .find(|w| lower.starts_with(**w))
        .map(|w| (if w.starts_with("fig") { "fig" } else { "tab" }, *w))?;
    let rest = &text[word.len()..];
    let digits: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let mut used = word.len() + (rest.len() - rest.trim_start().len()) + digits.len();
    let after = &text[used..];
    if after.starts_with(['.', ':']) {
        used += 1;
    }
    Some((kind, digits, text[..used].chars().count()))
}

/// The inline list of a caption (the first Para/Plain in its blocks, after
/// unwrapping style Divs).
fn caption_inlines(caption: &mut Value) -> Option<&mut Vec<Value>> {
    let blocks = caption.get_mut(1)?.as_array_mut()?;
    let first = blocks.iter_mut().find(|b| matches!(t(b), "Para" | "Plain"))?;
    first.get_mut("c")?.as_array_mut()
}

fn set_id(attr: &mut Value, id: &str) {
    attr[0] = Value::String(id.into());
}

/// `(12)` alone in a table cell: an equation number.
fn equation_number(cell_blocks: &Value) -> Option<String> {
    let s = stringify(cell_blocks);
    let n = s.strip_prefix('(')?.strip_suffix(')')?.trim();
    (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '.')).then(|| n.to_string())
}

/// The math of a cell that holds nothing else.
fn cell_math(cell_blocks: &Value) -> Option<String> {
    let mut math = Vec::new();
    for b in cell_blocks.as_array()? {
        if !matches!(t(b), "Para" | "Plain") {
            return None;
        }
        for i in b["c"].as_array()? {
            match t(i) {
                "Math" => math.push(i["c"][1].as_str()?.trim().to_string()),
                "Space" | "SoftBreak" => {}
                _ => return None,
            }
        }
    }
    (!math.is_empty()).then(|| math.join(" \\\\\n"))
}

/// A Word equation table (one row: the formula, its typed number) as an
/// `equation` with a label.
fn equation_table(b: &Value) -> Option<(String, String)> {
    let bodies = b["c"][4].as_array()?;
    let rows: Vec<&Value> = bodies.iter().flat_map(|tb| tb[3].as_array().into_iter().flatten()).collect();
    if rows.len() != 1 {
        return None;
    }
    let cells: Vec<&Value> = rows[0][1].as_array()?.iter().map(|c| &c[4]).filter(|c| !stringify(c).is_empty() || cell_math(c).is_some()).collect();
    let math = cells.iter().find_map(|c| cell_math(c))?;
    let n = cells.iter().find_map(|c| equation_number(c))?;
    let label = format!("eq:{n}");
    Some((label.clone(), format!("\\begin{{equation}}\n{math}\n\\label{{{label}}}\n\\end{{equation}}")))
}

fn escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                o.push('\\');
                o.push(c);
            }
            '\\' => o.push_str("\\textbackslash{}"),
            '~' => o.push_str("\\textasciitilde{}"),
            '^' => o.push_str("\\textasciicircum{}"),
            _ => o.push(c),
        }
    }
    o
}

/// Inlines as LaTeX: text, math, emphasis, sub/superscripts; markers and
/// raw LaTeX pass through. Enough for table cells and captions.
fn inline_tex(v: &Value) -> String {
    match v {
        Value::Array(a) => a.iter().map(inline_tex).collect(),
        Value::Object(_) => {
            let c = &v["c"];
            match t(v) {
                "Str" => escape(c.as_str().unwrap_or("")),
                "Space" | "SoftBreak" => " ".into(),
                "LineBreak" => "\\\\ ".into(),
                "Math" => {
                    let m = c[1].as_str().unwrap_or("");
                    if t(&c[0]) == "DisplayMath" { format!("\\[{m}\\]") } else { format!("${m}$") }
                }
                "RawInline" => c[1].as_str().unwrap_or("").to_string(),
                "Emph" => format!("\\emph{{{}}}", inline_tex(c)),
                "Strong" => format!("\\textbf{{{}}}", inline_tex(c)),
                "Superscript" => format!("\\textsuperscript{{{}}}", inline_tex(c)),
                "Subscript" => format!("\\textsubscript{{{}}}", inline_tex(c)),
                "Code" => format!("\\texttt{{{}}}", escape(c[1].as_str().unwrap_or(""))),
                "Span" | "Link" | "Cite" => inline_tex(&c[1]),
                "Quoted" => format!("``{}''", inline_tex(&c[1])),
                "Underline" | "SmallCaps" | "Strikeout" => inline_tex(c),
                "Para" | "Plain" => inline_tex(c),
                _ => escape(&stringify(v)),
            }
        }
        _ => String::new(),
    }
}

fn cell_tex(cell: &Value) -> String {
    let blocks = cell[4].as_array().cloned().unwrap_or_default();
    blocks.iter().map(inline_tex).collect::<Vec<_>>().join(" \\\\ ").trim().to_string()
}

/// A Word table as `table` + `tabular` (booktabs): the header rows, or the
/// first row when Word marked none, above the rule.
fn table_tex(b: &Value) -> Option<String> {
    let rows_of = |v: &Value| -> Vec<Vec<String>> {
        v.as_array().map(|rows| rows.iter().map(|r| r[1].as_array().map(|cs| cs.iter().map(cell_tex).collect()).unwrap_or_default()).collect()).unwrap_or_default()
    };
    let mut head = rows_of(&b["c"][3][1]);
    let mut body: Vec<Vec<String>> = b["c"][4].as_array()?.iter().flat_map(|tb| {
        let mut r = rows_of(&tb[2]);
        r.extend(rows_of(&tb[3]));
        r
    }).collect();
    body.extend(rows_of(&b["c"][5][1]));
    if head.is_empty() && body.len() > 1 {
        head.push(body.remove(0));
    }
    let cols = head.iter().chain(&body).map(|r| r.len()).max()?;
    if cols == 0 {
        return None;
    }
    let line = |r: &Vec<String>| format!("{} \\\\\n", (0..cols).map(|i| r.get(i).cloned().unwrap_or_default()).collect::<Vec<_>>().join(" & "));
    let id = b["c"][0][0].as_str().unwrap_or("");
    let caption = b["c"][1][1].as_array().map(|blocks| blocks.iter().map(inline_tex).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let mut s = String::from("\\begin{table}[t]\n\\centering\n");
    if !caption.trim().is_empty() {
        s += &format!("\\caption{{{}}}\n", caption.trim());
    }
    if !id.is_empty() {
        s += &format!("\\label{{{id}}}\n");
    }
    s += &format!("\\begin{{tabular}}{{{}}}\n\\toprule\n", "l".repeat(cols));
    for r in &head {
        s += &line(r);
    }
    if !head.is_empty() {
        s += "\\midrule\n";
    }
    for r in &body {
        s += &line(r);
    }
    s += "\\bottomrule\n\\end{tabular}\n\\end{table}";
    Some(s)
}

fn is_references_heading(text: &str) -> bool {
    let s = text.trim().trim_end_matches(':').to_lowercase();
    let s = heading_number(&s).map(|(_, n)| s[n..].trim().to_string()).unwrap_or(s);
    ["references", "reference", "bibliography", "literature cited", "參考文獻", "参考文献", "引用文獻"].contains(&s.as_str())
}

/// "[12] A. Writer, …" or "12. A. Writer, …" without its number.
fn strip_ref_number(s: &str) -> String {
    let s = s.trim();
    if let Some(r) = s.strip_prefix('[') {
        if let Some((n, rest)) = r.split_once(']') {
            if n.trim().chars().all(|c| c.is_ascii_digit()) {
                return rest.trim().to_string();
            }
        }
    }
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let rest = &s[digits.len()..];
        if let Some(r) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')')) {
            return r.trim().to_string();
        }
    }
    s.to_string()
}

/// Clean a block list in place (recursing into containers).
fn clean_blocks(blocks: &mut Vec<Value>, found: &mut Found, top: bool) {
    let mut out: Vec<Value> = Vec::with_capacity(blocks.len());
    let mut refs_level: Option<u64> = None;
    let mut queue: std::collections::VecDeque<Value> = std::mem::take(blocks).into();
    while let Some(mut b) = queue.pop_front() {
        // Word paragraph styles: a heading style pandoc does not know becomes a heading,
        // any other style wrapper goes (its blocks take its place)
        if t(&b) == "Div" {
            let style = attr_get(&b["c"][0], "custom-style").map(String::from);
            if let Some(style) = style {
                let mut inner: Vec<Value> = b["c"][1].as_array().cloned().unwrap_or_default();
                if let (Some(level), [only]) = (heading_style(&style), inner.as_slice()) {
                    if matches!(t(only), "Para" | "Plain") {
                        inner = vec![json!({"t": "Header", "c": [level, ["", [], []], only["c"].clone()]})];
                    }
                }
                for x in inner.into_iter().rev() {
                    queue.push_front(x);
                }
                continue;
            }
        }
        // a paragraph holding only a picture, then one that reads "Figure N. …"
        // in a style pandoc does not know as a caption: a figure
        if matches!(t(&b), "Para" | "Plain") && image_only(&b) {
            let next = queue.front().map(|n| if t(n) == "Div" && attr_get(&n["c"][0], "custom-style").is_some() { n["c"][1].get(0).cloned().unwrap_or(Value::Null) } else { n.clone() });
            if let Some(cap) = next.filter(|n| matches!(t(n), "Para" | "Plain") && caption_number(&stringify(n)).is_some_and(|(k, _, _)| k == "fig")) {
                queue.pop_front();
                queue.push_front(json!({"t": "Figure", "c": [["", [], []], [null, [{"t": "Plain", "c": cap["c"].clone()}]], [{"t": "Plain", "c": b["c"].clone()}]]}));
                continue;
            }
        }
        // a table's caption also written as a paragraph beside it: once is enough
        if t(&b) == "Table" {
            let cap = stringify(&b["c"][1][1]);
            let same = |n: &Value| !cap.is_empty() && matches!(t(n), "Para" | "Plain" | "Div") && stringify(n) == cap;
            if out.last().is_some_and(same) {
                out.pop();
            }
            if queue.front().is_some_and(same) {
                queue.pop_front();
            }
        }
        // a picture in a one-cell table with a "Figure N" caption (how pandoc
        // and many templates write figures): a figure
        if t(&b) == "Table" && has_image(&b) {
            unwrap_caption_divs(&mut b["c"][1]);
            if caption_number(&stringify(&b["c"][1][1])).is_some_and(|(k, _, _)| k == "fig") {
                let mut inner = Vec::new();
                collect_cell_blocks(&b["c"][4], &mut inner);
                queue.push_front(json!({"t": "Figure", "c": [b["c"][0].clone(), b["c"][1].clone(), inner]}));
                continue;
            }
        }
        match t(&b) {
            "Table" => {
                if let Some((label, tex)) = equation_table(&b) {
                    found.labels.push(label);
                    b = json!({"t": "RawBlock", "c": ["latex", tex]});
                } else {
                    unwrap_caption_divs(&mut b["c"][1]);
                    let cap_text = stringify(&b["c"][1][1]);
                    if let Some((kind, n, len)) = caption_number(&cap_text).filter(|(k, _, _)| *k == "tab") {
                        if let Some(i) = caption_inlines(&mut b["c"][1]) {
                            drop_lead(i, len);
                        }
                        let label = format!("{kind}:{n}");
                        set_id(&mut b["c"][0], &label);
                        found.labels.push(label);
                    }
                    unwrap_caption_divs(&mut b["c"][1]);
                    recurse(&mut b, found);
                }
            }
            "Figure" => {
                unwrap_caption_divs(&mut b["c"][1]);
                let cap_text = stringify(&b["c"][1][1]);
                if let Some((kind, n, len)) = caption_number(&cap_text).filter(|(k, _, _)| *k == "fig") {
                    if let Some(i) = caption_inlines(&mut b["c"][1]) {
                        drop_lead(i, len);
                    }
                    let label = format!("{kind}:{n}");
                    set_id(&mut b["c"][0], &label);
                    found.labels.push(label);
                }
                recurse(&mut b, found);
                // full width, no fixed height
                fix_figure_images(&mut b["c"][2]);
            }
            _ => recurse(&mut b, found),
        }
        push_block(b, &mut out, &mut refs_level, found, top);
    }
    *blocks = out;
}

fn image_only(b: &Value) -> bool {
    let inl = b["c"].as_array().cloned().unwrap_or_default();
    inl.iter().any(|x| t(x) == "Image") && inl.iter().all(|x| matches!(t(x), "Image" | "Space" | "SoftBreak" | "LineBreak"))
}

fn has_image(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.iter().any(has_image),
        Value::Object(_) if t(v) == "Image" => true,
        Value::Object(o) => o.get("c").is_some_and(has_image),
        _ => false,
    }
}

/// The blocks of every cell of a table body list, in order.
fn collect_cell_blocks(bodies: &Value, out: &mut Vec<Value>) {
    for body in bodies.as_array().into_iter().flatten() {
        for rows in [&body[2], &body[3]] {
            for row in rows.as_array().into_iter().flatten() {
                for cell in row[1].as_array().into_iter().flatten() {
                    out.extend(cell[4].as_array().cloned().unwrap_or_default());
                }
            }
        }
    }
}

fn unwrap_caption_divs(caption: &mut Value) {
    if let Some(blocks) = caption.get_mut(1).and_then(|b| b.as_array_mut()) {
        let mut out = Vec::new();
        for b in std::mem::take(blocks) {
            if t(&b) == "Div" {
                out.extend(b["c"][1].as_array().cloned().unwrap_or_default());
            } else {
                out.push(b);
            }
        }
        *blocks = out;
    }
}

fn fix_figure_images(v: &mut Value) {
    match v {
        Value::Array(a) => a.iter_mut().for_each(fix_figure_images),
        Value::Object(_) if t(v) == "Image" => {
            v["c"][0][2] = json!([["width", "100%"]]);
        }
        Value::Object(o) => {
            if let Some(c) = o.get_mut("c") {
                fix_figure_images(c);
            }
        }
        _ => {}
    }
}

/// Headings get their typed number as a label; the reference list is taken out.
fn push_block(mut b: Value, out: &mut Vec<Value>, refs_level: &mut Option<u64>, found: &mut Found, top: bool) {
    if t(&b) == "Header" {
        let level = b["c"][0].as_u64().unwrap_or(1);
        if refs_level.is_some_and(|l| level <= l) {
            *refs_level = None;
        }
        let text = stringify(&b["c"][2]);
        if top && is_references_heading(&text) {
            *refs_level = Some(level);
            return;
        }
        if let Some((n, len)) = heading_number(&text) {
            if let Some(i) = b["c"][2].as_array_mut() {
                drop_lead(i, len);
            }
            if level == 1 || n.contains('.') {
                let label = format!("sec:{n}");
                if !found.labels.contains(&label) {
                    set_id(&mut b["c"][1], &label);
                    found.labels.push(label);
                }
            }
        } else {
            set_id(&mut b["c"][1], "");
        }
    }
    if refs_level.is_some() {
        match t(&b) {
            "OrderedList" | "BulletList" => {
                for item in b["c"].as_array().and_then(|c| c.last()).and_then(|x| x.as_array()).cloned().unwrap_or_default() {
                    let s = stringify(&item);
                    if !s.is_empty() {
                        found.references.push(strip_ref_number(&s));
                    }
                }
            }
            _ => {
                let s = stringify(&b);
                if !s.is_empty() {
                    found.references.push(strip_ref_number(&s));
                }
            }
        }
        return;
    }
    out.push(b);
}

fn recurse(v: &mut Value, found: &mut Found) {
    match v {
        Value::Array(a) => {
            // a list of blocks: clean it as one
            if a.iter().any(|x| matches!(t(x), "Para" | "Plain" | "Div" | "Table" | "Figure" | "Header")) {
                clean_blocks(a, found, false);
            } else {
                a.iter_mut().for_each(|x| recurse(x, found));
            }
        }
        Value::Object(o) => {
            if o.get("t").and_then(|t| t.as_str()) == Some("Image") {
                // media/image1.png → figures/image1.png
                let src = v["c"][2][0].as_str().unwrap_or("").to_string();
                let name = src.rsplit('/').next().unwrap_or("").to_string();
                if !name.is_empty() && !src.starts_with("http") {
                    let from = src.trim_start_matches("./").to_string();
                    let to = format!("figures/{name}");
                    if !found.images.iter().any(|(f, _)| *f == from) {
                        found.images.push((from, to.clone()));
                    }
                    v["c"][2][0] = Value::String(to);
                }
                return;
            }
            // character styles
            if o.get("t").and_then(|t| t.as_str()) == Some("Span") && attr_get(&v["c"][0], "custom-style").is_some() && attr_classes(&v["c"][0]).is_empty() {
                v["c"][0] = json!(["", [], []]);
            }
            if let Some(c) = v.get_mut("c") {
                recurse(c, found);
            }
        }
        _ => {}
    }
}

/// Every table as a float with a booktabs tabular, as PaperComposer's table
/// editor writes them (after `resolve`: cells hold one version of the text).
pub fn tables_to_tex(doc: &mut Value) {
    fn go(v: &mut Value) {
        match v {
            Value::Array(a) => a.iter_mut().for_each(go),
            Value::Object(_) if t(v) == "Table" => {
                if let Some(tex) = table_tex(v) {
                    *v = json!({"t": "RawBlock", "c": ["latex", tex]});
                }
            }
            Value::Object(o) => {
                if let Some(c) = o.get_mut("c") {
                    go(c);
                }
            }
            _ => {}
        }
    }
    if let Some(b) = doc.get_mut("blocks") {
        go(b);
    }
}

/// Clean the document: headings, equations, captions, references, images.
pub fn clean(doc: &mut Value) -> Found {
    let mut found = Found::default();
    // links inside the document (Word's cross-references, "Fig. 1" pointing
    // at a bookmark) become their text: the typed numbers become \refs below
    fn unlink(v: &mut Value) {
        match v {
            Value::Array(a) => {
                let mut out = Vec::with_capacity(a.len());
                for mut x in std::mem::take(a) {
                    if t(&x) == "Link" && x["c"][2][0].as_str().is_some_and(|u| u.starts_with('#')) {
                        let mut inner = x["c"][1].take();
                        unlink(&mut inner);
                        out.extend(inner.as_array().cloned().unwrap_or_default());
                        continue;
                    }
                    unlink(&mut x);
                    out.push(x);
                }
                *a = out;
            }
            Value::Object(o) => {
                if let Some(c) = o.get_mut("c") {
                    unlink(c);
                }
            }
            _ => {}
        }
    }
    if let Some(b) = doc.get_mut("blocks") {
        unlink(b);
    }
    if let Some(blocks) = doc.get_mut("blocks").and_then(|b| b.as_array_mut()) {
        clean_blocks(blocks, &mut found, true);
    }
    found
}

// ── Tracked changes and comments ────────────────────────

fn span_kind(v: &Value) -> Option<&'static str> {
    if t(v) != "Span" {
        return None;
    }
    let cls = attr_classes(&v["c"][0]);
    ["insertion", "deletion", "paragraph-insertion", "paragraph-deletion", "comment-start", "comment-end"].into_iter().find(|k| cls.contains(k))
}

/// The first insertion or deletion under `v`.
fn first_change(v: &Value) -> Option<Who> {
    match v {
        Value::Array(a) => a.iter().find_map(first_change),
        Value::Object(_) => {
            if matches!(span_kind(v), Some("insertion" | "deletion")) {
                let a = &v["c"][0];
                return Some(Who { author: attr_get(a, "author").unwrap_or("").to_string(), date: attr_get(a, "date").unwrap_or("").to_string() });
            }
            v.get("c").and_then(first_change)
        }
        _ => None,
    }
}

/// Number every paragraph that holds a tracked change (a marker at its end,
/// in both versions) and turn comments into start/end markers.
pub fn mark(doc: &mut Value) -> Marks {
    let mut marks = Marks::default();
    fn go(v: &mut Value, marks: &mut Marks) {
        match v {
            Value::Array(a) => {
                let mut out = Vec::with_capacity(a.len());
                for mut x in std::mem::take(a) {
                    match span_kind(&x) {
                        Some("comment-start") => {
                            let attr = &x["c"][0];
                            let id = attr_get(attr, "comment-id").or_else(|| attr_get(attr, "id")).unwrap_or("").to_string();
                            marks.comments.insert(
                                id.clone(),
                                CommentInfo {
                                    author: attr_get(attr, "author").unwrap_or("").to_string(),
                                    date: attr_get(attr, "date").unwrap_or("").to_string(),
                                    text: stringify(&x["c"][1]),
                                },
                            );
                            out.push(raw_inline(&format!("{C_START}{id}{CLOSE}")));
                        }
                        Some("comment-end") => {
                            let id = attr_get(&x["c"][0], "comment-id").or_else(|| attr_get(&x["c"][0], "id")).unwrap_or("").to_string();
                            out.push(raw_inline(&format!("{C_END}{id}{CLOSE}")));
                        }
                        _ => {
                            if matches!(t(&x), "Para" | "Plain") {
                                if let Some(who) = first_change(&x) {
                                    let k = marks.changes.len();
                                    marks.changes.push(who);
                                    if let Some(c) = x.get_mut("c").and_then(|c| c.as_array_mut()) {
                                        c.push(raw_inline(&format!("{CHANGE}{k}{CLOSE}")));
                                    }
                                }
                            }
                            go(&mut x, marks);
                            out.push(x);
                        }
                    }
                }
                *a = out;
            }
            Value::Object(o) => {
                if let Some(c) = o.get_mut("c") {
                    go(c, marks);
                }
            }
            _ => {}
        }
    }
    if let Some(b) = doc.get_mut("blocks") {
        go(b, &mut marks);
    }
    marks
}

/// Every tracked change accepted or rejected.
pub fn resolve(doc: &mut Value, mode: Mode) {
    fn markers_only(v: &Value, out: &mut Vec<Value>) {
        match v {
            Value::Array(a) => a.iter().for_each(|x| markers_only(x, out)),
            Value::Object(_) if t(v) == "RawInline" => out.push(v.clone()),
            Value::Object(o) => {
                if let Some(c) = o.get("c") {
                    markers_only(c, out);
                }
            }
            _ => {}
        }
    }
    fn go(v: &mut Value, mode: Mode) {
        match v {
            Value::Array(a) => {
                let mut out = Vec::with_capacity(a.len());
                for mut x in std::mem::take(a) {
                    let keep = match (span_kind(&x), mode) {
                        (Some("insertion"), Mode::Accept) | (Some("deletion"), Mode::Reject) => true,
                        (Some("insertion"), Mode::Reject) | (Some("deletion"), Mode::Accept) => false,
                        (Some("paragraph-insertion" | "paragraph-deletion"), _) => continue,
                        _ => {
                            go(&mut x, mode);
                            // a paragraph whose every word went
                            if matches!(t(&x), "Para" | "Plain") && x["c"].as_array().is_some_and(|c| c.iter().all(|i| matches!(t(i), "Space" | "SoftBreak") || is_marker(i))) && x["c"].as_array().is_some_and(|c| !c.iter().any(is_marker)) {
                                continue;
                            }
                            out.push(x);
                            continue;
                        }
                    };
                    let mut inner = x["c"][1].take();
                    if keep {
                        go(&mut inner, mode);
                        out.extend(inner.as_array().cloned().unwrap_or_default());
                    } else {
                        // a comment inside removed text stays where the text was
                        markers_only(&inner, &mut out);
                    }
                }
                *a = out;
            }
            Value::Object(o) => {
                if let Some(c) = o.get_mut("c") {
                    go(c, mode);
                }
            }
            _ => {}
        }
    }
    if let Some(b) = doc.get_mut("blocks") {
        go(b, mode);
    }
}

fn is_marker(v: &Value) -> bool {
    t(v) == "RawInline" && v["c"][1].as_str().is_some_and(|s| s.starts_with([C_START, C_END, CHANGE]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_heading_styles_numbers_and_captions() {
        assert_eq!(heading_style("Heading1"), Some(1));
        assert_eq!(heading_style("heading 2"), Some(2));
        assert_eq!(heading_style("Normal"), None);
        assert_eq!(heading_number("2.1 Data"), Some(("2.1".into(), 3)));
        assert_eq!(heading_number("1. Introduction"), Some(("1".into(), 2)));
        assert_eq!(heading_number("III. Method"), Some(("III".into(), 4)));
        assert_eq!(heading_number("Introduction"), None);
        assert_eq!(caption_number("Figure 1. The rig."), Some(("fig", "1".into(), 9)));
        assert_eq!(caption_number("Fig. 12: Plot"), Some(("fig", "12".into(), 8)));
        assert_eq!(caption_number("Table 3 Widgets"), Some(("tab", "3".into(), 7)));
        assert_eq!(caption_number("Figures show"), None);
        assert_eq!(strip_ref_number("[3] C. Person, Loads."), "C. Person, Loads.");
        assert_eq!(strip_ref_number("12. D. Name"), "D. Name");
        assert!(is_references_heading("References") && is_references_heading("5. References") && is_references_heading("參考文獻"));
    }

    #[test]
    fn accepts_and_rejects_changes_and_keeps_comment_markers() {
        let para = json!({"t": "Para", "c": [
            {"t": "Str", "c": "A"}, {"t": "Space"},
            {"t": "Span", "c": [["", ["insertion"], [["author", "daidai"], ["date", "2026-11-01T10:00:00Z"]]], [{"t": "Str", "c": "new"}, {"t": "Space"}]]},
            {"t": "Span", "c": [["", ["deletion"], [["author", "daidai"], ["date", "2026-11-01T10:05:00Z"]]], [
                {"t": "Span", "c": [["", ["comment-start"], [["comment-id", "7"], ["author", "ellrin"], ["date", "d"]]], [{"t": "Str", "c": "Why?"}]]},
                {"t": "Str", "c": "old"}, {"t": "Space"}]]},
            {"t": "Str", "c": "word."}
        ]});
        let mut doc = json!({"blocks": [para]});
        let marks = mark(&mut doc);
        assert_eq!(marks.changes, [Who { author: "daidai".into(), date: "2026-11-01T10:00:00Z".into() }]);
        assert_eq!(marks.comments["7"].text, "Why?");
        let mut a = doc.clone();
        resolve(&mut a, Mode::Accept);
        let mut r = doc.clone();
        resolve(&mut r, Mode::Reject);
        let text = |d: &Value| d["blocks"][0]["c"].as_array().unwrap().iter().map(|i| match t(i) { "Str" => i["c"].as_str().unwrap().to_string(), "Space" => " ".into(), _ => i["c"][1].as_str().unwrap().to_string() }).collect::<String>();
        assert_eq!(text(&a), format!("A new {C_START}7{CLOSE}word.{CHANGE}0{CLOSE}"));
        assert_eq!(text(&r), format!("A {C_START}7{CLOSE}old word.{CHANGE}0{CLOSE}"));
    }

    #[test]
    fn equation_tables_become_equations() {
        let cell = |blocks: Value| json!([["", [], []], {"t": "AlignDefault"}, 1, 1, blocks]);
        let table = json!({"t": "Table", "c": [["", [], []], [null, []], [], [["", [], []], []],
            [[["", [], []], 0, [], [[["", [], []], [cell(json!([{"t": "Plain", "c": [{"t": "Math", "c": [{"t": "DisplayMath"}, "F = kx"]}]}])), cell(json!([{"t": "Plain", "c": [{"t": "Str", "c": "(2)"}]}]))]]]]],
            [["", [], []], []]]});
        let mut doc = json!({"blocks": [table]});
        let found = clean(&mut doc);
        assert_eq!(found.labels, ["eq:2"]);
        assert_eq!(doc["blocks"][0]["c"][1], "\\begin{equation}\nF = kx\n\\label{eq:2}\n\\end{equation}");
    }
}
