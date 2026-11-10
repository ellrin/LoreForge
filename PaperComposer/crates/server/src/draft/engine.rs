//! Which engine runs each 討論草稿 step, decided when the step runs.
//!
//! A step tries the task's candidates in order (the user's saved override,
//! the task's default — Claude —, then its fallback, the local model); the
//! first that is available and gives a usable reply wins. On the local model
//! a step gets prompts fitted to its context, long material in pieces (the
//! experiment record, the outline and the reviewer pass a section at a time,
//! the paper as first sentences for a revision), and every reply is repaired
//! to the schema before it is parsed, with one retry. The step reports which
//! engine ran (`Opus`, `Sonnet`, `本機`) and, after a fallback, why.

use async_trait::async_trait;
use pc_core::compose::Prompt;
use pc_core::draft::fallback::{self as fb, engine_label, is_local};
use pc_core::draft::lens::Lens;
use pc_core::draft::prompts::{self as dp, RecordReply};
use pc_core::draft::revise::{self as rv, Revision, RevisionPlan};
use pc_core::draft::session::{EngineUsed, Finding, Group, Item, OutlineSection, Review, Session, Claim, SECTIONS};
use serde_json::Value;
use std::path::Path;

/// Record text sent to Claude is cut here (a local model reads it all, in pieces).
pub const RECORD_MAX_CHARS: usize = 120_000;
/// Context assumed for the local model when it does not say.
const DEFAULT_LOCAL_CONTEXT: u32 = 16_384;

/// The model calls a step needs; the app's engines in the server, a mock in tests.
#[async_trait]
pub trait Engines: Send + Sync {
    /// Model refs to try for `task`, in order. `picked` (the draft window's
    /// model picker) is the only one when given.
    fn candidates(&self, task: &str, picked: Option<&str>) -> Vec<String>;
    /// One call on exactly `model_ref` (no fallback inside).
    async fn complete(&self, task: &str, model_ref: &str, prompt: &Prompt, extra: Option<&str>, schema: Value) -> Result<String, String>;
    /// The local engine's context in tokens, when one is connected.
    async fn local_context(&self, task: &str) -> Option<u32>;
}

/// The app's engines: lore-llm through `Llm`, candidates from the task table
/// (bundled + the user's tasks.toml).
pub struct Live<'a> {
    pub llm: &'a crate::llm::Llm,
    pub table: lore_llm::TaskTable,
}

#[async_trait]
impl Engines for Live<'_> {
    fn candidates(&self, task: &str, picked: Option<&str>) -> Vec<String> {
        // the shared routing (engine.toml: class routes, overrides); a pick is the only one
        self.table.candidates_for(task, &picked.map(lore_llm::RouteOptions::only).unwrap_or_default())
    }
    async fn complete(&self, task: &str, model_ref: &str, prompt: &Prompt, extra: Option<&str>, schema: Value) -> Result<String, String> {
        self.llm.complete_on(task, model_ref, prompt, extra, Some(schema)).await
    }
    async fn local_context(&self, task: &str) -> Option<u32> {
        let r = self.llm.route(task, Some(crate::llm::EngineChoice::Local)).await.ok()?;
        Some(r.provider.capabilities().context)
    }
}

/// One candidate engine: its ref, and the prompt budget when it is small.
pub struct Cand {
    pub model: String,
    /// Prompt characters it takes (`None` = Claude: the prompts as they are).
    pub budget: Option<usize>,
}

/// Why a candidate did not give a result.
#[derive(Debug)]
pub enum Fail {
    /// Not available, or the call failed: try the next engine.
    Engine(String),
    /// It answered, but twice not in a usable shape.
    Output(String),
}

impl Fail {
    fn text(&self) -> &str {
        match self {
            Fail::Engine(s) | Fail::Output(s) => s,
        }
    }
}

/// The task's candidates, tried in order until one gives a result.
pub struct Attempts {
    task: String,
    models: std::vec::IntoIter<String>,
    why: Vec<String>,
}

impl Attempts {
    pub fn new(e: &dyn Engines, task: &str, picked: Option<&str>) -> Attempts {
        Attempts { task: task.into(), models: e.candidates(task, picked).into_iter(), why: vec![] }
    }

