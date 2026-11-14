// Simulation scenes, the view (docs/plot.md §11): a figure whose `eq.sim` is a run (simcore.js computes every step).
// One drawing routine, two palettes and two painters:
//   live (app theme, glow, canvas)    LorePlot.eqSim.mount(host, spec, { role, onState })
//   file (the figure's style, SVG)    the `equation` chart draws a sim panel through LorePlot.eqSim.chart (PNG / PDF / the
//                                     editor's 輸出); LorePlot.eqSim.frames → LorePlot.eq.gifFrames / .gif (paced by change)
// Layout: left the animation with one pull arrow per term at the parameter marker; right one aligned row per term
// (on/off, weight slider, the term's LaTeX, its pull −wᵢ∂Lᵢ/∂θ, the weighted value); below them the weighted loss over
// the steps, stacked one colour per term, with a mark wherever a weight changed.
// Roles: "full" (editor, a single screen: everything), "control" (the presenter: runs the simulation and sends its state
// through onState), "view" (the audience: renders what it is sent, `apply(msg)`, computes no steps).
(() => {
  "use strict";
  const LP = window.LorePlot;
  const E = window.LoreEq;
  const L = window.LoreSim;
  if (!LP || !E || !L) throw new Error("plot.js, eqcore.js and simcore.js must load before eqsim.js");
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const clone = (o) => JSON.parse(JSON.stringify(o));
  const stats = { frames: 0, ms: 0, step: 0, draw: 0 };
  const VW = 960, VH = 540; // the virtual page of a frame (GIF / file)

  // ── colours ──
  const hex3 = (c) => {
    const m = /^#?([0-9a-f]{6})/i.exec(String(c).trim());
    if (m) { const n = parseInt(m[1], 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; }
    const r = /rgba?\(([^)]+)\)/.exec(String(c));
    return r ? r[1].split(",").slice(0, 3).map((x) => +x) : [128, 128, 128];
  };
  const rgba = (c, a) => `rgba(${hex3(c).join(",")},${a})`;
  function rotateHue(c, deg) {
    const [r, g, b] = hex3(c).map((v) => v / 255), mx = Math.max(r, g, b), mn = Math.min(r, g, b), l = (mx + mn) / 2, dd = mx - mn;
    let h = 0;
    const sat = dd === 0 ? 0 : dd / (1 - Math.abs(2 * l - 1));
    if (dd) h = mx === r ? ((g - b) / dd) % 6 : mx === g ? (b - r) / dd + 2 : (r - g) / dd + 4;
    h = (h * 60 + deg + 360) % 360;
    const C = (1 - Math.abs(2 * l - 1)) * Math.max(sat, 0.55), X = C * (1 - Math.abs(((h / 60) % 2) - 1)), m = l - C / 2;
    const [r1, g1, b1] = h < 60 ? [C, X, 0] : h < 120 ? [X, C, 0] : h < 180 ? [0, C, X] : h < 240 ? [0, X, C] : h < 300 ? [X, 0, C] : [C, 0, X];
    return "#" + [r1, g1, b1].map((v) => Math.round(Math.min(1, Math.max(0, v + m)) * 255).toString(16).padStart(2, "0")).join("");
  }
  /** Colours far enough from each other and from `avoid` (p and q): a theme whose accents are near-twins still
   *  gives every term its own colour; when too few remain, the near ones come back in order. */
  function distinct(cands, avoid) {
    const d = (a, b) => { const x = hex3(a), y = hex3(b); return Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]); };
    const out = [];
    // when the theme's own colours run out: the model's colour turned round the hue circle (same lightness)
    const turned = [90, 180, 270, 45, 135, 225, 315].map((deg) => rotateHue(avoid[1] ?? cands[0], deg));
    for (const c of [...cands, ...turned]) if (avoid.every((a) => d(a, c) > 70) && out.every((o) => d(o, c) > 70)) out.push(c);
    for (const c of cands) if (out.length < 4 && !out.includes(c)) out.push(c);
    return out;
  }
  /** The Lore theme (live view only). */
  function themePal() {
    const cs = getComputedStyle(document.documentElement);
    const t = (k, d) => cs.getPropertyValue(k).trim() || d;
    return {
      bg: t("--bg", "#151c26"), panel: t("--panel", "#1b2431"), line: t("--line", "#2b3748"), strong: t("--line-strong", "#425369"),
      text: t("--text", "#eaf0f8"), muted: t("--muted", "#aebdd0"), faint: t("--faint", "#7787a0"), dot: t("--dot", "#4fd8ff12"),
      // the -text variants: the same hues tuned to read as text on --bg (light themes too)
      p: t("--accent-2-text", "#4fd8ff"), q: t("--accent-text", "#ff5fa2"), loss: t("--accent-3-text", "#a98cff"),
      terms: distinct([t("--accent-3-text", "#a98cff"), t("--warn", "#ffcf5a"), t("--ok", "#4fe3a0"), t("--err", "#ff6b86"), t("--accent-3", "#a98cff"), t("--accent-2", "#2fb2e6"), t("--muted", "#aebdd0")], [t("--accent-2-text", "#4fd8ff"), t("--accent-text", "#ff5fa2"), t("--text", "#eaf0f8")]),
      // colour roles (lore-notation Entry.color): the same list as every equation figure's live view
      roles: LP.eqEditor?.themePalette?.() ?? [],
      mono: t("--mono", "ui-monospace, Menlo, monospace"), glow: true,
    };
  }
  /** The figure's style (files: GIF, PNG, PDF) — never the theme. */
  function docPal(st) {
    const pal = st.colors.lines?.length ? st.colors.lines : st.colors.palette;
    const at = (i) => pal[i % pal.length];
    return {
      bg: st.colors.background ?? "#ffffff", panel: st.colors.background ?? "#ffffff", line: "#d9d9d9", strong: st.colors.axis ?? "#000000",
      text: st.colors.text ?? "#000000", muted: "#555555", faint: "#8a8a8a", dot: null,
      p: at(0), q: at(3 % pal.length) === at(0) ? at(1) : at(3), loss: at(4 % pal.length),
      terms: [at(4 % pal.length), at(1), at(2), at(5 % pal.length), at(6 % pal.length), at(7 % pal.length)],
      roles: pal,
      mono: "Menlo, monospace", glow: false,
    };
  }
  /** A term's colour: its role (lore-notation Entry.color, set by assignRoles) else the next distinct accent. */
  function termColor(P, t, i) {
    const c = isNum(t.color) && P.roles?.length ? P.roles[t.color % P.roles.length] : null;
    // a role that lands on p's or q's colour would read as the target / the model: the term's own colour instead
    const near = (a, b) => { const x = hex3(a), y = hex3(b); return Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]) < 40; };
    return c && !near(c, P.p) && !near(c, P.q) ? c : P.terms[i % P.terms.length];
  }

  // ── LaTeX ──
  const fill = (tex, P) => String(tex ?? "").replace(/\\P\{/g, `\\textcolor{${P.p}}{`).replace(/\\Q\{/g, `\\textcolor{${P.q}}{`);
  const termTex = (sim, t, P) => (t.kind === "expr" ? E.toLatex(E.parse(String(t.expr ?? "0"), L.EXPR_NAMES[sim.scene] ?? []), { mu: "\\mu", sigma: "\\sigma" }) : fill(t.tex || L.TERMS[sim.scene]?.[t.kind]?.tex || t.kind, P).replace(/\\mu_0/g, isNum(t.mu0) ? `\\mu_0` : "\\mu_0"));
  const weightTex = (t, i) => t.name || `w_{${i + 1}}`;
  /** The whole objective: each weight in its term's colour (composite) or the scene's LaTeX. */
  function headTex(eq, P) {
    const sim = eq.sim;
    if (sim.terms.length < 2 || sim.scene === "cluster" && !sim.terms.some((t) => t.kind === "l2")) return fill(eq.latex || "", P);
    const parts = sim.terms.map((t, i) => {
      let tt = "";
      try { tt = termTex(sim, t, P); } catch { tt = "?"; }
      return `\\textcolor{${termColor(P, t, i)}}{${weightTex(t, i)}}\\,${needsParens(tt) ? `\\left(${tt}\\right)` : tt}`;
    });
    return `\\mathcal{L} = ${parts.join(" + ")}`;
  }
  /** A sum or a leading minus at the top level (outside braces and brackets) needs parentheses after a weight. */
  function needsParens(tt) {
    let d = 0;
    const s = tt.replace(/\\(left|right)[()[\]|.]/g, "").trim();
    for (let i = 0; i < s.length; i++) {
      const c = s[i];
      if ("{([".includes(c)) d++;
      else if ("})]".includes(c)) d--;
      else if (d === 0 && (c === "+" || c === "-") && (i > 0 || c === "-")) return true;
    }
    return false;
  }
  const escHtml = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  // specs come inside GIFs from other people's decks: without KaTeX the LaTeX is shown as escaped text
  const katexHtml = (tex) => {
    try { return window.katex ? window.katex.renderToString(tex, { throwOnError: false, output: "html" }) : escHtml(tex); } catch { return escHtml(tex); }
  };

  // ── painters: the same calls onto a canvas (live, pacing) or into SVG (files) ──
  function canvasPainter(ctx, P, tk = 1) {
    const path = (pts) => { const p = new Path2D(); let pen = false; for (const [x, y] of pts) { if (!isNum(x) || !isNum(y)) { pen = false; continue; } if (pen) p.lineTo(x, y); else { p.moveTo(x, y); pen = true; } } return p; };
    return {
      P, ctx,
      clear(w, h) {
        if (P.see) { ctx.clearRect(0, 0, w, h); return; } // the live view: the page's own background (and its dot grid) shows through
        ctx.fillStyle = P.bg; ctx.fillRect(0, 0, w, h);
        if (P.dot) { ctx.fillStyle = P.dot; for (let x = 12; x < w; x += 24) for (let y = 12; y < h; y += 24) ctx.fillRect(x, y, 1.5, 1.5); }
      },
      line(pts, color, w = 2.5, glow = 1, dash) {
        const p = path(pts);
        ctx.lineJoin = ctx.lineCap = "round";
        ctx.setLineDash(dash ?? []);
        if (glow && P.glow) { ctx.strokeStyle = rgba(color, 0.1 * glow); ctx.lineWidth = w * 5; ctx.stroke(p); ctx.strokeStyle = rgba(color, 0.22 * glow); ctx.lineWidth = w * 2.4; ctx.stroke(p); }
        ctx.strokeStyle = color; ctx.lineWidth = w; ctx.stroke(p);
        ctx.setLineDash([]);
      },
      area(pts, base, color, a) {
        if (!pts.length) return;
        ctx.beginPath(); ctx.moveTo(pts[0][0], base);
        for (const [x, y] of pts) ctx.lineTo(x, y);
        ctx.lineTo(pts.at(-1)[0], base); ctx.closePath();
        const top = Math.min(...pts.map((q) => q[1]));
        const g = ctx.createLinearGradient(0, top, 0, base);
        g.addColorStop(0, rgba(color, a)); g.addColorStop(1, rgba(color, a * 0.15));
        ctx.fillStyle = g; ctx.fill();
      },
      dot(x, y, color, r = 5, hole = true) {
        if (P.glow) { ctx.fillStyle = rgba(color, 0.18); ctx.beginPath(); ctx.arc(x, y, r * 2.4, 0, 7); ctx.fill(); }
        ctx.fillStyle = color; ctx.beginPath(); ctx.arc(x, y, r, 0, 7); ctx.fill();
        if (hole) { ctx.fillStyle = P.bg; ctx.beginPath(); ctx.arc(x, y, r * 0.4, 0, 7); ctx.fill(); }
      },
      rect(x, y, w, h, color, a = 1) { ctx.fillStyle = a === 1 ? color : rgba(color, a); ctx.fillRect(x, y, w, h); },
      text(s, x, y, color, size = 12, align = "left") { ctx.font = `${size * tk}px ${P.mono}`; ctx.fillStyle = color; ctx.textAlign = align; ctx.fillText(s, x, y); ctx.textAlign = "left"; },
      tex(tex, x, y, size, color, align) {
        // single letters (curve names) in TeX's italic; anything else is only measured (pacing) — plain text
        const t = String(tex).replace(/\\[a-z]+\{|[{}\\]/gi, "");
        if (/^[a-z]$/i.test(t)) { ctx.font = `italic ${size * tk}px "KaTeX_Math", "Times New Roman", serif`; ctx.fillStyle = color; ctx.textAlign = align ?? "left"; ctx.fillText(t, x, y); ctx.textAlign = "left"; return; }
        this.text(t, x, y, color, size, align);
      },
      ring(x, y, r, color) { ctx.lineWidth = 2; ctx.strokeStyle = color; ctx.fillStyle = P.bg; ctx.beginPath(); ctx.arc(x, y, r, 0, 7); ctx.fill(); ctx.stroke(); ctx.fillStyle = color; ctx.beginPath(); ctx.arc(x, y, 2.2, 0, 7); ctx.fill(); },
      image(cv, x, y, w, h) { ctx.imageSmoothingEnabled = true; ctx.imageSmoothingQuality = "high"; ctx.drawImage(cv, x, y, w, h); },
      clip(x, y, w, h, fn) { ctx.save(); ctx.beginPath(); ctx.rect(x, y, w, h); ctx.clip(); fn(); ctx.restore(); },
      alpha(a, fn) { const k = ctx.globalAlpha; ctx.globalAlpha = a; fn(); ctx.globalAlpha = k; },
    };
  }
  function svgPainter(c, P) {
    const out = [];
    const r2 = (v) => Math.round(v * 100) / 100;
    const d = (pts) => { let s = "", pen = false; for (const [x, y] of pts) { if (!isNum(x) || !isNum(y)) { pen = false; continue; } s += `${pen ? "L" : "M"}${r2(x)} ${r2(y)}`; pen = true; } return s; };
    let alpha = 1;
    const op = () => (alpha < 1 ? ` opacity="${alpha}"` : "");
    let clipN = 0;
    return {
      P, out, heat: null,
      clear(w, h) { out.push(`<rect x="0" y="0" width="${w}" height="${h}" fill="${P.bg}"/>`); },
      line(pts, color, w = 2.5, _glow, dash) { out.push(`<path d="${d(pts)}" fill="none" stroke="${color}" stroke-width="${r2(w)}" stroke-linejoin="round" stroke-linecap="round"${dash ? ` stroke-dasharray="${dash.join(" ")}"` : ""}${op()}/>`); },
      area(pts, base, color, a) { if (pts.length) out.push(`<path d="M${r2(pts[0][0])} ${r2(base)}${d(pts).replace(/^M/, "L")}L${r2(pts.at(-1)[0])} ${r2(base)}Z" fill="${color}" fill-opacity="${r2(a * 0.7)}"${op()}/>`); },
      dot(x, y, color, r = 5, hole = true) { out.push(`<circle cx="${r2(x)}" cy="${r2(y)}" r="${r2(r)}" fill="${color}"${op()}/>`); if (hole) out.push(`<circle cx="${r2(x)}" cy="${r2(y)}" r="${r2(r * 0.4)}" fill="${P.bg}"/>`); },
      rect(x, y, w, h, color, a = 1) { out.push(`<rect x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}" fill="${color}"${a < 1 ? ` fill-opacity="${a}"` : ""}${op()}/>`); },
      text(s, x, y, color, size = 12, align = "left") {
        const t = c.T(String(s), { ...c.font("tick"), size, color });
        out.push(t.place(x, y, { anchor: align === "right" ? "end" : align === "center" ? "middle" : "start" }));
      },
      tex(tex, x, y, size, color, align = "left") {
        const f = { ...c.font("label"), size, color };
        const t = c.T(`$${tex}$`, f);
        out.push(t.place(x, y, { anchor: align === "right" ? "end" : align === "center" ? "middle" : "start" }));
      },
      image(cv, x, y, w, h) {
        out.push(`<image x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}" preserveAspectRatio="none" href="${cv.toDataURL("image/png")}"/>`);
        this.heat = [x, y, w, h];
      },
      clip(x, y, w, h, fn) { const id = `lpsimclip${Date.now().toString(36)}${clipN++}`; out.push(`<clipPath id="${id}"><rect x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}"/></clipPath><g clip-path="url(#${id})">`); fn(); out.push("</g>"); },
      alpha(a, fn) { const k0 = alpha; alpha = a; fn(); alpha = k0; },
    };
  }

  // ── the scene ──
  /** Pull arrow (dx, dy in px; tiny pulls draw nothing). */
  function arrow(Pt, x, y, dx, dy, color, w = 2.4) {
    const n = Math.hypot(dx, dy);
    if (n < 2) return;
    const ux = dx / n, uy = dy / n;
    Pt.line([[x, y], [x + dx - ux * 3, y + dy - uy * 3]], color, w, 0.7);
    const hx = x + dx, hy = y + dy;
    Pt.line([[hx - ux * 8 - uy * 5, hy - uy * 8 + ux * 5], [hx, hy], [hx - ux * 8 + uy * 5, hy - uy * 8 - ux * 5]], color, w, 0);
  }
  /** A pull g → px: compressed so small pulls stay visible and big ones stay inside. */
  const pullPx = (g, max, ref = 0.6) => Math.sign(g) * max * Math.tanh(Math.abs(g) / ref);
  let heatCv = null;
  /** Density / partition heatmap into a small canvas (bg → colour), drawn scaled. */
  function heat(view, P, kind, gw = 120) {
    const [x0, x1, y0, y1] = view.box;
    const gh = Math.round((gw * (y1 - y0)) / (x1 - x0));
    if (typeof document === "undefined") return null;
    heatCv ??= document.createElement("canvas");
    if (heatCv.width !== gw || heatCv.height !== gh) { heatCv.width = gw; heatCv.height = gh; }
    const g = heatCv.getContext("2d"), img = g.createImageData(gw, gh), D = img.data;
    const b = hex3(P.bg);
    const cols = kind === "part" ? Array.from({ length: view.k }, (_, j) => hex3(P.terms[(j + 1) % P.terms.length])) : null;
    const q = hex3(P.q);
    for (let j = 0; j < gh; j++) for (let i = 0; i < gw; i++) {
      const x = x0 + ((i + 0.5) / gw) * (x1 - x0), y = y1 - ((j + 0.5) / gh) * (y1 - y0), o = (j * gw + i) * 4;
      let c = b;
      if (kind === "part") {
        const pr = view.classify(x, y);
        let best = 0; for (let k = 1; k < pr.length; k++) if (pr[k] > pr[best]) best = k;
        const t = 0.06 + 0.16 * clamp((pr[best] - 1 / pr.length) / (1 - 1 / pr.length), 0, 1);
        c = b.map((v, ch) => v + (cols[best][ch] - v) * t);
      } else {
        const v = Math.exp(view.logq(x, y));
        let t = Math.pow(v / 0.5, 0.55);
        // files: 64 levels (the GIF's ordered dither hides the steps; unchanged pixels between frames cost nothing)
        if (!P.glow) t = Math.round(t * 64) / 64;
        const u = Math.min(t, 1), w = Math.max(0, t - 1) * 0.35;
        c = b.map((bv, ch) => bv + (q[ch] - bv) * u * 0.75 + w * 220);
      }
      D[o] = c[0]; D[o + 1] = c[1]; D[o + 2] = c[2]; D[o + 3] = 255;
    }
    g.putImageData(img, 0, 0);
    return heatCv;
  }
  /** The y range a 1D frame needs: the target's peak with room, q's peak when taller (at most 4× the target's). */
  function yNeed(V) {
    const pm = Math.max(0.3, ...V.p);
    return Math.max(pm * 1.25, Math.min(1.1 / (V.sigma * Math.sqrt(2 * Math.PI)), pm * 4));
  }
  /** The animation: in rect R (virtual px), the run's view `V`, pulls per term, term colours. */
  function drawScene(Pt, R, sim, V, pulls, colors, opts = {}) {
    const P = Pt.P;
    const scene = sim.scene;
    if (scene === "fit1d") {
      const top = R.y + 14, base = R.y + R.h - 26 - 16 * Math.max(1, sim.terms.length), x0 = V.xs[0], x1 = V.xs.at(-1);
      // the y range: what this frame needs (the target's peak, q's when it is taller; capped), eased by the caller
      const ymax = opts.ymax ?? yNeed(V);
      const sx = (x) => R.x + ((x - x0) / (x1 - x0)) * R.w, sy = (v) => base - (Math.min(v, ymax * 1.08) / ymax) * (base - top);
      Pt.line([[R.x, base + 0.5], [R.x + R.w, base + 0.5]], P.strong, 1, 0);
      for (const t of sim.terms) if (t.kind === "anchor" && isNum(t.mu0)) {
        const c = colors[sim.terms.indexOf(t)];
        Pt.line([[sx(t.mu0), top + 12], [sx(t.mu0), base]], c, 1.2, 0, [3, 5]);
        Pt.text("μ₀", sx(t.mu0), top + 6, c, 13, "center");
      }
      const step = Math.max(1, Math.floor(V.xs.length / 240));
      const pp = V.xs.filter((_, i) => i % step === 0).map((x, j) => [sx(x), sy(V.p[j * step])]);
      // the target is a filled shape with a thin edge, the model a glowing stroke: told apart by form even in a one-hue theme
      Pt.area(pp, base, P.p, 0.34); Pt.line(pp, P.p, 1.4, 0);
      const qq = V.xs.filter((_, i) => i % step === 0).map((x, j) => [sx(x), sy(V.q[j * step])]);
      Pt.area(qq, base, P.q, 0.1); Pt.line(qq, P.q, 2.8, 1);
      // the curves' names at their peaks
      const pk = V.p.indexOf(Math.max(...V.p));
      Pt.tex("p", sx(V.xs[pk]) + 14, sy(V.p[pk]) + 4, 17, P.p);
      Pt.tex("q", sx(V.mu) + 10, Math.max(top + 14, sy(1 / (V.sigma * Math.sqrt(2 * Math.PI))) - 6), 17, P.q);
      // target handles (dragged in the live view): rings you can grab
      if (opts.handles) for (const c of V.target.cs) {
        const hx = sx(c.mu), hy = sy(c.w / (c.sigma * Math.sqrt(2 * Math.PI)));
        Pt.ring?.(hx, hy, 7, P.p);
        opts.handles.push({ x: hx, y: hy, kind: "comp", c });
      }
      const mx = sx(V.mu);
      Pt.dot(mx, base, P.q, 4.5);
      // each term's pull: on μ under the axis, on σ at the half-height points of q (out = wider)
      const yh = sy(0.6065 / (V.sigma * Math.sqrt(2 * Math.PI)));
      pulls?.forEach((g, i) => {
        if (!g || !sim.terms[i].on) return;
        arrow(Pt, mx, base + 18 + i * 16, pullPx(g[0], 130), 0, colors[i]);
        const ds = pullPx(g[1], 46), yy = yh + (i - (sim.terms.length - 1) / 2) * 9;
        for (const side of [-1, 1]) arrow(Pt, sx(V.mu + side * V.sigma), yy, side * ds, 0, colors[i], 2);
      });
      Pt.text(`μ ${V.mu.toFixed(2)}   σ ${V.sigma.toFixed(2)}`, R.x, R.y + R.h - 4, P.q, 14);
      opts.geom && Object.assign(opts.geom, { sx, sy, base, top, ymax, x0, x1 });
      return;
    }
    if (scene === "classes") {
      const K = V.K, top = R.y + 44, base = R.y + R.h - 40, bw = R.w / K;
      const sy = (v) => base - v * (base - top);
      Pt.line([[R.x, base + 0.5], [R.x + R.w, base + 0.5]], P.strong, 1, 0);
      for (let c = 0; c < K; c++) {
        const cx = R.x + c * bw + bw * 0.18, w = bw * 0.64;
        if (P.glow) Pt.rect(cx - 4, sy(V.q[c]) - 4, w + 8, base - sy(V.q[c]) + 4, P.q, 0.16);
        Pt.rect(cx, sy(V.q[c]), w, base - sy(V.q[c]), P.q, 0.8);
        if (V.t[c] > 0) Pt.line([[cx - 3, base], [cx - 3, sy(V.t[c])], [cx + w + 3, sy(V.t[c])], [cx + w + 3, base]], P.p, 1.8, 0.8);
        Pt.text(`${(V.q[c] * 100).toFixed(0)}%`, cx + w / 2, sy(Math.max(V.q[c], V.t[c])) - 10, P.q, 13, "center");
        Pt.text(`z ${V.z[c].toFixed(1)}`, cx + w / 2, base + 20, c === V.y ? P.text : P.faint, 13, "center");
        if (opts.handles) opts.handles.push({ x: cx + w / 2, y: (top + base) / 2, kind: "class", k: c, w, h: base - top });
      }
      // each term's pull on the true class's logit z_y, beside that bar
      const xb = R.x + V.y * bw + bw * 0.86, yb = sy(V.q[V.y]);
      pulls?.forEach((g, i) => { if (g && sim.terms[i].on) arrow(Pt, xb + 6 + i * 12, yb, 0, -pullPx(g[V.y], 70), colors[i]); });
      return;
    }
    // 2D: fit2d, cluster (kmeans / em / softmax partition)
    const [x0, x1, y0, y1] = V.box;
    const k = Math.min(R.w / (x1 - x0), R.h / (y1 - y0)), pw = (x1 - x0) * k, ph = (y1 - y0) * k, ox = R.x + (R.w - pw) / 2, oy = R.y + (R.h - ph) / 2;
    const sx = (x) => ox + (x - x0) * k, sy = (y) => oy + ph - (y - y0) * k;
    const part = scene === "cluster" && sim.cluster?.method === "ce";
    if (scene === "fit2d" || part || sim.cluster?.method === "em") {
      const cv = heat(V, P, part ? "part" : "density", opts.heatW ?? (part ? 200 : 120));
      if (cv) Pt.image(cv, ox, oy, pw, ph);
    }
    Pt.line([[ox, oy], [ox + pw, oy], [ox + pw, oy + ph], [ox, oy + ph], [ox, oy]], P.line, 1, 0);
    Pt.clip(ox, oy, pw, ph, () => {
      const pc = (i) => P.terms[(i + 1) % P.terms.length];
      V.pts.forEach((p, i) => {
        let col = P.p;
        if (V.resp) { let b = 0; V.resp[i].forEach((r, j) => { if (r > V.resp[i][b]) b = j; }); col = pc(b); }
        else if (part) col = pc(p.k % V.k);
        if (P.glow) Pt.dot(sx(p.x), sy(p.y), col, 1.6, false);
        else Pt.dot(sx(p.x), sy(p.y), col, 1.5, false);
      });
      if (V.comps) V.comps.forEach((c, j) => {
        const col = scene === "cluster" ? pc(j) : P.q;
        if (scene !== "cluster" || sim.cluster?.method === "em") for (const r of [1, 2]) {
          const pts = [];
          for (let a = 0; a <= 64; a++) { const u = Math.cos((a / 64) * 2 * Math.PI) * r, v = Math.sin((a / 64) * 2 * Math.PI) * r; pts.push([sx(c.mu[0] + c.L.l11 * u), sy(c.mu[1] + c.L.l21 * u + c.L.l22 * v)]); }
          Pt.line(pts, col, r === 1 ? 2.2 : 1.3, r === 1 ? 1 : 0.4, r === 1 ? null : [5, 6]);
        }
        Pt.dot(sx(c.mu[0]), sy(c.mu[1]), col, scene === "cluster" ? 6 : 4);
        if (scene === "fit2d") Pt.text(`π ${c.w.toFixed(2)}`, sx(c.mu[0]) + 10, sy(c.mu[1]) - 10, P.text, 12);
        // pulls on this component's mean (fit2d: per term; clustering: the next step's move)
        pulls?.forEach((g, i) => {
          if (!g || !sim.terms[i].on) return;
          const s = scene === "cluster" ? k * 3 : 60;
          const gx = scene === "cluster" ? g[j * 6] * s : pullPx(g[j * 6], 60, 0.15), gy = scene === "cluster" ? g[j * 6 + 1] * s : pullPx(g[j * 6 + 1], 60, 0.15);
          arrow(Pt, sx(c.mu[0]), sy(c.mu[1]), clamp(gx, -80, 80), clamp(-gy, -80, 80), colors[i], 2);
        });
      });
      if (opts.handles && V.target) for (const c of V.target.cs) { Pt.dot(sx(c.mu[0]), sy(c.mu[1]), P.p, 3.5, false); opts.handles.push({ x: sx(c.mu[0]), y: sy(c.mu[1]), kind: "comp2", c }); }
    });
    opts.geom && Object.assign(opts.geom, { sx, sy, k, ox, oy, pw, ph, x0, y0 });
  }
  /** Weighted loss over the steps, stacked one colour per term (negative parts below 0), marks at weight changes. */
  function drawStack(Pt, R, hist, upto, total, marks, colors) {
    const P = Pt.P;
    const n = Math.max(1, total);
    const rows = hist.slice(0, upto + 1);
    const pos = rows.map((h) => h.L.reduce((s, v, i) => s + (isNum(v) ? Math.max(0, h.w[i] * v) : 0), 0));
    const neg = rows.map((h) => h.L.reduce((s, v, i) => s + (isNum(v) ? Math.min(0, h.w[i] * v) : 0), 0));
    // the first steps' spike would flatten the rest: the scale follows the bulk (the spike is cut at the top)
    const sorted = pos.slice().sort((a, b) => a - b);
    const q90 = sorted[Math.floor(sorted.length * 0.9)] ?? 0;
    const hi = Math.max(1e-9, Math.min(sorted.at(-1) ?? 0, Math.max(q90 * 1.6, (pos.at(-1) ?? 0) * 1.3))), lo = Math.min(0, ...neg);
    const X = (s) => R.x + (s / n) * R.w, Y = (v) => R.y + R.h - ((v - lo) / (hi - lo)) * R.h;
    Pt.line([[R.x, Y(0) + 0.5], [R.x + R.w, Y(0) + 0.5]], P.line, 1, 0);
    const cut = pos.findIndex((v) => v > hi * 1.001);
    Pt.clip(R.x - 4, R.y, R.w + 8, R.h + 4, () => body());
    // the cut spike: a break mark on the top edge (not a flat cap that reads as data)
    if (cut >= 0) {
      const last = pos.reduce((m, v, i) => (v > hi * 1.001 ? i : m), cut), xa = X(cut), xb = Math.max(X(last), xa + 6);
      Pt.rect(xa - 1, R.y - 7, xb - xa + 2, 9, P.bg);
      for (const yy of [R.y - 5, R.y]) Pt.line([[xa - 3, yy + 3], [xa + 3, yy - 3]], P.faint, 1.2, 0), Pt.line([[xb - 3, yy + 3], [xb + 3, yy - 3]], P.faint, 1.2, 0);
    }
    for (const s of new Set(marks ?? [])) if (s > 0 && s <= upto) Pt.line([[X(s), R.y], [X(s), R.y + R.h]], P.faint, 1, 0, [2, 4]);
    function body() {
      // thin the history to ≤ 400 columns
      const every = Math.max(1, Math.ceil(rows.length / 400));
      const idx = rows.map((_, i) => i).filter((i) => i % every === 0 || i === rows.length - 1);
      let upP = idx.map(() => 0), dnP = idx.map(() => 0);
      const nT = Math.max(0, ...rows.map((h) => h.L.length));
      for (let j = 0; j < nT; j++) {
        const v = idx.map((i) => { const h = rows[i]; const x = h.L[j]; return isNum(x) ? h.w[j] * x : 0; });
        const up2 = upP.map((u, m) => u + Math.max(0, v[m])), dn2 = dnP.map((d, m) => d + Math.min(0, v[m]));
        band(idx.map((i, m) => [X(i), Y(up2[m])]), idx.map((i, m) => [X(i), Y(upP[m])]), colors[j]);
        if (v.some((x) => x < 0)) band(idx.map((i, m) => [X(i), Y(dn2[m])]), idx.map((i, m) => [X(i), Y(dnP[m])]), colors[j]);
        upP = up2; dnP = dn2;
      }
      function band(a, b, col) {
        // a polygon: a forward, b backward
        const pts = [...a, ...b.slice().reverse()];
        if (pts.length < 3) return;
        polyFill(Pt, pts, col, 0.55);
      }
      const tot = idx.map((i) => { const h = rows[i]; return h.L.reduce((s, x, j) => s + (isNum(x) ? h.w[j] * x : 0), 0); });
      Pt.line(idx.map((i, m) => [X(i), Y(tot[m])]), P.text, 1.4, 0.6);
      if (rows.length) Pt.dot(X(upto), Y(tot.at(-1)), P.text, 3, false);
    }
  }
  function polyFill(Pt, pts, col, a) {
    if (Pt.ctx) {
      const c = Pt.ctx; c.beginPath(); pts.forEach(([x, y], i) => (i ? c.lineTo(x, y) : c.moveTo(x, y))); c.closePath(); c.fillStyle = rgba(col, a); c.fill();
    } else Pt.out.push(`<path d="M${pts.map(([x, y]) => `${Math.round(x * 100) / 100} ${Math.round(y * 100) / 100}`).join("L")}Z" fill="${col}" fill-opacity="${a}"/>`);
  }
  /** A row's pull glyph: the pull on the main parameter (fit1d μ, classes z_y) as an arrow from the centre line. */
  function rowPull(Pt, W, H, scene, g, yi, col) {
    const cx = W / 2;
    Pt.line([[cx, 3], [cx, H - 3]], Pt.P.line, 1, 0);
    if (!g) return;
    if (scene === "fit1d") { arrow(Pt, cx, H / 2, pullPx(g[0], cx - 4), 0, col, 2.2); return; }
    if (scene === "classes") arrow(Pt, cx, H / 2, pullPx(g[yi] ?? 0, cx - 4), 0, col, 2);
  }
  /** GIF / file only: the rows on the canvas (the live view has DOM rows). */
  function drawRows(Pt, R, sim, row, pulls, colors, recent) {
    const P = Pt.P;
    const h = Math.min(78, R.h / Math.max(1, sim.terms.length));
    if (sim.terms.length === 1) {
      // one objective: its LaTeX and its value, nothing to weigh
      const v = row.L[0];
      Pt.tex(termTex(sim, sim.terms[0], P), R.x, R.y + 20, 18, P.text);
      Pt.text(isNum(v) ? v.toFixed(3) : "—", R.x + R.w, R.y + 20, colors[0], 14, "right");
      return;
    }
    sim.terms.forEach((t, i) => {
      const y = R.y + i * h, w = row.w[i] ?? 0, on = t.on && w > 0, col = colors[i];
      if (recent?.(i)) Pt.rect(R.x - 8, y - 4, R.w + 16, h - 6, col, 0.1);
      Pt.dot(R.x + 6, y + 14, on ? col : P.strong, 5, !on);
      Pt.tex(weightTex(t, i), R.x + 22, y + 20, 18, on ? col : P.faint);
      Pt.text(w.toFixed(2), R.x + 104, y + 20, on ? P.text : P.faint, 14, "right");
      let tt = "";
      try { tt = termTex(sim, t, P); } catch {}
      Pt.alpha(on ? 1 : 0.4, () => Pt.tex(tt, R.x + 122, y + 21, 18, P.text));
      const tx = R.x + 22, tw = 82, ty = y + 38, mw = Math.max(0, ...sim.terms.map((x) => x.w), 1);
      Pt.rect(tx, ty, tw, 3, P.line);
      Pt.rect(tx, ty, tw * Math.min(1, w / mw), 3, on ? col : P.strong);
      Pt.dot(tx + tw * Math.min(1, w / mw), ty + 1.5, on ? col : P.strong, 4, false);
      Pt.alpha(on ? 1 : 0.35, () => {
        const o = { ...Pt, line: (pts, c, w, gl, d) => Pt.line(pts.map(([x, yy]) => [x + R.x + 128, yy + ty - 12]), c, w, gl, d) };
        rowPull(o, 130, 24, sim.scene, pulls?.[i], (sim.classes?.y ?? 1) - 1, col);
      });
      const v = row.L[i];
      Pt.text(isNum(v) ? (w * v).toFixed(3) : "—", R.x + R.w, y + 20, on ? col : P.faint, 14, "right");
    });
  }
  /** The whole page of a frame (GIF, file): head, animation, rows, stack. */
  function drawPage(Pt, eq, run, k, snapK, colors) {
    const P = Pt.P, sim = run.sim;
    Pt.clear(VW, VH);
    Pt.tex(headTex(eq, P), 30, 40, 20, P.text);
    Pt.text(`step ${String(k).padStart(3)}`, VW - 30, 34, P.faint, 13, "right");
    const pulls = run.pulls();
    drawScene(Pt, { x: 30, y: 70, w: 520, h: 440 }, sim, run.view(), pulls, colors, { heatW: 150, ymax: run.ymaxNow });
    const row = run.hist[k] ?? run.hist.at(-1);
    const marks = run.marks;
    const recent = (i) => marks.some((s) => s > 0 && k >= s && k < s + 18 && (run.hist[s - 1]?.w[i] ?? 0) !== (run.hist[s]?.w[i] ?? 0));
    const rowsH = Math.min(300, 82 * sim.terms.length);
    drawRows(Pt, { x: 580, y: 80, w: 350, h: rowsH }, sim, row, pulls, colors, recent);
    const sy0 = 80 + rowsH + 28;
    drawStack(Pt, { x: 580, y: sy0, w: 350, h: 500 - sy0 }, run.hist, k, snapK ?? sim.opt.steps, marks, colors);
  }

  // ── the run for a file: the scripted run (plan) to its end, cached by spec ──
  const runs = new Map();
  function scriptedRun(eq) {
    const key = JSON.stringify(eq.sim);
    if (runs.has(key)) return runs.get(key);
    if (runs.size > 6) runs.clear();
    const run = L.create(eq.sim).scripted();
    const snaps = [run.snapshot()];
    for (let s = 0; s < run.sim.opt.steps; s++) { run.step(1); snaps.push({ theta: run.theta.slice(), step: run.stepNo }); }
    const r = { run, snaps, hist: run.hist.slice(), marks: run.marks.slice(), terms: clone(run.sim.terms) };
    // the y range of each step: what it needs, eased (the same easing as the live view)
    if (run.scene === "fit1d") {
      const need = snaps.map((sn) => { run.restore({ ...snaps[0], theta: sn.theta, step: 0, hist: [] }); return yNeed(run.view()); });
      r.ymax = [];
      need.forEach((v, k) => r.ymax.push(k ? r.ymax[k - 1] + (v - r.ymax[k - 1]) * EASE : v));
    }
    runs.set(key, r);
    return r;
  }
  /** The run positioned at step k (θ and the weights then; history cut at k). */
  function at(R, k) {
    const { run } = R;
    run.restore({ ...R.snaps[0], theta: R.snaps[k].theta, step: k, hist: R.hist.slice(0, k + 1), marks: R.marks.filter((s) => s <= k), terms: clone(R.terms) });
    // the weights in force at k
    const w = R.hist[k]?.w;
    if (w) run.sim.terms.forEach((t, i) => { t.w = w[i]; t.on = w[i] > 0 || t.on; });
    run.ymaxNow = R.ymax?.[k];
    return run;
  }
  const EASE = 0.12; // the 1D y range follows what a frame needs at this rate (per frame / per step)
  const FRAME = new Map(); // a GIF frame's step, by token (the spec is cloned through LP.render)

  /** The `equation` chart for a sim panel: the page in the figure's style inside the panel box. */
  function chart(p, st) {
    const eq = p.eq;
    return {
      polar: true, free: true, cats: [], legend: [], points: [],
      draw(c) {
        const P = docPal(st);
        const R = scriptedRun(eq);
        const k = FRAME.has(eq._frame) ? FRAME.get(eq._frame) : R.run.sim.opt.steps;
        const run = at(R, Math.min(k, R.snaps.length - 1));
        const s = Math.min(c.box.w / VW, c.box.h / VH);
        const ox = c.box.x + (c.box.w - VW * s) / 2, oy = c.box.y + (c.box.h - VH * s) / 2;
        const Pt = svgPainter(c, P);
        drawPage(Pt, eq, run, run.stepNo, R.run.sim.opt.steps, run.sim.terms.map((t, i) => termColor(P, t, i)));
        c.out.push(`<g data-part="sim" transform="translate(${ox} ${oy}) scale(${s})">${Pt.out.join("")}</g>`);
        if (Pt.heat) FRAME.set(`${eq._frame}:heat`, [ox + Pt.heat[0] * s, oy + Pt.heat[1] * s, Pt.heat[2] * s, Pt.heat[3] * s]);
      },
    };
  }

  /** GIF frames: the scripted run, F frames paced by visual change (75 %) and time (25 %), the end held. */
  async function frames(spec, opts = {}) {
    const pi = LP.eq.panelIndex(spec);
    const eq = spec.panels[pi].eq;
    const R = scriptedRun(eq);
    const n = R.snaps.length - 1;
    const heatScene = eq.sim.scene === "fit2d" || eq.sim.cluster?.method === "em" || eq.sim.cluster?.method === "ce";
    // heatmaps change everywhere every frame: fewer, narrower frames keep the file reasonable
    const F = Math.min(200, eq.sim.frames ?? (heatScene ? 60 : 100));
    // visual change per step, measured on a small canvas in the file's palette
    const st = LP.styleFor(spec.style, opts.styles ?? [], spec.set);
    const P = docPal(st);
    const cv = document.createElement("canvas");
    cv.width = 240; cv.height = 135;
    const ctx = cv.getContext("2d", { willReadFrequently: true });
    const Pt = canvasPainter(ctx, P);
    const colors = R.terms.map((t, i) => termColor(P, t, i));
    const diffs = [0];
    let prev = null;
    for (let k = 0; k <= n; k++) {
      ctx.setTransform(0.25, 0, 0, 0.25, 0, 0);
      drawPage(Pt, eq, at(R, k), k, n, colors);
      const px = ctx.getImageData(0, 0, 240, 135).data;
      if (prev) { let s = 0; for (let i = 0; i < px.length; i += 4) s += Math.abs(px[i] - prev[i]) + Math.abs(px[i + 1] - prev[i + 1]) + Math.abs(px[i + 2] - prev[i + 2]); diffs.push(s); }
      prev = px;
    }
    const sched = L.schedule(diffs, F, 0.25);
    const token = `f${Math.random().toString(36).slice(2)}`;
    const svgs = [];
    let heatR = null;
    try {
      for (const k of sched) {
        FRAME.set(token, k);
        const s = clone(spec);
        s.panels[pi].eq._frame = token;
        const r = await LP.render(s, opts);
        svgs.push(r.svg);
        heatR ??= FRAME.get(`${token}:heat`);
      }
    } finally { FRAME.delete(token); }
    const vb = /viewBox="([\d.\s-]+)"/.exec(svgs[0] ?? "")?.[1]?.split(/\s+/).map(Number);
    const dither = heatR && vb ? [[heatR[0] / vb[2], heatR[1] / vb[3], heatR[2] / vb[2], heatR[3] / vb[3]]] : [];
    const ms = eq.sim.ms ?? 70;
    return { svgs, delays: L.delays(svgs.length, eq.sim.frames ? ms : heatScene ? Math.round(ms * 1.5) : ms, eq.sim.hold ?? 1600), dither, ms, sched, maxW: heatScene ? 960 : 1200 };
  }

  // ── the live view ──
  const SPEEDS = [1, 2, 4];
  function mount(host, spec, opts = {}) {
    const role = opts.role ?? "full";
    const h = (tag, attrs = {}, ...kids) => {
      const el = document.createElement(tag);
      for (const [k, v] of Object.entries(attrs)) { if (v == null || v === false) continue; if (k.startsWith("on")) el[k] = v; else if (k === "class") el.className = v; else el.setAttribute(k, v === true ? "" : v); }
      for (const c of kids.flat()) if (c != null) el.append(c);
      return el;
    };
    host.textContent = "";
    const wrap = h("div", { class: "lp-sim", "data-role": role });
    const head = h("div", { class: "lp-sim-head" });
    const stepEl = h("span", { class: "lp-sim-step" });
    const cv = h("canvas", { class: "lp-sim-cv" });
    const main = h("div", { class: "lp-sim-main" }, cv);
    const rowsEl = h("div", { class: "lp-sim-rows" });
    const lcv = h("canvas", { class: "lp-sim-loss" });
    const side = h("div", { class: "lp-sim-side" }, rowsEl, h("div", { class: "lp-sim-lossbox" }, lcv));
    const bar = h("div", { class: "lp-sim-bar" });
    wrap.append(h("div", { class: "lp-sim-top" }, head, stepEl), main, side);
    if (role !== "view") wrap.append(bar);
    host.append(wrap);
    const S = { eq: null, run: null, playing: false, speed: 1, raf: 0, idle: 0, P: themePal(), handles: [], geom: {}, drag: null, rowEls: [], last: 0 };
    const ctx = cv.getContext("2d"), lctx = lcv.getContext("2d");
    const colors = () => S.run.sim.terms.map((t, i) => termColor(S.P, t, i));

    function fit(c) {
      const r = c.getBoundingClientRect(), dpr = Math.min(2, window.devicePixelRatio || 1);
      const W = Math.max(50, Math.floor(r.width)), H = Math.max(40, Math.floor(r.height));
      if (c.width !== W * dpr || c.height !== H * dpr) { c.width = W * dpr; c.height = H * dpr; }
      return { W, H, dpr };
    }
    function emitFull() { S.sent = S.run.hist.length; opts.onState?.({ t: "full", eq: clone(S.eq), snap: S.run.snapshot(), playing: S.playing, speed: S.speed }); }
    /** Every history row since the last message (2× / 4× add several per frame); `terms` when weights changed. */
    function emitTick(terms = false) {
      if (!opts.onState) return;
      const from = Math.min(S.sent ?? 0, S.run.hist.length);
      opts.onState({ t: "tick", step: S.run.stepNo, theta: S.run.theta.slice(), from, rows: S.run.hist.slice(from), marks: S.run.marks.slice(), playing: S.playing, ...(terms ? { terms: clone(S.run.sim.terms) } : {}) });
      S.sent = S.run.hist.length;
    }

    // ── rows (DOM) ──
    function buildRows() {
      rowsEl.textContent = "";
      const sim = S.run.sim, C = colors();
      const mw = (t) => Math.max(2, Math.ceil(Math.max(t.w, 1)));
      S.rowEls = sim.terms.map((t, i) => {
        const dotB = h("button", { type: "button", class: "lp-sim-on", "aria-label": "開／關", "aria-pressed": String(t.on) });
        dotB.style.setProperty("--c", C[i]);
        const name = h("span", { class: "lp-sim-w" });
        name.innerHTML = katexHtml(`\\textcolor{${C[i]}}{${weightTex(t, i)}}`);
        const val = h("span", { class: "lp-sim-wv" }, t.w.toFixed(2));
        const sl = h("input", { type: "range", min: "0", max: String(mw(t)), step: "0.01", value: String(t.w), "aria-label": "權重" });
        sl.style.setProperty("--c", C[i]);
        const tex = h("span", { class: "lp-sim-tex" });
        try { tex.innerHTML = katexHtml(termTex(sim, t, S.P)); } catch { tex.textContent = t.kind; }
        const pull = h("canvas", { class: "lp-sim-pull" });
        const wv = h("span", { class: "lp-sim-val" });
        wv.style.color = C[i];
        const rm = role !== "view" && sim.terms.length > 1 ? h("button", { type: "button", class: "lp-sim-rm", "aria-label": "移除", onclick: () => { S.run.removeTerm(i); buildRows(); emitFull(); opts.onEdit?.(); wake(); } }, "×") : null;
        const meter = h("span", { class: "lp-sim-meter" }, h("i"));
        meter.style.setProperty("--c", C[i]);
        const row = h("div", { class: "lp-sim-row" }, dotB, name, h("span", { class: "lp-sim-sl" }, role === "view" ? meter : sl, val), tex, pull, wv, rm);
        if (role === "view") { sl.disabled = true; dotB.disabled = true; }
        if (sim.scene === "cluster" && sim.cluster?.method !== "ce") { row.classList.add("algo"); sl.disabled = dotB.disabled = true; }
        dotB.onclick = () => { S.run.toggle(i); dotB.setAttribute("aria-pressed", String(S.run.sim.terms[i].on)); emitTick(true); opts.onEdit?.(); wake(); };
        // while dragging: the weight only (one mark when the slider is let go)
        sl.oninput = () => { S.run.setWeight(i, +sl.value, false); val.textContent = (+sl.value).toFixed(2); emitTick(true); wake(); };
        sl.onchange = () => { S.run.mark(); emitTick(true); opts.onEdit?.(); };
        sl.onkeydown = (e) => e.stopPropagation();
        return { row, sl, val, pull, wv, dotB, meter, mw: mw(t) };
      });
      for (const r of S.rowEls) rowsEl.append(r.row);
      // one objective: nothing to weigh, its LaTeX gets the room
      rowsEl.classList.toggle("one", sim.terms.length === 1);
      if (role !== "view" && (sim.scene !== "cluster" || sim.cluster?.method === "ce")) {
        const add = h("button", { type: "button", class: "ghost small lp-sim-add" }, h("span", { "data-icon": "plus" }), "加一項");
        add.onclick = (e) => addMenu(e.currentTarget);
        rowsEl.append(add);
        if (sim.terms.length >= L.LIMITS.terms) add.disabled = true;
      }
      head.innerHTML = katexHtml(headTex(S.eq, S.P));
      if (typeof window.paintIcons === "function") window.paintIcons(wrap);
    }
    function addMenu(anchor) {
      wrap.querySelector(".lp-sim-menu")?.remove();
      const kinds = Object.entries(L.TERMS[S.run.scene]).filter(([k]) => S.run.scene !== "cluster" || (S.run.sim.cluster?.method === "ce" ? ["ce", "l2", "smooth"].includes(k) : false));
      const menu = h("div", { class: "lp-sim-menu", role: "menu" });
      for (const [k, d] of kinds) {
        const b = h("button", { type: "button", role: "menuitem" });
        if (d.tex) b.innerHTML = katexHtml(fill(d.tex, S.P));
        else b.textContent = d.label;
        b.title = d.label;
        b.onclick = () => {
          if (k === "expr") return exprInput(); // the menu turns into the formula field
          menu.remove();
          const t = { kind: k, w: 0.5 };
          if (k === "anchor") t.mu0 = +(S.run.view().mu - 2).toFixed(1);
          if (k === "smooth") t.eps = 0.1;
          // a sample term needs samples: drawn from the target (fixed seed) the first time one is added
          if (k === "nll" && !(S.run.sim.samples?.n > 0)) { S.run.sim.samples = { n: 300, seed: 7 }; S.run.setTarget(S.run.sim.target); }
          addTerm(t);
        };
        menu.append(b);
      }
      /** A user term: typed in the safe expression language, checked as it is typed (no native dialogs). */
      function exprInput() {
        const names = L.EXPR_NAMES[S.run.scene] ?? [];
        const inp = h("input", { type: "text", class: "set-input lp-sim-expr", spellcheck: "false", placeholder: S.run.scene === "classes" ? "sum(z^2)" : "(mu - 1)^2", "aria-label": names.join(", ") });
        const msg = h("div", { class: "lp-sim-err" });
        menu.textContent = "";
        menu.append(h("div", { class: "lp-sim-names" }, names.join("  ")), inp, msg);
        inp.onkeydown = (e) => {
          e.stopPropagation();
          if (e.key === "Escape") return menu.remove();
          if (e.key !== "Enter") return;
          try { E.parse(inp.value, names); } catch (err) { msg.textContent = err.message; inp.setAttribute("aria-invalid", "true"); return; }
          menu.remove();
          addTerm({ kind: "expr", w: 0.5, expr: inp.value });
        };
        inp.oninput = () => { msg.textContent = ""; inp.removeAttribute("aria-invalid"); };
        setTimeout(() => inp.focus());
      }
      function addTerm(t) {
        const used = new Set(S.run.sim.terms.map((x) => x.name));
        t.name = ["a", "b", "c", "d", "e", "f", "g", "h"].find((n) => !used.has(n));
        if (S.run.addTerm(t)) { buildRows(); emitFull(); opts.onEdit?.(); wake(); }
      }
      wrap.append(menu);
      const r = anchor.getBoundingClientRect(), w = wrap.getBoundingClientRect();
      menu.style.left = `${r.left - w.left}px`;
      menu.style.top = `${r.bottom - w.top + 4}px`;
      setTimeout(() => document.addEventListener("pointerdown", function off(e) { if (!menu.contains(e.target)) { menu.remove(); document.removeEventListener("pointerdown", off, true); } }, true));
    }

    // ── bar ──
    function buildBar() {
      bar.textContent = "";
      const btn = (icon, label, fn, cls = "ico") => { const b = h("button", { type: "button", class: cls, "data-icon": icon, "aria-label": label }); b.onclick = fn; return b; };
      S.playB = btn(S.playing ? "pause" : "play", S.playing ? "暫停" : "播放", () => api.play(!S.playing));
      const seg = h("div", { class: "seg lp-sim-speed" }, ...SPEEDS.map((k) => h("button", { type: "button", "aria-pressed": String(S.speed === k), onclick: () => { api.speed(k); buildBar(); } }, `${k}×`)));
      const one = h("button", { type: "button", class: "ghost lp-sim-one", "aria-label": "走一步" }, "+1");
      one.onclick = () => api.stepOnce();
      bar.append(S.playB, one, btn("refresh", "重來", () => api.reset()), seg);
      if (opts.onClose) bar.append(h("span", { class: "grow" }), h("button", { type: "button", class: "ghost lp-sim-close", onclick: () => opts.onClose() }, h("span", { "data-icon": "x" }), "回到投影片"));
      if (typeof window.paintIcons === "function") window.paintIcons(bar);
    }

    // ── drawing ──
    function draw() {
      const t0 = performance.now();
      const { W, H, dpr } = fit(cv);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const tk = clamp(Math.max(W / 760, H / 560), 1, 2); // readouts grow with the screen (audience, projector)
      const Pt2 = canvasPainter(ctx, { ...S.P, see: true }, tk);
      Pt2.clear(W, H);
      S.handles = role === "view" ? null : [];
      S.geom = {};
      const pulls = S.run.pulls(), C = colors();
      const V = S.run.view();
      if (S.run.scene === "fit1d") { const need = yNeed(V); S.ymax = S.ymax ? S.ymax + (need - S.ymax) * EASE : need; S.easing = Math.abs(need - S.ymax) > need * 0.01; }
      drawScene(Pt2, { x: 24, y: 12, w: W - 48, h: H - 24 }, S.run.sim, V, pulls, C, { handles: S.handles, geom: S.geom, ymax: S.ymax });
      // the stack
      const l = fit(lcv);
      lctx.setTransform(l.dpr, 0, 0, l.dpr, 0, 0);
      const Lp = canvasPainter(lctx, { ...S.P, see: true }, tk);
      Lp.clear(l.W, l.H);
      drawStack(Lp, { x: 4, y: 8, w: l.W - 8, h: l.H - 16 }, S.run.hist, S.run.hist.length - 1, Math.max(S.run.sim.opt.steps, S.run.hist.length - 1), S.run.marks, C);
      // rows: values and pulls
      const row = S.run.hist.at(-1);
      S.rowEls.forEach((r, i) => {
        const v = row?.L[i], w = row?.w[i] ?? 0;
        r.wv.textContent = isNum(v) ? (w * v).toFixed(3) : "—";
        const t = S.run.sim.terms[i];
        if (t && document.activeElement !== r.sl && Math.abs(+r.sl.value - t.w) > 1e-9) { r.sl.value = String(t.w); r.val.textContent = t.w.toFixed(2); }
        if (t) r.meter.firstChild.style.width = `${Math.min(100, (100 * t.w) / Math.max(r.mw, t.w))}%`;
        r.dotB.setAttribute("aria-pressed", String(!!t?.on));
        r.row.classList.toggle("off", !(t?.on && t.w > 0));
        const p = fit(r.pull), pc = r.pull.getContext("2d");
        pc.setTransform(p.dpr, 0, 0, p.dpr, 0, 0);
        pc.clearRect(0, 0, p.W, p.H);
        rowPull(canvasPainter(pc, { ...S.P, dot: null }), p.W, p.H, S.run.scene, pulls[i], S.run.scene === "classes" ? S.run.view().y : 0, C[i]);
      });
      stepEl.textContent = `step ${S.run.stepNo}`;
      stats.draw = performance.now() - t0;
    }
    let lastT = 0;
    function frame(now) {
      S.raf = 0;
      if (lastT) { const dt = now - lastT; if (dt < 200) { stats.ms = (stats.ms * stats.frames + dt) / (stats.frames + 1); stats.frames++; } }
      if (S.playing && role !== "view") {
        const t0 = performance.now();
        const moved = advance(S.speed);
        stats.step = performance.now() - t0;
        S.idle = moved < 1e-7 ? S.idle + 1 : 0;
      }
      draw();
      const more = (S.playing && role !== "view" && S.idle < 40) || S.easing;
      lastT = more ? now : 0;
      if (more) S.raf = requestAnimationFrame(frame);
    }
    /** n steps, then one message with every new row (what a frame at n× does). → how far θ moved */
    function advance(n) {
      const before = S.run.theta.slice();
      S.run.step(n);
      emitTick();
      return S.run.theta.reduce((s, v, j) => s + Math.abs(v - before[j]), 0);
    }
    function wake() { S.idle = 0; if (!S.raf) S.raf = requestAnimationFrame(frame); }
    // narrow hosts (the editor's sheet): the rows go under the animation, beside the stack
    const ro = new ResizeObserver(() => { wrap.classList.toggle("lp-sim-narrow", wrap.clientWidth < 1100); wake(); });
    ro.observe(wrap);

    // ── dragging the target ──
    function pos(e) { const r = cv.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; }
    cv.addEventListener("pointerdown", (e) => {
      if (role === "view" || !S.handles) return;
      const [x, y] = pos(e);
      let best = null, bd = 14 * 14;
      for (const hd of S.handles) {
        if (hd.kind === "class") { if (Math.abs(x - hd.x) < hd.w / 2 + 4 && Math.abs(y - hd.y) < hd.h / 2) best = hd; continue; }
        const d = (x - hd.x) ** 2 + (y - hd.y) ** 2;
        if (d < bd) { bd = d; best = hd; }
      }
      if (!best) return;
      e.preventDefault();
      if (best.kind === "class") {
        S.run.sim.classes = { ...(S.run.sim.classes ?? {}), y: best.k + 1 };
        S.run.setTarget(S.run.sim.target);
        emitFull(); opts.onEdit?.(); wake();
        return;
      }
      const tg = clone(S.run.sim.target), i = S.run.view().target.cs.indexOf(best.c);
      S.drag = { i, tg, x0: x, y0: y };
      cv.setPointerCapture(e.pointerId);
    });
    cv.addEventListener("pointermove", (e) => {
      if (!S.drag) {
        if (role !== "view" && S.handles) { const [x, y] = pos(e); cv.style.cursor = S.handles.some((hd) => hd.kind === "class" ? Math.abs(x - hd.x) < hd.w / 2 && Math.abs(y - hd.y) < hd.h / 2 : (x - hd.x) ** 2 + (y - hd.y) ** 2 < 196) ? "grab" : ""; }
        return;
      }
      const [x, y] = pos(e), G = S.geom, c = S.drag.tg.mix[S.drag.i];
      if (S.run.scene === "fit1d") {
        c.mu = +clamp(G.x0 + ((x - 24) / (cv.clientWidth - 48)) * (G.x1 - G.x0), G.x0, G.x1).toFixed(2);
        // height ↔ weight: the peak follows the pointer
        const peak = clamp((G.base - y) / (G.base - G.top), 0.02, 1) * G.ymax;
        c.w = +Math.max(0.02, peak * c.sigma * Math.sqrt(2 * Math.PI)).toFixed(3);
      } else {
        c.mu = [+(G.x0 + (x - G.ox) / G.k).toFixed(2), +(G.y0 + (G.oy + G.ph - y) / G.k).toFixed(2)];
      }
      S.run.setTarget(clone(S.drag.tg));
      wake();
    });
    const up = () => { if (S.drag) { S.drag = null; emitFull(); opts.onEdit?.(); } };
    cv.addEventListener("pointerup", up);
    cv.addEventListener("pointercancel", up);
    cv.addEventListener("wheel", (e) => {
      if (role === "view" || !S.handles || S.run.scene !== "fit1d") return;
      const [x, y] = pos(e);
      const hd = S.handles.find((q) => q.kind === "comp" && (x - q.x) ** 2 + (y - q.y) ** 2 < 400);
      if (!hd) return;
      e.preventDefault();
      const tg = clone(S.run.sim.target), i = S.run.view().target.cs.indexOf(hd.c);
      tg.mix[i].sigma = +clamp((tg.mix[i].sigma ?? 1) * Math.exp(e.deltaY * 0.002), 0.08, 4).toFixed(3);
      S.run.setTarget(tg);
      emitFull(); wake();
      clearTimeout(S.wheelT);
      S.wheelT = setTimeout(() => opts.onEdit?.(), 400); // one edit per wheel gesture
    }, { passive: false });

    const api = {
      update(sp) {
        const p = sp?.panels?.[LP.eq.panelIndex(sp)];
        if (!p?.eq?.sim) return;
        S.eq = clone(p.eq);
        S.run = L.create(S.eq.sim);
        S.ymax = 0;
        S.eq.sim = S.run.sim; // one object: the rows edit what the run uses
        S.P = themePal();
        buildRows();
        buildBar();
        wake();
        // the rows' maths: KaTeX may still be loading (the editor loads it on demand)
        if (!window.katex && LP.loadKatex) LP.loadKatex().then(() => { buildRows(); wake(); }).catch(() => {});
      },
      play(on = true) { S.playing = !!on; buildBar(); wake(); emitTick(); },
      toggle() { api.play(!S.playing); },
      advance: (n = 1) => { advance(n); wake(); },
      stepOnce() { S.playing = false; S.run.step(1); buildBar(); emitTick(); wake(); },
      reset() { S.run.reset(); S.ymax = 0; emitFull(); opts.onEdit?.(); wake(); },
      speed(k) { S.speed = SPEEDS.includes(k) ? k : 1; },
      /** The full state (the audience's first message, a re-sync). */
      state() { S.sent = S.run.hist.length; return { t: "full", eq: clone(S.eq), snap: S.run.snapshot(), playing: S.playing, speed: S.speed }; },
      /** Audience: a message from the presenter. */
      apply(m) {
        if (!m) return;
        if (m.t === "full" || !S.run) {
          if (!m.eq) return;
          S.eq = clone(m.eq);
          S.run = L.create(S.eq.sim).restore(m.snap);
          S.eq.sim = S.run.sim;
          S.P = themePal();
          buildRows();
        } else if (m.t === "tick") S.run.follow(m);
        S.playing = !!m.playing;
        if (m.speed) S.speed = m.speed;
        if (role !== "view") buildBar(); // a presenter taking over (對調螢幕) gets the right ▶/⏸
        wake();
      },
      /** The sim spec as it is now (weights, terms, target) — the editor keeps it. */
      sim: () => clone(S.run.sim),
      run: () => S.run,
      redraw: () => wake(),
      destroy() { cancelAnimationFrame(S.raf); ro.disconnect(); wrap.remove(); },
    };
    if (spec) api.update(spec);
    return api;
  }

  /**
   * A curve figure (tier 1) live in a show: eqlive.js's view plus one slider per parameter (presenter / single
   * screen); the audience gets the values and only renders. Same contract as mount(): { apply, state, destroy }.
   */
  function mountCurve(host, spec, opts = {}) {
    const role = opts.role ?? "full";
    host.textContent = "";
    const wrap = document.createElement("div");
    wrap.className = "lp-sim lp-sim-curve";
    wrap.dataset.role = role;
    const main = document.createElement("div");
    main.className = "lp-sim-main lp-sim-curvebox";
    const side = document.createElement("div");
    side.className = "lp-sim-side lp-sim-params";
    wrap.append(main, side);
    if (opts.onClose && role !== "view") {
      const bar = document.createElement("div");
      bar.className = "lp-sim-bar";
      const x = document.createElement("button");
      Object.assign(x, { type: "button", className: "ghost lp-sim-close", onclick: () => opts.onClose() });
      x.innerHTML = `<span data-icon="x"></span>回到投影片`;
      bar.append(x);
      wrap.append(bar);
      window.paintIcons?.(bar);
    }
    host.append(wrap);
    let cur = clone(spec);
    const pi = LP.eq.panelIndex(cur);
    const live = LP.eqLive.mount(main, {});
    const params = () => (cur.panels[pi].eq.vars ?? []).filter((v) => v.role === "param");
    function rows() {
      side.textContent = "";
      const C = LP.eqColors.colorsFor(cur.panels[pi].eq, LP.eqEditor.themePalette());
      for (const v of params()) {
        const row = document.createElement("label");
        row.className = "lp-sim-prow";
        const name = document.createElement("span");
        name.className = "lp-sim-w";
        name.innerHTML = katexHtml(`\\textcolor{${C[v.name]}}{${String(v.label || v.name).replace(/\$/g, "").replace(/（.*?）/g, "")}}`);
        const sl = document.createElement("input");
        Object.assign(sl, { type: "range", min: String(v.min), max: String(v.max), step: String(v.step ?? (v.max - v.min) / 100), value: String(v.value) });
        sl.style.setProperty("--c", C[v.name]);
        sl.disabled = role === "view";
        const val = document.createElement("span");
        val.className = "lp-sim-wv";
        val.textContent = LP.eqFmt(v, v.value);
        sl.oninput = () => { v.value = +sl.value; val.textContent = LP.eqFmt(v, v.value); live.update(cur); opts.onState?.({ t: "curve", spec: clone(cur) }); };
        sl.onkeydown = (e) => e.stopPropagation();
        row.append(name, sl, val);
        side.append(row);
      }
    }
    rows();
    live.update(cur);
    return {
      state: () => ({ t: "curve", spec: clone(cur) }),
      apply(m) { if (m?.spec) { cur = clone(m.spec); rows(); live.update(cur); } },
      destroy() { live.destroy(); wrap.remove(); },
    };
  }
  /** Colour roles of the terms from lore-notation (the weight's symbol, e.g. \lambda_1, registered with a colour). */
  async function assignRoles(api, spec, notation) {
    const eq = spec?.panels?.[LP.eq.panelIndex(spec)]?.eq;
    if (!api || !eq?.sim) return;
    // only the author's own weight symbols (λ₁, β …): a generated a, b, c is no symbol of the registry
    const syms = eq.sim.terms.map((t, i) => (t.name && !/^[a-h]$/.test(t.name) ? weightTex(t, i) : ""));
    try {
      const r = await fetch(`${api}/equation/roles`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ symbols: syms, notation: { entries: notation ?? [] } }) });
      if (!r.ok) return;
      const { roles } = await r.json();
      eq.sim.terms.forEach((t, i) => { if (isNum(roles?.[i]) && !isNum(t.color)) t.color = roles[i]; });
    } catch {}
  }
  /** A figure spec for a catalogue scene. */
  async function starter(id, style) {
    const cat = await LP.eqCatalogue();
    const s = (cat.scenes ?? []).find((x) => x.id === id) ?? cat.scenes?.[0];
    if (!s) return null;
    const spec = LP.eq.specFor({ template: s.id, expr: "", latex: s.latex, sim: clone(s.sim), reduction: [], properties: [] }, style);
    spec.size = { w: 240, h: 135 };
    return spec;
  }
  /** Scenes that fit a paper's LaTeX (lore-plot `equation::sim::suggest`, no model). */
  async function suggest(api, latex) {
    try { const r = await fetch(`${api}/equation/scenes`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ latex }) }); return r.ok ? (await r.json()).scenes ?? [] : []; } catch { return []; }
  }

  /**
   * 動起來 from a LaTeX equation (LoreReader's ✦, KnowledgeBase's card equations): a composite loss or a distribution
   * objective first offers its simulation scenes next to `anchor` (曲線 = the curve path); the curve path asks the model
   * (`equation-plot`, when `propose`) and opens the editor — on the catalogue with the LaTeX when there is no answer.
   * `notation`: lore-notation entries whose colour roles the figure takes.
   */
  LP.animateLatex = async function ({ latex = "", symbols = [], context = "", title = "", anchor = null, api = "/plot/api", notation = [], propose = true }) {
    const toast = (m, o) => window.LoreShell?.toast?.(m, o);
    const scenes = latex ? await suggest(api, latex) : [];
    if (scenes.length) {
      const cat = await LP.eqCatalogue();
      const opts = scenes.slice(0, 2).map((id) => cat.scenes?.find((s) => s.id === id)).filter(Boolean);
      const pick = window.LoreShell?.ask ? await window.LoreShell.ask(anchor, ["動起來"], [...opts.map((s) => ({ label: s.label, value: s.id, icon: "play" })), { label: "曲線", value: "curve", primary: true }]) : "curve";
      if (pick == null) return;
      if (pick !== "curve") {
        const spec = await starter(pick);
        spec.panels[0].eq.latex = latex;
        await LP.edit({ spec, kind: "equation", title: title || "動起來", api, latex, notation });
        return;
      }
    }
    let spec = null;
    if (latex && propose) {
      const t = toast("動起來：讀式子…", { ms: 60000 });
      try {
        spec = (await LP.eqEditor.propose(api, { latex, symbols, context })).spec;
      } catch (e) {
        spec = e.draft ?? null;
        toast(`${e.message}${spec ? "（草稿，可手動修改）" : "（手動選式子）"}`, { err: true, ms: 8000 });
      }
      t?.remove?.();
      if (spec) spec.panels[0].eq.latex ||= latex;
    }
    await LP.edit({ spec, kind: "equation", title: title || "動起來", api, latex, notation });
  };
  LP.eqSim = { mount, mountCurve, chart, frames, assignRoles, starter, suggest, stats, headTex, themePal, docPal, VW, VH };
})();
