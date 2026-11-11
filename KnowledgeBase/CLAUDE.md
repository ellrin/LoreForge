# KnowledgeBase — notes for Claude Code

Personal knowledge base, Rust, local-first, later a Mac app (Tauri). Same structure and
habits as PaperComposer / JELearner. Design: `../private/shared/docs/ecosystem-design.md` (author's private notes) §3 and §4.

## Commands

```sh
cargo test                              # kb-server API tests, offline
cargo run -p kb-server                  # http://127.0.0.1:7700, mounts ../private/KnowledgeBase
cargo run -p knowledgebase              # desktop window (Tauri); not in default-members
../scripts/install-apps.sh KnowledgeBase   # build the .app, install to /Applications, Dock
cargo run -p kb-server -- --space <dir> --member ellrin --port 7700
cd ../commons && cargo test -p kb-core -p lore-store
```

`cargo test` (here and for the two commons crates) must pass before every commit.

## Layout

- `../commons/crates/lore-store`: spaces (`space.toml`, personal / household, members),
  `progress/<member>/`, content-addressed assets (iCloud Drive folder or git folder),
  `free_path` / `copy_missing` / `write_atomic`.
- `../commons/crates/kb-core`: nodes, kinds/apps registry, tags, learning state machine,
  SQLite + FTS5 index (rusqlite 0.32, bundled — it shares libsqlite3-sys 0.30 with sqlx 0.8),
  term scanner, `CardAssistant` (LLM trait, mock), `KnowledgeBase` façade.
- `crates/server` (`kb-server`): axum on 127.0.0.1:7700; `api.rs` handlers (plain fns over
  `Shared`, can become Tauri commands), `llm.rs` (lore-llm wiring for the card draft, `tasks.toml`), `config.rs` (args > env > config.toml > `../private/KnowledgeBase`),
  `reminders.rs` (settings/state in `<app dir>` = `$KB_HOME` or `<config dir>/KnowledgeBase`,
  stats, notification slots + osascript fallback via an injected `CommandRunner`, the minute
  scheduler), `digest.rs` (daily e-mail, lettre + rustls). SMTP password: keychain through
  `lore_secrets::KeyStore` (service `KnowledgeBase`, account `smtp-password`).
- `scripts/install-launchd.sh` + `scripts/launchd/kb-server.plist.template`: start at login.
- `web/`: static HTML/CSS/JS, no build step; visual language from PaperComposer / JELearner
  (icons, panels). Colours: the theme all LoreForge apps share — `/theme.css` and `/theme.js` are
  `commons/ui/theme.{css,js}` (through `lore_home::theme`), `/api/theme` reads and sets the shared
  choice (`private/shared/ui.toml`); the gear (設定) has the 外觀 picker. The card's reading sheet is the
  theme's `--paper`; domain colours `--g1…--g12` only differ between dark and light themes. UI text in Traditional Chinese. KaTeX vendored in `web/vendor/katex`
  (copied from PaperComposer). Graph (`graph.js`): Obsidian-style canvas view, Barnes–Hut force
  layout, global / local (`#/graph/<id>`), filters, playback; data from `/api/graph?full=true&missing=true`.
  Colour = a card's first domain (`--g1…--g12`, slot = the domain's place in `tags.json`), the
  same in the graph and the card list. The card list is an index: grouped by domain, one line per
  card, summary in a hover preview. The server sends web/ with `Cache-Control: no-cache` (the
  desktop WebView kept old files otherwise).
- Shared shell (`../commons/ui`, `docs/shell.md` §F): `icons.js`, `base.css`, `shell.js` are served by
  `lore_shell::FILES` (no own icons.js); load order `theme.css → base.css → styles.css`, `theme.js → icons.js →
  shell.js → app.js`. `mountShell()` in `app.js` builds the top bar: lead = `#q` + member chip, primary =
  新增知識卡, AI = 起草摘要, right = 對照表 · 匯出▾ (Markdown, 列印 of the open card) · gear. Gear tabs 一般
  (numbers, 重建索引), 外觀, 模型, 快捷鍵, 提醒 (到期通知), 郵件 (digest; forms are `<template>`s in index.html).
  Status chip only while a card is open. `confirm()` → `LoreShell.ask`, toasts → `LoreShell.toast`.
  對照表 = `<notation-panel>` (`/notation-panel.js`, `notation.rs`) over the global registry
  `private/shared/notation.json` (`GET/PUT /api/notation`). `/api/engines` is `lore-engines` (mounted in `router`; `router_isolated` + `examples/isolated_server` for tests and screenshots).
