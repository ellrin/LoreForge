//! 寫法筆記: what the author says in the interview about how to write and what to avoid
//! ("open this paragraph with the clinical problem, not a number", "never use novel"). The
//! turn call that grades the answer lifts them out (no extra call), in the author's words,
//! with a scope: this paragraph (the group's paragraphs), this section, or the whole paper.
//! The drafts take them as hard constraints; banned words are checked by code. A note that
//! asks for punctuation the author's guard forbids is flagged: the guard wins. At the end the
//! author may keep chosen notes for every paper: they go into `style/` only on their click.

use super::session::{OutlinePara, Session};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file in the workspace's `style/` that kept notes go to (read by every prompt, like the
/// other style notes).
pub const KEEP_FILE: &str = "50-interview-notes.md";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WritingNote {
    /// w1, w2 … (the code's).
    #[serde(default)]
    pub id: String,
    /// The author's words.
    pub text: String,
    /// `paragraph` (the group's paragraphs), `section` or `paper`.
    #[serde(default)]
    pub scope: String,
    /// The group it was said in.
    #[serde(default)]
    pub group: String,
    /// The section of that group (scope `section`).
    #[serde(default)]
    pub section: String,
    /// Words never to use (checked by code).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub banned: Vec<String>,
    /// The author wants it for every paper (written into `style/` on their click).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
    /// Already written into `style/`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub kept: bool,
    /// It asks for punctuation the author's punctuation rule forbids (the rule wins).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
}

/// What one turn lifted out of the answer.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct NoteUpdate {
    pub text: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub banned: Vec<String>,
}

/// The turn schema's field.
pub fn schema() -> serde_json::Value {
    use crate::compose::{json_array, json_object, json_string};
    json_array(json_object(&[("text", json_string()), ("scope", json_string()), ("banned", json_array(json_string()))]))
}

/// The turn prompt's rule (short).
pub const TURN_RULE: &str = "10. `writing_notes`: only when the latest answer says how to WRITE something or what to avoid \
(an order, a word, a tone: \"open with the clinical problem, not a number\", \"never say novel\"), quote it in the \
author's words, `scope` = paragraph (what this group becomes), section, or paper; `banned` = words they forbid. Not \
facts about the study. Usually [].";

/// Words about punctuation the guard decides.
const PUNCT_WORDS: &[&str] = &["冒號", "分號", "破折號", "引號", "colon", "semicolon", "dash", "quotation"];

/// Add a turn's notes (scope checked, the same text not twice).
pub fn add(sess: &mut Session, group: &str, notes: Vec<NoteUpdate>, guard_on: bool) {
    let section = sess.group(group).map(|g| g.section.clone()).unwrap_or_default();
    for n in notes {
        let text = n.text.trim().to_string();
        if text.is_empty() || sess.writing_notes.iter().any(|x| x.text == text) {
            continue;
        }
        let scope = match n.scope.trim() {
            "paper" => "paper",
            "section" => "section",
            _ => "paragraph",
        };
        let k = sess.writing_notes.iter().filter_map(|x| x.id.strip_prefix('w')?.parse::<usize>().ok()).max().unwrap_or(0) + 1;
        let low = text.to_lowercase();
        let conflict = guard_on && PUNCT_WORDS.iter().any(|w| low.contains(w));
        sess.writing_notes.push(WritingNote {
            id: format!("w{k}"),
            text,
            scope: scope.into(),
            group: group.into(),
            section: section.clone(),
            banned: n.banned.into_iter().map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).collect(),
            conflict,
            ..Default::default()
        });
    }
}

