//! LaTeX tables as an editable grid.
//!
//! `parse` reads a `table` / `table*` / `sidewaystable` environment (or a
//! bare `tabular`) into a [`TexTable`]: cells with column and row spans, the
//! rules between rows, caption, label, font size. `render` writes it back as
//! plain LaTeX using booktabs, multirow and standard `tabular` only. Anything
//! the grid cannot represent is either kept verbatim (lines around the
//! tabular) or the parse fails, so the editor falls back to source code and
//! never silently rewrites a table it does not understand.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Cell {
    pub text: String,
    #[serde(default = "one")]
    pub colspan: usize,
    #[serde(default = "one")]
    pub rowspan: usize,
    /// Hidden under a span that starts elsewhere.
    #[serde(default)]
    pub covered: bool,
    /// Column spec from `\multicolumn{n}{spec}{...}`.
    #[serde(default)]
    pub align: Option<String>,
}

fn one() -> usize {
    1
}

/// A horizontal rule between rows. `from` / `to` are 1-based columns for
/// partial rules (`\cline`, `\cmidrule`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub kind: String,
    #[serde(default)]
    pub from: Option<usize>,
    #[serde(default)]
    pub to: Option<usize>,
    /// `\cmidrule(lr)` trimming.
    #[serde(default)]
    pub trim: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TexTable {
    /// "table", "table*", "sidewaystable", or "" for a bare tabular.
    pub env: String,
    #[serde(default)]
    pub placement: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub caption_below: bool,
    #[serde(default)]
    pub centering: bool,
    /// Size command such as "small" or "footnotesize".
    #[serde(default)]
    pub font: Option<String>,
    /// One spec per column: "l", "c", "r", "p{3cm}", "S", ...
    pub cols: Vec<String>,
    /// Vertical bars at the `cols.len() + 1` column boundaries.
    #[serde(default)]
    pub vlines: Vec<bool>,
    /// `tabular*{width}`: the table is stretched to this width.
    #[serde(default)]
    pub width: Option<String>,
    /// `@{\extracolsep\fill}`: spread the extra width between columns.
    #[serde(default)]
    pub fill: bool,
    /// `\renewcommand{\arraystretch}{x}` inside the environment.
    #[serde(default)]
    pub stretch: Option<String>,
    /// `\setlength{\tabcolsep}{x}` inside the environment.
    #[serde(default)]
    pub colsep: Option<String>,
    /// `@{}` at the left / right edge.
    #[serde(default)]
    pub trim_left: bool,
    #[serde(default)]
    pub trim_right: bool,
    pub rows: Vec<Vec<Cell>>,
    /// Rules at the `rows.len() + 1` row boundaries (0 = above the first row).
    pub rules: Vec<Vec<Rule>>,
    /// Extra vertical space after each row (`\\[2pt]`).
    #[serde(default)]
    pub gaps: Vec<Option<String>>,
    /// Lines inside the environment before / after the tabular that the grid
    /// does not model (notes, `\footnotetext`, `\vspace`), kept verbatim.
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    /// Style preset id, for the layout rules.
    #[serde(default)]
    pub style: Option<String>,
}

const ENVS: &[&str] = &["table*", "table", "sidewaystable*", "sidewaystable"];
const SIZES: &[&str] = &["tiny", "scriptsize", "footnotesize", "small", "normalsize", "large"];

/// Is this paragraph a table this module can try to read?
pub fn looks_like_table(text: &str) -> bool {
    let t = text.trim_start();
    ENVS.iter().any(|e| t.starts_with(&format!("\\begin{{{e}}}"))) || t.starts_with("\\begin{tabular}")
}

// ── small scanning helpers ─────────────────────────────

/// `{...}` at the start of `s` (after spaces): (inside, rest).
fn group(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    let mut prev = '\0';
    for (i, c) in s.char_indices() {
        if prev != '\\' {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((&s[1..i], &s[i + 1..]));
                    }
                }
                _ => {}
            }
        }
        prev = if prev == '\\' && c == '\\' { '\0' } else { c };
    }
    None
}

/// `[...]` at the start of `s` (after spaces), if present.
fn optional(s: &str) -> (Option<&str>, &str) {
    let t = s.trim_start();
    if let Some(rest) = t.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return (Some(&rest[..end]), &rest[end + 1..]);
        }
    }
    (None, s)
}

