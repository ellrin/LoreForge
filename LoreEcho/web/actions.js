// 功能列表 (design §4, server: actions.rs): 整理成筆記, 做成知識卡, 記下靈感.
// Each opens a preview in place of the history (the header and the dialogue box stay); the
// author edits it and confirms, nothing is saved before. While the model drafts, the character
// shows `noting`. After a save: a one-line toast with 打開 (LoreLinks).
// Hooked from app.js: LoreEchoActions.run(tool) from the choice rows, .escape() from Esc,
// .close() from 新對話 / collapsing. Needs LoreShell, LoreLinks, window.LoreEcho.
(() => {
  const A = { panel: null, ctrl: null, dirty: false };

  function el(tag, attrs = {}, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else if (k === "class") e.className = v;
      else if (k === "value") e.value = v;
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null) e.append(c);
    return e;
  }

  async function post(path, body, signal) {
    const r = await fetch(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body), signal });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) { const e = new Error(data.error || `${r.status}`); e.status = r.status; e.body = data; throw e; }
    return data;
  }

  // ── the panel (in the card, in place of the history) ──
  function open(kind, ...kids) {
    close();
    const card = document.querySelector("#win .card");
    const hist = document.getElementById("hist");
    hist.hidden = true;
    A.panel = el("section", { class: `act act-${kind}`, "aria-label": { note: "整理成筆記", cards: "做成知識卡", idea: "記下靈感" }[kind] }, ...kids);
    card.append(A.panel);
    A.dirty = false;
    A.panel.addEventListener("input", () => { A.dirty = true; });
    return A.panel;
  }

  function close() {
    A.ctrl?.abort();
    A.ctrl = null;
    A.panel?.remove();
    A.panel = null;
    A.dirty = false;
    const hist = document.getElementById("hist");
    if (hist) hist.hidden = false;
  }

  function head(title) {
    return el("div", { class: "act-head" }, el("span", { class: "act-title" }, title), el("button", { type: "button", class: "ico small", "aria-label": "取消", title: "取消", onclick: close }, el("span", { "data-icon": "x" })));
  }

  function foot(...btns) { return el("div", { class: "act-foot" }, ...btns); }

  function busy(kind, title) {
    open(kind, head(title), el("div", { class: "act-wait" }, "整理中…"), foot(el("button", { type: "button", class: "btn", onclick: close }, "取消")));
    paint();
  }

  function paint() { if (A.panel) window.paintIcons?.(A.panel); }

  /** A failed draft: the message, and 「改用本機／改用 Claude」 when the server offers it. */
  function failed(kind, title, e, retry) {
    const alt = e.body?.alternative;
    open(kind, head(title), el("div", { class: "act-err" }, e.message),
      foot(el("button", { type: "button", class: "btn", onclick: close }, "取消"),
        alt?.engine ? el("button", { type: "button", class: "btn primary", onclick: () => retry(alt.engine) }, alt.label || "改用另一個") : null));
    paint();
  }

  /** The draft call, with the character `noting` meanwhile. */
  async function draft(path, engine) {
    const msgs = window.LoreEcho?.messages?.() ?? [];
    A.ctrl = new AbortController();
    const ctrl = A.ctrl;
    const run = () => post(path, { messages: msgs, character: window.LoreEcho?.character()?.id, engine }, ctrl.signal);
    const r = window.LoreEcho?.whileState ? await window.LoreEcho.whileState("noting", run) : await run();
    if (ctrl.signal.aborted) throw new DOMException("aborted", "AbortError");
    return r;
  }

  function hasTalk() {
    if ((window.LoreEcho?.messages?.() ?? []).some((m) => m.role === "user" && m.text.trim())) return true;
    LoreShell.toast("還沒有對話");
    return false;
  }

  function saved(msg, app, params) {
    LoreShell.toast(msg, { action: { label: "打開", run: () => window.LoreLinks?.open(app, params) } });
  }

  const tagsOf = (s) => s.split(/[,，、]/).map((t) => t.trim()).filter(Boolean);

  // ── KaTeX (vendor/katex, loaded the first time a preview needs it) ──
  let katexReady = null;
  function loadKatex() {
    if (window.katex) return Promise.resolve();
    katexReady ??= new Promise((ok) => {
      document.head.append(el("link", { rel: "stylesheet", href: "vendor/katex/katex.min.css" }));
      const s = el("script", { src: "vendor/katex/katex.min.js" });
      s.onload = ok; s.onerror = ok;
      document.head.append(s);
    });
    return katexReady;
  }

  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  function tex(src, display) {
    if (!window.katex) return `<code>${esc(src)}</code>`;
    try { return katex.renderToString(src, { displayMode: display, throwOnError: false, strict: "ignore", output: "html" }); }
    catch { return `<code>${esc(src)}</code>`; }
  }

  /** A note's LaTeX body as HTML: sections, lists, bold/italic, \ref as a tag, math through KaTeX. */
  function latexHtml(src) {
    const math = [];
    const keep = (html) => `\u0000${math.push(html) - 1}\u0000`;
    let s = src.replace(/%[^\n]*/g, (m, i) => (src[i - 1] === "\\" ? m : ""));   // comments
    s = s.replace(/\\begin\{(equation|align|gather|multline)\*?\}([\s\S]*?)\\end\{\1\*?\}/g, (_, env, body) => {
      const labels = [...body.matchAll(/\\label\{([^}]*)\}/g)].map((m) => m[1]);
      const inner = body.replace(/\\label\{[^}]*\}/g, "");
      return keep(`<div class="eq">${tex(env === "equation" ? inner : `\\begin{aligned}${inner}\\end{aligned}`, true)}${labels.map((l) => `<span class="lbl">${esc(l)}</span>`).join("")}</div>`);
    });
    s = s.replace(/\\\[([\s\S]*?)\\\]|\$\$([\s\S]*?)\$\$/g, (_, a, b) => keep(`<div class="eq">${tex(a ?? b, true)}</div>`));
    s = s.replace(/\\\(([\s\S]*?)\\\)|(?<!\\)\$([^$]+?)(?<!\\)\$/g, (_, a, b) => keep(tex(a ?? b, false)));
    s = esc(s);
    const arg = "\\{([^{}]*)\\}";
    s = s.replace(new RegExp(`\\\\(sub)?section\\*?${arg}`, "g"), (_, sub, t) => `\n\n<h${sub ? 5 : 4}>${t}</h${sub ? 5 : 4}>\n\n`)
      .replace(new RegExp(`\\\\textbf${arg}`, "g"), "<b>$1</b>")
      .replace(new RegExp(`\\\\(?:emph|textit)${arg}`, "g"), "<i>$1</i>")
      .replace(new RegExp(`\\\\texttt${arg}`, "g"), "<code>$1</code>")
      .replace(new RegExp(`\\\\(?:eq)?ref${arg}`, "g"), '<span class="ref">$1</span>')
      .replace(new RegExp(`\\\\label${arg}`, "g"), "")
      .replace(/\\(%|&amp;|#|_|\$)/g, "$1")
      .replace(/\\\\/g, "<br>");
    // lists
    s = s.replace(/\\begin\{(itemize|enumerate)\}([\s\S]*?)\\end\{\1\}/g, (_, kind, body) => {
      const items = body.split(/\\item\b/).slice(1).map((x) => `<li>${x.trim()}</li>`).join("");
      return `\n\n<${kind === "itemize" ? "ul" : "ol"}>${items}</${kind === "itemize" ? "ul" : "ol"}>\n\n`;
    });
    const out = s.split(/\n\s*\n/).map((p) => p.trim()).filter(Boolean).map((p) => (/^<(h\d|ul|ol)/.test(p) || /^\u0000\d+\u0000$/.test(p) ? p : `<p>${p}</p>`)).join("");
    return out.replace(/\u0000(\d+)\u0000/g, (_, i) => math[+i]);
  }

  // ── lore-notation findings ──
  function findingsList(list) {
    const box = el("ul", { class: "findings" });
    paintFindings(box, list);
    return box;
  }
  function paintFindings(box, list) {
    box.replaceChildren(...(list || []).map((f) => el("li", { class: `f-${f.severity}` }, f.message)));
    box.hidden = !list?.length;
  }
  const blocks = (list) => (list || []).some((f) => f.severity === "error");

  // ── 整理成筆記 ──
  async function note(engine) {
    if (!hasTalk()) return;
    busy("note", "整理成筆記");
    let d;
    try { d = await draft("/api/actions/note/draft", engine); }
    catch (e) { if (e.name !== "AbortError") failed("note", "整理成筆記", e, note); return; }
    await loadKatex();
    const title = el("input", { class: "set-input act-name", value: d.title, "aria-label": "標題", placeholder: "標題" });
    const src = el("textarea", { class: "act-src", spellcheck: "false", "aria-label": "LaTeX", hidden: true }, d.body);
    const view = el("div", { class: "act-view tex" });
    const tags = el("input", { class: "set-input", value: (d.tags || []).join(", "), "aria-label": "標籤", placeholder: "標籤" });
    let kb = (d.kb || []).map((id, i) => ({ id, title: d.kb_titles?.[i] || id }));
    const kbRow = el("div", { class: "act-links" });
    const paintKb = () => kbRow.replaceChildren(...kb.map((c) => el("span", { class: "link-chip" }, c.title, el("button", { type: "button", class: "x", "aria-label": `拿掉 ${c.title}`, onclick: () => { kb = kb.filter((x) => x !== c); A.dirty = true; paintKb(); } }, "×"))));
    paintKb();
    const finds = findingsList(d.findings);
    const save = el("button", { type: "button", class: "btn primary" }, "存到 PaperComposer");
    const mode = el("div", { class: "seg", role: "group" },
      el("button", { type: "button", "aria-pressed": "true", onclick: (e) => setMode(e, false) }, "預覽"),
      el("button", { type: "button", "aria-pressed": "false", onclick: (e) => setMode(e, true) }, "LaTeX"));
    function setMode(e, edit) {
      for (const b of mode.children) b.setAttribute("aria-pressed", String(b === e.currentTarget));
      src.hidden = !edit; view.hidden = edit;
      if (edit) src.focus(); else view.innerHTML = latexHtml(src.value);
    }
    const render = () => { view.innerHTML = latexHtml(src.value); };
    let t = 0;
    const recheck = async () => {
      try { const r = await post("/api/actions/note/check", { body: src.value }); paintFindings(finds, r.findings); save.disabled = r.blocking; } catch {}
    };
    src.addEventListener("input", () => { clearTimeout(t); t = setTimeout(recheck, 500); });
    save.disabled = !!d.blocking;
    save.addEventListener("click", async () => {
      save.disabled = true;
      try {
        const r = await post("/api/actions/note/save", { title: title.value, body: src.value, kind: "note", tags: tagsOf(tags.value), kb: kb.map((c) => c.id) });
        close();
        saved("筆記已存", "pc", { note: r.id });
      } catch (e) {
        if (e.body?.findings) paintFindings(finds, e.body.findings);
        LoreShell.toast(e.message, { err: true });
        save.disabled = blocks(e.body?.findings);
      }
    });
    open("note", head("整理成筆記"),
      el("div", { class: "act-body" }, title, el("div", { class: "act-bar" }, mode, el("span", { class: "act-engine" }, d.label || "")), view, src, finds,
        el("label", { class: "act-field" }, el("span", { class: "k" }, "標籤"), tags),
        kb.length ? el("div", { class: "act-field" }, el("span", { class: "k" }, "相關卡"), kbRow) : null),
      foot(el("button", { type: "button", class: "btn", onclick: close }, "取消"), save));
    render();
    paint();
  }

  // ── 做成知識卡 ──
  async function cards(engine) {
    if (!hasTalk()) return;
    busy("cards", "做成知識卡");
    let d;
    try { d = await draft("/api/actions/cards/draft", engine); }
    catch (e) { if (e.name !== "AbortError") failed("cards", "做成知識卡", e, cards); return; }
    if (!d.cards?.length) {
      open("cards", head("做成知識卡"), el("div", { class: "act-wait" }, "這段對話沒有新的概念"), foot(el("button", { type: "button", class: "btn", onclick: close }, "關閉")));
      paint();
      return;
    }
    await loadKatex();
    const send = el("button", { type: "button", class: "btn primary" });
    const rows = d.cards.map((c) => {
      const on = el("input", { type: "checkbox", checked: true, "aria-label": "要這張" });
      const title = el("input", { class: "set-input act-name", value: c.title, "aria-label": "標題" });
      const summary = el("textarea", { class: "act-sum", rows: 2, "aria-label": "摘要" }, c.summary);
      const body = el("textarea", { class: "act-src", rows: 4, "aria-label": "內容", spellcheck: "false" }, c.body);
      let links = [...(c.links || [])];
      const linkRow = el("div", { class: "act-links" });
      const paintLinks = () => linkRow.replaceChildren(...links.map((l) => el("span", { class: "link-chip" }, l.title || l.to, el("button", { type: "button", class: "x", "aria-label": `拿掉 ${l.title}`, onclick: () => { links = links.filter((x) => x !== l); paintLinks(); } }, "×"))));
      paintLinks();
      const tags = el("input", { class: "set-input", value: (c.tags || []).join(", "), "aria-label": "標籤", placeholder: "標籤" });
      const more = el("details", {}, el("summary", {}, "內容"), body);
      const card = el("li", { class: "cand" }, el("div", { class: "cand-head" }, on, title), summary, more,
        el("label", { class: "act-field" }, el("span", { class: "k" }, "標籤"), tags),
        links.length ? el("div", { class: "act-field" }, el("span", { class: "k" }, "連到"), linkRow) : null);
      on.addEventListener("change", () => { card.classList.toggle("off", !on.checked); count(); });
      return { on, li: card, get: () => ({ title: title.value, summary: summary.value, body: body.value, tags: tagsOf(tags.value), links: links.map(({ to, rel }) => ({ to, rel })) }) };
    });
    function count() {
      const n = rows.filter((r) => r.on.checked).length;
      send.textContent = n ? `送到待審（${n}）` : "送到待審";
      send.disabled = !n;
    }
    count();
    send.addEventListener("click", async () => {
      send.disabled = true;
      const topic = (window.LoreEcho?.messages?.() ?? []).find((m) => m.role === "user")?.text || "";
      try {
        const r = await post("/api/actions/cards/save", { cards: rows.filter((x) => x.on.checked).map((x) => x.get()), topic });
        close();
        const n = r.ids?.length || 0;
        saved(`${n} 張卡已放進待審`, "kb", { pending: r.ids?.[0] });
      } catch (e) { LoreShell.toast(e.message, { err: true }); count(); }
    });
    open("cards", head("做成知識卡"),
      el("div", { class: "act-body" }, el("div", { class: "act-bar" }, el("span"), el("span", { class: "act-engine" }, d.label || "")), el("ol", { class: "cands" }, rows.map((r) => r.li))),
      foot(el("button", { type: "button", class: "btn", onclick: close }, "取消"), send));
    paint();
  }

  // ── 記下靈感 (no model) ──
  function idea() {
    const input = document.getElementById("text");
    const line = el("textarea", { class: "act-idea", rows: 3, "aria-label": "靈感", placeholder: "一句話…" }, input?.value.trim() || "");
    const save = el("button", { type: "button", class: "btn primary" }, "存下");
    const go = async () => {
      if (!line.value.trim()) { line.focus(); return; }
      save.disabled = true;
      try {
        const r = await post("/api/actions/note/save", { body: line.value, kind: "idea" });
        if (input && input.value.trim() === line.value.trim()) { input.value = ""; input.dispatchEvent(new Event("input")); }
        close();
        saved("靈感已存", "pc", { note: r.id });
      } catch (e) { LoreShell.toast(e.message, { err: true }); save.disabled = false; }
    };
    save.addEventListener("click", go);
    line.addEventListener("keydown", (e) => { if (e.key === "Enter" && !e.shiftKey && !e.isComposing && e.keyCode !== 229) { e.preventDefault(); go(); } });
    open("idea", head("記下靈感"), el("div", { class: "act-body" }, line), foot(el("button", { type: "button", class: "btn", onclick: close }, "取消"), save));
    paint();
    line.focus();
  }

  const TOOLS = { note: () => note(), cards: () => cards(), idea };

  window.LoreEchoActions = {
    run(tool) {
      if (window.LoreEcho?.busy?.()) { LoreShell.toast("等這句回完"); return; }
      TOOLS[tool]?.();
    },
    /** Esc: closes an untouched preview (true = handled; an edited one stays). */
    escape() {
      if (!A.panel) return false;
      if (!A.dirty) close();
      return true;
    },
    close,
    isOpen: () => !!A.panel,
    latexHtml,
  };
})();
