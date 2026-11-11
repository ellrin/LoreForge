// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};

/// PDFs opened from Finder (double-click, "Open With", LoreReader as the default PDF app):
/// queued until the server answers, then added to the library through `/api/papers/import`
/// (the same call whether this app runs the server or reuses one on 7890) and shown.
#[derive(Default)]
struct Opener {
    addr: Mutex<Option<SocketAddr>>,
    pending: Mutex<Vec<PathBuf>>,
    /// `lore-lr://` pages that arrived before the server did.
    pending_pages: Mutex<Vec<String>>,
}

/// `v=<segment>` decoded the way a URL path segment is.
fn decode(seg: &str) -> String {
    url::form_urlencoded::parse(format!("v={}", seg.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
}

/// `/?k=v&…` with the values percent-encoded.
fn page(pairs: &[(&str, &str)]) -> String {
    let mut u = url::Url::parse("http://x/").expect("url");
    u.query_pairs_mut().extend_pairs(pairs);
    format!("/?{}", u.query().unwrap_or_default())
}

/// The page a `lore-lr://` link stands for: `lore-lr://` → `/`,
/// `lore-lr://paper/<sha256>?page=<n>` → `/?paper=<sha256>&page=<n>`.
fn link_page(u: &url::Url) -> Option<String> {
    if u.scheme() != "lore-lr" {
        return None;
    }
    match u.host_str().unwrap_or("") {
        "" => Some("/".into()),
        "paper" => {
            let sha = decode(u.path().trim_matches('/'));
            if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            match u.query_pairs().find(|(k, _)| k == "page").and_then(|(_, v)| v.parse::<u32>().ok()).filter(|n| *n > 0) {
                Some(n) => Some(page(&[("paper", &sha), ("page", &n.to_string())])),
                None => Some(page(&[("paper", &sha)])),
            }
        }
        _ => None,
    }
}

fn open_link(app: &AppHandle, u: url::Url) {
    let Some(pg) = link_page(&u) else {
        eprintln!("not a LoreReader link: {u}");
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

/// A JSON request to the server at `addr` (in-process or reused); the JSON reply.
fn http_json(addr: SocketAddr, method: &str, path: &str, body: Option<&serde_json::Value>) -> Result<serde_json::Value, String> {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(120)));
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).map_err(|e| e.to_string())?;
    let (head, json) = resp.split_once("\r\n\r\n").ok_or("empty response")?;
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("{e}: {json}"))?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(v["error"].as_str().unwrap_or(head).to_string());
    }
    Ok(v)
}

/// POST `{"path": …}` to the import endpoint; the paper's sha256.
fn import_pdf(addr: SocketAddr, path: &Path) -> Result<String, String> {
    let v = http_json(addr, "POST", "/api/papers/import", Some(&serde_json::json!({ "path": path.to_string_lossy() })))?;
    v["paper"]["sha256"].as_str().map(str::to_string).ok_or_else(|| "no sha256 in reply".into())
}

/// The native save panel (also behind the page's "另存新檔（含筆記）" via `lr_server::Saver`).
fn save_panel(app: &AppHandle, dir: &Path, name: &str) -> Option<PathBuf> {
    app.dialog().file().set_title("另存新檔（含筆記）").set_directory(dir).set_file_name(name).add_filter("PDF", &["pdf"]).blocking_save_file().and_then(|p| p.into_path().ok())
}

struct NativeSaver(AppHandle);

impl lr_server::Saver for NativeSaver {
    fn choose(&self, dir: &Path, name: &str) -> Result<Option<PathBuf>, String> {
        Ok(save_panel(&self.0, dir, name))
    }
}

/// Close / quit with notes that are not saved in a PDF yet: one prompt, for the paper that is
/// open in the window (the page asks the same when the author leaves a paper), with
/// 「儲存…」(the save panel; straight to the last export when there is one) / 「不儲存」 / 「取消」.
#[derive(Default)]
struct CloseGuard {
    /// The author already decided: let the close / exit through.
    decided: AtomicBool,
    /// A prompt is showing (a second Cmd+Q does not open another).
    asking: AtomicBool,
}