/// Drop `%` comments (not `\%`).
fn strip_comments(s: &str) -> String {
    s.lines()
        .map(|l| {
            let b = l.as_bytes();
            let cut = (0..b.len()).find(|&i| b[i] == b'%' && (i == 0 || b[i - 1] != b'\\')).unwrap_or(b.len());
            &l[..cut]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Split at a separator that is not inside braces and not escaped.
fn split_top(s: &str, sep: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let c = bytes[i];
        if c == b'\\' && i + 1 < s.len() && !s[i..].starts_with(sep) {
            // escaped char or command: copy both bytes of "\x"
            let next = s[i + 1..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
            cur.push_str(&s[i..i + 1 + next]);
            i += 1 + next;
            continue;
        }
        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
        }
        if depth == 0 && s[i..].starts_with(sep) {
            out.push(std::mem::take(&mut cur));
            i += sep.len();
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        cur.push(ch);
        i += ch.len_utf8();
    }
    out.push(cur);
    out
}

// ── parse ──────────────────────────────────────────────

pub fn parse(src: &str) -> Result<TexTable, String> {
    let text = src.trim();
    let mut t = TexTable::default();
    let inner: &str;
    if let Some(env) = ENVS.iter().find(|e| text.starts_with(&format!("\\begin{{{e}}}"))) {
        t.env = env.to_string();
        let open = format!("\\begin{{{env}}}");
        let close = format!("\\end{{{env}}}");
        let body = &text[open.len()..];
        let (pl, body) = optional(body);
        t.placement = pl.map(String::from);
        let end = body.rfind(&close).ok_or("environment is not closed")?;
        if !body[end + close.len()..].trim().is_empty() {
            return Err("text after the table environment".into());
        }
        inner = &body[..end];
    } else if text.starts_with("\\begin{tabular}") {
        inner = text;
    } else {
        return Err("not a table".into());
    }

    let star = inner.contains("\\begin{tabular*}");
    let (open, close) = if star { ("\\begin{tabular*}", "\\end{tabular*}") } else { ("\\begin{tabular}", "\\end{tabular}") };
    let tab_start = inner.find(open).ok_or("no tabular (tabularx and longtable are edited as code)")?;
    if inner[tab_start + 1..].contains("\\begin{tabular") {
        return Err("more than one tabular".into());
    }
    let tab_end = inner.rfind(close).ok_or("tabular is not closed")?;
    let pre = &inner[..tab_start];
    let post = &inner[tab_end + close.len()..];
    let mut tab = &inner[tab_start + open.len()..tab_end];
    if star {
        let (w, rest) = group(tab).ok_or("tabular* has no width")?;
        t.width = Some(w.trim().to_string());
        tab = rest;
    }

    // column spec
    let (_, tab) = optional(tab);
    let (spec, body) = group(tab).ok_or("tabular has no column spec")?;
    parse_spec(spec, &mut t)?;

    read_extras(pre, &mut t, false);
    read_extras(post, &mut t, true);
    parse_body(body, &mut t)?;
    Ok(t)
}

fn parse_spec(spec: &str, t: &mut TexTable) -> Result<(), String> {
    let spec = strip_comments(spec);
    // expand *{n}{x}
    let mut s = spec.trim().to_string();
    while let Some(i) = s.find("*{") {
        let (n, rest) = group(&s[i + 1..]).ok_or("bad *{} in column spec")?;
        let (what, rest) = group(rest).ok_or("bad *{} in column spec")?;
        let n: usize = n.trim().parse().map_err(|_| "bad *{} count")?;
        s = format!("{}{}{}", &s[..i], what.repeat(n), rest);
    }
    let mut cols = Vec::new();
    let mut vlines = vec![false];
    let mut rest = s.as_str();
    let mut first = true;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let c = rest.chars().next().unwrap();
        match c {
            '|' => {
                *vlines.last_mut().unwrap() = true;
                rest = &rest[1..];
            }
            '@' | '!' => {
                let (g, r) = group(&rest[1..]).ok_or("bad @{} in column spec")?;
                let compact: String = g.chars().filter(|c| !c.is_whitespace() && *c != '{' && *c != '}').collect();
                if first && compact == "\\extracolsep\\fill" {
                    t.fill = true;
                    rest = r;
                    continue;
                }
                if !g.trim().is_empty() {
                    return Err("@{...} with content is edited as code".into());
                }
                if first {
                    t.trim_left = true;
                } else {
                    t.trim_right = true;
                }
                rest = r;
            }
            'l' | 'c' | 'r' | 'S' | 'X' => {
                cols.push(c.to_string());
                vlines.push(false);
                rest = &rest[1..];
                first = false;
                t.trim_right = false;
            }
            'p' | 'm' | 'b' => {
                let (g, r) = group(&rest[1..]).ok_or("bad p{} in column spec")?;
                cols.push(format!("{c}{{{g}}}"));
                vlines.push(false);
                rest = r;
                first = false;
                t.trim_right = false;
            }
            '>' | '<' => return Err(">{...} column decorations are edited as code".into()),
            _ => return Err(format!("unknown column type '{c}'")),
        }
    }
    if cols.is_empty() {
        return Err("no columns".into());
    }
    t.cols = cols;
    t.vlines = vlines;
    Ok(())
}

/// Caption, label, centering and font size around the tabular; anything else
/// is kept verbatim in `before` / `after`.
fn read_extras(s: &str, t: &mut TexTable, after: bool) {
    let mut rest = strip_comments(s);
    loop {
        let r = rest.trim_start();
        if r.is_empty() {
            break;
        }
        let mut next: Option<String> = None;
        for (cmd, kind) in [("\\caption", 0), ("\\label", 1), ("\\centering", 2)] {
            if let Some(after_cmd) = r.strip_prefix(cmd) {
                if kind == 2 {
                    t.centering = true;
                    next = Some(after_cmd.to_string());
                } else if let Some((g, rr)) = group(optional(after_cmd).1) {
                    if kind == 0 {
                        t.caption = Some(g.trim().to_string());
                        t.caption_below = after;
                    } else {
                        t.label = Some(g.trim().to_string());
                    }
                    next = Some(rr.to_string());
                }
                break;
            }
        }
        if next.is_none() {
            for (cmd, slot) in [("\\renewcommand{\\arraystretch}", 0), ("\\renewcommand\\arraystretch", 0), ("\\setlength{\\tabcolsep}", 1), ("\\setlength\\tabcolsep", 1)] {
                if let Some(x) = r.strip_prefix(cmd) {
                    if let Some((v, rr)) = group(x) {
                        let v = Some(v.trim().to_string());
                        if slot == 0 { t.stretch = v } else { t.colsep = v }
                        next = Some(rr.to_string());
                        break;
                    }
                }
            }
        }
        if next.is_none() {
            if let Some(size) = SIZES.iter().find(|z| r.strip_prefix(&format!("\\{z}")).is_some_and(|x| !x.starts_with(|c: char| c.is_alphabetic()))) {
                t.font = Some(size.to_string());
                next = Some(r[size.len() + 1..].to_string());
            }
        }
        if next.is_none() {
            let (line, more) = r.split_once('\n').unwrap_or((r, ""));
            let line = line.trim();
            if !line.is_empty() {
                if after { t.after.push(line.to_string()) } else { t.before.push(line.to_string()) }
            }
            next = Some(more.to_string());
        }
        rest = next.unwrap_or_default();
    }
}

/// Rule commands at the start of a row chunk; returns them and the rest.
fn take_rules(s: &str) -> (Vec<Rule>, String) {
    let mut rules = Vec::new();
    let mut rest = s.trim_start().to_string();
    loop {
        let r = rest.trim_start();
        let mut took = None;
        // \botrule is Springer Nature's bottom rule.
        // \colrule is REVTeX's rule inside ruledtabular.
        for kind in ["hline", "toprule", "midrule", "bottomrule", "botrule", "colrule"] {
            if let Some(x) = r.strip_prefix(&format!("\\{kind}")) {
                if !x.starts_with(|c: char| c.is_alphabetic()) {
                    let (_, x) = optional(x); // \toprule[1pt]
                    rules.push(Rule { kind: kind.into(), from: None, to: None, trim: None });
                    took = Some(x.to_string());
                    break;
                }
            }
        }
        if took.is_none() {
            for kind in ["cline", "cmidrule"] {
                if let Some(x) = r.strip_prefix(&format!("\\{kind}")) {
                    let mut x = x.trim_start();
                    let mut trim = None;
                    if let Some(rest) = x.strip_prefix('(') {
                        if let Some(end) = rest.find(')') {
                            trim = Some(rest[..end].to_string());
                            x = &rest[end + 1..];
                        }
                    }
                    if let Some((range, xx)) = group(x) {
                        if let Some((a, b)) = range.split_once('-') {
                            if let (Ok(a), Ok(b)) = (a.trim().parse(), b.trim().parse()) {
                                rules.push(Rule { kind: kind.into(), from: Some(a), to: Some(b), trim });
                                took = Some(xx.to_string());
                            }
                        }
                    }
                    break;
                }
            }
        }
        match took {
            Some(x) => rest = x,
            None => return (rules, r.to_string()),
        }
    }
}

fn parse_body(body: &str, t: &mut TexTable) -> Result<(), String> {
    let body = strip_comments(body).replace("\\tabularnewline", "\\\\");
    let chunks = split_top(&body, "\\\\");
    let ncols = t.cols.len();
    let mut pending: Vec<usize> = vec![0; ncols]; // rows still covered by a multirow above
    let last = chunks.len() - 1;
    for (ci, chunk) in chunks.iter().enumerate() {
        // `\\[4pt]` leaves "[4pt]" at the start of the next chunk.
        let mut chunk = chunk.as_str();
        if ci > 0 {
            let (gap, rest) = optional(chunk);
            if let Some(g) = gap {
                if let Some(slot) = t.gaps.last_mut() {
                    *slot = Some(g.to_string());
                }
                chunk = rest;
            }
        }
        let (rules, rest) = take_rules(chunk);
        t.rules.push(rules);
        if ci == last {
            if !rest.trim().is_empty() {
                return Err("text after the last row".into());
            }
            break;
        }
        let mut row: Vec<Cell> = Vec::new();
        for raw in split_top(&rest, "&") {
            let raw = raw.trim();
            let col = row.len();
            if col < ncols && pending[col] > 0 {
                // The LaTeX cell under a multirow is empty; keep it covered.
                pending[col] -= 1;
                if !raw.is_empty() {
                    return Err("text inside a cell covered by \\multirow".into());
                }
                row.push(Cell { covered: true, colspan: 1, rowspan: 1, ..Default::default() });
                continue;
            }
            let mut cell = Cell { colspan: 1, rowspan: 1, ..Default::default() };
            let mut content = raw.to_string();
            if let Some(x) = content.strip_prefix("\\multicolumn") {
                let (n, x) = group(x).ok_or("bad \\multicolumn")?;
                let (spec, x) = group(x).ok_or("bad \\multicolumn")?;
                let (inside, x) = group(x).ok_or("bad \\multicolumn")?;
                if !x.trim().is_empty() {
                    return Err("text after \\multicolumn".into());
                }
                cell.colspan = n.trim().parse().map_err(|_| "bad \\multicolumn count")?;
                let spec = spec.trim();
                cell.align = Some(spec.to_string());
                content = inside.trim().to_string();
            }
            if let Some(x) = content.strip_prefix("\\multirow") {
                let (_, x) = optional(x);
                let (n, x) = group(x).ok_or("bad \\multirow")?;
                let (_, x) = optional(x);
                let (_, x) = group(x).ok_or("bad \\multirow")?; // width
                let (_, x) = optional(x);
                let (inside, x) = group(x).ok_or("bad \\multirow")?;
                if !x.trim().is_empty() {
                    return Err("text after \\multirow".into());
                }
                let n: i64 = n.trim().parse().map_err(|_| "bad \\multirow count")?;
                if n < 1 {
                    return Err("upward \\multirow is edited as code".into());
                }
                cell.rowspan = n as usize;
                content = inside.trim().to_string();
            }
            cell.text = content;
            let span = cell.colspan;
            let rs = cell.rowspan;
            row.push(cell);
            for k in 0..span {
                if col + k < ncols && rs > 1 {
                    pending[col + k] = rs - 1;
                }
                if k > 0 {
                    row.push(Cell { covered: true, colspan: 1, rowspan: 1, ..Default::default() });
                }
            }
        }
        if row.len() > ncols {
            return Err(format!("row {} has {} cells for {} columns", t.rows.len() + 1, row.len(), ncols));
        }
        while row.len() < ncols {
            let col = row.len();
            let covered = pending[col] > 0;
            if covered {
                pending[col] -= 1;
            }
            row.push(Cell { covered, colspan: 1, rowspan: 1, ..Default::default() });
        }
        t.rows.push(row);
        t.gaps.push(None);
    }
    if t.rows.is_empty() {
        return Err("table has no rows".into());
    }
    // Mark cells covered by a multirow that also spans columns.
    let rows = t.rows.len();
    for r in 0..rows {
        for c in 0..ncols {
            let (cs, rs, covered) = { let x = &t.rows[r][c]; (x.colspan, x.rowspan, x.covered) };
            if covered {
                continue;
            }
            for dr in 0..rs {
                for dc in 0..cs {
                    if (dr, dc) != (0, 0) && r + dr < rows && c + dc < ncols {
                        t.rows[r + dr][c + dc].covered = true;
                    }
                }
            }
        }
    }
    Ok(())
}

// ── render ─────────────────────────────────────────────

fn spec(t: &TexTable) -> String {
    let mut s = String::new();
    if t.fill {
        s += "@{\\extracolsep\\fill}";
    }
    if t.trim_left {
        s += "@{}";
    }
    for (i, c) in t.cols.iter().enumerate() {
        if t.vlines.get(i).copied().unwrap_or(false) {
            s.push('|');
        }
        s += c;
    }
    if t.vlines.get(t.cols.len()).copied().unwrap_or(false) {
        s.push('|');
    }
    if t.trim_right {
        s += "@{}";
    }
    s
}

fn rule_tex(r: &Rule) -> String {
    match (r.kind.as_str(), r.from, r.to) {
        ("cline", Some(a), Some(b)) => format!("\\cline{{{a}-{b}}}"),
        ("cmidrule", Some(a), Some(b)) => match &r.trim {
            Some(tr) => format!("\\cmidrule({tr}){{{a}-{b}}}"),
            None => format!("\\cmidrule{{{a}-{b}}}"),
        },
        (k, _, _) => format!("\\{k}"),
    }
}

pub fn render(t: &TexTable) -> String {
    let mut out = String::new();
    let wrapped = !t.env.is_empty();
    if wrapped {
        out += &format!("\\begin{{{}}}", t.env);
        if let Some(p) = &t.placement {
            out += &format!("[{p}]");
        }
        out.push('\n');
        if t.centering {
            out += "\\centering\n";
        }
        if let Some(f) = &t.font {
            out += &format!("\\{f}\n");
        }
        if let Some(x) = &t.stretch {
            out += &format!("\\renewcommand{{\\arraystretch}}{{{x}}}\n");
        }
        if let Some(x) = &t.colsep {
            out += &format!("\\setlength{{\\tabcolsep}}{{{x}}}\n");
        }
        if !t.caption_below {
            caption_label(t, &mut out);
        }
        for l in &t.before {
            out += l;
            out.push('\n');
        }
    }
    match &t.width {
        Some(w) => out += &format!("\\begin{{tabular*}}{{{w}}}{{{}}}\n", spec(t)),
        None => out += &format!("\\begin{{tabular}}{{{}}}\n", spec(t)),
    }
    let line = |rules: Option<&Vec<Rule>>, out: &mut String| {
        if let Some(rs) = rules.filter(|r| !r.is_empty()) {
            *out += &rs.iter().map(rule_tex).collect::<Vec<_>>().join(" ");
            out.push('\n');
        }
    };
    line(t.rules.first(), &mut out);
    for (r, row) in t.rows.iter().enumerate() {
        let mut cells = Vec::new();
        let mut c = 0;
        while c < row.len() {
            let cell = &row[c];
            if cell.covered {
                // Under a column span: the spanning cell already took this slot.
                let in_colspan = (0..c).rev().find(|&k| !row[k].covered).is_some_and(|k| k + row[k].colspan > c);
                if !in_colspan {
                    cells.push(String::new());
                }
                c += 1;
                continue;
            }
            let mut x = cell.text.clone();
            if cell.rowspan > 1 {
                x = format!("\\multirow{{{}}}{{*}}{{{}}}", cell.rowspan, x);
            }
            if cell.colspan > 1 || cell.align.is_some() {
                let align = cell.align.clone().unwrap_or_else(|| "c".into());
                x = format!("\\multicolumn{{{}}}{{{}}}{{{}}}", cell.colspan, align, x);
            }
            cells.push(x);
            c += 1;
        }
        out += &cells.join(" & ");
        out += " \\\\";
        if let Some(Some(g)) = t.gaps.get(r) {
            out += &format!("[{g}]");
        }
        out.push('\n');
        line(t.rules.get(r + 1), &mut out);
    }
    out += if t.width.is_some() { "\\end{tabular*}\n" } else { "\\end{tabular}\n" };
    if wrapped {
        if t.caption_below {
            caption_label(t, &mut out);
        }
        for l in &t.after {
            out += l;
            out.push('\n');
        }
        out += &format!("\\end{{{}}}", t.env);
    } else {
        out.truncate(out.trim_end().len());
    }
    out
}

fn caption_label(t: &TexTable, out: &mut String) {
    match (&t.caption, &t.label) {
        (Some(c), Some(l)) => *out += &format!("\\caption{{{c}}}\\label{{{l}}}\n"),
        (Some(c), None) => *out += &format!("\\caption{{{c}}}\n"),
        (None, Some(l)) => *out += &format!("\\label{{{l}}}\n"),
        (None, None) => {}
    }
}

// ── styles ─────────────────────────────────────────────

/// A table style: how it looks, which packages it needs, the layout limits
/// it imposes, and a starter grid for new tables. Built-in styles live in
/// `table_styles.json`, compiled into the program.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Style {
    pub id: String,
    pub name: String,
    pub group: String,
    /// Where the look comes from, for the "?" tip.
    #[serde(default)]
    pub from: String,
    pub look: Look,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub limits: Limits,
    /// A tabular (LaTeX) used when inserting a new table in this style.
    pub starter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Look {
    /// "booktabs", "hline", "grid", "ruled" (REVTeX), "minimal", "none"
    pub rules: String,
    /// Bottom rule for booktabs: "bottomrule" or Springer's "botrule".
    #[serde(default)]
    pub bottom: Option<String>,
    #[serde(default)]
    pub caption_below: bool,
    #[serde(default)]
    pub placement: Option<String>,
    #[serde(default = "yes")]
    pub centering: bool,
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub stretch: Option<String>,
    #[serde(default)]
    pub colsep: Option<String>,
    /// `@{}` on both edges.
    #[serde(default)]
    pub trim: bool,
    /// "table", "table*", "sidewaystable"
    #[serde(default = "table_env")]
    pub env: String,
    /// `tabular*` width, with `\extracolsep\fill`.
    #[serde(default)]
    pub width: Option<String>,
    /// "threeparttable": notes go in a tablenotes list under the table.
    #[serde(default)]
    pub notes: Option<String>,
    /// Numeric columns become siunitx `S` columns.
    #[serde(default)]
    pub numeric_s: bool,
}

fn yes() -> bool {
    true
}
fn table_env() -> String {
    "table".into()
}

/// Layout limits, enforced once the table can be measured against the
/// journal's column width.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Limits {
    /// Most columns per font size, e.g. {"normalsize": 6, "small": 8}.
    #[serde(default)]
    pub max_cols: std::collections::BTreeMap<String, usize>,
    /// What to try, in order, when the table is too wide.
    #[serde(default)]
    pub overflow: Vec<String>,
    /// Allowed float placements.
    #[serde(default)]
    pub placement: Vec<String>,
}

pub fn builtin_styles() -> Vec<Style> {
    serde_json::from_str(include_str!("table_styles.json")).expect("table_styles.json")
}

const WRAPPERS: &[(&str, &str, &str)] = &[
    // (style key, opening line, closing line)
    ("ruled", "\\begin{ruledtabular}", "\\end{ruledtabular}"),
    ("threeparttable", "\\begin{threeparttable}", "\\end{threeparttable}"),
];

const FULL_RULES: &[&str] = &["hline", "toprule", "midrule", "bottomrule", "botrule", "colrule"];

fn numeric(s: &str) -> bool {
    let s = s.trim().trim_matches('$');
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || "+-.,eE()".contains(c)) && s.chars().any(|c| c.is_ascii_digit())
}

