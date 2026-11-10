//! Canvas bases (底版, design §7.2 v2): a base describes, for each page kind
//! (`cover`, `section`, `content`, `end`), the fixed elements that belong to the
//! slide layout (bands, logo, page number, background), the starter objects a
//! new slide of that kind begins with (editable: a cover photo under a colour
//! block, …) and named areas (title, content, takeaway) that arrangements fill.
//!
//! [`compile`] turns a base into a `template.pptx`: one master and one
//! slideLayout per page kind, the fixed elements written as layout shapes (so
//! PowerPoint keeps them out of the way), titles as layout placeholders. Apps
//! then open, render and export slides through the usual layout path.
//!
//! Colours may be written as roles (`"@accent-1"`); [`Base::from_json`] resolves
//! them against a scheme's named colours before anything else sees the base.

use std::collections::BTreeMap;

use lore_ooxml::Deck as ODeck;
use lore_ooxml::package::Package;
use serde::{Deserialize, Serialize};

use crate::model::*;
use crate::pptx_out::{Assets, ExportError, Out, object_out};

/// Page kinds in the order they appear in the template.
pub const PAGE_KINDS: [&str; 4] = ["cover", "section", "content", "end"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Base {
    #[serde(default = "one_u32")]
    pub format: u32,
    pub id: String,
    /// Named by structure (e.g. 「斜切封面＋頁首內頁」), never by colour.
    pub name: String,
    pub size: Size,
    #[serde(default)]
    pub fonts: BaseFonts,
    /// Page kind → page. `content` is required.
    pub pages: BTreeMap<String, BasePage>,
}

/// Theme fonts written into the template (Latin / East Asian; headings / body).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseFonts {
    #[serde(default = "calibri")]
    pub major: String,
    #[serde(default = "calibri")]
    pub minor: String,
    #[serde(default)]
    pub major_ea: String,
    #[serde(default)]
    pub minor_ea: String,
}
impl Default for BaseFonts {
    fn default() -> Self {
        BaseFonts { major: calibri(), minor: calibri(), major_ea: String::new(), minor_ea: String::new() }
    }
}
fn calibri() -> String {
    "Calibri".into()
}
fn one_u32() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BasePage {
    /// Layout (`cSld`) name in the compiled template; what `Slide.layout` refers to.
    pub layout: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<Paint>,
    /// Written into the slide layout (not editable on slides).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixed: Vec<Object>,
    /// Copied onto every new slide of this kind (editable there).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starter: Vec<Object>,
    /// Title placeholder (`ctrTitle` on a cover, `title` elsewhere).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<Frame>,
    /// Small line above the title (section number + name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kicker: Option<Frame>,
    /// Where arrangements (版型) place their slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Frame>,
    /// Optional band for one take-home line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub takeaway: Option<Frame>,
}

impl Base {
    /// Parse `base.json`, resolving `"@role"` colours with `colors` (role → hex).
    /// An unknown role is an error, so a typo never becomes black.
    pub fn from_json(text: &str, colors: &BTreeMap<String, String>) -> Result<Base, String> {
        let mut v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("base.json: {e}"))?;
        resolve_roles(&mut v, colors)?;
        let b: Base = serde_json::from_value(v).map_err(|e| format!("base.json: {e}"))?;
        if !b.pages.contains_key("content") {
            return Err("base.json: pages.content is required".into());
        }
        if let Some(k) = b.pages.keys().find(|k| !PAGE_KINDS.contains(&k.as_str())) {
            return Err(format!("base.json: unknown page kind {k:?} (cover, section, content, end)"));
        }
        Ok(b)
    }

    /// Pages in template order.
    pub fn ordered(&self) -> Vec<(&str, &BasePage)> {
        PAGE_KINDS.iter().filter_map(|k| self.pages.get(*k).map(|p| (*k, p))).collect()
    }
}

