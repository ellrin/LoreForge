// LoreShelf: the 360° direction around the focus, from the server's factor shares.
// Until tags exist (docs/loreshelf.md §5), each neighbour's `parts` (how much of its similarity
// comes from name, time, kind, download host, folder) is a vector on fixed screen axes; the
// tag-vector PCA comes back when tag vectors do. θ: 0° = right, counter-clockwise.
//
//   Compass.AXES                 [{ key, deg, label }]  (time has two: 較新 → / 較舊 ←)
//   Compass.angleOf(parts, newer) → radians: argument of Σ share · axis
//   Compass.read(nodes, θ)       → [{ label, pct }] cosine-weighted mean of the parts of the
//                                  nodes within ±35° of θ ([] when none: nothing that way)
//   Compass.label(parts)         → "58% 名稱 · 27% 較新 · 15% 同資料夾"
"use strict";
(function (root) {
  const AXES = [
    { key: "time+", deg: 0, label: "較新", part: "較新" },
    { key: "name", deg: 90, label: "名稱相近", part: "名稱" },
    { key: "time-", deg: 180, label: "較舊", part: "較舊" },
    { key: "kind", deg: 225, label: "同類型", part: "類型" },
    { key: "folder", deg: 270, label: "同資料夾", part: "資料夾" },
    { key: "host", deg: 315, label: "同來源", part: "來源" },
    { key: "mention", deg: 135, label: "提及", part: "提及" },   // a paper that mentions a dataset
  ];
  const rad = (d) => (d * Math.PI) / 180;
  const axis = Object.fromEntries(AXES.map((a) => [a.key, [Math.cos(rad(a.deg)), Math.sin(rad(a.deg))]]));
  const keyOf = (k, newer) => (k === "time" ? (newer ? "time+" : "time-") : k);

  function angleOf(parts, newer) {
    let x = 0, y = 0;
    for (const [k, v] of Object.entries(parts)) { const a = axis[keyOf(k, newer)]; if (a) { x += v * a[0]; y += v * a[1]; } }
    return Math.atan2(y, x);
  }

  const WINDOW = Math.cos(rad(35));
  function read(nodes, th) {
    const acc = {}; let wsum = 0;
    for (const n of nodes) {
      const c = Math.cos(n.ang - th);
      if (c < WINDOW) continue;
      const w = c * c;
      wsum += w;
      for (const [k, v] of Object.entries(n.parts)) { const key = keyOf(k, n.newer); acc[key] = (acc[key] || 0) + w * v; }
    }
    if (!wsum) return [];
    let parts = Object.entries(acc).map(([k, v]) => ({ label: AXES.find((a) => a.key === k).part, raw: v / wsum })).filter((p) => p.raw >= 0.08);
    parts.sort((a, b) => b.raw - a.raw);
    parts = parts.slice(0, 3);
    const s = parts.reduce((a, p) => a + p.raw, 0) || 1;
    parts.forEach((p) => { p.exact = (100 * p.raw) / s; p.pct = Math.floor(p.exact); });
    let left = 100 - parts.reduce((a, p) => a + p.pct, 0);
    [...parts].sort((a, b) => (b.exact - b.pct) - (a.exact - a.pct)).forEach((p) => { if (left > 0) { p.pct++; left--; } });
    return parts.map(({ label, pct }) => ({ label, pct }));
  }
  const label = (parts) => parts.map((p) => `${p.pct}% ${p.label}`).join(" · ");

  const api = { AXES, angleOf, read, label };
  if (typeof module !== "undefined") module.exports = api; else root.Compass = api;
})(typeof window !== "undefined" ? window : globalThis);
