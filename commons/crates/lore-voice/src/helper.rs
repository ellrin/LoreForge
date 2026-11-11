//! The macOS backend: `lore-voice-helper` (swift/helper.swift) as a child process, JSON lines
//! on stdin/stdout. Started on first use; when it exits it is started again on the next call.
//! **Not tested on real hardware by `cargo test`** (the protocol parsing is).
//!
//! Where the helper is looked for: `LORE_VOICE_HELPER` (env), next to the running program
//! (`Contents/MacOS/` in an app bundle), `../Resources/` from it, then the one `build.rs` built.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Backend, ErrorKind, Event, ListenOptions, Quality, Sink, SpeakRequest, VoiceInfo};

pub const NAME: &str = "lore-voice-helper";
/// Path of the helper `build.rs` compiled (none off macOS or without swiftc).
pub const BUILT: Option<&str> = option_env!("LORE_VOICE_HELPER_BUILT");

/// The helper program to run, if there is one.
pub fn locate() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LORE_VOICE_HELPER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for p in [dir.join(NAME), dir.join("../Resources").join(NAME)] {
            if p.is_file() {
                return Some(p);
            }
        }
    }
    BUILT.map(PathBuf::from).filter(|p| p.is_file())
}

struct Proc {
    child: Child,
    stdin: ChildStdin,
}

pub struct HelperBackend {
    path: PathBuf,
    proc: Arc<Mutex<Option<Proc>>>,
    sink: Arc<Mutex<Option<Sink>>>,
    /// Requests waiting for their answer (`req` → where to send it).
    waiting: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>,
    req: AtomicU64,
}

impl HelperBackend {
    pub fn find() -> Option<HelperBackend> {
        locate().map(HelperBackend::at)
    }

    pub fn at(path: PathBuf) -> HelperBackend {
        HelperBackend { path, proc: Arc::new(Mutex::new(None)), sink: Arc::new(Mutex::new(None)), waiting: Arc::default(), req: AtomicU64::new(1) }
    }

    fn emit(sink: &Arc<Mutex<Option<Sink>>>, e: Event) {
        let s = sink.lock().unwrap().clone();
        if let Some(s) = s {
            s(e);
        }
    }

    /// Send one command, starting the helper when it is not running.
    fn send(&self, cmd: Value) -> Result<(), String> {
        let mut guard = self.proc.lock().unwrap();
        let alive = guard.as_mut().is_some_and(|p| matches!(p.child.try_wait(), Ok(None)));
        if !alive {
            *guard = Some(self.spawn()?);
        }
        let p = guard.as_mut().unwrap();
        let line = format!("{cmd}\n");
        if let Err(e) = p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()) {
            *guard = None;
            return Err(format!("語音輔助程式沒有回應：{e}"));
        }
        Ok(())
    }

    fn spawn(&self) -> Result<Proc, String> {
        let mut child = Command::new(&self.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("無法啟動語音輔助程式：{e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let sink = self.sink.clone();
        let waiting = self.waiting.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(req) = v["req"].as_u64()
                    && let Some(tx) = waiting.lock().unwrap().remove(&req)
                {
                    let _ = tx.send(v);
                    continue;
                }
                if let Some(e) = parse(&v) {
                    HelperBackend::emit(&sink, e);
                }
            }
            // the helper ended: whatever was listening or speaking is over
            HelperBackend::emit(&sink, Event::Error { kind: ErrorKind::Unavailable, message: "語音輔助程式結束了".into() });
        });
        Ok(Proc { child, stdin })
    }

    /// A command with a `req` id and its answer.
    fn ask(&self, mut cmd: Value, timeout: Duration) -> Result<Value, String> {
        let req = self.req.fetch_add(1, Ordering::SeqCst);
        cmd["req"] = json!(req);
        let (tx, rx) = mpsc::channel();
        self.waiting.lock().unwrap().insert(req, tx);
        if let Err(e) = self.send(cmd) {
            self.waiting.lock().unwrap().remove(&req);
            return Err(e);
        }
        let v = rx.recv_timeout(timeout).map_err(|_| {
            self.waiting.lock().unwrap().remove(&req);
            "語音輔助程式沒有回應".to_string()
        })?;
        if v["ev"] == "error" {
            return Err(v["message"].as_str().unwrap_or("失敗").to_string());
        }
        Ok(v)
    }

    /// Synthesise `r` into an audio file at `path` (no sound); the number of frames written.
    pub fn synth_file(&self, r: &SpeakRequest, path: &std::path::Path) -> Result<u64, String> {
        let v = self.ask(json!({ "cmd": "synth_file", "text": r.text, "voice": r.voice, "rate": r.rate, "language": r.language, "path": path }), Duration::from_secs(60))?;
        Ok(v["frames"].as_u64().unwrap_or(0))
    }

    /// The text in an audio file (SpeechTranscriber; macOS 26+).
    pub fn transcribe_file(&self, path: &std::path::Path, language: &str) -> Result<String, String> {
        let v = self.ask(json!({ "cmd": "transcribe_file", "path": path, "language": language }), Duration::from_secs(120))?;
        Ok(v["text"].as_str().unwrap_or("").to_string())
    }

    /// `(microphone, speech)`: "granted" | "denied" | "undetermined" (asks nothing).
    pub fn permissions(&self) -> Result<(String, String), String> {
        let v = self.ask(json!({ "cmd": "permissions" }), Duration::from_secs(5))?;
        Ok((v["microphone"].as_str().unwrap_or("").into(), v["speech"].as_str().unwrap_or("").into()))
    }
}

