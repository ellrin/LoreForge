//! The paper's noun list (track A, 「名詞整理」): the author's method for the red terms.
//!
//! 1. Word inventory of the whole body text (references cut off), with every
//!    occurrence (page + UTF-16 range in that page's text).
//! 2. Multi-word terms first: n-grams of 2–4 words in the text are looked up among
//!    the KnowledgeBase card titles and aliases ("cross entropy", "Dice coefficient",
//!    "U-Net" = two words). An acronym in parentheses after words whose initials spell
//!    it ("Spectral Boundary Loss (SBL)") is also a unit, and later uses of the acronym
//!    count for it. Then the rest is split into single words; a hyphenated compound
//!    ("label-smoothing") stays one unit unless every part is a general word.
//! 3. Normalisation for grouping and lookup: case, plurals, hyphen vs space,
//!    British/American spelling, ligatures, curly apostrophes, line-break hyphenation
//!    ("segmen-\ntation" → "segmentation" when the joined word is used elsewhere or
//!    the second part is lowercase and the hyphenated form never appears).
//! 4. Function words, general academic words (`academic_words.txt`), everyday English
//!    (`common_words.txt`, except [`TERMLIKE`] words that are terms in research
//!    papers) and the author's ignore list are dropped. Acronyms are kept.
//! 5. Each unit is compared with the KB: known, learning / stale, or no card.
//! 6. The no-card units are classified by a model in one batched call per paper
//!    ([`classify_prompt`], [`classify_schema`], cached by the server); without a
//!    model they stay unclassified.
//!
//! Colour on the page: known = green underline, learning / stale = amber, no card =
//! red — except 一般學術詞 and 人名, which are listed but not coloured.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::text::{PaperText, U16Map, body_end};

const COMMON: &str = include_str!("common_words.txt");
const ACADEMIC: &str = include_str!("academic_words.txt");
const FUNCTION: &str = include_str!("function_words.txt");

/// Everyday words that are specialist terms in research papers: never dropped as
/// "general" (they still go to the model, which can call them 一般學術詞).
pub const TERMLIKE: &[&str] = &[
    "accuracy", "precision", "recall", "sensitivity", "specificity", "regression", "variance", "correlation", "deviation", "entropy", "kernel",
    "attention", "distribution", "inference", "probability", "threshold", "vector", "convergence", "optimization", "gradient", "sampling",
    "simulation", "transformation", "representation", "reconstruction", "classification", "cluster", "clustering", "frequency", "signal",
    "noise", "filter", "metric", "statistic", "algorithm", "regularization", "interpolation", "segmentation", "embedding", "encoder", "decoder",
    "likelihood", "posterior", "prior", "residual", "convolution", "pooling", "dropout", "momentum", "boundary", "loss", "label", "mask",
];

/// The model's categories, in display order.
pub const CATEGORIES: [&str; 9] = ["方法", "指標", "資料集", "統計", "臨床／解剖", "其他術語", "基礎科學詞", "一般學術詞", "人名"];

/// Categories whose no-card terms are red: specialist terms an expert in the author's
/// field would still have to learn. 基礎科學詞, 一般學術詞, 人名 and unclassified words are
/// never red.
pub const SPECIALIST: [&str; 6] = ["方法", "指標", "資料集", "統計", "臨床／解剖", "其他術語"];

/// Bump when the categories or the classification prompt change: cached categories
/// from another version are dropped.
pub const CLASSIFY_VERSION: u32 = 2;

/// The field and level "basic" is judged for, unless the author sets their own.
pub const DEFAULT_FIELD: &str = "researcher in medical image analysis and deep learning";

pub fn is_specialist(category: Option<&str>) -> bool {
    category.is_some_and(|c| SPECIALIST.contains(&c))
}

/// One KB name (a card title or alias) and the card's state for this member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KbName {
    pub name: String,
    pub node_id: String,
    pub title: String,
    /// learning | known | stale | None (a card without learning state yet)
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KbRef {
    pub node_id: String,
    pub title: String,
}

/// One unit of the noun list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Noun {
    /// Normalised key (grouping, ignore list, classification cache).
    pub key: String,
    /// The most frequent spelling in the text.
    pub text: String,
    /// kb | expansion | compound | acronym | word
    pub source: String,
    /// The acronym of an expansion ("SBL").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb: Option<KbRef>,
    /// known | learning | stale | none (card without learning state) | new (no card)
    pub state: String,
    #[serde(default)]
    pub category: Option<String>,
    pub count: usize,
    pub first_page: u32,
    /// `[page, start, end]`, UTF-16 offsets in that page's text.
    pub occ: Vec<[u32; 3]>,
    /// No card, but the author used it in their own papers (my-vocab): counts as 記得
    /// (never red or amber); a KB card's own state always wins.
    #[serde(default)]
    pub used: bool,
    /// Not red yet: a single word while the reader's field is not set (see
    /// [`Inventory::quiet_single_words`]). Never stored; set when the list is built.
    #[serde(default)]
    pub quiet: bool,
}

impl Noun {
    /// Red on the page: no card, not used in the author's own papers, and classified
    /// as a specialist term (never 基礎科學詞, 一般學術詞, 人名 or unclassified).
    pub fn red(&self) -> bool {
        self.state == "new" && !self.used && !self.quiet && is_specialist(self.category.as_deref())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// Running words in the body.
    pub words: usize,
    /// Distinct word forms (after normalisation) before filtering.
    pub unique: usize,
    /// Units in the list (after filtering).
    pub units: usize,
    pub known: usize,
    pub learning: usize,
    pub no_card: usize,
    pub classified: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    pub entries: Vec<Noun>,
    pub stats: Stats,
}

// ── word lists ───────────────────────────────────────────────────────────

fn list(src: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for line in src.lines() {
        let line = line.split('#').next().unwrap_or("");
        for w in line.split_whitespace() {
            out.insert(w.to_lowercase());
        }
    }
    out
}

/// The word filters: function words, general words and the author's ignore list.
pub struct Filters {
    function: HashSet<String>,
    general: HashSet<String>,
    termlike: HashSet<String>,
    /// Keys from the ignore list (single words and phrases).
    ignore: HashSet<String>,
}

impl Default for Filters {
    fn default() -> Self {
        Filters::new(&[])
    }
}

impl Filters {
    /// Built-in lists plus `ignore` lines (one or more words per line, `#` comments;
    /// a line with several words that is a phrase is kept as one phrase key too).
    pub fn new(ignore: &[String]) -> Filters {
        let mut general = list(COMMON);
        general.extend(list(ACADEMIC));
        let termlike: HashSet<String> = TERMLIKE.iter().map(|s| s.to_string()).collect();
        let mut ig = HashSet::new();
        for line in ignore {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            ig.insert(phrase_key(line));
            for w in line.split_whitespace() {
                ig.insert(word_key(w));
            }
        }
        Filters { function: list(FUNCTION), general, termlike, ignore: ig }
    }

    /// The lines of an ignore file.
    pub fn with_file(path: &std::path::Path) -> Filters {
        Filters::new(&read_lines(path))
    }

    pub fn is_function(&self, w: &str) -> bool {
        let l = w.to_lowercase().replace('’', "'");
        self.function.contains(&l)
    }

    /// A general word (academic or everyday English, allowing for inflections), but
    /// never a [`TERMLIKE`] word.
    pub fn is_general(&self, w: &str) -> bool {
        let forms = variants(w);
        if forms.iter().any(|f| self.termlike.contains(f)) {
            return false;
        }
        forms.iter().any(|f| self.general.contains(f) || self.function.contains(f))
    }

