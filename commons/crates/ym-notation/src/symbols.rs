//! Symbol occurrences and where the text defines them.
//!
//! Definitions are found with plain heuristics, in this order of strength:
//! - structure: `$x = …$`, `$x \in \mathbb{R}^n$`, `$f: A \to B$`, the left
//!   side of every row of a display equation;
//! - wording: "$x$ denotes / represents / refers to / is the …", "let $x$ be";
//! - apposition: "the rib mask $\boldsymbol{m}$", "coefficients $a$, $b$, and $c$";
//! - introduction: "… generates $x$ from $y$", "$a$ and $b$ form …";
//! - inherited: `\theta` is introduced together with `f_\theta`;
//! - derived: `\tilde{p}^{\mathrm{f}}` when both `\tilde{p}` and `p^{\mathrm{f}}` are defined.
//!
//! By default captions and tables never define anything: a caption that uses
//! a symbol before the body text defines it is reported.

use crate::math::{Atom, Font, Sym};
use crate::scan::{Area, ItemKind, Scan};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DefKind {
    Equals,
    Value,
    Member,
    Signature,
    Denote,
    Apposition,
    Introduced,
    Inherited,
    Derived,
}

impl DefKind {
    /// Counts as a real definition sentence (for "defined twice" / "unused").
    pub fn explicit(self) -> bool {
        matches!(self, DefKind::Equals | DefKind::Denote | DefKind::Member | DefKind::Signature)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Def {
    pub key: String,
    pub kind: DefKind,
    pub meaning: Option<String>,
    /// Right-hand side for `=` / `\in` definitions (normalized).
    pub rhs: Option<String>,
    pub off: usize,
    pub seg: usize,
    pub sentence: usize,
    /// Meaning shared by a group ("… denote A and B, respectively").
    pub shared: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Top,
    Qualifier,
    Script,
}

#[derive(Debug, Clone)]
pub struct Occ {
    pub off: usize,
    pub seg: usize,
    pub role: Role,
    pub sentence: usize,
    pub area: Area,
    pub display: bool,
}

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub key: String,
    pub sample: Sym,
    pub occs: Vec<Occ>,
    pub defs: Vec<Def>,
}

impl SymbolInfo {
    pub fn first_def(&self) -> Option<&Def> {
        self.defs.iter().min_by_key(|d| d.off)
    }
    pub fn first_use(&self) -> Option<&Occ> {
        self.occs.iter().min_by_key(|o| o.off)
    }
    /// The best meaning the text gives.
    pub fn meaning(&self) -> Option<String> {
        let rank = |k: DefKind| match k {
            DefKind::Denote => 0,
            DefKind::Apposition => 1,
            DefKind::Equals | DefKind::Member | DefKind::Signature => 2,
            DefKind::Value => 3,
            _ => 4,
        };
        let mut v: Vec<&Def> = self.defs.iter().filter(|d| d.meaning.is_some()).collect();
        v.sort_by_key(|d| (rank(d.kind), d.off));
        v.first().and_then(|d| d.meaning.clone())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Symbols {
    pub table: BTreeMap<String, SymbolInfo>,
    /// Named operators used: name → offsets.
    pub operators: BTreeMap<String, Vec<usize>>,
}

#[derive(Debug, Clone)]
#[derive(Default)]
pub struct SymbolOptions {
    pub caption_defines: bool,
}


/// Keys that never need a definition.
pub fn builtin_conventional(key: &str) -> bool {
    matches!(
        key,
        "\\mathbb{R}" | "\\mathbb{N}" | "\\mathbb{Z}" | "\\mathbb{Q}" | "\\mathbb{C}" | "\\mathbb{E}" | "\\mathbb{P}" | "\\ell_{1}" | "\\ell_{2}" | "\\ell_{0}" | "\\ell_{p}"
    ) || key.starts_with("\\mathbb{R}")
}

const STOP: &[&str] = &[
    "the", "a", "an", "of", "to", "and", "or", "with", "from", "by", "using", "on", "in", "for", "as", "that", "which", "at", "than", "between", "into",
    "over", "under", "through", "via", "is", "are", "was", "were", "be", "been", "being", "when", "while", "whereas", "where", "if", "then", "both", "each",
    "every", "all", "its", "their", "this", "these", "those", "let", "only", "also", "not", "after", "before", "during", "since", "because", "so", "such",
    "here", "thus", "hence", "therefore", "we", "it", "they", "per", "about", "within", "without", "across", "against", "toward", "towards", "upon",
    "whose", "whether", "but", "however", "denote", "denotes", "fixed", "equal", "equals", "given", "approximately", "like", "via",
    "once", "until", "among", "along", "around", "near", "any", "some", "no", "our", "his", "her", "one", "two", "three", "more", "less", "most",
    "least", "other", "same", "different", "same", "use", "uses", "used", "update", "updates", "updated", "train", "trains", "trained", "pretrain",
    "pretrains", "generate", "generates", "generated", "produce", "produces", "produced", "predict", "predicts", "apply", "applies", "applied", "map",
    "maps", "yield", "yields", "pass", "passes", "passed", "receive", "receives", "combine", "combines", "contain", "contains", "define", "defines",
    "defined", "compute", "computes", "computed", "obtain", "obtains", "obtained", "replace", "replaces", "replacing", "reach", "reaches", "reached",
    "achieve", "achieves", "achieved", "remain", "remains", "becomes", "become", "has", "have", "had", "can", "may", "must", "will", "would", "should",
    "could", "do", "does", "did", "times", "plus", "minus", "versus", "vs", "e", "g", "i", "ie", "eg", "see", "fig", "table", "eq", "section",
    "increase", "increases", "increased", "decrease", "decreases", "decreased", "initialized", "processes", "process", "fix", "fixes", "select",
    "selects", "selected", "assign", "assigns", "assigned", "weight", "weights", "align", "aligns", "follow", "follows", "followed", "exceed", "exceeds",
];

const DENOTE: &[&str] = &["denote", "denotes", "represent", "represents", "designate", "designates", "indicate", "indicates", "stand", "stands"];
const INTRO_BEFORE: &[&str] = &["generates", "generate", "yields", "yield", "produces", "produce", "obtain", "obtains", "gives", "give", "introduce", "introduces", "define", "defines"];
const INTRO_AFTER: &[&str] = &["form", "forms", "constitute", "constitutes"];

fn word(it: &crate::scan::Item) -> Option<&str> {
    if let ItemKind::Word(w) = &it.kind { Some(w.as_str()) } else { None }
}

fn is_noun(w: &str) -> bool {
    let lw = w.to_lowercase();
    lw.chars().next().is_some_and(|c| c.is_alphabetic())
        && !STOP.contains(&lw.as_str())
        && !lw.ends_with("ly")
        && w.chars().all(|c| c.is_alphabetic() || c == '-' || c == '\'')
}

pub use crate::math::render;

fn ignorable(a: &Atom) -> bool {
    match a {
        Atom::Align => true,
        Atom::Other(o) => o.is_empty(),
        Atom::Bin(b) => matches!(b.as_str(), "left" | "right" | "!" | "," | ";" | ":" | " " | "quad" | "qquad" | "displaystyle" | "big" | "Big" | "bigl" | "bigr"),
        Atom::Group(g) => g.iter().all(ignorable),
        _ => false,
    }
}

fn as_rel(a: &Atom) -> Option<String> {
    match a {
        Atom::Rel(r) => Some(r.clone()),
        Atom::Group(g) => {
            let sig: Vec<&Atom> = g.iter().filter(|x| !ignorable(x)).collect();
            if let [Atom::Rel(r)] = sig.as_slice() { Some(r.clone()) } else { None }
        }
        _ => None,
    }
}

/// Skip a balanced `( … )` starting at `i`; returns the index after it.
fn skip_parens(atoms: &[Atom], i: usize) -> usize {
    if !matches!(atoms.get(i), Some(Atom::Punct('('))) {
        if let Some(Atom::Group(g)) = atoms.get(i) {
            // \!\left( … \right) arrives flat; a braced argument group is fine too
            if g.first().is_some_and(|a| matches!(a, Atom::Punct('('))) {
                return i + 1;
            }
        }
        return i;
    }
    let mut d = 0;
    for (j, a) in atoms.iter().enumerate().skip(i) {
        match a {
            Atom::Punct('(') => d += 1,
            Atom::Punct(')') => {
                d -= 1;
                if d == 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
    }
    atoms.len()
}

#[derive(Debug, Clone)]
pub struct Lhs {
    pub sym: Sym,
    pub kind: DefKind,
    pub rhs: String,
}

/// Definitions from the structure of one row: `x = …`, `a, b \in S`, `f: A \to B`.
pub fn row_lhs(row: &[Atom]) -> Vec<Lhs> {
    let atoms: Vec<&Atom> = row.iter().filter(|a| !ignorable(a)).collect();
    let flat: Vec<Atom> = atoms.iter().map(|a| (*a).clone()).collect();
    let mut syms = Vec::new();
    let mut i = 0;
    let mut out = Vec::new();
    loop {
        match flat.get(i) {
            Some(Atom::Sym(s)) => {
                syms.push(s.clone());
                i = skip_parens(&flat, i + 1);
                if let Some(Atom::Punct(',')) = flat.get(i) {
                    i += 1;
                    continue;
                }
            }
            _ => return out,
        }
        let Some(rel) = flat.get(i).and_then(as_rel) else { return out };
        let kind = match rel.as_str() {
            "=" | ":=" | "coloneqq" | "triangleq" | "defeq" | "equiv" => DefKind::Equals,
            "in" => DefKind::Member,
            ":" => DefKind::Signature,
            _ => return out,
        };
        // chained x = y = 0.1: collect y too
        let mut j = i + 1;
        if kind == DefKind::Equals {
            while let (Some(Atom::Sym(s)), Some(r)) = (flat.get(j), flat.get(j + 1).and_then(as_rel)) {
                if r != "=" {
                    break;
                }
                syms.push(s.clone());
                j += 2;
            }
        }
        let rhs_atoms: Vec<Atom> = flat[j.min(flat.len())..].to_vec();
        let rhs = render(&rhs_atoms);
        let numeric = !rhs_atoms.is_empty() && rhs_atoms.iter().all(|a| matches!(a, Atom::Num(_) | Atom::Bin(_)) || matches!(a, Atom::Punct('.')) || matches!(a, Atom::Group(g) if render(g).chars().all(|c| c.is_ascii_digit() || "{}.,-".contains(c))));
        let kind = if kind == DefKind::Equals && numeric { DefKind::Value } else { kind };
        let rhs = rhs.trim_end_matches(['.', ',']).to_string();
        for s in syms.drain(..) {
            out.push(Lhs { sym: s, kind, rhs: rhs.clone() });
        }
        return out;
    }
}

fn rows(atoms: &[Atom]) -> Vec<Vec<Atom>> {
    let mut out = vec![Vec::new()];
    for a in atoms {
        match a {
            Atom::Row => out.push(Vec::new()),
            // aligned/cases inside a group: rows of the group
            Atom::Group(g) if g.iter().any(|x| matches!(x, Atom::Row)) => {
                for r in rows(g) {
                    out.push(r);
                }
            }
            _ => out.last_mut().unwrap().push(a.clone()),
        }
    }
    out
}

/// The subject of an inline segment: its first symbol when that symbol is
/// alone or on the left of a relation.
fn subject(atoms: &[Atom]) -> Option<Sym> {
    let flat: Vec<Atom> = atoms.iter().filter(|a| !ignorable(a)).cloned().collect();
    let Some(Atom::Sym(s)) = flat.first() else { return None };
    let j = skip_parens(&flat, 1);
    match flat.get(j) {
        None => Some(s.clone()),
        Some(a) if as_rel(a).is_some() => Some(s.clone()),
        Some(Atom::Punct(',' | '.')) => Some(s.clone()),
        _ => None,
    }
}

/// `p < 0.05`: the p-value convention.
fn is_pvalue(atoms: &[Atom]) -> bool {
    let flat: Vec<&Atom> = atoms.iter().filter(|a| !ignorable(a)).collect();
    matches!(flat.as_slice(), [Atom::Sym(s), r, Atom::Num(_), ..] if s.base == "p" && s.is_bare_plain() && as_rel(r).is_some())
}

pub fn analyze(scan: &Scan, opt: &SymbolOptions) -> Symbols {
    let mut out = Symbols::default();
    let idx = &scan.opts.index_letters;
    // 1. occurrences
    for (si, seg) in scan.math.iter().enumerate() {
        if is_pvalue(&seg.atoms) {
            continue;
        }
        let mut all = Vec::new();
        walk(&seg.atoms, Role::Top, &mut all, &mut out.operators);
        for (s, role) in all {
            if s.is_bare_plain() && s.base.len() == 1 && idx.contains(&s.base) && s.sub.is_none() {
                continue;
            }
            let key = s.key();
            if builtin_conventional(&key) {
                continue;
            }
            let e = out.table.entry(key.clone()).or_insert_with(|| SymbolInfo { key, sample: s.clone(), occs: Vec::new(), defs: Vec::new() });
            e.occs.push(Occ { off: s.start, seg: si, role, sentence: seg.sentence, area: seg.ctx.area, display: seg.display });
        }
    }
    // 2. structural definitions
    let mut defs: Vec<(Def, Vec<Sym>)> = Vec::new();
    let can_define = |area: Area| opt.caption_defines || matches!(area, Area::Body | Area::Abstract | Area::Heading);
    for (si, seg) in scan.math.iter().enumerate() {
        if !can_define(seg.ctx.area) {
            continue;
        }
        for row in rows(&seg.atoms) {
            for l in row_lhs(&row) {
                defs.push((
                    Def { key: l.sym.key(), kind: l.kind, meaning: None, rhs: Some(l.rhs), off: l.sym.start, seg: si, sentence: seg.sentence, shared: false },
                    l.sym.qualifiers.clone(),
                ));
            }
            if !seg.display {
                break; // inline: only the first row
            }
        }
    }
    // 3. wording around inline segments, per sentence
    for (sn, sent) in scan.sentences.iter().enumerate() {
        let its = &scan.items[sent.items.clone()];
        let base = sent.items.start;
        let mut k = 0;
        while k < its.len() {
            let ItemKind::Math(_) = its[k].kind else {
                k += 1;
                continue;
            };
            // the run of math items joined by "," / "and" / "or"
            let rs = k;
            let mut re = k;
            let mut j = k + 1;
            while j < its.len() {
                match &its[j].kind {
                    ItemKind::Math(_) => {
                        re = j;
                        j += 1;
                    }
                    ItemKind::Punct(',') => j += 1,
                    ItemKind::Word(w) if w == "and" || w == "or" => j += 1,
                    _ => break,
                }
            }
            k = re + 1;
            let run: Vec<usize> = (rs..=re).filter_map(|x| if let ItemKind::Math(m) = its[x].kind { Some(m) } else { None }).collect();
            if run.iter().any(|m| !can_define(scan.math[*m].ctx.area) || scan.math[*m].display) {
                continue;
            }
            let subjects: Vec<(usize, Sym)> = run.iter().filter_map(|m| subject(&scan.math[*m].atoms).map(|s| (*m, s))).collect();
            if subjects.is_empty() {
                continue;
            }
            // words after the run
            let after: Vec<&str> = its[re + 1..].iter().take_while(|x| !matches!(x.kind, ItemKind::Math(_) | ItemKind::Break)).filter_map(word).take(14).collect();
            let after_l: Vec<String> = after.iter().map(|w| w.to_lowercase()).collect();
            let before: Vec<&str> = its[..rs].iter().rev().take_while(|x| matches!(x.kind, ItemKind::Word(_))).filter_map(word).collect();
            let prev_word = its[..rs].last().and_then(word);
            let has_let = its[..rs].iter().filter_map(word).any(|w| w.eq_ignore_ascii_case("let"));
            let a0 = after_l.first().map(|s| s.as_str()).unwrap_or("");
            let a1 = after_l.get(1).map(|s| s.as_str()).unwrap_or("");
            let denote_at = if DENOTE.contains(&a0) {
                Some(1)
            } else if (a0 == "refer" || a0 == "refers") && a1 == "to" {
                Some(2)
            } else if (a0 == "is" || a0 == "are") && matches!(a1, "a" | "an" | "the" | "defined" | "given" | "called" | "termed" | "referred") {
                Some(if matches!(a1, "defined" | "given" | "called" | "termed" | "referred") { 2 } else { 1 })
            } else if a0 == "be" && has_let {
                Some(1)
            } else {
                None
            };
            let shared = run.len() > 1;
            let emit = |kind: DefKind, meaning: Option<String>, defs: &mut Vec<(Def, Vec<Sym>)>| {
                for (m, s) in &subjects {
                    defs.push((
                        Def { key: s.key(), kind, meaning: meaning.clone(), rhs: None, off: s.start, seg: *m, sentence: sn, shared },
                        s.qualifiers.clone(),
                    ));
                }
            };
            if let Some(skip) = denote_at {
                let mut phrase: Vec<&str> = Vec::new();
                for x in its[re + 1..].iter().skip_while(|x| !matches!(x.kind, ItemKind::Word(_))).skip(skip) {
                    match &x.kind {
                        ItemKind::Word(w) if phrase.is_empty() && matches!(w.as_str(), "as" | "the" | "a" | "an" | "The" | "A" | "An") => {}
                        ItemKind::Word(w) if w == "respectively" => break,
                        ItemKind::Word(w) => phrase.push(w),
                        ItemKind::Punct('-') => {}
                        _ => break,
                    }
                    if phrase.len() >= 12 {
                        break;
                    }
                }
                while phrase.last().is_some_and(|w| matches!(*w, "and" | "or" | "with" | "of" | "for" | "in" | "to" | "from" | "by")) {
                    phrase.pop();
                }
                let meaning = if phrase.is_empty() { None } else { Some(phrase.join(" ")) };
                emit(DefKind::Denote, meaning, &mut defs);
                continue;
            }
            if INTRO_AFTER.contains(&a0) || prev_word.is_some_and(|w| INTRO_BEFORE.contains(&w.to_lowercase().as_str())) {
                emit(DefKind::Introduced, None, &mut defs);
                continue;
            }
            // "we set $x$" is a verb, "coordinate set $V$" a noun
            let pp = its[..rs].iter().rev().filter_map(word).nth(1).map(|w| w.to_lowercase());
            let verb_set = prev_word.is_some_and(|w| w == "set" || w == "sets") && pp.as_deref().is_none_or(|w| matches!(w, "we" | "i" | "they" | "you" | "also" | "then" | "and" | "to" | "first" | "simply" | "further"));
            if let Some(pw) = prev_word.filter(|_| !verb_set)
                && is_noun(pw) && !(its[..rs].len() == 1 && pw.chars().next().is_some_and(|c| c.is_uppercase()) && !pw.contains('\'')) {
                    let np: Vec<&str> = before.iter().take_while(|w| is_noun(w)).take(4).copied().collect();
                    let meaning: Vec<&str> = np.into_iter().rev().collect();
                    emit(DefKind::Apposition, Some(meaning.join(" ")), &mut defs);
                    continue;
                }
            // "set the smoothing coefficient to $m=0.99$": meaning of a value
            if prev_word.is_some_and(|w| w == "to") {
                let ws: Vec<&str> = its[..rs].iter().filter_map(word).collect();
                if let Some(p) = ws.iter().rposition(|w| *w == "set" || *w == "sets") {
                    let np: Vec<&str> = ws[p + 1..ws.len() - 1].iter().filter(|w| !matches!(**w, "the" | "a" | "an")).copied().collect();
                    if !np.is_empty() && np.len() <= 5 {
                        for (_m, s) in &subjects {
                            for (d, _) in defs.iter_mut().rev() {
                                if d.key == s.key() && d.off == s.start && d.meaning.is_none() {
                                    d.meaning = Some(np.join(" "));
                                }
                            }
                        }
                    }
                }
            }
            let _ = base;
        }
    }
    // attach meanings from wording to structural defs at the same spot
    let mut meanings: BTreeMap<(String, usize), String> = BTreeMap::new();
    for (d, _) in &defs {
        if let Some(m) = &d.meaning {
            meanings.entry((d.key.clone(), d.off)).or_insert(m.clone());
        }
    }
    for (d, _) in defs.iter_mut() {
        if d.meaning.is_none()
            && let Some(m) = meanings.get(&(d.key.clone(), d.off)) {
                d.meaning = Some(m.clone());
            }
    }
    // 4. store, plus inherited qualifiers
    defs.sort_by_key(|(d, _)| d.off);
    for (d, quals) in defs {
        for q in quals {
            let qk = q.key();
            if let Some(info) = out.table.get_mut(&qk)
                && !info.defs.iter().any(|x| x.off <= d.off) {
                    info.defs.push(Def { key: qk.clone(), kind: DefKind::Inherited, meaning: None, rhs: None, off: d.off, seg: d.seg, sentence: d.sentence, shared: false });
                }
        }
        if let Some(info) = out.table.get_mut(&d.key) {
            // the same spot twice (structure + wording): keep the stronger one
            if let Some(x) = info.defs.iter_mut().find(|x| x.off == d.off) {
                if x.meaning.is_none() {
                    x.meaning = d.meaning.clone();
                }
                if !x.kind.explicit() && d.kind.explicit() {
                    x.kind = d.kind;
                    x.rhs = d.rhs.clone();
                }
                continue;
            }
            info.defs.push(d);
        }
    }
    // 5. derived: \tilde{X}^{q} from \tilde{X} and X^{q}
    let keys: Vec<String> = out.table.keys().cloned().collect();
    for k in keys {
        let info = &out.table[&k];
        let s = &info.sample;
        if s.accents.is_empty() || (s.sub_name().is_none() && s.sup_name().is_none()) {
            continue;
        }
        let stem = s.stem();
        let mut plain = s.clone();
        plain.accents.clear();
        let plain_key = plain.key();
        let at = |key: &str| out.table.get(key).and_then(|i| i.first_def()).map(|d| (d.off, d.seg, d.sentence));
        if let (Some(a), Some(b)) = (at(&stem), at(&plain_key)) {
            let (off, seg, sentence) = if a.0 > b.0 { a } else { b };
            if out.table[&k].first_def().is_some_and(|d| d.off <= off) {
                continue;
            }
            out.table.get_mut(&k).unwrap().defs.push(Def { key: k.clone(), kind: DefKind::Derived, meaning: None, rhs: None, off, seg, sentence, shared: false });
        }
    }
    for info in out.table.values_mut() {
        info.defs.sort_by_key(|d| d.off);
        info.occs.sort_by_key(|o| o.off);
    }
    out
}

fn walk(atoms: &[Atom], role: Role, out: &mut Vec<(Sym, Role)>, ops: &mut BTreeMap<String, Vec<usize>>) {
    for a in atoms {
        match a {
            Atom::Sym(s) => {
                out.push((s.clone(), role));
                for q in &s.qualifiers {
                    out.push((q.clone(), Role::Qualifier));
                }
                for u in &s.script_uses {
                    out.push((u.clone(), Role::Script));
                    for q in &u.qualifiers {
                        out.push((q.clone(), Role::Qualifier));
                    }
                }
            }
            Atom::Op { name, custom: true, start } => ops.entry(name.clone()).or_default().push(*start),
            Atom::Group(g) => walk(g, role, out, ops),
            Atom::Arg(g) => walk(g, if role == Role::Top { Role::Script } else { role }, out, ops),
            _ => {}
        }
    }
}

/// Font family name for messages.
pub fn font_name(f: Font) -> &'static str {
    match f {
        Font::Plain => "斜體",
        Font::Bold => "粗體",
        Font::Cal => "花體",
        Font::Scr => "草體",
        Font::Bb => "黑板粗體",
        Font::Frak => "德文尖角體",
        Font::Sf => "無襯線",
        Font::Tt => "等寬",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syms(body: &str) -> Symbols {
        let src = format!("\\begin{{document}}\n{body}\n\\end{{document}}\n");
        analyze(&Scan::from_str("main.tex", &src), &SymbolOptions::default())
    }

    #[test]
    fn finds_definitions_by_structure_wording_and_apposition() {
        let s = syms(
            "Let $\\boldsymbol{u}\\in\\mathbb{R}^{n}$ denote a widget. The gadget $G$ turns knobs. \
             Here, $a$ and $b$ denote the left and right knobs, respectively. \
             We set the damping factor to $c=0.5$. \
             The model $h_\\phi$ is trained.",
        );
        let k = |key: &str| s.table.get(key).unwrap_or_else(|| panic!("{key}: {:?}", s.table.keys().collect::<Vec<_>>()));
        assert_eq!(k("\\boldsymbol{u}").defs[0].kind, DefKind::Member);
        assert_eq!(k("\\boldsymbol{u}").meaning().as_deref(), Some("widget"));
        assert_eq!(k("G").defs[0].kind, DefKind::Apposition);
        assert_eq!(k("G").meaning().as_deref(), Some("gadget"));
        assert!(k("a").defs[0].shared);
        assert_eq!(k("c").defs[0].kind, DefKind::Value);
        assert_eq!(k("c").meaning().as_deref(), Some("damping factor"));
        // \phi comes with h_\phi
        assert_eq!(k("\\phi").defs[0].kind, DefKind::Inherited);
        // n only appears as a dimension and is not defined
        assert!(k("n").defs.is_empty());
    }

    #[test]
    fn display_rows_define_their_left_side() {
        let s = syms("\\begin{align}\nL_{\\mathrm{a}} &= x + y,\\\\\nL &= L_{\\mathrm{a}} + 2.\n\\end{align}");
        assert_eq!(s.table["L_{\\mathrm{a}}"].defs[0].kind, DefKind::Equals);
        assert_eq!(s.table["L"].defs[0].rhs.as_deref(), Some("L_{\\mathrm{a}}+2"));
        assert!(s.table["x"].defs.is_empty());
    }

    #[test]
    fn derived_and_pvalue() {
        let s = syms("Let $\\tilde{\\boldsymbol{q}}$ denote a copy and $\\boldsymbol{q}^{\\mathrm{a}}$ denote a sample. Then $\\tilde{\\boldsymbol{q}}^{\\mathrm{a}}$ is used, with $p<0.05$.");
        assert_eq!(s.table["\\tilde{\\boldsymbol{q}}^{\\mathrm{a}}"].defs[0].kind, DefKind::Derived);
        assert!(!s.table.contains_key("p"));
    }
}
