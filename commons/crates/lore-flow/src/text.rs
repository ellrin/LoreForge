//! Labels: plain text with `$math$` and `@notation-id` references.
//!
//! Math goes to PowerPoint as Office Math when a [`MathSource`] can convert it
//! (pandoc through lore-math); without one it becomes ordinary text runs with
//! Unicode symbols, italics and sub/superscripts, which is also what the
//! preview draws and what widths are estimated from.

use std::cell::RefCell;
use std::collections::BTreeMap;

use lore_canvas::{Look, Para, Run, TextBody};

#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    Text(String),
    Math(String),
}

/// Split on unescaped `$`.
pub fn parse_rich(s: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut math = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' && it.peek() == Some(&'$') {
            cur.push('$');
            it.next();
        } else if c == '$' {
            if !cur.is_empty() {
                out.push(if math { Seg::Math(std::mem::take(&mut cur)) } else { Seg::Text(std::mem::take(&mut cur)) });
            }
            math = !math;
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        // an unclosed `$` is kept as text
        out.push(if math { Seg::Text(format!("${cur}")) } else { Seg::Text(cur) });
    }
    out
}

/// Every math segment of a label.
pub fn math_of(s: &str) -> Vec<String> {
    parse_rich(s).into_iter().filter_map(|x| if let Seg::Math(m) = x { Some(m) } else { None }).collect()
}

/// One stretch of converted math.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub text: String,
    /// -1 subscript, 0 baseline, 1 superscript
    pub script: i8,
    pub italic: bool,
    pub bold: bool,
}

#[derive(Clone, Copy)]
struct St {
    script: i8,
    upright: bool,
    bold: bool,
    cal: bool,
    bb: bool,
}

fn greek(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" | "varrho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "ϕ",
        "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        _ => return None,
    })
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "times" => "×",
        "cdot" => "·",
        "to" | "rightarrow" => "→",
        "leftarrow" | "gets" => "←",
        "leftrightarrow" => "↔",
        "odot" => "⊙",
        "oplus" => "⊕",
        "otimes" => "⊗",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "in" => "∈",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "approx" => "≈",
        "sim" => "∼",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "pm" => "±",
        "ell" => "ℓ",
        "mid" => "|",
        "lVert" | "rVert" | "Vert" | "|" => "‖",
        "lvert" | "rvert" | "vert" => "|",
        "langle" => "⟨",
        "rangle" => "⟩",
        "cup" => "∪",
        "cap" => "∩",
        "circ" => "∘",
        "ast" => "∗",
        "star" => "⋆",
        "prime" => "′",
        "forall" => "∀",
        "exists" => "∃",
        "," | ";" | ":" | " " | "quad" | "qquad" | "!" => " ",
        "{" => "{",
        "}" => "}",
        "%" => "%",
        "_" => "_",
        "&" => "&",
        "#" => "#",
        "left" | "right" | "big" | "Big" | "bigg" | "displaystyle" | "textstyle" | "limits" | "nolimits" => "",
        _ => return None,
    })
}

fn script_cap(c: char) -> Option<char> {
    // Mathematical script capitals; a few live in the Letterlike block.
    Some(match c {
        'B' => 'ℬ',
        'E' => 'ℰ',
        'F' => 'ℱ',
        'H' => 'ℋ',
        'I' => 'ℐ',
        'L' => 'ℒ',
        'M' => 'ℳ',
        'R' => 'ℛ',
        'A'..='Z' => char::from_u32(0x1D49C + (c as u32 - 'A' as u32))?,
        _ => return None,
    })
}