    /// The next engine to try (with its prompt budget when it is small).
    pub async fn next(&mut self, e: &dyn Engines) -> Option<Cand> {
        let model = self.models.next()?;
        let budget = if is_local(&model) { Some(fb::prompt_budget(e.local_context(&self.task).await.unwrap_or(DEFAULT_LOCAL_CONTEXT))) } else { None };
        Some(Cand { model, budget })
    }

    /// What ran, and why earlier candidates did not.
    pub fn used(&self, c: &Cand) -> EngineUsed {
        EngineUsed { label: engine_label(&c.model), note: self.why.join("；") }
    }

    pub fn failed(&mut self, c: &Cand, f: Fail) {
        self.why.push(format!("{}：{}", engine_label(&c.model), f.text()));
    }

    pub fn error(self) -> String {
        if self.why.is_empty() {
            return format!("沒有這個任務：{}", self.task);
        }
        format!("沒有可用的引擎（{}）", self.why.join("；"))
    }
}

/// Try `$body` (an expression giving `Result<T, Fail>`, with the candidate
/// bound to `$c`) on each candidate until one succeeds.
macro_rules! first_working {
    ($e:expr, $task:expr, $picked:expr, |$c:ident| $body:expr) => {{
        let mut at = Attempts::new($e, $task, $picked);
        let mut out = None;
        while let Some($c) = at.next($e).await {
            let r: Result<_, Fail> = $body;
            match r {
                Ok(v) => {
                    out = Some((v, at.used(&$c)));
                    break;
                }
                Err(f) => at.failed(&$c, f),
            }
        }
        match out {
            Some(v) => Ok(v),
            None => Err(at.error()),
        }
    }};
}

/// One structured call: on a small model the prompt is fitted and asks for
/// strict JSON; a reply that does not parse is repaired to the schema, then
/// asked again once with the reason.
pub async fn ask<T>(e: &dyn Engines, task: &str, c: &Cand, prompt: &Prompt, schema: &Value, parse: impl Fn(&str) -> Result<T, String>) -> Result<T, Fail> {
    let prompt = match c.budget {
        Some(b) => fb::fit(prompt, b),
        None => prompt.clone(),
    };
    let mut extra: Option<String> = None;
    let mut last = String::new();
    for _ in 0..2 {
        let reply = e.complete(task, &c.model, &prompt, extra.as_deref(), schema.clone()).await.map_err(Fail::Engine)?;
        match parse(&reply) {
            Ok(v) => return Ok(v),
            Err(err) => {
                if let Some(v) = fb::repair(&reply, schema).and_then(|r| parse(&r).ok()) {
                    return Ok(v);
                }
                last = err;
                extra = Some(format!("Your previous reply could not be used ({last}). Reply with exactly one JSON object in the shape given, every field present."));
            }
        }
    }
    Err(Fail::Output(last))
}

/// Material and style cut for a small model's budget.
fn small_parts(b: usize, style: &str, material: &str) -> (String, String) {
    (fb::cut(style, b / 6), fb::cut(material, b / 4))
}

// ── the steps ────────────────────────────────────────────

/// Step 2. `records` is the whole record text; tables whose numbers are not
/// in it get a note (whatever engine ran).
pub async fn record(e: &dyn Engines, key_value: &str, lens: &Lens, records: &str, material: &str) -> Result<(RecordReply, EngineUsed), String> {
    let task = "pc-draft-record";
    let schema = dp::record_schema();
    let parse = |x: &str| dp::parse_record(x, lens);
    let (mut r, used) = first_working!(e, task, None, |c| match c.budget {
        None => {
            let text = if records.chars().count() > RECORD_MAX_CHARS { records.chars().take(RECORD_MAX_CHARS).collect::<String>() + "\n[… record truncated …]\n" } else { records.to_string() };
            ask(e, task, &c, &dp::record_prompt(key_value, lens, &text, material), &schema, parse).await
        }
        Some(b) => record_in_pieces(e, &c, b, key_value, lens, records, material).await,
    })?;
    fb::flag_unfound_numbers(&mut r.tables, records);
    Ok((r, used))
}

