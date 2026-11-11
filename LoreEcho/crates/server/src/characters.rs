//! Characters (design §6): `private/LoreEcho/characters/<id>/`
//!
//! ```text
//! character.toml   name, persona (the system prompt's personality), language,
//!                  voice (a system voice id, "" = the language's default), voice_rate (1 = normal),
//!                  voice_pitch (1 = the voice's own),
//!                  the profile (角色設定: background, speech_style, catchphrases, call_user,
//!                  relationship, likes, dislikes, [[examples]] user/reply), see [`Profile`],
//!                  theme_loop, theme_volume (the theme song),
//!                  figure_scale, figure_offset_y (how big the full body stands in the open window)
//! avatar/          <state>.png .svg .webp .jpg   (portrait: the bubble, the name tag)
//! full/            the same states, full body (head to feet, transparent background): the open window
//! theme.<ext>      the theme song (mp3 m4a aac wav ogg opus flac), optional
//! ```
//!
//! Ten states ([`STATES`]); a state without its own picture follows [`chain`] (searching →
//! thinking → idle, noting → speaking → idle, waiting → thinking → idle, the rest → idle).
//! A character without `full/` shows its portrait in the open window too. With no character at all the app still talks, as
//! [`Character::builtin`] (no picture: the page shows the app icon).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DIR: &str = "characters";
pub const FILE: &str = "character.toml";
pub const STATES: [&str; 10] = ["idle", "listening", "thinking", "speaking", "happy", "unsure", "surprised", "waiting", "searching", "noting"];
/// Largest picture accepted (bytes).
pub const MAX_IMAGE: usize = 10 * 1024 * 1024;

