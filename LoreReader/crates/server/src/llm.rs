//! Model calls through lore-llm, wired like PaperComposer's: every call is a task from
//! `tasks.toml` (`lr-translate` → `local:current` then `claude:selected`; `lr-chat` →
//! `claude:selected`). Local engines get temperature 0.2 and Taiwan usage; the API key
//! comes from the shared LoreForge keychain item (the old `LoreReader` one still read) or
//! ANTHROPIC_API_KEY, never a file. A route that cannot run comes back as
//! [`LlmFail`]: lore-llm's JSON payload with the one-click alternative (「改用本機」…).
//!
//! The local engine (`local:current`): config.toml `[engine]` / `LOREREADER_LLM_URL`, else
//! the model all LoreForge apps share (`private/shared/engine.toml` `[engine] model`, re-read on
//! every call) on the machine's one llama-server (lore-llm `shared_runtime`, docs/local-models.md).

use std::sync::Arc;

use lore_llm::catalog::Catalog;
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::router::{ApiAccess, ExternalServer, LocalAccess, StandardFactory, SubscriptionAccess};
use lore_llm::shared_runtime::SharedRuntime;
use lore_llm::secrets::{Account, SharedKeys};
use lore_llm::{LlmRequest, RouteOptions, Router, TaskTable};
use lr_core::config::EngineConfig;

pub const TASKS: &str = include_str!("../tasks.toml");
pub const SERVICE: &str = "LoreReader";
pub const LOCAL_TEMPERATURE: f32 = 0.2;
/// Short name in the shared local-model registry (and `ports.json`).
pub const APP: &str = "lr";

/// At quit: this app's hold on the shared local model goes; the server stops when no other
/// app uses it.
pub fn quit() {
    SharedRuntime::shutdown_process();
}

pub struct Llm {
    pub catalog: Catalog,
    tasks: TaskTable,
    pub keys: SharedKeys,
    pub cli: Cli,
    pub cc: Arc<Cache>,
    /// The machine's shared llama-server (`None`: isolated).
    pub local_rt: Option<Arc<SharedRuntime>>,
    external: Option<ExternalServer>,
    /// Where the shared model choice is kept (`private/shared/engine.toml`), used when the
    /// app's own config names no server.
    shared: Option<lore_home::Home>,
    model: String,
    base_url: String,
}

/// Why a model call did not run: lore-llm's payload (`{ error, alternative? }`), so the
/// page can offer 「改用本機」／「改用 Claude」.
#[derive(Debug, Clone)]
pub struct LlmFail(pub serde_json::Value);

impl LlmFail {
    pub fn message(m: impl Into<String>) -> LlmFail {
        LlmFail(serde_json::json!({ "error": m.into() }))
    }
}

impl std::fmt::Display for LlmFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0["error"].as_str().unwrap_or("模型呼叫失敗"))
    }
}

/// What a model answered, and with which engine.
pub struct Answer {
    pub text: String,
    pub engine: String,
    pub fallback_used: bool,
}

