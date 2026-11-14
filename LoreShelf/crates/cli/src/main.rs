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
//! loreshelf-cli plan [<source> [page]]  歸位 phase A: where each file / folder of the source folders
//!                           should go in the new tree (`<catalog>/filing-plan.json`); a plan only,
//!                           nothing moves. With a source: its rows, 20 a page
//! loreshelf-cli ignore-words [--write]  general words and names → `private/LoreReader/ignore-words.txt`
//!                           (the 「這不是術語」 list every paper reader uses; lines tagged `# LoreShelf`)
//! ```
//!
//! Catalog: `$LORESHELF_HOME`, else `~/Library/Application Support/LoreForge/LoreShelf`;
//! hash cache: `<cache>/LoreForge/shelf/` (`$LOREFORGE_CACHE` overrides the base).

use lore_catalog::report::human;
use lore_catalog::{Catalog, Guard, ScanConfig, store};
use shelf_server::kbtags::Vocabulary;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "用法：\n  loreshelf-cli scan <資料夾>...   只讀掃描，寫入本機目錄與報告\n  loreshelf-cli report            最新報告的位置\n  loreshelf-cli extract           讀取檔案全文到快取（只補還沒讀的）\n  loreshelf-cli terms             整理候選詞（知識庫還沒有的詞）\n  loreshelf-cli mastered <資料夾>...  這些資料夾的檔案我全懂（裡面的術語會做成已懂的卡）\n  loreshelf-cli known-cards [--write]  把上面那些資料夾的術語做成已懂的知識卡（不加 --write 只算數量）\n  loreshelf-cli study-cards [--write]  正在學的領域（private/shared/learning.json）的詞做成不熟的卡（NeonDeck 會跳）\n  loreshelf-cli card-content      用 Claude Haiku 幫已懂的卡寫摘要與說明（基本詞不做卡）\n  loreshelf-cli known-cards --refill [--write]  先前只有原文引句的卡補上摘要與說明，基本詞移到垃圾桶（KnowledgeBase 要開著）\n  loreshelf-cli card-titles       用 Claude Haiku 確認全懂資料夾的術語並給卡片標題（只送論文與簡報裡的詞和句子）\n  loreshelf-cli classify [數量] [--cloud]   本機模型分類候選詞（每批存檔，可中斷續跑；跑完把一般詞寫進忽略清單；--cloud 用 Claude Haiku，只問論文與簡報裡的詞）\n  loreshelf-cli plan [來源資料夾 [頁]]   歸位計畫：每個檔案／資料夾該去新資料夾樹的哪裡（只產生計畫，不搬）\n  loreshelf-cli move <來源資料夾> [--dry]   把規則確定的列搬過去（只改名搬移，不複製不刪除；記在日誌裡）\n  loreshelf-cli undo              復原上一批搬移\n  loreshelf-cli guess [來源資料夾]   規則放不了的檔案問模型（論文只送內容給 Claude，其他只用本機模型）\n  loreshelf-cli labels [--count]  每個檔案 3–5 個英文標籤（同上兩條路；--count 只算數量）\n  loreshelf-cli move <來源資料夾> --model [--dry]   搬模型放的列（先 --dry 看過）\n  loreshelf-cli ignore-words [--write]  一般詞與名稱寫進共用的「這不是術語」清單（不加 --write 只算數量）\n  loreshelf-cli kb-tags [--write] [--sample 數量] [--ids 卡片id,…] [--vocab tags.json]  知識卡的領域與標籤（只從詞彙表選；本機詞做的卡只給本機模型；不加 --write 只列出）";

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
            let write = args.iter().any(|a| a == "--write");
            if args.iter().any(|a| a == "--refill") { refill_cards(&cache_dir, write) } else { known_cards_from_mastered(&cache_dir, write) }
        }
        Some("classify") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let cloud = args.iter().any(|a| a == "--cloud");
            let max = args.iter().skip(1).find(|a| *a != "--cloud").map(|n| n.parse::<usize>().map_err(|_| USAGE.to_string())).transpose()?;
            classify(&cache_dir, max, cloud)
        }
        Some("study-cards") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            study_cards(&cache_dir, args.iter().any(|a| a == "--write"))
        }
        Some("card-content") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
            let llm = shelf_server::llm::Llm::new(home);
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let (asked, answered) = rt.block_on(async {
                let p = llm.provider(shelf_server::llm::TASK_CARD_TITLES).await?;
                shelf_server::cards::write_content(&cache_dir, &*p, |done, total| eprintln!("{done} / {total}")).await
            })?;
            let t = shelf_server::cards::Titles::load(&cache_dir);
            let dropped = t.titles.values().filter(|x| x.written && !x.keep).count();
            Ok(format!("這次寫了 {asked}，答了 {answered}；有內容的卡 {}、當成基本詞不做 {dropped}", t.titles.values().filter(|x| x.written && x.keep).count()))
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
        Some("plan") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            plan(&catalog_dir, &cache_dir, args.get(1).map(expand), args.get(2).and_then(|n| n.parse().ok()).unwrap_or(1))
        }
        Some("move") => {
            let src = args.get(1).map(expand).ok_or(USAGE)?;
            move_certain(&catalog_dir, &src, args.iter().any(|a| a == "--dry"), args.iter().any(|a| a == "--model"))
        }
        Some("labels") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            labels(&catalog_dir, &cache_dir, args.iter().any(|a| a == "--count"))
        }
        Some("guess") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let src = args.get(1).filter(|a| !a.starts_with("--")).map(expand);
            guess(&catalog_dir, &cache_dir, src)
        }
        Some("undo") => {
            let (back, left) = shelf_server::filing::undo(&catalog_dir.join(shelf_server::filing::JOURNAL))?;
            Ok(format!("搬回 {back} 個；留在原地 {} 個{}", left.len(), left.iter().map(|l| format!("\n  {l}")).collect::<String>()))
        }
        Some("ignore-words") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
            if args.iter().any(|a| a == "--recheck") {
                recheck_general(&home, &cache_dir, args.iter().any(|a| a == "--write"))
            } else {
                ignore_words(&home, &cache_dir, args.get(1).is_some_and(|a| a == "--write"))
            }
        }
        Some("kb-tags") => {
            let cache_dir = store::default_cache_dir().ok_or("找不到快取位置（請設定 LOREFORGE_CACHE）")?;
            let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
            let sample = opt("--sample").map(|n| n.parse::<usize>().map_err(|_| USAGE.to_string())).transpose()?;
            let ids: Vec<String> = opt("--ids").map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
            kb_tags(&cache_dir, args.iter().any(|a| a == "--write"), sample, &ids, opt("--vocab").map(|p| expand(&p)))
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
    let unwritten = titles.titles.values().filter(|x| x.keep && !x.written).count();
    // one card per concept: titles sharing a name or an acronym (`SSIM` / `Structural Similarity
    // Index Measure (SSIM)`) are one card under the fullest title; none the KB already has
    let have: std::collections::HashSet<String> = known_cards(&home).iter().flat_map(|k| std::iter::once(&k.title).chain(k.aliases.iter())).flat_map(|x| title_keys(x)).collect();
    let picked: Vec<(&lore_catalog::terms::Term, &shelf_server::cards::Titled)> =
        shelf_server::cards::candidates(&pool).filter_map(|t| titles.titles.get(&t.key).filter(|x| x.keep && x.written && !x.title.contains(" / ")).map(|x| (t, x))).filter(|(_, x)| !title_keys(&x.title).iter().any(|k| have.contains(k))).collect();
    let cards = merge_cards(&picked, &kb_vocab(&home));
    let status = format!("全懂資料夾裡的候選詞 {}：可做成已懂的卡 {} 張（已去重、略過知識庫已有的）；Claude 還沒確認 {unchecked}，還沒寫內容 {unwritten}，模型還沒分類 {waiting}，只出現在非論文檔案而不送雲端的術語 {local_only}", mine.len(), cards.len());
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
    for (title, domains, summary, body) in cards.values() {
        let card = serde_json::json!({ "title": title, "domains": domains, "summary": summary, "body": body, "initial_state": "known", "via": "loreshelf" });
        kb_create(&home, &card)?;
        made += 1;
    }
    Ok(format!("做成已懂的知識卡 {made} 張\n{status}"))
}

