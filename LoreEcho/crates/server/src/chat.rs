//! The chat (design §2): the page keeps the conversation (memory only; 新對話 clears it)
//! and sends all of it with every turn; the server keeps nothing. The reply streams back
//! as server-sent events through lore-llm's router (task `le-chat`).
//!
//! `POST /api/chat { messages: [{ role: "user" | "assistant", text }], character?, engine?, speak? }`
//! → `text/event-stream` (`speak: true`: the reply is also read aloud sentence by sentence with the
//! character's voice, lore-voice; the page follows it on `/api/voice/events`):
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
    /// Read the reply aloud as it streams (朗讀回覆).
    #[serde(default)]
    pub speak: bool,
}

/// How much of 身分與背景 goes into the prompt (characters; the rest is left out with "…").
pub const BACKGROUND_LIMIT: usize = 800;
/// Each other profile line.
pub const LINE_LIMIT: usize = 120;
/// Example pairs sent as earlier turns, and characters per line of them.
pub const EXAMPLE_PAIRS: usize = 4;
pub const EXAMPLE_LIMIT: usize = 200;

/// `s` cut to `max` characters ("…" marks the cut).
pub fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

/// Persona, name, the 角色設定 as short labelled lines, then the fixed brevity rule.
pub fn system_prompt(c: &Character) -> String {
    let persona = if c.persona.trim().is_empty() { Character::builtin().persona } else { c.persona.trim().to_string() };
    let p = &c.profile;
    let mut lines: Vec<String> = vec![];
    fn line(lines: &mut Vec<String>, label: &str, v: &str, max: usize) {
        if !v.trim().is_empty() {
            lines.push(format!("{label}：{}", clip(&v.replace('\n', " "), max)));
        }
    }
    line(&mut lines, "身分與背景", &p.background, BACKGROUND_LIMIT);
    line(&mut lines, "說話風格", &p.speech_style, LINE_LIMIT);
    let phrases: Vec<String> = p.catchphrases.iter().filter(|x| !x.trim().is_empty()).take(5).map(|x| format!("「{}」", clip(x, 30))).collect();
    if !phrases.is_empty() {
        lines.push(format!("口頭禪：{}（偶爾自然地用，不要每句都用）", phrases.join("")));
    }
    line(&mut lines, "稱呼使用者", &p.call_user, 30);
    line(&mut lines, "和使用者的關係", &p.relationship, LINE_LIMIT);
    line(&mut lines, "喜歡", &p.likes, LINE_LIMIT);
    line(&mut lines, "不喜歡", &p.dislikes, LINE_LIMIT);
    let about = if p.background.trim().is_empty() {
        "被問到你是誰、你的來歷或名字由來時，照上面的個性和設定回答，不要只重複名字；沒有設定的事就說你還沒想過，不要自己編身世。"
    } else {
        "被問到你是誰、你的來歷或名字由來時，根據「身分與背景」回答，不要只重複名字；沒寫到的事就說你還沒想過，不要編出和設定矛盾的身世。"
    };
    let profile = if lines.is_empty() { String::new() } else { format!("\n\n角色設定：\n{}", lines.join("\n")) };
    format!("{persona}\n\n你的名字是「{}」。{profile}\n\n{about}\n\n{BREVITY_RULE}", c.name)
}

