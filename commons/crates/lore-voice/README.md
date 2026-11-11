# lore-voice

Listening (speech → text) and speaking (text → speech) for LoreForge apps, behind one small
interface. First user: LoreEcho (`docs/loreecho.md` §7).

| | macOS | tests | elsewhere |
|---|---|---|---|
| backend | `helper::HelperBackend` = `lore-voice-helper` (Swift, `swift/helper.swift`) | `fake::FakeBackend` | `Voice::unavailable` |
| listen | `SpeechAnalyzer` + `SpeechTranscriber` (macOS 26+, language supported and installed; a supported one that is not installed starts downloading and the fallback is used meanwhile), else `SFSpeechRecognizer` (on-device when it can) | scripted events | — |
| speak | `AVSpeechSynthesizer`, the system voices (zh / en, novelty voices left out) | records calls | — |

## API

- `Voice::system()` / `Voice::new(Box<dyn Backend>)` / `Voice::unavailable(why)`; `status()`.
- `voices()` → `[VoiceInfo { id, name, language, quality: default|enhanced|premium }]`;
  `sort_voices(&mut v, "zh-TW")` = the picker's order (that language, its family, the rest; better
  quality first).
- `listen(&ListenOptions { language, continuous })`, `stop_listening()`. Press and hold
  (`continuous: false`): `Partial` grows, `stop_listening` → one `Final` (everything said, may be
  empty) → `ListenEnded`. Continuous: each pause ends a sentence → one `Final` each, until stopped.
- `speak(&SpeakRequest { text, voice, rate, language })` → id (queued after the others; `rate` is a
  multiplier, 1 = normal, clamped to 0.5–2); `stop_speaking()` drops the queue.
- `speaker(voice, rate, language)` → `Speaker`: `push(piece)` speaks each finished sentence of a
  streamed reply at once (`split::Splitter`: 。！？!?；… and new lines, closing quotes kept, a
  clause over 80 characters cut at its last comma, Markdown marks dropped), `finish()` the rest.
- `subscribe()` → `tokio::sync::broadcast::Receiver<Event>`. `Event` (JSON `type` in snake_case):
  `listen_started {engine}`, `partial {text}`, `final {text}`, `listen_ended`, `speaking {on}`
  (true at the first queued sentence, false after the last or a stop), `speak_started {id}`,
  `word {id,start,len}`, `speak_finished {id,cancelled}`, `notice {message}`,
  `error {kind: microphone|speech|unavailable|failed, message}`.

## The helper

`build.rs` compiles `swift/helper.swift` with `swiftc` on macOS into `OUT_DIR` (target macOS 14;
SpeechAnalyzer behind `#available(macOS 26)`), with `swift/Info.plist` embedded (the two usage
strings). No swiftc or not macOS: a warning, no helper, voice reports unavailable (never a build
failure; `LORE_VOICE_NO_HELPER=1` skips it). `links = "lore_voice"`: an app's build script reads
`DEP_LORE_VOICE_HELPER` to put the helper into its bundle. Lookup at run time: `LORE_VOICE_HELPER`,
next to the program (`Contents/MacOS/`), `../Resources/`, then the built one.

Protocol: JSON lines; commands `voices`, `permissions`, `speak`, `stop_speaking`, `listen`,
`stop_listening`, `synth_file`, `transcribe_file` (the last two: checks without the microphone).
The helper starts on first use and again after it exits; it exits when its stdin closes.

Permissions: the microphone is checked (and asked once) before listening; the speech-recognition
permission only on the `SFSpeechRecognizer` path. A refusal is `error {kind: microphone|speech}`.

## Tested

`cargo test -p lore-voice`: event flow, the speaking edges, cancel, sentence splitting, helper
line parsing (fake backend; no audio). `cargo run -p lore-voice --example check_mac -- <dir>` on a
Mac: lists voices, synthesises a sentence to a file and transcribes it back (no microphone, no
sound). The microphone, permission prompts and live listening are **not tested on real hardware**
by either.
