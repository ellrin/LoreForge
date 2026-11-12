//! Offline tests: temp dirs and synthetic files only.

use super::*;
use crate::project::ProjectRecord;
use crate::record::axis;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write(p: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git").arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").output().unwrap()
}

/// A guard whose "iCloud" folder is inside the temp dir.
fn guard(t: &Path) -> Guard {
    Guard { cloud: vec![t.join("Library/Mobile Documents")], forbidden: vec![] }
}

/// A small valid PDF with an Info dictionary (xref offsets computed).
fn tiny_pdf(info: &str) -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_string(),
        info.to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
    out
}

fn tiny_pptx(created: &str, modified: &str) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("[Content_Types].xml", o).unwrap();
        z.write_all(b"<Types/>").unwrap();
        z.start_file("docProps/core.xml", o).unwrap();
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:creator>ellrin</dc:creator><cp:lastModifiedBy>daidai</cp:lastModifiedBy><dcterms:created xsi:type="dcterms:W3CDTF">{created}</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">{modified}</dcterms:modified></cp:coreProperties>"#
        );
        z.write_all(xml.as_bytes()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

struct Fixture {
    _t: tempfile::TempDir,
    base: PathBuf,
    root: PathBuf,
    cfg: ScanConfig,
}

/// A synthetic "Downloads": duplicates, a PDF, a pptx, empty files, a git
/// project, a LaTeX project, node_modules and build outside a project, a
/// package, a dataset, a mixed big folder, a symlink, an iCloud placeholder.
fn fixture() -> Fixture {
    let t = tempfile::tempdir().unwrap();
    let base = t.path().canonicalize().unwrap();
    let root = base.join("Downloads");
    write(&root.join("a.txt"), b"hello");
    write(&root.join("sub/b.txt"), b"hello");
    write(&root.join("sub/deep/c.txt"), b"hello");
    write(&root.join("paper.pdf"), &tiny_pdf("<< /Title (x) /CreationDate (D:20210304050607+08'00') /ModDate (D:2022) >>"));
    write(&root.join("deck.pptx"), &tiny_pptx("2020-01-02T03:04:05Z", "2020-02-03T00:00:00Z"));
    write(&root.join("broken.pdf"), b"%PDF-1.7\n garbage /CreationDate <FEFF0044003A0032003000310039> more");
    write(&root.join("noext"), b"%PDF-1.4 tiny");
    write(&root.join("empty1"), b"");
    write(&root.join("empty2"), b"");
    write(&root.join(".DS_Store"), b"junk");
    write(&root.join(".big.zip.icloud"), b"placeholder");
    // Git project: config with a remote carrying credentials, a reflog.
    let proj = root.join("tool");
    write(&proj.join(".git/config"), b"[core]\n\tbare = false\n[remote \"origin\"]\n\turl = https://ellrin:tok@example.org/ellrin/tool.git\n");
    write(
        &proj.join(".git/logs/HEAD"),
        b"0000 1111 ellrin <ellrin@example.com> 1600000000 +0800\tcommit (initial): a\n1111 2222 ellrin <ellrin@example.com> 1700000000 +0800\tcommit: b\n",
    );
    write(&proj.join("src/inner.txt"), b"inside a project");
    write(&proj.join("README.md"), b"tool");
    // LaTeX project by .tex + .bib, and one by main.tex inside an ordinary folder.
    write(&root.join("thesis/chapter.tex"), b"\\section{x}");
    write(&root.join("thesis/refs.bib"), b"@article{x}");
    write(&root.join("misc/talk/main.tex"), b"\\documentclass{beamer}");
    write(&root.join("misc/note.md"), b"note");
    write(&root.join("node_modules/x.js"), b"1");
    write(&root.join("build/out.log"), b"build log");
    write(&root.join("Viewer.app/Contents/MacOS/viewer"), b"binary");
    write(&root.join("Viewer.app/Contents/Info.plist"), b"<plist/>");
    for i in 0..501 {
        write(&root.join(format!("data/part{}/{i}.json", i % 3)), format!("{{\"i\":{i}}}").as_bytes());
    }
    write(&root.join("data/README.txt"), b"about the data");
    for i in 0..600 {
        let ext = if i % 2 == 0 { "txt" } else { "csv" };
        write(&root.join(format!("mixed/{i}.{ext}")), format!("m{i}").as_bytes());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("a.txt"), root.join("link.txt")).unwrap();
    let cfg = ScanConfig { roots: vec![root.clone()], catalog_dir: base.join("catalog"), cache_dir: base.join("cache/shelf"), guard: guard(&base) };
    Fixture { _t: t, base, root, cfg }
}

fn sha(b: &[u8]) -> String {
    hash::sha256_bytes(b)
}

#[test]
fn scan_records_duplicates_projects_times_and_skips() {
    let f = fixture();
    let out = scan(&f.cfg).unwrap();
    let r = &out.report;
    let files = out.catalog.join("files");

    // Duplicates: one record, three paths.
    let hello = FileRecord::load(&files, &sha(b"hello")).unwrap();
    assert_eq!(hello.seen_at.len(), 3);
    assert_eq!(hello.names, vec!["a.txt", "b.txt", "c.txt"]);
    assert_eq!(hello.kind, "text");
    assert_eq!(r.duplicate_groups, 1, "empty files are not a duplicate group");
    assert_eq!(r.wasted_bytes, 10);
    assert!(hello.has_axis(axis::FS_MODIFIED) && hello.has_axis(axis::FS_CREATED));

    // Projects: one record each, never descended into.
    assert!(FileRecord::load(&files, &sha(b"inside a project")).is_none());
    assert!(FileRecord::load(&files, &sha(b"\\section{x}")).is_none());
    assert!(FileRecord::load(&files, &sha(b"note")).is_some(), "the folder around a project is still read");
    let mut names: Vec<_> = r.projects.iter().map(|p| (p.name.clone(), p.markers.join(","))).collect();
    names.sort();
    assert_eq!(names, vec![("talk".into(), "latex".into()), ("thesis".into(), "latex".into()), ("tool".into(), "git".into())]);
    let tool = r.projects.iter().find(|p| p.name == "tool").unwrap();
    assert_eq!(tool.remote_hosts, vec!["example.org"]);
    assert_eq!(tool.git_first.as_deref(), Some("2020-09-13T12:26:40Z"));
    assert_eq!(tool.git_last.as_deref(), Some("2023-11-14T22:13:20Z"));
    let rec = ProjectRecord::load(&out.catalog.join("projects"), &tool.id).unwrap();
    assert_eq!(rec.remotes, vec!["https://example.org/ellrin/tool.git"], "credentials stripped");
    let json = std::fs::read_to_string(out.catalog.join("projects").join(format!("{}.json", tool.id))).unwrap();
    assert!(!json.contains("example.com"), "no e-mail from the reflog");
    assert!(tool.id.starts_with("proj-") && tool.id.len() == 21);

    // Document dates.
    let pdf = FileRecord::load(&files, &sha(&tiny_pdf("<< /Title (x) /CreationDate (D:20210304050607+08'00') /ModDate (D:2022) >>"))).unwrap();
    let doc = |rec: &FileRecord, ax: &str| rec.times.iter().find(|t| t.axis == ax).map(|t| (t.value.clone(), t.source.clone()));
    assert_eq!(doc(&pdf, axis::DOC_CREATED), Some(("2021-03-03T21:06:07Z".into(), "pdf:Info".into())));
    assert_eq!(doc(&pdf, axis::DOC_MODIFIED).unwrap().0, "2022-01-01T00:00:00Z");
    let broken = FileRecord::load(&files, &sha(b"%PDF-1.7\n garbage /CreationDate <FEFF0044003A0032003000310039> more")).unwrap();
    assert_eq!(doc(&broken, axis::DOC_CREATED).unwrap().0, "2019-01-01T00:00:00Z", "byte-scan fallback, UTF-16 hex string");
    let deck = FileRecord::load(&files, &sha(&tiny_pptx("2020-01-02T03:04:05Z", "2020-02-03T00:00:00Z"))).unwrap();
    assert_eq!(deck.kind, "slides");
    assert_eq!(doc(&deck, axis::DOC_CREATED), Some(("2020-01-02T03:04:05Z".into(), "ooxml:docProps/core.xml".into())));
    let raw = std::fs::read_to_string(FileRecord::path(&files, &deck.sha256)).unwrap();
    assert!(!raw.contains("daidai") && !raw.contains("ellrin"), "no author names from core.xml");
    assert_eq!(FileRecord::load(&files, &sha(b"%PDF-1.4 tiny")).unwrap().kind, "pdf", "sniffed from magic");

    // Skips.
    let md = std::fs::read_to_string(&out.report_md).unwrap();
    assert_eq!(r.ds_store, 1);
    assert_eq!(r.skipped_counts.get("placeholder"), Some(&1));
    assert!(!r.skipped_counts.contains_key("excluded"));
    assert!(FileRecord::load(&files, &sha(b"1")).is_some(), "node_modules outside a project is scanned");
    assert!(FileRecord::load(&files, &sha(b"build log")).is_some(), "build outside a project is scanned");

    // Package and dataset: one record each, nothing inside hashed.
    assert!(FileRecord::load(&files, &sha(b"binary")).is_none());
    assert!(FileRecord::load(&files, &sha(b"{\"i\":7}")).is_none());
    assert!(FileRecord::load(&files, &sha(b"about the data")).is_none());
    assert!(FileRecord::load(&files, &sha(b"m7")).is_some(), "a big mixed folder is walked");
    assert_eq!(r.packages.len(), 1);
    assert_eq!((r.packages[0].name.as_str(), r.packages[0].files, r.packages[0].kind.as_str()), ("Viewer.app", Some(2), "package"));
    assert!(out.catalog.join("packages").join(format!("{}.json", r.packages[0].id)).exists());
    assert_eq!(r.datasets.len(), 1);
    assert_eq!((r.datasets[0].name.as_str(), r.datasets[0].files, r.datasets[0].main_ext.as_deref()), ("data", Some(502), Some("json")));
    assert!(r.datasets[0].id.starts_with("data-"));
    assert!(md.contains("計數方式"));
    #[cfg(unix)]
    assert_eq!(r.skipped_counts.get("symlink"), Some(&1));
    assert!(FileRecord::load(&files, &sha(b"placeholder")).is_none());
    assert!(FileRecord::load(&files, &sha(b"junk")).is_none());

    // Coverage and single copies.
    let cov = |a: &str| r.coverage.iter().find(|c| c.axis == a).unwrap().contents;
    assert_eq!(cov(axis::FS_MODIFIED), r.contents);
    assert_eq!(cov(axis::DOC_CREATED), 3);
    assert!(r.single_copies.iter().all(|c| c.size > 0));
    assert!(!r.single_copies.iter().any(|c| c.path.ends_with("a.txt")));
    assert!(out.report_md.exists() && out.report_json.exists());
    assert!(md.contains("在掃描的資料夾內只有一份"));
    assert!(md.contains("example.org") && !md.contains("tok@"), "projects show the host only");

    // Nothing was written into the scanned folder.
    let mut listing = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(d).unwrap() {
            let e = e.unwrap();
            out.push(e.path());
            if e.file_type().unwrap().is_dir() {
                walk(&e.path(), out);
            }
        }
    }
    walk(&f.root, &mut listing);
    assert_eq!(listing.len(), 1145, "{listing:#?}");
}

