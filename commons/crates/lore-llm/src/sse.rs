//! A minimal Server-Sent Events parser for streamed replies (Claude and
//! OpenAI-compatible servers). Chunks may split lines and UTF-8 characters
//! anywhere; complete events come out in order.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    pub fn new() -> SseParser {
        SseParser::default()
    }

    /// Feed bytes; get the events completed by them.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            self.line(line, &mut out);
        }
        out
    }

    /// End of stream: an event without its trailing blank line still counts.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let rest = String::from_utf8_lossy(&std::mem::take(&mut self.buf)).to_string();
            self.line(rest.trim_end_matches('\r'), &mut out);
        }
        self.line("", &mut out);
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if !self.data.is_empty() || self.event.is_some() {
                out.push(SseEvent { event: self.event.take(), data: std::mem::take(&mut self.data).join("\n") });
            }
            return;
        }
        if line.starts_with(':') {
            return; // comment / keep-alive
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_survive_any_chunking() {
        let raw = "event: a\ndata: {\"t\":\"中文\"}\n\n: ping\n\nevent: b\r\ndata: x\r\ndata: y\r\n\r\ndata: [DONE]\n";
        let whole = {
            let mut p = SseParser::new();
            let mut v = p.push(raw.as_bytes());
            v.extend(p.finish());
            v
        };
        assert_eq!(whole.len(), 3);
        assert_eq!(whole[0], SseEvent { event: Some("a".into()), data: "{\"t\":\"中文\"}".into() });
        assert_eq!(whole[1].data, "x\ny");
        assert_eq!(whole[2].data, "[DONE]");
        // Byte by byte (splits the UTF-8 characters too).
        let mut p = SseParser::new();
        let mut v = Vec::new();
        for b in raw.as_bytes() {
            v.extend(p.push(&[*b]));
        }
        v.extend(p.finish());
        assert_eq!(v, whole);
    }
}
