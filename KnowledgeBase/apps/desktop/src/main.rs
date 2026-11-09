// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// A kb-server already answers on `addr` (e.g. started at login by launchd).
fn kb_server_at(addr: SocketAddr) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let req = format!("GET /api/health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut body = String::new();
    let _ = s.read_to_string(&mut body);
    body.starts_with("HTTP/1.1 200") && body.contains("\"member\"")
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let mut config = kb_server::Config::load(&[])?;
            // Bundled app: web/ ships as a resource. `cargo run`: the repository folder.
            if let Some(web) = app.path().resource_dir().ok().map(|d| d.join("web")).filter(|w| w.join("index.html").exists()) {
                config.web = web;
            }
            let wanted = SocketAddr::from(([127, 0, 0, 1], config.port));

            let addr = if kb_server_at(wanted) {
                wanted
            } else {
                let kb = kb_server::open(&config, kb_core::KbOptions::default())?;
                // The server (and the reminder scheduler) runs on its own runtime thread.
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                    rt.block_on(async move {
                        match kb_server::bind(config, kb).await {
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
                .title("KnowledgeBase")
                .inner_size(1360.0, 880.0)
                .min_inner_size(900.0, 600.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running KnowledgeBase");
}