/// The states to try for `state`, own first, `idle` last (design §6).
pub fn chain(state: &str) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = match state {
        "searching" => vec!["searching", "thinking"],
        "noting" => vec!["noting", "speaking"],
        "waiting" => vec!["waiting", "thinking"],
        other => STATES.iter().copied().filter(|s| *s == other).collect(),
    };
    if v.last() != Some(&"idle") {
        v.push("idle");
    }
    v
}
/// Picture formats, in the order they are looked for.
const EXTS: [(&str, &str); 5] = [("svg", "image/svg+xml"), ("png", "image/png"), ("webp", "image/webp"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg")];
/// The portrait folder (bubble) and the full-body folder (open window).
pub const AVATAR: &str = "avatar";
pub const FULL: &str = "full";
/// Id of [`Character::builtin`].
pub const BUILTIN: &str = "default";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct File {
    name: String,
    persona: String,
    voice: String,
    voice_rate: Option<f64>,
    voice_pitch: Option<f64>,
    language: String,
    figure_scale: Option<f64>,
    figure_offset_y: Option<f64>,
    theme_loop: Option<bool>,
    theme_volume: Option<f64>,
    #[serde(flatten)]
    profile: Profile,
}

/// 角色設定: who the character is, beyond the persona. All optional; the system prompt is built
/// from them (`chat::system_prompt`) and example pairs become earlier turns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// 身分與背景 (a story, several lines).
    pub background: String,
    /// 說話風格.
    pub speech_style: String,
    /// 口頭禪, one per entry.
    pub catchphrases: Vec<String>,
    /// 怎麼稱呼你 (what the character calls the user).
    pub call_user: String,
    /// 和你的關係.
    pub relationship: String,
    pub likes: String,
    pub dislikes: String,
    /// 範例對話: a few user line / character line pairs.
    pub examples: Vec<Example>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Example {
    pub user: String,
    pub reply: String,
}

/// What is kept of a profile (longer is refused when saving).
pub const PROFILE_MAX: usize = 4000;
pub const FIELD_MAX: usize = 300;
pub const MAX_CATCHPHRASES: usize = 20;
pub const MAX_EXAMPLES: usize = 8;

impl Profile {
    pub fn is_empty(&self) -> bool {
        *self == Profile::default()
    }

    /// Trimmed, empty entries dropped; an error for what is too long.
    pub fn clean(&self) -> Result<Profile, String> {
        let one = |label: &str, v: &str, max: usize| -> Result<String, String> {
            let t = v.trim();
            if t.chars().count() > max { Err(format!("「{label}」太長（最多 {max} 字）")) } else { Ok(t.to_string()) }
        };
        let catchphrases: Vec<String> = self.catchphrases.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
        if catchphrases.len() > MAX_CATCHPHRASES {
            return Err(format!("口頭禪最多 {MAX_CATCHPHRASES} 個"));
        }
        let mut examples = vec![];
        for e in &self.examples {
            let (u, r) = (one("範例對話", &e.user, FIELD_MAX)?, one("範例對話", &e.reply, FIELD_MAX)?);
            if !u.is_empty() && !r.is_empty() {
                examples.push(Example { user: u, reply: r });
            }
        }
        if examples.len() > MAX_EXAMPLES {
            return Err(format!("範例對話最多 {MAX_EXAMPLES} 組"));
        }
        Ok(Profile {
            background: one("身分與背景", &self.background, PROFILE_MAX)?,
            speech_style: one("說話風格", &self.speech_style, FIELD_MAX)?,
            catchphrases: catchphrases.iter().map(|c| one("口頭禪", c, 60)).collect::<Result<_, _>>()?,
            call_user: one("怎麼稱呼你", &self.call_user, 60)?,
            relationship: one("和你的關係", &self.relationship, FIELD_MAX)?,
            likes: one("喜歡", &self.likes, FIELD_MAX)?,
            dislikes: one("不喜歡", &self.dislikes, FIELD_MAX)?,
            examples,
        })
    }
}

/// The theme song's settings (sent to the page when the character has one).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Theme {
    /// Start again at the end.
    #[serde(rename = "loop")]
    pub repeat: bool,
    /// 0–1.
    pub volume: f64,
}

impl Theme {
    pub const DEFAULT_VOLUME: f64 = 0.5;
    fn from(repeat: Option<bool>, volume: Option<f64>) -> Theme {
        Theme { repeat: repeat.unwrap_or(true), volume: volume.filter(|v| v.is_finite()).map_or(Theme::DEFAULT_VOLUME, |v| v.clamp(0.0, 1.0)) }
    }
}

/// Speaking speed limits (a multiplier, 1 = normal; lore-voice clamps the same way).
pub const VOICE_RATE: (f64, f64) = (0.5, 2.0);

fn voice_rate(v: Option<f64>) -> f64 {
    v.filter(|v| v.is_finite()).map_or(1.0, |v| v.clamp(VOICE_RATE.0, VOICE_RATE.1))
}

/// Pitch limits (a multiplier of the voice's own, 1 = unchanged; lore-voice clamps the same way).
pub const VOICE_PITCH: (f64, f64) = (0.5, 2.0);

fn voice_pitch(v: Option<f64>) -> f64 {
    v.filter(|v| v.is_finite()).map_or(1.0, |v| v.clamp(VOICE_PITCH.0, VOICE_PITCH.1))
}

/// How the full body stands in the open window (a visual-novel sprite): `scale` 1.0 = head
/// to feet fit the window height, more = bigger, the legs run off the window's bottom edge;
/// `offset_y` moves it down by that fraction of the window height (negative = up).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Figure {
    pub scale: f64,
    pub offset_y: f64,
}

impl Default for Figure {
    fn default() -> Figure {
        Figure { scale: 1.0, offset_y: 0.0 }
    }
}

impl Figure {
    pub const SCALE: (f64, f64) = (0.5, 3.0);
    pub const OFFSET_Y: (f64, f64) = (-0.5, 0.5);

