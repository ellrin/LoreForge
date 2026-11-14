//! Voice (design §7): the shared `/api/voice*` routes of lore-voice (`lore_voice::web`), with
//! LoreEcho's characters as the defaults — a `character` in a body (or the chosen one) decides
//! the voice, rate, pitch and the language to listen in.
//!
//! A reply read aloud as it streams goes through `POST /api/chat { speak: true }` (`chat.rs`).

use std::sync::Arc;

use crate::Shared;

/// How `c` speaks (the text is filled in per sentence).
pub fn template(c: &crate::characters::Character) -> lore_voice::SpeakRequest {
    lore_voice::SpeakRequest { voice: c.voice.clone(), rate: c.voice_rate, pitch: c.voice_pitch, language: c.language.clone(), ..Default::default() }
}

/// The characters as lore-voice's defaults.
struct Characters(Shared);

impl lore_voice::web::Defaults for Characters {
    fn template(&self, who: Option<&str>) -> lore_voice::SpeakRequest {
        template(&self.0.character(who))
    }
}

/// `/api/voice*` (see `lore_voice::web`).
pub fn routes(shared: &Shared) -> axum::Router {
    lore_voice::web::router(shared.voice.clone(), Arc::new(Characters(shared.clone())))
}

/// The System Settings page for `what` (lore-voice's list).
pub fn settings_url(what: &str) -> Option<&'static str> {
    lore_voice::web::settings_url(what)
}
