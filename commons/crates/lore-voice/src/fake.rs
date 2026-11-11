//! A backend for tests: records the calls, sends the events a test scripts. No audio.
//! [`FakeTts`]: a custom-voice engine for tests (writes a few bytes, can be not installed or fail).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::qwen::{DesignJob, OnProgress, Player, Progress, SynthJob, Tts, TtsStatus};
use crate::{Backend, Event, ListenOptions, Quality, Sink, SpeakRequest, VoiceInfo};

#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Listen(ListenOptions),
    StopListening,
    Speak(u64, SpeakRequest),
    /// A synthesised sentence file played (id, path, rate, pitch).
    Play(u64, PathBuf, f64, f64),
    StopSpeaking,
    SynthFile(SpeakRequest),
}

#[derive(Default)]
struct State {
    sink: Option<Sink>,
    calls: Vec<Call>,
    fail_speak: bool,
    /// Answer every speak with start + finish at once.
    auto: bool,
    synth_seconds: Option<f64>,
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

    /// Writes [`silent_wav`] (a valid wav of 0.1 s; [`FakeControl::synth_seconds`] makes it longer).
    fn synth_file(&self, r: &SpeakRequest, path: &Path) -> Result<(), String> {
        let secs = {
            let mut s = self.state.lock().unwrap();
            s.calls.push(Call::SynthFile(r.clone()));
            s.synth_seconds
        };
        let n = (24000.0 * secs.unwrap_or(0.1)) as usize;
        std::fs::write(path, crate::qwen::wav_bytes(&vec![0i16; n], 24000)).map_err(|e| e.to_string())
    }
}

impl Player for FakeBackend {
    fn play(&self, id: u64, path: &Path, rate: f64, pitch: f64) -> Result<(), String> {
        let (auto, sink) = {
            let mut s = self.state.lock().unwrap();
            s.calls.push(Call::Play(id, path.to_path_buf(), rate, pitch));
            (s.auto, s.sink.clone())
        };
        if auto && let Some(s) = sink {
            s(Event::SpeakStarted { id });
            s(Event::SpeakFinished { id, cancelled: false });
        }
        Ok(())
    }
}

#[derive(Default)]
struct TtsState {
    ready: bool,
    design: bool,
    fail: bool,
    /// Each synth waits this long (to test stopping in the middle).
    delay_ms: u64,
    synths: Vec<SynthJob>,
    designs: Vec<DesignJob>,
    installs: usize,
}

/// A [`Tts`] for tests: `synth`/`design` write a tiny wav-like file to `out`.
#[derive(Clone, Default)]
pub struct FakeTts {
    state: Arc<Mutex<TtsState>>,
}

impl FakeTts {
    /// Installed (`ready`, `design`) or not.
    pub fn new(ready: bool) -> FakeTts {
        let t = FakeTts::default();
        {
            let mut s = t.state.lock().unwrap();
            s.ready = ready;
            s.design = ready;
        }
        t
    }
    pub fn fail(&self, on: bool) {
        self.state.lock().unwrap().fail = on;
    }
    pub fn delay(&self, ms: u64) {
        self.state.lock().unwrap().delay_ms = ms;
    }
    pub fn synths(&self) -> Vec<SynthJob> {
        self.state.lock().unwrap().synths.clone()
    }
    pub fn designs(&self) -> Vec<DesignJob> {
        self.state.lock().unwrap().designs.clone()
    }
    pub fn installs(&self) -> usize {
        self.state.lock().unwrap().installs
    }
}

/// The bytes the fake writes (a valid, silent 0.1 s 24 kHz mono wav).
pub fn silent_wav() -> Vec<u8> {
    crate::qwen::wav_bytes(&vec![0i16; 2400], 24000)
}

impl Tts for FakeTts {
    fn status(&self) -> TtsStatus {
        let s = self.state.lock().unwrap();
        TtsStatus { ready: s.ready, design: s.design, missing_bytes: if s.design { 0 } else { 6_000_000_000 }, why: String::new() }
    }

    fn install(&self, progress: OnProgress, cancel: &AtomicBool) -> Result<(), String> {
        for (i, phase) in ["下載 uv", "Python", "套件", "模型"].into_iter().enumerate() {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            progress(Progress { phase: phase.into(), done: i as u64 * 25, total: 100 });
        }
        let mut s = self.state.lock().unwrap();
        s.installs += 1;
        s.ready = true;
        s.design = true;
        Ok(())
    }

    fn synth(&self, job: &SynthJob) -> Result<(), String> {
        let delay = {
            let mut s = self.state.lock().unwrap();
            s.synths.push(job.clone());
            if s.fail {
                return Err("fake: synth failed".into());
            }
            s.delay_ms
        };
        std::thread::sleep(std::time::Duration::from_millis(delay));
        std::fs::write(&job.out, silent_wav()).map_err(|e| e.to_string())
    }

    fn design(&self, job: &DesignJob) -> Result<(), String> {
        {
            let mut s = self.state.lock().unwrap();
            s.designs.push(job.clone());
            if s.fail {
                return Err("fake: design failed".into());
            }
        }
        std::fs::write(&job.out, silent_wav()).map_err(|e| e.to_string())
    }
}
