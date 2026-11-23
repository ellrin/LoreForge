// Selected text through L-Echo: the page hands the author's selection to L-Echo, L-Echo sends a
// rewrite back, the page applies it as one undo step and says whether it worked.
// Served as `/selection.js` by `lore_shell::FILES`; loaded after shell.js. The server side is
// `lore_shell::selection` (POST /api/lore/selection, GET /api/lore/selection/<request>,
// POST /api/lore/selection/<request>/done; L-Echo sends each result to the app's
// POST /api/lore/selected and waits until the page has applied it).
//
//   LoreSelection.watch({
//     gather: () => ({                      // the current selection, or null when nothing usable
//       doc, source, text, prefix, suffix, context, scope, keep: [], language,   // language optional
//     }),
//     apply: async (after, item) => true,   // replace the selection (one undo step): true, or why not (string)
//   });
//   LoreSelection.send();                   // an explicit menu item: brings L-Echo up (promise)
// A settled selection (about 400 ms, non-empty, changed) goes to an L-Echo that is already open;
// when L-Echo is not open nothing is said. One open request per page: a new selection ends the old one.
"use strict";

window.LoreSelection = (() => {
  let opts = null;          // { gather, apply }
  let cur = null;           // { request, handled: Set, timer, since, busy }
  let last = "";            // the last payload sent, as JSON
  let debounce = null;
  let token = 0;            // the newest post wins
  const toast = (msg, err) => (window.LoreShell?.toast ? LoreShell.toast(msg, { err }) : console.warn(msg));

  function stop() {
    if (cur) clearTimeout(cur.timer);
    cur = null;
  }

  function gather() {
    try {
      const p = opts?.gather?.();
      return p && typeof p.text === "string" && p.text.trim() ? p : null;
    } catch (e) {
      console.error(e);
      return null;
    }
  }

  async function done(c, seq, body) {
    try {
      await fetch(`/api/lore/selection/${encodeURIComponent(c.request)}/done`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ seq, ...body }) });
    } catch { /* L-Echo hears a timeout */ }
  }

  async function poll(c) {
    if (cur !== c) return;
    let wait = 1000;
    try {
      const r = await fetch(`/api/lore/selection/${encodeURIComponent(c.request)}`, { cache: "no-store" });
      if (!r.ok) { if (cur === c) stop(); return; }
      const v = await r.json();
      for (const item of v.pending || []) {
        if (cur !== c) return;   // superseded: its other items are not applied
        if (c.handled.has(item.seq)) continue;
        c.handled.add(item.seq);
        let res;
        c.busy = true;   // the rewrite changes the selection: not a new one
        try { res = await opts.apply(item.after, item); } catch (e) { res = String(e?.message || e); }
        await done(c, item.seq, res === true ? { ok: true } : { ok: false, error: typeof res === "string" && res ? res : "沒有套用" });
        c.busy = false;
        const now = gather();
        last = now ? JSON.stringify(now) : last;   // what the rewrite left selected is the baseline
        wait = 300;   // more may follow soon
      }
      if (!v.open) { if (cur === c) stop(); return; }
    } catch { /* the server restarts: try again */ }
    if (cur !== c) return;
    if (Date.now() - c.since > 30 * 60 * 1000) { stop(); return; }
    c.timer = setTimeout(() => poll(c), wait);
  }

  async function post(p, launch) {
    const my = ++token;
    last = JSON.stringify(p);
    stop();
    let r;
    try {
      r = await fetch("/api/lore/selection", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ ...p, launch }) });
    } catch (e) {
      if (my === token) last = "";   // the server restarts: try again next time
      throw e;
    }
    const v = await r.json().catch(() => ({}));
    if (!r.ok) {
      if (my === token) last = "";   // try again next time
      const e = new Error(v.error || `${r.status}`);
      e.status = r.status;
      throw e;
    }
    if (my !== token) return v.request;   // a newer selection already took over
    cur = { request: v.request, handled: new Set(), since: Date.now(), timer: null };
    poll(cur);
    return v.request;
  }

  function settled() {
    clearTimeout(debounce);
    debounce = setTimeout(() => {
      if (cur?.busy) return;   // a rewrite is being applied
      const p = gather();
      if (!p || JSON.stringify(p) === last) return;
      post(p, false).catch((e) => { if (e.status !== 503) console.warn(e.message); });   // 503: L-Echo is not open
    }, 400);
  }

  /** Follow the page's selection; `gather` and `apply` come from the app. */
  function watch(o) {
    const first = !opts;
    opts = o;
    if (first) {
      document.addEventListener("selectionchange", settled);
      document.addEventListener("mouseup", settled);
      document.addEventListener("keyup", settled);
    }
    return api;
  }

  /** Hand the current selection to L-Echo, bringing it up (an explicit menu item). */
  async function send() {
    const p = gather();
    if (!p) return null;
    clearTimeout(debounce);
    try {
      return await post(p, true);
    } catch (e) {
      toast(String(e.message || e), true);
      return null;
    }
  }

  const api = { watch, send, active: () => !!cur };
  return api;
})();
