//! The chat (design §2): the page keeps the conversation (memory only; 新對話 clears it)
//! and sends all of it with every turn; the server keeps nothing. The reply streams back
//! as server-sent events through lore-llm's router (task `le-chat`).
//!
//! `POST /api/chat { messages: [{ role: "user" | "assistant", text }], character?, engine? }`
//! → `text/event-stream`:
//!
//! | event | data |
//! |---|---|
//! | `search` | `{}` the author's KnowledgeBase is being read (only when there is one) |
//! | `cards` | `{ cards, intent, quiz?, grade?, recorded?, notice?, available }` the cards this turn uses ([`crate::kbctx`]) |
//! | `start` | `{ engine, label }` (label 「本機 Gemma 4 12B」／「Claude Sonnet」, the catalog's name) |
//! | `delta` | `{ text }` the next piece |
//! | `done` | `{ engine, text }` the whole reply |
//! | `error` | `{ error }` one line, the call failed after it started |
//!
//! Before the stream starts: 400 `{ error }` for a bad request, 503 `{ error, alternative? }`
//! when no model can answer (the page offers 「改用本機」／「改用 Claude」, sent back as `engine`).

use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use lore_llm::{LlmRequest, Message, StreamEvent};
use serde::Deserialize;
use serde_json::json;

use crate::characters::Character;

/// Every reply follows this, whatever the persona says.
pub const BREVITY_RULE: &str = "說話規則：每次只回 1 到 3 句短句，用口語的台灣繁體中文，像面對面聊天；\
不要列點、標題或 Markdown。使用者說「詳細一點」或明確要求解釋時，才展開成完整的說明。";
/// Turns sent to the model (the oldest are left out).
pub const MAX_TURNS: usize = 40;
/// Characters per message.
pub const MAX_CHARS: usize = 8000;
/// Local engines: a little livelier than the drafting tasks.
const LOCAL_TEMPERATURE: f32 = 0.6;

#[derive(Debug, Clone, Deserialize)]
pub struct Turn {
    pub role: String,
    #[serde(alias = "content")]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatBody {
    #[serde(default)]
    pub messages: Vec<Turn>,
    pub character: Option<String>,
    /// `local` | `claude`: only that engine (the one-click alternative).
    pub engine: Option<String>,
    /// The page's KnowledgeBase context (quiz in progress, cards of the last reply).
    #[serde(default)]
    pub kb: Option<crate::kbctx::Ask>,
}

/// The persona, the character's name and the fixed brevity rule.
pub fn system_prompt(c: &Character) -> String {
    let persona = if c.persona.trim().is_empty() { Character::builtin().persona } else { c.persona.trim().to_string() };
    format!("{persona}\n\n你的名字是「{}」。\n\n{BREVITY_RULE}", c.name)
}

/// The page's turns as a transcript for lore-llm: empty turns dropped, the last
/// [`MAX_TURNS`], starting with the user and ending with the user's new message.
pub fn transcript(turns: &[Turn]) -> Result<Vec<Message>, String> {
    let mut out = Vec::new();
    for t in turns {
        let text = t.text.trim();
        if text.chars().count() > MAX_CHARS {
            return Err(format!("訊息太長（最多 {MAX_CHARS} 字）"));
        }
        match t.role.as_str() {
            _ if text.is_empty() => {}
            "user" => out.push(Message::user(text)),
            "assistant" => out.push(Message::assistant(text)),
            other => return Err(format!("不認得的角色「{other}」")),
        }
    }
    if out.len() > MAX_TURNS {
        out.drain(..out.len() - MAX_TURNS);
    }
    while out.first().is_some_and(|m| m.role != lore_llm::Role::User) {
        out.remove(0);
    }
    match out.last() {
        Some(m) if m.role == lore_llm::Role::User => Ok(out),
        _ => Err("沒有要回答的訊息".into()),
    }
}

/// The request for `le-chat`.
pub fn request(c: &Character, messages: Vec<Message>) -> LlmRequest {
    LlmRequest::chat(system_prompt(c), messages).temperature(LOCAL_TEMPERATURE)
}

fn event(name: &str, data: serde_json::Value) -> Event {
    Event::default().event(name).data(data.to_string())
}

/// Run `req` on `route` and stream it, after `kb` (the turn's KnowledgeBase cards: `search`,
/// `cards`, the prompt note). Stops the call when the page goes away.
pub fn stream(route: lore_llm::Route, req: LlmRequest, kb: Option<crate::kbctx::Pre>) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let engine = route.provider.id();
    tokio::spawn(async move {
        let req = match kb {
            Some(kb) => kb.run(req, |name, data| { let _ = tx.send(event(name, data)); }).await,
            None => req,
        };
        let _ = tx.send(event("start", json!({ "engine": engine, "label": crate::llm::engine_label(&engine) })));
        let req = route.apply(req);
        let pieces = tx.clone();
        let on = move |ev: StreamEvent| {
            if let StreamEvent::Text { text } = ev
                && !text.is_empty()
            {
                let _ = pieces.send(event("delta", json!({ "text": text })));
            }
        };
        tokio::select! {
            r = route.provider.stream(&req, &on) => {
                let _ = match r {
                    Ok(r) => tx.send(event("done", json!({ "engine": engine, "text": r.text }))),
                    Err(e) => {
                        let msg = format!("模型沒有回答：{e}");
                        tx.send(event("error", json!({ "error": msg.lines().next().unwrap_or("").trim() })))
                    }
                };
            }
            _ = tx.closed() => {}
        }
    });
    let s = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (Ok(e), rx)) });
    Sse::new(s).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(role: &str, text: &str) -> Turn {
        Turn { role: role.into(), text: text.into() }
    }

    #[test]
    fn transcript_starts_and_ends_with_the_user() {
        let m = transcript(&[t("assistant", "嗨"), t("user", "你好"), t("assistant", "  "), t("assistant", "嗯"), t("user", "再說")]).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].text(), "你好");
        assert_eq!(m[2].text(), "再說");
        assert!(transcript(&[t("user", "a"), t("assistant", "b")]).is_err());
        assert!(transcript(&[]).is_err());
        assert!(transcript(&[t("system", "x")]).is_err());
        assert!(transcript(&[t("user", &"字".repeat(MAX_CHARS + 1))]).is_err());
    }

    #[test]
    fn long_conversations_keep_the_newest_turns() {
        let turns: Vec<Turn> = (0..100).map(|i| t(if i % 2 == 0 { "user" } else { "assistant" }, &format!("{i}"))).collect::<Vec<_>>();
        let mut turns = turns;
        turns.push(t("user", "last"));
        let m = transcript(&turns).unwrap();
        assert!(m.len() <= MAX_TURNS);
        assert_eq!(m[0].role, lore_llm::Role::User);
        assert_eq!(m.last().unwrap().text(), "last");
    }

    #[test]
    fn system_prompt_has_persona_name_and_rule() {
        let mut c = Character::builtin();
        c.persona = "你是一隻貓。".into();
        c.name = "小咪".into();
        let s = system_prompt(&c);
        assert!(s.starts_with("你是一隻貓。") && s.contains("小咪") && s.contains(BREVITY_RULE));
    }
}