/// Restyle a table: rules, spacing, caption, environment and wrappers follow
/// the style; cells, spans, caption text and label are kept. Which inner
/// boundaries carry a line (e.g. under the header) is kept too.
pub fn apply_style(t: &mut TexTable, s: &Style) {
    let look = &s.look;
    let n = t.rows.len();
    // Inner boundaries that had a full line, and partial rules, by position.
    let mut inner_full: Vec<bool> = (0..=n).map(|b| t.rules.get(b).is_some_and(|r| r.iter().any(|x| FULL_RULES.contains(&x.kind.as_str())))).collect();
    if (1..n).all(|b| !inner_full[b]) && n > 1 {
        inner_full[1] = true; // a header line by default
    }
    let partial: Vec<Vec<(usize, usize)>> = (0..=n)
        .map(|b| t.rules.get(b).map(|r| r.iter().filter_map(|x| Some((x.from?, x.to?))).collect()).unwrap_or_default())
        .collect();
    let rule = |kind: &str| Rule { kind: kind.into(), from: None, to: None, trim: None };
    let part = |kind: &str, a: usize, b: usize, trim: Option<&str>| Rule { kind: kind.into(), from: Some(a), to: Some(b), trim: trim.map(String::from) };
    let bottom = look.bottom.clone().unwrap_or_else(|| "bottomrule".into());
    t.rules = (0..=n)
        .map(|b| {
            let edge = b == 0 || b == n;
            let mut rs: Vec<Rule> = Vec::new();
            match look.rules.as_str() {
                "booktabs" => {
                    if b == 0 { rs.push(rule("toprule")) } else if b == n { rs.push(rule(&bottom)) } else if inner_full[b] { rs.push(rule("midrule")) }
                    if !edge { rs.extend(partial[b].iter().map(|&(a, z)| part("cmidrule", a, z, Some("lr")))) }
                }
                "hline" => {
                    if edge || inner_full[b] { rs.push(rule("hline")) }
                    if !edge { rs.extend(partial[b].iter().map(|&(a, z)| part("cline", a, z, None))) }
                }
                "grid" => rs.push(rule("hline")),
                "ruled" => {
                    // ruledtabular draws the outer double rules itself.
                    if !edge && inner_full[b] { rs.push(rule("colrule")) }
                    if !edge { rs.extend(partial[b].iter().map(|&(a, z)| part("cline", a, z, None))) }
                }
                "minimal" => {
                    if b == 0 { rs.push(rule("toprule")) } else if b == n { rs.push(rule("bottomrule")) }
                }
                _ => {}
            }
            rs
        })
        .collect();
    t.vlines = vec![look.rules == "grid"; t.cols.len() + 1];
    t.caption_below = look.caption_below;
    t.placement = look.placement.clone();
    t.centering = look.centering;
    t.font = look.font.clone();
    t.stretch = look.stretch.clone();
    t.colsep = look.colsep.clone();
    t.trim_left = look.trim;
    t.trim_right = look.trim;
    t.env = look.env.clone();
    t.width = look.width.clone();
    t.fill = look.width.is_some();
    // numeric columns ↔ siunitx S
    for c in 0..t.cols.len() {
        if look.numeric_s {
            let body: Vec<&Cell> = t.rows.iter().skip(1).map(|r| &r[c]).filter(|x| !x.covered && !x.text.trim().is_empty()).collect();
            if !body.is_empty() && body.iter().all(|x| numeric(&x.text)) && t.cols[c].len() == 1 {
                t.cols[c] = "S".into();
                // headers in S columns must be braced
                if let Some(h) = t.rows.first_mut().map(|r| &mut r[c]) {
                    if !h.covered && !h.text.starts_with('{') && !h.text.is_empty() {
                        h.text = format!("{{{}}}", h.text);
                    }
                }
            }
        } else if t.cols[c] == "S" {
            t.cols[c] = "c".into();
            if let Some(h) = t.rows.first_mut().map(|r| &mut r[c]) {
                if h.text.starts_with('{') && h.text.ends_with('}') {
                    h.text = h.text[1..h.text.len() - 1].to_string();
                }
            }
        }
    }
    // wrappers: remove all known ones, then add the style's
    for (_, open, close) in WRAPPERS {
        t.before.retain(|l| l.trim() != *open);
        t.after.retain(|l| l.trim() != *close);
    }
    let notes_lines = ["\\begin{tablenotes}", "\\end{tablenotes}"];
    let has_notes = t.after.iter().any(|l| l.trim_start().starts_with(notes_lines[0]));
    let want = if look.rules == "ruled" { Some("ruled") } else { look.notes.as_deref() };
    if let Some(key) = want {
        if let Some((_, open, close)) = WRAPPERS.iter().find(|(k, _, _)| *k == key) {
            t.before.push(open.to_string());
            if key == "threeparttable" && !has_notes {
                t.after.push("\\begin{tablenotes}\\small".into());
                t.after.push("\\item Note.".into());
                t.after.push("\\end{tablenotes}".into());
            }
            // the wrapper closes after the tabular and any notes
            let pos = if key == "threeparttable" {
                t.after.iter().position(|l| l.trim_start().starts_with(notes_lines[1])).map(|i| i + 1).unwrap_or(0)
            } else {
                0
            };
            t.after.insert(pos, close.to_string());
        }
    }
    t.style = Some(s.id.clone());
}

