//! The people a user writes with, kept in the profile (`authors.csv`,
//! `affiliations.csv`), and the author block of a paper's main.tex written
//! the way each document class wants it.
//!
//! Each class family gets its own writer, so the result is predictable and
//! needs no model. A class the app does not know gets the plain
//! `\author{Name\\ Affiliation \and ...}` block, which every class accepts.

use serde::{Deserialize, Serialize};
use std::ops::Range;

/// One person in `authors.csv`. A family name and at least one affiliation
/// are required; everything else may stay empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Author {
    pub id: String,
    #[serde(default)]
    pub given: String,
    pub family: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub orcid: String,
    /// Affiliation ids, `;`-separated in the CSV (a list in JSON), in the order printed.
    #[serde(default, deserialize_with = "ids")]
    pub affiliations: Vec<String>,
    /// Free text: me, advisor, clinician, ...
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub note: String,
}

/// `"F001;F002"` (CSV) or `["F001","F002"]` (JSON).
fn ids<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("ids separated by ; or a list of ids")
        }
        fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Self::Value, E> {
            Ok(s.split(';').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        }
        fn visit_u64<E: serde::de::Error>(self, n: u64) -> Result<Self::Value, E> {
            Ok(vec![n.to_string()])
        }
        fn visit_i64<E: serde::de::Error>(self, n: i64) -> Result<Self::Value, E> {
            Ok(vec![n.to_string()])
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(vec![])
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(vec![])
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                if !s.trim().is_empty() {
                    out.push(s.trim().to_string());
                }
            }
            Ok(out)
        }
    }
    d.deserialize_any(V)
}

impl Author {
    pub fn name(&self) -> String {
        [self.given.trim(), self.family.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
    }
}

/// One institution in `affiliations.csv`; the organization is required.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Affiliation {
    pub id: String,
    #[serde(default)]
    pub department: String,
    pub organization: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub country: String,
}

/// An author as listed on one paper (kept in the paper's `paper.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PaperAuthor {
    pub id: String,
    #[serde(default)]
    pub corresponding: bool,
    /// Contributed equally (with the others marked so).
    #[serde(default)]
    pub equal: bool,
}

/// Drop `\cmd{…}` (with its braced argument) for each of `cmds`.
fn drop_commands(s: &str, cmds: &[&str]) -> String {
    let mut out = s.to_string();
    for c in cmds {
        while let Some(i) = out.find(c) {
            let after = &out[i + c.len()..];
            // only whole command names
            if after.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
                break;
            }
            let opt = after.trim_start_matches('*');
            let skip = after.len() - opt.len();
            let opt_len = if opt.starts_with('[') { opt.find(']').map_or(0, |e| e + 1) } else { 0 };
            let rest = &opt[opt_len..];
            let end = if rest.starts_with('{') {
                let mut depth = 0;
                let mut k = 0;
                for (j, ch) in rest.char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                k = j + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if k == 0 { rest.len() } else { k }
            } else {
                0
            };
            out.replace_range(i..i + c.len() + skip + opt_len + end, " ");
        }
    }
    out
}

/// The people named in a paper's title block, in order: `\author{A~One,
/// B~Two, and C~Three\thanks{…}}`, IEEE `\IEEEauthorblockN{…}`, Springer
/// `\author{\fnm{A} \sur{One}}`. Footnotes, memberships and marks are dropped.
pub fn names_in_block(main: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in crate::front::fields(main).into_iter().filter(|f| f.field == "author") {
        // comments out first (the block often ends a line with %)
        let v: String = f.value.lines().map(|l| match l.find('%').filter(|&i| i == 0 || !l[..i].ends_with('\\')) { Some(i) => &l[..i], None => l }).collect::<Vec<_>>().join(" ");
        let v = drop_commands(&v, &["\\thanks", "\\IEEEmembership", "\\inst", "\\textsuperscript", "\\footnote", "\\email", "\\orcidlink", "\\orcid", "\\affil", "\\affiliation"]);
        let v = v.replace("\\fnm", " ").replace("\\sur", " ");
        for part in v.split("\\and").flat_map(|p| p.split("\\\\")).flat_map(|p| p.split(',')).flat_map(|p| p.split(" and ")) {
            let mut name = String::new();
            let mut chars = part.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        // a command: drop its name
                        while chars.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                            chars.next();
                        }
                        name.push(' ');
                    }
                    '~' => name.push(' '),
                    '$' | '{' | '}' | '*' | '^' | '_' => name.push(' '),
                    c if c.is_ascii_digit() => name.push(' '),
                    c => name.push(c),
                }
            }
            let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
            let name = name.trim_start_matches("and ").trim();
            if name.split_whitespace().count() >= 2 && name.chars().count() <= 60 && name.chars().any(|c| c.is_alphabetic()) && !out.iter().any(|x| x == name) {
                out.push(name.to_string());
            }
        }
    }
    out
}

