# commons — notes for Claude Code

Shared Rust crates and web components used by the author's apps (PaperComposer,
JELearner, KnowledgeBase, SlideComposer, NeonDeck widgets, future apps). The overall
design is in `../docs/ecosystem-design.md` — read the section for the crate you touch.

## Layout

- `crates/<name>/` — one crate per module (`ym-llm`, `ym-secrets`, `ym-gitsync`, `ym-store`, `kb-core`,
  `ym-notation`, `ym-voice`, `ym-docsrc`, `ym-canvas`, `ym-ooxml`, `ym-math`).
  Each crate knows nothing about any app. Apps depend on them by path
  (`../../commons/crates/<name>` from an app's `public/` workspace).
- `ui/` — framework-free Web Components (no build step), one folder per component,
  each with a `demo.html` that works from `python3 -m http.server`.
- `docs/` — per-crate design notes and data formats when they go beyond the main design doc.

## Commands

```sh
cargo test                     # everything, offline
cargo test -p <crate>          # one crate
```

`cargo test` must pass before every commit.

## Rules (same habits as PaperComposer / JELearner)

- Files are the source of truth; SQLite indexes can always be rebuilt from them.
  Never overwrite a user's file: new files get a free name, template copies never overwrite.
- All LLM calls go through `ym-llm`. Model names live in catalog/config, never in code
  (tests excepted). Tests never call real Claude, the real `claude` CLI, a real
  llama-server, the network or Apple-only engines: use mocks/fakes. Things that cannot be
  tested here are marked **not tested on real hardware**.
- Personal vs household data (design §3): personal data never flows into a household
  space automatically; learning state is per member (`progress/<member>/`).
- Never commit keys, tokens, personal data, real user paths or e-mail addresses.
- Example/test user names: first user `ellrin`, second user `daidai`. Never invent names
  that look like real people.
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
- Public API changes in a crate: update its README and the design doc section.
- Replies to the author in Traditional Chinese (繁體中文).
