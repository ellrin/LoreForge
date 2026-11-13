//! `loreshelf` — LoreShelf wave 1 command line.
//!
//! ```text
//! loreshelf-cli scan <dir>...   read-only scan into the local catalog; prints a summary and the report path
//! loreshelf-cli report          path of the latest report
//! loreshelf-cli extract         read the whole text of catalogued PDFs, slides, Word and text files
//!                           into the cache (only what is not read yet; slow, run it in the background)
//! loreshelf-cli terms           build the term pool (words the KnowledgeBase lacks) from that text
//! loreshelf-cli mastered <dir>...  folders whose files the author knows all of (kept in the
//!                           catalog, local only); their terms become known cards
//! loreshelf-cli known-cards [--write]  the terms of those folders the model calls terms → known
//!                           KnowledgeBase cards (without --write: only how many)
//! loreshelf-cli card-titles   Claude Haiku checks those terms once and gives each card its usual
//!                           title (only terms and sentences from papers and slide decks are sent)
//! loreshelf-cli classify [N]    the local model sorts the pool (term / general / name / junk), N terms
//!                           at most; saved per batch, goes on where it stopped; then adds the general
//!                           words and names to the shared ignore list (`ignore-words` below)
//! loreshelf-cli ignore-words [--write]  general words and names → `private/LoreReader/ignore-words.txt`
//!                           (the 「這不是術語」 list every paper reader uses; lines tagged `# LoreShelf`)
//! ```
//!
//! Catalog: `$LORESHELF_HOME`, else `~/Library/Application Support/LoreForge/LoreShelf`;
//! hash cache: `<cache>/LoreForge/shelf/` (`$LOREFORGE_CACHE` overrides the base).

use lore_catalog::report::human;
use lore_catalog::{Catalog, Guard, ScanConfig, store};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "用法：\n  loreshelf-cli scan <資料夾>...   只讀掃描，寫入本機目錄與報告\n  loreshelf-cli report            最新報告的位置\n  loreshelf-cli extract           讀取檔案全文到快取（只補還沒讀的）\n  loreshelf-cli terms             整理候選詞（知識庫還沒有的詞）\n  loreshelf-cli mastered <資料夾>...  這些資料夾的檔案我全懂（裡面的術語會做成已懂的卡）\n  loreshelf-cli known-cards [--write]  把上面那些資料夾的術語做成已懂的知識卡（不加 --write 只算數量）\n  loreshelf-cli card-titles       用 Claude Haiku 確認全懂資料夾的術語並給卡片標題（只送論文與簡報裡的詞和句子）\n  loreshelf-cli classify [數量]   本機模型分類候選詞（每批存檔，可中斷續跑；跑完把一般詞寫進忽略清單）\n  loreshelf-cli ignore-words [--write]  一般詞與名稱寫進共用的「這不是術語」清單（不加 --write 只算數量）";

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
        Some("extract") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let t0 = std::time::Instant::now();
            let mut last = 0;
            let f = lore_catalog::fulltext::fill(&catalog_dir, &cache_dir, |done, total| {
                if done == total || done - last >= 25 {
                    last = done;
                    eprintln!("{done} / {total}");
                }
            })
            .map_err(|e| e.to_string())?;
            Ok(format!("讀了 {} 個（有文字 {}），之前已讀 {}，找不到檔案 {}；{} 秒", f.read, f.with_text, f.already, f.missing, t0.elapsed().as_secs()))
        }
        Some("terms") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            terms(&catalog_dir, &cache_dir)
        }
        Some("mastered") => {
            let mut roots = mastered_roots(&catalog_dir);
            for d in args[1..].iter().map(expand) {
                let d = d.canonicalize().map_err(|e| format!("{}: {e}", d.display()))?;
                if !roots.contains(&d) {
                    roots.push(d);
                }
            }
            let b = serde_json::to_vec_pretty(&serde_json::json!({ "roots": roots })).map_err(|e| e.to_string())?;
            store::write_atomic(&catalog_dir.join(MASTERED), &b).map_err(|e| e.to_string())?;
            Ok(format!("全懂的資料夾 {} 個（下次 terms 時套用）", roots.len()))
        }
        Some("known-cards") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            known_cards_from_mastered(&cache_dir, args.get(1).is_some_and(|a| a == "--write"))
        }
        Some("classify") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let max = args.get(1).map(|n| n.parse::<usize>().map_err(|_| USAGE.to_string())).transpose()?;
            classify(&cache_dir, max)
        }
        Some("card-titles") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
            let llm = shelf_server::llm::Llm::new(home);
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let (asked, answered) = rt.block_on(async {
                let p = llm.provider(shelf_server::llm::TASK_CARD_TITLES).await?;
                shelf_server::cards::run(&cache_dir, &*p, |done, total| eprintln!("{done} / {total}")).await
            })?;
            let t = shelf_server::cards::Titles::load(&cache_dir);
            let keep = t.titles.values().filter(|x| x.keep).count();
            Ok(format!("這次問了 {asked}，答了 {answered}；全部可做卡 {keep}、不做 {}", t.titles.len() - keep))
        }
        Some("ignore-words") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
            ignore_words(&home, &cache_dir, args.get(1).is_some_and(|a| a == "--write"))
        }
        Some("report") => Catalog::latest_report(&catalog_dir).map(|p| p.display().to_string()).ok_or_else(|| "還沒有報告；先執行 loreshelf-cli scan <資料夾>".to_string()),
        _ => Err(USAGE.into()),
    }
}

