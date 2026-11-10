//! A journal's house rules for cross-references: the word written before
//! `\ref` for each kind of label ("Fig." or "Figure"), the word at the start
//! of a sentence, or cleveref (`\cref` / `\Cref`, the class prints the word).
//!
//! `RefWords::default()` is what the apps wrote before journals could say:
//! `Fig.~\ref`, `Table~\ref`, `Section~\ref`, and a bare `\eqref` for
//! equations. Whether a sentence starts with the full word is the journal's
//! rule: only `sentence_start` decides, nothing is assumed.
//!
//! In `template.toml`:
//! ```toml
//! [ref_words]
//! fig = "Figure"
//! eq = "Eq."            # "" writes only \eqref{…}
//! sentence_start = { fig = "Figure" }
//! mode = "cleveref"     # or "words" (the default)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Built-in words, used for every kind the rules leave out.
pub const DEFAULT_WORDS: &[(&str, &str)] = &[("fig", "Fig."), ("tab", "Table"), ("eq", ""), ("sec", "Section"), ("alg", "Algorithm"), ("thm", "Theorem")];

/// The label kinds whose words a journal sets (`alg`, `thm` keep the defaults).
pub const KINDS: &[&str] = &["fig", "tab", "eq", "sec"];

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
pub struct RefWords {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fig: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    /// `Some("")`: only `\eqref{…}`, which prints "(1)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sec: Option<String>,
    /// Kind → word at the start of a sentence; a kind left out uses its
    /// mid-sentence word.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sentence_start: BTreeMap<String, String>,
    /// "words" (default) or "cleveref". Kept as text so a template with an
    /// unknown value still loads; anything but "cleveref" means words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

impl RefWords {
    pub fn is_empty(&self) -> bool {
        *self == RefWords::default()
    }

    /// True when only the mode is set (nothing else to follow).
    pub fn words_unset(&self) -> bool {
        self.fig.is_none() && self.tab.is_none() && self.eq.is_none() && self.sec.is_none() && self.sentence_start.is_empty()
    }

    pub fn cleveref(&self) -> bool {
        self.mode.as_deref().is_some_and(|m| m.trim().eq_ignore_ascii_case("cleveref"))
    }

    /// The configured mid-sentence word, if the rules set one.
    pub fn configured(&self, kind: &str) -> Option<&str> {
        match kind {
            "fig" => self.fig.as_deref(),
            "tab" => self.tab.as_deref(),
            "eq" => self.eq.as_deref(),
            "sec" => self.sec.as_deref(),
            _ => None,
        }
        .map(str::trim)
    }

    /// The word written before the reference ("" for none). Unknown kinds
    /// get no word.
    pub fn word(&self, kind: &str, sentence_start: bool) -> String {
        if sentence_start {
            if let Some(w) = self.sentence_start.get(kind) {
                return w.trim().to_string();
            }
        }
        match self.configured(kind) {
            Some(w) => w.to_string(),
            None => DEFAULT_WORDS.iter().find(|(k, _)| *k == kind).map(|(_, w)| w.to_string()).unwrap_or_default(),
        }
    }

    /// What to write for a reference to `key`: `Fig.~\ref{fig:a}`,
    /// `\eqref{eq:a}`, or `\cref{…}` / `\Cref{…}` in cleveref mode.
    pub fn snippet(&self, kind: &str, key: &str, sentence_start: bool) -> String {
        if self.cleveref() {
            return format!("\\{}{{{key}}}", if sentence_start { "Cref" } else { "cref" });
        }
        let cmd = if kind == "eq" { "eqref" } else { "ref" };
        match self.word(kind, sentence_start) {
            w if w.is_empty() => format!("\\{cmd}{{{key}}}"),
            w => format!("{w}~\\{cmd}{{{key}}}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_what_the_apps_always_wrote() {
        let d = RefWords::default();
        assert_eq!(d.snippet("fig", "fig:a", false), "Fig.~\\ref{fig:a}");
        assert_eq!(d.snippet("fig", "fig:a", true), "Fig.~\\ref{fig:a}", "no sentence rule: same word");
        assert_eq!(d.snippet("tab", "tab:a", false), "Table~\\ref{tab:a}");
        assert_eq!(d.snippet("eq", "eq:a", false), "\\eqref{eq:a}");
        assert_eq!(d.snippet("sec", "sec:a", false), "Section~\\ref{sec:a}");
        assert_eq!(d.snippet("alg", "alg:a", false), "Algorithm~\\ref{alg:a}");
        assert_eq!(d.snippet("other", "x", false), "\\ref{x}");
        assert!(d.is_empty() && !d.cleveref());
    }

    #[test]
    fn a_journal_sets_words_and_sentence_starts() {
        let r: RefWords = toml_like(r#"{"fig":"Fig.","eq":"Eq.","sentence_start":{"fig":"Figure"}}"#);
        assert_eq!(r.snippet("fig", "fig:a", false), "Fig.~\\ref{fig:a}");
        assert_eq!(r.snippet("fig", "fig:a", true), "Figure~\\ref{fig:a}");
        assert_eq!(r.snippet("eq", "eq:a", true), "Eq.~\\eqref{eq:a}");
        let c: RefWords = toml_like(r#"{"mode":"cleveref","fig":"Figure"}"#);
        assert_eq!(c.snippet("fig", "fig:a", false), "\\cref{fig:a}");
        assert_eq!(c.snippet("tab", "tab:a", true), "\\Cref{tab:a}");
        let none: RefWords = toml_like(r#"{"eq":""}"#);
        assert_eq!(none.snippet("eq", "eq:a", false), "\\eqref{eq:a}");
    }

    fn toml_like(json: &str) -> RefWords {
        serde_json::from_str(json).unwrap()
    }
}
