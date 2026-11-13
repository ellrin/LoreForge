// LorePlot 3-D data (only where the data is 3-D, docs/plot.md T1–T4):
//   contour   — filled contour + iso lines over a regular x/y grid (preferred over a surface)
//   surface   — a surface with a fixed camera stored in the spec (opts.azim, opts.elev)
//   scatter3d — a small 3-D point cloud with optional floor projections
// Medical volumes (T4) are three heatmap panels with opts.image (axial, coronal, sagittal).
(() => {
  "use strict";
  const LP = window.LorePlot;
  const { r2, isNum, num, col, ncol, cmap, polyline, markerPath, fmtNum, linearTicks } = LP._;
  const CHARTS = LP.CHARTS;

  /** Long table (x, y, z) → sorted unique xs, ys and Z[j][i]. */
  function grid(tbl, p) {
    const X = ncol(tbl, p.x), Y = ncol(tbl, p.y), Z = ncol(tbl, p.z ?? p.series?.[0]?.y);
    const xs = [...new Set(X.filter(isNum))].sort((a, b) => a - b), ys = [...new Set(Y.filter(isNum))].sort((a, b) => a - b);
    const ix = new Map(xs.map((v, i) => [v, i])), iy = new Map(ys.map((v, i) => [v, i]));
    const M = ys.map(() => xs.map(() => NaN));
    X.forEach((x, k) => { if (isNum(x) && isNum(Y[k]) && isNum(Z[k])) M[iy.get(Y[k])][ix.get(x)] = Z[k]; });
    const flat = M.flat().filter(isNum);
    return { xs, ys, M, zmin: Math.min(...flat), zmax: Math.max(...flat) };
  }

  CHARTS.contour = {
    label: "等高線（3D 資料）", group: "3d",
    prepare(p, tbl, st) {
      const g = grid(tbl, p);
      const o = p.opts ?? {};
      const name = o.cmap ?? st.colors.cmap ?? "viridis";
      const levels = linearTicks(g.zmin, g.zmax, o.levels ?? 8);
      const q = (v) => { let k = 0; while (k < levels.length && levels[k] <= v) k++; return (k - 0.5) / levels.length; };
      return {
        x: { kind: "linear", values: g.xs, fixed: [g.xs[0], g.xs.at(-1)] },
        y: { kind: "linear", values: g.ys, fixed: [g.ys[0], g.ys.at(-1)] },
        colorbar: { vmin: g.zmin, vmax: g.zmax, cmap: name, label: p.zaxis?.label ?? "" },
        legend: [],
        points: [],
        draw(c) {
          const out = [];
          const edge = (arr, i) => [i ? (arr[i - 1] + arr[i]) / 2 : arr[0], i < arr.length - 1 ? (arr[i] + arr[i + 1]) / 2 : arr.at(-1)];
          g.M.forEach((row, j) => row.forEach((v, i) => {
            if (!isNum(v)) return;
            const [x0, x1] = edge(g.xs, i), [y0, y1] = edge(g.ys, j);
            const t = o.filled === "smooth" ? (v - g.zmin) / (g.zmax - g.zmin || 1) : q(v);
            out.push(`<rect x="${r2(Math.min(c.sx(x0), c.sx(x1)))}" y="${r2(Math.min(c.sy(y0), c.sy(y1)))}" width="${r2(Math.abs(c.sx(x1) - c.sx(x0)) + 0.1)}" height="${r2(Math.abs(c.sy(y1) - c.sy(y0)) + 0.1)}" fill="${cmap(name, t)}"/>`);
          }));
          // iso lines (marching squares, one segment per cell crossing)
          if (o.lines !== false) {
            let d = "";
            for (const L of levels) {
              for (let j = 0; j < g.ys.length - 1; j++) for (let i = 0; i < g.xs.length - 1; i++) {
                const c4 = [[g.xs[i], g.ys[j], g.M[j][i]], [g.xs[i + 1], g.ys[j], g.M[j][i + 1]], [g.xs[i + 1], g.ys[j + 1], g.M[j + 1][i + 1]], [g.xs[i], g.ys[j + 1], g.M[j + 1][i]]];
                if (c4.some((q4) => !isNum(q4[2]))) continue;
                const hits = [];
                for (let e = 0; e < 4; e++) {
                  const [a, b] = [c4[e], c4[(e + 1) % 4]];
                  if ((a[2] - L) * (b[2] - L) < 0) { const t = (L - a[2]) / (b[2] - a[2]); hits.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]); }
                }
                for (let h = 0; h + 1 < hits.length; h += 2) d += `M${r2(c.sx(hits[h][0]))} ${r2(c.sy(hits[h][1]))}L${r2(c.sx(hits[h + 1][0]))} ${r2(c.sy(hits[h + 1][1]))}`;
              }
            }
            out.push(`<path d="${d}" stroke="${o.line_color ?? "#ffffff"}" stroke-opacity="0.75" stroke-width="${r2(st.axes.line * 0.8)}" fill="none"/>`);
          }
          c.out.push(`<g data-part="cells">${out.join("")}</g>`);
        },
      };
    },
  };

  /** Orthographic camera: azimuth / elevation in degrees. */
  function camera(azim, elev) {
    const a = (azim * Math.PI) / 180, e = (elev * Math.PI) / 180;
    return (x, y, z) => {
      const X = x * Math.cos(a) - y * Math.sin(a), Yd = x * Math.sin(a) + y * Math.cos(a);
      return { sx: X, sy: -(z * Math.cos(e) + Yd * Math.sin(e)), depth: Yd * Math.cos(e) - z * Math.sin(e) };
    };
  }
  function box3d(c, cam, st, labels, ranges) {
    const { cx, cy, R } = c.polar;
    const k = R / 1.75;
    const P = (x, y, z) => { const q = cam(x, y, z); return [cx + q.sx * k, cy + q.sy * k, q.depth]; };
    const corners = [-1, 1].flatMap((x) => [-1, 1].flatMap((y) => [-1, 1].map((z) => [x, y, z])));
    // the three back faces' edges (the ones behind the data)
    const edges = [];
    for (const a of corners) for (const b of corners) {
      const diff = a.filter((v, i) => v !== b[i]).length;
      if (diff === 1 && a.join() < b.join()) edges.push([a, b]);
    }
    const far = corners.reduce((m, q) => (P(...q)[2] > P(...m)[2] ? q : m), corners[0]);
    const back = edges.filter(([a, b]) => a.some((v, i) => v === far[i] && b[i] === far[i]));
    const g = [`<path d="${back.map(([a, b]) => { const [p1, p2] = [P(...a), P(...b)]; return `M${r2(p1[0])} ${r2(p1[1])}L${r2(p2[0])} ${r2(p2[1])}`; }).join("")}" stroke="#bdbdbd" stroke-width="${r2(st.axes.line * 0.8)}" fill="none"/>`];
    // axis names and their ranges at the front-bottom edges
    const f = c.font("tick");
    const near = [-far[0], -far[1], -1];
    const tick = (axis, v, at) => c.T(fmtNum(v, "", Math.abs(ranges[axis][1] - ranges[axis][0]) / 4), f).place(at[0], at[1], { anchor: "middle", valign: "middle" });
    const ends = [
      [[-1, near[1], -1], [1, near[1], -1], 0],
      [[near[0], -1, -1], [near[0], 1, -1], 1],
      [[near[0], far[1], -1], [near[0], far[1], 1], 2],
    ];
    ends.forEach(([a, b, axis]) => {
      const [p1, p2] = [P(...a), P(...b)];
      g.push(`<path d="M${r2(p1[0])} ${r2(p1[1])}L${r2(p2[0])} ${r2(p2[1])}" stroke="${st.colors.axis}" stroke-width="${r2(st.axes.line)}"/>`);
      const off = (q, s) => [q[0] + (q[0] - cx) * 0.08 * s, q[1] + (q[1] - cy) * 0.08 * s + (axis === 2 ? 0 : 4)];
      g.push(tick(axis, ranges[axis][0], off(p1, 1)), tick(axis, ranges[axis][1], off(p2, 1)));
      if (labels[axis]) { const m = off([(p1[0] + p2[0]) / 2, (p1[1] + p2[1]) / 2], 2.2); g.push(c.T(labels[axis], c.font("label")).place(m[0], m[1], { anchor: "middle", valign: "middle" })); }
    });
    return { P, svg: g.join("") };
  }
  const norm = (v, lo, hi) => (hi > lo ? ((v - lo) / (hi - lo)) * 2 - 1 : 0);

  CHARTS.surface = {
    label: "曲面（3D 資料）", group: "3d",
    polar: true,
    prepare(p, tbl, st) {
      const g = grid(tbl, p);
      const o = p.opts ?? {};
      const name = o.cmap ?? st.colors.cmap ?? "viridis";
      return {
        polar: true,
        cats: [],
        labelRoom: 10,
        legend: [],
        points: [],
        draw(c) {
          const cam = camera(o.azim ?? -55, o.elev ?? 28);
          const ranges = [[g.xs[0], g.xs.at(-1)], [g.ys[0], g.ys.at(-1)], [g.zmin, g.zmax]];
          const box = box3d(c, cam, st, [p.xaxis?.label, p.yaxis?.label, p.zaxis?.label], ranges);
          const quads = [];
          for (let j = 0; j < g.ys.length - 1; j++) for (let i = 0; i < g.xs.length - 1; i++) {
            const pts = [[i, j], [i + 1, j], [i + 1, j + 1], [i, j + 1]].map(([a, b]) => [norm(g.xs[a], ...ranges[0]), norm(g.ys[b], ...ranges[1]), norm(g.M[b][a], ...ranges[2]), g.M[b][a]]);
            if (pts.some((q) => !isNum(q[3]))) continue;
            const sp = pts.map((q) => box.P(q[0], q[1], q[2]));
            quads.push({ sp, depth: sp.reduce((a, q) => a + q[2], 0) / 4, z: pts.reduce((a, q) => a + q[3], 0) / 4 });
          }
          quads.sort((a, b) => b.depth - a.depth);
          const body = quads.map((q) => `<path d="${polyline(q.sp)}Z" fill="${cmap(name, (q.z - g.zmin) / (g.zmax - g.zmin || 1))}" stroke="#ffffff" stroke-opacity="0.35" stroke-width="0.2"/>`);
          c.out.push(`<g data-part="cells">${box.svg}${body.join("")}</g>`);
        },
      };
    },
  };

  CHARTS.scatter3d = {
    label: "3D 點雲", group: "3d",
    polar: true,
    prepare(p, tbl, st) {
      const X = ncol(tbl, p.x), Y = ncol(tbl, p.y), Z = ncol(tbl, p.z ?? p.series?.[0]?.y);
      const groups = p.group ? col(tbl, p.group).map((v) => String(v ?? "")) : X.map(() => "");
      const names = [...new Set(groups)];
      const pal = st.colors.palette;
      const colorOf = (g) => (p.series ?? []).find((q) => q.group === g)?.color ?? pal[names.indexOf(g) % pal.length];
      const ok = X.map((x, k) => isNum(x) && isNum(Y[k]) && isNum(Z[k]));
      const rng = (a) => { const v = a.filter((_, k) => ok[k]); return [Math.min(...v), Math.max(...v)]; };
      const ranges = [rng(X), rng(Y), rng(Z)];
      return {
        polar: true,
        cats: [],
        labelRoom: 10,
        legend: names.length > 1 ? names.map((g, i) => ({ label: (p.series ?? []).find((q) => q.group === g)?.label || g, kind: "marker", marker: "o", color: colorOf(g), part: `group:${i}` })) : [],
        legendPos: "outside right",
        points: [],
        draw(c) {
          const o = p.opts ?? {};
          const cam = camera(o.azim ?? -55, o.elev ?? 22);
          const box = box3d(c, cam, st, [p.xaxis?.label, p.yaxis?.label, p.zaxis?.label], ranges);
          const pts = X.map((x, k) => (ok[k] ? { k, n: [norm(x, ...ranges[0]), norm(Y[k], ...ranges[1]), norm(Z[k], ...ranges[2])] } : null)).filter(Boolean);
          const out = [box.svg];
          if (o.walls) for (const q of pts) { const s = box.P(q.n[0], q.n[1], -1); out.push(`<circle cx="${r2(s[0])}" cy="${r2(s[1])}" r="0.6" fill="#9a9a9a" fill-opacity="0.35"/>`); }
          const proj = pts.map((q) => ({ ...q, s: box.P(...q.n) })).sort((a, b) => b.s[2] - a.s[2]);
          for (const q of proj) out.push(markerPath("o", q.s[0], q.s[1], st.scatter.size, colorOf(groups[q.k]), { stroke: 0.25, edge: "#ffffff" }));
          c.out.push(`<g data-part="cells">${out.join("")}</g>`);
        },
      };
    },
  };
})();
