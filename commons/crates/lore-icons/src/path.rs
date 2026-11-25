//! SVG path data -> absolute `M` / `L` / `C` / `Z`.

use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    M(f64, f64),
    L(f64, f64),
    C(f64, f64, f64, f64, f64, f64),
    Z,
}

struct Scan<'a> {
    b: &'a [u8],
    i: usize,
}

impl Scan<'_> {
    fn skip(&mut self) {
        while self.i < self.b.len() && (self.b[self.i].is_ascii_whitespace() || self.b[self.i] == b',') {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.b.get(self.i).copied()
    }
    fn at_number(&mut self) -> bool {
        matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == b'+' || c == b'-' || c == b'.')
    }
    fn num(&mut self) -> Option<f64> {
        self.skip();
        let s = self.i;
        let b = self.b;
        let mut i = s;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let mut digits = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return None;
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            let ds = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > ds {
                i = j;
            }
        }
        self.i = i;
        std::str::from_utf8(&b[s..i]).ok()?.parse().ok()
    }
    /// An arc flag: one `0` or `1`, possibly glued to what follows.
    fn flag(&mut self) -> Option<bool> {
        match self.peek()? {
            b'0' => {
                self.i += 1;
                Some(false)
            }
            b'1' => {
                self.i += 1;
                Some(true)
            }
            _ => None,
        }
    }
}

