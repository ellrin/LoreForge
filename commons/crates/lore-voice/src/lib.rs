//! lore-voice: listening (speech → text) and speaking (text → speech) for LoreForge apps.
//!
//! One interface, [`Voice`], over a [`Backend`]:
//! - macOS: [`helper::HelperBackend`], a small Swift program (`swift/helper.swift`, built by
//!   `build.rs` with `swiftc`) spoken to in JSON lines. Listening uses Apple's
//!   `SpeechAnalyzer`/`SpeechTranscriber` when the language is supported and installed, else
//!   `SFSpeechRecognizer` (on-device when it can); speaking uses `AVSpeechSynthesizer` and the
//!   system voices.
//! - tests: [`fake::FakeBackend`] (scripted events, no audio).
//! - elsewhere: [`Voice::unavailable`].
//!
//! Everything that happens comes out as [`Event`]s on [`Voice::subscribe`]; the voice also
//! counts the utterances it queued and adds [`Event::Speaking`] when speech starts and when the
//! last queued sentence is done (or stopped). [`Speaker`] speaks a streamed reply sentence by
//! sentence ([`split::Splitter`]).
//!
//! The Apple engines are **not tested on real hardware** by `cargo test` (tests use the fake);
//! listing voices, synthesising to a file and transcribing that file were checked by hand on a Mac.

pub mod fake;
pub mod helper;
pub mod split;
#[cfg(feature = "web")]
pub mod web;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub use split::Splitter;
pub use lore_shell::permissions::{Permission, Status};

/// The three permissions voice uses (see [`Voice::permissions`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoicePermissions {
    pub microphone: Status,
    pub speech: Status,
    pub personal_voice: Status,
}

impl VoicePermissions {
    pub const UNSUPPORTED: VoicePermissions = VoicePermissions { microphone: Status::Unsupported, speech: Status::Unsupported, personal_voice: Status::Unsupported };
    /// The permissions voice covers, in the order they are asked.
    pub const COVERS: [Permission; 3] = [Permission::Microphone, Permission::Speech, Permission::PersonalVoice];

    pub fn get(&self, p: Permission) -> Status {
        match p {
            Permission::Microphone => self.microphone,
            Permission::Speech => self.speech,
            Permission::PersonalVoice => self.personal_voice,
        }
    }
}

impl From<PersonalVoice> for Status {
    fn from(p: PersonalVoice) -> Status {
        match p {
            PersonalVoice::Authorized => Status::Granted,
            PersonalVoice::Denied => Status::Denied,
            PersonalVoice::NotDetermined => Status::NotDetermined,
            PersonalVoice::Unsupported => Status::Unsupported,
        }
    }
}

/// A system voice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceInfo {
    pub id: String,
    pub name: String,
    /// BCP 47, e.g. `zh-TW`.
    pub language: String,
    pub quality: Quality,
    /// The author's own Personal Voice (made in System Settings → Accessibility → Personal Voice).
    #[serde(default)]
    pub personal: bool,
}

/// Whether this app may speak with the author's Personal Voice (macOS 14+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalVoice {
    /// Not asked yet: [`Voice::personal_voice`] with `ask` shows the system's prompt.
    NotDetermined,
    Denied,
    /// Not on this Mac (older macOS, or no engine).
    Unsupported,
    /// Allowed; [`Voice::voices`] lists the voices made (none until one is made in System Settings).
    Authorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Default,
    Enhanced,
    Premium,
}

