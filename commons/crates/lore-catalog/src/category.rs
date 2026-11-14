//! Categories for filtering, in two groups:
//! - **type** (`類型`, exact): from the kind / unit kind ([`type_of`]);
//! - **use** (`用途`, inferred): a table of rules ([`RULES`]) over a few
//!   facts (names, download host, first-pages text, page count, image size,
//!   project markers). 證件 needs a name keyword (shape and scan only add
//!   confidence to it). Each rule gives a confidence; an item belongs to a use
//!   category when the best rule reaches [`INCLUDE_MIN`]. Rules are a first
//!   pass (wave 3 adds a local model); the author's own choice
//!   (`edits/<id>.json` field `categories`) replaces them.
//!
//! Add a use category: a line in [`USES`] and its rules in [`RULES`].

use serde::Serialize;

pub const GROUP_TYPE: &str = "type";
pub const GROUP_USE: &str = "use";

/// (id, label) of the type categories.
pub const TYPES: [(&str, &str); 11] = [
    ("pdf", "PDF"),
    ("document", "文件"),
    ("slides", "簡報"),
    ("spreadsheet", "試算表"),
    ("image", "圖片"),
    ("video", "影片"),
    ("archive", "壓縮檔"),
    ("code", "程式碼"),
    ("project", "專案"),
    ("dataset", "資料集"),
    ("package", "套件"),
];

/// (id, label) of the use categories.
pub const USES: [(&str, &str); 5] = [("paper", "論文"), ("id-doc", "證件"), ("homework", "作業"), ("ticket", "票券收據"), ("secret", "機密")];

/// Use categories whose items are sensitive (thumbnails blurred, never sent anywhere).
pub const SENSITIVE: [&str; 2] = ["id-doc", "secret"];

/// A rule hit at or above this counts as membership.
pub const INCLUDE_MIN: f32 = 0.5;

pub fn label(id: &str) -> Option<&'static str> {
    TYPES.iter().chain(USES.iter()).find(|(i, _)| *i == id).map(|(_, l)| *l)
}

pub fn group(id: &str) -> Option<&'static str> {
    if TYPES.iter().any(|(i, _)| *i == id) {
        Some(GROUP_TYPE)
    } else if USES.iter().any(|(i, _)| *i == id) || custom_name(id).is_some() {
        Some(GROUP_USE)
    } else {
        None
    }
}

/// Prefix of a use category the author made up: `u:<name>` (shown as `<name>`).
pub const CUSTOM_PREFIX: &str = "u:";
/// Longest custom category name, in characters.
pub const CUSTOM_MAX: usize = 24;

/// The name of a custom use category (`u:<name>`), when the id is a valid one: 1 to
/// [`CUSTOM_MAX`] characters, trimmed, no comma (ids travel in `cat=a,b`) and no control characters.
pub fn custom_name(id: &str) -> Option<&str> {
    let name = id.strip_prefix(CUSTOM_PREFIX)?;
    let n = name.chars().count();
    (n >= 1 && n <= CUSTOM_MAX && name.trim() == name && !name.chars().any(|c| c == ',' || c.is_control())).then_some(name)
}

/// The type category of a kind (`None` for audio, fonts, installers, other…).
pub fn type_of(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "pdf" => "pdf",
        "document" | "text" | "ebook" | "latex" => "document",
        "slides" => "slides",
        "spreadsheet" => "spreadsheet",
        "image" => "image",
        "video" => "video",
        "archive" => "archive",
        "code" => "code",
        "project" => "project",
        "dataset" => "dataset",
        "package" => "package",
        _ => return None,
    })
}

/// What the rules look at.
#[derive(Debug, Clone, Default)]
pub struct Facts<'a> {
    pub kind: &'a str,
    pub names: &'a [String],
    pub host: Option<&'a str>,
    /// First-pages text of a PDF.
    pub text: Option<&'a str>,
    pub pages: Option<u32>,
    pub dims: Option<(u32, u32)>,
    /// Project markers (`latex`, `git`, …).
    pub markers: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub id: String,
    pub confidence: f32,
    /// `rule` (or `author` for the author's own choice).
    pub source: String,
    pub reason: String,
}

pub struct Rule {
    pub category: &'static str,
    pub reason: &'static str,
    pub confidence: f32,
    pub test: fn(&Facts) -> bool,
}