/// A new table in a style, from its starter grid.
pub fn new_table(s: &Style) -> Result<TexTable, String> {
    let mut t = parse(&s.starter)?;
    t.caption = Some("Caption".into());
    t.label = Some("tab:new".into());
    apply_style(&mut t, s);
    Ok(t)
}

// ── tables from pasted data ────────────────────────────

/// Rows of cells from pasted data: a spreadsheet copy (tab-separated), a
/// Markdown table, or CSV. Blank lines and Markdown separator rows are dropped;
/// cells are trimmed.
pub fn parse_grid(text: &str) -> Vec<Vec<String>> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return vec![];
    }
    let trim = |v: Vec<String>| -> Vec<String> { v.into_iter().map(|c| c.trim().to_string()).collect() };
    if lines.iter().all(|l| l.trim_start().starts_with('|')) {
        return lines
            .iter()
            .filter(|l| !l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '+')))
            .map(|l| {
                let t = l.trim();
                let t = t.strip_prefix('|').unwrap_or(t);
                let t = t.strip_suffix('|').unwrap_or(t);
                trim(t.split('|').map(String::from).collect())
            })
            .collect();
    }
    if lines.iter().any(|l| l.contains('\t')) {
        return lines.iter().map(|l| trim(l.split('\t').map(String::from).collect())).collect();
    }
    let mut r = csv::ReaderBuilder::new().has_headers(false).flexible(true).from_reader(text.as_bytes());
    r.records().flatten().map(|rec| trim(rec.iter().map(String::from).collect())).filter(|row: &Vec<String>| row.iter().any(|c| !c.is_empty())).collect()
}

