//! 預想選項: before the interview, a strong model reads the experiment record
//! and the key value and thinks of 2–3 possible answers for every item that
//! needs the author's motive, design reason, observation, reading of a result
//! or (待補) expectation. The interview shows them as choices under the
//! question; the author picks, edits, combines, or says 「都不是」 and answers
//! in their own words.
//!
//! One batched call per plan (and per group added later: 待補項目, 修訂,
//! 審稿追問), run in the background before the group is talked about — never
//! inside a turn. The code keeps only candidates that stand on the record:
//! every evidence id must exist and every number in the text must be in the
//! record exactly ([`validate`]).

use super::lens::Lens;
use super::session::{Item, Session};
use crate::compose::{json_array, json_object, json_string, Prompt};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const TASK: &str = "pc-draft-candidates";
/// Candidates per item at most.
pub const MAX: usize = 3;
/// Items per call: three short candidates each stay well inside one reply
/// (16k tokens); a longer plan is asked in pieces, each kept as it lands.
pub const BATCH: usize = 12;
/// What the author says when none fits.
pub const NONE_OF_THEM: &str = "都不是，我自己說";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Candidate {
    /// k1, k2, … within the item.
    pub id: String,
    /// The possible answer, short and concrete (Traditional Chinese).
    pub text: String,
    /// motive | reason | result | interpretation | expectation
    #[serde(default)]
    pub kind: String,
    /// Record ids it stands on: experiments (E3), timeline steps (T2), the
    /// author's written thoughts (H1), tables (R1), pending results (P1).
    #[serde(default)]
    pub evidence: Vec<String>,
    /// How it serves the key value.
    #[serde(default)]
    pub serves: String,
}

/// State of a group's candidates (shown while they are being prepared).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Prep {
    Running,
    Done,
    Failed,
}

/// An item asks for candidates when the author still owes it something the
/// record can suggest: a motive, a reason, a depth, a number's reading, or the
/// explanation of a pending result. Items that have them are not asked again.
pub fn wants(sess: &Session, lens: &Lens, it: &Item) -> bool {
    if !it.candidates.is_empty() || it.candidates_tried {
        return false;
    }
    match it.pending.as_deref().filter(|_| it.role == super::pending::ROLE) {
        Some(p) => sess.pending.iter().find(|x| x.id == p).is_some_and(|x| !x.note.complete()),
        None => !lens.missing(it).is_empty() || matches!(it.role.as_str(), "revision" | "audit"),
    }
}

/// The items of `groups` that want candidates.
pub fn targets<'a>(sess: &'a Session, lens: &Lens, groups: &[String]) -> Vec<&'a Item> {
    sess.items.iter().filter(|i| groups.contains(&i.group) && wants(sess, lens, i)).collect()
}

pub fn schema() -> serde_json::Value {
    let c = json_object(&[("text", json_string()), ("kind", json_string()), ("evidence", json_array(json_string())), ("serves", json_string())]);
    let it = json_object(&[("id", json_string()), ("candidates", json_array(c)), ("missing", json_string())]);
    json_object(&[("items", json_array(it))])
}

