//! 收件: knowledge the author hands over faster than it can be worked on (docs/brain-hub.md §16).
//! He reads sentences aloud to L-Echo, or selects them in a paper, one after another; each lands
//! here as an [`Entry`] and waits. A worker takes them in order: a [`Processor`] turns the raw
//! words into an [`Item`] of the target category (a sentence becomes a pattern with its holes
//! cut, placed on the family's tree, with its tone). What is ready waits again, for him: he
//! reads it, changes it if he likes, and 加入 writes it; 略過 drops it. Nothing reaches the
//! knowledge without that.
//!
//! The shapes in this file are the FROZEN contract between the inbox store (package I1), the
//! processors (I2), L-Brain's server and worker (I3), L-Echo (I4) and the pages.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Item;

/// Where an entry is in its life.
pub mod state {
    /// Handed over, not worked on yet.
    pub const WAITING: &str = "waiting";
    /// The worker has it.
    pub const WORKING: &str = "working";
    /// Worked out: `result` holds the item the author may add.
    pub const READY: &str = "ready";
    /// The worker could not (`error` says why in one line); 重試 puts it back to waiting.
    pub const FAILED: &str = "failed";
    /// The author added it (`added` = the stored item's id).
    pub const ADDED: &str = "added";
    /// The author passed.
    pub const SKIPPED: &str = "skipped";
}

/// How the words came in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct From {
    /// `voice` (read aloud to L-Echo) · `selection` (selected in a Lore app) · `typed`.
    pub via: String,
    /// The Lore app the words were selected in (`lr`, `pc` …); "" for voice.
    pub app: String,
    /// The title of the document they were selected in, when the app gave one (never a path).
    pub doc: String,
}

/// One thing handed over.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// `i` + a sortable id (`new_local`-style); the file is `<hub>/inbox/<id>.json`.
    pub id: String,
    /// The category the result goes into (`pc.patterns`).
    pub category: String,
    /// The raw words, as heard or selected (one sentence for a pattern).
    pub text: String,
    /// The author says these are his own words: only then may the result keep them as an example
    /// (others' sentences give their pattern, never their wording).
    pub own: bool,
    pub from: From,
    /// [`state`].
    pub state: String,
    /// The item worked out (id empty), when `ready`.
    pub result: Option<Item>,
    /// Existing items the result looks like (ids), so the author does not add a near copy.
    pub similar: Vec<String>,
    /// One line, Traditional Chinese, when `failed`.
    pub error: String,
    /// The stored item's id, when `added`.
    pub added: String,
    /// The engine that worked on it (shown small beside the result).
    pub engine: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Counts for the pages (the cover's 待你同意, L-Echo's running line).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub waiting: usize,
    pub working: usize,
    pub ready: usize,
    pub failed: usize,
}

/// What a processor is told besides the entry: what the target family already holds.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The family's tree (`reader::tree`), or `Null`.
    pub tree: Value,
    /// A few existing items of the category, as examples of how a result should look and as
    /// candidates for `similar` (the caller picks them; at most a few dozen).
    pub existing: Vec<Item>,
}

/// One model request (the worker runs it through the app's own engines).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    pub system: String,
    pub user: String,
    /// JSON schema of the reply.
    pub schema: Value,
}

/// What a processor makes of a reply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Made {
    pub item: Item,
    pub similar: Vec<String>,
}

/// Turns raw words into an item of one format. Pure: it writes the request and reads the reply;
/// the worker makes the call. One per format (`pattern` first).
pub trait Processor: Send + Sync {
    /// The format id it makes (`crate::format::PATTERN`).
    fn format(&self) -> &'static str;
    /// The request for `entry`.
    fn request(&self, entry: &Entry, cx: &Context) -> Request;
    /// The item from the model's reply, checked in code (never trusted): `Err` = one line,
    /// Traditional Chinese, why this reply cannot be used.
    fn read(&self, entry: &Entry, cx: &Context, reply: &Value) -> Result<Made, String>;
}

/// Words that end in a dot without ending the sentence (lower case, without the final dot).
const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "etc", "al", "fig", "figs", "eq", "eqs", "tab", "vs", "cf", "approx", "ref", "refs", "sec", "no", "resp",
    "viz", "ca", "est", "dr", "mr", "mrs", "ms", "prof",
];