/// Voices in the order a picker shows them: Personal Voices first, then `language` (e.g. zh-TW), then the same
/// language family (zh-*), then the rest; better quality first, Apple's own voices before the
/// Eloquence ones (Eddy, Flo, …), then by name.
pub fn sort_voices(v: &mut [VoiceInfo], language: &str) {
    let family = language.split('-').next().unwrap_or("").to_string();
    let rank = |x: &VoiceInfo| {
        if x.language.eq_ignore_ascii_case(language) {
            0
        } else if x.language.split('-').next() == Some(family.as_str()) {
            1
        } else {
            2
        }
    };
    let retro = |x: &VoiceInfo| x.id.contains(".eloquence.");
    v.sort_by(|a, b| b.personal.cmp(&a.personal).then(rank(a).cmp(&rank(b))).then(b.quality.cmp(&a.quality)).then(retro(a).cmp(&retro(b))).then(a.language.cmp(&b.language)).then(a.name.cmp(&b.name)));
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListenOptions {
    /// BCP 47 (`zh-TW`).
    pub language: String,
    /// true: keep listening, every pause ends a sentence (one [`Event::Final`] each); false
    /// (press and hold): partials grow until [`Voice::stop_listening`], then one final.
    pub continuous: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakRequest {
    pub text: String,
    /// A voice id from [`Voice::voices`]; empty = the system's voice for `language`.
    #[serde(default)]
    pub voice: String,
    /// 1.0 = normal speed (clamped to [`RATE`]).
    #[serde(default = "one")]
    pub rate: f64,
    #[serde(default)]
    pub language: String,
    /// Voice pitch, 1.0 = the voice's own (a multiplier, clamped to [`PITCH`]).
    #[serde(default = "one")]
    pub pitch: f64,
}

fn one() -> f64 {
    1.0
}

impl Default for SpeakRequest {
    fn default() -> Self {
        SpeakRequest { text: String::new(), voice: String::new(), rate: 1.0, language: String::new(), pitch: 1.0 }
    }
}

/// Speaking speed limits (a multiplier of the system's normal rate).
pub const RATE: (f64, f64) = (0.5, 2.0);

pub fn clamp_rate(r: f64) -> f64 {
    if r.is_finite() { r.clamp(RATE.0, RATE.1) } else { 1.0 }
}

/// Pitch limits (a multiplier of the voice's own pitch; AVSpeechUtterance.pitchMultiplier).
pub const PITCH: (f64, f64) = (0.5, 2.0);

pub fn clamp_pitch(p: f64) -> f64 {
    if p.is_finite() { p.clamp(PITCH.0, PITCH.1) } else { 1.0 }
}

/// Why listening or speaking cannot go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    /// The microphone permission was refused (System Settings → Privacy → Microphone).
    Microphone,
    /// The speech recognition permission was refused.
    Speech,
    /// No engine here (no helper, no language, not macOS).
    Unavailable,
    Failed,
}

/// What the backend reports, as the page sees it (`type` = the variant in snake_case).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ListenStarted { engine: String },
    /// The words so far (replaces the previous partial).
    Partial { text: String },
    /// A finished sentence (continuous) or everything said (press and hold; may be empty).
    Final { text: String },
    ListenEnded,
    /// true when the first queued sentence starts, false when the last one ends or is stopped.
    Speaking { on: bool },
    SpeakStarted { id: u64 },
    Word { id: u64, start: usize, len: usize },
    SpeakFinished { id: u64, cancelled: bool },
    /// Something worth one line (e.g. a recognition model is downloading).
    Notice { message: String },
    Error { kind: ErrorKind, message: String },
}

/// Where a backend sends its events.
pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// An engine. Calls return quickly; results come through the sink given to [`Backend::start`].
pub trait Backend: Send + Sync + 'static {
    /// Called once, before anything else.
    fn start(&self, sink: Sink);
    fn name(&self) -> &str;
    fn voices(&self) -> Result<Vec<VoiceInfo>, String>;
    fn listen(&self, o: &ListenOptions) -> Result<(), String>;
    fn stop_listening(&self);
    /// Queue one utterance (it plays after the ones before it).
    fn speak(&self, id: u64, r: &SpeakRequest) -> Result<(), String>;
    /// Stop now and drop the queue.
    fn stop_speaking(&self);
    /// Synthesise `r` with a system voice into an audio file at `path`, no sound (blocking).
    fn synth_file(&self, _r: &SpeakRequest, _path: &std::path::Path) -> Result<(), String> {
        Err("這個語音引擎不能存成檔案".into())
    }
    /// Personal Voice permission; `ask`: show the system's prompt when it was never asked.
    fn personal_voice(&self, _ask: bool) -> Result<PersonalVoice, String> {
        Ok(PersonalVoice::Unsupported)
    }
    /// The three permissions; `ask`: first show the system's prompt, one after another, for each
    /// of these that was never asked (answered ones are never asked again; blocks meanwhile).
    fn permissions(&self, _ask: &[Permission]) -> Result<VoicePermissions, String> {
        Ok(VoicePermissions::UNSUPPORTED)
    }
    /// Get ready to speak with `r.voice` soon (start the engine, load the voice), silently and
    /// without waiting.
    fn warm(&self, _r: &SpeakRequest) {}
}

