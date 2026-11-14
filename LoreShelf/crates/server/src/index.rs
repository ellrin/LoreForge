//! The in-memory index over the catalog: one item per content record and per
//! unit (project, package, dataset), with the features used for neighbours.
//!
//! No tags exist yet, so similarity is a weighted sum of five factors, each
//! in 0..1, computed by brute force (a few thousand items):
//! - `name`: cosine of IDF-weighted name tokens (separators, camelCase,
//!   letters / digits, CJK bigrams; the extension is not a token);
//! - `time`: mean over the axes both items have (modified, downloaded,
//!   document created, created) of `1 − ln(1+|Δdays|) / ln(1+3650)`;
//! - `kind`: 1 same extension, 0.5 same kind;
//! - `host`: 1 same download host;
//! - `folder`: 1 when they share a parent folder (same project / download batch).
//!
//! `parts` = each factor's share of the score (what the direction readout shows).

use lore_catalog::category::{self, Facts, Filter, Hit};
use lore_catalog::extract::Extract;
use lore_catalog::project::{self, UnitRecord};
use lore_catalog::record::{FileRecord, TimePoint, axis};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub const W_NAME: f64 = 0.40;
pub const W_TIME: f64 = 0.25;
pub const W_KIND: f64 = 0.15;
pub const W_HOST: f64 = 0.10;
pub const W_FOLDER: f64 = 0.10;
/// A dataset named in a paper (and the paper seen from the dataset).
pub const W_MENTION: f64 = 0.50;
/// Shared labels: what two files are about counts more than how they are named.
pub const W_LABEL: f64 = 0.80;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Times {
    pub created: Option<String>,
    pub modified: Option<String>,
    pub downloaded: Option<String>,
    pub opened: Option<String>,
    pub doc_created: Option<String>,
}

/// What the list shows (no paths).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub ext: String,
    pub size: u64,
    pub times: Times,
    pub download_host: Option<String>,
    pub seen_count: usize,
    /// "Same document, different copies": items whose names differ only by a
    /// copy suffix (` (1)`, ` copy`, `-2` when the plain name exists), case or
    /// extension share a group. Units are their own group.
    pub group: String,
    pub group_size: usize,
    /// Type and use categories (`lore_catalog::category`).
    pub categories: Vec<String>,
    /// In a sensitive category (證件): the UI blurs it.
    pub sensitive: bool,
    /// A cloud model may read it (author, 2026-11-13): an academic paper (decided here, by the
    /// rules or the author) or a slide deck — never a sensitive item. Everything else: local only.
    pub cloud_ok: bool,
    /// Its research topic and 3–5 English labels (`filing_ai`), the broadest first; empty until
    /// a model has looked at it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// One item plus what only the detail and the similarity need.
#[derive(Debug, Clone)]
pub struct Entry {
    pub item: Item,
    pub paths: Vec<String>,
    pub description: String,
    /// Use-category evidence (rules, or the author's choice).
    pub hits: Vec<Hit>,
    /// Dataset ↔ paper mentions (indices).
    pub mentions: HashSet<usize>,
    /// First-pages text (PDFs): searched, never sent whole.
    pub text: String,
    tokens: Vec<(usize, f64)>, // (token id, weight), L2-normalised
    days: [Option<f64>; 4],
    folders: HashSet<String>,
    /// (label key, weight), sorted by key, L2-normalised: rare labels weigh more.
    labs: Vec<(String, f64)>,
}

impl Entry {
    /// A bare entry with some times (tests).
    pub fn synthetic(id: &str, downloaded: Option<&str>, doc_created: Option<&str>, created: &str) -> Entry {
        let times = Times { created: Some(created.into()), modified: None, downloaded: downloaded.map(Into::into), opened: None, doc_created: doc_created.map(Into::into) };
        Entry {
            item: Item { id: id.into(), kind: "pdf".into(), name: format!("{id}.pdf"), ext: "pdf".into(), size: 1, times, download_host: None, seen_count: 1, group: String::new(), group_size: 1, categories: vec!["pdf".into()], sensitive: false, cloud_ok: false, labels: vec![] },
            paths: vec![],
            description: String::new(),
            hits: vec![],
            mentions: HashSet::new(),
            text: String::new(),
            tokens: vec![],
            days: [None; 4],
            folders: HashSet::new(),
            labs: vec![],
        }
    }
}

#[derive(Debug, Default)]
pub struct Index {
    pub entries: Vec<Entry>,
    pub by_id: HashMap<String, usize>,
    /// group → entry indices.
    pub groups: HashMap<String, Vec<usize>>,
    /// The whole text of the files that have one (`fulltext.rs`; empty until `loreshelf extract` ran).
    /// Shared between rebuilds that keep the same entries (an edit of a category).
    pub ft: std::sync::Arc<crate::fulltext::FullIndex>,
}

fn is_unit(kind: &str) -> bool {
    matches!(kind, "project" | "package" | "dataset")
}

