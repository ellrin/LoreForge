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
LOREECHO_LLM_URL=http://127.0.0.1:8080/v1 cargo run -p le-server   # a local OpenAI-compatible server
```

`cargo test` must pass before every commit.

## Layout

- `crates/server` (`le-server`): axum on 127.0.0.1 (7860 in browser mode, a free port in the
  desktop app, registered as `le` in `ports.json`). `api.rs` handlers (route table in `lib.rs`),
  `store.rs` (`state.toml` = bubble place + expanded, `settings.toml` = character + shortcut;
  atomic, unknown keys kept), `characters.rs` (`characters/<id>/character.toml` + `avatar/<state>.*` portraits +
  `full/<state>.*` full bodies; ten states, a missing one follows `chain()` (searching/waiting → thinking → idle, noting → speaking → idle, else idle), no full body → the portrait, no character → built-in; editing (`save_meta`, `save_image` with format sniffing and no scripts in SVG, `remove_image`, `create` = copy of the example, `export`/`import` zip, only known paths read);
  `figure_scale` / `figure_offset_y` clamped, sent as `figure`), `chat.rs` (transcript checks, system prompt =
  persona + name + `BREVITY_RULE`, SSE `search/cards/start/delta/done/error`), `kbctx.rs` (design §3: the author's KnowledgeBase cards read from disk with kb-core, never written; per turn titles/aliases in the message and recent turns + key words → a few cards within `BUDGET_CHARS` into the prompt as his own cards with their state; 抽／複習／考我／有哪些卡／記住這個 (zh/en, counts) chosen in code, the model only phrases; quiz carried by the page (`kb.quiz`, `kb.cited`); review outcomes only through KnowledgeBase's `POST /api/recall/{id}` while it runs (`POST /api/kb/recall`); `AppState::new` reads only its home's `private/KnowledgeBase`, `system()` also KB_SPACES/KB config/its server; tests pin a space, `tests/fixtures/kb`), `llm.rs` (task `le-chat` from
  `tasks.toml`, class `ask`: `local:current` then `claude:selected`; local engine =
  `LOREECHO_LLM_URL` else the shared model id of `private/shared/engine.toml` on the machine's one
  llama-server (lore-llm `shared_runtime`, `../docs/local-models.md`; `llm::quit()` at exit); keys via
  `SharedKeys::for_app("LoreEcho")`).
  Theme, `/prefs.css`, lore-shell `FILES` and the lore-engines router exactly as in KnowledgeBase.
  `DesktopHook`: `PUT /api/window {expanded}` and `PUT /api/settings {shortcut}` reach the desktop app.
- `apps/desktop` (`loreecho`): server in-process; one frameless, transparent, always-on-top window
  (64 px bubble ⇄ open window, default 620×620 = `WIN_W`/`WIN_H` = `--win-w`/`--win-h` in `web/styles.css`
  (a test checks), resizable 420–1000 px (`MIN_WIN`/`MAX_WIN` in `store.rs`; the page's left/top grips send `PUT /api/window {w,h}` → `DesktopEvent::Size`, the window keeps its bottom-right corner; `w`,`h` saved in state.toml), sharing the bottom-right corner, kept inside the monitor's work area); moves saved debounced as the bubble's place; tray item (left click = show/hide, menu =
  show/hide, quit); global shortcut (`tauri-plugin-global-shortcut`, from settings.toml);
  macOS: `macOSPrivateApi`, regular activation policy (Dock icon, installed into the Dock by `scripts/install-apps.sh`; Dock click = `RunEvent::Reopen` → show/unminimize; own menu bar: app 隱藏/結束 ⌘Q, 編輯, 視窗 最小化 ⌘M as custom items). No `lore-le://` URL scheme: other apps never open LoreEcho (author, 2026-11-11).
  `capabilities/default.json` lets the page on `http://127.0.0.1:*` call `startDragging` only.
  Icon source `icons/logo.svg` (= `docs/assets/loreecho-logo-draft.svg`, approved; do not redesign).
- `web/`: no build step. Open window = `.figure` (left, only the picture, `--fig-scale`/`--fig-dy`
  from `figure`, may run off the bottom, overlaps the column by ≤ `--overlap`) + `.col` (right): card
  with the shared shell header (collapse · engine chip · 功能列表 toggle · 新對話 · gear with 一般 外觀
  模型 快捷鍵 人物; 人物 = name, persona, per-state drop zones for 縮圖 and 對話人物, switch/new/export/import) and the quiet history, `.choices` (功能列表 as VN choice rows, ↑↓/Enter, disabled
  「下一階段」 until phase 4), `.box` (name tag = character switcher, the current line, input row).
  `styles.css` uses theme tokens only, the character pictures are shown as they are. The conversation lives only in the page (`S.msgs`); collapsing or
  新對話 clears it. Mic button is a disabled placeholder for the voice phase.
  States in the page (`setState`, also `window.LoreEcho.setState / whileState`): listening while a message is typed (the voice phase reuses it for recording), thinking while waiting for the model, waiting after 8 s without a first token, speaking while streaming, happy/unsure/surprised from a keyword cue in the finished reply (`moodOf`, 4 s); searching while the server reads KnowledgeBase (SSE `search`, `web/kbcards.js`, which also draws the 「引用：…」 row under the line, opening cards with `LoreLinks.open("kb", {node})`, and 記得／忘了 after a quiz answer); noting (writing a note) has no signal yet: wrap the future call in `whileState`. The name tag shows the avatar of the current state.
- `../private-example/LoreEcho/`: one neutral character `echo` (avatars = the icon's outline, full bodies; all ten states in both rows).

## Rules

- The server never writes a conversation anywhere. Tests use temp dirs (`Home::at`), the real
  `private-example/LoreEcho` copied in, `MockProvider`/`MockFactory`, `router_isolated`; never real
  Claude, the `claude` CLI, llama-server or the network.
- Transparency, the tray, the global shortcut, dragging from the page and multi-monitor placement
  are **not tested on real hardware** (Linux CI only type-checks the desktop crate).
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
- Replies to the author in Traditional Chinese (繁體中文).
