//! A backend for tests: records the calls, sends the events a test scripts. No audio.

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::{Backend, Event, ListenOptions, Permission, Quality, Sink, SpeakRequest, Status, VoiceInfo, VoicePermissions};

#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Listen(ListenOptions),
    StopListening,
    Speak(u64, SpeakRequest),
    StopSpeaking,
    SynthFile(SpeakRequest),
    /// The voice id warmed.
    Warm(String),
}

#[derive(Default)]
struct State {
    sink: Option<Sink>,
    calls: Vec<Call>,
    fail_speak: bool,
    /// Answer every speak with start + finish at once.
    auto: bool,
    synth_seconds: Option<f64>,
    /// Personal Voice: the state, and what the prompt answers.
    personal: Option<(crate::PersonalVoice, crate::PersonalVoice)>,
    /// A Personal Voice has been made (listed once authorized).
    personal_made: bool,
    /// Microphone and speech: the state, and what the prompt answers (missing = unsupported).
    perms: Vec<(Permission, Status, Status)>,
    /// Every prompt shown, in order.
    asked: Vec<Permission>,
}

pub struct FakeBackend {
    state: Arc<Mutex<State>>,
}

/// The test's side of a [`FakeBackend`].
#[derive(Clone)]
pub struct FakeControl {
    state: Arc<Mutex<State>>,
}

impl FakeBackend {
    pub fn new() -> (FakeBackend, FakeControl) {
        let state = Arc::new(Mutex::new(State::default()));
        (FakeBackend { state: state.clone() }, FakeControl { state })
    }

    /// Speaking finishes at once (start + finish for every utterance).
    pub fn instant() -> (FakeBackend, FakeControl) {
        let (b, c) = FakeBackend::new();
        c.state.lock().unwrap().auto = true;
        (b, c)
    }
}

impl FakeControl {
    /// Send `e` as if the engine had.
    pub fn emit(&self, e: Event) {
        let sink = self.state.lock().unwrap().sink.clone();
        if let Some(s) = sink {
            s(e);
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    /// How long the files of `synth_file` are.
    pub fn synth_seconds(&self, s: f64) {
        self.state.lock().unwrap().synth_seconds = Some(s);
    }

    /// Personal Voice starts as `now`; the prompt answers `answer`; `made`: the author has made one.
    pub fn personal(&self, now: crate::PersonalVoice, answer: crate::PersonalVoice, made: bool) {
        let mut s = self.state.lock().unwrap();
        s.personal = Some((now, answer));
        s.personal_made = made;
    }

    /// Microphone or speech starts as `now`; its prompt answers `answer`.
    pub fn permission(&self, p: Permission, now: Status, answer: Status) {
        let mut s = self.state.lock().unwrap();
        s.perms.retain(|x| x.0 != p);
        s.perms.push((p, now, answer));
    }

    /// The prompts shown so far, in order.
    pub fn asked(&self) -> Vec<Permission> {
        self.state.lock().unwrap().asked.clone()
    }

    pub fn fail_speak(&self, on: bool) {
        self.state.lock().unwrap().fail_speak = on;
    }
}

/// The voices the fake lists.
pub fn voices() -> Vec<VoiceInfo> {
    let mk = |id: &str, name: &str, lang: &str, q| VoiceInfo { id: id.into(), name: name.into(), language: lang.into(), quality: q, personal: false };
    vec![
        mk("fake.en-US.a", "Alpha", "en-US", Quality::Enhanced),
        mk("fake.zh-CN.b", "Beta", "zh-CN", Quality::Default),
        mk("fake.zh-TW.c", "Gamma", "zh-TW", Quality::Default),
        mk("fake.zh-TW.d", "Delta", "zh-TW", Quality::Premium),
        mk("com.apple.eloquence.zh-TW.Eddy", "Eddy", "zh-TW", Quality::Default),
    ]
}

impl Backend for FakeBackend {
    fn start(&self, sink: Sink) {
        self.state.lock().unwrap().sink = Some(sink);
    }

    fn name(&self) -> &str {
        "fake"
    }

    fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        let s = self.state.lock().unwrap();
        let mut v = voices();
        if s.personal_made && matches!(s.personal, Some((crate::PersonalVoice::Authorized, _))) {
            v.push(VoiceInfo { id: "fake.personal.me".into(), name: "My Voice".into(), language: "en-US".into(), quality: Quality::Premium, personal: true });
        }
        Ok(v)
    }

    fn personal_voice(&self, ask: bool) -> Result<crate::PersonalVoice, String> {
        let mut s = self.state.lock().unwrap();
        let Some((now, answer)) = s.personal else { return Ok(crate::PersonalVoice::Unsupported) };
        if ask && now == crate::PersonalVoice::NotDetermined {
            s.personal = Some((answer, answer));
            return Ok(answer);
        }
        Ok(now)
    }

    fn permissions(&self, ask: &[Permission]) -> Result<VoicePermissions, String> {
        let mut s = self.state.lock().unwrap();
        for p in VoicePermissions::COVERS.into_iter().filter(|p| ask.contains(p)) {
            if p == Permission::PersonalVoice {
                if let Some((crate::PersonalVoice::NotDetermined, answer)) = s.personal {
                    s.asked.push(p);
                    s.personal = Some((answer, answer));
                }
            } else if let Some(e) = s.perms.iter_mut().find(|x| x.0 == p && x.1 == Status::NotDetermined) {
                e.1 = e.2;
                s.asked.push(p);
            }
        }
        let get = |p: Permission| s.perms.iter().find(|x| x.0 == p).map(|x| x.1).unwrap_or(Status::Unsupported);
        Ok(VoicePermissions { microphone: get(Permission::Microphone), speech: get(Permission::Speech), personal_voice: s.personal.map(|x| x.0.into()).unwrap_or(Status::Unsupported) })
    }

    fn listen(&self, o: &ListenOptions) -> Result<(), String> {
        self.state.lock().unwrap().calls.push(Call::Listen(o.clone()));
        Ok(())
    }

    fn stop_listening(&self) {
        self.state.lock().unwrap().calls.push(Call::StopListening);
    }

    fn speak(&self, id: u64, r: &SpeakRequest) -> Result<(), String> {
        let (auto, sink) = {
            let mut s = self.state.lock().unwrap();
            if s.fail_speak {
                return Err("fake: speak failed".into());
            }
            s.calls.push(Call::Speak(id, r.clone()));
            (s.auto, s.sink.clone())
        };
        if auto && let Some(s) = sink {
            s(Event::SpeakStarted { id });
            s(Event::SpeakFinished { id, cancelled: false });
        }
        Ok(())
    }

    fn stop_speaking(&self) {
        self.state.lock().unwrap().calls.push(Call::StopSpeaking);
    }

    fn warm(&self, r: &SpeakRequest) {
        self.state.lock().unwrap().calls.push(Call::Warm(r.voice.clone()));
    }

    /// Writes [`silent_wav`] (a valid wav of 0.1 s; [`FakeControl::synth_seconds`] makes it longer).
    fn synth_file(&self, r: &SpeakRequest, path: &Path) -> Result<(), String> {
        let secs = {
            let mut s = self.state.lock().unwrap();
            s.calls.push(Call::SynthFile(r.clone()));
            s.synth_seconds
        };
        let n = (24000.0 * secs.unwrap_or(0.1)) as usize;
        std::fs::write(path, wav_bytes(&vec![0i16; n], 24000)).map_err(|e| e.to_string())
    }
}

/// A 16-bit mono PCM wav.
pub fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}
