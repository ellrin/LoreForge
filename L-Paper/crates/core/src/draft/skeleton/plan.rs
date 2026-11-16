//! The model's three jobs on a skeleton, each with its prompt, schema and checked parsing:
//!
//! - [`TASK`] `pc-draft-skeleton`: the whole paper at once (sections → blocks → paragraphs, the
//!   points the record already supports) and a short list of questions anchored to paragraphs.
//!   Planned in the order Results → Discussion → Methods → Introduction / Related work /
//!   Conclusion, returned in paper order. Code keeps only labels of the kept inventory, known
//!   reference keys, record ids that exist; empty blocks go; at most [`MAX_QUESTIONS`].
//! - [`REPAIR_TASK`] `pc-draft-skeleton-repair` (暫存): only the touched paragraphs and blocks and
//!   their neighbours; every other paragraph stays byte-identical (code), every change becomes
//!   an applied [`Change`] in an `edit` thread.
//! - [`FILL_TASK`] `pc-draft-fill`: one answer → that paragraph's points, the thread's
//!   conclusion, at most one follow-up when the answer opened a real gap (never after "no").

use super::*;
use crate::compose::{json_array, json_object, json_string, Prompt};
use crate::draft::prompts::{json_in, TW};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const TASK: &str = "pc-draft-skeleton";
pub const REPAIR_TASK: &str = "pc-draft-skeleton-repair";
pub const FILL_TASK: &str = "pc-draft-fill";
/// Questions kept from a plan, most important first.
pub const MAX_QUESTIONS: usize = 15;
pub const QUESTION_KINDS: &[&str] = &["conflict", "choice", "missing"];

fn strings() -> Value {
    json_array(json_string())
}

// ── the planner ──────────────────────────────────────────

pub fn schema() -> Value {
    let para = json_object(&[
        ("key", json_string()),
        ("kind", json_string()),
        ("say", json_string()),
        ("points", strings()),
        ("link_prev", json_string()),
        ("link_next", json_string()),
        ("floats", strings()),
        ("cites", strings()),
        ("sources", strings()),
        ("pending", strings()),
        ("answers", strings()),
    ]);
    let block = json_object(&[("key", json_string()), ("title", json_string()), ("purpose", json_string()), ("of", json_string()), ("paras", json_array(para))]);
    let section = json_object(&[("title", json_string()), ("role", json_string()), ("blocks", json_array(block))]);
    let q = json_object(&[("para", json_string()), ("kind", json_string()), ("question", json_string()), ("options", strings())]);
    json_object(&[("sections", json_array(section)), ("questions", json_array(q))])
}

/// What the planner reads.
#[derive(Default)]
pub struct Input<'a> {
    /// [`crate::draft::prompts::context`]: key value, hook, the organised record, the paper's
    /// floats and references, the notation registry.
    pub context: &'a str,
    /// [`super::inventory::block`]: what the author keeps.
    pub inventory: &'a str,
    /// The author's style notes (paragraph types).
    pub style: &'a str,
    /// The journal's outline block (期刊寫法).
    pub venue: &'a str,
    /// [`super::convert::answers_block`] when converting a session mid-interview.
    pub answers: &'a str,
}

