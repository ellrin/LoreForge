// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

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
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("SlideComposer")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1000.0, 640.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running SlideComposer");
}
