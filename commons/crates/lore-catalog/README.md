# lore-catalog

LoreShelf's read-only file catalog (design draft `docs/loreshelf.md` §1–3, wave 1). Knows
nothing about any app.

```toml
lore-catalog = { path = "../commons/crates/lore-catalog" }
```

## Rules

- Never modifies, moves, renames or writes into a scanned folder. Never follows symlinks
  (`symlink_metadata`, counted as skipped). Never opens an iCloud placeholder (`*.icloud`) or a
  dataless file (`SF_DATALESS`; opening it would download it). Skips `.DS_Store`.
- Units — one record each, never hashed file by file (`project.rs`):
  - **project**: a folder with markers (`.git`, `main.tex`, a `.tex` + a `.bib`, `Cargo.toml`,
    `package.json`, `*.xcodeproj`, `pyproject.toml`). Inside, only `.git/config` (remote URLs,
    credentials stripped), `.git/logs/HEAD` (timestamps only) and the root commit
    (`git rev-list --max-parents=0 HEAD`, `GIT_OPTIONAL_LOCKS=0`) are read.
  - **package**: a macOS package folder by extension (`PACKAGE_EXTS`: `.app`, `.framework`,
    `.bundle`, `.photoslibrary`, `.pages`, …), counted by `symlink_metadata` only.
  - **dataset**: a folder with more than `DATASET_MIN_FILES` (500) files of which at least
    `DATASET_MIN_SHARE` (90 %) share one extension, counted the same way.
  - `build`, `target`, `node_modules` are skipped only inside projects (which are not walked);
    elsewhere they are scanned like any folder. A scan root itself is never a unit.
- Unit ids: a git project with commits = `proj-git-<root commit>` (two clones are one record with
  two paths). Otherwise `proj-` / `pkg-` / `data-` + 16 hex of sha256(resolved path), plus a
  `fingerprint` (projects: marker types + sorted top-level names; packages: name + file count +
  bytes; datasets: main extension + file count + bytes + top-level names). A unit found at a new
  path whose fingerprint matches a stored record of the same kind whose paths all vanished is
  merged into that record (a moved folder keeps its record).
- Catalog location is guarded (`Guard`): not in iCloud Drive / `~/Library/CloudStorage`, not in
  the LoreForge repo, not in any git work tree with a remote. `Catalog::open` `git init`s it,
  writes `.lore-local-only`, installs a `pre-push` hook that always fails and sets a repo-local
  `core.hooksPath` (a global hooks path cannot bypass it); a remote added later stops the next
  scan. Its `.gitignore` ignores everything except `edits/`, `.gitignore` and the marker: scans
  never commit; only the author's edits (`Catalog::record_edit`) are versioned. `lore-gitsync` refuses network steps in any repo with `.lore-local-only`.
- Records merge on rescan (`first_seen` kept, unknown fields kept), atomic writes; reports get
  free names.

## Layout

| What | Where |
|---|---|
| One record per content | `<catalog>/files/<sha256>.json` |
| One record per unit | `<catalog>/projects/`, `packages/`, `datasets/` `<id>.json` |
| The author's edits (tags, …), versioned | `<catalog>/edits/<id>.json` — `record_edit(id, field, value)` writes it and commits it alone (`<field>: <id>`) |
| Reports | `<catalog>/reports/<YYYY-MM-DD>-scan[-n].{md,json}` |
| Hash cache (wipeable) | `<cache>/LoreForge/shelf/hash-cache.json`, keyed by path, valid while (size, mtime ns, inode) match; also keeps sniffed kind and document dates |

Catalog default: `$LORESHELF_HOME`, else `<data dir>/LoreForge/LoreShelf`
(`~/Library/Application Support/…` on macOS).

## Extract cache and categories

- `extract`: per content, filled by the scan for PDFs and images, in the cache tier
  `<cache>/LoreForge/shelf/extract/<sha256>.json` = `{pages, text (first 2 pages, ≤ 20,000 chars;
  lopdf ≤ 60 MB, panic-guarded), width, height (image header only)}`. `image_size(path)`.
- `category`: type categories (`TYPES`, exact from kind) and use categories (`USES`: 論文 `paper`,
  證件 `id-doc`, 作業 `homework`, 票券收據 `ticket`) inferred by the rule table `RULES`
  (`{category, reason, confidence, test}`; membership at `INCLUDE_MIN` 0.5): DOI / arXiv id /
  journal markers in the first pages, publisher host, LaTeX project; ID-document words in names (a card-shaped
  image or a one-page scan only adds confidence to such a name, never on its own); homework and
  ticket words.
  `SENSITIVE` = `id-doc`. `Filter::parse("a,b")`: OR within a group, AND across groups. The
  author's choice (`edits/<id>.json` `categories`, via `record_edit`) replaces the rules.

## File record

```json
{
  "sha256": "…", "size": 1234, "kind": "pdf",
  "names": ["paper.pdf"],
  "seen_at": [{"path": "/…/paper.pdf", "first_seen": "…Z", "last_seen": "…Z"}],
  "times": [{"axis": "downloaded", "value": "2024-01-02T03:04:05Z", "source": "xattr:com.apple.quarantine"}],
  "download": {"urls": ["https://…"], "referrer": "https://…", "agent": "Safari"}
}
```

Axes and sources: `fs_created` / `fs_modified` (`fs`), `downloaded`
(`xattr:com.apple.quarantine`), `last_opened` (`xattr:com.apple.lastuseddate#PS`),
`doc_created` / `doc_modified` (`pdf:Info` via lopdf's metadata reader ≤ 200 MB, panic-guarded,
then a byte scan of both ends; `ooxml:docProps/core.xml`); projects add `git_first` / `git_last`
(`git:logs/HEAD`). Kinds come from the extension, else from the first bytes. Author names
(PDF `/Author`, `cp:lastModifiedBy`, reflog names / e-mail) are never read into records.

## API

```rust
scan(&ScanConfig { roots, catalog_dir, cache_dir, guard: Guard::default() })
    -> Result<ScanOutcome { report: Report, report_md, report_json, catalog }, CatalogError>
store::{default_catalog_dir(), default_cache_dir(), Guard::check(dir), Catalog::open(dir, &guard),
        Catalog::latest_report(dir), catalog.record_edit(id, field, value) -> Option<commit>,
        catalog.edit(id), write_atomic, LOCAL_ONLY_MARKER, GITIGNORE}
record::{FileRecord::{load, save, observe}, TimePoint, Download, SeenAt, axis::*}
project::{markers, is_package, Tally, inspect, inspect_package, inspect_dataset, save_units,
          UnitRecord (= ProjectRecord), root_commit, url_host, strip_credentials, log_timestamps,
          PACKAGE_EXTS, DATASET_MIN_FILES, DATASET_MIN_SHARE}
times::{fs_times, xattr_info, parse_where_froms, parse_quarantine, parse_last_used,
        parse_pdf_date, pdf_dates, ooxml_dates, core_xml_dates}
hash::{sha256_file, HashCache, FileKey}
report::{build, markdown, write, human}
```

`CatalogError`: `Guard` (location refused / git), `Root` (a scan root cannot be read — e.g. macOS
privacy settings), `Io`.

```sh
cargo test -p lore-catalog   # temp dirs, synthetic PDFs / pptx / git dirs / xattrs (macOS)
```
