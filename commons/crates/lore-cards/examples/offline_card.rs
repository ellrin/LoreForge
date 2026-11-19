//! A card written while no L-Brain runs, into a synthetic space (never the author's KB):
//! `cargo run -p lore-cards --example offline_card -- <empty dir> <title>`. Then
//! `cargo run -p kb-server --example isolated_server -- <same dir> <port>` shows it in L-Brain.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut a = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(a.next().expect("an empty folder"));
    let title = a.next().unwrap_or_else(|| "Momentum queue".into());
    let space = dir.join("personal");
    std::fs::create_dir_all(space.join("nodes")).unwrap();
    std::fs::create_dir_all(space.join("progress/ellrin")).unwrap();
    std::fs::write(space.join("space.toml"), "id = \"me\"\nname = \"我的\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
    let nobody = { std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap() };
    let brain = lore_cards::Brain::at(Some(&format!("http://{nobody}")), vec![space], "ellrin");
    let v = brain.create(serde_json::json!({ "via": "papercomposer", "kind": "concept", "title": title, "summary": "存放舊特徵的佇列", "initial_state": "known", "sources": [{ "type": "document", "ref": "Synthetic draft" }] })).await.expect("create");
    println!("{} via {}", v["node"]["id"].as_str().unwrap_or(""), v["via"]);
}
