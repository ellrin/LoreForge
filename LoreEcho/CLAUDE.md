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
  `full/<state>.*` full bodies; missing state → idle, no full body → the portrait, no character → built-in;
  `figure_scale` / `figure_offset_y` clamped, sent as `figure`), `chat.rs` (transcript checks, system prompt =
  persona + name + `BREVITY_RULE`, SSE `start/delta/done/error`), `llm.rs` (task `le-chat` from
  `tasks.toml`, class `ask`: `local:current` then `claude:selected`; local engine =
  `LOREECHO_LLM_URL` else `private/shared/engine.toml`; keys via `SharedKeys::for_app("LoreEcho")`).
  Theme, `/prefs.css`, lore-shell `FILES` and the lore-engines router exactly as in KnowledgeBase.
  `DesktopHook`: `PUT /api/window {expanded}` and `PUT /api/settings {shortcut}` reach the desktop app.
- `apps/desktop` (`loreecho`): server in-process; one frameless, transparent, always-on-top window
  (64 px bubble ⇄ 620×620 open window, `WIN_W`/`WIN_H` = `--win-w`/`--win-h` in `web/styles.css`
  (a test checks), sharing the bottom-right corner, kept inside the monitor's work area); moves saved debounced as the bubble's place; tray item (left click = show/hide, menu =
  show/hide, quit); global shortcut (`tauri-plugin-global-shortcut`, from settings.toml);
  macOS: `macOSPrivateApi`, accessory activation policy (no Dock icon). No `lore-le://` URL scheme: other apps never open LoreEcho (author, 2026-11-11).
  `capabilities/default.json` lets the page on `http://127.0.0.1:*` call `startDragging` only.
  Icon source `icons/logo.svg` (= `docs/assets/loreecho-logo-draft.svg`, approved; do not redesign).
- `web/`: no build step. Open window = `.figure` (left, only the picture, `--fig-scale`/`--fig-dy`
  from `figure`, may run off the bottom, overlaps the column by ≤ `--overlap`) + `.col` (right): card
  with the shared shell header (collapse · engine chip · 功能列表 toggle · 新對話 · gear with 一般 外觀
  模型 快捷鍵) and the quiet history, `.choices` (功能列表 as VN choice rows, ↑↓/Enter, disabled
  「下一階段」 until phase 4), `.box` (name tag = character switcher, the current line, input row).
  `styles.css` uses theme tokens only, the character pictures are shown as they are. The conversation lives only in the page (`S.msgs`); collapsing or
  新對話 clears it. Mic button is a disabled placeholder for the voice phase.
- `../private-example/LoreEcho/`: one neutral character `echo` (avatars = the icon's outline).

## Rules

- The server never writes a conversation anywhere. Tests use temp dirs (`Home::at`), the real
  `private-example/LoreEcho` copied in, `MockProvider`/`MockFactory`, `router_isolated`; never real
  Claude, the `claude` CLI, llama-server or the network.
- Transparency, the tray, the global shortcut, dragging from the page and multi-monitor placement
  are **not tested on real hardware** (Linux CI only type-checks the desktop crate).
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
- Replies to the author in Traditional Chinese (繁體中文).
