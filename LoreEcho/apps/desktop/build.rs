fn main() {
    // The voice helper lore-voice built (macOS): copied to bin/ so the bundler puts it into
    // LoreEcho.app/Contents/MacOS (tauri.conf.json bundle.macOS.files). Elsewhere: nothing.
    if let Some(src) = std::env::var_os("DEP_LORE_VOICE_HELPER") {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("bin");
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = std::fs::copy(&src, dir.join("lore-voice-helper")) {
            println!("cargo:warning=loreecho: lore-voice-helper not copied: {e}");
        }
    }
    tauri_build::build()
}
