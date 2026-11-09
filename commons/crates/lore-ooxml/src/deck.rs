//! A presentation: raw package + typed slides/notes/layouts/masters/themes.
//!
//! Save = every untouched part byte-for-byte + slide/notes XML regenerated from
//! the typed model.

use std::collections::{BTreeSet, VecDeque};
use std::path::Path;

use serde::Serialize;

use crate::model::*;
use crate::package::*;
use crate::xml::{Attrs, Document, Element, Node};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct NotesEntry {
    pub part: String,
    pub doc: SlideDoc,
}

#[derive(Debug, Clone)]
pub struct SlideEntry {
    pub part: String,
    /// `p:sldId/@id` in presentation.xml.
    pub sld_id: u32,
    pub doc: SlideDoc,
    pub rels: Relationships,
    pub layout: Option<String>,
    pub notes: Option<NotesEntry>,
}

#[derive(Debug, Clone)]
pub struct LayoutEntry {
    pub part: String,
    pub doc: SlideDoc,
    pub master: Option<String>,
    pub rels: Relationships,
}

impl LayoutEntry {
    pub fn name(&self) -> &str {
        self.doc.name().unwrap_or("")
    }
    /// Layout `type` attribute (title, obj, cust …).
    pub fn kind(&self) -> &str {
        self.doc.attrs.get("type").unwrap_or("cust")
    }
}

#[derive(Debug, Clone)]
pub struct MasterEntry {
    pub part: String,
    pub doc: SlideDoc,
    pub theme: Option<String>,
    /// `p:txStyles` title/body/other.
    pub title_style: Option<ListStyle>,
    pub body_style: Option<ListStyle>,
    pub other_style: Option<ListStyle>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Theme {
    pub part: String,
    pub name: String,
    /// dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink -> hex.
    pub colors: Vec<(String, String)>,
    pub major_latin: String,
    pub major_ea: String,
    pub minor_latin: String,
    pub minor_ea: String,
    /// Script-specific fonts (`Hant` -> 新細明體 …) for minor font.
    pub minor_scripts: Vec<(String, String)>,
    pub major_scripts: Vec<(String, String)>,
}

impl Theme {
    pub fn parse(part: &str, d: &Document) -> Self {
        let mut t = Theme {
            part: part.into(),
            name: d.root.attr("name").unwrap_or_default().into(),
            ..Default::default()
        };
        if let Some(te) = d.root.child("a:themeElements") {
            if let Some(cs) = te.child("a:clrScheme") {
                for c in cs.elements() {
                    let hex = c
                        .elements()
                        .next()
                        .map(Color::parse)
                        .and_then(|c| c.hex())
                        .unwrap_or_default();
                    t.colors.push((c.local().to_string(), hex));
                }
            }
            if let Some(fs) = te.child("a:fontScheme") {
                let read = |f: Option<&Element>| -> (String, String, Vec<(String, String)>) {
                    let Some(f) = f else {
                        return Default::default();
                    };
                    let tf = |n: &str| {
                        f.child(n)
                            .and_then(|e| e.attr("typeface"))
                            .unwrap_or_default()
                            .to_string()
                    };
                    let scripts = f
                        .elements()
                        .filter(|e| e.local() == "font")
                        .map(|e| {
                            (
                                e.attr("script").unwrap_or_default().to_string(),
                                e.attr("typeface").unwrap_or_default().to_string(),
                            )
                        })
                        .collect();
                    (tf("a:latin"), tf("a:ea"), scripts)
                };
                (t.major_latin, t.major_ea, t.major_scripts) = read(fs.child("a:majorFont"));
                (t.minor_latin, t.minor_ea, t.minor_scripts) = read(fs.child("a:minorFont"));
            }
        }
        t
    }
    pub fn color(&self, name: &str) -> Option<&str> {
        self.colors
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, h)| h.as_str())
    }
    /// Resolve `+mj-lt`, `+mn-ea` … theme font references.
    pub fn resolve_font(&self, f: &str) -> String {
        let hant =
            |v: &Vec<(String, String)>| v.iter().find(|(s, _)| s == "Hant").map(|(_, t)| t.clone());
        match f {
            "+mj-lt" => self.major_latin.clone(),
            "+mn-lt" => self.minor_latin.clone(),
            "+mj-ea" => {
                if self.major_ea.is_empty() {
                    hant(&self.major_scripts).unwrap_or_default()
                } else {
                    self.major_ea.clone()
                }
            }
            "+mn-ea" => {
                if self.minor_ea.is_empty() {
                    hant(&self.minor_scripts).unwrap_or_default()
                } else {
                    self.minor_ea.clone()
                }
            }
            other => other.to_string(),
        }
    }
}