impl Drop for HelperBackend {
    fn drop(&mut self) {
        if let Some(mut p) = self.proc.lock().unwrap().take() {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
    }
}

/// One helper line as an [`Event`] (`None`: not an event the app needs).
pub fn parse(v: &Value) -> Option<Event> {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let id = || v["id"].as_i64().filter(|i| *i >= 0).map(|i| i as u64);
    Some(match v["ev"].as_str()? {
        "listen_start" => Event::ListenStarted { engine: s("engine") },
        "partial" => Event::Partial { text: s("text") },
        "final" => Event::Final { text: s("text") },
        "listen_end" => Event::ListenEnded,
        "speak_start" => Event::SpeakStarted { id: id()? },
        "word" => Event::Word { id: id()?, start: v["start"].as_u64()? as usize, len: v["len"].as_u64()? as usize },
        "speak_end" => Event::SpeakFinished { id: id()?, cancelled: v["cancelled"].as_bool().unwrap_or(false) },
        "notice" => Event::Notice { message: s("message") },
        "error" => Event::Error {
            kind: match v["kind"].as_str() {
                Some("microphone") => ErrorKind::Microphone,
                Some("speech") => ErrorKind::Speech,
                Some("unavailable") => ErrorKind::Unavailable,
                _ => ErrorKind::Failed,
            },
            message: s("message"),
        },
        _ => return None,
    })
}

fn quality(q: &str) -> Quality {
    match q {
        "premium" => Quality::Premium,
        "enhanced" => Quality::Enhanced,
        _ => Quality::Default,
    }
}

impl Backend for HelperBackend {
    fn start(&self, sink: Sink) {
        *self.sink.lock().unwrap() = Some(sink);
    }

    fn name(&self) -> &str {
        "apple"
    }

    fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        let v = self.ask(json!({ "cmd": "voices" }), Duration::from_secs(10))?;
        Ok(v["voices"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|x| VoiceInfo {
                        id: x["id"].as_str().unwrap_or("").into(),
                        name: x["name"].as_str().unwrap_or("").into(),
                        language: x["language"].as_str().unwrap_or("").into(),
                        quality: quality(x["quality"].as_str().unwrap_or("")),
                    })
                    .filter(|x| !x.id.is_empty())
                    .collect()
            })
            .unwrap_or_default())
    }

    fn listen(&self, o: &ListenOptions) -> Result<(), String> {
        self.send(json!({ "cmd": "listen", "language": o.language, "continuous": o.continuous }))
    }

    fn stop_listening(&self) {
        let _ = self.send(json!({ "cmd": "stop_listening" }));
    }

    fn speak(&self, id: u64, r: &SpeakRequest) -> Result<(), String> {
        self.send(json!({ "cmd": "speak", "id": id, "text": r.text, "voice": r.voice, "rate": r.rate, "language": r.language }))
    }

    fn stop_speaking(&self) {
        let _ = self.send(json!({ "cmd": "stop_speaking" }));
    }

    fn synth_file(&self, r: &SpeakRequest, path: &std::path::Path) -> Result<(), String> {
        HelperBackend::synth_file(self, r, path).map(|_| ())
    }
}

impl crate::qwen::Player for HelperBackend {
    fn play(&self, id: u64, path: &std::path::Path, rate: f64, pitch: f64) -> Result<(), String> {
        self.send(json!({ "cmd": "play", "id": id, "path": path, "rate": rate, "pitch": pitch }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_lines_become_events() {
        let p = |s: &str| parse(&serde_json::from_str(s).unwrap());
        assert_eq!(p(r#"{"ev":"partial","text":"你好"}"#), Some(Event::Partial { text: "你好".into() }));
        assert_eq!(p(r#"{"ev":"speak_end","id":3,"cancelled":true}"#), Some(Event::SpeakFinished { id: 3, cancelled: true }));
        assert_eq!(p(r#"{"ev":"word","id":3,"start":2,"len":1}"#), Some(Event::Word { id: 3, start: 2, len: 1 }));
        assert_eq!(p(r#"{"ev":"error","kind":"microphone","message":"x"}"#), Some(Event::Error { kind: ErrorKind::Microphone, message: "x".into() }));
        assert_eq!(p(r#"{"ev":"speak_start","id":-1}"#), None, "an utterance the app did not queue");
        assert_eq!(p(r#"{"ev":"ready"}"#), None);
    }

    #[test]
    fn a_missing_helper_is_an_error_not_a_panic() {
        let b = HelperBackend::at(PathBuf::from("/nonexistent/lore-voice-helper"));
        assert!(b.voices().is_err());
        assert!(b.speak(1, &SpeakRequest::default()).is_err());
    }
}
