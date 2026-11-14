# Writing profile template

A writing profile is your personal phrase bank, vocabulary preferences and checking
rules. **Keep your real profile in a private repository.** This folder only holds
demo data so the app runs out of the box.

## Use it

1. Create a private repository and copy every file in this folder into it.
2. Replace the demo rows with your own.
3. Tell the app where it is, in either way:
   - environment variable `PAPERCOMPOSER_PROFILE=/path/to/your/profile`
   - or `profile_path = "/path/to/your/profile"` in the app config file
     (`~/Library/Application Support/PaperComposer/config.toml` on macOS,
     `%APPDATA%\PaperComposer\config.toml` on Windows,
     `~/.config/PaperComposer/config.toml` on Linux).

The config file is outside every repository, so your path is never committed.

## Files

| File | Purpose |
| --- | --- |
| `profile.toml` | Name, `schema_version`, default language |
| `phrases.csv` | Sentence patterns you like to use |
| `expressions.csv` | Opening connectives, collocations and short phrases |
| `vocabulary.csv` | Words to avoid and what to write instead |
| `rules.csv` | Paragraph checks |
| `exemplars.csv` | Whole paragraphs that show your voice |
| `equations.csv` | Formulas you insert often |
| `authors.csv` | People you write with, picked when you create a paper |
| `affiliations.csv` | Their institutions |

Edit them in any spreadsheet app; save as UTF-8 CSV.

### phrases.csv

| Column | Required | Meaning |
| --- | --- | --- |
| `id` | yes | Unique id, e.g. `P001` |
| `section` | yes | `Abstract`, `Introduction`, `Related work`, `Method`, `Result`, `Discussion`, `Conclusion`, or `Any` |
| `function` | yes | What the sentence does: `research_gap`, `finding`, `limitation`, … (free text, keep it consistent) |
| `position` | yes | `opening`, `body`, `closing` or `transition` |
| `template` | yes | The pattern; `{slot}` marks what to fill in |
| `example` | no | A real sentence using the pattern |
| `tags` | no | Free tags, separated by `;` |
| `priority` | no | 1–5, higher is suggested first (default 3) |

### expressions.csv

| Column | Meaning |
| --- | --- |
| `id` | Unique id, e.g. `X001` |
| `kind` | `connective` (opens a sentence), `collocation` or `phrase` |
| `text` | The expression itself, e.g. `Taken together,` |
| `usage` | One line on when to use it |
| `section`, `example`, `tags` | Optional, as in phrases.csv |

The database editor's decompose tool fills this file and phrases.csv from a
sentence or paragraph you paste in.

### authors.csv and affiliations.csv

| Column | Required | Meaning |
| --- | --- | --- |
| `id` | yes | `A001` (authors), `F001` (affiliations) |
| `family` | yes | Family name |
| `given` | no | Given name |
| `email`, `orcid` | no | Printed where the journal's class has a place for them |
| `affiliations` | yes | Affiliation ids in print order, separated by `;` |
| `role`, `note` | no | Free text: me, advisor, clinician, ... |
| `organization` | yes | (affiliations.csv) The institution |
| `department`, `city`, `country` | no | (affiliations.csv) |

Each paper keeps its key value and the authors picked for it in
`papers/<paper>/paper.toml`.

### vocabulary.csv

`avoid` (required), `prefer` (optional replacement), `note`.

### rules.csv

| Column | Meaning |
| --- | --- |
| `id` | Unique id |
| `type` | `hard` = checked instantly by code; `soft` = needs a model |
| `scope` | `paragraph`, `sentence`, or `section:<Name>` |
| `check` | Hard checks: `sentence_count` (threshold `3-8`), `max_words` (threshold `35`), `banned_words` (uses vocabulary.csv), and the style checks (`phrases`, `before_number`, `comma_splice`, `sentence_start`, `end_preposition`, `double_comparative`: threshold = a list in `style/checks.toml`; `contractions`, `exclamation`: no threshold; see `style/README.md`). Soft checks: any name; the model reads `message` |
| `threshold` | Depends on `check` |
| `severity` | `info`, `warning` or `error` |
| `message` | Short text shown when the rule fires |

## Schema version

`schema_version` lets the app upgrade your files when columns change. The app commits
a backup before any upgrade.
