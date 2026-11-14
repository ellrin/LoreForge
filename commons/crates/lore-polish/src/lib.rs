//! lore-polish: 語句潤飾 (`docs/long-paper.md` §A). The author selects a passage, says how to
//! rewrite it (typed or spoken), and a model rewrites only that passage.
//!
//! - [`lock`]: citations, references, labels, math, comments and registered macros go out as
//!   locked placeholders and must come back exactly once ([`lock::Problem`] otherwise).
//! - [`prompt`]: the `polish` request (task `polish` in this crate's `tasks.toml`, class
//!   `create`; merged into an app's tasks with [`task_table`]).
//! - [`log`]: one JSON line per proposal in the document's own log.
//! - [`liked`]: the author's tone library (`private/shared/polish/liked.jsonl`).
//! - [`propose`] / [`like`] / [`applied`] / [`history`]: the whole flow over an app's [`Host`];
//!   [`web::router`] serves it as `/api/polish*` (feature `web`).

pub mod liked;
pub mod lock;
pub mod log;
pub mod prompt;
#[cfg(feature = "web")]
pub mod web;

use std::path::PathBuf;

use lore_home::Home;
use lore_llm::{LlmRequest, TaskTable};
use serde::{Deserialize, Serialize};

pub use lock::{Locked, Problem};

/// The task id.
pub const TASK: &str = "polish";

/// The `polish` task, to merge into an app's task table (and its 模型 tab).
pub fn task_table() -> TaskTable {
    TaskTable::parse(include_str!("../tasks.toml")).expect("lore-polish tasks.toml parses")
}

/// A named rewrite the app offers next to the free instruction (PaperComposer: 「用字」,
/// 「我的風格」). The page shows `label`; the model gets `instruction` and the app's
/// [`Host::preset_block`] for the passage.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preset {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing)]
    pub instruction: String,
}

/// The passage a preset's block is made for.
#[derive(Debug, Clone, Default)]
pub struct PresetPassage<'a> {
    pub text: &'a str,
    pub prefix: &'a str,
    pub suffix: &'a str,
    /// Where it sits, in the app's words (PaperComposer: the section title).
    pub scope: &'a str,
}

/// What an app knows about one document.
#[derive(Debug, Clone, Default)]
pub struct Doc {
    /// The proposal log (PaperComposer `draft/polish.jsonl`, SlideComposer `polish-log.jsonl`).
    pub log: PathBuf,
    /// Registered macro names (locked like citations).
    pub macros: Vec<String>,
    /// The document's language (`en`, `zh-TW`; empty = the passage's own).
    pub language: String,
    /// The author's phrases most useful for this passage (bounded by the app).
    pub phrases: Vec<String>,
    /// A ready block about the author's tone (SlideComposer: lines from his own decks).
    pub tone: String,
    /// The author's style rules for the rewrite (free text, bounded by the app; empty: none).
    pub style: String,
}

/// The app side: documents, the model, its own checks.
#[async_trait::async_trait]
pub trait Host: Send + Sync + 'static {
    /// `PaperComposer`, `SlideComposer` (written with each liked rewrite).
    fn app(&self) -> &str;
    /// Where the tone library lives.
    fn home(&self) -> Home;
    /// The document `doc` (a paper name, a deck name) for a passage `text`.
    async fn doc(&self, doc: &str, text: &str) -> Result<Doc, String>;
    /// Run `req` as task [`TASK`]: the JSON reply and the provider id. `engine` = the page's
    /// engine choice, in the app's own words.
    async fn complete(&self, req: LlmRequest, engine: Option<&str>) -> Result<(serde_json::Value, String), String>;
    /// New problems the app's own checks find once the rewrite is in place (`before` /
    /// `after` = the whole paragraph). PaperComposer: lore-notation's `check_document`.
    async fn check(&self, _doc: &str, _before: &str, _after: &str) -> Vec<String> {
        vec![]
    }
    /// Is the document LaTeX? Then Unicode a model writes (≤, −, ⁺, α…) comes back as LaTeX
    /// (`lore_notation::unicode::to_latex`), so a draft PDF never stops on it. SlideComposer: no.
    fn latex(&self) -> bool {
        false
    }
    /// The app's presets (none by default).
    fn presets(&self) -> Vec<Preset> {
        vec![]
    }
    /// What the model needs for preset `preset` on this passage (findings of the app's own
    /// checks, the author's style for its section…); empty: nothing more. Bounded by the app.
    async fn preset_block(&self, _doc: &str, _preset: &str, _p: &PresetPassage<'_>) -> String {
        String::new()
    }
}

