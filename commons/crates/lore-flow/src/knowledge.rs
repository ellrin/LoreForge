//! The author's know-how about figures, as a model is shown it (`docs/flow-figures.md` §8).
//!
//! The flow tool draws; what to draw comes from knowledge the author keeps in L-Brain: every
//! component, module and whole figure there says what it is and when to use it (its prompt),
//! which variables it takes and gives, and how its parts connect. An app reads that from the
//! hub and hands it in as a [`Knowledge`] (this crate does not know the hub). Drafting a new
//! figure then picks from it by the variables and words of the description ([`pick_modules`],
//! [`pick_examples`]) and every pick says why it was picked, so the author can see and correct
//! what the model was told.
//!
//! The types are FROZEN (the seam between this crate, lore-know's figure format and the apps).

use serde::{Deserialize, Serialize};

/// The words a variable's kind may be. A variable of an unknown kind is `other`.
pub const VAR_KINDS: [&str; 12] = ["image", "volume", "video", "signal", "mask", "label", "vector", "feature", "scalar", "text", "loss", "other"];

/// One variable a piece takes or gives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Var {
    /// A notation id (`x`, `y_hat`) or a plain name when it has none.
    pub id: String,
    /// One of [`VAR_KINDS`].
    pub kind: String,
    /// What it is, in a few words.
    pub meaning: String,
}

/// A component or module the model may put in a figure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModuleNote {
    /// The module kind of the DSL (`encoder`, `unet`, a saved drawing's kind).
    pub kind: String,
    /// The art style; empty = the kind as a whole.
    pub style: String,
    pub title: String,
    /// The author's words: what it is and when to use it. Empty = he has not said.
    pub prompt: String,
    pub inputs: Vec<Var>,
    pub outputs: Vec<Var>,
    /// Domain and kind tags (`segmentation`, `attention` …).
    pub tags: Vec<String>,
}

/// A whole figure the author made: a worked example for the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Example {
    /// The hub item's id (shown to the author, never to the model).
    pub id: String,
    pub title: String,
    /// What the figure shows (the description it was made from, or the author's prompt).
    pub description: String,
    /// The figure's L1 text.
    pub dsl: String,
    pub inputs: Vec<Var>,
    pub outputs: Vec<Var>,
    pub tags: Vec<String>,
}

/// Everything an app hands in for one draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Knowledge {
    pub modules: Vec<ModuleNote>,
    pub examples: Vec<Example>,
}

/// Something picked, with the reason in one short line the author can read
/// (`takes an image and gives a mask, like the description`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Picked<T> {
    pub item: T,
    pub score: f32,
    pub why: String,
}

/// Word lists per variable kind: English (whole words, case-insensitive; a plural `s` is
/// accepted) and Traditional Chinese (anywhere in the text).
const KIND_WORDS: [(&str, &[&str]); 11] = [
    ("image", &["image", "photo", "x-ray", "radiograph", "影像", "圖片"]),
    ("volume", &["volume", "ct", "mri", "3d", "體積"]),
    ("video", &["video", "frame", "影片"]),
    ("signal", &["signal", "ecg", "eeg", "audio", "訊號"]),
    ("mask", &["mask", "segmentation", "分割", "遮罩"]),
    ("label", &["label", "class", "classification", "diagnosis", "標籤", "分類"]),
    ("vector", &["vector", "embedding", "向量"]),
    ("feature", &["feature", "feature map", "特徵"]),
    ("scalar", &["score", "probability", "分數", "機率"]),
    ("text", &["text", "report", "caption", "文字", "報告"]),
    ("loss", &["loss", "損失"]),
];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The byte position of the first whole-word occurrence of `term` in `text` (both lower case).
/// A term with a non-ASCII letter matches anywhere; an ASCII one may carry a plural `s`.
fn find_word(text: &str, term: &str) -> Option<usize> {
    if term.is_empty() {
        return None;
    }
    if !term.is_ascii() {
        return text.find(term);
    }
    let mut from = 0;
    while let Some(i) = text[from..].find(term) {
        let start = from + i;
        let end = start + term.len();
        let before_ok = text[..start].chars().next_back().is_none_or(|c| !is_word_char(c));
        let rest = &text[end..];
        let after_ok = match rest.chars().next() {
            None => true,
            Some('s') => rest[1..].chars().next().is_none_or(|c| !is_word_char(c)),
            Some(c) => !is_word_char(c),
        };
        if before_ok && after_ok {
            return Some(start);
        }
        from = start + text[start..].chars().next().map_or(1, |c| c.len_utf8());
    }
    None
}

