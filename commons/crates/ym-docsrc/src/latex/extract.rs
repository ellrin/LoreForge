//! LaTeX project → [`AssetCatalog`]: metadata, sections, figures (with
//! sub-figures), tables (source + grid), equations (LaTeX, labels, numbers),
//! references.

use super::bib::{format_entry, parse_bibitems, parse_bibtex};
use super::plain::{PlainOpts, is_math_env, to_plain};
use super::scan::*;
use super::source::LatexProject;
use crate::error::Result;
use crate::model::*;
use std::collections::HashMap;
use std::path::Path;

/// Open a source folder (main file found automatically) and extract it.
pub fn extract_dir(root: &Path) -> Result<AssetCatalog> {
    let p = LatexProject::open(root)?;
    extract(&p)
}

fn is_figure_env(n: &str) -> bool {
    matches!(
        n,
        "figure" | "figure*" | "sidewaysfigure" | "wrapfigure" | "SCfigure"
    )
}
fn is_table_env(n: &str) -> bool {
    matches!(
        n,
        "table" | "table*" | "sidewaystable" | "wraptable" | "longtable"
    )
}

/// Remove `\label{…}`, `\nonumber`, `\notag`, `\tag{…}` from equation source.
pub fn strip_labels(s: &str) -> String {
    let mut t = remove_command(s, "label", 1);
    t = remove_command(&t, "tag", 1);
    t = remove_command(&t, "nonumber", 0);
    t = remove_command(&t, "notag", 0);
    t.trim().to_string()
}

fn labels_in(s: &str) -> Vec<String> {
    command_args(s, "label")
        .into_iter()
        .map(|(_, t)| t.trim().to_string())
        .collect()
}

fn letter(i: usize) -> char {
    (b'a' + (i % 26) as u8) as char
}

struct SecMark {
    start: usize,
    title_end: usize,
    level: u8,
    starred: bool,
    title: String,
    appendix: bool,
}