/// Hosts of publishers and preprint servers.
pub const PAPER_HOSTS: [&str; 26] = [
    "springer", "ieee", "arxiv", "sciencedirect", "elsevier", "mdpi", "nature.com", "wiley", "acm.org", "tandfonline", "sagepub", "oup.com", "frontiersin",
    "plos", "biorxiv", "medrxiv", "openreview", "aclanthology", "thecvf", "neurips", "iop.org", "aps.org", "jstor", "ncbi.nlm", "pubmed", "editorialmanager",
];

const PAPER_MARKERS: [&str; 14] =
    ["abstract", "references", "journal", "proceedings", "conference", "doi", "keywords", "introduction", "ieee", "elsevier", "springer", "received", "accepted", "©"];

const ID_WORDS: [&str; 14] = ["身分證", "身份證", "護照", "passport", "學生證", "健保卡", "駕照", "居留證", "idcard", "id card", "id_card", "id-card", "residence", "residencecard"];
const SECRET_WORDS: [&str; 16] = [
    "token", "password", "passwords", "passwd", "密碼", "密鑰", "金鑰", "recoverycodes", "backupcodes", "credentials", "secret", "secrets", "apikey", "privatekey", "idrsa", "seedphrase",
];
const SECRET_EXTS: [&str; 7] = ["pem", "key", "p12", "pfx", "kdbx", "keychain", "ovpn"];

/// A file whose name says it holds a password, a token or a key (a paper or deck about tokens
/// is not one). Such a file is never read, never sent anywhere and files under `Personal/Secrets`.
pub fn secret_name(kind: &str, name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|x| x.1.to_lowercase()).unwrap_or_default();
    SECRET_EXTS.contains(&ext.as_str())
        || (!matches!(kind, "pdf" | "slides" | "project" | "dataset" | "latex") && name_has(&[name.to_string()], &SECRET_WORDS))
}

const HOMEWORK_WORDS: [&str; 5] = ["hw", "homework", "作業", "assignment", "習題"];
const TICKET_WORDS: [&str; 11] = ["車票", "ticket", "receipt", "收據", "invoice", "發票", "boarding", "機票", "e-ticket", "訂位", "boardingpass"];

/// Lower-case words of a name (split on non-alphanumerics and letter/digit changes).
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut w = String::new();
    let mut prev: Option<char> = None;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            if prev.is_some_and(|p| p.is_ascii_alphanumeric() && p.is_ascii_digit() != c.is_ascii_digit()) && !w.is_empty() {
                out.push(std::mem::take(&mut w));
            }
            w.push(c);
        } else if !w.is_empty() {
            out.push(std::mem::take(&mut w));
        }
        prev = Some(c);
    }
    if !w.is_empty() {
        out.push(w);
    }
    out
}

/// A keyword in any name: ASCII keywords as whole words (or joined words),
/// others as substrings.
pub fn name_has(names: &[String], keys: &[&str]) -> bool {
    names.iter().any(|n| {
        let lower = n.to_lowercase();
        let ws = words(n);
        let joined: Vec<String> = ws.windows(2).map(|p| p.concat()).collect();
        keys.iter().any(|k| {
            if k.is_ascii() {
                let k2: String = k.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                ws.contains(&k2) || joined.contains(&k2)
            } else {
                lower.contains(k)
            }
        })
    })
}

/// A DOI (`10.<4–9 digits>/<non-space>`) in the text.
pub fn has_doi(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(p) = text[i..].find("10.") {
        let s = i + p + 3;
        let digits = b[s..].iter().take_while(|c| c.is_ascii_digit()).count();
        if (4..=9).contains(&digits) && b.get(s + digits) == Some(&b'/') && b.get(s + digits + 1).is_some_and(|c| !c.is_ascii_whitespace()) {
            return true;
        }
        i = s;
    }
    false
}

/// An arXiv identifier (`arXiv:2101.01234`).
pub fn has_arxiv(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.match_indices("arxiv:").any(|(i, _)| {
        let rest = lower[i + 6..].trim_start().as_bytes();
        rest.len() >= 9 && rest[..4].iter().all(u8::is_ascii_digit) && rest[4] == b'.' && rest[5..9].iter().all(u8::is_ascii_digit)
    })
}

fn paper_markers(text: &str) -> usize {
    let lower = text.to_lowercase();
    PAPER_MARKERS.iter().filter(|m| lower.contains(*m)).count()
}

