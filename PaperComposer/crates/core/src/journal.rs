//! Journal templates: one folder per journal under `publishers/`, holding
//! the class and style files, `sample.tex`, and `template.toml` with what
//! the journal is (name, publisher, where its rules are, notes). A new
//! journal starts as a copy of an existing template and is tuned from there.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
pub struct Meta {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub short: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    /// "journal" or "conference".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// The template this one was copied from.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub based_on: String,
    /// Author instructions.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Submission rules worth remembering (word limits, figure formats, ...).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// How the journal asks references to be listed, in its own words
    /// ("numbered, Vancouver", "author–year, APA").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reference_style: String,
    /// Word limits the journal sets, for the word count in the status bar.
    #[serde(default, skip_serializing_if = "WordLimits::is_empty")]
    pub word_limits: WordLimits,
    /// `[ref_words]`: the word before `\ref` for figures, tables, equations
    /// and sections, at the start of a sentence, or cleveref.
    #[serde(default, skip_serializing_if = "RefWords::is_empty")]
    pub ref_words: RefWords,
    /// `[captions]`: where table and figure captions go.
    #[serde(default, skip_serializing_if = "Captions::is_empty")]
    pub captions: Captions,
    /// `[limits]`: page, figure, table, reference and keyword limits, double-blind.
    #[serde(default, skip_serializing_if = "Limits::is_empty")]
    pub limits: Limits,
    /// `[requires]`: parts the submission must have besides the paper.
    #[serde(default, skip_serializing_if = "Requires::is_empty")]
    pub requires: Requires,
    /// `[sources]`: where a rule came from, by its key (`word_limits.abstract`,
    /// `ref_words.sentence_start.fig`, …): the sentence of the author
    /// guidelines it was read from, or the class whose convention it is.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sources: std::collections::BTreeMap<String, String>,
    /// `base` (a publisher's format journals are built on) or `umbrella` (a series, a
    /// multi-conference event): hidden from the picker; empty = a journal or conference.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// `[venue]`: which journal it is (OpenAlex source; `venueprofile::Identity`). Kept raw so
    /// an odd value never makes the template unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<toml::Table>,
    /// `[submission]`: the journal's submission requirements (`venueprofile::Submission`), raw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission: Option<toml::Table>,
}

/// `[requires]` in template.toml.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
pub struct Requires {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub highlights: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub graphical_abstract: bool,
}

impl Requires {
    pub fn is_empty(&self) -> bool {
        *self == Requires::default()
    }
}

pub use lore_notation::RefWords;

/// `[captions]` in template.toml: `table` / `figure` = "above" or "below".
/// Text, so a template with another value still loads; only those two count.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
pub struct Captions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub figure: Option<String>,
}

impl Captions {
    pub fn is_empty(&self) -> bool {
        self.table.is_none() && self.figure.is_none()
    }
}

/// `[limits]` in template.toml.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
pub struct Limits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    /// The page limit leaves the references out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pages_exclude_refs: bool,
    /// Double-blind review: no author names in the submission.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub anonymous: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub figures: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tables: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<u32>,
    /// Keywords: at most `keywords`, at least `keywords_min`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords_min: Option<u32>,
}

impl Limits {
    pub fn is_empty(&self) -> bool {
        *self == Limits::default()
    }
}

/// `[word_limits]` in template.toml: `abstract`, `total` (the body), and
/// `[word_limits.sections]` by section title. All optional.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
pub struct WordLimits {
    #[serde(default, rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub abstract_: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sections: std::collections::BTreeMap<String, usize>,
}

