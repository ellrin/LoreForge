//! A backend for tests: records the calls, sends the events a test scripts. No audio.

use std::sync::{Arc, Mutex};

use crate::{Backend, Event, ListenOptions, Quality, Sink, SpeakRequest, VoiceInfo};

#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Listen(ListenOptions),
    StopListening,
    Speak(u64, SpeakRequest),
    StopSpeaking,
}

#[derive(Default)]
struct State {
    sink: Option<Sink>,
    calls: Vec<Call>,
    fail_speak: bool,
    /// Answer every speak with start + finish at once.
    auto: bool,
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

    pub fn fail_speak(&self, on: bool) {
        self.state.lock().unwrap().fail_speak = on;
    }
}

/// The voices the fake lists.
pub fn voices() -> Vec<VoiceInfo> {
    let mk = |id: &str, name: &str, lang: &str, q| VoiceInfo { id: id.into(), name: name.into(), language: lang.into(), quality: q };
    vec![
        mk("fake.en-US.a", "Alpha", "en-US", Quality::Enhanced),
        mk("fake.zh-CN.b", "Beta", "zh-CN", Quality::Default),
        mk("fake.zh-TW.c", "Gamma", "zh-TW", Quality::Default),
        mk("fake.zh-TW.d", "Delta", "zh-TW", Quality::Premium),
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
        Ok(voices())
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
}
