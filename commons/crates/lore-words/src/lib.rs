//! lore-words — the word-frequency filter for domain terms, shared by the apps that pick
//! a paper's terms (LoreReader's noun list, SlideComposer's 「不熟」 marks).
//!
//! * [`Filters`]: function words (`function_words.txt`), general academic words
//!   (`academic_words.txt`), everyday English (`common_words.txt`, except [`TERMLIKE`]
//!   words that are terms in research papers), the most common English words
//!   (`english_common.txt`, SCOWL ≤ 20), fairly common ones (`english_familiar.txt`,
//!   SCOWL 35; see `english_words.LICENSE`) and an ignore list.
//! * Normalisers: [`word_key`] / [`phrase_key`] (case, plurals, British/American
//!   spelling, ligatures, possessives, acronym plurals), [`is_acronym`].
//!
//! The lists and rules are the ones LoreReader's `nouns.rs` uses (copied from it; it can
//! switch to this crate).

use std::collections::HashSet;

const COMMON: &str = include_str!("common_words.txt");
const ACADEMIC: &str = include_str!("academic_words.txt");
const FUNCTION: &str = include_str!("function_words.txt");
const ENGLISH_COMMON: &str = include_str!("english_common.txt");
const ENGLISH_FAMILIAR: &str = include_str!("english_familiar.txt");

/// Everyday words that are specialist terms in research papers: never dropped as
/// "general" (they still go to the model, which can call them 一般學術詞).
pub const TERMLIKE: &[&str] = &[
    "accuracy", "precision", "recall", "sensitivity", "specificity", "regression", "variance", "correlation", "deviation", "entropy", "kernel",
    "attention", "distribution", "inference", "probability", "threshold", "vector", "convergence", "optimization", "gradient", "sampling",
    "simulation", "transformation", "representation", "reconstruction", "classification", "cluster", "clustering", "frequency", "signal",
    "noise", "filter", "metric", "statistic", "algorithm", "regularization", "interpolation", "segmentation", "embedding", "encoder", "decoder",
    "likelihood", "posterior", "prior", "residual", "convolution", "pooling", "dropout", "momentum", "boundary", "loss", "label", "mask",
];

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
    familiar: HashSet<String>,
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
        general.extend(list(ENGLISH_COMMON));
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
        Filters { function: list(FUNCTION), general, termlike, ignore: ig, familiar: list(ENGLISH_FAMILIAR) }
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

    /// A fairly common English word (allowing for inflections), never a [`TERMLIKE`] one.
    pub fn is_familiar(&self, w: &str) -> bool {
        let forms = variants(w);
        !forms.iter().any(|f| self.termlike.contains(f)) && forms.iter().any(|f| self.familiar.contains(f))
    }

    /// An everyday word for someone in the field: any general, familiar or [`TERMLIKE`] word
    /// (`loss`, `mask`, `segmentation`). A single such word is a word the author already reads
    /// without stopping, never a card; hyphenated words count when every part is one.
    pub fn is_everyday(&self, w: &str) -> bool {
        let parts: Vec<&str> = w.split(is_hyphen).filter(|p| !p.is_empty()).collect();
        !parts.is_empty()
            && parts.iter().all(|p| {
                if parts.len() > 1 && ["pre", "post", "non", "re", "co", "multi", "semi", "self", "sub", "over", "under", "cross", "inter", "intra", "high", "low"].contains(&p.to_lowercase().as_str()) {
                    return true;
                }
                let forms = variants(p);
                forms.iter().any(|f| self.general.contains(f) || self.function.contains(f) || self.familiar.contains(f) || self.termlike.contains(f))
            })
    }

    pub fn is_ignored(&self, key: &str) -> bool {
        self.ignore.contains(key)
    }

    /// Every general and familiar word with one of the ligatures a PDF often loses (`fi`, `ff`,
    /// `fl`, `ffi`, `ffl`, `ft`) taken out: `significant` → `signicant`. A word in this set is a
    /// word broken by its PDF, not a term.
    pub fn ligature_losses(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for w in self.general.iter().chain(self.familiar.iter()) {
            for lig in ["ffi", "ffl", "fi", "ff", "fl", "ft"] {
                let mut from = 0;
                while let Some(i) = w[from..].find(lig) {
                    let at = from + i;
                    out.insert(format!("{}{}", &w[..at], &w[at + lig.len()..]));
                    from = at + 1;
                }
            }
        }
        out.retain(|x| x.len() >= 3 && !self.general.contains(x) && !self.familiar.contains(x));
        out
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

pub fn is_hyphen(c: char) -> bool {
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

/// An acronym: two or more capitals, only capitals and digits (a plural `s` allowed), at most 10 chars.
pub fn is_acronym(w: &str) -> bool {
    let core = w.strip_suffix('s').filter(|c| c.chars().filter(|x| x.is_uppercase()).count() >= 2).unwrap_or(w);
    core.chars().filter(|c| c.is_uppercase()).count() >= 2 && core.chars().all(|c| c.is_uppercase() || c.is_ascii_digit()) && core.chars().count() <= 10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_words_drop_terms_stay() {
        let f = Filters::default();
        assert!(f.is_function("the") && f.is_general("results") && f.is_general("method"));
        assert!(!f.is_general("segmentation") && !f.is_general("entropy"), "TERMLIKE words are never general");
        assert!(!f.is_general("hippocampus"));
        assert!(Filters::new(&["foo bar".into()]).is_ignored("foo bar"));
        assert!(Filters::new(&["loss  # LoreShelf".into()]).is_ignored("loss"), "comments after a word");
    }

    #[test]
    fn everyday_words_include_termlike_ones() {
        let f = Filters::default();
        for w in ["loss", "Masks", "segmentation", "pre-trained"] { assert!(f.is_everyday(w), "{w}"); }
        for w in ["discriminator", "glomeruli", "U-Net", "softmax"] { assert!(!f.is_everyday(w), "{w}"); }
    }

    #[test]
    fn keys_group_spellings() {
        assert_eq!(word_key("SBLs"), "sbl");
        assert_eq!(word_key("tumours"), "tumor");
        assert_eq!(phrase_key("Cross-Entropy losses"), "cross entropy loss");
        assert!(is_acronym("SBL") && is_acronym("CNNs") && !is_acronym("Loss") && !is_acronym("A"));
    }
}