pub fn prompt(i: &Input) -> Prompt {
    let system = format!(
        "You lay out a whole research paper as a skeleton before any text is written: sections → blocks → paragraphs.\n\
A block is a frame around a few paragraphs that serve one purpose (\"explain Table 1\", \"why the external result differs\"). \
Each paragraph says ONE thing, in one line.\n\
Plan in this order, because each part depends on the one before: first Results (one block per table or figure, read its \
numbers, the key comparisons), then Discussion (what the results mean, what differs, limits of the method itself), then \
Methods (what the results need explained), then Introduction, Related work (only when the author's style or the journal has \
one) and Conclusion. Return the sections in PAPER order (Introduction first).\n\
Rules:\n\
1. Section `role`: introduction, related, methods, results, discussion, conclusion or other. `title` = the section title \
in English (the journal's wording when given).\n\
2. Block: `key` (b1, b2 … yours), `title` and `purpose` for the author (what the reader knows after it), `of` = the key of \
the block or paragraph it expands (a Discussion block that explains a Results block), else \"\".\n\
3. Paragraph: `key` (x1, x2 … yours, unique), `kind` = one of the author's paragraph types from the style notes when one \
fits, else \"\"; `say` = what it says in one line; `points` = what the record ALREADY supports for it, short lines, numbers \
copied exactly as the record has them, each backed by an id in `sources` ([] when the record has nothing for it); \
`link_prev` / `link_next` = in a few words how it follows the paragraph before and what it hands to the next (\"\" at the \
ends of the paper); `floats` = labels of the author's list below that it presents (each label presented once, where its \
result is read; a workflow figure in Methods); `cites` = keys of the paper's references it cites (only keys listed, \
within the author's note on each reference); `sources` = record ids it draws on (E…, T…, H…, R…, P…; only ids listed); \
`pending` = pending result ids (P…) whose value it states or concludes from; `answers` = ids of the interview groups whose \
answers it uses (only when interview answers are listed, else []).\n\
4. Each claim is said in full once, in its home section; elsewhere it is only called back.\n\
5. Questions: only where the skeleton needs the author and the record cannot tell. Three kinds only: `conflict` (the \
record and what the author said or the key value disagree), `choice` (the author must pick between ways to say or order \
something), `missing` (the paragraph needs a fact or a reason the record lacks). Each is anchored to one paragraph \
(`para` = its key), most important first, at most {MAX_QUESTIONS}. `options` = 2 or 3 likely answers taken from the record. \
Never ask what the record or the given answers already say. Never name internal ids (turn numbers, E1, g3, x4) in the \
wording; address the author as 「你」; do not restate their earlier answers.\n\
6. Do not invent results, numbers or citations.\n\
{TW} `say`, `points`, `link_*`, titles, purposes and questions are for the author. Reply with JSON only."
    );
    let mut user = i.context.to_string();
    user += i.inventory;
    if !i.venue.trim().is_empty() {
        user += "\n## The journal's habits\n";
        user += i.venue;
    }
    if !i.style.trim().is_empty() {
        user += "\n## The author's style (paragraph types, proportions, habits)\n";
        user += i.style;
    }
    user += i.answers;
    user += "\nLay out the whole paper now.";
    Prompt { system, user }
}

