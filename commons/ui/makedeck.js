// 做成簡報: the open paper as a paper-report deck in SlideComposer, in one action — the same
// button and card in every app that has papers (PaperComposer, LoreReader). Served as
// `/makedeck.js` by `lore_shell::FILES`; loaded after shell.js. An app mounts it once:
//
//   LoreMakeDeck.mount({
//     before: "#actAsk",                   // the top-bar button it sits before (the AI button)
//     source: async () => ({ kind: "lr", sha }),   // SlideComposer's /api/todeck source; null = no paper open
//     key: "lr.todeck",                    // localStorage key for length / language / base
//   });
//
// The card picks the length, the language and SlideComposer's base; 先看大綱 (primary) or
// 直接產生 asks SlideComposer (POST /api/todeck) to make the deck — the paper is only read —
// then opens SlideComposer's job view. SlideComposer not running: it is opened and waited for.
"use strict";

window.LoreMakeDeck = (() => {
  const LEN = [["short", "短 10"], ["standard", "標準 15–20"], ["long", "長"]];
  const LANG = [["zh", "中文"], ["en", "English"]];
  const WAIT_MS = 30000;
  const sc = () => `http://127.0.0.1:${LoreLinks.ports.sc}`;
  let opts = null, pref = null, btn = null, card = null, job = null;

  function h(tag, attrs = {}, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c);
    return e;
  }

  function store(key) {
    let v = {};
    try { v = JSON.parse(localStorage.getItem(key) ?? "{}") ?? {}; } catch {}
    return { get: (k, d) => v[k] ?? d, set: (k, x) => { v[k] = x; try { localStorage.setItem(key, JSON.stringify(v)); } catch {} } };
  }

  const CSS = `
#actDeck { position: relative; color: var(--accent-2-text); }
#actDeck svg.i path, #actDeck svg.i rect { transition: transform .22s cubic-bezier(.3,.7,.3,1.3), fill .22s; }
#actDeck:hover:not(:disabled) { color: var(--accent-2-text); background: var(--accent-2-soft); border-color: color-mix(in srgb, var(--accent-2) 55%, transparent); }
#actDeck:hover:not(:disabled) .deck-3 { transform: translate(1.2px, -1.2px); }
#actDeck:hover:not(:disabled) .deck-2 { transform: translate(.6px, -.6px); }
#actDeck:hover:not(:disabled) .deck-1 { fill: var(--accent-2-soft); }
#actDeck[aria-expanded="true"] { color: var(--accent-2-text); background: var(--accent-2-soft); border-color: var(--accent-2); }
#actDeck.busy .deck-1 { animation: tdk-pulse 1.1s ease-in-out infinite; }
@keyframes tdk-pulse { 50% { fill: var(--accent-2-soft); } }
@media (prefers-reduced-motion: reduce) {
  #actDeck svg.i path, #actDeck svg.i rect { transition: none; }
  #actDeck:hover:not(:disabled) .deck-3, #actDeck:hover:not(:disabled) .deck-2 { transform: none; }
  #actDeck.busy .deck-1 { animation: none; fill: var(--accent-2-soft); }
}
.tdk-card { position: fixed; z-index: 60; width: 300px; max-width: calc(100vw - 16px); background: var(--panel); color: var(--text); border: 1px solid var(--line-strong); border-top: 2px solid var(--accent-2); border-radius: var(--r); box-shadow: 0 10px 30px rgba(0,0,0,.18); padding: 12px 14px 12px; display: grid; gap: 10px; font-size: 13px; }
.tdk-card .tdk-head { display: flex; align-items: center; gap: 8px; font-weight: 600; }
.tdk-card .tdk-head svg.i { color: var(--accent-2-text); }
.tdk-card .tdk-row { display: grid; gap: 5px; }
.tdk-card .tdk-label { font: 500 11px var(--mono); color: var(--muted); letter-spacing: .04em; }
.tdk-card .seg { width: 100%; }
.tdk-card .seg button { flex: 1; padding: 5px 6px; }
.tdk-card .tdk-bases { display: flex; flex-wrap: wrap; gap: 6px; }
.tdk-card .tdk-bases button { font-size: 12.5px; padding: 4px 9px; border: 1px solid var(--line-strong); border-radius: var(--r); background: var(--surface); color: var(--muted); }
.tdk-card .tdk-bases button:hover:not([aria-pressed="true"]) { color: var(--text); background: var(--surface-hover); }
.tdk-card .tdk-bases button[aria-pressed="true"] { color: var(--text); border-color: var(--accent-text); background: var(--accent-soft); font-weight: 600; }
.tdk-card .tdk-note { color: var(--muted); font-size: 12px; }
.tdk-card .tdk-foot { display: flex; align-items: center; gap: 8px; justify-content: flex-end; }
.tdk-card .tdk-bar { flex: 1; height: 3px; border-radius: 2px; background: var(--accent-2-soft); overflow: hidden; }
.tdk-card .tdk-bar::before { content: ""; display: block; width: 35%; height: 100%; background: var(--accent-2); animation: tdk-run 1.4s ease-in-out infinite; }
@keyframes tdk-run { from { transform: translateX(-100%); } to { transform: translateX(290%); } }
@media (prefers-reduced-motion: reduce) { .tdk-card .tdk-bar::before { animation: none; width: 100%; opacity: .5; } }
`;

  function place() {
    if (!card || !btn) return;
    const r = btn.getBoundingClientRect();
    card.style.top = `${r.bottom + 6}px`;
    card.style.left = `${Math.max(8, Math.min(r.left + r.width / 2 - card.offsetWidth / 2, innerWidth - card.offsetWidth - 8))}px`;
  }

  function close() {
    if (!card) return;
    card.remove();
    card = null;
    btn?.setAttribute("aria-expanded", "false");
    document.removeEventListener("pointerdown", away, true);
    document.removeEventListener("keydown", esc, true);
    removeEventListener("resize", place);
  }
  const away = (e) => { if (card && !card.contains(e.target) && !btn.contains(e.target)) close(); };
  const esc = (e) => { if (e.key === "Escape" && card) { e.stopPropagation(); close(); btn.focus(); } };

  async function toggle() {
    if (card) return close();
    if (!job && !(await opts.source({ check: true }).catch(() => null))) { LoreShell.toast("先開一篇論文"); return; }
    card = h("div", { class: "tdk-card", role: "dialog", "aria-label": "做成簡報" });
    document.body.append(card);
    btn.setAttribute("aria-expanded", "true");
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("keydown", esc, true);
    addEventListener("resize", place);
    draw();
  }

  function seg(list, key, def) {
    const cur = pref.get(key, def);
    const s = h("div", { class: "seg", role: "radiogroup" });
    for (const [v, label] of list) {
      s.append(h("button", { type: "button", "aria-pressed": String(v === cur), onclick: (e) => {
        pref.set(key, v);
        s.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(b === e.currentTarget)));
      } }, label));
    }
    return s;
  }

  /** SlideComposer answers: its port from the desktop apps' file, or the dev server's default. */
  async function alive() {
    await LoreLinks.refresh();
    try { return (await fetch(`${sc()}/api/info`, { cache: "no-store" })).ok; } catch { return false; }
  }

  /** Running already, else opened (lore-sc://, which starts the app) and waited for. */
  async function reach(onWait) {
    if (await alive()) return;
    onWait?.();
    await LoreLinks.open("sc");
    const end = Date.now() + WAIT_MS;
    while (Date.now() < end) {
      await new Promise((ok) => setTimeout(ok, 600));
      if (await alive()) return;
    }
    throw Object.assign(new Error("SlideComposer 沒有開啟"), { open: true });
  }

  async function bases() {
    const r = await fetch(`${sc()}/api/info`);
    if (!r.ok) throw new Error(String(r.status));
    const info = await r.json();
    return info.style_list ?? (info.styles ?? []).map((id) => ({ id, name: id }));
  }

  async function draw() {
    if (!card) return;
    card.textContent = "";
    card.append(h("div", { class: "tdk-head" }, icon("deckOutline"), "做成簡報"));
    if (job) {
      card.append(h("div", { class: "tdk-foot" }, h("div", { class: "tdk-bar" }), h("span", { class: "tdk-note" }, job.note ?? "產生中…")));
      place();
      return;
    }
    card.append(
      h("div", { class: "tdk-row" }, h("span", { class: "tdk-label" }, "長度"), seg(LEN, "length", "standard")),
      h("div", { class: "tdk-row" }, h("span", { class: "tdk-label" }, "語言"), seg(LANG, "lang", "zh")),
    );
    const row = h("div", { class: "tdk-row" }, h("span", { class: "tdk-label" }, "底版"));
    const list = h("div", { class: "tdk-bases", role: "radiogroup" }, h("span", { class: "tdk-note" }, "…"));
    row.append(list);
    const go = h("button", { type: "button", class: "btn primary small", disabled: true, onclick: () => create(undefined, "review") }, "先看大綱");
    const direct = h("button", { type: "button", class: "btn small", disabled: true, onclick: () => create(undefined, "direct") }, "直接產生");
    card.append(row, h("div", { class: "tdk-foot" }, direct, go));
    place();
    let styles;
    try {
      await reach(() => { list.textContent = ""; list.append(h("span", { class: "tdk-note" }, "正在開啟 SlideComposer…")); place(); });
      styles = await bases();
    } catch {
      if (!card) return;
      list.textContent = "";
      list.append(h("span", { class: "tdk-note" }, "SlideComposer 沒有開啟"));
      direct.remove();
      go.replaceWith(
        h("button", { type: "button", class: "btn small", onclick: () => LoreLinks.open("sc") }, "開啟 SlideComposer"),
        h("button", { type: "button", class: "btn small primary", onclick: draw }, "重試"),
      );
      place();
      return;
    }
    if (!card) return;
    list.textContent = "";
    const want = styles.some((s) => s.id === pref.get("style")) ? pref.get("style") : styles[0]?.id;
    for (const st of styles) {
      list.append(h("button", { type: "button", title: st.id, "aria-pressed": String(st.id === want), onclick: (e) => {
        pref.set("style", st.id);
        list.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(b === e.currentTarget)));
      } }, st.name || st.id));
    }
    if (want) pref.set("style", want);
    go.disabled = direct.disabled = !styles.length;
    place();
    go.focus();
  }

  /** Ask SlideComposer for the deck; `engine` = 改用本機／改用 Claude after a route failed. */
  async function create(engine, mode = "review") {
    if (job) return;
    job = {};
    btn.classList.add("busy");
    draw();
    try {
      const source = await opts.source({});
      if (!source) throw new Error("先開一篇論文");
      const body = { source, length: pref.get("length", "standard"), lang: pref.get("lang", "zh"), style: pref.get("style"), engine, mode };
      await reach(() => { job.note = "正在開啟 SlideComposer…"; draw(); });
      let r;
      try {
        r = await fetch(`${sc()}/api/todeck`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
      } catch { throw Object.assign(new Error("SlideComposer 沒有開啟"), { open: true }); }
      const v = await r.json().catch(() => ({}));
      if (!r.ok) throw Object.assign(new Error(v.error ?? `SlideComposer：${r.status}`), { alt: v.alternative });
      close();
      if (v.job) {
        // the work runs in SlideComposer as a job; its progress view opens there
        LoreLinks.open("sc", { job: v.job });
        LoreShell.toast("在 SlideComposer 進行中", { action: { label: "前往", run: () => LoreLinks.open("sc", { job: v.job }) }, ms: 8000 });
        return;
      }
      LoreShell.toast(`簡報已建立（${v.slides} 頁）`, { action: { label: "開啟", run: () => LoreLinks.open("sc", { deck: v.name }) }, ms: 8000 });
      LoreLinks.open("sc", { deck: v.name });
    } catch (e) {
      const action = e.alt ? { label: e.alt.label, run: () => create(e.alt.engine, mode) } : e.open ? { label: "開啟 SlideComposer", run: () => LoreLinks.open("sc") } : null;
      LoreShell.toast(e.message, { err: true, action, ms: 9000 });
    } finally {
      job = null;
      btn.classList.remove("busy");
      if (card) draw();
    }
  }

  function attach() {
    const anchor = document.querySelector(opts.before);
    if (!anchor) return false;
    if (document.getElementById("actDeck")) return true;
    if (!document.getElementById("makedeckCss")) document.head.append(h("style", { id: "makedeckCss" }, CSS));
    btn = h("button", { type: "button", class: "ico", id: "actDeck", "aria-label": "做成簡報", title: "做成簡報", "aria-haspopup": "dialog", "aria-expanded": "false", onclick: toggle });
    btn.append(icon("deckOutline"));
    anchor.before(btn);
    return true;
  }

  /** Put the button before `before` (now, or once the shell has built the top bar). */
  function mount(o) {
    opts = o;
    pref = store(o.key ?? "lore.todeck");
    if (!attach()) {
      const mo = new MutationObserver(() => { if (attach()) mo.disconnect(); });
      mo.observe(document.documentElement, { childList: true, subtree: true });
    }
  }

  return { mount, open: () => toggle(), create };
})();
