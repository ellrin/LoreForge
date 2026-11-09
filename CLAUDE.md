# LoreForge — notes for Claude Code

Monorepo of the author's local-first apps. Each top-level folder is its own Cargo workspace
(or Swift app for NeonDeck) with its own `CLAUDE.md`; read that before working in it.

- `commons/` shared crates and web components; apps depend on `../commons/crates/<name>`.
- `PaperComposer/`, `KnowledgeBase/`, `SlideComposer/`, `NeonDeck/` apps.
- `private/` is **ignored**: each subfolder is a separate private git repo with the author's
  personal or household data (`private/<App>/`), plus `private/docs/ecosystem-design.md`
  (the overall design; may be missing on other machines) and `private/deny-words.txt`.

## Rules

- This repo is public. Never commit anything from `private/`, personal data, real user paths,
  e-mail addresses, keys, real papers/decks, or employer/course names. The pre-commit guard
  (`sh scripts/install-hooks.sh`) blocks `private/`, words in `private/deny-words.txt`,
  files > 5 MB and (with gitleaks) secrets — but it is a safety net, not the rule.
- Never write to the real `private/` repos from tests; tests use temp dirs and synthetic data.
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
  Claude is mentioned only in README "開發說明" sections.
- `cargo test` in every workspace you touched must pass before a commit.
- Example/test users: `ellrin`, then `daidai`.
- Replies to the author in Traditional Chinese (繁體中文).
