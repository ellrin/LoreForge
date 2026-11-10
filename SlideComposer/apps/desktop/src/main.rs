// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, Wry};

/// .pptx files opened from Finder (double-click, "Open With"): queued until the
/// local server runs, then imported (or the deck already made from them) and shown.
#[derive(Default)]
struct Opener {
    ready: Mutex<Option<(sc_server::Shared, std::net::SocketAddr)>>,
    pending: Mutex<Vec<PathBuf>>,
}

fn open_file(app: &AppHandle, path: PathBuf) {
    let st = app.state::<Opener>();
    let Some((shared, addr)) = st.ready.lock().unwrap().clone() else {
        st.pending.lock().unwrap().push(path);
        return;
    };
    match shared.ws.open_pptx_file(&path) {
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
            // files Finder handed over while starting up
            *app.state::<Opener>().ready.lock().unwrap() = Some((for_opener, addr));
            let pending: Vec<PathBuf> = std::mem::take(&mut *app.state::<Opener>().pending.lock().unwrap());
            for p in pending {
                open_file(app.handle(), p);
            }
            Ok(())
        })
        .manage(Opener::default())
        .build(tauri::generate_context!())
        .expect("error while building SlideComposer")
        .run(|app, event| {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let RunEvent::Opened { urls } = event {
                for u in urls {
                    if let Ok(p) = u.to_file_path() {
                        open_file(app, p);
                    }
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            let _ = (app, event);
        });
}