struct Inner {
    backend: Option<Box<dyn Backend>>,
    why: String,
    tx: broadcast::Sender<Event>,
    /// Utterances queued and not finished.
    pending: Mutex<BTreeSet<u64>>,
    next: AtomicU64,
    listening: AtomicBool,
}

impl Inner {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
    }

    /// Every backend event passes here: utterance bookkeeping and the [`Event::Speaking`] edges.
    fn on_event(&self, e: Event) {
        match &e {
            Event::SpeakFinished { id, .. } => {
                let now_empty = {
                    let mut p = self.pending.lock().unwrap();
                    p.remove(id) && p.is_empty()
                };
                self.send(e);
                if now_empty {
                    self.send(Event::Speaking { on: false });
                }
                return;
            }
            Event::SpeakStarted { id } if !self.pending.lock().unwrap().contains(id) => return, // stopped already
            Event::ListenStarted { .. } => self.listening.store(true, Ordering::SeqCst),
            Event::ListenEnded => self.listening.store(false, Ordering::SeqCst),
            Event::Error { kind: ErrorKind::Microphone | ErrorKind::Speech, .. } => self.listening.store(false, Ordering::SeqCst),
            Event::Error { kind: ErrorKind::Unavailable, .. } => {
                // the engine is gone: nothing is listening or speaking any more
                self.listening.store(false, Ordering::SeqCst);
                let had = !std::mem::take(&mut *self.pending.lock().unwrap()).is_empty();
                self.send(e);
                if had {
                    self.send(Event::Speaking { on: false });
                }
                return;
            }
            _ => {}
        }
        self.send(e);
    }
}

/// The voice of an app (cheap to clone).
#[derive(Clone)]
pub struct Voice {
    inner: Arc<Inner>,
}

impl Voice {
    pub fn new(backend: Box<dyn Backend>) -> Voice {
        let v = Voice::make(Some(backend), String::new());
        let weak = Arc::downgrade(&v.inner);
        let sink: Sink = Arc::new(move |e| {
            if let Some(i) = weak.upgrade() {
                i.on_event(e);
            }
        });
        v.inner.backend.as_ref().unwrap().start(sink);
        v
    }

    /// No engine; every call says `why`.
    pub fn unavailable(why: impl Into<String>) -> Voice {
        Voice::make(None, why.into())
    }

    fn make(backend: Option<Box<dyn Backend>>, why: String) -> Voice {
        let (tx, _) = broadcast::channel(256);
        Voice { inner: Arc::new(Inner { backend, why, tx, pending: Mutex::new(BTreeSet::new()), next: AtomicU64::new(1), listening: AtomicBool::new(false) }) }
    }

    /// The Apple engines on macOS (the helper starts on first use), else [`Voice::unavailable`].
    pub fn system() -> Voice {
        match helper::HelperBackend::find() {
            Some(b) => Voice::new(Box::new(b)),
            None => Voice::unavailable(Voice::no_helper()),
        }
    }

