// LorePlot chart types (2-D). Each registers CHARTS.<type> = { label, group, prepare(panel, table, style, ctx) }
// and prepare returns a plan: { x, y, legend[], points[], draw(c) } (cartesian) or { polar: true, … }.
//   x / y: { kind: "linear"|"band", values | cats, zero, fixed, top, reverse }
//   plan extras: legendPos, legendPrefer, ends (direct labels), square, noTicks, spines, zeroBase,
//                horizontal, colorbar { vmin, vmax, cmap, label }, barAt / barTop / tops (brackets), heat
//   draw(c): c.sx, c.sy (scales), c.out (svg strings), c.T(text, font), c.font(which), c.inset, c.shapes
// Every number in here is a drawing default; formats (formats.json) and the spec choose the rest.
(() => {
  "use strict";
  const LP = window.LorePlot;
  const { r2, isNum, num, clamp, col, ncol, markerPath, dashArray, polyline, cmap, luminance, darker, hex, toHex, fmtNum, linearTicks, quantile, mean, sd, kde, hull, ellipse, trapz, rocFrom, prFrom, kaplanMeier } = LP._;
  const CHARTS = LP.CHARTS;

  // ── series colours: palette order; "ours" in the emphasis colour, the rest muted; highlight ──
  const mix = (c, to, k) => { const a = hex(c), b = hex(to); return toHex(a.map((v, i) => v + (b[i] - v) * k)); };
  const seriesOf = (p) => (p.series ?? []).filter((s) => !s.hidden);
  function seriesStyle(st, s, i, kind, p) {
    const pal = st.colors.palette?.length ? st.colors.palette : LP._.ROOT_STYLE.colors.palette;
    const lines = st.colors.lines?.length ? st.colors.lines : null;
    const list = (p?.series ?? []).filter((q) => !q.hidden);
    const anyOurs = list.some((q) => q.ours);
    let fill = pal[i % pal.length];
    let color = kind === "line" && lines ? lines[i % lines.length] : fill;
    if (s.ours) {
      fill = color = st.colors.emphasis || (kind === "line" && lines ? lines[0] : pal[0]);
    } else if (anyOurs) {
      const k = list.filter((q) => !q.ours).indexOf(s);
      const b = pal[(k + 1) % pal.length], c2 = kind === "line" && lines ? lines[(k + 1) % lines.length] : b;
      const m = st.colors.mute ?? 0.45;
      fill = mix(b, "#bdbdbd", m);
      color = mix(c2, "#bdbdbd", m);
    }
    const hl = p?.opts?.highlight;
    if (Array.isArray(hl) && hl.length && !hl.includes(s.label) && !hl.includes(s.y) && !s.ours) fill = color = st.colors.other ?? "#b0b0b0";
    if (s.color) fill = color = s.color;
    const markers = st.lines.markers ?? [];
    const dashes = st.lines.dashes?.length ? st.lines.dashes : ["solid"];
    return {
      color, fill,
      marker: s.marker ?? (st.lines.marker > 0 ? markers[i % (markers.length || 1)] ?? "o" : "none"),
      dash: s.dash ?? dashes[i % dashes.length],
      width: isNum(s.width) ? s.width : s.ours ? st.lines.width * 1.25 : st.lines.width,
      alpha: isNum(s.alpha) ? s.alpha : null,
      msize: isNum(s.msize) ? s.msize : st.lines.marker,
    };
  }
  LP._.seriesStyle = seriesStyle;
  const labelOf = (s, fallback) => (s.label != null && s.label !== "" ? s.label : fallback ?? s.y ?? "");
  const errOf = (tbl, s) => {
    if (!s.err) return null;
    const e = ncol(tbl, s.err);
    return e.length ? e : null;
  };
  const screen = (c, pts) => pts.map((q) => [c.sx(q[0]), c.sy(q[1])]);

  function errBars(pts, ys, err, c, st, color, horiz) {
    const cap = st.bars.cap / 2;
    const w = Math.max(0.5, st.axes.line);
    let d = "";
    pts.forEach((q, k) => {
      const e = err?.[k];
      if (!isNum(e) || !isNum(ys[k])) return;
      if (horiz) {
        if (!isNum(q[1])) return;
        const a = c.sx(ys[k] - e), b = c.sx(ys[k] + e);
        d += `M${r2(a)} ${r2(q[1])}L${r2(b)} ${r2(q[1])}`;
        if (cap > 0) d += `M${r2(a)} ${r2(q[1] - cap)}L${r2(a)} ${r2(q[1] + cap)}M${r2(b)} ${r2(q[1] - cap)}L${r2(b)} ${r2(q[1] + cap)}`;
      } else {
        if (!isNum(q[0])) return;
        const a = c.sy(ys[k] - e), b = c.sy(ys[k] + e);
        d += `M${r2(q[0])} ${r2(a)}L${r2(q[0])} ${r2(b)}`;
        if (cap > 0) d += `M${r2(q[0] - cap)} ${r2(a)}L${r2(q[0] + cap)} ${r2(a)}M${r2(q[0] - cap)} ${r2(b)}L${r2(q[0] + cap)} ${r2(b)}`;
      }
    });
    return d ? `<path d="${d}" stroke="${color}" stroke-width="${r2(w)}" fill="none"/>` : "";
  }
  /** Exponential moving average (0 = none, 0.9 = strong). */
  const ema = (ys, a) => { let m = null; return ys.map((y) => (isNum(y) ? (m = m == null ? y : a * m + (1 - a) * y) : NaN)); };

  // ── line (+ area, smoothing, bands, direct labels) ──────────────────────
  CHARTS.line = {
    label: "折線", group: "line",
    prepare(p, tbl, st) {
      const xs = ncol(tbl, p.x);
      const xcat = !p.x || xs.every((v) => !isNum(v));
      const cats = xcat ? col(tbl, p.x).map((v) => String(v ?? "")) : null;
      const area = p.opts?.area; // "fill" | "stack"
      const ser = seriesOf(p).map((s, i) => {
        const ys = ncol(tbl, s.y);
        return { s, i, st: seriesStyle(st, s, i, "line", p), ys, err: errOf(tbl, s), label: labelOf(s) };
      });
      if (area === "stack") {
        const base = new Array(xs.length || cats?.length || 0).fill(0);
        for (const r of ser) { r.base = [...base]; r.ys = r.ys.map((y, k) => (base[k] += isNum(y) ? y : 0)); }
      }
      const X = (k) => (xcat ? k : xs[k]);
      const allY = ser.flatMap((r) => r.ys.flatMap((y, k) => (r.err ? [y - r.err[k], y + r.err[k]] : [y]))).concat(area ? [0] : []);
      const smooth = isNum(p.opts?.smooth) && p.opts.smooth > 0 ? clamp(p.opts.smooth, 0, 0.99) : 0;
      return {
        x: xcat ? { kind: "band", cats } : { kind: "linear", values: xs },
        y: { kind: "linear", values: allY, zero: !!area },
        legend: ser.map((r) => ({ label: r.label, kind: area ? "patch" : "line", ...r.st, color: r.st.color, alpha: area ? 0.6 : null, part: `series:${r.i}` })),
        points: ser.flatMap((r) => r.ys.map((y, k) => [X(k), y])),
        ends: ser.map((r) => { let k = r.ys.length - 1; while (k >= 0 && !isNum(r.ys[k])) k--; return k < 0 ? null : { label: r.label, color: r.st.color, x: X(k), y: smooth ? ema(r.ys, smooth)[k] : r.ys[k], part: `series:${r.i}` }; }).filter(Boolean),
        legendPrefer: p.opts?.legend_prefer,
        draw(c) {
          const band = p.opts?.err_style !== "bars";
          for (const r of ser) {
            const g = [];
            const pts = r.ys.map((y, k) => [c.sx(X(k)), c.sy(y)]);
            if (area) {
              const lo = r.base ? r.ys.map((_, k) => [c.sx(X(k)), c.sy(r.base[k])]) : r.ys.map((_, k) => [c.sx(X(k)), c.sy(0)]);
              g.push(`<path d="${polyline(pts.concat(lo.reverse()))}Z" fill="${r.st.color}" fill-opacity="${area === "stack" ? 0.7 : 0.25}" stroke="none"/>`);
            }
            if (r.err && band) {
              const up = r.ys.map((y, k) => [c.sx(X(k)), c.sy(y + r.err[k])]);
              const dn = r.ys.map((y, k) => [c.sx(X(k)), c.sy(y - r.err[k])]).reverse();
              g.push(`<path d="${polyline(up.concat(dn))}Z" fill="${r.st.color}" fill-opacity="${p.opts?.band_alpha ?? 0.2}" stroke="none"/>`);
            }
            if (r.err && !band) g.push(errBars(pts, r.ys, r.err, c, st, r.st.color));
            const op = r.st.alpha != null ? ` stroke-opacity="${r.st.alpha}"` : "";
            let shown = pts;
            if (smooth) {
              g.push(`<path d="${polyline(pts)}" fill="none" stroke="${r.st.color}" stroke-width="${r2(r.st.width * 0.8)}" stroke-opacity="0.25"/>`);
              shown = ema(r.ys, smooth).map((y, k) => [c.sx(X(k)), c.sy(y)]);
            }
            let d = polyline(shown);
            if (p.opts?.step) d = polyline(shown.flatMap((q, k) => (k ? [[q[0], shown[k - 1][1]], q] : [q])));
            g.push(`<path d="${d}" fill="none" stroke="${r.st.color}" stroke-width="${r.st.width}" stroke-linejoin="round" stroke-linecap="round"${dashArray(r.st.dash, r.st.width)}${op}/>`);
            if (!smooth) for (const q of pts) if (isNum(q[0]) && isNum(q[1])) g.push(markerPath(r.st.marker, q[0], q[1], r.st.msize, r.st.color, { stroke: r.st.width * 0.6 }));
            c.shapes?.lines.push(shown.filter((q) => isNum(q[0]) && isNum(q[1])));
            c.out.push(`<g data-part="series:${r.i}">${g.join("")}</g>`);
          }
        },
      };
    },
  };
  CHARTS.area = { label: "面積", group: "line", prepare: (p, t, st) => CHARTS.line.prepare({ ...p, opts: { area: "stack", ...(p.opts ?? {}) } }, t, st) };

  // ── bar (grouped, stacked, percent, horizontal, sorted) ─────────────────
  CHARTS.bar = {
    label: "長條", group: "bar",
    prepare(p, tbl, st) {
      let cats = col(tbl, p.x).map((v) => String(v ?? ""));
      const stacked = p.opts?.mode === "stacked" || p.opts?.mode === "percent";
      const percent = p.opts?.mode === "percent";
      const horiz = !!p.opts?.horizontal;
      let ser = seriesOf(p).map((s, i) => ({ s, i, st: seriesStyle(st, s, i, "bar", p), ys: ncol(tbl, s.y), err: errOf(tbl, s), label: labelOf(s) }));
      // sorted by the first series (or the totals)
      if (p.opts?.sort === "desc" || p.opts?.sort === "asc") {
        const key = cats.map((_, k) => ser.reduce((a, r) => a + (isNum(r.ys[k]) ? r.ys[k] : 0), 0));
        const order = cats.map((_, k) => k).sort((a, b) => (p.opts.sort === "desc" ? key[b] - key[a] : key[a] - key[b]));
        cats = order.map((k) => cats[k]);
        ser = ser.map((r) => ({ ...r, ys: order.map((k) => r.ys[k]), err: r.err ? order.map((k) => r.err[k]) : null }));
      }
      if (percent) {
        const tot = cats.map((_, k) => ser.reduce((a, r) => a + (isNum(r.ys[k]) ? r.ys[k] : 0), 0) || 1);
        ser = ser.map((r) => ({ ...r, ys: r.ys.map((y, k) => (isNum(y) ? (100 * y) / tot[k] : y)), err: null }));
      }
      // horizontal bars: the first category on top
      const n = ser.length || 1;
      const tops = cats.map((_, k) => (stacked ? ser.reduce((a, r) => a + (isNum(r.ys[k]) ? r.ys[k] : 0), 0) : Math.max(...ser.map((r) => (r.ys[k] ?? 0) + (r.err?.[k] ?? 0)))));
      const vals = stacked ? tops.concat(0) : ser.flatMap((r) => r.ys.map((y, k) => y + (r.err?.[k] ?? 0)).concat(r.ys.map((y, k) => y - (r.err?.[k] ?? 0))));
      const zero = !(isNum((horiz ? p.xaxis : p.yaxis)?.min));
      const catAx = { kind: "band", cats, reverse: horiz }, valAx = { kind: "linear", values: vals, zero: zero && !p.opts?.zoom, fixed: percent ? [0, 100] : undefined };
      const groupW = (step) => Math.abs(step) * st.bars.width;
      const barW = (step) => { const g = Math.abs(step) * (st.bars.gap ?? 0); return stacked ? groupW(step) : (groupW(step) - g * (n - 1)) / n; };
      return {
        x: horiz ? valAx : catAx,
        y: horiz ? catAx : valAx,
        horizontal: horiz,
        zeroBase: true,
        legend: ser.length > 1 || p.opts?.legend_single ? ser.map((r) => ({ label: r.label, kind: "patch", ...r.st, color: r.st.fill, part: `series:${r.i}` })) : [],
        points: [],
        tops,
        barAt(c, k, si) {
          const band = horiz ? c.sy : c.sx;
          if (stacked || !isNum(si)) return band(k);
          const g = Math.abs(band.step) * (st.bars.gap ?? 0);
          return band(k) - groupW(band.step) / 2 + si * (barW(band.step) + g) + barW(band.step) / 2;
        },
        barTop(k, si) { const r = ser[si]; return r ? (r.ys[k] ?? 0) + (r.err?.[k] ?? 0) : tops[k]; },
        draw(c) {
          const band = horiz ? c.sy : c.sx, val = horiz ? c.sx : c.sy;
          const gw = groupW(band.step), bw = barW(band.step), gap = Math.abs(band.step) * (st.bars.gap ?? 0);
          // one series whose categories include "ours" (opts.emphasis): that category in the emphasis colour, the rest muted
          const emph = Array.isArray(p.opts?.emphasis) ? p.opts.emphasis.map(String) : [];
          const pal = st.colors.palette;
          const catFill = (k, base) => {
            if (!emph.length || ser.length !== 1 || ser[0].s.color) return base;
            return emph.includes(cats[k]) ? st.colors.emphasis || pal[0] : mix(pal[1 % pal.length], "#bdbdbd", st.colors.mute ?? 0.45);
          };
          const base = new Array(cats.length).fill(0);
          const edge = st.bars.edge > 0 ? ` stroke="${st.bars.edge_color || "#000"}" stroke-width="${st.bars.edge}"` : "";
          const [ra, rb] = [Math.min(...val.range), Math.max(...val.range)];
          for (const r of ser) {
            const g = [];
            const centers = [];
            cats.forEach((_, k) => {
              const y = r.ys[k];
              if (!isNum(y)) { centers.push([NaN, NaN]); return; }
              const c0 = band(k) - gw / 2 + (stacked ? 0 : r.i * (bw + gap));
              const v0 = stacked ? base[k] : 0, v1 = stacked ? base[k] + y : y;
              if (stacked) base[k] += y;
              const a = clamp(val(v0), ra, rb), b = clamp(val(v1), ra, rb);
              const alpha = r.st.alpha ?? st.bars.alpha;
              const op = alpha < 1 ? ` fill-opacity="${alpha}"` : "";
              const rect = horiz ? { x: Math.min(a, b), y: c0, w: Math.abs(b - a), h: bw } : { x: c0, y: Math.min(a, b), w: bw, h: Math.abs(b - a) };
              g.push(`<rect x="${r2(rect.x)}" y="${r2(rect.y)}" width="${r2(rect.w)}" height="${r2(rect.h)}" fill="${catFill(k, r.st.fill)}"${op}${edge}/>`);
              c.shapes?.rects.push(rect);
              centers.push(horiz ? [b, c0 + bw / 2] : [c0 + bw / 2, b]);
            });
            if (r.err && !stacked) {
              const errColor = st.bars.err_color || st.colors.axis;
              if (horiz) g.push(errBars(centers, r.ys, r.err, c, st, errColor, true));
              else g.push(errBars(centers.map((q) => [q[0], 0]), r.ys, r.err, c, st, errColor));
            }
            if (p.opts?.values && !c.inset) {
              const f = c.font("legend");
              cats.forEach((_, k) => {
                const y = r.ys[k], q = centers[k];
                if (!isNum(y) || !isNum(q[0])) return;
                const t = c.T(fmtNum(y, p.opts.fmt ?? (percent ? ".0f" : ""), c.valStep) + (percent && !p.opts.fmt ? "%" : ""), f);
                if (stacked) { g.push(t.place(horiz ? q[0] - 1.5 : q[0], horiz ? q[1] : q[1] + t.asc + 1, { anchor: horiz ? "end" : "middle", valign: horiz ? "middle" : "baseline" })); return; }
                const lift = (r.err?.[k] ? Math.abs(val(y + r.err[k]) - val(y)) : 0) + 1.5;
                g.push(horiz ? t.place(q[0] + lift, q[1], { valign: "middle" }) : t.place(q[0], q[1] - lift - t.desc, { anchor: "middle" }));
              });
            }
            c.out.push(`<g data-part="series:${r.i}">${g.join("")}</g>`);
          }
        },
      };
    },
  };

  // ── dumbbell / paired (B5): two values per category, joined ─────────────
  CHARTS.dumbbell = {
    label: "啞鈴（成對）", group: "bar",
    prepare(p, tbl, st) {
      const cats = col(tbl, p.x).map((v) => String(v ?? ""));
      const ser = seriesOf(p).slice(0, 2).map((s, i) => ({ s, i, st: seriesStyle(st, s, i, "line", p), ys: ncol(tbl, s.y), label: labelOf(s) }));
      return {
        x: { kind: "linear", values: ser.flatMap((r) => r.ys) },
        y: { kind: "band", cats, reverse: true },
        legend: ser.map((r) => ({ label: r.label, kind: "marker", ...r.st, marker: "o", color: r.st.color, part: `series:${r.i}` })),
        points: ser.flatMap((r) => r.ys.map((v, k) => [v, k])),
        legendPos: "above",
        draw(c) {
          const g = [];
          cats.forEach((_, k) => {
            const [a, b] = [ser[0]?.ys[k], ser[1]?.ys[k]];
            if (isNum(a) && isNum(b)) g.push(`<path d="M${r2(c.sx(a))} ${r2(c.sy(k))}L${r2(c.sx(b))} ${r2(c.sy(k))}" stroke="#a6a6a6" stroke-width="${r2(st.lines.width * 1.4)}"/>`);
          });
          c.out.push(`<g data-part="links">${g.join("")}</g>`);
          for (const r of ser) c.out.push(`<g data-part="series:${r.i}">${r.ys.map((v, k) => (isNum(v) ? markerPath("o", c.sx(v), c.sy(k), r.st.msize * 1.25, r.st.color, { stroke: 0.4, edge: "#fff" }) : "")).join("")}</g>`);
        },
      };
    },
  };

  // ── scatter (clusters, fit + CI, r/p, identity, counts, bubbles, hexbin) ──
  // Student t two-sided p-value (regularised incomplete beta, continued fraction)
  function betacf(a, b, x) {
    let m, aa, c = 1, d = 1 - ((a + b) * x) / (a + 1);
    if (Math.abs(d) < 1e-30) d = 1e-30;
    d = 1 / d;
    let h = d;
    for (m = 1; m <= 200; m++) {
      const m2 = 2 * m;
      aa = (m * (b - m) * x) / ((a - 1 + m2) * (a + m2));
      d = 1 + aa * d; if (Math.abs(d) < 1e-30) d = 1e-30; c = 1 + aa / c; if (Math.abs(c) < 1e-30) c = 1e-30; d = 1 / d; h *= d * c;
      aa = (-(a + m) * (a + b + m) * x) / ((a + m2) * (a + 1 + m2));
      d = 1 + aa * d; if (Math.abs(d) < 1e-30) d = 1e-30; c = 1 + aa / c; if (Math.abs(c) < 1e-30) c = 1e-30; d = 1 / d;
      const del = d * c; h *= del;
      if (Math.abs(del - 1) < 3e-12) break;
    }
    return h;
  }
  function lgamma(z) {
    const g = [76.18009172947146, -86.50532032941677, 24.01409824083091, -1.231739572450155, 0.1208650973866179e-2, -0.5395239384953e-5];
    let x = z, y = z, t = x + 5.5;
    t -= (x + 0.5) * Math.log(t);
    let s = 1.000000000190015;
    for (const q of g) s += q / ++y;
    return -t + Math.log((2.5066282746310005 * s) / x);
  }
  function ibeta(x, a, b) {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    const bt = Math.exp(lgamma(a + b) - lgamma(a) - lgamma(b) + a * Math.log(x) + b * Math.log(1 - x));
    return x < (a + 1) / (a + b + 2) ? (bt * betacf(a, b, x)) / a : 1 - (bt * betacf(b, a, 1 - x)) / b;
  }
  const tPvalue = (t, df) => ibeta(df / (df + t * t), df / 2, 0.5);
  const tCrit = (df) => { let lo = 0, hi = 50; for (let k = 0; k < 60; k++) { const m = (lo + hi) / 2; if (tPvalue(m, df) > 0.05) lo = m; else hi = m; } return (lo + hi) / 2; };
  LP._.tPvalue = tPvalue;
  function fit(xs, ys) {
    const n = xs.length, mx = mean(xs), my = mean(ys);
    let sxx = 0, sxy = 0, syy = 0;
    for (let k = 0; k < n; k++) { sxx += (xs[k] - mx) ** 2; sxy += (xs[k] - mx) * (ys[k] - my); syy += (ys[k] - my) ** 2; }
    const b = sxy / (sxx || 1), a = my - b * mx;
    const r = sxy / Math.sqrt((sxx || 1) * (syy || 1));
    const se2 = ys.reduce((s, y, k) => s + (y - (a + b * xs[k])) ** 2, 0) / Math.max(1, n - 2);
    const t = r * Math.sqrt((n - 2) / Math.max(1e-12, 1 - r * r));
    return { a, b, r, n, mx, sxx, se2, p: n > 2 ? tPvalue(Math.abs(t), n - 2) : NaN };
  }
  LP._.fit = fit;
  const fmtP = (p) => (!isNum(p) ? "" : p < 0.001 ? "$p<0.001$" : `$p=${p.toFixed(3)}$`);
  /** The emptiest of the four corners for a text block (fewest points). */
  function emptiest(pts, box, w, h) {
    const corners = [["upper left", box.x + 3, box.y + 3], ["upper right", box.x + box.w - w - 3, box.y + 3], ["lower right", box.x + box.w - w - 3, box.y + box.h - h - 3], ["lower left", box.x + 3, box.y + box.h - h - 3]];
    let best = corners[0], bn = Infinity;
    for (const c of corners) {
      const n = pts.filter((q) => q[0] >= c[1] && q[0] <= c[1] + w && q[1] >= c[2] && q[1] <= c[2] + h).length;
      if (n < bn) { bn = n; best = c; }
    }
    return { x: best[1], y: best[2] };
  }
  CHARTS.scatter = {
    label: "散佈／群集", group: "scatter",
    prepare(p, tbl, st) {
      const xs = ncol(tbl, p.x), ys = ncol(tbl, p.y ?? seriesOf(p)[0]?.y);
      const groups = p.group ? col(tbl, p.group).map((v) => String(v ?? "")) : null;
      const names = groups ? [...new Set(groups)] : [p.series?.[0]?.label || p.y || ""];
      const sizes = p.size ? ncol(tbl, p.size) : null;
      const smax = sizes ? Math.max(...sizes.filter(isNum)) : 1;
      const o = p.opts ?? {};
      const n = xs.filter(isNum).length;
      // many points: smaller and lighter (size ∝ 1/log n), tunable
      const auto = o.auto_size !== false && n > 200 ? clamp(st.scatter.size * (2.2 / Math.log10(n)), 0.8, st.scatter.size) : st.scatter.size;
      const sty = names.map((g, i) => {
        const s = (p.series ?? []).find((q) => q.group === g) ?? (groups ? { group: g } : p.series?.[0] ?? {});
        return { name: g, i, s, st: seriesStyle(st, s, i, "scatter", { series: names.map((nm) => (p.series ?? []).find((q) => q.group === nm) ?? { group: nm }) }), label: s.label || g };
      });
      const ok = xs.map((x, k) => isNum(x) && isNum(ys[k]));
      const all = { x: xs.filter((_, k) => ok[k]), y: ys.filter((_, k) => ok[k]) };
      const vals = { x: [...all.x], y: [...all.y] };
      if (o.identity) { const lo = Math.min(...vals.x, ...vals.y), hi = Math.max(...vals.x, ...vals.y); vals.x.push(lo, hi); vals.y.push(lo, hi); }
      const hex = o.hexbin ? hexbins(all.x, all.y, o.hexbin) : null;
      const legend = groups || p.series?.[0]?.label ? sty.map((g) => ({ label: g.label, kind: "marker", ...g.st, color: g.st.fill, marker: g.s.marker ?? "o", part: `group:${g.i}` })) : [];
      if (sizes && o.size_legend !== false) {
        const sv = sizes.filter(isNum).sort((a, b) => a - b);
        for (const q of [0.25, 0.5, 1]) { const v = quantile(sv, q); legend.push({ label: fmtNum(v, o.size_fmt ?? ""), kind: "marker", marker: "o", color: "#9a9a9a", size: st.scatter.size * (0.4 + 1.6 * Math.sqrt(Math.max(0, v) / (smax || 1))), part: "sizes" }); }
      }
      return {
        x: { kind: "linear", values: vals.x },
        y: { kind: "linear", values: vals.y },
        square: !!o.square || !!o.identity,
        noTicks: !!o.no_ticks,
        spines: o.no_ticks ? "box" : undefined,
        legend: hex ? [] : legend,
        legendPos: o.legend_pos,
        colorbar: hex ? { vmin: 0, vmax: hex.max, cmap: o.cmap ?? st.colors.cmap, label: "count" } : null,
        points: xs.map((x, k) => [x, ys[k]]),
        draw(c) {
          if (hex) {
            const g = [];
            const r = hex.r;
            for (const b of hex.bins) {
              const pts = Array.from({ length: 6 }, (_, k) => { const a = (Math.PI / 3) * k + Math.PI / 6; return [c.sx(b.x + r * Math.cos(a)), c.sy(b.y + r * Math.sin(a) * hex.ky)]; });
              g.push(`<path d="${polyline(pts)}Z" fill="${cmap(o.cmap ?? st.colors.cmap, b.n / hex.max)}"/>`);
            }
            c.out.push(`<g data-part="cells">${g.join("")}</g>`);
            return;
          }
          const alpha = isNum(o.alpha) ? o.alpha : n > (o.alpha_above ?? 5000) ? 0.6 : st.scatter.alpha;
          const scr = [];
          for (const g of sty) {
            const idx = xs.map((_, k) => k).filter((k) => (groups ? groups[k] === g.name : true) && ok[k]);
            const out = [];
            const pts = idx.map((k) => [xs[k], ys[k]]);
            if (o.hull && pts.length >= 3) out.push(`<path d="${polyline(hull(pts).map((q) => [c.sx(q[0]), c.sy(q[1])]))}Z" fill="${g.st.fill}" fill-opacity="0.12" stroke="${g.st.fill}" stroke-width="${r2(st.axes.line)}" stroke-linejoin="round"/>`);
            if (o.ellipse && pts.length >= 3) {
              const e = ellipse(pts);
              if (e) {
                const ring = [];
                for (let t = 0; t <= 72; t++) {
                  const a = (t / 72) * 2 * Math.PI, ex = e.rx * Math.cos(a), ey = e.ry * Math.sin(a);
                  ring.push([c.sx(e.cx + ex * Math.cos(e.ang) - ey * Math.sin(e.ang)), c.sy(e.cy + ex * Math.sin(e.ang) + ey * Math.cos(e.ang))]);
                }
                out.push(`<path d="${polyline(ring)}Z" fill="${g.st.fill}" fill-opacity="0.08" stroke="${g.st.fill}" stroke-width="${r2(st.axes.line * 1.2)}"${dashArray("dash", st.axes.line)}/>`);
              }
            }
            for (const k of idx) {
              const dd = sizes ? st.scatter.size * (0.4 + 1.6 * Math.sqrt(Math.max(0, sizes[k]) / (smax || 1))) : g.s.msize ?? auto;
              const sx = c.sx(xs[k]), sy = c.sy(ys[k]);
              scr.push([sx, sy]);
              out.push(markerPath(g.s.marker ?? "o", sx, sy, dd, g.st.fill, { stroke: n > 1000 ? 0 : 0.3, edge: st.scatter.edge || "#ffffff" }).replace("/>", alpha < 1 ? ` fill-opacity="${alpha}"/>` : "/>"));
            }
            if (o.centroids && pts.length && !c.inset) {
              const t = c.T(g.label, { ...c.font("legend"), weight: 700 });
              out.push(t.place(c.sx(mean(pts.map((q) => q[0]))), c.sy(mean(pts.map((q) => q[1]))), { anchor: "middle", valign: "middle" }).replace("<g ", `<g style="paint-order:stroke" `));
            }
            c.out.push(`<g data-part="group:${g.i}">${out.join("")}</g>`);
          }
          const lw = r2(st.axes.line);
          if (o.identity) {
            const [a, b] = [Math.max(Math.min(...c.sx.domain), Math.min(...c.sy.domain)), Math.min(Math.max(...c.sx.domain), Math.max(...c.sy.domain))];
            c.out.push(`<path d="M${r2(c.sx(a))} ${r2(c.sy(a))}L${r2(c.sx(b))} ${r2(c.sy(b))}" stroke="#8a8a8a" stroke-width="${lw}"${dashArray("dash", st.axes.line)}/>`);
          }
          const text = [];
          if (o.fit && all.x.length > 2) {
            const f = fit(all.x, all.y);
            const [x0, x1] = [Math.min(...all.x), Math.max(...all.x)];
            const tc = tCrit(f.n - 2);
            const band = Array.from({ length: 41 }, (_, k) => x0 + ((x1 - x0) * k) / 40).map((x) => {
              const se = Math.sqrt(f.se2 * (1 / f.n + (x - f.mx) ** 2 / (f.sxx || 1)));
              return [x, f.a + f.b * x - tc * se, f.a + f.b * x + tc * se];
            });
            const ink = o.fit_color || st.colors.axis;
            c.out.push(`<path d="${polyline(band.map((q) => [c.sx(q[0]), c.sy(q[2])]).concat(band.map((q) => [c.sx(q[0]), c.sy(q[1])]).reverse()))}Z" fill="${ink}" fill-opacity="0.12"/>`);
            c.out.push(`<path d="M${r2(c.sx(x0))} ${r2(c.sy(f.a + f.b * x0))}L${r2(c.sx(x1))} ${r2(c.sy(f.a + f.b * x1))}" stroke="${ink}" stroke-width="${r2(st.lines.width)}"/>`);
            if (o.stats !== false) text.push(`$r=${f.r.toFixed(2)}$, ${fmtP(f.p)}`);
          }
          if (o.counts) {
            const above = all.x.filter((x, k) => all.y[k] > x).length, below = all.x.filter((x, k) => all.y[k] < x).length;
            text.push(`above: ${above}　below: ${below}`);
          }
          if (text.length && !c.inset) {
            const f = c.font("legend");
            const ts = text.map((t) => c.T(t, f));
            const w = Math.max(...ts.map((t) => t.w)) + 2, h = ts.reduce((a, t) => a + t.h + 1, 0);
            const at = emptiest(scr, c.box, w, h);
            let y = at.y;
            for (const t of ts) { c.out.push(t.place(at.x + 1, y + t.asc)); y += t.h + 1; }
          }
          c.shapes?.points.push(...scr);
        },
      };
    },
  };
  function hexbins(xs, ys, size) {
    const lo = [Math.min(...xs), Math.min(...ys)], hi = [Math.max(...xs), Math.max(...ys)];
    const n = typeof size === "number" && size > 3 ? size : 30;
    const r = (hi[0] - lo[0] || 1) / n / Math.sqrt(3);
    const ky = (hi[1] - lo[1] || 1) / (hi[0] - lo[0] || 1);
    const w = r * Math.sqrt(3), hgt = r * 1.5;
    const bins = new Map();
    xs.forEach((x, k) => {
      const yy = (ys[k] - lo[1]) / ky;
      let row = Math.round(yy / hgt);
      let colI = Math.round((x - lo[0] - (row % 2 ? w / 2 : 0)) / w);
      const key = `${row},${colI}`;
      const b = bins.get(key) ?? { x: lo[0] + colI * w + (row % 2 ? w / 2 : 0), y: lo[1] + row * hgt * ky, n: 0 };
      b.n++;
      bins.set(key, b);
    });
    const list = [...bins.values()];
    return { bins: list, r, ky, max: Math.max(...list.map((b) => b.n)) };
  }

  // ── distributions: box / violin (+ raincloud, paired, horizontal, sorted) ─
  function distributions(p, tbl) {
    // groups × conditions (hue): one box per pair, side by side within the group's band
    if (p.group && p.hue && (p.y || p.series?.[0]?.y)) {
      const ycol = p.y ?? p.series[0].y;
      const g = col(tbl, p.group).map((v) => String(v ?? "")), hcol = col(tbl, p.hue).map((v) => String(v ?? "")), ys = ncol(tbl, ycol);
      const gs = [...new Set(g)], hs = [...new Set(hcol)];
      const out = [];
      gs.forEach((gn, gi) => hs.forEach((hn, hi) => {
        const vals = ys.filter((v, k) => g[k] === gn && hcol[k] === hn && isNum(v));
        out.push({ name: gn, i: hi, pos: gi, sub: hi, nh: hs.length, hue: hn, s: (p.series ?? []).find((q) => q.group === hn) ?? { group: hn }, vals, raw: [] });
      }));
      out.groups = gs;
      out.hues = hs;
      return out;
    }
    if (p.group && (p.y || p.series?.[0]?.y)) {
      const ycol = p.y ?? p.series[0].y;
      const g = col(tbl, p.group).map((v) => String(v ?? "")), ys = ncol(tbl, ycol);
      const names = [...new Set(g)];
      return names.map((n, i) => ({ name: n, i, s: (p.series ?? []).find((q) => q.group === n) ?? { group: n }, vals: ys.filter((v, k) => g[k] === n && isNum(v)), raw: ys.map((v, k) => (g[k] === n ? v : NaN)) }));
    }
    return seriesOf(p).map((s, i) => { const raw = ncol(tbl, s.y); return { name: labelOf(s), i, s, vals: raw.filter(isNum), raw }; });
  }
  function boxLike(kind) {
    return {
      label: kind === "box" ? "盒鬚圖" : "小提琴圖", group: "dist",
      prepare(p, tbl, st) {
        const o = p.opts ?? {};
        let dist = distributions(p, tbl);
        if (o.sort) {
          const med = (d) => quantile([...d.vals].sort((a, b) => a - b), 0.5);
          dist = [...dist].sort((a, b) => (o.sort === "asc" ? med(a) - med(b) : med(b) - med(a)));
        }
        const horiz = !!o.horizontal;
        const hue = !!dist.hues;
        const cats = hue ? dist.groups : dist.map((d) => d.s.label || d.name);
        const hueSeries = hue ? dist.hues.map((h) => (p.series ?? []).find((q) => q.group === h) ?? { group: h }) : null;
        const valAx = { kind: "linear", values: dist.flatMap((d) => d.vals) }, catAx = { kind: "band", cats, reverse: horiz };
        const showPts = o.points ?? (o.raincloud || dist.every((d) => d.vals.length < (o.points_below ?? 100)));
        return {
          x: horiz ? valAx : catAx,
          y: horiz ? catAx : valAx,
          horizontal: horiz,
          legend: hue ? dist.hues.map((h, k) => ({ label: hueSeries[k].label || h, kind: "patch", color: seriesStyle(st, hueSeries[k], k, "bar", { series: hueSeries }).fill, part: `series:${k}` })) : [],
          legendPos: hue ? "above" : undefined,
          points: dist.flatMap((d, i) => d.vals.map((v) => (horiz ? [v, d.pos ?? i] : [d.pos ?? i, v]))),
          draw(c) {
            const bandS = horiz ? c.sy : c.sx, valS = horiz ? c.sx : c.sy;
            const P = (b, v) => (horiz ? [valS(v), b] : [b, valS(v)]);
            const lw = r2(st.axes.line * 1.2), ink = st.colors.axis;
            const seg = (b0, v0, b1, v1) => { const [a, b] = [P(b0, v0), P(b1, v1)]; return `M${r2(a[0])} ${r2(a[1])}L${r2(b[0])} ${r2(b[1])}`; };
            const rect = (b0, b1, v0, v1) => { const a = P(b0, v0), b = P(b1, v1); return { x: Math.min(a[0], b[0]), y: Math.min(a[1], b[1]), w: Math.abs(b[0] - a[0]), h: Math.abs(b[1] - a[1]) }; };
            if (o.paired && dist.length > 1) {
              const n = Math.max(...dist.map((d) => d.raw.length));
              const lines = [];
              for (let k = 0; k < n; k++) {
                const pts = dist.map((d, i) => (isNum(d.raw[k]) ? P(bandS(i), d.raw[k]) : null)).filter(Boolean);
                if (pts.length > 1) lines.push(polyline(pts));
              }
              c.out.push(`<path d="${lines.join("")}" stroke="#9a9a9a" stroke-width="0.4" stroke-opacity="0.6" fill="none" data-part="pairs"/>`);
            }
            for (const d of dist) {
              const sst = o.mono ? { fill: d.s.color ?? st.colors.palette[0] } : seriesStyle(st, d.s, d.i, "bar", { series: hue ? hueSeries : dist.map((q) => q.s) });
              const sorted = [...d.vals].sort((a, b) => a - b);
              if (!sorted.length) continue;
              const q1 = quantile(sorted, 0.25), q2 = quantile(sorted, 0.5), q3 = quantile(sorted, 0.75);
              const iqr = q3 - q1;
              const lo = sorted.find((v) => v >= q1 - 1.5 * iqr), hi = [...sorted].reverse().find((v) => v <= q3 + 1.5 * iqr);
              const full = Math.abs(bandS.step);
              const step = hue ? (full * st.bars.width) / d.nh : full;
              const cb = hue ? bandS(d.pos) - (full * st.bars.width) / 2 + (d.sub + 0.5) * step : bandS(d.i);
              const rain = !!o.raincloud;
              const bw = step * (hue ? 0.85 : st.bars.width) * (kind === "box" && !rain ? (hue ? 1 : 0.6) : 0.9);
              const g = [];
              if (kind === "violin" || rain) {
                const [a, b] = [sorted[0], sorted.at(-1)];
                const at = Array.from({ length: 60 }, (_, k) => a + ((b - a) * k) / 59);
                const den = kde(sorted, at);
                const m = Math.max(...den) || 1;
                const half = rain ? bw * 0.55 : bw / 2;
                const off = rain ? -step * 0.08 : 0;
                const side1 = at.map((v, k) => P(cb + off - (den[k] / m) * half, v));
                const side2 = rain ? at.map((v) => P(cb + off, v)).reverse() : at.map((v, k) => P(cb + (den[k] / m) * half, v)).reverse();
                g.push(`<path d="${polyline(side1.concat(side2))}Z" fill="${sst.fill}" fill-opacity="${st.bars.alpha < 1 ? st.bars.alpha : 0.75}" stroke="${rain ? "none" : ink}" stroke-width="${r2(st.axes.line)}"/>`);
              }
              if (kind === "box" || rain) {
                const w = rain ? bw * 0.18 : bw;
                const cx = cb + (rain ? step * 0.04 : 0);
                const r = rect(cx - w / 2, cx + w / 2, q1, q3);
                g.push(`<path d="${seg(cx, lo, cx, q1)}${seg(cx, q3, cx, hi)}${rain ? "" : seg(cx - w / 4, lo, cx + w / 4, lo) + seg(cx - w / 4, hi, cx + w / 4, hi)}" stroke="${ink}" stroke-width="${lw}" fill="none"/>`);
                g.push(`<rect x="${r2(r.x)}" y="${r2(r.y)}" width="${r2(r.w)}" height="${r2(r.h)}" fill="${rain ? "#ffffff" : sst.fill}" fill-opacity="${rain ? 1 : st.bars.alpha}" stroke="${ink}" stroke-width="${lw}"/>`);
                g.push(`<path d="${seg(cx - w / 2, q2, cx + w / 2, q2)}" stroke="${ink}" stroke-width="${r2(lw * 1.4)}"/>`);
                if (!showPts) for (const v of sorted) if (v < lo || v > hi) { const q = P(cx, v); g.push(markerPath("o", q[0], q[1], 2.4, ink, { hollow: true, stroke: 0.5 })); }
              } else {
                const iw = Math.max(1.5, bw * 0.08);
                const r = rect(cb - iw / 2, cb + iw / 2, q1, q3);
                g.push(`<rect x="${r2(r.x)}" y="${r2(r.y)}" width="${r2(r.w)}" height="${r2(r.h)}" fill="${ink}"/>`);
                g.push(`<path d="${seg(cb, lo, cb, hi)}" stroke="${ink}" stroke-width="${r2(st.axes.line * 0.8)}"/>`);
                const q = P(cb, q2);
                g.push(`<circle cx="${r2(q[0])}" cy="${r2(q[1])}" r="${r2(Math.max(1, iw * 0.45))}" fill="#fff"/>`);
              }
              if (o.mean) { const q = P(cb, mean(sorted)); g.push(markerPath("^", q[0], q[1], 3.2, "#ffffff", { edge: ink, stroke: 0.6 })); }
              if (showPts) {
                let seed = 7 + d.i * 131;
                const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
                const centre = rain ? cb + step * 0.22 : cb;
                const spread = rain ? bw * 0.25 : bw * 0.55;
                for (const v of sorted) { const q = P(centre + (rnd() - 0.5) * spread, v); g.push(markerPath("o", q[0], q[1], 1.8, darker(sst.fill, 0.45), { stroke: 0 }).replace("/>", ` fill-opacity="0.7"/>`)); }
              }
              c.out.push(`<g data-part="series:${d.i}">${g.join("")}</g>`);
            }
          },
        };
      },
    };
  }
  CHARTS.box = boxLike("box");
  CHARTS.violin = boxLike("violin");

  // ── histogram (Freedman–Diaconis bins, overlap, KDE), ridge, ECDF ───────
  function fdBins(vals) {
    const s = [...vals].sort((a, b) => a - b);
    const iqr = quantile(s, 0.75) - quantile(s, 0.25);
    const w = (2 * iqr) / Math.cbrt(s.length || 1);
    return w > 0 ? Math.ceil((s.at(-1) - s[0]) / w) : Math.ceil(Math.log2(Math.max(2, s.length)) + 1);
  }
  CHARTS.hist = {
    label: "直方圖", group: "dist",
    prepare(p, tbl, st) {
      const o = p.opts ?? {};
      const ser = seriesOf(p).map((s, i) => ({ s, i, st: seriesStyle(st, s, i, "bar", p), vals: ncol(tbl, s.y).filter(isNum), label: labelOf(s) }));
      const all = ser.flatMap((r) => r.vals);
      const lo = all.length ? Math.min(...all) : 0, hi = all.length ? Math.max(...all) : 1;
      const nb = clamp(Math.round(o.bins ?? fdBins(all)), 2, 300);
      const w = (hi - lo || 1) / nb;
      const edges = Array.from({ length: nb + 1 }, (_, k) => lo + k * w);
      const density = !!o.density || !!o.kde;
      const counts = ser.map((r) => {
        const c = new Array(nb).fill(0);
        for (const v of r.vals) c[Math.min(nb - 1, Math.floor((v - lo) / w))]++;
        return density ? c.map((x) => x / ((r.vals.length || 1) * w)) : c;
      });
      const stacked = !!o.stacked;
      const kd = o.kde ? ser.map((r) => { const at = Array.from({ length: 80 }, (_, k) => lo - w + ((hi - lo + 2 * w) * k) / 79); return { at, d: kde(r.vals, at) }; }) : null;
      const tops = edges.slice(0, -1).map((_, k) => (stacked ? counts.reduce((a, c) => a + c[k], 0) : Math.max(...counts.map((c) => c[k]))));
      return {
        x: { kind: "linear", values: [lo, hi], fixed: [lo - w * 0.1, hi + w * 0.1] },
        y: { kind: "linear", values: tops.concat(kd ? kd.flatMap((q) => q.d) : []), zero: true },
        legend: ser.length > 1 || ser[0]?.s.label ? ser.map((r) => ({ label: r.label, kind: "patch", ...r.st, color: r.st.fill, alpha: ser.length > 1 && !stacked ? o.alpha ?? 0.45 : null, part: `series:${r.i}` })) : [],
        points: tops.map((t, k) => [edges[k] + w / 2, t]),
        draw(c) {
          const base = new Array(nb).fill(0);
          for (const r of ser) {
            const alpha = stacked || ser.length === 1 ? st.bars.alpha : o.alpha ?? 0.45;
            const g = counts[r.i].map((v, k) => {
              const b0 = stacked ? base[k] : 0;
              if (stacked) base[k] += v;
              const x0 = c.sx(edges[k]), x1 = c.sx(edges[k + 1]), y0 = c.sy(b0), y1 = c.sy(b0 + v);
              c.shapes?.rects.push({ x: x0, y: Math.min(y0, y1), w: x1 - x0, h: Math.abs(y0 - y1) });
              return `<rect x="${r2(x0)}" y="${r2(Math.min(y0, y1))}" width="${r2(x1 - x0)}" height="${r2(Math.abs(y0 - y1))}" fill="${r.st.fill}" fill-opacity="${alpha}" stroke="#ffffff" stroke-width="0.3"/>`;
            });
            if (kd) g.push(`<path d="${polyline(kd[r.i].at.map((x, k) => [c.sx(x), c.sy(kd[r.i].d[k])]))}" fill="none" stroke="${darker(r.st.fill, 0.3)}" stroke-width="${st.lines.width}"/>`);
            c.out.push(`<g data-part="series:${r.i}">${g.join("")}</g>`);
          }
        },
      };
    },
  };
  CHARTS.ridge = {
    label: "山脊圖", group: "dist",
    prepare(p, tbl, st) {
      const dist = distributions(p, tbl);
      const all = dist.flatMap((d) => d.vals);
      const lo = Math.min(...all), hi = Math.max(...all);
      const at = Array.from({ length: 90 }, (_, k) => lo + ((hi - lo) * k) / 89);
      const dens = dist.map((d) => kde(d.vals, at));
      const m = Math.max(...dens.flat()) || 1;
      const overlap = p.opts?.overlap ?? 1.6;
      return {
        x: { kind: "linear", values: [lo, hi], fixed: [lo, hi] },
        y: { kind: "band", cats: dist.map((d) => d.s.label || d.name), reverse: true },
        legend: [],
        points: [],
        spines: "b",
        draw(c) {
          dist.forEach((d, i) => {
            const sst = seriesStyle(st, d.s, i, "bar", { series: dist.map((q) => q.s) });
            const base = c.sy(i) + Math.abs(c.sy.step) * 0.45;
            const hgt = Math.abs(c.sy.step) * overlap;
            const pts = at.map((x, k) => [c.sx(x), base - (dens[i][k] / m) * hgt]);
            c.out.push(`<g data-part="series:${i}"><path d="${polyline(pts)}L${r2(c.sx(hi))} ${r2(base)}L${r2(c.sx(lo))} ${r2(base)}Z" fill="${sst.fill}" fill-opacity="0.8" stroke="#ffffff" stroke-width="${r2(st.lines.width * 0.8)}"/><path d="${polyline(pts)}" fill="none" stroke="${darker(sst.fill, 0.4)}" stroke-width="${r2(st.lines.width * 0.6)}"/></g>`);
          });
        },
      };
    },
  };
  CHARTS.ecdf = {
    label: "累積分布（ECDF）", group: "dist",
    prepare(p, tbl, st) {
      const ser = seriesOf(p).map((s, i) => ({ s, i, st: seriesStyle(st, s, i, "line", p), vals: ncol(tbl, s.y).filter(isNum).sort((a, b) => a - b), label: labelOf(s) }));
      return {
        x: { kind: "linear", values: ser.flatMap((r) => r.vals) },
        y: { kind: "linear", values: [0, 1], fixed: [0, 1.02] },
        legend: ser.map((r) => ({ label: r.label, kind: "line", ...r.st, marker: "none", part: `series:${r.i}` })),
        legendPrefer: "lower right",
        points: [],
        draw(c) {
          for (const r of ser) {
            const n = r.vals.length || 1;
            const pts = [[c.sx(r.vals[0]), c.sy(0)]];
            r.vals.forEach((v, k) => { pts.push([c.sx(v), c.sy(k / n)], [c.sx(v), c.sy((k + 1) / n)]); });
            c.shapes?.lines.push(pts);
            c.out.push(`<g data-part="series:${r.i}"><path d="${polyline(pts)}" fill="none" stroke="${r.st.color}" stroke-width="${r.st.width}"${dashArray(r.st.dash, r.st.width)}/></g>`);
          }
        },
      };
    },
  };

  // ── heatmap: confusion matrix, correlation (triangle, diverging), results table, attention, ablation grid ──
  CHARTS.heatmap = {
    label: "熱圖／混淆矩陣", group: "heat",
    prepare(p, tbl, st, ctx) {
      const o = p.opts ?? {};
      const rowsName = tbl.columns[0];
      const valueCols = p.series?.length ? seriesOf(p).map((s) => s.y) : tbl.columns.filter((c) => c !== (p.x ?? rowsName));
      const rowLabels = col(tbl, p.x ?? rowsName).map((v) => String(v ?? ""));
      const raw = rowLabels.map((_, r) => valueCols.map((cn) => num(tbl.rows[r][tbl.columns.indexOf(cn)])));
      const rowSum = raw.map((row) => row.reduce((a, v) => a + (isNum(v) ? v : 0), 0) || 1);
      let M = o.normalize === "rows" ? raw.map((row, r) => row.map((v) => v / rowSum[r])) : raw;
      const tri = o.triangle; // "lower" | "upper"
      const shown = (r, c) => !tri || (tri === "lower" ? c <= r : c >= r);
      const flat = M.flatMap((row, r) => row.filter((v, c) => isNum(v) && shown(r, c)));
      const diverging = isNum(o.center);
      let vmin = isNum(o.vmin) ? o.vmin : Math.min(...flat), vmax = isNum(o.vmax) ? o.vmax : Math.max(...flat);
      if (diverging) { const m = Math.max(Math.abs(vmin - o.center), Math.abs(vmax - o.center)); vmin = o.center - m; vmax = o.center + m; }
      if (isNum(ctx?.shared?.vmin)) { vmin = ctx.shared.vmin; vmax = ctx.shared.vmax; }
      const colLabels = seriesOf(p).length ? seriesOf(p).map((s) => labelOf(s)) : valueCols;
      const name = o.cmap || (diverging ? st.colors.diverging ?? "rdbu" : o.image ? "greys" : st.colors.cmap || "viridis");
      // best cells: per column / per row / overall, max or min
      const best = new Set();
      if (o.best) {
        const [scope, dir] = String(o.best).includes("-") ? String(o.best).split("-") : ["all", o.best];
        const better = (a, b) => (dir === "min" ? a < b : a > b);
        if (scope === "col") colLabels.forEach((_, c) => { let br = -1; M.forEach((row, r) => { if (isNum(row[c]) && (br < 0 || better(row[c], M[br][c]))) br = r; }); if (br >= 0) best.add(`${br},${c}`); });
        else if (scope === "row") M.forEach((row, r) => { let bc = -1; row.forEach((v, c) => { if (isNum(v) && (bc < 0 || better(v, row[bc]))) bc = c; }); if (bc >= 0) best.add(`${r},${bc}`); });
        else { let b = null; M.forEach((row, r) => row.forEach((v, c) => { if (isNum(v) && (!b || better(v, M[b[0]][b[1]]))) b = [r, c]; })); if (b) best.add(`${b[0]},${b[1]}`); }
      }
      return {
        x: { kind: "band", cats: colLabels, top: !!o.top },
        y: { kind: "band", cats: rowLabels, reverse: true },
        heat: true,
        square: o.square ?? (o.image || colLabels.length === rowLabels.length),
        noTicks: !!o.image,
        colorbar: o.colorbar ? { vmin, vmax, cmap: name, label: o.colorbar_label ?? "" } : null,
        legend: [],
        points: [],
        draw(c) {
          const g = [];
          const cw = Math.abs(c.sx.step), ch = Math.abs(c.sy.step);
          const sep = o.sep ? ` stroke="#ffffff" stroke-width="${isNum(o.sep) ? o.sep : 1}"` : "";
          const annot = o.image ? "none" : o.annot ?? (o.values === false ? "none" : "value");
          const fmt = o.fmt ?? (o.normalize === "rows" ? ".2f" : flat.every((q) => Number.isInteger(q)) ? "d" : ".2f");
          M.forEach((row, ri) => row.forEach((v, ci) => {
            if (!isNum(v) || !shown(ri, ci)) return;
            const [ca, cb2] = Array.isArray(o.cmap_range) ? o.cmap_range : [0, 1];
            const fill = cmap(name, ca + ((v - vmin) / (vmax - vmin || 1)) * (cb2 - ca));
            const x = c.sx(ci) - cw / 2, y = c.sy(ri) - ch / 2;
            g.push(`<rect x="${r2(x)}" y="${r2(y)}" width="${r2(cw + (sep ? 0 : 0.05))}" height="${r2(ch + (sep ? 0 : 0.05))}" fill="${fill}"${sep}/>`);
            if (annot !== "none" && !c.inset) {
              const f = { ...c.font("tick"), color: luminance(fill) > 0.4 ? "#000000" : "#ffffff" };
              if (annot === "count+row%") {
                const t1 = c.T(fmtNum(raw[ri][ci], "d"), { ...f, weight: 700 }), t2 = c.T(`${((100 * raw[ri][ci]) / rowSum[ri]).toFixed(1)}%`, { ...f, size: f.size * 0.85 });
                g.push(t1.place(c.sx(ci), c.sy(ri) - 0.5, { anchor: "middle", valign: "bottom" }), t2.place(c.sx(ci), c.sy(ri) + 0.5, { anchor: "middle", valign: "top" }));
              } else g.push(c.T(fmtNum(v, fmt), best.has(`${ri},${ci}`) ? { ...f, weight: 700 } : f).place(c.sx(ci), c.sy(ri), { anchor: "middle", valign: "middle" }));
            }
            if (best.has(`${ri},${ci}`)) g.push(`<rect x="${r2(x + 0.6)}" y="${r2(y + 0.6)}" width="${r2(cw - 1.2)}" height="${r2(ch - 1.2)}" fill="none" stroke="${o.best_color ?? "#000000"}" stroke-width="${r2(Math.max(0.8, st.axes.line * 1.5))}"/>`);
          }));
          c.out.push(`<g data-part="cells">${g.join("")}</g>`);
        },
      };
    },
  };

  // ── ROC / PR (+ prevalence baseline) / calibration / decision curve ─────
  function curveChart(kind) {
    return {
      label: kind === "roc" ? "ROC 曲線" : "PR 曲線", group: "curve",
      prepare(p, tbl, st) {
        let prevalence = NaN;
        const ser = seriesOf(p).map((s, i) => {
          let xs, ys, auc;
          if (s.score && s.truth) {
            const truth = ncol(tbl, s.truth);
            prevalence = truth.filter((t) => t > 0).length / (truth.filter(isNum).length || 1);
            ({ xs, ys, auc } = (kind === "roc" ? rocFrom : prFrom)(ncol(tbl, s.score), truth));
          } else {
            const pts = ncol(tbl, s.x ?? p.x).map((x, k) => [x, ncol(tbl, s.y)[k]]).filter((q) => isNum(q[0]) && isNum(q[1]));
            if (kind === "roc") pts.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
            xs = pts.map((q) => q[0]); ys = pts.map((q) => q[1]);
            auc = isNum(num(s.auc)) ? num(s.auc) : kind === "roc" ? trapz(xs, ys) : Math.abs(trapz(xs, ys));
          }
          const word = p.opts?.auc_label ?? (kind === "roc" ? "AUC" : "AP");
          const label = labelOf(s, s.score ?? s.y) + (p.opts?.auc !== false && isNum(auc) ? ` (${word} = ${auc.toFixed(p.opts?.auc_digits ?? 3)})` : "");
          return { s, i, xs, ys, auc, st: seriesStyle(st, s, i, "line", p), label };
        });
        const base = p.opts?.baseline === undefined || p.opts?.baseline === "auto" ? prevalence : num(p.opts.baseline);
        return {
          x: { kind: "linear", values: [0, 1], fixed: [0, 1] },
          y: { kind: "linear", values: [0, 1], fixed: [0, 1.0] },
          square: p.opts?.square !== false,
          legend: ser.map((r) => ({ label: r.label, kind: "line", ...r.st, marker: "none", part: `series:${r.i}` })),
          legendPos: kind === "roc" ? "lower right" : "lower left",
          points: ser.flatMap((r) => r.xs.map((x, k) => [x, r.ys[k]])),
          curves: ser,
          draw(c) {
            const lw = r2(st.axes.line);
            if (kind === "roc" && p.opts?.diagonal !== false) c.out.push(`<path d="M${r2(c.sx(0))} ${r2(c.sy(0))}L${r2(c.sx(1))} ${r2(c.sy(1))}" stroke="#9a9a9a" stroke-width="${lw}"${dashArray("dash", st.axes.line)} fill="none"/>`);
            if (kind === "pr" && isNum(base) && p.opts?.baseline !== false) c.out.push(`<path d="M${r2(c.sx(0))} ${r2(c.sy(base))}L${r2(c.sx(1))} ${r2(c.sy(base))}" stroke="#9a9a9a" stroke-width="${lw}"${dashArray("dash", st.axes.line)} fill="none"/>`);
            for (const r of ser) {
              const pts = r.xs.map((x, k) => [c.sx(x), c.sy(r.ys[k])]);
              const shown = kind === "pr" ? pts.flatMap((q, k) => (k ? [[q[0], pts[k - 1][1]], q] : [q])) : pts;
              c.shapes?.lines.push(shown);
              c.out.push(`<g data-part="series:${r.i}"><path d="${polyline(shown)}" fill="none" stroke="${r.st.color}" stroke-width="${r.st.width}" stroke-linejoin="round"${dashArray(r.st.dash, r.st.width)}/></g>`);
            }
          },
        };
      },
    };
  }
  CHARTS.roc = curveChart("roc");
  CHARTS.pr = curveChart("pr");

  CHARTS.calib = {
    label: "校準曲線", group: "curve",
    prepare(p, tbl, st) {
      const nb = p.opts?.bins ?? 10;
      const ser = seriesOf(p).map((s, i) => {
        const sc = ncol(tbl, s.score), tr = ncol(tbl, s.truth);
        const bins = Array.from({ length: nb }, () => ({ p: [], y: [] }));
        sc.forEach((v, k) => { if (isNum(v) && isNum(tr[k])) { const b = bins[Math.min(nb - 1, Math.max(0, Math.floor(v * nb)))]; b.p.push(v); b.y.push(tr[k] > 0 ? 1 : 0); } });
        return { s, i, st: seriesStyle(st, s, i, "line", p), label: labelOf(s, s.score), pts: bins.filter((b) => b.p.length).map((b) => [mean(b.p), mean(b.y), b.p.length]) };
      });
      const maxN = Math.max(1, ...ser.flatMap((r) => r.pts.map((q) => q[2])));
      return {
        x: { kind: "linear", values: [0, 1], fixed: [0, 1] },
        y: { kind: "linear", values: [0, 1], fixed: [0, 1] },
        square: true,
        legend: ser.map((r) => ({ label: r.label, kind: "line", ...r.st, part: `series:${r.i}` })),
        legendPos: "upper left",
        points: ser.flatMap((r) => r.pts.map((q) => [q[0], q[1]])),
        draw(c) {
          c.out.push(`<path d="M${r2(c.sx(0))} ${r2(c.sy(0))}L${r2(c.sx(1))} ${r2(c.sy(1))}" stroke="#9a9a9a" stroke-width="${r2(st.axes.line)}"${dashArray("dash", st.axes.line)}/>`);
          // counts: small bars along the bottom (20 % of the height)
          const hb = Math.abs(c.sy(0) - c.sy(1)) * 0.2;
          ser.forEach((r) => {
            const bw = Math.abs(c.sx(1) - c.sx(0)) / nb / Math.max(1, ser.length);
            const bars = r.pts.map((q) => `<rect x="${r2(c.sx(Math.floor(q[0] * nb) / nb) + r.i * bw)}" y="${r2(c.sy(0) - (hb * q[2]) / maxN)}" width="${r2(bw * 0.9)}" height="${r2((hb * q[2]) / maxN)}" fill="${r.st.color}" fill-opacity="0.3"/>`);
            const pts = r.pts.map((q) => [c.sx(q[0]), c.sy(q[1])]);
            c.shapes?.lines.push(pts);
            c.out.push(`<g data-part="series:${r.i}">${bars.join("")}<path d="${polyline(pts)}" fill="none" stroke="${r.st.color}" stroke-width="${r.st.width}"/>${pts.map((q) => markerPath(r.st.marker, q[0], q[1], r.st.msize, r.st.color)).join("")}</g>`);
          });
        },
      };
    },
  };

  CHARTS.dca = {
    label: "決策曲線", group: "curve",
    prepare(p, tbl, st) {
      const tmax = p.opts?.max ?? 0.6;
      const th = Array.from({ length: 61 }, (_, k) => 0.005 + ((tmax - 0.005) * k) / 60);
      let prev = NaN;
      const ser = seriesOf(p).map((s, i) => {
        const sc = ncol(tbl, s.score), tr = ncol(tbl, s.truth);
        const n = sc.filter((v, k) => isNum(v) && isNum(tr[k])).length || 1;
        prev = tr.filter((v) => v > 0).length / n;
        const nb = th.map((t) => {
          let tp = 0, fp = 0;
          sc.forEach((v, k) => { if (isNum(v) && v >= t) { if (tr[k] > 0) tp++; else fp++; } });
          return tp / n - (fp / n) * (t / (1 - t));
        });
        return { s, i, st: seriesStyle(st, s, i, "line", p), label: labelOf(s, s.score), nb };
      });
      const all = th.map((t) => prev - (1 - prev) * (t / (1 - t)));
      return {
        x: { kind: "linear", values: [0, tmax], fixed: [0, tmax] },
        y: { kind: "linear", values: [-0.05, ...ser.flatMap((r) => r.nb), prev], fixed: [Math.min(-0.05, -prev * 0.15), Math.max(prev, ...ser.flatMap((r) => r.nb)) * 1.08] },
        legend: [...ser.map((r) => ({ label: r.label, kind: "line", ...r.st, marker: "none", part: `series:${r.i}` })), { label: p.opts?.all_label ?? "Treat all", kind: "line", color: "#9a9a9a", dash: "dash", width: st.lines.width * 0.8 }, { label: p.opts?.none_label ?? "Treat none", kind: "line", color: "#000000", width: st.lines.width * 0.6 }],
        legendPos: "upper right",
        points: [],
        draw(c) {
          c.out.push(`<path d="${polyline(th.map((t, k) => [c.sx(t), c.sy(all[k])]))}" fill="none" stroke="#9a9a9a" stroke-width="${r2(st.lines.width * 0.8)}"${dashArray("dash", st.lines.width)}/>`);
          c.out.push(`<path d="M${r2(c.sx(0))} ${r2(c.sy(0))}L${r2(c.sx(tmax))} ${r2(c.sy(0))}" stroke="#000" stroke-width="${r2(st.lines.width * 0.6)}"/>`);
          for (const r of ser) {
            const pts = th.map((t, k) => [c.sx(t), c.sy(r.nb[k])]);
            c.shapes?.lines.push(pts);
            c.out.push(`<g data-part="series:${r.i}"><path d="${polyline(pts)}" fill="none" stroke="${r.st.color}" stroke-width="${r.st.width}"${dashArray(r.st.dash, r.st.width)}/></g>`);
          }
        },
      };
    },
  };

  // ── Bland–Altman, Kaplan–Meier ──────────────────────────────────────────
  CHARTS.bland = {
    label: "Bland–Altman", group: "scatter",
    prepare(p, tbl, st) {
      const s = seriesOf(p)[0] ?? {};
      const a = ncol(tbl, s.y), b = ncol(tbl, s.y2);
      const m = a.map((v, k) => (v + b[k]) / 2), d = a.map((v, k) => v - b[k]);
      const ok = d.map((v, k) => isNum(v) && isNum(m[k]));
      const dd = d.filter((_, k) => ok[k]);
      const md = mean(dd), s2 = sd(dd);
      const lim = [md - 1.96 * s2, md + 1.96 * s2];
      const sst = seriesStyle(st, s, 0, "scatter", p);
      return {
        x: { kind: "linear", values: m },
        y: { kind: "linear", values: d.concat(lim) },
        legend: [],
        points: m.map((x, k) => [x, d[k]]),
        draw(c) {
          const g = [];
          for (let k = 0; k < m.length; k++) if (ok[k]) g.push(markerPath(s.marker ?? "o", c.sx(m[k]), c.sy(d[k]), st.scatter.size, sst.fill, { stroke: 0.3, edge: "#fff" }));
          const [x0, x1] = c.sx.range;
          const f = c.font("legend");
          const line = (v, dash, word) => {
            g.push(`<path d="M${r2(x0)} ${r2(c.sy(v))}L${r2(x1)} ${r2(c.sy(v))}" stroke="${st.colors.axis}" stroke-width="${r2(st.axes.line)}"${dashArray(dash, st.axes.line)}/>`);
            if (p.opts?.limits !== false && !c.inset) g.push(c.T(`${word} ${fmtNum(v, ".2f")}`, f).place(x1 - 1.5, c.sy(v) - 1.5, { anchor: "end" }));
          };
          line(md, "solid", p.opts?.mean_label ?? "Mean");
          line(lim[1], "dash", "+1.96 SD");
          line(lim[0], "dash", "−1.96 SD");
          c.out.push(`<g data-part="series:0">${g.join("")}</g>`);
        },
      };
    },
  };
  CHARTS.km = {
    label: "Kaplan–Meier", group: "curve",
    prepare(p, tbl, st) {
      const t = ncol(tbl, p.x), e = ncol(tbl, p.event);
      const groups = p.group ? col(tbl, p.group).map((v) => String(v ?? "")) : t.map(() => "");
      const names = [...new Set(groups)];
      const pseudo = { series: names.map((g) => (p.series ?? []).find((q) => q.group === g) ?? { group: g }) };
      const curves = names.map((g, i) => {
        const idx = t.map((_, k) => k).filter((k) => groups[k] === g);
        const s = pseudo.series[i];
        return { g, i, s, st: seriesStyle(st, s, i, "line", pseudo), km: kaplanMeier(idx.map((k) => t[k]), idx.map((k) => e[k])), label: s.label || g };
      });
      const tmax = Math.max(...curves.map((c) => c.km.tmax), 1);
      return {
        x: { kind: "linear", values: [0, tmax], zero: true },
        y: { kind: "linear", values: [0, 1], fixed: [0, 1.02] },
        legend: names.length > 1 ? curves.map((c) => ({ label: c.label, kind: "line", ...c.st, marker: "none", part: `group:${c.i}` })) : [],
        points: curves.flatMap((c) => c.km.steps.map((q) => [q.t, q.s])),
        draw(c) {
          for (const cv of curves) {
            const pts = [];
            cv.km.steps.forEach((q, k) => { if (k) pts.push([c.sx(q.t), c.sy(cv.km.steps[k - 1].s)]); pts.push([c.sx(q.t), c.sy(q.s)]); });
            pts.push([c.sx(cv.km.tmax), c.sy(cv.km.steps.at(-1).s)]);
            c.shapes?.lines.push(pts);
            const g = [`<path d="${polyline(pts)}" fill="none" stroke="${cv.st.color}" stroke-width="${cv.st.width}"${dashArray(cv.st.dash, cv.st.width)}/>`];
            if (p.opts?.censor !== false) for (const q of cv.km.cens) g.push(markerPath("+", c.sx(q.t), c.sy(q.s), 3, cv.st.color));
            c.out.push(`<g data-part="group:${cv.i}">${g.join("")}</g>`);
          }
        },
      };
    },
  };

  // ── pie / donut ─────────────────────────────────────────────────────────
  CHARTS.pie = {
    label: "圓餅／甜甜圈", group: "part",
    polar: true,
    prepare(p, tbl, st) {
      const o = p.opts ?? {};
      const s = seriesOf(p)[0] ?? {};
      let slices = col(tbl, p.x).map((v, k) => ({ label: String(v ?? ""), v: ncol(tbl, s.y)[k] })).filter((q) => isNum(q.v) && q.v > 0);
      if (o.sort !== false) slices.sort((a, b) => b.v - a.v);
      const total = slices.reduce((a, q) => a + q.v, 0) || 1;
      // small slices grouped into "Other" below a share (tunable)
      if (isNum(o.other) && o.other > 0) {
        const small = slices.filter((q) => q.v / total < o.other);
        if (small.length > 1) slices = [...slices.filter((q) => q.v / total >= o.other), { label: o.other_label ?? "Other", v: small.reduce((a, q) => a + q.v, 0), other: true }];
      }
      const labels = o.labels ?? (slices.length <= (o.direct_max ?? 6) ? "direct" : "legend");
      const pal = st.colors.palette;
      slices.forEach((q, i) => { const ser = (p.series ?? []).find((x) => x.group === q.label); q.color = ser?.color ?? (q.other ? "#c4c4c4" : pal[i % pal.length]); q.i = i; });
      const pct = (q) => `${((100 * q.v) / total).toFixed(o.digits ?? 0)}%`;
      return {
        polar: true,
        cats: [],
        labelRoom: labels === "direct" ? 34 : 0,
        legend: labels === "legend" ? slices.map((q) => ({ label: `${q.label}${o.percent !== false ? ` (${pct(q)})` : ""}`, kind: "patch", color: q.color, part: `slice:${q.i}` })) : [],
        legendPos: "outside right",
        points: [],
        draw(c) {
          const { cx, cy, R } = c.polar;
          const hole = o.donut ? clamp(isNum(o.donut) ? o.donut : 0.55, 0.2, 0.85) : 0;
          let a0 = ((o.start ?? 90) * Math.PI) / 180;
          const g = [];
          const f = c.font("legend");
          for (const q of slices) {
            const da = (2 * Math.PI * q.v) / total;
            const a1 = a0 - da;
            const P = (r, a) => [cx + r * Math.cos(a), cy - r * Math.sin(a)];
            const large = da > Math.PI ? 1 : 0;
            const [o0, o1] = [P(R, a0), P(R, a1)];
            let d;
            if (hole) { const [i0, i1] = [P(R * hole, a0), P(R * hole, a1)]; d = `M${r2(o0[0])} ${r2(o0[1])}A${r2(R)} ${r2(R)} 0 ${large} 1 ${r2(o1[0])} ${r2(o1[1])}L${r2(i1[0])} ${r2(i1[1])}A${r2(R * hole)} ${r2(R * hole)} 0 ${large} 0 ${r2(i0[0])} ${r2(i0[1])}Z`; }
            else d = slices.length === 1 ? `M${r2(cx - R)} ${r2(cy)}A${r2(R)} ${r2(R)} 0 1 1 ${r2(cx + R)} ${r2(cy)}A${r2(R)} ${r2(R)} 0 1 1 ${r2(cx - R)} ${r2(cy)}Z` : `M${r2(cx)} ${r2(cy)}L${r2(o0[0])} ${r2(o0[1])}A${r2(R)} ${r2(R)} 0 ${large} 1 ${r2(o1[0])} ${r2(o1[1])}Z`;
            const parts = [`<path d="${d}" fill="${q.color}" stroke="#ffffff" stroke-width="${r2(Math.max(0.6, st.axes.line))}"/>`];
            if (labels === "direct") {
              const am = (a0 + a1) / 2;
              const inside = q.v / total > (o.inside_min ?? 0.08);
              if (inside && !hole) {
                const [x, y] = P(R * 0.62, am);
                const fill = luminance(q.color) > 0.4 ? "#000000" : "#ffffff";
                parts.push(c.T(pct(q), { ...f, color: fill }).place(x, y, { anchor: "middle", valign: "middle" }));
                const [lx, ly] = P(R + 3, am);
                parts.push(c.T(q.label, f).place(lx, ly, { anchor: Math.cos(am) >= 0 ? "start" : "end", valign: "middle" }));
              } else {
                const [x0, y0] = P(R * (hole ? 1 : 0.9), am), [x1, y1] = P(R + 4, am);
                parts.push(`<path d="M${r2(x0)} ${r2(y0)}L${r2(x1)} ${r2(y1)}" stroke="#8a8a8a" stroke-width="0.4"/>`);
                parts.push(c.T(`${q.label} ${pct(q)}`, f).place(x1 + (Math.cos(am) >= 0 ? 1 : -1), y1, { anchor: Math.cos(am) >= 0 ? "start" : "end", valign: "middle" }));
              }
            }
            g.push(`<g data-part="slice:${q.i}">${parts.join("")}</g>`);
            a0 = a1;
          }
          if (hole && o.total !== false) {
            const t1 = c.T(o.total_text ?? fmtNum(total, o.total_fmt ?? "d"), { ...c.font("title"), size: c.font("title").size * 1.3, weight: 700 });
            g.push(t1.place(cx, cy, { anchor: "middle", valign: "middle" }));
            if (o.total_label) g.push(c.T(o.total_label, f).place(cx, cy + t1.h * 0.75, { anchor: "middle", valign: "middle" }));
          }
          c.out.push(g.join(""));
        },
      };
    },
  };

  // ── radar (normalised axes, reference series) ───────────────────────────
  CHARTS.radar = {
    label: "雷達圖", group: "part",
    polar: true,
    prepare(p, tbl, st) {
      const o = p.opts ?? {};
      const cats = col(tbl, p.x).map((v) => String(v ?? ""));
      let ser = seriesOf(p).map((s, i) => ({ s, i, st: seriesStyle(st, s, i, "line", p), ys: ncol(tbl, s.y), label: labelOf(s) }));
      if (o.normalize) {
        const lo = cats.map((_, k) => Math.min(...ser.map((r) => r.ys[k]).filter(isNum))), hi = cats.map((_, k) => Math.max(...ser.map((r) => r.ys[k]).filter(isNum)));
        ser = ser.map((r) => ({ ...r, ys: r.ys.map((y, k) => (hi[k] > lo[k] ? 0.2 + (0.8 * (y - lo[k])) / (hi[k] - lo[k]) : 0.6)) }));
      }
      const vals = ser.flatMap((r) => r.ys).filter(isNum);
      const lo = o.normalize ? 0 : isNum(p.yaxis?.min) ? p.yaxis.min : 0;
      const hi = o.normalize ? 1 : isNum(p.yaxis?.max) ? p.yaxis.max : linearTicks(lo, Math.max(...vals, lo + 1), 4).at(-1);
      return {
        polar: true,
        cats, lo, hi,
        legend: ser.map((r) => ({ label: r.label, kind: "line", ...r.st, color: r.s.ref ? "#8a8a8a" : r.st.color, dash: r.s.ref ? "dash" : r.st.dash, part: `series:${r.i}` })),
        points: [],
        draw(c) {
          const { cx, cy, R } = c.polar;
          const ang = (k) => -Math.PI / 2 + (2 * Math.PI * k) / cats.length;
          const rr = (v) => (R * (v - lo)) / (hi - lo || 1);
          const g = [];
          const ticks = linearTicks(lo, hi, 4).filter((v) => v > lo);
          const gc = st.axes.grid === "none" ? "#d0d0d0" : st.axes.grid_color, gw = st.axes.grid_width, ga = st.axes.grid === "none" ? 1 : st.axes.grid_alpha ?? 1;
          for (const tv of ticks) g.push(`<path d="${polyline(cats.map((_, k) => [cx + rr(tv) * Math.cos(ang(k)), cy + rr(tv) * Math.sin(ang(k))]))}Z" fill="none" stroke="${gc}" stroke-opacity="${ga}" stroke-width="${gw}"/>`);
          cats.forEach((name, k) => {
            g.push(`<path d="M${r2(cx)} ${r2(cy)}L${r2(cx + R * Math.cos(ang(k)))} ${r2(cy + R * Math.sin(ang(k)))}" stroke="${gc}" stroke-opacity="${ga}" stroke-width="${gw}"/>`);
            const t = c.T(name, c.font("tick"));
            const a = ang(k), lx = cx + (R + 3) * Math.cos(a), ly = cy + (R + 3) * Math.sin(a);
            g.push(t.place(lx, ly, { anchor: Math.abs(Math.cos(a)) < 0.2 ? "middle" : Math.cos(a) > 0 ? "start" : "end", valign: Math.sin(a) > 0.5 ? "top" : Math.sin(a) < -0.5 ? "bottom" : "middle" }));
          });
          if (!o.normalize) {
            const tf = c.font("tick");
            for (const tv of ticks) g.push(c.T(fmtNum(tv, p.yaxis?.fmt, ticks.step), { ...tf, size: tf.size * 0.85, color: "#666666" }).place(cx + 1.5, cy - rr(tv) - 1, {}));
          }
          c.out.push(`<g data-part="grid">${g.join("")}</g>`);
          for (const r of ser) {
            const pts = cats.map((_, k) => [cx + rr(r.ys[k] ?? lo) * Math.cos(ang(k)), cy + rr(r.ys[k] ?? lo) * Math.sin(ang(k))]);
            const color = r.s.ref ? "#8a8a8a" : r.st.color;
            const fill = o.fill !== false && !r.s.ref ? ` fill="${color}" fill-opacity="0.12"` : ` fill="none"`;
            const m = r.s.ref ? "" : pts.map((q) => markerPath(r.st.marker, q[0], q[1], r.st.msize * 0.8, color)).join("");
            c.out.push(`<g data-part="series:${r.i}"><path d="${polyline(pts)}Z"${fill} stroke="${color}" stroke-width="${r.st.width}"${dashArray(r.s.ref ? "dash" : r.st.dash, r.st.width)}/>${m}</g>`);
          }
        },
      };
    },
  };

  // ── a fresh panel for a chart type over a table ─────────────────────────
  LP.starterPanel = function (type, tbl) {
    const cols = tbl?.columns ?? [];
    const isNumCol = (i) => tbl.rows.some((r) => isNum(num(r[i])));
    const numeric = cols.filter((c, i) => isNumCol(i));
    const text = cols.filter((c, i) => !isNumCol(i));
    const errs = new Set(numeric.filter((c) => /(_sd|_std|_err|_se|_ci)$/i.test(c)));
    const ys = numeric.filter((c) => !errs.has(c) && c !== cols[0]);
    const series = ys.map((y) => { const e = [...errs].find((c) => c.replace(/(_sd|_std|_err|_se|_ci)$/i, "") === y); return e ? { y, err: e, label: y } : { y, label: y }; });
    const p = { type, data: "t1", x: cols[0], series, xaxis: { label: cols[0] ?? "" }, yaxis: { label: "" }, legend: {}, opts: {}, notes: [] };
    const nums = numeric.filter((c) => !errs.has(c));
    switch (type) {
      case "scatter": Object.assign(p, { x: nums[0] ?? cols[0], y: nums[1] ?? nums[0], group: text[0] ?? null, series: [], xaxis: { label: nums[0] ?? "" }, yaxis: { label: nums[1] ?? "" } }); break;
      case "heatmap": Object.assign(p, { x: text[0] ?? cols[0], series: [], xaxis: { label: "" }, yaxis: { label: "" }, opts: { annot: "value" } }); break;
      case "roc": case "pr": case "calib": case "dca": {
        const truth = numeric.find((c) => tbl.rows.every((r) => [0, 1, null, undefined, ""].includes(num(r[cols.indexOf(c)])) || r[cols.indexOf(c)] === 0 || r[cols.indexOf(c)] === 1)) ?? cols[0];
        const scores = numeric.filter((c) => c !== truth);
        Object.assign(p, { x: null, series: scores.map((s) => ({ score: s, truth, label: s })) });
        if (type === "roc") Object.assign(p, { xaxis: { label: "False positive rate" }, yaxis: { label: "True positive rate" }, opts: { auc: true } });
        if (type === "pr") Object.assign(p, { xaxis: { label: "Recall" }, yaxis: { label: "Precision" }, opts: { auc: true } });
        if (type === "calib") Object.assign(p, { xaxis: { label: "Predicted probability" }, yaxis: { label: "Observed fraction" } });
        if (type === "dca") Object.assign(p, { xaxis: { label: "Threshold probability" }, yaxis: { label: "Net benefit" } });
        break;
      }
      case "bland": Object.assign(p, { series: [{ y: nums[0], y2: nums[1] }], xaxis: { label: "Mean of two methods" }, yaxis: { label: "Difference" } }); break;
      case "km": Object.assign(p, { x: nums[0], event: nums[1], group: text[0] ?? null, series: [], xaxis: { label: nums[0] ?? "Time" }, yaxis: { label: "Survival probability" } }); break;
      case "pie": Object.assign(p, { x: text[0] ?? cols[0], series: [{ y: nums[0] }], opts: {} }); break;
      case "box": case "violin": case "hist": case "ridge": case "ecdf":
        Object.assign(p, { series: nums.map((y) => ({ y, label: y })), xaxis: { label: "" } }); break;
      case "dumbbell": Object.assign(p, { series: series.slice(0, 2), xaxis: { label: "" }, yaxis: { label: "" } }); break;
      case "contour": case "surface": case "scatter3d":
        Object.assign(p, { x: nums[0], y: nums[1], z: nums[2], group: text[0] ?? null, series: [], xaxis: { label: nums[0] ?? "" }, yaxis: { label: nums[1] ?? "" }, zaxis: { label: nums[2] ?? "" } }); break;
    }
    return p;
  };
  LP.starter = function (type, tbl, style) {
    return { lore_plot: 1, style: style ?? "clean", set: {}, size: null, grid: { cols: 1, rows: 1, gap: 5, labels: "none" }, data: { t1: tbl }, panels: [LP.starterPanel(type, tbl)], caption: "" };
  };
})();