/// The notes that bind a paragraph of `section`: the paper's, the section's, and the paragraph
/// notes of the groups its outline sources come from.
pub fn for_paragraph<'a>(sess: &'a Session, section: &str, para: &OutlinePara) -> Vec<&'a WritingNote> {
    let groups: Vec<&str> = para
        .sources
        .iter()
        .map(|s| s.as_str())
        .flat_map(|src| {
            // a source is a group id, an item id (its group) or a claim id (its items' groups)
            let mut g: Vec<&str> = vec![];
            if sess.group(src).is_some() {
                g.push(src);
            }
            if let Some(i) = sess.items.iter().find(|i| i.id == src) {
                g.push(i.group.as_str());
            }
            if let Some(c) = sess.claims.iter().find(|c| c.id == src) {
                g.extend(sess.items.iter().filter(|i| c.items.contains(&i.id)).map(|i| i.group.as_str()));
            }
            g
        })
        .collect();
    sess.writing_notes
        .iter()
        .filter(|n| match n.scope.as_str() {
            "paper" => true,
            // the abstract takes every claim: a note about one paragraph is not its constraint
            _ if section == "Abstract" => false,
            "section" => n.section == section,
            _ => groups.contains(&n.group.as_str()),
        })
        .collect()
}

/// The compose block: hard constraints, the author's own words.
pub fn block(notes: &[&WritingNote]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let mut s = "The author's own instructions for this paper (hard constraints; where they differ from the style notes, these win; \
a punctuation rule of the code wins over them):\n"
        .to_string();
    for n in notes {
        s += &format!("- {}\n", n.text);
    }
    s
}

/// Banned words a text uses (whole words, any case).
pub fn banned_in(notes: &[&WritingNote], text: &str) -> Vec<String> {
    let plain = format!(" {} ", crate::text::strip_latex(text).to_lowercase());
    let mut out: Vec<String> = vec![];
    for b in notes.iter().flat_map(|n| &n.banned) {
        let w = b.to_lowercase();
        let hit = plain.match_indices(&w).any(|(i, _)| {
            let before = plain[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric());
            let after = plain[i + w.len()..].chars().next().is_some_and(|c| c.is_alphanumeric());
            !before && !after
        });
        if hit && !out.contains(b) {
            out.push(b.clone());
        }
    }
    out
}

