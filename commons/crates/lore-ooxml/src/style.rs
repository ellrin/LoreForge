//! Style packs (`styles/<id>/style.toml` + `template.pptx` + `preview/`) and the
//! statistical extractor that drafts tags/palette/components from real decks.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::deck::Deck;
use crate::model::*;

// ---------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StylePack {
    pub pack: PackInfo,
    /// Semantic layout name -> layout `cSld` name in template.pptx.
    #[serde(default)]
    pub layouts: BTreeMap<String, String>,
    /// Text tags (title, cover-title, venue, subtitle, body, body-dense, takeaway,
    /// label, caption, table, reference, equation).
    #[serde(default)]
    pub tags: BTreeMap<String, TagStyle>,
    #[serde(default)]
    pub palette: Palette,
    /// Emphasis tags (key, note, concept-1..n).
    #[serde(default)]
    pub emphasis: BTreeMap<String, Emphasis>,
    /// Visual components (numbered-circle, model-trapezoid, tab-card, highlight-box …)
    /// — free-form tables, interpreted by the editor.
    #[serde(default)]
    pub components: BTreeMap<String, toml::Table>,
    #[serde(default)]
    pub arrows: BTreeMap<String, ArrowStyle>,
    #[serde(default)]
    pub ghost: GhostRules,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub version: u32,
    /// Slide size in EMU.
    pub slide_cx: i64,
    pub slide_cy: i64,
    #[serde(default = "default_template")]
    pub template: String,
    #[serde(default)]
    pub notes: String,
}

