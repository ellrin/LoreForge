# LoreShelf — notes for Claude Code

Local file catalog: finds files by content (sha256), time axes and (later) tags, without ever
moving them. Design draft: `../docs/loreshelf.md`. Wave 1 = read-only scan + report
(`../commons/crates/lore-catalog`); later waves add content extraction, local-model tags, the
3D view, LoreEcho and KnowledgeBase links.

## Commands

```sh
cargo test                                   # CLI + shelf-server tests, offline (temp catalog, fake opener)
cargo run -p shelf-server                    # http://127.0.0.1:7900 (ports.json `ls`; LORESHELF_PORT, next free port if taken)
cargo run -p loreshelf                       # desktop window (Tauri); not in default-members
../scripts/install-apps.sh LoreShelf         # build the .app, install to /Applications, Dock
cd ../commons && cargo test -p lore-catalog -p lore-gitsync
cargo run --release -p loreshelf-cli -- scan ~/Downloads   # read-only scan; prints a summary and the report path
cargo run --release -p loreshelf-cli -- report            # latest report path
cargo run --release -p loreshelf-cli -- extract           # whole text of PDFs / pptx / docx / text into the cache (only new ones; ~1 min)
cargo run --release -p loreshelf-cli -- terms             # the term pool from that text and the KnowledgeBase (~20 s; run from inside LoreForge)
cargo run --release -p loreshelf-cli -- mastered <dir>...  # folders he knows all of (catalog `mastered.json`, local only)
cargo run --release -p loreshelf-cli -- known-cards [--write]  # their terms (model verdict `term`) → known KB cards (API while KB runs, else kb-core); no file names in cards
cargo run --release -p loreshelf-cli -- classify [N] [--cloud]  # the local model sorts the pool; per batch, resumable, background priority (hours); then ignore-words --write. --cloud: Claude Haiku (shelf-card-titles) for the terms found in papers / slide decks only (Term::cloud_ok), with only their sentences; the rest wait for the local model
cargo run --release -p loreshelf-cli -- plan [<source> [page]]  # 歸位 phase A: where each file / folder should go (`<catalog>/filing-plan.json`); nothing moves
cargo run --release -p loreshelf-cli -- move <source> [--dry]  # 歸位 phase B (CLI): move the rows the rules are sure of (`filing::certain`), rename only, journaled
cargo run --release -p loreshelf-cli -- undo  # move the last batch back (`<catalog>/filing-journal.jsonl`)
cargo run --release -p loreshelf-cli -- guess [<source>]  # 歸位 phase D: rows the rules left open → Haiku (paper / deck text only) or the local model (the rest); `<catalog>/filing-ai.json`
cargo run --release -p loreshelf-cli -- move <source> --model [--dry]  # move the rows the model placed (only after the author looked at the --dry list)
cargo run --release -p loreshelf-cli -- ignore-words [--write]  # general words + names → private/LoreReader/ignore-words.txt (「這不是術語」, every paper reader; lines tagged `# LoreShelf`)
cargo run --release -p loreshelf-cli -- card-content      # Claude Haiku writes each kept card's zh-TW summary + 2–4 sentence body (a card never gets PDF quotes)
cargo run --release -p loreshelf-cli -- known-cards --refill [--write]  # cards LoreShelf made with quotes only (progress log `via: loreshelf`) get those words, through KnowledgeBase's API
cargo run --release -p loreshelf-cli -- study-cards [--write]  # terms of the fields in private/shared/learning.json → 不熟 (learning) cards (NeonDeck's 知識卡輪播); paper terms by Claude, the rest by the local model
cargo run --release -p loreshelf-cli -- ignore-words --recheck [--write]  # words called general under the old rule that are not everyday: off the ignore list, asked again by classify
cargo run --release -p loreshelf-cli -- card-titles       # Claude Haiku (subscription CLI, else ANTHROPIC_API_KEY; never the keychain) checks the mastered terms once and titles the cards; paper / slide terms and sentences only
LORESHELF_HOME=/tmp/shelf LOREFORGE_CACHE=/tmp/cache cargo run -p loreshelf-cli -- scan <dir>   # throwaway catalog
```

`cargo test` here and for `lore-catalog` / `lore-gitsync` must pass before every commit.

## Layout

- `crates/cli` (package `loreshelf-cli`, `loreshelf-cli` binary): `scan <dir>...`, `report`. Reads `LORESHELF_HOME` and the
  cache dir; everything else is in `lore-catalog`.
- `crates/server` (`shelf-server`): axum on 127.0.0.1 only. Serves `web/` (no-cache), the shared
  theme (`/theme.css`, `/theme.js`, `/api/theme`) and shell kit (`/icons.js`, `/base.css`,
  `/shell.js`), `/api/health`, and a read-only API over the catalog: `GET /api/files?limit=`
  (names only, no paths), `GET /api/files/{id}` (+ `paths`, generated `description`),
  `GET /api/neighbors/{id}?k=12&layers=2` (`index.rs`: name tokens IDF cosine, time kernel,
  kind/ext, download host, shared folder; `parts` = each factor's share), `POST /api/open/{id}`
  (`openwith.rs`, one table `BY_EXT`: PDF → LoreReader, .pptx → SlideComposer, .docx / .tex / a LaTeX
  project's `main.tex` → PaperComposer, `open -a <App>`; the app not in `/Applications` (`Runner::installed`)
  or any other type (.key / .ppt / .doc: no Lore app reads them) → the system default; datasets as before;
  an `.app` is revealed, never launched), `POST /api/reveal/{id}` (`open -R`). `GET /api/thumb/{id}?s=512`
  (`thumbs.rs`: QuickLook `qlmanage -t` through the `Runner`, 5 s timeout, 2 at a time, cache
  `<cache>/LoreForge/shelf/thumbs/<id>-<s>.png` + `.none` markers, LRU 200 MB; never app bundles,
  datasets, symlinks or dataless files; a project = its `main.pdf`; 404 = kind icon),
  `GET /api/timeline` (`timeline.rs`: 本週 / 上週 / 本月 / 上個月 / seasons for two years / years;
  time = downloaded > doc_created > created). `group` / `group_size` on items and one neighbour
  per group with `members` (`index.rs`: name stem without copy suffixes and extension; `-N`
  only when the plain stem exists; only the document family `DOC_FAMILY` groups by name, others
  only by identical content; `GENERIC_STEMS` / `GENERIC_NUMBERED` / `GENERIC_PREFIXES` never
  group; the focus's own copies are not neighbours). The opener is the
  injected `Runner` (tests record calls). The index is rebuilt when a new report appears.
  Categories (`lore_catalog::category`): items carry `categories` (type + use) and `sensitive`
  (證件); `cat=a,b` filters `/api/files`, `/api/neighbors`, `/api/timeline`, `/api/nearest?from=`;
  `GET /api/categories`; `PUT /api/files/{id}/categories` = the author's use categories through
  `record_edit` (local commit), replacing the rules. A use category the author makes up is `u:<name>`
  (`category::custom_name`: ≤ 24 characters, no comma); `/api/categories` lists the ones in use (`custom: true`). Datasets (`dataset.rs`): `/api/files/{id}`
  adds `dataset` (files, main ext share, 5 sampled image sizes, top-level label files, first-level
  folders ≤ 20) and `mentions`; `GET /api/datasets/{id}/items?offset=&limit=60` (relative paths,
  `symlink_metadata` only); `GET /api/thumb/ds/{id}/{index}`; `/api/thumb/{dataset}` = 2×2 mosaic
  (`image` crate). Dataset ↔ paper: the dataset folder name (words, or joined; its first word
  alone only when the rest is version / size / format words) in a paper's file name, or in its
  first-pages text within 120 characters of a dataset word (dataset, database, cohort,
  benchmark…) → `parts.mention` (weight 0.5).
  Search (`search.rs`): `GET /api/search?q=&cat=&limit=` — every term must match the name, the
  first pages' text (`Entry::text`), a category label, the host, a folder on the path, or the start
  of a time; `{ total, items: [{ item, via, snippet }], facets }` (facets = categories, years, hosts
  that would narrow; nothing asked → only facets; only `cat` → its items, newest first; a sensitive
  item never gets a snippet). `?loose=1` = the query is a spoken sentence (LoreEcho): `search::loose`
  (`sentence_tokens`: Latin words ≥ 3 letters + CJK bigrams, weighted by rarity among names; name >
  category > text; only files reaching `LOOSE_MIN`). `POST /api/echo/open {right?, bottom?, height?}` =
  `open -g -a LoreEcho`, then LoreEcho's `PUT /api/window` (ports.json `le`) seats it in that corner at
  `ECHO_W` (the 開啟時同步開啟 setting; the page then centres the globe between the filters and it).
  `POST /api/act {focus}` + `GET /api/events?after=` (long poll, `AppState.acts`): another local app
  turns the page to a file. `/api/health` carries `build` (the program file's mtime): the desktop app
  joins a running server only of its own build.
  Whole text (`fulltext.rs`): `Index.ft` = token → (entry, part) only (Latin words, CJK pairs;
  a Latin query word matches tokens it starts); the text is read from `<cache>/fulltext/` only for a
  shown snippet; search results carry `at` (第 30 頁 / 第 4 張投影片 / 第 3 段). `/api/search` text hits
  and `loose` use it (a text-only hit weighs by its rarity among the texts, so one rare word passes).
  The index is rebuilt after a new report or a finished `loreshelf-cli extract` (`.stamp`). Items carry
  `cloud_ok` (a paper or a slide deck, never a sensitive item). Model calls (`llm.rs`, `tasks.toml`):
  `shelf-classify-terms`, local only, background priority (`classify.rs`: batches of 20, skipped keys
  asked again at once, saved per batch, mastered terms first, then terms next to known cards;
  `settle` = a single everyday word (`lore_words::Filters::is_everyday`: general, function or familiar, never TERMLIKE;
  author 2026-11-14: a basic word is one an ordinary middle schooler knows, a technical sense is a term) is never a term; `ignore_lines` = general words and names for the shared ignore list). The pool
  never takes the ignore list's words or my-vocab's. `shelf-card-titles` = Claude Haiku (`cards.rs`):
  keep / title / field per mastered `term` with `cloud_ok`, answers in `<cache>/card-titles.json`
  (beside the pool, so a running classify never drops them); `known-cards` makes one card per title,
  none the KB already has.
  歸位 (`filing.rs`, phase A, plan only): settings `<catalog>/filing.json` (`root` = `~/Documents/Shelf`,
  `sources`, `keep` = folders apps use); rows = a folder whose contents agree (course folders under
  `courses/[<school>/]<term>/<course>`, a research folder = a topic of its own `Research/<path below the
  opened container>`, a tool's fonts → `Software/<name>`), else its parts; projects / datasets / packages
  one row; personal papers by name (`PERSONAL` words, never a paper) → `Personal/{IDs, Finance, Medical,
  Contracts}`, the pictures beside ID scans with them, and never inside another folder's move; what
  rules cannot place stays (never `Archive` by default); hidden files skipped; flags `need-topic`,
  `need-name` (not plain ASCII), `sensitive`, `duplicate`. Name words (`BY_NAME`: School/{IRB, Funding, Travel, Forms}, Career,
  Templates, Software/Manuals, Learning/Languages, Research/?/Manuscripts, Library/Books) place a file
  only, never its folder (`Place::by_name`); `secret` names (lore-catalog `category::secret_name`) →
  `Personal/Secrets`, never read into full text. Phase B from the CLI: `move` = `filing::certain` rows
  (rename only, `free` name beside a taken one, journal per batch, `undo` = last batch back); the
  plan drops paths no longer on disk. Phase D (`filing_ai.rs`): open rows → Haiku with the text alone
  (`d1`, `d2` keys; no name / path / folder) or the local model (name + text; answer must be one of
  `TARGETS`); topic joins an existing `Research/<Topic>` by `match_topic`, else `new-topic`; labels
  (3–5 English) stay in `filing-ai.json` for LoreShelf's own search (no Finder tags, author
  2026-11-13); `by-model` rows are never `certain`.
  `web/` (no build step; shared files from root paths): `app.js` = the globe (the focus at the pole of a
  wire sphere, neighbours on it as small round nodes, drag = turning it, `place` → `toVec`; canvases
  `lback` ¼ res / `lglobe` ½ res / `lbond` full res, nodes are DOM), the callouts outside the limb
  (`paintAxes`: groups of neighbours near for the same reason, said concretely; a neighbour's `why`
  from the server wins when present), the hover preview (click = open, folder button = Finder), the search field in the top bar
  (`/`; chips = the server's facets, Enter = go to that file on the globe), the
  readout at the bottom-left with the 用途 chips (`+` → known ones or a new `u:<name>`), filters,
  timeline, dataset view and the release tiers; `scene.js` = frame scheduler and kind pictures;
  `compass.js` = factor shares → angle. Colours only from theme tokens (`readTheme`).
- `apps/desktop` (package `loreshelf`, Tauri 2): shelf-server in-process on 7900 (next free port
  if taken; an existing shelf-server answering `/api/health` there is reused), ports.json `ls`.
  One transparent window (`macOSPrivateApi`; the usual window buttons over the page, `TitleBarStyle::Overlay`, no title — the red button quits the app, full screen works, the saved size skips full screen) with vibrancy
  (`Effect::FullScreenUI`) behind the page. Regular app: Dock icon, menus (app 隱藏 / 結束 ⌘Q,
  編輯, 視窗 最小化 ⌘M / 收起 ⌘W); Dock click (`RunEvent::Reopen`) hides the window when it is in
  front, else shows it. Page → app: `window.__TAURI__.core.invoke("hide")` (Esc), allowed through
  the app manifest in `build.rs` + `capabilities/default.json` (`allow-hide`, remote 127.0.0.1).
  App → page: `window.LoreShelf && LoreShelf.setActivity("active" | "blurred" | "hidden")` on
  focus / blur and, polled once a second on the main thread, hide / minimise / macOS occlusion
  (`NSWindow.occlusionState` via objc2); sent only on change. Icon source `icons/logo.svg`; to
  change it, export a 1024 px `logo.png` (`sips -s format png -Z 1024 icons/logo.svg --out
  icons/logo.png`) and run `cargo tauri icon icons/logo.png -o icons` in `apps/desktop/`, keeping
  only the files `tauri.conf.json` lists (+ `icon.png`, `logo.*`).
- Catalog (not in this repo): `$LORESHELF_HOME` or `~/Library/Application Support/LoreForge/LoreShelf`
  — scan output `files/<sha256>.json`, `projects/`, `packages/`, `datasets/`, `reports/`
  (git-ignored, never committed); the author's edits `edits/<id>.json` (committed one by one,
  `lore_catalog::Catalog::record_edit`); a local git repo with `.lore-local-only` and a refusing
  `pre-push` hook. Hash cache: `<cache>/LoreForge/shelf/`.

## Decisions

- **Files move only in a batch the author confirms** (歸位, author 2026-11-13). Scans only read
  (no xattrs written, no Finder tags, no renames); placeholders and dataless iCloud files are never
  opened (no downloads). Phase A (`filing.rs`, `loreshelf-cli plan`) only writes a plan; the move
  itself (phase B: popup, 20 rows a page, select all, confirm, undo per batch; same volume = plain
  rename, other volume = copy, check the hash, then remove) comes later. Every folder name of the
  new tree is English.
- **Catalog is local-only.** It holds names and paths: never in a repo with a remote, never in
  iCloud Drive or this repo, never pushed (hook + `lore-gitsync` refusal). A cloud model may
  read the text of a paper or slide deck (`cloud_ok`), never a file name, path or folder name;
  every other file stays with the local model.
- **Code is public, data is not.** Tests use temp dirs and synthetic files; no real paths or file
  names in code, docs or tests. Example users `ellrin`, `daidai`.
- Projects, macOS packages (`.app`, …) and datasets (> 500 files, ≥ 90 % one extension) are one
  record each; their insides are never catalogued file by file. Git projects are identified by
  their root commit (clones = one record); other units by path + fingerprint (moves keep the record).
- `build` / `target` / `node_modules` are skipped only inside projects.
- The local git versions only the author's edits; scans never commit.
- Records merge on rescan; unknown fields are kept; reports never overwrite each other.