impl WordLimits {
    pub fn is_empty(&self) -> bool {
        self.abstract_.is_none() && self.total.is_none() && self.sections.is_empty()
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct Journal {
    /// Folder name under publishers/.
    pub id: String,
    #[serde(flatten)]
    pub meta: Meta,
    /// From sample.tex: \documentclass[options]{class}.
    pub class: String,
    pub class_options: String,
    /// The bibliography style in use, and the .bst files the template ships.
    pub bib_style: String,
    pub bib_styles: Vec<String>,
    pub has_sample: bool,
    /// sample.tex loads cleveref.
    pub cleveref: bool,
    /// A base format or an umbrella entry ([`list`] sets it): not offered as a target.
    pub hidden: bool,
}

impl Journal {
    /// The reference words papers follow: `[ref_words]`, or, when the
    /// template sets none and its sample loads cleveref, cleveref mode.
    pub fn effective_ref_words(&self) -> RefWords {
        if self.meta.ref_words.is_empty() && self.cleveref {
            return RefWords { mode: Some("cleveref".into()), ..Default::default() };
        }
        self.meta.ref_words.clone()
    }
}

/// sample.tex as text; Latin-1 samples are read byte for byte.
pub fn read_sample(dir: &Path) -> Option<String> {
    let bytes = std::fs::read(dir.join("sample.tex")).ok()?;
    Some(String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect()))
}

fn code_lines(src: &str) -> impl Iterator<Item = (usize, &str)> {
    // (byte offset of the line, the line without its comment)
    let mut at = 0;
    src.split_inclusive('\n').map(move |l| {
        let start = at;
        at += l.len();
        let b = l.as_bytes();
        let cut = (0..b.len()).find(|&i| b[i] == b'%' && (i == 0 || b[i - 1] != b'\\')).unwrap_or(l.len());
        (start, &l[..cut])
    })
}

/// (options range, options, class) of the first live \documentclass.
fn document_class(src: &str) -> Option<(std::ops::Range<usize>, String, String)> {
    for (start, code) in code_lines(src) {
        let Some(i) = code.find("\\documentclass") else { continue };
        let mut j = i + "\\documentclass".len();
        let rest = &code[j..];
        let (opt_range, opts) = if rest.trim_start().starts_with('[') {
            let k = j + rest.find('[')?;
            let e = k + code[k..].find(']')?;
            j = e + 1;
            (start + k + 1..start + e, code[k + 1..e].to_string())
        } else {
            // no options yet: an empty range right after \documentclass
            (start + j..start + j, String::new())
        };
        let open = j + code[j..].find('{')?;
        let close = open + code[open..].find('}')?;
        return Some((opt_range, opts, code[open + 1..close].trim().to_string()));
    }
    None
}

/// (argument range, style) of the first live \bibliographystyle.
fn bib_style_cmd(src: &str) -> Option<(std::ops::Range<usize>, String)> {
    for (start, code) in code_lines(src) {
        let Some(i) = code.find("\\bibliographystyle{") else { continue };
        let open = i + "\\bibliographystyle{".len();
        let close = open + code[open..].find('}')?;
        return Some((start + open..start + close, code[open..close].trim().to_string()));
    }
    None
}

/// .bst files at the top of the template and in bst/.
fn bst_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = [dir.to_path_buf(), dir.join("bst")]
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".bst")).map(String::from))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The style in use: \bibliographystyle, else a class option naming a .bst
/// (Springer Nature picks its style that way: [sn-vancouver-num]).
fn current_style(sample: &str, bsts: &[String]) -> String {
    if let Some((_, s)) = bib_style_cmd(sample) {
        return s;
    }
    document_class(sample)
        .and_then(|(_, opts, _)| opts.split(',').map(str::trim).find(|o| bsts.iter().any(|b| b == o)).map(String::from))
        .unwrap_or_default()
}

fn meta_of(dir: &Path, id: &str) -> Meta {
    let mut m: Meta = std::fs::read_to_string(dir.join("template.toml")).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
    if m.name.is_empty() {
        // older templates: the README's heading, else the folder name
        m.name = std::fs::read_to_string(dir.join("README.md"))
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("# ").map(|h| h.trim().to_string())))
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| id.to_string());
    }
    m
}

pub fn load(publishers: &Path, id: &str) -> Option<Journal> {
    let dir = publishers.join(id);
    if !dir.is_dir() || !crate::paper::safe_name(id) {
        return None;
    }
    let sample = read_sample(&dir);
    let bsts = bst_files(&dir);
    let (class, class_options) = sample.as_deref().and_then(document_class).map(|(_, o, c)| (c, o)).unwrap_or_default();
    Some(Journal {
        id: id.to_string(),
        meta: meta_of(&dir, id),
        class,
        class_options,
        bib_style: sample.as_deref().map(|s| current_style(s, &bsts)).unwrap_or_default(),
        bib_styles: bsts,
        has_sample: sample.is_some(),
        cleveref: sample.as_deref().is_some_and(|s| packages(s).iter().any(|p| p == "cleveref")),
        hidden: false,
    })
}

/// Every template, by name.
pub fn list(publishers: &Path) -> Vec<Journal> {
    let mut v: Vec<Journal> = std::fs::read_dir(publishers)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| load(publishers, &e.file_name().to_string_lossy()))
        .collect();
    v.sort_by_key(|j| j.meta.name.to_lowercase());
    let parents: std::collections::BTreeSet<String> = v.iter().map(|j| j.meta.based_on.clone()).filter(|b| !b.is_empty()).collect();
    for j in &mut v {
        j.hidden = crate::venueprofile::hidden_role(&j.meta.role, parents.contains(&j.id), crate::venueprofile::Identity::from_table(j.meta.venue.as_ref()).is_some());
    }
    v
}

/// A folder name from the short name or the journal name.
pub fn slug(meta: &Meta) -> String {
    let src = if meta.short.trim().is_empty() { &meta.name } else { &meta.short };
    let s: String = src.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "journal".into() } else { s.chars().take(40).collect() }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let n = e.file_name();
        if n.to_string_lossy().starts_with('.') {
            continue;
        }
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to.join(&n))?;
        } else {
            std::fs::copy(e.path(), to.join(&n))?;
        }
    }
    Ok(())
}

/// What every saved template must satisfy: a name, captions above or below.
fn check_rules(meta: &Meta) -> Result<(), String> {
    for (what, v) in [("表", &meta.captions.table), ("圖", &meta.captions.figure)] {
        if v.as_deref().is_some_and(|p| p != "above" && p != "below") {
            return Err(format!("{what}說明位置只能是 above 或 below"));
        }
    }
    if meta.name.trim().is_empty() {
        return Err("請輸入期刊名稱".into());
    }
    if let (Some(lo), Some(hi)) = (meta.limits.keywords_min, meta.limits.keywords) {
        if lo > hi {
            return Err("關鍵字下限大於上限".into());
        }
    }
    Ok(())
}