    /// From character.toml: missing or not a number → default, out of range → clamped.
    fn from(scale: Option<f64>, offset_y: Option<f64>) -> Figure {
        let pick = |v: Option<f64>, d: f64, (lo, hi): (f64, f64)| v.filter(|v| v.is_finite()).map_or(d, |v| v.clamp(lo, hi));
        Figure { scale: pick(scale, 1.0, Figure::SCALE), offset_y: pick(offset_y, 0.0, Figure::OFFSET_Y) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    /// Not sent to the page (it is the system prompt's).
    #[serde(skip)]
    pub persona: String,
    /// A system voice id ("" = the system's voice for `language`).
    pub voice: String,
    /// 1.0 = normal speed.
    pub voice_rate: f64,
    /// 1.0 = the voice's own pitch.
    pub voice_pitch: f64,
    pub language: String,
    /// The theme song's settings, when there is a `theme.<ext>`.
    pub theme: Option<Theme>,
    /// 角色設定 (not sent to the page with the list; the 人物 tab reads it with [`detail`]).
    #[serde(skip)]
    pub profile: Profile,
    /// The states that have their own portrait (`avatar/`).
    pub avatars: Vec<String>,
    /// How the full body stands in the open window.
    pub figure: Figure,
    /// The states that have their own full-body picture (`full/`); empty = no full body
    /// (the open window then shows the portrait).
    pub full: Vec<String>,
}

impl Character {
    /// Used when no character folder can be read.
    pub fn builtin() -> Character {
        Character {
            id: BUILTIN.into(),
            name: "LoreEcho".into(),
            persona: "你是使用者的學習夥伴：安靜、好奇、有耐心。你陪他把想法說清楚，必要時反問一句，幫他找出還不懂的地方。".into(),
            voice: String::new(),
            voice_rate: 1.0,
            voice_pitch: 1.0,
            language: "zh-TW".into(),
            theme: None,
            profile: Profile::default(),
            avatars: vec![],
            figure: Figure::default(),
            full: vec![],
        }
    }
}

/// Folder names that can be a character id (no paths, no dots).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn folder(app_dir: &Path, id: &str) -> PathBuf {
    app_dir.join(DIR).join(id)
}

/// `dir/<kind>/<state>.<ext>` (kind = [`AVATAR`] or [`FULL`]), the first format found.
fn picture(dir: &Path, kind: &str, state: &str) -> Option<(PathBuf, &'static str)> {
    EXTS.iter().map(|(ext, mime)| (dir.join(kind).join(format!("{state}.{ext}")), *mime)).find(|(p, _)| p.is_file())
}

fn states_with(dir: &Path, kind: &str) -> Vec<String> {
    STATES.iter().filter(|s| picture(dir, kind, s).is_some()).map(|s| s.to_string()).collect()
}

/// One character, or why it cannot be read.
pub fn load(app_dir: &Path, id: &str) -> Result<Character, String> {
    if !valid_id(id) {
        return Err(format!("人物名稱不對：「{id}」"));
    }
    let dir = folder(app_dir, id);
    let path = dir.join(FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let f: File = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = f.name.trim();
    Ok(Character {
        id: id.into(),
        name: if name.is_empty() { id.into() } else { name.into() },
        persona: f.persona.trim().into(),
        voice: f.voice.trim().into(),
        voice_rate: voice_rate(f.voice_rate),
        voice_pitch: voice_pitch(f.voice_pitch),
        language: Some(f.language.trim()).filter(|l| !l.is_empty()).unwrap_or("zh-TW").into(),
        theme: theme_file(&dir).map(|_| Theme::from(f.theme_loop, f.theme_volume)),
        profile: f.profile,
        avatars: states_with(&dir, AVATAR),
        figure: Figure::from(f.figure_scale, f.figure_offset_y),
        full: states_with(&dir, FULL),
    })
}

/// Every readable character, by id (a broken one is reported on stderr and left out).
pub fn list(app_dir: &Path) -> Vec<Character> {
    let Ok(rd) = std::fs::read_dir(app_dir.join(DIR)) else { return vec![] };
    let mut ids: Vec<String> = rd.flatten().filter(|e| e.path().join(FILE).is_file()).filter_map(|e| e.file_name().into_string().ok()).filter(|id| valid_id(id)).collect();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| match load(app_dir, &id) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("le-server: {e}");
                None
            }
        })
        .collect()
}

/// The character to talk as: `wanted` if it loads, else the first one, else the built-in.
pub fn pick(app_dir: Option<&Path>, wanted: Option<&str>) -> Character {
    let Some(dir) = app_dir else { return Character::builtin() };
    if let Some(id) = wanted.filter(|w| *w != BUILTIN)
        && let Ok(c) = load(dir, id)
    {
        return c;
    }
    list(dir).into_iter().next().unwrap_or_else(Character::builtin)
}

/// The picture for `state` (the next of its [`chain`] when it has none): path and content type. `None` for
/// an unknown state, id or character without pictures.
pub fn avatar(app_dir: &Path, id: &str, state: &str) -> Option<(PathBuf, &'static str)> {
    if !valid_id(id) || !STATES.contains(&state) {
        return None;
    }
    let dir = folder(app_dir, id);
    chain(state).into_iter().find_map(|s| picture(&dir, AVATAR, s))
}

