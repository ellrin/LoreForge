<p align="center">
  <img src="apps/desktop/icons/logo.svg" alt="L-Reader" width="120">
</p>

<h1 align="center">L-Reader</h1>

<p align="center">
  <b>A PDF reader for papers that shows you what you do not know yet.</b><br>
  <sub>Part of <a href="../README.md">LoreForge</a> · local-first · interface in Traditional Chinese</sub>
</p>

<p align="center"><img src="../docs/assets/readme/divider.svg" alt="" width="100%"></p>

## Features

- **Knowledge marks**: red = a term with no card, amber = learning, green underline = remembered (from L-Brain).
- **Terms, symbols and equations**: one model call sorts a paper's terms; the symbol and equation table is built without a model.
- **Read with a pen**: highlights, notes, free text boxes, and quotes you can cite later in L-Paper.
- **Ask in place**: translate a selection, or ask Claude with the paragraph as context.
- **Pages without touching the original**: delete, duplicate, reorder, insert, merge; each result is a new PDF.
- **Hand-offs**: send the paper to L-Slide as a deck, or to L-Paper to write a review.

## Run

```sh
cargo run -p lr-server                     # http://127.0.0.1:7890
../scripts/install-apps.sh L-Reader        # the Mac app (also a PDF viewer for Finder)
```

## How to use

- **Open a PDF**: button, drag in, paste a path, or open it from Finder. It is copied into the library; the original is untouched.
- **Select text, right-click**: copy, highlight (1–4), note, translate, ask Claude, save as quote, add to knowledge base.
- **Right panel**: terms (名詞), notes, symbols, paragraphs, quotes, chat.
- **Save with notes** (另存新檔): writes a new PDF with your annotations; on quit it asks once per paper.
- **Views**: scroll or two-page spread (←/→, space).

## Your data

- PDFs: `~/Documents/LoreReader/library` (named by sha256, never inside a git repo).
- Notes, quotes and chats: `private/LoreReader/papers/<sha256>/`.

## Limits

- Prototype; it does not edit PDF content.
- Knowledge marks and adding cards need L-Brain running.

Detailed guide, data formats and API (繁體中文): [`docs/guide.zh-TW.md`](docs/guide.zh-TW.md)

## Development notes

Developed by ellrin with Claude (Anthropic); the code is written in claude.ai and Claude Code.
