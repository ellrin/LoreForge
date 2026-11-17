//! The server on a throwaway copy of everything it reads and writes, for
//! trying the UI without touching real papers or settings:
//! `cargo run -p pc-server --example isolated_server -- <empty folder> [port] [--mock <file>]`.
//! The folder gets a copy of private-example/ (the workspace is its
//! PaperComposer/), a home/ for config and caches, and opened.toml.
//! `ISOLATED_KEEP_HOME=1` keeps the real HOME (the `claude` CLI's sign-in, for a real 寫書 run);
//! books.toml and the job cache still go under the folder.
//!
//! `--mock <file>`: every model call is answered from the file's scripted replies, per task id in
//! order, the last one repeating (`{ "pc-draft-skeleton": [ … ], "pc-draft-fill": [ … ] }`;
//! `crates/server/tests/fixtures/skeleton_mock.json` drives a whole 骨架 run). Nothing reaches the
//! network, the `claude` CLI or a local model. A synthetic paper `skeleton-demo` (an organised
//! record, a figure plan) is added to the workspace to try it on.
//!
//! 寫書 on `crates/server/tests/fixtures/book_mock.json`: the job asks no web source and no running
//! app; the L-Brain is a synthetic one in `<folder>/KnowledgeBase` (member `ellrin`: softmax known,
//! cosine similarity learning), where the cards it makes land. Start a keyword-only book from the
//! start panel (e.g. the keyword `temperature scaling`): plan, review, then two scripted chapters and
//! four 不熟 cards.
fn main() {
    let mut root: Option<std::path::PathBuf> = None;
    let mut port: u16 = 7990;
    let mut mock: Option<std::path::PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--mock" {
            mock = Some(std::path::PathBuf::from(args.next().expect("--mock <file>")));
        } else if root.is_none() {
            root = Some(std::path::PathBuf::from(a));
        } else if let Ok(p) = a.parse() {
            port = p;
        }
    }
    let root = root.expect("a folder");
    std::fs::create_dir_all(root.join("home")).expect("home");
    let mock = mock.map(|m| std::fs::canonicalize(&m).unwrap_or_else(|e| panic!("{}: {e}", m.display())));
    // SAFETY: before any thread starts.
    unsafe {
        std::env::set_var("LOREFORGE_PRIVATE", &root);
        std::env::set_var("PAPERCOMPOSER_WORKSPACE", root.join("PaperComposer"));
        std::env::set_var("PAPERCOMPOSER_OPENED", root.join("home/opened.toml"));
        std::env::set_var("PAPERCOMPOSER_BOOKS", root.join("home/books.toml"));
        std::env::set_var("PAPERCOMPOSER_CACHE", root.join("home/cache"));
        if std::env::var_os("ISOLATED_KEEP_HOME").is_none() {
            std::env::set_var("HOME", root.join("home"));
        }
        if let Some(m) = &mock {
            std::env::set_var("PAPERCOMPOSER_LLM_MOCK", m);
            std::env::set_var("PAPERCOMPOSER_ENGINES_ISOLATED", "1");
            // 寫書: a synthetic L-Brain inside the folder, never the author's
            std::env::set_var("KB_SPACES", root.join("KnowledgeBase"));
            std::env::set_var("KB_HOME", root.join("home/kb"));
            std::env::set_var("KB_MEMBER", "ellrin");
        }
    }
    if mock.is_some() {
        // a workspace named by the environment is not seeded by the server: give it the example's
        // profile, style, lens and templates before the server reads them (writing needs the
        // phrase bank), never overwriting
        let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/PaperComposer");
        copy_missing(&example, &root.join("PaperComposer")).expect("example workspace");
        seed_kb(&root.join("KnowledgeBase")).expect("knowledge base");
    }
    tokio::runtime::Runtime::new().expect("runtime").block_on(async {
        let (addr, serve) = pc_server::bind(pc_server::Paths::from_repo(), port).await.expect("bind");
        if let Some(m) = &mock {
            let paper = pc_server::seed_skeleton_demo(&root.join("PaperComposer")).expect("seed");
            println!("mock replies {}; paper {}", m.display(), paper.display());
        }
        println!("http://{addr}/  workspace {}", root.join("PaperComposer").display());
        serve.await.expect("serve");
    });
}

/// A synthetic L-Brain space for 寫書 (kept when it exists): softmax known, cosine similarity learning.
fn seed_kb(dir: &std::path::Path) -> std::io::Result<()> {
    if dir.join("space.toml").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir.join("nodes"))?;
    std::fs::create_dir_all(dir.join("progress/ellrin"))?;
    std::fs::write(dir.join("space.toml"), "id = \"me\"\nname = \"Me\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n")?;
    for (id, title, state) in [("k1", "softmax", "known"), ("k2", "cosine similarity", "learning")] {
        let node = format!("{{\"id\": \"{id}\", \"kind\": \"concept\", \"title\": \"{title}\", \"summary\": \"A synthetic card.\", \"created_at\": \"2026-11-01T09:00:00+00:00\", \"updated_at\": \"2026-11-01T09:00:00+00:00\"}}");
        std::fs::write(dir.join("nodes").join(format!("{id}.json")), node)?;
        let progress = format!("{{\"state\": \"{state}\", \"streak\": 0, \"next_check_at\": null, \"log\": [{{\"at\": \"2026-11-02T09:00:00+00:00\", \"event\": \"created\"}}]}}");
        std::fs::write(dir.join("progress/ellrin").join(format!("{id}.json")), progress)?;
    }
    Ok(())
}

/// Copy `from` into `to`, keeping every file that is already there.
fn copy_missing(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_missing(&e.path(), &dst)?;
        } else if !dst.exists() {
            std::fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}
