//! `lore-modeld` — the shared local-model broker (see the crate README).
//!
//! ```text
//! lore-modeld [--home <LoreForge folder>] [--gguf <dir> --runtime <dir>]
//!             [--linger <seconds>] [--slots <n>] [--promote-after <seconds, 0 = never>]
//! ```
//!
//! Apps start it through `lore_llm::broker::ensure`; a second start exits at once.

use lore_llm::catalog::Catalog;
use lore_llm::local::{self, Layout, LocalRuntime};
use lore_modeld::upstream::LocalUpstream;
use lore_modeld::{Options, Outcome, serve};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

struct Args {
    home: Option<PathBuf>,
    gguf: Option<PathBuf>,
    runtime: Option<PathBuf>,
    linger: Option<Duration>,
    slots: Option<usize>,
    promote_after: Option<Option<Duration>>,
}

fn secs(v: &str) -> Result<Duration, String> {
    v.parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0).map(Duration::from_secs_f64).ok_or_else(|| format!("不是秒數：{v}"))
}

fn parse() -> Result<Args, String> {
    let mut a = Args { home: None, gguf: None, runtime: None, linger: None, slots: None, promote_after: None };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{k} 後面少了值"));
        match k.as_str() {
            "--home" => a.home = Some(val()?.into()),
            "--gguf" => a.gguf = Some(val()?.into()),
            "--runtime" => a.runtime = Some(val()?.into()),
            "--linger" => a.linger = Some(secs(&val()?)?),
            "--slots" => a.slots = Some(val()?.parse().map_err(|_| "--slots 要是正整數".to_string())?),
            "--promote-after" => {
                let d = secs(&val()?)?;
                a.promote_after = Some((!d.is_zero()).then_some(d));
            }
            "-h" | "--help" => {
                println!("lore-modeld [--home <dir>] [--gguf <dir> --runtime <dir>] [--linger <s>] [--slots <n>] [--promote-after <s>]");
                std::process::exit(0);
            }
            other => return Err(format!("不認得的參數：{other}")),
        }
    }
    Ok(a)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() {
    let args = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lore-modeld: {e}");
            std::process::exit(2);
        }
    };
    let Some(state) = lore_llm::broker::state_path() else {
        eprintln!("lore-modeld: 找不到快取資料夾（設定 LOREFORGE_CACHE）");
        std::process::exit(2);
    };
    let start = args.home.clone().or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from))).unwrap_or_default();
    let home = lore_home::Home::discover(&start);

    let layout = match (args.gguf, args.runtime) {
        (Some(g), Some(r)) => Some(Layout::new(g, r)),
        (None, None) => local::home(None).map(|h| Layout::shared(&h)),
        _ => {
            eprintln!("lore-modeld: --gguf 和 --runtime 要一起給");
            std::process::exit(2);
        }
    };
    let rt = Arc::new(LocalRuntime::new());
    let h = home.clone();
    rt.idle_from(Arc::new(move || lore_engines::idle_duration(lore_engines::idle_minutes(&h))));
    rt.watch_idle();
    let upstream = Arc::new(LocalUpstream { runtime: rt.clone(), layout, catalog: Catalog::bundled() });

    let mut opts = Options::new(state);
    if let Some(l) = args.linger {
        opts.linger = l;
    }
    if let Some(n) = args.slots {
        opts.sched.slots = n.max(1);
    }
    if let Some(p) = args.promote_after {
        opts.sched.promote_after = p;
    }
    match serve(opts, upstream, shutdown_signal()).await {
        Ok(Outcome::AlreadyRunning(url)) => println!("lore-modeld 已在執行：{url}"),
        Ok(Outcome::Stopped) => {}
        Err(e) => {
            eprintln!("lore-modeld: {e}");
            rt.kill_now();
            std::process::exit(1);
        }
    }
}
