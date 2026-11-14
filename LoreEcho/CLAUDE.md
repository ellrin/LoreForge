# LoreEcho — notes for Claude Code

Personal learning companion: a small round bubble on the desktop that opens into a
visual-novel-like window: the character's full body on the left, the conversation on the right. Rust server + Tauri shell, same structure and habits as
KnowledgeBase / LoreReader. Design: `../docs/loreecho.md` (phase 2 = this skeleton).

## Commands

```sh
cargo test                       # le-server: unit + API tests, offline
cargo run -p le-server           # http://127.0.0.1:7860, data ../private/LoreEcho
cargo run -p loreecho            # desktop bubble (Tauri); not in default-members
cargo test -p loreecho           # geometry, Info.plist, capability (needs the WebView libraries)
LOREECHO_VOICE=1 cargo run -p le-server --example isolated_server -- <empty dir> 7987   # UI with the real voice
LOREECHO_VOICE=paced cargo run -p le-server --example isolated_server -- <empty dir> 7987 --mock "<reply>"   # silent voice with word events (文字跟著聲音)
cargo run -p lore-voice --example check_mac -- <dir>   # (in ../commons) voices, synth → file → transcript, no mic
LOREECHO_LLM_URL=http://127.0.0.1:8080/v1 cargo run -p le-server   # a local OpenAI-compatible server
```

Before a commit, the tests of the code you changed must pass (affected scope only, see the root CLAUDE.md).

## Layout