/// One earlier turn, as the page keeps it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Turn {
    pub instruction: String,
    /// The restored rewrite of that turn.
    pub after: String,
}

/// `POST /api/polish`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProposeBody {
    pub doc: String,
    /// The selected passage (LaTeX source).
    pub text: String,
    /// The rest of its paragraph, before and after it.
    pub prefix: String,
    pub suffix: String,
    /// Nearby paragraphs (context only).
    pub context: String,
    /// Overrides the document's language.
    pub language: Option<String>,
    pub instruction: String,
    pub turns: Vec<Turn>,
    /// The conversation's passage id (from an earlier answer); found from `text` when missing.
    pub passage: Option<String>,
    pub engine: Option<String>,
    /// More literal strings to keep locked (SlideComposer: a line break).
    pub keep: Vec<String>,
    /// One of the app's [`Preset`] ids; the free `instruction` may then be empty.
    pub preset: Option<String>,
    /// Where the passage sits (the app's words; passed to [`Host::preset_block`]).
    pub scope: String,
}

/// The answer to a proposal.
#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub id: String,
    pub passage: String,
    /// The rewrite with every locked part back in place.
    pub after: String,
    /// Safe to apply (no problems).
    pub ok: bool,
    pub problems: Vec<String>,
    pub engine: String,
}

/// Errors as a status-like code and a message.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Bad(String),
    NotFound(String),
    Model(String),
    Io(String),
}

impl Error {
    pub fn message(&self) -> &str {
        match self {
            Error::Bad(m) | Error::NotFound(m) | Error::Model(m) | Error::Io(m) => m,
        }
    }
}

