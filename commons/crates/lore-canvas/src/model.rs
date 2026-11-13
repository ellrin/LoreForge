//! Scene model: deck → slides → objects. Plain serde data (the editor's JSON).
//!
//! Units: positions/sizes in EMU, rotation in degrees (clockwise), font sizes in
//! pt, colours as 6-digit RGB hex, alpha as opacity fraction (1.0 = opaque),
//! picture crop as fractions of the source image.
//!
//! Objects imported from PPTX carry `src` (their original XML plus per-field
//! fingerprints taken at import time): an object whose fields still match its
//! fingerprints is written back verbatim, so an unedited deck exports
//! byte-identically; an edited object is *patched* field by field onto its
//! original XML, keeping everything the model does not know about.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub type Emu = i64;
pub const EMU_PER_PT: f64 = 12_700.0;
pub const EMU_PER_INCH: f64 = 914_400.0;

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn is_one(v: &f64) -> bool {
    *v == 1.0
}
fn one() -> f64 {
    1.0
}

/// Position, size, rotation and flips of an object (in its parent's coordinate space).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub x: Emu,
    pub y: Emu,
    pub w: Emu,
    pub h: Emu,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rot: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_h: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_v: bool,
}

impl Frame {
    pub fn new(x: Emu, y: Emu, w: Emu, h: Emu) -> Self {
        Frame {
            x,
            y,
            w,
            h,
            ..Default::default()
        }
    }
    pub fn center(&self) -> (f64, f64) {
        (
            self.x as f64 + self.w as f64 / 2.0,
            self.y as f64 + self.h as f64 / 2.0,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub w: Emu,
    pub h: Emu,
}

// ---------------------------------------------------------------------------
// Deck / slide
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deck {
    /// Format version of deck.json.
    pub format: u32,
    pub size: Size,
    /// Style pack id (`styles/<id>/`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    pub slides: Vec<Slide>,
    /// Counter for new slide uids.
    #[serde(default)]
    pub next_uid: u64,
    /// Default arrangement (版型) per page kind (`cover`, `content`, `end`) for new slides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, String>,
}

impl Deck {
    pub fn new(w: Emu, h: Emu, style: Option<String>) -> Self {
        Deck {
            format: 1,
            size: Size { w, h },
            style,
            slides: vec![],
            next_uid: 1,
            defaults: BTreeMap::new(),
        }
    }
    pub fn new_uid(&mut self) -> String {
        let u = format!("s{}", self.next_uid.max(1));
        self.next_uid = self.next_uid.max(1) + 1;
        u
    }
    pub fn slide(&self, uid: &str) -> Option<&Slide> {
        self.slides.iter().find(|s| s.uid == uid)
    }
    pub fn slide_mut(&mut self, uid: &str) -> Option<&mut Slide> {
        self.slides.iter_mut().find(|s| s.uid == uid)
    }
    pub fn slide_index(&self, uid: &str) -> Option<usize> {
        self.slides.iter().position(|s| s.uid == uid)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    /// Stable id inside the deck (`s12`); never reused.
    pub uid: String,
    /// Layout name (`cSld/@name` of the layout in the template).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Speaker notes, plain text (one paragraph per line).
    #[serde(default)]
    pub notes: String,
    /// Arrangement (版型) the slide was laid out with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrangement: Option<String>,
    pub objects: Vec<Object>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anims: Vec<Anim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<SlideSrc>,
}

/// Where an imported slide came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideSrc {
    /// Slide part in the source package (`ppt/slides/slide3.xml`).
    pub part: String,
    /// Fingerprint of the slide content at import time.
    pub k: u64,
    /// Fingerprint of the notes text at import time.
    #[serde(default)]
    pub notes_k: u64,
    /// Fingerprint of the animation list at import time.
    #[serde(default)]
    pub anims_k: u64,
}

impl Object {
    /// An arrangement slot nobody filled yet: an image slot that is still a
    /// shape, or a slot whose text has nothing but whitespace. Left out of
    /// exports and shows.
    pub fn is_empty_slot(&self) -> bool {
        // colour panels of a recipe (`deco-…`) are part of the design, never empty
        match self.slot.as_deref() {
            None => return false,
            Some(s) if s.starts_with("deco") => return false,
            _ => {}
        }
        let blank = |t: &TextBody| t.paras.iter().all(|p| p.runs.iter().all(|r| r.kind != "math" && r.text.trim().is_empty()));
        match &self.kind {
            Kind::Text(sp) | Kind::Shape(sp) => sp.text.as_ref().is_none_or(blank),
            Kind::Table(t) => t.rows.iter().all(|r| r.cells.iter().all(|c| blank(&c.text))),
            _ => false,
        }
    }
}

impl Slide {
    pub fn new(uid: String, layout: Option<String>) -> Self {
        Slide {
            uid,
            layout,
            hidden: false,
            notes: String::new(),
            arrangement: None,
            objects: vec![],
            anims: vec![],
            src: None,
        }
    }
    /// Find an object anywhere in the tree (groups included).
    pub fn find(&self, id: u32) -> Option<&Object> {
        fn rec(v: &[Object], id: u32) -> Option<&Object> {
            for o in v {
                if o.id == id {
                    return Some(o);
                }
                if let Kind::Group(g) = &o.kind
                    && let Some(f) = rec(&g.children, id)
                {
                    return Some(f);
                }
            }
            None
        }
        rec(&self.objects, id)
    }
    pub fn find_mut(&mut self, id: u32) -> Option<&mut Object> {
        fn rec(v: &mut [Object], id: u32) -> Option<&mut Object> {
            for o in v.iter_mut() {
                if o.id == id {
                    return Some(o);
                }
                if let Kind::Group(g) = &mut o.kind
                    && let Some(f) = rec(&mut g.children, id)
                {
                    return Some(f);
                }
            }
            None
        }
        rec(&mut self.objects, id)
    }
    /// (parent group id or None for the slide, index) of an object.
    pub fn locate(&self, id: u32) -> Option<(Option<u32>, usize)> {
        fn rec(v: &[Object], parent: Option<u32>, id: u32) -> Option<(Option<u32>, usize)> {
            for (i, o) in v.iter().enumerate() {
                if o.id == id {
                    return Some((parent, i));
                }
                if let Kind::Group(g) = &o.kind
                    && let Some(f) = rec(&g.children, Some(o.id), id)
                {
                    return Some(f);
                }
            }
            None
        }
        rec(&self.objects, None, id)
    }
    /// The child list of `parent` (None = slide top level).
    pub fn list_mut(&mut self, parent: Option<u32>) -> Option<&mut Vec<Object>> {
        match parent {
            None => Some(&mut self.objects),
            Some(p) => match &mut self.find_mut(p)?.kind {
                Kind::Group(g) => Some(&mut g.children),
                _ => None,
            },
        }
    }
    pub fn list(&self, parent: Option<u32>) -> Option<&Vec<Object>> {
        match parent {
            None => Some(&self.objects),
            Some(p) => match &self.find(p)?.kind {
                Kind::Group(g) => Some(&g.children),
                _ => None,
            },
        }
    }
    pub fn max_id(&self) -> u32 {
        fn rec(v: &[Object], m: &mut u32) {
            for o in v {
                *m = (*m).max(o.id);
                if let Kind::Group(g) = &o.kind {
                    rec(&g.children, m);
                }
            }
        }
        let mut m = 1;
        rec(&self.objects, &mut m);
        m
    }
    /// Visit every object depth-first.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Object)) {
        fn rec<'a>(v: &'a [Object], f: &mut dyn FnMut(&'a Object)) {
            for o in v {
                f(o);
                if let Kind::Group(g) = &o.kind {
                    rec(&g.children, f);
                }
            }
        }
        rec(&self.objects, f)
    }
    pub fn walk_mut(&mut self, f: &mut dyn FnMut(&mut Object)) {
        fn rec(v: &mut [Object], f: &mut dyn FnMut(&mut Object)) {
            for o in v.iter_mut() {
                f(o);
                if let Kind::Group(g) = &mut o.kind {
                    rec(&mut g.children, f);
                }
            }
        }
        rec(&mut self.objects, f)
    }
}

/// Entrance animation (the five SlideComposer supports).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anim {
    pub obj: u32,
    /// appear | fade | fly_in | wipe | zoom
    pub effect: String,
    /// click | with | after
    #[serde(default = "default_trigger")]
    pub trigger: String,
    #[serde(default = "default_dur")]
    pub duration_ms: u32,
    #[serde(default)]
    pub delay_ms: u32,
}
fn default_trigger() -> String {
    "click".into()
}
fn default_dur() -> u32 {
    500
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    /// Shape id (`cNvPr/@id`), unique within the slide.
    pub id: u32,
    #[serde(default)]
    pub name: String,
    pub frame: Frame,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Placeholder type when the object is a layout placeholder (`title`, `ctrTitle`, `sldNum` …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ph: Option<String>,
    /// Arrangement slot (`photo`, `cards[1]` …) this object fills. An empty slot
    /// is drawn dashed in the editor and left out of shows and exports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    #[serde(flatten)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<ObjSrc>,
}

/// Original XML of an imported object + per-field fingerprints at import time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjSrc {
    pub xml: String,
    pub k: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    /// Text box, placeholder or equation box (`p:sp` with txBox / ph / a14:m).
    Text(SpObj),
    /// Geometric shape (`p:sp`), optionally with text.
    Shape(SpObj),
    Connector(ConnObj),
    Picture(PicObj),
    Group(GroupObj),
    Table(TableObj),
    /// Opaque object kept verbatim (chart, SmartArt, OLE, ink …).
    Locked(LockedObj),
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Text(_) => "text",
            Kind::Shape(_) => "shape",
            Kind::Connector(_) => "connector",
            Kind::Picture(_) => "picture",
            Kind::Group(_) => "group",
            Kind::Table(_) => "table",
            Kind::Locked(_) => "locked",
        }
    }
    pub fn sp(&self) -> Option<&SpObj> {
        match self {
            Kind::Text(s) | Kind::Shape(s) => Some(s),
            _ => None,
        }
    }
    pub fn sp_mut(&mut self) -> Option<&mut SpObj> {
        match self {
            Kind::Text(s) | Kind::Shape(s) => Some(s),
            _ => None,
        }
    }
}