/// The fields the author is learning (`private/shared/learning.json`: `{"learning": [...]}`, the
/// model's field names, matched as parts): their terms become 不熟 (learning) cards.
fn learning_fields(home: &lore_home::Home) -> Vec<String> {
    let p = home.shared_file("learning.json");
    let t: serde_json::Value = p.and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    t.get("learning").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).filter(|x| !x.trim().is_empty()).collect()).unwrap_or_default()
}

/// Cards for the terms of the fields he is learning (author, 2026-11-14): titles and words by
/// Claude for terms found in papers and slide decks, by the local model for the rest; written as
/// 不熟 (learning) cards, which NeonDeck's 知識卡輪播 shows. None the KB already has.
fn study_cards(cache_dir: &std::path::Path, write: bool) -> Result<String, String> {
    let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
    let fields = learning_fields(&home);
    if fields.is_empty() {
        return Err("先在 private/shared/learning.json 寫 {\"learning\": [\"領域\", …]}".into());
    }
    let pool = lore_catalog::terms::Pool::load(cache_dir).ok_or("還沒有候選詞；先執行 loreshelf-cli terms")?;
    let keys: std::collections::HashSet<String> = pool
        .terms
        .iter()
        .filter(|t| !t.mastered && t.verdict.as_deref() == Some("term") && t.field.as_deref().is_some_and(|f| fields.iter().any(|x| f.contains(x.as_str()))))
        .map(|t| t.key.clone())
        .collect();
    let llm = shelf_server::llm::Llm::new(home.clone());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let r = rt.block_on(async {
        let cloud = llm.provider(shelf_server::llm::TASK_CARD_TITLES).await?;
        let local = llm.provider(shelf_server::llm::TASK_CARD_LOCAL).await?;
        let tick = |d: usize, t: usize| eprintln!("{d} / {t}");
        shelf_server::cards::run_for(cache_dir, &*cloud, Some(&keys), false, tick).await?;
        shelf_server::cards::run_for(cache_dir, &*local, Some(&keys), true, tick).await?;
        shelf_server::cards::write_content_for(cache_dir, &*cloud, Some(&keys), false, tick).await?;
        shelf_server::cards::write_content_for(cache_dir, &*local, Some(&keys), true, tick).await
    });
    // the local model this started is not left running
    shelf_server::llm::quit();
    r?;
    let titles = shelf_server::cards::Titles::load(cache_dir);
    let have: std::collections::HashSet<String> = known_cards(&home).iter().flat_map(|k| std::iter::once(&k.title).chain(k.aliases.iter())).flat_map(|x| title_keys(x)).collect();
    let picked: Vec<(&lore_catalog::terms::Term, &shelf_server::cards::Titled)> = pool
        .terms
        .iter()
        .filter(|t| keys.contains(&t.key))
        .filter_map(|t| titles.titles.get(&t.key).filter(|x| x.keep && x.written && !x.title.contains(" / ")).map(|x| (t, x)))
        .filter(|(_, x)| !title_keys(&x.title).iter().any(|k| have.contains(k)))
        .collect();
    let cards = merge_cards(&picked, &kb_vocab(&home));
    if !write {
        if std::env::var_os("LORESHELF_LIST").is_some() {
            for c in cards.values() {
                println!("{}｜{}", c.0, c.2);
            }
        }
        return Ok(format!("{} 的詞 {}：可做成不熟的卡 {} 張；加 --write 才會寫進知識庫", fields.join("、"), keys.len(), cards.len()));
    }
    let mut made = 0;
    for (title, domains, summary, body) in cards.values() {
        let card = serde_json::json!({ "title": title, "domains": domains, "summary": summary, "body": body, "initial_state": "learning", "via": "loreshelf" });
        kb_create(&home, &card)?;
        made += 1;
    }
    Ok(format!("做成不熟的知識卡 {made} 張（NeonDeck 的知識卡輪播會挑它們）"))
}