/// Escape what breaks LaTeX in a pasted cell; text already escaped, and
/// math in `$...$`, is left alone.
fn tex_cell(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut math = false;
    let mut prev = ' ';
    for c in s.chars() {
        if c == '$' && prev != '\\' {
            math = !math;
        }
        if !math && matches!(c, '&' | '%' | '#' | '_') && prev != '\\' {
            out.push('\\');
        }
        out.push(c);
        prev = c;
    }
    out
}

/// A table in a style from a grid of plain text; the first row is the header.
pub fn from_grid(grid: &[Vec<String>], caption: &str, label: &str, style: &Style) -> TexTable {
    let ncols = grid.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let rows: Vec<Vec<Cell>> = grid
        .iter()
        .map(|r| (0..ncols).map(|i| Cell { text: tex_cell(r.get(i).map(String::as_str).unwrap_or("")), colspan: 1, rowspan: 1, covered: false, align: None }).collect())
        .collect();
    let n = rows.len();
    let mut cols = vec!["c".to_string(); ncols];
    cols[0] = "l".into();
    let mut t = TexTable {
        env: "table".into(),
        caption: Some(caption.trim().to_string()),
        label: Some(label.to_string()),
        cols,
        vlines: vec![false; ncols + 1],
        rows,
        rules: vec![vec![]; n + 1],
        gaps: vec![None; n],
        ..Default::default()
    };
    apply_style(&mut t, style);
    t
}

