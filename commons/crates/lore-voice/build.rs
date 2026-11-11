//! On macOS: compile `swift/helper.swift` into `OUT_DIR/lore-voice-helper` with `swiftc`
//! (an Info.plist with the two usage strings is embedded, for when it runs on its own).
//! Elsewhere, or without swiftc: no helper (the crate then reports voice as unavailable;
//! never a build failure).

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=swift/helper.swift");
    println!("cargo:rerun-if-changed=swift/Info.plist");
    println!("cargo:rerun-if-env-changed=LORE_VOICE_NO_HELPER");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") || std::env::var_os("LORE_VOICE_NO_HELPER").is_some() {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("lore-voice-helper");
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x86_64",
        _ => "arm64",
    };
    let status = Command::new("swiftc")
        .args(["-O", "-swift-version", "5", "-target", &format!("{arch}-apple-macos14.0")])
        .args(["-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist", "-Xlinker", "swift/Info.plist"])
        .arg("swift/helper.swift")
        .arg("-o")
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-env=LORE_VOICE_HELPER_BUILT={}", out.display());
            println!("cargo:helper={}", out.display());
        }
        Ok(s) => println!("cargo:warning=lore-voice: swiftc failed ({s}); voice is unavailable in this build"),
        Err(e) => println!("cargo:warning=lore-voice: no swiftc ({e}); voice is unavailable in this build"),
    }
}
