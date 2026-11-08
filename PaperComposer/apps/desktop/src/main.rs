// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

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
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building PaperComposer")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                // The local model server must not outlive the app.
                pc_server::models::kill_runtime();
            }
        });
}
