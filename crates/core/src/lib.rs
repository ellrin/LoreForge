//! sc-core: SlideComposer's workspace and deck folders.
//!
//! ```text
//! <workspace>/                    SLIDECOMPOSER_WORKSPACE (default ../private)
//! ├── styles/<id>/style.toml      style packs (+ template.pptx)
//! └── decks/<name>/               SLIDECOMPOSER_DECKS overrides this folder
//!     ├── deck.json               the canvas model (source of truth)
//!     ├── source.pptx             the imported .pptx or the style template (never modified)
//!     ├── assets/                 inserted images (content-addressed)
//!     ├── exports/                exported .pptx / .pdf (never overwritten)
//!     └── cache/latex.json        OMML → LaTeX conversions
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use ym_canvas::pptx_in::{self, Backdrop};
use ym_canvas::{Deck, Editor, StyleSystem, pptx_out};
use ym_ooxml::Deck as ODeck;
use ym_ooxml::style::StylePack;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("檔案讀寫失敗：{0}")]
    Io(#[from] std::io::Error),
    #[error("無法讀取 PPTX：{0}")]
    Ooxml(#[from] ym_ooxml::Error),
    #[error("匯出失敗：{0}")]
    Export(#[from] pptx_out::ExportError),
    #[error("資料格式錯誤：{0}")]
    Json(#[from] serde_json::Error),
    #[error("方程式：{0}")]
    Math(#[from] ym_math::Error),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Where styles and decks live.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub decks: PathBuf,
}

impl Workspace {
    pub fn new(root: impl Into<PathBuf>, decks: Option<PathBuf>) -> Self {
        let root = root.into();
        let decks = decks.unwrap_or_else(|| root.join("decks"));
        Workspace { root, decks }
    }

    /// `SLIDECOMPOSER_WORKSPACE` (default `<repo>/../private`), `SLIDECOMPOSER_DECKS`.
    pub fn from_env(default_root: &Path) -> Self {
        let root = std::env::var("SLIDECOMPOSER_WORKSPACE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_root.to_path_buf());
        let decks = std::env::var("SLIDECOMPOSER_DECKS").ok().map(PathBuf::from);
        Self::new(root, decks)
    }

    pub fn styles_dir(&self) -> PathBuf {
        self.root.join("styles")
    }

    pub fn style_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.styles_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("style.toml").is_file())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        v.sort();
        v
    }

    pub fn style(&self, id: &str) -> Result<StyleSystem> {
        check_name(id)?;
        let pack = StylePack::load_dir(&self.styles_dir().join(id))?;
        Ok(StyleSystem::new(pack))
    }

    pub fn template(&self, id: &str) -> Result<PathBuf> {
        let st = self.style(id)?;
        Ok(self.styles_dir().join(id).join(&st.pack.pack.template))
    }

    pub fn deck_dir(&self, name: &str) -> Result<PathBuf> {
        check_name(name)?;
        Ok(self.decks.join(name))
    }

    pub fn deck_names(&self) -> Vec<DeckInfo> {
        let mut v: Vec<DeckInfo> = std::fs::read_dir(&self.decks)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("deck.json").is_file())
            .map(|e| {
                let p = e.path().join("deck.json");
                let modified = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                DeckInfo {
                    name: e.file_name().to_string_lossy().to_string(),
                    modified,
                }
            })
            .collect();
        v.sort_by(|a, b| b.modified.cmp(&a.modified));
        v
    }

    /// A deck folder name not yet used (`name`, `name-2`, …).
    pub fn free_deck_name(&self, name: &str) -> Result<String> {
        check_name(name)?;
        let mut n = name.to_string();
        let mut i = 2;
        while self.decks.join(&n).exists() {
            n = format!("{name}-{i}");
            i += 1;
        }
        Ok(n)
    }

    /// Import a .pptx into a new deck folder. Returns the deck name used.
    pub fn import_pptx(&self, name: &str, bytes: &[u8], style: Option<&str>) -> Result<String> {
        check_name(name)?;
        let od = ODeck::from_bytes(bytes)?;
        let st = match style {
            Some(s) => Some(self.style(s)?),
            None => None,
        };
        let deck = pptx_in::import(&od, st.as_ref(), style.map(str::to_string));
        let name = self.free_deck_name(name)?;
        let dir = self.decks.join(&name);
        std::fs::create_dir_all(dir.join("assets"))?;
        std::fs::write(dir.join("source.pptx"), bytes)?;
        write_atomic(&dir.join("deck.json"), &serde_json::to_vec(&deck)?)?;
        Ok(name)
    }

    /// New deck from a style pack's template: one cover slide and one content slide.
    pub fn new_deck(&self, name: &str, style: &str) -> Result<String> {
        check_name(name)?;
        let st = self.style(style)?;
        let tpl = std::fs::read(self.template(style)?)?;
        let od = ODeck::from_bytes(&tpl)?;
        let mut deck = Deck::new(od.slide_size.0, od.slide_size.1, Some(style.into()));
        for sem in ["cover", "content"] {
            let layout = st.pack.layouts.get(sem).cloned();
            let uid = deck.new_uid();
            let mut s = ym_canvas::Slide::new(uid, layout.clone());
            if let Some(l) = &layout {
                s.objects = layout_titles(&od, Some(&st), l);
            }
            deck.slides.push(s);
        }
        let name = self.free_deck_name(name)?;
        let dir = self.decks.join(&name);
        std::fs::create_dir_all(dir.join("assets"))?;
        std::fs::write(dir.join("source.pptx"), &tpl)?;
        write_atomic(&dir.join("deck.json"), &serde_json::to_vec(&deck)?)?;
        Ok(name)
    }

    pub fn open(&self, name: &str) -> Result<Session> {
        let dir = self.deck_dir(name)?;
        if !dir.join("deck.json").is_file() {
            return Err(Error::NotFound(format!("找不到簡報 {name}")));
        }
        let deck: Deck = serde_json::from_slice(&std::fs::read(dir.join("deck.json"))?)?;
        let source = ODeck::open(dir.join("source.pptx"))?;
        let style = match &deck.style {
            Some(s) => self.style(s).ok(),
            None => None,
        };
        let backdrops = pptx_in::backdrops(&source);
        let latex: BTreeMap<String, String> = std::fs::read(dir.join("cache/latex.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ok(Session {
            name: name.into(),
            dir,
            editor: Editor::new(deck, style),
            source,
            backdrops,
            latex,
            saved_rev: 0,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeckInfo {
    pub name: String,
    pub modified: u64,
}

fn check_name(n: &str) -> Result<()> {
    if n.is_empty() || n.starts_with('.') || n.contains(['/', '\\']) || n.contains("..") {
        return Err(Error::Invalid(format!("名稱不合法：{n}")));
    }
    Ok(())
}

/// Write via a temp file + rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp~");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// `dir/stem.ext`, or `dir/stem-2.ext` … — never an existing file.
pub fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let p = dir.join(format!("{stem}.{ext}"));
    if !p.exists() {
        return p;
    }
    (2..)
        .map(|i| dir.join(format!("{stem}-{i}.{ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

/// An open deck.
pub struct Session {
    pub name: String,
    pub dir: PathBuf,
    pub editor: Editor,
    pub source: ODeck,
    pub backdrops: Vec<Backdrop>,
    /// OMML fingerprint → LaTeX.
    pub latex: BTreeMap<String, String>,
    pub saved_rev: u64,
}

pub fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "wdp" => "image/vnd.ms-photo",
        _ => "application/octet-stream",
    }
}

impl Session {
    pub fn deck(&self) -> &Deck {
        &self.editor.deck
    }

    pub fn save(&mut self) -> Result<()> {
        write_atomic(&self.dir.join("deck.json"), &serde_json::to_vec(&self.editor.deck)?)?;
        self.saved_rev = self.editor.rev;
        Ok(())
    }

    pub fn dirty(&self) -> bool {
        self.saved_rev != self.editor.rev
    }

    /// Bytes of an asset (`pkg:<part>` from source.pptx, `file:<path>` in the deck folder).
    pub fn asset(&self, id: &str) -> Option<(Vec<u8>, &'static str)> {
        if let Some(p) = id.strip_prefix("pkg:") {
            return self.source.pkg.get(p).map(|b| (b.to_vec(), mime_of(p)));
        }
        if let Some(f) = id.strip_prefix("file:") {
            if f.contains("..") || f.starts_with('/') {
                return None;
            }
            return std::fs::read(self.dir.join(f)).ok().map(|b| (b, mime_of(f)));
        }
        None
    }

    /// Store an inserted image (content-addressed). Returns `file:assets/<hash>.<ext>`.
    pub fn add_asset(&self, bytes: &[u8], ext: &str) -> Result<String> {
        let ext: String = ext.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
        if ext.is_empty() {
            return Err(Error::Invalid("缺少副檔名".into()));
        }
        let h = ym_canvas::fnv(bytes);
        let rel = format!("assets/{h:016x}.{ext}");
        let p = self.dir.join(&rel);
        std::fs::create_dir_all(self.dir.join("assets"))?;
        if !p.exists() {
            std::fs::write(&p, bytes)?;
        }
        Ok(format!("file:{rel}"))
    }

    pub fn export_bytes(&self) -> Result<Vec<u8>> {
        let dir = self.dir.clone();
        let assets = move |p: &str| -> Option<Vec<u8>> {
            if p.contains("..") {
                return None;
            }
            std::fs::read(dir.join(p)).ok()
        };
        Ok(pptx_out::export_bytes(
            &self.editor.deck,
            &self.source,
            self.editor.style.as_ref(),
            &assets,
        )?)
    }

    /// Export to `exports/<name>.pptx` (free name). Returns the path.
    pub fn export_pptx(&self) -> Result<PathBuf> {
        let bytes = self.export_bytes()?;
        let dir = self.dir.join("exports");
        std::fs::create_dir_all(&dir)?;
        let p = free_path(&dir, &self.name, "pptx");
        std::fs::write(&p, bytes)?;
        Ok(p)
    }

    /// Cached LaTeX for an OMML string.
    pub fn latex_for(&mut self, omml: &str, pandoc: &ym_math::Pandoc) -> Result<String> {
        let key = format!("{:016x}", ym_canvas::fnv(omml.as_bytes()));
        if let Some(l) = self.latex.get(&key) {
            return Ok(l.clone());
        }
        let el = ym_ooxml::xml::parse_element(omml.as_bytes())?;
        let l = pandoc.omml_to_latex(&el)?;
        self.latex.insert(key, l.clone());
        std::fs::create_dir_all(self.dir.join("cache"))?;
        write_atomic(&self.dir.join("cache/latex.json"), &serde_json::to_vec_pretty(&self.latex)?)?;
        Ok(l)
    }
}

/// LaTeX → PowerPoint OMML (`a14:m`) as an XML string.
pub fn latex_to_omml(pandoc: &ym_math::Pandoc, latex: &str, display: bool, size_pt: f64) -> Result<String> {
    let el = pandoc.latex_to_pptx_math(latex, display, size_pt)?;
    Ok(el.to_xml_string())
}

/// Placeholder objects (titles) of a layout, empty, tagged from the style pack.
pub fn layout_titles(source: &ODeck, st: Option<&StyleSystem>, layout: &str) -> Vec<ym_canvas::Object> {
    use ym_canvas::{Kind, Object};
    let Some(l) = source.layout_by_name(layout) else { return vec![] };
    let mut out = Vec::new();
    for (ph, sh) in l.doc.placeholders() {
        let (tag, kind) = match ph.kind() {
            "title" => ("title", "title"),
            "ctrTitle" => ("cover-title", "ctrTitle"),
            "subTitle" => ("subtitle", "subTitle"),
            _ => continue,
        };
        let Some(x) = sh.xfrm() else { continue };
        let frame = ym_canvas::Frame::new(x.off.unwrap_or((0, 0)).0, x.off.unwrap_or((0, 0)).1, x.ext.unwrap_or((0, 0)).0, x.ext.unwrap_or((0, 0)).1);
        let look = st.and_then(|st| st.look(tag, None)).unwrap_or_default();
        let mut p = ym_canvas::Para::new(vec![]);
        p.end = Some(look);
        let tb = ym_canvas::TextBody {
            anchor: if kind == "title" { "ctr".into() } else { "t".into() },
            autofit: "norm".into(),
            paras: vec![p],
            ..Default::default()
        };
        out.push(Object {
            id: (out.len() + 2) as u32,
            name: sh.nv().map(|n| n.name().to_string()).unwrap_or_default(),
            frame,
            hidden: false,
            ph: Some(kind.into()),
            kind: Kind::Text(ym_canvas::SpObj {
                geom: ym_canvas::Geom::preset("rect"),
                fill: ym_canvas::Paint::None,
                line: None,
                text: Some(tb),
                equation: false,
            }),
            src: None,
        });
    }
    out
}

/// Headless Chrome/Chromium for PDF export, if installed.
pub fn find_chrome() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SLIDECOMPOSER_CHROME") {
        return Some(p.into());
    }
    [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// Print `url` (the app's print route) to `out` with headless Chrome.
/// **Not part of the offline tests** (needs a browser).
pub fn print_pdf(chrome: &Path, url: &str, out: &Path) -> Result<()> {
    let profile = std::env::temp_dir().join(format!("sc-chrome-{}", std::process::id()));
    let st = Command::new(chrome)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-pdf-header-footer")
        .arg("--virtual-time-budget=20000")
        .arg("--run-all-compositor-stages-before-draw")
        .arg(format!("--print-to-pdf={}", out.display()))
        .arg(url)
        .output()?;
    let _ = std::fs::remove_dir_all(&profile);
    if !out.is_file() {
        return Err(Error::Invalid(format!(
            "Chrome 沒有產生 PDF：{}",
            String::from_utf8_lossy(&st.stderr).lines().last().unwrap_or("")
        )));
    }
    Ok(())
}
