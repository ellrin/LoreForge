// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// `lore-kb://node/<id>` links (NeonDeck's card widget, other apps): queued until the
/// window exists, then the window goes to that card. macOS starts the app for them.
#[derive(Default)]
struct Links {
    addr: Mutex<Option<SocketAddr>>,
    pending: Mutex<Vec<url::Url>>,
}

/// The page hash a `lore-kb://` link stands for: `node/<id>` → `#/node/<id>`.
fn link_hash(u: &url::Url) -> Option<String> {
    let id = u.path().trim_matches('/');
    let ok = u.host_str() == Some("node") && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| format!("#/node/{id}"))
}

fn open_link(app: &AppHandle, u: url::Url) {
    let st = app.state::<Links>();
    let Some(addr) = *st.addr.lock().unwrap() else {
        st.pending.lock().unwrap().push(u);
        return;
    };
    let Some(hash) = link_hash(&u) else {
        eprintln!("not a card link: {u}");
        return;
    };
    let Ok(target) = url::Url::parse(&format!("http://{addr}/{hash}")) else { return };
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.navigate(target);
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

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

            lore_home::ports::register("kb", addr.port());
            let url = url::Url::parse(&format!("http://{addr}/"))?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("KnowledgeBase")
                .inner_size(1360.0, 880.0)
                .min_inner_size(900.0, 600.0)
                .build()?;
            // links macOS handed over while starting up
            *app.state::<Links>().addr.lock().unwrap() = Some(addr);
            let pending: Vec<url::Url> = std::mem::take(&mut *app.state::<Links>().pending.lock().unwrap());
            for u in pending {
                open_link(app.handle(), u);
            }
            Ok(())
        })
        .manage(Links::default())
        .build(tauri::generate_context!())
        .expect("error while building KnowledgeBase")
        .run(|app, event| {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let RunEvent::Opened { urls } = event {
                for u in urls.into_iter().filter(|u| u.scheme() == "lore-kb") {
                    open_link(app, u);
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_hash_and_plist() {
        let h = |s: &str| link_hash(&url::Url::parse(s).unwrap());
        assert_eq!(h("lore-kb://node/abc-1"), Some("#/node/abc-1".into()));
        assert_eq!(h("lore-kb://node/"), None);
        assert_eq!(h("lore-kb://other/x"), None);
        let plist = include_str!("../Info.plist");
        assert!(plist.contains("CFBundleURLTypes") && plist.contains("<string>lore-kb</string>"));
    }
}
