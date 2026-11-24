// 選取的文字 (selection mode; this file is selected.js because /selection.js is the shared LoreSelection): another Lore app handed over the text the author selected
// (server: selection.rs, GET /api/selection polled). The dialogue box shows it as a quotation
// with its scope and the actions for its kind: a term → 加入知識庫 · 解釋 · 翻譯, a passage →
// 改寫 · 翻譯 · 問他. 改寫 and 翻譯 are the asking app's own 語句潤飾 (LorePolish drawn inline,
// its routes proxied under /api/selection; 翻譯 = a fixed instruction sent at once), the result
// goes back to the app with 套用 and lands on the selection there. 解釋 and 問他 are chat turns
// that carry the selection (send() asks take()). 加入知識庫 is the app's own add-card route.
// A read-only document (`readonly`, a PDF): no 改寫, and 翻譯 is a chat turn; nothing goes back.
// Its 加入知識庫 sends the paper's id (`doc`), title (`source`), page (`scope`) and page text: L-Reader's
// /api/lore/card is its own 一鍵加卡.
// Esc closes the rewrite, then the selection; 新對話, collapse or a new selection end it. A 套用 the
// app refuses (409) or a selection already over (410) ends it too; 502 / 504 keep the rewrite.
"use strict";

window.LoreEchoSel = (() => {
  const X = { req: null, panel: null, slot: null, dirs: null, watch: null, armed: null, busy: false, ended: null };
  const TERM = [["card", "cards", "加入知識庫"], ["explain", "spark", "解釋"], ["translate", "languages", "翻譯"]];
  const PASSAGE = [["rewrite", "polish", "改寫"], ["translate", "languages", "翻譯"], ["ask", "chat", "問他"]];
  // 翻譯: the direction, the fixed instruction, the language of the result, the chat turn (read-only)
  const DIRS = [
    ["中 → 英", "翻譯成英文，保持原意", "en", "翻譯成英文"],
    ["英 → 中", "翻譯成繁體中文，保持原意", "zh-TW", "翻譯成中文"],
    ["中 → 日", "翻譯成日文，保持原意", "ja", "翻譯成日文"],
    ["日 → 中", "翻譯成繁體中文，保持原意", "zh-TW", "翻譯成中文"],
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
      (r.kind === "term" ? TERM : PASSAGE).filter(([id]) => !(r.readonly && id === "rewrite")).map(([id, ic, label]) => {
        const b = h("button", { type: "button", class: "btn small", "data-act": id, onclick: () => act(id) }, h("span", { "data-icon": ic }), label);
        if (id === "translate") { b.append(h("span", { class: "caret", "data-icon": "chevron" })); b.setAttribute("aria-expanded", "false"); }
        if (id === "ask") b.setAttribute("aria-pressed", "false");
        return b;
      }));
    X.dirs = h("div", { class: "sel-dirs", hidden: true }, h("div", { class: "seg", role: "group", "aria-label": "翻譯方向" },
      DIRS.map(([label, instruction, language, turn]) => h("button", { type: "button", onclick: () => (r.readonly ? chatTurn(turn) : rewrite({ instruction, language })) }, label))));
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
    X.watch = new MutationObserver(() => { if (!X.slot?.firstChild) box().classList.remove("sel-polish"); S.chip?.actual(""); });
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

  /** 翻譯 of a read-only selection: a chat turn that carries it; the reply is the translation. */
  function chatTurn(message) {
    dirs(false); arm(false);
    X.armed = "translate"; $("text").value = message; send();
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
        let res;
        try {
          // no `polish` id: LorePolish marks it applied itself
          res = await fetch("/api/selection/apply", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ after }), cache: "no-store" });
        } catch { return "L-Echo 沒有回應"; }
        const v = await res.json().catch(() => ({}));
        if (res.ok) { done(v.message); return true; }
        const why = v.error || `${res.status}`;
        // 409 the app refused (the text changed meanwhile), 410 the selection is over: it is stale,
        // said once, and the selection ends. Anything else (502 / 504: the app not reached) keeps
        // the rewrite so 套用 can be tried again.
        if (res.status === 409 || res.status === 410) { LoreShell.toast(why, { err: true, ms: 6000 }); end(); return ""; }
        return why;
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
        // a read-only document (L-Reader): the title as the source, and what its 一鍵加卡 needs
        const body = r.readonly ? { title, source: r.source || r.doc || "", doc: r.doc, scope: r.scope ?? "", context: r.context ?? "" } : { title, source: r.doc || r.source || "" };
        const res = await fetch("/api/selection/lore/card", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).catch(() => null);
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
    S.chip?.paint();                             // the chip is the chat's model again
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
    /** A rewrite is shown: the header's chip is the rewrite's model (app.js). */
    rewriting: () => !!X.slot?.firstChild,
  };
})();
