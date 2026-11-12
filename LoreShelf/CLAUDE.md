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
LORESHELF_HOME=/tmp/shelf LOREFORGE_CACHE=/tmp/cache cargo run -p loreshelf-cli -- scan <dir>   # throwaway catalog
```

`cargo test` here and for `lore-catalog` / `lore-gitsync` must pass before every commit.

## Layout

- `crates/cli` (package `loreshelf-cli`, `loreshelf` binary): `scan <dir>...`, `report`. Reads `LORESHELF_HOME` and the
  cache dir; everything else is in `lore-catalog`.
- `crates/server` (`shelf-server`): axum on 127.0.0.1 only. Serves `web/` (no-cache), the shared
  theme (`/theme.css`, `/theme.js`, `/api/theme`) and shell kit (`/icons.js`, `/base.css`,
  `/shell.js`), `/api/health`, and a read-only API over the catalog: `GET /api/files?limit=`
  (names only, no paths), `GET /api/files/{id}` (+ `paths`, generated `description`),
  `GET /api/neighbors/{id}?k=12&layers=2` (`index.rs`: name tokens IDF cosine, time kernel,
  kind/ext, download host, shared folder; `parts` = each factor's share), `POST /api/open/{id}`
  (an `.app` is revealed, never launched), `POST /api/reveal/{id}` (`open -R`). `GET /api/thumb/{id}?s=512`
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

- **No file moves.** Scanned folders are only read (no xattrs written, no Finder tags, no
  renames). Placeholders and dataless iCloud files are never opened (no downloads).
- **Catalog is local-only.** It holds names and paths: never in a repo with a remote, never in
  iCloud Drive or this repo, never pushed (hook + `lore-gitsync` refusal). Nothing about a
  file goes to a cloud model.
- **Code is public, data is not.** Tests use temp dirs and synthetic files; no real paths or file
  names in code, docs or tests. Example users `ellrin`, `daidai`.
- Projects, macOS packages (`.app`, …) and datasets (> 500 files, ≥ 90 % one extension) are one
  record each; their insides are never catalogued file by file. Git projects are identified by
  their root commit (clones = one record); other units by path + fingerprint (moves keep the record).
- `build` / `target` / `node_modules` are skipped only inside projects.
- The local git versions only the author's edits; scans never commit.
- Records merge on rescan; unknown fields are kept; reports never overwrite each other.
