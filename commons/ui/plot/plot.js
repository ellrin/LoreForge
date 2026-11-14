// LorePlot: a figure spec (docs/plot.md §3) → one SVG, in the browser.
//
//   await LorePlot.render(spec, { styles, macros })  → { svg, w, h, warnings }
//   LorePlot.resolveStyle(id, styles, set)            → the style object in force
//   LorePlot.TYPES                                    → chart types (id, label)
//
// Units: the SVG's user unit is the point (1/72 in); width/height are written in mm.
// Text (plain or with $…$ maths) is laid out once in a hidden element with KaTeX and
// flattened into <text>, <rect> and KaTeX's own <svg> paths, so the result has no
// HTML in it: the same SVG is the editor view, the PDF (Chrome prints it) and the PNG.
// Colours and fonts come only from the figure's style, never from the app theme.
(() => {
  "use strict";
  const PT = 72 / 25.4; // pt per mm
  const S = 8; // measuring scale (sub-pixel precision for small type)
  const script = document.currentScript?.src ?? "";
  const LorePlot = {
    base: script ? script.replace(/[^/]*$/, "") : "",
    katexBase: null, // default: <base>katex/
  };

  // ── small helpers ──────────────────────────────────────────────────────
  const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const r2 = (v) => Math.round(v * 100) / 100;
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  const clone = (o) => (o == null ? o : JSON.parse(JSON.stringify(o)));
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  function num(v) {
    if (isNum(v)) return v;
    if (typeof v !== "string") return NaN;
    const t = v.trim().replace(/[−–]/g, "-").replace(/,/g, "");
    if (t === "") return NaN;
    const pct = t.endsWith("%");
    const n = Number(pct ? t.slice(0, -1) : t);
    return Number.isFinite(n) ? (pct ? n / 100 : n) : NaN;
  }
  const isPlainObj = (o) => o && typeof o === "object" && !Array.isArray(o);
  function merge(a, b) {
    const out = clone(a) ?? {};
    for (const [k, v] of Object.entries(b ?? {})) out[k] = isPlainObj(v) && isPlainObj(out[k]) ? merge(out[k], v) : clone(v);
    return out;
  }
  function setPath(o, path, v) {
    const ks = path.split(".");
    let cur = o;
    for (const k of ks.slice(0, -1)) cur = isPlainObj(cur[k]) ? cur[k] : (cur[k] = {});
    cur[ks.at(-1)] = v;
  }
  function getPath(o, path) {
    return path.split(".").reduce((c, k) => (c == null ? undefined : c[k]), o);
  }

  // ── styles ─────────────────────────────────────────────────────────────
  // The built-in fallback, used when no style list is given (and as the root of every style).
  const ROOT_STYLE = {
    id: "root",
    size: { w: 85, h: 60, widths: [85, 120, 175] },
    font: {
      kind: "serif",
      family: ["STIX Two Text", "TeX Gyre Termes", "Liberation Serif", "Tinos", "serif"],
      size: 8, title: 8, label: 8, tick: 7, legend: 7, panel: 9, panel_weight: "bold", title_weight: "normal", math: "katex", // maths: TeX fonts ("text": letters in the text font)
    },
    // grid: none | y | x | both; minor lines (grid_minor) at grid_density subdivisions; always behind the data
    axes: { line: 0.6, tick_len: 3, tick_dir: "out", spines: "lb", grid: "none", grid_dash: "solid", grid_color: "#000000", grid_alpha: 0.2, grid_width: 0.4, grid_minor: false, grid_density: 2, pad: 3, padding: 0.05, max_ticks: 7 },
    lines: { width: 1.2, marker: 3.5, markers: ["o", "s", "^", "D", "v", "x"], dashes: ["solid"] },
    bars: { width: 0.8, gap: 0.04, edge: 0, edge_color: "", cap: 2, err_width: 0.6, alpha: 1, break_mark: false },
    box: { alpha: 0.6 },
    inset: { frame: 0.4, connector: 0.3, connector_color: "#c4c4c4" },
    scatter: { size: 3, alpha: 0.8 },
    // legend: inside the emptiest corner (85 % white backing), else one row above (≤ top_max entries), else outside right
    legend: { frame: false, pos: "best", size: 7, backing: 0.85, top_max: 6, frame_width: 0.3, frame_color: "#c8c8c8" },
    colors: {
      palette: ["#0072B2", "#E69F00", "#009E73", "#D55E00", "#CC79A7", "#56B4E9", "#F0E442", "#000000"],
      lines: [], cmap: "viridis", diverging: "rdbu", text: "#000000", axis: "#000000", background: "#ffffff",
      // "ours" (series.ours): the emphasis colour; the other series muted toward grey by `mute` (0–1)
      emphasis: "", mute: 0.45, other: "#b0b0b0",
      // one series (nothing marked "ours"): a neutral mid-tone, never the emphasis colour
      single: "#6E86A0",
    },
  };
  /** The style `id` from `styles` (a list or a map), its `base` chain applied, then `set`. */
  LorePlot.resolveStyle = function (id, styles, set) {
    const list = Array.isArray(styles) ? styles : Object.values(styles ?? {});
    const find = (i) => list.find((s) => s.id === i);
    const chain = [];
    let cur = find(id) ?? find("clean") ?? null;
    for (let guard = 0; cur && guard < 8; guard++) {
      chain.unshift(cur);
      cur = cur.base ? find(cur.base) : null;
    }
    let st = clone(ROOT_STYLE);
    for (const s of chain) st = merge(st, s);
    for (const [k, v] of Object.entries(set ?? {})) setPath(st, k, clone(v));
    st.id = chain.at(-1)?.id ?? "root";
    return st;
  };
  LorePlot.ROOT_STYLE = ROOT_STYLE;
  LorePlot.getPath = getPath;
  LorePlot.setPath = setPath;
  LorePlot.merge = merge;
  const fontCss = (fam) => (Array.isArray(fam) ? fam : [fam]).map((f) => (/^(serif|sans-serif|monospace)$/.test(f) ? f : `"${f}"`)).join(", ");

  // ── colour maps ────────────────────────────────────────────────────────
  const CMAPS = {
    viridis: ["#440154", "#482878", "#3e4989", "#31688e", "#26828e", "#1f9e89", "#35b779", "#6ece58", "#b5de2b", "#fde725"],
    cividis: ["#00224e", "#123570", "#3b496c", "#575d6d", "#707173", "#8a8678", "#a59c74", "#c3b369", "#e1cc55", "#fee838"],
    magma: ["#000004", "#180f3d", "#440f76", "#721f81", "#9e2f7f", "#cd4071", "#f1605d", "#fd9668", "#feca8d", "#fcfdbf"],
    blues: ["#f7fbff", "#deebf7", "#c6dbef", "#9ecae1", "#6baed6", "#4292c6", "#2171b5", "#08519c", "#08306b"],
    purples: ["#fcfbfd", "#efedf5", "#dadaeb", "#bcbddc", "#9e9ac8", "#807dba", "#6a51a3", "#54278f", "#3f007d"],
    greys: ["#ffffff", "#f0f0f0", "#d9d9d9", "#bdbdbd", "#969696", "#737373", "#525252", "#252525", "#000000"],
    rdbu: ["#67001f", "#b2182b", "#d6604d", "#f4a582", "#fddbc7", "#f7f7f7", "#d1e5f0", "#92c5de", "#4393c3", "#2166ac", "#053061"],
  };
  LorePlot.CMAPS = Object.keys(CMAPS);
  const hex = (c) => {
    const m = /^#?([0-9a-f]{6})$/i.exec(c);
    if (!m) return [0, 0, 0];
    const n = parseInt(m[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  };
  const toHex = (rgb) => "#" + rgb.map((v) => clamp(Math.round(v), 0, 255).toString(16).padStart(2, "0")).join("");
  function cmap(name, t) {
    const stops = CMAPS[name] ?? CMAPS.viridis;
    t = clamp(isNum(t) ? t : 0, 0, 1) * (stops.length - 1);
    const i = Math.min(stops.length - 2, Math.floor(t));
    const a = hex(stops[i]), b = hex(stops[i + 1]), f = t - i;
    return toHex(a.map((v, k) => v + (b[k] - v) * f));
  }
  const luminance = (c) => {
    const [r, g, b] = hex(c).map((v) => {
      v /= 255;
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const darker = (c, k = 0.35) => toHex(hex(c).map((v) => v * (1 - k)));

  // ── numbers ────────────────────────────────────────────────────────────
  function niceStep(span, count) {
    if (!(span > 0)) return 1;
    const raw = span / Math.max(1, count);
    const p = 10 ** Math.floor(Math.log10(raw));
    const e = raw / p;
    return (e >= 7.5 ? 10 : e >= 3.5 ? 5 : e >= 1.5 ? 2 : 1) * p;
  }
  function linearTicks(lo, hi, count) {
    const step = niceStep(hi - lo, count);
    const out = [];
    const start = Math.ceil(lo / step - 1e-9) * step;
    for (let v = start; v <= hi + step * 1e-9; v += step) out.push(Math.abs(v) < step * 1e-9 ? 0 : v);
    out.step = step;
    return out;
  }
  function decimalsFor(step) {
    if (!(step > 0)) return 2;
    const d = -Math.floor(Math.log10(step) + 1e-9);
    // 0.25, 2.5: one more digit when the step is not a single significant digit
    const k = step / 10 ** -d;
    return Math.max(0, d + (Math.abs(k - Math.round(k)) > 1e-6 ? 1 : 0));
  }
  /** A number as tick / value text: fmt "" (auto from step), ".2f", "%", ".1%", "e". */
  function fmtNum(v, fmt, step) {
    if (!isNum(v)) return "";
    fmt = fmt ?? "";
    let s;
    let m;
    if ((m = /^\.(\d)f$/.exec(fmt))) s = v.toFixed(+m[1]);
    else if ((m = /^(?:\.(\d))?%$/.exec(fmt))) s = (v * 100).toFixed(m[1] != null ? +m[1] : step ? Math.max(0, decimalsFor(step * 100)) : 0) + "%";
    else if (fmt === "e") {
      if (v === 0) return "0";
      const ex = Math.floor(Math.log10(Math.abs(v)));
      const mant = r2(v / 10 ** ex);
      return mant === 1 ? `$10^{${ex}}$` : `$${mant}\\times10^{${ex}}$`;
    } else if (fmt === "d") s = String(Math.round(v));
    else if (step) s = v.toFixed(Math.min(8, decimalsFor(step)));
    else s = String(+v.toPrecision(4));
    return s.replace(/^-(?=\d)/, "−");
  }
  LorePlot.fmtNum = fmtNum;
  const quantile = (sorted, q) => {
    if (!sorted.length) return NaN;
    const p = (sorted.length - 1) * q, i = Math.floor(p), f = p - i;
    return sorted[i] + (sorted[Math.min(i + 1, sorted.length - 1)] - sorted[i]) * f;
  };
  const mean = (a) => a.reduce((s, v) => s + v, 0) / (a.length || 1);
  const sd = (a) => {
    if (a.length < 2) return 0;
    const m = mean(a);
    return Math.sqrt(a.reduce((s, v) => s + (v - m) ** 2, 0) / (a.length - 1));
  };
  function kde(values, at) {
    const n = values.length;
    const s = sd(values) || 1;
    const sorted = [...values].sort((a, b) => a - b);
    const iqr = quantile(sorted, 0.75) - quantile(sorted, 0.25);
    const h = 0.9 * Math.min(s, iqr / 1.34 || s) * n ** -0.2 || 1;
    return at.map((x) => values.reduce((acc, v) => acc + Math.exp(-0.5 * ((x - v) / h) ** 2), 0) / (n * h * Math.sqrt(2 * Math.PI)));
  }
  function hull(pts) {
    const p = [...pts].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    if (p.length < 3) return p;
    const cross = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    const lo = [], up = [];
    for (const q of p) { while (lo.length >= 2 && cross(lo.at(-2), lo.at(-1), q) <= 0) lo.pop(); lo.push(q); }
    for (const q of [...p].reverse()) { while (up.length >= 2 && cross(up.at(-2), up.at(-1), q) <= 0) up.pop(); up.push(q); }
    return lo.slice(0, -1).concat(up.slice(0, -1));
  }
  /** 95 % covariance ellipse of 2-D points: centre, radii, angle (radians), in data units. */
  function ellipse(pts) {
    const n = pts.length;
    if (n < 3) return null;
    const mx = mean(pts.map((p) => p[0])), my = mean(pts.map((p) => p[1]));
    let sxx = 0, syy = 0, sxy = 0;
    for (const [x, y] of pts) { sxx += (x - mx) ** 2; syy += (y - my) ** 2; sxy += (x - mx) * (y - my); }
    sxx /= n - 1; syy /= n - 1; sxy /= n - 1;
    const tr = sxx + syy, det = sxx * syy - sxy * sxy;
    const l1 = tr / 2 + Math.sqrt(Math.max(0, tr * tr / 4 - det)), l2 = tr / 2 - Math.sqrt(Math.max(0, tr * tr / 4 - det));
    const ang = Math.abs(sxy) < 1e-12 ? (sxx >= syy ? 0 : Math.PI / 2) : Math.atan2(l1 - sxx, sxy);
    const k = Math.sqrt(5.991); // chi² 95 %, 2 dof
    return { cx: mx, cy: my, rx: k * Math.sqrt(Math.max(l1, 0)), ry: k * Math.sqrt(Math.max(l2, 0)), ang };
  }
  const trapz = (xs, ys) => {
    let a = 0;
    for (let i = 1; i < xs.length; i++) a += ((xs[i] - xs[i - 1]) * (ys[i] + ys[i - 1])) / 2;
    return a;
  };
  /** ROC points (fpr, tpr) from scores and 0/1 truth, plus AUC. */
  function rocFrom(score, truth) {
    const pairs = score.map((s, i) => [s, truth[i] > 0 ? 1 : 0]).filter((p) => isNum(p[0])).sort((a, b) => b[0] - a[0]);
    const P = pairs.filter((p) => p[1]).length, N = pairs.length - P;
    const xs = [0], ys = [0];
    let tp = 0, fp = 0;
    for (let i = 0; i < pairs.length; i++) {
      if (pairs[i][1]) tp++; else fp++;
      if (i === pairs.length - 1 || pairs[i + 1][0] !== pairs[i][0]) { xs.push(fp / (N || 1)); ys.push(tp / (P || 1)); }
    }
    return { xs, ys, auc: trapz(xs, ys) };
  }
  function prFrom(score, truth) {
    const pairs = score.map((s, i) => [s, truth[i] > 0 ? 1 : 0]).filter((p) => isNum(p[0])).sort((a, b) => b[0] - a[0]);
    const P = pairs.filter((p) => p[1]).length;
    const xs = [0], ys = [1];
    let tp = 0, fp = 0, ap = 0, lastR = 0;
    for (let i = 0; i < pairs.length; i++) {
      if (pairs[i][1]) tp++; else fp++;
      if (i === pairs.length - 1 || pairs[i + 1][0] !== pairs[i][0]) {
        const r = tp / (P || 1), p = tp / (tp + fp);
        ap += (r - lastR) * p;
        lastR = r;
        xs.push(r); ys.push(p);
      }
    }
    return { xs, ys, auc: ap };
  }
  /** Kaplan–Meier steps: [{t, s}] and censor marks. */
  function kaplanMeier(times, events) {
    const d = times.map((t, i) => [t, events[i] > 0 ? 1 : 0]).filter((p) => isNum(p[0])).sort((a, b) => a[0] - b[0]);
    let atRisk = d.length, s = 1;
    const steps = [{ t: 0, s: 1 }], cens = [];
    for (let i = 0; i < d.length;) {
      const t = d[i][0];
      let ev = 0, n = 0;
      while (i < d.length && d[i][0] === t) { ev += d[i][1]; n++; i++; }
      if (ev) { s *= 1 - ev / atRisk; steps.push({ t, s }); }
      if (n - ev) cens.push({ t, s });
      atRisk -= n;
    }
    return { steps, cens, tmax: d.length ? d.at(-1)[0] : 1 };
  }
  LorePlot.stats = { quantile, mean, sd, kde, hull, ellipse, trapz, rocFrom, prFrom, kaplanMeier, linearTicks, niceStep };

  // ── data ───────────────────────────────────────────────────────────────
  function table(spec, panel) {
    const t = spec.data?.[panel.data] ?? Object.values(spec.data ?? {})[0];
    return t ?? { columns: [], rows: [] };
  }
  function col(t, name) {
    const i = t.columns.indexOf(name);
    return i < 0 ? [] : t.rows.map((r) => r[i]);
  }
  const ncol = (t, name) => col(t, name).map(num);

  // ── text: KaTeX laid out, then flattened into SVG (commons/ui/mathsvg.js, shared with the flow editor) ──
  const MS = window.LoreMathSvg;
  if (!MS) throw new Error("mathsvg.js must load before plot.js");
  const { ensureHost, hasMath, htmlOf, measure } = MS;
  const loadKatex = () => MS.loadKatex(LorePlot.katexBase ?? LorePlot.base + "katex/");

  /** Every string in the spec (not the data) — laid out once so their fonts load before measuring. */
  function strings(spec) {
    const out = new Set();
    const walk = (o, k) => {
      if (k === "data") return;
      if (typeof o === "string") out.add(o);
      else if (Array.isArray(o)) o.forEach((v) => walk(v));
      else if (o && typeof o === "object") for (const [kk, v] of Object.entries(o)) walk(v, kk);
    };
    walk(spec);
    return [...out];
  }
  async function warm(spec, st, macros) {
    // ticks on log / symlog axes and some labels are maths too: load KaTeX whenever it may be needed
    if (strings(spec).some(hasMath) || /"(log|symlog)"|"log":true|"fmt":"e"|"sim":/.test(JSON.stringify(spec.panels ?? []))) await loadKatex();
    const h = ensureHost();
    const fam = fontCss(st.font.family);
    const probe = document.createElement("span");
    probe.className = "lp-t" + (st.font.math === "text" ? " lp-mtext" : "");
    probe.style.cssText = `font-family:${fam};--lp-font:${fam};font-size:20px`;
    probe.innerHTML = strings(spec).map((s) => htmlOf(s, macros)).join(" ") + " 0123456789.−% <b>ab</b> <i>xy</i>";
    h.append(probe);
    try {
      await Promise.all(
        [400, 700].flatMap((w) => ["normal", "italic"].map((it) => document.fonts.load(`${it} ${w} 20px ${fam}`).catch(() => null))),
      );
      await document.fonts.ready;
    } catch {}
    probe.remove();
  }
  LorePlot.measure = measure;
  LorePlot.loadKatex = loadKatex;

  // ── data files: palettes and formats (docs/plot.md §4, §3) ────────────
  let DATA = null;
  /** palettes.json and formats.json next to this file (cached). */
  LorePlot.data = async function () {
    if (DATA) return DATA;
    try {
      const [palettes, formats] = await Promise.all(["palettes.json", "formats.json"].map((f) => fetch(LorePlot.base + f).then((r) => (r.ok ? r.json() : {}))));
      DATA = { palettes: palettes ?? {}, formats: formats ?? {} };
    } catch {
      DATA = { palettes: {}, formats: {} };
    }
    return DATA;
  };
  LorePlot.setData = (d) => { DATA = d; };
  /** resolveStyle with palette names (palettes.json) turned into colour lists. */
  LorePlot.styleFor = (id, styles, set) => {
    const st = LorePlot.resolveStyle(id, styles, set);
    st.colors.palette = colorsOf(st.colors.palette, ROOT_STYLE.colors.palette);
    st.colors.lines = colorsOf(st.colors.lines, []);
    return st;
  };
  LorePlot.dataSync = () => DATA;
  /** A palette given as a list or as a name in palettes.json. */
  function colorsOf(v, fallback) {
    if (Array.isArray(v) && v.length) return v;
    if (typeof v === "string") return DATA?.palettes?.[v]?.colors ?? fallback;
    return fallback;
  }

  // ── scales ─────────────────────────────────────────────────────────────
  // Continuous scales: linear, log (base 10), symlog, stretch (piecewise segments with weights;
  // weight 0 = a broken axis). f(v) → position; f.ticks(n), f.minor(ticks, density), f.marks (break
  // positions in data units), f.gaps (data intervals cut out).
  const GAP = 0.035; // share of the axis a broken (weight 0) segment takes
  function contScale(conf, d0, d1, r0, r1) {
    conf = conf ?? {};
    let kind = conf.scale ?? (conf.log ? "log" : "linear");
    const rev = d0 > d1;
    const lo = Math.min(d0, d1), hi = Math.max(d0, d1);
    let fwd, segs = null;
    if (kind === "log") {
      const a = Math.log10(Math.max(lo, 1e-12)), b = Math.log10(Math.max(hi, 1e-12));
      fwd = (v) => (v > 0 ? (Math.log10(v) - a) / (b - a || 1) : NaN);
    } else if (kind === "symlog") {
      const c = conf.linthresh > 0 ? conf.linthresh : 1;
      const t = (v) => Math.sign(v) * Math.log10(1 + Math.abs(v) / c);
      const a = t(lo), b = t(hi);
      fwd = (v) => (t(v) - a) / (b - a || 1);
    } else if (kind === "stretch" && Array.isArray(conf.segments) && conf.segments.length) {
      // the given segments, the rest of [lo, hi] at weight 1
      const given = conf.segments.map((s) => [num(s[0]), num(s[1]), Math.max(0, num(s[2] ?? 1))]).filter((s) => isNum(s[0]) && isNum(s[1]) && s[1] > s[0]).sort((a, b) => a[0] - b[0]);
      segs = [];
      let at = lo;
      for (const [a, b, w] of given) {
        const A = Math.max(a, lo), B = Math.min(b, hi);
        if (B <= A) continue;
        if (A > at) segs.push([at, A, 1]);
        segs.push([A, B, isNum(w) ? w : 1]);
        at = B;
      }
      if (at < hi) segs.push([at, hi, 1]);
      const len = (s) => (s[2] > 0 ? (s[1] - s[0]) * s[2] : 0);
      const span = segs.reduce((t, s) => t + len(s), 0) || 1;
      const gaps = segs.filter((s) => s[2] === 0).length;
      const live = 1 - gaps * GAP;
      fwd = (v) => {
        let u = 0;
        for (const s of segs) {
          if (v <= s[0]) break;
          const part = Math.min(v, s[1]) - s[0];
          u += s[2] > 0 ? ((part * s[2]) / span) * live : (part / (s[1] - s[0])) * GAP;
          if (v <= s[1]) break;
        }
        return u;
      };
    } else {
      kind = "linear";
      fwd = (v) => (v - lo) / (hi - lo || 1);
    }
    const f = (v) => r0 + (rev ? 1 - fwd(v) : fwd(v)) * (r1 - r0);
    Object.assign(f, { kind, domain: [d0, d1], range: [r0, r1] });
    f.segs = segs;
    f.gaps = segs ? segs.filter((s) => s[2] === 0).map((s) => [s[0], s[1]]) : [];
    // break marks: every boundary where the weight changes (and both ends of a gap)
    f.marks = segs ? segs.slice(1).filter((s, k) => s[2] !== segs[k][2]).map((s) => s[0]) : [];
    f.ticks = (count) => {
      if (kind === "log") {
        const out = [];
        for (let e = Math.floor(Math.log10(Math.max(lo, 1e-12))); e <= Math.ceil(Math.log10(hi)); e++) {
          const v = 10 ** e;
          if (v >= lo * 0.999 && v <= hi * 1.001) out.push(v);
        }
        out.step = 0;
        out.log = true;
        return out;
      }
      if (kind === "symlog") {
        const c = conf.linthresh > 0 ? conf.linthresh : 1;
        const out = [0];
        for (let e = Math.floor(Math.log10(c)); e <= 12; e++) {
          const v = 10 ** e;
          if (v <= hi * 1.001) out.push(v);
          if (-v >= lo * 1.001) out.unshift(-v);
          if (v > Math.max(Math.abs(lo), Math.abs(hi))) break;
        }
        const t = out.filter((v) => v >= lo - 1e-12 && v <= hi + 1e-12);
        t.step = 0;
        t.log = true;
        return t;
      }
      if (segs) {
        const out = [];
        const span = Math.abs(r1 - r0);
        let step = Infinity;
        for (const s of segs) {
          if (s[2] === 0) continue;
          const px = Math.abs(f(s[1]) - f(s[0]));
          const n = Math.max(2, Math.round((count * px) / (span || 1)));
          const t = linearTicks(s[0], s[1], n);
          step = Math.min(step, t.step);
          for (const v of t) if (!out.some((w) => Math.abs(w - v) < t.step * 1e-6)) out.push(v);
        }
        out.sort((a, b) => a - b);
        out.step = isFinite(step) ? step : 0;
        return out;
      }
      return linearTicks(lo, hi, count);
    };
    f.minor = (ticks, density) => {
      const out = [];
      if (kind === "log") {
        for (const v of ticks) for (let m = 2; m < 10; m++) if (v * m <= hi && v * m >= lo) out.push(v * m);
        return out;
      }
      const d = Math.max(1, Math.round(density ?? 2));
      if (!ticks.step || d < 2) return out;
      const st = ticks.step / d;
      for (let v = Math.ceil(lo / st - 1e-9) * st; v <= hi + 1e-9; v += st) if (!ticks.some((t) => Math.abs(t - v) < st * 1e-3)) out.push(v);
      return out;
    };
    return f;
  }
  function bandScale(n, r0, r1, pad = 0) {
    const step = (r1 - r0) / Math.max(1, n);
    const f = (i) => r0 + step * (i + 0.5);
    Object.assign(f, { kind: "band", n, step, range: [r0, r1], domain: [-0.5, n - 0.5], pad, marks: [], gaps: [] });
    return f;
  }
  const linScale = (d0, d1, r0, r1) => contScale({}, d0, d1, r0, r1);
  /** Numeric domain: data extent, padded, axis min/max win. */
  function domainOf(values, ax, o = {}) {
    const log = ax?.log || ax?.scale === "log";
    let v = values.filter(isNum);
    if (log) v = v.filter((x) => x > 0);
    let lo = v.length ? Math.min(...v) : 0, hi = v.length ? Math.max(...v) : 1;
    if (o.zero) { lo = Math.min(lo, 0); hi = Math.max(hi, 0); }
    if (lo === hi) { lo -= 0.5; hi += 0.5; }
    if (log) {
      lo = 10 ** Math.floor(Math.log10(lo));
      hi = 10 ** Math.ceil(Math.log10(hi));
    } else if (o.fixed) {
      [lo, hi] = o.fixed;
    } else {
      const pad = (hi - lo) * (o.pad ?? 0.05);
      if (!(o.zero && lo === 0)) lo -= pad;
      if (!(o.zero && hi === 0)) hi += pad;
      if (o.nice) {
        const st = niceStep(hi - lo, 5);
        if (!(o.zero && lo === 0)) lo = Math.floor(lo / st + 1e-9) * st;
        hi = Math.ceil(hi / st - 1e-9) * st;
      }
    }
    if (isNum(ax?.min)) lo = ax.min;
    if (isNum(ax?.max)) hi = ax.max;
    if (ax?.reverse) [lo, hi] = [hi, lo];
    return [lo, hi];
  }

  // ── markers and dashes ─────────────────────────────────────────────────
  function markerPath(m, x, y, d, color, o = {}) {
    if (!m || m === "none" || !(d > 0)) return "";
    const r = d / 2;
    const fill = o.hollow ? "#fff" : color;
    const sw = o.stroke ?? Math.max(0.4, d * 0.12);
    const st = ` fill="${fill}" stroke="${o.edge ?? color}" stroke-width="${r2(sw)}"`;
    const P = (pts) => `<path d="M${pts.map((p) => `${r2(x + p[0])} ${r2(y + p[1])}`).join("L")}Z"${st}/>`;
    switch (m) {
      case "s": return `<rect x="${r2(x - r * 0.9)}" y="${r2(y - r * 0.9)}" width="${r2(r * 1.8)}" height="${r2(r * 1.8)}"${st}/>`;
      case "^": return P([[0, -r * 1.15], [r * 1.05, r * 0.75], [-r * 1.05, r * 0.75]]);
      case "v": return P([[0, r * 1.15], [r * 1.05, -r * 0.75], [-r * 1.05, -r * 0.75]]);
      case "D": return P([[0, -r * 1.2], [r * 0.9, 0], [0, r * 1.2], [-r * 0.9, 0]]);
      case "x": return `<path d="M${r2(x - r)} ${r2(y - r)}L${r2(x + r)} ${r2(y + r)}M${r2(x - r)} ${r2(y + r)}L${r2(x + r)} ${r2(y - r)}" stroke="${color}" stroke-width="${r2(Math.max(0.6, d * 0.18))}" fill="none"/>`;
      case "+": return `<path d="M${r2(x - r)} ${r2(y)}L${r2(x + r)} ${r2(y)}M${r2(x)} ${r2(y - r)}L${r2(x)} ${r2(y + r)}" stroke="${color}" stroke-width="${r2(Math.max(0.6, d * 0.18))}" fill="none"/>`;
      default: return `<circle cx="${r2(x)}" cy="${r2(y)}" r="${r2(r)}"${st}/>`;
    }
  }
  function dashArray(d, w) {
    const k = Math.max(w, 0.5);
    switch (d) {
      case "dash": return ` stroke-dasharray="${r2(k * 3.7)} ${r2(k * 1.6)}"`;
      case "dot": return ` stroke-dasharray="${r2(k)} ${r2(k * 1.65)}"`;
      case "dashdot": return ` stroke-dasharray="${r2(k * 6.4)} ${r2(k * 1.6)} ${r2(k)} ${r2(k * 1.6)}"`;
      default: return "";
    }
  }
  const polyline = (pts) => pts.filter((p) => isNum(p[0]) && isNum(p[1])).map((p, i) => `${i ? "L" : "M"}${r2(p[0])} ${r2(p[1])}`).join("");

  // helpers the chart files use
  LorePlot._ = { r2, esc, isNum, num, clamp, clone, col, ncol, table, markerPath, dashArray, polyline, cmap, luminance, darker, hex, toHex, fmtNum, linearTicks, niceStep, quantile, mean, sd, kde, hull, ellipse, trapz, rocFrom, prFrom, kaplanMeier, colorsOf, contScale, bandScale, linScale, ROOT_STYLE, PT };
  const CHARTS = (LorePlot.CHARTS = LorePlot.CHARTS ?? {});
  Object.defineProperty(LorePlot, "TYPES", { get: () => Object.entries(CHARTS).map(([id, c]) => ({ id, label: c.label, group: c.group ?? "" })) });

  // ── axes, legend, notes ────────────────────────────────────────────────
  function axisTicks(ax, scale, conf, count) {
    if (scale.kind === "band") return ax.cats.map((c, i) => ({ v: i, text: c }));
    if (Array.isArray(conf?.ticks) && conf.ticks.length) {
      const nums = conf.ticks.map(num).filter(isNum);
      const step = nums.length > 1 ? Math.abs(nums[1] - nums[0]) : 0;
      const out = nums.map((v) => ({ v, text: fmtNum(v, conf.fmt, step) }));
      out.step = step;
      return out;
    }
    const t = scale.ticks(count);
    const out = t.map((v) => {
      if (t.log && (!conf?.fmt || conf.fmt === "e")) {
        if (v === 0) return { v, text: "0" };
        const e = Math.round(Math.log10(Math.abs(v)));
        return { v, text: `$${v < 0 ? "-" : ""}10^{${e}}$` };
      }
      return { v, text: fmtNum(v, conf?.fmt, t.step) };
    });
    out.step = t.step;
    out.raw = t;
    return out;
  }
  const withUnit = (ax) => (ax?.label ? ax.label + (ax.unit ? ` (${ax.unit})` : "") : "");

  /** Legend block of `items` in font `f`: { w, h, draw(x, y, inside) }. */
  function legendBlock(items, f, st, conf, T) {
    const cols = Math.max(1, Math.min(items.length, conf?.cols ?? 1));
    const fs = f.size;
    const handle = fs * 1.9, gap = fs * 0.45, pad = fs * 0.45, rowGap = fs * 0.32, colGap = fs * 1.1;
    const texts = items.map((it) => T(it.label, f));
    const rows = Math.ceil(items.length / cols);
    const colW = Array.from({ length: cols }, (_, ci) => Math.max(0, ...texts.filter((_, k) => k % cols === ci).map((t) => t.w)));
    const rowH = Math.max(fs, ...texts.map((t) => t.h));
    const title = conf?.title ? T(conf.title, { ...f, weight: 700 }) : null;
    const w = pad * 2 + colW.reduce((a, cw) => a + handle + gap + cw, 0) + colGap * (cols - 1);
    const h = pad * 2 + rows * rowH + (rows - 1) * rowGap + (title ? title.h + rowGap : 0);
    const frame = conf?.frame ?? st.legend.frame;
    return {
      w: Math.max(w, title ? title.w + pad * 2 : 0),
      h,
      draw(x, y, inside) {
        const g = [];
        const backing = st.legend.backing ?? 0.85;
        const light = frame === "light";
        if (frame) g.push(`<rect x="${r2(x)}" y="${r2(y)}" width="${r2(this.w)}" height="${r2(h)}" fill="#ffffff" fill-opacity="${backing}" stroke="${light ? st.legend.frame_color ?? "#c8c8c8" : st.colors.axis}" stroke-width="${r2(light ? st.legend.frame_width ?? 0.3 : Math.max(0.4, st.axes.line * 0.8))}" rx="${frame === "round" ? r2(fs * 0.3) : 0}"/>`);
        else if (inside && backing > 0) g.push(`<rect x="${r2(x)}" y="${r2(y)}" width="${r2(this.w)}" height="${r2(h)}" fill="#ffffff" fill-opacity="${backing}"/>`);
        let y0 = y + pad;
        if (title) { g.push(title.place(x + pad, y0 + title.asc)); y0 += title.h + rowGap; }
        items.forEach((it, k) => {
          const ci = k % cols, ri = Math.floor(k / cols);
          const cx = x + pad + colW.slice(0, ci).reduce((a, cw) => a + handle + gap + cw + colGap, 0);
          const cy = y0 + ri * (rowH + rowGap) + rowH / 2;
          const sw = [];
          if (it.kind === "patch") sw.push(`<rect x="${r2(cx + handle * 0.15)}" y="${r2(cy - fs * 0.35)}" width="${r2(handle * 0.7)}" height="${r2(fs * 0.7)}" fill="${it.color}"${it.alpha != null && it.alpha < 1 ? ` fill-opacity="${it.alpha}"` : ""}${st.bars.edge > 0 ? ` stroke="${st.bars.edge_color || "#000"}" stroke-width="${st.bars.edge}"` : ""}/>`);
          else if (it.kind === "marker") sw.push(markerPath(it.marker ?? "o", cx + handle / 2, cy, it.size ?? Math.max(2.5, st.scatter.size * 1.1), it.color, { stroke: 0.3, edge: "#fff" }));
          else {
            sw.push(`<path d="M${r2(cx)} ${r2(cy)}L${r2(cx + handle)} ${r2(cy)}" stroke="${it.color}" stroke-width="${it.width ?? st.lines.width}"${dashArray(it.dash, it.width ?? 1)}/>`);
            if (it.marker && it.marker !== "none") sw.push(markerPath(it.marker, cx + handle / 2, cy, it.msize ?? st.lines.marker, it.color, { stroke: (it.width ?? 1) * 0.6 }));
          }
          g.push(`<g data-part="${it.part ?? ""}">${sw.join("")}${texts[k].place(cx + handle + gap, cy, { valign: "middle" })}</g>`);
        });
        return `<g data-part="${conf?.part ?? "legend"}">${g.join("")}</g>`;
      },
    };
  }

  function fontOf(st, which) {
    const F = st.font;
    const size = which === "base" ? F.size : F[which] ?? F.size;
    return { family: fontCss(F.family), size, math: F.math, color: st.colors.text, weight: which === "title" ? (F.title_weight === "bold" ? 700 : 400) : which === "panel" ? (F.panel_weight === "bold" ? 700 : 400) : 400 };
  }

  const INSIDE = ["upper right", "upper left", "lower right", "lower left", "center right", "center left", "upper center", "lower center"];
  function placeLegend(pos, leg, box, pad, cell) {
    const m = pad * 0.8;
    switch (pos) {
      case "upper left": return { x: box.x + m, y: box.y + m };
      case "lower left": return { x: box.x + m, y: box.y + box.h - leg.h - m };
      case "lower right": return { x: box.x + box.w - leg.w - m, y: box.y + box.h - leg.h - m };
      case "upper center": return { x: box.x + (box.w - leg.w) / 2, y: box.y + m };
      case "lower center": return { x: box.x + (box.w - leg.w) / 2, y: box.y + box.h - leg.h - m };
      case "center left": return { x: box.x + m, y: box.y + (box.h - leg.h) / 2 };
      case "center right": return { x: box.x + box.w - leg.w - m, y: box.y + (box.h - leg.h) / 2 };
      case "center": return { x: box.x + (box.w - leg.w) / 2, y: box.y + (box.h - leg.h) / 2 };
      case "outside right": return { x: cell.x + cell.w - leg.w, y: box.y };
      case "below": return { x: box.x + (box.w - leg.w) / 2, y: cell.y + cell.h - leg.h };
      case "above": return { x: box.x + (box.w - leg.w) / 2, y: cell.y + (cell.top ?? 0) };
      default: return { x: box.x + box.w - leg.w - m, y: box.y + m };
    }
  }
  /** How much of the data a rectangle covers: points inside + line samples inside (screen units). */
  function occupancy(rect, shapes) {
    let n = 0;
    const inside = (x, y) => x >= rect.x - 1 && x <= rect.x + rect.w + 1 && y >= rect.y - 1 && y <= rect.y + rect.h + 1;
    for (const p of shapes.points) if (inside(p[0], p[1])) n++;
    for (const l of shapes.lines) {
      for (let k = 1; k < l.length; k++) {
        const [a, b] = [l[k - 1], l[k]];
        for (let t = 0; t <= 1; t += 0.125) if (inside(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)) { n++; break; }
      }
    }
    for (const r of shapes.rects) if (!(r.x > rect.x + rect.w || r.x + r.w < rect.x || r.y > rect.y + rect.h || r.y + r.h < rect.y)) n += 3;
    return n;
  }
  /** The emptiest inside corner, or null when every one covers data (docs/plot.md: occupancy). */
  function bestInside(leg, shapes, box, pad, prefer) {
    const order = prefer ? [prefer, ...INSIDE.filter((p) => p !== prefer)] : INSIDE;
    let best = null, bestN = Infinity;
    order.forEach((c, k) => {
      const at = placeLegend(c, leg, box, pad, box);
      const n = occupancy({ x: at.x, y: at.y, w: leg.w, h: leg.h }, shapes) + k * 0.01;
      if (n < bestN) { bestN = n; best = c; }
    });
    return bestN < 1 ? best : null;
  }

  function panelLabel(spec, i) {
    const mode = spec.grid?.labels ?? ((spec.panels?.length ?? 0) > 1 ? "a" : "none");
    if (!mode || mode === "none") return "";
    const ch = String.fromCharCode(97 + i);
    return mode === "A" ? ch.toUpperCase() : mode === "(a)" ? `(${ch})` : mode === "a)" ? `${ch})` : ch;
  }

  // ── notes: text, arrow, point, bracket (auto-stacked), hline, vline ─────
  function notes(p, pi, c, plan, st, box) {
    const out = [];
    const f = { ...fontOf(st, "base"), size: st.font.legend ?? st.font.size };
    const ink = st.colors.axis;
    const lw = r2(Math.max(0.5, st.axes.line));
    const X = (n, v) => (n.coords === "axes" ? box.x + v * box.w : c.sx(v));
    const Y = (n, v) => (n.coords === "axes" ? box.y + (1 - v) * box.h : c.sy(v));
    // brackets without a height: stacked by level (shortest first) above the bars they span
    const auto = (p.notes ?? []).map((n, k) => ({ n, k })).filter(({ n }) => n.type === "bracket" && !isNum(num(n.y)));
    auto.sort((a, b) => Math.abs(num(a.n.x2 ?? a.n.x) - num(a.n.x)) - Math.abs(num(b.n.x2 ?? b.n.x) - num(b.n.x)));
    const level = new Map();
    const tiers = [];
    for (const { n, k } of auto) {
      const a = Math.min(num(n.x), num(n.x2 ?? n.x)), b = Math.max(num(n.x), num(n.x2 ?? n.x));
      let L = 0;
      while (tiers[L]?.some(([u, v]) => !(b < u || a > v))) L++;
      (tiers[L] ??= []).push([a, b]);
      level.set(k, L);
    }
    (p.notes ?? []).forEach((n, k) => {
      const part = `note:${pi}:${k}`;
      const nf = { ...f, size: isNum(n.size) ? n.size : f.size, color: n.color || f.color };
      if (n.type === "hline" || n.type === "vline") {
        const v = num(n.type === "hline" ? n.y : n.x);
        if (!isNum(v)) return;
        const d = n.type === "hline" ? `M${r2(box.x)} ${r2(c.sy(v))}L${r2(box.x + box.w)} ${r2(c.sy(v))}` : `M${r2(c.sx(v))} ${r2(box.y)}L${r2(c.sx(v))} ${r2(box.y + box.h)}`;
        let s = `<path d="${d}" stroke="${n.color || "#777777"}" stroke-width="${lw}"${dashArray(n.dash ?? "dash", lw)} fill="none"/>`;
        if (n.text) s += n.type === "hline" ? c.T(n.text, nf).place(box.x + box.w - 1.5, c.sy(v) - 1.5, { anchor: "end" }) : c.T(n.text, nf).place(c.sx(v) + 1.5, box.y + 1.5, { valign: "top" });
        out.push(`<g data-part="${part}">${s}</g>`);
      } else if (n.type === "bracket") {
        const a = num(n.x), b = num(n.x2 ?? n.x);
        if (!isNum(a) || !isNum(b)) return;
        const s1 = num(n.s), s2 = num(n.s2 ?? n.s);
        const xa = plan.barAt ? plan.barAt(c, a, s1) : c.sx(a), xb = plan.barAt ? plan.barAt(c, b, s2) : c.sx(b);
        const h = st.font.size * 0.4;
        const t = c.T(n.text ?? "*", nf);
        let y;
        const yv = num(n.y);
        if (isNum(yv)) y = c.sy(yv);
        else {
          let top = -Infinity;
          if (plan.barAt && isNum(s1)) top = Math.max(plan.barTop(a, s1), plan.barTop(b, s2));
          else if (plan.tops) top = Math.max(...plan.tops.slice(Math.min(a, b), Math.max(a, b) + 1).filter(isNum));
          if (!isFinite(top)) return;
          y = c.sy(top) - 4 - (level.get(k) ?? 0) * (t.h + h + 3);
        }
        c.shapes?.rects.push({ x: Math.min(xa, xb) - 1, y: y - t.h - 2, w: Math.abs(xb - xa) + 2, h: t.h + h + 2 });
        out.push(`<g data-part="${part}"><path d="M${r2(xa)} ${r2(y + h)}L${r2(xa)} ${r2(y)}L${r2(xb)} ${r2(y)}L${r2(xb)} ${r2(y + h)}" stroke="${ink}" stroke-width="${lw}" fill="none"/>${t.place((xa + xb) / 2, y - 1 - t.desc, { anchor: "middle" })}</g>`);
      } else if (n.type === "arrow") {
        const x1 = X(n, num(n.x)), y1 = Y(n, num(n.y));
        const x0 = X(n, num(n.x2 ?? n.x)), y0 = Y(n, num(n.y2 ?? n.y));
        if (![x0, y0, x1, y1].every(isNum)) return;
        const a = Math.atan2(y1 - y0, x1 - x0), hl = 2.4 + lw * 2;
        const hx = (s) => `${r2(x1 - hl * Math.cos(a + s))} ${r2(y1 - hl * Math.sin(a + s))}`;
        let s = `<path d="M${r2(x0)} ${r2(y0)}L${r2(x1 - Math.cos(a) * hl * 0.6)} ${r2(y1 - Math.sin(a) * hl * 0.6)}" stroke="${n.color || ink}" stroke-width="${lw}" fill="none"/><path d="M${r2(x1)} ${r2(y1)}L${hx(0.4)}L${hx(-0.4)}Z" fill="${n.color || ink}"/>`;
        if (n.text) {
          const t = c.T(n.text, nf);
          const anchor = Math.cos(a) > 0.3 ? "end" : Math.cos(a) < -0.3 ? "start" : "middle";
          s += t.place(x0 - Math.cos(a) * 1.5, y0 - Math.sin(a) * 1.5, { anchor, valign: Math.sin(a) > 0.3 ? "bottom" : Math.sin(a) < -0.3 ? "top" : "middle" });
        }
        out.push(`<g data-part="${part}">${s}</g>`);
      } else if (n.type === "point") {
        // an operating point: marker + label
        const x = X(n, num(n.x)), y = Y(n, num(n.y));
        if (!isNum(x) || !isNum(y)) return;
        let s = markerPath(n.marker ?? "o", x, y, (st.lines.marker || 3.5) * 1.3, n.color || ink, { hollow: n.hollow !== false, stroke: lw * 1.4 });
        if (n.text) s += c.T(n.text, nf).place(x + 3.5, y - 3.5, { valign: "bottom" });
        out.push(`<g data-part="${part}">${s}</g>`);
      } else {
        const x = X(n, num(n.x)), y = Y(n, num(n.y));
        if (!isNum(x) || !isNum(y)) return;
        out.push(`<g data-part="${part}">${c.T(n.text ?? "", nf).place(x, y, { anchor: n.anchor ?? "middle", valign: "middle" })}</g>`);
      }
    });
    return out;
  }

  /** Break marks ("//") across an axis line at screen position `at`. */
  function breakMark(horizAxis, x, y, ink, lw) {
    const d = 2.2, s = 1.6;
    const seg = (o) => (horizAxis ? `M${r2(x - s + o)} ${r2(y + d)}L${r2(x + s + o)} ${r2(y - d)}` : `M${r2(x - d)} ${r2(y + s + o)}L${r2(x + d)} ${r2(y - s + o)}`);
    const clearW = horizAxis ? `<rect x="${r2(x - 1.3)}" y="${r2(y - d)}" width="2.6" height="${r2(2 * d)}" fill="#ffffff"/>` : `<rect x="${r2(x - d)}" y="${r2(y - 1.3)}" width="${r2(2 * d)}" height="2.6" fill="#ffffff"/>`;
    return `${clearW}<path d="${seg(-1.3)}${seg(1.3)}" stroke="${ink}" stroke-width="${lw}" fill="none"/>`;
  }

  function inset(p, pi, plan, st, box, T, uid, main) {
    const at = p.inset.at ?? [0.45, 0.3, 0.5, 0.45];
    const ib = { x: box.x + at[0] * box.w, y: box.y + (1 - at[1] - at[3]) * box.h, w: at[2] * box.w, h: at[3] * box.h };
    const fx = p.inset.x ?? [0, 0.1], fy = p.inset.y ?? [0.8, 1];
    const fT = { ...fontOf(st, "tick"), size: st.font.tick * 0.85 };
    const yt = linearTicks(Math.min(...fy), Math.max(...fy), 3), xt = linearTicks(Math.min(...fx), Math.max(...fx), 3);
    const ins = st.inset ?? {};
    const sx = linScale(fx[0], fx[1], ib.x, ib.x + ib.w), sy = linScale(fy[0], fy[1], ib.y + ib.h, ib.y);
    const g = [];
    const grey = st.inset?.connector_color ?? "#c4c4c4";
    // the zoomed region on the main plot and thin grey connectors to the inset
    if (p.inset.mark !== false && main) {
      const zx0 = main.sx(fx[0]), zx1 = main.sx(fx[1]), zy0 = main.sy(fy[0]), zy1 = main.sy(fy[1]);
      const zr = { x: Math.min(zx0, zx1), y: Math.min(zy0, zy1), w: Math.abs(zx1 - zx0), h: Math.abs(zy1 - zy0) };
      g.push(`<rect x="${r2(zr.x)}" y="${r2(zr.y)}" width="${r2(zr.w)}" height="${r2(zr.h)}" fill="none" stroke="${grey}" stroke-width="${st.inset?.connector ?? 0.3}"/>`);
      // connect the two corners that face the inset
      const right = ib.x > zr.x + zr.w, below = ib.y > zr.y + zr.h;
      const zc = [[right ? zr.x + zr.w : zr.x, zr.y], [right ? zr.x + zr.w : zr.x, zr.y + zr.h]];
      const ic = [[right ? ib.x : ib.x + ib.w, ib.y], [right ? ib.x : ib.x + ib.w, ib.y + ib.h]];
      if (Math.abs(ib.x - zr.x) < 1 || !(right || ib.x + ib.w < zr.x)) {
        zc[0] = [zr.x, below ? zr.y + zr.h : zr.y]; zc[1] = [zr.x + zr.w, below ? zr.y + zr.h : zr.y];
        ic[0] = [ib.x, below ? ib.y : ib.y + ib.h]; ic[1] = [ib.x + ib.w, below ? ib.y : ib.y + ib.h];
      }
      g.push(`<path d="M${r2(zc[0][0])} ${r2(zc[0][1])}L${r2(ic[0][0])} ${r2(ic[0][1])}M${r2(zc[1][0])} ${r2(zc[1][1])}L${r2(ic[1][0])} ${r2(ic[1][1])}" stroke="${grey}" stroke-width="${st.inset?.connector ?? 0.3}"${dashArray("dash", 0.4)} fill="none"/>`);
    }
    g.push(`<rect x="${r2(ib.x)}" y="${r2(ib.y)}" width="${r2(ib.w)}" height="${r2(ib.h)}" fill="#ffffff"/>`);
    const gs = ` stroke="${st.axes.grid_color}" stroke-width="${st.axes.grid_width}" stroke-opacity="${st.axes.grid_alpha ?? 1}"${dashArray(st.axes.grid_dash, st.axes.grid_width)}`;
    if (p.inset.grid) g.push(`<path d="${xt.map((v) => `M${r2(sx(v))} ${r2(ib.y)}L${r2(sx(v))} ${r2(ib.y + ib.h)}`).join("")}${yt.map((v) => `M${r2(ib.x)} ${r2(sy(v))}L${r2(ib.x + ib.w)} ${r2(sy(v))}`).join("")}"${gs}/>`);
    const clip = `${uid}i${pi}`;
    const d = [];
    plan.draw({ sx, sy, out: d, T, font: (w) => fontOf(st, w), inset: true, shapes: { points: [], lines: [], rects: [] } });
    g.push(`<clipPath id="${clip}"><rect x="${r2(ib.x)}" y="${r2(ib.y)}" width="${r2(ib.w)}" height="${r2(ib.h)}"/></clipPath><g clip-path="url(#${clip})">${d.join("")}</g>`);
    g.push(`<rect x="${r2(ib.x)}" y="${r2(ib.y)}" width="${r2(ib.w)}" height="${r2(ib.h)}" fill="none" stroke="${st.colors.axis}" stroke-width="${ins.frame ?? 0.4}"/>`);
    const tl = st.axes.tick_len * 0.7;
    let tk = "";
    for (const v of xt) { tk += `M${r2(sx(v))} ${r2(ib.y + ib.h)}L${r2(sx(v))} ${r2(ib.y + ib.h + tl)}`; g.push(T(fmtNum(v, "", xt.step), fT).place(sx(v), ib.y + ib.h + tl + 1 + T("0", fT).asc, { anchor: "middle" })); }
    for (const v of yt.filter((v) => Math.abs(sy(v) - (ib.y + ib.h)) > 1)) { tk += `M${r2(ib.x)} ${r2(sy(v))}L${r2(ib.x - tl)} ${r2(sy(v))}`; g.push(T(fmtNum(v, "", yt.step), fT).place(ib.x - tl - 1, sy(v), { anchor: "end", valign: "middle" })); }
    g.push(`<path d="${tk}" stroke="${st.colors.axis}" stroke-width="${ins.frame ?? 0.4}"/>`);
    return { svg: `<g data-part="inset:${pi}">${g.join("")}</g>`, rect: { x: ib.x - 14, y: ib.y - 2, w: ib.w + 16, h: ib.h + 12 } };
  }

  // ── one panel ──────────────────────────────────────────────────────────
  function drawPanel(spec, p, pi, cell, st, o) {
    const T = (s, f) => measure(s, f, o.macros);
    const chart = CHARTS[p.type] ?? CHARTS.line;
    if (!CHARTS[p.type]) o.warnings.push(`不認得的圖表種類：${p.type}`);
    const tbl = table(spec, p);
    let plan;
    try {
      plan = o.plan ?? chart.prepare(p, tbl, st, { shared: o.shared ?? {} });
    } catch (e) {
      o.warnings.push(`第 ${pi + 1} 格：${e.message}`);
      return `<g data-part="panel:${pi}"></g>`;
    }
    const legendPos = o.legendPos ?? p.legend?.pos ?? plan.legendPos ?? st.legend.pos ?? "best";
    const legItems = p.legend?.show === false || o.shared?.noLegend ? [] : plan.legend.filter((it) => it.label !== "");
    let pos = legendPos === "direct" && !plan.ends?.length ? "best" : legendPos;
    const out = drawCartesianOrPolar(spec, p, pi, cell, st, o, plan, T, legItems, pos);
    if (out.retry) return drawCartesianOrPolar(spec, p, pi, cell, st, o, plan, T, legItems, out.retry).svg;
    return out.svg;
  }

  function drawCartesianOrPolar(spec, p, pi, cell, st, o, plan, T, legItems, pos) {
    const out = [];
    const fTick = fontOf(st, "tick"), fLab = fontOf(st, "label"), fTitle = fontOf(st, "title"), fLeg = { ...fontOf(st, "legend"), size: p.legend?.size ?? st.legend.size ?? st.font.legend };
    const ax = st.axes;
    const pad = ax.pad ?? 3;
    const tickOut = ax.tick_dir === "out" ? ax.tick_len : 0;
    const plabel = o.shared?.noPanelLabel ? "" : panelLabel(spec, pi);
    const pl = plabel ? T(plabel, fontOf(st, "panel")) : null;
    const title = p.title ? T(p.title, fTitle) : null;
    // panel letter outside the axes, top-left; the title on its own line
    let top = cell.y + Math.max(title ? title.h + pad : 0, pl ? pl.h + pad * 0.5 : 0) + (title || pl ? 0 : pad);
    let left = cell.x + (pl && !title ? 0 : 0), right = cell.x + cell.w, bottom = cell.y + cell.h;
    const outsideTopMax = st.legend.top_max ?? 6;
    const leg = legItems.length && pos !== "direct" ? legendBlock(legItems, fLeg, st, { ...p.legend, cols: p.legend?.cols ?? (pos === "below" || pos === "above" ? Math.min(legItems.length, outsideTopMax) : 1) }, T) : null;
    const legTop = top;
    if (leg && pos === "outside right") right -= leg.w + pad;
    if (leg && pos === "below") bottom -= leg.h + pad;
    if (leg && pos === "above") top += leg.h + pad * 0.6;

    if (plan.polar) {
      const lab = Math.max(...(plan.cats ?? []).map((c) => T(c, fTick).w), plan.labelRoom ?? 0, 0);
      const R = Math.max(5, Math.min((right - left) / 2 - lab - 4, (bottom - top) / 2 - fTick.size - 4));
      const cx = (left + right) / 2, cy = (top + bottom) / 2;
      // a free chart (a simulation scene, eqsim.js) draws its own layout in the panel's box
      const c = { out, T, font: (w) => fontOf(st, w), polar: { cx, cy, R }, box: { x: left, y: top, w: right - left, h: bottom - top }, st, inset: false };
      plan.draw(c);
      if (title) out.push(title.place(cx, cell.y + title.asc, { anchor: "middle", part: `title:${pi}` }));
      if (pl) out.push(pl.place(cell.x, cell.y + pl.asc, { part: `panellabel:${pi}` }));
      if (leg) {
        const ppos = pos === "best" ? "upper right" : pos;
        const at = placeLegend(ppos, leg, { x: left, y: top, w: right - left, h: bottom - top }, pad, { ...cell, top: legTop - cell.y });
        out.push(leg.draw(at.x, at.y, INSIDE.includes(ppos)));
      }
      return { svg: `<g data-part="panel:${pi}">${out.join("")}</g>` };
    }

    const X = p.xaxis ?? {}, Y = p.yaxis ?? {};
    const hideYT = o.shared?.hideYTicks, hideXT = o.shared?.hideXTicks;
    const xTicksN = clamp(Math.round(cell.w / PT / 14), 3, st.axes.max_ticks ?? 7);
    const yTicksN = clamp(Math.round(cell.h / PT / 11), 3, st.axes.max_ticks ?? 7);
    const xconf = { ...X, scale: X.scale ?? (X.log ? "log" : undefined) }, yconf = { ...Y, scale: Y.scale ?? (Y.log ? "log" : undefined) };
    const mk = (a, conf, r0, r1, sharedDom) => {
      if (a.kind === "band") return bandScale(a.cats.length, r0, r1);
      let dom = sharedDom ?? domainOf(a.values, conf, { zero: a.zero, fixed: a.fixed, pad: a.zero ? 0.06 : st.axes.padding ?? 0.05, nice: a.zero });
      if (conf?.scale === "stretch" && !isNum(conf?.min) && !isNum(conf?.max) && !a.fixed && Array.isArray(conf.segments)) {
        const e = conf.segments.flatMap((s) => [num(s[0]), num(s[1])]).filter(isNum);
        dom = [Math.min(dom[0], ...e), Math.max(dom[1], ...e)];
      }
      return contScale(conf, dom[0], dom[1], r0, r1);
    };
    let sx = mk(plan.x, xconf, 0, 1, o.shared?.xdom), sy = plan.y.kind === "band" ? bandScale(plan.y.cats.length, 0, 1) : mk(plan.y, yconf, 1, 0, o.shared?.ydom);
    const xt = X.hidden ? [] : axisTicks(plan.x, sx, xconf, xTicksN);
    const yt = Y.hidden ? [] : axisTicks(plan.y, sy, yconf, yTicksN);
    const xTexts = xt.map((t) => T(t.text, fTick)), yTexts = yt.map((t) => T(t.text, fTick));
    let rot = X.rotate ?? 0;
    if (X.rotate == null && plan.x.kind === "band" && xTexts.length > 1) {
      const est = (cell.w - (Y.label ? fLab.size * 2 : 0) - fTick.size * 3) / xTexts.length;
      if (xTexts.some((t) => t.w > est * 0.92)) rot = 30;
    }
    const rad = (Math.abs(rot) * Math.PI) / 180;
    const xTickH = hideXT ? 0 : Math.max(0, ...xTexts.map((t) => (rot ? t.w * Math.sin(rad) + t.h * Math.cos(rad) : t.h)));
    const yTickW = hideYT ? 0 : Math.max(0, ...yTexts.map((t) => t.w));
    // a chart may colour its axis labels (equation figures: each variable in its colour)
    const xl = !X.hidden && !o.shared?.hideXLabel && withUnit(X) ? T(plan.xlabel ?? withUnit(X), fLab) : null;
    const yl = !Y.hidden && !o.shared?.hideYLabel && withUnit(Y) ? T(plan.ylabel ?? withUnit(Y), fLab) : null;
    const tp = pad * 0.6;
    // direct labels at line ends: room on the right
    const ends = pos === "direct" ? (plan.ends ?? []).map((e) => ({ ...e, t: T(e.label, { ...fLeg, color: e.color }) })) : [];
    if (ends.length) right -= Math.max(...ends.map((e) => e.t.w)) + pad;
    const plotL = left + (yl ? yl.h + pad : 0) + yTickW + (hideYT ? 0 : tp) + tickOut + 0.5;
    const plotB = bottom - (xl ? xl.h + pad * 0.7 : 0) - xTickH - (hideXT ? 0 : tp) - tickOut - 0.5;
    const lastHalf = !rot && plan.x.kind !== "band" && xTexts.length && !hideXT ? xTexts.at(-1).w / 2 : 0;
    let plotR = right - Math.max(pad * 0.6, lastHalf - 2);
    let plotT = top + (yTexts.length && !hideYT ? yTexts.at(-1).h / 2 : 0);
    let cbar = null;
    if (plan.colorbar && !o.shared?.noColorbar) {
      const cb = plan.colorbar;
      const cbt = linearTicks(cb.vmin, cb.vmax, 4).filter((v) => v >= Math.min(cb.vmin, cb.vmax) - 1e-9 && v <= Math.max(cb.vmin, cb.vmax) + 1e-9);
      const texts = cbt.map((v) => T(fmtNum(v, cb.fmt, cbt.step), fTick));
      const bw = Math.max(4, st.font.size * 0.9);
      cbar = { cbt, texts, bw, label: cb.label ? T(cb.label, fLab) : null };
      plotR -= bw + pad + tickOut + tp + Math.max(0, ...texts.map((t) => t.w)) + (cbar.label ? cbar.label.h + pad : 0);
    }
    if (plan.x.top) plotT += xTickH + tp;
    let W = Math.max(10, plotR - plotL), H = Math.max(10, plotB - plotT);
    // square plots (ROC, confusion matrix, embeddings): the smaller side, centred
    if (plan.square || p.square) {
      const s = Math.min(W, H);
      if (W > s) { const d = (W - s) / 2; W = s; plotR = plotL + s; void d; }
      if (H > s) { plotT += H - s; H = s; }
    }
    sx = plan.x.kind === "band" ? bandScale(plan.x.cats.length, plotL, plotL + W) : mk(plan.x, xconf, plotL, plotL + W, o.shared?.xdom);
    sy = plan.y.kind === "band" ? (plan.y.reverse ? bandScale(plan.y.cats.length, plotT, plotT + H) : bandScale(plan.y.cats.length, plotT + H, plotT)) : mk(plan.y, yconf, plotT + H, plotT, o.shared?.ydom);
    const box = { x: plotL, y: plotT, w: W, h: H };

    // grid: major (and minor), always behind the data
    const gridMode = plan.noGrid ? "none" : p.grid ?? ax.grid;
    const gx = !plan.noGrid && (X.grid ?? (gridMode === "x" || gridMode === "both"));
    const gy = !plan.noGrid && (Y.grid ?? (gridMode === "y" || gridMode === "both"));
    const minorOn = (a) => a.minor ?? ax.grid_minor ?? false;
    const gs = (w, a) => ` stroke="${ax.grid_color}" stroke-width="${r2(w)}" stroke-opacity="${a}"${dashArray(ax.grid_dash, w)} fill="none"`;
    const grid = [];
    const ga = ax.grid_alpha ?? 1;
    const xMinor = sx.kind !== "band" && xt.raw ? sx.minor(xt.raw, ax.grid_density) : [];
    const yMinor = sy.kind !== "band" && yt.raw ? sy.minor(yt.raw, ax.grid_density) : [];
    if (gx && sx.kind !== "band") {
      if (minorOn(X)) grid.push(`<path d="${xMinor.map((v) => `M${r2(sx(v))} ${r2(plotT)}L${r2(sx(v))} ${r2(plotT + H)}`).join("")}"${gs(ax.grid_width * 0.6, ga * 0.6)}/>`);
      grid.push(`<path d="${xt.map((t) => `M${r2(sx(t.v))} ${r2(plotT)}L${r2(sx(t.v))} ${r2(plotT + H)}`).join("")}"${gs(ax.grid_width, ga)}/>`);
    }
    if (gy && sy.kind !== "band") {
      if (minorOn(Y)) grid.push(`<path d="${yMinor.map((v) => `M${r2(plotL)} ${r2(sy(v))}L${r2(plotL + W)} ${r2(sy(v))}`).join("")}"${gs(ax.grid_width * 0.6, ga * 0.6)}/>`);
      grid.push(`<path d="${yt.map((t) => `M${r2(plotL)} ${r2(sy(t.v))}L${r2(plotL + W)} ${r2(sy(t.v))}`).join("")}"${gs(ax.grid_width, ga)}/>`);
    }
    if (grid.length) out.push(`<g data-part="grid:${pi}">${grid.join("")}</g>`);

    // data, clipped to the plot area; shapes collected for legend placement
    const clip = `${o.uid}c${pi}`;
    const valT = plan.horizontal ? xt : yt;
    const valStep = valT.length > 1 ? Math.abs(valT[1].v - valT[0].v) / 10 : 0;
    out.push(`<clipPath id="${clip}"><rect x="${r2(plotL - 1)}" y="${r2(plotT - 1)}" width="${r2(W + 2)}" height="${r2(H + 2)}"/></clipPath>`);
    const dataOut = [];
    const shapes = { points: [], lines: [], rects: [] };
    const ctx = { sx, sy, out: dataOut, T, font: (w) => fontOf(st, w), inset: false, valStep, box, shapes };
    plan.draw(ctx);
    for (const q of plan.points ?? []) {
      const x = sx(q[0]), y = sy(q[1]);
      if (isNum(x) && isNum(y)) shapes.points.push([x, y]);
    }
    out.push(`<g clip-path="url(#${clip})">${dataOut.join("")}</g>`);
    out.push(...notes(p, pi, ctx, plan, st, box));

    // spines (interrupted at broken-axis gaps), major and minor ticks, break marks
    const aw = ax.line, ink = st.colors.axis;
    const sp = p.spines ?? plan.spines ?? ax.spines ?? "lb";
    const sides = sp === "box" ? "lbrt" : sp === "none" ? "" : sp;
    const spine = [];
    const runs = (scale, a, b) => {
      const g = scale.gaps ?? [];
      if (!g.length) return [[a, b]];
      const cuts = g.map(([u, v]) => [scale(u), scale(v)].sort((m, n) => m - n));
      const r = [];
      let at = Math.min(a, b);
      for (const [u, v] of cuts.sort((m, n) => m[0] - n[0])) { r.push([at, u]); at = v; }
      r.push([at, Math.max(a, b)]);
      return r;
    };
    if (!plan.heat) {
      if (sides.includes("l")) for (const [a, b] of runs(sy, plotT, plotT + H)) spine.push(`M${r2(plotL)} ${r2(a)}L${r2(plotL)} ${r2(b)}`);
      if (sides.includes("b")) for (const [a, b] of runs(sx, plotL, plotL + W)) spine.push(`M${r2(a)} ${r2(plotT + H)}L${r2(b)} ${r2(plotT + H)}`);
      if (sides.includes("r")) for (const [a, b] of runs(sy, plotT, plotT + H)) spine.push(`M${r2(plotL + W)} ${r2(a)}L${r2(plotL + W)} ${r2(b)}`);
      if (sides.includes("t")) for (const [a, b] of runs(sx, plotL, plotL + W)) spine.push(`M${r2(a)} ${r2(plotT)}L${r2(b)} ${r2(plotT)}`);
    }
    const tl = ax.tick_len, dir = ax.tick_dir === "in" ? -1 : 1;
    const ticks = [];
    const xAxisY = plan.x.top ? plotT : plotT + H;
    const xs = plan.x.top ? -1 : 1;
    const showTicks = !plan.heat && !plan.noTicks;
    let lastX = null;
    xt.forEach((t, k) => {
      const x = sx(t.v);
      if (!(x >= plotL - 0.5 && x <= plotL + W + 0.5)) return;
      if (showTicks) ticks.push(`M${r2(x)} ${r2(xAxisY)}L${r2(x)} ${r2(xAxisY + xs * dir * tl)}`);
      if (hideXT || plan.noTicks) return;
      if (!rot && lastX != null && sx.kind !== "band" && Math.abs(x - lastX) < (xTexts[k].w + xTexts[k - 1 >= 0 ? k - 1 : k].w) / 2 + 1) return;
      lastX = x;
      const ty = xAxisY + xs * (tickOut + tp);
      const tx = xTexts[k];
      if (rot) out.push(tx.place(x, ty + tx.asc * Math.cos(rad) * 0.5, { anchor: "end", rotate: -Math.abs(rot), valign: "middle", part: `xaxis:${pi}` }));
      else out.push(tx.place(x, plan.x.top ? ty - tx.desc : ty + tx.asc, { anchor: "middle", part: `xaxis:${pi}` }));
    });
    let lastY = null;
    yt.forEach((t, k) => {
      const y = sy(t.v);
      if (!(y >= plotT - 0.5 && y <= plotT + H + 0.5)) return;
      if (showTicks) ticks.push(`M${r2(plotL)} ${r2(y)}L${r2(plotL - dir * tl)} ${r2(y)}`);
      if (hideYT || plan.noTicks) return;
      // labels that would touch the previous one are left out (the tick stays)
      if (lastY != null && Math.abs(y - lastY) < yTexts[k].h * 1.05 && sy.kind !== "band") return;
      lastY = y;
      out.push(yTexts[k].place(plotL - tickOut - tp, y, { anchor: "end", valign: "middle", part: `yaxis:${pi}` }));
    });
    if (showTicks) {
      if (minorOn(X)) for (const v of xMinor) ticks.push(`M${r2(sx(v))} ${r2(xAxisY)}L${r2(sx(v))} ${r2(xAxisY + xs * dir * tl * 0.55)}`);
      if (minorOn(Y)) for (const v of yMinor) ticks.push(`M${r2(plotL)} ${r2(sy(v))}L${r2(plotL - dir * tl * 0.55)} ${r2(sy(v))}`);
    }
    if (spine.length || ticks.length) out.push(`<path d="${spine.join("")}${ticks.join("")}" stroke="${ink}" stroke-width="${aw}" fill="none" stroke-linecap="square" data-part="axes:${pi}"/>`);
    const marks = [];
    for (const v of sy.marks ?? []) marks.push(breakMark(false, plotL, sy(v), ink, aw));
    for (const [u, v] of sy.gaps ?? []) { marks.push(breakMark(false, plotL, sy(u), ink, aw), breakMark(false, plotL, sy(v), ink, aw)); }
    for (const v of sx.marks ?? []) marks.push(breakMark(true, sx(v), plotT + H, ink, aw));
    for (const [u, v] of sx.gaps ?? []) { marks.push(breakMark(true, sx(u), plotT + H, ink, aw), breakMark(true, sx(v), plotT + H, ink, aw)); }
    // a bar axis that does not start at zero: a break mark when the figure asks for one (opts.break_mark / bars.break_mark)
    if (plan.zeroBase && !plan.horizontal && sy.kind !== "band" && Math.min(...sy.domain) > 0 && (p.opts?.break_mark ?? st.bars.break_mark)) marks.push(breakMark(false, plotL, plotT + H - Math.min(6, H * 0.04), ink, aw));
    if (plan.zeroBase && plan.horizontal && sx.kind !== "band" && Math.min(...sx.domain) > 0 && (p.opts?.break_mark ?? st.bars.break_mark)) marks.push(breakMark(true, plotL + Math.min(6, W * 0.04), plotT + H, ink, aw));
    if (marks.length) out.push(`<g data-part="axes:${pi}">${marks.join("")}</g>`);
    if (xl) {
      const y = plotB + tickOut + tp + xTickH + pad * 0.7 + xl.asc;
      out.push(xl.place(plotL + W / 2, plan.x.top ? plotT + H + tickOut + pad + xl.asc : y, { anchor: "middle", part: `xaxis:${pi}` }));
    }
    if (yl) out.push(yl.place(left + yl.asc, plotT + H / 2, { anchor: "middle", rotate: -90, part: `yaxis:${pi}` }));
    if (title) out.push(title.place(plotL + W / 2, cell.y + title.asc, { anchor: "middle", part: `title:${pi}` }));
    if (pl) out.push(pl.place(cell.x, cell.y + pl.asc, { part: `panellabel:${pi}` }));

    if (cbar) {
      const x0 = plotL + W + pad, n = 48;
      const seg = [];
      const cb = plan.colorbar;
      for (let k = 0; k < n; k++) {
        const y0 = plotT + H - (H * (k + 1)) / n;
        seg.push(`<rect x="${r2(x0)}" y="${r2(y0)}" width="${r2(cbar.bw)}" height="${r2(H / n + 0.1)}" fill="${cb.color ? cb.color((k + 0.5) / n) : cmap(cb.cmap, (k + 0.5) / n)}"/>`);
      }
      const cs = linScale(cb.vmin, cb.vmax, plotT + H, plotT);
      const tk = cbar.cbt.map((v) => `M${r2(x0 + cbar.bw)} ${r2(cs(v))}L${r2(x0 + cbar.bw + tl)} ${r2(cs(v))}`).join("");
      seg.push(`<rect x="${r2(x0)}" y="${r2(plotT)}" width="${r2(cbar.bw)}" height="${r2(H)}" fill="none" stroke="${ink}" stroke-width="${aw}"/><path d="${tk}" stroke="${ink}" stroke-width="${aw}"/>`);
      const tw = Math.max(0, ...cbar.texts.map((t) => t.w));
      cbar.cbt.forEach((v, k) => seg.push(cbar.texts[k].place(x0 + cbar.bw + tl + tp, cs(v), { valign: "middle" })));
      if (cbar.label) seg.push(cbar.label.place(x0 + cbar.bw + tl + tp + tw + pad + cbar.label.asc, plotT + H / 2, { anchor: "middle", rotate: 90 }));
      out.push(`<g data-part="colorbar:${pi}">${seg.join("")}</g>`);
    }

    let insetRect = null;
    if (p.inset && !plan.heat && sx.kind !== "band") {
      const r = inset(p, pi, plan, st, box, T, o.uid, { sx, sy });
      out.push(r.svg);
      insetRect = r.rect;
      shapes.rects.push(insetRect);
    }

    // direct labels at line ends (nudged apart)
    if (ends.length) {
      const items = ends.map((e) => ({ ...e, y: sy(e.y) })).filter((e) => isNum(e.y)).sort((a, b) => a.y - b.y);
      const hgt = Math.max(...items.map((e) => e.t.h)) + 0.5;
      for (let k = 1; k < items.length; k++) if (items[k].y - items[k - 1].y < hgt) items[k].y = items[k - 1].y + hgt;
      const over = items.length ? items.at(-1).y - (plotT + H) : 0;
      if (over > 0) for (const e of items) e.y -= over;
      out.push(`<g data-part="legend">${items.map((e) => `<g data-part="${e.part ?? ""}">${e.t.place(plotL + W + pad * 0.6, e.y, { valign: "middle" })}</g>`).join("")}</g>`);
    }

    if (leg) {
      let ppos = pos;
      if (pos === "best") {
        ppos = bestInside(leg, shapes, box, pad, plan.legendPrefer);
        if (!ppos && !o.legendPos) return { retry: legItems.length <= outsideTopMax ? "above" : "outside right" };
        ppos ??= "upper right";
      }
      const at = placeLegend(ppos, leg, box, pad, { ...cell, top: legTop - cell.y, right });
      out.push(leg.draw(at.x, at.y, INSIDE.includes(ppos) || ppos === "center"));
    }
    return { svg: `<g data-part="panel:${pi}">${out.join("")}</g>` };
  }

  // ── the figure: layout + panels ────────────────────────────────────────
  /** Grid cells for each panel: spans (`span` columns, `rowspan` rows) fill the first free slot. */
  function placePanels(spec) {
    const cols = Math.max(1, spec.grid?.cols ?? 1);
    let rows = Math.max(1, spec.grid?.rows ?? 1);
    const used = [];
    const free = (r, c, w, h) => {
      if (c + w > cols) return false;
      for (let i = r; i < r + h; i++) for (let j = c; j < c + w; j++) if (used[i]?.[j]) return false;
      return true;
    };
    const at = [];
    for (const p of spec.panels ?? []) {
      const w = clamp(Math.round(p.span ?? 1), 1, cols), h = Math.max(1, Math.round(p.rowspan ?? 1));
      let placed = null;
      for (let r = 0; !placed && r < 64; r++) for (let c = 0; c < cols; c++) if (free(r, c, w, h)) { placed = { r, c, w, h }; break; }
      for (let i = placed.r; i < placed.r + h; i++) for (let j = placed.c; j < placed.c + w; j++) (used[i] ??= [])[j] = true;
      at.push(placed);
      rows = Math.max(rows, placed.r + h);
    }
    return { cols, rows, at };
  }
  LorePlot.placePanels = placePanels;

  let uidN = 0;
  /** Render a spec. opts: { styles, macros, style (resolved, wins) }. */
  LorePlot.render = async function (spec, opts = {}) {
    spec = spec ?? {};
    await LorePlot.data();
    const st = opts.style ?? LorePlot.styleFor(spec.style, opts.styles, spec.set);
    await warm(spec, st, opts.macros);
    const warnings = [];
    const Wmm = spec.size?.w ?? st.size.w, Hmm = spec.size?.h ?? st.size.h;
    const W = Wmm * PT, H = Hmm * PT;
    const { cols, rows, at } = placePanels(spec);
    const gap = (spec.grid?.gap ?? 5) * PT;
    const margin = (spec.margin ?? 1) * PT;
    const uid = `lp${++uidN}`;
    const body = [];
    const panels = spec.panels ?? [];
    // a shared legend for the whole figure (top / right / bottom)
    let top = margin, right = W - margin, bottomEdge = H - margin;
    const shareLeg = spec.grid?.legend;
    const T = (s, f) => measure(s, f, opts.macros);
    const plans = panels.map((p) => {
      try { return (CHARTS[p.type] ?? CHARTS.line).prepare(p, table(spec, p), st, { shared: {} }); } catch { return null; }
    });
    if (shareLeg && ["top", "right", "bottom"].includes(shareLeg)) {
      const seen = new Map();
      plans.forEach((pl) => (pl?.legend ?? []).forEach((it) => { if (it.label && !seen.has(it.label)) seen.set(it.label, it); }));
      const items = [...seen.values()];
      if (items.length) {
        const fLeg = { ...fontOf(st, "legend"), size: st.legend.size ?? st.font.legend };
        const leg = legendBlock(items, fLeg, st, { cols: shareLeg === "right" ? 1 : Math.min(items.length, st.legend.top_max ?? 6), frame: false, part: "legend:all" }, T);
        if (shareLeg === "top") { body.push(leg.draw((W - leg.w) / 2, top)); top += leg.h + gap * 0.4; }
        else if (shareLeg === "bottom") { body.push(leg.draw((W - leg.w) / 2, bottomEdge - leg.h)); bottomEdge -= leg.h + gap * 0.4; }
        else { body.push(leg.draw(right - leg.w, top)); right -= leg.w + gap * 0.5; }
      }
    }
    const cw = (right - margin - gap * (cols - 1)) / cols, ch = (bottomEdge - top - gap * (rows - 1)) / rows;
    // shared axes: one domain per row (y) / column (x) over the panels that are not bands
    const share = { y: spec.grid?.sharey, x: spec.grid?.sharex };
    const ydoms = new Map(), xdoms = new Map();
    if (share.y || share.x) {
      plans.forEach((pl, i) => {
        if (!pl || pl.polar) return;
        const a = at[i];
        if (share.y && pl.y.kind !== "band") ydoms.set(a.r, [...(ydoms.get(a.r) ?? []), ...pl.y.values, ...(pl.y.zero ? [0] : [])]);
        if (share.x && pl.x.kind !== "band") xdoms.set(a.c, [...(xdoms.get(a.c) ?? []), ...pl.x.values]);
      });
    }
    // a shared colour range for heatmaps
    let cRange = null;
    if (spec.grid?.share_color) {
      const v = plans.flatMap((pl) => (pl?.colorbar ? [pl.colorbar.vmin, pl.colorbar.vmax] : []));
      if (v.length) cRange = [Math.min(...v), Math.max(...v)];
    }
    const lastHeat = plans.map((pl, i) => (pl?.colorbar ? i : -1)).filter((i) => i >= 0).at(-1);
    panels.forEach((p, i) => {
      const a = at[i];
      const cell = { x: margin + a.c * (cw + gap), y: top + a.r * (ch + gap), w: cw * a.w + gap * (a.w - 1), h: ch * a.h + gap * (a.h - 1) };
      const shared = { noLegend: !!shareLeg };
      if (share.y && ydoms.has(a.r)) {
        const pl = plans[i];
        shared.ydom = domainOf(ydoms.get(a.r), p.yaxis, { zero: pl?.y.zero, pad: 0.05, nice: pl?.y.zero });
        if (a.c > 0) { shared.hideYTicks = true; shared.hideYLabel = true; }
      }
      if (share.x && xdoms.has(a.c)) {
        shared.xdom = domainOf(xdoms.get(a.c), p.xaxis, {});
        const below = at.some((b, j) => j !== i && b.c === a.c && b.r > a.r);
        if (below) { shared.hideXTicks = true; shared.hideXLabel = true; }
      }
      let plan = plans[i];
      if (plan && cRange && plan.colorbar) { plan = (CHARTS[p.type]).prepare(p, table(spec, p), st, { shared: { vmin: cRange[0], vmax: cRange[1] } }); if (i !== lastHeat) shared.noColorbar = true; }
      body.push(drawPanel(spec, p, i, cell, st, { macros: opts.macros, warnings, uid, shared, plan: plan && !share.y && !share.x ? plan : undefined }));
    });
    const bg = st.colors.background && st.colors.background !== "none" ? `<rect width="${r2(W)}" height="${r2(H)}" fill="${st.colors.background}" data-part="figure"/>` : "";
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${r2(Wmm)}mm" height="${r2(Hmm)}mm" viewBox="0 0 ${r2(W)} ${r2(H)}" text-rendering="geometricPrecision" data-lore-plot="1">${bg}${body.join("")}</svg>`;
    return { svg, w: Math.round(Wmm * 10) / 10, h: Math.round(Hmm * 10) / 10, warnings, style: st };
  };

  /** Font families an SVG uses (for embedding / checking). */
  LorePlot.fontsIn = (svg) => [...new Set([...svg.matchAll(/font-family="([^"]+)"/g)].flatMap((m) => m[1].split(",").map((s) => s.trim().replace(/^'|'$/g, ""))))];

  window.LorePlot = LorePlot;
})();
