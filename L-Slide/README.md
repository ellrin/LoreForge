<p align="center">
  <img src="apps/desktop/icons/logo.svg" alt="L-Slide" width="120">
</p>

<h1 align="center">L-Slide</h1>

<p align="center">
  <b>A slide editor that keeps your PowerPoint files intact and your decks consistent.</b><br>
  <sub>Part of <a href="../README.md">LoreForge</a> · local-first · interface in Traditional Chinese</sub>
</p>

<p align="center"><img src="../docs/assets/readme/divider.svg" alt="" width="100%"></p>

## Features

- **Lossless PPTX**: open a `.pptx` where it lives and edits are written back; untouched slides stay byte-identical.
- **Styles as rules**: text takes a style tag and colours come from the palette, so a deck cannot drift.
- **Paper → deck**: from an L-Paper or L-Reader paper, an outline first, then figures, native tables and equations, and references.
- **Shared notation**: symbols and equations come from the same registry as L-Paper.
- **Progressive reveal**: pick steps on one slide and get the build-up pages.
- **Presenting**: presenter view, slide picker, laser, and live equation figures.

## Run

```sh
cargo run -p sc-server                     # http://127.0.0.1:7880
../scripts/install-apps.sh L-Slide         # the Mac app
```

## How to use

- **⌘O** open a `.pptx`; it saves itself about a second after each edit. **⇧⌘S** save as.
- The original is backed up once as `.<name>.original.pptx` before the first overwrite.
- **Double-click** text to edit; choose a tag instead of a font and size.
- **⌘Enter** show from this slide, **⇧⌘Enter** from the start; **G** picks a slide, **L** laser, **B** black screen.
- **Export**: PPTX or PDF.

## Your data

- Style packs, layouts and deck caches: `private/SlideComposer/` (`styles/`, `arrangements/`, `decks/`).

## Limits

- Fonts you do not have are substituted, so some lines wrap differently than in PowerPoint.
- Picture effects, shadows and pattern fills are not drawn; charts and SmartArt are kept but cannot be moved.
- PDF export needs Chrome; equations need pandoc.

Detailed guide (繁體中文): [`docs/guide.zh-TW.md`](docs/guide.zh-TW.md) · known gaps: [`CLAUDE.md`](CLAUDE.md)

## Development notes

Developed by ellrin with Claude (Anthropic); the code is written in claude.ai and Claude Code.
