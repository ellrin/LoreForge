//! The five entrance animations SlideComposer supports (appear, fade, fly in,
//! wipe, zoom), written as `p:timing` the way PowerPoint writes them:
//! tmRoot → mainSeq → one click group per `OnClick` step → effect `p:par`
//! (presetClass="entr") → `p:set style.visibility` + effect behaviours, with a
//! `p:bldLst` entry for text shapes.
//!
//! Preset ids/subtypes follow PowerPoint's built-in table (Appear 1, Fly In 2,
//! Fade 10, Wipe 22, Zoom 53). **Not verified against PowerPoint output on real
//! hardware** (no PowerPoint on the build machine) — see the TODOs.

use serde::{Deserialize, Serialize};

use crate::xml::{Element, parse_element};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entrance {
    Appear,
    Fade,
    /// Fly in from the bottom (presetSubtype 4).
    FlyIn,
    /// Wipe from the bottom (presetSubtype 4).
    Wipe,
    /// Zoom from the object centre (presetSubtype 16).
    Zoom,
}

impl Entrance {
    pub fn preset_id(self) -> u32 {
        match self {
            Entrance::Appear => 1,
            Entrance::FlyIn => 2,
            Entrance::Fade => 10,
            Entrance::Wipe => 22,
            Entrance::Zoom => 53,
        }
    }
    pub fn preset_subtype(self) -> u32 {
        match self {
            Entrance::Appear | Entrance::Fade => 0,
            Entrance::FlyIn | Entrance::Wipe => 4,
            // TODO(verify on PowerPoint): Zoom "Object Center" subtype 16.
            Entrance::Zoom => 16,
        }
    }
    fn from_preset(id: u32) -> Option<Self> {
        Some(match id {
            1 => Entrance::Appear,
            2 => Entrance::FlyIn,
            10 => Entrance::Fade,
            22 => Entrance::Wipe,
            53 => Entrance::Zoom,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    OnClick,
    WithPrevious,
    AfterPrevious,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntranceAnim {
    /// `p:cNvPr/@id` of the target shape.
    pub shape_id: u32,
    pub effect: Entrance,
    pub trigger: Trigger,
    /// Effect duration (ignored for Appear).
    pub duration_ms: u32,
    pub delay_ms: u32,
    /// Target has a text body (adds `p:bldP`; PowerPoint does this for sp, not pic).
    pub is_text_shape: bool,
}

impl EntranceAnim {
    pub fn new(shape_id: u32, effect: Entrance) -> Self {
        Self {
            shape_id,
            effect,
            trigger: Trigger::OnClick,
            duration_ms: 500,
            delay_ms: 0,
            is_text_shape: true,
        }
    }
}

struct Ids(u32);
impl Ids {
    fn next(&mut self) -> u32 {
        self.0 += 1;
        self.0
    }
}

fn behaviours(a: &EntranceAnim, ids: &mut Ids) -> String {
    let sp = a.shape_id;
    let tgt = format!(r#"<p:tgtEl><p:spTgt spid="{sp}"/></p:tgtEl>"#);
    let set = format!(
        r#"<p:set><p:cBhvr><p:cTn id="{}" dur="1" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst></p:cTn>{tgt}<p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="visible"/></p:to></p:set>"#,
        ids.next()
    );
    let d = a.duration_ms;
    let anim = |ids: &mut Ids, attr: &str, from: &str, to: &str| {
        format!(
            r#"<p:anim calcmode="lin" valueType="num"><p:cBhvr additive="base"><p:cTn id="{}" dur="{d}" fill="hold"/>{tgt}<p:attrNameLst><p:attrName>{attr}</p:attrName></p:attrNameLst></p:cBhvr><p:tavLst><p:tav tm="0"><p:val><p:strVal val="{from}"/></p:val></p:tav><p:tav tm="100000"><p:val><p:strVal val="{to}"/></p:val></p:tav></p:tavLst></p:anim>"#,
            ids.next()
        )
    };
    let effect = |ids: &mut Ids, filter: &str| {
        format!(
            r#"<p:animEffect transition="in" filter="{filter}"><p:cBhvr><p:cTn id="{}" dur="{d}"/>{tgt}</p:cBhvr></p:animEffect>"#,
            ids.next()
        )
    };
    match a.effect {
        Entrance::Appear => set,
        Entrance::Fade => set + &effect(ids, "fade"),
        Entrance::FlyIn => {
            set + &anim(ids, "ppt_x", "#ppt_x", "#ppt_x")
                + &anim(ids, "ppt_y", "1+#ppt_h/2", "#ppt_y")
        }
        // TODO(verify on PowerPoint): "From Bottom" wipe is believed to be filter wipe(down).
        Entrance::Wipe => set + &effect(ids, "wipe(down)"),
        // TODO(verify on PowerPoint): Zoom = ppt_w/ppt_h from 0 + fade.
        Entrance::Zoom => {
            set + &anim(ids, "ppt_w", "0", "#ppt_w")
                + &anim(ids, "ppt_h", "0", "#ppt_h")
                + &effect(ids, "fade")
        }
    }
}

/// Build a complete `p:timing` element for these entrance animations (in order).
pub fn build_timing(anims: &[EntranceAnim]) -> Element {
    let mut ids = Ids(2); // 1 = tmRoot, 2 = mainSeq
    // group into click steps: OnClick starts a new step; others join the current one
    let mut steps: Vec<Vec<&EntranceAnim>> = Vec::new();
    for a in anims {
        if a.trigger == Trigger::OnClick || steps.is_empty() {
            steps.push(vec![a]);
        } else {
            steps.last_mut().unwrap().push(a);
        }
    }
    let mut seq_children = String::new();
    for step in &steps {
        let click_id = ids.next();
        let first_is_click = step[0].trigger == Trigger::OnClick;
        // sub-groups: AfterPrevious starts a new timed sub-group
        let mut groups: Vec<(u32, Vec<&EntranceAnim>)> = Vec::new();
        let mut t = 0u32;
        for a in step {
            if a.trigger == Trigger::AfterPrevious && !groups.is_empty() {
                let prev_max = groups
                    .last()
                    .unwrap()
                    .1
                    .iter()
                    .map(|x| x.delay_ms + dur(x))
                    .max()
                    .unwrap_or(0);
                t += prev_max;
                groups.push((t, vec![a]));
            } else if groups.is_empty() {
                groups.push((0, vec![a]));
            } else {
                groups.last_mut().unwrap().1.push(a);
            }
        }
        let mut inner = String::new();
        for (gi, (delay, g)) in groups.iter().enumerate() {
            let gid = ids.next();
            let mut effects = String::new();
            for (ei, a) in g.iter().enumerate() {
                let node = if gi == 0 && ei == 0 && first_is_click {
                    "clickEffect"
                } else if ei == 0 && gi > 0 {
                    "afterEffect"
                } else {
                    "withEffect"
                };
                let eid = ids.next();
                let beh = behaviours(a, &mut ids);
                effects.push_str(&format!(
                    r#"<p:par><p:cTn id="{eid}" presetID="{}" presetClass="entr" presetSubtype="{}" fill="hold"{} nodeType="{node}"><p:stCondLst><p:cond delay="{}"/></p:stCondLst><p:childTnLst>{beh}</p:childTnLst></p:cTn></p:par>"#,
                    a.effect.preset_id(),
                    a.effect.preset_subtype(),
                    if a.is_text_shape { r#" grpId="0""# } else { "" },
                    a.delay_ms,
                ));
            }
            inner.push_str(&format!(
                r#"<p:par><p:cTn id="{gid}" fill="hold"><p:stCondLst><p:cond delay="{delay}"/></p:stCondLst><p:childTnLst>{effects}</p:childTnLst></p:cTn></p:par>"#
            ));
        }
        let start = if first_is_click {
            "indefinite".to_string()
        } else {
            "0".to_string()
        };
        seq_children.push_str(&format!(
            r#"<p:par><p:cTn id="{click_id}" fill="hold"><p:stCondLst><p:cond delay="{start}"/></p:stCondLst><p:childTnLst>{inner}</p:childTnLst></p:cTn></p:par>"#
        ));
    }
    let mut bld = String::new();
    let mut seen = Vec::new();
    for a in anims.iter().filter(|a| a.is_text_shape) {
        if !seen.contains(&a.shape_id) {
            seen.push(a.shape_id);
            bld.push_str(&format!(
                r#"<p:bldP spid="{}" grpId="0" animBg="1"/>"#,
                a.shape_id
            ));
        }
    }
    let bld = if bld.is_empty() {
        String::new()
    } else {
        format!("<p:bldLst>{bld}</p:bldLst>")
    };
    let xml = format!(
        r#"<p:timing xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst><p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst>{seq_children}</p:childTnLst></p:cTn><p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq></p:childTnLst></p:cTn></p:par></p:tnLst>{bld}</p:timing>"#
    );
    let mut el = parse_element(xml.as_bytes()).expect("timing template is well-formed");
    el.attrs.remove("xmlns:p"); // declared on the slide root
    el
}

fn dur(a: &EntranceAnim) -> u32 {
    if a.effect == Entrance::Appear {
        1
    } else {
        a.duration_ms
    }
}

/// Read back entrance animations from a `p:timing` (ours or PowerPoint's).
/// Effects outside the five supported presets are skipped; returns `None`
/// when the timing has no main sequence.
pub fn read_timing(timing: &Element) -> Option<Vec<EntranceAnim>> {
    let main = timing
        .find_all("p:cTn")
        .into_iter()
        .find(|c| c.attr("nodeType") == Some("mainSeq"))?;
    let bld: Vec<u32> = timing
        .find_all("p:bldP")
        .iter()
        .filter_map(|b| b.attr("spid").and_then(|v| v.parse().ok()))
        .collect();
    let mut out = Vec::new();
    for c in main.find_all("p:cTn") {
        if c.attr("presetClass") != Some("entr") {
            continue;
        }
        let Some(effect) = c
            .attr("presetID")
            .and_then(|v| v.parse().ok())
            .and_then(Entrance::from_preset)
        else {
            continue;
        };
        let trigger = match c.attr("nodeType") {
            Some("withEffect") => Trigger::WithPrevious,
            Some("afterEffect") => Trigger::AfterPrevious,
            _ => Trigger::OnClick,
        };
        let spid = c
            .find_all("p:spTgt")
            .first()
            .and_then(|t| t.attr("spid"))
            .and_then(|v| v.parse().ok())?;
        let duration_ms = c
            .find_all("p:cTn")
            .iter()
            .filter(|x| x.attr("dur").is_some_and(|d| d != "1" && d != "indefinite"))
            .filter_map(|x| x.attr("dur").and_then(|d| d.parse().ok()))
            .next()
            .unwrap_or(500);
        let delay_ms = c
            .child("p:stCondLst")
            .and_then(|s| s.child("p:cond"))
            .and_then(|x| x.attr("delay"))
            .and_then(|d| d.parse().ok())
            .unwrap_or(0);
        out.push(EntranceAnim {
            shape_id: spid,
            effect,
            trigger,
            duration_ms,
            delay_ms,
            is_text_shape: bld.contains(&spid),
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_effects_roundtrip_through_timing() {
        let mut v = vec![
            EntranceAnim::new(4, Entrance::Appear),
            EntranceAnim::new(5, Entrance::Fade),
            EntranceAnim::new(6, Entrance::FlyIn),
            EntranceAnim::new(7, Entrance::Wipe),
            EntranceAnim::new(8, Entrance::Zoom),
        ];
        v[2].trigger = Trigger::WithPrevious;
        v[3].trigger = Trigger::AfterPrevious;
        v[4].is_text_shape = false;
        let t = build_timing(&v);
        let xml = t.to_xml_string();
        assert!(xml.contains(r#"presetID="10""#) && xml.contains(r#"filter="fade""#));
        assert!(xml.contains("ppt_y") && xml.contains("wipe(down)") && xml.contains("ppt_w"));
        assert!(xml.contains(r#"<p:bldP spid="4""#) && !xml.contains(r#"<p:bldP spid="8""#));
        // unique cTn ids
        let ids: Vec<_> = t
            .find_all("p:cTn")
            .iter()
            .map(|c| c.attr("id").unwrap().to_string())
            .collect();
        let mut d = ids.clone();
        d.sort();
        d.dedup();
        assert_eq!(ids.len(), d.len());
        let back = read_timing(&t).unwrap();
        assert_eq!(back.len(), 5);
        for (a, b) in v.iter().zip(&back) {
            assert_eq!(
                (a.shape_id, a.effect, a.trigger),
                (b.shape_id, b.effect, b.trigger)
            );
        }
    }
}