/// Effective text style of a run after placeholder / master inheritance.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ResolvedRun {
    pub latin: Option<String>,
    pub ea: Option<String>,
    pub size_pt: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// Hex when resolvable (scheme colours via theme + clrMap, lumMod/lumOff/shade/tint applied).
    pub color: Option<String>,
    /// Alpha (1.0 opaque) of the colour.
    pub alpha: Option<f64>,
    #[serde(skip)]
    pending: Option<Color>,
}

impl ResolvedRun {
    fn absorb(&mut self, r: &RunProps) {
        if self.latin.is_none() {
            self.latin = r.latin_font().map(str::to_string);
        }
        if self.ea.is_none() {
            self.ea = r.ea_font().map(str::to_string);
        }
        if self.size_pt.is_none() {
            self.size_pt = r.size_pt();
        }
        if self.bold.is_none() {
            self.bold = r.bold();
        }
        if self.italic.is_none() {
            self.italic = r.italic();
        }
        if self.pending.is_none()
            && let Some(c) = r.color()
        {
            self.pending = Some(c.clone());
        }
    }
}

/// Apply lumMod/lumOff (HSL luminance), shade and tint to an RGB hex.
pub fn apply_color_mods(hex: &str, c: &Color) -> String {
    let v = u32::from_str_radix(hex, 16).unwrap_or(0);
    let mut rgb = [
        ((v >> 16) & 255) as f64 / 255.0,
        ((v >> 8) & 255) as f64 / 255.0,
        (v & 255) as f64 / 255.0,
    ];
    let val = |n: &str| {
        c.mods
            .iter()
            .find(|m| m.local() == n)
            .and_then(|m| m.attrs.get_i64("val"))
            .map(|x| x as f64 / 100_000.0)
    };
    if let Some(s) = val("shade") {
        rgb = rgb.map(|x| x * s);
    }
    if let Some(t) = val("tint") {
        rgb = rgb.map(|x| x + (1.0 - x) * (1.0 - t));
    }
    let (lm, lo) = (val("lumMod"), val("lumOff"));
    if lm.is_some() || lo.is_some() {
        let (h, s, l) = rgb_to_hsl(rgb);
        let l = (l * lm.unwrap_or(1.0) + lo.unwrap_or(0.0)).clamp(0.0, 1.0);
        rgb = hsl_to_rgb(h, s, l);
    }
    let b = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
    format!("{:02X}{:02X}{:02X}", b(rgb[0]), b(rgb[1]), b(rgb[2]))
}

fn rgb_to_hsl([r, g, b]: [f64; 3]) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-12 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s == 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

#[derive(Debug, Clone)]
pub struct Deck {
    pub pkg: Package,
    pub pres_part: String,
    /// Slide size in EMU (cx, cy).
    pub slide_size: (i64, i64),
    pub slides: Vec<SlideEntry>,
    pub layouts: Vec<LayoutEntry>,
    pub masters: Vec<MasterEntry>,
    pub themes: Vec<Theme>,
    pub default_text_style: Option<ListStyle>,
}

fn rel_targets(pkg: &Package, part: &str, rel_type: &str) -> Result<Vec<(String, String)>> {
    let rels = pkg.rels(part)?;
    Ok(rels
        .by_type(rel_type)
        .map(|r| (r.id.clone(), resolve_target(part, &r.target)))
        .collect())
}

