//! 歸位 (filing), phase A: where each file or folder of the source folders should go in the
//! author's new tree — a plan only. Nothing here moves, creates or renames anything; a move
//! happens later only in a batch the author confirms (phase B).
//!
//! The tree (every segment in English):
//! `Research/<Topic>/{Papers, Datasets, Code, Slides}`, `Courses/<Term>/<Course>`,
//! `Personal/{IDs, Secrets, Finance, Medical, Contracts}`, `Media/{Photos, Videos, Music}`,
//! `Software/{Installers, <Tool>}`, `Archive/<Year>`.
//!
//! Rows: a folder is one row when everything in it goes to the same place (a course folder, a
//! paper's data, a tool's files); otherwise it is opened and its parts become rows. Projects,
//! datasets and app packages are always one row. A loose file is a row of its own. Rules only:
//! what they cannot place is reported as such (never sent to `Archive` by default), a research
//! row's topic is left open (`?`), and a name that is not plain ASCII is flagged for an English name.
//!
//! Settings (`<catalog>/filing.json`, local only): `root`, `sources`, `keep` (folders apps use,
//! never moved).

use crate::index::Entry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SETTINGS: &str = "filing.json";
pub const PLAN: &str = "filing-plan.json";
/// The open topic segment of a research row.
pub const OPEN: &str = "?";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub root: PathBuf,
    pub sources: Vec<PathBuf>,
    /// Never moved (folders apps keep their data in).
    #[serde(default)]
    pub keep: Vec<PathBuf>,
    /// The author's topic merges: a model's topic → the folder it goes to (`Orthopedic Imaging` →
    /// `HipFracture`), matched ignoring case and spaces.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub topics: BTreeMap<String, String>,
}

impl Settings {
    pub fn load(catalog_dir: &Path) -> Option<Settings> {
        std::fs::read(catalog_dir.join(SETTINGS)).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// First run: root `~/Documents/Shelf`, sources Downloads / Desktop / Documents, and the
    /// first-level folders of those that apps use (a LoreForge app's name, an Obsidian vault,
    /// a folder holding an `.app`, Windows leftovers) kept where they are.
    pub fn default_for(home: &Path) -> Settings {
        let root = home.join("Documents").join("Shelf");
        let sources: Vec<PathBuf> = ["Downloads", "Desktop", "Documents"].iter().map(|d| home.join(d)).collect();
        let mut keep = Vec::new();
        for s in &sources {
            let Ok(rd) = std::fs::read_dir(s) else { continue };
            for d in rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                if app_folder(&d) {
                    keep.push(d);
                }
            }
        }
        keep.sort();
        Settings { root, sources, keep, topics: BTreeMap::new() }
    }
}

const LOREFORGE_APPS: [&str; 9] = ["LoreForge", "LoreReader", "LoreShelf", "LoreEcho", "KnowledgeBase", "PaperComposer", "SlideComposer", "JELearner", "NeonDeck"];

fn app_folder(d: &Path) -> bool {
    let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if LOREFORGE_APPS.contains(&name.as_str()) || name.starts_with('$') || name.starts_with('.') {
        return true;
    }
    let apps_inside = LOREFORGE_APPS.iter().filter(|a| d.join(a).is_dir()).count();
    if apps_inside >= 2 || d.join(".obsidian").is_dir() || d.join(".git").is_dir() && d.join("private").is_dir() {
        return true;
    }
    std::fs::read_dir(d).map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "app"))).unwrap_or(false)
}

/// Where one thing goes, by itself: (branch, leaf) like ("Research", "Papers"), and how sure
/// the rule is (a strong rule decides a folder; weak ones only when nothing strong is there).
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub branch: &'static str,
    pub leaf: String,
    pub strong: bool,
    pub why: &'static str,
    /// Placed by a word in the file's own name (`BY_NAME`): decides that file only, never the
    /// folder it is in (a few forms in a research folder do not make it a school folder).
    pub by_name: bool,
}

/// Personal papers by their name (Chinese and English words people name them with): (leaf, why).
const PERSONAL: [(&str, &str, &[&str]); 4] = [
    ("IDs", "證件", &["身分證", "身份證", "id正面", "id背面", "idcard", "id_card", "戶口", "戶籍", "護照", "passport", "學生證", "studentid", "student_id", "在學證明", "畢業證書", "駕照", "健保卡", "居留證", "大頭照", "結訓證明", "結訓令", "退伍令"]),
    ("Finance", "財務", &["存簿", "存摺", "郵局", "銀行", "帳戶", "請款", "收據", "發票", "薪資", "扣繳", "報稅", "bank", "invoice", "receipt", "payslip"]),
    ("Medical", "醫療", &["疫苗", "體檢", "健檢", "病歷", "診斷", "處方", "vaccine"]),
    ("Contracts", "合約", &["合約", "契約", "投保", "投退保", "保險", "租約"]),
];