impl Llm {
    pub fn new(engine: &EngineConfig, claude_model: Option<String>, user_tasks: Option<&std::path::Path>) -> Llm {
        let catalog = Catalog::bundled();
        let tasks = Llm::task_table(user_tasks).unwrap_or_else(|e| {
            eprintln!("lr-server: tasks.toml: {e} (using the built-in tasks)");
            Llm::task_table(None).expect("built-in tasks")
        });
        let model = claude_model.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| catalog.cloud.default.clone());
        Llm {
            catalog,
            tasks,
            keys: SharedKeys::for_app(SERVICE, None),
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            local_rt: SharedRuntime::process(APP),
            external: engine.url.clone().map(|url| ExternalServer { url, model: engine.model.clone() }),
            shared: None,
            model,
            base_url: std::env::var("LOREREADER_CLAUDE_BASE_URL").unwrap_or_else(|_| lore_llm::claude_api::DEFAULT_BASE_URL.into()),
        }
    }

    /// Never the real keychain, environment key or `claude` CLI (tests).
    pub fn isolate(&mut self) {
        self.keys.isolate();
        self.cli = Cli { bin: Some(std::env::temp_dir().join("lr-no-such-claude")), config_dir: None };
        self.local_rt = None;
    }

    /// The app's tasks over lore-llm's shared ones, then the user's file.
    pub fn task_table(user_file: Option<&std::path::Path>) -> Result<TaskTable, String> {
        let base = TaskTable::bundled().merge(TaskTable::parse(TASKS)?);
        TaskTable::load(base, user_file.filter(|p| p.is_file()))
    }

    /// Use the shared model choice of `home` when the app's own config names no server.
    pub fn share_engine(&mut self, home: lore_home::Home) {
        self.shared = Some(home);
    }

    /// The app's own server (`[engine]` / `LOREREADER_LLM_URL`), when set.
    pub fn external(&self) -> Option<ExternalServer> {
        self.external.clone()
    }

    /// `engine.toml` `[engine]` (the shared default model ids).
    fn shared_engine(&self) -> lore_home::engine::SharedEngine {
        self.shared.as_ref().and_then(|h| h.engine()).unwrap_or_default()
    }

    /// A local engine to ask: the own server, or a shared model on the shared llama-server.
    pub fn has_local(&self) -> bool {
        self.external().is_some() || (self.local_rt.is_some() && self.shared_engine().model.is_some())
    }

    fn factory(&self) -> StandardFactory {
        let mut f = StandardFactory::new(self.catalog.clone());
        f.prefer_subscription = true;
        f.selected_claude = Some(self.model.clone());
        f.api = self.keys.get(None, &Account::Single).map(|(k, _)| ApiAccess { base_url: self.base_url.clone(), api_key: k, fallbacks: true });
        f.subscription = Some(SubscriptionAccess { cli: self.cli.clone(), cache: self.cc.clone(), allow_web: false, temp_prefix: "lorereader-claude".into() });
        let e = self.shared_engine();
        f.local = Some(LocalAccess {
            external: self.external(),
            shared: self.local_rt.clone(),
            default_model: e.model,
            fallback_home: self.shared.as_ref().and_then(|h| h.local_home()),
            vision_model: e.vision_model,
            zh_tw: true,
            ..Default::default()
        });
        f
    }

    /// Run `req` as `task` in the task's order (local first for translation).
    pub async fn complete(&self, task: &str, req: LlmRequest) -> Result<Answer, LlmFail> {
        self.complete_on(task, None, req).await
    }

    /// Run `req` as `task` on the engine picked in the model menu (`local`, `haiku`,
    /// `sonnet`, `opus`): that engine or the reason it cannot run, no fallback.
    /// `None` / `auto`: the task's own order from tasks.toml.
    pub async fn complete_on(&self, task: &str, choice: Option<&str>, req: LlmRequest) -> Result<Answer, LlmFail> {
        let router = Router::new(self.tasks.clone(), self.factory());
        let pick = model_ref(choice).map_err(LlmFail::message)?;
        let strict = pick.is_some();
        // translation and the chat are read on screen as they come: interactive for a local
        // model; term classification and paragraph runs keep their class default (background)
        let prio = |o: RouteOptions| match task {
            "lr-translate" | "lr-chat" => o.with_priority(lore_llm::shared_runtime::Priority::Interactive),
            _ => o,
        };
        let route = match pick {
            Some(r) => router.provider_for_with(task, &prio(RouteOptions::only(r))).await,
            None => router.provider_for_with(task, &prio(RouteOptions::default())).await,
        }
        .map_err(|e| LlmFail(e.payload(strict)))?;
        let req = route.apply(req.temperature(LOCAL_TEMPERATURE));
        let text = route.provider.complete(&req).await.map_err(|e| LlmFail::message(e.to_string()))?;
        Ok(Answer { text: text.trim().to_string(), engine: route.provider.id().to_string(), fallback_used: route.fallback_used })
    }
}

/// The tasks the model menu offers, in menu order.
pub const MENU_TASKS: [&str; 4] = ["lr-classify-terms", "lr-paragraph", "lr-chat", "lr-translate"];
/// The engines of a feature's engine chip (`auto` = the task's order from tasks.toml,
/// `cloud` = the selected Claude model, `local` = the local engine). haiku / sonnet / opus
/// are the old model menu's choices: still accepted, so an older models.json keeps working.
pub const CHOICES: [&str; 6] = ["auto", "local", "cloud", "haiku", "sonnet", "opus"];

/// A menu choice as a lore-llm model ref (`None` = the task's own order).
pub fn model_ref(choice: Option<&str>) -> Result<Option<String>, String> {
    match choice.map(str::trim).filter(|m| !m.is_empty() && *m != "auto") {
        None => Ok(None),
        Some("local") => Ok(Some("local:current".into())),
        Some("cloud") => Ok(Some("claude:selected".into())),
        Some(a @ ("haiku" | "sonnet" | "opus")) => Ok(Some(format!("claude:{a}"))),
        Some(other) => Err(format!("不認得的模型選擇「{other}」")),
    }
}

