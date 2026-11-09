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
//! claim register → review → outline → draft → reviewer pass. State lives in
//! the paper folder ([`session`]); prompts are in [`prompts`].

pub mod lens;
pub mod material;
pub mod prompts;
pub mod session;

use std::path::Path;

pub const STYLE_DIR: &str = "style";
/// Style notes beyond this are cut (prompts stay a sensible size).
pub const STYLE_MAX_CHARS: usize = 24_000;

/// Every `.md` / `.toml` / `.txt` file in the workspace's `style/` folder,
/// sorted by name, each under its file name; cut at `STYLE_MAX_CHARS`.
/// Files starting with `_` are skipped (raw material such as per-paragraph
/// annotations, too long for a prompt).
pub fn style_text(workspace: &Path) -> String {
    let mut files: Vec<_> = std::fs::read_dir(workspace.join(STYLE_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("md" | "toml" | "txt")))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.starts_with(['_', '.']) && !n.eq_ignore_ascii_case("README.md")))
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
        let _ = std::fs::remove_dir_all(&ws);
    }
}
