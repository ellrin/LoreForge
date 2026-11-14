# style/ — your writing style, as notes for the model

Every `.md`, `.toml` or `.txt` file here (sorted by name; this README,
`checks.toml`, `patterns.toml` and files starting with `_` are skipped) goes, as rules to
follow, into every prompt that writes or rewrites English: 寫作, 潤飾, captions,
the new-paper outline, paraphrases, 寫書 chapters, and the planning, outline,
drafting and review prompts of「討論草稿」. They are sent every time, so keep
them short; put long reference material under a `_` name. Useful files:

- a profile: paragraph length, sentence length, how much of each section is
  facts vs reasoning;
- paragraph types: the units each kind of paragraph has, in order;
- habits to fix: things you do not want to repeat, with examples;
- exemplars: your own best paragraphs (and a few bad ones), to copy the moves,
  not the sentences.

Keep the total under ~24,000 characters; longer notes are cut. Raw material
(per-paragraph annotations of your papers) can live here under a `_` name.

`checks.toml` holds the word lists of the code checks (no model): named lists of
`"text"` or `{ find = "…", use = "…" }`. A hard row in `profile/rules.csv` turns a
check on and names its list in `threshold`. Checks: `phrases`, `before_number`,
`comma_splice`, `sentence_start`, `end_preposition`, `double_comparative`, and
`contractions`, `exclamation` (no list).

`patterns.toml` holds sentence structures from writing guides (`[[pattern]]` with
`id, situation, when, structure, example, source = "guide"`). They are not your
voice: only the entries whose situation fits the paragraph being drafted are sent
(Introduction's last or contribution paragraph → `thesis_statement`,
`thesis_with_reasons`, `purpose_to_thesis`; a comparison → `compare_two`; a
paragraph's first sentence → `topic_sentence`), as structure to reuse. They never
enter the phrase bank, 潤飾 or 我的風格.

`_thesis-statement.md` (optional): your notes on thesis statements; the Key value
check uses them as its criteria (bounded). Without it a built-in generic list is used.

Rows of `profile/*.csv` tagged `guide` (or with `source` = guide) are treated as
guide material too: never used as your voice or in your style profile.