// ── layout: fit the table to the journal's page ────────

pub const FONT_STEPS: &[&str] = &["normalsize", "small", "footnotesize", "scriptsize"];

/// Widths in points, measured by LaTeX with the paper's own class.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Measured {
    /// Natural width of the tabular at each font size.
    pub widths: BTreeMap<String, f64>,
    pub columnwidth: f64,
    pub textwidth: f64,
    pub textheight: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Placement {
    pub env: String,
    pub font: String,
    pub width: f64,
    /// Space the environment offers: column, text width, or (rotated) text height.
    pub avail: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Advice {
    pub ok: bool,
    pub current: Placement,
    /// The first placement in the style's order that fits, when the current
    /// one does not.
    pub suggest: Option<Placement>,
    pub reason: String,
}

fn family(env: &str) -> &str {
    if env.starts_with("sideways") { "sidewaystable" } else if env == "table*" { "table*" } else { "table" }
}

fn avail(env: &str, m: &Measured) -> f64 {
    match family(env) {
        "sidewaystable" => m.textheight,
        "table*" => m.textwidth,
        _ => m.columnwidth,
    }
}

/// Is `font` allowed for this many columns? The limits are for one
/// column; a wider environment allows proportionally more.
fn cols_ok(limits: &Limits, font: &str, cols: usize, widen: f64) -> bool {
    limits.max_cols.get(font).is_none_or(|&max| cols as f64 <= (max as f64 * widen.max(1.0)).floor())
}

/// Packages the table's own markup needs.
pub fn needed_packages(t: &TexTable) -> Vec<String> {
    let mut out: Vec<&str> = Vec::new();
    let kinds: Vec<&str> = t.rules.iter().flatten().map(|r| r.kind.as_str()).collect();
    if kinds.iter().any(|k| ["toprule", "midrule", "bottomrule", "cmidrule"].contains(k)) {
        out.push("booktabs");
    }
    if t.rows.iter().flatten().any(|c| c.rowspan > 1) {
        out.push("multirow");
    }
    if t.cols.iter().any(|c| c == "S") {
        out.push("siunitx");
    }
    if t.env.starts_with("sideways") {
        out.push("rotating");
    }
    if t.before.iter().any(|l| l.contains("threeparttable")) {
        out.push("threeparttable");
    }
    if t.rows.iter().flatten().any(|c| c.text.contains("\\checkmark")) {
        out.push("amssymb");
    }
    out.into_iter().map(String::from).collect()
}

/// Check the table against the page and the style's limits, and find the
/// first placement that fits, trying the style's overflow steps in order:
/// smaller fonts in the same environment, then a wider environment
/// (`table*`, `sidewaystable`) from the normal size down. A style whose base
/// environment is already wide is never moved back to one column.
pub fn advise(t: &TexTable, limits: &Limits, base_env: &str, m: &Measured) -> Advice {
    let cols = t.cols.len();
    let cur_font = t.font.clone().filter(|f| FONT_STEPS.contains(&f.as_str())).unwrap_or_else(|| "normalsize".into());
    let cur_env = if t.env.is_empty() { "table".to_string() } else { t.env.clone() };
    let width_at = |f: &str| m.widths.get(f).copied().unwrap_or(f64::INFINITY);
    let place = |env: &str, font: &str| Placement { env: env.into(), font: font.into(), width: width_at(font), avail: avail(env, m) };
    let widen = |p: &Placement| if m.columnwidth > 0.0 { p.avail / m.columnwidth } else { 1.0 };
    let fits = |p: &Placement| p.width <= p.avail + 0.5 && cols_ok(limits, &p.font, cols, widen(p));
    let current = place(&cur_env, &cur_font);
    if fits(&current) {
        return Advice { ok: true, reason: String::new(), current, suggest: None };
    }

    // Candidate order from the style: fonts listed in `overflow` before the
    // first environment step stay in the base environment.
    let mut envs: Vec<String> = vec![family(base_env).to_string()];
    let mut fonts: Vec<String> = vec!["normalsize".into()];
    for step in &limits.overflow {
        if FONT_STEPS.contains(&step.as_str()) {
            if !fonts.contains(step) { fonts.push(step.clone()); }
        } else if !envs.contains(step) && family(step) != "table" {
            envs.push(family(step).to_string());
        }
    }
    let rank = |e: &str| ["table", "table*", "sidewaystable"].iter().position(|x| *x == family(e)).unwrap_or(0);
    let floor = rank(base_env);
    let mut suggest = None;
    'outer: for env in envs.iter().filter(|e| rank(e) >= floor) {
        for font in &fonts {
            let p = place(env, font);
            if fits(&p) {
                suggest = Some(p);
                break 'outer;
            }
        }
    }
    let over = current.width - current.avail;
    let reason = if !cols_ok(limits, &cur_font, cols, widen(&current)) {
        "欄數過多".to_string()
    } else if over.is_finite() {
        format!("超出 {:.0}pt", over)
    } else {
        "無法量測".into()
    };
    Advice { ok: false, current, suggest, reason }
}

