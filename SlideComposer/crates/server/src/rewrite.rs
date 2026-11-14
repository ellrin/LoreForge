//! 語句潤飾 in a deck (lore-polish, `docs/long-paper.md` §A) and the voice (lore-voice) for its
//! hold-to-talk. Not 排版用字 (`deckpolish.rs`, `polish.json`): this rewrites one selected run of
//! text by the author's instructions.
//!
//! | Method | Path | |
//! |---|---|---|
//! | * | /api/polish… | `lore_polish::web::router` (`doc` = the deck's name) |
//! | * | /api/voice… | `lore_voice::web::router` |
//!
//! The page sends the selected text of one paragraph with its equations as `$latex$` and line
//! breaks as `\n` (kept locked, `keep`); the rewrite comes back with every one of them in place
//! or it cannot be applied. The deck's registry macros are locked too. Proposals go to
//! `decks/<name>/polish-log.jsonl`; the author's tone = the example lines of his own decks
//! (`deckvoice.rs`, `voice/examples.json`), closest to the passage first, bounded.

use std::sync::Arc;

use axum::Router;

use crate::{AppState, Shared};

pub const LOG: &str = "polish-log.jsonl";
const TONE_CHARS: usize = 1200;

struct ScHost(Shared);

#[async_trait::async_trait]
impl lore_polish::Host for ScHost {
    fn app(&self) -> &str {
        "SlideComposer"
    }
    fn home(&self) -> lore_home::Home {
        self.0.home.clone()
    }
    async fn doc(&self, doc: &str, text: &str) -> Result<lore_polish::Doc, String> {
        let dir = self.0.ws.deck_dir(doc).map_err(|e| e.to_string())?;
        if !dir.is_dir() {
            return Err(format!("找不到簡報 {doc}"));
        }
        let (glob, own) = crate::notation::load(&dir, crate::global_notation(&self.0).as_deref());
        let mut macros = lore_polish::lock::macros_of(&glob);
        macros.extend(lore_polish::lock::macros_of(&own));
        let tone = if text.is_empty() { String::new() } else { crate::deckvoice::load(&self.0.home).map(|v| v.block(text, if mostly_latin(text) { "en" } else { "zh" }, TONE_CHARS)).unwrap_or_default() };
        Ok(lore_polish::Doc { log: dir.join(LOG), macros, language: String::new(), phrases: vec![], tone, style: String::new() })
    }
    async fn complete(&self, req: lore_llm::LlmRequest, engine: Option<&str>) -> Result<(serde_json::Value, String), String> {
        let a = llm(&self.0).complete_json(lore_polish::TASK, req, engine.filter(|e| !e.is_empty())).await.map_err(|(m, _)| m)?;
        Ok((a.value, a.engine))
    }
}

fn llm(s: &AppState) -> &crate::llm::Llm {
    crate::todeck::llm(s)
}

fn mostly_latin(s: &str) -> bool {
    let han = s.chars().filter(|c| ('\u{3400}'..='\u{9fff}').contains(c)).count();
    han * 3 <= s.chars().filter(|c| !c.is_whitespace()).count()
}

/// `/api/polish*` and `/api/voice*` over `voice`.
pub fn router(shared: &Shared, voice: lore_voice::Voice) -> Router {
    let def = lore_voice::SpeakRequest { language: "zh-TW".into(), ..Default::default() };
    lore_polish::web::router(Arc::new(ScHost(shared.clone()))).merge(lore_voice::web::router(voice, Arc::new(def)))
}
