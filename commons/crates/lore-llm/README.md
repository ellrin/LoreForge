# lore-llm

The single LLM layer for every app (design §2.2, §9). Upper layers never name an engine
or a model: they ask the router for a **task**'s provider, and the provider answers a
`LlmRequest`.

```toml
lore-llm = { path = "../commons/crates/lore-llm" }   # default feature: zhtw (OpenCC)
```

## API overview

| Module | What |
|---|---|
| `types` | `LlmRequest` (system, `messages: Vec<Message>`, `max_tokens`, `json_schema`, `web_search`, `effort`, `temperature`, `top_p`; builders `.json()`, `.web()`, `.effort()`, `.temperature()`, `.top_p()`, `.image()`), `Message` / `Part::{Text, Image}` / `Image`, `LlmResponse` (text, provider id, web `sources`, `usage`), `StreamEvent::{Text, Searching, Source}`, `Capabilities`, `Needs` |
| `provider` | `trait LlmProvider { id, capabilities, send, stream(req, on), complete }`; `OnEvent = &(dyn Fn(StreamEvent) + Send + Sync)` |
| `claude_api` | `ClaudeApi` + `ClaudeApiConfig` — Messages API, `anthropic-version: 2026-10-21`, no sampling params, `output_config.{format,effort}`, `fallbacks: "default"` (beta `server-side-fallback-2026-07-01`, not for Haiku), `stop_reason` checks (`refusal`, `max_tokens`, `pause_turn` resumed), SSE streaming, web search (`web_search_20260209`), base64 image blocks |
| `claude_code` | `ClaudeCode` — the user's subscription via `claude -p`: empty temp folder, env allowlist (`ALLOW_ENV`; billing vars `STRIP_ENV` never pass), `--safe-mode` (or older equivalents read from `--help`), `--tools ""`, `--json-schema`, prompt on stdin, per-user `CLAUDE_CONFIG_DIR`; optional WebSearch (`allow_web`). Also `Cli`, `binary`, `status`, `Cache`, `start_login`, `parse_output`, `fake` (test CLI) |
| `openai` | `OpenAiCompat` (= `LlamaServer`) + `OpenAiCompatConfig::llama_server(url, model)` — `/v1/chat/completions`, `response_format: json_schema`, `temperature` / `top_p` when the request sets them, `image_url` data URLs, SSE streaming, `ping()`, optional `zh_tw` post-processing |
| `mock` | `MockProvider` — queued replies, records `seen` requests, configurable capabilities |
| `json` | `extract_json`, `validate` (JSON Schema subset: `type`, `properties`, `required`, `items`, `enum`, `const`, `anyOf`, `oneOf`, `allOf`, `additionalProperties: false`; other keywords accepted as is), `complete_json` / `complete_as<T>` (validate + one retry) |
| `catalog` | `Catalog` (one schema for all apps; bundled `catalog.json`), `Model`, `CloudModel`, aliases (`sonnet` → id), `resolve_local` (exact id or prefix), `plan`, `gpu_usable`, `recommend(&Machine)` |
| `local` | `LOCAL_LLM_HOME` layout (`<home>/gguf/<owner>/<repo>/<file>`, `<home>/runtimes/llama.cpp/…/llama-server`): `home`, `find_model`, `find_mmproj`, `find_runtime`, `installed`, `missing`, `server_args`, `start`; `Layout` (`shared(home)` or `new(gguf_dir, runtime_dir)` for an app's own folders; same `find_*`/`installed`/`missing` as methods; `start_in`); `LocalRuntime` (one server at a time: `ensure(home, m)` / `ensure_in(&layout, m)`, `current_info`, `stop`, `kill_now`, `phase`) |
| `router` | `TaskTable` (tasks.toml + `[override]`), `ModelRef` (`claude:`, `claude-api:`, `claude-code:`, `local:`, `mock:`), `Router::provider_for(task) -> Route`, `Router::provider_for_with(task, &RouteOptions { override_ref, strict })` (`RouteOptions::only(ref)` / `::prefer(ref)`), `candidates_with`, `RouteError::{UnknownTask, NeedsNetwork, Unavailable}` + `.reason()`, `ProviderFactory` (also for `Arc<F>`, `&F`, `Arc<dyn ProviderFactory>`), `StandardFactory` (`selected_claude`, `claude_model`, `local_layout`, `current_local`), `LocalAccess` (`Default`, `layout`, `.current(&catalog)`), `LOCAL_CURRENT`, `CLAUDE_SELECTED`, `MockFactory`, `probe_online` |
| `secrets` | re-export of the `lore-secrets` crate (keyring only; apps that only store keys depend on it directly): `KeyStore::anthropic(service, app_dir)` with `get(explicit, &Account)` / `save`; `Account::{Single, User(Who), Nobody}`; `KeySource`; `mask`; `KEYS_PAGE` |
| `zhtw` | `to_taiwan` (OpenCC s2twp with keep-list) |

### Capabilities per engine

| Engine | json | vision | web | streaming | network |
|---|---|---|---|---|---|
| `ClaudeApi` | yes | yes | yes (server tool) | yes (SSE) | yes |
| `ClaudeCode` | yes (`--json-schema`) | no | only with `allow_web` | no (one piece) | yes |
| `OpenAiCompat` (llama-server) | yes (`json_schema`) | when started with mmproj | no | yes | no (local) |
| `MockProvider` | configurable | | | | |

### Task routing (`tasks.toml`)

```toml
[task.sc-crop-modules]
label = "從架構圖標出子模組框"
needs = { vision = true, json = true }      # also: web, context = 32000, streaming
default = "claude:sonnet"                   # alias from catalog.json cloud.aliases
fallback = "local:gemma-4-12b-q8"           # catalog id (or prefix)
offline = true                              # false = disabled offline; unset = true when a local candidate exists
effort = "medium"                           # optional
max_tokens = 8000                           # optional

[override]                                  # the user's per-task model picker
kb-summarize-card = "claude:haiku"
```

The bundled sample (`tasks.toml`) has the §9 tasks plus `pc-compose-paragraph`,
`kb-extract-terms`, `reader-chat`, `jl-generate`, `jl-realtime`.

```rust
let tasks = TaskTable::load(TaskTable::bundled(), Some(&user_tasks_toml))?;
let mut f = StandardFactory::new(Catalog::bundled());
f.prefer_subscription = true;                                 // JL/PC "auto"
f.api = key.map(|k| ApiAccess { base_url: DEFAULT_BASE_URL.into(), api_key: k, fallbacks: true });
f.subscription = Some(SubscriptionAccess { cli, cache, allow_web: false, temp_prefix: "jelearner-claude".into() });
f.local = Some(LocalAccess { runtime: Some(runtime.clone()), ..Default::default() }); // home: LOCAL_LLM_HOME
f.selected_claude = settings.claude_model.clone();              // for claude:selected
let router = Router::new(tasks, f);                             // or Router::new(tasks, Arc<StandardFactory>)
router.set_online(probe_online("api.anthropic.com:443", Duration::from_secs(2)).await);

let route = router.provider_for("kb-summarize-card").await?;   // RouteError explains offline/unavailable
let req = route.apply(LlmRequest::prompt(system, text).json(schema));
let card: Card = lore_llm::complete_as(route.provider.as_ref(), &req).await?;
// streaming:  route.provider.stream(&req, &|ev| { let _ = tx.send(ev); }).await?
// the user picked an engine for this call (no fallback):
let route = router.provider_for_with("jl-generate", &RouteOptions::only("claude-api:selected")).await
    .map_err(|e| e.reason())?;
```

### App-level refs and per-call choices

- `local:current` (`LOCAL_CURRENT`): the local engine that is connected — `LocalAccess.external`
  with any model name (the catalog adds vision/context when it knows the name; no name =
  `local`), else whatever `LocalRuntime` is running. Fails with "沒有連接本機模型".
- `claude:selected` / `claude-api:selected` / `claude-code:selected` (`CLAUDE_SELECTED`): the
  model in `StandardFactory.selected_claude` (alias or id; an id not in the catalog is used as
  is with a 200k context), or `catalog.cloud.default` when unset.
- An engine switch in the UI: `router.provider_for_with(task, &RouteOptions::only(r))` — only
  that ref; failure is `Unavailable` with one skip (`e.reason()` = why) or `NeedsNetwork`.
  `RouteOptions::prefer(r)` tries it first, then the usual order. Neither changes the
  router's saved overrides, so one router can serve concurrent calls.
- Sampling: `LlmRequest.temperature` / `top_p` go to OpenAI-compatible engines only. The
  Claude API rejects sampling parameters on current models and the Claude Code CLI has no
  flag for them, so `ClaudeApi` and `ClaudeCode` leave them out (no error).
- An app's own model folders: `LocalAccess { layout: Some(Layout::new(gguf, runtime)), .. }`
  (wins over `home`).

### Task classes and shared routes (`shared`)

Every task has `class = "create" | "tidy" | "ask"`. Once the app mounts `lore-engines`
(which calls `shared::install` with `private/shared/engine.toml`), every `Router` of the
process reads the shared settings on each route; before that, nothing changes.

- Mode = per-task override (`[overrides]`) → class route (`[route]`) → `auto`.
- `auto`: create = Claude → local, tidy / ask = local → Claude. The call's ref and the
  saved `[override]` model still come first. A classed task that names no model of a kind
  gets `claude:selected` / `local:current`.
- `cloud` / `local`: that kind only. When it cannot run, `RouteError::Unavailable` carries
  `alternative` (`{ engine, label: 「改用本機」|「改用 Claude」, model_ref }`, only when the
  other engine passes `ProviderFactory::check`, which starts nothing); the UI runs that one
  call with `RouteOptions::only(model_ref)`. `e.payload(reason)` is the JSON body.
  Offline with `cloud` is `Unavailable` (skip 「離線」) with the local alternative.
- A strict per-call choice ignores routes; its failure offers the other engine too.
- `[claude] model` replaces `selected` in `claude:selected`; `[claude] via` turns `claude:`
  into `claude-code:` / `claude-api:` (not for tasks that need images).
- `TaskTable::candidates_for(task, opts)` = the same list for code that walks candidates
  itself; `shared::last_used(task)` = what the task last ran on.
- Keys: `secrets::SharedKeys::for_app(app, dir)` reads the `LoreForge` keychain service,
  then the app's old one (and the other apps'); saves only under `LoreForge`.