/// 存進我的寫法: the chosen notes into `style/KEEP_FILE` with where they came from (appended;
/// only the ones not written before). Returns the file and how many were added.
pub fn keep(workspace: &Path, paper_name: &str, date: &str, sess: &mut Session) -> Result<(PathBuf, usize), String> {
    let chosen: Vec<usize> = sess.writing_notes.iter().enumerate().filter(|(_, n)| n.keep && !n.kept).map(|(k, _)| k).collect();
    let file = workspace.join(super::STYLE_DIR).join(KEEP_FILE);
    if chosen.is_empty() {
        return Ok((file, 0));
    }
    std::fs::create_dir_all(file.parent().unwrap_or(workspace)).map_err(|e| e.to_string())?;
    let mut text = std::fs::read_to_string(&file).unwrap_or_else(|_| "# 訪談裡說的寫法\n\n討論草稿時作者親口說的寫法，存下來給以後每一篇用。\n".into());
    text += &format!("\n## {paper_name}（{date}）\n\n");
    for &k in &chosen {
        let n = &mut sess.writing_notes[k];
        text += &format!("- {}\n", n.text);
        if !n.banned.is_empty() {
            text += &format!("  - 不用：{}\n", n.banned.join("、"));
        }
        n.kept = true;
    }
    crate::paper::write_atomic(&file, &text).map_err(|e| e.to_string())?;
    Ok((file, chosen.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::session::{Claim, Group, Item};

    fn sess() -> Session {
        let mut s = Session::default();
        s.set_plan(vec![
            (Group { section: "Introduction".into(), title: "動機".into(), ..Default::default() }, vec![Item { label: "clinical".into(), ..Default::default() }]),
            (Group { section: "Results".into(), title: "主表".into(), ..Default::default() }, vec![Item { label: "auc".into(), ..Default::default() }]),
        ]);
        s
    }

    #[test]
    fn notes_bind_by_scope_and_banned_words_are_checked() {
        let mut s = sess();
        add(&mut s, "g1", vec![NoteUpdate { text: "這段先講臨床，不要一開始就丟數字".into(), scope: "paragraph".into(), banned: vec![] }], false);
        add(&mut s, "g1", vec![NoteUpdate { text: "整篇不要用 novel".into(), scope: "paper".into(), banned: vec!["novel".into(), " ".into()] }], false);
        add(&mut s, "g2", vec![NoteUpdate { text: "Results 每段先指表".into(), scope: "section".into(), banned: vec![] }, NoteUpdate { text: "整篇不要用 novel".into(), ..Default::default() }], true);
        assert_eq!(s.writing_notes.iter().map(|n| (n.id.as_str(), n.scope.as_str(), n.section.as_str())).collect::<Vec<_>>(), [("w1", "paragraph", "Introduction"), ("w2", "paper", "Introduction"), ("w3", "section", "Results")]);
        assert_eq!(s.writing_notes[1].banned, ["novel"]);
        // an Introduction paragraph from g1 (by its item), a Results paragraph from a claim of g2
        let intro = OutlinePara { id: "p1".into(), sources: vec!["i1".into()], ..Default::default() };
        let mine: Vec<&str> = for_paragraph(&s, "Introduction", &intro).iter().map(|n| n.id.as_str()).collect();
        assert_eq!(mine, ["w1", "w2"]);
        s.claims.push(Claim { id: "c1".into(), zh: "z".into(), items: vec!["i2".into()], ..Default::default() });
        let res = OutlinePara { id: "p9".into(), sources: vec!["c1".into()], ..Default::default() };
        assert_eq!(for_paragraph(&s, "Results", &res).iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["w2", "w3"]);
        let abs = OutlinePara { id: "abs".into(), sources: vec!["c1".into(), "i1".into()], ..Default::default() };
        assert_eq!(for_paragraph(&s, "Abstract", &abs).iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["w2"], "only the paper's notes bind the abstract");
        let b = block(&for_paragraph(&s, "Results", &res));
        assert!(b.contains("hard constraints") && b.contains("- 整篇不要用 novel") && b.contains("- Results 每段先指表"));
        let n = for_paragraph(&s, "Results", &res);
        assert_eq!(banned_in(&n, "A Novel method, not a novelty."), ["novel"]);
        assert!(banned_in(&n, "A new method and its novelty.").is_empty(), "whole words only");
        // a note about punctuation while the guard is on is flagged
        add(&mut s, "g2", vec![NoteUpdate { text: "這裡可以用冒號列出".into(), scope: "section".into(), banned: vec![] }], true);
        assert!(s.writing_notes.last().unwrap().conflict);
    }

    #[test]
    fn kept_notes_go_into_style_once() {
        let ws = std::env::temp_dir().join(format!("pc-notes-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        let mut s = sess();
        add(&mut s, "g1", vec![NoteUpdate { text: "先講臨床".into(), scope: "paper".into(), banned: vec!["novel".into()] }, NoteUpdate { text: "只這篇".into(), scope: "paper".into(), banned: vec![] }], false);
        assert_eq!(keep(&ws, "demo", "2026-11-14", &mut s).unwrap().1, 0, "nothing chosen, nothing written");
        assert!(!ws.join("style").join(KEEP_FILE).exists());
        s.writing_notes[0].keep = true;
        let (f, n) = keep(&ws, "demo", "2026-11-14", &mut s).unwrap();
        assert_eq!(n, 1);
        let t = std::fs::read_to_string(&f).unwrap();
        assert!(t.contains("## demo（2026-11-14）") && t.contains("- 先講臨床") && t.contains("不用：novel") && !t.contains("只這篇"), "{t}");
        assert_eq!(keep(&ws, "demo", "2026-11-14", &mut s).unwrap().1, 0, "not twice");
        // the kept notes reach every later prompt as style notes
        assert!(crate::draft::style_text(&ws).contains("先講臨床"));
        let _ = std::fs::remove_dir_all(&ws);
    }
}