/// One card per concept: titles sharing a name or an acronym (`SSIM` / `Structural Similarity
/// Index Measure (SSIM)`) are one card under the fullest title, with that title's words.
/// (title, domains, summary, body)
/// Domains are only ever registered ones (`vocab`, the KnowledgeBase's tags.json): a model's free
/// field (「醫學影像」) is mapped to the one domain it names, else left out for `kb-tags`.
fn merge_cards(picked: &[(&lore_catalog::terms::Term, &shelf_server::cards::Titled)], vocab: &Vocabulary) -> std::collections::BTreeMap<usize, (String, Vec<String>, String, String)> {
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
    // (title, domains, summary, body): the card's own words, from its fullest title's answer
    let mut cards: std::collections::BTreeMap<usize, (String, Vec<String>, String, String)> = std::collections::BTreeMap::new();
    for (i, (t, x)) in picked.iter().enumerate() {
        let c = cards.entry(root(&mut group, i)).or_insert_with(|| (x.title.clone(), Vec::new(), x.summary.clone(), x.body.clone()));
        // the fullest title: one with its acronym, then the longest
        let score = |s: &str| (s.contains('('), s.len());
        if score(&x.title) > score(&c.0) {
            c.0 = x.title.clone();
            c.2 = x.summary.clone();
            c.3 = x.body.clone();
        }
        for d in std::iter::once(&x.field).filter(|f| !f.is_empty()).chain(t.domains.iter()).filter_map(|f| vocab.map_field(f)) {
            if !c.1.contains(&d) {
                c.1.push(d);
            }
        }
    }
    cards
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

/// One call to KnowledgeBase's API while it runs (ports.json `kb`): (status, body); `None` = not running.
fn kb_api(method: &str, path: &str, body: Option<&serde_json::Value>) -> Option<Result<(u16, serde_json::Value), String>> {
    let port = lore_home::ports::live().get("kb").copied()?;
    Some(kb_api_at(port, method, path, body))
}

/// The cards LoreShelf made before they had words of their own (2026-11-13: quotes as the body,
/// no summary) get their summary and body, or go to the KnowledgeBase's trash when the content
/// pass found them a field basic. Only through KnowledgeBase's API (it must be open).
fn refill_cards(cache_dir: &std::path::Path, write: bool) -> Result<String, String> {
    let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
    let dir = home.app_dir("KnowledgeBase").map(|d| d.join("nodes")).filter(|d| d.is_dir()).ok_or("找不到知識庫（private/KnowledgeBase）")?;
    let titles = shelf_server::cards::Titles::load(cache_dir);
    let by_key: std::collections::HashMap<String, &shelf_server::cards::Titled> = titles.titles.values().filter(|x| x.written && !x.title.is_empty()).flat_map(|x| title_keys(&x.title).into_iter().map(move |k| (k, x))).collect();
    // dropped titles keep no title: find them by the term key
    let dropped: std::collections::HashSet<String> = titles.titles.iter().filter(|(_, x)| x.written && !x.keep).map(|(k, _)| k.to_lowercase()).collect();
    let (mut fill, mut drop, mut left) = (vec![], vec![], 0);
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&std::fs::read(e.path()).unwrap_or_default()) else { continue };
        let (summary, body, title) = (v["summary"].as_str().unwrap_or(""), v["body"].as_str().unwrap_or(""), v["title"].as_str().unwrap_or(""));
        // only cards LoreShelf made (their learning log says so) that still have no words of their own
        let id = v["id"].as_str().unwrap_or("");
        if !summary.trim().is_empty() || !(body.trim().is_empty() || body.trim_start().starts_with("> ")) || !made_by_loreshelf(&dir, id) {
            continue;
        }
        let keys = title_keys(title);
        match keys.iter().find_map(|k| by_key.get(k)) {
            Some(x) if x.keep => fill.push((v["id"].as_str().unwrap_or("").to_string(), v["revision"].as_u64(), title.to_string(), x.summary.clone(), x.body.clone())),
            Some(_) => drop.push((v["id"].as_str().unwrap_or("").to_string(), title.to_string())),
            None if keys.iter().any(|k| dropped.contains(k)) => drop.push((v["id"].as_str().unwrap_or("").to_string(), title.to_string())),
            None => left += 1,
        }
    }
    let mut out = format!("要補內容 {} 張、移到垃圾桶 {} 張（{}）、還沒有內容可補 {left} 張", fill.len(), drop.len(), drop.iter().map(|d| d.1.as_str()).collect::<Vec<_>>().join("、"));
    if !write {
        return Ok(format!("{out}；加 --write 才會改"));
    }
    let (mut done, mut trashed) = (0, 0);
    for (id, rev, title, summary, body) in &fill {
        match kb_api("PUT", &format!("/api/nodes/{id}"), Some(&serde_json::json!({ "revision": rev, "summary": summary, "body": body }))).ok_or("KnowledgeBase 沒開：打開它再跑")? {
            Ok((200, _)) => done += 1,
            Ok((s, v)) => eprintln!("〈{title}〉沒改：{s} {}", v["error"].as_str().unwrap_or("")),
            Err(e) => eprintln!("〈{title}〉沒改：{e}"),
        }
    }
    for (id, title) in &drop {
        match kb_api("DELETE", &format!("/api/nodes/{id}"), None).ok_or("KnowledgeBase 沒開：打開它再跑")? {
            Ok((200, _)) => trashed += 1,
            Ok((s, v)) => eprintln!("〈{title}〉沒移：{s} {}", v["error"].as_str().unwrap_or("")),
            Err(e) => eprintln!("〈{title}〉沒移：{e}"),
        }
    }
    out = format!("補好 {done} 張，移到垃圾桶 {trashed} 張（可以從 KnowledgeBase 的垃圾桶救回）");
    Ok(out)
}