async fn record_in_pieces(e: &dyn Engines, c: &Cand, b: usize, key_value: &str, lens: &Lens, records: &str, material: &str) -> Result<RecordReply, Fail> {
    let task = "pc-draft-record";
    let schema = dp::record_schema();
    let m = fb::cut(material, b / 4);
    let overhead = fb::fit(&dp::record_prompt(key_value, lens, "", &m), b);
    let room = b.saturating_sub(overhead.system.chars().count() + overhead.user.chars().count() + 200).max(1_500);
    let pieces = fb::chunk_records(records, room);
    let n = pieces.len();
    // Each piece already fits: the budget is only there to keep it whole.
    let whole = Cand { model: c.model.clone(), budget: Some(b + 400) };
    let mut parts = vec![];
    for (i, p) in pieces.iter().enumerate() {
        let text = if n > 1 { format!("[Part {} of {n} of the record: organise only what this part shows.]\n{p}", i + 1) } else { p.clone() };
        parts.push(ask(e, task, &whole, &dp::record_prompt(key_value, lens, &text, &m), &schema, |x| dp::parse_record(x, lens)).await?);
    }
    Ok(fb::merge_records(parts))
}

/// Step 3.
pub async fn plan(e: &dyn Engines, key_value: &str, sess: &Session, lens: &Lens, style: &str, material: &str) -> Result<(Vec<(Group, Vec<Item>)>, EngineUsed), String> {
    let task = "pc-draft-plan";
    let schema = dp::plan_schema();
    first_working!(e, task, None, |c| {
        let p = match c.budget {
            None => dp::plan_prompt(key_value, sess, lens, style, material),
            Some(b) => {
                let (st, m) = small_parts(b, style, material);
                dp::plan_prompt(key_value, sess, lens, &st, &m)
            }
        };
        ask(e, task, &c, &p, &schema, |x| dp::parse_plan(x, lens)).await
    })
}

/// Step 4: one interview turn (`picked` from the model picker: that engine
/// only). On a small model the prompt's middle (context, material) is cut;
/// the current group and the latest answer at its end stay.
pub async fn turn(e: &dyn Engines, picked: Option<&str>, prompt: &Prompt) -> Result<(dp::TurnReply, EngineUsed), String> {
    let task = "pc-draft-turn";
    let schema = dp::turn_schema();
    first_working!(e, task, picked, |c| ask(e, task, &c, prompt, &schema, dp::parse_turn).await)
}

/// Step 4.5.
pub async fn claims(e: &dyn Engines, key_value: &str, sess: &Session, lens: &Lens, material: &str) -> Result<(Vec<Claim>, EngineUsed), String> {
    let task = "pc-draft-claims";
    let schema = dp::claims_schema();
    first_working!(e, task, None, |c| {
        let m = match c.budget {
            Some(b) => fb::cut(material, b / 4),
            None => material.to_string(),
        };
        ask(e, task, &c, &dp::claims_prompt(key_value, sess, lens, &m), &schema, dp::parse_claims).await
    })
}

/// Step 5.
pub async fn review(e: &dyn Engines, key_value: &str, sess: &Session, lens: &Lens) -> Result<(Review, EngineUsed), String> {
    let task = "pc-draft-review";
    let schema = dp::review_schema();
    let p = dp::review_prompt(key_value, sess, lens);
    first_working!(e, task, None, |c| ask(e, task, &c, &p, &schema, dp::parse_review).await)
}

/// Step 6. `only` = revise one section after a comment. A small model
/// outlines the whole paper a section at a time.
pub async fn outline(e: &dyn Engines, key_value: &str, sess: &Session, lens: &Lens, style: &str, only: Option<(&str, &str)>, material: &str) -> Result<(Vec<OutlineSection>, EngineUsed), String> {
    let task = "pc-draft-outline";
    let schema = dp::outline_schema();
    first_working!(e, task, None, |c| match (c.budget, only) {
        (None, _) => ask(e, task, &c, &dp::outline_prompt(key_value, sess, lens, style, only, material), &schema, dp::parse_outline).await,
        (Some(b), Some(_)) => {
            let (st, m) = small_parts(b, style, material);
            ask(e, task, &c, &dp::outline_prompt(key_value, sess, lens, &st, only, &m), &schema, dp::parse_outline).await
        }
        (Some(b), None) => outline_by_section(e, &c, b, key_value, sess, lens, style, material).await,
    })
}