- `../private-example/KnowledgeBase/`: starting layout of a personal space (copied to `../private/KnowledgeBase` on first run).
- NeonDeck widget: `../NeonDeck/desktop/widgets/knowledge.js`.
- `apps/desktop` (`knowledgebase`): Tauri shell on the configured port (7700, what the NeonDeck
  widget reads); if a kb-server already answers `/api/health` there (launchd), it opens that one.
  `lore-kb://node/<id>` (URL scheme in `apps/desktop/Info.plist`, merged by tauri build) opens the
  app on that card — NeonDeck's card widget uses it. Icon source `icons/logo.svg` (PaperComposer's neon style).

## Decisions

- Files are the source of truth: `nodes/<id>.json` (content only), `progress/<member>/<id>.json`
  (learning state). The index lives in memory and is rebuilt at start (`POST /api/reindex`).
- Ids are `n` + ULID; renaming never renames files. Edits bump `revision` and `updated_at`;
  `PUT` with a stale `revision` returns 409. Delete = move to `trash/`; `POST /api/nodes/{id}/restore`
  moves the latest trashed copy back (the web UI's 復原 toast, no confirm dialog).
- Every write names its space. Only `share(id, to_space)` moves content between spaces (a
  copy with a new id). Household cards never link to or record personal ids; the relation is
  kept on the personal side (`copies`).
- Familiarity: two consecutive due `remembered` (+1 d, +3 d) → known; forgot / unfamiliar
  reset; known maintenance +30/+90 (optional); stale after 180 d without activity. An early
  `remembered` is logged but does not advance (`count_early`). Intervals in config.
- Scanner: Aho-Corasick over titles + aliases, Unicode case-folding, English plural forms,
  word boundaries only for Latin names; spans in UTF-16 offsets for JavaScript.
- Card draft (`POST /api/draft`): `kb_core::{draft_prompt, parse_draft, clean_source}` build/check the
  request, `crates/server/src/llm.rs` runs it through lore-llm as task `kb-summarize-card`
  (`crates/server/tasks.toml`: `local:current` then `claude:selected`; engine URL in config.toml
  `[engine]` or `KB_LLM_URL`; optional body `engine: "local"|"claude"`). Never saved; no engine = 503 with
  one line. Candidate terms still use `CardAssistant`/`MockAssistant` (tests and offline only).
- 待審區: `<space>/pending/<id>.json` (kb-core `pending`, README there), not nodes (never in list/search/review). `GET/POST /api/pending`, `POST /api/pending/{id}/accept` (edits; `merge_into` folds into an existing card), `DELETE`. UI: left nav 待審 N (hidden at 0), `#/pending`, `#/pending/<id>` = card editor in 待審 mode (確認 · 丟掉; 合併到既有卡 when title/alias matches).
- iCloud placeholders (`.<name>.icloud`) are fetched with `brctl download` through an injected
  runner: **not tested on real hardware**. Tests never touch the real iCloud folder.
- Reminders: a notification slot is handled once (NeonDeck claims it; kb-server shows it with
  `osascript` after `grace_minutes` if nobody did). The digest is sent once per local day
  (`email_last_sent` persisted), catches up once after sleep, retries 3×/day 15 min apart.
  Tests use a mock SMTP server, a fake runner and `Reminders::isolated` (no keychain, no env).
  Real Gmail, real notifications and launchd: **not tested on real hardware**.

## Rules

- Never commit keys, tokens, personal data, real user paths or the user's e-mail. Nothing
  personal goes into this repo; personal cards live in `../private/KnowledgeBase`.
- Example and test users: `ellrin`, then `daidai`. Never invent names that look like real people.
- Household spaces are only ever created in temp dirs by tests; the real one will live in the
  JELearner shared repo (`knowledge/`).
- Never overwrite a user's file (`free_path`, `copy_missing`); writes are atomic.
- Every frontend change (web/) is reviewed afterwards (screenshots across light/dark and
  narrow widths); new CSS uses `--accent-text` for accent text, line icons and thin borders.
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
- Replies to the author in Traditional Chinese (繁體中文).