/// The term pool: the catalogue's files (never a sensitive one), the KnowledgeBase's cards, the
/// cached text → `<cache>/terms.json`. Prints counts only.
fn terms(catalog_dir: &std::path::Path, cache_dir: &std::path::Path) -> Result<String, String> {
    use lore_catalog::terms::{self, Source};
    let t0 = std::time::Instant::now();
    let idx = shelf_server::index::Index::load(catalog_dir, &lore_catalog::extract::Extract::dir(cache_dir));
    let roots = mastered_roots(catalog_dir);
    let sources: Vec<Source> = idx
        .entries
        .iter()
        .filter(|e| !e.item.sensitive && !matches!(e.item.kind.as_str(), "project" | "package" | "dataset"))
        .map(|e| Source { id: e.item.id.clone(), cloud_ok: e.item.cloud_ok, mastered: e.paths.iter().any(|p| roots.iter().any(|r| std::path::Path::new(p).starts_with(r))) })
        .collect();
    let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
    let known = known_cards(&home);
    if known.is_empty() {
        eprintln!("注意：沒有讀到知識庫的卡片（從 LoreForge 資料夾裡執行，或設定 KB_SPACES）；所有詞都會當成知識庫沒有");
    }
    let fulltext = lore_catalog::fulltext::FullText::dir(cache_dir);
    let filters = reader_filters(&home);
    let mut pool = terms::build(&fulltext, &sources, &known, &skip_keys(&home), &filters);
    if let Some(old) = terms::Pool::load(cache_dir) {
        terms::carry_verdicts(&mut pool, &old);
    }
    shelf_server::classify::settle(&mut pool, &filters);
    pool.save(cache_dir).map_err(|e| e.to_string())?;
    let near = pool.terms.iter().filter(|t| t.near_known >= 1).count();
    let cloud = pool.terms.iter().filter(|t| t.cloud_ok).count();
    Ok(format!("讀了 {} 個有文字的檔案；知識庫卡片 {} 張；候選詞 {} 個（其中 {} 個常和你已懂的卡出現在一起，{} 個出現在論文或簡報裡）；{} 秒", pool.files, known.len(), pool.terms.len(), near, cloud, t0.elapsed().as_secs()))
}

/// Where the folders he knows all of are kept (the catalog: local only, never in a repo).
const MASTERED: &str = "mastered.json";

fn mastered_roots(catalog_dir: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read(catalog_dir.join(MASTERED))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["roots"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(PathBuf::from)).collect()))
        .unwrap_or_default()
}

