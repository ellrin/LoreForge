//! A deck bound to a .pptx where the author keeps it (documents live outside
//! `private/`): every save writes the file back through the lossless exporter.
//!
//! - `decks/<name>/file.json` ([`Bound`]): the path and what SlideComposer last
//!   read from or wrote to it (size, mtime, hash). A file that differs is never
//!   overwritten silently ([`Disk::Changed`]).
//! - Before the first overwrite the original is copied once to
//!   `.<stem>.original.pptx` next to it (hidden in Finder, outside the
//!   wipeable cache, never deleted).
//! - Writes are atomic: `.<file>.sc-tmp` in the same folder, then rename.
//! - Editor-only state PowerPoint has no place for (slots, empty slots,
//!   arrangements, deck defaults, style) travels inside the .pptx as an Office
//!   custom XML part ([`EditorState`]), so a wiped deck folder is rebuilt from
//!   the file alone.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use lore_canvas::{Deck, Object, pptx_in, pptx_out};
use lore_ooxml::Deck as ODeck;
use lore_ooxml::package::{ContentTypes, Package, relative_target, resolve_target};
use lore_ooxml::xml::{Document, Element};
use serde::{Deserialize, Serialize};

use crate::{Error, Result, Session, Workspace, write_atomic};

/// Namespace of the editor-state part (`customXml/itemN.xml`).
pub const STATE_NS: &str = "urn:loreforge:slidecomposer:state";
const REL_CUSTOM_XML: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml";
const REL_CUSTOM_XML_PROPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps";
const CT_CUSTOM_XML_PROPS: &str = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";

/// The .pptx a deck is bound to (`decks/<name>/file.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bound {
    pub path: PathBuf,
    /// Identifies the deck inside the file (editor-state part), so a moved file
    /// finds its deck folder again.
    pub deck_id: String,
    /// Size, mtime (ns) and hash of what SlideComposer last read or wrote.
    pub len: u64,
    pub mtime: u64,
    pub k: u64,
    /// Hash of the deck model last written to the file (pending write when it differs).
    pub deck_k: u64,
    /// The one-time copy of the original, once made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
}

/// The bound file on disk compared with what SlideComposer last read or wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Disk {
    Same,
    Changed,
    Missing,
}

/// What [`Session::write_back`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrote {
    Written,
    /// Nothing to write (unbound, or the file already has this deck).
    Unchanged,
    /// The file changed on disk (or vanished): not written.
    Conflict(Disk),
}

/// For the UI: the bound file and its save state.
#[derive(Debug, Clone, Serialize)]
pub struct FileStatus {
    pub path: PathBuf,
    pub file: String,
    /// `saved` | `pending` | `changed` | `missing`
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
}

/// Editor-only state of a deck, stored in the .pptx (custom XML part).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EditorState {
    pub v: u32,
    pub deck_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slides: Vec<SlideState>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SlideState {
    /// Position in the presentation and `p:sldId/@id` (stable when PowerPoint reorders).
    pub i: usize,
    pub sld: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrangement: Option<String>,
    /// Object id → slot it fills.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slots: BTreeMap<u32, String>,
    /// Empty slots (never exported): their position in the slide's object list and the object.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub empty: Vec<(usize, Object)>,
}

