//! 期刊寫法 over HTTP on a temp workspace: which journal a template is (confirmed once, written
//! into template.toml), base formats hidden and refused, 新增期刊 on the right base (and the same
//! journal not made twice), 選期刊 for a paper on a base format. OpenAlex is a `MockHttp` (no
//! network); the profile job itself is covered in `venueprof_tests.rs`. Own process.

use std::path::Path;
use std::sync::Arc;

use lore_research::{MockHttp, Response};
use pc_server::book::model::{Ask, BookModel, OnChars};
use pc_server::book::source::Extract;
use serde_json::{Value, json};

struct NoModel;
#[async_trait::async_trait]
impl BookModel for NoModel {
    async fn json(&self, _: Ask<'_>, _: OnChars<'_>) -> Result<(Value, String), lore_jobs::StageError> {
        Err(lore_jobs::StageError::new(503, "no model in tests"))
    }
}
struct NoExtract;
impl Extract for NoExtract {
    fn extract(&self, _: &Path, _: &Path) -> Result<lore_docsrc::AssetCatalog, String> {
        Err("no extraction in tests".into())
    }
}

const SAMPLE: &str = "\\documentclass{article}\n\\begin{document}\n\\title{T}\n\\maketitle\n\\section{Introduction}\nText.\n\\end{document}\n";

fn tpl(ws: &Path, id: &str, toml: &str) {
    let d = ws.join("publishers").join(id);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("template.toml"), toml).unwrap();
    std::fs::write(d.join("sample.tex"), SAMPLE).unwrap();
}

