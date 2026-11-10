// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// PDFs opened from Finder (double-click, "Open With", LoreReader as the default PDF app):
/// queued until the server answers, then added to the library through `/api/papers/import`
/// (the same call whether this app runs the server or reuses one on 7890) and shown.
#[derive(Default)]
struct Opener {
    addr: Mutex<Option<SocketAddr>>,
    pending: Mutex<Vec<PathBuf>>,
}

/// POST `{"path": …}` to the import endpoint; the paper's sha256.
fn import_pdf(addr: SocketAddr, path: &Path) -> Result<String, String> {
    let body = serde_json::json!({ "path": path.to_string_lossy() }).to_string();
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(120)));
    let req = format!(
        "POST /api/papers/import HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
    v["paper"]["sha256"].as_str().map(str::to_string).ok_or_else(|| "no sha256 in reply".into())
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
            *app.state::<Opener>().addr.lock().unwrap() = Some(addr);
            let pending: Vec<PathBuf> = std::mem::take(&mut *app.state::<Opener>().pending.lock().unwrap());
            for p in pending {
                open_file(app.handle(), p);
            }
            Ok(())
        })
        .manage(Opener::default())
        .build(tauri::generate_context!())
        .expect("error while building LoreReader")
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