    pub fn is_ignored(&self, key: &str) -> bool {
        self.ignore.contains(key)
    }
}

pub fn read_lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path).map(|s| s.lines().map(str::to_string).collect()).unwrap_or_default()
}

// ── normalisation ────────────────────────────────────────────────────────

const LIGATURES: &[(&str, &str)] = &[("ﬀ", "ff"), ("ﬁ", "fi"), ("ﬂ", "fl"), ("ﬃ", "ffi"), ("ﬄ", "ffl"), ("ﬅ", "st"), ("ﬆ", "st"), ("’", "'"), ("‘", "'")];

/// Irregular plurals and words a suffix rule would break.
const IRREGULAR: &[(&str, &str)] = &[
    ("indices", "index"), ("matrices", "matrix"), ("vertices", "vertex"), ("appendices", "appendix"), ("analyses", "analysis"),
    ("hypotheses", "hypothesis"), ("theses", "thesis"), ("diagnoses", "diagnosis"), ("prognoses", "prognosis"), ("syntheses", "synthesis"),
    ("criteria", "criterion"), ("phenomena", "phenomenon"), ("maxima", "maximum"), ("minima", "minimum"), ("optima", "optimum"),
    ("spectra", "spectrum"), ("nuclei", "nucleus"), ("radii", "radius"), ("foci", "focus"), ("stimuli", "stimulus"), ("bacteria", "bacterium"),
    ("vertebrae", "vertebra"), ("men", "man"), ("women", "woman"), ("children", "child"), ("feet", "foot"), ("teeth", "tooth"), ("mice", "mouse"),
];

/// Words ending in -s that are not plurals.
const NOT_PLURAL: &[&str] = &[
    "analysis", "basis", "thesis", "axis", "diagnosis", "prognosis", "synthesis", "hypothesis", "stenosis", "fibrosis", "sclerosis",
    "this", "his", "its", "is", "was", "has", "does", "gas", "bias", "atlas", "lens", "focus", "radius", "corpus", "status", "virus",
    "consensus", "census", "bonus", "nucleus", "stimulus", "calculus", "apparatus", "series", "species", "news", "physics", "mathematics",
    "statistics", "economics", "always", "perhaps", "whereas", "thus", "plus", "minus", "versus", "gaussian", "process", "loss", "less",
];

/// British → American for a lowercase word (only to group spellings; a rule that
/// also changes a word without a variant changes its key consistently, which is harmless).
pub fn americanise(w: &str) -> String {
    const WORDS: &[(&str, &str)] = &[
        ("grey", "gray"), ("programme", "program"), ("programmes", "programs"), ("defence", "defense"), ("licence", "license"),
        ("offence", "offense"), ("aluminium", "aluminum"), ("sulphur", "sulfur"), ("mould", "mold"), ("plough", "plow"), ("tyre", "tire"),
        ("enrol", "enroll"), ("fulfil", "fulfill"), ("judgement", "judgment"), ("ageing", "aging"), ("artefact", "artifact"),
        ("artefacts", "artifacts"), ("manoeuvre", "maneuver"), ("cheque", "check"), ("draught", "draft"),
    ];
    if let Some((_, a)) = WORDS.iter().find(|(b, _)| *b == w) {
        return a.to_string();
    }
    let mut s = w.to_string();
    for (b, a) in [("haem", "hem"), ("anaes", "anes"), ("paed", "ped"), ("oedem", "edem"), ("oesoph", "esoph"), ("oestr", "estr"), ("foet", "fet"), ("leukaem", "leukem"), ("orthopaed", "orthoped"), ("caesar", "cesar")] {
        if s.starts_with(b) || s.contains(b) && s.len() > 6 {
            s = s.replacen(b, a, 1);
        }
    }
    // -isation/-ise/-yse → -ization/-ize/-yze
    for (b, a) in [("isation", "ization"), ("isations", "izations"), ("yse", "yze"), ("ysed", "yzed"), ("yses", "yzes"), ("ysing", "yzing"), ("ise", "ize"), ("ised", "ized"), ("ises", "izes"), ("ising", "izing"), ("iser", "izer"), ("isers", "izers")] {
        if s.len() > b.len() + 3
            && let Some(stem) = s.strip_suffix(b)
        {
            s = format!("{stem}{a}");
            break;
        }
    }
    // colour → color, tumours → tumors (long words only: four, hour, your stay)
    for (b, a) in [("our", "or"), ("ours", "ors"), ("oured", "ored"), ("ouring", "oring"), ("ourful", "orful")] {
        if s.len() >= 6
            && let Some(stem) = s.strip_suffix(b)
            && !stem.ends_with(['a', 'e', 'i', 'o', 'u', 'y'])
        {
            s = format!("{stem}{a}");
            break;
        }
    }
    // centre → center, fibre → fiber, metres → meters
    for (b, a) in [("tre", "ter"), ("tres", "ters"), ("bre", "ber"), ("bres", "bers")] {
        if s.len() >= 5
            && let Some(stem) = s.strip_suffix(b)
            && !stem.ends_with(['a', 'e', 'i', 'o', 'u'])
        {
            s = format!("{stem}{a}");
            break;
        }
    }
    // modelling → modeling, labelled → labeled
    for (b, a) in [("lling", "ling"), ("lled", "led"), ("ller", "ler"), ("llers", "lers")] {
        if s.len() >= 8
            && let Some(stem) = s.strip_suffix(b)
            && stem.ends_with(['e', 'a', 'o'])
        {
            s = format!("{stem}{a}");
            break;
        }
    }
    // catalogue → catalog, analogue → analog
    if s.len() >= 8
        && let Some(stem) = s.strip_suffix("ogue")
    {
        s = format!("{stem}og");
    }
    s
}

/// Singular of a lowercase word (rules + exceptions).
pub fn singular(w: &str) -> String {
    if let Some((_, s)) = IRREGULAR.iter().find(|(p, _)| *p == w) {
        return s.to_string();
    }
    if NOT_PLURAL.contains(&w) || w.len() <= 3 || !w.chars().all(|c| c.is_alphabetic() || c == '\'') {
        return w.to_string();
    }
    if let Some(r) = w.strip_suffix("ies").filter(|r| r.len() >= 3) {
        return format!("{r}y");
    }
    for suf in ["sses", "xes", "ches", "shes", "zzes"] {
        if let Some(r) = w.strip_suffix(suf) {
            return format!("{r}{}", &suf[..suf.len() - 2]);
        }
    }
    if w.ends_with("ss") || w.ends_with("us") || w.ends_with("is") || w.ends_with("os") && w.len() <= 4 {
        return w.to_string();
    }
    if let Some(r) = w.strip_suffix('s') {
        return r.to_string();
    }
    w.to_string()
}

/// Key of one word: lowercase, ligatures and apostrophes folded, possessive dropped,
/// American spelling, singular. Acronyms keep their letters (`SBLs` → `sbl`).
pub fn word_key(w: &str) -> String {
    let mut s = w.to_string();
    for (a, b) in LIGATURES {
        if s.contains(a) {
            s = s.replace(a, b);
        }
    }
    let all_caps = s.chars().filter(|c| c.is_alphabetic()).count() >= 2 && s.trim_end_matches('s').chars().all(|c| !c.is_lowercase());
    let mut l = s.to_lowercase();
    if let Some(r) = l.strip_suffix("'s") {
        l = r.to_string();
    }
    l = l.trim_end_matches('\'').to_string();
    if let Some((_, s)) = IRREGULAR.iter().find(|(p, _)| *p == l) {
        return s.to_string();
    }
    if all_caps {
        // SBLs → sbl
        return if s.ends_with('s') && s.len() > 2 && s[..s.len() - 1].chars().all(|c| !c.is_lowercase()) { l[..l.len() - 1].to_string() } else { l };
    }
    singular(&americanise(&l))
}

