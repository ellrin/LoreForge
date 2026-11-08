//! Typed slide model.
//!
//! Every typed struct keeps (a) all attributes of its element in an ordered
//! [`Attrs`] (typed accessors read/write them), and (b) every child element it does
//! not model in [`Extras`], remembered with its original position. Writing an
//! unedited model therefore reproduces the source element exactly (canonically).
//! Whole objects the model cannot edit are kept as [`Shape::Locked`].

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::xml::{Attrs, Document, Element, Node};

pub const EMU_PER_PT: i64 = 12_700;
pub const EMU_PER_INCH: i64 = 914_400;

// ---------------------------------------------------------------------------
// Extras: unknown children with their original position
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Extras(pub Vec<(usize, Element)>);

impl Extras {
    fn push(&mut self, i: usize, e: &Element) {
        self.0.push((i, e.clone()));
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &Element> {
        self.0.iter().map(|(_, e)| e)
    }
    pub fn find(&self, name: &str) -> Option<&Element> {
        self.iter().find(|e| e.name == name)
    }
    /// Merge known children (in schema order) with the extras at their original
    /// indices; `extLst` always goes last (it is last in every schema type).
    fn weave(&self, mut known: Vec<Element>) -> Vec<Node> {
        let mut tail = Vec::new();
        for (i, e) in &self.0 {
            if e.local() == "extLst" {
                tail.push(e.clone());
            } else {
                let i = (*i).min(known.len());
                known.insert(i, e.clone());
            }
        }
        known.extend(tail);
        known.into_iter().map(Node::Element).collect()
    }
}

fn build(name: &str, attrs: &Attrs, known: Vec<Element>, extras: &Extras) -> Element {
    Element {
        name: name.to_string(),
        attrs: attrs.clone(),
        children: extras.weave(known),
    }
}

fn opt_push<T>(v: &mut Vec<Element>, x: &Option<T>, f: impl Fn(&T) -> Element) {
    if let Some(x) = x {
        v.push(f(x));
    }
}

// ---------------------------------------------------------------------------
// Transform, colour, fill, line, geometry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Xfrm {
    /// `a:xfrm` (shapes) or `p:xfrm` (graphic frames).
    pub tag: String,
    /// rot (60000ths of a degree), flipH, flipV.
    pub attrs: Attrs,
    pub off: Option<(i64, i64)>,
    pub ext: Option<(i64, i64)>,
    /// Group child coordinate space.
    pub ch_off: Option<(i64, i64)>,
    pub ch_ext: Option<(i64, i64)>,
    pub extras: Extras,
}

impl Xfrm {
    pub fn new(x: i64, y: i64, cx: i64, cy: i64) -> Self {
        Self {
            tag: "a:xfrm".into(),
            off: Some((x, y)),
            ext: Some((cx, cy)),
            ..Default::default()
        }
    }
    pub fn parse(el: &Element) -> Self {
        let mut x = Xfrm {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            ..Default::default()
        };
        let pt = |e: &Element, a: &str, b: &str| {
            (
                e.attrs.get_i64(a).unwrap_or(0),
                e.attrs.get_i64(b).unwrap_or(0),
            )
        };
        for (i, c) in el.elements().enumerate() {
            match c.local() {
                "off" => x.off = Some(pt(c, "x", "y")),
                "ext" => x.ext = Some(pt(c, "cx", "cy")),
                "chOff" => x.ch_off = Some(pt(c, "x", "y")),
                "chExt" => x.ch_ext = Some(pt(c, "cx", "cy")),
                _ => x.extras.push(i, c),
            }
        }
        x
    }
    pub fn to_element(&self) -> Element {
        let mut k = Vec::new();
        let p = |n: &str, a: &str, b: &str, v: (i64, i64)| {
            Element::new(n)
                .with_attr(a, v.0.to_string())
                .with_attr(b, v.1.to_string())
        };
        opt_push(&mut k, &self.off, |v| p("a:off", "x", "y", *v));
        opt_push(&mut k, &self.ext, |v| p("a:ext", "cx", "cy", *v));
        opt_push(&mut k, &self.ch_off, |v| p("a:chOff", "x", "y", *v));
        opt_push(&mut k, &self.ch_ext, |v| p("a:chExt", "cx", "cy", *v));
        build(&self.tag, &self.attrs, k, &self.extras)
    }
    pub fn rot_deg(&self) -> f64 {
        self.attrs.get_i64("rot").unwrap_or(0) as f64 / 60_000.0
    }
    pub fn flip_h(&self) -> bool {
        self.attrs.get_bool("flipH").unwrap_or(false)
    }
    pub fn flip_v(&self) -> bool {
        self.attrs.get_bool("flipV").unwrap_or(false)
    }
}

/// A DrawingML colour: `srgbClr`, `schemeClr`, `sysClr`, `prstClr`, `scrgbClr`, `hslClr`
/// plus its transform children (`alpha`, `lumMod`, `lumOff`, `shade`, `tint`…).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub tag: String,
    pub attrs: Attrs,
    pub mods: Vec<Element>,
}

