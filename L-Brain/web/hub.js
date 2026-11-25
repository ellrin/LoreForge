// The hub's screens (docs/brain-hub.md §8): the cover and a family's page, over /api/hub/*.
//   #/hub                 the cover: a dashboard, every part opens what it measures
//   #/hub/<family id>     a family: tags | one row per item (or a wall of drawings) | the detail
// app.js owns the routes and the left navigation's frame; this file draws inside
// <section data-view="hub"> and fills <ol id="hubNav">.
const Hub = (() => {
  const LV = { figure: "流程圖", module: "模塊", component: "元件" };
  const OP = { add: "新增", update: "修改", remove: "移除" };
  const TREE_FORMATS = new Set(["pattern", "expression", "exemplar"]);
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
    S.stats = r.stats; S.cards = r.cards ?? {}; S.inbox = r.inbox ?? {}; S.review = r.review ?? {};
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
  function cover(sources) {
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
        S.review?.open ? el("a", { class: "h-box h-cta", href: "#/hub/review" }, el("div", {}, el("div", { class: "h-cap" }, "等你決定"), el("b", {}, fmt(S.review.open)), el("span", {}, "個說法")), el("span", { class: "go" }, "開始看")) : null,
        el("div", { class: "h-box" }, el("div", { class: "h-cap" }, "組成", el("span", {}, `${fams.length} 類`)),
          el("div", { class: "h-ring" }, ring, el("div", { class: "mid" }, el("div", {}, el("b", {}, fmt(total)), el("span", {}, "條知識"))))),
        el("div", { class: "h-box" }, el("div", { class: "h-cap" }, "每週新增"), line),
        el("div", { class: "h-two" },
          el("a", { class: `h-box h-stat ${st.proposals ? "hot" : ""}`, href: "#/hub/proposals" }, el("div", { class: "h-cap" }, "待你同意"), el("b", {}, st.proposals ?? 0), el("span", {}, "批")),
          el("a", { class: `h-box h-stat ${S.inbox?.ready ? "hot" : ""}`, href: "#/hub/inbox" }, el("div", { class: "h-cap" }, "收件", (S.inbox?.waiting ?? 0) + (S.inbox?.working ?? 0) ? el("span", {}, `處理中 ${(S.inbox.waiting ?? 0) + (S.inbox.working ?? 0)}`) : null), el("b", {}, S.inbox?.ready ?? 0), el("span", {}, "句")),
          el("a", { class: "h-box h-stat", href: "#/review" }, el("div", { class: "h-cap" }, "卡到期"), el("b", {}, S.cards.due ?? 0), el("span", {}, "張")))),
      el("div", { class: "h-col" }, el("div", { class: "h-box", style: "padding:16px 18px 6px" },
        el("table", { class: "h-table" },
          el("tr", {}, el("th", {}, "類別"), el("th", { style: "text-align:right;padding-right:22px" }, "數量"), el("th", {}, "佔比與細項"),
            el("th", {}, "新增", marks.length ? el("span", { class: "h-ym" }, marks.length > 1 ? `${marks[0][1]} – ${marks.at(-1)[1]}` : marks[0][1]) : null),
            el("th", { style: "text-align:right" }, "30 天用到"), el("th", { style: "text-align:right;padding-right:6px" }, "缺口")),
          fams.map(row))), sourcesBox(sources)));
    tips(node);
    return node;
  }

  // ── a family's page ──
  const isFigure = (f) => (f.parts ?? []).some((p) => Object.values(LV).includes(p.label) || p.label === "待製作");
  const prefKey = (f) => `kb.hub.mode.${f.id}`;
  function defaults(f) {
    let mode = null; try { mode = localStorage.getItem(prefKey(f)); } catch {}
    return { q: "", sort: "updated", level: "", entity: "", section: "", place: "", openPlaces: new Set(), more: new Set(), tags: new Set(), source: "", mode: mode ?? (isFigure(f) ? "wall" : "list") };
  }
  // ── 知識卡: the same page over L-Brain's own cards (/api/nodes), which carry a learning state ──
  const STATE = { known: "記得", learning: "學習中", stale: "久未複習", none: "尚無紀錄" };
  const isCards = () => S.fam?.app === "kb";
  async function loadCards() {
    const f = S.f;
    S.reg ??= await api("GET", "/api/registry");
    const sort = { updated: "updated", created: "created", title: "title", todo: "score" }[f.sort] ?? "updated";
    const p = new URLSearchParams({ limit: "5000", sort });
    if (f.q) p.set("q", f.q);
    const nodes = (await api("GET", `/api/nodes?${p}`)).nodes ?? [];
    const dl = (d) => S.reg.domains?.[d]?.label ?? d, kl = (k) => S.reg.kinds?.find((x) => x.id === k)?.label ?? k;
    const all = nodes.map((n) => ({ id: n.id, card: true, title: n.title, line: n.summary ?? "", category: n.domains?.[0] ? dl(n.domains[0]) : "未分類", format: "card", entity: true, state: n.state ?? "none", node: n,
      tags: { domain: (n.domains ?? []).map(dl), state: [STATE[n.state ?? "none"] ?? n.state], ckind: [kl(n.kind)] } }));
    S.tags = {};
    for (const it of all) for (const [facet, ts] of Object.entries(it.tags)) for (const t of ts) { S.tags[facet] ??= {}; S.tags[facet][t] = (S.tags[facet][t] ?? 0) + 1; }
    S.sections = {};
    S.items = all.filter((it) => { const by = {}; for (const k of f.tags) { const [facet, ...r] = k.split(":"); (by[facet] ??= []).push(r.join(":")); } return Object.entries(by).every(([facet, ts]) => ts.some((t) => it.tags[facet]?.includes(t))); });
    S.total = S.items.length;
  }
  async function loadItems() {
    if (isCards()) return loadCards();
    const f = S.f, p = new URLSearchParams({ family: S.fam.id, sort: f.sort, limit: "1000" });
    if (f.q) p.set("q", f.q);
    if (f.level) p.set("level", f.level);
    if (f.entity) p.set("entity", f.entity);
    if (f.source) p.set("source", f.source);
    for (const t of f.tags) p.append("tag", t);
    const r = await api("GET", `/api/hub/items?${p}`);
    S.items = r.items ?? []; S.total = r.total ?? S.items.length;
    if (f.section) S.items = S.items.filter((it) => it.fields?.section === f.section);
    // what was made from a reference stands for it: the reference itself leaves the lists
    if (S.made?.size) { S.items = S.items.filter((it) => !S.made.has(it.id)); S.total = S.items.length; }
    // a place on the family's tree: everything at or under that path; "?" = not placed yet
    if (f.place === "?") S.items = S.items.filter((it) => TREE_FORMATS.has(it.format) && !(it.tags?.place ?? []).length);
    else if (f.place?.startsWith("=")) S.items = S.items.filter((it) => it.category === f.place.slice(1));
    else if (f.place) S.items = S.items.filter((it) => (it.tags?.place ?? []).some((p) => p === f.place || p.startsWith(`${f.place}/`)));
    if (f.place || f.section) S.total = S.items.length;
    // the left column lists the whole family's tags, whatever is filtered: a tag never disappears
    if (S.allFor !== S.fam.id || S.allDirty) {
      const all = (await api("GET", `/api/hub/items?${new URLSearchParams({ family: S.fam.id, limit: "1000" })}`)).items ?? [];
      S.tags = {}; S.sections = {}; S.allItems = all;
      // a reference a figure was made from (reference id → the figure): no longer something to make
      S.made = new Map(all.filter((it) => it.fields?.made_from).map((it) => [it.fields.made_from, it]));
      S.tree = (await api("GET", `/api/hub/tree?family=${enc(S.fam.id)}`).catch(() => ({}))).tree ?? null;
      for (const it of all) {
        for (const [facet, ts] of Object.entries(it.tags ?? {})) for (const t of ts) { S.tags[facet] ??= {}; S.tags[facet][t] = (S.tags[facet][t] ?? 0) + 1; }
        const sec = it.fields?.section; if (typeof sec === "string" && sec) S.sections[sec] = (S.sections[sec] ?? 0) + 1;
      }
      S.allFor = S.fam.id; S.allDirty = false;
    }
  }
  const thumb = (it, cls = "") => (it.thumb ? el("span", { class: `h-paper ${cls}` }, el("img", { src: it.thumb, alt: "", loading: "lazy" })) : it.format === "figure" || it.format === "layout" ? el("span", { class: `h-paper none ${cls}` }, it.entity === false ? "待製作" : "") : null);
  const catLabel = (id) => S.meta?.categories.find((c) => c.id === id)?.label ?? id;
  const chips = (it) => [it.card && it.state !== "known" ? el("span", { class: `h-tag st-${it.state}` }, STATE[it.state] ?? "") : null,
    it.entity === false ? el("span", { class: "h-tag todo" }, "待製作") : null,
    it.source?.kind === "external" ? el("span", { class: "h-tag src" }, it.source.name) : null];
  function groups() {
    const fig = isFigure(S.fam), out = new Map();
    for (const it of S.items) {
      const k = fig ? (it.entity === false ? "todo" : it.level || "other") : it.category;
      (out.get(k) ?? out.set(k, []).get(k)).push(it);
    }
    const name = (k) => (fig ? (k === "todo" ? "待製作" : LV[k] ?? "其他") : catLabel(k));
    if (S.tree?.tree?.length && S.f.place && S.f.place !== "?" && !S.f.place.startsWith("=") && !isCards()) {
      // inside a place: one group per next level down, in the tree's order
      const depth = S.f.place.split("/").length, by = new Map();
      for (const it of S.items) { const p = (it.tags?.place ?? []).find((x) => x === S.f.place || x.startsWith(`${S.f.place}/`)) ?? ""; const k = p.split("/").slice(0, depth + 1).join("/"); (by.get(k) ?? by.set(k, []).get(k)).push(it); }
      return [...by.entries()].map(([k, v]) => ({ key: k, label: k.split("/").slice(depth).join(" › ") || k.split("/").at(-1), items: v }));
    }
    if (S.tree?.tree?.length && !S.f.place && !fig && !isCards()) {
      // nothing chosen yet: the tree's sections, one line each, then the kinds that are not sentences
      const by = new Map();
      for (const it of S.items) { const p = it.tags?.place?.[0]; const k = p ? p.split("/")[0] : TREE_FORMATS.has(it.format) ? "?" : `=${it.category}`; (by.get(k) ?? by.set(k, []).get(k)).push(it); }
      const order = [...S.tree.tree.map((n) => n.label), "?"];
      return [...by.entries()].sort((a, b) => (order.indexOf(a[0]) < 0 ? 99 : order.indexOf(a[0])) - (order.indexOf(b[0]) < 0 ? 99 : order.indexOf(b[0])))
        .map(([k, v]) => ({ key: k, label: k === "?" ? "還沒歸位" : k.startsWith("=") ? catLabel(k.slice(1)) : k, items: v, folded: true }));
    }
    if (isCards()) return [...out.entries()].sort((a, b) => (a[0] === "未分類") - (b[0] === "未分類") || b[1].length - a[1].length).map(([k, v]) => ({ key: k, label: k, items: v }));
    const order = fig ? ["figure", "module", "component", "other", "todo"] : S.fam.categories;
    return [...out.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0])).map(([k, v]) => ({ key: k, label: name(k), items: v }));
  }
  function listBody() {
    const g = groups();
    if (!S.items.length) return el("div", { class: "h-empty" }, S.f.q || S.f.tags.size || S.f.level || S.f.entity ? "沒有符合條件的知識" : "這一類還沒有知識");
    if (S.f.mode === "wall") return el("div", {}, g.map((x) => [el("div", { class: "h-wh" }, x.label, el("small", {}, x.items.length)),
      el("div", { class: `h-wall ${x.key === "figure" ? "lg" : ""}` }, x.items.map((it) => el("button", { type: "button", class: "h-pc", "aria-selected": String(S.sel === it.id), onclick: () => open(it.id) }, thumb(it) ?? el("span", { class: "h-paper none" }), el("div", { class: "cap" }, el("b", {}, `${x.label}｜`), it.title))))]));
    // the quiet word beside a row: on the tree, where the sentence sits below what is chosen (its
    // old section · function · position say less than that); elsewhere the format's own line
    const side = (it) => {
      const p = it.tags?.place?.[0];
      if (!S.tree?.tree?.length || !TREE_FORMATS.has(it.format) || !p) return S.tree?.tree?.length && TREE_FORMATS.has(it.format) ? "" : it.line ?? "";
      const chosen = S.f.place && !"?=".includes(S.f.place[0]) ? (it.tags.place.find((x) => x === S.f.place || x.startsWith(`${S.f.place}/`)) ?? p) : p;
      return chosen.split("/").slice(Math.max(S.f.place && !"?=".includes(S.f.place[0]) ? S.f.place.split("/").length + 1 : 2, 0)).join(" › ");
    };
    // a place holds sentences, short phrases and whole paragraphs: never one undivided run of them
    const byKind = (items) => {
      const order = S.fam.categories ?? [], kinds = [...new Set(items.map((it) => it.category))];
      if (!S.tree?.tree?.length || isFigure(S.fam) || isCards() || kinds.length < 2) return items;
      kinds.sort((a, b) => order.indexOf(a) - order.indexOf(b));
      return kinds.flatMap((k) => { const v = items.filter((it) => it.category === k); return [{ kindHead: catLabel(k), n: v.length }, ...v]; });
    };
    return el("div", {}, g.map((x, i) => el("details", { class: "h-grp", open: x.items.some((it) => it.id === S.sel) || (!x.folded && (g.length === 1 || i === 0)) },
      el("summary", {}, x.label, el("small", {}, x.items.length)),
      byKind(x.items).map((it) => { if (it.kindHead) return el("div", { class: "h-kind" }, it.kindHead, el("small", {}, it.n)); const th = thumb(it); return el("button", { type: "button", class: `h-row ${th ? "" : "text"}`, "aria-selected": String(S.sel === it.id), onclick: () => open(it.id) }, th, el("span", { class: "t" }, it.title), el("span", { class: "d" }, side(it)), el("span", { class: "g" }, chips(it))); }))));
  }
  /** The family's tree (段落 → 小節 → 使用時機) with how many items sit at or under each node. */
  function treePane() {
    const f = S.f, all = S.allItems ?? [];
    const count = (path) => all.filter((it) => (it.tags?.place ?? []).some((p) => p === path || p.startsWith(`${path}/`))).length;
    const node = (label, path, depth, kids) => {
      const n = count(path), open = f.openPlaces.has(path) || (f.place && f.place.startsWith(`${path}/`));
      const b = el("button", { type: "button", class: `tr d${depth} ${n ? "" : "zero"}`, "aria-pressed": String(f.place === path), onclick: () => {
        if (f.place === path) { f.place = ""; f.openPlaces.delete(path); } else { f.place = path; if (kids) f.openPlaces.add(path); }
        refresh(); } }, kids ? el("i", {}, open ? "▾" : "▸") : el("i", {}), label, el("span", {}, n || ""));
      return [b, kids && open ? kids() : null];
    };
    // only sentences sit on the tree; rules, wording and notes are listed by their own kind below it
    const onTree = (it) => TREE_FORMATS.has(it.format);
    const unplaced = all.filter((it) => onTree(it) && !(it.tags?.place ?? []).length).length;
    const others = new Map(); for (const it of all) if (!onTree(it)) others.set(it.category, (others.get(it.category) ?? 0) + 1);
    return [el("h4", {}, (S.tree.levels ?? []).join(" › ") || "位置"),
      S.tree.tree.map((sec) => node(sec.label, sec.label, 0, () => (sec.children ?? []).map((sub) => node(sub.label, `${sec.label}/${sub.label}`, 1, () => (sub.children ?? []).map((leaf) => node(leaf.label ?? leaf, `${sec.label}/${sub.label}/${leaf.label ?? leaf}`, 2, null)))))),
      unplaced ? el("button", { type: "button", class: "tr d0", "aria-pressed": String(f.place === "?"), onclick: () => { f.place = f.place === "?" ? "" : "?"; refresh(); } }, el("i", {}), "還沒歸位", el("span", {}, unplaced)) : null,
      others.size ? fold("其他", f.place?.startsWith("="), [...others.entries()].map(([c, n]) => el("button", { type: "button", class: "tr d0", "aria-pressed": String(f.place === `=${c}`), onclick: () => { f.place = f.place === `=${c}` ? "" : `=${c}`; refresh(); } }, el("i", {}), catLabel(c), el("span", {}, n)))) : null];
  }
  /** A part of the left column that stays one line until opened (it remembers) or in use. */
  function fold(label, inUse, rows) {
    const key = `${S.fam.id}/${label}`;
    return el("details", { class: "h-fold", open: inUse || S.f.more.has(key) ? true : null, ontoggle: (e) => { e.target.open ? S.f.more.add(key) : S.f.more.delete(key); } }, el("summary", {}, label), rows);
  }
  function tagsPane() {
    const f = S.f, out = [];
    const btn = (label, n, on, click) => el("button", { type: "button", "aria-pressed": String(!!on), onclick: click }, label, el("span", {}, n ?? ""));
    if (isFigure(S.fam)) {
      const parts = Object.fromEntries((S.fam.parts ?? []).map((p) => [p.label, p.count]));
      out.push(el("h4", {}, "階層"), ...Object.entries(LV).map(([k, l]) => btn(l, parts[l], f.level === k, () => { f.level = f.level === k ? "" : k; f.entity = ""; refresh(); })),
        btn("待製作", parts["待製作"], f.entity === "false", () => { f.entity = f.entity === "false" ? "" : "false"; f.level = ""; refresh(); }));
    }
    if (S.tree?.tree?.length && !isCards()) out.push(...treePane());
    const secs = S.tree?.tree?.length ? [] : Object.entries(S.sections ?? {}).sort((x, y) => y[1] - x[1]);
    if (secs.length > 1) out.push(el("h4", {}, "段落"), ...secs.map(([t, n]) => btn(t, n, f.section === t, () => { f.section = f.section === t ? "" : t; refresh(); })));
    for (const [facet, tags] of Object.entries(S.tags)) {
      if (facet === "place" || facet === "paper" || (S.tree?.tree?.length && facet === "kind" && !isFigure(S.fam))) continue; // the tree shows places; the rows' old free tags are not a structure
      const list = Object.entries(tags).sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
      if (!list.length) continue;
      const open = f.more.has(facet), on = (t) => f.tags.has(`${facet}:${t}`);
      // the tags that group something (two or more), at most eight; chosen ones always stay
      const main = list.filter(([t, n], i) => on(t) || (n > 1 && i < 8)), rest = list.length - main.length;
      const rows = [...(open ? list : main).map(([t, n]) => { const k = `${facet}:${t}`; return el("button", { type: "button", class: "multi", role: "checkbox", "aria-checked": String(f.tags.has(k)), onclick: () => { f.tags.has(k) ? f.tags.delete(k) : f.tags.add(k); refresh(); } }, el("b", {}), t, el("span", {}, n)); }),
        rest > 0 ? el("button", { type: "button", class: "more", onclick: () => { open ? f.more.delete(facet) : f.more.add(facet); paint(); } }, open ? "收起" : `其他 ${rest}`) : null];
      // beside a tree the facets are secondary: one line each until opened (or in use)
      if (S.tree?.tree?.length && !isFigure(S.fam) && !isCards()) out.push(fold(FACET[facet] ?? facet, list.some(([t]) => on(t)), rows));
      else out.push(el("h4", {}, FACET[facet] ?? facet), ...rows);
    }
    return out.length ? el("aside", { class: "h-tags" }, out) : null;
  }
  function head() {
    const f = S.f, sorts = isCards() ? [["updated", "最近修改"], ["created", "最近新增"], ["title", "名稱"], ["todo", "最不熟的先"]] : S.meta?.sorts ?? [["updated", "最近修改"], ["title", "名稱"]];
    let timer = 0;
    return el("div", { class: "h-head" }, el("h2", {}, S.fam.label), el("span", { class: "cnt" }, fmt(S.total)), el("span", { class: "sp" }),
      el("input", { type: "search", placeholder: "在這一類裡找", value: f.q, "aria-label": "在這一類裡找", oninput: (e) => { clearTimeout(timer); timer = setTimeout(() => { f.q = e.target.value.trim(); refresh(true); }, 250); } }),
      el("select", { "aria-label": "排序", onchange: (e) => { f.sort = e.target.value; refresh(); } }, sorts.map(([id, label]) => el("option", { value: id, selected: id === f.sort }, label))),
      !S.items.some((it) => it.thumb) && S.f.mode !== "wall" ? null : el("span", { class: "h-sw", role: "group", "aria-label": "顯示方式" }, [["list", "清單"], ["wall", "圖"]].map(([m, t]) => el("button", { type: "button", "aria-pressed": String(f.mode === m), onclick: () => { f.mode = m; try { localStorage.setItem(prefKey(S.fam), m); } catch {} paint(); } }, t))),
      isFigure(S.fam) ? el("label", { class: "h-plus", title: "給 L-Echo 一張流程圖的圖片，拆成流程圖、模塊、元件" }, "從圖片拆…", el("input", { type: "file", accept: "image/png,image/jpeg", hidden: true, onchange: async (e) => { const f = e.target.files?.[0]; e.target.value = ""; if (!f) return; try { await splitPicture(f, ""); } catch (err) { S.host.fail?.(err); } } })) : null,
      addBtn());
  }
  /** ＋ 新增: straight to the form when one category takes it, else a small choice of which. */
  function addBtn() {
    if (isCards()) return el("button", { type: "button", class: "h-plus", "aria-label": "新增知識卡", onclick: () => { location.hash = "#/new"; } }, "＋");
    const cs = addable();
    if (!cs.length) return null;
    if (cs.length === 1) return el("button", { type: "button", class: "h-plus", "aria-label": `新增${cs[0].label}`, onclick: () => add(cs[0].id) }, "＋");
    return el("select", { class: "h-plus", "aria-label": "新增", onchange: (e) => { const v = e.target.value; e.target.value = ""; if (v) add(v); } }, el("option", { value: "" }, "＋ 新增"), cs.map((c) => el("option", { value: c.id }, c.label)));
  }
  let detailEl = null;
  function paint(keepFocus) {
    const q = keepFocus ? S.root.querySelector(".h-head input") : null, pos = q?.selectionStart;
    const tp = tagsPane();
    const cat = el("div", { class: `h-cat ${S.sel ? "open" : ""} ${tp ? "" : "notags"}` }, tp, el("div", { class: "h-mid" }, head(), listBody()), S.sel ? (detailEl ?? el("aside", { class: "h-detail" })) : null);
    S.root.replaceChildren(el("div", { class: "hubv" }, cat));
    if (q) { const n = S.root.querySelector(".h-head input"); n.focus(); n.setSelectionRange(pos, pos); }
  }
  async function refresh(keepFocus) { try { await loadItems(); paint(keepFocus); } catch (e) { S.host.fail?.(e); } }

  // ── the detail to the right ──
  // ── 製作: a reference to make opens in the flow editor (the same one L-Slide uses); saving
  //    stores a drawable figure linked to it (server: figmake.rs) ──
  let flowLoading = null;
  function loadFlow() {
    if (window.LoreFlow?.edit) return Promise.resolve();
    flowLoading ??= new Promise((ok, no) => {
      document.head.append(el("link", { rel: "stylesheet", href: "/flow/editor.css" }));
      document.head.append(el("script", { src: "/flow/editor.js", onload: ok, onerror: () => { flowLoading = null; no(new Error("讀不到流程圖編輯器")); } }));
    });
    return flowLoading;
  }
  async function make(it) {
    try {
      await loadFlow();
      const r = await api("GET", `/api/hub/make/${enc(it.id)}`);
      let spec = r.spec ?? null;
      if (!spec) {
        // a first figure from the description (and the picture), so the editor does not open empty
        S.host.toast?.("正在從描述起草…");
        try { spec = (await api("POST", `/api/hub/make/${enc(it.id)}/draft`)).spec; }
        catch (e) { if (e.status === 422 && /時間軸/.test(e.message)) { S.host.fail?.(e); return; } S.host.toast?.(`沒有起草成功，從空白開始：${e.message}`); }
      }
      await LoreFlow.edit({ target: "brain", title: r.title, api: "/flow/api", doc: "", source: r.source, spec: spec ?? undefined,
        save: async ({ spec: made }) => { try { await api("POST", `/api/hub/make/${enc(it.id)}`, { spec: made, title: r.title }); return true; } catch (e) { return e.message; } } });
      S.sel = null; detailEl = null;   // the reference gave way to its figure
      S.allDirty = true; await loadStats(); await loadItems(); paint();
    } catch (e) { S.host.fail?.(e); }
  }

  const FACET = { domain: "領域", kind: "結構", use: "用途", state: "熟悉度", ckind: "種類", paper: "來源論文", tone: "語氣", place: "使用時機" };
  const META = ["domain", "kind", "use"];
  const cat = (id) => S.meta?.categories.find((c) => c.id === id);
  const fdef = (id) => S.meta?.formats.find((x) => x.id === id);
  const split = (v) => v.split(/[,，、\n]/).map((x) => x.trim()).filter(Boolean);
  const close = () => { S.sel = null; detailEl = null; paint(); };
  /** The detail: a form where the category can be written, the same things to read where it cannot. */
  function detail(it, d, isNew) {
    const c = cat(it.category), def = fdef(it.format), form = !!(c?.writable && def?.form), meta = !!c && !c.independent;
    // what the form holds until 儲存
    // which tag facets this family's form offers: on a tree, where the item sits and its tone (which paper a sentence came from is not something the author sorts by)
    const metaKeys = S.tree?.tree?.length && !isFigure(S.fam) ? ["place", "tone"] : META;
    const draft = { title: it.title ?? "", fields: { ...(it.fields ?? {}) }, tags: Object.fromEntries(metaKeys.map((k) => [k, (it.tags?.[k] ?? []).join("、")])), triggers: (it.triggers ?? []).join("、") };
    let saveBtn = null;
    const dirty = () => { if (saveBtn) saveBtn.disabled = false; };
    const input = (fd) => {
      const v = draft.fields[fd.name] ?? "", set = (e) => { draft.fields[fd.name] = fd.kind === "number" && e.target.value !== "" ? e.target.value : e.target.value; dirty(); };
      if (fd.kind === "choice" && fd.choices?.length) return el("select", { onchange: set }, el("option", { value: "" }, ""), fd.choices.map((x) => el("option", { value: x, selected: x === v }, x)));
      const long = fd.kind === "markdown" || fd.kind === "latex" || String(v).length > 60 || /template|example|text|message|body|note|usage/.test(fd.name);
      return long ? el("textarea", { rows: fd.name === "body" ? 14 : 2, oninput: set, spellcheck: "false" }, String(v)) : el("input", { value: String(v), type: fd.kind === "number" ? "number" : "text", oninput: set });
    };
    // `what` (generator / drawing / spec / reference) is the program's word, not the author's
    const onTree = metaKeys.includes("place") && metaKeys.length === 2;
    // beside a tree the place says where a sentence goes: the older section / function / position columns stay in the record, unseen
    const fields = (def?.fields ?? []).filter((fd) => fd.name !== "what" && typeof (it.fields?.[fd.name] ?? "") !== "object" && !(onTree && ["section", "function", "position"].includes(fd.name)));
    const allPlaces = () => (S.tree?.tree ?? []).flatMap((sec) => (sec.children ?? []).flatMap((sub) => (sub.children ?? []).map((leaf) => `${sec.label}/${sub.label}/${leaf.label ?? leaf}`)));
    /** The places of an item as chips: ✕ takes one away, the menu adds one (the first is the main one). */
    const placeEd = () => {
      const box = el("div", { class: "h-places" });
      const draw = () => {
        const ps = split(draft.tags.place ?? "");
        box.replaceChildren(...ps.map((p, i) => el("span", { class: i ? "pl" : "pl main" },
          el("button", { type: "button", onclick: () => { S.f.place = p; refresh(); } }, p.split("/").join(" › ")),
          el("button", { type: "button", class: "rm", "aria-label": "拿掉這個使用時機", onclick: () => { draft.tags.place = ps.filter((x) => x !== p).join("、"); dirty(); draw(); } }, "✕"))),
          el("select", { "aria-label": "加一個使用時機", onchange: (e) => { const v = e.target.value; if (v && !ps.includes(v)) { draft.tags.place = [...ps, v].join("、"); dirty(); } draw(); } },
            el("option", { value: "" }, "＋"), allPlaces().map((p) => el("option", { value: p }, p.split("/").join(" › ")))));
      };
      draw();
      return box;
    };
    const builtin = it.fields?.what === "generator";
    const link = (r) => el("button", { type: "button", class: "h-lk", onclick: () => open(r.id) }, thumb(r), r.title);
    const row = (fd) => form ? el("label", { class: "h-f" }, el("span", {}, fd.label, fd.required ? el("i", {}, "＊") : null), input(fd))
      : (it.fields?.[fd.name] ? el("label", { class: "h-f" }, el("span", {}, fd.label), el("div", { class: "ro" }, String(it.fields[fd.name]))) : null);
    const metaIn = (label, key, hint) => el("label", { class: "h-f" }, el("span", {}, label), el("input", { value: key === "triggers" ? draft.triggers : draft.tags[key], placeholder: hint, oninput: (e) => { key === "triggers" ? (draft.triggers = e.target.value) : (draft.tags[key] = e.target.value); dirty(); } }));
    async function save() {
      const first = (def?.fields ?? []).find((fd) => fd.required);
      const title = def?.shape === "row" ? String(draft.fields[first?.name] ?? draft.title).split("\n")[0].slice(0, 80) : draft.title.trim();
      if (onTree && form && "section" in Object.fromEntries((def?.fields ?? []).map((fd) => [fd.name, 1])) && !draft.fields.section) { const sec = split(draft.tags.place ?? "")[0]?.split("/")[0]; if (sec) draft.fields.section = sec; }
      const body = { ...it, line: undefined, thumb: undefined, title, fields: form ? draft.fields : it.fields, triggers: split(draft.triggers),
        // the facets the form does not show are kept as they are
        tags: Object.fromEntries(Object.entries({ ...(it.tags ?? {}), ...Object.fromEntries(Object.entries(draft.tags).map(([k, v]) => [k, split(v)])) }).filter(([, v]) => v.length)) };
      try {
        saveBtn.disabled = true;
        const r = isNew ? await api("POST", "/api/hub/items", body) : await api("PUT", `/api/hub/items/${enc(it.id)}`, body);
        const saved = r.item ?? r;
        S.host.toast?.(isNew ? "已新增" : "已儲存");
        S.allDirty = true; await loadStats(); await loadItems(); await open(saved.id);
      } catch (e) {
        saveBtn.disabled = false;
        const probs = e.body?.problems?.map((x) => x.message).join("；");
        S.host.fail?.(probs ? new Error(probs) : e);
      }
    }
    const canMeta = meta && !isNew || form;
    return el("aside", { class: "h-detail" },
      el("div", { class: "hd" }, el("h3", {}, isNew ? `新增${c?.label ?? ""}` : it.title), el("button", { type: "button", class: "x", "aria-label": "關閉", onclick: close }, "✕")),
      isNew ? null : el("div", { class: "ln" }, [c?.label, it.entity === false ? "待製作" : onTree || (it.line ?? "").length > 70 ? null : it.line].filter(Boolean).join(" · ")),
      it.thumb ? el("span", { class: "h-paper stage" }, el("img", { src: it.thumb, alt: "" })) : null,
      form && def?.shape !== "row" ? el("label", { class: "h-f" }, el("span", {}, "名稱", el("i", {}, "＊")), el("input", { value: draft.title, oninput: (e) => { draft.title = e.target.value; dirty(); } })) : null,
      fields.map(row),
      canMeta ? el("details", { class: "h-more", open: isNew ? null : (Object.values(it.tags ?? {}).flat().length || it.triggers?.length ? true : null) }, el("summary", {}, "標籤與關鍵字"), metaKeys.map((k) => k === "place" && onTree ? [el("div", { class: "k" }, "使用時機"), placeEd()] : metaIn(FACET[k], k, k === "place" ? "段落/小節/使用時機，可多個，用、分開" : "用、分開")), metaIn("觸發關鍵字", "triggers", "內容出現這些字就適用")) : [
        Object.entries(it.tags ?? {}).map(([facet, ts]) => [el("div", { class: "k" }, FACET[facet] ?? facet), el("div", { class: "chips" }, ts.map((t) => el("span", { class: "h-tag" }, t)))])],
      (it.tags?.place ?? []).length && !(onTree && canMeta) ? [el("div", { class: "k" }, "使用時機"), el("div", { class: "h-places" }, it.tags.place.map((p, i) => el("button", { type: "button", class: i ? "" : "main", onclick: () => { S.f.place = p; refresh(); } }, p.split("/").join(" › "))))] : null,
      d?.contains?.length ? [el("div", { class: "k" }, "由這些組成"), d.contains.map(link)] : null,
      d?.used_by?.length ? [el("div", { class: "k" }, "用在"), d.used_by.map(link)] : null,
      el("div", { class: "h-foot" }, c?.independent ? "不與 Lore 共用" : c?.tasks?.length ? `用在 ${c.tasks.length} 個步驟` : "還沒有步驟在用",
        isNew ? null : ` · ${it.source?.kind === "external" ? it.source.name : it.source?.kind === "author" ? "我加的" : "Lore"}`, it.used ? ` · 用過 ${it.used} 次` : null),
      el("div", { class: "h-acts" },
        !isNew && it.format === "figure" && it.entity === false ? el("button", { type: "button", class: "pri", onclick: () => make(it) }, "製作") : null,
        !isNew && it.fields?.made_from ? el("button", { type: "button", onclick: () => make({ id: it.fields.made_from }) }, "編輯圖形") : null,
        canMeta ? (saveBtn = el("button", { type: "button", class: "pri", disabled: !isNew, onclick: save }, isNew ? "新增" : "儲存")) : null,
        !isNew && c?.writable && !builtin ? el("button", { type: "button", class: "quiet", onclick: () => remove(it) }, "移除") : null),
      !isNew && canMeta ? el("details", { class: "h-more" }, el("summary", {}, "請 L-Echo 改"), askEcho(it)) : null);
  }
  function cardDetail(it) {
    const n = it.node, known = it.state === "known";
    const recall = async (outcome) => { try { await api("POST", `/api/recall/${enc(n.id)}`, { outcome, via: "kb-list" }); await loadStats(); await loadItems(); await open(n.id); } catch (e) { S.host.fail?.(e); } };
    return el("aside", { class: "h-detail" },
      el("div", { class: "hd" }, el("h3", {}, n.title), el("button", { type: "button", class: "x", "aria-label": "關閉", onclick: close }, "✕")),
      el("div", { class: "ln" }, [it.tags.ckind[0], ...it.tags.domain, STATE[it.state]].filter(Boolean).join(" · ")),
      n.aliases?.length ? el("div", { class: "ln" }, n.aliases.join(" · ")) : null,
      n.summary ? el("p", { class: "h-sum" }, n.summary) : null,
      n.tags?.length ? [el("div", { class: "k" }, "標籤"), el("div", { class: "chips" }, n.tags.map((t) => el("span", { class: "h-tag" }, t)))] : null,
      el("div", { class: "h-foot" }, n.next_check_at ? `下次確認 ${n.next_check_at.slice(0, 10).replaceAll("-", "/")}` : known ? "不需再確認" : "還沒排複習", n.streak ? ` · 連續記得 ${n.streak} 次` : null),
      el("div", { class: "h-acts" }, el("button", { type: "button", class: "pri", onclick: () => { location.hash = `#/node/${n.id}`; } }, "打開"),
        el("button", { type: "button", onclick: () => recall(known ? "unfamiliar" : "mastered") }, known ? "改回學習" : "已熟悉，不用學")));
  }
  async function open(id) {
    S.sel = id;
    if (isCards()) { const it = S.items.find((x) => x.id === id); if (it) { detailEl = cardDetail(it); paint(); } else close(); return; }
    try {
      const d = await api("GET", `/api/hub/items/${enc(id)}`);
      detailEl = detail(d.item, d, false);
      paint();
    } catch (e) { S.sel = null; detailEl = null; S.host.fail?.(e); }
  }
  /** ＋: a blank form for one of the family's categories that can be written by hand. */
  function add(catId) {
    const c = cat(catId);
    S.sel = "new";
    detailEl = detail({ id: "", category: c.id, format: c.format, title: "", fields: {}, tags: {}, triggers: [], entity: true, source: { kind: "author", name: "" } }, null, true);
    paint();
    detailEl.querySelector("input, textarea, select")?.focus();
  }
  const addable = () => (S.fam.categories ?? []).map(cat).filter((c) => c?.writable && fdef(c.format)?.form);
  async function remove(it) {
    try {
      await api("DELETE", `/api/hub/items/${enc(it.id)}`);
      S.sel = null; detailEl = null; S.allDirty = true;
      S.host.toast?.(`「${it.title}」已移除`, { label: "復原", run: async () => { try { await api("POST", `/api/hub/items/${enc(it.id)}/restore`); await loadStats(); await refresh(); } catch (e) { S.host.fail?.(e); } } });
      await loadStats(); await refresh();
    } catch (e) { S.host.fail?.(e); }
  }

  // ── L-Echo: it proposes, the author approves in 待同意 (docs/brain-hub.md §7) ──
  async function echo(path, body) {
    const port = await LoreLinks.ensure("le", { onWait: () => S.host.toast?.("正在開啟 L-Echo…") });
    if (!port) throw new Error("L-Echo 沒有開起來");
    const r = await fetch(`http://127.0.0.1:${port}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error ?? `L-Echo 回答 ${r.status}`);
    return j;
  }
  /** A draft from L-Echo in the right pane: what it would change, and 提出 to put it in 待同意. */
  function draftPane(title, body, proposal, message, picture) {
    const send = el("button", { type: "button", class: "pri", onclick: async () => {
      try {
        send.disabled = true;
        const r = await echo("/api/actions/know/propose", { proposal });
        // the picture waits beside the proposal and becomes the reference's picture once approved
        if (picture?.data_url && r.id) await api("POST", `/api/hub/proposals/${enc(r.id)}/picture`, { name: picture.name, data_url: picture.data_url });
        S.host.toast?.("已放進待同意"); close(); await loadStats();
      }
      catch (e) { send.disabled = false; S.host.fail?.(e); } } }, "提出到待同意");
    S.sel = "draft";
    detailEl = el("aside", { class: "h-detail" },
      el("div", { class: "hd" }, el("h3", {}, title), el("button", { type: "button", class: "x", "aria-label": "關閉", onclick: close }, "✕")),
      el("div", { class: "ln" }, "L-Echo 的提議，還沒有寫入"), body, message ? el("p", { class: "h-sum" }, message) : null,
      el("div", { class: "h-acts" }, proposal ? send : null, el("button", { type: "button", class: "quiet", onclick: close }, "不要")));
    paint();
  }
  /** A changed part in the author's words (`tags.kind` → 標籤, `fields.template` → the field's label). */
  const partName = (n) => (n === "title" ? "名稱" : n === "triggers" ? "觸發關鍵字" : n === "contains" ? "組成" : n.startsWith("tags.") ? "標籤" : (S.meta?.formats.flatMap((f) => f.fields).find((fd) => `fields.${fd.name}` === n)?.label ?? n.replace(/^fields\./, "")));
  const show1 = (v) => (v == null ? "" : typeof v === "string" ? v : Array.isArray(v) ? v.join("、") : JSON.stringify(v));
  const pick = (o, path) => path.split(".").reduce((x, k) => x?.[k], o);
  /** 請 Echo 改: one instruction about one item → a draft of what would change. */
  function askEcho(it) {
    const input = el("textarea", { rows: 3, placeholder: "要怎麼改？例如：改得更簡短、補上英文、加上觸發關鍵字 timeline" });
    const go = el("button", { type: "button", class: "pri", onclick: async () => {
      const instruction = input.value.trim(); if (!instruction) return;
      go.disabled = true; go.textContent = "L-Echo 正在看…";
      try {
        const d = await echo("/api/actions/know/edit", { id: it.id, instruction });
        draftPane(`改「${it.title}」`, [
          (d.changed ?? []).map((name) => el("div", { class: "h-diff" }, el("div", { class: "k" }, partName(name)), el("del", {}, show1(pick(d.before, name))), el("ins", {}, show1(pick(d.after, name))))),
          (d.dropped ?? []).length ? [el("div", { class: "k" }, "沒有採用"), d.dropped.map((x) => el("div", { class: "ln" }, x))] : null], d.proposal, d.proposal ? d.note : d.message);
      } catch (e) { go.disabled = false; go.textContent = "請 Echo 改"; S.host.fail?.(e); }
    } }, "請 Echo 改");
    return el("div", { class: "h-echo" }, input, go);
  }
  /** 從圖片拆: a figure's picture → figure, modules, components as references to make. */
  async function splitPicture(file, note) {
    const data = await new Promise((ok, no) => { const r = new FileReader(); r.onload = () => ok(r.result); r.onerror = () => no(new Error("讀不了這個檔案")); r.readAsDataURL(file); });
    S.host.toast?.("L-Echo 正在看這張圖…");
    const d = await echo("/api/actions/know/figure", { image: data, note });
    const sm = d.summary ?? {};
    draftPane(d.proposal?.title ?? "這張圖", [
      d.image?.data_url ? el("span", { class: "h-paper stage" }, el("img", { src: d.image.data_url, alt: "" })) : null,
      el("div", { class: "h-foot" }, `新的流程圖 ${sm.new_figure ? 1 : 0} · 新的模塊 ${sm.new_modules ?? 0} · 新的元件 ${sm.new_components ?? 0} · 連到已有的 ${sm.linked ?? 0}`),
      (d.proposal?.changes ?? []).map((c) => el("div", { class: "h-ch", style: "grid-template-columns:44px minmax(0,1fr) auto" }, el("span", { class: `op ${c.op}` }, OP[c.op] ?? c.op), el("span", { class: "t" }, c.item?.title ?? ""), el("span", { class: "d" }, LV[c.item?.level] ?? "")))], d.proposal, d.message, d.image);
  }

  // ── 待同意: what an AI or a tool proposes; nothing is written until the author says so ──
  async function proposals() {
    const [open, done] = await Promise.all([api("GET", "/api/hub/proposals?state=open"), api("GET", "/api/hub/proposals?state=applied")]);
    const one = async (p) => {
      const d = await api("GET", `/api/hub/proposals/${enc(p.id)}`), chosen = new Set(d.shown.map((x) => x.index));
      const line = (x) => { const it = x.after ?? x.before ?? {}; const fig = it.thumb || it.format === "figure";
        return el("label", { class: `h-ch ${x.problem ? "bad" : ""}` },
          el("input", { type: "checkbox", checked: !x.problem, disabled: !!x.problem, onchange: (e) => { e.target.checked ? chosen.add(x.index) : chosen.delete(x.index); } }),
          el("span", { class: `op ${x.op}` }, OP[x.op] ?? x.op),
          el("span", { class: "t" }, it.title ?? x.note ?? "", it.level && LV[it.level] ? el("small", {}, LV[it.level]) : null, fig && it.entity === false ? el("small", {}, "待製作") : null),
          // what the change says it does (where a row is placed) comes first; else which parts change
          el("span", { class: "d", title: x.note || null }, x.problem || x.note || (x.op === "update" && x.changed?.length ? `改：${[...new Set(x.changed.map(partName))].join("、")}` : "")));
      };
      if (!d.shown.some((x) => !x.problem)) chosen.clear();
      const act = async (fn, msg) => { try { await fn(); S.host.toast?.(msg); S.allDirty = true; await show(S.root, "proposals"); } catch (e) { S.host.fail?.(e); } };
      // a long batch stays folded: the first few rows, the rest on demand
      const FEW = 6, rows = d.shown.map(line), rest = rows.slice(FEW);
      const more = rest.length ? el("details", { class: "h-rest" }, el("summary", {}, `其餘 ${rest.length} 條`), rest) : null;
      return el("section", { class: "h-box h-prop" },
        el("div", { class: "h-cap" }, p.by === "le" ? "L-Echo" : p.by, el("span", {}, (p.created_at ?? "").slice(0, 10).replaceAll("-", "/"))),
        el("h3", {}, p.title), rows.slice(0, FEW), more,
        el("div", { class: "h-acts" },
          el("button", { type: "button", class: "pri", onclick: () => act(() => api("POST", `/api/hub/proposals/${enc(p.id)}/apply`, { chosen: [...chosen].sort((a, b) => a - b) }), "已套用勾選的") }, "同意勾選的"),
          el("button", { type: "button", class: "quiet", onclick: () => act(() => api("POST", `/api/hub/proposals/${enc(p.id)}/dismiss`), "已略過") }, "都不要")));
    };
    const undo = (p) => el("div", { class: "h-done" }, el("span", {}, p.title), el("button", { type: "button", onclick: async () => { try { const r = await api("POST", `/api/hub/proposals/${enc(p.id)}/undo`); S.host.toast?.(r.not_undone?.length ? r.not_undone[0] : "已復原"); S.allDirty = true; await show(S.root, "proposals"); } catch (e) { S.host.fail?.(e); } } }, "復原"));
    const boxes = await Promise.all((open.proposals ?? []).map(one));
    return el("div", { class: "h-props" }, el("div", { class: "h-head" }, el("h2", {}, "待同意"), el("span", { class: "cnt" }, boxes.length || "")),
      boxes.length ? boxes : el("div", { class: "h-empty" }, "沒有等你同意的修改"),
      (done.proposals ?? []).length ? [el("div", { class: "h-wh" }, "已套用", el("small", {}, done.proposals.length)), done.proposals.slice(0, 10).map(undo)] : null);
  }

  // ── 收件: sentences handed over (read aloud to L-Echo, or selected) wait here, worked out;
  //    加入 writes one into the knowledge, 略過 drops it (docs/brain-hub.md §16) ──
  // 確認: what a model wrote for his own sentences. Three of his sentences a page, each with the
  // other ways to say it; every one is decided on its own (加入 / 不要) and can be taken back.
  const RV = { set: null, groups: [], page: 0, counts: {} }, RV_PER = 3;
  async function review(mode = "open") {
    const sets = (await api("GET", "/api/hub/review")).sets ?? [];
    const pick = (mode === "open" && sets.find((s) => s.counts?.open)) || sets.at(-1);
    RV.set = null; RV.groups = []; RV.page = 0; RV.counts = {};
    if (pick) {
      const d = await api("GET", `/api/hub/review/${enc(pick.id)}`);
      RV.set = d.set; RV.counts = d.counts ?? {};
      RV.groups = (d.groups ?? []).filter((g) => g.candidates.some((c) => c.state === mode));
    }
    const box = el("div", { class: "h-rv" });
    reviewPaint(box);
    return box;
  }
  function reviewPaint(box) {
    const pages = Math.ceil(RV.groups.length / RV_PER), c = RV.counts;
    const again = async (mode) => { try { box.replaceWith(await review(mode)); } catch (e) { S.host.fail?.(e); } };
    if (RV.page >= pages) {
      box.replaceChildren(el("div", { class: "h-rv-done" },
        el("h2", {}, !RV.set ? "沒有要你決定的" : c.open ? `還有 ${fmt(c.open)} 個沒看` : "看完了"),
        RV.set ? el("p", {}, `加入 ${fmt(c.accepted)} 個說法，不要 ${fmt(c.rejected)} 個。`) : null,
        el("div", { class: "h-rv-nav" },
          c.open ? el("button", { type: "button", class: "pri", onclick: () => again("open") }, "繼續看") : null,
          el("a", { class: c.open ? "btn" : "pri", href: "#/hub" }, "回總覽"),
          c.rejected ? el("button", { type: "button", class: "q", onclick: () => again("rejected") }, "再看一次不要的") : null)));
      return;
    }
    const row = (cand, own) => {
      const r = el("div", {});
      const act = async (what) => {
        try {
          const j = await api("POST", `/api/hub/review/${enc(RV.set.id)}/${enc(cand.id)}/${what}`);
          Object.assign(cand, j.candidate); RV.counts = j.counts ?? RV.counts; S.allDirty = true; draw(); loadStats().catch(() => {});
        } catch (e) { S.host.fail?.(e); }
      };
      const draw = () => {
        r.className = `h-rv-r ${cand.state}`;
        r.replaceChildren(el("i", {}),
          el("div", { class: "s" }, cand.kind === "swap" ? [el("span", { class: "was" }, cand.slot), " → ", cand.text] : cand.text),
          cand.note ? el("div", { class: "n" }, cand.note) : null,
          el("div", { class: "act" }, cand.state === "open"
            ? [el("button", { type: "button", class: "q", onclick: () => act("reject") }, "不要"), own ? el("button", { type: "button", class: "pri", onclick: () => act("accept") }, "加入") : null]
            : el("button", { type: "button", class: "q", onclick: () => act("undo") }, `${cand.state === "accepted" ? "已加入" : "不要了"} · 復原`)));
      };
      draw();
      return r;
    };
    const turn = (d) => { RV.page += d; reviewPaint(box); box.scrollTop = 0; };
    box.replaceChildren(
      el("div", { class: "h-rv-bar" }, el("i", { style: `width:${Math.round(((RV.page + 1) / pages) * 100)}%` })),
      el("div", { class: "h-rv-in" },
        el("div", { class: "h-rv-top" }, el("h2", {}, "你會這樣寫嗎？"), el("span", { class: "pg" }, `第 ${RV.page + 1} 頁，共 ${pages} 頁`)),
        el("div", { class: "h-rv-lg" }, el("span", {}, el("i", {}), "你寫的"), el("span", { class: "m" }, el("i", {}), "模型寫的")),
        RV.groups.slice(RV.page * RV_PER, (RV.page + 1) * RV_PER).map((g) => el("div", { class: "h-rv-blk" },
          el("div", { class: `mine ${g.own ? "" : "gone"}` }, el("i", {}), el("span", {}, g.own ? g.own.fields?.template || g.own.title : "這一句已經不在了")),
          g.candidates.map((cand) => row(cand, g.own)))),
        el("div", { class: "h-rv-nav" },
          RV.page ? el("button", { type: "button", class: "q", onclick: () => turn(-1) }, "← 上一頁") : null,
          el("button", { type: "button", class: "next", onclick: () => turn(1) }, RV.page + 1 < pages ? "下一頁 →" : "看完了 →"))));
  }

  let inboxTimer = 0;
  async function inbox() {
    clearTimeout(inboxTimer);
    const r = await api("GET", "/api/hub/inbox?state=ready,failed,waiting,working"), c = r.counts ?? {};
    const again = () => show(S.root, "inbox").catch(() => {});
    const act = async (fn) => { try { await fn(); S.allDirty = true; await again(); } catch (e) { S.host.fail?.(e); } };
    const places = (it) => (it?.tags?.place ?? []).map((p, i) => el("span", { class: `h-tag ${i ? "" : "src"}` }, p.split("/").join(" › ")));
    const ready = (e) => {
      const it = e.result ?? {}, tpl = el("textarea", { rows: 2, spellcheck: "false" }, it.fields?.template ?? it.title ?? "");
      const edited = () => ({ ...it, title: tpl.value.trim().slice(0, 80), fields: { ...(it.fields ?? {}), template: tpl.value.trim() } });
      return el("section", { class: "h-in" },
        el("div", { class: "src" }, e.text),
        tpl,
        el("div", { class: "meta" }, places(it), (it.tags?.tone ?? []).map((t) => el("span", { class: "h-tag" }, t)),
          (e.similar_items ?? []).length ? el("span", { class: "sim", title: e.similar_items.map((x) => x.title).join("\n") }, `和 ${e.similar_items.length} 條已有的很像`) : null),
        el("div", { class: "h-acts" }, el("button", { type: "button", class: "pri", onclick: () => act(() => api("POST", `/api/hub/inbox/${enc(e.id)}/accept`, { item: edited() })) }, "加入"),
          el("button", { type: "button", class: "quiet", onclick: () => act(() => api("POST", `/api/hub/inbox/${enc(e.id)}/skip`)) }, "略過")));
    };
    const failed = (e) => el("section", { class: "h-in bad" }, el("div", { class: "src" }, e.text), el("div", { class: "err" }, e.error),
      el("div", { class: "h-acts" }, el("button", { type: "button", onclick: () => act(() => api("POST", `/api/hub/inbox/${enc(e.id)}/retry`)) }, "再試一次"),
        el("button", { type: "button", class: "quiet", onclick: () => act(() => api("POST", `/api/hub/inbox/${enc(e.id)}/skip`)) }, "略過")));
    const es = r.entries ?? [], rd = es.filter((e) => e.state === "ready"), fl = es.filter((e) => e.state === "failed"), wt = es.filter((e) => e.state === "waiting" || e.state === "working");
    // while something is being worked on, look again by itself
    if (wt.length && S.root?.isConnected) inboxTimer = setTimeout(() => { if (S.view === "inbox") again(); }, 3000);
    return el("div", { class: "h-props" },
      el("div", { class: "h-head" }, el("h2", {}, "收件"), el("span", { class: "cnt" }, rd.length || ""), el("span", { class: "sp" }),
        wt.length ? el("span", { class: "cnt" }, `處理中 ${wt.length}`) : null,
        rd.length > 1 ? el("button", { type: "button", class: "h-plus", onclick: () => act(() => api("POST", "/api/hub/inbox/accept", { ids: rd.map((e) => e.id) })) }, `全部加入（${rd.length}）`) : null),
      rd.length || fl.length || wt.length ? null : el("div", { class: "h-empty" }, "沒有等你確認的句子"),
      rd.map(ready), fl.map(failed),
      wt.length ? [el("div", { class: "h-wh" }, "排隊中", el("small", {}, wt.length)), wt.slice(0, 20).map((e) => el("div", { class: "h-done" }, el("span", {}, e.text)))] : null);
  }

  // ── ＋ 來源: an outside application's knowledge, by a file in the fixed format ──
  function sourcesBox(sources) {
    const out = el("div", { class: "h-srcout" });
    const own = el("input", { type: "checkbox" });
    const file = el("input", { type: "file", accept: ".json,application/json", hidden: true, onchange: async (e) => {
      const f = e.target.files?.[0]; e.target.value = ""; if (!f) return;
      try {
        const r = await fetch(`/api/hub/import${own.checked ? "?own=1" : ""}`, { method: "POST", headers: { "content-type": "application/json" }, body: await f.text() });
        const j = await r.json().catch(() => ({}));
        if (!r.ok) throw new Error(j.error ?? `${r.status}`);
        out.replaceChildren(el("div", {}, `新增 ${j.added} · 取代 ${j.replaced} · 移除 ${j.removed}`), (j.problems ?? []).slice(0, 8).map((p) => el("div", { class: "bad" }, `${p.at}：${p.message}`)), (j.problems ?? []).length > 8 ? el("div", { class: "bad" }, `還有 ${j.problems.length - 8} 筆不符合`) : null);
        S.meta = null; S.allDirty = true; await Promise.all([loadStats(), loadMeta()]);
        S.root.querySelector(".h-cover")?.replaceWith(cover(await api("GET", "/api/hub/sources").catch(() => [])));
      } catch (err) { out.replaceChildren(el("div", { class: "bad" }, err.message)); }
    } });
    return el("div", { class: "h-box h-src" }, el("div", { class: "h-cap" }, "外部來源"),
      (sources ?? []).map((s) => el("div", { class: "h-srow" }, el("span", {}, s.name), el("span", { class: "d" }, s.sets.map((x) => `${catLabel(x.category)} ${x.count}${x.aligned ? "" : "（獨立）"}`).join(" · ")))),
      el("div", { class: "h-srow add" }, el("button", { type: "button", class: "h-plus", onclick: () => file.click() }, "＋ 來源"), el("label", { class: "own" }, own, "這是我自己的檔案"), file), out);
  }

  /** Draw the hub for `arg` (undefined: the cover; a family id: its page). */
  async function show(root, arg) {
    S.root = root;
    try {
      await Promise.all([loadStats(), loadMeta()]);
      S.view = arg === "inbox" ? "inbox" : arg === "proposals" ? "proposals" : arg === "review" ? "review" : "page";
      if (arg === "review") { S.fam = null; S.sel = null; detailEl = null; nav(); root.replaceChildren(el("div", { class: "hubv" }, await review())); return; }
      if (arg === "inbox") { S.fam = null; S.sel = null; detailEl = null; nav(); root.replaceChildren(el("div", { class: "hubv" }, await inbox())); return; }
      if (arg === "proposals") { S.fam = null; S.sel = null; detailEl = null; nav(); root.replaceChildren(el("div", { class: "hubv" }, await proposals())); return; }
      const fam = arg ? S.stats.families.find((f) => f.id === arg) : null;
      if (fam?.app === "kb") S.reg = null; // labels may have changed
      if (!fam) { S.fam = null; S.sel = null; detailEl = null; nav(); root.replaceChildren(el("div", { class: "hubv" }, cover(await api("GET", "/api/hub/sources").catch(() => [])))); return; }
      if (S.fam?.id !== fam.id) { S.f = defaults(fam); S.sel = null; detailEl = null; }
      if (fam.app === "kb" && S.pendingQ != null) { S.f.q = S.pendingQ; }
      S.fam = fam; nav();
      await loadItems(); paint();
    } catch (e) {
      root.replaceChildren(el("div", { class: "hubv" }, el("div", { class: "h-empty" }, e.message || "知識中樞現在打不開")));
    }
  }
  /** Another view took the page: no family is the current one. */
  const leave = () => { S.fam = null; S.sel = null; detailEl = null; S.allFor = null; S.view = ""; clearTimeout(inboxTimer); nav(); };
  /** The top search box looks through the cards. */
  const query = (q) => { S.pendingQ = q; if (isCards() && S.f && S.f.q !== q) { S.f.q = q; refresh(); } };
  return { show, leave, query, init: (host) => { S.host = host ?? {}; }, refreshNav: () => loadStats().catch(() => {}) };
})();