impl EditorState {
    /// From the deck model; `sld[i]` = `p:sldId/@id` of slide i in the exported file.
    pub fn of(deck: &Deck, deck_id: &str, sld: &[u32]) -> Self {
        let slides = deck
            .slides
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let mut st = SlideState { i, sld: sld.get(i).copied().unwrap_or(0), arrangement: s.arrangement.clone(), ..Default::default() };
                fn walk(v: &[Object], out: &mut BTreeMap<u32, String>) {
                    for o in v {
                        if let (Some(sl), false) = (&o.slot, o.is_empty_slot()) {
                            out.insert(o.id, sl.clone());
                        }
                        if let lore_canvas::Kind::Group(g) = &o.kind {
                            walk(&g.children, out);
                        }
                    }
                }
                walk(&s.objects, &mut st.slots);
                st.empty = s.objects.iter().enumerate().filter(|(_, o)| o.is_empty_slot()).map(|(j, o)| (j, o.clone())).collect();
                (st.arrangement.is_some() || !st.slots.is_empty() || !st.empty.is_empty()).then_some(st)
            })
            .collect();
        EditorState { v: 1, deck_id: deck_id.into(), style: deck.style.clone(), defaults: deck.defaults.clone(), slides }
    }

    /// Put the state back onto a deck imported from the file (`sld` as in [`EditorState::of`]).
    /// Slides are matched by slide id, else by position; slots only land on objects that exist.
    pub fn apply(&self, deck: &mut Deck, sld: &[u32]) {
        deck.defaults = self.defaults.clone();
        for st in &self.slides {
            let idx = sld.iter().position(|&x| x != 0 && x == st.sld).or_else(|| (st.i < deck.slides.len() && sld.get(st.i).is_none_or(|&x| x == 0 || st.sld == 0)).then_some(st.i));
            let Some(s) = idx.and_then(|i| deck.slides.get_mut(i)) else { continue };
            if st.arrangement.is_some() {
                s.arrangement = st.arrangement.clone();
            }
            for (id, slot) in &st.slots {
                if let Some(o) = s.find_mut(*id) {
                    o.slot = Some(slot.clone());
                }
            }
            for (at, o) in &st.empty {
                let mut o = o.clone();
                if s.find(o.id).is_some() {
                    o.id = s.max_id() + 1;
                }
                let at = (*at).min(s.objects.len());
                s.objects.insert(at, o);
            }
        }
    }

    fn to_xml(&self) -> Result<Vec<u8>> {
        let json = serde_json::to_string(self)?;
        let root = Element::new("sc:state").with_attr("xmlns:sc", STATE_NS).with_text(json);
        Ok(Document::new(root).to_bytes())
    }
}

/// The editor-state part of a package, if any: (part name, state).
pub fn read_state(pkg: &Package, pres: &str) -> Option<(String, EditorState)> {
    let rels = pkg.rels(pres).ok()?;
    rels.by_type(REL_CUSTOM_XML).find_map(|r| {
        let part = resolve_target(pres, &r.target);
        let doc = Document::parse(pkg.get(&part)?).ok()?;
        if doc.root.local() != "state" || !doc.root.attrs.iter().any(|(_, v)| v == STATE_NS) {
            return None;
        }
        let st: EditorState = serde_json::from_str(&doc.root.text()).ok()?;
        Some((part, st))
    })
}

/// Write `state` into the package: the existing state part is replaced, else a
/// new `customXml/itemN.xml` (+ itemProps) is related from the presentation.
pub fn write_state(pkg: &mut Package, pres: &str, state: &EditorState) -> Result<()> {
    let xml = state.to_xml()?;
    if let Some((part, _)) = read_state(pkg, pres) {
        pkg.set(&part, xml);
        return Ok(());
    }
    let part = pkg.free_name("customXml/item", ".xml");
    let n = part.trim_start_matches("customXml/item").trim_end_matches(".xml").to_string();
    let props = format!("customXml/itemProps{n}.xml");
    pkg.set(&part, xml);
    let guid = {
        let h = lore_canvas::fnv(state.deck_id.as_bytes());
        let g = format!("{h:016X}{:016X}", lore_canvas::fnv(format!("{}{n}", state.deck_id).as_bytes()));
        format!("{{{}-{}-{}-{}-{}}}", &g[0..8], &g[8..12], &g[12..16], &g[16..20], &g[20..32])
    };
    let props_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n<ds:datastoreItem ds:itemID=\"{guid}\" xmlns:ds=\"http://schemas.openxmlformats.org/officeDocument/2006/customXml\"><ds:schemaRefs><ds:schemaRef ds:uri=\"{STATE_NS}\"/></ds:schemaRefs></ds:datastoreItem>"
    );
    pkg.set(&props, props_xml.into_bytes());
    let mut item_rels = pkg.rels(&part)?;
    item_rels.add(REL_CUSTOM_XML_PROPS, &relative_target(&part, &props));
    pkg.set_rels(&part, &item_rels);
    let mut pres_rels = pkg.rels(pres)?;
    pres_rels.add(REL_CUSTOM_XML, &relative_target(pres, &part));
    pkg.set_rels(pres, &pres_rels);
    let mut ct = ContentTypes::load(pkg)?;
    ct.ensure_default("xml", "application/xml");
    ct.set_override(&props, CT_CUSTOM_XML_PROPS);
    ct.store(pkg);
    Ok(())
}

