// 選取的文字 (selection mode; this file is selected.js because /selection.js is the shared LoreSelection): another Lore app handed over the text the author selected
// (server: selection.rs, GET /api/selection polled). The dialogue box shows it as a quotation
// with its scope and the actions for its kind: a term → 加入知識庫 · 解釋 · 翻譯, a passage →
// 改寫 · 翻譯 · 問他. 改寫 and 翻譯 are the asking app's own 語句潤飾 (LorePolish drawn inline,
// its routes proxied under /api/selection; 翻譯 = a fixed instruction sent at once), the result
// goes back to the app with 套用 and lands on the selection there. 解釋 and 問他 are chat turns
// that carry the selection (send() asks take()). 加入知識庫 is the app's own add-card route.
// Esc closes the rewrite, then the selection; 新對話, collapse or a new selection end it.
"use strict";

window.LoreEchoSel = (() => {
  const X = { req: null, panel: null, slot: null, dirs: null, watch: null, armed: null, busy: false, ended: null };
  const TERM = [["card", "cards", "加入知識庫"], ["explain", "spark", "解釋"], ["translate", "languages", "翻譯"]];
  const PASSAGE = [["rewrite", "polish", "改寫"], ["translate", "languages", "翻譯"], ["ask", "chat", "問他"]];
  // 翻譯: the direction, the fixed instruction, the language of the result
  const DIRS = [
    ["中 → 英", "翻譯成英文，保持原意", "en"],
    ["英 → 中", "翻譯成繁體中文，保持原意", "zh-TW"],
    ["中 → 日", "翻譯成日文，保持原意", "ja"],
    ["日 → 中", "翻譯成繁體中文，保持原意", "zh-TW"],
  ];
  const box = () => $("box");

  async function poll() {
    try {
      const v = await api("GET", "/api/selection");
      // (an ended request may still be on the server for a moment: not a new one)
      if (v.open && v.request !== X.req?.request && v.request !== X.ended) begin(v);
      else if (!v.open && X.req) finish();
    } catch { /* the server restarts */ }
    setTimeout(poll, 1200);
  }

  function begin(r) {
    finish();
    window.LoreEchoActions?.close();
    if (!$("choices").hidden) toggleChoices(false);
    X.req = r;
    if (!S.expanded) show(true);
    mount();
    $("text").focus();
  }

  function mount() {
    const r = X.req;
    const where = [r.scope, r.source].find((x) => x && x.trim()) ?? "";
    const acts = h("div", { class: "sel-acts", role: "group", "aria-label": "對選取的文字" },
      (r.kind === "term" ? TERM : PASSAGE).map(([id, ic, label]) => {
        const b = h("button", { type: "button", class: "btn small", "data-act": id, onclick: () => act(id) }, h("span", { "data-icon": ic }), label);
        if (id === "translate") { b.append(h("span", { class: "caret", "data-icon": "chevron" })); b.setAttribute("aria-expanded", "false"); }
        if (id === "ask") b.setAttribute("aria-pressed", "false");
        return b;
      }));
    X.dirs = h("div", { class: "sel-dirs", hidden: true }, h("div", { class: "seg", role: "group", "aria-label": "翻譯方向" },
      DIRS.map(([label, instruction, language]) => h("button", { type: "button", onclick: () => rewrite({ instruction, language }) }, label))));
    X.slot = h("div", { class: "sel-work" });
    X.panel = h("div", { class: "sel", id: "sel" },
      where ? h("div", { class: "sel-scope" }, where) : null,
      h("blockquote", { class: `sel-quote${r.kind === "term" ? " term" : ""}` }, r.text),
      acts, X.dirs, X.slot);
    $("line").before(X.panel);
    paintIcons(X.panel);
    box().classList.add("sel-on");
    $("text").placeholder = "說點什麼…";
    // the inline rewrite closes itself (套用, its ✕): back to the actions
    X.watch = new MutationObserver(() => { if (!X.slot?.firstChild) box().classList.remove("sel-polish"); });
    X.watch.observe(X.slot, { childList: true });
  }

  const button = (id) => X.panel?.querySelector(`[data-act="${id}"]`);
  function dirs(on) {
    if (!X.dirs) return;
    X.dirs.hidden = !on;
    button("translate")?.setAttribute("aria-expanded", String(on));
    if (on) X.dirs.scrollIntoView({ block: "nearest" });
  }
  function arm(on) {
    X.armed = on ? "ask" : null;
    button("ask")?.setAttribute("aria-pressed", String(!!on));
    $("text").placeholder = on ? "問這段…" : "說點什麼…";
  }

  function act(id) {
    if (!X.req || X.busy) return;
    if (id !== "translate") dirs(false);
    if (id !== "ask") arm(false);
    if (id === "translate") dirs(X.dirs.hidden);
    else if (id === "rewrite") rewrite({});
    else if (id === "ask") { arm(true); $("text").focus(); }
    else if (id === "explain") { X.armed = "explain"; $("text").value = `解釋「${X.req.text.trim()}」`; send(); }
    else if (id === "card") card();
  }

  /** 改寫 (he says how) or 翻譯 (the instruction runs at once): the app's 語句潤飾, drawn in the box. */
  function rewrite({ instruction, language }) {
    const r = X.req;
    dirs(false); arm(false);
    box().classList.add("sel-polish");
    window.LorePolish.open({
      into: X.slot, api: "/api/selection", doc: r.doc, text: r.text, prefix: r.prefix, suffix: r.suffix, context: r.context,
      keep: r.keep ?? [], scope: r.scope, language: language ?? r.language ?? undefined, instruction,
      apply: async (after) => {
        try {
          const v = await api("POST", "/api/selection/apply", { after });   // no `polish` id: LorePolish marks it applied itself
          done(v.message);
          return true;
        } catch (e) { return e.message; }   // the app's refusal, as it is
      },
    });
    if (!X.slot.firstChild) box().classList.remove("sel-polish");   // nothing was opened (no document)
  }

  /** The result is in the app: this selection is over. The page leaves it once LorePolish has
   *  closed its card; the server is told a moment later, after LorePolish's own
   *  /polish/applied went through the open request. */
  function done(message) {
    if (message) LoreShell.toast(message);
    setTimeout(() => end(800), 0);
  }

  async function card() {
    const r = X.req;
    const title = window.LoreAddCard?.clean(r.text) ?? r.text.trim();
    if (!title) return;
    X.busy = true;
    for (const b of X.panel.querySelectorAll(".sel-acts .btn")) b.disabled = true;
    try {
      // the same answers as LoreAddCard.add (commons/ui/addcard.js), through the asking app
      await whileState("noting", async () => {
        const res = await fetch("/api/selection/lore/card", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ title, source: r.doc || r.source || "" }) }).catch(() => null);
        const v = res ? await res.json().catch(() => ({})) : { error: "沒有回應" };
        const open = (id) => ({ label: "開啟", run: () => LoreLinks.open("kb", { node: id }) });
        if (res?.status === 409 && v.id) LoreShell.toast(v.error, { action: open(v.id), ms: 8000 });
        else if (!res?.ok) LoreShell.toast(v.error ?? `加入失敗（${res?.status ?? "—"}）`, { err: true });
        else LoreShell.toast(`已加入知識庫：${v.node.title}`, { action: open(v.node.id), ms: 6000 });
      });
    } finally {
      X.busy = false;
      if (X.req === r) for (const b of X.panel.querySelectorAll(".sel-acts .btn")) b.disabled = false;
    }
  }

  /** send() asks: the selection this chat turn is about (解釋, or his question after 問他), once. */
  function take() {
    if (!X.req) return null;
    box().classList.add("sel-talk");   // a turn while a selection is shown: its reply shows under the quotation
    if (!X.armed) return null;         // a message of his own, not about the selection
    const r = X.req;
    arm(false);
    return { text: r.text, scope: r.scope ?? "", context: [r.prefix, r.suffix, r.context].filter((x) => x && x.trim()).join("\n") };
  }

  async function end(wait = 0) {
    if (!X.req) return;
    X.ended = X.req.request;
    finish();
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    try { await api("POST", "/api/selection/end"); } catch { /* the app stops waiting after a while */ }
  }

  function finish() {
    if (!X.req) return;
    X.req = null; X.armed = null; X.busy = false;
    X.watch?.disconnect(); X.watch = null;
    if (window.LorePolish?.isOpen() && X.slot?.firstChild) window.LorePolish.close();
    X.panel?.remove(); X.panel = null; X.slot = null; X.dirs = null;
    box().classList.remove("sel-on", "sel-polish", "sel-talk");
    $("text").placeholder = "說點什麼…";
  }

  /** Esc: the rewrite or the direction row first, then the selection. */
  function escape() {
    if (!X.req) return false;
    if (X.slot?.firstChild) { window.LorePolish.close(); $("text").focus(); }
    else if (!X.dirs.hidden) dirs(false);
    else if (X.armed) arm(false);
    else end();
    return true;
  }

  let started = false;
  return {
    /** app.js, once boot() has opened or collapsed the window. */
    start: () => { if (!started) { started = true; poll(); } },
    take, escape, end: () => end(),
    active: () => !!X.req,
  };
})();
