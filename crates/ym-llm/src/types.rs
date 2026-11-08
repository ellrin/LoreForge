//! Requests, replies and capabilities shared by every provider.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// An image sent to a vision model, base64 without line breaks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// `image/png`, `image/jpeg`, `image/webp` or `image/gif`.
    pub media_type: String,
    pub data: String,
}

impl Image {
    pub fn from_bytes(media_type: impl Into<String>, bytes: &[u8]) -> Image {
        Image { media_type: media_type.into(), data: base64::engine::general_purpose::STANDARD.encode(bytes) }
    }

    /// Read an image file; the media type comes from the extension.
    pub fn from_file(path: &Path) -> std::io::Result<Image> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let mt = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("不支援的圖片格式：{ext}"))),
        };
        Ok(Image::from_bytes(mt, &std::fs::read(path)?))
    }

    /// `data:<type>;base64,<data>` for OpenAI-compatible servers.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.data)
    }
}

/// One piece of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text { text: String },
    Image(Image),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Part>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Message {
        Message { role: Role::User, content: vec![Part::Text { text: text.into() }] }
    }
    pub fn assistant(text: impl Into<String>) -> Message {
        Message { role: Role::Assistant, content: vec![Part::Text { text: text.into() }] }
    }
    /// A user turn with images first, then the text (the order vision models read best).
    pub fn user_with_images(text: impl Into<String>, images: Vec<Image>) -> Message {
        let mut content: Vec<Part> = images.into_iter().map(Part::Image).collect();
        content.push(Part::Text { text: text.into() });
        Message { role: Role::User, content }
    }
    /// The text parts joined (images left out).
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                Part::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn has_images(&self) -> bool {
        self.content.iter().any(|p| matches!(p, Part::Image(_)))
    }
}

/// One call: a single prompt or a whole multi-turn transcript (oldest first,
/// starting with a user turn). No server-side session, so any engine can answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LlmRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    /// When set, the reply must be one JSON value matching this schema.
    pub json_schema: Option<Value>,
    /// Allow the model to search the web (Claude API server tool, or the
    /// subscription's WebSearch when enabled).
    pub web_search: bool,
    /// `low` | `medium` | `high` | `xhigh` | `max`; overrides the provider's default.
    pub effort: Option<String>,
    /// Sampling temperature for engines that take one (OpenAI-compatible /
    /// llama-server). **Not sent to Claude**: the current Claude API rejects
    /// sampling parameters, and the Claude Code CLI has no flag for them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Nucleus sampling, same rule as `temperature` (local engines only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
}

impl LlmRequest {
    pub const DEFAULT_MAX_TOKENS: u32 = 4096;

    /// A single prompt.
    pub fn prompt(system: impl Into<String>, user: impl Into<String>) -> LlmRequest {
        LlmRequest::chat(system, vec![Message::user(user)])
    }

    /// A multi-turn transcript.
    pub fn chat(system: impl Into<String>, messages: Vec<Message>) -> LlmRequest {
        LlmRequest {
            system: system.into(),
            messages,
            max_tokens: Self::DEFAULT_MAX_TOKENS,
            json_schema: None,
            web_search: false,
            effort: None,
            temperature: None,
            top_p: None,
        }
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }
    pub fn json(mut self, schema: Value) -> Self {
        self.json_schema = Some(schema);
        self
    }
    pub fn web(mut self, on: bool) -> Self {
        self.web_search = on;
        self
    }
    pub fn effort(mut self, e: impl Into<String>) -> Self {
        self.effort = Some(e.into());
        self
    }
    /// Sampling temperature for local / OpenAI-compatible engines (ignored by Claude).
    pub fn temperature(mut self, t: f32) -> Self {
        self.temperature = Some(t);
        self
    }
    /// Nucleus sampling for local / OpenAI-compatible engines (ignored by Claude).
    pub fn top_p(mut self, p: f32) -> Self {
        self.top_p = Some(p);
        self
    }
    /// Attach an image to the last user turn (before its text).
    pub fn image(mut self, img: Image) -> Self {
        if let Some(m) = self.messages.iter_mut().rev().find(|m| m.role == Role::User) {
            let at = m.content.iter().take_while(|p| matches!(p, Part::Image(_))).count();
            m.content.insert(at, Part::Image(img));
        } else {
            self.messages.push(Message { role: Role::User, content: vec![Part::Image(img)] });
        }
        self
    }

