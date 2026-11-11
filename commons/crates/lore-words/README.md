# lore-words

The word-frequency filter for domain terms: which words of a paper are common English and
never count as terms.

- `Filters::new(ignore)` / `Filters::default()`: `is_function`, `is_general` (function words,
  general academic words, everyday English and SCOWL ≤ 20, never a `TERMLIKE` word),
  `is_familiar` (SCOWL 35), `is_ignored` (the caller's ignore list, words and phrases).
- `word_key`, `phrase_key`: grouping keys (case, plurals, British/American spelling,
  ligatures, possessives, `SBLs` → `sbl`; hyphens count as spaces). `is_acronym`, `is_hyphen`.

The lists and rules are LoreReader's (`lr-core::nouns`), copied here so other apps
(SlideComposer's 「不熟」 marks) use the same filter; LoreReader can switch to this crate.
SCOWL licence: `src/english_words.LICENSE`.
