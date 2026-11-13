# commons — notes for Claude Code

Shared Rust crates and web components used by the author's apps (PaperComposer,
JELearner, KnowledgeBase, SlideComposer, NeonDeck widgets, future apps). The overall
design is in `../private/shared/docs/ecosystem-design.md` (author's private notes) — read the section for the crate you touch.

## Layout

- `crates/<name>/` — one crate per module (`lore-llm`, `lore-secrets`, `lore-gitsync`, `lore-store`, `kb-core`,
  `lore-notation`, `lore-notes` (PaperComposer 筆記 files), `lore-voice`, `lore-docsrc`, `lore-canvas`, `lore-ooxml`, `lore-math`, `lore-fonts`, `lore-home`, `lore-flow` (流程圖: L1 → layout → canvas objects; the figure spec, layout templates, the shared editor `ui/flow/` and its `/flow` routes, `../docs/flow-figures.md`), `lore-upscale`, `lore-shell` (serves `ui/icons.js`, `base.css`, `shell.js`),
  `lore-engines` (the shared 模型 settings: `/api/engines`, `/api/links/open`),
  `lore-catalog` (LoreShelf's read-only file catalog: hashes, time axes, projects, reports, local-only guard; `fulltext` = every page / slide / block of PDFs, pptx, docx, text into the cache; `terms` = the term pool of words the KnowledgeBase lacks),
  `lore-jobs` (long-running background jobs: stages, checkpoints, progress, stop/resume, review wait; SlideComposer's 論文報告, PaperComposer's 寫書),
  `lore-learn` (what the author knows and read, read-only: a paper's terms against the KnowledgeBase, card prerequisites, LoreReader's files),
  `lore-research` (related papers on the web, open access only: OpenAlex / arXiv / Semantic Scholar, dedupe, OA PDF download, BibTeX, LoreReader import; `docs/book.md`),
  `lore-plot` (繪圖: figure spec, data import, style presets, Chrome/poppler export, `plot-propose`, the `/plot` routes; `ui/plot/`, `../docs/plot.md`)).
  Local models: one llama-server per machine shared by every app in-process,
  `lore_llm::shared_runtime` (`../docs/local-models.md`; no broker process).
  Each crate knows nothing about any app. Apps depend on them by path
  (`../commons/crates/<name>` from an app folder).
- `ui/` — framework-free Web Components (no build step), one folder per component,
  each with a `demo.html` that works from `python3 -m http.server`. `ui/theme.css` + `ui/theme.js`
  are the colour themes of every app (20 palettes, the 外觀 picker, one shared choice kept by
  `lore-home`); `cargo test -p lore-home` checks the ids and the contrast rules after any change.
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
- All LLM calls go through `lore-llm`. Model names live in catalog/config, never in code
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
