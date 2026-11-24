//! Selected text through L-Echo (`lore_shell::selection`): the shared script is served and the
//! route the page posts to is mounted (an empty selection is refused before L-Echo is looked for).
//! Own process (the workspace comes from the environment).

#[tokio::test]
async fn selection_routes_are_mounted() {
    let root = std::env::temp_dir().join(format!("pc-selection-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["ws/profile", "ws/papers", "home", "private/shared"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    // SAFETY: set before the server starts, in a test binary with this one test.
    unsafe {
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", root.join("ws"));
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("LOREFORGE_PRIVATE", root.join("private"));
        std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
        std::env::set_var("PAPERCOMPOSER_NO_ECHO", "1");
        std::env::set_var("HOME", root.join("home"));
    }
    let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), 0).await.unwrap();
    tokio::spawn(serve);
    let http = reqwest::Client::new();
    let url = |p: &str| format!("http://{addr}{p}");

    let r = http.get(url("/selection.js")).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert!(r.text().await.unwrap().contains("window.LoreSelection"));
    let r = http.post(url("/api/lore/selection")).json(&serde_json::json!({ "text": " ", "launch": false })).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let _ = std::fs::remove_dir_all(&root);
}