/// Only documents group by name across copies and formats; images, code and
/// everything else group only by identical content (one record already).
pub const DOC_FAMILY: [&str; 9] = ["pdf", "doc", "docx", "pptx", "key", "pages", "odt", "rtf", "tex"];
/// Stems that never group by name.
pub const GENERIC_STEMS: [&str; 12] = ["main", "index", "readme", "untitled", "document", "file", "output", "result", "test", "new", "copy", "draft"];
/// Generic when followed by digits only (`img001`, `fig_3`, `figure 2`).
pub const GENERIC_NUMBERED: [&str; 4] = ["image", "img", "fig", "figure"];
/// Generic as a prefix (`Screenshot 2026-…`, `截圖 …`, `未命名 3`).
pub const GENERIC_PREFIXES: [&str; 4] = ["screenshot", "screen shot", "截圖", "未命名"];

/// A stem too generic to say two files are the same document.
pub fn generic_stem(stem: &str) -> bool {
    let s = stem.trim().to_lowercase();
    if GENERIC_STEMS.contains(&s.as_str()) || GENERIC_PREFIXES.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    let compact: String = s.chars().filter(|c| c.is_alphanumeric()).collect();
    GENERIC_NUMBERED.iter().any(|p| compact.strip_prefix(p).is_some_and(|r| r.chars().all(|c| c.is_ascii_digit())))
}

/// Lower-case stem without the extension and without unambiguous copy
/// suffixes (` (1)`, `(2)`, ` copy`, ` copy 2`, ` - copy`, ` 副本`, ` 拷貝`).
pub fn base_stem(name: &str) -> String {
    let mut s = stem(name).trim().to_lowercase();
    loop {
        let before = s.clone();
        for suf in [" - copy", " copy", " 副本", " 拷貝", "-copy", "_copy"] {
            if let Some(x) = s.strip_suffix(suf) {
                s = x.trim_end().to_string();
            }
        }
        // " copy 2"
        if let Some((head, n)) = s.rsplit_once(" copy ")
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            s = head.trim_end().to_string();
        }
        // "(1)" / " (12)"
        if s.ends_with(')')
            && let Some(open) = s.rfind('(')
            && s[open + 1..s.len() - 1].chars().all(|c| c.is_ascii_digit())
            && s.len() - open > 2
        {
            s = s[..open].trim_end().to_string();
        }
        if s == before {
            return s;
        }
    }
}

/// The group key: [`base_stem`], then `-N` / `_N` / ` N` (N ≤ 2 digits)
/// dropped only when the plain stem also exists (so `lecture-3` stays
/// apart unless there is a `lecture`). `None` for an empty stem.
pub fn group_key(name: &str, stems: &HashSet<String>) -> Option<String> {
    let s = base_stem(name);
    if s.is_empty() {
        return None;
    }
    for sep in ['-', '_', ' '] {
        if let Some((head, n)) = s.rsplit_once(sep)
            && (1..=2).contains(&n.len())
            && n.chars().all(|c| c.is_ascii_digit())
            && stems.contains(head.trim_end())
        {
            return Some(head.trim_end().to_string());
        }
    }
    Some(s)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Parts {
    /// Shared labels (what the files are about).
    pub label: f64,
    pub name: f64,
    pub time: f64,
    pub kind: f64,
    pub host: f64,
    pub folder: f64,
    pub mention: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Neighbor {
    pub id: String,
    pub score: f64,
    pub layer: u32,
    /// The group this entry stands for, and all its members (the best one is `id`).
    pub group: String,
    pub members: Vec<String>,
    /// The item this one was reached from (the focus for layer 1).
    pub via: String,
    pub parts: Parts,
    /// The labels it shares with `via`, the most telling first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<String>,
}

/// A label's key: lower case, letters and digits only (`Fuzzy-Systems` = `fuzzy systems`).
pub fn label_key(l: &str) -> String {
    l.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.').filter(|(s, _)| !s.is_empty()).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').filter(|(s, e)| !s.is_empty() && e.len() <= 8).map(|(s, _)| s).unwrap_or(name)
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff)
}

/// Name tokens: lower-case words split on separators, camelCase and
/// letter/digit changes; CJK runs as bigrams (a lone character as itself).
pub fn tokens(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut cjk = Vec::new();
    let flush_cjk = |cjk: &mut Vec<char>, out: &mut Vec<String>| {
        if cjk.len() == 1 {
            out.push(cjk[0].to_string());
        }
        for w in cjk.windows(2) {
            out.push(w.iter().collect());
        }
        cjk.clear();
    };
    let flush_word = |word: &mut String, out: &mut Vec<String>| {
        if word.chars().count() >= 2 {
            out.push(word.to_lowercase());
        }
        word.clear();
    };
    let mut prev: Option<char> = None;
    for c in stem(name).chars() {
        if is_cjk(c) {
            flush_word(&mut word, &mut out);
            cjk.push(c);
        } else if c.is_alphanumeric() {
            flush_cjk(&mut cjk, &mut out);
            if let Some(p) = prev {
                let camel = p.is_lowercase() && c.is_uppercase();
                let digit_change = p.is_ascii_digit() != c.is_ascii_digit() && p.is_alphanumeric();
                if camel || digit_change {
                    flush_word(&mut word, &mut out);
                }
            }
            word.push(c);
        } else {
            flush_word(&mut word, &mut out);
            flush_cjk(&mut cjk, &mut out);
        }
        prev = Some(c);
    }
    flush_word(&mut word, &mut out);
    flush_cjk(&mut cjk, &mut out);
    out
}

fn pick(times: &[TimePoint], ax: &str, latest: bool) -> Option<String> {
    let it = times.iter().filter(|t| t.axis == ax).map(|t| t.value.clone());
    if latest { it.max() } else { it.min() }
}

