//! The author's tone library: rewrites he liked (👍), shared by every app, in his private
//! shared folder `private/shared/polish/liked.jsonl` (through lore-home). Read back only from
//! there — never from `private-example`, a cache or the public repo — and offered to later
//! polish calls as examples, the most similar first, bounded.

use std::collections::BTreeSet;
use std::path::PathBuf;

use lore_home::Home;
use serde::{Deserialize, Serialize};

pub const FILE: &str = "polish/liked.jsonl";
/// Examples per call and characters per example side.
pub const EXAMPLES: usize = 4;
pub const EXAMPLE_CHARS: usize = 600;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Liked {
    pub before: String,
    pub after: String,
    pub instruction: String,
    pub app: String,
    pub time: String,
}

/// `private/shared/polish/liked.jsonl`, None without a private folder.
pub fn path(home: &Home) -> Option<PathBuf> {
    home.shared_dir().map(|d| d.join(FILE))
}

pub fn add(home: &Home, item: &Liked) -> Result<PathBuf, String> {
    use std::io::Write;
    let p = path(home).ok_or("沒有私人資料夾（private/shared），讚無法保存")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    writeln!(f, "{}", serde_json::to_string(item).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

pub fn read(home: &Home) -> Vec<Liked> {
    let Some(p) = path(home) else { return vec![] };
    std::fs::read_to_string(p).map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

fn bigrams(s: &str) -> BTreeSet<(char, char)> {
    let c: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect();
    c.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Character-bigram overlap (works for Chinese and English alike).
pub fn similarity(a: &str, b: &str) -> f64 {
    let (x, y) = (bigrams(a), bigrams(b));
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }
    x.intersection(&y).count() as f64 / x.union(&y).count() as f64
}

fn cut(s: &str) -> String {
    if s.chars().count() <= EXAMPLE_CHARS { s.to_string() } else { s.chars().take(EXAMPLE_CHARS).collect::<String>() + "…" }
}

/// Up to `n` liked rewrites for `text`, the most similar first (newer first on a tie).
pub fn examples(all: &[Liked], text: &str, n: usize) -> Vec<Liked> {
    let mut v: Vec<(f64, usize, &Liked)> = all.iter().enumerate().map(|(i, l)| (similarity(&l.before, text), i, l)).collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    v.into_iter().take(n).map(|(_, _, l)| Liked { before: cut(&l.before), after: cut(&l.after), ..l.clone() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_only_under_the_private_shared_folder_and_ranked_by_similarity() {
        let dir = tempfile::tempdir().unwrap();
        // a private-example tone library is never read
        std::fs::create_dir_all(dir.path().join("private-example/shared/polish")).unwrap();
        std::fs::write(dir.path().join("private-example/shared/polish/liked.jsonl"), "{\"before\":\"example\",\"after\":\"x\"}\n").unwrap();
        let home = Home::at(dir.path());
        assert!(read(&home).is_empty());
        for (b, a) in [("The model converges fast.", "The model converges quickly."), ("我們提出一個新方法。", "本文提出一種新方法。")] {
            add(&home, &Liked { before: b.into(), after: a.into(), instruction: "smoother".into(), app: "PaperComposer".into(), time: "t".into() }).unwrap();
        }
        let p = dir.path().join("private/shared/polish/liked.jsonl");
        assert_eq!(path(&home).unwrap(), p);
        assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 2);
        let ex = examples(&read(&home), "我們提出的方法", 1);
        assert_eq!(ex[0].after, "本文提出一種新方法。");
        let ex = examples(&read(&home), "The network converges fast", 2);
        assert_eq!(ex[0].before, "The model converges fast.");
        // no private folder: nothing to write into
        assert!(add(&Home::default(), &Liked::default()).is_err());
    }
}