#[cfg(test)]
mod grid_tests {
    use super::*;

    #[test]
    fn reads_spreadsheet_markdown_and_csv() {
        let tsv = "Model\tAUC\tF1\nResNet50\t0.91\t0.85\n\nViT\t0.93\t0.88\n";
        assert_eq!(parse_grid(tsv), vec![vec!["Model", "AUC", "F1"], vec!["ResNet50", "0.91", "0.85"], vec!["ViT", "0.93", "0.88"]]);
        let md = "| Model | AUC |\n|---|:---:|\n| A | 0.9 |\n";
        assert_eq!(parse_grid(md), vec![vec!["Model", "AUC"], vec!["A", "0.9"]]);
        let csv = "Model,\"AUC, 95% CI\"\nA,\"0.91 (0.88-0.93)\"\n";
        assert_eq!(parse_grid(csv), vec![vec!["Model", "AUC, 95% CI"], vec!["A", "0.91 (0.88-0.93)"]]);
        assert!(parse_grid(" \n").is_empty());
    }

    #[test]
    fn builds_a_styled_table() {
        let style = builtin_styles().into_iter().find(|s| s.id == "booktabs").unwrap();
        let grid = parse_grid("Model\tAUC (95% CI)\tn_test\nA & B\t$0.91$\t120\nC\t0.8");
        let t = from_grid(&grid, "Results on the test set", "tab:results", &style);
        assert_eq!(t.cols, ["l", "c", "c"]);
        let tex = render(&t);
        assert!(tex.contains("\\caption{Results on the test set}"), "{tex}");
        assert!(tex.contains("\\label{tab:results}"));
        assert!(tex.contains("AUC (95\\% CI)") && tex.contains("n\\_test") && tex.contains("A \\& B") && tex.contains("$0.91$"));
        assert!(tex.contains("\\toprule") && tex.contains("\\midrule") && tex.contains("\\bottomrule"));
        // the short last row is padded
        assert_eq!(t.rows[2].len(), 3);
        assert!(parse(&tex).is_ok(), "the editor can read it back");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPRINGER: &str = r"\begin{table}[h]
\caption{Caption text}\label{tab1}%
\begin{tabular}{@{}llll@{}}
\toprule
Column 1 & Column 2  & Column 3 & Column 4\\
\midrule
row 1    & data 1   & data 2  & data 3  \\
row 2    & data 4   & data 5\footnotemark[1]  & data 6  \\
\botrule
\end{tabular}
\footnotetext{Source: This is an example of table footnote.}
\end{table}";

    #[test]
    fn reads_a_booktabs_table() {
        let t = parse(SPRINGER).unwrap();
        assert_eq!(t.env, "table");
        assert_eq!(t.placement.as_deref(), Some("h"));
        assert_eq!(t.caption.as_deref(), Some("Caption text"));
        assert_eq!(t.label.as_deref(), Some("tab1"));
        assert_eq!(t.cols, vec!["l", "l", "l", "l"]);
        assert!(t.trim_left && t.trim_right);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.rows[2][2].text, "data 5\\footnotemark[1]");
        assert_eq!(t.rules[0][0].kind, "toprule");
        assert_eq!(t.rules[1][0].kind, "midrule");
        assert_eq!(t.rules[3][0].kind, "botrule");
        assert!(render(&t).contains("\\botrule\n\\end{tabular}"));
        assert_eq!(t.after, vec!["\\footnotetext{Source: This is an example of table footnote.}"]);
    }

    #[test]
    fn spans_round_trip() {
        let src = r"\begin{table*}[t]
\centering
\small
\caption{Results}\label{tab:r}
\begin{tabular}{|l|cc|c|}
\hline
\multirow{2}{*}{Method} & \multicolumn{2}{c|}{Accuracy} & Time \\
\cline{2-3}
 & Top-1 & Top-5 & (s) \\[2pt]
\hline
A & 70.1 & 89.2 & 1.2 \\
\hline
\end{tabular}
\end{table*}";
        let t = parse(src).unwrap();
        assert_eq!(t.font.as_deref(), Some("small"));
        assert_eq!(t.rows[0][0].rowspan, 2);
        assert_eq!(t.rows[0][1].colspan, 2);
        assert!(t.rows[0][2].covered && t.rows[1][0].covered);
        assert_eq!(t.rules[1], vec![Rule { kind: "cline".into(), from: Some(2), to: Some(3), trim: None }]);
        assert_eq!(t.gaps[1].as_deref(), Some("2pt"));
        assert_eq!(t.vlines, vec![true, true, false, true, true]);
        // render → parse gives the same grid
        let again = parse(&render(&t)).unwrap();
        assert_eq!(again, t);
        assert!(render(&t).contains("\\multirow{2}{*}{Method} & \\multicolumn{2}{c|}{Accuracy} & Time \\\\"));
    }

    #[test]
    fn stretched_tabular_star() {
        let src = r"\begin{table}[h]
\caption{Example of a lengthy table}\label{tab2}
\begin{tabular*}{\textwidth}{@{\extracolsep\fill}lcc}
\toprule
 & \multicolumn{2}{@{}c@{}}{Element 1} \\\cmidrule{2-3}
Project & Energy & $\sigma_{calc}$ \\
\botrule
\end{tabular*}
\end{table}";
        let t = parse(src).unwrap();
        assert_eq!(t.width.as_deref(), Some("\\textwidth"));
        assert!(t.fill);
        assert_eq!(t.rows[0][1].align.as_deref(), Some("@{}c@{}"));
        assert_eq!(t.rules[1][0].kind, "cmidrule");
        assert_eq!(parse(&render(&t)).unwrap(), t);
        assert!(render(&t).contains("\\begin{tabular*}{\\textwidth}{@{\\extracolsep\\fill}lcc}"));
    }