/// Change template.toml through `f` (the journal's identity, requirements, role): only what
/// `f` touches changes; the rest is written back as it was.
pub fn edit_meta(publishers: &Path, id: &str, f: impl FnOnce(&mut Meta)) -> Result<Journal, String> {
    let dir = publishers.join(id);
    if !crate::paper::safe_name(id) || !dir.is_dir() {
        return Err("找不到模板".into());
    }
    let mut m = meta_of(&dir, id);
    f(&mut m);
    write_meta(&dir, &m)?;
    load(publishers, id).ok_or_else(|| "找不到模板".into())
}

fn write_meta(dir: &Path, m: &Meta) -> Result<(), String> {
    let t = toml::to_string_pretty(m).map_err(|e| e.to_string())?;
    crate::paper::write_atomic(&dir.join("template.toml"), &t).map_err(|e| e.to_string())
}

/// A new journal template: a copy of `based_on` with its own name.
pub fn create(publishers: &Path, mut meta: Meta, based_on: &str) -> Result<String, String> {
    meta.name = meta.name.trim().to_string();
    if meta.name.is_empty() {
        return Err("請輸入期刊名稱".into());
    }
    let base = publishers.join(based_on);
    if !crate::paper::safe_name(based_on) || !base.is_dir() {
        return Err("找不到來源模板".into());
    }
    let id = free_id(publishers, &meta);
    let dir: PathBuf = publishers.join(&id);
    copy_dir(&base, &dir).map_err(|e| e.to_string())?;
    let base_meta = meta_of(&base, based_on);
    if meta.publisher.is_empty() {
        meta.publisher = base_meta.publisher;
    }
    if meta.kind.is_empty() {
        meta.kind = base_meta.kind;
    }
    // the house rules come along with the files
    if meta.word_limits.is_empty() {
        meta.word_limits = base_meta.word_limits;
    }
    if meta.ref_words.is_empty() {
        meta.ref_words = base_meta.ref_words;
    }
    if meta.captions.is_empty() {
        meta.captions = base_meta.captions;
    }
    if meta.limits.is_empty() {
        meta.limits = base_meta.limits;
    }
    if meta.requires.is_empty() {
        meta.requires = base_meta.requires;
    }
    if meta.reference_style.is_empty() {
        meta.reference_style = base_meta.reference_style;
    }
    if meta.sources.is_empty() {
        meta.sources = base_meta.sources;
    }
    meta.based_on = based_on.to_string();
    write_meta(&dir, &meta)?;
    Ok(id)
}

/// A folder name for `meta` that is not taken yet: its slug, else slug-2, -3, …
fn free_id(publishers: &Path, meta: &Meta) -> String {
    let stem = slug(meta);
    let mut id = stem.clone();
    let mut n = 2;
    while publishers.join(&id).exists() {
        id = format!("{stem}-{n}");
        n += 1;
    }
    id
}

/// A new template from files the author brought (an unpacked LaTeX kit, or
/// what a Word sample became): `files` (paths relative to `src`, one of them
/// `sample.tex`) are copied into a new folder with `meta` as template.toml.
/// Nothing is left behind when a check or a copy fails.
pub fn create_from_files(publishers: &Path, mut meta: Meta, src: &Path, files: &[String]) -> Result<String, String> {
    meta.name = meta.name.trim().to_string();
    meta.based_on.clear();
    check_rules(&meta)?;
    if !files.iter().any(|f| f == "sample.tex") {
        return Err("模板裡沒有 sample.tex".into());
    }
    if !read_sample(src).unwrap_or_default().contains("\\begin{document}") {
        return Err("範本需要 \\begin{document}".into());
    }
    for f in files {
        let p = Path::new(f);
        if p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return Err(format!("檔名不合法：{f}"));
        }
    }
    std::fs::create_dir_all(publishers).map_err(|e| e.to_string())?;
    let id = free_id(publishers, &meta);
    let dir = publishers.join(&id);
    let copy = || -> std::io::Result<()> {
        for f in files {
            let to = dir.join(f);
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(src.join(f), to)?;
        }
        Ok(())
    };
    if let Err(e) = copy().map_err(|e| e.to_string()).and_then(|_| write_meta(&dir, &meta)) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(id)
}

/// What a document class fixes for every journal that uses it: where
/// captions go and, where the publisher's style guide sets it, how figures,
/// tables, equations and sections are named in the text. Limits are per
/// journal and never come from here.
pub struct ClassDefaults {
    /// Whose class it is, for the source label.
    pub family: &'static str,
    pub ref_words: RefWords,
    pub captions: Captions,
}

