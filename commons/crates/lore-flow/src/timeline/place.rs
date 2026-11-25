//! Where everything goes (package T-place): spec + style + frame → [`Plan`]. See `timeline/mod.rs`.
//!
//! Everything of a point is first laid out relative to the axis (axis at y = 0, text side
//! `d = +1` below / `-1` above), then the axis's own y is chosen from what the points need and
//! every point is moved there. If the result leaves the region vertically the whole thing is
//! done again with a smaller shrink factor `k`.

use super::{can, AxisGeom, Line, Plan, PointGeom, Side, Slot, Style, TimelineSpec};
use crate::layout::Rect;
use crate::text;

const EPS: f64 = 1e-6;

/// Rule places this file knows: `can::RULE` plus `far` (beyond the block, away from the axis).
const RULES: [&str; 4] = ["none", "under", "over", "far"];

/// Place `spec` in `style` inside `frame` (pt). Pure geometry: no objects, no colours but the
/// resolved colour of each point. Values the program cannot draw ([`Style::cannot`]) fall back
/// to the field's default, so a plan always comes back.
pub fn plan(spec: &TimelineSpec, style: &Style, frame: Rect) -> Plan {
    let st = norm(style);
    let u = (frame.w / 672.0).min(frame.h / 357.0);
    let u = if u.is_finite() { u.max(0.01) } else { 1.0 };
    if spec.points.is_empty() {
        return Plan { bounds: frame, title: None, legend: vec![], axis: AxisGeom { x0: frame.x, x1: frame.right(), y: frame.cy(), thickness: 0.0 }, points: vec![], scale: u };
    }
    let mut out = build(spec, &st, frame, u, 1.0);
    for step in 1..=5 {
        if out.1 {
            break;
        }
        out = build(spec, &st, frame, u, 1.0 - 0.1 * step as f64);
    }
    out.0
}

// ── the style with unknown values replaced by the defaults ───────────────────

fn pick(v: &str, list: &[&str], d: &str) -> String {
    if list.contains(&v) { v.to_string() } else { d.to_string() }
}

fn pos(v: f64, d: f64) -> f64 {
    if v.is_finite() && v >= 0.0 { v } else { d }
}

fn norm(s: &Style) -> Style {
    let d = Style::default();
    let mut o = s.clone();
    o.axis.kind = pick(&s.axis.kind, &can::AXIS, &d.axis.kind);
    o.axis.at = pick(&s.axis.at, &can::AXIS_AT, &d.axis.at);
    o.axis.weight = pos(s.axis.weight, d.axis.weight);
    o.marker.kind = pick(&s.marker.kind, &can::MARKER, &d.marker.kind);
    o.marker.inside = pick(&s.marker.inside, &can::INSIDE, &d.marker.inside);
    o.marker.size = pos(s.marker.size, d.marker.size);
    o.time.at = pick(&s.time.at, &can::TIME_AT, &d.time.at);
    o.time.size = pos(s.time.size, d.time.size);
    o.text.kind = pick(&s.text.kind, &can::TEXT, &d.text.kind);
    o.text.side = pick(&s.text.side, &can::SIDE, &d.text.side);
    o.text.first = pick(&s.text.first, &["above", "below"], &d.text.first);
    o.text.align = pick(&s.text.align, &can::ALIGN, &d.text.align);
    o.text.rule = pick(&s.text.rule, &RULES, &d.text.rule);
    o.text.title_size = pos(s.text.title_size, d.text.title_size);
    o.text.body_size = pos(s.text.body_size, d.text.body_size);
    o.text.width = pos(s.text.width, 0.0);
    o.card.pad = pos(s.card.pad, d.card.pad);
    o.stem.kind = pick(&s.stem.kind, &can::STEM, &d.stem.kind);
    o.stem.length = pos(s.stem.length, d.stem.length);
    o.icon.at = pick(&s.icon.at, &can::ICON_AT, &d.icon.at);
    o.icon.size = pos(s.icon.size, d.icon.size);
    o.colors.mode = pick(&s.colors.mode, &can::COLORS, &d.colors.mode);
    o.title.size = pos(s.title.size, d.title.size);
    o.title.align = pick(&s.title.align, &can::ALIGN, &d.title.align);
    o
}

// ── text ─────────────────────────────────────────────────────────────────────

fn ellipsize(line: &str, width: f64, size: f64) -> String {
    let mut s = line.trim_end().to_string();
    while !s.is_empty() && text::width(&format!("{s}…"), size) > width {
        s.pop();
        s = s.trim_end().to_string();
    }
    s.push('…');
    s
}

