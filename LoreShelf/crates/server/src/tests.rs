//! Offline API tests: a temporary catalog built by lore-catalog from
//! synthetic files, a temporary LoreForge home, a recording runner.

use super::*;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use std::path::Path;
use tower::ServiceExt;

#[derive(Default)]
struct Rec(Mutex<Vec<(String, Vec<String>)>>);

impl Runner for Rec {
    fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
        self.0.lock().unwrap().push((program.into(), args.to_vec()));
        Ok(())
    }

    /// A fake `qlmanage -t -s N -o DIR PATH`: writes `DIR/<name>.png` unless the
    /// file name contains "noql".
    fn run_wait(&self, program: &str, args: &[String], _t: std::time::Duration) -> Result<(), String> {
        self.0.lock().unwrap().push((program.into(), args.to_vec()));
        let (dir, path) = (Path::new(&args[4]), Path::new(&args[5]));
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.contains("noql") {
            std::fs::write(dir.join(format!("{name}.png")), tiny_png(8, 8)).unwrap();
        }
        Ok(())
    }
}

fn tiny_png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 80, 40, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn write(p: &Path, b: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, b).unwrap();
}

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
    app: Router,
    rec: Arc<Rec>,
}

fn fixture() -> Fx {
    let t = tempfile::tempdir().unwrap();
    let base = t.path().canonicalize().unwrap();
    let root = base.join("in");
    write(&root.join("papers/segmentation-review.pdf"), b"%PDF-1.4 a");
    write(&root.join("papers/segmentation-survey.pdf"), b"%PDF-1.4 b");
    write(&root.join("cooking notes.txt"), b"soup");
    write(&root.join("copy/cooking notes.txt"), b"soup");
    write(&root.join("影像分割綜述.pdf"), b"%PDF-1.4 c");
    write(&root.join("Viewer.app/Contents/MacOS/viewer"), b"bin");
    write(&root.join("talk/main.tex"), b"\\documentclass{beamer}");
    write(&root.join("talk/main.pdf"), b"%PDF-1.4 talk");
    write(&root.join("noql.bin"), b"no preview");
    write(&root.join("daidai 護照.jpg"), b"jpeg bytes");
    write(&root.join("HW2.pdf"), b"%PDF-1.4 hw");
    write(&root.join("refs/cxr-study.pdf"), b"%PDF-1.4 study");
    for i in 0..600 {
        let split = if i % 3 == 0 { "test" } else { "train" };
        write(&root.join(format!("PadChest-sample/{split}/{i:04}.png")), &tiny_png(2 + (i % 5) as u32, 3));
    }
    write(&root.join("PadChest-sample/labels.csv"), b"file,label\n");
    let catalog = base.join("catalog");
    let cfg = lore_catalog::ScanConfig {
        roots: vec![root.clone()],
        catalog_dir: catalog.clone(),
        cache_dir: base.join("cache"),
        guard: lore_catalog::Guard { cloud: vec![], forbidden: vec![] },
    };
    lore_catalog::scan(&cfg).unwrap();
    // The first-pages text of one PDF, as the scan would have read it from a real paper.
    let study = lore_catalog::hash::sha256_bytes(b"%PDF-1.4 study");
    let ex = lore_catalog::extract::Extract { pages: Some(9), text: Some("A study. https://doi.org/10.1234/cxr.2024.1 We train on the PadChest dataset.".into()), ..Default::default() };
    ex.save(&lore_catalog::extract::Extract::dir(&base.join("cache")), &study).unwrap();
    let web = base.join("web");
    write(&web.join("index.html"), b"<!doctype html><title>LoreShelf</title>");
    let rec = Arc::new(Rec::default());
    let shared = AppState::new(lore_home::Home::at(base.join("lf")), web, catalog, base.join("cache"), rec.clone());
    Fx { _t: t, root, app: router(shared), rec }
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String, Vec<u8>) {
    let mut b = Request::builder().method(method).uri(uri);
    if body.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/json");
    }
    let req = b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cc = res.headers().get(header::CACHE_CONTROL).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    (status, cc, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

async fn json_of(app: &Router, method: &str, uri: &str) -> Value {
    let (s, _, b) = call(app, method, uri, None).await;
    assert!(s.is_success(), "{uri}: {s} {}", String::from_utf8_lossy(&b));
    serde_json::from_slice(&b).unwrap()
}

fn id_of(list: &Value, name: &str) -> String {
    list.as_array().unwrap().iter().find(|i| i["name"] == name).unwrap_or_else(|| panic!("{name}"))["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn shared_theme_kit_and_web_are_served() {
    let f = fixture();
    for (uri, needle) in [("/theme.css", "giftbox"), ("/theme.js", "LORE_THEME"), ("/base.css", ""), ("/icons.js", ""), ("/shell.js", ""), ("/", "LoreShelf")] {
        let (s, cc, b) = call(&f.app, "GET", uri, None).await;
        assert_eq!(s, StatusCode::OK, "{uri}");
        assert_eq!(cc, "no-cache", "{uri}");
        assert!(String::from_utf8_lossy(&b).contains(needle), "{uri}");
    }
    assert_eq!(json_of(&f.app, "GET", "/api/theme").await["theme"], Value::Null);
    let (s, _, _) = call(&f.app, "PUT", "/api/theme", Some(json!({ "theme": "giftbox" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(json_of(&f.app, "GET", "/api/theme").await["theme"], "giftbox");
    let (_, _, js) = call(&f.app, "GET", "/theme.js", None).await;
    assert!(String::from_utf8_lossy(&js).starts_with("window.LORE_THEME = \"giftbox\""));
    let (s, _, _) = call(&f.app, "PUT", "/api/theme", Some(json!({ "theme": "nope" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let h = json_of(&f.app, "GET", "/api/health").await;
    assert_eq!(h["ok"], true);
    assert_eq!(h["items"], 11, "8 contents + package + project + dataset");
}

#[tokio::test]
async fn files_list_has_names_only_and_detail_has_paths() {
    let f = fixture();
    let (_, _, raw) = call(&f.app, "GET", "/api/files", None).await;
    let text = String::from_utf8_lossy(&raw);
    assert!(!text.contains(&*f.root.to_string_lossy()), "no paths in the list");
    let list: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 11);
    let cooking = list.as_array().unwrap().iter().find(|i| i["name"] == "cooking notes.txt").unwrap();
    assert_eq!(cooking["seen_count"], 2);
    assert_eq!(cooking["kind"], "text");
    assert_eq!(cooking["ext"], "txt");
    assert!(cooking["times"]["modified"].is_string());
    let app = list.as_array().unwrap().iter().find(|i| i["kind"] == "package").unwrap();
    assert_eq!((app["name"].as_str(), app["ext"].as_str()), (Some("Viewer.app"), Some("app")));
    assert!(list.as_array().unwrap().iter().any(|i| i["kind"] == "project" && i["name"] == "talk"));
    assert_eq!(json_of(&f.app, "GET", "/api/files?limit=2").await.as_array().unwrap().len(), 2);

    let d = json_of(&f.app, "GET", &format!("/api/files/{}", cooking["id"].as_str().unwrap())).await;
    assert_eq!(d["paths"].as_array().unwrap().len(), 2);
    assert!(d["description"].as_str().unwrap().contains("2 個位置"), "{d}");
    let (s, _, _) = call(&f.app, "GET", "/api/files/0000", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn neighbors_rank_by_name_time_kind_and_explain_the_parts() {
    let f = fixture();
    let list = json_of(&f.app, "GET", "/api/files").await;
    let review = id_of(&list, "segmentation-review.pdf");
    let survey = id_of(&list, "segmentation-survey.pdf");
    let n = json_of(&f.app, "GET", &format!("/api/neighbors/{review}?k=3&layers=2")).await;
    let n = n.as_array().unwrap();
    assert_eq!(n[0]["id"], survey.as_str(), "shared name token, same kind, same folder");
    assert_eq!(n[0]["layer"], 1);
    assert_eq!(n[0]["via"], review.as_str());
    let p = &n[0]["parts"];
    let sum: f64 = ["name", "time", "kind", "host", "folder"].iter().map(|k| p[k].as_f64().unwrap()).sum();
    assert!((sum - 1.0).abs() < 0.01, "{p}");
    assert!(p["name"].as_f64().unwrap() > 0.0 && p["folder"].as_f64().unwrap() > 0.0);
    assert!(n.iter().filter(|x| x["layer"] == 1).count() <= 3);
    assert!(n.iter().any(|x| x["layer"] == 2), "second layer reached");
    let ids: std::collections::HashSet<&str> = n.iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), n.len(), "no duplicates");
    assert!(!ids.contains(review.as_str()), "never itself");
    let (s, _, _) = call(&f.app, "GET", "/api/neighbors/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn open_and_reveal_use_the_runner_and_never_launch_apps() {
    let f = fixture();
    let list = json_of(&f.app, "GET", "/api/files").await;
    let pdf = id_of(&list, "影像分割綜述.pdf");
    let app = id_of(&list, "Viewer.app");
    assert_eq!(json_of(&f.app, "POST", &format!("/api/open/{pdf}")).await["opened"], true);
    assert_eq!(json_of(&f.app, "POST", &format!("/api/reveal/{pdf}")).await["revealed"], true);
    assert_eq!(json_of(&f.app, "POST", &format!("/api/open/{app}")).await["revealed"], true);
    let calls = f.rec.0.lock().unwrap().clone();
    let pdf_path = f.root.join("影像分割綜述.pdf").to_string_lossy().to_string();
    if cfg!(target_os = "macos") {
        assert_eq!(calls[0], ("open".into(), vec![pdf_path.clone()]));
        assert_eq!(calls[1], ("open".into(), vec!["-R".into(), pdf_path]));
        assert_eq!(calls[2].1[0], "-R", "an .app is revealed, not launched");
    }
    let (s, _, _) = call(&f.app, "POST", "/api/open/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // A file that moved away: nothing is run.
    std::fs::remove_file(f.root.join("影像分割綜述.pdf")).unwrap();
    let (s, _, _) = call(&f.app, "POST", &format!("/api/open/{pdf}"), None).await;
    assert_eq!(s, StatusCode::GONE);
    assert_eq!(f.rec.0.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn empty_or_missing_catalog_is_an_empty_list() {
    let t = tempfile::tempdir().unwrap();
    let shared = AppState::new(lore_home::Home::at(t.path()), t.path().join("web"), t.path().join("none"), t.path().join("cache"), Arc::new(SystemRunner));
    let app = router(shared);
    assert_eq!(json_of(&app, "GET", "/api/files").await, json!([]));
}


#[tokio::test]
async fn thumbnails_via_quicklook_cached_and_never_for_apps() {
    let f = fixture();
    let list = json_of(&f.app, "GET", "/api/files").await;
    let pdf = id_of(&list, "影像分割綜述.pdf");
    let (s, _, b) = call(&f.app, "GET", &format!("/api/thumb/{pdf}?s=256"), None).await;
    if !cfg!(target_os = "macos") {
        assert_eq!(s, StatusCode::NOT_FOUND);
        return;
    }
    assert_eq!(s, StatusCode::OK);
    assert!(b.starts_with(b"\x89PNG"));
    let calls = f.rec.0.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "qlmanage");
    assert_eq!(&calls[0].1[..3], &["-t".to_string(), "-s".into(), "256".into()]);
    // Cached: QuickLook is not asked again.
    let (s, _, _) = call(&f.app, "GET", &format!("/api/thumb/{pdf}?s=256"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(f.rec.0.lock().unwrap().len(), 1);
    // App bundles and unknown ids: 404 without running anything.
    for id in [id_of(&list, "Viewer.app"), "nope".into()] {
        let (s, _, _) = call(&f.app, "GET", &format!("/api/thumb/{id}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }
    assert_eq!(f.rec.0.lock().unwrap().len(), 1);
    // A project shows its main.pdf.
    let talk = id_of(&list, "talk");
    let (s, _, _) = call(&f.app, "GET", &format!("/api/thumb/{talk}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(f.rec.0.lock().unwrap()[1].1[5].ends_with("talk/main.pdf"));
    // QuickLook gives nothing: 404, remembered.
    let noql = id_of(&list, "noql.bin");
    for _ in 0..2 {
        let (s, _, _) = call(&f.app, "GET", &format!("/api/thumb/{noql}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }
    assert_eq!(f.rec.0.lock().unwrap().len(), 3);
}

#[test]
fn thumb_cache_drops_least_recently_used() {
    let t = tempfile::tempdir().unwrap();
    for (i, name) in ["a-512.png", "b-512.png", "c-512.png"].iter().enumerate() {
        let p = t.path().join(name);
        std::fs::write(&p, vec![0u8; 100]).unwrap();
        let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + i as u64);
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(when).unwrap();
    }
    thumbs::prune(t.path(), 250);
    assert!(!t.path().join("a-512.png").exists());
    assert!(t.path().join("b-512.png").exists() && t.path().join("c-512.png").exists());
}

#[test]
fn copy_names_share_a_group() {
    use index::{base_stem, group_key};
    assert_eq!(base_stem("Report (1).pdf"), "report");
    assert_eq!(base_stem("report copy 2.docx"), "report");
    assert_eq!(base_stem("Report - Copy.pptx"), "report");
    assert_eq!(base_stem("2024 (draft).pdf"), "2024 (draft)");
    let stems: std::collections::HashSet<String> = ["report", "lecture-3", "lecture-4"].iter().map(|s| s.to_string()).collect();
    assert_eq!(group_key("report-2.pdf", &stems).as_deref(), Some("report"));
    assert_eq!(group_key("lecture-3.pdf", &stems).as_deref(), Some("lecture-3"), "no plain 'lecture': stays apart");
    assert_eq!(group_key("", &stems), None);
    for g in ["main", "Fig1", "fig_3", "Figure 2", "IMG0042", "Screenshot 2026-01-02 at 10.00.00", "截圖 2026", "未命名 3", "untitled"] {
        assert!(index::generic_stem(&index::base_stem(&format!("{g}.pdf"))), "{g}");
    }
    assert!(!index::generic_stem("figure-skating-notes") && !index::generic_stem("mainland report"));
}

#[tokio::test]
async fn groups_in_files_and_one_neighbor_per_group() {
    let f = fixture();
    write(&f.root.join("papers/segmentation-review (1).pdf"), b"%PDF-1.4 a2");
    write(&f.root.join("papers/Segmentation-Review.docx"), b"docx bytes");
    lore_catalog::scan(&lore_catalog::ScanConfig {
        roots: vec![f.root.clone()],
        catalog_dir: f.root.parent().unwrap().join("catalog"),
        cache_dir: f.root.parent().unwrap().join("cache"),
        guard: lore_catalog::Guard { cloud: vec![], forbidden: vec![] },
    })
    .unwrap();
    let list = json_of(&f.app, "GET", "/api/files").await;
    let g = |n: &str| list.as_array().unwrap().iter().find(|i| i["name"] == n).unwrap().clone();
    let (a, b, c) = (g("segmentation-review.pdf"), g("segmentation-review (1).pdf"), g("Segmentation-Review.docx"));
    assert_eq!(a["group"], b["group"]);
    assert_eq!(a["group"], c["group"], "same stem, other format");
    assert_eq!(a["group_size"], 3);
    assert_ne!(a["group"], g("segmentation-survey.pdf")["group"]);
    assert_eq!(g("talk")["group_size"], 1);

    let survey = g("segmentation-survey.pdf")["id"].as_str().unwrap().to_string();
    let n = json_of(&f.app, "GET", &format!("/api/neighbors/{survey}?k=5&layers=1")).await;
    let n = n.as_array().unwrap();
    let hits: Vec<&Value> = n.iter().filter(|x| x["group"] == a["group"]).collect();
    assert_eq!(hits.len(), 1, "one entry per group");
    assert_eq!(hits[0]["members"].as_array().unwrap().len(), 3);
    // The focus's own copies are not neighbours.
    let r = json_of(&f.app, "GET", &format!("/api/neighbors/{}?k=10&layers=2", a["id"].as_str().unwrap())).await;
    assert!(r.as_array().unwrap().iter().all(|x| x["group"] != a["group"]));
}

#[test]
fn timeline_buckets() {
    use chrono::TimeZone;
    let tz = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    // Thursday 2026-11-12 10:00 +08:00.
    let now = tz.with_ymd_and_hms(2026, 11, 12, 10, 0, 0).unwrap();
    let item = |id: &str, downloaded: Option<&str>, doc: Option<&str>, created: &str| index::Entry::synthetic(id, downloaded, doc, created);
    let idx = index::Index::from_entries(vec![
        item("a", Some("2026-11-10T03:00:00Z"), None, "2020-01-01T00:00:00Z"), // 本週 (downloaded wins)
        item("b", None, Some("2026-11-03T03:00:00Z"), "2026-11-11T00:00:00Z"), // 上週 (doc date over created)
        item("c", None, None, "2026-11-01T03:00:00Z"),                          // 本月
        item("d", None, None, "2026-10-15T03:00:00Z"),                          // 上個月
        item("e", None, None, "2026-09-20T03:00:00Z"),                          // 2026 秋
        item("f", None, None, "2026-01-20T03:00:00Z"),                          // 2025 冬
        item("g", None, None, "2026-01-25T03:00:00Z"),                          // 2025 冬, newer
        item("h", None, None, "2019-06-01T03:00:00Z"),                          // 2019
    ]);
    let b = timeline::buckets(&idx, now, &lore_catalog::category::Filter::default());
    let labels: Vec<&str> = b.iter().map(|x| x.label.as_str()).collect();
    assert_eq!(labels, vec!["本週", "上週", "本月", "上個月", "2026 秋", "2025 冬", "2019"]);
    assert_eq!(b[0].latest_id, "a");
    assert_eq!(b[0].from, "2026-11-09T00:00:00+08:00");
    assert_eq!(b[5].count, 2);
    assert_eq!(b[5].latest_id, "g");
    assert_eq!(b[4].to, "2026-10-01T00:00:00+08:00", "clipped at last month");
}


fn names_of(v: &Value) -> Vec<String> {
    let mut n: Vec<String> = v.as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect();
    n.sort();
    n
}

#[tokio::test]
async fn categories_filters_and_the_authors_choice() {
    let f = fixture();
    let cats = json_of(&f.app, "GET", "/api/categories").await;
    let count = |id: &str| cats.as_array().unwrap().iter().find(|c| c["id"] == id).unwrap()["count"].as_u64().unwrap();
    assert_eq!(cats.as_array().unwrap().iter().find(|c| c["id"] == "id-doc").unwrap()["group"], "use");
    assert_eq!(count("paper"), 2, "the DOI PDF and the LaTeX project");
    assert_eq!((count("id-doc"), count("homework"), count("dataset"), count("package")), (1, 1, 1, 1));

    assert_eq!(names_of(&json_of(&f.app, "GET", "/api/files?cat=paper").await), vec!["cxr-study.pdf", "talk"]);
    assert_eq!(names_of(&json_of(&f.app, "GET", "/api/files?cat=pdf,paper").await), vec!["cxr-study.pdf"], "AND across groups");
    assert_eq!(names_of(&json_of(&f.app, "GET", "/api/files?cat=homework,id-doc").await), vec!["HW2.pdf", "daidai 護照.jpg"], "OR within a group");
    let id_doc = json_of(&f.app, "GET", "/api/files?cat=id-doc").await;
    assert_eq!(id_doc[0]["sensitive"], true);
    assert_eq!(id_doc[0]["cloud_ok"], false);
    let all = json_of(&f.app, "GET", "/api/files").await;
    let ok = |n: &str| all.as_array().unwrap().iter().find(|i| i["name"] == n).unwrap()["cloud_ok"].as_bool().unwrap();
    assert!(ok("cxr-study.pdf"), "a paper may go to a cloud model");
    assert!(!ok("HW2.pdf"), "homework is not academic: local only");
    assert!(json_of(&f.app, "GET", "/api/files").await.as_array().unwrap().iter().filter(|i| i["sensitive"] == true).count() == 1);

    // The author overrides the rules (a local git commit in the catalog).
    let list = json_of(&f.app, "GET", "/api/files").await;
    let hw = id_of(&list, "HW2.pdf");
    let (s, _, b) = call(&f.app, "PUT", &format!("/api/files/{hw}/categories"), Some(json!({ "categories": ["paper"] }))).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    let r: Value = serde_json::from_slice(&b).unwrap();
    assert!(r["commit"].is_string());
    assert_eq!(names_of(&json_of(&f.app, "GET", "/api/files?cat=homework").await), Vec::<String>::new());
    assert!(names_of(&json_of(&f.app, "GET", "/api/files?cat=paper").await).contains(&"HW2.pdf".to_string()));
    let d = json_of(&f.app, "GET", &format!("/api/files/{hw}")).await;
    assert_eq!(d["category_hits"][0]["source"], "author");
    let (s, _, _) = call(&f.app, "PUT", &format!("/api/files/{hw}/categories"), Some(json!({ "categories": ["pdf"] }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "type categories are not editable");

    // A category the author makes up: `u:<name>`, listed once an item carries it.
    let (s, _, _) = call(&f.app, "PUT", &format!("/api/files/{hw}/categories"), Some(json!({ "categories": ["paper", "u:實驗紀錄"] }))).await;
    assert_eq!(s, StatusCode::OK);
    let cats = json_of(&f.app, "GET", "/api/categories").await;
    let mine = cats.as_array().unwrap().iter().find(|c| c["id"] == "u:實驗紀錄").expect("the custom category is listed");
    assert_eq!((mine["label"].as_str(), mine["group"].as_str(), mine["count"].as_u64()), (Some("實驗紀錄"), Some("use"), Some(1)));
    assert_eq!(names_of(&json_of(&f.app, "GET", "/api/files?cat=u:實驗紀錄").await), vec!["HW2.pdf"]);
    for bad in ["u:", "u:a,b", "u: x", "mine"] {
        let (s, _, _) = call(&f.app, "PUT", &format!("/api/files/{hw}/categories"), Some(json!({ "categories": [bad] }))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (s, _, _) = call(&f.app, "PUT", &format!("/api/files/{hw}/categories"), Some(json!({ "categories": ["paper"] }))).await;
    assert_eq!(s, StatusCode::OK);

    // Filtered neighbours and the nearest match for a focus outside the filter.
    let pass = id_of(&list, "daidai 護照.jpg");
    let n = json_of(&f.app, "GET", &format!("/api/neighbors/{pass}?cat=pdf&k=20&layers=1")).await;
    assert!(n.as_array().unwrap().iter().all(|x| list.as_array().unwrap().iter().any(|i| i["id"] == x["id"] && i["categories"].as_array().unwrap().contains(&json!("pdf")))));
    let near = json_of(&f.app, "GET", &format!("/api/nearest?from={pass}&cat=paper")).await;
    assert!(near["item"]["categories"].as_array().unwrap().contains(&json!("paper")));
    let tl = json_of(&f.app, "GET", "/api/timeline?cat=id-doc").await;
    assert_eq!(tl.as_array().unwrap().iter().map(|b| b["count"].as_u64().unwrap()).sum::<u64>(), 1);
}

#[tokio::test]
async fn guided_search_finds_by_text_category_and_date_and_says_how_to_narrow() {
    let f = fixture();
    // a word that is in no file name: found in the paper's first pages
    let r = json_of(&f.app, "GET", "/api/search?q=doi").await;
    let names: Vec<&str> = r["items"].as_array().unwrap().iter().map(|x| x["item"]["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"cxr-study.pdf"), "{names:?}");
    let hit = r["items"].as_array().unwrap().iter().find(|x| x["item"]["name"] == "cxr-study.pdf").unwrap();
    assert!(hit["via"].as_array().unwrap().contains(&json!("text")));
    assert!(hit["snippet"].as_str().unwrap().to_lowercase().contains("doi"));
    // a category label is a word too; every term must match
    let r = json_of(&f.app, "GET", "/api/search?q=%E4%BD%9C%E6%A5%AD").await;   // 作業
    assert_eq!(r["items"].as_array().unwrap().iter().map(|x| x["item"]["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["HW2.pdf"]);
    let r = json_of(&f.app, "GET", "/api/search?q=hw2%20nothing-like-this").await;
    assert_eq!(r["total"], 0);
    // a name match ranks above a text match; cat= narrows
    let r = json_of(&f.app, "GET", "/api/search?q=cxr").await;
    assert_eq!(r["items"][0]["item"]["name"], "cxr-study.pdf");
    let r = json_of(&f.app, "GET", "/api/search?q=&cat=paper").await;
    assert_eq!(r["total"], 2);
    assert_eq!(r["items"].as_array().unwrap().len(), 2, "only a category: what is in it");
    // with nothing typed: what the catalog holds, as ways in
    let r = json_of(&f.app, "GET", "/api/search").await;
    assert!(r["items"].as_array().unwrap().is_empty(), "nothing asked: only the ways in");
    let facets = r["facets"].as_array().unwrap();
    assert!(facets.iter().any(|x| x["kind"] == "cat" && x["value"] == "paper" && x["count"] == 2), "{facets:?}");
    assert!(facets.iter().any(|x| x["kind"] == "term" && x["label"].as_str().unwrap().ends_with("年")));
    // the sensitive item never shows its text
    assert!(json_of(&f.app, "GET", "/api/search?q=daidai").await["items"].as_array().unwrap().iter().all(|x| x["snippet"].is_null()));
}

#[tokio::test]
async fn a_spoken_sentence_finds_the_file_and_another_app_can_turn_the_page_to_it() {
    let f = fixture();
    let toks = crate::search::sentence_tokens("我想找 CXR 的 study，生物力學");
    for t in ["cxr", "study", "生物", "物力", "力學"] {
        assert!(toks.contains(&t.to_string()), "{t} in {toks:?}");
    }
    assert!(!toks.contains(&"的".to_string()));
    // the sentence is not a query: only the file it is about comes back
    let r = json_of(&f.app, "GET", "/api/search?loose=1&q=%E5%B9%AB%E6%88%91%E6%89%BE%E9%82%A3%E4%BB%BD%20cxr%20study").await;   // 幫我找那份 cxr study
    assert_eq!(r["items"][0]["item"]["name"], "cxr-study.pdf");
    let chat = json_of(&f.app, "GET", "/api/search?loose=1&q=%E4%BD%A0%E5%A5%BD%E5%97%8E").await;   // 你好嗎
    assert_eq!(chat["total"], 0);
    // another local app asks the page to turn to it; the page hears it on its long poll
    let id = r["items"][0]["item"]["id"].as_str().unwrap().to_string();
    let seq = json_of(&f.app, "GET", "/api/events").await["seq"].as_u64().unwrap();
    let (s, _, _) = call(&f.app, "POST", "/api/act", Some(json!({ "focus": id }))).await;
    assert_eq!(s, StatusCode::OK);
    let ev = json_of(&f.app, "GET", &format!("/api/events?after={seq}")).await;
    assert_eq!((ev["seq"].as_u64(), ev["act"]["focus"].as_str()), (Some(seq + 1), Some(id.as_str())));
    let (s, _, _) = call(&f.app, "POST", "/api/act", Some(json!({ "focus": "nope" }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // LoreEcho's seat: the bubble's place under the corner, the narrowest window that shows the character
    assert_eq!(crate::echo_place(1500.0, 900.0, None), json!({ "x": 1436.0, "y": 836.0, "w": 540.0, "h": 560.0, "expanded": true }));
}

#[tokio::test]
async fn echo_is_opened_in_the_background() {
    let f = fixture();
    let (s, _, _) = call(&f.app, "POST", "/api/echo/open", None).await;
    if cfg!(target_os = "macos") {
        assert_eq!(s, StatusCode::OK);
        assert_eq!(f.rec.0.lock().unwrap().last().unwrap().1, vec!["-g", "-a", "LoreEcho"]);
    }
}

#[tokio::test]
async fn datasets_summary_pages_mosaic_and_paper_links() {
    let f = fixture();
    let list = json_of(&f.app, "GET", "/api/files").await;
    let ds = id_of(&list, "PadChest-sample");
    let paper = id_of(&list, "cxr-study.pdf");
    let d = json_of(&f.app, "GET", &format!("/api/files/{ds}")).await;
    let sum = &d["dataset"];
    assert_eq!(sum["files"], 601);
    assert_eq!(sum["main_ext"], "png");
    assert_eq!(sum["meta_files"], json!(["labels.csv"]));
    assert_eq!(sum["subfolders"], json!([["test", 200], ["train", 400]]));
    assert_eq!(sum["image_sizes"].as_array().unwrap().len(), 5);
    assert_eq!(sum["image_sizes"][0][1], 3);
    assert_eq!(d["mentions"], json!([paper]));

    let page = json_of(&f.app, "GET", &format!("/api/datasets/{ds}/items?offset=590&limit=60")).await;
    assert_eq!(page["total"], 601);
    assert_eq!(page["items"].as_array().unwrap().len(), 11);
    assert_eq!(page["items"][0]["index"], 590);
    assert!(!page.to_string().contains(&*f.root.to_string_lossy()), "relative paths only");

    // Dataset ↔ paper neighbours, through the mention.
    let n = json_of(&f.app, "GET", &format!("/api/neighbors/{ds}?k=3&layers=1")).await;
    assert_eq!(n[0]["id"], paper.as_str());
    assert!(n[0]["parts"]["mention"].as_f64().unwrap() > 0.5);
    let back = json_of(&f.app, "GET", &format!("/api/neighbors/{paper}?k=3&layers=1")).await;
    assert!(back.as_array().unwrap().iter().any(|x| x["id"] == ds.as_str()));

    if !cfg!(target_os = "macos") {
        return;
    }
    let (s, _, b) = call(&f.app, "GET", &format!("/api/thumb/ds/{ds}/3?s=128"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(b.starts_with(b"\x89PNG"));
    let (s, _, b) = call(&f.app, "GET", &format!("/api/thumb/{ds}?s=200"), None).await;
    assert_eq!(s, StatusCode::OK);
    let img = image::load_from_memory(&b).unwrap();
    assert_eq!((img.width(), img.height()), (200, 200));
    let ql = f.rec.0.lock().unwrap().iter().filter(|c| c.0 == "qlmanage").count();
    assert_eq!(ql, 5, "one member + four mosaic tiles");
    let (s, _, _) = call(&f.app, "GET", &format!("/api/thumb/{ds}?s=200"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(f.rec.0.lock().unwrap().len(), 5, "mosaic cached");
    let (s, _, _) = call(&f.app, "GET", &format!("/api/thumb/ds/{ds}/9999"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}


#[tokio::test]
async fn only_documents_group_by_name_and_generic_names_never() {
    let f = fixture();
    write(&f.root.join("a/main.png"), b"png one");
    write(&f.root.join("b/main.png"), b"png two");
    write(&f.root.join("a/main.py"), b"print()");
    write(&f.root.join("a/main.pdf"), b"%PDF-1.4 main a");
    write(&f.root.join("b/main.pdf"), b"%PDF-1.4 main b");
    write(&f.root.join("notes.txt"), b"notes text");
    write(&f.root.join("notes.pdf"), b"%PDF-1.4 notes");
    lore_catalog::scan(&lore_catalog::ScanConfig {
        roots: vec![f.root.clone()],
        catalog_dir: f.root.parent().unwrap().join("catalog"),
        cache_dir: f.root.parent().unwrap().join("cache"),
        guard: lore_catalog::Guard { cloud: vec![], forbidden: vec![] },
    })
    .unwrap();
    let list = json_of(&f.app, "GET", "/api/files").await;
    let mains: Vec<&Value> = list.as_array().unwrap().iter().filter(|i| i["name"].as_str().unwrap().starts_with("main.")).collect();
    assert_eq!(mains.len(), 5);
    assert!(mains.iter().all(|i| i["group_size"] == 1), "generic stem: never grouped by name");
    let notes: Vec<&Value> = list.as_array().unwrap().iter().filter(|i| i["name"].as_str().unwrap().starts_with("notes.")).collect();
    assert_ne!(notes[0]["group"], notes[1]["group"], "a .txt is not in the document family");
    let g = |n: &str| list.as_array().unwrap().iter().find(|i| i["name"] == n).unwrap()["group_size"].as_u64().unwrap();
    assert_eq!(g("cooking notes.txt"), 1, "same content at two paths is one item, not a name group");
}


#[test]
fn default_focus_prefers_opened_documents_and_never_photos() {
    use lore_catalog::category::Filter;
    let e = |id: &str, kind: &str, ext: &str, opened: Option<&str>, created: &str| {
        let mut x = index::Entry::synthetic(id, None, None, created);
        x.item.kind = kind.into();
        x.item.ext = ext.into();
        x.item.categories = lore_catalog::category::type_of(kind).map(|t| vec![t.to_string()]).unwrap_or_default();
        x.item.times.opened = opened.map(Into::into);
        x
    };
    let photo = e("photo", "image", "jpg", Some("2026-11-12T09:00:00Z"), "2026-11-12T09:00:00Z");
    let deck = e("deck", "slides", "pptx", Some("2026-11-10T09:00:00Z"), "2026-01-01T00:00:00Z");
    let old_pdf = e("old", "pdf", "pdf", Some("2026-11-01T09:00:00Z"), "2025-01-01T00:00:00Z");
    let new_pdf = e("new", "pdf", "pdf", None, "2026-11-11T00:00:00Z");
    let mut secret = e("secret", "pdf", "pdf", Some("2026-11-12T10:00:00Z"), "2026-11-12T10:00:00Z");
    secret.item.sensitive = true;
    let idx = index::Index::from_entries(vec![photo.clone(), deck.clone(), old_pdf.clone(), new_pdf.clone(), secret.clone()]);
    assert_eq!(idx.default_focus(&Filter::default()).unwrap().id, "deck", "last opened document; the photo and the sensitive one are skipped");
    assert_eq!(idx.default_focus(&Filter::parse(Some("pdf"))).unwrap().id, "old");
    let none_opened = index::Index::from_entries(vec![photo, e("n2", "pdf", "pdf", None, "2026-10-01T00:00:00Z"), new_pdf]);
    assert_eq!(none_opened.default_focus(&Filter::default()).unwrap().id, "new", "else the newest PDF");
    assert!(index::Index::from_entries(vec![]).default_focus(&Filter::default()).is_none());
}