/// The character's example pairs as earlier turns (user, character, …), oldest first.
pub fn example_turns(c: &Character) -> Vec<Message> {
    c.profile
        .examples
        .iter()
        .filter(|e| !e.user.trim().is_empty() && !e.reply.trim().is_empty())
        .take(EXAMPLE_PAIRS)
        .flat_map(|e| [Message::user(clip(&e.user, EXAMPLE_LIMIT)), Message::assistant(clip(&e.reply, EXAMPLE_LIMIT))])
        .collect()
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

/// The request for `le-chat`: the example turns, then the conversation.
pub fn request(c: &Character, messages: Vec<Message>) -> LlmRequest {
    let mut all = example_turns(c);
    all.extend(messages);
    LlmRequest::chat(system_prompt(c), all).temperature(LOCAL_TEMPERATURE)
}

fn event(name: &str, data: serde_json::Value) -> Event {
    Event::default().event(name).data(data.to_string())
}

/// Run `req` on `route` and stream it, after `kb` (the turn's KnowledgeBase cards: `search`,
/// `cards`, the prompt note); each piece also goes to `speaker` when reading aloud. Stops the call
/// when the page goes away (and queues nothing more to speak).
pub fn stream(route: lore_llm::Route, req: LlmRequest, kb: Option<crate::kbctx::Pre>, speaker: Option<lore_voice::Speaker>) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let engine = route.provider.id();
    let speaker = std::sync::Arc::new(std::sync::Mutex::new(speaker));
    tokio::spawn(async move {
        let req = match kb {
            Some(kb) => kb.run(req, |name, data| { let _ = tx.send(event(name, data)); }).await,
            None => req,
        };
        let _ = tx.send(event("start", json!({ "engine": engine, "label": crate::llm::engine_label(&engine) })));
        let req = route.apply(req);
        let pieces = tx.clone();
        let sp = speaker.clone();
        let on = move |ev: StreamEvent| {
            if let StreamEvent::Text { text } = ev
                && !text.is_empty()
            {
                if let Some(s) = sp.lock().unwrap().as_mut() {
                    s.push(&text);
                }
                let _ = pieces.send(event("delta", json!({ "text": text })));
            }
        };
        let finish = |cancel: bool| {
            if let Some(s) = speaker.lock().unwrap().as_mut() {
                if cancel { s.cancel() } else { s.finish() }
            }
        };
        tokio::select! {
            r = route.provider.stream(&req, &on) => {
                finish(false);
                let _ = match r {
                    Ok(r) => tx.send(event("done", json!({ "engine": engine, "text": r.text }))),
                    Err(e) => {
                        let msg = format!("模型沒有回答：{e}");
                        tx.send(event("error", json!({ "error": msg.lines().next().unwrap_or("").trim() })))
                    }
                };
            }
            _ = tx.closed() => finish(true),
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
    fn the_profile_goes_into_the_prompt_as_labelled_lines() {
        let mut c = Character::builtin();
        c.name = "King".into();
        c.profile = crate::characters::Profile {
            background: "從北方的圖書館來，\n管理一座會說話的書架。".into(),
            speech_style: "慢條斯理".into(),
            catchphrases: vec!["嗯哼".into(), "".into()],
            call_user: "小鬼".into(),
            ..Default::default()
        };
        let s = system_prompt(&c);
        assert!(s.contains("角色設定：\n身分與背景：從北方的圖書館來， 管理一座會說話的書架。"), "{s}");
        assert!(s.contains("說話風格：慢條斯理") && s.contains("口頭禪：「嗯哼」") && s.contains("稱呼使用者：小鬼"));
        assert!(!s.contains("喜歡：") && !s.contains("和使用者的關係"), "empty fields leave no line");
        assert!(s.contains("根據「身分與背景」回答"));
        assert!(s.ends_with(BREVITY_RULE), "the brevity rule stays last");
        // without a profile: no 角色設定 block, still told not to answer with the name only
        let plain = system_prompt(&Character::builtin());
        assert!(!plain.contains("角色設定") && plain.contains("不要只重複名字") && plain.ends_with(BREVITY_RULE));
    }

    #[test]
    fn a_long_background_is_cut() {
        let mut c = Character::builtin();
        c.profile.background = "字".repeat(BACKGROUND_LIMIT + 500);
        let s = system_prompt(&c);
        assert!(s.contains(&format!("{}…", "字".repeat(BACKGROUND_LIMIT))));
        assert!(!s.contains(&"字".repeat(BACKGROUND_LIMIT + 1)));
    }

    #[test]
    fn examples_come_before_the_conversation_in_order() {
        use crate::characters::Example;
        let mut c = Character::builtin();
        c.profile.examples = (1..=6).map(|i| Example { user: format!("問{i}"), reply: format!("答{i}") }).collect();
        c.profile.examples.insert(0, Example { user: "沒有回答".into(), reply: " ".into() });
        let req = request(&c, transcript(&[t("user", "真的問題")]).unwrap());
        let texts: Vec<String> = req.messages.iter().map(|m| m.text().to_string()).collect();
        assert_eq!(texts, vec!["問1", "答1", "問2", "答2", "問3", "答3", "問4", "答4", "真的問題"]);
        assert_eq!(req.messages[1].role, lore_llm::Role::Assistant);
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
