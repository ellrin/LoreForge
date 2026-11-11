//! A check of the Apple engines on a real Mac, without the microphone and without playing sound:
//! list the voices, synthesise a sentence to a file, transcribe that file back.
//!
//! ```sh
//! cargo run -p lore-voice --example check_mac -- <empty folder>
//! ```

use lore_voice::helper::HelperBackend;
use lore_voice::{Backend, SpeakRequest, sort_voices};

fn main() {
    let dir = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let Some(b) = HelperBackend::find() else {
        eprintln!("no lore-voice-helper (not macOS, or swiftc missing at build time)");
        std::process::exit(1);
    };
    let mut voices = b.voices().expect("voices");
    sort_voices(&mut voices, "zh-TW");
    println!("{} voices; first zh-TW:", voices.len());
    for v in voices.iter().filter(|v| v.language == "zh-TW").take(5) {
        println!("  {} {} {:?}", v.id, v.name, v.quality);
    }
    match b.permissions() {
        Ok((mic, speech)) => println!("permissions: microphone {mic}, speech {speech}"),
        Err(e) => println!("permissions: {e}"),
    }
    let text = "今天我們來複習一下微積分的基本定理。";
    let path = dir.join("lore-voice-check.caf");
    let frames = b.synth_file(&SpeakRequest { text: text.into(), voice: String::new(), rate: 1.0, language: "zh-TW".into(), ..Default::default() }, &path).expect("synth");
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("synthesised {frames} frames, {bytes} bytes → {}", path.display());
    assert!(frames > 0 && bytes > 0, "empty audio");
    match b.transcribe_file(&path, "zh-TW") {
        Ok(t) => println!("said:  {text}\nheard: {t}"),
        Err(e) => println!("transcribe: {e}"),
    }
    let _ = std::fs::remove_file(&path);
}