/// `dir/.<stem>.original.pptx` (then `-2`, …): where the one-time backup of `path` goes.
pub fn backup_path(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "deck".into());
    let first = dir.join(format!(".{stem}.original.pptx"));
    if !first.exists() {
        return first;
    }
    (2..).map(|i| dir.join(format!(".{stem}.original-{i}.pptx"))).find(|p| !p.exists()).unwrap()
}

/// Replace `path` atomically: `.<name>.sc-tmp` in the same folder (same
/// volume), synced, permissions of the old file, then rename. On any failure
/// the temp file is removed and `path` is untouched.
pub fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.sc-tmp"));
    let res = (|| -> Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if let Ok(m) = std::fs::metadata(path) {
            std::fs::set_permissions(&tmp, m.permissions())?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn mtime_ns(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

/// Absolute path with the folder resolved (`/tmp` → `/private/tmp` on macOS); the file itself may not exist yet.
pub fn canonical(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    match (path.parent().and_then(|d| d.canonicalize().ok()), path.file_name()) {
        (Some(d), Some(f)) => d.join(f),
        _ => path.to_path_buf(),
    }
}

fn new_deck_id(seed: &str) -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:016x}", lore_canvas::fnv(format!("{seed}:{t}:{}", std::process::id()).as_bytes()))
}

/// `p:sldId/@id` of each slide of a package, in presentation order.
fn slide_ids(od: &ODeck) -> Vec<u32> {
    od.slides.iter().map(|s| s.sld_id).collect()
}

fn deck_k(deck: &Deck) -> u64 {
    serde_json::to_vec(deck).map(|b| lore_canvas::fnv(&b)).unwrap_or(0)
}

impl Bound {
    /// A binding to `path` as it is on disk now (missing file: all zero).
    fn at(path: &Path, deck_id: String, deck_k: u64) -> Self {
        let path = canonical(path);
        let (len, mtime, k) = match (std::fs::metadata(&path), std::fs::read(&path)) {
            (Ok(m), Ok(b)) => (m.len(), mtime_ns(&m), lore_canvas::fnv(&b)),
            _ => (0, 0, 0),
        };
        Bound { path, deck_id, len, mtime, k, deck_k, backup: None }
    }

    pub fn load(deck_dir: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(deck_dir.join("file.json")).ok()?).ok()
    }

    fn store(&self, deck_dir: &Path) -> Result<()> {
        write_atomic(&deck_dir.join("file.json"), &serde_json::to_vec_pretty(self)?)
    }

    /// The file on disk against what SlideComposer last read or wrote.
    pub fn disk(&self) -> Disk {
        let Ok(m) = std::fs::metadata(&self.path) else { return Disk::Missing };
        if m.len() == self.len && mtime_ns(&m) == self.mtime {
            return Disk::Same;
        }
        match std::fs::read(&self.path) {
            Ok(b) if lore_canvas::fnv(&b) == self.k && b.len() as u64 == self.len => Disk::Same,
            Ok(_) => Disk::Changed,
            Err(_) => Disk::Missing,
        }
    }
}

impl Workspace {
    /// Decks bound to a file, with their binding.
    pub fn bound_decks(&self) -> Vec<(String, Bound)> {
        self.deck_names().into_iter().filter_map(|d| Some((d.name.clone(), Bound::load(&self.decks.join(&d.name))?))).collect()
    }

    /// The style whose layouts `od` shares most (none when no style matches).
    fn match_style(&self, od: &ODeck) -> Option<String> {
        let names: std::collections::BTreeSet<&str> = od.layouts.iter().map(|l| l.name()).collect();
        self.style_ids()
            .into_iter()
            .filter_map(|id| {
                let tpl = ODeck::from_bytes(&self.template_bytes(&id).ok()?).ok()?;
                let shared = tpl.layouts.iter().filter(|l| names.contains(l.name())).count();
                (shared > 0).then_some((shared, id))
            })
            .max()
            .map(|(_, id)| id)
    }

