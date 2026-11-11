//! Variables and equations, step 1 (no model): a deterministic symbol table.
//!
//! **Detection** works on the pdf.js items of each page ([`crate::text::Item`]):
//!
//! - an item is *math* when its font is a math font (Computer / Latin Modern math italic,
//!   symbols, extension, AMS, STIX / STIX Two Math, Cambria Math, XITS, TeX Gyre and New CM
//!   math, Euler; extendable in the glyph table), when it holds Unicode math alphanumerics,
//!   Greek letters or math operators and little else, or when it is a single letter in the
//!   italic (or bold) variant of an upright body font;
//! - a smaller item right after math, raised or lowered, is a super- or subscript;
//! - an accent glyph (ˆ ˜ ¯ ˙ ¨ ⃗) above a math letter is that letter's accent (by
//!   position, whatever the content order);
//! - short upright runs of digits, brackets and `= + − , |` between math items join them;
//!   an upright word inside math is an operator (`\log`, `\max`) or `\mathrm{…}`.
//!
//! Each run becomes LaTeX (`w_{i}`, `\hat{y}`, `\mathbf{x}`, `\mathcal{L}_{\mathrm{CE}}`)
//! through the glyph → LaTeX table ([`Glyphs`]: a public default `glyphs.toml` plus the
//! author's override files). Symbols are read out of that LaTeX with lore-notation's own
//! math parser, so keys are lore-notation canonical keys (`\bm{x}_i` → `\boldsymbol{x}`).
//!
//! Word's equations (Cambria Math) come as Unicode math letters: combining accents
//! (x + U+0303) become `\tilde{x}`, `~` / `∼` between symbols `\sim`, a script small word
//! (`𝓇ℯ𝒶ℓ`) or a math-italic word in a script (`𝑠𝑦𝑛`) `\mathrm{…}`, a script small letter the
//! letter itself (`\mathcal` has capitals only), a prime on the letter `x'`. A stacked
//! fraction (numerator raised, denominator lowered around a baseline, at the line's size)
//! is rebuilt as `\frac{…}{…}` by geometry, whatever the text order.
//!
//! **Display equations**: a line that is mostly math, roughly centred in its column, with
//! `(n)` at the right (or a little off the row's baseline); neighbouring centred math lines
//! join it while they have at most one number between them. Several rows are one
//! `aligned` block ([`join_rows`]); a number per row makes separate equations. Ids are
//! unique (`eq:1`, then `eq:1-2` when a bound-twice paper repeats a number). Bounding box
//! and number are recorded; the LaTeX is best effort.
//!
//! **Definitions** come from regular expressions over each sentence with math runs
//! replaced by placeholders: "where ⟦x⟧ is/denotes/represents …", "⟦x⟧ denotes …",
//! "let ⟦x⟧ be …", "we denote … by ⟦x⟧". A found definition always carries the sentence
//! (the quote) and its page; only then is a symbol 🟢 `defined`. Two different
//! definitions of one symbol in this paper make it 🔴 `conflict` — never a difference from
//! the author's own notation. Nothing found: ⚪ `unresolved`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use lore_notation::math::{self, Atom, Font, Options, Script};
use lore_notation::model::{Entry, Kind};
use serde::{Deserialize, Serialize};

use crate::text::{Item, PageText, PaperText, base_font};

pub const SYMBOLS_SCHEMA: &str = "lorereader.symbols/1";
const DEFAULT_GLYPHS: &str = include_str!("glyphs.toml");

// ── the glyph table ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
struct GlyphFile {
    #[serde(default)]
    glyphs: BTreeMap<String, String>,
    #[serde(default)]
    accents: BTreeMap<String, String>,
    #[serde(default)]
    fonts: FontLists,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct FontLists {
    #[serde(default)]
    math: Vec<String>,
}

/// Glyph → LaTeX (default + overrides), accent glyphs and extra math font names.
#[derive(Debug, Clone)]
pub struct Glyphs {
    map: HashMap<char, String>,
    accents: HashMap<char, String>,
    extra_math: Vec<String>,
}

impl Default for Glyphs {
    fn default() -> Self {
        let mut g = Glyphs { map: HashMap::new(), accents: HashMap::new(), extra_math: vec![] };
        g.apply(toml::from_str(DEFAULT_GLYPHS).expect("built-in glyphs.toml"));
        g
    }
}

impl Glyphs {
    /// The default table with each existing override file applied in order. Errors
    /// (unreadable or invalid files) are returned and that file is skipped.
    pub fn load(overrides: &[&Path]) -> (Glyphs, Vec<String>) {
        let mut g = Glyphs::default();
        let mut errs = vec![];
        for p in overrides {
            if !p.is_file() {
                continue;
            }
            match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|s| Glyphs::parse(&s)) {
                Ok(f) => g.apply(f),
                Err(e) => errs.push(format!("{}：{e}", p.display())),
            }
        }
        (g, errs)
    }

    fn parse(s: &str) -> Result<GlyphFile, String> {
        toml::from_str::<GlyphFile>(s).map_err(|e| e.to_string())
    }

    /// Check an override file's text (the editor saves only what parses).
    pub fn validate(s: &str) -> Result<usize, String> {
        let f = Glyphs::parse(s)?;
        for k in f.glyphs.keys().chain(f.accents.keys()) {
            if k.chars().count() != 1 {
                return Err(format!("「{k}」不是單一字元"));
            }
        }
        Ok(f.glyphs.len() + f.accents.len())
    }

    fn apply(&mut self, f: GlyphFile) {
        for (k, v) in f.glyphs {
            if let Some(c) = k.chars().next().filter(|_| k.chars().count() == 1) {
                self.map.insert(c, v);
            }
        }
        for (k, v) in f.accents {
            if let Some(c) = k.chars().next().filter(|_| k.chars().count() == 1) {
                self.accents.insert(c, v);
            }
        }
        self.extra_math.extend(f.fonts.math.into_iter().map(|m| m.to_uppercase().replace([' ', '-', '_'], "")));
    }

    /// LaTeX of one character outside any font style (`α` → `\alpha`, `𝐱` → `\mathbf{x}`).
    pub fn latex(&self, c: char) -> Option<String> {
        if let Some(v) = self.map.get(&c) {
            return Some(v.clone());
        }
        math_alnum(c)
    }

    pub fn accent(&self, c: char) -> Option<&str> {
        self.accents.get(&c).map(String::as_str)
    }

    pub fn is_mathy(&self, c: char) -> bool {
        (self.map.contains_key(&c) && !c.is_ascii()) || math_alnum(c).is_some()
    }
}

/// Unicode mathematical alphanumerics (U+1D400–1D7FF) → LaTeX.
pub fn math_alnum(c: char) -> Option<String> {
    let u = c as u32;
    if !(0x1D400..=0x1D7FF).contains(&u) {
        return None;
    }
    const LATIN: [&str; 13] = ["\\mathbf", "", "\\boldsymbol", "\\mathcal", "\\mathcal", "\\mathfrak", "\\mathbb", "\\mathfrak", "\\mathsf", "\\mathsf", "\\mathsf", "\\mathsf", "\\mathtt"];
    if u < 0x1D6A4 {
        let i = u - 0x1D400;
        let style = (i / 52) as usize;
        let k = i % 52;
        let letter = if k < 26 { (b'A' + k as u8) as char } else { (b'a' + (k - 26) as u8) as char };
        // \mathcal has capitals only: a script small letter (Word's 𝓍, 𝔁) is the letter
        // itself, bold when the script is bold
        if (style == 3 || style == 4) && k >= 26 {
            return Some(if style == 4 { format!("\\boldsymbol{{{letter}}}") } else { letter.to_string() });
        }
        let wrap = LATIN.get(style).copied().unwrap_or("");
        return Some(if wrap.is_empty() { letter.to_string() } else { format!("{wrap}{{{letter}}}") });
    }
    if (0x1D6A8..0x1D7CA).contains(&u) {
        const GREEK: [&str; 58] = [
            "A", "B", "\\Gamma", "\\Delta", "E", "Z", "H", "\\Theta", "I", "K", "\\Lambda", "M", "N", "\\Xi", "O", "\\Pi", "P", "\\Theta", "\\Sigma", "T", "\\Upsilon", "\\Phi", "X", "\\Psi", "\\Omega", "\\nabla",
            "\\alpha", "\\beta", "\\gamma", "\\delta", "\\varepsilon", "\\zeta", "\\eta", "\\theta", "\\iota", "\\kappa", "\\lambda", "\\mu", "\\nu", "\\xi", "o", "\\pi", "\\rho", "\\varsigma", "\\sigma", "\\tau", "\\upsilon", "\\varphi", "\\chi", "\\psi", "\\omega", "\\partial",
            "\\epsilon", "\\vartheta", "\\varkappa", "\\phi", "\\varrho", "\\varpi",
        ];
        let i = u - 0x1D6A8;
        let style = i / 58;
        let g = GREEK[(i % 58) as usize];
        // bold, italic, bold italic, sans bold, sans bold italic
        return Some(match style {
            1 => g.to_string(),
            _ => format!("\\boldsymbol{{{g}}}"),
        });
    }
    if (0x1D7CE..=0x1D7FF).contains(&u) {
        let d = (b'0' + ((u - 0x1D7CE) % 10) as u8) as char;
        return Some(d.to_string());
    }
    None
}

/// A script small letter as its plain letter (`𝓇` `𝒶` `ℯ` `ℓ` `𝔁` → r a e l x): Word sets a
/// word typed in a script style (`𝓇ℯ𝒶ℓ`) letter by letter.
fn script_small(c: char) -> Option<char> {
    let u = c as u32;
    for start in [0x1D49C + 26, 0x1D4D0 + 26] {
        if (start..start + 26).contains(&u) {
            return Some((b'a' + (u - start) as u8) as char);
        }
    }
    match c {
        'ℯ' => Some('e'),
        'ℊ' => Some('g'),
        'ℴ' => Some('o'),
        'ℓ' => Some('l'),
        _ => None,
    }
}

/// A Unicode math italic letter (`𝑠` `𝐷`, `ℎ`) as its plain letter.
fn math_italic(c: char) -> Option<char> {
    let u = c as u32;
    if (0x1D434..0x1D434 + 52).contains(&u) {
        let k = u - 0x1D434;
        return Some(if k < 26 { (b'A' + k as u8) as char } else { (b'a' + (k - 26) as u8) as char });
    }
    (c == 'ℎ').then_some('h')
}

/// Combining marks (U+0300–036F, U+20D0–20FF): Word writes x̃ as `x` + U+0303.
fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x20D0..=0x20FF)
}

// ── fonts ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontClass {
    MathItalic,
    MathBoldItalic,
    /// CMSY: operators, and calligraphic capitals.
    MathSymbol,
    MathExt,
    Blackboard,
    Fraktur,
    ScriptFont,
    /// One font for everything (STIX Two Math, Cambria Math, Latin Modern Math…).
    UnicodeMath,
    Text { italic: bool, bold: bool },
}

impl FontClass {
    pub fn is_math(self) -> bool {
        !matches!(self, FontClass::Text { .. })
    }
}

pub fn font_class(name: &str, italic: bool, bold: bool, g: &Glyphs) -> FontClass {
    let n = base_font(name).to_uppercase().replace([' ', '-', '_', ','], "");
    let has = |p: &str| n.contains(p);
    let starts = |p: &str| n.starts_with(p);
    if starts("CMMIB") || starts("CMBMI") || has("MATHBOLDITALIC") {
        return FontClass::MathBoldItalic;
    }
    if starts("CMMI") || has("LMMATHITALIC") || starts("MTMI") || starts("TXMI") || starts("PXMI") || starts("EURM") || starts("EURB") || starts("ZEUR") {
        return FontClass::MathItalic;
    }
    if starts("CMSY") || starts("CMBSY") || has("LMMATHSYMBOLS") || starts("MSAM") || starts("TXSY") || starts("PXSY") || starts("MTSY") || starts("EUSM") || starts("EUSB") {
        return if starts("EUSM") || starts("EUSB") { FontClass::ScriptFont } else { FontClass::MathSymbol };
    }
    if starts("CMEX") || has("LMMATHEXTENSION") || starts("TXEX") || starts("PXEX") || starts("MTEX") {
        return FontClass::MathExt;
    }
    if starts("MSBM") || starts("BBOLD") || starts("DSROM") || starts("BBM") || has("DOUBLESTRUCK") {
        return FontClass::Blackboard;
    }
    if starts("EUFM") || starts("EUFB") || has("FRAKTUR") {
        return FontClass::Fraktur;
    }
    if starts("RSFS") || has("SCRIPTMT") {
        return FontClass::ScriptFont;
    }
    const UNI: [&str; 12] = ["STIXMATH", "STIXTWOMATH", "STIX2MATH", "CAMBRIAMATH", "LATINMODERNMATH", "XITSMATH", "ASANAMATH", "NEWCMMATH", "FIRAMATH", "LIBERTINUSMATH", "DEJAVUMATH", "GARAMONDMATH"];
    if UNI.iter().any(|u| has(u)) || (starts("TEXGYRE") && has("MATH")) || has("STIXGENERAL") && italic {
        return FontClass::UnicodeMath;
    }
    if g.extra_math.iter().any(|m| !m.is_empty() && has(m)) {
        return FontClass::UnicodeMath;
    }
    let italic = italic || has("ITALIC") || has("OBLIQUE") || n.ends_with("IT") || n.ends_with("I") && has("TIMES") || starts("CMTI") || starts("CMMIB") || has("SLANT") || starts("CMSL");
    let bold = bold || has("BOLD") || starts("CMBX") || starts("CMB") || has("SEMIBOLD") || has("BLACK") || n.ends_with("BD");
    FontClass::Text { italic, bold }
}

// ── detection ────────────────────────────────────────────────────────────