fn name_key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Profile authors whose names appear in the paper's author block, in the
/// block's order (given + family, or family + given). Nobody is guessed:
/// a name that matches no one is left out.
pub fn match_block(main: &str, authors: &[Author]) -> Vec<PaperAuthor> {
    let mut out: Vec<PaperAuthor> = Vec::new();
    for n in names_in_block(main) {
        let k = name_key(&n.replace('-', ""));
        let hit = authors.iter().find(|a| {
            let g = a.given.replace('-', "");
            let f = a.family.replace('-', "");
            name_key(&format!("{g} {f}")) == k || name_key(&format!("{f} {g}")) == k
        });
        if let Some(a) = hit.filter(|a| !out.iter().any(|x| x.id == a.id)) {
            out.push(PaperAuthor { id: a.id.clone(), corresponding: false, equal: false });
        }
    }
    out
}

/// An author resolved for printing: affiliations as 1-based numbers into
/// the paper's own affiliation list.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub given: String,
    pub family: String,
    pub email: String,
    pub orcid: String,
    pub affils: Vec<usize>,
    pub corresponding: bool,
    pub equal: bool,
}

impl Person {
    fn name(&self) -> String {
        [self.given.trim(), self.family.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
    }
    /// `J. Smith`, for running heads.
    fn short(&self) -> String {
        match self.given.trim().chars().next() {
            Some(c) => format!("{c}. {}", self.family.trim()),
            None => self.family.trim().to_string(),
        }
    }
}

/// The people chosen for a paper, in order, and the affiliations they use,
/// numbered by first appearance.
pub fn resolve(chosen: &[PaperAuthor], authors: &[Author], affils: &[Affiliation]) -> Result<(Vec<Person>, Vec<Affiliation>), String> {
    let mut used: Vec<Affiliation> = Vec::new();
    let mut people = Vec::new();
    for c in chosen {
        let a = authors.iter().find(|a| a.id == c.id).ok_or_else(|| format!("作者資料庫裡沒有 {}", c.id))?;
        let mut nums = Vec::new();
        for id in &a.affiliations {
            let f = affils.iter().find(|f| &f.id == id).ok_or_else(|| format!("{} 的機構 {id} 不在機構資料庫裡", a.name()))?;
            let n = match used.iter().position(|u| u.id == f.id) {
                Some(i) => i + 1,
                None => {
                    used.push(f.clone());
                    used.len()
                }
            };
            nums.push(n);
        }
        if nums.is_empty() {
            return Err(format!("{} 沒有填機構", a.name()));
        }
        people.push(Person {
            given: a.given.clone(),
            family: a.family.clone(),
            email: a.email.trim().to_string(),
            orcid: a.orcid.trim().to_string(),
            affils: nums,
            corresponding: c.corresponding,
            equal: c.equal,
        });
    }
    Ok((people, used))
}

/// How a class writes its author block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Springer Nature `sn-jnl`: `\author[1]{\fnm{}\sur{}}`, `\affil[1]{}`.
    Springer,
    /// Elsevier `elsarticle`: `\author[1]{}`, `\affiliation[1]{organization=...}`.
    Elsevier,
    /// Elsevier `cas-dc` / `cas-sc`: like elsarticle, plus `\cormark`, `[orcid=]`.
    Cas,
    /// IEEE journals: one `\author{A, B, and C \thanks{...}}`.
    IeeeJournal,
    /// IEEE conferences: `\IEEEauthorblockN` / `\IEEEauthorblockA` per author.
    IeeeConference,
    /// Springer LNCS: `\author{A\inst{1}}`, `\institute{}`.
    Lncs,
    /// Plain `\author{Name\\ Affiliation ... \and ...}` (CVPR, NeurIPS, article);
    /// `big_and` uses NeurIPS/ICLR's `\And`.
    Blocks { big_and: bool },
}