/// What the description is about, as far as code can tell: the variable kinds it names.
/// Pure and cheap (word lists, no model): `CT volume` → `volume`, `segmentation mask` → `mask`.
/// Order of first appearance, no duplicates.
pub fn kinds_in(description: &str) -> Vec<String> {
    let text = description.to_lowercase();
    let mut found: Vec<(usize, usize, &str)> = Vec::new();
    for (order, (kind, words)) in KIND_WORDS.iter().enumerate() {
        if let Some(pos) = words.iter().filter_map(|w| find_word(&text, w)).min() {
            found.push((pos, order, kind));
        }
    }
    found.sort();
    found.into_iter().map(|(_, _, k)| k.to_string()).collect()
}

/// Distinct words of at least four letters, lower case.
fn long_words(text: &str) -> std::collections::BTreeSet<String> {
    text.to_lowercase().split(|c: char| !c.is_alphabetic()).filter(|w| w.chars().count() >= 4).map(str::to_string).collect()
}

/// Score and reason of one candidate against the description.
fn score(description: &str, wanted: &[String], inputs: &[Var], outputs: &[Var], tags: &[String], text: &str) -> Option<(f32, String)> {
    let kinds_of = |vars: &[Var]| -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for x in vars {
            if wanted.contains(&x.kind) && !v.contains(&x.kind) {
                v.push(x.kind.clone());
            }
        }
        v
    };
    let (takes, gives) = (kinds_of(inputs), kinds_of(outputs));
    let mut shared: Vec<&String> = takes.iter().collect();
    for g in &gives {
        if !shared.contains(&g) {
            shared.push(g);
        }
    }
    let lower = description.to_lowercase();
    let tags_hit: Vec<String> = tags.iter().filter(|t| !t.trim().is_empty() && find_word(&lower, &t.trim().to_lowercase()).is_some()).cloned().collect();
    let mine = long_words(description);
    let words: Vec<String> = long_words(text).into_iter().filter(|w| mine.contains(w)).take(6).collect();
    let total = 3.0 * shared.len() as f32 + tags_hit.len() as f32 + 0.5 * words.len() as f32;
    if total <= 0.0 {
        return None;
    }
    // the reason names at most three things
    let mut left = 3usize;
    let mut take_some = |label: &str, items: Vec<String>| -> Option<String> {
        let items: Vec<String> = items.into_iter().take(left).collect();
        left -= items.len();
        if items.is_empty() { None } else { Some(format!("{label}{}", items.join(", "))) }
    };
    let flow: Vec<String> = [take_some("takes ", takes), take_some("gives ", gives)].into_iter().flatten().collect();
    let more: Vec<String> = [take_some("tags: ", tags_hit), take_some("words: ", words)].into_iter().flatten().collect();
    let mut why = flow.join(", ");
    for m in more {
        if !why.is_empty() {
            why.push_str("; ");
        }
        why.push_str(&m);
    }
    Some((total, why))
}

fn rank<T>(mut v: Vec<(Picked<&T>, String)>, limit: usize) -> Vec<Picked<&T>> {
    v.sort_by(|a, b| b.0.score.total_cmp(&a.0.score).then_with(|| a.1.cmp(&b.1)));
    v.into_iter().take(limit).map(|x| x.0).collect()
}

/// The made figures closest to the description, best first, at most `limit`: ranked by the
/// variable kinds they share with it (inputs and outputs), then by shared tags and words. A
/// figure that shares nothing is not picked. Deterministic.
pub fn pick_examples<'a>(description: &str, k: &'a Knowledge, limit: usize) -> Vec<Picked<&'a Example>> {
    let wanted = kinds_in(description);
    let v = k
        .examples
        .iter()
        .filter_map(|e| {
            let text = format!("{} {}", e.title, e.description);
            let (score, why) = score(description, &wanted, &e.inputs, &e.outputs, &e.tags, &text)?;
            Some((Picked { item: e, score, why }, e.id.clone()))
        })
        .collect();
    rank(v, limit)
}

/// The modules worth a line of their own in the prompt, best first, at most `limit`: those
/// whose prompt is not empty and whose variables, tags or words meet the description.
pub fn pick_modules<'a>(description: &str, k: &'a Knowledge, limit: usize) -> Vec<Picked<&'a ModuleNote>> {
    let wanted = kinds_in(description);
    let v = k
        .modules
        .iter()
        .filter(|m| !m.prompt.trim().is_empty())
        .filter_map(|m| {
            let text = format!("{} {}", m.title, m.prompt);
            let (score, why) = score(description, &wanted, &m.inputs, &m.outputs, &m.tags, &text)?;
            Some((Picked { item: m, score, why }, format!("{}\u{0}{}", m.kind, m.style)))
        })
        .collect();
    rank(v, limit)
}