impl Deck {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_package(Package::open(path)?)
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        Self::from_package(Package::from_bytes(b)?)
    }

    pub fn from_package(pkg: Package) -> Result<Self> {
        let root_rels = pkg.rels("")?;
        let pres_part = root_rels
            .items
            .iter()
            .find(|r| r.rel_type.ends_with("/officeDocument"))
            .map(|r| resolve_target("", &r.target))
            .ok_or_else(|| Error::MissingPart("officeDocument relationship".into()))?;
        let pres = pkg.xml(&pres_part)?;
        let pres_rels = pkg.rels(&pres_part)?;
        let slide_size = pres
            .root
            .child("p:sldSz")
            .map(|e| {
                (
                    e.attrs.get_i64("cx").unwrap_or(9_144_000),
                    e.attrs.get_i64("cy").unwrap_or(5_143_500),
                )
            })
            .unwrap_or((9_144_000, 5_143_500));
        let default_text_style = pres.root.child("p:defaultTextStyle").map(ListStyle::parse);

        // masters, layouts, themes
        let mut masters = Vec::new();
        let mut layouts = Vec::new();
        let mut themes: Vec<Theme> = Vec::new();
        for r in pres_rels.by_type(REL_SLIDE_MASTER) {
            let mpart = resolve_target(&pres_part, &r.target);
            let mdoc_raw = pkg.xml(&mpart)?;
            let mdoc = SlideDoc::from_document(&mdoc_raw);
            let tx = mdoc_raw.root.child("p:txStyles");
            let st = |n: &str| tx.and_then(|t| t.child(n)).map(ListStyle::parse);
            let theme = rel_targets(&pkg, &mpart, REL_THEME)?
                .into_iter()
                .next()
                .map(|(_, t)| t);
            if let Some(t) = &theme
                && !themes.iter().any(|x| &x.part == t)
            {
                themes.push(Theme::parse(t, &pkg.xml(t)?));
            }
            for (_, lpart) in rel_targets(&pkg, &mpart, REL_SLIDE_LAYOUT)? {
                layouts.push(LayoutEntry {
                    doc: SlideDoc::parse(
                        pkg.get(&lpart)
                            .ok_or_else(|| Error::MissingPart(lpart.clone()))?,
                    )?,
                    rels: pkg.rels(&lpart)?,
                    master: Some(mpart.clone()),
                    part: lpart,
                });
            }
            masters.push(MasterEntry {
                part: mpart,
                title_style: st("p:titleStyle"),
                body_style: st("p:bodyStyle"),
                other_style: st("p:otherStyle"),
                doc: mdoc,
                theme,
            });
        }
        // keep layouts in part-number order for stable listings
        layouts.sort_by_key(|l| part_number(&l.part));

        // slides in presentation order
        let mut slides = Vec::new();
        if let Some(lst) = pres.root.child("p:sldIdLst") {
            for s in lst.elements() {
                let rid = s.attr("r:id").unwrap_or_default();
                let sld_id = s.attr("id").and_then(|v| v.parse().ok()).unwrap_or(0);
                let rel = pres_rels
                    .get(rid)
                    .ok_or_else(|| Error::MissingPart(format!("presentation rel {rid}")))?;
                let part = resolve_target(&pres_part, &rel.target);
                let doc = SlideDoc::parse(
                    pkg.get(&part)
                        .ok_or_else(|| Error::MissingPart(part.clone()))?,
                )?;
                let rels = pkg.rels(&part)?;
                let layout = rels
                    .by_type(REL_SLIDE_LAYOUT)
                    .next()
                    .map(|r| resolve_target(&part, &r.target));
                let notes = match rels.by_type(REL_NOTES_SLIDE).next() {
                    Some(r) => {
                        let np = resolve_target(&part, &r.target);
                        Some(NotesEntry {
                            doc: SlideDoc::parse(
                                pkg.get(&np).ok_or_else(|| Error::MissingPart(np.clone()))?,
                            )?,
                            part: np,
                        })
                    }
                    None => None,
                };
                slides.push(SlideEntry {
                    part,
                    sld_id,
                    doc,
                    rels,
                    layout,
                    notes,
                });
            }
        }
        Ok(Deck {
            pkg,
            pres_part,
            slide_size,
            slides,
            layouts,
            masters,
            themes,
            default_text_style,
        })
    }

    /// Package with every slide/notes part regenerated from the typed model.
    pub fn to_package(&self) -> Package {
        let mut pkg = self.pkg.clone();
        for s in &self.slides {
            pkg.set(&s.part, s.doc.to_bytes());
            if let Some(n) = &s.notes {
                pkg.set(&n.part, n.doc.to_bytes());
            }
        }
        pkg
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.to_package().to_bytes()
    }

    /// Save to a new file (never overwrites).
    pub fn save_new(&self, path: impl AsRef<Path>) -> Result<()> {
        self.to_package().save_new(path)
    }

    pub fn layout(&self, part: &str) -> Option<&LayoutEntry> {
        self.layouts.iter().find(|l| l.part == part)
    }
    pub fn layout_by_name(&self, name: &str) -> Option<&LayoutEntry> {
        self.layouts.iter().find(|l| l.name() == name)
    }
    pub fn master(&self, part: &str) -> Option<&MasterEntry> {
        self.masters.iter().find(|m| m.part == part)
    }
    pub fn slide_layout(&self, slide: usize) -> Option<&LayoutEntry> {
        self.slides
            .get(slide)?
            .layout
            .as_deref()
            .and_then(|p| self.layout(p))
    }
    pub fn theme_for_layout(&self, l: &LayoutEntry) -> Option<&Theme> {
        let m = self.master(l.master.as_deref()?)?;
        self.themes
            .iter()
            .find(|t| Some(&t.part) == m.theme.as_ref())
    }
    pub fn theme(&self) -> Option<&Theme> {
        self.themes.first()
    }

    /// Resolve any colour to hex via the first master's clrMap + theme, with transforms.
    pub fn resolve_color(&self, c: &Color) -> Option<String> {
        let base = match c.hex() {
            Some(h) => h,
            None if c.kind() == "schemeClr" => {
                let name = c.val().unwrap_or("");
                let mapped = self
                    .masters
                    .first()
                    .and_then(|m| m.doc.clr_map.as_ref())
                    .and_then(|cm| cm.attr(name))
                    .unwrap_or(name);
                self.theme()?.color(mapped)?.to_string()
            }
            None => return None,
        };
        Some(apply_color_mods(&base, c))
    }

    // ---------------------------------------------------------------------
    // Placeholder inheritance
    // ---------------------------------------------------------------------

    /// Matching placeholder shapes on the layout and master for a slide placeholder.
    pub fn inherited<'a>(
        &'a self,
        slide: usize,
        ph: &Placeholder,
    ) -> (Option<&'a Shape>, Option<&'a Shape>) {
        let Some(layout) = self.slide_layout(slide) else {
            return (None, None);
        };
        let lsh = find_ph(&layout.doc, ph);
        let master = layout.master.as_deref().and_then(|m| self.master(m));
        let msh = master.and_then(|m| {
            let key = lsh
                .and_then(|s| s.nv())
                .and_then(|n| n.ph.clone())
                .unwrap_or_else(|| ph.clone());
            find_ph_by_type(&m.doc, master_type(key.kind()))
        });
        (lsh, msh)
    }

    /// Effective transform: own, else layout's, else master's.
    pub fn effective_xfrm(&self, slide: usize, shape: &Shape) -> Option<Xfrm> {
        if let Some(x) = shape.xfrm()
            && x.off.is_some()
        {
            return Some(x.clone());
        }
        let ph = shape.nv()?.ph.as_ref()?;
        let (l, m) = self.inherited(slide, ph);
        l.and_then(|s| s.xfrm())
            .filter(|x| x.off.is_some())
            .or_else(|| m.and_then(|s| s.xfrm()))
            .cloned()
    }

    /// Effective font/size/bold/colour of a run (`run` = None for the paragraph end).
    pub fn resolve_run(
        &self,
        slide: usize,
        sp: &Sp,
        para: &Paragraph,
        run: Option<&RunProps>,
    ) -> ResolvedRun {
        let mut out = ResolvedRun::default();
        if let Some(r) = run {
            out.absorb(r);
        }
        let lvl = para.level();
        let lvl_props = |ls: Option<&ListStyle>| -> Option<RunProps> {
            ls.and_then(|l| l.level(lvl))
                .and_then(|p| p.def_rpr.clone())
        };
        if let Some(r) = lvl_props(sp.text.as_ref().and_then(|t| t.list_style.as_ref())) {
            out.absorb(&r);
        }
        let ph = sp.nv.ph.as_ref();
        let layout = self.slide_layout(slide);
        let master = layout
            .and_then(|l| l.master.as_deref())
            .and_then(|m| self.master(m))
            .or(self.masters.first());
        if let Some(ph) = ph {
            let (l, m) = self.inherited(slide, ph);
            for s in [l, m].into_iter().flatten() {
                if let Shape::Sp(s) = s
                    && let Some(r) = lvl_props(s.text.as_ref().and_then(|t| t.list_style.as_ref()))
                {
                    out.absorb(&r);
                }
            }
            if let Some(m) = master {
                let st = match master_type(ph.kind()) {
                    "title" => m.title_style.as_ref(),
                    "body" => m.body_style.as_ref(),
                    _ => m.other_style.as_ref(),
                };
                if let Some(r) = lvl_props(st) {
                    out.absorb(&r);
                }
            }
        } else {
            // shape style font colour (p:style/a:fontRef) beats the defaults
            if out.pending.is_none()
                && let Some(c) = sp
                    .style
                    .as_ref()
                    .and_then(|s| s.child("a:fontRef"))
                    .and_then(|f| f.elements().next())
            {
                out.pending = Some(Color::parse(c));
            }
            let ds = self.default_text_style.as_ref();
            if let Some(r) = lvl_props(ds) {
                out.absorb(&r);
            }
            if let Some(r) = ds
                .and_then(|d| d.default_level())
                .and_then(|p| p.def_rpr.clone())
            {
                out.absorb(&r);
            }
        }
        // theme fonts & colours
        let theme = layout
            .and_then(|l| self.theme_for_layout(l))
            .or(self.theme());
        if let Some(t) = theme {
            out.latin = Some(t.resolve_font(out.latin.as_deref().unwrap_or("+mn-lt")));
            out.ea = Some(t.resolve_font(out.ea.as_deref().unwrap_or("+mn-ea")));
        }
        if let Some(c) = out.pending.take() {
            let base = match c.hex() {
                Some(h) => Some(h),
                None if c.kind() == "schemeClr" => {
                    let name = c.val().unwrap_or("");
                    let mapped = master
                        .and_then(|m| m.doc.clr_map.as_ref())
                        .and_then(|cm| cm.attr(name))
                        .unwrap_or(name);
                    theme.and_then(|t| t.color(mapped)).map(str::to_string)
                }
                None => None,
            };
            out.alpha = Some(c.alpha());
            out.color = Some(match base {
                Some(h) => apply_color_mods(&h, &c),
                None => format!("{}:{}", c.kind(), c.val().unwrap_or("")),
            });
        }
        if out.size_pt.is_none() {
            out.size_pt = Some(18.0);
        }
        out
    }

    // ---------------------------------------------------------------------
    // Building decks
    // ---------------------------------------------------------------------

    /// Style template: masters/layouts/theme/media only, zero slides, no notes,
    /// comments, thumbnail or document properties carrying content.
    pub fn template_package(&self) -> Result<Package> {
        let mut pkg = self.pkg.clone();
        let skip = [
            REL_SLIDE,
            REL_NOTES_SLIDE,
            REL_THUMBNAIL,
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentAuthors",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments",
        ];
        // drop slide rels from presentation, thumbnail rel from root
        for part in [self.pres_part.as_str(), ""] {
            let mut rels = pkg.rels(part)?;
            let n = rels.items.len();
            rels.items.retain(|r| !skip.contains(&r.rel_type.as_str()));
            if rels.items.len() != n {
                pkg.set_rels(part, &rels);
            }
        }
        // reachability from the root
        let mut keep: BTreeSet<String> = BTreeSet::new();
        let mut q: VecDeque<String> = VecDeque::from(vec![String::new()]);
        while let Some(p) = q.pop_front() {
            for r in pkg.rels(&p)?.items {
                if r.external || skip.contains(&r.rel_type.as_str()) {
                    continue;
                }
                let t = resolve_target(&p, &r.target);
                if pkg.has(&t) && keep.insert(t.clone()) {
                    q.push_back(t);
                }
            }
        }
        let removed: Vec<String> = pkg
            .names()
            .filter(|n| *n != ContentTypes::PART && !n.ends_with(".rels") && !keep.contains(*n))
            .map(str::to_string)
            .collect();
        let mut ct = ContentTypes::load(&pkg)?;
        for n in &removed {
            pkg.remove(n);
            pkg.remove(&rels_path(n));
            ct.remove_override(n);
        }
        // drop orphan rels files
        let orphan: Vec<String> = pkg
            .names()
            .filter(|n| n.ends_with(".rels") && *n != "_rels/.rels")
            .filter(|n| {
                let owner = n.replace("/_rels/", "/");
                let owner = owner.trim_end_matches(".rels");
                !pkg.has(owner)
            })
            .map(str::to_string)
            .collect();
        for n in orphan {
            pkg.remove(&n);
        }
        // presentation.xml: no slide list, no sections / custom shows
        let mut pres = pkg.xml(&self.pres_part)?;
        pres.root.children.retain(|c| match c {
            Node::Element(e) => !matches!(e.local(), "sldIdLst" | "custShowLst"),
            _ => true,
        });
        if let Some(ext) = pres.root.child_mut("p:extLst") {
            ext.children.retain(|c| match c {
                Node::Element(e) => !e.contains("p14:sectionLst"),
                _ => true,
            });
        }
        pkg.set(&self.pres_part, pres.to_bytes());
        // viewProps outline view lists slides by r:id
        let vp = pkg
            .names()
            .find(|n| n.ends_with("viewProps.xml"))
            .map(str::to_string);
        if let Some(vp) = vp {
            let mut d = pkg.xml(&vp)?;
            d.root
                .children
                .retain(|c| !matches!(c, Node::Element(e) if e.local() == "outlineViewPr"));
            pkg.set(&vp, d.to_bytes());
        }
        // neutral document properties
        if pkg.has("docProps/app.xml") {
            pkg.set("docProps/app.xml", APP_XML.as_bytes().to_vec());
        }
        if pkg.has("docProps/core.xml") {
            pkg.set("docProps/core.xml", CORE_XML.as_bytes().to_vec());
        }
        ct.store(&mut pkg);
        Ok(pkg)
    }

    /// Add a slide on the layout with this `cSld` name (or layout `type`).
    /// Placeholders of the layout (except date/footer/slide number) are
    /// instantiated empty. Returns the slide index.
    pub fn add_slide(&mut self, layout_name: &str) -> Result<usize> {
        let layout = self
            .layouts
            .iter()
            .find(|l| l.name() == layout_name)
            .or_else(|| self.layouts.iter().find(|l| l.kind() == layout_name))
            .ok_or_else(|| Error::Invalid(format!("no layout named {layout_name:?}")))?
            .clone();
        let part = self.pkg.free_name("ppt/slides/slide", ".xml");
        let mut doc = SlideDoc::parse(EMPTY_SLIDE.as_bytes())?;
        let mut next_id = 2;
        for (ph, sh) in layout.doc.placeholders() {
            if matches!(ph.kind(), "dt" | "ftr" | "sldNum") {
                continue;
            }
            let name = sh.nv().map(|n| n.name().to_string()).unwrap_or_default();
            let mut nv = NvProps::new("p:nvSpPr", "p:cNvSpPr", next_id, &name);
            nv.kind.locks = Some(Element::new("a:spLocks").with_attr("noGrp", "1"));
            nv.ph = Some(Placeholder {
                attrs: ph.attrs.clone(),
                children: vec![],
            });
            let mut tb = TextBody::new("p:txBody");
            tb.paragraphs.push(Paragraph::default());
            doc.tree.children.push(Shape::Sp(Sp {
                attrs: Attrs::new(),
                nv,
                props: ShapeProps::new("p:spPr"),
                style: None,
                text: Some(tb),
                extras: Extras::default(),
            }));
            next_id += 1;
        }
        let mut rels = Relationships::default();
        rels.add(REL_SLIDE_LAYOUT, &relative_target(&part, &layout.part));
        self.pkg.set_rels(&part, &rels);
        let mut ct = ContentTypes::load(&self.pkg)?;
        ct.set_override(&part, CT_SLIDE);
        ct.store(&mut self.pkg);
        // presentation rel + sldId
        let mut prels = self.pkg.rels(&self.pres_part)?;
        let rid = prels.add(REL_SLIDE, &relative_target(&self.pres_part, &part));
        self.pkg.set_rels(&self.pres_part, &prels);
        let sld_id = self
            .slides
            .iter()
            .map(|s| s.sld_id)
            .max()
            .unwrap_or(255)
            .max(255)
            + 1;
        let mut pres = self.pkg.xml(&self.pres_part)?;
        if pres.root.child("p:sldIdLst").is_none() {
            let pos = pres
                .root
                .children
                .iter()
                .position(|c| matches!(c, Node::Element(e) if e.local() == "sldSz"))
                .unwrap_or(pres.root.children.len());
            pres.root
                .children
                .insert(pos, Node::Element(Element::new("p:sldIdLst")));
        }
        pres.root.child_mut("p:sldIdLst").unwrap().push(
            Element::new("p:sldId")
                .with_attr("id", sld_id.to_string())
                .with_attr("r:id", rid),
        );
        self.pkg.set(&self.pres_part, pres.to_bytes());
        self.pkg.set(&part, doc.to_bytes());
        self.slides.push(SlideEntry {
            part,
            sld_id,
            doc,
            rels,
            layout: Some(layout.part.clone()),
            notes: None,
        });
        Ok(self.slides.len() - 1)
    }

    /// Copy slide `idx` of `src` (shapes, notes not included) to the end of this
    /// deck on the layout with the same name, copying media and other related
    /// parts (identical parts are shared; new ones get free names). Relationship
    /// ids are kept so the slide XML stays valid.
    pub fn import_slide(&mut self, src: &Deck, idx: usize) -> Result<usize> {
        let s = src
            .slides
            .get(idx)
            .ok_or_else(|| Error::Invalid(format!("no slide {idx}")))?;
        let layout_name = src
            .slide_layout(idx)
            .map(|l| l.name().to_string())
            .unwrap_or_default();
        let new = self.add_slide(&layout_name)?;
        let part = self.slides[new].part.clone();
        let layout_part = self.slides[new].layout.clone().unwrap_or_default();
        let src_ct = ContentTypes::load(&src.pkg)?;
        let mut ct = ContentTypes::load(&self.pkg)?;
        let mut rels = Relationships::default();
        for r in &s.rels.items {
            if r.rel_type == REL_SLIDE_LAYOUT {
                rels.items.push(Relationship {
                    target: relative_target(&part, &layout_part),
                    ..r.clone()
                });
                continue;
            }
            if r.rel_type == REL_NOTES_SLIDE {
                continue;
            }
            if r.external {
                rels.items.push(r.clone());
                continue;
            }
            let sp = resolve_target(&s.part, &r.target);
            let Some(bytes) = src.pkg.get(&sp) else {
                continue;
            };
            // same part already here (identical bytes, any name) -> share it
            let existing = if self.pkg.get(&sp) == Some(bytes) {
                Some(sp.clone())
            } else {
                self.pkg
                    .parts
                    .iter()
                    .find(|p| p.data.len() == bytes.len() && p.data == bytes)
                    .map(|p| p.name.clone())
            };
            let dst = if let Some(e) = existing {
                e
            } else if !self.pkg.has(&sp) {
                // keep the source name when it is free
                self.pkg.set(&sp, bytes.to_vec());
                let ext = sp.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
                if let Some(t) = src_ct.content_type_of(&sp) {
                    if src_ct.has_override(&sp) {
                        ct.set_override(&sp, &t);
                    } else {
                        ct.ensure_default(ext, &t);
                    }
                }
                sp.clone()
            } else {
                let (stem, ext) = sp.rsplit_once('.').unwrap_or((&sp, ""));
                let stem = stem.trim_end_matches(|c: char| c.is_ascii_digit());
                let name = self.pkg.free_name(stem, &format!(".{ext}"));
                self.pkg.set(&name, bytes.to_vec());
                if let Some(t) = src_ct.content_type_of(&sp) {
                    if src_ct.has_override(&sp) {
                        ct.set_override(&name, &t);
                    } else {
                        ct.ensure_default(ext, &t);
                    }
                }
                name
            };
            rels.items.push(Relationship {
                target: relative_target(&part, &dst),
                ..r.clone()
            });
        }
        ct.store(&mut self.pkg);
        self.pkg.set_rels(&part, &rels);
        let mut doc = s.doc.clone();
        doc.decl = self.slides[new].doc.decl.clone();
        self.slides[new].doc = doc;
        self.slides[new].rels = rels;
        Ok(new)
    }

    /// Store an image in `ppt/media/` and relate it to the slide. Returns the rel id.
    pub fn add_image(&mut self, slide: usize, bytes: Vec<u8>, ext: &str) -> Result<String> {
        let media = self.pkg.free_name("ppt/media/image", &format!(".{ext}"));
        self.pkg.set(&media, bytes);
        let mut ct = ContentTypes::load(&self.pkg)?;
        let mime = match ext {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            "emf" => "image/x-emf",
            _ => "application/octet-stream",
        };
        ct.ensure_default(ext, mime);
        ct.store(&mut self.pkg);
        let s = self
            .slides
            .get_mut(slide)
            .ok_or_else(|| Error::Invalid(format!("no slide {slide}")))?;
        let rid = s.rels.add(REL_IMAGE, &relative_target(&s.part, &media));
        self.pkg.set_rels(&s.part.clone(), &s.rels.clone());
        Ok(rid)
    }

    /// Fill a placeholder's text (first placeholder of this type) with plain paragraphs.
    pub fn set_placeholder_text(
        &mut self,
        slide: usize,
        ph_type: &str,
        lines: &[&str],
    ) -> Result<()> {
        let s = self
            .slides
            .get_mut(slide)
            .ok_or_else(|| Error::Invalid(format!("no slide {slide}")))?;
        for sh in s.doc.shapes_mut() {
            if let Shape::Sp(sp) = sh
                && sp.nv.ph.as_ref().map(|p| p.kind()) == Some(ph_type)
            {
                let tb = sp.text.get_or_insert_with(|| TextBody::new("p:txBody"));
                tb.paragraphs = lines
                    .iter()
                    .map(|l| Paragraph {
                        props: None,
                        runs: vec![Run::text(l)],
                        end_props: None,
                    })
                    .collect();
                return Ok(());
            }
        }
        Err(Error::Invalid(format!(
            "slide {slide} has no {ph_type} placeholder"
        )))
    }
}