    /// Import `bytes` into `dir` (source.pptx + deck.json), restoring the editor
    /// state stored in the file. Returns the deck and the state's deck id.
    fn import_into(&self, dir: &Path, bytes: &[u8], style: Option<&str>) -> Result<(Deck, Option<String>)> {
        let od = ODeck::from_bytes(bytes)?;
        let state = read_state(&od.pkg, &od.pres_part).map(|(_, s)| s);
        let known = |s: &str| self.style_ids().iter().any(|x| x == s);
        let style = state
            .as_ref()
            .and_then(|s| s.style.clone())
            .filter(|s| known(s))
            .or_else(|| style.map(str::to_string))
            .or_else(|| self.match_style(&od));
        let st = match &style {
            Some(s) => Some(self.style(s)?),
            None => None,
        };
        let mut deck = pptx_in::import(&od, st.as_ref(), style.clone());
        if let Some(s) = &state {
            s.apply(&mut deck, &slide_ids(&od));
        }
        std::fs::create_dir_all(dir.join("assets"))?;
        write_atomic(&dir.join("source.pptx"), bytes)?;
        write_atomic(&dir.join("deck.json"), &serde_json::to_vec(&deck)?)?;
        Ok((deck, state.map(|s| s.deck_id).filter(|s| !s.is_empty())))
    }

    /// Open a .pptx where it lives (Finder, 開啟…) and bind the deck to it:
    /// 1. the deck already bound to this path;
    /// 2. the deck whose id the file carries, when its old path is gone (moved file);
    /// 3. an unbound deck imported from the same bytes (before files were bound);
    /// 4. a new import named after the file (editor state restored from the file).
    pub fn open_path(&self, path: &Path) -> Result<String> {
        let path = canonical(path);
        let bound = self.bound_decks();
        if let Some((name, _)) = bound.iter().find(|(_, b)| b.path == path) {
            return Ok(name.clone());
        }
        let bytes = std::fs::read(&path)?;
        let od = ODeck::from_bytes(&bytes)?;
        let file_id = read_state(&od.pkg, &od.pres_part).map(|(_, s)| s.deck_id).filter(|s| !s.is_empty());
        if let Some(id) = &file_id
            && let Some((name, b)) = bound.iter().find(|(_, b)| &b.deck_id == id && !b.path.exists())
        {
            let dir = self.decks.join(name);
            // keep what was last written: edited elsewhere after the move → hash differs → asked
            let mut nb = b.clone();
            nb.path = path.clone();
            nb.mtime = 0;
            nb.store(&dir)?;
            return Ok(name.clone());
        }
        let h = lore_canvas::fnv(&bytes);
        for d in self.deck_names() {
            let dir = self.decks.join(&d.name);
            if Bound::load(&dir).is_some() {
                continue;
            }
            let src = dir.join("source.pptx");
            if std::fs::metadata(&src).map(|m| m.len() == bytes.len() as u64).unwrap_or(false)
                && std::fs::read(&src).map(|b| lore_canvas::fnv(&b) == h).unwrap_or(false)
            {
                let deck = Session::load_deck(&dir)?;
                Bound::at(&path, new_deck_id(&d.name), deck_k(&deck)).store(&dir)?;
                return Ok(d.name);
            }
        }
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "imported".into());
        let stem: String = stem.chars().map(|c| if matches!(c, '/' | '\\') { '-' } else { c }).collect();
        let name = self.free_deck_name(stem.trim_start_matches('.').trim()).or_else(|_| self.free_deck_name("imported"))?;
        let dir = self.decks.join(&name);
        let (deck, id) = self.import_into(&dir, &bytes, None)?;
        // a copy of a file whose deck is still bound elsewhere gets its own id
        let id = id.filter(|i| !bound.iter().any(|(_, b)| &b.deck_id == i)).unwrap_or_else(|| new_deck_id(&name));
        Bound::at(&path, id, deck_k(&deck)).store(&dir)?;
        Ok(name)
    }

    /// 重新載入: replace a bound deck with its file as it is on disk now.
    pub fn reload_file(&self, name: &str) -> Result<()> {
        let dir = self.deck_dir(name)?;
        let b = Bound::load(&dir).ok_or_else(|| Error::Invalid(format!("{name} 沒有對應的檔案")))?;
        let bytes = std::fs::read(&b.path)?;
        let style = Session::load_deck(&dir).ok().and_then(|d| d.style);
        let (deck, _) = self.import_into(&dir, &bytes, style.as_deref())?;
        let mut nb = Bound::at(&b.path, b.deck_id.clone(), deck_k(&deck));
        nb.backup = b.backup;
        nb.store(&dir)
    }
}