/// `id | kind | meaning`, one variable per line: the text form the hub stores. Lines that are
/// empty or start with `#` are skipped; a missing kind is `other`.
pub fn vars_from_text(text: &str) -> Vec<Var> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.splitn(3, '|').map(str::trim);
            let id = it.next().unwrap_or("");
            if id.is_empty() {
                return None;
            }
            let kind = it.next().unwrap_or("").to_lowercase();
            let kind = if VAR_KINDS.contains(&kind.as_str()) { kind } else { "other".to_string() };
            Some(Var { id: id.to_string(), kind, meaning: it.next().unwrap_or("").to_string() })
        })
        .collect()
}

/// The inverse of [`vars_from_text`].
pub fn vars_to_text(vars: &[Var]) -> String {
    vars.iter()
        .map(|v| if v.meaning.is_empty() { format!("{} | {}", v.id, v.kind) } else { format!("{} | {} | {}", v.id, v.kind, v.meaning) })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(id: &str, kind: &str) -> Var {
        Var { id: id.into(), kind: kind.into(), meaning: String::new() }
    }
    fn fig(id: &str, title: &str, ins: &[(&str, &str)], outs: &[(&str, &str)], tags: &[&str]) -> Example {
        Example {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            dsl: "fig f\n".into(),
            inputs: ins.iter().map(|(a, b)| var(a, b)).collect(),
            outputs: outs.iter().map(|(a, b)| var(a, b)).collect(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn variables_round_trip_through_their_text_form() {
        let vars = vec![Var { id: "x".into(), kind: "volume".into(), meaning: "a CT scan".into() }, var("y_hat", "mask")];
        let text = vars_to_text(&vars);
        assert_eq!(text, "x | volume | a CT scan\ny_hat | mask");
        assert_eq!(vars_from_text(&text), vars);
        let loose = vars_from_text("# inputs\n\n  x |image|  a photo \n");
        assert_eq!(loose, vec![Var { id: "x".into(), kind: "image".into(), meaning: "a photo".into() }]);
    }

    #[test]
    fn an_unknown_kind_is_other() {
        let v = vars_from_text("a | banana | odd\nb\nc |  | none");
        assert_eq!(v.iter().map(|x| x.kind.as_str()).collect::<Vec<_>>(), ["other", "other", "other"]);
        assert_eq!(v[1].id, "b");
        assert_eq!(v[0].meaning, "odd");
    }

    #[test]
    fn a_description_names_its_variable_kinds() {
        assert_eq!(kinds_in("A CT volume goes through an encoder and gives a segmentation mask"), ["volume", "mask"]);
        // whole words only: "classic" is not "class", "act" is not "ct"
        assert!(kinds_in("a classic act").is_empty());
        assert_eq!(kinds_in("Frames of a video, then a loss"), ["video", "loss"]);
    }

    #[test]
    fn chinese_words_count_too() {
        assert_eq!(kinds_in("輸入影像，輸出分類標籤與損失"), ["image", "label", "loss"]);
    }

    #[test]
    fn a_figure_that_shares_the_variables_is_picked_first_and_says_why() {
        let k = Knowledge {
            modules: vec![],
            examples: vec![
                fig("b", "Other", &[("x", "signal")], &[("y", "label")], &["segmentation"]),
                fig("a", "Seg", &[("x", "volume")], &[("y", "mask")], &["segmentation"]),
            ],
        };
        let d = "A CT volume goes into a network that gives a segmentation mask";
        let p = pick_examples(d, &k, 5);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].item.id, "a");
        assert_eq!(p[0].why, "takes volume, gives mask; tags: segmentation");
        assert!(p[0].score > p[1].score);
        assert_eq!(pick_examples(d, &k, 1).len(), 1);
    }

    #[test]
    fn a_figure_that_shares_nothing_is_not_picked() {
        let k = Knowledge { modules: vec![], examples: vec![fig("a", "Zzz", &[("x", "signal")], &[], &["audio"])] };
        assert!(pick_examples("A photo goes in", &k, 5).is_empty());
    }

    #[test]
    fn only_modules_the_author_wrote_about_are_picked() {
        let m = |kind: &str, prompt: &str| ModuleNote { kind: kind.into(), prompt: prompt.into(), inputs: vec![var("x", "volume")], ..Default::default() };
        let k = Knowledge { modules: vec![m("unet", "Use for 3D segmentation"), m("encoder", "")], examples: vec![] };
        let p = pick_modules("a CT volume is encoded", &k, 5);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].item.kind, "unet");
        assert_eq!(p[0].why, "takes volume");
    }

    #[test]
    fn picking_is_the_same_every_time() {
        let k = Knowledge { modules: vec![], examples: vec![fig("b", "", &[("x", "image")], &[], &[]), fig("a", "", &[("x", "image")], &[], &[])] };
        let ids = |k: &Knowledge| pick_examples("an image", k, 5).iter().map(|p| p.item.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&k), ["a", "b"]);
        assert_eq!(ids(&k), ids(&k));
    }
}
