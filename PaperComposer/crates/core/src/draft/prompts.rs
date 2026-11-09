//! Prompts, reply schemas and reply parsing for each step of the draft
//! interview. Everything the author cares about comes from the lens and the
//! style folder (their private repository); these prompts only say how to
//! use them.
//!
//! Text meant for the author (questions, summaries, outline) is Traditional
//! Chinese; text meant for the paper (claims, outline points, drafts) is
//! English.

use super::lens::Lens;
use super::session::*;
use crate::compose::{json_array, json_object, json_string, json_string_or_null, Prompt};
use serde::Deserialize;
use serde_json::{json, Value};

fn json_int_or_null() -> Value {
    json!({ "anyOf": [{ "type": "integer" }, { "type": "null" }] })
}
fn json_bool() -> Value {
    json!({ "type": "boolean" })
}
fn json_int() -> Value {
    json!({ "type": "integer" })
}
fn strings() -> Value {
    json_array(json_string())
}

/// The JSON object in a reply (code fences and chatter around it are fine).
pub fn json_in<T: for<'de> Deserialize<'de>>(reply: &str) -> Result<T, String> {
    let start = reply.find('{').ok_or("回覆裡沒有 JSON")?;
    let end = reply.rfind('}').ok_or("回覆裡沒有 JSON")?;
    serde_json::from_str(&reply[start..=end]).map_err(|e| format!("看不懂回覆：{e}"))
}

const TW: &str = "Write every text meant for the author in Traditional Chinese as used in Taiwan (繁體中文, never simplified).";

/// Shared context: key value, hook, experiments and timeline, compact.
pub fn context(key_value: &str, s: &Session) -> String {
    let mut c = format!("Key value (the one thing the paper argues): {}\n", key_value.trim());
    if !s.hook.reader_accepts.trim().is_empty() || !s.hook.twist.trim().is_empty() {
        c += &format!("Interesting hypothesis — readers already accept: {} / the twist: {}\n", s.hook.reader_accepts.trim(), s.hook.twist.trim());
    }
    if !s.record_summary.trim().is_empty() {
        c += &format!("\nExperiment record summary: {}\n", s.record_summary.trim());
    }
    if !s.experiments.is_empty() {
        c += "\nExperiments:\n";
        for e in &s.experiments {
            c += &format!(
                "- {} {}: changed {}; metrics {}; result {}; purpose {}{}{}\n",
                e.id,
                e.title,
                e.change,
                e.metrics,
                e.result,
                e.purpose,
                if e.purpose_sure { "" } else { " (UNSURE: confirm with the author)" },
                e.mode.as_deref().map(|m| format!("; mode guess {m}")).unwrap_or_default()
            );
        }
    }
    if !s.timeline.is_empty() {
        c += "\nTimeline (what was tried, in order):\n";
        for t in &s.timeline {
            c += &format!("- T{} {} → observed: {} → changed: {}\n", t.step, t.what, t.observed, t.changed);
        }
    }
    c
}

// ── Step 2: experiment record ─────────────────────────────

pub fn record_schema() -> Value {
    let exp = json_object(&[
        ("id", json_string()),
        ("title", json_string()),
        ("change", json_string()),
        ("metrics", json_string()),
        ("result", json_string()),
        ("purpose", json_string()),
        ("purpose_sure", json_bool()),
        ("mode", json_string_or_null()),
    ]);
    let step = json_object(&[("step", json_int()), ("what", json_string()), ("observed", json_string()), ("changed", json_string())]);
    json_object(&[("summary", json_string()), ("experiments", json_array(exp)), ("timeline", json_array(step))])
}

pub fn record_prompt(key_value: &str, lens: &Lens, records: &str) -> Prompt {
    let modes: Vec<String> = lens.modes.iter().map(|m| format!("{} ({})", m.id, m.means)).collect();
    let system = format!(
        "You read a researcher's raw experiment record and organise it for a paper.\n\
Return:\n\
1. `summary`: 2–4 sentences on what the study did.\n\
2. `experiments`: one entry per distinct experiment or comparison (id E1, E2, …). `change` = what was varied or \
compared; `metrics` and `result` = the numbers as recorded (copy them, never invent or round); `purpose` = what \
the experiment was for, as you read it; `purpose_sure` = false whenever the record does not say it plainly — the \
author will be asked. `mode` = your guess among these modes: {}; null if you cannot tell.\n\
3. `timeline`: the order in which things were tried, as far as the record shows (dates, run numbers, versions). \
For each step: what was tried, what was observed, what was changed next. This is where the author's thinking \
shows (a parameter that did not fit, a baseline that failed); keep those details, they matter more than tidy \
summaries. Empty if the record shows no order.\n\
Do not invent experiments, numbers or reasons. {TW} Reply with JSON only.",
        modes.join("; ")
    );
    let user = format!("Key value: {}\n\nExperiment record:\n{}", key_value.trim(), records);
    Prompt { system, user }
}

