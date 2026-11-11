//! A check of Qwen3-TTS on a real Mac (no sound is played): with an installed `<home>/tts`
//! (`--install` installs it there first: ≈ 6 GB), make the Taiwan accent reference with the
//! Apple zh-TW voice, clone it, design a few voices from a description, and print the timings.
//! Files go to `<out>`; listen to them yourself.
//!
//! ```sh
//! cargo run -p lore-voice --example check_qwen -- <home> <out> [--install]
//! ```
//! Use a scratch folder for `<home>` unless you mean to install into the real model folder.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use lore_voice::helper::HelperBackend;
use lore_voice::qwen::{self, CustomVoice, DesignJob, Qwen, SynthJob, Tts};
use lore_voice::{Backend, SpeakRequest, sort_voices};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (Some(home), Some(out)) = (a.get(1).map(PathBuf::from), a.get(2).map(PathBuf::from)) else {
        eprintln!("usage: check_qwen <home> <out> [--install]");
        std::process::exit(2);
    };
    std::fs::create_dir_all(&out).unwrap();
    let q = Qwen::new(Some(home));
    if a.iter().any(|x| x == "--install") {
        let t = Instant::now();
        q.install(&|p| eprint!("\r{} {}/{} MB   ", p.phase, p.done / 1_000_000, p.total / 1_000_000), &AtomicBool::new(false)).expect("install");
        eprintln!("\ninstalled in {:.0} s", t.elapsed().as_secs_f64());
    }
    println!("status: {:?}", q.status());

    // 1. the accent reference: the best Apple zh-TW voice reads the passage
    let b = HelperBackend::find().expect("lore-voice-helper");
    let mut voices = b.voices().expect("voices");
    sort_voices(&mut voices, "zh-TW");
    let tw = voices.iter().find(|v| v.language == "zh-TW").expect("a zh-TW voice");
    println!("accent voice: {} ({:?})", tw.name, tw.quality);
    let caf = out.join("accent.caf");
    b.synth_file(&SpeakRequest { text: qwen::ACCENT_PASSAGE.into(), voice: tw.id.clone(), rate: 1.0, language: "zh-TW".into(), ..Default::default() }, &caf).expect("apple synth");
    let accent = out.join("accent.wav");
    qwen::to_wav24(&caf, &accent).expect("afconvert");
    println!("accent reference: {:.1} s", qwen::wav_seconds(&std::fs::read(&accent).unwrap()).unwrap_or(0.0));

    let line = "你好，我是你的學習夥伴。今天想先複習哪一章？我們可以把垃圾分類的作業和期末報告一起排進禮拜三。";
    let voice = CustomVoice { audio: accent.clone(), text: qwen::ACCENT_PASSAGE.into(), taiwan: true, han: false };
    for i in 0..2 {
        let t = Instant::now();
        let o = out.join(format!("clone-{i}.wav"));
        match q.synth(&SynthJob { text: line.into(), language: "zh-TW".into(), voice: voice.clone(), seed: None, out: o.clone() }) {
            Ok(()) => {
                let s = qwen::wav_seconds(&std::fs::read(&o).unwrap()).unwrap_or(0.0);
                println!("clone {i}: {s:.1} s audio in {:.1} s (RTF {:.2})", t.elapsed().as_secs_f64(), t.elapsed().as_secs_f64() / s.max(0.01));
            }
            Err(e) => println!("clone {i}: {e}"),
        }
    }
    let description = "三十多歲男性，嗓音低沉略帶沙啞，語速慢，冷靜，台灣口音，臺灣華語";
    for seed in 1..=3u64 {
        let t = Instant::now();
        let o = out.join(format!("design-{seed}.wav"));
        match q.design(&DesignJob { text: line.into(), language: "zh-TW".into(), description: description.into(), seed, out: o.clone() }) {
            Ok(()) => {
                let s = qwen::wav_seconds(&std::fs::read(&o).unwrap()).unwrap_or(0.0);
                println!("design seed {seed}: {s:.1} s audio in {:.1} s", t.elapsed().as_secs_f64());
            }
            Err(e) => println!("design {seed}: {e}"),
        }
    }
    // the designed voice, cloned: the same timbre for every sentence of a reply
    let d1 = out.join("design-1.wav");
    if d1.is_file() {
        let t = Instant::now();
        let o = out.join("design-1-cloned.wav");
        let r = q.synth(&SynthJob { text: "第二句話，聲音應該和剛才一樣。".into(), language: "zh-TW".into(), voice: CustomVoice { audio: d1, text: line.into(), taiwan: true, han: false }, seed: None, out: o });
        println!("designed voice cloned: {r:?} in {:.1} s", t.elapsed().as_secs_f64());
    }
    // transcribe what came out, as a rough check that it is Mandarin and intelligible
    for f in ["clone-0.wav", "design-1.wav", "design-1-cloned.wav"] {
        if let Ok(t) = b.transcribe_file(&out.join(f), "zh-TW") {
            println!("{f} heard: {t}");
        }
    }
    q.stop();
}