/// Lock, ask, restore, check, log.
pub async fn propose(host: &dyn Host, b: ProposeBody) -> Result<Proposal, Error> {
    if b.text.trim().is_empty() {
        return Err(Error::Bad("先選取要潤飾的文字".into()));
    }
    let preset = match b.preset.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(id) => Some(host.presets().into_iter().find(|p| p.id == id).ok_or_else(|| Error::Bad(format!("沒有這個預設：{id}")))?),
        None => None,
    };
    if b.instruction.trim().is_empty() && preset.is_none() {
        return Err(Error::Bad("說說要怎麼改".into()));
    }
    if b.text.chars().count() > 6000 {
        return Err(Error::Bad("選取的文字太長了（一次一段）".into()));
    }
    let doc = host.doc(&b.doc, &b.text).await.map_err(Error::NotFound)?;
    // a preset: its instruction (+ the author's words) and the app's block for this passage
    let (instruction, guidance, logged) = match &preset {
        Some(p) => {
            let block = host.preset_block(&b.doc, &p.id, &PresetPassage { text: &b.text, prefix: &b.prefix, suffix: &b.suffix, scope: &b.scope }).await;
            let free = b.instruction.trim();
            let instruction = if free.is_empty() { p.instruction.trim().to_string() } else { format!("{}\nAlso: {free}", p.instruction.trim()) };
            let logged = if free.is_empty() { format!("「{}」", p.label) } else { format!("「{}」{free}", p.label) };
            (instruction, block, logged)
        }
        None => (b.instruction.trim().to_string(), String::new(), b.instruction.trim().to_string()),
    };
    let locked = lock::lock(&b.text, &doc.macros, &b.keep);
    let turns: Vec<prompt::TurnText> = b.turns.iter().map(|t| prompt::TurnText { instruction: t.instruction.clone(), rewrite: locked.encode(&t.after, &doc.macros, &b.keep) }).collect();
    let liked = liked::examples(&liked::read(&host.home()), &b.text, liked::EXAMPLES);
    let language = b.language.clone().filter(|l| !l.trim().is_empty()).unwrap_or(doc.language.clone());
    let req = prompt::request(&prompt::Input {
        passage: &locked.text,
        before: &b.prefix,
        after: &b.suffix,
        context: &b.context,
        language: &language,
        phrases: &doc.phrases,
        tone: &doc.tone,
        style: &doc.style,
        liked: &liked,
        turns: &turns,
        instruction: &instruction,
        guidance: &guidance,
        preset: preset.as_ref().map(|p| p.label.as_str()).unwrap_or(""),
    });
    let (reply, engine) = host.complete(req, b.engine.as_deref()).await.map_err(Error::Model)?;
    let Some(rewrite) = prompt::rewrite_of(&reply) else { return Err(Error::Model("模型沒有給改寫".into())) };
    let (after, problems) = locked.restore(&rewrite, &doc.macros, &b.keep);
    let after = if host.latex() { lore_notation::unicode::prose_to_latex(&after) } else { after };
    let mut problems: Vec<String> = problems.iter().map(Problem::message).collect();
    if problems.is_empty() {
        let (old, new) = (format!("{}{}{}", b.prefix, b.text, b.suffix), format!("{}{}{}", b.prefix, after, b.suffix));
        problems.extend(host.check(&b.doc, &old, &new).await);
    }
    let all = log::read(&doc.log);
    let passage = b.passage.clone().filter(|p| !p.trim().is_empty()).unwrap_or_else(|| log::passage_of(&all, &b.text));
    let e = log::Entry {
        id: log::new_id(),
        passage: passage.clone(),
        time: log::now(),
        // what the author asked, in his words (a preset by its label, never its block)
        instruction: logged,
        before: b.text.clone(),
        after: after.clone(),
        engine: engine.clone(),
        problems: problems.clone(),
        ..Default::default()
    };
    log::append(&doc.log, &e).map_err(Error::Io)?;
    Ok(Proposal { id: e.id, passage, after, ok: problems.is_empty(), problems, engine })
}

/// 套用 happened: mark the line.
pub async fn applied(host: &dyn Host, doc: &str, id: &str) -> Result<log::Entry, Error> {
    let d = host.doc(doc, "").await.map_err(Error::NotFound)?;
    log::update(&d.log, id, |e| e.applied = true).map_err(Error::NotFound)
}

/// 👍: mark the line and add it to the tone library.
pub async fn like(host: &dyn Host, doc: &str, id: &str) -> Result<log::Entry, Error> {
    let d = host.doc(doc, "").await.map_err(Error::NotFound)?;
    let e = log::read(&d.log).into_iter().find(|e| e.id == id).ok_or_else(|| Error::NotFound("找不到這次的改寫".into()))?;
    if !e.liked {
        liked::add(&host.home(), &liked::Liked { before: e.before.clone(), after: e.after.clone(), instruction: e.instruction.clone(), app: host.app().to_string(), time: log::now() }).map_err(Error::Io)?;
    }
    log::update(&d.log, id, |e| e.liked = true).map_err(Error::Io)
}

/// Earlier proposals of the passage `passage` (or the one `text` belongs to).
pub async fn history(host: &dyn Host, doc: &str, passage: Option<&str>, text: Option<&str>) -> Result<(String, Vec<log::Entry>), Error> {
    let d = host.doc(doc, text.unwrap_or("")).await.map_err(Error::NotFound)?;
    let all = log::read(&d.log);
    let p = match (passage.filter(|p| !p.is_empty()), text) {
        (Some(p), _) => p.to_string(),
        (None, Some(t)) => log::passage_of(&all, t),
        (None, None) => return Err(Error::Bad("要 passage 或 text".into())),
    };
    Ok((p.clone(), log::history(&all, &p)))
}

#[cfg(test)]
pub(crate) mod testhost {
    use super::*;
    use std::sync::Mutex;