/// Replace every `"color": "@role"` (and gradient stop colours) by the scheme's hex.
pub fn resolve_roles(v: &mut serde_json::Value, colors: &BTreeMap<String, String>) -> Result<(), String> {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if k == "color"
                    && let serde_json::Value::String(s) = x
                    && let Some(role) = s.strip_prefix('@')
                {
                    let hex = colors.get(role).ok_or_else(|| format!("base.json: no colour for role @{role} in the scheme"))?;
                    *s = hex.trim_start_matches('#').to_uppercase();
                } else {
                    resolve_roles(x, colors)?;
                }
            }
        }
        serde_json::Value::Array(a) => {
            for x in a {
                resolve_roles(x, colors)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Compile → template.pptx
// ---------------------------------------------------------------------------

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT: &str = "application/vnd.openxmlformats-officedocument";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn rels_xml(items: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s = format!("{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    for (id, ty, target) in items {
        s += &format!("<Relationship Id=\"{id}\" Type=\"{R}/{ty}\" Target=\"{target}\"/>");
    }
    s += "</Relationships>";
    s.into_bytes()
}

/// Theme colours from the scheme roles (with neutral fallbacks).
fn theme_xml(base: &Base, colors: &BTreeMap<String, String>) -> Vec<u8> {
    let c = |role: &str, dflt: &str| colors.get(role).map(|h| h.trim_start_matches('#').to_uppercase()).unwrap_or_else(|| dflt.into());
    let ink = c("ink", "1B1B1F");
    let bg = c("slide-bg", "FFFFFF");
    let accents = [c("accent-1", "4472C4"), c("accent-2", "ED7D31"), c("label", "A5A5A5"), c("emphasis", "FFC000"), c("annotate", "5B9BD5"), c("muted", "70AD47")];
    let mut clr = format!(
        "<a:dk1><a:srgbClr val=\"{ink}\"/></a:dk1><a:lt1><a:srgbClr val=\"{bg}\"/></a:lt1><a:dk2><a:srgbClr val=\"{}\"/></a:dk2><a:lt2><a:srgbClr val=\"{}\"/></a:lt2>",
        c("inverse", &ink),
        c("surface", "E7E6E6")
    );
    for (i, a) in accents.iter().enumerate() {
        clr += &format!("<a:accent{n}><a:srgbClr val=\"{a}\"/></a:accent{n}>", n = i + 1);
    }
    clr += "<a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink><a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink>";
    let f = &base.fonts;
    let font = |latin: &str, ea: &str| format!("<a:latin typeface=\"{}\"/><a:ea typeface=\"{}\"/><a:cs typeface=\"\"/>", esc(latin), esc(ea));
    let solid = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let ln = |w: u32| format!("<a:ln w=\"{w}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\">{solid}<a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>");
    format!(
        "{DECL}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"{name}\"><a:themeElements>\
<a:clrScheme name=\"{name}\">{clr}</a:clrScheme>\
<a:fontScheme name=\"{name}\"><a:majorFont>{maj}</a:majorFont><a:minorFont>{min}</a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"{name}\"><a:fillStyleLst>{solid}{solid}{solid}</a:fillStyleLst><a:lnStyleLst>{l1}{l2}{l3}</a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>\
<a:bgFillStyleLst>{solid}{solid}{solid}</a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>",
        name = esc(&base.name),
        maj = font(&f.major, &f.major_ea),
        min = font(&f.minor, &f.minor_ea),
        l1 = ln(6350),
        l2 = ln(12700),
        l3 = ln(19050),
    )
    .into_bytes()
}

fn master_xml(n_layouts: usize) -> Vec<u8> {
    let ids: String = (0..n_layouts).map(|i| format!("<p:sldLayoutId id=\"{}\" r:id=\"rId{}\"/>", 2147483649u64 + i as u64, i + 1)).collect();
    let lvl = |tag: &str, sz: u32, font: &str| format!("<{tag}><a:lvl1pPr><a:defRPr sz=\"{sz}\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"{font}\"/><a:ea typeface=\"{font}\"/></a:defRPr></a:lvl1pPr></{tag}>");
    format!(
        "{DECL}<p:sldMaster {NS}><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst>{ids}</p:sldLayoutIdLst><p:txStyles>{t}{b}{o}</p:txStyles></p:sldMaster>",
        t = lvl("p:titleStyle", 3200, "+mj-lt"),
        b = lvl("p:bodyStyle", 1800, "+mn-lt"),
        o = lvl("p:otherStyle", 1800, "+mn-lt"),
    )
    .into_bytes()
}

fn layout_xml(name: &str) -> Vec<u8> {
    format!(
        "{DECL}<p:sldLayout {NS} type=\"cust\" preserve=\"1\"><p:cSld name=\"{}\"><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>",
        esc(name)
    )
    .into_bytes()
}

/// A layout placeholder for a title-like area (text from the scheme tags on new slides).
fn placeholder(id: u32, name: &str, ph: &str, frame: Frame) -> Object {
    Object {
        id,
        name: name.into(),
        frame,
        hidden: false,
        ph: Some(ph.into()),
        slot: None,
        kind: Kind::Text(SpObj {
            geom: Geom::preset("rect"),
            fill: Paint::None,
            line: None,
            text: Some(TextBody { paras: vec![Para::new(vec![])], ..Default::default() }),
            equation: false,
        }),
        src: None,
    }
}

/// `template.pptx` bytes for this base: one master, one slideLayout per page
/// kind (named `BasePage::layout`), zero slides. `colors` are the scheme's
/// role colours (theme colours; the base itself is already resolved).
pub fn compile(base: &Base, colors: &BTreeMap<String, String>, assets: &dyn Assets) -> Result<Vec<u8>, ExportError> {
    let pages = base.ordered();
    let n = pages.len();
    let mut pkg = Package { parts: Vec::new() };
    let mut ct = format!(
        "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/><Default Extension=\"jpg\" ContentType=\"image/jpeg\"/><Default Extension=\"gif\" ContentType=\"image/gif\"/><Default Extension=\"svg\" ContentType=\"image/svg+xml\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"{CT}.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"{CT}.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"{CT}.theme+xml\"/>\
<Override PartName=\"/ppt/presProps.xml\" ContentType=\"{CT}.presentationml.presProps+xml\"/>\
<Override PartName=\"/ppt/viewProps.xml\" ContentType=\"{CT}.presentationml.viewProps+xml\"/>\
<Override PartName=\"/ppt/tableStyles.xml\" ContentType=\"{CT}.presentationml.tableStyles+xml\"/>"
    );
    for i in 1..=n {
        ct += &format!("<Override PartName=\"/ppt/slideLayouts/slideLayout{i}.xml\" ContentType=\"{CT}.presentationml.slideLayout+xml\"/>");
    }
    ct += "</Types>";
    pkg.set("[Content_Types].xml", ct.into_bytes());
    pkg.set("_rels/.rels", rels_xml(&[("rId1", "officeDocument", "ppt/presentation.xml")]));
    pkg.set(
        "ppt/presentation.xml",
        format!(
            "{DECL}<p:presentation {NS} saveSubsetFonts=\"1\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>\
<p:sldSz cx=\"{}\" cy=\"{}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/><p:defaultTextStyle><a:lvl1pPr><a:defRPr sz=\"1800\"><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/></a:defRPr></a:lvl1pPr></p:defaultTextStyle></p:presentation>",
            base.size.w, base.size.h
        )
        .into_bytes(),
    );
    pkg.set(
        "ppt/_rels/presentation.xml.rels",
        rels_xml(&[
            ("rId1", "slideMaster", "slideMasters/slideMaster1.xml"),
            ("rId2", "presProps", "presProps.xml"),
            ("rId3", "viewProps", "viewProps.xml"),
            ("rId4", "theme", "theme/theme1.xml"),
            ("rId5", "tableStyles", "tableStyles.xml"),
        ]),
    );
    pkg.set("ppt/presProps.xml", format!("{DECL}<p:presentationPr {NS}/>").into_bytes());
    pkg.set("ppt/viewProps.xml", format!("{DECL}<p:viewPr {NS}><p:normalViewPr/><p:gridSpacing cx=\"76200\" cy=\"76200\"/></p:viewPr>").into_bytes());
    pkg.set("ppt/tableStyles.xml", format!("{DECL}<a:tblStyleLst xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" def=\"{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}\"/>").into_bytes());
    pkg.set("ppt/theme/theme1.xml", theme_xml(base, colors));
    pkg.set("ppt/slideMasters/slideMaster1.xml", master_xml(n));
    let mut mrels: Vec<(String, &str, String)> = (1..=n).map(|i| (format!("rId{i}"), "slideLayout", format!("../slideLayouts/slideLayout{i}.xml"))).collect();
    mrels.push((format!("rId{}", n + 1), "theme", "../theme/theme1.xml".into()));
    pkg.set(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels_xml(&mrels.iter().map(|(a, b, c)| (a.as_str(), *b, c.as_str())).collect::<Vec<_>>()),
    );
    for (i, (_, page)) in pages.iter().enumerate() {
        let part = format!("ppt/slideLayouts/slideLayout{}.xml", i + 1);
        pkg.set(&part, layout_xml(&page.layout));
        pkg.set(&format!("ppt/slideLayouts/_rels/slideLayout{}.xml.rels", i + 1), rels_xml(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]));
    }

    // fill each layout: background, fixed shapes, title placeholders
    let mut od = ODeck::from_package(pkg)?;
    for (i, (kind, page)) in pages.iter().enumerate() {
        let part = format!("ppt/slideLayouts/slideLayout{}.xml", i + 1);
        let mut objs: Vec<Object> = page.fixed.clone();
        let mut next = objs.iter().map(|o| o.id).max().unwrap_or(1) + 1;
        let mut ph = |name: &str, t: &str, f: &Option<Frame>, objs: &mut Vec<Object>| {
            if let Some(f) = f {
                objs.push(placeholder(next, name, t, f.clone()));
                next += 1;
            }
        };
        // cover and end pages carry a centred title (cover-title tag), others a slide title
        ph("Title", if *kind == "cover" || *kind == "end" { "ctrTitle" } else { "title" }, &page.title, &mut objs);
        ph("Subtitle", "subTitle", &page.subtitle, &mut objs);
        let rels = od.pkg.rels(&part)?;
        let mut out = Out { pkg: &mut od.pkg, part: part.clone(), rels, style: None, assets, next_id: next + 1000, slide: format!("layout{}", i + 1) };
        let shapes: Vec<_> = objs.iter().flat_map(|o| object_out(&mut out, o)).collect();
        let rels = out.rels;
        let li = od.layouts.iter().position(|l| l.part == part).ok_or_else(|| ExportError::Other(format!("{part} missing")))?;
        let doc = &mut od.layouts[li].doc;
        doc.tree.children = shapes;
        if let Some(bg) = &page.bg
            && let Some(e) = bg_element(bg)
        {
            doc.background = Some(e);
        }
        od.pkg.set(&part, doc.to_bytes());
        od.pkg.set_rels(&part, &rels);
    }
    // every part the shapes added (media) needs a content type: Defaults cover png/jpeg/gif/svg
    Ok(od.pkg.to_bytes()?)
}

/// `p:bg` for a solid or gradient background.
fn bg_element(p: &Paint) -> Option<lore_ooxml::xml::Element> {
    use lore_ooxml::xml::Element;
    let fill = match p {
        Paint::Solid { color, .. } => {
            let mut f = Element::new("a:solidFill");
            f.push(Element::new("a:srgbClr").with_attr("val", color.to_uppercase()));
            f
        }
        Paint::Gradient { stops, angle, path } => crate::pptx_out::grad_fill(stops, *angle, *path),
        _ => return None,
    };
    let mut pr = Element::new("p:bgPr");
    pr.push(fill);
    pr.push(Element::new("a:effectLst"));
    let mut bg = Element::new("p:bg");
    bg.push(pr);
    Some(bg)
}
