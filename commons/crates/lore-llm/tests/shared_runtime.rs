//! The shared local model across app processes (docs/local-models.md §9): real child
//! processes (`lore-llm-testkit client …`) over one registry in a temp folder, and a fake
//! llama-server (the same binary copied as `llama-server`). Never the real cache folder,
//! models or llama-server.

#![cfg(unix)]

use lore_llm::Catalog;
use lore_llm::shared_runtime::{REGISTRY, Registry};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const A: &str = "qwen3.5-4b-q8";
const B: &str = "qwen3.5-9b-q8";
const C: &str = "gemma-4-12b-q8";

struct Env {
    _dir: tempfile::TempDir,
    reg: PathBuf,
    home: PathBuf,
    timing: String,
}

impl Env {
    /// `timing` = `min_residency,hold,promote_after` in seconds.
    fn new(timing: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("models");
        let bin = home.join("runtimes/llama.cpp/llama-server");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        // a copy, not a link: `ps` must report the name llama-server
        std::fs::copy(env!("CARGO_BIN_EXE_lore-llm-testkit"), &bin).unwrap();
        let env = Env { reg: dir.path().join("cache"), home, _dir: dir, timing: timing.into() };
        for id in [A, B, C] {
            env.weights(id, "load_ms=300\n");
        }
        env
    }

