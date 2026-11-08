//! Recall reminders: stats, notification slots (NeonDeck claim + osascript fallback), the daily
//! digest over a mock SMTP server. Offline: no real mail, no real notification, no keychain.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use kb_core::{FixedClock, KbOptions};
use kb_server::reminders::{self, Reminders, applescript_str};
use kb_server::{AppState, Config, Shared};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tower::ServiceExt;
use ym_store::{CommandOutput, CommandRunner};

#[derive(Default)]
struct FakeRunner {
    calls: Mutex<Vec<Vec<String>>>,
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CommandOutput> {
        let mut v = vec![program.to_string()];
        v.extend(args.iter().map(|s| s.to_string()));
        self.calls.lock().unwrap().push(v);
        Ok(CommandOutput { status: 0, stdout: String::new(), stderr: String::new() })
    }
}

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    shared: Shared,
    app: axum::Router,
    clock: Arc<FixedClock>,
    runner: Arc<FakeRunner>,
}

fn space(root: &Path) -> PathBuf {
    let p = root.join("personal");
    if !p.join("space.toml").is_file() {
        std::fs::create_dir_all(p.join("nodes")).unwrap();
        std::fs::write(
            p.join("space.toml"),
            format!("id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n[assets]\nbackend = \"icloud\"\nroot = \"{}\"\n", root.join("icloud").display()),
        )
        .unwrap();
    }
    p
}

/// A server over `root` (re-opening the same root = a restart).
fn open(root: &Path, clock: Arc<FixedClock>, runner: Arc<FakeRunner>) -> (Shared, axum::Router) {
    let config = Config { spaces: vec![space(root)], member: "ellrin".into(), port: 0, web: root.join("web"), intervals: Default::default(), app_dir: Some(root.join("app")) };
    let kb = kb_server::open(&config, KbOptions { clock, ..Default::default() }).unwrap();
    let rem = Reminders::isolated(&root.join("app"), runner, "http://127.0.0.1:7700");
    let shared = AppState::new(kb, config.web.clone(), rem);
    (shared.clone(), kb_server::router(shared))
}