/// Key of a phrase: word keys joined by one space; hyphens count as spaces.
pub fn phrase_key(p: &str) -> String {
    p.split(|c: char| c.is_whitespace() || is_hyphen(c)).filter(|w| !w.is_empty()).map(word_key).collect::<Vec<_>>().join(" ")
}

fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '‐' | '‑' | '–')
}

/// Lowercase forms of `w` with common inflections removed (for the general lists).
fn variants(w: &str) -> Vec<String> {
    let l = word_key(w);
    let mut forms = vec![l.clone(), w.to_lowercase()];
    let strip = |suf: &str| l.strip_suffix(suf).filter(|r| r.len() >= 3).map(str::to_string);
    for suf in ["ed", "d", "ing", "ly", "er", "est", "ness", "ment", "al", "ity", "ally", "ation"] {
        if let Some(r) = strip(suf) {
            forms.push(r.clone());
            forms.push(format!("{r}e"));
            let b = r.as_bytes();
            if b.len() >= 3 && b[b.len() - 1] == b[b.len() - 2] {
                forms.push(r[..r.len() - 1].to_string());
            }
        }
    }
    if let Some(r) = strip("ily") {
        forms.push(format!("{r}y"));
    }
    for suf in ["ied", "ier", "iest"] {
        if let Some(r) = strip(suf) {
            forms.push(format!("{r}y"));
        }
    }
    if let Some(r) = strip("ically") {
        forms.push(format!("{r}ic"));
    }
    forms
}

// ── tokens ───────────────────────────────────────────────────────────────

/// One word of the body text (a hyphen part counts as a word).
#[derive(Debug, Clone)]
struct Word {
    page: usize,
    /// byte range in the page text (a line-break-joined word spans the break)
    b0: usize,
    b1: usize,
    /// surface (hyphenation joined: "segmentation")
    text: String,
    key: String,
    /// joined to the next word by a hyphen ("cross"-"entropy")
    hy_next: bool,
    /// nothing but spaces / line breaks before the next word (n-grams may continue)
    open_next: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() && !matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF) || c == '\'' || c == '’'
}