/// The author's other papers by name (2026-11-13 categories): (branch, leaf, why, words). ASCII
/// words match whole words (`cv` is not `cvpr`); Chinese ones anywhere in the name.
const BY_NAME: [(&str, &str, &str, &[&str]); 10] = [
    ("School", "IRB", "IRB", &["irb", "倫理審查", "人體研究", "受試者"]),
    ("School", "Funding", "經費補助", &["補助", "核定通知", "預算", "經費", "獎助", "貨款", "轉帳申請", "budget", "grant"]),
    ("School", "Travel", "出國出差", &["出國報告", "出差", "旅費", "差旅", "日支數額", "派赴國外", "國外進修", "travel"]),
    ("Career", "", "申請與履歷", &["cv", "vita", "resume", "履歷", "推薦函", "自傳", "獎學金", "傑出表現", "求職", "應徵", "applicant", "recommendation"]),
    ("School", "Forms", "學校表單", &["確認表", "申請表", "申請書", "報到", "表單", "調查表", "作答調查", "公告", "辦法"]),
    ("Templates", "", "範本", &["template", "範本", "模板", "samplepaper"]),
    ("Software", "Manuals", "說明書", &["使用說明書", "說明書", "manual", "手冊", "userguide"]),
    ("Learning", "Languages", "語言學習", &["japanese", "日文", "日語", "韓文", "verb", "vocabulary", "單字"]),
    ("Research", "Manuscripts", "自己的稿件", &["manuscript", "highlights", "graphicalabstract", "responsetoreviewers", "rebuttal", "coverletter", "投稿"]),
    ("Library", "Books", "書", &["ebook", "textbook", "教科書"]),
];

fn by_name(name: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let n = [name.to_string()];
    BY_NAME.iter().find(|(_, _, _, words)| lore_catalog::category::name_has(&n, words)).map(|(b, l, why, _)| (*b, *l, *why))
}

fn personal(name: &str) -> Option<(&'static str, &'static str)> {
    let n = name.to_lowercase();
    PERSONAL.iter().find(|(_, _, words)| words.iter().any(|w| n.contains(w))).map(|(leaf, why, _)| (*leaf, *why))
}

fn place(kind: &str, name: &str, categories: &[String], sensitive: bool) -> Option<Place> {
    let has = |c: &str| categories.iter().any(|x| x == c);
    let p = |branch, leaf: &str, strong, why| Some(Place { branch, leaf: leaf.into(), strong, why, by_name: false });
    if has("secret") || lore_catalog::category::secret_name(kind, name) {
        return p("Personal", "Secrets", true, "機密（密碼、金鑰、token）");
    }
    if sensitive || has("id-doc") {
        return p("Personal", "IDs", true, "證件");
    }
    // a paper about medicine is a paper; personal words count only for the rest
    if !has("paper") && !matches!(kind, "project" | "dataset" | "package" | "code" | "latex" | "slides") {
        if let Some((leaf, why)) = personal(name) {
            return p("Personal", leaf, true, why);
        }
    }
    if !matches!(kind, "project" | "dataset" | "package" | "code" | "latex") {
        if let Some((branch, leaf, why)) = by_name(name) {
            // a paper's own name only says it is a manuscript; the other words are not about papers
            if !has("paper") || leaf == "Manuscripts" {
                return Some(Place { branch, leaf: leaf.into(), strong: true, why, by_name: true });
            }
        }
    }
    if kind == "ebook" {
        return p("Library", "Books", true, "電子書");
    }
    if has("ticket") {
        return p("Personal", "Finance", true, "票券收據");
    }
    match kind {
        "installer" | "package" => p("Software", "Installers", true, "安裝檔"),
        "font" => p("Software", "Fonts", false, "字型"),
        "dataset" => p("Research", "Datasets", true, "資料集"),
        "project" => p("Research", "Code", true, "專案"),
        "latex" => p("Research", "Papers", true, "LaTeX"),
        "code" => p("Research", "Code", false, "程式"),
        "data" => p("Research", "Datasets", false, "資料"),
        "slides" => p("Research", "Slides", false, "簡報"),
        "pdf" | "document" if has("paper") => p("Research", "Papers", true, "論文"),
        "video" => p("Media", "Videos", false, "影片"),
        "audio" => p("Media", "Music", false, "音樂"),
        "image" => p("Media", "Photos", false, "圖片"),
        _ => None,
    }
}

/// A course path: the segments after a `courses` folder → (term, course).
/// `courses/<school>/<term>/<course>/…` or `courses/<term>/<course>/…`.
fn course_of(rel: &[String]) -> Option<(String, Option<String>, usize)> {
    let i = rel.iter().position(|s| matches!(s.to_lowercase().as_str(), "courses" | "course" | "課程"))?;
    // the folders below it (the last segment is the file itself)
    let rest = &rel[i + 1..rel.len() - 1];
    // a school level: its child is the term (`<school>/<term>/…`)
    let k = usize::from(rest.len() >= 2 && !looks_like_term(&rest[0]) && looks_like_term(&rest[1]));
    let term = rest.get(k)?;
    // `at` = the length of the course folder's path (or the term folder's, for loose files)
    Some((term.clone(), rest.get(k + 1).cloned(), i + 2 + k + usize::from(rest.len() > k + 1)))
}