/// One run of inline (or display) math.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MathSpan {
    pub page: u32,
    /// UTF-16 range in the page text.
    pub s: usize,
    pub e: usize,
    /// `[x0, y0, x1, y1]` as fractions of the page, top-left origin (like annotations).
    pub bbox: [f32; 4],
    pub latex: String,
    /// The display equation this run belongs to (index into `equations`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayEq {
    pub id: String,
    pub page: u32,
    /// "3", "2.1", "4a" — without parentheses.
    pub number: Option<String>,
    pub bbox: [f32; 4],
    pub latex: String,
    pub s: usize,
    pub e: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub spans: Vec<MathSpan>,
    pub equations: Vec<DisplayEq>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Text,
    Math,
    Script,
    Connector,
    Accent,
    Space,
}

const OPERATOR_WORDS: &[&str] = &["log", "ln", "exp", "sin", "cos", "tan", "tanh", "sinh", "cosh", "max", "min", "sup", "inf", "arg", "argmax", "argmin", "lim", "det", "dim", "ker", "deg", "Pr", "mod", "softmax", "sign", "diag", "tr", "var", "Var", "Cov", "E"];

fn weighted_mode<K: std::hash::Hash + Eq + Clone>(xs: impl Iterator<Item = (K, usize)>) -> Option<K> {
    let mut m: HashMap<K, usize> = HashMap::new();
    for (k, w) in xs {
        *m.entry(k).or_default() += w;
    }
    m.into_iter().max_by_key(|(_, w)| *w).map(|(k, _)| k)
}

fn is_connector_text(t: &str) -> bool {
    let t = t.trim();
    !t.is_empty() && t.chars().count() <= 8 && t.chars().all(|c| "=+-−()[]{}|,.:;/<>!'0123456789 ".contains(c))
}

struct PageCtx<'a> {
    p: &'a PageText,
    /// The page's items, with math characters split out of body-text items (pdflatex
    /// puts upright Greek capitals and accent glyphs in the text font: "Let Π(").
    items: Vec<Item>,
    g: &'a Glyphs,
    body: f32,
    classes: Vec<FontClass>,
    roles: Vec<Role>,
    /// (item, char index) → accent name
    accents: HashMap<(usize, usize), String>,
    /// Stacked fractions (display `\frac`): first numerator item → the fraction.
    fracs: HashMap<usize, Frac>,
    /// Numerator / denominator item → the first numerator item of its fraction.
    frac_of: HashMap<usize, usize>,
}

/// A fraction read from its layout: numerator items above the line's baseline, the
/// denominator below, both at the line's size (the bar is a rule, not text).
#[derive(Debug, Clone)]
struct Frac {
    num: Vec<usize>,
    den: Vec<usize>,
    /// Baseline of the line the fraction sits on.
    base: f32,
    size: f32,
    /// The line's items right before and after it (where the fraction goes in a run).
    before: Option<usize>,
    after: Option<usize>,
}

