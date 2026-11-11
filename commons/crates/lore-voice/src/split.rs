//! Sentences out of a streamed reply, so speaking can start before the reply ends.

/// Ends a sentence (closing quotes and brackets right after it stay with it).
const ENDS: &[char] = &['。', '！', '？', '!', '?', '；', ';', '…', '\n'];
const CLOSERS: &[char] = &['」', '』', '"', '\'', '”', '’', '）', ')', '】', '》'];
/// A clause this long without an end is cut at its last comma (speaking starts sooner).
pub const LONG: usize = 80;

/// Feed it the pieces of a streamed text with [`Splitter::push`]; it hands back each finished
/// sentence once, cleaned for speaking ([`speakable`]). [`Splitter::finish`] returns the rest.
#[derive(Debug, Default, Clone)]
pub struct Splitter {
    buf: String,
}

impl Splitter {
    pub fn new() -> Splitter {
        Splitter::default()
    }

    /// The sentences finished by `piece` (possibly none).
    pub fn push(&mut self, piece: &str) -> Vec<String> {
        self.buf.push_str(piece);
        let mut out = vec![];
        while let Some(end) = self.boundary() {
            let s: String = self.buf.drain(..end).collect();
            if let Some(s) = speakable(&s) {
                out.push(s);
            }
        }
        out
    }

    /// What is left at the end of the stream.
    pub fn finish(&mut self) -> Option<String> {
        let s = std::mem::take(&mut self.buf);
        speakable(&s)
    }

    /// Byte index just past the first sentence in the buffer, if one is complete.
    fn boundary(&self) -> Option<usize> {
        let b = &self.buf;
        let chars: Vec<(usize, char)> = b.char_indices().collect();
        for (i, &(at, c)) in chars.iter().enumerate() {
            let is_end = ENDS.contains(&c)
                // "." ends an English sentence only before a space (3.14, e.g. stay whole)
                || (c == '.' && chars.get(i + 1).is_some_and(|(_, n)| n.is_whitespace()));
            if !is_end {
                continue;
            }
            // a run of ends and closers ("！？」") belongs to this sentence
            let mut j = i + 1;
            while let Some(&(_, n)) = chars.get(j) {
                if ENDS.contains(&n) || CLOSERS.contains(&n) || n == '.' {
                    j += 1;
                } else {
                    break;
                }
            }
            if j == chars.len() && c != '\n' {
                // the stream may still add "」" or "！": wait for the next piece
                return None;
            }
            return Some(chars.get(j).map_or(b.len(), |(p, _)| *p)).filter(|_| at < b.len());
        }
        // a very long clause: cut after its last comma
        if chars.len() > LONG
            && let Some(&(at, c)) = chars.iter().rev().find(|(_, c)| matches!(c, '，' | ',' | '、' | '：' | ':'))
        {
            return Some(at + c.len_utf8());
        }
        None
    }
}

/// `s` as a line to speak: Markdown marks, list bullets and extra spaces gone; `None` when nothing is left to say.
pub fn speakable(s: &str) -> Option<String> {
    let cleaned: String = s.chars().filter(|c| !matches!(c, '*' | '#' | '`' | '_' | '>' | '|')).collect();
    let t = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim_start_matches(['-', '•', '·', ' ']).to_string();
    t.chars().any(|c| c.is_alphanumeric()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(pieces: &[&str]) -> Vec<String> {
        let mut sp = Splitter::new();
        let mut out: Vec<String> = pieces.iter().flat_map(|p| sp.push(p)).collect();
        out.extend(sp.finish());
        out
    }

    #[test]
    fn chinese_sentences_in_small_pieces() {
        assert_eq!(all(&["今天", "天氣很好。我們", "去散步吧！要", "不要？"]), vec!["今天天氣很好。", "我們去散步吧！", "要不要？"]);
    }

    #[test]
    fn a_sentence_is_handed_out_as_soon_as_the_next_one_starts() {
        let mut sp = Splitter::new();
        assert!(sp.push("你好。").is_empty(), "a closing quote might still come");
        assert_eq!(sp.push("我"), vec!["你好。"]);
        assert_eq!(sp.finish().as_deref(), Some("我"));
    }

    #[test]
    fn closing_quotes_and_runs_stay_with_the_sentence() {
        assert_eq!(all(&["他說：「真的嗎？」然後", "笑了。"]), vec!["他說：「真的嗎？」", "然後笑了。"]);
        assert_eq!(all(&["什麼！？", "好吧"]), vec!["什麼！？", "好吧"]);
    }

    #[test]
    fn english_periods_and_numbers() {
        assert_eq!(all(&["Pi is 3.14 roughly. Next one", " here."]), vec!["Pi is 3.14 roughly.", "Next one here."]);
    }

    #[test]
    fn markdown_and_empty_lines_are_not_spoken() {
        assert_eq!(all(&["**重點**：", "一\n\n", "- 二\n", "---\n"]), vec!["重點：一", "二"]);
        assert_eq!(speakable("  ** \n"), None);
    }

    #[test]
    fn a_long_clause_is_cut_at_a_comma() {
        let long = format!("{}，{}", "長".repeat(LONG), "還沒完");
        let mut sp = Splitter::new();
        let got = sp.push(&long);
        assert_eq!(got, vec![format!("{}，", "長".repeat(LONG))]);
        assert_eq!(sp.finish().as_deref(), Some("還沒完"));
    }
}
