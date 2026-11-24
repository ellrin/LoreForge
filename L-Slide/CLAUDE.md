# L-Slide — notes for Claude Code

Task-driven slide editor, Rust, local-first, later a Mac app (Tauri). Same structure and
habits as L-Paper / JELearner / L-Brain. Design: `../private/shared/docs/ecosystem-design.md` (author's private notes) §7.

## Commands

```sh
cargo test                                         # sc-core + sc-server, offline (synthetic decks)
cargo run -p slidecomposer                         # desktop window (Tauri), free port; not in default-members
../scripts/install-apps.sh L-Slide           # build the .app, install to /Applications, Dock
cargo run -p sc-server                             # http://127.0.0.1:7880, workspace ../private/SlideComposer
SLIDECOMPOSER_DECKS=/tmp/decks cargo run --release -p sc-server
cd ../commons && cargo test -p lore-canvas        # the canvas core
LORE_DECKS="a.pptx:b.pptx" LORE_STYLE_DIR=../private/SlideComposer/styles/my-style \
  cargo run --release -p lore-canvas --example deck_roundtrip   # real decks: unedited export identical?
```

`CARGO_TARGET_DIR`: this workspace builds into `target/`; commons into `commons/target/canvas`
when working on lore-canvas in parallel with other agents.

Before a commit, the tests of the code you changed must pass (affected scope only, see the root CLAUDE.md).

## Layout

- `../commons/crates/lore-canvas`: app-agnostic scene model, style rules, ops/undo,
  geometry, reveal generator, components, PPTX ↔ canvas (`pptx_in`, `pptx_out`).
- Bases (底版, design §7.2 v2): a style folder with `base.json` is a canvas base — `Workspace::template_bytes`
  compiles it in memory (`lore_canvas::base::compile`; nothing written into the style folder), new slides get
  the page kind's `starter` objects (`Workspace::new_slide_objects`; `style:` pictures copied into the deck's
  assets). Without base.json the style's template.pptx is used (imported masters, e.g. a company template).
  `/api/styles/{id}/sample` gives the cover and content page for client-rendered previews.