#[derive(Debug, Deserialize)]
pub struct RecordReply {
    pub summary: String,
    pub experiments: Vec<Experiment>,
    pub timeline: Vec<TimelineStep>,
}

pub fn parse_record(reply: &str, lens: &Lens) -> Result<RecordReply, String> {
    let mut r: RecordReply = json_in(reply)?;
    for e in &mut r.experiments {
        if e.mode.as_deref().is_some_and(|m| lens.mode(m).is_none()) {
            e.mode = None;
        }
    }
    r.experiments.retain(|e| !e.title.trim().is_empty());
    Ok(r)
}

// ── Step 3: question groups ──────────────────────────────

pub fn plan_schema() -> Value {
    let item = json_object(&[("label", json_string()), ("role", json_string()), ("mode", json_string_or_null())]);
    let group = json_object(&[("section", json_string()), ("title", json_string()), ("purpose", json_string()), ("items", json_array(item))]);
    json_object(&[("groups", json_array(group))])
}

pub fn plan_prompt(key_value: &str, s: &Session, lens: &Lens, style: &str) -> Prompt {
    let roles = lens.targets.iter().map(|t| t.role.as_str()).collect::<Vec<_>>().join(", ");
    let system = format!(
        "You plan the question groups for an interview that will turn a study into a paper draft.\n\
Sections, in order: {}. Each group is one unit of the paper's argument (usually what becomes one subsection or a \
few paragraphs) and must end with three things: an explanation, a motive and a conclusion.\n\
Rules:\n\
1. Introduction: centre the first group on how the authors came to the idea (the observation that triggered it, \
what readers already accept, the twist), then the gap and the contribution.\n\
2. Methods: one group per key step of the pipeline as the record shows it; the number of groups is your estimate \
of the subsections, not final.\n\
3. Results: one group per message the results must carry (what the main table shows, which comparison matters), \
built from the experiments.\n\
4. Discussion and Conclusion: one or two groups each.\n\
5. `items` = what must be clarified in the group: decisions, experiments, findings. Give each a `role` (use these \
when they fit: {roles}; otherwise design, experiment, result, background) and a `mode` guess from the lens or null.\n\
6. Follow the author's structure habits below when given. Do not invent results.\n\
{TW} Titles and purposes are for the author. Reply with JSON only.",
        SECTIONS.join(", ")
    );
    let mut user = context(key_value, s);
    user += "\n";
    user += &lens.describe();
    if !style.trim().is_empty() {
        user += "\n## The author's writing style and structure habits\n";
        user += style;
    }
    Prompt { system, user }
}

#[derive(Debug, Deserialize)]
struct PlanReply {
    groups: Vec<PlanGroup>,
}
#[derive(Debug, Deserialize)]
struct PlanGroup {
    section: String,
    title: String,
    purpose: String,
    items: Vec<PlanItem>,
}
#[derive(Debug, Deserialize)]
struct PlanItem {
    label: String,
    role: String,
    mode: Option<String>,
}

/// Groups in section order; unknown sections are dropped.
pub fn parse_plan(reply: &str, lens: &Lens) -> Result<Vec<(Group, Vec<Item>)>, String> {
    let r: PlanReply = json_in(reply)?;
    let mut out: Vec<(usize, Group, Vec<Item>)> = r
        .groups
        .into_iter()
        .filter_map(|g| {
            let sec = SECTIONS.iter().position(|s| s.eq_ignore_ascii_case(g.section.trim()))?;
            let items = g
                .items
                .into_iter()
                .filter(|i| !i.label.trim().is_empty())
                .map(|i| Item { label: i.label, role: i.role, mode: i.mode.filter(|m| lens.mode(m).is_some()), ..Default::default() })
                .collect();
            Some((sec, Group { section: SECTIONS[sec].into(), title: g.title, purpose: g.purpose, ..Default::default() }, items))
        })
        .collect();
    out.sort_by_key(|(s, _, _)| *s);
    if out.is_empty() {
        return Err("沒有規劃出任何題組".into());
    }
    Ok(out.into_iter().map(|(_, g, i)| (g, i)).collect())
}

// ── Step 4: one interview turn ───────────────────────────