    fn no_helper() -> &'static str {
        if cfg!(target_os = "macos") { "找不到語音輔助程式（lore-voice-helper）" } else { "語音目前只支援 macOS" }
    }

    pub fn available(&self) -> bool {
        self.inner.backend.is_some()
    }

    /// The engine's name, or why there is none.
    pub fn status(&self) -> (bool, String) {
        match &self.inner.backend {
            Some(b) => (true, b.name().to_string()),
            None => (false, self.inner.why.clone()),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.tx.subscribe()
    }

    fn backend(&self) -> Result<&dyn Backend, String> {
        self.inner.backend.as_deref().ok_or_else(|| self.inner.why.clone())
    }

    /// Chinese and English system voices (not sorted; see [`sort_voices`]). Blocks up to a few seconds.
    pub fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        self.backend()?.voices()
    }

    /// [`Backend::personal_voice`] (blocks while the system's prompt is open).
    pub fn personal_voice(&self, ask: bool) -> Result<PersonalVoice, String> {
        self.backend()?.personal_voice(ask)
    }

    /// Microphone, speech recognition and Personal Voice: where each stands (asks nothing).
    pub fn permissions(&self) -> Result<VoicePermissions, String> {
        self.backend()?.permissions(&[])
    }

    /// Ask every voice permission that was never asked, one after another (microphone, speech,
    /// Personal Voice); answered ones are left alone. Blocks while a prompt is open.
    pub fn request_permissions(&self) -> Result<VoicePermissions, String> {
        self.backend()?.permissions(&VoicePermissions::COVERS)
    }

    /// [`Voice::request_permissions`] for one of them.
    pub fn request_permission(&self, p: Permission) -> Result<Status, String> {
        if !VoicePermissions::COVERS.contains(&p) {
            return Ok(Status::Unsupported);
        }
        Ok(self.backend()?.permissions(&[p])?.get(p))
    }

    /// [`Backend::warm`]: the first sentence of the next reply starts sooner. No sound.
    pub fn warm(&self, r: &SpeakRequest) {
        if let Ok(b) = self.backend() {
            b.warm(r);
        }
    }

    /// [`Backend::synth_file`]: `r` in a system voice to an audio file (no sound; blocks).
    pub fn synth_file(&self, r: &SpeakRequest, path: &std::path::Path) -> Result<(), String> {
        self.backend()?.synth_file(r, path)
    }

    pub fn listening(&self) -> bool {
        self.inner.listening.load(Ordering::SeqCst)
    }

    pub fn listen(&self, o: &ListenOptions) -> Result<(), String> {
        self.backend()?.listen(o)
    }

    pub fn stop_listening(&self) {
        if let Ok(b) = self.backend() {
            b.stop_listening();
        }
    }

    pub fn speaking(&self) -> bool {
        !self.inner.pending.lock().unwrap().is_empty()
    }

    /// Queue `r` (after what is already queued); its id.
    pub fn speak(&self, r: &SpeakRequest) -> Result<u64, String> {
        let b = self.backend()?;
        let id = self.inner.next.fetch_add(1, Ordering::SeqCst);
        let r = SpeakRequest { rate: clamp_rate(r.rate), pitch: clamp_pitch(r.pitch), ..r.clone() };
        let first = {
            let mut p = self.inner.pending.lock().unwrap();
            p.insert(id);
            p.len() == 1
        };
        if first {
            self.inner.send(Event::Speaking { on: true });
        }
        if let Err(e) = b.speak(id, &r) {
            self.inner.on_event(Event::SpeakFinished { id, cancelled: true });
            return Err(e);
        }
        Ok(id)
    }

    /// Stop speaking now; everything queued is dropped ([`Event::SpeakFinished`] cancelled for
    /// each, then `Speaking { on: false }`).
    pub fn stop_speaking(&self) {
        let Ok(b) = self.backend() else { return };
        let ids: Vec<u64> = std::mem::take(&mut *self.inner.pending.lock().unwrap()).into_iter().collect();
        b.stop_speaking();
        for id in &ids {
            self.inner.send(Event::SpeakFinished { id: *id, cancelled: true });
        }
        if !ids.is_empty() {
            self.inner.send(Event::Speaking { on: false });
        }
    }

    /// Speak a streamed text sentence by sentence with this voice id, rate and language.
    pub fn speaker(&self, voice: &str, rate: f64, language: &str) -> Speaker {
        self.speaker_from(SpeakRequest { voice: voice.into(), rate, language: language.into(), ..Default::default() })
    }

    /// [`Voice::speaker`] with every setting of `template`; its text is ignored.
    pub fn speaker_from(&self, template: SpeakRequest) -> Speaker {
        Speaker { voice: self.clone(), split: Splitter::new(), template, stopped: false, at: 0 }
    }
}

/// A streamed reply spoken as it arrives ([`Voice::speaker`]).
pub struct Speaker {
    voice: Voice,
    split: Splitter,
    template: SpeakRequest,
    stopped: bool,
    /// Characters of the stream handed to the splitter so far (where the next piece starts).
    at: usize,
}

