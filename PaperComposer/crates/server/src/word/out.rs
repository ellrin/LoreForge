//! Word export with the review: the open tracked changes of review.json
//! become `w:ins` / `w:del` with their author and time, comments become
//! Word comments on their text (replies as Word replies, resolved ones
//! marked done), and figures, tables and equations carry their numbers so
//! "Fig. 1" and "Eq. (1)" read as in the PDF.
//!
//! How: the section files are copied with private-use markers around each
//! change and comment (placed between whole LaTeX tokens: `$…$`, `\cmd{…}`,
//! environments, words, so pandoc's reader keeps them in the paragraph's
//! inline list), pandoc reads that to JSON, the markers become pandoc's own
//! `insertion` / `deletion` / `comment-start` / `comment-end` spans (which
//! its docx writer turns into Word's), captions and equations get their
//! numbers, and the finished .docx gets what pandoc does not write: deleted
//! and inserted paragraph marks, unique change ids, and
//! `commentsExtended.xml` (replies and done).
//!
//! What review mode holds, Word holds: accepted changes are already part of
//! the text (review.json drops them), open ones are marked. Changes to
//! figures, tables, equations and headings cannot be marked inside them:
//! they export as they read now and are counted in the report's warnings.

use pc_core::paper;
use pc_core::review::{self, Place, Review};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub const INS_S: char = '\u{E010}';
pub const INS_E: char = '\u{E011}';
pub const DEL_S: char = '\u{E012}';
pub const DEL_E: char = '\u{E013}';
pub const CM_S: char = '\u{E014}';
pub const CM_E: char = '\u{E015}';
pub const SHUT: char = '\u{E016}';

