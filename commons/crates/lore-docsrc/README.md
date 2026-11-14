# lore-docsrc

Paper material extraction (design §7.4, §6 learning flow, §13-2). Two sources, one output:
an **asset catalog** of sections (plain-text paragraphs), figures, sub-figures, tables,
equations, captions, references and metadata. SlideComposer uses the page regions and PNG
crops; PaperComposer's book/learning pipeline and the term/notation scanners use the text.

```toml
lore-docsrc = { path = "../commons/crates/lore-docsrc" }
```

## API

| Module | What |
|---|---|
| `model` | `AssetCatalog { schema, source, meta, sections, assets, references, pages }`, `Asset { id, kind, number, label, caption, caption_latex, text, source, grid, page, bbox, caption_bbox, masks, files, parent, section, source_ref, crop, confidence, notes }`, `AssetKind::{Figure, Subfigure, Table, Equation, Region}`, `BBox` (PDF points, top-left origin, y down), `Section`, `Reference`, `Meta`, `PageInfo`, `SourceRef { file, line, page }`. `catalog.by_number(kind, "IV")` (roman = arabic), `count_numbered`, `free_id`, `save`/`load`, **`attach_pdf(&pdf_catalog)`** (LaTeX assets get the page/bbox/crop of the same-numbered PDF asset) |
| `latex` | `extract_dir(root)` / `extract(&LatexProject)`; `LatexProject::open` (main file = has `\documentclass` + `\begin{document}`; conventional name, else largest), follows `\input`/`\include`/`\subfile`, strips comments, `locate(offset) → file:line`. Figures (`figure`, `figure*`, …): files resolved with `\graphicspath` and extensions, caption, label, sub-figures (`subfigure`/`subtable` env, `\subfloat[..]`, `\subcaptionbox`) in source order. Tables: whole env in `source`, `grid` (rows × cells, `\multicolumn` padded, rules dropped) and tab-separated `text`. Equations: one asset per **number** (`align`/`gather`/`eqnarray` lines honour `\nonumber`/`\notag`/`\tag`), `text` = stand-alone LaTeX, `source` = the whole env; unnumbered displays (`equation*`, `\[..\]`, `$$..$$`) as `eq-u<n>`. Sections numbered like LaTeX (appendix → `A`, `A.1`); `\ref`/`\eqref` resolved to numbers in the plain text. References: `.bbl`/`thebibliography` order and text, `.bib` fields, `cited` flag. `latex::to_plain(fragment, &PlainOpts)` is public |
| `pdf` | `extract(pdf, &PdfOptions)` → catalog + `out_dir/{catalog.json, pages/, crops/}`; `analyze(pages, inks, info)` is the pure pipeline. `pdf::poppler::Poppler` (`discover()`: `$LORE_POPPLER_DIR` → `/opt/homebrew/bin` → `/usr/local/bin` → `/usr/bin` → `PATH`; `with_dir`; `require()` fails with `Error::ToolMissing` and an install hint). `pdf::layout` (bbox-layout parser, `Columns`, `Ink`), `pdf::detect` (`Doc`, captions, regions, headings, equations, references). A caption cut mid-sentence goes on into the next block at the same left edge (Word's math words split it); Word's math words become `$…$` (`𝐶` + lowered `𝑒𝑛𝑐` → `$C_{\mathrm{enc}}$`, a combining accent word on its letter): `detect::line_text_math`, `join_lines_math` |
| `crop` | `Renderer { poppler, pdf, cache_dir }`: `page_png(page, dpi, gray)` (cached `pages/page-003@200.png`), `crop_png`, `render_asset(&asset, dpi)`; `crop_image`; `crop_png_fraction(png, [x0,y0,x1,y1] page fractions, pad_px)`. Region ops that add `Region` assets: `recrop(cat, id, rel_bbox)`, `split(cat, id, Axis, cuts)`, `split_grid(cat, id, rows, cols)`, `mask(cat, id, &[rel_bbox])` (white-out, inherited by later children), `add_module_regions(cat, id, boxes, img_w, img_h)` |
| `boxer` | `trait ModuleBoxer { module_boxes(&FigureImage, caption) -> Vec<ModuleBox { label, role, bbox: [x0,y0,x1,y1] px }> }`; `LlmModuleBoxer::new(provider)` / `::from_route(route)` for lore-llm task **`sc-crop-modules`** (`TASK_ID`): vision request + JSON schema, `complete_json` retry, images > 1568 px downscaled and boxes scaled back, fractions accepted, boxes clamped/validated; `MockBoxer`; `box_modules(cat, id, &renderer, &boxer, dpi)` = render → ask → region assets |
| `text` | `catalog.section_texts(TextOptions { captions, math })` → `SectionText { id, level, number, title, text }`; `section_text(id)`, `plain_text()`, `captions()`. Inline math stays `$…$` unless `math: false`; citations are dropped. Feed these strings to kb-core's scanner and lore-notation's extractor (no dependency either way) |
| `arxiv` | `ArxivId::parse` (ids, `arXiv:` prefix, abs/pdf URLs, old-style ids), `trait ArxivFetcher { eprint, metadata }`, `MockArxiv`, `HttpArxiv::new(contact, cache_dir)` (export.arxiv.org, one request at a time ≥ 3 s apart, User-Agent with the app-supplied contact, on-disk e-print cache), `unpack` (tar.gz / gzipped single .tex / tar / PDF; refuses `..` and absolute paths, skips links), `fetch_and_extract` (fills title/authors/abstract from the API when the source lacks them), `parse_atom` |

```rust
// PDF only
let cat = lore_docsrc::pdf::extract(&pdf, &PdfOptions::new(&out_dir))?;   // crops at 200 dpi
// LaTeX source + its compiled PDF: exact LaTeX and page crops together
let mut tex = lore_docsrc::latex::extract_dir(&src)?;
tex.attach_pdf(&cat);
// semantic sub-module boxes of an architecture figure
let route = router.provider_for(lore_docsrc::boxer::TASK_ID).await?;
let boxer = LlmModuleBoxer::from_route(route);
let ids = box_modules(&mut tex, "fig1", &Renderer::new(Poppler::discover(), &pdf, &out_dir), &boxer, 200).await?;
// re-crop / split / white-mask for slides
let left = crop::recrop(&mut tex, "fig1", BBox::new(0.0, 0.0, 200.0, 150.0))?;
crop::mask(&mut tex, &left, &[BBox::new(10.0, 10.0, 60.0, 30.0)])?;
```

Equations: by design §13-2 the default slide output is the **crop** (`crop`, page `bbox`); the
LaTeX `text` (LaTeX mode) is there for lore-math → OMML when a slide wants native math.

## How PDF mode works

1. `pdftotext -bbox-layout` → pages / blocks / lines / words; `pdfinfo`; gray page renders at
   `analysis_dpi` (100) → **ink** map (summed-area table).
2. Columns from line extents (one or two); lines that pdftotext joined across the gutter, and
   blocks holding both columns, are split; reading order = full-width blocks cut the page into
   bands, left column before right.
3. Line roles: furniture (header/footer band), **prose** (≥ 4 words, mostly words, no wide
   gaps, ≥ 55 % of the column; whole prose blocks), captions (`Fig. 3.`, `Figure 3:`,
   `Table 2`, `TABLE IV`, small caps rejoined), headings (`1.`, `2.3.`, `IV.`, `A.` in roman
   papers, number and title split into two blocks, wrapped headings, named headings incl.
   letter-spaced `a b s t r a c t`).
4. Figure/table region: the free vertical interval next to the caption (figures above, tables
   below, then the other side), bounded by prose/headings/captions/other regions; a table and a
   figure facing each other share it and are cut at the widest blank gap; widened to full width
   when the other column has ink but no prose, caption or region beside it; trimmed to ink;
   stray page rules dropped.
5. Equations: `(n)` at a column's right edge, alone or after a wide gap; the slot between
   stoppers is split from neighbouring tags at the widest blank gap; ink bands near the tag are
   kept, running text glued to them ("where …", "as:") is cut off or white-masked.
6. Sections/paragraphs from prose in reading order (indent / short-line paragraph breaks,
   de-hyphenation, continuation across columns and equations), references after the
   References heading (`[n]`, `n.` or hanging indent), title (pdfinfo or the wordiest tallest
   group on page 1), authors, abstract (`Abstract—` or heading), keywords.

`LORE_DOCSRC_DEBUG=1` prints the equation slots and bands; `examples/debug_lines.rs` prints line
roles, headings and reading order.

## Evaluation (2026-11-09)

Run on five real two-column papers (IEEE and Elsevier styles, one with its LaTeX source);
the papers are not in the repository and the per-paper notes are kept privately.
Every numbered figure, table and equation was found and cropped (30/30 figures,
29/29 tables, 64/64 equations); sections and references matched the true counts; in LaTeX
mode `attach_pdf` placed every numbered asset, captions LaTeX vs PDF token Jaccard
0.92–1.00 (figures) and 0.75–0.93 (tables: small caps/emphasis). ~5 s per 10-page paper (release).

```sh
cargo run -p lore-docsrc --example docsrc -- pdf paper.pdf out/            # summary + crops
cargo run -p lore-docsrc --example docsrc -- both src/ main.pdf out/       # LaTeX + PDF
LORE_DOCSRC_PAPERS='a.pdf=4,5,17|b.pdf=8,4,9' LORE_DOCSRC_OUT=out \
  cargo test -p lore-docsrc --test real_papers -- --ignored --nocapture
LORE_DOCSRC_LATEX_SRC=src LORE_DOCSRC_LATEX_PDF=main.pdf \
  cargo test -p lore-docsrc --test real_papers latex_vs_pdf -- --ignored
```

Tests are offline: unit tests on synthetic pages/ink, and `tests/pdf_fixture.rs` on
`tests/fixtures/twocol.pdf` (built from `tests/fixtures/twocol/main.tex`, invented content;
skipped when poppler is missing).

## Known gaps / next steps

- Unnumbered display equations are not detected in PDF mode (LaTeX mode has them).
- Sub-figure panels `(a)(b)…` are not split in PDF mode (use `split`/`split_grid`, or the
  module boxer); LaTeX sub-figures carry files and captions but no page region of their own.
- Figures without a caption, side-by-side floats sharing one column band, captions placed
  beside a figure, and rotated (landscape) tables are not handled.
- Equation crops may include a neighbouring unnumbered display that touches the numbered one.
- Section text: inline math comes out as pdftotext glyphs (`𝐳 𝑖𝑚𝑔`); front-matter boxes (article
  info, affiliations) are best effort; author lists rely on the line under the title or pdfinfo.
- `lopdf`-based image XObject placement is not used (ink analysis covers raster and vector).

## Not tested on real hardware

- `HttpArxiv` against the real arXiv (tests use `MockArxiv`).
- `LlmModuleBoxer` with a real vision model (tests use `MockProvider` / `MockBoxer`).

**Unnumbered headings (opt-in).** `PdfOptions::unnumbered_headings` (default off): in a document without numbered headings, a short
Title-Case line set larger than the body is a heading (≥ 1.3× → section, ≥ 1.08× → subsection), after the abstract. Springer journals
set their headings this way; PaperComposer's 期刊寫法 turns it on. `pdf::analyze_with(…, unnumbered)`; real-paper check:
`LORE_DOCSRC_UNNUMBERED='a.pdf|b.pdf' cargo test -p lore-docsrc --test real_papers unnumbered -- --ignored`.
