//! lore-modeld — the shared local-model broker (design `docs/loreecho.md` §8).
//!
//! One small background process per machine owns the local llama-server (through one
//! [`lore_llm::local::LocalRuntime`]); every app sends its local-model requests here, on
//! the same OpenAI-compatible interface, and the broker decides who goes first
//! ([`sched`]): the focused app, then interactive before background, then first come.
//!
//! - [`serve`]: single instance (the `{port, pid}` in `<cache>/modeld.json`, see
//!   [`lore_llm::broker`]), binds `127.0.0.1` on a free port, exits once the model is
//!   released and nobody has talked to it for [`Options::linger`].
//! - [`server`]: the HTTP routes ([`server::router`]) over a [`server::Broker`].
//! - [`upstream`]: what requests go to ([`upstream::LocalUpstream`], or a fake in tests).
//!
//! The client side (find/start, headers, routing `local:` models) is
//! [`lore_llm::broker`].

pub mod sched;
pub mod server;
pub mod upstream;

use server::Broker;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Exit after the model has been released and nobody talked to the broker this long.
pub const DEFAULT_LINGER: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct Options {
    /// `<cache>/modeld.json` ([`lore_llm::broker::state_path`]).
    pub state_file: PathBuf,
    /// See [`DEFAULT_LINGER`].
    pub linger: Duration,
    pub sched: sched::Config,
}

impl Options {
    pub fn new(state_file: PathBuf) -> Options {
        Options { state_file, linger: DEFAULT_LINGER, sched: sched::Config::default() }
    }
}

/// How [`serve`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Another broker answers at this URL; nothing was started.
    AlreadyRunning(String),
    /// Served until idle (or the shutdown signal), then stopped.
    Stopped,
}

/// A short-lived lock around "is one running? if not, bind and record": two apps
/// starting the broker at the same moment get one broker.
struct StartLock(PathBuf);

impl StartLock {
    async fn take(state: &Path) -> std::io::Result<StartLock> {
        let path = state.with_extension("lock");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = write!(f, "{}", std::process::id());
                    return Ok(StartLock(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let holder = std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<u32>().ok());
                    if holder.is_none_or(|p| !lore_home::ports::pid_alive(p)) || tokio::time::Instant::now() >= deadline {
                        let _ = std::fs::remove_file(&path); // stale
                        continue;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for StartLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Run the broker over `upstream` until it has been idle for `opts.linger` or
/// `shutdown` completes. A second broker for the same state file returns
/// [`Outcome::AlreadyRunning`] at once.
pub async fn serve(opts: Options, upstream: Arc<dyn upstream::Upstream>, shutdown: impl Future<Output = ()> + Send + 'static) -> std::io::Result<Outcome> {
    let lock = StartLock::take(&opts.state_file).await?;
    if let Some(url) = lore_llm::broker::find_at(&opts.state_file).await {
        return Ok(Outcome::AlreadyRunning(url));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let pid = std::process::id();
    lore_llm::broker::write_state(&opts.state_file, port, pid)?;
    drop(lock);

    let broker = Broker::new(upstream, opts.sched.clone());
    let idle = idle_exit(broker.clone(), opts.linger);
    let app = server::router(broker.clone());
    let res = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = idle => {}
                _ = shutdown => {}
            }
        })
        .await;
    // forget the state file if it still names this process
    if lore_llm::broker::read_state(&opts.state_file).is_some_and(|(_, p)| p == pid) {
        let _ = std::fs::remove_file(&opts.state_file);
    }
    broker.upstream().release().await;
    res.map(|_| Outcome::Stopped)
}

/// Resolves once no model is loaded, nothing waits or runs, and nobody talked to the
/// broker for `linger`.
async fn idle_exit(b: Arc<Broker>, linger: Duration) {
    let tick = (linger / 4).clamp(Duration::from_millis(20), Duration::from_secs(15));
    loop {
        tokio::time::sleep(tick).await;
        if !b.is_empty() || b.upstream().loaded().await.is_some() {
            b.touch(); // in use: the linger counts from when the model went away
            continue;
        }
        if b.quiet_for() >= linger {
            return;
        }
    }
}
