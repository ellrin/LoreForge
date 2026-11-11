# lore-voice

Listening (speech → text) and speaking (text → speech) for LoreForge apps, behind one small
interface. First user: LoreEcho (`docs/loreecho.md` §7).

| | macOS | tests | elsewhere |
|---|---|---|---|
| backend | `helper::HelperBackend` = `lore-voice-helper` (Swift, `swift/helper.swift`) | `fake::FakeBackend` | `Voice::unavailable` |
| listen | `SpeechAnalyzer` + `SpeechTranscriber` (macOS 26+, language supported and installed; a supported one that is not installed starts downloading and the fallback is used meanwhile), else `SFSpeechRecognizer` (on-device when it can) | scripted events | — |
| speak | `AVSpeechSynthesizer`, the system voices (zh / en, novelty voices left out) and Personal Voice | records calls | — |

## API

- `Voice::system()` / `Voice::new(Box<dyn Backend>)` / `Voice::unavailable(why)`; `status()`.
- `voices()` → `[VoiceInfo { id, name, language, quality: default|enhanced|premium }]`;
  `sort_voices(&mut v, "zh-TW")` = the picker's order (that language, its family, the rest; better
  quality first).
- `listen(&ListenOptions { language, continuous })`, `stop_listening()`. Press and hold
  (`continuous: false`): `Partial` grows, `stop_listening` → one `Final` (everything said, may be
  empty) → `ListenEnded`. Continuous: each pause ends a sentence → one `Final` each, until stopped.
- `speak(&SpeakRequest { text, voice, rate, pitch, language })` → id (queued after the others; `rate` and
  `pitch` are multipliers, 1 = normal, clamped to 0.5–2; pitch = `pitchMultiplier`); `stop_speaking()` drops the queue.
- `warm(&SpeakRequest)`: load that voice now (written to a throwaway buffer, no sound), so the first
  sentence starts sooner.
- `speaker(voice, rate, language)` / `speaker_from(template)` → `Speaker`: `push(piece)` speaks each finished sentence of a
  streamed reply at once (`split::Splitter`: 。！？!?；… and new lines, closing quotes kept, the first
  sentence cut at its first comma once 6 characters come before it, a clause over 80 characters
  cut at its last comma, Markdown marks dropped), `finish()` the rest. Both return `Said { id,
  start, end, spoken }`: the utterance and where it sits in everything pushed (UTF-16 units; `spoken`
  = the length of what is said, which `word` ranges count in), so a page can show the text as it
  is spoken. `Splitter::push_pieces` gives every cut, said or not, with its raw length.
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

Protocol: JSON lines; commands `voices`, `permissions`, `personal_voice`, `speak`, `warm`, `stop_speaking`, `listen`, `stop_listening`, `synth_file`, `transcribe_file` (the last two: checks without the microphone).
The helper starts on first use and again after it exits; it exits when its stdin closes.

Permissions: the microphone is checked (and asked once) before listening; the speech-recognition
permission only on the `SFSpeechRecognizer` path. A refusal is `error {kind: microphone|speech}`.

## Personal Voice

`VoiceInfo.personal` marks the author's own Personal Voice (macOS 14+, made in System Settings →
Accessibility → Personal Voice); `sort_voices` puts them first. `Voice::personal_voice(ask)` →
`PersonalVoice::{NotDetermined, Denied, Unsupported, Authorized}`; `ask` shows the system's prompt
(`requestPersonalVoiceAuthorization`) only when it was never asked. The system lists Personal
Voices only to an authorized process; the authorization belongs to the app the helper runs in
(LoreEcho.app when bundled; a terminal when run from one). The SDK header names no Info.plist key
for it. **Not tested on real hardware** (prompt, speaking with it).

Measured on an M6 32 GB Mac (`check_mac`): the best zh-TW voice (Meijia, compact) makes a
28-character sentence in 0.35 s in a fresh helper, 0.18 s after `warm`; the first words up to the
comma take about as long, so the gain of the first cut is not waiting for the rest of the sentence
from the language model.

## Tested

`cargo test -p lore-voice`: event flow, the speaking edges, cancel, sentence splitting (first-comma
cut, pieces in UTF-16 units), where each spoken sentence sits, rate and pitch clamping, warm,
helper line parsing, Personal Voice states (fake backend; no audio).
`cargo run -p lore-voice --example check_mac -- <dir>` on a Mac: lists voices, the Personal Voice
state (never asks), synthesises a sentence to a file and transcribes it back, times a first
sentence cold and warm (no microphone, no sound). The microphone, permission prompts, speaking
aloud and word timing are **not tested on real hardware**.
