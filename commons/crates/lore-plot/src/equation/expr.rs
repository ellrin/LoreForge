//! The safe expression language of equation figures (`docs/plot.md` §10). Its own parser and
//! evaluator — nothing is ever handed to an interpreter. `commons/ui/plot/eqcore.js` is the same
//! language in the browser (sliders); `ui/plot/eqtests.json` holds the cases both must agree on.
//!
//! Grammar (precedence low → high): `||`, `&&`, comparisons (`< <= > >= == !=`, value 1 or 0),
//! `+ -`, `* /`, unary `- + !`, `^` (right-assoc; `-x^2` = `-(x^2)`), postfix `v[i]` (1-based).
//! Atoms: numbers, names (declared variables, bound indices, `pi`, `e`), `( … )`, vectors
//! `[a, b, …]` (`[cond]` with one element is the Iverson bracket: a scalar), calls `f(…)`.
//! Binding forms (the first argument is a new name): `sum(k, 1, K, body)`, `prod(k, 1, K, body)`,
//! `vec(k, 1, K, body)`, `let(name, value, body)`, `E(x, normal(m, s), body)` (expectation by a
//! fixed-seed Monte Carlo of [`MC_SAMPLES`] draws; also `uniform(a, b)`, `bernoulli(p)`,
//! `categorical(pvec)` → index 1..K).
//! Limits: [`MAX_LEN`] characters, [`MAX_DEPTH`] nesting, [`MAX_VEC`] elements, [`BUDGET`] steps per
//! evaluation (sums, nested sums and expectations included).

use std::collections::HashMap;

pub const MAX_LEN: usize = 4000;
pub const MAX_DEPTH: usize = 48;
pub const MAX_VEC: usize = 4096;
pub const BUDGET: u64 = 2_000_000;
pub const MC_SAMPLES: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Num(f64),
    Name(String),
    Vector(Vec<Node>),
    Index(Box<Node>, Box<Node>),
    Neg(Box<Node>),
    Not(Box<Node>),
    Bin(Op, Box<Node>, Box<Node>),
    Call(String, Vec<Node>),
    /// sum / prod / vec over an integer index
    Loop(LoopKind, String, Box<Node>, Box<Node>, Box<Node>),
    Let(String, Box<Node>, Box<Node>),
    /// E(x, dist(args), body)
    Expect(String, String, Vec<Node>, Box<Node>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    Sum,
    Prod,
    Vec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

/// Functions callable in an expression: (name, min args, max args; usize::MAX = any).
pub const FUNCTIONS: &[(&str, usize, usize)] = &[
    ("log", 1, 1), ("ln", 1, 1), ("log2", 1, 1), ("log10", 1, 1), ("log1p", 1, 1), ("exp", 1, 1), ("sqrt", 1, 1), ("abs", 1, 1),
    ("sign", 1, 1), ("floor", 1, 1), ("ceil", 1, 1), ("round", 1, 1), ("sin", 1, 1), ("cos", 1, 1), ("tanh", 1, 1),
    ("sigmoid", 1, 1), ("softplus", 1, 1), ("relu", 1, 1), ("gelu", 1, 1), ("silu", 1, 1), ("erf", 1, 1), ("sg", 1, 1),
    ("ind", 1, 1), ("pow", 2, 2), ("clamp", 3, 3), ("if", 3, 3), ("piecewise", 3, usize::MAX),
    ("min", 1, usize::MAX), ("max", 1, usize::MAX), ("sum", 1, 1), ("prod", 1, 1), ("mean", 1, 1), ("len", 1, 1),
    ("argmax", 1, 1), ("argmin", 1, 1), ("dot", 2, 2), ("norm", 1, 1), ("norm1", 1, 1), ("cosine", 2, 2),
    ("softmax", 1, 1), ("logsumexp", 1, 1),
    // loss building blocks (docs/plot.md §10)
    ("ce", 1, 1), ("bce", 2, 2), ("focal_w", 2, 2), ("focal", 3, 3), ("dice", 2, 3), ("huber", 2, 2), ("mse", 1, 2),
    ("mae", 1, 2), ("kl_gauss", 4, 4), ("info_nce", 3, 3), ("triplet", 3, 3), ("smooth_ce", 3, 3), ("hinge", 1, 1),
];
pub const BINDERS: &[&str] = &["sum", "prod", "vec", "let", "E"];
pub const DISTRIBUTIONS: &[(&str, usize)] = &[("normal", 2), ("uniform", 2), ("bernoulli", 1), ("categorical", 1)];
pub const CONSTANTS: &[(&str, f64)] = &[("pi", std::f64::consts::PI), ("e", std::f64::consts::E)];

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExprError {
    #[error("算式太長（最多 {MAX_LEN} 字）")]
    TooLong,
    #[error("算式巢狀太深")]
    TooDeep,
    #[error("第 {0} 字：{1}")]
    Syntax(usize, String),
    #[error("不認得的名稱「{0}」")]
    UnknownName(String),
    #[error("不認得的函式「{0}」")]
    UnknownFunction(String),
    #[error("「{0}」的參數個數不對")]
    Arity(String),
    #[error("計算量太大")]
    Budget,
    #[error("{0}")]
    Eval(String),
}

// ── tokens ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Sym(&'static str),
}

