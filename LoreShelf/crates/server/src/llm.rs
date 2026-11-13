//! Model calls through lore-llm: the term pool's sorting (`classify.rs`, local only) and the known
//! cards' titles (`cards.rs`, Claude Haiku: the subscription's `claude` CLI, else
//! `ANTHROPIC_API_KEY`; never the keychain, whose prompt would stop a background run). The local
//! engine is the model all LoreForge apps share (`private/shared/engine.toml`) on the machine's one
//! llama-server (lore-llm `shared_runtime`, `docs/local-models.md`), at background priority: it
//! waits while another app uses the model for the author.

use std::sync::Arc;

use lore_llm::catalog::Catalog;
use lore_llm::claude_code::{Cache, Cli};
use lore_llm::router::{ApiAccess, LocalAccess, StandardFactory, SubscriptionAccess};
use lore_llm::shared_runtime::{Priority, SharedRuntime};
use lore_llm::{LlmProvider, RouteOptions, Router, TaskTable};

pub const TASKS: &str = include_str!("../tasks.toml");
pub const TASK_CLASSIFY: &str = "shelf-classify-terms";
pub const TASK_CARD_TITLES: &str = "shelf-card-titles";
/// Short name in the shared local-model registry (and `ports.json`).
pub const APP: &str = "ls";

/// At quit: this process's hold on the shared local model goes.
pub fn quit() {
    SharedRuntime::shutdown_process();
}

pub struct Llm {
    tasks: TaskTable,
    home: Option<lore_home::Home>,
    local_rt: Option<Arc<SharedRuntime>>,
    /// Tests: this provider instead of any engine.
    fake: Option<Arc<dyn LlmProvider>>,
}

impl Llm {
    pub fn new(home: lore_home::Home) -> Llm {
        let tasks = TaskTable::bundled().merge(TaskTable::parse(TASKS).expect("built-in tasks"));
        Llm { tasks, home: Some(home), local_rt: SharedRuntime::process(APP), fake: None }
    }

    /// Answers from `p` (tests: a mock).
    pub fn fake(p: Arc<dyn LlmProvider>) -> Llm {
        Llm { tasks: TaskTable::bundled().merge(TaskTable::parse(TASKS).expect("built-in tasks")), home: None, local_rt: None, fake: Some(p) }
    }

    /// The model the machine shares, if one is chosen (for messages).
    pub fn model(&self) -> Option<String> {
        self.home.as_ref().and_then(|h| h.engine()).and_then(|e| e.model)
    }

    /// The engine for `task`, at background priority, or why there is none.
    pub async fn provider(&self, task: &str) -> Result<Arc<dyn LlmProvider>, String> {
        if let Some(p) = &self.fake {
            return Ok(p.clone());
        }
        let e = self.home.as_ref().and_then(|h| h.engine()).unwrap_or_default();
        if e.model.is_none() && task == TASK_CLASSIFY {
            return Err("沒有選本機模型（private/shared/engine.toml 的 [engine] model）".into());
        }
        let mut f = StandardFactory::new(Catalog::bundled());
        if task != TASK_CLASSIFY {
            // Claude for the tasks that may use it (their tasks.toml route says which model)
            f.prefer_subscription = true;
            // the API key only from the environment: a command-line run must not stop on a keychain prompt
            f.api = std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty()).map(|k| ApiAccess { base_url: lore_llm::claude_api::DEFAULT_BASE_URL.into(), api_key: k, fallbacks: false });
            f.subscription = Some(SubscriptionAccess { cli: Cli::default(), cache: Arc::new(Cache::default()), allow_web: false, temp_prefix: "loreshelf-claude".into() });
        }
        f.local = Some(LocalAccess {
            shared: self.local_rt.clone(),
            default_model: e.model,
            fallback_home: self.home.as_ref().and_then(|h| h.local_home()),
            zh_tw: true,
            ..Default::default()
        });
        let r = Router::new(self.tasks.clone(), f).provider_for_with(task, &RouteOptions::default().with_priority(Priority::Background)).await.map_err(|e| e.to_string())?;
        Ok(r.provider)
    }
}
