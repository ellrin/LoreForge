# LoreReader — notes for Claude Code

Local-first PDF reader for research papers, Rust, later a Mac app (Tauri). Same structure and
habits as SlideComposer / PaperComposer / KnowledgeBase. Goal: learn fast what a paper needs you
to master; the KB holds only what was digested (cards only on explicit action); the main output
is citable sentences for PaperComposer. Not in scope: PDF content editing, slide wizard (the top bar's 做成簡報 =
`commons/ui/makedeck.js` hands the PDF to SlideComposer's 論文報告, `{kind: "lr", sha}`), writing a review
(the top bar's 寫 review = `commons/ui/makebook.js` hands it to PaperComposer's 寫書, `docs/book.md`, with
`readingExtras()` = red terms, paragraphs without 懂了, 問這篇 questions). Downloaded open-access papers come
in through `POST /api/papers/import {path}` (`lore_research::import_into_reader`).

## Commands

```sh
cargo test                                         # lr-core + lr-server, offline (synthetic PDFs, fake KB, fake engine)
cargo run -p lr-server                             # http://127.0.0.1:7890, workspace ../private/LoreReader
cargo run -p lorereader                            # desktop window (Tauri); not in default-members
../scripts/install-apps.sh LoreReader              # build the .app, install to /Applications, Dock
cargo run -p lr-core --example synth_pdf -- demo out.pdf   # the synthetic demo paper
cargo run -p lr-server --example isolated_server -- <empty folder> [port] [kb url]   # UI without model, keychain or claude CLI
LOREREADER_LIBRARY=/tmp/lib LOREREADER_EXPORTS=/tmp/out LOREREADER_WORKSPACE=/tmp/ws cargo run -p lr-server
```

`cargo test` must pass before every commit (and KnowledgeBase's when its web/ or API changes).

## Layout

- `crates/core` (`lr-core`): `config` (env > `<app dir>/config.toml` > defaults; app dir =
  `$LOREREADER_HOME` or `<OS config dir>/LoreReader`), `library` (PDFs outside git, sha256
  addressed, never modified; refuses a git work tree; exports with free names), `papers`
  (`private/LoreReader/papers/<sha256>/{paper,annotations,quotes,chat,export}.json`, atomic writes,
  unknown fields kept; dirty = annotations hash ≠ last export), `annotate` (notes ⇄ PDF
  annotations: `/Highlight` + `/AP`, sticky `/Text` for notes, `/FreeText` + `/AP` for `kind: "text"` boxes (web/notes.js), `/NM (lr:<id>)`, Info
  `/LoreReaderSource`; display fractions ⇄ user space via CropBox + `/Rotate`), `pdfops` (lopdf `compose(sources, picks)` = every page operation;
  title, DOI), `text` (the page's pdf.js text: items with font name, size, position; UTF-16
  offsets shared with the page; `blank_line_numbers` = a manuscript's margin line numbers
  as spaces before any analysis, ranges sent back to the page; `boilerplate_lines` =
  submission-system lines and running heads, masked out of the noun list), `nouns` (track A noun list: inventory, KB n-grams 2–4 over
  titles + aliases, normalisation, `function_words.txt` / `academic_words.txt` /
  `common_words.txt` + SCOWL ≤20 `english_common.txt` minus `TERMLIKE`, ignore list; SCOWL 35
  `english_familiar.txt` = listed only once classified (`Noun::familiar`, `Inventory::listed`);
  names / affiliations / title-case front matter, identifiers, addresses dropped (`FrontMatter`);
  batched classification prompt + schema),
  `symbols` (step 1 of variables/equations, no model: math runs from item fonts / sizes /
  baselines / accent positions, `glyphs.toml` + overrides + TeX font-encoding tables (OML /
  OMS / OT1 / CMEX), display equations, regex definition finder, `SymbolFile` with
  lore-notation `Entry` + scope / grade / resolved_by, `merge` keeps author / Claude edits),
  `paragraphs` (track B: split with sections / indents / gaps, display equations kept inside
  as ［式 (n)］, chips = red terms + symbols + equations, run order methods/results first,
  `lr-paragraph` prompt + schema; no paragraph summary or translation),
  `myvocab` (cross-app my-vocab, see Decisions), `synth` (synthetic PDFs, incl. the ≈8,000-word `long_paper`, `paper_text` = what the page
  would post for a synthetic PDF — never a real paper).
  `tests/math/`: synthetic LaTeX papers + their pdf.js items (`dump-items.js`); the PDFs are
  not kept. `examples/symbols_json.rs --tex` = precision / recall against a LaTeX source.
- `crates/server` (`lr-server`): axum on 127.0.0.1:7890; handlers are plain async fns over
  `Shared`. `kb.rs` = KnowledgeBase client (read only except `POST /api/nodes`, only from
  the author's action, `initial_state` learning|known; `lexicon()` = `GET /api/nodes` titles +
  aliases), `llm.rs` = lore-llm wired like PaperComposer (`tasks.toml`: `lr-translate`
  local:current → claude:selected, `lr-chat` claude:selected only, `lr-classify-terms` and `lr-paragraph`
  local:current → claude:haiku; `complete_on` = a per-call choice or the old models.json one (`auto | local | cloud | haiku | sonnet | opus`), strict, else the shared routes / overrides; errors are `LlmFail` = lore-llm's `payload` (with `alternative`); keys `SharedKeys::for_app("LoreReader")`; the local
  engine is config `[engine]`, else the shared model id of `private/shared/engine.toml` (re-read per
  call) on the machine's one llama-server (lore-llm `shared_runtime`, `../docs/local-models.md`;
  `llm::quit()` on Quit / ⌘Q / Ctrl-C)),
  `reading.rs` = the reading aids over the text the page posts once per open (memory only):
  nouns, symbols, paragraphs (background run per paper, newest run wins, cached per
  paragraph id), 懂了 (`reading.json`), and `chat_context` (title, section, paragraph, chip
  + its definition here, the author's KB states) for Ask Claude.
  `Printer` / `Saver` traits = the desktop print / save-panel bridges.
- `apps/desktop` (`lorereader`): Tauri shell on 7890 (KB links point there); reuses an
  lr-server already on 7890, else binds it (free port if taken). Registers `NativePrinter`:
  print.html in a window, `WebviewWindow::print()` when `/api/print/ready` arrives.
  Registered as a PDF viewer (`fileAssociations`): `RunEvent::Opened` files go through
  `POST /api/papers/import {path}` on whichever server it uses, then `/?paper=<sha>`.
  `NativeSaver` (tauri-plugin-dialog). Close / ⌘Q: one prompt, for the paper in the window's
  URL (`paper=<sha>`; `GET …/export` → `dirty`), the same 儲存 / 不儲存 / 取消 the page asks when
  the author leaves a paper (no list across papers; `/api/dirty` stays for tests); the menu's Quit is our own item (the predefined one calls `terminate:`, which no
  Tauri event can stop — Dock Quit / logout still bypass the prompt).
- `web/`: static HTML/CSS/JS, no build step; `app.js` is an ES module over pdf.js
  (`web/vendor/pdfjs`, 5.7.284, Apache-2.0; keep LICENSE + VERSION). `print.html` renders pages
  at ~216 dpi. The shared shell (`docs/shell.md`): `/icons.js`, `/base.css`, `/shell.js` are
  `lore_shell::FILES` (no icons.js in web/), `/notation-panel.js` the shared 對照表 component;
  `LoreShell.mount` builds the top bar, gear (一般 · 外觀 · 模型 · 快捷鍵), drawers, ⌘F / ⌘S;
  the right panel is `LoreShell.block`s (01 名詞 … 06 問這篇). `/api/engines` (the 模型 tab, the
  engine chips' `overrides`) and `/api/links/open` are `lore_engines::router` (isolated: no
  keychain / CLI / opener, process routing not installed). styles.css keeps only what base.css
  does not own.
- `../private-example/LoreReader/`: seed of the private workspace (README, `ignore-words.txt`, `papers/`).

## Decisions

- The view is a list of page refs `{sha, page}`; page operations edit a pending list and
  "另存新檔" composes a new PDF (library + exports). Annotations, quotes and KB sources always
  name the page's own PDF and its page number there.
- Annotations anchor by page + rects (page fractions) + text quote (exact/prefix/suffix).
  "另存新檔（含筆記）" writes them into a new PDF (never the original; overwrites only files
  LoreReader saved). Pending page operations must be saved first. Importing such a PDF maps
  it to the paper in `/LoreReaderSource` when that PDF is in the library, else stores it
  without `lr:` annotations as a new paper with `source_sha`; notes merge by id.
- Knowledge colouring = the noun list (`nouns.rs`): red = a term to learn (see Classification);
  amber = learning / stale / no state; green = known. The page sends all pages' pdf.js text
  once (`POST /api/papers/{sha}/read`); the server never stores the paper's text. KB names
  come from `GET /api/nodes` (read only; no `/api/scan`, no encounters).
  `ignore-words.txt` replaced `known-words.txt` (migrated once by `reading::ignore_file`).
- Classification: one `lr-classify-terms` call per paper (local first, then Haiku), cached in
  `nouns.json` under `key = v<CLASSIFY_VERSION>|<field>`; `asked` keys are never re-sent
  automatically. Red = SPECIALIST categories only (not 基礎科學詞 / 一般學術詞 / 人名 /
  unclassified), no card, not in my-vocab; while no field is set single words (`source: word`)
  stay quiet (`Noun.quiet`), only phrases / acronyms are red, and the nouns pane asks for the
  field once, inline. The reader's field: `private/shared/profile.toml`
  `field` (an old `private/LoreReader/reading.toml` field is moved there once), else
  `nouns::DEFAULT_FIELD`.
- 我的論文: `paper.json` `mine`; `private/shared/my-vocab.json` (lr-core `myvocab`, cross-app,
  `lore.my-vocab/1`; per-source entries — a paper sha or `import:<name>`; a flag change
  replaces only that sha's source; `AppState.shared` = lore-home shared dir, a temp folder
  in isolated tests; the old per-app file is merged once; `examples/my_vocab_import.rs`
  builds import sources from paragraph text)
  marks no-card units `used` = 記得 (a KB card's own state wins). Not the ignore list. Math runs and display equations are masked out of
  the noun list.
- Titles: `paper.json` `title_source` = author | page | pdf | file. A `file` title (the name of
  the file) is replaced by the largest-type block of page 1 when the page posts its text
  (`text::title_from_first_page`); a placeholder `/Title` counts as `file`. The author's edit wins.
- Track B never writes a Chinese summary or translation of a paragraph. Claude's answers
  reach the symbol table only through 存進符號表… (`resolved_by: claude`, evidence kept).
- Readers elsewhere: PaperComposer (`crates/server/src/reader.rs`, the 讀過的 tab and panel) and
  SlideComposer (`crates/server/src/reader.rs`, the 論文 panel) read the library's `papers/<sha256>/*.json`
  and PDFs straight from disk, read-only; they do not call `/api/quotes`. Changing these files' schema
  means changing both readers too.
- Theme: the one all LoreForge apps share. `/theme.css` and `/theme.js` are `commons/ui/theme.{css,js}`
  (compiled in through `lore_home::theme`; the script carries the shared choice from
  `private/shared/ui.toml`), `GET`/`PUT /api/theme` read and set it; the gear's 外觀 tab is the
  picker (`LoreTheme.picker`). `data-mode` (dark / light) follows the theme. PDF pages,
  thumbnails and `print.html` never use theme tokens.
- Symbol table: `papers/<sha256>/symbols.json` (lore-notation fields + scope, grade,
  resolved_by). Grades are evidence-based: the program gives 🟢 only with the quoted sentence,
  🔴 only for two definitions in the same paper (never vs the author's notation). Keys are
  lore-notation canonical keys; ground truth in tests = lore-notation's scan of the source.
  對照表 (top bar, 03 符號): `symbols.json` in `<notation-panel>` as document scope; 加入全域 copies
  the selected row into `private/shared/notation.json` (lore-notation `Notation`, free id, paper-only
  fields dropped; an existing symbol is refused). Glyph table: default in lr-core, overrides `private/shared/glyphs.toml` then
  `private/LoreReader/glyphs.toml`. KaTeX in `web/vendor/katex` (MIT, same as PaperComposer).

## Not tested on real hardware

- The Tauri app (no WebKitGTK here: `apps/desktop` does not compile on this Linux box), the
  native print dialog and a real printer; real local models / Claude; real papers.

## Rules

- Never commit PDFs (other than tests' generated bytes), personal data, real user paths, keys
  or the user's e-mail. The library is outside git by construction; tests use temp dirs.
- Tests never call the real KB, a real model, the `claude` CLI or the keychain (`state(…, isolated=true)`).
- Every web/ change is reviewed with screenshots in light/dark and narrow widths; new CSS uses
  `--accent-text` for accent text, line icons and thin borders. UI text in Traditional Chinese.
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
- Replies to the author in Traditional Chinese (繁體中文).