    /// A host over a temp folder, answering with canned replies and recording requests.
    pub struct TestHost {
        pub home: Home,
        pub log: PathBuf,
        pub replies: Mutex<Vec<String>>,
        pub seen: Mutex<Vec<LlmRequest>>,
        pub check: Vec<String>,
        pub presets: Vec<Preset>,
        /// The preset block, with the scope it was asked for appended.
        pub block: String,
    }

    impl TestHost {
        pub fn new(root: &std::path::Path, replies: &[&str]) -> TestHost {
            TestHost { home: Home::at(root), log: root.join("doc/draft/polish.jsonl"), replies: Mutex::new(replies.iter().rev().map(|s| s.to_string()).collect()), seen: Mutex::new(vec![]), check: vec![], presets: vec![], block: String::new() }
        }
    }

    #[async_trait::async_trait]
    impl Host for TestHost {
        fn app(&self) -> &str {
            "PaperComposer"
        }
        fn home(&self) -> Home {
            self.home.clone()
        }
        async fn doc(&self, doc: &str, _text: &str) -> Result<Doc, String> {
            if doc != "P" {
                return Err("找不到".into());
            }
            Ok(Doc { log: self.log.clone(), macros: vec!["ribmask".into()], language: "en".into(), phrases: vec!["we argue that".into()], tone: String::new(), style: String::new() })
        }
        async fn complete(&self, req: LlmRequest, _engine: Option<&str>) -> Result<(serde_json::Value, String), String> {
            self.seen.lock().unwrap().push(req);
            let r = self.replies.lock().unwrap().pop().ok_or("no reply")?;
            Ok((serde_json::json!({ "rewrite": r }), "mock".into()))
        }
        async fn check(&self, _doc: &str, _before: &str, _after: &str) -> Vec<String> {
            self.check.clone()
        }
        fn presets(&self) -> Vec<Preset> {
            self.presets.clone()
        }
        async fn preset_block(&self, _doc: &str, preset: &str, p: &PresetPassage<'_>) -> String {
            if self.block.is_empty() { String::new() } else { format!("{} [{preset} in {}]", self.block, p.scope) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testhost::TestHost;
    use super::*;

    fn body(text: &str, instruction: &str) -> ProposeBody {
        ProposeBody { doc: "P".into(), text: text.into(), instruction: instruction.into(), ..Default::default() }
    }

    #[tokio::test]
    async fn a_conversation_is_logged_liked_and_restored() {
        let dir = tempfile::tempdir().unwrap();
        let text = "Moreover, as \\citet{a} showed, the \\ribmask loss $L$ drops very quickly and in a fast way.";
        let h = TestHost::new(dir.path(), &["As ⟦1⟧ showed, the ⟦2⟧ loss ⟦3⟧ drops quickly.", "As ⟦1⟧ showed, the ⟦2⟧ loss ⟦3⟧ falls quickly."]);
        let p = propose(&h, body(text, "drop Moreover, the ending is wordy")).await.unwrap();
        assert!(p.ok, "{:?}", p.problems);
        assert_eq!(p.after, "As \\citet{a} showed, the \\ribmask loss $L$ drops quickly.");
        // 再改: the earlier rewrite goes back locked with the same placeholders
        let mut b = body(text, "use 'falls'");
        b.passage = Some(p.passage.clone());
        b.turns = vec![Turn { instruction: "drop Moreover, the ending is wordy".into(), after: p.after.clone() }];
        let p2 = propose(&h, b).await.unwrap();
        assert!(h.seen.lock().unwrap()[1].flatten().contains("Your rewrite 1: As ⟦1⟧ showed, the ⟦2⟧ loss ⟦3⟧ drops quickly."));
        assert_eq!(p2.passage, p.passage);
        applied(&h, "P", &p2.id).await.unwrap();
        let e = like(&h, "P", &p2.id).await.unwrap();
        assert!(e.liked && e.applied);
        // liked twice is one line in the library
        like(&h, "P", &p2.id).await.unwrap();
        let lib = liked::read(&h.home);
        assert_eq!(lib.len(), 1);
        assert_eq!((lib[0].after.as_str(), lib[0].app.as_str()), ("As \\citet{a} showed, the \\ribmask loss $L$ falls quickly.", "PaperComposer"));
        assert!(dir.path().join("private/shared/polish/liked.jsonl").is_file());
        let (pid, items) = history(&h, "P", None, Some(&p2.after)).await.unwrap();
        assert_eq!((pid, items.len()), (p.passage.clone(), 2));
        // the next call shows the liked rewrite as an example
        let h2 = TestHost { replies: std::sync::Mutex::new(vec!["x ⟦1⟧ ⟦2⟧ ⟦3⟧".into()]), ..TestHost::new(dir.path(), &[]) };
        propose(&h2, body(text, "again")).await.unwrap();
        assert!(h2.seen.lock().unwrap()[0].flatten().contains("Rewrites the author liked before"));
    }

    #[tokio::test]
    async fn a_dropped_citation_or_a_failed_check_cannot_be_applied() {
        let dir = tempfile::tempdir().unwrap();
        let h = TestHost::new(dir.path(), &["The loss drops."]);
        let p = propose(&h, body("The loss $L$ drops \\cite{a}.", "shorter")).await.unwrap();
        assert!(!p.ok);
        assert_eq!(p.problems, ["少了 $L$", "少了 \\cite{a}"]);
        let logged = log::read(&h.log);
        assert_eq!(logged[0].problems.len(), 2);
        let mut h = TestHost::new(dir.path(), &["The loss ⟦1⟧ drops ⟦2⟧."]);
        h.check = vec!["縮寫 CV 第一次出現沒有展開".into()];
        let p = propose(&h, body("The loss $L$ drops \\cite{a}.", "shorter")).await.unwrap();
        assert!(!p.ok && p.problems == h.check);
        assert_eq!(propose(&h, body("x", "")).await.unwrap_err(), Error::Bad("說說要怎麼改".into()));
        assert!(matches!(propose(&h, ProposeBody { doc: "Q".into(), ..body("x", "y") }).await, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn a_preset_brings_its_instruction_and_block_and_keeps_latex_locked() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = TestHost::new(dir.path(), &["The loss ⟦1⟧ falls ⟦2⟧.", "x"]);
        h.presets = vec![Preset { id: "diction".into(), label: "用字".into(), instruction: "Change word choice only.".into() }];
        h.block = "wordy: in order to → to".into();
        let mut b = body("In order to see it, the loss $L$ drops \\cite{a}.", "");
        b.preset = Some("diction".into());
        b.scope = "Results".into();
        let p = propose(&h, b.clone()).await.unwrap();
        assert!(p.ok, "{:?}", p.problems);
        assert_eq!(p.after, "The loss $L$ falls \\cite{a}.", "math and citation restored");
        let sent = h.seen.lock().unwrap()[0].flatten();
        assert!(sent.contains("For this rewrite (用字):\nwordy: in order to → to [diction in Results]"), "{sent}");
        assert!(sent.contains("Instruction: Change word choice only."));
        assert!(sent.contains("⟦1⟧") && !sent.contains("$L$") && !sent.contains("\\cite{a}"), "LaTeX still goes out locked");
        // logged by its label, never the block; a liked rewrite keeps only that
        let e = &log::read(&h.log)[0];
        assert_eq!(e.instruction, "「用字」");
        like(&h, "P", &e.id).await.unwrap();
        let lib = std::fs::read_to_string(dir.path().join("private/shared/polish/liked.jsonl")).unwrap();
        assert!(lib.contains("「用字」") && !lib.contains("wordy:"));
        // the author's own words go with it
        b.instruction = "keep 'loss'".into();
        propose(&h, b.clone()).await.unwrap();
        assert!(h.seen.lock().unwrap()[1].flatten().contains("Change word choice only.\nAlso: keep 'loss'"));
        assert_eq!(log::read(&h.log)[1].instruction, "「用字」keep 'loss'");
        b.preset = Some("nope".into());
        assert!(matches!(propose(&h, b).await, Err(Error::Bad(m)) if m.contains("nope")));
    }
}