/// Did LoreShelf make this card? (`progress/<member>/<id>.json`: `{event: created, via: loreshelf}`)
fn made_by_loreshelf(nodes: &std::path::Path, id: &str) -> bool {
    let Some(progress) = nodes.parent().map(|d| d.join("progress")) else { return false };
    std::fs::read_dir(progress).into_iter().flatten().flatten().any(|m| {
        std::fs::read(m.path().join(format!("{id}.json"))).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).is_some_and(|v| {
            v["log"].as_array().is_some_and(|l| l.iter().any(|e| e["event"] == "created" && e["via"] == "loreshelf"))
        })
    })
}

/// One card into the KnowledgeBase: its API while it runs (ports.json `kb`), else kb-core on its folder.
fn kb_create(home: &lore_home::Home, card: &serde_json::Value) -> Result<(), String> {
    // only registered domains (a free one would be a colour of its own in KnowledgeBase)
    let vocab = kb_vocab(home);
    let mut card = card.clone();
    let doms: Vec<serde_json::Value> = card["domains"].as_array().map(|a| a.iter().filter_map(|d| d.as_str().and_then(|d| vocab.map_field(d))).map(serde_json::Value::from).collect()).unwrap_or_default();
    card["domains"] = serde_json::Value::from(doms);
    let card = &card;
    if let Some(port) = lore_home::ports::live().get("kb").copied() {
        // every write names its space: the member's personal one (else the first)
        let mut card = card.clone();
        if card.get("space").is_none() {
            let (_, v) = kb_api("GET", "/api/spaces", None).ok_or("KnowledgeBase 沒開")??;
            let spaces = v["spaces"].as_array().cloned().unwrap_or_default();
            let sp = spaces.iter().find(|x| x["kind"] == "personal" && x["is_member"] == true).or(spaces.first()).and_then(|x| x["id"].as_str()).ok_or("知識庫沒有 space")?;
            card["space"] = serde_json::json!(sp);
        }
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
        summary: card["summary"].as_str().map(str::to_string),
        domains: card["domains"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()),
        body: card["body"].as_str().map(str::to_string),
        ..Default::default()
    };
    let state = if card["initial_state"] == "learning" { kb_core::State::Learning } else { kb_core::State::Known };
    kb.create_as(&space, fields, Some("loreshelf"), state).map_err(|e| e.to_string())?;
    Ok(())
}

