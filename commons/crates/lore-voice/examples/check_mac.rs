//! A check of the Apple engines on a real Mac, without the microphone and without playing sound:
//! list the voices, synthesise a sentence to a file, transcribe that file back, and time how long
//! the best zh-TW voice takes to make a first sentence (a whole one or the first words up to a comma)
//! in a fresh helper, cold and after `warm`.
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
    println!("personal voice (not asked): {:?}", b.personal_voice(false));
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

    // timings: each case in a fresh helper process, the best zh-TW voice
    let best = voices.iter().find(|v| v.language == "zh-TW").map(|v| v.id.clone()).unwrap_or_default();
    let whole = "你上次說微積分基本定理還不太熟，我們今天就從那裡開始吧。";
    let first = "你上次說微積分基本定理還不太熟，";
    for (label, warm, line) in [("cold, whole sentence", false, whole), ("cold, first words", false, first), ("warm, whole sentence", true, whole), ("warm, first words", true, first)] {
        let b = HelperBackend::find().unwrap();
        let r = SpeakRequest { text: line.into(), voice: best.clone(), rate: 1.0, language: "zh-TW".into(), ..Default::default() };
        let t = std::time::Instant::now();
        b.voices().expect("voices"); // the helper is running
        let started = t.elapsed().as_secs_f64();
        if warm {
            b.warm(&r);
            std::thread::sleep(std::time::Duration::from_secs(2)); // the window is open a moment before the first reply
        }
        let t = std::time::Instant::now();
        let frames = b.synth_file(&r, &path).expect("synth");
        println!("{label:22} helper start {started:.2} s; made in {:.2} s ({frames} frames)", t.elapsed().as_secs_f64());
        let _ = std::fs::remove_file(&path);
    }
}