fn to_days(v: &Option<String>) -> Option<f64> {
    let t = chrono_like_parse(v.as_deref()?)?;
    Some(t as f64 / 86400.0)
}

/// Seconds since the epoch of an RFC 3339 UTC value written by lore-catalog
/// (`YYYY-MM-DDTHH:MM:SSZ`).
fn chrono_like_parse(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // days from civil (Howard Hinnant)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se)
}

fn times_of(t: &[TimePoint]) -> Times {
    Times {
        created: pick(t, axis::FS_CREATED, false),
        modified: pick(t, axis::FS_MODIFIED, true),
        downloaded: pick(t, axis::DOWNLOADED, false),
        opened: pick(t, axis::LAST_OPENED, true),
        doc_created: pick(t, axis::DOC_CREATED, false),
    }
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "pdf" => "PDF",
        "image" => "圖片",
        "video" => "影片",
        "audio" => "音訊",
        "document" => "文件",
        "slides" => "簡報",
        "spreadsheet" => "試算表",
        "text" => "文字",
        "latex" => "LaTeX",
        "archive" => "壓縮檔",
        "installer" => "安裝檔",
        "ebook" => "電子書",
        "data" => "資料",
        "code" => "程式碼",
        "font" => "字型",
        "project" => "專案資料夾",
        "package" => "套件",
        "dataset" => "資料集",
        _ => "檔案",
    }
}

fn parent(p: &str) -> String {
    Path::new(p).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default()
}

struct Raw {
    item: Item,
    paths: Vec<String>,
    description: String,
    words: Vec<String>,
    hits: Vec<Hit>,
    /// First-pages text (PDFs), for dataset mentions.
    text: String,
}

/// Type category + use categories (the author's choice replaces the rules).
fn categorize(kind: &str, facts: &Facts, author: Option<Vec<String>>) -> (Vec<String>, Vec<Hit>) {
    let hits = match author {
        Some(ids) => ids
            .into_iter()
            .filter(|i| category::group(i) == Some(category::GROUP_USE))
            .map(|id| Hit { id, confidence: 1.0, source: "author".into(), reason: "作者指定".into() })
            .collect(),
        None => category::rules(facts),
    };
    let mut cats: Vec<String> = category::type_of(kind).map(|t| vec![t.to_string()]).unwrap_or_default();
    cats.extend(category::members(&hits));
    (cats, hits)
}

/// Papers and slide decks may be read by a cloud model; never a sensitive item.
pub fn cloud_ok(cats: &[String]) -> bool {
    !sensitive(cats) && cats.iter().any(|c| c == "paper" || c == "slides")
}

fn sensitive(cats: &[String]) -> bool {
    cats.iter().any(|c| category::SENSITIVE.contains(&c.as_str()))
}

/// The author's `categories` from `edits/<id>.json`.
fn author_categories(catalog: &Path, id: &str) -> Option<Vec<String>> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(catalog.join("edits").join(format!("{id}.json"))).ok()?).ok()?;
    Some(v.get("categories")?.as_array()?.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
}

/// Words a dataset folder name may end with besides its own name
/// (`chexpert-v1.0-small`, `padchest_images`): the first word alone is then a key.
const QUALIFIERS: [&str; 18] = ["v", "small", "large", "mini", "full", "sample", "subset", "release", "public", "data", "dataset", "images", "image", "png", "jpg", "train", "test", "master"];
/// Names that are never dataset keys on their own.
const GENERIC: [&str; 18] = ["dataset", "datasets", "data", "images", "image", "train", "test", "valid", "files", "download", "downloads", "archive", "sample", "small", "version", "final", "json", "output"];
/// Words near a mention that make it a dataset mention.
const CONTEXT: [&str; 9] = ["dataset", "database", "data set", "cohort", "collection", "benchmark", "challenge", "images from", "corpus"];

/// Lower-case words of a text (letters and digits; everything else separates).
fn norm_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = true;
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out
}

/// Keys a dataset is mentioned by, as word lists: its whole folder name
/// (`mimic-cxr` → `mimic cxr`, also matched joined as `mimiccxr`), and its
/// first word when the rest is only version / size / format words
/// (`chexpert-v1.0-small` → `chexpert`). Generic or short names give none.
pub fn dataset_keys(name: &str) -> Vec<Vec<String>> {
    let words: Vec<String> = norm_words(name).split_whitespace().map(str::to_string).collect();
    let joined: String = words.concat();
    let mut keys = Vec::new();
    if joined.chars().count() >= 5 && !GENERIC.contains(&joined.as_str()) && !joined.chars().all(|c| c.is_ascii_digit()) {
        keys.push(words.clone());
    }
    if words.len() > 1
        && words[0].chars().count() >= 5
        && !GENERIC.contains(&words[0].as_str())
        && words[1..].iter().all(|w| QUALIFIERS.contains(&w.as_str()) || w.chars().all(|c| c.is_ascii_digit()) || (w.starts_with('v') && w[1..].chars().all(|c| c.is_ascii_digit())))
    {
        keys.push(vec![words[0].clone()]);
    }
    keys
}