fn course_target(term: &str, course: Option<&str>) -> String {
    match course {
        Some(c) => format!("Courses/{term}/{c}"),
        None => format!("Courses/{term}"),
    }
}

fn looks_like_term(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit())
}

/// A folder name usable as is: plain ASCII letters, digits and `-_. ` .
pub fn plain(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_. ()".contains(c))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// What moves: a file path or a folder path.
    pub from: String,
    /// `file`, `folder`, `project`, `dataset`, `package`.
    pub kind: String,
    /// Catalogued files inside (1 for a file).
    pub files: usize,
    /// Content id for a file or unit; empty for a folder.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// Relative to the root, `?` for an open topic; `None` = cannot place.
    pub target: Option<String>,
    pub reason: String,
    /// `need-topic`, `need-name`, `sensitive`, `duplicate`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Plan {
    pub made: String,
    pub root: PathBuf,
    pub rows: Vec<Row>,
}

/// One thing under a source: its path below the source and where it goes by itself.
struct Thing<'a> {
    rel: Vec<String>,
    entry: &'a Entry,
    path: String,
    place: Option<Place>,
}

/// The plan for these entries (paths of every catalogued file and unit).
pub fn plan(entries: &[Entry], s: &Settings, made: &str) -> Plan {
    let mut by_source: BTreeMap<PathBuf, Vec<Thing>> = BTreeMap::new();
    // the content ids seen more than once among the things to move
    let mut copies: BTreeMap<&str, usize> = BTreeMap::new();
    for e in entries {
        for p in &e.paths {
            let pb = Path::new(p);
            if pb.starts_with(&s.root) || s.keep.iter().any(|k| pb.starts_with(k)) {
                continue;
            }
            let Some(src) = s.sources.iter().filter(|r| pb.starts_with(r)).max_by_key(|r| r.components().count()) else { continue };
            let rel: Vec<String> = pb.strip_prefix(src).unwrap().components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            // hidden files and folders (browser temp files, `.localized`, checkpoints) are not the author's to file
            if rel.is_empty() || rel.iter().any(|x| x.starts_with('.')) || rel.last().is_some_and(|x| x == "desktop.ini" || x.starts_with("~$")) {
                continue;
            }
            *copies.entry(e.item.id.as_str()).or_default() += 1;
            by_source.entry(src.clone()).or_default().push(Thing { rel, entry: e, path: p.clone(), place: place(&e.item.kind, &e.item.name, &e.item.categories, e.item.sensitive) });
        }
    }
    let mut rows = Vec::new();
    for (src, things) in &by_source {
        let refs: Vec<&Thing> = things.iter().collect();
        split(src, &[], &refs, &mut rows);
    }
    for r in &mut rows {
        if !r.id.is_empty() && copies.get(r.id.as_str()).is_some_and(|&n| n > 1) {
            r.flags.push("duplicate".into());
        }
    }
    rows.sort_by(|a, b| a.from.cmp(&b.from));
    Plan { made: made.into(), root: s.root.clone(), rows }
}

/// Rows for the things under `src/prefix`: one row for the folder when they agree, else per
/// child folder and loose file.
fn split(src: &Path, prefix: &[String], things: &[&Thing], rows: &mut Vec<Row>) {
    let depth = prefix.len();
    let mut children: BTreeMap<&str, Vec<&Thing>> = BTreeMap::new();
    for t in things {
        if t.rel.len() == depth + 1 {
            rows.push(file_row(t));
        } else {
            children.entry(t.rel[depth].as_str()).or_default().push(t);
        }
    }
    // pictures next to personal papers (a head shot beside the ID scans) go with them
    let personal_here = things.iter().filter(|t| t.rel.len() == depth + 1).filter(|t| t.place.as_ref().is_some_and(|p| p.branch == "Personal")).count();
    if personal_here * 3 >= things.iter().filter(|t| t.rel.len() == depth + 1).count().max(1) && personal_here > 0 {
        for r in rows.iter_mut().filter(|r| r.kind == "file" && Path::new(&r.from).parent() == Some(&src.join(prefix.join("/"))) && r.target.as_deref().is_none_or(|t| t == "Media/Photos")) {
            r.target = Some("Personal/IDs".into());
            r.reason = "和證件放在一起".into();
            if !r.flags.iter().any(|f| f == "sensitive") {
                r.flags.push("sensitive".into());
            }
        }
    }
    for (name, inner) in children {
        let mut p = prefix.to_vec();
        p.push(name.to_string());
        match folder_target(&p, &inner) {
            Some(row) => rows.push(Row { from: src.join(p.join("/")).display().to_string(), files: inner.len(), ..row }),
            None => split(src, &p, &inner, rows),
        }
    }
}

