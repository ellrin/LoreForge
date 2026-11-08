//! One pass over the document body: words, punctuation and math segments in
//! reading order (with their context: body, caption, table, heading,
//! abstract), sentences, labels, references, captions, floats and
//! `\includegraphics`. Everything later (symbols, abbreviations, checks,
//! auto-extract) works on this.

use crate::math::{self, Atom, Macro};
use crate::source::{Document, Loc};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Area {
    Body,
    Abstract,
    Heading,
    Caption,
    Table,
}

impl Area {
    pub fn as_str(self) -> &'static str {
        match self {
            Area::Body => "body",
            Area::Abstract => "abstract",
            Area::Heading => "heading",
            Area::Caption => "caption",
            Area::Table => "table",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ctx {
    pub area: Area,
    pub float: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Word(String),
    Math(usize),
    Punct(char),
    Ref(usize),
    Break,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: ItemKind,
    pub start: usize,
    pub end: usize,
    pub ctx: Ctx,
    pub sentence: usize,
}

#[derive(Debug, Clone)]
pub struct MathSeg {
    pub start: usize,
    pub end: usize,
    /// Source of the math content (without delimiters).
    pub src: String,
    pub display: bool,
    pub env: Option<String>,
    pub ctx: Ctx,
    pub atoms: Vec<Atom>,
    pub labels: Vec<String>,
    pub sentence: usize,
    pub item: usize,
}

#[derive(Debug, Clone)]
pub struct Label {
    pub key: String,
    /// fig | tab | eq | sec | other
    pub kind: String,
    pub off: usize,
    pub ctx: Ctx,
}

#[derive(Debug, Clone)]
pub struct RefUse {
    pub key: String,
    pub cmd: String,
    pub off: usize,
    pub ctx: Ctx,
    pub item: usize,
}

#[derive(Debug, Clone)]
pub struct Caption {
    pub float: Option<usize>,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct Float {
    pub env: String,
    /// figure | table
    pub kind: String,
    pub start: usize,
    pub end: usize,
    pub labels: Vec<String>,
    pub graphics: Vec<String>,
    pub caption: Option<usize>,
    /// Offset of the first `\begin{tabular}` (caption-above vs below).
    pub tabular_at: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Heading {
    pub level: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct Sentence {
    pub start: usize,
    pub end: usize,
    pub file: usize,
    /// 1-based in its file.
    pub n: u32,
    pub items: std::ops::Range<usize>,
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Single letters treated as indices (`_i`, `$i$th`).
    pub index_letters: Vec<String>,
    /// Extra macros (from the registry) expanded in math.
    pub extra_macros: BTreeMap<String, Macro>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { index_letters: math::DEFAULT_INDEX.iter().map(|s| s.to_string()).collect(), extra_macros: BTreeMap::new() }
    }
}

#[derive(Debug, Clone)]
pub struct Scan {
    pub doc: Document,
    pub items: Vec<Item>,
    pub math: Vec<MathSeg>,
    pub labels: Vec<Label>,
    pub refs: Vec<RefUse>,
    pub captions: Vec<Caption>,
    pub floats: Vec<Float>,
    pub headings: Vec<Heading>,
    pub sentences: Vec<Sentence>,
    /// Macros expanded in math: the document's own plus the registry's.
    pub macros: BTreeMap<String, Macro>,
    /// Names the document itself defines (`\newcommand`, `\def`, `\DeclareMathOperator`).
    pub doc_macros: std::collections::BTreeSet<String>,
    pub opts: ScanOptions,
}

const MATH_ENVS: &[&str] = &["equation", "align", "gather", "multline", "eqnarray", "displaymath", "flalign", "alignat", "math", "dmath"];
const SKIP_ENVS: &[&str] = &["thebibliography", "verbatim", "lstlisting", "comment", "minted", "IEEEkeywords", "keywords", "tikzpicture", "filecontents"];
const REF_CMDS: &[&str] = &["ref", "eqref", "autoref", "cref", "Cref", "pageref", "vref", "subref", "nameref", "labelcref", "cpageref"];
const CITE_CMDS: &[&str] = &["cite", "citep", "citet", "citealp", "citealt", "citeauthor", "citeyear", "nocite", "parencite", "textcite", "autocite", "footcite", "Cite"];
const FRONT_CMDS: &[&str] = &["title", "author", "thanks", "date", "affil", "affiliation", "address", "email", "IEEEauthorblockN", "IEEEauthorblockA", "institute", "keywords", "markboth", "IEEEspecialpapernotice", "IEEEpubid"];
const HEADINGS: &[&str] = &["part", "chapter", "section", "subsection", "subsubsection", "paragraph", "subparagraph"];

/// Commands whose braced arguments are skipped: (name, number of args).
fn skip_args(name: &str) -> Option<usize> {
    Some(match name {
        "url" | "bibliography" | "bibliographystyle" | "usepackage" | "RequirePackage" | "documentclass" | "graphicspath" | "hypersetup"
        | "vspace" | "hspace" | "enlargethispage" | "color" | "pagestyle" | "thispagestyle" | "hyphenation" | "captionsetup" | "scalebox"
        | "raisebox" | "bibitem" | "pagenumbering" | "addvspace" | "linespread" | "setstretch" | "includepdf" | "DeclareGraphicsExtensions"
        | "href" | "textcolor" | "label" | "footnotemark" => 1,
        "setcounter" | "setlength" | "addtolength" | "rule" | "resizebox" | "multicolumn" | "multirow" | "fontsize" | "DeclareMathOperator" | "providecommand" => 2,
        "definecolor" => 3,
        _ => return None,
    })
}

struct Frame {
    depth: usize,
    kind: FrameKind,
}

enum FrameKind {
    Caption(usize),
    Heading(usize),
}

struct Scanner<'a> {
    t: &'a str,
    i: usize,
    end: usize,
    items: Vec<Item>,
    math: Vec<MathSeg>,
    labels: Vec<Label>,
    refs: Vec<RefUse>,
    captions: Vec<Caption>,
    floats: Vec<Float>,
    headings: Vec<Heading>,
    envs: Vec<String>,
    open_floats: Vec<usize>,
    frames: Vec<Frame>,
    depth: usize,
    word: String,
    word_start: usize,
    macros: BTreeMap<String, Macro>,
    opts: &'a ScanOptions,
}

/// Commands the document defines (name without backslash), from the whole text.
pub fn document_macros(text: &str) -> BTreeMap<String, Macro> {
    let mut out = BTreeMap::new();
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(j) = text[i..].find('\\') {
        let s = i + j;
        let name: String = text[s + 1..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let mut k = s + 1 + name.len();
        i = k.max(s + 1);
        match name.as_str() {
            "newcommand" | "renewcommand" | "providecommand" | "DeclareMathOperator" | "DeclareRobustCommand" => {
                if text[k..].starts_with('*') {
                    k += 1;
                }
                k = skip_ws(text, k);
                // \newcommand{\x} or \newcommand\x
                let (cmd, after) = if b.get(k) == Some(&b'{') {
                    match group(text, k) {
                        Some((inner, e)) => (inner.trim().to_string(), e),
                        None => continue,
                    }
                } else if b.get(k) == Some(&b'\\') {
                    let n: String = text[k + 1..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                    (format!("\\{n}"), k + 1 + n.len())
                } else {
                    continue;
                };
                let Some(cmd) = cmd.strip_prefix('\\').map(|c| c.to_string()) else { continue };
                let mut k = skip_ws(text, after);
                let mut nargs = 0u8;
                if b.get(k) == Some(&b'[')
                    && let Some(e) = text[k..].find(']') {
                        nargs = text[k + 1..k + e].trim().parse().unwrap_or(0);
                        k = skip_ws(text, k + e + 1);
                        // optional default value
                        if b.get(k) == Some(&b'[')
                            && let Some(e2) = text[k..].find(']') {
                                k = skip_ws(text, k + e2 + 1);
                            }
                    }
                if let Some((body, e)) = group(text, k) {
                    let body = if name == "DeclareMathOperator" { format!("\\operatorname{{{body}}}") } else { body.to_string() };
                    out.insert(cmd, Macro { nargs, body });
                    i = e;
                }
            }
            "def" => {
                let n: String = text.get(k + 1..).unwrap_or("").chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                if b.get(k) != Some(&b'\\') || n.is_empty() {
                    continue;
                }
                let k2 = k + 1 + n.len();
                // \def\x{...} only (no parameter text)
                if b.get(k2) == Some(&b'{')
                    && let Some((body, e)) = group(text, k2) {
                        out.insert(n, Macro { nargs: 0, body: body.to_string() });
                        i = e;
                    }
            }
            _ => {}
        }
    }
    out
}

fn skip_ws(t: &str, mut k: usize) -> usize {
    while t[k..].starts_with(|c: char| c.is_whitespace()) {
        k += t[k..].chars().next().unwrap().len_utf8();
    }
    k
}

/// `{...}` at `k`: (inside, offset after the closing brace).
pub fn group(t: &str, k: usize) -> Option<(&str, usize)> {
    if !t[k..].starts_with('{') {
        return None;
    }
    let mut d = 0usize;
    let b = t.as_bytes();
    let mut j = k;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 1,
            b'{' => d += 1,
            b'}' => {
                d -= 1;
                if d == 0 {
                    return Some((&t[k + 1..j], j + 1));
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

impl Scan {
    pub fn new(doc: Document, opts: ScanOptions) -> Scan {
        let mut macros = document_macros(&doc.text);
        let doc_macros = macros.keys().cloned().collect();
        // registry macros win only where the document has none
        for (k, v) in &opts.extra_macros {
            macros.entry(k.clone()).or_insert_with(|| v.clone());
        }
        let text = doc.text.clone();
        let mut s = Scanner {
            t: &text,
            i: doc.body.start,
            end: doc.body.end,
            items: Vec::new(),
            math: Vec::new(),
            labels: Vec::new(),
            refs: Vec::new(),
            captions: Vec::new(),
            floats: Vec::new(),
            headings: Vec::new(),
            envs: Vec::new(),
            open_floats: Vec::new(),
            frames: Vec::new(),
            depth: 0,
            word: String::new(),
            word_start: 0,
            macros: macros.clone(),
            opts: &opts,
        };
        s.run();
        let Scanner { mut items, mut math, labels, refs, captions, floats, headings, .. } = s;
        let sentences = split_sentences(&doc, &mut items);
        for m in math.iter_mut() {
            m.sentence = items[m.item].sentence;
        }
        Scan { doc, items, math, labels, refs, captions, floats, headings, sentences, macros, doc_macros, opts: opts.clone() }
    }

    pub fn from_str(name: &str, text: &str) -> Scan {
        Scan::new(Document::from_str(name, text), ScanOptions::default())
    }

    /// Location of an offset, with the context of what is there.
    pub fn loc(&self, off: usize, context: &str) -> Loc {
        let (f, line, col) = self.doc.position(off);
        let si = self.sentence_at(off);
        let sentence = si.map(|s| self.sentences[s].n).unwrap_or(0);
        let para = self.doc.para_of(f, line);
        // sentences of this file and paragraph up to this one
        let para_sentence = si
            .map(|si| {
                self.sentences[..=si]
                    .iter()
                    .rev()
                    .take_while(|s| {
                        let (sf, sl, _) = self.doc.position(s.start);
                        sf == f && self.doc.para_of(sf, sl) == para
                    })
                    .filter(|s| self.items[s.items.start].ctx.area != Area::Heading)
                    .count() as u32
            })
            .unwrap_or(0)
            .max(1);
        Loc {
            file: self.doc.files.get(f).cloned().unwrap_or_default(),
            line,
            col,
            para,
            sentence,
            para_sentence,
            context: context.to_string(),
            ord: off,
            snippet: self.doc.snippet(off),
        }
    }

    pub fn sentence_at(&self, off: usize) -> Option<usize> {
        if self.sentences.is_empty() {
            return None;
        }
        let i = match self.sentences.binary_search_by(|s| s.start.cmp(&off)) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        Some(i)
    }

    /// Plain text of a sentence (words and math source), for contexts.
    pub fn sentence_text(&self, si: usize) -> String {
        let Some(s) = self.sentences.get(si) else { return String::new() };
        let mut out = String::new();
        for it in &self.items[s.items.clone()] {
            match &it.kind {
                ItemKind::Word(w) => {
                    if !out.is_empty() && !out.ends_with('(') {
                        out.push(' ');
                    }
                    out.push_str(w);
                }
                ItemKind::Math(m) => {
                    if !out.is_empty() && !out.ends_with('(') {
                        out.push(' ');
                    }
                    out.push('$');
                    out.push_str(self.math[*m].src.trim());
                    out.push('$');
                }
                ItemKind::Punct(c) => {
                    if *c == '(' {
                        out.push(' ');
                    }
                    out.push(*c);
                }
                ItemKind::Ref(r) => {
                    out.push_str(&format!("\\ref{{{}}}", self.refs[*r].key));
                }
                ItemKind::Break => {}
            }
        }
        out.trim().to_string()
    }

    pub fn context_of(&self, ctx: Ctx, display: bool) -> &'static str {
        if display && matches!(ctx.area, Area::Body | Area::Abstract) { "equation" } else { ctx.area.as_str() }
    }
}

impl Scanner<'_> {
    fn ctx(&self) -> Ctx {
        let mut area = Area::Body;
        if self.envs.iter().any(|e| e == "abstract") {
            area = Area::Abstract;
        }
        if self.envs.iter().any(|e| e.starts_with("tabular") || e == "array" || e == "longtable" || e == "tabu") {
            area = Area::Table;
        }
        for f in &self.frames {
            match f.kind {
                FrameKind::Caption(_) => area = Area::Caption,
                FrameKind::Heading(_) => {
                    if area != Area::Caption {
                        area = Area::Heading
                    }
                }
            }
        }
        Ctx { area, float: self.open_floats.last().copied() }
    }

    fn push(&mut self, kind: ItemKind, start: usize, end: usize) {
        let ctx = self.ctx();
        self.items.push(Item { kind, start, end, ctx, sentence: 0 });
    }

    fn flush(&mut self) {
        if !self.word.is_empty() {
            let w = std::mem::take(&mut self.word);
            let start = self.word_start;
            // a trailing hyphen / apostrophe is not part of the word
            let w = w.trim_end_matches(['-', '\'']).to_string();
            if !w.is_empty() {
                let end = start + w.len();
                self.push(ItemKind::Word(w), start, end);
            }
        }
    }

    fn brk(&mut self, at: usize) {
        self.flush();
        if !matches!(self.items.last().map(|i| &i.kind), Some(ItemKind::Break) | None) {
            self.push(ItemKind::Break, at, at);
        }
    }

    fn peek(&self) -> Option<char> {
        self.t[self.i..self.end].chars().next()
    }

    fn run(&mut self) {
        while self.i < self.end {
            let c = self.peek().unwrap();
            match c {
                '\\' => self.command(),
                '$' => self.dollar(),
                '{' => {
                    self.flush();
                    self.depth += 1;
                    self.i += 1;
                }
                '}' => {
                    self.flush();
                    self.close_brace();
                    self.i += 1;
                }
                '\n' => {
                    self.flush();
                    let rest = &self.t[self.i + 1..self.end];
                    let ws = rest.len() - rest.trim_start_matches([' ', '\t', '\r']).len();
                    if rest[ws..].starts_with('\n') {
                        self.brk(self.i);
                    }
                    self.i += 1;
                }
                c if c.is_alphanumeric() => {
                    if self.word.is_empty() {
                        self.word_start = self.i;
                    }
                    self.word.push(c);
                    self.i += c.len_utf8();
                }
                '-' | '\'' if !self.word.is_empty() && self.t[self.i + 1..self.end].starts_with(|c: char| c.is_alphanumeric()) => {
                    self.word.push(c);
                    self.i += 1;
                }
                '.' | ',' | ';' | ':' | '?' | '!' | '(' | ')' | '[' | ']' => {
                    self.flush();
                    self.push(ItemKind::Punct(c), self.i, self.i + 1);
                    self.i += 1;
                }
                _ => {
                    self.flush();
                    self.i += c.len_utf8();
                }
            }
        }
        self.flush();
    }

    fn close_brace(&mut self) {
        if let Some(f) = self.frames.last()
            && f.depth == self.depth {
                let f = self.frames.pop().unwrap();
                match f.kind {
                    FrameKind::Caption(c) => self.captions[c].end = self.i,
                    FrameKind::Heading(h) => self.headings[h].end = self.i,
                }
                self.brk(self.i);
            }
        self.depth = self.depth.saturating_sub(1);
    }

    /// Raw `{...}` argument at the cursor (after spaces); cursor moves past it.
    fn raw_arg(&mut self) -> Option<(usize, String)> {
        let k = skip_ws(self.t, self.i);
        let (inner, e) = group(&self.t[..self.end], k)?;
        self.i = e;
        Some((k + 1, inner.to_string()))
    }

    fn skip_optional(&mut self) {
        let k = skip_ws(self.t, self.i);
        if self.t[k..self.end].starts_with('[') {
            let mut d = 0;
            for (j, c) in self.t[k..self.end].char_indices() {
                match c {
                    '[' => d += 1,
                    ']' => {
                        d -= 1;
                        if d == 0 {
                            self.i = k + j + 1;
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn command(&mut self) {
        let s = self.i;
        let rest = &self.t[s + 1..self.end];
        let n: usize = rest.chars().take_while(|c| c.is_ascii_alphabetic()).count();
        if n == 0 {
            let Some(c) = rest.chars().next() else {
                self.i += 1;
                return;
            };
            self.i = s + 1 + c.len_utf8();
            match c {
                '(' => self.math_until(s, self.i, "\\)", false, None),
                '[' => self.math_until(s, self.i, "\\]", true, None),
                '\'' | '`' | '"' | '^' | '~' | '=' | '.' => {
                    // accent on the next letter: stays inside the word
                    let k = self.i;
                    let (letter, e) = if self.t[k..].starts_with('{') {
                        match group(self.t, k) {
                            Some((inner, e)) => (inner.chars().find(|c| c.is_alphabetic()), e),
                            None => (None, k),
                        }
                    } else {
                        (self.t[k..].chars().next().filter(|c| c.is_alphabetic()), k + self.t[k..].chars().next().map(|c| c.len_utf8()).unwrap_or(0))
                    };
                    if let Some(l) = letter {
                        if self.word.is_empty() {
                            self.word_start = s;
                        }
                        self.word.push(l);
                        self.i = e;
                    }
                }
                '-' => {}
                '\\' => {
                    self.skip_optional();
                    if self.ctx().area == Area::Table {
                        self.brk(s);
                    } else {
                        self.flush();
                    }
                }
                _ => self.flush(),
            }
            return;
        }
        let mut name = rest[..n].to_string();
        self.i = s + 1 + n;
        if self.t[self.i..self.end].starts_with('*') {
            name.push('*');
            self.i += 1;
        }
        let base = name.trim_end_matches('*');
        // commands inside a word (\-) never reach here; any command ends the word
        self.flush();
        match base {
            "begin" => self.begin(s),
            "end" => self.end_env(s),
            "label" => {
                if let Some((at, key)) = self.raw_arg() {
                    let key = key.trim().to_string();
                    let kind = self.label_kind(&key);
                    let ctx = self.ctx();
                    if let Some(f) = ctx.float {
                        self.floats[f].labels.push(key.clone());
                    }
                    self.labels.push(Label { key, kind, off: at, ctx });
                }
            }
            b if REF_CMDS.contains(&b) => {
                if let Some((at, keys)) = self.raw_arg() {
                    let ctx = self.ctx();
                    let item = self.items.len();
                    let first = self.refs.len();
                    for k in keys.split(',') {
                        let k = k.trim();
                        if !k.is_empty() {
                            self.refs.push(RefUse { key: k.to_string(), cmd: base.to_string(), off: at, ctx, item });
                        }
                    }
                    self.push(ItemKind::Ref(first), s, self.i);
                }
            }
            b if CITE_CMDS.contains(&b) => {
                self.skip_optional();
                self.skip_optional();
                self.raw_arg();
            }
            "includegraphics" => {
                self.skip_optional();
                if let Some((_, f)) = self.raw_arg()
                    && let Some(fl) = self.open_floats.last() {
                        self.floats[*fl].graphics.push(f.trim().to_string());
                    }
            }
            "caption" | "subcaption" => {
                self.skip_optional();
                let k = skip_ws(self.t, self.i);
                if self.t[k..self.end].starts_with('{') {
                    self.brk(s);
                    let float = self.open_floats.last().copied();
                    let c = self.captions.len();
                    self.captions.push(Caption { float, start: k + 1, end: k + 1 });
                    if let Some(f) = float
                        && self.floats[f].caption.is_none() {
                            self.floats[f].caption = Some(c);
                        }
                    self.i = k + 1;
                    self.depth += 1;
                    self.frames.push(Frame { depth: self.depth, kind: FrameKind::Caption(c) });
                }
            }
            b if HEADINGS.contains(&b) => {
                self.skip_optional();
                let k = skip_ws(self.t, self.i);
                if self.t[k..self.end].starts_with('{') {
                    self.brk(s);
                    let h = self.headings.len();
                    self.headings.push(Heading { level: b.to_string(), start: k + 1, end: k + 1 });
                    self.i = k + 1;
                    self.depth += 1;
                    self.frames.push(Frame { depth: self.depth, kind: FrameKind::Heading(h) });
                }
            }
            b if FRONT_CMDS.contains(&b) => {
                self.skip_optional();
                let args = if b == "markboth" { 2 } else { 1 };
                for _ in 0..args {
                    self.raw_arg();
                }
            }
            "IEEEPARstart" => {
                let a = self.raw_arg().map(|x| x.1).unwrap_or_default();
                let b2 = self.raw_arg().map(|x| x.1).unwrap_or_default();
                self.word_start = s;
                self.word = format!("{a}{b2}");
            }
            "newcommand" | "renewcommand" | "DeclareRobustCommand" | "newenvironment" | "renewenvironment" => {
                // definitions inside the body: skip name, [n], [default], body(s)
                let k = skip_ws(self.t, self.i);
                if self.t[k..].starts_with('{') {
                    self.raw_arg();
                } else if self.t[k..].starts_with('\\') {
                    let n: usize = self.t[k + 1..].chars().take_while(|c| c.is_ascii_alphabetic()).count();
                    self.i = k + 1 + n;
                }
                self.skip_optional();
                self.skip_optional();
                self.raw_arg();
                if base.contains("environment") {
                    self.raw_arg();
                }
            }
            "def" => {
                let k = self.i;
                if self.t[k..].starts_with('\\') {
                    let n: usize = self.t[k + 1..].chars().take_while(|c| c.is_ascii_alphabetic()).count();
                    self.i = k + 1 + n;
                    if let Some(o) = self.t[self.i..self.end].find('{') {
                        self.i += o;
                        self.raw_arg();
                    }
                }
            }
            "item" => {
                self.brk(s);
                self.skip_optional();
            }
            "par" | "newline" | "linebreak" | "noindent" | "medskip" | "bigskip" | "smallskip" | "clearpage" | "newpage" | "maketitle" => {
                if base != "noindent" {
                    self.brk(s);
                }
            }
            "cmidrule" => {
                let k = skip_ws(self.t, self.i);
                if self.t[k..].starts_with('(')
                    && let Some(e) = self.t[k..].find(')') {
                        self.i = k + e + 1;
                    }
                self.raw_arg();
            }
            "ensuremath" => {
                if let Some((at, inner)) = self.raw_arg() {
                    self.push_math(s, at, &inner, self.i, false, None);
                }
            }
            b => {
                if let Some(n) = skip_args(b) {
                    self.skip_optional();
                    for _ in 0..n {
                        self.raw_arg();
                    }
                }
                // anything else: drop the name, its braces are processed as text
            }
        }
    }

    fn label_kind(&self, key: &str) -> String {
        let ctx = self.ctx();
        if let Some(f) = ctx.float {
            return if self.floats[f].kind == "table" { "tab".into() } else { "fig".into() };
        }
        if ctx.area == Area::Heading || self.items.last().is_some_and(|it| it.ctx.area == Area::Heading) || matches!(self.items.last().map(|i| &i.kind), Some(ItemKind::Break)) && self.headings.last().is_some_and(|h| self.i.saturating_sub(h.end) < 40) {
            return "sec".into();
        }
        let p = key.split(':').next().unwrap_or("").to_ascii_lowercase();
        match p.as_str() {
            "fig" | "figure" => "fig",
            "tab" | "table" => "tab",
            "eq" | "eqn" | "equation" => "eq",
            "sec" | "section" | "ssec" | "subsec" | "chap" => "sec",
            "alg" | "algorithm" => "alg",
            _ => "other",
        }
        .into()
    }

    fn begin(&mut self, s: usize) {
        let Some((_, env)) = self.raw_arg() else { return };
        let env = env.trim().to_string();
        let base = env.trim_end_matches('*');
        if MATH_ENVS.contains(&base) {
            let inner = self.i;
            let close = format!("\\end{{{env}}}");
            self.math_until(s, inner, &close, true, Some(env));
            return;
        }
        if SKIP_ENVS.contains(&base) {
            let close = format!("\\end{{{env}}}");
            self.brk(s);
            self.i = self.t[self.i..self.end].find(&close).map(|k| self.i + k + close.len()).unwrap_or(self.end);
            return;
        }
        self.brk(s);
        match base {
            "figure" | "table" | "wrapfigure" | "wraptable" | "SCfigure" | "sidewaysfigure" | "sidewaystable" => {
                self.skip_optional();
                if base.starts_with("wrap") {
                    self.raw_arg();
                }
                let kind = if base.contains("table") { "table" } else { "figure" };
                let f = self.floats.len();
                self.floats.push(Float { env: env.clone(), kind: kind.into(), start: s, end: s, labels: vec![], graphics: vec![], caption: None, tabular_at: None });
                self.open_floats.push(f);
            }
            b if b.starts_with("tabular") || b == "array" || b == "longtable" => {
                if let Some(f) = self.open_floats.last()
                    && self.floats[*f].tabular_at.is_none() {
                        self.floats[*f].tabular_at = Some(s);
                    }
                self.skip_optional();
                if b == "tabularx" || b == "tabular*" || env == "tabular*" {
                    self.raw_arg();
                }
                self.raw_arg();
            }
            _ => self.skip_optional(),
        }
        self.envs.push(env);
    }

    fn end_env(&mut self, s: usize) {
        let Some((_, env)) = self.raw_arg() else { return };
        let env = env.trim();
        self.brk(s);
        if let Some(p) = self.envs.iter().rposition(|e| e == env) {
            self.envs.truncate(p);
        }
        let base = env.trim_end_matches('*');
        if matches!(base, "figure" | "table" | "wrapfigure" | "wraptable" | "SCfigure" | "sidewaysfigure" | "sidewaystable")
            && let Some(f) = self.open_floats.pop() {
                self.floats[f].end = self.i;
            }
    }

    fn dollar(&mut self) {
        let s = self.i;
        if self.t[s..].starts_with("$$") {
            self.math_until(s, s + 2, "$$", true, None);
        } else {
            self.math_until(s, s + 1, "$", false, None);
        }
    }

    /// Math from `inner` up to the unescaped `close`.
    fn math_until(&mut self, s: usize, inner: usize, close: &str, display: bool, env: Option<String>) {
        self.flush();
        let mut j = inner;
        let end_inner;
        loop {
            match self.t[j..self.end].find(close) {
                Some(k) => {
                    let at = j + k;
                    // escaped \$ ?
                    let bs = self.t[..at].chars().rev().take_while(|c| *c == '\\').count();
                    if close == "$" && bs % 2 == 1 {
                        j = at + 1;
                        continue;
                    }
                    end_inner = at;
                    break;
                }
                None => {
                    end_inner = self.end;
                    break;
                }
            }
        }
        let src = self.t[inner..end_inner].to_string();
        let end = (end_inner + close.len()).min(self.end);
        self.i = end;
        self.push_math(s, inner, &src, end, display, env);
    }

    fn push_math(&mut self, s: usize, inner: usize, src: &str, end: usize, display: bool, env: Option<String>) {
        let mut labels = Vec::new();
        let mut from = 0;
        while let Some(k) = src[from..].find("\\label{") {
            let at = from + k + 7;
            if let Some(e) = src[at..].find('}') {
                labels.push(src[at..at + e].trim().to_string());
            }
            from = at;
        }
        let ctx = self.ctx();
        for l in &labels {
            let off = inner + src.find(l.as_str()).unwrap_or(0);
            if let Some(f) = ctx.float {
                self.floats[f].labels.push(l.clone());
            }
            self.labels.push(Label { key: l.clone(), kind: "eq".into(), off, ctx });
        }
        let toks = math::expand(math::tokenize(src, inner), &self.macros);
        let atoms = math::parse(&toks, &math::Options { index_letters: &self.opts.index_letters });
        let idx = self.math.len();
        let item = self.items.len();
        self.math.push(MathSeg { start: s, end, src: src.to_string(), display, env, ctx, atoms, labels, sentence: 0, item });
        if display {
            self.brk(s);
        }
        self.push(ItemKind::Math(idx), s, end);
        // keep the item index right after a break was inserted
        self.math[idx].item = self.items.len() - 1;
        if display {
            // a display equation does not end the sentence: "… is\n\[x\]\nwhere …"
        }
    }
}

const ABBREV_WORDS: &[&str] = &[
    "Fig", "Figs", "fig", "figs", "Eq", "Eqs", "eq", "eqs", "Sec", "Secs", "sec", "Tab", "tab", "e", "g", "i", "al", "vs", "cf", "approx", "resp",
    "Ref", "Refs", "ref", "Dr", "Prof", "No", "Nos", "no", "pp", "vol", "Vol", "Ch", "Alg", "Def", "Thm", "Lem", "Eqn", "Eqns", "Mr", "Ms", "St",
];

/// Assign sentence numbers to items and return the sentence list.
fn split_sentences(doc: &Document, items: &mut [Item]) -> Vec<Sentence> {
    let mut out: Vec<Sentence> = Vec::new();
    let mut per_file: BTreeMap<usize, u32> = BTreeMap::new();
    let mut start: Option<usize> = None;
    let mut close = |out: &mut Vec<Sentence>, from: usize, to: usize, items: &mut [Item]| {
        if from >= to {
            return;
        }
        let f = doc.file_of(items[from].start);
        let n = per_file.entry(f).or_insert(0);
        *n += 1;
        let si = out.len();
        for it in &mut items[from..to] {
            it.sentence = si;
        }
        out.push(Sentence { start: items[from].start, end: items[to - 1].end, file: f, n: *n, items: from..to });
    };
    let len = items.len();
    let mut i = 0;
    while i < len {
        let k = items[i].kind.clone();
        match k {
            ItemKind::Break => {
                if let Some(s) = start.take() {
                    close(&mut out, s, i, items);
                }
                // the break belongs to the previous sentence for lookups
                items[i].sentence = out.len().saturating_sub(1);
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
                // file change ends a sentence
                if let Some(s) = start
                    && i > s && doc.file_of(items[i].start) != doc.file_of(items[i - 1].start) {
                        close(&mut out, s, i, items);
                        start = Some(i);
                    }
                if let ItemKind::Punct(c) = &k
                    && matches!(c, '.' | '?' | '!') && ends_sentence(items, i)
                        && let Some(s) = start.take() {
                            close(&mut out, s, i + 1, items);
                        }
            }
        }
        i += 1;
    }
    if let Some(s) = start {
        close(&mut out, s, len, items);
    }
    out
}

fn ends_sentence(items: &[Item], i: usize) -> bool {
    if let Some(ItemKind::Word(w)) = items.get(i.wrapping_sub(1)).map(|x| &x.kind)
        && ABBREV_WORDS.contains(&w.as_str()) && items[i - 1].end == items[i].start {
            return false;
        }
        // decimal number "0.5" never reaches here as one word; "0" + "." + "5" → no break
    let mut j = i + 1;
    while let Some(ItemKind::Punct('(' | ')')) = items.get(j).map(|x| &x.kind) {
        j += 1;
    }
    match items.get(j).map(|x| &x.kind) {
        None | Some(ItemKind::Break) | Some(ItemKind::Math(_)) | Some(ItemKind::Ref(_)) => true,
        Some(ItemKind::Word(w)) => {
            let c = w.chars().next().unwrap_or('a');
            (c.is_uppercase() || !c.is_ascii()) && items[j].start > items[i].end
        }
        Some(ItemKind::Punct(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\\documentclass{article}\n\\newcommand{\\vx}{\\boldsymbol{x}}\n\\begin{document}\n\\begin{abstract}\nWe study widgets.\n\\end{abstract}\n\\section{Intro}\\label{sec:intro}\nLet $\\vx$ denote an input. See Fig.~\\ref{fig:a} and \\eqref{eq:one}.\n\\begin{figure}[t]\n\\includegraphics[width=1in]{plot}\n\\caption{A plot of $y$.}\n\\label{fig:a}\n\\end{figure}\n\\begin{equation}\n\\label{eq:one}\ny = 2\\vx.\n\\end{equation}\nDone.\n\\end{document}\n";

    #[test]
    fn finds_math_labels_refs_floats_and_sentences() {
        let s = Scan::from_str("main.tex", SAMPLE);
        assert_eq!(s.math.len(), 3);
        assert!(s.math[2].display);
        assert_eq!(s.math[2].labels, vec!["eq:one"]);
        let kinds: Vec<(String, String)> = s.labels.iter().map(|l| (l.key.clone(), l.kind.clone())).collect();
        assert!(kinds.contains(&("sec:intro".into(), "sec".into())));
        assert!(kinds.contains(&("fig:a".into(), "fig".into())));
        assert!(kinds.contains(&("eq:one".into(), "eq".into())));
        assert_eq!(s.refs.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), vec!["fig:a", "eq:one"]);
        assert_eq!(s.floats[0].graphics, vec!["plot"]);
        assert_eq!(s.math[1].ctx.area, Area::Caption);
        // "We study widgets." | "Intro" | "Let x denote an input." | "See Fig. 1 and (1)." | caption ...
        let texts: Vec<String> = (0..s.sentences.len()).map(|i| s.sentence_text(i)).collect();
        assert!(texts.iter().any(|t| t.starts_with("Let $\\vx$ denote an input.")), "{texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("See Fig.")), "{texts:?}");
        // macro expanded in math
        let mut v = Vec::new();
        math::collect_syms(&s.math[0].atoms, &mut v);
        assert_eq!(v[0].key(), "\\boldsymbol{x}");
    }

    #[test]
    fn document_macros_are_read() {
        let m = document_macros("\\newcommand{\\a}[1]{x_{#1}}\\DeclareMathOperator{\\mean}{mean}\\def\\b{y}\\renewcommand\\c{z}");
        assert_eq!(m["a"].nargs, 1);
        assert_eq!(m["mean"].body, "\\operatorname{mean}");
        assert_eq!(m["b"].body, "y");
        assert_eq!(m["c"].body, "z");
    }
}