impl Llm {
    /// The task's label and default for the model menu.
    pub fn task_info(&self, task: &str) -> (String, String) {
        match self.tasks.task.get(task) {
            Some(t) => (t.label.clone().unwrap_or_else(|| task.into()), t.default.clone()),
            None => (task.into(), String::new()),
        }
    }
}

pub const TRANSLATE_SYSTEM: &str = "你是學術論文的翻譯助手。把使用者選取的英文翻成台灣繁體中文。\
如果選取的是一個術語或短詞，第一行給常用的中文譯名（保留英文原文），第二行用一句話說明它在這段上下文裡的意思。\
如果是句子或段落，直接給通順的譯文。只輸出譯文與說明，不要加前言。";

pub fn chat_system(title: &str) -> String {
    format!(
        "你是作者讀論文時的助手。論文：《{title}》。作者會附上原文引文（含頁碼與所在段落）。\
用台灣繁體中文回答，術語保留英文；提到原文時標出頁碼（p. N）。只根據附上的內容和一般知識回答，不確定就直說。\
如果附上了「卡住的地方」與作者的知識庫狀態：只解釋卡住的點，不要翻譯或摘要整段；用作者已掌握的詞來解釋不熟的詞，不要用作者還沒有卡片的詞來解釋別的詞。\
解釋符號時，說明它在這篇論文裡的意思並引用原文定義句（含頁碼）；原文沒有定義就說明是依慣例推測。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_parse_with_the_intended_order() {
        let t = Llm::task_table(None).unwrap();
        assert_eq!(t.task["lr-translate"].default, "local:current");
        assert_eq!(t.task["lr-translate"].fallback.as_deref(), Some("claude:selected"));
        assert_eq!(t.task["lr-chat"].default, "claude:selected");
        assert!(t.task["lr-chat"].fallback.is_none());
        assert_eq!(t.task["lr-classify-terms"].default, "local:current");
        assert_eq!(t.task["lr-classify-terms"].fallback.as_deref(), Some("claude:haiku"));
        assert_eq!(t.task["lr-paragraph"].default, "local:current");
        assert_eq!(t.task["lr-paragraph"].fallback.as_deref(), Some("claude:haiku"));
        assert_eq!(model_ref(Some("haiku")).unwrap().as_deref(), Some("claude:haiku"));
        assert_eq!(model_ref(Some("auto")).unwrap(), None);
        assert_eq!(model_ref(Some("cloud")).unwrap().as_deref(), Some("claude:selected"));
        assert!(model_ref(Some("gpt")).is_err());
    }

    #[tokio::test]
    async fn nothing_configured_explains_why() {
        let mut l = Llm::new(&EngineConfig::default(), None, None);
        l.isolate();
        let e = l.complete("lr-translate", LlmRequest::prompt("s", "u")).await.err().unwrap();
        assert!(!e.to_string().is_empty() && e.0["error"].is_string());
        assert!(l.complete("lr-chat", LlmRequest::prompt("s", "u")).await.is_err());
    }

    #[test]
    fn the_shared_model_is_the_default_and_the_own_config_wins() {
        let d = tempfile::tempdir().unwrap();
        let home = lore_home::Home::at(d.path());
        let mut l = Llm::new(&EngineConfig::default(), None, None);
        l.keys.isolate(); // factory() reads keys: never the real keychain (it can wait on a prompt)
        // a runtime over a temp registry, never the real cache folder
        l.local_rt = Some(SharedRuntime::new(APP, d.path().join("cache"), Default::default()));
        l.share_engine(home.clone());
        assert!(!l.has_local());
        home.set_engine_model(Some("qwen3.5-4b-q8")).unwrap();
        assert!(l.has_local(), "a shared model is picked up without a restart");
        assert!(l.external().is_none(), "never a URL from engine.toml");
        assert_eq!(l.factory().local.unwrap().default_model.as_deref(), Some("qwen3.5-4b-q8"));
        l.isolate();
        assert!(!l.has_local(), "isolated: no shared server");
        let mut own = Llm::new(&EngineConfig { url: Some("http://127.0.0.1:7000/v1".into()), model: None }, None, None);
        own.share_engine(home);
        assert_eq!(own.external().unwrap().url, "http://127.0.0.1:7000/v1");
    }
}