/// The local model sorts the pool (background priority: it waits while the author uses the model).
/// `cloud`: Claude Haiku instead, for the terms found in papers and slide decks only.
fn classify(cache_dir: &std::path::Path, max: Option<usize>, cloud: bool) -> Result<String, String> {
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
    // the prompt reads the fields by name: the zh labels of tags.json
    let vocab = kb_vocab(&home);
    let fields: Vec<String> = doms.into_iter().take(5).map(|d| vocab.label(d.0).to_string()).collect();
    let llm = shelf_server::llm::Llm::new(home.clone());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let t0 = std::time::Instant::now();
    let r = rt.block_on(async {
        let p = llm.provider(if cloud { shelf_server::llm::TASK_CARD_TITLES } else { shelf_server::llm::TASK_CLASSIFY }).await?;
        shelf_server::classify::run(cache_dir, &*p, &fields, &reader_filters(&home), max, cloud, |done, total| eprintln!("{done} / {total}")).await
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

/// 歸位 phase A: the plan (settings made once from the home folder), a summary, or one page of rows.
fn plan(catalog_dir: &std::path::Path, cache_dir: &std::path::Path, source: Option<PathBuf>, page: usize) -> Result<String, String> {
    use shelf_server::filing::{self, Settings};
    let settings = match Settings::load(catalog_dir) {
        Some(s) => s,
        None => {
            let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("找不到家目錄")?;
            let s = Settings::default_for(&home);
            let b = serde_json::to_vec_pretty(&s).map_err(|e| e.to_string())?;
            store::write_atomic(&catalog_dir.join(filing::SETTINGS), &b).map_err(|e| e.to_string())?;
            s
        }
    };
    let mut idx = shelf_server::index::Index::load(catalog_dir, &lore_catalog::extract::Extract::dir(cache_dir));
    // the catalogue remembers where a file was seen; only where it is now can move
    for e in &mut idx.entries {
        e.paths.retain(|p| std::fs::symlink_metadata(p).is_ok());
    }
    let made = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs().to_string()).unwrap_or_default();
    let mut p = filing::plan(&idx.entries, &settings, &made);
    shelf_server::filing_ai::apply(&mut p, &shelf_server::filing_ai::Guesses::load(catalog_dir), &settings.topics);
    let b = serde_json::to_vec_pretty(&p).map_err(|e| e.to_string())?;
    store::write_atomic(&catalog_dir.join(filing::PLAN), &b).map_err(|e| e.to_string())?;
    if let Some(src) = source {
        let rows: Vec<&filing::Row> = p.rows.iter().filter(|r| std::path::Path::new(&r.from).starts_with(&src)).collect();
        let pages = rows.len().div_ceil(20).max(1);
        let mut out = format!("{}：{} 列，第 {page} / {pages} 頁\n", src.display(), rows.len());
        for r in rows.iter().skip((page.max(1) - 1) * 20).take(20) {
            let rel = std::path::Path::new(&r.from).strip_prefix(&src).map(|x| x.display().to_string()).unwrap_or_else(|_| r.from.clone());
            let what = if r.files > 1 { format!("{rel}/（{} 個檔）", r.files) } else { rel };
            let to = r.target.clone().unwrap_or_else(|| "（不搬：規則判斷不了）".into());
            let flags = if r.flags.is_empty() { String::new() } else { format!(" [{}]", r.flags.join(", ")) };
            out += &format!("  {what}  →  {to}  ·{}{flags}\n", r.reason);
        }
        return Ok(out);
    }
    let mut out = format!("歸位計畫（只是計畫，沒有搬任何東西）：根資料夾 {}；不動的資料夾 {} 個（{}）\n", settings.root.display(), settings.keep.len(), filing::SETTINGS);
    out += "來源 · 列 · 檔案 · 判斷不了 · 要定主題 · 要英文名\n";
    for (s, n) in filing::summary(&p, &settings.sources) {
        out += &format!("  {s} · {} · {} · {} · {} · {}\n", n[0], n[1], n[2], n[3], n[4]);
    }
    let mut tops: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for r in &p.rows {
        let t = r.target.as_deref().map(|t| t.split('/').take(if t.starts_with("Research") { 3 } else { 2 }).collect::<Vec<_>>().join("/")).unwrap_or_else(|| "（判斷不了）".into());
        *tops.entry(t).or_default() += r.files;
    }
    out += "去向（檔案數）：\n";
    for (t, n) in tops {
        out += &format!("  {t}: {n}\n");
    }
    Ok(out)
}

/// 歸位 phase B (from the command line): the rows of the saved plan under `src` that the rules
/// are sure of, moved as one batch (rename only) and journaled for `undo`.
fn move_certain(catalog_dir: &std::path::Path, src: &std::path::Path, dry: bool, model: bool) -> Result<String, String> {
    use shelf_server::filing::{self, Plan};
    let b = std::fs::read(catalog_dir.join(filing::PLAN)).map_err(|_| "還沒有計畫；先執行 loreshelf-cli plan")?;
    let p: Plan = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
    let rows: Vec<filing::Row> = p.rows.into_iter().filter(|r| std::path::Path::new(&r.from).starts_with(src) && if model { r.flags.iter().any(|f| f == "by-model") && r.target.is_some() } else { filing::certain(r) }).collect();
    let mut out = String::new();
    if dry {
        for r in &rows {
            let to = filing::dest(r, &p.root).map(|d| d.display().to_string()).unwrap_or_default();
            out += &format!("  {}  →  {to}\n", r.from);
        }
        return Ok(format!("{out}{} {} 列（{} 個檔）；沒有搬", if model { "模型放的" } else { "確定的" }, rows.len(), rows.iter().map(|r| r.files).sum::<usize>()));
    }
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs().to_string()).unwrap_or_default();
    let (batch, skipped) = filing::apply(&rows, &p.root, &catalog_dir.join(filing::JOURNAL), &at);
    for m in &batch.moves {
        out += &format!("  {}  →  {}\n", m.from, m.to);
    }
    for s in &skipped {
        out += &format!("  沒搬：{s}\n");
    }
    Ok(format!("{out}搬了 {} 列，沒搬 {}（loreshelf-cli undo 可以整批復原）", batch.moves.len(), skipped.len()))
}

/// 歸位 phase D: the files under `src` (default: every source) the rules left open, asked of the
/// models — a paper or deck's text alone to Claude, everything else to the local model — then the
/// plan again with the answers.
fn guess(catalog_dir: &std::path::Path, cache_dir: &std::path::Path, src: Option<PathBuf>) -> Result<String, String> {
    use shelf_server::filing::{self, Plan, OPEN};
    plan(catalog_dir, cache_dir, None, 1)?;
    let p: Plan = serde_json::from_slice(&std::fs::read(catalog_dir.join(filing::PLAN)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let idx = shelf_server::index::Index::load(catalog_dir, &lore_catalog::extract::Extract::dir(cache_dir));
    let by_id: std::collections::HashMap<&str, &shelf_server::index::Entry> = idx.entries.iter().map(|e| (e.item.id.as_str(), e)).collect();
    let ft_dir = lore_catalog::fulltext::FullText::dir(cache_dir);
    let mut asks = Vec::new();
    for r in &p.rows {
        let open = r.target.as_deref().is_none_or(|t| t.contains(OPEN));
        if !open || r.id.is_empty() || r.flags.iter().any(|f| f == "by-model") || src.as_ref().is_some_and(|s| !std::path::Path::new(&r.from).starts_with(s)) {
            continue;
        }
        let Some(e) = by_id.get(r.id.as_str()) else { continue };
        if let Some(a) = ask_of(e, &ft_dir) {
            asks.push(a);
        }
    }
    let (asked, answered) = ask_models(catalog_dir, &asks)?;
    let summary = plan(catalog_dir, cache_dir, None, 1)?;
    Ok(format!("這次問了 {asked}，答了 {answered}\n{}", summary.lines().take(5).collect::<Vec<_>>().join("\n")))
}

/// What a file is asked with: never a sensitive one or a secret; its first text (the whole text's
/// start when the first pages say little); to the cloud only a paper or deck with enough text.
fn ask_of(e: &shelf_server::index::Entry, ft_dir: &std::path::Path) -> Option<shelf_server::filing_ai::Ask> {
    if e.item.sensitive || lore_catalog::category::secret_name(&e.item.kind, &e.item.name) {
        return None;
    }
    let mut text = e.text.clone();
    if text.trim().chars().count() < 200 {
        if let Some(ft) = lore_catalog::fulltext::FullText::load(ft_dir, &e.item.id) {
            text = ft.joined().chars().take(3000).collect();
        }
    }
    let cloud = e.item.cloud_ok && text.trim().chars().count() >= 200;
    Some(shelf_server::filing_ai::Ask { id: e.item.id.clone(), name: e.item.name.clone(), text, cloud })
}

/// Labels for every catalogued file with something to read (or a project / dataset name), the
/// same two routes as `guess`; the answers also serve the plan.
fn labels(catalog_dir: &std::path::Path, cache_dir: &std::path::Path, count_only: bool) -> Result<String, String> {
    let idx = shelf_server::index::Index::load(catalog_dir, &lore_catalog::extract::Extract::dir(cache_dir));
    let ft_dir = lore_catalog::fulltext::FullText::dir(cache_dir);
    let have = shelf_server::filing_ai::Guesses::load(catalog_dir);
    let asks: Vec<_> = idx
        .entries
        .iter()
        .filter(|e| !have.guesses.contains_key(&e.item.id))
        .filter(|e| matches!(e.item.kind.as_str(), "pdf" | "document" | "slides" | "text" | "latex" | "spreadsheet" | "code" | "project" | "dataset" | "ebook"))
        .filter_map(|e| ask_of(e, &ft_dir))
        .collect();
    let n_cloud = asks.iter().filter(|a| a.cloud).count();
    if count_only {
        return Ok(format!("還沒有標籤的 {}：Claude（只送內容）{n_cloud}、本機 {}", asks.len(), asks.len() - n_cloud));
    }
    let (asked, answered) = ask_models(catalog_dir, &asks)?;
    Ok(format!("這次問了 {asked}，答了 {answered}"))
}

/// Papers and decks to Claude (their text only), the rest to the local model.
fn ask_models(catalog_dir: &std::path::Path, asks: &[shelf_server::filing_ai::Ask]) -> Result<(usize, usize), String> {
    use shelf_server::filing_ai;
    let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
    let llm = shelf_server::llm::Llm::new(home);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let n_cloud = asks.iter().filter(|a| a.cloud).count();
    eprintln!("要問 {}：Claude（只送內容）{n_cloud}、本機 {}", asks.len(), asks.len() - n_cloud);
    rt.block_on(async {
        let cloud = match llm.provider(shelf_server::llm::TASK_FILING_PAPERS).await {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("Claude 不能用（{e}），論文也交給本機模型");
                None
            }
        };
        let local = llm.provider(shelf_server::llm::TASK_FILING_FILES).await?;
        filing_ai::run(catalog_dir, asks, cloud.as_deref(), &*local, |d, t| eprintln!("{d} / {t}")).await
    })
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

/// The words the model called general under the old rule (field basics counted as general) that
/// are not everyday words (author, 2026-11-14: only what an ordinary middle schooler knows): asked
/// again by `classify`, and taken off the shared ignore list meanwhile (only `# LoreShelf` lines).
fn recheck_general(home: &lore_home::Home, cache_dir: &std::path::Path, write: bool) -> Result<String, String> {
    let mut pool = lore_catalog::terms::Pool::load(cache_dir).ok_or("還沒有候選詞；先執行 loreshelf-cli terms")?;
    let path = ignore_file(home).ok_or("找不到 private/LoreReader（從 LoreForge 資料夾裡執行）")?;
    let filters = lore_words::Filters::default();
    let again: std::collections::HashSet<String> = pool.terms.iter().filter(|t| t.verdict.as_deref() == Some("general") && !filters.is_everyday(&t.label)).map(|t| lore_words::phrase_key(&t.label)).collect();
    let cur = std::fs::read_to_string(&path).unwrap_or_default();
    let tag = shelf_server::classify::IGNORE_TAG;
    let kept: Vec<&str> = cur.lines().filter(|l| !(l.contains(tag) && again.contains(&lore_words::phrase_key(l.split('#').next().unwrap_or("").trim())))).collect();
    let removed = cur.lines().count() - kept.len();
    if !write {
        return Ok(format!("要重新判斷的詞 {}（清單上拿掉 {removed} 行）；加 --write 才會改", again.len()));
    }
    for t in pool.terms.iter_mut().filter(|t| t.verdict.as_deref() == Some("general") && again.contains(&lore_words::phrase_key(&t.label))) {
        t.verdict = None;
        t.field = None;
    }
    pool.save(cache_dir).map_err(|e| e.to_string())?;
    store::write_atomic(&path, (kept.join("\n") + "\n").as_bytes()).map_err(|e| e.to_string())?;
    Ok(format!("{} 個詞等 classify 重新判斷；清單拿掉 {removed} 行", again.len()))
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

/// The KnowledgeBase's registered domains and tags (`private/KnowledgeBase/tags.json`; empty
/// when there is none).
fn kb_vocab(home: &lore_home::Home) -> Vocabulary {
    home.app_dir("KnowledgeBase").and_then(|d| Vocabulary::load(&d.join("tags.json")).ok()).unwrap_or_default()
}

/// Where KnowledgeBase writes go: its API while it runs, else kb-core on its folder.
enum KbTarget {
    Api(u16),
    Folder { root: PathBuf, member: String },
}

fn kb_target(home: &lore_home::Home) -> Result<KbTarget, String> {
    if let Some(port) = lore_home::ports::live().get("kb").copied() {
        return Ok(KbTarget::Api(port));
    }
    let root = home.app_dir("KnowledgeBase").filter(|d| d.join("space.toml").is_file()).ok_or("找不到知識庫（private/KnowledgeBase）")?;
    Ok(KbTarget::Folder { root, member: home.prefs().profile.id.unwrap_or_else(|| "me".into()) })
}

/// One call to a KnowledgeBase on `port`: (status, body).
fn kb_api_at(port: u16, method: &str, path: &str, body: Option<&serde_json::Value>) -> Result<(u16, serde_json::Value), String> {
    use std::io::{Read, Write};
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3)).map_err(|e| e.to_string())?;
    let b = body.map(|b| b.to_string()).unwrap_or_default();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len());
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let json = out.split_once("\r\n\r\n").and_then(|(_, b)| serde_json::from_str(b.trim()).ok()).unwrap_or(serde_json::Value::Null);
    Ok((status, json))
}

/// The cards of a KnowledgeBase folder (read only), with their revision.
fn read_cards(nodes: &std::path::Path) -> Vec<(shelf_server::kbtags::Card, u64)> {
    let mut out = Vec::new();
    for f in std::fs::read_dir(nodes).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")) {
        let Some(v) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { continue };
        let strs = |k: &str| -> Vec<String> { v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let (Some(id), Some(title)) = (v["id"].as_str(), v["title"].as_str()) else { continue };
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        out.push((shelf_server::kbtags::Card { id: id.into(), title: title.into(), summary: s("summary"), body: s("body"), domains: strs("domains"), tags: strs("tags"), local_only: false }, v["revision"].as_u64().unwrap_or(0)));
    }
    out.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    out
}

/// Every card's domains and 2–4 tags from the vocabulary (`--vocab`, default the KB's own
/// tags.json), on Claude Haiku (the local model only when Haiku is unavailable). Without `--write` only a list; with it, the vocabulary goes into tags.json and
/// each card is edited through KnowledgeBase (its API with the card's revision, else kb-core).
fn kb_tags(cache_dir: &std::path::Path, write: bool, sample: Option<usize>, ids: &[String], vocab_file: Option<PathBuf>) -> Result<String, String> {
    use shelf_server::kbtags;
    let home = lore_home::Home::discover(&std::env::current_dir().unwrap_or_default());
    let kb_dir = home.app_dir("KnowledgeBase").filter(|d| d.join("nodes").is_dir()).ok_or("找不到知識庫（private/KnowledgeBase）")?;
    let vocab = match &vocab_file {
        Some(p) => Vocabulary::load(p)?,
        None => kb_vocab(&home),
    };
    if vocab.tags.is_empty() {
        return Err("詞彙表沒有標籤（--vocab 指定一個 tags.json，或先在知識庫的 tags.json 寫好領域與標籤）".into());
    }
    let _ = cache_dir;
    let mut cards = read_cards(&kb_dir.join("nodes"));
    if !ids.is_empty() {
        cards.retain(|(c, _)| ids.contains(&c.id));
    } else if let Some(n) = sample.filter(|&n| n > 0 && n < cards.len()) {
        // spread over the list, local-only ones included
        let step = cards.len() as f64 / n as f64;
        cards = (0..n).map(|i| cards[(i as f64 * step) as usize].clone()).collect();
    }
    eprintln!("卡片 {}", cards.len());
    let llm = shelf_server::llm::Llm::new(home.clone());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let list: Vec<kbtags::Card> = cards.iter().map(|(c, _)| c.clone()).collect();
    let r = rt.block_on(async {
        // every card to Claude Haiku (author, 2026-11-14); the local model only when Haiku is not there
        let (cloud, local) = match llm.provider(shelf_server::llm::TASK_CARD_TITLES).await {
            Ok(p) => (Some(p), None),
            Err(e) => {
                eprintln!("Claude 不能用（{e}）：改用本機模型");
                (None, Some(llm.provider(shelf_server::llm::TASK_CARD_LOCAL).await?))
            }
        };
        kbtags::run(&list, &vocab, cloud.as_deref(), local.as_deref(), |d, t| eprintln!("{d} / {t}")).await
    });
    // the local model this started is not left running
    shelf_server::llm::quit();
    let got = r?;
    let missing: Vec<&kbtags::Card> = list.iter().filter(|c| !got.contains_key(&c.id)).collect();
    let mut out = String::new();
    for c in &list {
        let Some(t) = got.get(&c.id) else { continue };
        let doms: Vec<String> = t.domains.iter().map(|d| format!("{}（{d}）", vocab.label(d))).collect();
        out += &format!("{} · {} · {} → {} | {}\n", c.id, if t.by == "cloud" { "Claude" } else { "本機" }, c.title, doms.join("、"), t.tags.join("、"));
    }
    let left = if missing.is_empty() { String::new() } else { format!("；沒有可用答案 {}（{}）", missing.len(), missing.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join("、")) };
    if !write {
        return Ok(format!("{out}有答案 {} 張{left}；加 --write 才會寫進知識庫", got.len()));
    }
    let revs: std::collections::HashMap<&str, u64> = cards.iter().map(|(c, r)| (c.id.as_str(), *r)).collect();
    let writes: Vec<(String, u64, kbtags::Tagged)> = got.iter().map(|(id, t)| (id.clone(), revs.get(id.as_str()).copied().unwrap_or(0), t.clone())).collect();
    let (done, failed) = write_tags(&kb_target(&home)?, &vocab, &writes)?;
    Ok(format!("{out}寫了 {done} 張{}{left}", if failed.is_empty() { String::new() } else { format!("；沒寫成 {}（{}）", failed.len(), failed.join("；")) }))
}

/// The vocabulary into tags.json, then each card's domains and tags (with the revision it was
/// read at: a card edited meanwhile is left alone). Returns (written, failures).
fn write_tags(target: &KbTarget, vocab: &Vocabulary, cards: &[(String, u64, shelf_server::kbtags::Tagged)]) -> Result<(usize, Vec<String>), String> {
    let mut failed = Vec::new();
    let mut done = 0;
    match target {
        KbTarget::Api(port) => {
            match kb_api_at(*port, "PUT", "/api/tags", Some(&vocab.to_json()))? {
                (200, _) => {}
                (s, v) => return Err(format!("tags.json 沒寫成：{s} {}", v["error"].as_str().unwrap_or(""))),
            }
            for (id, rev, t) in cards {
                match kb_api_at(*port, "PUT", &format!("/api/nodes/{id}"), Some(&serde_json::json!({ "revision": rev, "domains": t.domains, "tags": t.tags }))) {
                    Ok((200, _)) => done += 1,
                    Ok((s, v)) => failed.push(format!("{id} {s} {}", v["error"].as_str().unwrap_or(""))),
                    Err(e) => failed.push(format!("{id} {e}")),
                }
            }
        }
        KbTarget::Folder { root, member } => {
            let mut spaces = lore_store::Spaces::new();
            spaces.mount(root).map_err(|e| e.to_string())?;
            let mut kb = kb_core::KnowledgeBase::open(spaces, member, kb_core::KbOptions::default()).map_err(|e| e.to_string())?;
            let space = kb.spaces_info().map_err(|e| e.to_string())?.first().map(|s| s.id.clone()).ok_or("知識庫沒有 space")?;
            let info = |m: &std::collections::BTreeMap<String, String>| m.iter().map(|(k, l)| (k.clone(), kb_core::tags::TagInfo { label: Some(l.clone()), node: None })).collect();
            kb.set_vocabulary(&space, info(&vocab.domains), info(&vocab.tags)).map_err(|e| format!("tags.json 沒寫成：{e}"))?;
            for (id, rev, t) in cards {
                let f = kb_core::NodeFields { domains: Some(t.domains.clone()), tags: Some(t.tags.clone()), ..Default::default() };
                match kb.update(id, f, Some(*rev)) {
                    Ok(_) => done += 1,
                    Err(e) => failed.push(format!("{id} {e}")),
                }
            }
        }
    }
    Ok((done, failed))
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

    fn test_vocab() -> Vocabulary {
        Vocabulary::from_json(&serde_json::json!({
            "domains": { "alpha": { "label": "甲學分析" }, "beta": { "label": "乙學" } },
            "tags": { "alpha/one": { "label": "一" }, "beta/two": { "label": "二" } }
        }))
    }

    fn term(key: &str, domains: &[&str]) -> lore_catalog::terms::Term {
        lore_catalog::terms::Term { key: key.into(), label: key.into(), files: 1, count: 1, score: 0.0, cloud_ok: true, examples: vec![], near: vec![], near_known: 0, domains: domains.iter().map(|d| d.to_string()).collect(), mastered: true, verdict: Some("term".into()), field: None }
    }

    /// LoreShelf's cards only carry registered domains: a free field maps to the one it names.
    #[test]
    fn made_cards_only_get_registered_domains() {
        let titled = |t: &str, field: &str| shelf_server::cards::Titled { keep: true, title: t.into(), field: field.into(), written: true, ..Default::default() };
        let (t1, t2) = (term("dice", &["乙學", "影像評估"]), term("iou", &["alpha"]));
        let (x1, x2) = (titled("Dice", "甲學"), titled("IoU", "影像評估"));
        let cards = merge_cards(&[(&t1, &x1), (&t2, &x2)], &test_vocab());
        let doms: Vec<Vec<String>> = cards.values().map(|c| c.1.clone()).collect();
        assert_eq!(doms, vec![vec!["alpha".to_string(), "beta".to_string()], vec!["alpha".to_string()]]);
        assert!(cards.values().all(|c| c.1.iter().all(|d| test_vocab().domains.contains_key(d))));
    }

    /// kb-tags writes through kb-core when KnowledgeBase is not running: tags.json gets the
    /// vocabulary, cards their domains and tags with their revision (a stale one is left alone).
    #[test]
    fn kb_tags_writes_through_kb_core() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("kb");
        std::fs::create_dir_all(root.join("nodes")).unwrap();
        std::fs::write(root.join("space.toml"), "id = \"personal\"\nname = \"test\"\nkind = \"personal\"\nmembers = [\"ellrin\"]\n").unwrap();
        let mut spaces = lore_store::Spaces::new();
        spaces.mount(&root).unwrap();
        let mut kb = kb_core::KnowledgeBase::open(spaces, "ellrin", kb_core::KbOptions::default()).unwrap();
        let mk = |kb: &mut kb_core::KnowledgeBase, title: &str| kb.create("personal", kb_core::NodeFields { title: Some(title.into()), domains: Some(vec!["自由領域".into()]), ..Default::default() }, None).unwrap();
        let (a, b) = (mk(&mut kb, "Synthetic A"), mk(&mut kb, "Synthetic B"));
        drop(kb);
        let cards = read_cards(&root.join("nodes"));
        assert_eq!(cards.len(), 2);
        let tag = |d: &str, t: &str| shelf_server::kbtags::Tagged { domains: vec![d.into()], tags: vec![t.into()], by: "cloud" };
        let target = KbTarget::Folder { root: root.clone(), member: "ellrin".into() };
        let (done, failed) = write_tags(&target, &test_vocab(), &[(a.id.clone(), a.revision, tag("alpha", "alpha/one")), (b.id.clone(), b.revision + 7, tag("beta", "beta/two"))]).unwrap();
        assert_eq!((done, failed.len()), (1, 1), "{failed:?}");
        let card: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("nodes").join(format!("{}.json", a.id))).unwrap()).unwrap();
        assert_eq!(card["domains"], serde_json::json!(["alpha"]));
        assert_eq!(card["tags"], serde_json::json!(["alpha/one"]));
        assert_eq!(card["revision"], a.revision + 1);
        let tags: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("tags.json")).unwrap()).unwrap();
        assert_eq!(tags["domains"]["alpha"]["label"], "甲學分析");
        assert_eq!(tags["tags"]["beta/two"]["label"], "二");
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