fn file_row(t: &Thing) -> Row {
    let it = &t.entry.item;
    let kind = if matches!(it.kind.as_str(), "project" | "dataset" | "package") { it.kind.clone() } else { "file".into() };
    let mut flags = Vec::new();
    if it.sensitive {
        flags.push("sensitive".into());
    }
    let folders = &t.rel[..t.rel.len() - 1];
    let target = if let Some((term, course, _)) = course_of(&t.rel) {
        Some(course_target(&term, course.as_deref()))
    } else if t.place.as_ref().is_some_and(|p| p.branch == "Research") && folders.len() >= 2 {
        // a research file in an opened folder stays with its folder's other research
        if folders[1..].iter().any(|f| !plain(f)) {
            flags.push("need-name".into());
        }
        Some(research_path(folders))
    } else {
        t.place.as_ref().map(|p| leaf_path(p, None, &mut flags))
    };
    let reason = t.place.as_ref().map(|p| p.why.to_string()).unwrap_or_else(|| "規則判斷不了".into());
    Row { from: t.path.clone(), kind, files: 1, id: it.id.clone(), target, reason, flags }
}

/// `Research/?/Papers` (+ the folder's own name when it moves as a folder).
fn leaf_path(p: &Place, folder: Option<&str>, flags: &mut Vec<String>) -> String {
    let base = match p.branch {
        "Research" => {
            flags.push("need-topic".into());
            format!("Research/{OPEN}/{}", p.leaf)
        }
        b if p.leaf.is_empty() => b.to_string(),
        b => format!("{b}/{}", p.leaf),
    };
    match folder {
        Some(f) => {
            if !plain(f) {
                flags.push("need-name".into());
            }
            format!("{base}/{f}")
        }
        None => base,
    }
}

/// A research folder's place: its path below the source, without the first folder when that was
/// opened as a container (`lab/rib/code` → `Research/rib/code`, `rib_data` → `Research/rib_data`).
fn research_path(path: &[String]) -> String {
    let below = if path.len() >= 2 { &path[1..] } else { path };
    format!("Research/{}", below.join("/"))
}

/// The whole folder's row when everything in it goes to one place; `None` = open it.
/// What a thing counts for when its folder is judged (a by-name place does not count).
fn seen<'a>(t: &'a Thing) -> Option<&'a Place> {
    t.place.as_ref().filter(|p| !p.by_name)
}

fn folder_target(path: &[String], inner: &[&Thing]) -> Option<Row> {
    let name = path.last().unwrap().as_str();
    // a course folder: everything under courses/<term>/<course> (or a term folder of loose files)
    if let Some((term, course, at)) = inner.first().and_then(|t| course_of(&t.rel)) {
        if path.len() == at && inner.iter().all(|t| course_of(&t.rel).is_some_and(|(a, b, _)| a == term && b == course)) {
            let mut flags = Vec::new();
            if !plain(&term) || course.as_deref().is_some_and(|c| !plain(c)) {
                flags.push("need-name".into());
            }
            return Some(Row { kind: "folder".into(), target: Some(course_target(&term, course.as_deref())), reason: "課程資料夾".into(), flags, ..Row::default() });
        }
        if path.len() < at || inner.iter().any(|t| course_of(&t.rel).is_none_or(|(a, b, _)| a != term || b != course)) {
            return None; // above a course folder: open it
        }
    }
    let personal_n = inner.iter().filter(|t| seen(t).is_some_and(|p| p.branch == "Personal")).count();
    if personal_n > 0 {
        return None; // personal papers never travel inside another folder's move: each to its own place
    }
    if inner.iter().all(|t| seen(t).is_none()) {
        // nothing in it is known: the folder stays one row the rules cannot place
        return Some(Row { kind: "folder".into(), target: None, reason: "規則判斷不了".into(), ..Row::default() });
    }
    let fonts = inner.iter().filter(|t| t.entry.item.kind == "font").count();
    if fonts * 2 >= inner.len() {
        let flags = if plain(name) { vec![] } else { vec!["need-name".into()] };
        return Some(Row { kind: "folder".into(), target: Some(format!("Software/{name}")), reason: "資料夾裡大多是字型".into(), flags, ..Row::default() });
    }
    let strong: Vec<&Place> = inner.iter().filter_map(|t| seen(t)).filter(|p| p.strong).collect();
    let branches = |ps: &[&Place]| {
        let mut b: Vec<&str> = ps.iter().map(|p| p.branch).collect();
        b.sort_unstable();
        b.dedup();
        b
    };
    let mut decided: Vec<&Place> = if strong.is_empty() { inner.iter().filter_map(|t| seen(t)).collect() } else { strong };
    // pictures next to research files are its data, not photos
    if decided.iter().any(|p| p.branch == "Research") {
        decided.retain(|p| !(p.branch == "Media" && p.leaf == "Photos"));
    }
    if decided.is_empty() || branches(&decided).len() > 1 {
        // a mixed folder: open it, unless it is a single loose level (then it cannot be placed whole)
        return None;
    }
    // the leaf most of it is (a research folder of images and labels is a dataset)
    let mut leaves: BTreeMap<&str, usize> = BTreeMap::new();
    for p in inner.iter().filter_map(|t| seen(t)) {
        let leaf = if decided[0].branch == "Research" && p.branch == "Media" && p.leaf == "Photos" { "Datasets" } else { p.leaf.as_str() };
        if p.branch == decided[0].branch || decided[0].branch == "Research" {
            *leaves.entry(leaf).or_default() += 1;
        }
    }
    let leaf = leaves.into_iter().max_by_key(|(_, n)| *n).map(|(l, _)| l.to_string()).unwrap_or_else(|| decided[0].leaf.clone());
    let p = Place { leaf: leaf.clone(), ..decided[0].clone() };
    let mut flags = Vec::new();
    // a tool's folder (fonts, TeX files) is the tool itself under Software
    // a research folder is a topic of its own: it moves whole, its inside as it is
    let target = if p.branch == "Research" {
        if !plain(name) {
            flags.push("need-name".into());
        }
        research_path(path)
    } else if p.branch == "Software" && p.leaf != "Installers" {
        format!("Software/{name}")
    } else {
        leaf_path(&p, Some(name), &mut flags)
    };
    if p.branch == "Software" && !plain(name) {
        flags.push("need-name".into());
    }
    Some(Row { kind: "folder".into(), target: Some(target), reason: format!("資料夾裡大多是{}", p.why), flags, ..Row::default() })
}