#[test]
fn rescan_uses_the_cache_and_merges_records() {
    let f = fixture();
    let first = scan(&f.cfg).unwrap();
    assert!(first.report.hashed_files > 0 && first.report.cached_files == 0);
    let files = first.catalog.join("files");
    let p = FileRecord::path(&files, &sha(b"hello"));
    // A field from a later version or a hand edit survives.
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["tags"] = serde_json::json!(["greeting"]);
    v["seen_at"][0]["first_seen"] = "2000-01-01T00:00:00Z".into();
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(1100));
    let second = scan(&f.cfg).unwrap();
    assert_eq!(second.report.hashed_files, 0, "unchanged files are not read again");
    assert_eq!(second.report.cached_files, first.report.files);
    assert_ne!(second.report_md, first.report_md, "reports never overwrite");
    let rec = FileRecord::load(&files, &sha(b"hello")).unwrap();
    assert_eq!(rec.extra.get("tags"), Some(&serde_json::json!(["greeting"])));
    assert_eq!(rec.seen_at.len(), 3);
    assert_eq!(rec.seen_at[0].first_seen, "2000-01-01T00:00:00Z");
    assert_ne!(rec.seen_at[0].last_seen, rec.seen_at[0].first_seen);
    assert_eq!(rec.times.iter().filter(|t| t.axis == axis::FS_MODIFIED).count(), rec.times.iter().filter(|t| t.axis == axis::FS_MODIFIED).map(|t| &t.value).collect::<std::collections::HashSet<_>>().len());

    // A changed file is read again; the doc dates come back from the cache otherwise.
    write(&f.root.join("sub/b.txt"), b"changed");
    let third = scan(&f.cfg).unwrap();
    assert_eq!(third.report.hashed_files, 1);
    assert_eq!(third.report.duplicate_groups, 1);
    assert_eq!(third.report.coverage.iter().find(|c| c.axis == axis::DOC_CREATED).unwrap().contents, 3);
    let cache: serde_json::Value = serde_json::from_slice(&std::fs::read(f.base.join("cache/shelf").join(hash::CACHE_FILE)).unwrap()).unwrap();
    assert_eq!(cache.as_object().unwrap().len() as u64, third.report.files);
}

