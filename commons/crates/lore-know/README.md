# lore-know

The author's private know-how as one database (design: `docs/brain-hub.md`). L-Brain is its
screen. Every read, sort, filter and search goes through a SQLite index; every write goes through
one place (`Store`, behind the `Sink` trait). The items themselves stay what they were: text
records in the hub folder, or the app's own CSV rows, notes, folders and drawings, which the hub
reads and writes in place. The index is a cache and is rebuilt from them.

## Words

| word | meaning |
|---|---|
| **format** | a fixed shape of knowledge: its fields, how a list shows it, how it is written into a prompt. `pattern` `expression` `wording` `rule` `exemplar` `note` `bundle` `layout` `figure` (`card` is registered but has no format here yet) |
| **category** | a named set of items in one format, with an owner: a Lore app's (`pc.patterns`), the author's by hand (`my.<name>`), or an outside application's independent set (`x.<source>.<name>`) |
| **source** | who gave an item: `author`, `lore:<app>`, `external:<name>`. An outside application is only ever a source; it cannot make a format or a category of its own kind |
| **item** | one piece of knowledge, `<category>:<local id>` |

Rules the code keeps: an outside application aligns to an existing category or keeps an
independent set; no Lore step reads an independent set (`select`); an AI changes knowledge only as
a `Proposal` the author approves; a reference picture is never made into a drawable entity
(`entity = false`); an entry that does not fit is listed in the report, never coerced or dropped
silently; removing goes to the trash and can be restored.

## Modules

| module | what |
|---|---|
| `types` | the contract: `Item`, `Category`, `Source`, `Format`, `Adapter`, `Sink`, `Proposal`, ids |
| `registry` | categories from the apps' manifests, the author's own and independent sets (`<hub>/categories.toml`) |
| `store` | the hub folder behind `Sink`: records, overlays (tags, triggers, usage, source) beside an app's own data, trash, stale writes refused |
| `index` | SQLite: eight sort keys, filters (category, tags, source, level, entity), Latin and CJK search, counts |
| `formats` | `table` (pattern, expression, wording, rule, exemplar, `CsvAdapter`), `text` (note, bundle, layout and their adapters), `figure` (entities from the flow tool, references, thumbnails, saved drawings); `formats::all()` is the id → format table |
| `proposal` | batches an AI or a tool proposes: show, apply (all or some), undo |
| `intake` | outside applications: the batch file, import (whole-set replace per source), export |
| `select` | what applies to a step and its content, written into a prompt, and the report of what was used |
| `decompose` | a figure's picture (the model's reply) into figure / modules / components as a proposal |
| `hub` | all of it opened together for an app: `Hub::open` |

## Batch file (intake)

One JSON file, the same for every application. `source` names the application; an empty
`source` is the author's own file (an export of a Lore or hand-made category) and is only valid for
`align`.

```json
{
  "lore_know": 1,
  "source": "wordbox",
  "target": { "kind": "align", "category": "pc.patterns" },
  "items": [
    { "id": "7", "title": "Little is known about X.",
      "fields": { "section": "intro", "template": "Little is known about X." },
      "tags": { "use": ["paper"] }, "triggers": ["gap"] }
  ]
}
```

`target` is `{"kind":"align","category":"<id>"}` (join an existing category, in its format) or
`{"kind":"independent","name":"Ideas","format":"note"}` (a set of its own, never read by a step).
Adding the same source to the same target again replaces only what that source gave before:
entries are matched by `id` (or, without one, by their content), what it no longer gives is moved
to the trash. A category whose app data names rows by their own words (wording: `avoid`) matches an
entry that says exactly what a row already says; a changed one is written as a new row.

## An app declares its categories

`know.toml`, included in the app's code and passed to `Hub::open`:

```toml
[[category]]
id = "pc.patterns"
label = "句型"
group = "寫作準則"
format = "pattern"
tasks = ["draft", "revise"]     # lore-llm tasks that read it; empty = 未接上
writable = true
[category.native]
kind = "csv"                    # "" records · csv · files · dir · list · flow
app_dir = "PaperComposer"       # a key of the `app_dirs` map
path = "patterns.csv"           # relative to that folder

[[category]]
id = "pc.journals"
label = "期刊模板"
format = "bundle"
[category.native]
kind = "dir"
app_dir = "PaperComposer"
path = "templates"
```

Native kinds and formats: `csv` (pattern, expression, wording, rule, exemplar), `files` (note,
layout), `dir` (bundle, figure references), `list` (layout list in one JSON file), `flow` (the flow
tool's pieces and saved drawings), `""` (the hub's own records). A missing folder or file is an
empty category. `writable = false` still takes tags, triggers and links (kept beside the data).

## What an app's server needs

```rust
let mut hub = Hub::open(hub_dir, &[KNOW_TOML], &app_dirs, &flow_dirs)?;   // app_dirs: Native.app_dir -> folder

hub.counts()?;                    // dashboard: one Count per category
hub.query(&Query { .. })?;        // (items, total) — sort, filter, search, paging
hub.store.get(id)? / hub.store.list(cat)? / hub.store.problems(cat)
hub.put(item)? / hub.remove(id)? / hub.restore(id)?   // keep the index in step; put returns the stable id
hub.import(&batch)? / hub.export(cat, source)?        // intake; intake::parse / to_text for the file
hub.proposals.create(by, title, changes)? / .list(Some("open")) / .dismiss(id)
proposal::show(&hub.store, &p)?   // before / after for the author
hub.apply_proposal(id, chosen)? / hub.undo_proposal(id)?
hub.thumbnail(id)?                // SVG drawn by the flow tool
hub.record_use(&ids)?             // what a step took into its prompt
hub.reindex()?                    // after files changed outside the hub
```

For a step that builds a prompt: `select::select(cats, items, &Ask { task, text, .. })`, then
`select::render(&picked, &hub.formats)`, `select::report`, and `hub.record_use(&select::used_ids(&picked))`.
For the hub's own category list: `hub.store.registry()` / `registry_mut()` (`add_manual`,
`add_external`, `set_tasks`, `rename`, `remove`). Write through the `Hub` methods, not through
`hub.store`: the index follows only those.

`cargo run -p lore-know --example thumbs -- <out dir> [flow dir]` draws every entity of the flow
catalogue to SVG files, to look at.