fn lex(src: &str) -> Result<Vec<(usize, Tok)>, ExprError> {
    if src.chars().count() > MAX_LEN {
        return Err(ExprError::TooLong);
    }
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    const SYMS: &[&str] = &["**", "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "^", "(", ")", "[", "]", ",", "<", ">", "!", "~"];
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                i += 1;
            }
            if i < cs.len() && (cs[i] == 'e' || cs[i] == 'E') {
                let mut j = i + 1;
                if j < cs.len() && (cs[j] == '+' || cs[j] == '-') {
                    j += 1;
                }
                if j < cs.len() && cs[j].is_ascii_digit() {
                    i = j;
                    while i < cs.len() && cs[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let t: String = cs[start..i].iter().collect();
            let v: f64 = t.parse().map_err(|_| ExprError::Syntax(start + 1, format!("數字「{t}」不對")))?;
            out.push((start, Tok::Num(v)));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_ascii_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            out.push((start, Tok::Name(cs[start..i].iter().collect())));
            continue;
        }
        let rest: String = cs[i..(i + 2).min(cs.len())].iter().collect();
        match SYMS.iter().find(|s| rest.starts_with(**s)) {
            Some(s) => {
                out.push((i, Tok::Sym(if *s == "**" { "^" } else { s })));
                i += s.len();
            }
            None => return Err(ExprError::Syntax(i + 1, format!("不能用的字元「{c}」"))),
        }
    }
    Ok(out)
}

// ── parser ──────────────────────────────────────────────────────────────

struct Parser<'a> {
    toks: Vec<(usize, Tok)>,
    at: usize,
    depth: usize,
    names: Vec<String>,
    known: &'a dyn Fn(&str) -> bool,
}

/// Parse `src`; `known(name)` says which variable names exist (declared x and parameters).
pub fn parse(src: &str, known: &dyn Fn(&str) -> bool) -> Result<Node, ExprError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, at: 0, depth: 0, names: vec![], known };
    let n = p.or()?;
    if let Some((pos, t)) = p.toks.get(p.at) {
        return Err(ExprError::Syntax(pos + 1, format!("多出來的「{}」", tok_text(t))));
    }
    Ok(n)
}

/// Parse with a fixed list of variable names.
pub fn parse_with(src: &str, vars: &[&str]) -> Result<Node, ExprError> {
    parse(src, &|n| vars.contains(&n))
}

fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Num(v) => v.to_string(),
        Tok::Name(n) => n.clone(),
        Tok::Sym(s) => s.to_string(),
    }
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at).map(|t| &t.1)
    }
    fn pos(&self) -> usize {
        self.toks.get(self.at).map(|t| t.0 + 1).unwrap_or_else(|| self.toks.last().map(|t| t.0 + 2).unwrap_or(1))
    }
    fn is(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Sym(x)) if *x == s)
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<(), ExprError> {
        if self.eat(s) { Ok(()) } else { Err(ExprError::Syntax(self.pos(), format!("這裡要「{s}」"))) }
    }
    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(ExprError::TooDeep) } else { Ok(()) }
    }
    fn bin(&mut self, ops: &[(&'static str, Op)], next: fn(&mut Self) -> Result<Node, ExprError>) -> Result<Node, ExprError> {
        let mut l = next(self)?;
        'outer: loop {
            for (s, op) in ops {
                if self.eat(s) {
                    let r = next(self)?;
                    l = Node::Bin(*op, Box::new(l), Box::new(r));
                    continue 'outer;
                }
            }
            return Ok(l);
        }
    }
    fn or(&mut self) -> Result<Node, ExprError> {
        self.enter()?;
        let r = self.bin(&[("||", Op::Or)], Self::and);
        self.depth -= 1;
        r
    }
    fn and(&mut self) -> Result<Node, ExprError> {
        self.bin(&[("&&", Op::And)], Self::cmp)
    }
    fn cmp(&mut self) -> Result<Node, ExprError> {
        self.bin(&[("<=", Op::Le), (">=", Op::Ge), ("==", Op::Eq), ("!=", Op::Ne), ("<", Op::Lt), (">", Op::Gt)], Self::add)
    }
    fn add(&mut self) -> Result<Node, ExprError> {
        self.bin(&[("+", Op::Add), ("-", Op::Sub)], Self::mul)
    }
    fn mul(&mut self) -> Result<Node, ExprError> {
        self.bin(&[("*", Op::Mul), ("/", Op::Div)], Self::unary)
    }
    fn unary(&mut self) -> Result<Node, ExprError> {
        self.enter()?;
        let r = if self.eat("-") {
            self.unary().map(|n| Node::Neg(Box::new(n)))
        } else if self.eat("+") {
            self.unary()
        } else if self.eat("!") {
            self.unary().map(|n| Node::Not(Box::new(n)))
        } else {
            self.power()
        };
        self.depth -= 1;
        r
    }
    fn power(&mut self) -> Result<Node, ExprError> {
        let base = self.postfix()?;
        if self.eat("^") {
            // right-assoc; the exponent may carry its own sign: 2^-1
            let e = self.unary()?;
            return Ok(Node::Bin(Op::Pow, Box::new(base), Box::new(e)));
        }
        Ok(base)
    }
    fn postfix(&mut self) -> Result<Node, ExprError> {
        let mut n = self.atom()?;
        while self.eat("[") {
            let i = self.or()?;
            self.expect("]")?;
            n = Node::Index(Box::new(n), Box::new(i));
        }
        Ok(n)
    }
    fn args(&mut self) -> Result<Vec<Node>, ExprError> {
        let mut a = Vec::new();
        if self.eat(")") {
            return Ok(a);
        }
        loop {
            a.push(self.or()?);
            if self.eat(")") {
                return Ok(a);
            }
            self.expect(",")?;
        }
    }
    fn bound_name(&mut self, f: &str) -> Result<String, ExprError> {
        match self.peek().cloned() {
            Some(Tok::Name(n)) if !is_reserved(&n) => {
                self.at += 1;
                Ok(n)
            }
            _ => Err(ExprError::Syntax(self.pos(), format!("{f}( 的第一個參數要是新的名稱"))),
        }
    }
    fn atom(&mut self) -> Result<Node, ExprError> {
        let pos = self.pos();
        let Some((_, t)) = self.toks.get(self.at).cloned() else { return Err(ExprError::Syntax(pos, "算式沒寫完".into())) };
        self.at += 1;
        match t {
            Tok::Num(v) => Ok(Node::Num(v)),
            Tok::Sym("(") => {
                let n = self.or()?;
                self.expect(")")?;
                Ok(n)
            }
            Tok::Sym("[") => {
                self.enter()?;
                let mut items = vec![];
                if !self.eat("]") {
                    loop {
                        items.push(self.or()?);
                        if self.eat("]") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                self.depth -= 1;
                if items.len() > MAX_VEC {
                    return Err(ExprError::Eval("向量太長".into()));
                }
                // [cond] with one element: the Iverson bracket, a scalar
                Ok(if items.len() == 1 { items.pop().unwrap() } else { Node::Vector(items) })
            }
            Tok::Name(n) => {
                if self.eat("(") {
                    self.enter()?;
                    let r = self.call(&n);
                    self.depth -= 1;
                    return r;
                }
                if self.names.contains(&n) || (self.known)(&n) || CONSTANTS.iter().any(|c| c.0 == n) {
                    Ok(Node::Name(n))
                } else {
                    Err(ExprError::UnknownName(n))
                }
            }
            Tok::Sym(s) => Err(ExprError::Syntax(pos, format!("這裡不能是「{s}」"))),
        }
    }
    fn call(&mut self, f: &str) -> Result<Node, ExprError> {
        // binding forms: sum(k, a, b, body) … (sum(v) with one argument is the plain function)
        let binder = BINDERS.contains(&f) && matches!(self.toks.get(self.at + 1).map(|t| &t.1), Some(Tok::Sym(",")));
        if binder {
            let name = self.bound_name(f)?;
            self.expect(",")?;
            match f {
                "let" => {
                    let v = self.or()?;
                    self.expect(",")?;
                    self.names.push(name.clone());
                    let body = self.or();
                    self.names.pop();
                    let body = body?;
                    self.expect(")")?;
                    Ok(Node::Let(name, Box::new(v), Box::new(body)))
                }
                "E" => {
                    let Some(Tok::Name(d)) = self.peek().cloned() else { return Err(ExprError::Syntax(self.pos(), "E( 的第二個參數要是分佈，例如 normal(0, 1)".into())) };
                    let Some((_, k)) = DISTRIBUTIONS.iter().find(|x| x.0 == d) else { return Err(ExprError::UnknownFunction(d)) };
                    self.at += 1;
                    self.expect("(")?;
                    let a = self.args()?;
                    if a.len() != *k {
                        return Err(ExprError::Arity(d));
                    }
                    self.expect(",")?;
                    self.names.push(name.clone());
                    let body = self.or();
                    self.names.pop();
                    let body = body?;
                    self.expect(")")?;
                    Ok(Node::Expect(name, d, a, Box::new(body)))
                }
                _ => {
                    let lo = self.or()?;
                    self.expect(",")?;
                    let hi = self.or()?;
                    self.expect(",")?;
                    self.names.push(name.clone());
                    let body = self.or();
                    self.names.pop();
                    let body = body?;
                    self.expect(")")?;
                    let kind = match f {
                        "sum" => LoopKind::Sum,
                        "prod" => LoopKind::Prod,
                        _ => LoopKind::Vec,
                    };
                    Ok(Node::Loop(kind, name, Box::new(lo), Box::new(hi), Box::new(body)))
                }
            }
        } else {
            let a = self.args()?;
            let Some((_, lo, hi)) = FUNCTIONS.iter().find(|x| x.0 == f) else { return Err(ExprError::UnknownFunction(f.to_string())) };
            if a.len() < *lo || a.len() > *hi {
                return Err(ExprError::Arity(f.to_string()));
            }
            if f == "piecewise" && a.len() % 2 == 0 {
                return Err(ExprError::Arity(f.to_string()));
            }
            Ok(Node::Call(f.to_string(), a))
        }
    }
}

fn is_reserved(n: &str) -> bool {
    FUNCTIONS.iter().any(|f| f.0 == n) || BINDERS.contains(&n) || CONSTANTS.iter().any(|c| c.0 == n) || DISTRIBUTIONS.iter().any(|d| d.0 == n)
}

/// Names a parsed expression reads from outside (not bound inside, not constants).
pub fn free_names(n: &Node) -> Vec<String> {
    fn walk(n: &Node, bound: &mut Vec<String>, out: &mut Vec<String>) {
        match n {
            Node::Num(_) => {}
            Node::Name(s) => {
                if !bound.contains(s) && !CONSTANTS.iter().any(|c| c.0 == s) && !out.contains(s) {
                    out.push(s.clone());
                }
            }
            Node::Vector(v) | Node::Call(_, v) => v.iter().for_each(|x| walk(x, bound, out)),
            Node::Index(a, b) | Node::Bin(_, a, b) => {
                walk(a, bound, out);
                walk(b, bound, out);
            }
            Node::Neg(a) | Node::Not(a) => walk(a, bound, out),
            Node::Loop(_, k, lo, hi, body) => {
                walk(lo, bound, out);
                walk(hi, bound, out);
                bound.push(k.clone());
                walk(body, bound, out);
                bound.pop();
            }
            Node::Let(k, v, body) => {
                walk(v, bound, out);
                bound.push(k.clone());
                walk(body, bound, out);
                bound.pop();
            }
            Node::Expect(k, _, a, body) => {
                a.iter().for_each(|x| walk(x, bound, out));
                bound.push(k.clone());
                walk(body, bound, out);
                bound.pop();
            }
        }
    }
    let mut out = vec![];
    walk(n, &mut vec![], &mut out);
    out
}

// ── values ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    S(f64),
    V(Vec<f64>),
}

impl Val {
    /// A scalar (a one-element vector counts).
    pub fn scalar(&self) -> Option<f64> {
        match self {
            Val::S(x) => Some(*x),
            Val::V(v) if v.len() == 1 => Some(v[0]),
            _ => None,
        }
    }
    pub fn as_vec(&self) -> Vec<f64> {
        match self {
            Val::S(x) => vec![*x],
            Val::V(v) => v.clone(),
        }
    }
}

/// mulberry32 — the same generator as eqcore.js (u32 arithmetic), so Monte Carlo agrees.
#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(seed)
    }
    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6D2B79F5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        t ^ (t >> 14)
    }
    /// (0, 1): never 0 or 1
    pub fn uniform(&mut self) -> f64 {
        (self.next_u32() as f64 + 0.5) / 4294967296.0
    }
}

