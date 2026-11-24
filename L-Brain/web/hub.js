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
    return { q: "", sort: "updated", level: "", entity: "", section: "", more: new Set(), tags: new Set(), source: "", mode: mode ?? (isFigure(f) ? "wall" : "list") };
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
    // the left column lists the whole family's tags, whatever is filtered: a tag never disappears
    if (S.allFor !== S.fam.id || S.allDirty) {
      const all = (await api("GET", `/api/hub/items?${new URLSearchParams({ family: S.fam.id, limit: "1000" })}`)).items ?? [];
      S.tags = {}; S.sections = {};
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
    if (isCards()) return [...out.entries()].sort((a, b) => (a[0] === "未分類") - (b[0] === "未分類") || b[1].length - a[1].length).map(([k, v]) => ({ key: k, label: k, items: v }));
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
    const secs = Object.entries(S.sections ?? {}).sort((x, y) => y[1] - x[1]);
    if (secs.length > 1) out.push(el("h4", {}, "段落"), ...secs.map(([t, n]) => btn(t, n, f.section === t, () => { f.section = f.section === t ? "" : t; refresh(); })));
    for (const [facet, tags] of Object.entries(S.tags)) {
      const list = Object.entries(tags).sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
      if (!list.length) continue;
      const open = f.more.has(facet), on = (t) => f.tags.has(`${facet}:${t}`);
      // the tags that group something (two or more), at most eight; chosen ones always stay
      const main = list.filter(([t, n], i) => on(t) || (n > 1 && i < 8)), rest = list.length - main.length;
      out.push(el("h4", {}, FACET[facet] ?? facet), ...(open ? list : main).map(([t, n]) => { const k = `${facet}:${t}`; return btn(t, n, f.tags.has(k), () => { f.tags.has(k) ? f.tags.delete(k) : f.tags.add(k); refresh(); }); }),
        rest > 0 ? el("button", { type: "button", class: "more", onclick: () => { open ? f.more.delete(facet) : f.more.add(facet); paint(); } }, open ? "收起" : `其他 ${rest}`) : null);
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
  const FACET = { domain: "領域", kind: "結構", use: "用途", state: "熟悉度", ckind: "種類" };
  const META = ["domain", "kind", "use"];
  const cat = (id) => S.meta?.categories.find((c) => c.id === id);
  const fdef = (id) => S.meta?.formats.find((x) => x.id === id);
  const split = (v) => v.split(/[,，、\n]/).map((x) => x.trim()).filter(Boolean);
  const close = () => { S.sel = null; detailEl = null; paint(); };
  /** The detail: a form where the category can be written, the same things to read where it cannot. */
  function detail(it, d, isNew) {
    const c = cat(it.category), def = fdef(it.format), form = !!(c?.writable && def?.form), meta = !!c && !c.independent;
    // what the form holds until 儲存
    const draft = { title: it.title ?? "", fields: { ...(it.fields ?? {}) }, tags: Object.fromEntries(META.map((k) => [k, (it.tags?.[k] ?? []).join("、")])), triggers: (it.triggers ?? []).join("、") };
    let saveBtn = null;
    const dirty = () => { if (saveBtn) saveBtn.disabled = false; };
    const input = (fd) => {
      const v = draft.fields[fd.name] ?? "", set = (e) => { draft.fields[fd.name] = fd.kind === "number" && e.target.value !== "" ? e.target.value : e.target.value; dirty(); };
      if (fd.kind === "choice" && fd.choices?.length) return el("select", { onchange: set }, el("option", { value: "" }, ""), fd.choices.map((x) => el("option", { value: x, selected: x === v }, x)));
      const long = fd.kind === "markdown" || fd.kind === "latex" || String(v).length > 60 || /template|example|text|message|body|note|usage/.test(fd.name);
      return long ? el("textarea", { rows: fd.name === "body" ? 14 : 2, oninput: set, spellcheck: "false" }, String(v)) : el("input", { value: String(v), type: fd.kind === "number" ? "number" : "text", oninput: set });
    };
    // `what` (generator / drawing / spec / reference) is the program's word, not the author's
    const fields = (def?.fields ?? []).filter((fd) => fd.name !== "what" && typeof (it.fields?.[fd.name] ?? "") !== "object");
    const builtin = it.fields?.what === "generator";
    const link = (r) => el("button", { type: "button", class: "h-lk", onclick: () => open(r.id) }, thumb(r), r.title);
    const row = (fd) => form ? el("label", { class: "h-f" }, el("span", {}, fd.label, fd.required ? el("i", {}, "＊") : null), input(fd))
      : (it.fields?.[fd.name] ? el("label", { class: "h-f" }, el("span", {}, fd.label), el("div", { class: "ro" }, String(it.fields[fd.name]))) : null);
    const metaIn = (label, key, hint) => el("label", { class: "h-f" }, el("span", {}, label), el("input", { value: key === "triggers" ? draft.triggers : draft.tags[key], placeholder: hint, oninput: (e) => { key === "triggers" ? (draft.triggers = e.target.value) : (draft.tags[key] = e.target.value); dirty(); } }));
    async function save() {
      const first = (def?.fields ?? []).find((fd) => fd.required);
      const title = def?.shape === "row" ? String(draft.fields[first?.name] ?? draft.title).split("\n")[0].slice(0, 80) : draft.title.trim();
      const body = { ...it, line: undefined, thumb: undefined, title, fields: form ? draft.fields : it.fields, triggers: split(draft.triggers),
        tags: Object.fromEntries(Object.entries(draft.tags).map(([k, v]) => [k, split(v)]).filter(([, v]) => v.length)) };
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
      isNew ? null : el("div", { class: "ln" }, [c?.label, it.line].filter(Boolean).join(" · ")),
      it.thumb ? el("span", { class: "h-paper stage" }, el("img", { src: it.thumb, alt: "" })) : null,
      form && def?.shape !== "row" ? el("label", { class: "h-f" }, el("span", {}, "名稱", el("i", {}, "＊")), el("input", { value: draft.title, oninput: (e) => { draft.title = e.target.value; dirty(); } })) : null,
      fields.map(row),
      canMeta ? el("details", { class: "h-more", open: isNew ? null : (Object.values(it.tags ?? {}).flat().length || it.triggers?.length ? true : null) }, el("summary", {}, "標籤與關鍵字"), META.map((k) => metaIn(FACET[k], k, "用、分開")), metaIn("觸發關鍵字", "triggers", "內容出現這些字就適用")) : [
        Object.entries(it.tags ?? {}).map(([facet, ts]) => [el("div", { class: "k" }, FACET[facet] ?? facet), el("div", { class: "chips" }, ts.map((t) => el("span", { class: "h-tag" }, t)))])],
      d?.contains?.length ? [el("div", { class: "k" }, "由這些組成"), d.contains.map(link)] : null,
      d?.used_by?.length ? [el("div", { class: "k" }, "用在"), d.used_by.map(link)] : null,
      el("div", { class: "h-foot" }, c?.independent ? "不與 Lore 共用" : c?.tasks?.length ? `用在 ${c.tasks.length} 個步驟` : "還沒有步驟在用",
        isNew ? null : ` · ${it.source?.kind === "external" ? it.source.name : it.source?.kind === "author" ? "我加的" : "Lore"}`, it.used ? ` · 用過 ${it.used} 次` : null),
      el("div", { class: "h-acts" },
        canMeta ? (saveBtn = el("button", { type: "button", class: "pri", disabled: !isNew, onclick: save }, isNew ? "新增" : "儲存")) : null,
        !isNew && c?.writable && !builtin ? el("button", { type: "button", class: "quiet", onclick: () => remove(it) }, "移除") : null));
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

  /** Draw the hub for `arg` (undefined: the cover; a family id: its page). */
  async function show(root, arg) {
    S.root = root;
    try {
      await Promise.all([loadStats(), loadMeta()]);
      const fam = arg ? S.stats.families.find((f) => f.id === arg) : null;
      if (fam?.app === "kb") S.reg = null; // labels may have changed
      if (!fam) { S.fam = null; S.sel = null; detailEl = null; nav(); root.replaceChildren(el("div", { class: "hubv" }, cover())); return; }
      if (S.fam?.id !== fam.id) { S.f = defaults(fam); S.sel = null; detailEl = null; }
      if (fam.app === "kb" && S.pendingQ != null) { S.f.q = S.pendingQ; }
      S.fam = fam; nav();
      await loadItems(); paint();
    } catch (e) {
      root.replaceChildren(el("div", { class: "hubv" }, el("div", { class: "h-empty" }, e.message || "知識中樞現在打不開")));
    }
  }
  /** Another view took the page: no family is the current one. */
  const leave = () => { S.fam = null; S.sel = null; detailEl = null; S.allFor = null; nav(); };
  /** The top search box looks through the cards. */
  const query = (q) => { S.pendingQ = q; if (isCards() && S.f && S.f.q !== q) { S.f.q = q; refresh(); } };
  return { show, leave, query, init: (host) => { S.host = host ?? {}; }, refreshNav: () => loadStats().catch(() => {}) };
})();