/// Raw runs of word characters with byte offsets, in one page text.
fn runs(text: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if is_word_char(c) && !(start.is_none() && (c == '\'' || c == '’')) {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push((s, i));
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    // trailing apostrophes are not part of the word
    out.into_iter()
        .map(|(s, mut e)| {
            while e > s && text[..e].ends_with(['\'', '’']) {
                e -= text[..e].chars().next_back().map(char::len_utf8).unwrap_or(1);
            }
            (s, e)
        })
        .filter(|(s, e)| e > s)
        .collect()
}

/// Words of the body with line-break hyphenation resolved. `seen` holds every
/// unbroken lowercase word of the paper and every hyphenated pair `a-b`.
fn words(paper: &PaperText) -> Vec<Word> {
    let end = body_end(paper);
    let mut raw: Vec<(usize, usize, usize)> = vec![]; // page, b0, b1
    for (pi, p) in paper.pages.iter().enumerate() {
        let limit = match end {
            Some((ep, u)) if ep == pi => U16Map::new(&p.text).byte(u),
            Some((ep, _)) if pi > ep => 0,
            _ => p.text.len(),
        };
        for (s, e) in runs(&p.text[..limit]) {
            raw.push((pi, s, e));
        }
    }
    let text = |k: usize| -> &str { &paper.pages[raw[k].0].text[raw[k].1..raw[k].2] };
    let between = |k: usize| -> &str {
        let (p0, _, e0) = raw[k];
        let (p1, s1, _) = raw[k + 1];
        if p0 == p1 { &paper.pages[p0].text[e0..s1] } else { "\n" }
    };
    // Words written whole somewhere, and hyphenated pairs written on one line.
    let mut whole: HashSet<String> = HashSet::new();
    let mut pairs: HashSet<String> = HashSet::new();
    for k in 0..raw.len() {
        whole.insert(text(k).to_lowercase());
        if k + 1 < raw.len() && between(k).chars().count() == 1 && between(k).chars().all(is_hyphen) {
            pairs.insert(format!("{}-{}", text(k).to_lowercase(), text(k + 1).to_lowercase()));
        }
    }
    let mut out: Vec<Word> = vec![];
    let mut k = 0;
    while k < raw.len() {
        let (pg, b0, mut b1) = raw[k];
        let mut surface = text(k).to_string();
        let mut next = k + 1;
        // "segmen-\ntation": a hyphen at the end of a line, then a lowercase word.
        if k + 1 < raw.len() && raw[k + 1].0 == pg {
            let gap = between(k);
            let g = gap.trim_end_matches([' ', '\t']);
            if g.chars().next().is_some_and(is_hyphen) && gap.contains('\n') && g[g.chars().next().unwrap().len_utf8()..].trim().is_empty() {
                let second = text(k + 1);
                let joined = format!("{}{}", surface, second);
                let pair = format!("{}-{}", surface.to_lowercase(), second.to_lowercase());
                let lower_next = second.chars().next().is_some_and(char::is_lowercase);
                if whole.contains(&joined.to_lowercase()) || (lower_next && !pairs.contains(&pair)) {
                    surface = joined;
                    b1 = raw[k + 1].2;
                    next = k + 2;
                }
            }
        }
        out.push(Word { page: pg, b0, b1, key: word_key(&surface), text: surface, hy_next: false, open_next: false });
        k = next;
    }
    // Links to the next word.
    for i in 0..out.len().saturating_sub(1) {
        if out[i].page != out[i + 1].page {
            // a phrase may continue over a page break (the gap is not known): allow
            out[i].open_next = true;
            continue;
        }
        let gap = &paper.pages[out[i].page].text[out[i].b1..out[i + 1].b0];
        // "a-b", or "a-" at the end of a line that was not joined (a real hyphen)
        let g = gap.trim();
        out[i].hy_next = g.chars().count() == 1 && g.chars().all(is_hyphen) && !gap.starts_with(char::is_whitespace);
        out[i].open_next = out[i].hy_next || (!gap.is_empty() && gap.chars().all(char::is_whitespace));
    }
    out
}

// ── the inventory ────────────────────────────────────────────────────────

fn is_acronym(w: &str) -> bool {
    let core = w.strip_suffix('s').filter(|c| c.chars().filter(|x| x.is_uppercase()).count() >= 2).unwrap_or(w);
    core.chars().filter(|c| c.is_uppercase()).count() >= 2 && core.chars().all(|c| c.is_uppercase() || c.is_ascii_digit()) && core.chars().count() <= 10
}

fn is_roman(w: &str) -> bool {
    w.len() <= 5 && w.chars().all(|c| "IVXLCivxlc".contains(c)) && w.len() >= 2
}

/// Worth listing as a single word at all (before the word lists).
fn listable(w: &str) -> bool {
    let letters = w.chars().filter(|c| c.is_alphabetic()).count();
    if letters == 0 || is_roman(w) {
        return false;
    }
    if is_acronym(w) {
        return true;
    }
    // single letters, two-letter lowercase words, Greek letters (symbols: track B)
    if letters <= 2 && !w.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    !w.chars().all(|c| matches!(c as u32, 0x0370..=0x03FF))
}

struct Acc {
    noun: Noun,
    forms: BTreeMap<String, usize>,
}

#[allow(clippy::too_many_arguments)]
fn add(acc: &mut BTreeMap<String, Acc>, id: String, key: &str, surface: &str, source: &str, kbn: Option<&KbName>, short: Option<String>, occ: [u32; 3]) {
    let a = acc.entry(id).or_insert_with(|| Acc {
        noun: Noun {
            key: key.to_string(),
            text: String::new(),
            source: source.to_string(),
            short: short.clone(),
            kb: kbn.map(|n| KbRef { node_id: n.node_id.clone(), title: n.title.clone() }),
            state: match kbn {
                Some(n) => n.state.clone().unwrap_or_else(|| "none".into()),
                None => "new".into(),
            },
            category: None,
            count: 0,
            first_page: occ[0],
            occ: vec![],
            used: false,
            quiet: false,
        },
        forms: BTreeMap::new(),
    });
    if a.noun.short.is_none() {
        a.noun.short = short;
    }
    a.noun.count += 1;
    a.noun.first_page = a.noun.first_page.min(occ[0]);
    a.noun.occ.push(occ);
    *a.forms.entry(surface.to_string()).or_default() += 1;
}

/// Build the inventory. `kb` = every card title and alias (empty when the KB is
/// down); `cats` = the cached classification (key → category).
pub fn inventory(paper: &PaperText, kb: &[KbName], filters: &Filters, cats: &HashMap<String, String>) -> Inventory {
    inventory_masked(paper, kb, filters, cats, &[])
}

/// [`inventory`] without the words inside `mask` (`(page, start, end)`, UTF-16): the math
/// runs and display equations of the symbol table, whose letters are not words.
pub fn inventory_masked(paper: &PaperText, kb: &[KbName], filters: &Filters, cats: &HashMap<String, String>, mask: &[(u32, usize, usize)]) -> Inventory {
    let mut ws = words(paper);
    if !mask.is_empty() {
        let u16s: Vec<U16Map> = paper.pages.iter().map(|p| U16Map::new(&p.text)).collect();
        ws.retain(|w| {
            let page = paper.pages[w.page].page.max(1);
            let (a, b) = (u16s[w.page].u16(w.b0), u16s[w.page].u16(w.b1));
            !mask.iter().any(|&(pg, s, e)| pg == page && a < e && s < b)
        });
    }
    let u16s: Vec<U16Map> = paper.pages.iter().map(|p| U16Map::new(&p.text)).collect();
    let page_no = |pi: usize| paper.pages[pi].page.max(1);
    let occ_of = |a: &Word, b: &Word| -> [u32; 3] { [page_no(a.page), u16s[a.page].u16(a.b0) as u32, u16s[b.page].u16(b.b1) as u32] };

    // KB names by phrase key; single-word names separately.
    let mut kb_multi: HashMap<String, usize> = HashMap::new();
    let mut kb_single: HashMap<String, usize> = HashMap::new();
    for (i, n) in kb.iter().enumerate() {
        let k = phrase_key(&n.name);
        let len = k.split(' ').count();
        if k.is_empty() {
            continue;
        }
        if len == 1 {
            kb_single.entry(k).or_insert(i);
        } else if len <= 4 {
            kb_multi.entry(k).or_insert(i);
        }
    }

    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    let surface = |a: usize, b: usize| -> String {
        // words a..b as written (hyphens kept, line breaks as spaces)
        let mut s = String::new();
        for (k, w) in ws[a..b].iter().enumerate() {
            s += &w.text;
            if a + k + 1 < b {
                s += if w.hy_next { "-" } else { " " };
            }
        }
        s
    };
    let contiguous = |a: usize, n: usize| -> bool { a + n <= ws.len() && (a..a + n - 1).all(|i| ws[i].open_next) };

    let mut taken = vec![false; ws.len()];
    // 2. KB phrases, longest first.
    let mut i = 0;
    while i < ws.len() {
        let mut hit = None;
        for n in (2..=4).rev() {
            if !contiguous(i, n) {
                continue;
            }
            let key = ws[i..i + n].iter().map(|w| w.key.as_str()).collect::<Vec<_>>().join(" ");
            if let Some(&k) = kb_multi.get(&key) {
                hit = Some((n, k, key));
                break;
            }
        }
        if let Some((n, k, key)) = hit {
            let n_ = &kb[k];
            add(&mut acc, format!("kb:{}", n_.node_id), &key, &surface(i, i + n), "kb", Some(n_), None, occ_of(&ws[i], &ws[i + n - 1]));
            taken[i..i + n].iter_mut().for_each(|x| *x = true);
            i += n;
        } else {
            i += 1;
        }
    }
    // Acronyms in parentheses and their long forms.
    let mut acronym_of: HashMap<String, String> = HashMap::new(); // acronym key → entry id
    for i in 0..ws.len() {
        let w = &ws[i];
        let t = &paper.pages[w.page].text;
        let open = t[..w.b0].ends_with('(');
        let close = t[w.b1..].starts_with(')') || t[w.b1..].starts_with("s)");
        if !(open && close && is_acronym(&w.text)) {
            continue;
        }
        let letters: Vec<char> = w.text.trim_end_matches('s').chars().filter(|c| c.is_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
        // walk back over the words before "(", skipping small connectors
        let mut idx = letters.len();
        let mut k = i;
        let mut start = None;
        while k > 0 && idx > 0 && i - k < letters.len() + 4 {
            k -= 1;
            let prev = &ws[k];
            if k + 1 < i && !ws[k].open_next {
                break;
            }
            let lw = prev.text.to_lowercase();
            if ["of", "and", "for", "the", "in", "on", "with", "to", "a", "an"].contains(&lw.as_str()) && idx < letters.len() {
                continue;
            }
            if lw.starts_with(letters[idx - 1]) {
                idx -= 1;
                start = Some(k);
            } else {
                break;
            }
        }
        let Some(s) = start.filter(|_| idx == 0) else { continue };
        if !(s..i - 1).all(|j| ws[j].open_next) {
            continue;
        }
        let id = if taken[s] {
            // the long form is a KB phrase: the acronym belongs to that card
            acc.iter().find(|(_, a)| a.noun.occ.iter().any(|o| o[0] == page_no(ws[s].page) && o[1] == u16s[ws[s].page].u16(ws[s].b0) as u32)).map(|(id, _)| id.clone())
        } else {
            let key = ws[s..i].iter().map(|w| w.key.as_str()).collect::<Vec<_>>().join(" ");
            let id = format!("x:{key}");
            add(&mut acc, id.clone(), &key, &surface(s, i), "expansion", None, Some(w.text.trim_end_matches('s').to_string()), occ_of(&ws[s], &ws[i - 1]));
            taken[s..i].iter_mut().for_each(|x| *x = true);
            Some(id)
        };
        if let Some(id) = id {
            acronym_of.entry(w.key.clone()).or_insert(id);
        }
    }
    // Later uses of an expansion's long form ("the Spectral Boundary Loss weights …").
    let longs: HashMap<String, String> = acc.iter().filter(|(id, _)| id.starts_with("x:")).map(|(id, a)| (a.noun.key.clone(), id.clone())).collect();
    if !longs.is_empty() {
        let max = longs.keys().map(|k| k.split(' ').count()).max().unwrap_or(0);
        let mut i = 0;
        while i < ws.len() {
            let mut hit = None;
            for n in (2..=max.min(8)).rev() {
                if !contiguous(i, n) || taken[i..i + n].iter().any(|t| *t) {
                    continue;
                }
                let key = ws[i..i + n].iter().map(|w| w.key.as_str()).collect::<Vec<_>>().join(" ");
                if let Some(id) = longs.get(&key) {
                    hit = Some((n, id.clone(), key));
                    break;
                }
            }
            if let Some((n, id, key)) = hit {
                add(&mut acc, id, &key, &surface(i, i + n), "expansion", None, None, occ_of(&ws[i], &ws[i + n - 1]));
                taken[i..i + n].iter_mut().for_each(|x| *x = true);
                i += n;
            } else {
                i += 1;
            }
        }
    }
    // 3. Compounds and single words.
    let mut i = 0;
    while i < ws.len() {
        if taken[i] {
            i += 1;
            continue;
        }
        // a hyphenated run of untaken words
        let mut j = i;
        while j + 1 < ws.len() && ws[j].hy_next && !taken[j + 1] {
            j += 1;
        }
        if j > i {
            let parts = &ws[i..=j];
            let key = parts.iter().map(|w| w.key.as_str()).collect::<Vec<_>>().join(" ");
            let all_general = parts.iter().all(|w| !listable(&w.text) || filters.is_function(&w.text) || filters.is_general(&w.text));
            if let Some(&k) = kb_single.get(&key.replace(' ', "")) {
                let n_ = &kb[k];
                add(&mut acc, format!("kb:{}", n_.node_id), &key, &surface(i, j + 1), "kb", Some(n_), None, occ_of(&ws[i], &ws[j]));
            } else if !all_general && !filters.is_ignored(&key) && parts.iter().any(|w| w.text.chars().any(char::is_alphabetic)) {
                add(&mut acc, format!("c:{key}"), &key, &surface(i, j + 1), "compound", None, None, occ_of(&ws[i], &ws[j]));
            }
            i = j + 1;
            continue;
        }
        let w = &ws[i];
        i += 1;
        if let Some(&k) = kb_single.get(&w.key) {
            let n_ = &kb[k];
            add(&mut acc, format!("kb:{}", n_.node_id), &w.key, &w.text, "kb", Some(n_), None, occ_of(w, w));
            continue;
        }
        if let Some(id) = acronym_of.get(&w.key) {
            let key = acc.get(id).map(|a| a.noun.key.clone()).unwrap_or_default();
            let src = acc.get(id).map(|a| a.noun.source.clone()).unwrap_or_default();
            add(&mut acc, id.clone(), &key, &w.text, &src, None, None, occ_of(w, w));
            if let Some(a) = acc.get_mut(id) {
                // the acronym's own spelling does not become the display text
                *a.forms.entry(w.text.clone()).or_default() -= 1;
            }
            continue;
        }
        if !listable(&w.text) || filters.is_ignored(&w.key) {
            continue;
        }
        let acr = is_acronym(&w.text);
        if !acr && (filters.is_function(&w.text) || filters.is_general(&w.text)) {
            continue;
        }
        add(&mut acc, format!("w:{}", w.key), &w.key, &w.text, if acr { "acronym" } else { "word" }, None, None, occ_of(w, w));
    }

    let unique: HashSet<&str> = ws.iter().map(|w| w.key.as_str()).collect();
    let mut entries: Vec<Noun> = acc
        .into_values()
        .filter(|a| !filters.is_ignored(&a.noun.key) || a.noun.kb.is_some())
        .map(|mut a| {
            // display: the most frequent spelling (ties: the first in sort order, which
            // puts capitalised forms first — "Dice" over "dice")
            let best = a.forms.iter().filter(|(_, n)| **n > 0).max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0))).map(|(f, _)| f.clone());
            a.noun.text = best.unwrap_or_else(|| a.noun.key.clone());
            if a.noun.kb.is_none() {
                a.noun.category = cats.get(&a.noun.key).cloned();
            }
            a.noun
        })
        .collect();
    entries.sort_by(|a, b| a.first_page.cmp(&b.first_page).then(a.occ[0][1].cmp(&b.occ[0][1])));
    let stats = Stats {
        words: ws.len(),
        unique: unique.len(),
        units: entries.len(),
        known: entries.iter().filter(|e| e.state == "known").count(),
        learning: entries.iter().filter(|e| matches!(e.state.as_str(), "learning" | "stale" | "none")).count(),
        no_card: entries.iter().filter(|e| e.state == "new").count(),
        classified: entries.iter().filter(|e| e.state == "new" && e.category.is_some()).count(),
    };
    Inventory { entries, stats }
}