fn default_template() -> String {
    "template.pptx".into()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TagStyle {
    /// Latin font.
    pub font: String,
    /// East-Asian font (defaults to `font`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ea_font: Option<String>,
    /// Substitute when `font` is not installed (preview/PDF renderers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// Size in pt.
    pub size: f64,
    /// Allowed range when the editor must shrink/grow (e.g. title 18–24).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_range: Option<[f64; 2]>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tint {
    pub hex: String,
    #[serde(default)]
    pub meaning: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    /// Named solid colours.
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    /// Semantic light tints.
    #[serde(default)]
    pub tints: BTreeMap<String, Tint>,
    /// Ordered colour sequences (e.g. numbered circles).
    #[serde(default)]
    pub sequences: BTreeMap<String, Vec<String>>,
    /// Named alpha levels (1.0 = opaque).
    #[serde(default)]
    pub alpha: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Emphasis {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub meaning: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArrowStyle {
    pub color: String,
    pub width_pt: f64,
    /// `straightConnector1`, `bentConnector3`, `curvedConnector3`.
    #[serde(default = "default_conn")]
    pub connector: String,
    #[serde(default = "default_dash")]
    pub dash: String,
    /// headEnd/tailEnd type (`triangle`, `none` …).
    #[serde(default)]
    pub head: String,
    #[serde(default = "default_tail")]
    pub tail: String,
}
fn default_conn() -> String {
    "straightConnector1".into()
}
fn default_dash() -> String {
    "solid".into()
}
fn default_tail() -> String {
    "triangle".into()
}

/// Progressive-reveal ("ghosting") rules for non-current objects.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GhostRules {
    /// Picture alphaModFix (fraction kept, e.g. 0.25).
    pub picture_alpha: f64,
    /// Line/arrow colour alpha.
    pub line_alpha: f64,
    /// Shape fill alpha.
    #[serde(default)]
    pub fill_alpha: f64,
    /// Text recoloured to this grey.
    pub text_color: String,
    /// Agenda lists: bold the current item instead of ghosting.
    #[serde(default)]
    pub agenda_bold_current: bool,
}

impl StylePack {
    pub fn from_toml(s: &str) -> std::result::Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("style pack serialises")
    }
    pub fn load_dir(dir: &std::path::Path) -> Result<Self> {
        let s = std::fs::read_to_string(dir.join("style.toml"))?;
        Self::from_toml(&s).map_err(|e| crate::Error::Invalid(format!("style.toml: {e}")))
    }
    /// Expand a tag (+ optional emphasis) into a run.
    pub fn run(&self, tag: &str, emphasis: Option<&str>, text: &str) -> Option<Run> {
        let t = self.tags.get(tag)?;
        let e = emphasis.and_then(|e| self.emphasis.get(e));
        let color = e.and_then(|e| e.color.clone()).or_else(|| t.color.clone());
        Some(crate::shapes::styled_run(
            text,
            &t.font,
            Some(t.ea_font.as_deref().unwrap_or(&t.font)),
            t.size,
            t.bold || e.map(|e| e.bold).unwrap_or(false),
            color.as_deref(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Extractor (statistical drafts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextKey {
    pub latin: String,
    pub ea: String,
    /// Size in 1/10 pt (integer for grouping).
    pub size_10: i64,
    pub bold: bool,
    pub color: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TextCluster {
    pub key: TextKey,
    pub chars: usize,
    pub runs: usize,
    /// Where it occurs: ph:title, ph:ctrTitle, textbox, shape, table, …
    pub roles: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ColorStat {
    pub hex: String,
    pub text: usize,
    pub fill: usize,
    pub line: usize,
    /// alpha (percent) -> count, for fills/lines with transparency.
    pub alphas: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Draft {
    pub decks: usize,
    pub slides: usize,
    pub slide_cx: i64,
    pub slide_cy: i64,
    /// layout name -> (type, slides using it)
    pub layouts: BTreeMap<String, (String, usize)>,
    /// layout name -> number of decks whose first slide uses it (cover candidates)
    pub first_slide_layouts: BTreeMap<String, usize>,
    pub clusters: Vec<TextCluster>,
    pub colors: Vec<ColorStat>,
    /// Picture alphaModFix percent -> count.
    pub picture_alpha: BTreeMap<String, usize>,
    /// Preset geometry -> count.
    pub geometries: BTreeMap<String, usize>,
    /// Small ellipse fills (numbered-circle candidates), most common first.
    pub circle_fills: Vec<(String, usize)>,
    /// Rotated trapezoid fills.
    pub trapezoid_fills: Vec<(String, usize)>,
    /// Dashed roundRect count (tab-card candidates).
    pub dashed_round_rects: usize,
    /// Connector line widths (pt*100) -> count.
    pub connector_widths: BTreeMap<String, usize>,
    pub dashes: BTreeMap<String, usize>,
    pub math_objects: usize,
    pub table_styles: BTreeMap<String, usize>,
}

fn role_of(sp: &Sp) -> String {
    match &sp.nv.ph {
        Some(p) => format!("ph:{}", p.kind()),
        None if sp.nv.kind.attrs.get_bool("txBox") == Some(true) => "textbox".into(),
        None => "shape".into(),
    }
}

/// Collect statistics from decks (no text content is stored, only counts).
pub fn extract_draft(decks: &[Deck]) -> Draft {
    let mut d = Draft {
        decks: decks.len(),
        ..Default::default()
    };
    let mut clusters: BTreeMap<TextKey, TextCluster> = BTreeMap::new();
    let mut colors: BTreeMap<String, ColorStat> = BTreeMap::new();
    let mut circles: BTreeMap<String, usize> = BTreeMap::new();
    let mut traps: BTreeMap<String, usize> = BTreeMap::new();
    fn col(m: &mut BTreeMap<String, ColorStat>, h: Option<String>) -> Option<&mut ColorStat> {
        let h = h?;
        Some(m.entry(h.clone()).or_insert_with(|| ColorStat {
            hex: h,
            ..Default::default()
        }))
    }
    for deck in decks {
        d.slide_cx = deck.slide_size.0;
        d.slide_cy = deck.slide_size.1;
        if let Some(l) = deck.slide_layout(0) {
            *d.first_slide_layouts
                .entry(l.name().to_string())
                .or_default() += 1;
        }
        for (si, s) in deck.slides.iter().enumerate() {
            d.slides += 1;
            if let Some(l) = deck.slide_layout(si) {
                let e = d
                    .layouts
                    .entry(l.name().to_string())
                    .or_insert((l.kind().to_string(), 0));
                e.1 += 1;
            }
            let mut texts: Vec<(String, &Sp)> = Vec::new();
            s.doc.tree.walk(&mut |sh, _| match sh {
                Shape::Sp(sp) => {
                    texts.push((role_of(sp), sp));
                    let geom = sp
                        .props
                        .geom
                        .as_ref()
                        .map(|g| g.name().to_string())
                        .unwrap_or_default();
                    if !geom.is_empty() {
                        *d.geometries.entry(geom.clone()).or_default() += 1;
                    }
                    let fill_hex = sp
                        .props
                        .fill
                        .as_ref()
                        .and_then(Fill::color)
                        .and_then(|c| deck.resolve_color(c));
                    let ext = sp.props.xfrm.as_ref().and_then(|x| x.ext).unwrap_or((0, 0));
                    if geom == "ellipse"
                        && ext.0 < 400_000
                        && ext.1 < 400_000
                        && let Some(h) = &fill_hex
                    {
                        *circles.entry(h.clone()).or_default() += 1;
                    }
                    if geom == "trapezoid"
                        && sp
                            .props
                            .xfrm
                            .as_ref()
                            .map(|x| x.rot_deg() != 0.0)
                            .unwrap_or(false)
                        && let Some(h) = &fill_hex
                    {
                        *traps.entry(h.clone()).or_default() += 1;
                    }
                    if geom == "roundRect"
                        && let Some(Dash::Preset(p)) =
                            sp.props.line.as_ref().and_then(|l| l.dash.as_ref())
                        && p != "solid"
                    {
                        d.dashed_round_rects += 1;
                    }
                    if let Some(c) = sp.props.fill.as_ref().and_then(Fill::color)
                        && let Some(cs) = col(&mut colors, deck.resolve_color(c))
                    {
                        cs.fill += 1;
                        if c.alpha() < 1.0 {
                            *cs.alphas
                                .entry(format!("{:.0}", c.alpha() * 100.0))
                                .or_default() += 1;
                        }
                    }
                    if let Some(c) = sp
                        .props
                        .line
                        .as_ref()
                        .and_then(|l| l.fill.as_ref())
                        .and_then(Fill::color)
                        && let Some(cs) = col(&mut colors, deck.resolve_color(c))
                    {
                        cs.line += 1;
                    }
                }
                Shape::Pic(p) => {
                    if let Some(b) = &p.fill.blip
                        && b.alpha() < 1.0
                    {
                        *d.picture_alpha
                            .entry(format!("{:.0}", b.alpha() * 100.0))
                            .or_default() += 1;
                    }
                }
                Shape::Cxn(c) => {
                    if let Some(l) = &c.props.line {
                        if let Some(w) = l.width_pt() {
                            *d.connector_widths.entry(format!("{w:.2}")).or_default() += 1;
                        }
                        if let Some(Dash::Preset(p)) = &l.dash {
                            *d.dashes.entry(p.clone()).or_default() += 1;
                        }
                        if let Some(c) = l.fill.as_ref().and_then(Fill::color)
                            && let Some(cs) = col(&mut colors, deck.resolve_color(c))
                        {
                            cs.line += 1;
                            if c.alpha() < 1.0 {
                                *cs.alphas
                                    .entry(format!("{:.0}", c.alpha() * 100.0))
                                    .or_default() += 1;
                            }
                        }
                    }
                }
                Shape::Frame(f) => {
                    if let FrameContent::Table(t) = &f.content {
                        *d.table_styles
                            .entry(t.style_id().unwrap_or_default())
                            .or_default() += 1;
                        for row in &t.rows {
                            for cell in &row.cells {
                                if let Some(tb) = &cell.text {
                                    for p in &tb.paragraphs {
                                        for r in &p.runs {
                                            if let Run::Text { props, text } = r {
                                                let mut res = crate::deck::ResolvedRun::default();
                                                if let Some(rp) = props {
                                                    res = deck.resolve_run(
                                                        si,
                                                        &dummy_sp(),
                                                        p,
                                                        Some(rp),
                                                    );
                                                }
                                                add_cluster(
                                                    &mut clusters,
                                                    &res,
                                                    "table",
                                                    text.chars().count(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Shape::Alt(a) if a.has_math() => d.math_objects += 1,
                _ => {}
            });
            for (role, sp) in texts {
                let Some(tb) = &sp.text else { continue };
                for p in &tb.paragraphs {
                    for r in &p.runs {
                        match r {
                            Run::Text { props, text } if !text.trim().is_empty() => {
                                let res = deck.resolve_run(si, sp, p, props.as_ref());
                                add_cluster(&mut clusters, &res, &role, text.chars().count());
                                if let Some(c) = props.as_ref().and_then(|p| p.color())
                                    && let Some(cs) = col(&mut colors, deck.resolve_color(c))
                                {
                                    cs.text += 1;
                                }
                            }
                            Run::Math(_) => d.math_objects += 1,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    let mut cl: Vec<TextCluster> = clusters.into_values().collect();
    cl.sort_by_key(|c| std::cmp::Reverse(c.chars));
    d.clusters = cl;
    let mut cs: Vec<ColorStat> = colors.into_values().collect();
    cs.sort_by_key(|c| std::cmp::Reverse(c.text + c.fill + c.line));
    d.colors = cs;
    let sort = |m: BTreeMap<String, usize>| {
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        v
    };
    d.circle_fills = sort(circles);
    d.trapezoid_fills = sort(traps);
    d
}

fn dummy_sp() -> Sp {
    Sp {
        attrs: Default::default(),
        nv: NvProps::new("p:nvSpPr", "p:cNvSpPr", 0, ""),
        props: ShapeProps::new("p:spPr"),
        style: None,
        text: None,
        extras: Extras::default(),
    }
}

fn add_cluster(
    m: &mut BTreeMap<TextKey, TextCluster>,
    r: &crate::deck::ResolvedRun,
    role: &str,
    chars: usize,
) {
    let key = TextKey {
        latin: r.latin.clone().unwrap_or_default(),
        ea: r.ea.clone().unwrap_or_default(),
        size_10: (r.size_pt.unwrap_or(18.0) * 10.0).round() as i64,
        bold: r.bold.unwrap_or(false),
        color: r.color.clone().unwrap_or_default(),
    };
    let c = m.entry(key.clone()).or_insert_with(|| TextCluster {
        key,
        ..Default::default()
    });
    c.chars += chars;
    c.runs += 1;
    *c.roles.entry(role.to_string()).or_default() += 1;
}

impl Draft {
    /// Turn statistics into a first StylePack: tags by rule of thumb, palette by
    /// usage. Meant to be hand-tuned afterwards.
    pub fn to_style_pack(&self, id: &str, name: &str) -> StylePack {
        let mut sp = StylePack {
            pack: PackInfo {
                id: id.into(),
                name: name.into(),
                source: format!("draft from {} deck(s), {} slides", self.decks, self.slides),
                version: 1,
                slide_cx: self.slide_cx,
                slide_cy: self.slide_cy,
                template: default_template(),
                notes: "statistical draft — hand-tune before use".into(),
            },
            ..Default::default()
        };
        // layouts: title-type -> cover, most used other -> content
        let mut ls: Vec<_> = self.layouts.iter().collect();
        ls.sort_by_key(|x| std::cmp::Reverse(x.1.1));
        let cover = ls
            .iter()
            .find(|(_, (t, _))| t == "title")
            .map(|(n, _)| (*n).clone())
            .or_else(|| {
                self.first_slide_layouts
                    .iter()
                    .max_by_key(|(_, n)| **n)
                    .map(|(n, _)| n.clone())
            });
        if let Some(c) = &cover {
            sp.layouts.insert("cover".into(), c.clone());
        }
        if let Some((n, _)) = ls.iter().find(|(n, _)| Some(*n) != cover.as_ref()) {
            sp.layouts.insert("content".into(), (*n).clone());
        }
        let tag = |c: &TextCluster| TagStyle {
            font: c.key.latin.clone(),
            ea_font: (c.key.ea != c.key.latin && !c.key.ea.is_empty()).then(|| c.key.ea.clone()),
            fallback: None,
            size: c.key.size_10 as f64 / 10.0,
            size_range: None,
            bold: c.key.bold,
            color: (!c.key.color.is_empty()).then(|| c.key.color.clone()),
            note: format!("{} chars, roles {:?}", c.chars, c.roles),
        };
        let pick = |f: &dyn Fn(&TextCluster) -> bool| {
            self.clusters
                .iter()
                .filter(|c| c.key.latin != "Cambria Math")
                .find(|c| f(c))
        };
        let role = |c: &TextCluster, r: &str| c.roles.contains_key(r);
        let size = |c: &TextCluster| c.key.size_10 as f64 / 10.0;
        let rules: Vec<(&str, Box<dyn Fn(&TextCluster) -> bool>)> = vec![
            ("cover-title", Box::new(move |c| role(c, "ph:ctrTitle"))),
            ("title", Box::new(move |c| role(c, "ph:title"))),
            ("subtitle", Box::new(move |c| role(c, "ph:subTitle"))),
            ("table", Box::new(move |c| role(c, "table"))),
            (
                "reference",
                Box::new(move |c| size(c) <= 9.0 && !role(c, "table")),
            ),
            (
                "label",
                Box::new(move |c| size(c) >= 15.0 && size(c) <= 17.0 && c.key.bold),
            ),
            (
                "takeaway",
                Box::new(move |c| size(c) >= 15.0 && size(c) <= 17.0 && !c.key.bold),
            ),
            (
                "body",
                Box::new(move |c| (size(c) - 14.0).abs() < 0.5 && !c.key.bold && !role(c, "table")),
            ),
            (
                "body-dense",
                Box::new(move |c| {
                    size(c) >= 10.5 && size(c) < 12.5 && !c.key.bold && !role(c, "table")
                }),
            ),
            (
                "caption",
                Box::new(move |c| {
                    (size(c) - 12.0).abs() < 0.5 && !c.key.bold && !role(c, "ph:title")
                }),
            ),
        ];
        for (name, f) in &rules {
            if let Some(c) = pick(f.as_ref()) {
                sp.tags.insert((*name).into(), tag(c));
            }
        }
        if self.math_objects > 0 {
            sp.tags.insert(
                "equation".into(),
                TagStyle {
                    font: "Cambria Math".into(),
                    size: 14.0,
                    note: format!("{} OMML objects", self.math_objects),
                    ..Default::default()
                },
            );
        }
        // palette
        for (i, c) in self.colors.iter().take(24).enumerate() {
            sp.palette
                .colors
                .insert(format!("c{:02}-{}", i + 1, c.hex), c.hex.clone());
        }
        if !self.circle_fills.is_empty() {
            sp.palette.sequences.insert(
                "numbered-circle".into(),
                self.circle_fills.iter().map(|(h, _)| h.clone()).collect(),
            );
        }
        if let Some((a, _)) = self.picture_alpha.iter().max_by_key(|(_, n)| **n) {
            sp.ghost.picture_alpha = a.parse::<f64>().unwrap_or(25.0) / 100.0;
        }
        sp.ghost.text_color = "A6A6A6".into();
        sp.ghost.line_alpha = 0.2;
        sp
    }
}