    fn weights(&self, id: &str, conf: &str) {
        let cat = Catalog::bundled();
        let m = cat.get(id).unwrap();
        let p = self.home.join("gguf").join(&m.repo).join(&m.file_suffix);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, conf).unwrap();
    }

    fn app(&self, name: &str) -> App {
        self.app_in(name, self.home.to_str().unwrap())
    }

    /// `home` = `-`: an app that does not know the model folder.
    fn app_in(&self, name: &str, home: &str) -> App {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lore-llm-testkit"))
            .args(["client", self.reg.to_str().unwrap(), home, name])
            .env("LORE_TESTKIT_TIMING", &self.timing)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    return;
                }
            }
        });
        App { child, rx }
    }

    fn registry(&self) -> Registry {
        std::fs::read_to_string(self.reg.join(REGISTRY)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn server_pid(&self) -> Option<u32> {
        self.registry().server.map(|s| s.pid)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // never leave a fake server behind
        if let Some(pid) = self.server_pid() {
            kill(pid, "-KILL");
        }
    }
}

struct App {
    child: Child,
    rx: mpsc::Receiver<String>,
}

impl App {
    fn send(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }
    fn read(&mut self) -> String {
        self.rx.recv_timeout(Duration::from_secs(40)).expect("the app answers")
    }
    fn cmd(&mut self, line: &str) -> String {
        self.send(line);
        self.read()
    }
    fn pid(&self) -> u32 {
        self.child.id()
    }
    /// Gone without the quit path.
    fn crash(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    fn quit(mut self) {
        assert_eq!(self.cmd("quit"), "ok bye");
        let _ = self.child.wait();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn kill(pid: u32, sig: &str) {
    let _ = Command::new("kill").args([sig, &pid.to_string()]).status();
}

fn alive(pid: u32) -> bool {
    Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn gone(pid: u32) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Run a command on its own thread: `(when it answered, the answer)`.
fn spawn_cmd(mut app: App, line: &'static str, t0: Instant) -> std::thread::JoinHandle<(Duration, String, App)> {
    std::thread::spawn(move || {
        let r = app.cmd(line);
        (t0.elapsed(), r, app)
    })
}

#[test]
fn two_apps_share_one_server_and_the_last_to_quit_stops_it() {
    let env = Env::new("0,0,120");
    let mut pc = env.app("pc");
    let mut le = env.app("le");
    assert_eq!(pc.cmd(&format!("chat {A} background hi")), format!("ok {A}: hi"));
    let pid = env.server_pid().expect("registered");
    assert_eq!(le.cmd(&format!("chat {A} interactive hello")), format!("ok {A}: hello"));
    let reg = env.registry();
    assert_eq!(env.server_pid(), Some(pid), "one server for both");
    assert_eq!(reg.leases.len(), 2);
    assert_eq!(reg.server.as_ref().unwrap().started_by.app, "pc");
    assert!(reg.wanting.is_empty());
    assert!(le.cmd("status").starts_with("ok 已載入 Qwen3.5 4B"));
    assert_eq!(le.cmd("release"), "err PaperComposer 正在用", "釋放 only when nobody else uses it");

    // the starter quits: the server stays for the other app
    pc.quit();
    assert!(alive(pid));
    assert_eq!(le.cmd(&format!("chat {A} interactive again")), format!("ok {A}: again"));
    assert_eq!(env.server_pid(), Some(pid));
    // the last one quits: the server stops
    le.quit();
    assert!(gone(pid), "the last user stops the server");
    assert!(env.registry().server.is_none() && env.registry().leases.is_empty());
}

/// An app launched without the model folder (no LOCAL_LLM_HOME) uses a server another app
/// started, and says why when it would have to start one itself.
#[test]
fn an_app_without_the_model_folder_uses_but_never_starts() {
    let env = Env::new("0,0,120");
    let mut le = env.app_in("le", "-");
    let r = le.cmd(&format!("chat {A} interactive hi"));
    assert!(r.starts_with("err") && r.contains("LOCAL_LLM_HOME"), "{r}");
    assert!(env.registry().server.is_none() && env.registry().wanting.is_empty());
    let mut pc = env.app("pc");
    assert!(pc.cmd(&format!("chat {A} interactive hi")).starts_with("ok"));
    assert_eq!(le.cmd(&format!("chat {A} interactive again")), format!("ok {A}: again"));
    assert_eq!(env.registry().leases.len(), 2);
}

#[test]
fn a_crashed_starter_leaves_the_server_to_the_others() {
    let env = Env::new("0,0,120");
    let mut pc = env.app("pc");
    let mut sc = env.app("sc");
    assert!(pc.cmd(&format!("chat {A} background hi")).starts_with("ok"));
    let pid = env.server_pid().unwrap();
    let pc_pid = pc.pid();
    pc.crash();
    assert_eq!(sc.cmd(&format!("chat {A} interactive x")), format!("ok {A}: x"));
    assert_eq!(env.server_pid(), Some(pid), "not restarted");
    assert!(!env.registry().leases.iter().any(|l| l.pid == pc_pid), "the dead app's lease was pruned");
    sc.quit();
    assert!(gone(pid));
}

#[test]
fn a_switch_waits_until_the_running_request_is_done() {
    let env = Env::new("0,0,120");
    let t0 = Instant::now();
    let mut pc = env.app("pc");
    assert!(pc.cmd(&format!("chat {A} background warm")).starts_with("ok"));
    let first = env.server_pid().unwrap();
    let long = spawn_cmd(pc, "chat qwen3.5-4b-q8 background sleep:2500", t0);
    std::thread::sleep(Duration::from_millis(600));
    let mut lr = env.app("lr");
    lr.send(&format!("chat {B} interactive now"));
    std::thread::sleep(Duration::from_millis(600));
    let reg = env.registry();
    assert!(reg.wanting.iter().any(|w| w.pid == lr.pid() && w.model == B), "waiting in line: {reg:?}");
    assert_eq!(reg.server.as_ref().unwrap().model, A, "not switched while a request runs");
    let switched = lr.read();
    let at = t0.elapsed();
    let (done_a, ra, _pc) = long.join().unwrap();
    assert_eq!(ra, format!("ok {A}: sleep:2500"), "the running request was not cut");
    assert_eq!(switched, format!("ok {B}: now"));
    assert!(at >= done_a, "the switch came after the running request ({at:?} < {done_a:?})");
    assert!(gone(first), "the old server was stopped");
    let reg = env.registry();
    assert_eq!(reg.server.as_ref().map(|s| (s.model.as_str(), s.from.as_deref())), Some((B, Some(A))));
    assert!(reg.wanting.is_empty());
}

/// Interactive before background, whatever came first; a background wanter also waits out
/// the hold after the last use.
#[test]
fn interactive_goes_first_and_background_waits_out_the_hold() {
    let env = Env::new("0,2,120");
    let t0 = Instant::now();
    let mut pc = env.app("pc");
    assert!(pc.cmd(&format!("acquire {A} interactive")).starts_with("ok"), "a request held open");
    let bg = spawn_cmd(env.app("sc"), "chat qwen3.5-9b-q8 background b", t0);
    std::thread::sleep(Duration::from_millis(500));
    let fg = spawn_cmd(env.app("le"), "chat gemma-4-12b-q8 interactive c", t0);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(env.registry().wanting.len(), 2);
    assert_eq!(pc.cmd("finish"), "ok");
    let (t_fg, r_fg, _le) = fg.join().unwrap();
    let (t_bg, r_bg, _sc) = bg.join().unwrap();
    assert_eq!(r_fg, format!("ok {C}: c"));
    assert_eq!(r_bg, format!("ok {B}: b"));
    assert!(t_fg < t_bg, "interactive first: {t_fg:?} vs {t_bg:?}");
    assert!(t_bg - t_fg >= Duration::from_millis(900), "the background switch waited for the hold: {:?}", t_bg - t_fg);
}

/// A background wanter waiting longer than `promote_after` counts as interactive, so it is
/// no longer overtaken by a later interactive one.
#[test]
fn a_long_waiting_background_request_is_promoted() {
    let env = Env::new("0,0,1");
    let t0 = Instant::now();
    let mut pc = env.app("pc");
    assert!(pc.cmd(&format!("acquire {A} interactive")).starts_with("ok"));
    let bg = spawn_cmd(env.app("sc"), "chat qwen3.5-9b-q8 background b", t0);
    std::thread::sleep(Duration::from_millis(1500));
    let fg = spawn_cmd(env.app("le"), "chat gemma-4-12b-q8 interactive c", t0);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(pc.cmd("finish"), "ok");
    let (t_bg, r_bg, _sc) = bg.join().unwrap();
    let (t_fg, r_fg, _le) = fg.join().unwrap();
    assert!(r_bg.starts_with("ok") && r_fg.starts_with("ok"), "{r_bg} / {r_fg}");
    assert!(t_bg < t_fg, "promoted and older: {t_bg:?} vs {t_fg:?}");
}

#[test]
fn a_model_stays_for_the_minimum_residency() {
    let env = Env::new("3,0,120");
    let mut pc = env.app("pc");
    assert!(pc.cmd(&format!("chat {A} interactive hi")).starts_with("ok"));
    let since = env.registry().server.unwrap().since;
    let mut le = env.app("le");
    assert_eq!(le.cmd(&format!("chat {B} interactive x")), format!("ok {B}: x"));
    let reg = env.registry();
    assert!(reg.server.as_ref().unwrap().since >= since + 3, "switched only after 3 s: {} → {}", since, reg.server.unwrap().since);
}

#[test]
fn any_app_releases_an_idle_server_also_an_orphan() {
    let env = Env::new("0,0,120");
    let mut pc = env.app("pc");
    let mut kb = env.app("kb");
    assert!(pc.cmd(&format!("chat {A} interactive hi")).starts_with("ok"));
    let pid = env.server_pid().unwrap();
    assert_eq!(kb.cmd("idle 2"), "ok false", "just used");
    std::thread::sleep(Duration::from_millis(3100));
    assert_eq!(kb.cmd("idle 2"), "ok true", "an app that never used it releases it");
    assert!(gone(pid));
    assert_eq!(kb.cmd("status"), "ok 未載入");

    // every user crashed: the next app's check stops the orphan
    assert!(pc.cmd(&format!("chat {A} interactive again")).starts_with("ok"));
    let pid = env.server_pid().unwrap();
    pc.crash();
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(kb.cmd("idle 1"), "ok true");
    assert!(gone(pid));
}

#[test]
fn a_dead_lock_holder_releases_the_lock() {
    let env = Env::new("0,0,120");
    let mut holder = env.app("pc");
    assert_eq!(holder.cmd("lock 60000"), "ok locked");
    let mut le = env.app("le");
    le.send(&format!("chat {A} interactive hi"));
    std::thread::sleep(Duration::from_millis(500));
    holder.crash();
    assert_eq!(le.read(), format!("ok {A}: hi"));
}

#[test]
fn stale_pids_are_pruned_and_an_unverified_pid_is_never_signalled() {
    let env = Env::new("0,0,120");
    let mut dead = Command::new("true").spawn().unwrap();
    let dead_pid = dead.id();
    dead.wait().unwrap();
    // a "server" whose pid is this test process (not llama-server): only forgotten
    let me = std::process::id();
    std::fs::create_dir_all(&env.reg).unwrap();
    let fake = serde_json::json!({
        "server": { "model": A, "url": "http://127.0.0.1:9/v1", "pid": me, "started_by": { "app": "pc", "pid": dead_pid },
                    "since": 1, "started_at": "Thu Jan  1 00:00:00 1970", "vision": false },
        "leases": [ { "app": "pc", "pid": dead_pid, "since": 1, "last_used": 1, "priority": "background", "active": 1 } ],
        "wanting": [ { "app": "sc", "pid": dead_pid, "model": B, "priority": "interactive", "since": 1 } ],
        "last_activity": 1
    });
    std::fs::write(env.reg.join(REGISTRY), fake.to_string()).unwrap();
    let mut kb = env.app("kb");
    assert_eq!(kb.cmd("status"), "ok 未載入");
    let reg = env.registry();
    assert!(reg.server.is_none() && reg.leases.is_empty() && reg.wanting.is_empty(), "{reg:?}");
    assert!(alive(me));
    assert_eq!(kb.cmd(&format!("chat {A} interactive ok")), format!("ok {A}: ok"));
}

#[test]
fn a_broken_registry_finds_the_running_server_again() {
    let env = Env::new("0,0,120");
    let mut pc = env.app("pc");
    assert!(pc.cmd(&format!("chat {A} interactive hi")).starts_with("ok"));
    let s = env.registry().server.unwrap();
    std::fs::write(env.reg.join(REGISTRY), format!("{{ \"server\": {{ \"url\": \"{}\", broken", s.url)).unwrap();
    let mut le = env.app("le");
    assert_eq!(le.cmd("status"), format!("ok 已載入 Qwen3.5 4B"));
    let back = env.registry().server.unwrap();
    assert_eq!((back.pid, back.model.as_str(), back.url.as_str()), (s.pid, A, s.url.as_str()));
    assert_eq!(le.cmd(&format!("chat {A} interactive x")), format!("ok {A}: x"));
    assert_eq!(env.server_pid(), Some(s.pid), "the same server, not a second one");
}

#[test]
fn a_server_that_died_is_started_again_by_the_next_request() {
    let env = Env::new("0,0,120");
    let mut pc = env.app("pc");
    assert!(pc.cmd(&format!("chat {A} interactive hi")).starts_with("ok"));
    let pid = env.server_pid().unwrap();
    kill(pid, "-KILL");
    assert!(gone(pid));
    assert_eq!(pc.cmd(&format!("chat {A} interactive again")), format!("ok {A}: again"));
    let new = env.server_pid().unwrap();
    assert_ne!(new, pid);
}

#[test]
fn a_server_that_fails_to_start_says_why() {
    let env = Env::new("0,0,120");
    env.weights(A, "fail=error: out of memory\n");
    let mut pc = env.app("pc");
    let r = pc.cmd(&format!("chat {A} interactive hi"));
    assert!(r.starts_with("err") && r.contains("out of memory"), "{r}");
    let reg = env.registry();
    assert!(reg.server.is_none() && reg.wanting.is_empty(), "{reg:?}");
    assert!(Path::new(&env.reg.join(lore_llm::shared_runtime::LOG)).exists());
}