impl<'a> PageCtx<'a> {
    fn new(p: &'a PageText, g: &'a Glyphs) -> PageCtx<'a> {
        let body = weighted_mode(p.items.iter().filter(|i| i.size > 0.0).map(|i| (((i.size * 2.0).round() as i32), i.str.chars().count()))).map(|k| k as f32 / 2.0).unwrap_or(10.0);
        let items = join_letters(split_items(&p.items, g), body);
        let classes: Vec<FontClass> = items.iter().map(|i| font_class(&i.font, i.italic, i.bold, g)).collect();
        let mut c = PageCtx { p, items, g, body, classes, roles: vec![], accents: HashMap::new(), fracs: HashMap::new(), frac_of: HashMap::new() };
        c.roles = c.classify();
        c.attach_accents();
        c.find_fractions();
        c
    }

    /// Stacked fractions, by geometry (pdf.js may give the numerator before the row
    /// starts): a short group of items raised above a baseline and one lowered below it,
    /// overlapping in x, both at the line's size (limits of a big operator are smaller and
    /// stay scripts), with an item of that line right before or after them.
    fn find_fractions(&mut self) {
        let items = &self.items;
        let n = items.len();
        let solid = |j: usize| !items[j].str.trim().is_empty();
        // groups: consecutive items on one baseline (their own scripts allowed)
        let mut groups: Vec<Vec<usize>> = vec![];
        let mut i = 0;
        while i < n {
            if !solid(i) || self.roles[i] == Role::Script || items[i].size < self.body * 0.6 {
                i += 1;
                continue;
            }
            let (y0, s) = (items[i].y, items[i].size);
            let mut g = vec![i];
            let mut j = i + 1;
            while j < n && g.len() < 12 {
                if !solid(j) {
                    j += 1;
                    continue;
                }
                let it = &items[j];
                let script = it.size < s * 0.9;
                if (it.y - y0).abs() < s * 0.12 && it.size >= s * 0.9 || script && (it.y - y0).abs() < s * 0.45 {
                    g.push(j);
                    j += 1;
                } else {
                    break;
                }
            }
            if g.iter().map(|&m| items[m].str.trim().chars().count()).sum::<usize>() <= 30 {
                groups.push(g);
            }
            i = j;
        }
        let span = |v: &[usize]| v.iter().fold((f32::MAX, f32::MIN), |(l, r), &m| (l.min(items[m].x), r.max(items[m].x + items[m].w)));
        // every raised / lowered pair first; a pair whose baseline item is itself part of
        // another pair is not a fraction (rows of a multi-line display line up the same way)
        let mut cands: Vec<(usize, usize, Option<usize>, Option<usize>)> = vec![];
        for (gi, num) in groups.iter().enumerate() {
            let a = &items[num[0]];
            let s = a.size;
            let (nl, nr) = span(num);
            for (gj, den) in groups.iter().enumerate() {
                if gj == gi {
                    continue;
                }
                let d = &items[den[0]];
                let drop = a.y - d.y;
                let (dl, dr) = span(den);
                if !(drop > s * 0.9 && drop < s * 2.6 && (d.size - s).abs() < s * 0.15 && nr.min(dr) - nl.max(dl) > -s * 0.2 && (nl - dl).abs() < s * 3.0) {
                    continue;
                }
                let (fl, fr) = (nl.min(dl), nr.max(dr));
                // the baseline: an item of the line between them, right before or after
                let between = |m: usize| {
                    let it = &items[m];
                    solid(m) && it.y > d.y + s * 0.25 && it.y < a.y - s * 0.25 && it.size >= s * 0.85 && s >= it.size * 0.85 && !num.contains(&m) && !den.contains(&m)
                };
                let before = (0..n).filter(|&m| between(m) && {
                    let end = items[m].x + items[m].w;
                    end <= fl + s * 0.5 && end > fl - s * 2.0
                }).max_by(|&x, &y| (items[x].x + items[x].w).partial_cmp(&(items[y].x + items[y].w)).unwrap_or(std::cmp::Ordering::Equal));
                let after = (0..n).filter(|&m| between(m) && items[m].x >= fr - s * 0.5 && items[m].x < fr + s * 2.0).min_by(|&x, &y| items[x].x.partial_cmp(&items[y].x).unwrap_or(std::cmp::Ordering::Equal));
                if before.is_some() || after.is_some() {
                    cands.push((gi, gj, before, after));
                }
            }
        }
        // small parts first; a group is in one fraction only, and a baseline item is never
        // part of a fraction already taken
        // the baseline halfway between numerator and denominator first
        let asym = |gi: usize, gj: usize, m: Option<usize>| -> f32 {
            let (a, d) = (&items[groups[gi][0]], &items[groups[gj][0]]);
            m.map_or(9.0, |m| ((a.y - items[m].y) - (items[m].y - d.y)).abs() / a.size.max(1.0))
        };
        cands.retain(|&(gi, gj, b, a)| asym(gi, gj, b.or(a)) < 0.5);
        cands.sort_by_key(|&(gi, gj, b, a)| ((asym(gi, gj, b.or(a)) * 8.0) as usize, groups[gi].len() + groups[gj].len()));
        let mut parts: BTreeSet<usize> = BTreeSet::new();
        let mut used: BTreeSet<usize> = BTreeSet::new();
        let mut found: Vec<Frac> = vec![];
        for (gi, gj, before, after) in cands {
            if used.contains(&gi) || used.contains(&gj) {
                continue;
            }
            let (before, after) = (before.filter(|b| !parts.contains(b)), after.filter(|b| !parts.contains(b)));
            let Some(m) = before.or(after) else { continue };
            used.insert(gi);
            used.insert(gj);
            parts.extend(groups[gi].iter().chain(groups[gj].iter()).copied());
            found.push(Frac { num: groups[gi].clone(), den: groups[gj].clone(), base: items[m].y, size: items[groups[gi][0]].size, before, after });
        }
        for f in found {
            // the line's items next to it are math (`) =` before a fraction is not prose)
            for m in [f.before, f.after].into_iter().flatten() {
                if self.roles[m] == Role::Text && is_connector_text(&items[m].str) {
                    self.roles[m] = Role::Connector;
                }
            }
            let start = f.num[0];
            for &x in f.num.iter().chain(f.den.iter()) {
                self.frac_of.insert(x, start);
                if matches!(self.roles[x], Role::Text | Role::Connector) {
                    self.roles[x] = Role::Math;
                }
            }
            self.fracs.insert(start, f);
        }
    }

    fn items(&self) -> &[Item] {
        &self.items
    }

    /// Body text upright? (Then a lone italic letter is a variable.)
    fn body_upright(&self) -> bool {
        let mut up = 0;
        let mut it = 0;
        for (i, x) in self.items().iter().enumerate() {
            if let FontClass::Text { italic, .. } = self.classes[i] {
                let n = x.str.chars().filter(|c| c.is_alphabetic()).count();
                if italic { it += n } else { up += n }
            }
        }
        up >= it
    }

    fn classify(&self) -> Vec<Role> {
        let items = self.items();
        let upright = self.body_upright();
        let mut roles: Vec<Role> = items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let t = it.str.trim();
                if t.is_empty() {
                    return Role::Space;
                }
                let fc = self.classes[i];
                if t.chars().count() == 1 && self.g.accent(t.chars().next().unwrap()).is_some() && (fc.is_math() || !t.is_ascii()) {
                    return Role::Accent;
                }
                if t == "•" || t == "·" && fc == FontClass::MathSymbol && items.get(i + 1).is_some_and(|n| n.str.starts_with(' ')) {
                    return Role::Text; // a list bullet
                }
                if fc.is_math() {
                    return Role::Math;
                }
                let mathy = t.chars().filter(|&c| self.g.is_mathy(c)).count();
                let letters = t.chars().filter(|c| c.is_ascii_alphabetic()).count();
                if mathy > 0 && (t.chars().count() <= 3 || letters == 0 || (mathy * 2 >= t.chars().count())) {
                    return Role::Math;
                }
                if let FontClass::Text { italic, bold } = fc {
                    let single = t.chars().filter(|c| c.is_alphabetic()).count() == 1 && t.chars().all(|c| c.is_alphabetic() || c == '\'' || c == '′');
                    if single && ((italic && upright) || bold) {
                        // not inside a run of italic / bold words
                        let neighbour_same = |j: Option<usize>| j.is_some_and(|j| self.classes[j] == fc && items[j].str.trim().chars().filter(|c| c.is_alphabetic()).count() > 1);
                        let prev = (0..i).rev().find(|&j| !items[j].str.trim().is_empty());
                        let next = (i + 1..items.len()).find(|&j| !items[j].str.trim().is_empty());
                        if !neighbour_same(prev) && !neighbour_same(next) {
                            return Role::Math;
                        }
                    }
                }
                if is_connector_text(t) {
                    return Role::Connector;
                }
                Role::Text
            })
            .collect();
        // Scripts: a smaller item right after math, raised or lowered relative to the
        // last full-size math item (the base), or continuing the script before it.
        for i in 1..items.len() {
            if matches!(roles[i], Role::Space | Role::Accent) {
                continue;
            }
            // a closing bracket in the text font carries scripts too: `(…)^{2}`
            let closing = |j: usize, roles: &[Role]| roles[j] == Role::Connector && items[j].str.trim_end().ends_with([')', ']', '|', '}']);
            let Some(prev) = (0..i).rev().find(|&j| matches!(roles[j], Role::Math | Role::Script) || closing(j, &roles)) else { continue };
            if (prev + 1..i).any(|j| !matches!(roles[j], Role::Space | Role::Accent)) {
                continue;
            }
            let Some(main) = (0..=prev).rev().take_while(|&j| matches!(roles[j], Role::Math | Role::Script | Role::Space | Role::Accent) || closing(j, &roles)).find(|&j| (roles[j] == Role::Math || closing(j, &roles)) && items[j].size > 0.0).filter(|&j| j == prev || roles[prev] == Role::Script) else { continue };
            let (m, a, b) = (&items[main], &items[prev], &items[i]);
            let base = m.size.max(1.0);
            let gap = b.x - (a.x + a.w);
            let near = gap < base * 0.6 && gap > -base * 1.5 && b.str.trim().chars().count() <= 12;
            let small = b.size < base * 0.88;
            // raised / lowered from the base, or (nested) from the script before it
            let shifted = (b.y - m.y).abs() > base * 0.12 || (roles[prev] == Role::Script && b.size < a.size * 0.9 && (b.y - a.y).abs() > a.size * 0.1);
            if near && small && shifted {
                roles[i] = Role::Script;
            }
        }
        // Connectors only between math on the same line; upright words inside math are
        // operators or \mathrm; everything else stays text.
        let n = items.len();
        for i in 0..n {
            if roles[i] != Role::Connector && roles[i] != Role::Text {
                continue;
            }
            let prev = (0..i).rev().find(|&j| !matches!(roles[j], Role::Space | Role::Accent));
            let next = (i + 1..n).find(|&j| !matches!(roles[j], Role::Space | Role::Accent));
            let mathish = |j: Option<usize>| j.is_some_and(|j| matches!(roles[j], Role::Math | Role::Script) && (items[j].y - items[i].y).abs() < self.body * 0.9);
            let word = items[i].str.trim();
            if roles[i] == Role::Connector {
                if !(mathish(prev) && (mathish(next) || word.chars().all(|c| ")]}|'".contains(c)))) {
                    roles[i] = if mathish(next) && word.chars().all(|c| "([{|-−".contains(c)) { Role::Connector } else { Role::Text };
                }
            } else if (mathish(prev) && mathish(next) && (OPERATOR_WORDS.contains(&word) || (word.len() <= 2 && word.chars().all(|c| c.is_ascii_alphabetic()) && !["or", "if", "of", "to", "in", "on", "is", "as", "at", "be", "by", "an", "we", "it"].contains(&word))) && self.classes[i] != (FontClass::Text { italic: true, bold: false }))
                || (mathish(next) && OPERATOR_WORDS.contains(&word))
            {
                roles[i] = Role::Connector;
            }
        }
        roles
    }

    fn attach_accents(&mut self) {
        let items = self.items();
        let mut found = vec![];
        let mut lone_tilde = vec![];
        for (a, ai) in items.iter().enumerate() {
            if self.roles[a] != Role::Accent {
                continue;
            }
            let c = ai.str.trim().chars().next().unwrap();
            let name = self.g.accent(c).unwrap_or("hat").to_string();
            let cx = ai.x + ai.w / 2.0;
            let best = items
                .iter()
                .enumerate()
                .filter(|(j, it)| matches!(self.roles[*j], Role::Math | Role::Text | Role::Script) && it.w > 0.0)
                .filter(|(_, it)| {
                    let dy = ai.y - it.y;
                    let pad = it.size * 0.35;
                    cx >= it.x - pad && cx <= it.x + it.w + pad && dy >= -it.size * 0.1 && dy <= it.size * 1.2
                })
                .min_by(|x, y| {
                    let d = |it: &Item| (cx - (it.x + it.w / 2.0)).abs() + (ai.y - it.y).abs() * 0.5;
                    d(x.1).partial_cmp(&d(y.1)).unwrap_or(std::cmp::Ordering::Equal)
                });
            if let Some((j, it)) = best {
                let n = it.str.chars().count().max(1);
                let k = (((cx - it.x) / it.w.max(0.1)) * n as f32).floor().clamp(0.0, (n - 1) as f32) as usize;
                found.push((a, j, k, name));
            } else if c == '~' {
                lone_tilde.push(a); // over nothing: "distributed as"
            }
        }
        for a in lone_tilde {
            self.roles[a] = Role::Math;
        }
        for (a, j, k, name) in found {
            // an accented upright letter in text is a math letter too
            if self.roles[j] == Role::Text && self.items()[j].str.trim().chars().count() == 1 {
                self.roles[j] = Role::Math;
            }
            if matches!(self.roles[j], Role::Math | Role::Script) {
                self.accents.insert((j, k), name);
                self.roles[a] = Role::Space;
            }
        }
    }

    /// LaTeX of one item's text within math.
    fn item_latex(&self, i: usize) -> String {
        let it = &self.items()[i];
        let fc = self.classes[i];
        let word = it.str.trim();
        if let FontClass::Text { italic: false, bold: false } = fc
            && word.chars().count() > 1
            && word.chars().all(|c| c.is_ascii_alphabetic())
        {
            return if OPERATOR_WORDS.contains(&word) && word != "E" { format!("\\{word} ") } else { format!("\\mathrm{{{word}}}") };
        }
        let script = self.roles.get(i) == Some(&Role::Script);
        // A word in a script set letter by letter in Word's math italic (`𝑠𝑦𝑛`, `𝑑𝑖𝑠𝑐`):
        // a name, upright (`\mathrm{syn}`), not a product of single-letter symbols.
        if script && fc == FontClass::UnicodeMath && word.chars().count() >= 3 && word.chars().all(|c| math_italic(c).is_some()) {
            return format!("\\mathrm{{{}}}", word.chars().filter_map(math_italic).collect::<String>());
        }
        let mut pieces: Vec<String> = vec![];
        let lead = it.str.len() - it.str.trim_start().len();
        let lead_chars = it.str[..lead].chars().count();
        let chars: Vec<char> = it.str.trim().chars().collect();
        let mut k = 0;
        while k < chars.len() {
            let c = chars[k];
            k += 1;
            if c.is_whitespace() {
                pieces.push(" ".into());
                continue;
            }
            // two or more script small letters in a row: a word (`𝓇ℯ𝒶ℓ` → \mathrm{real})
            if script_small(c).is_some() {
                let n = chars[k - 1..].iter().take_while(|&&x| script_small(x).is_some()).count();
                if n >= 2 {
                    let w: String = chars[k - 1..k - 1 + n].iter().filter_map(|&x| script_small(x)).collect();
                    pieces.push(format!("\\mathrm{{{w}}}"));
                    k += n - 1;
                    continue;
                }
            }
            // a combining accent (x + U+0303) goes on the piece before it
            if is_combining(c) {
                if let (Some(acc), Some(last)) = (self.g.accent(c), pieces.pop()) {
                    pieces.push(format!("\\{acc}{{{}}}", last.trim()));
                }
                continue;
            }
            // `∼` / `~` between symbols is "distributed as" (`~` alone is a LaTeX space)
            if c == '~' || c == '∼' {
                pieces.push("\\sim ".into());
                continue;
            }
            // a prime set on the letter itself (Word's `𝔁′`): x', not a raised \prime
            if (c == '′' || c == '″') && !script {
                pieces.push(if c == '′' { "'".into() } else { "''".into() });
                continue;
            }
            let k0 = k - 1;
            if let Some(code) = tex_code(fc, &it.font, c) {
                if !code.is_empty() {
                    let mut piece = code.to_string();
                    if piece.starts_with('\\') && piece.ends_with(|x: char| x.is_ascii_alphabetic()) {
                        piece.push(' ');
                    }
                    if let Some(acc) = self.accents.get(&(i, k0 + lead_chars)) {
                        piece = format!("\\{acc}{{{}}}", piece.trim());
                    }
                    pieces.push(piece);
                }
                continue; // "7" in CMSY is \mapstochar (the bar of ↦): nothing
            }
            if fc == FontClass::MathExt
                && let Some(op) = cmex(c)
            {
                if !op.is_empty() {
                    pieces.push(format!("{op} "));
                }
                continue;
            }
            let mut piece = match (fc, c) {
                // Computer Modern's \phi is named "phi" in old fonts (→ φ) and maps to ϕ in
                // newer ones; \varphi cannot be told apart by pdf.js, so both are \phi.
                (FontClass::MathItalic, 'φ' | 'ϕ') if base_font(&it.font).to_uppercase().starts_with("CMMI") => Some("\\phi ".to_string()),
                _ => self.g.latex(c),
            };
            let mut piece = match piece.take() {
                Some(l) => l,
                None => c.to_string(),
            };
            if piece.starts_with('\\') && piece.chars().last().is_some_and(|x| x.is_ascii_alphabetic()) {
                piece.push(' ');
            }
            let letter = c.is_alphabetic();
            piece = match fc {
                FontClass::MathBoldItalic if letter => format!("\\boldsymbol{{{}}}", piece.trim()),
                FontClass::MathSymbol if c.is_ascii_uppercase() => format!("\\mathcal{{{c}}}"),
                FontClass::ScriptFont if c.is_ascii_uppercase() => format!("\\mathcal{{{c}}}"),
                FontClass::Blackboard if c.is_ascii_alphabetic() => format!("\\mathbb{{{c}}}"),
                FontClass::Fraktur if c.is_ascii_alphabetic() => format!("\\mathfrak{{{c}}}"),
                FontClass::Text { bold: true, .. } if letter => format!("\\mathbf{{{}}}", piece.trim()),
                FontClass::Text { italic: false, .. } if c.is_ascii_alphabetic() => format!("\\mathrm{{{c}}}"),
                _ => piece,
            };
            if let Some(acc) = self.accents.get(&(i, k0 + lead_chars)) {
                piece = format!("\\{acc}{{{}}}", piece.trim());
            }
            pieces.push(piece);
        }
        pieces.concat()
    }

    /// Runs of math items: (first, last) item indexes.
    fn runs(&self) -> Vec<Vec<usize>> {
        let items = self.items();
        let mut out: Vec<Vec<usize>> = vec![];
        let mut cur: Vec<usize> = vec![];
        let close = |cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>| {
            // no trailing spaces or punctuation-only connectors
            while let Some(&l) = cur.last() {
                let t = items[l].str.trim();
                if self.roles[l] == Role::Space || (self.roles[l] == Role::Connector && t.chars().all(|c| ",.;:".contains(c))) {
                    cur.pop();
                } else {
                    break;
                }
            }
            // an unmatched closing bracket at the end belongs to the text
            if let Some(&l) = cur.last() {
                let joined: String = cur.iter().map(|&i| items[i].str.as_str()).collect();
                if self.roles[l] == Role::Connector && items[l].str.trim() == ")" && joined.matches('(').count() < joined.matches(')').count() {
                    cur.pop();
                }
            }
            while let Some(&f) = cur.first() {
                let before_frac = self.fracs.values().any(|fr| fr.before == Some(f));
                if self.roles[f] == Role::Connector && !before_frac && !items[f].str.trim().chars().any(|c| "([{|".contains(c)) || self.roles[f] == Role::Space {
                    cur.remove(0);
                } else {
                    break;
                }
            }
            if cur.iter().any(|&i| matches!(self.roles[i], Role::Math | Role::Script)) {
                out.push(std::mem::take(cur));
            }
            cur.clear();
        };
        // same line, not far to the right
        let near = |l: usize, i: usize| {
            let (a, b) = (&items[l], &items[i]);
            (b.y - a.y).abs() < self.body * 0.9 && b.x - (a.x + a.w) < self.body * 1.6 && b.x > a.x - self.body * 3.0
        };
        // fractions go into the run next to the line's item before (or after) them
        let member = |i: usize| self.frac_of.contains_key(&i);
        let mut placed: BTreeSet<usize> = BTreeSet::new();
        let frac_items = |f: &Frac| f.num.iter().chain(f.den.iter()).copied().collect::<Vec<usize>>();
        let base_of = |cur: &[usize]| cur.iter().rev().copied().find(|&m| self.roles[m] != Role::Space);
        for i in 0..items.len() {
            if member(i) {
                continue;
            }
            let r = self.roles[i];
            let joins = match r {
                Role::Math | Role::Script => true,
                Role::Connector | Role::Space => !cur.is_empty() || r == Role::Connector,
                _ => false,
            };
            if joins
                && let Some(l) = base_of(&cur)
                && !near(l, i)
                && r != Role::Script
            {
                close(&mut cur, &mut out);
            }
            if joins {
                if r == Role::Space && cur.is_empty() {
                    continue;
                }
                for (st, f) in &self.fracs {
                    if f.after == Some(i) && f.before.is_none_or(|b| !cur.contains(&b)) && placed.insert(*st) {
                        cur.extend(frac_items(f));
                    }
                }
                cur.push(i);
                for (st, f) in &self.fracs {
                    if f.before == Some(i) && placed.insert(*st) {
                        cur.extend(frac_items(f));
                    }
                }
                // a line end closes the run unless the math goes on right there (pdf.js
                // marks a line end before a lowered script)
                let goes_on = (i + 1..items.len()).find(|&j| !items[j].str.trim().is_empty() && !member(j)).is_some_and(|j| {
                    let last = base_of(&cur).unwrap_or(i);
                    matches!(self.roles[j], Role::Math | Role::Script | Role::Connector) && (self.roles[j] == Role::Script || near(last, j))
                });
                if items[i].eol && r != Role::Script && !goes_on {
                    close(&mut cur, &mut out);
                }
            } else {
                close(&mut cur, &mut out);
            }
        }
        close(&mut cur, &mut out);
        out
    }

    fn bbox(&self, idx: &[usize]) -> [f32; 4] {
        let (w, h) = (self.p.width.max(1.0), self.p.height.max(1.0));
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for &i in idx {
            let it = &self.items()[i];
            let size = it.size.max(1.0);
            let top = h - (it.y + size * 0.85);
            let bottom = h - (it.y - size * 0.25);
            b[0] = b[0].min(it.x / w);
            b[1] = b[1].min(top / h);
            b[2] = b[2].max((it.x + it.w) / w);
            b[3] = b[3].max(bottom / h);
        }
        b.map(|x| x.clamp(0.0, 1.0))
    }

    /// LaTeX of a run: scripts as `_{…}` / `^{…}` relative to the item before; a smaller
    /// script after a script nests (`P_{\theta_{t}}`).
    fn run_latex(&self, run: &[usize]) -> String {
        self.latex_of(run, true)
    }

    /// `fracs`: write stacked fractions (off inside a numerator / denominator).
    fn latex_of(&self, run: &[usize], fracs: bool) -> String {
        if run.is_empty() {
            return String::new();
        }
        let items = self.items();
        let mut out = String::new();
        let mut base: (f32, f32) = (items[run[0]].y, self.body); // (baseline, size) of the last main item
        let mut levels: Vec<(f32, f32)> = vec![]; // open script groups: (baseline, size)
        for &i in run {
            let it = &items[i];
            if let Some(f) = self.fracs.get(&i).filter(|_| fracs) {
                for _ in levels.drain(..) {
                    out.push('}');
                }
                let part = |v: &[usize]| self.latex_of(&v.iter().copied().filter(|&m| run.contains(&m)).collect::<Vec<_>>(), false);
                out += &format!("\\frac{{{}}}{{{}}}", part(&f.num), part(&f.den));
                base = (f.base, f.size.max(1.0));
                continue;
            }
            if fracs && self.frac_of.contains_key(&i) {
                continue; // inside the fraction written above
            }
            match self.roles[i] {
                Role::Space => {
                    if levels.is_empty() {
                        out.push(' ');
                    }
                }
                Role::Script => {
                    loop {
                        let (ry, rs) = levels.last().copied().unwrap_or(base);
                        if let Some(&(ty, ts)) = levels.last()
                            && (it.size - ts).abs() < ts * 0.12
                            && (it.y - ty).abs() < ts * 0.18
                        {
                            break; // same group
                        }
                        if levels.is_empty() || it.size < rs * 0.9 {
                            let kind = if it.y < ry - rs * 0.05 { '_' } else { '^' };
                            while out.ends_with(' ') {
                                out.pop();
                            }
                            out.push(kind);
                            out.push('{');
                            levels.push((it.y, it.size));
                            break;
                        }
                        levels.pop();
                        out.push('}');
                    }
                    out += self.item_latex(i).trim_start();
                }
                _ => {
                    for _ in levels.drain(..) {
                        out.push('}');
                    }
                    out += &self.item_latex(i);
                    base = (it.y, it.size.max(1.0));
                }
            }
        }
        for _ in levels.drain(..) {
            out.push('}');
        }
        // a prime is written x', as in the source (`x^{\prime}_{p}` → `x'_{p}`)
        tidy_latex(&out).replace("^{\\prime\\prime}", "''").replace("^{\\prime}", "'")
    }
}

