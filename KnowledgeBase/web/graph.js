// Knowledge graph (Obsidian-style) on a plain 2D canvas, no library, no build step.
//
// • Global graph (every card) and local graph (a card and its neighbours, depth 1–3).
// • Force simulation: Barnes–Hut quadtree repulsion, link springs, centering, d3-style
//   alpha cooling; several ticks per frame within a time budget; stops when settled,
//   when the tab or the view is hidden.
// • Nodes coloured by familiarity (or domain / tag group / kind / space), sized by
//   degree, shaped by kind; placeholders for unresolved [[links]]; optional tag nodes.
// • Edges styled by link type (prerequisite arrows, contrast dashed, mentions thin…).
// • Hover highlights neighbours, click selects (side card), drag pins, double-click
//   unpins, wheel / pinch zoom, pan, fit, search, filters, physics sliders (saved in
//   localStorage), timeline playback by created_at.
//
// app.js calls Graph.init(wrap, opts) once, Graph.show(centerId|null) when the view
// opens, Graph.hide() when it closes and Graph.draw() after a theme change.

const Graph = (() => {
  // ── Constants ──────────────────────────────────────────
  const STATES = [
    ["learning", "學習中", "--warn"],
    ["known", "記得", "--ok"],
    ["stale", "久未複習", "--stale"],
    ["none", "尚無紀錄", "--faint"],
  ];
  const STATE_LABEL = Object.fromEntries(STATES.map(([k, l]) => [k, l]));
  const SHAPE = { concept: "circle", term: "circle", vocab: "circle", method: "square", equation: "diamond", metric: "diamond", dataset: "triangle", anatomy: "hexagon", paper: "pentagon", person: "hexagon", tag: "ring", missing: "ring" };
  const shapeOf = (kind) => SHAPE[kind] ?? "circle";
  // Edge styles: colour token, width (screen px), dash (screen px), opacity, arrowhead.
  const REL = {
    prerequisite: { c: "accentText", w: 1.5, dash: null, a: 0.75, arrow: true },
    contrast: { c: "muted", w: 1.3, dash: [6, 4], a: 0.85, arrow: false },
    related: { c: "edge", w: 1.1, dash: null, a: 0.8, arrow: false },
    translation: { c: "muted", w: 1.1, dash: [1.5, 3], a: 0.85, arrow: false },
    mentions: { c: "edge", w: 0.7, dash: null, a: 0.6, arrow: false, softArrow: true },
    tag: { c: "faint", w: 0.7, dash: [2, 3], a: 0.55, arrow: false },
    _directed: { c: "muted", w: 1.2, dash: null, a: 0.75, arrow: true },
  };
  const relStyle = (rel) => REL[rel] ?? REL._directed;
  const REL_ORDER = ["tag", "mentions", "related", "translation", "contrast", "_directed", "prerequisite"];
  const GROUP_N = 8; // categorical slots (--g1…--g8); the rest fold into 其他
  const LS_KEY = "kb.graph";
  const PHYS = { center: 0.05, repel: 140, linkDist: 55, linkForce: 1 };
  const DEFAULTS = { ...PHYS, colorBy: "state", showTags: false, showMissing: true, orphans: true, arrows: true, labelAt: 1, nodeSize: 1, rels: { tag: false }, depth: 2, panel: false };
  const ALPHA_MIN = 0.001, ALPHA_DECAY = 1 - Math.pow(ALPHA_MIN, 1 / 300), VELOCITY_KEEP = 0.6, THETA2 = 0.81;
  const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)");

  // ── State ──────────────────────────────────────────────
  let S = loadSettings();
  const F = { kinds: new Set(), states: new Set(), spaces: new Set(), tags: new Set(), from: "", to: "" };
  let opts = {}, wrap, canvas, ctx, ui = {};
  let nodes = [], edges = [], byId = new Map(), active = [], activeEdges = [], edgeGroups = new Map();
  const memory = new Map(); // id → {x, y, fx, fy}: positions survive reloads and global ↔ local
  let mode = "global", center = null, data = null, loadSeq = 0;
  let view = { x: 0, y: 0, k: 1 }, W = 0, H = 0, dpr = 1;
  let alpha = 0, alphaTarget = 0, raf = 0, dirty = false;
  let hover = null, selected = null, query = "", matches = null;
  let drag = null, pinch = null, autoFit = true, viewAnim = null;
  const pointers = new Map();
  let play = null; // { t0, t1, cur, running, last }
  let P = {}; // palette
  let groupColour = new Map(), groupOrder = [];
  const stats = { nodes: 0, edges: 0, loadAt: 0, settleMs: null, ticks: 0, frames: 0, fps: 0, tickMs: 0, drawMs: 0, _fpsT: 0, _fpsF: 0 };

  function loadSettings() {
    try { const v = JSON.parse(localStorage.getItem(LS_KEY) ?? "{}"); return { ...DEFAULTS, ...v, rels: { ...DEFAULTS.rels, ...(v.rels ?? {}) } }; }
    catch { return { ...DEFAULTS, rels: { ...DEFAULTS.rels } }; }
  }
  function saveSettings() { try { localStorage.setItem(LS_KEY, JSON.stringify(S)); } catch {} }

  // ── Small DOM helpers ──────────────────────────────────
  function el(tag, attrs = {}, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else if (k === "class") e.className = v;
      else if (k === "html") e.innerHTML = v;
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c instanceof Node ? c : document.createTextNode(String(c)));
    return e;
  }
  /** replaceChildren without the null / false entries (they would become text). */
  const fill = (e, ...kids) => e.replaceChildren(...kids.flat().filter((k) => k != null && k !== false));
  const ico = (name) => (typeof icon === "function" ? icon(name) : document.createTextNode(""));
  const icoBtn = (name, label, onclick, extra = {}) => { const b = el("button", { class: "ico", type: "button", "aria-label": label, onclick, ...extra }); b.append(ico(name)); return b; };
  /** Width covered by the side card / settings panel on wide layouts (fit leaves it free). */
  const sideCover = () => { const on = !ui.card.hidden || !ui.panel.hidden; wrap.classList.toggle("has-side", on); return on && W > 720 ? 320 : 0; };
  const svgNS = "http://www.w3.org/2000/svg";
  function shapeSwatch(shape, fill, hollow = false) {
    const s = document.createElementNS(svgNS, "svg");
    s.setAttribute("viewBox", "-7 -7 14 14"); s.setAttribute("class", "g-sw"); s.setAttribute("aria-hidden", "true");
    const p = document.createElementNS(svgNS, "path");
    p.setAttribute("d", shapePath(shape, 5));
    if (hollow) { p.setAttribute("fill", "none"); p.setAttribute("stroke", fill); p.setAttribute("stroke-width", "1.5"); }
    else p.setAttribute("fill", fill);
    s.append(p);
    return s;
  }
  function shapePath(shape, r) {
    if (shape === "circle" || shape === "ring") return `M${-r} 0a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0`;
    const pts = polygon(shape, 0, 0, r);
    return "M" + pts.map((p) => p.join(" ")).join("L") + "Z";
  }
  function lineSwatch(st) {
    const s = document.createElementNS(svgNS, "svg");
    s.setAttribute("viewBox", "0 0 28 8"); s.setAttribute("class", "g-ln"); s.setAttribute("aria-hidden", "true");
    const l = document.createElementNS(svgNS, "path");
    l.setAttribute("d", st.arrow ? "M1 4H22" : "M1 4H27");
    l.setAttribute("stroke", `var(${tokenOf(st.c)})`); l.setAttribute("stroke-width", String(Math.max(1, st.w)));
    if (st.dash) l.setAttribute("stroke-dasharray", st.dash.join(" "));
    s.append(l);
    if (st.arrow) { const a = document.createElementNS(svgNS, "path"); a.setAttribute("d", "M21 1L27 4L21 7Z"); a.setAttribute("fill", `var(${tokenOf(st.c)})`); s.append(a); }
    return s;
  }
  const tokenOf = (c) => ({ accentText: "--accent-text", muted: "--muted", faint: "--faint", edge: "--g-edge", lineStrong: "--line-strong" })[c] ?? "--muted";

  // ── Palette ────────────────────────────────────────────
  function readPalette() {
    const cs = getComputedStyle(document.documentElement);
    const v = (n) => cs.getPropertyValue(n).trim();
    P = {
      text: v("--text"), muted: v("--muted"), faint: v("--faint"), line: v("--line"), lineStrong: v("--line-strong"), edge: v("--g-edge") || v("--line-strong"),
      accent: v("--accent"), accentText: v("--accent-text"), panel: v("--panel"), ok: v("--ok"), warn: v("--warn"), stale: v("--stale"),
      groups: Array.from({ length: GROUP_N }, (_, i) => v(`--g${i + 1}`)), sans: v("--sans") || "sans-serif",
    };
    for (const n of nodes) n.col = colourOf(n);
  }

  // ── Init / DOM ─────────────────────────────────────────
  function init(wrapEl, o) {
    wrap = wrapEl; opts = o ?? {};
    canvas = wrap.querySelector("canvas");
    ctx = canvas.getContext("2d");
    canvas.tabIndex = 0;
    canvas.setAttribute("role", "img");
    canvas.setAttribute("aria-label", "知識圖譜：拖曳移動、滾輪縮放、點選節點查看卡片");
    buildUi();
    readPalette();
    new ResizeObserver(() => { resize(); if (W > 0) kick(); }).observe(canvas);
    new MutationObserver(() => { readPalette(); renderLegend(); kick(); }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-mode", "data-theme"] });
    document.addEventListener("visibilitychange", () => { if (!document.hidden) kick(); });
    canvas.addEventListener("pointerdown", onDown);
    canvas.addEventListener("pointermove", onMove);
    canvas.addEventListener("pointerup", onUp);
    canvas.addEventListener("pointercancel", onUp);
    canvas.addEventListener("pointerleave", (e) => { if (e.pointerType === "mouse" && !drag) setHover(null); });
    canvas.addEventListener("dblclick", onDbl);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("keydown", onKey);
  }

  function buildUi() {
    // Top-left: search. Top-right: tools.
    ui.search = el("input", { type: "search", placeholder: "搜尋節點…", "aria-label": "搜尋圖譜中的節點", autocomplete: "off" });
    ui.sugg = el("ul", { class: "g-sugg", hidden: true, role: "listbox" });
    ui.searchBox = el("div", { class: "g-search" }, el("span", { class: "g-search-i" }, ico("search")), ui.search, ui.sugg);
    ui.search.addEventListener("input", () => setQuery(ui.search.value));
    ui.search.addEventListener("focus", () => { setHover(null); if (query) ui.sugg.hidden = false; });
    ui.searchBox.addEventListener("focusout", () => setTimeout(() => { if (!ui.searchBox.contains(document.activeElement)) ui.sugg.hidden = true; }, 150));
    ui.search.addEventListener("keydown", (e) => {
      const items = [...ui.sugg.querySelectorAll("button")];
      const i = items.indexOf(document.activeElement);
      if (e.key === "Enter") { e.preventDefault(); const m = matches?.[0]; if (m) jumpTo(m); }
      else if (e.key === "ArrowDown" && items.length) { e.preventDefault(); items[0].focus(); }
      else if (e.key === "Escape") { ui.search.value = ""; setQuery(""); canvas.focus(); }
      void i;
    });
    ui.sugg.addEventListener("keydown", (e) => {
      const items = [...ui.sugg.querySelectorAll("button")], i = items.indexOf(document.activeElement);
      if (e.key === "ArrowDown") { e.preventDefault(); items[Math.min(items.length - 1, i + 1)]?.focus(); }
      if (e.key === "ArrowUp") { e.preventDefault(); (i <= 0 ? ui.search : items[i - 1]).focus(); }
      if (e.key === "Escape") ui.search.focus();
    });
    ui.playBtn = icoBtn("play", "播放：看知識怎麼長出來", () => togglePlay());
    ui.panelBtn = icoBtn("sliders", "篩選、外觀與力度", () => togglePanel(), { "aria-expanded": "false" });
    ui.tools = el("div", { class: "g-tools" },
      icoBtn("zoomIn", "放大", () => zoomBy(1.4), { class: "ico g-zoom" }),
      icoBtn("zoomOut", "縮小", () => zoomBy(1 / 1.4), { class: "ico g-zoom" }),
      icoBtn("fit", "全部置中（F）", () => fit(true)),
      ui.playBtn, ui.panelBtn);
    ui.tip = el("div", { class: "g-tip", hidden: true, role: "tooltip" });
    ui.card = el("aside", { class: "g-card", hidden: true, "aria-label": "節點資訊" });
    ui.panel = el("aside", { class: "g-panel", hidden: true, "aria-label": "圖譜設定" });
    ui.legend = el("div", { class: "g-legend", "aria-label": "圖例" });
    ui.play = el("div", { class: "g-play", hidden: true });
    ui.status = el("div", { class: "g-status", hidden: true });
    ui.empty = el("div", { class: "g-empty", hidden: true },
      el("b", {}, "圖譜還是空的"),
      el("span", {}, "新增幾張卡片，在內文用 [[標題]] 互相連結，這裡就會長出知識網路。"),
      el("button", { class: "btn primary", type: "button", onclick: () => opts.newCard?.() }, "新增知識卡"));
    wrap.append(ui.searchBox, ui.tools, ui.play, ui.legend, ui.status, ui.empty, ui.tip, ui.card, ui.panel);
    // Head controls (mode, depth) live in the view head.
    ui.mode = document.getElementById("gMode");
    ui.depth = document.getElementById("gDepth");
    ui.depthOut = document.getElementById("gDepthOut");
    ui.depthWrap = document.getElementById("gDepthWrap");
    ui.mode?.addEventListener("click", (e) => {
      const b = e.target.closest("button[data-v]"); if (!b) return;
      if (b.dataset.v === "global") setCenter(null);
      else if (center || selected?.isCard) setCenter(center ?? selected.id);
      else toast("先點選一張卡片，再切換到局部圖譜");
    });
    if (ui.depth) {
      ui.depth.value = S.depth;
      ui.depth.addEventListener("input", () => { ui.depthOut.textContent = ui.depth.value; });
      ui.depth.addEventListener("change", () => { S.depth = +ui.depth.value; saveSettings(); if (center) load(); });
    }
  }

  const toast = (m) => opts.toast?.(m);

  // ── Loading ────────────────────────────────────────────
  function show(c) {
    center = c || null;
    mode = center ? "local" : "global";
    syncHead();
    load();
  }

  function hide() {
    cancelAnimationFrame(raf); raf = 0;
    setHover(null);
    if (play) play.running = false;
  }

  async function load() {
    const seq = ++loadSeq;
    const p = new URLSearchParams({ missing: "true" });
    if (center) { p.set("center", center); p.set("depth", String(S.depth)); p.set("limit", "5000"); }
    else p.set("full", "true");
    status("載入中…");
    try {
      const res = await fetch(`/api/graph?${p}`);
      const d = await res.json();
      if (!res.ok) throw new Error(d.error ?? res.statusText);
      if (seq !== loadSeq) return;
      status(null);
      setData(d);
    } catch (e) {
      if (seq === loadSeq) { status("無法載入圖譜"); opts.fail?.(e); }
    }
  }

  function status(t) { ui.status.hidden = !t; ui.status.textContent = t ?? ""; }

  function setData(d) {
    data = d;
    ui.empty.hidden = d.nodes.some((n) => n.kind !== "missing" && n.kind !== "tag");
    for (const n of nodes) memory.set(n.id, { x: n.x, y: n.y, fx: n.fx, fy: n.fy });
    byId = new Map();
    const fresh = [];
    nodes = d.nodes.filter((raw) => raw.kind !== "tag").map((raw, i) => {
      const m = memory.get(raw.id);
      const n = {
        ...raw, i, isCard: raw.kind !== "missing" && raw.kind !== "tag",
        x: m?.x ?? NaN, y: m?.y ?? NaN, vx: 0, vy: 0, fx: m?.fx ?? null, fy: m?.fy ?? null,
        t: raw.created_at ? Date.parse(raw.created_at) : NaN, day: raw.created_at?.slice(0, 10) ?? "",
        deg: 0, nb: [], on: true, born: 0,
      };
      if (!m) fresh.push(n);
      byId.set(n.id, n);
      return n;
    });
    // Tag nodes (shared tags), with their hierarchy: a/b/c → a/b → a.
    const tagIds = new Map();
    const tagNode = (t) => {
      let n = tagIds.get(t);
      if (n) return n;
      const id = `tag:${t}`, m = memory.get(id);
      n = { id, title: `#${t.split("/").pop()}`, tagPath: t, kind: "tag", isCard: false, tags: [], aliases: [], space: "", state: null, summary: "", x: m?.x ?? NaN, y: m?.y ?? NaN, vx: 0, vy: 0, fx: m?.fx ?? null, fy: m?.fy ?? null, t: NaN, day: "", deg: 0, nb: [], on: true, born: 0, i: nodes.length };
      if (!m) fresh.push(n);
      nodes.push(n); byId.set(id, n); tagIds.set(t, n);
      const parent = t.includes("/") ? t.slice(0, t.lastIndexOf("/")) : null;
      if (parent) edges.push(mkEdge(n, tagNode(parent), "tag"));
      return n;
    };
    edges = [];
    for (const e of d.edges) {
      if (e.origin === "tag") continue; // the server's tag nodes are rebuilt here
      const a = byId.get(e.from), b = byId.get(e.to);
      if (a && b && a !== b) edges.push(mkEdge(a, b, e.rel));
    }
    for (const n of nodes.slice()) if (n.isCard) for (const t of n.tags ?? []) edges.push(mkEdge(n, tagNode(t), "tag"));
    nodes.forEach((n, i) => (n.i = i));
    playTimes = nodes.filter((n) => n.isCard && Number.isFinite(n.t)).map((n) => n.t).sort((a, b) => a - b);
    for (const e of edges) if (e.rel !== "tag") { e.a.deg++; e.b.deg++; }
    for (const n of nodes) if (n.kind === "tag") n.deg = 0;
    // Initial positions: near a placed neighbour, else on a phyllotaxis spiral.
    const placed = (n) => Number.isFinite(n.x);
    const adj = new Map();
    for (const e of edges) { (adj.get(e.a) ?? adj.set(e.a, []).get(e.a)).push(e.b); (adj.get(e.b) ?? adj.set(e.b, []).get(e.b)).push(e.a); }
    fresh.sort((a, b) => b.deg - a.deg);
    let spiral = 0;
    for (const n of fresh) {
      const nb = (adj.get(n) ?? []).find(placed);
      if (nb) { const a = Math.random() * Math.PI * 2; n.x = nb.x + Math.cos(a) * S.linkDist * 0.6; n.y = nb.y + Math.sin(a) * S.linkDist * 0.6; }
      else { const r = 12 * Math.sqrt(0.5 + spiral), a = spiral * Math.PI * (3 - Math.sqrt(5)); spiral++; n.x = r * Math.cos(a); n.y = r * Math.sin(a); }
    }
    const c = center && byId.get(center);
    if (c && !memory.has(c.id)) { c.x = 0; c.y = 0; }
    assignGroups();
    for (const n of nodes) n.col = colourOf(n);
    if (selected) selected = byId.get(selected.id) ?? null;
    hover = null;
    // Hubs (top ~3 % by degree) keep their labels even when zoomed out.
    const degs = nodes.filter((n) => n.isCard).map((n) => n.deg).sort((a, b) => b - a);
    const hubDeg = Math.max(4, degs[Math.floor(degs.length * 0.03)] ?? 0);
    for (const n of nodes) n.hub = n.isCard && n.deg >= hubDeg;
    stats.nodes = nodes.length; stats.edges = edges.length;
    renderPanel();
    rebuild();
    if (query) setQuery(query);
    stats.loadAt = performance.now(); stats.settleMs = null; stats.ticks = 0;
    autoFit = true;
    reheat(1);
    if (selected) renderCard(); else ui.card.hidden = true;
    if (S.panel && !panelRestored) { panelRestored = true; if (wrap.clientWidth > 720) togglePanel(true); else S.panel = false; }
  }
  let panelRestored = false, playTimes = [];

  const mkEdge = (a, b, rel) => ({ a, b, rel, st: relStyle(rel), on: true });

  // ── Colour groups ──────────────────────────────────────
  function groupKey(n) {
    switch (S.colorBy) {
      case "domain": return n.domains?.[0] ?? null;
      case "tag": return n.tags?.[0]?.split("/")[0] ?? null;
      case "kind": return n.kind;
      case "space": return n.space || null;
      default: return null;
    }
  }
  function assignGroups() {
    // Colour follows the entity: slots are assigned by frequency once per data load
    // (and colour mode), never re-ranked by filters.
    groupColour = new Map(); groupOrder = [];
    if (S.colorBy === "state") return;
    const count = new Map();
    for (const n of nodes) if (n.isCard) { const k = groupKey(n); if (k != null) count.set(k, (count.get(k) ?? 0) + 1); }
    groupOrder = [...count.entries()].sort((a, b) => b[1] - a[1] || String(a[0]).localeCompare(String(b[0])));
    groupOrder.slice(0, GROUP_N).forEach(([k], i) => groupColour.set(k, i));
  }
  function colourOf(n) {
    if (n.kind === "missing") return P.faint;
    if (n.kind === "tag") return P.muted;
    if (S.colorBy === "state") return ({ learning: P.warn, known: P.ok, stale: P.stale })[n.state] ?? P.faint;
    const i = groupColour.get(groupKey(n));
    return i == null ? P.faint : P.groups[i];
  }

  // ── Filters → active set ───────────────────────────────
  function tagMatch(tags) {
    for (const sel of F.tags) for (const t of tags ?? []) if (t === sel || t.startsWith(sel + "/")) return true;
    return false;
  }
  function passCard(n) {
    if (F.kinds.size && !F.kinds.has(n.kind)) return false;
    if (F.states.size && !F.states.has(n.state ?? "none")) return false;
    if (F.spaces.size && !F.spaces.has(n.space)) return false;
    if (F.tags.size && !tagMatch(n.tags)) return false;
    if (F.from && n.day && n.day < F.from) return false;
    if (F.to && n.day && n.day > F.to) return false;
    if (play && Number.isFinite(n.t) && n.t > play.cur) return false;
    return true;
  }
  function rebuild() {
    const was = new Set(active);
    for (const n of nodes) {
      if (n.isCard) n.on = passCard(n);
      else if (n.kind === "missing") n.on = S.showMissing && (!F.kinds.size || F.kinds.has("missing"));
      else n.on = false; // tag nodes: below
    }
    if (S.showTags) { // a tag shows when a visible card carries it (or a tag below it)
      const keep = new Set();
      for (const e of edges) if (e.rel === "tag" && e.a.isCard && e.a.on) for (let t = e.b.tagPath; t; t = t.includes("/") ? t.slice(0, t.lastIndexOf("/")) : null) keep.add(t);
      for (const n of nodes) if (n.kind === "tag") n.on = keep.has(n.tagPath);
    }
    const relOn = (r) => (r === "tag" ? S.showTags : S.rels[r] !== false);
    const live = new Map();
    for (let pass = 0; pass < 2; pass++) {
      live.clear();
      for (const e of edges) {
        e.on = e.a.on && e.b.on && relOn(e.rel);
        if (e.on) { live.set(e.a, (live.get(e.a) ?? 0) + 1); live.set(e.b, (live.get(e.b) ?? 0) + 1); }
      }
      let changed = false;
      for (const n of nodes) {
        if (!n.on || n.kind === "tag") continue;
        const keepLone = n.isCard && (S.orphans || n.id === center);
        if (!keepLone && !live.get(n)) { n.on = false; changed = true; }
      }
      if (!changed) break;
    }
    active = nodes.filter((n) => n.on);
    activeEdges = edges.filter((e) => e.on && e.a.on && e.b.on);
    for (const n of nodes) n.nb = [];
    for (const e of activeEdges) { e.a.nb.push(e.b); e.b.nb.push(e.a); }
    edgeGroups = new Map();
    for (const e of activeEdges) {
      const key = REL[e.rel] ? e.rel : "_directed";
      (edgeGroups.get(key) ?? edgeGroups.set(key, []).get(key)).push(e);
    }
    // d3-style link strength/bias from active degrees.
    const cnt = new Map();
    for (const e of activeEdges) { cnt.set(e.a, (cnt.get(e.a) ?? 0) + 1); cnt.set(e.b, (cnt.get(e.b) ?? 0) + 1); }
    for (const e of activeEdges) {
      const ca = cnt.get(e.a), cb = cnt.get(e.b);
      e.k = 1 / Math.min(ca, cb); e.bias = ca / (ca + cb);
      e.len = e.rel === "tag" ? S.linkDist * 0.6 : e.rel === "mentions" ? S.linkDist * 1.15 : S.linkDist;
    }
    // Newly visible nodes start next to a visible neighbour and pop in.
    const now = performance.now();
    for (const n of active) if (!was.has(n) && was.size) {
      n.born = now;
      const nb = n.nb.find((m) => was.has(m));
      if (nb && play) { const a = Math.random() * 6.283; n.x = nb.x + Math.cos(a) * 8; n.y = nb.y + Math.sin(a) * 8; n.vx = n.vy = 0; }
    }
    if (hover && !hover.on) hover = null;
    renderInfo();
    renderLegend();
  }

  function renderInfo() {
    const cards = active.filter((n) => n.isCard).length, total = nodes.filter((n) => n.isCard).length;
    const links = activeEdges.filter((e) => e.rel !== "tag").length;
    const c = center && byId.get(center);
    const parts = [];
    if (c) parts.push(`以「${c.title}」為中心 · ${S.depth} 層`);
    parts.push(cards === total ? `${total} 張卡片` : `${cards} / ${total} 張卡片`);
    parts.push(`${links} 條連結`);
    if (data?.truncated) parts.push(`（只顯示 ${data.nodes.filter((n) => n.kind !== "missing").length} / ${data.total}）`);
    opts.info?.(parts.join(" · "));
  }

  // ── Simulation ─────────────────────────────────────────
  // Barnes–Hut quadtree in flat typed arrays, rebuilt every tick.
  let qCap = 0, qN = 0, qX0, qY0, qS, qM, qMX, qMY, qC, qB;
  let bodyNext = new Int32Array(0);
  function qAlloc(cap) {
    qCap = cap;
    qX0 = new Float64Array(cap); qY0 = new Float64Array(cap); qS = new Float64Array(cap);
    qM = new Float64Array(cap); qMX = new Float64Array(cap); qMY = new Float64Array(cap);
    qC = new Int32Array(cap * 4); qB = new Int32Array(cap);
  }
  function qNew(x0, y0, s) {
    if (qN >= qCap) {
      const old = { qX0, qY0, qS, qM, qMX, qMY, qC, qB }, n = qN;
      qAlloc(qCap * 2);
      qX0.set(old.qX0.subarray(0, n)); qY0.set(old.qY0.subarray(0, n)); qS.set(old.qS.subarray(0, n));
      qM.set(old.qM.subarray(0, n)); qMX.set(old.qMX.subarray(0, n)); qMY.set(old.qMY.subarray(0, n));
      qC.set(old.qC.subarray(0, n * 4)); qB.set(old.qB.subarray(0, n));
    }
    const c = qN++;
    qX0[c] = x0; qY0[c] = y0; qS[c] = s; qM[c] = 0; qMX[c] = 0; qMY[c] = 0;
    qC[c * 4] = qC[c * 4 + 1] = qC[c * 4 + 2] = qC[c * 4 + 3] = -1; qB[c] = -1;
    return c;
  }
  function buildTree(list) {
    if (qCap < list.length * 4 + 16) qAlloc(list.length * 4 + 16);
    if (bodyNext.length < nodes.length) bodyNext = new Int32Array(nodes.length);
    qN = 0;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const n of list) { if (n.x < x0) x0 = n.x; if (n.x > x1) x1 = n.x; if (n.y < y0) y0 = n.y; if (n.y > y1) y1 = n.y; }
    const s = Math.max(x1 - x0, y1 - y0, 1) * 1.0001;
    const root = qNew(x0, y0, s);
    for (const n of list) insert(root, n);
    return root;
  }
  function insert(c, n) {
    const i = n.i;
    bodyNext[i] = -1;
    for (let depth = 0; ; depth++) {
      qM[c] += 1; qMX[c] += n.x; qMY[c] += n.y;
      const leaf = qC[c * 4] === -1 && qC[c * 4 + 1] === -1 && qC[c * 4 + 2] === -1 && qC[c * 4 + 3] === -1;
      if (leaf) {
        if (qB[c] === -1 && qM[c] === 1) { qB[c] = i; return; }
        if (qB[c] !== -1) {
          const j = qB[c], o = nodes[j];
          if (depth > 40 || (o.x === n.x && o.y === n.y)) { bodyNext[i] = qB[c]; qB[c] = i; return; } // coincident: chain
          // Split: push the existing body chain down one level.
          qB[c] = -1;
          const half = qS[c] / 2;
          const q = (o.x >= qX0[c] + half ? 1 : 0) + (o.y >= qY0[c] + half ? 2 : 0);
          const ch = qNew(qX0[c] + (q & 1 ? half : 0), qY0[c] + (q & 2 ? half : 0), half);
          qC[c * 4 + q] = ch;
          let cnt = 0, mx = 0, my = 0;
          for (let b = j; b !== -1; b = bodyNext[b]) { cnt++; mx += nodes[b].x; my += nodes[b].y; }
          qM[ch] = cnt; qMX[ch] = mx; qMY[ch] = my; qB[ch] = j;
        }
      }
      const half = qS[c] / 2;
      const q = (n.x >= qX0[c] + half ? 1 : 0) + (n.y >= qY0[c] + half ? 2 : 0);
      let ch = qC[c * 4 + q];
      if (ch === -1) { ch = qNew(qX0[c] + (q & 1 ? half : 0), qY0[c] + (q & 2 ? half : 0), half); qC[c * 4 + q] = ch; qM[ch] = 1; qMX[ch] = n.x; qMY[ch] = n.y; qB[ch] = i; return; }
      c = ch;
    }
  }
  const stack = new Int32Array(4096);
  function repel(root, n, strength) {
    let sp = 0;
    stack[sp++] = root;
    while (sp) {
      const c = stack[--sp];
      const m = qM[c];
      if (!m) continue;
      let dx = qMX[c] / m - n.x, dy = qMY[c] / m - n.y, l = dx * dx + dy * dy;
      const s = qS[c];
      const leaf = qB[c] !== -1;
      if (!leaf && s * s / THETA2 < l) {
        if (l < 1) l = Math.sqrt(l);
        const w = strength * m / l;
        n.vx += dx * w; n.vy += dy * w;
        continue;
      }
      if (leaf) {
        for (let b = qB[c]; b !== -1; b = bodyNext[b]) {
          if (b === n.i) continue;
          const o = nodes[b];
          dx = o.x - n.x; dy = o.y - n.y; l = dx * dx + dy * dy;
          if (l === 0) { dx = (Math.random() - 0.5) * 1e-3; dy = (Math.random() - 0.5) * 1e-3; l = dx * dx + dy * dy; }
          if (l < 1) l = Math.sqrt(l);
          const w = strength / l;
          n.vx += dx * w; n.vy += dy * w;
        }
        continue;
      }
      for (let k = 0; k < 4; k++) { const ch = qC[c * 4 + k]; if (ch !== -1 && sp < stack.length) stack[sp++] = ch; }
    }
  }
  function tick() {
    const list = active;
    if (!list.length) { alpha = 0; return; }
    const a = alpha;
    const root = buildTree(list);
    const strength = -S.repel * a;
    for (const n of list) repel(root, n, strength);
    const lf = S.linkForce;
    for (const e of activeEdges) {
      const s = e.a, t = e.b;
      let dx = t.x + t.vx - s.x - s.vx || 1e-6, dy = t.y + t.vy - s.y - s.vy || 1e-6;
      let l = Math.sqrt(dx * dx + dy * dy);
      l = (l - e.len) / l * a * e.k * lf;
      dx *= l; dy *= l;
      t.vx -= dx * e.bias; t.vy -= dy * e.bias;
      s.vx += dx * (1 - e.bias); s.vy += dy * (1 - e.bias);
    }
    const cs = S.center * a;
    for (const n of list) {
      n.vx -= n.x * cs; n.vy -= n.y * cs;
      if (n.fx != null) { n.x = n.fx; n.y = n.fy; n.vx = n.vy = 0; continue; }
      n.vx *= VELOCITY_KEEP; n.vy *= VELOCITY_KEEP;
      n.x += n.vx; n.y += n.vy;
    }
    alpha += (alphaTarget - alpha) * ALPHA_DECAY;
    stats.ticks++;
  }

  function reheat(a = 0.3) { alpha = Math.max(alpha, a); kick(); }
  function kick() { dirty = true; if (!raf) raf = requestAnimationFrame(frame); }

  function frame(ts) {
    raf = 0;
    if (document.hidden || !canvas || canvas.clientWidth === 0) return; // resumes on visibility / resize
    const t0 = performance.now();
    let busy = false;
    if (play?.running) { advancePlay(ts); busy = true; }
    if (alpha >= ALPHA_MIN || alphaTarget > 0) {
      // As many ticks as fit in ~9 ms (≥1): small graphs settle quickly, big ones stay smooth.
      let n = 0;
      do { tick(); n++; } while (performance.now() - t0 < 9 && n < 12 && alpha >= ALPHA_MIN);
      stats.tickMs = (performance.now() - t0) / n;
      busy = true;
      if (alpha < ALPHA_MIN && alphaTarget === 0 && stats.settleMs == null) stats.settleMs = Math.round(performance.now() - stats.loadAt);
    }
    if (autoFit && busy && !pointers.size) fit(false, true);
    if (viewAnim) { stepViewAnim(ts); busy = true; }
    const t1 = performance.now();
    if (busy || dirty) { draw(); dirty = false; }
    stats.drawMs = performance.now() - t1;
    if (pops()) busy = true;
    stats.frames++; stats._fpsF++;
    if (ts - stats._fpsT > 1000) { stats.fps = Math.round(stats._fpsF * 1000 / (ts - stats._fpsT)); stats._fpsT = ts; stats._fpsF = 0; }
    if (busy) raf = requestAnimationFrame(frame);
  }
  const pops = () => { const now = performance.now(); for (const n of active) if (n.born && now - n.born < 450) return true; return false; };

  // ── View ───────────────────────────────────────────────
  function resize() {
    const r = canvas.getBoundingClientRect();
    dpr = window.devicePixelRatio || 1;
    W = r.width; H = r.height;
    canvas.width = Math.max(1, Math.round(W * dpr)); canvas.height = Math.max(1, Math.round(H * dpr));
    dirty = true;
  }
  const toWorld = (sx, sy) => ({ x: (sx - view.x) / view.k, y: (sy - view.y) / view.k });
  function bounds(list) {
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const n of list) { const r = radius(n); x0 = Math.min(x0, n.x - r); x1 = Math.max(x1, n.x + r); y0 = Math.min(y0, n.y - r); y1 = Math.max(y1, n.y + r); }
    return { x0, y0, x1, y1 };
  }
  function fitTarget(list = active) {
    const side = sideCover(), w = W - side, top = 50; // keep clear of the search bar and the side panel
    if (!list.length || !W) return { x: w / 2, y: H / 2, k: 1 };
    const b = bounds(list), pad = Math.min(50, w * 0.06);
    const k = Math.max(0.03, Math.min(1.6, Math.min((w - pad * 2) / Math.max(1, b.x1 - b.x0), (H - top - pad * 2) / Math.max(1, b.y1 - b.y0))));
    return { k, x: w / 2 - (b.x0 + b.x1) / 2 * k, y: top + (H - top) / 2 - (b.y0 + b.y1) / 2 * k };
  }
  function fit(animate, follow = false) {
    const t = fitTarget();
    if (follow) { // ease toward the target while the layout is still moving
      const f = 0.18;
      view.k += (t.k - view.k) * f; view.x += (t.x - view.x) * f; view.y += (t.y - view.y) * f;
      return;
    }
    animateView(t, animate);
  }
  function animateView(t, animate = true) {
    if (!animate || reduceMotion.matches) { view = { ...t }; viewAnim = null; kick(); return; }
    viewAnim = { from: { ...view }, to: t, t0: performance.now(), dur: 420 };
    kick();
  }
  function stepViewAnim() {
    const a = viewAnim, p = Math.min(1, (performance.now() - a.t0) / a.dur), e = 1 - Math.pow(1 - p, 3);
    // interpolate in world space so the zoom feels even
    view.k = a.from.k * Math.pow(a.to.k / a.from.k, e);
    const cx0 = (W / 2 - a.from.x) / a.from.k, cy0 = (H / 2 - a.from.y) / a.from.k;
    const cx1 = (W / 2 - a.to.x) / a.to.k, cy1 = (H / 2 - a.to.y) / a.to.k;
    const cx = cx0 + (cx1 - cx0) * e, cy = cy0 + (cy1 - cy0) * e;
    view.x = W / 2 - cx * view.k; view.y = H / 2 - cy * view.k;
    if (p >= 1) viewAnim = null;
  }
  function zoomAt(sx, sy, factor) {
    const k = Math.max(0.03, Math.min(8, view.k * factor));
    const w = toWorld(sx, sy);
    view.x = sx - w.x * k; view.y = sy - w.y * k; view.k = k;
    autoFit = false; viewAnim = null; kick();
  }
  function zoomBy(f) {
    autoFit = false;
    const k = Math.max(0.03, Math.min(8, view.k * f)), w = toWorld(W / 2, H / 2);
    animateView({ k, x: W / 2 - w.x * k, y: H / 2 - w.y * k });
  }
  function jumpTo(n) {
    if (!n.on) { toast("這個節點目前被篩選隱藏"); return; }
    autoFit = false;
    const k = Math.max(view.k, 1.4);
    animateView({ k, x: W / 2 - n.x * k, y: H / 2 - n.y * k });
    select(n);
    ui.sugg.hidden = true;
    if (W < 720) ui.search.blur();
  }

  // ── Drawing ────────────────────────────────────────────
  // World radius by degree; never smaller than ~2.4 screen px so zoomed-out nodes stay visible.
  const radius = (n) => Math.max(2.4 / view.k, n.kind === "tag" ? 3.2 : n.kind === "missing" ? 3.4 : (3.4 + Math.sqrt(n.deg) * 2.6) * S.nodeSize);
  function polygon(shape, x, y, r) {
    const sides = { square: 4, diamond: 4, triangle: 3, hexagon: 6, pentagon: 5 }[shape] ?? 4;
    const rot = { square: Math.PI / 4, diamond: 0, triangle: -Math.PI / 2, hexagon: 0, pentagon: -Math.PI / 2 }[shape] ?? 0;
    const rr = r * ({ square: 1.25, diamond: 1.3, triangle: 1.45, hexagon: 1.1, pentagon: 1.15 }[shape] ?? 1);
    const pts = [];
    for (let i = 0; i < sides; i++) { const a = rot + (i * 2 * Math.PI) / sides; pts.push([x + Math.cos(a) * rr, y + Math.sin(a) * rr]); }
    return pts;
  }
  function tracePath(n, r) {
    const sh = shapeOf(n.kind);
    if (sh === "circle" || sh === "ring") { ctx.moveTo(n.x + r, n.y); ctx.arc(n.x, n.y, r, 0, Math.PI * 2); return; }
    const pts = polygon(sh, n.x, n.y, r);
    ctx.moveTo(pts[0][0], pts[0][1]);
    for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i][0], pts[i][1]);
    ctx.closePath();
  }
  const popScale = (n, now) => { if (!n.born) return 1; const p = (now - n.born) / 450; if (p >= 1) { n.born = 0; return 1; } return 0.2 + 0.8 * (1 - Math.pow(1 - p, 3)) + Math.sin(p * Math.PI) * 0.25; };

  function draw() {
    if (!ctx || !W) return;
    if ((window.devicePixelRatio || 1) !== dpr) resize();
    const t0 = performance.now(), k = view.k;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);
    // Visible world rectangle (with a margin) for culling.
    const m = 40 / k, vx0 = -view.x / k - m, vy0 = -view.y / k - m, vx1 = (W - view.x) / k + m, vy1 = (H - view.y) / k + m;
    const inView = (n) => n.x >= vx0 && n.x <= vx1 && n.y >= vy0 && n.y <= vy1;
    const focus = hover ?? selected;
    let lit = null; // nodes kept bright
    if (focus?.on) { lit = new Set(focus.nb); lit.add(focus); }
    else if (matches) lit = new Set(matches);
    const dim = !!lit;
    ctx.setTransform(dpr * k, 0, 0, dpr * k, dpr * view.x, dpr * view.y);
    ctx.lineCap = "round";

    // Edges, one path per style.
    const edgeVisible = (e) => !((e.a.x < vx0 && e.b.x < vx0) || (e.a.x > vx1 && e.b.x > vx1) || (e.a.y < vy0 && e.b.y < vy0) || (e.a.y > vy1 && e.b.y > vy1));
    const hot = [];
    for (const key of REL_ORDER) {
      const list = edgeGroups.get(key);
      if (!list?.length) continue;
      const st = REL[key];
      ctx.beginPath();
      let any = false;
      for (const e of list) {
        if (!edgeVisible(e)) continue;
        if (focus && (e.a === focus || e.b === focus)) { hot.push(e); continue; }
        ctx.moveTo(e.a.x, e.a.y); ctx.lineTo(e.b.x, e.b.y); any = true;
      }
      if (!any) continue;
      ctx.strokeStyle = P[st.c] ?? P.muted;
      ctx.globalAlpha = st.a * (dim ? 0.15 : 1) * (k < 0.7 ? 0.55 + 0.45 * Math.max(0, (k - 0.15) / 0.55) : 1);
      ctx.lineWidth = st.w / k;
      ctx.setLineDash(st.dash ? st.dash.map((d) => d / k) : []);
      ctx.stroke();
      if (S.arrows && st.arrow && k > 0.45 && !dim) arrows(list.filter((e) => edgeVisible(e) && !(focus && (e.a === focus || e.b === focus))), st, k);
    }
    ctx.setLineDash([]);
    // Edges touching the hovered / selected node, on top and in accent.
    for (const e of hot) {
      const st = e.st;
      ctx.beginPath(); ctx.moveTo(e.a.x, e.a.y); ctx.lineTo(e.b.x, e.b.y);
      ctx.strokeStyle = P.accentText; ctx.globalAlpha = 0.95; ctx.lineWidth = Math.max(1.4, st.w + 0.4) / k;
      ctx.setLineDash(st.dash ? st.dash.map((d) => d / k) : []);
      ctx.stroke();
      if (S.arrows && (st.arrow || st.softArrow)) { ctx.setLineDash([]); arrows([e], { ...st, c: "accentText", a: 0.95 }, k); }
    }
    ctx.setLineDash([]);

    // Nodes, batched by colour × shape; dimmed ones first.
    const now = performance.now();
    const batches = new Map();
    for (const n of active) {
      if (!inView(n)) continue;
      const bright = !dim || lit.has(n);
      const hollow = n.kind === "missing" || n.kind === "tag";
      const key = `${bright ? 1 : 0}|${n.col}|${hollow ? "h" : "f"}`;
      (batches.get(key) ?? batches.set(key, []).get(key)).push(n);
    }
    const keys = [...batches.keys()].sort();
    for (const key of keys) {
      const [bright, col, h] = key.split("|");
      const list = batches.get(key);
      ctx.globalAlpha = bright === "1" ? 1 : 0.16;
      ctx.beginPath();
      for (const n of list) tracePath(n, radius(n) * popScale(n, now));
      if (h === "h") {
        ctx.fillStyle = P.panel; ctx.fill();
        ctx.strokeStyle = col; ctx.lineWidth = 1.3 / k;
        ctx.setLineDash(list[0].kind === "missing" ? [2.5 / k, 2 / k] : []);
        ctx.stroke(); ctx.setLineDash([]);
      } else {
        ctx.fillStyle = col; ctx.fill();
        // A thin ring in the background colour keeps overlapping nodes apart.
        ctx.strokeStyle = P.panel; ctx.lineWidth = 1 / k; ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;
    // Rings: pinned, local center, search matches, selection, hover.
    const ring = (n, colour, w, gap = 2.5) => { ctx.beginPath(); ctx.arc(n.x, n.y, radius(n) + gap / k + w / k / 2, 0, Math.PI * 2); ctx.strokeStyle = colour; ctx.lineWidth = w / k; ctx.stroke(); };
    for (const n of active) {
      if (!inView(n)) continue;
      if (n.fx != null) { ctx.beginPath(); ctx.arc(n.x, n.y, Math.max(1.3, radius(n) * 0.3), 0, Math.PI * 2); ctx.fillStyle = P.panel; ctx.fill(); }
    }
    if (matches && !focus) for (const n of matches) if (n.on && inView(n)) ring(n, P.accentText, 1.5);
    const c = center && byId.get(center);
    if (c?.on) ring(c, P.text, 1.5, 4);
    if (selected?.on) ring(selected, P.accentText, 2.2);
    if (hover?.on && hover !== selected) ring(hover, P.text, 1.5);

    drawLabels(lit, inView);
    ctx.globalAlpha = 1;
    stats.drawMs = performance.now() - t0;
  }

  function arrows(list, st, k) {
    const L = 7 / k, Wd = 3.2 / k;
    ctx.beginPath();
    for (const e of list) {
      const dx = e.b.x - e.a.x, dy = e.b.y - e.a.y, d = Math.hypot(dx, dy);
      const rb = radius(e.b) + 1.5 / k;
      if (d < rb + L * 1.2) continue;
      const ux = dx / d, uy = dy / d, tx = e.b.x - ux * rb, ty = e.b.y - uy * rb;
      ctx.moveTo(tx, ty);
      ctx.lineTo(tx - ux * L - uy * Wd, ty - uy * L + ux * Wd);
      ctx.lineTo(tx - ux * L + uy * Wd, ty - uy * L - ux * Wd);
      ctx.closePath();
    }
    ctx.fillStyle = P[st.c] ?? P.muted; ctx.globalAlpha = st.a; ctx.fill();
  }

  function drawLabels(lit, inView) {
    const k = view.k;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.font = `500 12px ${P.sans}`;
    ctx.textAlign = "center"; ctx.textBaseline = "top";
    ctx.lineJoin = "round";
    const th = 5.5 * S.labelAt; // screen radius at which a label appears
    const cand = [];
    for (const n of active) {
      if (!inView(n)) continue;
      const rs = radius(n) * k;
      let a;
      if (n === hover || n === selected) a = 1;
      else if (lit) a = lit.has(n) ? 1 : 0;
      else a = Math.max(n.hub && k > 0.2 ? 0.85 : 0, Math.min(1, (rs - th) / (th * 0.25)));
      if (a > 0.02) cand.push([n, a, rs]);
    }
    const pri = (x) => (x[0] === hover || x[0] === selected ? 1e9 : 0) + x[2];
    cand.sort((p, q) => pri(q) - pri(p));
    const grid = new Map(), CELL = 80, placed = [];
    const hit = (r) => {
      for (let gx = Math.floor(r.x0 / CELL); gx <= Math.floor(r.x1 / CELL); gx++)
        for (let gy = Math.floor(r.y0 / CELL); gy <= Math.floor(r.y1 / CELL); gy++)
          for (const o of grid.get(gx * 7919 + gy) ?? []) if (r.x0 < o.x1 && r.x1 > o.x0 && r.y0 < o.y1 && r.y1 > o.y0) return true;
      return false;
    };
    const put = (r) => {
      for (let gx = Math.floor(r.x0 / CELL); gx <= Math.floor(r.x1 / CELL); gx++)
        for (let gy = Math.floor(r.y0 / CELL); gy <= Math.floor(r.y1 / CELL); gy++) (grid.get(gx * 7919 + gy) ?? grid.set(gx * 7919 + gy, []).get(gx * 7919 + gy)).push(r);
    };
    let count = 0;
    for (const [n, a, rs] of cand) {
      if (count > 320) break;
      const text = n.title.length > 40 ? n.title.slice(0, 38) + "…" : n.title;
      if (n._tw == null || n._tt !== text) { n._tw = ctx.measureText(text).width; n._tt = text; }
      const sx = n.x * k + view.x, sy = n.y * k + view.y + rs + 3;
      const r = { x0: sx - n._tw / 2 - 2, x1: sx + n._tw / 2 + 2, y0: sy, y1: sy + 15 };
      const force = n === hover || n === selected;
      if (!force && hit(r)) continue;
      put(r); placed.push([n, a, sx, sy, text]); count++;
    }
    for (const [n, a, sx, sy, text] of placed) {
      ctx.globalAlpha = a;
      ctx.lineWidth = 3.5; ctx.strokeStyle = P.panel; ctx.strokeText(text, sx, sy);
      ctx.fillStyle = n.kind === "missing" || n.kind === "tag" ? P.muted : P.text;
      ctx.fillText(text, sx, sy);
    }
    ctx.globalAlpha = 1;
  }

  // ── Interaction ────────────────────────────────────────
  function hitTest(sx, sy, slop = 4) {
    const w = toWorld(sx, sy);
    let best = null, bd = Infinity;
    for (let i = active.length - 1; i >= 0; i--) {
      const n = active[i], r = radius(n) + slop / view.k;
      const d = (n.x - w.x) ** 2 + (n.y - w.y) ** 2;
      if (d <= r * r && d < bd) { best = n; bd = d; }
    }
    return best;
  }
  const local = (e) => { const r = canvas.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };

  function onDown(e) {
    canvas.setPointerCapture(e.pointerId);
    const p = local(e);
    pointers.set(e.pointerId, p);
    hideTip();
    if (pointers.size === 2) { // pinch
      if (drag?.node) { drag.node.fx = drag.node.fy = null; alphaTarget = 0; }
      drag = null;
      const [a, b] = [...pointers.values()];
      pinch = { d: Math.hypot(a.x - b.x, a.y - b.y) || 1, k: view.k, mid: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }, view: { ...view } };
      return;
    }
    if (pointers.size > 2) return;
    const n = hitTest(p.x, p.y, e.pointerType === "mouse" ? 4 : 12);
    drag = { node: n, sx: p.x, sy: p.y, vx: view.x, vy: view.y, moved: false, type: e.pointerType, meta: e.metaKey || e.ctrlKey, wasPinned: n?.fx != null };
    viewAnim = null;
  }
  function onMove(e) {
    const p = local(e);
    if (pointers.has(e.pointerId)) pointers.set(e.pointerId, p);
    if (pinch && pointers.size >= 2) {
      const [a, b] = [...pointers.values()];
      const d = Math.hypot(a.x - b.x, a.y - b.y) || 1, mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
      const k = Math.max(0.03, Math.min(8, pinch.k * d / pinch.d));
      const w = { x: (pinch.mid.x - pinch.view.x) / pinch.view.k, y: (pinch.mid.y - pinch.view.y) / pinch.view.k };
      view.k = k; view.x = mid.x - w.x * k; view.y = mid.y - w.y * k;
      autoFit = false; kick();
      return;
    }
    if (!drag) {
      if (e.pointerType === "mouse") { const n = hitTest(p.x, p.y); setHover(n, p); }
      return;
    }
    if (!drag.moved && Math.abs(p.x - drag.sx) + Math.abs(p.y - drag.sy) > (drag.type === "mouse" ? 3 : 8)) {
      drag.moved = true; autoFit = false;
      if (drag.node) { alphaTarget = 0.25; reheat(0.25); }
    }
    if (!drag.moved) return;
    if (drag.node) {
      const w = toWorld(p.x, p.y);
      drag.node.fx = w.x; drag.node.fy = w.y; drag.node.x = w.x; drag.node.y = w.y;
      if (hover !== drag.node) setHover(drag.node);
      kick();
    } else {
      view.x = drag.vx + p.x - drag.sx; view.y = drag.vy + p.y - drag.sy;
      canvas.style.cursor = "grabbing";
      kick();
    }
  }
  function onUp(e) {
    pointers.delete(e.pointerId);
    if (pinch) { if (pointers.size < 2) pinch = null; drag = null; return; }
    if (!drag) return;
    const d = drag; drag = null;
    canvas.style.cursor = hover ? "pointer" : "grab";
    if (d.node && d.moved) { alphaTarget = 0; d.node.pinned = true; memory.set(d.node.id, { x: d.node.x, y: d.node.y, fx: d.node.fx, fy: d.node.fy }); kick(); return; }
    if (d.moved || e.type === "pointercancel") return;
    if (d.node) {
      if (d.meta && d.node.isCard) { opts.open?.(d.node.id); return; }
      select(d.node);
      if (d.type !== "mouse") setHover(null);
    } else {
      select(null);
    }
  }
  function onDbl(e) {
    const p = local(e), n = hitTest(p.x, p.y);
    if (!n) { zoomAt(p.x, p.y, 1.6); return; }
    if (n.fx != null) { n.fx = n.fy = null; n.pinned = false; memory.delete(n.id); reheat(0.2); toast(`已取消固定「${n.title}」`); renderCard(); }
    else if (n.isCard) opts.open?.(n.id);
  }
  function onWheel(e) {
    e.preventDefault();
    const p = local(e);
    // Trackpad pinch arrives as ctrl+wheel with small deltas.
    const dy = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    zoomAt(p.x, p.y, Math.exp(-dy * (e.ctrlKey ? 0.01 : 0.0015)));
    hideTip();
  }
  function onKey(e) {
    if (e.key === "f" || e.key === "F") { autoFit = false; fit(true); }
    else if (e.key === "Escape") { select(null); setQuery(""); ui.search.value = ""; }
    else if (e.key === "/") { e.preventDefault(); ui.search.focus(); }
    else if (e.key === "+" || e.key === "=") zoomBy(1.3);
    else if (e.key === "-") zoomBy(1 / 1.3);
    else if (e.key === "Enter" && selected?.isCard) opts.open?.(selected.id);
  }

  function setHover(n, p) {
    if (n !== hover) {
      hover = n;
      canvas.style.cursor = n ? "pointer" : "grab";
      kick();
    }
    if (n && p && !drag) showTip(n, p); else hideTip();
  }
  function hideTip() { ui.tip.hidden = true; ui.tip._n = null; }
  function showTip(n, p) {
    const t = ui.tip;
    if (t._n !== n || t.hidden) t._n = n, fill(t,
      el("div", { class: "g-tip-h" }, shapeSwatch(shapeOf(n.kind), n.col, !n.isCard), el("b", {}, n.title)),
      n.isCard ? el("div", { class: "g-tip-m" }, stateText(n), " · ", opts.kindLabel?.(n.kind) ?? n.kind, ` · ${n.deg} 條連結`) : null,
      n.kind === "missing" ? el("div", { class: "g-tip-m" }, "尚未建立的 [[連結]] · 點選可建立") : null,
      n.kind === "tag" ? el("div", { class: "g-tip-m" }, `標籤 ${n.tagPath} · ${n.nb.filter((m) => m.isCard).length} 張卡片`) : null,
      n.summary ? el("div", { class: "g-tip-s" }, n.summary) : null,
    );
    t.hidden = false;
    const tw = t.offsetWidth, th = t.offsetHeight;
    let x = p.x + 14, y = p.y + 14;
    if (x + tw > W - 8) x = p.x - tw - 14;
    if (y + th > H - 8) y = p.y - th - 14;
    t.style.left = `${Math.max(8, x)}px`; t.style.top = `${Math.max(8, y)}px`;
  }
  const stateText = (n) => el("span", { class: "st" }, el("i", { class: `st-${n.state ?? "none"}` }), STATE_LABEL[n.state ?? "none"]);

  function select(n) {
    selected = n;
    kick();
    renderCard();
    // Phones: the card is a bottom sheet; keep the selected node in the upper part.
    if (n && W < 720 && !ui.card.hidden) {
      const sy = n.y * view.k + view.y, sx = n.x * view.k + view.x;
      if (sy > H * 0.42 || sx < 20 || sx > W - 20) { autoFit = false; animateView({ k: view.k, x: W / 2 - n.x * view.k, y: H * 0.26 - n.y * view.k }); }
    }
  }

  // ── Search ─────────────────────────────────────────────
  function setQuery(q) {
    query = q.trim().toLowerCase();
    if (!query) { matches = null; ui.sugg.hidden = true; kick(); return; }
    const score = (n) => {
      const t = n.title.toLowerCase();
      if (t === query) return 0;
      if (t.startsWith(query)) return 1;
      if (t.includes(query)) return 2;
      if ((n.aliases ?? []).some((a) => a.toLowerCase().includes(query))) return 3;
      return 9;
    };
    matches = nodes.filter((n) => n.kind !== "tag" && score(n) < 9).sort((a, b) => score(a) - score(b) || b.deg - a.deg);
    const shown = matches.slice(0, 8);
    ui.sugg.replaceChildren(...(shown.length ? shown.map((n) => el("li", {}, el("button", { type: "button", onclick: () => jumpTo(n) },
      shapeSwatch(shapeOf(n.kind), n.col, !n.isCard), el("span", { class: "t" }, n.title), n.on ? null : el("span", { class: "muted small" }, "已隱藏")))) : [el("li", { class: "muted small g-none" }, "找不到")]));
    if (matches.length > 8) ui.sugg.append(el("li", { class: "muted small g-none" }, `另有 ${matches.length - 8} 個符合`));
    ui.sugg.hidden = false;
    kick();
  }

  // ── Local / global ─────────────────────────────────────
  function setCenter(id) {
    center = id; mode = id ? "local" : "global";
    syncHead();
    opts.onCenter?.(id);
    load();
  }
  function syncHead() {
    ui.mode?.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.v === mode)));
    if (ui.depthWrap) ui.depthWrap.hidden = mode !== "local";
    if (ui.depth) { ui.depth.value = S.depth; ui.depthOut.textContent = S.depth; }
  }

  // ── Side card ──────────────────────────────────────────
  function renderCard() {
    const n = selected;
    if (!n) { ui.card.hidden = true; sideCover(); return; }
    const c = ui.card;
    const close = icoBtn("x", "關閉", () => select(null));
    if (n.kind === "missing") {
      fill(c,el("div", { class: "g-card-h" }, el("span", { class: "tag" }, "尚未建立"), close),
        el("h3", {}, n.title),
        el("p", { class: "muted" }, `有 ${n.nb.length || n.deg} 張卡片用 [[${n.title}]] 提到它，但還沒有這張卡。`),
        neighbours(n),
        el("div", { class: "g-card-a" }, el("button", { class: "btn primary", type: "button", onclick: () => opts.create?.(n.title) }, ico("plus"), "建立這張卡")));
    } else if (n.kind === "tag") {
      fill(c,el("div", { class: "g-card-h" }, el("span", { class: "tag" }, "標籤"), close),
        el("h3", {}, n.tagPath),
        neighbours(n),
        el("div", { class: "g-card-a" }, el("button", { class: "btn", type: "button", onclick: () => { F.tags = new Set([n.tagPath]); renderPanel(); rebuild(); reheat(0.4); autoFit = true; } }, ico("search"), "只看這個標籤")));
    } else {
      const meta = [n.created_at ? `建立 ${n.created_at.slice(0, 10)}` : null, opts.spaceName?.(n.space) ?? n.space].filter(Boolean).join(" · ");
      fill(c,
        el("div", { class: "g-card-h" }, el("span", { class: "tag kind" }, opts.kindLabel?.(n.kind) ?? n.kind), stateText(n), close),
        el("h3", {}, n.title),
        n.aliases?.length ? el("div", { class: "g-card-al" }, n.aliases.join(" · ")) : null,
        n.summary ? el("p", { class: "g-card-s" }, n.summary) : null,
        el("div", { class: "g-card-m muted small" }, meta),
        n.tags?.length ? el("div", { class: "g-card-t" }, n.tags.map((t) => el("span", { class: "tag" }, t))) : null,
        el("div", { class: "g-card-a" },
          el("button", { class: "btn primary", type: "button", onclick: () => opts.open?.(n.id) }, ico("pen"), "開啟卡片"),
          n.id !== center ? el("button", { class: "btn", type: "button", onclick: () => setCenter(n.id) }, ico("target"), "局部圖譜") : null,
          n.fx != null ? el("button", { class: "btn", type: "button", onclick: () => { n.fx = n.fy = null; memory.delete(n.id); reheat(0.2); renderCard(); } }, "取消固定") : null),
        neighbours(n));
    }
    c.hidden = false;
    sideCover();
  }
  function neighbours(n) {
    const out = new Map(), inc = new Map();
    for (const e of activeEdges) {
      if (e.rel === "tag" && n.kind !== "tag") continue;
      const sym = !relStyle(e.rel).arrow && !relStyle(e.rel).softArrow; // undirected: one list
      if (e.a === n) (out.get(e.rel) ?? out.set(e.rel, []).get(e.rel)).push(e.b);
      else if (e.b === n) { const m = sym ? out : inc; (m.get(e.rel) ?? m.set(e.rel, []).get(e.rel)).push(e.a); }
    }
    if (!out.size && !inc.size) return el("p", { class: "muted small" }, "目前沒有顯示中的連結");
    const IN = { prerequisite: "以它為先備知識", "is-a": "下位概念", "part-of": "組成部分", "example-of": "例子", uses: "被用到" };
    const relName = (r, dir) => r === "mentions" ? (dir === "out" ? "提到" : "被提到") : r === "tag" ? (dir === "out" ? "上層標籤" : "卡片與子標籤")
      : dir === "in" && relStyle(r).arrow ? (IN[r] ?? `← ${opts.relLabel?.(r) ?? r}`) : (opts.relLabel?.(r) ?? r);
    const block = (map, dir) => [...map.entries()].map(([r, list]) => el("div", { class: "g-nb" },
      el("div", { class: "g-nb-h" }, lineSwatch(relStyle(r)), relName(r, dir), el("span", { class: "muted" }, ` ${list.length}`)),
      el("ul", {}, list.slice(0, 30).map((m) => el("li", {}, el("button", { type: "button", onclick: () => jumpTo(m) }, shapeSwatch(shapeOf(m.kind), m.col, !m.isCard), m.title))),
        list.length > 30 ? el("li", { class: "muted small" }, `…另有 ${list.length - 30} 個`) : null)));
    return el("div", { class: "g-nbs" }, block(out, "out"), block(inc, "in"));
  }

  // ── Legend ─────────────────────────────────────────────
  function renderLegend() {
    if (!ui.legend) return;
    const items = [];
    if (S.colorBy === "state") {
      const cnt = new Map();
      for (const n of active) if (n.isCard) cnt.set(n.state ?? "none", (cnt.get(n.state ?? "none") ?? 0) + 1);
      for (const [k, l, tok] of STATES) items.push(el("span", {}, el("i", { style: `background:var(${tok})` }), l, el("em", {}, cnt.get(k) ?? 0)));
    } else {
      const cnt = new Map();
      for (const n of active) if (n.isCard) { const k = groupKey(n); const key = groupColour.has(k) ? k : "__other"; cnt.set(key, (cnt.get(key) ?? 0) + 1); }
      for (const [k] of groupOrder.slice(0, GROUP_N)) items.push(el("span", {}, el("i", { style: `background:var(--g${groupColour.get(k) + 1})` }), groupLabel(k), el("em", {}, cnt.get(k) ?? 0)));
      if (groupOrder.length > GROUP_N || cnt.get("__other")) items.push(el("span", {}, el("i", { style: "background:var(--faint)" }), "其他", el("em", {}, cnt.get("__other") ?? 0)));
    }
    if (S.showMissing && active.some((n) => n.kind === "missing")) items.push(el("span", {}, el("i", { class: "hollow dashed" }), "尚未建立"));
    if (S.showTags && active.some((n) => n.kind === "tag")) items.push(el("span", {}, el("i", { class: "hollow" }), "標籤"));
    ui.legend.replaceChildren(...items);
  }
  const groupLabel = (k) => S.colorBy === "kind" ? (opts.kindLabel?.(k) ?? k) : S.colorBy === "space" ? (opts.spaceName?.(k) ?? k) : S.colorBy === "domain" ? (opts.domainLabel?.(k) ?? k) : k;

  // ── Settings panel ─────────────────────────────────────
  function togglePanel(force) {
    S.panel = force ?? !S.panel; saveSettings();
    ui.panel.hidden = !S.panel;
    ui.panelBtn.setAttribute("aria-expanded", String(S.panel));
    ui.panelBtn.classList.toggle("on", S.panel);
    if (S.panel && !ui.card.hidden && W < 720) ui.card.hidden = true; // one sheet at a time on phones
    sideCover();
    if (S.panel) renderPanel();
  }
  function section(title, open, ...kids) {
    const d = el("details", { class: "g-sec", open }, el("summary", {}, title), ...kids);
    d.addEventListener("toggle", () => { (S.openSecs ??= {})[title] = d.open; saveSettings(); });
    if (S.openSecs && title in S.openSecs) d.open = S.openSecs[title];
    return d;
  }
  function chip(label, on, onclick, ...pre) {
    return el("button", { type: "button", class: "g-chip", "aria-pressed": String(on), onclick }, ...pre, label);
  }
  const refilter = () => { rebuild(); reheat(0.35); renderPanel(); kick(); };

  function renderPanel() {
    if (!ui.panel || ui.panel.hidden && !S.panel) return;
    const cards = nodes.filter((n) => n.isCard);
    const count = (f) => { const m = new Map(); for (const n of cards) for (const k of [f(n)].flat()) if (k != null) m.set(k, (m.get(k) ?? 0) + 1); return m; };
    const toggle = (set, k) => () => { set.has(k) ? set.delete(k) : set.add(k); refilter(); };
    // Filters
    const kinds = count((n) => n.kind), states = count((n) => n.state ?? "none"), spaces = count((n) => n.space);
    const kindChips = [...kinds.entries()].sort((a, b) => b[1] - a[1]).map(([k, c]) => chip(`${opts.kindLabel?.(k) ?? k} ${c}`, F.kinds.has(k), toggle(F.kinds, k), shapeSwatch(shapeOf(k), "currentColor")));
    const stateChips = STATES.filter(([k]) => states.has(k)).map(([k, l, tok]) => chip(`${l} ${states.get(k)}`, F.states.has(k), toggle(F.states, k), el("i", { class: "g-dot", style: `background:var(${tok})` })));
    const spaceChips = spaces.size > 1 ? [...spaces.entries()].map(([k, c]) => chip(`${opts.spaceName?.(k) ?? k} ${c}`, F.spaces.has(k), toggle(F.spaces, k))) : [];
    const days = cards.map((n) => n.day).filter(Boolean).sort();
    const date = (v, ph, set) => { const i = el("input", { type: "date", value: v, min: days[0], max: days[days.length - 1], "aria-label": ph }); i.addEventListener("change", () => { set(i.value); refilter(); }); return i; };
    const anyFilter = F.kinds.size || F.states.size || F.spaces.size || F.tags.size || F.from || F.to;
    const filters = section("篩選", true,
      el("div", { class: "g-row" }, el("span", { class: "g-lab" }, "種類"), el("div", { class: "g-chips" }, kindChips)),
      el("div", { class: "g-row" }, el("span", { class: "g-lab" }, "熟悉度"), el("div", { class: "g-chips" }, stateChips)),
      spaceChips.length ? el("div", { class: "g-row" }, el("span", { class: "g-lab" }, "空間"), el("div", { class: "g-chips" }, spaceChips)) : null,
      el("div", { class: "g-row" }, el("span", { class: "g-lab" }, "標籤"), tagTree(cards)),
      el("div", { class: "g-row" }, el("span", { class: "g-lab" }, "建立日期"), el("div", { class: "g-dates" }, date(F.from, "從", (v) => (F.from = v)), el("span", { class: "muted" }, "–"), date(F.to, "到", (v) => (F.to = v)))),
      check("顯示孤立節點", S.orphans, (v) => { S.orphans = v; }),
      check("顯示尚未建立的 [[連結]]", S.showMissing, (v) => { S.showMissing = v; }),
      check("顯示標籤節點（共同標籤）", S.showTags, (v) => { S.showTags = v; }),
      anyFilter ? el("button", { class: "btn small", type: "button", onclick: () => { F.kinds.clear(); F.states.clear(); F.spaces.clear(); F.tags.clear(); F.from = F.to = ""; refilter(); } }, "清除篩選") : null);
    // Link types
    const relCount = new Map();
    for (const e of edges) relCount.set(e.rel, (relCount.get(e.rel) ?? 0) + 1);
    const rels = [...relCount.entries()].filter(([r]) => r !== "tag").sort((a, b) => b[1] - a[1]);
    const links = section("連結類型", true,
      el("div", { class: "g-rels" }, rels.map(([r, c]) => {
        const i = el("input", { type: "checkbox", checked: S.rels[r] !== false });
        i.addEventListener("change", () => { S.rels[r] = i.checked; saveSettings(); refilter(); });
        return el("label", { class: "g-rel" }, i, lineSwatch(relStyle(r)), el("span", {}, r === "mentions" ? "內文提到 [[ ]]" : opts.relLabel?.(r) ?? r), el("em", {}, c));
      })),
      check("有方向的連結畫箭頭", S.arrows, (v) => { S.arrows = v; }, false),
      el("p", { class: "muted small" }, "箭頭方向 = 連結方向：A →先備知識→ B 表示 A 需要先懂 B。"));
    // Appearance
    const colourSel = el("select", { "aria-label": "著色依據" }, [["state", "熟悉度"], ["domain", "領域"], ["tag", "標籤群組"], ["kind", "種類"], ["space", "空間"]].map(([v, l]) => el("option", { value: v, selected: S.colorBy === v }, l)));
    colourSel.addEventListener("change", () => { S.colorBy = colourSel.value; saveSettings(); assignGroups(); for (const n of nodes) n.col = colourOf(n); renderLegend(); renderPanel(); kick(); });
    const look = section("外觀", false,
      el("label", { class: "g-field" }, "著色依據", colourSel),
      slider("節點大小", "nodeSize", 0.5, 2, 0.05, () => kick()),
      slider("文字出現門檻", "labelAt", 0.3, 3, 0.05, () => kick(), (v) => (v <= 0.5 ? "多" : v >= 2 ? "少" : v.toFixed(1))));
    // Forces
    const forces = section("力度", false,
      slider("中心力", "center", 0, 0.3, 0.005, () => reheat(0.3)),
      slider("排斥力", "repel", 5, 600, 5, () => reheat(0.3)),
      slider("連結距離", "linkDist", 10, 250, 1, () => { rebuild(); reheat(0.3); }),
      slider("連結力", "linkForce", 0, 2, 0.05, () => reheat(0.3)),
      el("div", { class: "g-row-btns" },
        el("button", { class: "btn small", type: "button", onclick: () => { Object.assign(S, PHYS); saveSettings(); rebuild(); reheat(0.5); renderPanel(); } }, "重設力度"),
        el("button", { class: "btn small", type: "button", onclick: () => { for (const n of nodes) { n.fx = n.fy = null; } memory.clear(); reheat(0.5); toast("已取消所有固定"); } }, "取消所有固定"),
        el("button", { class: "btn small", type: "button", onclick: () => relayout() }, "重新排列")));
    const head = el("div", { class: "g-panel-h" }, el("b", {}, "圖譜設定"), icoBtn("x", "關閉設定", () => togglePanel(false)));
    const scroll = ui.panel.scrollTop;
    ui.panel.replaceChildren(head, filters, links, look, forces);
    ui.panel.scrollTop = scroll;
  }
  function check(label, on, set, refilterAfter = true) {
    const i = el("input", { type: "checkbox", checked: on });
    i.addEventListener("change", () => { set(i.checked); saveSettings(); if (refilterAfter) refilter(); else kick(); });
    return el("label", { class: "check g-check" }, i, label);
  }
  function slider(label, key, min, max, step, after, fmt) {
    const out = el("output", {}, fmt ? fmt(S[key]) : String(S[key]));
    const i = el("input", { type: "range", min, max, step, value: S[key], "aria-label": label });
    i.addEventListener("input", () => { S[key] = +i.value; out.textContent = fmt ? fmt(S[key]) : String(S[key]); after(); });
    i.addEventListener("change", saveSettings);
    return el("label", { class: "g-slider" }, el("span", {}, label, out), i);
  }
  function tagTree(cards) {
    // Counts include descendants; selecting a parent selects its subtree.
    const cnt = new Map();
    for (const n of cards) {
      const seen = new Set();
      for (const t of n.tags ?? []) { const parts = t.split("/"); for (let i = 1; i <= parts.length; i++) seen.add(parts.slice(0, i).join("/")); }
      for (const t of seen) cnt.set(t, (cnt.get(t) ?? 0) + 1);
    }
    if (!cnt.size) return el("span", { class: "muted small" }, "沒有標籤");
    const kids = (p) => [...cnt.keys()].filter((t) => (p ? t.startsWith(p + "/") && !t.slice(p.length + 1).includes("/") : !t.includes("/"))).sort((a, b) => cnt.get(b) - cnt.get(a) || a.localeCompare(b));
    const item = (t) => {
      const i = el("input", { type: "checkbox", checked: F.tags.has(t) });
      i.addEventListener("change", () => { i.checked ? F.tags.add(t) : F.tags.delete(t); refilter(); });
      const sub = kids(t);
      const row = el("label", { class: "g-tag" }, i, el("span", {}, t.split("/").pop()), el("em", {}, cnt.get(t)));
      if (!sub.length) return el("li", {}, row);
      const open = [...F.tags].some((s) => s.startsWith(t + "/"));
      return el("li", {}, el("details", { open }, el("summary", {}, row), el("ul", {}, sub.map(item))));
    };
    return el("ul", { class: "g-tags" }, kids("").map(item));
  }
  function relayout() {
    memory.clear();
    let i = 0;
    for (const n of nodes) { n.fx = n.fy = null; const r = 12 * Math.sqrt(0.5 + i), a = i * Math.PI * (3 - Math.sqrt(5)); i++; n.x = r * Math.cos(a); n.y = r * Math.sin(a); n.vx = n.vy = 0; }
    stats.loadAt = performance.now(); stats.settleMs = null;
    autoFit = true; reheat(1);
  }

  // ── Timeline playback ──────────────────────────────────
  function togglePlay() {
    if (play?.running) { play.running = false; renderPlay(); return; }
    const ts = nodes.filter((n) => n.isCard && Number.isFinite(n.t)).map((n) => n.t);
    if (!ts.length) { toast("沒有可播放的建立時間"); return; }
    const t0 = Math.min(...ts), t1 = Math.max(...ts);
    if (!play || play.cur >= play.t1) play = { t0, t1, cur: t0, running: true, last: 0 };
    else { play.running = true; play.last = 0; }
    play.dur = Math.max(8000, Math.min(20000, ts.length * 25));
    autoFit = true;
    rebuild(); alphaTarget = 0.12; reheat(0.4);
    renderPlay();
  }
  function stopPlay() {
    play = null; alphaTarget = 0;
    rebuild(); reheat(0.2); renderPlay();
  }
  function advancePlay(ts) {
    if (!play.last) play.last = ts;
    const dt = ts - play.last; play.last = ts;
    const before = play.cur;
    play.cur = Math.min(play.t1, play.cur + (play.t1 - play.t0) * dt / play.dur);
    if (play.cur >= play.t1) { play.running = false; alphaTarget = 0; renderPlay(); }
    if (visibleAt(play.cur) !== visibleAt(before)) { rebuild(); if (alpha < 0.12) alpha = 0.12; }
    updatePlayBar();
  }
  function visibleAt(t) { // cards created at or before t (binary search)
    let lo = 0, hi = playTimes.length;
    while (lo < hi) { const m = (lo + hi) >> 1; if (playTimes[m] <= t) lo = m + 1; else hi = m; }
    return lo;
  }
  function renderPlay() {
    ui.playBtn.replaceChildren(ico(play?.running ? "pause" : "play"));
    ui.playBtn.setAttribute("aria-label", play?.running ? "暫停播放" : "播放：看知識怎麼長出來");
    if (!play) { ui.play.hidden = true; return; }
    ui.scrub = el("input", { type: "range", min: 0, max: 1000, step: 1, "aria-label": "時間" });
    ui.scrub.addEventListener("input", () => { play.running = false; play.cur = play.t0 + (play.t1 - play.t0) * (+ui.scrub.value / 1000); rebuild(); reheat(0.2); renderPlayBtn(); updatePlayBar(); });
    ui.playDate = el("span", { class: "g-play-d mono" });
    ui.playCount = el("span", { class: "g-play-c muted small" });
    ui.play.replaceChildren(
      icoBtn(play.running ? "pause" : "play", play.running ? "暫停" : "繼續", () => togglePlay()),
      el("div", { class: "g-play-m" }, el("div", { class: "g-play-t" }, el("b", {}, "看知識怎麼長出來"), ui.playDate, ui.playCount), ui.scrub),
      icoBtn("x", "結束播放", () => stopPlay()));
    ui.play.hidden = false;
    updatePlayBar();
  }
  function renderPlayBtn() { ui.playBtn.replaceChildren(ico(play?.running ? "pause" : "play")); const b = ui.play.querySelector(".ico"); if (b) { b.replaceChildren(ico(play?.running ? "pause" : "play")); b.setAttribute("aria-label", play?.running ? "暫停" : "繼續"); } }
  function updatePlayBar() {
    if (!play || !ui.scrub) return;
    ui.scrub.value = String(Math.round(1000 * (play.cur - play.t0) / Math.max(1, play.t1 - play.t0)));
    const d = new Date(play.cur);
    ui.playDate.textContent = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    ui.playCount.textContent = `${active.filter((n) => n.isCard).length} 張`;
  }

  // ── Debug / measurement ────────────────────────────────
  /** Keep the simulation hot for `ms` and report the frame rate (for perf checks). */
  function bench(ms = 3000) {
    return new Promise((done) => {
      const f0 = stats.frames, t0 = performance.now();
      alphaTarget = 0.3; reheat(0.3);
      setTimeout(() => {
        const fps = (stats.frames - f0) * 1000 / (performance.now() - t0);
        alphaTarget = 0;
        done({ fps: Math.round(fps * 10) / 10, tickMs: +stats.tickMs.toFixed(2), drawMs: +stats.drawMs.toFixed(2), nodes: active.length, edges: activeEdges.length });
      }, ms);
    });
  }

  /** Synchronous cost of one simulation tick and one full redraw, in ms (works even when
   *  the page is in the background and requestAnimationFrame is throttled). */
  function measure(n = 60) {
    const saved = alpha;
    alpha = Math.max(alpha, 0.3);
    let t0 = performance.now();
    for (let i = 0; i < n; i++) tick();
    const tickMs = (performance.now() - t0) / n;
    t0 = performance.now();
    for (let i = 0; i < n; i++) draw();
    const drawMs = (performance.now() - t0) / n;
    alpha = saved;
    const perFrame = Math.max(1, Math.min(12, Math.floor(9 / tickMs)));
    return { nodes: active.length, edges: activeEdges.length, tickMs: +tickMs.toFixed(3), drawMs: +drawMs.toFixed(3), ticksPerFrame: perFrame,
      estFps: Math.min(60, Math.round(1000 / (perFrame * tickMs + drawMs))), estSettleMs: Math.round(300 / perFrame * Math.max(1000 / 60, perFrame * tickMs + drawMs)) };
  }

  return {
    init, show, hide, measure, draw: () => { readPalette(); renderLegend(); kick(); },
    stats: () => ({ ...stats, alpha, active: active.length, activeEdges: activeEdges.length, view: { ...view } }),
    bench, fit: () => fit(true), select: (id) => select(byId.get(id) ?? null), hover: (id, x, y) => setHover(byId.get(id) ?? null, x != null ? { x, y } : null),
    /** Screen position of a node by id or title (tests). */
    where: (q) => { const n = byId.get(q) ?? nodes.find((m) => m.title === q); return n && { id: n.id, x: n.x * view.k + view.x, y: n.y * view.k + view.y, on: n.on }; },
    togglePanel, togglePlay, setQuery: (q) => { ui.search.value = q; setQuery(q); },
  };
})();
