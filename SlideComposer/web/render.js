// Slide renderer: canvas model (lore-canvas JSON) → DOM/SVG. Used by the editor
// canvas, the slide sorter thumbnails and the print route (PDF).
// Units: 1 px = 9525 EMU (96 dpi); font sizes in pt.

const EMU_PX = 9525;
const px = (e) => e / EMU_PX;
const ptpx = (pt) => (pt * 96) / 72;
const SVGNS = "http://www.w3.org/2000/svg";

// Font stacks: fonts named in the decks → what this machine has.
const FONT_MAP = {
  "微軟正黑體": ["Microsoft JhengHei", "微軟正黑體", "PingFang TC", "Noto Sans TC"],
  "Microsoft JhengHei": ["Microsoft JhengHei", "PingFang TC", "Noto Sans TC"],
  "新細明體": ["PMingLiU", "新細明體", "LiSong Pro", "PingFang TC"],
  "Calibri": ["Calibri", "Carlito", "PT Sans", "Helvetica Neue", "Arial"],
  "Calibri Light": ["Calibri Light", "Calibri", "Carlito", "PT Sans", "Helvetica Neue", "Arial"],
  "Cambria Math": ["Cambria Math", "STIX Two Math", "Latin Modern Math", "Times New Roman", "serif"],
  "Times New Roman": ["Times New Roman", "Times", "serif"],
};
// The user's own fallbacks (private/shared/fonts.toml → /prefs.js) win: company or
// licensed fonts are named there, never here.
Object.assign(FONT_MAP, window.SC_PREFS?.fonts?.fallback ?? {});
// Fonts neither table knows (e.g. a template's brand font) fall back like Calibri.
const LATIN_FALLBACK = ["Calibri", "Carlito", "PT Sans", "Helvetica Neue", "Arial"];
function fontStack(look) {
  const out = [];
  for (const f of [look.font, look.ea_font]) {
    if (!f) continue;
    for (const x of FONT_MAP[f] ?? [f, ...LATIN_FALLBACK]) if (!out.includes(x)) out.push(x);
  }
  out.push("PingFang TC", "Noto Sans TC", "sans-serif");
  return out.map((f) => (/^[a-z-]+$/i.test(f) && !f.includes(" ") ? f : `"${f}"`)).join(", ");
}

