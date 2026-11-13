# lore-jobs

Long-running background jobs for any app: a list of stages run in order on a spawned tokio
task, one checkpoint file per stage, progress for the page, stop and resume, an optional wait
for the author's review, redo from a stage. First user: SlideComposer's 論文報告
(`docs/paper-deck.md` §工作與階段, §API); next: PaperComposer's 寫書.

No HTTP: the app maps its own routes onto `Jobs` and returns `Jobs::view(&job)` (the job object).

## On disk

`<root>/<id>/job.json` (options, status, per-stage time and counts) and `<root>/<id>/<stage>.json`
per finished stage (`{stage, output, counts, elapsed_ms, finished}`), all written atomically.
A stage runs iff its file is missing or unreadable. Jobs found running or waiting when the process
started again are `interrupted`. Other files in the job folder belong to the pipeline.

## API

| Item | What |
|---|---|
| `trait Pipeline` | `stages() -> &'static [StageSpec]`, `run(&StageCx, stage) -> StageFuture` (output = checkpoint), `decorate(&Job, &mut Map)` (extra fields of the job object) |
| `StageSpec { id, label, model, wait }` | `model`: streams characters; `wait`: the author's review (skipped when the job is `direct`) |
| `StageError { status, message, extra }` | HTTP-like status, one line, extra error fields; `From<String>` = 500 |
| `StageCx` | `dir()`, `params()`, `output(stage)` (earlier stage), `counts(v)`, `counter()` (for blocking threads), `chars(n)`, `title(t)` |
| `Jobs::open(root, pipeline)` | the jobs of a folder |
| `Jobs::create(params, direct, title)` | new job, started at once |
| `get`, `list` (newest first), `view(&job)`, `specs()` | |
| `start(&job)` | resume from the first stage without a checkpoint (`false` if running) |
| `submit(&job, stage, output)` | the author's input for the waiting stage |
| `redo_from(&job, stage)` | drop that checkpoint and later ones, run again |
| `set_params(&job, f)`, `stop(&job)` (async), `delete(&job)` | |
| `Job` | `meta()`, `status()`, `is_running()`, `seq()`, `output(stage)`, `checkpoint(stage)`, `replace_output`, `cancel()`, `changed_after(seq, timeout)` (long polling), `settled()` |
| `status::{RUNNING, REVIEW, DONE, FAILED, STOPPED, INTERRUPTED}` | job states |
| `write_atomic`, `write_json`, `valid_id`, `now_ms` | helpers |

## Example

```rust
use std::sync::Arc;
use lore_jobs::*;
use serde_json::json;

struct Book;
const STAGES: &[StageSpec] = &[
    StageSpec { id: "collect", label: "收集", model: false, wait: false },
    StageSpec { id: "review", label: "確認", model: false, wait: true },
    StageSpec { id: "write", label: "撰寫", model: true, wait: false },
];

impl Pipeline for Book {
    fn stages(&self) -> &'static [StageSpec] { STAGES }
    fn run<'a>(&'a self, cx: &'a StageCx, stage: &'static str) -> StageFuture<'a> {
        Box::pin(async move {
            match stage {
                "collect" => Ok(json!({ "parts": ["a", "b"] })),
                _ => Ok(json!({ "from": cx.output("collect")?, "kept": cx.output("review")? })),
            }
        })
    }
}

let jobs = Jobs::open(cache_dir.join("jobs"), Arc::new(Book));
let job = jobs.create(json!({ "paper": "x" }), false, "寫書")?;   // runs until `review`
// … later, from the page:
jobs.submit(&job, "review", json!({ "keep": ["a"] }))?;            // goes on to `write`
let page = jobs.view(&job);                                         // the job object
```

`cargo test -p lore-jobs`: stage order, review wait / `direct`, cancel drops the stage future,
resume after a killed process, a corrupt checkpoint redone alone, `redo_from`.