/// Every word of the key is everyday English (lore-words: SCOWL common /
/// familiar, academic, function words, with inflections): such a name in a
/// paper's text says nothing (`visualization`, `adjusted masks`).
pub fn common_key(key: &[String]) -> bool {
    static WORDS: std::sync::OnceLock<lore_words::Filters> = std::sync::OnceLock::new();
    let f = WORDS.get_or_init(lore_words::Filters::default);
    key.iter().all(|w| w.chars().all(|c| c.is_ascii_digit()) || f.is_general(w) || f.is_familiar(w))
}

/// Is the dataset named in this paper? In its file name, or — unless the
/// name is everyday English ([`common_key`]) — in its text with a dataset
/// word (dataset, database, cohort, benchmark…) within 120 characters.
pub fn mentions_in(paper_name: &str, paper_text: &str, keys: &[Vec<String>]) -> bool {
    let name = norm_words(paper_name);
    let text = norm_words(paper_text);
    keys.iter().any(|k| {
        let spaced = k.join(" ");
        let joined = k.concat();
        let in_name = |n: &str| format!(" {n} ").contains(&format!(" {spaced} ")) || format!(" {n} ").contains(&format!(" {joined} "));
        if in_name(&name) {
            return true;
        }
        if common_key(k) {
            return false;
        }
        let t = format!(" {text} ");
        [format!(" {spaced} "), format!(" {joined} ")].iter().any(|needle| {
            t.match_indices(needle.as_str()).any(|(i, _)| {
                let lo = t.floor_char_boundary(i.saturating_sub(120));
                let hi = t.ceil_char_boundary((i + needle.len() + 120).min(t.len()));
                let w = &t[lo..hi];
                CONTEXT.iter().any(|c| w.contains(c))
            })
        })
    })
}

fn from_file(r: FileRecord, ex: Option<Extract>, author: Option<Vec<String>>) -> Raw {
    let name = r.names.first().cloned().unwrap_or_default();
    let times = times_of(&r.times);
    let host = r.download.as_ref().and_then(|d| d.urls.first()).and_then(|u| project::url_host(u));
    let paths: Vec<String> = r.seen_at.iter().map(|s| s.path.clone()).collect();
    let mut desc = vec![kind_label(&r.kind).to_string(), lore_catalog::report::human(r.size)];
    if paths.len() > 1 {
        desc.push(format!("{} 個位置", paths.len()));
    }
    if let Some(h) = &host {
        desc.push(format!("下載自 {h}"));
    }
    if let Some(d) = &times.doc_created {
        desc.push(format!("文件日期 {}", &d[..10.min(d.len())]));
    }
    let mut words: Vec<String> = Vec::new();
    for n in &r.names {
        words.extend(tokens(n));
    }
    let ex = ex.unwrap_or_default();
    if let Some(p) = ex.pages {
        desc.push(format!("{p} 頁"));
    }
    if let (Some(w), Some(h)) = (ex.width, ex.height) {
        desc.push(format!("{w}×{h}"));
    }
    let facts = Facts {
        kind: &r.kind,
        names: &r.names,
        host: host.as_deref(),
        text: ex.text.as_deref(),
        pages: ex.pages,
        dims: ex.width.zip(ex.height),
        markers: &[],
    };
    let (categories, hits) = categorize(&r.kind, &facts, author);
    let text = ex.text.unwrap_or_default();
    Raw {
        item: Item {
            id: r.sha256.clone(),
            kind: r.kind.clone(),
            ext: ext_of(&name),
            name,
            size: r.size,
            times,
            download_host: host,
            seen_count: paths.len(),
            group: String::new(),
            group_size: 1,
            sensitive: sensitive(&categories),
            cloud_ok: cloud_ok(&categories),
            categories,
            labels: vec![],
        },
        paths,
        description: desc.join(" · "),
        words,
        hits,
        text,
    }
}