/// The style for a main.tex, from its document class.
pub fn detect(main: &str) -> Style {
    let m = crate::front::masked(main);
    let (class, opts) = document_class(&m).unwrap_or_default();
    match class.as_str() {
        "sn-jnl" => Style::Springer,
        "elsarticle" => Style::Elsevier,
        c if c.starts_with("cas-") => Style::Cas,
        "IEEEtran" | "ieeecolor" | "IEEEtranTCOM" | "ieeeconf" => {
            if opts.split(',').any(|o| o.trim() == "conference") || class == "ieeeconf" || m.contains("\\IEEEauthorblockN") {
                Style::IeeeConference
            } else {
                Style::IeeeJournal
            }
        }
        "llncs" => Style::Lncs,
        _ => Style::Blocks { big_and: m.contains("\\And") },
    }
}

fn document_class(m: &str) -> Option<(String, String)> {
    let rest = m[m.find("\\documentclass")? + "\\documentclass".len()..].trim_start();
    let (opts, rest) = match rest.strip_prefix('[') {
        Some(r) => {
            let e = r.find(']')?;
            (r[..e].to_string(), r[e + 1..].trim_start())
        }
        None => (String::new(), rest),
    };
    let r = rest.strip_prefix('{')?;
    Some((r[..r.find('}')?].trim().to_string(), opts))
}

/// Commands that make up the author block, per style.
fn block_commands(style: Style) -> &'static [&'static str] {
    match style {
        Style::Springer => &["author", "affil", "email", "equalcont"],
        Style::Elsevier => &["author", "affiliation", "address", "ead", "cortext", "fntext"],
        Style::Cas => &["author", "affiliation", "cormark", "fnmark", "ead", "credit", "cortext", "fntext"],
        Style::Lncs => &["author", "authorrunning", "institute"],
        Style::IeeeJournal | Style::IeeeConference | Style::Blocks { .. } => &["author"],
    }
}

/// Where the title block ends: the abstract, \maketitle or the first section.
fn front_limit(m: &str) -> usize {
    let body = m.find("\\begin{document}").unwrap_or(0);
    ["\\begin{abstract}", "\\abstract{", "\\abstract[", "\\maketitle", "\\section", "\\input{sections/", "\\end{frontmatter}"]
        .iter()
        .filter_map(|k| m[body..].find(k).map(|i| body + i))
        .min()
        .unwrap_or(m.len())
}

/// Full range of every `\cmd*[..]{..}[..]` in `m` before `limit`.
fn full_ranges(m: &str, cmd: &str, limit: usize) -> Vec<Range<usize>> {
    let b = m.as_bytes();
    let pat = format!("\\{cmd}");
    let mut out = Vec::new();
    let mut at = 0;
    while at < limit {
        let Some(i) = m[at..limit].find(&pat) else { break };
        let start = at + i;
        let mut j = start + pat.len();
        at = j;
        if j < b.len() && b[j].is_ascii_alphabetic() {
            continue;
        }
        if j < b.len() && b[j] == b'*' {
            j += 1;
        }
        // any run of [..] and {..} groups, spaces allowed between them
        loop {
            let mut k = j;
            while k < b.len() && (b[k] == b' ' || b[k] == b'\t') {
                k += 1;
            }
            match b.get(k) {
                Some(b'{') => match crate::front::group_end(m, k) {
                    Some(e) => j = e,
                    None => break,
                },
                Some(b'[') => match bracket_end(m, k) {
                    Some(e) => j = e,
                    None => break,
                },
                _ => break,
            }
        }
        out.push(start..j);
        at = j;
    }
    out
}

fn bracket_end(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s[open + 1..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ']' if depth == 0 => return Some(open + 1 + i + 1),
            '\n' if s[open + 1..open + 1 + i].ends_with('\n') => return None, // a blank line: not an argument
            _ => {}
        }
    }
    None
}

/// main.tex with its author block replaced by one written for `people`.
/// Without an author block, the new one goes right after `\title{...}`.
pub fn apply(main: &str, people: &[Person], affils: &[Affiliation]) -> String {
    let style = detect(main);
    let block = write(style, people, affils);
    let m = crate::front::masked(main);
    let limit = front_limit(&m);
    let ranges: Vec<Range<usize>> = block_commands(style).iter().flat_map(|c| full_ranges(&m, c, limit)).collect();
    let region = match (ranges.iter().map(|r| r.start).min(), ranges.iter().map(|r| r.end).max()) {
        (Some(s), Some(e)) => s..e,
        _ => {
            let at = full_ranges(&m, "title", limit).first().map(|r| r.end).unwrap_or(limit);
            return format!("{}\n{}{}", &main[..at], block, &main[at..]);
        }
    };
    format!("{}{}{}", &main[..region.start], block, &main[region.end..])
}

