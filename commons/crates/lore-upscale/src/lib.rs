//! The shared image upscaler (docs/img2img.md, phase 1).
//!
//! Runs the Real-ESRGAN **ncnn-vulkan** command line (BSD-3-Clause; the macOS release is a
//! universal binary, arm64 native) as a child process:
//!
//! ```no_run
//! use lore_upscale::{Install, Model, Options, upscale};
//! let mut o = Options::new(Model::AnimeX4, 4);
//! o.install = Some(Install::in_home(std::path::Path::new("/models")));
//! upscale("in.png".as_ref(), "out.png".as_ref(), &o).unwrap();
//! ```
//!
//! - **Where it lives**: not in the repo. `<[local] home>/runtimes/realesrgan/` of the shared
//!   model folder (`engine.toml`, or `LOCAL_LLM_HOME`) holds the binary, `models/`, the licence
//!   texts and `install.json`. [`download`] puts it there (official release, SHA-256 pinned,
//!   quarantine attribute removed, chmod); a UI calls it only when the author presses 下載.
//! - **Missing binary or models** give [`UpscaleError::NotInstalled`] with the folder and what
//!   is absent; nothing is downloaded by itself.
//! - **Progress**: the CLI prints `97.50%` lines on stderr; [`parse_progress`] reads them.
//!   **Cancel**: [`CancelToken`] kills the child; the output file is written next to its final
//!   name and renamed only on success, so a cancel or failure never leaves a half image or
//!   replaces an existing one.
//! - **The runtime's slot**: with [`Options::runtime`] the run takes an "image tool" lease of
//!   `lore_llm::shared_runtime` (`begin_tool_blocking`): it waits until no local-model request
//!   runs, and while it runs the local language model's requests wait. Without one nothing
//!   is yielded (a tool that does not know the runtime).
//!
//! Tested with a fake CLI (a shell script) and a local fake download server; the real binary
//! is one `#[ignore]`d test (`REALESRGAN_DIR=<folder with the binary and models>`).

mod install;
mod run;

pub use install::{DEFAULT_SHA256, DEFAULT_URL, Install, Progress, Source, download};
pub use run::{CancelToken, Model, Options, UpscaleError, build_args, parse_progress, upscale, upscale_with};

/// Folder of the tool inside the model folder: `<home>/runtimes/realesrgan`.
pub const RUNTIME_DIR: &str = "runtimes/realesrgan";
/// The executable inside it.
pub const BINARY: &str = "realesrgan-ncnn-vulkan";