/// One prompt for every item at once. `record` is the raw record text.
pub fn prompt(key_value: &str, sess: &Session, lens: &Lens, items: &[&Item], record: &str, material: &str) -> Prompt {
    let system = "You prepare answer options for an interview about a researcher's own study, before it starts. For \
each item below, think of 2 or 3 answers the author might give — the motive of a decision, the reason for a design, \
what they observed, how they read a result, or (pending results) why the result is needed, what they expect and what \
changes if it comes out the other way. The author picks one, edits it, or answers in their own words.\n\
Rules:\n\
1. Every candidate stands on the experiment record: cite in `evidence` the ids it rests on — experiments (E…), \
timeline steps (T…), the author's written thoughts (H…), tables (R…), pending results (P…). The author's own written \
thoughts come first, close to their words.\n\
2. Numbers only as the record writes them, digit for digit; never a number the record does not have.\n\
3. Candidates of one item differ in substance (not rewordings) and each says something concrete (\"在 run 3 \
中心偏離平均後 Dice 掉到 0.61，所以改成依面積降權\"), never a generic phrase.\n\
4. `serves`: one short line on how the answer serves the key value. `kind`: motive, reason, result, interpretation \
or expectation.\n\
5. If the record cannot support two answers, give fewer (even none) and say in `missing` what the record lacks. \
Never invent facts, motives the record does not hint at, or references.\n\
Write `text`, `serves` and `missing` in Traditional Chinese as used in Taiwan with full-width punctuation. Reply with JSON only.";
    let mut user = super::prompts::context(key_value, sess, material);
    user += "\n";
    user += &lens.describe();
    user += "\n## Experiment record (raw)\n";
    user += record;
    user += "\n## Items that need the author's answer\n";
    for it in items {
        let g = sess.group(&it.group);
        user += &format!("- {} [{} / {}] \"{}\" role={}", it.id, g.map(|g| g.section.as_str()).unwrap_or(""), g.map(|g| g.title.as_str()).unwrap_or(""), it.label, it.role);
        if let Some(p) = it.pending.as_deref().and_then(|p| sess.pending.iter().find(|x| x.id == p)) {
            user += &format!(" — pending {}: {}; expected {}; waits for {}", p.id, p.what, p.expected, p.waits_for);
        }
        if !it.stance.is_empty() {
            user += &format!(" — said so far: {}", it.stance);
        }
        user += "\n";
    }
    Prompt { system: system.into(), user }
}

#[derive(Debug, Deserialize)]
pub struct Reply {
    pub items: Vec<RawItem>,
}
#[derive(Debug, Deserialize)]
pub struct RawItem {
    pub id: String,
    pub candidates: Vec<RawCandidate>,
    #[serde(default)]
    pub missing: String,
}
#[derive(Debug, Deserialize)]
pub struct RawCandidate {
    pub text: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub serves: String,
}

pub fn parse(reply: &str) -> Result<Reply, String> {
    super::prompts::json_in(reply)
}

/// The ids a candidate may cite.
pub fn known_ids(sess: &Session) -> BTreeSet<String> {
    let mut k: BTreeSet<String> = BTreeSet::new();
    k.extend(sess.experiments.iter().map(|e| e.id.clone()));
    k.extend(sess.timeline.iter().map(|t| format!("T{}", t.step)));
    k.extend(sess.thoughts.iter().map(|h| h.id.clone()));
    k.extend(sess.tables.iter().map(|t| t.id.clone()));
    k.extend(sess.pending.iter().map(|p| p.id.clone()));
    k
}

/// Everything the record says, for the number check.
pub fn corpus(sess: &Session, record: &str) -> String {
    let mut c = record.to_string();
    c += &sess.record_summary;
    for e in &sess.experiments {
        c += &format!("\n{} {} {} {}", e.change, e.metrics, e.result, e.title);
    }
    for t in &sess.timeline {
        c += &format!("\n{} {} {}", t.what, t.observed, t.changed);
    }
    for h in &sess.thoughts {
        c += &format!("\n{}", h.text);
    }
    for t in &sess.tables {
        c += &format!("\n{}", t.tsv());
    }
    c
}

