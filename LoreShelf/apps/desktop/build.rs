fn main() {
    // App commands the remote page (127.0.0.1) may call: listed here, allowed in capabilities/default.json.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["hide", "frame"]))).expect("tauri build");
}