/// Extract everything from an opened project.
pub fn extract(p: &LatexProject) -> Result<AssetCatalog> {
    let (pre, body, body_off) = p.split_document();
    let full = p.text.as_str();
    let mut cat = AssetCatalog {
        schema: SCHEMA,
        source: DocSource::Latex {
            root: p.root.display().to_string(),
            main: p.main.clone(),
        },
        ..Default::default()
    };

    let graphicspath: Vec<String> = first_arg(full, "graphicspath")
        .map(|g| {
            let mut v = vec![];
            let mut i = 0;
            while let Some((t, after)) = group_text(g, i) {
                v.push(t.to_string());
                i = after;
            }
            v
        })
        .unwrap_or_default();

    // ── sections (positions first, numbers below) ──────────────────────
    let mut marks: Vec<SecMark> = vec![];
    let appendix_at = find_command(body, "appendix", 0).map(|x| x.0);
    for (name, level) in [("section", 1u8), ("subsection", 2), ("subsubsection", 3)] {
        let mut i = 0;
        while let Some((start, after, star)) = find_command(body, name, i) {
            let q = skip_optionals(body, after);
            match group_text(body, q) {
                Some((t, end)) => {
                    marks.push(SecMark {
                        start,
                        title_end: end,
                        level,
                        starred: star,
                        title: to_plain(
                            t,
                            &PlainOpts {
                                keep_math: true,
                                labels: None,
                            },
                        ),
                        appendix: appendix_at.is_some_and(|a| start > a),
                    });
                    i = end;
                }
                None => i = after,
            }
        }
    }
    marks.sort_by_key(|m| m.start);
    // where the text ends: bibliography / end of body
    let text_end = [
        find_command(body, "bibliography", 0).map(|x| x.0),
        body.find("\\begin{thebibliography}"),
        find_command(body, "printbibliography", 0).map(|x| x.0),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(body.len());

    // section numbers and label map
    let mut labels: HashMap<String, String> = HashMap::new();
    let mut counters = [0usize; 4];
    let mut sec_numbers: Vec<Option<String>> = vec![];
    let mut appendix_started = false;
    for m in &marks {
        if m.appendix && !appendix_started {
            appendix_started = true;
            counters = [0; 4];
        }
        if m.starred {
            sec_numbers.push(None);
            continue;
        }
        let l = m.level as usize;
        counters[l] += 1;
        for c in counters.iter_mut().skip(l + 1) {
            *c = 0;
        }
        let head = if m.appendix {
            ((b'A' + (counters[1].max(1) - 1) as u8) as char).to_string()
        } else {
            counters[1].to_string()
        };
        let mut parts = vec![head];
        parts.extend(counters.iter().take(l + 1).skip(2).map(|c| c.to_string()));
        sec_numbers.push(Some(parts.join(".")));
    }

    // ── figures, tables, equations ───────────────────────────────────────
    let float_envs = envs(body, &|n| is_figure_env(n) || is_table_env(n));
    let mut fig_n = 0;
    let mut tab_n = 0;
    let mut floats: Vec<(usize, Vec<Asset>)> = vec![];
    for e in &float_envs {
        let whole = e.whole(body);
        let inner = e.body(body);
        let src_ref = p.locate(body_off + e.start);
        if is_figure_env(&e.name) {
            fig_n += 1;
            let mut list = figure_assets(p, inner, whole, fig_n, &graphicspath);
            for a in list.iter_mut() {
                a.source_ref = src_ref.clone();
                if let (Some(l), Some(n)) = (&a.label, &a.number) {
                    labels.insert(l.clone(), n.clone());
                }
            }
            floats.push((e.start, list));
        } else {
            tab_n += 1;
            let mut a = table_asset(inner, whole, tab_n);
            a.source_ref = src_ref;
            if let (Some(l), Some(n)) = (&a.label, &a.number) {
                labels.insert(l.clone(), n.clone());
            }
            floats.push((e.start, vec![a]));
        }
    }

    let mut eqs: Vec<(usize, Asset)> = vec![];
    let mut eq_n = 0;
    let mut unnumbered = 0;
    for e in envs(body, &|n| is_math_env(n) && n != "math") {
        let inner = e.body(body);
        let whole = e.whole(body).to_string();
        let src_ref = p.locate(body_off + e.start);
        let starred = e.name.ends_with('*') || e.name == "displaymath";
        let multi = matches!(
            e.name.trim_end_matches('*'),
            "align" | "gather" | "eqnarray" | "flalign" | "alignat"
        );
        let mut numbered_any = false;
        if !starred {
            let lines: Vec<&str> = if multi {
                split_top(inner, "\\\\")
            } else {
                vec![inner]
            };
            for line in lines {
                let clean = strip_labels(line);
                if clean.trim().is_empty() {
                    continue;
                }
                let no_num = find_command(line, "nonumber", 0).is_some()
                    || find_command(line, "notag", 0).is_some();
                let tag = first_arg(line, "tag").map(|t| t.trim().to_string());
                if no_num && tag.is_none() {
                    continue;
                }
                let number = match tag {
                    Some(t) => t,
                    None => {
                        eq_n += 1;
                        eq_n.to_string()
                    }
                };
                let mut a = Asset::new(format!("eq{number}"), AssetKind::Equation);
                a.label = labels_in(line).into_iter().next();
                if let Some(l) = &a.label {
                    labels.insert(l.clone(), number.clone());
                }
                a.text = Some(if multi {
                    clean
                        .replace('&', " ")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    clean
                });
                a.number = Some(number);
                a.source = Some(whole.clone());
                a.source_ref = src_ref.clone();
                eqs.push((e.start, a));
                numbered_any = true;
            }
        }
        if !numbered_any {
            unnumbered += 1;
            let mut a = Asset::new(format!("eq-u{unnumbered}"), AssetKind::Equation);
            a.text = Some(strip_labels(inner));
            a.source = Some(whole);
            a.source_ref = src_ref;
            eqs.push((e.start, a));
        }
    }
    // \[ … \] and $$ … $$
    for (start, inner_s, inner_e) in display_brackets(body) {
        unnumbered += 1;
        let mut a = Asset::new(format!("eq-u{unnumbered}"), AssetKind::Equation);
        a.text = Some(body[inner_s..inner_e].trim().to_string());
        a.source = Some(body[start..inner_e + 2].to_string());
        a.source_ref = p.locate(body_off + start);
        eqs.push((start, a));
    }

    // ── sections with plain paragraphs ───────────────────────────────────
    let mut sec_ids: Vec<(usize, usize, String)> = vec![]; // (start, end, id)
    for (k, m) in marks.iter().enumerate() {
        if m.start >= text_end {
            break;
        }
        if let (Some(n), Some(rest)) = (&sec_numbers[k], body.get(m.title_end..)) {
            let rest = rest.trim_start();
            if rest.starts_with("\\label")
                && let Some(l) = first_arg(rest, "label")
            {
                labels.insert(l.trim().to_string(), n.clone());
            }
        }
    }
    let opts = PlainOpts {
        keep_math: true,
        labels: Some(&labels),
    };
    // abstract as its own section
    if let Some(e) = next_env(full, 0, &|n| n == "abstract") {
        let t = to_plain(e.body(full), &opts);
        cat.meta.abstract_text = Some(t.clone());
        cat.sections.push(Section {
            id: "abstract".into(),
            level: 1,
            number: None,
            title: "Abstract".into(),
            label: None,
            paragraphs: vec![t],
            source_ref: p.locate(e.start),
        });
    }
    for (k, m) in marks.iter().enumerate() {
        if m.start >= text_end {
            break;
        }
        let end = marks
            .get(k + 1)
            .map(|n| n.start)
            .unwrap_or(text_end)
            .min(text_end);
        let raw = &body[m.title_end..end];
        let label = {
            let r = raw.trim_start();
            if r.starts_with("\\label") {
                first_arg(r, "label").map(|s| s.trim().to_string())
            } else {
                None
            }
        };
        let number = sec_numbers[k].clone();
        let mut id = match &number {
            Some(n) => format!("s{n}"),
            None => format!("s-{}", slug(&m.title)),
        };
        if cat.sections.iter().any(|s| s.id == id) {
            id = format!("{id}-{k}");
        }
        let paragraphs = split_paragraphs(raw)
            .into_iter()
            .map(|p| to_plain(p, &opts))
            .filter(|t| !t.trim().is_empty())
            .collect();
        sec_ids.push((m.start, end, id.clone()));
        cat.sections.push(Section {
            id,
            level: m.level,
            number,
            title: m.title.clone(),
            label,
            paragraphs,
            source_ref: p.locate(body_off + m.start),
        });
    }
    // text before the first section (no \section at all: one section)
    if marks.is_empty() {
        let paragraphs = split_paragraphs(&body[..text_end])
            .into_iter()
            .map(|p| to_plain(p, &opts))
            .filter(|t| !t.trim().is_empty())
            .collect();
        cat.sections.push(Section {
            id: "body".into(),
            level: 1,
            title: String::new(),
            paragraphs,
            source_ref: p.locate(body_off),
            ..Default::default()
        });
        sec_ids.push((0, text_end, "body".into()));
    }
    let section_of = |off: usize| {
        sec_ids
            .iter()
            .find(|(s, e, _)| off >= *s && off < *e)
            .map(|x| x.2.clone())
    };

    // captions to plain now that all labels are known
    let mut all: Vec<(usize, Asset)> = vec![];
    for (start, list) in floats {
        for mut a in list {
            if let Some(c) = &a.caption_latex {
                a.caption = Some(to_plain(c, &opts));
            }
            if let Some(g) = a.grid.as_mut() {
                for row in g.iter_mut() {
                    for cell in row.iter_mut() {
                        *cell = to_plain(cell, &opts);
                    }
                }
                a.text = Some(
                    g.iter()
                        .map(|r| r.join("\t"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            a.section = section_of(start);
            all.push((start, a));
        }
    }
    for (start, mut a) in eqs {
        a.section = section_of(start);
        all.push((start, a));
    }
    all.sort_by_key(|(s, a)| (*s, a.kind != AssetKind::Figure));
    let mut seen = std::collections::HashSet::new();
    for (_, mut a) in all {
        if !seen.insert(a.id.clone()) {
            a.id = format!("{}-dup{}", a.id, seen.len());
        }
        cat.assets.push(a);
    }

    // ── metadata ──────────────────────────────────────────────────────────
    cat.meta.title = first_arg(full, "title")
        .map(|t| {
            let t = remove_command(t, "thanks", 1);
            to_plain(
                &t,
                &PlainOpts {
                    keep_math: true,
                    labels: None,
                },
            )
            .replace('\n', " ")
        })
        .unwrap_or_default();
    cat.meta.authors = authors(full);
    cat.meta.keywords = keywords(full);
    let _ = pre;

    // ── references ────────────────────────────────────────────────────────
    cat.references = references(p, body);
    Ok(cat)
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Paragraphs: split at blank lines outside environments.
fn split_paragraphs(raw: &str) -> Vec<&str> {
    let b = raw.as_bytes();
    let mut out = vec![];
    let mut last = 0;
    let mut i = 0;
    while i < b.len() {
        if raw[i..].starts_with("\\begin{")
            && let Some((name, after)) = group_text(raw, i + 6)
            && let Some((_, end)) = find_env_end(raw, name.trim(), after)
        {
            i = end;
            continue;
        }
        if b[i] == b'\n' {
            let mut j = i + 1;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t' || b[j] == b'\r') {
                j += 1;
            }
            if j < b.len() && b[j] == b'\n' {
                out.push(&raw[last..i]);
                while j < b.len() && (b[j] as char).is_ascii_whitespace() {
                    j += 1;
                }
                last = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out.push(&raw[last..]);
    out
}

/// `\[ … \]` and `$$ … $$` outside environments: (start, inner start, inner end).
fn display_brackets(body: &str) -> Vec<(usize, usize, usize)> {
    let b = body.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'\\' && b[i + 1] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == b'\\'
            && b[i + 1] == b'['
            && let Some(k) = body[i + 2..].find("\\]")
        {
            out.push((i, i + 2, i + 2 + k));
            i = i + 2 + k + 2;
            continue;
        }
        if b[i] == b'$'
            && b[i + 1] == b'$'
            && (i == 0 || b[i - 1] != b'\\')
            && let Some(k) = body[i + 2..].find("$$")
        {
            out.push((i, i + 2, i + 2 + k));
            i = i + 2 + k + 2;
            continue;
        }
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

fn graphics(p: &LatexProject, s: &str, gp: &[String]) -> Vec<String> {
    command_args(s, "includegraphics")
        .into_iter()
        .map(|(_, f)| {
            p.find_graphic(f, gp)
                .unwrap_or_else(|| f.trim().to_string())
        })
        .collect()
}

struct SubCand {
    pos: usize,
    caption: Option<String>,
    label: Option<String>,
    files: Vec<String>,
    source: String,
}

fn figure_assets(
    p: &LatexProject,
    inner: &str,
    whole: &str,
    n: usize,
    gp: &[String],
) -> Vec<Asset> {
    let id = format!("fig{n}");
    let mut cands: Vec<SubCand> = vec![];
    let mut cut: Vec<(usize, usize)> = vec![];
    // subfigure / subtable environments
    for e in envs(inner, &|x| matches!(x, "subfigure" | "subtable")) {
        let b = e.body(inner);
        cands.push(SubCand {
            pos: e.start,
            caption: first_arg(b, "caption")
                .or_else(|| first_arg(b, "subcaption"))
                .map(|c| c.trim().to_string()),
            label: labels_in(b).into_iter().next(),
            files: graphics(p, b, gp),
            source: e.whole(inner).to_string(),
        });
        cut.push((e.start, e.end));
    }
    // \subfloat[caption]{…} and \subcaptionbox{caption}{…}
    for cmd in ["subfloat", "subcaptionbox"] {
        let mut i = 0;
        while let Some((start, after, _)) = find_command(inner, cmd, i) {
            if cut.iter().any(|(s, e)| start >= *s && start < *e) {
                i = after;
                continue;
            }
            let (cap, mut q) = if cmd == "subfloat" {
                match read_optional(inner, after) {
                    Some((a, b, next)) => {
                        (Some(inner[a..b].to_string()), skip_optionals(inner, next))
                    }
                    None => (None, after),
                }
            } else {
                match group_text(inner, skip_optionals(inner, after)) {
                    Some((t, next)) => (Some(t.to_string()), next),
                    None => (None, after),
                }
            };
            q = skip_optionals(inner, q);
            let Some((content, end)) = group_text(inner, q) else {
                i = after;
                continue;
            };
            let cap_labels = labels_in(cap.as_deref().unwrap_or(""));
            cands.push(SubCand {
                pos: start,
                caption: cap
                    .map(|c| remove_command(&c, "label", 1).trim().to_string())
                    .filter(|c| !c.is_empty()),
                label: labels_in(content)
                    .into_iter()
                    .next()
                    .or_else(|| cap_labels.into_iter().next()),
                files: graphics(p, content, gp),
                source: inner[start..end].to_string(),
            });
            cut.push((start, end));
            i = end;
        }
    }
    cands.sort_by_key(|c| c.pos);
    cut.sort();
    let mut rest = String::new();
    let mut last = 0;
    for (s, e) in &cut {
        if *s >= last {
            rest.push_str(&inner[last..*s]);
            rest.push(' ');
            last = *e;
        }
    }
    rest.push_str(&inner[last..]);

    let mut fig = Asset::new(id.clone(), AssetKind::Figure);
    fig.number = Some(n.to_string());
    fig.caption_latex = first_arg(&rest, "caption").map(|c| c.trim().to_string());
    fig.label = labels_in(&rest).into_iter().next();
    fig.files = graphics(p, inner, gp);
    fig.source = Some(whole.to_string());
    let mut out = vec![fig];
    for (k, c) in cands.into_iter().enumerate() {
        let mut a = Asset::new(format!("{id}{}", letter(k)), AssetKind::Subfigure);
        a.number = Some(format!("{n}{}", letter(k)));
        a.caption_latex = c.caption;
        a.label = c.label;
        a.files = c.files;
        a.parent = Some(id.clone());
        a.source = Some(c.source);
        out.push(a);
    }
    out
}

fn table_asset(inner: &str, whole: &str, n: usize) -> Asset {
    let mut a = Asset::new(format!("tab{n}"), AssetKind::Table);
    a.number = Some(n.to_string());
    a.caption_latex = first_arg(inner, "caption").map(|c| c.trim().to_string());
    a.label = labels_in(inner).into_iter().next();
    a.source = Some(whole.to_string());
    a.grid = tabular_grid(inner);
    a
}

const RULES0: &[&str] = &[
    "hline",
    "toprule",
    "midrule",
    "bottomrule",
    "hhline",
    "noalign",
    "endhead",
    "endfirsthead",
    "endfoot",
    "endlastfoot",
];
const RULES1: &[&str] = &["cline", "cmidrule", "rowcolor", "arrayrulecolor"];

/// Cells of the first `tabular`-like environment (LaTeX cell source; turned
/// to plain text by the caller). Multicolumn cells are followed by empty cells.
pub fn tabular_grid(s: &str) -> Option<Vec<Vec<String>>> {
    let e = next_env(s, 0, &|n| {
        matches!(
            n,
            "tabular" | "tabular*" | "tabularx" | "longtable" | "array" | "tabulary"
        )
    })?;
    let body = e.body(s);
    let mut q = 0;
    let groups = match e.name.as_str() {
        "tabular*" | "tabularx" | "tabulary" => 2,
        _ => 1,
    };
    q = skip_optionals(body, q);
    for _ in 0..groups {
        q = read_group(body, q).map(|g| g.2).unwrap_or(q);
    }
    let body = &body[q..];
    let mut rows = vec![];
    for row in split_top(body, "\\\\") {
        let mut r = row.to_string();
        // `\\[2pt]` leaves `[2pt]` at the start of the next row
        if r.trim_start().starts_with('[')
            && let Some((_, _, after)) = read_optional(&r, 0)
        {
            r = r[after..].to_string();
        }
        for c in RULES0 {
            r = remove_command(&r, c, 0);
        }
        for c in RULES1 {
            r = remove_command(&r, c, 1);
        }
        r = remove_command(&r, "specialrule", 3);
        r = remove_command(&r, "addlinespace", 0);
        if r.trim().is_empty() {
            continue;
        }
        let mut cells = vec![];
        for cell in split_top(&r, "&") {
            let c = cell.trim();
            if let Some((start, after, _)) = find_command(c, "multicolumn", 0)
                && start == 0
                && let Some((nstr, a1)) = group_text(c, after)
                && let Some((_, _, a2)) = read_group(c, a1)
                && let Some((text, _)) = group_text(c, a2)
            {
                cells.push(text.trim().to_string());
                let span: usize = nstr.trim().parse().unwrap_or(1);
                for _ in 1..span {
                    cells.push(String::new());
                }
                continue;
            }
            if let Some((start, after, _)) = find_command(c, "multirow", 0)
                && start == 0
            {
                let a1 = read_group(c, after).map(|g| g.2).unwrap_or(after);
                let a1 = skip_optionals(c, a1);
                let a2 = read_group(c, a1).map(|g| g.2).unwrap_or(a1);
                if let Some((text, _)) = group_text(c, a2) {
                    cells.push(text.trim().to_string());
                    continue;
                }
            }
            cells.push(c.to_string());
        }
        rows.push(cells);
    }
    (!rows.is_empty()).then_some(rows)
}

fn authors(full: &str) -> Vec<String> {
    let mut out = vec![];
    for (_, arg) in command_args(full, "author") {
        let mut t = arg.to_string();
        for c in [
            "thanks",
            "IEEEmembership",
            "inst",
            "footnote",
            "textsuperscript",
            "orcidlink",
            "IEEEauthorblockA",
            "affiliation",
            "email",
            "ead",
            "fnref",
            "corref",
        ] {
            t = remove_command(&t, c, 1);
        }
        // remove $^{…}$ marks
        let mut clean = String::new();
        let mut in_math = false;
        for ch in t.chars() {
            if ch == '$' {
                in_math = !in_math;
                continue;
            }
            if !in_math {
                clean.push(ch);
            }
        }
        let blocks: Vec<String> = {
            let mut v = vec![clean.clone()];
            for sep in ["\\and", "\\And", "\\AND"] {
                v = v
                    .iter()
                    .flat_map(|b| b.split(sep).map(str::to_string).collect::<Vec<_>>())
                    .collect();
            }
            v
        };
        for b in blocks {
            let first = b.split("\\\\").next().unwrap_or("");
            let plain = to_plain(
                first,
                &PlainOpts {
                    keep_math: false,
                    labels: None,
                },
            )
            .replace('\n', " ");
            for name in plain.split(',').flat_map(|x| x.split(" and ")) {
                let n = name
                    .trim()
                    .trim_start_matches("and ")
                    .trim()
                    .trim_matches(|c: char| c == '*' || c.is_ascii_digit())
                    .trim();
                if !n.is_empty()
                    && n.len() < 60
                    && n.chars().any(char::is_alphabetic)
                    && !n.contains('@')
                    && n.split_whitespace().count() <= 5
                {
                    out.push(n.to_string());
                }
            }
        }
    }
    out
}

fn keywords(full: &str) -> Vec<String> {
    let raw = next_env(full, 0, &|n| {
        matches!(n, "IEEEkeywords" | "keyword" | "keywords")
    })
    .map(|e| e.body(full).to_string())
    .or_else(|| first_arg(full, "keywords").map(str::to_string));
    let Some(raw) = raw else { return vec![] };
    let raw = raw.replace("\\sep", ",");
    to_plain(
        &raw,
        &PlainOpts {
            keep_math: true,
            labels: None,
        },
    )
    .split([',', ';', '·', '\n'])
    .map(|k| k.trim().trim_end_matches('.').to_string())
    .filter(|k| !k.is_empty())
    .collect()
}

fn references(p: &LatexProject, body: &str) -> Vec<Reference> {
    // cited keys in order of first citation
    let mut cited: Vec<String> = vec![];
    for c in [
        "cite",
        "citep",
        "citet",
        "citealp",
        "citealt",
        "parencite",
        "textcite",
        "autocite",
        "nocite",
        "citeauthor",
        "citeyear",
    ] {
        for (pos, arg) in command_args(body, c) {
            for k in arg.split(',') {
                let k = k.trim().to_string();
                if !k.is_empty() {
                    cited.push(format!("{pos:012}\u{0}{k}"));
                }
            }
        }
    }
    cited.sort();
    let mut order: Vec<String> = vec![];
    for c in cited {
        let k = c.split('\u{0}').nth(1).unwrap_or("").to_string();
        if !order.contains(&k) {
            order.push(k);
        }
    }

    // .bib files
    let mut bib_files: Vec<String> = vec![];
    for name in ["bibliography", "addbibresource"] {
        for (_, arg) in command_args(&p.text, name) {
            for f in arg.split(',') {
                let f = f.trim();
                let f = if f.ends_with(".bib") {
                    f.to_string()
                } else {
                    format!("{f}.bib")
                };
                bib_files.push(f);
            }
        }
    }
    let mut entries = vec![];
    for f in &bib_files {
        if let Ok(src) = std::fs::read_to_string(p.root.join(f)) {
            entries.extend(parse_bibtex(&src));
        }
    }
    // formatted items: thebibliography in the text, else <main>.bbl
    let mut items = parse_bibitems(body);
    if items.is_empty() {
        let bbl = Path::new(&p.main).with_extension("bbl");
        if let Ok(src) = std::fs::read_to_string(p.root.join(bbl)) {
            items = parse_bibitems(&src);
        }
    }
    let by_key: HashMap<&str, &super::bib::BibEntry> =
        entries.iter().map(|e| (e.key.as_str(), e)).collect();
    let mut out = vec![];
    if !items.is_empty() {
        for (key, text) in &items {
            let e = by_key.get(key.as_str());
            out.push(Reference {
                key: key.clone(),
                text: text.clone(),
                fields: e.map(|e| e.fields.clone()).unwrap_or_default(),
                entry_type: e.map(|e| e.entry_type.clone()),
                cited: order.contains(key),
            });
        }
        return out;
    }
    let mut keys: Vec<&str> = order
        .iter()
        .map(String::as_str)
        .filter(|k| by_key.contains_key(k))
        .collect();
    for e in &entries {
        if !keys.contains(&e.key.as_str()) {
            keys.push(&e.key);
        }
    }
    for k in keys {
        let e = by_key[k];
        out.push(Reference {
            key: k.to_string(),
            text: format_entry(e),
            fields: e.fields.clone(),
            entry_type: Some(e.entry_type.clone()),
            cited: order.iter().any(|o| o == k),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"\documentclass{article}
\graphicspath{{figs/}}
\title{A Tiny Paper\thanks{Funded.}}
\author{Ellrin Example$^{1}$, Daidai Sample\thanks{x}, and Third Person}
\begin{document}
\maketitle
\begin{abstract}
We study \emph{things}.
\end{abstract}
\begin{IEEEkeywords}
X-ray, knowledge base; alignment
\end{IEEEkeywords}
\section{Introduction}\label{sec:intro}
First paragraph cites~\cite{a,b}.

Second, see Fig.~\ref{fig:arch} and Table~\ref{tab:res} and Eq.~\eqref{eq:loss}.
\begin{figure*}[t]
\centering
\includegraphics[width=\linewidth]{arch}
\caption{The architecture. Encoder $E$.}
\label{fig:arch}
\end{figure*}
\subsection{Loss}
\begin{equation}
\mathcal{L} = a + b \label{eq:loss}
\end{equation}
\begin{align}
x &= 1 \label{eq:x}\\
y &= 2 \nonumber\\
z &= 3
\end{align}
\[ q = r \]
\begin{figure}
\subfloat[Left part\label{fig:l}]{\includegraphics{l.png}}
\begin{subfigure}{0.4\linewidth}\includegraphics{r}\caption{Right part}\label{fig:r}\end{subfigure}
\caption{Two parts.}\label{fig:two}
\end{figure}
\section{Results}
\begin{table}
\caption{Results.}\label{tab:res}
\begin{tabular}{lcc}
\toprule
Method & \multicolumn{2}{c}{Score} \\
\midrule
Ours & \textbf{0.9} & 0.8 \\
\bottomrule
\end{tabular}
\end{table}
Done \cite{b}.
\appendix
\section{Proof}
Trivial.
\bibliography{refs}
\end{document}
"#;

    fn project() -> (tempfile::TempDir, LatexProject) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::write(r.join("main.tex"), DOC).unwrap();
        std::fs::create_dir_all(r.join("figs")).unwrap();
        std::fs::write(r.join("figs/arch.pdf"), b"%PDF").unwrap();
        std::fs::write(r.join("refs.bib"), "@article{a, author={A. One}, title={First}, journal={J}, year={2020}}\n@misc{b, title={Second}, year=2021}\n@misc{c, title={Unused}}").unwrap();
        let p = LatexProject::open(r).unwrap();
        (dir, p)
    }

    #[test]
    fn extracts_everything() {
        let (_d, p) = project();
        let c = extract(&p).unwrap();
        assert_eq!(c.meta.title, "A Tiny Paper");
        assert_eq!(
            c.meta.authors,
            vec!["Ellrin Example", "Daidai Sample", "Third Person"]
        );
        assert_eq!(
            c.meta.keywords,
            vec!["X-ray", "knowledge base", "alignment"]
        );
        assert_eq!(c.meta.abstract_text.as_deref(), Some("We study things."));

        let ids: Vec<&str> = c.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["abstract", "s1", "s1.1", "s2", "sA"]);
        let intro = &c.sections[1];
        assert_eq!(intro.label.as_deref(), Some("sec:intro"));
        assert_eq!(
            intro.paragraphs,
            vec![
                "First paragraph cites.",
                "Second, see Fig. 1 and Table 1 and Eq. (1)."
            ]
        );

        let fig = c.asset("fig1").unwrap();
        assert_eq!(
            fig.caption.as_deref(),
            Some("The architecture. Encoder $E$.")
        );
        assert_eq!(fig.label.as_deref(), Some("fig:arch"));
        assert_eq!(fig.files, vec!["figs/arch.pdf"]);
        assert_eq!(fig.section.as_deref(), Some("s1"));
        assert_eq!(fig.source_ref.line, Some(17));

        let two = c.asset("fig2").unwrap();
        assert_eq!(two.caption.as_deref(), Some("Two parts."));
        assert_eq!(two.label.as_deref(), Some("fig:two"));
        let subs: Vec<&Asset> = c.of_kind(AssetKind::Subfigure).collect();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].id, "fig2a");
        assert_eq!(subs[0].caption.as_deref(), Some("Left part"));
        assert_eq!(subs[0].label.as_deref(), Some("fig:l"));
        assert_eq!(subs[0].files, vec!["l.png"]);
        assert_eq!(subs[1].caption.as_deref(), Some("Right part"));
        assert_eq!(subs[1].label.as_deref(), Some("fig:r"));

        let eqs: Vec<(&str, Option<&str>, Option<&str>)> = c
            .of_kind(AssetKind::Equation)
            .map(|a| (a.id.as_str(), a.number.as_deref(), a.text.as_deref()))
            .collect();
        assert_eq!(
            eqs,
            vec![
                ("eq1", Some("1"), Some("\\mathcal{L} = a + b")),
                ("eq2", Some("2"), Some("x = 1")),
                ("eq3", Some("3"), Some("z = 3")),
                ("eq-u1", None, Some("q = r")),
            ]
        );
        assert_eq!(c.asset("eq2").unwrap().label.as_deref(), Some("eq:x"));

        let tab = c.asset("tab1").unwrap();
        assert_eq!(tab.caption.as_deref(), Some("Results."));
        assert_eq!(
            tab.grid.as_ref().unwrap(),
            &vec![vec!["Method", "Score", ""], vec!["Ours", "0.9", "0.8"]]
        );
        assert!(tab.source.as_ref().unwrap().starts_with("\\begin{table}"));
        assert_eq!(tab.section.as_deref(), Some("s2"));

        let keys: Vec<(&str, bool)> = c
            .references
            .iter()
            .map(|r| (r.key.as_str(), r.cited))
            .collect();
        assert_eq!(keys, vec![("a", true), ("b", true), ("c", false)]);
        assert_eq!(c.references[0].text, "A. One. First. J, 2020.");
    }
}
