fn main() {
    // The voice helper lore-voice built (macOS, for 潤飾's hold-to-talk): copied to bin/ so the
    // bundler puts it into the .app's Contents/MacOS (tauri.conf.json bundle.macOS.files).
    // Elsewhere, or without swiftc: nothing, and the page types only.
    if let Some(src) = std::env::var_os("DEP_LORE_VOICE_HELPER") {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("bin");
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = std::fs::copy(&src, dir.join("lore-voice-helper")) {
            println!("cargo:warning=lore-voice-helper not copied: {e}");
        }
    }
    tauri_build::build()
}