/// Preset or custom geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geom {
    /// Preset name (`rect`, `roundRect`, `ellipse`, `triangle`, `trapezoid`, …) or `custom`.
    pub prst: String,
    /// Adjust values (`adj`, `adj1` …) in the preset's units (usually 1/100000).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adj: BTreeMap<String, f64>,
    /// Custom geometry: SVG path data in a 0..1 × 0..1 box (`M`, `L`, `C`, `Q`, `Z`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl Geom {
    pub fn preset(p: &str) -> Self {
        Geom {
            prst: p.into(),
            adj: BTreeMap::new(),
            path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    /// 0..1
    pub pos: f64,
    pub color: String,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub alpha: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Paint {
    #[default]
    None,
    Solid {
        color: String,
        #[serde(default = "one", skip_serializing_if = "is_one")]
        alpha: f64,
    },
    /// Linear (angle in degrees) or path gradient; display only, kept raw on export.
    Gradient {
        stops: Vec<Stop>,
        #[serde(default)]
        angle: f64,
        #[serde(default, skip_serializing_if = "is_false")]
        path: bool,
    },
    /// Picture / pattern fill etc.: display hint only, kept raw.
    Other { label: String },
}

impl Paint {
    pub fn solid(hex: &str) -> Self {
        Paint::Solid {
            color: hex.to_uppercase(),
            alpha: 1.0,
        }
    }
    pub fn is_none(&self) -> bool {
        matches!(self, Paint::None)
    }
}

/// Outline / connector line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: String,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub alpha: f64,
    pub width_pt: f64,
    /// solid | dash | sysDash | sysDot | dashDot | lgDash …
    #[serde(default = "solid")]
    pub dash: String,
    /// Arrowhead at the start: none | triangle | stealth | arrow | oval | diamond
    #[serde(default = "none_str")]
    pub head: String,
    #[serde(default = "none_str")]
    pub tail: String,
}
fn solid() -> String {
    "solid".into()
}
fn none_str() -> String {
    "none".into()
}

impl Stroke {
    pub fn new(hex: &str, width_pt: f64) -> Self {
        Stroke {
            color: hex.to_uppercase(),
            alpha: 1.0,
            width_pt,
            dash: solid(),
            head: none_str(),
            tail: none_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpObj {
    pub geom: Geom,
    #[serde(default)]
    pub fill: Paint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Stroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextBody>,
    /// Office Math box (`mc:AlternateContent` with `a14:m`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub equation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attach {
    /// Target object id.
    pub id: u32,
    /// Connection site index.
    pub site: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnObj {
    /// straightConnector1 | bentConnector2..5 | curvedConnector2..5 | line
    pub geom: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adj: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Stroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Attach>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Attach>,
    /// Text on the connector (rare; kept for display).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextBody>,
}

/// Crop as fractions of the source image (negative = padding).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub l: f64,
    pub t: f64,
    pub r: f64,
    pub b: f64,
}

/// A rectangle in fractions of the picture frame (white mask patch).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct FracRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PicObj {
    /// `pkg:<part>` (inside the source package) or `file:<path in deck folder>`.
    pub asset: String,
    #[serde(default)]
    pub crop: Crop,
    #[serde(default = "one")]
    pub alpha: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Stroke>,
    /// Clip geometry (`rect`, `ellipse`, `roundRect` …).
    #[serde(default = "rect_str")]
    pub clip: String,
    /// White patches drawn over the picture (exported as white rectangles).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub masks: Vec<FracRect>,
    /// The picture carries effects the editor does not model (background removal, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<String>,
}
fn rect_str() -> String {
    "rect".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupObj {
    /// Child coordinate space (chOff / chExt).
    pub ch: Frame,
    pub children: Vec<Object>,
    /// One picture: resized with its aspect ratio kept, and text sizes, line
    /// widths and arrowheads scale with it ([`crate::Op::Scale`]). PPTX:
    /// `a:grpSpLocks noChangeAspect="1"`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lock_aspect: bool,
    /// App data riding with the group (a flow figure's spec, …). Not part of the
    /// PPTX shape: an app keeps it in its own state part to survive a round trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockedObj {
    /// e.g. `graphicFrame:chart`, `p:contentPart`.
    pub kind: String,
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableObj {
    pub cols: Vec<Emu>,
    pub rows: Vec<Row>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub first_row: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub band_row: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub h: Emu,
    pub cells: Vec<Cell>,
    /// Index of the source row this row was copied from (export reuses its XML).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub text: TextBody,
    #[serde(default = "one_u", skip_serializing_if = "is_one_u")]
    pub span: u32,
    #[serde(default = "one_u", skip_serializing_if = "is_one_u")]
    pub rspan: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hmerge: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub vmerge: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Borders left, right, top, bottom (None = inherit from table style).
    #[serde(default)]
    pub borders: [Option<Stroke>; 4],
    /// Source cell (row, col) whose `tcPr` is reused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<[u32; 2]>,
}
fn one_u() -> u32 {
    1
}
fn is_one_u(v: &u32) -> bool {
    *v == 1
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBody {
    /// Insets left, top, right, bottom (PowerPoint's default when omitted).
    #[serde(default = "default_insets")]
    pub insets: [Emu; 4],
    /// t | ctr | b
    #[serde(default = "top")]
    pub anchor: String,
    #[serde(default = "yes")]
    pub wrap: bool,
    /// none | norm | shape
    #[serde(default = "none_str")]
    pub autofit: String,
    /// normAutofit fontScale (1.0 = 100 %).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub font_scale: f64,
    /// normAutofit lnSpcReduction (0.2 = 20 %).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ln_reduction: f64,
    /// vert attribute (horz, vert, vert270, eaVert …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vert: Option<String>,
    pub paras: Vec<Para>,
}
fn top() -> String {
    "t".into()
}
fn yes() -> bool {
    true
}

fn default_insets() -> [Emu; 4] {
    [91_440, 45_720, 91_440, 45_720]
}

impl Default for TextBody {
    fn default() -> Self {
        TextBody {
            insets: default_insets(),
            anchor: top(),
            wrap: true,
            autofit: none_str(),
            font_scale: 1.0,
            ln_reduction: 0.0,
            vert: None,
            paras: vec![],
        }
    }
}

impl TextBody {
    pub fn plain_text(&self) -> String {
        self.paras
            .iter()
            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Bullet {
    None,
    Char { ch: String },
    /// Auto numbering (`arabicPeriod`, `alphaLcPeriod`, `romanLcPeriod` …).
    Num {
        scheme: String,
        #[serde(default = "one_u")]
        start: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Para {
    /// l | ctr | r | just | dist
    #[serde(default = "left")]
    pub align: String,
    #[serde(default)]
    pub level: u32,
    #[serde(default = "no_bullet")]
    pub bullet: Bullet,
    /// Left margin and first-line indent.
    #[serde(default)]
    pub mar_l: Emu,
    #[serde(default)]
    pub indent: Emu,
    /// Line spacing as a multiple (1.0 = single), or exact points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_pt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_pt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_pt: Option<f64>,
    pub runs: Vec<Run>,
    /// Look of the paragraph end (height of an empty paragraph).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Look>,
    /// Source paragraph index and fingerprint of the paragraph properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<u32>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub k: u64,
}
fn left() -> String {
    "l".into()
}
fn no_bullet() -> Bullet {
    Bullet::None
}
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

impl Para {
    pub fn new(runs: Vec<Run>) -> Self {
        Para {
            align: left(),
            level: 0,
            bullet: Bullet::None,
            mar_l: 0,
            indent: 0,
            line: None,
            line_pt: None,
            before_pt: None,
            after_pt: None,
            runs,
            end: None,
            src: None,
            k: 0,
        }
    }
}

/// Rendered look of a run (resolved through tag/emphasis or, for imported text
/// without a tag, through PPTX inheritance).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Look {
    pub font: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ea_font: String,
    pub size: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub strike: bool,
    pub color: String,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub alpha: f64,
    /// Superscript (> 0) / subscript (< 0) offset in percent.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub baseline: f64,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            font: "Calibri".into(),
            ea_font: String::new(),
            size: 18.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: "000000".into(),
            alpha: 1.0,
            baseline: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// text | br | field | math
    #[serde(default = "text_kind")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    /// Style-pack text tag (`body`, `title` …). `None` only for imported text
    /// whose formatting matches no tag (kept as is until a tag is applied).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Emphasis tag (`key`, `note`, `concept-1` …, `strong`, `ghost`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub em: Option<String>,
    pub look: Look,
    /// Field type (`slidenum`, `datetime1` …) for kind = field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Office Math XML (`a14:m`, PowerPoint flavour) for kind = math.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omml: Option<String>,
    /// LaTeX source of the math (typed by the user, or converted from OMML).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latex: Option<String>,
    /// Source (paragraph, run) indices and fingerprint of the run formatting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub k: u64,
}
fn text_kind() -> String {
    "text".into()
}

impl Run {
    pub fn text(text: &str, tag: &str, look: Look) -> Self {
        Run {
            kind: text_kind(),
            text: text.into(),
            tag: Some(tag.into()),
            em: None,
            look,
            field: None,
            omml: None,
            latex: None,
            src: None,
            k: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Fingerprints
// ---------------------------------------------------------------------------

/// FNV-1a 64 over bytes.
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Stable fingerprint of any serialisable value (its JSON form).
pub fn fp<T: Serialize + ?Sized>(v: &T) -> u64 {
    fnv(serde_json::to_string(v).unwrap_or_default().as_bytes())
}

/// Fingerprint of a paragraph's own properties (not its runs / src).
pub fn para_k(p: &Para) -> u64 {
    let mut q = p.clone();
    q.runs.clear();
    q.src = None;
    q.k = 0;
    fp(&q)
}

/// Fingerprint of a run's formatting (not its text / src).
pub fn run_k(r: &Run) -> u64 {
    let mut q = r.clone();
    q.text.clear();
    q.src = None;
    q.k = 0;
    if q.kind == "math" {
        // math identity = its OMML; latex is a cache
        q.latex = None;
    }
    fp(&q)
}

/// Per-field fingerprints of an object (frame, name, hidden, ph + every field
/// of its kind except group children).
pub fn object_ks(o: &Object) -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    m.insert("frame".into(), fp(&o.frame));
    m.insert("name".into(), fp(&o.name));
    m.insert("hidden".into(), fp(&o.hidden));
    m.insert("type".into(), fp(o.kind.name()));
    let v = match &o.kind {
        Kind::Text(s) | Kind::Shape(s) => serde_json::to_value(s),
        Kind::Connector(c) => serde_json::to_value(c),
        Kind::Picture(p) => serde_json::to_value(p),
        Kind::Group(g) => serde_json::to_value(&g.ch).map(|ch| serde_json::json!({ "ch": ch, "ids": g.children.iter().map(|c| c.id).collect::<Vec<_>>() })),
        Kind::Table(t) => serde_json::to_value(t),
        Kind::Locked(l) => serde_json::to_value(l),
    }
    .unwrap_or_default();
    if let serde_json::Value::Object(map) = v {
        for (k, v) in map {
            m.insert(k, fp(&v));
        }
    }
    m
}

/// Names of fields that differ from the import-time fingerprints (empty = untouched).
/// Objects without `src` report `["*new*"]`.
pub fn changed_fields(o: &Object) -> Vec<String> {
    let Some(src) = &o.src else {
        return vec!["*new*".into()];
    };
    let now = object_ks(o);
    let mut out: Vec<String> = now
        .iter()
        .filter(|(k, v)| src.k.get(*k) != Some(v))
        .map(|(k, _)| k.clone())
        .collect();
    for k in src.k.keys() {
        if !now.contains_key(k) {
            out.push(k.clone());
        }
    }
    out
}

/// True when the object (and, for groups, every descendant) is unedited.
pub fn untouched(o: &Object) -> bool {
    if !changed_fields(o).is_empty() {
        return false;
    }
    match &o.kind {
        Kind::Group(g) => g.children.iter().all(untouched),
        _ => true,
    }
}

/// Fingerprint of a slide's content (what decides whether its XML is regenerated).
pub fn slide_k(s: &Slide) -> u64 {
    fp(&(&s.layout, s.hidden, &s.objects))
}
