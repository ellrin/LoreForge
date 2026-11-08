//! The daily e-mail digest: due questions (answers at the bottom, so the question can be
//! tried first), recently learned cards, small numbers. Plain text + HTML. The phone cannot
//! reach the Mac, so the mail is a reminder; answering happens on the Mac (design §4.5).

use std::time::Duration as StdDuration;

use kb_core::{KbError, KnowledgeBase, NodeSummary};
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::reminders::{EmailSettings, Security, Stats, stats, weekday_zh};

#[derive(Debug, Clone)]
pub struct DigestItem {
    pub id: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub answer: String,
}

#[derive(Debug, Clone)]
pub struct Digest {
    pub subject: String,
    pub text: String,
    pub html: String,
    pub due: Vec<DigestItem>,
    pub due_total: usize,
    pub recent: Vec<DigestItem>,
    pub stats: Stats,
}

impl Digest {
    /// Nothing due and nothing learned lately.
    pub fn is_empty(&self) -> bool {
        self.due_total == 0 && self.recent.is_empty()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// First `max` characters of Markdown, flattened to one line (good enough for a reminder).
fn excerpt(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = flat.replace("[[", "").replace("]]", "").replace("**", "").replace('#', "");
    let flat = flat.trim();
    if flat.chars().count() <= max { flat.to_string() } else { format!("{}…", flat.chars().take(max).collect::<String>()) }
}

fn item(kb: &KnowledgeBase, n: &NodeSummary) -> DigestItem {
    let answer = if !n.summary.trim().is_empty() {
        excerpt(&n.summary, 240)
    } else {
        kb.node(&n.id).map(|(_, node)| excerpt(&node.body, 240)).unwrap_or_default()
    };
    DigestItem { id: n.id.clone(), title: n.title.clone(), aliases: n.aliases.clone(), answer }
}

pub fn build(kb: &KnowledgeBase, s: &EmailSettings, base_url: &str) -> Result<Digest, KbError> {
    let now = kb.now();
    let all_due = kb.due(500)?;
    let due: Vec<DigestItem> = all_due.iter().take(s.max_due as usize).map(|n| item(kb, n)).collect();
    let due_ids: std::collections::HashSet<&str> = all_due.iter().map(|n| n.id.as_str()).collect();
    let recent: Vec<DigestItem> = kb
        .recent(s.recent_days, 30)?
        .iter()
        // Due cards are left out here: their answers would sit right under the question.
        .filter(|n| !due_ids.contains(n.id.as_str()))
        .take(12)
        .map(|n| item(kb, n))
        .collect();
    let st = stats(kb)?;
    let day = format!("{}/{}（{}）", now.format("%-m"), now.format("%-d"), weekday_zh(&now));
    let subject = if all_due.is_empty() { format!("知識庫每日摘要 · {day}") } else { format!("知識庫每日摘要 · {day} · {} 張待複習", all_due.len()) };
    let link = |id: &str| format!("{base_url}/#/node/{id}");
    let review = format!("{base_url}/#/review");
    let alias_txt = |a: &[String]| if a.is_empty() { String::new() } else { format!("（{}）", a.join("、")) };
    let numbers = format!(
        "今天新增 {} · 待複習 {} · 7 天內記住 {} · 共 {} 張（學習中 {}、記得 {}、久未複習 {}）",
        st.new_today, st.due, st.known_week, st.total, st.learning, st.known, st.stale
    );

    // Plain text.
    let mut t = String::new();
    if due.is_empty() {
        t.push_str("今天沒有到期的卡片。\n");
    } else {
        t.push_str(&format!("今天有 {} 張卡片等你確認。先想一想，答案在信的最後；作答請到 Mac 上的 KnowledgeBase 或 NeonDeck。\n\n── 還記得嗎？──\n", all_due.len()));
        for (i, d) in due.iter().enumerate() {
            t.push_str(&format!("{}. 還記得「{}」嗎？{}\n", i + 1, d.title, alias_txt(&d.aliases)));
        }
        if all_due.len() > due.len() {
            t.push_str(&format!("…還有 {} 張\n", all_due.len() - due.len()));
        }
    }
    if !recent.is_empty() {
        t.push_str(&format!("\n── 最近 {} 天學到 ──\n", s.recent_days));
        for r in &recent {
            t.push_str(&format!("• {}{}\n", r.title, if r.answer.is_empty() { String::new() } else { format!(" — {}", r.answer) }));
        }
    }
    t.push_str(&format!("\n── 數字 ──\n{numbers}\n"));
    if !due.is_empty() {
        t.push_str(&format!("\n複習頁（只能在這台 Mac 上開）：{review}\n"));
        t.push_str("\n\n\n\n\n\n── 答案 ──\n");
        for (i, d) in due.iter().enumerate() {
            t.push_str(&format!("{}. {}：{}\n   {}\n", i + 1, d.title, if d.answer.is_empty() { "（卡片沒有摘要）" } else { &d.answer }, link(&d.id)));
        }
    }

    // HTML (inline styles: mail clients ignore <style> and <details>).
    let mut h = String::new();
    h.push_str(r#"<!doctype html><html lang="zh-Hant"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"></head><body style="margin:0;padding:24px 16px;background:#f6f5f2;color:#1d1b18;font:15px/1.6 -apple-system,'PingFang TC','Noto Sans TC',sans-serif">"#);
    h.push_str(r#"<div style="max-width:560px;margin:0 auto">"#);
    h.push_str(&format!(r#"<div style="font:600 12px/1 ui-monospace,Menlo,monospace;letter-spacing:.12em;color:#b4501f">KNOWLEDGE/BASE · {}</div>"#, esc(&day)));
    let sec = |title: &str| format!(r#"<h2 style="margin:28px 0 10px;font-size:13px;letter-spacing:.08em;color:#6b655c;border-bottom:1px solid #e2ddd4;padding-bottom:6px">{}</h2>"#, esc(title));
    if due.is_empty() {
        h.push_str(r#"<p style="margin:16px 0 0">今天沒有到期的卡片。</p>"#);
    } else {
        h.push_str(&format!(r#"<h1 style="margin:12px 0 4px;font-size:22px">今天有 {} 張卡片等你確認</h1><p style="margin:0;color:#6b655c">先想一想，答案在信的最後。作答請到 Mac 上的 KnowledgeBase 或 NeonDeck。</p>"#, all_due.len()));
        h.push_str(&sec("還記得嗎？"));
        h.push_str(r#"<ol style="margin:0;padding-left:22px">"#);
        for d in &due {
            h.push_str(&format!(
                r#"<li style="margin:6px 0">還記得「<b>{}</b>」嗎？{}</li>"#,
                esc(&d.title),
                if d.aliases.is_empty() { String::new() } else { format!(r#"<span style="color:#8a8378">（{}）</span>"#, esc(&d.aliases.join("、"))) }
            ));
        }
        h.push_str("</ol>");
        if all_due.len() > due.len() {
            h.push_str(&format!(r#"<p style="color:#8a8378;margin:6px 0 0">…還有 {} 張</p>"#, all_due.len() - due.len()));
        }
    }
    if !recent.is_empty() {
        h.push_str(&sec(&format!("最近 {} 天學到", s.recent_days)));
        for r in &recent {
            h.push_str(&format!(
                r#"<div style="margin:0 0 10px"><a href="{}" style="color:#b4501f;font-weight:600;text-decoration:none">{}</a>{}</div>"#,
                esc(&link(&r.id)),
                esc(&r.title),
                if r.answer.is_empty() { String::new() } else { format!(r#"<div style="color:#4a463f;font-size:14px">{}</div>"#, esc(&r.answer)) }
            ));
        }
    }
    h.push_str(&sec("數字"));
    h.push_str(&format!(
        r#"<table role="presentation" style="border-collapse:collapse"><tr>{}</tr></table><p style="color:#8a8378;font-size:13px;margin:8px 0 0">共 {} 張 · 學習中 {} · 記得 {} · 久未複習 {}</p>"#,
        [("今天新增", st.new_today), ("待複習", st.due), ("7 天內記住", st.known_week)]
            .iter()
            .map(|(k, v)| format!(r#"<td style="padding:0 22px 0 0"><div style="font:700 24px/1.1 ui-monospace,Menlo,monospace;color:#b4501f">{v}</div><div style="font-size:12px;color:#6b655c">{k}</div></td>"#))
            .collect::<String>(),
        st.total,
        st.learning,
        st.known,
        st.stale
    ));
    if !due.is_empty() {
        h.push_str(&format!(r#"<p style="margin:20px 0 0"><a href="{}" style="color:#b4501f">開啟複習頁</a> <span style="color:#8a8378;font-size:13px">（只能在這台 Mac 上開）</span></p>"#, esc(&review)));
        h.push_str(r#"<div style="height:420px"></div>"#);
        h.push_str(&sec("答案（先想再看）"));
        h.push_str(r#"<ol style="margin:0;padding-left:22px">"#);
        for d in &due {
            h.push_str(&format!(
                r#"<li style="margin:0 0 10px"><a href="{}" style="color:#1d1b18;font-weight:600">{}</a><div style="color:#4a463f;font-size:14px">{}</div></li>"#,
                esc(&link(&d.id)),
                esc(&d.title),
                if d.answer.is_empty() { "（卡片沒有摘要）".to_string() } else { esc(&d.answer) }
            ));
        }
        h.push_str("</ol>");
    }
    h.push_str(r#"<p style="margin:32px 0 0;color:#a39c90;font-size:12px">這封信由你 Mac 上的 kb-server 寄出。到 KnowledgeBase ▸ 設定 可以改時間或關掉。</p></div></body></html>"#);

    Ok(Digest { subject, text: t, html: h, due_total: all_due.len(), due, recent, stats: st })
}

/// Send `d` with the settings' SMTP server.
pub async fn send(s: &EmailSettings, password: Option<String>, d: &Digest) -> Result<(), String> {
    let from_addr = if s.from.is_empty() { &s.user } else { &s.from };
    let from: Mailbox = from_addr.parse().map_err(|_| format!("寄件人「{from_addr}」不是有效的 e-mail 地址"))?;
    if s.to.is_empty() {
        return Err("沒有收件人".into());
    }
    let mut b = Message::builder().from(from).subject(&d.subject);
    for to in &s.to {
        b = b.to(to.parse::<Mailbox>().map_err(|_| format!("收件人「{to}」不是有效的 e-mail 地址"))?);
    }
    let msg = b.multipart(MultiPart::alternative_plain_html(d.text.clone(), d.html.clone())).map_err(|e| e.to_string())?;
    let builder = match s.security {
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&s.smtp_host).map_err(|e| e.to_string())?,
        Security::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&s.smtp_host).map_err(|e| e.to_string())?,
        Security::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&s.smtp_host),
    };
    let mut builder = builder.port(s.smtp_port).timeout(Some(StdDuration::from_secs(30)));
    if !s.user.is_empty() {
        let pw = password.filter(|p| !p.is_empty()).ok_or("還沒有設定 SMTP 密碼（Gmail 請用「應用程式密碼」）")?;
        builder = builder.credentials(Credentials::new(s.user.clone(), pw));
    }
    builder.build().send(msg).await.map(|_| ()).map_err(|e| smtp_error_zh(&e, &s.smtp_host, s.smtp_port))
}

/// A send error in plain Traditional Chinese (the raw text stays in parentheses for
/// troubleshooting).
pub fn smtp_error_zh(e: &lettre::transport::smtp::Error, host: &str, port: u16) -> String {
    let mut chain = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        chain.push_str(": ");
        chain.push_str(&s.to_string());
        src = s.source();
    }
    let low = chain.to_lowercase();
    let code = e.status().map(|c| c.to_string()).unwrap_or_default();
    let what = if e.is_timeout() || low.contains("timed out") {
        format!("連線到 {host}:{port} 逾時，請確認主機、連接埠和網路")
    } else if low.contains("connection refused") {
        format!("{host}:{port} 拒絕連線，請確認連接埠和加密方式（Gmail：587 + STARTTLS）")
    } else if low.contains("failed to lookup") || low.contains("nodename nor servname") || low.contains("name or service not known") || low.contains("dns") {
        format!("找不到 SMTP 主機「{host}」，請確認拼字和網路")
    } else if code.starts_with("535") || code.starts_with("534") || low.contains("authentication") || low.contains("credentials") {
        "帳號或密碼不對（Gmail 要用「應用程式密碼」，不是登入密碼）".to_string()
    } else if e.is_tls() || low.contains("tls") || low.contains("certificate") || low.contains("handshake") {
        "加密連線失敗，請確認加密方式和連接埠是否搭配（587 用 STARTTLS、465 用 TLS）".to_string()
    } else if low.contains("network is unreachable") || low.contains("no route") {
        "目前沒有網路連線".to_string()
    } else if e.is_permanent() {
        "SMTP 伺服器拒絕了這封信".to_string()
    } else if e.is_transient() {
        "SMTP 伺服器暫時無法處理，稍後再試".to_string()
    } else {
        "寄信失敗".to_string()
    };
    format!("{what}（{chain}）")
}
