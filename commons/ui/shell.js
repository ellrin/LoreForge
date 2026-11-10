// LoreForge shell: the parts every app has in the same place (docs/shell.md).
// Needs icons.js (ICONS, icon, paintIcons) and theme.js (LoreTheme) first,
// base.css for the look. No build step, no framework.
//
//   const shell = LoreShell.mount({
//     app: "sc",                                  // pc | sc | lr | kb
//     brand: "SLIDE/COMPOSER",                    // the slash is drawn in the accent
//     home: () => goHome(),                       // brand click, 首頁 in the picker
//     picker: { icon: "cards", name: "", recent: async () => [{ label, current, run }],
//               actions: [{ icon: "folder", label: "開啟…", run }, …] },
//     status: { onClick: (state) => … },          // the chip's action (LR: 另存新檔)
//     primary: [{ id, icon, label: "放映", run | menu: [items] | () => items }],   // 2–4
//     ai: { icon: "spark", label: "AI", run | menu },          // last of the primary group
//     notation: () => openNotation(),             // 對照表
//     export: [{ icon: "download", label: "PPTX", run }, …, { icon: "printer", label: "列印", run }],
//     undo: { undo: () => …, redo: () => … },     // shown only while editing (shell.editing(true))
//     drawers: { left: "#sorter", leftLabel: "投影片", leftIcon: "cards", right: "#side", rightLabel: "面板" },
//     search: () => openFind(),                   // ⌘F
//     save: () => saveNow(),                      // ⌘S
//     settingsTabs: [{ id: "paths", label: "路徑", build: (el) => … }],   // after the shared tabs
//     general: (el) => …, look: (el) => …, sync: (el) => …,   // app content of 一般 / 外觀 extra / 同步
//     modelSetup: (kind) => …,                    // 下載／登入… in 模型 (keys stay in the app)
//     keys: [{ label: "下一張", keys: "→" }],      // shortcuts that have no button
//     host: document.querySelector("header.top"), // fill this header (default: a new one first in <body>)
//   });
//   shell.slots.lead / .primary / .right           elements for app extras (font chip, member chip)
//   shell.setName(name) · shell.setStatus(state, { label, title, action }) · shell.editing(on)
//   shell.setUndo({ undo: "移動" | null, redo: null }) · shell.drawer("left" | "right", on?)
//
//   LoreShell.toast(msg, { err, action: { label, run }, ms })
//   LoreShell.ask(anchor, lines, buttons) → Promise<value | null>     (never a native confirm)
//   LoreShell.menu(anchor, items)                    items: { icon, label, run, kbd, current, disabled } | "-"
//   LoreShell.settings(tab?)                         the gear dialog: 一般 外觀 模型 同步 快捷鍵 + the app's tabs
//   LoreShell.engineChip({ task, value, used, onChange })   a feature's own engine choice
//   LoreShell.block({ n: "01", title, count, help, open, key }) → element with .body
//   LoreShell.shortcuts(root) → [{ group, label, keys }]  (from aria-labels that end in keys)
//   LoreLinks.open(app, params) · LoreLinks.url(app, params)

