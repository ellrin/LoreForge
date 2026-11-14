// Equation figures (docs/plot.md §10): the `equation` chart, the GIF frames and the editor's
// equation blocks. Needs plot.js + eqcore.js (LoreEq) first; editor.js calls LorePlot.eqEditor.
//
//   panel: { type: "equation", eq: { template, expr, latex, tex, x, vars, output, reduction,
//            properties, animate, family, show, at_x, mark, ghost, ydom, badge } }
//   LorePlot.eq.starter(templateId, style)     → a new figure spec (catalogue: equations.json)
//   LorePlot.eq.frameSpecs(spec, n)            → n specs sweeping eq.animate.var (ping-pong, axes pinned)
//   LorePlot.eq.frameSvgs(spec, n, opts)       → their SVGs (opts as for LorePlot.render)
//   LorePlot.eq.gif(api, spec, opts)           → a GIF Blob (frames drawn here, encoded by POST <api>/gif)
// The figure takes colours only from its style (never the app theme).
(() => {
  "use strict";
  const LP = window.LorePlot;
  const E = window.LoreEq;
  if (!LP || !E) throw new Error("plot.js and eqcore.js must load before equation.js");
  const { r2, isNum, clone, markerPath, dashArray } = LP._;
  const CHARTS = LP.CHARTS;
  const N = 201;

  // ── catalogue ──
  let CAT = null;
  LP.eqCatalogue = async function () {
    if (CAT) return CAT;
    try { CAT = await fetch(LP.base + "equations.json").then((r) => (r.ok ? r.json() : { templates: [] })); } catch { CAT = { templates: [] }; }
    return CAT;
  };
  const astCache = new Map();
  function compile(eq, src) {
    const key = `${src ?? eq.expr}\u0000${E.eq.names(eq).join(",")}`;
    if (!astCache.has(key)) {
      if (astCache.size > 200) astCache.clear();
      try { astCache.set(key, { ast: E.eq.compile(eq, src) }); } catch (e) { astCache.set(key, { err: e.message }); }
    }
    const r = astCache.get(key);
    if (r.err) throw new Error(r.err);
    return r.ast;
  }
  LP.eqCompile = compile;
  /** Whether the panel's automatic y range cut a far tail (the axis's 自動 field says so). */
  LP.eqClipped = (p, st) => { try { return !!CHARTS.equation.prepare(p, null, st).clipped; } catch { return false; } };
  const texOf = (label) => String(label ?? "").replace(/^\$|\$$/g, "").replace(/（.*?）/g, "").trim();
  function decimals(v) {
    const st = Math.abs(v?.step ?? 0.01);
    if (v?.integer || st >= 1) return 0;
    return Math.min(4, Math.max(0, Math.ceil(-Math.log10(st) - 1e-9)));
  }
  const fmtVal = (v, x) => (x == null ? "" : (+x).toFixed(decimals(v)));

  // ── colour roles: each variable one stable colour everywhere it shows (LaTeX, slider, curve, bar, 3D axis).
  // The role is an index (eq.out_color, var.color; else output 0, the input on x 1, parameters 2, 3, …), mapped
  // onto the palette in force: the figure's style in exports, the app theme in the interactive view (eqlive.js).
  const ROLES = 6;
  function roleOf(eq, name) {
    if (name === "__out") return eq.out_color ?? 0;
    if (eq.x?.name === name) return eq.x.color ?? 1;
    const ps = (eq.vars ?? []).filter((v) => v.role === "param");
    const v = (eq.vars ?? []).find((q) => q.name === name);
    if (isNum(v?.color)) return v.color;
    const k = ps.indexOf(v);
    return k < 0 ? 1 : 2 + (k % (ROLES - 2));
  }
  /** name → colour (`__out` = the function itself) from a palette list. */
  function colorsFor(eq, palette) {
    const pick = (r) => palette[r % palette.length];
    const out = { __out: pick(roleOf(eq, "__out")) };
    if (eq.x) out[eq.x.name] = pick(roleOf(eq, eq.x.name));
    for (const v of eq.vars ?? []) if (v.role === "param" || v.role === "vector") out[v.name] = pick(roleOf(eq, v.name));
    return out;
  }
  /** [{ tex, color }] for colouring someone's LaTeX (labels are `$…$`). */
  const symbolsFor = (eq, colors) => [...(eq.x ? [eq.x] : []), ...(eq.vars ?? [])].filter((v) => colors[v.name] && texOf(v.label || v.name)).map((v) => ({ tex: texOf(v.label || v.name), color: colors[v.name] }));
  /** A label with `$…$`: each variable inside the maths in its colour. */
  const colorLabel = (str, syms) => String(str ?? "").replace(/\$([^$]+)\$/g, (_, m) => `$${E.colorTex(m, syms)}$`);
  LP.eqColors = { roleOf, colorsFor, symbolsFor, colorLabel, ROLES };
  /** The export palette: the style's line colours (else its palette), never the app theme. */
  const stylePalette = (st) => (st.colors.lines?.length ? st.colors.lines : st.colors.palette);
  LP.eqFmt = fmtVal;

  /** y values of what the panel shows (f, ∂f/∂x or ∂f/∂θ) at the current values + `over`. */
  function shown(eq, ast, xs, over) {
    const mode = eq.show ?? "f";
    if (mode === "f") return E.eq.curve(eq, ast, xs, over);
    if (mode === "dx") {
      const ys = E.eq.curve(eq, ast, xs, over);
      return ys.map((y, i) => {
        const a = ys[i - 1] ?? null, b = ys[i + 1] ?? null;
        if (a != null && b != null) return (b - a) / (xs[i + 1] - xs[i - 1]);
        if (y != null && b != null) return (b - y) / (xs[i + 1] - xs[i]);
        if (y != null && a != null) return (y - a) / (xs[i] - xs[i - 1]);
        return null;
      });
    }
    const name = mode.slice(2);
    return xs.map((x) => E.eq.partial(eq, ast, name, { ...over, [eq.x.name]: x }));
  }
  /** x ends: an open end next to a round number (0.001 → 0, 0.999 → 1) is shown as the round number. */
  function niceEnds(a, b) {
    const span = b - a;
    const step = LP.stats.niceStep(span, 5);
    const near = (v) => { const r = Math.round(v / step) * step; return Math.abs(r - v) <= 0.012 * span ? +r.toFixed(10) : v; };
    return [near(a), near(b)];
  }
  /** A robust extent: outliers near a log's pole do not flatten the rest. */
  function extent(vals) {
    const v = vals.filter(isNum).sort((a, b) => a - b);
    if (!v.length) return Object.assign([0, 1], { clipped: false });
    const q = (t) => v[Math.min(v.length - 1, Math.max(0, Math.round(t * (v.length - 1))))];
    let lo = v[0], hi = v.at(-1);
    const a = q(0.08), b = q(0.92);
    const mid = b - a;
    let clipped = false;
    // a pole (log p at p → 0, 1/p in a gradient) would flatten everything else: cut the far tail
    if (mid > 0 && hi - b > 0.6 * mid) { hi = b + 0.2 * mid; clipped = true; }
    if (mid > 0 && a - lo > 0.6 * mid) { lo = a - 0.2 * mid; clipped = true; }
    return Object.assign([lo, hi], { clipped });
  }
  LP.eqExtent = extent;
  LP.eqShown = (eq, ast, xs, over) => shown(eq, ast, xs, over);
  LP.eqPaperOver = (eq) => paperOver(eq);
  const differsFromPaper = (eq) => (eq.vars ?? []).some((v) => v.role === "param" && isNum(v.paper) && Math.abs(v.paper - +v.value) > 1e-12);
  const paperOver = (eq) => Object.fromEntries((eq.vars ?? []).filter((v) => v.role === "param" && isNum(v.paper)).map((v) => [v.name, v.paper]));
  function segments(pts) {
    const out = [];
    let cur = [];
    for (const p of pts) {
      if (isNum(p[0]) && isNum(p[1])) cur.push(p);
      else if (cur.length) { out.push(cur); cur = []; }
    }
    if (cur.length) out.push(cur);
    return out.map((s) => s.map((p, i) => `${i ? "L" : "M"}${r2(p[0])} ${r2(p[1])}`).join("")).join("");
  }

  // ── the chart ──
  CHARTS.equation = {
    label: "方程式", group: "equation",
    prepare(p, _tbl, st) {
      const eq = p.eq ?? {};
      if (eq.sim) {
        if (!LP.eqSim) throw new Error("eqsim.js 沒有載入");
        return LP.eqSim.chart(p, st);
      }
      const ast = compile(eq);
      const pal = stylePalette(st);
      const C = colorsFor(eq, pal);
      const syms = symbolsFor(eq, C);
      const main = C.__out ?? pal[0] ?? "#0072B2";
      // journal figures: text and axis labels stay uncoloured (docs/plot.md §2); slides colour-code them
      const slideLook = /slide/.test(st.id ?? "");
      const labels = !slideLook ? {} : { xlabel: p.xaxis?.label ? colorLabel(p.xaxis.label, syms) : undefined, ylabel: /^\$[^$]+\$$/.test(p.yaxis?.label ?? "") ? `$\\textcolor{${main}}{${texOf(p.yaxis.label)}}$` : p.yaxis?.label ? colorLabel(p.yaxis.label, syms) : undefined };
      const lw = st.lines.width;
      if (E.eq.bars(eq)) {
        const v = [].concat(E.evaluate(ast, E.eq.env(eq)));
        const cats = Array.isArray(eq.cats) && eq.cats.length === v.length ? eq.cats.map(String) : v.map((_, i) => String(i + 1));
        const ghost = eq.ghost !== false && differsFromPaper(eq) ? [].concat(E.evaluate(ast, E.eq.env(eq, paperOver(eq)))) : null;
        const fill = st.colors.single || st.colors.palette[0];
        return {
          x: { kind: "band", cats },
          y: { kind: "linear", values: [0, ...v, ...(ghost ?? [])], zero: true, fixed: eq.ydom },
          legend: [], zeroBase: true, ...labels,
          draw(c) {
            const bw = Math.abs(c.sx.step) * (st.bars.width ?? 0.8);
            const g = [];
            v.forEach((y, k) => {
              if (!isNum(y)) return;
              const x = c.sx(k), y0 = c.sy(0), y1 = c.sy(y);
              g.push(`<rect x="${r2(x - bw / 2)}" y="${r2(Math.min(y0, y1))}" width="${r2(bw)}" height="${r2(Math.abs(y0 - y1))}" fill="${fill}"/>`);
              if (ghost && isNum(ghost[k])) g.push(`<path d="M${r2(x - bw / 2)} ${r2(c.sy(ghost[k]))}L${r2(x + bw / 2)} ${r2(c.sy(ghost[k]))}" stroke="${st.colors.axis}" stroke-width="${r2(lw * 0.7)}"${dashArray("dash", lw * 0.7)}/>`);
              c.shapes?.rects.push({ x: x - bw / 2, y: Math.min(y0, y1), w: bw, h: Math.abs(y0 - y1) });
            });
            c.out.push(`<g data-part="series:0">${g.join("")}</g>`);
            badge(c, eq);
          },
        };
      }
      const xs = E.eq.xs(eq, N);
      const curves = [];
      const fam = eq.family?.on && eq.family.var && Array.isArray(eq.family.values) ? eq.family : null;
      if (fam) {
        const fv = E.eq.param(eq, fam.var);
        fam.values.forEach((val, i) => curves.push({ ys: shown(eq, ast, xs, { [fam.var]: val }), color: pal[i % pal.length], width: lw, label: `$${texOf(fv?.label) || fam.var} = ${+(+val).toFixed(decimals(fv))}$`, part: `series:${i}` }));
      } else {
        if ((eq.ghost === "always" && Object.keys(paperOver(eq)).length) || (eq.ghost !== false && differsFromPaper(eq))) {
          const an = E.eq.param(eq, eq.animate?.var);
          // the document's language: an English paper says "(paper)", a Chinese deck 「（論文）」
          const en = (eq.lang ?? (slideLook ? "zh" : "en")).startsWith("en");
          const sym = slideLook ? `\\textcolor{${C[an?.name] ?? main}}{${texOf(an?.label) || an?.name}}` : texOf(an?.label) || an?.name;
          const label = eq.ghost === "always" && an && isNum(an.paper) ? `$${sym} = ${+(+an.paper).toFixed(decimals(an))}$${en ? " (paper)" : "（論文）"}` : "";
          curves.push({ ys: shown(eq, ast, xs, paperOver(eq)), color: st.colors.other ?? "#b0b0b0", width: lw * 0.9, dash: "dash", label, part: "series:ghost" });
        }
        curves.push({ ys: shown(eq, ast, xs, {}), color: main, width: lw * 1.15, label: "", part: "series:0", main: true });
      }
      const ys = curves.flatMap((c) => c.ys);
      const ext = extent(ys);
      const [lo, hi] = ext;
      // the template's y range holds for the function itself, not for its derivatives
      const yfix = eq.ydom ?? ((eq.show ?? "f") === "f" && Array.isArray(eq.yrange) ? [isNum(eq.yrange[0]) ? eq.yrange[0] : Math.min(lo, 0), isNum(eq.yrange[1]) ? eq.yrange[1] : hi + (hi - lo) * 0.05] : undefined);
      return {
        x: { kind: "linear", values: [xs[0], xs.at(-1)], fixed: niceEnds(xs[0], xs.at(-1)) },
        y: { kind: "linear", values: [lo, hi], fixed: yfix ?? (ext.clipped ? [lo, hi] : undefined) },
        clipped: !yfix && ext.clipped,
        ...labels,
        legend: curves.filter((c) => c.label).map((c) => ({ label: c.label, kind: "line", color: c.color, width: c.width, dash: c.dash ?? "solid", marker: "none", part: c.part })),
        points: curves.flatMap((c) => c.ys.map((y, i) => [xs[i], y]).filter((q) => isNum(q[1]))).filter((_, i) => i % 8 === 0),
        draw(c) {
          for (const cv of curves) {
            const pts = cv.ys.map((y, i) => [c.sx(xs[i]), isNum(y) ? c.sy(y) : NaN]);
            c.out.push(`<g data-part="${cv.part}"><path d="${segments(pts)}" fill="none" stroke="${cv.color}" stroke-width="${r2(cv.width)}" stroke-linejoin="round" stroke-linecap="round"${dashArray(cv.dash ?? "solid", cv.width)}/></g>`);
            c.shapes?.lines.push(pts.filter((q) => isNum(q[0]) && isNum(q[1])));
          }
          // the point the sensitivity is read at
          if (eq.mark && isNum(eq.at_x)) {
            const mc = curves.find((q) => q.main) ?? curves[0];
            const x = c.sx(eq.at_x);
            const y = E.eq.value(eq, ast, { [eq.x.name]: eq.at_x });
            const g = [`<path d="M${r2(x)} ${r2(c.box.y)}L${r2(x)} ${r2(c.box.y + c.box.h)}" stroke="${st.colors.axis}" stroke-width="0.4"${dashArray("dot", 0.6)}/>`];
            if (isNum(y) && (eq.show ?? "f") === "f") g.push(markerPath("o", x, c.sy(y), st.lines.marker || 3.5, mc.color));
            c.out.push(`<g data-part="mark">${g.join("")}</g>`);
          }
          badge({ ...c, badgeColor: slideLook ? C[eq.animate?.var] : undefined }, eq);
        },
      };
    },
  };
  /** "γ = 2.00" in the corner of an animation frame (right-aligned: the digits stay put). */
  function badge(c, eq) {
    if (!eq.badge) return;
    const base = c.font("label");
    const f = { ...base, size: (base.size ?? 8) * 1.6, weight: "bold", ...(eq.badge_color ? { color: eq.badge_color } : c.badgeColor ? { color: c.badgeColor } : {}) };
    const t = c.T(eq.badge, f);
    const pad = 4;
    c.out.push(`<g data-part="badge">${t.place(c.box.x + c.box.w - pad, c.box.y + pad + t.asc, { anchor: "end" })}</g>`);
    c.shapes?.rects.push({ x: c.box.x + c.box.w - pad - t.w, y: c.box.y + pad, w: t.w, h: t.h });
  }

  // ── specs ──
  const EQ = (LP.eq = {});
  /** An equation figure on a slide (mm): with the slide style's 16 pt type it reads from the back row. */
  const SLIDE_W = 150, SLIDE_H = 95;
  /** Projection type for a slide equation figure (the figure's own `set`, editable). */
  const SLIDE_SET = { "font.tick": 16, "font.label": 18, "legend.size": 16, "axes.max_ticks": 5 };
  EQ.panelIndex = (spec) => (spec?.panels ?? []).findIndex((p) => p.type === "equation");
  /** The `eq` of a catalogue template (paper values not applied). */
  EQ.fromTemplate = function (t) {
    return {
      template: t.id, expr: t.expr, tex: t.latex, latex: "",
      ...(t.x && t.output !== "bars" ? { x: { ...clone(t.x), role: "x" } } : {}),
      ...(t.output === "bars" && t.x ? { xlabel: t.x.label } : {}),
      vars: clone(t.vars ?? []).map((v) => (v.role === "param" ? { ...v, paper: v.paper ?? v.value } : v)),
      ...(t.output ? { output: t.output } : {}),
      reduction: clone(t.reduction ?? []),
      properties: clone(t.properties ?? []),
      ...(t.animate ? { animate: { ...t.animate } } : {}),
      ...(t.family ? { family: { ...t.family, on: false } } : {}),
    };
  };
  /** A figure around an `eq` (one panel, no data). */
  EQ.specFor = function (eq, style, t) {
    const y = t?.y ?? {};
    const yaxis = { label: eq.y_label ?? y.label ?? "$f$" };
    if ((isNum(y.min) || isNum(y.max)) && !eq.yrange) eq.yrange = [y.min ?? null, y.max ?? null];
    const xlabel = E.eq.bars(eq) ? eq.xlabel ?? t?.x?.label ?? "" : eq.x?.label || (eq.x ? `$${eq.x.name}$` : "");
    const slide = /slide/.test(style ?? "");
    return { lore_plot: 1, kind: "equation", style, set: slide ? { ...SLIDE_SET } : {}, size: slide ? { w: SLIDE_W, h: SLIDE_H } : null, grid: { cols: 1, rows: 1, gap: 5, labels: "none" }, data: {}, panels: [{ type: "equation", eq, title: "", xaxis: { label: xlabel }, yaxis, legend: { pos: "best" }, opts: {}, notes: [] }], caption: "" };
  };
  EQ.starter = async function (id, style) {
    const cat = await LP.eqCatalogue();
    const t = cat.templates.find((x) => x.id === id) ?? cat.templates[0];
    return EQ.specFor(EQ.fromTemplate(t), style, t);
  };
  /** n frame specs: the animated parameter swept up and back, y and the legend pinned. */
  EQ.frameSpecs = function (spec, n = 24, valuesIn) {
    const pi = EQ.panelIndex(spec);
    if (pi < 0) throw new Error("不是方程式的圖");
    const p = spec.panels[pi];
    const eq = p.eq;
    const v = E.eq.param(eq, eq.animate?.var) ?? (eq.vars ?? []).find((q) => q.role === "param");
    if (!v) throw new Error("沒有可以動的參數");
    const hold = isNum(v.paper) ? 3 : 0;
    const values = valuesIn ?? EQ.sweepWithHold(v, n, hold);
    const st = LP.styleFor(spec.style, [], spec.set);
    // one y range over the whole sweep (the curve moves, the axes do not)
    let ydom = eq.ydom;
    if (!ydom && !(isNum(p.yaxis?.min) && isNum(p.yaxis?.max))) {
      const all = [];
      for (const val of E.eq.sweep(v, 13)) {
        try { all.push(...CHARTS.equation.prepare({ ...p, eq: { ...eq, vars: eq.vars.map((q) => (q.name === v.name ? { ...q, value: val } : q)), ghost: false } }, null, st).y.values); } catch {}
      }
      const [lo, hi] = isNum(eq.yrange?.[0]) && isNum(eq.yrange?.[1]) && (eq.show ?? "f") === "f" ? eq.yrange : extent(all);
      const pad = isNum(eq.yrange?.[1]) && (eq.show ?? "f") === "f" ? 0 : (hi - lo || 1) * 0.05;
      ydom = [isNum(p.yaxis?.min) ? p.yaxis.min : lo - pad, isNum(p.yaxis?.max) ? p.yaxis.max : hi + pad];
      if (E.eq.bars(eq)) ydom[0] = Math.min(0, ydom[0]);
    }
    return values.map((val) => {
      const s = clone(spec);
      const q = s.panels[pi];
      q.eq.vars = q.eq.vars.map((x) => (x.name === v.name ? { ...x, value: val } : x));
      q.eq.ydom = ydom;
      // the paper's curve stays in every frame with one steady legend line above the plot
      if (q.eq.ghost !== false) q.eq.ghost = "always";
      q.eq.badge = `$${texOf(v.label) || v.name} = ${fmtVal(v, val)}$`;
      const atPaper = isNum(v.paper) && Math.abs(val - v.paper) < 1e-12;
      if (atPaper) q.eq.badge_color = LP._.darker(st.colors.other ?? "#b0b0b0", 0.35);
      q.legend = { ...(q.legend ?? {}), pos: q.eq.family?.on ? q.legend?.pos ?? "best" : "above" };
      if (q.legend.pos === "best") q.legend.pos = "upper left";
      return s;
    });
  };
  /**
   * Within one equation every variable keeps a different role: the input first, then the parameters in order;
   * a role already taken (the function's 0, an earlier variable's) moves to the lowest free one.
   */
  EQ.distinctRoles = function (eq) {
    const used = new Set([LP.eqColors.roleOf(eq, "__out")]);
    for (const v of [...(eq.x ? [eq.x] : []), ...(eq.vars ?? []).filter((q) => q.role === "param")]) {
      let r = LP.eqColors.roleOf(eq, v.name);
      if (used.has(r)) {
        r = [...Array(LP.eqColors.ROLES).keys()].find((k) => !used.has(k)) ?? r;
        v.color = r;
      }
      used.add(r);
    }
    return eq;
  };
  /** Ping-pong values with the paper's value held `hold` frames each time the sweep crosses it. */
  EQ.sweepWithHold = function (v, n, hold) {
    const vals = E.eq.sweepFrames(v, n);
    if (!hold || !isNum(v.paper)) return vals;
    const out = [];
    vals.forEach((x, i) => {
      const prev = i ? vals[i - 1] : null;
      if (prev != null && ((prev < v.paper && x >= v.paper) || (prev > v.paper && x <= v.paper))) for (let k = 0; k < hold; k++) out.push(v.paper);
      out.push(x);
    });
    return out;
  };
  /**
   * GIF pacing for a sweep (docs/plot.md §10): the up-sweep's values spaced by how much the curve changes (75 %)
   * mixed with plain parameter steps (25 %), LoreSim.schedule — the same clock as simulation GIFs; then back down.
   * The paper's value is one frame held longer; both turning points are held. → { values, delays }
   */
  EQ.pacedSweep = function (spec, n = 24, ms = 80, holdMs = 700) {
    const p = spec.panels[EQ.panelIndex(spec)];
    const eq = p.eq;
    const v = E.eq.param(eq, eq.animate?.var) ?? (eq.vars ?? []).find((q) => q.role === "param");
    if (!v) throw new Error("沒有可以動的參數");
    n = Math.max(4, n);
    const up = Math.floor(n / 2) + 1;
    const dense = E.eq.sweep(v, Math.max(64, up * 4));
    let vals = E.eq.sweep(v, up);
    if (window.LoreSim) {
      const ast = compile(eq);
      const xs = E.eq.bars(eq) ? null : E.eq.xs(eq, 101);
      const ys = dense.map((val) => (xs ? shown(eq, ast, xs, { [v.name]: val }) : [].concat(E.evaluate(ast, E.eq.env(eq, { [v.name]: val })))));
      const all = ys.flat().filter(isNum);
      const [lo, hi] = extent(all);
      const span = hi - lo || 1;
      const diffs = ys.map((y, k) => (k ? y.reduce((s, yy, i) => s + (isNum(yy) && isNum(ys[k - 1][i]) ? Math.abs(Math.min(hi, Math.max(lo, yy)) - Math.min(hi, Math.max(lo, ys[k - 1][i]))) / span : 0), 0) : 0));
      vals = window.LoreSim.schedule(diffs, up, 0.25).map((k) => dense[k]);
    }
    const loop = [...vals, ...vals.slice(1, -1).reverse()].slice(0, n);
    const values = [], delays = [];
    loop.forEach((x, i) => {
      const prev = i ? loop[i - 1] : null;
      if (isNum(v.paper) && prev != null && ((prev < v.paper && x >= v.paper) || (prev > v.paper && x <= v.paper))) { values.push(v.paper); delays.push(ms * 4); }
      const d = i === 0 || i === up - 1 ? Math.max(ms, holdMs) : ms;
      // an integer parameter repeats values: one frame, shown that much longer
      if (values.length && values.at(-1) === x) { delays[delays.length - 1] += d; return; }
      values.push(x);
      delays.push(d);
    });
    return { values, delays };
  };
  /** Everything a GIF needs: SVG frames, per-frame delays, dithered rectangles (sims: the heatmap). */
  EQ.gifFrames = async function (spec, opts = {}) {
    const eq = spec.panels[EQ.panelIndex(spec)].eq;
    if (eq.sim && LP.eqSim) {
      const f = await LP.eqSim.frames(spec, opts);
      return { frames: f.svgs, delays: f.delays, dither: f.dither, ms: f.ms, maxW: f.maxW };
    }
    const ms = eq.animate?.ms ?? 80;
    const { values, delays } = EQ.pacedSweep(spec, eq.animate?.frames ?? 24, ms);
    const frames = [];
    for (const s of EQ.frameSpecs(spec, values.length, values)) frames.push((await LP.render(s, opts)).svg);
    return { frames, delays, dither: [], ms };
  };
  EQ.frameSvgs = async function (spec, n, opts = {}) {
    const out = [];
    for (const s of EQ.frameSpecs(spec, n)) out.push((await LP.render(s, opts)).svg);
    return out;
  };
  EQ.gif = async function (api, spec, opts = {}) {
    const f = await EQ.gifFrames(spec, opts);
    const r = await fetch(`${api}/gif`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ frames: f.frames, delays: f.delays, dither: f.dither, spec, dpi: opts.dpi ?? (spec.panels[EQ.panelIndex(spec)].eq.sim ? 150 : 200), ms: f.ms, ...(f.maxW ? { max_w: f.maxW } : {}) }) });
    if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error ?? r.statusText);
    return r.blob();
  };

  // ── the editor's equation blocks (editor.js passes its helpers in `k`) ──
  function texHtml(tex, display = true) {
    const el = document.createElement("div");
    el.className = "lp-eqtex";
    try { el.innerHTML = window.katex.renderToString(tex, { displayMode: display, throwOnError: false, output: "html" }); } catch { el.textContent = tex; }
    return el;
  }
  function mixedHtml(s) {
    const el = document.createElement("span");
    String(s ?? "").split(/(\$[^$]+\$)/).forEach((part) => {
      if (/^\$[^$]+\$$/.test(part) && window.katex) {
        const m = document.createElement("span");
        try { m.innerHTML = window.katex.renderToString(part.slice(1, -1), { throwOnError: false, output: "html" }); } catch { m.textContent = part; }
        el.append(m);
      } else el.append(part);
    });
    return el;
  }
  const sliderPos = (v, x) => {
    const a = +v.min, b = +v.max;
    if (v.log && a > 0) return (Math.log(x) - Math.log(a)) / (Math.log(b) - Math.log(a));
    return (x - a) / (b - a);
  };
  const sliderVal = (v, t) => {
    const a = +v.min, b = +v.max;
    let x = v.log && a > 0 ? Math.exp(Math.log(a) + t * (Math.log(b) - Math.log(a))) : a + t * (b - a);
    const st = +v.step || 0;
    if (v.integer) x = Math.round(x);
    else if (st > 0 && !v.log) x = Math.round(x / st) * st;
    else if (st > 0) x = +x.toFixed(decimals(v) + 1);
    return +x.toFixed(10);
  };

  const ED = (LP.eqEditor = {});
  /** The app theme's role colours (interactive view and the editor's own chrome only — never a figure file). */
  ED.themePalette = function () {
    const cs = getComputedStyle(document.documentElement);
    const t = (k, d) => cs.getPropertyValue(k).trim() || d;
    return [t("--accent-2", "#2de2e6"), t("--accent-3", "#ffd23f"), t("--accent", "#ff3fb4"), t("--ok", "#3ee8b5"), t("--warn", "#ff9f43"), t("--err", "#ff6b86")];
  };
  let checkTimer = null, checkSeq = 0;
  /** Left panel: the catalogue, a custom expression, a paper's LaTeX through the model. */
  ED.left = function (k) {
    const { h, ic, block, S } = k;
    const out = [];
    const b = block(null, "方程式", "eqpick");
    const grid = h("div", { class: "lp-types lp-eqtypes" });
    const cur = k.panel()?.eq?.template;
    LP.eqCatalogue().then((cat) => {
      for (const grp of [["loss", "損失"], ["act", "函數"]]) {
        grid.append(h("div", { class: "lp-gh lp-eqgrp" }, grp[1]));
        for (const t of cat.templates.filter((x) => x.group === grp[0])) {
          const btn = h("button", { type: "button", "aria-pressed": String(cur === t.id) }, h("span", {}, t.label));
          btn.onclick = () => k.useSpec(EQ.specFor(EQ.fromTemplate(t), S.spec?.style ?? S.defStyle, t), t.label);
          grid.append(btn);
        }
      }
      if (cat.scenes?.length && LP.eqSim) {
        grid.append(h("div", { class: "lp-gh lp-eqgrp" }, "模擬"));
        for (const t of cat.scenes) {
          const btn = h("button", { type: "button", "aria-pressed": String(cur === t.id) }, h("span", {}, t.label));
          btn.onclick = async () => k.useSpec(await LP.eqSim.starter(t.id, S.spec?.style ?? S.defStyle), t.label);
          grid.append(btn);
        }
      }
      const own = h("button", { type: "button", "aria-pressed": String(!!k.panel() && !cur) }, h("span", {}, "自訂"));
      own.onclick = () => k.useSpec(EQ.specFor({ expr: "x^2", x: { name: "x", label: "$x$", role: "x", min: -2, max: 2 }, vars: [], reduction: [], properties: [] }, S.spec?.style ?? S.defStyle), "自訂");
      grid.append(own);
    });
    b.body.append(grid);
    out.push(b);
    // from a paper's LaTeX (the equation-plot task)
    const fromTex = block(null, "從論文的式子", "eqtex");
    const ta = h("textarea", { class: "set-input lp-ask", rows: 3, placeholder: "LaTeX", spellcheck: "false" }, k.latex || "");
    const ctx = h("textarea", { class: "set-input lp-ask", rows: 2, placeholder: "上下文（選填）", spellcheck: "false" });
    const go = h("button", { type: "button", class: "primary small" }, ic("spark"), h("span", {}, "動起來"));
    go.disabled = !k.api;
    go.onclick = async () => {
      if (!ta.value.trim()) return ta.focus();
      go.disabled = true;
      go.lastChild.textContent = "想一下…";
      try {
        const r = await ED.propose(k.api, { latex: ta.value, context: ctx.value, style: S.spec?.style ?? S.defStyle });
        k.useSpec(r.spec, "從論文");
        if (r.engine) k.toast(r.engine);
      } catch (e) {
        if (e.draft) k.useSpec(e.draft, "從論文（草稿）");
        k.toast(e.message, { err: true, ms: 8000 });
      }
      go.disabled = false;
      go.lastChild.textContent = "動起來";
    };
    fromTex.body.append(ta, ctx, h("div", { class: "lp-row" }, go));
    out.push(fromTex);
    return out;
  };
  /**
   * Colour roles from the shared notation registry (templates < the author's global notation.json < the
   * document's notation, `notation` = its entries): written onto the spec's variables that have none, so the
   * same symbol keeps its colour in every app and every export. Without a server nothing changes.
   */
  ED.assignRoles = async function (api, spec, notation) {
    const p = spec?.panels?.[EQ.panelIndex(spec)];
    if (!api || !p?.eq) return spec;
    if (p.eq.sim) { await LP.eqSim?.assignRoles(api, spec, notation); return spec; }
    const vs = [...(p.eq.x ? [p.eq.x] : []), ...(p.eq.vars ?? []).filter((v) => v.role === "param")];
    if (vs.every((v) => isNum(v.color))) return spec;
    try {
      const r = await fetch(`${api}/equation/roles`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ symbols: vs.map((v) => texOf(v.label || v.name)), notation: notation ? { version: 1, entries: notation } : undefined }) });
      const j = await r.json();
      vs.forEach((v, i) => { if (!isNum(v.color) && isNum(j.roles?.[i])) v.color = j.roles[i]; });
    } catch {}
    EQ.distinctRoles(p.eq);
    return spec;
  };
  /** POST <api>/equation/propose → { spec, report, engine } (throws with .draft when unusable). */
  ED.propose = async function (api, body) {
    const r = await fetch(`${api}/equation/propose`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) { const e = new Error(j.error ?? r.statusText); e.draft = j.draft ?? null; throw e; }
    if (j.report) j.spec.panels[0].eq.checks = j.report;
    return j;
  };

  /** A simulation (eqsim.js): the run's length and the GIF; the terms, weights and target are edited in the view itself. */
  ED.simRight = function (k) {
    const { h, block, row } = k;
    const sim = k.panel().eq.sim;
    sim.opt ??= {};
    const b = block(null, "模擬", "eqsim");
    b.body.append(
      row("步數", k.number(() => sim.opt.steps ?? 300, (v) => (sim.opt.steps = v ? Math.max(10, Math.min(5000, Math.round(v))) : undefined), { step: 10, min: 10 })),
      row("學習率", k.number(() => sim.opt.lr, (v) => (sim.opt.lr = v > 0 ? Math.min(10, v) : undefined), { step: 0.005, min: 0 })),
      row("格數", k.number(() => sim.frames ?? 100, (v) => (sim.frames = v ? Math.max(10, Math.min(200, Math.round(v))) : undefined), { step: 10, min: 10 }), h("span", { class: "lp-x" }, "ms"), k.number(() => sim.ms ?? 70, (v) => (sim.ms = v ? Math.max(20, Math.round(v)) : undefined), { step: 10, min: 20 })));
    const gifB = h("button", { type: "button", class: "ghost" }, k.ic("download"), "GIF");
    gifB.disabled = !k.api;
    gifB.onclick = () => k.exportGif(gifB);
    b.body.append(h("div", { class: "lp-row" }, gifB));
    return [b];
  };
  /** Right panel blocks for an equation panel. */
  ED.right = function (k) {
    const { h, block, row, S, change } = k;
    const p = k.panel();
    const eq = p.eq;
    const out = [];
    if (eq.sim) return ED.simRight(k);
    // the side panel's maths needs KaTeX (the figure loads it only when its own text has maths)
    if (!window.katex && !ED.katexAsked) { ED.katexAsked = true; LP.loadKatex().then(() => k.refresh(true)).catch(() => {}); }
    let ast = null, err = null;
    try { ast = compile(eq); } catch (e) { err = e.message; }
    const labels = Object.fromEntries([...(eq.vars ?? []), ...(eq.x ? [eq.x] : [])].map((v) => [v.name, v.label]));
    const TC = colorsFor(eq, ED.themePalette());
    const TS = symbolsFor(eq, TC);

    // 算式: the paper's LaTeX | ours (from the parsed tree) | the reduction | the checks
    const bF = block(null, "算式", "eqexpr");
    const cmp = h("div", { class: "lp-eqcmp" });
    if (eq.latex) cmp.append(h("div", { class: "lp-eqrow" }, h("span", { class: "k" }, "論文"), texHtml(E.colorTex(eq.latex, TS), false)));
    const ours = h("div", { class: "lp-eqrow" }, h("span", { class: "k" }, "計算"));
    ours.append(err ? h("span", { class: "lp-warn" }, err) : texHtml(E.toLatex(ast, labels, TC), false));
    cmp.append(ours);
    bF.body.append(cmp);
    const code = h("textarea", { class: "set-input lp-eqcode", rows: 2, spellcheck: "false" }, eq.expr);
    code.oninput = () => {
      const src = code.value;
      try {
        E.eq.compile(eq, src);
        code.classList.remove("bad");
        change((sp) => { sp.panels[S.pi].eq.expr = src; delete sp.panels[S.pi].eq.template; }, "edit", code);
        ED.recheck(k);
      } catch (e) { code.classList.add("bad"); code.title = e.message; }
    };
    code.onblur = () => k.refresh(true);
    bF.body.append(code);
    if (eq.reduction?.length) bF.body.append(h("ul", { class: "lp-eqred" }, ...eq.reduction.map((r) => h("li", {}, mixedHtml(r)))));
    const checks = h("ul", { class: "lp-eqchecks", "data-eqchecks": "" });
    bF.body.append(checks);
    out.push(bF);
    ED.showChecks(checks, eq.checks);
    ED.recheck(k);

    // 參數: sliders (paper value marked), x range, fixed vectors
    const bP = block(null, "參數", "eqparams");
    for (const v of eq.vars ?? []) {
      const i = eq.vars.indexOf(v);
      if (v.role === "vector") {
        const inp = h("input", { class: "set-input", value: (v.value ?? []).join(", "), spellcheck: "false" });
        inp.oninput = () => {
          const arr = inp.value.split(/[,\s]+/).filter(Boolean).map(Number);
          if (arr.length && arr.every(Number.isFinite)) change((sp) => { sp.panels[S.pi].eq.vars[i].value = arr; }, "edit", inp);
        };
        bP.body.append(h("div", { class: "lp-eqp" }, h("span", { class: "lp-eqname", style: `color:${TC[v.name]}` }, mixedHtml(v.label || v.name)), inp));
        continue;
      }
      if (v.role !== "param") continue;
      const rng = h("input", { type: "range", min: "0", max: "1000", step: "1", class: "lp-eqslider", "aria-label": v.name, style: `accent-color:${TC[v.name]}` });
      rng.value = String(Math.round(sliderPos(v, +v.value) * 1000));
      const box = h("input", { class: "set-input lp-num lp-eqnum", type: "number", step: v.step ?? "any", value: fmtVal(v, v.value) });
      const set = (x, src) => {
        change((sp) => { sp.panels[S.pi].eq.vars[i].value = x; }, "edit", src);
        if (src !== box) box.value = fmtVal(v, x);
        if (src !== rng) rng.value = String(Math.round(sliderPos(v, x) * 1000));
        ED.updateSens(k);
        ED.recheck(k);
      };
      rng.oninput = () => set(sliderVal(v, +rng.value / 1000), rng);
      box.oninput = () => { const x = +box.value; if (box.value !== "" && Number.isFinite(x)) set(x, box); };
      const wrap = h("div", { class: "lp-eqslide" }, rng);
      if (isNum(v.paper) && +v.max > +v.min) {
        const mk = h("button", { type: "button", class: "lp-eqpaper", title: `論文 ${fmtVal(v, v.paper)}`, "aria-label": `回到論文的值 ${fmtVal(v, v.paper)}` });
        mk.style.left = `calc(${(sliderPos(v, v.paper) * 100).toFixed(2)}% - 4px)`;
        mk.onclick = () => set(v.paper, null);
        wrap.append(mk);
      }
      const lim = (key) => {
        const n = h("input", { class: "set-input lp-num lp-eqlim", type: "number", step: "any", value: v[key], "aria-label": key === "min" ? "最小" : "最大" });
        n.onchange = () => { const x = +n.value; if (n.value !== "" && Number.isFinite(x)) change((sp) => { sp.panels[S.pi].eq.vars[i][key] = x; }, "structure"); };
        return n;
      };
      bP.body.append(h("div", { class: "lp-eqp" },
        h("span", { class: "lp-eqname", style: `color:${TC[v.name]}` }, mixedHtml(v.label || v.name)), wrap, box),
        h("div", { class: "lp-eqlims" }, lim("min"), h("span", { class: "lp-x" }, "–"), lim("max")));
    }
    if (eq.x && !E.eq.bars(eq)) {
      const xr = (key) => k.number(() => eq.x[key], (x) => { if (isNum(x)) eq.x[key] = x; }, { step: "any" });
      bP.body.append(row(h("span", {}, mixedHtml(eq.x.label || eq.x.name)), xr("min"), h("span", { class: "lp-x" }, "到"), xr("max")));
    }
    out.push(bP);

    // 敏感度: at a point on x, each slider's normalised ∂f/∂θ; the curve can show ∂f/∂x or ∂f/∂θ
    const params = (eq.vars ?? []).filter((v) => v.role === "param");
    if (params.length || eq.x) {
      const bS = block(null, "敏感度", "eqsens");
      if (eq.x && !E.eq.bars(eq)) {
        const x0 = isNum(eq.at_x) ? eq.at_x : (+eq.x.min + +eq.x.max) / 2;
        const xs = h("input", { type: "range", min: "0", max: "1000", step: "1", class: "lp-eqslider", "aria-label": "點" });
        xs.value = String(Math.round(((x0 - eq.x.min) / (eq.x.max - eq.x.min)) * 1000));
        const xl = h("span", { class: "lp-eqat" }, `${eq.x.name} = ${x0.toPrecision(3)}`);
        xs.oninput = () => {
          const x = +eq.x.min + (+xs.value / 1000) * (eq.x.max - eq.x.min);
          change((sp) => { sp.panels[S.pi].eq.at_x = +x.toPrecision(6); }, "edit", xs);
          xl.textContent = `${eq.x.name} = ${x.toPrecision(3)}`;
          ED.updateSens(k);
        };
        bS.body.append(h("div", { class: "lp-eqp" }, xl, h("div", { class: "lp-eqslide" }, xs)),
          h("div", { class: "lp-row" }, k.check(() => eq.mark, (v) => { eq.mark = v || undefined; if (v && !isNum(eq.at_x)) eq.at_x = x0; }, "標在圖上")));
      }
      bS.body.append(h("div", { class: "lp-sens", "data-eqsens": "" }));
      if (eq.x && !E.eq.bars(eq)) {
        const opts = [["f", "f"], ["dx", `∂f/∂${eq.x.name}`], ...params.map((v) => [`d:${v.name}`, `∂f/∂${v.name}`])];
        bS.body.append(row("曲線", k.select(() => eq.show ?? "f", (v) => {
          const ya = (p.yaxis ??= {});
          if ((eq.show ?? "f") === "f") eq.ylabel_f = ya.label;
          eq.show = v === "f" ? undefined : v;
          const f = texOf(eq.ylabel_f) || "f";
          const by = v === "dx" ? texOf(eq.x.label) || eq.x.name : texOf(E.eq.param(eq, v.slice(2))?.label) || v.slice(2);
          ya.label = v === "f" ? eq.ylabel_f ?? ya.label : `$\\partial ${f} / \\partial ${by}$`;
        }, opts, { structure: true })));
      }
      out.push(bS);
    }

    // 動畫 + 曲線族
    if (params.length) {
      const bA = block(null, "動畫", "eqanim");
      const an = (eq.animate ??= { var: params[0].name });
      bA.body.append(
        row("參數", k.select(() => an.var, (v) => (an.var = v), params.map((v) => [v.name, v.name]), { structure: true })),
        row("格數", k.number(() => an.frames ?? 24, (v) => (an.frames = v ? Math.max(4, Math.min(120, Math.round(v))) : undefined), { step: 1, min: 4 }), h("span", { class: "lp-x" }, "ms"), k.number(() => an.ms ?? 80, (v) => (an.ms = v ? Math.max(20, Math.round(v)) : undefined), { step: 10, min: 20 })));
      const gifB = h("button", { type: "button", class: "ghost" }, k.ic("download"), "GIF");
      gifB.disabled = !k.api;
      gifB.onclick = () => k.exportGif(gifB);
      bA.body.append(h("div", { class: "lp-row" }, gifB, h("button", { type: "button", class: "ghost", onclick: () => ED.preview(k) }, k.ic("play"), "播放")));
      if (eq.family || params.length) {
        const fam = eq.family ?? { var: params[0].name, values: [] };
        bA.body.append(h("div", { class: "lp-row" }, k.check(() => eq.family?.on, (v) => {
          eq.family = { ...fam, on: v || undefined };
          if (v && !eq.family.values?.length) { const pv = E.eq.param(eq, fam.var); eq.family.values = E.eq.sweep(pv, 4).map((x) => +x.toFixed(decimals(pv))); }
        }, "曲線族")));
        if (eq.family?.on) {
          bA.body.append(
            row("參數", k.select(() => eq.family.var, (v) => { eq.family.var = v; const pv = E.eq.param(eq, v); eq.family.values = E.eq.sweep(pv, 4).map((x) => +x.toFixed(decimals(pv))); }, params.map((v) => [v.name, v.name]), { structure: true })),
            row("值", k.text(() => (eq.family.values ?? []).join(", "), (v) => { const a = v.split(/[,\s]+/).filter(Boolean).map(Number).filter(Number.isFinite); if (a.length) eq.family.values = a; })));
        }
      }
      out.push(bA);
    }
    setTimeout(() => ED.updateSens(k));
    return out;
  };

  ED.updateSens = function (k) {
    const el = k.right.querySelector("[data-eqsens]");
    const p = k.panel();
    if (!el || !p?.eq) return;
    const eq = p.eq;
    el.textContent = "";
    let ast;
    try { ast = compile(eq); } catch { return; }
    const x0 = eq.x && !E.eq.bars(eq) ? (isNum(eq.at_x) ? eq.at_x : (+eq.x.min + +eq.x.max) / 2) : 0;
    let s;
    try { s = E.eq.sensitivity(eq, ast, x0); } catch { return; }
    for (const r of s) {
      const w = Math.abs(r.norm) * 50;
      const bar = k.h("span", { class: `lp-sbar${eq.animate?.var === r.name ? " anim" : ""}` });
      bar.style.background = colorsFor(eq, ED.themePalette())[r.name];
      bar.style.width = `${w.toFixed(1)}%`;
      bar.style[r.norm < 0 ? "right" : "left"] = "50%";
      el.append(k.h("div", { class: "lp-srow" },
        k.h("span", { class: "lp-sname", style: `color:${colorsFor(eq, ED.themePalette())[r.name]}` }, mixedHtml(E.eq.param(eq, r.name)?.label || r.name)),
        k.h("span", { class: "lp-strack" }, k.h("span", { class: "lp-szero" }), bar),
        k.h("span", { class: "lp-sval", title: `∂f/∂${r.name} = ${r.d.toPrecision(4)}` }, (r.norm >= 0 ? "+" : "−") + Math.abs(r.norm).toFixed(2))));
    }
  };
  ED.showChecks = function (ul, rep) {
    if (!ul) return;
    ul.textContent = "";
    if (!rep) return;
    for (const p of rep.problems ?? []) ul.append(Object.assign(document.createElement("li"), { className: "bad", textContent: `✗ ${p}` }));
    for (const c of rep.checks ?? []) {
      const li = document.createElement("li");
      li.className = c.ok ? "ok" : "bad";
      li.append(c.ok ? "✓ " : "✗ ", mixedHtml(c.text));
      if (!c.ok && c.detail) li.append(Object.assign(document.createElement("span"), { className: "lp-eqdetail", textContent: ` ${c.detail}` }));
      ul.append(li);
    }
  };
  /** The server's checks (parse, grid, properties), debounced; nothing without a server. */
  ED.recheck = function (k) {
    if (!k.api) return;
    clearTimeout(checkTimer);
    checkTimer = setTimeout(async () => {
      const my = ++checkSeq;
      const p = k.panel();
      if (!p?.eq) return;
      const eq = { ...p.eq };
      for (const key of ["checks", "ydom", "badge"]) delete eq[key];
      try {
        const r = await fetch(`${k.api}/equation/check`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ eq }) });
        const j = await r.json();
        if (my !== checkSeq || !r.ok) return;
        ED.showChecks(k.right.querySelector("[data-eqchecks]"), j);
      } catch {}
    }, 350);
  };
  /** Play the sweep in the editor (frames drawn here, no server). */
  ED.preview = async function (k) {
    const spec = k.S.spec;
    const an = spec.panels[k.S.pi].eq.animate ?? {};
    let specs;
    try { specs = EQ.frameSpecs(spec, an.frames ?? 24); } catch (e) { return k.toast(e.message, { err: true }); }
    // the interactive view plays it in place (morphing from frame to frame)
    const lv = k.live?.();
    if (lv) {
      for (let loop = 0; loop < 2; loop++) for (const s2 of specs) { lv.update(s2); await new Promise((ok) => setTimeout(ok, Math.max(an.ms ?? 80, 60))); }
      lv.update(k.S.spec);
      return;
    }
    const svgs = [];
    for (const s of specs) svgs.push((await LP.render(s, { styles: k.S.styleList, macros: k.macros })).svg);
    const sheet = k.sheet;
    const keep = sheet.innerHTML;
    const w = sheet.firstElementChild?.style.width, hh = sheet.firstElementChild?.style.height;
    for (let loop = 0; loop < 2; loop++) {
      for (const svg of svgs) {
        sheet.innerHTML = svg;
        Object.assign(sheet.firstElementChild.style, { width: w, height: hh });
        await new Promise((ok) => setTimeout(ok, an.ms ?? 80));
      }
    }
    sheet.innerHTML = keep;
  };
})();