/// Escape the characters that break LaTeX in names and addresses.
fn tex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.trim().chars() {
        match c {
            '&' | '%' | '$' | '#' | '_' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn nums(v: &[usize]) -> String {
    v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",")
}

/// "A", "A and B", "A, B, and C".
fn and_list(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{} and {}", names[0], names[1]),
        n => format!("{}, and {}", names[..n - 1].join(", "), names[n - 1]),
    }
}

/// "Department, Organization, City, Country" without the empty parts.
fn affil_line(a: &Affiliation) -> String {
    [&a.department, &a.organization, &a.city, &a.country].iter().map(|s| tex(s)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")
}

const EQUAL: &str = "These authors contributed equally to this work.";

/// The author block for a style.
pub fn write(style: Style, people: &[Person], affils: &[Affiliation]) -> String {
    let mut o = String::new();
    let any_corr = people.iter().any(|p| p.corresponding);
    let any_equal = people.iter().any(|p| p.equal);
    match style {
        Style::Springer => {
            for p in people {
                let name = match (tex(&p.given), tex(&p.family)) {
                    (g, f) if g.is_empty() => format!("\\sur{{{f}}}"),
                    (g, f) => format!("\\fnm{{{g}}} \\sur{{{f}}}"),
                };
                o += &format!("\\author{}[{}]{{{name}}}", if p.corresponding { "*" } else { "" }, nums(&p.affils));
                if !p.email.is_empty() {
                    o += &format!("\\email{{{}}}", tex(&p.email));
                }
                o += "\n";
                if p.equal {
                    o += &format!("\\equalcont{{{EQUAL}}}\n");
                }
            }
            // the corresponding author's first affiliation carries the star
            let star = people.iter().find(|p| p.corresponding).map(|p| p.affils[0]);
            for (i, a) in affils.iter().enumerate() {
                let mut parts = Vec::new();
                if !a.department.trim().is_empty() {
                    parts.push(format!("\\orgdiv{{{}}}", tex(&a.department)));
                }
                parts.push(format!("\\orgname{{{}}}", tex(&a.organization)));
                let addr: Vec<String> = [("city", &a.city), ("country", &a.country)]
                    .iter()
                    .filter(|(_, v)| !v.trim().is_empty())
                    .map(|(k, v)| format!("\\{k}{{{}}}", tex(v)))
                    .collect();
                if !addr.is_empty() {
                    parts.push(format!("\\orgaddress{{{}}}", addr.join(", ")));
                }
                o += &format!("\\affil{}[{}]{{{}}}\n", if star == Some(i + 1) { "*" } else { "" }, i + 1, parts.join(", "));
            }
        }
        Style::Elsevier | Style::Cas => {
            let cas = style == Style::Cas;
            for p in people {
                if cas {
                    o += &format!("\\author[{}]{{{}}}", nums(&p.affils), tex(&p.name()));
                    if !p.orcid.is_empty() {
                        o += &format!("[orcid={}]", tex(&p.orcid));
                    }
                    o += "\n";
                    if p.corresponding {
                        o += "\\cormark[1]\n";
                    }
                    if p.equal {
                        o += "\\fnmark[1]\n";
                    }
                } else {
                    let mut marks = String::new();
                    if p.corresponding {
                        marks += "\\corref{cor1}";
                    }
                    if p.equal {
                        marks += "\\fnref{eq1}";
                    }
                    o += &format!("\\author[{}]{{{}{marks}}}\n", nums(&p.affils), tex(&p.name()));
                }
                if p.corresponding && !p.email.is_empty() {
                    o += &format!("\\ead{{{}}}\n", tex(&p.email));
                }
            }
            for (i, a) in affils.iter().enumerate() {
                let org = [&a.department, &a.organization].iter().map(|s| tex(s)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                let mut keys = vec![format!("organization={{{org}}}")];
                if !a.city.trim().is_empty() {
                    keys.push(format!("city={{{}}}", tex(&a.city)));
                }
                if !a.country.trim().is_empty() {
                    keys.push(format!("country={{{}}}", tex(&a.country)));
                }
                o += &format!("\\affiliation[{}]{{{}}}\n", i + 1, keys.join(", "));
            }
            if any_corr {
                o += &format!("\\cortext[{}]{{Corresponding author}}\n", if cas { "1" } else { "cor1" });
            }
            if any_equal {
                o += &format!("\\fntext[{}]{{{EQUAL}}}\n", if cas { "1" } else { "eq1" });
            }
        }
        Style::IeeeJournal => {
            let names: Vec<String> = people.iter().map(|p| tex(&p.name())).collect();
            o += &format!("\\author{{{}", and_list(&names));
            // "A and B are with <first affiliation> (e-mail: ...)."
            for (i, a) in affils.iter().enumerate() {
                let group: Vec<&Person> = people.iter().filter(|p| p.affils[0] == i + 1).collect();
                if group.is_empty() {
                    continue;
                }
                let who = and_list(&group.iter().map(|p| tex(&p.name())).collect::<Vec<_>>());
                let mails: Vec<String> = group.iter().filter(|p| !p.email.is_empty()).map(|p| tex(&p.email)).collect();
                let mail = if mails.is_empty() { String::new() } else { format!(" (e-mail: {})", mails.join("; ")) };
                o += &format!("\n\\thanks{{{who} {} with {}{mail}.}}", if group.len() == 1 { "is" } else { "are" }, affil_line(a));
            }
            if any_equal {
                let eq: Vec<String> = people.iter().filter(|p| p.equal).map(|p| tex(&p.name())).collect();
                o += &format!("\n\\thanks{{{} contributed equally to this work.}}", and_list(&eq));
            }
            for p in people.iter().filter(|p| p.corresponding) {
                let mail = if p.email.is_empty() { String::new() } else { format!(" (e-mail: {})", tex(&p.email)) };
                o += &format!("\n\\thanks{{Corresponding author: {}{mail}.}}", tex(&p.name()));
            }
            o += "}";
        }
        Style::IeeeConference => {
            let blocks: Vec<String> = people
                .iter()
                .map(|p| {
                    let a = &affils[p.affils[0] - 1];
                    let mut lines = Vec::new();
                    if !a.department.trim().is_empty() {
                        lines.push(format!("\\textit{{{}}}", tex(&a.department)));
                    }
                    lines.push(format!("\\textit{{{}}}", tex(&a.organization)));
                    let place = [&a.city, &a.country].iter().map(|s| tex(s)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                    if !place.is_empty() {
                        lines.push(place);
                    }
                    if !p.email.is_empty() {
                        lines.push(tex(&p.email));
                    }
                    format!("\\IEEEauthorblockN{{{}}}\n\\IEEEauthorblockA{{{}}}", tex(&p.name()), lines.join(" \\\\\n"))
                })
                .collect();
            o += &format!("\\author{{{}}}", blocks.join("\n\\and\n"));
        }
        Style::Lncs => {
            let names: Vec<String> = people
                .iter()
                .map(|p| {
                    let mut s = format!("{}\\inst{{{}}}", tex(&p.name()), nums(&p.affils));
                    if !p.orcid.is_empty() {
                        s += &format!("\\orcidID{{{}}}", tex(&p.orcid));
                    }
                    if p.corresponding {
                        s += "\\thanks{Corresponding author}";
                    }
                    if p.equal {
                        s += &format!("\\thanks{{{EQUAL}}}");
                    }
                    s
                })
                .collect();
            o += &format!("\\author{{{}}}\n", names.join(" \\and\n"));
            let running = match people.len() {
                0 => String::new(),
                1 => people[0].short(),
                2 => format!("{} and {}", people[0].short(), people[1].short()),
                _ => format!("{} et al.", people[0].short()),
            };
            o += &format!("\\authorrunning{{{}}}\n", tex(&running));
            let insts: Vec<String> = affils
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let mails: Vec<String> = people.iter().filter(|p| p.affils[0] == i + 1 && !p.email.is_empty()).map(|p| tex(&p.email)).collect();
                    if mails.is_empty() {
                        affil_line(a)
                    } else {
                        format!("{}\\\\\n\\email{{{}}}", affil_line(a), mails.join(", "))
                    }
                })
                .collect();
            o += &format!("\\institute{{{}}}", insts.join(" \\and\n"));
        }
        Style::Blocks { big_and } => {
            let blocks: Vec<String> = people
                .iter()
                .map(|p| {
                    let mut name = tex(&p.name());
                    if p.corresponding {
                        name += "\\thanks{Corresponding author.}";
                    }
                    if p.equal {
                        name += "\\thanks{Equal contribution.}";
                    }
                    let mut lines = vec![name];
                    for &n in &p.affils {
                        let a = &affils[n - 1];
                        for s in [&a.department, &a.organization] {
                            if !s.trim().is_empty() {
                                lines.push(tex(s));
                            }
                        }
                        let place = [&a.city, &a.country].iter().map(|s| tex(s)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                        if !place.is_empty() {
                            lines.push(place);
                        }
                    }
                    if !p.email.is_empty() {
                        lines.push(if big_and { format!("\\texttt{{{}}}", tex(&p.email)) } else { format!("{{\\tt\\small {}}}", tex(&p.email)) });
                    }
                    lines.join("\\\\\n")
                })
                .collect();
            o += &format!("\\author{{{}\n}}", blocks.join(if big_and { "\n\\And\n" } else { "\n\\and\n" }));
        }
    }
    o.trim_end_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people() -> (Vec<Person>, Vec<Affiliation>) {
        let authors = vec![
            Author { id: "A001".into(), given: "Shang-Lin".into(), family: "Chung".into(), email: "me@u.edu".into(), orcid: "0000-0001-2345-6789".into(), affiliations: vec!["F001".into()], ..Default::default() },
            Author { id: "A002".into(), given: "Mei".into(), family: "Lin".into(), affiliations: vec!["F002".into(), "F001".into()], ..Default::default() },
        ];
        let affils = vec![
            Affiliation { id: "F001".into(), department: "Dept. of CS & IE".into(), organization: "National U".into(), city: "Taipei".into(), country: "Taiwan".into() },
            Affiliation { id: "F002".into(), department: String::new(), organization: "City Hospital".into(), city: String::new(), country: "Taiwan".into() },
        ];
        let chosen = vec![PaperAuthor { id: "A001".into(), corresponding: true, equal: true }, PaperAuthor { id: "A002".into(), corresponding: false, equal: true }];
        resolve(&chosen, &authors, &affils).unwrap()
    }

    #[test]
    fn affiliations_read_from_csv_text_or_a_json_list() {
        let a: Author = serde_json::from_str(r#"{"id":"A1","family":"X","affiliations":["F1"," F2 "]}"#).unwrap();
        assert_eq!(a.affiliations, ["F1", "F2"]);
        let b: Author = serde_json::from_str(r#"{"id":"A1","family":"X","affiliations":"F1; F2;"}"#).unwrap();
        assert_eq!(b.affiliations, ["F1", "F2"]);
        // the CSV reader: a list, an empty cell, a number-only cell
        let csv = "id,given,family,email,orcid,affiliations,role,note\nA1,,X,,,F1;F2,,\nA2,,Y,,,,,\nA3,,Z,,,7,,\n";
        let rows: Vec<Author> = csv::Reader::from_reader(csv.as_bytes()).deserialize().map(|r| r.unwrap()).collect();
        assert_eq!(rows[0].affiliations, ["F1", "F2"]);
        assert!(rows[1].affiliations.is_empty());
        assert_eq!(rows[2].affiliations, ["7"]);
    }

    #[test]
    fn numbers_affiliations_by_first_use() {
        let (p, a) = people();
        assert_eq!(p[0].affils, vec![1]);
        assert_eq!(p[1].affils, vec![2, 1]);
        assert_eq!(a.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["F001", "F002"]);
        let missing = resolve(&[PaperAuthor { id: "A404".into(), ..Default::default() }], &[], &[]);
        assert!(missing.is_err());
    }

    #[test]
    fn detects_the_class_family() {
        assert_eq!(detect("\\documentclass[referee,sn-basic]{sn-jnl}"), Style::Springer);
        assert_eq!(detect("\\documentclass[preprint,12pt]{elsarticle}"), Style::Elsevier);
        assert_eq!(detect("\\documentclass[a4paper,fleqn]{cas-dc}"), Style::Cas);
        assert_eq!(detect("\\documentclass[journal]{IEEEtran}"), Style::IeeeJournal);
        assert_eq!(detect("\\documentclass[journal,twoside,web]{ieeecolor}"), Style::IeeeJournal);
        assert_eq!(detect("\\documentclass[conference]{IEEEtran}"), Style::IeeeConference);
        assert_eq!(detect("\\documentclass[runningheads]{llncs}"), Style::Lncs);
        assert_eq!(detect("\\documentclass{article}\n\\author{A \\And B}"), Style::Blocks { big_and: true });
        assert_eq!(detect("% \\documentclass{llncs}\n\\documentclass{article}"), Style::Blocks { big_and: false });
    }

    #[test]
    fn springer_block_replaces_the_sample() {
        let main = "\\documentclass{sn-jnl}\n\\begin{document}\n\\title[T]{T}\n\\author*[1,2]{\\fnm{First} \\sur{Author}}\\email{x@y}\n\\equalcont{eq}\n\\affil*[1]{\\orgdiv{D}, \\orgname{O}}\n\\affil[2]{\\orgname{O2}}\n\n\\abstract{A.}\n\\maketitle\n\\end{document}\n";
        let (p, a) = people();
        let out = apply(main, &p, &a);
        assert!(out.contains("\\author*[1]{\\fnm{Shang-Lin} \\sur{Chung}}\\email{me@u.edu}\n\\equalcont{"), "{out}");
        assert!(out.contains("\\author[2,1]{\\fnm{Mei} \\sur{Lin}}\n"));
        assert!(out.contains("\\affil*[1]{\\orgdiv{Dept. of CS \\& IE}, \\orgname{National U}, \\orgaddress{\\city{Taipei}, \\country{Taiwan}}}"));
        assert!(out.contains("\\affil[2]{\\orgname{City Hospital}, \\orgaddress{\\country{Taiwan}}}\n\n\\abstract{A.}"));
        assert!(!out.contains("First"));
        assert!(out.starts_with("\\documentclass{sn-jnl}\n\\begin{document}\n\\title[T]{T}\n\\author*"));
    }

    #[test]
    fn cas_keeps_orcid_and_marks() {
        let main = "\\documentclass{cas-dc}\n\\begin{document}\n\\title [mode = title]{}\n\\tnotemark[1]\n\\author[1]{}\n\\cormark[1]\n\\ead{}\n\\affiliation[1]{organization={},\n            city={}}\n\\cortext[1]{Corresponding author}\n\\fntext[1]{}\n\\begin{abstract}\nA\n\\end{abstract}\n";
        let (p, a) = people();
        let out = apply(main, &p, &a);
        assert!(out.contains("\\tnotemark[1]\n\\author[1]{Shang-Lin Chung}[orcid=0000-0001-2345-6789]\n\\cormark[1]\n\\fnmark[1]\n\\ead{me@u.edu}\n"), "{out}");
        assert!(out.contains("\\affiliation[2]{organization={City Hospital}, country={Taiwan}}\n\\cortext[1]{Corresponding author}\n\\fntext[1]{These"));
        assert!(out.contains("\\fntext[1]{These authors contributed equally to this work.}\n\\begin{abstract}"));
        assert_eq!(out.matches("\\cortext").count(), 1);
    }

    #[test]
    fn elsarticle_uses_corref() {
        let main = "\\documentclass{elsarticle}\n\\begin{document}\n\\begin{frontmatter}\n\\title{}\n\\author{} \n\\affiliation{organization={},\n            country={}}\n\\begin{abstract}\n";
        let (p, a) = people();
        let out = apply(main, &p, &a);
        assert!(out.contains("\\author[1]{Shang-Lin Chung\\corref{cor1}\\fnref{eq1}}\n\\ead{me@u.edu}\n\\author[2,1]{Mei Lin\\fnref{eq1}}\n"), "{out}");
        assert!(out.contains("\\cortext[cor1]{Corresponding author}\n\\fntext[eq1]{"));
        assert!(out.contains(" \n\\begin{abstract}") || out.contains("}\n\\begin{abstract}"));
    }

    #[test]
    fn ieee_journal_thanks_per_affiliation() {
        let main = "\\documentclass[journal]{IEEEtran}\n\\begin{document}\n\\title{T}\n\\author{Michael~Shell,~\\IEEEmembership{Member}\n\\thanks{M. Shell was with GT.}}\n\\markboth{J}{S}\n\\maketitle\n";
        let (p, a) = people();
        let out = apply(main, &p, &a);
        assert!(out.contains("\\author{Shang-Lin Chung and Mei Lin\n\\thanks{Shang-Lin Chung is with Dept. of CS \\& IE, National U, Taipei, Taiwan (e-mail: me@u.edu).}\n\\thanks{Mei Lin is with City Hospital, Taiwan.}"), "{out}");
        assert!(out.contains("\\thanks{Corresponding author: Shang-Lin Chung (e-mail: me@u.edu).}}\n\\markboth"));
        assert!(!out.contains("Shell"));
    }

    #[test]
    fn ieee_conference_blocks() {
        let main = "\\documentclass[conference]{IEEEtran}\n\\begin{document}\n\\title{T}\n\\author{\\IEEEauthorblockN{M}\n\\IEEEauthorblockA{GT}\n\\and\n\\IEEEauthorblockN{H}\n\\IEEEauthorblockA{Fox}}\n\\maketitle\n";
        let (p, a) = people();
        let out = apply(main, &p, &a);
        assert!(out.contains("\\author{\\IEEEauthorblockN{Shang-Lin Chung}\n\\IEEEauthorblockA{\\textit{Dept. of CS \\& IE} \\\\\n\\textit{National U} \\\\\nTaipei, Taiwan \\\\\nme@u.edu}\n\\and\n\\IEEEauthorblockN{Mei Lin}\n\\IEEEauthorblockA{\\textit{City Hospital} \\\\\nTaiwan}}\n\\maketitle"), "{out}");
    }

    #[test]
    fn lncs_institutes_and_running_head() {
        let main = "\\documentclass[runningheads]{llncs}\n\\begin{document}\n\\title{T}\n\\titlerunning{T}\n\\author{First Author\\inst{1} \\and Second Author\\inst{2}}\n\\authorrunning{F. Author et al.}\n\\institute{I1 \\and I2}\n\\maketitle\n";
        let (p, a) = people();
        let out = apply(main, &p, &a);
        assert!(out.contains("\\titlerunning{T}\n\\author{Shang-Lin Chung\\inst{1}\\orcidID{0000-0001-2345-6789}\\thanks{Corresponding author}\\thanks{These authors contributed equally to this work.} \\and\nMei Lin\\inst{2,1}\\thanks{"), "{out}");
        assert!(out.contains("\\authorrunning{S. Chung and M. Lin}\n\\institute{Dept. of CS \\& IE, National U, Taipei, Taiwan\\\\\n\\email{me@u.edu} \\and\nCity Hospital, Taiwan}\n\\maketitle"));
    }

    #[test]
    fn plain_blocks_in_the_preamble() {
        let main = "\\documentclass{article}\n\\title{T}\n\\author{First Author\\\\\nInstitution1\\\\\n{\\tt\\small a@b}\n\\and\nSecond Author\n}\n\\begin{document}\n\\maketitle\n";
        let (p, a) = people();
        let out = apply(main, &p, &a);
        assert!(out.contains("\\author{Shang-Lin Chung\\thanks{Corresponding author.}\\thanks{Equal contribution.}\\\\\nDept. of CS \\& IE\\\\\nNational U\\\\\nTaipei, Taiwan\\\\\n{\\tt\\small me@u.edu}\n\\and\nMei Lin"), "{out}");
        assert!(out.contains("}\n\\begin{document}\n\\maketitle"));
        let neurips = "\\documentclass{article}\n\\title{T}\n\\author{A \\\\ \\texttt{a} \\And B}\n\\begin{document}\n";
        assert!(apply(neurips, &p, &a).contains("\n\\And\nMei Lin"));
    }

    #[test]
    fn inserts_after_title_when_there_is_no_author() {
        let main = "\\documentclass{article}\n\\title{T}\n\\begin{document}\n\\maketitle\n";
        let (p, a) = people();
        let out = apply(main, &p, &a);
        assert!(out.starts_with("\\documentclass{article}\n\\title{T}\n\\author{Shang-Lin Chung"), "{out}");
    }

    #[test]
    fn reads_names_from_an_imported_author_block() {
        let ieee = "\\documentclass{ieeecolor}\n\\author{Ellrin~Lan, Kiro~Tamsin, and Daidai~Wu%\n\\thanks{Supported by a grant.}\n\\thanks{Ellrin~Lan is with the Lab.}}\n\\begin{document}\n\\maketitle\n\\end{document}\n";
        assert_eq!(names_in_block(ieee), ["Ellrin Lan", "Kiro Tamsin", "Daidai Wu"]);
        let sn = "\\documentclass{sn-jnl}\n\\begin{document}\n\\author*[1]{\\fnm{Daidai} \\sur{Wu}}\\email{d@x}\n\\author[2]{\\fnm{Ellrin} \\sur{Lan}}\n\\maketitle\n\\end{document}\n";
        assert_eq!(names_in_block(sn), ["Daidai Wu", "Ellrin Lan"]);
        let ieee_conf = "\\documentclass{IEEEtran}\n\\begin{document}\n\\author{\\IEEEauthorblockN{Ellrin Lan\\IEEEauthorrefmark{1}}\n\\IEEEauthorblockA{Lab}}\n\\maketitle\n\\end{document}\n";
        assert!(names_in_block(ieee_conf).contains(&"Ellrin Lan".to_string()), "{:?}", names_in_block(ieee_conf));
        let profile = vec![
            Author { id: "A001".into(), given: "Ellrin".into(), family: "Lan".into(), ..Default::default() },
            Author { id: "A002".into(), given: "Dai-dai".into(), family: "Wu".into(), ..Default::default() },
        ];
        // block order; a hyphen in the given name does not matter; unknown people are left out
        let got: Vec<String> = match_block(ieee, &profile).into_iter().map(|a| a.id).collect();
        assert_eq!(got, ["A001", "A002"]);
        assert!(match_block("\\documentclass{article}\n\\begin{document}\n\\end{document}\n", &profile).is_empty());
    }
}
