//! A character's own voice (design §7, 自訂聲音): `characters/<id>/voice/`
//!
//! ```text
//! voice.toml      mode = "design" (default; the system voice until a take is picked) | "accent" | "clone" | "system", pitch (semitones), taiwan
//!                 (Taiwan readings), han (和 as hàn), [chosen] (the picked candidate: kind, description,
//!                 apple_voice, text, seed), [clone] (text, consent)
//! chosen.wav      the picked candidate (a designed voice or the Taiwan-accent take): what the
//!                 character speaks with in `design` / `accent`
//! reference.wav   the author's recording (`clone`; 24 kHz mono, 3–30 s), only with consent
//! ```
//!
//! Every mode speaks through Qwen3-TTS by cloning one clip (`chosen.wav` or `reference.wav`), so
//! a reply keeps one timbre from sentence to sentence; the system voice of `character.toml` is the
//! fallback when the runtime is missing. `reference.wav` stays out of an export unless asked for;
//! an imported one needs consent again.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::characters::{self, Character};

pub const DIR: &str = "voice";
pub const FILE: &str = "voice.toml";
pub const CHOSEN: &str = "chosen.wav";
pub const REFERENCE: &str = "reference.wav";
/// A recording's length limits (seconds).
pub const REF_SECONDS: (f64, f64) = (3.0, 30.0);
pub const MODES: [&str; 4] = ["system", "design", "accent", "clone"];
/// The words added to a description so the designed voice leans Taiwanese.
pub const TAIWAN_HINT: &str = "台灣口音，臺灣華語";

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceFile {
    pub mode: String,
    pub pitch: f64,
    #[serde(default = "yes")]
    pub taiwan: bool,
    pub han: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chosen: Option<Chosen>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone: Option<CloneRef>,
}

impl Default for VoiceFile {
    fn default() -> Self {
        VoiceFile { mode: "design".into(), pitch: 0.0, taiwan: true, han: false, chosen: None, clone: None }
    }
}

/// The picked candidate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chosen {
    /// `design` | `accent`.
    pub kind: String,
    pub description: String,
    pub apple_voice: String,
    /// What is said in `chosen.wav`.
    pub text: String,
    pub seed: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloneRef {
    /// What is said in `reference.wav`.
    pub text: String,
    /// 「這是我的聲音或我有權使用」 was ticked.
    pub consent: bool,
}

pub fn folder(app_dir: &Path, id: &str) -> PathBuf {
    app_dir.join(characters::DIR).join(id).join(DIR)
}