- 版型 (recipes, `web/arrange.js`, right panel block 00): blocks on the slide (fractions) with roles —
  title, text, image, caption, card, table, takeaway, colour panels (`deco`) — plus the composition's guide
  lines and anchors (輔助線, drawn over everything, never exported). Sources: `web/recipes.json`, built from
  GraphicDesigner (MIT, the author's) by `node --import ./scripts/gd-register.mjs scripts/import-gd-recipes.mjs <GraphicDesigner>`
  (its own resolver and generators, 16:9 + any-canvas recipes), and the author's `arrangements/*.json` in the
  private workspace (same block format, `class: "academic"`). Objects keep their block as `slot`; empty slots are
  dashed in the editor and left out of shows, PDFs and PPTX (`Object::is_empty_slot`). `deck.defaults` = recipe
  per page kind for new slides (Enter / ＋); `slide.arrangement` = the recipe a slide was laid out with.
- Files where they live (`crates/core/src/file.rs`): a .pptx opened from Finder / 開啟… (native dialog
  through the shell: `/api/shell/dialog` calls a hook the desktop app installs — the page has no Tauri IPC)
  is bound to its deck (`decks/<name>/file.json`: path, size/mtime/hash last read or written, deck id).
  Lookup (`Workspace::open_path`): bound path → deck id in the file (moved file) → unbound deck from the
  same bytes → new import on the style whose layouts the file shares most. The page writes the file back
  ~0.9 s after the last edit (`/file/write`); the desktop app flushes pending decks on quit.
  A file changed on disk (hash differs from the last read/write) or gone is never written silently: the
  page asks 重新載入 / 保留我的版本，覆蓋 / 另存新檔…. Before the first overwrite the original is copied
  once to `.<stem>.original.pptx` next to it (never deleted). Writes: `.<file>.sc-tmp` + rename.
  Editor-only state (style, deck defaults, slide arrangement, object slots, empty slots) rides in the
  file as an Office custom XML part (`customXml/itemN.xml`, root `sc:state` in `urn:loreforge:slidecomposer:state`),
  so a wiped deck folder is rebuilt from the file (`EditorState`). Equation fallbacks for autosaves are
  cached per object content (`AppState::fallback_cache`). Writes copy the Finder tags and xattrs over
  (`copy_xattrs`). A changed file with no unsaved edits reloads by itself (`reloadFile(true)` in `web/app.js`).
- Paper material (論文 panel, `web/reader.js`): `crates/server/src/reader.rs` reads L-Reader's papers and
  PDFs read-only (`LOREREADER_WORKSPACE`, else lore-home's `private/LoreReader`; the library from L-Reader's
  config). Pages are rendered with `pdftoppm` into the workspace's `cache/lorereader/`; a figure is the whole
  page as one asset, cropped by the picture. Symbols and equations register into the deck's own `notation.json`.
- 論文報告 (home tile, `web/todeck.js`; L-Paper's and L-Reader's 做成簡報 = `commons/ui/makedeck.js`, calls the same route): `crates/server/src/todeck/` (stages) + `../commons/crates/lore-jobs` (the runner, re-exported as `sc_server::jobs`; 論文報告 is a job, see docs/paper-deck.md §API)
  turns an L-Paper LaTeX folder or an L-Reader PDF (lore-docsrc, read-only) into a paper-report deck: one
  lore-llm call (`sc-paper-deck`, `crates/server/tasks.toml`; `llm.rs`; every 論文報告 task is Opus → Sonnet, no class) plans the slides in the
  design §7.1 order by material id. An L-Reader PDF's material is seen and confirmed by a model in the `source` stage (`todeck/extract.rs`,
  docs/paper-deck.md 「素材」): one look per page (`sc-locate-material`: figures, tables, display equations with LaTeX, authors; the detector and L-Reader's
  text-layer LaTeX are proposals only), every crop shown back (`sc-check-crop`, re-cut from the model's tight box), table cells from the PDF's own words
  (`lore_docsrc::pdf::table::grid_in`), every equation's typeset reading compared with its crop (`sc-read-equation`, `AppState::typeset` = Chrome + KaTeX or a
  test fake); answers cached per paper (`cache/todeck/<sha>/verify/`); the planner gets the pictures too. Before the plan a vision call looks at the figures (`sc-figure-parts`, `todeck/figparts.rs`): which one is the
  method's big picture, whether to explain it in parts and where to cut (cuts snapped to blank gutters, `lore_docsrc::crop::snap_cut`; 微調's
  方法拆幾個模組 fixes the number; two failed calls = an equal split with a warning). The plan has one walk-through slide per part: the figure across
  the body (never a small copy), that part clear and the rest at the style's ghost alpha, its lines appearing one by one over the faint side
  (`build::LineReveal` → `expand_reveals`: copies of the slide, as many per deck as there are walk-through slides). Slots come from the academic
  arrangements, text from the style's tags, tables native, equations OMML (registry macros expanded), `Ref:` from
  cited keys, typed "Fig. 3" stripped; the paper's notation joins the deck's `notation.json` with its ids. `/api/todeck*` and `/api/info` answer 127.0.0.1 / localhost origins (CORS).
  A draft paper's 待補 marks (lore-notation `pending`) stay marks: `[pending: …]` in text (lore-docsrc), `[pending]` in table cells and after a
  dependent sentence (`source.rs`, `build.rs`), and `plan::PENDING_RULE` tells the planner to copy them, never a number (`../docs/long-paper.md` §E).
  L-Paper papers are found like L-Paper does (`PAPERCOMPOSER_WORKSPACE`, its config, lore-home).
  Layout rules (docs/paper-deck.md 「版面」, the author 2026-11-19): one size per text tag across the deck and nothing shrunk to fit (`plan::limit`,
  `check_plan` → one corrective request → `enforce_limits` splits or reports; polish runs with `keep_sizes`), every content page shares `build::page_frames`
  (the layout's own title frame, one body line, `Ref:` beside the logo: whole entries only), the deck's fonts are the author's `[deck]` fonts
  (`private/shared/fonts.toml` → `Deck::fonts` → `StyleSystem::with_fonts` at `Workspace::open`; never a font name in this repo), the cover's Speaker / Supervisor
  come from L-Paper's people (`todeck/people.rs`, `GET /api/todeck/people`: id + name only; last choice in the workspace's `paper-deck.json`). Everything
  skipped or degraded is a line of the job's `warnings` (job view, 微調, the deck's 排版用字 notes), never silent. Guard: `tests/todeck.rs` `assert_one_size_per_role`.
  微調 window: 長度／語言／底版 (`POST …/jobs/{id}/params`: 語言 or 長度 redo only the outline, `Jobs::redo_from`),
  建議 (`todeck/suggest.rs`, task `sc-outline-review`, class `create`, best-effort inside the outline stage: ≤5
  structured edits on the review rows, applied by the page). 補充 (`Params.brief`, `params` → redo the outline; `plan::brief_block`
  in the prompt, empty = unchanged; `plan::brief_missing` → warning) and 預想選項 (`todeck/interview.rs`, task `sc-deck-interview`, not in direct mode:
  2–3 options per point and for the brief, each citing records sent — `paper:` `pc:` `deck:` `card:` — numbers and ids checked in code; `review.interview`).
  The page (`web/todeck.js`): block 我想補充 (cards, field + mic, lines, 重寫大綱 = `params {brief, outline}`; 確定 reads 重寫大綱 while lines are unsent), a row's cards under its focused 重點, header switch 候選 (`sc.todeck` `cands`), `LoreChoices` from `/choices.js`; look at it with `?job=mock&at=review` (`&iv=0`, `&brief=1`) or `at=rewrite`.
  排版用字: the polish stage also writes the deck's
  `polish.json`; the editor shows it as the right-panel block 排版用字 (`?polish=1` after a job, AI → 排版用字),
  復原／套用 through `POST /api/decks/{name}/polish/{item}`. 語氣 (`deckvoice.rs`, author-approved 2026-11-13):
  example lines from the author's own imported decks (cover, chrome, repeated text, refs, names, deny words left
  out) live in `private/SlideComposer/voice/examples.json` (through lore-home; rewritten only when they change);
  the outline and 建議 prompts get the lines closest to the paper, bounded. Speaker notes follow `plan::NOTES_SHAPE` (opening line + numbered list,
  checked by `check_plan`); the program's symbol lines (`build::symbol_notes`, model-confirmed symbols) are never on the slide but the notes' last
  numbered item (`build::notes_with_symbols`; empty notes get an opening line), and `notation::view` counts a definition in a slide's notes as one on
  that slide (`notes_definitions`). The author's own note patterns are the hand-written `private/SlideComposer/voice/notes.md` (`deckvoice::notes_guide`,
  optional, ≤ 1500 chars, never written by the program). Never put example text in code,
  tests (synthetic decks only), the job cache or this repo. The terms stage's finder
  (`kbterms.rs`: terms, L-Brain states, `Matcher`) lives in `../commons/crates/lore-learn`, shared with
  L-Paper's 寫書; `kbterms.rs` re-exports it and keeps the slide marks, 我懂 and the L-Brain write.