(() => {
  const $ = (s, r = document) => (typeof s === "string" ? r.querySelector(s) : s);
  function h(tag, attrs = {}, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs ?? {})) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on") && typeof v === "function") e.addEventListener(k.slice(2), v);
      else if (k === "dataset") Object.assign(e.dataset, v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c);
    return e;
  }
  const ico = (name, label, onclick, extra = {}) => {
    const b = h("button", { type: "button", class: "ico", "aria-label": label, ...extra });
    b.append(icon(name));
    if (onclick) b.addEventListener("click", onclick);
    return b;
  };
  /** Fixed-position layers go into the open modal dialog, else they sit under its top layer. */
  const layer = (near) => near?.closest?.("dialog[open]") ?? [...document.querySelectorAll("dialog[open]")].pop() ?? document.body;
  const place = (el, anchor, gap = 4) => {
    const r = anchor?.getBoundingClientRect?.() ?? { left: innerWidth / 2 - el.offsetWidth / 2, right: 0, top: 56, bottom: 56 };
    const w = el.offsetWidth, hh = el.offsetHeight;
    const below = r.bottom + gap + hh < innerHeight - 8 || r.top - gap - hh < 8;
    el.style.top = `${below ? r.bottom + gap : r.top - gap - hh}px`;
    el.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
  };

  // ── menu ───────────────────────────────────────────────────────────
  let openMenu = null;
  function closeMenu() {
    if (!openMenu) return;
    openMenu.anchor?.setAttribute("aria-expanded", "false");
    openMenu.el.remove();
    document.removeEventListener("pointerdown", openMenu.away, true);
    document.removeEventListener("keydown", openMenu.key, true);
    openMenu = null;
  }
  function menu(anchor, items) {
    const same = openMenu?.anchor === anchor;
    closeMenu();
    if (same) return null;
    const m = h("div", { class: "menu", role: "menu" });
    for (const it of items) {
      if (it === "-") { m.append(h("div", { class: "menu-sep", role: "separator" })); continue; }
      if (it.head) { m.append(h("div", { class: "menu-head" }, it.head)); continue; }
      const b = h("button", { type: "button", role: "menuitem", disabled: it.disabled, "aria-current": it.current ? "true" : null, title: it.title });
      if (it.icon) b.append(icon(it.icon));
      b.append(h("span", { class: "lbl" }, it.label));
      if (it.kbd) b.append(h("kbd", {}, it.kbd));
      b.onclick = () => { closeMenu(); it.run?.(); };
      m.append(b);
    }
    layer(anchor).append(m);
    place(m, anchor);
    anchor?.setAttribute("aria-expanded", "true");
    const away = (e) => { if (!m.contains(e.target) && !anchor?.contains(e.target)) closeMenu(); };
    const key = (e) => {
      const list = [...m.querySelectorAll("button:not(:disabled)")];
      const i = list.indexOf(document.activeElement);
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); closeMenu(); anchor?.focus?.(); }
      else if (e.key === "ArrowDown") { e.preventDefault(); list[(i + 1) % list.length]?.focus(); }
      else if (e.key === "ArrowUp") { e.preventDefault(); list[(i - 1 + list.length) % list.length]?.focus(); }
    };
    setTimeout(() => document.addEventListener("pointerdown", away, true), 0);
    document.addEventListener("keydown", key, true);
    openMenu = { el: m, anchor, away, key };
    return m;
  }
  const runOrMenu = (spec) => async (e) => {
    const btn = e.currentTarget;
    if (spec.menu) menu(btn, typeof spec.menu === "function" ? await spec.menu() : spec.menu);
    else spec.run?.(e);
  };

  // ── toast ──────────────────────────────────────────────────────────
  let toastTimer = 0;
  function toast(msg, opt = {}) {
    if (typeof opt === "boolean") opt = { err: opt };
    document.querySelectorAll(".toast").forEach((t) => t.remove());
    clearTimeout(toastTimer);
    const t = h("div", { class: "toast" + (opt.err ? " err" : ""), role: opt.err ? "alert" : "status" }, h("span", {}, msg));
    if (opt.action) t.append(h("button", { type: "button", class: "toast-act", onclick: () => { t.remove(); opt.action.run(); } }, opt.action.label));
    layer().append(t);
    toastTimer = setTimeout(() => t.remove(), opt.ms ?? (opt.action ? 8000 : opt.err ? 5000 : 2600));
    return t;
  }

  // ── ask: inline, next to what it is about ──────────────────────────
  let asking = null;
  function ask(anchor, lines, buttons) {
    asking?.(null);
    if (typeof lines === "string") lines = [lines];
    buttons ??= [{ label: "取消", value: null }, { label: "確定", value: true, primary: true }];
    return new Promise((resolve) => {
      const box = h("div", { class: "ask", role: "alertdialog", "aria-label": lines[0] }, ...lines.map((l) => h("p", {}, l)));
      const acts = h("div", { class: "ask-acts" });
      let primary = null;
      for (const b of buttons) {
        const el = h("button", { type: "button", class: "btn small" + (b.primary ? " primary" : "") + (b.danger ? " danger" : "") }, b.icon ? icon(b.icon) : null, b.label);
        el.onclick = () => done(b.value === undefined ? b.label : b.value);
        if (b.primary) primary = el;
        acts.append(el);
      }
      box.append(acts);
      layer(anchor).append(box);
      place(box, anchor, 6);
      const away = (e) => { if (!box.contains(e.target)) done(null); };
      const key = (e) => {
        if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); done(null); }
        else if (e.key === "Enter" && primary && !e.isComposing && document.activeElement?.tagName !== "BUTTON") { e.preventDefault(); primary.click(); }
      };
      function done(v) {
        box.remove();
        document.removeEventListener("pointerdown", away, true);
        document.removeEventListener("keydown", key, true);
        asking = null;
        resolve(v);
      }
      setTimeout(() => document.addEventListener("pointerdown", away, true), 0);
      document.addEventListener("keydown", key, true);
      asking = done;
      (primary ?? acts.lastElementChild)?.focus();
    });
  }

  // ── ? help tips (any .help[data-tip]) ──────────────────────────────
  let tipFor = null;
  function hideTip() { document.querySelectorAll(".tip.lore-tip").forEach((t) => t.remove()); tipFor?.setAttribute("aria-expanded", "false"); tipFor = null; }
  function wireTips() {
    document.addEventListener("click", (e) => {
      const b = e.target.closest?.(".help[data-tip]");
      if (!b) { if (!e.target.closest?.(".tip")) hideTip(); return; }
      e.preventDefault();
      const again = tipFor === b;
      hideTip();
      if (again) return;
      const t = h("div", { class: "tip lore-tip", role: "tooltip" }, b.dataset.tip);
      layer(b).append(t);
      place(t, b, 6);
      b.setAttribute("aria-expanded", "true");
      tipFor = b;
    });
    document.addEventListener("keydown", (e) => { if (e.key === "Escape" && tipFor) hideTip(); });
  }

  // ── numbered block: 01 名稱 (count) ? ± ────────────────────────────
  function block({ n, title, count, help, open = true, key } = {}) {
    const stored = (() => { try { return key ? localStorage.getItem(`lore.block.${key}`) : null; } catch { return null; } })();
    const isOpen = stored == null ? open : stored === "1";
    const toggle = h("button", { type: "button", class: "toggle", "aria-label": isOpen ? "收起" : "展開", "aria-expanded": String(isOpen) }, isOpen ? "−" : "+");
    const c = h("span", { class: "c" }, count ?? "");
    const el = h("section", { class: "block", "data-open": String(isOpen) },
      h("div", { class: "block-head" }, n ? h("span", { class: "n" }, n) : null, h("h3", {}, title), c, help ? h("button", { type: "button", class: "help", "data-tip": help, "aria-label": "說明" }, "?") : null, toggle));
    const body = h("div", { class: "block-body" });
    el.append(body);
    el.body = body;
    el.setCount = (v) => { c.textContent = v ?? ""; };
    el.setOpen = (on) => {
      el.dataset.open = String(on);
      toggle.textContent = on ? "−" : "+";
      toggle.setAttribute("aria-expanded", String(on));
      toggle.setAttribute("aria-label", on ? "收起" : "展開");
      try { if (key) localStorage.setItem(`lore.block.${key}`, on ? "1" : "0"); } catch {}
    };
    toggle.onclick = () => el.setOpen(el.dataset.open !== "true");
    el.querySelector("h3").onclick = () => el.setOpen(el.dataset.open !== "true");
    return el;
  }

  // ── shortcuts: from aria-labels that end in keys ───────────────────
  // "復原 ⌘Z", "復原「移動」(⌘Z)", "選取 V", "吸附 ⌥", "粗體　Ctrl+B", "關閉 Esc"
  const KEY = /^(?:[⌘⇧⌥⌃]+\S{0,8}|(?:Ctrl|Alt|Shift|Cmd|Option)\+\S+|Esc|Enter|↵|Tab|Space|Del|⌫|F\d{1,2}|[A-Z0-9]|[[\]/\\,.;'=+\-←→↑↓])$/;
  function parseKeys(label) {
    const s = String(label ?? "").replace(/「[^」]*」/g, " ").replace(/\s+$/, "");
    let m = /^(.*?)\s*[(（]\s*([^()（）]+?)\s*[)）]$/.exec(s);
    if (m && m[2].split(/\s*[/／]\s*/).every((k) => KEY.test(k))) return { label: m[1].trim(), keys: m[2] };
    m = /^(.*?)[\s　]+(\S+)$/.exec(s);
    if (m && m[1].trim() && KEY.test(m[2])) return { label: m[1].trim(), keys: m[2] };
    return null;
  }
  const GROUPS = [[".top", "上方列"], [".capsule, .writebar, [role=toolbar]", "工具"], [".drawer-left, aside.left", "左側"], [".drawer-right, aside.side, .side", "右側"], [".statusbar", "狀態列"]];
  function shortcuts(root = document) {
    const out = [], seen = new Set();
    const add = (group, label, keys) => {
      const k = `${label}\u0000${keys}`;
      if (seen.has(k)) return;
      seen.add(k);
      out.push({ group, label, keys });
    };
    for (const el of root.querySelectorAll("[aria-label], [data-keys]")) {
      if (el.closest(".lore-settings")) continue;
      const p = el.dataset.keys ? { label: el.getAttribute("aria-label") ?? el.textContent.trim(), keys: el.dataset.keys } : parseKeys(el.getAttribute("aria-label"));
      if (!p) continue;
      const g = el.closest("[data-keys-group]")?.dataset.keysGroup ?? GROUPS.find(([sel]) => el.closest(sel))?.[1] ?? "其他";
      add(g, p.label, p.keys);
    }
    for (const k of S.opts?.keys ?? []) add(k.group ?? "其他", k.label, k.keys);
    if (S.opts?.save) add("其他", "儲存", "⌘S");
    if (S.opts?.search) add("其他", "搜尋", "⌘F");
    add("其他", "設定", "⌘,");
    const order = ["上方列", "工具", "左側", "右側", "狀態列", "其他"];
    return out.sort((a, b) => (order.indexOf(a.group) + 99) % 99 - (order.indexOf(b.group) + 99) % 99);
  }

  // ── engines (模型 tab, engine chip): GET/PUT /api/engines ───────────
  const ROUTE_LABEL = { auto: "自動", cloud: "雲", local: "地" };
  const ENGINE_LABEL = { local: "地", claude: "雲" };
  const engines = {
    data: null,
    async get(force) {
      if (this.data && !force) return this.data;
      try {
        const r = await fetch("/api/engines", { cache: "no-store" });
        this.data = r.ok ? await r.json() : null;
      } catch { this.data = null; }
      return this.data;
    },
    async put(patch) {
      const r = await fetch("/api/engines", { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(patch) });
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error ?? `${r.status}`);
      this.data = await r.json();
      document.dispatchEvent(new CustomEvent("lore-engines", { detail: this.data }));
      return this.data;
    },
  };
  function seg(options, value, onPick, label) {
    const s = h("div", { class: "seg", role: "group", "aria-label": label });
    for (const [v, text] of options) {
      const b = h("button", { type: "button", "aria-pressed": String(v === value) }, text);
      b.onclick = async () => {
        if (b.getAttribute("aria-pressed") === "true") return;
        const prev = [...s.children].map((x) => x.getAttribute("aria-pressed"));
        s.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
        try { await onPick(v); } catch (e) {
          [...s.children].forEach((x, i) => x.setAttribute("aria-pressed", prev[i]));
          toast(e.message, { err: true });
        }
      };
      s.append(b);
    }
    return s;
  }
  const vision = (ok) => h("span", { class: ok ? "yes" : "no", title: "看圖" }, `看圖 ${ok ? "✓" : "✗"}`);
  async function buildModels(el) {
    el.textContent = "";
    const d = await engines.get(true);
    if (!d) { el.append(h("div", { class: "set-empty" }, "—")); return; }
    const conn = h("div", { class: "set-sec" }, h("div", { class: "set-label" }, "連線"));
    const row = (k, ...v) => h("div", { class: "set-row" }, h("span", { class: "k" }, k), ...v);
    const L = d.local ?? {}, C = d.claude ?? {};
    const lv = h("span", { class: "v mono" }, L.label ?? L.model ?? "—");
    const lsel = L.models?.length > 1 ? h("select", { class: "set-input", "aria-label": "本機模型" }, ...L.models.map((m) => h("option", { value: m.id, selected: m.id === L.model }, m.label ?? m.id))) : null;
    if (lsel) lsel.onchange = () => engines.put({ local: { model: lsel.value } }).then(() => buildModels(el)).catch((e) => toast(e.message, { err: true }));
    conn.append(row("本機", lsel ?? lv, h("span", { class: "chip" + (L.state === "ready" ? " ok" : "") }, STATE[L.state] ?? "—"), vision(!!L.vision),
      S.opts?.modelSetup ? h("button", { type: "button", class: "btn small", onclick: () => S.opts.modelSetup("local") }, L.state === "ready" ? "切換…" : "下載…") : null));
    const csel = C.models?.length ? h("select", { class: "set-input", "aria-label": "Claude 模型" }, ...C.models.map((m) => h("option", { value: m.id, selected: m.id === C.model }, m.label ?? m.id))) : h("span", { class: "v mono" }, C.model ?? "—");
    if (csel.tagName === "SELECT") csel.onchange = () => engines.put({ claude: { model: csel.value } }).catch((e) => toast(e.message, { err: true }));
    const vias = Object.entries(C.vias ?? {}).filter(([, ok]) => ok).map(([v]) => [v, v === "subscription" ? "訂閱" : "金鑰"]);
    conn.append(row("Claude", csel, vias.length > 1 ? seg(vias, C.via, (v) => engines.put({ claude: { via: v } }), "連線方式") : h("span", { class: "chip" + (C.state === "ready" ? " ok" : "") }, C.state === "ready" ? (C.via === "api" ? "金鑰" : "訂閱") : STATE[C.state] ?? "—"), vision(!!C.vision),
      S.opts?.modelSetup ? h("button", { type: "button", class: "btn small", onclick: () => S.opts.modelSetup("claude") }, C.state === "ready" ? "更換…" : "登入…") : null));
    const route = h("div", { class: "set-sec" }, h("div", { class: "set-label" }, "走向"));
    for (const c of d.classes ?? []) {
      route.append(row(c.label, seg([["auto", "自動"], ["cloud", "雲"], ["local", "地"]], c.route ?? "auto", (v) => engines.put({ routes: { [c.id]: v } }), c.label),
        h("span", { class: "v no", title: "自動" }, (c.auto ?? []).map((x) => ENGINE_LABEL[x] ?? x).join(" → "))));
    }
    el.append(conn, route);
  }
  const STATE = { ready: "可用", starting: "啟動中", off: "未連線", missing: "未安裝", error: "錯誤" };
  /** A feature's own engine choice: 自動 · Sonnet. onChange(v) or PUT overrides. */
  function engineChip({ task, value = "auto", used = "", onChange, label = "模型" } = {}) {
    const b = h("button", { type: "button", class: "engine-chip", "aria-haspopup": "menu", "aria-label": label });
    const paint = () => {
      b.textContent = "";
      b.classList.toggle("set", value !== "auto");
      b.append(ROUTE_LABEL[value] ?? value);
      if (used) b.append(h("span", { class: "used" }, `· ${used}`));
    };
    b.set = (v, u = used) => { value = v ?? value; used = u; paint(); };
    b.onclick = () => menu(b, Object.entries(ROUTE_LABEL).map(([v, text]) => ({ label: text, current: v === value, run: async () => {
      const old = value;
      b.set(v);
      try { await (onChange ? onChange(v) : engines.put({ overrides: { [task]: v } })); } catch (e) { b.set(old); toast(e.message, { err: true }); }
    } })));
    paint();
    return b;
  }

  // ── settings (the gear) ────────────────────────────────────────────
  const SHARED_TABS = [
    { id: "general", label: "一般", build: (el) => (S.opts?.general ? S.opts.general(el) : el.append(h("div", { class: "set-empty" }, "—"))) },
    { id: "look", label: "外觀", build: (el) => { const p = h("div"); el.append(p); LoreTheme.picker(p); if (S.opts?.look) { const x = h("div", { class: "set-sec" }); el.append(x); S.opts.look(x); } } },
    { id: "model", label: "模型", build: buildModels },
    { id: "sync", label: "同步", build: (el) => S.opts.sync(el), when: () => !!S.opts?.sync },
    { id: "keys", label: "快捷鍵", build: buildKeys },
  ];
  function buildKeys(el) {
    const list = shortcuts();
    let g = null, dl = null;
    for (const k of list) {
      if (k.group !== g) { g = k.group; el.append(h("div", { class: "set-label kbd-group" }, g)); dl = h("dl", { class: "kbd-list" }); el.append(dl); }
      dl.append(h("dt", {}, k.label), h("dd", {}, h("kbd", {}, k.keys)));
    }
  }
  let dlg = null;
  function settings(arg) {
    if (Array.isArray(arg)) { S.opts = { ...(S.opts ?? {}), settingsTabs: arg }; arg = undefined; }
    const tabs = [...SHARED_TABS.filter((t) => !t.when || t.when()), ...(S.opts?.settingsTabs ?? [])];
    if (!dlg) {
      dlg = h("dialog", { class: "dialog lore-settings", "aria-label": "設定" });
      dlg.addEventListener("close", () => { S.gear?.setAttribute("aria-expanded", "false"); });
      document.body.append(dlg);
    }
    dlg.textContent = "";
    const tl = h("div", { class: "tabs", role: "tablist" });
    const body = h("div", { class: "dlg-body", role: "tabpanel" });
    const show = (t) => {
      tl.querySelectorAll("button").forEach((b) => b.setAttribute("aria-selected", String(b.dataset.tab === t.id)));
      body.textContent = "";
      body.dataset.tab = t.id;
      Promise.resolve(t.build(body)).then(() => paintIcons(body));
      try { localStorage.setItem("lore.settings.tab", t.id); } catch {}
    };
    for (const t of tabs) tl.append(h("button", { type: "button", role: "tab", dataset: { tab: t.id }, onclick: () => show(t) }, t.label));
    dlg.append(h("div", { class: "dlg" }, h("div", { class: "dlg-head" }, h("h2", {}, "設定"), tl, ico("x", "關閉 Esc", () => dlg.close(), { class: "ico close" })), body));
    let last = null;
    try { last = localStorage.getItem("lore.settings.tab"); } catch {}
    show(tabs.find((t) => t.id === arg) ?? tabs.find((t) => t.id === last) ?? tabs[0]);
    if (!dlg.open) dlg.showModal();
    S.gear?.setAttribute("aria-expanded", "true");
    return dlg;
  }

  // ── the top bar ────────────────────────────────────────────────────
  const S = { opts: null };
  const STATUS = { saved: "已儲存", saving: "儲存中", dirty: "未儲存", syncing: "同步中", unsynced: "未同步", error: "未能儲存" };
  function brandNode(brand) {
    const b = h("button", { type: "button", class: "brand", "aria-label": "首頁" });
    const parts = String(brand ?? "").split("/");
    parts.forEach((p, i) => { if (i) b.append(h("b", {}, "/")); b.append(p); });
    return b;
  }
  function mount(opts) {
    S.opts = opts;
    let top = $(opts.host);
    if (!top || top.tagName !== "HEADER") {
      const into = top ?? document.body;
      top = h("header", { class: "top" });
      into.prepend(top);
    }
    top.classList.add("top", "lore-top");
    top.setAttribute("data-tauri-drag-region", "");
    top.textContent = "";
    const slot = (name) => h("div", { class: `slot slot-${name}` });
    const lead = slot("lead"), primary = slot("primary"), right = slot("right");
    const drawers = opts.drawers ?? {};
    const toggles = {};
    const drawerEl = (w) => $(drawers[w]);
    function drawer(which, force) {
      const el = drawerEl(which);
      if (!el) return;
      const on = force ?? !el.classList.contains("open");
      el.classList.toggle("open", on);
      toggles[which]?.setAttribute("aria-pressed", String(on));
      if (on) { const other = which === "left" ? "right" : "left"; if (drawerEl(other)?.classList.contains("open") && innerWidth <= 760) drawer(other, false); }
    }
    for (const w of ["left", "right"]) {
      const el = drawerEl(w);
      if (!el) continue;
      el.classList.add(`drawer-${w}`);
      toggles[w] = ico(w === "left" ? drawers.leftIcon ?? "panelLeft" : drawers.rightIcon ?? "panel", (w === "left" ? drawers.leftLabel : drawers.rightLabel) ?? (w === "left" ? "導覽" : "面板"), () => drawer(w), { class: "ico narrow-only", "aria-pressed": "false" });
    }
    if (toggles.left) top.append(toggles.left);
    const brand = brandNode(opts.brand);
    brand.onclick = () => opts.home?.();
    top.append(brand);

    // 文件選單 名稱 ▾
    let nameEl = null, pick = null;
    if (opts.picker) {
      nameEl = h("span", { class: "name" }, opts.picker.name ?? "");
      pick = h("button", { type: "button", class: "picker", "aria-haspopup": "menu", "aria-expanded": "false", "aria-label": "文件" },
        h("span", { "data-icon": opts.picker.icon ?? "file" }), nameEl, h("span", { class: "caret", "data-icon": "chevron" }));
      pick.onclick = async () => {
        const recent = (await opts.picker.recent?.()) ?? [];
        const items = [...recent];
        const acts = opts.picker.actions ?? [];
        if (items.length && acts.length) items.push("-");
        items.push(...acts);
        if (opts.home && !acts.some((a) => a.label === "首頁")) items.push("-", { icon: "home", label: "首頁", run: opts.home });
        menu(pick, items);
      };
      top.append(pick);
    }
    // the status chip
    const chip = h("button", { type: "button", class: "fstate", hidden: true });
    let chipAction = null;
    chip.onclick = () => { if (chipAction) chipAction(chip.dataset.state); };
    top.append(chip, lead);

    // primary 2–4, AI last
    for (const p of opts.primary ?? []) primary.append(ico(p.icon, p.label, runOrMenu(p), { id: p.id, "aria-haspopup": p.menu ? "menu" : null, "aria-pressed": p.pressed == null ? null : String(p.pressed) }));
    if (opts.ai) primary.append(ico(opts.ai.icon ?? "spark", opts.ai.label ?? "AI", runOrMenu(opts.ai), { id: opts.ai.id ?? "actAI", "aria-haspopup": opts.ai.menu ? "menu" : null }));
    if (toggles.right) primary.append(toggles.right);
    top.append(primary);

    // right: [undo redo] 對照表 · 匯出▾ · 齒輪
    let undoB = null, redoB = null;
    if (opts.undo) {
      undoB = ico("undo", "復原 ⌘Z", () => opts.undo.undo(), { disabled: true });
      redoB = ico("redo", "重做 ⇧⌘Z", () => opts.undo.redo(), { disabled: true });
      right.append(h("span", { class: "edit-only" }, undoB, redoB, h("span", { class: "vsep" })));
    }
    if (opts.notation) right.append(ico("book", "對照表", typeof opts.notation === "function" ? opts.notation : opts.notation.run, { id: "actNotation" }));
    if (opts.export?.length) right.append(ico("download", "匯出", (e) => menu(e.currentTarget, opts.export), { id: "actExport", "aria-haspopup": "menu" }));
    S.gear = ico("gear", "設定 ⌘,", () => settings(), { id: "actSettings", "aria-haspopup": "dialog" });
    right.append(S.gear);
    top.append(right);
    paintIcons(top);

    // keys: ⌘S ⌘F ⌘, (⌘Z stays the app's: it knows what is being edited)
    document.addEventListener("keydown", (e) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.isComposing) return;
      const k = e.key.toLowerCase();
      if (k === "f" && !e.shiftKey && opts.search) { e.preventDefault(); opts.search(); }
      else if (k === "s" && !e.shiftKey && opts.save) { e.preventDefault(); opts.save(); }
      else if (k === "," && !e.shiftKey) { e.preventDefault(); settings(); }
    });
    // back to wide: drawers close
    matchMedia("(max-width: 1100px)").addEventListener("change", (m) => { if (!m.matches) { drawer("left", false); drawer("right", false); } });

    const shell = {
      el: top,
      slots: { lead, primary, right },
      drawer,
      setName(n) { if (nameEl) { nameEl.textContent = n ?? ""; pick.title = n ?? ""; } },
      /** state: saved saving dirty syncing unsynced error | null (hidden). label/title/action override per call. */
      setStatus(state, o = {}) {
        chip.hidden = !state;
        if (!state) { chip.dataset.state = ""; return; }
        chip.className = `fstate ${state}`;
        chip.dataset.state = state;
        chip.textContent = "";
        const text = o.label ?? STATUS[state] ?? state;
        chip.append(h("i"), text);
        chip.title = o.title ?? "";
        chip.setAttribute("aria-label", text);
        chipAction = o.action ?? (opts.status?.onClick && (opts.status.clickable ?? ["dirty", "unsynced", "error"]).includes(state) ? opts.status.onClick : null);
        chip.classList.toggle("can-act", !!chipAction);
      },
      editing(on) { top.classList.toggle("editing", !!on); },
      setUndo({ undo, redo } = {}) {
        if (!undoB) return;
        undoB.disabled = !undo; redoB.disabled = !redo;
        undoB.setAttribute("aria-label", undo ? `復原「${undo}」⌘Z` : "復原 ⌘Z");
        redoB.setAttribute("aria-label", redo ? `重做「${redo}」⇧⌘Z` : "重做 ⇧⌘Z");
      },
      settings,
    };
    S.shell = shell;
    return shell;
  }

  window.LoreShell = { mount, toast, ask, menu, closeMenu, settings, engineChip, engines, block, shortcuts, parseKeys };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", wireTips); else wireTips();
})();

