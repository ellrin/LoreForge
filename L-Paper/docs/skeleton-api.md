# 骨架（skeleton）— 前後端合約

Design: [`docs/long-paper.md`](../../docs/long-paper.md) §G. This file is the contract between
`pc-server` and `ui/skeleton.js`. The fixture
[`crates/core/tests/fixtures/skeleton_view.json`](../crates/core/tests/fixtures/skeleton_view.json)
is one full `View`; a core test reads it into the Rust types and writes it back unchanged, and the
front-end demo (`ui/skeleton-demo.html`) draws it. Change the three together.

All routes are under `/api/papers/{name}/draft/skeleton`. Every POST answers with the new `View`
(same as GET) unless noted. Errors: 4xx/5xx with a plain-text reason in Traditional Chinese.
Field names are snake_case. Ids are stable for the life of the skeleton and never reused.

## View

```jsonc
{
  "rev": 12,                    // grows with every change; the page drops answers older than what it shows
  "exists": true,               // false: no skeleton yet (the page offers 素材清單 → 排骨架, or 改用骨架)
  "can_convert": false,         // an old session with groups/outline/drafts that /convert can turn into a skeleton
  "job": null,                  // or { "kind": "plan|save|write|convert", "label": "排骨架", "done": 3, "total": 8, "error": "" }
  "dirty": false,               // edits since the last 暫存
  "can_undo": true,
  "targets": { "figure": [4, 7], "table": [2, 4], "equation": [3, 10] },   // the journal's habit, or null
  "inventory": [ Inv ],
  "sections": [ Section ],
  "threads": [ Thread ],        // in the order they are asked
  "next": "q7"                  // the open thread to show now, or null
}
```

```jsonc
// Inv — one thing the paper could present
{ "id": "v3", "kind": "finding|table|figure|flow|equation|algorithm",
  "title": "…", "note": "…",            // what it would support
  "evidence": ["E2", "T3"],             // record ids that exist
  "label": "tab:main",                  // what the text \ref's ("" for a finding)
  "keep": true, "by_author": false, "pending": false,
  "tool": "table|plot|flow|image|equation|algorithm|" }

// Section
{ "id": "s3", "title": "Results", "role": "results",   // introduction|related|methods|results|discussion|conclusion|other
  "blocks": [ Block ] }

// Block — a frame around a few paragraphs
{ "id": "b4", "title": "解釋表 1", "purpose": "…",
  "of": "b2",                           // the block or paragraph it expands, or null
  "paras": [ Para ] }

// Para
{ "id": "p9", "kind": "Results key result",   // a paragraph type of the author's style, or ""
  "say": "A 對 B：…",                          // what it says, one line
  "points": [ { "text": "…", "from": "q7" } ], // `from`: the thread it grew from, or null (the planner's)
  "link_prev": "…", "link_next": "…",          // how it follows / hands over ("" at the ends)
  "jump": null,                                // or { "why": "…", "fix": "…" }: too far from the paragraph before
  "floats": ["tab:main"], "cites": ["key2024"], "sources": ["E2"], "pending": ["P1"],
  "text": "",                                  // LaTeX prose once written
  "state": "outline|points|text|done",
  "busy": null,                                // or "fill|write|refine|repair"
  "problems": ["…"],                           // checks on `text`
  "reason": "",                                // the author's reason for adding / moving it, until the next 暫存
  "touched": false }                           // edited since the last 暫存

// Thread — one entry in the margin, anchored to a paragraph or a block
{ "id": "q7", "anchor": "p9",
  "kind": "conflict|choice|missing|note|edit|refine",
  "status": "open|answered|skipped|done",       // answered = sent, being folded in; done = folded in
  "question": "…",
  "options": [ { "id": "o1", "text": "…" } ],
  "messages": [ { "who": "ai|me", "text": "…" } ],
  "conclusion": "…",                            // one line once done
  "changes": [ Change ] }                       // edit / refine threads

// Change
{ "id": "c1", "target": "p9", "field": "say|link_prev|link_next|purpose|text|points",
  "before": "…", "after": "…", "why": "…",
  "state": "proposed|applied|reverted|rejected" }   // edit: applied (revertible); refine: proposed
```

A block's state is the lowest state of its paragraphs (the page works it out).

## Routes

| Method | Path | Body | Does |
|---|---|---|---|
| GET | `` | | the `View` |
| POST | `/inventory` | `{ "keep": ["v1"], "add": [{ "kind", "title", "note" }] }` | the author's choice of what to present (`keep` = every kept id) |
| POST | `/plan` | `{ "redo": false }` | plan the skeleton in the background (`job.kind = "plan"`); `redo` replaces an existing one after an archive copy |
| POST | `/ops` | `{ "ops": [ Op ] }` | edits, applied in order under the lock; never calls a model |
| POST | `/save` | `{}` | 暫存: repair what was touched and its neighbours (background, `job.kind = "save"`); makes `edit` threads |
| POST | `/answer` | `{ "thread", "text", "picked": ["o1"], "skip": false }` | answers at once with `next` moved on; the fold-in runs in the background (`para.busy = "fill"`) |
| POST | `/note` | `{ "anchor", "text" }` | the author says something about a paragraph or block unasked: a `note` thread, folded in like an answer |
| POST | `/write` | `{ "id" }` | write the text of a paragraph, a block, a section or `"all"` (background, `para.busy = "write"`) |
| POST | `/refine` | `{ "para", "preset": "wording|style|check" }` | a `refine` thread with proposed changes |
| POST | `/change` | `{ "thread", "change", "accept": true }` | take or drop one change (`edit`: false = revert) |
| GET | `/style` | | `StyleReport` |
| POST | `/convert` | `{}` | an old session → a skeleton (archive copy first) |
| POST | `/place` | `{ "section": "s3" }` or `{}` | 放進論文 (all sections when empty) |

```jsonc
// Op
{ "op": "add_para",  "block": "b4", "at": 2, "say": "", "reason": "" }      // → new id in the View
{ "op": "add_block", "section": "s3", "at": 1, "title": "", "of": null }
{ "op": "remove",    "id": "p9" }                                            // a paragraph, or a block with its paragraphs
{ "op": "move",      "id": "p9", "to": "b5", "at": 0, "reason": "" }         // a paragraph into a block; a block into a section
{ "op": "edit",      "id": "p9", "fields": { "say": "…" } }                  // para: say kind text points; block: title purpose of
{ "op": "reason",    "id": "p9", "reason": "…" }
{ "op": "done",      "id": "p9", "done": true }                              // 細修 finished (state text ⇄ done)
{ "op": "undo" }
```

The page polls GET every 800 ms while `job` is set or any paragraph is `busy` or any thread is `answered`.

```jsonc
// StyleReport
{ "sections": [ { "id": "s3", "title": "Results",
    "rows": [ { "id": "sentence_words", "label": "每句字數", "mine": [17, 25], "draft": 31.5, "ok": false } ],
    "paras": [ { "id": "p9", "issues": ["句長變化太小"] } ] } ] }
```