fn mk(c: char, id: usize) -> String {
    format!("{c}{id}{SHUT}")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Who {
    pub author: String,
    pub date: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WordComment {
    pub author: String,
    pub date: String,
    pub text: String,
    pub done: bool,
    /// The comment this one replies to (an index into `comments`).
    pub parent: Option<usize>,
}

/// What the markers stand for.
#[derive(Debug, Default)]
pub struct Marks {
    pub changes: Vec<Who>,
    pub comments: Vec<WordComment>,
    /// Changes that could not be marked (figures, tables, …), comments placed on a whole paragraph.
    pub unmarked: usize,
}

impl Marks {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.comments.is_empty()
    }
}

// ── LaTeX tokens and the word diff (as review.js draws them) ────

fn group_end(s: &[char], i: usize) -> usize {
    let mut d = 0i32;
    let mut k = i;
    while k < s.len() {
        match s[k] {
            '\\' => k += 1,
            '{' => d += 1,
            '}' => {
                d -= 1;
                if d == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
        k += 1;
    }
    s.len()
}

fn until(s: &[char], i: usize, close: &str) -> usize {
    let close: Vec<char> = close.chars().collect();
    let mut k = i;
    while k < s.len() {
        if s[k..].starts_with(&close) {
            return k + close.len();
        }
        if s[k] == '\\' {
            k += 1;
        }
        k += 1;
    }
    s.len()
}

fn cjk(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}')
}

/// Words, spaces and punctuation; math, commands with their arguments and environments whole.
pub fn tokens(text: &str) -> Vec<String> {
    let s: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        let mut j = i + 1;
        if c.is_whitespace() {
            while j < s.len() && s[j].is_whitespace() {
                j += 1;
            }
        } else if c == '%' {
            j = s[i..].iter().position(|&x| x == '\n').map(|k| i + k).unwrap_or(s.len());
        } else if c == '$' {
            j = if s.get(i + 1) == Some(&'$') { until(&s, i + 2, "$$") } else { until(&s, i + 1, "$") };
        } else if c == '{' {
            j = group_end(&s, i);
        } else if c == '\\' {
            let d = s.get(i + 1).copied().unwrap_or(' ');
            if d == '(' {
                j = until(&s, i + 2, "\\)");
            } else if d == '[' {
                j = until(&s, i + 2, "\\]");
            } else if d.is_ascii_alphabetic() || d == '@' {
                j = i + 1;
                while j < s.len() && (s[j].is_ascii_alphabetic() || s[j] == '@') {
                    j += 1;
                }
                let name: String = s[i + 1..j].iter().collect();
                if s.get(j) == Some(&'*') {
                    j += 1;
                }
                if name == "begin" && s.get(j) == Some(&'{') {
                    let e = group_end(&s, j);
                    let env: String = s[j + 1..e.saturating_sub(1)].iter().collect();
                    let open: Vec<char> = format!("\\begin{{{env}}}").chars().collect();
                    let end: Vec<char> = format!("\\end{{{env}}}").chars().collect();
                    let mut depth = 0;
                    let mut k = i;
                    while k < s.len() {
                        if s[k..].starts_with(&open) {
                            depth += 1;
                        } else if s[k..].starts_with(&end) {
                            depth -= 1;
                            if depth == 0 {
                                k += end.len();
                                break;
                            }
                        }
                        k += 1;
                    }
                    j = k.min(s.len());
                } else {
                    loop {
                        if s.get(j) == Some(&'[') {
                            j = until(&s, j + 1, "]");
                            continue;
                        }
                        if s.get(j) == Some(&'{') {
                            j = group_end(&s, j);
                            continue;
                        }
                        break;
                    }
                }
            } else {
                j = (i + 2).min(s.len());
            }
        } else if c.is_ascii_digit() {
            while j < s.len() && (s[j].is_ascii_digit() || (matches!(s[j], '.' | ',') && s.get(j + 1).is_some_and(|x| x.is_ascii_digit()))) {
                j += 1;
            }
        } else if cjk(c) {
        } else if c.is_alphanumeric() {
            while j < s.len() && (s[j].is_alphanumeric() || matches!(s[j], '\'' | '’' | '-')) && !cjk(s[j]) {
                j += 1;
            }
        }
        out.push(s[i..j].iter().collect());
        i = j;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    Eq,
    Del,
    Ins,
}

/// Word-level runs from `a` to `b` (each a list of whole tokens).
pub fn diff(a: &str, b: &str) -> Vec<(Run, Vec<String>)> {
    // spaces ride on the token before them, so words align, not the gaps between them
    let glue = |v: Vec<String>| {
        let mut out: Vec<String> = Vec::new();
        for t in v {
            match out.last_mut() {
                Some(l) if t.trim().is_empty() && !l.trim().is_empty() => l.push_str(&t),
                _ => out.push(t),
            }
        }
        out
    };
    let x = glue(tokens(a));
    let y = glue(tokens(b));
    let (m, n) = (x.len(), y.len());
    let same = |p: &str, q: &str| p.trim_end() == q.trim_end();
    let mut raw: Vec<(Run, String)> = Vec::new();
    if m * n > 400_000 {
        raw.extend(x.into_iter().map(|t| (Run::Del, t)));
        raw.extend(y.into_iter().map(|t| (Run::Ins, t)));
    } else {
        let mut l = vec![vec![0u32; n + 1]; m + 1];
        for i in (0..m).rev() {
            for j in (0..n).rev() {
                l[i][j] = if same(&x[i], &y[j]) { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < m || j < n {
            if i < m && j < n && same(&x[i], &y[j]) {
                raw.push((Run::Eq, y[j].clone()));
                i += 1;
                j += 1;
            } else if j < n && (i == m || l[i][j + 1] >= l[i + 1][j]) {
                raw.push((Run::Ins, y[j].clone()));
                j += 1;
            } else {
                raw.push((Run::Del, x[i].clone()));
                i += 1;
            }
        }
    }
    // a lone space between two changes joins them
    let mut toks: Vec<(Run, String)> = Vec::new();
    for k in 0..raw.len() {
        let (r, t) = &raw[k];
        if *r == Run::Eq && t.trim().is_empty() && k > 0 && k + 1 < raw.len() && raw[k - 1].0 != Run::Eq && raw[k + 1].0 != Run::Eq {
            toks.push((Run::Del, t.clone()));
            toks.push((Run::Ins, t.clone()));
        } else {
            toks.push((*r, t.clone()));
        }
    }
    // each stretch of changes: its deletion, then its insertion
    let mut out: Vec<(Run, Vec<String>)> = Vec::new();
    let mut k = 0;
    while k < toks.len() {
        if toks[k].0 == Run::Eq {
            match out.last_mut() {
                Some((Run::Eq, v)) => v.push(toks[k].1.clone()),
                _ => out.push((Run::Eq, vec![toks[k].1.clone()])),
            }
            k += 1;
            continue;
        }
        let (mut del, mut ins) = (Vec::new(), Vec::new());
        while k < toks.len() && toks[k].0 != Run::Eq {
            if toks[k].0 == Run::Del { del.push(toks[k].1.clone()) } else { ins.push(toks[k].1.clone()) }
            k += 1;
        }
        if !del.is_empty() {
            out.push((Run::Del, del));
        }
        if !ins.is_empty() {
            out.push((Run::Ins, ins));
        }
    }
    out
}

/// Running text that can carry markers between its tokens (not an
/// environment, display math, a heading, a list, a line comment).
pub fn prose(p: &str) -> bool {
    let t = p.trim_start();
    if t.is_empty() || t.starts_with("\\[") || t.starts_with("$$") || t.contains("\\begin{") || t.contains("\\item") {
        return false;
    }
    if ["\\section", "\\subsection", "\\subsubsection", "\\paragraph{", "\\chapter", "\\caption"].iter().any(|h| t.starts_with(h)) {
        return false;
    }
    // an unescaped % would hide the markers after it
    let b: Vec<char> = p.chars().collect();
    !(0..b.len()).any(|i| b[i] == '%' && (i == 0 || b[i - 1] != '\\'))
}

// ── Anchors ─────────────────────────────────────────────

/// Whitespace runs (and `~`) as one space, with each squashed character's byte offset in `t`.
fn squash_map(t: &str) -> (String, Vec<usize>) {
    let mut out = String::new();
    let mut at = Vec::new();
    let mut space = false;
    for (i, ch) in t.char_indices() {
        if ch.is_whitespace() || ch == '~' {
            if !space {
                out.push(' ');
                at.push(i);
            }
            space = true;
        } else {
            out.push(ch);
            for _ in 0..ch.len_utf8() {
                at.push(i);
            }
            space = false;
        }
    }
    at.push(t.len());
    (out, at)
}

fn squash(t: &str) -> String {
    squash_map(t).0
}

/// Byte range of a comment's anchor in the paragraph's source, best match
/// of `exact` by its `prefix` / `suffix` (as review.rs finds it).
pub fn anchor(text: &str, exact: &str, prefix: &str, suffix: &str) -> Option<(usize, usize)> {
    let (t, at) = squash_map(text);
    let e = squash(exact).trim().to_string();
    if e.is_empty() {
        return None;
    }
    let (pre, suf) = (squash(prefix), squash(suffix));
    let mut best: Option<(usize, usize)> = None;
    for (i, _) in t.match_indices(&e) {
        let s1 = t[..i].chars().rev().zip(pre.chars().rev()).take_while(|(x, y)| x == y).count();
        let s2 = t[i + e.len()..].chars().zip(suf.chars()).take_while(|(x, y)| x == y).count();
        if best.is_none_or(|(_, s)| s1 + s2 > s) {
            best = Some((i, s1 + s2));
        }
    }
    let (i, _) = best?;
    let start = at[i];
    let end = at.get(i + e.len()).copied().unwrap_or(text.len());
    // the end offset points at the character after the match: step back over trailing space
    Some((start, end.max(start)))
}

/// One comment on a paragraph: where (bytes of the current text, None = all of it) and its Word ids.
#[derive(Debug, Clone)]
struct OnPara {
    range: Option<(usize, usize)>,
    /// The comment and its replies, in order.
    ids: Vec<usize>,
}

/// The paragraph with its change and comment markers. `before` is
/// None for an inserted paragraph; `change` the Word change index.
fn render_para(before: Option<&str>, after: &str, change: Option<usize>, comments: &[OnPara]) -> String {
    let runs: Vec<(Run, Vec<String>)> = match change {
        Some(_) => diff(before.unwrap_or(""), after),
        None => vec![(Run::Eq, tokens(after))],
    };
    let mut out = String::new();
    // after-text byte position of each token
    let mut pos = 0usize;
    let mut started = vec![false; comments.len()];
    let mut ended = vec![false; comments.len()];
    let starts = |c: &OnPara| c.range.map(|r| r.0).unwrap_or(0);
    let ends = |c: &OnPara| c.range.map(|r| r.1).unwrap_or(after.len());
    for (run, toks) in &runs {
        let id = change.unwrap_or(0);
        match run {
            Run::Del => {
                out += &mk(DEL_S, id);
                out += &toks.concat();
                out += &mk(DEL_E, id);
                continue;
            }
            Run::Ins => out += &mk(INS_S, id),
            Run::Eq => {}
        }
        // spaces apart again, so a comment ends at its word
        for t in &tokens(&toks.concat()) {
            let end = pos + t.len();
            for (k, c) in comments.iter().enumerate() {
                if !started[k] && starts(c) < end.max(pos + 1) && !t.trim().is_empty() {
                    c.ids.iter().for_each(|&i| out += &mk(CM_S, i));
                    started[k] = true;
                }
            }
            out += t;
            pos = end;
            for (k, c) in comments.iter().enumerate() {
                if started[k] && !ended[k] && ends(c) <= pos {
                    c.ids.iter().for_each(|&i| out += &mk(CM_E, i));
                    ended[k] = true;
                }
            }
        }
        if *run == Run::Ins {
            out += &mk(INS_E, change.unwrap_or(0));
        }
    }
    for (k, c) in comments.iter().enumerate() {
        if !started[k] {
            c.ids.iter().for_each(|&i| out += &mk(CM_S, i));
        }
        if !ended[k] {
            c.ids.iter().for_each(|&i| out += &mk(CM_E, i));
        }
    }
    out
}

/// A non-prose paragraph's comment: inside its `\caption{…}`, else around it.
fn comment_block(p: &str, ids: &[usize]) -> String {
    let open: String = ids.iter().map(|&i| mk(CM_S, i)).collect();
    let close: String = ids.iter().map(|&i| mk(CM_E, i)).collect();
    if let Some(i) = p.find("\\caption{") {
        let chars: Vec<char> = p.chars().collect();
        let ci = p[..i + "\\caption".len()].chars().count();
        let e = group_end(&chars, ci);
        let inner_start: usize = chars[..ci + 1].iter().map(|c| c.len_utf8()).sum();
        let inner_end: usize = chars[..e.saturating_sub(1)].iter().map(|c| c.len_utf8()).sum();
        if inner_end >= inner_start {
            return format!("{}{open}{}{close}{}", &p[..inner_start], &p[inner_start..inner_end], &p[inner_end..]);
        }
    }
    format!("{open}{p}{close}")
}

/// The section files with markers, and what the markers stand for.
/// Keys of the map: section file names (under `sections/`).
pub fn mark(dir: &Path, r: &Review) -> (HashMap<String, String>, Marks) {
    let mut marks = Marks::default();
    let mut files = HashMap::new();
    for file in paper::section_files(dir) {
        let Ok(text) = std::fs::read_to_string(dir.join("sections").join(&file)) else { continue };
        let changes: Vec<&review::Change> = r.changes.iter().filter(|c| c.file == file).collect();
        let comments: Vec<&review::Comment> = r.comments.iter().filter(|c| c.file == file).collect();
        if changes.is_empty() && comments.is_empty() {
            continue;
        }
        let sec = paper::parse_section(&file, &text);
        let paras = &sec.paragraphs;
        // the change on each paragraph, and deleted paragraphs by gap
        let mut on: Vec<Option<(Option<String>, usize)>> = vec![None; paras.len()];
        let mut gaps: Vec<(usize, String, usize, String)> = Vec::new();
        let mut placed: Vec<(&review::Change, Place)> = changes.iter().map(|c| (*c, review::locate(paras, &r.changes, c))).collect();
        placed.sort_by(|a, b| a.0.at.cmp(&b.0.at));
        for (c, place) in &placed {
            match place {
                Place::Para(i) => {
                    // the oldest text under this paragraph's changes: rejecting all in Word gives it back
                    let mut before = c.before.clone();
                    let mut guard = 0;
                    while let Some(under) = placed.iter().find(|(d, p)| matches!(p, Place::Under(j) if j == i) && d.after.as_deref().map(review::norm) == before.as_deref().map(review::norm) && d.id != c.id) {
                        before = under.0.before.clone();
                        guard += 1;
                        if guard > 20 {
                            break;
                        }
                    }
                    let ok = prose(&paras[*i]) && before.as_deref().is_none_or(prose);
                    if ok {
                        let k = marks.changes.len();
                        marks.changes.push(Who { author: who(&c.author), date: c.at.clone() });
                        on[*i] = Some((before, k));
                    } else {
                        marks.unmarked += 1;
                    }
                }
                Place::Gap(i) => match &c.before {
                    Some(b) if prose(b) => {
                        let k = marks.changes.len();
                        marks.changes.push(Who { author: who(&c.author), date: c.at.clone() });
                        gaps.push((*i, c.at.clone(), k, b.clone()));
                    }
                    _ => marks.unmarked += 1,
                },
                Place::Under(_) => {}
                Place::Lost => marks.unmarked += 1,
            }
        }
        // comments by paragraph, each followed by its replies
        let mut by_para: Vec<Vec<(Option<(usize, usize)>, Vec<usize>)>> = vec![vec![]; paras.len()];
        for c in &comments {
            if paras.is_empty() {
                break;
            }
            let i = c.index.min(paras.len() - 1);
            let mut ids = vec![marks.comments.len()];
            let parent = marks.comments.len();
            marks.comments.push(WordComment { author: who(&c.author), date: c.at.clone(), text: c.text.clone(), done: c.resolved, parent: None });
            for rp in &c.replies {
                ids.push(marks.comments.len());
                marks.comments.push(WordComment { author: who(&rp.author), date: rp.at.clone(), text: rp.text.clone(), done: c.resolved, parent: Some(parent) });
            }
            let range = if c.orphan { None } else { anchor(&paras[i], &c.exact, &c.prefix, &c.suffix) };
            by_para[i].push((range, ids));
        }
        let mut out: Vec<String> = Vec::new();
        for (i, p) in paras.iter().enumerate() {
            let mut here: Vec<&(usize, String, usize, String)> = gaps.iter().filter(|g| g.0 == i).collect();
            here.sort_by(|a, b| a.1.cmp(&b.1));
            for g in here {
                out.push(format!("{}{}{}", mk(DEL_S, g.2), g.3, mk(DEL_E, g.2)));
            }
            let cms: Vec<OnPara> = by_para[i].iter().map(|(r, ids)| OnPara { range: *r, ids: ids.clone() }).collect();
            out.push(match &on[i] {
                Some((before, k)) => render_para(before.as_deref(), p, Some(*k), &cms),
                None if cms.is_empty() => p.clone(),
                None if prose(p) => render_para(None, p, None, &cms),
                None => comment_block(p, &cms.iter().flat_map(|c| c.ids.clone()).collect::<Vec<_>>()),
            });
        }
        let mut tail: Vec<&(usize, String, usize, String)> = gaps.iter().filter(|g| g.0 >= paras.len()).collect();
        tail.sort_by(|a, b| a.1.cmp(&b.1));
        for g in tail {
            out.push(format!("{}{}{}", mk(DEL_S, g.2), g.3, mk(DEL_E, g.2)));
        }
        let head = if text.lines().any(|l| l.trim_start().starts_with("\\section")) { format!("\\section{{{}}}{}\n\n", sec.title, sec.tail) } else { String::new() };
        files.insert(file, format!("{head}{}\n", out.join("\n\n")));
    }
    (files, marks)
}

fn who(a: &str) -> String {
    if a.trim().is_empty() { "PaperComposer".into() } else { a.to_string() }
}

/// main.tex with each marked section's `\input` replaced by its text.
pub fn main_with(main: &str, files: &HashMap<String, String>) -> String {
    let mut out = String::new();
    for line in main.lines() {
        let code = line.split('%').next().unwrap_or("");
        let mut replaced = false;
        for cmd in ["\\input{", "\\include{"] {
            let Some(i) = code.find(cmd) else { continue };
            let Some(e) = code[i + cmd.len()..].find('}') else { continue };
            let arg = code[i + cmd.len()..i + cmd.len() + e].trim();
            let name = arg.trim_start_matches("./").strip_prefix("sections/").unwrap_or("");
            let name = if name.ends_with(".tex") { name.to_string() } else { format!("{name}.tex") };
            if let Some(t) = files.get(&name) {
                out += &code[..i];
                out += "\n";
                out += t;
                out += "\n";
                out += &line[i + cmd.len() + e + 1..];
                out += "\n";
                replaced = true;
                break;
            }
        }
        if !replaced {
            out += line;
            out += "\n";
        }
    }
    out
}

// ── The pandoc tree: markers to spans ───────────────────

fn t(v: &Value) -> &str {
    v.get("t").and_then(|t| t.as_str()).unwrap_or("")
}

const INLINES: &[&str] = &[
    "Str", "Emph", "Underline", "Strong", "Strikeout", "Superscript", "Subscript", "SmallCaps", "Quoted", "Cite", "Code", "Space", "SoftBreak", "LineBreak", "Math", "RawInline", "Link", "Image", "Note", "Span",
];

fn is_inline_list(a: &[Value]) -> bool {
    !a.is_empty() && a.iter().all(|x| INLINES.contains(&t(x)))
}

fn has_marker(v: &Value) -> bool {
    match v {
        Value::String(s) => s.chars().any(|c| ('\u{E010}'..='\u{E016}').contains(&c)),
        Value::Array(a) => a.iter().any(has_marker),
        Value::Object(o) => o.values().any(has_marker),
        _ => false,
    }
}

enum Piece {
    Text(String),
    Mark(char, usize),
}

fn pieces(s: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if matches!(c, INS_S | INS_E | DEL_S | DEL_E | CM_S | CM_E) {
            let mut id = String::new();
            for d in it.by_ref() {
                if d == SHUT {
                    break;
                }
                id.push(d);
            }
            if !text.is_empty() {
                out.push(Piece::Text(std::mem::take(&mut text)));
            }
            out.push(Piece::Mark(c, id.parse().unwrap_or(0)));
        } else if c != SHUT {
            text.push(c);
        }
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    out
}

fn text_inlines(s: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for (k, line) in s.lines().enumerate() {
        if k > 0 {
            out.push(json!({"t": "LineBreak"}));
        }
        for (j, w) in line.split_whitespace().enumerate() {
            if j > 0 {
                out.push(json!({"t": "Space"}));
            }
            out.push(json!({"t": "Str", "c": w}));
        }
    }
    out
}

struct State<'a> {
    marks: &'a Marks,
    /// The change being inside: (insertion?, change index).
    active: Option<(bool, usize)>,
}

fn flush(buf: &mut Vec<Value>, out: &mut Vec<Value>, st: &State) {
    if buf.is_empty() {
        return;
    }
    match st.active {
        Some((ins, k)) => {
            let w = st.marks.changes.get(k).cloned().unwrap_or(Who { author: "PaperComposer".into(), date: review::now() });
            out.push(json!({"t": "Span", "c": [["", [if ins { "insertion" } else { "deletion" }], [["author", w.author], ["date", w.date]]], std::mem::take(buf)]}));
        }
        None => out.append(buf),
    }
}

fn inlines(list: Vec<Value>, st: &mut State) -> Vec<Value> {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    for mut x in list {
        if t(&x) == "Str" && has_marker(&x) {
            for p in pieces(x["c"].as_str().unwrap_or("")) {
                match p {
                    Piece::Text(s) => buf.push(json!({"t": "Str", "c": s})),
                    Piece::Mark(c, id) => {
                        flush(&mut buf, &mut out, st);
                        match c {
                            INS_S => st.active = Some((true, id)),
                            DEL_S => st.active = Some((false, id)),
                            INS_E | DEL_E => st.active = None,
                            CM_S => {
                                if let Some(m) = st.marks.comments.get(id) {
                                    out.push(json!({"t": "Span", "c": [["", ["comment-start"], [["id", id.to_string()], ["author", m.author], ["date", m.date]]], text_inlines(&m.text)]}));
                                }
                            }
                            _ => {
                                if st.marks.comments.get(id).is_some() {
                                    out.push(json!({"t": "Span", "c": [["", ["comment-end"], [["id", id.to_string()]]], []]}));
                                }
                            }
                        }
                    }
                }
            }
        } else if has_marker(&x) {
            flush(&mut buf, &mut out, st);
            if let Some(c) = x.get_mut("c") {
                walk(c, st);
            }
            // a container entered or left a change: what it holds is marked inside it
            out.push(x);
        } else {
            buf.push(x);
        }
    }
    flush(&mut buf, &mut out, st);
    out
}

fn walk(v: &mut Value, st: &mut State) {
    match v {
        Value::Array(a) if is_inline_list(a) && has_marker(&Value::Array(a.clone())) => {
            *a = inlines(std::mem::take(a), st);
        }
        Value::Array(a) => a.iter_mut().for_each(|x| walk(x, st)),
        Value::Object(o) => {
            if let Some(c) = o.get_mut("c") {
                walk(c, st);
            }
        }
        _ => {}
    }
}

/// Turn every marker into pandoc's insertion / deletion / comment spans.
pub fn spans(doc: &mut Value, marks: &Marks) {
    let mut st = State { marks, active: None };
    if let Some(b) = doc.get_mut("blocks") {
        // markers in math / code are removed with their ids
        remove_hidden(b);
        walk(b, &mut st);
    }
}

/// Markers pandoc put inside math or code (strings that are not `Str`): out, ids too.
fn remove_hidden(v: &mut Value) {
    fn clean(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if ('\u{E010}'..='\u{E015}').contains(&c) {
                for d in it.by_ref() {
                    if d == SHUT {
                        break;
                    }
                }
            } else if c != SHUT {
                out.push(c);
            }
        }
        out
    }
    match v {
        Value::Array(a) => a.iter_mut().for_each(remove_hidden),
        Value::Object(o) => {
            let kind = o.get("t").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if matches!(kind.as_str(), "Math" | "Code" | "RawInline" | "CodeBlock" | "RawBlock") {
                if let Some(Value::Array(c)) = o.get_mut("c") {
                    if let Some(Value::String(s)) = c.get_mut(1) {
                        *s = clean(s);
                    }
                }
                return;
            }
            if let Some(c) = o.get_mut("c") {
                remove_hidden(c);
            }
        }
        _ => {}
    }
}

// ── Numbers: figures, tables, equations ─────────────────

const NUMBERED_ENVS: &[&str] = &["equation", "align", "gather", "multline", "eqnarray", "flalign", "alignat"];

/// Is this display math a numbered environment? The math without its
/// labels and number commands, and its labels.
fn numbered_math(m: &str) -> Option<(String, Vec<String>)> {
    let s = m.trim();
    let env = NUMBERED_ENVS.iter().find(|e| s.starts_with(&format!("\\begin{{{e}}}")))?;
    let mut labels = Vec::new();
    let mut body = s[format!("\\begin{{{env}}}").len()..].to_string();
    if let Some(e) = body.rfind(&format!("\\end{{{env}}}")) {
        body.truncate(e);
    }
    // \label{…} out
    while let Some(i) = body.find("\\label{") {
        let rest = &body[i + 7..];
        let Some(e) = rest.find('}') else { break };
        labels.push(rest[..e].trim().to_string());
        body = format!("{}{}", &body[..i], &rest[e + 1..]);
    }
    for x in ["\\nonumber", "\\notag"] {
        body = body.replace(x, "");
    }
    let body = body.trim().to_string();
    let body = match *env {
        "equation" | "multline" => body,
        "gather" => format!("\\begin{{gathered}}{body}\\end{{gathered}}"),
        _ => format!("\\begin{{aligned}}{body}\\end{{aligned}}"),
    };
    Some((body, labels))
}

fn plain(inl: Vec<Value>) -> Value {
    json!({"t": "Plain", "c": inl})
}

/// A Word equation line: the formula and its number in a two-column table
/// without a header (what `ast::equation_table` reads back).
fn equation_table(math: &str, n: usize) -> Value {
    let cell = |align: &str, blocks: Value| json!([["", [], []], {"t": align}, 1, 1, blocks]);
    json!({"t": "Table", "c": [
        ["", ["pc-equation"], []],
        [null, []],
        [[{"t": "AlignCenter"}, {"t": "ColWidth", "c": 0.88}], [{"t": "AlignRight"}, {"t": "ColWidth", "c": 0.12}]],
        [["", [], []], []],
        [[["", [], []], 0, [], [[["", [], []], [
            cell("AlignCenter", json!([plain(vec![json!({"t": "Math", "c": [{"t": "DisplayMath"}, math]})])])),
            cell("AlignRight", json!([plain(vec![json!({"t": "Str", "c": format!("({n})")})])])),
        ]]]]],
        [["", [], []], []]
    ]})
}

fn prepend_caption(caption: &mut Value, word: &str, n: usize) {
    let lead = vec![json!({"t": "Str", "c": word}), json!({"t": "Space"}), json!({"t": "Str", "c": format!("{n}.")}), json!({"t": "Space"})];
    let Some(blocks) = caption.get_mut(1).and_then(|b| b.as_array_mut()) else { return };
    match blocks.iter_mut().find(|b| matches!(t(b), "Plain" | "Para")) {
        Some(b) => {
            if let Some(c) = b.get_mut("c").and_then(|c| c.as_array_mut()) {
                for (i, x) in lead.into_iter().enumerate() {
                    c.insert(i, x);
                }
                if matches!(c.last().map(t), Some("Space")) {
                    c.pop();
                }
            }
        }
        None => {
            let mut l = lead;
            l.pop();
            blocks.insert(0, plain(l));
        }
    }
}

struct Counter {
    fig: usize,
    tab: usize,
    eq: usize,
    /// label → (number as text, is equation)
    labels: HashMap<String, (String, bool)>,
}

fn number_blocks(blocks: &mut Vec<Value>, n: &mut Counter) {
    let mut out = Vec::with_capacity(blocks.len());
    for mut b in std::mem::take(blocks) {
        match t(&b) {
            "Figure" => {
                n.fig += 1;
                let id = b["c"][0][0].as_str().unwrap_or("").to_string();
                if !id.is_empty() {
                    n.labels.insert(id, (n.fig.to_string(), false));
                }
                prepend_caption(&mut b["c"][1], "Figure", n.fig);
                if let Some(inner) = b["c"].get_mut(2).and_then(|x| x.as_array_mut()) {
                    number_blocks(inner, n);
                }
                out.push(b);
            }
            "Table" => {
                n.tab += 1;
                let id = b["c"][0][0].as_str().unwrap_or("").to_string();
                if !id.is_empty() {
                    n.labels.insert(id, (n.tab.to_string(), false));
                }
                prepend_caption(&mut b["c"][1], "Table", n.tab);
                out.push(b);
            }
            "Para" | "Plain" => {
                // split around numbered display equations
                let inl = b["c"].as_array().cloned().unwrap_or_default();
                if !inl.iter().any(|x| t(x) == "Math" && t(&x["c"][0]) == "DisplayMath" && numbered_math(x["c"][1].as_str().unwrap_or("")).is_some()) {
                    out.push(b);
                    continue;
                }
                let kind = t(&b).to_string();
                let mut cur: Vec<Value> = Vec::new();
                let push_text = |cur: &mut Vec<Value>, out: &mut Vec<Value>| {
                    while cur.first().is_some_and(|x| matches!(t(x), "Space" | "SoftBreak")) {
                        cur.remove(0);
                    }
                    while cur.last().is_some_and(|x| matches!(t(x), "Space" | "SoftBreak")) {
                        cur.pop();
                    }
                    if !cur.is_empty() {
                        out.push(json!({"t": kind, "c": std::mem::take(cur)}));
                    }
                    cur.clear();
                };
                for x in inl {
                    let num = (t(&x) == "Math" && t(&x["c"][0]) == "DisplayMath").then(|| numbered_math(x["c"][1].as_str().unwrap_or(""))).flatten();
                    match num {
                        Some((math, labels)) => {
                            push_text(&mut cur, &mut out);
                            n.eq += 1;
                            for l in labels {
                                n.labels.insert(l, (n.eq.to_string(), true));
                            }
                            out.push(equation_table(&math, n.eq));
                        }
                        None => cur.push(x),
                    }
                }
                push_text(&mut cur, &mut out);
            }
            "Div" | "BlockQuote" => {
                let i = if t(&b) == "Div" { 1 } else { usize::MAX };
                let inner = if i == 1 { b["c"].get_mut(1) } else { b.get_mut("c") };
                if let Some(inner) = inner.and_then(|x| x.as_array_mut()) {
                    number_blocks(inner, n);
                }
                out.push(b);
            }
            _ => out.push(b),
        }
    }
    *blocks = out;
}

fn fix_links(v: &mut Value, n: &Counter) {
    match v {
        Value::Array(a) => a.iter_mut().for_each(|x| fix_links(x, n)),
        Value::Object(_) if t(v) == "Link" => {
            let attrs = v["c"][0][2].as_array().cloned().unwrap_or_default();
            let get = |k: &str| attrs.iter().find(|kv| kv[0] == k).and_then(|kv| kv[1].as_str()).map(String::from);
            let target = get("reference").or_else(|| v["c"][2][0].as_str().and_then(|u| u.strip_prefix('#')).map(String::from));
            let kind = get("reference-type").unwrap_or_default();
            if let Some((num, _eq)) = target.and_then(|l| n.labels.get(&l)) {
                if kind == "eqref" {
                    v["c"][1] = json!([{"t": "Str", "c": format!("({num})")}]);
                } else if matches!(kind.as_str(), "ref" | "autoref" | "cref" | "Cref") || kind.is_empty() {
                    v["c"][1] = json!([{"t": "Str", "c": num}]);
                }
            }
        }
        Value::Object(o) => {
            if let Some(c) = o.get_mut("c") {
                fix_links(c, n);
            }
        }
        _ => {}
    }
}

/// Number figures, tables and equations in captions and in every `\ref` / `\eqref`.
pub fn numbers(doc: &mut Value) {
    let mut n = Counter { fig: 0, tab: 0, eq: 0, labels: HashMap::new() };
    if let Some(b) = doc.get_mut("blocks").and_then(|b| b.as_array_mut()) {
        number_blocks(b, &mut n);
    }
    if let Some(b) = doc.get_mut("blocks") {
        fix_links(b, &n);
    }
}

// ── The citation style ─────────────────────────────────

/// A numeric citation style ([1], IEEE-like list) for papers whose
/// bibliography style is numeric and that bring no .csl of their own.
pub const NUMERIC_CSL: &str = include_str!("numeric.csl");

/// Does the paper cite by number? (`\bibliographystyle`, natbib's
/// `numbers`, biblatex's `style=`); a paper opened from Word does.
pub fn numeric_citations(main: &str) -> bool {
    let code: String = main.lines().map(|l| l.split('%').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let lower = code.to_lowercase();
    if lower.contains("natbib") && (lower.contains("[numbers") || lower.contains(",numbers")) {
        return true;
    }
    let arg = |cmd: &str| lower.find(cmd).and_then(|i| lower[i + cmd.len()..].split('}').next().map(|s| s.trim().to_string()));
    if let Some(style) = arg("\\bibliographystyle{") {
        const AUTHOR_YEAR: &[&str] = &["apalike", "plainnat", "abbrvnat", "unsrtnat", "agsm", "harvard", "chicago", "apa", "harv", "spbasic", "authordate", "named", "jfm", "ecta", "aer"];
        if style.contains("num") || style.contains("ieee") || style.contains("unsrt") && !style.contains("nat") {
            return true;
        }
        return !AUTHOR_YEAR.iter().any(|a| style.contains(a));
    }
    if let Some(i) = lower.find("\\usepackage[") {
        let opts = lower[i..].split(']').next().unwrap_or("");
        if opts.contains("biblatex") || lower[i..].split('}').next().unwrap_or("").contains("biblatex") {
            return opts.contains("numeric") || opts.contains("ieee") || opts.contains("nature") || opts.contains("vancouver");
        }
    }
    false
}

// ── The finished .docx ─────────────────────────────────

fn attr_of(tag: &str, name: &str) -> Option<String> {
    let k = format!("{name}=\"");
    let i = tag.find(&k)? + k.len();
    let e = tag[i..].find('"')?;
    Some(tag[i..i + e].to_string())
}

/// Every element `<name …>…</name>` (no nesting of the same name) removed.
fn without(s: &str, name: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    let open = format!("<{name} ");
    let close = format!("</{name}>");
    while let Some(i) = rest.find(&open) {
        out += &rest[..i];
        let after = &rest[i..];
        let gt = after.find('>').unwrap_or(after.len() - 1);
        if after[..gt].ends_with('/') {
            rest = &after[gt + 1..];
            continue;
        }
        match after.find(&close) {
            Some(e) => rest = &after[e + close.len()..],
            None => {
                rest = "";
            }
        }
    }
    out + rest
}

/// A paragraph whose every run is inserted (deleted) gets its paragraph
/// mark inserted (deleted) too, as Word writes it, so accepting a deletion
/// leaves no empty paragraph behind.
fn paragraph_marks(doc: &str) -> String {
    let mut out = String::with_capacity(doc.len() + 1024);
    let mut rest = doc;
    loop {
        let next = [rest.find("<w:p>"), rest.find("<w:p ")].into_iter().flatten().min();
        let Some(i) = next else { break };
        let Some(e) = rest[i..].find("</w:p>").map(|e| i + e + 6) else { break };
        out += &rest[..i];
        let p = &rest[i..e];
        out += &mark_paragraph(p);
        rest = &rest[e..];
    }
    out + rest
}

fn mark_paragraph(p: &str) -> String {
    // nested paragraphs (text boxes) are left alone
    if p[1..].contains("<w:p>") || p[1..].contains("<w:p ") {
        return p.to_string();
    }
    let has_content = |s: &str| s.contains("<w:t") || s.contains("<w:delText") || s.contains("<m:oMath") || s.contains("<w:drawing");
    for name in ["w:ins", "w:del"] {
        let Some(first) = p.find(&format!("<{name} ")) else { continue };
        let other = if name == "w:ins" { "w:del" } else { "w:ins" };
        if p.contains(&format!("<{other} ")) {
            continue;
        }
        let rest = without(p, name);
        if has_content(&rest) {
            continue;
        }
        let tag = &p[first..first + p[first..].find('>').unwrap_or(0)];
        let (author, date) = (attr_of(tag, "w:author").unwrap_or_default(), attr_of(tag, "w:date").unwrap_or_default());
        let mark = format!("<{name} w:id=\"0\" w:author=\"{author}\" w:date=\"{date}\"/>");
        if let Some(k) = p.find("<w:pPr>") {
            let end = p.find("</w:pPr>").unwrap_or(k);
            let ppr = &p[k..end];
            if let Some(r) = ppr.find("<w:rPr>") {
                let at = k + r + "<w:rPr>".len();
                return format!("{}{mark}{}", &p[..at], &p[at..]);
            }
            return format!("{}<w:rPr>{mark}</w:rPr>{}", &p[..end], &p[end..]);
        }
        let open_end = p.find('>').unwrap_or(0) + 1;
        return format!("{}<w:pPr><w:rPr>{mark}</w:rPr></w:pPr>{}", &p[..open_end], &p[open_end..]);
    }
    p.to_string()
}

/// Number every `w:ins` / `w:del` uniquely (pandoc gives a change's insertion
/// and deletion one id), starting above the comment ids.
fn renumber_changes(doc: &str) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut rest = doc;
    let mut n = 1000;
    loop {
        let next = [rest.find("<w:ins w:id=\""), rest.find("<w:del w:id=\"")].into_iter().flatten().min();
        let Some(i) = next else { break };
        let start = i + "<w:ins w:id=\"".len();
        let Some(e) = rest[start..].find('"') else { break };
        out += &rest[..start];
        out += &n.to_string();
        n += 1;
        rest = &rest[start + e..];
    }
    out + rest
}

fn para_id(k: usize) -> String {
    format!("{:08X}", 0x1000_0000 + k)
}

/// comments.xml with a paragraph id on each comment's last paragraph.
fn comment_para_ids(xml: &str) -> (String, HashMap<String, String>) {
    let mut ids = HashMap::new();
    let mut out = String::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<w:comment ") {
        let Some(e) = rest[i..].find("</w:comment>").map(|e| i + e) else { break };
        out += &rest[..i];
        let c = &rest[i..e];
        let id = attr_of(&c[..c.find('>').unwrap_or(c.len())], "w:id").unwrap_or_default();
        let pid = para_id(ids.len());
        let last = [c.rfind("<w:p>"), c.rfind("<w:p ")].into_iter().flatten().max();
        match last {
            Some(k) if !c[k..].starts_with("<w:p w14:") => {
                out += &c[..k + 4];
                out += &format!(" w14:paraId=\"{pid}\" w14:textId=\"77777777\"");
                out += &c[k + 4..];
            }
            _ => out += c,
        }
        ids.insert(id, pid);
        rest = &rest[e..];
    }
    out += rest;
    // the w14 namespace, ignorable by readers that do not know it
    let out = match out.find("<w:comments ") {
        Some(i) if !out.contains("xmlns:w14=") => {
            let at = i + "<w:comments ".len();
            format!("{}xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" mc:Ignorable=\"w14\" {}", &out[..at], &out[at..])
        }
        _ => out,
    };
    (out, ids)
}

const CX_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
const CX_REL: &str = "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";

fn comments_extended(marks: &Marks, para: &HashMap<String, String>) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w15:commentsEx xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\" mc:Ignorable=\"w15\">");
    for (k, c) in marks.comments.iter().enumerate() {
        let Some(pid) = para.get(&k.to_string()) else { continue };
        let parent = c.parent.and_then(|p| para.get(&p.to_string())).map(|p| format!(" w15:paraIdParent=\"{p}\"")).unwrap_or_default();
        s += &format!("<w15:commentEx w15:paraId=\"{pid}\"{parent} w15:done=\"{}\"/>", if c.done { 1 } else { 0 });
    }
    s + "</w15:commentsEx>"
}

