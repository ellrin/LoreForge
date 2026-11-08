//! Math-mode LaTeX → symbol atoms.
//!
//! A symbol is a base letter (Latin, Greek or a bold digit) with a font
//! (plain, bold, calligraphic, blackboard…), accents (tilde, hat, bar…) and
//! the sub/superscripts that are part of its *name* (`^{\mathrm{nf}}`,
//! `_\theta`, `_{\boldsymbol{x}}`). Index subscripts (`_i`, `_{(j)}`) and
//! exponents (`^2`, `^{N\times D}`) are not part of the name; symbols inside
//! exponents still count as uses.
//!
//! Every symbol has a canonical key, e.g. `\tilde{\boldsymbol{p}}^{\mathrm{f}}`;
//! `\bm`, `\mathbf` and `\boldsymbol` all become `\boldsymbol`, `\widetilde`
//! becomes `\tilde`, `\bar\theta` becomes `\bar{\theta}`.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// `\name` (letters) or `\c` (one non-letter), without the backslash.
    Cmd(String),
    Ch(char),
    Open,
    Close,
    Sub,
    Sup,
    Amp,
    Param(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct T {
    pub tok: Tok,
    /// Byte offset in the source the segment came from.
    pub off: usize,
    /// Offset just after the token.
    pub end: usize,
}

/// Tokenize math source; offsets are `base + local offset`.
pub fn tokenize(src: &str, base: usize) -> Vec<T> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = src[i..].chars().next().unwrap();
        let off = base + i;
        match c {
            '\\' => {
                let rest = &src[i + 1..];
                let n: usize = rest.chars().take_while(|c| c.is_ascii_alphabetic()).map(|c| c.len_utf8()).sum();
                if n > 0 {
                    let mut name = rest[..n].to_string();
                    let mut len = 1 + n;
                    if rest[n..].starts_with('*') && matches!(name.as_str(), "operatorname" | "tag") {
                        name.push('*');
                        len += 1;
                    }
                    out.push(T { tok: Tok::Cmd(name), off, end: off + len });
                    i += len;
                } else if let Some(ch) = rest.chars().next() {
                    out.push(T { tok: Tok::Cmd(ch.to_string()), off, end: off + 1 + ch.len_utf8() });
                    i += 1 + ch.len_utf8();
                } else {
                    i += 1;
                }
                continue;
            }
            '{' => out.push(T { tok: Tok::Open, off, end: off + 1 }),
            '}' => out.push(T { tok: Tok::Close, off, end: off + 1 }),
            '_' => out.push(T { tok: Tok::Sub, off, end: off + 1 }),
            '^' => out.push(T { tok: Tok::Sup, off, end: off + 1 }),
            '&' => out.push(T { tok: Tok::Amp, off, end: off + 1 }),
            '#' => {
                let d = src[i + 1..].chars().next().and_then(|d| d.to_digit(10));
                if let Some(d) = d {
                    out.push(T { tok: Tok::Param(d as u8), off, end: off + 2 });
                    i += 2;
                    continue;
                }
            }
            c if c.is_whitespace() => {}
            '~' => {}
            _ => out.push(T { tok: Tok::Ch(c), off, end: off + c.len_utf8() }),
        }
        i += c.len_utf8();
    }
    out
}

/// A user macro: number of arguments and body tokens (offsets irrelevant).
#[derive(Debug, Clone, PartialEq)]
pub struct Macro {
    pub nargs: u8,
    pub body: String,
}

/// Expand user macros (registry + `\newcommand` in the document) in a
/// token stream. Expanded tokens take the offset of the macro call so
/// locations still point at the source.
pub fn expand(toks: Vec<T>, macros: &BTreeMap<String, Macro>) -> Vec<T> {
    if macros.is_empty() {
        return toks;
    }
    expand_depth(toks, macros, 0)
}

