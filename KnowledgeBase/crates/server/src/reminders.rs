//! Recall reminders (design §4.5): today's numbers, the daily e-mail digest and the macOS
//! notification fallback, with a small scheduler that runs inside kb-server.
//!
//! Where things live (never inside a repository):
//! - settings: `<app dir>/reminders.toml` (written by the web UI's settings page);
//! - state: `<app dir>/reminders-state.json` (last e-mail day, last notification slot);
//! - SMTP password: the OS keychain, service `KnowledgeBase`, account `smtp-password`
//!   (`lore_secrets::KeyStore`; `KB_SMTP_PASSWORD` overrides it for this process,
//!   `<app dir>/credentials.toml` is the fallback when no keychain is usable).
//!
//! `<app dir>` is `$KB_HOME` or `<OS config dir>/KnowledgeBase`.
//!
//! Notification slots (`notify.times`, default 09:00 and 21:00) are claimed once per day
//! each: NeonDeck's `knowledge` widget claims a slot when it passes
//! (`POST /api/reminders/claim`) and shows the notification itself; when nobody claimed it
//! `grace_minutes` later, kb-server shows it with `osascript` (if `notify.enabled`). A Mac
//! that slept past a slot or the e-mail time catches up once when it wakes; the persisted
//! state makes sure nothing is sent twice.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDateTime, NaiveTime};
use kb_core::{Filter, KbError, KnowledgeBase};
use serde::{Deserialize, Serialize};
use lore_secrets::{Account, KeySource, KeyStore};
use lore_store::{CommandRunner, SystemRunner, write_atomic};

use crate::Shared;
use crate::digest;

pub type Time = DateTime<FixedOffset>;

// ── Settings ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// Port 587: plain connection upgraded with STARTTLS (Gmail's usual setting).
    #[default]
    Starttls,
    /// Port 465: TLS from the first byte.
    Tls,
    /// No encryption. Only for a local relay or tests.
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailSettings {
    pub enabled: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub security: Security,
    /// SMTP login (for Gmail: the full address). Empty = no authentication.
    pub user: String,
    /// Sender; empty = `user`.
    pub from: String,
    pub to: Vec<String>,
    /// Local time `HH:MM`.
    pub send_at: String,
    /// No mail on a day with nothing due and nothing new.
    pub skip_empty: bool,
    pub recent_days: i64,
    pub max_due: u32,
}