/// `None` for a class with no house style of its own (article, …).
pub fn class_defaults(class: &str) -> Option<ClassDefaults> {
    let words = |fig: &str, fig_start: &str, tab: &str, eq: &str, eq_start: &str, sec: &str, sec_start: &str| {
        let mut r = RefWords { fig: Some(fig.into()), tab: Some(tab.into()), eq: Some(eq.into()), sec: Some(sec.into()), ..Default::default() };
        for (k, mid, start) in [("fig", fig, fig_start), ("eq", eq, eq_start), ("sec", sec, sec_start)] {
            if start != mid {
                r.sentence_start.insert(k.into(), start.into());
            }
        }
        r
    };
    let (family, ref_words) = match class.trim().rsplit('/').next().unwrap_or("") {
        // IEEE Editorial Style: "Fig." even at a sentence start; "(1)", "Equation (1)" first
        "IEEEtran" | "IEEEtranTCOM" | "IEEEtranTIE" | "ieeecolor" | "ieeeaccess" | "ieeeojies" => ("IEEE", words("Fig.", "Fig.", "Table", "", "Equation", "Section", "Section")),
        // APS / AIP style
        "revtex4" | "revtex4-1" | "revtex4-2" => ("REVTeX（APS／AIP）", words("Fig.", "Figure", "Table", "Eq.", "Equation", "Sec.", "Section")),
        "mdpi" => ("MDPI", words("Figure", "Figure", "Table", "Equation", "Equation", "Section", "Section")),
        "elsarticle" | "cas-sc" | "cas-dc" => ("Elsevier", RefWords::default()),
        "sn-jnl" | "svjour3" | "svjour" => ("Springer Nature", RefWords::default()),
        "llncs" => ("Springer LNCS", RefWords::default()),
        "acmart" => ("ACM", RefWords::default()),
        _ => return None,
    };
    Some(ClassDefaults { family, ref_words, captions: Captions { table: Some("above".into()), figure: Some("below".into()) } })
}

#[derive(Deserialize, Debug, Default)]
pub struct Update {
    #[serde(flatten)]
    pub meta: Meta,
    #[serde(default)]
    pub class_options: Option<String>,
    #[serde(default)]
    pub bib_style: Option<String>,
    /// The house rules: left out, what template.toml has stays; sent (even
    /// empty), they replace it.
    #[serde(default)]
    pub word_limits: Option<WordLimits>,
    #[serde(default)]
    pub ref_words: Option<RefWords>,
    #[serde(default)]
    pub captions: Option<Captions>,
    #[serde(default)]
    pub limits: Option<Limits>,
    #[serde(default)]
    pub requires: Option<Requires>,
    #[serde(default)]
    pub sources: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub reference_style: Option<String>,
}

/// Save a journal's description, and its class options and bibliography
/// style into sample.tex (only those spots change).
pub fn update(publishers: &Path, id: &str, u: Update) -> Result<Journal, String> {
    let dir = publishers.join(id);
    let cur = load(publishers, id).ok_or("找不到模板")?;
    let mut meta = u.meta;
    meta.based_on = cur.meta.based_on.clone();
    // rules not sent stay as template.toml has them
    meta.word_limits = u.word_limits.unwrap_or_else(|| if meta.word_limits.is_empty() { cur.meta.word_limits.clone() } else { meta.word_limits });
    meta.ref_words = u.ref_words.unwrap_or_else(|| if meta.ref_words.is_empty() { cur.meta.ref_words.clone() } else { meta.ref_words });
    meta.captions = u.captions.unwrap_or_else(|| if meta.captions.is_empty() { cur.meta.captions.clone() } else { meta.captions });
    meta.limits = u.limits.unwrap_or_else(|| if meta.limits.is_empty() { cur.meta.limits.clone() } else { meta.limits });
    meta.requires = u.requires.unwrap_or_else(|| if meta.requires.is_empty() { cur.meta.requires.clone() } else { meta.requires });
    meta.sources = u.sources.unwrap_or_else(|| if meta.sources.is_empty() { cur.meta.sources.clone() } else { meta.sources });
    meta.reference_style = u.reference_style.unwrap_or_else(|| if meta.reference_style.is_empty() { cur.meta.reference_style.clone() } else { meta.reference_style });
    // the journal's identity, requirements and role are set by their own calls, never dropped here
    meta.role = if meta.role.is_empty() { cur.meta.role.clone() } else { meta.role };
    meta.venue = meta.venue.or(cur.meta.venue.clone());
    meta.submission = meta.submission.or(cur.meta.submission.clone());
    check_rules(&meta)?;
    write_meta(&dir, &meta)?;
    if let Some(mut s) = read_sample(&dir) {
        let before = s.clone();
        let mut opts = u.class_options.unwrap_or_else(|| cur.class_options.clone()).trim().to_string();
        if opts.contains(['{', '}', '[', ']']) {
            return Err("class 選項不能有括號".into());
        }
        if let Some(style) = u.bib_style.map(|b| b.trim().to_string()).filter(|b| !b.is_empty() && *b != cur.bib_style) {
            if !style.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err("文獻格式名稱不正確".into());
            }
            match bib_style_cmd(&s) {
                Some((r, _)) => s.replace_range(r, &style),
                // Springer Nature names the style as a class option
                None => {
                    let mut v: Vec<&str> = opts.split(',').map(str::trim).filter(|o| !o.is_empty() && *o != cur.bib_style).collect();
                    v.push(&style);
                    opts = v.join(",");
                }
            }
        }
        if let Some((r, old, _)) = document_class(&s) {
            if old.trim() != opts {
                let text = if r.is_empty() { if opts.is_empty() { String::new() } else { format!("[{opts}]") } } else { opts.clone() };
                s.replace_range(r, &text);
            }
        }
        if s != before {
            crate::paper::write_atomic(&dir.join("sample.tex"), &s).map_err(|e| e.to_string())?;
        }
    }
    load(publishers, id).ok_or_else(|| "找不到模板".into())
}

