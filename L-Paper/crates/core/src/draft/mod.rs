//! 討論草稿: building a paper draft through a conversation.
//!
//! The engine is generic; what it cares about comes from the workspace:
//!
//! - `lens/<name>.toml` (+ a Markdown guide): what to ask, how answers are
//!   graded, when a question group is done ([`lens`]);
//! - `style/`: the author's writing style as free-form notes (paragraph
//!   types, proportions, habits to keep and to fix, exemplars), put into the
//!   planning, outline, drafting and review prompts ([`style_text`]).
//!
//! The steps: topic → experiment record → question groups → interview →
//! claim register → review → journal template ([`venue`] suggests some) →
//! outline → draft → reviewer pass; then 修訂
//! ([`revise`]) for what has to be added to an existing paper. State lives in
//! the paper folder ([`session`]); prompts are in [`prompts`]. Every step runs
//! on Claude when it is available and on the local model otherwise; what a
//! smaller model needs (shorter prompts, pieces, reply repair) is in
//! [`fallback`]. 待補 (results still to come) are in [`pending`]; the answer
//! options prepared before the interview in [`candidates`].

pub mod candidates;
pub mod discuss;
pub mod fallback;
pub mod figures;
pub mod lens;
pub mod notes;
pub mod material;
pub mod pending;
pub mod prompts;
pub mod revise;
pub mod session;
pub mod venue;

use std::path::Path;

pub const STYLE_DIR: &str = "style";
/// Style notes beyond this are cut (prompts stay a sensible size).
pub const STYLE_MAX_CHARS: usize = 24_000;

/// Every `.md` / `.toml` / `.txt` file in the workspace's `style/` folder,
/// sorted by name, each under its file name; cut at `STYLE_MAX_CHARS`.
/// Files starting with `_` are skipped (raw material such as per-paragraph
/// annotations, too long for a prompt), and so are `checks.toml` (the data of
/// the code checks, [`crate::stylecheck`]) and `patterns.toml` (guide sentence
/// structures, [`crate::patterns`]: only the entries that fit a paragraph, never all).
pub fn style_text(workspace: &Path) -> String {
    let mut files: Vec<_> = std::fs::read_dir(workspace.join(STYLE_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("md" | "toml" | "txt")))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.starts_with(['_', '.']) && !n.eq_ignore_ascii_case("README.md") && !n.eq_ignore_ascii_case(crate::stylecheck::FILE) && !n.eq_ignore_ascii_case(crate::patterns::FILE)))
        .collect();
    files.sort();
    let mut out = String::new();
    for f in files {
        let Ok(t) = std::fs::read_to_string(&f) else { continue };
        if t.trim().is_empty() {
            continue;
        }
        out += &format!("\n#### {}\n{}\n", f.file_name().and_then(|n| n.to_str()).unwrap_or(""), t.trim());
    }
    if out.chars().count() > STYLE_MAX_CHARS {
        out = out.chars().take(STYLE_MAX_CHARS).collect::<String>() + "\n[… style notes cut …]\n";
    }
    out
}

/// The style notes as a block for a prompt that writes or rewrites English prose: rules to
/// follow, not content. Empty notes → empty block (the prompt stays as it was).
pub fn style_block(style: &str) -> String {
    let style = style.trim();
    if style.is_empty() {
        return String::new();
    }
    format!(
        "\n\nThe author's style rules: follow them in every English sentence you write. They are rules, not content: \
never copy their examples and never mention them.\n{style}\n"
    )
}

/// `prompt` with [`style_block`] after its system part.
pub fn with_style(mut prompt: crate::compose::Prompt, style: &str) -> crate::compose::Prompt {
    prompt.system += &style_block(style);
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_folder_becomes_prompt_text() {
        let ws = std::env::temp_dir().join(format!("pc-style-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        assert_eq!(style_text(&ws), "", "no folder, no notes");
        std::fs::create_dir_all(ws.join("style")).unwrap();
        std::fs::write(ws.join("style/b-anti-patterns.md"), "Never end on a platitude.").unwrap();
        std::fs::write(ws.join("style/a-profile.toml"), "paragraph_words = [80, 150]").unwrap();
        std::fs::write(ws.join("style/README.md"), "how to fill this folder").unwrap();
        std::fs::write(ws.join("style/_paragraphs.jsonl.txt"), "raw").unwrap();
        std::fs::write(ws.join("style/figure.png"), "x").unwrap();
        let t = style_text(&ws);
        assert!(t.find("a-profile.toml").unwrap() < t.find("b-anti-patterns.md").unwrap());
        assert!(!t.contains("how to fill") && !t.contains("raw") && !t.contains("figure"));
        std::fs::write(ws.join("style/checks.toml"), "[[wordy]]\nfind = \"in order to\"").unwrap();
        assert!(!style_text(&ws).contains("in order to"), "the checks' data never goes into prompts");
        std::fs::write(ws.join("style/patterns.toml"), "[[pattern]]\nid = \"g\"\nsituation = \"compare_two\"\nstructure = \"Although A and B differ, both C.\"\nsource = \"guide\"").unwrap();
        assert!(!style_text(&ws).contains("Although A and B"), "guide patterns are never sent whole");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn style_block_is_rules_or_nothing() {
        use crate::compose::Prompt;
        let p = Prompt { system: "S".into(), user: "U".into() };
        assert_eq!(with_style(p.clone(), "  \n").system, "S", "no notes, prompt unchanged");
        let w = with_style(p, "No contractions.");
        assert!(w.system.starts_with("S") && w.system.contains("style rules: follow them") && w.system.ends_with("No contractions.\n"));
        assert_eq!(w.user, "U");
    }
}
