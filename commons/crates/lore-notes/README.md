# lore-notes — PaperComposer 筆記與靈感

The notes PaperComposer lists under 筆記 (`docs/loreecho.md` §5, §4a). PaperComposer serves them
(`/api/notes`); LoreEcho writes the same files through this crate when PaperComposer is closed.
Pure Rust, no network, no app knowledge (the caller passes the folder and the registry).

```sh
cargo test -p lore-notes
```

## Where

`<PaperComposer workspace>/notes/` — normally `private/PaperComposer/notes/`:

```rust
let home = lore_home::Home::discover(&std::env::current_exe()?);
let dir = home.app_dir("PaperComposer").map(|d| d.join("notes"));   // Notes::new(dir)
let notation = home.shared_file("notation.json")
    .and_then(|p| lore_notation::Notation::load(&p).ok()).unwrap_or_default();
```

Created on the first write. `.trash/` (deleted notes), hidden files and anything not `.tex` are not
notes. The id is the file stem.

## File

`<YYYY-MM-DD>-<slug>.tex` (date of `created`; slug = the title's letters and digits of any script,
lower-cased, other runs → `-`, ≤ 40 chars; no title → from the body's first line, else `note`/`idea`).
A taken name gets `-2`, `-3` … (also when two writers race: the file is created with `create_new`).

```tex
% ---
% source: loreecho
% created: 2026-11-12T10:04:00+08:00
% kind: note
% kb: kb-1, kb-2
% tags: loss, training
% title: Why the loss weights matter
% ---
By \eqref{eq:loss}, \(\vw\) shrinks …
```

- keys: `source` (`loreecho` 學習夥伴 · `manual` 手寫 · anything else kept), `created` (ISO 8601 with
  offset), `updated` (written by edits), `kind` (`note` | `idea`), `kb`, `tags` (comma lists; a comma
  inside one item becomes a space), `title`; unknown keys are kept on rewrite.
- body: LaTeX, no preamble; use the registry's macros and `\ref{…}`/`\eqref{…}`.
- a `.tex` without the block (written by hand) is a note titled by its id.

## Check (every write)

`check(body, &notation)` runs `lore_notation::check_document` on the body as a fragment plus a
structure check. **Blocking** (`BLOCKING`; nothing is written): `hardcoded_reference` (a typed
"Table 1", "Eq. (3)"), `label_duplicate`, `registry_invalid`, `note_structure` (unbalanced `{}`,
`$`, `\begin`/`\end`; `\documentclass` or `\begin{document}`). Everything else is reported, not
blocking. `ref_undefined` is a warning (the label may be the paper's) and is dropped for labels the
registry knows. Finding lines count from the body's first line.

## API

```rust
pub enum Kind { Note, Idea }                       // serde: "note" | "idea"
pub struct Note { id, title, kind, source, created, updated: Option<String>,
                  kb: Vec<String>, tags: Vec<String>, extra: BTreeMap<String,String>,
                  body, modified: u64 /* file mtime, ms; not in the file */ }
pub struct NewNote { title, body, kind, tags, kb, source: Option<String> }   // POST body
pub struct Edit { title, body, kind, tags, kb: all Option }                   // PUT body
pub struct Finding { code, severity /* error|warning|info */, message, blocking: bool,
                     subject: Option<String>, line: Option<u32>, snippet: Option<String> }
pub struct Saved { id, path: PathBuf, findings: Vec<Finding> }
pub enum Error { Blocked(Vec<Finding>), NotFound(String), BadId(String), Empty, Io(io::Error) }

Notes::new(dir) / Notes::in_workspace(ws)            // ws/notes
notes.list() -> Vec<Note>                           // newest first (created, then mtime)
notes.get(id) -> Result<Note>
notes.create(NewNote, &Notation) -> Result<Saved>   // check, then a new file; source defaults to "manual"
notes.update(id, Edit, &Notation) -> Result<Saved>  // check, then rewrite in place; id stays
notes.delete(id) -> Result<PathBuf>                 // moves to .trash/ (free name there)
check(body, &Notation) -> Vec<Finding>  ·  is_blocked(&[Finding])
parse(id, text) -> Note  ·  to_tex(&Note) -> String  ·  slug(title)  ·  valid_id(id)  ·  unique_path(dir, stem)
```

## HTTP (PaperComposer, when it runs)

| Method | Path | |
|---|---|---|
| GET | `/api/notes` | `{notes: [Note…], dir}` newest first; `?q=` filters title, tags, body |
| GET | `/api/notes/{id}` | `Note` |
| POST | `/api/notes` | `NewNote` → `{id, path, findings}`; blocking → **422** `{error, findings}` |
| PUT | `/api/notes/{id}` | `Edit` → `{id, path, findings}`; blocking → **422** |
| DELETE | `/api/notes/{id}` | → `{trash}` (moved to `notes/.trash/`) |
| GET | `/api/notes/macros` | KaTeX macros of the global registry (for previews) |

LoreEcho finds the port in lore-home's `ports.json` (`pc`); without it, it writes with
`Notes::create` and PaperComposer lists the file the next time its 筆記 panel polls.
