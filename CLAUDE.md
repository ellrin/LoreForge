# KnowledgeBase — notes for Claude Code

Personal knowledge base, Rust, local-first, later a Mac app (Tauri). Same structure and
habits as PaperComposer / JELearner. Design: `../../docs/ecosystem-design.md` §3 and §4.

## Commands

```sh
cargo test                              # kb-server API tests, offline
cargo run -p kb-server                  # http://127.0.0.1:7700, mounts ../private
cargo run -p kb-server -- --space <dir> --member ellrin --port 7700
cd ../../commons && cargo test -p kb-core -p ym-store
```

`cargo test` (here and for the two commons crates) must pass before every commit.

## Layout

- `../../commons/crates/ym-store`: spaces (`space.toml`, personal / household, members),
  `progress/<member>/`, content-addressed assets (iCloud Drive folder or git folder),
  `free_path` / `copy_missing` / `write_atomic`.
- `../../commons/crates/kb-core`: nodes, kinds/apps registry, tags, learning state machine,
  SQLite + FTS5 index (rusqlite 0.32, bundled — it shares libsqlite3-sys 0.30 with sqlx 0.8),
  term scanner, `CardAssistant` (LLM trait, mock), `KnowledgeBase` façade.
- `crates/server` (`kb-server`): axum on 127.0.0.1:7700; `api.rs` handlers (plain fns over
  `Shared`, can become Tauri commands), `config.rs` (args > env > config.toml > `../private`).
- `web/`: static HTML/CSS/JS, no build step; visual language from PaperComposer / JELearner
  (tokens, icons, panels). UI text in Traditional Chinese. KaTeX vendored in `web/vendor/katex`
  (copied from PaperComposer). Graph is a plain canvas force layout (`graph.js`).
- `workspace-template/`: starting layout of a personal space.
- Next: `apps/desktop` (Tauri shell), NeonDeck widget, notifications, e-mail digest.

## Decisions

- Files are the source of truth: `nodes/<id>.json` (content only), `progress/<member>/<id>.json`
  (learning state). The index lives in memory and is rebuilt at start (`POST /api/reindex`).
- Ids are `n` + ULID; renaming never renames files. Edits bump `revision` and `updated_at`;
  `PUT` with a stale `revision` returns 409. Delete = move to `trash/`.
- Every write names its space. Only `share(id, to_space)` moves content between spaces (a
  copy with a new id). Household cards never link to or record personal ids; the relation is
  kept on the personal side (`copies`).
- Familiarity: two consecutive due `remembered` (+1 d, +3 d) → known; forgot / unfamiliar
  reset; known maintenance +30/+90 (optional); stale after 180 d without activity. An early
  `remembered` is logged but does not advance (`count_early`). Intervals in config.
- Scanner: Aho-Corasick over titles + aliases, Unicode case-folding, English plural forms,
  word boundaries only for Latin names; spans in UTF-16 offsets for JavaScript.
- LLM features (draft card, candidate terms) go through `kb_core::CardAssistant`;
  `MockAssistant` until `ym-llm` exists (integration point marked in `kb-core/src/llm.rs`).
- iCloud placeholders (`.<name>.icloud`) are fetched with `brctl download` through an injected
  runner: **not tested on real hardware**. Tests never touch the real iCloud folder.

## Rules

- Never commit keys, tokens, personal data, real user paths or the user's e-mail. Nothing
  personal goes into this repo; personal cards live in `../private`.
- Example and test users: `ellrin`, then `daidai`. Never invent names that look like real people.
- Household spaces are only ever created in temp dirs by tests; the real one will live in the
  JELearner shared repo (`knowledge/`).
- Never overwrite a user's file (`free_path`, `copy_missing`); writes are atomic.
- Every frontend change (web/) is reviewed afterwards (screenshots across light/dark and
  narrow widths); new CSS uses `--accent-text` for accent text, line icons and thin borders.
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
- Replies to the author in Traditional Chinese (繁體中文).
