// The interactive view of an equation figure (docs/plot.md §10): what the author plays with inside an app
// (the editor's 互動 view, LoreReader 動起來). It follows the Lore THEME — dark, glowing curves, each
// variable in its role colour (LorePlot.eqColors over the theme palette) — while every exported file
// (GIF, PNG, PDF) keeps the figure's own style (equation.js). Canvas 2D, no dependencies.
//
//   const v = LorePlot.eqLive.mount(host, { onPoint(x) })   // host: an element that gets the canvas
//   v.update(spec)        // new values → the curves morph there (eased), the point and its tangent glide
//   v.mode("2d" | "3d")   // 3D: f over (input, the animated parameter), orbit by dragging, contours, marbles
//   v.marbles()           // drop marbles that roll down the surface along −∇f
//   v.destroy()
// Frames are drawn only while something moves (morph, drag, marbles): requestAnimationFrame on demand.
// LorePlot.eqLive.stats = { frames, ms } (mean frame time while animating) for checks.
(() => {
  "use strict";
  const LP = window.LorePlot;
  const E = window.LoreEq;
  if (!LP?.eq || !E) throw new Error("equation.js must load before eqlive.js");
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
  const lerp = (a, b, t) => a + (b - a) * t;
  const N = 201;
  const GREEK = { alpha: "α", beta: "β", gamma: "γ", delta: "δ", epsilon: "ε", varepsilon: "ε", lambda: "λ", mu: "μ", sigma: "σ", tau: "τ", theta: "θ", eta: "η", rho: "ρ", phi: "φ", omega: "ω", pi: "π" };
  /** A label for canvas text (no KaTeX there): `$\gamma$` → γ, `$p_t$` → p_t. */
  const plain = (label, name) => {
    const t = String(label || name || "").replace(/\$/g, "").replace(/（.*?）/g, "").replace(/\\([a-zA-Z]+)/g, (_, g) => GREEK[g] ?? g).replace(/[{}]/g, "").replace(/\\,/g, "").trim();
    return t || name;
  };
  const stats = { frames: 0, ms: 0 };

  function themeTok() {
    const cs = getComputedStyle(document.documentElement);
    const t = (k, d) => cs.getPropertyValue(k).trim() || d;
    return { bg: t("--bg", "#111"), panel: t("--panel", "#181818"), line: t("--line", "#333"), strong: t("--line-strong", "#555"), text: t("--text", "#eee"), muted: t("--muted", "#aaa"), faint: t("--faint", "#777"), mono: t("--mono", "ui-monospace, monospace") };
  }
  const rgba = (c, a) => {
    const m = /^#?([0-9a-f]{6})$/i.exec(String(c).trim());
    if (!m) return c;
    const n = parseInt(m[1], 16);
    return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
  };
  const hex3 = (c) => {
    const m = /^#?([0-9a-f]{6})$/i.exec(String(c).trim());
    if (m) { const n = parseInt(m[1], 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; }
    const r = /rgba?\(([^)]+)\)/.exec(String(c));
    return r ? r[1].split(",").slice(0, 3).map((x) => +x) : [200, 200, 200];
  };
  const escHtml = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const katexHtml = (tex) => {
    try { return window.katex ? window.katex.renderToString(tex, { throwOnError: false, output: "html" }) : escHtml(tex); } catch { return escHtml(tex); }
  };

  function mount(host, opts = {}) {
    host.textContent = "";
    const wrap = document.createElement("div");
    wrap.className = "lp-live";
    const cv = document.createElement("canvas");
    const head = document.createElement("div");
    head.className = "lp-live-head";
    const read = document.createElement("div");
    read.className = "lp-live-read";
    const xlab = document.createElement("div");
    xlab.className = "lp-live-x";
    const ylab = document.createElement("div");
    ylab.className = "lp-live-y";
    const ax3 = { x: document.createElement("div"), p: document.createElement("div"), f: document.createElement("div") };
    for (const el of Object.values(ax3)) el.className = "lp-live-ax3";
    wrap.append(cv, head, read, xlab, ylab, ax3.x, ax3.p, ax3.f);
    host.append(wrap);
    const ctx = cv.getContext("2d");
    const S = { spec: null, eq: null, ast: null, mode: "2d", cur: null, from: null, to: null, t0: 0, raf: 0, drag: null, yaw: -0.75, pitch: 0.55, grid: null, marbles: [], W: 0, H: 0, dpr: 1, tok: themeTok(), C: {}, lastKey: "" };

    function size() {
      const r = wrap.getBoundingClientRect();
      const W = Math.max(320, Math.floor(r.width)), H = Math.max(240, Math.floor(r.height));
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      if (W !== S.W || H !== S.H || dpr !== S.dpr) {
        Object.assign(S, { W, H, dpr });
        cv.width = W * dpr;
        cv.height = H * dpr;
        cv.style.width = `${W}px`;
        cv.style.height = `${H}px`;
      }
    }
    const ro = new ResizeObserver(() => { size(); kick(); });
    ro.observe(wrap);

    // ── the target state for the current spec ──
    function target() {
      const eq = S.eq;
      const p = S.spec.panels[LP.eq.panelIndex(S.spec)];
      if (LP.eqColors && eq) S.C = LP.eqColors.colorsFor(eq, LP.eqEditor.themePalette());
      if (E.eq.bars(eq)) {
        const v = [].concat(E.evaluate(S.ast, E.eq.env(eq)));
        return { bars: v, dom: [0, Math.max(1e-9, ...v.map((x) => (isNum(x) ? x : 0))) * 1.08] };
      }
      const xs = E.eq.xs(eq, N);
      const curves = [];
      const ghostOn = Object.keys(LP.eqPaperOver(eq)).length && (eq.vars ?? []).some((v) => isNum(v.paper) && Math.abs(v.paper - +v.value) > 1e-12);
      if (ghostOn) curves.push({ ys: LP.eqShown(eq, S.ast, xs, LP.eqPaperOver(eq)), ghost: true });
      else curves.push({ ys: LP.eqShown(eq, S.ast, xs, {}), ghost: true, hidden: true });
      curves.push({ ys: LP.eqShown(eq, S.ast, xs, {}) });
      const fixed = (eq.show ?? "f") === "f" && Array.isArray(eq.yrange) && isNum(eq.yrange[1]) ? [isNum(eq.yrange[0]) ? eq.yrange[0] : 0, eq.yrange[1]] : null;
      const ext = fixed ?? LP.eqExtent(curves.filter((c) => !c.hidden).flatMap((c) => c.ys));
      const pad = fixed ? 0 : (ext[1] - ext[0] || 1) * 0.06;
      const x0 = isNum(eq.at_x) ? eq.at_x : (+eq.x.min + +eq.x.max) / 2;
      return { xs, curves, dom: [ext[0] - pad, ext[1] + pad], x0 };
    }
    /** Current frame between `from` and `to` (curves and the domain interpolated). */
    function mix(a, b, t) {
      if (!a || b.bars || a.bars || a.curves?.length !== b.curves?.length || a.xs?.length !== b.xs?.length || a.xs[0] !== b.xs[0] || a.xs.at(-1) !== b.xs.at(-1)) return b;
      const k = ease(t);
      return {
        xs: b.xs,
        curves: b.curves.map((c, i) => ({ ...c, ys: c.ys.map((y, j) => { const y0 = a.curves[i].ys[j]; return isNum(y) && isNum(y0) ? lerp(y0, y, k) : y; }) })),
        dom: [lerp(a.dom[0], b.dom[0], k), lerp(a.dom[1], b.dom[1], k)],
        x0: lerp(a.x0, b.x0, k),
      };
    }

    function kick() {
      if (!S.raf) S.raf = requestAnimationFrame(frame);
    }
    let last = 0;
    function frame(now) {
      S.raf = 0;
      if (last) { const dt = now - last; if (dt < 200) { stats.ms = (stats.ms * stats.frames + dt) / (stats.frames + 1); stats.frames++; } }
      let more = false;
      if (S.mode === "2d") {
        const t = Math.min(1, (now - S.t0) / 280);
        S.cur = mix(S.from, S.to, t);
        draw2d(S.cur);
        more = t < 1;
      } else {
        more = stepMarbles() || !!S.drag;
        draw3d();
      }
      last = more ? now : 0;
      if (more) S.raf = requestAnimationFrame(frame);
    }

    // ── 2D ──
    const M = { l: 64, r: 28, t: 58, b: 46 };
    function draw2d(F) {
      size();
      const { W, H, dpr, tok } = S;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.fillStyle = tok.bg;
      ctx.fillRect(0, 0, W, H);
      if (!F) return;
      const pw = W - M.l - M.r, ph = H - M.t - M.b;
      const eq = S.eq;
      const out = S.C.__out ?? "#2de2e6";
      if (F.bars) {
        const n = F.bars.length, bw = (pw / n) * 0.62;
        const sy = (v) => M.t + ph - (v / F.dom[1]) * ph;
        grid(F.dom, sy, null, pw, ph);
        F.bars.forEach((v, i) => {
          const x = M.l + (pw / n) * (i + 0.5);
          glowRect(x - bw / 2, sy(v), bw, M.t + ph - sy(v), out);
          ctx.fillStyle = tok.muted;
          ctx.font = `11px ${tok.mono}`;
          ctx.textAlign = "center";
          ctx.fillText(String(i + 1), x, M.t + ph + 16);
        });
        return;
      }
      const [x0, x1] = [F.xs[0], F.xs.at(-1)];
      const sx = (x) => M.l + ((x - x0) / (x1 - x0)) * pw;
      const sy = (y) => M.t + ph - ((y - F.dom[0]) / (F.dom[1] - F.dom[0])) * ph;
      grid(F.dom, sy, [x0, x1, sx], pw, ph);
      ctx.save();
      ctx.beginPath();
      ctx.rect(M.l - 2, M.t - 2, pw + 4, ph + 4);
      ctx.clip();
      for (const c of F.curves) {
        if (c.hidden) continue;
        const pts = c.ys.map((y, i) => [sx(F.xs[i]), isNum(y) ? sy(y) : NaN]);
        if (c.ghost) line(pts, S.tok.faint, 1.4, [6, 5], 0);
        else line(pts, out, 2.4, null, 1);
      }
      // the point and its tangent glide with the curve
      const main = F.curves.at(-1);
      const i = Math.max(1, Math.min(N - 2, Math.round(((F.x0 - x0) / (x1 - x0)) * (N - 1))));
      const y = main.ys[i], ya = main.ys[i - 1], yb = main.ys[i + 1];
      if (isNum(y) && isNum(ya) && isNum(yb)) {
        const slope = (yb - ya) / (F.xs[i + 1] - F.xs[i - 1]);
        const half = (x1 - x0) * 0.12;
        const xc = F.xs[i];
        const tcol = S.C[eq.x.name] ?? "#ffd23f";
        line([[sx(xc - half), sy(y - slope * half)], [sx(xc + half), sy(y + slope * half)]], tcol, 1.6, null, 0.6);
        ctx.strokeStyle = rgba(S.tok.text, 0.25);
        ctx.setLineDash([2, 4]);
        ctx.beginPath();
        ctx.moveTo(sx(xc), sy(y));
        ctx.lineTo(sx(xc), M.t + ph);
        ctx.stroke();
        ctx.setLineDash([]);
        dot(sx(xc), sy(y), tcol);
        S.read = { x: xc, y, slope };
      }
      ctx.restore();
    }
    function grid(dom, sy, xs, pw, ph) {
      const { tok } = S;
      ctx.lineWidth = 1;
      const yt = LP.stats.linearTicks(dom[0], dom[1], 5);
      ctx.font = `11px ${tok.mono}`;
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      for (const v of yt) {
        const y = Math.round(sy(v)) + 0.5;
        if (y < M.t - 1 || y > M.t + ph + 1) continue;
        ctx.strokeStyle = rgba(tok.line, 0.9);
        ctx.beginPath();
        ctx.moveTo(M.l, y);
        ctx.lineTo(M.l + pw, y);
        ctx.stroke();
        ctx.fillStyle = tok.muted;
        ctx.fillText(LP.fmtNum(v, null, yt.step ?? Math.abs(yt[1] - yt[0])), M.l - 8, y);
      }
      if (xs) {
        const [a, b, sx] = xs;
        const xt = LP.stats.linearTicks(a, b, 6);
        ctx.textAlign = "center";
        ctx.textBaseline = "top";
        for (const v of xt) {
          const x = Math.round(sx(v)) + 0.5;
          if (x < M.l - 1 || x > M.l + pw + 1) continue;
          ctx.strokeStyle = rgba(tok.line, 0.6);
          ctx.beginPath();
          ctx.moveTo(x, M.t);
          ctx.lineTo(x, M.t + ph);
          ctx.stroke();
          ctx.fillStyle = tok.muted;
          ctx.fillText(LP.fmtNum(v, null, Math.abs(xt[1] - xt[0])), x, M.t + ph + 8);
        }
      }
      ctx.strokeStyle = tok.strong;
      ctx.beginPath();
      ctx.moveTo(M.l + 0.5, M.t);
      ctx.lineTo(M.l + 0.5, M.t + ph + 0.5);
      ctx.lineTo(M.l + pw, M.t + ph + 0.5);
      ctx.stroke();
    }
    /** A glowing stroke: two wide translucent passes under the line (baked into the stroke, no filters). */
    function line(pts, color, w, dash, glow) {
      const path = new Path2D();
      let on = false;
      for (const [x, y] of pts) {
        if (!isNum(x) || !isNum(y)) { on = false; continue; }
        if (on) path.lineTo(x, y); else { path.moveTo(x, y); on = true; }
      }
      ctx.lineJoin = ctx.lineCap = "round";
      ctx.setLineDash(dash ?? []);
      if (glow) {
        ctx.strokeStyle = rgba(color, 0.1 * glow);
        ctx.lineWidth = w * 5;
        ctx.stroke(path);
        ctx.strokeStyle = rgba(color, 0.22 * glow);
        ctx.lineWidth = w * 2.4;
        ctx.stroke(path);
      }
      ctx.strokeStyle = color;
      ctx.lineWidth = w;
      ctx.stroke(path);
      ctx.setLineDash([]);
    }
    function dot(x, y, color, r = 5) {
      ctx.fillStyle = rgba(color, 0.18);
      ctx.beginPath();
      ctx.arc(x, y, r * 2.4, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = S.tok.bg;
      ctx.beginPath();
      ctx.arc(x, y, r * 0.4, 0, Math.PI * 2);
      ctx.fill();
    }
    function glowRect(x, y, w, h, color) {
      ctx.fillStyle = rgba(color, 0.16);
      ctx.fillRect(x - 4, y - 4, w + 8, h + 4);
      ctx.fillStyle = rgba(color, 0.85);
      ctx.fillRect(x, y, w, h);
    }

    // ── 3D: f over (input, parameter) ──
    const G = 44;
    function surface() {
      const eq = S.eq;
      const pv = E.eq.param(eq, eq.animate?.var) ?? (eq.vars ?? []).find((v) => v.role === "param");
      if (!pv || !eq.x || E.eq.bars(eq)) return null;
      const xs = E.eq.xs(eq, G), ps = E.eq.sweep(pv, G);
      const z = ps.map((v) => E.eq.curve(eq, S.ast, xs, { [pv.name]: v }));
      const all = z.flat().filter(isNum);
      // the same robust range as the 2D views: a pole (p → 0) is capped so the rest keeps its height
      const ext = LP.eqExtent(all);
      const zn = z.map((row) => row.map((v) => (isNum(v) ? Math.max(0, Math.min(1, (v - ext[0]) / (ext[1] - ext[0] || 1))) : NaN)));
      const capped = z.map((row) => row.map((v) => isNum(v) && v > ext[1]));
      // per quad: height tint between two theme colours, light from above, alpha — computed once here, not per frame
      const lo = hex3(S.C.__out ?? "#2de2e6"), hi = hex3(S.tok.text);
      const fills = [];
      for (let j = 0; j < G - 1; j++) {
        const row = [];
        for (let i = 0; i < G - 1; i++) {
          const zz = [zn[j][i], zn[j][i + 1], zn[j + 1][i + 1], zn[j + 1][i]];
          if (!zz.every(isNum)) { row.push(null); continue; }
          const h = (zz[0] + zz[1] + zz[2] + zz[3]) / 4;
          const nx = (zz[0] - zz[1] + zz[3] - zz[2]) * 0.5 * (G - 1) * 0.62, ny = (zz[0] - zz[3] + zz[1] - zz[2]) * 0.5 * (G - 1) * 0.62;
          const light = 0.5 + 0.5 / Math.sqrt(1 + nx * nx + ny * ny);
          const cap = capped[j][i] || capped[j][i + 1] || capped[j + 1][i + 1] || capped[j + 1][i];
          const c = lo.map((a, k) => Math.round((a + (hi[k] - a) * h * 0.8) * light));
          row.push({ fill: `rgba(${c[0]},${c[1]},${c[2]},${cap ? 0.18 : 0.42 + 0.4 * h})`, edge: `rgba(${lo[0]},${lo[1]},${lo[2]},${cap ? 0.12 : 0.22 + 0.3 * h})` });
        }
        fills.push(row);
      }
      return { pv, xs, ps, zn, ext, fills };
    }
    function project(u, v, w) {
      // u, v ∈ [0, 1] (input, parameter), w ∈ [0, 1] (height) → screen
      const x = u - 0.5, y = v - 0.5, z = w * 0.62 - 0.25;
      const cy = Math.cos(S.yaw), syw = Math.sin(S.yaw), cp = Math.cos(S.pitch), sp = Math.sin(S.pitch);
      const X = x * cy - y * syw, Y = x * syw + y * cy;
      const depth = Y * cp + z * sp;
      const Z = z * cp - Y * sp;
      const k = Math.min(S.W * 0.8, S.H) * 0.78 / (1 + depth * 0.35);
      return [S.W / 2 + X * k, S.H * 0.6 - Z * k, depth];
    }
    function draw3d() {
      size();
      const { W, H, dpr, tok } = S;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.fillStyle = tok.bg;
      ctx.fillRect(0, 0, W, H);
      const g = S.grid;
      if (!g) {
        ctx.fillStyle = tok.muted;
        ctx.font = `13px ${tok.mono}`;
        ctx.textAlign = "center";
        ctx.fillText("—", W / 2, H / 2);
        return;
      }
      const out = S.C.__out ?? "#2de2e6", cx = S.C[S.eq.x.name] ?? "#ffd23f", cp = S.C[g.pv.name] ?? "#ff3fb4";
      // floor frame and axes in the variables' colours
      const axis = (a, b, c) => { const p = project(...a), q = project(...b); line([[p[0], p[1]], [q[0], q[1]]], c, 1.6, null, 0.5); };
      axis([0, 0, 0], [1, 0, 0], cx);
      axis([0, 0, 0], [0, 1, 0], cp);
      axis([0, 0, 0], [0, 0, 1], out);
      ctx.font = `11px ${tok.mono}`;
      ctx.textBaseline = "middle";
      ctx.textAlign = "center";
      const tick = (pos, text, c) => { const p = project(...pos); ctx.fillStyle = rgba(c, 0.85); ctx.fillText(text, p[0], p[1]); };
      const [xa, xb] = [g.xs[0], g.xs.at(-1)];
      for (const v of LP.stats.linearTicks(xa, xb, 4)) if (v >= xa - 1e-9 && v <= xb + 1e-9) tick([(v - xa) / (xb - xa), -0.07, 0], LP.fmtNum(v, null, Math.abs(xb - xa) / 4), cx);
      const [pa, pb] = [g.ps[0], g.ps.at(-1)];
      const pticks = g.pv.log ? [pa, Math.sqrt(pa * pb), pb] : LP.stats.linearTicks(pa, pb, 4).filter((v) => v >= pa - 1e-9 && v <= pb + 1e-9);
      for (const v of pticks.slice(1)) {
        const t = g.pv.log ? (Math.log(v) - Math.log(pa)) / (Math.log(pb) - Math.log(pa)) : (v - pa) / (pb - pa);
        tick([-0.07, t, 0], LP.eqFmt(g.pv, v), cp);
      }
      for (const v of LP.stats.linearTicks(g.ext[0], g.ext[1], 3)) if (v > g.ext[0] + 1e-9 && v <= g.ext[1] + 1e-9) tick([-0.06, -0.06, (v - g.ext[0]) / (g.ext[1] - g.ext[0])], LP.fmtNum(v, null, Math.abs(g.ext[1] - g.ext[0]) / 3), out);
      const place = (el, pos) => { const p = project(...pos); el.style.left = `${p[0]}px`; el.style.top = `${p[1]}px`; };
      place(ax3.x, [0.55, -0.2, 0]);
      place(ax3.p, [-0.2, 0.55, 0]);
      place(ax3.f, [0, 0, 1.3]);
      // quads back to front, height-tinted, lit from above
      const quads = [];
      for (let j = 0; j < G - 1; j++) for (let i = 0; i < G - 1; i++) {
        const zz = [g.zn[j][i], g.zn[j][i + 1], g.zn[j + 1][i + 1], g.zn[j + 1][i]];
        if (!zz.every(isNum)) continue;
        const u0 = i / (G - 1), u1 = (i + 1) / (G - 1), v0 = j / (G - 1), v1 = (j + 1) / (G - 1);
        const P = [project(u0, v0, zz[0]), project(u1, v0, zz[1]), project(u1, v1, zz[2]), project(u0, v1, zz[3])];
        quads.push({ P, d: (P[0][2] + P[1][2] + P[2][2] + P[3][2]) / 4, f: g.fills[j][i] });
      }
      quads.sort((a, b) => b.d - a.d);
      ctx.lineWidth = 0.6;
      for (const q of quads) {
        ctx.beginPath();
        ctx.moveTo(q.P[0][0], q.P[0][1]);
        for (let k = 1; k < 4; k++) ctx.lineTo(q.P[k][0], q.P[k][1]);
        ctx.closePath();
        ctx.fillStyle = q.f.fill;
        ctx.fill();
        ctx.strokeStyle = q.f.edge;
        ctx.stroke();
      }
      // contour lines (marching squares on the grid)
      for (let L = 1; L < 8; L++) {
        const lv = L / 8;
        const segs = [];
        for (let j = 0; j < G - 1; j++) for (let i = 0; i < G - 1; i++) {
          const c = [g.zn[j][i], g.zn[j][i + 1], g.zn[j + 1][i + 1], g.zn[j + 1][i]];
          if (!c.every(isNum)) continue;
          const pts = [];
          const E4 = [[0, 1, [i, j], [i + 1, j]], [1, 2, [i + 1, j], [i + 1, j + 1]], [2, 3, [i + 1, j + 1], [i, j + 1]], [3, 0, [i, j + 1], [i, j]]];
          for (const [a, b, A, Bp] of E4) {
            if ((c[a] - lv) * (c[b] - lv) < 0) {
              const t = (lv - c[a]) / (c[b] - c[a]);
              pts.push([(A[0] + (Bp[0] - A[0]) * t) / (G - 1), (A[1] + (Bp[1] - A[1]) * t) / (G - 1)]);
            }
          }
          if (pts.length >= 2) segs.push([pts[0], pts[1]]);
        }
        ctx.strokeStyle = rgba(cx, 0.55);
        ctx.lineWidth = 1;
        ctx.beginPath();
        for (const [a, b] of segs) {
          const p = project(a[0], a[1], lv + 0.004), q = project(b[0], b[1], lv + 0.004);
          ctx.moveTo(p[0], p[1]);
          ctx.lineTo(q[0], q[1]);
        }
        ctx.stroke();
      }
      // marbles
      for (const m of S.marbles) {
        const tr = m.trail.map(([u, v]) => { const p = project(u, v, zAt(u, v) + 0.01); return [p[0], p[1]]; });
        line(tr, S.trail, 1.6, null, 1);
        const p = project(m.u, m.v, zAt(m.u, m.v) + 0.015);
        dot(p[0], p[1], S.trail, 4.2);
      }
    }
    function zAt(u, v) {
      const g = S.grid;
      const fi = Math.max(0, Math.min(G - 1.0001, u * (G - 1))), fj = Math.max(0, Math.min(G - 1.0001, v * (G - 1)));
      const i = Math.floor(fi), j = Math.floor(fj), a = fi - i, b = fj - j;
      const z = (jj, ii) => (isNum(g.zn[jj][ii]) ? g.zn[jj][ii] : 1);
      return (1 - b) * ((1 - a) * z(j, i) + a * z(j, i + 1)) + b * ((1 - a) * z(j + 1, i) + a * z(j + 1, i + 1));
    }
    /** One step of every marble down −∇f (with a little inertia); false once all are still. */
    function stepMarbles() {
      if (!S.marbles.length || !S.grid) return false;
      let moving = false;
      const h = 1 / (G - 1);
      for (const m of S.marbles) {
        if (m.still) continue;
        const gu = (zAt(Math.min(1, m.u + h), m.v) - zAt(Math.max(0, m.u - h), m.v)) / (2 * h);
        const gv = (zAt(m.u, Math.min(1, m.v + h)) - zAt(m.u, Math.max(0, m.v - h))) / (2 * h);
        m.vu = 0.86 * m.vu - 0.0022 * gu;
        m.vv = 0.86 * m.vv - 0.0022 * gv;
        m.u = Math.max(0, Math.min(1, m.u + m.vu));
        m.v = Math.max(0, Math.min(1, m.v + m.vv));
        m.trail.push([m.u, m.v]);
        if (m.trail.length > 160) m.trail.shift();
        if (Math.hypot(m.vu, m.vv) < 2e-5 && ++m.slow > 20) m.still = true;
        else moving = true;
      }
      return moving;
    }

    // ── orbit ──
    cv.addEventListener("pointerdown", (e) => {
      if (S.mode !== "3d") return;
      S.drag = { x: e.clientX, y: e.clientY, yaw: S.yaw, pitch: S.pitch };
      cv.setPointerCapture(e.pointerId);
      kick();
    });
    cv.addEventListener("pointermove", (e) => {
      if (!S.drag) return;
      S.yaw = S.drag.yaw + (e.clientX - S.drag.x) * 0.008;
      S.pitch = Math.max(0.08, Math.min(1.45, S.drag.pitch + (e.clientY - S.drag.y) * 0.006));
      kick();
    });
    const up = () => { S.drag = null; };
    cv.addEventListener("pointerup", up);
    cv.addEventListener("pointercancel", up);

    // ── HTML overlays: the equation and the axis labels, colour-coded (KaTeX) ──
    function overlays() {
      const eq = S.eq;
      const C = S.C;
      const syms = LP.eqColors.symbolsFor(eq, C);
      const labels = Object.fromEntries([...(eq.vars ?? []), ...(eq.x ? [eq.x] : [])].map((v) => [v.name, v.label]));
      // the paper's form (or the template's), each variable in its colour; else ours from the parsed tree
      const src = eq.latex || eq.tex || "";
      // the left-hand side (the function itself) in the output's colour
      const at = src.indexOf("=");
      let tex = !src ? "" : at > 0 && !/\\begin/.test(src.slice(0, at)) ? `\\textcolor{${C.__out}}{${src.slice(0, at)}}=${E.colorTex(src.slice(at + 1), syms)}` : E.colorTex(src, syms);
      if (!tex) {
        try {
          tex = `\\textcolor{${C.__out}}{f} = ${E.toLatex(S.ast, labels, C)}`;
        } catch { tex = ""; }
      }
      head.innerHTML = katexHtml(tex);
      const p = S.spec.panels[LP.eq.panelIndex(S.spec)];
      const xl = p.xaxis?.label ?? "";
      xlab.innerHTML = xl ? katexHtml(E.colorTex(xl.replace(/\$/g, ""), syms)) : "";
      const yl = (p.yaxis?.label ?? "").replace(/\$/g, "");
      ylab.innerHTML = yl ? katexHtml(`\\textcolor{${C.__out}}{${yl}}`) : "";
      xlab.hidden = ylab.hidden = S.mode === "3d";
      const pv = E.eq.param(eq, eq.animate?.var) ?? (eq.vars ?? []).find((v) => v.role === "param");
      ax3.x.innerHTML = eq.x ? katexHtml(E.colorTex((eq.x.label || eq.x.name).replace(/\$/g, "").replace(/（.*?）/g, ""), syms)) : "";
      ax3.p.innerHTML = pv ? katexHtml(`\\textcolor{${C[pv.name]}}{${(pv.label || pv.name).replace(/\$/g, "").replace(/（.*?）/g, "")}}`) : "";
      ax3.f.innerHTML = yl ? katexHtml(`\\textcolor{${C.__out}}{${yl}}`) : "";
      for (const el of Object.values(ax3)) el.hidden = S.mode !== "3d";
      S.trail = LP.eqEditor.themePalette()[1] ?? S.tok.text;
      const an = E.eq.param(eq, eq.animate?.var);
      read.innerHTML = an ? katexHtml(`\\textcolor{${C[an.name]}}{${(an.label || an.name).replace(/\$/g, "").replace(/（.*?）/g, "")}} = ${LP.eqFmt(an, an.value)}`) : "";
    }

    const api = {
      update(spec) {
        S.spec = spec;
        const p = spec?.panels?.[LP.eq.panelIndex(spec)];
        if (!p) return;
        S.eq = p.eq;
        try { S.ast = LP.eqCompile(p.eq); } catch { return; }
        S.tok = themeTok();
        const to = target();
        overlays();
        if (S.mode === "3d") {
          S.grid = surface();
          if (S.marbles.length) api.marbles(S.marbles.length);
          kick();
          return;
        }
        S.from = S.cur ?? null;
        S.to = to;
        S.t0 = performance.now();
        kick();
      },
      mode(m) {
        if (m && m !== S.mode) {
          S.mode = m;
          wrap.dataset.mode = m;
          if (m === "3d") { S.grid = surface(); S.marbles = []; } else { S.cur = null; S.from = null; S.t0 = 0; }
          overlays();
          kick();
        }
        return S.mode;
      },
      marbles(n = 7) {
        if (S.mode !== "3d" || !S.grid) return;
        // one row across the parameter, each at the input where the (uncapped) loss is highest
        S.marbles = Array.from({ length: n }, (_, k) => {
          const v = 0.08 + (0.84 * k) / (n - 1);
          let u = 0.5, best = -1;
          for (let i = 1; i < G - 1; i++) {
            const uu = i / (G - 1), z = zAt(uu, v);
            if (z < 0.97 && z > best) { best = z; u = uu; }
          }
          return { u, v, vu: 0, vv: 0, trail: [], slow: 0, still: false };
        });
        for (const m of S.marbles) m.trail.push([m.u, m.v]);
        kick();
      },
      reading: () => S.read,
      destroy() {
        cancelAnimationFrame(S.raf);
        ro.disconnect();
        wrap.remove();
      },
    };
    size();
    return api;
  }

  LP.eqLive = { mount, stats };
})();
