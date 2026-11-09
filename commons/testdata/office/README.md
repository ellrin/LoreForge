# Office fixtures

Synthetic files (a fictional study) for testing how PaperComposer and SlideComposer open, convert
and edit Office documents. Rebuild with `python3 make.py` (needs pandoc and Pillow).

| File | What it exercises |
|---|---|
| `fixture-paper.docx` | 17 native equations (OMML): inline variables, fractions, sums, a matrix, aligned rows; an equation that is only a picture (`fig-equation-picture.png`, as MathType leaves it); two figures with captions; a table with a caption; a reviewer comment (Reviewer 2); a tracked insertion and deletion |
| `fixture-slides.pptx` | 5 slides: a display equation and inline variables (`a14:m` OMML, no fallback picture), two pictures, speaker notes on 3 slides |

What a correct tool does with them:

- Every equation survives (open, edit, save, export). The picture-only equation is reported as
  "needs retyping", never silently treated as text.
- Equations converted to LaTeX use the author's registered notation where it matches
  (`lore-notation`): `\tilde{q}_b` and `\widetilde{q}_{b}` are the same symbol.
- A preview that cannot draw equations says so. LibreOffice without its Math component drops
  every equation from a .docx without a warning, and LibreOffice Impress draws no `a14:m`
  equation that lacks a fallback picture.
- The comment and the tracked changes are kept (or turned into revision items), never lost.