/// Numbers a reader would check: anything with a decimal point or percent, or
/// two digits and more (`3 個` or `T2` are not numbers here).
pub fn numbers(text: &str) -> Vec<String> {
    let mut out = vec![];
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let prev_alpha = i > 0 && (cs[i - 1].is_ascii_alphabetic() || cs[i - 1] == '_');
        if cs[i].is_ascii_digit() && !prev_alpha {
            let mut j = i;
            while j < cs.len() && (cs[j].is_ascii_digit() || (cs[j] == '.' && cs.get(j + 1).is_some_and(|c| c.is_ascii_digit()))) {
                j += 1;
            }
            let n: String = cs[i..j].iter().collect();
            let pct = cs.get(j) == Some(&'%');
            if n.contains('.') || pct || n.len() >= 2 {
                out.push(n);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

/// What the check did to one item's candidates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Checked {
    pub item: String,
    pub kept: Vec<Candidate>,
    /// Why each dropped candidate went (for the log / the tests).
    pub dropped: Vec<String>,
    pub missing: String,
}

/// Keep candidates that stand on the record: evidence ids that do not exist
/// are removed and a candidate left with none goes; a number not in the record
/// (digit for digit) drops the candidate; duplicates go; at most [`MAX`].
/// Items not asked about are ignored.
pub fn validate(reply: Reply, sess: &Session, asked: &[String], record: &str) -> Vec<Checked> {
    let ids = known_ids(sess);
    // numbers compared as whole tokens: 0.6 is not 0.61, 69 is not 0.69
    let corpus: BTreeSet<String> = numbers(&corpus(sess, record)).into_iter().collect();
    let norm = |s: &str| s.trim().trim_matches(['(', ')', '[', ']', ' ']).to_uppercase().replace(' ', "");
    let mut out: Vec<Checked> = vec![];
    for it in reply.items {
        let id = it.id.trim().to_string();
        if !asked.contains(&id) || out.iter().any(|c| c.item == id) {
            continue;
        }
        let mut ch = Checked { item: id, missing: it.missing.trim().to_string(), ..Default::default() };
        for c in it.candidates {
            let text = c.text.trim().to_string();
            if text.is_empty() || ch.kept.iter().any(|k| k.text == text) {
                continue;
            }
            let evidence: Vec<String> = c.evidence.iter().map(|e| norm(e)).filter(|e| ids.iter().any(|k| k.to_uppercase() == *e)).map(|e| ids.iter().find(|k| k.to_uppercase() == e).cloned().unwrap_or(e)).collect();
            if evidence.is_empty() {
                ch.dropped.push(format!("「{text}」：沒有記錄裡存在的依據"));
                continue;
            }
            if let Some(n) = numbers(&text).into_iter().find(|n| !corpus.contains(n)) {
                ch.dropped.push(format!("「{text}」：記錄裡沒有 {n}"));
                continue;
            }
            if ch.kept.len() >= MAX {
                continue;
            }
            let mut ev = evidence;
            ev.dedup();
            ch.kept.push(Candidate { id: format!("k{}", ch.kept.len() + 1), text, kind: c.kind.trim().to_string(), evidence: ev, serves: c.serves.trim().to_string() });
        }
        out.push(ch);
    }
    out
}

/// Put checked candidates on their items (an item's earlier candidates are kept
/// unless new ones came). Returns how many items got some.
pub fn apply(sess: &mut Session, asked: &[String], checked: &[Checked]) -> usize {
    mark_tried(sess, asked);
    let mut n = 0;
    for c in checked {
        let Some(it) = sess.items.iter_mut().find(|i| i.id == c.item) else { continue };
        if !c.kept.is_empty() {
            it.candidates = c.kept.clone();
            n += 1;
        }
        if !c.missing.is_empty() {
            it.candidates_missing = c.missing.clone();
        }
    }
    n
}

/// Items asked about once, whatever came back (a failed call too): the
/// interview never asks again by itself.
pub fn mark_tried(sess: &mut Session, asked: &[String]) {
    for it in sess.items.iter_mut().filter(|i| asked.contains(&i.id)) {
        it.candidates_tried = true;
    }
}

/// 重試: the items of `group` without candidates may be asked again.
pub fn retry(sess: &mut Session, group: &str) {
    for it in sess.items.iter_mut().filter(|i| i.group == group && i.candidates.is_empty()) {
        it.candidates_tried = false;
    }
}

/// A spoken or typed choice (「第二個」, `2`, `k2`) → the candidate id.
pub fn spoken_choice(answer: &str, it: &Item) -> Option<String> {
    let a = answer.trim().trim_end_matches(['。', '.', '！', '!']).trim();
    let a = a.strip_prefix("選").unwrap_or(a).trim();
    let n = match a.trim_start_matches('第').trim_end_matches(['個', '项', '項']).trim() {
        "1" | "一" | "k1" => 1,
        "2" | "二" | "k2" => 2,
        "3" | "三" | "k3" => 3,
        _ => return None,
    };
    it.candidates.get(n - 1).map(|c| c.id.clone())
}

/// The author's answer to a question about item `about`: candidates they
/// clicked (`picked`, possibly edited in the answer), or a spoken / typed
/// choice (「第二個」 → that candidate's text). Unknown ids are dropped; any
/// other answer (都不是) goes through as said, with nothing picked.
pub fn resolve_answer(sess: &Session, about: &str, answer: Option<String>, mut picked: Vec<String>) -> (Option<String>, Vec<String>) {
    let Some(it) = sess.items.iter().find(|i| i.id == about) else { return (answer, vec![]) };
    if picked.is_empty()
        && let Some(k) = answer.as_deref().and_then(|a| spoken_choice(a, it))
    {
        let text = it.candidates.iter().find(|c| c.id == k).map(|c| c.text.clone());
        return (text, vec![k]);
    }
    picked.retain(|k| it.candidates.iter().any(|c| &c.id == k));
    picked.dedup();
    (answer, picked)
}

/// Remember the candidates the author chose on the item.
pub fn note_picked(sess: &mut Session, about: &str, picked: &[String]) {
    if let Some(it) = sess.items.iter_mut().find(|i| i.id == about) {
        for k in picked {
            if !it.picked.contains(k) {
                it.picked.push(k.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::session::{Experiment, Group, Thought, TimelineStep};

    fn sess() -> Session {
        let mut s = Session::default();
        s.experiments.push(Experiment { id: "E1".into(), title: "Fuzzy weight".into(), metrics: "Dice".into(), result: "0.72 vs 0.69".into(), ..Default::default() });
        s.timeline.push(TimelineStep { step: 3, what: "moved the centre away from the mean".into(), observed: "Dice fell to 0.61".into(), changed: "weight by area".into() });
        s.thoughts.push(Thought { id: "H1".into(), text: "模糊的影格其實資訊很多，丟掉可惜".into(), about: vec!["E1".into()] });
        s.set_plan(vec![(Group { section: "Methods".into(), title: "Loss".into(), ..Default::default() }, vec![Item { label: "fuzzy weight".into(), role: "core-contribution".into(), ..Default::default() }, Item { label: "BCE".into(), role: "design".into(), ..Default::default() }])]);
        s
    }

    const REPLY: &str = r#"{"items":[
      {"id":"i1","candidates":[
        {"text":"模糊影格資訊很多，丟掉可惜，所以改成降權","kind":"motive","evidence":["H1","E1"],"serves":"讓不確定影格也算數"},
        {"text":"run 3 中心偏離平均後 Dice 掉到 0.61，所以改成依面積降權","kind":"reason","evidence":["T3"],"serves":"權重設計的由來"},
        {"text":"加權後 Dice 從 0.69 升到 0.75","kind":"result","evidence":["E1"],"serves":"x"},
        {"text":"文獻都這樣做","kind":"reason","evidence":["kim2019"],"serves":"x"},
        {"text":"模糊影格資訊很多，丟掉可惜，所以改成降權","kind":"motive","evidence":["H1"],"serves":"dup"}
      ],"missing":""},
      {"id":"i2","candidates":[],"missing":"記錄沒有寫為什麼選 BCE"},
      {"id":"i9","candidates":[{"text":"x","kind":"","evidence":["E1"],"serves":""}],"missing":""}
    ]}"#;

    #[test]
    fn only_candidates_that_stand_on_the_record_are_kept() {
        let mut s = sess();
        let lens = Lens::builtin();
        let asked: Vec<String> = targets(&s, &lens, &["g1".into()]).iter().map(|i| i.id.clone()).collect();
        assert_eq!(asked, ["i1", "i2"]);
        let p = prompt("Make uncertain frames count", &s, &lens, &targets(&s, &lens, &["g1".into()]), "run 3: Dice 0.61", "");
        assert!(p.user.contains("i1 [Methods / Loss] \"fuzzy weight\"") && p.user.contains("run 3: Dice 0.61") && p.user.contains("H1"));
        let c = validate(parse(REPLY).unwrap(), &s, &asked, "run 3: Dice 0.61");
        assert_eq!(c.len(), 2, "an item that was not asked about is ignored");
        assert_eq!(c[0].kept.iter().map(|k| (k.id.as_str(), k.evidence.join(","))).collect::<Vec<_>>(), [("k1", "H1,E1".to_string()), ("k2", "T3".to_string())]);
        assert!(c[0].dropped.iter().any(|d| d.contains("0.75")), "a number the record does not have: {:?}", c[0].dropped);
        assert!(c[0].dropped.iter().any(|d| d.contains("文獻都這樣做")), "evidence that does not exist");
        assert_eq!(c[1].missing, "記錄沒有寫為什麼選 BCE");
        assert_eq!(apply(&mut s, &asked, &c), 1);
        assert_eq!(s.items[0].candidates.len(), 2);
        assert_eq!(s.items[1].candidates_missing, "記錄沒有寫為什麼選 BCE");
        assert!(targets(&s, &lens, &["g1".into()]).is_empty(), "asked once: never again by itself, kept or not (no Opus call per turn)");
        retry(&mut s, "g1");
        assert_eq!(targets(&s, &lens, &["g1".into()]).iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["i2"], "重試 asks again for the item that got none");
        // whole numbers only: 0.6 is not 0.61, 69 is not 0.69
        let r = parse(r#"{"items":[{"id":"i2","candidates":[{"text":"Dice 掉到 0.6","kind":"result","evidence":["T3"],"serves":""},{"text":"baseline 69 分","kind":"result","evidence":["E1"],"serves":""},{"text":"Dice 0.61","kind":"result","evidence":["T3"],"serves":""}],"missing":""}]}"#).unwrap();
        let c = validate(r, &s, &["i2".into()], "run 3: Dice 0.61");
        assert_eq!(c[0].kept.iter().map(|k| k.text.as_str()).collect::<Vec<_>>(), ["Dice 0.61"]);
    }

    #[test]
    fn numbers_are_what_a_reader_would_check() {
        assert_eq!(numbers("run 3 的 Dice 0.61，提升 12%，T2 與 E10 不算"), ["0.61", "12"]);
        assert!(numbers("3 個 run").is_empty());
    }

    #[test]
    fn a_spoken_choice_picks_the_candidate() {
        let mut s = sess();
        s.items[0].candidates = vec![Candidate { id: "k1".into(), text: "a".into(), ..Default::default() }, Candidate { id: "k2".into(), text: "b".into(), ..Default::default() }];
        let it = &s.items[0];
        assert_eq!(spoken_choice("第二個", it).as_deref(), Some("k2"));
        assert_eq!(spoken_choice("選 1。", it).as_deref(), Some("k1"));
        assert_eq!(spoken_choice("第三個", it), None, "there is no third");
        assert_eq!(spoken_choice("第二個原因是資料太少", it), None, "an answer, not a choice");
    }

    #[test]
    fn a_choice_is_recorded_and_none_of_them_is_an_answer() {
        let mut s = sess();
        s.items[0].candidates = vec![
            Candidate { id: "k1".into(), text: "模糊影格資訊很多".into(), ..Default::default() },
            Candidate { id: "k2".into(), text: "run 3 之後改成依面積降權".into(), ..Default::default() },
        ];
        // clicked two and edited the text: the edit is the answer, both are recorded
        let (a, p) = resolve_answer(&s, "i1", Some("兩個都是，主要是第一個".into()), vec!["k1".into(), "k2".into(), "k9".into()]);
        assert_eq!((a.as_deref(), p.as_slice()), (Some("兩個都是，主要是第一個"), ["k1".to_string(), "k2".to_string()].as_slice()));
        note_picked(&mut s, "i1", &p);
        note_picked(&mut s, "i1", &p);
        assert_eq!(s.items[0].picked, ["k1", "k2"]);
        // said aloud
        let (a, p) = resolve_answer(&s, "i1", Some("第二個".into()), vec![]);
        assert_eq!((a.as_deref(), p.as_slice()), (Some("run 3 之後改成依面積降權"), ["k2".to_string()].as_slice()));
        // 都不是: the author's own words, nothing picked
        let (a, p) = resolve_answer(&s, "i1", Some("其實是醫師建議的".into()), vec![]);
        assert_eq!((a.as_deref(), p.len()), (Some("其實是醫師建議的"), 0));
        // a question about no item: as said
        assert_eq!(resolve_answer(&s, "", Some("第二個".into()), vec!["k2".into()]), (Some("第二個".into()), vec![]));
    }

    #[test]
    fn pending_items_want_candidates_until_explained() {
        use crate::draft::pending::{sync_group, PendingItem, GROUP_ID};
        let mut s = sess();
        s.pending.push(PendingItem { id: "P1".into(), what: "external Dice".into(), ..Default::default() });
        sync_group(&mut s);
        let lens = Lens::builtin();
        assert_eq!(targets(&s, &lens, &[GROUP_ID.into()]).len(), 1);
        s.pending[0].note = crate::draft::pending::PendingNote { what: "a".into(), why: "b".into(), expect: "c".into(), if_not: "d".into() };
        assert!(targets(&s, &lens, &[GROUP_ID.into()]).is_empty());
    }
}