fn setup(at: &str) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let clock = Arc::new(FixedClock::at(at));
    let runner = Arc::new(FakeRunner::default());
    let (shared, app) = open(&root, clock.clone(), runner.clone());
    Env { _dir: dir, root, shared, app, clock, runner }
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn card(app: &axum::Router, title: &str, summary: &str) -> String {
    let (s, v) = call(app, "POST", "/api/nodes", Some(json!({ "space": "me", "title": title, "summary": summary, "aliases": [format!("{title}-alias")] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["node"]["id"].as_str().unwrap().to_string()
}

fn set(clock: &FixedClock, at: &str) {
    clock.set(chrono::DateTime::parse_from_rfc3339(at).unwrap());
}

#[tokio::test]
async fn stats_and_notification_slots() {
    let e = setup("2026-11-08T08:00:00+08:00");
    let id = card(&e.app, "Dice coefficient", "衡量兩個集合重疊程度的指標").await;
    set(&e.clock, "2026-11-09T08:30:00+08:00");
    card(&e.app, "IoU", "交集除以聯集").await;

    let (s, v) = call(&e.app, "GET", "/api/stats", None).await;
    assert_eq!(s, StatusCode::OK);
    let st = &v["stats"];
    assert_eq!((st["new_today"].as_u64(), st["due"].as_u64(), st["total"].as_u64(), st["learning"].as_u64()), (Some(1), Some(1), Some(2), Some(2)));
    assert_eq!(v["notify"]["times"], json!(["09:00", "21:00"]));

    // Before the first slot: nothing to claim.
    let (_, v) = call(&e.app, "POST", "/api/reminders/claim", Some(json!({ "by": "neondeck" }))).await;
    assert_eq!((v["slot"].clone(), v["claimed"].clone()), (Value::Null, json!(false)));

    // 09:01: still inside the grace period, kb-server waits for NeonDeck.
    set(&e.clock, "2026-11-09T09:01:00+08:00");
    let r = reminders::tick(&e.shared).await;
    assert!(!r.notified && e.runner.calls.lock().unwrap().is_empty());
    let (_, v) = call(&e.app, "POST", "/api/reminders/claim", Some(json!({ "by": "neondeck" }))).await;
    assert_eq!(v["claimed"], true, "{v}");
    assert_eq!(v["slot"], "2026-11-09T09:00");
    assert_eq!(v["due"], 1);
    assert!(v["body"].as_str().unwrap().contains("Dice coefficient"));
    let (_, v) = call(&e.app, "POST", "/api/reminders/claim", None).await;
    assert_eq!(v["claimed"], false, "a slot is claimed once");
    set(&e.clock, "2026-11-09T09:05:00+08:00");
    assert!(!reminders::tick(&e.shared).await.notified, "NeonDeck already showed it");

    // 21:03 without NeonDeck: kb-server shows it, once.
    set(&e.clock, "2026-11-09T21:03:00+08:00");
    assert!(reminders::tick(&e.shared).await.notified);
    assert!(!reminders::tick(&e.shared).await.notified);
    {
        let calls = e.runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "osascript");
        assert!(calls[0][2].starts_with("display notification \"") && calls[0][2].contains("with title \"記得嗎？"), "{:?}", calls[0]);
    }
    let (_, v) = call(&e.app, "GET", "/api/reminders", None).await;
    assert_eq!(v["state"]["notify_last_by"], "kb-server");

    // The Mac slept through the next morning: one catch-up notification at wake (15:00).
    set(&e.clock, "2026-11-10T15:00:00+08:00");
    assert!(reminders::tick(&e.shared).await.notified);
    assert!(!reminders::tick(&e.shared).await.notified);

    // Known within the week, and the fallback can be turned off.
    let (_, v) = call(&e.app, "POST", &format!("/api/recall/{id}"), Some(json!({ "outcome": "remembered", "via": "neondeck" }))).await;
    assert_eq!(v["progress"]["streak"], 1);
    set(&e.clock, "2026-11-13T21:30:00+08:00");
    let (_, v) = call(&e.app, "POST", &format!("/api/recall/{id}"), Some(json!({ "outcome": "remembered" }))).await;
    assert_eq!(v["progress"]["state"], "known");
    let (_, v) = call(&e.app, "GET", "/api/stats", None).await;
    assert_eq!(v["stats"]["known_week"], 1);
    assert_eq!(v["stats"]["known"], 1);

    let (_, mut cfg) = call(&e.app, "GET", "/api/reminders", None).await;
    cfg["settings"]["notify"]["enabled"] = json!(false);
    let (s, _) = call(&e.app, "PUT", "/api/reminders", Some(cfg["settings"].clone())).await;
    assert_eq!(s, StatusCode::OK);
    set(&e.clock, "2026-11-14T21:30:00+08:00");
    let n = e.runner.calls.lock().unwrap().len();
    reminders::tick(&e.shared).await;
    assert_eq!(e.runner.calls.lock().unwrap().len(), n);

    // Test notification goes through the same runner.
    let (s, _) = call(&e.app, "POST", "/api/reminders/test-notify", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(e.runner.calls.lock().unwrap().len(), n + 1);
}

#[test]
fn applescript_quoting() {
    assert_eq!(applescript_str("a \"b\" \\ c\nd"), "\"a \\\"b\\\" \\\\ c d\"");
}

// ── Mock SMTP ───────────────────────────────────────────

#[derive(Clone, Default)]
struct Smtp {
    mails: Arc<Mutex<Vec<String>>>,
    auths: Arc<Mutex<Vec<String>>>,
}

async fn mock_smtp() -> (u16, Smtp) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let smtp = Smtp::default();
    let s2 = smtp.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = l.accept().await {
            let s3 = s2.clone();
            tokio::spawn(async move {
                let _ = session(sock, s3).await;
            });
        }
    });
    (port, smtp)
}

async fn session(sock: tokio::net::TcpStream, smtp: Smtp) -> std::io::Result<()> {
    let (r, mut w) = sock.into_split();
    let mut r = BufReader::new(r);
    w.write_all(b"220 mock ESMTP\r\n").await?;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let cmd = line.trim_end().to_string();
        let up = cmd.to_ascii_uppercase();
        if up.starts_with("EHLO") || up.starts_with("HELO") {
            w.write_all(b"250-mock\r\n250-AUTH PLAIN LOGIN\r\n250-8BITMIME\r\n250 SMTPUTF8\r\n").await?;
        } else if up.starts_with("AUTH PLAIN") {
            let mut arg = cmd[10..].trim().to_string();
            if arg.is_empty() {
                w.write_all(b"334 \r\n").await?;
                line.clear();
                r.read_line(&mut line).await?;
                arg = line.trim().to_string();
            }
            smtp.auths.lock().unwrap().push(arg);
            w.write_all(b"235 2.7.0 ok\r\n").await?;
        } else if up.starts_with("DATA") {
            w.write_all(b"354 go ahead\r\n").await?;
            let mut data = String::new();
            loop {
                line.clear();
                if r.read_line(&mut line).await? == 0 {
                    return Ok(());
                }
                if line == ".\r\n" {
                    break;
                }
                data.push_str(&line);
            }
            smtp.mails.lock().unwrap().push(data);
            w.write_all(b"250 2.0.0 queued\r\n").await?;
        } else if up.starts_with("QUIT") {
            w.write_all(b"221 bye\r\n").await?;
            return Ok(());
        } else {
            w.write_all(b"250 ok\r\n").await?;
        }
    }
}