fn bb_cap(c: char) -> Option<char> {
    Some(match c {
        'C' => 'ℂ',
        'H' => 'ℍ',
        'N' => 'ℕ',
        'P' => 'ℙ',
        'Q' => 'ℚ',
        'R' => 'ℝ',
        'Z' => 'ℤ',
        'A'..='Z' => char::from_u32(0x1D538 + (c as u32 - 'A' as u32))?,
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum T {
    Cmd(String),
    Group(Vec<T>),
    Ch(char),
}

fn lex(cs: &[char], i: &mut usize, in_group: bool) -> Vec<T> {
    let mut out = Vec::new();
    while *i < cs.len() {
        let c = cs[*i];
        *i += 1;
        match c {
            '{' => out.push(T::Group(lex(cs, i, true))),
            '}' if in_group => return out,
            '}' => {}
            '\\' => {
                let mut name = String::new();
                while *i < cs.len() && cs[*i].is_ascii_alphabetic() {
                    name.push(cs[*i]);
                    *i += 1;
                }
                if name.is_empty() && *i < cs.len() {
                    name.push(cs[*i]);
                    *i += 1;
                }
                out.push(T::Cmd(name));
            }
            c if c.is_whitespace() => {}
            c => out.push(T::Ch(c)),
        }
    }
    out
}

fn push(out: &mut Vec<Piece>, text: &str, st: St, italic: bool) {
    if text.is_empty() {
        return;
    }
    if let Some(l) = out.last_mut()
        && l.script == st.script
        && l.italic == italic
        && l.bold == st.bold
    {
        l.text.push_str(text);
        return;
    }
    out.push(Piece { text: text.to_string(), script: st.script, italic, bold: st.bold });
}

fn emit(toks: &[T], st: St, out: &mut Vec<Piece>) {
    let mut i = 0;
    // the next argument: a group's tokens, or the single following token
    fn arg(toks: &[T], i: &mut usize) -> Vec<T> {
        match toks.get(*i) {
            Some(T::Group(g)) => {
                *i += 1;
                g.clone()
            }
            Some(t) => {
                *i += 1;
                vec![t.clone()]
            }
            None => vec![],
        }
    }
    while i < toks.len() {
        let t = &toks[i];
        i += 1;
        match t {
            T::Group(g) => emit(g, st, out),
            T::Ch('_') | T::Ch('^') => {
                let a = arg(toks, &mut i);
                let script = if *t == T::Ch('_') { -1 } else { 1 };
                emit(&a, St { script, ..st }, out);
            }
            T::Ch('\'') => push(out, "′", st, false),
            T::Ch('~') => push(out, " ", st, false),
            T::Ch(c) => {
                let s = if st.cal { script_cap(*c) } else if st.bb { bb_cap(*c) } else { None }.unwrap_or(*c).to_string();
                let italic = c.is_alphabetic() && !st.upright && !st.cal && !st.bb;
                push(out, &s, st, italic);
            }
            T::Cmd(name) => {
                let n = name.as_str();
                if let Some(g) = greek(n) {
                    // lower-case Greek is italic, capitals upright
                    push(out, g, st, n.chars().next().is_some_and(|c| c.is_lowercase()) && !st.upright);
                } else if let Some(s) = symbol(n) {
                    push(out, s, st, false);
                } else {
                    match n {
                        "mathcal" | "mathscr" => {
                            let a = arg(toks, &mut i);
                            emit(&a, St { cal: true, ..st }, out);
                        }
                        "mathbb" => {
                            let a = arg(toks, &mut i);
                            emit(&a, St { bb: true, ..st }, out);
                        }
                        "mathbf" | "textbf" => {
                            let a = arg(toks, &mut i);
                            emit(&a, St { bold: true, upright: true, ..st }, out);
                        }
                        "boldsymbol" | "bm" => {
                            let a = arg(toks, &mut i);
                            emit(&a, St { bold: true, ..st }, out);
                        }
                        "mathrm" | "text" | "operatorname" | "mathsf" | "texttt" | "mathtt" | "textrm" => {
                            let a = arg(toks, &mut i);
                            emit(&a, St { upright: true, ..st }, out);
                        }
                        "mathit" | "textit" => {
                            let a = arg(toks, &mut i);
                            emit(&a, St { upright: false, ..st }, out);
                        }
                        "hat" | "widehat" | "tilde" | "widetilde" | "bar" | "overline" | "vec" | "dot" | "ddot" => {
                            let a = arg(toks, &mut i);
                            let mark = match n {
                                "hat" | "widehat" => '\u{0302}',
                                "tilde" | "widetilde" => '\u{0303}',
                                "bar" | "overline" => '\u{0304}',
                                "vec" => '\u{20D7}',
                                "dot" => '\u{0307}',
                                _ => '\u{0308}',
                            };
                            let mut inner = Vec::new();
                            emit(&a, st, &mut inner);
                            // the mark sits on the first base character
                            if let Some(p) = inner.first_mut() {
                                let mut cs: Vec<char> = p.text.chars().collect();
                                if !cs.is_empty() {
                                    cs.insert(1, mark);
                                }
                                p.text = cs.into_iter().collect();
                            }
                            for p in inner {
                                // no merging across the accent so the mark stays with its letter
                                out.push(p);
                            }
                        }
                        "frac" | "tfrac" | "dfrac" => {
                            let a = arg(toks, &mut i);
                            let b = arg(toks, &mut i);
                            emit(&a, st, out);
                            push(out, "/", st, false);
                            emit(&b, st, out);
                        }
                        "sqrt" => {
                            let a = arg(toks, &mut i);
                            push(out, "√", st, false);
                            emit(&a, st, out);
                        }
                        "log" | "exp" | "min" | "max" | "arg" | "sin" | "cos" | "tanh" | "det" | "dim" | "lim" | "softmax" => push(out, n, st, false),
                        _ => push(out, n, st, false),
                    }
                }
            }
        }
    }
}

/// LaTeX math → pieces of Unicode text (approximation: fractions become `a/b`).
pub fn latex_pieces(latex: &str) -> Vec<Piece> {
    let cs: Vec<char> = latex.chars().collect();
    let mut i = 0;
    let toks = lex(&cs, &mut i, false);
    let mut out = Vec::new();
    emit(&toks, St { script: 0, upright: false, bold: false, cal: false, bb: false }, &mut out);
    out
}

/// Plain Unicode reading of a label (math approximated).
pub fn plain(s: &str) -> String {
    parse_rich(s)
        .into_iter()
        .map(|seg| match seg {
            Seg::Text(t) => t,
            Seg::Math(m) => latex_pieces(&m).into_iter().map(|p| p.text).collect(),
        })
        .collect()
}

fn char_w(c: char) -> f64 {
    match c {
        '\u{0300}'..='\u{036F}' | '\u{20D0}'..='\u{20FF}' => 0.0,
        ' ' => 0.27,
        'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' | '′' | 'I' | 't' | 'f' | 'r' => 0.3,
        '(' | ')' | '[' | ']' | '-' | '/' | '·' => 0.33,
        'm' | 'w' | 'M' | 'W' | '⊕' | '⊗' | '⊙' | '∑' => 0.82,
        'A'..='Z' => 0.63,
        'a'..='z' | '0'..='9' => 0.5,
        c if (c as u32) >= 0x1D400 => 0.72,
        c if (c as u32) >= 0x2E80 => 1.0,
        _ => 0.56,
    }
}

/// Estimated width in pt of a label set at `size` pt (one line).
pub fn width(s: &str, size: f64) -> f64 {
    let mut w = 0.0;
    for seg in parse_rich(s) {
        match seg {
            Seg::Text(t) => w += t.chars().map(char_w).sum::<f64>() * size,
            Seg::Math(m) => {
                for p in latex_pieces(&m) {
                    let k = if p.script == 0 { 1.0 } else { 0.68 };
                    w += p.text.chars().map(char_w).sum::<f64>() * size * k * 1.05;
                }
            }
        }
    }
    w
}

/// Lines of a label: a real line break or LaTeX's `\\` starts a new line.
pub fn lines(s: &str) -> Vec<String> {
    s.replace("\\\\", "\n").split('\n').map(|l| l.trim().to_string()).collect()
}

/// Width of the widest line and the number of lines.
pub fn block(s: &str, size: f64) -> (f64, usize) {
    let lines = lines(s);
    (lines.iter().map(|l| width(l, size)).fold(0.0, f64::max), lines.len().max(1))
}

// ---------------------------------------------------------------------------
// Variables (notation ids, registered macros)
// ---------------------------------------------------------------------------

/// What `@id` and registered macros stand for.
#[derive(Debug, Clone, Default)]
pub struct Vars {
    /// notation id → LaTeX
    pub ids: BTreeMap<String, String>,
    /// `\macro` → LaTeX body (registered macros, so labels may use them)
    pub macros: BTreeMap<String, String>,
}

impl Vars {
    pub fn from_notation(n: &lore_notation::Notation) -> Vars {
        let mut v = Vars::default();
        v.add_notation(n);
        v
    }
    pub fn add_notation(&mut self, n: &lore_notation::Notation) {
        let macros = lore_notation::katex_macros(n);
        for e in &n.entries {
            if let Some(l) = &e.latex {
                // a reference through the registry uses the registered macro when there is one
                let tex = match e.macro_name() {
                    Some(m) if macros.contains_key(&format!("\\{m}")) => format!("\\{m}"),
                    _ => l.clone(),
                };
                self.ids.insert(e.id.clone(), tex);
            }
        }
        self.macros.extend(macros);
    }
    pub fn add_new(&mut self, vars: &[crate::spec::VarNew]) {
        for v in vars {
            self.ids.entry(v.id.clone()).or_insert_with(|| v.latex.clone());
        }
    }

    /// Replace `@id` with `$latex$` (unknown ids stay as typed; they are
    /// reported by [`Vars::unknown_refs`]).
    pub fn resolve(&self, s: &str) -> String {
        let mut out = String::new();
        let mut math = false;
        let cs: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < cs.len() {
            let c = cs[i];
            if c == '$' && (i == 0 || cs[i - 1] != '\\') {
                math = !math;
            }
            if c == '@' {
                let mut j = i + 1;
                while j < cs.len() && (cs[j].is_ascii_alphanumeric() || "_-.:".contains(cs[j])) {
                    j += 1;
                }
                // trailing punctuation is not part of the id
                while j > i + 1 && ".:-".contains(cs[j - 1]) {
                    j -= 1;
                }
                let id: String = cs[i + 1..j].iter().collect();
                if let Some(l) = self.ids.get(&id) {
                    if math {
                        out.push_str(&format!("{{{l}}}"));
                    } else {
                        out.push_str(&format!("${l}$"));
                    }
                    i = j;
                    continue;
                }
            }
            out.push(c);
            i += 1;
        }
        out
    }

    /// `@ids` in `s` that no registry entry or new variable defines.
    pub fn unknown_refs(&self, s: &str) -> Vec<String> {
        let cs: Vec<char> = s.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < cs.len() {
            if cs[i] == '@' {
                let mut j = i + 1;
                while j < cs.len() && (cs[j].is_ascii_alphanumeric() || "_-.:".contains(cs[j])) {
                    j += 1;
                }
                while j > i + 1 && ".:-".contains(cs[j - 1]) {
                    j -= 1;
                }
                let id: String = cs[i + 1..j].iter().collect();
                if !id.is_empty() && !self.ids.contains_key(&id) {
                    out.push(id);
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
        out
    }

    /// Expand registered macros in a piece of math (`\ribmask` → its body).
    pub fn expand(&self, latex: &str) -> String {
        if self.macros.is_empty() || !latex.contains('\\') {
            return latex.to_string();
        }
        let mut cur = latex.to_string();
        for _ in 0..4 {
            let cs: Vec<char> = cur.chars().collect();
            let mut out = String::new();
            let mut i = 0;
            let mut changed = false;
            while i < cs.len() {
                if cs[i] == '\\' {
                    let mut j = i + 1;
                    while j < cs.len() && cs[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    let name: String = cs[i..j].iter().collect();
                    if let Some(body) = self.macros.get(&name) {
                        out.push_str(&format!("{{{body}}}"));
                        changed = true;
                        i = j;
                        continue;
                    }
                    out.push_str(&name);
                    if j == i + 1 && j < cs.len() {
                        out.push(cs[j]);
                        j += 1;
                    }
                    i = j;
                } else {
                    out.push(cs[i]);
                    i += 1;
                }
            }
            cur = out;
            if !changed {
                break;
            }
        }
        cur
    }
}

// ---------------------------------------------------------------------------
// Office Math
// ---------------------------------------------------------------------------

/// Converts LaTeX math to PowerPoint Office Math (`a14:m` XML).
pub trait MathSource {
    fn omml(&self, latex: &str, size_pt: f64) -> Option<String>;
}

/// No converter: math is written as ordinary text runs.
pub struct NoMath;
impl MathSource for NoMath {
    fn omml(&self, _: &str, _: f64) -> Option<String> {
        None
    }
}

/// pandoc (through lore-math), with a cache per formula and size.
pub struct PandocMath {
    pandoc: lore_math::Pandoc,
    ok: bool,
    cache: RefCell<BTreeMap<(String, i64), Option<String>>>,
}

impl PandocMath {
    pub fn new(pandoc: lore_math::Pandoc) -> Self {
        let ok = pandoc.available();
        PandocMath { pandoc, ok, cache: RefCell::new(BTreeMap::new()) }
    }
    pub fn available(&self) -> bool {
        self.ok
    }
}

impl MathSource for PandocMath {
    fn omml(&self, latex: &str, size_pt: f64) -> Option<String> {
        if !self.ok {
            return None;
        }
        let key = (latex.to_string(), (size_pt * 100.0).round() as i64);
        if let Some(v) = self.cache.borrow().get(&key) {
            return v.clone();
        }
        let v = self.pandoc.latex_to_pptx_math(latex, false, size_pt).ok().map(|e| e.to_xml_string());
        self.cache.borrow_mut().insert(key, v.clone());
        v
    }
}

/// How text is set.
#[derive(Debug, Clone)]
pub struct Type {
    pub font: String,
    /// East Asian font (empty = the deck's own).
    pub ea_font: String,
    pub math_font: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub color: String,
}

impl Type {
    pub fn look(&self) -> Look {
        Look { font: self.font.clone(), ea_font: self.ea_font.clone(), size: self.size, bold: self.bold, italic: self.italic, color: self.color.clone(), ..Look::default() }
    }
}

fn text_run(text: &str, look: Look) -> Run {
    Run { kind: "text".into(), text: text.into(), tag: None, em: None, look, field: None, omml: None, latex: None, src: None, k: 0 }
}

/// Runs of one line. Returns whether any Office Math run was made.
pub fn runs(s: &str, ty: &Type, vars: &Vars, math: &dyn MathSource) -> (Vec<Run>, bool) {
    let mut out = Vec::new();
    let mut has_math = false;
    for seg in parse_rich(s) {
        match seg {
            Seg::Text(t) => out.push(text_run(&t, ty.look())),
            Seg::Math(m) => {
                let full = vars.expand(&m);
                let wrapped = if ty.bold { format!("\\boldsymbol{{{full}}}") } else { full.clone() };
                if let Some(omml) = math.omml(&wrapped, ty.size) {
                    has_math = true;
                    let look = Look { font: ty.math_font.clone(), size: ty.size, color: ty.color.clone(), ..Look::default() };
                    let mut r = text_run(&m, look);
                    r.kind = "math".into();
                    r.omml = Some(if ty.color != "000000" { recolor(&omml, &ty.color) } else { omml });
                    r.latex = Some(m.clone());
                    out.push(r);
                } else {
                    for p in latex_pieces(&full) {
                        let mut l = ty.look();
                        l.italic = p.italic;
                        l.bold = ty.bold || p.bold;
                        l.baseline = match p.script {
                            1 => 30.0,
                            -1 => -25.0,
                            _ => 0.0,
                        };
                        out.push(text_run(&p.text, l));
                    }
                }
            }
        }
    }
    (out, has_math)
}

fn recolor(omml: &str, hex: &str) -> String {
    match lore_ooxml::xml::parse_element(omml.as_bytes()) {
        Ok(mut e) => {
            lore_canvas::pptx_out::recolor_math(&mut e, hex);
            e.to_xml_string()
        }
        Err(_) => omml.to_string(),
    }
}

/// A centred text body (lines split on `\n`). Returns whether it holds Office Math.
pub fn body(s: &str, ty: &Type, vars: &Vars, math: &dyn MathSource, align: &str, anchor: &str) -> (TextBody, bool) {
    let mut has = false;
    let mut paras = Vec::new();
    for line in lines(s) {
        let (rs, m) = runs(&line, ty, vars, math);
        has |= m;
        let mut p = Para::new(rs);
        p.align = align.into();
        p.end = Some(ty.look());
        paras.push(p);
    }
    (TextBody { insets: [0, 0, 0, 0], anchor: anchor.into(), wrap: false, paras, ..TextBody::default() }, has)
}