impl Session {
    pub(crate) fn load_deck(dir: &Path) -> Result<Deck> {
        Ok(serde_json::from_slice(&std::fs::read(dir.join("deck.json"))?)?)
    }

    /// The deck changed since it was last written to its file.
    pub fn pending(&self) -> bool {
        self.file.as_ref().is_some_and(|b| b.deck_k != deck_k(&self.editor.deck))
    }

    pub fn file_status(&self) -> Option<FileStatus> {
        let b = self.file.as_ref()?;
        let state = match b.disk() {
            Disk::Missing => "missing",
            Disk::Changed => "changed",
            Disk::Same if self.pending() => "pending",
            Disk::Same => "saved",
        };
        Some(FileStatus {
            path: b.path.clone(),
            file: b.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            state,
            backup: b.backup.clone(),
        })
    }

    /// The .pptx bytes for the bound file: the lossless export plus the editor-state part.
    pub fn file_bytes(&self, fallbacks: &std::collections::HashMap<String, Vec<u8>>, deck_id: &str) -> Result<Vec<u8>> {
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
        let (od, keep) = pptx_out::export_parts(&self.editor.deck, &self.source, self.editor.style.as_ref(), &assets)?;
        let mut pkg = od.to_package();
        for p in keep {
            if let Some(b) = self.source.pkg.get(&p) {
                pkg.set(&p, b.to_vec());
            }
        }
        let state = EditorState::of(&self.editor.deck, deck_id, &slide_ids(&od));
        write_state(&mut pkg, &od.pres_part, &state)?;
        Ok(pkg.to_bytes()?)
    }

    /// Write the deck back to its file (autosave). Never over a file that
    /// changed on disk unless `force` (保留我的版本，覆蓋). The original is
    /// copied once before the first overwrite.
    pub fn write_back(&mut self, fallbacks: &std::collections::HashMap<String, Vec<u8>>, force: bool) -> Result<Wrote> {
        let Some(b) = self.file.clone() else { return Ok(Wrote::Unchanged) };
        let disk = b.disk();
        if disk != Disk::Same && !force {
            return Ok(Wrote::Conflict(disk));
        }
        let dk = deck_k(&self.editor.deck);
        if disk == Disk::Same && dk == b.deck_k {
            return Ok(Wrote::Unchanged);
        }
        let bytes = self.file_bytes(fallbacks, &b.deck_id)?;
        let mut nb = b.clone();
        if let Ok(old) = std::fs::read(&b.path) {
            // written by this deck before (state part with its id): the original was already kept
            let ours = ODeck::from_bytes(&old)
                .ok()
                .and_then(|od| read_state(&od.pkg, &od.pres_part))
                .is_some_and(|(_, s)| s.deck_id == b.deck_id);
            if b.backup.is_none() && !ours {
                let to = backup_path(&b.path);
                std::fs::write(&to, &old)?;
                nb.backup = Some(to);
                // recorded before the overwrite: a failed write never makes a second backup
                nb.store(&self.dir)?;
            }
        }
        replace_file(&b.path, &bytes)?;
        let m = std::fs::metadata(&b.path)?;
        nb.len = m.len();
        nb.mtime = mtime_ns(&m);
        nb.k = lore_canvas::fnv(&bytes);
        nb.deck_k = dk;
        nb.store(&self.dir)?;
        self.file = Some(nb);
        Ok(Wrote::Written)
    }

    /// 另存新檔: bind the deck to `path` and write it there (an existing file
    /// there — the save dialog asked — is backed up once first).
    pub fn save_as(&mut self, path: &Path, fallbacks: &std::collections::HashMap<String, Vec<u8>>) -> Result<()> {
        let id = self.file.as_ref().map(|b| b.deck_id.clone()).unwrap_or_else(|| new_deck_id(&self.name));
        let id = if self.file.as_ref().is_some_and(|b| b.path != canonical(path)) { new_deck_id(&self.name) } else { id };
        let b = Bound::at(path, id, 0);
        b.store(&self.dir)?;
        self.file = Some(b);
        self.write_back(fallbacks, true)?;
        Ok(())
    }
}