#[derive(Debug, Deserialize)]
struct Reply {
    sections: Vec<RawSection>,
    #[serde(default)]
    questions: Vec<RawQuestion>,
}
#[derive(Debug, Deserialize)]
struct RawSection {
    title: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    blocks: Vec<RawBlock>,
}
#[derive(Debug, Deserialize)]
struct RawBlock {
    #[serde(default)]
    key: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    purpose: String,
    #[serde(default)]
    of: Option<String>,
    #[serde(default)]
    paras: Vec<RawPara>,
}
#[derive(Debug, Deserialize)]
struct RawPara {
    #[serde(default)]
    key: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    say: String,
    #[serde(default)]
    points: Vec<String>,
    #[serde(default)]
    link_prev: String,
    #[serde(default)]
    link_next: String,
    #[serde(default)]
    floats: Vec<String>,
    #[serde(default)]
    cites: Vec<String>,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    pending: Vec<String>,
    #[serde(default)]
    answers: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct RawQuestion {
    para: String,
    kind: String,
    question: String,
    #[serde(default)]
    options: Vec<String>,
}

/// What the code checks against.
#[derive(Default)]
pub struct Known {
    /// Labels of the kept inventory.
    pub labels: BTreeSet<String>,
    /// Reference keys of the paper.
    pub cites: BTreeSet<String>,
    /// Record ids ([`crate::draft::candidates::known_ids`]).
    pub sources: BTreeSet<String>,
    /// Pending result ids.
    pub pending: BTreeSet<String>,
    /// Interview groups (when converting).
    pub groups: BTreeSet<String>,
}

/// A checked plan: the skeleton's sections and questions (ids given from `sk`'s counters), and
/// for a conversion the groups each paragraph uses.
pub struct Planned {
    pub sections: Vec<Section>,
    pub questions: Vec<Thread>,
    pub uses: BTreeMap<String, Vec<String>>,
}

/// Internal ids a question must not show the author (turn numbers, record and plan ids).
pub fn scrub(text: &str, ids: &BTreeSet<String>) -> String {
    let cs: Vec<char> = text.chars().collect();
    let word = |k: usize| -> usize {
        let mut j = k;
        while j < cs.len() && cs[j].is_ascii_alphanumeric() {
            j += 1;
        }
        j
    };
    let skip_digits = |mut j: usize| -> Option<usize> {
        while j < cs.len() && cs[j] == ' ' {
            j += 1;
        }
        let d = j;
        while j < cs.len() && cs[j].is_ascii_digit() {
            j += 1;
        }
        (j > d).then_some(j)
    };
    let mut s = String::new();
    let mut k = 0;
    while k < cs.len() {
        if cs[k].is_ascii_alphanumeric() {
            let j = word(k);
            let w: String = cs[k..j].iter().collect();
            if w.eq_ignore_ascii_case("turn") {
                if let Some(e) = skip_digits(j) {
                    k = e;
                    continue;
                }
            }
            if !ids.contains(&w) {
                s.push_str(&w);
            }
            k = j;
            continue;
        }
        // 第 3 輪 / 第 3 題
        if cs[k] == '第' {
            if let Some(mut j) = skip_digits(k + 1) {
                while j < cs.len() && cs[j] == ' ' {
                    j += 1;
                }
                if j < cs.len() && (cs[j] == '輪' || cs[j] == '題') {
                    k = j + 1;
                    continue;
                }
            }
        }
        s.push(cs[k]);
        k += 1;
    }
    for pat in ["( )", "（ ）", "()", "（）"] {
        s = s.replace(pat, "");
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

/// The reply, checked, with ids from `sk`.
pub fn parse(reply: &str, known: &Known, sk: &mut Skeleton) -> Result<Planned, String> {
    let r: Reply = json_in(reply)?;
    let mut key_to_id: BTreeMap<String, String> = BTreeMap::new();
    let mut of_keys: Vec<(String, String)> = vec![]; // (block id, its `of` key)
    let mut uses: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut sections: Vec<Section> = vec![];
    let keep = |v: Vec<String>, set: &BTreeSet<String>| -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for x in v.into_iter().map(|x| x.trim().to_string()) {
            if set.contains(&x) && !out.contains(&x) {
                out.push(x);
            }
        }
        out
    };
    for rs in r.sections {
        if rs.title.trim().is_empty() {
            continue;
        }
        let role = if ROLES.contains(&rs.role.trim()) { rs.role.trim().to_string() } else { role_of(&rs.title) };
        let mut sec = Section { id: String::new(), title: rs.title.trim().to_string(), role, blocks: vec![] };
        for rb in rs.blocks {
            let mut b = Block { id: String::new(), title: rb.title.trim().to_string(), purpose: rb.purpose.trim().to_string(), of: None, paras: vec![] };
            for rp in rb.paras {
                if rp.say.trim().is_empty() {
                    continue;
                }
                let sources = keep(rp.sources, &known.sources);
                let mut p = Para {
                    id: sk.new_id("p"),
                    kind: rp.kind.trim().to_string(),
                    say: rp.say.trim().to_string(),
                    points: rp.points.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map(|text| Point { text, from: None }).collect(),
                    link_prev: rp.link_prev.trim().to_string(),
                    link_next: rp.link_next.trim().to_string(),
                    floats: keep(rp.floats, &known.labels),
                    cites: keep(rp.cites, &known.cites),
                    sources,
                    pending: keep(rp.pending, &known.pending),
                    ..Default::default()
                };
                p.settle_state();
                let used = keep(rp.answers, &known.groups);
                if !used.is_empty() {
                    uses.insert(p.id.clone(), used);
                }
                if !rp.key.trim().is_empty() {
                    key_to_id.insert(rp.key.trim().to_string(), p.id.clone());
                }
                b.paras.push(p);
            }
            if b.paras.is_empty() {
                continue;
            }
            b.id = sk.new_id("b");
            if !rb.key.trim().is_empty() {
                key_to_id.insert(rb.key.trim().to_string(), b.id.clone());
            }
            if let Some(o) = rb.of.map(|o| o.trim().to_string()).filter(|o| !o.is_empty()) {
                of_keys.push((b.id.clone(), o));
            }
            sec.blocks.push(b);
        }
        if sec.blocks.is_empty() {
            continue;
        }
        sec.id = sk.new_id("s");
        sections.push(sec);
    }
    if sections.is_empty() {
        return Err("骨架是空的".into());
    }
    // paper order by role (the model was asked for it; a misplaced section is moved, never dropped)
    let rank = |r: &str| ["introduction", "related", "methods", "results", "discussion", "conclusion"].iter().position(|x| *x == r).unwrap_or(5);
    sections.sort_by_key(|s| rank(&s.role));
    for (bid, key) in of_keys {
        let target = key_to_id.get(&key).cloned().filter(|t| t != &bid);
        for b in sections.iter_mut().flat_map(|s| s.blocks.iter_mut()) {
            if b.id == bid {
                b.of = target.clone();
            }
        }
    }
    let mut ids: BTreeSet<String> = known.sources.iter().chain(known.groups.iter()).cloned().collect();
    ids.extend(key_to_id.keys().cloned());
    let mut questions: Vec<Thread> = vec![];
    for q in r.questions {
        let kind = q.kind.trim().to_lowercase();
        let Some(anchor) = key_to_id.get(q.para.trim()).filter(|a| a.starts_with('p')).cloned() else { continue };
        if !QUESTION_KINDS.contains(&kind.as_str()) {
            continue;
        }
        let text = scrub(&q.question, &ids);
        if text.is_empty() || questions.iter().any(|x| x.question == text) {
            continue;
        }
        let opts: Vec<String> = q.options.into_iter().map(|o| scrub(&o, &ids)).filter(|o| !o.is_empty()).take(3).collect();
        questions.push(Thread { id: String::new(), anchor, kind, status: "open".into(), question: text.clone(), options: options(opts), messages: vec![Msg { who: "ai".into(), text }], conclusion: String::new(), changes: vec![] });
        if questions.len() == MAX_QUESTIONS {
            break;
        }
    }
    Ok(Planned { sections, questions, uses })
}

/// A checked plan becomes the skeleton (the inventory and the counters stay; questions get ids).
pub fn install(sk: &mut Skeleton, p: Planned) {
    sk.sections = p.sections;
    sk.threads.clear();
    sk.undo.clear();
    sk.seams.clear();
    sk.touched_blocks.clear();
    sk.follow_of.clear();
    sk.edits = 0;
    for q in p.questions {
        sk.add_thread(q);
    }
}

// ── 暫存: the repair ─────────────────────────────────────

pub fn repair_schema() -> Value {
    let p = json_object(&[("id", json_string()), ("say", json_string()), ("link_prev", json_string()), ("link_next", json_string()), ("why", json_string())]);
    let b = json_object(&[("id", json_string()), ("purpose", json_string()), ("why", json_string())]);
    let j = json_object(&[("id", json_string()), ("why", json_string()), ("fix", json_string())]);
    json_object(&[("paras", json_array(p)), ("blocks", json_array(b)), ("jumps", json_array(j))])
}

/// The touched paragraphs and blocks with their neighbours, nothing else.
pub fn repair_prompt(key_value: &str, sk: &Skeleton, paras: &[String], blocks: &[String]) -> Prompt {
    let system = format!(
        "The author edited the skeleton of their paper (added, removed, moved or rewrote paragraphs). Repair the seams.\n\
You see only the paragraphs that changed (marked CHANGED, with the author's reason when given) and their neighbours. For \
each paragraph you would change give `id`, the new `say` (keep it when it is right), `link_prev` and `link_next` (how it \
follows the paragraph before and what it hands to the next, a few words, \"\" at the ends), and `why` (one short line). \
Blocks: `id`, a new `purpose` when the paragraphs it holds changed what it does, `why`. `jumps`: a paragraph that does \
not follow from the one before it even with a link (`why`, and a `fix` the author could make). Change as little as \
possible: the author's own `say` is theirs (only rephrase it when the move made it wrong), list only what you change. \
Never mention internal ids in the text.\n{TW} Reply with JSON only."
    );
    let mut user = format!("Key value of the paper: {}\n", key_value.trim());
    let order = sk.order();
    let mut last_block = String::new();
    for id in order.iter().filter(|id| paras.contains(id)) {
        let (Some(p), Some(b), Some(s)) = (sk.para(id), sk.block_of(id), sk.section_of(id)) else { continue };
        if b.id != last_block {
            user += &format!("\n## [{}] {} → block {} \"{}\": {}{}\n", s.title, s.role, b.id, b.title, b.purpose, if blocks.contains(&b.id) { " (CHANGED)" } else { "" });
            last_block = b.id.clone();
        }
        let changed = p.touched || sk.seams.contains(id);
        user += &format!(
            "- {}{}: {} | link_prev: {} | link_next: {}{}\n",
            p.id,
            if changed { " CHANGED" } else { "" },
            p.say,
            p.link_prev,
            p.link_next,
            if p.reason.is_empty() { String::new() } else { format!(" | the author's reason: {}", p.reason) }
        );
    }
    Prompt { system, user }
}

#[derive(Debug, Deserialize, Default)]
pub struct RepairReply {
    #[serde(default)]
    pub paras: Vec<RepairPara>,
    #[serde(default)]
    pub blocks: Vec<RepairBlock>,
    #[serde(default)]
    pub jumps: Vec<RepairJump>,
}
#[derive(Debug, Deserialize, Default)]
pub struct RepairPara {
    pub id: String,
    #[serde(default)]
    pub say: Option<String>,
    #[serde(default)]
    pub link_prev: Option<String>,
    #[serde(default)]
    pub link_next: Option<String>,
    #[serde(default)]
    pub why: String,
}
#[derive(Debug, Deserialize, Default)]
pub struct RepairBlock {
    pub id: String,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub why: String,
}
#[derive(Debug, Deserialize, Default)]
pub struct RepairJump {
    pub id: String,
    pub why: String,
    #[serde(default)]
    pub fix: String,
}

pub fn parse_repair(reply: &str) -> Result<RepairReply, String> {
    json_in(reply)
}

/// The repair into the skeleton: only `paras` / `blocks` (the scope the model saw) change, each
/// change an applied [`Change`] in an `edit` thread on its target; the marks of what was repaired
/// clear (an edit made while the repair ran stays marked).
pub fn apply_repair(sk: &mut Skeleton, r: RepairReply, paras: &[String], blocks: &[String]) -> usize {
    let mut by_target: Vec<(String, Vec<Change>)> = vec![];
    let mut add = |target: &str, c: Change| match by_target.iter_mut().find(|(t, _)| t == target) {
        Some((_, v)) => v.push(c),
        None => by_target.push((target.to_string(), vec![c])),
    };
    for rp in r.paras {
        if !paras.contains(&rp.id) {
            continue;
        }
        let Some(p) = sk.para_mut(&rp.id) else { continue };
        for (field, new) in [("say", rp.say), ("link_prev", rp.link_prev), ("link_next", rp.link_next)] {
            let Some(new) = new.map(|x| x.trim().to_string()) else { continue };
            let cur = match field {
                "say" => &mut p.say,
                "link_prev" => &mut p.link_prev,
                _ => &mut p.link_next,
            };
            if *cur == new || (field == "say" && new.is_empty()) {
                continue;
            }
            let before = std::mem::replace(cur, new.clone());
            add(&rp.id, Change { id: String::new(), target: rp.id.clone(), field: field.into(), before, after: new, why: rp.why.trim().to_string(), state: "applied".into() });
        }
    }
    for rb in r.blocks {
        if !blocks.contains(&rb.id) {
            continue;
        }
        let Some(b) = sk.block_mut(&rb.id) else { continue };
        let Some(new) = rb.purpose.map(|x| x.trim().to_string()).filter(|x| !x.is_empty() && *x != b.purpose) else { continue };
        let before = std::mem::replace(&mut b.purpose, new.clone());
        add(&rb.id, Change { id: String::new(), target: rb.id.clone(), field: "purpose".into(), before, after: new, why: rb.why.trim().to_string(), state: "applied".into() });
    }
    for j in r.jumps {
        if paras.contains(&j.id) && !j.why.trim().is_empty() {
            if let Some(p) = sk.para_mut(&j.id) {
                p.jump = Some(Jump { why: j.why.trim().to_string(), fix: j.fix.trim().to_string() });
            }
        }
    }
    let n = by_target.iter().map(|(_, v)| v.len()).sum();
    for (target, mut changes) in by_target {
        for (k, c) in changes.iter_mut().enumerate() {
            c.id = format!("c{}", k + 1);
        }
        let conclusion = changes.iter().map(|c| c.why.clone()).find(|w| !w.is_empty()).unwrap_or_else(|| "暫存時補上銜接".into());
        sk.add_thread(Thread { id: String::new(), anchor: target, kind: "edit".into(), status: "done".into(), question: String::new(), options: vec![], messages: vec![], conclusion, changes });
    }
    // what was in the scope is repaired; the rest keeps its marks
    for id in paras {
        if let Some(p) = sk.para_mut(id) {
            p.touched = false;
            p.reason.clear();
        }
    }
    sk.seams.retain(|s| !paras.contains(s));
    sk.touched_blocks.retain(|b| !blocks.contains(b));
    if sk.seams.is_empty() && sk.touched_blocks.is_empty() && !sk.paras().any(|p| p.touched) {
        sk.edits = 0;
    }
    n
}

// ── 補肉: one answer folded in ───────────────────────────

pub fn fill_schema() -> Value {
    let follow = json_object(&[("kind", json_string()), ("question", json_string()), ("options", strings())]);
    let note = json_object(&[("text", json_string()), ("scope", json_string()), ("banned", strings())]);
    json_object(&[
        ("points", strings()),
        ("conclusion", json_string()),
        ("gap", json!({ "type": "boolean" })),
        ("follow_up", json!({ "anyOf": [follow, { "type": "null" }] })),
        ("writing_notes", json_array(note)),
    ])
}

/// One answer (or an unasked remark) on a paragraph.
pub fn fill_prompt(key_value: &str, sk: &Skeleton, t: &Thread, record: &str) -> Prompt {
    let system = format!(
        "The author answered a question about one paragraph of their paper's skeleton (or said something about it unasked). \
Fold the answer into the paragraph.\n\
`points` = the paragraph's points that come from THIS answer: short lines in the author's sense, numbers exactly as given, \
nothing the answer does not say (the paragraph's other points stay as they are; do not repeat them). `conclusion` = the \
answer in one short line for the margin. If the answer means no, not yet, or it did not happen, record exactly that as the \
point and the conclusion. `gap` = true only when the answer itself opened something the paragraph now needs and does not \
have (a new claim without its reason or number); then `follow_up` = one question (kind missing or choice, 2–3 options, \
addressed to 「你」, no ids, not a rewording of this question), else null. Never a follow-up after no / not yet. \
`writing_notes` = only when the answer says how to WRITE something or what to avoid, in the author's words, `scope` = \
paragraph, section or paper, `banned` = words they forbid; usually [].\n{TW} Reply with JSON only."
    );
    let anchor = sk.para(&t.anchor).or_else(|| sk.block(&t.anchor).and_then(|b| b.paras.first()));
    let mut user = format!("Key value of the paper: {}\n", key_value.trim());
    if let (Some(p), Some(b)) = (anchor, anchor.and_then(|p| sk.block_of(&p.id))) {
        user += &format!("Block \"{}\": {}\nParagraph: {}\n", b.title, b.purpose, p.say);
        if !p.points.is_empty() {
            user += "Its points so far:\n";
            for x in &p.points {
                user += &format!("- {}\n", x.text);
            }
        }
    }
    if !t.question.is_empty() {
        user += &format!("\nQuestion: {}\n", t.question);
    }
    for m in t.messages.iter().filter(|m| m.who == "me") {
        user += &format!("The author: {}\n", m.text);
    }
    if !record.trim().is_empty() {
        user += "\n## The record (to check numbers against)\n";
        user += record;
    }
    Prompt { system, user }
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct FollowUp {
    #[serde(default)]
    pub kind: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct FillReply {
    #[serde(default)]
    pub points: Vec<String>,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub gap: bool,
    #[serde(default)]
    pub follow_up: Option<FollowUp>,
    #[serde(default)]
    pub writing_notes: Vec<crate::draft::notes::NoteUpdate>,
}

pub fn parse_fill(reply: &str) -> Result<FillReply, String> {
    json_in(reply)
}

/// The author's answer means no / not yet / it did not happen: recorded as said, never asked again.
pub fn is_negative(answer: &str) -> bool {
    let a = answer.trim().to_lowercase();
    let starts = ["沒有", "還沒", "沒做", "不是", "不用", "不需要", "無", "否", "不知道", "不確定", "no", "not yet", "none", "n/a", "nope"];
    starts.iter().any(|s| a.starts_with(s)) || ["還沒有", "沒有做", "沒發生", "沒有發生", "not yet", "did not happen", "didn't happen"].iter().any(|s| a.contains(s))
}

/// The fill-in lands: the paragraph found by id where it is now (moved meanwhile is fine;
/// removed = the points are dropped, the thread still closes); its points from this thread are
/// replaced, others stay. A follow-up only when the answer opened a real gap, never after a "no",
/// never a follow-up of a follow-up; it comes after the question shown now. Returns the follow-up's id.
pub fn apply_fill(sk: &mut Skeleton, thread: &str, r: FillReply, ids: &BTreeSet<String>) -> Option<String> {
    let t = sk.thread(thread)?.clone();
    let answer: String = t.messages.iter().filter(|m| m.who == "me").map(|m| m.text.clone()).collect::<Vec<_>>().join("\n");
    let negative = is_negative(&answer);
    let target = if sk.orphaned.contains(&t.id) {
        None
    } else if sk.para(&t.anchor).is_some() {
        Some(t.anchor.clone())
    } else {
        sk.block(&t.anchor).and_then(|b| b.paras.first()).map(|p| p.id.clone())
    };
    sk.orphaned.retain(|o| o != thread);
    let mut points: Vec<String> = r.points.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    let conclusion = if r.conclusion.trim().is_empty() { answer.lines().next().unwrap_or("").trim().to_string() } else { r.conclusion.trim().to_string() };
    if points.is_empty() && negative {
        points.push(conclusion.clone());
    }
    if let Some(p) = target.as_deref().and_then(|id| sk.para_mut(id)) {
        p.points.retain(|x| x.from.as_deref() != Some(thread));
        p.points.extend(points.into_iter().map(|text| Point { text, from: Some(thread.to_string()) }));
        if p.busy.as_deref() == Some("fill") {
            p.busy = None;
        }
        p.settle_state();
    }
    if let Some(x) = sk.thread_mut(thread) {
        x.status = "done".into();
        x.conclusion = conclusion;
    }
    let follow = r.follow_up.filter(|f| r.gap && !negative && !f.question.trim().is_empty() && !sk.follow_of.contains_key(thread) && target.is_some())?;
    let kind = if QUESTION_KINDS.contains(&f_kind(&follow)) { f_kind(&follow).to_string() } else { "missing".into() };
    let text = scrub(&follow.question, ids);
    if text.is_empty() || text == t.question {
        return None;
    }
    let opts: Vec<String> = follow.options.iter().map(|o| scrub(o, ids)).filter(|o| !o.is_empty()).take(3).collect();
    let new = Thread { id: String::new(), anchor: target.unwrap_or_default(), kind, status: "open".into(), question: text.clone(), options: options(opts), messages: vec![Msg { who: "ai".into(), text }], conclusion: String::new(), changes: vec![] };
    let id = sk.add_thread(new);
    // asked soon: after the question the page shows now (the first open one after the thread it
    // follows), never in its place
    if let (Some(from), Some(at)) = (sk.threads.iter().position(|x| x.id == thread), sk.threads.iter().position(|x| x.id == id)) {
        let t = sk.threads.remove(at);
        let shown = sk.threads.iter().enumerate().skip(from + 1).find(|(_, x)| x.status == "open" && !x.question.is_empty()).map(|(k, _)| k + 1);
        let to = shown.unwrap_or(sk.threads.len()).min(sk.threads.len());
        sk.threads.insert(to, t);
    }
    sk.follow_of.insert(id.clone(), thread.to_string());
    Some(id)
}

fn f_kind(f: &FollowUp) -> &str {
    f.kind.trim()
}