fn email_settings(port: u16) -> Value {
    json!({
        "email": { "enabled": true, "smtp_host": "127.0.0.1", "smtp_port": port, "security": "none",
                   "user": "ellrin@example.com", "to": ["ellrin@example.com"], "send_at": "07:30" },
        "notify": { "enabled": false }
    })
}

#[tokio::test]
async fn daily_digest_over_mock_smtp() {
    let (port, smtp) = mock_smtp().await;
    let e = setup("2026-11-08T06:00:00+08:00");
    let id = card(&e.app, "Dice coefficient", "衡量兩個集合重疊程度的指標").await;
    set(&e.clock, "2026-11-09T06:50:00+08:00");
    card(&e.app, "IoU", "交集除以聯集").await;
    set(&e.clock, "2026-11-09T07:00:00+08:00");

    // Validation.
    let mut bad = email_settings(port);
    bad["email"]["send_at"] = json!("7點半");
    let (s, v) = call(&e.app, "PUT", "/api/reminders", Some(bad)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let mut bad = email_settings(port);
    bad["email"]["to"] = json!(["not an address"]);
    assert_eq!(call(&e.app, "PUT", "/api/reminders", Some(bad)).await.0, StatusCode::BAD_REQUEST);
    let mut bad = email_settings(port);
    bad["email"]["user"] = json!("");
    let (s, v) = call(&e.app, "PUT", "/api/reminders", Some(bad)).await;
    assert_eq!((s, v["error"].as_str().map(|m| m.starts_with("請填 SMTP 帳號"))), (StatusCode::BAD_REQUEST, Some(true)), "{v}");

    let (s, v) = call(&e.app, "PUT", "/api/reminders", Some(email_settings(port))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["password"]["set"], false);
    assert!(e.root.join("app/reminders.toml").is_file());

    // No password yet: a send fails with a readable error.
    let (s, v) = call(&e.app, "POST", "/api/reminders/test-email", None).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert!(v["error"].as_str().unwrap().contains("SMTP 密碼"), "{v}");

    // Tests never touch the keychain: the password goes to the owner-only file fallback.
    let (s, v) = call(&e.app, "PUT", "/api/reminders/password", Some(json!({ "password": "abcd efgh ijkl mnop" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The answer names the store actually used, so the UI does not claim "keychain".
    assert_eq!((v["saved"].as_str(), v["password"]["source"].as_str()), (Some("file"), Some("file")), "{v}");
    let (_, v) = call(&e.app, "GET", "/api/reminders", None).await;
    assert_eq!((v["password"]["set"].clone(), v["password"]["source"].clone()), (json!(true), json!("file")));
    assert!(!v.to_string().contains("abcd efgh"), "the password is never returned");

    // Preview: questions first, answers at the bottom.
    let (_, v) = call(&e.app, "GET", "/api/reminders/preview?format=json", None).await;
    let text = v["text"].as_str().unwrap();
    let q = text.find("還記得「Dice coefficient」嗎？").expect(text);
    let a = text.find("── 答案 ──").expect(text);
    let ans = text.find("衡量兩個集合").expect(text);
    assert!(q < a && a < ans, "{text}");
    assert!(text.contains(&format!("http://127.0.0.1:7700/#/node/{id}")));
    assert!(text.contains("• IoU — 交集除以聯集"), "recent cards: {text}");
    assert_eq!(v["due"], 1);
    assert!(v["html"].as_str().unwrap().contains("答案（先想再看）"));

    // 07:00: not yet. 07:31: sent once, even across a restart.
    assert_eq!(reminders::tick(&e.shared).await.email, None);
    set(&e.clock, "2026-11-09T07:31:00+08:00");
    let r = reminders::tick(&e.shared).await;
    assert_eq!(r.email.as_deref(), Some("sent"), "{r:?}");
    assert_eq!(smtp.mails.lock().unwrap().len(), 1);
    {
        let mail = smtp.mails.lock().unwrap()[0].clone();
        assert!(mail.contains("multipart/alternative") && mail.contains("text/html") && mail.contains("text/plain"), "{mail}");
        assert!(mail.contains("Subject: "), "{mail}");
        let auth = smtp.auths.lock().unwrap()[0].clone();
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD.decode(auth).unwrap();
        assert_eq!(raw, b"\0ellrin@example.com\0abcd efgh ijkl mnop");
    }
    set(&e.clock, "2026-11-09T07:45:00+08:00");
    assert_eq!(reminders::tick(&e.shared).await.email, None);
    let (shared2, _) = open(&e.root, e.clock.clone(), e.runner.clone());
    assert_eq!(reminders::tick(&shared2).await.email, None, "persisted: never twice a day");
    assert_eq!(smtp.mails.lock().unwrap().len(), 1);

    // Slept through the next morning: one catch-up at wake.
    set(&e.clock, "2026-11-10T13:00:00+08:00");
    assert_eq!(reminders::tick(&shared2).await.email.as_deref(), Some("sent"));
    assert_eq!(reminders::tick(&e.shared).await.email, None);
    assert_eq!(smtp.mails.lock().unwrap().len(), 2);

    // Test send does not count as the daily mail.
    let (s, v) = call(&e.app, "POST", "/api/reminders/test-email", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(smtp.mails.lock().unwrap().len(), 3);
    let (_, v) = call(&e.app, "GET", "/api/reminders", None).await;
    assert_eq!(v["state"]["email_last_sent"], "2026-11-10");

    // A failing server: retried after 15 minutes, at most 3 times a day.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    call(&e.app, "PUT", "/api/reminders", Some(email_settings(closed))).await;
    set(&e.clock, "2026-11-11T07:31:00+08:00");
    assert_eq!(reminders::tick(&e.shared).await.email.as_deref(), Some("failed"));
    set(&e.clock, "2026-11-11T07:40:00+08:00");
    assert_eq!(reminders::tick(&e.shared).await.email, None);
    for t in ["2026-11-11T07:47:00+08:00", "2026-11-11T08:03:00+08:00"] {
        set(&e.clock, t);
        assert_eq!(reminders::tick(&e.shared).await.email.as_deref(), Some("failed"));
    }
    set(&e.clock, "2026-11-11T09:00:00+08:00");
    assert_eq!(reminders::tick(&e.shared).await.email, None, "three attempts a day");
    let (_, v) = call(&e.app, "GET", "/api/reminders", None).await;
    assert_eq!(v["state"]["email_attempts"], 3);
    assert!(v["state"]["email_last_error"].as_str().unwrap().contains("拒絕連線"), "{v}");
}

#[tokio::test]
async fn empty_day_is_skipped() {
    let (port, smtp) = mock_smtp().await;
    let e = setup("2026-11-09T08:00:00+08:00");
    call(&e.app, "PUT", "/api/reminders", Some(email_settings(port))).await;
    call(&e.app, "PUT", "/api/reminders/password", Some(json!({ "password": "x" }))).await;
    assert_eq!(reminders::tick(&e.shared).await.email.as_deref(), Some("skipped"));
    assert_eq!(reminders::tick(&e.shared).await.email, None);
    assert!(smtp.mails.lock().unwrap().is_empty());
}

#[tokio::test]
async fn smtp_errors_are_in_chinese() {
    // A port nobody listens on: "connection refused" becomes a sentence the author can act on.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let e = setup("2026-11-08T06:00:00+08:00");
    card(&e.app, "Dice coefficient", "衡量兩個集合重疊程度的指標").await;
    let mut st = email_settings(free);
    st["email"]["user"] = json!("");
    st["email"]["from"] = json!("ellrin@example.com");
    assert_eq!(call(&e.app, "PUT", "/api/reminders", Some(st)).await.0, StatusCode::OK);
    let (s, v) = call(&e.app, "POST", "/api/reminders/test-email", None).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert!(v["error"].as_str().unwrap().contains("拒絕連線"), "{v}");
}
