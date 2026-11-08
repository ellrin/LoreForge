//! Plain text per section, for kb-core's term scanner and ym-notation's
//! extractor (design §6 learning flow, §7.5). No dependency on either: they
//! take strings.

use crate::model::{AssetCatalog, AssetKind};
use serde::{Deserialize, Serialize};

/// One section's text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionText {
    pub id: String,
    pub level: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    pub title: String,
    /// Paragraphs joined by blank lines.
    pub text: String,
}

/// What to include besides section paragraphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOptions {
    /// Append the captions of figures/tables that sit in the section
    /// (`Fig. 1. …` lines). ym-notation does not treat captions as definitions.
    pub captions: bool,
    /// Keep `$…$` / `$$…$$` math; when false math is removed.
    pub math: bool,
}

impl Default for TextOptions {
    fn default() -> Self {
        TextOptions {
            captions: false,
            math: true,
        }
    }
}

fn strip_math(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_math = false;
    for ch in s.chars() {
        if ch == '$' {
            in_math = !in_math;
            continue;
        }
        if !in_math {
            out.push(ch);
        }
    }
    out.split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" .", ".")
        .replace(" ,", ",")
}

impl AssetCatalog {
    /// Plain text of every section, in document order (abstract first when present).
    pub fn section_texts(&self, o: TextOptions) -> Vec<SectionText> {
        self.sections
            .iter()
            .map(|s| {
                let mut paras: Vec<String> = s.paragraphs.clone();
                if o.captions {
                    for a in self
                        .assets
                        .iter()
                        .filter(|a| a.section.as_deref() == Some(s.id.as_str()))
                    {
                        if let (Some(c), Some(n)) = (&a.caption, &a.number) {
                            let head = match a.kind {
                                AssetKind::Figure => "Fig.",
                                AssetKind::Table => "Table",
                                _ => continue,
                            };
                            paras.push(format!("{head} {n}. {c}"));
                        }
                    }
                }
                let mut text = paras.join("\n\n");
                if !o.math {
                    text = text
                        .split("\n\n")
                        .map(strip_math)
                        .collect::<Vec<_>>()
                        .join("\n\n");
                }
                SectionText {
                    id: s.id.clone(),
                    level: s.level,
                    number: s.number.clone(),
                    title: s.title.clone(),
                    text,
                }
            })
            .collect()
    }

    /// One section's text by id.
    pub fn section_text(&self, id: &str, o: TextOptions) -> Option<SectionText> {
        self.section_texts(o).into_iter().find(|s| s.id == id)
    }

    /// The whole paper as one string: `# title` headings and paragraphs.
    pub fn plain_text(&self, o: TextOptions) -> String {
        let mut out = String::new();
        if !self.meta.title.is_empty() {
            out.push_str(&self.meta.title);
            out.push_str("\n\n");
        }
        for s in self.section_texts(o) {
            let head = match &s.number {
                Some(n) => format!("{} {n} {}", "#".repeat(s.level as usize), s.title),
                None => format!("{} {}", "#".repeat(s.level as usize), s.title),
            };
            out.push_str(head.trim_end());
            out.push_str("\n\n");
            if !s.text.is_empty() {
                out.push_str(&s.text);
                out.push_str("\n\n");
            }
        }
        out.trim_end().to_string()
    }

    /// `(asset id, caption)` for every captioned asset.
    pub fn captions(&self) -> Vec<(String, String)> {
        self.assets
            .iter()
            .filter_map(|a| a.caption.clone().map(|c| (a.id.clone(), c)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Section};

    #[test]
    fn section_texts_with_captions_and_without_math() {
        let mut c = AssetCatalog::default();
        c.meta.title = "T".into();
        c.sections.push(Section {
            id: "s1".into(),
            level: 1,
            number: Some("1".into()),
            title: "Intro".into(),
            paragraphs: vec!["Let $x$ be a knowledge base.".into(), "Second.".into()],
            ..Default::default()
        });
        let mut f = Asset::new("fig1", AssetKind::Figure);
        f.number = Some("1".into());
        f.caption = Some("Overview.".into());
        f.section = Some("s1".into());
        c.assets.push(f);
        let t = c.section_texts(TextOptions {
            captions: true,
            math: false,
        });
        assert_eq!(
            t[0].text,
            "Let be a knowledge base.\n\nSecond.\n\nFig. 1. Overview."
        );
        assert_eq!(
            c.plain_text(TextOptions::default()),
            "T\n\n# 1 Intro\n\nLet $x$ be a knowledge base.\n\nSecond."
        );
        assert_eq!(
            c.captions(),
            vec![("fig1".to_string(), "Overview.".to_string())]
        );
    }
}
