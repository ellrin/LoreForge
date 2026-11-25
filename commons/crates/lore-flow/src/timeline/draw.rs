//! The plan as objects (package T-draw). Signatures are frozen; see `timeline/mod.rs`.

use lore_canvas::model::Stop;
use lore_canvas::{Geom, Kind, Object, Paint, SpObj, Stroke};

use super::{Plan, PointGeom, Side, Slot, Style, TimelineSpec};
use crate::build::{Built, node_name};
use crate::layout::Rect;
use crate::modules::{frame, obj, poly, round_adj, seg, shape};
use crate::text::{NoMath, Type, Vars, body};
use crate::theme::shade;

/// Every part of `plan` as a `lore_canvas` object, back to front (axis, stems, markers, cards,
/// texts), ids from `first_id` up. Names are `flow:<fig>/<part>` ([`crate::build::node_name`]
/// with the parts `axis`, `title`, `legend-<i>`, and for point `i` (from 1) `p<i>`,
/// `p<i>-stem`, `p<i>-time`, `p<i>-title`, `p<i>-body`, `p<i>-card`, `p<i>-icon` …), so
/// [`crate::build::is_flow_object`] finds them when the figure is edited again.
pub fn objects(spec: &TimelineSpec, style: &Style, plan: &Plan, fonts: &crate::theme::Font, first_id: u32) -> Built {
    let mut d = Drawer { spec, style, plan, fonts, out: Built { objects: vec![], node_ids: Default::default(), next_id: first_id, pictures: vec![], warnings: vec![] } };
    d.axis();
    d.legend();
    for (i, pg) in plan.points.iter().enumerate() {
        d.point(i + 1, pg);
    }
    d.texts();
    d.out
}

// ── colours ──────────────────────────────────────────────────────────────────

fn hex_ok(s: &str) -> bool {
    s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// A colour field: `none` = no colour, `point` = the point's, `paper` = white, six hex digits
/// as written, anything else = `fallback`.
fn pick(v: &str, point: &str, fallback: &str) -> Option<String> {
    match v.trim() {
        "none" => None,
        "point" => Some(point.to_uppercase()),
        "paper" => Some("FFFFFF".into()),
        h if hex_ok(h) => Some(h.to_uppercase()),
        _ => Some(fallback.to_uppercase()),
    }
}

/// A text colour field (`none` falls back to `fallback`).
fn pick_text(v: &str, point: &str, fallback: &str) -> String {
    pick(v, point, fallback).unwrap_or_else(|| fallback.to_uppercase())
}

fn dark(hex: &str) -> bool {
    let p = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
    0.2126 * p(0) + 0.7152 * p(2) + 0.0722 * p(4) < 0.6
}

fn solid(c: &Option<String>) -> Paint {
    match c {
        Some(h) => Paint::solid(h),
        None => Paint::None,
    }
}

/// `tabler:…` / `healthicons:…`: written as an icon id (it may still not exist).
fn looks_like_id(s: &str) -> bool {
    s.split_once(':').is_some_and(|(a, _)| a == "tabler" || a == "healthicons")
}

// ── the drawer ───────────────────────────────────────────────────────────────

struct Drawer<'a> {
    spec: &'a TimelineSpec,
    style: &'a Style,
    plan: &'a Plan,
    fonts: &'a crate::theme::Font,
    out: Built,
}