Candidate order without shared settings is (call's ref →) override → default → fallback. For each: offline + cloud → skipped;
the factory builds it (or says why not: "Claude 訂閱尚未登入", "還沒下載 …", "不能看圖");
its capabilities must cover `needs`. `Route.fallback_used` and `Route.skipped` let the UI
say "改用本機模型（Claude：離線）". `claude:` tries the subscription first when
`prefer_subscription`, but skips it for needs it lacks (vision, streaming, web without
`allow_web`).

## Migration map

### JELearner (`crates/core`, `crates/server`)

| JL today | lore-llm |
|---|---|
| `provider::LlmProvider { id, complete(&CompletionRequest) -> String, chat(&ChatRequest) }` | `LlmProvider { id, capabilities, send(&LlmRequest) -> LlmResponse, stream, complete(&LlmRequest) -> String }` |
| `CompletionRequest { system, user, max_tokens, json_schema }` | `LlmRequest::prompt(system, user).max_tokens(n)` + `.json(schema)` |
| `ChatRequest { system, messages, … }`, `ChatMessage::{user, assistant}`, `Role` | `LlmRequest::chat(system, messages)`, `Message::{user, assistant}`, `Role` |
| `ChatRequest::flatten() -> CompletionRequest` | `LlmRequest::flatten() -> String` (user prompt only) |
| `ProviderError` (all variants) | `LlmError` (alias `ProviderError`, same variants + `Unsupported`) |
| `ClaudeProvider::new(ClaudeConfig { base_url, model, effort, fallbacks }, key)` | `ClaudeApi::new(ClaudeApiConfig { base_url, model, effort, fallbacks, ..ClaudeApiConfig::new(model) }, key)` |
| `ClaudeProvider::body` / `chat_body` | `ClaudeApi::body` |
| `CLAUDE_API_VERSION`, `CLAUDE_FALLBACK_BETA`, `supports_fallbacks` | `claude_api::{API_VERSION, FALLBACK_BETA, supports_fallbacks}` |
| `LlamaServerProvider::new(LlamaServerConfig { url, model })` | `LlamaServer::new(OpenAiCompatConfig::llama_server(url, model))` (set `vision`, `context` from the catalog) |
| `MockProvider { seen, seen_chats }` | `MockProvider { seen }` (one list of `LlmRequest`) |
| `claude_code::{Cli, binary, status, Status, Cache, start_login, parse_output, fake, ALLOW_ENV, STRIP_ENV, INSTALL_HINT, SETUP_PAGE}` | same names in `lore_llm::claude_code` (fake now also answers `--help`; default account `ellrin@example.com`; `reply.txt` for plain replies) |
| `ClaudeCodeProvider::new(cli, model, effort)`, `.args(req)` | `ClaudeCode::new(cli, model, effort)` (+ `temp_prefix = "jelearner-claude"`), `.args(req, isolation)` |
| `local_llm::{home, find_model, find_mmproj, find_runtime, missing, start, Running}` | `lore_llm::local::{…}` same signatures; plus `LocalRuntime` for the start/stop bookkeeping in `settings.rs` |
| `catalog::{Catalog, Model, Cloud, CloudModel, Defaults, plan, gpu_usable}` | `lore_llm::catalog::{…}`; `Defaults.ears/mouth` are `Option`; filter `brains(false)` (no abliterated models in JL) |
| `models/catalog.json` | the shared `lore-llm/catalog.json` (v2: adds `vision`, `uncensored`, `cloud.aliases`, cloud `context`); JL may keep its own file in v2 shape via `Catalog::load(Some(path))` |
| `Task::Generate` / `Task::Realtime` + `provider_task` | tasks `jl-generate` / `jl-realtime` through `Router` (or keep `provider_task` building `ClaudeApi` from `catalog.cloud.realtime`) |
| `[engine] kind = subscription / api / llama-server` (explicit, no fallback) | `provider_for_with(task, &RouteOptions::only("claude-code:selected" / "claude-api:selected" / "local:current"))`; `auto` = `provider_for(task)`. The settings' Claude model → `StandardFactory.selected_claude`; the `[llama] url` → `LocalAccess.external` |
| `server::secrets::{key, save_key, mask, KeySource, Who}` | `lore_secrets::KeyStore` (= `lore_llm::secrets::KeyStore`) `::anthropic("JELearner", Some(app_dir))` (`use_keychain` from settings) → `.get(explicit, &Account::User(who))` / `.save(k, &Account::User(who))`; `who = None` → `Account::Nobody`; same file layout and legacy rule |

### PaperComposer (`crates/server`)

| PC today | lore-llm |
|---|---|
| `claude::Claude::new(model).complete(&Prompt)` | `ClaudeApi` (model from catalog/config) `.complete(&LlmRequest::prompt(p.system, p.user).max_tokens(4096))`. **Behaviour change (intended):** `anthropic-version` 2023-06-01 → 2026-10-21, no `temperature` to Claude (rejected by current models; `LlmRequest.temperature(0.2)` still reaches local engines), fallbacks on, `stop_reason` checked |
| `claude::{DEFAULT_MODEL, MODELS}` | `catalog.cloud.default`, `catalog.cloud.models` (no model names in code); the model picked in settings → `StandardFactory.selected_claude` and `claude:selected` in tasks.toml |
| engine switch 本機 / 訂閱 / API | `RouteOptions::only("local:current" / "claude-code:selected" / "claude-api:selected")`; auto = the task's order |
| `claude::{key, save_key, mask, KeySource}` (service `PaperComposer`, account `anthropic-api-key`) | `KeyStore::anthropic("PaperComposer", None).get(None, &Account::Single)` / `.save(k, &Account::Single)` (stale-item repair kept; `app_dir: None` keeps "never in a file") |
| `claude::KEYS_PAGE` | `secrets::KEYS_PAGE` (`open_in_browser` stays in PC) |
| `PAPERCOMPOSER_CLAUDE_BASE_URL` | `ClaudeApiConfig.base_url` (PC reads the variable) |
| `claude_code::{binary(), status(), Cache, start_login()}` | `claude_code::{binary(None), status(&cli), Cache::get(&cli, fresh), start_login(&cli)}` with `Cli::default()`; login now uses `--claudeai` |
| `claude_code::complete(&prompt, model, extra)` | `ClaudeCode::new(Cli::default(), model, None)` (+ `temp_prefix = "papercomposer-claude"`) `.complete(&LlmRequest::prompt(system, user + extra))`. Env is now an allowlist (PC stripped 5 vars) |
| `engine::{Engine, EngineConfig { url, model }, EngineStatus}` | `LocalAccess.external = Some(ExternalServer { url, model })` and the ref `local:current` (any model name); `LocalAccess::current(&catalog)` → `OpenAiCompat` for `ping()`; env overrides stay in PC. PC sent `json_object`; send `.json(schema)` and `.temperature(0.2)`, set `zh_tw = true` |
| `zhtw::to_taiwan` | `lore_llm::zhtw::to_taiwan` (automatic with `OpenAiCompatConfig.zh_tw`) |
| `models::{Catalog, ModelEntry}` | `catalog::{Catalog, Model}`: `default` → `defaults.brain`; `approx_gb` f32→f64; `min_memory_gb` u32→f64; `context` u32→`Option<u32>` (`context_or_default()`) |
| `models::recommend(&cat, &SystemInfo)` | `catalog::recommend(&cat, &Machine { memory_gb, gpu_memory_gb })` (`system_info` stays in PC) |
| `models::{llm_home, models_dir, runtime_dir, runtime_binary, find_local}` | `local::Layout`: `Layout::shared(home)` with `LOCAL_LLM_HOME`, else `Layout::new(<data>/models, <data>/llama.cpp)` (PC's first layout); `.find_runtime()`, `.find_model(m)`, `.installed(cat, true)` |
| `models::{Manager, start, stop, kill_runtime, register, Running}` | `local::LocalRuntime::{ensure_in(&layout, m), current_info, stop, kill_now, phase}` (download progress map stays in PC) |
| `models::{plan, install, hf_find, pick_runtime, extract, InstalledModel, installed}` | stay in PC for now (downloads); `local::installed` covers catalog models already on disk |
| "writing: local engine else Claude" | PC tasks (`pc-compose-paragraph`, …): `default = "local:current"`, `fallback = "claude:selected"` via `Router` |

## Not tested on real hardware

- `ClaudeApi` against the real API (mock servers only), incl. streaming web search and
  `pause_turn` resumption.
- `ClaudeCode` with a real account / real CLI (fake CLI only), incl. `--tools WebSearch
  --allowedTools WebSearch`, `CLAUDE_CONFIG_DIR` on macOS (Keychain-held credentials).
- `LocalRuntime` with a real llama-server (`cargo test -p lore-llm real_local -- --ignored`
  needs `LOCAL_LLM_HOME` with llama.cpp and Qwen3.5 4B); vision through `--mmproj`.
- `secrets` (`lore-secrets`) against the real OS keychain (tests use the file fallback only).
- `temperature` / `top_p` with a real llama-server (request bodies checked against mocks).
