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
      size: 8, title: 8, label: 8, tick: 7, legend: 7, panel: 9, panel_weight: "bold", title_weight: "normal", math: "text",
    },
    axes: { line: 0.6, tick_len: 3, tick_dir: "out", spines: "lb", grid: "none", grid_dash: "dot", grid_color: "#c8c8c8", grid_width: 0.4, pad: 3 },
    lines: { width: 1.2, marker: 3.5, markers: ["o", "s", "^", "D", "v", "x"], dashes: ["solid"] },
    bars: { width: 0.8, gap: 0.04, edge: 0, edge_color: "", cap: 3, alpha: 1 },
    scatter: { size: 3, alpha: 0.8 },
    legend: { frame: false, pos: "best", size: 7 },
    colors: {
      palette: ["#0072B2", "#E69F00", "#009E73", "#D55E00", "#CC79A7", "#56B4E9", "#F0E442", "#000000"],
      lines: [], cmap: "viridis", text: "#000000", axis: "#000000", background: "#ffffff",
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

  // ── text: KaTeX laid out, then flattened into SVG ──────────────────────
  let host = null;
  const cache = new Map();
  let katexLoading = null;
  function ensureHost() {
    if (host?.isConnected) return host;
    host = document.createElement("div");
    host.className = "lp-measure";
    host.setAttribute("aria-hidden", "true");
    host.style.cssText = "position:absolute;left:-30000px;top:0;white-space:nowrap;pointer-events:none;contain:layout style";
    document.body.append(host);
    if (!document.getElementById("lp-measure-css")) {
      const st = document.createElement("style");
      st.id = "lp-measure-css";
      st.textContent = `.lp-measure .lp-t{display:inline-block;line-height:1;white-space:pre}
.lp-measure .lp-t .katex{font-size:1.1em;line-height:1;text-rendering:geometricPrecision}
.lp-measure .lp-t.lp-mtext .katex{font-size:1em;font-family:var(--lp-font)}
.lp-measure .lp-t.lp-mtext .katex .mathnormal,.lp-measure .lp-t.lp-mtext .katex .mathit{font-family:var(--lp-font);font-style:italic}
.lp-measure .lp-t.lp-mtext .katex .mathrm,.lp-measure .lp-t.lp-mtext .katex .textrm{font-family:var(--lp-font);font-style:normal}
.lp-measure .lp-t.lp-mtext .katex .mathbf,.lp-measure .lp-t.lp-mtext .katex .textbf{font-family:var(--lp-font);font-weight:bold;font-style:normal}
.lp-measure .lp-t.lp-mtext .katex .boldsymbol{font-family:var(--lp-font);font-weight:bold;font-style:italic}`;
      document.head.append(st);
    }
    return host;
  }
  const hasMath = (s) => /(^|[^\\])\$/.test(s);
  function loadKatex() {
    if (window.katex) return Promise.resolve();
    if (katexLoading) return katexLoading;
    const base = LorePlot.katexBase ?? LorePlot.base + "katex/";
    katexLoading = new Promise((ok) => {
      if (!document.querySelector(`link[data-lp-katex]`)) {
        const l = document.createElement("link");
        l.rel = "stylesheet";
        l.href = base + "katex.min.css";
        l.dataset.lpKatex = "1";
        document.head.append(l);
      }
      const s = document.createElement("script");
      s.src = base + "katex.min.js";
      s.onload = () => ok();
      s.onerror = () => ok();
      document.head.append(s);
    });
    return katexLoading;
  }
  /** Split on unescaped $…$ into [{math:bool, s}]. */
  function pieces(str) {
    const out = [];
    let buf = "", math = false;
    for (let i = 0; i < str.length; i++) {
      const c = str[i];
      if (c === "\\" && str[i + 1] === "$") { buf += math ? "\\$" : "$"; i++; continue; }
      if (c === "$") { if (buf) out.push({ math, s: buf }); buf = ""; math = !math; continue; }
      buf += c;
    }
    if (buf) out.push({ math: math && false, s: math ? "$" + buf : buf });
    return out;
  }
  function htmlOf(str, macros) {
    return pieces(str)
      .map((p) => {
        if (!p.math) return esc(p.s);
        if (!window.katex) return esc(p.s);
        try {
          return window.katex.renderToString(p.s, { throwOnError: false, output: "html", macros: { ...(macros ?? {}) }, strict: "ignore" });
        } catch {
          return esc(p.s);
        }
      })
      .join("");
  }
  const famAttr = (f) => f.replace(/"/g, "'");
  /**
   * Lay out `str` in font `f` ({ family, size, weight, italic, math, color }) and flatten it.
   * Returns { w, asc, desc, place(x, y, { anchor, rotate, valign }) → svg string }.
   */
  function measure(str, f, macros) {
    str = String(str ?? "");
    const key = JSON.stringify([str, f, macros ? Object.keys(macros).length : 0]);
    if (cache.has(key)) return cache.get(key);
    const h = ensureHost();
    const span = document.createElement("span");
    span.className = "lp-t" + (f.math === "text" ? " lp-mtext" : "");
    span.style.cssText = `font-family:${f.family};--lp-font:${f.family};font-size:${f.size * S}px;font-weight:${f.weight ?? 400};font-style:${f.italic ? "italic" : "normal"};color:${f.color ?? "#000"}`;
    span.innerHTML = htmlOf(str, macros) || "​";
    h.append(span);
    const box = span.getBoundingClientRect();
    const probe = document.createElement("span");
    probe.style.cssText = "display:inline-block;width:0;height:0;vertical-align:baseline;padding:0;margin:0;border:0";
    span.insertBefore(probe, span.firstChild);
    const base = probe.getBoundingClientRect().bottom;
    probe.remove();
    const X = (v) => r2((v - box.left) / S), Y = (v) => r2((v - base) / S);
    const parts = [];
    // text runs
    const walker = document.createTreeWalker(span, NodeFilter.SHOW_TEXT);
    const nodes = [];
    for (let n = walker.nextNode(); n; n = walker.nextNode()) if (n.nodeValue && n.nodeValue !== "​" && n.nodeValue.trim() !== "") nodes.push(n);
    for (const n of nodes) {
      const parent = n.parentElement;
      const cs = getComputedStyle(parent);
      if (cs.display === "none") continue;
      const range = document.createRange();
      range.selectNodeContents(n);
      const rr = range.getClientRects()[0] ?? range.getBoundingClientRect();
      parent.insertBefore(probe, n);
      const bl = probe.getBoundingClientRect().bottom;
      probe.remove();
      const fs = r2(parseFloat(cs.fontSize) / S);
      const wt = cs.fontWeight === "400" || cs.fontWeight === "normal" ? "" : ` font-weight="${cs.fontWeight}"`;
      const it = cs.fontStyle === "normal" ? "" : ` font-style="${cs.fontStyle}"`;
      parts.push(`<text x="${X(rr.left)}" y="${Y(bl)}" font-family="${famAttr(cs.fontFamily)}" font-size="${fs}"${wt}${it} fill="${f.color ?? "#000"}" xml:space="preserve">${esc(n.nodeValue)}</text>`);
    }
    // rules (fraction bars, over/underlines) and KaTeX's own svg paths
    for (const el of span.querySelectorAll("*")) {
      if (el.tagName.toLowerCase() === "svg") {
        const r = el.getBoundingClientRect();
        let clip = el.parentElement;
        while (clip && clip !== span && getComputedStyle(clip).overflow !== "hidden") clip = clip.parentElement;
        const c = clip && clip !== span ? clip.getBoundingClientRect() : r;
        const inner = el.cloneNode(true);
        inner.setAttribute("x", r2((r.left - c.left) / S));
        inner.setAttribute("y", r2((r.top - c.top) / S));
        inner.setAttribute("width", r2(r.width / S));
        inner.setAttribute("height", r2(r.height / S));
        inner.removeAttribute("style");
        inner.setAttribute("fill", f.color ?? "#000");
        const xml = new XMLSerializer().serializeToString(inner).replace(/ xmlns="http:\/\/www\.w3\.org\/2000\/svg"/, "");
        parts.push(`<svg x="${X(c.left)}" y="${Y(c.top)}" width="${r2(c.width / S)}" height="${r2(c.height / S)}" overflow="hidden">${xml}</svg>`);
        continue;
      }
      if (el.closest("svg")) continue;
      const cs = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      for (const side of ["Top", "Bottom"]) {
        const w = parseFloat(cs[`border${side}Width`]);
        if (w > 0 && cs[`border${side}Style`] !== "none" && r.width > 0) {
          const y = side === "Top" ? r.top : r.bottom - w;
          parts.push(`<rect x="${X(r.left)}" y="${Y(y)}" width="${r2(r.width / S)}" height="${r2(w / S)}" fill="${f.color ?? "#000"}"/>`);
        }
      }
      for (const side of ["Left", "Right"]) {
        const w = parseFloat(cs[`border${side}Width`]);
        if (w > 0 && cs[`border${side}Style`] !== "none" && r.height > 0) {
          const x = side === "Left" ? r.left : r.right - w;
          parts.push(`<rect x="${X(x)}" y="${Y(r.top)}" width="${r2(w / S)}" height="${r2(r.height / S)}" fill="${f.color ?? "#000"}"/>`);
        }
      }
    }
    span.remove();
    const empty = !str.trim();
    const out = {
      w: empty ? 0 : r2(box.width / S),
      asc: empty ? 0 : r2((base - box.top) / S),
      desc: empty ? 0 : r2((box.bottom - base) / S),
      body: parts.join(""),
    };
    out.h = r2(out.asc + out.desc);
    /** anchor: start | middle | end; valign: baseline | middle | top | bottom (before rotation). */
    out.place = (x, y, o = {}) => {
      if (empty) return "";
      const dx = o.anchor === "middle" ? -out.w / 2 : o.anchor === "end" ? -out.w : 0;
      const dy = o.valign === "middle" ? (out.asc - out.desc) / 2 : o.valign === "top" ? out.asc : o.valign === "bottom" ? -out.desc : 0;
      const rot = o.rotate ? ` rotate(${o.rotate})` : "";
      const part = o.part ? ` data-part="${esc(o.part)}"` : "";
      return `<g transform="translate(${r2(x)} ${r2(y)})${rot} translate(${r2(dx)} ${r2(dy)})"${part}>${out.body}</g>`;
    };
    cache.set(key, out);
    if (cache.size > 4000) cache.delete(cache.keys().next().value);
    return out;
  }

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
    if (strings(spec).some(hasMath)) await loadKatex();
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

  // ── scales ─────────────────────────────────────────────────────────────
  function linScale(d0, d1, r0, r1) {
    const f = (v) => r0 + ((v - d0) / (d1 - d0 || 1)) * (r1 - r0);
    Object.assign(f, { kind: "linear", domain: [d0, d1], range: [r0, r1] });
    return f;
  }
  function logScale(d0, d1, r0, r1) {
    const l0 = Math.log10(d0), l1 = Math.log10(d1);
    const f = (v) => (v > 0 ? r0 + ((Math.log10(v) - l0) / (l1 - l0 || 1)) * (r1 - r0) : NaN);
    Object.assign(f, { kind: "log", domain: [d0, d1], range: [r0, r1] });
    return f;
  }
  function bandScale(n, r0, r1, pad = 0) {
    const step = (r1 - r0) / Math.max(1, n);
    const f = (i) => r0 + step * (i + 0.5);
    Object.assign(f, { kind: "band", n, step, range: [r0, r1], domain: [-0.5, n - 0.5], pad });
    return f;
  }
  /** Numeric domain: data extent, padded, axis min/max win. */
  function domainOf(values, ax, o = {}) {
    let v = values.filter(isNum);
    if (ax?.log) v = v.filter((x) => x > 0);
    let lo = v.length ? Math.min(...v) : 0, hi = v.length ? Math.max(...v) : 1;
    if (o.zero) { lo = Math.min(lo, 0); hi = Math.max(hi, 0); }
    if (lo === hi) { lo -= 0.5; hi += 0.5; }
    if (ax?.log) {
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

  // ── chart types ────────────────────────────────────────────────────────
  // Each: prepare(panel, tbl, st, ctx) → { x, y, legend[], points[], draw(c) }
  //   x / y: { kind: "linear"|"log"|"band", values | domain, cats, fixed, zero }
  //   draw(c): c.sx, c.sy, c.out (push svg), c.inset (bool)
  const seriesOf = (p) => (p.series ?? []).filter((s) => !s.hidden);
  function seriesStyle(st, s, i, kind) {
    const pal = st.colors.palette?.length ? st.colors.palette : ROOT_STYLE.colors.palette;
    const lines = st.colors.lines?.length ? st.colors.lines : null;
    const base = pal[i % pal.length];
    const color = s.color || (kind === "line" && lines ? lines[i % lines.length] : base);
    const markers = st.lines.markers ?? [];
    const dashes = st.lines.dashes?.length ? st.lines.dashes : ["solid"];
    return {
      color,
      fill: s.color || base,
      marker: s.marker ?? (st.lines.marker > 0 ? markers[i % (markers.length || 1)] ?? "o" : "none"),
      dash: s.dash ?? dashes[i % dashes.length],
      width: isNum(s.width) ? s.width : st.lines.width,
      alpha: isNum(s.alpha) ? s.alpha : null,
      msize: isNum(s.msize) ? s.msize : st.lines.marker,
    };
  }
  const labelOf = (s, fallback) => (s.label != null && s.label !== "" ? s.label : fallback ?? s.y ?? "");
  const errOf = (tbl, s, n) => {
    if (!s.err) return null;
    const e = ncol(tbl, s.err);
    return e.length ? e : null;
  };

  const CHARTS = {};
  CHARTS.line = {
    label: "折線",
    prepare(p, tbl, st) {
      const xs = ncol(tbl, p.x);
      const xcat = !p.x || xs.every((v) => !isNum(v));
      const cats = xcat ? col(tbl, p.x).map((v) => String(v ?? "")) : null;
      const ser = seriesOf(p).map((s, i) => {
        const ys = ncol(tbl, s.y);
        const err = errOf(tbl, s, ys.length);
        return { s, i, st: seriesStyle(st, s, i, "line"), ys, err, label: labelOf(s) };
      });
      const allY = ser.flatMap((r) => r.ys.flatMap((y, k) => (r.err ? [y - r.err[k], y + r.err[k]] : [y])));
      const X = (k) => (xcat ? k : xs[k]);
      return {
        x: xcat ? { kind: "band", cats } : { kind: "linear", values: xs },
        y: { kind: "linear", values: allY },
        legend: ser.map((r) => ({ label: r.label, kind: "line", ...r.st, part: `series:${r.i}` })),
        points: ser.flatMap((r) => r.ys.map((y, k) => [X(k), y])),
        draw(c) {
          const band = p.opts?.err_style !== "bars";
          for (const r of ser) {
            const pts = r.ys.map((y, k) => [c.sx(X(k)), c.sy(y)]);
            const g = [];
            if (r.err && band) {
              const up = r.ys.map((y, k) => [c.sx(X(k)), c.sy(y + r.err[k])]);
              const dn = r.ys.map((y, k) => [c.sx(X(k)), c.sy(y - r.err[k])]).reverse();
              g.push(`<path d="${polyline(up.concat(dn))}Z" fill="${r.st.color}" fill-opacity="0.18" stroke="none"/>`);
            }
            if (r.err && !band) g.push(errBars(pts, r.ys, r.err, c, st, r.st.color));
            let d = polyline(pts);
            if (p.opts?.step) d = polyline(pts.flatMap((q, k) => (k ? [[q[0], pts[k - 1][1]], q] : [q])));
            g.push(`<path d="${d}" fill="none" stroke="${r.st.color}" stroke-width="${r.st.width}" stroke-linejoin="round" stroke-linecap="round"${dashArray(r.st.dash, r.st.width)}${r.st.alpha != null ? ` stroke-opacity="${r.st.alpha}"` : ""}/>`);
            for (const q of pts) if (isNum(q[0]) && isNum(q[1])) g.push(markerPath(r.st.marker, q[0], q[1], r.st.msize, r.st.color, { stroke: r.st.width * 0.6 }));
            c.out.push(`<g data-part="series:${r.i}">${g.join("")}</g>`);
          }
        },
      };
    },
  };
  function errBars(pts, ys, err, c, st, color, horiz) {
    const cap = st.bars.cap / 2;
    const w = Math.max(0.5, st.axes.line);
    let d = "";
    pts.forEach((q, k) => {
      const e = err?.[k];
      if (!isNum(e) || !isNum(ys[k]) || !isNum(q[0])) return;
      if (horiz) {
        const a = c.sx(ys[k] - e), b = c.sx(ys[k] + e);
        d += `M${r2(a)} ${r2(q[1])}L${r2(b)} ${r2(q[1])}`;
        if (cap > 0) d += `M${r2(a)} ${r2(q[1] - cap)}L${r2(a)} ${r2(q[1] + cap)}M${r2(b)} ${r2(q[1] - cap)}L${r2(b)} ${r2(q[1] + cap)}`;
      } else {
        const a = c.sy(ys[k] - e), b = c.sy(ys[k] + e);
        d += `M${r2(q[0])} ${r2(a)}L${r2(q[0])} ${r2(b)}`;
        if (cap > 0) d += `M${r2(q[0] - cap)} ${r2(a)}L${r2(q[0] + cap)} ${r2(a)}M${r2(q[0] - cap)} ${r2(b)}L${r2(q[0] + cap)} ${r2(b)}`;
      }
    });
    return d ? `<path d="${d}" stroke="${color}" stroke-width="${r2(w)}" fill="none"/>` : "";
  }

  CHARTS.bar = {
    label: "長條",
    prepare(p, tbl, st) {
      const cats = col(tbl, p.x).map((v) => String(v ?? ""));
      const stacked = p.opts?.mode === "stacked";
      const horiz = !!p.opts?.horizontal;
      const ser = seriesOf(p).map((s, i) => ({ s, i, st: seriesStyle(st, s, i, "bar"), ys: ncol(tbl, s.y), err: errOf(tbl, s), label: labelOf(s) }));
      const n = ser.length || 1;
      const tops = cats.map((_, k) => (stacked ? ser.reduce((a, r) => a + (isNum(r.ys[k]) ? r.ys[k] : 0), 0) : Math.max(...ser.map((r) => (r.ys[k] ?? 0) + (r.err?.[k] ?? 0)))));
      const vals = stacked ? tops : ser.flatMap((r) => r.ys.map((y, k) => y + (r.err?.[k] ?? 0)).concat(r.ys.map((y, k) => y - (r.err?.[k] ?? 0))));
      const catAx = { kind: "band", cats }, valAx = { kind: "linear", values: vals, zero: true };
      const plan = {
        x: horiz ? valAx : catAx,
        y: horiz ? catAx : valAx,
        horizontal: horiz,
        legend: ser.map((r) => ({ label: r.label, kind: "patch", ...r.st, color: r.st.fill, part: `series:${r.i}` })),
        points: cats.flatMap((_, k) => [horiz ? [tops[k], k] : [k, tops[k]]]),
        tops,
        /** Centre of category k's bar of series si (band axis coordinate), for brackets. */
        barAt(c, k, si) {
          const band = horiz ? c.sy : c.sx;
          if (stacked || !isNum(si)) return band(k);
          const groupW = Math.abs(band.step) * st.bars.width, gap = Math.abs(band.step) * (st.bars.gap ?? 0);
          const bw = (groupW - gap * (n - 1)) / n;
          return band(k) - groupW / 2 + si * (bw + gap) + bw / 2;
        },
        barTop(k, si) {
          const r = ser[si];
          return r ? (r.ys[k] ?? 0) + (r.err?.[k] ?? 0) : tops[k];
        },
        draw(c) {
          const band = horiz ? c.sy : c.sx, val = horiz ? c.sx : c.sy;
          const groupW = Math.abs(band.step) * st.bars.width;
          const gap = Math.abs(band.step) * (st.bars.gap ?? 0);
          const bw = stacked ? groupW : (groupW - gap * (n - 1)) / n;
          const base = new Array(cats.length).fill(0);
          const edge = st.bars.edge > 0 ? ` stroke="${st.bars.edge_color || "#000"}" stroke-width="${st.bars.edge}"` : "";
          for (const r of ser) {
            const g = [];
            const centers = [];
            cats.forEach((_, k) => {
              const y = r.ys[k];
              if (!isNum(y)) { centers.push([NaN, NaN]); return; }
              const c0 = band(k) - groupW / 2 + (stacked ? 0 : r.i * (bw + gap));
              const v0 = stacked ? base[k] : 0, v1 = stacked ? base[k] + y : y;
              if (stacked) base[k] += y;
              const [ra, rb] = [Math.min(...val.range), Math.max(...val.range)];
              const a = clamp(val(v0), ra, rb), b = clamp(val(v1), ra, rb);
              const alpha = r.st.alpha ?? st.bars.alpha;
              const op = alpha < 1 ? ` fill-opacity="${alpha}"` : "";
              if (horiz) g.push(`<rect x="${r2(Math.min(a, b))}" y="${r2(c0)}" width="${r2(Math.abs(b - a))}" height="${r2(bw)}" fill="${r.st.fill}"${op}${edge}/>`);
              else g.push(`<rect x="${r2(c0)}" y="${r2(Math.min(a, b))}" width="${r2(bw)}" height="${r2(Math.abs(b - a))}" fill="${r.st.fill}"${op}${edge}/>`);
              centers.push(horiz ? [b, c0 + bw / 2] : [c0 + bw / 2, b]);
            });
            if (r.err && !stacked) {
              const errColor = st.bars.err_color || st.colors.axis;
              const pts = centers.map((q) => (horiz ? [NaN, q[1]] : [q[0], NaN]));
              if (horiz) g.push(errBars(centers.map((q) => [q[0], q[1]]), r.ys, r.err, { sx: c.sx, sy: c.sy }, st, errColor, true));
              else g.push(errBars(pts.map((q, k) => [q[0], 0]), r.ys, r.err, c, st, errColor));
            }
            if (p.opts?.values && !c.inset) {
              const f = c.font("legend");
              cats.forEach((_, k) => {
                const y = r.ys[k], q = centers[k];
                if (!isNum(y) || !isNum(q[0])) return;
                const t = c.T(fmtNum(y, p.opts.fmt ?? "", c.valStep), f);
                const lift = (r.err?.[k] && !stacked ? Math.abs(val(y + r.err[k]) - val(y)) : 0) + 1.5;
                g.push(horiz ? t.place(q[0] + lift, q[1], { valign: "middle" }) : t.place(q[0], q[1] - lift - t.desc, { anchor: "middle" }));
              });
            }
            c.out.push(`<g data-part="series:${r.i}">${g.join("")}</g>`);
          }
        },
      };
      return plan;
    },
  };

  CHARTS.scatter = {
    label: "散佈／群集",
    prepare(p, tbl, st) {
      const xs = ncol(tbl, p.x), ys = ncol(tbl, p.y ?? seriesOf(p)[0]?.y);
      const groups = p.group ? col(tbl, p.group).map((v) => String(v ?? "")) : null;
      const names = groups ? [...new Set(groups)] : [p.series?.[0]?.label || p.y || ""];
      const sizes = p.size ? ncol(tbl, p.size) : null;
      const smax = sizes ? Math.max(...sizes.filter(isNum)) : 1;
      const sty = names.map((g, i) => {
        const s = (p.series ?? []).find((q) => q.group === g) ?? (groups ? {} : p.series?.[0] ?? {});
        return { name: g, i, s, st: seriesStyle(st, s, i, "scatter"), label: s.label || g };
      });
      return {
        x: { kind: "linear", values: xs },
        y: { kind: "linear", values: ys },
        legend: groups || p.series?.[0]?.label ? sty.map((g) => ({ label: g.label, kind: "marker", ...g.st, marker: g.s.marker ?? "o", part: `group:${g.i}` })) : [],
        points: xs.map((x, k) => [x, ys[k]]),
        draw(c) {
          const alpha = isNum(p.opts?.alpha) ? p.opts.alpha : st.scatter.alpha;
          const d = st.scatter.size;
          for (const g of sty) {
            const idx = xs.map((_, k) => k).filter((k) => (groups ? groups[k] === g.name : true) && isNum(xs[k]) && isNum(ys[k]));
            const out = [];
            const pts = idx.map((k) => [xs[k], ys[k]]);
            if (p.opts?.hull && pts.length >= 3) {
              const hp = hull(pts).map((q) => [c.sx(q[0]), c.sy(q[1])]);
              out.push(`<path d="${polyline(hp)}Z" fill="${g.st.fill}" fill-opacity="0.12" stroke="${g.st.fill}" stroke-width="${r2(st.axes.line)}" stroke-linejoin="round"/>`);
            }
            if (p.opts?.ellipse && pts.length >= 3) {
              const e = ellipse(pts);
              if (e) {
                const steps = 72, ring = [];
                for (let t = 0; t <= steps; t++) {
                  const a = (t / steps) * 2 * Math.PI;
                  const ex = e.rx * Math.cos(a), ey = e.ry * Math.sin(a);
                  ring.push([c.sx(e.cx + ex * Math.cos(e.ang) - ey * Math.sin(e.ang)), c.sy(e.cy + ex * Math.sin(e.ang) + ey * Math.cos(e.ang))]);
                }
                out.push(`<path d="${polyline(ring)}Z" fill="${g.st.fill}" fill-opacity="0.08" stroke="${g.st.fill}" stroke-width="${r2(st.axes.line * 1.2)}"${dashArray("dash", st.axes.line)}/>`);
              }
            }
            for (const k of idx) {
              const dd = sizes ? d * (0.4 + 1.6 * Math.sqrt(Math.max(0, sizes[k]) / (smax || 1))) : g.s.msize ?? d;
              out.push(markerPath(g.s.marker ?? "o", c.sx(xs[k]), c.sy(ys[k]), dd, g.st.fill, { stroke: 0.3, edge: st.scatter.edge || "#ffffff" }).replace("/>", alpha < 1 ? ` fill-opacity="${alpha}"/>` : "/>"));
            }
            c.out.push(`<g data-part="group:${g.i}">${out.join("")}</g>`);
          }
        },
      };
    },
  };

  function distributions(p, tbl) {
    // long format (group + y) or wide (one column per series)
    if (p.group && (p.y || p.series?.[0]?.y)) {
      const ycol = p.y ?? p.series[0].y;
      const g = col(tbl, p.group).map((v) => String(v ?? "")), ys = ncol(tbl, ycol);
      const names = [...new Set(g)];
      return names.map((n, i) => ({ name: n, i, s: (p.series ?? []).find((q) => q.group === n) ?? {}, vals: ys.filter((v, k) => g[k] === n && isNum(v)) }));
    }
    return seriesOf(p).map((s, i) => ({ name: labelOf(s), i, s, vals: ncol(tbl, s.y).filter(isNum) }));
  }
  function boxLike(kind) {
    return {
      label: kind === "box" ? "盒鬚圖" : "小提琴圖",
      prepare(p, tbl, st) {
        const dist = distributions(p, tbl);
        return {
          x: { kind: "band", cats: dist.map((d) => d.s.label || d.name) },
          y: { kind: "linear", values: dist.flatMap((d) => d.vals) },
          legend: [],
          points: dist.flatMap((d, i) => d.vals.map((v) => [i, v])),
          draw(c) {
            for (const d of dist) {
              const sst = seriesStyle(st, d.s, d.i, "bar");
              const sorted = [...d.vals].sort((a, b) => a - b);
              if (!sorted.length) continue;
              const q1 = quantile(sorted, 0.25), q2 = quantile(sorted, 0.5), q3 = quantile(sorted, 0.75);
              const iqr = q3 - q1;
              const lo = sorted.find((v) => v >= q1 - 1.5 * iqr), hi = [...sorted].reverse().find((v) => v <= q3 + 1.5 * iqr);
              const cx = c.sx(d.i), bw = Math.abs(c.sx.step) * st.bars.width * (kind === "box" ? 0.6 : 0.9);
              const lw = r2(st.axes.line * 1.2), ink = st.colors.axis;
              const g = [];
              if (kind === "violin") {
                const [a, b] = [sorted[0], sorted.at(-1)];
                const at = Array.from({ length: 60 }, (_, k) => a + ((b - a) * k) / 59);
                const den = kde(sorted, at);
                const m = Math.max(...den) || 1;
                const right = at.map((v, k) => [cx + (den[k] / m) * (bw / 2), c.sy(v)]);
                const left = at.map((v, k) => [cx - (den[k] / m) * (bw / 2), c.sy(v)]).reverse();
                g.push(`<path d="${polyline(right.concat(left))}Z" fill="${sst.fill}" fill-opacity="${st.bars.alpha < 1 ? st.bars.alpha : 0.75}" stroke="${ink}" stroke-width="${r2(st.axes.line)}"/>`);
                const iw = Math.max(1.5, bw * 0.08);
                g.push(`<rect x="${r2(cx - iw / 2)}" y="${r2(c.sy(q3))}" width="${r2(iw)}" height="${r2(Math.abs(c.sy(q1) - c.sy(q3)))}" fill="${ink}"/>`);
                g.push(`<path d="M${r2(cx)} ${r2(c.sy(lo))}L${r2(cx)} ${r2(c.sy(hi))}" stroke="${ink}" stroke-width="${r2(st.axes.line * 0.8)}"/>`);
                g.push(`<circle cx="${r2(cx)}" cy="${r2(c.sy(q2))}" r="${r2(Math.max(1, iw * 0.45))}" fill="#fff"/>`);
              } else {
                g.push(`<path d="M${r2(cx)} ${r2(c.sy(lo))}L${r2(cx)} ${r2(c.sy(q1))}M${r2(cx)} ${r2(c.sy(q3))}L${r2(cx)} ${r2(c.sy(hi))}M${r2(cx - bw / 4)} ${r2(c.sy(lo))}L${r2(cx + bw / 4)} ${r2(c.sy(lo))}M${r2(cx - bw / 4)} ${r2(c.sy(hi))}L${r2(cx + bw / 4)} ${r2(c.sy(hi))}" stroke="${ink}" stroke-width="${lw}" fill="none"/>`);
                g.push(`<rect x="${r2(cx - bw / 2)}" y="${r2(c.sy(q3))}" width="${r2(bw)}" height="${r2(Math.abs(c.sy(q1) - c.sy(q3)))}" fill="${sst.fill}" fill-opacity="${st.bars.alpha}" stroke="${ink}" stroke-width="${lw}"/>`);
                g.push(`<path d="M${r2(cx - bw / 2)} ${r2(c.sy(q2))}L${r2(cx + bw / 2)} ${r2(c.sy(q2))}" stroke="${ink}" stroke-width="${r2(lw * 1.4)}"/>`);
                if (!p.opts?.points) for (const v of sorted) if (v < lo || v > hi) g.push(markerPath("o", cx, c.sy(v), 2.4, ink, { hollow: true, stroke: 0.5 }));
              }
              if (p.opts?.mean) g.push(markerPath("^", cx, c.sy(mean(sorted)), 3.2, "#ffffff", { edge: ink, stroke: 0.6 }));
              if (p.opts?.points) {
                let seed = 7 + d.i * 131;
                const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
                for (const v of sorted) g.push(markerPath("o", cx + (rnd() - 0.5) * bw * 0.55, c.sy(v), 1.8, darker(sst.fill, 0.45), { stroke: 0 }).replace("/>", ` fill-opacity="0.7"/>`));
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

  CHARTS.hist = {
    label: "直方圖",
    prepare(p, tbl, st) {
      const ser = seriesOf(p).map((s, i) => ({ s, i, st: seriesStyle(st, s, i, "bar"), vals: ncol(tbl, s.y).filter(isNum), label: labelOf(s) }));
      const all = ser.flatMap((r) => r.vals);
      const lo = all.length ? Math.min(...all) : 0, hi = all.length ? Math.max(...all) : 1;
      const nb = clamp(Math.round(p.opts?.bins ?? Math.ceil(Math.log2(Math.max(2, all.length / Math.max(1, ser.length))) + 1)), 2, 200);
      const w = (hi - lo || 1) / nb;
      const edges = Array.from({ length: nb + 1 }, (_, k) => lo + k * w);
      const counts = ser.map((r) => {
        const c = new Array(nb).fill(0);
        for (const v of r.vals) c[Math.min(nb - 1, Math.floor((v - lo) / w))]++;
        return p.opts?.density ? c.map((x) => x / ((r.vals.length || 1) * w)) : c;
      });
      const stacked = !!p.opts?.stacked;
      const tops = edges.slice(0, -1).map((_, k) => (stacked ? counts.reduce((a, c) => a + c[k], 0) : Math.max(...counts.map((c) => c[k]))));
      return {
        x: { kind: "linear", values: [lo, hi], fixed: [lo - w * 0.1, hi + w * 0.1] },
        y: { kind: "linear", values: tops, zero: true },
        legend: ser.length > 1 || ser[0]?.s.label ? ser.map((r) => ({ label: r.label, kind: "patch", ...r.st, color: r.st.fill, part: `series:${r.i}` })) : [],
        points: tops.map((t, k) => [edges[k] + w / 2, t]),
        draw(c) {
          const base = new Array(nb).fill(0);
          for (const r of ser) {
            const alpha = stacked || ser.length === 1 ? st.bars.alpha : Math.min(st.bars.alpha, 0.6);
            const g = counts[r.i].map((v, k) => {
              const b0 = stacked ? base[k] : 0;
              if (stacked) base[k] += v;
              const x0 = c.sx(edges[k]), x1 = c.sx(edges[k + 1]);
              const y0 = c.sy(b0), y1 = c.sy(b0 + v);
              return `<rect x="${r2(x0)}" y="${r2(Math.min(y0, y1))}" width="${r2(x1 - x0)}" height="${r2(Math.abs(y0 - y1))}" fill="${r.st.fill}" fill-opacity="${alpha}" stroke="#ffffff" stroke-width="0.3"/>`;
            });
            c.out.push(`<g data-part="series:${r.i}">${g.join("")}</g>`);
          }
        },
      };
    },
  };

  CHARTS.heatmap = {
    label: "熱圖／混淆矩陣",
    prepare(p, tbl, st) {
      const rowsName = tbl.columns[0];
      const valueCols = p.series?.length ? seriesOf(p).map((s) => s.y) : tbl.columns.slice(1);
      const rowLabels = col(tbl, p.x ?? rowsName).map((v) => String(v ?? ""));
      let M = rowLabels.map((_, r) => valueCols.map((cn) => num(tbl.rows[r][tbl.columns.indexOf(cn)])));
      if (p.opts?.normalize === "rows") M = M.map((row) => { const s = row.reduce((a, v) => a + (isNum(v) ? v : 0), 0) || 1; return row.map((v) => v / s); });
      const flat = M.flat().filter(isNum);
      const vmin = isNum(p.opts?.vmin) ? p.opts.vmin : Math.min(...flat), vmax = isNum(p.opts?.vmax) ? p.opts.vmax : Math.max(...flat);
      const colLabels = seriesOf(p).length ? seriesOf(p).map((s) => labelOf(s)) : valueCols;
      const name = p.opts?.cmap || st.colors.cmap || "viridis";
      return {
        x: { kind: "band", cats: colLabels, top: !!p.opts?.top },
        y: { kind: "band", cats: rowLabels, reverse: true },
        heat: true,
        colorbar: p.opts?.colorbar ? { vmin, vmax, cmap: name } : null,
        legend: [],
        points: [],
        draw(c) {
          const g = [];
          const cw = Math.abs(c.sx.step), ch = Math.abs(c.sy.step);
          M.forEach((row, ri) => row.forEach((v, ci) => {
            if (!isNum(v)) return;
            const t = (v - vmin) / (vmax - vmin || 1);
            const fill = cmap(name, t);
            const x = c.sx(ci) - cw / 2, y = c.sy(ri) - ch / 2;
            g.push(`<rect x="${r2(x)}" y="${r2(y)}" width="${r2(cw + 0.05)}" height="${r2(ch + 0.05)}" fill="${fill}"/>`);
            if (p.opts?.values !== false) {
              const f = { ...c.font("tick"), color: luminance(fill) > 0.4 ? "#000000" : "#ffffff" };
              const fmt = p.opts?.fmt ?? (p.opts?.normalize === "rows" ? ".2f" : flat.every((q) => Number.isInteger(q)) ? "d" : ".2f");
              g.push(c.T(fmtNum(v, fmt), f).place(c.sx(ci), c.sy(ri), { anchor: "middle", valign: "middle" }));
            }
          }));
          c.out.push(`<g data-part="cells">${g.join("")}</g>`);
        },
      };
    },
  };

  function curveChart(kind) {
    return {
      label: kind === "roc" ? "ROC 曲線" : "PR 曲線",
      prepare(p, tbl, st) {
        const ser = seriesOf(p).map((s, i) => {
          let xs, ys, auc;
          if (s.score && s.truth) ({ xs, ys, auc } = (kind === "roc" ? rocFrom : prFrom)(ncol(tbl, s.score), ncol(tbl, s.truth)));
          else {
            const pts = ncol(tbl, s.x ?? p.x).map((x, k) => [x, ncol(tbl, s.y)[k]]).filter((q) => isNum(q[0]) && isNum(q[1]));
            if (kind === "roc") pts.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
            xs = pts.map((q) => q[0]); ys = pts.map((q) => q[1]);
            auc = isNum(num(s.auc)) ? num(s.auc) : kind === "roc" ? trapz(xs, ys) : Math.abs(trapz(xs, ys));
          }
          const word = p.opts?.auc_label ?? (kind === "roc" ? "AUC" : "AP");
          const label = labelOf(s, s.score ?? s.y) + (p.opts?.auc !== false && isNum(auc) ? ` (${word} = ${auc.toFixed(p.opts?.auc_digits ?? 3)})` : "");
          return { s, i, xs, ys, auc, st: seriesStyle(st, s, i, "line"), label };
        });
        return {
          x: { kind: "linear", values: [0, 1], fixed: [0, 1] },
          y: { kind: "linear", values: [0, 1], fixed: [0, 1.0] },
          legend: ser.map((r) => ({ label: r.label, kind: "line", ...r.st, marker: "none", part: `series:${r.i}` })),
          points: ser.flatMap((r) => r.xs.map((x, k) => [x, r.ys[k]])),
          curves: ser,
          draw(c) {
            if (kind === "roc" && p.opts?.diagonal !== false) c.out.push(`<path d="M${r2(c.sx(0))} ${r2(c.sy(0))}L${r2(c.sx(1))} ${r2(c.sy(1))}" stroke="#9a9a9a" stroke-width="${r2(st.axes.line)}"${dashArray("dash", st.axes.line)} fill="none"/>`);
            for (const r of ser) {
              const pts = r.xs.map((x, k) => [c.sx(x), c.sy(r.ys[k])]);
              const d = kind === "pr" ? polyline(pts.flatMap((q, k) => (k ? [[q[0], pts[k - 1][1]], q] : [q]))) : polyline(pts);
              c.out.push(`<g data-part="series:${r.i}"><path d="${d}" fill="none" stroke="${r.st.color}" stroke-width="${r.st.width}" stroke-linejoin="round"${dashArray(r.st.dash, r.st.width)}/></g>`);
            }
          },
        };
      },
    };
  }
  CHARTS.roc = curveChart("roc");
  CHARTS.pr = curveChart("pr");

  CHARTS.bland = {
    label: "Bland–Altman",
    prepare(p, tbl, st) {
      const s = seriesOf(p)[0] ?? {};
      const a = ncol(tbl, s.y), b = ncol(tbl, s.y2);
      const m = a.map((v, k) => (v + b[k]) / 2), d = a.map((v, k) => v - b[k]);
      const ok = d.map((v, k) => isNum(v) && isNum(m[k]));
      const dd = d.filter((_, k) => ok[k]);
      const md = mean(dd), s2 = sd(dd);
      const lim = [md - 1.96 * s2, md + 1.96 * s2];
      const sst = seriesStyle(st, s, 0, "scatter");
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
    label: "Kaplan–Meier",
    prepare(p, tbl, st) {
      const t = ncol(tbl, p.x), e = ncol(tbl, p.event);
      const groups = p.group ? col(tbl, p.group).map((v) => String(v ?? "")) : t.map(() => "");
      const names = [...new Set(groups)];
      const curves = names.map((g, i) => {
        const idx = t.map((_, k) => k).filter((k) => groups[k] === g);
        const s = (p.series ?? []).find((q) => q.group === g) ?? {};
        return { g, i, s, st: seriesStyle(st, s, i, "line"), km: kaplanMeier(idx.map((k) => t[k]), idx.map((k) => e[k])), label: s.label || g };
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
            cv.km.steps.forEach((q, k) => {
              if (k) pts.push([c.sx(q.t), c.sy(cv.km.steps[k - 1].s)]);
              pts.push([c.sx(q.t), c.sy(q.s)]);
            });
            pts.push([c.sx(cv.km.tmax), c.sy(cv.km.steps.at(-1).s)]);
            const g = [`<path d="${polyline(pts)}" fill="none" stroke="${cv.st.color}" stroke-width="${cv.st.width}"${dashArray(cv.st.dash, cv.st.width)}/>`];
            if (p.opts?.censor !== false) for (const q of cv.km.cens) g.push(markerPath("+", c.sx(q.t), c.sy(q.s), 3, cv.st.color));
            c.out.push(`<g data-part="group:${cv.i}">${g.join("")}</g>`);
          }
        },
      };
    },
  };

  CHARTS.radar = {
    label: "雷達圖",
    polar: true,
    prepare(p, tbl, st) {
      const cats = col(tbl, p.x).map((v) => String(v ?? ""));
      const ser = seriesOf(p).map((s, i) => ({ s, i, st: seriesStyle(st, s, i, "line"), ys: ncol(tbl, s.y), label: labelOf(s) }));
      const vals = ser.flatMap((r) => r.ys).filter(isNum);
      const lo = isNum(p.yaxis?.min) ? p.yaxis.min : 0;
      const hi = isNum(p.yaxis?.max) ? p.yaxis.max : linearTicks(lo, Math.max(...vals, lo + 1), 4).at(-1);
      return {
        polar: true,
        cats, lo, hi,
        legend: ser.map((r) => ({ label: r.label, kind: "line", ...r.st, part: `series:${r.i}` })),
        points: [],
        draw(c) {
          const { cx, cy, R } = c.polar;
          const ang = (k) => -Math.PI / 2 + (2 * Math.PI * k) / cats.length;
          const rr = (v) => (R * (v - lo)) / (hi - lo || 1);
          const g = [];
          const ticks = linearTicks(lo, hi, 4).filter((v) => v > lo);
          for (const tv of ticks) g.push(`<path d="${polyline(cats.map((_, k) => [cx + rr(tv) * Math.cos(ang(k)), cy + rr(tv) * Math.sin(ang(k))]))}Z" fill="none" stroke="${st.axes.grid_color}" stroke-width="${st.axes.grid_width}"/>`);
          cats.forEach((name, k) => {
            g.push(`<path d="M${r2(cx)} ${r2(cy)}L${r2(cx + R * Math.cos(ang(k)))} ${r2(cy + R * Math.sin(ang(k)))}" stroke="${st.axes.grid_color}" stroke-width="${st.axes.grid_width}"/>`);
            const t = c.T(name, c.font("tick"));
            const a = ang(k), lx = cx + (R + 3) * Math.cos(a), ly = cy + (R + 3) * Math.sin(a);
            g.push(t.place(lx, ly, { anchor: Math.abs(Math.cos(a)) < 0.2 ? "middle" : Math.cos(a) > 0 ? "start" : "end", valign: Math.sin(a) > 0.5 ? "top" : Math.sin(a) < -0.5 ? "bottom" : "middle" }));
          });
          const tf = c.font("tick");
          for (const tv of ticks) g.push(c.T(fmtNum(tv, p.yaxis?.fmt, ticks.step), { ...tf, size: tf.size * 0.85, color: "#666666" }).place(cx + 1.5, cy - rr(tv) - 1, {}));
          c.out.push(`<g data-part="grid">${g.join("")}</g>`);
          for (const r of ser) {
            const pts = cats.map((_, k) => [cx + rr(r.ys[k] ?? lo) * Math.cos(ang(k)), cy + rr(r.ys[k] ?? lo) * Math.sin(ang(k))]);
            const fill = p.opts?.fill !== false ? ` fill="${r.st.color}" fill-opacity="0.12"` : ` fill="none"`;
            const m = pts.map((q) => markerPath(r.st.marker, q[0], q[1], r.st.msize * 0.8, r.st.color)).join("");
            c.out.push(`<g data-part="series:${r.i}"><path d="${polyline(pts)}Z"${fill} stroke="${r.st.color}" stroke-width="${r.st.width}"${dashArray(r.st.dash, r.st.width)}/>${m}</g>`);
          }
        },
      };
    },
  };

  LorePlot.TYPES = Object.entries(CHARTS).map(([id, c]) => ({ id, label: c.label }));

  // ── axes, legend, notes ────────────────────────────────────────────────
  function axisTicks(ax, scale, conf, count) {
    if (scale.kind === "band") return ax.cats.map((c, i) => ({ v: i, text: c }));
    if (Array.isArray(conf?.ticks) && conf.ticks.length) {
      const nums = conf.ticks.map(num).filter(isNum);
      const step = nums.length > 1 ? Math.abs(nums[1] - nums[0]) : 0;
      return nums.map((v) => ({ v, text: fmtNum(v, conf.fmt, step) }));
    }
    const [a, b] = [Math.min(...scale.domain), Math.max(...scale.domain)];
    if (scale.kind === "log") {
      const out = [];
      for (let e = Math.floor(Math.log10(a)); e <= Math.ceil(Math.log10(b)); e++) {
        const v = 10 ** e;
        if (v >= a * 0.999 && v <= b * 1.001) out.push({ v, text: conf?.fmt && conf.fmt !== "e" ? fmtNum(v, conf.fmt) : `$10^{${e}}$` });
      }
      return out;
    }
    const t = linearTicks(a, b, count);
    return t.map((v) => ({ v, text: fmtNum(v, conf?.fmt, t.step) }));
  }
  const withUnit = (ax) => (ax?.label ? ax.label + (ax.unit ? ` (${ax.unit})` : "") : "");

  /** Legend block of `items` in font `f`: { w, h, draw(x, y) }. */
  function legendBlock(items, f, st, conf, T) {
    const cols = Math.max(1, conf?.cols ?? 1);
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
      draw(x, y) {
        const g = [];
        if (frame) g.push(`<rect x="${r2(x)}" y="${r2(y)}" width="${r2(this.w)}" height="${r2(h)}" fill="#ffffff" fill-opacity="0.9" stroke="${frame === "light" ? "#cccccc" : st.colors.axis}" stroke-width="${r2(Math.max(0.4, st.axes.line * 0.8))}" rx="${frame === "round" ? r2(fs * 0.3) : 0}"/>`);
        let y0 = y + pad;
        if (title) { g.push(title.place(x + pad, y0 + title.asc)); y0 += title.h + rowGap; }
        items.forEach((it, k) => {
          const ci = k % cols, ri = Math.floor(k / cols);
          const cx = x + pad + colW.slice(0, ci).reduce((a, cw) => a + handle + gap + cw + colGap, 0);
          const cy = y0 + ri * (rowH + rowGap) + rowH / 2;
          const sw = [];
          if (it.kind === "patch") sw.push(`<rect x="${r2(cx + handle * 0.15)}" y="${r2(cy - fs * 0.35)}" width="${r2(handle * 0.7)}" height="${r2(fs * 0.7)}" fill="${it.color}"${st.bars.edge > 0 ? ` stroke="${st.bars.edge_color || "#000"}" stroke-width="${st.bars.edge}"` : ""}/>`);
          else if (it.kind === "marker") sw.push(markerPath(it.marker ?? "o", cx + handle / 2, cy, Math.max(2.5, st.scatter.size * 1.1), it.color, { stroke: 0.3, edge: "#fff" }));
          else {
            sw.push(`<path d="M${r2(cx)} ${r2(cy)}L${r2(cx + handle)} ${r2(cy)}" stroke="${it.color}" stroke-width="${it.width ?? st.lines.width}"${dashArray(it.dash, it.width ?? 1)}/>`);
            if (it.marker && it.marker !== "none") sw.push(markerPath(it.marker, cx + handle / 2, cy, it.msize ?? st.lines.marker, it.color, { stroke: (it.width ?? 1) * 0.6 }));
          }
          const t = texts[k];
          g.push(`<g data-part="${it.part ?? ""}">${sw.join("")}${t.place(cx + handle + gap, cy, { valign: "middle" })}</g>`);
        });
        return `<g data-part="legend">${g.join("")}</g>`;
      },
    };
  }

  // ── a panel ────────────────────────────────────────────────────────────
  function fontOf(st, which) {
    const F = st.font;
    const size = which === "base" ? F.size : F[which] ?? F.size;
    return { family: fontCss(F.family), size, math: F.math, color: st.colors.text, weight: which === "title" ? (F.title_weight === "bold" ? 700 : 400) : which === "panel" ? (F.panel_weight === "bold" ? 700 : 400) : 400 };
  }

  function drawPanel(spec, p, pi, cell, st, macros, warnings, uid) {
    const T = (s, f) => measure(s, f, macros);
    const out = [];
    const chart = CHARTS[p.type] ?? CHARTS.line;
    if (!CHARTS[p.type]) warnings.push(`不認得的圖表種類：${p.type}`);
    const tbl = table(spec, p);
    let plan;
    try {
      plan = chart.prepare(p, tbl, st, {});
    } catch (e) {
      warnings.push(`第 ${pi + 1} 格：${e.message}`);
      return `<g data-part="panel:${pi}"></g>`;
    }
    const fTick = fontOf(st, "tick"), fLab = fontOf(st, "label"), fTitle = fontOf(st, "title"), fLeg = { ...fontOf(st, "legend"), size: p.legend?.size ?? st.legend.size ?? st.font.legend };
    const ax = st.axes;
    const pad = ax.pad ?? 3;
    const tickOut = ax.tick_dir === "out" ? ax.tick_len : 0;
    // panel label + title
    const plabel = panelLabel(spec, pi);
    const pl = plabel ? T(plabel, fontOf(st, "panel")) : null;
    const title = p.title ? T(p.title, fTitle) : null;
    let top = cell.y + Math.max(title ? title.h + pad : 0, pl ? pl.h + pad * 0.5 : 0) + (title || pl ? 0 : pad);
    let left = cell.x, right = cell.x + cell.w, bottom = cell.y + cell.h;
    // legend outside
    const legItems = p.legend?.show === false ? [] : plan.legend.filter((it) => it.label !== "");
    const lpos = p.legend?.pos ?? st.legend.pos ?? "best";
    const leg = legItems.length ? legendBlock(legItems, fLeg, st, { ...p.legend, cols: p.legend?.cols ?? (lpos === "below" || lpos === "above" ? Math.min(legItems.length, 4) : 1) }, T) : null;
    if (leg && lpos === "outside right") right -= leg.w + pad;
    if (leg && lpos === "below") bottom -= leg.h + pad;
    if (leg && lpos === "above") top += leg.h + pad;

    if (plan.polar) {
      const fT = fTick;
      const lab = Math.max(...plan.cats.map((c) => T(c, fT).w), 0);
      const R = Math.max(5, Math.min((right - left) / 2 - lab - 4, (bottom - top) / 2 - fT.size - 4));
      const cx = (left + right) / 2, cy = (top + bottom) / 2;
      const c = { out, T, font: (w) => fontOf(st, w), polar: { cx, cy, R }, inset: false };
      plan.draw(c);
      if (title) out.push(title.place(cx, cell.y + title.asc, { anchor: "middle", part: `title:${pi}` }));
      if (pl) out.push(pl.place(cell.x, cell.y + pl.asc, { part: `panellabel:${pi}` }));
      if (leg) {
        const pos = placeLegend(lpos === "best" ? "upper right" : lpos, leg, { x: left, y: top, w: right - left, h: bottom - top }, pad, cell);
        out.push(leg.draw(pos.x, pos.y));
      }
      return `<g data-part="panel:${pi}">${out.join("")}</g>`;
    }

    // ticks and labels: measure first
    const X = p.xaxis ?? {}, Y = p.yaxis ?? {};
    const xTicksN = Math.max(3, Math.round((cell.w / PT) / 14));
    const yTicksN = Math.max(3, Math.round((cell.h / PT) / 10));
    const mk = (a, conf, r0, r1) => {
      if (a.kind === "band") return bandScale(a.cats.length, r0, r1);
      const dom = domainOf(a.values, conf, { zero: a.zero, fixed: a.fixed, pad: a.zero ? 0.06 : 0.05, nice: a.zero });
      return conf?.log ? logScale(Math.max(dom[0], 1e-12), dom[1], r0, r1) : linScale(dom[0], dom[1], r0, r1);
    };
    // first pass with a guessed box to get tick texts
    let sx = mk(plan.x, X, 0, 1), sy = mk(plan.y, Y, 1, 0);
    if (plan.y.reverse && sy.kind === "band") sy = bandScale(plan.y.cats.length, 0, 1);
    const xt = X.hidden ? [] : axisTicks(plan.x, sx, X, xTicksN);
    const yt = Y.hidden ? [] : axisTicks(plan.y, sy, Y, yTicksN);
    const xTexts = xt.map((t) => T(t.text, fTick)), yTexts = yt.map((t) => T(t.text, fTick));
    let rot = X.rotate ?? 0;
    if (X.rotate == null && plan.x.kind === "band" && xTexts.length > 1) {
      const est = (cell.w - (Y.label ? fLab.size * 2 : 0) - fTick.size * 3) / xTexts.length;
      if (xTexts.some((t) => t.w > est * 0.92)) rot = 30;
    }
    const rad = (Math.abs(rot) * Math.PI) / 180;
    const xTickH = Math.max(0, ...xTexts.map((t) => (rot ? t.w * Math.sin(rad) + t.h * Math.cos(rad) : t.h)));
    const yTickW = Math.max(0, ...yTexts.map((t) => t.w));
    const xl = !X.hidden && withUnit(X) ? T(withUnit(X), fLab) : null;
    const yl = !Y.hidden && withUnit(Y) ? T(withUnit(Y), fLab) : null;
    const tp = pad * 0.6; // tick → label text
    const plotL = left + (yl ? yl.h + pad : 0) + yTickW + tp + tickOut + 0.5;
    const plotB = bottom - (xl ? xl.h + pad * 0.7 : 0) - xTickH - tp - tickOut - 0.5;
    const lastHalf = !rot && plan.x.kind !== "band" && xTexts.length ? xTexts.at(-1).w / 2 : 0;
    let plotR = right - Math.max(pad * 0.6, lastHalf - 2);
    let plotT = top + (yTexts.length ? yTexts.at(-1).h / 2 : 0);
    // colour bar on the right
    let cbar = null;
    if (plan.colorbar) {
      const cbt = linearTicks(plan.colorbar.vmin, plan.colorbar.vmax, 4).filter((v) => v >= plan.colorbar.vmin - 1e-9 && v <= plan.colorbar.vmax + 1e-9);
      const texts = cbt.map((v) => T(fmtNum(v, p.opts?.fmt === "d" ? "" : p.opts?.fmt, cbt.step), fTick));
      const bw = Math.max(4, st.font.size * 0.9);
      cbar = { cbt, texts, bw };
      plotR -= bw + pad + tickOut + tp + Math.max(0, ...texts.map((t) => t.w));
    }
    if (plan.x.top) { plotT += xTickH + tp; }
    const W = Math.max(10, plotR - plotL), H = Math.max(10, plotB - plotT);
    sx = mk(plan.x, X, plotL, plotL + W);
    sy = plan.y.kind === "band" ? bandScale(plan.y.cats.length, plotT, plotT + H) : mk(plan.y, Y, plotT + H, plotT);
    if (plan.y.kind === "band" && !plan.y.reverse) sy = bandScale(plan.y.cats.length, plotT + H, plotT);
    const box = { x: plotL, y: plotT, w: W, h: H };

    // grid
    const gridMode = p.grid ?? ax.grid;
    const gx = X.grid ?? (gridMode === "x" || gridMode === "both");
    const gy = Y.grid ?? (gridMode === "y" || gridMode === "both");
    const gs = ` stroke="${ax.grid_color}" stroke-width="${ax.grid_width}"${dashArray(ax.grid_dash, ax.grid_width)} fill="none"`;
    const grid = [];
    if (gx && sx.kind !== "band") grid.push(`<path d="${xt.map((t) => `M${r2(sx(t.v))} ${r2(plotT)}L${r2(sx(t.v))} ${r2(plotT + H)}`).join("")}"${gs}/>`);
    if (gy && sy.kind !== "band") grid.push(`<path d="${yt.map((t) => `M${r2(plotL)} ${r2(sy(t.v))}L${r2(plotL + W)} ${r2(sy(t.v))}`).join("")}"${gs}/>`);
    if (grid.length) out.push(`<g data-part="grid:${pi}">${grid.join("")}</g>`);

    // data, clipped to the plot area
    const clip = `${uid}c${pi}`;
    const valStep = (plan.horizontal ? xt : yt).length > 1 ? Math.abs((plan.horizontal ? xt : yt)[1].v - (plan.horizontal ? xt : yt)[0].v) / 10 : 0;
    out.push(`<clipPath id="${clip}"><rect x="${r2(plotL - 1)}" y="${r2(plotT - 1)}" width="${r2(W + 2)}" height="${r2(H + 2)}"/></clipPath>`);
    const dataOut = [];
    const ctx = { sx, sy, out: dataOut, T, font: (w) => fontOf(st, w), inset: false, valStep };
    plan.draw(ctx);
    out.push(`<g clip-path="url(#${clip})">${dataOut.join("")}</g>`);

    // notes (on top of data, not clipped)
    out.push(...notes(p, pi, ctx, plan, st, box));

    // spines and ticks
    const aw = ax.line, ink = st.colors.axis;
    const sp = p.spines ?? ax.spines ?? "lb";
    const sides = sp === "box" ? "lbrt" : sp === "none" ? "" : sp;
    const spine = [];
    if (!plan.heat) {
      if (sides.includes("l")) spine.push(`M${r2(plotL)} ${r2(plotT)}L${r2(plotL)} ${r2(plotT + H)}`);
      if (sides.includes("b")) spine.push(`M${r2(plotL)} ${r2(plotT + H)}L${r2(plotL + W)} ${r2(plotT + H)}`);
      if (sides.includes("r")) spine.push(`M${r2(plotL + W)} ${r2(plotT)}L${r2(plotL + W)} ${r2(plotT + H)}`);
      if (sides.includes("t")) spine.push(`M${r2(plotL)} ${r2(plotT)}L${r2(plotL + W)} ${r2(plotT)}`);
    }
    const tl = ax.tick_len, dir = ax.tick_dir === "in" ? -1 : 1;
    const ticks = [];
    const xAxisY = plan.x.top ? plotT : plotT + H;
    const xs = plan.x.top ? -1 : 1;
    xt.forEach((t, k) => {
      const x = sx(t.v);
      if (!(x >= plotL - 0.5 && x <= plotL + W + 0.5)) return;
      if (!plan.heat) ticks.push(`M${r2(x)} ${r2(xAxisY)}L${r2(x)} ${r2(xAxisY + xs * dir * tl)}`);
      const ty = xAxisY + xs * (tickOut + tp);
      const tx = xTexts[k];
      if (rot) out.push(tx.place(x, ty + tx.asc * Math.cos(rad) * 0.5, { anchor: "end", rotate: -Math.abs(rot), valign: "middle", part: `xaxis:${pi}` }));
      else out.push(tx.place(x, plan.x.top ? ty - tx.desc : ty + tx.asc, { anchor: "middle", part: `xaxis:${pi}` }));
    });
    yt.forEach((t, k) => {
      const y = sy(t.v);
      if (!(y >= plotT - 0.5 && y <= plotT + H + 0.5)) return;
      if (!plan.heat) ticks.push(`M${r2(plotL)} ${r2(y)}L${r2(plotL - dir * tl)} ${r2(y)}`);
      out.push(yTexts[k].place(plotL - tickOut - tp, y, { anchor: "end", valign: "middle", part: `yaxis:${pi}` }));
    });
    if (spine.length || ticks.length) out.push(`<path d="${spine.join("")}${ticks.join("")}" stroke="${ink}" stroke-width="${aw}" fill="none" stroke-linecap="square" data-part="axes:${pi}"/>`);
    if (xl) {
      const y = plan.x.top ? cell.y + xl.asc : plotB + tickOut + tp + xTickH + pad * 0.7 + xl.asc;
      out.push(xl.place(plotL + W / 2, plan.x.top ? plotT + H + tickOut + pad + xl.asc : y, { anchor: "middle", part: `xaxis:${pi}` }));
    }
    if (yl) out.push(yl.place(left + yl.asc, plotT + H / 2, { anchor: "middle", rotate: -90, part: `yaxis:${pi}` }));
    if (title) out.push(title.place(plotL + W / 2, cell.y + title.asc, { anchor: "middle", part: `title:${pi}` }));
    if (pl) out.push(pl.place(cell.x, cell.y + pl.asc, { part: `panellabel:${pi}` }));

    // colour bar
    if (cbar) {
      const x0 = plotL + W + pad, n = 40;
      const seg = [];
      for (let k = 0; k < n; k++) {
        const y0 = plotT + H - (H * (k + 1)) / n;
        seg.push(`<rect x="${r2(x0)}" y="${r2(y0)}" width="${r2(cbar.bw)}" height="${r2(H / n + 0.1)}" fill="${cmap(plan.colorbar.cmap, (k + 0.5) / n)}"/>`);
      }
      const cs = linScale(plan.colorbar.vmin, plan.colorbar.vmax, plotT + H, plotT);
      const tk = cbar.cbt.map((v) => `M${r2(x0 + cbar.bw)} ${r2(cs(v))}L${r2(x0 + cbar.bw + tl)} ${r2(cs(v))}`).join("");
      seg.push(`<rect x="${r2(x0)}" y="${r2(plotT)}" width="${r2(cbar.bw)}" height="${r2(H)}" fill="none" stroke="${ink}" stroke-width="${aw}"/><path d="${tk}" stroke="${ink}" stroke-width="${aw}"/>`);
      cbar.cbt.forEach((v, k) => seg.push(cbar.texts[k].place(x0 + cbar.bw + tl + tp, cs(v), { valign: "middle" })));
      out.push(`<g data-part="colorbar:${pi}">${seg.join("")}</g>`);
    }

    // inset (zoom)
    if (p.inset && !plan.heat && sx.kind !== "band") out.push(inset(p, pi, plan, st, box, T, uid));

    // legend
    if (leg) {
      let pos = lpos;
      if (pos === "best") pos = bestCorner(leg, plan.points, sx, sy, box, pad, p.inset);
      const at = placeLegend(pos, leg, box, pad, { x: cell.x, y: cell.y, w: cell.w, h: cell.h, right: right, bottom: bottom });
      out.push(leg.draw(at.x, at.y));
    }
    return `<g data-part="panel:${pi}">${out.join("")}</g>`;
  }

  function panelLabel(spec, i) {
    const mode = spec.grid?.labels ?? (spec.panels?.length > 1 ? "a" : "none");
    if (!mode || mode === "none" || (spec.panels?.length ?? 0) < 2 && !spec.grid?.labels) return "";
    const ch = String.fromCharCode(97 + i);
    return mode === "A" ? ch.toUpperCase() : mode === "(a)" ? `(${ch})` : mode === "a)" ? `${ch})` : ch;
  }

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
      case "above": return { x: box.x + (box.w - leg.w) / 2, y: cell.y + (cell.h - cell.h) };
      default: return { x: box.x + box.w - leg.w - m, y: box.y + m };
    }
  }
  function bestCorner(leg, points, sx, sy, box, pad, inset) {
    const cands = ["upper right", "upper left", "lower right", "lower left", "center right", "upper center", "lower center"];
    const scr = points.map((q) => [sx(q[0]), sy(q[1])]).filter((q) => isNum(q[0]) && isNum(q[1]));
    let best = cands[0], bestN = Infinity;
    const insetBox = inset?.at ? { x: box.x + inset.at[0] * box.w, y: box.y + (1 - inset.at[1] - inset.at[3]) * box.h, w: inset.at[2] * box.w, h: inset.at[3] * box.h } : null;
    cands.forEach((c, k) => {
      const at = placeLegend(c, leg, box, pad, box);
      let n = scr.filter((q) => q[0] >= at.x - 1 && q[0] <= at.x + leg.w + 1 && q[1] >= at.y - 1 && q[1] <= at.y + leg.h + 1).length;
      // a line crossing the box also counts: sample segments between points
      if (insetBox && !(at.x > insetBox.x + insetBox.w || at.x + leg.w < insetBox.x || at.y > insetBox.y + insetBox.h || at.y + leg.h < insetBox.y)) n += 1e6;
      n += k * 0.01;
      if (n < bestN) { bestN = n; best = c; }
    });
    return best;
  }

  function notes(p, pi, c, plan, st, box) {
    const out = [];
    const f = { ...fontOf(st, "base"), size: st.font.legend ?? st.font.size };
    const ink = st.colors.axis;
    const lw = r2(Math.max(0.5, st.axes.line));
    const X = (n, v) => (n.coords === "axes" ? box.x + v * box.w : c.sx(v));
    const Y = (n, v) => (n.coords === "axes" ? box.y + (1 - v) * box.h : c.sy(v));
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
        let yv = num(n.y);
        if (!isNum(yv) && plan.barAt && isNum(s1)) yv = Math.max(plan.barTop(a, s1), plan.barTop(b, s2)) * 1.04;
        if (!isNum(yv) && plan.tops) yv = Math.max(...plan.tops.slice(Math.min(a, b), Math.max(a, b) + 1).filter(isNum)) * 1.06;
        if (!isNum(yv)) return;
        const y = c.sy(yv), h = st.font.size * 0.4;
        const t = c.T(n.text ?? "*", nf);
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
      } else {
        const x = X(n, num(n.x)), y = Y(n, num(n.y));
        if (!isNum(x) || !isNum(y)) return;
        out.push(`<g data-part="${part}">${c.T(n.text ?? "", nf).place(x, y, { anchor: n.anchor ?? "middle", valign: "middle" })}</g>`);
      }
    });
    return out;
  }

  function inset(p, pi, plan, st, box, T, uid) {
    const at = p.inset.at ?? [0.45, 0.3, 0.5, 0.45];
    const ib = { x: box.x + at[0] * box.w, y: box.y + (1 - at[1] - at[3]) * box.h, w: at[2] * box.w, h: at[3] * box.h };
    const fx = p.inset.x ?? [0, 0.1], fy = p.inset.y ?? [0.8, 1];
    const fT = { ...fontOf(st, "tick"), size: st.font.tick * 0.85 };
    const yt = linearTicks(Math.min(...fy), Math.max(...fy), 2), xt = linearTicks(Math.min(...fx), Math.max(...fx), 2);
    const sx = linScale(fx[0], fx[1], ib.x, ib.x + ib.w), sy = linScale(fy[0], fy[1], ib.y + ib.h, ib.y);
    const g = [`<rect x="${r2(ib.x)}" y="${r2(ib.y)}" width="${r2(ib.w)}" height="${r2(ib.h)}" fill="#ffffff"/>`];
    const gs = ` stroke="${st.axes.grid_color}" stroke-width="${st.axes.grid_width}"${dashArray(st.axes.grid_dash, st.axes.grid_width)}`;
    if (st.axes.grid !== "none" || p.inset.grid) g.push(`<path d="${xt.map((v) => `M${r2(sx(v))} ${r2(ib.y)}L${r2(sx(v))} ${r2(ib.y + ib.h)}`).join("")}${yt.map((v) => `M${r2(ib.x)} ${r2(sy(v))}L${r2(ib.x + ib.w)} ${r2(sy(v))}`).join("")}"${gs}/>`);
    const clip = `${uid}i${pi}`;
    const d = [];
    plan.draw({ sx, sy, out: d, T, font: (w) => fontOf(st, w), inset: true });
    g.push(`<clipPath id="${clip}"><rect x="${r2(ib.x)}" y="${r2(ib.y)}" width="${r2(ib.w)}" height="${r2(ib.h)}"/></clipPath><g clip-path="url(#${clip})">${d.join("")}</g>`);
    g.push(`<rect x="${r2(ib.x)}" y="${r2(ib.y)}" width="${r2(ib.w)}" height="${r2(ib.h)}" fill="none" stroke="${st.colors.axis}" stroke-width="${st.axes.line}"/>`);
    const tl = st.axes.tick_len * 0.7;
    let tk = "";
    for (const v of xt) { tk += `M${r2(sx(v))} ${r2(ib.y + ib.h)}L${r2(sx(v))} ${r2(ib.y + ib.h + tl)}`; g.push(T(fmtNum(v, "", xt.step), fT).place(sx(v), ib.y + ib.h + tl + 1 + T("0", fT).asc, { anchor: "middle" })); }
    for (const v of yt.filter((v) => Math.abs(sy(v) - (ib.y + ib.h)) > 1)) { tk += `M${r2(ib.x)} ${r2(sy(v))}L${r2(ib.x - tl)} ${r2(sy(v))}`; g.push(T(fmtNum(v, "", yt.step), fT).place(ib.x - tl - 1, sy(v), { anchor: "end", valign: "middle" })); }
    g.push(`<path d="${tk}" stroke="${st.colors.axis}" stroke-width="${st.axes.line}"/>`);
    return `<g data-part="inset:${pi}">${g.join("")}</g>`;
  }

  // ── the figure ─────────────────────────────────────────────────────────
  let uidN = 0;
  /** Render a spec. opts: { styles, macros, style (resolved, wins) }. */
  LorePlot.render = async function (spec, opts = {}) {
    spec = spec ?? {};
    const st = opts.style ?? LorePlot.resolveStyle(spec.style, opts.styles, spec.set);
    await warm(spec, st, opts.macros);
    const warnings = [];
    const Wmm = spec.size?.w ?? st.size.w, Hmm = spec.size?.h ?? st.size.h;
    const W = Wmm * PT, H = Hmm * PT;
    const cols = Math.max(1, spec.grid?.cols ?? 1), rows = Math.max(1, spec.grid?.rows ?? Math.ceil((spec.panels?.length ?? 1) / cols));
    const gap = (spec.grid?.gap ?? 5) * PT;
    const margin = (spec.margin ?? 1) * PT;
    const cw = (W - margin * 2 - gap * (cols - 1)) / cols, ch = (H - margin * 2 - gap * (rows - 1)) / rows;
    const uid = `lp${++uidN}`;
    const body = [];
    let slot = 0;
    (spec.panels ?? []).forEach((p, i) => {
      const span = clamp(p.span ?? 1, 1, cols);
      if ((slot % cols) + span > cols) slot += cols - (slot % cols);
      const r = Math.floor(slot / cols), c = slot % cols;
      slot += span;
      if (r >= rows) { warnings.push(`第 ${i + 1} 格超出 ${rows}×${cols} 的版面`); return; }
      const cell = { x: margin + c * (cw + gap), y: margin + r * (ch + gap), w: cw * span + gap * (span - 1), h: ch };
      body.push(drawPanel(spec, p, i, cell, st, opts.macros, warnings, uid));
    });
    const bg = st.colors.background && st.colors.background !== "none" ? `<rect width="${r2(W)}" height="${r2(H)}" fill="${st.colors.background}" data-part="figure"/>` : "";
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${r2(Wmm)}mm" height="${r2(Hmm)}mm" viewBox="0 0 ${r2(W)} ${r2(H)}" text-rendering="geometricPrecision" data-lore-plot="1">${bg}${body.join("")}</svg>`;
    return { svg, w: Wmm, h: Hmm, warnings, style: st };
  };

  /** Font families an SVG uses (for embedding / checking). */
  LorePlot.fontsIn = (svg) => [...new Set([...svg.matchAll(/font-family="([^"]+)"/g)].flatMap((m) => m[1].split(",").map((s) => s.trim().replace(/^'|'$/g, ""))))];

  /** A fresh spec for a table (first column = x, the other numeric columns = series). */
  LorePlot.starter = function (type, tbl, style) {
    const cols = tbl?.columns ?? [];
    const numeric = cols.filter((c, i) => i > 0 && tbl.rows.some((r) => isNum(num(r[i]))));
    const errs = new Set(numeric.filter((c) => /(_sd|_std|_err|_se|_ci|±)$/i.test(c)));
    const ys = numeric.filter((c) => !errs.has(c));
    const series = ys.map((y) => {
      const e = [...errs].find((c) => c.replace(/(_sd|_std|_err|_se|_ci|±)$/i, "") === y);
      return e ? { y, err: e, label: y } : { y, label: y };
    });
    const p = { type, data: "t1", x: cols[0], series, xaxis: { label: cols[0] ?? "" }, yaxis: { label: "" }, legend: {}, opts: {}, notes: [] };
    if (type === "scatter") Object.assign(p, { x: ys[0] ?? cols[0], y: ys[1] ?? ys[0], group: cols.find((c, i) => !numeric.includes(c) && i >= 0) ?? null, series: [], xaxis: { label: ys[0] ?? "" }, yaxis: { label: ys[1] ?? "" } });
    if (type === "heatmap") Object.assign(p, { series: [], xaxis: { label: "" }, yaxis: { label: "" }, opts: { values: true } });
    if (type === "roc") Object.assign(p, { xaxis: { label: "False positive rate" }, yaxis: { label: "True positive rate" }, opts: { auc: true, diagonal: true }, legend: { pos: "lower right" } });
    if (type === "pr") Object.assign(p, { xaxis: { label: "Recall" }, yaxis: { label: "Precision" }, opts: { auc: true }, legend: { pos: "lower left" } });
    if (type === "bland") Object.assign(p, { series: [{ y: ys[0], y2: ys[1] }], xaxis: { label: "Mean of two methods" }, yaxis: { label: "Difference" } });
    if (type === "box" || type === "violin" || type === "hist") p.xaxis = { label: "" };
    return { lore_plot: 1, style: style ?? "clean", set: {}, size: null, grid: { cols: 1, rows: 1, gap: 5, labels: "none" }, data: { t1: tbl }, panels: [p], caption: "" };
  };

  window.LorePlot = LorePlot;
})();
