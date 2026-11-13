// 寫 review: the open paper → a review written for the author (PaperComposer's 寫書,
// docs/book.md). The same kind of button and card as 做成簡報 (makedeck.js). Served as
// `/makebook.js` by `lore_shell::FILES`; loaded after shell.js. An app mounts it once:
//
//   LoreMakeBook.mount({
//     before: "#actAsk",                         // the top-bar button it sits before
//     source: async () => ({ kind: "lr", sha }), // PaperComposer's /api/book source; null = no paper open
//     key: "lr.tobook",                          // localStorage key for book / length / language
//     extras: () => ({ terms: [{ term, n }], stuck: [paragraph id], questions: [text] }),  // optional
//   });
//
// The card picks the book (新書 or one of PaperComposer's books), the length and the language;
// 先看大綱 (primary) or 直接產生 asks PaperComposer (POST /api/book), then opens its job view
// (LoreLinks.open("pc", { bookjob })). PaperComposer not running: it is opened and waited for.
"use strict";

window.LoreMakeBook = (() => {
  const LEN = [["short", "短"], ["standard", "標準"], ["long", "長"]];
  const LANG = [["en-zh", "英中"], ["en", "English"], ["zh", "中文"]];
  const WAIT_MS = 30000;
  const pc = () => `http://127.0.0.1:${LoreLinks.ports.pc}`;
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
#actBook { position: relative; color: var(--accent-2-text); }
#actBook svg.i path { transition: transform .22s cubic-bezier(.3,.7,.3,1.3); }
#actBook:hover:not(:disabled) { color: var(--accent-2-text); background: var(--accent-2-soft); border-color: color-mix(in srgb, var(--accent-2) 55%, transparent); }
#actBook:hover:not(:disabled) .bk-pen { transform: translate(-1px, -1px); }
#actBook[aria-expanded="true"] { color: var(--accent-2-text); background: var(--accent-2-soft); border-color: var(--accent-2); }
#actBook.busy .bk-pen { animation: tbk-write 1.1s ease-in-out infinite; }
@keyframes tbk-write { 50% { transform: translate(-1.4px, .6px); } }
@media (prefers-reduced-motion: reduce) {
  #actBook svg.i path { transition: none; }
  #actBook:hover:not(:disabled) .bk-pen, #actBook.busy .bk-pen { transform: none; animation: none; }
}
.tbk-card { position: fixed; z-index: 60; width: 300px; max-width: calc(100vw - 16px); background: var(--panel); color: var(--text); border: 1px solid var(--line-strong); border-top: 2px solid var(--accent-2); border-radius: var(--r); box-shadow: 0 10px 30px rgba(0,0,0,.18); padding: 12px 14px; display: grid; gap: 10px; font-size: 13px; }
.tbk-card .tbk-head { display: flex; align-items: center; gap: 8px; font-weight: 600; }
.tbk-card .tbk-head svg.i { color: var(--accent-2-text); }
.tbk-card .tbk-row { display: grid; gap: 5px; }
.tbk-card .tbk-label { font: 500 11px var(--mono); color: var(--muted); letter-spacing: .04em; }
.tbk-card .seg { width: 100%; }
.tbk-card .seg button { flex: 1; padding: 5px 6px; }
.tbk-card .tbk-books { display: flex; flex-wrap: wrap; gap: 6px; max-height: 128px; overflow: auto; }
.tbk-card .tbk-books button { font-size: 12.5px; padding: 4px 9px; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; border: 1px solid var(--line-strong); border-radius: var(--r); background: var(--surface); color: var(--muted); }
.tbk-card .tbk-books button:hover:not([aria-pressed="true"]) { color: var(--text); background: var(--surface-hover); }
.tbk-card .tbk-books button[aria-pressed="true"] { color: var(--text); border-color: var(--accent-text); background: var(--accent-soft); font-weight: 600; }
.tbk-card .tbk-note { color: var(--muted); font-size: 12px; }
.tbk-card .tbk-foot { display: flex; align-items: center; gap: 8px; justify-content: flex-end; }
.tbk-card .tbk-bar { flex: 1; height: 3px; border-radius: 2px; background: var(--accent-2-soft); overflow: hidden; }
.tbk-card .tbk-bar::before { content: ""; display: block; width: 35%; height: 100%; background: var(--accent-2); animation: tbk-run 1.4s ease-in-out infinite; }
@keyframes tbk-run { from { transform: translateX(-100%); } to { transform: translateX(290%); } }
@media (prefers-reduced-motion: reduce) { .tbk-card .tbk-bar::before { animation: none; width: 100%; opacity: .5; } }
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
    card = h("div", { class: "tbk-card", role: "dialog", "aria-label": "寫 review" });
    document.body.append(card);
    btn.setAttribute("aria-expanded", "true");
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("keydown", esc, true);
    addEventListener("resize", place);
    draw();
  }

  /** A row of toggle buttons; `items` = [[value, label, title?]]. */
  function choices(cls, items, key, def) {
    const cur = pref.get(key, def);
    const s = h("div", { class: cls, role: "radiogroup" });
    for (const [v, label, title] of items) {
      s.append(h("button", { type: "button", title, "aria-pressed": String(v === cur), onclick: (e) => {
        pref.set(key, v);
        s.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(b === e.currentTarget)));
      } }, label));
    }
    return s;
  }

  /** PaperComposer answers: its port from the desktop apps' file, or the dev server's default. */
  async function alive() {
    await LoreLinks.refresh();
    try { return (await fetch(`${pc()}/api/info`, { cache: "no-store" })).ok; } catch { return false; }
  }

  /** Running already, else opened (lore-pc://, which starts the app) and waited for. */
  async function reach(onWait) {
    if (await alive()) return;
    onWait?.();
    await LoreLinks.open("pc");
    const end = Date.now() + WAIT_MS;
    while (Date.now() < end) {
      await new Promise((ok) => setTimeout(ok, 600));
      if (await alive()) return;
    }
    throw Object.assign(new Error("PaperComposer 沒有開啟"), { open: true });
  }

  /** PaperComposer's books: GET /api/books (a list or { books }), else /api/info's books. */
  async function books() {
    const list = (v) => (Array.isArray(v) ? v : Array.isArray(v?.books) ? v.books : null);
    try {
      const r = await fetch(`${pc()}/api/books`, { cache: "no-store" });
      if (r.ok) { const b = list(await r.json()); if (b) return b; }
    } catch {}
    const r = await fetch(`${pc()}/api/info`, { cache: "no-store" });
    if (!r.ok) throw new Error(String(r.status));
    return list(await r.json()) ?? [];
  }

  async function draw() {
    if (!card) return;
    card.textContent = "";
    card.append(h("div", { class: "tbk-head" }, icon("bookWrite"), "寫 review"));
    if (job) {
      card.append(h("div", { class: "tbk-foot" }, h("div", { class: "tbk-bar" }), h("span", { class: "tbk-note" }, job.note ?? "送出中…")));
      place();
      return;
    }
    const row = h("div", { class: "tbk-row" }, h("span", { class: "tbk-label" }, "書"));
    const list = h("div", { class: "tbk-books" }, h("span", { class: "tbk-note" }, "…"));
    row.append(list);
    const go = h("button", { type: "button", class: "btn primary small", disabled: true, onclick: () => create("review") }, "先看大綱");
    const direct = h("button", { type: "button", class: "btn small", disabled: true, onclick: () => create("direct") }, "直接產生");
    card.append(
      row,
      h("div", { class: "tbk-row" }, h("span", { class: "tbk-label" }, "篇幅"), choices("seg", LEN, "length", "standard")),
      h("div", { class: "tbk-row" }, h("span", { class: "tbk-label" }, "語言"), choices("seg", LANG, "lang", "en-zh")),
      h("div", { class: "tbk-foot" }, direct, go),
    );
    place();
    let have;
    try {
      await reach(() => { list.textContent = ""; list.append(h("span", { class: "tbk-note" }, "正在開啟 PaperComposer…")); place(); });
      have = await books();
    } catch {
      if (!card) return;
      list.textContent = "";
      list.append(h("span", { class: "tbk-note" }, "PaperComposer 沒有開啟"));
      direct.remove();
      go.replaceWith(
        h("button", { type: "button", class: "btn small", onclick: () => LoreLinks.open("pc") }, "開啟 PaperComposer"),
        h("button", { type: "button", class: "btn small primary", onclick: draw }, "重試"),
      );
      place();
      return;
    }
    if (!card) return;
    // the book that already has this paper, else the last choice when it still exists, else 新書
    const src = await opts.source({ check: true }).catch(() => null);
    const holder = have.find((b) => src?.sha && (b.papers ?? []).includes(src.sha))?.id;
    const want = holder ?? (have.some((b) => b.id === pref.get("book")) ? pref.get("book") : "new");
    pref.set("book", want);
    list.replaceWith(choices("tbk-books", [["new", "新書"], ...have.map((b) => [b.id, `加進〈${b.title || b.id}〉`, b.path])], "book", want));
    go.disabled = direct.disabled = false;
    place();
    go.focus();
  }

  /** What LoreReader knows about the author's reading of this paper; never blocks the request. */
  async function extras() {
    try {
      const x = await opts.extras?.();
      if (!x) return undefined;
      const out = {};
      if (x.terms?.length) out.terms = x.terms;
      if (x.stuck?.length) out.stuck = x.stuck;
      if (x.questions?.length) out.questions = x.questions;
      return Object.keys(out).length ? out : undefined;
    } catch { return undefined; }
  }

  async function create(mode = "review") {
    if (job) return;
    job = {};
    btn.classList.add("busy");
    draw();
    try {
      const source = await opts.source({});
      if (!source) throw new Error("先開一篇論文");
      const body = { source, book: pref.get("book", "new"), length: pref.get("length", "standard"), lang: pref.get("lang", "en-zh"), mode, extras: await extras() };
      await reach(() => { job.note = "正在開啟 PaperComposer…"; draw(); });
      let r;
      try {
        r = await fetch(`${pc()}/api/book`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
      } catch { throw Object.assign(new Error("PaperComposer 沒有開啟"), { open: true }); }
      const v = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(v.error ?? `PaperComposer：${r.status}`);
      if (!v.job) throw new Error("PaperComposer 沒有回傳工作");
      close();
      LoreLinks.open("pc", { bookjob: v.job });
      LoreShell.toast("在 PaperComposer 進行中", { action: { label: "前往", run: () => LoreLinks.open("pc", { bookjob: v.job }) }, ms: 8000 });
    } catch (e) {
      const action = e.open ? { label: "開啟 PaperComposer", run: () => LoreLinks.open("pc") } : null;
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
    if (document.getElementById("actBook")) return true;
    if (!document.getElementById("makebookCss")) document.head.append(h("style", { id: "makebookCss" }, CSS));
    btn = h("button", { type: "button", class: "ico", id: "actBook", "aria-label": "寫 review", title: "寫 review", "aria-haspopup": "dialog", "aria-expanded": "false", onclick: toggle });
    btn.append(icon("bookWrite"));
    anchor.before(btn);
    return true;
  }

  /** Put the button before `before` (now, or once the shell has built the top bar). */
  function mount(o) {
    opts = o;
    pref = store(o.key ?? "lore.tobook");
    if (!attach()) {
      const mo = new MutationObserver(() => { if (attach()) mo.disconnect(); });
      mo.observe(document.documentElement, { childList: true, subtree: true });
    }
  }

  return { mount, open: () => toggle(), create };
})();
