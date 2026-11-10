//! Review: tracked changes and comments, kept beside the paper in
//! `review.json` and never written into the .tex files.
//!
//! Changes are recorded at paragraph granularity when a section file is
//! saved: the old and new paragraph lists are aligned (LCS over
//! whitespace-normalised paragraphs, then the unmatched ones in each gap are
//! paired as edits by similarity), and every inserted, deleted or edited
//! paragraph becomes `{before, after}` (`None` for the missing side). A
//! second edit by the same author to a paragraph that already carries an
//! open change updates that change's `after` and keeps the original
//! `before`, as Word does. Every change keeps an `index` hint: the
//! paragraph's place in its section (for a deletion, where it would go
//! back), kept up to date on every save, tracked or not.
//!
//! Accepting drops the record (the file already holds `after`); rejecting
//! puts `before` back, refused when the paragraph no longer reads `after`.
//!
//! Comments anchor on LaTeX source text (`exact` with some `prefix` and
//! `suffix`) in a paragraph, and are re-anchored by that text after every
//! save; when the text is gone they stay, marked `orphan`.
//!
//! Unknown fields at every level are kept, so later importers (Word: an
//! imported author and time, `source: "word"`) can add to the format.

use crate::paper;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

pub const SCHEMA: &str = "papercomposer.review/1";
pub const FILE: &str = "review.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Review {
    #[serde(default = "schema")]
    pub schema: String,
    /// 追蹤修訂: record edits while on.
    #[serde(default)]
    pub tracking: bool,
    #[serde(default)]
    pub changes: Vec<Change>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn schema() -> String {
    SCHEMA.to_string()
}