fn from_unit(u: UnitRecord, author: Option<Vec<String>>) -> Raw {
    let times = times_of(&u.times);
    let paths: Vec<String> = u.seen_at.iter().map(|s| s.path.clone()).collect();
    let ext = match u.kind.as_str() {
        project::KIND_PACKAGE => u.markers.first().cloned().unwrap_or_default(),
        project::KIND_DATASET => u.main_ext.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let mut desc = vec![kind_label(&u.kind).to_string()];
    if !u.markers.is_empty() && u.kind == project::KIND_PROJECT {
        desc[0] = format!("{}（{}）", desc[0], u.markers.join(", "));
    }
    if let Some(f) = u.files {
        desc.push(format!("{f} 個檔"));
    }
    if let Some(b) = u.bytes {
        desc.push(lore_catalog::report::human(b));
    }
    if u.kind == project::KIND_DATASET && !ext.is_empty() {
        desc.push(format!("主要 .{ext}"));
    }
    let hosts: Vec<String> = u.remotes.iter().filter_map(|r| project::url_host(r)).collect();
    if !hosts.is_empty() {
        desc.push(format!("遠端 {}", hosts.join(", ")));
    }
    let first_git = u.times.iter().find(|t| t.axis == axis::GIT_FIRST).map(|t| t.value.clone());
    let last_git = u.times.iter().find(|t| t.axis == axis::GIT_LAST).map(|t| t.value.clone());
    if let (Some(a), Some(b)) = (first_git, last_git) {
        desc.push(format!("git {} → {}", &a[..10.min(a.len())], &b[..10.min(b.len())]));
    }
    let names = vec![u.name.clone()];
    let facts = Facts { kind: &u.kind, names: &names, markers: &u.markers, ..Facts::default() };
    let (categories, hits) = categorize(&u.kind, &facts, author);
    Raw {
        words: tokens(&u.name),
        item: Item {
            id: u.id.clone(),
            kind: u.kind.clone(),
            name: u.name.clone(),
            ext,
            size: u.bytes.unwrap_or(0),
            times,
            download_host: None,
            seen_count: paths.len(),
            group: String::new(),
            group_size: 1,
            sensitive: sensitive(&categories),
            cloud_ok: cloud_ok(&categories),
            categories,
            labels: vec![],
        },
        paths,
        description: desc.join(" · "),
        hits,
        text: String::new(),
    }
}

impl Index {
    /// Read every record of the catalog at `dir` (missing folders = empty);
    /// `extract` = the extract cache (`<cache>/LoreForge/shelf/extract`).
    pub fn load(dir: &Path, extract: &Path) -> Index {
        Index::load_with(dir, extract, None)
    }

    /// [`Index::load`], reusing `ft` (the whole-text index of the same entries) when given.
    pub fn load_with(dir: &Path, extract: &Path, ft: Option<std::sync::Arc<crate::fulltext::FullIndex>>) -> Index {
        let mut raws = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir.join("files")) {
            let mut names: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".json")).collect();
            names.sort();
            for n in names {
                if let Some(r) = FileRecord::load(&dir.join("files"), n.trim_end_matches(".json")) {
                    let ex = Extract::load(extract, &r.sha256);
                    let author = author_categories(dir, &r.sha256);
                    raws.push(from_file(r, ex, author));
                }
            }
        }
        for kind in [project::KIND_PROJECT, project::KIND_PACKAGE, project::KIND_DATASET] {
            for u in UnitRecord::load_all(&dir.join(project::dir_name(kind))) {
                let author = author_categories(dir, &u.id);
                raws.push(from_unit(u, author));
            }
        }
        let mut idx = Index::build(raws);
        idx.set_labels(&crate::filing_ai::shown_labels(dir));
        // the full text sits next to the extract cache (`<cache>/LoreForge/shelf/fulltext`)
        if let Some(ft) = ft {
            idx.ft = ft;
        } else if let Some(cache) = extract.parent() {
            let dir = lore_catalog::fulltext::FullText::dir(cache);
            let ids: Vec<(usize, String)> = idx.entries.iter().enumerate().filter(|(_, e)| !is_unit(&e.item.kind)).map(|(i, e)| (i, e.item.id.clone())).collect();
            idx.ft = std::sync::Arc::new(crate::fulltext::FullIndex::load(&dir, ids.into_iter()));
        }
        idx
    }

    fn build(raws: Vec<Raw>) -> Index {
        let mut vocab: HashMap<String, usize> = HashMap::new();
        let mut df: Vec<u32> = Vec::new();
        let docs: Vec<Vec<usize>> = raws
            .iter()
            .map(|r| {
                let mut ids: Vec<usize> = r
                    .words
                    .iter()
                    .map(|w| {
                        let n = vocab.len();
                        *vocab.entry(w.clone()).or_insert(n)
                    })
                    .collect();
                ids.sort_unstable();
                ids.dedup();
                for &i in &ids {
                    if i >= df.len() {
                        df.resize(i + 1, 0);
                    }
                    df[i] += 1;
                }
                ids
            })
            .collect();
        let n = raws.len().max(1) as f64;
        let mut entries = Vec::with_capacity(raws.len());
        let mut by_id = HashMap::new();
        // Dataset ↔ paper: a dataset's name in a paper's file name, or in its first pages next to a dataset word.
        let mut mentions: Vec<HashSet<usize>> = vec![HashSet::new(); raws.len()];
        let papers: Vec<usize> = raws.iter().enumerate().filter(|(_, r)| r.item.categories.iter().any(|c| c == "paper")).map(|(i, _)| i).collect();
        for (d, r) in raws.iter().enumerate().filter(|(_, r)| r.item.kind == project::KIND_DATASET) {
            let keys = dataset_keys(&r.item.name);
            if keys.is_empty() {
                continue;
            }
            for &p in &papers {
                if mentions_in(&raws[p].item.name, &raws[p].text, &keys) {
                    mentions[d].insert(p);
                    mentions[p].insert(d);
                }
            }
        }
        for ((mut raw, ids), mentions) in raws.into_iter().zip(docs).zip(mentions) {
            let mut v: Vec<(usize, f64)> = ids.iter().map(|&i| (i, (n / (1.0 + df[i] as f64)).ln() + 1.0)).collect();
            let norm = v.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
            if norm > 0.0 {
                for x in &mut v {
                    x.1 /= norm;
                }
            }
            let t = &raw.item.times;
            let days = [to_days(&t.modified), to_days(&t.downloaded), to_days(&t.doc_created), to_days(&t.created)];
            let folders = raw.paths.iter().map(|p| parent(p)).collect();
            by_id.insert(raw.item.id.clone(), entries.len());
            let text = std::mem::take(&mut raw.text);
            entries.push(Entry { item: raw.item, paths: raw.paths, description: raw.description, hits: raw.hits, mentions, text, tokens: v, days, folders, labs: vec![] });
        }
        let mut idx = Index { entries, by_id, groups: HashMap::new(), ft: Default::default() };
        idx.assign_groups();
        idx
    }

    fn assign_groups(&mut self) {
        let by_name = |e: &Entry| !is_unit(&e.item.kind) && DOC_FAMILY.contains(&e.item.ext.as_str());
        let stems: HashSet<String> = self.entries.iter().filter(|e| by_name(e)).map(|e| base_stem(&e.item.name)).collect();
        for i in 0..self.entries.len() {
            let key = if by_name(&self.entries[i]) { group_key(&self.entries[i].item.name, &stems).filter(|k| !generic_stem(k)) } else { None };
            let e = &mut self.entries[i];
            e.item.group = match key {
                Some(k) => format!("g-{}", &lore_catalog::hash::sha256_bytes(k.as_bytes())[..12]),
                None => format!("g-{}", e.item.id),
            };
        }
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, e) in self.entries.iter().enumerate() {
            groups.entry(e.item.group.clone()).or_default().push(i);
        }
        for members in groups.values() {
            for &i in members {
                self.entries[i].item.group_size = members.len();
            }
        }
        self.groups = groups;
    }

    /// An index over ready entries (tests; groups recomputed).
    pub fn from_entries(entries: Vec<Entry>) -> Index {
        let by_id = entries.iter().enumerate().map(|(i, e)| (e.item.id.clone(), i)).collect();
        let mut idx = Index { entries, by_id, groups: HashMap::new(), ft: Default::default() };
        idx.assign_groups();
        idx
    }

    /// Gives the items their labels (id → labels, the broadest first) and weighs them: a label
    /// few files carry tells more than one many do.
    pub fn set_labels(&mut self, labels: &HashMap<String, Vec<String>>) {
        let mut df: HashMap<String, f64> = HashMap::new();
        for e in &mut self.entries {
            e.item.labels = labels.get(&e.item.id).cloned().unwrap_or_default();
            for k in e.item.labels.iter().map(|l| label_key(l)).collect::<HashSet<_>>() {
                *df.entry(k).or_default() += 1.0;
            }
        }
        let n = self.entries.iter().filter(|e| !e.item.labels.is_empty()).count().max(1) as f64;
        for e in &mut self.entries {
            let mut v: Vec<(String, f64)> = e.item.labels.iter().map(|l| label_key(l)).collect::<std::collections::BTreeSet<_>>().into_iter().map(|k| {
                let w = (n / (1.0 + df[&k])).ln() + 1.0;
                (k, w)
            }).collect();
            let norm = v.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
            if norm > 0.0 {
                for x in &mut v {
                    x.1 /= norm;
                }
            }
            e.labs = v;
        }
    }

    /// The labels entries `a` and `b` share, as `b` writes them, the most telling first.
    fn shared_labels(&self, a: usize, b: usize) -> Vec<String> {
        let (a, b) = (&self.entries[a], &self.entries[b]);
        let mut out: Vec<(f64, &String)> = b.item.labels.iter().filter_map(|l| {
            let k = label_key(l);
            let wa = a.labs.iter().find(|(x, _)| *x == k)?.1;
            let wb = b.labs.iter().find(|(x, _)| *x == k)?.1;
            Some((wa * wb, l))
        }).collect();
        out.sort_by(|x, y| y.0.total_cmp(&x.0));
        out.into_iter().map(|(_, l)| l.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    /// (score, parts) of b seen from a.
    pub fn similarity(&self, i: usize, j: usize) -> (f64, Parts) {
        let (a, b) = (&self.entries[i], &self.entries[j]);
        let mention = if a.mentions.contains(&j) { 1.0 } else { 0.0 };
        let (base, p) = Index::pair(a, b);
        let parts = [p.name, p.time, p.kind, p.host, p.folder, p.label].map(|x| x * base);
        let score = base + W_MENTION * mention;
        let share = |x: f64| if score > 0.0 { (x / score * 1000.0).round() / 1000.0 } else { 0.0 };
        (score, Parts { label: share(parts[5]), name: share(parts[0]), time: share(parts[1]), kind: share(parts[2]), host: share(parts[3]), folder: share(parts[4]), mention: share(W_MENTION * mention) })
    }

    fn pair(a: &Entry, b: &Entry) -> (f64, Parts) {
        let name = {
            let (mut i, mut j, mut s) = (0, 0, 0.0);
            while i < a.tokens.len() && j < b.tokens.len() {
                match a.tokens[i].0.cmp(&b.tokens[j].0) {
                    std::cmp::Ordering::Equal => {
                        s += a.tokens[i].1 * b.tokens[j].1;
                        i += 1;
                        j += 1;
                    }
                    std::cmp::Ordering::Less => i += 1,
                    std::cmp::Ordering::Greater => j += 1,
                }
            }
            s
        };
        let span = (1.0f64 + 3650.0).ln();
        let ks: Vec<f64> = a.days.iter().zip(b.days.iter()).filter_map(|(x, y)| Some(1.0 - ((1.0 + (x.as_ref()? - y.as_ref()?).abs()).ln() / span).min(1.0))).collect();
        let time = if ks.is_empty() { 0.0 } else { ks.iter().sum::<f64>() / ks.len() as f64 };
        let kind = if !a.item.ext.is_empty() && a.item.ext == b.item.ext && a.item.kind == b.item.kind {
            1.0
        } else if a.item.kind == b.item.kind {
            0.5
        } else {
            0.0
        };
        let host = match (&a.item.download_host, &b.item.download_host) {
            (Some(x), Some(y)) if x == y => 1.0,
            _ => 0.0,
        };
        let folder = if a.folders.iter().any(|f| b.folders.contains(f)) { 1.0 } else { 0.0 };
        let label = {
            let (mut i, mut j, mut s) = (0, 0, 0.0);
            while i < a.labs.len() && j < b.labs.len() {
                match a.labs[i].0.cmp(&b.labs[j].0) {
                    std::cmp::Ordering::Equal => {
                        s += a.labs[i].1 * b.labs[j].1;
                        i += 1;
                        j += 1;
                    }
                    std::cmp::Ordering::Less => i += 1,
                    std::cmp::Ordering::Greater => j += 1,
                }
            }
            s
        };
        let w = [W_NAME * name, W_TIME * time, W_KIND * kind, W_HOST * host, W_FOLDER * folder, W_LABEL * label];
        let score: f64 = w.iter().sum();
        let share = |x: f64| if score > 0.0 { (x / score * 1000.0).round() / 1000.0 } else { 0.0 };
        (score, Parts { label: share(w[5]), name: share(w[0]), time: share(w[1]), kind: share(w[2]), host: share(w[3]), folder: share(w[4]), mention: 0.0 })
    }

    /// The `k` most similar items to entry `i` (never itself).
    fn top(&self, i: usize, k: usize, filter: &Filter) -> Vec<(usize, f64, Parts)> {
        let mut v: Vec<(usize, f64, Parts)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(j, b)| *j != i && filter.accepts(&b.item.categories))
            .map(|(j, _)| {
                let (s, p) = self.similarity(i, j);
                (j, s, p)
            })
            .filter(|(_, s, _)| *s > 0.0)
            .collect();
        v.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
        v.truncate(k);
        v
    }

    /// Layer 1 = the `k` nearest to the focus; layer n+1 = the `k` nearest to
    /// each item of layer n that are not already listed.
    ///
    /// One entry per group (its best-scoring member, with `members` = the
    /// whole group); the focus's own group (its copies) is not listed.
    /// Under a filter only matching items are listed.
    pub fn neighbors(&self, id: &str, k: usize, layers: u32, filter: &Filter) -> Option<Vec<Neighbor>> {
        let &focus = self.by_id.get(id)?;
        let mut seen: HashSet<&str> = HashSet::from([self.entries[focus].item.group.as_str()]);
        let mut out = Vec::new();
        let mut frontier = vec![focus];
        for layer in 1..=layers.max(1) {
            let mut next = Vec::new();
            for &f in &frontier {
                let mut taken = 0;
                for (j, score, parts) in self.top(f, usize::MAX, filter) {
                    if taken == k {
                        break;
                    }
                    let g = self.entries[j].item.group.as_str();
                    if seen.insert(g) {
                        taken += 1;
                        let members = self.groups.get(g).map(|m| m.iter().map(|&i| self.entries[i].item.id.clone()).collect()).unwrap_or_default();
                        out.push(Neighbor {
                            id: self.entries[j].item.id.clone(),
                            score: (score * 1000.0).round() / 1000.0,
                            layer,
                            group: g.to_string(),
                            members,
                            via: self.entries[f].item.id.clone(),
                            parts,
                            shared: self.shared_labels(f, j),
                        });
                        next.push(j);
                    }
                }
            }
            frontier = next;
        }
        Some(out)
    }

    /// Every entry (search).
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The matching item most similar to `from` (for a focus outside the filter).
    pub fn nearest(&self, from: &str, filter: &Filter) -> Option<(&Item, f64, Parts)> {
        let &i = self.by_id.get(from)?;
        self.top(i, 1, filter).into_iter().next().map(|(j, s, p)| (&self.entries[j].item, s, p))
    }

    /// Where the view starts: the most recently opened document (pdf, Word,
    /// slides, Pages / Keynote, LaTeX, or a project), else the newest PDF. Never
    /// an image or video, never a sensitive item.
    pub fn default_focus(&self, filter: &Filter) -> Option<&Item> {
        const DOC_EXTS: [&str; 7] = ["pdf", "doc", "docx", "pptx", "key", "pages", "tex"];
        let ok = |i: &&Item| !i.sensitive && i.kind != "image" && i.kind != "video" && filter.accepts(&i.categories);
        let items: Vec<&Item> = self.entries.iter().map(|e| &e.item).filter(ok).collect();
        let doc = |i: &&&Item| i.kind == "project" || DOC_EXTS.contains(&i.ext.as_str());
        // the document last opened or last modified, whichever is later (ISO times compare as text)
        let recent = |i: &&&Item| i.times.opened.clone().max(i.times.modified.clone());
        if let Some(i) = items.iter().filter(doc).filter(|i| recent(i).is_some()).max_by(|a, b| recent(a).cmp(&recent(b)).then(b.id.cmp(&a.id))) {
            return Some(i);
        }
        let newest = |i: &&&Item| i.times.downloaded.clone().or(i.times.modified.clone()).or(i.times.created.clone());
        items.iter().filter(|i| i.kind == "pdf").max_by(|a, b| newest(a).cmp(&newest(b)).then(b.id.cmp(&a.id))).copied()
    }

    /// The list, most recently modified first.
    pub fn list(&self, limit: usize, filter: &Filter) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.entries.iter().map(|e| &e.item).filter(|i| filter.accepts(&i.categories)).collect();
        v.sort_by(|a, b| b.times.modified.cmp(&a.times.modified).then(a.id.cmp(&b.id)));
        v.truncate(limit);
        v
    }

    /// Every category with its label, group and count.
    pub fn categories(&self) -> Vec<serde_json::Value> {
        category::TYPES
            .iter()
            .map(|c| (c, category::GROUP_TYPE))
            .chain(category::USES.iter().map(|c| (c, category::GROUP_USE)))
            .map(|((id, label), group)| {
                let count = self.entries.iter().filter(|e| e.item.categories.iter().any(|c| c == id)).count();
                serde_json::json!({ "id": id, "label": label, "group": group, "count": count })
            })
            .chain(self.custom_uses().into_iter().map(|(id, count)| {
                serde_json::json!({ "id": id, "label": category::custom_name(&id).unwrap_or_default(), "group": category::GROUP_USE, "count": count, "custom": true })
            }))
            .collect()
    }

    /// The use categories the author made up (`u:<name>`) that some item carries, with counts.
    fn custom_uses(&self) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for c in self.entries.iter().flat_map(|e| e.item.categories.iter()).filter(|c| category::custom_name(c).is_some()) {
            *m.entry(c.clone()).or_insert(0) += 1;
        }
        m
    }

    /// Kinds and counts (for a legend).
    pub fn kinds(&self) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for e in &self.entries {
            *m.entry(e.item.kind.clone()).or_insert(0) += 1;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_tokens() {
        assert_eq!(tokens("MyReport_v2-final.pdf"), vec!["my", "report", "final"]);
        assert_eq!(tokens("影像分割綜述.pdf"), vec!["影像", "像分", "分割", "割綜", "綜述"]);
        assert_eq!(tokens("lecture03 筆記.md"), vec!["lecture", "03", "筆記"]);
        assert_eq!(tokens("a.b"), Vec::<String>::new());
    }

    #[test]
    fn dataset_mentions() {
        let k = dataset_keys("CheXpert-v1.0-small");
        assert_eq!(k, vec![vec!["chexpert".to_string(), "v1".into(), "0".into(), "small".into()], vec!["chexpert".to_string()]]);
        assert!(mentions_in("x.pdf", "We use the CheXpert dataset (Irvin et al.)", &k));
        assert!(!mentions_in("x.pdf", "CheXpert is mentioned without any context word nearby at all.", &k));
        assert!(mentions_in("chexpert-labels.pdf", "", &k), "in the paper's name");
        let m = dataset_keys("mimic-cxr");
        assert!(mentions_in("x.pdf", "the MIMIC-CXR database of chest radiographs", &m));
        assert!(mentions_in("x.pdf", "MIMICCXR benchmark", &m));
        assert!(!mentions_in("x.pdf", "MIMIC-III clinical database", &m), "first word alone only for version/size suffixes");
        assert!(dataset_keys("adjusted_masks").iter().all(|k| k.len() == 2), "'adjusted' alone is not a key");
        assert!(dataset_keys("images").is_empty() && dataset_keys("json").is_empty() && dataset_keys("2024").is_empty());
        // Everyday English names: no text mention, a file-name match still counts.
        let v = dataset_keys("visualization");
        assert!(common_key(&v[0]) && !common_key(&k[1]) && !common_key(&m[0]));
        assert!(!mentions_in("x.pdf", "a visualization of the dataset", &v));
        assert!(mentions_in("visualization-notes.pdf", "", &v));
    }

    #[test]
    fn days_from_rfc3339() {
        assert_eq!(chrono_like_parse("1970-01-02T00:00:00Z"), Some(86400));
        assert_eq!(chrono_like_parse("2024-03-01T00:00:00Z"), Some(1_709_251_200));
        assert_eq!(chrono_like_parse("bad"), None);
    }
    #[test]
    fn files_about_the_same_thing_are_neighbours_and_say_which_labels_they_share() {
        let mut idx = Index::from_entries(vec![
            Entry::synthetic("a", None, None, "2026-01-01T00:00:00Z"),
            Entry::synthetic("b", None, None, "2020-01-01T00:00:00Z"),
            Entry::synthetic("c", None, None, "2026-01-02T00:00:00Z"),
        ]);
        let labels: HashMap<String, Vec<String>> = [("a", vec!["Fuzzy Systems", "loss"]), ("b", vec!["fuzzy-systems", "IJFS"]), ("c", vec!["Travel"])].into_iter().map(|(k, v)| (k.to_string(), v.into_iter().map(String::from).collect())).collect();
        idx.set_labels(&labels);
        let n = idx.neighbors("a", 2, 1, &Filter::default()).unwrap();
        assert_eq!(n[0].id, "b", "a shared label outweighs a nearer date");
        assert_eq!(n[0].shared, vec!["fuzzy-systems"]);
        assert!(n[0].parts.label > 0.4 && n[1].parts.label == 0.0 && n[1].shared.is_empty());
        assert_eq!(idx.get("a").unwrap().item.labels, vec!["Fuzzy Systems", "loss"]);
    }

}
