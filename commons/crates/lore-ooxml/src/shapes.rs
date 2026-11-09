//! Constructors for new objects (text boxes, preset shapes, pictures, connectors,
//! groups) in the form PowerPoint itself writes them.

use crate::model::*;
use crate::xml::{Attrs, Element};

/// Rectangle in EMU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub cx: i64,
    pub cy: i64,
}

impl Rect {
    pub fn pt(x: f64, y: f64, w: f64, h: f64) -> Self {
        let e = |v: f64| (v * EMU_PER_PT as f64).round() as i64;
        Rect {
            x: e(x),
            y: e(y),
            cx: e(w),
            cy: e(h),
        }
    }
    fn xfrm(&self) -> Xfrm {
        Xfrm::new(self.x, self.y, self.cx, self.cy)
    }
}

/// A run with explicit font/size/bold/colour (what a style-pack tag expands to).
pub fn styled_run(
    text: &str,
    latin: &str,
    ea: Option<&str>,
    size_pt: f64,
    bold: bool,
    hex: Option<&str>,
) -> Run {
    let mut rp = RunProps::new();
    rp.attrs.set("lang", "en-US");
    rp.attrs.set("altLang", "zh-TW");
    rp.set_size_pt(size_pt);
    if bold {
        rp.attrs.set("b", "1");
    }
    rp.attrs.set("dirty", "0");
    if let Some(h) = hex {
        rp.fill = Some(Fill::solid(h));
    }
    rp.set_font(latin, ea);
    Run::Text {
        props: Some(rp),
        text: text.into(),
    }
}

pub fn paragraph(runs: Vec<Run>) -> Paragraph {
    Paragraph {
        props: None,
        runs,
        end_props: None,
    }
}

/// `p:sp` text box (`txBox="1"`, wrap square, shape auto-fit) like PowerPoint's Insert ▸ Text Box.
pub fn text_box(id: u32, name: &str, r: Rect, paragraphs: Vec<Paragraph>) -> Shape {
    let mut nv = NvProps::new("p:nvSpPr", "p:cNvSpPr", id, name);
    nv.kind.attrs.set("txBox", "1");
    let mut props = ShapeProps::new("p:spPr");
    props.xfrm = Some(r.xfrm());
    props.geom = Some(Geometry::preset("rect"));
    props.fill = Some(Fill::NoFill);
    let mut tb = TextBody::new("p:txBody");
    tb.body.attrs = Attrs(vec![
        ("wrap".into(), "square".into()),
        ("rtlCol".into(), "0".into()),
    ]);
    tb.body.autofit = Some(Autofit::Shape);
    tb.paragraphs = paragraphs;
    Shape::Sp(Sp {
        attrs: Attrs::new(),
        nv,
        props,
        style: None,
        text: Some(tb),
        extras: Extras::default(),
    })
}

/// Preset geometry shape (`rect`, `roundRect`, `ellipse`, `trapezoid`, …) with fill and outline.
pub fn preset_shape(
    id: u32,
    name: &str,
    prst: &str,
    r: Rect,
    fill: Option<Fill>,
    line: Option<Line>,
) -> Shape {
    let nv = NvProps::new("p:nvSpPr", "p:cNvSpPr", id, name);
    let mut props = ShapeProps::new("p:spPr");
    props.xfrm = Some(r.xfrm());
    props.geom = Some(Geometry::preset(prst));
    props.fill = fill;
    props.line = line;
    let mut tb = TextBody::new("p:txBody");
    tb.body.attrs = Attrs(vec![
        ("rtlCol".into(), "0".into()),
        ("anchor".into(), "ctr".into()),
    ]);
    tb.paragraphs.push(Paragraph {
        props: Some(ParaProps {
            attrs: Attrs(vec![("algn".into(), "ctr".into())]),
            ..ParaProps::new()
        }),
        runs: vec![],
        end_props: None,
    });
    Shape::Sp(Sp {
        attrs: Attrs::new(),
        nv,
        props,
        style: None,
        text: Some(tb),
        extras: Extras::default(),
    })
}

/// `p:pic` referencing an image relationship, optional crop and transparency.
pub fn picture(
    id: u32,
    name: &str,
    rel_id: &str,
    r: Rect,
    crop: Option<SrcRect>,
    alpha: Option<f64>,
) -> Shape {
    let mut nv = NvProps::new("p:nvPicPr", "p:cNvPicPr", id, name);
    nv.kind.locks = Some(Element::new("a:picLocks").with_attr("noChangeAspect", "1"));
    let mut blip = Blip {
        attrs: Attrs(vec![("r:embed".into(), rel_id.into())]),
        effects: vec![],
    };
    if alpha.is_some() {
        blip.set_alpha(alpha);
    }
    let mut fill = BlipFill {
        tag: "p:blipFill".into(),
        blip: Some(blip),
        ..Default::default()
    };
    fill.set_crop(crop);
    if fill.src_rect.is_none() {
        fill.src_rect = Some(Attrs::new());
    }
    fill.mode = Some(Element::new("a:stretch").with_child(Element::new("a:fillRect")));
    let mut props = ShapeProps::new("p:spPr");
    props.xfrm = Some(r.xfrm());
    props.geom = Some(Geometry::preset("rect"));
    Shape::Pic(Pic {
        attrs: Attrs::new(),
        nv,
        fill,
        props,
        style: None,
        extras: Extras::default(),
    })
}