#[tokio::test]
async fn venue_identity_new_journal_and_paper_journal() {
    let root = std::env::temp_dir().join(format!("pc-venue-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws/profile", "ws/papers", "home", "private/shared", "cache"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let ws = root.join("ws");
    tpl(&ws, "demo-base", "name = \"Demo Press article（通用）\"\npublisher = \"Demo Press\"\nkind = \"journal\"\n");
    tpl(&ws, "jdi", "name = \"Journal of Demo Imaging\"\nshort = \"JDI\"\npublisher = \"Demo Press\"\nkind = \"journal\"\nbased_on = \"demo-base\"\n");
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", &ws);
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("PAPERCOMPOSER_CACHE", root.join("cache"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("HOME", root.join("home"));
    }
    let sources = json!({ "results": [
        { "id": "https://openalex.org/S900001", "display_name": "Journal of Demo Imaging", "issn_l": "1234-5678", "issn": ["1234-5678"], "host_organization_name": "Demo Press", "type": "journal", "works_count": 10 },
        { "id": "https://openalex.org/S900002", "display_name": "Demo Widget Letters", "issn_l": "2345-6789", "host_organization_name": "Demo Press BV", "type": "journal", "works_count": 10 },
        { "id": "https://openalex.org/S900003", "display_name": "Unknown House Journal", "host_organization_name": "Unknown House", "type": "journal", "works_count": 10 }
    ]});
    // works: no route = "no network", so the jobs started here stop at once
    let http = Arc::new(MockHttp::new().on("api.openalex.org/sources", Response::ok(sources.to_string())));
    pc_server::venueprof::use_deps(Arc::new(pc_server::venueprof::Deps {
        cache: root.join("cache/venue"),
        http: http.clone(),
        research: lore_research::Config { pace: lore_research::Pace::none(), ..Default::default() },
        model: Arc::new(NoModel),
        extract: Arc::new(NoExtract),
    }));
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let c = reqwest::Client::new();
    let url = |p: &str| format!("http://{addr}{p}");

    // the base is hidden from the picker, the journal is not
    let all: Value = c.get(url("/api/templates")).send().await.unwrap().json().await.unwrap();
    let hidden = |id: &str| all.as_array().unwrap().iter().find(|t| t["id"] == id).unwrap()["hidden"].as_bool().unwrap();
    assert!(hidden("demo-base") && !hidden("jdi"));

    // not confirmed yet: the guess is its name
    let v: Value = c.get(url("/api/venue/t/jdi")).send().await.unwrap().json().await.unwrap();
    assert!(v["identity"].is_null() && v["profile"].is_null());
    assert_eq!(v["guess"], "Journal of Demo Imaging");
    let r = c.post(url("/api/venue/t/jdi/make")).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 422, "a journal not confirmed is not fetched");

    // candidates: only the name went out
    let v: Value = c.get(url("/api/venue/sources?q=Journal%20of%20Demo%20Imaging")).send().await.unwrap().json().await.unwrap();
    let first = v["sources"][0].clone();
    assert_eq!(first["id"], "S900001");
    // confirmed once: written into template.toml, the rest of it kept
    let v: Value = c.put(url("/api/venue/t/jdi/identity")).json(&json!({ "source": first })).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["identity"]["openalex"], "S900001");
    let t = std::fs::read_to_string(ws.join("publishers/jdi/template.toml")).unwrap();
    assert!(t.contains("[venue]") && t.contains("openalex = \"S900001\"") && t.contains("based_on = \"demo-base\""), "{t}");

    // a profile of this journal: the panel shows it and a new paper takes its sections
    let sec = |role: &str, title: &str, subs: &[&str]| pc_core::venueprofile::SectionProfile {
        role: role.into(),
        label: role.into(),
        titles: vec![pc_core::venueprofile::Count { text: title.into(), n: 2 }],
        present: 2,
        subsections: subs.iter().map(|t| pc_core::venueprofile::Count { text: (*t).into(), n: 2 }).collect(),
        ..Default::default()
    };
    let prof = pc_core::venueprofile::Profile {
        template: "jdi".into(),
        venue: "Journal of Demo Imaging".into(),
        openalex: "S900001".into(),
        made: "2026-11-14".into(),
        articles: vec![Default::default(), Default::default()],
        sections: vec![sec("introduction", "Introduction", &[]), sec("methods", "Materials and Methods", &["Study population"]), sec("results", "Results", &[])],
        ..Default::default()
    };
    pc_core::venueprofile::save(&ws, &prof).unwrap();
    let v: Value = c.get(url("/api/venue/t/jdi")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["profile"]["openalex"], "S900001");
    let sk: Value = c.get(url("/api/venue/t/jdi/skeleton")).send().await.unwrap().json().await.unwrap();
    let titles: Vec<&str> = sk["outline"].as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Introduction", "Materials and Methods", "Results"]);
    let r = c.post(url("/api/papers")).json(&json!({ "name": "p0", "template": "jdi", "key_value": "", "outline": sk["outline"] })).send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let made: Vec<String> = pc_core::open::sections(&ws.join("papers/p0")).into_iter().map(|s| s.title).collect();
    assert_eq!(made, ["Introduction", "Materials and Methods", "Results"]);
    let mut all_tex = std::fs::read_to_string(ws.join("papers/p0/main.tex")).unwrap();
    for f in std::fs::read_dir(ws.join("papers/p0/sections")).unwrap().flatten() {
        all_tex.push_str(&std::fs::read_to_string(f.path()).unwrap());
    }
    assert!(all_tex.contains("\\subsection{Study population}") && all_tex.is_ascii(), "{all_tex}");
    // the template is changed to another journal: that profile is not this journal's any more
    let other = json!({ "id": "S900009", "name": "Another Demo Journal", "kind": "journal" });
    let v: Value = c.put(url("/api/venue/t/jdi/identity")).json(&json!({ "source": other })).send().await.unwrap().json().await.unwrap();
    assert!(v["profile"].is_null());
    let sk: Value = c.get(url("/api/venue/t/jdi/skeleton")).send().await.unwrap().json().await.unwrap();
    assert_eq!(sk["outline"], json!([]));
    let sums: Value = c.get(url("/api/venue/summaries")).send().await.unwrap().json().await.unwrap();
    assert!(sums.get("jdi").is_none());
    let v: Value = c.put(url("/api/venue/t/jdi/identity")).json(&json!({ "source": first })).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["profile"]["openalex"], "S900001", "back to the journal it was made for");

    // a base format is not a journal
    let r = c.post(url("/api/venue/t/demo-base/make")).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 422);

    // 新增期刊: built on the publisher's base, its identity set, its profile started
    let v: Value = c.post(url("/api/venue/journals")).json(&json!({ "source": v_source(&sources, 1) })).send().await.unwrap().json().await.unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["base"], "demo-base");
    assert!(v["job"]["job"].is_string());
    let t = std::fs::read_to_string(ws.join("publishers").join(&id).join("template.toml")).unwrap();
    assert!(t.contains("based_on = \"demo-base\"") && t.contains("openalex = \"S900002\"") && t.contains("role = \"journal\""), "{t}");
    // the same journal again: the same template
    let v: Value = c.post(url("/api/venue/journals")).json(&json!({ "source": v_source(&sources, 1) })).send().await.unwrap().json().await.unwrap();
    assert_eq!((v["id"].as_str(), v["existing"].as_bool()), (Some(id.as_str()), Some(true)));
    // no base known for the publisher: the bases to pick from
    let r = c.post(url("/api/venue/journals")).json(&json!({ "source": v_source(&sources, 2) })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 422);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["bases"][0]["id"], "demo-base");

    // 選期刊: a paper on the base moves onto the journal; never onto a base
    let r = c.post(url("/api/papers")).json(&json!({ "name": "p1", "template": "demo-base", "key_value": "" })).send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let r = c.post(url("/api/venue/paper/p1/journal")).json(&json!({ "template": "demo-base" })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 422);
    let r = c.post(url("/api/venue/paper/p1/journal")).json(&json!({ "template": "jdi" })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200, "{}", r.text().await.unwrap());
    assert_eq!(pc_core::meta::read_meta(&ws.join("papers/p1")).journal.as_deref(), Some("jdi"));

    // a journal with no profile yet: no skeleton (the paper starts with the usual empty sections)
    let v: Value = c.get(url(&format!("/api/venue/t/{id}/skeleton"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["outline"], json!([]));

    // roles from the requirements data: an umbrella entry leaves the picker
    tpl(&ws, "demo-series", "name = \"Demo Journals series\"\nkind = \"journal\"\n");
    std::fs::create_dir_all(ws.join("venues")).unwrap();
    std::fs::write(ws.join("venues/requirements.toml"), "[template.demo-series]\nrole = \"umbrella\"\n[template.jdi.submission]\nabstract_words = 200\nchecked = \"2026-11-14\"\n").unwrap();
    let v: Value = c.post(url("/api/venue/requirements/apply")).send().await.unwrap().json().await.unwrap();
    let mut up: Vec<&str> = v["updated"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    up.sort();
    assert_eq!(up, ["demo-series", "jdi"]);
    let all: Value = c.get(url("/api/templates")).send().await.unwrap().json().await.unwrap();
    assert_eq!(all.as_array().unwrap().iter().find(|t| t["id"] == "demo-series").unwrap()["hidden"], true);
    let v: Value = c.get(url("/api/venue/t/jdi")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["submission"]["abstract_words"], 200);

    // nothing private and no e-mail ever went out
    for u in http.urls() {
        assert!(!u.contains("mailto") && !u.contains("p1"), "{u}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

fn v_source(sources: &Value, i: usize) -> Value {
    let s = &sources["results"][i];
    json!({ "id": s["id"], "name": s["display_name"], "issn_l": s["issn_l"], "publisher": s["host_organization_name"], "kind": s["type"] })
}