pub fn turn_schema() -> Value {
    let upd = json_object(&[
        ("id", json_string_or_null()),
        ("label", json_string_or_null()),
        ("role", json_string_or_null()),
        ("mode", json_string_or_null()),
        ("depth", json_int_or_null()),
        ("flags", strings()),
        ("number", json_string_or_null()),
        ("stance", json_string_or_null()),
        ("evidence", strings()),
    ]);
    let summary = json_object(&[
        ("explain", json_string()),
        ("motive", json_string()),
        ("conclusion", json_string()),
        ("reader_accepts", json_string()),
        ("twist", json_string()),
    ]);
    json_object(&[
        ("updates", json_array(upd)),
        ("summary", summary),
        ("reply", json_string()),
        ("question", json_string()),
        ("options", strings()),
    ])
}

const TURN_SYSTEM: &str = "You interview a researcher about their own study so that a paper draft can be written in their \
voice. Readers read papers to see the author's mind: how they came up with the idea, what they were thinking while \
doing it, how they read their numbers. Your job is to get that out of them, one question at a time, and to stop \
when a group has what it needs.

Each turn:
1. Grade the latest answer into `updates` for the group's items (by `id`; a new item gets `id` null and a `label`).
   - Judge by what the answer MEANS, never by its words. A plain fact stated to justify something is reasoning,
     graded at the level of the reasoning it serves. A bare fact or a number is the lowest level.
   - `mode`: sort the item into one of the lens's modes as soon as the answer allows. An answer like \"BCE for
     two classes, we do not plan to try others\" settles it as routine practice: no further why.
   - `depth`: the lens scale index the author's reasoning on this item has reached so far (keep the highest).
   - `flags`: lens flags the answer earns (e.g. how the idea came about).
   - `number`: the number that backs a result, from the answer or the record.
   - `stance`: the author's own view on the item, condensed, in their terms (Traditional Chinese).
   - `evidence`: \"turn N\", experiment ids (E2) or timeline steps (T3) it rests on.
   Leave out items the answer did not touch.
2. Rewrite `summary` for the group (explanation, motive, conclusion; what readers accept and the twist) from
   everything said so far. Leave a field empty rather than guess.