pub fn write_sample(publishers: &Path, id: &str, text: &str) -> Result<(), String> {
    let dir = publishers.join(id);
    if !crate::paper::safe_name(id) || !dir.is_dir() {
        return Err("找不到模板".into());
    }
    if !text.contains("\\begin{document}") {
        return Err("範本需要 \\begin{document}".into());
    }
    crate::paper::write_atomic(&dir.join("sample.tex"), text).map_err(|e| e.to_string())
}

/// FNV-1a: enough to tell files apart (length is compared too).
fn fingerprint(bytes: &[u8]) -> (usize, u64) {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (bytes.len(), h)
}

/// The `.cls` / `.sty` files beside a paper's main.tex that it loads: its
/// `\documentclass` and every `\usepackage` / `\RequirePackage`, with the
/// class first. Files that only sit in the folder do not count.
pub fn used_style_files(paper: &Path) -> Vec<PathBuf> {
    let main = std::fs::read(crate::open::main_file(paper)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let mut names: Vec<String> = document_class(&main).map(|(_, _, c)| vec![format!("{c}.cls")]).unwrap_or_default();
    names.extend(packages(&main).into_iter().map(|n| format!("{n}.sty")));
    names.into_iter().map(|n| paper.join(n)).filter(|p| p.is_file()).collect()
}

/// Packages the preamble loads with `\usepackage` / `\RequirePackage`
/// (commented-out lines do not count).
pub fn packages(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    let begin = src.find("\\begin{document}").unwrap_or(src.len());
    for (_, code) in code_lines(&src[..begin]) {
        for cmd in ["\\usepackage", "\\RequirePackage"] {
            let mut rest = code;
            while let Some(i) = rest.find(cmd) {
                rest = &rest[i + cmd.len()..];
                let r = rest.trim_start();
                let r = if r.starts_with('[') { r.find(']').map_or("", |e| &r[e + 1..]) } else { r };
                if let Some(arg) = r.trim_start().strip_prefix('{').and_then(|a| a.split('}').next()) {
                    names.extend(arg.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()));
                }
            }
        }
    }
    names
}