/// Straight connector/arrow from (x1,y1) to (x2,y2) in EMU.
pub fn connector(id: u32, name: &str, from: (i64, i64), to: (i64, i64), line: Line) -> Shape {
    let nv = NvProps::new("p:nvCxnSpPr", "p:cNvCxnSpPr", id, name);
    let mut x = Xfrm::new(
        from.0.min(to.0),
        from.1.min(to.1),
        (to.0 - from.0).abs(),
        (to.1 - from.1).abs(),
    );
    if to.0 < from.0 {
        x.attrs.set("flipH", "1");
    }
    if to.1 < from.1 {
        x.attrs.set("flipV", "1");
    }
    let mut props = ShapeProps::new("p:spPr");
    props.xfrm = Some(x);
    props.geom = Some(Geometry::preset("straightConnector1"));
    props.line = Some(line);
    Shape::Cxn(Cxn {
        attrs: Attrs::new(),
        nv,
        props,
        style: None,
        extras: Extras::default(),
    })
}

/// Group the given shapes; child space = union of their boxes.
pub fn group(id: u32, name: &str, children: Vec<Shape>) -> Shape {
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for c in &children {
        if let Some(Xfrm {
            off: Some((x, y)),
            ext: Some((cx, cy)),
            ..
        }) = c.xfrm()
        {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(x + cx);
            y1 = y1.max(y + cy);
        }
    }
    if x0 == i64::MAX {
        (x0, y0, x1, y1) = (0, 0, 0, 0);
    }
    let mut x = Xfrm::new(x0, y0, x1 - x0, y1 - y0);
    x.ch_off = Some((x0, y0));
    x.ch_ext = Some((x1 - x0, y1 - y0));
    let mut props = ShapeProps::new("p:grpSpPr");
    props.xfrm = Some(x);
    Shape::Group(Group {
        tag: "p:grpSp".into(),
        attrs: Attrs::new(),
        nv: NvProps::new("p:nvGrpSpPr", "p:cNvGrpSpPr", id, name),
        props,
        children,
        ext_lst: None,
    })
}

/// Map a child-space rectangle of a group to slide space (one level).
pub fn child_to_parent(group_xfrm: &Xfrm, r: Rect) -> Rect {
    let (ox, oy) = group_xfrm.off.unwrap_or((0, 0));
    let (ex, ey) = group_xfrm.ext.unwrap_or((1, 1));
    let (cx0, cy0) = group_xfrm.ch_off.unwrap_or((ox, oy));
    let (cex, cey) = group_xfrm.ch_ext.unwrap_or((ex, ey));
    let sx = if cex == 0 {
        1.0
    } else {
        ex as f64 / cex as f64
    };
    let sy = if cey == 0 {
        1.0
    } else {
        ey as f64 / cey as f64
    };
    Rect {
        x: ox + ((r.x - cx0) as f64 * sx).round() as i64,
        y: oy + ((r.y - cy0) as f64 * sy).round() as i64,
        cx: (r.cx as f64 * sx).round() as i64,
        cy: (r.cy as f64 * sy).round() as i64,
    }
}

/// Absolute (slide-space) rectangles of every leaf shape, resolving nested group transforms.
pub fn absolute_rects(tree: &Group) -> Vec<(u32, Rect)> {
    fn rec(shapes: &[Shape], chain: &[Xfrm], out: &mut Vec<(u32, Rect)>) {
        for s in shapes {
            match s {
                Shape::Group(g) => {
                    let mut c = chain.to_vec();
                    if let Some(x) = &g.props.xfrm {
                        c.push(x.clone());
                    }
                    rec(&g.children, &c, out);
                }
                _ => {
                    if let Some(Xfrm {
                        off: Some((x, y)),
                        ext: Some((cx, cy)),
                        ..
                    }) = s.xfrm()
                    {
                        let mut r = Rect {
                            x: *x,
                            y: *y,
                            cx: *cx,
                            cy: *cy,
                        };
                        for g in chain.iter().rev() {
                            r = child_to_parent(g, r);
                        }
                        out.push((s.id().unwrap_or(0), r));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    rec(&tree.children, &[], &mut out);
    out
}