#[allow(clippy::too_many_arguments)]
async fn outline_by_section(e: &dyn Engines, c: &Cand, b: usize, key_value: &str, sess: &Session, lens: &Lens, style: &str, material: &str) -> Result<Vec<OutlineSection>, Fail> {
    let task = "pc-draft-outline";
    let schema = dp::outline_schema();
    let (st, m) = small_parts(b, style, material);
    let mut parts = vec![];
    for sec in SECTIONS {
        // Asked for this section only: whatever the model called it, it is this one.
        let part = ask(e, task, c, &dp::outline_section_prompt(key_value, sess, lens, &st, sec, &m), &schema, |x| dp::parse_outline_section(x, sec)).await?;
        parts.push((sec, part));
    }
    Ok(dp::merge_outline_parts(parts))
}

/// Step 8. A small model reads the draft a section at a time; findings are
/// numbered across sections.
pub async fn audit(e: &dyn Engines, key_value: &str, sess: &Session, lens: &Lens, style: &str, material: &str) -> Result<(Vec<Finding>, EngineUsed), String> {
    let task = "pc-draft-audit";
    let schema = dp::audit_schema();
    first_working!(e, task, None, |c| match c.budget {
        None => ask(e, task, &c, &dp::audit_prompt(key_value, sess, lens, style, material), &schema, dp::parse_audit).await,
        Some(b) => audit_by_section(e, &c, b, key_value, sess, lens, style, material).await,
    })
}

#[allow(clippy::too_many_arguments)]
async fn audit_by_section(e: &dyn Engines, c: &Cand, b: usize, key_value: &str, sess: &Session, lens: &Lens, style: &str, material: &str) -> Result<Vec<Finding>, Fail> {
    let task = "pc-draft-audit";
    let schema = dp::audit_schema();
    let (st, m) = small_parts(b, style, material);
    let mut all: Vec<Finding> = vec![];
    for d in &sess.drafts {
        let only = [d.section.as_str()];
        all.extend(ask(e, task, c, &dp::audit_prompt_for(key_value, sess, lens, &st, &m, Some(&only)), &schema, dp::parse_audit).await?);
    }
    for (i, f) in all.iter_mut().enumerate() {
        f.id = format!("f{}", i + 1);
    }
    Ok(all)
}

