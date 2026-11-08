//! Edges (design §4.3). Explicit typed links are stored in the node (`links[]`);
//! `[[title or alias]]` in the body become `mentions` edges (Obsidian style), resolved
//! by the index against titles and aliases in the same space; shared tags are drawn by
//! the graph on request. The index keeps every edge in both directions (backlinks).

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// A link type (`prerequisite`, …), `mentions`, or `tag`.
    pub rel: String,
    /// `link`, `mention` or `tag`.
    pub origin: String,
}

/// Targets of `[[target]]` / `[[target|shown text]]` in a body, in order, without
/// duplicates. Code spans and fenced code blocks are skipped.
pub fn wikilinks(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut in_code = false;
        let mut rest = line;
        while !rest.is_empty() {
            if rest.starts_with('`') {
                in_code = !in_code;
                rest = &rest[1..];
                continue;
            }
            if !in_code && rest.starts_with("[[") && let Some(end) = rest[2..].find("]]") {
                let inner = &rest[2..2 + end];
                let target = inner.split('|').next().unwrap_or("").split('#').next().unwrap_or("").trim();
                if !target.is_empty() && !target.contains('[') && !out.iter().any(|t| t == target) {
                    out.push(target.to_string());
                }
                rest = &rest[2 + end + 2..];
                continue;
            }
            let n = rest.chars().next().map(char::len_utf8).unwrap_or(1);
            rest = &rest[n..];
        }
    }
    out
}

/// Case-folded key used to resolve wikilinks and names.
pub fn name_key(s: &str) -> String {
    s.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wikilinks() {
        let body = "Dice 與 [[IoU]] 相關，見 [[Jaccard index|Jaccard]] 與 [[IoU]]。\n`[[not this]]`\n```\n[[nor this]]\n```\n[[膝關節#解剖]] [[ ]]";
        assert_eq!(wikilinks(body), vec!["IoU", "Jaccard index", "膝關節"]);
        assert_eq!(name_key(" U-Net "), "u-net");
    }
}