/// Add to the .docx what pandoc does not write. Rewrites the file in place.
pub fn finish(docx: &Path, marks: &Marks) -> Result<(), String> {
    use std::io::{Read, Write};
    let bytes = std::fs::read(docx).map_err(|e| e.to_string())?;
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).map_err(|e| e.to_string())?;
        let mut b = Vec::new();
        f.read_to_end(&mut b).map_err(|e| e.to_string())?;
        parts.push((f.name().to_string(), b));
    }
    let text = |b: &[u8]| String::from_utf8_lossy(b).to_string();
    let mut para = HashMap::new();
    let has_comments = parts.iter().any(|(n, _)| n == "word/comments.xml");
    for (name, b) in parts.iter_mut() {
        match name.as_str() {
            "word/document.xml" if !marks.changes.is_empty() => *b = renumber_changes(&paragraph_marks(&text(b))).into_bytes(),
            "word/comments.xml" => {
                let (x, ids) = comment_para_ids(&text(b));
                para = ids;
                *b = x.into_bytes();
            }
            _ => {}
        }
    }
    if has_comments && !marks.comments.is_empty() && !parts.iter().any(|(n, _)| n == "word/commentsExtended.xml") {
        for (name, b) in parts.iter_mut() {
            if name == "[Content_Types].xml" {
                let s = text(b);
                if !s.contains("/word/commentsExtended.xml") {
                    *b = s.replace("</Types>", &format!("<Override PartName=\"/word/commentsExtended.xml\" ContentType=\"{CX_TYPE}\"/></Types>")).into_bytes();
                }
            }
            if name == "word/_rels/document.xml.rels" {
                let s = text(b);
                *b = s.replace("</Relationships>", &format!("<Relationship Id=\"rIdPcCommentsEx\" Type=\"{CX_REL}\" Target=\"commentsExtended.xml\"/></Relationships>")).into_bytes();
            }
        }
        parts.push(("word/commentsExtended.xml".into(), comments_extended(marks, &para).into_bytes()));
    }
    let mut zout = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, b) in &parts {
        zout.start_file(name.as_str(), opts).map_err(|e| e.to_string())?;
        zout.write_all(b).map_err(|e| e.to_string())?;
    }
    let out = zout.finish().map_err(|e| e.to_string())?.into_inner();
    paper::write_atomic_bytes(docx, &out).map_err(|e| e.to_string())
}

