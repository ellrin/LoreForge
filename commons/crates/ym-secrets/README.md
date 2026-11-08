# ym-secrets

API keys and passwords in the OS keychain (macOS Keychain / Windows Credential Manager /
Linux keyutils), with an owner-only file fallback. Keyring only — no HTTP client, no tokio —
so an app that just stores a secret (KnowledgeBase's SMTP password) stays light.
`ym_llm::secrets` re-exports this crate unchanged.

```toml
ym-secrets = { path = "../../commons/crates/ym-secrets" }
```

| Item | What |
|---|---|
| `KeyStore::anthropic(service, app_dir)` | Anthropic key store: env `ANTHROPIC_API_KEY`, keychain account `anthropic-api-key`, file key `anthropic_api_key`. Other secrets: same struct with your own `env_var` / `account` / `file_key` |
| `KeyStore::get(explicit, &Account) -> Option<(String, KeySource)>` | explicit → env → keychain (read once per process) → file → owner's legacy single key |
| `KeyStore::save(k, &Account)` | keychain first (stale macOS item repaired), file only when the keychain is unusable; empty = remove |
| `Account::{Single, User(Who), Nobody}`, `Who { workspace, user, legacy }` | one key per app, or per workspace user (never another user's key) |
| `KeySource::{Settings, Env, Keychain, File, Legacy}`, `mask(k)`, `KEYS_PAGE` | |

Tests set `use_keychain = false` and `env_var = None` (file fallback only).
**Not tested on real hardware:** the real OS keychain.
