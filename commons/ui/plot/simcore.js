// LoreSim: the numeric core of simulation scenes (docs/living-equations.md tier 2, docs/plot.md §11), DOM-free.
// An equation figure whose `eq.sim` describes a scene is not a curve but a run: a model fitted to a target by
// minimising Σ wᵢ Lᵢ (fit1d, fit2d, classes) or a clustering algorithm stepping (cluster). Every step is computed
// here (Adam / gradient descent on central differences, numerical integration on a grid, EM, Lloyd); nothing is
// pre-recorded. lore-plot's Rust `equation::sim` validates the same spec and computes the same term values
// (ui/plot/simtests.json holds the cases both pass; tests/simcore.test.js under node).
//
//   const run = LoreSim.create(sim)       sim = eq.sim (the spec)
//   run.step(n)  run.reset()  run.setWeight(i, w)  run.toggle(i)  run.addTerm(t)  run.removeTerm(i)
//   run.setTarget(target)  run.values()  run.grads()  run.pulls()  run.view()  run.snapshot() / run.restore(s)
//   LoreSim.TERMS[scene]                  the term kinds a scene offers (label, LaTeX)
//   LoreSim.schedule(diffs, F, mix)       GIF pacing: frame → step index from visual change mixed with time
(function (root) {
  "use strict";
  const E = typeof module !== "undefined" && module.exports ? require("./eqcore.js") : root.LoreEq;
  const LN2PI = Math.log(2 * Math.PI);
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  const num = (v, d) => (isNum(+v) && v !== null && v !== "" ? +v : d);
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  function lse(a) {
    let m = -Infinity;
    for (const v of a) if (v > m) m = v;
    if (!Number.isFinite(m)) return m;
    let s = 0;
    for (const v of a) s += Math.exp(v - m);
    return m + Math.log(s);
  }
  const logN1 = (x, mu, s) => { const z = (x - mu) / s; return -0.5 * z * z - Math.log(s) - 0.5 * LN2PI; };
  /** A standard normal pair from the shared mulberry32 (= Rust `sim::normal`). */
  function normal(r) {
    const u1 = r.uniform(), u2 = r.uniform();
    const m = Math.sqrt(-2 * Math.log(u1));
    return [m * Math.cos(2 * Math.PI * u2), m * Math.sin(2 * Math.PI * u2)];
  }

  // ── scenes and their terms ──
  // tex: the term's LaTeX with p (target) and q (model) as \p{} / \q{} placeholders the view colours.
  const TERMS = {
    fit1d: {
      kl_pq: { label: "正向 KL", tex: "\\mathrm{KL}(\\P{p}\\,\\|\\,\\Q{q})" },
      ce: { label: "交叉熵", tex: "-\\textstyle\\int \\P{p}\\log \\Q{q}" },
      kl_qp: { label: "反向 KL", tex: "\\mathrm{KL}(\\Q{q}\\,\\|\\,\\P{p})" },
      nll: { label: "樣本 NLL", tex: "-\\tfrac1N\\textstyle\\sum_i \\log \\Q{q}(\\P{x_i})" },
      moment: { label: "動差", tex: "(\\Q{\\mu}-\\P{m})^2+(\\Q{\\sigma}-\\P{s})^2" },
      anchor: { label: "錨點", tex: "(\\Q{\\mu}-\\mu_0)^2" },
      entropy: { label: "負熵", tex: "-H(\\Q{q})" },
      mse: { label: "密度 MSE", tex: "\\textstyle\\int (\\Q{q}-\\P{p})^2" },
      expr: { label: "自訂", tex: "" },
    },
    classes: {
      ce: { label: "交叉熵", tex: "-\\log \\Q{q}_{\\P{y}}" },
      smooth: { label: "label smoothing", tex: "-\\textstyle\\sum_k \\P{t_k}\\log \\Q{q_k}" },
      l2: { label: "L2", tex: "\\tfrac12\\|z\\|^2" },
      entropy: { label: "負熵", tex: "-H(\\Q{q})" },
      expr: { label: "自訂", tex: "" },
    },
    fit2d: {
      nll: { label: "樣本 NLL", tex: "-\\tfrac1N\\textstyle\\sum_i \\log \\Q{q}(\\P{x_i})" },
      kl_pq: { label: "正向 KL", tex: "\\mathrm{KL}(\\P{p}\\,\\|\\,\\Q{q})" },
      kl_qp: { label: "反向 KL", tex: "\\mathrm{KL}(\\Q{q}\\,\\|\\,\\P{p})" },
    },
    cluster: {
      kmeans: { label: "k-means", tex: "\\tfrac1N\\textstyle\\sum_i \\|x_i-\\Q{c}_{z_i}\\|^2" },
      em: { label: "GMM（EM）", tex: "-\\tfrac1N\\textstyle\\sum_i \\log \\Q{q}(x_i)" },
      ce: { label: "softmax 交叉熵", tex: "-\\tfrac1N\\textstyle\\sum_i \\log \\Q{q}_{\\P{y_i}}(x_i)" },
      l2: { label: "L2", tex: "\\tfrac12\\|W\\|^2" },
      smooth: { label: "label smoothing", tex: "-\\tfrac1N\\textstyle\\sum_{i,k} \\P{t_{ik}}\\log \\Q{q_k}(x_i)" },
    },
  };
  /** Names a user term (`kind: "expr"`) may use, per scene. */
  const EXPR_NAMES = { fit1d: ["mu", "sigma"], classes: ["z", "q", "y", "K"] };
  const LIMITS = { terms: 8, comps: 6, samples: 2000, steps: 5000, K: 12, k: 8 };

  // ── targets ──
  /** 1D mixture: comps [{ w, mu, sigma }] (weights normalised). */
  function mix1(t) {
    const cs = (t?.mix ?? []).slice(0, LIMITS.comps).map((c) => ({ w: Math.max(0, num(c.w, 1)), mu: num(c.mu, 0), sigma: Math.max(1e-3, num(c.sigma, 1)) }));
    const W = cs.reduce((s, c) => s + c.w, 0) || 1;
    for (const c of cs) { c.w /= W; c.lw = Math.log(Math.max(c.w, 1e-300)); }
    const mean = cs.reduce((s, c) => s + c.w * c.mu, 0);
    const vr = cs.reduce((s, c) => s + c.w * (c.sigma * c.sigma + c.mu * c.mu), 0) - mean * mean;
    return {
      cs, mean, var: vr, std: Math.sqrt(Math.max(vr, 0)),
      logp: (x) => lse(cs.map((c) => c.lw + logN1(x, c.mu, c.sigma))),
      sample(r) {
        let u = r.uniform(), k = 0;
        while (k < cs.length - 1 && u > cs[k].w) { u -= cs[k].w; k++; }
        return cs[k].mu + cs[k].sigma * normal(r)[0];
      },
    };
  }
  /** 2D Gaussian (mean m, covariance [sxx, sxy, syy]) as its Cholesky factor. */
  function chol2(cov) {
    const a = Math.max(1e-6, num(cov?.[0], 1)), b = num(cov?.[1], 0), c = Math.max(1e-6, num(cov?.[2], 1));
    const l11 = Math.sqrt(a), l21 = b / l11, l22 = Math.sqrt(Math.max(1e-8, c - l21 * l21));
    return { l11, l21, l22 };
  }
  const logN2 = (x, y, m, L) => {
    const d0 = (x - m[0]) / L.l11, d1 = (y - m[1] - L.l21 * d0) / L.l22;
    return -0.5 * (d0 * d0 + d1 * d1) - Math.log(L.l11 * L.l22) - LN2PI;
  };
  function mix2(t) {
    const cs = (t?.mix ?? []).slice(0, LIMITS.comps).map((c) => ({ w: Math.max(0, num(c.w, 1)), mu: [num(c.mu?.[0], 0), num(c.mu?.[1], 0)], cov: c.cov ?? [1, 0, 1] }));
    const W = cs.reduce((s, c) => s + c.w, 0) || 1;
    for (const c of cs) { c.w /= W; c.lw = Math.log(Math.max(c.w, 1e-300)); c.L = chol2(c.cov); }
    return {
      cs,
      logp: (x, y) => lse(cs.map((c) => c.lw + logN2(x, y, c.mu, c.L))),
      sample(r) {
        let u = r.uniform(), k = 0;
        while (k < cs.length - 1 && u > cs[k].w) { u -= cs[k].w; k++; }
        const c = cs[k], [a, b] = normal(r);
        return { x: c.mu[0] + c.L.l11 * a, y: c.mu[1] + c.L.l21 * a + c.L.l22 * b, k };
      },
    };
  }

  // ── GIF pacing ──
  /** F frame → step indices: cumulative visual change (75 %) mixed with plain time (25 %), equal spacing on that clock.
   *  diffs[k] = how much the picture changed from step k−1 to k (diffs[0] ignored). Pure; the Rust twin is `sim::schedule`. */
  function schedule(diffs, F, mix = 0.25) {
    const n = diffs.length - 1;
    if (n <= 0) return Array.from({ length: F }, () => 0);
    const tot = diffs.slice(1).reduce((a, b) => a + Math.max(0, b), 0);
    const w = tot > 0 ? 1 - mix : 0;
    const cum = [0];
    let c = 0;
    for (let k = 1; k <= n; k++) { c += Math.max(0, diffs[k]); cum.push((tot > 0 ? (w * c) / tot : 0) + (1 - w) * (k / n)); }
    const out = [];
    let k = 0;
    for (let i = 0; i < F; i++) {
      const t = F === 1 ? 1 : i / (F - 1);
      while (k < n && cum[k] < t - 1e-12) k++;
      out.push(k);
    }
    return out;
  }
  /** Per-frame delays (ms): `ms` each, the last frame held `hold` ms. */
  const delays = (F, ms, hold) => Array.from({ length: F }, (_, i) => (i === F - 1 ? Math.max(ms, hold) : ms));

  // ── a run ──
  function create(spec) {
    const sim = JSON.parse(JSON.stringify(spec ?? {}));
    const scene = TERMS[sim.scene] ? sim.scene : "fit1d";
    sim.scene = scene;
    sim.terms = (sim.terms ?? []).slice(0, LIMITS.terms).map((t) => ({ ...t, w: num(t.w, 1), on: t.on !== false }));
    sim.opt = { method: sim.opt?.method ?? (scene === "classes" ? "gd" : "adam"), lr: num(sim.opt?.lr, scene === "classes" ? 0.5 : scene === "cluster" ? 0.3 : 0.045), steps: Math.min(LIMITS.steps, num(sim.opt?.steps, 300)) };
    const R = { sim, scene, theta: null, m: null, v: null, t: 0, step: 0, hist: [], marks: [], cache: null, ast: [] };

    // ── per scene: setup(), theta0(), terms(θ) → [values], view(θ) ──
    let S = {};
    function compileExprs() {
      R.ast = sim.terms.map((t) => {
        if (t.kind !== "expr") return null;
        try { return E.parse(String(t.expr ?? ""), EXPR_NAMES[scene] ?? []); } catch { return null; }
      });
    }
    function setup() {
      compileExprs();
      if (scene === "fit1d") {
        const [x0, x1] = [num(sim.x?.[0], -6), num(sim.x?.[1], 6)];
        const n = 400, dx = (x1 - x0) / (n - 1);
        const xs = Array.from({ length: n }, (_, i) => x0 + i * dx);
        const T = mix1(sim.target);
        const lp = xs.map((x) => T.logp(x)), p = lp.map(Math.exp);
        const r = E.rng(num(sim.samples?.seed, 7) >>> 0);
        const ns = Math.min(LIMITS.samples, num(sim.samples?.n, 0));
        const xsmp = Array.from({ length: ns }, () => T.sample(r));
        S = { xs, dx, T, lp, p, xsmp };
      } else if (scene === "classes") {
        const K = clamp(Math.round(num(sim.classes?.K, 6)), 2, LIMITS.K);
        const y = clamp(Math.round(num(sim.classes?.y, 1)), 1, K) - 1;
        S = { K, y };
      } else {
        const T = mix2(sim.target);
        const r = E.rng(num(sim.samples?.seed, 11) >>> 0);
        const ns = clamp(Math.round(num(sim.samples?.n, 300)), 1, LIMITS.samples);
        const pts = Array.from({ length: ns }, () => T.sample(r));
        const b = sim.box ?? [-4.4, 4, -3.3, 3];
        const box = [num(b[0], -4), num(b[1], 4), num(b[2], -3), num(b[3], 3)];
        // the grid KL terms integrate over a G×G grid of the box
        const G = 36, gx = Array.from({ length: G }, (_, i) => box[0] + ((i + 0.5) / G) * (box[1] - box[0])), gy = Array.from({ length: G }, (_, j) => box[2] + ((j + 0.5) / G) * (box[3] - box[2]));
        const dA = ((box[1] - box[0]) / G) * ((box[3] - box[2]) / G);
        let glp = null;
        const gridLp = () => (glp ??= gy.flatMap((y) => gx.map((x) => T.logp(x, y))));
        const k = clamp(Math.round(num(sim.model?.k ?? sim.cluster?.k, 3)), 1, LIMITS.k);
        S = { T, pts, box, G, gx, gy, dA, gridLp, k, method: sim.cluster?.method ?? "kmeans" };
      }
    }
    function theta0() {
      if (scene === "fit1d") return [num(sim.model?.mu, 0), Math.log(Math.max(1e-3, num(sim.model?.sigma, 1)))];
      if (scene === "classes") {
        const z = Array.isArray(sim.model?.z) ? sim.model.z.map((v) => num(v, 0)) : [];
        return Array.from({ length: S.K }, (_, i) => num(z[i], 0));
      }
      // 2D: k components spread around the box centre (seeded), or the given means
      const r = E.rng(num(sim.model?.seed, 3) >>> 0);
      const [x0, x1, y0, y1] = S.box, cx = (x0 + x1) / 2, cy = (y0 + y1) / 2, sx = (x1 - x0) * 0.12, sy = (y1 - y0) * 0.12;
      const init = sim.model?.mu ?? [];
      if (scene === "cluster" && S.method === "ce") {
        // softmax partition: logits W·[x, y, 1] per class, small seeded start
        return Array.from({ length: S.k * 3 }, () => (r.uniform() - 0.5) * 0.2);
      }
      const th = [];
      for (let j = 0; j < S.k; j++) {
        const m = init[j] ?? [cx + (r.uniform() - 0.5) * 2 * sx, cy + (r.uniform() - 0.5) * 2 * sy];
        const s0 = Math.log(num(sim.model?.sigma, 0.7));
        th.push(num(m[0], cx), num(m[1], cy), s0, 0, s0, 0);
      }
      return th;
    }

    // 1D model q = N(μ, σ)
    const q1 = (th) => { const mu = th[0], s = Math.exp(th[1]); return { mu, s, lq: S.xs.map((x) => logN1(x, mu, s)) }; };
    const softmax = (z) => { const m = Math.max(...z), e = z.map((v) => Math.exp(v - m)), s = e.reduce((a, b) => a + b, 0); return e.map((v) => v / s); };
    // 2D mixture model from θ (6 per component)
    function comps(th) {
      const k = S.k, a = [];
      for (let j = 0; j < k; j++) a.push(th[j * 6 + 5]);
      const la = lse(a);
      return Array.from({ length: k }, (_, j) => {
        const o = j * 6;
        return { mu: [th[o], th[o + 1]], L: { l11: Math.exp(th[o + 2]), l21: th[o + 3], l22: Math.exp(th[o + 4]) }, lw: th[o + 5] - la, w: Math.exp(th[o + 5] - la) };
      });
    }
    const logq2 = (cs, x, y) => lse(cs.map((c) => c.lw + logN2(x, y, c.mu, c.L)));

    /** One term's value at θ (null: no value). */
    function termValue(t, i, th, ctx) {
      if (scene === "fit1d") {
        const Q = ctx.q ??= q1(th), { p, lp, dx } = S;
        switch (t.kind) {
          case "kl_pq": { let s = 0; for (let j = 0; j < p.length; j++) if (p[j] > 1e-300) s += p[j] * (lp[j] - Q.lq[j]); return s * dx; }
          case "ce": { let s = 0; for (let j = 0; j < p.length; j++) s -= p[j] * Q.lq[j]; return s * dx; }
          case "kl_qp": { let s = 0; for (let j = 0; j < p.length; j++) { const qj = Math.exp(Q.lq[j]); if (qj > 1e-300) s += qj * (Q.lq[j] - lp[j]); } return s * dx; }
          case "nll": { const xs = S.xsmp; if (!xs.length) return null; let s = 0; for (const x of xs) s -= logN1(x, Q.mu, Q.s); return s / xs.length; }
          case "moment": return (Q.mu - S.T.mean) ** 2 + (Q.s - S.T.std) ** 2;
          case "anchor": return (Q.mu - num(t.mu0, 0)) ** 2;
          case "entropy": return -(Math.log(Q.s) + 0.5 * (1 + LN2PI));
          case "mse": { let s = 0; for (let j = 0; j < p.length; j++) { const d = Math.exp(Q.lq[j]) - p[j]; s += d * d; } return s * dx; }
          case "expr": { const a = R.ast[i]; if (!a) return null; const v = E.evaluate(a, { mu: Q.mu, sigma: Q.s }); return isNum(v) ? v : null; }
        }
        return null;
      }
      if (scene === "classes") {
        const q = ctx.qv ??= softmax(th), K = S.K, y = S.y;
        const lq = q.map((v) => Math.log(Math.max(v, 1e-300)));
        switch (t.kind) {
          case "ce": return -lq[y];
          case "smooth": { const e = clamp(num(t.eps, 0.1), 0, 1); let s = 0; for (let k = 0; k < K; k++) s -= ((k === y ? 1 - e : 0) + e / K) * lq[k]; return s; }
          case "l2": return 0.5 * th.reduce((s, z) => s + z * z, 0);
          case "entropy": return q.reduce((s, v, k) => s + (v > 0 ? v * lq[k] : 0), 0);
          case "expr": { const a = R.ast[i]; if (!a) return null; const v = E.evaluate(a, { z: th.slice(), q: q.slice(), y: y + 1, K }); return isNum(v) ? v : null; }
        }
        return null;
      }
      if (scene === "fit2d") {
        const cs = ctx.cs ??= comps(th);
        switch (t.kind) {
          case "nll": { let s = 0; for (const p of S.pts) s -= logq2(cs, p.x, p.y); return s / S.pts.length; }
          case "kl_pq": case "kl_qp": {
            const glp = S.gridLp(), lq = ctx.glq ??= S.gy.flatMap((y) => S.gx.map((x) => logq2(cs, x, y)));
            let s = 0;
            for (let j = 0; j < glp.length; j++) {
              if (t.kind === "kl_pq") s += Math.exp(glp[j]) * (glp[j] - lq[j]);
              else s += Math.exp(lq[j]) * (lq[j] - glp[j]);
            }
            return s * S.dA;
          }
        }
        return null;
      }
      // cluster, softmax partition (ce, l2, smooth over W)
      if (S.method === "ce") {
        const K = S.k, P = S.pts;
        const lq = ctx.lq ??= P.map((p) => { const z = Array.from({ length: K }, (_, c) => th[c * 3] * p.x + th[c * 3 + 1] * p.y + th[c * 3 + 2]); const l = lse(z); return z.map((v) => v - l); });
        switch (t.kind) {
          case "ce": return -lq.reduce((s, l, i) => s + l[P[i].k % K], 0) / P.length;
          case "smooth": { const e = clamp(num(t.eps, 0.1), 0, 1); return -lq.reduce((s, l, i) => s + l.reduce((a, v, c) => a + ((c === P[i].k % K ? 1 - e : 0) + e / K) * v, 0), 0) / P.length; }
          case "l2": return 0.5 * th.reduce((s, w, j) => s + (j % 3 === 2 ? 0 : w * w), 0);
        }
        return null;
      }
      return clusterObjective(th);
    }
    function values(th = R.theta) {
      const ctx = {};
      return sim.terms.map((t, i) => { try { const v = termValue(t, i, th, ctx); return isNum(v) ? v : null; } catch { return null; } });
    }
    /** Each term's gradient (central differences; the 2D KL grid included). */
    function grads(th = R.theta) {
      // the view's pulls and the next step need the same gradients at the same θ: computed once
      const key = `${th.join(",")}|${JSON.stringify(sim.terms)}|${JSON.stringify(sim.target ?? null)}|${sim.classes?.y}|${JSON.stringify(sim.samples ?? null)}`;
      if (R.gkey === key) return R.gval;
      R.gkey = key;
      R.gval = gradsAt(th);
      return R.gval;
    }
    function gradsAt(th) {
      const h = 1e-5;
      const out = sim.terms.map(() => th.map(() => 0));
      if (scene === "cluster" && S.method !== "ce") return out;
      for (let j = 0; j < th.length; j++) {
        const a = th.slice(), b = th.slice();
        a[j] += h; b[j] -= h;
        const va = values(a), vb = values(b);
        for (let i = 0; i < out.length; i++) out[i][j] = isNum(va[i]) && isNum(vb[i]) ? (va[i] - vb[i]) / (2 * h) : 0;
      }
      return out;
    }

    // ── clustering (Lloyd / EM): θ = per component [mx, my, log l11, l21, log l22, log w] ──
    function resp(th) {
      const cs = comps(th);
      if (S.method === "kmeans") return S.pts.map((p) => { let b = 0, bd = Infinity; cs.forEach((c, j) => { const d = (p.x - c.mu[0]) ** 2 + (p.y - c.mu[1]) ** 2; if (d < bd) { bd = d; b = j; } }); return cs.map((_, j) => (j === b ? 1 : 0)); });
      return S.pts.map((p) => { const l = cs.map((c) => c.lw + logN2(p.x, p.y, c.mu, c.L)), m = lse(l); return l.map((v) => Math.exp(v - m)); });
    }
    function clusterObjective(th) {
      const cs = comps(th);
      if (S.method === "kmeans") return S.pts.reduce((s, p) => s + Math.min(...cs.map((c) => (p.x - c.mu[0]) ** 2 + (p.y - c.mu[1]) ** 2)), 0) / S.pts.length;
      return -S.pts.reduce((s, p) => s + logq2(cs, p.x, p.y), 0) / S.pts.length;
    }
    function clusterStep() {
      const r = resp(R.theta), th = R.theta.slice();
      for (let j = 0; j < S.k; j++) {
        let n = 0, mx = 0, my = 0;
        r.forEach((rr, i) => { n += rr[j]; mx += rr[j] * S.pts[i].x; my += rr[j] * S.pts[i].y; });
        if (n < 1e-9) continue;
        mx /= n; my /= n;
        const o = j * 6;
        th[o] = mx; th[o + 1] = my;
        if (S.method === "em") {
          let a = 0, b = 0, c = 0;
          r.forEach((rr, i) => { const dx = S.pts[i].x - mx, dy = S.pts[i].y - my; a += rr[j] * dx * dx; b += rr[j] * dx * dy; c += rr[j] * dy * dy; });
          const L = chol2([a / n + 1e-4, b / n, c / n + 1e-4]);
          th[o + 2] = Math.log(L.l11); th[o + 3] = L.l21; th[o + 4] = Math.log(L.l22); th[o + 5] = Math.log(n / S.pts.length);
        }
      }
      R.theta = th;
    }

    const effW = () => sim.terms.map((t) => (t.on ? Math.max(0, t.w) : 0));
    /** A changed objective restarts the optimiser's moments (θ is kept): Adam's long memory of the old gradients would stall it. */
    const fresh = (mark = true) => { R.m = R.theta.map(() => 0); R.v = R.theta.map(() => 0); R.t = 0; if (mark) addMark(); };
    const addMark = () => { if (R.marks.at(-1) !== R.step) R.marks.push(R.step); };
    function record() {
      const L = values(), w = effW();
      R.hist.push({ L, w });
      if (R.hist.length > 20000) R.hist.shift();
    }
    function applyPlan() {
      if (!R.scripted) return;
      for (const p of sim.plan ?? []) {
        if (num(p.at, -1) !== R.step || !Array.isArray(p.w)) continue;
        p.w.forEach((w, i) => { if (sim.terms[i] && isNum(+w)) { sim.terms[i].w = +w; sim.terms[i].on = +w > 0 || sim.terms[i].on; } });
        // `reset`: the same start again under the new weights (two objectives compared in one run)
        if (p.reset) R.theta = theta0();
        fresh();
      }
    }
    function stepOnce() {
      applyPlan();
      if (scene === "cluster" && S.method !== "ce") clusterStep();
      else {
        const G = grads(), w = effW(), th = R.theta;
        const g = th.map((_, j) => G.reduce((s, gi, i) => s + w[i] * gi[j], 0));
        const lr = sim.opt.lr;
        if (sim.opt.method === "gd") for (let j = 0; j < th.length; j++) th[j] -= lr * g[j];
        else {
          R.t++;
          for (let j = 0; j < th.length; j++) {
            R.m[j] = 0.9 * R.m[j] + 0.1 * g[j];
            R.v[j] = 0.999 * R.v[j] + 0.001 * g[j] * g[j];
            th[j] -= (lr * (R.m[j] / (1 - 0.9 ** R.t))) / (Math.sqrt(R.v[j] / (1 - 0.999 ** R.t)) + 1e-8);
          }
        }
        if (scene === "fit1d") th[1] = clamp(th[1], Math.log(0.02), Math.log(50));
      }
      R.step++;
      record();
    }

    const api = {
      get sim() { return sim; },
      get scene() { return scene; },
      get theta() { return R.theta; },
      get stepNo() { return R.step; },
      get hist() { return R.hist; },
      get marks() { return R.marks; },
      /** GIF mode: `plan` changes the weights at its steps. */
      scripted(on = true) { R.scripted = on; return api; },
      reset() {
        setup();
        R.theta = theta0();
        R.m = R.theta.map(() => 0);
        R.v = R.theta.map(() => 0);
        R.t = 0; R.step = 0; R.hist = []; R.marks = [];
        if (R.scripted) for (const p of sim.plan ?? []) if (num(p.at, -1) === 0 && Array.isArray(p.w)) p.w.forEach((w, i) => { if (sim.terms[i]) sim.terms[i].w = +w; });
        record();
        return api;
      },
      step(n = 1) { for (let i = 0; i < n; i++) stepOnce(); return api; },
      values: (th) => values(th ?? R.theta),
      grads: (th) => grads(th ?? R.theta),
      weights: effW,
      /** Weighted total at θ. */
      total(th) { const v = values(th ?? R.theta), w = effW(); return v.reduce((s, x, i) => s + (isNum(x) ? w[i] * x : 0), 0); },
      /** Each term's pull −wᵢ ∂Lᵢ/∂θ on the main parameters (fit1d: [μ, log σ]; classes: z; 2D: each mean). */
      pulls() {
        const w = effW();
        if (scene === "cluster" && S.method !== "ce") {
          // the next step's move of each centre (one "term": the algorithm)
          const keep = R.theta.slice();
          clusterStep();
          const d = R.theta.map((v, j) => v - keep[j]);
          R.theta = keep;
          return sim.terms.map(() => d);
        }
        return grads().map((g, i) => g.map((x) => -w[i] * x));
      },
      /** mark = false while a slider is being dragged (one mark per drag: `mark()` when it is let go). */
      setWeight(i, w, mark = true) { if (!sim.terms[i]) return; sim.terms[i].w = Math.max(0, +w || 0); fresh(mark); },
      mark: () => addMark(),
      toggle(i, on) { if (!sim.terms[i]) return; sim.terms[i].on = on ?? !sim.terms[i].on; fresh(); },
      addTerm(t) { if (sim.terms.length >= LIMITS.terms) return false; sim.terms.push({ w: 1, on: true, ...t }); compileExprs(); for (const h of R.hist) h.L.push(null), h.w.push(0); fresh(); return true; },
      removeTerm(i) {
        sim.terms.splice(i, 1);
        // the scripted weights follow the terms (a plan never names more terms than there are)
        for (const p of sim.plan ?? []) if (Array.isArray(p.w)) p.w.splice(i, 1);
        compileExprs(); for (const h of R.hist) h.L.splice(i, 1), h.w.splice(i, 1); fresh();
      },
      /** A new target (dragged): the run continues from the current θ. */
      setTarget(target) {
        sim.target = target;
        const th = R.theta;
        setup();
        R.theta = th;
        fresh();
      },
      /** What the view draws. */
      view() {
        const th = R.theta;
        if (scene === "fit1d") { const Q = q1(th); return { xs: S.xs, p: S.p, q: Q.lq.map(Math.exp), mu: Q.mu, sigma: Q.s, samples: S.xsmp, target: S.T }; }
        if (scene === "classes") return { K: S.K, y: S.y, z: th.slice(), q: softmax(th), t: api.targetBars() };
        if (scene === "cluster" && S.method === "ce") {
          return { pts: S.pts, box: S.box, k: S.k, W: th.slice(), classify: (x, y) => softmax(Array.from({ length: S.k }, (_, c) => th[c * 3] * x + th[c * 3 + 1] * y + th[c * 3 + 2])) };
        }
        const cs = comps(th);
        return { pts: S.pts, box: S.box, comps: cs, target: S.T, resp: scene === "cluster" ? resp(th) : null, logq: (x, y) => logq2(cs, x, y), k: S.k };
      },
      /** classes: the target bars (one-hot, or the smoothing term's when it is the one in force). */
      targetBars() {
        if (scene !== "classes") return null;
        const sm = sim.terms.find((t) => t.kind === "smooth" && t.on && t.w > 0);
        const e = sm ? clamp(num(sm.eps, 0.1), 0, 1) : 0;
        return Array.from({ length: S.K }, (_, k) => (k === S.y ? 1 - e : 0) + e / S.K);
      },
      snapshot() { return { theta: R.theta.slice(), m: R.m.slice(), v: R.v.slice(), t: R.t, step: R.step, hist: R.hist.map((h) => ({ L: h.L.slice(), w: h.w.slice() })), marks: R.marks.slice(), terms: JSON.parse(JSON.stringify(sim.terms)), target: JSON.parse(JSON.stringify(sim.target ?? null)) }; },
      restore(s) {
        if (!s) return api;
        if (s.terms) { sim.terms = s.terms; compileExprs(); }
        if (s.target !== undefined && JSON.stringify(s.target) !== JSON.stringify(sim.target)) { sim.target = s.target; setup(); }
        R.theta = s.theta.slice(); R.m = (s.m ?? R.theta.map(() => 0)).slice(); R.v = (s.v ?? R.theta.map(() => 0)).slice();
        R.t = s.t ?? 0; R.step = s.step ?? 0; R.hist = s.hist ?? []; R.marks = s.marks ?? [];
        return api;
      },
      /** Audience side: θ and the newest history row from the presenter, nothing computed but the view. */
      follow(m) {
        if (Array.isArray(m.theta)) R.theta = m.theta.slice();
        if (m.terms) { sim.terms = m.terms; compileExprs(); }
        // every history row since the last message (2×, 4× send several per frame)
        if (Array.isArray(m.rows) && isNum(m.from)) { R.hist.length = Math.min(R.hist.length, m.from); R.hist.push(...m.rows); }
        if (isNum(m.step)) R.step = m.step;
        if (m.marks) R.marks = m.marks.slice();
        return api;
      },
    };
    api.reset();
    return api;
  }

  /** The problems that make a sim unusable (the JS mirror of Rust `sim::problems`; Rust is the authority). */
  function problems(sim) {
    const out = [];
    if (!sim || !TERMS[sim.scene]) return ["不認得的場景"];
    if (!Array.isArray(sim.terms) || !sim.terms.length) out.push("沒有任何一項");
    for (const t of sim.terms ?? []) {
      if (!TERMS[sim.scene][t.kind]) out.push(`這個場景沒有「${t.kind}」`);
      if (!(num(t.w, NaN) >= 0)) out.push(`${t.kind} 的權重要 ≥ 0`);
      if (t.kind === "expr") { try { E.parse(String(t.expr ?? ""), EXPR_NAMES[sim.scene] ?? []); } catch (e) { out.push(`自訂項：${e.message}`); } }
    }
    return out;
  }

  const LoreSim = { create, problems, schedule, delays, TERMS, EXPR_NAMES, LIMITS, mix1, mix2, normal, lse };
  if (typeof module !== "undefined" && module.exports) module.exports = LoreSim;
  else root.LoreSim = LoreSim;
})(typeof window !== "undefined" ? window : globalThis);