impl Default for EmailSettings {
    fn default() -> EmailSettings {
        EmailSettings {
            enabled: false,
            smtp_host: "smtp.gmail.com".into(),
            smtp_port: 587,
            security: Security::Starttls,
            user: String::new(),
            from: String::new(),
            to: Vec::new(),
            send_at: "07:30".into(),
            skip_empty: true,
            recent_days: 7,
            max_due: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySettings {
    /// kb-server shows the notification itself when NeonDeck did not.
    pub enabled: bool,
    /// Local times `HH:MM`; NeonDeck uses the same list.
    pub times: Vec<String>,
    /// How long kb-server waits for NeonDeck to claim a slot.
    pub grace_minutes: i64,
}

impl Default for NotifySettings {
    fn default() -> NotifySettings {
        NotifySettings { enabled: true, times: vec!["09:00".into(), "21:00".into()], grace_minutes: 2 }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReminderSettings {
    pub email: EmailSettings,
    pub notify: NotifySettings,
}

pub fn parse_hm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

fn check_address(a: &str, what: &str) -> Result<(), String> {
    a.trim().parse::<lettre::message::Mailbox>().map(|_| ()).map_err(|_| format!("{what}「{a}」不是有效的 e-mail 地址"))
}

impl ReminderSettings {
    /// Normalise (trim, sort times) and check. Errors are in Traditional Chinese for the UI.
    pub fn validate(&mut self) -> Result<(), String> {
        let n = &mut self.notify;
        let mut times = Vec::new();
        for t in &n.times {
            if t.trim().is_empty() {
                continue;
            }
            let hm = parse_hm(t).ok_or_else(|| format!("通知時間「{t}」格式要像 09:00"))?;
            times.push(hm.format("%H:%M").to_string());
        }
        times.sort();
        times.dedup();
        n.times = times;
        n.grace_minutes = n.grace_minutes.clamp(0, 120);

        let e = &mut self.email;
        e.smtp_host = e.smtp_host.trim().to_string();
        e.user = e.user.trim().to_string();
        e.from = e.from.trim().to_string();
        e.to = e.to.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        e.send_at = parse_hm(&e.send_at).ok_or_else(|| format!("寄送時間「{}」格式要像 07:30", e.send_at))?.format("%H:%M").to_string();
        e.recent_days = e.recent_days.clamp(1, 60);
        e.max_due = e.max_due.clamp(1, 200);
        if e.smtp_port == 0 {
            return Err("SMTP 連接埠不能是 0".into());
        }
        for t in &e.to {
            check_address(t, "收件人")?;
        }
        if !e.from.is_empty() {
            check_address(&e.from, "寄件人")?;
        }
        if e.enabled {
            if e.smtp_host.is_empty() {
                return Err("請填 SMTP 主機".into());
            }
            if e.to.is_empty() {
                return Err("請填收件人".into());
            }
            if e.from.is_empty() {
                if e.user.is_empty() {
                    return Err("請填 SMTP 帳號（也會當成寄件人）".into());
                }
                check_address(&e.user, "SMTP 帳號（同時當寄件人）")?;
            }
        }
        Ok(())
    }
}

// ── State ───────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReminderState {
    /// Local day `YYYY-MM-DD` of the last digest sent (or skipped because empty).
    pub email_last_sent: Option<String>,
    pub email_last_result: Option<String>,
    /// Failed attempts on `email_attempt_day` (at most 3 per day, 15 minutes apart).
    pub email_attempt_day: Option<String>,
    pub email_attempts: u32,
    pub email_last_attempt: Option<String>,
    pub email_last_error: Option<String>,
    /// Last notification slot handled, `YYYY-MM-DDTHH:MM` (sorts as text).
    pub notify_last_slot: Option<String>,
    /// `neondeck` | `kb-server`.
    pub notify_last_by: Option<String>,
    pub notify_last_error: Option<String>,
}

// ── Store ───────────────────────────────────────────────

pub struct Reminders {
    /// `<app dir>`; `None` = no config folder (settings are defaults, nothing persists).
    pub dir: Option<PathBuf>,
    pub keys: KeyStore,
    /// Runs `osascript` for the notification fallback.
    pub runner: Arc<dyn CommandRunner>,
    /// Base of links in e-mails and notifications (`http://127.0.0.1:7700`).
    pub base_url: String,
    /// Serialises the scheduler and manual sends.
    pub busy: tokio::sync::Mutex<()>,
}

pub const KEYCHAIN_SERVICE: &str = "KnowledgeBase";

/// `$KB_HOME` or `<OS config dir>/KnowledgeBase`.
pub fn app_dir() -> Option<PathBuf> {
    std::env::var_os("KB_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).or_else(|| dirs::config_dir().map(|d| d.join("KnowledgeBase")))
}

impl Reminders {
    /// The real thing: keychain, `osascript`.
    pub fn system(dir: Option<PathBuf>, base_url: String) -> Reminders {
        let keys = KeyStore {
            service: KEYCHAIN_SERVICE.into(),
            use_keychain: std::env::var_os("KB_NO_KEYCHAIN").is_none(),
            app_dir: dir.clone(),
            env_var: Some("KB_SMTP_PASSWORD".into()),
            account: "smtp-password".into(),
            file_key: "smtp_password".into(),
        };
        Reminders { dir, keys, runner: Arc::new(SystemRunner), base_url, busy: tokio::sync::Mutex::new(()) }
    }

    /// Tests: no keychain, no environment, a fake runner.
    pub fn isolated(dir: &Path, runner: Arc<dyn CommandRunner>, base_url: &str) -> Reminders {
        let mut r = Reminders::system(Some(dir.to_path_buf()), base_url.into());
        r.keys.use_keychain = false;
        r.keys.env_var = None;
        r.runner = runner;
        r
    }

    pub fn settings_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("reminders.toml"))
    }
    fn state_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("reminders-state.json"))
    }

    pub fn settings(&self) -> ReminderSettings {
        let Some(p) = self.settings_path().filter(|p| p.is_file()) else { return ReminderSettings::default() };
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| toml::from_str(&s).map_err(|e| e.to_string())) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("kb-server: {}: {e} (using defaults)", p.display());
                ReminderSettings::default()
            }
        }
    }

    pub fn save_settings(&self, s: &ReminderSettings) -> Result<(), String> {
        let p = self.settings_path().ok_or("這台電腦沒有設定資料夾")?;
        let body = format!("# KnowledgeBase 提醒設定（由設定頁寫入；密碼不在這裡，在鑰匙圈）\n{}", toml::to_string(s).map_err(|e| e.to_string())?);
        write_atomic(&p, body.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn state(&self) -> ReminderState {
        self.state_path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_state(&self, st: &ReminderState) {
        if let Some(p) = self.state_path() {
            let mut body = serde_json::to_string_pretty(st).unwrap_or_default();
            body.push('\n');
            if let Err(e) = write_atomic(&p, body.as_bytes()) {
                eprintln!("kb-server: {}: {e}", p.display());
            }
        }
    }

    /// SMTP password and where it came from. May prompt for keychain access once per process.
    pub fn password(&self) -> Option<(String, KeySource)> {
        self.keys.get(None, &Account::Single)
    }

    /// Store the SMTP password (empty removes it).
    pub fn set_password(&self, pw: &str) -> Result<KeySource, String> {
        self.keys.save(pw, &Account::Single)
    }

    /// Show a macOS notification with `osascript`. **Not tested on real hardware.**
    pub fn notify(&self, title: &str, body: &str) -> Result<(), String> {
        let script = format!("display notification {} with title {}", applescript_str(body), applescript_str(title));
        let out = self.runner.run("osascript", &["-e", &script]).map_err(|e| e.to_string())?;
        if out.status != 0 {
            return Err(format!("osascript: {}", out.stderr.trim()));
        }
        Ok(())
    }
}

/// An AppleScript string literal.
pub fn applescript_str(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    format!("\"{}\"", flat.replace('\\', "\\\\").replace('"', "\\\""))
}

// ── Numbers ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    /// Local day `YYYY-MM-DD`.
    pub today: String,
    pub new_today: usize,
    pub updated_today: usize,
    pub due: usize,
    /// Cards that became known in the last 7 days.
    pub known_week: usize,
    pub total: usize,
    pub learning: usize,
    pub known: usize,
    pub stale: usize,
}

pub fn stats(kb: &KnowledgeBase) -> Result<Stats, KbError> {
    let now = kb.now();
    let today = now.date_naive().to_string();
    let all = kb.list(&Filter { limit: Some(5000), ..Default::default() })?;
    let week_ago = now - Duration::days(7);
    let mut s = Stats { today: today.clone(), total: all.len(), due: kb.due(5000)?.len(), ..Default::default() };
    for n in &all {
        if n.created_at.starts_with(&today) {
            s.new_today += 1;
        } else if n.updated_at.starts_with(&today) {
            s.updated_today += 1;
        }
        match n.state.as_deref() {
            Some("learning") => s.learning += 1,
            Some("known") => {
                s.known += 1;
                if kb.progress(&n.id)?.is_some_and(|p| p.log.iter().any(|e| e.event == "known" && e.at >= week_ago)) {
                    s.known_week += 1;
                }
            }
            Some("stale") => s.stale += 1,
            _ => {}
        }
    }
    Ok(s)
}

/// Title and body of the "cards are due" notification.
pub fn notify_text(kb: &KnowledgeBase) -> Result<(usize, String, String), KbError> {
    let due = kb.due(200)?;
    let titles: Vec<&str> = due.iter().take(3).map(|n| n.title.as_str()).collect();
    let more = if due.len() > 3 { "…" } else { "" };
    Ok((due.len(), format!("記得嗎？{} 張知識卡到期", due.len()), format!("{}{more}", titles.join("、"))))
}

// ── Notification slots ──────────────────────────────────

/// The latest configured time today that is not in the future: (`YYYY-MM-DDTHH:MM`, local time).
pub fn latest_slot(now: &Time, times: &[String]) -> Option<(String, NaiveDateTime)> {
    let today = now.date_naive();
    let t = now.time();
    times.iter().filter_map(|s| parse_hm(s)).filter(|x| *x <= t).max().map(|x| (format!("{today}T{}", x.format("%H:%M")), today.and_time(x)))
}

#[derive(Debug, Clone, Serialize)]
pub struct Claim {
    pub slot: Option<String>,
    /// The caller should show the notification (it claimed the slot and cards are due).
    pub claimed: bool,
    pub due: usize,
    pub title: String,
    pub body: String,
}

/// NeonDeck (or anything else) takes the current slot. Only the first caller per slot gets
/// `claimed: true`.
pub fn claim(rem: &Reminders, kb: &KnowledgeBase, by: &str) -> Result<Claim, KbError> {
    let settings = rem.settings();
    let now = kb.now();
    let (due, title, body) = notify_text(kb)?;
    let Some((slot, _)) = latest_slot(&now, &settings.notify.times) else {
        return Ok(Claim { slot: None, claimed: false, due, title, body });
    };
    let mut st = rem.state();
    let fresh = st.notify_last_slot.as_deref().is_none_or(|l| l < slot.as_str());
    if fresh {
        st.notify_last_slot = Some(slot.clone());
        st.notify_last_by = Some(by.to_string());
        rem.save_state(&st);
    }
    Ok(Claim { slot: Some(slot), claimed: fresh && due > 0, due, title, body })
}

// ── Scheduler ───────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct TickReport {
    pub notified: bool,
    pub notify_error: Option<String>,
    /// `sent` | `skipped` (nothing to say) | `failed`.
    pub email: Option<String>,
    pub email_error: Option<String>,
}

