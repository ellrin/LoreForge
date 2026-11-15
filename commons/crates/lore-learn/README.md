# lore-learn

What the author knows and has read, read-only (docs/paper-deck.md 「不熟」標示, docs/book.md):

- `terms` (re-exported at the root): a paper's domain terms (acronyms, expansions, repeated phrases,
  L-Brain titles and aliases; common words filtered by `lore-words`) with the member's state per term:
  `Known` (記得) · `Learning` (學習中, or no state) · `Unknown` (no card, or 要學). `find_terms(items, kb, filters)`
  is pure; `terms_for(home, items)` reads the L-Brain from disk. `space_dirs(home)`, `member(home)`,
  `load_names(spaces, member)`, `prerequisites(spaces)` (`(card, needs)` from the cards' `prerequisite` links),
  `TermsResult` (`understood`, `matcher`), `Matcher` (term spellings in text, by word keys).
  L-Slide's 論文報告 (`kbterms.rs` re-exports it) and L-Paper's 寫書 use it.
- `reader`: L-Reader's files without L-Reader running: `Reader::discover(home)` (`LOREREADER_WORKSPACE`, else
  `private/LoreReader`; the library from `LOREREADER_LIBRARY` → L-Reader's `config.toml` → `~/Documents/LoreReader/library`),
  `pdf(sha)`, `paper_json`, `symbols_json`, `understood(sha)` (懂了 paragraphs, `reading.json`),
  `questions(sha)` (問這篇, `chat.json`), `list()`.

Nothing here writes. `cargo test -p lore-learn`: offline, temp folders.
