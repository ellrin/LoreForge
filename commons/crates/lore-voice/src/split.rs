//! Sentences out of a streamed reply, so speaking can start before the reply ends.

/// Ends a sentence (closing quotes and brackets right after it stay with it).
const ENDS: &[char] = &['。', '！', '？', '!', '?', '；', ';', '…', '\n'];
const CLOSERS: &[char] = &['」', '』', '"', '\'', '”', '’', '）', ')', '】', '》'];
/// A clause this long without an end is cut at its last comma (speaking starts sooner).
pub const LONG: usize = 80;
/// The first sentence of a stream is cut at its first comma once at least this many
/// characters come before it, so the voice starts after a few words instead of a whole sentence.
pub const FIRST: usize = 6;
const COMMAS: &[char] = &['，', ',', '、', '：', ':'];

/// What the stream had up to a cut: `raw` = its length in UTF-16 units (what a web page
/// counts), `text` = the line to speak ([`speakable`]; `None` when nothing in it is said).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub raw: usize,
    pub text: Option<String>,
}

/// Feed it the pieces of a streamed text with [`Splitter::push`]; it hands back each finished
/// sentence once, cleaned for speaking ([`speakable`]). [`Splitter::finish`] returns the rest.
#[derive(Debug, Default, Clone)]
pub struct Splitter {
    buf: String,
    /// Something has been handed out to speak (the first-sentence cut is used up).
    spoke: bool,
}

impl Splitter {
    pub fn new() -> Splitter {
        Splitter::default()
    }

    /// The sentences finished by `piece` (possibly none).
    pub fn push(&mut self, piece: &str) -> Vec<String> {
        self.push_pieces(piece).into_iter().filter_map(|p| p.text).collect()
    }

    /// What is left at the end of the stream.
    pub fn finish(&mut self) -> Option<String> {
        self.finish_piece().and_then(|p| p.text)
    }

    /// [`Splitter::push`] with every cut, said or not, and how much of the stream it covers.
    pub fn push_pieces(&mut self, piece: &str) -> Vec<Piece> {
        self.buf.push_str(piece);
        let mut out = vec![];
        while let Some(end) = self.boundary() {
            let s: String = self.buf.drain(..end).collect();
            out.push(self.piece(&s));
        }
        out
    }

    /// [`Splitter::finish`] as a [`Piece`] (`None` when nothing was left).
    pub fn finish_piece(&mut self) -> Option<Piece> {
        let s = std::mem::take(&mut self.buf);
        (!s.is_empty()).then(|| self.piece(&s))
    }

    fn piece(&mut self, s: &str) -> Piece {
        let text = speakable(s);
        self.spoke |= text.is_some();
        Piece { raw: s.encode_utf16().count(), text }
    }

    /// Byte index just past the first sentence in the buffer, if one is complete.
    fn boundary(&self) -> Option<usize> {
        let b = &self.buf;
        let chars: Vec<(usize, char)> = b.char_indices().collect();
        // the first words of the reply: cut at its first comma that has enough before it
        if !self.spoke {
            let mut words = 0;
            for &(at, c) in &chars {
                if ENDS.contains(&c) {
                    break;
                }
                if COMMAS.contains(&c) && words >= FIRST {
                    return Some(at + c.len_utf8());
                }
                words += usize::from(c.is_alphanumeric());
            }
        }
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
            && let Some(&(at, c)) = chars.iter().rev().find(|(_, c)| COMMAS.contains(c))
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
    fn the_first_sentence_is_cut_at_a_comma_when_it_has_a_few_words_before_it() {
        assert_eq!(all(&["你上次說微積分", "還不太熟，我們今天", "就從那裡開始吧。先看定義，再做題。"]), vec!["你上次說微積分還不太熟，", "我們今天就從那裡開始吧。", "先看定義，再做題。"]);
        assert_eq!(all(&["嗯，這題不難。"]), vec!["嗯，這題不難。"], "too short before the comma");
        assert_eq!(all(&["---\n", "這一段很長很長，所以"]), vec!["這一段很長很長，", "所以"], "after something unspoken it still applies");
        assert_eq!(all(&["**重點**\n", "這一段很長很長，所以"]), vec!["重點", "這一段很長很長，所以"], "only the first line said");
    }

    #[test]
    fn pieces_cover_the_whole_stream_in_utf16_units() {
        let mut sp = Splitter::new();
        let mut got = sp.push_pieces("---\n😀 好棒啊，對吧。");
        got.extend(sp.finish_piece());
        let raw: usize = got.iter().map(|p| p.raw).sum();
        assert_eq!(raw, "---\n😀 好棒啊，對吧。".encode_utf16().count());
        assert_eq!(got[0], Piece { raw: 4, text: None });
        assert_eq!(got.iter().filter_map(|p| p.text.clone()).collect::<Vec<_>>(), vec!["😀 好棒啊，對吧。"]);
        assert_eq!(Splitter::new().finish_piece(), None);
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