/// One sentence queued by a [`Speaker`]: its utterance id and where it is in the streamed text,
/// in UTF-16 units (what a web page counts), `start..end` of everything pushed; `spoken` = the
/// UTF-16 length of what is actually said (Markdown marks dropped), which [`Event::Word`]
/// ranges refer to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Said {
    pub id: u64,
    pub start: usize,
    pub end: usize,
    pub spoken: usize,
}

impl Speaker {
    /// The next piece of the reply; finished sentences are queued at once (and returned).
    pub fn push(&mut self, piece: &str) -> Vec<Said> {
        let pieces = self.split.push_pieces(piece);
        pieces.into_iter().filter_map(|p| self.say(p)).collect()
    }

    /// The reply is complete: the rest is queued.
    pub fn finish(&mut self) -> Option<Said> {
        let p = self.split.finish_piece()?;
        self.say(p)
    }

    /// Stop queueing (what is already queued is left to [`Voice::stop_speaking`]).
    pub fn cancel(&mut self) {
        self.stopped = true;
        self.split = Splitter::new();
    }

    fn say(&mut self, p: split::Piece) -> Option<Said> {
        let start = self.at;
        self.at += p.raw;
        let text = p.text?;
        if self.stopped {
            return None;
        }
        let spoken = text.encode_utf16().count();
        match self.voice.speak(&SpeakRequest { text, ..self.template.clone() }) {
            Ok(id) => Some(Said { id, start, end: self.at, spoken }),
            Err(e) => {
                self.voice.inner.send(Event::Error { kind: ErrorKind::Failed, message: e });
                self.stopped = true;
                None
            }
        }
    }
}

