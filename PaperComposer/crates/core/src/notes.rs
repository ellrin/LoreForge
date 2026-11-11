//! 筆記: the workspace's `notes/` folder (LaTeX notes and 靈感 from LoreEcho or written here).
//! The file format, names, trash and the lore-notation check live in `commons/crates/lore-notes`
//! so LoreEcho writes the same files when PaperComposer is closed.

pub use lore_notes::*;

use std::path::Path;

/// The notes of a workspace: `<workspace>/notes/`.
pub fn of(workspace: &Path) -> Notes {
    Notes::in_workspace(workspace)
}

/// Lower-cased words of a search: a note matches when every word is in its
/// title, tags or body.
pub fn matches(n: &Note, q: &str) -> bool {
    let hay = format!("{}\n{}\n{}", n.title, n.tags.join(" "), n.body).to_lowercase();
    q.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_words_all_match() {
        let n = Note { title: "Loss weights".into(), tags: vec!["training".into()], body: "By \\eqref{eq:loss}.".into(), ..Default::default() };
        assert!(matches(&n, "loss TRAIN"));
        assert!(matches(&n, ""));
        assert!(!matches(&n, "loss deck"));
    }
}