/// The full-body picture for `state`: `full/<state>`, else `full/idle`; a character with no
/// full body at all gets its portrait ([`avatar`]) instead. `None` as for [`avatar`].
pub fn full_body(app_dir: &Path, id: &str, state: &str) -> Option<(PathBuf, &'static str)> {
    if !valid_id(id) || !STATES.contains(&state) {
        return None;
    }
    let dir = folder(app_dir, id);
    chain(state).into_iter().find_map(|s| picture(&dir, FULL, s)).or_else(|| avatar(app_dir, id, state))
}

// ── editing: the 人物 tab ────────────────────────────────────────

use std::io::{Read, Write};

/// Everything the 人物 tab edits (persona and profile included).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Detail {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub voice: String,
    pub voice_rate: f64,
    pub voice_pitch: f64,
    pub language: String,
    pub theme: Option<Theme>,
    pub profile: Profile,
    /// How much of 身分與背景 reaches the model (the tab shows the count against it).
    pub background_limit: usize,
}

pub fn detail(app_dir: &Path, id: &str) -> Result<Detail, String> {
    let c = load(app_dir, id)?;
    Ok(Detail { id: c.id, name: c.name, persona: c.persona, voice: c.voice, voice_rate: c.voice_rate, voice_pitch: c.voice_pitch, language: c.language, theme: c.theme, profile: c.profile, background_limit: crate::chat::BACKGROUND_LIMIT })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetaPatch {
    pub name: Option<String>,
    pub persona: Option<String>,
    pub voice: Option<String>,
    pub voice_rate: Option<f64>,
    pub voice_pitch: Option<f64>,
    pub theme_loop: Option<bool>,
    pub theme_volume: Option<f64>,
    /// The whole 角色設定 (replaces the one there).
    pub profile: Option<Profile>,
}

const PROFILE_KEYS: [&str; 8] = ["background", "speech_style", "catchphrases", "call_user", "relationship", "likes", "dislikes", "examples"];

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Change name / persona / voice in `character.toml` (other keys kept; comments are not).
pub fn save_meta(app_dir: &Path, id: &str, p: &MetaPatch) -> Result<Detail, String> {
    load(app_dir, id)?;
    let path = folder(app_dir, id).join(FILE);
    let mut t: toml::Table = std::fs::read_to_string(&path).map_err(|e| e.to_string())?.parse().map_err(|e| format!("{e}"))?;
    for (key, v) in [("name", &p.name), ("persona", &p.persona), ("voice", &p.voice)] {
        if let Some(v) = v {
            if v.chars().count() > 4000 {
                return Err("文字太長".into());
            }
            t.insert(key.into(), toml::Value::String(v.trim().to_string()));
        }
    }
    if let Some(r) = p.voice_rate {
        t.insert("voice_rate".into(), toml::Value::Float(voice_rate(Some(r))));
    }
    if let Some(r) = p.voice_pitch {
        t.insert("voice_pitch".into(), toml::Value::Float(voice_pitch(Some(r))));
    }
    if let Some(l) = p.theme_loop {
        t.insert("theme_loop".into(), toml::Value::Boolean(l));
    }
    if let Some(v) = p.theme_volume {
        t.insert("theme_volume".into(), toml::Value::Float(Theme::from(None, Some(v)).volume));
    }
    if let Some(pr) = &p.profile {
        let pr = pr.clean()?;
        for k in PROFILE_KEYS {
            t.remove(k);
        }
        // only what is filled in is written (an empty profile leaves no keys)
        if let toml::Value::Table(pt) = toml::Value::try_from(&pr).map_err(|e| e.to_string())? {
            for (k, v) in pt {
                let empty = match &v {
                    toml::Value::String(s) => s.is_empty(),
                    toml::Value::Array(a) => a.is_empty(),
                    _ => false,
                };
                if !empty {
                    t.insert(k, v);
                }
            }
        }
    }
    write_atomic(&path, t.to_string().as_bytes()).map_err(|e| e.to_string())?;
    detail(app_dir, id)
}

/// The format of `bytes` as a file extension, or why it is not a usable picture.
pub fn sniff(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.is_empty() {
        return Err("空的檔案".into());
    }
    if bytes.len() > MAX_IMAGE {
        return Err("圖太大（上限 10 MB）".into());
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok("png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("jpg");
    }
    if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok("webp");
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        let low = text.to_ascii_lowercase();
        if low.contains("<svg") {
            // shown through <img>, which runs nothing; refused anyway so a direct visit is harmless too
            let handler = low.match_indices(" on").any(|(i, _)| {
                let rest = &low[i + 3..];
                let n = rest.chars().take_while(|c| c.is_ascii_lowercase()).count();
                n > 0 && rest[n..].trim_start().starts_with('=')
            });
            if low.contains("<script") || low.contains("javascript:") || low.contains("<foreignobject") || handler {
                return Err("SVG 裡有程式碼，不能用".into());
            }
            return Ok("svg");
        }
    }
    Err("只收 png、jpg、webp、svg".into())
}

/// The theme song's largest size (bytes).
pub const MAX_AUDIO: usize = 30 * 1024 * 1024;
/// Theme song formats: extension and content type.
pub const AUDIO_EXTS: [(&str, &str); 7] = [("mp3", "audio/mpeg"), ("m4a", "audio/mp4"), ("aac", "audio/aac"), ("wav", "audio/wav"), ("ogg", "audio/ogg"), ("opus", "audio/ogg"), ("flac", "audio/flac")];
pub const THEME: &str = "theme";

/// The format of a theme song as a file extension, or why it is not one we take.
pub fn sniff_audio(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.len() < 12 {
        return Err("空的或太短的音樂檔".into());
    }
    if bytes.len() > MAX_AUDIO {
        return Err("音樂檔太大（上限 30 MB）".into());
    }
    let b = bytes;
    if b.starts_with(b"ID3") {
        return Ok("mp3");
    }
    if b[0] == 0xFF && (b[1] & 0xF6) == 0xF0 {
        return Ok("aac"); // ADTS: sync word, layer 00
    }
    if b[0] == 0xFF && (b[1] & 0xE0) == 0xE0 && (b[1] & 0x06) != 0 {
        return Ok("mp3"); // MPEG audio frame sync, layer I–III
    }
    if &b[4..8] == b"ftyp" {
        return Ok("m4a");
    }
    if &b[..4] == b"RIFF" && &b[8..12] == b"WAVE" {
        return Ok("wav");
    }
    if b.starts_with(b"OggS") {
        let head = &b[..b.len().min(128)];
        return Ok(if head.windows(8).any(|w| w == b"OpusHead") { "opus" } else { "ogg" });
    }
    if b.starts_with(b"fLaC") {
        return Ok("flac");
    }
    Err("只收 mp3、m4a、aac、wav、ogg、opus、flac".into())
}

/// `theme.<ext>` of a character folder: path and content type.
fn theme_file(dir: &Path) -> Option<(PathBuf, &'static str)> {
    AUDIO_EXTS.iter().map(|(ext, mime)| (dir.join(format!("{THEME}.{ext}")), *mime)).find(|(p, _)| p.is_file())
}

/// The theme song of `id`, if it has one.
pub fn theme(app_dir: &Path, id: &str) -> Option<(PathBuf, &'static str)> {
    if !valid_id(id) {
        return None;
    }
    theme_file(&folder(app_dir, id))
}

/// Put a theme song in place (replacing the one there, whatever its format).
pub fn save_theme(app_dir: &Path, id: &str, bytes: &[u8]) -> Result<(), String> {
    load(app_dir, id)?;
    let ext = sniff_audio(bytes)?;
    let dir = folder(app_dir, id);
    write_atomic(&dir.join(format!("{THEME}.{ext}")), bytes).map_err(|e| e.to_string())?;
    for (other, _) in AUDIO_EXTS {
        if other != ext {
            let _ = std::fs::remove_file(dir.join(format!("{THEME}.{other}")));
        }
    }
    Ok(())
}

pub fn remove_theme(app_dir: &Path, id: &str) -> Result<(), String> {
    load(app_dir, id)?;
    let dir = folder(app_dir, id);
    let mut found = false;
    for (ext, _) in AUDIO_EXTS {
        found |= std::fs::remove_file(dir.join(format!("{THEME}.{ext}"))).is_ok();
    }
    if found { Ok(()) } else { Err("沒有主題曲".into()) }
}

fn check_slot(kind: &str, state: &str) -> Result<(), String> {
    if kind != AVATAR && kind != FULL {
        return Err(format!("不認得「{kind}」"));
    }
    if !STATES.contains(&state) {
        return Err(format!("沒有這個狀態：{state}"));
    }
    Ok(())
}

/// Put a picture into `<kind>/<state>` (replacing the one there, whatever its format).
pub fn save_image(app_dir: &Path, id: &str, kind: &str, state: &str, bytes: &[u8]) -> Result<(), String> {
    check_slot(kind, state)?;
    load(app_dir, id)?;
    let ext = sniff(bytes)?;
    let dir = folder(app_dir, id).join(kind);
    write_atomic(&dir.join(format!("{state}.{ext}")), bytes).map_err(|e| e.to_string())?;
    for (other, _) in EXTS {
        if other != ext {
            let _ = std::fs::remove_file(dir.join(format!("{state}.{other}")));
        }
    }
    Ok(())
}

/// Remove the picture of `<kind>/<state>` (the state then follows its [`chain`]).
pub fn remove_image(app_dir: &Path, id: &str, kind: &str, state: &str) -> Result<(), String> {
    check_slot(kind, state)?;
    load(app_dir, id)?;
    let dir = folder(app_dir, id).join(kind);
    let mut found = false;
    for (ext, _) in EXTS {
        found |= std::fs::remove_file(dir.join(format!("{state}.{ext}"))).is_ok();
    }
    if found { Ok(()) } else { Err("這一格本來就沒有圖".into()) }
}

/// An id for `want` that is not taken: the name itself when it is ASCII and valid, else `char`, `char-2`…
fn free_id(app_dir: &Path, want: &str) -> String {
    let slug: String = want
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { Some(c) } else if c == ' ' { Some('-') } else { None })
        .collect();
    let base = if slug.is_empty() || slug.len() > 40 { "char".to_string() } else { slug };
    let taken = |id: &str| folder(app_dir, id).exists() || id == BUILTIN;
    if !taken(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id)).unwrap_or(base)
}