/// Counts for the summary: per source, rows, files, cannot place, need topic, need name.
pub fn summary(plan: &Plan, sources: &[PathBuf]) -> Vec<(String, [usize; 6])> {
    let mut out = Vec::new();
    for s in sources {
        let rows: Vec<&Row> = plan.rows.iter().filter(|r| Path::new(&r.from).starts_with(s)).collect();
        let f = |flag: &str| rows.iter().filter(|r| r.flags.iter().any(|x| x == flag)).count();
        out.push((
            s.display().to_string(),
            [rows.len(), rows.iter().map(|r| r.files).sum(), rows.iter().filter(|r| r.target.is_none()).count(), f("need-topic"), f("need-name"), rows.iter().filter(|r| r.target.as_deref().is_some_and(|t| t.starts_with("Archive"))).count()],
        ));
    }
    out
}

// ---- phase B: moving a confirmed batch, and undoing it ----

/// The batch waiting for the author's confirmation (`<catalog>/filing-batch.json`).
pub const BATCH: &str = "filing-batch.json";
/// Every move done, one batch per line (`<catalog>/filing-journal.jsonl`), for undo.
pub const JOURNAL: &str = "filing-journal.jsonl";

/// A row the rules are sure of: a place, no open topic, no name to choose, and not a research
/// topic or a folder of pictures (those wait for the model to group them into topics).
pub fn certain(r: &Row) -> bool {
    let Some(t) = r.target.as_deref() else { return false };
    !t.contains(OPEN)
        && !r.flags.iter().any(|f| f == "need-name" || f == "need-topic" || f == "by-model")
        && !t.starts_with("Research/")
        && !(r.kind == "folder" && t.starts_with("Media/"))
}

/// Where a row lands: a folder row's target is its own new path; a file's (or unit's) target is
/// the folder it goes into.
pub fn dest(r: &Row, root: &Path) -> Option<PathBuf> {
    let t = root.join(r.target.as_deref()?);
    if r.kind == "folder" {
        Some(t)
    } else {
        Some(t.join(Path::new(&r.from).file_name()?))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Moved {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Batch {
    pub at: String,
    pub moves: Vec<Moved>,
}

/// A free name next to `p` when `p` is taken (`a.pdf` → `a 2.pdf`).
fn free(p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| p.with_file_name(format!("{stem} {n}{ext}"))).find(|q| !q.exists()).unwrap()
}

/// Moves the rows (a rename on the same volume — nothing is copied or deleted) and appends the
/// batch to the journal after each move, so a crash never loses where a file went. A row whose
/// source is gone, or that would cross volumes, is skipped and reported.
pub fn apply(rows: &[Row], root: &Path, journal: &Path, at: &str) -> (Batch, Vec<String>) {
    let mut batch = Batch { at: at.into(), moves: Vec::new() };
    let mut skipped = Vec::new();
    for r in rows {
        let from = Path::new(&r.from);
        let Some(to) = dest(r, root) else {
            skipped.push(format!("{}：沒有去處", r.from));
            continue;
        };
        if !from.exists() {
            skipped.push(format!("{}：已經不在原處", r.from));
            continue;
        }
        let to = free(to);
        if let Some(parent) = to.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                skipped.push(format!("{}：{e}", r.from));
                continue;
            }
        }
        match std::fs::rename(from, &to) {
            Ok(()) => {
                batch.moves.push(Moved { from: r.from.clone(), to: to.display().to_string() });
                write_journal(journal, &batch);
            }
            // EXDEV and anything else: leave it where it is
            Err(e) => skipped.push(format!("{}：{e}", r.from)),
        }
    }
    (batch, skipped)
}