/// The terms of the folders he knows all of (`mastered`) that the model calls terms, as known
/// cards: title, the domains of the cards they sit next to, and the sentences they come from — no
/// file name or path (the KnowledgeBase is a repo). Through KnowledgeBase's API while it runs,
/// else straight into its folder with kb-core.
fn known_cards_from_mastered(cache_dir: &std::path::Path, write: bool) -> Result<String, String> {
    let mut pool = lore_catalog::terms::Pool::load(cache_dir).ok_or("還沒有候選詞；先執行 loreshelf-cli terms")?;
    let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
    shelf_server::classify::settle(&mut pool, &reader_filters(&home));
    let mine: Vec<&lore_catalog::terms::Term> = pool.terms.iter().filter(|t| t.mastered).collect();
    let waiting = mine.iter().filter(|t| t.verdict.is_none()).count();
    let local_only = mine.iter().filter(|t| t.verdict.as_deref() == Some("term") && !t.cloud_ok).count();
    let titles = shelf_server::cards::Titles::load(cache_dir);
    let unchecked = shelf_server::cards::candidates(&pool).filter(|t| !titles.titles.contains_key(&t.key)).count();
    // one card per concept: titles sharing a name or an acronym (`SSIM` / `Structural Similarity
    // Index Measure (SSIM)`) are one card under the fullest title; none the KB already has
    let have: std::collections::HashSet<String> = known_cards(&home).iter().flat_map(|k| std::iter::once(&k.title).chain(k.aliases.iter())).flat_map(|x| title_keys(x)).collect();
    let picked: Vec<(&lore_catalog::terms::Term, &shelf_server::cards::Titled)> =
        shelf_server::cards::candidates(&pool).filter_map(|t| titles.titles.get(&t.key).filter(|x| x.keep && !x.title.contains(" / ")).map(|x| (t, x))).filter(|(_, x)| !title_keys(&x.title).iter().any(|k| have.contains(k))).collect();
    let mut group: Vec<usize> = (0..picked.len()).collect();
    fn root(g: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while g[r] != r {
            r = g[r];
        }
        g[i] = r;
        r
    }
    let mut first: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, (_, x)) in picked.iter().enumerate() {
        for k in title_keys(&x.title) {
            match first.get(&k) {
                Some(&j) => {
                    let (a, b) = (root(&mut group, i), root(&mut group, j));
                    group[a] = b;
                }
                None => {
                    first.insert(k, i);
                }
            }
        }
    }
    let mut cards: std::collections::BTreeMap<usize, (String, Vec<String>, Vec<String>)> = std::collections::BTreeMap::new();
    for (i, (t, x)) in picked.iter().enumerate() {
        let c = cards.entry(root(&mut group, i)).or_insert_with(|| (x.title.clone(), Vec::new(), Vec::new()));
        // the fullest title: one with its acronym, then the longest
        let score = |s: &str| (s.contains('('), s.len());
        if score(&x.title) > score(&c.0) {
            c.0 = x.title.clone();
        }
        for d in std::iter::once(&x.field).filter(|f| !f.is_empty()).chain(t.domains.iter()) {
            if !c.1.contains(d) {
                c.1.push(d.clone());
            }
        }
        // sentences from papers and slide decks only (the KnowledgeBase is a repo with a remote)
        for e in t.examples.iter().filter(|e| e.cloud_ok).map(|e| e.text.trim()).filter(|e| e.chars().count() >= 40) {
            if c.2.len() < 3 && !c.2.iter().any(|x| x == e) {
                c.2.push(e.to_string());
            }
        }
    }
    let status = format!("全懂資料夾裡的候選詞 {}：可做成已懂的卡 {} 張（已去重、略過知識庫已有的）；Claude 還沒確認 {unchecked}，模型還沒分類 {waiting}，只出現在非論文檔案而不送雲端的術語 {local_only}", mine.len(), cards.len());
    if !write {
        // LORESHELF_LIST=1: the titles, one per line
        if std::env::var_os("LORESHELF_LIST").is_some() {
            for c in cards.values() {
                println!("{}", c.0);
            }
        }
        return Ok(format!("{status}；加 --write 才會寫進知識庫"));
    }
    let mut made = 0;
    for (title, domains, examples) in cards.values() {
        let body = examples.iter().map(|e| format!("> {e}")).collect::<Vec<_>>().join("\n\n");
        let card = serde_json::json!({ "title": title, "domains": domains, "body": body, "initial_state": "known", "via": "loreshelf" });
        kb_create(&home, &card)?;
        made += 1;
    }
    Ok(format!("做成已懂的知識卡 {made} 張\n{status}"))
}

/// The names a card title goes by: the title without its brackets, and what is in them
/// (`Structural Similarity Index (SSIM)` → `structural similarity index`, `ssim`).
fn title_keys(title: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut base, mut inner, mut depth) = (String::new(), String::new(), 0);
    for c in title.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                out.push(lore_words::phrase_key(&std::mem::take(&mut inner)));
            }
            _ if depth > 0 => inner.push(c),
            _ => base.push(c),
        }
    }
    out.push(lore_words::phrase_key(&base));
    // spelling apart (`IU-Xray` / `IU X-Ray`)
    let compact: Vec<String> = out.iter().map(|k| k.chars().filter(|c| c.is_alphanumeric()).collect()).collect();
    out.extend(compact);
    out.retain(|k| !k.is_empty());
    out.dedup();
    out
}

