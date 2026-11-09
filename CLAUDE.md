# LoreForge — notes for Claude Code

Monorepo of the author's local-first apps. Each top-level folder is its own Cargo workspace
(or Swift app for NeonDeck) with its own `CLAUDE.md`; read that before working in it.

- `commons/` shared crates and web components; apps depend on `../commons/crates/<name>`.
- `PaperComposer/`, `KnowledgeBase/`, `SlideComposer/`, `NeonDeck/` apps.
- `private-example/` (public): the minimal content each app needs to open, same layout as
  `private/`. Apps seed `private/<App>` from it on first run and never write into it.
- `private/` is **ignored**: each subfolder is a separate private git repo —
  `private/shared/` (profile, fonts, deny words, `docs/ecosystem-design.md` = the overall
  design; may be missing on other machines) and `private/<App>/` (one app's data).
- Find these with `ym-home` (`Home::discover`, `seed_app`, `shared_file`, `prefs`); never
  hard-code `../private`. Personal choices (fonts, field-specific lists, who the user is)
  go in `private/shared` or the app's private folder, with a neutral default in public.

## Rules

- This repo is public. Never commit anything from `private/`, personal data, real user paths,
  e-mail addresses, keys, real papers/decks, employer/course names or licensed font names.
  The hooks (`sh scripts/install-hooks.sh`) block `private/`, words in
  `private/shared/deny-words.txt`, files > 5 MB and (with gitleaks) secrets — a safety net,
  not the rule.
- Keys, passwords and tokens live outside LoreForge (OS keychain; see README "金鑰與模型放在哪裡");
  local models in `LOCAL_LLM_HOME`. Code refuses to write key files inside a git work tree.
- Never write to the real `private/` repos from tests; tests use temp dirs and synthetic data.
- Commits: ellrin is the only author; no `Co-Authored-By` or AI attribution trailers.
  Claude is mentioned only in README "開發說明" sections.
- `cargo test` in every workspace you touched must pass before a commit.
- Example/test users: `ellrin`, then `daidai`.
- Replies to the author in Traditional Chinese (繁體中文).