- 繪圖 (toolbar 圖表, `web/figplot.js`, `crates/server/src/plot.rs`, `../docs/plot.md`): the shared lore-plot editor (`/plot/*`); the figure is a
  300 dpi PNG picture that carries its spec (`iTXt`, survives the .pptx); the selected figure's 編輯 button (`SC_FIGURE_TOOLS`, drawn by `renderOverlay`) re-opens it and replaces the image in place (one undo step); a double-click is not special.
- 會動的方程式 (toolbar `#insLiveEq`, `web/figplot.js` `create("equation")`, `../docs/plot.md` §10): an equation figure is a GIF picture (spec in its `LOREPLOT1.0`
  extension, `plot.rs` saves `frames` as GIF); 論文報告 adds one for the main loss (`todeck/liveeq.rs`: `main_loss`, `equation-plot`, `fit_for_auto` gate, `Render`
  = Chrome or a test fake via `AppState::liveeq`; 微調 `answers.liveEq = false` turns it off; skipped with a warning, never a guess).
- 流程圖 (toolbar 流程圖 and AI → 流程圖, `web/figflow.js`, `crates/server/src/flow.rs`, `../docs/flow-figures.md`): the shared lore-flow editor (`/flow/*`,
  `doc` = the deck → its notation); the figure is one picture — a group of native shapes/connectors/text with `lock_aspect` (resizing = `Op::Scale`:
  text, line widths, arrowheads scale together) and the spec in `GroupObj::data` (kept in the editor-state part, `SlideState::data`, so it survives the
  .pptx); the selected figure's 編輯流程圖 button or 排列 → 編輯流程圖 replaces it in place (one undo step); new variables go to the deck's `notation.json`.
  排列 → 一張圖 (group as one picture / toggle the lock), 另存圖片 PNG/SVG/PDF of any object (`/api/decks/{name}/image`, object.html through Chrome).