impl Default for Review {
    fn default() -> Self {
        Review { schema: schema(), tracking: false, changes: vec![], comments: vec![], extra: Map::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub id: String,
    pub author: String,
    /// RFC 3339, UTC.
    pub at: String,
    /// Section file under `sections/`.
    pub file: String,
    /// The paragraph's place in its section; for a deletion, where it goes back.
    #[serde(default)]
    pub index: usize,
    /// `None`: the paragraph was inserted.
    pub before: Option<String>,
    /// `None`: the paragraph was deleted.
    pub after: Option<String>,
    /// Where the change came from when not typed here (e.g. "word").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub at: String,
    pub file: String,
    /// Paragraph index (a hint: the anchor text decides).
    #[serde(default)]
    pub index: usize,
    /// The commented source text; empty = the whole paragraph.
    #[serde(default)]
    pub exact: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    pub text: String,
    #[serde(default)]
    pub replies: Vec<Reply>,
    #[serde(default)]
    pub resolved: bool,
    /// The anchor text is gone (原文已更動).
    #[serde(default)]
    pub orphan: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reply {
    pub id: String,
    pub author: String,
    pub at: String,
    pub text: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ── Files ───────────────────────────────────────────────

/// `review.json` of a paper (an empty review when missing or unreadable).
pub fn load(paper_dir: &Path) -> Review {
    std::fs::read_to_string(paper::app_dir(paper_dir).join(FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Write `review.json` atomically. An empty review with tracking off and no
/// file yet writes nothing, so papers that never use review stay as they are.
pub fn save(paper_dir: &Path, r: &Review) -> Result<(), String> {
    let path = paper::app_dir(paper_dir).join(FILE);
    if !path.exists() && !r.tracking && r.changes.is_empty() && r.comments.is_empty() && r.extra.is_empty() {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(r).map_err(|e| e.to_string())?;
    paper::write_atomic(&path, &(text + "\n")).map_err(|e| e.to_string())
}

// ── Time and ids ────────────────────────────────────────

/// Now as RFC 3339 UTC, e.g. `2026-11-10T10:21:00Z`.
pub fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    rfc3339(secs)
}

pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// A short id, unique within this process and across runs.
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{prefix}{ms:x}{:x}", N.fetch_add(1, Ordering::Relaxed))
}

// ── Alignment ───────────────────────────────────────────

/// Same words whatever the line breaks (saving puts one sentence per line).
pub fn norm(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whitespace runs as one space (`~` too), ends kept: for anchor context.
fn squash(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut space = false;
    for ch in t.chars() {
        if ch.is_whitespace() || ch == '~' {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(ch);
            space = false;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// old[i] == new[j]
    Same(usize, usize),
    /// old[i] became new[j]
    Edit(usize, usize),
    /// old[i] removed; `j` = how many new paragraphs come before its place
    Del(usize, usize),
    /// new[j] added
    Ins(usize),
}

/// How alike two paragraphs are: shared words over all words (0..1).
fn similarity(a: &str, b: &str) -> f64 {
    let wa: Vec<&str> = a.split_whitespace().collect();
    let wb: Vec<&str> = b.split_whitespace().collect();
    if wa.is_empty() && wb.is_empty() {
        return 1.0;
    }
    let mut left = wb.clone();
    let mut common = 0usize;
    for w in &wa {
        if let Some(k) = left.iter().position(|x| x == w) {
            left.swap_remove(k);
            common += 1;
        }
    }
    2.0 * common as f64 / (wa.len() + wb.len()) as f64
}

const PAIR_MIN: f64 = 0.3;

/// Pair the deleted and inserted paragraphs of one gap as edits, keeping
/// their order: one each always pairs; otherwise the pairing with the most
/// similarity, counting only pairs at least `PAIR_MIN` alike.
fn pair_gap(old: &[String], new: &[String], dels: &[usize], ins: &[usize]) -> Vec<(usize, usize)> {
    if dels.is_empty() || ins.is_empty() {
        return vec![];
    }
    if dels.len() == 1 && ins.len() == 1 {
        return vec![(dels[0], ins[0])];
    }
    let (m, n) = (dels.len(), ins.len());
    let sim: Vec<Vec<f64>> = dels.iter().map(|&i| ins.iter().map(|&j| similarity(&norm(&old[i]), &norm(&new[j]))).collect()).collect();
    let mut best = vec![vec![0.0f64; n + 1]; m + 1];
    for a in (0..m).rev() {
        for b in (0..n).rev() {
            let mut v = best[a + 1][b].max(best[a][b + 1]);
            if sim[a][b] >= PAIR_MIN {
                v = v.max(sim[a][b] + best[a + 1][b + 1]);
            }
            best[a][b] = v;
        }
    }
    let (mut a, mut b, mut out) = (0, 0, vec![]);
    while a < m && b < n {
        if sim[a][b] >= PAIR_MIN && (best[a][b] - (sim[a][b] + best[a + 1][b + 1])).abs() < 1e-9 {
            out.push((dels[a], ins[b]));
            a += 1;
            b += 1;
        } else if (best[a][b] - best[a + 1][b]).abs() < 1e-9 {
            a += 1;
        } else {
            b += 1;
        }
    }
    out
}

/// Align two paragraph lists: unchanged paragraphs by LCS, the rest of each
/// gap as edits, deletions and insertions, in document order.
pub fn align(old: &[String], new: &[String]) -> Vec<Op> {
    let a: Vec<String> = old.iter().map(|t| norm(t)).collect();
    let b: Vec<String> = new.iter().map(|t| norm(t)).collect();
    let (m, n) = (a.len(), b.len());
    let mut l = vec![vec![0u32; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            l[i][j] = if a[i] == b[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let mut ops = vec![];
    let (mut i, mut j) = (0, 0);
    let (mut dels, mut ins) = (vec![], vec![]);
    let flush = |ops: &mut Vec<Op>, dels: &mut Vec<usize>, ins: &mut Vec<usize>| {
        let pairs = pair_gap(old, new, dels, ins);
        let (mut di, mut ii) = (0, 0);
        for &(d, e) in &pairs {
            while dels[di] != d {
                ops.push(Op::Del(dels[di], 0));
                di += 1;
            }
            while ins[ii] != e {
                ops.push(Op::Ins(ins[ii]));
                ii += 1;
            }
            ops.push(Op::Edit(d, e));
            di += 1;
            ii += 1;
        }
        while di < dels.len() {
            ops.push(Op::Del(dels[di], 0));
            di += 1;
        }
        while ii < ins.len() {
            ops.push(Op::Ins(ins[ii]));
            ii += 1;
        }
        dels.clear();
        ins.clear();
    };
    while i < m || j < n {
        if i < m && j < n && a[i] == b[j] {
            flush(&mut ops, &mut dels, &mut ins);
            ops.push(Op::Same(i, j));
            i += 1;
            j += 1;
        } else if j < n && (i == m || l[i][j + 1] >= l[i + 1][j]) {
            ins.push(j);
            j += 1;
        } else {
            dels.push(i);
            i += 1;
        }
    }
    flush(&mut ops, &mut dels, &mut ins);
    // a deletion's place: the number of new paragraphs before it
    let mut seen = 0;
    for op in ops.iter_mut() {
        match op {
            Op::Same(..) | Op::Edit(..) | Op::Ins(_) => seen += 1,
            Op::Del(_, at) => *at = seen,
        }
    }
    ops
}

/// Where each old paragraph and each gap went: `para[i]` = new index of a
/// kept or edited old paragraph; `gap(k)` = new place of old position k.
struct Map2 {
    para: Vec<Option<usize>>,
    gap: Vec<usize>,
}

fn mapping(ops: &[Op], old_len: usize, new_len: usize) -> Map2 {
    let mut para = vec![None; old_len];
    let mut gap = vec![new_len; old_len + 1];
    for op in ops {
        match *op {
            Op::Same(i, j) | Op::Edit(i, j) => {
                para[i] = Some(j);
                gap[i] = j;
            }
            Op::Del(i, at) => gap[i] = at,
            Op::Ins(_) => {}
        }
    }
    Map2 { para, gap }
}

// ── Locating ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "index", rename_all = "lowercase")]
pub enum Place {
    /// The paragraph holding `after`.
    Para(usize),
    /// Where a deleted paragraph would go back.
    Gap(usize),
    /// Under a later change by someone else to the same paragraph (that
    /// change's `before` is this one's `after`): handled after it.
    Under(usize),
    /// `after` is no longer in the section (edited since without tracking).
    Lost,
}

impl Place {
    /// The paragraph it is on, if any.
    pub fn para(self) -> Option<usize> {
        match self {
            Place::Para(i) | Place::Under(i) => Some(i),
            _ => None,
        }
    }
}

/// Where a change is in the section's paragraphs now; `all` = every change
/// (a change another one was made on top of is `Under` it).
pub fn locate(paras: &[String], all: &[Change], c: &Change) -> Place {
    locate_depth(paras, all, c, all.len())
}

fn locate_depth(paras: &[String], all: &[Change], c: &Change, depth: usize) -> Place {
    let Some(after) = &c.after else { return Place::Gap(c.index.min(paras.len())) };
    let want = norm(after);
    if paras.get(c.index).is_some_and(|p| norm(p) == want) {
        return Place::Para(c.index);
    }
    // nearest paragraph with that text
    let mut hits: Vec<usize> = paras.iter().enumerate().filter(|(_, p)| norm(p) == want).map(|(i, _)| i).collect();
    hits.sort_by_key(|&i| i.abs_diff(c.index));
    if let Some(&i) = hits.first() {
        return Place::Para(i);
    }
    if depth > 0 {
        for d in all.iter().filter(|d| d.id != c.id && d.file == c.file && d.before.as_deref().map(norm).as_deref() == Some(want.as_str())) {
            if let Some(i) = locate_depth(paras, all, d, depth - 1).para() {
                return Place::Under(i);
            }
        }
    }
    Place::Lost
}

// ── Recording ───────────────────────────────────────────

/// A section file was saved: keep every change and comment of `file` in
/// its place, and, while tracking, record what changed as `author`.
pub fn record(r: &mut Review, file: &str, old: &[String], new: &[String], author: &str, at: &str) {
    let ops = align(old, new);
    let map = mapping(&ops, old.len(), new.len());
    let tracking = r.tracking;
    // the open change on each old paragraph (by where it is now)
    let mut on_para: Vec<Option<usize>> = vec![None; old.len()];
    for (k, c) in r.changes.iter().enumerate() {
        if c.file != file {
            continue;
        }
        if let Place::Para(i) = locate(old, &r.changes, c) {
            // the latest one wins when two authors changed the same paragraph
            on_para[i] = Some(k);
        }
    }
    let mut drop = vec![false; r.changes.len()];
    let mut handled = vec![false; old.len()];
    let mut added = vec![];
    for op in &ops {
        match *op {
            Op::Same(i, j) => {
                if let Some(k) = on_para[i] {
                    r.changes[k].index = j;
                }
            }
            Op::Edit(i, j) => {
                let mine = on_para[i].filter(|&k| !tracking || r.changes[k].author == author);
                if let Some(k) = mine {
                    // a later edit of the same change: keep `before`
                    let c = &mut r.changes[k];
                    c.after = Some(new[j].clone());
                    c.index = j;
                    if tracking {
                        c.at = at.to_string();
                    }
                    if c.before.as_deref().map(norm) == Some(norm(&new[j])) {
                        drop[k] = true;
                    }
                    handled[i] = true;
                } else if let Some(k) = on_para[i] {
                    r.changes[k].index = j;
                }
                if tracking && !handled[i] {
                    added.push(change(author, at, file, j, Some(old[i].clone()), Some(new[j].clone())));
                }
            }
            Op::Del(i, place) => {
                let mine = on_para[i].filter(|&k| !tracking || r.changes[k].author == author);
                if let Some(k) = mine {
                    let c = &mut r.changes[k];
                    if c.before.is_none() || !tracking {
                        // inserted and deleted again, or deleted untracked: nothing left to review
                        drop[k] = true;
                    } else {
                        c.after = None;
                        c.index = place;
                        c.at = at.to_string();
                    }
                    handled[i] = true;
                } else if let Some(k) = on_para[i] {
                    r.changes[k].index = place;
                }
                if tracking && !handled[i] {
                    added.push(change(author, at, file, place, Some(old[i].clone()), None));
                }
            }
            Op::Ins(j) => {
                if tracking {
                    added.push(change(author, at, file, j, None, Some(new[j].clone())));
                }
            }
        }
    }
    // deletions and lost changes: their place moves with the text around it
    for (k, c) in r.changes.iter_mut().enumerate() {
        if c.file != file || on_para.contains(&Some(k)) {
            continue;
        }
        c.index = map.gap[c.index.min(old.len())];
    }
    let mut k = 0;
    r.changes.retain(|_| {
        k += 1;
        !drop[k - 1]
    });
    r.changes.extend(added);
    // comments follow their paragraph, then their text
    for c in r.comments.iter_mut().filter(|c| c.file == file) {
        c.index = match map.para.get(c.index).copied().flatten() {
            Some(j) => j,
            None => map.gap[c.index.min(old.len())],
        };
    }
    reanchor(r, file, new);
}

fn change(author: &str, at: &str, file: &str, index: usize, before: Option<String>, after: Option<String>) -> Change {
    Change { id: new_id("c"), author: author.into(), at: at.into(), file: file.into(), index, before, after, source: None, extra: Map::new() }
}

/// Where `c`'s anchor is in a paragraph's source: the occurrence of
/// `exact` whose surroundings best match `prefix` / `suffix`.
fn find_anchor(text: &str, c: &Comment) -> Option<(usize, usize)> {
    let t = squash(text);
    let e = squash(&c.exact).trim().to_string();
    if e.is_empty() {
        return None;
    }
    let (pre, suf) = (squash(&c.prefix), squash(&c.suffix));
    let mut best: Option<(usize, usize)> = None;
    for (at, _) in t.match_indices(&e) {
        let before = &t[..at];
        let after = &t[at + e.len()..];
        let s1 = before.chars().rev().zip(pre.chars().rev()).take_while(|(x, y)| x == y).count();
        let s2 = after.chars().zip(suf.chars()).take_while(|(x, y)| x == y).count();
        if best.is_none_or(|(_, s)| s1 + s2 > s) {
            best = Some((at, s1 + s2));
        }
    }
    best
}

/// Find every comment of `file` again by its text: the paragraph at its
/// index first, else the paragraph where it fits best; else it is orphaned.
pub fn reanchor(r: &mut Review, file: &str, paras: &[String]) {
    for c in r.comments.iter_mut().filter(|c| c.file == file) {
        if norm(&c.exact).is_empty() {
            // the whole paragraph: kept while it exists
            c.orphan = c.index >= paras.len();
            continue;
        }
        if paras.get(c.index).is_some_and(|p| find_anchor(p, c).is_some()) {
            c.orphan = false;
            continue;
        }
        let mut best: Option<(usize, usize)> = None;
        for (i, p) in paras.iter().enumerate() {
            if let Some((_, score)) = find_anchor(p, c) {
                let better = match best {
                    None => true,
                    Some((bi, bs)) => score > bs || (score == bs && i.abs_diff(c.index) < bi.abs_diff(c.index)),
                };
                if better {
                    best = Some((i, score));
                }
            }
        }
        match best {
            Some((i, _)) => {
                c.index = i;
                c.orphan = false;
            }
            None => c.orphan = true,
        }
    }
}

// ── Accept / reject ─────────────────────────────────────

/// The section's paragraphs with change `c` undone, or why it cannot be.
pub fn rejected(paras: &[String], all: &[Change], c: &Change) -> Result<Vec<String>, String> {
    let mut out = paras.to_vec();
    match (locate(paras, all, c), &c.before) {
        (Place::Lost, _) => Err("段落之後又改過，無法拒絕；可以接受或手動修改".into()),
        (Place::Under(_), _) => Err("這段之後還有別人的修改，請先處理那一項".into()),
        (Place::Para(i), Some(b)) => {
            out[i] = b.clone();
            Ok(out)
        }
        (Place::Para(i), None) => {
            out.remove(i);
            Ok(out)
        }
        (Place::Gap(i), Some(b)) => {
            out.insert(i, b.clone());
            Ok(out)
        }
        (Place::Gap(_), None) => Err("這項修改是空的".into()),
    }
}

/// Reject change `id`: returns the section's new paragraphs; the review
/// loses the change and keeps everything else in place. The caller writes
/// the section file.
pub fn reject(r: &mut Review, id: &str, paras: &[String]) -> Result<Vec<String>, String> {
    let k = r.changes.iter().position(|c| c.id == id).ok_or("沒有這項修改")?;
    let c = r.changes[k].clone();
    let new = rejected(paras, &r.changes, &c)?;
    r.changes.remove(k);
    // move the rest without recording the reversion
    let tracking = std::mem::replace(&mut r.tracking, false);
    record(r, &c.file, paras, &new, "", "");
    r.tracking = tracking;
    Ok(new)
}

/// Accept change `id`: the record goes, the text stays.
pub fn accept(r: &mut Review, id: &str) -> Result<Change, String> {
    let k = r.changes.iter().position(|c| c.id == id).ok_or("沒有這項修改")?;
    Ok(r.changes.remove(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    fn tracked() -> Review {
        Review { tracking: true, ..Review::default() }
    }

    #[test]
    fn rfc3339_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_794_220_860), "2026-11-09T10:41:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn align_finds_inserts_deletes_and_edits() {
        let old = v(&["Alpha one.", "Beta two words here.", "Gamma three."]);
        // edit Beta, delete Gamma, insert Delta at the front
        let new = v(&["Delta new.", "Alpha one.", "Beta two words there."]);
        let ops = align(&old, &new);
        assert_eq!(ops, vec![Op::Ins(0), Op::Same(0, 1), Op::Edit(1, 2), Op::Del(2, 3)]);
    }

    #[test]
    fn align_ignores_line_breaks() {
        let old = v(&["One. Two."]);
        let new = v(&["One.\nTwo."]);
        assert_eq!(align(&old, &new), vec![Op::Same(0, 0)]);
    }

    #[test]
    fn align_pairs_by_similarity_in_a_gap() {
        // a new paragraph typed before an edited one: the edit pairs with its own paragraph
        let old = v(&["Intro.", "The model uses a retriever to find passages."]);
        let new = v(&["Intro.", "Something entirely unrelated appears.", "The model uses a dense retriever to find passages."]);
        assert_eq!(align(&old, &new), vec![Op::Same(0, 0), Op::Ins(1), Op::Edit(1, 2)]);
    }

    #[test]
    fn a_move_is_a_delete_and_an_insert() {
        let old = v(&["A a a.", "B b b.", "C c c."]);
        let new = v(&["B b b.", "C c c.", "A a a."]);
        let ops = align(&old, &new);
        assert!(ops.contains(&Op::Del(0, 0)));
        assert!(ops.contains(&Op::Ins(2)));
    }

    #[test]
    fn tracking_off_records_nothing() {
        let mut r = Review::default();
        record(&mut r, "intro.tex", &v(&["A.", "B."]), &v(&["A!", "C.", "B."]), "ellrin", "t1");
        assert!(r.changes.is_empty());
    }

    #[test]
    fn tracking_records_each_kind() {
        let mut r = tracked();
        let old = v(&["Alpha one.", "Beta two words here.", "Gamma three."]);
        let new = v(&["Delta new.", "Alpha one.", "Beta two words there."]);
        record(&mut r, "intro.tex", &old, &new, "ellrin", "t1");
        assert_eq!(r.changes.len(), 3);
        let ins = r.changes.iter().find(|c| c.before.is_none()).unwrap();
        assert_eq!((ins.index, ins.after.as_deref()), (0, Some("Delta new.")));
        let edit = r.changes.iter().find(|c| c.before.is_some() && c.after.is_some()).unwrap();
        assert_eq!((edit.index, edit.before.as_deref(), edit.after.as_deref()), (2, Some("Beta two words here."), Some("Beta two words there.")));
        let del = r.changes.iter().find(|c| c.after.is_none()).unwrap();
        assert_eq!((del.index, del.before.as_deref()), (3, Some("Gamma three.")));
        assert!(r.changes.iter().all(|c| c.author == "ellrin" && c.file == "intro.tex"));
    }

    #[test]
    fn same_author_edits_merge_and_keep_before() {
        let mut r = tracked();
        record(&mut r, "a.tex", &v(&["One two three."]), &v(&["One two four."]), "ellrin", "t1");
        record(&mut r, "a.tex", &v(&["One two four."]), &v(&["One two five."]), "ellrin", "t2");
        assert_eq!(r.changes.len(), 1);
        let c = &r.changes[0];
        assert_eq!((c.before.as_deref(), c.after.as_deref(), c.at.as_str()), (Some("One two three."), Some("One two five."), "t2"));
        // and back to where it started: nothing to review
        record(&mut r, "a.tex", &v(&["One two five."]), &v(&["One two three."]), "ellrin", "t3");
        assert!(r.changes.is_empty());
    }

    #[test]
    fn another_author_gets_a_change_of_their_own() {
        let mut r = tracked();
        record(&mut r, "a.tex", &v(&["One two three."]), &v(&["One two four."]), "ellrin", "t1");
        record(&mut r, "a.tex", &v(&["One two four."]), &v(&["One two five."]), "daidai", "t2");
        assert_eq!(r.changes.len(), 2);
        assert_eq!(r.changes[1].before.as_deref(), Some("One two four."));
        assert_eq!(r.changes[1].author, "daidai");
    }

    #[test]
    fn a_change_on_top_of_another_authors_change() {
        let mut r = tracked();
        record(&mut r, "a.tex", &v(&["One two three."]), &v(&["One two four."]), "daidai", "t1");
        record(&mut r, "a.tex", &v(&["One two four."]), &v(&["One two five."]), "ellrin", "t2");
        let now = v(&["One two five."]);
        let (below, top) = (r.changes[0].clone(), r.changes[1].clone());
        assert_eq!(locate(&now, &r.changes, &below), Place::Under(0));
        assert_eq!(locate(&now, &r.changes, &top), Place::Para(0));
        // the one below waits for the one on top
        assert!(reject(&mut r, &below.id, &now).unwrap_err().contains("先處理"));
        let back = reject(&mut r, &top.id, &now).unwrap();
        assert_eq!(back, v(&["One two four."]));
        assert_eq!(locate(&back, &r.changes, &r.changes[0]), Place::Para(0));
        assert_eq!(reject(&mut r, &below.id, &back).unwrap(), v(&["One two three."]));
    }

    #[test]
    fn inserted_then_deleted_leaves_nothing() {
        let mut r = tracked();
        record(&mut r, "a.tex", &v(&["A a."]), &v(&["A a.", "New one."]), "ellrin", "t1");
        record(&mut r, "a.tex", &v(&["A a.", "New one."]), &v(&["A a."]), "ellrin", "t2");
        assert!(r.changes.is_empty());
    }

    #[test]
    fn edited_then_deleted_becomes_a_deletion_of_the_original() {
        let mut r = tracked();
        record(&mut r, "a.tex", &v(&["A a.", "B b b."]), &v(&["A a.", "B b c."]), "ellrin", "t1");
        record(&mut r, "a.tex", &v(&["A a.", "B b c."]), &v(&["A a."]), "ellrin", "t2");
        assert_eq!(r.changes.len(), 1);
        assert_eq!((r.changes[0].before.as_deref(), r.changes[0].after.as_deref(), r.changes[0].index), (Some("B b b."), None, 1));
    }

    #[test]
    fn untracked_edits_keep_changes_in_place() {
        let mut r = tracked();
        record(&mut r, "a.tex", &v(&["A a.", "B b b."]), &v(&["A a.", "B b c."]), "ellrin", "t1");
        r.tracking = false;
        // a paragraph goes in before it: the change moves down
        record(&mut r, "a.tex", &v(&["A a.", "B b c."]), &v(&["Z.", "A a.", "B b c."]), "ellrin", "t2");
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].index, 2);
        assert_eq!(locate(&v(&["Z.", "A a.", "B b c."]), &r.changes, &r.changes[0]), Place::Para(2));
    }

    #[test]
    fn accept_drops_the_record() {
        let mut r = tracked();
        record(&mut r, "a.tex", &v(&["A a."]), &v(&["A b."]), "ellrin", "t1");
        let id = r.changes[0].id.clone();
        accept(&mut r, &id).unwrap();
        assert!(r.changes.is_empty());
        assert!(accept(&mut r, &id).is_err());
    }

    #[test]
    fn reject_puts_back_each_kind() {
        let mut r = tracked();
        let old = v(&["Alpha one.", "Beta two words here.", "Gamma three."]);
        let new = v(&["Delta new.", "Alpha one.", "Beta two words there."]);
        record(&mut r, "intro.tex", &old, &new, "ellrin", "t1");
        let id_of = |r: &Review, f: fn(&Change) -> bool| r.changes.iter().find(|c| f(c)).unwrap().id.clone();
        let mut now = new.clone();
        // the edit
        let id = id_of(&r, |c| c.before.is_some() && c.after.is_some());
        now = reject(&mut r, &id, &now).unwrap();
        assert_eq!(now, v(&["Delta new.", "Alpha one.", "Beta two words here."]));
        // the insertion
        let id = id_of(&r, |c| c.before.is_none());
        now = reject(&mut r, &id, &now).unwrap();
        assert_eq!(now, v(&["Alpha one.", "Beta two words here."]));
        // the deletion goes back where it was
        let id = id_of(&r, |c| c.after.is_none());
        now = reject(&mut r, &id, &now).unwrap();
        assert_eq!(now, old);
        assert!(r.changes.is_empty());
    }

    #[test]
    fn reject_refuses_when_the_paragraph_changed_since() {
        let mut r = tracked();
        record(&mut r, "a.tex", &v(&["A a."]), &v(&["A b."]), "ellrin", "t1");
        let id = r.changes[0].id.clone();
        // edited outside tracking, without this save path (e.g. by hand)
        let err = reject(&mut r, &id, &v(&["Something else."])).unwrap_err();
        assert!(err.contains("無法拒絕"));
        assert_eq!(r.changes.len(), 1);
    }

    fn comment(file: &str, index: usize, exact: &str, prefix: &str, suffix: &str) -> Comment {
        Comment {
            id: new_id("m"), author: "daidai".into(), at: "t".into(), file: file.into(), index,
            exact: exact.into(), prefix: prefix.into(), suffix: suffix.into(), text: "define?".into(),
            replies: vec![], resolved: false, orphan: false, source: None, extra: Map::new(),
        }
    }

    #[test]
    fn comments_follow_their_text() {
        let mut r = Review::default();
        r.comments.push(comment("a.tex", 1, "long-context grounding", "advances in retrieval, ", " remains"));
        let old = v(&["Intro.", "Despite advances in retrieval, long-context grounding remains open."]);
        // a paragraph goes in before, and the commented one is edited elsewhere
        let new = v(&["Intro.", "New.", "Despite recent advances in retrieval,\nlong-context grounding remains largely open."]);
        record(&mut r, "a.tex", &old, &new, "ellrin", "t1");
        assert_eq!((r.comments[0].index, r.comments[0].orphan), (2, false));
        // the text moves to another paragraph: found there
        let moved = v(&["Long-context grounding? See below.", "New.", "Intro. Now long-context grounding remains open."]);
        reanchor(&mut r, "a.tex", &moved);
        assert_eq!((r.comments[0].index, r.comments[0].orphan), (2, false));
        // the text is gone: orphaned, kept
        let gone = v(&["Intro.", "New.", "Despite advances, context use remains open."]);
        record(&mut r, "a.tex", &moved, &gone, "ellrin", "t2");
        assert!(r.comments[0].orphan);
        assert_eq!(r.comments.len(), 1);
    }

    #[test]
    fn comment_picks_the_occurrence_by_context() {
        let c = comment("a.tex", 0, "the model", "we train ", " on");
        let t = "Here the model is fixed; we train the model on data.";
        let (at, _) = find_anchor(t, &c).unwrap();
        assert_eq!(&t[at..at + 9], "the model");
        assert!(at > 10);
    }

    #[test]
    fn unknown_fields_survive() {
        let src = r#"{"schema":"papercomposer.review/1","tracking":true,"later":1,
          "changes":[{"id":"c1","author":"daidai","at":"2026-01-01T00:00:00Z","file":"a.tex","index":0,"before":"x","after":"y","source":"word","w:rsid":"00AB"}],
          "comments":[]}"#;
        let r: Review = serde_json::from_str(src).unwrap();
        let back: Value = serde_json::to_value(&r).unwrap();
        assert_eq!(back["later"], 1);
        assert_eq!(back["changes"][0]["w:rsid"], "00AB");
        assert_eq!(back["changes"][0]["source"], "word");
    }

    #[test]
    fn save_writes_nothing_for_an_unused_review() {
        let d = std::env::temp_dir().join(format!("pc-review-{}", new_id("t")));
        std::fs::create_dir_all(&d).unwrap();
        save(&d, &Review::default()).unwrap();
        assert!(!d.join(FILE).exists());
        let mut r = tracked();
        r.extra.insert("x".into(), Value::Bool(true));
        save(&d, &r).unwrap();
        assert_eq!(load(&d), r);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