function rgba(hex, alpha = 1) {
  if (!hex || hex.length !== 6) return hex ? `#${hex}` : "transparent";
  const n = parseInt(hex, 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${alpha})`;
}

function el(tag, cls, style) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (style) Object.assign(e.style, style);
  return e;
}
function svg(tag, attrs) {
  const e = document.createElementNS(SVGNS, tag);
  for (const [k, v] of Object.entries(attrs ?? {})) e.setAttribute(k, v);
  return e;
}

// ── Preset geometry → SVG path (in px, box w × h) ─────────────────────────
const KNOWN_GEOMS = new Set(["rect", "roundRect", "ellipse", "triangle", "rtTriangle", "trapezoid", "parallelogram", "diamond",
  "pentagon", "homePlate", "chevron", "hexagon", "octagon", "plus", "star5", "rightArrow", "leftArrow", "upArrow", "downArrow",
  "leftRightArrow", "line", "straightConnector1", "bentConnector2", "bentConnector3", "bentConnector4", "curvedConnector2",
  "curvedConnector3", "curvedConnector4", "flowChartProcess", "flowChartDecision", "flowChartTerminator", "flowChartAlternateProcess",
  "snip1Rect", "round2SameRect", "custom", "can", "frame", "donut", "wedgeRectCallout", "wedgeRoundRectCallout", "cloud", "teardrop",
  "leftBracket", "rightBracket", "leftBrace", "rightBrace"]);

function geomPath(prst, w, h, adj = {}, custom) {
  const ss = Math.min(w, h);
  const a = (k, d) => (adj[k] ?? d) / 100000;
  const poly = (pts) => "M" + pts.map((p) => p.join(" ")).join(" L") + " Z";
  switch (prst) {
    case "custom": return null; // handled by caller (custom path in 0..1)
    case "roundRect": case "flowChartAlternateProcess": {
      const r = Math.min(ss * a("adj", 16667), w / 2, h / 2);
      return `M${r} 0 H${w - r} A${r} ${r} 0 0 1 ${w} ${r} V${h - r} A${r} ${r} 0 0 1 ${w - r} ${h} H${r} A${r} ${r} 0 0 1 0 ${h - r} V${r} A${r} ${r} 0 0 1 ${r} 0 Z`;
    }
    case "round2SameRect": {
      const r = Math.min(ss * a("adj1", 16667), w / 2, h / 2);
      return `M${r} 0 H${w - r} A${r} ${r} 0 0 1 ${w} ${r} V${h} H0 V${r} A${r} ${r} 0 0 1 ${r} 0 Z`;
    }
    case "flowChartTerminator": {
      const r = h / 2;
      return `M${r} 0 H${w - r} A${r} ${r} 0 0 1 ${w - r} ${h} H${r} A${r} ${r} 0 0 1 ${r} 0 Z`;
    }
    case "snip1Rect": { const s = ss * a("adj", 16667); return poly([[0, 0], [w - s, 0], [w, s], [w, h], [0, h]]); }
    case "ellipse": case "donut": return `M0 ${h / 2} A${w / 2} ${h / 2} 0 1 0 ${w} ${h / 2} A${w / 2} ${h / 2} 0 1 0 0 ${h / 2} Z`;
    case "triangle": return poly([[w * a("adj", 50000), 0], [w, h], [0, h]]);
    case "rtTriangle": return poly([[0, 0], [w, h], [0, h]]);
    case "trapezoid": { const o = ss * a("adj", 25000); return poly([[0, h], [o, 0], [w - o, 0], [w, h]]); }
    case "parallelogram": { const o = ss * a("adj", 25000); return poly([[o, 0], [w, 0], [w - o, h], [0, h]]); }
    case "diamond": case "flowChartDecision": return poly([[w / 2, 0], [w, h / 2], [w / 2, h], [0, h / 2]]);
    case "pentagon": case "homePlate": { const o = ss * a("adj", 50000); return poly([[0, 0], [w - o, 0], [w, h / 2], [w - o, h], [0, h]]); }
    case "chevron": { const o = ss * a("adj", 50000); return poly([[0, 0], [w - o, 0], [w, h / 2], [w - o, h], [0, h], [o, h / 2]]); }
    case "hexagon": { const o = ss * a("adj", 25000); return poly([[o, 0], [w - o, 0], [w, h / 2], [w - o, h], [o, h], [0, h / 2]]); }
    case "octagon": { const o = ss * a("adj", 29289); return poly([[o, 0], [w - o, 0], [w, o], [w, h - o], [w - o, h], [o, h], [0, h - o], [0, o]]); }
    case "plus": { const o = ss * a("adj", 25000); return poly([[o, 0], [w - o, 0], [w - o, o], [w, o], [w, h - o], [w - o, h - o], [w - o, h], [o, h], [o, h - o], [0, h - o], [0, o], [o, o]]); }
    case "star5": {
      const pts = [];
      for (let i = 0; i < 10; i++) {
        const r = i % 2 ? 0.38 : 0.5, t = -Math.PI / 2 + (i * Math.PI) / 5;
        pts.push([w / 2 + r * w * Math.cos(t), h / 2 + r * h * Math.sin(t)]);
      }
      return poly(pts);
    }
    case "rightArrow": { const t = h * a("adj1", 50000), hd = ss * a("adj2", 50000); const y0 = (h - t) / 2; return poly([[0, y0], [w - hd, y0], [w - hd, 0], [w, h / 2], [w - hd, h], [w - hd, h - y0], [0, h - y0]]); }
    case "leftArrow": { const t = h * a("adj1", 50000), hd = ss * a("adj2", 50000); const y0 = (h - t) / 2; return poly([[w, y0], [hd, y0], [hd, 0], [0, h / 2], [hd, h], [hd, h - y0], [w, h - y0]]); }
    case "upArrow": { const t = w * a("adj1", 50000), hd = ss * a("adj2", 50000); const x0 = (w - t) / 2; return poly([[x0, h], [x0, hd], [0, hd], [w / 2, 0], [w, hd], [w - x0, hd], [w - x0, h]]); }
    case "downArrow": { const t = w * a("adj1", 50000), hd = ss * a("adj2", 50000); const x0 = (w - t) / 2; return poly([[x0, 0], [x0, h - hd], [0, h - hd], [w / 2, h], [w, h - hd], [w - x0, h - hd], [w - x0, 0]]); }
    case "leftRightArrow": { const t = h * a("adj1", 50000), hd = ss * a("adj2", 50000); const y0 = (h - t) / 2; return poly([[0, h / 2], [hd, 0], [hd, y0], [w - hd, y0], [w - hd, 0], [w, h / 2], [w - hd, h], [w - hd, h - y0], [hd, h - y0], [hd, h]]); }
    case "can": { const ry = ss * a("adj", 25000) / 2; return `M0 ${ry} A${w / 2} ${ry} 0 0 1 ${w} ${ry} V${h - ry} A${w / 2} ${ry} 0 0 1 0 ${h - ry} Z M0 ${ry} A${w / 2} ${ry} 0 0 0 ${w} ${ry}`; }
    case "frame": { const t = ss * a("adj1", 12500); return `M0 0 H${w} V${h} H0 Z M${t} ${t} V${h - t} H${w - t} V${t} Z`; }
    case "wedgeRectCallout": case "wedgeRoundRectCallout": case "cloud": case "teardrop":
    case "rect": case "flowChartProcess": return `M0 0 H${w} V${h} H0 Z`;
    case "leftBracket": return `M${w} 0 Q0 0 0 ${Math.min(w, h / 2)} V${h - Math.min(w, h / 2)} Q0 ${h} ${w} ${h}`;
    case "rightBracket": return `M0 0 Q${w} 0 ${w} ${Math.min(w, h / 2)} V${h - Math.min(w, h / 2)} Q${w} ${h} 0 ${h}`;
    case "leftBrace": return `M${w} 0 Q${w / 2} 0 ${w / 2} ${h / 4} V${h / 2 - w / 2} Q${w / 2} ${h / 2} 0 ${h / 2} Q${w / 2} ${h / 2} ${w / 2} ${h / 2 + w / 2} V${(3 * h) / 4} Q${w / 2} ${h} ${w} ${h}`;
    case "rightBrace": return `M0 0 Q${w / 2} 0 ${w / 2} ${h / 4} V${h / 2 - w / 2} Q${w / 2} ${h / 2} ${w} ${h / 2} Q${w / 2} ${h / 2} ${w / 2} ${h / 2 + w / 2} V${(3 * h) / 4} Q${w / 2} ${h} 0 ${h}`;
    // lines / connectors (open paths)
    case "line": case "straightConnector1": return `M0 0 L${w} ${h}`;
    case "bentConnector2": return `M0 0 H${w} V${h}`;
    case "bentConnector3": { const x = w * a("adj1", 50000); return `M0 0 H${x} V${h} H${w}`; }
    case "bentConnector4": { const x = w * a("adj1", 50000), y = h * a("adj2", 50000); return `M0 0 H${x} V${y} H${w} V${h}`; }
    case "curvedConnector2": return `M0 0 C${w} 0 ${w} 0 ${w} ${h}`;
    case "curvedConnector3": { const x = w * a("adj1", 50000); return `M0 0 C${x} 0 ${x} ${h} ${w} ${h}`; }
    case "curvedConnector4": { const x = w * a("adj1", 50000), y = h * a("adj2", 50000); return `M0 0 C${x} 0 ${x} ${y} ${(x + w) / 2} ${y} S${w} ${y} ${w} ${h}`; }
    default: return `M0 0 H${w} V${h} H0 Z`;
  }
}
const OPEN_GEOMS = new Set(["line", "straightConnector1", "bentConnector2", "bentConnector3", "bentConnector4", "curvedConnector2", "curvedConnector3", "curvedConnector4", "leftBracket", "rightBracket", "leftBrace", "rightBrace"]);

function dashArray(dash, w) {
  const d = { dash: [4, 3], sysDash: [3, 1], sysDot: [1, 1], dot: [1, 2], lgDash: [8, 3], dashDot: [4, 3, 1, 3], sysDashDot: [3, 1, 1, 1], lgDashDot: [8, 3, 1, 3], lgDashDotDot: [8, 3, 1, 3, 1, 3], sysDashDotDot: [3, 1, 1, 1, 1, 1] }[dash];
  return d ? d.map((x) => x * Math.max(w, 1)).join(" ") : null;
}

let markerSeq = 0;
function marker(defs, kind, color, start) {
  if (!kind || kind === "none") return null;
  const id = `mk${++markerSeq}`;
  const m = svg("marker", { id, viewBox: "0 0 10 10", refX: kind === "arrow" ? 8 : 6, refY: 5, markerWidth: 4, markerHeight: 4, orient: start ? "auto-start-reverse" : "auto", markerUnits: "strokeWidth", overflow: "visible" });
  let s;
  switch (kind) {
    case "oval": s = svg("circle", { cx: 5, cy: 5, r: 4, fill: color }); m.setAttribute("refX", 5); break;
    case "diamond": s = svg("path", { d: "M5 0 L10 5 L5 10 L0 5 Z", fill: color }); m.setAttribute("refX", 5); break;
    case "arrow": s = svg("path", { d: "M0 0 L9 5 L0 10", fill: "none", stroke: color, "stroke-width": 1.6 }); break;
    case "stealth": s = svg("path", { d: "M0 0 L10 5 L0 10 L3 5 Z", fill: color }); break;
    default: s = svg("path", { d: "M0 0 L10 5 L0 10 Z", fill: color });
  }
  m.append(s);
  defs.append(m);
  return `url(#${id})`;
}

function paintCss(p, defs, w, h) {
  if (!p || p.kind === "none") return "none";
  if (p.kind === "solid") return rgba(p.color, p.alpha ?? 1);
  if (p.kind === "gradient" && defs) {
    const id = `gr${++markerSeq}`;
    const ang = ((p.angle ?? 0) * Math.PI) / 180;
    const g = p.path
      ? svg("radialGradient", { id })
      : svg("linearGradient", { id, x1: 0.5 - Math.cos(ang) / 2, y1: 0.5 - Math.sin(ang) / 2, x2: 0.5 + Math.cos(ang) / 2, y2: 0.5 + Math.sin(ang) / 2 });
    for (const s of p.stops) g.append(svg("stop", { offset: s.pos, "stop-color": `#${s.color}`, "stop-opacity": s.alpha ?? 1 }));
    defs.append(g);
    return `url(#${id})`;
  }
  if (p.kind === "other") return "rgba(200,200,200,.4)";
  return "none";
}

// ── Text ───────────────────────────────────────────────────────────────────
const NUM_FMT = {
  arabicPeriod: (n) => `${n}.`, arabicParenR: (n) => `${n})`, arabicParenBoth: (n) => `(${n})`, arabicPlain: (n) => `${n}`,
  alphaLcPeriod: (n) => `${String.fromCharCode(96 + n)}.`, alphaUcPeriod: (n) => `${String.fromCharCode(64 + n)}.`,
  alphaLcParenR: (n) => `${String.fromCharCode(96 + n)})`, alphaUcParenR: (n) => `${String.fromCharCode(64 + n)})`,
  romanLcPeriod: (n) => `${roman(n).toLowerCase()}.`, romanUcPeriod: (n) => `${roman(n)}.`,
  circleNumDbPlain: (n) => String.fromCharCode(0x2460 + n - 1),
};
function roman(n) {
  const t = [[10, "X"], [9, "IX"], [5, "V"], [4, "IV"], [1, "I"]];
  let s = "";
  for (const [v, r] of t) while (n >= v) { s += r; n -= v; }
  return s;
}

function runStyle(span, look, scale) {
  span.style.fontFamily = fontStack(look);
  let size = look.size * scale;
  if (look.baseline) { size *= 0.7; span.style.verticalAlign = look.baseline > 0 ? "super" : "sub"; }
  span.style.fontSize = `${size}pt`;
  if (look.bold) span.style.fontWeight = "700";
  if (look.italic) span.style.fontStyle = "italic";
  const deco = [look.underline && "underline", look.strike && "line-through"].filter(Boolean).join(" ");
  if (deco) span.style.textDecoration = deco;
  span.style.color = rgba(look.color, look.alpha ?? 1);
}

/** Render one paragraph's runs into `p` (editable: spans carry indices). */
function renderRuns(p, para, pi, ctx, scale, editable) {
  para.runs.forEach((r, ri) => {
    let span;
    if (r.kind === "br") {
      span = document.createElement("br");
    } else if (r.kind === "math") {
      span = el("span", "math");
      span.contentEditable = "false";
      runStyle(span, r.look, scale);
      // pandoc sometimes leaves a dangling "\" at the end
      const tex = (r.latex ?? ctx.latex?.(r.omml))?.replace(/\\\s*$/, "");
      if (tex && window.katex) {
        try { katex.render(tex, span, { throwOnError: false, displayMode: false }); }
        catch { span.textContent = r.text; }
      } else {
        span.textContent = r.text || "∎";
        span.classList.add("pending");
        ctx.needLatex?.(r.omml);
      }
    } else {
      span = el("span", "r");
      runStyle(span, r.look, scale);
      let t = r.text;
      if (r.kind === "field" && r.field === "slidenum" && ctx.slideNo) t = String(ctx.slideNo);
      span.textContent = t;
      if (r.kind === "raw") span.contentEditable = "false";
    }
    if (editable) { span.dataset.pi = pi; span.dataset.ri = ri; }
    p.append(span);
  });
}

function renderText(tb, ctx, editable = false, cellMode = false) {
  const box = el("div", "tx");
  const [l, t, r, b] = tb.insets;
  Object.assign(box.style, { paddingLeft: `${px(l)}px`, paddingTop: `${px(t)}px`, paddingRight: `${px(r)}px`, paddingBottom: `${px(b)}px` });
  box.style.justifyContent = { t: "flex-start", ctr: "center", b: "flex-end" }[tb.anchor] ?? "flex-start";
  if (!tb.wrap) box.classList.add("nowrap");
  if (tb.vert && tb.vert !== "horz") box.style.writingMode = tb.vert === "eaVert" || tb.vert === "vert" ? "vertical-rl" : "vertical-lr";
  if (cellMode) box.classList.add("cell-tx");
  const scale = tb.font_scale ?? 1;
  const red = tb.ln_reduction ?? 0;
  const counters = [];
  const body = el("div", "tb");
  tb.paras.forEach((para, pi) => {
    const p = el("div", "p");
    if (editable) p.dataset.pi = pi;
    const first = para.runs.find((x) => x.kind !== "br") ?? null;
    const look = first?.look ?? para.end ?? { size: 18, font: "Calibri", color: "000000" };
    p.style.textAlign = { l: "left", ctr: "center", r: "right", just: "justify", dist: "justify" }[para.align] ?? "left";
    if (para.align === "dist") p.style.textAlignLast = "justify";
    p.style.paddingLeft = `${px(para.mar_l ?? 0)}px`;
    p.style.textIndent = `${px(para.indent ?? 0)}px`;
    if (para.line_pt) p.style.lineHeight = `${para.line_pt * scale}pt`;
    else p.style.lineHeight = String(1.2 * (para.line ?? 1) * (1 - red));
    if (para.before_pt) p.style.marginTop = `${para.before_pt}pt`;
    if (para.after_pt) p.style.marginBottom = `${para.after_pt}pt`;
    p.style.fontSize = `${look.size * scale}pt`;
    p.style.fontFamily = fontStack(look);
    // bullets / numbering (counters per level, reset by a lower-level paragraph)
    const lvl = para.level ?? 0;
    counters.length = Math.min(counters.length, lvl + 1);
    const bu = para.bullet ?? { kind: "none" };
    const hasText = para.runs.some((x) => (x.text ?? "").length || x.kind === "math");
    if (bu.kind !== "none" && hasText) {
      const s = el("span", "bu");
      s.contentEditable = "false";
      if (bu.kind === "num") {
        counters[lvl] = (counters[lvl] ?? (bu.start ?? 1) - 1) + 1;
        s.textContent = (NUM_FMT[bu.scheme] ?? NUM_FMT.arabicPeriod)(counters[lvl]);
      } else {
        s.textContent = bu.ch ?? "•";
      }
      runStyle(s, { ...look, bold: false, underline: false, baseline: 0 }, scale);
      const ind = -(para.indent ?? 0);
      if (ind > 0) s.style.minWidth = `${px(ind)}px`;
      s.style.display = "inline-block";
      s.style.textIndent = "0";
      p.append(s);
    } else if (bu.kind === "num") {
      counters[lvl] = counters[lvl] ?? (bu.start ?? 1) - 1;
    }
    renderRuns(p, para, pi, ctx, scale, editable);
    if (!para.runs.length || para.runs.every((x) => x.kind === "text" && !x.text)) {
      const z = document.createElement("br");
      p.append(z);
    }
    body.append(p);
  });
  box.append(body);
  return box;
}

// ── Objects ────────────────────────────────────────────────────────────────
function frameStyle(e, f) {
  Object.assign(e.style, { left: `${px(f.x)}px`, top: `${px(f.y)}px`, width: `${px(f.w)}px`, height: `${px(f.h)}px` });
  if (f.rot) e.style.transform = `rotate(${f.rot}deg)`;
}

function strokeAttrs(line) {
  if (!line) return { stroke: "none" };
  const w = ptpx(line.width_pt);
  const a = { stroke: rgba(line.color, line.alpha ?? 1), "stroke-width": w, "stroke-linejoin": "round" };
  const d = dashArray(line.dash, w);
  if (d) a["stroke-dasharray"] = d;
  return a;
}

function geomSvg(o, sp, w, h, flip) {
  const s = svg("svg", { width: Math.max(w, 1), height: Math.max(h, 1), class: "geom", overflow: "visible" });
  if (flip) s.style.transform = flip;
  const defs = svg("defs");
  s.append(defs);
  const prst = sp.geom.prst;
  let path;
  if (prst === "custom" && sp.geom.path) {
    path = svg("path", { d: sp.geom.path, transform: `scale(${Math.max(w, 0.01)} ${Math.max(h, 0.01)})`, "vector-effect": "non-scaling-stroke" });
  } else {
    path = svg("path", { d: geomPath(prst, w, h, sp.geom.adj ?? {}) });
  }
  const open = OPEN_GEOMS.has(prst);
  path.setAttribute("fill", open ? "none" : paintCss(sp.fill, defs, w, h));
  for (const [k, v] of Object.entries(strokeAttrs(sp.line))) path.setAttribute(k, v);
  if (sp.line) {
    const c = rgba(sp.line.color, sp.line.alpha ?? 1);
    const m1 = marker(defs, sp.line.head, c, true);
    const m2 = marker(defs, sp.line.tail, c, false);
    if (m1) path.setAttribute("marker-start", m1);
    if (m2) path.setAttribute("marker-end", m2);
  }
  s.append(path);
  if (!KNOWN_GEOMS.has(prst)) s.dataset.unknown = prst;
  return s;
}

function flipCss(f) {
  if (!f.flip_h && !f.flip_v) return "";
  return `scale(${f.flip_h ? -1 : 1}, ${f.flip_v ? -1 : 1})`;
}

/** Render an object (recursively for groups). */
function renderObject(o, ctx) {
  const f = o.frame;
  const e = el("div", `obj t-${o.type}`);
  // layout (backdrop) objects get their own attribute: their ids can collide with slide objects
  if (ctx.backdrop) e.dataset.bid = o.id;
  else e.dataset.id = o.id;
  frameStyle(e, f);
  if (o.hidden) e.classList.add("is-hidden");
  if (o.ph) {
    e.dataset.ph = o.ph;
    const empty = !o.text || o.text.paras.every((p) => !p.runs.some((r) => (r.text ?? "").length || r.kind === "math"));
    if (empty) e.classList.add("ph-empty");
  }
  const w = px(f.w), h = px(f.h);
  switch (o.type) {
    case "text":
    case "shape": {
      if (o.fill?.kind !== "none" || o.line || o.type === "shape") e.append(geomSvg(o, o, w, h, flipCss(f)));
      if (o.text) {
        const t = renderText(o.text, ctx, ctx.editing === o.id);
        if (f.flip_v) t.style.transform = "rotate(180deg)";
        e.append(t);
      }
      if (o.equation) e.classList.add("equation");
      break;
    }
    case "connector": {
      const sp = { geom: { prst: o.geom, adj: o.adj ?? {} }, fill: { kind: "none" }, line: o.line };
      e.append(geomSvg(o, sp, w, h, flipCss(f)));
      break;
    }
    case "picture": {
      const clip = el("div", "pic");
      if (o.clip === "ellipse") clip.style.borderRadius = "50%";
      else if (o.clip === "roundRect") clip.style.borderRadius = `${Math.min(w, h) * 0.16667}px`;
      const c = o.crop ?? { l: 0, t: 0, r: 0, b: 0 };
      const fw = 1 - c.l - c.r, fh = 1 - c.t - c.b;
      const img = el("img");
      img.draggable = false;
      img.src = ctx.assetUrl(o.asset);
      Object.assign(img.style, { width: `${w / (fw || 1)}px`, height: `${h / (fh || 1)}px`, left: `${(-c.l * w) / (fw || 1)}px`, top: `${(-c.t * h) / (fh || 1)}px` });
      clip.style.opacity = o.alpha ?? 1;
      const flip = flipCss(f);
      if (flip) clip.style.transform = flip;
      clip.append(img);
      e.append(clip);
      if (o.line) {
        const b = el("div", "pic-line");
        b.style.border = `${ptpx(o.line.width_pt)}px ${o.line.dash === "solid" ? "solid" : "dashed"} ${rgba(o.line.color, o.line.alpha ?? 1)}`;
        b.style.margin = `${-ptpx(o.line.width_pt) / 2}px`;
        if (o.clip === "ellipse") b.style.borderRadius = "50%";
        e.append(b);
      }
      for (const m of o.masks ?? []) {
        const mk = el("div", "mask", { left: `${m.x * w}px`, top: `${m.y * h}px`, width: `${m.w * w}px`, height: `${m.h * h}px` });
        e.append(mk);
      }
      break;
    }
    case "group": {
      const inner = el("div", "grp");
      const sx = o.ch.w ? f.w / o.ch.w : 1, sy = o.ch.h ? f.h / o.ch.h : 1;
      inner.style.transform = `${flipCss(f) ? flipCss(f) + " " : ""}scale(${sx}, ${sy}) translate(${-px(o.ch.x)}px, ${-px(o.ch.y)}px)`;
      if (flipCss(f)) inner.style.transformOrigin = "center";
      for (const c of o.children) inner.append(renderObject(c, ctx));
      e.append(inner);
      break;
    }
    case "table": {
      e.append(renderTable(o, ctx));
      break;
    }
    case "locked": {
      e.classList.add("locked");
      e.append(el("span", "locked-label")).textContent = `🔒 ${o.kind}`;
      break;
    }
  }
  return e;
}

function renderTable(o, ctx) {
  const t = el("table", "tbl");
  const grid = !o.style_id || o.style_id === "{5940675A-B579-460E-94D1-54222C63F5DA}";
  const cg = el("colgroup");
  for (const w of o.cols) cg.append(el("col", null, { width: `${px(w)}px` }));
  t.append(cg);
  t.style.width = `${px(o.cols.reduce((a, b) => a + b, 0))}px`;
  o.rows.forEach((row, ri) => {
    const tr = el("tr");
    tr.style.height = `${px(row.h)}px`;
    row.cells.forEach((c, ci) => {
      if (c.hmerge || c.vmerge) return;
      const td = el("td");
      td.dataset.r = ri; td.dataset.c = ci;
      if (c.span > 1) td.colSpan = c.span;
      if (c.rspan > 1) td.rowSpan = c.rspan;
      if (c.fill) td.style.background = `#${c.fill}`;
      else if (!grid && o.first_row && ri === 0) td.style.background = "#4472C4";
      else if (!grid && o.band_row && ri % 2 === 1) td.style.background = "#CFD5EA";
      const sides = ["Left", "Right", "Top", "Bottom"];
      (c.borders ?? []).forEach((b, i) => {
        if (b) td.style[`border${sides[i]}`] = b.width_pt > 0 ? `${ptpx(b.width_pt)}px solid ${rgba(b.color, b.alpha ?? 1)}` : "none";
        else if (grid) td.style[`border${sides[i]}`] = "1px solid #000";
      });
      const editing = ctx.editingCell && ctx.editingCell.id === o.id && ctx.editingCell.r === ri && ctx.editingCell.c === ci;
      td.append(renderText(c.text, ctx, editing, true));
      tr.append(td);
    });
    t.append(tr);
  });
  return t;
}

/** Render a whole slide into a fixed-size element (W × H px). */
function renderSlide(slide, ctx) {
  const W = px(ctx.size.w), H = px(ctx.size.h);
  const root = el("div", "slide", { width: `${W}px`, height: `${H}px` });
  const bd = ctx.backdrops?.find((b) => b.layout === slide.layout);
  const bg = bd?.bg;
  root.style.background = bg?.kind === "solid" ? rgba(bg.color, bg.alpha ?? 1) : "#fff";
  if (bd) {
    const back = el("div", "backdrop");
    const bctx = { ...ctx, editing: null, editingCell: null, backdrop: true };
    for (const o of bd.objects) back.append(renderObject(o, bctx));
    root.append(back);
  }
  const layer = el("div", "objects");
  for (const o of slide.objects) layer.append(renderObject(o, ctx));
  root.append(layer);
  return root;
}

/** Problems a renderer cannot draw faithfully (for the acceptance report). */
function renderWarnings(slide) {
  const w = [];
  const visit = (o) => {
    if (o.type === "locked") w.push(`${o.id} locked:${o.kind}`);
    if ((o.type === "shape" || o.type === "text") && o.geom && !KNOWN_GEOMS.has(o.geom.prst)) w.push(`${o.id} geom:${o.geom.prst}`);
    if (o.type === "connector" && !KNOWN_GEOMS.has(o.geom)) w.push(`${o.id} conn:${o.geom}`);
    if (o.type === "picture" && (o.effects ?? []).length) w.push(`${o.id} pic-effects:${o.effects.join("+")}`);
    if ((o.type === "shape" || o.type === "text") && o.fill?.kind === "other") w.push(`${o.id} fill:${o.fill.label}`);
    if (o.type === "group") o.children.forEach(visit);
  };
  slide.objects.forEach(visit);
  return w;
}

window.SC = { EMU_PX, px, ptpx, rgba, fontStack, renderSlide, renderObject, renderText, renderWarnings, geomPath, KNOWN_GEOMS };