pub fn read(app_dir: &Path, id: &str) -> VoiceFile {
    std::fs::read_to_string(folder(app_dir, id).join(FILE)).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

pub fn write(app_dir: &Path, id: &str, v: &VoiceFile) -> Result<(), String> {
    let dir = folder(app_dir, id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = toml::to_string(v).map_err(|e| e.to_string())?;
    write_atomic(&dir.join(FILE), text.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_file_name(format!(".{}.{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("f"), std::process::id()));
    std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| e.to_string())
}

/// What the page shows: the file plus which clips exist (`clone` is usable only with consent).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct View {
    #[serde(flatten)]
    pub file: VoiceFile,
    pub has_chosen: bool,
    pub has_reference: bool,
    pub reference_seconds: Option<f64>,
    /// The clip the character speaks with now ("" = the system voice).
    pub speaking_with: String,
}

pub fn view(app_dir: &Path, id: &str) -> View {
    let file = read(app_dir, id);
    let dir = folder(app_dir, id);
    let reference_seconds = std::fs::read(dir.join(REFERENCE)).ok().and_then(|b| lore_voice::qwen::wav_seconds(&b));
    let speaking_with = custom(app_dir, id, &file).map(|c| c.audio.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string()).unwrap_or_default();
    View { has_chosen: dir.join(CHOSEN).is_file(), has_reference: reference_seconds.is_some(), reference_seconds, speaking_with, file }
}

/// The custom voice of the mode, when its clip is there (and, for `clone`, consent was given).
fn custom(app_dir: &Path, id: &str, f: &VoiceFile) -> Option<lore_voice::qwen::CustomVoice> {
    let dir = folder(app_dir, id);
    let (audio, text) = match f.mode.as_str() {
        "design" | "accent" => {
            let c = f.chosen.as_ref().filter(|c| c.kind == f.mode && !c.text.trim().is_empty())?;
            (dir.join(CHOSEN), c.text.clone())
        }
        "clone" => {
            let c = f.clone.as_ref().filter(|c| c.consent && !c.text.trim().is_empty())?;
            (dir.join(REFERENCE), c.text.clone())
        }
        _ => return None,
    };
    audio.is_file().then_some(lore_voice::qwen::CustomVoice { audio, text, taiwan: f.taiwan, han: f.han })
}

/// How `c` speaks: its system voice and rate, plus its custom voice and pitch when it has one.
pub fn template(app_dir: Option<&Path>, c: &Character) -> lore_voice::SpeakRequest {
    let mut r = lore_voice::SpeakRequest { text: String::new(), voice: c.voice.clone(), rate: c.voice_rate, language: c.language.clone(), ..Default::default() };
    if let Some(dir) = app_dir
        && characters::valid_id(&c.id)
    {
        let f = read(dir, &c.id);
        if let Some(v) = custom(dir, &c.id, &f) {
            r.custom = Some(v);
            r.pitch = lore_voice::clamp_pitch(f.pitch);
        }
    }
    r
}

/// What the page may change directly (the clips come from pick / upload).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Patch {
    pub mode: Option<String>,
    pub pitch: Option<f64>,
    pub taiwan: Option<bool>,
    pub han: Option<bool>,
    /// What is said in the recording.
    pub clone_text: Option<String>,
    /// 「這是我的聲音或我有權使用」 for the recording that is there (e.g. after an import).
    pub consent: Option<bool>,
}

pub fn patch(app_dir: &Path, id: &str, p: &Patch) -> Result<View, String> {
    characters::load(app_dir, id)?;
    let mut f = read(app_dir, id);
    if let Some(m) = &p.mode {
        if !MODES.contains(&m.as_str()) {
            return Err(format!("不認得「{m}」"));
        }
        f.mode = m.clone();
    }
    if let Some(x) = p.pitch {
        f.pitch = lore_voice::clamp_pitch(x);
    }
    if let Some(x) = p.taiwan {
        f.taiwan = x;
    }
    if let Some(x) = p.han {
        f.han = x;
    }
    if let Some(t) = &p.clone_text {
        if t.chars().count() > 500 {
            return Err("逐字稿太長（最多 500 字）".into());
        }
        f.clone.get_or_insert_with(CloneRef::default).text = t.trim().to_string();
    }
    if let Some(c) = p.consent {
        if c && !folder(app_dir, id).join(REFERENCE).is_file() {
            return Err("還沒有錄音".into());
        }
        f.clone.get_or_insert_with(CloneRef::default).consent = c;
    }
    write(app_dir, id, &f)?;
    Ok(view(app_dir, id))
}

/// Keep candidate `from` (a wav) as the character's voice.
pub fn pick(app_dir: &Path, id: &str, from: &Path, chosen: Chosen) -> Result<View, String> {
    characters::load(app_dir, id)?;
    let dir = folder(app_dir, id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(from).map_err(|_| "這個候選已經不在了，請重新產生".to_string())?;
    write_atomic(&dir.join(CHOSEN), &bytes)?;
    let mut f = read(app_dir, id);
    f.mode = chosen.kind.clone();
    f.chosen = Some(chosen);
    write(app_dir, id, &f)?;
    Ok(view(app_dir, id))
}

/// The author's recording (any format macOS reads; `consent` must be true), checked for length
/// and kept as 24 kHz mono wav; the mode becomes `clone`.
pub fn save_reference(app_dir: &Path, id: &str, bytes: &[u8], consent: bool, text: Option<&str>) -> Result<View, String> {
    if !consent {
        return Err("要先勾選「這是我的聲音或我有權使用」".into());
    }
    characters::load(app_dir, id)?;
    let ext = characters::sniff_audio(bytes)?;
    let dir = folder(app_dir, id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let raw = dir.join(format!(".upload.{}.in.{ext}", std::process::id()));
    let wav = dir.join(format!(".upload.{}.wav", std::process::id()));
    std::fs::write(&raw, bytes).map_err(|e| e.to_string())?;
    let converted = if cfg!(target_os = "macos") {
        lore_voice::qwen::to_wav24(&raw, &wav)
    } else if ext == "wav" {
        std::fs::copy(&raw, &wav).map(|_| ()).map_err(|e| e.to_string())
    } else {
        Err("這台電腦只收 wav".into())
    };
    let _ = std::fs::remove_file(&raw);
    let result = converted.and_then(|_| {
        let b = std::fs::read(&wav).map_err(|e| e.to_string())?;
        let s = lore_voice::qwen::wav_seconds(&b).ok_or("讀不出長度")?;
        if s < REF_SECONDS.0 || s > REF_SECONDS.1 {
            return Err(format!("錄音要 {:.0}–{:.0} 秒（這段 {s:.1} 秒）", REF_SECONDS.0, REF_SECONDS.1));
        }
        std::fs::rename(&wav, dir.join(REFERENCE)).map_err(|e| e.to_string())
    });
    let _ = std::fs::remove_file(&wav);
    result?;
    let mut f = read(app_dir, id);
    let c = f.clone.get_or_insert_with(CloneRef::default);
    c.consent = true;
    if let Some(t) = text {
        c.text = t.trim().chars().take(500).collect();
    }
    f.mode = "clone".into();
    write(app_dir, id, &f)?;
    Ok(view(app_dir, id))
}

/// Remove the recording (and its consent); a `clone` character goes back to 描述 (its picked take, or the system voice).
pub fn remove_reference(app_dir: &Path, id: &str) -> Result<View, String> {
    characters::load(app_dir, id)?;
    let _ = std::fs::remove_file(folder(app_dir, id).join(REFERENCE));
    let mut f = read(app_dir, id);
    f.clone = None;
    if f.mode == "clone" {
        f.mode = "design".into();
    }
    write(app_dir, id, &f)?;
    Ok(view(app_dir, id))
}

/// The files of `voice/` that go into an export: voice.toml and chosen.wav always,
/// reference.wav only with `with_reference` (then its consent travels too).
pub fn export_files(app_dir: &Path, id: &str, with_reference: bool) -> Vec<(String, Vec<u8>)> {
    let dir = folder(app_dir, id);
    let mut f = read(app_dir, id);
    if !dir.join(FILE).is_file() {
        return vec![];
    }
    let mut out = vec![];
    if let Ok(b) = std::fs::read(dir.join(CHOSEN)) {
        out.push((format!("{DIR}/{CHOSEN}"), b));
    }
    match std::fs::read(dir.join(REFERENCE)) {
        Ok(b) if with_reference => out.push((format!("{DIR}/{REFERENCE}"), b)),
        _ => {
            f.clone = None;
            if f.mode == "clone" {
                f.mode = "design".into();
            }
        }
    }
    out.push((format!("{DIR}/{FILE}"), toml::to_string(&f).unwrap_or_default().into_bytes()));
    out
}

/// The `voice/` files of an imported character (names as in [`export_files`]); a recording comes
/// in without consent (the author ticks it again before it is used).
pub fn import_files(app_dir: &Path, id: &str, files: &[(String, Vec<u8>)]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let dir = folder(app_dir, id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut f = VoiceFile::default();
    for (name, bytes) in files {
        match name.as_str() {
            FILE => f = toml::from_str(std::str::from_utf8(bytes).map_err(|_| "voice.toml 不是 UTF-8")?).map_err(|e| format!("voice.toml 讀不懂：{e}"))?,
            CHOSEN | REFERENCE => {
                if lore_voice::qwen::wav_seconds(bytes).is_none() {
                    return Err(format!("{name} 不是 wav"));
                }
                write_atomic(&dir.join(name), bytes)?;
            }
            _ => {}
        }
    }
    if let Some(c) = f.clone.as_mut() {
        c.consent = false;
    }
    if !MODES.contains(&f.mode.as_str()) {
        f.mode = "system".into();
    }
    f.pitch = lore_voice::clamp_pitch(f.pitch);
    write(app_dir, id, &f)
}

/// A file name inside `voice/` that an import reads.
pub fn importable(name: &str) -> bool {
    [FILE, CHOSEN, REFERENCE].contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().to_path_buf();
        let d = app.join(characters::DIR).join("echo");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(characters::FILE), "name = \"Echo\"\nvoice = \"apple.tw\"\n").unwrap();
        (tmp, app)
    }

    fn wav(secs: f64) -> Vec<u8> {
        lore_voice::qwen::wav_bytes(&vec![0i16; (24000.0 * secs) as usize], 24000)
    }

    #[test]
    fn no_voice_folder_is_the_system_voice() {
        let (_t, app) = home();
        let c = characters::load(&app, "echo").unwrap();
        let r = template(Some(&app), &c);
        assert_eq!((r.voice.as_str(), r.custom.is_none()), ("apple.tw", true));
        assert_eq!(view(&app, "echo").file.mode, "design", "描述 is where the tab opens; nothing picked yet = the system voice");
    }

    #[test]
    fn a_picked_candidate_is_spoken_with_its_pitch() {
        let (_t, app) = home();
        let cand = app.join("cand.wav");
        std::fs::write(&cand, wav(2.0)).unwrap();
        let v = pick(&app, "echo", &cand, Chosen { kind: "design".into(), description: "低沉".into(), text: "你好。".into(), seed: 7, ..Default::default() }).unwrap();
        assert!(v.has_chosen && v.speaking_with == CHOSEN);
        patch(&app, "echo", &Patch { pitch: Some(-20.0), ..Default::default() }).unwrap();
        let r = template(Some(&app), &characters::load(&app, "echo").unwrap());
        let c = r.custom.unwrap();
        assert_eq!((c.text.as_str(), c.taiwan, r.pitch), ("你好。", true, lore_voice::PITCH.0));
        assert_eq!(r.voice, "apple.tw", "the fallback stays");
        // back to the system voice: the clip stays for later
        patch(&app, "echo", &Patch { mode: Some("system".into()), ..Default::default() }).unwrap();
        assert!(template(Some(&app), &characters::load(&app, "echo").unwrap()).custom.is_none());
        assert!(patch(&app, "echo", &Patch { mode: Some("x".into()), ..Default::default() }).is_err());
    }

    #[test]
    fn a_recording_needs_consent_a_transcript_and_the_right_length() {
        let (_t, app) = home();
        assert!(save_reference(&app, "echo", &wav(5.0), false, Some("一二三")).unwrap_err().contains("有權使用"));
        assert!(!folder(&app, "echo").join(REFERENCE).exists());
        assert!(save_reference(&app, "echo", &wav(1.0), true, Some("一")).unwrap_err().contains("3–30"));
        assert!(save_reference(&app, "echo", &wav(31.0), true, Some("一")).is_err());
        assert!(save_reference(&app, "echo", b"not audio at all, really", true, None).is_err());
        let v = save_reference(&app, "echo", &wav(5.0), true, None).unwrap();
        assert_eq!(v.file.mode, "clone");
        assert!((v.reference_seconds.unwrap() - 5.0).abs() < 0.05);
        assert_eq!(v.speaking_with, "", "no transcript yet: not used");
        let v = patch(&app, "echo", &Patch { clone_text: Some("一二三四五".into()), ..Default::default() }).unwrap();
        assert_eq!(v.speaking_with, REFERENCE);
        let v = remove_reference(&app, "echo").unwrap();
        assert!(!v.has_reference && v.file.mode == "design" && v.file.clone.is_none());
    }

    #[test]
    fn export_leaves_the_recording_out_unless_asked_and_import_asks_consent_again() {
        let (_t, app) = home();
        save_reference(&app, "echo", &wav(4.0), true, Some("一二三")).unwrap();
        let cand = app.join("cand.wav");
        std::fs::write(&cand, wav(2.0)).unwrap();
        pick(&app, "echo", &cand, Chosen { kind: "accent".into(), apple_voice: "apple.tw".into(), text: "你好。".into(), ..Default::default() }).unwrap();
        patch(&app, "echo", &Patch { mode: Some("clone".into()), ..Default::default() }).unwrap();
        let plain = export_files(&app, "echo", false);
        let names: Vec<&str> = plain.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["voice/chosen.wav", "voice/voice.toml"]);
        let toml_text = String::from_utf8(plain[1].1.clone()).unwrap();
        assert!(!toml_text.contains("consent") && toml_text.contains("mode = \"design\""), "{toml_text}");
        let with = export_files(&app, "echo", true);
        assert!(with.iter().any(|f| f.0 == "voice/reference.wav"));
        // into another character
        let d = app.join(characters::DIR).join("copy");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(characters::FILE), "name = \"Copy\"\n").unwrap();
        let files: Vec<(String, Vec<u8>)> = with.into_iter().map(|(n, b)| (n.trim_start_matches("voice/").to_string(), b)).collect();
        import_files(&app, "copy", &files).unwrap();
        let v = view(&app, "copy");
        assert!(v.has_reference && v.has_chosen);
        assert_eq!(v.file.clone.as_ref().map(|c| c.consent), Some(false));
        assert_eq!(v.speaking_with, "", "no consent on this machine yet");
        // ticked again on this machine
        patch(&app, "copy", &Patch { mode: Some("clone".into()), consent: Some(true), ..Default::default() }).unwrap();
        assert_eq!(view(&app, "copy").speaking_with, REFERENCE);
        assert!(patch(&app, "echo", &Patch { consent: Some(true), ..Default::default() }).is_ok());
        remove_reference(&app, "echo").unwrap();
        assert!(patch(&app, "echo", &Patch { consent: Some(true), ..Default::default() }).is_err(), "no recording, nothing to consent to");
    }
}
