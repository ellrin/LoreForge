//! Fake CLI (a shell script), a local fake download server, temp folders only. The real
//! binary: the `#[ignore]`d test at the end (`REALESRGAN_DIR`).

#![cfg(unix)]

use lore_llm::shared_runtime::{SharedRuntime, Timing};
use lore_upscale::{CancelToken, Install, Model, Options, Progress, Source, UpscaleError, download, upscale, upscale_with};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const FAKE_CLI: &str = r#"#!/bin/sh
while [ $# -gt 0 ]; do case "$1" in -i) i="$2";; -o) o="$2";; esac; shift; done
if grep -q FAIL "$i"; then echo "vkCreateInstance failed" >&2; exit 3; fi
if grep -q NOOUT "$i"; then exit 0; fi
echo "[0 Fake]  queueC=0[1]" >&2
for p in 0.00 50.00 97.50; do printf '%s%%\n' $p >&2; if grep -q SLOW "$i"; then sleep 3; fi; done
cp "$i" "$o"
"#;

const MODELS: [&str; 5] = ["realesrgan-x4plus-anime", "realesrgan-x4plus", "realesr-animevideov3-x2", "realesr-animevideov3-x3", "realesr-animevideov3-x4"];

fn fake_zip() -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("realesrgan-ncnn-vulkan", o).unwrap();
    z.write_all(FAKE_CLI.as_bytes()).unwrap();
    z.add_directory("models/", o).unwrap();
    for m in MODELS {
        for ext in ["param", "bin"] {
            z.start_file(format!("models/{m}.{ext}"), o).unwrap();
            z.write_all(b"x").unwrap();
        }
    }
    for extra in ["input.jpg", "onepiece_demo.mp4", "../evil.txt", "models/sub/deep.bin"] {
        z.start_file(extra, o).unwrap();
        z.write_all(b"no").unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

async fn serve(body: Vec<u8>) -> String {
    let app = axum::Router::new().route("/r.zip", axum::routing::get(move || { let b = body.clone(); async move { b } }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/r.zip", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    url
}

async fn installed(dir: &Path) -> Install {
    let zip = fake_zip();
    let src = Source { url: serve(zip.clone()).await, sha256: sha(&zip) };
    let inst = Install::in_home(dir);
    download(&inst, &src, &CancelToken::new(), |_| {}).await.unwrap();
    inst
}

#[test]
fn missing_binary_is_a_clear_error() {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("a.png"), "img").unwrap();
    let mut o = Options::new(Model::AnimeX4, 4);
    o.install = Some(Install::in_home(t.path()));
    match upscale(&t.path().join("a.png"), &t.path().join("b.png"), &o) {
        Err(UpscaleError::NotInstalled { dir, missing }) => {
            assert!(dir.ends_with("runtimes/realesrgan"));
            assert!(missing.contains("realesrgan-ncnn-vulkan") && missing.contains("realesrgan-x4plus-anime.param"), "{missing}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!t.path().join("b.png").exists());
}

#[tokio::test]
async fn download_verifies_unpacks_and_records() {
    let t = tempfile::tempdir().unwrap();
    let zip = fake_zip();
    let url = serve(zip.clone()).await;
    let inst = Install::in_home(t.path());
    let seen = Mutex::new(Vec::new());
    download(&inst, &Source { url: url.clone(), sha256: sha(&zip).to_uppercase() }, &CancelToken::new(), |p| seen.lock().unwrap().push(p)).await.unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(matches!(seen.first(), Some(Progress::Downloading { done: 0, total: Some(_) })));
    assert!(seen.contains(&Progress::Verifying) && seen.last() == Some(&Progress::Installing));
    assert!(inst.is_installed());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(inst.binary().metadata().unwrap().permissions().mode() & 0o111, 0o111, "executable");
    for f in ["LICENSE-Real-ESRGAN.txt", "LICENSE-Real-ESRGAN-ncnn-vulkan.txt", "install.json"] {
        assert!(inst.dir.join(f).is_file(), "{f}");
    }
    let lic = std::fs::read_to_string(inst.dir.join("LICENSE-Real-ESRGAN.txt")).unwrap();
    assert!(lic.contains("BSD 3-Clause"));
    // only the binary and models: no demo files, nothing outside the folder, no staging left
    assert!(!inst.dir.join("input.jpg").exists() && !inst.dir.join("onepiece_demo.mp4").exists());
    assert!(!t.path().join("runtimes/evil.txt").exists() && !t.path().join("evil.txt").exists());
    assert!(!inst.dir.join("models/sub").exists());
    let left: Vec<_> = std::fs::read_dir(t.path().join("runtimes")).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left.len(), 1, "{left:?}");
    assert!(std::fs::read_to_string(inst.dir.join("install.json")).unwrap().contains(&url));
}

#[tokio::test]
async fn wrong_checksum_installs_nothing_and_keeps_the_old() {
    let t = tempfile::tempdir().unwrap();
    let zip = fake_zip();
    let url = serve(zip.clone()).await;
    let inst = Install::in_home(t.path());
    let bad = Source { url: url.clone(), sha256: "0".repeat(64) };
    match download(&inst, &bad, &CancelToken::new(), |_| {}).await {
        Err(UpscaleError::Checksum { expected, actual }) => assert!(expected == "0".repeat(64) && actual == sha(&zip)),
        other => panic!("{other:?}"),
    }
    assert!(!inst.dir.exists());
    assert_eq!(std::fs::read_dir(t.path().join("runtimes")).unwrap().count(), 0, "staging removed");
    // an existing install survives a failed re-download
    let good = Source { url, sha256: sha(&zip) };
    download(&inst, &good, &CancelToken::new(), |_| {}).await.unwrap();
    assert!(download(&inst, &bad, &CancelToken::new(), |_| {}).await.is_err());
    assert!(inst.is_installed());
    // a server that is not there
    let gone = Source { url: "http://127.0.0.1:1/x.zip".into(), sha256: sha(&zip) };
    assert!(matches!(download(&inst, &gone, &CancelToken::new(), |_| {}).await, Err(UpscaleError::Download(_))));
}

#[tokio::test]
async fn upscale_reports_progress_and_writes_the_output() {
    let t = tempfile::tempdir().unwrap();
    let inst = installed(t.path()).await;
    let (input, output) = (t.path().join("in.png"), t.path().join("out.png"));
    std::fs::write(&input, "pixels").unwrap();
    std::fs::write(&output, "old").unwrap();
    let mut o = Options::new(Model::AnimeVideo, 2);
    o.install = Some(inst);
    let got = tokio::task::spawn_blocking(move || {
        let mut p = Vec::new();
        upscale_with(&input, &output, &o, &mut |x| p.push(x)).map(|_| p)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(got, [0.0, 50.0, 97.5, 100.0]);
    assert_eq!(std::fs::read_to_string(t.path().join("out.png")).unwrap(), "pixels", "replaced on success");
    let names: Vec<_> = std::fs::read_dir(t.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(!names.iter().any(|n| n.contains("upscaling")), "{names:?}");
}

#[tokio::test]
async fn failures_keep_the_old_output_and_say_why() {
    let t = tempfile::tempdir().unwrap();
    let inst = installed(t.path()).await;
    let mut o = Options::new(Model::AnimeX4, 4);
    o.install = Some(inst);
    let (input, output) = (t.path().join("in.png"), t.path().join("out.png"));
    std::fs::write(&output, "old").unwrap();
    std::fs::write(&input, "FAIL").unwrap();
    let (i2, o2, o3) = (input.clone(), output.clone(), o.clone());
    match tokio::task::spawn_blocking(move || upscale(&i2, &o2, &o3)).await.unwrap() {
        Err(UpscaleError::Failed { code, tail }) => assert!(code == "3" && tail.contains("vkCreateInstance"), "{code} {tail}"),
        other => panic!("{other:?}"),
    }
    std::fs::write(&input, "NOOUT").unwrap();
    assert!(matches!(upscale(&input, &output, &o), Err(UpscaleError::Failed { .. })));
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "old");
    assert!(matches!(upscale(&t.path().join("none.png"), &output, &o), Err(UpscaleError::NoInput(_))));
}

#[tokio::test]
async fn cancel_kills_the_child() {
    let t = tempfile::tempdir().unwrap();
    let inst = installed(t.path()).await;
    let (input, output) = (t.path().join("in.png"), t.path().join("out.png"));
    std::fs::write(&input, "SLOW").unwrap();
    let mut o = Options::new(Model::AnimeX4, 4);
    o.install = Some(inst);
    let cancel = o.cancel.clone();
    let start = Instant::now();
    let h = tokio::task::spawn_blocking(move || upscale(&input, &output, &o));
    tokio::time::sleep(Duration::from_millis(600)).await;
    cancel.cancel();
    assert!(matches!(h.await.unwrap(), Err(UpscaleError::Cancelled)));
    assert!(start.elapsed() < Duration::from_secs(3), "did not wait for the child");
    assert!(!t.path().join("out.png").exists());
}

#[tokio::test]
async fn the_run_takes_the_runtimes_slot() {
    let t = tempfile::tempdir().unwrap();
    let inst = installed(t.path()).await;
    let (input, output) = (t.path().join("in.png"), t.path().join("out.png"));
    std::fs::write(&input, "SLOW").unwrap();
    let rt = SharedRuntime::new("sc", t.path().join("cache"), Timing { poll_min: Duration::from_millis(20), ..Timing::default() });
    let mut o = Options::new(Model::AnimeX4, 4);
    o.install = Some(inst);
    o.runtime = Some(rt.clone());
    let cancel = o.cancel.clone();
    let h = tokio::task::spawn_blocking(move || upscale(&input, &output, &o));
    tokio::time::sleep(Duration::from_millis(800)).await;
    let during = rt.snapshot_blocking().registry.tools;
    assert_eq!(during.len(), 1);
    assert_eq!((during[0].app.as_str(), during[0].tool.as_str()), ("sc", "upscale"));
    cancel.cancel();
    let _ = h.await.unwrap();
    assert!(rt.snapshot_blocking().registry.tools.is_empty(), "slot released");
}

/// The real binary: `REALESRGAN_DIR=<folder with realesrgan-ncnn-vulkan and models/>`.
#[test]
#[ignore = "needs the real Real-ESRGAN binary, models and a GPU (REALESRGAN_DIR)"]
fn real_binary() {
    let dir = std::env::var("REALESRGAN_DIR").expect("REALESRGAN_DIR");
    let t = tempfile::tempdir().unwrap();
    let input = Path::new(&dir).join("input.jpg");
    let input = if input.is_file() { input } else { std::env::var("REALESRGAN_INPUT").expect("REALESRGAN_INPUT").into() };
    let output = t.path().join("out.png");
    let mut o = Options::new(Model::AnimeVideo, 2);
    o.install = Some(Install::from_dir(dir));
    let mut last = 0.0;
    upscale_with(&input, &output, &o, &mut |p| last = p).unwrap();
    assert_eq!(last, 100.0);
    assert!(output.metadata().unwrap().len() > 1000);
}