impl Color {
    pub fn srgb(hex: &str) -> Self {
        Self {
            tag: "a:srgbClr".into(),
            attrs: Attrs(vec![("val".into(), hex.to_uppercase())]),
            mods: vec![],
        }
    }
    pub fn scheme(name: &str) -> Self {
        Self {
            tag: "a:schemeClr".into(),
            attrs: Attrs(vec![("val".into(), name.into())]),
            mods: vec![],
        }
    }
    pub fn parse(el: &Element) -> Self {
        Self {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            mods: el.elements().cloned().collect(),
        }
    }
    pub fn to_element(&self) -> Element {
        Element {
            name: self.tag.clone(),
            attrs: self.attrs.clone(),
            children: self.mods.iter().cloned().map(Node::Element).collect(),
        }
    }
    pub fn kind(&self) -> &str {
        crate::xml::local_name(&self.tag)
    }
    pub fn val(&self) -> Option<&str> {
        self.attrs.get("val")
    }
    /// RGB hex when directly known (srgbClr, or sysClr lastClr).
    pub fn hex(&self) -> Option<String> {
        match self.kind() {
            "srgbClr" => self.val().map(|s| s.to_uppercase()),
            "sysClr" => self.attrs.get("lastClr").map(|s| s.to_uppercase()),
            _ => None,
        }
    }
    fn mod_val(&self, name: &str) -> Option<i64> {
        self.mods
            .iter()
            .find(|m| m.local() == name)
            .and_then(|m| m.attrs.get_i64("val"))
    }
    /// Alpha as a fraction (1.0 = opaque).
    pub fn alpha(&self) -> f64 {
        self.mod_val("alpha")
            .map(|v| v as f64 / 100_000.0)
            .unwrap_or(1.0)
    }
    pub fn set_alpha(&mut self, frac: Option<f64>) {
        self.mods.retain(|m| m.local() != "alpha");
        if let Some(f) = frac {
            let v = (f.clamp(0.0, 1.0) * 100_000.0).round() as i64;
            self.mods
                .push(Element::new("a:alpha").with_attr("val", v.to_string()));
        }
    }
    pub fn mod_names(&self) -> Vec<&str> {
        self.mods.iter().map(|m| m.local()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Fill {
    NoFill,
    Solid(Option<Color>),
    /// Kept raw (editable as a whole, not stop by stop).
    Gradient(Element),
    Blip(Element),
    Pattern(Element),
    Group,
}

impl Fill {
    pub fn is_fill(name: &str) -> bool {
        matches!(
            crate::xml::local_name(name),
            "noFill" | "solidFill" | "gradFill" | "blipFill" | "pattFill" | "grpFill"
        )
    }
    pub fn parse(el: &Element) -> Self {
        match el.local() {
            "noFill" => Fill::NoFill,
            "solidFill" => Fill::Solid(el.elements().next().map(Color::parse)),
            "gradFill" => Fill::Gradient(el.clone()),
            "blipFill" => Fill::Blip(el.clone()),
            "pattFill" => Fill::Pattern(el.clone()),
            _ => Fill::Group,
        }
    }
    pub fn to_element(&self) -> Element {
        match self {
            Fill::NoFill => Element::new("a:noFill"),
            Fill::Solid(c) => {
                let mut e = Element::new("a:solidFill");
                if let Some(c) = c {
                    e.push(c.to_element());
                }
                e
            }
            Fill::Gradient(e) | Fill::Blip(e) | Fill::Pattern(e) => e.clone(),
            Fill::Group => Element::new("a:grpFill"),
        }
    }
    pub fn solid(hex: &str) -> Self {
        Fill::Solid(Some(Color::srgb(hex)))
    }
    pub fn color(&self) -> Option<&Color> {
        match self {
            Fill::Solid(c) => c.as_ref(),
            _ => None,
        }
    }
    pub fn color_mut(&mut self) -> Option<&mut Color> {
        match self {
            Fill::Solid(c) => c.as_mut(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Dash {
    Preset(String),
    Custom(Element),
}

/// `a:headEnd` / `a:tailEnd` (attributes `type`, `w`, `len`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LineEnd {
    pub attrs: Attrs,
}
impl LineEnd {
    pub fn kind(&self) -> &str {
        self.attrs.get("type").unwrap_or("none")
    }
    pub fn arrow(kind: &str) -> Self {
        Self {
            attrs: Attrs(vec![("type".into(), kind.into())]),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub tag: String,
    /// w (EMU), cap, cmpd, algn.
    pub attrs: Attrs,
    pub fill: Option<Fill>,
    pub dash: Option<Dash>,
    /// round / bevel / miter (raw).
    pub join: Option<Element>,
    pub head: Option<LineEnd>,
    pub tail: Option<LineEnd>,
    pub extras: Extras,
}

impl Line {
    pub fn new(width_pt: f64, hex: &str) -> Self {
        let mut l = Line {
            tag: "a:ln".into(),
            ..Default::default()
        };
        l.set_width_pt(width_pt);
        l.fill = Some(Fill::solid(hex));
        l
    }
    pub fn parse(el: &Element) -> Self {
        let mut l = Line {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            ..Default::default()
        };
        for (i, c) in el.elements().enumerate() {
            match c.local() {
                n if Fill::is_fill(n) => l.fill = Some(Fill::parse(c)),
                "prstDash" => l.dash = Some(Dash::Preset(c.attr("val").unwrap_or("solid").into())),
                "custDash" => l.dash = Some(Dash::Custom(c.clone())),
                "round" | "bevel" | "miter" => l.join = Some(c.clone()),
                "headEnd" => {
                    l.head = Some(LineEnd {
                        attrs: c.attrs.clone(),
                    })
                }
                "tailEnd" => {
                    l.tail = Some(LineEnd {
                        attrs: c.attrs.clone(),
                    })
                }
                _ => l.extras.push(i, c),
            }
        }
        l
    }
    pub fn to_element(&self) -> Element {
        let mut k = Vec::new();
        opt_push(&mut k, &self.fill, Fill::to_element);
        opt_push(&mut k, &self.dash, |d| match d {
            Dash::Preset(v) => Element::new("a:prstDash").with_attr("val", v),
            Dash::Custom(e) => e.clone(),
        });
        opt_push(&mut k, &self.join, Clone::clone);
        opt_push(&mut k, &self.head, |h| Element {
            name: "a:headEnd".into(),
            attrs: h.attrs.clone(),
            children: vec![],
        });
        opt_push(&mut k, &self.tail, |h| Element {
            name: "a:tailEnd".into(),
            attrs: h.attrs.clone(),
            children: vec![],
        });
        build(&self.tag, &self.attrs, k, &self.extras)
    }
    pub fn width_pt(&self) -> Option<f64> {
        self.attrs
            .get_i64("w")
            .map(|w| w as f64 / EMU_PER_PT as f64)
    }
    pub fn set_width_pt(&mut self, pt: f64) {
        self.attrs
            .set("w", ((pt * EMU_PER_PT as f64).round() as i64).to_string());
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Geometry {
    Preset {
        prst: String,
        /// Adjust values (`gd name fmla`); `None` when `avLst` was absent.
        adj: Option<Vec<(String, String)>>,
        extras: Extras,
    },
    /// Free-form path, kept as raw XML (still movable/resizable/fillable).
    Custom(Element),
}

impl Geometry {
    pub fn preset(prst: &str) -> Self {
        Geometry::Preset {
            prst: prst.into(),
            adj: Some(vec![]),
            extras: Extras::default(),
        }
    }
    fn parse(el: &Element) -> Self {
        if el.local() == "custGeom" {
            return Geometry::Custom(el.clone());
        }
        let mut adj = None;
        let mut extras = Extras::default();
        for (i, c) in el.elements().enumerate() {
            if c.local() == "avLst" {
                adj = Some(
                    c.elements()
                        .map(|g| {
                            (
                                g.attr("name").unwrap_or_default().to_string(),
                                g.attr("fmla").unwrap_or_default().to_string(),
                            )
                        })
                        .collect(),
                );
            } else {
                extras.push(i, c);
            }
        }
        Geometry::Preset {
            prst: el.attr("prst").unwrap_or("rect").into(),
            adj,
            extras,
        }
    }
    fn to_element(&self) -> Element {
        match self {
            Geometry::Custom(e) => e.clone(),
            Geometry::Preset { prst, adj, extras } => {
                let mut k = Vec::new();
                if let Some(adj) = adj {
                    let mut av = Element::new("a:avLst");
                    for (n, f) in adj {
                        av.push(
                            Element::new("a:gd")
                                .with_attr("name", n)
                                .with_attr("fmla", f),
                        );
                    }
                    k.push(av);
                }
                build(
                    "a:prstGeom",
                    &Attrs(vec![("prst".into(), prst.clone())]),
                    k,
                    extras,
                )
            }
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Geometry::Preset { prst, .. } => prst,
            Geometry::Custom(_) => "custom",
        }
    }
}

/// `p:spPr` / `p:grpSpPr` / `a:tcPr`-like shape properties.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeProps {
    pub tag: String,
    pub attrs: Attrs,
    pub xfrm: Option<Xfrm>,
    pub geom: Option<Geometry>,
    pub fill: Option<Fill>,
    pub line: Option<Line>,
    /// effectLst, scene3d, sp3d, a14:hiddenFill … (raw).
    pub extras: Extras,
}

impl ShapeProps {
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.into(),
            ..Default::default()
        }
    }
    pub fn parse(el: &Element) -> Self {
        let mut s = ShapeProps {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            ..Default::default()
        };
        for (i, c) in el.elements().enumerate() {
            match c.local() {
                "xfrm" => s.xfrm = Some(Xfrm::parse(c)),
                "prstGeom" | "custGeom" => s.geom = Some(Geometry::parse(c)),
                n if Fill::is_fill(n) => s.fill = Some(Fill::parse(c)),
                "ln" => s.line = Some(Line::parse(c)),
                _ => s.extras.push(i, c),
            }
        }
        s
    }
    pub fn to_element(&self) -> Element {
        let mut k = Vec::new();
        opt_push(&mut k, &self.xfrm, Xfrm::to_element);
        opt_push(&mut k, &self.geom, Geometry::to_element);
        opt_push(&mut k, &self.fill, Fill::to_element);
        opt_push(&mut k, &self.line, Line::to_element);
        build(&self.tag, &self.attrs, k, &self.extras)
    }
}

// ---------------------------------------------------------------------------
// Non-visual properties
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Placeholder {
    /// type, idx, sz, orient, hasCustomPrompt.
    pub attrs: Attrs,
    pub children: Vec<Element>,
}
impl Placeholder {
    /// Placeholder type; the schema default is `obj`.
    pub fn kind(&self) -> &str {
        self.attrs.get("type").unwrap_or("obj")
    }
    pub fn idx(&self) -> Option<u32> {
        self.attrs.get("idx").and_then(|v| v.parse().ok())
    }
}

/// `p:cNvSpPr` / `p:cNvPicPr` / `p:cNvCxnSpPr` / `p:cNvGrpSpPr` / `p:cNvGraphicFramePr`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KindProps {
    pub tag: String,
    /// txBox="1", preferRelativeResize …
    pub attrs: Attrs,
    /// spLocks / picLocks / cxnSpLocks … (raw).
    pub locks: Option<Element>,
    /// Connector start/end: (shape id, connection site index).
    pub st_cxn: Option<(u32, u32)>,
    pub end_cxn: Option<(u32, u32)>,
    pub extras: Extras,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NvProps {
    pub tag: String,
    /// `p:cNvPr` attributes: id, name, descr, title, hidden.
    pub cnv_attrs: Attrs,
    /// `p:cNvPr` children (hlinkClick, extLst with creationId) raw.
    pub cnv_children: Vec<Element>,
    pub kind: KindProps,
    /// `p:nvPr` attributes (isPhoto, userDrawn).
    pub nvpr_attrs: Attrs,
    pub ph: Option<Placeholder>,
    /// other `p:nvPr` children (media, custDataLst, extLst) raw, with positions.
    pub nvpr_extras: Extras,
    pub extras: Extras,
}

impl NvProps {
    pub fn new(tag: &str, kind_tag: &str, id: u32, name: &str) -> Self {
        NvProps {
            tag: tag.into(),
            cnv_attrs: Attrs(vec![
                ("id".into(), id.to_string()),
                ("name".into(), name.into()),
            ]),
            kind: KindProps {
                tag: kind_tag.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    pub fn id(&self) -> u32 {
        self.cnv_attrs
            .get("id")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }
    pub fn name(&self) -> &str {
        self.cnv_attrs.get("name").unwrap_or("")
    }
    pub fn hidden(&self) -> bool {
        self.cnv_attrs.get_bool("hidden").unwrap_or(false)
    }
    pub fn parse(el: &Element) -> Self {
        let mut nv = NvProps {
            tag: el.name.clone(),
            ..Default::default()
        };
        for (i, c) in el.elements().enumerate() {
            match c.local() {
                "cNvPr" => {
                    nv.cnv_attrs = c.attrs.clone();
                    nv.cnv_children = c.elements().cloned().collect();
                }
                "nvPr" => {
                    nv.nvpr_attrs = c.attrs.clone();
                    for (j, d) in c.elements().enumerate() {
                        if d.local() == "ph" {
                            nv.ph = Some(Placeholder {
                                attrs: d.attrs.clone(),
                                children: d.elements().cloned().collect(),
                            });
                        } else {
                            nv.nvpr_extras.push(j, d);
                        }
                    }
                }
                n if n.starts_with("cNv") => {
                    let mut k = KindProps {
                        tag: c.name.clone(),
                        attrs: c.attrs.clone(),
                        ..Default::default()
                    };
                    let cx = |d: &Element| {
                        (
                            d.attr("id").and_then(|v| v.parse().ok()).unwrap_or(0),
                            d.attr("idx").and_then(|v| v.parse().ok()).unwrap_or(0),
                        )
                    };
                    for (j, d) in c.elements().enumerate() {
                        match d.local() {
                            l if l.ends_with("Locks") => k.locks = Some(d.clone()),
                            "stCxn" => k.st_cxn = Some(cx(d)),
                            "endCxn" => k.end_cxn = Some(cx(d)),
                            _ => k.extras.push(j, d),
                        }
                    }
                    nv.kind = k;
                }
                _ => nv.extras.push(i, c),
            }
        }
        nv
    }
    pub fn to_element(&self) -> Element {
        let ns = self.tag.split(':').next().unwrap_or("p");
        let cnv = Element {
            name: format!("{ns}:cNvPr"),
            attrs: self.cnv_attrs.clone(),
            children: self
                .cnv_children
                .iter()
                .cloned()
                .map(Node::Element)
                .collect(),
        };
        let mut kk = Vec::new();
        opt_push(&mut kk, &self.kind.locks, Clone::clone);
        let cx = |n: &str, v: &(u32, u32)| {
            Element::new(n)
                .with_attr("id", v.0.to_string())
                .with_attr("idx", v.1.to_string())
        };
        opt_push(&mut kk, &self.kind.st_cxn, |v| cx("a:stCxn", v));
        opt_push(&mut kk, &self.kind.end_cxn, |v| cx("a:endCxn", v));
        let kind = build(&self.kind.tag, &self.kind.attrs, kk, &self.kind.extras);
        let mut nk = Vec::new();
        opt_push(&mut nk, &self.ph, |p| Element {
            name: format!("{ns}:ph"),
            attrs: p.attrs.clone(),
            children: p.children.iter().cloned().map(Node::Element).collect(),
        });
        let nvpr = build(
            &format!("{ns}:nvPr"),
            &self.nvpr_attrs,
            nk,
            &self.nvpr_extras,
        );
        build(
            &self.tag,
            &Attrs::new(),
            vec![cnv, kind, nvpr],
            &self.extras,
        )
    }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Spacing {
    /// Percent in 1/1000 % (100000 = single).
    Pct(i64),
    /// Points in 1/100 pt.
    Pts(i64),
    Raw(Element),
}
impl Spacing {
    fn parse(el: &Element) -> Self {
        match el.elements().next() {
            Some(c) if c.local() == "spcPct" => {
                Spacing::Pct(c.attrs.get_i64("val").unwrap_or(100_000))
            }
            Some(c) if c.local() == "spcPts" => Spacing::Pts(c.attrs.get_i64("val").unwrap_or(0)),
            _ => Spacing::Raw(el.clone()),
        }
    }
    fn to_element(&self, tag: &str) -> Element {
        match self {
            Spacing::Pct(v) => Element::new(tag)
                .with_child(Element::new("a:spcPct").with_attr("val", v.to_string())),
            Spacing::Pts(v) => Element::new(tag)
                .with_child(Element::new("a:spcPts").with_attr("val", v.to_string())),
            Spacing::Raw(e) => e.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Bullet {
    None,
    /// `buChar` (attribute `char`).
    Char(Attrs),
    /// `buAutoNum` (attributes `type` e.g. arabicPeriod, `startAt`).
    AutoNum(Attrs),
    Blip(Element),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParaProps {
    /// `a:pPr`, `a:lvl1pPr`…, `a:defPPr`.
    pub tag: String,
    /// lvl, algn, marL, indent, rtl, …
    pub attrs: Attrs,
    pub line_spacing: Option<Spacing>,
    pub space_before: Option<Spacing>,
    pub space_after: Option<Spacing>,
    /// buClrTx / buClr (raw).
    pub bullet_color: Option<Element>,
    /// buSzTx / buSzPct / buSzPts (raw).
    pub bullet_size: Option<Element>,
    /// buFontTx / buFont (raw).
    pub bullet_font: Option<Element>,
    pub bullet: Option<Bullet>,
    pub tab_lst: Option<Element>,
    pub def_rpr: Option<RunProps>,
    pub extras: Extras,
}

impl ParaProps {
    pub fn new() -> Self {
        Self {
            tag: "a:pPr".into(),
            ..Default::default()
        }
    }
    pub fn level(&self) -> u32 {
        self.attrs.get_i64("lvl").unwrap_or(0) as u32
    }
    pub fn align(&self) -> Option<&str> {
        self.attrs.get("algn")
    }
    pub fn parse(el: &Element) -> Self {
        let mut p = ParaProps {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            ..Default::default()
        };
        for (i, c) in el.elements().enumerate() {
            match c.local() {
                "lnSpc" => p.line_spacing = Some(Spacing::parse(c)),
                "spcBef" => p.space_before = Some(Spacing::parse(c)),
                "spcAft" => p.space_after = Some(Spacing::parse(c)),
                "buClrTx" | "buClr" => p.bullet_color = Some(c.clone()),
                "buSzTx" | "buSzPct" | "buSzPts" => p.bullet_size = Some(c.clone()),
                "buFontTx" | "buFont" => p.bullet_font = Some(c.clone()),
                "buNone" => p.bullet = Some(Bullet::None),
                "buChar" => p.bullet = Some(Bullet::Char(c.attrs.clone())),
                "buAutoNum" => p.bullet = Some(Bullet::AutoNum(c.attrs.clone())),
                "buBlip" => p.bullet = Some(Bullet::Blip(c.clone())),
                "tabLst" => p.tab_lst = Some(c.clone()),
                "defRPr" => p.def_rpr = Some(RunProps::parse(c)),
                _ => p.extras.push(i, c),
            }
        }
        p
    }
    pub fn to_element(&self) -> Element {
        let mut k = Vec::new();
        opt_push(&mut k, &self.line_spacing, |s| s.to_element("a:lnSpc"));
        opt_push(&mut k, &self.space_before, |s| s.to_element("a:spcBef"));
        opt_push(&mut k, &self.space_after, |s| s.to_element("a:spcAft"));
        opt_push(&mut k, &self.bullet_color, Clone::clone);
        opt_push(&mut k, &self.bullet_size, Clone::clone);
        opt_push(&mut k, &self.bullet_font, Clone::clone);
        opt_push(&mut k, &self.bullet, |b| match b {
            Bullet::None => Element::new("a:buNone"),
            Bullet::Char(a) => Element {
                name: "a:buChar".into(),
                attrs: a.clone(),
                children: vec![],
            },
            Bullet::AutoNum(a) => Element {
                name: "a:buAutoNum".into(),
                attrs: a.clone(),
                children: vec![],
            },
            Bullet::Blip(e) => e.clone(),
        });
        opt_push(&mut k, &self.tab_lst, Clone::clone);
        opt_push(&mut k, &self.def_rpr, RunProps::to_element);
        build(&self.tag, &self.attrs, k, &self.extras)
    }
}

/// `a:rPr` / `a:endParaRPr` / `a:defRPr`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunProps {
    pub tag: String,
    /// lang, altLang, sz (1/100 pt), b, i, u, strike, baseline, dirty, …
    pub attrs: Attrs,
    pub line: Option<Line>,
    pub fill: Option<Fill>,
    /// typeface attributes of `a:latin`, `a:ea`, `a:cs`, `a:sym`.
    pub latin: Option<Attrs>,
    pub ea: Option<Attrs>,
    pub cs: Option<Attrs>,
    pub sym: Option<Attrs>,
    /// effectLst, highlight, hlinkClick, uFillTx … (raw).
    pub extras: Extras,
}

impl RunProps {
    pub fn new() -> Self {
        Self {
            tag: "a:rPr".into(),
            ..Default::default()
        }
    }
    pub fn parse(el: &Element) -> Self {
        let mut r = RunProps {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            ..Default::default()
        };
        for (i, c) in el.elements().enumerate() {
            match c.local() {
                "ln" => r.line = Some(Line::parse(c)),
                n if Fill::is_fill(n) => r.fill = Some(Fill::parse(c)),
                "latin" if c.children.is_empty() => r.latin = Some(c.attrs.clone()),
                "ea" if c.children.is_empty() => r.ea = Some(c.attrs.clone()),
                "cs" if c.children.is_empty() => r.cs = Some(c.attrs.clone()),
                "sym" if c.children.is_empty() => r.sym = Some(c.attrs.clone()),
                _ => r.extras.push(i, c),
            }
        }
        r
    }
    pub fn to_element(&self) -> Element {
        let mut k = Vec::new();
        opt_push(&mut k, &self.line, Line::to_element);
        opt_push(&mut k, &self.fill, Fill::to_element);
        let font = |n: &str, a: &Attrs| Element {
            name: n.into(),
            attrs: a.clone(),
            children: vec![],
        };
        opt_push(&mut k, &self.latin, |a| font("a:latin", a));
        opt_push(&mut k, &self.ea, |a| font("a:ea", a));
        opt_push(&mut k, &self.cs, |a| font("a:cs", a));
        opt_push(&mut k, &self.sym, |a| font("a:sym", a));
        build(&self.tag, &self.attrs, k, &self.extras)
    }
    /// Size in points (from `sz`, 1/100 pt).
    pub fn size_pt(&self) -> Option<f64> {
        self.attrs.get_i64("sz").map(|v| v as f64 / 100.0)
    }
    pub fn set_size_pt(&mut self, pt: f64) {
        self.attrs
            .set("sz", ((pt * 100.0).round() as i64).to_string());
    }
    pub fn bold(&self) -> Option<bool> {
        self.attrs.get_bool("b")
    }
    pub fn italic(&self) -> Option<bool> {
        self.attrs.get_bool("i")
    }
    pub fn latin_font(&self) -> Option<&str> {
        self.latin.as_ref().and_then(|a| a.get("typeface"))
    }
    pub fn ea_font(&self) -> Option<&str> {
        self.ea.as_ref().and_then(|a| a.get("typeface"))
    }
    pub fn set_font(&mut self, latin: &str, ea: Option<&str>) {
        self.latin = Some(Attrs(vec![("typeface".into(), latin.into())]));
        if let Some(ea) = ea {
            self.ea = Some(Attrs(vec![("typeface".into(), ea.into())]));
        }
    }
    pub fn color(&self) -> Option<&Color> {
        self.fill.as_ref().and_then(Fill::color)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Run {
    Text {
        props: Option<RunProps>,
        text: String,
    },
    Break {
        props: Option<RunProps>,
    },
    /// `a:fld` (slide number, date…): attributes id/type, optional pPr, text.
    Field {
        attrs: Attrs,
        props: Option<RunProps>,
        para_props: Option<ParaProps>,
        text: Option<String>,
    },
    /// Inline Office Math (`a14:m` holding `m:oMathPara`/`m:oMath`), kept raw.
    Math(Element),
    /// Anything else inside a paragraph (e.g. `mc:AlternateContent`), kept raw.
    Raw(Element),
}

impl Run {
    pub fn text(s: &str) -> Self {
        Run::Text {
            props: Some(RunProps::new()),
            text: s.into(),
        }
    }
    pub fn plain_text(&self) -> String {
        match self {
            Run::Text { text, .. } => text.clone(),
            Run::Break { .. } => "\n".into(),
            Run::Field { text, .. } => text.clone().unwrap_or_default(),
            Run::Math(e) | Run::Raw(e) => e.find_all("m:t").iter().map(|t| t.text()).collect(),
        }
    }
    pub fn props(&self) -> Option<&RunProps> {
        match self {
            Run::Text { props, .. } | Run::Break { props } | Run::Field { props, .. } => {
                props.as_ref()
            }
            _ => None,
        }
    }
    pub fn props_mut(&mut self) -> Option<&mut Option<RunProps>> {
        match self {
            Run::Text { props, .. } | Run::Break { props } | Run::Field { props, .. } => {
                Some(props)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub props: Option<ParaProps>,
    pub runs: Vec<Run>,
    pub end_props: Option<RunProps>,
}

impl Paragraph {
    pub fn parse(el: &Element) -> Self {
        let mut p = Paragraph::default();
        for c in el.elements() {
            match c.local() {
                "pPr" => p.props = Some(ParaProps::parse(c)),
                "r" => p.runs.push(Run::Text {
                    props: c.child("a:rPr").map(RunProps::parse),
                    text: c.child("a:t").map(|t| t.text()).unwrap_or_default(),
                }),
                "br" => p.runs.push(Run::Break {
                    props: c.child("a:rPr").map(RunProps::parse),
                }),
                "fld" => p.runs.push(Run::Field {
                    attrs: c.attrs.clone(),
                    props: c.child("a:rPr").map(RunProps::parse),
                    para_props: c.child("a:pPr").map(ParaProps::parse),
                    text: c.child("a:t").map(|t| t.text()),
                }),
                "endParaRPr" => p.end_props = Some(RunProps::parse(c)),
                "m" if c.name.starts_with("a14") => p.runs.push(Run::Math(c.clone())),
                _ => p.runs.push(Run::Raw(c.clone())),
            }
        }
        p
    }
    pub fn to_element(&self) -> Element {
        let mut e = Element::new("a:p");
        if let Some(pp) = &self.props {
            e.push(pp.to_element());
        }
        for r in &self.runs {
            e.push(match r {
                Run::Text { props, text } => {
                    let mut x = Element::new("a:r");
                    if let Some(p) = props {
                        x.push(p.to_element());
                    }
                    x.push(text_el("a:t", text));
                    x
                }
                Run::Break { props } => {
                    let mut x = Element::new("a:br");
                    if let Some(p) = props {
                        x.push(p.to_element());
                    }
                    x
                }
                Run::Field {
                    attrs,
                    props,
                    para_props,
                    text,
                } => {
                    let mut x = Element {
                        name: "a:fld".into(),
                        attrs: attrs.clone(),
                        children: vec![],
                    };
                    if let Some(p) = props {
                        x.push(p.to_element());
                    }
                    if let Some(p) = para_props {
                        x.push(p.to_element());
                    }
                    if let Some(t) = text {
                        x.push(text_el("a:t", t));
                    }
                    x
                }
                Run::Math(e) | Run::Raw(e) => e.clone(),
            });
        }
        if let Some(ep) = &self.end_props {
            e.push(ep.to_element());
        }
        e
    }
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(Run::plain_text).collect()
    }
    pub fn level(&self) -> u32 {
        self.props.as_ref().map(|p| p.level()).unwrap_or(0)
    }
}

fn text_el(name: &str, t: &str) -> Element {
    let mut e = Element::new(name);
    if !t.is_empty() {
        e.children.push(Node::Text(t.into()));
    }
    e
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Autofit {
    None,
    Normal(Attrs),
    Shape,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BodyProps {
    /// wrap, lIns/tIns/rIns/bIns, anchor, vert, rtlCol …
    pub attrs: Attrs,
    pub autofit: Option<Autofit>,
    pub extras: Extras,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TextBody {
    /// `p:txBody` (shapes) or `a:txBody` (table cells).
    pub tag: String,
    pub body: BodyProps,
    /// `a:lstStyle` (per-level defaults) — parsed per level.
    pub list_style: Option<ListStyle>,
    pub paragraphs: Vec<Paragraph>,
    pub extras: Extras,
}

/// `a:lstStyle`, master `p:titleStyle`/`p:bodyStyle`, `p:defaultTextStyle`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListStyle {
    pub tag: String,
    pub attrs: Attrs,
    /// `defPPr` + `lvl1pPr`..`lvl9pPr` in order.
    pub levels: Vec<ParaProps>,
    pub extras: Extras,
}

impl ListStyle {
    pub fn parse(el: &Element) -> Self {
        let mut l = ListStyle {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            ..Default::default()
        };
        for (i, c) in el.elements().enumerate() {
            if c.local() == "defPPr" || (c.local().starts_with("lvl") && c.local().ends_with("pPr"))
            {
                l.levels.push(ParaProps::parse(c));
            } else {
                l.extras.push(i, c);
            }
        }
        l
    }
    pub fn to_element(&self) -> Element {
        build(
            &self.tag,
            &self.attrs,
            self.levels.iter().map(ParaProps::to_element).collect(),
            &self.extras,
        )
    }
    /// Properties for level 0..8 (`lvl{n+1}pPr`).
    pub fn level(&self, lvl: u32) -> Option<&ParaProps> {
        let want = format!("lvl{}pPr", lvl + 1);
        self.levels
            .iter()
            .find(|p| crate::xml::local_name(&p.tag) == want)
    }
    pub fn default_level(&self) -> Option<&ParaProps> {
        self.levels
            .iter()
            .find(|p| crate::xml::local_name(&p.tag) == "defPPr")
    }
}

impl TextBody {
    pub fn new(tag: &str) -> Self {
        TextBody {
            tag: tag.into(),
            body: BodyProps::default(),
            list_style: Some(ListStyle {
                tag: "a:lstStyle".into(),
                ..Default::default()
            }),
            paragraphs: vec![],
            extras: Extras::default(),
        }
    }
    pub fn parse(el: &Element) -> Self {
        let mut t = TextBody {
            tag: el.name.clone(),
            ..Default::default()
        };
        for (i, c) in el.elements().enumerate() {
            match c.local() {
                "bodyPr" => {
                    let mut b = BodyProps {
                        attrs: c.attrs.clone(),
                        ..Default::default()
                    };
                    for (j, d) in c.elements().enumerate() {
                        match d.local() {
                            "noAutofit" => b.autofit = Some(Autofit::None),
                            "normAutofit" => b.autofit = Some(Autofit::Normal(d.attrs.clone())),
                            "spAutoFit" => b.autofit = Some(Autofit::Shape),
                            _ => b.extras.push(j, d),
                        }
                    }
                    t.body = b;
                }
                "lstStyle" => t.list_style = Some(ListStyle::parse(c)),
                "p" => t.paragraphs.push(Paragraph::parse(c)),
                _ => t.extras.push(i, c),
            }
        }
        t
    }
    pub fn to_element(&self) -> Element {
        let mut bk = Vec::new();
        opt_push(&mut bk, &self.body.autofit, |a| match a {
            Autofit::None => Element::new("a:noAutofit"),
            Autofit::Normal(at) => Element {
                name: "a:normAutofit".into(),
                attrs: at.clone(),
                children: vec![],
            },
            Autofit::Shape => Element::new("a:spAutoFit"),
        });
        let body = build("a:bodyPr", &self.body.attrs, bk, &self.body.extras);
        let mut k = vec![body];
        opt_push(&mut k, &self.list_style, ListStyle::to_element);
        k.extend(self.paragraphs.iter().map(Paragraph::to_element));
        build(&self.tag, &Attrs::new(), k, &self.extras)
    }
    pub fn plain_text(&self) -> String {
        self.paragraphs
            .iter()
            .map(Paragraph::plain_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sp {
    pub attrs: Attrs,
    pub nv: NvProps,
    pub props: ShapeProps,
    /// `p:style` (theme style refs) raw.
    pub style: Option<Element>,
    pub text: Option<TextBody>,
    pub extras: Extras,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Blip {
    /// r:embed, r:link, cstate.
    pub attrs: Attrs,
    /// alphaModFix, lum, grayscl, extLst (a14:imgProps backgroundRemoval, useLocalDpi) — raw, ordered.
    pub effects: Vec<Element>,
}

impl Blip {
    pub fn embed(&self) -> Option<&str> {
        self.attrs.get("r:embed")
    }
    /// alphaModFix amount as a fraction (1.0 = opaque).
    pub fn alpha(&self) -> f64 {
        self.effects
            .iter()
            .find(|e| e.local() == "alphaModFix")
            .map(|e| e.attrs.get_i64("amt").unwrap_or(100_000) as f64 / 100_000.0)
            .unwrap_or(1.0)
    }
    pub fn set_alpha(&mut self, frac: Option<f64>) {
        self.effects.retain(|e| e.local() != "alphaModFix");
        if let Some(f) = frac {
            let v = (f.clamp(0.0, 1.0) * 100_000.0).round() as i64;
            self.effects.insert(
                0,
                Element::new("a:alphaModFix").with_attr("amt", v.to_string()),
            );
        }
    }
    pub fn has_background_removal(&self) -> bool {
        self.effects
            .iter()
            .any(|e| e.contains("a14:backgroundRemoval"))
    }
}

/// Crop rectangle in 1/1000 % of the source image (l, t, r, b).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SrcRect {
    pub l: i64,
    pub t: i64,
    pub r: i64,
    pub b: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlipFill {
    pub tag: String,
    pub attrs: Attrs,
    pub blip: Option<Blip>,
    /// `a:srcRect` attributes (l, t, r, b), see [`BlipFill::crop`].
    pub src_rect: Option<Attrs>,
    /// `a:stretch` / `a:tile` raw.
    pub mode: Option<Element>,
    pub extras: Extras,
}

impl BlipFill {
    /// Crop in 1/1000 % of the source image (negative = padding).
    pub fn crop(&self) -> Option<SrcRect> {
        self.src_rect.as_ref().map(|a| {
            let g = |k: &str| a.get_i64(k).unwrap_or(0);
            SrcRect {
                l: g("l"),
                t: g("t"),
                r: g("r"),
                b: g("b"),
            }
        })
    }
    pub fn set_crop(&mut self, r: Option<SrcRect>) {
        self.src_rect = r.map(|r| {
            let mut a = Attrs::new();
            for (n, v) in [("l", r.l), ("t", r.t), ("r", r.r), ("b", r.b)] {
                if v != 0 {
                    a.set(n, v.to_string());
                }
            }
            a
        });
    }
    pub fn parse(el: &Element) -> Self {
        let mut b = BlipFill {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            ..Default::default()
        };
        for (i, c) in el.elements().enumerate() {
            match c.local() {
                "blip" => {
                    b.blip = Some(Blip {
                        attrs: c.attrs.clone(),
                        effects: c.elements().cloned().collect(),
                    })
                }
                "srcRect" => b.src_rect = Some(c.attrs.clone()),
                "stretch" | "tile" => b.mode = Some(c.clone()),
                _ => b.extras.push(i, c),
            }
        }
        b
    }
    pub fn to_element(&self) -> Element {
        let mut k = Vec::new();
        opt_push(&mut k, &self.blip, |b| Element {
            name: "a:blip".into(),
            attrs: b.attrs.clone(),
            children: b.effects.iter().cloned().map(Node::Element).collect(),
        });
        opt_push(&mut k, &self.src_rect, |a| Element {
            name: "a:srcRect".into(),
            attrs: a.clone(),
            children: vec![],
        });
        opt_push(&mut k, &self.mode, Clone::clone);
        build(&self.tag, &self.attrs, k, &self.extras)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pic {
    pub attrs: Attrs,
    pub nv: NvProps,
    pub fill: BlipFill,
    pub props: ShapeProps,
    pub style: Option<Element>,
    pub extras: Extras,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cxn {
    pub attrs: Attrs,
    pub nv: NvProps,
    pub props: ShapeProps,
    pub style: Option<Element>,
    pub extras: Extras,
}

/// `p:grpSp`, and also the slide's `p:spTree`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub tag: String,
    pub attrs: Attrs,
    pub nv: NvProps,
    pub props: ShapeProps,
    pub children: Vec<Shape>,
    pub ext_lst: Option<Element>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    /// gridSpan, rowSpan, hMerge, vMerge.
    pub attrs: Attrs,
    pub text: Option<TextBody>,
    /// `a:tcPr` (margins, anchor, borders lnL/lnR/lnT/lnB, fill) raw.
    pub props: Option<Element>,
    pub extras: Extras,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    /// h (EMU).
    pub attrs: Attrs,
    pub cells: Vec<TableCell>,
    pub ext_lst: Option<Element>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub graphic_attrs: Attrs,
    pub data_attrs: Attrs,
    pub tbl_attrs: Attrs,
    /// `a:tblPr` (firstRow, bandRow, tableStyleId child) raw.
    pub props: Option<Element>,
    pub grid_attrs: Attrs,
    /// `a:gridCol` elements (w + extLst colId) raw.
    pub grid: Vec<Element>,
    pub rows: Vec<TableRow>,
    pub extras: Extras,
}

impl Table {
    pub fn style_id(&self) -> Option<String> {
        self.props
            .as_ref()
            .and_then(|p| p.child("a:tableStyleId"))
            .map(|e| e.text())
    }
    pub fn col_widths(&self) -> Vec<i64> {
        self.grid
            .iter()
            .map(|g| g.attrs.get_i64("w").unwrap_or(0))
            .collect()
    }
    fn parse(graphic: &Element, data: &Element, tbl: &Element) -> Self {
        let mut t = Table {
            graphic_attrs: graphic.attrs.clone(),
            data_attrs: data.attrs.clone(),
            tbl_attrs: tbl.attrs.clone(),
            ..Default::default()
        };
        for (i, c) in tbl.elements().enumerate() {
            match c.local() {
                "tblPr" => t.props = Some(c.clone()),
                "tblGrid" => {
                    t.grid_attrs = c.attrs.clone();
                    t.grid = c.elements().cloned().collect();
                }
                "tr" => {
                    let mut row = TableRow {
                        attrs: c.attrs.clone(),
                        ..Default::default()
                    };
                    for d in c.elements() {
                        match d.local() {
                            "tc" => {
                                let mut cell = TableCell {
                                    attrs: d.attrs.clone(),
                                    ..Default::default()
                                };
                                for (j, e) in d.elements().enumerate() {
                                    match e.local() {
                                        "txBody" => cell.text = Some(TextBody::parse(e)),
                                        "tcPr" => cell.props = Some(e.clone()),
                                        _ => cell.extras.push(j, e),
                                    }
                                }
                                row.cells.push(cell);
                            }
                            _ => row.ext_lst = Some(d.clone()),
                        }
                    }
                    t.rows.push(row);
                }
                _ => t.extras.push(i, c),
            }
        }
        t
    }
    fn to_graphic(&self) -> Element {
        let mut k = Vec::new();
        opt_push(&mut k, &self.props, Clone::clone);
        k.push(Element {
            name: "a:tblGrid".into(),
            attrs: self.grid_attrs.clone(),
            children: self.grid.iter().cloned().map(Node::Element).collect(),
        });
        for r in &self.rows {
            let mut re = Element {
                name: "a:tr".into(),
                attrs: r.attrs.clone(),
                children: vec![],
            };
            for c in &r.cells {
                let mut ck = Vec::new();
                opt_push(&mut ck, &c.text, TextBody::to_element);
                opt_push(&mut ck, &c.props, Clone::clone);
                re.push(build("a:tc", &c.attrs, ck, &c.extras));
            }
            if let Some(x) = &r.ext_lst {
                re.push(x.clone());
            }
            k.push(re);
        }
        let tbl = build("a:tbl", &self.tbl_attrs, k, &self.extras);
        Element {
            name: "a:graphic".into(),
            attrs: self.graphic_attrs.clone(),
            children: vec![Node::Element(Element {
                name: "a:graphicData".into(),
                attrs: self.data_attrs.clone(),
                children: vec![Node::Element(tbl)],
            })],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameContent {
    Table(Table),
    /// Charts, SmartArt, OLE, media … kept raw (locked).
    Other(Element),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub attrs: Attrs,
    pub nv: NvProps,
    pub xfrm: Option<Xfrm>,
    pub content: FrameContent,
    pub extras: Extras,
}

impl Frame {
    /// graphicData uri (e.g. `…/drawingml/2006/table`, `…/chart`, `…/diagram`).
    pub fn uri(&self) -> String {
        match &self.content {
            FrameContent::Table(t) => t.data_attrs.get("uri").unwrap_or_default().to_string(),
            FrameContent::Other(g) => g
                .child("a:graphicData")
                .and_then(|d| d.attr("uri"))
                .unwrap_or_default()
                .to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AltBranch {
    /// `Requires="a14"` and the namespace declarations it carries.
    pub attrs: Attrs,
    pub shapes: Vec<Shape>,
}

/// `mc:AlternateContent` at shape level, typically an OMML equation shape
/// (Choice, requires a14) with a picture fallback for old readers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AltContent {
    pub attrs: Attrs,
    pub choices: Vec<AltBranch>,
    pub fallback: Option<AltBranch>,
}

impl AltContent {
    /// True when any choice holds Office Math.
    pub fn has_math(&self) -> bool {
        self.choices
            .iter()
            .any(|c| c.shapes.iter().any(|s| s.to_element().contains("a14:m")))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Locked {
    /// Why it is locked, e.g. `graphicFrame:chart`, `p:contentPart`.
    pub kind: String,
    pub element: Element,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Sp(Sp),
    Pic(Pic),
    Cxn(Cxn),
    Group(Group),
    Frame(Frame),
    Alt(AltContent),
    Locked(Locked),
}

impl Shape {
    pub fn parse(el: &Element) -> Shape {
        match el.local() {
            "sp" => {
                let mut s = Sp {
                    attrs: el.attrs.clone(),
                    nv: NvProps::default(),
                    props: ShapeProps::new("p:spPr"),
                    style: None,
                    text: None,
                    extras: Extras::default(),
                };
                for (i, c) in el.elements().enumerate() {
                    match c.local() {
                        "nvSpPr" => s.nv = NvProps::parse(c),
                        "spPr" => s.props = ShapeProps::parse(c),
                        "style" => s.style = Some(c.clone()),
                        "txBody" => s.text = Some(TextBody::parse(c)),
                        _ => s.extras.push(i, c),
                    }
                }
                Shape::Sp(s)
            }
            "pic" => {
                let mut p = Pic {
                    attrs: el.attrs.clone(),
                    nv: NvProps::default(),
                    fill: BlipFill::default(),
                    props: ShapeProps::new("p:spPr"),
                    style: None,
                    extras: Extras::default(),
                };
                for (i, c) in el.elements().enumerate() {
                    match c.local() {
                        "nvPicPr" => p.nv = NvProps::parse(c),
                        "blipFill" => p.fill = BlipFill::parse(c),
                        "spPr" => p.props = ShapeProps::parse(c),
                        "style" => p.style = Some(c.clone()),
                        _ => p.extras.push(i, c),
                    }
                }
                Shape::Pic(p)
            }
            "cxnSp" => {
                let mut x = Cxn {
                    attrs: el.attrs.clone(),
                    nv: NvProps::default(),
                    props: ShapeProps::new("p:spPr"),
                    style: None,
                    extras: Extras::default(),
                };
                for (i, c) in el.elements().enumerate() {
                    match c.local() {
                        "nvCxnSpPr" => x.nv = NvProps::parse(c),
                        "spPr" => x.props = ShapeProps::parse(c),
                        "style" => x.style = Some(c.clone()),
                        _ => x.extras.push(i, c),
                    }
                }
                Shape::Cxn(x)
            }
            "grpSp" | "spTree" => Shape::Group(Group::parse(el)),
            "graphicFrame" => {
                let mut nv = NvProps::default();
                let mut xfrm = None;
                let mut content = None;
                let mut extras = Extras::default();
                for (i, c) in el.elements().enumerate() {
                    match c.local() {
                        "nvGraphicFramePr" => nv = NvProps::parse(c),
                        "xfrm" => xfrm = Some(Xfrm::parse(c)),
                        "graphic" => {
                            let tbl = c
                                .child("a:graphicData")
                                .and_then(|d| d.child("a:tbl").map(|t| (d, t)));
                            content = Some(match tbl {
                                Some((d, t)) if d.elements().count() == 1 => {
                                    FrameContent::Table(Table::parse(c, d, t))
                                }
                                _ => FrameContent::Other(c.clone()),
                            });
                        }
                        _ => extras.push(i, c),
                    }
                }
                match content {
                    Some(content) => Shape::Frame(Frame {
                        attrs: el.attrs.clone(),
                        nv,
                        xfrm,
                        content,
                        extras,
                    }),
                    None => Shape::Locked(Locked {
                        kind: "graphicFrame:empty".into(),
                        element: el.clone(),
                    }),
                }
            }
            "AlternateContent" => {
                let mut a = AltContent {
                    attrs: el.attrs.clone(),
                    choices: vec![],
                    fallback: None,
                };
                for c in el.elements() {
                    let br = AltBranch {
                        attrs: c.attrs.clone(),
                        shapes: c.elements().map(Shape::parse).collect(),
                    };
                    match c.local() {
                        "Choice" => a.choices.push(br),
                        "Fallback" => a.fallback = Some(br),
                        _ => {
                            return Shape::Locked(Locked {
                                kind: "mc:AlternateContent".into(),
                                element: el.clone(),
                            });
                        }
                    }
                }
                Shape::Alt(a)
            }
            _ => Shape::Locked(Locked {
                kind: el.name.clone(),
                element: el.clone(),
            }),
        }
    }

    pub fn to_element(&self) -> Element {
        match self {
            Shape::Sp(s) => {
                let mut k = vec![s.nv.to_element(), s.props.to_element()];
                opt_push(&mut k, &s.style, Clone::clone);
                opt_push(&mut k, &s.text, TextBody::to_element);
                build("p:sp", &s.attrs, k, &s.extras)
            }
            Shape::Pic(p) => {
                let mut k = vec![p.nv.to_element(), p.fill.to_element(), p.props.to_element()];
                opt_push(&mut k, &p.style, Clone::clone);
                build("p:pic", &p.attrs, k, &p.extras)
            }
            Shape::Cxn(x) => {
                let mut k = vec![x.nv.to_element(), x.props.to_element()];
                opt_push(&mut k, &x.style, Clone::clone);
                build("p:cxnSp", &x.attrs, k, &x.extras)
            }
            Shape::Group(g) => g.to_element(),
            Shape::Frame(f) => {
                let mut k = vec![f.nv.to_element()];
                opt_push(&mut k, &f.xfrm, Xfrm::to_element);
                k.push(match &f.content {
                    FrameContent::Table(t) => t.to_graphic(),
                    FrameContent::Other(e) => e.clone(),
                });
                build("p:graphicFrame", &f.attrs, k, &f.extras)
            }
            Shape::Alt(a) => {
                let mut e = Element {
                    name: "mc:AlternateContent".into(),
                    attrs: a.attrs.clone(),
                    children: vec![],
                };
                let br = |n: &str, b: &AltBranch| Element {
                    name: n.into(),
                    attrs: b.attrs.clone(),
                    children: b
                        .shapes
                        .iter()
                        .map(|s| Node::Element(s.to_element()))
                        .collect(),
                };
                for c in &a.choices {
                    e.push(br("mc:Choice", c));
                }
                if let Some(f) = &a.fallback {
                    e.push(br("mc:Fallback", f));
                }
                e
            }
            Shape::Locked(l) => l.element.clone(),
        }
    }

    pub fn nv(&self) -> Option<&NvProps> {
        match self {
            Shape::Sp(s) => Some(&s.nv),
            Shape::Pic(p) => Some(&p.nv),
            Shape::Cxn(c) => Some(&c.nv),
            Shape::Group(g) => Some(&g.nv),
            Shape::Frame(f) => Some(&f.nv),
            Shape::Alt(a) => a
                .choices
                .first()
                .and_then(|c| c.shapes.first())
                .and_then(|s| s.nv()),
            Shape::Locked(_) => None,
        }
    }
    pub fn id(&self) -> Option<u32> {
        self.nv().map(NvProps::id)
    }
    pub fn xfrm(&self) -> Option<&Xfrm> {
        match self {
            Shape::Sp(s) => s.props.xfrm.as_ref(),
            Shape::Pic(p) => p.props.xfrm.as_ref(),
            Shape::Cxn(c) => c.props.xfrm.as_ref(),
            Shape::Group(g) => g.props.xfrm.as_ref(),
            Shape::Frame(f) => f.xfrm.as_ref(),
            Shape::Alt(a) => a
                .choices
                .first()
                .and_then(|c| c.shapes.first())
                .and_then(|s| s.xfrm()),
            Shape::Locked(_) => None,
        }
    }
    pub fn kind_name(&self) -> &'static str {
        match self {
            Shape::Sp(s) if s.nv.kind.attrs.get_bool("txBox") == Some(true) => "textbox",
            Shape::Sp(_) => "sp",
            Shape::Pic(_) => "pic",
            Shape::Cxn(_) => "cxn",
            Shape::Group(_) => "grp",
            Shape::Frame(f) if matches!(f.content, FrameContent::Table(_)) => "table",
            Shape::Frame(_) => "frame",
            Shape::Alt(a) if a.has_math() => "math",
            Shape::Alt(_) => "alt",
            Shape::Locked(_) => "locked",
        }
    }
}

impl Group {
    pub fn parse(el: &Element) -> Self {
        let mut g = Group {
            tag: el.name.clone(),
            attrs: el.attrs.clone(),
            nv: NvProps::default(),
            props: ShapeProps::new("p:grpSpPr"),
            children: vec![],
            ext_lst: None,
        };
        for c in el.elements() {
            match c.local() {
                "nvGrpSpPr" => g.nv = NvProps::parse(c),
                "grpSpPr" => g.props = ShapeProps::parse(c),
                "extLst" => g.ext_lst = Some(c.clone()),
                _ => g.children.push(Shape::parse(c)),
            }
        }
        g
    }
    pub fn to_element(&self) -> Element {
        let mut e = Element {
            name: self.tag.clone(),
            attrs: self.attrs.clone(),
            children: vec![],
        };
        e.push(self.nv.to_element());
        e.push(self.props.to_element());
        for s in &self.children {
            e.push(s.to_element());
        }
        if let Some(x) = &self.ext_lst {
            e.push(x.clone());
        }
        e
    }
    /// Visit every shape depth-first (groups and alternate-content branches included).
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Shape, usize)) {
        fn rec<'a>(v: &'a [Shape], depth: usize, f: &mut dyn FnMut(&'a Shape, usize)) {
            for s in v {
                f(s, depth);
                match s {
                    Shape::Group(g) => rec(&g.children, depth + 1, f),
                    Shape::Alt(a) => {
                        for c in &a.choices {
                            rec(&c.shapes, depth + 1, f);
                        }
                    }
                    _ => {}
                }
            }
        }
        rec(&self.children, 0, f)
    }
    pub fn max_id(&self) -> u32 {
        let mut m = self.nv.id();
        self.walk(&mut |s, _| {
            if let Some(id) = s.id() {
                m = m.max(id);
            }
        });
        m
    }
}

// ---------------------------------------------------------------------------
// Slide-like documents (sld, notes, sldLayout, sldMaster)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideDoc {
    pub decl: Option<String>,
    pub prolog_ws: String,
    /// `p:sld`, `p:notes`, `p:sldLayout`, `p:sldMaster`.
    pub root: String,
    /// Root attributes: namespace declarations, `show="0"` (hidden), layout `type`…
    pub attrs: Attrs,
    /// `p:cSld` attributes (name).
    pub csld_attrs: Attrs,
    pub background: Option<Element>,
    pub tree: Group,
    pub csld_extras: Extras,
    /// `p:clrMapOvr` (slides) or `p:clrMap` (masters).
    pub clr_map: Option<Element>,
    pub transition: Option<Element>,
    /// `p:timing` kept raw (see [`crate::anim`] for building entrance effects).
    pub timing: Option<Element>,
    pub ext_lst: Option<Element>,
    /// Layout/master-only parts (sldLayoutIdLst, hf, txStyles) raw.
    pub extras: Extras,
}

impl SlideDoc {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let d = Document::parse(bytes)?;
        Ok(Self::from_document(&d))
    }
    pub fn from_document(d: &Document) -> Self {
        let r = &d.root;
        let mut s = SlideDoc {
            decl: d.decl.clone(),
            prolog_ws: d.prolog_ws.clone(),
            root: r.name.clone(),
            attrs: r.attrs.clone(),
            csld_attrs: Attrs::new(),
            background: None,
            tree: Group::parse(&Element::new("p:spTree")),
            csld_extras: Extras::default(),
            clr_map: None,
            transition: None,
            timing: None,
            ext_lst: None,
            extras: Extras::default(),
        };
        let mut idx = 0;
        for c in r.elements() {
            match c.local() {
                "cSld" => {
                    s.csld_attrs = c.attrs.clone();
                    for (j, e) in c.elements().enumerate() {
                        match e.local() {
                            "bg" => s.background = Some(e.clone()),
                            "spTree" => s.tree = Group::parse(e),
                            _ => s.csld_extras.push(j, e),
                        }
                    }
                }
                "clrMapOvr" | "clrMap" => s.clr_map = Some(c.clone()),
                "transition" => s.transition = Some(c.clone()),
                "timing" => s.timing = Some(c.clone()),
                "extLst" => s.ext_lst = Some(c.clone()),
                _ => s.extras.push(idx, c),
            }
            idx += 1;
        }
        s
    }
    pub fn to_document(&self) -> Document {
        let mut ck = Vec::new();
        opt_push(&mut ck, &self.background, Clone::clone);
        ck.push(self.tree.to_element());
        let csld = build("p:cSld", &self.csld_attrs, ck, &self.csld_extras);
        let mut k = vec![csld];
        opt_push(&mut k, &self.clr_map, Clone::clone);
        opt_push(&mut k, &self.transition, Clone::clone);
        opt_push(&mut k, &self.timing, Clone::clone);
        opt_push(&mut k, &self.ext_lst, Clone::clone);
        let root = build(&self.root, &self.attrs, k, &self.extras);
        Document {
            decl: self.decl.clone(),
            prolog_ws: self.prolog_ws.clone(),
            root,
        }
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_document().to_bytes()
    }
    pub fn name(&self) -> Option<&str> {
        self.csld_attrs.get("name")
    }
    /// Hidden slide (`show="0"`).
    pub fn hidden(&self) -> bool {
        self.attrs
            .get("show")
            .map(|v| v == "0" || v == "false")
            .unwrap_or(false)
    }
    pub fn set_hidden(&mut self, h: bool) {
        if h {
            self.attrs.set("show", "0");
        } else {
            self.attrs.remove("show");
        }
    }
    pub fn shapes(&self) -> &[Shape] {
        &self.tree.children
    }
    pub fn shapes_mut(&mut self) -> &mut Vec<Shape> {
        &mut self.tree.children
    }
    pub fn next_shape_id(&self) -> u32 {
        self.tree.max_id() + 1
    }
    /// All placeholders (top level), with their shapes.
    pub fn placeholders(&self) -> Vec<(&Placeholder, &Shape)> {
        self.tree
            .children
            .iter()
            .filter_map(|s| s.nv().and_then(|nv| nv.ph.as_ref()).map(|p| (p, s)))
            .collect()
    }
    /// Plain text of all text in the slide (reading order = tree order).
    pub fn plain_text(&self) -> String {
        let mut out = Vec::new();
        self.tree.walk(&mut |s, _| {
            if let Shape::Sp(sp) = s
                && let Some(t) = &sp.text
            {
                let t = t.plain_text();
                if !t.trim().is_empty() {
                    out.push(t);
                }
            }
        });
        out.join("\n")
    }
}
