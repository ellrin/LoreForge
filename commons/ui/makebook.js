// 寫 review: the open paper → L-Paper's 寫書 start panel with this paper already in it
// (docs/book.md 統一入口). Books begin in one place, that panel; this button only fills in what
// the reading knows (the paper, the red terms, the paragraphs without 懂了, the questions) and
// sends the author there, where everything can still be added to or removed. Served as
// `/makebook.js` by `lore_shell::FILES`; loaded after shell.js. An app mounts it once:
//
//   LoreMakeBook.mount({
//     before: "#actAsk",                         // the top-bar button it sits before
//     source: async () => ({ kind: "lr", sha, title }), // the open paper; null = none
//     extras: () => ({ terms: [{ term, n }], stuck: [paragraph id], questions: [text] }),  // optional
//   });
//
// L-Paper not running: it is opened and waited for (POST /api/book/setup, then
// LoreLinks.open("pc", { booksetup })).
"use strict";

window.LoreMakeBook = (() => {
  const WAIT_MS = 30000;
  const pc = () => `http://127.0.0.1:${LoreLinks.ports.pc}`;
  let opts = null, btn = null, busy = false;

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

  const CSS = `
#actBook { position: relative; color: var(--accent-2-text); }
#actBook svg.i path { transition: transform .22s cubic-bezier(.3,.7,.3,1.3); }
#actBook:hover:not(:disabled) { color: var(--accent-2-text); background: var(--accent-2-soft); border-color: color-mix(in srgb, var(--accent-2) 55%, transparent); }
#actBook:hover:not(:disabled) .bk-pen { transform: translate(-1px, -1px); }
#actBook.busy .bk-pen { animation: tbk-write 1.1s ease-in-out infinite; }
@keyframes tbk-write { 50% { transform: translate(-1.4px, .6px); } }
@media (prefers-reduced-motion: reduce) {
  #actBook svg.i path { transition: none; }
  #actBook:hover:not(:disabled) .bk-pen, #actBook.busy .bk-pen { transform: none; animation: none; }
}
`;

  /** L-Paper answers: its port from the desktop apps' file, or the dev server's default. */
  async function alive() {
    await LoreLinks.refresh();
    try { return (await fetch(`${pc()}/api/info`, { cache: "no-store" })).ok; } catch { return false; }
  }

  /** Running already, else started behind this window (`open -g -a L-Paper`) and waited for;
   *  the start panel link then brings L-Paper forward once, already on the panel. */
  async function reach(onWait) {
    if (await alive()) return;
    onWait?.();
    await LoreLinks.open("pc", {}, { background: true });
    const end = Date.now() + WAIT_MS;
    while (Date.now() < end) {
      await new Promise((ok) => setTimeout(ok, 600));
      if (await alive()) return;
    }
    throw Object.assign(new Error("L-Paper 沒有開啟"), { open: true });
  }

  /** What L-Reader knows about the author's reading of this paper; never blocks the request. */
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

  /** The paper and the reading go to L-Paper's start panel; the author goes on there. */
  async function go() {
    if (busy) return;
    const source = await opts.source({}).catch(() => null);
    if (!source) { LoreShell.toast("先開一篇論文"); return; }
    busy = true;
    btn.classList.add("busy");
    try {
      await reach(() => LoreShell.toast("正在開啟 L-Paper…"));
      let r;
      try {
        r = await fetch(`${pc()}/api/book/setup`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ sources: [source], extras: await extras() }) });
      } catch { throw Object.assign(new Error("L-Paper 沒有開啟"), { open: true }); }
      const v = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(v.error ?? `L-Paper：${r.status}`);
      if (!v.setup) throw new Error("L-Paper 沒有回應");
      LoreLinks.open("pc", { booksetup: v.setup });
    } catch (e) {
      const action = e.open ? { label: "開啟 L-Paper", run: () => LoreLinks.open("pc") } : null;
      LoreShell.toast(e.message, { err: true, action, ms: 9000 });
    } finally {
      busy = false;
      btn.classList.remove("busy");
    }
  }

  function attach() {
    const anchor = document.querySelector(opts.before);
    if (!anchor) return false;
    if (document.getElementById("actBook")) return true;
    if (!document.getElementById("makebookCss")) document.head.append(h("style", { id: "makebookCss" }, CSS));
    btn = h("button", { type: "button", class: "ico", id: "actBook", "aria-label": "寫 review", title: "寫 review", onclick: go });
    btn.append(icon("bookWrite"));
    anchor.before(btn);
    return true;
  }

  /** Put the button before `before` (now, or once the shell has built the top bar). */
  function mount(o) {
    opts = o;
    if (!attach()) {
      const mo = new MutationObserver(() => { if (attach()) mo.disconnect(); });
      mo.observe(document.documentElement, { childList: true, subtree: true });
    }
  }

  return { mount, open: go };
})();
