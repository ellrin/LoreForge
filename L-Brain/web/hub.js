// The hub's screens (docs/brain-hub.md §8): the cover and a family's page, over /api/hub/*.
//   #/hub                 the cover: a dashboard, every part opens what it measures
//   #/hub/<family id>     a family: tags | one row per item (or a wall of drawings) | the detail
// app.js owns the routes and the left navigation's frame; this file draws inside
// <section data-view="hub"> and fills <ol id="hubNav">.
const Hub = (() => {
  const LV = { figure: "流程圖", module: "模塊", component: "元件" };
  const NEWEST = new Set(["created", "updated", "used", "used_at"]);
  const S = { root: null, stats: null, cards: null, meta: null, fam: null, items: [], total: 0, tags: {}, sel: null, f: null, host: {} };
  const el = (tag, attrs = {}, ...kids) => {
    const e = document.createElementNS(tag === "svg" || attrs?.ns ? "http://www.w3.org/2000/svg" : "http://www.w3.org/1999/xhtml", tag);
    for (const [k, v] of Object.entries(attrs ?? {})) {
      if (v == null || v === false || k === "ns") continue;
      if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else if (k === "class") e.setAttribute("class", v);
      else if (k === "html") e.innerHTML = v;
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat(Infinity)) if (c != null && c !== false) e.append(c instanceof Node ? c : document.createTextNode(String(c)));
    return e;
  };
  const sv = (tag, attrs, ...kids) => el(tag, { ...attrs, ns: 1 }, ...kids);
  const fmt = (n) => Number(n ?? 0).toLocaleString("en-US");
  const enc = encodeURIComponent;
  async function api(method, path, body) {
    const r = await fetch(path, { method, headers: body ? { "content-type": "application/json" } : {}, body: body ? JSON.stringify(body) : undefined });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw Object.assign(new Error(j.error ?? `${r.status}`), { status: r.status, body: j });
    return j;
  }
  const colour = (i) => (i < 8 ? `var(--hf${i + 1})` : "var(--faint)");
  const famHash = (f) => (f.app === "kb" ? "#/cards" : `#/hub/${enc(f.id)}`);

  // one tooltip for every mark that carries data-tip
  let tip = null;
  function tips(node) {
    node.addEventListener("mousemove", (e) => {
      const t = e.target.closest?.("[data-tip]");
      if (!t) { if (tip) tip.hidden = true; return; }
      tip ??= document.body.appendChild(el("div", { class: "h-tip" }));
      tip.textContent = t.dataset.tip; tip.hidden = false;
      tip.style.left = `${Math.min(e.clientX + 12, innerWidth - tip.offsetWidth - 8)}px`; tip.style.top = `${e.clientY + 14}px`;
    });
    node.addEventListener("mouseleave", () => { if (tip) tip.hidden = true; });
  }

  async function loadStats() {
    const r = await api("GET", "/api/hub/stats");
    S.stats = r.stats; S.cards = r.cards ?? {};
    nav();
    return r;
  }
  async function loadMeta() { S.meta ??= await api("GET", "/api/hub/categories"); return S.meta; }

  /** The families under the app's own views in the left column. */
  function nav() {
    const ol = document.getElementById("hubNav");
    if (!ol || !S.stats) return;
    const cur = S.fam?.id;
    ol.replaceChildren(...S.stats.families.filter((f) => f.app !== "kb").map((f) =>
      el("li", {}, el("button", { type: "button", "aria-current": f.id === cur ? "page" : null, onclick: () => { location.hash = famHash(f); } },
        el("span", {}, f.label), el("span", { class: "count" }, f.count || "")))));
  }

  // ── the cover ──
  const ym = (iso) => iso.slice(0, 7).replace("-", "/");
  const weekTip = (iso) => `${iso.replaceAll("-", "/")} 那一週`;
  function cover() {
    const st = S.stats, fams = st.families, total = st.total || fams.reduce((n, f) => n + f.count, 0);
    const ws = st.week_starts ?? [], nW = ws.length || 12;
    // a month is named once, where it begins
    const marks = []; ws.forEach((d, i) => { const m = ym(d); if (!marks.length || marks.at(-1)[1] !== m) marks.push([i, m]); });
    const R = 138, C = 2 * Math.PI * R, GAP = 5; let acc = 0;
    const ring = sv("svg", { viewBox: "0 0 350 350" },
      Array.from({ length: 60 }, (_, i) => { const a = (i / 60) * 2 * Math.PI, r1 = 158, r2 = i % 5 ? 162 : 167; return sv("line", { x1: 175 + r1 * Math.cos(a), y1: 175 + r1 * Math.sin(a), x2: 175 + r2 * Math.cos(a), y2: 175 + r2 * Math.sin(a), stroke: "var(--line-strong)", "stroke-width": 1 }); }),
      total ? fams.map((f, i) => { const len = (f.count / total) * C; const c = sv("circle", { cx: 175, cy: 175, r: R, fill: "none", stroke: colour(i), "stroke-width": 20, "stroke-dasharray": `${Math.max(len - GAP, 1)} ${C}`, "stroke-dashoffset": -acc, transform: "rotate(-90 175 175)", "data-tip": `${f.label} ${fmt(f.count)}（${Math.round((f.count / total) * 100)}%）` }); acc += len; return f.count ? c : null; })
        : sv("circle", { cx: 175, cy: 175, r: R, fill: "none", stroke: "var(--surface)", "stroke-width": 20 }));
    // additions per week, the outside sets left out (one import would flatten everything else)
    const wk = Array.from({ length: nW }, (_, i) => fams.filter((f) => !f.independent).reduce((n, f) => n + (f.weeks?.[i] ?? 0), 0));
    const mx = Math.max(...wk, 1), X = (i) => (nW > 1 ? i * (312 / (nW - 1)) : 0), Y = (v) => 70 - (v / mx) * 60;
    const pts = wk.map((v, i) => `${X(i)},${Y(v)}`).join(" ");
    const line = sv("svg", { viewBox: "-4 0 320 94", style: "width:100%;display:block;overflow:visible" },
      sv("line", { x1: 0, x2: 312, y1: 70, y2: 70, stroke: "var(--line)" }),
      sv("polyline", { points: `0,70 ${pts} 312,70`, fill: "var(--accent-2-soft)", stroke: "none" }),
      sv("polyline", { points: pts, fill: "none", stroke: "var(--accent-2)", "stroke-width": 2, "stroke-linejoin": "round" }),
      marks.map(([i, m]) => [sv("line", { x1: X(i), x2: X(i), y1: 70, y2: 75, stroke: "var(--line-strong)" }), sv("text", { class: "h-ym", x: X(i), y: 89, "text-anchor": i === 0 ? "start" : "middle" }, m)]),
      wk.map((v, i) => sv("circle", { cx: X(i), cy: Y(v), r: 7, fill: "transparent", "data-tip": `${weekTip(ws[i] ?? "")} +${v}` })));
    const spark = (f, i) => { const v = f.weeks ?? [], m2 = Math.max(...v, 1); return sv("svg", { class: "h-sp", viewBox: "0 0 150 30" }, v.map((x, k) => { const hh = Math.max((x / m2) * 28, x ? 2 : 1); return sv("rect", { x: k * 12.5, y: 30 - hh, width: 9, height: hh, rx: 1.5, fill: x ? colour(i) : "var(--line)", "data-tip": `${weekTip(ws[k] ?? "")} +${x}` }); })); };
    const big = Math.max(...fams.map((f) => f.count), 1);
    const row = (f, i) => el("tr", { class: "r", onclick: () => { location.hash = famHash(f); } },
      el("td", { class: "nm" }, el("i", { style: `background:${colour(i)}` }), f.label),
      el("td", { class: "n" }, fmt(f.count)),
      el("td", {}, el("div", { class: "sb", style: `width:${Math.max((f.count / big) * 100, 1.5)}%;background:${colour(i)}`, "data-tip": total ? `${Math.round((f.count / total) * 100)}%` : null }),
        el("div", { class: "sub" }, (f.parts ?? []).filter((p) => p.count).map((p) => `${p.label} ${fmt(p.count)}`).join(" · "))),
      el("td", {}, spark(f, i)),
      el("td", { class: "use num" }, f.used_30d ? fmt(f.used_30d) : "—"),
      el("td", { class: `gap ${f.gap ? "" : "ok"}`, title: f.gap || null }, f.gap || "—"));
    const node = el("div", { class: "h-cover" },
      el("div", { class: "h-col" },
        el("div", { class: "h-box" }, el("div", { class: "h-cap" }, "組成", el("span", {}, `${fams.length} 類`)),
          el("div", { class: "h-ring" }, ring, el("div", { class: "mid" }, el("div", {}, el("b", {}, fmt(total)), el("span", {}, "條知識"))))),
        el("div", { class: "h-box" }, el("div", { class: "h-cap" }, "每週新增"), line),
        el("div", { class: "h-two" },
          el("a", { class: `h-box h-stat ${st.proposals ? "hot" : ""}`, href: "#/hub/proposals" }, el("div", { class: "h-cap" }, "待你同意"), el("b", {}, st.proposals ?? 0), el("span", {}, "批")),
          el("a", { class: "h-box h-stat", href: "#/review" }, el("div", { class: "h-cap" }, "卡到期"), el("b", {}, S.cards.due ?? 0), el("span", {}, "張")))),
      el("div", { class: "h-box", style: "padding:16px 18px 6px" },
        el("table", { class: "h-table" },
          el("tr", {}, el("th", {}, "類別"), el("th", { style: "text-align:right;padding-right:22px" }, "數量"), el("th", {}, "佔比與細項"),
            el("th", {}, "新增", marks.length ? el("span", { class: "h-ym" }, marks.length > 1 ? `${marks[0][1]} – ${marks.at(-1)[1]}` : marks[0][1]) : null),
            el("th", { style: "text-align:right" }, "30 天用到"), el("th", { style: "text-align:right;padding-right:6px" }, "缺口")),
          fams.map(row))));
    tips(node);
    return node;
  }

  // ── a family's page ──
  const isFigure = (f) => (f.parts ?? []).some((p) => Object.values(LV).includes(p.label) || p.label === "待製作");
  const prefKey = (f) => `kb.hub.mode.${f.id}`;
  function defaults(f) {
    let mode = null; try { mode = localStorage.getItem(prefKey(f)); } catch {}
    return { q: "", sort: "updated", level: "", entity: "", tags: new Set(), source: "", mode: mode ?? (isFigure(f) ? "wall" : "list") };
  }
  async function loadItems() {
    const f = S.f, p = new URLSearchParams({ family: S.fam.id, sort: f.sort, limit: "1000" });
    if (f.q) p.set("q", f.q);
    if (f.level) p.set("level", f.level);
    if (f.entity) p.set("entity", f.entity);
    if (f.source) p.set("source", f.source);
    for (const t of f.tags) p.append("tag", t);
    const r = await api("GET", `/api/hub/items?${p}`);
    S.items = r.items ?? []; S.total = r.total ?? S.items.length;
    // the tags of what is listed (this family, these filters), so a tag always leads somewhere
    S.tags = {};
    for (const it of S.items) for (const [facet, ts] of Object.entries(it.tags ?? {})) for (const t of ts) { S.tags[facet] ??= {}; S.tags[facet][t] = (S.tags[facet][t] ?? 0) + 1; }
    for (const k of S.f.tags) { const [facet, ...rest] = k.split(":"); S.tags[facet] ??= {}; S.tags[facet][rest.join(":")] ??= 0; }
  }
  const thumb = (it, cls = "") => (it.thumb ? el("span", { class: `h-paper ${cls}` }, el("img", { src: it.thumb, alt: "", loading: "lazy" })) : it.format === "figure" || it.format === "layout" ? el("span", { class: `h-paper none ${cls}` }, it.entity === false ? "待製作" : "") : null);
  const catLabel = (id) => S.meta?.categories.find((c) => c.id === id)?.label ?? id;
  const chips = (it) => [it.entity === false ? el("span", { class: "h-tag todo" }, "待製作") : null,
    it.source?.kind === "external" ? el("span", { class: "h-tag src" }, it.source.name) : null,
    ...Object.values(it.tags ?? {}).flat().slice(0, 3).map((t) => el("span", { class: "h-tag" }, t))];
  function groups() {
    const fig = isFigure(S.fam), out = new Map();
    for (const it of S.items) {
      const k = fig ? (it.entity === false ? "todo" : it.level || "other") : it.category;
      (out.get(k) ?? out.set(k, []).get(k)).push(it);
    }
    const name = (k) => (fig ? (k === "todo" ? "待製作" : LV[k] ?? "其他") : catLabel(k));
    const order = fig ? ["figure", "module", "component", "other", "todo"] : S.fam.categories;
    return [...out.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0])).map(([k, v]) => ({ key: k, label: name(k), items: v }));
  }
  function listBody() {
    const g = groups();
    if (!S.items.length) return el("div", { class: "h-empty" }, S.f.q || S.f.tags.size || S.f.level || S.f.entity ? "沒有符合條件的知識" : "這一類還沒有知識");
    if (S.f.mode === "wall") return el("div", {}, g.map((x) => [el("div", { class: "h-wh" }, x.label, el("small", {}, x.items.length)),
      el("div", { class: `h-wall ${x.key === "figure" ? "lg" : ""}` }, x.items.map((it) => el("button", { type: "button", class: "h-pc", "aria-selected": String(S.sel === it.id), onclick: () => open(it.id) }, thumb(it) ?? el("span", { class: "h-paper none" }), el("div", { class: "cap" }, el("b", {}, `${x.label}｜`), it.title))))]));
    return el("div", {}, g.map((x, i) => el("details", { class: "h-grp", open: g.length === 1 || i === 0 || x.items.some((it) => it.id === S.sel) },
      el("summary", {}, x.label, el("small", {}, x.items.length)),
      x.items.map((it) => { const th = thumb(it); return el("button", { type: "button", class: `h-row ${th ? "" : "text"}`, "aria-selected": String(S.sel === it.id), onclick: () => open(it.id) }, th, el("span", { class: "t" }, it.title), el("span", { class: "d" }, it.line ?? ""), el("span", { class: "g" }, chips(it))); }))));
  }
  function tagsPane() {
    const f = S.f, out = [];
    const btn = (label, n, on, click) => el("button", { type: "button", "aria-pressed": String(!!on), onclick: click }, label, el("span", {}, n ?? ""));
    if (isFigure(S.fam)) {
      const parts = Object.fromEntries((S.fam.parts ?? []).map((p) => [p.label, p.count]));
      out.push(el("h4", {}, "階層"), ...Object.entries(LV).map(([k, l]) => btn(l, parts[l], f.level === k, () => { f.level = f.level === k ? "" : k; f.entity = ""; refresh(); })),
        btn("待製作", parts["待製作"], f.entity === "false", () => { f.entity = f.entity === "false" ? "" : "false"; f.level = ""; refresh(); }));
    }
    const FACET = { domain: "領域", kind: "結構", use: "用途" };
    for (const [facet, tags] of Object.entries(S.tags)) {
      const list = Object.entries(tags).sort((a, b) => b[1] - a[1]);
      if (!list.length) continue;
      out.push(el("h4", {}, FACET[facet] ?? facet), ...list.map(([t, n]) => { const k = `${facet}:${t}`; return btn(t, n, f.tags.has(k), () => { f.tags.has(k) ? f.tags.delete(k) : f.tags.add(k); refresh(); }); }));
    }
    return el("aside", { class: "h-tags" }, out.length ? out : el("h4", {}, "還沒有標籤"));
  }
  function head() {
    const f = S.f, sorts = S.meta?.sorts ?? [["updated", "最近修改"], ["title", "名稱"]];
    let timer = 0;
    return el("div", { class: "h-head" }, el("h2", {}, S.fam.label), el("span", { class: "cnt" }, fmt(S.total)), el("span", { class: "sp" }),
      el("input", { type: "search", placeholder: "在這一類裡找", value: f.q, "aria-label": "在這一類裡找", oninput: (e) => { clearTimeout(timer); timer = setTimeout(() => { f.q = e.target.value.trim(); refresh(true); }, 250); } }),
      el("select", { "aria-label": "排序", onchange: (e) => { f.sort = e.target.value; refresh(); } }, sorts.map(([id, label]) => el("option", { value: id, selected: id === f.sort }, label))),
      el("span", { class: "h-sw", role: "group", "aria-label": "顯示方式" }, [["list", "清單"], ["wall", "圖"]].map(([m, t]) => el("button", { type: "button", "aria-pressed": String(f.mode === m), onclick: () => { f.mode = m; try { localStorage.setItem(prefKey(S.fam), m); } catch {} paint(); } }, t))));
  }
  let detailEl = null;
  function paint(keepFocus) {
    const q = keepFocus ? S.root.querySelector(".h-head input") : null, pos = q?.selectionStart;
    const cat = el("div", { class: `h-cat ${S.sel ? "open" : ""}` }, tagsPane(), el("div", { class: "h-mid" }, head(), listBody()), S.sel ? (detailEl ?? el("aside", { class: "h-detail" })) : null);
    S.root.replaceChildren(el("div", { class: "hubv" }, cat));
    if (q) { const n = S.root.querySelector(".h-head input"); n.focus(); n.setSelectionRange(pos, pos); }
  }
  async function refresh(keepFocus) { try { await loadItems(); paint(keepFocus); } catch (e) { S.host.fail?.(e); } }

  // ── the detail to the right ──
  async function open(id) {
    S.sel = id;
    try {
      const d = await api("GET", `/api/hub/items/${enc(id)}`), it = d.item;
      const def = S.meta?.formats.find((x) => x.id === it.format);
      const FACET = { domain: "領域", kind: "結構", use: "用途" };
      const link = (r) => el("button", { type: "button", class: "h-lk", onclick: () => open(r.id) }, thumb(r), r.title);
      const fields = (def?.fields ?? []).filter((fd) => { const v = it.fields?.[fd.name]; return v != null && v !== "" && typeof v !== "object"; });
      detailEl = el("aside", { class: "h-detail" },
        el("div", { class: "hd" }, el("h3", {}, it.title), el("button", { type: "button", class: "x", "aria-label": "關閉", onclick: () => { S.sel = null; detailEl = null; paint(); } }, "✕")),
        el("div", { class: "ln" }, [catLabel(it.category), it.line].filter(Boolean).join(" · ")),
        it.thumb ? el("span", { class: "h-paper stage" }, el("img", { src: it.thumb, alt: "" })) : null,
        fields.length ? el("dl", {}, fields.map((fd) => [el("dt", {}, fd.label), el("dd", {}, String(it.fields[fd.name]))])) : null,
        Object.entries(it.tags ?? {}).map(([facet, ts]) => [el("div", { class: "k" }, FACET[facet] ?? facet), el("div", { class: "chips" }, ts.map((t) => el("span", { class: "h-tag" }, t)))]),
        it.triggers?.length ? [el("div", { class: "k" }, "觸發關鍵字"), el("div", { class: "chips" }, it.triggers.map((t) => el("span", { class: "h-tag" }, t)))] : null,
        d.contains?.length ? [el("div", { class: "k" }, "由這些組成"), d.contains.map(link)] : null,
        d.used_by?.length ? [el("div", { class: "k" }, "用在"), d.used_by.map(link)] : null,
        el("div", { class: "k" }, "來源"), el("div", { class: "chips" }, el("span", { class: "h-tag" }, it.source?.kind === "external" ? it.source.name : it.source?.kind === "author" ? "我" : "Lore"), it.used ? el("span", { class: "h-tag" }, `用過 ${it.used} 次`) : null),
        el("div", { class: "h-acts" }, el("button", { type: "button", onclick: () => remove(it) }, "移除")));
      paint();
    } catch (e) { S.sel = null; detailEl = null; S.host.fail?.(e); }
  }
  async function remove(it) {
    try {
      await api("DELETE", `/api/hub/items/${enc(it.id)}`);
      S.sel = null; detailEl = null;
      S.host.toast?.(`「${it.title}」已移除`, { label: "復原", run: async () => { try { await api("POST", `/api/hub/items/${enc(it.id)}/restore`); await loadStats(); await refresh(); } catch (e) { S.host.fail?.(e); } } });
      await loadStats(); await refresh();
    } catch (e) { S.host.fail?.(e); }
  }

  /** Draw the hub for `arg` (undefined: the cover; a family id: its page). */
  async function show(root, arg) {
    S.root = root;
    try {
      await Promise.all([loadStats(), loadMeta()]);
      const fam = arg ? S.stats.families.find((f) => f.id === arg) : null;
      if (!fam) { S.fam = null; S.sel = null; detailEl = null; nav(); root.replaceChildren(el("div", { class: "hubv" }, cover())); return; }
      if (S.fam?.id !== fam.id) { S.f = defaults(fam); S.sel = null; detailEl = null; }
      S.fam = fam; nav();
      await loadItems(); paint();
    } catch (e) {
      root.replaceChildren(el("div", { class: "hubv" }, el("div", { class: "h-empty" }, e.message || "知識中樞現在打不開")));
    }
  }
  return { show, init: (host) => { S.host = host ?? {}; }, refreshNav: () => loadStats().catch(() => {}) };
})();