impl Inventory {
    /// The no-card units that still need a category (display texts, for the model);
    /// words the author used in their own papers are not asked about.
    pub fn unclassified(&self) -> Vec<&Noun> {
        self.entries.iter().filter(|e| e.state == "new" && !e.used && e.category.is_none()).collect()
    }

    /// Before the reader's field is set the model cannot tell what is basic for them, so
    /// single words (diabetic, sensitivity ...) would be red by the dozen: only phrases,
    /// hyphenated terms and acronyms stay red until the field is known.
    pub fn quiet_single_words(&mut self) {
        for e in self.entries.iter_mut().filter(|e| e.state == "new" && e.source == "word") {
            e.quiet = true;
        }
    }

    /// Mark the no-card units found in the author's own papers (`my` = my-vocab keys).
    /// A KB card always wins: only `new` units are marked.
    pub fn mark_used(&mut self, my: &HashSet<String>) {
        for e in self.entries.iter_mut().filter(|e| e.state == "new") {
            e.used = my.contains(&e.key) || e.short.as_ref().is_some_and(|s| my.contains(&word_key(s)));
        }
        // my-vocab counts as 記得
        self.stats.known = self.entries.iter().filter(|e| e.state == "known" || e.used).count();
        self.stats.no_card = self.entries.iter().filter(|e| e.state == "new" && !e.used).count();
    }
}

/// The words and terms of one paper for the author's own vocabulary (my-vocab), with no
/// model: every listable word of the body (references cut off) by its normalised key,
/// hyphenated compounds, 2–4-word runs that occur at least twice (no function word at
/// either end, not only general words) and runs that are KB titles or aliases.
/// key → spelling.
pub fn vocabulary(paper: &PaperText, kb: &[KbName], filters: &Filters) -> BTreeMap<String, String> {
    let ws = words(paper);
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for w in &ws {
        if listable(&w.text) && !filters.is_function(&w.text) {
            out.entry(w.key.clone()).or_insert_with(|| w.text.clone());
        }
    }
    let kb_keys: HashSet<String> = kb.iter().map(|n| phrase_key(&n.name)).filter(|k| k.contains(' ')).collect();
    let mut grams: HashMap<String, (usize, String)> = HashMap::new();
    for i in 0..ws.len() {
        for n in 2..=4 {
            if i + n > ws.len() || !(i..i + n - 1).all(|k| ws[k].open_next) {
                break;
            }
            let run = &ws[i..i + n];
            if filters.is_function(&run[0].text) || filters.is_function(&run[n - 1].text) || run.iter().any(|w| !w.text.chars().any(char::is_alphabetic)) {
                continue;
            }
            // "accurately capture": only general words is not a term
            if run.iter().all(|w| filters.is_general(&w.text) || filters.is_function(&w.text)) {
                continue;
            }
            let key = run.iter().map(|w| w.key.as_str()).collect::<Vec<_>>().join(" ");
            let hyph = run[..n - 1].iter().all(|w| w.hy_next);
            let text = run.iter().enumerate().map(|(k, w)| if k + 1 < n && w.hy_next { format!("{}-", w.text) } else if k + 1 < n { format!("{} ", w.text) } else { w.text.clone() }).collect::<String>();
            let e = grams.entry(key).or_insert((0, text));
            e.0 += if hyph { 2 } else { 1 };
        }
    }
    for (key, (count, text)) in grams {
        if count >= 2 || kb_keys.contains(&key) {
            out.entry(key).or_insert(text);
        }
    }
    out
}