/// One space between tokens, none after a control word before a non-letter
/// (`\theta _{t}` → `\theta_{t}`).
pub fn tidy_latex(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = flat.chars().collect();
    let mut out = String::with_capacity(flat.len());
    let mut in_word = false;
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' && in_word && chars.get(i + 1).is_some_and(|n| !n.is_ascii_alphabetic()) {
            in_word = false;
            continue;
        }
        if c == '\\' {
            in_word = chars.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic());
        } else if !c.is_ascii_alphabetic() {
            in_word = false;
        }
        out.push(c);
    }
    out
}

/// Computer Modern fonts without a ToUnicode map: pdf.js falls back to the character
/// code when it does not know a glyph name (WGAN-era pdfTeX output gives "\x0f" for
/// \epsilon in CMMI). OML (CMMI), OMS (CMSY) and OT1 (CMR) codes → LaTeX.
pub fn tex_code(fc: FontClass, font: &str, c: char) -> Option<&'static str> {
    const OML: [&str; 40] = [
        "\\varGamma", "\\varDelta", "\\varTheta", "\\varLambda", "\\varXi", "\\varPi", "\\varSigma", "\\varUpsilon", "\\varPhi", "\\varPsi", "\\varOmega",
        "\\alpha", "\\beta", "\\gamma", "\\delta", "\\epsilon", "\\zeta", "\\eta", "\\theta", "\\iota", "\\kappa", "\\lambda", "\\mu", "\\nu", "\\xi", "\\pi",
        "\\rho", "\\sigma", "\\tau", "\\upsilon", "\\phi", "\\chi", "\\psi", "\\omega", "\\varepsilon", "\\vartheta", "\\varpi", "\\varrho", "\\varsigma", "\\varphi",
    ];
    const OMS: [&str; 64] = [
        "-", "\\cdot", "\\times", "\\ast", "\\div", "\\diamond", "\\pm", "\\mp", "\\oplus", "\\ominus", "\\otimes", "\\oslash", "\\odot", "\\bigcirc", "\\circ", "\\bullet",
        "\\asymp", "\\equiv", "\\subseteq", "\\supseteq", "\\leq", "\\geq", "\\preceq", "\\succeq", "\\sim", "\\approx", "\\subset", "\\supset", "\\ll", "\\gg", "\\prec", "\\succ",
        "\\leftarrow", "\\rightarrow", "\\uparrow", "\\downarrow", "\\leftrightarrow", "\\nearrow", "\\searrow", "\\simeq", "\\Leftarrow", "\\Rightarrow", "\\Uparrow", "\\Downarrow", "\\Leftrightarrow", "\\nwarrow", "\\swarrow", "\\propto",
        "\\prime", "\\infty", "\\in", "\\ni", "\\bigtriangleup", "\\bigtriangledown", "/", "", "\\forall", "\\exists", "\\neg", "\\emptyset", "\\Re", "\\Im", "\\top", "\\perp",
    ];
    let u = c as u32;
    let base = base_font(font).to_uppercase();
    match fc {
        FontClass::MathItalic | FontClass::MathBoldItalic if base.starts_with("CMMI") || base.starts_with("CMBMI") => match u {
            0x00..=0x1F | 0x21..=0x27 => Some(OML[if u < 0x20 { u as usize } else { u as usize - 1 }]),
            0x60 => Some("\\ell"),
            0x7B => Some("\\imath"),
            0x7C => Some("\\jmath"),
            0x7D => Some("\\wp"),
            _ => None,
        },
        FontClass::MathSymbol if base.starts_with("CMSY") || base.starts_with("CMBSY") => match u {
            0x00..=0x1F | 0x21..=0x3F => OMS.get(u as usize).copied(),
            0x6A => Some("|"),
            0x6B => Some("\\|"),
            0x72 => Some("\\nabla"),
            _ => None,
        },
        FontClass::Text { .. } if base.starts_with("CMR") || base.starts_with("CMBX") || base.starts_with("CMSL") || base.starts_with("CMSS") => match u {
            0x00..=0x0A => Some(["\\Gamma", "\\Delta", "\\Theta", "\\Lambda", "\\Xi", "\\Pi", "\\Sigma", "\\Upsilon", "\\Phi", "\\Psi", "\\Omega"][u as usize]),
            _ => None,
        },
        _ => None,
    }
}

/// CMEX10 without a ToUnicode map: pdf.js gives the character code ("X" for the display
/// sum). Big operators by code; delimiters (brackets of every size) are kept as text.
fn cmex(c: char) -> Option<&'static str> {
    Some(match c {
        'P' | 'X' => "\\sum",
        'Q' | 'Y' => "\\prod",
        'R' | 'Z' => "\\int",
        'H' | 'I' => "\\oint",
        'S' | '[' => "\\bigcup",
        'T' | '\\' => "\\bigcap",
        'V' | '^' => "\\bigwedge",
        'W' | '_' => "\\bigvee",
        'J' | 'K' => "\\bigodot",
        'L' | 'M' => "\\bigoplus",
        'N' | 'O' => "\\bigotimes",
        'a' | 'b' => "\\coprod",
        'c'..='z' | 'A'..='G' | '0'..='9' => "",
        _ => return None,
    })
}

/// Split math characters out of text-font items: each Greek letter, math alphanumeric
/// or (non-ASCII) accent glyph becomes its own item; x is estimated by character share.
fn split_items(items: &[Item], g: &Glyphs) -> Vec<Item> {
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let fc = font_class(&it.font, it.italic, it.bold, g);
        let chars: Vec<char> = it.str.chars().collect();
        let special = |c: char| (c as u32) < 0x0B || (!c.is_ascii() && g.accent(c).is_some()) || matches!(c as u32, 0x0391..=0x03C9 | 0x03D1..=0x03F6) || math_alnum(c).is_some();
        if fc.is_math() || chars.len() <= 1 || !chars.iter().any(|&c| special(c)) {
            out.push(it.clone());
            continue;
        }
        let n = chars.len() as f32;
        let mut k = 0;
        let mut u = it.s;
        while k < chars.len() {
            let start = k;
            if special(chars[k]) {
                k += 1;
            } else {
                while k < chars.len() && !special(chars[k]) {
                    k += 1;
                }
            }
            let piece: String = chars[start..k].iter().collect();
            let len16 = piece.encode_utf16().count();
            out.push(Item { s: u, e: u + len16, str: piece, x: it.x + it.w * start as f32 / n, w: it.w * (k - start) as f32 / n, eol: it.eol && k == chars.len(), ..it.clone() });
            u += len16;
        }
    }
    out
}

/// Letters of one word set as separate items (Word: `𝓇` `ℯ` `𝒶` `ℓ`, or `𝑠` `𝑦` `𝑛` in a
/// script) back into one item: touching, same size and baseline, adjacent in the text.
fn join_letters(items: Vec<Item>, body: f32) -> Vec<Item> {
    let script_word = |t: &str| !t.is_empty() && t.chars().all(|c| script_small(c).is_some());
    let small_italic = |it: &Item| it.size < body * 0.88 && !it.str.is_empty() && it.str.chars().all(|c| math_italic(c).is_some());
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    for it in items {
        if let Some(last) = out.last_mut()
            && last.e == it.s
            && !last.eol
            && (it.size - last.size).abs() < last.size * 0.05
            && (it.y - last.y).abs() < last.size * 0.05
            && (it.x - (last.x + last.w)).abs() < last.size * 0.3
            && last.font == it.font
            && (script_word(&last.str) && script_word(&it.str) || small_italic(last) && small_italic(&it))
        {
            last.str.push_str(&it.str);
            last.e = it.e;
            last.w = it.x + it.w - last.x;
            last.eol = it.eol;
            continue;
        }
        out.push(it);
    }
    out
}

fn eq_number(t: &str) -> Option<String> {
    let t = t.trim();
    let inner = t.strip_prefix('(')?.strip_suffix(')')?.trim();
    let ok = !inner.is_empty() && inner.len() <= 6 && inner.chars().next().is_some_and(|c| c.is_ascii_digit()) && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    ok.then(|| inner.to_string())
}

/// Detect math runs and display equations on every page.
pub fn detect(paper: &PaperText, g: &Glyphs) -> Detection {
    let mut det = Detection::default();
    for p in &paper.pages {
        if p.items.is_empty() {
            continue;
        }
        let cx = PageCtx::new(p, g);
        let runs = cx.runs();
        let first_span = det.spans.len();
        for run in &runs {
            let latex = cx.run_latex(run);
            if latex.trim().is_empty() || !latex.chars().any(|c| c.is_alphabetic()) {
                continue;
            }
            let items = cx.items();
            let (s, e) = (items[run[0]].s, items[*run.last().unwrap()].e);
            det.spans.push(MathSpan { page: p.page, s, e, bbox: cx.bbox(run), latex, eq: None });
        }
        display_equations(&cx, &mut det, first_span);
    }
    // one id per equation: a paper bound twice (a manuscript and its marked-up copy)
    // repeats its numbers — the later ones get `eq:1-2`
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for q in &mut det.equations {
        let n = seen.entry(q.id.clone()).or_default();
        *n += 1;
        if *n > 1 {
            q.id = format!("{}-{n}", q.id);
        }
    }
    det
}