// ── Reading Word's comment threads back (import) ────────

/// From a .docx: each comment id → (the id it replies to, done), from
/// `commentsExtended.xml` (Word 2013 and later). Empty when there is none.
pub fn comment_threads(docx: &[u8]) -> HashMap<String, (Option<String>, bool)> {
    use std::io::Read;
    let mut out = HashMap::new();
    let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(docx)) else { return out };
    let mut read = |name: &str| -> Option<String> {
        let mut f = z.by_name(name).ok()?;
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let (Some(comments), Some(ext)) = (read("word/comments.xml"), read("word/commentsExtended.xml")) else { return out };
    // paraId of each comment's last paragraph → comment id
    let mut by_para: HashMap<String, String> = HashMap::new();
    let mut rest = comments.as_str();
    while let Some(i) = rest.find("<w:comment ") {
        let e = rest[i..].find("</w:comment>").map(|e| i + e).unwrap_or(rest.len());
        let c = &rest[i..e];
        let id = attr_of(&c[..c.find('>').unwrap_or(c.len())], "w:id").unwrap_or_default();
        let mut last = None;
        let mut r = c;
        while let Some(k) = r.find("w14:paraId=\"") {
            let v = &r[k + 12..];
            last = v.find('"').map(|q| v[..q].to_string());
            r = v;
        }
        if let Some(p) = last {
            by_para.insert(p, id);
        }
        rest = &rest[e.min(rest.len())..];
        if e >= rest.len() && rest.is_empty() {
            break;
        }
    }
    let mut r = ext.as_str();
    while let Some(k) = r.find("<w15:commentEx ") {
        let e = r[k..].find('>').map(|e| k + e).unwrap_or(r.len());
        let tag = &r[k..e];
        if let Some(id) = attr_of(tag, "w15:paraId").and_then(|p| by_para.get(&p).cloned()) {
            let parent = attr_of(tag, "w15:paraIdParent").and_then(|p| by_para.get(&p).cloned());
            let done = attr_of(tag, "w15:done").is_some_and(|d| d == "1" || d == "true");
            out.insert(id, (parent, done));
        }
        r = &r[e..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_keep_latex_whole() {
        assert_eq!(tokens("See $a+b$ and \\cite{x,y}, Fig.~\\ref{fig:1}."), ["See", " ", "$a+b$", " ", "and", " ", "\\cite{x,y}", ",", " ", "Fig", ".", "~", "\\ref{fig:1}", "."]);
        assert_eq!(tokens("x \\begin{equation}a\\end{equation} y").len(), 5);
    }

    #[test]
    fn diff_marks_word_changes() {
        let d = diff("The method is very simple.", "The method is simple and fast.");
        let shown: Vec<(Run, String)> = d.iter().map(|(r, t)| (*r, t.concat())).collect();
        assert_eq!(shown, [(Run::Eq, "The method is ".into()), (Run::Del, "very ".into()), (Run::Eq, "simple ".into()), (Run::Ins, "and fast".into()), (Run::Eq, ".".into())]);
        // the runs give both texts back
        let side = |keep: Run| d.iter().filter(|(r, _)| *r == Run::Eq || *r == keep).map(|(_, t)| t.concat()).collect::<String>();
        assert_eq!(side(Run::Ins), "The method is simple and fast.");
        assert_eq!(side(Run::Del).split_whitespace().collect::<Vec<_>>(), ["The", "method", "is", "very", "simple", "."]);
    }

    #[test]
    fn a_paragraph_with_a_change_and_a_comment() {
        let after = "We study folded widgets. Section~\\ref{sec:2} describes it.";
        let (s, e) = anchor(after, "Section~\\ref{sec:2} describes", "widgets. ", " it.").unwrap();
        assert_eq!(&after[s..e], "Section~\\ref{sec:2} describes");
        let out = render_para(Some("We study widgets. Section~\\ref{sec:2} describes it."), after, Some(4), &[OnPara { range: Some((s, e)), ids: vec![0, 1] }]);
        assert_eq!(out, format!("We study {}folded {}widgets. {}{}Section~\\ref{{sec:2}} describes{}{} it.", mk(INS_S, 4), mk(INS_E, 4), mk(CM_S, 0), mk(CM_S, 1), mk(CM_E, 0), mk(CM_E, 1)));
        // a figure's comment goes into its caption
        let fig = "\\begin{figure}\n\\includegraphics{a}\n\\caption{The rig.}\n\\end{figure}";
        assert!(comment_block(fig, &[2]).contains(&format!("\\caption{{{}The rig.{}}}", mk(CM_S, 2), mk(CM_E, 2))));
        assert!(prose("Plain text with \\% and $x$.") && !prose("\\begin{table}x\\end{table}") && !prose("text % note"));
    }

    #[test]
    fn markers_become_spans() {
        let marks = Marks {
            changes: vec![Who { author: "daidai".into(), date: "2026-11-01T10:00:00Z".into() }],
            comments: vec![WordComment { author: "ellrin".into(), date: "2026-11-02T09:00:00Z".into(), text: "Why?".into(), done: false, parent: None }],
            unmarked: 0,
        };
        let mut doc = json!({"blocks": [{"t": "Para", "c": [
            {"t": "Str", "c": format!("A{}new", mk(INS_S, 0))}, {"t": "Space"},
            {"t": "Str", "c": format!("words{}{}{}old{}", mk(INS_E, 0), mk(CM_S, 0), mk(DEL_S, 0), mk(DEL_E, 0))},
            {"t": "Emph", "c": [{"t": "Str", "c": format!("x{}", mk(CM_E, 0))}]}
        ]}]});
        spans(&mut doc, &marks);
        let c = &doc["blocks"][0]["c"];
        assert_eq!(c[0], json!({"t": "Str", "c": "A"}));
        assert_eq!(c[1]["c"][0][1], json!(["insertion"]));
        assert_eq!(c[1]["c"][1], json!([{"t": "Str", "c": "new"}, {"t": "Space"}, {"t": "Str", "c": "words"}]));
        assert_eq!(c[2]["c"][0][1], json!(["comment-start"]));
        assert_eq!(c[2]["c"][1], json!([{"t": "Str", "c": "Why?"}]));
        assert_eq!(c[3]["c"][0][1], json!(["deletion"]));
        assert_eq!(c[4]["t"], "Emph");
        assert_eq!(c[4]["c"][1]["c"][0][1], json!(["comment-end"]));
        assert!(!has_marker(&doc));
    }

    #[test]
    fn equations_figures_and_tables_get_numbers() {
        let mut doc = json!({"blocks": [
            {"t": "Para", "c": [{"t": "Str", "c": "See"}, {"t": "Space"},
                {"t": "Link", "c": [["", [], [["reference-type", "eqref"], ["reference", "eq:a"]]], [{"t": "Str", "c": "[eq:a]"}], ["#eq:a", ""]]},
                {"t": "Space"}, {"t": "Link", "c": [["", [], [["reference-type", "ref"], ["reference", "tab:x"]]], [{"t": "Str", "c": "[tab:x]"}], ["#tab:x", ""]]}]},
            {"t": "Para", "c": [{"t": "Str", "c": "where"}, {"t": "Space"}, {"t": "Math", "c": [{"t": "DisplayMath"}, "\\begin{equation}\nF = kx\n\\label{eq:a}\n\\end{equation}"]}, {"t": "Space"}, {"t": "Str", "c": "holds."}]},
            {"t": "Para", "c": [{"t": "Math", "c": [{"t": "DisplayMath"}, "x = 1"]}]},
            {"t": "Figure", "c": [["fig:r", [], []], [null, [{"t": "Plain", "c": [{"t": "Str", "c": "Rig."}]}]], []]},
            {"t": "Table", "c": [["tab:x", [], []], [null, [{"t": "Plain", "c": [{"t": "Str", "c": "Widgets."}]}]], [], [["", [], []], []], [], [["", [], []], []]]}
        ]});
        numbers(&mut doc);
        let b = doc["blocks"].as_array().unwrap();
        assert_eq!(b[0]["c"][2]["c"][1], json!([{"t": "Str", "c": "(1)"}]));
        assert_eq!(b[0]["c"][4]["c"][1], json!([{"t": "Str", "c": "1"}]));
        assert_eq!(b[1], json!({"t": "Para", "c": [{"t": "Str", "c": "where"}]}));
        assert_eq!(b[2]["t"], "Table");
        assert_eq!(b[2]["c"][4][0][3][0][1][0][4][0]["c"][0]["c"][1], "F = kx");
        assert_eq!(b[2]["c"][4][0][3][0][1][1][4][0]["c"][0]["c"], "(1)");
        assert_eq!(b[3], json!({"t": "Para", "c": [{"t": "Str", "c": "holds."}]}));
        assert_eq!(b[4]["c"][0]["c"][1], "x = 1", "unnumbered display math stays");
        assert_eq!(b[5]["c"][1][1][0]["c"].as_array().unwrap()[0..3], *json!([{"t": "Str", "c": "Figure"}, {"t": "Space"}, {"t": "Str", "c": "1."}]).as_array().unwrap());
        assert_eq!(b[6]["c"][1][1][0]["c"][2], json!({"t": "Str", "c": "1."}));
        assert_eq!(numbered_math("\\begin{align}a&=b\\label{eq:2}\\\\c&=d\\nonumber\\end{align}"), Some(("\\begin{aligned}a&=b\\\\c&=d\\end{aligned}".into(), vec!["eq:2".into()])));
    }

    #[test]
    fn numeric_or_author_year() {
        assert!(numeric_citations("\\bibliographystyle{unsrt}\n\\bibliography{refs}"));
        assert!(numeric_citations("\\bibliographystyle{IEEEtran}"));
        assert!(numeric_citations("\\bibliographystyle{elsarticle-num}"));
        assert!(!numeric_citations("\\bibliographystyle{apalike}"));
        assert!(!numeric_citations("\\bibliographystyle{elsarticle-harv}"));
        assert!(numeric_citations("\\usepackage[numbers]{natbib}\n\\bibliographystyle{plainnat}"));
        assert!(numeric_citations("\\usepackage[style=numeric]{biblatex}"));
        assert!(!numeric_citations("\\usepackage[style=authoryear]{biblatex}"));
    }

    #[test]
    fn paragraph_marks_and_ids() {
        let doc = "<w:body><w:p><w:pPr><w:pStyle w:val=\"BodyText\" /></w:pPr><w:del w:id=\"1\" w:author=\"daidai\" w:date=\"D\"><w:r><w:delText>Gone.</w:delText></w:r></w:del></w:p><w:p><w:ins w:id=\"1\" w:author=\"a\" w:date=\"D\"><w:r><w:t>x</w:t></w:r></w:ins><w:r><w:t>kept</w:t></w:r></w:p></w:body>";
        let out = renumber_changes(&paragraph_marks(doc));
        assert!(out.contains("<w:pPr><w:pStyle w:val=\"BodyText\" /><w:rPr><w:del w:id=\"1000\" w:author=\"daidai\" w:date=\"D\"/></w:rPr></w:pPr><w:del w:id=\"1001\""), "{out}");
        assert!(out.contains("<w:p><w:ins w:id=\"1002\""), "a paragraph with kept text keeps its mark: {out}");
    }

    #[test]
    fn comment_threads_round_trip() {
        let marks = Marks {
            changes: vec![],
            comments: vec![
                WordComment { author: "a".into(), date: "d".into(), text: "Q".into(), done: true, parent: None },
                WordComment { author: "b".into(), date: "d".into(), text: "A".into(), done: true, parent: Some(0) },
            ],
            unmarked: 0,
        };
        let xml = "<?xml version=\"1.0\"?><w:comments xmlns:w=\"x\"><w:comment w:id=\"0\" w:author=\"a\"><w:p><w:r><w:t>Q</w:t></w:r></w:p></w:comment><w:comment w:id=\"1\" w:author=\"b\"><w:p><w:r><w:t>A</w:t></w:r></w:p></w:comment></w:comments>";
        let (c, ids) = comment_para_ids(xml);
        assert!(c.contains("<w:p w14:paraId=\"10000000\" w14:textId=\"77777777\">") && c.contains("mc:Ignorable=\"w14\""), "{c}");
        let ext = comments_extended(&marks, &ids);
        assert!(ext.contains("<w15:commentEx w15:paraId=\"10000001\" w15:paraIdParent=\"10000000\" w15:done=\"1\"/>"), "{ext}");
        // read back from a zip
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("word/comments.xml", o).unwrap();
        z.write_all(c.as_bytes()).unwrap();
        z.start_file("word/commentsExtended.xml", o).unwrap();
        z.write_all(ext.as_bytes()).unwrap();
        let bytes = z.finish().unwrap().into_inner();
        let t = comment_threads(&bytes);
        assert_eq!(t.get("0"), Some(&(None, true)));
        assert_eq!(t.get("1"), Some(&(Some("0".into()), true)));
    }

    #[test]
    fn main_tex_gets_the_marked_sections() {
        let mut f = HashMap::new();
        f.insert("01-intro.tex".to_string(), "\\section{Intro}\n\nMarked.".to_string());
        let m = main_with("\\begin{document}\n\\input{sections/01-intro}\n\\input{sections/02-rest}\n\\end{document}\n", &f);
        assert_eq!(m, "\\begin{document}\n\n\\section{Intro}\n\nMarked.\n\n\\input{sections/02-rest}\n\\end{document}\n");
    }
}
