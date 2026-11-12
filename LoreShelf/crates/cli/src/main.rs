//! `loreshelf` — LoreShelf wave 1 command line.
//!
//! ```text
//! loreshelf scan <dir>...   read-only scan into the local catalog; prints a summary and the report path
//! loreshelf report          path of the latest report
//! ```
//!
//! Catalog: `$LORESHELF_HOME`, else `~/Library/Application Support/LoreForge/LoreShelf`;
//! hash cache: `<cache>/LoreForge/shelf/` (`$LOREFORGE_CACHE` overrides the base).

use lore_catalog::report::human;
use lore_catalog::{Catalog, Guard, ScanConfig, store};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "用法：\n  loreshelf scan <資料夾>...   只讀掃描，寫入本機目錄與報告\n  loreshelf report            最新報告的位置";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    let catalog_dir = store::default_catalog_dir().ok_or("找不到目錄位置（請設定 LORESHELF_HOME）")?;
    match args.first().map(String::as_str) {
        Some("scan") if args.len() > 1 => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let cfg = ScanConfig { roots: args[1..].iter().map(expand).collect(), catalog_dir, cache_dir, guard: Guard::default() };
            let out = lore_catalog::scan(&cfg).map_err(|e| e.to_string())?;
            Ok(summary(&out))
        }
        Some("report") => Catalog::latest_report(&catalog_dir).map(|p| p.display().to_string()).ok_or_else(|| "還沒有報告；先執行 loreshelf scan <資料夾>".to_string()),
        _ => Err(USAGE.into()),
    }
}

fn expand(s: &String) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or_else(|| PathBuf::from(s)),
        None => PathBuf::from(s),
    }
}

fn summary(out: &lore_catalog::ScanOutcome) -> String {
    let r = &out.report;
    let skipped: u64 = r.skipped_counts.values().sum();
    format!(
        "檔案 {}（{}），不同內容 {}\n重複 {} 組，多佔 {}\n在掃描的資料夾內只有一份：{}\n專案 {} 個、套件 {} 個、資料集 {} 個（各算一筆）；略過 {} 個\n報告：{}",
        r.files,
        human(r.bytes),
        r.contents,
        r.duplicate_groups,
        human(r.wasted_bytes),
        r.single_copy_count,
        r.projects.len(),
        r.packages.len(),
        r.datasets.len(),
        skipped,
        out.report_md.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_and_expand() {
        assert!(run(&["nope".into()]).unwrap_err().contains("loreshelf scan"));
        assert!(run(&["scan".into()]).is_err(), "scan needs a folder");
        if let Some(h) = std::env::var_os("HOME") {
            assert_eq!(expand(&"~/x".to_string()), PathBuf::from(h).join("x"));
        }
        assert_eq!(expand(&"/a/b".to_string()), PathBuf::from("/a/b"));
    }

    #[test]
    fn scan_through_the_library_with_temp_dirs() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("in/sub")).unwrap();
        std::fs::write(base.join("in/a.txt"), "ellrin").unwrap();
        std::fs::write(base.join("in/sub/b.txt"), "ellrin").unwrap();
        let cfg = ScanConfig {
            roots: vec![base.join("in")],
            catalog_dir: base.join("catalog"),
            cache_dir: base.join("cache"),
            guard: Guard { cloud: vec![], forbidden: vec![] },
        };
        let out = lore_catalog::scan(&cfg).unwrap();
        let s = summary(&out);
        assert!(s.contains("重複 1 組") && s.contains("報告："), "{s}");
        assert_eq!(Catalog::latest_report(&base.join("catalog")), Some(out.report_md));
    }
}