/// Lines of a page: clusters of item indexes by baseline (main-size items define lines;
/// smaller ones join the nearest line).
fn lines(cx: &PageCtx) -> Vec<(f32, Vec<usize>)> {
    let items = cx.items();
    let mut order: Vec<usize> = (0..items.len()).filter(|&i| cx.roles[i] != Role::Space).collect();
    order.sort_by(|&a, &b| items[b].y.partial_cmp(&items[a].y).unwrap_or(std::cmp::Ordering::Equal));
    let mut lines: Vec<(f32, Vec<usize>)> = vec![];
    for &i in &order {
        if items[i].size < cx.body * 0.88 && cx.roles[i] == Role::Script || cx.frac_of.contains_key(&i) {
            continue;
        }
        match lines.iter_mut().find(|(y, _)| (items[i].y - *y).abs() < cx.body * 0.3) {
            Some((_, v)) => v.push(i),
            None => lines.push((items[i].y, vec![i])),
        }
    }
    for &i in &order {
        if !(items[i].size < cx.body * 0.88 && cx.roles[i] == Role::Script) {
            continue;
        }
        // the line of its base (the item before it that is not a script), else the nearest
        let base = (0..i).rev().find(|&j| cx.roles[j] != Role::Space && cx.roles[j] != Role::Script && !cx.frac_of.contains_key(&j));
        let own = base.and_then(|b| lines.iter().position(|(_, v)| v.contains(&b))).filter(|&k| (items[i].y - lines[k].0).abs() < cx.body * 1.0);
        let near = || lines.iter().enumerate().filter(|(_, (y, _))| (items[i].y - *y).abs() < cx.body * 1.0).min_by(|a, b| (items[i].y - a.1.0).abs().partial_cmp(&(items[i].y - b.1.0).abs()).unwrap_or(std::cmp::Ordering::Equal)).map(|(k, _)| k);
        if let Some(k) = own.or_else(near) {
            lines[k].1.push(i);
        }
    }
    // numerators and denominators belong to the line of their fraction's baseline
    for &i in &order {
        if let Some(f) = cx.frac_of.get(&i).and_then(|f| cx.fracs.get(f))
            && let Some((_, v)) = lines.iter_mut().min_by(|a, b| (f.base - a.0).abs().partial_cmp(&(f.base - b.0).abs()).unwrap_or(std::cmp::Ordering::Equal))
        {
            v.push(i);
        }
    }
    let mut out: Vec<(f32, Vec<usize>)> = lines
        .into_iter()
        .map(|(y, mut v)| {
            v.sort_by(|&a, &b| items[a].x.partial_cmp(&items[b].x).unwrap_or(std::cmp::Ordering::Equal));
            (y, v)
        })
        .collect();
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Rows of one display equation as one LaTeX: a single row as it is; several rows as
/// `aligned` at their first relation (a row without one continues the row before).
pub fn join_rows(rows: &[String]) -> String {
    let rows: Vec<&str> = rows.iter().map(|r| r.trim()).filter(|r| !r.is_empty()).collect();
    match rows.len() {
        0 => return String::new(),
        1 => return rows[0].to_string(),
        _ => {}
    }
    let rel = |r: &str| -> Option<usize> {
        let b = r.as_bytes();
        let mut depth = 0i32;
        for (i, &c) in b.iter().enumerate() {
            match c {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                b'=' | b'<' | b'>' if depth == 0 => return Some(i),
                b'\\' if depth == 0 => {
                    for w in ["\\leq", "\\geq", "\\approx", "\\equiv", "\\propto", "\\coloneqq"] {
                        if r[i..].starts_with(w) && !r[i + w.len()..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                            return Some(i);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    };
    if rows.iter().all(|r| rel(r).is_none()) {
        return format!("\\begin{{gathered}}{}\\end{{gathered}}", rows.join(" \\\\ "));
    }
    let lines: Vec<String> = rows
        .iter()
        .map(|r| match rel(r) {
            Some(i) => format!("{} &{}", r[..i].trim_end(), &r[i..]),
            None => format!("&\\quad {r}"),
        })
        .collect();
    format!("\\begin{{aligned}}{}\\end{{aligned}}", lines.join(" \\\\ "))
}

fn display_equations(cx: &PageCtx, det: &mut Detection, first_span: usize) {
    let items = cx.items();
    let w = cx.p.width.max(1.0);
    let all = lines(cx);
    if all.is_empty() {
        return;
    }
    let ys: Vec<f32> = all.iter().map(|l| l.0).collect();
    let ls: Vec<Vec<usize>> = all.into_iter().map(|l| l.1).collect();
    let x0 = |l: &Vec<usize>| l.iter().map(|&i| items[i].x).fold(f32::MAX, f32::min);
    let x1 = |l: &Vec<usize>| l.iter().map(|&i| items[i].x + items[i].w).fold(f32::MIN, f32::max);
    // Columns: two when many lines sit wholly in each half.
    let left = ls.iter().filter(|l| x1(l) < w * 0.53).count();
    let right = ls.iter().filter(|l| x0(l) > w * 0.47).count();
    let two = left >= 8 && right >= 8;
    let column = |l: &Vec<usize>| -> (f32, f32) {
        let pick: Vec<&Vec<usize>> = if !two {
            ls.iter().collect()
        } else if x0(l) > w * 0.47 {
            ls.iter().filter(|m| x0(m) > w * 0.47).collect()
        } else {
            ls.iter().filter(|m| x1(m) < w * 0.53).collect()
        };
        let mut starts: Vec<f32> = pick.iter().map(|m| x0(m)).collect();
        let mut ends: Vec<f32> = pick.iter().map(|m| x1(m)).collect();
        starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ends.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (starts[starts.len() / 10], ends[ends.len() * 9 / 10])
    };
    let math_share = |l: &[usize]| -> f32 {
        let total: usize = l.iter().map(|&i| items[i].str.trim().chars().count()).sum();
        let math: usize = l.iter().filter(|&&i| matches!(cx.roles[i], Role::Math | Role::Script | Role::Connector)).map(|&i| items[i].str.trim().chars().count()).sum();
        if total == 0 { 0.0 } else { math as f32 / total as f32 }
    };
    let mut kinds: Vec<Option<Option<String>>> = vec![None; ls.len()]; // Some(number?) = an equation line
    for (k, l) in ls.iter().enumerate() {
        let (cl, cr) = column(l);
        let cw = (cr - cl).max(1.0);
        let last = *l.last().unwrap();
        let number = eq_number(&items[last].str).filter(|_| items[last].x + items[last].w >= cr - cw * 0.06 && l.len() > 1);
        let body: Vec<usize> = if number.is_some() { l[..l.len() - 1].to_vec() } else { l.clone() };
        if body.is_empty() {
            continue;
        }
        let (bx0, bx1) = (x0(&body), x1(&body));
        let centre = (bx0 + bx1) / 2.0;
        let centred = (centre - (cl + cr) / 2.0).abs() < cw * 0.15 && bx0 > cl + cw * 0.06;
        let share = math_share(&body);
        if (number.is_some() && share >= 0.5 && (centred || bx0 > cl + cw * 0.08)) || (centred && share >= 0.7 && bx1 - bx0 < cw * 0.85) {
            kinds[k] = Some(number);
        }
    }
    // A number on a line of its own: the row level with it is that equation (Word sets the
    // number a little off the row's baseline); else it sits between the rows of a
    // multi-line equation, and the math rows next to it are that equation.
    let mut gone = vec![false; ls.len()]; // number lines merged into their row
    for k in 0..ls.len() {
        let l = &ls[k];
        let Some(num) = (l.len() == 1).then(|| eq_number(&items[l[0]].str)).flatten() else { continue };
        let (cl, cr) = column(l);
        if items[l[0]].x + items[l[0]].w < cr - (cr - cl) * 0.06 {
            continue;
        }
        let level = (0..ls.len())
            .filter(|&r| r != k && !gone[r] && (ys[r] - ys[k]).abs() < cx.body * 0.6 && !matches!(kinds[r], Some(Some(_))) && math_share(&ls[r]) >= 0.4)
            .min_by(|&a, &b| (ys[a] - ys[k]).abs().partial_cmp(&(ys[b] - ys[k]).abs()).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(r) = level {
            kinds[r] = Some(Some(num));
            gone[k] = true;
            continue;
        }
        kinds[k] = Some(Some(num));
        for j in [k.wrapping_sub(1), k + 1] {
            if let Some(m) = ls.get(j)
                && kinds[j].is_none()
                && (ys[j] - ys[k]).abs() < cx.body * 1.6
                && math_share(m) >= 0.4
            {
                kinds[j] = Some(None);
            }
        }
    }
    // Group neighbouring equation lines; a group is one equation when it has at most one
    // number. Rows of a display with fractions are far apart: up to three body lines.
    let order: Vec<usize> = (0..ls.len()).filter(|&k| !gone[k]).collect();
    let mut p = 0;
    while p < order.len() {
        let k = order[p];
        let Some(num) = kinds[k].clone() else {
            p += 1;
            continue;
        };
        let mut members = vec![k];
        let mut number = num;
        let mut q = p + 1;
        while q < order.len() {
            let j = order[q];
            let gap = ys[order[q - 1]] - ys[j];
            match &kinds[j] {
                Some(n) if gap < cx.body * 3.0 && (number.is_none() || n.is_none()) => {
                    if number.is_none() {
                        number = n.clone();
                    }
                    members.push(j);
                    q += 1;
                }
                _ => break,
            }
        }
        p = q;
        let is_number = |i: usize| number.as_ref().is_some_and(|n| eq_number(&items[i].str).as_deref() == Some(n.as_str()));
        let mut rows: Vec<(f32, Vec<usize>)> = members.iter().map(|&m| (ys[m], ls[m].iter().copied().filter(|&i| !is_number(i)).collect::<Vec<_>>())).filter(|r| !r.1.is_empty()).collect();
        // a line of small type only (the limits under \sup, \sum) belongs to the row above or below
        let small = |r: &[usize]| r.iter().all(|&i| items[i].size < cx.body * 0.88);
        while rows.len() > 1
            && let Some(k) = rows.iter().position(|r| small(&r.1))
        {
            let y = rows[k].0;
            let (_, v) = rows.remove(k);
            if let Some(t) = (0..rows.len()).filter(|&t| !small(&rows[t].1)).min_by(|&a, &b| (rows[a].0 - y).abs().partial_cmp(&(rows[b].0 - y).abs()).unwrap_or(std::cmp::Ordering::Equal)) {
                rows[t].1.extend(v);
            } else {
                rows.insert(k, (y, v));
                break;
            }
        }
        let rows: Vec<Vec<usize>> = rows.into_iter().map(|r| r.1).collect();
        let idx: Vec<usize> = rows.iter().flatten().copied().collect();
        if idx.is_empty() {
            continue;
        }
        let s = idx.iter().map(|&i| items[i].s).min().unwrap_or(0);
        let e = idx.iter().map(|&i| items[i].e).max().unwrap_or(0);
        let eqi = det.equations.len();
        let in_eq: Vec<usize> = (first_span..det.spans.len()).filter(|&si| det.spans[si].s >= s && det.spans[si].e <= e).collect();
        // each run to the row its first item is on
        let row_latex: Vec<String> = rows
            .iter()
            .map(|r| {
                let starts: BTreeSet<usize> = r.iter().map(|&i| items[i].s).collect();
                in_eq.iter().filter(|&&si| starts.contains(&det.spans[si].s)).map(|&si| det.spans[si].latex.clone()).collect::<Vec<_>>().join(" ")
            })
            .collect();
        let placed: usize = rows.iter().map(|r| r.iter().map(|&i| items[i].s).collect::<BTreeSet<_>>()).map(|st| in_eq.iter().filter(|&&si| st.contains(&det.spans[si].s)).count()).sum();
        let latex = if placed == in_eq.len() { join_rows(&row_latex) } else { in_eq.iter().map(|&si| det.spans[si].latex.clone()).collect::<Vec<_>>().join(" ") };
        for si in &in_eq {
            det.spans[*si].eq = Some(eqi);
        }
        let id = match &number {
            Some(n) => format!("eq:{n}"),
            None => format!("eq:p{}-{}", cx.p.page, det.equations.iter().filter(|e| e.page == cx.p.page).count() + 1),
        };
        det.equations.push(DisplayEq { id, page: cx.p.page, number, bbox: cx.bbox(&idx), latex, s, e });
    }
}

// ── symbols out of the LaTeX of each run ─────────────────────────────────

/// The symbols of one LaTeX run: lore-notation canonical keys and ids, without bare
/// index letters (i, j, k, l, t, d), numbers and operators. The second value is true
/// when the run has a relation (`=`, `\in`) and the first key is its subject.
pub fn run_symbols(latex: &str) -> (Vec<(String, String)>, Option<usize>) {
    let idx: Vec<String> = math::DEFAULT_INDEX.iter().map(|s| s.to_string()).collect();
    let toks = math::tokenize(latex, 0);
    let atoms = math::parse(&toks, &Options { index_letters: &idx });
    let mut out: Vec<(String, String)> = vec![];
    let mut subject_count = None;
    let mut seen_rel = false;
    for a in &atoms {
        match a {
            Atom::Rel(_) if !seen_rel => {
                seen_rel = true;
                subject_count = Some(out.len());
            }
            Atom::Sym(s) => push_sym(s, &idx, &mut out),
            other => {
                let mut v = vec![];
                math::collect_syms(std::slice::from_ref(other), &mut v);
                for s in &v {
                    push_sym(s, &idx, &mut out);
                }
            }
        }
    }
    (out, subject_count)
}

fn push_sym(s: &math::Sym, idx: &[String], out: &mut Vec<(String, String)>) {
    if s.font == Font::Plain && s.base.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return;
    }
    if s.font == Font::Plain && s.accents.is_empty() && s.sub_name().is_none() && s.sup_name().is_none() && idx.contains(&s.base) && !matches!(s.sub, Some(Script::Name(_))) {
        return;
    }
    let key = s.key();
    if !out.iter().any(|(k, _)| *k == key) {
        out.push((key, lore_notation::extract::symbol_id(s)));
    }
    for q in s.qualifiers.iter().chain(s.script_uses.iter()) {
        push_sym(q, idx, out);
    }
}

/// Every symbol key of a detection (for precision / recall against a LaTeX source).
/// Conventional symbols (ℝ, ℕ, 𝔼, ℙ …) are left out, as lore-notation's scanner does.
pub fn detected_keys(det: &Detection) -> BTreeSet<String> {
    det.spans.iter().flat_map(|s| run_symbols(&s.latex).0.into_iter().map(|(k, _)| k)).filter(|k| !lore_notation::symbols::builtin_conventional(k)).collect()
}

/// The symbol keys lore-notation finds in a LaTeX source (the ground truth of tests).
pub fn source_keys(tex: &str) -> BTreeSet<String> {
    let scan = lore_notation::scan::Scan::from_str("paper.tex", tex);
    let syms = lore_notation::symbols::analyze(&scan, &Default::default());
    syms.table.keys().cloned().collect()
}

// ── definitions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// The sentence, as in the paper (math as `$…$`).
    pub quote: String,
    pub page: u32,
    /// program | local | claude | author
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// The defining phrase taken from the quote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
}

/// Keep a non-ASCII character in a quote unless it is a stray accent glyph.
fn det_accent(c: char) -> bool {
    !matches!(c, 'ˆ' | '˜' | '¯' | 'ˉ' | '˙' | '¨' | '˘' | 'ˇ')
}

/// Sentences of a page text: UTF-16 ranges.
pub fn sentences(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut u16 = Vec::with_capacity(chars.len() + 1);
    let mut u = 0;
    for c in &chars {
        u16.push(u);
        u += c.len_utf16();
    }
    u16.push(u);
    let mut out = vec![];
    let mut start = 0;
    let abbrev = |end: usize| -> bool {
        let w: String = chars[..end].iter().rev().take_while(|c| c.is_alphabetic() || **c == '.').collect::<Vec<_>>().into_iter().rev().collect();
        let w = w.to_lowercase();
        ["e.g", "i.e", "et al", "al", "fig", "eq", "eqs", "sec", "cf", "vs", "resp", "approx", "no", "ref", "refs", "tab"].iter().any(|a| w == *a || w.ends_with(&format!(" {a}"))) || (w.len() == 1 && chars[end - 1].is_uppercase())
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let blank = c == '\n' && chars[i + 1..].iter().take_while(|c| **c == ' ').count() + i + 1 < chars.len() && chars[i + 1..].iter().find(|c| **c != ' ') == Some(&'\n');
        let stop = matches!(c, '.' | '?' | '!') && next.is_none_or(|n| n.is_whitespace()) && !(c == '.' && abbrev(i)) && !(c == '.' && i > 0 && chars[i - 1].is_ascii_digit() && next.is_none_or(|n| n != '\n') && chars.get(i + 2).is_some_and(|d| d.is_ascii_digit()));
        if stop || blank {
            let end = i + 1;
            if chars[start..end].iter().any(|c| c.is_alphanumeric()) {
                out.push((u16[start], u16[end]));
            }
            start = end;
        }
        i += 1;
    }
    if chars[start..].iter().any(|c| c.is_alphanumeric()) {
        out.push((u16[start], u16[chars.len()]));
    }
    out
}

fn utf16_slice(text: &str, s: usize, e: usize) -> String {
    let u: Vec<u16> = text.encode_utf16().collect();
    String::from_utf16_lossy(&u[s.min(u.len())..e.min(u.len())])
}

fn norm_meaning(m: &str) -> String {
    let l = m.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric() && c != '$' && c != '\\').filter(|w| !w.is_empty() && !["the", "a", "an", "of", "for", "each", "every"].contains(w)).collect();
    words.iter().map(|w| w.strip_suffix('s').filter(|r| r.len() > 3).unwrap_or(w)).collect::<Vec<_>>().join(" ")
}

/// A definition found by the regex finder: run index → (meaning, quote).
#[derive(Debug, Clone, PartialEq)]
pub struct FoundDef {
    pub span: usize,
    pub meaning: String,
    pub quote: String,
    pub page: u32,
}

/// "where ⟦x⟧ denotes …" and friends, per sentence.
pub fn find_definitions(paper: &PaperText, det: &Detection) -> Vec<FoundDef> {
    use regex::Regex;
    let verbs = r"(?:is|are|denotes?|represents?|stands? for|refers? to|is defined as|are defined as|corresponds? to|indicates?|is given by|describes?)";
    let p_where = Regex::new(&format!(r"(?i)\b(?:where|here|in which|and|with)\s*,?\s+⟦(\d+)⟧\s*,?\s+{verbs}\s+(.+)")).unwrap();
    let p_plain = Regex::new(r"(?i)⟦(\d+)⟧\s*,?\s+(?:denotes?|represents?|stands? for|refers? to|is defined as|indicates?)\s+(.+)").unwrap();
    let p_let = Regex::new(r"(?i)\blet\s+⟦(\d+)⟧\s+(?:be|denote)\s+(.+)").unwrap();
    let p_denote = Regex::new(r"(?i)\b(?:we\s+)?(?:denote|write)\s+(.+?)\s+(?:by|as|with)\s+⟦(\d+)⟧").unwrap();
    let mut out = vec![];
    let re_mark = Regex::new(r"⟦(\d+)⟧").unwrap();
    let stop = Regex::new(r"(?i)(,\s|;|\.\s*$|\.\s|\s+and\s+⟦|\s+and\s+let\s|\s+where\s|\s+while\s|\s+respectively)").unwrap();
    let copula_ok = Regex::new(r"(?i)^(?:is|are)\s+(?:the|a|an|one|our|its|their)\b").unwrap();
    let not_def = Regex::new(r"(?i)^(?:assumed|used|known|shown|given|set|chosen|said|called|not|also|then|equal|close|small|large|bounded|continuous|differentiable|fixed|sampled|drawn|obtained|computed|defined\s+on|in|on|at|to|by|from|as)\b").unwrap();
    for p in &paper.pages {
        let spans: Vec<usize> = (0..det.spans.len()).filter(|&i| det.spans[i].page == p.page && det.spans[i].eq.is_none()).collect();
        let eqs: Vec<&DisplayEq> = det.equations.iter().filter(|q| q.page == p.page).collect();
        // a display equation inside a "sentence" splits it: "… loss is (1) where w is …"
        let mut parts = vec![];
        for (ss, se) in sentences(&p.text) {
            let mut inside: Vec<&&DisplayEq> = eqs.iter().filter(|q| q.s >= ss && q.e <= se).collect();
            inside.sort_by_key(|q| q.s);
            let mut a = ss;
            for q in inside {
                if q.s > a {
                    parts.push((a, q.s));
                }
                a = a.max(q.e);
            }
            parts.push((a, se));
        }
        for (ss, se) in parts {
            // the sentence with math runs as ⟦n⟧, and the same with $latex$ for quoting
            let mut marked = String::new();
            let mut quoted = String::new();
            let mut pos = ss;
            for &i in spans.iter().filter(|&&i| det.spans[i].s >= ss && det.spans[i].e <= se) {
                let sp = &det.spans[i];
                if sp.s < pos {
                    continue;
                }
                let t = utf16_slice(&p.text, pos, sp.s);
                marked += &t;
                quoted += &t.chars().filter(|&c| c.is_ascii() || det_accent(c)).collect::<String>();
                marked += &format!("⟦{i}⟧");
                quoted += &format!("${}$", sp.latex);
                pos = sp.e;
            }
            let t = utf16_slice(&p.text, pos, se);
            marked += &t;
            quoted += &t.chars().filter(|&c| c.is_ascii() || det_accent(c)).collect::<String>();
            let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
            let marked = flat(&marked);
            let quote = flat(&quoted);
            if !marked.contains('⟦') {
                continue;
            }
            let back = |m: &str| -> String {
                // ⟦n⟧ back to $latex$
                re_mark.replace_all(m, |c: &regex::Captures| format!("${}$", det.spans[c[1].parse::<usize>().unwrap()].latex)).to_string()
            };
            let cut = |m: &str| -> Option<String> {
                let m = m.trim();
                let end = stop.find(m).map(|x| x.start()).unwrap_or(m.len());
                let d = m[..end].trim().trim_end_matches('.').trim();
                (d.chars().filter(|c| c.is_alphabetic()).count() >= 3 && d.chars().count() <= 160).then(|| back(d))
            };
            let mut push = |span: usize, def: &str| {
                if let Some(m) = cut(def) {
                    out.push(FoundDef { span, meaning: m, quote: quote.clone(), page: p.page });
                }
            };
            for c in p_where.captures_iter(&marked) {
                let verb_and_def = c.get(0).map(|m| m.as_str()).unwrap_or("");
                let after = verb_and_def.split('⟧').nth(1).unwrap_or("").trim_start_matches([',', ' ']);
                let copula = after.starts_with("is ") || after.starts_with("are ") || after.starts_with("Is ");
                if (copula && !copula_ok.is_match(after)) || not_def.is_match(c[2].trim()) {
                    continue;
                }
                push(c[1].parse().unwrap(), &c[2]);
            }
            for c in p_plain.captures_iter(&marked) {
                push(c[1].parse().unwrap(), &c[2]);
            }
            for c in p_let.captures_iter(&marked) {
                push(c[1].parse().unwrap(), &c[2]);
            }
            for c in p_denote.captures_iter(&marked) {
                let span: usize = c[2].parse().unwrap();
                if let Some(m) = cut(&c[1]) {
                    out.push(FoundDef { span, meaning: m, quote: quote.clone(), page: p.page });
                }
            }
        }
    }
    // one definition per (span, meaning)
    let mut seen = BTreeSet::new();
    out.retain(|d| seen.insert((d.span, norm_meaning(&d.meaning))));
    out
}

// ── the per-paper symbol table ───────────────────────────────────────────

/// Where a symbol appears.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Occurrence {
    pub page: u32,
    pub s: usize,
    pub e: usize,
    pub bbox: [f32; 4],
    /// The run as LaTeX (`w_{i}`).
    pub latex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FirstUse {
    pub page: u32,
    pub sentence: String,
}

/// One row of the paper's symbol table: lore-notation's registry fields plus the
/// paper scope, the grade and who resolved it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolEntry {
    #[serde(flatten)]
    pub entry: Entry,
    /// The paper's sha256 (a symbol's meaning never crosses papers).
    pub scope: String,
    /// certain ✅ | defined 🟢 | inferred 🟡 | conflict 🔴 | unresolved ⚪
    pub grade: String,
    /// program | local | claude | author
    pub resolved_by: String,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub occurrences: Vec<Occurrence>,
    #[serde(default)]
    pub first: Option<FirstUse>,
    /// Display equations that use it (ids).
    #[serde(default)]
    pub equations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A display equation: a lore-notation `equation` entry (`label`, `latex`, `uses`) plus
/// its place in the PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquationEntry {
    #[serde(flatten)]
    pub entry: Entry,
    pub scope: String,
    pub page: u32,
    pub number: Option<String>,
    pub bbox: [f32; 4],
    pub s: usize,
    pub e: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SymbolFile {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub entries: Vec<SymbolEntry>,
    #[serde(default)]
    pub equations: Vec<EquationEntry>,
}

pub const GRADES: [&str; 5] = ["certain", "defined", "inferred", "conflict", "unresolved"];
pub const RESOLVERS: [&str; 4] = ["program", "local", "claude", "author"];

fn sentence_at(p: &PageText, at: usize) -> String {
    let (s, e) = sentences(&p.text).into_iter().find(|(s, e)| *s <= at && at < *e).unwrap_or((at, at));
    utf16_slice(&p.text, s, e).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build the table for a paper from a detection (program results only).
pub fn build(paper: &PaperText, det: &Detection, sha: &str) -> SymbolFile {
    let defs = find_definitions(paper, det);
    let mut by_key: BTreeMap<String, SymbolEntry> = BTreeMap::new();
    let mut order: Vec<String> = vec![];
    let page_of = |n: u32| paper.pages.iter().find(|p| p.page == n);
    for (si, sp) in det.spans.iter().enumerate() {
        let (syms, _) = run_symbols(&sp.latex);
        let eq = sp.eq.map(|i| det.equations[i].id.clone());
        for (key, id) in syms {
            let e = by_key.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                let mut entry = Entry::new(&id, Kind::Symbol);
                entry.latex = Some(key.clone());
                if lore_notation::symbols::builtin_conventional(&key) {
                    entry.tags.push("conventional".into());
                }
                SymbolEntry {
                    entry,
                    scope: sha.to_string(),
                    grade: "unresolved".into(),
                    resolved_by: "program".into(),
                    evidence: vec![],
                    occurrences: vec![],
                    first: page_of(sp.page).map(|p| FirstUse { page: sp.page, sentence: sentence_at(p, sp.s) }),
                    equations: vec![],
                    note: None,
                }
            });
            e.occurrences.push(Occurrence { page: sp.page, s: sp.s, e: sp.e, bbox: sp.bbox, latex: sp.latex.clone(), eq: eq.clone() });
            if let Some(q) = &eq
                && !e.equations.contains(q)
            {
                e.equations.push(q.clone());
            }
            let _ = si;
        }
    }
    // Definitions apply to a run that holds one symbol (or one subject before a relation).
    for d in &defs {
        let (syms, subj) = run_symbols(&det.spans[d.span].latex);
        let subject = match subj {
            Some(1) => Some(&syms[0]),
            None if syms.len() == 1 => Some(&syms[0]),
            _ => None,
        };
        let Some((key, _)) = subject else { continue };
        if let Some(e) = by_key.get_mut(key) {
            let same = e.evidence.iter().any(|x| x.meaning.as_deref().map(norm_meaning) == Some(norm_meaning(&d.meaning)) || x.meaning.as_deref().is_some_and(|m| norm_meaning(m).contains(&norm_meaning(&d.meaning)) || norm_meaning(&d.meaning).contains(&norm_meaning(m))));
            if same {
                continue;
            }
            e.evidence.push(Evidence { quote: d.quote.clone(), page: d.page, by: "program".into(), at: None, meaning: Some(d.meaning.clone()) });
        }
    }
    for e in by_key.values_mut() {
        match e.evidence.len() {
            0 => {}
            1 => {
                e.grade = "defined".into();
                e.entry.meaning_en = e.evidence[0].meaning.clone();
                e.entry.defined_at = Some(format!("p.{}", e.evidence[0].page));
            }
            _ => {
                e.grade = "conflict".into();
                e.entry.meaning_en = e.evidence[0].meaning.clone();
                e.entry.defined_at = Some(format!("p.{}", e.evidence[0].page));
            }
        }
    }
    let mut entries: Vec<SymbolEntry> = order.into_iter().filter_map(|k| by_key.remove(&k)).collect();
    // unique ids
    let mut ids: BTreeMap<String, usize> = BTreeMap::new();
    for e in &mut entries {
        let n = ids.entry(e.entry.id.clone()).or_default();
        *n += 1;
        if *n > 1 {
            e.entry.id = format!("{}-{n}", e.entry.id);
        }
    }
    let equations = det
        .equations
        .iter()
        .map(|q| {
            let mut entry = Entry::new(&q.id, Kind::Equation);
            entry.label = q.number.as_ref().map(|n| format!("({n})"));
            entry.latex = Some(q.latex.clone()).filter(|l| !l.is_empty());
            entry.uses = entries.iter().filter(|e| e.equations.contains(&q.id)).map(|e| e.entry.id.clone()).collect();
            EquationEntry { entry, scope: sha.to_string(), page: q.page, number: q.number.clone(), bbox: q.bbox, s: q.s, e: q.e }
        })
        .collect();
    SymbolFile { schema: SYMBOLS_SCHEMA.into(), sha256: sha.into(), generated_at: crate::now(), entries, equations }
}

/// A re-run never overwrites what the author (or Claude, or a local model) decided:
/// those entries keep their meaning, grade, evidence and note and get fresh occurrences;
/// entries only the author added are kept even when not detected again.
pub fn merge(old: Option<SymbolFile>, mut new: SymbolFile) -> SymbolFile {
    let Some(old) = old else { return new };
    let mut kept: Vec<SymbolEntry> = vec![];
    for o in old.entries {
        let latex = o.entry.latex.clone().unwrap_or_default();
        if o.resolved_by == "program" {
            continue;
        }
        match new.entries.iter_mut().find(|n| n.entry.id == o.entry.id || n.entry.latex.as_deref() == Some(latex.as_str())) {
            Some(n) => {
                let occ = std::mem::take(&mut n.occurrences);
                let eqs = std::mem::take(&mut n.equations);
                let first = n.first.take();
                let program_evidence: Vec<Evidence> = n.evidence.drain(..).filter(|e| !o.evidence.iter().any(|x| x.quote == e.quote)).collect();
                *n = SymbolEntry { occurrences: occ, equations: eqs, first, ..o };
                n.evidence.extend(program_evidence);
            }
            None => kept.push(SymbolEntry { occurrences: vec![], ..o }),
        }
    }
    new.entries.extend(kept);
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn item(s: &mut usize, text: &mut String, str_: &str, font: &str, size: f32, x: f32, y: f32, w: f32) -> Item {
        let start = *s;
        text.push_str(str_);
        *s += str_.encode_utf16().count();
        Item { s: start, e: *s, str: str_.into(), font: font.into(), size, x, y, w, h: size, eol: false, italic: false, bold: false }
    }

    /// "where w_i is the weight of pixel i, and ŷ = f(x) with 𝐱 ∈ ℝ^d." built by hand:
    /// CMR10 body, CMMI10 letters, a CMMI7 subscript, an accent over y, CMBX10 bold x.
    fn page() -> PageText {
        let mut s = 0;
        let mut t = String::new();
        let mut items = vec![];
        let r = "ABCDEF+CMR10";
        let mi = "ABCDEF+CMMI10";
        items.push(item(&mut s, &mut t, "The loss is small where ", r, 10.0, 72.0, 700.0, 110.0));
        items.push(item(&mut s, &mut t, "w", mi, 10.0, 182.0, 700.0, 7.2));
        items.push(item(&mut s, &mut t, "i", "ABCDEF+CMMI7", 7.0, 189.2, 698.5, 2.9));
        items.push(item(&mut s, &mut t, " is the weight of pixel ", r, 10.0, 194.0, 700.0, 100.0));
        items.push(item(&mut s, &mut t, "i", mi, 10.0, 294.0, 700.0, 3.4));
        items.push(item(&mut s, &mut t, ", and ", r, 10.0, 297.4, 700.0, 25.0));
        items.push(item(&mut s, &mut t, "ˆ", r, 10.0, 323.5, 703.0, 5.0));
        items.push(item(&mut s, &mut t, "y", mi, 10.0, 322.4, 700.0, 4.9));
        items.push(item(&mut s, &mut t, " = ", r, 10.0, 327.3, 700.0, 10.0));
        items.push(item(&mut s, &mut t, "f", mi, 10.0, 337.3, 700.0, 4.9));
        items.push(item(&mut s, &mut t, "(", r, 10.0, 342.2, 700.0, 3.9));
        items.push(item(&mut s, &mut t, "x", "ABCDEF+CMBX10", 10.0, 346.1, 700.0, 6.0));
        items.push(item(&mut s, &mut t, ")", r, 10.0, 352.1, 700.0, 3.9));
        items.push(item(&mut s, &mut t, " denotes the prediction.", r, 10.0, 356.0, 700.0, 100.0));
        let mut eol = items.last().unwrap().clone();
        eol.eol = true;
        *items.last_mut().unwrap() = eol;
        t.push('\n');
        s += 1;
        // a display equation: L = −∑ w_i log p_i     (1)
        items.push(item(&mut s, &mut t, "L", "CMSY10", 10.0, 260.0, 660.0, 7.0));
        items.push(item(&mut s, &mut t, " = ", r, 10.0, 267.0, 660.0, 10.0));
        items.push(item(&mut s, &mut t, "−", "CMSY10", 10.0, 277.0, 660.0, 7.8));
        items.push(item(&mut s, &mut t, "∑", "CMEX10", 10.0, 285.0, 660.0, 10.0));
        items.push(item(&mut s, &mut t, "w", mi, 10.0, 297.0, 660.0, 7.2));
        items.push(item(&mut s, &mut t, "i", "CMMI7", 7.0, 304.2, 658.5, 2.9));
        items.push(item(&mut s, &mut t, "log", r, 10.0, 310.0, 660.0, 12.0));
        items.push(item(&mut s, &mut t, "p", mi, 10.0, 324.0, 660.0, 5.0));
        items.push(item(&mut s, &mut t, "i", "CMMI7", 7.0, 329.0, 658.5, 2.9));
        items.push(item(&mut s, &mut t, "(1)", r, 10.0, 520.0, 660.0, 12.0));
        let n = items.len();
        items[n - 1].eol = true;
        t.push('\n');
        // body lines so the page has a column
        let mut y = 640.0;
        for _ in 0..12 {
            let st = t.encode_utf16().count();
            let line = "Plain body text continues across the whole column width here.";
            t.push_str(line);
            items.push(Item { s: st, e: st + line.len(), str: line.into(), font: r.into(), size: 10.0, x: 72.0, y, w: 468.0, h: 10.0, eol: true, italic: false, bold: false });
            t.push('\n');
            y -= 12.0;
        }
        PageText { page: 1, text: t, items, width: 612.0, height: 792.0 }
    }

    /// A page as Word writes equations (Cambria Math, Unicode math letters, one item per
    /// run), synthetic: a least-squares GAN loss over three rows with stacked ½ (numerator
    /// before the row in the text), a combining tilde, `~` for "distributed as", a script
    /// word in a subscript, a prime on the letter, a two-row equation whose number sits
    /// between its rows, and numbers set a little below their row's baseline.
    fn word_page() -> PageText {
        let mut s = 0;
        let mut t = String::new();
        let mut items: Vec<Item> = vec![];
        let m = "ABCDEE+Cambria Math";
        let tx = "Times New Roman";
        let mut put = |str_: &str, font: &str, size: f32, x: f32, y: f32, w: f32, eol: bool| {
            let mut it = item(&mut s, &mut t, str_, font, size, x, y, w);
            it.eol = eol;
            items.push(it);
            if eol {
                t.push('\n');
                s += 1;
            }
        };
        let mut y = 740.0;
        for _ in 0..6 {
            put("Plain body text continues across the whole column width here.", tx, 12.0, 76.0, y, 440.0, true);
            y -= 18.0;
        }
        // (1), row 1: L_disc(D_p) = ½ E_{x_p ~ P_real}[(D_p(x_p) − 1)^2] +
        put("", m, 12.0, 171.0, 620.0, 0.0, true);
        put("ℒ", m, 12.0, 171.0, 620.0, 7.1, false);
        put("𝑑𝑖𝑠𝑐", m, 8.52, 178.3, 617.6, 20.3, false);
        put("(𝐷", m, 12.0, 199.1, 620.0, 13.3, false);
        put("𝑝", m, 8.52, 212.4, 617.6, 4.8, false);
        put(") =", m, 12.0, 217.7, 620.0, 17.1, false);
        put(" ", m, 12.0, 234.9, 620.0, 10.7, false);
        put("1", m, 12.0, 245.6, 629.2, 6.6, true);
        put("2 ", m, 12.0, 245.6, 612.0, 8.6, false);
        put("𝔼", m, 12.0, 254.2, 620.0, 7.6, false);
        put("𝔁", m, 8.52, 261.9, 617.6, 5.5, false);
        put("𝒑", m, 6.96, 267.4, 616.0, 4.3, false);
        put("~𝒫", m, 8.52, 272.1, 617.6, 12.3, false);
        put("𝓇ℯ𝒶ℓ", m, 6.96, 284.4, 615.8, 16.4, false);
        put(" ", m, 6.96, 300.9, 615.8, 1.0, false);
        put("[(𝐷", m, 12.0, 301.9, 620.0, 17.5, false);
        put("𝑝", m, 8.52, 319.4, 617.6, 4.8, false);
        put("(𝔁", m, 12.0, 324.7, 620.0, 12.8, false);
        put("𝒑", m, 8.52, 337.4, 617.6, 5.3, false);
        put(") − 1)", m, 12.0, 343.1, 620.0, 30.8, false);
        put("2", m, 8.52, 374.0, 624.5, 4.7, false);
        put("]", m, 12.0, 379.1, 620.0, 4.2, false);
        put(" ", m, 12.0, 383.3, 620.0, 10.1, false);
        put("+", m, 12.0, 393.4, 620.0, 9.0, false);
        // row 2: ½ E_{x̃_p ~ P_syn}[D_p(x̃_p)^2]      (1)
        put("", m, 12.0, 275.2, 598.7, 0.0, true);
        put("1", m, 12.0, 275.2, 598.7, 6.6, true);
        put("2 ", m, 12.0, 275.2, 581.5, 8.5, false);
        put("𝔼", m, 12.0, 283.7, 589.6, 7.6, false);
        put("𝔁\u{303}", m, 8.52, 291.4, 587.2, 5.5, false);
        put("𝒑", m, 6.96, 296.9, 585.5, 4.3, false);
        put("~𝒫", m, 8.52, 301.6, 587.2, 12.3, false);
        put("𝑠𝑦𝑛", m, 6.96, 313.9, 585.5, 13.2, false);
        put(" ", m, 6.96, 327.0, 585.5, 0.9, false);
        put("[𝐷", m, 12.0, 327.9, 589.6, 12.0, false);
        put("𝑝", m, 8.52, 339.9, 587.2, 4.8, false);
        put("(𝔁\u{303}", m, 12.0, 345.2, 589.6, 12.7, false);
        put("𝒑", m, 8.52, 357.9, 587.2, 5.3, false);
        put(")", m, 12.0, 363.7, 589.6, 4.0, false);
        put("2", m, 8.52, 368.0, 594.0, 4.7, false);
        put("]", m, 12.0, 373.2, 589.6, 4.2, false);
        put("", tx, 12.0, 504.8, 589.3, 0.0, true);
        put("(1)", tx, 12.0, 504.8, 589.3, 14.0, false);
        // row 3: L_gene(G_p) = ½ E_{x̃_p ~ P_syn}[(D_p(x̃_p) − 1)^2]
        put("", m, 12.0, 186.9, 559.1, 0.0, true);
        put("ℒ", m, 12.0, 186.9, 559.1, 7.1, false);
        put("𝑔𝑒𝑛𝑒", m, 8.52, 194.0, 556.7, 19.8, false);
        put("(𝐺", m, 12.0, 214.1, 559.1, 12.6, false);
        put("𝑝", m, 8.52, 226.7, 556.7, 4.8, false);
        put(") = ", m, 12.0, 232.0, 559.1, 20.5, false);
        put("1", m, 12.0, 252.5, 568.2, 6.6, true);
        put("2 ", m, 12.0, 252.5, 551.0, 8.6, false);
        put("𝔼", m, 12.0, 261.2, 559.1, 7.6, false);
        put("𝔁\u{303}", m, 8.52, 268.9, 556.7, 5.5, false);
        put("𝒑", m, 6.96, 274.4, 555.0, 4.3, false);
        put("~𝒫", m, 8.52, 279.1, 556.7, 12.3, false);
        put("𝑠𝑦𝑛", m, 6.96, 291.3, 555.0, 13.2, false);
        put(" ", m, 6.96, 304.5, 555.0, 0.9, false);
        put("[(𝐷", m, 12.0, 305.3, 559.1, 17.5, false);
        put("𝑝", m, 8.52, 322.9, 556.7, 4.8, false);
        put("(𝔁\u{303}", m, 12.0, 328.1, 559.1, 12.7, false);
        put("𝒑", m, 8.52, 340.9, 556.7, 5.3, false);
        put(") − 1)", m, 12.0, 346.6, 559.1, 30.8, false);
        put("2", m, 8.52, 377.5, 563.5, 4.7, false);
        put("]", m, 12.0, 382.6, 559.1, 4.2, true);
        put("where the rows share one number and the body goes on here.", tx, 12.0, 76.0, 530.0, 440.0, true);
        // (2): L_reco = E[‖x_p − x'_p‖_2] / x'_p = R_p(D_p(x_p)), number between the rows
        put("", m, 12.0, 194.2, 500.0, 0.0, true);
        put("ℒ", m, 12.0, 194.2, 500.0, 7.1, false);
        put("𝑟𝑒𝑐𝑜", m, 8.52, 201.3, 497.6, 21.7, false);
        put(" ", m, 8.52, 223.0, 497.6, 3.8, false);
        put("=", m, 12.0, 226.7, 500.0, 9.0, false);
        put(" ", m, 12.0, 235.7, 500.0, 10.7, false);
        put("𝔼[‖𝔁", m, 12.0, 246.4, 500.0, 26.0, false);
        put("𝒑", m, 8.52, 272.4, 497.6, 5.3, false);
        put(" ", m, 8.52, 277.7, 497.6, 10.6, false);
        put("−", m, 12.0, 288.3, 500.0, 9.0, false);
        put(" ", m, 12.0, 297.3, 500.0, 10.0, false);
        put("𝔁′", m, 12.0, 307.3, 500.0, 10.9, false);
        put("𝒑", m, 8.52, 318.2, 497.6, 5.3, false);
        put("‖", m, 12.0, 324.0, 500.0, 6.3, false);
        put("2", m, 8.52, 330.3, 493.9, 4.7, false);
        put("]", m, 12.0, 335.5, 500.0, 4.2, false);
        put("", m, 12.0, 240.2, 461.7, 0.0, true);
        put("𝔁′", m, 12.0, 240.2, 461.7, 10.9, false);
        put("𝒑", m, 8.52, 251.1, 459.3, 5.3, false);
        put(" ", m, 8.52, 256.4, 459.3, 3.7, false);
        put("= ℛ", m, 12.0, 260.1, 461.7, 21.2, false);
        put("𝑝", m, 8.52, 281.3, 459.3, 4.8, false);
        put("(𝐷", m, 12.0, 286.6, 461.7, 13.3, false);
        put("𝑝", m, 8.52, 299.9, 459.3, 4.8, false);
        put("(𝔁", m, 12.0, 305.1, 461.7, 12.8, false);
        put("𝒑", m, 8.52, 317.9, 459.3, 5.3, false);
        put("))", m, 12.0, 323.7, 461.7, 9.9, false);
        put("", tx, 12.0, 504.8, 479.2, 0.0, true);
        put("(2)", tx, 12.0, 504.8, 479.2, 14.0, true);
        // (3) and (4): one number per row, each a little below its baseline
        put("ℒ", m, 12.0, 232.7, 437.9, 7.1, false);
        put("𝑡𝑜𝑡𝑎𝑙", m, 8.52, 239.8, 435.6, 18.8, false);
        put(" = ℒ", m, 12.0, 258.6, 437.9, 25.0, false);
        put("𝐺", m, 8.52, 283.6, 435.6, 5.1, false);
        put(" ", m, 8.52, 288.7, 435.6, 100.0, false);
        put("(3)", tx, 12.0, 504.8, 432.6, 14.0, true);
        put("ℒ", m, 12.0, 232.7, 412.1, 7.1, false);
        put("𝐺", m, 8.52, 239.8, 409.8, 5.1, false);
        put(" = 𝜆ℒ", m, 12.0, 244.9, 412.1, 30.0, false);
        put("𝐷", m, 8.52, 274.9, 409.8, 5.1, false);
        put(" ", m, 8.52, 280.0, 409.8, 100.0, false);
        put("(4)", tx, 12.0, 504.8, 406.8, 14.0, true);
        let mut y = 380.0;
        for _ in 0..6 {
            put("Plain body text continues across the whole column width here.", tx, 12.0, 76.0, y, 440.0, true);
            y -= 18.0;
        }
        PageText { page: 1, text: t, items, width: 595.0, height: 842.0 }
    }

    #[test]
    fn word_equations_fractions_accents_words_primes_and_numbers() {
        let p = PaperText { pages: vec![word_page()] };
        let det = detect(&p, &Glyphs::default());
        let eqs: Vec<(&str, Option<&str>, &str)> = det.equations.iter().map(|q| (q.id.as_str(), q.number.as_deref(), q.latex.as_str())).collect();
        assert_eq!(eqs.len(), 4, "{eqs:#?}");
        let e1 = det.equations[0].latex.as_str();
        assert_eq!(det.equations[0].number.as_deref(), Some("1"));
        assert_eq!(
            e1,
            "\\begin{aligned}\\mathcal{L}_{\\mathrm{disc}}(D_{p}) &=\\frac{1}{2} \\mathbb{E}_{\\boldsymbol{x}_{\\boldsymbol{p}}\\sim\\mathcal{P}_{\\mathrm{real}}}[(D_{p}(\\boldsymbol{x}_{\\boldsymbol{p}}) - 1)^{2}] + \\\\ \
             &\\quad \\frac{1}{2}\\mathbb{E}_{\\tilde{\\boldsymbol{x}}_{\\boldsymbol{p}}\\sim\\mathcal{P}_{\\mathrm{syn}}}[D_{p}(\\tilde{\\boldsymbol{x}}_{\\boldsymbol{p}})^{2}] \\\\ \
             \\mathcal{L}_{\\mathrm{gene}}(G_{p}) &=\\frac{1}{2}\\mathbb{E}_{\\tilde{\\boldsymbol{x}}_{\\boldsymbol{p}}\\sim\\mathcal{P}_{\\mathrm{syn}}}[(D_{p}(\\tilde{\\boldsymbol{x}}_{\\boldsymbol{p}}) - 1)^{2}]\\end{aligned}"
        );
        let e2 = &det.equations[1];
        assert_eq!(e2.number.as_deref(), Some("2"));
        assert_eq!(e2.latex, "\\begin{aligned}\\mathcal{L}_{\\mathrm{reco}} &= \\mathbb{E}[\\|\\boldsymbol{x}_{\\boldsymbol{p}}- \\boldsymbol{x}'_{\\boldsymbol{p}}\\|_{2}] \\\\ \\boldsymbol{x}'_{\\boldsymbol{p}} &= \\mathcal{R}_{p}(D_{p}(\\boldsymbol{x}_{\\boldsymbol{p}}))\\end{aligned}");
        assert_eq!((det.equations[2].number.as_deref(), det.equations[2].latex.as_str()), (Some("3"), "\\mathcal{L}_{\\mathrm{total}}= \\mathcal{L}_{G}"));
        assert_eq!((det.equations[3].number.as_deref(), det.equations[3].latex.as_str()), (Some("4"), "\\mathcal{L}_{G}= \\lambda\\mathcal{L}_{D}"));
        for q in &det.equations {
            assert!(q.latex.is_ascii() && !q.latex.contains('~') && !q.latex.contains("\\mathcal{x}") && !q.latex.contains("\\prime"), "{}", q.latex);
        }
        // no single letters out of the subscript words, no \mathcal on small letters
        let keys = detected_keys(&det);
        for k in ["r", "e", "a", "s", "y", "n", "\\mathcal{r}"] {
            assert!(!keys.contains(k), "{k} in {keys:?}");
        }
        for k in ["\\mathcal{L}_{\\mathrm{disc}}", "\\boldsymbol{x}_{\\boldsymbol{p}}", "\\tilde{\\boldsymbol{x}}_{\\boldsymbol{p}}"] {
            assert!(keys.contains(k), "{k} not in {keys:?}");
        }
        // a paper bound twice keeps one id per equation
        let mut p2 = p.clone();
        let mut again = word_page();
        again.page = 2;
        p2.pages.push(again);
        let ids: Vec<String> = detect(&p2, &Glyphs::default()).equations.iter().map(|q| q.id.clone()).collect();
        assert_eq!(ids, ["eq:1", "eq:2", "eq:3", "eq:4", "eq:1-2", "eq:2-2", "eq:3-2", "eq:4-2"]);
    }

    #[test]
    fn join_rows_aligns_at_the_first_relation() {
        assert_eq!(join_rows(&["a = b +".into(), "c".into(), "d \\leq e".into()]), "\\begin{aligned}a &= b + \\\\ &\\quad c \\\\ d &\\leq e\\end{aligned}");
        assert_eq!(join_rows(&["x_{a=1}".into(), "y".into()]), "\\begin{gathered}x_{a=1} \\\\ y\\end{gathered}");
        assert_eq!(join_rows(&["  ".into(), "x = 1".into()]), "x = 1");
    }

    #[test]
    fn glyph_table_and_math_alphanumerics() {
        let g = Glyphs::default();
        assert_eq!(g.latex('α').as_deref(), Some("\\alpha"));
        assert_eq!(g.latex('ℝ').as_deref(), Some("\\mathbb{R}"));
        assert_eq!(math_alnum('𝐱').as_deref(), Some("\\mathbf{x}"));
        assert_eq!(math_alnum('𝑥').as_deref(), Some("x"));
        assert_eq!(math_alnum('𝒙').as_deref(), Some("\\boldsymbol{x}"));
        assert_eq!(math_alnum('𝓛').as_deref(), Some("\\mathcal{L}"));
        assert_eq!(math_alnum('𝔽').as_deref(), Some("\\mathbb{F}"));
        assert_eq!(math_alnum('𝜃').as_deref(), Some("\\theta"));
        assert_eq!(math_alnum('𝛉').as_deref(), Some("\\boldsymbol{\\theta}"));
        assert_eq!(math_alnum('𝟑').as_deref(), Some("3"));
        assert_eq!(g.accent('ˆ'), Some("hat"));
        // overrides win; invalid files are reported and skipped
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.toml");
        std::fs::write(&a, "[glyphs]\n\"ε\" = \"\\\\epsilon\"\n[fonts]\nmath = [\"MyMath\"]\n").unwrap();
        let b = d.path().join("b.toml");
        std::fs::write(&b, "[glyphs\n").unwrap();
        let (g2, errs) = Glyphs::load(&[&a, &b]);
        assert_eq!(g2.latex('ε').as_deref(), Some("\\epsilon"));
        assert_eq!(errs.len(), 1);
        assert!(font_class("XYZABC+MyMath-Regular", false, false, &g2).is_math());
        assert!(Glyphs::validate("[glyphs]\n\"ab\" = \"x\"").is_err());
        assert_eq!(font_class("ABCDEF+CMMI10", false, false, &g), FontClass::MathItalic);
        assert_eq!(font_class("STIXTwoMath-Regular", false, false, &g), FontClass::UnicodeMath);
        assert_eq!(font_class("CambriaMath", false, false, &g), FontClass::UnicodeMath);
        assert_eq!(font_class("Times-Italic", false, false, &g), FontClass::Text { italic: true, bold: false });
    }

    #[test]
    fn inline_math_scripts_accents_bold_and_a_display_equation() {
        let p = PaperText { pages: vec![page()] };
        let det = detect(&p, &Glyphs::default());
        let latex: Vec<&str> = det.spans.iter().map(|s| s.latex.as_str()).collect();
        assert!(latex.contains(&"w_{i}"), "{latex:?}");
        assert!(latex.contains(&"\\hat{y}=f(\\mathbf{x})"), "{latex:?}");
        // the display equation: number, bbox, and its run
        assert_eq!(det.equations.len(), 1, "{:?}", det.equations);
        let q = &det.equations[0];
        assert_eq!((q.id.as_str(), q.number.as_deref()), ("eq:1", Some("1")));
        assert!(q.bbox[0] > 0.3 && q.bbox[2] < 0.6, "{:?}", q.bbox);
        assert!(q.latex.contains("\\mathcal{L}") && q.latex.contains("\\log") && q.latex.contains("p_{i}"), "{}", q.latex);
        let keys = detected_keys(&det);
        for k in ["w", "\\hat{y}", "f", "\\boldsymbol{x}", "\\mathcal{L}", "p"] {
            assert!(keys.contains(k), "{k} not in {keys:?}");
        }
        assert!(!keys.contains("i"), "index letters are not symbols");
        // offsets point at the text
        let w = det.spans.iter().find(|s| s.latex == "w_{i}").unwrap();
        assert_eq!(utf16_slice(&p.pages[0].text, w.s, w.e), "wi");
    }

    #[test]
    fn definitions_grades_and_conflicts() {
        let p = PaperText { pages: vec![page()] };
        let det = detect(&p, &Glyphs::default());
        let t = build(&p, &det, &"ab".repeat(32));
        let w = t.entries.iter().find(|e| e.entry.latex.as_deref() == Some("w")).unwrap();
        assert_eq!(w.grade, "defined");
        assert_eq!(w.entry.meaning_en.as_deref(), Some("the weight of pixel $i$"));
        assert_eq!(w.resolved_by, "program");
        assert!(w.evidence[0].quote.contains("where $w_{i}$ is the weight"), "{:?}", w.evidence);
        assert_eq!(w.equations, ["eq:1"]);
        assert_eq!(w.occurrences.len(), 2);
        assert_eq!(w.entry.kind, Some(Kind::Symbol));
        let hat = t.entries.iter().find(|e| e.entry.latex.as_deref() == Some("\\hat{y}")).unwrap();
        assert_eq!(hat.grade, "defined", "{hat:?}");
        assert_eq!(hat.entry.meaning_en.as_deref(), Some("the prediction"));
        let f = t.entries.iter().find(|e| e.entry.latex.as_deref() == Some("f")).unwrap();
        assert_eq!(f.grade, "unresolved");
        assert_eq!(t.equations[0].entry.label.as_deref(), Some("(1)"));
        assert!(t.equations[0].entry.uses.contains(&w.entry.id));

        // A second, different definition in the same paper: conflict.
        let mut p2 = p.clone();
        let mut pg = page();
        pg.page = 2;
        let txt = pg.text.replace("the weight of pixel ", "the learning rate xx");
        pg.text = txt;
        p2.pages.push(pg);
        let det2 = detect(&p2, &Glyphs::default());
        let t2 = build(&p2, &det2, "x");
        let w2 = t2.entries.iter().find(|e| e.entry.latex.as_deref() == Some("w")).unwrap();
        assert_eq!(w2.grade, "conflict", "{:?}", w2.evidence);

        // Author edits survive a re-run.
        let mut edited = t.clone();
        let fe = edited.entries.iter_mut().find(|e| e.entry.latex.as_deref() == Some("f")).unwrap();
        fe.entry.meaning = Some("網路".into());
        fe.grade = "certain".into();
        fe.resolved_by = "author".into();
        let mut mine = Entry::new("extra", Kind::Symbol);
        mine.latex = Some("\\beta".into());
        edited.entries.push(SymbolEntry { entry: mine, scope: "x".into(), grade: "inferred".into(), resolved_by: "author".into(), evidence: vec![], occurrences: vec![], first: None, equations: vec![], note: Some("我加的".into()) });
        let again = merge(Some(edited), build(&p, &det, "x"));
        let f2 = again.entries.iter().find(|e| e.entry.latex.as_deref() == Some("f")).unwrap();
        assert_eq!((f2.entry.meaning.as_deref(), f2.grade.as_str(), f2.resolved_by.as_str()), (Some("網路"), "certain", "author"));
        assert!(!f2.occurrences.is_empty());
        assert!(again.entries.iter().any(|e| e.entry.id == "extra"));
        // the file round-trips through JSON with lore-notation's field names
        let v = serde_json::to_value(&again).unwrap();
        assert_eq!(v["entries"][0]["kind"], "symbol");
        assert!(v["entries"][0]["latex"].is_string() && v["entries"][0]["scope"].is_string());
        let back: SymbolFile = serde_json::from_value(v).unwrap();
        assert_eq!(back, again);
    }

    #[test]
    fn sentences_skip_abbreviations_and_decimals() {
        let t = "We use e.g. the Dice score of 0.81 here. Next one?\n\nNew para";
        let ss: Vec<String> = sentences(t).into_iter().map(|(s, e)| utf16_slice(t, s, e).trim().to_string()).collect();
        assert_eq!(ss, ["We use e.g. the Dice score of 0.81 here.", "Next one?", "New para"]);
    }
}