fn copy_character(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    std::fs::copy(from.join(FILE), to.join(FILE))?;
    if let Some((p, _)) = theme_file(from) {
        std::fs::copy(&p, to.join(p.file_name().unwrap_or_default()))?;
    }
    for kind in [AVATAR, FULL] {
        let Ok(rd) = std::fs::read_dir(from.join(kind)) else { continue };
        for e in rd.flatten().filter(|e| e.path().is_file()) {
            std::fs::create_dir_all(to.join(kind))?;
            std::fs::copy(e.path(), to.join(kind).join(e.file_name()))?;
        }
    }
    Ok(())
}

/// A new character named `name`: a copy of `template` (a character folder, the public example)
/// when there is one, else only a name and the built-in persona.
pub fn create(app_dir: &Path, template: Option<&Path>, name: &str) -> Result<Detail, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("名稱要 1–40 字".into());
    }
    let id = free_id(app_dir, name);
    let to = folder(app_dir, &id);
    match template.filter(|t| t.join(FILE).is_file()) {
        Some(t) => copy_character(t, &to).map_err(|e| e.to_string())?,
        None => {
            let mut t = toml::Table::new();
            t.insert("persona".into(), toml::Value::String(Character::builtin().persona));
            write_atomic(&to.join(FILE), t.to_string().as_bytes()).map_err(|e| e.to_string())?;
        }
    }
    save_meta(app_dir, &id, &MetaPatch { name: Some(name.into()), ..Default::default() })
}