/// One card into the KnowledgeBase: its API while it runs (ports.json `kb`), else kb-core on its folder.
fn kb_create(home: &lore_home::Home, card: &serde_json::Value) -> Result<(), String> {
    if let Some(port) = lore_home::ports::live().get("kb").copied() {
        let body = card.to_string();
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3)).map_err(|e| e.to_string())?;
        let req = format!("POST /api/nodes HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        return if out.starts_with("HTTP/1.1 200") || out.starts_with("HTTP/1.1 201") { Ok(()) } else { Err(format!("KnowledgeBase：{}", out.lines().next().unwrap_or_default())) };
    }
    let root = home.app_dir("KnowledgeBase").filter(|d| d.join("space.toml").is_file()).ok_or("找不到知識庫（private/KnowledgeBase）")?;
    let mut spaces = lore_store::Spaces::new();
    spaces.mount(&root).map_err(|e| e.to_string())?;
    let member = home.prefs().profile.id.unwrap_or_else(|| "me".into());
    let mut kb = kb_core::KnowledgeBase::open(spaces, &member, kb_core::KbOptions::default()).map_err(|e| e.to_string())?;
    let space = kb.spaces_info().map_err(|e| e.to_string())?.first().map(|s| s.id.clone()).ok_or("知識庫沒有 space")?;
    let fields = kb_core::NodeFields {
        title: card["title"].as_str().map(str::to_string),
        domains: card["domains"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()),
        body: card["body"].as_str().map(str::to_string),
        ..Default::default()
    };
    kb.create_as(&space, fields, Some("loreshelf"), kb_core::State::Known).map_err(|e| e.to_string())?;
    Ok(())
}

/// The local model sorts the pool (background priority: it waits while the author uses the model).
fn classify(cache_dir: &std::path::Path, max: Option<usize>) -> Result<String, String> {
    let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
    let known = known_cards(&home);
    // the reader's fields: the domains of his cards, most frequent first
    let mut doms: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for k in &known {
        for d in &k.domains {
            *doms.entry(d.as_str()).or_default() += 1;
        }
    }
    let mut doms: Vec<(&str, usize)> = doms.into_iter().collect();
    doms.sort_by(|a, b| b.1.cmp(&a.1));
    let fields: Vec<String> = doms.into_iter().take(5).map(|d| d.0.to_string()).collect();
    let llm = shelf_server::llm::Llm::new(home.clone());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let t0 = std::time::Instant::now();
    let r = rt.block_on(async {
        let p = llm.provider(shelf_server::llm::TASK_CLASSIFY).await?;
        shelf_server::classify::run(cache_dir, &*p, &fields, &reader_filters(&home), max, |done, total| eprintln!("{done} / {total}")).await
    });
    shelf_server::llm::quit();
    let r = r?;
    let added = ignore_words(&home, cache_dir, true)?;
    let pool = lore_catalog::terms::Pool::load(cache_dir).unwrap_or_default();
    let count = |v: &str| pool.terms.iter().filter(|t| t.verdict.as_deref() == Some(v)).count();
    Ok(format!(
        "這次問了 {}，答了 {}（{} 批沒有結果）；全部：術語 {}、一般詞 {}、名稱 {}、雜訊 {}，還沒分 {}；{} 秒\n{added}",
        r.asked, r.answered, r.failed_batches, count("term"), count("general"), count("name"), count("junk"), r.left, t0.elapsed().as_secs()
    ))
}

/// The ignore list every reader of papers uses (LoreReader's 「這不是術語」, also read by
/// PaperComposer and lore-learn).
fn ignore_file(home: &lore_home::Home) -> Option<PathBuf> {
    home.app_dir("LoreReader").filter(|d| d.is_dir()).map(|d| d.join("ignore-words.txt"))
}

/// The word filters with the author's ignore list.
fn reader_filters(home: &lore_home::Home) -> lore_words::Filters {
    ignore_file(home).map(|p| lore_words::Filters::with_file(&p)).unwrap_or_default()
}

