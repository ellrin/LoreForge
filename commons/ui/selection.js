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
//   LoreSelection.quiet(fn);                // the app's own undo / redo: what it leaves selected is not sent
// A settled selection (about 400 ms, non-empty, changed) goes to an L-Echo that is already open;
// when L-Echo is not open nothing is said. One open request per page: a new selection ends the old one.
// The same text again goes only when the author made it again (a press or key that changed the
// selection, then its release); a selection that stays on screen, an undo or a redo is never sent.
"use strict";

window.LoreSelection = (() => {
  let opts = null;          // { gather, apply }
  let cur = null;           // { request, handled: Set, timer, since, busy }
  let last = "";            // the last payload sent, as JSON
  let debounce = null;
  let token = 0;            // the newest post wins
  let down = false;         // a press or a key is held
  let moved = false;        // the selection changed while it was held
  let fresh = false;        // the author's own gesture ended on this selection: the same text may go again
  let holds = 0;            // quiet() calls running
  let hushUntil = 0;        // an undo / redo just ran: what it selects is the new baseline, not a selection
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
        adopt();   // what the rewrite left selected is the baseline
        wait = 300;   // more may follow soon
      }
      if (!v.open) { if (cur === c) { stop(); last = ""; } return; }   // ended in L-Echo: the same text may be handed over again
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

  /** What is selected now is where things stand: nothing to send. */
  function adopt() {
    const p = gather();
    if (p) last = JSON.stringify(p);
    fresh = false;
  }
  const hushed = () => holds > 0 || Date.now() < hushUntil;

  function settled() {
    clearTimeout(debounce);
    debounce = setTimeout(() => {
      if (cur?.busy) return;   // a rewrite is being applied
      if (hushed()) { adopt(); return; }
      const again = fresh;
      fresh = false;
      const p = gather();
      if (!p || (JSON.stringify(p) === last && !again)) return;
      post(p, false).catch((e) => { if (e.status !== 503) console.warn(e.message); });   // 503: L-Echo is not open
    }, 400);
  }

  // A gesture: pressed (mouse or key), the selection changed while held, released. Only that makes
  // the same text a new selection; a click or key that leaves the selection alone does not.
  const press = () => { if (!down) { down = true; moved = false; } };   // a held key repeats: one gesture
  const release = () => {
    if (down && moved) fresh = true;
    down = false; moved = false;
    settled();
  };
  const changed = () => { if (down) moved = true; settled(); };
  // the browser's own undo / redo in a text field (⌘Z, ⇧⌘Z): not a selection of his
  const history = (e) => {
    if (e.inputType !== "historyUndo" && e.inputType !== "historyRedo") return;
    hushUntil = Date.now() + 1000;
    moved = false; fresh = false;
  };

  /** The app's own undo / redo (or any change it makes itself): run `fn`; what it leaves selected
   *  is not handed to L-Echo. Returns what `fn` returns. */
  async function quiet(fn) {
    holds++;
    clearTimeout(debounce);
    try {
      return await fn?.();
    } finally {
      holds--;
      hushUntil = Date.now() + 1000;   // its selectionchange events arrive after it
      fresh = false;
      adopt();
    }
  }

  /** Follow the page's selection; `gather` and `apply` come from the app. */
  function watch(o) {
    const first = !opts;
    opts = o;
    if (first) {
      document.addEventListener("selectionchange", changed);
      document.addEventListener("mousedown", press, true);
      document.addEventListener("keydown", press, true);
      document.addEventListener("mouseup", release, true);
      document.addEventListener("keyup", release, true);
      document.addEventListener("beforeinput", history, true);
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

  const api = { watch, send, quiet, active: () => !!cur };
  return api;
})();
