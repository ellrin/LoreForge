# LoreReader — notes for Claude Code

Local-first PDF reader for research papers, Rust, later a Mac app (Tauri). Same structure and
habits as SlideComposer / PaperComposer / KnowledgeBase. Goal: learn fast what a paper needs you
to master; the KB holds only what was digested (cards only on explicit action); the main output
is citable sentences for PaperComposer. Not in scope: PDF content editing, slide wizard.

## Commands

```sh
cargo test                                         # lr-core + lr-server, offline (synthetic PDFs, fake KB, fake engine)
cargo run -p lr-server                             # http://127.0.0.1:7890, workspace ../private/LoreReader
cargo run -p lorereader                            # desktop window (Tauri); not in default-members
../scripts/install-apps.sh LoreReader              # build the .app, install to /Applications, Dock
cargo run -p lr-core --example synth_pdf -- demo out.pdf   # the synthetic demo paper
LOREREADER_LIBRARY=/tmp/lib LOREREADER_EXPORTS=/tmp/out LOREREADER_WORKSPACE=/tmp/ws cargo run -p lr-server
```

`cargo test` must pass before every commit (and KnowledgeBase's when its web/ or API changes).

## Layout

- `crates/core` (`lr-core`): `config` (env > `<app dir>/config.toml` > defaults; app dir =
  `$LOREREADER_HOME` or `<OS config dir>/LoreReader`), `library` (PDFs outside git, sha256
  addressed, never modified; refuses a git work tree; exports with free names), `papers`
  (`private/LoreReader/papers/<sha256>/{paper,annotations,quotes,chat}.json`, atomic writes,
  unknown fields kept), `pdfops` (lopdf `compose(sources, picks)` = every page operation;
  title, DOI), `text` (the page's pdf.js text: items with font name, size, position; UTF-16
  offsets shared with the page), `nouns` (track A noun list: inventory, KB n-grams 2–4 over
  titles + aliases, normalisation, `function_words.txt` / `academic_words.txt` /
  `common_words.txt` minus `TERMLIKE`, ignore list, batched classification prompt + schema),
  `symbols` (step 1 of variables/equations, no model: math runs from item fonts / sizes /
  baselines / accent positions, `glyphs.toml` + overrides + TeX font-encoding tables (OML /
  OMS / OT1 / CMEX), display equations, regex definition finder, `SymbolFile` with
  lore-notation `Entry` + scope / grade / resolved_by, `merge` keeps author / Claude edits),
  `paragraphs` (track B: split with sections / indents / gaps, display equations kept inside
  as ［式 (n)］, chips = red terms + symbols + equations, run order methods/results first,
  `lr-paragraph` prompt + schema; no paragraph summary or translation),
  `synth` (synthetic PDFs, incl. the ≈8,000-word `long_paper`, `paper_text` = what the page
  would post for a synthetic PDF — never a real paper).
  `tests/math/`: synthetic LaTeX papers + their pdf.js items (`dump-items.js`); the PDFs are
  not kept. `examples/symbols_json.rs --tex` = precision / recall against a LaTeX source.
- `crates/server` (`lr-server`): axum on 127.0.0.1:7890; handlers are plain async fns over
  `Shared`. `kb.rs` = KnowledgeBase client (read only except `POST /api/nodes`, only from
  the author's action, `initial_state` learning|known; `lexicon()` = `GET /api/nodes` titles +
  aliases), `llm.rs` = lore-llm wired like PaperComposer (`tasks.toml`: `lr-translate`
  local:current → claude:selected, `lr-chat` claude:selected only, `lr-classify-terms` local:current →
  claude:haiku, `lr-paragraph` claude:haiku → local:current; `complete_on` = the model menu's choice, strict),
  `reading.rs` = the reading aids over the text the page posts once per open (memory only):
  nouns, symbols, paragraphs (background run per paper, newest run wins, cached per
  paragraph id), 懂了 (`reading.json`), and `chat_context` (title, section, paragraph, chip
  + its definition here, the author's KB states) for Ask Claude.
  `Printer` trait = the desktop print bridge.
- `apps/desktop` (`lorereader`): Tauri shell on 7890 (KB links point there); reuses an
  lr-server already on 7890, else binds it (free port if taken). Registers `NativePrinter`:
  print.html in a window, `WebviewWindow::print()` when `/api/print/ready` arrives.
  Registered as a PDF viewer (`fileAssociations`): `RunEvent::Opened` files go through
  `POST /api/papers/import {path}` on whichever server it uses, then `/?paper=<sha>`.
- `web/`: static HTML/CSS/JS, no build step; `app.js` is an ES module over pdf.js
  (`web/vendor/pdfjs`, 5.7.284, Apache-2.0; keep LICENSE + VERSION). `print.html` renders pages
  at ~216 dpi. Tokens/icons from SlideComposer (icons.js copied, extra icons at the end).
- `../private-example/LoreReader/`: seed of the private workspace (README, `ignore-words.txt`, `papers/`).

## Decisions

- The view is a list of page refs `{sha, page}`; page operations edit a pending list and
  "另存新檔" composes a new PDF (library + exports). Annotations, quotes and KB sources always
  name the page's own PDF and its page number there.
- Annotations anchor by page + rects (page fractions) + text quote (exact/prefix/suffix).
  Writing them into the PDF is a later option.
- Knowledge colouring = the noun list (`nouns.rs`): red = no card and not 一般學術詞 / 人名;
  amber = learning / stale / no state; green = known. The page sends all pages' pdf.js text
  once (`POST /api/papers/{sha}/read`); the server never stores the paper's text. KB names
  come from `GET /api/nodes` (read only; no `/api/scan`, no encounters).
  `ignore-words.txt` replaced `known-words.txt` (migrated once by `reading::ignore_file`).
- Classification: one `lr-classify-terms` call per paper (local first, then Haiku), cached in
  `nouns.json` under `key = v<CLASSIFY_VERSION>|<field>`; `asked` keys are never re-sent
  automatically. Red = SPECIALIST categories only (not 基礎科學詞 / 一般學術詞 / 人名 /
  unclassified), no card, not in my-vocab. The reader's field: `reading.toml` `field`, else
  `private/shared/profile.toml` `field`, else `nouns::DEFAULT_FIELD`.
- 我的論文: `paper.json` `mine`; `my-vocab.json` (per-source entries, rebuilt on flag change)
  marks no-card units `used` = 記得 (a KB card's own state wins). Not the ignore list. Math runs and display equations are masked out of
  the noun list.
- Track B never writes a Chinese summary or translation of a paragraph. Claude's answers
  reach the symbol table only through 寫回符號表 (`resolved_by: claude`, evidence kept).
- Theme: `web/theme.js` = `data-mode` on `<html>`, `lr.mode` in localStorage, then
  `window.LORE_PREFS.theme` from `/prefs.js` (always `null` today). **Later**: a shared, synced
  theme in lore-home prefs fills `theme` server-side in `prefs_js`; pages need no change.
- Symbol table: `papers/<sha256>/symbols.json` (lore-notation fields + scope, grade,
  resolved_by). Grades are evidence-based: the program gives 🟢 only with the quoted sentence,
  🔴 only for two definitions in the same paper (never vs the author's notation). Keys are
  lore-notation canonical keys; ground truth in tests = lore-notation's scan of the source.
  Glyph table: default in lr-core, overrides `private/shared/glyphs.toml` then
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