/// Rewrites the journal's last line for this batch (or appends it).
fn write_journal(journal: &Path, batch: &Batch) {
    let mut lines: Vec<String> = std::fs::read_to_string(journal).unwrap_or_default().lines().map(String::from).collect();
    let line = serde_json::to_string(batch).unwrap_or_default();
    match lines.last().and_then(|l| serde_json::from_str::<Batch>(l).ok()) {
        Some(last) if last.at == batch.at => *lines.last_mut().unwrap() = line,
        _ => lines.push(line),
    }
    let _ = std::fs::write(journal, lines.join("\n") + "\n");
}

/// Undoes the journal's last batch (moves back what is still where it went, never over an
/// existing file) and drops it from the journal. Returns (moved back, left as is).
pub fn undo(journal: &Path) -> Result<(usize, Vec<String>), String> {
    let text = std::fs::read_to_string(journal).map_err(|_| "沒有可以復原的批次".to_string())?;
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let last = lines.pop().ok_or("沒有可以復原的批次")?;
    let batch: Batch = serde_json::from_str(last).map_err(|e| e.to_string())?;
    let (mut back, mut left) = (0, Vec::new());
    for m in batch.moves.iter().rev() {
        let (to, from) = (Path::new(&m.to), Path::new(&m.from));
        if !to.exists() || from.exists() {
            left.push(m.to.clone());
            continue;
        }
        if let Some(p) = from.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        match std::fs::rename(to, from) {
            Ok(()) => back += 1,
            Err(_) => left.push(m.to.clone()),
        }
    }
    let rest = if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" };
    std::fs::write(journal, rest).map_err(|e| e.to_string())?;
    Ok((back, left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_moves_by_rename_never_over_a_file_and_undoes() {
        let d = tempfile::tempdir().unwrap();
        let (src, root, j) = (d.path().join("Downloads"), d.path().join("Shelf"), d.path().join("j.jsonl"));
        std::fs::create_dir_all(src.join("pics")).unwrap();
        std::fs::write(src.join("receipt.pdf"), "a").unwrap();
        std::fs::write(src.join("pics/1.jpg"), "b").unwrap();
        std::fs::create_dir_all(root.join("Personal/Finance")).unwrap();
        std::fs::write(root.join("Personal/Finance/receipt.pdf"), "old").unwrap();
        let row = |from: &Path, kind: &str, t: &str| Row { from: from.display().to_string(), kind: kind.into(), files: 1, target: Some(t.into()), ..Row::default() };
        let rows = vec![
            row(&src.join("receipt.pdf"), "file", "Personal/Finance"),
            row(&src.join("pics"), "folder", "Media/Photos/pics"),
            row(&src.join("gone.pdf"), "file", "Personal/Finance"),
        ];
        let (b, skipped) = apply(&rows, &root, &j, "1");
        assert_eq!(b.moves.len(), 2);
        assert_eq!(skipped.len(), 1);
        assert_eq!(std::fs::read_to_string(root.join("Personal/Finance/receipt.pdf")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(root.join("Personal/Finance/receipt 2.pdf")).unwrap(), "a");
        assert!(root.join("Media/Photos/pics/1.jpg").exists() && !src.join("pics").exists());
        let (back, left) = undo(&j).unwrap();
        assert_eq!((back, left.len()), (2, 0));
        assert_eq!(std::fs::read_to_string(src.join("receipt.pdf")).unwrap(), "a");
        assert!(src.join("pics/1.jpg").exists());
        assert!(undo(&j).is_err());
    }

    #[test]
    fn only_rows_the_rules_are_sure_of_are_certain() {
        let r = |kind: &str, t: Option<&str>, flags: &[&str]| Row { kind: kind.into(), target: t.map(String::from), flags: flags.iter().map(|f| f.to_string()).collect(), ..Row::default() };
        assert!(certain(&r("file", Some("Personal/Finance"), &["sensitive"])));
        assert!(certain(&r("file", Some("Media/Photos"), &[])));
        assert_eq!(place("text", "github_token.txt", &[], false).unwrap().leaf, "Secrets");
        let at = |kind: &str, name: &str, cats: &[&str]| {
            let c: Vec<String> = cats.iter().map(|c| c.to_string()).collect();
            place(kind, name, &c, false).map(|p| leaf_path(&p, None, &mut Vec::new()))
        };
        assert_eq!(at("document", "國外出差旅費報告表.xlsx", &[]).as_deref(), Some("School/Travel"));
        assert_eq!(at("pdf", "IRB-送件證明.pdf", &[]).as_deref(), Some("School/IRB"));
        assert_eq!(at("pdf", "ellrin_cv_2024.pdf", &[]).as_deref(), Some("Career"));
        assert_eq!(at("document", "JBHI_word_template.docx", &[]).as_deref(), Some("Templates"));
        assert_eq!(at("pdf", "revised manuscript X.pdf", &["paper"]).as_deref(), Some("Research/?/Manuscripts"));
        // whole words only, and a paper's title words are not school words
        assert_eq!(at("pdf", "cvpr2024.pdf", &["paper"]).as_deref(), Some("Research/?/Papers"));
        assert_eq!(at("pdf", "travel time estimation.pdf", &["paper"]).as_deref(), Some("Research/?/Papers"));
        assert!(!certain(&r("folder", Some("Media/Photos/fig"), &[])));
        assert!(!certain(&r("folder", Some("Research/x"), &[])));
        assert!(!certain(&r("file", Some("Research/?/Papers"), &["need-topic"])));
        assert!(!certain(&r("file", None, &[])));
        assert!(!certain(&r("folder", Some("Courses/t/c"), &["need-name"])));
    }

    fn e(id: &str, kind: &str, cats: &[&str], path: &str) -> Entry {
        let mut x = Entry::synthetic(id, None, None, "2026-01-01T00:00:00Z");
        x.item.kind = kind.into();
        x.item.categories = cats.iter().map(|c| c.to_string()).collect();
        x.item.sensitive = cats.contains(&"id-doc");
        x.item.name = path.rsplit('/').next().unwrap().into();
        x.paths = vec![path.into()];
        x
    }

    fn settings() -> Settings {
        Settings { root: "/h/Documents/Shelf".into(), sources: vec!["/h/Downloads".into(), "/h/Documents".into()], keep: vec!["/h/Documents/LoreReader".into()], topics: BTreeMap::new() }
    }

    fn find<'a>(p: &'a Plan, from: &str) -> &'a Row {
        p.rows.iter().find(|r| r.from == from).unwrap_or_else(|| panic!("no row {from}: {:#?}", p.rows))
    }

    #[test]
    fn folders_move_whole_when_they_agree_and_open_when_mixed() {
        let es = vec![
            // a paper's data: images and labels → one dataset folder
            e("a", "image", &["image"], "/h/Downloads/rib_data/img/1.png"),
            e("b", "data", &[], "/h/Downloads/rib_data/img/1.json"),
            e("c", "code", &[], "/h/Downloads/rib_data/train.py"),
            // a mixed folder: a passport scan and a paper → opened
            e("d", "pdf", &["id-doc"], "/h/Downloads/misc/passport.pdf"),
            e("f", "pdf", &["paper"], "/h/Downloads/misc/ellrin2026.pdf"),
            // loose files
            e("g", "installer", &[], "/h/Downloads/tool.dmg"),
            e("h", "other", &[], "/h/Downloads/notes.xyz"),
            // a course
            e("i", "slides", &[], "/h/Documents/school/courses/uni/term_1/seminar/week1.pptx"),
            e("j", "pdf", &["paper"], "/h/Documents/school/courses/uni/term_1/seminar/paper.pdf"),
            // kept and already in place
            e("k", "pdf", &["paper"], "/h/Documents/LoreReader/x.pdf"),
            e("l", "pdf", &["paper"], "/h/Documents/Shelf/Research/A/Papers/y.pdf"),
            // a Chinese-named fonts folder
            e("m", "font", &[], "/h/Downloads/字型/a.ttf"),
            e("n", "other", &[], "/h/Downloads/.com.google.Chrome.x"),
        ];
        let p = plan(&es, &settings(), "now");
        let r = find(&p, "/h/Downloads/rib_data");
        assert_eq!((r.kind.as_str(), r.files, r.target.as_deref()), ("folder", 3, Some("Research/rib_data")), "a research folder is a topic of its own");
        assert_eq!(find(&p, "/h/Downloads/misc/passport.pdf").target.as_deref(), Some("Personal/IDs"));
        assert_eq!(find(&p, "/h/Downloads/misc/ellrin2026.pdf").target.as_deref(), Some("Research/?/Papers"));
        assert_eq!(find(&p, "/h/Downloads/tool.dmg").target.as_deref(), Some("Software/Installers"));
        assert_eq!(find(&p, "/h/Downloads/notes.xyz").target, None, "cannot place, never Archive by default");
        let c = find(&p, "/h/Documents/school/courses/uni/term_1/seminar");
        assert_eq!((c.files, c.target.as_deref()), (2, Some("Courses/term_1/seminar")));
        assert!(!p.rows.iter().any(|r| r.from.contains("LoreReader") || r.from.contains("Shelf")), "kept folders and the new tree are left alone");
        let f = find(&p, "/h/Downloads/字型");
        assert_eq!(f.target.as_deref(), Some("Software/字型"));
        assert!(f.flags.contains(&"need-name".to_string()));
        assert_eq!(p.rows.len(), 7);
    }

    #[test]
    fn personal_papers_by_name_and_the_pictures_beside_them() {
        let es = vec![
            e("a", "image", &["image"], "/h/Documents/info/id正面.jpeg"),
            e("b", "image", &["image"], "/h/Documents/info/存簿.jpg"),
            e("c", "pdf", &[], "/h/Documents/info/ellrin_疫苗.pdf"),
            e("p", "pdf", &["paper"], "/h/Documents/lab/rib/A survey of medical image registration.pdf"),
            e("q", "code", &[], "/h/Documents/lab/rib/code/a.py"),
            e("r", "pdf", &["id-doc"], "/h/Documents/lab/rib/scan_id.pdf"),
            e("d", "image", &["image"], "/h/Documents/info/face.jpg"),
            e("f", "other", &[], "/h/Documents/docker/a/Dockerfile"),
            e("g", "other", &[], "/h/Documents/docker/b/Dockerfile2"),
            e("h", "docx", &[], "/h/Documents/courses/uni/term_2/ta_note.docx"),
        ];
        let p = plan(&es, &settings(), "now");
        assert_eq!(find(&p, "/h/Documents/info/id正面.jpeg").target.as_deref(), Some("Personal/IDs"));
        assert_eq!(find(&p, "/h/Documents/info/存簿.jpg").target.as_deref(), Some("Personal/Finance"));
        assert_eq!(find(&p, "/h/Documents/info/ellrin_疫苗.pdf").target.as_deref(), Some("Personal/Medical"));
        assert_eq!(find(&p, "/h/Documents/lab/rib/A survey of medical image registration.pdf").target.as_deref(), Some("Research/rib"), "a paper about medicine is no medical record");
        assert_eq!(find(&p, "/h/Documents/lab/rib/code").target.as_deref(), Some("Research/rib/code"), "the path below the opened container");
        assert_eq!(find(&p, "/h/Documents/info/face.jpg").target.as_deref(), Some("Personal/IDs"), "a photo beside ID scans");
        let d = find(&p, "/h/Documents/docker");
        assert_eq!((d.files, d.target.as_deref()), (2, None), "an unknown folder stays one row");
        assert_eq!(find(&p, "/h/Documents/courses/uni/term_2").target.as_deref(), Some("Courses/term_2"));
    }

    #[test]
    fn a_copy_in_two_places_is_flagged() {
        let mut x = e("a", "pdf", &["paper"], "/h/Downloads/p.pdf");
        x.paths.push("/h/Downloads/p (1).pdf".into());
        let p = plan(&[x], &settings(), "now");
        assert!(p.rows.iter().all(|r| r.flags.contains(&"duplicate".to_string())) && p.rows.len() == 2);
    }

    #[test]
    fn a_word_in_a_file_name_places_the_file_never_its_folder() {
        let p = plan(
            &[
                e("a", "spreadsheet", &[], "/h/Downloads/data/notes/budget 1.xlsx"),
                e("b", "spreadsheet", &[], "/h/Downloads/data/notes/budget 2.xlsx"),
                e("c", "code", &[], "/h/Downloads/data/notes/run.py"),
                e("d", "document", &[], "/h/Downloads/travel report.docx"),
            ],
            &settings(),
            "now",
        );
        assert_eq!(find(&p, "/h/Downloads/data").target.as_deref(), Some("Research/data"));
        assert_eq!(find(&p, "/h/Downloads/travel report.docx").target.as_deref(), Some("School/Travel"));
    }

    #[test]
    fn course_paths_with_and_without_a_school_level() {
        let v = |s: &str| s.split('/').map(String::from).collect::<Vec<_>>();
        assert_eq!(course_of(&v("courses/uni/term_1/seminar/a.pdf")).map(|x| (x.0, x.1)), Some(("term_1".into(), Some("seminar".into()))));
        assert_eq!(course_of(&v("courses/2025-Fall/ML/a.pdf")).map(|x| (x.0, x.1)), Some(("2025-Fall".into(), Some("ML".into()))));
        assert_eq!(course_of(&v("courses/online/web/3_html/a.html")).map(|x| (x.0, x.1)), Some(("online".into(), Some("web".into()))));
        assert_eq!(course_of(&v("courses/topic/a.pdf")).map(|x| (x.0, x.1)), Some(("topic".into(), None)));
        assert!(plain("term_1_1") && !plain("資料夾"));
    }
}
