//! 做成簡報: the page asks this server for the paper's folder, then SlideComposer reads it.
//! Own process (it sets the workspace through the environment); nothing is written.

use serde_json::Value;

#[tokio::test]
async fn todeck_names_the_paper_folder_and_writes_nothing() {
    let root = std::env::temp_dir().join(format!("pc-todeck-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let paper = root.join("ws/papers/demo");
    for d in ["ws/profile", "home", "private"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::create_dir_all(&paper).unwrap();
    std::fs::write(paper.join("main.tex"), "\\documentclass{article}\n\\title{A Synthetic Paper}\n\\begin{document}\n\\section{Intro}\nOne sentence.\n\\end{document}\n").unwrap();
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
    let r = http.get(format!("http://{addr}/api/papers/demo/todeck")).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["path"].as_str().map(std::path::PathBuf::from), Some(paper.canonicalize().unwrap()));
    let r = http.get(format!("http://{addr}/api/papers/nope/todeck")).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 404);
    let names: Vec<String> = std::fs::read_dir(&paper).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(names, vec!["main.tex"]);
    // the page loads the button script and the icon it draws
    let index = std::fs::read_to_string(pc_server::Paths::from_repo().ui.join("index.html")).unwrap();
    assert!(index.contains("<script src=\"todeck.js\"></script>"));
    let icons = http.get(format!("http://{addr}/icons.js")).send().await.unwrap().text().await.unwrap();
    assert!(icons.contains("deckOutline:"));
    let _ = std::fs::remove_dir_all(&root);
}