#[test]
fn guard_refuses_remote_icloud_and_the_repo() {
    let t = tempfile::tempdir().unwrap();
    let base = t.path().canonicalize().unwrap();
    let g = guard(&base);
    // iCloud Drive.
    let icloud = base.join("Library/Mobile Documents/com~apple~CloudDocs/LoreShelf");
    assert!(matches!(g.check(&icloud), Err(CatalogError::Guard(_))));
    assert!(Catalog::open(&icloud, &g).is_err());
    assert!(!icloud.exists(), "nothing created when refused");
    assert!(Guard { cloud: vec![], forbidden: vec![] }.check(&base.join("x/Mobile Documents/y")).is_err());
    // Inside the LoreForge repository.
    assert!(Guard::default().check(&Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog")).is_err());
    // Inside a work tree with a remote.
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(git(&repo, &["init", "-q"]).status.success());
    assert!(g.check(&repo.join("catalog")).is_ok(), "no remote yet");
    assert!(git(&repo, &["remote", "add", "origin", "https://example.org/x.git"]).status.success());
    let e = g.check(&repo.join("catalog")).unwrap_err().to_string();
    assert!(e.contains("origin"), "{e}");
    // The default location is outside any repo.
    if let Some(d) = store::default_catalog_dir() {
        assert!(d.ends_with("LoreForge/LoreShelf") || std::env::var_os("LORESHELF_HOME").is_some());
    }
}

#[test]
fn catalog_is_local_only_with_a_refusing_pre_push_hook() {
    let t = tempfile::tempdir().unwrap();
    let base = t.path().canonicalize().unwrap();
    let dir = base.join("catalog");
    let c = Catalog::open(&dir, &guard(&base)).unwrap();
    assert!(dir.join(".git").is_dir());
    assert!(dir.join(LOCAL_ONLY_MARKER).exists());
    let hook = dir.join(".git/hooks/pre-push");
    let body = std::fs::read_to_string(&hook).unwrap();
    assert!(body.contains("exit 1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&hook).unwrap().permissions().mode() & 0o111, 0o111);
    }
    let hp = String::from_utf8(git(&dir, &["config", "--local", "--get", "core.hooksPath"]).stdout).unwrap();
    assert_eq!(hp.trim(), dir.join(".git/hooks").to_string_lossy(), "a global hooksPath cannot bypass the hook");

    // Opening again is fine; a remote added by hand makes the push fail and the next open refuse.
    Catalog::open(&c.dir, &guard(&base)).unwrap();
    let bare = base.join("remote.git");
    assert!(Command::new("git").args(["init", "-q", "--bare"]).arg(&bare).status().unwrap().success());
    write(&dir.join("files/x.json"), b"{}");
    let id = ["-c", "user.name=ellrin", "-c", "user.email=ellrin@example.com", "-c", "commit.gpgsign=false"];
    assert!(git(&dir, &["add", "-A"]).status.success());
    assert!(git(&dir, &[&id[..], &["commit", "-q", "--no-verify", "-m", "x"]].concat()).status.success());
    assert!(git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]).status.success());
    let push = git(&dir, &["push", "-q", "origin", "HEAD:main"]);
    assert!(!push.status.success(), "pre-push hook refuses");
    assert!(String::from_utf8_lossy(&push.stderr).contains("local-only"));
    assert!(matches!(Catalog::open(&dir, &guard(&base)), Err(CatalogError::Guard(_))));
}

#[test]
fn scan_refuses_catalog_inside_a_scanned_folder_and_unreadable_roots() {
    let f = fixture();
    let cfg = ScanConfig { catalog_dir: f.root.join("catalog"), ..f.cfg.clone() };
    assert!(matches!(scan(&cfg), Err(CatalogError::Guard(_))));
    let missing = ScanConfig { roots: vec![f.base.join("nope")], ..f.cfg.clone() };
    assert!(matches!(scan(&missing), Err(CatalogError::Root { .. })));
}

#[test]
fn project_detection_rules() {
    let m = |names: &[(&str, bool)]| project::markers(&names.iter().map(|(n, d)| (n.to_string(), *d)).collect::<Vec<_>>());
    assert_eq!(m(&[(".git", true), ("Cargo.toml", false)]), vec!["git", "cargo"]);
    assert_eq!(m(&[(".git", false)]), vec!["git"], "a .git file (worktree / submodule)");
    assert_eq!(m(&[("a.tex", false)]), Vec::<String>::new(), "a lone .tex is not a project");
    assert_eq!(m(&[("a.tex", false), ("b.bib", false)]), vec!["latex"]);
    assert_eq!(m(&[("App.xcodeproj", true), ("package.json", false), ("pyproject.toml", false)]), vec!["npm", "xcode", "python"]);
    assert!(m(&[("notes.txt", false), ("Cargo.toml.bak", false)]).is_empty());
}

#[test]
fn parsers() {
    use chrono::{TimeZone, Utc};
    use times::*;
    assert_eq!(parse_pdf_date("D:20230102030405Z").unwrap(), Utc.with_ymd_and_hms(2023, 1, 2, 3, 4, 5).unwrap());
    assert_eq!(parse_pdf_date("D:20230102030405-05'00'").unwrap(), Utc.with_ymd_and_hms(2023, 1, 2, 8, 4, 5).unwrap());
    assert_eq!(parse_pdf_date("2019").unwrap(), Utc.with_ymd_and_hms(2019, 1, 1, 0, 0, 0).unwrap());
    assert!(parse_pdf_date("D:19").is_none() && parse_pdf_date("D:00000101").is_none() && parse_pdf_date("D:20231345").is_none());
    let (t, agent) = parse_quarantine(b"0083;65553f00;Safari;ABCD-1234").unwrap();
    assert_eq!(t.unwrap(), Utc.timestamp_opt(0x6555_3f00, 0).unwrap());
    assert_eq!(agent.as_deref(), Some("Safari"));
    assert_eq!(parse_quarantine(b"0081;;;").unwrap(), (None, None));
    let mut lu = 1_700_000_000i64.to_le_bytes().to_vec();
    lu.extend(5i64.to_le_bytes());
    assert_eq!(parse_last_used(&lu).unwrap(), Utc.timestamp_opt(1_700_000_000, 5).unwrap());
    assert!(parse_last_used(&[0; 8]).is_none());
    let mut pl = Vec::new();
    plist::Value::Array(vec!["https://files.example.org/a.pdf".into(), "https://example.org/page".into()]).to_writer_binary(&mut pl).unwrap();
    assert_eq!(parse_where_froms(&pl), vec!["https://files.example.org/a.pdf", "https://example.org/page"]);
    assert!(parse_where_froms(b"not a plist").is_empty());
    assert_eq!(parse_w3c_date("2020-05-06").unwrap(), Utc.with_ymd_and_hms(2020, 5, 6, 0, 0, 0).unwrap());
    assert_eq!(core_xml_dates("<x><dcterms:created>broken</dcterms:created>").0, None);
    for (u, h) in [
        ("https://Example.org:8443/x?y", "example.org"),
        ("git@example.org:ellrin/x.git", "example.org"),
        ("ssh://git@example.org:22/x", "example.org"),
        ("blob:https://example.org/uuid", "example.org"),
    ] {
        assert_eq!(project::url_host(u).as_deref(), Some(h), "{u}");
    }
    assert_eq!(project::url_host("data:text/plain,hi"), None);
    assert_eq!(project::strip_credentials("https://u:p@example.org/x"), "https://example.org/x");
    assert_eq!(project::log_timestamps("a b N <m@x> 12 +0000\tmsg <odd> 99\n"), vec![12]);
    assert_eq!(scan::kind_from_magic(b"\x89PNG\r\n"), "image");
    assert_eq!(scan::kind_from_ext("Report.PDF"), Some("pdf"));
    assert_eq!(scan::kind_from_ext("file.download"), None);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_download_xattrs_become_times_and_sources() {
    let f = fixture();
    let p = f.root.join("paper.pdf");
    let mut pl = Vec::new();
    plist::Value::Array(vec!["https://files.example.org/paper.pdf".into(), "https://example.org/list".into()]).to_writer_binary(&mut pl).unwrap();
    xattr::set(&p, times::XATTR_WHERE_FROMS, &pl).unwrap();
    xattr::set(&p, times::XATTR_QUARANTINE, b"0083;65553f00;Firefox;").unwrap();
    let mut lu = 1_710_000_000i64.to_le_bytes().to_vec();
    lu.extend(0i64.to_le_bytes());
    xattr::set(&p, times::XATTR_LAST_USED, &lu).unwrap();
    let out = scan(&f.cfg).unwrap();
    let rec = FileRecord::load(&out.catalog.join("files"), &sha(&std::fs::read(&p).unwrap())).unwrap();
    let d = rec.download.clone().unwrap();
    assert_eq!(d.urls, vec!["https://files.example.org/paper.pdf"]);
    assert_eq!(d.referrer.as_deref(), Some("https://example.org/list"));
    assert_eq!(d.agent.as_deref(), Some("Firefox"));
    assert!(rec.times.iter().any(|t| t.axis == axis::DOWNLOADED && t.source == "xattr:com.apple.quarantine" && t.value == "2023-11-15T21:58:24Z"));
    assert!(rec.times.iter().any(|t| t.axis == axis::LAST_OPENED && t.value == "2024-03-09T16:00:00Z"));
    assert_eq!(out.report.download_domains[0].domain, "files.example.org");
    assert_eq!(out.report.with_download_url, 1);
    // Reading xattrs left them as they were.
    assert_eq!(xattr::get(&p, times::XATTR_QUARANTINE).unwrap().unwrap(), b"0083;65553f00;Firefox;");
}


fn git_ok(dir: &Path, args: &[&str]) -> String {
    let id = ["-c", "user.name=ellrin", "-c", "user.email=ellrin@example.com", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"];
    let o = git(dir, &[&id[..], args].concat());
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

#[test]
fn git_clones_are_one_record_by_root_commit() {
    let f = fixture();
    let a = f.root.join("repo-a");
    write(&a.join("README.md"), b"repo");
    git_ok(&a, &["init", "-q", "-b", "main"]);
    git_ok(&a, &["add", "-A"]);
    git_ok(&a, &["commit", "-q", "-m", "first"]);
    let root_sha = git_ok(&a, &["rev-parse", "HEAD"]);
    write(&a.join("more.md"), b"more");
    git_ok(&a, &["add", "-A"]);
    git_ok(&a, &["commit", "-q", "-m", "second"]);
    let b = f.root.join("misc/repo-b");
    git_ok(&f.root, &["clone", "-q", a.to_str().unwrap(), b.to_str().unwrap()]);

    let out = scan(&f.cfg).unwrap();
    let id = format!("proj-git-{root_sha}");
    let rec = ProjectRecord::load(&out.catalog.join("projects"), &id).unwrap();
    assert_eq!(rec.root_commit.as_deref(), Some(root_sha.as_str()));
    let mut paths: Vec<_> = rec.seen_at.iter().map(|s| s.path.clone()).collect();
    paths.sort();
    assert_eq!(paths, vec![b.to_string_lossy().to_string(), a.to_string_lossy().to_string()]);
    assert!(rec.remotes.len() == 1, "the clone's origin: {:?}", rec.remotes);
    assert!(rec.fingerprint.is_none());
    // The fake .git without commits keeps a path id.
    assert!(out.report.projects.iter().any(|p| p.name == "tool" && p.id.starts_with("proj-") && !p.id.starts_with("proj-git-")));
    // Scanning did not write into the repos (no index refresh, no locks).
    assert!(git_ok(&a, &["status", "--porcelain"]).is_empty());
}

#[test]
fn moved_project_keeps_its_record() {
    let f = fixture();
    let first = scan(&f.cfg).unwrap();
    let thesis = first.report.projects.iter().find(|p| p.name == "thesis").unwrap().id.clone();
    std::fs::rename(f.root.join("thesis"), f.root.join("misc/thesis-final")).unwrap();
    let second = scan(&f.cfg).unwrap();
    let dir = second.catalog.join("projects");
    let rec = ProjectRecord::load(&dir, &thesis).unwrap();
    assert_eq!(rec.seen_at.len(), 2, "old and new path on one record");
    assert!(rec.seen_at[1].path.ends_with("misc/thesis-final"));
    assert_eq!(project::UnitRecord::load_all(&dir).len(), 3, "no new record for the moved project");
    // A different folder with other names is not merged.
    write(&f.root.join("other/x.tex"), b"x");
    write(&f.root.join("other/y.bib"), b"y");
    let third = scan(&f.cfg).unwrap();
    assert_eq!(project::UnitRecord::load_all(&third.catalog.join("projects")).len(), 4);
    assert_eq!(ProjectRecord::load(&dir, &thesis).unwrap().seen_at.len(), 2, "the moved path stays on the old record");
}

#[test]
fn scans_never_commit_and_edits_do() {
    let f = fixture();
    let out = scan(&f.cfg).unwrap();
    let dir = &out.catalog;
    assert!(!git(dir, &["rev-parse", "--verify", "-q", "HEAD"]).status.success(), "a scan makes no commit");
    let status = git_ok(dir, &["status", "--porcelain", "--untracked-files=all"]);
    let mut lines: Vec<&str> = status.lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["?? .gitignore", "?? .lore-local-only"], "scan output is ignored");
    assert!(git(dir, &["check-ignore", "-q", "files/x.json"]).status.success());
    assert!(git(dir, &["check-ignore", "-q", "reports/x.md"]).status.success());
    assert!(git(dir, &["check-ignore", "-q", "datasets/x.json"]).status.success());

    let c = Catalog { dir: dir.clone() };
    let id = sha(b"hello");
    let commit = c.record_edit(&id, "tags", serde_json::json!(["greeting"])).unwrap().unwrap();
    assert_eq!(git_ok(dir, &["log", "--format=%s"]), format!("tags: {id}"));
    assert_eq!(git_ok(dir, &["rev-parse", "HEAD"]), commit);
    let mut tracked: Vec<String> = git_ok(dir, &["ls-files"]).lines().map(str::to_string).collect();
    tracked.sort();
    assert_eq!(tracked, vec![".gitignore".to_string(), ".lore-local-only".into(), format!("edits/{id}.json")]);
    assert_eq!(c.edit(&id).unwrap()["tags"], serde_json::json!(["greeting"]));
    assert_eq!(c.record_edit(&id, "tags", serde_json::json!(["greeting"])).unwrap(), None, "unchanged: no commit");
    assert!(c.record_edit("../x", "tags", serde_json::json!([])).is_err());
    assert!(c.record_edit(&id, "Tags!", serde_json::json!([])).is_err());

    scan(&f.cfg).unwrap();
    assert_eq!(git_ok(dir, &["rev-list", "--count", "HEAD"]), "1", "rescans still commit nothing");
}

#[test]
fn extract_and_category_rules() {
    use category::*;
    // Image headers.
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(856u32.to_be_bytes());
    png.extend(540u32.to_be_bytes());
    assert_eq!(extract::image_size_bytes(&png), Some((856, 540)));
    assert_eq!(extract::image_size_bytes(b"GIF89a\x10\x00\x20\x00"), Some((16, 32)));
    let jpg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 20, 0, 30, 3, 0, 0, 0];
    assert_eq!(extract::image_size_bytes(&jpg), Some((30, 20)));
    // Text of a PDF made with a DOI.
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("a.pdf");
    std::fs::write(&p, tiny_pdf("<< /Title (x) >>")).unwrap();
    let (pages, _) = extract::pdf_text(&p, 10);
    assert_eq!(pages, Some(1));
    assert_eq!(extract::pdf_text(&t.path().join("missing.pdf"), 10), (None, None));

    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let n = names(&["final paper.pdf"]);
    let doi = Facts { kind: "pdf", names: &n, text: Some("Some title\nhttps://doi.org/10.1016/j.media.2020.101797"), ..Facts::default() };
    assert_eq!(members(&rules(&doi)), vec!["paper"]);
    assert_eq!(rules(&doi)[0].reason, "DOI");
    assert!(has_arxiv("see arXiv: 2101.01234v2"));
    assert!(!has_doi("version 10.1 / build"));
    let host = Facts { kind: "pdf", names: &n, host: Some("link.springer.com"), ..Facts::default() };
    assert_eq!(members(&rules(&host)), vec!["paper"]);
    let m = names(&["latex"]);
    assert_eq!(members(&rules(&Facts { kind: "project", markers: &m, ..Facts::default() })), vec!["paper"]);
    // 證件 by name or card-sized image; a scan PDF alone stays below the bar.
    for n in [&["daidai 護照.jpg"][..], &["ellrin-passport.pdf"], &["Student ID card.png"]] {
        let n = names(n);
        assert_eq!(members(&rules(&Facts { kind: "image", names: &n, ..Facts::default() })), vec!["id-doc"], "{n:?}");
    }
    // Card shape or a one-page scan alone: never 證件 (figures are often card-shaped).
    let n2 = names(&["main.png"]);
    assert!(rules(&Facts { kind: "image", names: &n2, dims: Some((5128, 3208)), ..Facts::default() }).is_empty());
    assert!(rules(&Facts { kind: "image", names: &n2, dims: Some((1011, 638)), ..Facts::default() }).is_empty());
    assert!(rules(&Facts { kind: "pdf", names: &n2, pages: Some(1), text: Some(" "), ..Facts::default() }).is_empty());
    let n3 = names(&["護照 scan.png"]);
    let both = rules(&Facts { kind: "image", names: &n3, dims: Some((1011, 638)), ..Facts::default() });
    assert_eq!((both[0].confidence, both[0].reason.as_str()), (0.95, "name + card-sized image"));
    // 作業 and 票券收據; words, not substrings.
    let hw = names(&["HW3_solution.pdf"]);
    assert_eq!(members(&rules(&Facts { kind: "pdf", names: &hw, ..Facts::default() })), vec!["homework"]);
    let not_hw = names(&["show.pdf", "whatever.txt"]);
    assert!(members(&rules(&Facts { kind: "pdf", names: &not_hw, ..Facts::default() })).is_empty());
    let tk = names(&["高鐵車票.pdf", "Receipt-2026.pdf"]);
    assert_eq!(members(&rules(&Facts { kind: "pdf", names: &tk, ..Facts::default() })), vec!["ticket"]);
    // Filters: OR within a group, AND across.
    let f = Filter::parse(Some("pdf,image,paper,nope"));
    assert_eq!((f.types.len(), f.uses.len()), (2, 1));
    assert!(f.accepts(&names(&["pdf", "paper"])));
    assert!(!f.accepts(&names(&["pdf"])));
    assert!(!f.accepts(&names(&["slides", "paper"])));
    assert!(Filter::parse(None).accepts(&[]));
    assert_eq!(type_of("text"), Some("document"));
    assert_eq!(label("id-doc"), Some("證件"));
}

#[test]
fn scan_fills_the_extract_cache() {
    let f = fixture();
    write(&f.root.join("card.png"), &{
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(1011u32.to_be_bytes());
        png.extend(638u32.to_be_bytes());
        png
    });
    scan(&f.cfg).unwrap();
    let dir = extract::Extract::dir(&f.cfg.cache_dir);
    let pdf = sha(&tiny_pdf("<< /Title (x) /CreationDate (D:20210304050607+08'00') /ModDate (D:2022) >>"));
    assert_eq!(extract::Extract::load(&dir, &pdf).unwrap().pages, Some(1));
    let card = extract::Extract::load(&dir, &sha(&std::fs::read(f.root.join("card.png")).unwrap())).unwrap();
    assert_eq!((card.width, card.height), (Some(1011), Some(638)));
    assert!(extract::Extract::load(&dir, &sha(b"hello")).is_none(), "text files are not extracted");
}
