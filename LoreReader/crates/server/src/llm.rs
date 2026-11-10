//! Model calls through lore-llm, wired like PaperComposer's: every call is a task from
//! `tasks.toml` (`lr-translate` → `local:current` then `claude:selected`; `lr-chat` →
//! `claude:selected`). Local engines get temperature 0.2 and Taiwan usage; the API key
//! comes from the OS keychain (service `LoreReader`) or ANTHROPIC_API_KEY, never a file.

use std::sync::Arc;

use lore_llm::catalog::Catalog;
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::local::LocalRuntime;
use lore_llm::router::{ApiAccess, ExternalServer, LocalAccess, StandardFactory, SubscriptionAccess};
use lore_llm::secrets::{Account, KeyStore};
use lore_llm::{LlmRequest, Router, TaskTable};
use lr_core::config::EngineConfig;

const TASKS: &str = include_str!("../tasks.toml");
pub const SERVICE: &str = "LoreReader";
pub const LOCAL_TEMPERATURE: f32 = 0.2;

pub struct Llm {
    pub catalog: Catalog,
    tasks: TaskTable,
    pub keys: KeyStore,
    pub cli: Cli,
    pub cc: Arc<Cache>,
    runtime: Arc<LocalRuntime>,
    external: Option<ExternalServer>,
    model: String,
    base_url: String,
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
            keys: KeyStore::anthropic(SERVICE, None),
            cli: Cli::default(),
            cc: Arc::new(Cache::default()),
            runtime: Arc::new(LocalRuntime::new()),
            external: engine.url.clone().map(|url| ExternalServer { url, model: engine.model.clone() }),
            model,
            base_url: std::env::var("LOREREADER_CLAUDE_BASE_URL").unwrap_or_else(|_| lore_llm::claude_api::DEFAULT_BASE_URL.into()),
        }
    }

    /// Never the real keychain, environment key or `claude` CLI (tests).
    pub fn isolate(&mut self) {
        self.keys.use_keychain = false;
        self.keys.env_var = None;
        self.cli = Cli { bin: Some(std::env::temp_dir().join("lr-no-such-claude")), config_dir: None };
    }

    /// The app's tasks over lore-llm's shared ones, then the user's file.
    pub fn task_table(user_file: Option<&std::path::Path>) -> Result<TaskTable, String> {
        let base = TaskTable::bundled().merge(TaskTable::parse(TASKS)?);
        TaskTable::load(base, user_file.filter(|p| p.is_file()))
    }

    pub fn has_local(&self) -> bool {
        self.external.is_some()
    }

    fn factory(&self) -> StandardFactory {
        let mut f = StandardFactory::new(self.catalog.clone());
        f.prefer_subscription = true;
        f.selected_claude = Some(self.model.clone());
        f.api = self.keys.get(None, &Account::Single).map(|(k, _)| ApiAccess { base_url: self.base_url.clone(), api_key: k, fallbacks: true });
        f.subscription = Some(SubscriptionAccess { cli: self.cli.clone(), cache: self.cc.clone(), allow_web: false, temp_prefix: "lorereader-claude".into() });
        f.local = Some(LocalAccess {
            external: self.external.clone(),
            runtime: Some(self.runtime.clone()),
            zh_tw: true,
            ..Default::default()
        });
        f
    }

    /// Run `req` as `task` in the task's order (local first for translation).
    pub async fn complete(&self, task: &str, req: LlmRequest) -> Result<Answer, String> {
        let router = Router::new(self.tasks.clone(), self.factory());
        let route = router.provider_for(task).await.map_err(|e| e.to_string())?;
        let req = route.apply(req.temperature(LOCAL_TEMPERATURE));
        let text = route.provider.complete(&req).await.map_err(|e| e.to_string())?;
        Ok(Answer { text: text.trim().to_string(), engine: route.provider.id().to_string(), fallback_used: route.fallback_used })
    }
}

pub const TRANSLATE_SYSTEM: &str = "你是學術論文的翻譯助手。把使用者選取的英文翻成台灣繁體中文。\
如果選取的是一個術語或短詞，第一行給常用的中文譯名（保留英文原文），第二行用一句話說明它在這段上下文裡的意思。\
如果是句子或段落，直接給通順的譯文。只輸出譯文與說明，不要加前言。";

pub fn chat_system(title: &str) -> String {
    format!(
        "你是作者讀論文時的助手。論文：《{title}》。作者會附上原文引文（含頁碼與所在段落）。\
用台灣繁體中文回答，術語保留英文；提到原文時標出頁碼（p. N）。只根據附上的內容和一般知識回答，不確定就直說。"
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
    }

    #[tokio::test]
    async fn nothing_configured_explains_why() {
        let mut l = Llm::new(&EngineConfig::default(), None, None);
        l.isolate();
        let e = l.complete("lr-translate", LlmRequest::prompt("s", "u")).await.err().unwrap();
        assert!(!e.is_empty());
        assert!(l.complete("lr-chat", LlmRequest::prompt("s", "u")).await.is_err());
    }
}
