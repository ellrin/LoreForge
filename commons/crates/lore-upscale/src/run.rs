//! Arguments, child process, progress, cancellation.

use crate::install::Install;
use lore_llm::shared_runtime::SharedRuntime;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// Which network. `scale` in [`Options`] must be 4 for the 4x models; the video model also
/// does 2 and 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// `realesrgan-x4plus-anime`: drawings, anime, flat colours (x4).
    AnimeX4,
    /// `realesr-animevideov3`: small, fast; x2, x3 or x4 (the scale picks the file).
    AnimeVideo,
    /// `realesrgan-x4plus`: photos and general images (x4, the biggest, slowest).
    GeneralX4,
}

impl Model {
    /// The `-n` name for a scale (a valid one; see [`Model::supports`]).
    pub fn name(self, scale: u8) -> String {
        match self {
            Model::AnimeX4 => "realesrgan-x4plus-anime".into(),
            Model::AnimeVideo => format!("realesr-animevideov3-x{scale}"),
            Model::GeneralX4 => "realesrgan-x4plus".into(),
        }
    }

    pub fn supports(self, scale: u8) -> bool {
        match self {
            Model::AnimeVideo => matches!(scale, 2..=4),
            _ => scale == 4,
        }
    }
}

/// Cancel a running (or waiting) upscale from another thread.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What to run.
#[derive(Clone)]
pub struct Options {
    pub model: Model,
    /// 2, 3 or 4 (see [`Model::supports`]).
    pub scale: u8,
    /// Tile size, 0 = automatic, else at least 32 (smaller tiles use less memory).
    pub tile: u32,
    /// Where the tool is installed; `None` finds the shared model folder from the working
    /// directory ([`Install::discover`]).
    pub install: Option<Install>,
    pub cancel: CancelToken,
    /// The shared runtime: take its slot while running (the local language model yields).
    pub runtime: Option<Arc<SharedRuntime>>,
}

impl Options {
    pub fn new(model: Model, scale: u8) -> Options {
        Options { model, scale, tile: 0, install: None, cancel: CancelToken::new(), runtime: None }
    }
}