/// The paper open in the main window: `paper=<sha256>` of its URL (the page keeps it there).
fn open_paper(app: &AppHandle) -> Option<String> {
    let url = app.get_webview_window("main")?.url().ok()?;
    url.query_pairs().find(|(k, _)| k == "paper").map(|(_, v)| v.to_string()).filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Runs off the main thread (blocking dialogs). `true` = go ahead and close.
fn confirm_close(app: &AppHandle) -> bool {
    let Some(addr) = *app.state::<Opener>().addr.lock().unwrap() else { return true };
    let Some(sha) = open_paper(app) else { return true };
    let st = match http_json(addr, "GET", &format!("/api/papers/{sha}/export"), None) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("dirty check: {e}");
            return true;
        }
    };
    if st["dirty"] != true {
        return true;
    }
    let title = http_json(addr, "GET", &format!("/api/papers/{sha}"), None).ok().and_then(|v| v["paper"]["title"].as_str().map(str::to_string)).unwrap_or_default();
    let parent = app.get_webview_window("main");
    let last = st["last"]["path"].as_str().filter(|_| st["last"]["exists"] == true).map(PathBuf::from);
    let (msg, yes) = match &last {
        Some(l) => (format!("《{title}》\n→ {}", l.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()), "儲存"),
        None => (format!("《{title}》"), "儲存…"),
    };
    let mut d = app.dialog().message(msg).title("儲存筆記到新的 PDF？").kind(MessageDialogKind::Warning).buttons(MessageDialogButtons::YesNoCancelCustom(yes.into(), "不儲存".into(), "取消".into()));
    if let Some(w) = &parent {
        d = d.parent(w);
    }
    match d.blocking_show_with_result() {
        MessageDialogResult::Custom(b) if b == yes => {
            let target = match last {
                Some(l) => l,
                None => {
                    let dir = PathBuf::from(st["default_dir"].as_str().unwrap_or_default());
                    match save_panel(app, &dir, st["default_name"].as_str().unwrap_or("notes.pdf")) {
                        Some(t) => t,
                        None => return false,
                    }
                }
            };
            if let Err(e) = http_json(addr, "POST", &format!("/api/papers/{sha}/export"), Some(&serde_json::json!({ "path": target }))) {
                app.dialog().message(e).title("沒有存成").kind(MessageDialogKind::Error).blocking_show();
                return false;
            }
            true
        }
        MessageDialogResult::Custom(b) if b == "不儲存" => {
            if let Err(e) = http_json(addr, "POST", &format!("/api/papers/{sha}/export/decline"), Some(&serde_json::json!({}))) {
                eprintln!("decline: {e}");
            }
            true
        }
        _ => false,
    }
}

/// Ask (on a worker thread), then `then` on the main thread when the author lets it close.
fn guarded(app: &AppHandle, then: impl FnOnce(&AppHandle) + Send + 'static) {
    let g = app.state::<CloseGuard>();
    if g.asking.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let go = confirm_close(&app);
        let g = app.state::<CloseGuard>();
        g.asking.store(false, Ordering::SeqCst);
        if go {
            g.decided.store(true, Ordering::SeqCst);
            let a = app.clone();
            let _ = app.run_on_main_thread(move || then(&a));
        }
    });
}

/// The default menu with its Quit item replaced by ours: the predefined one calls
/// `terminate:` directly, which no Tauri event can stop.
fn app_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;
    #[cfg(target_os = "macos")]
    if let Some(first) = menu.items()?.into_iter().next()
        && let Some(sub) = first.as_submenu()
    {
        let n = sub.items()?.len();
        if n > 0 {
            sub.remove_at(n - 1)?;
        }
        sub.append(&MenuItem::with_id(app, "lr-quit", "Quit LoreReader", true, Some("CmdOrCtrl+Q"))?)?;
    }
    Ok(menu)
}

fn open_file(app: &AppHandle, path: PathBuf) {
    let st = app.state::<Opener>();
    let Some(addr) = *st.addr.lock().unwrap() else {
        st.pending.lock().unwrap().push(path);
        return;
    };
    let app = app.clone();
    // off the main thread: a large PDF takes a moment to hash and copy
    std::thread::spawn(move || match import_pdf(addr, &path) {
        Ok(sha) => {
            let url = url::Url::parse(&format!("http://{addr}/?paper={sha}&page=1")).expect("url");
            let a = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(w) = a.get_webview_window("main") {
                    let _ = w.navigate(url);
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            });
        }
        Err(e) => eprintln!("open {}: {e}", path.display()),
    });
}

/// An lr-server already answers on `addr` (e.g. `cargo run -p lr-server`).
fn lr_server_at(addr: SocketAddr) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /api/info HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut body = String::new();
    let _ = s.read_to_string(&mut body);
    body.starts_with("HTTP/1.1 200") && body.contains("\"library\"")
}

/// Printing: print.html opens in its own window; when it has rendered every page at
/// print resolution it calls /api/print/ready and the webview's native print dialog
/// (macOS: the system dialog, any installed printer) is shown on that window.
struct NativePrinter(AppHandle);

impl lr_server::Printer for NativePrinter {
    fn open(&self, url: &str, id: &str) -> Result<(), String> {
        let app = self.0.clone();
        let url = url::Url::parse(url).map_err(|e| e.to_string())?;
        let label = format!("print-{id}");
        self.0
            .run_on_main_thread(move || {
                if let Err(e) = WebviewWindowBuilder::new(&app, label, WebviewUrl::External(url)).title("列印 · LoreReader").inner_size(820.0, 960.0).build() {
                    eprintln!("print window: {e}");
                }
            })
            .map_err(|e| e.to_string())
    }

