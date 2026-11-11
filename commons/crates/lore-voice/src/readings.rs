//! Taiwan readings for a synthesised voice (Qwen3-TTS reads characters the mainland way).
//!
//! The model takes characters, not pinyin, so a word whose Taiwan reading differs is written
//! with characters that sound like the Taiwan reading (垃圾 → 樂色 = lè sè). Only words where the
//! swap is safe are on by default; 和 as hàn is an option (it would also change 和平, so only the
//! standalone conjunction between two words is touched). Words that are a different word in
//! Taiwan (星期 / 禮拜) are left as written.

/// `(written, sounds like)`, always applied: 垃圾 lè sè, 企 qì, 危 wéi, 液 yì.
pub const TABLE: [(&str, &str); 8] = [
    ("垃圾", "樂色"),
    ("企業", "器業"),
    ("企圖", "器圖"),
    ("危險", "圍險"),
    ("危機", "圍機"),
    ("液體", "意體"),
    ("血液", "血意"),
    ("溶液", "溶意"),
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// 和 as hàn (the conjunction).
    pub han: bool,
}

/// `text` with the Taiwan readings of [`TABLE`] (and 和 → 汗 with `han`).
pub fn taiwan(text: &str, o: Options) -> String {
    let mut s = text.to_string();
    for (from, to) in TABLE {
        s = s.replace(from, to);
    }
    if o.han {
        s = han(&s);
    }
    s
}

/// 和 between two Han characters, not part of a word that keeps hé (和平, 和諧, 溫和, 總和, 和尚…).
fn han(s: &str) -> String {
    const KEEP_AFTER: [char; 8] = ['平', '諧', '好', '尚', '藹', '氣', '解', '睦'];
    const KEEP_BEFORE: [char; 6] = ['溫', '總', '柔', '緩', '調', '附'];
    let cs: Vec<char> = s.chars().collect();
    let is_han = |c: char| ('\u{4e00}'..='\u{9fff}').contains(&c);
    cs.iter()
        .enumerate()
        .map(|(i, &c)| {
            if c != '和' {
                return c;
            }
            let (prev, next) = (i.checked_sub(1).map(|j| cs[j]), cs.get(i + 1).copied());
            let between = prev.is_some_and(is_han) && next.is_some_and(is_han);
            if between && !next.is_some_and(|n| KEEP_AFTER.contains(&n)) && !prev.is_some_and(|p| KEEP_BEFORE.contains(&p)) { '汗' } else { c }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taiwan_readings() {
        assert_eq!(taiwan("把垃圾拿去倒", Options::default()), "把樂色拿去倒");
        assert_eq!(taiwan("星期三見", Options::default()), "星期三見");
        assert_eq!(taiwan("我和你", Options::default()), "我和你", "和 is an option");
        assert_eq!(taiwan("我和你，和平與溫和", Options { han: true }), "我汗你，和平與溫和");
        assert_eq!(taiwan("和你說", Options { han: true }), "和你說", "not between two words");
    }
}