impl Drawer<'_> {
    fn k(&self) -> f64 {
        if self.plan.scale.is_finite() && self.plan.scale > 0.0 { self.plan.scale } else { 1.0 }
    }

    fn push(&mut self, part: &str, mut o: Object) -> u32 {
        let id = self.out.next_id;
        o.id = id;
        o.name = node_name(self.spec.fig_id(), part);
        self.out.next_id += 1;
        self.out.objects.push(o);
        id
    }

    fn fill(&self, c: &Option<String>) -> Paint {
        match c {
            Some(h) if self.style.colors.gradient => Paint::Gradient {
                stops: vec![Stop { pos: 0.0, color: shade(h, 0.35), alpha: 1.0 }, Stop { pos: 1.0, color: h.clone(), alpha: 1.0 }],
                angle: 0.0,
                path: false,
            },
            _ => solid(c),
        }
    }

    fn line_of(&self, c: &Option<String>, w: f64) -> Option<Stroke> {
        c.as_ref().filter(|_| w > 0.0).map(|h| Stroke::new(h, w))
    }

    fn rounded(&self, r: &Rect, radius: f64, fill: Paint, line: Option<Stroke>) -> Object {
        let mut o = shape("", "roundRect", r.x, r.y, r.w, r.h, fill, line);
        let short = r.w.min(r.h);
        if short > 0.0 {
            round_adj(&mut o, (radius * self.k() / short).clamp(0.0, 0.5));
        }
        o
    }

    fn circle(&self, cx: f64, cy: f64, d: f64, fill: Paint, line: Option<Stroke>) -> Object {
        shape("", "ellipse", cx - d / 2.0, cy - d / 2.0, d, d, fill, line)
    }

    // 1. axis
    fn axis(&mut self) {
        let (ax, st, k) = (&self.plan.axis, self.style, self.k());
        let muted = st.colors.muted.to_uppercase();
        if st.axis.kind == "joins" {
            for (j, w) in self.plan.points.windows(2).enumerate() {
                let (a, b) = (&w[0].marker, &w[1].marker);
                let (p, q) = ((a.x + a.w, a.y + a.h / 2.0), (b.x, b.y + b.h / 2.0));
                if q.0 <= p.0 {
                    continue;
                }
                let mut s = Stroke::new(&muted, 0.75 * k);
                s.dash = "sysDash".into();
                let o = seg("", p, q, s);
                self.push(&format!("join-{}", j + 1), o);
            }
            return;
        }
        if ax.thickness.is_nan() || ax.thickness <= 0.0 || ax.x1 <= ax.x0 {
            return;
        }
        match st.axis.kind.as_str() {
            "line" => {
                let Some(c) = pick(&st.axis.color, &muted, &muted) else { return };
                let mut s = Stroke::new(&c, ax.thickness);
                s.dash = st.axis.dash.clone();
                let o = seg("", (ax.x0, ax.y), (ax.x1, ax.y), s);
                self.push("axis", o);
            }
            "band" => {
                let y = ax.y - ax.thickness / 2.0;
                let pts = &self.plan.points;
                let one = pts.first().map(|p| p.color.to_uppercase()).filter(|c| pts.iter().all(|p| p.color.eq_ignore_ascii_case(c)));
                if let (true, Some(c)) = (st.axis.color.trim() == "point", &one) {
                    // every point has the same colour: one band, no seams
                    let o = shape("", "rect", ax.x0, y, ax.x1 - ax.x0, ax.thickness, Paint::solid(c), None);
                    self.push("axis", o);
                } else if st.axis.color.trim() == "point" && !pts.is_empty() {
                    for (i, p) in pts.iter().enumerate() {
                        let l = if i == 0 { ax.x0 } else { (pts[i - 1].x + p.x) / 2.0 };
                        let r = if i + 1 == pts.len() { ax.x1 } else { (p.x + pts[i + 1].x) / 2.0 };
                        let c = Some(p.color.to_uppercase());
                        let o = shape("", "rect", l, y, (r - l).max(0.0), ax.thickness, solid(&c), None);
                        self.push(&format!("axis-{}", i + 1), o);
                    }
                } else {
                    let Some(c) = pick(&st.axis.color, &muted, &muted) else { return };
                    let o = shape("", "rect", ax.x0, y, ax.x1 - ax.x0, ax.thickness, Paint::solid(&c), None);
                    self.push("axis", o);
                }
            }
            "chevrons" => self.chevrons(),
            "path" => self.path(),
            _ => {}
        }
    }

    /// One arrow-shaped piece per point, pointing right, a small gap between neighbours.
    fn chevrons(&mut self) {
        let (ax, st, k) = (self.plan.axis.clone(), self.style, self.k());
        let muted = st.colors.muted.to_uppercase();
        let pts = &self.plan.points;
        let n = pts.len();
        let (t, gap) = (0.35 * ax.thickness, 2.0 * k);
        let (top, bot, mid) = (ax.y - ax.thickness / 2.0, ax.y + ax.thickness / 2.0, ax.y);
        let per_point = st.axis.color.trim() == "point";
        let base = pick(&st.axis.color, &muted, &muted).unwrap_or_else(|| muted.clone());
        let mut made = vec![];
        for (i, p) in pts.iter().enumerate() {
            let l = if i == 0 { ax.x0 } else { (pts[i - 1].x + p.x) / 2.0 };
            let r = if i + 1 == n { ax.x1 } else { (p.x + pts[i + 1].x) / 2.0 };
            let rb = if i + 1 == n { r - t } else { r - gap };
            let mut v = vec![(l, top), (rb, top), (rb + t, mid), (rb, bot), (l, bot)];
            if i > 0 {
                v.push((l + t, mid));
            }
            let c = if per_point {
                p.color.to_uppercase()
            } else if i % 2 == 1 {
                shade(&base, 0.3)
            } else {
                base.clone()
            };
            made.push((i, poly("", &v, true, Paint::solid(&c), None)));
        }
        for (i, o) in made {
            self.push(&format!("axis-{}", i + 1), o);
        }
    }

    /// A thick line through the markers as a square wave; each piece steps to the next height.
    fn path(&mut self) {
        let (ax, st) = (self.plan.axis.clone(), self.style);
        let muted = st.colors.muted.to_uppercase();
        let pts = &self.plan.points;
        let n = pts.len();
        let hy = |p: &PointGeom| p.marker.y + p.marker.h / 2.0;
        let mut made = vec![];
        for (i, p) in pts.iter().enumerate() {
            let l = if i == 0 { ax.x0 } else { (pts[i - 1].x + p.x) / 2.0 };
            let r = if i + 1 == n { ax.x1 } else { (p.x + pts[i + 1].x) / 2.0 };
            let h = hy(p);
            let mut v = vec![(l, h), (r, h)];
            if i + 1 < n {
                let h2 = hy(&pts[i + 1]);
                if (h2 - h).abs() > 1e-9 {
                    v.push((r, h2));
                }
            }
            let Some(c) = pick(&st.axis.color, &p.color, &muted) else { return };
            made.push((i, poly("", &v, false, Paint::None, Some(Stroke::new(&c, ax.thickness)))));
        }
        for (i, o) in made {
            self.push(&format!("axis-{}", i + 1), o);
        }
    }

    // 2. legend dots
    fn legend(&mut self) {
        let k = self.k();
        for (i, (x, y)) in self.plan.legend.iter().enumerate() {
            let c = self.plan.points.get(i).map(|p| p.color.clone()).unwrap_or_else(|| self.style.colors.ink.clone());
            let o = self.circle(*x, *y, 6.0 * k, Paint::solid(&c), None);
            self.push(&format!("legend-{}", i + 1), o);
        }
    }

    fn marker_fill(&self, pg: &PointGeom) -> Option<String> {
        pick(&self.style.marker.fill, &pg.color, &pg.color)
    }

    // 3. one point
    fn point(&mut self, n: usize, pg: &PointGeom) {
        let st = self.style;
        let k = self.k();
        let p = format!("p{n}");
        let pc = pg.color.to_uppercase();
        let m = &pg.marker;

        // stem
        if let Some(line) = pg.stem {
            let c = pick(&st.stem.color, &pc, &pc);
            match st.stem.kind.as_str() {
                "line" => {
                    if let Some(s) = self.line_of(&c, st.stem.weight * k) {
                        let o = seg("", line[0], line[1], s);
                        self.push(&format!("{p}-stem"), o);
                    }
                    if st.stem.end == "dot" {
                        let o = self.circle(line[1].0, line[1].1, 4.0 * k, solid(&c), None);
                        self.push(&format!("{p}-stem-end"), o);
                    }
                }
                "drop" => {
                    let half = m.w * 0.4 / 2.0;
                    let pts = [(line[0].0 - half, line[0].1), (line[0].0 + half, line[0].1), line[1]];
                    let o = poly("", &pts, true, solid(&c), None);
                    self.push(&format!("{p}-stem"), o);
                }
                _ => {}
            }
        }

        // card and its strip
        if let Some(r) = &pg.card {
            let c = pick(&st.card.fill, &pc, &pc);
            let o = self.rounded(r, st.card.radius, self.fill(&c), None);
            self.push(&format!("{p}-card"), o);
            if let Some(h) = &pg.card_header {
                let dk = Some(shade(c.as_deref().unwrap_or(&pc), -0.25));
                let o = shape("", "rect", h.x, h.y, h.w, h.h, solid(&dk), None);
                self.push(&format!("{p}-card-head"), o);
            }
        }

        // marker
        let fill = self.marker_fill(pg);
        let line = pick(&st.marker.line, &pc, &pc);
        let lw = st.marker.line_weight * k;
        let (cx, cy) = (m.x + m.w / 2.0, m.y + m.h / 2.0);
        let main = match st.marker.kind.as_str() {
            "square" | "block" => self.rounded(m, st.marker.radius, self.fill(&fill), self.line_of(&line, lw)),
            "circle" => shape("", "ellipse", m.x, m.y, m.w, m.h, self.fill(&fill), self.line_of(&line, lw)),
            "ring" => self.circle(cx, cy, m.w.min(m.h) * 0.5, self.fill(&fill), None),
            _ => shape("", "ellipse", m.x, m.y, m.w, m.h, self.fill(&fill), None),
        };
        let id = self.push(&p, main);
        self.out.node_ids.insert(p.clone(), id);
        if st.marker.kind == "ring" {
            let rings = st.marker.rings;
            let d = m.w.min(m.h);
            for r in 1..=rings {
                let dia = d * (0.5 + 0.5 * r as f64 / rings as f64);
                let c = Some(if r == 1 { pc.clone() } else { st.colors.muted.to_uppercase() });
                let o = if rings >= 3 && r == rings {
                    // the outermost ring is a half ring, on the side away from the text
                    let sign = if pg.side == Side::Below { -1.0 } else { 1.0 };
                    let rad = dia / 2.0;
                    let arc: Vec<(f64, f64)> = (0..=24).map(|j| {
                        let a = std::f64::consts::PI * j as f64 / 24.0;
                        (cx + rad * a.cos(), cy + sign * rad * a.sin())
                    }).collect();
                    poly("", &arc, false, Paint::None, self.line_of(&c, lw))
                } else {
                    self.circle(cx, cy, dia, Paint::None, self.line_of(&c, lw))
                };
                self.push(&format!("{p}-ring-{r}"), o);
            }
        }

        // rule
        if let Some(l) = pg.rule {
            let o = seg("", l[0], l[1], Stroke::new(&pc, 1.5 * k));
            self.push(&format!("{p}-rule"), o);
        }

        // icon slot
        if let Some(r) = &pg.icon {
            let c = self.icon_color(&fill);
            let word = self.spec.points.get(n - 1).map(|sp| sp.icon.trim().to_string()).unwrap_or_default();
            if lore_icons::exists(&word) {
                if let Some(ic) = lore_icons::icon(&word) {
                    // inset 15 % of the slot's side, drawn in the slot's colour
                    let (dx, dy) = (r.w * 0.15, r.h * 0.15);
                    let fr = frame(r.x + dx, r.y + dy, (r.w - 2.0 * dx).max(0.0), (r.h - 2.0 * dy).max(0.0));
                    let objs = lore_icons::objects(&ic, fr, &c, 0, "");
                    for (j, o) in objs.into_iter().enumerate() {
                        let part = if j == 0 { format!("{p}-icon") } else { format!("{p}-icon-{}", j + 1) };
                        self.push(&part, o);
                    }
                    return;
                }
                self.out.warnings.push(format!("圖示 {word} 無法畫出"));
            } else if looks_like_id(&word) {
                self.out.warnings.push(format!("找不到圖示 {word}"));
            }
            let mut s = Stroke::new(&c, 0.75 * k);
            s.dash = "sysDash".into();
            let o = shape("", "ellipse", r.x, r.y, r.w, r.h, Paint::None, Some(s));
            self.push(&format!("{p}-icon"), o);
        }
    }

    /// What the icon slot of point `i` (0-based) says: the words, or an unknown id's name.
    fn slot_words(&self, i: usize) -> String {
        let w = self.spec.points.get(i).map(|sp| sp.icon.trim()).unwrap_or("");
        if looks_like_id(w) { w.split_once(':').map(|x| x.1).unwrap_or(w).to_string() } else { w.to_string() }
    }

    fn icon_color(&self, marker_fill: &Option<String>) -> String {
        if self.style.icon.at == "in-marker"
            && let Some(f) = marker_fill
            && dark(f)
        {
            return "FFFFFF".into();
        }
        self.style.colors.muted.to_uppercase()
    }

    // 4. texts
    fn ty(&self, size: f64, bold: bool, color: &str) -> Type {
        Type { font: self.fonts.family.clone(), ea_font: self.fonts.ea.clone(), math_font: self.fonts.math.clone(), size, bold, italic: false, color: color.to_uppercase() }
    }

    fn text(&mut self, part: &str, slot: &Slot, ty: &Type, align: &str, anchor: &str) {
        let (tb, has_math) = body(&slot.text, ty, &Vars::default(), &NoMath, align, anchor);
        let r = &slot.rect;
        let o = obj("", frame(r.x, r.y, r.w, r.h), Kind::Text(SpObj { geom: Geom::preset("rect"), fill: Paint::None, line: None, text: Some(tb), equation: has_math }));
        self.push(part, o);
    }

    fn texts(&mut self) {
        let (st, plan, spec) = (self.style, self.plan, self.spec);
        let ink = st.colors.ink.clone();
        let k = self.k();
        let anchor_of = |side: Side| if side == Side::Above { "b" } else { "t" };
        for (i, pg) in plan.points.iter().enumerate() {
            let p = format!("p{}", i + 1);
            let pc = pg.color.to_uppercase();
            let in_card = pg.card.is_some();
            let text_anchor = anchor_of(pg.side);
            let text_align = if in_card || st.text.align != "left" { "ctr" } else { "l" };

            // what picture to put in the empty icon slot
            let words = self.slot_words(i);
            if let Some(r) = &pg.icon
                && !words.is_empty()
                && !lore_icons::exists(spec.points[i].icon.trim())
            {
                let c = self.icon_color(&self.marker_fill(pg));
                let slot = Slot { rect: *r, text: words, size: 7.0 * k };
                let ty = self.ty(slot.size, false, &c);
                self.text(&format!("{p}-icon-text"), &slot, &ty, "ctr", "ctr");
            }

            if let Some(s) = &pg.time {
                let at = st.time.at.as_str();
                let (color, anchor) = if at == "in-marker" || at == "in-axis" {
                    let behind = if at == "in-marker" {
                        self.marker_fill(pg)
                    } else if st.axis.kind == "band" || st.axis.kind == "chevrons" {
                        pick(&st.axis.color, &pc, &pc)
                    } else {
                        None
                    };
                    let c = match behind {
                        Some(b) if dark(&b) => "FFFFFF".to_string(),
                        _ => ink.to_uppercase(),
                    };
                    (c, "ctr")
                } else {
                    let side = if at == "opposite" {
                        if pg.side == Side::Above { Side::Below } else { Side::Above }
                    } else {
                        pg.side
                    };
                    (pick_text(&st.time.color, &pc, &ink), anchor_of(side))
                };
                let ty = self.ty(s.size, st.time.bold, &color);
                self.text(&format!("{p}-time"), s, &ty, "ctr", anchor);
            }
            if let Some(s) = &pg.title {
                let c = if in_card { pick_text(&st.card.text, &pc, &ink) } else { pick_text(&st.text.title_color, &pc, &ink) };
                let ty = self.ty(s.size, true, &c);
                self.text(&format!("{p}-title"), s, &ty, text_align, text_anchor);
            }
            if let Some(s) = &pg.body {
                let c = if in_card { pick_text(&st.card.text, &pc, &ink) } else { pick_text(&st.text.body_color, &pc, &ink) };
                let ty = self.ty(s.size, false, &c);
                self.text(&format!("{p}-body"), s, &ty, text_align, text_anchor);
            }
        }
        if let Some(s) = &plan.title {
            let c = pick_text(&st.title.color, &ink, &ink);
            let ty = self.ty(s.size, false, &c);
            let align = if st.title.align == "left" { "l" } else { "ctr" };
            self.text("title", s, &ty, align, "ctr");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::is_flow_object;
    use crate::timeline::{AxisGeom, Point};

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, w, h)
    }
    fn slot(rect: Rect, text: &str, size: f64) -> Slot {
        Slot { rect, text: text.into(), size }
    }
    fn fonts() -> crate::theme::Font {
        crate::theme::Font { family: "Sans".into(), math: "Math".into(), ea: "EA".into(), title: String::new(), caption: String::new(), size: 10.0, title_scale: 1.0, small_scale: 1.0, title_size: 0.0, small_size: 0.0, title_bold: true }
    }
    fn spec() -> TimelineSpec {
        TimelineSpec {
            id: "tl".into(),
            title: "Five years".into(),
            points: vec![
                Point { time: "2021".into(), title: "Pilot".into(), body: "Two sites, one protocol.".into(), icon: "map".into(), ..Default::default() },
                Point { time: "2022".into(), title: "Launch".into(), body: "Three sites.".into(), ..Default::default() },
                Point { time: "2023".into(), title: "Scale".into(), body: "Ten sites.".into(), ..Default::default() },
            ],
            ..Default::default()
        }
    }
    /// Three points on y = 100, the first side above, the others alternating.
    fn plan() -> Plan {
        let cols = ["112233", "445566", "AABBCC"];
        let points = (0..3)
            .map(|i| {
                let x = 100.0 + 100.0 * i as f64;
                let above = i % 2 == 0;
                let (ty, my) = if above { (20.0, 60.0) } else { (130.0, 130.0) };
                let _ = my;
                PointGeom {
                    x,
                    side: if above { Side::Above } else { Side::Below },
                    color: cols[i].into(),
                    marker: r(x - 5.0, 95.0, 10.0, 10.0),
                    time: Some(slot(r(x - 40.0, if above { 130.0 } else { 60.0 }, 80.0, 20.0), ["2021", "2022", "2023"][i], 16.0)),
                    title: Some(slot(r(x - 40.0, ty, 80.0, 14.0), ["Pilot", "Launch", "Scale"][i], 11.0)),
                    body: Some(slot(r(x - 40.0, ty + 14.0, 80.0, 30.0), ["Two sites, one protocol.", "Three sites.", "Ten sites."][i], 9.0)),
                    card: None,
                    card_header: None,
                    stem: Some([(x, 95.0), (x, 70.0)]),
                    icon: None,
                    rule: None,
                }
            })
            .collect();
        Plan { bounds: r(0.0, 0.0, 400.0, 200.0), title: Some(slot(r(0.0, 0.0, 400.0, 20.0), "Five years", 16.0)), legend: vec![], axis: AxisGeom { x0: 20.0, x1: 380.0, y: 100.0, thickness: 1.0 }, points, scale: 1.0 }
    }
    fn names(b: &Built) -> Vec<String> {
        b.objects.iter().map(|o| o.name.rsplit('/').next().unwrap_or("").to_string()).collect()
    }
    fn find<'a>(b: &'a Built, part: &str) -> &'a Object {
        let n = node_name("tl", part);
        b.objects.iter().find(|o| o.name == n).unwrap_or_else(|| panic!("no {part} in {:?}", names(b)))
    }
    fn has(b: &Built, part: &str) -> bool {
        let n = node_name("tl", part);
        b.objects.iter().any(|o| o.name == n)
    }
    fn sp(o: &Object) -> &SpObj {
        match &o.kind {
            Kind::Shape(s) | Kind::Text(s) => s,
            _ => panic!("not a shape: {}", o.name),
        }
    }
    fn draw(st: &Style, pl: &Plan) -> Built {
        objects(&spec(), st, pl, &fonts(), 5)
    }
    fn fill_hex(o: &Object) -> String {
        match &sp(o).fill {
            Paint::Solid { color, .. } => color.clone(),
            other => panic!("not solid: {other:?}"),
        }
    }
    fn text_color(o: &Object) -> String {
        sp(o).text.as_ref().and_then(|t| t.paras.first()).and_then(|p| p.runs.first()).map(|r| r.look.color.clone()).unwrap_or_default()
    }
    fn emu_pt(v: i64) -> f64 {
        v as f64 / 12_700.0
    }

    // ── step 1 ──
    #[test]
    fn every_object_is_named_under_the_figure_and_found_again() {
        let b = draw(&Style::default(), &plan());
        assert!(!b.objects.is_empty());
        assert!(b.objects.iter().all(|o| is_flow_object(o, "tl")));
        assert!(b.objects.iter().all(|o| !is_flow_object(o, "other")));
        assert!(has(&b, "axis") && has(&b, "p1") && has(&b, "p3-body") && has(&b, "title"));
        assert_eq!(b.node_ids.get("p2"), Some(&find(&b, "p2").id));
    }

    #[test]
    fn ids_run_from_the_first_id() {
        let b = draw(&Style::default(), &plan());
        for (i, o) in b.objects.iter().enumerate() {
            assert_eq!(o.id, 5 + i as u32);
        }
        assert_eq!(b.next_id, 5 + b.objects.len() as u32);
        let mut empty = plan();
        empty.points.clear();
        empty.title = None;
        empty.axis.thickness = 0.0;
        let e = draw(&Style::default(), &empty);
        assert!(e.objects.is_empty());
        assert_eq!(e.next_id, 5);
        assert!(b.pictures.is_empty() && b.warnings.is_empty());
    }

    #[test]
    fn shapes_come_before_every_text() {
        let mut st = Style::default();
        st.icon.at = "in-marker".into();
        let mut pl = plan();
        pl.points[0].icon = Some(r(95.0, 95.0, 10.0, 10.0));
        let b = draw(&st, &pl);
        let first_text = b.objects.iter().position(|o| matches!(o.kind, Kind::Text(_))).unwrap();
        assert!(b.objects[first_text..].iter().all(|o| matches!(o.kind, Kind::Text(_))));
        assert!(b.objects[..first_text].iter().all(|o| !matches!(o.kind, Kind::Text(_))));
        assert_eq!(names(&b).last().map(String::as_str), Some("title"));
    }

    #[test]
    fn a_line_axis_is_one_segment_and_a_band_is_a_filled_box() {
        let mut st = Style::default();
        st.axis.dash = "dash".into();
        let b = draw(&st, &plan());
        assert_eq!(names(&b).iter().filter(|n| n.starts_with("axis")).count(), 1);
        let a = find(&b, "axis");
        let Kind::Connector(c) = &a.kind else { panic!("axis is not a connector") };
        let l = c.line.as_ref().unwrap();
        assert_eq!((l.width_pt, l.dash.as_str(), l.color.as_str()), (1.0, "dash", "9AA0A6"));
        assert!((emu_pt(a.frame.w) - 360.0).abs() < 0.01);

        st.axis.kind = "band".into();
        st.axis.color = "point".into();
        let mut pl = plan();
        pl.axis.thickness = 20.0;
        let b = draw(&st, &pl);
        let bands: Vec<&Object> = b.objects.iter().filter(|o| o.name.contains("/axis-")).collect();
        assert_eq!(bands.len(), 3);
        assert_eq!(fill_hex(bands[0]), "112233");
        assert!((emu_pt(bands[0].frame.x) - 20.0).abs() < 0.01);
        assert!((emu_pt(bands[0].frame.x + bands[0].frame.w) - 150.0).abs() < 0.01);
        assert!((emu_pt(bands[2].frame.x + bands[2].frame.w) - 380.0).abs() < 0.01);
        assert!((emu_pt(bands[1].frame.h) - 20.0).abs() < 0.01 && (emu_pt(bands[1].frame.y) - 90.0).abs() < 0.01);
        assert!(sp(bands[0]).line.is_none());

        st.axis.color = "ABCDEF".into();
        let b = draw(&st, &pl);
        assert_eq!(fill_hex(find(&b, "axis")), "ABCDEF");
    }

    #[test]
    fn a_band_in_the_points_one_colour_is_one_piece() {
        let mut st = Style::default();
        st.axis.kind = "band".into();
        st.axis.color = "point".into();
        let mut pl = plan();
        pl.axis.thickness = 40.0;
        for p in pl.points.iter_mut() {
            p.color = "2F6DB0".into();
        }
        let b = objects(&spec(), &st, &pl, &fonts(), 2);
        let bands: Vec<&str> = b.objects.iter().map(|o| o.name.as_str()).filter(|n| n.contains("/axis")).collect();
        assert_eq!(bands.len(), 1, "{bands:?}");
    }

    #[test]
    fn joins_run_between_neighbouring_markers() {
        let mut st = Style::default();
        st.axis.kind = "joins".into();
        let mut pl = plan();
        pl.axis.thickness = 0.0;
        pl.scale = 2.0;
        let b = draw(&st, &pl);
        assert!(has(&b, "join-1") && has(&b, "join-2") && !has(&b, "join-3") && !has(&b, "axis"));
        let j = find(&b, "join-1");
        let Kind::Connector(c) = &j.kind else { panic!() };
        let l = c.line.as_ref().unwrap();
        assert_eq!((l.dash.as_str(), l.color.as_str(), l.width_pt), ("sysDash", "5F6368", 1.5));
        assert!((emu_pt(j.frame.x) - 105.0).abs() < 0.01 && (emu_pt(j.frame.w) - 90.0).abs() < 0.01);
    }

    #[test]
    fn no_axis_is_drawn_when_it_has_no_thickness() {
        let mut pl = plan();
        pl.axis.thickness = 0.0;
        let b = draw(&Style::default(), &pl);
        assert!(!has(&b, "axis"));
        let mut st = Style::default();
        st.axis.kind = "band".into();
        assert!(!has(&draw(&st, &pl), "axis"));
    }

    // ── step 2 ──
    #[test]
    fn a_ring_has_its_rings_round_a_dot() {
        let mut st = Style::default();
        st.marker.kind = "ring".into();
        st.marker.rings = 2;
        st.marker.line_weight = 1.5;
        let mut pl = plan();
        pl.points[0].marker = r(90.0, 90.0, 20.0, 20.0);
        let b = draw(&st, &pl);
        let dot = find(&b, "p1");
        assert!((emu_pt(dot.frame.w) - 10.0).abs() < 0.01);
        let r1 = find(&b, "p1-ring-1");
        let r2 = find(&b, "p1-ring-2");
        assert!((emu_pt(r1.frame.w) - 15.0).abs() < 0.01 && (emu_pt(r2.frame.w) - 20.0).abs() < 0.01);
        assert!(!has(&b, "p1-ring-3"));
        assert!(matches!(sp(r1).fill, Paint::None));
        assert_eq!(sp(r1).line.as_ref().unwrap().color, "112233");
        assert_eq!(sp(r2).line.as_ref().unwrap().color, "5F6368");
        assert_eq!(sp(r2).line.as_ref().unwrap().width_pt, 1.5);
        // dot first, rings after
        let n = names(&b);
        assert!(n.iter().position(|x| x == "p1") < n.iter().position(|x| x == "p1-ring-1"));
    }

    #[test]
    fn a_square_marker_keeps_its_outline_and_radius() {
        let mut st = Style::default();
        st.marker.kind = "square".into();
        st.marker.line = "paper".into();
        st.marker.line_weight = 2.0;
        st.marker.radius = 2.0;
        let mut pl = plan();
        pl.scale = 2.0;
        let b = draw(&st, &pl);
        let o = find(&b, "p1");
        let s = sp(o);
        assert_eq!(s.geom.prst, "roundRect");
        assert_eq!(s.line.as_ref().unwrap().color, "FFFFFF");
        assert_eq!(s.line.as_ref().unwrap().width_pt, 4.0);
        // radius 2 x 2 = 4pt of a 10pt side
        assert_eq!(s.geom.adj.get("adj"), Some(&40_000.0));
        assert_eq!(fill_hex(o), "112233");
        st.marker.line = "none".into();
        assert!(sp(find(&draw(&st, &pl), "p1")).line.is_none());
        st.marker.kind = "circle".into();
        assert_eq!(sp(find(&draw(&st, &pl), "p1")).geom.prst, "ellipse");
    }

    #[test]
    fn a_stem_ends_in_a_dot_when_the_style_says_so() {
        let mut st = Style::default();
        st.stem.end = "dot".into();
        st.stem.weight = 1.0;
        let b = draw(&st, &plan());
        assert!(has(&b, "p1-stem"));
        let e = find(&b, "p1-stem-end");
        assert!((emu_pt(e.frame.w) - 4.0).abs() < 0.01);
        assert!((emu_pt(e.frame.x) + 2.0 - 100.0).abs() < 0.01 && (emu_pt(e.frame.y) + 2.0 - 70.0).abs() < 0.01);
        st.stem.end = "none".into();
        assert!(!has(&draw(&st, &plan()), "p1-stem-end"));
        st.stem.kind = "none".into();
        assert!(!has(&draw(&st, &plan()), "p1-stem"));
    }

    #[test]
    fn a_drop_is_a_triangle_towards_the_text() {
        let mut st = Style::default();
        st.stem.kind = "drop".into();
        let mut pl = plan();
        pl.points[0].marker = r(90.0, 90.0, 20.0, 20.0);
        pl.points[0].stem = Some([(100.0, 90.0), (100.0, 60.0)]);
        let b = draw(&st, &pl);
        let o = find(&b, "p1-stem");
        let s = sp(o);
        assert_eq!(s.geom.prst, "custom");
        assert!(s.geom.path.as_deref().unwrap_or("").ends_with('Z'));
        // base 40% of 20 = 8 wide at the marker edge, tip 30 above
        assert!((emu_pt(o.frame.w) - 8.0).abs() < 0.01 && (emu_pt(o.frame.h) - 30.0).abs() < 0.01);
        assert!((emu_pt(o.frame.x) - 96.0).abs() < 0.01 && (emu_pt(o.frame.y) - 60.0).abs() < 0.01);
        assert_eq!(fill_hex(o), "112233");
    }

    #[test]
    fn the_points_colour_reaches_marker_stem_and_rule() {
        let mut pl = plan();
        pl.points[1].rule = Some([(190.0, 150.0), (210.0, 150.0)]);
        let b = draw(&Style::default(), &pl);
        assert_eq!(fill_hex(find(&b, "p2")), "445566");
        for part in ["p2-stem", "p2-rule"] {
            let Kind::Connector(c) = &find(&b, part).kind else { panic!() };
            assert_eq!(c.line.as_ref().unwrap().color, "445566", "{part}");
        }
        let Kind::Connector(c) = &find(&b, "p2-rule").kind else { panic!() };
        assert_eq!(c.line.as_ref().unwrap().width_pt, 1.5);
        let Kind::Connector(c) = &find(&b, "p3-stem").kind else { panic!() };
        assert_eq!(c.line.as_ref().unwrap().color, "AABBCC");
    }

    // ── step 3 ──
    fn card_plan() -> Plan {
        let mut pl = plan();
        pl.points[0].card = Some(r(60.0, 120.0, 80.0, 60.0));
        pl.points[0].card_header = Some(r(60.0, 120.0, 80.0, 16.0));
        pl
    }

    #[test]
    fn a_card_has_a_darker_strip() {
        let mut st = Style::default();
        st.card.radius = 4.0;
        let b = draw(&st, &card_plan());
        let c = find(&b, "p1-card");
        assert_eq!(sp(c).geom.prst, "roundRect");
        assert_eq!(fill_hex(c), "112233");
        assert!(sp(c).line.is_none());
        assert_eq!(sp(c).geom.adj.get("adj"), Some(&6_667.0));
        assert_eq!(fill_hex(find(&b, "p1-card-head")), shade("112233", -0.25));
        let n = names(&b);
        let pos = |s: &str| n.iter().position(|x| x == s).unwrap();
        assert!(pos("p1-card") < pos("p1-card-head") && pos("p1-card-head") < pos("p1"));
        assert!(!has(&b, "p2-card"));
    }

    #[test]
    fn an_icon_slot_is_dashed_and_says_what_to_put_there() {
        let mut pl = plan();
        pl.points[0].icon = Some(r(60.0, 40.0, 28.0, 28.0));
        pl.scale = 2.0;
        let b = draw(&Style::default(), &pl);
        let o = find(&b, "p1-icon");
        let l = sp(o).line.as_ref().unwrap();
        assert_eq!((l.dash.as_str(), l.color.as_str(), l.width_pt), ("sysDash", "5F6368", 1.5));
        assert!(matches!(sp(o).fill, Paint::None));
        let t = find(&b, "p1-icon-text");
        assert!(matches!(t.kind, Kind::Text(_)));
        assert_eq!(sp(t).text.as_ref().unwrap().paras[0].runs[0].text, "map");
        assert_eq!(sp(t).text.as_ref().unwrap().paras[0].runs[0].look.size, 14.0);
        assert_eq!(sp(t).text.as_ref().unwrap().anchor, "ctr");
        assert_eq!(text_color(t), "5F6368");
        // in a dark marker the slot turns white
        let mut st = Style::default();
        st.icon.at = "in-marker".into();
        st.marker.fill = "112233".into();
        let b = draw(&st, &pl);
        assert_eq!(sp(find(&b, "p1-icon")).line.as_ref().unwrap().color, "FFFFFF");
        assert_eq!(text_color(find(&b, "p1-icon-text")), "FFFFFF");
        st.marker.fill = "EEEEEE".into();
        assert_eq!(sp(find(&draw(&st, &pl), "p1-icon")).line.as_ref().unwrap().color, "5F6368");
    }

    fn draw_icon(word: &str, st: &Style) -> Built {
        let mut sp = spec();
        sp.points[0].icon = word.into();
        let mut pl = plan();
        pl.points[0].icon = Some(r(60.0, 40.0, 28.0, 28.0));
        objects(&sp, st, &pl, &fonts(), 5)
    }

    fn icon_parts(b: &Built) -> Vec<&Object> {
        b.objects.iter().filter(|o| o.name.rsplit('/').next().unwrap_or("").starts_with("p1-icon")).collect()
    }

    #[test]
    fn an_icon_id_is_drawn_as_the_icon_not_a_slot() {
        let (id, n) = lore_icons::search("server", None, 50)
            .into_iter()
            .chain(lore_icons::search("doctor", None, 50))
            .filter_map(|h| lore_icons::icon(&h.id).map(|i| (h.id, i.shapes.len())))
            .find(|(_, n)| *n > 1)
            .expect("a multi-shape icon");
        let b = draw_icon(&id, &Style::default());
        assert_eq!(icon_parts(&b).len(), n);
        assert!(has(&b, "p1-icon") && has(&b, "p1-icon-2") && !has(&b, &format!("p1-icon-{}", n + 1)));
        for o in icon_parts(&b) {
            let s = sp(o);
            assert_eq!(s.geom.prst, "custom");
            assert!(s.line.as_ref().is_none_or(|l| l.dash != "sysDash"));
        }
        assert!(!has(&b, "p1-icon-text"));
        assert!(b.warnings.is_empty());
    }

    #[test]
    fn the_icon_takes_the_slots_colour_and_sits_inside_it() {
        let slot = frame(60.0, 40.0, 28.0, 28.0);
        let mut st = Style::default();
        st.icon.at = "in-marker".into();
        st.marker.fill = "112233".into();
        let b = draw_icon("tabler:server", &st);
        for o in icon_parts(&b) {
            assert!(o.frame.x >= slot.x && o.frame.y >= slot.y);
            assert!(o.frame.x + o.frame.w <= slot.x + slot.w && o.frame.y + o.frame.h <= slot.y + slot.h);
            let c = sp(o).line.as_ref().map(|l| l.color.clone()).unwrap_or_else(|| fill_hex(o));
            assert_eq!(c, "FFFFFF");
        }
        st.marker.fill = "EEEEEE".into();
        let b = draw_icon("tabler:server", &st);
        let c = icon_parts(&b).iter().map(|o| sp(o).line.as_ref().map(|l| l.color.clone()).unwrap_or_else(|| fill_hex(o))).collect::<Vec<_>>();
        assert!(c.iter().all(|c| c == "5F6368"));
    }

    #[test]
    fn plain_words_still_draw_the_dashed_slot() {
        let b = draw_icon("map", &Style::default());
        assert_eq!(sp(find(&b, "p1-icon")).line.as_ref().unwrap().dash, "sysDash");
        assert!(has(&b, "p1-icon-text") && !has(&b, "p1-icon-2") && b.warnings.is_empty());
    }

    #[test]
    fn an_id_the_sets_do_not_have_is_a_slot_and_a_warning() {
        let b = draw_icon("tabler:no-such-thing-xyz", &Style::default());
        assert_eq!(sp(find(&b, "p1-icon")).line.as_ref().unwrap().dash, "sysDash");
        let t = find(&b, "p1-icon-text");
        assert_eq!(sp(t).text.as_ref().unwrap().paras[0].runs[0].text, "no-such-thing-xyz");
        assert_eq!(b.warnings, vec!["找不到圖示 tabler:no-such-thing-xyz".to_string()]);
    }

    #[test]
    fn an_empty_icon_slot_has_no_text() {
        let mut pl = plan();
        pl.points[1].icon = Some(r(160.0, 40.0, 28.0, 28.0));
        let b = draw(&Style::default(), &pl);
        assert!(has(&b, "p2-icon"));
        assert!(!has(&b, "p2-icon-text"));
    }

    #[test]
    fn gradient_fills_go_from_a_tint_to_the_colour() {
        let mut st = Style::default();
        st.colors.gradient = true;
        st.marker.kind = "square".into();
        let b = draw(&st, &card_plan());
        for part in ["p1", "p1-card"] {
            let Paint::Gradient { stops, angle, .. } = &sp(find(&b, part)).fill else { panic!("{part} is not a gradient") };
            assert_eq!(*angle, 0.0);
            assert_eq!(stops.len(), 2);
            assert_eq!((stops[0].pos, stops[0].color.as_str()), (0.0, shade("112233", 0.35).as_str()));
            assert_eq!((stops[1].pos, stops[1].color.as_str()), (1.0, "112233"));
        }
        // the strip and the axis stay solid
        assert!(matches!(sp(find(&b, "p1-card-head")).fill, Paint::Solid { .. }));
        st.colors.gradient = false;
        assert!(matches!(sp(find(&draw(&st, &card_plan()), "p1")).fill, Paint::Solid { .. }));
    }

    // ── step 4 ──
    #[test]
    fn texts_sit_exactly_in_their_slots() {
        let b = draw(&Style::default(), &plan());
        let pl = plan();
        for (part, s) in [("p1-time", &pl.points[0].time), ("p2-title", &pl.points[1].title), ("p3-body", &pl.points[2].body)] {
            let o = find(&b, part);
            let s = s.as_ref().unwrap();
            assert!((emu_pt(o.frame.x) - s.rect.x).abs() < 0.01 && (emu_pt(o.frame.y) - s.rect.y).abs() < 0.01);
            assert!((emu_pt(o.frame.w) - s.rect.w).abs() < 0.01 && (emu_pt(o.frame.h) - s.rect.h).abs() < 0.01);
            let tb = sp(o).text.as_ref().unwrap();
            assert_eq!(tb.paras[0].runs[0].text, s.text.lines().next().unwrap());
            assert_eq!(tb.paras[0].runs[0].look.size, s.size);
            assert_eq!(tb.paras[0].runs[0].look.font, "Sans");
        }
        let t = find(&b, "title");
        assert_eq!(text_color(t), "202124");
        assert!(!sp(t).text.as_ref().unwrap().paras[0].runs[0].look.bold);
        assert!(sp(find(&b, "p1-title")).text.as_ref().unwrap().paras[0].runs[0].look.bold);
        assert!(!sp(find(&b, "p1-body")).text.as_ref().unwrap().paras[0].runs[0].look.bold);
        // two lines make two paragraphs
        let mut pl = plan();
        pl.points[0].body = Some(slot(r(60.0, 34.0, 80.0, 30.0), "Two sites,\none protocol.", 9.0));
        assert_eq!(sp(find(&draw(&Style::default(), &pl), "p1-body")).text.as_ref().unwrap().paras.len(), 2);
        // alignment
        let mut st = Style::default();
        st.text.align = "left".into();
        let b = draw(&st, &plan());
        assert_eq!(sp(find(&b, "p1-title")).text.as_ref().unwrap().paras[0].align, "l");
        assert_eq!(sp(find(&b, "p1-time")).text.as_ref().unwrap().paras[0].align, "ctr");
    }

    #[test]
    fn text_in_a_card_takes_the_cards_text_colour() {
        let mut st = Style::default();
        st.card.text = "FEFEFE".into();
        st.text.align = "left".into();
        let b = draw(&st, &card_plan());
        assert_eq!(text_color(find(&b, "p1-title")), "FEFEFE");
        assert_eq!(text_color(find(&b, "p1-body")), "FEFEFE");
        assert_eq!(sp(find(&b, "p1-body")).text.as_ref().unwrap().paras[0].align, "ctr");
        // outside a card they follow the text colours
        assert_eq!(text_color(find(&b, "p2-title")), "445566");
        assert_eq!(text_color(find(&b, "p2-body")), "5F6368");
        assert_eq!(sp(find(&b, "p2-body")).text.as_ref().unwrap().paras[0].align, "l");
    }

    #[test]
    fn a_time_inside_a_dark_marker_is_white() {
        let mut st = Style::default();
        st.time.at = "in-marker".into();
        st.marker.kind = "block".into();
        let b = draw(&st, &plan());
        let t = find(&b, "p1-time");
        assert_eq!(text_color(t), "FFFFFF");
        assert_eq!(sp(t).text.as_ref().unwrap().anchor, "ctr");
        assert_eq!(text_color(find(&b, "p3-time")), "202124"); // AABBCC is light
        st.marker.fill = "paper".into();
        assert_eq!(text_color(find(&draw(&st, &plan()), "p1-time")), "202124");
        // in a band
        st.time.at = "in-axis".into();
        st.axis.kind = "band".into();
        st.axis.color = "point".into();
        let mut pl = plan();
        pl.axis.thickness = 20.0;
        let b = draw(&st, &pl);
        assert_eq!(text_color(find(&b, "p1-time")), "FFFFFF");
        assert_eq!(text_color(find(&b, "p3-time")), "202124");
        // beside the axis it is the time colour
        st.time.at = "opposite".into();
        st.time.color = "point".into();
        assert_eq!(text_color(find(&draw(&st, &pl), "p1-time")), "112233");
    }

    #[test]
    fn texts_above_the_axis_are_anchored_to_the_bottom() {
        let b = draw(&Style::default(), &plan());
        let anchor = |part: &str| sp(find(&b, part)).text.as_ref().unwrap().anchor.clone();
        assert_eq!(anchor("p1-title"), "b");
        assert_eq!(anchor("p1-body"), "b");
        assert_eq!(anchor("p2-title"), "t");
        assert_eq!(anchor("p2-body"), "t");
        // the time is across the axis from the text
        assert_eq!(anchor("p1-time"), "t");
        assert_eq!(anchor("p2-time"), "b");
        let mut st = Style::default();
        st.time.at = "with-text".into();
        let b = draw(&st, &plan());
        assert_eq!(sp(find(&b, "p1-time")).text.as_ref().unwrap().anchor, "b");
    }

    // ── step 5 ──
    fn all_styles() -> Vec<Style> {
        let mut v = vec![Style::default()];
        let mut s = Style::default();
        s.marker.kind = "ring".into();
        s.colors.gradient = true;
        s.stem.end = "dot".into();
        v.push(s);
        let mut s = Style::default();
        s.text.kind = "card".into();
        s.marker.kind = "square".into();
        s.marker.line = "paper".into();
        s.stem.kind = "drop".into();
        s.icon.at = "in-marker".into();
        v.push(s);
        let mut s = Style::default();
        s.axis.kind = "band".into();
        s.axis.color = "point".into();
        s.time.at = "in-axis".into();
        s.marker.kind = "block".into();
        v.push(s);
        v
    }
    fn busy_plan() -> Plan {
        let mut pl = card_plan();
        pl.axis.thickness = 14.0;
        pl.legend = vec![(150.0, 12.0), (160.0, 12.0), (170.0, 12.0)];
        pl.points[0].icon = Some(r(90.0, 60.0, 20.0, 20.0));
        pl.points[1].rule = Some([(190.0, 150.0), (210.0, 150.0)]);
        pl
    }

    #[test]
    fn the_preview_svg_shows_every_text_and_no_nan() {
        for st in all_styles() {
            let pl = busy_plan();
            let b = draw(&st, &pl);
            let svg = crate::preview::svg(&b.objects, 400.0, 200.0, 1.0);
            assert!(!svg.contains("NaN") && !svg.contains("inf"), "{}", st.marker.kind);
            for w in ["Pilot", "2021", "Launch", "Scale", "Ten sites.", "Five years", "map", "Two sites, one protocol."] {
                assert!(svg.contains(w), "{w} missing");
            }
        }
    }

    #[test]
    fn colours_come_only_from_the_style_and_the_plan() {
        for st in all_styles() {
            let pl = busy_plan();
            let b = draw(&st, &pl);
            let mut base: Vec<String> = vec!["FFFFFF".into()];
            base.extend([st.colors.ink.clone(), st.colors.muted.clone(), st.axis.color.clone(), st.card.text.clone(), st.text.body_color.clone(), st.title.color.clone()]);
            base.extend(st.colors.palette.iter().cloned());
            base.extend(pl.points.iter().map(|p| p.color.clone()));
            let mut ok = base.clone();
            for c in &base {
                ok.push(shade(c, 0.35));
                ok.push(shade(c, -0.25));
            }
            let mut used: Vec<String> = vec![];
            for o in &b.objects {
                match &o.kind {
                    Kind::Shape(s) | Kind::Text(s) => {
                        match &s.fill {
                            Paint::Solid { color, .. } => used.push(color.clone()),
                            Paint::Gradient { stops, .. } => used.extend(stops.iter().map(|x| x.color.clone())),
                            _ => {}
                        }
                        if let Some(l) = &s.line {
                            used.push(l.color.clone());
                        }
                        if let Some(t) = &s.text {
                            for p in &t.paras {
                                used.extend(p.runs.iter().map(|r| r.look.color.clone()));
                            }
                        }
                    }
                    Kind::Connector(c) => {
                        if let Some(l) = &c.line {
                            used.push(l.color.clone());
                        }
                    }
                    _ => {}
                }
            }
            assert!(used.len() > 10);
            for c in used {
                assert!(ok.iter().any(|k| k.eq_ignore_ascii_case(&c)), "stray colour {c}");
            }
        }
    }

    // ── package T-axes ──
    /// (left, top, right, bottom) of an object in pt.
    fn bbox(o: &Object) -> (f64, f64, f64, f64) {
        (emu_pt(o.frame.x), emu_pt(o.frame.y), emu_pt(o.frame.x + o.frame.w), emu_pt(o.frame.y + o.frame.h))
    }
    fn vertices(o: &Object) -> usize {
        sp(o).geom.path.as_deref().unwrap_or("").matches(['M', 'L']).count()
    }
    fn chevron_style(color: &str) -> Style {
        let mut st = Style::default();
        st.axis.kind = "chevrons".into();
        st.axis.color = color.into();
        st
    }
    fn chevron_plan() -> Plan {
        let mut pl = plan();
        pl.axis.thickness = 20.0;
        pl
    }

    #[test]
    fn chevrons_are_one_arrow_piece_per_point() {
        let b = draw(&chevron_style("point"), &chevron_plan());
        assert!(!has(&b, "axis"));
        for i in 1..=3 {
            let o = find(&b, &format!("axis-{i}"));
            assert_eq!(sp(o).geom.prst, "custom");
            assert!(sp(o).geom.path.as_deref().unwrap().ends_with('Z'));
            assert!(sp(o).line.is_none());
            let (_, t, _, bt) = bbox(o);
            assert!((t - 90.0).abs() < 0.01 && (bt - 110.0).abs() < 0.01);
            // the first piece has a straight left edge, the others a notch
            assert_eq!(vertices(o), if i == 1 { 5 } else { 6 });
        }
        // spans: the first starts at the axis, the point of an arrow reaches 0.35 x 20 = 7 past its body
        let (l, _, r, _) = bbox(find(&b, "axis-1"));
        assert!((l - 20.0).abs() < 0.01 && (r - (150.0 - 2.0 + 7.0)).abs() < 0.01, "{l} {r}");
    }

    #[test]
    fn chevron_pieces_do_not_overlap_and_end_at_the_axis_end() {
        let b = draw(&chevron_style("point"), &chevron_plan());
        for i in 1..3 {
            let (_, _, tip, _) = bbox(find(&b, &format!("axis-{i}")));
            let (notch, _, _, _) = bbox(find(&b, &format!("axis-{}", i + 1)));
            // the next piece's notch is 7 in from its left edge; the gap between is 2
            assert!((notch + 7.0 - tip - 2.0).abs() < 0.01, "{i}: tip {tip} notch {notch}");
        }
        let (_, _, r, _) = bbox(find(&b, "axis-3"));
        assert!((r - 380.0).abs() < 0.01);
        // the gap scales with the figure
        let mut pl = chevron_plan();
        pl.scale = 2.0;
        pl.axis.thickness = 20.0;
        let b = draw(&chevron_style("point"), &pl);
        let (_, _, tip, _) = bbox(find(&b, "axis-1"));
        let (notch, _, _, _) = bbox(find(&b, "axis-2"));
        assert!((notch + 7.0 - tip - 4.0).abs() < 0.01);
    }

    #[test]
    fn chevrons_take_the_points_colour_or_alternate_a_tint() {
        let b = draw(&chevron_style("point"), &chevron_plan());
        assert_eq!(fill_hex(find(&b, "axis-1")), "112233");
        assert_eq!(fill_hex(find(&b, "axis-2")), "445566");
        assert_eq!(fill_hex(find(&b, "axis-3")), "AABBCC");
        let b = draw(&chevron_style("ABCDEF"), &chevron_plan());
        assert_eq!(fill_hex(find(&b, "axis-1")), "ABCDEF");
        assert_eq!(fill_hex(find(&b, "axis-2")), shade("ABCDEF", 0.3));
        assert_eq!(fill_hex(find(&b, "axis-3")), "ABCDEF");
    }

    fn path_style(color: &str) -> Style {
        let mut st = Style::default();
        st.axis.kind = "path".into();
        st.axis.color = color.into();
        st
    }
    /// Markers at heights 90, 110, 90 (centres), the line 6 pt thick.
    fn path_plan() -> Plan {
        let mut pl = plan();
        pl.axis.thickness = 6.0;
        for (p, cy) in pl.points.iter_mut().zip([90.0, 110.0, 90.0]) {
            p.marker = r(p.x - 5.0, cy - 5.0, 10.0, 10.0);
        }
        pl
    }

    #[test]
    fn a_path_runs_through_every_marker_centre() {
        let b = draw(&path_style("point"), &path_plan());
        assert!(!has(&b, "axis"));
        let (a1, a2, a3) = (bbox(find(&b, "axis-1")), bbox(find(&b, "axis-2")), bbox(find(&b, "axis-3")));
        // each piece runs from the midpoint before to the midpoint after (the ends: the axis)
        assert!((a1.0 - 20.0).abs() < 0.01 && (a1.2 - 150.0).abs() < 0.01);
        assert!((a2.0 - 150.0).abs() < 0.01 && (a2.2 - 250.0).abs() < 0.01);
        assert!((a3.0 - 250.0).abs() < 0.01 && (a3.2 - 380.0).abs() < 0.01);
        // level at the marker's centre height
        assert!((a1.1 - 90.0).abs() < 0.01);
        assert!((a3.1 - 90.0).abs() < 0.01 && (a3.3 - 90.0).abs() < 0.01);
        assert!((a2.3 - 110.0).abs() < 0.01);
        assert!(sp(find(&b, "axis-1")).fill == Paint::None);
    }

    #[test]
    fn the_step_between_two_heights_is_drawn_once() {
        let b = draw(&path_style("point"), &path_plan());
        // 90 -> 110 is drawn by piece 1 (level, then down); 110 -> 90 by piece 2; piece 3 is level
        assert_eq!(vertices(find(&b, "axis-1")), 3);
        assert_eq!(vertices(find(&b, "axis-2")), 3);
        assert_eq!(vertices(find(&b, "axis-3")), 2);
        assert!(!sp(find(&b, "axis-1")).geom.path.as_deref().unwrap().ends_with('Z'));
        // the step of piece 1 is at the midpoint 150: its box spans 90 .. 110
        let a1 = bbox(find(&b, "axis-1"));
        assert!((a1.1 - 90.0).abs() < 0.01 && (a1.3 - 110.0).abs() < 0.01);
        // a straight path is one level piece each
        let mut pl = path_plan();
        for p in pl.points.iter_mut() {
            p.marker = r(p.x - 5.0, 95.0, 10.0, 10.0);
        }
        let b = draw(&path_style("point"), &pl);
        assert!((1..=3).all(|i| vertices(find(&b, &format!("axis-{i}"))) == 2));
    }

    #[test]
    fn a_path_takes_each_points_colour() {
        let b = draw(&path_style("point"), &path_plan());
        for (i, c) in ["112233", "445566", "AABBCC"].iter().enumerate() {
            let l = sp(find(&b, &format!("axis-{}", i + 1))).line.as_ref().unwrap();
            assert_eq!((l.color.as_str(), l.width_pt), (*c, 6.0));
        }
        let b = draw(&path_style("ABCDEF"), &path_plan());
        assert_eq!(sp(find(&b, "axis-2")).line.as_ref().unwrap().color, "ABCDEF");
    }

    fn ring_style(rings: u32) -> Style {
        let mut st = Style::default();
        st.marker.kind = "ring".into();
        st.marker.rings = rings;
        st
    }
    fn ring_plan() -> Plan {
        // point 1 has its text above, point 2 below
        let mut pl = plan();
        pl.points[0].marker = r(90.0, 90.0, 20.0, 20.0);
        pl.points[1].marker = r(190.0, 90.0, 20.0, 20.0);
        pl
    }

    #[test]
    fn a_third_ring_is_a_half_ring_on_the_side_away_from_the_text() {
        let b = draw(&ring_style(3), &ring_plan());
        for i in [1, 2] {
            assert_eq!(sp(find(&b, &format!("p{i}-ring-1"))).geom.prst, "ellipse");
            assert_eq!(sp(find(&b, &format!("p{i}-ring-2"))).geom.prst, "ellipse");
        }
        // text above: the arc is the bottom half, 100 .. 110
        let up = find(&b, "p1-ring-3");
        assert_eq!(sp(up).geom.prst, "custom");
        let (l, t, rr, bt) = bbox(up);
        assert!((t - 100.0).abs() < 0.01 && (bt - 110.0).abs() < 0.01 && (l - 90.0).abs() < 0.01 && (rr - 110.0).abs() < 0.01, "{l} {t} {rr} {bt}");
        assert!(vertices(up) >= 24 && !sp(up).geom.path.as_deref().unwrap().ends_with('Z'));
        assert!(matches!(sp(up).fill, Paint::None));
        assert_eq!(sp(up).line.as_ref().unwrap().color, "5F6368");
        // text below: the top half, 90 .. 100
        let (_, t, _, bt) = bbox(find(&b, "p2-ring-3"));
        assert!((t - 90.0).abs() < 0.01 && (bt - 100.0).abs() < 0.01);
        assert!(!has(&b, "p1-ring-4"));
    }

    #[test]
    fn two_rings_stay_whole() {
        for rings in [1, 2] {
            let b = draw(&ring_style(rings), &ring_plan());
            for r in 1..=rings {
                assert_eq!(sp(find(&b, &format!("p1-ring-{r}"))).geom.prst, "ellipse");
            }
        }
    }

    #[test]
    fn the_preview_svg_of_chevrons_and_a_path_has_no_nan() {
        use crate::timeline::place;
        let f = r(0.0, 0.0, 672.0, 357.0);
        let mut sp3 = spec();
        sp3.points.push(Point { time: "2024".into(), title: "Grow".into(), body: "Four sites.".into(), ..Default::default() });
        for (kind, color) in [("chevrons", "point"), ("chevrons", "2F6DB0"), ("path", "point"), ("path", "2F6DB0")] {
            for side in ["alternate", "below"] {
                let mut st = Style::default();
                st.axis.kind = kind.into();
                st.axis.color = color.into();
                st.axis.weight = if kind == "chevrons" { 26.0 } else { 4.0 };
                st.text.side = side.into();
                st.marker.kind = "ring".into();
                st.marker.rings = 3;
                if kind == "chevrons" {
                    st.time.at = "in-axis".into();
                }
                let pl = place::plan(&sp3, &st, f);
                let b = objects(&sp3, &st, &pl, &fonts(), 1);
                let svg = crate::preview::svg(&b.objects, 672.0, 357.0, 1.0);
                assert!(!svg.contains("NaN") && !svg.contains("inf"), "{kind} {color} {side}");
                assert!(svg.contains("Pilot") && svg.contains("Grow"));
                assert!(has(&b, "axis-4"));
            }
        }
    }
}