fn part_number(p: &str) -> u32 {
    let digits: String = p
        .chars()
        .rev()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits
        .chars()
        .rev()
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// Placeholder type on the master that a layout/slide placeholder inherits from.
pub fn master_type(kind: &str) -> &str {
    match kind {
        "title" | "ctrTitle" => "title",
        "dt" | "ftr" | "sldNum" | "hdr" | "sldImg" => kind,
        _ => "body",
    }
}

fn find_ph<'a>(doc: &'a SlideDoc, ph: &Placeholder) -> Option<&'a Shape> {
    let all = doc.placeholders();
    if let Some(idx) = ph.idx()
        && let Some((_, s)) = all.iter().find(|(p, _)| p.idx() == Some(idx))
    {
        return Some(s);
    }
    all.iter()
        .find(|(p, _)| p.kind() == ph.kind())
        .or_else(|| {
            all.iter()
                .find(|(p, _)| master_type(p.kind()) == master_type(ph.kind()))
        })
        .map(|(_, s)| *s)
}

fn find_ph_by_type<'a>(doc: &'a SlideDoc, t: &str) -> Option<&'a Shape> {
    doc.placeholders()
        .into_iter()
        .find(|(p, _)| master_type(p.kind()) == t)
        .map(|(_, s)| s)
}

const EMPTY_SLIDE: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    "\r\n",
    r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">"#,
    r#"<p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>"#,
    r#"<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#,
    r#"</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#
);

const APP_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    "\r\n",
    r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Application>lore-ooxml</Application><Slides>0</Slides></Properties>"#
);

const CORE_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    "\r\n",
    r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title></dc:title></cp:coreProperties>"#
);