impl Default for Options {
    fn default() -> Self {
        Options::new(Model::AnimeX4, 4)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpscaleError {
    #[error("畫質提升工具還沒下載（缺少 {missing}，資料夾 {dir}）")]
    NotInstalled { dir: PathBuf, missing: String },
    #[error("找不到模型資料夾，無法定位畫質提升工具")]
    NoHome,
    #[error("{0}")]
    Options(String),
    #[error("找不到圖片 {0}")]
    NoInput(PathBuf),
    #[error("已取消")]
    Cancelled,
    #[error("畫質提升失敗（{code}）：{tail}")]
    Failed { code: String, tail: String },
    #[error("無法使用共用執行環境：{0}")]
    Slot(String),
    #[error("下載的檔案校驗碼不符（應為 {expected}，實際 {actual}）")]
    Checksum { expected: String, actual: String },
    #[error("下載失敗：{0}")]
    Download(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// The percentage of a progress line (`"97.50%"` -> `97.5`), `None` for any other line.
pub fn parse_progress(line: &str) -> Option<f32> {
    let n: f32 = line.trim().strip_suffix('%')?.trim().parse().ok()?;
    (n.is_finite() && (0.0..=100.0).contains(&n)).then_some(n)
}

fn format_of(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "webp" => Some("webp"),
        _ => None,
    }
}

/// The command line: `-i in -o out -n name -s scale [-t tile] -m models -f format`.
pub fn build_args(models: &Path, input: &Path, output: &Path, o: &Options) -> Result<Vec<OsString>, UpscaleError> {
    if !o.model.supports(o.scale) {
        return Err(UpscaleError::Options(format!("這個模型不支援 x{}", o.scale)));
    }
    if o.tile != 0 && o.tile < 32 {
        return Err(UpscaleError::Options("tile 要是 0（自動）或至少 32".into()));
    }
    format_of(input).ok_or_else(|| UpscaleError::Options("輸入圖片要是 png、jpg 或 webp".into()))?;
    let fmt = format_of(output).ok_or_else(|| UpscaleError::Options("輸出檔要是 png、jpg 或 webp".into()))?;
    let mut a: Vec<OsString> = vec!["-i".into(), input.into(), "-o".into(), output.into()];
    a.extend(["-n".into(), o.model.name(o.scale).into(), "-s".into(), o.scale.to_string().into()]);
    if o.tile != 0 {
        a.extend(["-t".into(), o.tile.to_string().into()]);
    }
    a.extend(["-m".into(), models.into(), "-f".into(), fmt.into()]);
    Ok(a)
}

/// [`upscale_with`] without progress.
pub fn upscale(input: &Path, output: &Path, o: &Options) -> Result<(), UpscaleError> {
    upscale_with(input, output, o, &mut |_| {})
}

/// Upscale `input` to `output` (replaced only on success). `on_progress` gets 0..=100.
/// Blocking: call it from a worker thread (`spawn_blocking`).
pub fn upscale_with(input: &Path, output: &Path, o: &Options, on_progress: &mut dyn FnMut(f32)) -> Result<(), UpscaleError> {
    let install = match &o.install {
        Some(i) => i.clone(),
        None => Install::discover(&std::env::current_dir().unwrap_or_default()).ok_or(UpscaleError::NoHome)?,
    };
    let name = o.model.name(o.scale);
    let bin = install.check(&name)?;
    if !input.is_file() {
        return Err(UpscaleError::NoInput(input.to_path_buf()));
    }
    // validate before waiting for the slot, and write beside the final name
    let part = part_path(output);
    let args = build_args(&install.models_dir(), input, &part, o)?;
    if o.cancel.is_cancelled() {
        return Err(UpscaleError::Cancelled);
    }
    let _slot = match &o.runtime {
        Some(rt) => Some(rt.begin_tool_blocking("upscale", &|| o.cancel.is_cancelled()).map_err(|e| {
            if o.cancel.is_cancelled() { UpscaleError::Cancelled } else { UpscaleError::Slot(e) }
        })?),
        None => None,
    };
    let result = run_child(&bin, &install.dir, &args, &o.cancel, on_progress);
    match result {
        Ok(()) if part.metadata().map(|m| m.len() > 0).unwrap_or(false) => {
            std::fs::rename(&part, output)?;
            on_progress(100.0);
            Ok(())
        }
        Ok(()) => {
            let _ = std::fs::remove_file(&part);
            Err(UpscaleError::Failed { code: "no output".into(), tail: "沒有產生輸出圖片（顯示卡不支援？）".into() })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn part_path(output: &Path) -> PathBuf {
    let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "out.png".into());
    let ext = output.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    output.with_file_name(format!(".{name}.upscaling.{ext}"))
}

/// Run the CLI, reading its stderr (progress and messages, lines end with `\n` or `\r`).
fn run_child(bin: &Path, cwd: &Path, args: &[OsString], cancel: &CancelToken, on_progress: &mut dyn FnMut(f32)) -> Result<(), UpscaleError> {
    let mut child = Command::new(bin).args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
    let mut err = child.stderr.take().expect("piped");
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let (mut line, mut buf) = (Vec::new(), [0u8; 4096]);
        while let Ok(n) = err.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &b in &buf[..n] {
                if b == b'\n' || b == b'\r' {
                    if !line.is_empty() && tx.send(String::from_utf8_lossy(&line).into_owned()).is_err() {
                        return;
                    }
                    line.clear();
                } else {
                    line.push(b);
                }
            }
        }
        if !line.is_empty() {
            let _ = tx.send(String::from_utf8_lossy(&line).into_owned());
        }
    });
    let mut tail: Vec<String> = Vec::new();
    let mut last = -1.0f32;
    let mut take = |l: String, on: &mut dyn FnMut(f32)| match parse_progress(&l) {
        Some(p) => {
            if p != last {
                last = p;
                on(p);
            }
        }
        None => {
            tail.push(l);
            if tail.len() > 6 {
                tail.remove(0);
            }
        }
    };
    let status = loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            // not joined: a grandchild may still hold the pipe; the reader ends with it
            return Err(UpscaleError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(l) => take(l, on_progress),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(s) = child.try_wait()? {
                    break s;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break child.wait()?,
        }
    };
    let _ = reader.join();
    while let Ok(l) = rx.try_recv() {
        take(l, on_progress);
    }
    if status.success() {
        Ok(())
    } else {
        let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into());
        Err(UpscaleError::Failed { code, tail: tail.join(" | ") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        assert_eq!(parse_progress("97.50%"), Some(97.5));
        assert_eq!(parse_progress("  0.00% \n"), Some(0.0));
        assert_eq!(parse_progress("100%"), Some(100.0));
        assert_eq!(parse_progress("101.0%"), None);
        assert_eq!(parse_progress("-3%"), None);
        assert_eq!(parse_progress("input.jpg -> out.png done"), None);
        assert_eq!(parse_progress("[0 Apple M6]  queueC=0[1]"), None);
        assert_eq!(parse_progress("%"), None);
    }

    #[test]
    fn arguments() {
        let mut o = Options::new(Model::AnimeVideo, 3);
        o.tile = 64;
        let a = build_args(Path::new("/m/models"), Path::new("a.JPG"), Path::new("b.webp"), &o).unwrap();
        let a: Vec<String> = a.iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(a, ["-i", "a.JPG", "-o", "b.webp", "-n", "realesr-animevideov3-x3", "-s", "3", "-t", "64", "-m", "/m/models", "-f", "webp"]);
        let a = build_args(Path::new("m"), Path::new("a.png"), Path::new("b.png"), &Options::new(Model::AnimeX4, 4)).unwrap();
        let a: Vec<String> = a.iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(a.contains(&"realesrgan-x4plus-anime".to_string()) && !a.contains(&"-t".to_string()), "{a:?}");
        let g = Options::new(Model::GeneralX4, 4);
        assert!(build_args(Path::new("m"), Path::new("a.png"), Path::new("b.png"), &g).unwrap().iter().any(|s| s == "realesrgan-x4plus"));
    }

    #[test]
    fn bad_options() {
        let p = Path::new("a.png");
        assert!(matches!(build_args(p, p, p, &Options::new(Model::AnimeX4, 2)), Err(UpscaleError::Options(_))));
        assert!(matches!(build_args(p, p, p, &Options::new(Model::AnimeVideo, 5)), Err(UpscaleError::Options(_))));
        let mut o = Options::new(Model::AnimeX4, 4);
        o.tile = 16;
        assert!(matches!(build_args(p, p, p, &o), Err(UpscaleError::Options(_))));
        assert!(matches!(build_args(p, Path::new("a.gif"), p, &Options::default()), Err(UpscaleError::Options(_))));
    }
}