    fn ready(&self, id: &str) -> Result<(), String> {
        let app = self.0.clone();
        let label = format!("print-{id}");
        self.0
            .run_on_main_thread(move || match app.get_webview_window(&label) {
                Some(w) => {
                    if let Err(e) = w.print() {
                        eprintln!("print: {e}");
                    }
                }
                None => eprintln!("print: no window {label}"),
            })
            .map_err(|e| e.to_string())
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let config = lr_core::Config::load().map_err(|e| e.to_string())?;
            // Bundled app: web/ ships as a resource. `cargo run`: the repository folder.
            let web = app
                .path()
                .resource_dir()
                .ok()
                .map(|dir| dir.join("web"))
                .filter(|w| w.join("index.html").exists())
                .unwrap_or_else(|| lr_server::repo_dir().join("web"));
            let wanted = SocketAddr::from(([127, 0, 0, 1], lr_core::config::DEFAULT_PORT));

            let addr = if lr_server_at(wanted) {
                // Printing then uses that server's browser fallback (no native dialog).
                wanted
            } else {
                let shared = lr_server::state(config, lr_server::default_workspace(), web, false);
                *shared.printer.lock().unwrap() = Some(Arc::new(NativePrinter(app.handle().clone())));
                *shared.saver.lock().unwrap() = Some(Arc::new(NativeSaver(app.handle().clone())));
                // The server runs on its own runtime thread for the life of the app.
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                    rt.block_on(async move {
                        // 7890 is what KnowledgeBase links to; another port only if it is taken.
                        let bound = match lr_server::bind(shared.clone(), lr_core::config::DEFAULT_PORT).await {
                            Ok(b) => Ok(b),
                            Err(e) => {
                                eprintln!("port {}: {e}; using a free port (links from KnowledgeBase will not reach this window)", lr_core::config::DEFAULT_PORT);
                                lr_server::bind(shared, 0).await
                            }
                        };
                        match bound {
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
                rx.recv()??
            };

            let url = url::Url::parse(&format!("http://{addr}/"))?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("LoreReader")
                .inner_size(1440.0, 900.0)
                .min_inner_size(900.0, 600.0)
                .build()?;
            // files Finder handed over while starting up
            lore_home::ports::register("lr", addr.port());
            *app.state::<Opener>().addr.lock().unwrap() = Some(addr);
            let pending: Vec<PathBuf> = std::mem::take(&mut *app.state::<Opener>().pending.lock().unwrap());
            for p in pending {
                open_file(app.handle(), p);
            }
            let pages: Vec<String> = std::mem::take(&mut *app.state::<Opener>().pending_pages.lock().unwrap());
            if let Some(pg) = pages.into_iter().last() {
                navigate_page(app.handle(), addr, &pg);
            }
            Ok(())
        })
        .manage(Opener::default())
        .manage(CloseGuard::default())
        .plugin(tauri_plugin_dialog::init())
        .menu(app_menu)
        .on_menu_event(|app, e| {
            if e.id() == "lr-quit" {
                guarded(app, |a| a.exit(0));
            }
        })
        .on_window_event(|w, e| {
            if w.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = e
                && !w.app_handle().state::<CloseGuard>().decided.load(Ordering::SeqCst)
            {
                api.prevent_close();
                let label = w.label().to_string();
                guarded(w.app_handle(), move |a| {
                    if let Some(w) = a.get_webview_window(&label) {
                        let _ = w.destroy();
                    }
                });
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building LoreReader")
        .run(|app, event| {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let RunEvent::Opened { urls } = event {
                for u in urls {
                    if u.scheme() == "lore-lr" {
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
    fn info_plist_registers_the_scheme() {
        let plist = include_str!("../Info.plist");
        assert!(plist.contains("CFBundleURLTypes") && plist.contains("<string>lore-lr</string>"));
    }

    #[test]
    fn links_map_to_pages() {
        let sha = "ab".repeat(32);
        assert_eq!(p("lore-lr://"), Some("/".into()));
        assert_eq!(p(&format!("lore-lr://paper/{sha}?page=3")), Some(format!("/?paper={sha}&page=3")));
        assert_eq!(p(&format!("lore-lr://paper/{sha}")), Some(format!("/?paper={sha}")));
        assert_eq!(p(&format!("lore-lr://paper/{sha}?page=0")), Some(format!("/?paper={sha}")));
        assert_eq!(p("lore-lr://paper/short?page=3"), None);
        assert_eq!(p("lore-lr://paper/"), None);
        assert_eq!(p("lore-kb://paper/x"), None);
    }
}
