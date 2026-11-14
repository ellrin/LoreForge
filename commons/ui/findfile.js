// 搜尋檔案 through LoreEcho: the one way a Lore app finds one of the author's files.
// Served as `/findfile.js` by `lore_shell::FILES`; loaded after shell.js. The server side is
// `lore_shell::findfile` (POST /api/lore/find, GET /api/lore/find/<request>; LoreEcho sends each
// chosen file to the app's POST /api/lore/picked, and the app's callback takes it).
//
//   const b = LoreFindFile.button({
//     purpose: "實驗紀錄",                  // what the file is for (LoreEcho says it)
//     exts: ["md", "csv", "pdf"],           // accepted extensions (optional)
//     context: { paper: name },             // handed to the app's callback (optional)
//     onAdded: (names) => reload(),         // files the app took, as they arrive
//   });
// The page may redraw: a new button for the same purpose and context picks up the open request.
"use strict";

window.LoreFindFile = (() => {
  const open = new Map();   // key → { request, seen, onAdded, timer, buttons }
  const keyOf = (o) => `${o.purpose}\u0000${JSON.stringify(o.context ?? null)}`;
  const toast = (msg, err) => (window.LoreShell?.toast ? LoreShell.toast(msg, { err }) : console.warn(msg));

  function paint(k) {
    const on = open.has(k);
    document.querySelectorAll("button.lore-findfile").forEach((b) => {
      if (b._key !== k) return;
      b.classList.toggle("on", on);
      b.setAttribute("aria-pressed", on ? "true" : "false");
      b.title = on ? "在 LoreEcho 選檔案" : "用 LoreEcho 找檔案";
    });
  }

  function stop(k) {
    const o = open.get(k);
    if (o) clearTimeout(o.timer);
    open.delete(k);
    paint(k);
  }

  async function poll(k) {
    const o = open.get(k);
    if (!o) return;
    try {
      const r = await fetch(`/api/lore/find/${encodeURIComponent(o.request)}`, { cache: "no-store" });
      if (!r.ok) { stop(k); return; }
      const v = await r.json();
      const fresh = (v.added || []).slice(o.seen);
      o.seen = (v.added || []).length;
      if (fresh.length) try { o.onAdded?.(fresh); } catch (e) { console.error(e); }
      if (!v.open) { stop(k); return; }
    } catch { /* the server restarts: try again */ }
    if (Date.now() - o.since > 30 * 60 * 1000) { stop(k); return; }
    o.timer = setTimeout(() => poll(k), 1000);
  }

  async function start(opts, k) {
    if (open.has(k)) return;   // already looking: LoreEcho has it
    open.set(k, { request: null, seen: 0, onAdded: opts.onAdded, since: Date.now() });
    paint(k);
    try {
      const r = await fetch("/api/lore/find", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ purpose: opts.purpose, exts: opts.exts || [], context: opts.context ?? null }) });
      const v = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(v.error || `${r.status}`);
      open.get(k).request = v.request;
      poll(k);
    } catch (e) {
      stop(k);
      toast(String(e.message || e), true);
    }
  }

  /** The 「搜尋檔案」 button. */
  function button(opts) {
    const k = keyOf(opts);
    const b = document.createElement("button");
    b.type = "button";
    b.className = `btn lore-findfile ${opts.className || "small"}`;
    b.textContent = opts.label || "搜尋檔案";
    b._key = k;
    if (open.has(k)) open.get(k).onAdded = opts.onAdded;   // a redraw: the newest page code hears it
    b.onclick = () => start(opts, k);
    queueMicrotask(() => paint(k));
    return b;
  }

  return { button, active: (opts) => open.has(keyOf(opts)) };
})();
