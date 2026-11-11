//! Characters (design §6): `private/LoreEcho/characters/<id>/`
//!
//! ```text
//! character.toml   name, persona (the system prompt's personality), voice (later), language,
//!                  figure_scale, figure_offset_y (how big the full body stands in the open window)
//! avatar/          <state>.png .svg .webp .jpg   (portrait: the bubble, the name tag)
//! full/            the same states, full body (head to feet, transparent background): the open window
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
    language: String,
    figure_scale: Option<f64>,
    figure_offset_y: Option<f64>,
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
    /// Placeholder for the voice phase (design §7).
    pub voice: String,
    pub language: String,
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
            language: "zh-TW".into(),
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
        language: Some(f.language.trim()).filter(|l| !l.is_empty()).unwrap_or("zh-TW").into(),
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

/// Everything the 人物 tab edits (persona included).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Detail {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub voice: String,
}

pub fn detail(app_dir: &Path, id: &str) -> Result<Detail, String> {
    let c = load(app_dir, id)?;
    Ok(Detail { id: c.id, name: c.name, persona: c.persona, voice: c.voice })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetaPatch {
    pub name: Option<String>,
    pub persona: Option<String>,
    pub voice: Option<String>,
}

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

/// The character as a zip: `<id>/character.toml`, `<id>/avatar/…`, `<id>/full/…`.
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
/// `character.toml`, `avatar/<state>.<ext>` and `full/<state>.<ext>` are read, wherever they sit in
/// the zip; nothing existing is overwritten (a taken id gets `-2`, `-3`…).
pub fn import(app_dir: &Path, zip_bytes: &[u8]) -> Result<Detail, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|_| "這不是 zip 檔".to_string())?;
    if z.len() > 400 {
        return Err("zip 裡的檔案太多".into());
    }
    let mut toml_text: Option<(String, String)> = None; // (folder name, text)
    let mut pics: Vec<(&'static str, String, Vec<u8>)> = vec![];
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
    fn a_wrong_type_is_an_error_like_any_other_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(DIR).join("x");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "figure_scale = \"big\"\n").unwrap();
        assert!(load(tmp.path(), "x").is_err());
    }
}
