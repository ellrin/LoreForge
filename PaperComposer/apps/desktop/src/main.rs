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
            *app.state::<Opener>().addr.lock().unwrap() = Some(addr);
            let pending: Vec<PathBuf> = std::mem::take(&mut *app.state::<Opener>().pending.lock().unwrap());
            // one page at a time: the last file wins
            if let Some(p) = pending.into_iter().last() {
                open_file(app.handle(), p);
            }
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .manage(Opener::default())
        .build(tauri::generate_context!())
        .expect("error while building PaperComposer")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                // The local model server must not outlive the app.
                pc_server::models::kill_runtime();
                return;
            }
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
