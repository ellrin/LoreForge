//! The scan report: `reports/<YYYY-MM-DD>-scan.{md,json}` (a second scan on
//! the same day gets `-scan-2`, …; nothing is overwritten).
//!
//! Numbers are about this scan's roots only: totals by kind, duplicate groups
//! and wasted bytes, coverage of each time axis (share of distinct contents
//! that have it), contents that exist only once within the scanned folders,
//! download source domains, projects (remote host only), what was skipped and
//! why. Zero-byte files are never counted as duplicates or single copies.

use crate::CatalogError;
use crate::record::axis;
use crate::scan::{SkipReason, Skipped, Walk};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize)]
pub struct KindTotal {
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DupGroup {
    pub sha256: String,
    pub size: u64,
    pub kind: String,
    pub paths: Vec<String>,
    pub wasted_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Coverage {
    pub axis: String,
    /// Distinct contents with at least one value on this axis.
    pub contents: u64,
    pub percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Domain {
    pub domain: String,
    pub contents: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectLine {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub path: String,
    pub markers: Vec<String>,
    pub remote_hosts: Vec<String>,
    pub git_first: Option<String>,
    pub git_last: Option<String>,
    /// Packages and datasets: files and bytes inside (not catalogued one by one).
    pub files: Option<u64>,
    pub bytes: Option<u64>,
    pub main_ext: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Folder {
    pub folder: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SingleCopy {
    pub path: String,
    pub size: u64,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub started_at: String,
    pub finished_at: String,
    pub roots: Vec<String>,
    /// Files (paths) catalogued.
    pub files: u64,
    pub bytes: u64,
    /// Distinct contents.
    pub contents: u64,
    pub content_bytes: u64,
    pub by_kind: BTreeMap<String, KindTotal>,
    pub duplicate_groups: u64,
    pub wasted_bytes: u64,
    pub duplicates: Vec<DupGroup>,
    pub coverage: Vec<Coverage>,
    /// Non-empty contents found at exactly one path within the scanned folders.
    pub single_copy_count: u64,
    pub single_copy_bytes: u64,
    pub single_copies: Vec<SingleCopy>,
    /// Contents with a download URL (kMDItemWhereFroms).
    pub with_download_url: u64,
    pub download_domains: Vec<Domain>,
    /// How files were counted (packages and datasets are one record each).
    pub counting: String,
    pub projects: Vec<ProjectLine>,
    pub packages: Vec<ProjectLine>,
    pub datasets: Vec<ProjectLine>,
    pub skipped_counts: BTreeMap<String, u64>,
    pub skipped: Vec<Skipped>,
    pub ds_store: u64,
    /// First-level folders by file count (to spot app bundles and the like).
    pub top_folders: Vec<Folder>,
    pub hashed_files: u64,
    pub hashed_bytes: u64,
    pub cached_files: u64,
}

/// One line in every report: the counting rules (changed after the first
/// reports, which walked packages and datasets file by file).
pub const COUNTING: &str = "計數方式：macOS 套件（.app 等）與資料集資料夾（超過 500 個檔、九成同一副檔名）各算一筆、不逐檔計入；與更早（逐檔計入）的報告數字不能直接比較。";

fn pct(n: u64, d: u64) -> f64 {
    if d == 0 { 0.0 } else { (n as f64 * 1000.0 / d as f64).round() / 10.0 }
}

pub fn build(roots: &[PathBuf], w: &Walk, started: DateTime<Utc>, finished: DateTime<Utc>) -> Report {
    let mut by_kind: BTreeMap<String, KindTotal> = BTreeMap::new();
    let mut groups: BTreeMap<&str, Vec<&crate::scan::Seen>> = BTreeMap::new();
    for s in &w.files {
        let k = by_kind.entry(s.kind.clone()).or_default();
        k.files += 1;
        k.bytes += s.size;
        groups.entry(&s.sha256).or_default().push(s);
    }
    let contents = groups.len() as u64;
    let content_bytes = groups.values().map(|g| g[0].size).sum();

    let mut duplicates: Vec<DupGroup> = groups
        .iter()
        .filter(|(_, g)| g.len() > 1 && g[0].size > 0)
        .map(|(sha, g)| DupGroup {
            sha256: sha.to_string(),
            size: g[0].size,
            kind: g[0].kind.clone(),
            paths: g.iter().map(|s| s.path.clone()).collect(),
            wasted_bytes: g[0].size * (g.len() as u64 - 1),
        })
        .collect();
    duplicates.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes).then(a.sha256.cmp(&b.sha256)));
    let wasted_bytes = duplicates.iter().map(|d| d.wasted_bytes).sum();

    let mut axes: HashMap<&str, HashSet<&str>> = HashMap::new();
    for s in &w.files {
        for t in &s.times {
            axes.entry(&s.sha256).or_default().insert(t.axis.as_str());
        }
    }
    let coverage = axis::FILE_AXES
        .iter()
        .map(|a| {
            let n = axes.values().filter(|set| set.contains(a)).count() as u64;
            Coverage { axis: a.to_string(), contents: n, percent: pct(n, contents) }
        })
        .collect();

    let mut single_copies: Vec<SingleCopy> =
        groups.values().filter(|g| g.len() == 1 && g[0].size > 0).map(|g| SingleCopy { path: g[0].path.clone(), size: g[0].size, kind: g[0].kind.clone() }).collect();
    single_copies.sort_by(|a, b| b.size.cmp(&a.size).then(a.path.cmp(&b.path)));
    let single_copy_bytes = single_copies.iter().map(|s| s.size).sum();

    let mut domains: BTreeMap<String, u64> = BTreeMap::new();
    let mut with_download_url = 0;
    for g in groups.values() {
        if let Some(u) = g.iter().find_map(|s| s.download.urls.first()) {
            with_download_url += 1;
            let host = crate::project::url_host(u).unwrap_or_else(|| u.split(':').next().unwrap_or("?").to_string() + ":");
            *domains.entry(host).or_default() += 1;
        }
    }
    let mut download_domains: Vec<Domain> = domains.into_iter().map(|(domain, contents)| Domain { domain, contents }).collect();
    download_domains.sort_by(|a, b| b.contents.cmp(&a.contents).then(a.domain.cmp(&b.domain)));

    let lines = |kind: &str| -> Vec<ProjectLine> {
        w.units
        .iter()
        .filter(|p| p.kind == kind)
        .map(|p| {
            let t = |ax: &str| p.times.iter().find(|t| t.axis == ax).map(|t| t.value.clone());
            let mut hosts: Vec<String> = p.remotes.iter().filter_map(|r| crate::project::url_host(r)).collect();
            hosts.dedup();
            ProjectLine {
                id: p.id.clone(),
                kind: p.kind.clone(),
                name: p.name.clone(),
                path: p.seen_at.first().map(|s| s.path.clone()).unwrap_or_default(),
                markers: p.markers.clone(),
                remote_hosts: hosts,
                git_first: t(axis::GIT_FIRST),
                git_last: t(axis::GIT_LAST),
                files: p.files,
                bytes: p.bytes,
                main_ext: p.main_ext.clone(),
            }
        })
        .collect()
    };
    let projects = lines(crate::project::KIND_PROJECT);
    let packages = lines(crate::project::KIND_PACKAGE);
    let datasets = lines(crate::project::KIND_DATASET);

    let mut skipped_counts = BTreeMap::new();
    for s in &w.skipped {
        *skipped_counts.entry(reason_id(s.reason).to_string()).or_insert(0) += 1;
    }

    let mut folders: BTreeMap<String, Folder> = BTreeMap::new();
    for s in &w.files {
        if let Some(t) = &s.top {
            let f = folders.entry(format!("{}/{t}", s.root)).or_insert_with(|| Folder { folder: format!("{}/{t}", s.root), files: 0, bytes: 0 });
            f.files += 1;
            f.bytes += s.size;
        }
    }
    let mut top_folders: Vec<Folder> = folders.into_values().collect();
    top_folders.sort_by(|a, b| b.files.cmp(&a.files).then(a.folder.cmp(&b.folder)));
    top_folders.truncate(20);

    let ts = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Report {
        started_at: ts(started),
        finished_at: ts(finished),
        roots: roots.iter().map(|r| r.to_string_lossy().to_string()).collect(),
        files: w.files.len() as u64,
        bytes: w.files.iter().map(|s| s.size).sum(),
        contents,
        content_bytes,
        by_kind,
        duplicate_groups: duplicates.len() as u64,
        wasted_bytes,
        duplicates,
        coverage,
        single_copy_count: single_copies.len() as u64,
        single_copy_bytes,
        single_copies,
        with_download_url,
        download_domains,
        counting: COUNTING.into(),
        projects,
        packages,
        datasets,
        skipped_counts,
        skipped: w.skipped.clone(),
        ds_store: w.ds_store,
        top_folders,
        hashed_files: w.hashed_files,
        hashed_bytes: w.hashed_bytes,
        cached_files: w.cached_files,
    }
}

pub fn reason_id(r: SkipReason) -> &'static str {
    match r {
        SkipReason::Unreadable => "unreadable",
        SkipReason::Placeholder => "placeholder",
        SkipReason::Symlink => "symlink",
        SkipReason::Special => "special",
    }
}

fn reason_label(r: &str) -> &'static str {
    match r {
        "unreadable" => "無法讀取",
        "placeholder" => "iCloud 佔位檔／未下載（不下載）",
        "symlink" => "符號連結（不跟隨）",
        "special" => "特殊檔案",
        _ => "其他",
    }
}

fn axis_label(a: &str) -> &'static str {
    match a {
        axis::FS_CREATED => "建立時間（檔案系統）",
        axis::FS_MODIFIED => "修改時間（檔案系統）",
        axis::DOWNLOADED => "下載時間（quarantine）",
        axis::LAST_OPENED => "最後打開（lastuseddate）",
        axis::DOC_CREATED => "文件內建立日期（PDF／Office）",
        axis::DOC_MODIFIED => "文件內修改日期（PDF／Office）",
        _ => "其他",
    }
}

/// `1.2 GB` style sizes (decimal units, like Finder).
pub fn human(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", units[i]) }
}

pub fn markdown(r: &Report) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "# LoreShelf 掃描報告\n");
    let _ = writeln!(s, "- 時間：{} → {}", r.started_at, r.finished_at);
    for root in &r.roots {
        let _ = writeln!(s, "- 掃描：`{root}`");
    }
    let _ = writeln!(s, "- 檔案 {} 個，{}；不同內容 {} 份，{}", r.files, human(r.bytes), r.contents, human(r.content_bytes));
    let _ = writeln!(s, "- 本次讀取（雜湊）{} 個檔案 {}，沿用快取 {} 個", r.hashed_files, human(r.hashed_bytes), r.cached_files);
    let _ = writeln!(s, "- {}\n", r.counting);

    let _ = writeln!(s, "## 種類\n\n| 種類 | 檔案 | 大小 |\n|---|---:|---:|");
    let mut kinds: Vec<_> = r.by_kind.iter().collect();
    kinds.sort_by_key(|k| std::cmp::Reverse(k.1.bytes));
    for (k, t) in kinds {
        let _ = writeln!(s, "| {k} | {} | {} |", t.files, human(t.bytes));
    }

    let _ = writeln!(s, "\n## 重複\n\n{} 組重複，多佔 {}（不含 0 位元組檔案）。\n", r.duplicate_groups, human(r.wasted_bytes));
    for d in r.duplicates.iter().take(30) {
        let _ = writeln!(s, "- {} × {}（{}，多佔 {}）", d.paths.len(), human(d.size), d.kind, human(d.wasted_bytes));
        for p in &d.paths {
            let _ = writeln!(s, "  - `{p}`");
        }
    }
    if r.duplicates.len() > 30 {
        let _ = writeln!(s, "- …其餘 {} 組在 JSON", r.duplicates.len() - 30);
    }

    let _ = writeln!(s, "\n## 時間軸覆蓋率（以不同內容計）\n\n| 時間軸 | 有值 | 比例 |\n|---|---:|---:|");
    for c in &r.coverage {
        let _ = writeln!(s, "| {} | {} | {:.1}% |", axis_label(&c.axis), c.contents, c.percent);
    }

    let _ = writeln!(
        s,
        "\n## 在掃描的資料夾內只有一份\n\n{} 份，共 {}（只就這次掃描的資料夾而言；別處可能還有副本）。最大的 20 份：\n",
        r.single_copy_count,
        human(r.single_copy_bytes)
    );
    for c in r.single_copies.iter().take(20) {
        let _ = writeln!(s, "- {}（{}）`{}`", human(c.size), c.kind, c.path);
    }

    let _ = writeln!(s, "\n## 下載來源\n\n{} 份內容有下載網址。\n\n| 網域 | 內容數 |\n|---|---:|", r.with_download_url);
    for d in r.download_domains.iter().take(20) {
        let _ = writeln!(s, "| {} | {} |", d.domain, d.contents);
    }

    let _ = writeln!(s, "\n## 專案資料夾（整包一個單位，不往內讀）\n\n{} 個。\n", r.projects.len());
    if !r.projects.is_empty() {
        let _ = writeln!(s, "| 名稱 | 標記 | 遠端主機 | git 時間 |\n|---|---|---|---|");
        for p in &r.projects {
            let range = match (&p.git_first, &p.git_last) {
                (Some(a), Some(b)) => format!("{} → {}", &a[..10.min(a.len())], &b[..10.min(b.len())]),
                _ => String::new(),
            };
            let _ = writeln!(s, "| {} | {} | {} | {range} |", p.name, p.markers.join(", "), p.remote_hosts.join(", "));
        }
    }

    for (title, list) in [("macOS 套件", &r.packages), ("資料集資料夾", &r.datasets)] {
        let files: u64 = list.iter().filter_map(|u| u.files).sum();
        let bytes: u64 = list.iter().filter_map(|u| u.bytes).sum();
        let _ = writeln!(s, "\n## {title}（整包一筆，不逐檔讀）\n\n{} 個，內含 {files} 個檔、{}。\n", list.len(), human(bytes));
        for u in list {
            let ext = u.main_ext.as_deref().map(|e| format!("，主要 .{e}")).unwrap_or_default();
            let _ = writeln!(s, "- {}（{} 個檔，{}{ext}）`{}`", u.name, u.files.unwrap_or(0), human(u.bytes.unwrap_or(0)), u.path);
        }
    }

    let _ = writeln!(s, "\n## 第一層資料夾（檔案數）\n");
    for f in r.top_folders.iter().take(15) {
        let _ = writeln!(s, "- {} 個，{} `{}`", f.files, human(f.bytes), f.folder);
    }

    let _ = writeln!(s, "\n## 略過\n");
    let _ = writeln!(s, "- .DS_Store：{}", r.ds_store);
    for (k, n) in &r.skipped_counts {
        let _ = writeln!(s, "- {}：{n}", reason_label(k));
    }
    for k in r.skipped.iter().take(50) {
        let _ = writeln!(s, "  - {} `{}`", reason_label(reason_id(k.reason)), k.path);
    }
    if r.skipped.len() > 50 {
        let _ = writeln!(s, "  - …其餘 {} 個在 JSON", r.skipped.len() - 50);
    }
    s
}

/// Write `<dir>/<date>-scan.{md,json}` under free names.
pub fn write(dir: &Path, r: &Report) -> Result<(PathBuf, PathBuf), CatalogError> {
    std::fs::create_dir_all(dir).map_err(|e| CatalogError::io(dir, e))?;
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut n = 1;
    let (md, json) = loop {
        let stem = if n == 1 { format!("{date}-scan") } else { format!("{date}-scan-{n}") };
        let (md, json) = (dir.join(format!("{stem}.md")), dir.join(format!("{stem}.json")));
        if !md.exists() && !json.exists() {
            break (md, json);
        }
        n += 1;
    };
    let j = serde_json::to_vec_pretty(r).map_err(|e| CatalogError::io(&json, std::io::Error::other(e)))?;
    crate::store::write_atomic(&json, &j)?;
    crate::store::write_atomic(&md, markdown(r).as_bytes())?;
    Ok((md, json))
}
