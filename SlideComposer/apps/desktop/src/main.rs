// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, Wry};

/// .pptx files opened from Finder (double-click, "Open With"): queued until the
/// local server runs, then opened where they live (the deck is bound to the file
/// and saves back into it) and shown.
#[derive(Default)]
struct Opener {
    ready: Mutex<Option<(sc_server::Shared, std::net::SocketAddr)>>,
    pending: Mutex<Vec<PathBuf>>,
    /// `lore-sc://` pages that arrived before the server did.
    pending_pages: Mutex<Vec<String>>,
    addr: Mutex<Option<std::net::SocketAddr>>,
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

/// The page a `lore-sc://` link stands for: `lore-sc://` → `/`, `lore-sc://deck/<name>` → `/?deck=<name>`.
fn link_page(u: &url::Url) -> Option<String> {
    if u.scheme() != "lore-sc" {
        return None;
    }
    match u.host_str().unwrap_or("") {
        "" => Some("/".into()),
        "deck" => {
            let name = decode(u.path().trim_matches('/'));
            (!name.is_empty() && !name.contains('/')).then(|| page(&[("deck", &name)]))
        }
        // a 論文報告 job (PaperComposer's 做成簡報 opens its progress here)
        "job" => {
            let id = decode(u.path().trim_matches('/'));
            sc_server::jobs::valid_id(&id).then(|| page(&[("job", &id)]))
        }
        _ => None,
    }
}

fn open_link(app: &AppHandle, u: url::Url) {
    let Some(pg) = link_page(&u) else {
        eprintln!("not a SlideComposer link: {u}");
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
    let Some((shared, addr)) = st.ready.lock().unwrap().clone() else {
        st.pending.lock().unwrap().push(path);
        return;
    };
    match shared.ws.open_path(&path) {
        Ok(name) => {
            let mut url = url::Url::parse(&format!("http://{addr}/")).expect("url");
            url.query_pairs_mut().append_pair("deck", &name);
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.navigate(url);
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        Err(e) => eprintln!("open {}: {e}", path.display()),
    }
}

/// Title the audience page sets when the show ends (see web/present.js).
const CLOSE_TITLE: &str = "sc:close";

/// The audience window of a slide show: full screen on another display than
/// the main window when there is one (the presenter keeps notes and timer),
/// otherwise a normal window next to it.
fn audience_window(app: &AppHandle, features: NewWindowFeatures) -> NewWindowResponse<Wry> {
    static N: AtomicU32 = AtomicU32::new(0);
    let label = format!("audience-{}", N.fetch_add(1, Ordering::Relaxed));
    let blank = "about:blank".parse().expect("about:blank");
    let mut b = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(blank))
        .window_features(features)
        .title("SlideComposer 放映")
        .inner_size(1280.0, 720.0)
        // a page cannot close a window the app created: it asks by setting this title
        .on_document_title_changed(|w, title| {
            if title == CLOSE_TITLE {
                let _ = w.set_fullscreen(false);
                let _ = w.close();
            }
        });
    let here = app.get_webview_window("main").and_then(|w| w.current_monitor().ok().flatten()).map(|m| *m.position());
    let other = app.available_monitors().ok().and_then(|ms| ms.into_iter().find(|m| Some(*m.position()) != here));
    if let Some(m) = other {
        let (p, k) = (m.position(), m.scale_factor());
        b = b.position(p.x as f64 / k, p.y as f64 / k).fullscreen(true);
    }
    match b.build() {
        Ok(window) => NewWindowResponse::Create { window },
        Err(e) => {
            eprintln!("audience window: {e}");
            NewWindowResponse::Deny
        }
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // Bundled app: web/ ships as a resource. `cargo run`: the repository folder.
            let web = app
                .path()
                .resource_dir()
                .ok()
                .map(|dir| dir.join("web"))
                .filter(|w| w.join("index.html").exists())
                .unwrap_or_else(|| sc_server::repo_dir().join("web"));
            let shared = sc_server::state(sc_server::default_workspace(), web);
            // native open/save dialogs for 開啟… / 另存新檔… (the page has no Tauri IPC:
            // it asks the local server, which calls this)
            let dlg_app = app.handle().clone();
            let _ = shared.dialog.set(std::sync::Arc::new(move |req: sc_server::DialogReq| {
                use tauri_plugin_dialog::DialogExt;
                let d = dlg_app.dialog().file().add_filter("PowerPoint", &["pptx"]);
                let picked = match req.kind.as_str() {
                    "save" => {
                        let name = req.name.unwrap_or_else(|| "簡報".into());
                        let name = if name.to_lowercase().ends_with(".pptx") { name } else { format!("{name}.pptx") };
                        d.set_file_name(name).blocking_save_file()
                    }
                    _ => d.blocking_pick_file(),
                };
                picked.and_then(|p| p.into_path().ok())
            }));
            let for_opener = shared.clone();

            // The server runs on its own runtime thread for the life of the app.
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(async move {
                    match sc_server::bind(shared, 0).await {
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
            let handle = app.handle().clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("SlideComposer")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1000.0, 640.0)
                // the slide show's audience window (window.open from 簡報者檢視)
                .on_new_window(move |_url, features| audience_window(&handle, features))
                .build()?;
            // the first launch asks every permission the app needs, once, in the background
            // (answered ones are never asked again; later changes: 設定 → 一般 → 權限)
            lore_shell::permissions::ask_at_launch();
            // files Finder handed over while starting up
            lore_home::ports::register("sc", addr.port());
            *app.state::<Opener>().addr.lock().unwrap() = Some(addr);
            *app.state::<Opener>().ready.lock().unwrap() = Some((for_opener, addr));
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
        .plugin(tauri_plugin_dialog::init())
        .manage(Opener::default())
        .build(tauri::generate_context!())
        .expect("error while building SlideComposer")
        .run(|app, event| {
            // quitting inside the autosave delay: write pending decks to their files
            if let RunEvent::Exit = event {
                if let Some((shared, _)) = app.state::<Opener>().ready.lock().unwrap().clone() {
                    sc_server::flush_files(&shared);
                }
                sc_server::llm::quit();
                return;
            }
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let RunEvent::Opened { urls } = event {
                for u in urls {
                    if u.scheme() == "lore-sc" {
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
        assert!(plist.contains("CFBundleURLTypes") && plist.contains("<string>lore-sc</string>"));
    }

    #[test]
    fn links_map_to_pages() {
        assert_eq!(p("lore-sc://"), Some("/".into()));
        assert_eq!(p("lore-sc://deck/My%20deck"), Some("/?deck=My+deck".into()));
        assert_eq!(p("lore-sc://deck/%E7%B0%A1%E5%A0%B1"), Some("/?deck=%E7%B0%A1%E5%A0%B1".into()));
        assert_eq!(p("lore-sc://deck/"), None);
        assert_eq!(p("lore-sc://deck/a%2Fb"), None);
        assert_eq!(p("lore-sc://job/j18f3a2b4c5d1"), Some("/?job=j18f3a2b4c5d1".into()));
        assert_eq!(p("lore-sc://job/..%2Fx"), None);
        assert_eq!(p("lore-sc://nope"), None);
        assert_eq!(p("lore-pc://deck/x"), None);
    }
}
