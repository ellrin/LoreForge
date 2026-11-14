// LoreShelf page: the focus and its neighbours, the direction readout, areas when zoomed out,
// the info card, settings and the resource tiers. Needs /theme.js, /icons.js, /shell.js,
// demo-data.js (fallback only), compass.js, scene.js.
//
// API (shelf-server): GET /api/files · GET /api/files/{id} · GET /api/neighbors/{id}?k=&layers=
// · POST /api/open/{id} · POST /api/reveal/{id}. Without /api/files → ShelfDemo (synthetic).
//
// window.LoreShelf = { setActivity("active" | "blurred" | "hidden"), stats(), open(), close(), hop(deg), zoom(z) }
"use strict";
(() => {
  // the desktop app calls setActivity on every window state change, possibly before the data is in
  const tauri = window.__TAURI__ ?? null;
  window.LoreShelf = { setActivity: (st) => setActivity(st) };
  if (tauri) document.documentElement.classList.add("desk");   // room for the window buttons
  const $ = (id) => document.getElementById(id);
  const q = new URLSearchParams(location.search);
  const h = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) { if (v == null || v === false) continue; if (k === "class") e.className = v; else if (k.startsWith("on")) e.addEventListener(k.slice(2), v); else e.setAttribute(k, v === true ? "" : v); }
    for (const c of kids.flat()) if (c != null) e.append(c);
    return e;
  };

  // ── config ──
  const CFG = {
    freeAfterMs: (+q.get("t3") || 30) * 1000,      // hidden this long → ③ free pictures and caches
    releaseAfterMs: (+q.get("t4") || 300) * 1000,  // hidden this long → ④ drop the whole scene
    demo: q.has("demo"),
  };
  const PREF_KEY = "lore.shelf.prefs";
  const DEFAULTS = { k: 12, layer2: true, fps: 60, echo: false };
  const ECHO_W = 540;   // LoreEcho's window when it is seated next to the shelf (shelf-server ECHO_W)
  const prefs = { ...DEFAULTS };
  try { Object.assign(prefs, JSON.parse(localStorage.getItem(PREF_KEY) || "{}")); } catch {}
  for (const k of Object.keys(prefs)) if (!(k in DEFAULTS)) delete prefs[k];
  const savePrefs = () => { try { localStorage.setItem(PREF_KEY, JSON.stringify(prefs)); } catch {} };

  const KIND_LABEL = { pdf: "PDF", latex: "LaTeX", document: "文件", text: "文字", data: "資料", spreadsheet: "試算表", slides: "簡報", image: "圖片", video: "影片", audio: "音訊", archive: "壓縮檔", installer: "安裝檔", code: "程式碼", ebook: "電子書", font: "字型", project: "專案", package: "套件", dataset: "資料集", projects: "專案", packages: "套件", datasets: "資料集", other: "其他" };
  const TIME_LABEL = [["modified", "修改"], ["created", "建立"], ["downloaded", "下載"], ["opened", "上次開啟"], ["doc_created", "文件日期"]];

  // ── data ──
  const S = {
    demo: false, items: new Map(), list: [], groups: new Map(), detail: new Map(), nbrs: new Map(),
    thumbs: new Map(), noThumb: new Set(), thumbBytes: 0,
    focus: null, zoom: 1, zoomT: 1,
    nodes: new Map(),      // id → entry { role, el, cv, d, v (where it is on the sphere), t (where it goes), k, kT (size), a, aT, ang, parts, newer, via, w }
    ghosts: [], off: [0, 0], swipe: null, candidate: null, pending: null,
    tier: 1, open: true, timers: {}, restoreT0: 0, restoreMs: null, frames: 0,
    cat: { kind: new Set(), use: new Set() }, cats: null, catApi: true, ds: null,
    vel: [0, 0], coast: false, wheeling: false, moving: false, hover: null, far: [],
  };
  async function api(method, path, body) {
    if (S.demo) return ShelfDemo.api(method, path, body);
    const r = await fetch(path, { method, cache: "no-store", headers: body ? { "content-type": "application/json" } : {}, body: body ? JSON.stringify(body) : undefined });
    const d = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(d.error || `${r.status}`);
    return d;
  }
  const ms = (s) => (s ? Date.parse(s) : NaN);
  // the one place the server's field names are read
  function normalize(it) {
    const t = it.times ?? {};
    const time = [t.modified, t.created, t.downloaded, t.opened].map(ms).find((x) => !Number.isNaN(x)) ?? 0;
    return { id: it.id, name: it.name ?? "", kind: it.kind ?? "", ext: it.ext ?? "", time, host: it.download_host ?? null, times: t, group: it.group ?? null,
      cats: it.categories ?? [TYPE_OF[it.kind]].filter(Boolean), labels: it.labels ?? [], sensitive: !!it.sensitive, count: null, mainExt: null, size: it.size ?? 0 };
  }

  // ── filters: 類型 and 用途 (OR within a group, AND across); `cat=pdf,paper` (lore_catalog::category ids) ──
  const TYPE_OF = { pdf: "pdf", document: "document", text: "document", ebook: "document", latex: "document", slides: "slides", spreadsheet: "spreadsheet", image: "image", video: "video", archive: "archive", code: "code", project: "project", dataset: "dataset", package: "package" };
  const USE_LABEL = { paper: "論文", "id-doc": "證件", homework: "作業", ticket: "票券收據" };
  const isUse = (c) => c in USE_LABEL || c.startsWith("u:");   // u:<name> = a category the author made up
  const useLabel = (c) => USE_LABEL[c] ?? c.replace(/^u:/, "");
  const usesOf = (it) => it.cats.filter(isUse);
  const catParam = () => [...S.cat.kind, ...S.cat.use].join(",");
  const filtering = () => S.cat.kind.size + S.cat.use.size > 0;
  const matches = (it) => !!it && (!S.cat.kind.size || it.cats.some((c) => S.cat.kind.has(c))) && (!S.cat.use.size || it.cats.some((c) => S.cat.use.has(c)));
  // copies of one document ("X.pdf", "X (1).pdf", "X.docx"): the server's `group` when it sends one,
  // else the name without extension, copy numbers and "copy"/"副本"
  // "X (1).pdf" → "X": the name a stack of copies goes by
  const stemOf = (name) => name.replace(/\.[^.]{1,8}$/, "").replace(/\s*[(（]\d+[)）]$/, "").replace(/[\s_-]*(copy|副本)(\s*\d+)?$/i, "").trim() || name;
  function groupKey(it) {
    if (it.group) return `g:${it.group}`;
    const st = it.name.replace(/\.[^.]{1,8}$/, "").toLowerCase().replace(/\s*[(（]\d+[)）]$/, "").replace(/[\s_-]*(copy|副本)(\s*\d+)?$/, "").trim();
    return st.length >= 3 ? `n:${st}` : `id:${it.id}`;
  }
  const membersOf = (id) => S.groups.get(S.items.get(id)?.gkey) ?? [id];
  // one entry per group; the focus's own copies are not neighbours
  function dedupe(list) {
    const seen = new Set([S.items.get(S.focus)?.gkey]), out = [];
    for (const n of list) {
      const it = S.items.get(n.id); if (!it) continue;
      if (seen.has(it.gkey)) continue;
      seen.add(it.gkey);
      out.push({ ...n, members: n.members ?? membersOf(n.id) });
    }
    return out;
  }
  async function neighbours(id) {
    const cat = catParam();
    const key = `${id}|${prefs.k}|${prefs.layer2 ? 2 : 1}|${cat}`;
    if (S.nbrs.has(key)) return S.nbrs.get(key);
    // a server that ignores `cat` gets asked for more and is filtered here
    const k = filtering() && !S.catApi ? prefs.k * 4 : prefs.k;
    const p = api("GET", `/api/neighbors/${encodeURIComponent(id)}?k=${k}&layers=${prefs.layer2 ? 2 : 1}${cat ? `&cat=${encodeURIComponent(cat)}` : ""}`)
      .then((v) => (Array.isArray(v) ? v : v.neighbors ?? []))
      .then((v) => {
        if (!filtering()) return v;
        // keep the k closest that match, and the second layer hanging from them
        const l1 = v.filter((n) => n.layer === 1 && matches(S.items.get(n.id))).slice(0, prefs.k), keep = new Set(l1.map((n) => n.id));
        return [...l1, ...v.filter((n) => n.layer > 1 && keep.has(n.via) && matches(S.items.get(n.id)))];
      });
    S.nbrs.set(key, p);
    p.catch(() => S.nbrs.delete(key));
    return p;
  }
  async function detail(id) {
    if (!S.detail.has(id)) { const p = api("GET", `/api/files/${encodeURIComponent(id)}`); S.detail.set(id, p); p.catch(() => S.detail.delete(id)); }
    return S.detail.get(id);
  }

  // ── geometry: a globe. The focus sits at the pole facing the viewer; its neighbours lie on the
  // front of the sphere around it (direction = compass angle, distance from the pole = how unlike);
  // dragging turns the globe. View space: x right, y up, z towards the viewer; unit vectors.
  const CAM = 3.2;   // camera distance in sphere radii
  const LIMB = (Math.sqrt(1 - 1 / (CAM * CAM)) * (CAM - 1)) / (CAM - 1 / CAM);   // silhouette radius / sphere radius
  const clamp = (x, a, b) => Math.max(a, Math.min(b, x));
  const norm = (v) => { const l = Math.hypot(v[0], v[1], v[2]) || 1; return [v[0] / l, v[1] / l, v[2] / l]; };
  const mulV = (m, v) => [m[0] * v[0] + m[1] * v[1] + m[2] * v[2], m[3] * v[0] + m[4] * v[1] + m[5] * v[2], m[6] * v[0] + m[7] * v[1] + m[8] * v[2]];
  const mulM = (a, b) => { const o = new Array(9); for (let r = 0; r < 3; r++) for (let c = 0; c < 3; c++) o[r * 3 + c] = a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c]; return o; };
  const transpose = (m) => [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];
  // a drag of (ax, ay) radians: the point at the pole follows the finger (Ry · Rx)
  const rotM = (ax, ay) => { const cx = Math.cos(ax), sx = Math.sin(ax), cy = Math.cos(ay), sy = Math.sin(ay); return [cx, sx * sy, sx * cy, 0, cy, -sy, -sx, cx * sy, cx * cy]; };
  const rotAxis = (a, t) => { const c = Math.cos(t), s = Math.sin(t), k = 1 - c, [x, y, z] = a; return [c + x * x * k, x * y * k - z * s, x * z * k + y * s, y * x * k + z * s, c + y * y * k, y * z * k - x * s, z * x * k - y * s, z * y * k + x * s, c + z * z * k]; };
  S.G = mulM(rotAxis([1, 0, 0], -0.42), rotAxis([0, 0, 1], 0.24));   // the wire globe's orientation (object → view)
  const turn = (L) => rotM(S.off[0] / L.R, S.off[1] / L.R);

  // zoom 0–1: from the areas to the globe; 1–2: the globe enlarged by hand (up to MAG_MAX times)
  const MAG_MAX = 2.1, magOf = (z) => 1 + (MAG_MAX - 1) * clamp(z - 1, 0, 1);
  function layout() {
    const w = innerWidth, hh = innerHeight, narrow = w <= 760, avail = hh - 48;
    // the globe sits in the middle of the window — with LoreEcho seated at the right, in the middle of
    // what is left between the filters and it; it shrinks before it would meet either
    const left = narrow ? 0 : $("filters").getBoundingClientRect().right;
    const echo = prefs.echo && !narrow ? ECHO_W - 16 : 0;   // its figure's own margin may overlap the callouts' edge
    const cx = echo ? (left + w - echo) / 2 : w / 2;
    const rv0 = narrow ? Math.min(w * 0.44, avail * 0.3) : Math.max(150, Math.min(avail * 0.42, cx - left - (echo ? 110 : 120)));
    const sc = (0.72 + 0.28 * Math.min(S.zoom, 1)) * magOf(S.zoom), rv = rv0 * sc;
    return {
      w, h: hh, narrow, sc, rv0, rv, R: rv / LIMB,
      cx, cy: 48 + avail * (narrow ? 0.38 : 0.47),
      F: Math.round(clamp(rv0 * 0.34, 76, 116)), B: Math.round(clamp(rv0 * 0.17, 40, 56)), D: Math.round(clamp(rv0 * 0.1, 24, 32)),
      preview: Math.max(200, Math.min(380, avail / 2.6)),
    };
  }
  const proj = (v, L) => { const s = (CAM - 1) / (CAM - v[2]); return [L.cx + L.R * v[0] * s, L.cy - L.R * v[1] * s, s]; };
  // a screen offset from the centre (px, y down, zoom 1) → the point on the front of the sphere shown there
  function toVec(x, y, R0) {
    const r = Math.min(Math.hypot(x, y) / R0, LIMB * 0.999), th = Math.atan2(-y, x);
    let lo = 0, hi = Math.acos(1 / CAM);
    for (let i = 0; i < 24; i++) { const m = (lo + hi) / 2; if ((Math.sin(m) * (CAM - 1)) / (CAM - Math.cos(m)) < r) lo = m; else hi = m; }
    const p = (lo + hi) / 2, sp = Math.sin(p);
    return [sp * Math.cos(th), sp * Math.sin(th), Math.cos(p)];
  }

  // ── regions: why these files are the neighbours. A region = the neighbours that share one label
  // with the focus (at most 4: the labels that cover the most of them); each has a colour that
  // stays with its label, a part of the sphere of its own, and its label written on the sphere.
  const lkey = (l) => l.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "");
  function regionsOf(l1) {
    const by = new Map();
    for (const p of l1) for (const lb of p.n.shared ?? []) { const k = lkey(lb); if (!by.has(k)) by.set(k, { key: k, label: lb, ids: new Set() }); by.get(k).ids.add(p.n.id); }
    const picked = [], covered = new Set();
    while (picked.length < 4) {
      let best = null, bn = 1;
      for (const c of by.values()) { if (picked.includes(c)) continue; const n = [...c.ids].filter((id) => !covered.has(id)).length; if (n > bn) { bn = n; best = c; } }
      if (!best) break;
      best.ids = [...best.ids].filter((id) => !covered.has(id)); best.ids.forEach((id) => covered.add(id)); picked.push(best);
    }
    const used = new Set();
    for (const r of picked) {
      let ci = [...r.key].reduce((a, ch) => (a * 31 + ch.charCodeAt(0)) >>> 0, 7) % 4;
      while (used.has(ci)) ci = (ci + 1) % 4;
      used.add(ci); r.ci = ci;
    }
    return picked;
  }

  // place the neighbours on the screen plane (then lifted onto the sphere): angle from the factor
  // shares (screen angle = compass angle), distance from how unlike; overlaps pushed apart, clear of
  // the focus, the panels and the edges. Beyond the threshold: badges along the limb, by their parent.
  function place(focusItem, list, L) {
    const rv = L.rv0;
    const l1 = list.filter((n) => n.layer === 1).slice(0, L.narrow ? 8 : 64), l2 = L.narrow ? [] : list.filter((n) => n.layer > 1);
    const smax = Math.max(...l1.map((n) => n.score), 1e-9), smin = Math.min(...l1.map((n) => n.score), smax);
    const out = new Map();
    for (const n of l1) {
      const it = S.items.get(n.id); if (!it) continue;
      const newer = it.time >= focusItem.time;
      const ang = Compass.angleOf(n.parts, newer);
      const t = smax > smin ? (smax - n.score) / (smax - smin) : 0.5;   // 0 = the most alike
      const r = rv * (0.47 + 0.3 * t);
      out.set(n.id, { n, role: "l1", ang, newer, t, x: Math.cos(ang) * r, y: -Math.sin(ang) * r });
    }
    // regions take their own sectors around the focus (the rest keep one, in compass order)
    const regions = regionsOf([...out.values()]);
    if (regions.length) {
      const inR = new Map(); regions.forEach((r, i) => r.ids.forEach((id) => inR.set(id, i)));
      const rest = [...out.values()].filter((p) => !inR.has(p.n.id)).sort((a, b) => a.ang - b.ang);
      const groups = [...regions.map((r, i) => ({ i, ps: r.ids.map((id) => out.get(id)).sort((a, b) => a.t - b.t) })), ...(rest.length ? [{ i: null, ps: rest }] : [])];
      const GAP = 0.2, wsum = groups.reduce((a, g) => a + Math.max(g.ps.length, 2), 0), free = 2 * Math.PI - GAP * groups.length;
      let a = Math.PI * 0.62;   // the first region starts at the upper left and they go clockwise
      for (const g of groups) {
        const span = (free * Math.max(g.ps.length, 2)) / wsum, n = g.ps.length;
        // the most alike in the middle of its sector, the others alternately to its sides
        const order = []; g.ps.forEach((p, k) => (k % 2 ? order.push(p) : order.unshift(p)));
        order.forEach((p, k) => {
          p.region = g.i; p.ang = a - (span * (k + 0.5)) / n;
          const r = Math.hypot(p.x, p.y); p.x = Math.cos(p.ang) * r; p.y = -Math.sin(p.ang) * r;
        });
        if (g.i != null) Object.assign(regions[g.i], { a0: a - span, a1: a });
        a -= span + GAP;
      }
    } else spread([...out.values()]);
    const rect = (el, pad) => { const b = el.getBoundingClientRect(); return [b.left - L.cx - pad, b.top - L.cy - pad, b.right - L.cx + pad, b.bottom - L.cy + pad]; };
    const panels = [rect($("filters"), 8), rect($("card"), 8), [L.w - 130 - L.cx, L.h - 130 - L.cy, L.w - L.cx, L.h - L.cy]];
    const keep = [...panels, [-L.F / 2 - 14, -L.F / 2 - 14, L.F / 2 + 14, L.F / 2 + 48]];   // the focus and its name
    const items = [...out.values()];
    const ax = Math.max(L.B, 150) / 2 + 2, ay = L.B / 2 + 13, as = 9;   // footprint: the orb and its name (shifted down)
    const rmin = L.F / 2 + L.B / 2 + 16, rmax = rv * (regions.length ? 0.64 : 0.82);   // regions keep the rim for their labels
    for (let iter = 0; iter < 160; iter++) {
      for (let i = 0; i < items.length; i++) {
        const A = items[i];
        for (let j = i + 1; j < items.length; j++) {
          const B = items[j];
          const dx = B.x - A.x, dy = B.y - A.y, ox = 2 * ax - Math.abs(dx), oy = 2 * ay - Math.abs(dy);
          if (ox > 0 && oy > 0) {
            if (ox / ax < oy / ay) { const s = Math.sign(dx) || 1; A.x -= (s * ox) / 2; B.x += (s * ox) / 2; }
            else { const s = Math.sign(dy) || 1; A.y -= (s * oy) / 2; B.y += (s * oy) / 2; }
          }
        }
        for (const [x0, y0, x1, y1] of keep) {
          const ay0 = A.y + as - ay, ay1 = A.y + as + ay;
          if (A.x + ax > x0 && A.x - ax < x1 && ay1 > y0 && ay0 < y1) {
            const pushL = A.x + ax - x0, pushR = x1 - (A.x - ax), pushU = ay1 - y0, pushD = y1 - ay0;
            const m = Math.min(pushL, pushR, pushU, pushD);
            if (m === pushL) A.x -= m; else if (m === pushR) A.x += m; else if (m === pushU) A.y -= m; else A.y += m;
          }
        }
        const r = Math.hypot(A.x, A.y) || 1, k = r > rmax ? rmax / r : r < rmin ? rmin / r : 1;
        A.x *= k; A.y *= k;
      }
    }
    const ring = [], seen = new Set();
    for (const n of l2) {
      const p = out.get(n.via);
      if (!p || p.role !== "l1" || out.has(n.id) || seen.has(n.via) || !S.items.has(n.id)) continue;
      seen.add(n.via);
      ring.push({ n, role: "l2", newer: true, t: 1, ang: Math.atan2(-p.y, p.x) + 0.12 });
    }
    ring.sort((a, b) => a.ang - b.ang);
    const rr = rv * 0.965, gap = (L.D + 16) / rr;
    for (let iter = 0; iter < 40 && ring.length > 1; iter++) {
      for (let i = 0; i < ring.length; i++) {
        const a = ring[i], b = ring[(i + 1) % ring.length];
        const d = b.ang - a.ang + (i === ring.length - 1 ? 2 * Math.PI : 0);
        if (d < gap) { a.ang -= (gap - d) / 2; b.ang += (gap - d) / 2; }
      }
    }
    for (const q of ring) {
      q.x = Math.cos(q.ang) * rr; q.y = -Math.sin(q.ang) * rr;
      if (panels.some(([x0, y0, x1, y1]) => q.x + L.D > x0 && q.x - L.D < x1 && q.y + L.D > y0 && q.y - L.D < y1)) continue;
      out.set(q.n.id, q);
    }
    out.regions = regions.map((r) => ({ key: r.key, label: r.label, ci: r.ci, a0: r.a0, a1: r.a1, n: r.ids.length, r0: L.F / 2 + 22, r1: rv * 0.9 }));
    return out;
  }

  // Factor shares bunch neighbours into one quadrant (e.g. everything older and in the same
  // folder). Keep their order around the focus but give them room: at least 26° each, centred
  // where they were; the readings are taken from where they end up, so they stay truthful.
  function spread(list) {
    const n = list.length; if (n < 2) return;
    list.sort((a, b) => a.ang - b.ang);
    let gi = 0, gmax = -1;
    for (let i = 0; i < n; i++) { const g = ((list[(i + 1) % n].ang - list[i].ang) + 4 * Math.PI) % (2 * Math.PI) || 2 * Math.PI; if (g > gmax) { gmax = g; gi = (i + 1) % n; } }
    const ord = [...list.slice(gi), ...list.slice(0, gi)];
    const base = ord[0].ang, rel = ord.map((p) => ((p.ang - base) + 4 * Math.PI) % (2 * Math.PI));
    const span = rel[n - 1], need = Math.min((2 * Math.PI * (n - 1)) / n, ((26 * Math.PI) / 180) * (n - 1));
    if (span >= need) return;
    const mid = base + span / 2;
    ord.forEach((p, i) => {
      const even = mid - need / 2 + (need * i) / (n - 1);
      const a = 0.25 * (base + rel[i]) + 0.75 * even;
      const r = Math.hypot(p.x, p.y);
      p.ang = a; p.x = Math.cos(a) * r; p.y = -Math.sin(a) * r;
    });
  }

  // ── the scene (DOM): every file is a small round node — a glowing ring with its picture inside
  // (the kind's icon until a thumbnail arrives; beyond the threshold always the icon) ──
  const nodesEl = $("nodes"), stage = $("stage");
  const KIND_ICON = { pdf: "pdf", document: "file", text: "file", latex: "sigma", ebook: "book", slides: "layout", spreadsheet: "table", image: "image", video: "film", audio: "speaker", archive: "layers", code: "code", data: "database", project: "folder", dataset: "database", package: "layers", font: "text", installer: "download" };
  const iconFor = (it) => icon(typeof ICONS !== "undefined" && ICONS[KIND_ICON[it.kind]] ? KIND_ICON[it.kind] : "file");
  function makeEl(id, role, L) {
    const it = S.items.get(id);
    const d = role === "focus" ? L.F : role === "l1" ? L.B : L.D;
    const n = membersOf(id).length;
    const cv = role === "l2" ? null : document.createElement("canvas");
    const orb = h("span", { class: "orb" + (it.sensitive ? " sens" : ""), style: `width:${d}px;height:${d}px;margin:${-d / 2}px 0 0 ${-d / 2}px` },
      iconFor(it), cv,
      it.sensitive && cv ? h("span", { class: "veil", "aria-hidden": "true" }, icon("eyeOff")) : null,
      n > 1 && cv ? h("span", { class: "badge" }, `×${n}`) : null);
    const el = h("div", { role: "button", tabindex: role === "focus" ? null : "0", class: `node ${role}`, "data-id": id, "aria-label": n > 1 ? `${it.name} ×${n}` : it.name },
      orb, cv ? h("span", { class: "nm", style: `top:${d / 2 + 8}px` }, n > 1 ? stemOf(it.name) : it.name) : null);
    el.style.visibility = "hidden";   // until the first frame places it
    nodesEl.append(el);
    const e = { el, cv, d, thumbed: null, deep: false };
    const t = cv && (S.thumbs.get(`${id}@${thumbSize(role, it)}`) ?? S.thumbs.get(`${id}@256`) ?? S.thumbs.get(`${id}@128`));
    if (t) paintOrb(e, t.bmp);
    return e;
  }
  // under a neighbour: what it is about (its labels, without its region's — that one is written on
  // the sphere), else the start of its name; the whole name while the pointer is on it
  const shortName = (name) => { const st = stemOf(name); return st.length > 12 ? `${st.slice(0, 10)}…` : st; };
  function nameNode(id, e) {
    const nm = e.el?.querySelector(".nm"); if (!nm) return;
    for (let i = 0; i < 4; i++) e.el.classList.toggle(`rg${i}`, e.region != null && S.regions[e.region]?.ci === i);
    if (e.role !== "l1") return;
    const it = S.items.get(id), rk = e.region != null ? S.regions[e.region].key : null;
    const labs = it.labels.filter((l) => lkey(l) !== rk).slice(0, 2);
    const full = membersOf(id).length > 1 ? stemOf(it.name) : it.name;
    const more = it.labels.filter((l) => !labs.includes(l));
    nm.replaceChildren(h("span", { class: labs.length ? "lb" : "lb stub" }, labs.length ? labs.join(" · ") : shortName(it.name)), more.length ? h("span", { class: "more" }, more.join(" · ")) : null, h("span", { class: "fn" }, full));
  }
  // the picture, cropped to fill the circle ("切邊"); a page shows its top
  function paintOrb(e, bmp) {
    const px = Math.round(e.d * dprOf()), cv = e.cv;
    cv.width = cv.height = px;
    const c = cv.getContext("2d"), k = Math.max(px / bmp.width, px / bmp.height), w = bmp.width * k, hh = bmp.height * k;
    c.fillStyle = "#fbfaf6"; c.fillRect(0, 0, px, px);   // paper under a transparent picture (a document's own colour)
    c.drawImage(bmp, (px - w) / 2, (px - hh) * (hh > px * 1.05 ? 0.1 : 0.5), w, hh);
    e.el.classList.add("pic");
  }
  function dropEntry(e) { if (e.cv) e.cv.width = e.cv.height = 0; e.el?.remove(); e.el = null; }
  function setRole(e, id, role, L) {
    if (e.role === role && e.el) return;
    const oldD = e.d, old = { el: e.el, cv: e.cv };
    Object.assign(e, makeEl(id, role, L), { role });
    e.vis = "hidden"; e.zi = undefined;   // a new element starts hidden (makeEl): the per-frame cache must know
    if (old.el) { dropEntry(old); e.k = ((e.k ?? 1) * oldD) / e.d; }   // keep the size it is shown at, then ease
  }

  // build the neighbourhood of S.focus from a neighbour list. Shared nodes glide over the sphere
  // from where they are to their new place; only entering and leaving ones fade.
  function build(list, { fresh = false } = {}) {
    const L = layout();
    const f = S.items.get(S.focus);
    if (!f) return;
    const placed = place(f, list, L), R0 = L.rv0 / LIMB;
    const next = new Map();
    const take = (id, role, t, extra) => {
      let e = S.nodes.get(id);
      if (e) S.nodes.delete(id);
      else e = { v: t, a: fresh ? 1 : 0, k: extra.kT };
      setRole(e, id, role, L);
      Object.assign(e, { id, t, aT: 1 }, extra);
      next.set(id, e);
    };
    take(S.focus, "focus", [0, 0, 1], { parts: null, w: 0, kT: 1 });
    for (const [id, p] of placed) {
      const l1 = p.role === "l1";
      take(id, p.role, toVec(p.x, p.y, R0), { ang: Math.atan2(-p.y, p.x), parts: p.n.parts, newer: l1 ? p.newer : true, via: p.n.via, w: l1 ? 1 - p.t : 0, kT: l1 ? 1 - 0.22 * p.t : 1, region: l1 ? p.region ?? null : null, link: null });
    }
    S.regions = placed.regions ?? [];
    S.R0 = R0;
    // a region's files hang together: the most alike holds on to the focus, the others to the next one towards it
    S.regions.forEach((r, i) => {
      const ms = [...next.values()].filter((e) => e.region === i).sort((a, b) => b.ang - a.ang);
      const hub = ms.reduce((b, e, k) => (e.w > ms[b].w ? k : b), 0);
      ms.forEach((e, k) => { e.link = k === hub ? null : ms[k < hub ? k + 1 : k - 1].id; });
    });
    for (const [id, e] of next) nameNode(id, e);
    if (!S.hover) updateCard();
    for (const e of S.nodes.values()) { e.aT = 0; e.t = e.v; S.ghosts.push(e); }
    S.nodes = next;
    paintAxes(L);
    loadFar();
    frames.invalidate();
    loadThumbs();
  }

  // ── QuickLook thumbnails (GET /api/thumb/{id}?s=): the focus 256, a neighbour 128, the hover
  // preview 512; 404 → the kind's icon. Kept as ImageBitmaps under a byte cap (least recently used first).
  const THUMB_CAP = 48 << 20;
  const IMG_EXT = new Set(["png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp", "heic", "dng", "gif"]);
  // a sensitive item (證件) is never fetched large
  const thumbSize = (role, it) => (role === "preview" ? (it?.sensitive ? 256 : 512) : role === "focus" ? 256 : role === "l1" ? (S.zoomT > 1.25 ? 256 : 128) : 0);
  async function getThumb(id, size, url = `/api/thumb/${encodeURIComponent(id)}?s=${size}`) {
    const key = `${id}@${size}`;
    const hit = S.thumbs.get(key);
    if (hit) { hit.used = performance.now(); return hit.bmp; }
    if (S.demo || S.noThumb.has(key)) return null;
    try {
      const r = await fetch(url);
      if (!r.ok) { S.noThumb.add(key); return null; }
      const bmp = await createImageBitmap(await r.blob());
      const bytes = bmp.width * bmp.height * 4;
      S.thumbs.set(key, { bmp, bytes, used: performance.now() });
      S.thumbBytes += bytes;
      while (S.thumbBytes > THUMB_CAP && S.thumbs.size > 1) {
        let old = null, ou = Infinity;
        for (const [k2, t] of S.thumbs) if (k2 !== key && t.used < ou) { ou = t.used; old = k2; }
        if (!old) break;
        dropThumb(old);
      }
      return bmp;
    } catch { S.noThumb.add(key); return null; }
  }
  function dropThumb(key) { const t = S.thumbs.get(key); if (!t) return; t.bmp.close?.(); S.thumbBytes -= t.bytes; S.thumbs.delete(key); }
  let thumbRun = 0;
  async function loadThumbs() {
    const run = ++thumbRun;
    const order = [...S.nodes].filter(([, e]) => e.cv).sort((a, b) => (a[1].role === "focus" ? -1 : 0) - (b[1].role === "focus" ? -1 : 0));
    for (const [id, e] of order) {
      if (run !== thumbRun || S.tier > 2) return;
      const key = `${id}@${thumbSize(e.role, S.items.get(id))}`;
      if (e.thumbed === key) continue;
      const bmp = await getThumb(id, thumbSize(e.role, S.items.get(id)));
      if (!bmp || run !== thumbRun || S.nodes.get(id) !== e || !e.cv) continue;
      paintOrb(e, bmp);
      e.thumbed = key;
    }
  }

  const smooth = (dt, tau) => 1 - Math.exp(-dt / tau);
  function tick(dt) {
    if (S.tier >= 4) return false;
    let busy = false;
    if (Math.abs(S.zoom - S.zoomT) > 1e-3) { S.zoom += (S.zoomT - S.zoom) * smooth(dt, 80); busy = true; } else S.zoom = S.zoomT;
    const L = layout();
    // after a release: coast with the finger's speed, then settle on the node nearest the pole
    if (S.coast) {
      S.off[0] += S.vel[0] * dt; S.off[1] += S.vel[1] * dt;
      const k = Math.exp(-dt / 170); S.vel = S.vel.map((v) => v * k);
      // decide once it is slow (the landing glide takes over); once settled, no candidate is picked again in the same frame
      if (Math.hypot(...S.vel) < 0.15) { S.coast = false; settle(); } else pickCandidate();
      busy = true;
    } else if (!S.swipe && !S.wheeling && (S.off[0] || S.off[1])) {
      const k = smooth(dt, 90); S.off = S.off.map((x) => (Math.abs(x) < 0.3 ? 0 : x * (1 - k))); busy = true;
    }
    // landing: globe and nodes glide together; a shorter time constant ends the slow tail sooner
    const kP = smooth(dt, 85), kA = smooth(dt, 110);
    // landing on a new focus: the wire globe (and what is leaving) turns the same way the nodes glide
    if (S.snap) {
      const da = S.snap.left * kP, Rm = rotAxis(S.snap.axis, da);
      S.G = mulM(Rm, S.G);
      for (const g of S.ghosts) g.v = g.t = mulV(Rm, g.v);
      S.snap.left -= da;
      if (S.snap.left < 2e-3) S.snap = null;
      busy = true;
    }
    const step = (e) => {
      let moved = false;
      const dx = e.t[0] - e.v[0], dy = e.t[1] - e.v[1], dz = e.t[2] - e.v[2];
      if (Math.hypot(dx, dy, dz) > 1.5e-3) { e.v = norm([e.v[0] + dx * kP, e.v[1] + dy * kP, e.v[2] + dz * kP]); moved = true; } else e.v = e.t;
      if (Math.abs(e.k - e.kT) > 0.004) { e.k += (e.kT - e.k) * kP; moved = true; } else e.k = e.kT;
      if (Math.abs(e.a - e.aT) > 0.01) { e.a += (e.aT - e.a) * kA; moved = true; } else e.a = e.aT;
      if (moved) busy = true;
    };
    for (const e of S.nodes.values()) step(e);
    for (const g of S.ghosts) step(g);
    S.ghosts = S.ghosts.filter((g) => { if (g.a > 0.02) return true; dropEntry(g); return false; });
    const moving = !!(S.swipe?.moved || S.coast || S.wheeling);
    if (moving !== S.moving) { S.moving = moving; stage.classList.toggle("moving", moving); if (moving) hidePreview(); }
    draw(L);
    S.frames++;
    // (resting again waits for the focus that was asked for)
    if (S.pauseWhenSettled && !busy && S.nodes.size > 1 && (S.pending == null || S.pending === S.focus)) { S.pauseWhenSettled = false; S.tier = 2; frames.pause(true); return false; }
    if (S.restoreT0 && S.restoreMs == null) { S.restoreMs = performance.now() - S.restoreT0; S.restoreT0 = 0; }
    return busy || moving;
  }
  const smoothstep = (a, b, x) => { const t = Math.min(1, Math.max(0, (x - a) / (b - a))); return t * t * (3 - 2 * t); };

  // ── theme colours, read from the theme's roles at runtime (never literal values): ground and fog
  // --bg/--panel; grid, ring and neutral bonds --line-strong/--muted; badges and their glow
  // --accent-2; the focus frame and the candidate's bond --accent
  const col = {};
  const probe = document.createElement("canvas").getContext("2d");
  const css = (r, a = 1) => `rgba(${Math.round(r[0] * 255)},${Math.round(r[1] * 255)},${Math.round(r[2] * 255)},${a})`;
  const mixc = (a, b, t) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
  function rgbOf(v) {
    probe.fillStyle = "#000"; probe.fillStyle = (v || "#888").trim();
    const x = probe.fillStyle;
    if (x[0] === "#") return [1, 3, 5].map((i) => parseInt(x.slice(i, i + 2), 16) / 255);
    const m = x.match(/[\d.]+/g).map(Number); return [m[0] / 255, m[1] / 255, m[2] / 255];
  }
  function readTheme() {
    const cs = getComputedStyle(document.documentElement), v = (n) => rgbOf(cs.getPropertyValue(n));
    col.light = document.documentElement.dataset.mode === "light";
    Object.assign(col, { accent: v("--accent"), bg: v("--bg"), panel: v("--panel"), surface: v("--surface"), line: v("--line-strong"), muted: v("--muted"), text: v("--text") });
    col.a2 = col.light ? mixc(v("--accent-2-text"), col.muted, 0.3) : v("--accent-2");   // light themes: a quieter accent
    col.wire = col.light ? col.line : mixc(col.line, col.a2, 0.45);                           // the globe, the rings
    col.bond = col.light ? mixc(col.line, col.muted, 0.6) : mixc(col.muted, col.a2, 0.35);    // neutral bonds
    col.dot = mixc(col.muted, col.a2, 0.4);                                                   // far files
    col.rg = [v("--accent-2"), v("--accent-3"), v("--warn"), v("--accent")];   // region colours (styles.css .rg0–.rg3)
    col.stamp = (col.stamp || 0) + 1;
    frames.invalidate();
  }

  // ── layers, back to front ──
  //   L0 ground (¼ res, painted once): the glow of the sphere, its body, the light under it
  //   L1 globe (½ res): the wire sphere, the far files as soft dots, the rings it floats over
  //   L2 compass + bonds (full res): the limb ring with ticks, the bonds along the sphere
  //   L3 badges · L4 neighbours · L5 the focus and the panels (DOM)
  // Depth is real: everything sits on one sphere, so turning it moves the near side with the
  // finger and the far side against it. Nothing animates at rest.
  const LAYER = { back: [$("lback"), 0.25], globe: [$("lglobe"), 0.5], bond: [$("lbond"), 1] };
  const dprOf = () => Math.min(2, devicePixelRatio || 1);
  function ctxOf(name, L) {
    const [cv, scale] = LAYER[name], k = dprOf() * scale;
    const W = Math.round(L.w * k), H = Math.round(L.h * k);
    if (cv.width !== W || cv.height !== H) { cv.width = W; cv.height = H; }
    const c = cv.getContext("2d");
    c.setTransform(k, 0, 0, k, 0, 0);
    c.clearRect(0, 0, L.w, L.h);
    return c;
  }
  function freeLayers() { for (const [cv] of Object.values(LAYER)) cv.width = cv.height = 0; S.backKey = null; }
  const layerBytes = () => Object.fromEntries(Object.entries(LAYER).map(([k, [cv]]) => [k, +((cv.width * cv.height * 4) / 1048576).toFixed(2)]));

  // L0: where the sphere is — a halo at its limb, a lit body, a soft light from the ring below
  function paintBack(L) {
    const c = ctxOf("back", L), { cx, cy, rv } = L, glow = col.a2, k = col.light ? 0.7 : 1;
    let g = c.createRadialGradient(cx, cy, 0, cx, cy, Math.max(L.w, L.h) * 0.62);
    g.addColorStop(0, css(col.surface, col.light ? 0.5 : 0.42)); g.addColorStop(1, css(col.surface, 0));
    c.fillStyle = g; c.fillRect(0, 0, L.w, L.h);
    g = c.createRadialGradient(cx, cy, rv * 0.8, cx, cy, rv * 1.3);
    g.addColorStop(0, css(glow, 0)); g.addColorStop(0.4, css(glow, 0.13 * k)); g.addColorStop(1, css(glow, 0));
    c.fillStyle = g; c.fillRect(0, 0, L.w, L.h);
    g = c.createRadialGradient(cx - rv * 0.32, cy - rv * 0.38, 0, cx, cy, rv);
    g.addColorStop(0, css(glow, 0.1 * k)); g.addColorStop(0.7, css(glow, 0.03 * k)); g.addColorStop(1, css(col.bg, col.light ? 0 : 0.25));
    c.fillStyle = g; c.beginPath(); c.arc(cx, cy, rv, 0, 7); c.fill();
    // rays from the far distance towards the sphere
    c.lineWidth = 1;
    for (let i = 0; i < 40; i++) {
      const a = (i / 40) * 2 * Math.PI + 0.07, r0 = rv * (1.5 + 0.35 * ((i * 7) % 5) / 4), r1 = Math.max(L.w, L.h);
      const ray = c.createLinearGradient(cx + Math.cos(a) * r0, cy + Math.sin(a) * r0, cx + Math.cos(a) * r1, cy + Math.sin(a) * r1);
      ray.addColorStop(0, css(col.wire, 0)); ray.addColorStop(0.25, css(col.wire, (col.light ? 0.3 : 0.22) * (i % 3 ? 0.6 : 1))); ray.addColorStop(1, css(col.wire, 0));
      c.strokeStyle = ray; c.beginPath(); c.moveTo(cx + Math.cos(a) * r0, cy + Math.sin(a) * r0); c.lineTo(cx + Math.cos(a) * r1, cy + Math.sin(a) * r1); c.stroke();
    }
    const by = cy + rv * 1.17;
    g = c.createLinearGradient(0, by, 0, cy + rv * 0.55);
    g.addColorStop(0, css(glow, 0.1 * k)); g.addColorStop(1, css(glow, 0));
    c.fillStyle = g; c.beginPath(); c.moveTo(cx - rv * 0.6, by); c.lineTo(cx + rv * 0.6, by); c.lineTo(cx + rv * 0.86, cy + rv * 0.55); c.lineTo(cx - rv * 0.86, cy + rv * 0.55); c.closePath(); c.fill();
  }

  // the wire sphere: meridians and parallels in object space
  const WIRE = (() => {
    const cs = [], N = 60;
    for (let k = 0; k < 6; k++) { const lon = (k * Math.PI) / 6, pts = []; for (let i = 0; i <= N; i++) { const t = (i / N) * 2 * Math.PI; pts.push([Math.sin(t) * Math.cos(lon), Math.cos(t), Math.sin(t) * Math.sin(lon)]); } cs.push(pts); }
    for (const lat of [-60, -30, 0, 30, 60]) { const p = (lat * Math.PI) / 180, pts = []; for (let i = 0; i <= N; i++) { const t = (i / N) * 2 * Math.PI; pts.push([Math.cos(p) * Math.cos(t), Math.sin(p), Math.cos(p) * Math.sin(t)]); } cs.push(pts); }
    return cs;
  })();
  // the far files: real further neighbours of the focus (never the whole disk), at most 70, on the
  // rest of the sphere — most of them on its far side
  async function loadFar() {
    const id = S.focus, cat = catParam();
    let list = [];
    try { list = await api("GET", `/api/neighbors/${encodeURIComponent(id)}?k=80&layers=1${cat ? `&cat=${encodeURIComponent(cat)}` : ""}`); } catch { return; }
    if (S.focus !== id || S.tier >= 3) return;
    const f = S.items.get(id), shown = new Set(S.nodes.keys()), Gt = transpose(S.G);
    const far = (Array.isArray(list) ? list : list.neighbors ?? []).filter((n) => !shown.has(n.id) && S.items.has(n.id)).slice(0, 70);
    S.far = far.map((n, i) => {
      const it = S.items.get(n.id), hsh = [...n.id.slice(0, 8)].reduce((a, ch) => (a * 31 + ch.charCodeAt(0)) >>> 0, 7);
      const th = Compass.angleOf(n.parts, it.time >= f.time) + ((hsh % 1000) / 1000 - 0.5) * 1.6;
      const ph = 1.02 + 1.95 * (i / far.length) + (((hsh >> 10) % 100) / 100 - 0.5) * 0.3;
      return { o: mulV(Gt, [Math.sin(ph) * Math.cos(th), Math.sin(ph) * Math.sin(th), Math.cos(ph)]), s: 1.6 + 2.6 * (1 - i / far.length) };
    });
    frames.invalidate();
  }
  // a point of the screen plane (zoom 1, y down) as it is on the turned sphere now: [x, y, z]
  const onSphere = (x, y, M, L) => { const v = mulV(M, toVec(x, y, S.R0)), p = proj(v, L); return [p[0], p[1], v[2]]; };
  // a region: a soft light of its colour under each of its files (author's choice, 2026-11-13)
  function paintRegions(c, L, M, near) {
    const zl = 1 / CAM;
    for (const e of S.nodes.values()) {
      const r = e.region != null ? S.regions[e.region] : null; if (!r) continue;
      const rgb = col.rg[r.ci], v = mulV(M, e.v), [x, y, s] = proj(v, L), a = near * e.a * smoothstep(zl - 0.05, zl + 0.3, v[2]);
      if (a < 0.01) continue;
      const R = L.B * 1.9 * s * L.sc, g = c.createRadialGradient(x, y, 0, x, y, R);
      g.addColorStop(0, css(rgb, 0.3 * a)); g.addColorStop(0.5, css(rgb, 0.12 * a)); g.addColorStop(1, css(rgb, 0));
      c.fillStyle = g; c.beginPath(); c.arc(x, y, R, 0, 7); c.fill();
    }
  }
  // words written on the sphere: along the circle of radius `rho` around the pole (screen plane,
  // zoom 1), centred at angle `mid`; every letter lies in the sphere's surface there, so the line
  // bends with it and turns with the globe. Upright on the upper half, and on the lower.
  function sphereText(c, str, M, L, rho, mid, rgb, alpha, px) {
    const top = Math.sin(mid) >= 0, zl = 1 / CAM;
    c.font = `700 ${px}px "JetBrains Mono", ui-monospace, monospace`; c.textAlign = "center"; c.textBaseline = "middle";
    const chars = [...str], adv = chars.map((ch) => c.measureText(ch).width + px * 0.14), total = adv.reduce((a, b) => a + b, 0);
    let sx = -total / 2;
    chars.forEach((ch, i) => {
      const at = sx + adv[i] / 2; sx += adv[i];
      if (ch === " ") return;
      const a = mid + ((top ? -1 : 1) * at) / rho, ca = Math.cos(a), sa = Math.sin(a);
      const x = rho * ca, y = -rho * sa, t = top ? [sa, ca] : [-sa, -ca], u = top ? [ca, -sa] : [-ca, sa];
      const p0 = onSphere(x, y, M, L), p1 = onSphere(x + t[0] * 2, y + t[1] * 2, M, L), p2 = onSphere(x + u[0] * 2, y + u[1] * 2, M, L);
      const al = alpha * smoothstep(zl - 0.02, zl + 0.22, p0[2]);
      if (al < 0.01) return;
      c.save();
      c.transform((p1[0] - p0[0]) / 2 / L.sc, (p1[1] - p0[1]) / 2 / L.sc, -(p2[0] - p0[0]) / 2 / L.sc, -(p2[1] - p0[1]) / 2 / L.sc, p0[0], p0[1]);
      c.scale(L.sc, L.sc);
      c.fillStyle = css(rgb, al); c.fillText(ch, 0, 0);
      c.restore();
    });
    return total;
  }
  function paintRegionWords(c, L, M, near) {
    for (const r of S.regions ?? []) {
      const rgb = col.rg[r.ci], mid = (r.a0 + r.a1) / 2, span = r.a1 - r.a0;
      // on the rim beyond its files, as large as its sector lets it be
      const str = r.label.toUpperCase(), rho = r.r1 - L.rv0 * 0.05;
      const px = clamp((span * rho * 0.86) / (str.length * 0.74), 11, L.rv0 * 0.075);
      sphereText(c, str, M, L, rho, mid, col.light ? mixc(rgb, col.text, 0.35) : rgb, 0.62 * near, px);
    }
  }

  function paintGlobe(L, MG, near, M) {
    const c = ctxOf("globe", L);
    if (near < 0.01) return;
    paintRegions(c, L, M, near);
    const zl = 1 / CAM, { cx, cy, rv } = L;
    const front = new Path2D(), back = new Path2D();
    for (const pts of WIRE) {
      let px = 0, py = 0, pz = 0;
      for (let i = 0; i < pts.length; i++) {
        const v = mulV(MG, pts[i]), s = (CAM - 1) / (CAM - v[2]), x = cx + L.R * v[0] * s, y = cy - L.R * v[1] * s;
        if (i) { const p = (v[2] + pz) / 2 > zl ? front : back; p.moveTo(px, py); p.lineTo(x, y); }
        px = x; py = y; pz = v[2];
      }
    }
    c.lineWidth = 1;
    c.strokeStyle = css(col.wire, (col.light ? 0.2 : 0.13) * near); c.stroke(back);
    c.strokeStyle = css(col.wire, (col.light ? 0.55 : 0.4) * near); c.stroke(front);
    // far files: the near side larger and brighter, the far side small and dim
    for (const d of S.far) {
      const v = mulV(MG, d.o), [x, y, s] = proj(v, L), fr = v[2] > zl;
      c.fillStyle = css(col.dot, (fr ? 0.5 + 0.3 * v[2] : 0.22) * near);
      c.beginPath(); c.arc(x, y, d.s * s * (fr ? 1.25 : 0.8), 0, 7); c.fill();
    }
    // beyond the sphere (never across it): a floor of rings and spokes it floats over, two orbits, a dial
    const k = near * (col.light ? 1.25 : 1), by = cy + rv * 1.17, FL = 0.087;
    c.save();
    c.beginPath(); c.rect(0, 0, L.w, L.h); c.arc(cx, cy, rv + 2, 0, 7); c.clip("evenodd");
    for (const [rx, a] of [[0.6, 0.5], [0.93, 0.3], [1.3, 0.2], [1.8, 0.13], [2.5, 0.08], [3.4, 0.05]]) {
      c.strokeStyle = css(col.wire, a * k);
      c.beginPath(); c.ellipse(cx, by, rv * rx, rv * rx * FL, 0, 0, 7); c.stroke();
    }
    c.strokeStyle = css(col.wire, 0.1 * k); c.beginPath();
    for (let i = 0; i < 24; i++) { const a = (i / 24) * 2 * Math.PI, x = Math.cos(a), y = Math.sin(a) * FL; c.moveTo(cx + x * rv * 0.6, by + y * rv * 0.6); c.lineTo(cx + x * rv * 3.4, by + y * rv * 3.4); }
    c.stroke();
    for (const [rx, ry, rot, a] of [[1.62, 0.5, -0.3, 0.2], [2.05, 0.42, 0.16, 0.12]]) {
      c.strokeStyle = css(col.wire, a * k);
      c.beginPath(); c.ellipse(cx, cy, rv * rx, rv * ry, rot, 0, 7); c.stroke();
    }
    c.strokeStyle = css(col.wire, 0.34 * k);
    for (let q = 0; q < 4; q++) { const a0 = (q * Math.PI) / 2 + 0.24; c.beginPath(); c.arc(cx, cy, rv * 1.2, a0, a0 + Math.PI / 2 - 0.48); c.stroke(); }
    c.strokeStyle = css(col.wire, 0.22 * k); c.beginPath();
    for (let d = 0; d < 360; d += 2) { const m = ((d % 90) + 90) % 90; if (m < 16 || m > 74) continue; const a = (d * Math.PI) / 180, l = d % 10 ? 3 : 6; c.moveTo(cx + Math.cos(a) * rv * 1.2, cy + Math.sin(a) * rv * 1.2); c.lineTo(cx + Math.cos(a) * (rv * 1.2 + l), cy + Math.sin(a) * (rv * 1.2 + l)); }
    c.stroke();
    c.restore();
  }

  function draw(L) {
    const near = smoothstep(0.35, 0.75, S.zoom);
    const M = turn(L), zl = 1 / CAM;
    const key = `${L.w}|${L.h}|${Math.round(L.cx)}|${Math.round(L.cy)}|${Math.round(L.rv)}|${col.stamp}`;
    if (S.backKey !== key) { paintBack(L); S.backKey = key; }
    LAYER.back[0].style.opacity = near.toFixed(3);
    paintGlobe(L, mulM(M, S.G), near, M);
    // nothing is drawn under the filter panel or the info card: within 24 px of their edge things fade out
    const panels = [$("filters"), $("card")].filter((p) => !p.hidden).map((p) => p.getBoundingClientRect());
    const clear = (x, y, r) => {
      let f = 1;
      for (const P of panels) {
        const ox = Math.min(x + r, P.right) - Math.max(x - r, P.left), oy = Math.min(y + r, P.bottom) - Math.max(y - r, P.top);
        if (ox > 0 && oy > 0) f = Math.min(f, 1 - Math.min(1, Math.min(ox, oy) / 24));
      }
      return f;
    };
    // nodes: size and fog from depth (the pole is nearest); behind the limb they are gone
    const geo = new Map();
    const put = (id, e) => {
      const v = mulV(M, e.v), [x, y] = proj(v, L);
      const depth = smoothstep(0.1, 0.96, v[2]);
      const scale = L.sc * e.k * (0.58 + 0.42 * depth), r = (e.d * scale) / 2;
      const a = e.a * near * smoothstep(zl - 0.12, zl + 0.1, v[2]) * (e.role === "l2" ? 0.5 + 0.5 * depth : 0.45 + 0.55 * depth) * clear(x, y, r);
      e.el.style.transform = `translate3d(${x.toFixed(1)}px, ${y.toFixed(1)}px, 0) scale(${scale.toFixed(4)})`;
      e.el.style.opacity = a.toFixed(3);
      // style writes only on change: rewriting them every frame makes the page recompute styles
      const vis = a < 0.01 ? "hidden" : "", zi = e.role === "focus" ? 3 : e.role === "l1" ? 2 : 1;
      if (e.vis !== vis) { e.vis = vis; e.el.style.visibility = vis; }
      if (e.zi !== zi) { e.zi = zi; e.el.style.zIndex = zi; }
      const deep = v[2] < 0.62;   // near the limb the names would pile up: only the frames stay
      if (deep !== e.deep) { e.deep = deep; e.el.classList.toggle("deep", deep); }
      if (id) geo.set(id, { v, x, y, r, a });
    };
    for (const [id, e] of S.nodes) put(id, e);
    for (const g of S.ghosts) if (g.el) put(null, g);

    const c = ctxOf("bond", L);
    if (near > 0.01) {
      // the compass: the sphere's limb, with ticks; while dragging, the side things come from is lit
      c.lineWidth = 1;
      c.strokeStyle = css(col.wire, (col.light ? 0.8 : 0.7) * near);
      c.beginPath(); c.arc(L.cx, L.cy, L.rv, 0, 7); c.stroke();
      c.beginPath();
      for (let d = 0; d < 360; d += 6) { const a = (d * Math.PI) / 180, l = d % 30 ? 3 : 8, x = Math.cos(a), y = -Math.sin(a); c.moveTo(L.cx + x * (L.rv + 3), L.cy + y * (L.rv + 3)); c.lineTo(L.cx + x * (L.rv + 3 + l), L.cy + y * (L.rv + 3 + l)); }
      c.strokeStyle = css(col.muted, 0.5 * near); c.stroke();
      if (S.liveTh != null) {
        c.lineCap = "round";
        c.beginPath(); c.arc(L.cx, L.cy, L.rv, -S.liveTh - 0.2, -S.liveTh + 0.2);
        // the glow: wide faint strokes under the line (no shadowBlur: it blurs the whole layer every frame)
        if (!col.light) for (const [w, a] of [[11, 0.10], [7, 0.18], [4.5, 0.3]]) { c.strokeStyle = css(col.accent, a); c.lineWidth = w; c.stroke(); }
        c.strokeStyle = css(col.accent, 0.95); c.lineWidth = 2.5; c.stroke();
      }
      // callouts: from the limb out to what lies that way
      if (near > 0.95) for (const q of S.callouts ?? []) {
        const on = q.el.classList.contains("on"), [p0, p1, p2] = q.line;
        c.strokeStyle = c.fillStyle = css(on ? col.accent : col.bond, on ? 1 : 0.85); c.lineWidth = on ? 1.6 : 1;
        c.beginPath(); c.moveTo(p0[0], p0[1]); c.lineTo(p1[0], p1[1]); c.lineTo(p2[0], p2[1]); c.stroke();
        c.beginPath(); c.arc(p0[0], p0[1], on ? 2.6 : 2, 0, 7); c.fill();
      }
      paintRegionWords(c, L, M, near);
      // bonds: arcs along the sphere from frame to frame; thickness = similarity; the candidate's in --accent
      c.lineCap = "round";
      for (const [id, e] of S.nodes) {
        if (e.role === "focus") continue;
        const A = geo.get(e.role === "l1" ? e.link ?? S.focus : e.via), B = geo.get(id);
        const rgb = e.region != null && S.regions[e.region] ? col.rg[S.regions[e.region].ci] : col.bond;
        if (!A || !B) continue;
        const lit = id === S.candidate;
        const a = Math.min(A.a, B.a) * (lit ? 1 : e.region != null ? 0.8 : e.role === "l1" ? 0.62 : 0.4);
        if (a < 0.01) continue;
        const N = 12, pts = [];
        for (let i = 0; i <= N; i++) { const t = i / N; pts.push(proj(norm([A.v[0] + (B.v[0] - A.v[0]) * t, A.v[1] + (B.v[1] - A.v[1]) * t, A.v[2] + (B.v[2] - A.v[2]) * t]), L)); }
        // from the edge of one frame to the edge of the other
        let i0 = 0, i1 = N;
        while (i0 < N && Math.hypot(pts[i0 + 1][0] - A.x, pts[i0 + 1][1] - A.y) < A.r + 3) i0++;
        while (i1 > i0 + 1 && Math.hypot(pts[i1 - 1][0] - B.x, pts[i1 - 1][1] - B.y) < B.r + 3) i1--;
        const cut = (p, q, o, r) => { const d0 = Math.hypot(p[0] - o.x, p[1] - o.y), d1 = Math.hypot(q[0] - o.x, q[1] - o.y), t = d1 > d0 ? clamp((r + 3 - d0) / (d1 - d0), 0, 1) : 0; return [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]; };
        if (i1 <= i0) continue;
        const s0 = cut(pts[i0], pts[i0 + 1], A, A.r), s1 = cut(pts[i1], pts[i1 - 1], B, B.r);
        c.beginPath(); c.moveTo(s0[0], s0[1]);
        for (let i = i0 + 1; i < i1; i++) c.lineTo(pts[i][0], pts[i][1]);
        c.lineTo(s1[0], s1[1]);
        if (!col.light && lit) for (const [w, ga] of [[9, 0.09], [5.5, 0.16], [3.5, 0.28]]) { c.strokeStyle = css(col.accent, ga * a); c.lineWidth = w; c.stroke(); }
        c.strokeStyle = css(lit ? col.accent : rgb, a);
        c.lineWidth = lit ? 1.8 : e.role === "l1" ? 0.8 + 0.9 * (e.w ?? 0) : 0.8;
        c.stroke();
        // where a bond meets a frame: a small bead
        c.fillStyle = css(lit ? col.accent : rgb, Math.min(1, a * 1.5));
        c.beginPath(); c.arc(s1[0], s1[1], lit ? 2.2 : 1.5, 0, 7); c.fill();
      }
    }
    LAYER.globe[0].style.opacity = "1";
    const axesOn = near > 0.95 && S.zoom < 1.04 ? "1" : "0";   // enlarged: the limb is off the window
    // enlarged: the neighbours say more (all their labels, their names), sharper pictures; a button goes back
    const mag = S.zoom > 1.25, magBtn = $("magReset");
    // the words grow less than the globe does, so they keep clear of each other
    const tk = (1 / Math.pow(magOf(S.zoom), 0.6)).toFixed(2);
    if (S.tk !== tk) { S.tk = tk; stage.style.setProperty("--tk", tk); }
    if (stage.classList.contains("mag") !== mag) { stage.classList.toggle("mag", mag); if (mag) loadThumbs(); }
    if (magBtn.hidden !== S.zoomT <= 1.02) magBtn.hidden = S.zoomT <= 1.02;
    if (!magBtn.hidden) { const t = `${magOf(S.zoom).toFixed(1)}×`; if (magBtn.lastChild.textContent !== t) magBtn.lastChild.textContent = t; }
    if ($("axes").style.opacity !== axesOn) $("axes").style.opacity = axesOn;
    const areas = $("areas");
    const showAreas = S.zoom < 0.55;
    if (areas.hidden === showAreas) { areas.hidden = !showAreas; if (showAreas) paintTimeline(); }
    if (showAreas) areas.style.opacity = (1 - smoothstep(0.15, 0.55, S.zoom)).toFixed(3);
  }


  // ── hover: after a short dwell the node grows into a preview (its own shape, 512 picture);
  // the info card follows the hovered item. Other nodes do not move.
  const previewEl = $("preview");
  let hoverT = 0;
  nodesEl.addEventListener("pointerover", (e) => {
    const n = e.target.closest(".node"); if (!n || S.moving || S.swipe) return;
    clearTimeout(hoverT);
    hoverT = setTimeout(() => showPreview(n.dataset.id, n), 150);
  });
  nodesEl.addEventListener("pointerout", (e) => {
    const n = e.target.closest(".node"); if (!n || n.contains(e.relatedTarget) || previewEl.contains(e.relatedTarget)) return;
    clearTimeout(hoverT);
    hoverT = setTimeout(() => hidePreview(), 60);
  });
  // the preview is the way in: a click opens the file (a dataset opens in place), the folder button shows it in Finder
  previewEl.addEventListener("pointerenter", () => clearTimeout(hoverT));
  previewEl.addEventListener("pointerleave", () => { clearTimeout(hoverT); hoverT = setTimeout(() => hidePreview(), 80); });
  previewEl.addEventListener("click", (e) => {
    const id = S.hover; if (!id || e.detail > 1) return;
    if (e.target.closest(".pv-reveal")) { act("reveal", id); return; }
    if (S.items.get(id)?.sensitive && !previewEl.classList.contains("revealed")) { previewEl.classList.add("revealed"); return; }
    openItem(id);
  });
  async function showPreview(id, nodeEl) {
    if (S.tier !== 1 || S.moving || !S.items.has(id)) return;
    const it = S.items.get(id), L = layout();
    S.hover = id; updateCard(id);
    const r = nodeEl.querySelector(".orb").getBoundingClientRect();
    const cv = previewEl.querySelector("canvas");
    const bmp = await getThumb(id, thumbSize("preview", it));
    if (S.hover !== id) return;
    const aspect = bmp ? bmp.width / bmp.height : Scene.aspect(it.kind);
    const ph = Math.min(L.preview, (L.preview * 1.4) / aspect), pw = ph * aspect, dpr = dprOf();
    cv.width = Math.round(pw * dpr); cv.height = Math.round(ph * dpr); cv.style.width = `${pw}px`; cv.style.height = `${ph}px`;
    const g = cv.getContext("2d");
    if (bmp) { g.drawImage(bmp, 0, 0, cv.width, cv.height); } else Scene.paint(cv, it, { w: pw, h: ph, big: true });
    previewEl.classList.toggle("sens", !!it.sensitive); previewEl.classList.remove("revealed");
    const x = Math.max(8, Math.min(L.w - pw - 8, r.left + r.width / 2 - pw / 2)), y = Math.max(56, Math.min(L.h - ph - 8, r.top + r.height / 2 - ph / 2));
    previewEl.style.left = `${x}px`; previewEl.style.top = `${y}px`;
    previewEl.style.transformOrigin = `${r.left + r.width / 2 - x}px ${r.top + r.height / 2 - y}px`;
    previewEl.hidden = false;
    previewEl.classList.remove("open"); void previewEl.offsetWidth; previewEl.classList.add("open");
  }
  function hidePreview(now = false) {
    clearTimeout(hoverT);
    if (S.hover == null && previewEl.hidden) return;
    S.hover = null;
    previewEl.classList.remove("open");
    const cv = previewEl.querySelector("canvas");
    const done = () => { if (S.hover == null) { previewEl.hidden = true; cv.width = cv.height = 0; } };
    if (now) done(); else setTimeout(done, 160);
    if (S.focus) updateCard(S.focus);
  }

  const frames = Scene.frames(tick, { fps: () => prefs.fps });

  // ── compass: what each direction means, written outside the limb; the live reading at its angle ──
  const limbPoint = (L, th, pad) => [L.cx + Math.cos(th) * (L.rv + pad), L.cy - Math.sin(th) * (L.rv + pad)];
  // Neighbours that are near for the same reason sit together; each such group gets a callout outside
  // the limb saying what lies that way, concretely: a period, a word the names share, a kind, a source.
  // When the server sends `why` on a neighbour (tags, knowledge-card links), that is said instead.
  const tokens = (name) => stemOf(name).toLowerCase().split(/[^\p{L}\p{N}]+/u).map((t) => t.replace(/\d+$/, "")).filter((t) => t.length >= 2 && !/^\d+$/.test(t));
  const most = (list) => { const m = new Map(); for (const x of list) if (x != null && x !== "") m.set(x, (m.get(x) ?? 0) + 1); return [...m].sort((p, q) => q[1] - p[1] || String(q[0]).length - String(p[0]).length)[0] ?? [null, 0]; };
  // a period, as fine as it needs to be: days inside the focus's own month, else months, else years
  function period(ts, ft) {
    const of = (t) => { const d = new Date(t); return [d.getFullYear(), d.getMonth() + 1, d.getDate()]; };
    const a = of(Math.min(...ts)), b = of(Math.max(...ts)), f = of(ft);
    if (a[0] === b[0] && a[1] === b[1]) return a[0] === f[0] && a[1] === f[1] ? `${a[1]} 月 ${a[2]}${a[2] === b[2] ? "" : `–${b[2]}`} 日` : `${a[0]} 年 ${a[1]} 月`;
    if (a[0] === b[0]) return `${a[0]} 年 ${a[1]}–${b[1]} 月`;
    return `${a[0]}–${b[0]} 年`;
  }
  function describe(key, es, f) {
    const its = es.map((e) => S.items.get(e.id)).filter(Boolean);
    const why = most(es.flatMap((e) => e.why ?? []));
    if (why[0] && why[1] >= es.length / 2) return why[0];
    if (key === "time+" || key === "time-") { const ts = its.map((i) => i.time).filter(Boolean); return [key === "time+" ? "較新" : "較早", ts.length ? period(ts, f.time) : null].filter(Boolean).join(" · "); }
    if (key === "name") {
      const ft = new Set(tokens(f.name)), all = its.flatMap((i) => [...new Set(tokens(i.name))]);
      const shared = most(all.filter((t) => ft.has(t))), any = most(all);
      const w = shared[0] ?? (any[1] >= 2 ? any[0] : null);
      return w ? `名稱「${w}」` : "名稱相近";
    }
    if (key === "kind") return `同類型 · ${most(its.map((i) => KIND_LABEL[i.kind] ?? i.ext.toUpperCase()))[0] ?? ""}`;
    if (key === "host") { const hs = most(its.map((i) => i.host))[0]; return hs ? `同來源 · ${hs}` : "同來源"; }
    if (key === "folder") return "同資料夾";
    if (key === "mention") return "提及這份資料";
    return "";
  }
  function paintAxes(L) {
    const box = $("axes"); box.textContent = ""; S.callouts = [];
    const f = S.items.get(S.focus);
    if (!f || !S.nodes.has(S.focus) || L.narrow) return;
    const l1 = [...S.nodes.values()].filter((e) => e.role === "l1" && e.parts && e.region == null).sort((a, b) => a.ang - b.ang);
    const keyOf = (e) => { let bk = null, bv = -1; for (const [k, v] of Object.entries(e.parts)) if (k !== "label" && v > bv) { bv = v; bk = k; } return bk === "time" ? (e.newer ? "time+" : "time-") : bk; };
    let groups = [];
    for (const e of l1) { const k = keyOf(e), g = groups[groups.length - 1]; if (g && g.key === k) g.es.push(e); else groups.push({ key: k, es: [e] }); }
    if (groups.length > 1 && groups[0].key === groups[groups.length - 1].key) groups[0].es = [...groups.pop().es, ...groups[0].es];
    groups = groups.sort((a, b) => b.es.length - a.es.length).slice(0, 6);
    const taken = [$("filters"), $("card")].map((p) => p.getBoundingClientRect()).map((P) => [P.left - 6, P.top - 6, P.right + 6, P.bottom + 6]);
    const hits = (x, y, w, hh) => taken.some(([x0, y0, x1, y1]) => x < x1 && x + w > x0 && y < y1 && y + hh > y0);
    const echoBox = prefs.echo ? [L.w - ECHO_W - 12, L.h - clamp(L.h * 0.62, 420, 640) - 12] : null;   // where LoreEcho is seated (openEcho)
    if (echoBox) taken.push([echoBox[0], echoBox[1], L.w, L.h]);
    for (const g of groups) {
      const main = describe(g.key, g.es, f); if (!main) continue;
      const th = Math.atan2(g.es.reduce((a, e) => a + Math.sin(e.ang), 0), g.es.reduce((a, e) => a + Math.cos(e.ang), 0));
      const use = most(g.es.flatMap((e) => usesOf(S.items.get(e.id))));
      const sub = [`${g.es.length} 個`, use[1] >= g.es.length * 0.6 ? useLabel(use[0]) : null].filter(Boolean).join(" · ");
      const dir = Math.cos(th) > 0.3 ? 1 : Math.cos(th) < -0.3 ? -1 : 0, up = Math.sin(th) > 0;
      const el = h("div", { class: `ax ${dir > 0 ? "r" : dir < 0 ? "l" : "c"}` }, h("b", {}, main), h("span", {}, sub));
      box.append(el);
      const bw = el.offsetWidth, bh = el.offsetHeight;
      const p0 = limbPoint(L, th, 16), p1 = limbPoint(L, th, 44), p2 = [p1[0] + dir * 26, p1[1]];
      let x = dir > 0 ? p2[0] + 8 : dir < 0 ? p2[0] - 8 - bw : p1[0] - bw / 2, y = dir ? p2[1] - bh / 2 : up ? p1[1] - bh - 6 : p1[1] + 6;
      x = clamp(x, 8, L.w - 8 - bw); y = clamp(y, 54, L.h - 8 - bh);
      let line = [p0, p1, p2];
      // LoreEcho sits there: the words go above or below the end of the line instead of beside it
      if (echoBox && x + bw > echoBox[0] && y + bh > echoBox[1]) { x = clamp(p1[0] - bw / 2, 8, echoBox[0] - 8 - bw); y = up ? p1[1] - bh - 6 : p1[1] + 6; line = [p0, p1, p1]; el.className = "ax c"; }
      if (hits(x, y, bw, bh)) y += (up ? -1 : 1) * (bh + 6);
      if (hits(x, y, bw, bh) || y < 54 || y > L.h - 8 - bh) { el.remove(); continue; }
      taken.push([x - 4, y - 4, x + bw + 4, y + bh + 4]);
      el.style.transform = `translate(${x.toFixed(0)}px, ${y.toFixed(0)}px)`;
      S.callouts.push({ th, el, line });
    }
  }
  // θ is the side the content comes from (opposite to the drag): its callout lights up
  function showLive(th) {
    S.liveTh = th;
    stage.classList.toggle("swiping", th != null);
    let best = null, bc = Math.cos((55 * Math.PI) / 180);
    if (th != null) for (const q of S.callouts ?? []) { const c = Math.cos(q.th - th); if (c > bc) { bc = c; best = q; } }
    for (const q of S.callouts ?? []) q.el.classList.toggle("on", q === best);
    frames.invalidate();
  }
  // the node now nearest the pole (where the focus would land if motion stopped here) and its angle from it
  function nearestToCentre() {
    const M = turn(layout());
    let best = null, bz = -2;
    for (const [id, e] of S.nodes) { const z = mulV(M, e.v)[2]; if (z > bz) { bz = z; best = id; } }
    return [best, Math.acos(clamp(bz, -1, 1))];
  }
  function pickCandidate() {
    const [id] = nearestToCentre();
    markCandidate(id && id !== S.focus ? id : null);
    const d = Math.hypot(...S.off);
    showLive(d > 14 ? Math.atan2(S.off[1], -S.off[0]) : null);
  }
  let prefetchT = 0;
  function markCandidate(id) {
    if (S.candidate === id) return;
    S.nodes.get(S.candidate)?.el?.classList.remove("cand");
    S.candidate = id;
    if (!id) return;
    S.nodes.get(id)?.el?.classList.add("cand");
    // fetch ahead so landing is instant, but only once the candidate stays a moment: while the
    // globe sweeps past many nodes, a fetch + thumbnail per node made the drag stutter
    clearTimeout(prefetchT);
    prefetchT = setTimeout(() => {
      if (S.candidate !== id) return;
      neighbours(id).catch(() => {});
      const it = S.items.get(id);
      if (it && !it.sensitive) getThumb(id, 256);
    }, 120);
  }
  // motion has stopped: the node nearest the pole becomes the focus if it has clearly come over;
  // otherwise everything springs back
  function settle() {
    S.wheeling = false;
    const [id, ang] = nearestToCentre();
    showLive(null);
    if (id && id !== S.focus) {
      const from = Math.acos(clamp(S.nodes.get(id).v[2], -1, 1));
      if (ang < from * 0.55) { setFocus(id); return; }
    }
    markCandidate(null);
    frames.invalidate();
  }


  // ── focus ──
  async function setFocus(id, { quiet = false } = {}) {
    if (!S.items.has(id)) return;
    S.pending = id;
    let list;
    try { list = await neighbours(id); } catch (e) { LoreShell.toast(e.message, { err: true }); return; }
    if (S.pending !== id || S.tier >= 3) return;
    // the turn so far becomes part of where things are; the globe then turns the new focus to the pole
    const M = turn(layout());
    for (const e of [...S.nodes.values(), ...S.ghosts]) { e.v = mulV(M, e.v); if (e.aT === 0) e.t = e.v; }
    S.G = mulM(M, S.G);
    const nv = S.nodes.get(id)?.v;
    S.snap = nv && nv[2] < 0.9999 ? { axis: norm([nv[1], -nv[0], 0]), left: Math.acos(clamp(nv[2], -1, 1)) } : null;
    S.off = [0, 0]; S.coast = false; S.vel = [0, 0];
    markCandidate(null);
    S.focus = id;
    build(dedupe(list));
    if (!quiet) updateCard();
  }
  // arrow keys: the layer-1 node shown nearest that direction becomes the focus
  function hop(th) {
    let best = -Infinity, j = null;
    for (const [id, e] of S.nodes) {
      if (e.role !== "l1") continue;
      const c = Math.cos(Math.atan2(e.t[1], e.t[0]) - th);
      if (c > Math.cos((50 * Math.PI) / 180) && c > best) { best = c; j = id; }
    }
    if (!j) { frames.invalidate(); return false; }
    setFocus(j);
    return true;
  }

  // ── info card ──
  const fmt = (s) => { const d = ms(s); return Number.isNaN(d) ? null : new Date(d).toISOString().slice(0, 10); };
  async function updateCard(id = S.hover ?? S.focus) {
    const it = S.items.get(id);
    if (!it) return;
    $("cKind").textContent = KIND_LABEL[it.kind] ?? (it.ext || it.kind).toUpperCase();
    $("cTitle").textContent = it.name;
    $("cDesc").textContent = "";
    const rgOf = (l) => (S.regions ?? []).find((r) => r.key === lkey(l));
    $("cLabels").replaceChildren(...it.labels.map((l) => { const r = id === S.focus ? rgOf(l) : null; return h("span", { class: r ? `lab rg${r.ci}` : "lab" }, l); }));
    // equal dates share one line: 2026-10-05　下載・建立・文件日期
    const byDate = new Map();
    for (const [k, l] of TIME_LABEL) { const d = fmt(it.times[k]); if (!d) continue; if (!byDate.has(d)) byDate.set(d, []); byDate.get(d).push(l); }
    const rows = [...byDate].map(([d, ls]) => [ls.join("・"), d]);
    if (it.host) rows.push(["來源", it.host]);
    $("cTimes").replaceChildren(...rows.flatMap(([l, v]) => [h("dt", {}, l), h("dd", {}, v)]));
    const mem = membersOf(id);
    $("cMembers").replaceChildren(...(mem.length > 1 ? mem.slice(0, 2).map((m) => { const x = S.items.get(m); return h("li", {}, h("span", { class: "kind" }, KIND_LABEL[x.kind] ?? x.ext.toUpperCase()), h("span", { class: "mn" }, x.name)); }) : []));
    if (mem.length > 2) $("cMembers").append(h("li", { class: "more" }, `+${mem.length - 2}`));   // two rows keep the card short
    $("cMembers").hidden = mem.length < 2;
    paintUses(it);
    try {
      const d = await detail(id);
      if ((S.hover ?? S.focus) !== id) return;
      $("cDesc").textContent = d.description ?? "";
      const p = (d.paths ?? [])[0];
      if (p) { const parts = p.split("/").filter(Boolean); $("cTimes").append(h("dt", {}, "位置"), h("dd", { title: p }, parts.length > 1 ? `…/${parts[parts.length - 2]}` : "/")); }   // full path on hover
    } catch {}
  }
  async function act(kind, id = S.focus) {
    try { await api("POST", `/api/${kind}/${encodeURIComponent(id)}`); }
    catch (e) { LoreShell.toast(e.message, { err: true }); }
  }

  // ── zoomed out: time segments (GET /api/timeline, else bucketed here) ──
  let timeline = null;
  async function loadTimeline() {
    if (timeline) return timeline;
    const cat = catParam();
    try { if (S.demo || (cat && !S.catApi)) throw 0; const v = await api("GET", `/api/timeline${cat ? `?cat=${encodeURIComponent(cat)}` : ""}`); if (Array.isArray(v) && v.length) return (timeline = v); } catch {}
    return (timeline = bucket(filtering() ? S.list.filter(matches) : S.list));
  }
  // 本週 · 上週 · 本月 · 上個月 · this year's and last year's seasons · older years
  function bucket(list) {
    const now = new Date(), day = 86400000;
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    const week = today - ((now.getDay() + 6) % 7) * day, lastWeek = week - 7 * day;
    const month = new Date(now.getFullYear(), now.getMonth(), 1).getTime(), lastMonth = new Date(now.getFullYear(), now.getMonth() - 1, 1).getTime();
    const SEASON = ["冬", "冬", "春", "春", "春", "夏", "夏", "夏", "秋", "秋", "秋", "冬"];
    const segs = new Map();
    const add = (key, label, order, it) => { let g = segs.get(key); if (!g) segs.set(key, (g = { label, order, count: 0, latest: null, from: it.time, to: it.time })); g.count++; if (!g.latest || it.time > g.latest.time) g.latest = it; g.from = Math.min(g.from, it.time); g.to = Math.max(g.to, it.time); };
    for (const it of list) {
      const t = it.time; if (!t) continue;
      if (t >= week) add("w0", "本週", 1e13, it);
      else if (t >= lastWeek) add("w1", "上週", 1e13 - 1, it);
      else if (t >= month) add("m0", "本月", 1e13 - 2, it);
      else if (t >= lastMonth) add("m1", "上個月", 1e13 - 3, it);
      else {
        const d = new Date(t), y = d.getFullYear(), m = d.getMonth();
        const sy = m === 11 ? y : m < 2 ? y - 1 : y;   // 冬 = December and the next January–February
        if (y >= now.getFullYear() - 1) add(`s${sy}${SEASON[m]}`, `${sy} ${SEASON[m]}`, t, it); else add(`y${y}`, `${y}`, Date.UTC(y, 11, 31), it);
      }
    }
    // a season keeps its newest time as its order
    for (const g of segs.values()) if (g.order < 1e12) g.order = g.to;
    return [...segs.values()].sort((a, b) => b.order - a.order).map((g) => ({ label: g.label, from: new Date(g.from).toISOString(), to: new Date(g.to).toISOString(), count: g.count, latest_id: g.latest.id }));
  }
  async function paintTimeline() {
    const box = $("areas");
    const segs = await loadTimeline();
    box.textContent = "";
    const f = S.items.get(S.focus);
    const wrap = h("div", { class: "timeline" });
    for (const g of segs.slice(0, 12)) {   // the twelve most recent periods
      const here = f && ms(g.from) <= f.time && f.time <= ms(g.to);
      const b = h("button", { type: "button", class: "tblock" + (here ? " here" : "") }, h("span", { class: "tl" }, g.label), h("span", { class: "tc" }, `${g.count}`));
      b.onclick = () => { if (S.items.has(g.latest_id)) setFocus(g.latest_id); zoomTo(1); };
      wrap.append(b);
    }
    box.append(wrap);
  }

  // ── filter panel ──
  const PANEL_KEY = "lore.shelf.panel";
  let panelOpen = true;
  try { panelOpen = localStorage.getItem(PANEL_KEY) !== "closed"; } catch {}
  if (innerWidth <= 760) panelOpen = false;
  async function loadCategories() {
    const cat = catParam();
    let c = null;
    if (S.catApi) {
      try { c = await api("GET", `/api/categories${cat ? `?cat=${encodeURIComponent(cat)}` : ""}`); }
      catch { S.catApi = false; }
    }
    if (!c) {
      // without the server's categories: 類型 counted here; 用途 only if the items carry it
      const kind = new Map(), use = new Map();
      for (const it of S.list) for (const x of it.cats) (x in USE_LABEL ? use : kind).set(x, ((x in USE_LABEL ? use : kind).get(x) ?? 0) + 1);
      c = { kind: [...kind].sort((a, b) => b[1] - a[1]).map(([id, count]) => ({ id, label: KIND_LABEL[id] ?? id, count })), use: [...use].map(([id, count]) => ({ id, label: USE_LABEL[id] ?? id, count })) };
    }
    if (Array.isArray(c)) c = { kind: c.filter((x) => x.group === "type" && x.count), use: c.filter((x) => x.group === "use") };
    S.cats = c;
    for (const u of c.use ?? []) USE_LABEL[u.id] = u.label;
    paintFilters();
    if (S.focus) paintUses(S.items.get(S.focus));
  }
  function paintFilters() {
    const box = $("filters");
    box.classList.toggle("rail", !panelOpen);
    const n = S.cat.kind.size + S.cat.use.size;
    const toggle = h("button", { type: "button", class: "ico small", "aria-label": panelOpen ? "收起篩選" : "篩選", "aria-expanded": String(panelOpen), onclick: () => { panelOpen = !panelOpen; try { localStorage.setItem(PANEL_KEY, panelOpen ? "open" : "closed"); } catch {} paintFilters(); rebuild(); } }, icon("panelLeft"));
    if (!panelOpen) { box.replaceChildren(...[toggle, n ? h("span", { class: "f-count" }, `${n}`) : null].filter(Boolean)); return; }
    const head = h("div", { class: "f-head" }, toggle, h("span", { class: "f-title" }, "篩選"), n ? h("button", { type: "button", class: "ghost f-clear", onclick: () => setCat(null) }, "清除") : null);
    const group = (title, key, list) => {
      if (!list?.length) return null;
      return h("div", { class: "f-group" }, h("div", { class: "set-label" }, title), ...list.map((x) => {
        const cb = h("input", { type: "checkbox" }); cb.checked = S.cat[key].has(x.id);
        cb.onchange = () => setCat(key, x.id, cb.checked);
        return h("label", { class: "f-row" + (x.count ? "" : " zero") }, cb, h("span", { class: "f-name" }, x.label), h("span", { class: "f-n" }, `${x.count}`));
      }));
    };
    box.replaceChildren(...[head, group("類型", "kind", S.cats?.kind), group("用途", "use", S.cats?.use)].filter(Boolean));
  }
  async function setCat(key, id, on) {
    if (!key) { S.cat.kind.clear(); S.cat.use.clear(); }
    else if (on) S.cat[key].add(id); else S.cat[key].delete(id);
    timeline = null;
    loadCategories();
    if (!$("areas").hidden) paintTimeline();
    // the focus no longer matches: go to the nearest one that does
    let target = S.focus;
    if (filtering() && !matches(S.items.get(S.focus))) {
      target = null;
      if (S.catApi) { try { const r = await api("GET", `/api/nearest?from=${encodeURIComponent(S.focus)}&cat=${encodeURIComponent(catParam())}`); target = r.item?.id ?? r.id ?? null; } catch {} }
      if (!target) { const f = S.items.get(S.focus); let best = Infinity; for (const it of S.list) if (matches(it)) { const d = Math.abs(it.time - f.time); if (d < best) { best = d; target = it.id; } } }
      if (!target) { LoreShell.toast("沒有符合的檔案"); return; }
    }
    S.nbrs = new Map();
    setFocus(target, { quiet: target === S.focus });
  }
  // the panel sits above the info card in the left column
  // when the card grows (details arrive), the neighbours make room for it
  let cardH = 0, cardT = 0;
  new ResizeObserver(() => {
    const hh = $("card").offsetHeight;
    $("shelf").style.setProperty("--card-h", `${hh}px`);
    if (Math.abs(hh - cardH) > 16 && cardH && S.tier === 1 && !S.ds) { clearTimeout(cardT); cardT = setTimeout(() => rebuild(), 150); }
    cardH = hh;
  }).observe($("card"));

  // ── 用途 on the info card: click a chip to remove it, + to add one from the known list ──
  function paintUses(it) {
    const box = $("cUses");
    if (!it) return;
    const known = (S.cats?.use ?? []).map((u) => u.id);
    const mine = usesOf(it);
    const chips = mine.map((u) => h("button", { type: "button", class: "chip use", "aria-pressed": "true", title: "移除", onclick: () => putUses(it, mine.filter((x) => x !== u)) }, useLabel(u)));
    const rest = known.filter((u) => !mine.includes(u));
    // a new category: typed in place, Enter adds it
    const addNew = () => {
      const inp = h("input", { class: "use-new", type: "text", maxlength: "24", placeholder: "新類別", "aria-label": "新類別" });
      let sent = false;
      const done = (ok) => {
        if (sent) return; sent = true;
        const name = inp.value.trim();
        if (ok && name && !name.includes(",")) { const id = `u:${name}`; USE_LABEL[id] = name; putUses(it, [...mine.filter((x) => x !== id), id]); } else paintUses(it);
      };
      inp.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Enter") done(true); else if (e.key === "Escape") done(false); };
      inp.onblur = () => done(!!inp.value.trim());
      box.lastChild.replaceWith(inp); inp.focus();
    };
    const add = h("button", { type: "button", class: "ico tiny", "aria-label": "加用途", onclick: (e) => LoreShell.menu(e.currentTarget, [...rest.map((u) => ({ label: useLabel(u), run: () => putUses(it, [...mine, u]) })), ...(rest.length ? ["-"] : []), { icon: "plus", label: "新增類別…", run: addNew }]) }, icon("plus"));
    box.replaceChildren(...chips, add);
  }
  async function putUses(it, use) {
    try {
      const r = await api("PUT", `/api/files/${encodeURIComponent(it.id)}/categories`, { categories: use });
      const now = r.categories ?? use;
      it.cats = [...it.cats.filter((c) => !isUse(c)), ...now];
      it.sensitive = now.includes("id-doc");
      S.nbrs = new Map(); timeline = null;
      paintUses(it);
      loadCategories();
    } catch (e) { LoreShell.toast(e.message, { err: true }); }
  }

  // ── dataset view: opened in place; a lazily paged, virtualised thumbnail grid ──
  const PAGE = 60, CELL = 120, GAP = 10;
  // count, main type, label files and first-level folders come with /api/files/{id} (`dataset`)
  async function datasetInfo(id) {
    const it = S.items.get(id);
    if (it.dsInfo) return it.dsInfo;
    const d = (await detail(id)).dataset;
    if (!d) return null;
    it.count = d.files; it.mainExt = d.main_ext; it.dsInfo = d;
    return d;
  }
  async function openDataset(id) {
    const it = S.items.get(id); if (!it) return;
    let info = null;
    try { info = await datasetInfo(id); } catch {}
    S.ds = { id, folder: null, start: 0, total: it.count ?? 0, full: it.count ?? 0, pages: new Map(), folders: (info?.subfolders ?? []).map(([name, count]) => ({ name, count })), labels: info?.meta_files ?? [], cells: new Map() };
    $("dsview").hidden = false; $("card").hidden = true;
    stage.classList.add("ds-open");
    try { await dsPage(0); D_total_fix(); } catch (e) { LoreShell.toast(e.message, { err: true }); }
    paintDsHead();
    renderGrid();
    $("dsGrid").focus({ preventScroll: true });
  }
  function closeDataset() {
    if (!S.ds) return;
    dropGrid();
    for (const k of [...S.thumbs.keys()]) if (k.startsWith("ds:")) dropThumb(k);
    S.ds = null;
    $("dsview").hidden = true; $("card").hidden = false;
    stage.classList.remove("ds-open");
    frames.invalidate();
  }
  // pages are over the whole (path-sorted) list; a folder is a contiguous range of it
  function dsPage(n) {
    const D = S.ds; if (!D) return Promise.resolve(null);
    const key = `${n}`;
    if (!D.pages.has(key)) {
      const p = api("GET", `/api/datasets/${encodeURIComponent(D.id)}/items?offset=${n * PAGE}&limit=${PAGE}`).then((r) => {
        if (r.total != null) { D.full = r.total; if (!D.folder) D.total = r.total; }
        return r.items ?? [];
      });
      D.pages.set(key, p);
      p.catch(() => D.pages.delete(key));
    }
    return D.pages.get(key);
  }
  function D_total_fix() { const D = S.ds; if (D && !D.folder) D.total = D.full; }
  // first index whose path is ≥ key (binary search with one-item requests)
  async function lowerBound(key) {
    const D = S.ds; let lo = 0, hi = D.full;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      const r = await api("GET", `/api/datasets/${encodeURIComponent(D.id)}/items?offset=${mid}&limit=1`);
      const rel = r.items?.[0]?.rel ?? r.items?.[0]?.name ?? "";
      if (rel < key) lo = mid + 1; else hi = mid;
    }
    return lo;
  }
  async function showFolder(name) {
    const D = S.ds;
    D.folder = D.folder === name ? null : name;
    if (D.folder) { const a = await lowerBound(`${name}/`), b = await lowerBound(`${name}/\uffff`); if (S.ds !== D) return; D.start = a; D.total = b - a; }
    else { D.start = 0; D.total = D.full; }
    dropGrid(); $("dsGrid").scrollTop = 0;
    paintDsHead(); renderGrid();
  }
  function paintDsHead() {
    const D = S.ds, it = S.items.get(D.id);
    const bytes = it.dsInfo?.bytes ?? it.size;
    const size = bytes ? (bytes >= 1073741824 ? `${(bytes / 1073741824).toFixed(1)} GB` : `${Math.round(bytes / 1048576)} MB`) : null;
    const unit = IMG_EXT.has(it.mainExt) ? "張" : "個";
    const chips = D.folders.length > 1 ? D.folders.map((f) => h("button", { type: "button", class: "chip", "aria-pressed": String(D.folder === f.name), onclick: () => showFolder(f.name) }, f.name, h("span", { class: "f-n" }, `${f.count}`))) : [];
    $("dsHead").replaceChildren(
      h("button", { type: "button", class: "ico", "aria-label": "返回 Esc", onclick: closeDataset }, icon("back")),
      h("div", { class: "ds-title" }, h("h1", {}, it.name), h("span", { class: "ds-meta" }, [`${(it.count ?? D.full).toLocaleString()} ${unit}`, size, it.mainExt?.toUpperCase()].filter(Boolean).join(" · "))),
      h("div", { class: "ds-chips" }, ...chips),
      D.labels.length ? h("div", { class: "ds-labels" }, ...D.labels.map((l) => h("span", { class: "kind" }, l))) : "",
      h("span", { class: "grow" }),
      h("button", { type: "button", class: "ghost", onclick: () => act("reveal", D.id) }, icon("folder"), "在 Finder 顯示"));
  }
  // only the rows on screen (±1) have cells; cells are reused as they scroll out
  function renderGrid() {
    const D = S.ds; if (!D || S.tier >= 3) return;
    const grid = $("dsGrid"), inner = $("dsInner");
    const W = grid.clientWidth - 2 * GAP, cols = Math.max(1, Math.floor((W + GAP) / (CELL + GAP)));
    const rows = Math.ceil(D.total / cols);
    inner.style.height = `${rows * (CELL + GAP) + GAP}px`;
    const r0 = Math.max(0, Math.floor(grid.scrollTop / (CELL + GAP)) - 1), r1 = Math.min(rows, Math.ceil((grid.scrollTop + grid.clientHeight) / (CELL + GAP)) + 1);
    const want = new Set();
    for (let k = r0 * cols; k < Math.min(D.total, r1 * cols); k++) want.add(k);
    const free = [];
    for (const [k, c] of D.cells) if (!want.has(k)) { D.cells.delete(k); free.push(c); }
    const left = (W - (cols * (CELL + GAP) - GAP)) / 2 + GAP;
    for (const k of want) {
      if (D.cells.has(k)) continue;
      let c = free.pop();
      if (!c) { c = h("div", { class: "dcell" }, h("canvas"), h("span", { class: "dn" })); inner.append(c); }
      c.style.transform = `translate(${left + (k % cols) * (CELL + GAP)}px, ${GAP + Math.floor(k / cols) * (CELL + GAP)}px)`;
      c.dataset.k = k; c.lastChild.textContent = "";
      c.firstChild.width = c.firstChild.height = 0;
      D.cells.set(k, c);
      fillCell(c, k);
    }
    for (const c of free) { c.firstChild.width = c.firstChild.height = 0; c.remove(); }
  }
  async function fillCell(c, k) {
    const D = S.ds, abs = D.start + k;
    let page; try { page = await dsPage(Math.floor(abs / PAGE)); } catch { return; }
    if (!page || S.ds !== D || c.dataset.k !== String(k)) return;
    const item = page[abs % PAGE]; if (!item) return;
    c.lastChild.textContent = item.name.split("/").pop();
    c.title = item.rel ?? item.name;
    const cv = c.firstChild, dpr = Math.min(2, devicePixelRatio || 1);
    const bmp = await getThumb(`ds:${D.id}:${item.index}`, 128, `/api/thumb/ds/${encodeURIComponent(D.id)}/${item.index}?s=128`);
    if (S.ds !== D || c.dataset.k !== String(k)) return;
    const side = CELL - 24;
    if (bmp) {
      cv.width = cv.height = Math.round(side * dpr);
      const g = cv.getContext("2d"), s2 = Math.max(cv.width / bmp.width, cv.height / bmp.height);
      g.drawImage(bmp, (cv.width - bmp.width * s2) / 2, (cv.height - bmp.height * s2) / 2, bmp.width * s2, bmp.height * s2);
    } else {
      const it = S.items.get(D.id);
      Scene.paint(cv, { id: `${D.id}:${item.index}`, kind: IMG_EXT.has(it.mainExt) ? "image" : it.mainExt === "wav" ? "audio" : "text", name: item.name, ext: it.mainExt }, { w: side, h: side });
    }
    cv.style.width = cv.style.height = `${side}px`;
  }
  function dropGrid() {
    if (!S.ds) return;
    for (const c of S.ds.cells.values()) { c.firstChild.width = c.firstChild.height = 0; c.remove(); }
    S.ds.cells = new Map();
  }
  let gridRaf = 0;
  $("dsGrid").addEventListener("scroll", () => { if (!gridRaf) gridRaf = requestAnimationFrame(() => { gridRaf = 0; renderGrid(); }); }, { passive: true });

  // ── input ──
  const near = () => S.zoomT >= 0.5;
  const pointers = new Map();
  let pinch = null;
  function anyInput() { if (S.tier >= 2 && S.open) setActivity("active"); }
  stage.addEventListener("pointerdown", (e) => {
    anyInput();
    if (e.target.closest(".tblock, .preview")) return;
    stage.setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
    if (pointers.size === 2) { const [a, b] = [...pointers.values()]; pinch = { d: Math.hypot(a[0] - b[0], a[1] - b[1]), z: S.zoomT }; S.swipe = null; return; }
    S.coast = false; S.vel = [0, 0];
    S.swipe = { x0: e.clientX, y0: e.clientY, x: e.clientX, y: e.clientY, t: performance.now(), moved: false, target: e.target.closest(".node")?.dataset.id ?? null };
  });
  // the whole neighbourhood follows the finger 1:1
  stage.addEventListener("pointermove", (e) => {
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, [e.clientX, e.clientY]);
    if (pinch && pointers.size === 2) { const [a, b] = [...pointers.values()]; zoomTo(pinch.z + Math.log(Math.hypot(a[0] - b[0], a[1] - b[1]) / pinch.d)); return; }
    const sw = S.swipe; if (!sw) return;
    const now = performance.now(), dt = Math.max(1, now - sw.t), mx = e.clientX - sw.x, my = e.clientY - sw.y;
    sw.x = e.clientX; sw.y = e.clientY; sw.t = now;
    if (!sw.moved && Math.hypot(sw.x - sw.x0, sw.y - sw.y0) > 6) { sw.moved = true; stage.classList.add("dragging"); mx && (S.off[0] += sw.x - sw.x0 - mx); my && (S.off[1] += sw.y - sw.y0 - my); }
    if (!sw.moved || !near()) return;
    S.off[0] += mx; S.off[1] += my;
    S.vel = [0.6 * (mx / dt) + 0.4 * S.vel[0], 0.6 * (my / dt) + 0.4 * S.vel[1]];
    pickCandidate();
    frames.invalidate();
  });
  const endPointer = (e) => {
    pointers.delete(e.pointerId);
    if (pinch) { if (pointers.size < 2) pinch = null; return; }
    const sw = S.swipe; S.swipe = null;
    stage.classList.remove("dragging");
    if (!sw) return;
    if (!sw.moved) {
      if (e.type !== "pointerup") return;
      const id = sw.target;
      if (!id) return;
      if (id === S.focus) { S.nodes.get(S.focus)?.el.querySelector(".orb.sens")?.classList.toggle("revealed"); return; }
      setFocus(id); return;
    }
    if (!near()) return;
    // a pause before letting go means no throw
    if (performance.now() - sw.t > 80) S.vel = [0, 0];
    if (Math.hypot(...S.vel) > 0.06) S.coast = true; else settle();
    frames.invalidate();
  };
  stage.addEventListener("pointerup", endPointer);
  stage.addEventListener("pointercancel", endPointer);
  stage.addEventListener("dblclick", (e) => { const id = e.target.closest(".node")?.dataset.id; if (id) openItem(id); });
  // a dataset opens in place (its files never spread into the graph); anything else in its app
  const openItem = (id) => (S.items.get(id)?.kind === "dataset" ? openDataset(id) : act("open", id));

  // trackpad: two-finger scroll moves the scene 1:1, momentum included; a 100 ms gap = the end.
  // two fingers apart on the trackpad (WebKit's gesture events; elsewhere it arrives as ctrl+wheel)
  let gz = null;
  stage.addEventListener("gesturestart", (e) => { e.preventDefault(); gz = S.zoomT; });
  stage.addEventListener("gesturechange", (e) => { e.preventDefault(); if (gz != null) zoomTo(gz + Math.log(e.scale) * 0.9); });
  stage.addEventListener("gestureend", (e) => { e.preventDefault(); gz = null; });
  $("magReset").addEventListener("click", () => zoomTo(1));
  // pinch / ctrl+wheel = zoom
  let lastWheel = 0;
  stage.addEventListener("wheel", (e) => {
    e.preventDefault();
    anyInput();
    const lineMode = e.deltaMode === 1 || (e.deltaX === 0 && Math.abs(e.deltaY) >= 100 && Number.isInteger(e.deltaY));
    if (e.ctrlKey || lineMode || !near()) { zoomTo(S.zoomT - e.deltaY * (e.ctrlKey ? 0.012 : 0.0025) * (e.deltaMode === 1 ? 33 : 1)); return; }
    S.coast = false; S.wheeling = true;
    S.off[0] -= e.deltaX; S.off[1] -= e.deltaY;
    lastWheel = performance.now();
    pickCandidate();
    clearTimeout(S.timers.wheel);
    S.timers.wheel = setTimeout(settle, 100);
    frames.invalidate();
  }, { passive: false });
  function zoomTo(z) { S.zoomT = Math.max(0, Math.min(2, z)); frames.invalidate(); }

  document.addEventListener("keydown", (e) => {
    if (e.target.closest?.("input, textarea, dialog") || e.metaKey || e.ctrlKey || e.altKey) return;
    if (S.ds && e.key !== "Escape") return;
    anyInput();
    const dirs = { ArrowRight: 0, ArrowUp: 90, ArrowLeft: 180, ArrowDown: 270 };
    if (e.key in dirs && S.open && near()) { e.preventDefault(); hop((dirs[e.key] * Math.PI) / 180); }
    else if (e.key === "/") { e.preventDefault(); qEl.focus(); qEl.select(); }
    else if (e.key === "0") zoomTo(0);
    else if (e.key === "=" || e.key === "+") zoomTo(S.zoomT < 1 ? 1 : S.zoomT + 0.25);
    else if (e.key === "-") zoomTo(S.zoomT > 1 ? Math.max(1, S.zoomT - 0.25) : 0);
    else if (e.key === "Enter" && S.open && !S.ds && !e.target.closest("button")) openItem(S.focus);
    else if (e.key === "Escape" && S.ds) closeDataset();
    else if (e.key === "Escape" && S.open) close();
  });

  // ── resource tiers ──
  //  ① active · ② blurred: nothing animates or draws · ③ hidden > freeAfterMs: every picture but the
  //  focus's dropped, neighbour and detail caches cleared · ④ hidden > releaseAfterMs: the whole scene
  //  removed (only the focus id and zoom kept)
  function snapAll() {
    S.zoom = S.zoomT; S.off = [0, 0]; S.swipe = null; S.coast = false; S.wheeling = false; markCandidate(null); showLive(null);
    for (const e of S.nodes.values()) { e.v = e.t; e.k = e.kT; e.a = e.aT; }
    for (const g of S.ghosts) dropEntry(g);
    S.ghosts = []; S.snap = null;
  }
  function freeTier3() {
    S.tier = 3;
    for (const [id, e] of S.nodes) if (id !== S.focus) { dropEntry(e); S.nodes.delete(id); }
    hidePreview(true);
    S.far = [];
    $("axes").textContent = ""; $("areas").textContent = ""; timeline = null;
    S.nbrs = new Map(); S.detail = new Map();
    for (const k of [...S.thumbs.keys()]) if (!k.startsWith(`${S.focus}@`)) dropThumb(k);
    if (S.ds) dropGrid();
  }
  function releaseTier4() {
    if (S.tier < 3) freeTier3();
    S.tier = 4;
    for (const e of S.nodes.values()) dropEntry(e);
    S.nodes = new Map();
    for (const k of [...S.thumbs.keys()]) dropThumb(k);
    freeLayers();   // the layer canvases go too
  }
  async function restore() {
    S.restoreT0 = performance.now(); S.restoreMs = null;
    S.backKey = null;
    if (S.ds) renderGrid();
    // the focus first (one frame), then its neighbours
    if (!S.nodes.has(S.focus)) build([], { fresh: true });
    frames.invalidate();
    try { const list = await neighbours(S.focus); if (S.tier < 3) build(dedupe(list)); } catch {}
  }
  function setActivity(state) {
    clearTimeout(S.timers.t3); clearTimeout(S.timers.t4);
    if (state === "active") {
      if (!S.open) return;
      if (S.wasHidden && prefs.echo) openEcho();
      S.wasHidden = false;
      S.pauseWhenSettled = false;
      const was = S.tier;
      S.tier = 1;
      frames.pause(false);
      if (was >= 3) restore(); else frames.invalidate();
      return;
    }
    if (state === "blurred" && S.tier >= 3 && S.open) { setActivity("active"); S.pauseWhenSettled = true; return; }
    hidePreview(true);
    if (S.tier === 1) { snapAll(); tick(0); }
    frames.pause(true);
    if (S.tier < 2) S.tier = 2;
    if (state === "blurred") return;
    S.wasHidden = true;
    if (S.tier < 3) S.timers.t3 = setTimeout(freeTier3, CFG.freeAfterMs);
    if (S.tier < 4) S.timers.t4 = setTimeout(releaseTier4, CFG.releaseAfterMs);
  }
  document.addEventListener("visibilitychange", () => setActivity(document.hidden ? "hidden" : document.hasFocus() ? "active" : "blurred"));
  addEventListener("blur", () => { if (!document.hidden) setActivity("blurred"); });
  addEventListener("focus", () => setActivity("active"));

  const shelfEl = $("shelf");
  // Esc in the graph: release the scene at once; the desktop app hides its window (and calls
  // setActivity("active") when it shows it again), the browser shows a reopen button
  function close() {
    setActivity("hidden"); clearTimeout(S.timers.t3); clearTimeout(S.timers.t4);
    releaseTier4();
    if (tauri) { tauri.core?.invoke?.("hide")?.catch?.(() => {}); return; }
    S.open = false; shelfEl.classList.add("closed"); $("reopen").hidden = false;
  }
  function open() { if (S.open) return; S.open = true; shelfEl.classList.remove("closed"); $("reopen").hidden = true; setActivity("active"); }
  $("reopen").onclick = open;

  // ── shell: brand (zoom out to areas) and the gear ──
  function segRow(label, key, opts, after) {
    const seg = h("div", { class: "seg", role: "group", "aria-label": label });
    for (const [v, l] of opts) {
      const b = h("button", { type: "button", "aria-pressed": String(prefs[key] === v) }, l);
      b.onclick = () => { prefs[key] = v; savePrefs(); seg.querySelectorAll("button").forEach((x, i) => x.setAttribute("aria-pressed", String(opts[i][0] === v))); after?.(); };
      seg.append(b);
    }
    return h("div", { class: "set-row" }, h("span", { class: "k" }, label), seg);
  }
  function checkRow(label, key, after) {
    const i = h("input", { type: "checkbox" }); i.checked = !!prefs[key];
    i.onchange = () => { prefs[key] = i.checked; savePrefs(); after?.(); };
    return h("label", { class: "set-check" }, i, label);
  }
  const rebuild = () => { if (S.tier === 1 && S.focus) setFocus(S.focus, { quiet: true }); };
  function buildGeneral(el) {
    const sec = (title, ...rows) => el.append(h("div", { class: "set-sec" }, h("div", { class: "set-label" }, title), ...rows));
    sec("鄰居", segRow("數量", "k", [[8, "8"], [12, "12"], [16, "16"]], rebuild), checkRow("第二層", "layer2", rebuild));
    sec("畫面", segRow("最高幀率", "fps", [[60, "60"], [30, "30"]]));
    sec("LoreEcho", checkRow("開啟時同步開啟 LoreEcho", "echo", () => { if (prefs.echo) openEcho(); rebuild(); }));
  }
  const shell = LoreShell.mount({
    app: "ls", brand: "LORE/SHELF", host: "#top",
    home: () => zoomTo(S.zoomT < 0.5 ? 1 : 0),
    general: buildGeneral,
    keys: [{ label: "搜尋", keys: "/" }, { label: "往該方向移動", keys: "← ↑ → ↓" }, { label: "放大／縮小", keys: "+ −" }, { label: "開啟", keys: "↵" }, { label: "收起", keys: "Esc" }],
  });
  document.querySelector("#top .brand").setAttribute("aria-label", "時間 0");

  // LoreEcho alongside (設定 → LoreEcho): brought up when the shelf opens or comes back
  // (in the desktop app) seated in this window's bottom-right corner, at its narrowest
  async function openEcho() {
    if (S.demo) return;
    let seat = {};
    try {
      const f = await tauri?.core?.invoke?.("frame");   // [x, y, w, h] of this window, in screen points
      if (f?.[2] > 0) seat = { right: f[0] + f[2] - 12, bottom: f[1] + f[3] - 12, height: Math.round(clamp(f[3] * 0.62, 420, 640)) };
    } catch {}
    api("POST", "/api/echo/open", seat).catch((e) => LoreShell.toast(`LoreEcho：${e.message}`, { err: true }));
  }
  // another local app (LoreEcho) asks to see a file: the globe turns to it
  // (the window is usually not the front one then: it draws until the globe has settled, then rests again)
  function goTo(id) {
    if (!S.items.has(id)) return;
    if (S.tier >= 3) { S.focus = id; return; }
    if (S.tier === 2) { S.tier = 1; frames.pause(false); S.pauseWhenSettled = true; }
    zoomTo(1); setFocus(id);
  }
  async function listen() {
    let seq = null;
    for (;;) {
      try {
        const r = await (await fetch(`/api/events${seq == null ? "" : `?after=${seq}`}`, { cache: "no-store" })).json();
        if (seq != null && r.seq > seq && r.act?.focus) goTo(r.act.focus);
        if (typeof r.seq !== "number") throw 0;
        seq = r.seq;
      } catch { await new Promise((ok) => setTimeout(ok, 5000)); }
    }
  }

  // ── guided search: words need not be in a file name (first pages' text, categories, sources,
  // folders, years all count); the chips say how to go on ──
  const VIA = { text: "內文", category: "類別", host: "來源", folder: "資料夾", time: "日期" };
  const find = { q: "", cats: new Set(), res: null, sel: 0, seq: 0, open: false, t: 0 };
  const qEl = h("input", { id: "q", type: "search", placeholder: "搜尋名稱、內文、類別、年份…", autocomplete: "off", spellcheck: "false", "aria-label": "搜尋 /" });
  const searchEl = h("label", { class: "search" }, h("span", { "data-icon": "search" }), qEl);
  const findEl = h("div", { class: "find", hidden: true });
  shell.slots.lead.append(searchEl);
  shelfEl.append(findEl);
  const catLabel = (id) => [...(S.cats?.kind ?? []), ...(S.cats?.use ?? [])].find((c) => c.id === id)?.label ?? useLabel(id);
  async function runFind() {
    const seq = ++find.seq, cat = [...find.cats].join(",");
    let r;
    try { r = await api("GET", `/api/search?q=${encodeURIComponent(find.q)}&limit=24${cat ? `&cat=${encodeURIComponent(cat)}` : ""}`); if (!r?.items) throw 0; }
    catch { const t = find.q.toLowerCase().split(/\s+/).filter(Boolean), m = t.length ? S.list.filter((it) => t.every((w) => it.name.toLowerCase().includes(w))) : []; r = { total: m.length, items: m.slice(0, 24).map((it) => ({ item: it, via: ["name"], snippet: null })), facets: [] }; }
    if (seq !== find.seq) return;
    find.res = r; find.sel = 0;
    paintFind();
  }
  function paintFind() {
    const r = find.res;
    if (!r || !find.open) { findEl.hidden = true; return; }
    const narrow = (f) => { if (f.kind === "cat") find.cats.add(f.value); else { find.q = `${find.q} ${f.value}`.trim(); qEl.value = find.q; } runFind(); qEl.focus(); };
    const chosen = [...find.cats].map((id) => h("button", { type: "button", class: "chip on", "aria-pressed": "true", onclick: () => { find.cats.delete(id); runFind(); qEl.focus(); } }, catLabel(id), icon("x")));
    const chips = (r.facets ?? []).map((f) => h("button", { type: "button", class: "chip", onclick: () => narrow(f) }, f.label, h("span", { class: "f-n" }, `${f.count}`)));
    const rows = (r.items ?? []).map((x, i) => h("button", { type: "button", class: "frow" + (i === find.sel ? " sel" : ""), onclick: () => goFind(x.item.id) },
      h("span", { class: "kind" }, KIND_LABEL[x.item.kind] ?? (x.item.ext || "").toUpperCase()),
      h("span", { class: "fmain" }, h("span", { class: "fname" }, x.item.name), x.snippet ? h("span", { class: "fsnip" }, x.snippet) : null),
      h("span", { class: "fvia" }, [x.at, ...(x.via ?? []).map((v) => VIA[v])].filter(Boolean).join("・"))));
    const asked = find.q.trim() || find.cats.size;
    findEl.replaceChildren(...[
      chosen.length + chips.length ? h("div", { class: "fchips" }, ...chosen, ...chips) : null,
      rows.length ? h("div", { class: "frows" }, ...rows) : null,
      asked ? h("div", { class: "fcount" }, r.total ? (r.total > rows.length ? `${rows.length} / ${r.total}` : `${r.total}`) : "沒有符合的檔案") : null,
    ].filter(Boolean));
    const b = searchEl.getBoundingClientRect();
    findEl.style.left = `${Math.max(12, b.left)}px`; findEl.style.top = `${b.bottom + 8}px`;
    findEl.hidden = false;
    findEl.querySelector(".frow.sel")?.scrollIntoView({ block: "nearest" });
  }
  function closeFind() { find.open = false; findEl.hidden = true; }
  function goFind(id) { closeFind(); qEl.blur(); if (S.items.has(id)) { zoomTo(1); setFocus(id); } }
  qEl.addEventListener("focus", () => { find.open = true; anyInput(); runFind(); });
  qEl.addEventListener("input", () => { find.q = qEl.value; find.open = true; clearTimeout(find.t); find.t = setTimeout(runFind, 120); });
  qEl.addEventListener("compositionend", () => { find.q = qEl.value; runFind(); });
  qEl.addEventListener("keydown", (e) => {
    if (e.isComposing || e.keyCode === 229) return;   // Enter that confirms an input-method candidate is not ours
    const n = find.res?.items?.length ?? 0;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); if (n) { find.sel = (find.sel + (e.key === "ArrowDown" ? 1 : n - 1)) % n; paintFind(); } }
    else if (e.key === "Enter") { const x = find.res?.items?.[find.sel]; if (x) goFind(x.item.id); }
    else if (e.key === "Escape") { e.stopPropagation(); closeFind(); qEl.blur(); }
  });
  document.addEventListener("pointerdown", (e) => { if (find.open && !findEl.contains(e.target) && !searchEl.contains(e.target)) closeFind(); }, true);
  paintIcons();
  $("echoSpot").hidden = !CFG.demo;

  document.addEventListener("lore-theme", readTheme);
  readTheme();
  let resizeT = 0;
  addEventListener("resize", () => { clearTimeout(resizeT); resizeT = setTimeout(() => { if (S.ds) renderGrid(); if (S.tier === 1 && S.focus) { rebuild(); if (!$("areas").hidden) paintTimeline(); } }, 120); });

  // where to start: the server's choice when it has one, else the most recently opened document,
  // else the newest PDF, else the newest item (never a picture when anything else exists)
  async function defaultFocus() {
    try { if (!S.demo) { const cat = catParam(); const r = await api("GET", `/api/default-focus${cat ? `?cat=${encodeURIComponent(cat)}` : ""}`); if (S.items.has(r.id)) return r.id; } } catch {}
    const DOC = new Set(["pdf", "document", "slides", "spreadsheet"]);
    const newest = (l, k = (x) => x.time) => l.reduce((a, b) => (k(b) > k(a) ? b : a), l[0]);
    const opened = S.list.filter((x) => DOC.has(TYPE_OF[x.kind]) && ms(x.times.opened) > 0);
    if (opened.length) return newest(opened, (x) => ms(x.times.opened)).id;
    const pdfs = S.list.filter((x) => x.kind === "pdf");
    if (pdfs.length) return newest(pdfs).id;
    const notImg = S.list.filter((x) => x.kind !== "image");
    return newest(notImg.length ? notImg : S.list).id;
  }

  // ── boot ──
  (async () => {
    let raw;
    S.demo = CFG.demo;   // ?demo: always the synthetic catalog
    try { raw = await api("GET", "/api/files"); if (!Array.isArray(raw)) raw = raw.files ?? []; }
    catch { S.demo = true; raw = await ShelfDemo.api("GET", "/api/files"); }
    S.list = raw.map(normalize);
    S.catApi = true;   // until /api/categories says otherwise
    for (const it of S.list) { S.items.set(it.id, it); it.gkey = groupKey(it); let g = S.groups.get(it.gkey); if (!g) S.groups.set(it.gkey, (g = [])); g.push(it.id); }
    if (!S.list.length) { LoreShell.toast("目錄是空的：先用 loreshelf scan 掃描資料夾"); return; }
    const start = q.get("focus") && S.items.has(q.get("focus")) ? q.get("focus") : await defaultFocus();
    S.focus = start;
    build([], { fresh: true });
    updateCard();
    await loadCategories();
    setFocus(start, { quiet: true });
    if (prefs.echo) openEcho();
    if (!S.demo) listen();
  })();

  Object.assign(window.LoreShelf, {
    setActivity, open, close,
    hop: (deg) => hop((deg * Math.PI) / 180),
    openItem: (id) => openItem(id ?? S.focus), setCat: (g, id, on = true) => setCat(g, id, on),
    zoom: zoomTo,
    // for tests: move the scene by (dx, dy) as a drag would, then let go
    // for the layer breakdown: show only some of L0–L5 (null = all)
    layers: (only) => {
      const parts = { L0: ["#lback"], L1: ["#lglobe"], L2: ["#lbond", "#axes"], L3: [".node.l2"], L4: [".node.l1"], L5: [".node.focus", "#card", "#filters", "#top"] };
      for (const [k, sels] of Object.entries(parts)) for (const sel of sels) for (const el of document.querySelectorAll(sel)) el.style.visibility = !only || only.includes(k) ? "" : "hidden";
      if (!only) frames.invalidate();
    },
    layerMB: () => layerBytes(),
    drag: (dx, dy) => { S.coast = false; S.off[0] += dx; S.off[1] += dy; pickCandidate(); frames.invalidate(); }, settle,
    stats: () => {
      const cvs = [...document.querySelectorAll("#nodes canvas")];
      return {
        tier: S.tier, focus: S.focus, zoom: +S.zoomT.toFixed(2), demo: S.demo, items: S.list.length,
        nodes: S.nodes.size, domNodes: document.getElementsByTagName("*").length, canvases: cvs.length,
        canvasMB: +(cvs.reduce((s, c) => s + c.width * c.height * 4, 0) / 1048576).toFixed(2),
        cached: S.nbrs.size + S.detail.size, thumbs: S.thumbs.size, thumbMB: +(S.thumbBytes / 1048576).toFixed(2), frames: S.frames, animating: frames.running, restoreMs: S.restoreMs,
        layerMB: layerBytes(),
        heapMB: performance.memory ? +(performance.memory.usedJSHeapSize / 1048576).toFixed(2) : null,
      };
    },
    CFG,
  });
})();
