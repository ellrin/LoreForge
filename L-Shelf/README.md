<p align="center">
  <img src="apps/desktop/icons/logo.svg" alt="L-Shelf" width="120">
</p>

<h1 align="center">L-Shelf</h1>

<p align="center">
  <b>A catalog of the files on your Mac. It reads; it moves only what you confirm.</b><br>
  <sub>Part of <a href="../README.md">LoreForge</a> · local-first · interface in Traditional Chinese</sub>
</p>

<p align="center"><img src="../docs/assets/readme/divider.svg" alt="" width="100%"></p>

## Features

- **Files by content**: sha256 identifies a file, so duplicates and single copies are found wherever they are.
- **Every date it can find**: created, modified, downloaded, last opened, dates inside the document, the download URL.
- **Projects as one unit**: a folder with `.git`, `main.tex`, `Cargo.toml`, `package.json` and similar is one entry.
- **Opens in the right app**: PDF → L-Reader, `.pptx` → L-Slide, `.docx` and `.tex` → L-Paper.
- **Filing on your say**: it plans where files belong; a move happens only in a batch you confirm, with undo.

## Run

```sh
cargo run --release -p loreshelf-cli -- scan ~/Downloads   # read-only scan and report
cargo run -p shelf-server                                  # http://127.0.0.1:7900
../scripts/install-apps.sh L-Shelf                         # the Mac app
```

## Your data

- Catalog: `~/Library/Application Support/LoreForge/LoreShelf` (or `LORESHELF_HOME`), local only: it refuses iCloud Drive, this repo, or any git repo with a remote.
- Hash cache: `~/Library/Caches/LoreForge/shelf/`, safe to delete.

## Limits

- Scans never write, rename or delete, do not follow symlinks and do not download iCloud placeholders.
- macOS may need a privacy permission for folders such as Downloads.

Command reference: [`CLAUDE.md`](CLAUDE.md) · design: [`../docs/loreshelf.md`](../docs/loreshelf.md)

## Development notes

Developed by ellrin with Claude (Anthropic); the code is written in claude.ai and Claude Code.
