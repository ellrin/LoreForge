//! Style-pack components → canvas objects (numbered circle, model trapezoid,
//! tab card, highlight box, semantic overlay) and arrow styles → connectors.

use std::collections::BTreeMap;

use crate::model::*;
use crate::style::StyleSystem;

fn tget<'a>(t: &'a toml_like::Table, k: &str) -> Option<&'a toml_like::Value> {
    t.get(k)
}

/// The components table is free-form TOML; we read it through serde_json.
mod toml_like {
    pub type Table = serde_json::Map<String, serde_json::Value>;
    pub type Value = serde_json::Value;
}

fn text_with(st: &StyleSystem, tag: &str, s: &str, align: &str) -> TextBody {
    let look = st.look(tag, None).unwrap_or_default();
    let mut p = Para::new(vec![Run::text(s, tag, look.clone())]);
    p.align = align.into();
    p.end = Some(look);
    TextBody {
        insets: [0, 0, 0, 0],
        anchor: "ctr".into(),
        wrap: false,
        paras: vec![p],
        ..TextBody::default()
    }
}

/// Instantiate component `name` at (x, y) EMU. `n` = sequence number (numbered
/// circle: the digit and n-th colour; others: variant). `first_id` = id of the
/// (first) new object; groups use consecutive ids.
pub fn instantiate(
    st: &StyleSystem,
    name: &str,
    x: Emu,
    y: Emu,
    n: usize,
    first_id: u32,
) -> Option<Object> {
    let comp = st.pack.components.get(name)?;
    let t: toml_like::Table = serde_json::to_value(comp).ok()?.as_object()?.clone();
    let s = |k: &str| tget(&t, k).and_then(|v| v.as_str()).map(str::to_string);
    let f = |k: &str| tget(&t, k).and_then(|v| v.as_f64());
    let inch = |v: f64| (v * EMU_PER_INCH).round() as Emu;
    let tint0 = || {
        st.pack
            .palette
            .tints
            .values()
            .next()
            .map(|t| t.hex.clone())
            .unwrap_or_else(|| "EDEDED".into())
    };
    let stroke = |c: Option<String>, w: f64| -> Option<Stroke> {
        match c.as_deref() {
            None | Some("none") => None,
            Some(c) => Some(Stroke::new(c, w)),
        }
    };
    let base = |kind: Kind, frame: Frame, nm: &str| Object {
        id: first_id,
        name: nm.into(),
        frame,
        hidden: false,
        ph: None,
        slot: None,
        kind,
        src: None,
    };
    let obj = match name {
        "numbered-circle" => {
            let d = inch(f("size_in").unwrap_or(0.24));
            let seq = s("fills").unwrap_or_else(|| "numbered-circle".into());
            let fill = st
                .color(&format!("{seq}[{}]", n.saturating_sub(1)))
                .unwrap_or_else(|| "FFFFFF".into());
            let tag = s("text_tag").unwrap_or_else(|| "caption".into());
            base(
                Kind::Shape(SpObj {
                    geom: Geom::preset("ellipse"),
                    fill: Paint::solid(&fill),
                    line: stroke(s("line"), 0.75),
                    text: Some(text_with(st, &tag, &n.max(1).to_string(), "ctr")),
                    equation: false,
                }),
                Frame::new(x, y, d, d),
                &format!("編號 {n}"),
            )
        }
        "model-trapezoid" => {
            let fills: Vec<String> = std::iter::once(s("fill").unwrap_or_else(|| "F7D9F0".into()))
                .chain(
                    tget(&t, "alt_fills")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>())
                        .unwrap_or_default(),
                )
                .collect();
            let fill = fills[n.saturating_sub(1) % fills.len()].clone();
            let mut fr = Frame::new(x, y, inch(1.0), inch(0.6));
            fr.rot = f("rotation_deg").unwrap_or(90.0);
            base(
                Kind::Shape(SpObj {
                    geom: Geom::preset("trapezoid"),
                    fill: Paint::solid(&fill),
                    line: stroke(s("line"), 1.0),
                    text: None,
                    equation: false,
                }),
                fr,
                "模型",
            )
        }
        "highlight-box" => base(
            Kind::Shape(SpObj {
                geom: Geom::preset(&s("geometry").unwrap_or_else(|| "rect".into())),
                fill: Paint::None,
                line: stroke(s("line").or(Some("FF0000".into())), f("line_width_pt").unwrap_or(2.25)),
                text: None,
                equation: false,
            }),
            Frame::new(x, y, inch(1.5), inch(0.5)),
            "重點框",
        ),
        "semantic-overlay" => {
            let alpha = s("alpha")
                .and_then(|a| st.pack.palette.alpha.get(&a).copied())
                .unwrap_or(0.35);
            let tints: Vec<&String> = st.pack.palette.tints.keys().collect();
            let hex = tints
                .get(n.saturating_sub(1) % tints.len().max(1))
                .and_then(|k| st.color(k))
                .unwrap_or_else(tint0);
            base(
                Kind::Shape(SpObj {
                    geom: Geom::preset("rect"),
                    fill: Paint::Solid { color: hex, alpha },
                    line: None,
                    text: None,
                    equation: false,
                }),
                Frame::new(x, y, inch(2.0), inch(1.0)),
                "語意遮罩",
            )
        }
        "tab-card" => {
            let tints: Vec<&String> = st.pack.palette.tints.keys().collect();
            let hex = tints
                .get(n.saturating_sub(1) % tints.len().max(1))
                .and_then(|k| st.color(k))
                .unwrap_or_else(tint0);
            let (w, h) = (inch(3.0), inch(2.0));
            let lh = inch(f("label_height_in").unwrap_or(0.26));
            let mut frame_line = Stroke::new("000000", 1.0);
            frame_line.dash = s("frame_dash").unwrap_or_else(|| "dash".into());
            let mut adj = BTreeMap::new();
            adj.insert("adj".into(), 6000.0);
            let card = Object {
                id: first_id + 1,
                name: "標籤卡框".into(),
                frame: Frame::new(x, y + lh / 2, w, h),
                hidden: false,
                ph: None,
                slot: None,
                kind: Kind::Shape(SpObj {
                    geom: Geom {
                        prst: s("frame_geometry").unwrap_or_else(|| "roundRect".into()),
                        adj: adj.clone(),
                        path: None,
                    },
                    fill: Paint::solid(&hex),
                    line: Some(frame_line),
                    text: None,
                    equation: false,
                }),
                src: None,
            };
            let tag = s("label_text_tag").unwrap_or_else(|| "label".into());
            let label = Object {
                id: first_id + 2,
                name: "標籤卡標籤".into(),
                frame: Frame::new(x + inch(0.1), y, inch(1.2), lh),
                hidden: false,
                ph: None,
                slot: None,
                kind: Kind::Shape(SpObj {
                    geom: Geom {
                        prst: s("label_geometry").unwrap_or_else(|| "roundRect".into()),
                        adj: BTreeMap::new(),
                        path: None,
                    },
                    fill: Paint::solid(&hex),
                    line: Some(Stroke::new("000000", 1.0)),
                    text: Some(text_with(st, &tag, "標籤", "ctr")),
                    equation: false,
                }),
                src: None,
            };
            let fr = crate::geom::bounds([&card.frame, &label.frame].into_iter());
            Object {
                id: first_id,
                name: "標籤卡".into(),
                frame: fr.clone(),
                hidden: false,
                ph: None,
                slot: None,
                kind: Kind::Group(GroupObj {
                    ch: fr,
                    children: vec![card, label],
                    lock_aspect: false,
                    data: None,
                }),
                src: None,
            }
        }
        _ => return None,
    };
    Some(obj)
}

/// Connector with a style-pack arrow style from `a` to `b`.
pub fn arrow(st: &StyleSystem, style: &str, a: (f64, f64), b: (f64, f64), id: u32) -> Option<Object> {
    let s = st.pack.arrows.get(style)?;
    let mut l = Stroke::new(&s.color, s.width_pt);
    l.dash = s.dash.clone();
    l.head = if s.head.is_empty() { "none".into() } else { s.head.clone() };
    l.tail = s.tail.clone();
    Some(Object {
        id,
        name: format!("箭頭 {id}"),
        frame: crate::geom::connector_frame(a, b),
        hidden: false,
        ph: None,
        slot: None,
        kind: Kind::Connector(ConnObj {
            geom: s.connector.clone(),
            adj: BTreeMap::new(),
            line: Some(l),
            start: None,
            end: None,
            text: None,
        }),
        src: None,
    })
}
