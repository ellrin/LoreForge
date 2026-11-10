// LoreForge theme, shared by every app. Each server sends this file as
// /theme.js with one line in front: `window.LORE_THEME = "<id>" | null;`
// (the shared choice in lore-home, private/shared/ui.toml). Load it in <head>
// with a plain <script src="theme.js"></script>, after theme.css, so the first
// paint already has data-theme and data-mode on <html>.
//
//   LoreTheme.picker(el)   fills el with the 外觀 picker (light first, then dark;
//                          單色 / 搭配色; hover or focus previews, click applies)
//   LoreTheme.current()    the chosen id
//   LoreTheme.choose(id)   apply + save (PUT /api/theme), other apps follow
//   LoreTheme.refresh()    read the shared choice again (also done on focus)
//   "lore-theme" event on document whenever the shown theme changes.
//
// localStorage "lore.theme" is only a cache against a flash of the wrong theme.
// The ids are also in theme.css and lore-home (theme::THEMES); a test keeps
// the three in step.
(() => {
  const THEMES = [
    { id: "hacker", name: "駭客終端", dark: true, multi: false, sw: ["#121712", "#16c94d", "#39ff6a", "#a8ffbc"] },
    { id: "amber", name: "琥珀終端", dark: true, multi: false, sw: ["#1c1a16", "#ff8a1f", "#ffb000", "#ffd98a"] },
    { id: "ice", name: "冰晶藍", dark: true, multi: false, sw: ["#172028", "#2fb2e6", "#5ce1ff", "#b8f2ff"] },
    { id: "ultraviolet", name: "紫外線", dark: true, multi: false, sw: ["#1b1530", "#8c5cff", "#ae7aff", "#dcc8ff"] },
    { id: "redalert", name: "紅色警戒", dark: true, multi: false, sw: ["#1d1517", "#e8323f", "#ff4752", "#ffa3a8"] },
    { id: "neon", name: "霓虹夜市", dark: true, multi: true, sw: ["#161731", "#ff3fb4", "#2de2e6", "#ffd23f"] },
    { id: "shibuya", name: "澀谷雨夜", dark: true, multi: true, sw: ["#151c26", "#ff5fa2", "#4fd8ff", "#a98cff"] },
    { id: "hongkong", name: "香港霓虹", dark: true, multi: true, sw: ["#122220", "#ff5147", "#2ee6a6", "#ffc845"] },
    { id: "synthwave", name: "落日合成", dark: true, multi: true, sw: ["#1e1329", "#ff8a3d", "#ff4fa0", "#b691ff"] },
    { id: "aurora", name: "極光", dark: true, multi: true, sw: ["#121b2a", "#4cf5a8", "#46c8ff", "#d58cff"] },
    { id: "lavender", name: "薰衣草", dark: false, multi: false, sw: ["#ebe7f6", "#b7abdb", "#6f4fd0", "#5232ad"] },
    { id: "lemon", name: "檸檬塔", dark: false, multi: false, sw: ["#faf2d2", "#ffdf5e", "#f5c518", "#e9a200"] },
    { id: "mint", name: "薄荷", dark: false, multi: false, sw: ["#e2f3eb", "#9eccb5", "#0b7a5d", "#08604a"] },
    { id: "blueberry", name: "藍莓", dark: false, multi: false, sw: ["#e3e8f6", "#a6b3db", "#3a4bb0", "#2a3888"] },
    { id: "strawberry", name: "草莓", dark: false, multi: false, sw: ["#f9e4ea", "#d9a2b4", "#c8254f", "#9c1a3c"] },
    { id: "giftbox", name: "甜點禮盒", dark: false, multi: true, sw: ["#f8efe4", "#e0457b", "#3ec7a0", "#f5c518"] },
    { id: "peachlavender", name: "蜜桃薰衣草", dark: false, multi: true, sw: ["#fae7da", "#ff8a5c", "#9b7fe6", "#e86a9e"] },
    { id: "sakuramatcha", name: "櫻花抹茶", dark: false, multi: true, sw: ["#f5eaee", "#f08cae", "#8dba4a", "#8c4f42"] },
    { id: "blueberrylemon", name: "藍莓檸檬", dark: false, multi: true, sw: ["#e6ebf7", "#4a5fd0", "#f5c518", "#a462d4"] },
    { id: "caramelpistachio", name: "焦糖開心果", dark: false, multi: true, sw: ["#f2e7d6", "#d68b3a", "#93c572", "#cf6f84"] },
  ];
  const DEFAULT = "giftbox";
  const KEY = "lore.theme";
  const root = document.documentElement;
  const themeOf = (id) => THEMES.find((t) => t.id === id);

  // The three palettes of the first version (× light/dark) → the nearest theme.
  const OLD = {
    orange: { dark: "amber", light: "peachlavender" },
    night: { dark: "neon", light: "lavender" },
    terminal: { dark: "hacker", light: "mint" },
  };
  const nearest = (theme, mode) => (themeOf(theme) ? theme : OLD[theme]?.[mode === "light" ? "light" : "dark"] ?? null);

  const ls = {
    // Apps stored their values either raw or as JSON strings.
    get(k) {
      try {
        const v = localStorage.getItem(k);
        if (v == null) return null;
        try { const j = JSON.parse(v); return typeof j === "string" ? j : v; } catch { return v; }
      } catch { return null; }
    },
    set(k, v) { try { localStorage.setItem(k, v); } catch {} },
    del(k) { try { localStorage.removeItem(k); } catch {} },
  };

  // What an app remembered by itself before the shared choice existed.
  const LEGACY = [["pc.theme", "pc.mode"], ["sc.theme", "sc.mode"], ["kb.theme", "kb.mode"], [null, "lr.mode"]];
  function legacy() {
    for (const [tk, mk] of LEGACY) {
      const t = tk ? ls.get(tk) : null, m = ls.get(mk);
      if (t == null && m == null) continue;
      if (tk) ls.del(tk);
      ls.del(mk);
      const id = nearest(t ?? "orange", m ?? "dark");
      if (id) return id;
    }
    return null;
  }

  const served = window.LORE_THEME !== undefined;   // false when opened as a plain file (demo)
  let chosen = null;      // the shared choice, when there is one
  let shown = null;

  function show(id) {
    const t = themeOf(id) ?? themeOf(DEFAULT);
    if (shown === t.id && root.dataset.theme === t.id) return;
    shown = t.id;
    root.dataset.theme = t.id;
    root.dataset.mode = t.dark ? "dark" : "light";
    document.dispatchEvent(new CustomEvent("lore-theme", { detail: t }));
  }
  const current = () => chosen ?? DEFAULT;
  function mark() {
    document.querySelectorAll(".theme-card").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.theme === current())));
  }
  function set(id) {
    chosen = themeOf(id) ? id : null;
    if (chosen) ls.set(KEY, chosen); else ls.del(KEY);
    show(current());
    mark();
  }
  async function save(id) {
    if (!served) return;
    try { await fetch("/api/theme", { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify({ theme: id }) }); } catch {}
  }
  function choose(id) {
    if (!themeOf(id)) return;
    set(id);
    return save(id);
  }
  async function refresh() {
    if (!served) return;
    try {
      const r = await fetch("/api/theme", { cache: "no-store" });
      if (!r.ok) return;
      const id = nearest((await r.json()).theme, "dark");
      if (id && id !== chosen) set(id);
    } catch {}
  }

  // First paint: the shared choice. Without one: what this app remembered
  // before (it becomes the shared choice), else the default. A page opened
  // without a server (demo) keeps its own choice in localStorage.
  chosen = nearest(window.LORE_THEME, "dark");
  if (chosen) ls.set(KEY, chosen);
  else if (!served) chosen = themeOf(ls.get(KEY)) ? ls.get(KEY) : null;
  else {
    ls.del(KEY);
    chosen = legacy();
    if (chosen) { ls.set(KEY, chosen); save(chosen); }
  }
  show(current());

  addEventListener("focus", refresh);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) refresh(); });
  addEventListener("pageshow", (e) => { if (e.persisted) refresh(); });

  function picker(host) {
    host.classList.add("theme-pick");
    host.textContent = "";
    const el = (tag, cls, text) => {
      const e = document.createElement(tag);
      if (cls) e.className = cls;
      if (text) e.textContent = text;
      return e;
    };
    for (const dark of [false, true]) {
      host.append(el("h4", "", dark ? "深色" : "淺色"));
      for (const multi of [false, true]) {
        const grid = el("div", "theme-grid");
        grid.setAttribute("role", "group");
        grid.setAttribute("aria-label", `${dark ? "深色" : "淺色"}・${multi ? "搭配色" : "單色"}`);
        for (const t of THEMES.filter((t) => t.dark === dark && t.multi === multi)) {
          const b = el("button", "theme-card");
          b.type = "button";
          b.dataset.theme = t.id;
          b.setAttribute("aria-label", t.name);
          const sw = el("span", "sw");
          for (const c of t.sw) { const i = el("i"); i.style.background = c; sw.append(i); }
          b.append(sw, el("span", "nm", t.name));
          b.onmouseenter = b.onfocus = () => show(t.id);
          b.onmouseleave = b.onblur = () => show(current());
          b.onclick = () => choose(t.id);
          grid.append(b);
        }
        host.append(el("h5", "", multi ? "搭配色" : "單色"), grid);
      }
    }
    mark();
  }

  window.LoreTheme = { THEMES, DEFAULT, themeOf, nearest, current, show, choose, refresh, picker };
})();