/// Wrap on spaces to `width` (a word wider than a line is broken by characters, so text without
/// spaces wraps too); `\n` starts a new line; more than `max` lines ends in `…`.
fn wrap(raw: &str, size: f64, width: f64, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    for para in raw.split('\n') {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if text::width(&cand, size) <= width {
                cur = cand;
                continue;
            }
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            if text::width(word, size) <= width {
                cur = word.to_string();
            } else {
                for ch in word.chars() {
                    let t = format!("{cur}{ch}");
                    if cur.is_empty() || text::width(&t, size) <= width {
                        cur = t;
                    } else {
                        lines.push(std::mem::replace(&mut cur, ch.to_string()));
                    }
                }
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    if lines.len() > max {
        lines.truncate(max);
        if let Some(l) = lines.last_mut() {
            *l = ellipsize(l, width, size);
        }
    }
    lines
}

fn color_of(own: &str, st: &Style, i: usize) -> String {
    let c = own.trim().trim_start_matches('#');
    if c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return c.to_uppercase();
    }
    let p = &st.colors.palette;
    if p.is_empty() {
        return "5F6368".into();
    }
    let idx = if st.colors.mode == "single" { 0 } else { i % p.len() };
    p[idx].clone()
}

// ── the figure ───────────────────────────────────────────────────────────────

struct Env<'a> {
    st: &'a Style,
    s: f64,
    gap: f64,
    region: Rect,
    col: f64,
    thick: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum K {
    Icon,
    Time,
    Title,
    Body,
}

fn build(spec: &TimelineSpec, st: &Style, frame: Rect, u: f64, k: f64) -> (Plan, bool) {
    let s = u * k;
    let pad = 12.0 * s;
    let gap = 4.0 * s;
    let n = spec.points.len();

    // title and legend dots
    let mut top = frame.y;
    let mut title = None;
    let mut legend = vec![];
    if !spec.title.trim().is_empty() {
        let size = st.title.size * s;
        let h = 1.3 * size;
        let t = if st.title.caps { spec.title.to_uppercase() } else { spec.title.clone() };
        title = Some(Slot { rect: Rect::new(frame.x + pad, frame.y, (frame.w - 2.0 * pad).max(0.0), h), text: t, size });
        top += h;
        if st.title.legend_dots {
            let step = 10.0 * s;
            let total = step * (n as f64 - 1.0);
            let x0 = if st.title.align == "center" { frame.cx() - total / 2.0 } else { frame.x + pad };
            for i in 0..n {
                legend.push((x0 + step * i as f64, top + 5.0 * s));
            }
            top += step;
        }
    }
    let region = Rect::new(frame.x + pad, top + pad, (frame.w - 2.0 * pad).max(0.0), (frame.bottom() - pad - (top + pad)).max(0.0));
    let col = region.w / n as f64;
    let thick = if st.axis.kind == "line" || st.axis.kind == "band" { st.axis.weight * s } else { 0.0 };
    let env = Env { st, s, gap, region, col, thick };

    let mut pts: Vec<PointGeom> = spec.points.iter().enumerate().map(|(i, p)| point(&env, p, i, region.x + col * (i as f64 + 0.5))).collect();

    // what the points need on each side of the axis
    let (mut na, mut nb) = (thick / 2.0, thick / 2.0);
    for p in &pts {
        let (lo, hi) = extent_y(p);
        na = na.max(-lo);
        nb = nb.max(hi);
    }
    let alt = st.text.side == "alternate";
    let y = match st.axis.at.as_str() {
        "top" => region.y + na,
        "bottom" => region.bottom() - nb,
        _ if alt => region.cy(),
        _ => region.y + (region.h - (na + nb)) / 2.0 + na,
    };
    let fits = y - na >= region.y - EPS && y + nb <= region.bottom() + EPS;
    for p in pts.iter_mut() {
        shift(p, y);
    }
    let (x0, x1) = if st.axis.bleed { (frame.x, frame.right()) } else { (region.x + gap, region.right() - gap) };
    let axis = AxisGeom { x0, x1, y, thickness: thick };

    // bounds: every part
    let mut bb = Bb::default();
    if let Some(t) = &title {
        bb.rect(t.rect);
    }
    for &(cx, cy) in &legend {
        bb.rect(Rect::new(cx - 5.0 * s, cy - 5.0 * s, 10.0 * s, 10.0 * s));
    }
    bb.rect(Rect::new(x0, y - thick / 2.0, x1 - x0, thick));
    for p in &pts {
        for r in rects(p) {
            bb.rect(r);
        }
        for l in [p.stem, p.rule].into_iter().flatten() {
            bb.pt(l[0]);
            bb.pt(l[1]);
        }
    }
    let bounds = bb.get().unwrap_or(frame);
    (Plan { bounds, title, legend, axis, points: pts, scale: s }, fits)
}

#[derive(Default)]
struct Bb(Option<(f64, f64, f64, f64)>);
impl Bb {
    fn pt(&mut self, p: (f64, f64)) {
        self.0 = Some(match self.0 {
            None => (p.0, p.1, p.0, p.1),
            Some((a, b, c, d)) => (a.min(p.0), b.min(p.1), c.max(p.0), d.max(p.1)),
        });
    }
    fn rect(&mut self, r: Rect) {
        self.pt((r.x, r.y));
        self.pt((r.right(), r.bottom()));
    }
    fn get(&self) -> Option<Rect> {
        self.0.map(|(a, b, c, d)| Rect::new(a, b, c - a, d - b))
    }
}

fn rects(p: &PointGeom) -> Vec<Rect> {
    let mut v = vec![p.marker];
    for s in [&p.time, &p.title, &p.body].into_iter().flatten() {
        v.push(s.rect);
    }
    for r in [p.card, p.card_header, p.icon].into_iter().flatten() {
        v.push(r);
    }
    v
}

fn extent_y(p: &PointGeom) -> (f64, f64) {
    let mut bb = Bb::default();
    for r in rects(p) {
        bb.rect(r);
    }
    for l in [p.stem, p.rule].into_iter().flatten() {
        bb.pt(l[0]);
        bb.pt(l[1]);
    }
    bb.get().map(|r| (r.y, r.bottom())).unwrap_or((0.0, 0.0))
}

fn shift(p: &mut PointGeom, dy: f64) {
    p.marker.y += dy;
    for s in [&mut p.time, &mut p.title, &mut p.body].into_iter().flatten() {
        s.rect.y += dy;
    }
    for r in [&mut p.card, &mut p.card_header, &mut p.icon].into_iter().flatten() {
        r.y += dy;
    }
    for l in [&mut p.stem, &mut p.rule].into_iter().flatten() {
        l[0].1 += dy;
        l[1].1 += dy;
    }
}

/// Shift a span `left .. left + w` so it lies in `rl .. rr` (the left edge wins when too wide).
fn clamp_dx(left: f64, w: f64, rl: f64, rr: f64) -> f64 {
    let mut dx = 0.0;
    if left + w > rr {
        dx = rr - (left + w);
    }
    if left + dx < rl {
        dx = rl - left;
    }
    dx
}

/// One point laid out with the axis at y = 0.
fn point(e: &Env, p: &super::Point, i: usize, x: f64) -> PointGeom {
    let st = e.st;
    let (s, gap, col) = (e.s, e.gap, e.col);
    let (rl, rr) = (e.region.x, e.region.right());
    let alt = st.text.side == "alternate";
    let side = match st.text.side.as_str() {
        "above" => Side::Above,
        "below" => Side::Below,
        _ => {
            if i.is_multiple_of(2) == (st.text.first == "above") { Side::Above } else { Side::Below }
        }
    };
    let d = if side == Side::Below { 1.0 } else { -1.0 };
    let color = color_of(&p.color, st, i);
    let in_axis = st.time.at == "in-axis";

    // marker
    let ts = st.time.size * s;
    let time_txt = p.time.trim();
    let mh = st.marker.size * s;
    let mw = if st.marker.kind == "block" { mh.max(text::width(time_txt, ts) + 12.0 * s) } else { mh };
    let mc = if in_axis { -d * e.thick / 2.0 } else { 0.0 };
    let marker = Rect::new(x - mw / 2.0, mc - mh / 2.0, mw, mh);
    // where the stem (or the text) leaves: the marker's edge, or with times in the band the
    // band's edge on the text side
    let edge_t = if in_axis { d * e.thick / 2.0 } else { mc + d * mh / 2.0 };
    let edge_o = mc - d * mh / 2.0;

    // stem
    let stem_len = match st.stem.kind.as_str() {
        "line" => st.stem.length * s,
        "drop" => 0.4 * mh,
        _ => 0.0,
    };
    let stem: Option<Line> = if st.stem.kind == "none" { None } else { Some([(x, edge_t), (x, edge_t + d * stem_len)]) };
    let start = edge_t + d * (stem_len + gap);

    // the other side of the axis: icon, then the time label beyond it
    let si = st.icon.size * s;
    let other_w = (if alt { 2.0 * col - 3.0 * gap } else { col - 2.0 * gap }).min(2.0 * (x - rl).min(rr - x)).max(1.0);
    let mut icon: Option<Rect> = None;
    let mut off = gap;
    match st.icon.at.as_str() {
        "in-marker" => icon = Some(Rect::new(marker.x + 0.2 * marker.w, marker.y + 0.2 * marker.h, 0.6 * marker.w, 0.6 * marker.h)),
        "opposite" => {
            let iy = if d > 0.0 { edge_o - gap - si } else { edge_o + gap };
            icon = Some(Rect::new(x - si / 2.0, iy, si, si));
            off = gap + si + gap;
        }
        _ => {}
    }
    let time_slot = |rect: Rect| Slot { rect, text: time_txt.to_string(), size: ts };
    let has_time = !time_txt.is_empty();
    let mut time: Option<Slot> = None;
    if has_time {
        match st.time.at.as_str() {
            "opposite" => {
                let h = 1.25 * ts;
                let y = if d > 0.0 { edge_o - off - h } else { edge_o + off };
                let l = x - other_w / 2.0;
                let dx = clamp_dx(l, other_w, rl, rr);
                time = Some(time_slot(Rect::new(l + dx, y, other_w, h)));
            }
            "in-marker" => time = Some(time_slot(marker)),
            "in-axis" => {
                let w = (col - 2.0 * gap).max(1.0);
                let (y, h) = if st.time.stagger {
                    if i.is_multiple_of(2) { (-e.thick / 2.0, e.thick / 2.0) } else { (0.0, e.thick / 2.0) }
                } else {
                    (-e.thick / 2.0, e.thick)
                };
                time = Some(time_slot(Rect::new(x - w / 2.0, y, w, h)));
            }
            _ => {}
        }
    }

    // text column width
    let card = st.text.kind == "card";
    let base = if alt { 2.0 * col - 3.0 * gap } else { col - 2.0 * gap };
    let base = if st.text.width > 0.0 { base.min(st.text.width * s) } else { base };
    let centred = card || st.text.align == "center";
    let col_w = if centred { base.min(2.0 * (x - rl).min(rr - x)) } else { base.min(rr - x - gap) }.max(1.0);
    let padc = if card { st.card.pad * s } else { 0.0 };
    let tw = if card { (col_w - 2.0 * padc).max(1.0) } else { col_w };

    let tsz = st.text.title_size * s;
    let bsz = st.text.body_size * s;
    let up = |t: &str| if st.text.caps { t.to_uppercase() } else { t.to_string() };
    let tl = wrap(&up(p.title.trim()), tsz, tw, 2);
    let bl = wrap(&up(p.body.trim()), bsz, tw, 4);
    let th = tl.len() as f64 * 1.25 * tsz;
    let bh = bl.len() as f64 * 1.25 * bsz;
    let strip = card && st.card.header && !tl.is_empty();

    let mut items: Vec<(K, f64)> = vec![];
    if st.icon.at == "with-text" {
        items.push((K::Icon, si));
    }
    if st.time.at == "with-text" && has_time {
        items.push((K::Time, 1.25 * ts));
    }
    if !tl.is_empty() && !strip {
        items.push((K::Title, th));
    }
    if !bl.is_empty() {
        items.push((K::Body, bh));
    }
    let half = gap / 2.0;
    let stack: f64 = if items.is_empty() { 0.0 } else { items.iter().map(|t| t.1).sum::<f64>() + half * (items.len() as f64 - 1.0) };
    let strip_h = if strip { th + padc } else { 0.0 };
    let has_any = strip || !items.is_empty();

    // the rule's place: `far` is under the block below the axis, over it above the axis
    let rule_at = match st.text.rule.as_str() {
        "far" => if d > 0.0 { "under" } else { "over" },
        "under" => "under",
        "over" => "over",
        _ => "none",
    };

    // block size, left edge, horizontal shift
    let rule_ex = rule_at != "none" && has_any && !card;
    let ex_top = if rule_ex && rule_at == "over" { half } else { 0.0 };
    let ex_bot = if rule_ex && rule_at == "under" { half } else { 0.0 };
    let block_h = if card {
        if has_any { strip_h + if items.is_empty() { 0.0 } else { 2.0 * padc + stack } } else { 0.0 }
    } else {
        stack + ex_top + ex_bot
    };
    let (left0, span) = if card { (x - col_w / 2.0, col_w) } else if centred { (x - tw / 2.0, tw) } else { (x + gap, tw) };
    let dx = clamp_dx(left0, span, rl, rr);
    let card_left = left0 + dx;
    let text_left = if card { card_left + padc } else { card_left };
    let btop = if d > 0.0 { start } else { start - block_h };

    let mut title_slot = None;
    let mut body_slot = None;
    let mut card_rect = None;
    let mut card_header = None;
    let mut icon_rect = icon;
    let mut first_y = None;
    let mut last_b = 0.0;
    let mut cursor;
    if card && has_any {
        card_rect = Some(Rect::new(card_left, btop, col_w, block_h));
        if strip {
            card_header = Some(Rect::new(card_left, btop, col_w, strip_h));
            let r = Rect::new(text_left, btop + padc / 2.0, tw, th);
            title_slot = Some(Slot { rect: r, text: tl.join("\n"), size: tsz });
            first_y = Some(r.y);
            last_b = r.bottom();
        }
        cursor = btop + strip_h + padc;
    } else {
        cursor = btop + ex_top;
    }
    for &(kind, h) in &items {
        let r = match kind {
            K::Icon => {
                let ix = if centred { text_left + tw / 2.0 - si / 2.0 } else { text_left };
                let r = Rect::new(ix, cursor, si, si);
                icon_rect = Some(r);
                r
            }
            K::Time => {
                let r = Rect::new(text_left, cursor, tw, h);
                time = Some(time_slot(r));
                r
            }
            K::Title => {
                let r = Rect::new(text_left, cursor, tw, h);
                title_slot = Some(Slot { rect: r, text: tl.join("\n"), size: tsz });
                r
            }
            K::Body => {
                let r = Rect::new(text_left, cursor, tw, h);
                body_slot = Some(Slot { rect: r, text: bl.join("\n"), size: bsz });
                r
            }
        };
        if first_y.is_none() {
            first_y = Some(r.y);
        }
        last_b = r.bottom();
        cursor += h + half;
    }

    // the short rule
    let mut rule: Option<Line> = None;
    if let (true, Some(fy)) = (rule_at != "none" && has_any, first_y) {
        let ry = if rule_at == "over" { fy - half } else { last_b + half };
        let len = tw.min(40.0 * s);
        rule = Some([(text_left, ry), (text_left + len, ry)]);
    }

    PointGeom { x, side, color, marker, time, title: title_slot, body: body_slot, card: card_rect, card_header, stem, icon: icon_rect, rule }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::Point;

    fn frame() -> Rect {
        Rect::new(0.0, 0.0, 672.0, 357.0)
    }
    fn spec(n: usize) -> TimelineSpec {
        let mut t = TimelineSpec::default();
        for i in 0..n {
            t.points.push(Point { time: format!("{}", 2021 + i), title: "Pilot".into(), body: "Two sites, one protocol.".into(), ..Default::default() });
        }
        t
    }
    fn region(f: Rect, pad: f64) -> Rect {
        f.inflate(-pad)
    }
    fn parts(p: &PointGeom) -> Vec<Rect> {
        rects(p)
    }

    #[test]
    fn points_are_equally_spaced_in_order() {
        let p = plan(&spec(5), &Style::default(), frame());
        assert_eq!(p.points.len(), 5);
        let col = (672.0 - 24.0) / 5.0;
        assert!((p.points[0].x - (12.0 + col / 2.0)).abs() < 1e-6);
        for w in p.points.windows(2) {
            assert!((w[1].x - w[0].x - col).abs() < 1e-6);
        }
        assert!(plan(&spec(0), &Style::default(), frame()).points.is_empty());
    }

    #[test]
    fn sides_alternate_from_the_first() {
        let mut st = Style::default();
        st.text.side = "alternate".into();
        st.text.first = "above".into();
        let p = plan(&spec(5), &st, frame());
        let sides: Vec<Side> = p.points.iter().map(|q| q.side).collect();
        assert_eq!(sides, vec![Side::Above, Side::Below, Side::Above, Side::Below, Side::Above]);
        st.text.first = "below".into();
        assert_eq!(plan(&spec(2), &st, frame()).points[0].side, Side::Below);
        st.text.side = "above".into();
        assert!(plan(&spec(3), &st, frame()).points.iter().all(|q| q.side == Side::Above));
    }

    #[test]
    fn the_palette_goes_round_and_a_points_own_colour_wins() {
        let mut st = Style::default();
        st.colors.palette = vec!["111111".into(), "222222".into()];
        let mut sp = spec(5);
        sp.points[1].color = "aabbcc".into();
        sp.points[2].color = "nothex".into();
        let c: Vec<String> = plan(&sp, &st, frame()).points.iter().map(|q| q.color.clone()).collect();
        assert_eq!(c, vec!["111111", "AABBCC", "111111", "222222", "111111"]);
        st.colors.mode = "single".into();
        assert!(plan(&spec(3), &st, frame()).points.iter().all(|q| q.color == "111111"));
        st.colors.palette.clear();
        assert_eq!(plan(&spec(1), &st, frame()).points[0].color, "5F6368");
    }

    #[test]
    fn a_title_takes_the_top_and_its_dots_follow_the_points() {
        let mut st = Style::default();
        st.title.legend_dots = true;
        st.title.caps = true;
        let mut sp = spec(4);
        sp.title = "History".into();
        let p = plan(&sp, &st, frame());
        let t = p.title.clone().unwrap();
        assert_eq!(t.text, "HISTORY");
        assert_eq!(t.rect.y, 0.0);
        assert!((t.rect.w - (672.0 - 24.0)).abs() < 1e-6);
        assert_eq!(p.legend.len(), 4);
        assert!(p.legend[0].1 > t.rect.bottom());
        assert!((p.legend[1].0 - p.legend[0].0 - 10.0).abs() < 1e-6);
        let mid = (p.legend[0].0 + p.legend[3].0) / 2.0;
        assert!((mid - 336.0).abs() < 1e-6);
        let row_bottom = p.legend[0].1 + 5.0;
        for q in &p.points {
            for r in parts(q) {
                assert!(r.y >= row_bottom + 12.0 - 1e-6);
            }
        }
        st.title.align = "left".into();
        assert!((plan(&sp, &st, frame()).legend[0].0 - 12.0).abs() < 1e-6);
        sp.title.clear();
        let q = plan(&sp, &st, frame());
        assert!(q.title.is_none() && q.legend.is_empty());
    }

    #[test]
    fn sizes_scale_with_the_frame() {
        let full = plan(&spec(3), &Style::default(), frame());
        let half = plan(&spec(3), &Style::default(), Rect::new(0.0, 0.0, 336.0, 178.5));
        assert!((full.scale - 1.0).abs() < 1e-9);
        assert!((half.scale - 0.5).abs() < 1e-9);
        assert!((half.points[0].marker.w / full.points[0].marker.w - 0.5).abs() < 1e-9);
        let wide = plan(&spec(3), &Style::default(), Rect::new(0.0, 0.0, 1344.0, 357.0));
        assert!((wide.scale - 1.0).abs() < 1e-9);
    }

    #[test]
    fn markers_sit_on_the_axis() {
        let p = plan(&spec(4), &Style::default(), frame());
        for q in &p.points {
            assert!((q.marker.cy() - p.axis.y).abs() < 1e-6);
            assert!((q.marker.cx() - q.x).abs() < 1e-6);
            assert!((q.marker.w - 10.0 * p.scale).abs() < 1e-6);
        }
        let mut st = Style::default();
        st.marker.kind = "block".into();
        let b = plan(&spec(2), &st, frame());
        assert!(b.points[0].marker.w > b.points[0].marker.h);
        assert!((b.points[0].marker.cy() - b.axis.y).abs() < 1e-6);
    }

    #[test]
    fn a_bleeding_axis_reaches_the_frame_edges() {
        let mut st = Style::default();
        st.axis.bleed = true;
        let p = plan(&spec(4), &st, frame());
        assert_eq!((p.axis.x0, p.axis.x1), (0.0, 672.0));
        assert!((p.axis.thickness - p.scale).abs() < 1e-9);
        st.axis.bleed = false;
        let q = plan(&spec(4), &st, frame());
        let col = (672.0 - 24.0) / 4.0;
        assert!((q.axis.x0 - (q.points[0].x - col / 2.0 + 4.0)).abs() < 1e-6);
        assert!((q.axis.x1 - (q.points[3].x + col / 2.0 - 4.0)).abs() < 1e-6);
        st.axis.kind = "joins".into();
        assert_eq!(plan(&spec(2), &st, frame()).axis.thickness, 0.0);
    }

    #[test]
    fn text_all_below_centres_the_block_not_the_axis() {
        let p = plan(&spec(3), &Style::default(), frame());
        let r = region(frame(), 12.0);
        assert!(p.axis.y < r.cy());
        let above = p.bounds.y - r.y;
        let below = r.bottom() - p.bounds.bottom();
        assert!((above - below).abs() < 1e-6, "{above} {below}");
        let mut st = Style::default();
        st.axis.at = "top".into();
        let t = plan(&spec(3), &st, frame());
        assert!(t.axis.y < p.axis.y);
        st.axis.at = "bottom".into();
        let b = plan(&spec(3), &st, frame());
        assert!(b.axis.y > p.axis.y);
    }

    #[test]
    fn no_stem_means_no_line() {
        let mut st = Style::default();
        st.time.at = "none".into();
        let p = plan(&spec(2), &st, frame());
        let q = &p.points[0];
        let l = q.stem.unwrap();
        assert!((l[1].1 - l[0].1 - 18.0 * p.scale).abs() < 1e-6);
        assert!((l[0].1 - q.marker.bottom()).abs() < 1e-6);
        st.stem.kind = "none".into();
        let p = plan(&spec(2), &st, frame());
        let q = &p.points[0];
        assert!(q.stem.is_none());
        assert!((q.title.as_ref().unwrap().rect.y - q.marker.bottom() - 4.0 * p.scale).abs() < 1e-6);
        st.stem.kind = "drop".into();
        let p = plan(&spec(2), &st, frame());
        let l = p.points[0].stem.unwrap();
        assert!((l[1].1 - l[0].1 - 0.4 * p.points[0].marker.h).abs() < 1e-6);
    }

    #[test]
    fn long_text_is_wrapped_and_cut_with_an_ellipsis() {
        let mut sp = spec(5);
        sp.points[0].title = "Pilot study of two sites with one shared protocol".into();
        sp.points[0].body = "Two sites, one protocol. ".repeat(12);
        let p = plan(&sp, &Style::default(), frame());
        let q = &p.points[0];
        let t = q.title.as_ref().unwrap();
        let b = q.body.as_ref().unwrap();
        assert!(t.text.lines().count() <= 2 && b.text.lines().count() == 4);
        assert!(b.text.ends_with('…'));
        for l in b.text.lines() {
            assert!(text::width(l, b.size) <= b.rect.w + 1e-6);
        }
        assert!((b.rect.h - 4.0 * 1.25 * b.size).abs() < 1e-6);
        assert_eq!(wrap("a\nb", 9.0, 500.0, 4), vec!["a", "b"]);
        assert!(wrap("兩個地點使用同一份流程書寫的長句子需要換行才行", 9.0, 40.0, 4).len() > 1);
        sp.points[1].body.clear();
        assert!(plan(&sp, &Style::default(), frame()).points[1].body.is_none());
        let mut st = Style::default();
        st.text.caps = true;
        assert_eq!(plan(&spec(1), &st, frame()).points[0].title.as_ref().unwrap().text, "PILOT");
    }

    #[test]
    fn above_the_axis_the_body_is_nearest_the_axis() {
        let mut st = Style::default();
        st.text.side = "above".into();
        st.time.at = "with-text".into();
        let p = plan(&spec(3), &st, frame());
        let q = &p.points[0];
        let (t, ti, b) = (q.time.as_ref().unwrap(), q.title.as_ref().unwrap(), q.body.as_ref().unwrap());
        assert!(t.rect.bottom() <= ti.rect.y && ti.rect.bottom() <= b.rect.y);
        assert!(b.rect.bottom() < q.marker.y);
        st.text.side = "below".into();
        let p = plan(&spec(3), &st, frame());
        let q = &p.points[0];
        assert!(q.time.as_ref().unwrap().rect.y >= q.marker.bottom());
        assert!(q.time.as_ref().unwrap().rect.bottom() <= q.title.as_ref().unwrap().rect.y);
    }

    #[test]
    fn left_aligned_text_starts_beside_the_stem() {
        let mut st = Style::default();
        st.text.align = "left".into();
        let p = plan(&spec(4), &st, frame());
        for q in &p.points {
            let t = q.title.as_ref().unwrap();
            assert!((t.rect.x - (q.x + 4.0 * p.scale)).abs() < 1e-6);
            assert!(t.rect.right() <= 672.0 - 12.0 + 1e-6);
        }
    }

    #[test]
    fn a_card_holds_its_strip_and_its_body() {
        let mut st = Style::default();
        st.text.kind = "card".into();
        let p = plan(&spec(4), &st, frame());
        let q = &p.points[1];
        let (c, h) = (q.card.unwrap(), q.card_header.unwrap());
        let (t, b) = (q.title.as_ref().unwrap(), q.body.as_ref().unwrap());
        assert!((c.cx() - q.x).abs() < 1e-6);
        assert!((h.y - c.y).abs() < 1e-6 && (h.w - c.w).abs() < 1e-6);
        assert!((h.h - (t.rect.h + 6.0 * p.scale)).abs() < 1e-6);
        assert!(t.rect.y >= h.y && t.rect.bottom() <= h.bottom());
        assert!(b.rect.y >= h.bottom() && b.rect.bottom() <= c.bottom() + 1e-6);
        assert!(b.rect.x >= c.x && b.rect.right() <= c.right() + 1e-6);
        st.card.header = false;
        let q = &plan(&spec(4), &st, frame()).points[1];
        assert!(q.card.is_some() && q.card_header.is_none());
    }

    #[test]
    fn the_rule_is_under_or_over_the_text() {
        let mut st = Style::default();
        st.text.rule = "under".into();
        let p = plan(&spec(3), &st, frame());
        let q = &p.points[0];
        let b = q.body.as_ref().unwrap();
        let r = q.rule.unwrap();
        assert!((r[0].1 - (b.rect.bottom() + 2.0 * p.scale)).abs() < 1e-6);
        assert!((r[0].0 - b.rect.x).abs() < 1e-6);
        assert!(r[1].0 - r[0].0 <= 40.0 * p.scale + 1e-6);
        st.text.rule = "over".into();
        let p = plan(&spec(3), &st, frame());
        let q = &p.points[0];
        let t = q.title.as_ref().unwrap();
        assert!((q.rule.unwrap()[0].1 - (t.rect.y - 2.0 * p.scale)).abs() < 1e-6);
        st.text.rule = "none".into();
        assert!(plan(&spec(3), &st, frame()).points[0].rule.is_none());
    }

    #[test]
    fn a_far_rule_is_beyond_the_text_on_either_side() {
        let mut st = Style::default();
        st.text.rule = "far".into();
        st.text.side = "alternate".into();
        st.text.first = "below".into();
        let p = plan(&spec(2), &st, frame());
        let (below, above) = (&p.points[0], &p.points[1]);
        assert_eq!((below.side, above.side), (Side::Below, Side::Above));
        let b = below.body.as_ref().unwrap();
        assert!((below.rule.unwrap()[0].1 - (b.rect.bottom() + 2.0 * p.scale)).abs() < 1e-6);
        let t = above.title.as_ref().unwrap();
        assert!((above.rule.unwrap()[0].1 - (t.rect.y - 2.0 * p.scale)).abs() < 1e-6);
        // never nearer the axis than the text
        assert!(above.rule.unwrap()[0].1 < t.rect.y && below.rule.unwrap()[0].1 > b.rect.bottom());
    }

    #[test]
    fn the_time_is_across_the_axis_from_the_text() {
        let p = plan(&spec(3), &Style::default(), frame());
        let q = &p.points[0];
        let t = q.time.as_ref().unwrap();
        assert!((t.rect.bottom() - (q.marker.y - 4.0 * p.scale)).abs() < 1e-6);
        assert!((t.rect.cx() - q.x).abs() < 1e-6);
        assert_eq!(t.text, "2021");
        let mut st = Style::default();
        st.text.side = "above".into();
        let p = plan(&spec(3), &st, frame());
        let q = &p.points[0];
        assert!(q.time.as_ref().unwrap().rect.y >= q.marker.bottom());
        st.time.at = "none".into();
        assert!(plan(&spec(3), &st, frame()).points[0].time.is_none());
    }

    #[test]
    fn a_time_inside_the_marker_is_the_marker() {
        let mut st = Style::default();
        st.time.at = "in-marker".into();
        st.marker.kind = "block".into();
        let p = plan(&spec(3), &st, frame());
        let q = &p.points[0];
        assert_eq!(q.time.as_ref().unwrap().rect, q.marker);
        st.icon.at = "in-marker".into();
        let p = plan(&spec(3), &st, frame());
        let (i, m) = (p.points[0].icon.unwrap(), p.points[0].marker);
        assert!((i.w - 0.6 * m.w).abs() < 1e-6 && (i.cx() - m.cx()).abs() < 1e-6);
    }

    #[test]
    fn times_in_a_band_take_two_rows_when_staggered() {
        let mut st = Style::default();
        st.axis.kind = "band".into();
        st.axis.weight = 24.0;
        st.time.at = "in-axis".into();
        st.time.stagger = true;
        let p = plan(&spec(4), &st, frame());
        let (a, b) = (p.points[0].time.as_ref().unwrap().rect, p.points[1].time.as_ref().unwrap().rect);
        let half = p.axis.thickness / 2.0;
        assert!((a.y - (p.axis.y - half)).abs() < 1e-6 && (a.h - half).abs() < 1e-6);
        assert!((b.y - p.axis.y).abs() < 1e-6 && (b.bottom() - (p.axis.y + half)).abs() < 1e-6);
        assert!((p.points[2].time.as_ref().unwrap().rect.y - a.y).abs() < 1e-6);
        // text below: the marker bites the band's upper edge
        assert!((p.points[0].marker.cy() - (p.axis.y - half)).abs() < 1e-6);
        st.time.stagger = false;
        let q = plan(&spec(4), &st, frame());
        assert!((q.points[0].time.as_ref().unwrap().rect.h - q.axis.thickness).abs() < 1e-6);
    }

    #[test]
    fn with_times_in_the_band_the_stem_leaves_the_band_edge() {
        let mut st = Style::default();
        st.axis.kind = "band".into();
        st.axis.weight = 24.0;
        st.time.at = "in-axis".into();
        st.text.side = "alternate".into();
        st.text.first = "below".into();
        let p = plan(&spec(2), &st, frame());
        let half = p.axis.thickness / 2.0;
        let (below, above) = (&p.points[0], &p.points[1]);
        assert!((below.stem.unwrap()[0].1 - (p.axis.y + half)).abs() < 1e-6);
        assert!((above.stem.unwrap()[0].1 - (p.axis.y - half)).abs() < 1e-6);
        let l = below.stem.unwrap();
        assert!((below.title.as_ref().unwrap().rect.y - (l[1].1 + 4.0 * p.scale)).abs() < 1e-6);
        // the marker bites the band's edge away from the text
        assert!((below.marker.cy() - (p.axis.y - half)).abs() < 1e-6);
        assert!((above.marker.cy() - (p.axis.y + half)).abs() < 1e-6);
        st.stem.kind = "none".into();
        let q = plan(&spec(2), &st, frame());
        assert!(q.points[0].stem.is_none());
        assert!((q.points[0].title.as_ref().unwrap().rect.y - (q.axis.y + half + 4.0 * q.scale)).abs() < 1e-6);
    }

    #[test]
    fn an_icon_across_the_axis_pushes_the_time_out() {
        let mut st = Style::default();
        st.icon.at = "opposite".into();
        let p = plan(&spec(3), &st, frame());
        let q = &p.points[0];
        let (i, t) = (q.icon.unwrap(), q.time.as_ref().unwrap());
        let g = 4.0 * p.scale;
        assert!((i.bottom() - (q.marker.y - g)).abs() < 1e-6);
        assert!((i.w - 28.0 * p.scale).abs() < 1e-6);
        assert!((t.rect.bottom() - (i.y - g)).abs() < 1e-6);
        st.icon.at = "with-text".into();
        let p = plan(&spec(3), &st, frame());
        let q = &p.points[0];
        assert!(q.icon.unwrap().bottom() <= q.title.as_ref().unwrap().rect.y);
        assert!(q.icon.unwrap().y >= q.marker.bottom());
    }

    fn configs() -> Vec<Style> {
        let mut v = vec![];
        for side in ["above", "below", "alternate"] {
            for card in [false, true] {
                let mut st = Style::default();
                st.text.side = side.into();
                st.text.kind = if card { "card".into() } else { "plain".into() };
                v.push(st);
            }
        }
        v
    }
    fn long_spec(n: usize) -> TimelineSpec {
        let mut s = spec(n);
        s.title = "Five years".into();
        for p in &mut s.points {
            p.body = "Two sites, one protocol. Two sites, one protocol.".into();
        }
        s
    }
    fn inside(r: Rect, f: Rect) -> bool {
        r.x >= f.x - 1e-6 && r.y >= f.y - 1e-6 && r.right() <= f.right() + 1e-6 && r.bottom() <= f.bottom() + 1e-6
    }

    #[test]
    fn everything_stays_inside_the_frame() {
        for f in [frame(), Rect::new(0.0, 0.0, 400.0, 300.0)] {
            for n in [1, 5, 10] {
                for st in configs() {
                    let p = plan(&long_spec(n), &st, f);
                    assert!(inside(p.bounds, f), "n={n} {:?} {:?} {:?}", st.text.side, st.text.kind, p.bounds);
                }
            }
        }
    }

    #[test]
    fn texts_of_neighbours_on_one_side_do_not_overlap() {
        let hit = |a: Rect, b: Rect| a.x < b.right() - 1e-6 && b.x < a.right() - 1e-6 && a.y < b.bottom() - 1e-6 && b.y < a.bottom() - 1e-6;
        for f in [frame(), Rect::new(0.0, 0.0, 400.0, 300.0)] {
            for n in [5, 10] {
                for st in configs() {
                    let p = plan(&long_spec(n), &st, f);
                    let blocks: Vec<(Side, Vec<Rect>)> = p.points.iter().map(|q| (q.side, [q.card, q.title.as_ref().map(|s| s.rect), q.body.as_ref().map(|s| s.rect)].into_iter().flatten().collect())).collect();
                    for i in 0..n {
                        for j in i + 1..n {
                            if blocks[i].0 != blocks[j].0 {
                                continue;
                            }
                            for a in &blocks[i].1 {
                                for b in &blocks[j].1 {
                                    assert!(!hit(*a, *b), "n={n} {} {:?} {i} {j}", st.text.side, st.text.kind);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn ten_points_still_fit_by_shrinking() {
        let mut st = Style::default();
        st.text.side = "alternate".into();
        st.text.kind = "card".into();
        let f = Rect::new(0.0, 0.0, 400.0, 300.0);
        let p = plan(&long_spec(10), &st, f);
        let u = 400.0 / 672.0;
        assert!(p.scale <= u + 1e-9 && p.scale >= u * 0.5 - 1e-9);
        assert!(inside(p.bounds, f));
        let one = plan(&long_spec(1), &Style::default(), frame());
        assert!((one.scale - 1.0).abs() < 1e-9);
    }

    #[test]
    fn an_unknown_value_falls_back_to_the_default() {
        let mut st = Style::default();
        st.axis.kind = "path".into();
        st.marker.kind = "star".into();
        st.text.side = "sideways".into();
        st.stem.kind = "zigzag".into();
        st.time.at = "nowhere".into();
        st.colors.mode = "rainbow".into();
        st.axis.weight = f64::NAN;
        assert_eq!(plan(&long_spec(4), &st, frame()), plan(&long_spec(4), &Style::default(), frame()));
    }

    #[test]
    fn bounds_hold_every_part() {
        let mut st = Style::default();
        st.text.rule = "under".into();
        st.icon.at = "opposite".into();
        for side in ["above", "alternate"] {
            st.text.side = side.into();
            let p = plan(&long_spec(5), &st, frame());
            let b = p.bounds.inflate(1e-6);
            assert!(inside(p.title.as_ref().unwrap().rect, b));
            assert!(b.y <= p.axis.y && p.axis.y <= b.bottom());
            for q in &p.points {
                for r in parts(q) {
                    assert!(inside(r, b));
                }
                for l in [q.stem, q.rule].into_iter().flatten() {
                    assert!(l[0].1 >= b.y && l[0].1 <= b.bottom() && l[1].1 >= b.y && l[1].1 <= b.bottom());
                    assert!(l[0].0 >= b.x && l[1].0 <= b.right());
                }
            }
        }
    }
}