/// Level anchors for the classifier from my-vocab: multi-word terms first, then rare
/// single words (not general English), at most `n`.
pub fn vocab_anchors(vocab: &BTreeMap<String, String>, filters: &Filters, n: usize) -> Vec<String> {
    let mut multi: Vec<&String> = vocab.iter().filter(|(k, _)| k.contains(' ')).map(|(_, t)| t).filter(|t| !t.split([' ', '-']).all(|w| filters.is_general(w) || filters.is_function(w))).collect();
    let mut single: Vec<&String> = vocab.iter().filter(|(k, t)| !k.contains(' ') && t.chars().count() >= 6 && !filters.is_general(t)).map(|(_, t)| t).collect();
    multi.sort_by_key(|t| std::cmp::Reverse(t.len()));
    single.sort();
    multi.into_iter().take(n * 2 / 3).chain(single).take(n).cloned().collect()
}

// ── classification (one call per paper) ──────────────────────────────────

pub const CLASSIFY_SYSTEM: &str = "你是學術論文的術語分類器。使用者會給一份從一篇論文整理出來的英文詞表（一行一個），\
以及讀者的領域與程度、讀者已經記得的術語（程度的參考）。目的是找出**這位讀者還需要學**的專業術語。\
把每個詞放進一個分類：方法（模型、演算法、技術、損失函數…）、指標（評估指標、測度）、資料集（資料集、基準、資料庫名稱）、\
統計（統計方法、檢定、統計量）、臨床／解剖（疾病、器官、解剖結構、臨床術語）、其他術語（其他專業術語）、\
基礎科學詞（這個領域的專家早就會的基礎科學、數學、工程詞彙，例如 pixel、vector、gradient、variance、boundary、loss；\
就算是術語，只要對這位讀者是基礎就放這裡）、一般學術詞（任何領域的論文都會用的普通詞）、人名（作者、單獨出現的姓氏）。\
拿不準是不是基礎時，和讀者已經記得的術語比程度。詞要原樣照抄，不要翻譯、不要解釋、不要新增詞。";

/// The user prompt: the reader's field and level, terms they already know (level
/// anchors), the paper title and the words, one per line.
pub fn classify_prompt(title: &str, field: &str, known: &[String], terms: &[&str]) -> String {
    let anchors = if known.is_empty() { "（沒有）".to_string() } else { known.join("、") };
    format!("讀者的領域與程度：{field}\n讀者已經記得的術語（程度參考）：{anchors}\n論文標題：{title}\n\n詞表（{} 個）：\n{}", terms.len(), terms.join("\n"))
}

/// JSON schema of the answer: one array of terms per category.
pub fn classify_schema() -> serde_json::Value {
    let mut props = serde_json::Map::new();
    for c in CATEGORIES {
        props.insert(c.to_string(), serde_json::json!({ "type": "array", "items": { "type": "string" } }));
    }
    serde_json::json!({ "type": "object", "properties": props, "required": CATEGORIES, "additionalProperties": false })
}