pub const SEED: u32 = 0x5EED_1234;

/// The variables an evaluation sees.
pub type Env = HashMap<String, Val>;

struct Ev<'a> {
    env: &'a Env,
    locals: Vec<(String, Val)>,
    steps: u64,
    mc_depth: u32,
}

/// erf with fractional error < 1.2e-7 (Numerical Recipes `erfcc`, Chebyshev); eqcore.js is the
/// same formula, so the two agree to rounding.
pub fn erf(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let ans = t * (-z * z - 1.26551223 + t * (1.00002368 + t * (0.37409196 + t * (0.09678418 + t * (-0.18628806 + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277))))))))).exp();
    if x >= 0.0 { 1.0 - ans } else { ans - 1.0 }
}

pub fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

pub fn softplus(x: f64) -> f64 {
    // log(1 + e^x) without overflow
    x.max(0.0) + (-x.abs()).exp().ln_1p()
}

fn logsumexp(v: &[f64]) -> f64 {
    let m = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if !m.is_finite() {
        return m;
    }
    m + v.iter().map(|x| (x - m).exp()).sum::<f64>().ln()
}

fn map1(v: Val, f: impl Fn(f64) -> f64) -> Val {
    match v {
        Val::S(x) => Val::S(f(x)),
        Val::V(xs) => Val::V(xs.into_iter().map(f).collect()),
    }
}

