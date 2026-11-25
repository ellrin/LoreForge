//! L-Brain with its hub on a copy of the public example data, to look at the hub's screens
//! without the author's folders: no keychain, no `claude` CLI, nothing under the real `private/`.
//! `<dir>/private` must be a copy of the repository's `private-example/`.
//! `cargo run -p kb-server --example hub_demo -- <dir> <port> [inbox]`

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut a = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(a.next().expect("a folder holding private/ (a copy of private-example)"));
    let port: u16 = a.next().and_then(|p| p.parse().ok()).expect("a port");
    let home = lore_home::Home::at(&dir);
    let space = home.app_dir("KnowledgeBase").expect("private/KnowledgeBase");
    std::fs::create_dir_all(space.join("nodes")).unwrap();
    let config = kb_server::Config { spaces: vec![space], member: "ellrin".into(), port, web: kb_server::config::repo_dir().join("web"), intervals: Default::default(), app_dir: Some(dir.join("app")) };
    let kb = kb_server::open(&config, Default::default()).expect("open");
    let rem = kb_server::reminders::Reminders::isolated(&dir.join("app"), Arc::new(lore_store::SystemRunner), "http://127.0.0.1:7700");
    let mut shared = kb_server::AppState::with_hub(kb, config.web.clone(), rem, kb_server::llm::Llm::none(), kb_server::hub::HubState::from_home(&home));
    // a third argument `inbox`: 收件 with three synthetic sentences already worked out (no worker, no model)
    if a.next().as_deref() == Some("inbox") {
        let inbox = kb_server::inbox::real(&home).expect("the demo's inbox");
        if inbox.store.counts().ready == 0 {
            seed(&lore_know::inbox_store::Inbox::open(lore_know::reader::hub_dir(&home).unwrap().join("inbox")).expect("inbox"));
        }
        Arc::get_mut(&mut shared).expect("not shared yet").inbox = Some(inbox);
    }
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    eprintln!("http://{}", l.local_addr().unwrap());
    axum::serve(l, kb_server::router_isolated(shared)).await.unwrap();
}

fn seed(b: &lore_know::inbox_store::Inbox) {
    use lore_know::{Item, Source};
    let rows = [
        ("The network was trained for 200 epochs with a batch size of 16.", "The {model} was trained for {n} epochs with a batch size of {value}.", "Method/訓練與實作/訓練設定", "量化陳述"),
        ("Images were resized to 256 by 256 pixels before augmentation.", "{inputs} were resized to {size} before {step}.", "Result/主要結果/數值報告", "量化陳述"),
        ("Unlike earlier work, the proposed loss does not require paired samples.", "Unlike {prior}, the proposed {component} does not require {requirement}.", "Introduction/缺口/前人不足", "對比"),
    ];
    for (text, template, place, tone) in rows {
        let e = b.add("pc.patterns", text, false, Default::default()).expect("add").remove(0);
        let mut item = Item::new("pc.patterns", lore_know::format::PATTERN, template, Source::lore("demo"));
        item.set("template", template);
        item.set("section", place.split('/').next().unwrap());
        item.set("position", "body");
        item.tags.insert("place".into(), vec![place.to_string()]);
        item.tags.insert("tone".into(), vec![tone.to_string()]);
        b.ready(&e.id, lore_know::inbox::Made { item, similar: vec![] }, "demo").expect("ready");
    }
}