/// Read the model's answer into key → category for the asked terms only (a term the
/// model made up or misspelled is dropped; a term in two categories gets the non-red one).
pub fn parse_classification(answer: &serde_json::Value, asked: &[&str]) -> HashMap<String, String> {
    let asked: HashMap<String, ()> = asked.iter().map(|t| (phrase_key(t), ())).collect();
    let mut out = HashMap::new();
    // a term the model put in two categories: the one that is not red wins
    let order = ["人名", "一般學術詞", "基礎科學詞", "方法", "指標", "資料集", "統計", "臨床／解剖", "其他術語"];
    for c in order {
        for t in answer[c].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
            let k = phrase_key(t);
            if asked.contains_key(&k) {
                out.entry(k).or_insert_with(|| c.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb(name: &str, id: &str, state: Option<&str>) -> KbName {
        KbName { name: name.into(), node_id: id.into(), title: name.into(), state: state.map(str::to_string) }
    }

    fn texts(inv: &Inventory) -> Vec<&str> {
        inv.entries.iter().map(|e| e.text.as_str()).collect()
    }

    fn get<'a>(inv: &'a Inventory, key: &str) -> &'a Noun {
        inv.entries.iter().find(|e| e.key == key).unwrap_or_else(|| panic!("no {key} in {:?}", texts(inv)))
    }

    #[test]
    fn normalisation() {
        assert_eq!(word_key("Colours"), "color");
        assert_eq!(word_key("tumours"), "tumor");
        assert_eq!(word_key("normalisation"), "normalization");
        assert_eq!(word_key("Normalization"), "normalization");
        assert_eq!(word_key("analysed"), "analyzed");
        assert_eq!(word_key("centres"), "center");
        assert_eq!(word_key("labelled"), "labeled");
        assert_eq!(word_key("haemorrhage"), "hemorrhage");
        assert_eq!(word_key("oedema"), "edema");
        assert_eq!(word_key("matrices"), "matrix");
        assert_eq!(word_key("analyses"), "analysis");
        assert_eq!(word_key("vessels"), "vessel");
        assert_eq!(word_key("ontologies"), "ontology");
        assert_eq!(word_key("losses"), "loss");
        assert_eq!(word_key("bias"), "bias");
        assert_eq!(word_key("SBLs"), "sbl");
        assert_eq!(word_key("CNNs"), "cnn");
        assert_eq!(word_key("model’s"), "model");
        assert_eq!(word_key("ﬁlter"), "filter");
        assert_eq!(word_key("four"), "four");
        assert_eq!(phrase_key("Cross-Entropy"), "cross entropy");
        assert_eq!(phrase_key("cross  entropies"), "cross entropy");
        assert_eq!(phrase_key("U-Net"), "u net");
    }

    #[test]
    fn ngrams_match_kb_titles_and_aliases() {
        let names = vec![
            kb("Dice coefficient", "n1", Some("known")),
            kb("Sørensen–Dice index", "n1", Some("known")),
            kb("cross entropy", "n2", Some("learning")),
            kb("Hausdorff distance", "n3", None),
            kb("U-Net", "n4", Some("stale")),
            kb("generalized Dice loss function", "n5", Some("known")),
        ];
        let p = PaperText::from_strings(&[
            "We report Dice coefficients and the Hausdorff\ndistance. A cross-entropy loss and a Cross Entropy baseline.\n\
             The U-Net and U-Nets are trained with the Generalised Dice Loss Function.",
        ]);
        let inv = inventory(&p, &names, &Filters::default(), &HashMap::new());
        let dice = get(&inv, "dice coefficient");
        assert_eq!((dice.source.as_str(), dice.state.as_str(), dice.count, dice.text.as_str()), ("kb", "known", 1, "Dice coefficients"));
        assert_eq!(dice.kb.as_ref().unwrap().node_id, "n1");
        // across a line break; a card without learning state
        assert_eq!(get(&inv, "hausdorff distance").state, "none");
        // hyphen and space spellings are one card, counted twice
        let ce = get(&inv, "cross entropy");
        assert_eq!((ce.count, ce.state.as_str()), (2, "learning"));
        assert_eq!(get(&inv, "u net").count, 2);
        // a four-word alias with British spelling and case differences
        assert_eq!(get(&inv, "generalized dice loss function").kb.as_ref().unwrap().node_id, "n5");
        // words inside a KB phrase are not listed again on their own
        assert!(!inv.entries.iter().any(|e| e.key == "dice" || e.key == "hausdorff" || e.key == "coefficient"), "{:?}", texts(&inv));
        // offsets are UTF-16 into the page text
        let u: Vec<u16> = p.pages[0].text.encode_utf16().collect();
        let o = dice.occ[0];
        assert_eq!(String::from_utf16(&u[o[1] as usize..o[2] as usize]).unwrap(), "Dice coefficients");
        let o = get(&inv, "hausdorff distance").occ[0];
        assert_eq!(String::from_utf16(&u[o[1] as usize..o[2] as usize]).unwrap(), "Hausdorff\ndistance");
    }

    #[test]
    fn filters_ignore_list_hyphenation_and_acronyms() {
        let p = PaperText::from_strings(&[
            "We propose a Spectral Boundary Loss (SBL) for thin vessels. The SBL improves results significantly.\n\
             Anisotropic diffusion regularises the masks; anisotropic smoothing and label-smoothing help. The segmen-\n\
             tation is evaluated with heteroscedastic noise and state-of-the-art baselines by Hausdorff and Sobel.",
            "More segmentation. Then the colour and color maps. Wavelet-\nbased and wavelet-based pre-training.\nReferences\n[1] Quaternion Wavelets. Journal of Examples.",
        ]);
        let f = Filters::new(&["heteroscedastic  # mine".into(), "Sobel".into()]);
        let inv = inventory(&p, &[], &f, &HashMap::new());
        let t = texts(&inv);
        // general and function words are gone, specialist ones stay
        for gone in ["propose", "results", "significantly", "the", "thin", "state-of-the-art", "heteroscedastic", "Sobel", "Quaternion", "Wavelets", "maps"] {
            assert!(!t.iter().any(|x| x.eq_ignore_ascii_case(gone)), "{gone} in {t:?}");
        }
        for kept in ["anisotropic", "diffusion", "Hausdorff", "label-smoothing"] {
            assert!(t.iter().any(|x| x.eq_ignore_ascii_case(kept)), "{kept} not in {t:?}");
        }
        // the expansion is one unit and the acronym (also inside the parentheses) counts for it
        let sbl = get(&inv, "spectral boundary loss");
        assert_eq!((sbl.source.as_str(), sbl.short.as_deref(), sbl.count, sbl.text.as_str()), ("expansion", Some("SBL"), 3, "Spectral Boundary Loss"));
        assert!(!inv.entries.iter().any(|e| e.key == "sbl"));
        // line-break hyphenation: joined because "segmentation" is written whole elsewhere
        let seg = get(&inv, "segmentation");
        assert_eq!(seg.count, 2);
        // "Wavelet-\nbased" stays hyphenated: "wavelet-based" appears on one line
        assert_eq!(get(&inv, "wavelet based").count, 2);
        assert!(!inv.entries.iter().any(|e| e.key == "waveletbased"));
        // British and American spellings are one unit; nothing after References
        assert!(!t.iter().any(|x| x.contains("Quaternion")));
        let st = &inv.stats;
        assert!(st.unique > st.units && st.no_card == st.units, "{st:?}");
        assert!(!get(&inv, "anisotropic").red(), "unclassified words are never red");
    }

    #[test]
    fn classification_round_trip() {
        let p = PaperText::from_strings(&["The anisotropic Kaggle datasets by Hausdorff, scored with mIoU and dropout."]);
        let inv = inventory(&p, &[], &Filters::default(), &HashMap::new());
        let asked: Vec<&str> = inv.unclassified().iter().map(|n| n.text.as_str()).collect();
        assert!(asked.contains(&"Hausdorff") && asked.contains(&"mIoU"), "{asked:?}");
        let answer = serde_json::json!({ "人名": ["Hausdorff", "Invented"], "指標": ["mIoU"], "其他術語": ["Hausdorff"], "方法": ["dropout"], "一般學術詞": [] });
        let cats = parse_classification(&answer, &asked);
        assert_eq!(cats.get("hausdorff").map(String::as_str), Some("人名"));
        assert!(!cats.contains_key("invented"));
        let inv = inventory(&p, &[], &Filters::default(), &cats);
        assert!(!get(&inv, "hausdorff").red(), "names are listed, not red");
        assert!(get(&inv, "miou").red());
        assert_eq!(inv.stats.classified, 3);
        let s = classify_schema();
        assert_eq!(s["required"].as_array().unwrap().len(), CATEGORIES.len());
        assert!(classify_prompt("T", DEFAULT_FIELD, &["Dice coefficient".into()], &asked).contains("mIoU"));
        // basic science words and unclassified words are never red
        let cats2 = parse_classification(&serde_json::json!({ "基礎科學詞": ["dropout"] }), &asked);
        let inv2 = inventory(&p, &[], &Filters::default(), &cats2);
        assert!(!get(&inv2, "dropout").red());
        assert!(!get(&inv2, "miou").red(), "unclassified: not red");
    }

    /// The target scenario: a synthetic paper of about 8,000 words reduces to a few
    /// hundred units; KB phrases are matched as units; general words are gone.
    #[test]
    fn long_paper_reduces_to_a_few_hundred_units() {
        let pages: Vec<String> = crate::synth::long_paper_lines(7).iter().map(|ls| ls.iter().map(|l| l.trim_start_matches("# ")).collect::<Vec<_>>().join("\n")).collect();
        let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
        let p = PaperText::from_strings(&refs);
        let names = vec![kb("Dice coefficient", "n1", Some("known")), kb("Hausdorff distance", "n2", Some("learning")), kb("Wilcoxon signed-rank test", "n3", Some("known")), kb("focal loss", "n4", Some("known"))];
        let inv = inventory(&p, &names, &Filters::default(), &HashMap::new());
        let st = &inv.stats;
        eprintln!("{st:?}");
        assert!((7_000..11_000).contains(&st.words), "{st:?}");
        assert!((100..=500).contains(&st.units), "a few hundred units: {st:?}");
        assert!(st.units * 2 < st.unique, "{st:?}");
        assert_eq!(st.known + st.learning, 4, "{st:?}");
        let w = inv.entries.iter().find(|e| e.key.starts_with("wilcoxon")).map(|e| (e.key.clone(), e.count));
        assert_eq!(w.as_ref().map(|x| x.0.as_str()), Some("wilcoxon signed rank test"), "{w:?}");
        // the references are not read
        assert!(!inv.entries.iter().any(|e| e.key.contains("quaternion")));
        for general in ["implementation", "procedure", "framework", "result", "approach"] {
            assert!(!inv.entries.iter().any(|e| e.key == general), "{general}");
        }
    }
}

#[cfg(test)]
mod red_tests {
    use super::*;

    /// A MOCK classifier (rules, not a model) that behaves like a sensible one for a
    /// reader in medical image analysis: basic science words → 基礎科學詞, names → 人名,
    /// clinical words → 臨床／解剖, the rest → 方法. `basic: false` = the previous
    /// categories, where basic words had to go to 其他術語.
    fn mock_classify(terms: &[&str], basic: bool) -> serde_json::Value {
        const BASIC: &[&str] = &["mask", "pixel", "boundary", "curvature", "momentum", "vector", "variance", "gradient", "noise", "filter", "signal", "frequency", "loss", "label", "segmentation", "entropy", "kernel", "tensor", "probability", "distribution", "sampling", "threshold", "regression", "sigmoid", "softmax", "logit", "encoder", "decoder", "embedding", "dropout", "convolution", "pooling", "residual", "prior", "likelihood", "posterior", "uncertainty", "ensemble", "calibration", "dilation", "erosion", "morphology", "voxel", "upsampling", "downsampling", "overfitting", "underfitting", "warmup", "annealing", "regularizer", "sparsity", "quantile", "hyperparameter", "backpropagation", "manifold", "interpolation", "spline", "mesh", "heatmap", "attention", "transformer", "accuracy", "precision", "recall"];
        // basic for this reader too: everyday deep-learning, statistics and anatomy words
        const BASIC2: &[&str] = &["autoencoder", "augmentation", "stochastic", "diffusion", "bootstrap", "bootstrapping", "cosine", "descent", "graph", "topology", "annotation", "supervision", "cluster", "pretraining", "divergence", "deviation", "anisotropic", "isotropy", "anisotropy", "denoising", "registration", "deformation", "bayesian", "adversarial", "contrastive", "permutation", "imputation", "skewness", "kurtosis", "autocorrelation", "quantization", "pruning", "bottleneck", "batchnorm", "tokenizer", "critic", "wavelet", "spectrogram", "pointcloud", "octree", "geodesic", "eigendecomposition", "interval", "correction", "intersection", "statistical", "volumetric", "saliency", "occlusion", "inpainting", "ensembling", "boosting", "lasso", "ridge", "elastic net", "skip connection", "superpixel", "focal", "curriculum", "distillation", "regime", "formulation", "protocol", "imbalance", "stratification", "noisy", "dice", "coefficient", "spectral", "hepatic", "pulmonary", "renal", "retinal", "coronary", "colonic", "optic", "disc", "lesion", "nodule", "polyp", "cortex", "ventricle", "artery", "vasculature", "stenosis", "perfusion", "angiography", "clinician", "centerline", "bifurcation", "tortuous", "tetrahedral", "monte", "carlo", "brier", "kappa", "auroc", "graphlet", "imputation", "deformation"];
        const NAMES: &[&str] = &["hausdorff", "sobel", "kullback", "gaussian", "laplacian", "wilcoxon", "bonferroni", "cohen", "gumbel", "wasserstein", "ellrin", "daidai", "jacobian", "hessian"];
        let mut out = serde_json::Map::new();
        for c in CATEGORIES {
            out.insert(c.into(), serde_json::json!([]));
        }
        for t in terms {
            let k = phrase_key(t);
            let c = if NAMES.iter().any(|n| k.split(' ').any(|w| w == *n)) && !k.contains(' ') {
                "人名"
            } else if BASIC.iter().chain(BASIC2).any(|b| k == *b) {
                if basic { "基礎科學詞" } else { "其他術語" }
            } else if (["ises", "izes", "ifies", "ates", "ains", "ated", "ized", "ised"].iter().any(|e| k.ends_with(e)) || ["constrain", "simplify", "stabilize", "regularize", "accelerate", "generalize", "calibrate", "outperform", "complement"].contains(&k.as_str())) && !k.contains(' ') {
                "一般學術詞"
            } else if ["graphy", "pathy", "sclerosis", "lysis", "trophy", "genesis", "metry", "plasty", "artery", "lesion", "nodule", "polyp", "cortex", "stenosis", "perfusion", "vasculature", "ventricle"].iter().any(|s| k.contains(s)) {
                "臨床／解剖"
            } else {
                "方法"
            };
            out.get_mut(c).unwrap().as_array_mut().unwrap().push(serde_json::Value::String(t.to_string()));
        }
        serde_json::Value::Object(out)
    }

    #[test]
    fn red_means_a_term_to_learn_not_a_word_without_a_card() {
        let pages: Vec<String> = crate::synth::long_paper_lines(7).iter().map(|ls| ls.iter().map(|l| l.trim_start_matches("# ")).collect::<Vec<_>>().join("\n")).collect();
        let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
        let p = PaperText::from_strings(&refs);
        let f = Filters::default();
        let unclassified = inventory(&p, &[], &f, &HashMap::new());
        assert_eq!(unclassified.entries.iter().filter(|e| e.red()).count(), 0, "unclassified words are never red");
        let asked: Vec<&str> = unclassified.unclassified().iter().map(|n| n.text.as_str()).collect();
        let before = inventory(&p, &[], &f, &parse_classification(&mock_classify(&asked, false), &asked));
        let after = inventory(&p, &[], &f, &parse_classification(&mock_classify(&asked, true), &asked));
        let red = |i: &Inventory| i.entries.iter().filter(|e| e.red()).count();
        let all_new = unclassified.entries.iter().filter(|e| e.state == "new").count();
        eprintln!("要學 (MOCK classifier) — every no-card unit: {all_new}; before (no 基礎科學詞): {}; after: {}", red(&before), red(&after));
        if std::env::var("LR_SHOW_RED").is_ok() { eprintln!("{:?}", after.entries.iter().filter(|e| e.red()).map(|e| (e.text.as_str(), e.category.as_deref().unwrap_or(""))).collect::<Vec<_>>()); }
        assert!(red(&after) * 10 < red(&before) * 8, "{} → {}", red(&before), red(&after));
        for basic in ["mask", "pixel", "boundary", "curvature", "momentum", "vector", "variance"] {
            if let Some(e) = after.entries.iter().find(|e| e.key == basic) {
                assert!(!e.red(), "{basic} is basic for this reader");
            }
        }
        // my-vocab: words the author used are never red; a KB card still wins
        let mut v = after.clone();
        v.mark_used(&["tortuosity".to_string(), "skeletonization".to_string()].into_iter().collect());
        assert!(v.entries.iter().find(|e| e.key == "tortuosity").is_some_and(|e| e.used && !e.red()));
        assert_eq!(red(&v) + 2, red(&after));
        assert_eq!(v.stats.known, after.stats.known + 2, "my-vocab counts as 記得");
    }

    #[test]
    fn vocabulary_of_own_papers() {
        let p = PaperText::from_strings(&["We measure vessel tortuosity with a skeletonization step.\nThe skeletonization step and vessel tortuosity again.\nReferences\n[1] Quaternion Wavelets."]);
        let kb = vec![KbName { name: "Dice coefficient".into(), node_id: "n".into(), title: "Dice coefficient".into(), state: None }];
        let v = vocabulary(&p, &kb, &Filters::default());
        assert!(v.contains_key("tortuosity") && v.contains_key("skeletonization"));
        assert!(v.contains_key("vessel tortuosity") && v.contains_key("skeletonization step"), "{v:?}");
        assert!(!v.contains_key("quaternion"), "references are not read");
        assert!(!v.contains_key("the"));
        let anchors = vocab_anchors(&v, &Filters::default(), 10);
        assert!(anchors.iter().any(|a| a.contains("tortuosity")), "{anchors:?}");
    }
}
