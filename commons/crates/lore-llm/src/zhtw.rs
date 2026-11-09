//! Taiwan usage for local-model replies (from PaperComposer).
//!
//! Local models (Gemma, Qwen) asked for Traditional Chinese write traditional
//! characters but mainland vocabulary: 數據, 網絡, 視頻, 信息, 代碼. OpenCC's
//! s2twp table turns those into Taiwan usage (資料, 網路, 影片, 資訊, 程式碼)
//! and leaves English, LaTeX and JSON structure alone. Words that s2twp would
//! change but are also correct in Taiwan are kept as written.

use ferrous_opencc::{OpenCC, config::BuiltinConfig};
use std::sync::OnceLock;

/// Correct Taiwan words that s2twp still rewrites (文件 "document" → 檔案 "file").
const KEEP: &[&str] = &["文件"];

/// Placeholder for KEEP[i]: a private-use character no model writes.
fn placeholder(i: usize) -> char {
    char::from_u32(0xE000 + i as u32).expect("private-use range")
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
}

/// Rewrite Chinese text in Taiwan usage; text without Chinese comes back unchanged.
pub fn to_taiwan(text: &str) -> String {
    if !text.chars().any(is_cjk) {
        return text.to_string();
    }
    static CC: OnceLock<Option<OpenCC>> = OnceLock::new();
    let Some(cc) = CC.get_or_init(|| OpenCC::from_config(BuiltinConfig::S2twp).ok()) else {
        return text.to_string();
    };
    let mut s = text.to_string();
    for (i, w) in KEEP.iter().enumerate() {
        s = s.replace(w, &placeholder(i).to_string());
    }
    let mut out = cc.convert(&s);
    for (i, w) in KEEP.iter().enumerate() {
        out = out.replace(placeholder(i), w);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::to_taiwan;

    #[test]
    fn mainland_terms_become_taiwan_usage() {
        assert_eq!(to_taiwan("生成對抗網絡降低對標註數據的依賴。"), "生成對抗網路降低對標註資料的依賴。");
        assert_eq!(to_taiwan("我们用开源软件处理视频信息，代码放在服务器上。"), "我們用開源軟體處理影片資訊，程式碼放在伺服器上。");
    }

    #[test]
    fn taiwan_text_is_left_alone() {
        for s in ["她的頭髮乾淨，後面的著作在里程碑之後發表。", "物體的質量與加速度成正比。", "請參考附件文件與專案計畫書。"] {
            assert_eq!(to_taiwan(s), s);
        }
    }

    #[test]
    fn english_latex_and_json_unchanged() {
        let s = r#"{"text": "We train ResNet-50 with $\alpha=0.1$ \cite{he2016}.", "reason": "符合 Key value"}"#;
        assert_eq!(to_taiwan(s), s);
    }
}
