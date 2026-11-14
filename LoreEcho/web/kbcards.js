// The author's KnowledgeBase cards in the conversation (server: kbctx.rs).
// SSE `search` → the character shows searching; `cards` → back to thinking, and under the line
// a compact 「引用：卡片A、卡片B」 row (each opens the card in KnowledgeBase), 記得／忘了 after a
// quiz answer, or a one-line notice. What the page sends back with the next message (`kb`):
// the card being quizzed on and the cards of the last reply.
(() => {
  const K = { quiz: null, cited: [], row: null };
  const LABEL = { learning: "學習中", known: "已懂", stale: "久未複習", "": "尚無紀錄" };

  function row() {
    if (K.row?.isConnected) return K.row;
    const line = document.getElementById("line");
    if (!line) return null;
    K.row = document.createElement("div");
    K.row.className = "cites";
    K.row.id = "cites";
    K.row.hidden = true;
    line.after(K.row);
    return K.row;
  }

  function el(tag, attrs = {}, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (k.startsWith("on")) e.addEventListener(k.slice(2), v); else e.setAttribute(k, v);
    }
    e.append(...kids);
    return e;
  }

  function show(...parts) {
    const r = row();
    if (!r) return;
    r.replaceChildren(...parts);
    r.hidden = !parts.length;
  }

  async function grade(id, outcome, box) {
    box.replaceChildren(el("span", { class: "note" }, "…"));
    let msg;
    try {
      const r = await fetch("/api/kb/recall", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ id, outcome }) });
      const d = await r.json().catch(() => ({}));
      msg = d.ok ? `已記下：${outcome === "remembered" ? "記得" : "忘了"}` : (d.error || `${r.status}`);
    } catch (e) { msg = e.message; }
    box.replaceChildren(el("span", { class: "note" }, msg));
  }

  function render(d) {
    const parts = [];
    const cards = d.cards || [];
    if (cards.length) {
      parts.push(el("span", { class: "k" }, "引用："));
      cards.forEach((c, i) => {
        if (i) parts.push("、");
        parts.push(el("button", { type: "button", class: "cite", "data-state": c.state || "", title: `${c.title}（${LABEL[c.state || ""] || ""}）`, onclick: () => window.LoreLinks?.open("kb", { node: c.id }) }, c.title));
      });
    }
    if (d.grade) {
      const box = el("span", { class: "grade" });
      box.append(
        el("button", { type: "button", class: "btn", onclick: () => grade(d.grade, "remembered", box) }, "記得"),
        el("button", { type: "button", class: "btn", onclick: () => grade(d.grade, "forgot", box) }, "忘了"),
      );
      parts.push(box);
    }
    if (d.recorded?.length) parts.push(el("span", { class: "note" }, "已記下：記得"));
    if (d.notice) parts.push(el("span", { class: "note" }, d.notice));
    show(...parts);
  }

  // the author's files (server: shelfctx.rs, while LoreShelf runs): 「檔案：a、b」 under the line;
  // a click turns LoreShelf's globe to that file
  let fileRow = null;
  function files(d) {
    const list = d?.files || [];
    if (!fileRow?.isConnected) {
      const anchor = row() || document.getElementById("line");
      if (!anchor) return;
      fileRow = el("div", { class: "cites files", id: "shelfFiles" });
      anchor.after(fileRow);
    }
    const go = (f) => fetch("/api/shelf/focus", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ id: f.id }) }).catch(() => {});
    const parts = list.length ? [el("span", { class: "k" }, "檔案：")] : [];
    list.forEach((f, i) => {
      if (i) parts.push("、");
      parts.push(el("button", { type: "button", class: "cite", title: f.date ? `${f.name}（${f.date}）` : f.name, onclick: () => go(f) }, f.name));
    });
    fileRow.replaceChildren(...parts);
    fileRow.hidden = !parts.length;
  }

  window.LoreEchoKb = {
    files,
    /** The `kb` part of the next chat body (a quiz is answered once); clears the row. */
    ask() {
      const out = { quiz: K.quiz, cited: K.cited };
      K.quiz = null;
      show();
      files(null);
      return out;
    },
    /** `search` / `cards` from the chat stream. */
    event(ev, d) {
      if (ev === "search") { window.LoreEcho?.setState("searching"); return; }
      if (ev !== "cards") return;
      K.quiz = d.quiz || null;
      K.cited = (d.cards || []).map((c) => c.id);
      K.citedCards = (d.cards || []).map((c) => ({ id: c.id, title: c.title }));
      render(d);
      if (d.intent === "fix") window.LoreEchoActions?.fix(K.citedCards);
      window.LoreEcho?.setState("thinking");
    },
    /** The cards the last reply used, [{id, title}] (修正知識卡 from the 功能列表). */
    cited: () => K.citedCards ?? [],
    /** 新對話 / collapse. */
    clear() { K.quiz = null; K.cited = []; show(); files(null); },
  };
})();