fn card_ratio(d: (u32, u32)) -> bool {
    let (w, h) = (d.0.max(d.1) as f32, d.0.min(d.1) as f32);
    h >= 100.0 && (w / h - 1.586).abs() < 0.04
}

/// The rule table.
pub static RULES: &[Rule] = &[
    Rule { category: "paper", reason: "DOI", confidence: 0.9, test: |f| f.kind == "pdf" && f.text.is_some_and(has_doi) },
    Rule { category: "paper", reason: "arXiv id", confidence: 0.9, test: |f| f.kind == "pdf" && f.text.is_some_and(has_arxiv) },
    Rule { category: "paper", reason: "journal markers", confidence: 0.7, test: |f| f.kind == "pdf" && f.text.is_some_and(|t| paper_markers(t) >= 4) },
    Rule {
        category: "paper",
        reason: "publisher host",
        confidence: 0.8,
        test: |f| f.kind == "pdf" && f.host.is_some_and(|h| PAPER_HOSTS.iter().any(|p| h.contains(p))),
    },
    Rule { category: "paper", reason: "LaTeX project", confidence: 0.6, test: |f| f.kind == "project" && f.markers.iter().any(|m| m == "latex") },
    Rule { category: "id-doc", reason: "name", confidence: 0.9, test: |f| f.kind != "project" && f.kind != "dataset" && name_has(f.names, &ID_WORDS) },
    // Shape alone never makes an ID document (figures are often card-shaped):
    // it only adds confidence to a name that already says so.
    Rule {
        category: "id-doc",
        reason: "name + card-sized image",
        confidence: 0.95,
        test: |f| f.kind == "image" && f.dims.is_some_and(card_ratio) && name_has(f.names, &ID_WORDS),
    },
    Rule {
        category: "id-doc",
        reason: "name + single-page scan",
        confidence: 0.95,
        test: |f| f.kind == "pdf" && f.pages == Some(1) && f.text.is_some_and(|t| t.trim().chars().count() < 20) && name_has(f.names, &ID_WORDS),
    },
    Rule { category: "secret", reason: "name", confidence: 0.9, test: |f| f.names.iter().any(|n| secret_name(f.kind, n)) },
    Rule { category: "homework", reason: "name", confidence: 0.7, test: |f| f.kind != "dataset" && name_has(f.names, &HOMEWORK_WORDS) },
    Rule { category: "ticket", reason: "name", confidence: 0.7, test: |f| f.kind != "dataset" && f.kind != "project" && name_has(f.names, &TICKET_WORDS) },
];

/// Every rule hit, best per category first.
pub fn rules(f: &Facts) -> Vec<Hit> {
    let mut best: Vec<Hit> = Vec::new();
    for r in RULES {
        if !(r.test)(f) {
            continue;
        }
        match best.iter_mut().find(|h| h.id == r.category) {
            Some(h) if h.confidence >= r.confidence => {}
            Some(h) => {
                h.confidence = r.confidence;
                h.reason = r.reason.into();
            }
            None => best.push(Hit { id: r.category.into(), confidence: r.confidence, source: "rule".into(), reason: r.reason.into() }),
        }
    }
    best
}

/// Use categories the rules put this item in.
pub fn members(hits: &[Hit]) -> Vec<String> {
    hits.iter().filter(|h| h.confidence >= INCLUDE_MIN).map(|h| h.id.clone()).collect()
}

/// A filter: `cat=a,b,c` → OR within a group, AND across groups. Unknown ids are ignored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub types: Vec<String>,
    pub uses: Vec<String>,
}

impl Filter {
    pub fn parse(s: Option<&str>) -> Filter {
        let mut f = Filter::default();
        for id in s.unwrap_or_default().split(',').map(str::trim).filter(|x| !x.is_empty()) {
            match group(id) {
                Some(GROUP_TYPE) => f.types.push(id.into()),
                Some(GROUP_USE) => f.uses.push(id.into()),
                _ => {}
            }
        }
        f
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty() && self.uses.is_empty()
    }

    /// Does an item with these categories pass?
    pub fn accepts(&self, cats: &[String]) -> bool {
        (self.types.is_empty() || self.types.iter().any(|t| cats.contains(t))) && (self.uses.is_empty() || self.uses.iter().any(|u| cats.contains(u)))
    }
}