/// The first character folder of the public example (`template` for [`create`]).
pub fn example_template(example_app_dir: &Path) -> Option<PathBuf> {
    let c = list(example_app_dir).into_iter().next()?;
    Some(folder(example_app_dir, &c.id))
}

/// The character as a zip: `<id>/character.toml`, `<id>/avatar/…`, `<id>/full/…`, `<id>/theme.<ext>`.
pub fn export(app_dir: &Path, id: &str) -> Result<Vec<u8>, String> {
    load(app_dir, id)?;
    let dir = folder(app_dir, id);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut add = |rel: String, path: &Path| -> Result<(), String> {
        zip.start_file(format!("{id}/{rel}"), opts).map_err(|e| e.to_string())?;
        zip.write_all(&std::fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    };
    add(FILE.into(), &dir.join(FILE))?;
    if let Some((p, _)) = theme_file(&dir) {
        add(p.file_name().and_then(|n| n.to_str()).unwrap_or("theme").to_string(), &p)?;
    }
    for kind in [AVATAR, FULL] {
        for s in STATES {
            if let Some((p, _)) = picture(&dir, kind, s) {
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("png");
                add(format!("{kind}/{s}.{ext}"), &p)?;
            }
        }
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// A zip made by [`export`] (or a zipped character folder) as a new character. Only
/// `character.toml`, `avatar/<state>.<ext>`, `full/<state>.<ext>` and `theme.<ext>` are read,
/// wherever they sit in the zip; nothing existing is overwritten (a taken id gets `-2`, `-3`…).
pub fn import(app_dir: &Path, zip_bytes: &[u8]) -> Result<Detail, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|_| "這不是 zip 檔".to_string())?;
    if z.len() > 400 {
        return Err("zip 裡的檔案太多".into());
    }
    let mut toml_text: Option<(String, String)> = None; // (folder name, text)
    let mut pics: Vec<(&'static str, String, Vec<u8>)> = vec![];
    let mut song: Option<Vec<u8>> = None;
    let mut total = 0usize;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().replace('\\', "/");
        let parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
        if parts.iter().any(|p| p.starts_with('.') || *p == "__MACOSX") {
            continue;
        }
        if let [.., file] = parts.as_slice()
            && let Some((THEME, ext)) = file.rsplit_once('.')
            && AUDIO_EXTS.iter().any(|(e, _)| *e == ext)
        {
            if f.size() as usize > MAX_AUDIO {
                return Err(format!("{name}：檔案太大"));
            }
            let mut buf = Vec::new();
            f.by_ref().take(MAX_AUDIO as u64 + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            sniff_audio(&buf).map_err(|e| format!("{name}：{e}"))?;
            song = Some(buf);
            continue;
        }
        // (kind, state) for a picture, None for character.toml
        let slot: Option<Option<(&'static str, String)>> = match parts.as_slice() {
            [.., "character.toml"] => Some(None),
            [.., kind @ ("avatar" | "full"), file] => file
                .rsplit_once('.')
                .filter(|(st, ext)| STATES.contains(st) && EXTS.iter().any(|(e, _)| e == ext))
                .map(|(st, _)| Some((if *kind == AVATAR { AVATAR } else { FULL }, st.to_string()))),
            _ => None,
        };
        let Some(slot) = slot else { continue };
        if f.size() as usize > MAX_IMAGE {
            return Err(format!("{name}：檔案太大"));
        }
        let mut buf = Vec::new();
        f.by_ref().take(MAX_IMAGE as u64 + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        total += buf.len();
        if total > 6 * MAX_IMAGE {
            return Err("zip 太大".into());
        }
        match slot {
            None => {
                if toml_text.is_some() {
                    return Err("zip 裡有不只一個人物".into());
                }
                let folder_name = parts.len().checked_sub(2).map(|i| parts[i]).unwrap_or("").to_string();
                toml_text = Some((folder_name, String::from_utf8(buf).map_err(|_| "character.toml 不是 UTF-8".to_string())?));
            }
            Some((kind, state)) => {
                sniff(&buf).map_err(|e| format!("{name}：{e}"))?;
                pics.push((kind, state, buf));
            }
        }
    }
    let (folder_name, text) = toml_text.ok_or("zip 裡沒有 character.toml")?;
    let parsed: File = toml::from_str(&text).map_err(|e| format!("character.toml 讀不懂：{e}"))?;
    let want = if valid_id(&folder_name) { folder_name } else { parsed.name.clone() };
    let id = free_id(app_dir, &want);
    let result = (|| {
        write_atomic(&folder(app_dir, &id).join(FILE), text.as_bytes()).map_err(|e| e.to_string())?;
        for (kind, state, bytes) in &pics {
            save_image(app_dir, &id, kind, state, bytes)?;
        }
        if let Some(bytes) = &song {
            save_theme(app_dir, &id, bytes)?;
        }
        detail(app_dir, &id)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(folder(app_dir, &id));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(toml: &str) -> Character {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(DIR).join("x");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), toml).unwrap();
        load(tmp.path(), "x").unwrap()
    }

    #[test]
    fn figure_defaults_and_limits() {
        assert_eq!(one("name = \"x\"\n").figure, Figure { scale: 1.0, offset_y: 0.0 });
        assert_eq!(one("figure_scale = 1.35\nfigure_offset_y = -0.1\n").figure, Figure { scale: 1.35, offset_y: -0.1 });
        assert_eq!(one("figure_scale = 2\n").figure.scale, 2.0, "an integer is fine");
        assert_eq!(one("figure_scale = 9.0\nfigure_offset_y = 3.0\n").figure, Figure { scale: 3.0, offset_y: 0.5 });
        assert_eq!(one("figure_scale = 0.1\nfigure_offset_y = -3.0\n").figure, Figure { scale: 0.5, offset_y: -0.5 });
        assert_eq!(one("figure_scale = nan\n").figure.scale, 1.0);
        assert_eq!(Character::builtin().figure, Figure::default());
    }

    #[test]
    fn audio_sniffing() {
        let pad = |head: &[u8]| {
            let mut v = head.to_vec();
            v.resize(64, 0);
            v
        };
        assert_eq!(sniff_audio(&pad(b"ID3\x04")), Ok("mp3"));
        assert_eq!(sniff_audio(&pad(&[0xFF, 0xFB, 0x90])), Ok("mp3"));
        assert_eq!(sniff_audio(&pad(&[0xFF, 0xF1, 0x50])), Ok("aac"));
        assert_eq!(sniff_audio(&pad(b"\0\0\0\x20ftypM4A ")), Ok("m4a"));
        assert_eq!(sniff_audio(&pad(b"RIFF\0\0\0\0WAVEfmt ")), Ok("wav"));
        assert_eq!(sniff_audio(&pad(b"OggS\0\x02")), Ok("ogg"));
        let mut opus = pad(b"OggS\0\x02");
        opus[28..36].copy_from_slice(b"OpusHead");
        assert_eq!(sniff_audio(&opus), Ok("opus"));
        assert_eq!(sniff_audio(&pad(b"fLaC")), Ok("flac"));
        assert!(sniff_audio(&pad(b"\x89PNG\r\n\x1a\n")).is_err(), "a picture is not a song");
        assert!(sniff_audio(b"ID3").is_err(), "too short");
        assert!(sniff_audio(&vec![0u8; MAX_AUDIO + 1]).is_err());
    }

    #[test]
    fn voice_rate_theme_and_profile_from_the_file() {
        let c = one("voice_rate = 9.0\ntheme_volume = 3.0\nbackground = \"從海邊來\"\ncatchphrases = [\"嗯哼\"]\n[[examples]]\nuser = \"你好\"\nreply = \"嗨\"\n");
        assert_eq!(c.voice_rate, VOICE_RATE.1);
        assert_eq!(c.theme, None, "no theme file, no theme");
        assert_eq!(c.profile.background, "從海邊來");
        assert_eq!(c.profile.catchphrases, vec!["嗯哼"]);
        assert_eq!(c.profile.examples, vec![Example { user: "你好".into(), reply: "嗨".into() }]);
        assert_eq!(one("name = \"x\"\n").voice_rate, 1.0);
    }

    #[test]
    fn saving_a_profile_writes_only_what_is_filled_in() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(DIR).join("x");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "name = \"x\"\nlikes = \"舊的\"\nkeep = 1\n").unwrap();
        let p = Profile { background: "  身世  ".into(), catchphrases: vec!["a".into(), " ".into()], examples: vec![Example { user: "u".into(), reply: "".into() }], ..Default::default() };
        let d = save_meta(tmp.path(), "x", &MetaPatch { profile: Some(p), voice_rate: Some(1.25), ..Default::default() }).unwrap();
        assert_eq!(d.profile.background, "身世");
        assert_eq!(d.profile.catchphrases, vec!["a"]);
        assert!(d.profile.examples.is_empty(), "a half pair is dropped");
        assert_eq!(d.voice_rate, 1.25);
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(!text.contains("likes") && !text.contains("speech_style") && text.contains("keep = 1"), "{text}");
        let long = Profile { background: "字".repeat(PROFILE_MAX + 1), ..Default::default() };
        assert!(save_meta(tmp.path(), "x", &MetaPatch { profile: Some(long), ..Default::default() }).is_err());
    }

    #[test]
    fn a_wrong_type_is_an_error_like_any_other_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(DIR).join("x");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "figure_scale = \"big\"\n").unwrap();
        assert!(load(tmp.path(), "x").is_err());
    }
}