// ── links between the Lore apps: lore-<app>://… with an http fallback ──
//   lore-pc://open?path=<file>          http://127.0.0.1:7878/?open=<file>
//   lore-sc://deck/<name>               http://127.0.0.1:7880/?deck=<name>
//   lore-lr://paper/<sha>?page=<n>      http://127.0.0.1:7890/?paper=<sha>&page=<n>
//   lore-kb://node/<id>                 http://127.0.0.1:7700/#/node/<id>
//   lore-<app>://                       the app's home
// open(): the desktop shell's hook (window.LoreDesktop.openUrl) when there is
// one, else POST /api/links/open { url } on this app's server (it hands the URL
// to the OS), else the http address in a new window. The ports are the dev
// servers' defaults; a desktop app on a free port overrides LoreLinks.ports.
(() => {
  const enc = encodeURIComponent;
  const ports = { pc: 7878, sc: 7880, lr: 7890, kb: 7700 };
  const FORMS = {
    pc: (p) => (p.path ? [`open?path=${enc(p.path)}`, `/?open=${enc(p.path)}`] : ["", "/"]),
    sc: (p) => (p.deck ? [`deck/${enc(p.deck)}`, `/?deck=${enc(p.deck)}`] : ["", "/"]),
    lr: (p) => (p.paper ? [`paper/${enc(p.paper)}${p.page ? `?page=${enc(p.page)}` : ""}`, `/?paper=${enc(p.paper)}${p.page ? `&page=${enc(p.page)}` : ""}`] : ["", "/"]),
    kb: (p) => (p.node ? [`node/${enc(p.node)}`, `/#/node/${enc(p.node)}`] : ["", "/"]),
  };
  function url(app, params = {}) {
    const f = FORMS[app];
    if (!f) throw new Error(`unknown app ${app}`);
    const [s, path] = f(params);
    return { scheme: `lore-${app}://${s}`, http: `http://127.0.0.1:${ports[app]}${path}` };
  }
  async function open(app, params) {
    const u = url(app, params);
    if (window.LoreDesktop?.openUrl) return window.LoreDesktop.openUrl(u.scheme);
    try {
      const r = await fetch("/api/links/open", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ url: u.scheme }) });
      if (r.ok) return;
    } catch {}
    window.open(u.http, "_blank", "noopener");
  }
  window.LoreLinks = { url, open, ports };
})();
