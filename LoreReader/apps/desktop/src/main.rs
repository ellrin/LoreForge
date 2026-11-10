// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running LoreReader");
}