/// The template a paper was written with: the publisher folder holding
/// byte-identical copies of the class and style files the paper loads (the
/// class must be among them). `None` when nothing matches or two templates
/// match equally well (IEEEtran.cls alone fits every IEEE template).
pub fn detect(publishers: &Path, paper: &Path) -> Option<String> {
    let used = used_style_files(paper);
    let class_local = used.first().is_some_and(|p| p.extension().is_some_and(|x| x == "cls"));
    let ours: Vec<(usize, u64)> = used.iter().filter_map(|p| std::fs::read(p).ok()).map(|b| fingerprint(&b)).collect();
    if ours.is_empty() {
        return None;
    }
    let mut best: Vec<(usize, String)> = Vec::new();
    for e in std::fs::read_dir(publishers).ok()?.flatten() {
        let id = e.file_name().to_string_lossy().to_string();
        if !e.path().is_dir() || !crate::paper::safe_name(&id) {
            continue;
        }
        let theirs: Vec<(usize, u64)> = std::fs::read_dir(e.path())
            .into_iter()
            .flatten()
            .flatten()
            .map(|f| f.path())
            .filter(|p| p.extension().is_some_and(|x| x == "cls" || x == "sty"))
            .filter_map(|p| std::fs::read(p).ok())
            .map(|b| fingerprint(&b))
            .collect();
        if class_local && !theirs.contains(&ours[0]) {
            continue;
        }
        let n = ours.iter().filter(|f| theirs.contains(f)).count();
        if n > 0 {
            best.push((n, id));
        }
    }
    best.sort();
    match best.as_slice() {
        [.., (a, _), (b, _)] if a == b => None,
        [.., (_, id)] => Some(id.clone()),
        [] => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("pc-journal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sn = root.join("springer-nature");
        std::fs::create_dir_all(sn.join("bst")).unwrap();
        std::fs::write(sn.join("sample.tex"), "%%\\documentclass[pdflatex,sn-basic]{sn-jnl}\n\\documentclass[pdflatex,sn-mathphys-num]{sn-jnl}% Math\n\\begin{document}\nx\n\\end{document}\n").unwrap();
        std::fs::write(sn.join("bst/sn-mathphys-num.bst"), "").unwrap();
        std::fs::write(sn.join("bst/sn-vancouver-num.bst"), "").unwrap();
        std::fs::write(sn.join("sn-jnl.cls"), "").unwrap();
        let ieee = root.join("ieee-journal");
        std::fs::create_dir_all(&ieee).unwrap();
        std::fs::write(ieee.join("README.md"), "# IEEE journals\n").unwrap();
        std::fs::write(ieee.join("sample.tex"), "\\documentclass{IEEEtran}\n\\begin{document}\n\\bibliographystyle{IEEEtran}\n\\end{document}\n").unwrap();
        std::fs::write(ieee.join("IEEEtran.bst"), "").unwrap();
        root
    }

    #[test]
    fn word_limits_are_optional_and_kept() {
        let root = setup("limits");
        let sn = root.join("springer-nature");
        std::fs::write(sn.join("template.toml"), "name = \"Example Journal\"\n\n[word_limits]\nabstract = 250\ntotal = 6000\n\n[word_limits.sections]\nIntroduction = 1000\n").unwrap();
        let j = load(&root, "springer-nature").unwrap();
        assert_eq!(j.meta.word_limits.abstract_, Some(250));
        assert_eq!(j.meta.word_limits.total, Some(6000));
        assert_eq!(j.meta.word_limits.sections.get("Introduction"), Some(&1000));
        let json = serde_json::to_value(&j).unwrap();
        assert_eq!(json["word_limits"]["abstract"], 250);
        // the template editor sends no limits: they stay
        update(&root, "springer-nature", Update { meta: Meta { name: "Example Journal".into(), ..Default::default() }, ..Default::default() }).unwrap();
        assert_eq!(load(&root, "springer-nature").unwrap().meta.word_limits.total, Some(6000));
        // a template without limits writes none
        let ieee = load(&root, "ieee-journal").unwrap();
        assert!(ieee.meta.word_limits.is_empty());
        assert!(!toml::to_string(&ieee.meta).unwrap().contains("word_limits"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn house_rules_load_save_and_carry_over() {
        let root = setup("rules");
        let sn = root.join("springer-nature");
        let toml_text = "name = \"Example Journal\"\n\n[word_limits]\nabstract = 200\n\n[ref_words]\nfig = \"Figure\"\neq = \"\"\nsentence_start = { tab = \"Table\" }\n\n[captions]\ntable = \"above\"\nfigure = \"below\"\n\n[limits]\npages = 8\npages_exclude_refs = true\nanonymous = true\n";
        std::fs::write(sn.join("template.toml"), toml_text).unwrap();
        let j = load(&root, "springer-nature").unwrap();
        assert_eq!(j.meta.ref_words.fig.as_deref(), Some("Figure"));
        assert_eq!(j.meta.ref_words.eq.as_deref(), Some(""), "an empty word round-trips");
        assert_eq!(j.meta.ref_words.snippet("tab", "tab:a", true), "Table~\\ref{tab:a}");
        assert_eq!(j.meta.captions.table.as_deref(), Some("above"));
        assert_eq!(j.meta.limits, Limits { pages: Some(8), pages_exclude_refs: true, anonymous: true, ..Default::default() });
        // written back the same way
        let again: Meta = toml::from_str(&toml::to_string_pretty(&j.meta).unwrap()).unwrap();
        assert_eq!(again, j.meta);

        // the editor's JSON: rules left out stay, rules sent replace (even empty)
        let u: Update = serde_json::from_str(r#"{"name":"Example Journal","notes":"x"}"#).unwrap();
        let j = update(&root, "springer-nature", u).unwrap();
        assert_eq!((j.meta.ref_words.fig.as_deref(), j.meta.limits.pages, j.meta.word_limits.abstract_), (Some("Figure"), Some(8), Some(200)));
        let u: Update = serde_json::from_str(r#"{"name":"Example Journal","limits":{},"word_limits":{"total":5000},"ref_words":{"mode":"cleveref"}}"#).unwrap();
        let j = update(&root, "springer-nature", u).unwrap();
        assert!(j.meta.limits.is_empty(), "cleared");
        assert_eq!((j.meta.word_limits.abstract_, j.meta.word_limits.total), (None, Some(5000)));
        assert!(j.meta.ref_words.cleveref() && j.meta.ref_words.fig.is_none());
        assert_eq!(j.meta.captions.table.as_deref(), Some("above"), "not sent: kept");
        let bad: Update = serde_json::from_str(r#"{"name":"Example Journal","captions":{"table":"left"}}"#).unwrap();
        assert!(update(&root, "springer-nature", bad).is_err());

        // a journal made from this one starts with the same rules
        let id = create(&root, Meta { name: "Derived Journal".into(), ..Default::default() }, "springer-nature").unwrap();
        let d = load(&root, &id).unwrap();
        assert_eq!((d.meta.word_limits.total, d.meta.captions.table.as_deref()), (Some(5000), Some("above")));
        assert!(d.meta.ref_words.cleveref());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn templates_without_the_new_rules_write_the_same_text() {
        // template.toml as the app wrote it before [requires], [sources] and the new limits
        for text in [
            "name = \"Example Journal\"\nshort = \"EJ\"\npublisher = \"Example Press\"\nkind = \"journal\"\nurl = \"https://example.org/guide\"\nnotes = \"x\"\n\n[word_limits]\nabstract = 200\n\n[word_limits.sections]\nIntroduction = 1000\n\n[ref_words]\nfig = \"Figure\"\neq = \"\"\n\n[ref_words.sentence_start]\ntab = \"Table\"\n\n[captions]\ntable = \"above\"\n\n[limits]\npages = 8\nanonymous = true\n",
            "name = \"Plain\"\n",
        ] {
            let m: Meta = toml::from_str(text).unwrap();
            assert_eq!(toml::to_string_pretty(&m).unwrap(), text);
        }
    }

    #[test]
    fn new_rules_round_trip_and_survive_the_editor() {
        let root = setup("newrules");
        let sn = root.join("springer-nature");
        let mut m = Meta { name: "Example Journal".into(), reference_style: "numbered, Vancouver".into(), ..Default::default() };
        m.limits = Limits { figures: Some(8), tables: Some(5), references: Some(50), keywords: Some(6), keywords_min: Some(3), ..Default::default() };
        m.requires = Requires { highlights: true, graphical_abstract: true };
        m.sources.insert("limits.figures".into(), "No more than 8 figures.".into());
        write_meta(&sn, &m).unwrap();
        assert_eq!(load(&root, "springer-nature").unwrap().meta, m);
        // the editor's JSON without the new keys keeps them
        let u: Update = serde_json::from_str(r#"{"name":"Example Journal","notes":"y"}"#).unwrap();
        let j = update(&root, "springer-nature", u).unwrap();
        assert_eq!((j.meta.limits.figures, j.meta.requires.highlights, j.meta.reference_style.as_str()), (Some(8), true, "numbered, Vancouver"));
        assert_eq!(j.meta.sources.len(), 1);
        let bad: Update = serde_json::from_str(r#"{"name":"Example Journal","limits":{"keywords":2,"keywords_min":4}}"#).unwrap();
        assert!(update(&root, "springer-nature", bad).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn creates_a_template_from_brought_files() {
        let root = setup("fromfiles");
        let src = root.join("staging");
        std::fs::create_dir_all(src.join("bst")).unwrap();
        std::fs::write(src.join("sample.tex"), "\\documentclass[review]{elsarticle}\n\\begin{document}\nx\n\\end{document}\n").unwrap();
        std::fs::write(src.join("elsarticle.cls"), "%cls").unwrap();
        std::fs::write(src.join("bst/model1-num-names.bst"), "").unwrap();
        let meta = Meta { name: "Journal of Synthetic Examples".into(), short: "JSE".into(), captions: Captions { table: Some("above".into()), figure: None }, ..Default::default() };
        let files: Vec<String> = ["sample.tex", "elsarticle.cls", "bst/model1-num-names.bst"].map(String::from).to_vec();
        let id = create_from_files(&root, meta.clone(), &src, &files).unwrap();
        assert_eq!(id, "jse");
        let j = load(&root, "jse").unwrap();
        assert_eq!((j.class.as_str(), j.class_options.as_str(), j.meta.captions.table.as_deref()), ("elsarticle", "review", Some("above")));
        assert_eq!(j.bib_styles, ["model1-num-names"]);
        assert_eq!(create_from_files(&root, meta.clone(), &src, &files).unwrap(), "jse-2");
        // refused: nothing written
        let bad = Meta { captions: Captions { table: Some("left".into()), figure: None }, ..meta.clone() };
        assert!(create_from_files(&root, bad, &src, &files).is_err());
        assert!(create_from_files(&root, meta.clone(), &src, &["../x".into(), "sample.tex".into()]).is_err());
        assert!(create_from_files(&root, meta, &src, &["elsarticle.cls".into()]).is_err());
        assert!(!root.join("jse-3").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn class_defaults_set_only_house_style() {
        let ieee = class_defaults("IEEEtran").unwrap();
        assert_eq!(ieee.ref_words.snippet("fig", "fig:a", true), "Fig.~\\ref{fig:a}");
        assert_eq!(ieee.ref_words.snippet("eq", "eq:a", false), "\\eqref{eq:a}");
        assert_eq!(ieee.captions.table.as_deref(), Some("above"));
        assert_eq!(class_defaults("revtex4-2").unwrap().ref_words.snippet("fig", "fig:a", true), "Figure~\\ref{fig:a}");
        assert!(class_defaults("elsarticle").unwrap().ref_words.is_empty());
        assert!(class_defaults("article").is_none());
    }

    #[test]
    fn cleveref_in_the_sample_is_the_default_mode() {
        let root = setup("cleveref");
        let ieee = root.join("ieee-journal");
        assert!(load(&root, "ieee-journal").unwrap().effective_ref_words().is_empty(), "no cleveref, no rules: defaults");
        std::fs::write(ieee.join("sample.tex"), "\\documentclass{IEEEtran}\n% \\usepackage{cleveref}\n\\begin{document}\n\\end{document}\n").unwrap();
        assert!(!load(&root, "ieee-journal").unwrap().cleveref, "a commented-out line does not count");
        std::fs::write(ieee.join("sample.tex"), "\\documentclass{IEEEtran}\n\\usepackage{hyperref}\n\\usepackage[capitalise,noabbrev]{cleveref}\n\\begin{document}\n\\end{document}\n").unwrap();
        let j = load(&root, "ieee-journal").unwrap();
        assert!(j.cleveref && j.effective_ref_words().cleveref());
        assert!(j.meta.ref_words.is_empty(), "detected, not written");
        // words the template sets win
        std::fs::write(ieee.join("template.toml"), "name = \"IEEE\"\n[ref_words]\nfig = \"Fig.\"\n").unwrap();
        assert!(!load(&root, "ieee-journal").unwrap().effective_ref_words().cleveref());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_templates_old_and_new() {
        let root = setup("read");
        let v = list(&root);
        let sn = v.iter().find(|j| j.id == "springer-nature").unwrap();
        assert_eq!((sn.class.as_str(), sn.class_options.as_str(), sn.bib_style.as_str()), ("sn-jnl", "pdflatex,sn-mathphys-num", "sn-mathphys-num"));
        assert_eq!(sn.bib_styles, ["sn-mathphys-num", "sn-vancouver-num"]);
        let ieee = v.iter().find(|j| j.id == "ieee-journal").unwrap();
        assert_eq!(ieee.meta.name, "IEEE journals", "name from the README heading");
        assert_eq!(ieee.bib_style, "IEEEtran");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn derives_and_tunes_a_journal() {
        let root = setup("derive");
        let id = create(&root, Meta { name: "Journal of Imaging Informatics in Medicine".into(), short: "JIIM".into(), ..Default::default() }, "springer-nature").unwrap();
        assert_eq!(id, "jiim");
        assert!(root.join("jiim/sn-jnl.cls").is_file() && root.join("jiim/bst/sn-vancouver-num.bst").is_file());
        let j = update(&root, "jiim", Update {
            meta: Meta { name: "Journal of Imaging Informatics in Medicine".into(), short: "JIIM".into(), publisher: "Springer".into(), notes: "Refs numbered by first citation.".into(), ..Default::default() },
            bib_style: Some("sn-vancouver-num".into()),
            class_options: None,
            ..Default::default()
        }).unwrap();
        assert_eq!(j.bib_style, "sn-vancouver-num");
        assert_eq!(j.class_options, "pdflatex,sn-vancouver-num");
        assert_eq!(j.meta.based_on, "springer-nature");
        let sample = read_sample(&root.join("jiim")).unwrap();
        assert!(sample.starts_with("%%\\documentclass[pdflatex,sn-basic]{sn-jnl}\n"), "comments untouched");
        assert!(std::fs::read_to_string(root.join("springer-nature/sample.tex")).unwrap().contains("sn-mathphys-num"), "the base is untouched");
        // IEEE: the style lives in \bibliographystyle; options are added where there were none
        let t = create(&root, Meta { name: "IEEE Transactions on Medical Imaging".into(), short: "TMI".into(), ..Default::default() }, "ieee-journal").unwrap();
        let j = update(&root, &t, Update { meta: Meta { name: "IEEE Transactions on Medical Imaging".into(), ..Default::default() }, class_options: Some("journal".into()), bib_style: Some("IEEEtranN".into()), ..Default::default() }).unwrap();
        assert_eq!((j.class_options.as_str(), j.bib_style.as_str()), ("journal", "IEEEtranN"));
        assert!(create(&root, Meta { name: "IEEE Transactions on Medical Imaging".into(), short: "TMI".into(), ..Default::default() }, "ieee-journal").unwrap() == "tmi-2");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_the_template_by_its_files() {
        let root = setup("detect");
        let pubs = root.join("publishers");
        for (id, files) in [("demo-a", vec![("demo.cls", "class v2"), ("demo-logo.sty", "logo")]), ("demo-b", vec![("demo.cls", "class v1")]), ("plain-1", vec![("common.cls", "c")]), ("plain-2", vec![("common.cls", "c")])] {
            std::fs::create_dir_all(pubs.join(id)).unwrap();
            for (f, body) in files {
                std::fs::write(pubs.join(id).join(f), body).unwrap();
            }
        }
        let paper = root.join("paper");
        std::fs::create_dir_all(&paper).unwrap();
        std::fs::write(paper.join("main.tex"), "\\documentclass[journal]{demo}\n\\usepackage{demo-logo,amsmath}\n% \\usepackage{old}\n\\begin{document}\nx\n\\end{document}\n").unwrap();
        std::fs::write(paper.join("demo.cls"), "class v2").unwrap();
        std::fs::write(paper.join("demo-logo.sty"), "logo").unwrap();
        std::fs::write(paper.join("old.sty"), "unused").unwrap();
        std::fs::write(paper.join("common.cls"), "c").unwrap(); // in the folder, not loaded
        assert_eq!(used_style_files(&paper).len(), 2);
        assert_eq!(detect(&pubs, &paper).as_deref(), Some("demo-a"));
        // An edited class matches nothing.
        std::fs::write(paper.join("demo.cls"), "class v3").unwrap();
        assert_eq!(detect(&pubs, &paper), None);
        // Two templates with the same class: ambiguous.
        std::fs::write(paper.join("main.tex"), "\\documentclass{common}\n\\begin{document}\n\\end{document}\n").unwrap();
        assert_eq!(detect(&pubs, &paper), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
