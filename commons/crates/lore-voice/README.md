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

Protocol: JSON lines; commands `voices`, `permissions`, `speak`, `play` (a synthesised sentence
file with rate and pitch), `stop_speaking`, `listen`, `stop_listening`, `synth_file`, `transcribe_file` (the last two: checks without the microphone).
The helper starts on first use and again after it exits; it exits when its stdin closes.

Permissions: the microphone is checked (and asked once) before listening; the speech-recognition
permission only on the `SFSpeechRecognizer` path. A refusal is `error {kind: microphone|speech}`.

## Custom voices (Qwen3-TTS, `qwen`)

A character can speak in its own voice: Qwen3-TTS through [mlx-audio](https://github.com/Blaizzy/mlx-audio)
(Apple Silicon), next to the Apple engines.

- `SpeakRequest.custom = Some(CustomVoice { audio, text, taiwan, han })`: the sentence is cloned from
  that clip (24 kHz mono wav + what is said in it); `voice` is then the fallback system voice and
  `pitch` (semitones, ±6) applies. `Voice::speaker_from(template)` carries all of it through a reply.
- `Voice::system_with(tts, cache)` = `qwen::Mixed`: listening and system voices as before; a custom
  sentence is synthesised by the `Tts` (one at a time, the next while the one before plays) and
  played by the helper (`play` command: `AVAudioEngine` + `AVAudioUnitTimePitch`, so rate keeps the
  pitch and pitch keeps the rate). Not installed, or a sentence fails → the system voice, with one
  `notice`. `stop_speaking` also drops sentences still being made.
- `qwen::Tts`: `status()` (ready, design, missing bytes, why not here), `install(progress, cancel)`,
  `synth(SynthJob)` (clone; `seed` for reproducible takes), `design(DesignJob)` (a clip from a
  description), `stop()`. `qwen::Qwen` = the real one; `fake::FakeTts` for tests.
- What is installed (`tts.json`; nothing in the repo): under the shared model folder
  (`LOCAL_LLM_HOME` / `engine.toml [local] home`) `tts/`: `uv/` (release tarball, sha256 checked),
  `python/` + `venv/` (Python 3.12 made by uv, `mlx-audio` version-pinned; pip packages are pinned by
  version, not by hash), `models/` (two mlx-community repos at pinned revisions, sizes checked:
  0.6B Base bf16 = speaking/cloning, 1.7B VoiceDesign 8-bit = designing), `lore_tts.py` (the worker,
  `python/lore_tts.py`, rewritten when it changes). Downloads use `curl` and resume a `.part`;
  cancel keeps it. Model files ≈ 5.6 GB + environment ≈ 0.6 GB.
- The worker: `venv/bin/python -I lore_tts.py`, JSON lines (`synth`, `design`, `unload`), offline
  (`HF_HUB_OFFLINE`), one model in memory (≈ 3–4 GB), started on first use, stopped after
  `qwen::IDLE` (10 min) or when the app ends. Not one of the shared llama-server's leases
  (`docs/local-models.md` §7).
- `readings::taiwan`: Taiwan readings the model would get wrong, written with characters that
  sound right (垃圾 → 樂色, 企業, 危險, 液體…; 和 → hàn only as an option).
- `qwen::ACCENT_PASSAGE`: what a Taiwan system voice reads to make an accent reference;
  `qwen::to_wav24` (afconvert), `wav_seconds`, `wav_bytes`.

Measured on an M6 32 GB Mac (`check_qwen`, scratch install): install of the environment 14 s
(models already there); with a ≈ 6 s reference clip a sentence takes ≈ 0.5× its length after the
first (1.4 s for the first short one); with a 20 s reference ≈ 1.8–2.7× (so keep references short);
four designed takes ≈ 16 s including the model load; worker peak ≈ 4.2 GB. Clips came back
intelligible to SpeechTranscriber (zh-TW). How Taiwanese the accent sounds was **not judged by ear**.

## Tested

`cargo test -p lore-voice`: event flow, the speaking edges, cancel, sentence splitting, helper
line parsing, custom voices (`qwen::Mixed` with `FakeTts`: order, pitch, stop mid-reply, not
installed / failing → system voice), Taiwan readings, wav helpers (fake backend; no audio).
`cargo run -p lore-voice --example check_qwen -- <home> <out> [--install]` on a Mac: installs into
`<home>/tts` (use a scratch folder), makes the accent reference, clones and designs, prints timings. `cargo run -p lore-voice --example check_mac -- <dir>` on a
Mac: lists voices, synthesises a sentence to a file and transcribes it back (no microphone, no
sound). The microphone, permission prompts and live listening are **not tested on real hardware**
by either.