/// The words of one hand-over cut into what becomes one entry each (a pattern takes one
/// sentence). A sentence ends at `.`, `?` or `!` (closing quotes and brackets may follow) when
/// whitespace and then an upper-case letter, a digit, `(` or `{` comes, or the text ends. Kept
/// whole: abbreviations (`e.g.`, `et al.`, `Fig.` ...), single capital initials (`J. Smith`),
/// decimals, ellipses (`...` never ends a sentence), and anything inside `(...)` or `$...$`.
/// Each piece is trimmed with inner whitespace collapsed; pieces under three words are dropped;
/// order is kept.
pub fn sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    let n = chars.len();
    let mut out = vec![];
    let (mut start, mut depth, mut math) = (0usize, 0i32, false);
    let mut i = 0;
    while i < n {
        let c = chars[i];
        match c {
            '$' => math = !math,
            '(' if !math => depth += 1,
            ')' if !math => depth = (depth - 1).max(0),
            '.' | '?' | '!' if !math && depth == 0 => {
                // the run of terminators, then closing quotes / brackets
                let mut j = i;
                while j + 1 < n && matches!(chars[j + 1], '.' | '?' | '!') {
                    j += 1;
                }
                let ellipsis = chars[i..=j].iter().filter(|&&x| x == '.').count() >= 2;
                let mut k = j;
                while k + 1 < n && matches!(chars[k + 1], '"' | '\'' | ']' | '\u{201d}' | '\u{2019}') {
                    k += 1;
                }
                let at_end = k + 1 >= n;
                let next_ok = at_end
                    || (chars[k + 1] == ' '
                        && chars.get(k + 2).is_none_or(|x| x.is_uppercase() || x.is_ascii_digit() || matches!(x, '(' | '{')));
                let abbreviated = c == '.' && j == i && is_abbreviation(&chars[start..i]);
                if next_ok && !ellipsis && !abbreviated {
                    push_sentence(&mut out, &chars[start..=k]);
                    start = k + 1;
                }
                i = k;
            }
            _ => {}
        }
        i += 1;
    }
    if start < n {
        push_sentence(&mut out, &chars[start..]);
    }
    out
}

/// Is the word before this dot an abbreviation or a single capital initial?
fn is_abbreviation(before: &[char]) -> bool {
    let word: String = before.iter().rev().take_while(|c| **c != ' ' && **c != '(').collect::<Vec<_>>().into_iter().rev().collect();
    let word = word.trim_start_matches(['"', '\'', '[', '{']);
    let mut it = word.chars();
    if let (Some(a), None) = (it.next(), it.next())
        && a.is_uppercase()
    {
        return true;
    }
    ABBREVIATIONS.contains(&word.to_lowercase().as_str())
}

fn push_sentence(out: &mut Vec<String>, piece: &[char]) {
    let s: String = piece.iter().collect();
    let s = s.trim();
    if s.split_whitespace().count() >= 3 {
        out.push(s.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::sentences;

    #[test]
    fn a_paragraph_is_cut_into_its_sentences() {
        let t = "We train the model for ten epochs. Does the loss fall?  Yes, it falls quickly!\nThen 3 runs are averaged. The last one has no end";
        assert_eq!(
            sentences(t),
            vec![
                "We train the model for ten epochs.",
                "Does the loss fall?",
                "Yes, it falls quickly!",
                "Then 3 runs are averaged.",
                "The last one has no end"
            ]
        );
        assert_eq!(sentences("  one sentence without any end  "), vec!["one sentence without any end"]);
        assert!(sentences("   ").is_empty());
    }

    #[test]
    fn abbreviations_decimals_and_parentheses_stay_whole() {
        let one = |t: &str| assert_eq!(sentences(t), vec![t.to_string()], "{t}");
        one("Several methods, e.g. dropout and weight decay, reduce overfitting.");
        one("This was shown by Smith et al. in a recent study.");
        one("The accuracy rises to 0.85 after training.");
        one("As Fig. 3 shows, the curve is flat.");
        one("This follows from Eq. (2) and Tab. 1 together.");
        one("Our model is faster vs. the baseline in most cases.");
        one("Wait... Then the loss rose again somehow.");
        one("The model was proposed by J. Smith and colleagues.");
        one("The gain (see Fig. 2. Then Tab. 3) is small anyway.");
        one("The bound $x. Y$ holds for every input here.");
        assert_eq!(
            sentences("It is about 5 times larger, approx. 2 GB. The next one is smaller."),
            vec!["It is about 5 times larger, approx. 2 GB.", "The next one is smaller."]
        );
        assert_eq!(sentences("See the loss (Fig. 1). Then it drops fast."), vec!["See the loss (Fig. 1).", "Then it drops fast."]);
    }

    #[test]
    fn short_fragments_are_dropped() {
        assert_eq!(sentences("Yes. We then train the model again. Done."), vec!["We then train the model again."]);
        assert!(sentences("Too short.").is_empty());
        assert!(sentences("Ok. Fine.").is_empty());
    }
}