/// The voice as a source of the shared permission rows (`lore_shell::permissions`).
impl lore_shell::permissions::Provider for Voice {
    fn covers(&self) -> Vec<Permission> {
        VoicePermissions::COVERS.to_vec()
    }
    fn available(&self) -> bool {
        Voice::available(self)
    }
    fn status(&self, p: Permission) -> Result<Status, String> {
        Ok(self.permissions()?.get(p))
    }
    fn request(&self, p: Permission) -> Result<Status, String> {
        self.request_permission(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fake::{Call, FakeBackend};

    fn drain(rx: &mut broadcast::Receiver<Event>) -> Vec<Event> {
        let mut v = vec![];
        while let Ok(e) = rx.try_recv() {
            v.push(e);
        }
        v
    }

    #[test]
    fn speaking_edges_follow_the_queue() {
        let (fake, ctl) = FakeBackend::new();
        let v = Voice::new(Box::new(fake));
        let mut rx = v.subscribe();
        let a = v.speak(&SpeakRequest { text: "一。".into(), rate: 1.0, ..Default::default() }).unwrap();
        let b = v.speak(&SpeakRequest { text: "二。".into(), rate: 9.0, pitch: 0.1, ..Default::default() }).unwrap();
        assert!(v.speaking());
        ctl.emit(Event::SpeakStarted { id: a });
        ctl.emit(Event::SpeakFinished { id: a, cancelled: false });
        assert!(v.speaking(), "b is still queued");
        ctl.emit(Event::SpeakStarted { id: b });
        ctl.emit(Event::Word { id: b, start: 0, len: 1 });
        ctl.emit(Event::SpeakFinished { id: b, cancelled: false });
        assert!(!v.speaking());
        assert_eq!(
            drain(&mut rx),
            vec![
                Event::Speaking { on: true },
                Event::SpeakStarted { id: a },
                Event::SpeakFinished { id: a, cancelled: false },
                Event::SpeakStarted { id: b },
                Event::Word { id: b, start: 0, len: 1 },
                Event::SpeakFinished { id: b, cancelled: false },
                Event::Speaking { on: false },
            ]
        );
        // the rate was clamped before it reached the engine
        assert!(ctl.calls().iter().any(|c| matches!(c, Call::Speak(_, r) if r.text == "二。" && r.rate == RATE.1 && r.pitch == PITCH.0)));
    }

    #[test]
    fn stop_cancels_the_whole_queue_once() {
        let (fake, ctl) = FakeBackend::new();
        let v = Voice::new(Box::new(fake));
        let mut rx = v.subscribe();
        let a = v.speak(&SpeakRequest { text: "一".into(), ..Default::default() }).unwrap();
        let b = v.speak(&SpeakRequest { text: "二".into(), ..Default::default() }).unwrap();
        ctl.emit(Event::SpeakStarted { id: a });
        drain(&mut rx);
        v.stop_speaking();
        // the engine's own late cancel of `a` is not counted twice; b never starts
        ctl.emit(Event::SpeakFinished { id: a, cancelled: true });
        ctl.emit(Event::SpeakStarted { id: b });
        assert!(!v.speaking());
        let ev = drain(&mut rx);
        assert_eq!(ev.iter().filter(|e| **e == Event::Speaking { on: false }).count(), 1, "{ev:?}");
        assert!(ev.contains(&Event::SpeakFinished { id: b, cancelled: true }));
        assert!(!ev.contains(&Event::SpeakStarted { id: b }));
        assert!(ctl.calls().contains(&Call::StopSpeaking));
        // stopping when nothing is queued says nothing
        v.stop_speaking();
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn listening_flow_and_errors() {
        let (fake, ctl) = FakeBackend::new();
        let v = Voice::new(Box::new(fake));
        let mut rx = v.subscribe();
        v.listen(&ListenOptions { language: "zh-TW".into(), continuous: false }).unwrap();
        ctl.emit(Event::ListenStarted { engine: "fake".into() });
        assert!(v.listening());
        ctl.emit(Event::Partial { text: "你".into() });
        ctl.emit(Event::Partial { text: "你好".into() });
        v.stop_listening();
        ctl.emit(Event::Final { text: "你好".into() });
        ctl.emit(Event::ListenEnded);
        assert!(!v.listening());
        assert_eq!(drain(&mut rx).len(), 5);
        assert_eq!(ctl.calls()[..2], [Call::Listen(ListenOptions { language: "zh-TW".into(), continuous: false }), Call::StopListening]);
        // a refused permission ends listening
        v.listen(&ListenOptions { language: "zh-TW".into(), continuous: true }).unwrap();
        ctl.emit(Event::ListenStarted { engine: "fake".into() });
        ctl.emit(Event::Error { kind: ErrorKind::Microphone, message: "no".into() });
        assert!(!v.listening());
        let json = serde_json::to_value(Event::Error { kind: ErrorKind::Microphone, message: "x".into() }).unwrap();
        assert_eq!(json, serde_json::json!({ "type": "error", "kind": "microphone", "message": "x" }));
    }

    #[test]
    fn a_streamed_reply_is_spoken_sentence_by_sentence() {
        let (fake, ctl) = FakeBackend::new();
        let v = Voice::new(Box::new(fake));
        let mut sp = v.speaker("com.example.voice", 1.2, "zh-TW");
        for piece in ["嗯，", "這題", "不難。先看", "定義吧！最後", "一句"] {
            sp.push(piece);
        }
        let said = |ctl: &fake::FakeControl| ctl.calls().into_iter().filter_map(|c| if let Call::Speak(_, r) = c { Some(r) } else { None }).collect::<Vec<_>>();
        assert_eq!(said(&ctl).iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["嗯，這題不難。", "先看定義吧！"]);
        sp.finish();
        let all = said(&ctl);
        assert_eq!(all.last().unwrap().text, "最後一句");
        assert!(all.iter().all(|r| r.voice == "com.example.voice" && r.rate == 1.2 && r.language == "zh-TW"));
        // cancelled: nothing more is queued
        let mut sp = v.speaker("", 1.0, "zh-TW");
        sp.cancel();
        sp.push("還有嗎？好。");
        sp.finish();
        assert_eq!(said(&ctl).len(), 3);
    }

    #[test]
    fn the_speaker_says_where_each_sentence_is_in_the_stream() {
        let (fake, ctl) = FakeBackend::new();
        let v = Voice::new(Box::new(fake));
        let mut sp = v.speaker("", 1.0, "zh-TW");
        let mut said = vec![];
        for piece in ["**好**。", "---\n", "第二句，", "對吧？尾"] {
            said.extend(sp.push(piece));
        }
        said.extend(sp.finish());
        let texts: Vec<String> = ctl.calls().into_iter().filter_map(|c| if let Call::Speak(_, r) = c { Some(r.text) } else { None }).collect();
        assert_eq!(texts, ["好。", "第二句，對吧？", "尾"]);
        // "**好**。" = 0..6, "---\n" unspoken = 6..10, then 10..17 and 17..18
        assert_eq!(said, vec![Said { id: 1, start: 0, end: 6, spoken: 2 }, Said { id: 2, start: 10, end: 17, spoken: 7 }, Said { id: 3, start: 17, end: 18, spoken: 1 }]);
    }

    #[test]
    fn warming_goes_to_the_engine_and_is_harmless_without_one() {
        let (fake, ctl) = FakeBackend::new();
        let v = Voice::new(Box::new(fake));
        v.warm(&SpeakRequest { voice: "fake.zh-TW.d".into(), language: "zh-TW".into(), ..Default::default() });
        assert!(ctl.calls().contains(&Call::Warm("fake.zh-TW.d".into())));
        assert!(!v.speaking());
        Voice::unavailable("沒有").warm(&SpeakRequest::default());
    }

    #[test]
    fn unavailable_says_why() {
        let v = Voice::unavailable("沒有");
        assert_eq!(v.status(), (false, "沒有".into()));
        assert_eq!(v.speak(&SpeakRequest::default()), Err("沒有".into()));
        assert!(v.listen(&ListenOptions { language: "zh-TW".into(), continuous: false }).is_err());
        v.stop_speaking();
        v.stop_listening();
    }

    #[test]
    fn a_failing_engine_does_not_leave_the_voice_speaking() {
        let (fake, ctl) = FakeBackend::new();
        ctl.fail_speak(true);
        let v = Voice::new(Box::new(fake));
        assert!(v.speak(&SpeakRequest { text: "x".into(), ..Default::default() }).is_err());
        assert!(!v.speaking());
    }

    #[test]
    fn voices_are_sorted_for_the_picker() {
        let mk = |id: &str, lang: &str, q| VoiceInfo { id: id.into(), name: id.into(), language: lang.into(), quality: q, personal: false };
        let mut v = vec![mk("en", "en-US", Quality::Premium), mk("cn", "zh-CN", Quality::Default), mk("a.eloquence.tw", "zh-TW", Quality::Default), mk("tw", "zh-TW", Quality::Default), mk("tw+", "zh-TW", Quality::Enhanced), VoiceInfo { personal: true, ..mk("me", "en-US", Quality::Premium) }];
        sort_voices(&mut v, "zh-TW");
        assert_eq!(v.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["me", "tw+", "tw", "a.eloquence.tw", "cn", "en"]);
    }

    #[test]
    fn permissions_are_asked_only_when_never_asked() {
        use lore_shell::permissions::Provider;
        let (fake, ctl) = FakeBackend::new();
        ctl.permission(Permission::Microphone, Status::NotDetermined, Status::Granted);
        ctl.permission(Permission::Speech, Status::Denied, Status::Granted);
        ctl.personal(PersonalVoice::NotDetermined, PersonalVoice::Authorized, false);
        let v = Voice::new(Box::new(fake));
        assert_eq!(v.permissions().unwrap(), VoicePermissions { microphone: Status::NotDetermined, speech: Status::Denied, personal_voice: Status::NotDetermined });
        assert!(ctl.asked().is_empty(), "reading asks nothing");
        let after = v.request_permissions().unwrap();
        assert_eq!(after, VoicePermissions { microphone: Status::Granted, speech: Status::Denied, personal_voice: Status::Granted });
        assert_eq!(ctl.asked(), [Permission::Microphone, Permission::PersonalVoice], "in order; the denied one is never asked");
        v.request_permissions().unwrap();
        assert_eq!(ctl.asked().len(), 2, "never twice");
        // through the shared Provider
        assert_eq!(Provider::covers(&v), VoicePermissions::COVERS);
        assert!(!Provider::available(&Voice::unavailable("沒有")));
        assert!(Voice::unavailable("沒有").permissions().is_err());
    }
}