    pub fn has_images(&self) -> bool {
        self.messages.iter().any(Message::has_images)
    }

    /// The request as one prompt, for engines that take a single prompt (the
    /// Claude Code CLI). A single user turn comes back as it is; a transcript
    /// is written into a `<conversation>` block followed by the instruction to
    /// write the next assistant turn.
    pub fn flatten(&self) -> String {
        if let [only] = self.messages.as_slice()
            && only.role == Role::User
        {
            return only.text();
        }
        let mut user = String::from("The conversation so far, oldest first:\n\n<conversation>\n");
        for m in &self.messages {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            user.push_str(&format!("<turn role=\"{role}\">\n{}\n</turn>\n", m.text().trim()));
        }
        user.push_str("</conversation>\n\nWrite the next assistant turn, following the system prompt.");
        user
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebSource {
    pub url: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LlmResponse {
    pub text: String,
    /// The provider id that answered, e.g. `claude:claude-sonnet-5-5`.
    pub provider: String,
    /// Pages the model read through web search.
    pub sources: Vec<WebSource>,
    pub usage: Option<Usage>,
}

/// What a streaming call reports while it runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// The next piece of the reply.
    Text { text: String },
    /// The model started a web search.
    Searching { query: String },
    /// A page found by web search.
    Source(WebSource),
}

/// What a task needs from a model (the `needs` table in tasks.toml).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Needs {
    pub json: bool,
    pub vision: bool,
    pub web: bool,
    /// Minimum context window in tokens.
    pub context: Option<u32>,
    /// Token-by-token streaming (a chat window). Engines without it still
    /// answer, in one piece; set this only when one piece is not acceptable.
    pub streaming: bool,
}

/// What a provider can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Constrained JSON output (schema).
    pub json: bool,
    pub vision: bool,
    pub web: bool,
    /// Real token streaming (otherwise `stream` delivers the reply in one piece).
    pub streaming: bool,
    /// Context window in tokens.
    pub context: u32,
    /// Needs the internet (cloud models).
    pub needs_network: bool,
}

impl Capabilities {
    /// Why these capabilities fall short of `needs`, in the user's words.
    pub fn shortfall(&self, needs: &Needs) -> Option<String> {
        let mut missing = Vec::new();
        if needs.json && !self.json {
            missing.push("固定格式（JSON）輸出".to_string());
        }
        if needs.vision && !self.vision {
            missing.push("看圖".to_string());
        }
        if needs.web && !self.web {
            missing.push("網路搜尋".to_string());
        }
        if needs.streaming && !self.streaming {
            missing.push("逐字串流".to_string());
        }
        if let Some(c) = needs.context
            && self.context < c
        {
            missing.push(format!("{c} tokens 的上下文（只有 {}）", self.context));
        }
        (!missing.is_empty()).then(|| format!("不支援{}", missing.join("、")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_keeps_single_prompt_and_transcripts() {
        assert_eq!(LlmRequest::prompt("s", "the article").flatten(), "the article");
        let r = LlmRequest::chat(
            "s",
            vec![Message::user("start"), Message::assistant("Hi! What do you do?"), Message::user("I study design.")],
        );
        let flat = r.flatten();
        assert!(flat.contains("<turn role=\"assistant\">\nHi! What do you do?"));
        assert!(flat.trim_end().ends_with("following the system prompt."));
        assert!(flat.find("start").unwrap() < flat.find("I study design.").unwrap(), "oldest first");
    }

    #[test]
    fn images_go_before_text_of_the_last_user_turn() {
        let r = LlmRequest::prompt("s", "what is this?").image(Image::from_bytes("image/png", b"png"));
        assert!(r.has_images());
        assert!(matches!(r.messages[0].content[0], Part::Image(_)));
        assert!(matches!(r.messages[0].content[1], Part::Text { .. }));
        assert_eq!(Image::from_bytes("image/png", b"hi").data_url(), "data:image/png;base64,aGk=");
    }

    #[test]
    fn shortfall_names_what_is_missing() {
        let caps = Capabilities { json: true, vision: false, web: false, streaming: true, context: 16384, needs_network: false };
        assert!(caps.shortfall(&Needs { json: true, ..Default::default() }).is_none());
        let why = caps.shortfall(&Needs { vision: true, context: Some(32000), ..Default::default() }).unwrap();
        assert!(why.contains("看圖") && why.contains("32000"), "{why}");
    }
}
