//! 筆記 over HTTP on a temp workspace: create, list, read, edit, the lore-notation block (422,
//! nothing written), delete to `.trash/`, and a note LoreEcho wrote straight into the folder
//! (lore-notes) showing up without a restart. Own process (the workspace comes from the environment).

use serde_json::{json, Value};

#[tokio::test]
async fn notes_api_round_trip_block_and_trash() {
    let root = std::env::temp_dir().join(format!("pc-notes-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws/profile", "home", "private/shared"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(
        root.join("private/shared/notation.json"),
        r#"{"version":1,"entries":[{"id":"w","kind":"symbol","latex":"\\boldsymbol{w}","macro":"vw","meaning":"weight vector"},{"id":"loss","kind":"equation","latex":"L","label":"eq:loss"}]}"#,
    )
    .unwrap();
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", root.join("ws"));
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("HOME", root.join("home"));
    }
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let http = reqwest::Client::new();
    let url = |p: &str| format!("http://{addr}{p}");
    let dir = root.join("ws/notes");

    let v: Value = http.get(url("/api/notes")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["notes"], json!([]));
    assert!(!dir.exists(), "nothing is created by reading");

    // 設定 → 一般 → 權限: the voice's three (潤飾); isolated = no voice, nothing asked or opened
    let v: Value = http.get(url("/api/permissions")).send().await.unwrap().json().await.unwrap();
    let ids: Vec<&str> = v["permissions"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["microphone", "speech", "personal_voice"]);
    assert!(v["permissions"].as_array().unwrap().iter().all(|r| r["status"] == "unsupported"));
    let v: Value = http.post(url("/api/permissions/open")).json(&json!({ "what": "microphone" })).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["opened"], false);

    // blocked: a typed reference number; 422 with the findings, no file
    let r = http.post(url("/api/notes")).json(&json!({ "title": "Typed", "body": "As Table 1 shows." })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 422);
    let v: Value = r.json().await.unwrap();
    assert!(v["findings"].as_array().unwrap().iter().any(|f| f["code"] == "hardcoded_reference" && f["blocking"] == true), "{v}");
    assert!(!dir.exists());

    // created with the registry's macro and label
    let r = http
        .post(url("/api/notes"))
        .json(&json!({ "title": "Loss weights", "body": "By \\eqref{eq:loss}, \\(\\vw\\) shrinks.", "tags": ["loss"], "source": "loreecho" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.json().await.unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    assert!(id.ends_with("-loss-weights"), "{id}");
    assert!(v["findings"].as_array().unwrap().iter().all(|f| f["blocking"] == false));
    let text = std::fs::read_to_string(dir.join(format!("{id}.tex"))).unwrap();
    assert!(text.starts_with("% ---\n% source: loreecho\n"), "{text}");

    // LoreEcho wrote one while PaperComposer was running: listed on the next poll
    let echo = pc_core::notes::Note {
        id: "2030-01-01-quick-idea".into(),
        title: "Quick idea".into(),
        body: "Try $\\vw = 0$.".into(),
        kind: pc_core::notes::Kind::Idea,
        source: "loreecho".into(),
        created: "2030-01-01T00:00:00+00:00".into(),
        ..Default::default()
    };
    std::fs::write(dir.join(format!("{}.tex", echo.id)), pc_core::notes::to_tex(&echo)).unwrap();
    let v: Value = http.get(url("/api/notes")).send().await.unwrap().json().await.unwrap();
    let ids: Vec<&str> = v["notes"].as_array().unwrap().iter().map(|n| n["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [echo.id.as_str(), id.as_str()], "newest first");
    assert_eq!(v["notes"][0]["kind"], "idea");
    let v: Value = http.get(url("/api/notes?q=shrinks")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["notes"].as_array().unwrap().len(), 1);

    // read, edit (blocked edit leaves the file), edit
    let v: Value = http.get(url(&format!("/api/notes/{id}"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["title"], "Loss weights");
    let r = http.put(url(&format!("/api/notes/{id}"))).json(&json!({ "body": "Fig. 2 shows {it." })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 422);
    assert_eq!(std::fs::read_to_string(dir.join(format!("{id}.tex"))).unwrap(), text);
    let r = http.put(url(&format!("/api/notes/{id}"))).json(&json!({ "body": "Now \\ref{eq:loss} only." })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = http.get(url(&format!("/api/notes/{id}"))).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["body"], "Now \\ref{eq:loss} only.");
    assert!(v["updated"].is_string());

    // macros for the preview
    let v: Value = http.get(url("/api/notes/macros")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["macros"]["\\vw"], "\\boldsymbol{w}");

    // bad ids, missing notes, delete to the trash
    assert_eq!(http.get(url("/api/notes/..%2Fx")).send().await.unwrap().status().as_u16(), 400);
    assert_eq!(http.get(url("/api/notes/2020-01-01-none")).send().await.unwrap().status().as_u16(), 404);
    let v: Value = http.delete(url(&format!("/api/notes/{id}"))).send().await.unwrap().json().await.unwrap();
    assert!(v["trash"].as_str().unwrap().ends_with(&format!(".trash/{id}.tex")));
    assert!(dir.join(".trash").join(format!("{id}.tex")).is_file());
    let v: Value = http.get(url("/api/notes")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["notes"].as_array().unwrap().len(), 1);

    // the page loads the panel and has the button
    let ui = pc_server::Paths::from_repo().ui;
    assert!(std::fs::read_to_string(ui.join("index.html")).unwrap().contains("<script src=\"notes.js\"></script>"));
    assert!(std::fs::read_to_string(ui.join("frame.js")).unwrap().contains("PCNotes.open()"));
    let _ = std::fs::remove_dir_all(&root);
}