- `crates/server` (`le-server`): axum on 127.0.0.1 (7860 in browser mode, a free port in the
  desktop app, registered as `le` in `ports.json`). `api.rs` handlers (route table in `lib.rs`),
  `store.rs` (`state.toml` = bubble place + expanded, `settings.toml` = character + shortcut + `talk` (hold|continuous) + `auto_send` + `read_aloud` + `text_follows` (文字跟著聲音);
  atomic, unknown keys kept), `characters.rs` (`characters/<id>/character.toml` + `avatar/<state>.*` portraits +
  `full/<state>.*` full bodies; ten states, a missing one follows `chain()` (searching/waiting → thinking → idle, noting → speaking → idle, else idle), no full body → the portrait, no character → built-in; editing (`save_meta`, `save_image` with format sniffing and no scripts in SVG, `remove_image`, `create` = copy of the example, `export`/`import` zip, only known paths read);
  `figure_scale` / `figure_offset_y` clamped, sent as `figure`; `voice`, `voice_rate` (0.5–2), `voice_pitch` (0.5–2, AVSpeechUtterance pitchMultiplier); `Profile` = 角色設定
  (background, speech_style, catchphrases, call_user, relationship, likes, dislikes, examples; `clean()` limits);
  theme song `theme.<ext>` sniffed by `sniff_audio`, `theme_loop`/`theme_volume`, in export/import), `chat.rs` (transcript checks, system prompt =
  persona + name + 角色設定 lines (background cut at `BACKGROUND_LIMIT`) + the about-yourself rule + `BREVITY_RULE`, example pairs as
  earlier turns; `speak: true` = the deltas also go to a lore-voice `Speaker`; SSE `search/cards/start/delta/say/done/error`, `say {id,start,end,spoken}` = where a queued sentence is in the reply (UTF-16 units)),
  `voice.rs` (`/api/voice*` = lore-voice's shared router `lore_voice::web::router`, with the characters as its `Defaults`: status, voices, events SSE, listen/stop, speak/stop (`pitch`), open-settings (microphone, speech,
  personal-voice, voices; only with the real engine), warm (the open window loads the character's voice silently), personal
  (Personal Voice status; POST = 使用個人聲音, the system's prompt only when never asked); `template()` = how a character speaks;
  `AppState.voice` = `Voice::system()` in `system()`, unavailable in `new()`, `with_voice()` for fakes; a `voice` id in
  `/api/voice/speak` = that system voice (試聽 before saving)). Voices are system voices only (author, 2026-11-12: no Qwen3-TTS);
  a character folder's old `voice/` (an earlier custom voice) is ignored, not exported, and the character speaks with its system voice. `kbctx.rs` (design §3: the author's KnowledgeBase cards read from disk with kb-core, never written from here (修正 goes through KnowledgeBase, see `fix.rs`); per turn titles/aliases in the message and recent turns + key words → a few cards within `BUDGET_CHARS` into the prompt as his own cards with their state; 抽／複習／考我／有哪些卡／記住這個 (zh/en, counts) chosen in code, the model only phrases; quiz carried by the page (`kb.quiz`, `kb.cited`); review outcomes only through KnowledgeBase's `POST /api/recall/{id}` while it runs (`POST /api/kb/recall`); `AppState::new` reads only its home's `private/KnowledgeBase`, `system()` also KB_SPACES/KB config/its server; tests pin a space, `tests/fixtures/kb`), `llm.rs` (task `le-chat` from
  `tasks.toml`, class `ask`: `local:current` then `claude:selected`; local engine =
  `LOREECHO_LLM_URL` else the shared model id of `private/shared/engine.toml` on the machine's one
  llama-server (lore-llm `shared_runtime`, `../docs/local-models.md`; `llm::quit()` at exit); keys via
  `SharedKeys::for_app("LoreEcho")`), `shelfctx.rs` (the author's files, only in the real app (`AppState.shelf`) and while LoreShelf runs (ports.json `ls`): per turn `GET /api/search?loose=1&q=<the message>` → SSE `files` (the 「檔案：…」 row in `kbcards.js`; a click = `POST /api/shelf/focus` → LoreShelf `POST /api/act`), the globe turned to the first one right away, and a note in the prompt; asked before the engine is chosen (`lookup`): a turn whose files are all `cloud_ok` (papers, slide decks — LoreShelf decides) keeps the chosen engine and the names; any other file → the local model when one can, whatever the chip says, and only it gets the names (`note`) — Claude is then only told that some were found), `actions.rs` (功能列表, design §4/§4a: `/api/actions/note/{draft,check,save}`,
  `/api/actions/cards/{draft,save}`; drafts = tasks `le-note` / `le-cards` (class `tidy`, one JSON reply via
  `complete_json`, the conversation as one labelled text, the author's related cards from kbctx offered by id and
  only those kept); the note's LaTeX through lore-notation (`check`: an empty registry, error = blocking, 422);
  saves: PaperComposer's `POST /api/notes` while it runs (`Pc`: ports.json `pc`, else 7878, `GET /api/notes`
  answers; pinned in tests), else the §4a file in `private/PaperComposer/notes/` (`write_note_file`, ADAPTER until
  lore-notes lands; never overwrites); cards: KnowledgeBase's `POST /api/pending` while it runs, else
  `kb_core::pending::add` on the member's personal space (only `pending/`); a refusal from a running app is shown,
  never followed by a local copy; 記下靈感 = `kind: idea`, no model, `% & #` escaped).
  `find.rs` (搜尋檔案 for another app, `lore_shell::findfile`: `POST /api/find {request, app, purpose, exts, port}` → find mode (window opens, `web/find.js` + `find.css` in place of the history; the page polls `GET /api/find`); `send()` hands the sentence to find.js, never to the chat; candidates = LoreShelf guided search then loose, ≤ 5 after the extension filter, folder via `/api/files/{id}`, thumbnails proxied (`/api/find/thumb/{id}`, none for sensitive); a click picks at once, 「第二個」/a name part highlights + 「加入這份？」 (only 對/加入 sends), 「好了」/Esc/closing ends (`/api/find/end` tells the app); pick = path from LoreShelf → the app's `POST /api/lore/picked`; nothing is opened; LoreShelf pinned in tests / `isolated_server --shelf <url>`).
  `fix.rs` (修正知識卡, author 2026-11-14): 「這張卡寫錯了／幫我修正…那張卡」 (kbctx `Intent::Fix`: the card he names, else
  the last reply's; the model only says it will check) or the 功能列表 row → `POST /api/actions/fix/draft {messages, card}`:
  the card from KnowledgeBase's `GET /api/nodes/{id}` + his own words only → task `le-fix` (Claude Haiku, web search,
  no local fallback; `allow_web` on the subscription, used only by requests that ask for it) → old and new per field,
  `changes` (what/why/url) and `sources` (http(s) only); 套用 = `POST /api/actions/fix/save` → KnowledgeBase's
  `PUT /api/nodes/{id}` with the draft's `revision` (409 = changed meanwhile, never overwritten), the web sources added
  as `{type: "web", ref, title, via: "loreecho"}`; KnowledgeBase closed → 503 and nothing written (no file fallback);
  `/api/actions/fix/source {url}` opens an http(s) source in the browser.
  Theme, `/prefs.css`, lore-shell `FILES` and the lore-engines router exactly as in KnowledgeBase.
  `DesktopHook`: `PUT /api/window {expanded}` and `PUT /api/settings {shortcut}` reach the desktop app.
- `apps/desktop` (`loreecho`): server in-process; one frameless, transparent, always-on-top window
  (64 px bubble ⇄ open window, default 620×620 = `WIN_W`/`WIN_H` = `--win-w`/`--win-h` in `web/styles.css`
  (a test checks), resizable 420–1000 px (`MIN_WIN`/`MAX_WIN` in `store.rs`; the page's left/top grips send `PUT /api/window {w,h}` → `DesktopEvent::Size`, the window keeps its bottom-right corner; `w`,`h` saved in state.toml), sharing the bottom-right corner, kept inside the monitor's work area); moves saved debounced as the bubble's place; tray item (left click = show/hide, menu =
  show/hide, quit); global shortcut (`tauri-plugin-global-shortcut`, from settings.toml);
  macOS: `macOSPrivateApi`, regular activation policy (Dock icon, installed into the Dock by `scripts/install-apps.sh`; Dock click = `RunEvent::Reopen` → show/unminimize; own menu bar: app 隱藏/結束 ⌘Q, 編輯, 視窗 最小化 ⌘M as custom items). No `lore-le://` URL scheme. LoreShelf's 開啟時同步開啟 LoreEcho (author, 2026-11-13) launches the app with `open -g -a LoreEcho` and seats the window through `PUT /api/window {x, y, w, h, expanded}`.
  `capabilities/default.json` lets the page on `http://127.0.0.1:*` call `startDragging` only.
  Icon source `icons/logo.svg` (= `docs/assets/loreecho-logo-draft.svg`, approved; do not redesign).
- `web/`: no build step. Open window = `.figure` (left, only the picture, `--fig-scale`/`--fig-dy`
  from `figure`, may run off the bottom, overlaps the column by ≤ `--overlap`) + `.col` (right): card
  with the shared shell header (collapse · engine chip · 功能列表 toggle · 新對話 · gear with 一般 外觀
  模型 快捷鍵 人物; 人物 = name, persona, per-state drop zones for 縮圖 and 對話人物, switch/new/export/import) and the quiet history, `.choices` (功能列表 as VN choice rows, ↑↓/Enter; a row opens
  its preview from `actions.js` + `actions.css` in place of the history, KaTeX from `web/vendor/katex` loaded on first use,
  Esc closes an untouched preview, 新對話/collapse close it, toast with 打開 = `LoreLinks.open("pc", {note})` / `("kb", {pending})`), `.box` (name tag = character switcher, the current line, input row).
  `styles.css` uses theme tokens only, the character pictures are shown as they are. The conversation lives only in the page (`S.msgs`); collapsing or
  新對話 clears it. Voice: `voice.js` (mic hold/continuous, 朗讀回覆 toggle, theme song button + ducking/fade, 一般 → 語音
  (麥克風, 說完就送出, 朗讀回覆, 文字跟著聲音), 人物 → 聲音 (個人聲音 group first, system voices of the character's language first,
  better quality marked, Eloquence voices last in 其他（機械音）) + 試聽 / 更多聲音 hint / 個人聲音 state row / 語速 / 音調 / 主題曲;
  `LoreVoice.follow` = 文字跟著聲音: the line reveals per sentence when its utterance starts, by the voice's word events (exact
  for system voices; paced at 4.5 chars/s until they come), whole on stop, cancel, the reply ending quietly or 4 s after the first queued sentence without
  any speech; the whole reply stays in `S.msgs` and goes to the history; `LoreVoice.opened` warms the voice) and `profile.js` (人物 → 角色設定), hooked from app.js through `window.LoreVoice` / `window.LoreEcho`;
  `voice.css`. Desktop: `apps/desktop/Info.plist` (mic + speech usage strings, merged by the bundler), `build.rs` copies the
  helper to `bin/` (ignored) → `bundle.macOS.files` → `Contents/MacOS/lore-voice-helper`.
  States in the page (`setState`, also `window.LoreEcho.setState / whileState`): listening while a message is typed (the voice phase reuses it for recording), thinking while waiting for the model, waiting after 8 s without a first token, speaking while streaming and while the voice reads the reply (`LoreVoice.afterReply` delays the mood), listening also while recording, happy/unsure/surprised from a keyword cue in the finished reply (`moodOf`, 4 s); searching while the server reads KnowledgeBase (SSE `search`, `web/kbcards.js`, which also draws the 「引用：…」 row under the line, opening cards with `LoreLinks.open("kb", {node})`, and 記得／忘了 after a quiz answer); noting while a 功能列表 draft is written (`whileState` in actions.js). The name tag shows the avatar of the current state.
- `../private-example/LoreEcho/`: one neutral character `echo` (avatars = the icon's outline, full bodies; all ten states in both rows).

## Rules

- The server never writes a conversation anywhere (only what the author confirms in a 功能列表 preview; a pending card keeps his first question as `from_conversation`). Tests use temp dirs (`Home::at`), the real
  `private-example/LoreEcho` copied in, `MockProvider`/`MockFactory`, `router_isolated`; never real
  Claude, the `claude` CLI, llama-server or the network.
- Transparency, the tray, the global shortcut, dragging from the page and multi-monitor placement
  are **not tested on real hardware** (Linux CI only type-checks the desktop crate). Voice: tests use
  `lore_voice::fake` (Personal Voice states scripted); the microphone, permission prompts (also Personal Voice's), live listening,
  speaking aloud, word timing and the bundled helper are **not tested on real hardware**.
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
- Replies to the author in Traditional Chinese (繁體中文).
