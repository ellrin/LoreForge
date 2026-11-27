//! One sentence about a figure's placement through a real engine: the commands the model gives,
//! what they changed, what could not be done.
//! `cargo run -p lore-flow --example flow_say -- <spec.json> <flow dir> <sentence> <out commands.json>`
//! `SETTLE=1`: the structure settles first (as the author would see the figure); `ENGINE=<ref>`:
//! only that engine. One model call, two when the first list cannot all be done.
use std::sync::Arc;

use lore_flow::figure::{Catalogue, FlowSpec};
use lore_flow::llm::Llm;
use lore_flow::say;
use lore_llm::router::{LocalAccess, SubscriptionAccess};
use lore_llm::{Catalog, LlmProvider, LlmRequest, RouteOptions, Router, StandardFactory, TaskTable};

struct Engine(Arc<dyn LlmProvider>, std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl Llm for Engine {
    async fn complete_json(&self, _task: &str, req: LlmRequest) -> Result<(serde_json::Value, String), String> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        lore_llm::complete_json(self.0.as_ref(), &req).await.map(|v| (v, self.0.id())).map_err(|e| e.to_string())
    }
}

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let text = std::fs::read_to_string(&a[0]).expect("read the figure");
    let spec = FlowSpec::from_value(serde_json::from_str(&text).expect("json")).expect("spec");
    let cat = Catalogue::load(Some(std::path::Path::new(&a[1])));
    let (g, _, _) = spec.prepare(&cat, spec.frame_rect()).expect("prepare");
    let g = if std::env::var("SETTLE").is_ok() { lore_flow::settle::settle(&g).0 } else { g };
    let mut factory = StandardFactory::new(Catalog::load(None));
    factory.subscription = Some(SubscriptionAccess { cli: Default::default(), cache: Arc::new(Default::default()), allow_web: false, temp_prefix: "lore-flow-say".into() });
    factory.local = Some(LocalAccess::default());
    let router = Router::new(TaskTable::bundled().merge(lore_flow::llm::task_table()), factory);
    let opts = std::env::var("ENGINE").ok().map(|e| RouteOptions::only(&e)).unwrap_or_default();
    let route = router.provider_for_with(say::TASK, &opts).await.expect("an engine for flow-command");
    let engine = Engine(route.provider.clone(), Default::default());
    let t = std::time::Instant::now();
    match say::say(&engine, &g, &a[2]).await {
        Ok(s) => {
            println!("engine {}; calls {}; {:.1} s; retried {}", s.provider, engine.1.load(std::sync::atomic::Ordering::SeqCst), t.elapsed().as_secs_f64(), s.retried);
            println!("commands {}", serde_json::to_string(&s.commands).unwrap());
            for d in &s.done {
                println!("  said: {d}");
            }
            for c in &s.cannot {
                println!("  cannot: {}", c.why);
            }
            if !s.unclear.is_empty() {
                println!("  unclear: {}", s.unclear);
            }
            std::fs::write(&a[3], serde_json::to_string_pretty(&s.commands).unwrap()).expect("write the commands");
        }
        Err(e) => println!("failed after {} calls: {e}", engine.1.load(std::sync::atomic::Ordering::SeqCst)),
    }
}
