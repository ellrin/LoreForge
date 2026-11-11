// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// Files Finder hands over (Open With: a .docx or a .tex): queued until the
/// local server runs, then given to the page (`?open=<path>`), which starts
/// 開啟 Word… or 開啟 LaTeX… on them.
#[derive(Default)]
struct Opener {
    addr: Mutex<Option<std::net::SocketAddr>>,
    pending: Mutex<Vec<PathBuf>>,
    /// `lore-pc://` pages that arrived before the server did.
    pending_pages: Mutex<Vec<String>>,
}

/// `/?k=v&…` with the values percent-encoded.
fn page(pairs: &[(&str, &str)]) -> String {
    let mut u = url::Url::parse("http://x/").expect("url");
    u.query_pairs_mut().extend_pairs(pairs);
    format!("/?{}", u.query().unwrap_or_default())
}

/// The page a `lore-pc://` link stands for: `lore-pc://` → `/`, `lore-pc://open?path=<file>` → `/?open=<file>`,
/// `lore-pc://note/<id>` → `/?note=<id>` (the 筆記 window on that note).
fn link_page(u: &url::Url) -> Option<String> {
    if u.scheme() != "lore-pc" {
        return None;
    }
    match u.host_str().unwrap_or("") {
        "" => Some("/".into()),
        "open" => {
            let path = u.query_pairs().find(|(k, _)| k == "path").map(|(_, v)| v.into_owned()).filter(|v| !v.is_empty())?;
            Some(page(&[("open", &path)]))
        }
        // 筆記 (LoreEcho's 「打開」): `lore-pc://note/<id>` → `/?note=<id>`; no id = the list
        "note" => {
            let seg = u.path_segments().and_then(|mut s| s.next()).unwrap_or("");
            let id: String = url::form_urlencoded::parse(format!("n={}", seg.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default();
            Some(page(&[("note", &id)]))
        }
        _ => None,
    }
}

fn open_link(app: &AppHandle, u: url::Url) {
    let Some(pg) = link_page(&u) else {
        eprintln!("not a PaperComposer link: {u}");
        return;
    };
    let st = app.state::<Opener>();
    let Some(addr) = *st.addr.lock().unwrap() else {
        st.pending_pages.lock().unwrap().push(pg);
        return;
    };
    navigate_page(app, addr, &pg);
}

fn navigate_page(app: &AppHandle, addr: std::net::SocketAddr, pg: &str) {
    if let Ok(target) = url::Url::parse(&format!("http://{addr}{pg}"))
        && let Some(w) = app.get_webview_window("main")
    {
        let _ = w.navigate(target);
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn open_file(app: &AppHandle, path: PathBuf) {
    let st = app.state::<Opener>();
    let Some(addr) = *st.addr.lock().unwrap() else {
        st.pending.lock().unwrap().push(path);
        return;
    };
    let mut url = url::Url::parse(&format!("http://{addr}/")).expect("url");
    url.query_pairs_mut().append_pair("open", &path.to_string_lossy());
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.navigate(url);
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // Bundled app: ui/ and workspace-template/ ship as resources.
            // `cargo run` / `tauri dev`: fall back to the repository folders.
            let paths = app
                .path()
                .resource_dir()
                .ok()
                .map(|dir| pc_server::Paths {
                    ui: dir.join("ui"),
                    demo_workspace: dir.join("workspace-template"),
                })
                .filter(|p| p.ui.join("index.html").exists())
                .unwrap_or_else(pc_server::Paths::from_repo);

            // native dialogs for 開啟 LaTeX… / 開啟 Word… (the page has no Tauri
            // IPC: it asks the local server, which calls this)
            let dlg = app.handle().clone();
            let _ = pc_server::opened::DIALOG.set(std::sync::Arc::new(move |req: pc_server::opened::DialogReq| {
                use tauri_plugin_dialog::DialogExt;
                let mut d = dlg.dialog().file();
                if let Some(start) = req.start.as_ref().filter(|s| s.is_dir()) {
                    d = d.set_directory(start);
                }
                let picked = match req.kind.as_str() {
                    // a folder to put a new folder in
                    "folder" => d.set_can_create_directories(true).blocking_pick_folder(),
                    "word" => d.add_filter("Word", &["docx"]).blocking_pick_file(),
                    // a project's .tex (its folder opens) or an archive
                    _ => d.add_filter("LaTeX", &["tex", "zip", "gz", "tgz", "tar"]).blocking_pick_file(),
                };
                picked.and_then(|p| p.into_path().ok())
            }));

            // The server runs on its own runtime thread for the life of the app.
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(async move {
                    match pc_server::bind(paths, 0).await {
                        Ok((addr, serve)) => {
                            let _ = tx.send(Ok(addr));
                            if let Err(e) = serve.await {
                                eprintln!("server stopped: {e}");
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                        }
                    }
                });
            });
            let addr = rx.recv()??;

            let url = url::Url::parse(&format!("http://{addr}/"))?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("PaperComposer")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1000.0, 640.0)
                .build()?;
            // files Finder handed over while starting up
            lore_home::ports::register("pc", addr.port());
            *app.state::<Opener>().addr.lock().unwrap() = Some(addr);
            let pending: Vec<PathBuf> = std::mem::take(&mut *app.state::<Opener>().pending.lock().unwrap());
            // one page at a time: the last file wins
            if let Some(p) = pending.into_iter().last() {
                open_file(app.handle(), p);
            }
            let pages: Vec<String> = std::mem::take(&mut *app.state::<Opener>().pending_pages.lock().unwrap());
            if let Some(pg) = pages.into_iter().last() {
                navigate_page(app.handle(), addr, &pg);
            }
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .manage(Opener::default())
        .build(tauri::generate_context!())
        .expect("error while building PaperComposer")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                // this app's hold on the shared local model goes (the last user stops the server)
                pc_server::models::quit();
                return;
            }
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let RunEvent::Opened { urls } = event {
                for u in urls {
                    if u.scheme() == "lore-pc" {
                        open_link(app, u);
                    } else if let Ok(p) = u.to_file_path() {
                        open_file(app, p);
                    }
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Option<String> {
        link_page(&url::Url::parse(s).unwrap())
    }

    #[test]
    fn links_map_to_pages() {
        assert_eq!(p("lore-pc://"), Some("/".into()));
        assert_eq!(p("lore-pc://open?path=%2Ftmp%2Fa%20b%2Fmain.tex"), Some("/?open=%2Ftmp%2Fa+b%2Fmain.tex".into()));
        assert_eq!(p("lore-pc://open"), None);
        assert_eq!(p("lore-pc://open?path="), None);
        assert_eq!(p("lore-pc://other"), None);
        assert_eq!(p("lore-pc://note/2026-11-12-loss-weights"), Some("/?note=2026-11-12-loss-weights".into()));
        // LoreLinks encodes the id (encodeURIComponent): a CJK slug comes back as the same id
        let page = p("lore-pc://note/2026-11-12-%E6%A2%AF%E5%BA%A6").unwrap();
        assert_eq!(url::Url::parse(&format!("http://x{page}")).unwrap().query_pairs().next().unwrap().1, "2026-11-12-梯度");
        assert_eq!(p("lore-pc://note"), Some("/?note=".into()));
        assert_eq!(p("lore-sc://deck/x"), None);
    }

    #[test]
    fn info_plist_registers_the_scheme() {
        let plist = include_str!("../Info.plist");
        assert!(plist.contains("CFBundleURLTypes") && plist.contains("<string>lore-pc</string>"));
    }
}