/// Keys that never enter the pool: the ignore list and my-vocab (words of his own papers).
fn skip_keys(home: &lore_home::Home) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    if let Some(p) = ignore_file(home) {
        for l in lore_words::read_lines(&p) {
            let l = l.split('#').next().unwrap_or("").trim().to_string();
            if !l.is_empty() {
                out.insert(lore_words::phrase_key(&l));
            }
        }
    }
    let vocab = home.shared_dir().map(|d| d.join("my-vocab.json")).and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    for e in vocab.iter().flat_map(|v| v["entries"].as_array().into_iter().flatten()) {
        if let Some(k) = e["key"].as_str() {
            out.insert(k.to_string());
        }
    }
    out
}

/// General words and names of the pool → the shared ignore list (appended, tagged).
fn ignore_words(home: &lore_home::Home, cache_dir: &std::path::Path, write: bool) -> Result<String, String> {
    let mut pool = lore_catalog::terms::Pool::load(cache_dir).ok_or("還沒有候選詞；先執行 loreshelf-cli terms")?;
    let path = ignore_file(home).ok_or("找不到 private/LoreReader（從 LoreForge 資料夾裡執行）")?;
    let filters = reader_filters(home);
    shelf_server::classify::settle(&mut pool, &filters);
    let cur = std::fs::read_to_string(&path).unwrap_or_default();
    let lines = shelf_server::classify::ignore_lines(&pool, &filters, &cur);
    if !write || lines.is_empty() {
        return Ok(format!("可加進「這不是術語」清單的一般詞與名稱 {} 個{}", lines.len(), if write { "" } else { "；加 --write 才會寫入" }));
    }
    let mut out = cur.trim_end().to_string();
    if !out.is_empty() {
        out.push('\n');
    }
    out += &lines.join("\n");
    out.push('\n');
    store::write_atomic(&path, out.as_bytes()).map_err(|e| e.to_string())?;
    Ok(format!("「這不是術語」清單加了 {} 個一般詞與名稱（標 {}）", lines.len(), shelf_server::classify::IGNORE_TAG))
}

/// The KnowledgeBase's cards (titles, aliases, domains) and the member's state of each, read from
/// its folders (`KB_SPACES`, else `private/KnowledgeBase`); never written.
fn known_cards(home: &lore_home::Home) -> Vec<lore_catalog::terms::Known> {
    let spaces: Vec<PathBuf> = match std::env::var("KB_SPACES") {
        Ok(v) if !v.is_empty() => v.split(':').map(PathBuf::from).collect(),
        _ => home.app_dir("KnowledgeBase").into_iter().collect(),
    };
    let member = std::env::var("KB_MEMBER").ok().or_else(|| home.prefs().profile.id).unwrap_or_else(|| "me".into());
    let mut out = Vec::new();
    for root in spaces.into_iter().filter(|d| d.join("space.toml").is_file()) {
        let Ok(rd) = std::fs::read_dir(root.join("nodes")) else { continue };
        for f in rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")) {
            let Some(v) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { continue };
            let strs = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let (Some(id), Some(title)) = (v["id"].as_str(), v["title"].as_str()) else { continue };
            let state = std::fs::read(root.join("progress").join(&member).join(format!("{id}.json"))).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|p| p["state"].as_str().map(str::to_string)).unwrap_or_default();
            out.push(lore_catalog::terms::Known { title: title.to_string(), aliases: strs("aliases"), domains: strs("domains"), state });
        }
    }
    out
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
        assert!(run(&["nope".into()]).unwrap_err().contains("loreshelf-cli scan"));
        assert!(run(&["scan".into()]).is_err(), "scan needs a folder");
        if let Some(h) = std::env::var_os("HOME") {
            assert_eq!(expand(&"~/x".to_string()), PathBuf::from(h).join("x"));
        }
        assert_eq!(expand(&"/a/b".to_string()), PathBuf::from("/a/b"));
    }

    #[test]
    fn a_card_title_goes_by_its_name_and_its_acronym() {
        assert_eq!(title_keys("Structural Similarity Index (SSIM)"), vec!["ssim", "structural similarity index", "ssim", "structuralsimilarityindex"]);
        assert_eq!(title_keys("SSIM"), vec!["ssim"]);
        assert!(title_keys("IU X-Ray").contains(&"iuxray".to_string()) && title_keys("IU-Xray").contains(&"iuxray".to_string()));
        assert_eq!(title_keys("CPAB (Continuous Piecewise-Affine Based) Transformations")[1], "cpab transformation");
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
