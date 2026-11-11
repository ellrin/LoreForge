# SlideComposer — notes for Claude Code

Task-driven slide editor, Rust, local-first, later a Mac app (Tauri). Same structure and
habits as PaperComposer / JELearner / KnowledgeBase. Design: `../private/shared/docs/ecosystem-design.md` (author's private notes) §7.

## Commands

```sh
cargo test                                         # sc-core + sc-server, offline (synthetic decks)
cargo run -p slidecomposer                         # desktop window (Tauri), free port; not in default-members
../scripts/install-apps.sh SlideComposer           # build the .app, install to /Applications, Dock
cargo run -p sc-server                             # http://127.0.0.1:7880, workspace ../private/SlideComposer
SLIDECOMPOSER_DECKS=/tmp/decks cargo run --release -p sc-server
cd ../commons && cargo test -p lore-canvas        # the canvas core
LORE_DECKS="a.pptx:b.pptx" LORE_STYLE_DIR=../private/SlideComposer/styles/my-style \
  cargo run --release -p lore-canvas --example deck_roundtrip   # real decks: unedited export identical?
```

`CARGO_TARGET_DIR`: this workspace builds into `target/`; commons into `commons/target/canvas`
when working on lore-canvas in parallel with other agents.

`cargo test` (here and `-p lore-canvas` in commons) must pass before every commit.

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
- Paper material (論文 panel, `web/reader.js`): `crates/server/src/reader.rs` reads LoreReader's papers and
  PDFs read-only (`LOREREADER_WORKSPACE`, else lore-home's `private/LoreReader`; the library from LoreReader's
  config). Pages are rendered with `pdftoppm` into the workspace's `cache/lorereader/`; a figure is the whole
  page as one asset, cropped by the picture. Symbols and equations register into the deck's own `notation.json`.
- 論文報告 (home tile, `web/todeck.js`; PaperComposer's 做成簡報 calls the same route): `crates/server/src/todeck/ (stages) + src/jobs.rs (runner; 論文報告 is a job, see docs/paper-deck.md §API)`
  turns a PaperComposer LaTeX folder or a LoreReader PDF (lore-docsrc, read-only) into a paper-report deck: one
  lore-llm call (`sc-paper-deck`, class `create`, `crates/server/tasks.toml`; `llm.rs`) plans the slides in the
  design §7.1 order by material id; `sc-crop-modules` (vision, optional) boxes the main figure's modules for the
  method slides (whole figure ghosted + the module cropped back on top, framed); slots come from the academic
  arrangements, text from the style's tags, tables native, equations OMML (registry macros expanded), `Ref:` from
  cited keys, typed "Fig. 3" stripped, the big picture expanded by `reveal`; the paper's notation joins the
  deck's `notation.json` with its ids. `/api/todeck*` and `/api/info` answer 127.0.0.1 / localhost origins (CORS).
  PaperComposer papers are found like PaperComposer does (`PAPERCOMPOSER_WORKSPACE`, its config, lore-home).
- `crates/core` (`sc-core`): `Workspace` (styles/, decks/), `Session` (open deck: editor,
  source package, layout backdrops, LaTeX cache), export PPTX/PDF, `layout_titles`.
- `crates/server` (`sc-server`): axum on 127.0.0.1:7880; handlers are plain async fns over
  `Shared` so they can become Tauri commands later. Static files are served `no-cache`.
- `apps/desktop` (`slidecomposer`): Tauri shell; starts sc-server in-process on a free port,
  `web/` bundled as a resource, workspace from `sc_server::default_workspace()`. Icon source
  `icons/logo.svg` (PaperComposer's neon style).
- `web/`: static HTML/CSS/JS, no build step; the top bar, gear, menus, toast, ask and drawers are the shared shell (`lore_shell::FILES` serves `/icons.js` `/base.css` `/shell.js`; `mountShell` in `app.js`), ⌘F is `find.js`. `render.js` is the only renderer (editor canvas,
  thumbnails, `print.html` for PDF). UI text in Traditional Chinese; icons from
  KnowledgeBase/PaperComposer. Colours: the theme all LoreForge apps share — `/theme.css` and
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
