# lore-research

Find related papers on the web, **open access only** (`docs/book.md` §上網找論文): OpenAlex,
arXiv and Semantic Scholar → one record shape, de-duplicated; OA PDFs downloaded into a folder
the caller gives (the cache tier); BibTeX for every record; downloaded files into LoreReader's
library. Knows nothing about any app.

```toml
lore-research = { path = "../commons/crates/lore-research" }
```

## API

```rust
use lore_research::{Client, Config, Source, bibtex, import_into_reader};

let c = Client::new(Config::default());              // reqwest, generic User-Agent, no e-mail
let found = c.search("sparse attention", 10).await;  // Found { records, errors: [(Source, Error)] }
if found.offline() { /* 沒有網路: go on with KB cards and LoreReader papers only */ }

let r = c.resolve("10.5555/demo.2024.001").await?;  // DOI, arXiv id / link, or a title → Option<Record>
for rec in found.records.iter().filter(|r| r.open_access()) {
    let d = c.download(rec, &cache_dir).await?;      // <cache_dir>/<sha256>.pdf
}
let (keys, bib) = bibtex::bib(&found.records);       // unique keys, one entry per record
let rep = import_into_reader(&*c.http(), "http://127.0.0.1:7890", &files).await;
// rep.imported [{path, sha, new}], rep.pending [path] (LoreReader not running), rep.failed [(path, msg)]
```

- `Record`: `title, authors, year, venue, doi` (bare, lower-case), `arxiv` (no version),
  `abstract, is_oa, pdf_url` (an OA PDF only), `url, source, sources, citations`. JSON:
  `source` is `openalex | arxiv | s2`, the abstract is `abstract`.
- `Config`: `user_agent` (default `LoreForge/0.1 (+https://github.com/ellrin/LoreForge)`),
  `mailto` (OpenAlex's optional polite-pool e-mail; **None by default**, the author's setting),
  `s2_api_key` (optional, from the keychain), `sources`, `pace` (pause per source: OpenAlex
  0.2 s, arXiv 3 s, S2 1.1 s; 429 / 503 retried with `Retry-After` or doubling backoff from 2 s;
  `Pace::none()` in tests), `retries` (3), `max_pdf_bytes` (80 MB), `timeout`.
- `Client::search_in(source, query, limit)` asks one source. `search` asks each configured
  source in turn; a failing source is listed in `errors`, the others still count; after an
  offline source the rest are skipped.
- `download`: refuses a record without an OA PDF (`Error::NotOpenAccess`) before any request;
  tries arXiv's PDF first, then the OA location; checks `%PDF-` in the first KB (a landing page
  is `NotPdf`) and the size (`TooLarge`, also from `Content-Length` / while streaming); writes
  `<sha256>.pdf` atomically, keeps an existing one. `save_pdf(bytes, dir, max)` is that step alone.
- `Error`: `Offline` (connect / DNS / timeout — `is_offline()`), `Network`, `RateLimited`,
  `Status`, `Parse`, `NotOpenAccess`, `NotPdf`, `TooLarge`, `Io`; `Display` is one line of Chinese.
- `dedupe(records)`: same DOI, arXiv id or normalised title (`norm_title`: lower-case
  letters and digits) → one record: sources joined, first non-empty field, highest citation
  count, an arXiv PDF preferred. Helpers: `norm_doi`, `norm_arxiv`, `arxiv_from_url`,
  `find_doi`, `find_arxiv`.
- `bibtex`: `key(r)` = `doi:<doi>`, else `arXiv:<id>`, else `<last name><year><first title word>`
  (`example2024gains`); characters `\cite` cannot take become `-`; `keys` / `bib` add `a`, `b`, …
  to repeats in a set. Values escape `& % $ # _ { } ~ ^ \`; the title is double-braced;
  `@article` with a venue, else `@misc` (+ `eprint`, `archivePrefix = {arXiv}`). No macros are
  written (lore-notation: escaping only).
- `import_into_reader(http, base, files)`: `POST <base>/api/papers/import {path}` per file
  (LoreReader copies it into its sha-addressed library). The first "no network" failure makes
  that file and the rest `pending`; a refusal (`{error}`) is `failed` and the next file is tried.
  The report is serde, for a job's `pending_imports`.
- `Http` trait (`send(Request) -> Response`): `ReqwestHttp`, and `MockHttp` for tests (routes
  by URL substring, sequences such as 429 → 200, no route = offline, a request log). Apps'
  tests use `MockHttp` too.

## Tests

`cargo test -p lore-research` — offline. The fixtures in `tests/fixtures/` have the shape of the
real responses with synthetic content only.