- 語句潤飾 (toolbar 潤飾 `#insPolish` and the right-click menu of a text box being edited, `polishSelection` in `web/app.js`;
  `crates/server/src/rewrite.rs`, `../docs/long-paper.md` §A): the shared `commons/ui/polish.js` over lore-polish's `/api/polish*`
  (`doc` = the deck) and lore-voice's `/api/voice*` (unavailable in `router_isolated`). One paragraph's selection is sent with
  equation runs as `$latex$` and breaks as `\n` (locked, `keep`; fields / LaTeX-less equations as `⟪n⟫`), the deck's registry
  macros locked too; 套用 = `exitEdit()` first (typing is its own step), then one `set_obj /text` transaction 「潤飾」 with the
  runs rebuilt (`polishRuns`: original equation/break runs back, new text in the first selected run's look). Log
  `decks/<name>/polish-log.jsonl` (not 排版用字's `polish.json`); tone = `deckvoice` lines closest to the passage; 👍 →
  `private/shared/polish/liked.jsonl`. Task `polish` (lore-polish's tasks.toml, merged in `app_tasks()` and the 模型 tab). Its `engine` may also name one model (`claude:<alias>`, `local:<catalog id>`, `agy:<model id>`, L-Echo's model menu): that model or an error, for this task only (`Llm::route`).
  Every rewrite sees `scope` 「第 n 頁：title」 and labelled `context` (the box's other paragraphs, the slide's other text top then left, 講者備註 ≤ 600; ≤ 2,500 in all; `selectionContext` in `web/arrange.js`, guard `web/tests/selection.test.js`); the same selection goes to L-Echo through `lore_shell::selection` (`LoreSelection.watch` in `web/app.js`, right-click 交給 L-Echo), its rewrite applied by `applyRewrite` on the anchor captured when it was sent.
  Desktop: `apps/desktop/build.rs` copies the lore-voice helper to `bin/` → `Contents/MacOS` (`bundle.macOS.files`), Info.plist has the mic/speech usage strings.
- 互動 in a show (`web/present.js` `live*`, `../docs/plot.md` §11): a slide with an equation figure (a GIF whose spec is a lore-plot equation, looked up per asset) shows 互動 (I) in the presenter bar / single-screen bar. The presenter (or the single screen) mounts `LorePlot.eqSim.mount` (simulation) or `mountCurve` (a curve: one slider per parameter) over the stage in the Lore theme, runs it and sends `{ live }` messages (`open` = the spec and full state, ticks with every new history row and the weights); the audience mounts role `view` and only renders (a tick past its history → `hello` again). `SHOW.live` is put back into the stage by `showRender` (black, step back, re-sync); another slide closes it; Esc / 回到投影片 / leaving full screen closes 互動 only; 對調螢幕 carries the state (`snapshot().live`). Guard: `web/tests/show.test.js` (fake DOM).
- Show (`web/present.js`): 選頁 = `openPicker` (presenter, G; thumbnails rendered once per show, light-table `pickLayout`, hidden slides join
  the show through `showIndexOf`). The presenter console is dark in every theme (chrome takes only `--accent`). Logic tests:
  `web/tests/show.test.js` (run by `crates/server/tests/show.rs`). Camera hand control was tried and dropped on
  2026-11-13 (`../docs/ideas.md` §5): no camera permission or helper.
- `crates/core` (`sc-core`): `Workspace` (styles/, decks/), `Session` (open deck: editor,
  source package, layout backdrops, LaTeX cache), export PPTX/PDF, `layout_titles`.
- `crates/server` (`sc-server`): axum on 127.0.0.1:7880; handlers are plain async fns over
  `Shared` so they can become Tauri commands later. Static files are served `no-cache`.
- `apps/desktop` (`slidecomposer`): Tauri shell; starts sc-server in-process on a free port,
  `web/` bundled as a resource, workspace from `sc_server::default_workspace()`. Icon source
  `icons/logo.svg` (L-Paper's neon style).
- `web/`: static HTML/CSS/JS, no build step; the top bar, gear, menus, toast, ask and drawers are the shared shell (`lore_shell::FILES` serves `/icons.js` `/base.css` `/shell.js`; `mountShell` in `app.js`), ⌘F is `find.js`. `render.js` is the only renderer (editor canvas,
  thumbnails, `print.html` for PDF). UI text in Traditional Chinese; icons from
  L-Brain/L-Paper. Colours: the theme all LoreForge apps share — `/theme.css` and
  `/theme.js` are `commons/ui/theme.{css,js}` (through `lore_home::theme`), `/api/theme` reads and
  sets the shared choice, 設定 has the 外觀 picker (`LoreTheme.picker`). Slides, thumbnails, the
  show, `object.html` and `print.html` never use theme tokens. KaTeX vendored in `web/vendor/katex`.
- `../private-example/SlideComposer/`: starting layout of the private workspace (copied to `../private/SlideComposer` on first run).

## Decisions

- Three tiers: documents (the bound .pptx, where the author keeps it) are the source of truth for
  bound decks; tool data (styles, arrangements) stays in `private/`; the deck folder is a regenerable
  cache for a bound deck. `decks/<name>/deck.json` (written after every transaction), `source.pptx`
  (the bytes the deck was imported from, never modified; exports diff against it), `assets/`
  (content-addressed images), `exports/` (never overwritten, `free_path`). Unbound decks
  (新簡報, browser import) live only in their folder until 另存新檔….
- Lossless: every imported object keeps its XML + per-field fingerprints. Untouched objects
  export verbatim; edited ones are patched field by field on their original XML; slides whose
  fingerprint is unchanged keep their original part bytes. Duplicated slides (reveal) reuse
  the source slide's relationships.
- Rules (enforced in `lore-canvas::style`): text runs carry a style-pack tag + optional emphasis
  tag; new colours must be palette colours. Imported runs that match no tag stay untagged
  (their formatting is kept) until a tag is applied.
- Undo/redo lives in Rust (`lore_canvas::Editor`); the UI sends op batches and re-renders the
  slides the server returns. Attached connectors are re-routed in the same transaction.
- Progressive reveal = duplicate slides + ghost (default, design §13-4); animations optional.
- Equations: imported OMML kept raw; display via pandoc OMML→LaTeX (cached per deck) + KaTeX;
  new equations LaTeX → OMML via `lore-math` (pandoc). PDF: headless Chrome prints `print.html`
  (**v2: Typst**). Fonts not installed here (a template's brand font, Calibri, 微軟正黑體) fall back to
  PT Sans / PingFang TC — widths differ slightly.

## Gaps (v1, prioritised)

1. Text metrics: substitute fonts → some lines wrap/overflow differently than PowerPoint;
   autofit (`normAutofit`) uses the saved fontScale only; table row heights are not
   re-measured (frame height can be smaller than the rendered table).
2. Rendering not implemented: picture effects (a14 background removal, recolour), shadows/
   effectLst, Wingdings/Symbol private-use bullets, pattern/picture fills, gradient path fills.
3. Editing: children inside groups can be text-edited but not moved/resized individually;
   connector text labels; custom geometry point editing; merged-cell editing UI; per-cell
   borders UI; picture mask rectangles are edited numerically (no handles).
4. Locked objects (charts/SmartArt/OLE) are kept but cannot be moved (none in the 4 decks).
5. Equation edit replaces the first math run of a box. New/edited equation boxes get an
   `mc:Fallback` picture rendered by headless Chrome (`object.html`, KaTeX — looks close to,
   not identical with, PowerPoint's own rendering); without Chrome they export with no
   fallback (export warns). Chrome is run without `--user-data-dir` (Chrome 153 headless never
   finishes `--screenshot`/`--print-to-pdf` with one) and with a timeout that kills its process group.
6. Animations: preset ids follow lore-ooxml (Wipe/Zoom subtypes **not verified on PowerPoint**).

## Rules

- Never commit keys, tokens, personal data, real user paths, real decks or the user's e-mail.
  Real decks live only in the private workspace or a scratch folder; tests use synthetic decks.
- Example and test users: `ellrin`, then `daidai`. Never invent names that look like real people.
- Never overwrite a user's file, except the .pptx a deck is bound to (opened there or chosen in
  另存新檔): one-time backup first, atomic write, never over a version changed elsewhere. Exports never
  overwrite; deck.json writes are atomic; no code deletes backups or caches automatically.
- Every frontend change (web/) is reviewed afterwards (screenshots in light/dark and narrow
  widths); new CSS uses `--accent-text` for accent text, line icons and thin borders.
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
- Replies to the author in Traditional Chinese (繁體中文).
