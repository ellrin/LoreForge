//! `cargo run -p lore-upscale --example upscale -- in.png out.png [--model anime|video|general]
//! [--scale N] [--tile N] [--dir <folder with the binary and models>] [--download]`
//!
//! `--download` fetches the pinned official release into `<model folder>/runtimes/realesrgan`
//! first (the model folder: `LOCAL_LLM_HOME`, else `[local] home` of `private/shared/engine.toml`).
//! Takes the shared runtime's slot while running, like an app would.

use lore_llm::shared_runtime::SharedRuntime;
use lore_upscale::{CancelToken, Install, Model, Options, Progress, Source, download, upscale_with};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let (mut files, mut o, mut dl) = (Vec::new(), Options::default(), false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--model" => {
                o.model = match args.next().as_deref() {
                    Some("video") => Model::AnimeVideo,
                    Some("general") => Model::GeneralX4,
                    _ => Model::AnimeX4,
                }
            }
            "--scale" => o.scale = args.next().and_then(|v| v.parse().ok()).unwrap_or(4),
            "--tile" => o.tile = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--dir" => o.install = args.next().map(Install::from_dir),
            "--download" => dl = true,
            _ => files.push(PathBuf::from(a)),
        }
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    if dl {
        let install = o.install.clone().or_else(|| Install::discover(&cwd)).expect("no model folder (LOCAL_LLM_HOME or engine.toml [local] home)");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(download(&install, &Source::official(), &CancelToken::new(), |p| match p {
            Progress::Downloading { done, total: Some(t) } => eprint!("\rdownload {:.0}%", done as f64 * 100.0 / t as f64),
            Progress::Downloading { .. } => {}
            Progress::Verifying => eprintln!("\nverifying"),
            Progress::Installing => eprintln!("installing"),
        }));
        match r {
            Ok(()) => eprintln!("installed in {}", install.dir.display()),
            Err(e) => return eprintln!("{e}"),
        }
    }
    let [input, output] = files.as_slice() else { return eprintln!("usage: upscale in.png out.png [--model anime|video|general] [--scale N] [--tile N] [--dir D] [--download]") };
    o.runtime = SharedRuntime::process("up");
    match upscale_with(input, output, &o, &mut |p| eprint!("\r{p:.1}%")) {
        Ok(()) => eprintln!("\n-> {}", output.display()),
        Err(e) => eprintln!("\n{e}"),
    }
    SharedRuntime::shutdown_process();
}