fn expand_depth(toks: Vec<T>, macros: &BTreeMap<String, Macro>, depth: usize) -> Vec<T> {
    if depth > 8 {
        return toks;
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    let mut changed = false;
    while i < toks.len() {
        if let Tok::Cmd(name) = &toks[i].tok
            && let Some(m) = macros.get(name) {
                let off = toks[i].off;
                i += 1;
                let call_start = i;
                let mut args: Vec<Vec<T>> = Vec::new();
                for _ in 0..m.nargs {
                    let (arg, next) = take_arg(&toks, i);
                    args.push(arg);
                    i = next;
                }
                let end = toks.get(i.max(call_start) - 1).map(|t| t.end).unwrap_or(off + 1);
                for t in tokenize(&m.body, 0) {
                    match t.tok {
                        Tok::Param(k) if (k as usize) >= 1 && (k as usize) <= args.len() => {
                            out.push(T { tok: Tok::Open, off, end });
                            out.extend(args[k as usize - 1].iter().cloned());
                            out.push(T { tok: Tok::Close, off, end });
                        }
                        tok => out.push(T { tok, off, end }),
                    }
                }
                changed = true;
                continue;
            }
        out.push(toks[i].clone());
        i += 1;
    }
    if changed { expand_depth(out, macros, depth + 1) } else { out }
}

/// One argument starting at `i`: a braced group (without braces) or one token.
fn take_arg(toks: &[T], i: usize) -> (Vec<T>, usize) {
    match toks.get(i).map(|t| &t.tok) {
        Some(Tok::Open) => {
            let end = matching(toks, i);
            (toks[i + 1..end.min(toks.len())].to_vec(), (end + 1).min(toks.len()))
        }
        Some(_) => (vec![toks[i].clone()], i + 1),
        None => (Vec::new(), i),
    }
}

/// Index of the `Close` matching the `Open` at `i` (or len).
fn matching(toks: &[T], i: usize) -> usize {
    let mut d = 0usize;
    for (j, t) in toks.iter().enumerate().skip(i) {
        match t.tok {
            Tok::Open => d += 1,
            Tok::Close => {
                d = d.saturating_sub(1);
                if d == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    toks.len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Font {
    Plain,
    Bold,
    Cal,
    Scr,
    Bb,
    Frak,
    Sf,
    Tt,
}

impl Font {
    fn wrap(self, s: &str) -> String {
        match self {
            Font::Plain => s.to_string(),
            Font::Bold => format!("\\boldsymbol{{{s}}}"),
            Font::Cal => format!("\\mathcal{{{s}}}"),
            Font::Scr => format!("\\mathscr{{{s}}}"),
            Font::Bb => format!("\\mathbb{{{s}}}"),
            Font::Frak => format!("\\mathfrak{{{s}}}"),
            Font::Sf => format!("\\mathsf{{{s}}}"),
            Font::Tt => format!("\\mathtt{{{s}}}"),
        }
    }
}

fn font_of(cmd: &str) -> Option<Font> {
    Some(match cmd {
        "mathbf" | "boldsymbol" | "bm" | "mathbold" | "pmb" | "mathbfit" | "symbf" => Font::Bold,
        "mathcal" | "symcal" => Font::Cal,
        "mathscr" => Font::Scr,
        "mathbb" | "mathbbm" | "mathds" | "bbm" | "symbb" => Font::Bb,
        "mathfrak" => Font::Frak,
        "mathsf" => Font::Sf,
        "mathtt" => Font::Tt,
        _ => return None,
    })
}

fn accent_of(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "tilde" | "widetilde" | "wtilde" => "tilde",
        "hat" | "widehat" => "hat",
        "bar" | "overline" => "bar",
        "vec" | "overrightarrow" => "vec",
        "dot" => "dot",
        "ddot" => "ddot",
        "check" | "widecheck" => "check",
        "breve" => "breve",
        "acute" => "acute",
        "grave" => "grave",
        "mathring" => "mathring",
        "underline" => "underline",
        _ => return None,
    })
}

pub const GREEK: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "varepsilon", "zeta", "eta", "theta", "vartheta", "iota", "kappa", "varkappa",
    "lambda", "mu", "nu", "xi", "pi", "varpi", "rho", "varrho", "sigma", "varsigma", "tau", "upsilon", "phi", "varphi", "chi",
    "psi", "omega", "Gamma", "Delta", "Theta", "Lambda", "Xi", "Pi", "Sigma", "Upsilon", "Phi", "Psi", "Omega", "varGamma",
    "varDelta", "varTheta", "varLambda", "varXi", "varPi", "varSigma", "varUpsilon", "varPhi", "varPsi", "varOmega", "ell",
    "hbar", "imath", "jmath", "aleph", "digamma",
];

const OPERATORS: &[&str] = &[
    "log", "ln", "lg", "exp", "sin", "cos", "tan", "cot", "sec", "csc", "sinh", "cosh", "tanh", "arcsin", "arccos", "arctan",
    "max", "min", "sup", "inf", "arg", "lim", "liminf", "limsup", "det", "dim", "ker", "deg", "gcd", "hom", "Pr", "sum", "prod",
    "coprod", "int", "iint", "iiint", "oint", "bigcup", "bigcap", "bigoplus", "bigotimes", "bigvee", "bigwedge", "bigsqcup",
    "mod", "bmod", "pmod", "argmax", "argmin", "partial", "nabla",
];

const RELATIONS: &[&str] = &[
    "in", "notin", "leq", "le", "geq", "ge", "neq", "ne", "approx", "sim", "simeq", "equiv", "subset", "subseteq", "supset",
    "supseteq", "coloneqq", "triangleq", "propto", "leftarrow", "gets", "rightarrow", "to", "mapsto", "Rightarrow",
    "Leftarrow", "Leftrightarrow", "leftrightarrow", "longrightarrow", "longmapsto", "ll", "gg", "prec", "succ", "cong",
    "doteq", "defeq", "eqqcolon",
];

/// Commands whose braced argument is text or a key, never math symbols.
const TEXT_ARG: &[&str] = &[
    "text", "textrm", "textit", "textbf", "textsf", "texttt", "textup", "textnormal", "mbox", "hbox", "label", "tag", "tag*",
    "eqref", "ref", "cite", "hspace", "vspace", "intertext", "shortintertext", "phantom", "hphantom", "vphantom", "color",
    "mathrm", "mathup", "mathit", "operatorname", "operatorname*", "mathop", "begin", "end", "SI", "si", "unit", "qty",
];

/// Commands that are neither symbols nor take arguments we care about.
const IGNORE: &[&str] = &[
    "left", "right", "big", "Big", "bigg", "Bigg", "bigl", "bigr", "Bigl", "Bigr", "biggl", "biggr", "middle", "quad", "qquad",
    ",", "!", ";", ":", " ", ">", "displaystyle", "textstyle", "scriptstyle", "scriptscriptstyle", "nonumber", "notag",
    "limits", "nolimits", "cdots", "ldots", "dots", "vdots", "ddots", "dotsc", "dotsb", "infty", "top", "prime", "ensuremath",
    "lbrace", "rbrace", "langle", "rangle", "lceil", "rceil", "lfloor", "rfloor", "vert", "Vert", "|", "{", "}", "allowbreak",
    "cdot", "times", "odot", "otimes", "oplus", "circ", "pm", "mp", "div", "ast", "star", "cup", "cap", "setminus", "wedge",
    "vee", "land", "lor", "neg", "lnot", "forall", "exists", "emptyset", "varnothing", "intercal", "dagger", "backslash",
    "#", "%", "$", "&", "_", "mid", "parallel", "perp", "angle", "triangle", "square", "checkmark", "dag", "S", "P",
];

pub fn is_greek(s: &str) -> bool {
    GREEK.contains(&s)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Script {
    /// Part of the symbol's name; canonical form.
    Name(String),
    /// Index (`_i`, `_{(j)}`): dropped from the name.
    Index,
    /// Exponent / dimension (`^2`, `^{D}`): dropped, inner symbols are uses.
    Exponent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sym {
    /// `m`, `\theta`, `1`.
    pub base: String,
    pub font: Font,
    /// Inner first.
    pub accents: Vec<&'static str>,
    pub sub: Option<Script>,
    pub sup: Option<Script>,
    /// Single italic letter used as a name subscript (`\lambda_m` → `m`).
    pub sub_letter: Option<String>,
    pub start: usize,
    pub end: usize,
    /// Symbols inside name scripts (`\theta` in `f_\theta`).
    pub qualifiers: Vec<Sym>,
    /// Symbols inside exponents / index scripts that are not bound.
    pub script_uses: Vec<Sym>,
    /// Rendered exponent (`3\times4` in `\mathbb{R}^{3\times4}`).
    pub exp: Option<String>,
}

impl Sym {
    /// Canonical key without sub/superscripts.
    pub fn stem(&self) -> String {
        let mut s = self.font.wrap(&self.base);
        for a in &self.accents {
            s = format!("\\{a}{{{s}}}");
        }
        s
    }

    pub fn key(&self) -> String {
        let mut s = self.stem();
        if let Some(Script::Name(n)) = &self.sub {
            s.push_str(&format!("_{{{n}}}"));
        }
        if let Some(Script::Name(n)) = &self.sup {
            s.push_str(&format!("^{{{n}}}"));
        }
        s
    }

    /// Name superscript, if any.
    pub fn sup_name(&self) -> Option<&str> {
        match &self.sup {
            Some(Script::Name(n)) => Some(n),
            _ => None,
        }
    }

    pub fn sub_name(&self) -> Option<&str> {
        match &self.sub {
            Some(Script::Name(n)) => Some(n),
            _ => None,
        }
    }

    /// The bare letter, e.g. `m` for `\boldsymbol{m}` and `\lambda` for `\bar\lambda`.
    pub fn letter(&self) -> String {
        if self.base.chars().count() == 1 { self.base.clone() } else { format!("\\{}", self.base.trim_start_matches('\\')) }
    }

    /// Plain italic bare letter (no font, accent or scripts).
    pub fn is_bare_plain(&self) -> bool {
        self.font == Font::Plain && self.accents.is_empty() && self.sub_name().is_none() && self.sup_name().is_none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Atom {
    Sym(Sym),
    /// Named operator (`log`, `max`, `\operatorname{mean}` → `mean`).
    Op { name: String, custom: bool, start: usize },
    Num(String),
    Rel(String),
    /// Opening/closing delimiter or other punctuation.
    Punct(char),
    Bin(String),
    Text(String),
    Group(Vec<Atom>),
    /// Content of a command argument such as `\frac{..}{..}` (not top level).
    Arg(Vec<Atom>),
    /// `&`
    Align,
    /// `\\`
    Row,
    Other(String),
}

pub struct Options<'a> {
    pub index_letters: &'a [String],
}

/// Parse a token stream into atoms.
pub fn parse(toks: &[T], opt: &Options) -> Vec<Atom> {
    let mut p = Parser { t: toks, i: 0, opt };
    p.seq(false)
}

struct Parser<'a, 'b> {
    t: &'a [T],
    i: usize,
    opt: &'b Options<'b>,
}

impl Parser<'_, '_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i).map(|t| &t.tok)
    }
    fn off(&self) -> usize {
        self.t.get(self.i).or(self.t.last()).map(|t| t.off).unwrap_or(0)
    }
    fn end_off(&self) -> usize {
        self.t.get(self.i.saturating_sub(1)).map(|t| t.end).unwrap_or(0)
    }

    /// Atoms until end or a closing brace (consumed when `in_group`).
    fn seq(&mut self, in_group: bool) -> Vec<Atom> {
        let mut out: Vec<Atom> = Vec::new();
        while let Some(tok) = self.peek() {
            if *tok == Tok::Close {
                self.i += 1;
                if in_group {
                    break;
                }
                continue;
            }
            match tok {
                Tok::Sub | Tok::Sup => {
                    let atom = out.pop().unwrap_or(Atom::Other(String::new()));
                    let atom = self.scripts(atom);
                    out.push(atom);
                }
                Tok::Ch('\'') => {
                    self.i += 1;
                    if let Some(Atom::Sym(s)) = out.last_mut()
                        && s.sup.is_none() {
                            s.sup = Some(Script::Name("\\prime".into()));
                        }
                }
                _ => {
                    if let Some(a) = self.atom() {
                        out.push(a);
                    }
                }
            }
        }
        out
    }

    /// One argument: group content or single atom.
    fn arg(&mut self) -> Vec<Atom> {
        match self.peek() {
            Some(Tok::Open) => {
                self.i += 1;
                self.seq(true)
            }
            Some(_) => self.atom().into_iter().collect(),
            None => Vec::new(),
        }
    }

    /// Raw text of a braced argument (for `\mathrm{nf}`, `\operatorname{mean}`).
    fn raw_arg(&mut self) -> String {
        let mut s = String::new();
        match self.peek() {
            Some(Tok::Open) => {
                let end = matching(self.t, self.i);
                for t in &self.t[self.i + 1..end.min(self.t.len())] {
                    match &t.tok {
                        Tok::Ch(c) => s.push(*c),
                        Tok::Cmd(c) if c == " " || c == "," => s.push(' '),
                        Tok::Cmd(c) => {
                            s.push('\\');
                            s.push_str(c);
                        }
                        Tok::Sub => s.push('_'),
                        Tok::Sup => s.push('^'),
                        _ => {}
                    }
                }
                self.i = (end + 1).min(self.t.len());
            }
            Some(Tok::Ch(c)) => {
                s.push(*c);
                self.i += 1;
            }
            Some(Tok::Cmd(c)) => {
                s.push('\\');
                s.push_str(c);
                self.i += 1;
            }
            _ => {}
        }
        s
    }

    fn skip_optional(&mut self) {
        if self.peek() == Some(&Tok::Ch('[')) {
            let mut d = 0;
            while let Some(t) = self.peek().cloned() {
                self.i += 1;
                match t {
                    Tok::Ch('[') => d += 1,
                    Tok::Ch(']') => {
                        d -= 1;
                        if d == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn atom(&mut self) -> Option<Atom> {
        let start = self.off();
        let tok = self.peek()?.clone();
        self.i += 1;
        Some(match tok {
            Tok::Open => Atom::Group(self.seq(true)),
            Tok::Close => return None,
            Tok::Amp => Atom::Align,
            Tok::Param(_) => Atom::Other("#".into()),
            Tok::Sub | Tok::Sup => return None,
            Tok::Ch(c) if c.is_ascii_alphabetic() => Atom::Sym(self.sym(c.to_string(), Font::Plain, start)),
            Tok::Ch(c) if c.is_ascii_digit() || c == '.' => {
                let mut s = c.to_string();
                while let Some(Tok::Ch(d)) = self.peek() {
                    if d.is_ascii_digit() || *d == '.' {
                        s.push(*d);
                        self.i += 1;
                    } else {
                        break;
                    }
                }
                Atom::Num(s)
            }
            Tok::Ch(c) if matches!(c, '=' | '<' | '>' | ':') => {
                if c == ':' && self.peek() == Some(&Tok::Ch('=')) {
                    self.i += 1;
                    Atom::Rel(":=".into())
                } else {
                    Atom::Rel(c.to_string())
                }
            }
            Tok::Ch(c) if matches!(c, '+' | '-' | '*' | '/') => Atom::Bin(c.to_string()),
            Tok::Ch(c) => Atom::Punct(c),
            Tok::Cmd(name) => return self.command(&name, start),
        })
    }

    fn command(&mut self, name: &str, start: usize) -> Option<Atom> {
        if name == "\\" {
            self.skip_optional();
            return Some(Atom::Row);
        }
        if let Some(f) = font_of(name) {
            let inner = self.arg();
            return Some(self.apply(inner, |s| {
                if s.font == Font::Plain {
                    s.font = f;
                }
            }, start));
        }
        if let Some(a) = accent_of(name) {
            let inner = self.arg();
            return Some(self.apply(inner, |s| s.accents.push(a), start));
        }
        if is_greek(name) {
            return Some(Atom::Sym(self.sym(format!("\\{name}"), Font::Plain, start)));
        }
        if OPERATORS.contains(&name) {
            return Some(Atom::Op { name: name.to_string(), custom: false, start });
        }
        if RELATIONS.contains(&name) {
            return Some(Atom::Rel(name.to_string()));
        }
        match name {
            "operatorname" | "operatorname*" | "mathop" => {
                let n = self.raw_arg();
                return Some(Atom::Op { name: n.trim().to_string(), custom: true, start });
            }
            "mathrm" | "mathup" | "text" | "textrm" | "textit" | "textbf" | "mathit" | "mbox" | "textsf" | "texttt" => {
                let n = self.raw_arg();
                return Some(Atom::Text(n.trim().to_string()));
            }
            "begin" | "end" => {
                self.raw_arg();
                self.skip_optional();
                if name == "begin" && self.peek() == Some(&Tok::Open) && matches!(self.t.get(self.i + 1).map(|t| &t.tok), Some(Tok::Ch(c)) if "lcr|".contains(*c))
                {
                    self.raw_arg();
                }
                return Some(Atom::Other(String::new()));
            }
            "color" => {
                self.raw_arg();
                return Some(Atom::Other(String::new()));
            }
            "textcolor" => {
                self.raw_arg();
                return Some(Atom::Arg(self.arg()));
            }
            "frac" | "dfrac" | "tfrac" | "cfrac" | "binom" | "dbinom" | "tbinom" | "stackrel" | "overset" | "underset" => {
                let mut a = self.arg();
                a.extend(self.arg());
                return Some(Atom::Arg(a));
            }
            "sqrt" => {
                self.skip_optional();
                return Some(Atom::Arg(self.arg()));
            }
            "xrightarrow" | "xleftarrow" | "xmapsto" => {
                self.skip_optional();
                return Some(Atom::Arg(self.arg()));
            }
            "{" | "}" | "|" | "lbrace" | "rbrace" | "langle" | "rangle" | "Vert" | "vert" => {
                return Some(Atom::Punct(match name {
                    "{" | "lbrace" => '{',
                    "}" | "rbrace" => '}',
                    "langle" => '⟨',
                    "rangle" => '⟩',
                    _ => '|',
                }));
            }
            _ => {}
        }
        if TEXT_ARG.contains(&name) {
            self.raw_arg();
            return Some(Atom::Other(String::new()));
        }
        if IGNORE.contains(&name) {
            return Some(Atom::Bin(name.to_string()));
        }
        Some(Atom::Other(name.to_string()))
    }

    /// Apply a font/accent to the argument: one symbol → modified symbol,
    /// several plain letters → one multi-letter symbol, else a group.
    fn apply(&mut self, mut inner: Vec<Atom>, f: impl Fn(&mut Sym), start: usize) -> Atom {
        if inner.len() == 1 {
            match inner.pop().unwrap() {
                Atom::Sym(mut s) => {
                    f(&mut s);
                    s.start = start;
                    s.end = self.end_off();
                    return Atom::Sym(s);
                }
                Atom::Num(n) => {
                    let mut s = blank_sym(n, start);
                    f(&mut s);
                    s.end = self.end_off();
                    return Atom::Sym(s);
                }
                other => return Atom::Group(vec![other]),
            }
        }
        if !inner.is_empty() && inner.iter().all(|a| matches!(a, Atom::Sym(s) if s.base.len() == 1 && s.sub.is_none() && s.sup.is_none() && s.accents.is_empty() && s.font == Font::Plain))
        {
            let base: String = inner.iter().map(|a| if let Atom::Sym(s) = a { s.base.clone() } else { String::new() }).collect();
            let mut s = blank_sym(base, start);
            f(&mut s);
            s.end = self.end_off();
            return Atom::Sym(s);
        }
        Atom::Group(inner)
    }

    fn sym(&mut self, base: String, font: Font, start: usize) -> Sym {
        let mut s = blank_sym(base, start);
        s.font = font;
        s.end = self.end_off();
        s
    }

    /// Attach `_x` / `^y` to the previous atom.
    fn scripts(&mut self, atom: Atom) -> Atom {
        let mut atom = atom;
        let mut extra: Vec<Atom> = Vec::new();
        while let Some(t) = self.peek().cloned() {
            if t != Tok::Sub && t != Tok::Sup {
                break;
            }
            self.i += 1;
            let content = self.arg();
            let is_sub = t == Tok::Sub;
            match &mut atom {
                Atom::Sym(s) => {
                    let (script, quals, uses, letter) = classify(&content, is_sub, self.opt);
                    if is_sub {
                        s.sub = Some(script);
                        s.sub_letter = letter;
                    } else {
                        if script == Script::Exponent {
                            s.exp = Some(render(&content));
                        }
                        s.sup = Some(script);
                    }
                    s.qualifiers.extend(quals);
                    s.script_uses.extend(uses);
                    s.end = self.end_off();
                }
                _ => {
                    // Subscript of an operator / bracket: binders (`d=1`,
                    // `\tilde p \in P`) are local; other symbols are uses.
                    extra.push(Atom::Arg(unbind(content)));
                }
            }
        }
        if extra.is_empty() {
            atom
        } else {
            extra.insert(0, atom);
            Atom::Group(extra)
        }
    }
}

fn blank_sym(base: String, start: usize) -> Sym {
    Sym {
        base,
        font: Font::Plain,
        accents: Vec::new(),
        sub: None,
        sup: None,
        sub_letter: None,
        start,
        end: start + 1,
        qualifiers: Vec::new(),
        script_uses: Vec::new(),
        exp: None,
    }
}

/// Drop the bound variable of `x = 1` / `x \in S` in an operator script.
fn unbind(content: Vec<Atom>) -> Vec<Atom> {
    let rel = content.iter().position(|a| matches!(a, Atom::Rel(r) if r == "=" || r == "in"));
    match rel {
        Some(k) => content.into_iter().skip(k + 1).collect(),
        None => content,
    }
}

fn significant(content: &[Atom]) -> Vec<&Atom> {
    content.iter().filter(|a| !matches!(a, Atom::Bin(b) if b.is_empty() || b == "," || b == "!" || b == "displaystyle") && !matches!(a, Atom::Other(o) if o.is_empty())).collect()
}

/// (script, qualifier symbols, other symbol uses, single-letter name subscript)
fn classify(content: &[Atom], is_sub: bool, opt: &Options) -> (Script, Vec<Sym>, Vec<Sym>, Option<String>) {
    let sig = significant(content);
    let syms_in = |atoms: &[Atom]| {
        let mut v = Vec::new();
        collect_syms(atoms, &mut v);
        v.retain(|s| !(s.is_bare_plain() && opt.index_letters.contains(&s.base)));
        v
    };
    if sig.is_empty() {
        return (Script::Index, vec![], vec![], None);
    }
    // \mathrm{nf}, \text{real}
    if sig.iter().all(|a| matches!(a, Atom::Text(_))) {
        let n: String = sig.iter().map(|a| if let Atom::Text(t) = a { t.as_str() } else { "" }).collect::<Vec<_>>().join("");
        return (Script::Name(format!("\\mathrm{{{n}}}")), vec![], vec![], None);
    }
    if !is_sub {
        // primes, stars: part of the name
        if sig.iter().all(|a| matches!(a, Atom::Bin(b) if b == "prime" || b == "*" || b == "star" || b == "ast") || matches!(a, Atom::Punct('\''))) {
            return (Script::Name("\\prime".into()), vec![], vec![], None);
        }
        if matches!(sig.first(), Some(Atom::Punct('('))) {
            return (Script::Index, vec![], syms_in(content), None);
        }
        // transpose / inverse / dimensions → exponent; T alone is transpose
        if sig.len() == 1
            && let Atom::Sym(s) = sig[0]
                && (s.base == "T" && s.is_bare_plain() || s.base == "H" && s.is_bare_plain()) {
                    return (Script::Exponent, vec![], vec![], None);
                }
        return (Script::Exponent, vec![], syms_in(content), None);
    }
    // subscripts
    if matches!(sig.first(), Some(Atom::Punct('('))) || sig.iter().any(|a| matches!(a, Atom::Rel(_))) {
        return (Script::Index, vec![], syms_in(&unbind(content.to_vec())), None);
    }
    let all_index = sig.iter().all(|a| matches!(a, Atom::Sym(s) if s.is_bare_plain() && opt.index_letters.contains(&s.base)));
    if all_index {
        return (Script::Index, vec![], vec![], None);
    }
    if sig.iter().all(|a| matches!(a, Atom::Num(_))) {
        let n: String = sig.iter().map(|a| if let Atom::Num(t) = a { t.as_str() } else { "" }).collect();
        return (Script::Name(n), vec![], vec![], None);
    }
    if sig.iter().all(|a| matches!(a, Atom::Sym(_) | Atom::Num(_) | Atom::Text(_))) {
        let mut name = String::new();
        let mut quals = Vec::new();
        for a in &sig {
            match a {
                Atom::Sym(s) => {
                    name.push_str(&s.key());
                    let mut s2 = s.clone();
                    // a subscript's own index parts are irrelevant here
                    s2.qualifiers.clear();
                    quals.extend(s.qualifiers.iter().cloned());
                    quals.push(s2);
                }
                Atom::Num(n) => name.push_str(n),
                Atom::Text(t) => name.push_str(&format!("\\mathrm{{{t}}}")),
                _ => {}
            }
        }
        let letter = match sig.as_slice() {
            [Atom::Sym(s)] if s.is_bare_plain() && s.base.len() == 1 && s.base.chars().all(|c| c.is_ascii_lowercase()) => Some(s.base.clone()),
            _ => None,
        };
        // a single italic letter is a label (\lambda_m), not a use of m
        let quals = if letter.is_some() { Vec::new() } else { quals };
        return (Script::Name(name), quals, vec![], letter);
    }
    (Script::Index, vec![], syms_in(content), None)
}

/// Every symbol, including those inside name scripts and exponents.
pub fn all_syms(atoms: &[Atom]) -> Vec<Sym> {
    let mut v = Vec::new();
    collect_syms(atoms, &mut v);
    let mut out = Vec::new();
    for s in v {
        let nested: Vec<Sym> = s.qualifiers.iter().chain(s.script_uses.iter()).cloned().collect();
        out.push(s);
        for n in nested {
            out.extend(all_syms(&[Atom::Sym(n)]));
        }
    }
    out
}

/// Symbols in atoms, recursively through groups and arguments (not into scripts).
pub fn collect_syms(atoms: &[Atom], out: &mut Vec<Sym>) {
    for a in atoms {
        match a {
            Atom::Sym(s) => out.push(s.clone()),
            Atom::Group(g) | Atom::Arg(g) => collect_syms(g, out),
            _ => {}
        }
    }
}

/// Render atoms to a normalized string (for comparing right-hand sides).
pub fn render(atoms: &[Atom]) -> String {
    let mut s = String::new();
    for a in atoms {
        match a {
            Atom::Sym(x) => {
                s.push_str(&x.key());
                if let Some(e) = &x.exp {
                    s.push_str(&format!("^{{{e}}}"));
                }
            }
            Atom::Op { name, .. } => s.push_str(&format!("\\{name}")),
            Atom::Num(n) => s.push_str(n),
            Atom::Rel(r) => s.push_str(r),
            Atom::Punct(c) => s.push(*c),
            Atom::Bin(b) if b.len() == 1 && !matches!(b.as_str(), "," | "!" | ";" | ":" | " ") => s.push_str(b),
            Atom::Bin(b) if matches!(b.as_str(), "times" | "cdot" | "odot" | "cup" | "cap") => s.push_str(&format!("\\{b}")),
            Atom::Bin(_) => {}
            Atom::Text(t) => s.push_str(t),
            Atom::Group(g) | Atom::Arg(g) => {
                let r = render(g);
                if !r.is_empty() {
                    s.push('{');
                    s.push_str(&r);
                    s.push('}');
                }
            }
            Atom::Align | Atom::Row => {}
            Atom::Other(o) if !o.is_empty() => s.push_str(&format!("\\{o}")),
            Atom::Other(_) => {}
        }
    }
    s
}

/// Canonical key of a LaTeX snippet holding one symbol (`\bm{x}_{i}` →
/// `\boldsymbol{x}`). Snippets that are not a single symbol are normalized
/// by whitespace only.
pub fn canonical(latex: &str) -> String {
    let idx: Vec<String> = DEFAULT_INDEX.iter().map(|s| s.to_string()).collect();
    let toks = tokenize(latex.trim(), 0);
    let atoms = parse(&toks, &Options { index_letters: &idx });
    let sig: Vec<&Atom> = atoms.iter().filter(|a| !matches!(a, Atom::Other(o) if o.is_empty())).collect();
    if let [Atom::Sym(s)] = sig.as_slice() {
        return s.key();
    }
    if let [Atom::Op { name, .. }] = sig.as_slice() {
        return format!("\\operatorname{{{name}}}");
    }
    latex.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub const DEFAULT_INDEX: &[&str] = &["i", "j", "k", "l", "t", "d"];

/// Display text of a symbol key for messages (unchanged LaTeX).
pub fn display(key: &str) -> String {
    key.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(src: &str) -> Vec<String> {
        let idx: Vec<String> = DEFAULT_INDEX.iter().map(|s| s.to_string()).collect();
        let atoms = parse(&tokenize(src, 0), &Options { index_letters: &idx });
        all_syms(&atoms).iter().map(|s| s.key()).collect()
    }

    #[test]
    fn canonical_forms_agree() {
        assert_eq!(canonical("\\bm{x}"), "\\boldsymbol{x}");
        assert_eq!(canonical("\\mathbf x"), "\\boldsymbol{x}");
        assert_eq!(canonical("\\bar\\theta"), "\\bar{\\theta}");
        assert_eq!(canonical("\\widetilde{\\boldsymbol{p}}^{\\mathrm{f}}"), "\\tilde{\\boldsymbol{p}}^{\\mathrm{f}}");
        assert_eq!(canonical("\\tilde{\\bm{p}^{\\mathrm f}}"), "\\tilde{\\boldsymbol{p}}^{\\mathrm{f}}");
        assert_eq!(canonical("\\lambda_m"), "\\lambda_{m}");
        assert_eq!(canonical("\\boldsymbol{c}_i"), "\\boldsymbol{c}");
        assert_eq!(canonical("f_\\theta"), "f_{\\theta}");
        assert_eq!(canonical("\\operatorname{mean}"), "\\operatorname{mean}");
    }

    #[test]
    fn exponents_and_indices_are_not_names() {
        assert_eq!(keys("\\mathbb{R}^{N_{\\mathrm{a}}\\times D}"), vec!["\\mathbb{R}", "N_{\\mathrm{a}}", "D"]);
        assert_eq!(keys("q_{(j)}"), vec!["q"]);
        assert_eq!(keys("\\sum_{d=1}^{D} w_d"), vec!["D", "w"]);
        assert_eq!(keys("x^2 + y^{\\top}"), vec!["x", "y"]);
        assert_eq!(keys("n_{\\boldsymbol{z}}"), vec!["n_{\\boldsymbol{z}}", "\\boldsymbol{z}"]);
    }

    #[test]
    fn arguments_of_unknown_commands_are_scanned() {
        assert_eq!(keys("a\\xrightarrow{\\mathrm{head}+\\sigma} b"), vec!["a", "\\sigma", "b"]);
        assert_eq!(keys("\\frac{u}{v}"), vec!["u", "v"]);
        assert!(keys("\\text{for all } y").contains(&"y".to_string()));
        assert!(!keys("\\text{for all } y").contains(&"f".to_string()));
    }

    #[test]
    fn macros_expand_with_arguments() {
        let mut m = BTreeMap::new();
        m.insert("vx".to_string(), Macro { nargs: 0, body: "\\boldsymbol{x}".into() });
        m.insert("pair".to_string(), Macro { nargs: 2, body: "#1+#2".into() });
        let idx: Vec<String> = DEFAULT_INDEX.iter().map(|s| s.to_string()).collect();
        let toks = expand(tokenize("\\vx + \\pair{a}{\\vx}", 0), &m);
        let mut v = Vec::new();
        collect_syms(&parse(&toks, &Options { index_letters: &idx }), &mut v);
        let k: Vec<String> = v.iter().map(|s| s.key()).collect();
        assert_eq!(k, vec!["\\boldsymbol{x}", "a", "\\boldsymbol{x}"]);
    }
}