fn zip2(a: Val, b: Val, f: impl Fn(f64, f64) -> f64) -> Result<Val, ExprError> {
    Ok(match (a, b) {
        (Val::S(x), Val::S(y)) => Val::S(f(x, y)),
        (Val::S(x), Val::V(ys)) => Val::V(ys.into_iter().map(|y| f(x, y)).collect()),
        (Val::V(xs), Val::S(y)) => Val::V(xs.into_iter().map(|x| f(x, y)).collect()),
        (Val::V(xs), Val::V(ys)) => {
            if xs.len() != ys.len() {
                return Err(ExprError::Eval(format!("向量長度不同（{} 與 {}）", xs.len(), ys.len())));
            }
            Val::V(xs.into_iter().zip(ys).map(|(x, y)| f(x, y)).collect())
        }
    })
}

fn truth(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

fn b(x: bool) -> f64 {
    if x { 1.0 } else { 0.0 }
}

impl Ev<'_> {
    fn tick(&mut self, n: u64) -> Result<(), ExprError> {
        self.steps += n;
        if self.steps > BUDGET { Err(ExprError::Budget) } else { Ok(()) }
    }
    fn lookup(&self, n: &str) -> Result<Val, ExprError> {
        if let Some((_, v)) = self.locals.iter().rev().find(|(k, _)| k == n) {
            return Ok(v.clone());
        }
        if let Some(v) = self.env.get(n) {
            return Ok(v.clone());
        }
        if let Some((_, c)) = CONSTANTS.iter().find(|c| c.0 == n) {
            return Ok(Val::S(*c));
        }
        Err(ExprError::UnknownName(n.to_string()))
    }
    fn scalar(&mut self, n: &Node) -> Result<f64, ExprError> {
        let v = self.eval(n)?;
        v.scalar().ok_or_else(|| ExprError::Eval("這裡要一個數，不是向量".into()))
    }
    fn int(&mut self, n: &Node) -> Result<i64, ExprError> {
        let v = self.scalar(n)?;
        if !v.is_finite() {
            return Err(ExprError::Eval("範圍不是有限的數".into()));
        }
        Ok(v.round() as i64)
    }

    fn eval(&mut self, n: &Node) -> Result<Val, ExprError> {
        self.tick(1)?;
        Ok(match n {
            Node::Num(v) => Val::S(*v),
            Node::Name(s) => self.lookup(s)?,
            Node::Vector(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.extend(self.eval(it)?.as_vec());
                }
                if out.len() > MAX_VEC {
                    return Err(ExprError::Eval("向量太長".into()));
                }
                Val::V(out)
            }
            Node::Index(v, i) => {
                let v = self.eval(v)?.as_vec();
                let i = self.int(i)?;
                Val::S(if i >= 1 && (i as usize) <= v.len() { v[i as usize - 1] } else { f64::NAN })
            }
            Node::Neg(a) => map1(self.eval(a)?, |x| -x),
            Node::Not(a) => map1(self.eval(a)?, |x| b(!truth(x))),
            Node::Bin(op, l, r) => {
                let a = self.eval(l)?;
                let c = self.eval(r)?;
                match op {
                    Op::Add => zip2(a, c, |x, y| x + y)?,
                    Op::Sub => zip2(a, c, |x, y| x - y)?,
                    Op::Mul => zip2(a, c, |x, y| x * y)?,
                    Op::Div => zip2(a, c, |x, y| x / y)?,
                    Op::Pow => zip2(a, c, f64::powf)?,
                    Op::Lt => zip2(a, c, |x, y| b(x < y))?,
                    Op::Le => zip2(a, c, |x, y| b(x <= y))?,
                    Op::Gt => zip2(a, c, |x, y| b(x > y))?,
                    Op::Ge => zip2(a, c, |x, y| b(x >= y))?,
                    Op::Eq => zip2(a, c, |x, y| b(x == y))?,
                    Op::Ne => zip2(a, c, |x, y| b(x != y))?,
                    Op::And => zip2(a, c, |x, y| b(truth(x) && truth(y)))?,
                    Op::Or => zip2(a, c, |x, y| b(truth(x) || truth(y)))?,
                }
            }
            Node::Loop(kind, k, lo, hi, body) => {
                let (lo, hi) = (self.int(lo)?, self.int(hi)?);
                let count = (hi - lo + 1).max(0) as u64;
                if count as usize > MAX_VEC {
                    return Err(ExprError::Eval(format!("索引範圍太大（{count}，最多 {MAX_VEC}）")));
                }
                self.tick(count)?;
                let mut acc: Option<Val> = None;
                let mut items = vec![];
                for i in lo..=hi {
                    self.locals.push((k.clone(), Val::S(i as f64)));
                    let v = self.eval(body);
                    self.locals.pop();
                    let v = v?;
                    match kind {
                        LoopKind::Vec => items.extend(v.as_vec()),
                        LoopKind::Sum => acc = Some(match acc.take() { None => v, Some(a) => zip2(a, v, |x, y| x + y)? }),
                        LoopKind::Prod => acc = Some(match acc.take() { None => v, Some(a) => zip2(a, v, |x, y| x * y)? }),
                    }
                    if items.len() > MAX_VEC {
                        return Err(ExprError::Eval("向量太長".into()));
                    }
                }
                match kind {
                    LoopKind::Vec => Val::V(items),
                    LoopKind::Sum => acc.unwrap_or(Val::S(0.0)),
                    LoopKind::Prod => acc.unwrap_or(Val::S(1.0)),
                }
            }
            Node::Let(k, v, body) => {
                let v = self.eval(v)?;
                self.locals.push((k.clone(), v));
                let r = self.eval(body);
                self.locals.pop();
                r?
            }
            Node::Expect(k, dist, args, body) => {
                let a: Vec<Val> = args.iter().map(|x| self.eval(x)).collect::<Result<_, _>>()?;
                self.mc_depth += 1;
                let mut rng = Rng::new(SEED.wrapping_add(self.mc_depth.wrapping_mul(7919)));
                let mut acc: Option<Val> = None;
                let r = (|| {
                    for _ in 0..MC_SAMPLES {
                        let s = sample(dist, &a, &mut rng)?;
                        self.locals.push((k.clone(), Val::S(s)));
                        let v = self.eval(body);
                        self.locals.pop();
                        let v = v?;
                        acc = Some(match acc.take() { None => v, Some(x) => zip2(x, v, |p, q| p + q)? });
                    }
                    Ok::<(), ExprError>(())
                })();
                self.mc_depth -= 1;
                r?;
                map1(acc.unwrap_or(Val::S(f64::NAN)), |x| x / MC_SAMPLES as f64)
            }
            Node::Call(f, args) => self.call(f, args)?,
        })
    }

    fn call(&mut self, f: &str, args: &[Node]) -> Result<Val, ExprError> {
        // lazy ones first
        match f {
            "if" => {
                let c = self.scalar(&args[0])?;
                return self.eval(&args[if truth(c) { 1 } else { 2 }]);
            }
            "piecewise" => {
                let mut i = 0;
                while i + 1 < args.len() {
                    if truth(self.scalar(&args[i])?) {
                        return self.eval(&args[i + 1]);
                    }
                    i += 2;
                }
                return self.eval(&args[args.len() - 1]);
            }
            _ => {}
        }
        let mut a: Vec<Val> = Vec::with_capacity(args.len());
        for x in args {
            a.push(self.eval(x)?);
        }
        let n = a.iter().map(|v| if let Val::V(x) = v { x.len() as u64 } else { 1 }).sum::<u64>();
        self.tick(n)?;
        let s = |i: usize| -> Result<f64, ExprError> { a[i].scalar().ok_or_else(|| ExprError::Eval(format!("{f}( 的第 {} 個參數要是一個數", i + 1))) };
        let one = || a[0].clone();
        Ok(match f {
            "log" | "ln" => map1(one(), f64::ln),
            "log2" => map1(one(), f64::log2),
            "log10" => map1(one(), f64::log10),
            "log1p" => map1(one(), f64::ln_1p),
            "exp" => map1(one(), f64::exp),
            "sqrt" => map1(one(), f64::sqrt),
            "abs" => map1(one(), f64::abs),
            "sign" => map1(one(), |x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }),
            "floor" => map1(one(), f64::floor),
            "ceil" => map1(one(), f64::ceil),
            "round" => map1(one(), f64::round),
            "sin" => map1(one(), f64::sin),
            "cos" => map1(one(), f64::cos),
            "tanh" => map1(one(), f64::tanh),
            "sigmoid" => map1(one(), sigmoid),
            "softplus" => map1(one(), softplus),
            "relu" => map1(one(), |x| x.max(0.0)),
            "gelu" => map1(one(), |x| 0.5 * x * (1.0 + erf(x / std::f64::consts::SQRT_2))),
            "silu" => map1(one(), |x| x * sigmoid(x)),
            "erf" => map1(one(), erf),
            "sg" => one(),
            "ind" => map1(one(), |x| b(truth(x))),
            "hinge" => map1(one(), |x| (1.0 - x).max(0.0)),
            "pow" => zip2(a[0].clone(), a[1].clone(), f64::powf)?,
            "clamp" => {
                let (lo, hi) = (s(1)?, s(2)?);
                map1(one(), |x| x.max(lo).min(hi))
            }
            "min" | "max" => {
                let all: Vec<f64> = a.iter().flat_map(|v| v.as_vec()).collect();
                if all.is_empty() {
                    return Err(ExprError::Eval(format!("{f}( 沒有東西")));
                }
                let r = if f == "min" { all.iter().cloned().fold(f64::INFINITY, f64::min) } else { all.iter().cloned().fold(f64::NEG_INFINITY, f64::max) };
                Val::S(if all.iter().any(|x| x.is_nan()) { f64::NAN } else { r })
            }
            "sum" => Val::S(a[0].as_vec().iter().sum()),
            "prod" => Val::S(a[0].as_vec().iter().product()),
            "mean" => {
                let v = a[0].as_vec();
                Val::S(v.iter().sum::<f64>() / v.len() as f64)
            }
            "len" => Val::S(a[0].as_vec().len() as f64),
            "argmax" | "argmin" => {
                let v = a[0].as_vec();
                let mut best = 0;
                for (i, x) in v.iter().enumerate() {
                    if (f == "argmax" && *x > v[best]) || (f == "argmin" && *x < v[best]) {
                        best = i;
                    }
                }
                Val::S(best as f64 + 1.0)
            }
            "dot" => Val::S(zip2(a[0].clone(), a[1].clone(), |x, y| x * y)?.as_vec().iter().sum()),
            "norm" => Val::S(a[0].as_vec().iter().map(|x| x * x).sum::<f64>().sqrt()),
            "norm1" => Val::S(a[0].as_vec().iter().map(|x| x.abs()).sum()),
            "cosine" => {
                let (u, v) = (a[0].as_vec(), a[1].as_vec());
                let d: f64 = zip2(Val::V(u.clone()), Val::V(v.clone()), |x, y| x * y)?.as_vec().iter().sum();
                let nu = u.iter().map(|x| x * x).sum::<f64>().sqrt();
                let nv = v.iter().map(|x| x * x).sum::<f64>().sqrt();
                Val::S(d / (nu * nv))
            }
            "softmax" => {
                let v = a[0].as_vec();
                let l = logsumexp(&v);
                Val::V(v.iter().map(|x| (x - l).exp()).collect())
            }
            "logsumexp" => Val::S(logsumexp(&a[0].as_vec())),
            "ce" => map1(one(), |p| -p.ln()),
            "bce" => zip2(a[0].clone(), a[1].clone(), |p, y| -(y * p.ln() + (1.0 - y) * (1.0 - p).ln()))?,
            "focal_w" => zip2(a[0].clone(), a[1].clone(), |p, g| (1.0 - p).powf(g))?,
            "focal" => {
                let (g, al) = (s(1)?, s(2)?);
                map1(one(), |p| -al * (1.0 - p).powf(g) * p.ln())
            }
            "dice" => {
                // soft Dice loss 1 − (2 Σ p g + s) / (Σ p + Σ g + s)
                let sm = if a.len() > 2 { s(2)? } else { 0.0 };
                let pg: f64 = zip2(a[0].clone(), a[1].clone(), |x, y| x * y)?.as_vec().iter().sum();
                let sp: f64 = a[0].as_vec().iter().sum();
                let sg: f64 = a[1].as_vec().iter().sum();
                Val::S(1.0 - (2.0 * pg + sm) / (sp + sg + sm))
            }
            "huber" => {
                let d = s(1)?;
                map1(one(), |r| if r.abs() <= d { 0.5 * r * r } else { d * (r.abs() - 0.5 * d) })
            }
            "mse" | "mae" => {
                let r = if a.len() == 2 { zip2(a[0].clone(), a[1].clone(), |x, y| x - y)? } else { one() };
                let v = r.as_vec();
                let m = v.iter().map(|x| if f == "mse" { x * x } else { x.abs() }).sum::<f64>() / v.len() as f64;
                Val::S(m)
            }
            "kl_gauss" => {
                // KL( N(μ1, σ1²) ‖ N(μ2, σ2²) )
                let (m1, s1, m2, s2) = (s(0)?, s(1)?, s(2)?, s(3)?);
                Val::S((s2 / s1).ln() + (s1 * s1 + (m1 - m2) * (m1 - m2)) / (2.0 * s2 * s2) - 0.5)
            }
            "info_nce" => {
                // −log( e^{s⁺/τ} / (e^{s⁺/τ} + Σ_j e^{s⁻_j/τ}) )
                let (sp, tau) = (s(0)?, s(2)?);
                let mut all = vec![sp / tau];
                all.extend(a[1].as_vec().iter().map(|x| x / tau));
                Val::S(logsumexp(&all) - sp / tau)
            }
            "triplet" => Val::S((s(0)? - s(1)? + s(2)?).max(0.0)),
            "smooth_ce" => {
                // label smoothing over K classes, p on the true class, 1 − p spread evenly
                let (p, eps, k) = (s(0)?, s(1)?, s(2)?);
                let on = 1.0 - eps + eps / k;
                let off = eps / k;
                Val::S(-on * p.ln() - (k - 1.0) * off * ((1.0 - p) / (k - 1.0)).ln())
            }
            other => return Err(ExprError::UnknownFunction(other.to_string())),
        })
    }
}