fn email_is_due(now: &Time, e: &EmailSettings, st: &ReminderState) -> bool {
    let today = now.date_naive().to_string();
    let Some(at) = parse_hm(&e.send_at) else { return false };
    if !e.enabled || now.time() < at || st.email_last_sent.as_deref() == Some(today.as_str()) {
        return false;
    }
    if st.email_attempt_day.as_deref() == Some(today.as_str()) {
        if st.email_attempts >= 3 {
            return false;
        }
        if let Some(last) = st.email_last_attempt.as_deref().and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            && *now - last < Duration::minutes(15)
        {
            return false;
        }
    }
    true
}

/// One scheduler step: the notification fallback, then the digest. Safe to call often.
pub async fn tick(shared: &Shared) -> TickReport {
    let rem = &shared.rem;
    let _busy = rem.busy.lock().await;
    let settings = rem.settings();
    let mut report = TickReport::default();

    // Notification fallback.
    {
        let kb = shared.kb.lock().unwrap_or_else(|p| p.into_inner());
        let now = kb.now();
        if settings.notify.enabled
            && let Some((slot, at)) = latest_slot(&now, &settings.notify.times)
        {
            let mut st = rem.state();
            let fresh = st.notify_last_slot.as_deref().is_none_or(|l| l < slot.as_str());
            if fresh && now.naive_local() >= at + Duration::minutes(settings.notify.grace_minutes) {
                st.notify_last_slot = Some(slot);
                st.notify_last_by = Some("kb-server".into());
                st.notify_last_error = None;
                match notify_text(&kb) {
                    Ok((due, title, body)) if due > 0 => match rem.notify(&title, &body) {
                        Ok(()) => report.notified = true,
                        Err(e) => {
                            st.notify_last_error = Some(e.clone());
                            report.notify_error = Some(e);
                        }
                    },
                    Ok(_) => {}
                    Err(e) => report.notify_error = Some(e.to_string()),
                }
                rem.save_state(&st);
            }
        }
    }

    // Daily digest.
    let job = {
        let kb = shared.kb.lock().unwrap_or_else(|p| p.into_inner());
        let now = kb.now();
        let st = rem.state();
        if email_is_due(&now, &settings.email, &st) { Some((now, digest::build(&kb, &settings.email, &rem.base_url))) } else { None }
    };
    let Some((now, built)) = job else { return report };
    let today = now.date_naive().to_string();
    let mut st = rem.state();
    let result = match built {
        Ok(d) if settings.email.skip_empty && d.is_empty() => Ok("skipped"),
        Ok(d) => send_with_keychain(rem, &settings.email, &d).await.map(|_| "sent"),
        Err(e) => Err(e.to_string()),
    };
    match result {
        Ok(what) => {
            st.email_last_sent = Some(today);
            st.email_last_result = Some(format!("{what} {}", now.to_rfc3339()));
            st.email_attempts = 0;
            st.email_attempt_day = None;
            st.email_last_error = None;
            report.email = Some(what.into());
        }
        Err(e) => {
            if st.email_attempt_day.as_deref() != Some(today.as_str()) {
                st.email_attempt_day = Some(today);
                st.email_attempts = 0;
            }
            st.email_attempts += 1;
            st.email_last_attempt = Some(now.to_rfc3339());
            st.email_last_error = Some(e.clone());
            report.email = Some("failed".into());
            report.email_error = Some(e);
        }
    }
    rem.save_state(&st);
    report
}

/// Read the password (off the async threads: the keychain may block) and send.
pub async fn send_with_keychain(rem: &Reminders, e: &EmailSettings, d: &digest::Digest) -> Result<(), String> {
    let keys = rem.keys.clone();
    let pw = tokio::task::spawn_blocking(move || keys.get(None, &Account::Single).map(|(k, _)| k)).await.map_err(|e| e.to_string())?;
    digest::send(e, pw, d).await
}

/// Run [`tick`] every minute (first after a few seconds). A Mac waking from sleep simply
/// runs the next tick late, which is the catch-up.
pub fn spawn_scheduler(shared: Shared) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let mut iv = tokio::time::interval(std::time::Duration::from_secs(60));
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            iv.tick().await;
            let r = tick(&shared).await;
            if r.notified || r.email.is_some() || r.notify_error.is_some() {
                eprintln!("kb-server reminders: {}", serde_json::to_string(&r).unwrap_or_default());
            }
        }
    });
}

/// Weekday in Traditional Chinese (`一`…`日`).
pub fn weekday_zh(t: &Time) -> &'static str {
    ["一", "二", "三", "四", "五", "六", "日"][t.weekday().num_days_from_monday() as usize]
}
