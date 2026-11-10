//! sc-core: SlideComposer's workspace and deck folders.
//!
//! ```text
//! <workspace>/                    SLIDECOMPOSER_WORKSPACE (default ../private/SlideComposer)
//! ├── styles/<id>/style.toml      style packs (+ template.pptx)
//! └── decks/<name>/               SLIDECOMPOSER_DECKS overrides this folder
//!     ├── deck.json               the canvas model (source of truth)
//!     ├── source.pptx             the imported .pptx or the style template (never modified)
//!     ├── assets/                 inserted images (content-addressed)
//!     ├── exports/                exported .pptx / .pdf (never overwritten)
//!     └── cache/latex.json        OMML → LaTeX conversions
//! ```

use std::collections::HashMap;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use lore_canvas::base::Base;
use lore_canvas::pptx_in::{self, Backdrop};
use lore_canvas::{Deck, Editor, StyleSystem, pptx_out};
use lore_ooxml::Deck as ODeck;
use lore_ooxml::style::StylePack;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("檔案讀寫失敗：{0}")]
    Io(#[from] std::io::Error),
    #[error("無法讀取 PPTX：{0}")]
    Ooxml(#[from] lore_ooxml::Error),
    #[error("匯出失敗：{0}")]
    Export(#[from] pptx_out::ExportError),
    #[error("資料格式錯誤：{0}")]
    Json(#[from] serde_json::Error),
    #[error("方程式：{0}")]
    Math(#[from] lore_math::Error),
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

    /// `SLIDECOMPOSER_WORKSPACE` (default `<repo>/../private/SlideComposer`), `SLIDECOMPOSER_DECKS`.
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

    /// The canvas base (`styles/<id>/base.json`, design §7.2 v2) when the style has
    /// one; colour roles resolve against the style's palette colours.
    pub fn base(&self, id: &str) -> Result<Option<Base>> {
        let st = self.style(id)?;
        let path = self.styles_dir().join(id).join("base.json");
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)?;
        Base::from_json(&text, &st.pack.palette.colors).map(Some).map_err(Error::Invalid)
    }

    /// Template bytes: compiled from base.json when present (nothing is written
    /// into the style folder), else the style's template.pptx.
    pub fn template_bytes(&self, id: &str) -> Result<Vec<u8>> {
        match self.base(id)? {
            Some(b) => {
                let st = self.style(id)?;
                let dir = self.styles_dir().join(id);
                let assets = move |a: &str| style_asset(&dir, a);
                Ok(lore_canvas::base::compile(&b, &st.pack.palette.colors, &assets).map_err(|e| Error::Invalid(e.to_string()))?)
            }
            None => Ok(std::fs::read(self.template(id)?)?),
        }
    }

    /// What a style looks like (for previews): its cover and content page as a
    /// small deck (layout backdrops, title placeholders, starter objects).
    /// Pictures keep `pkg:` (inside the template) or `style:` (style folder) ids.
    pub fn style_sample(&self, id: &str) -> Result<StyleSample> {
        let st = self.style(id)?;
        let tpl = self.template_bytes(id)?;
        let od = ODeck::from_bytes(&tpl)?;
        let base = self.base(id)?;
        let mut slides = Vec::new();
        for sem in ["cover", "content"] {
            let Some(layout) = st.pack.layouts.get(sem) else { continue };
            let mut objs = layout_titles(&od, Some(&st), layout);
            if let Some(page) = base.as_ref().and_then(|b| b.pages.values().find(|p| &p.layout == layout)) {
                let mut starter = page.starter.clone();
                starter.extend(objs);
                objs = starter;
            }
            let mut s = lore_canvas::Slide::new(format!("s-{sem}"), Some(layout.clone()));
            s.objects = objs;
            slides.push(s);
        }
        Ok(StyleSample { size: lore_canvas::Size { w: od.slide_size.0, h: od.slide_size.1 }, backdrops: pptx_in::backdrops(&od), slides })
    }

    /// A picture a style preview refers to: `pkg:<part>` inside the template, `style:<file>` in the style folder.
    pub fn style_asset(&self, id: &str, asset: &str) -> Option<Vec<u8>> {
        if let Some(f) = asset.strip_prefix("style:") {
            return style_asset(&self.styles_dir().join(id), &format!("file:{f}"));
        }
        let part = asset.strip_prefix("pkg:")?;
        let tpl = self.template_bytes(id).ok()?;
        lore_ooxml::package::Package::from_bytes(&tpl).ok()?.get(part).map(<[u8]>::to_vec)
    }

    /// Named arrangements (版型) the author saved: `arrangements/*.json` in the
    /// workspace (private), in file-name order. Each is free JSON the editor
    /// interprets (`split`, `regions`, …); unreadable files are skipped.
    pub fn arrangements(&self) -> Vec<serde_json::Value> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(self.root.join("arrangements"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        files
            .iter()
            .filter_map(|p| {
                let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
                if v.get("id").is_none() {
                    v["id"] = serde_json::json!(p.file_stem()?.to_string_lossy());
                }
                Some(v)
            })
            .collect()
    }

    /// Save a named arrangement as `arrangements/<id>.json` (atomic; an
    /// existing file of the same id is replaced — it is the same arrangement).
    pub fn save_arrangement(&self, v: &serde_json::Value) -> Result<String> {
        let id = v.get("id").and_then(|x| x.as_str()).ok_or_else(|| Error::Invalid("版型缺少 id".into()))?.to_string();
        check_name(&id)?;
        let dir = self.root.join("arrangements");
        std::fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(format!("{id}.json")), &serde_json::to_vec_pretty(v)?)?;
        Ok(id)
    }

    /// Where arrangements go on each page kind: title, kicker, content and
    /// takeaway areas (EMU). From the canvas base when the style has one,
    /// otherwise from the layout's title placeholder with plain margins.
    pub fn page_areas(&self, style: Option<&StyleSystem>, source: &ODeck) -> Vec<serde_json::Value> {
        let (w, h) = source.slide_size;
        let base = style.and_then(|st| self.base(&st.pack.pack.id).ok().flatten());
        let mut out = Vec::new();
        let kinds: Vec<(String, String)> = match style {
            Some(st) => st.pack.layouts.iter().map(|(k, l)| (k.clone(), l.clone())).collect(),
            None => source.layouts.iter().take(1).map(|l| ("content".to_string(), l.name().to_string())).collect(),
        };
        for (kind, layout) in kinds {
            if let Some(p) = base.as_ref().and_then(|b| b.pages.get(&kind)) {
                out.push(serde_json::json!({ "kind": kind, "layout": layout, "title": p.title, "kicker": p.kicker, "content": p.content, "takeaway": p.takeaway }));
                continue;
            }
            let titles = layout_titles(source, style, &layout);
            let title = titles.iter().find(|o| o.ph.as_deref() == Some("title") || o.ph.as_deref() == Some("ctrTitle")).map(|o| o.frame.clone());
            let mx = w / 20;
            let top = title.as_ref().map(|t| t.y + t.h + h / 30).unwrap_or(h / 6);
            let content = lore_canvas::Frame::new(mx, top, w - 2 * mx, (h * 9 / 10 - top).max(h / 4));
            out.push(serde_json::json!({ "kind": kind, "layout": layout, "title": title, "kicker": null, "content": content, "takeaway": null }));
        }
        out
    }

    /// Objects a new slide on `layout` starts with: the layout's title
    /// placeholders, then the base's starter objects for that page kind (style
    /// pictures are copied into the deck's assets).
    pub fn new_slide_objects(&self, style: Option<&StyleSystem>, source: &ODeck, layout: &str, deck_dir: &Path) -> Result<Vec<lore_canvas::Object>> {
        let mut objs = layout_titles(source, style, layout);
        let Some(st) = style else { return Ok(objs) };
        let id = st.pack.pack.id.clone();
        let Some(base) = self.base(&id)? else { return Ok(objs) };
        let Some(page) = base.pages.values().find(|p| p.layout == layout) else { return Ok(objs) };
        // an imported deck keeps its own layouts: without a title placeholder there,
        // the base's title area becomes a plain title box
        if objs.is_empty() && let Some(f) = &page.title {
            let tag = if page.layout == st.pack.layouts.get("content").cloned().unwrap_or_default() { "title" } else { "cover-title" };
            let look = st.look(tag, None).or_else(|| st.look("title", None)).unwrap_or_default();
            let mut p = lore_canvas::Para::new(vec![]);
            p.end = Some(look);
            objs.push(lore_canvas::Object {
                id: 2,
                name: "Title".into(),
                frame: f.clone(),
                hidden: false,
                ph: None,
                slot: Some("title".into()),
                kind: lore_canvas::Kind::Text(lore_canvas::SpObj {
                    geom: lore_canvas::Geom::preset("rect"),
                    fill: lore_canvas::Paint::None,
                    line: None,
                    text: Some(lore_canvas::TextBody { insets: [0, 0, 0, 0], anchor: "ctr".into(), paras: vec![p], ..Default::default() }),
                    equation: false,
                }),
                src: None,
            });
        }
        let mut next = objs.iter().map(|o| o.id).max().unwrap_or(1) + 1;
        let mut starter = page.starter.clone();
        // starters go under the title placeholders
        for o in &mut starter {
            o.id = next;
            next += 1;
            if let lore_canvas::Kind::Picture(pic) = &mut o.kind
                && let Some(f) = pic.asset.strip_prefix("style:")
            {
                let bytes = style_asset(&self.styles_dir().join(&id), &format!("file:{f}")).ok_or_else(|| Error::NotFound(format!("底版圖片 {f}")))?;
                let ext = f.rsplit('.').next().unwrap_or("png");
                pic.asset = store_asset(deck_dir, &bytes, ext)?;
            }
        }
        starter.extend(objs);
        objs = starter;
        Ok(objs)
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

    /// Open a .pptx from disk (Finder double-click): the deck already imported from
    /// the same bytes, else a new import named after the file, on the style whose
    /// layouts this file shares most (none when no style matches).
    pub fn open_pptx_file(&self, path: &Path) -> Result<String> {
        let bytes = std::fs::read(path)?;
        let h = lore_canvas::fnv(&bytes);
        for d in self.deck_names() {
            let src = self.decks.join(&d.name).join("source.pptx");
            if std::fs::metadata(&src).map(|m| m.len() == bytes.len() as u64).unwrap_or(false)
                && std::fs::read(&src).map(|b| lore_canvas::fnv(&b) == h).unwrap_or(false)
            {
                return Ok(d.name);
            }
        }
        let od = ODeck::from_bytes(&bytes)?;
        let names: std::collections::BTreeSet<&str> = od.layouts.iter().map(|l| l.name()).collect();
        let style = self
            .style_ids()
            .into_iter()
            .filter_map(|id| {
                let tpl = ODeck::from_bytes(&self.template_bytes(&id).ok()?).ok()?;
                let shared = tpl.layouts.iter().filter(|l| names.contains(l.name())).count();
                (shared > 0).then_some((shared, id))
            })
            .max()
            .map(|(_, id)| id);
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "imported".into());
        self.import_pptx(&stem, &bytes, style.as_deref())
    }

    /// New deck from a style pack's template: one cover slide and one content slide.
    pub fn new_deck(&self, name: &str, style: &str) -> Result<String> {
        check_name(name)?;
        let st = self.style(style)?;
        let tpl = self.template_bytes(style)?;
        let od = ODeck::from_bytes(&tpl)?;
        let mut deck = Deck::new(od.slide_size.0, od.slide_size.1, Some(style.into()));
        let name = self.free_deck_name(name)?;
        let dir = self.decks.join(&name);
        std::fs::create_dir_all(dir.join("assets"))?;
        for sem in ["cover", "content"] {
            let layout = st.pack.layouts.get(sem).cloned();
            let uid = deck.new_uid();
            let mut s = lore_canvas::Slide::new(uid, layout.clone());
            if let Some(l) = &layout {
                s.objects = self.new_slide_objects(Some(&st), &od, l, &dir)?;
            }
            deck.slides.push(s);
        }
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
pub struct StyleSample {
    pub size: lore_canvas::Size,
    pub backdrops: Vec<Backdrop>,
    pub slides: Vec<lore_canvas::Slide>,
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
        let h = lore_canvas::fnv(bytes);
        let rel = format!("assets/{h:016x}.{ext}");
        let p = self.dir.join(&rel);
        std::fs::create_dir_all(self.dir.join("assets"))?;
        if !p.exists() {
            std::fs::write(&p, bytes)?;
        }
        Ok(format!("file:{rel}"))
    }

    pub fn export_bytes(&self) -> Result<Vec<u8>> {
        self.export_bytes_with(&HashMap::new())
    }

    /// Export with fallback pictures for equation boxes (key:
    /// `pptx_out::equation_fallback_key`, value: PNG bytes), rendered by the
    /// caller before taking the session lock.
    pub fn export_bytes_with(&self, fallbacks: &HashMap<String, Vec<u8>>) -> Result<Vec<u8>> {
        let dir = self.dir.clone();
        let assets = move |p: &str| -> Option<Vec<u8>> {
            if p.starts_with("fallback:") {
                return fallbacks.get(p).cloned();
            }
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
        self.export_pptx_with(&HashMap::new())
    }

    pub fn export_pptx_with(&self, fallbacks: &HashMap<String, Vec<u8>>) -> Result<PathBuf> {
        let bytes = self.export_bytes_with(fallbacks)?;
        let dir = self.dir.join("exports");
        std::fs::create_dir_all(&dir)?;
        let p = free_path(&dir, &self.name, "pptx");
        std::fs::write(&p, bytes)?;
        Ok(p)
    }

    /// Cached LaTeX for an OMML string.
    pub fn latex_for(&mut self, omml: &str, pandoc: &lore_math::Pandoc) -> Result<String> {
        let key = format!("{:016x}", lore_canvas::fnv(omml.as_bytes()));
        if let Some(l) = self.latex.get(&key) {
            return Ok(l.clone());
        }
        let el = lore_ooxml::xml::parse_element(omml.as_bytes())?;
        let l = pandoc.omml_to_latex(&el)?;
        self.latex.insert(key, l.clone());
        std::fs::create_dir_all(self.dir.join("cache"))?;
        write_atomic(&self.dir.join("cache/latex.json"), &serde_json::to_vec_pretty(&self.latex)?)?;
        Ok(l)
    }
}

/// LaTeX → PowerPoint OMML (`a14:m`) as an XML string.
pub fn latex_to_omml(pandoc: &lore_math::Pandoc, latex: &str, display: bool, size_pt: f64) -> Result<String> {
    let el = pandoc.latex_to_pptx_math(latex, display, size_pt)?;
    Ok(el.to_xml_string())
}

/// Placeholder objects (titles) of a layout, empty, tagged from the style pack.
pub fn layout_titles(source: &ODeck, st: Option<&StyleSystem>, layout: &str) -> Vec<lore_canvas::Object> {
    use lore_canvas::{Kind, Object};
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
        let frame = lore_canvas::Frame::new(x.off.unwrap_or((0, 0)).0, x.off.unwrap_or((0, 0)).1, x.ext.unwrap_or((0, 0)).0, x.ext.unwrap_or((0, 0)).1);
        let look = st.and_then(|st| st.look(tag, None)).unwrap_or_default();
        let mut p = lore_canvas::Para::new(vec![]);
        p.end = Some(look);
        let tb = lore_canvas::TextBody {
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
            slot: None,
            kind: Kind::Text(lore_canvas::SpObj {
                geom: lore_canvas::Geom::preset("rect"),
                fill: lore_canvas::Paint::None,
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

/// Extra Chrome arguments from `SLIDECOMPOSER_CHROME_ARGS` (space-separated),
/// e.g. `--no-sandbox` in a container that runs as root. Empty by default:
/// the sandbox stays on.
fn chrome_extra_args() -> Vec<String> {
    std::env::var("SLIDECOMPOSER_CHROME_ARGS").map(|v| v.split_whitespace().map(str::to_string).collect()).unwrap_or_default()
}

/// Run headless Chrome once with `args` and wait at most `limit`.
///
/// - No `--user-data-dir`: headless Chrome then makes and removes its own
///   temporary profile. With an explicit profile folder Chrome 153 opens a
///   normal (invisible) browser window and never runs `--screenshot` /
///   `--print-to-pdf`, so the call hung forever.
/// - stdin/stdout closed, stderr to a file (no pipe a helper process could
///   keep open); its own process group, so on timeout (and afterwards) the
///   whole tree — renderers, GPU process, crash handler — is killed.
///
/// Returns the last line Chrome wrote to stderr (for error messages).
fn run_chrome(chrome: &Path, args: &[String], limit: std::time::Duration) -> Result<String> {
    use std::process::Stdio;
    let log = std::env::temp_dir().join(format!(
        "sc-chrome-{}-{}.log",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
    ));
    let mut cmd = Command::new(chrome);
    cmd.arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-component-update")
        .arg("--disable-sync")
        .args(chrome_extra_args())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).map(Stdio::from).unwrap_or_else(|_| Stdio::null()));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break Some(st);
        }
        if start.elapsed() >= limit {
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    kill_tree(&mut child);
    let last = std::fs::read_to_string(&log)
        .ok()
        .and_then(|t| t.lines().filter(|l| !l.contains("dbus")).last().map(str::to_string))
        .unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    if status.is_none() {
        return Err(Error::Invalid(format!("Chrome 超過 {} 秒沒有完成，已停止", limit.as_secs())));
    }
    Ok(last)
}

/// Kill Chrome and everything it started (its process group on Unix).
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: plain kill(2) on the process group created for this child.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Screenshot `url` (the app's single-object route) at `w`×`h` CSS pixels,
/// twice the resolution, transparent background, into `out`; at most 30 s.
/// **Not part of the offline tests** (needs a browser).
pub fn screenshot_png(chrome: &Path, url: &str, w: u32, h: u32, out: &Path) -> Result<()> {
    let args = [
        "--hide-scrollbars".to_string(),
        "--force-device-scale-factor=2".into(),
        "--default-background-color=00000000".into(),
        "--virtual-time-budget=10000".into(),
        format!("--window-size={w},{h}"),
        format!("--screenshot={}", out.display()),
        url.into(),
    ];
    let last = run_chrome(chrome, &args, std::time::Duration::from_secs(30))?;
    if !out.is_file() {
        return Err(Error::Invalid(format!("Chrome 沒有產生圖片：{last}")));
    }
    Ok(())
}

/// Print `url` (the app's print route) to `out` with headless Chrome; at most
/// 3 minutes. **Not part of the offline tests** (needs a browser).
pub fn print_pdf(chrome: &Path, url: &str, out: &Path) -> Result<()> {
    let args = [
        "--no-pdf-header-footer".to_string(),
        "--virtual-time-budget=20000".into(),
        "--run-all-compositor-stages-before-draw".into(),
        format!("--print-to-pdf={}", out.display()),
        url.into(),
    ];
    let last = run_chrome(chrome, &args, std::time::Duration::from_secs(180))?;
    if !out.is_file() {
        return Err(Error::Invalid(format!("Chrome 沒有產生 PDF：{last}")));
    }
    Ok(())
}

/// A file inside a style folder named by an asset id (`file:<relative path>`).
fn style_asset(dir: &Path, asset: &str) -> Option<Vec<u8>> {
    let f = asset.strip_prefix("file:")?;
    if f.contains("..") || f.starts_with('/') {
        return None;
    }
    std::fs::read(dir.join(f)).ok()
}

/// Store bytes in a deck's `assets/` (content-addressed, same names as
/// `Session::add_asset`). Returns `file:assets/<hash>.<ext>`.
pub fn store_asset(deck_dir: &Path, bytes: &[u8], ext: &str) -> Result<String> {
    let ext: String = ext.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    if ext.is_empty() {
        return Err(Error::Invalid("缺少副檔名".into()));
    }
    let rel = format!("assets/{:016x}.{ext}", lore_canvas::fnv(bytes));
    let path = deck_dir.join(&rel);
    if !path.exists() {
        std::fs::create_dir_all(deck_dir.join("assets"))?;
        write_atomic(&path, bytes)?;
    }
    Ok(format!("file:{rel}"))
}