fn sample(dist: &str, a: &[Val], rng: &mut Rng) -> Result<f64, ExprError> {
    let s = |i: usize| a[i].scalar().ok_or_else(|| ExprError::Eval(format!("{dist}( 的參數要是數")));
    Ok(match dist {
        "normal" => {
            let (u1, u2) = (rng.uniform(), rng.uniform());
            s(0)? + s(1)? * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        }
        "uniform" => {
            let u = rng.uniform();
            s(0)? + (s(1)? - s(0)?) * u
        }
        "bernoulli" => b(rng.uniform() < s(0)?),
        "categorical" => {
            let p = a[0].as_vec();
            let total: f64 = p.iter().sum();
            let mut u = rng.uniform() * total;
            let mut k = p.len();
            for (i, q) in p.iter().enumerate() {
                if u < *q {
                    k = i + 1;
                    break;
                }
                u -= q;
            }
            k as f64
        }
        _ => return Err(ExprError::UnknownFunction(dist.to_string())),
    })
}

/// Evaluate a parsed expression.
pub fn eval(n: &Node, env: &Env) -> Result<Val, ExprError> {
    Ev { env, locals: vec![], steps: 0, mc_depth: 0 }.eval(n)
}

/// Evaluate to one number (a vector → error).
pub fn eval_scalar(n: &Node, env: &Env) -> Result<f64, ExprError> {
    eval(n, env)?.scalar().ok_or_else(|| ExprError::Eval("算式的結果是向量，不是一個數".into()))
}

/// Parse and evaluate `src` once with scalar variables (tests, quick checks).
pub fn eval_str(src: &str, vars: &[(&str, f64)]) -> Result<f64, ExprError> {
    let names: Vec<&str> = vars.iter().map(|v| v.0).collect();
    let n = parse_with(src, &names)?;
    let env: Env = vars.iter().map(|(k, v)| (k.to_string(), Val::S(*v))).collect();
    eval_scalar(&n, &env)
}
