# Publishers

One folder per journal (or a generic one per publisher), e.g. `ieee-tmi/`:

```
ieee-tmi/
  ieeecolor.cls, tmi.sty, ...   the class and style files
  sample.tex          the journal's example document
  template.toml       name, short name, publisher, author instructions, notes
  *.csl               optional: citation style for Word export
  reference.docx      optional: Word styles for Word export
```

New papers get a copy of every file here except `sample.tex`. Word export
(Pandoc) uses the paper's `.csl` for citations and `reference.docx` for
fonts and headings. The `.csl` files come from the Citation Style Language
project (github.com/citation-style-language/styles, CC BY-SA 3.0).

In the app, Templates lists every journal (searchable). "New journal" copies
an existing template under a new name; then tune its class options,
bibliography style, notes and sample.tex there. `template.toml` is written
by the app; a folder without one shows its README heading or folder name.

## House rules

What the journal asks for, all optional, in `template.toml`. The template
editor's 投稿規則 section edits them; a journal made from another starts with
its rules. A template without them behaves as before.

```toml
[word_limits]          # shown beside the status bar's word count
abstract = 250
total = 6000           # the parts ticked in the word count

[word_limits.sections]
Introduction = 1000    # by section title

[ref_words]            # the word before \ref in running text
fig = "Fig."           # or "Figure"
tab = "Table"
eq = "Eq."             # "" writes only \eqref{…} → "(1)"
sec = "Section"
sentence_start = { fig = "Figure" }   # per kind, at the start of a sentence; a kind left out uses its word above
mode = "words"         # or "cleveref": \cref{…} (\Cref{…} at a sentence start), the class prints the word

[captions]
table = "above"        # or "below"
figure = "below"

[limits]
pages = 8
pages_exclude_refs = true
anonymous = true       # double-blind
figures = 8
tables = 5
references = 50
keywords = 6           # at most
keywords_min = 3

[requires]
highlights = true
graphical_abstract = true

[sources]              # where a value came from, by its key
"word_limits.abstract" = "The abstract should not exceed 250 words."
"captions.table" = "Elsevier 慣例（elsarticle）"
```

`reference_style = "numbered, Vancouver"` (a top-level key, before the
tables) records the journal's reference style in its own words.

"New journal" also builds a template from the journal's own files: drop a
LaTeX template (.zip or folder; its main file becomes `sample.tex`, and only
the .tex / .pdf files the sample uses are kept) or a Word template (.docx,
converted by the Word importer, kept as `reference.docx`), and give the
author guidelines URL. The class's house style (caption places; Fig./Figure
for IEEE, REVTeX, MDPI) is filled in; the guidelines page, and the sample's
own text, are read by the configured engine, and every value is shown with
the sentence it came from. A value whose sentence is not on the page is left
out. Nothing is written until 建立; `[sources]` keeps the sentences.

- `[ref_words]`: references inserted in the editor are written this way, the
  preview prints these words for `\cref` / `\autoref`, drafting and revising
  follow them, and the notation check reports a reference written differently.
  Only `sentence_start` decides what a sentence starts with. Kinds left out
  keep the defaults: `Fig.~\ref`, `Table~\ref`, `\eqref`, `Section~\ref`.
- With no `[ref_words]` and a `sample.tex` that loads cleveref
  (`\usepackage{cleveref}`), the template counts as `mode = "cleveref"`
  (nothing is written to the file).
- A value of the wrong type (a word for `pages`, …) makes the whole file
  unreadable: the template then shows only its folder name.

## The journal itself (期刊寫法)

Also in `template.toml`, written by the app:

```toml
role = "journal"       # or "base" (a publisher's format journals are built on) or
                       # "umbrella" (a series): both are left out of the template list

[venue]                # which journal it is, confirmed once (OpenAlex source)
openalex = "S1234567"
name = "Journal of Synthetic Examples"
issn_l = "1234-5678"
confirmed = "2026-11-14"

[submission]           # what the journal requires at submission (see docs/long-paper.md §D)
abstract_words = 250
required_statements = ["Data availability"]
checked = "2026-11-14"
```

A template other templates are `based_on`, with no `[venue]`, counts as a base format.
The journal's writing profile is kept beside, in `venues/<template id>/profile.toml`
(and a readable `profile.md`): no article text, only titles, counts, shares and short phrases.