/// 修訂: the plan for one revision. A small model sees the paper as first
/// sentences and may not replace a paragraph (those become effects).
#[allow(clippy::too_many_arguments)]
pub async fn revise(e: &dyn Engines, key_value: &str, sess: &Session, r: &Revision, dir: &Path, material: &str, style: &str, conversation: &str) -> Result<(RevisionPlan, EngineUsed), String> {
    let task = "pc-draft-revise";
    let schema = rv::plan_schema();
    first_working!(e, task, None, |c| match c.budget {
        None => ask(e, task, &c, &rv::plan_prompt(key_value, sess, r, dir, material, style, conversation), &schema, |x| rv::parse_plan(x, dir)).await,
        Some(b) => {
            let (st, m) = small_parts(b, style, material);
            let p = rv::compact_plan_prompt(key_value, sess, r, dir, &m, &st, &fb::cut(conversation, b / 4), b / 3);
            match ask(e, task, &c, &p, &schema, |x| rv::parse_plan(x, dir)).await {
                Ok(mut plan) => {
                    rv::drop_replaces(&mut plan);
                    Ok(plan)
                }
                Err(f) => Err(f),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pc_core::draft::session::{DraftPara, DraftSection};
    use std::sync::Mutex;

    /// Claude is never available; the local model answers from a queue and
    /// every call is recorded.
    struct NoClaude {
        replies: Mutex<Vec<String>>,
        seen: Mutex<Vec<(String, Prompt)>>,
        claude: bool,
        context: u32,
    }

    impl NoClaude {
        fn new(replies: &[&str]) -> NoClaude {
            NoClaude { replies: Mutex::new(replies.iter().map(|s| s.to_string()).collect()), seen: Mutex::new(vec![]), claude: false, context: 16_384 }
        }
        fn calls(&self) -> Vec<(String, Prompt)> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Engines for NoClaude {
        fn candidates(&self, task: &str, picked: Option<&str>) -> Vec<String> {
            if let Some(p) = picked {
                return vec![p.into()];
            }
            let t = crate::llm::Llm::task_table(None).unwrap();
            let t = &t.task[task];
            [Some(t.default.clone()), t.fallback.clone()].into_iter().flatten().collect()
        }
        async fn complete(&self, _task: &str, model_ref: &str, prompt: &Prompt, _extra: Option<&str>, _schema: Value) -> Result<String, String> {
            if model_ref.starts_with("claude") && !self.claude {
                return Err("沒有登入 Claude，也沒有 API key".into());
            }
            self.seen.lock().unwrap().push((model_ref.into(), prompt.clone()));
            let mut r = self.replies.lock().unwrap();
            if r.is_empty() { Err("mock 沒有回覆".into()) } else { Ok(r.remove(0)) }
        }
        async fn local_context(&self, _task: &str) -> Option<u32> {
            Some(self.context)
        }
    }

    fn lens() -> Lens {
        Lens::builtin()
    }

    fn fell_back(u: &EngineUsed, from: &str) {
        assert_eq!(u.label, "本機");
        assert!(u.note.contains(from) && u.note.contains("沒有登入"), "{u:?}");
    }

    fn all_local_strict(e: &NoClaude) {
        for (m, p) in e.calls() {
            assert_eq!(m, "local:current");
            assert!(p.system.contains("Every field must be present"));
        }
    }

    #[test]
    fn every_draft_task_falls_back_to_local() {
        let t = crate::llm::Llm::task_table(None).unwrap();
        for id in ["pc-draft-record", "pc-draft-plan", "pc-draft-turn", "pc-draft-claims", "pc-draft-review", "pc-draft-outline", "pc-draft-audit", "pc-draft-revise"] {
            assert!(t.task[id].default.starts_with("claude:"), "{id}: Claude first");
            assert_eq!(t.task[id].fallback.as_deref(), Some("local:current"), "{id}");
            assert_ne!(t.task[id].offline, Some(false), "{id} runs offline on the local model");
        }
    }

    #[tokio::test]
    async fn record_reads_a_long_record_in_pieces() {
        let mut records = String::new();
        for i in 0..400 {
            records += &format!("run {i}: lr 0.{i:03} Dice 0.8{}\n", i % 10);
        }
        let records = format!("=== log.md ===\n{records}\n\n");
        let piece = |n: usize, cap: &str, num: &str| {
            format!(
                r#"{{"summary":"第{n}段","experiments":[{{"id":"E1","title":"Run set {n}","change":"lr","metrics":"Dice","result":"{num}","purpose":"","purpose_sure":false,"mode":null}}],"timeline":[{{"step":1,"what":"t{n}","observed":"","changed":""}}],"thoughts":[{{"text":"想法{n}","about":["E1","T1"]}}],"tables":[{{"caption":"{cap}","columns":["run","Dice"],"rows":[["1","{num}"]],"note":"","from":["E1"]}}],"conflicts":[]}}"#
            )
        };
        let mut replies: Vec<String> = (0..40).map(|n| piece(n, &format!("Table {n}"), "0.81")).collect();
        replies[1] = piece(1, "Made up", "0.97");
        replies[0] = format!("```json\n{}\n```", replies[0]);
        let e = NoClaude { context: 8_192, ..NoClaude::new(&replies.iter().map(String::as_str).collect::<Vec<_>>()) };
        let (r, used) = record(&e, "kv", &lens(), &records, "").await.unwrap();
        fell_back(&used, "Opus");
        all_local_strict(&e);
        let calls = e.calls();
        assert!(calls.len() > 2, "a record longer than the context is read in pieces ({})", calls.len());
        for (_, p) in &calls {
            assert!(p.user.chars().count() + p.system.chars().count() <= fb::prompt_budget(8_192) + 600, "each piece fits the local context");
        }
        let sent: String = calls.iter().map(|(_, p)| p.user.clone()).collect();
        for i in 0..400 {
            assert!(sent.contains(&format!("run {i}: lr")), "line {i} was sent");
        }
        assert_eq!(r.experiments.len(), calls.len());
        assert_eq!(r.timeline.len(), calls.len());
        assert_eq!(r.thoughts[1].about, ["E2", "T2"], "ids follow the merge");
        let made_up = r.tables.iter().find(|t| t.caption == "Made up").unwrap();
        assert!(made_up.note.contains("0.97"), "a number not in the record is flagged: {}", made_up.note);
    }

    #[tokio::test]
    async fn claude_runs_first_when_it_is_there() {
        let e = NoClaude { claude: true, ..NoClaude::new(&[r#"{"chain":[],"suggestions":[]}"#]) };
        let (_, used) = review(&e, "kv", &Session::default(), &lens()).await.unwrap();
        assert_eq!(used, EngineUsed { label: "Sonnet".into(), note: String::new() });
        let calls = e.calls();
        assert_eq!(calls[0].0, "claude:sonnet");
        assert!(!calls[0].1.system.contains("Every field must be present"), "Claude gets the prompt as it is");
    }

    #[tokio::test]
    async fn plan_claims_and_review_on_local() {
        let mut s = Session::default();
        let e = NoClaude::new(&[r#"{"groups":[{"section":"Methods","title":"損失","purpose":"為什麼","items":[{"label":"BCE","role":"design"}]}]}"#]);
        let style = "s".repeat(50_000);
        let (g, used) = plan(&e, "kv", &s, &lens(), &style, "m").await.unwrap();
        fell_back(&used, "Sonnet");
        all_local_strict(&e);
        assert!(e.calls()[0].1.user.chars().count() < 20_000, "the style notes are cut to fit");
        assert_eq!(g[0].1[0].label, "BCE", "a missing `mode` is filled by the repair");
        s.set_plan(g);

        // A first reply that is not JSON at all: asked again.
        let e = NoClaude::new(&["我覺得主張是……", r#"{"claims":[{"zh":"加權有效","en":"Weighting helps.","home":"Results"}]}"#]);
        let (c, used) = claims(&e, "kv", &s, &lens(), "").await.unwrap();
        fell_back(&used, "Sonnet");
        assert_eq!(e.calls().len(), 2);
        assert_eq!((c[0].id.as_str(), c[0].en.as_str()), ("c1", "Weighting helps."));

        let e = NoClaude::new(&[r#"{"chain":[{"group":"g1","conclusion":"c","follows":"true"}],"suggestions":[]}"#]);
        let (r, used) = review(&e, "kv", &s, &lens()).await.unwrap();
        fell_back(&used, "Sonnet");
        assert!(r.chain[0].follows);
    }

    #[tokio::test]
    async fn turns_follow_the_picker() {
        let p = Prompt { system: "S".into(), user: "U".into() };
        let reply = r#"{"updates":[],"summary":{"explain":"","motive":"","conclusion":"","reader_accepts":"","twist":""},"reply":"了解","question":"為什麼？","options":[]}"#;
        let e = NoClaude::new(&[reply]);
        let (t, used) = turn(&e, None, &p).await.unwrap();
        fell_back(&used, "Haiku");
        assert_eq!(t.question, "為什麼？");
        // A model picked in the window is that model or an error, never another.
        let e = NoClaude::new(&[reply]);
        let err = turn(&e, Some("claude:opus"), &p).await.unwrap_err();
        assert!(err.contains("Opus") && e.calls().is_empty(), "{err}");
    }

    #[tokio::test]
    async fn outline_goes_section_by_section_on_local() {
        let para = |zh: &str| format!(r#"{{"kind":"exposition","plan":"fact","zh":"{zh}","en":["{zh}"],"sources":[],"cites":[]}}"#);
        let replies: Vec<String> = SECTIONS
            .iter()
            .map(|s| format!(r#"{{"sections":[{{"section":"{}","subsection":null,"paras":[{}]}}]}}"#, if *s == "Methods" { "Method" } else { s }, para(s)))
            .collect();
        let e = NoClaude::new(&replies.iter().map(String::as_str).collect::<Vec<_>>());
        let (o, used) = outline(&e, "kv", &Session::default(), &lens(), "", None, "").await.unwrap();
        fell_back(&used, "Sonnet");
        assert_eq!(e.calls().len(), 5);
        assert_eq!(o.iter().map(|x| x.section.as_str()).collect::<Vec<_>>(), SECTIONS, "a section the model misnamed is still that section");
        assert_eq!(o.iter().map(|x| x.paras[0].id.as_str()).collect::<Vec<_>>(), ["p1", "p2", "p3", "p4", "p5"]);
        // Revising one section is one call.
        let e = NoClaude::new(&[&replies[2]]);
        let (o, _) = outline(&e, "kv", &Session::default(), &lens(), "", Some(("Results", "加數字")), "").await.unwrap();
        assert_eq!((e.calls().len(), o[0].section.as_str()), (1, "Results"));
    }

    #[tokio::test]
    async fn audit_reads_the_draft_a_section_at_a_time() {
        let mut s = Session::default();
        for (sec, text) in [("Methods", "We use BCE."), ("Results", "Ours drives the improvement.")] {
            s.drafts.push(DraftSection { section: sec.into(), paras: vec![DraftPara { para: "p1".into(), text: text.into(), problems: vec![] }], new_symbols: vec![], engine: String::new() });
        }
        let f = |k: &str| format!(r#"{{"findings":[{{"kind":"{k}","para":"p1","quote":"q","why":"w","ask":""}}]}}"#);
        let e = NoClaude::new(&[&f("missing-why"), &f("overclaim")]);
        let (found, used) = audit(&e, "kv", &s, &lens(), "", "").await.unwrap();
        fell_back(&used, "Opus");
        let calls = e.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].1.user.contains("We use BCE.") && !calls[0].1.user.contains("drives the improvement"));
        assert_eq!(found.iter().map(|x| (x.id.as_str(), x.kind.as_str())).collect::<Vec<_>>(), [("f1", "missing-why"), ("f2", "overclaim")]);
    }

    #[tokio::test]
    async fn a_revision_on_local_never_replaces_a_paragraph() {
        let dir = std::env::temp_dir().join(format!("pc-draft-engine-rev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sections")).unwrap();
        std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/results}\n\\end{document}\n").unwrap();
        std::fs::write(dir.join("sections/results.tex"), "\\section{Results}\nFirst paragraph. It has a second sentence that must not be lost.\n\nSecond paragraph.\n").unwrap();
        let r = Revision { id: "v1".into(), request: "加外部驗證".into(), group: "v1".into(), ..Default::default() };
        let reply = r#"{"summary":"加入外部驗證","changes":[
          {"file":"results.tex","action":"replace","index":0,"text":"First paragraph.","reason":"更新"},
          {"file":"results.tex","action":"insert-after","index":1,"text":"External validation (Table~\\ref{tab:ext}) gave Dice 0.71.","reason":"新數據"}],
          "effects":[],"caption":null}"#;
        let e = NoClaude::new(&[reply]);
        let (plan, used) = revise(&e, "kv", &Session::default(), &r, &dir, "", "", "Q: 放哪？\nA: 主表後").await.unwrap();
        fell_back(&used, "Opus");
        assert!(e.calls()[0].1.user.contains("first sentence") && !e.calls()[0].1.user.contains("must not be lost"));
        assert_eq!(plan.changes.len(), 1);
        assert_eq!((plan.changes[0].id.as_str(), plan.changes[0].action.as_str()), ("x1", "insert-after"));
        assert!(plan.effects.iter().any(|x| x.contains("results.tex ¶0")), "{:?}", plan.effects);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn nothing_available_says_why_for_each_engine() {
        let e = NoClaude::new(&[]);
        let err = review(&e, "kv", &Session::default(), &lens()).await.unwrap_err();
        assert!(err.contains("Sonnet：沒有登入") && err.contains("本機：mock 沒有回覆"), "{err}");
        // Local answers, but twice in no usable shape.
        let e = NoClaude::new(&["not json", "still not"]);
        let err = review(&e, "kv", &Session::default(), &lens()).await.unwrap_err();
        assert!(err.contains("本機："), "{err}");
    }
}
