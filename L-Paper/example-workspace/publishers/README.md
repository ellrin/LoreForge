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