/// Parse path data into absolute segments. After a `Z` a drawing command starts a new
/// subpath at the closed one's start (an explicit `M` is inserted). `None` on bad data.
pub fn parse(d: &str) -> Option<Vec<Seg>> {
    let mut sc = Scan { b: d.as_bytes(), i: 0 };
    let mut out: Vec<Seg> = Vec::new();
    let (mut cx, mut cy, mut sx, mut sy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut need_move = true;
    let mut first = true;
    // second control of the last cubic / control of the last quadratic, for S / T
    let mut last_c: Option<(f64, f64)> = None;
    let mut last_q: Option<(f64, f64)> = None;
    loop {
        let Some(ch) = sc.peek() else { break };
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        sc.i += 1;
        let rel = ch.is_ascii_lowercase();
        let cmd = ch.to_ascii_uppercase();
        if first && cmd != b'M' {
            return None;
        }
        first = false;
        let mut again_as_line = false;
        loop {
            let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
            let mut nc: Option<(f64, f64)> = None;
            let mut nq: Option<(f64, f64)> = None;
            let mut c = cmd;
            if cmd == b'M' && again_as_line {
                c = b'L';
            }
            if !matches!(c, b'M' | b'Z') && need_move {
                out.push(Seg::M(cx, cy));
                sx = cx;
                sy = cy;
                need_move = false;
            }
            match c {
                b'M' => {
                    let x = sc.num()? + ox;
                    let y = sc.num()? + oy;
                    out.push(Seg::M(x, y));
                    cx = x;
                    cy = y;
                    sx = x;
                    sy = y;
                    need_move = false;
                }
                b'L' => {
                    let x = sc.num()? + ox;
                    let y = sc.num()? + oy;
                    out.push(Seg::L(x, y));
                    cx = x;
                    cy = y;
                }
                b'H' => {
                    let x = sc.num()? + ox;
                    out.push(Seg::L(x, cy));
                    cx = x;
                }
                b'V' => {
                    let y = sc.num()? + oy;
                    out.push(Seg::L(cx, y));
                    cy = y;
                }
                b'C' => {
                    let (x1, y1) = (sc.num()? + ox, sc.num()? + oy);
                    let (x2, y2) = (sc.num()? + ox, sc.num()? + oy);
                    let (x, y) = (sc.num()? + ox, sc.num()? + oy);
                    out.push(Seg::C(x1, y1, x2, y2, x, y));
                    nc = Some((x2, y2));
                    cx = x;
                    cy = y;
                }
                b'S' => {
                    let (x1, y1) = match last_c {
                        Some((px, py)) => (2.0 * cx - px, 2.0 * cy - py),
                        None => (cx, cy),
                    };
                    let (x2, y2) = (sc.num()? + ox, sc.num()? + oy);
                    let (x, y) = (sc.num()? + ox, sc.num()? + oy);
                    out.push(Seg::C(x1, y1, x2, y2, x, y));
                    nc = Some((x2, y2));
                    cx = x;
                    cy = y;
                }
                b'Q' => {
                    let (qx, qy) = (sc.num()? + ox, sc.num()? + oy);
                    let (x, y) = (sc.num()? + ox, sc.num()? + oy);
                    quad(&mut out, cx, cy, qx, qy, x, y);
                    nq = Some((qx, qy));
                    cx = x;
                    cy = y;
                }
                b'T' => {
                    let (qx, qy) = match last_q {
                        Some((px, py)) => (2.0 * cx - px, 2.0 * cy - py),
                        None => (cx, cy),
                    };
                    let (x, y) = (sc.num()? + ox, sc.num()? + oy);
                    quad(&mut out, cx, cy, qx, qy, x, y);
                    nq = Some((qx, qy));
                    cx = x;
                    cy = y;
                }
                b'A' => {
                    let rx = sc.num()?;
                    let ry = sc.num()?;
                    let rot = sc.num()?;
                    let large = sc.flag()?;
                    let sweep = sc.flag()?;
                    let x = sc.num()? + ox;
                    let y = sc.num()? + oy;
                    arc(&mut out, cx, cy, rx, ry, rot, large, sweep, x, y);
                    cx = x;
                    cy = y;
                }
                b'Z' => {
                    if !need_move {
                        out.push(Seg::Z);
                    }
                    cx = sx;
                    cy = sy;
                    need_move = true;
                }
                _ => return None,
            }
            last_c = nc;
            last_q = nq;
            if c == b'Z' || !sc.at_number() {
                break;
            }
            again_as_line = true;
        }
    }
    Some(out)
}

fn quad(out: &mut Vec<Seg>, x0: f64, y0: f64, qx: f64, qy: f64, x: f64, y: f64) {
    out.push(Seg::C(
        x0 + 2.0 / 3.0 * (qx - x0),
        y0 + 2.0 / 3.0 * (qy - y0),
        x + 2.0 / 3.0 * (qx - x),
        y + 2.0 / 3.0 * (qy - y),
        x,
        y,
    ));
}

/// An SVG elliptical arc as cubics of at most 90 degrees each (spec F.6.5).
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<Seg>, x0: f64, y0: f64, rx: f64, ry: f64, rot: f64, large: bool, sweep: bool, x: f64, y: f64) {
    if x0 == x && y0 == y {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 {
        out.push(Seg::L(x, y));
        return;
    }
    let phi = rot.to_radians();
    let (sp, cp) = phi.sin_cos();
    let (dx2, dy2) = ((x0 - x) / 2.0, (y0 - y) / 2.0);
    let x1p = cp * dx2 + sp * dy2;
    let y1p = -sp * dx2 + cp * dy2;
    let lam = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lam > 1.0 {
        let s = lam.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let coef = if den == 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() };
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let cxp = sign * coef * rx * y1p / ry;
    let cyp = -sign * coef * ry * x1p / rx;
    let cx = cp * cxp - sp * cyp + (x0 + x) / 2.0;
    let cy = sp * cxp + cp * cyp + (y0 + y) / 2.0;
    let t1 = ((y1p - cyp) / ry).atan2((x1p - cxp) / rx);
    let t2 = ((-y1p - cyp) / ry).atan2((-x1p - cxp) / rx);
    let mut dt = t2 - t1;
    if sweep && dt < 0.0 {
        dt += 2.0 * PI;
    } else if !sweep && dt > 0.0 {
        dt -= 2.0 * PI;
    }
    let n = ((dt.abs() / (PI / 2.0)) - 1e-9).ceil().max(1.0) as usize;
    let d = dt / n as f64;
    let k = 4.0 / 3.0 * (d / 4.0).tan();
    let pt = |t: f64| {
        let (s, c) = t.sin_cos();
        (cx + rx * c * cp - ry * s * sp, cy + rx * c * sp + ry * s * cp)
    };
    let dv = |t: f64| {
        let (s, c) = t.sin_cos();
        (-rx * s * cp - ry * c * sp, -rx * s * sp + ry * c * cp)
    };
    for i in 0..n {
        let a = t1 + d * i as f64;
        let b = a + d;
        let (p1, p2) = (pt(a), pt(b));
        let (d1, d2) = (dv(a), dv(b));
        let end = if i + 1 == n { (x, y) } else { p2 };
        out.push(Seg::C(p1.0 + k * d1.0, p1.1 + k * d1.1, p2.0 - k * d2.0, p2.1 - k * d2.1, end.0, end.1));
    }
}

/// A number with at most 5 decimals, no trailing zeros.
pub fn fmt(v: f64) -> String {
    let s = format!("{:.5}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.to_string() }
}

/// Write segments scaled into the 0..1 box (`/ w`, `/ h`).
pub fn write(segs: &[Seg], w: f64, h: f64) -> String {
    let mut s = String::new();
    let p = |s: &mut String, x: f64, y: f64| {
        s.push(' ');
        s.push_str(&fmt(x / w));
        s.push(' ');
        s.push_str(&fmt(y / h));
    };
    for g in segs {
        if !s.is_empty() {
            s.push(' ');
        }
        match *g {
            Seg::M(x, y) => {
                s.push('M');
                p(&mut s, x, y);
            }
            Seg::L(x, y) => {
                s.push('L');
                p(&mut s, x, y);
            }
            Seg::C(a, b, c, d, x, y) => {
                s.push('C');
                p(&mut s, a, b);
                p(&mut s, c, d);
                p(&mut s, x, y);
            }
            Seg::Z => s.push('Z'),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(g: &Seg) -> (f64, f64) {
        match *g {
            Seg::M(x, y) | Seg::L(x, y) | Seg::C(_, _, _, _, x, y) => (x, y),
            Seg::Z => (f64::NAN, f64::NAN),
        }
    }

    #[test]
    fn packed_numbers_and_glued_arc_flags_are_read() {
        let s = parse("M.5.5 1-2").unwrap();
        assert_eq!(s, vec![Seg::M(0.5, 0.5), Seg::L(1.0, -2.0)]);
        // flags 1,0 glued to the end x (4)
        let s = parse("M0 0a2 2 0 104 0").unwrap();
        assert!(s.len() >= 3);
        assert_eq!(end(s.last().unwrap()), (4.0, 0.0));
        let s = parse("m1 1 2 2").unwrap();
        assert_eq!(s, vec![Seg::M(1.0, 1.0), Seg::L(3.0, 3.0)]);
        assert!(parse("L1 1").is_none());
        assert!(parse("M1 x").is_none());
    }

    #[test]
    fn relative_and_shorthand_commands_become_absolute_curves() {
        let s = parse("M0 0c1 0 2 1 3 3s1 2 3 0").unwrap();
        assert_eq!(s[1], Seg::C(1.0, 0.0, 2.0, 1.0, 3.0, 3.0));
        // reflected control (4,5), second control (3+1,3+2), end (6,3)
        assert_eq!(s[2], Seg::C(4.0, 5.0, 4.0, 5.0, 6.0, 3.0));
        let s = parse("M1 1H4V5h-1v-1z").unwrap();
        assert_eq!(s[1], Seg::L(4.0, 1.0));
        assert_eq!(s[2], Seg::L(4.0, 5.0));
        assert_eq!(s[3], Seg::L(3.0, 5.0));
        assert_eq!(s[4], Seg::L(3.0, 4.0));
        assert_eq!(s[5], Seg::Z);
        // quadratic raised to a cubic that keeps the ends
        let s = parse("M0 0Q3 3 6 0T12 0").unwrap();
        assert_eq!(s[1], Seg::C(2.0, 2.0, 4.0, 2.0, 6.0, 0.0));
        assert_eq!(s[2], Seg::C(8.0, -2.0, 10.0, -2.0, 12.0, 0.0));
        // a drawing command after z starts again at the start point
        let s = parse("M1 1h2zv2").unwrap();
        assert_eq!(s[3], Seg::M(1.0, 1.0));
        assert_eq!(s[4], Seg::L(1.0, 3.0));
    }

    fn bez(p0: (f64, f64), g: &Seg, t: f64) -> (f64, f64) {
        let Seg::C(a, b, c, d, x, y) = *g else { panic!() };
        let u = 1.0 - t;
        (
            u * u * u * p0.0 + 3.0 * u * u * t * a + 3.0 * u * t * t * c + t * t * t * x,
            u * u * u * p0.1 + 3.0 * u * u * t * b + 3.0 * u * t * t * d + t * t * t * y,
        )
    }

    #[test]
    fn an_arc_ends_where_it_should_and_stays_on_its_circle() {
        // a 180 degree arc round the origin, radius 2, and a 270 degree one (large flag)
        for (d, n) in [("M2 0A2 2 0 0 1-2 0", 2), ("M2 0A2 2 0 1 1 0-2", 3)] {
            let s = parse(d).unwrap();
            assert_eq!(s.len(), n + 1, "{d}");
            let mut p = end(&s[0]);
            for g in &s[1..] {
                for i in 0..=10 {
                    let q = bez(p, g, i as f64 / 10.0);
                    let r = (q.0 * q.0 + q.1 * q.1).sqrt();
                    assert!((r - 2.0).abs() < 1e-3, "{d}: radius {r}");
                }
                p = end(g);
            }
            let e = end(s.last().unwrap());
            let want = if n == 2 { (-2.0, 0.0) } else { (0.0, -2.0) };
            assert!((e.0 - want.0).abs() < 1e-9 && (e.1 - want.1).abs() < 1e-9);
        }
        // radii too small are scaled up so the arc still reaches its end
        let s = parse("M0 0A1 1 0 0 1 10 0").unwrap();
        assert_eq!(end(s.last().unwrap()), (10.0, 0.0));
    }

    #[test]
    fn a_half_circle_arc_pair_closes_a_circle() {
        let s = parse("M0 0a5 5 0 0 1 10 0a5 5 0 0 1-10 0z").unwrap();
        assert_eq!(s.last(), Some(&Seg::Z));
        let ends: Vec<_> = s.iter().filter(|g| matches!(g, Seg::C(..))).map(end).collect();
        assert_eq!(ends.len(), 4);
        assert!((ends[1].0 - 10.0).abs() < 1e-9 && ends[1].1.abs() < 1e-9);
        assert!(ends[3].0.abs() < 1e-9 && ends[3].1.abs() < 1e-9);
        // the first half bulges to one side only (centre at 5,0 with sweep 1: y negative)
        let Seg::C(_, y1, _, _, _, _) = s[1] else { panic!() };
        assert!(y1 < 0.0);
    }

    #[test]
    fn numbers_are_written_short() {
        assert_eq!(fmt(0.5), "0.5");
        assert_eq!(fmt(-0.000001), "0");
        assert_eq!(fmt(1.0 / 3.0), "0.33333");
        assert_eq!(write(&[Seg::M(12.0, 6.0), Seg::Z], 24.0, 24.0), "M 0.5 0.25 Z");
    }
}