3. `reply`: one short sentence that shows you understood (no praise, no repetition of the whole answer).
4. `question`: the single next question. Choose it from the open items below — the code has worked out what each
   lacks and which probes fit; rephrase probes naturally and concretely, using the record and timeline (\"In run 3
   you moved the centre away from the mean — what did you see?\" beats \"why?\"). Never ask about items in a mode
   that is never asked; for routine items only confirm. If nothing is open, ask whether anything is missing or
   wrap up, and leave `question` empty when the group is clearly complete.
5. `options`: when the author said they are unsure, or the question invites a choice, 2–4 short candidate answers
   (Traditional Chinese) built from the record; otherwise empty.
Never invent facts about the study. Ask one thing at a time.";

pub fn turn_prompt(key_value: &str, s: &Session, lens: &Lens, group: &str, turns: &[Turn], answer: Option<&str>) -> Prompt {
    let system = format!("{TURN_SYSTEM}\n{TW} Reply with JSON only.");
    let mut user = context(key_value, s);
    user += "\n";
    user += &lens.describe();
    let done: Vec<&Group> = s.groups.iter().filter(|g| g.status == GroupStatus::Done && g.id != group).collect();
    if !done.is_empty() {
        user += "\n## Groups already settled\n";
        for g in done {
            user += &format!("- {} [{}] {}: {} / {}\n", g.id, g.section, g.title, g.summary.motive, g.summary.conclusion);
        }
    }
    if let Some(g) = s.group(group) {
        user += &format!("\n## Current group {} [{}] {}\nPurpose: {}\n", g.id, g.section, g.title, g.purpose);
        let gs = s.group_state(lens, group);
        user += "Items (what each still lacks, decided by code from the lens):\n";
        for it in s.items_of(group) {
            let st = gs.items.iter().find(|x| x.id == it.id);
            let missing = st.map(|x| &x.missing[..]).unwrap_or(&[]);
            let need = if missing.is_empty() {
                "done".to_string()
            } else {
                missing
                    .iter()
                    .map(|m| match m {
                        super::lens::Missing::Mode => "mode not yet known".to_string(),
                        super::lens::Missing::Depth(d) => format!("needs depth {}", lens.level_name(*d)),
                        super::lens::Missing::Flag(f) => format!("needs flag {f}"),
                        super::lens::Missing::Number => "needs its number".to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            user += &format!(
                "- {} \"{}\" role={} mode={} depth={} flags=[{}] stance=\"{}\" → {need}",
                it.id,
                it.label,
                it.role,
                it.mode.as_deref().unwrap_or("?"),
                it.depth.map(|d| lens.level_name(d)).unwrap_or_else(|| "?".into()),
                it.flags.join(","),
                it.stance
            );
            if let Some(p) = st.and_then(|x| x.preferred) {
                user += &format!(" (optional: {} welcome)", lens.level_name(p));
            }
            let probes = lens.probes_for(&it.role, missing);
            if !probes.is_empty() {
                user += &format!(" probes: {}", probes.iter().map(|p| format!("\"{}\"", p.ask)).collect::<Vec<_>>().join(" | "));
            }
            user += "\n";
        }
        user += &format!(
            "Summary so far: explain=\"{}\" motive=\"{}\" conclusion=\"{}\" accepts=\"{}\" twist=\"{}\"\n",
            g.summary.explain, g.summary.motive, g.summary.conclusion, g.summary.reader_accepts, g.summary.twist
        );
        user += &format!("Answers so far in this group: {} of {}\n", g.turns, lens.stop.max_turns_per_group);
    }
    let here: Vec<&Turn> = turns.iter().filter(|t| t.group == group).collect();
    if !here.is_empty() {
        user += "\n## Conversation in this group\n";
        for t in here {
            let who = if t.who == Who::Ai { "Q" } else { "A" };
            user += &format!("[turn {}] {who}: {}\n", t.n, t.text);
        }
    }
    match answer {
        Some(a) => user += &format!("\n## Latest answer\n{a}\n"),
        None => user += "\n## No answer yet: open the group with its first question.\n",
    }
    Prompt { system, user }
}

#[derive(Debug, Deserialize)]
pub struct TurnReply {
    pub updates: Vec<ItemUpdate>,
    pub summary: GroupSummary,
    pub reply: String,
    pub question: String,
    pub options: Vec<String>,
}

pub fn parse_turn(reply: &str) -> Result<TurnReply, String> {
    let mut r: TurnReply = json_in(reply)?;
    r.options.retain(|o| !o.trim().is_empty());
    r.options.truncate(4);
    Ok(r)
}

// ── Step 4.5: claim register ─────────────────────────────

pub fn claims_schema() -> Value {
    let c = json_object(&[
        ("zh", json_string()),
        ("en", json_string()),
        ("depth", json_int_or_null()),
        ("evidence", json_string()),
        ("reader_accepts", json_string()),
        ("twist", json_string()),
        ("home", json_string()),
        ("items", strings()),
    ]);
    json_object(&[("claims", json_array(c))])
}

/// Groups and the ledger as text, for the steps after the interview.
pub fn ledger_text(s: &Session, lens: &Lens) -> String {
    let mut t = String::new();
    for g in s.groups.iter().filter(|g| g.status != GroupStatus::Skipped) {
        t += &format!(
            "\n### {} [{}] {}\nexplain: {}\nmotive: {}\nconclusion: {}\nreaders accept: {}\ntwist: {}\n",
            g.id, g.section, g.title, g.summary.explain, g.summary.motive, g.summary.conclusion, g.summary.reader_accepts, g.summary.twist
        );
        for it in s.items_of(&g.id) {
            t += &format!(
                "- {} {} (role {}, mode {}, depth {}, flags [{}]{}) stance: {}\n",
                it.id,
                it.label,
                it.role,
                it.mode.as_deref().unwrap_or("?"),
                it.depth.map(|d| lens.level_name(d)).unwrap_or_else(|| "?".into()),
                it.flags.join(","),
                it.number.as_deref().map(|n| format!(", number {n}")).unwrap_or_default(),
                it.stance
            );
        }
    }
    t
}

pub fn claims_prompt(key_value: &str, s: &Session, lens: &Lens) -> Prompt {
    let system = format!(
        "You build the claim register of a paper from an interview's ledger. A claim is something the paper asserts \
and must earn. Each claim appears once: `home` is the section where it is said in full (Introduction, Methods, \
Results, Discussion or Conclusion); everywhere else the draft will only call it back in a few words. Merge items \
that make the same claim — repeating the main line in many places is a known weakness of this author.\n\
`zh` for the author, `en` one crisp English sentence for the paper, `depth` = lens scale index of the reasoning \
behind it, `evidence` = what backs it (table, number, experiment id), `reader_accepts` / `twist` = what makes it \
interesting (empty when it is a plain fact), `items` = ledger ids. Order claims from the strongest down. Only \
claims the ledger supports. {TW} Reply with JSON only."
    );
    let mut user = context(key_value, s);
    user += "\n";
    user += &lens.describe();
    user += "\n## Ledger\n";
    user += &ledger_text(s, lens);
    Prompt { system, user }
}

#[derive(Debug, Deserialize)]
struct ClaimsReply {
    claims: Vec<Claim>,
}

pub fn parse_claims(reply: &str) -> Result<Vec<Claim>, String> {
    let r: ClaimsReply = json_in(reply)?;
    Ok(r.claims.into_iter().filter(|c| !c.zh.trim().is_empty()).enumerate().map(|(i, c)| Claim { id: format!("c{}", i + 1), ..c }).collect())
}

// ── Step 5: order and narrative check ────────────────────

pub fn review_schema() -> Value {
    let link = json_object(&[("group", json_string()), ("conclusion", json_string()), ("follows", json_bool()), ("note", json_string())]);
    let sug = json_object(&[("kind", json_string()), ("groups", strings()), ("detail", json_string()), ("reason", json_string())]);
    json_object(&[("chain", json_array(link)), ("suggestions", json_array(sug))])
}

pub fn review_prompt(key_value: &str, s: &Session, lens: &Lens) -> Prompt {
    let system = format!(
        "You check the order of a paper's argument before it is outlined. Go through the groups in order. For each, \
state in `chain` the conclusion a reader holds after it, and whether it follows from the previous one (`follows`; \
the first group of a section follows when the section opens naturally). Then propose `suggestions`: kind = move \
(reorder), split (one group carries too much), merge (two groups carry too little), resize, table (results that \
belong in one table), or other; `groups` = ids involved; `detail` = what to do; `reason` = why the narrative gets \
better. Also flag a claim stated in full in more than one place. Only suggest what clearly helps. {TW} Reply with \
JSON only."
    );
    let mut user = format!("Key value: {}\n", key_value.trim());
    user += &ledger_text(s, lens);
    if !s.claims.is_empty() {
        user += "\n## Claims\n";
        for c in &s.claims {
            user += &format!("- {} (home {}): {}\n", c.id, c.home, c.zh);
        }
    }
    Prompt { system, user }
}

#[derive(Debug, Deserialize)]
struct ReviewReply {
    chain: Vec<ChainLink>,
    suggestions: Vec<RawSuggestion>,
}
#[derive(Debug, Deserialize)]
struct RawSuggestion {
    kind: String,
    groups: Vec<String>,
    detail: String,
    reason: String,
}

pub fn parse_review(reply: &str) -> Result<Review, String> {
    let r: ReviewReply = json_in(reply)?;
    Ok(Review {
        chain: r.chain,
        suggestions: r
            .suggestions
            .into_iter()
            .filter(|x| !x.detail.trim().is_empty())
            .enumerate()
            .map(|(i, x)| Suggestion { id: format!("s{}", i + 1), kind: x.kind, groups: x.groups, detail: x.detail, reason: x.reason, accepted: None })
            .collect(),
    })
}

// ── Step 6: paragraph-level outline ──────────────────────

pub fn outline_schema() -> Value {
    let para = json_object(&[("kind", json_string()), ("plan", json_string()), ("zh", json_string()), ("en", strings()), ("sources", strings())]);
    let sec = json_object(&[("section", json_string()), ("subsection", json_string_or_null()), ("paras", json_array(para))]);
    json_object(&[("sections", json_array(sec))])
}

const OUTLINE_RULES: &str = "Rules:\n\
1. Outline paragraph by paragraph. Each paragraph gets `kind` (a paragraph type from the author's style when given, \
e.g. \"Methods custom decision\", \"Results key result\", \"exposition\"), `plan` (its units in order, e.g. \
\"fact → L2 reason → consequence\" or \"table pointer → numbers → L1 reading → L2 why\"), `zh` (what it says, for \
the author), `en` (the same as short English points, one per line; these drive phrase retrieval), `sources` \
(group, item and claim ids).\n\
2. Each claim is said in full only in its home section; elsewhere call it back briefly.\n\
3. Exposition paragraphs (definitions, variables, procedure) need no motive; do not force opinions into them. Keep \
the author's balance of reasoning — a few paragraphs carry the thinking, the rest carry the reader there.\n\
4. Put the story of how the idea came about in the Introduction; keep local operational insights (a parameter that \
did not fit and was changed) in the Methods paragraph where they happened.\n\
5. Results: every judgement comes with its number; the key results get an explanation, not only higher/lower.\n\
6. Conclusion: carry the strongest claims with the main numbers and one honest limitation of the method itself.\n\
7. Follow the author's style notes below (paragraph types, proportions, habits to keep and habits to fix).\n\
8. Do not invent results, numbers or citations.";

pub fn outline_prompt(key_value: &str, s: &Session, lens: &Lens, style: &str, only: Option<(&str, &str)>) -> Prompt {
    let system = format!("You outline a paper from its interview ledger, claim register and review.\n{OUTLINE_RULES}\n{TW} Reply with JSON only.");
    let mut user = context(key_value, s);
    user += "\n";
    user += &lens.describe();
    user += "\n## Ledger\n";
    user += &ledger_text(s, lens);
    if !s.claims.is_empty() {
        user += "\n## Claims (strongest first)\n";
        for c in &s.claims {
            user += &format!("- {} home={} depth={:?}: {} | {} | evidence: {}\n", c.id, c.home, c.depth, c.zh, c.en, c.evidence);
        }
    }
    if let Some(r) = &s.review {
        let accepted: Vec<&Suggestion> = r.suggestions.iter().filter(|x| x.accepted == Some(true)).collect();
        if !accepted.is_empty() {
            user += "\n## Accepted changes from the review\n";
            for x in accepted {
                user += &format!("- {} {:?}: {}\n", x.kind, x.groups, x.detail);
            }
        }
    }
    if !style.trim().is_empty() {
        user += "\n## The author's style\n";
        user += style;
    }
    match only {
        Some((section, comment)) => {
            let current = s.outline.iter().filter(|o| o.section == section).map(|o| serde_json::to_string(o).unwrap_or_default()).collect::<Vec<_>>().join("\n");
            user += &format!(
                "\n## Revise only the section {section}\nCurrent outline of it:\n{current}\nThe author's comment: {comment}\nReturn only this section (it may hold several subsections)."
            );
        }
        None => user += &format!("\nOutline all sections: {}.", SECTIONS.join(", ")),
    }
    Prompt { system, user }
}

#[derive(Debug, Deserialize)]
struct OutlineReply {
    sections: Vec<RawSection>,
}
#[derive(Debug, Deserialize)]
struct RawSection {
    section: String,
    subsection: Option<String>,
    paras: Vec<RawPara>,
}
#[derive(Debug, Deserialize)]
struct RawPara {
    kind: String,
    plan: String,
    zh: String,
    en: Vec<String>,
    sources: Vec<String>,
}

/// Sections in paper order, paragraph ids p1, p2, … across the outline.
pub fn parse_outline(reply: &str) -> Result<Vec<OutlineSection>, String> {
    let r: OutlineReply = json_in(reply)?;
    let mut secs: Vec<(usize, OutlineSection)> = r
        .sections
        .into_iter()
        .filter_map(|x| {
            let i = SECTIONS.iter().position(|s| s.eq_ignore_ascii_case(x.section.trim()))?;
            let paras = x
                .paras
                .into_iter()
                .filter(|p| !p.zh.trim().is_empty())
                .map(|p| OutlinePara { id: String::new(), kind: p.kind, plan: p.plan, zh: p.zh, en: p.en.into_iter().filter(|e| !e.trim().is_empty()).collect(), sources: p.sources })
                .collect();
            Some((i, OutlineSection { section: SECTIONS[i].into(), subsection: x.subsection.filter(|s| !s.trim().is_empty()), paras, confirmed: false }))
        })
        .collect();
    secs.sort_by_key(|(i, _)| *i);
    if secs.is_empty() {
        return Err("大綱是空的".into());
    }
    Ok(secs.into_iter().map(|(_, s)| s).collect())
}

/// Give every paragraph a unique id (p1, p2, …), keeping ids already set.
pub fn number_paras(outline: &mut [OutlineSection]) {
    let mut n = outline.iter().flat_map(|s| &s.paras).filter_map(|p| p.id.strip_prefix('p')?.parse::<usize>().ok()).max().unwrap_or(0);
    for p in outline.iter_mut().flat_map(|s| s.paras.iter_mut()) {
        if p.id.is_empty() {
            n += 1;
            p.id = format!("p{n}");
        }
    }
}

/// Replace one section's entries (all its subsections) with a revision.
pub fn replace_section(outline: &mut Vec<OutlineSection>, section: &str, new: Vec<OutlineSection>) {
    let at = outline.iter().position(|o| o.section == section).unwrap_or(outline.len());
    outline.retain(|o| o.section != section);
    let new: Vec<OutlineSection> = new.into_iter().filter(|o| o.section == section).collect();
    let at = at.min(outline.len());
    outline.splice(at..at, new);
    number_paras(outline);
}

/// Extra instructions appended to the phrase-bank compose prompt for one
/// outline paragraph: its unit plan, the claims it may state in full, and the
/// author's style notes.
pub fn compose_extra(para: &OutlinePara, s: &Session, section: &str, style: &str) -> String {
    let mut x = format!(
        "\nThis paragraph is part of a draft built from an interview with the author.\nParagraph type: {}\nUnit plan, in order: {}\nWhat it must say (author's words): {}\n",
        para.kind, para.plan, para.zh
    );
    let full: Vec<&Claim> = s.claims.iter().filter(|c| c.home == section && para.sources.iter().any(|src| src == &c.id)).collect();
    let back: Vec<&Claim> = s.claims.iter().filter(|c| c.home != section && para.sources.iter().any(|src| src == &c.id)).collect();
    if !full.is_empty() {
        x += "State these claims in full here:\n";
        for c in full {
            x += &format!("- {}\n", c.en);
        }
    }
    if !back.is_empty() {
        x += "Only call these back in a few words (they are stated in full elsewhere):\n";
        for c in back {
            x += &format!("- {}\n", c.en);
        }
    }
    if !style.trim().is_empty() {
        x += "\nThe author's style notes (follow them; never copy example sentences):\n";
        x += style;
        x += "\n";
    }
    x
}

// ── Step 8: reviewer pass ────────────────────────────────

pub fn audit_schema() -> Value {
    let f = json_object(&[("kind", json_string()), ("para", json_string()), ("quote", json_string()), ("why", json_string()), ("ask", json_string())]);
    json_object(&[("findings", json_array(f))])
}

pub fn audit_prompt(key_value: &str, s: &Session, lens: &Lens, style: &str) -> Prompt {
    let system = format!(
        "You are a demanding but fair reviewer reading a paper draft against the study's own record. Find:\n\
- contradiction: two places that cannot both be true (including Introduction stance vs Discussion), or text vs record;\n\
- overclaim: a claim stronger than its evidence (compare effect sizes with spread and sample size; \"drives the \
improvement\" on a gap inside one standard deviation is an overclaim);\n\
- missing-number: a result judged (better, significant, best) without its number;\n\
- impossible: numbers that cannot hold together (e.g. IoU above Dice for the same masks);\n\
- generic: a sentence, often a closing, that would stay true if moved into another paper;\n\
- repeat: a claim stated in full in more than one place;\n\
- buried-origin: how the idea came about is told late or as after-the-fact support;\n\
- missing-why: an own design decision written as procedure with no reason.\n\
Exposition paragraphs need no motive — do not flag them for lacking one. For each finding give `para` (paragraph \
id), `quote` (the exact words), `why`, and `ask`: a question for the author when the fix needs their knowledge, \
else empty. Report only real problems, most serious first. {TW} Reply with JSON only."
    );
    let mut user = context(key_value, s);
    user += "\n";
    user += &lens.describe();
    if !s.claims.is_empty() {
        user += "\n## Claims\n";
        for c in &s.claims {
            user += &format!("- {} home={}: {} | evidence: {}\n", c.id, c.home, c.en, c.evidence);
        }
    }
    if !style.trim().is_empty() {
        user += "\n## The author's style notes (habits to fix are worth flagging)\n";
        user += style;
    }
    user += "\n## Draft\n";
    for d in &s.drafts {
        user += &format!("\n### {}\n", d.section);
        for p in &d.paras {
            user += &format!("[{}] {}\n", p.para, p.text);
        }
    }
    Prompt { system, user }
}

#[derive(Debug, Deserialize)]
struct AuditReply {
    findings: Vec<RawFinding>,
}
#[derive(Debug, Deserialize)]
struct RawFinding {
    kind: String,
    para: String,
    quote: String,
    why: String,
    ask: String,
}

pub fn parse_audit(reply: &str) -> Result<Vec<Finding>, String> {
    let r: AuditReply = json_in(reply)?;
    Ok(r.findings
        .into_iter()
        .filter(|f| !f.why.trim().is_empty())
        .enumerate()
        .map(|(i, f)| Finding { id: format!("f{}", i + 1), kind: f.kind, para: f.para, quote: f.quote, why: f.why, ask: f.ask, resolved: false })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens() -> Lens {
        Lens::builtin()
    }

    #[test]
    fn schemas_are_strict_objects() {
        for s in [record_schema(), plan_schema(), turn_schema(), claims_schema(), review_schema(), outline_schema(), audit_schema()] {
            assert_eq!(s["type"], "object");
            assert_eq!(s["additionalProperties"], false);
        }
    }

    #[test]
    fn parses_a_plan_in_section_order() {
        let reply = r#"```json
{"groups":[
 {"section":"Results","title":"主表","purpose":"p","items":[{"label":"Dice","role":"key-result","mode":"own"}]},
 {"section":"Introduction","title":"發想","purpose":"p","items":[{"label":"idea","role":"core-contribution","mode":"vibes"}]},
 {"section":"Appendix","title":"x","purpose":"p","items":[]}
]}```"#;
        let g = parse_plan(reply, &lens()).unwrap();
        assert_eq!(g.iter().map(|(g, _)| g.section.as_str()).collect::<Vec<_>>(), ["Introduction", "Results"]);
        assert_eq!(g[0].1[0].mode, None, "a mode the lens does not have is dropped");
        assert_eq!(g[1].1[0].mode.as_deref(), Some("own"));
    }

    #[test]
    fn turn_prompt_carries_what_is_missing_and_the_probes() {
        let l = lens();
        let mut s = Session::default();
        s.set_plan(vec![(
            Group { section: "Methods".into(), title: "Loss".into(), ..Default::default() },
            vec![Item { label: "BCE".into(), role: "design".into(), ..Default::default() }],
        )]);
        let p = turn_prompt("Make uncertain frames count", &s, &l, "g1", &[], Some("用 BCE"));
        assert!(p.user.contains("mode not yet known"));
        assert!(p.user.contains("這是領域常規做法"), "the lens's probe is offered");
        assert!(p.user.contains("用 BCE"));
        let r = parse_turn(r#"{"updates":[{"id":"i1","label":null,"role":null,"mode":"routine","depth":null,"flags":[],"number":null,"stance":"二分類常規","evidence":["turn 2"]}],
          "summary":{"explain":"","motive":"","conclusion":"","reader_accepts":"","twist":""},"reply":"了解","question":"","options":["a","b","c","d","e"]}"#)
            .unwrap();
        assert_eq!(r.options.len(), 4);
        s.apply(&l, "g1", r.updates);
        assert!(s.group_state(&l, "g1").items[0].missing.is_empty());
    }

    #[test]
    fn outline_numbers_paragraphs_and_revises_one_section() {
        let reply = r#"{"sections":[
 {"section":"Methods","subsection":"Loss","paras":[{"kind":"exposition","plan":"fact","zh":"定義","en":["define the loss"],"sources":[]}]},
 {"section":"Introduction","subsection":null,"paras":[{"kind":"Intro turn","plan":"fact → L4","zh":"轉折","en":["a","",""],"sources":["c1"]}]}
]}"#;
        let mut o = parse_outline(reply).unwrap();
        number_paras(&mut o);
        assert_eq!(o[0].section, "Introduction");
        assert_eq!(o[0].paras[0].en, ["a"]);
        assert_eq!((o[0].paras[0].id.as_str(), o[1].paras[0].id.as_str()), ("p1", "p2"));
        let rev = parse_outline(r#"{"sections":[{"section":"Methods","subsection":"Loss","paras":[{"kind":"Methods custom decision","plan":"fact → L2","zh":"為什麼用加權","en":["why weighting"],"sources":[]},{"kind":"exposition","plan":"fact","zh":"變數","en":["variables"],"sources":[]}]}]}"#).unwrap();
        replace_section(&mut o, "Methods", rev);
        assert_eq!(o.len(), 2);
        assert_eq!(o[1].paras.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["p2", "p3"]);
    }

    #[test]
    fn compose_extra_separates_full_claims_from_callbacks() {
        let mut s = Session::default();
        s.claims = vec![
            Claim { id: "c1".into(), zh: "z".into(), en: "Reliability is a joint condition.".into(), home: "Results".into(), ..Default::default() },
            Claim { id: "c2".into(), zh: "z".into(), en: "Excluding hard frames is damaging.".into(), home: "Introduction".into(), ..Default::default() },
        ];
        let p = OutlinePara { id: "p9".into(), kind: "Results key result".into(), plan: "table → numbers → L2".into(), zh: "主表".into(), en: vec![], sources: vec!["c1".into(), "c2".into()] };
        let x = compose_extra(&p, &s, "Results", "Paragraphs 80–150 words.");
        let (full, back) = x.split_once("Only call these back").unwrap();
        assert!(full.contains("joint condition") && back.contains("damaging"));
        assert!(x.contains("80–150"));
    }

    #[test]
    fn claims_are_numbered_by_the_code() {
        let c = parse_claims(r#"{"claims":[{"zh":"可靠性是聯合條件","en":"Reliability is a joint condition.","depth":4,"evidence":"Table 6","reader_accepts":"","twist":"","home":"Results","items":["i4"]},{"zh":" ","en":"","depth":null,"evidence":"","reader_accepts":"","twist":"","home":"","items":[]}]}"#).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].id, "c1");
    }

    #[test]
    fn parses_review_and_audit() {
        let r = parse_review(r#"{"chain":[{"group":"g1","conclusion":"c","follows":true,"note":""}],"suggestions":[{"kind":"merge","groups":["g3","g4"],"detail":"合成一張表","reason":"同一個比較"},{"kind":"other","groups":[],"detail":" ","reason":""}]}"#).unwrap();
        assert_eq!(r.suggestions.len(), 1);
        assert_eq!(r.suggestions[0].id, "s1");
        let f = parse_audit(r#"{"findings":[{"kind":"overclaim","para":"p7","quote":"drives the improvement","why":"差距小於一個標準差","ask":"要不要改成 is consistent with？"}]}"#).unwrap();
        assert_eq!(f[0].id, "f1");
        assert!(!f[0].resolved);
    }
}
