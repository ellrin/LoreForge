# lens/ — what the draft interview cares about

「討論草稿」asks you about your study one question at a time. The engine is
neutral; a **lens** tells it what matters to you:

| Part | Meaning |
|---|---|
| `[scale]` | levels an answer is graded on (lowest first) |
| `[[flag]]` | marks an answer can earn (e.g. how the idea came about) |
| `[[mode]]` | how an item is treated: `ask = "deep"` (ask down to `min_depth`), `"confirm"` (only confirm it is routine), `"never"` (never ask, e.g. definitions) |
| `[[target]]` | what an item of a role needs before it is done (`min_depth`, `flags`, `number`, `prefer_depth`) |
| `[[probe]]` | question hints used when something is missing (`when_missing` = `mode`, `depth`, `number` or a flag id) |
| `[stop]` | answers per group before the window offers to move on |
| `guide = "x.md"` | a long note in your own words, put into every interview prompt |

The window uses `lens/mine.toml` when it exists, else the first lens here,
else the built-in `default`. Whether an item is done is decided by the app
from the lens, not by the model.

Your lens says what YOU find important in a paper. Keep it in your private
repository.