    #[test]
    fn every_builtin_style_makes_a_table_that_round_trips() {
        let styles = builtin_styles();
        assert_eq!(styles.len(), 22);
        let mut ids = std::collections::HashSet::new();
        for s in &styles {
            assert!(ids.insert(s.id.clone()), "duplicate style id {}", s.id);
            let t = new_table(s).unwrap_or_else(|e| panic!("{}: starter does not parse: {e}", s.id));
            let tex = render(&t);
            let again = parse(&tex).unwrap_or_else(|e| panic!("{}: rendered table does not parse: {e}\n{tex}", s.id));
            assert_eq!(again.rows, t.rows, "{}", s.id);
            assert_eq!(again.rules, t.rules, "{}", s.id);
            // restyling with the same style changes nothing
            let mut twice = again.clone();
            apply_style(&mut twice, s);
            assert_eq!(render(&twice), tex, "{} is not idempotent", s.id);
        }
    }

    #[test]
    fn switching_style_keeps_content_and_header_line() {
        let styles = builtin_styles();
        let by = |id: &str| styles.iter().find(|s| s.id == id).unwrap().clone();
        let mut t = new_table(&by("multiheader")).unwrap();
        let cells: Vec<Vec<String>> = t.rows.iter().map(|r| r.iter().map(|c| c.text.clone()).collect()).collect();
        apply_style(&mut t, &by("ieee"));
        assert!(t.vlines.iter().all(|&v| v), "grid has vertical lines");
        assert!(t.rules.iter().all(|r| r.iter().any(|x| x.kind == "hline")));
        apply_style(&mut t, &by("springer-nature"));
        assert_eq!(t.rules[0][0].kind, "toprule");
        assert_eq!(t.rules.last().unwrap()[0].kind, "botrule");
        assert!(t.vlines.iter().all(|&v| !v));
        apply_style(&mut t, &by("aps"));
        assert_eq!(t.before.iter().filter(|l| l.contains("ruledtabular")).count(), 1);
        apply_style(&mut t, &by("booktabs"));
        assert!(!t.before.iter().any(|l| l.contains("ruledtabular")) && !t.after.iter().any(|l| l.contains("ruledtabular")));
        let after: Vec<Vec<String>> = t.rows.iter().map(|r| r.iter().map(|c| c.text.clone()).collect()).collect();
        assert_eq!(after, cells, "content survives restyling");
    }

    #[test]
    fn layout_advice_follows_the_style() {
        let styles = builtin_styles();
        let by = |id: &str| styles.iter().find(|s| s.id == id).unwrap().clone();
        let m = |n: f64, sm: f64, fs: f64, ss: f64| Measured {
            widths: [("normalsize", n), ("small", sm), ("footnotesize", fs), ("scriptsize", ss)].into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            columnwidth: 250.0, textwidth: 520.0, textheight: 680.0,
        };
        let st = by("booktabs");
        let t = new_table(&st).unwrap(); // 3 columns
        // fits as is
        assert!(advise(&t, &st.limits, &st.look.env, &m(200.0, 180.0, 160.0, 140.0)).ok);
        // too wide: shrink to small first
        let a = advise(&t, &st.limits, &st.look.env, &m(270.0, 245.0, 220.0, 200.0));
        assert!(!a.ok);
        assert_eq!(a.suggest.as_ref().map(|p| (p.env.as_str(), p.font.as_str())), Some(("table", "small")));
        // even footnotesize too wide: go to table* at normal size
        let a = advise(&t, &st.limits, &st.look.env, &m(400.0, 360.0, 330.0, 300.0));
        assert_eq!(a.suggest.as_ref().map(|p| (p.env.as_str(), p.font.as_str())), Some(("table*", "normalsize")));
        // wider than the page: landscape
        let a = advise(&t, &st.limits, &st.look.env, &m(600.0, 560.0, 530.0, 500.0));
        assert_eq!(a.suggest.as_ref().map(|p| p.env.as_str()), Some("sidewaystable"));
        // IEEE style never shrinks the font (overflow goes straight to table*)
        let ie = by("ieee");
        let a = advise(&new_table(&ie).unwrap(), &ie.limits, &ie.look.env, &m(270.0, 245.0, 220.0, 200.0));
        assert_eq!(a.suggest.as_ref().map(|p| (p.env.as_str(), p.font.as_str())), Some(("table*", "normalsize")));
        // a wide style is never moved back to one column
        let w = by("wide");
        let mut wt = new_table(&w).unwrap();
        wt.font = None;
        let a = advise(&wt, &w.limits, &w.look.env, &m(600.0, 560.0, 515.0, 500.0));
        assert_eq!(a.suggest.as_ref().map(|p| (p.env.as_str(), p.font.as_str())), Some(("table*", "footnotesize")));
        // a wide environment allows more columns at the same size
        let mut nine = t.clone();
        nine.cols = vec!["c".into(); 9];
        let a = advise(&nine, &st.limits, &st.look.env, &m(300.0, 280.0, 260.0, 240.0));
        assert_eq!(a.suggest.as_ref().map(|p| (p.env.as_str(), p.font.as_str())), Some(("table*", "normalsize")));
        // too many columns for the font size, even if the width fits
        let mut many = t.clone();
        many.cols = vec!["c".into(); 9];
        let a = advise(&many, &st.limits, &st.look.env, &m(240.0, 230.0, 220.0, 210.0));
        assert!(!a.ok && a.reason.contains("欄"));
        assert_eq!(a.suggest.as_ref().map(|p| p.font.as_str()), Some("footnotesize"));
    }

    #[test]
    fn unknown_constructs_fall_back_to_code() {
        assert!(parse(r"\begin{table}\begin{tabularx}{\linewidth}{lX}a&b\\\end{tabularx}\end{table}").is_err());
        assert!(parse(r"\begin{table}\begin{tabular}{>{\bfseries}l}a\\\end{tabular}\end{table}").is_err());
        assert!(parse(r"\begin{tabular}{ll}a & b & c\\\end{tabular}").is_err(), "too many cells");
    }

    #[test]
    fn escaped_ampersand_and_braces_stay_in_cells() {
        let t = parse(r"\begin{tabular}{ll}R\&D & {a & b}\\ x & $1\\2$\\\end{tabular}").unwrap();
        assert_eq!(t.rows[0][0].text, "R\\&D");
        assert_eq!(t.rows[0][1].text, "{a & b}");
    }
}
