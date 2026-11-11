// 知識卡: the author's KnowledgeBase cards while writing (server: crates/server/src/kb.rs).
//
// - The right panel's 「知識卡」 block lists the cards whose title or alias occurs in the
//   current paragraph, plus a search box over all cards. The right-click 「知識卡…」 opens
//   the same list as a popover at the clicked place.
// - Each card: summary, its sources (a paper read in LoreReader links to the page through
//   LoreLinks.open("lr", …)), 開啟 (lore-kb://node/<id>), and
//     定義句  the summary restated as a draft sentence by the configured engine (task
//             pc-kb-paraphrase, class tidy), with \cite{key} when the card has a paper source
//             that LoreReader holds (filed in the library like reader.js does)
//     符號／方程式 插入: registered in this paper's 對照表 (the card's id in its `kb` field),
//             then put in with the registered macro and label
//     貼到筆記  the card's summary as a 註解 on the current paragraph
// - 生成時參考知識卡 (the 寫作 block, off by default): the generation prompt gets the cards
//   for the terms in the points and the paragraph as context (kb::with_cards).
// Insertions go through PCReader.put, so the 對照表 checks run before anything is saved.

const PCKbCards = (() => {
  document.head.append(Object.assign(document.createElement("link"), { rel: "stylesheet", href: "kbcards.css" }));
  let block = null, timer = 0, shownFor = "", available = null;

  const lang = () => document.querySelector('#cLang [aria-pressed="true"]')?.dataset.v ?? "en";
  const paragraph = () => (state.paper ? sections()[state.cur.s]?.paragraphs[state.cur.p] ?? "" : "");

  async function fetchCards(text, q) {
    try { return await api("/api/kb/cards", { paragraph: text, q }); } catch { return { available: false, cards: [] }; }
  }

  // ── insertions ──
  async function definition(card, at, host, btn) {
    const engine = typeof chosenEngine === "function" ? chosenEngine() : null;
    if (!engine) { LoreShell.toast("連接本機模型或 Claude 後可改寫", { ms: 5000 }); return false; }
    const label = btn?.textContent;
    if (btn) { btn.disabled = true; btn.textContent = "改寫中…"; }
    let res;
    try { res = await api("/api/kb/paraphrase", { id: card.id, language: lang(), engine }); } catch (e) { res = { error: e.message }; }
    if (btn) { btn.disabled = false; btn.textContent = label; }
    if (!res.text) { LoreShell.toast(res.error ?? "改寫失敗", { ms: 6000 }); return false; }
    let key = null;
    if (res.paper?.sha) {
      await PCReader.load();
      const r = PCReader.papers().find((x) => x.sha === res.paper.sha);
      if (r) key = await PCReader.ensureKey(r, host);
      else LoreShell.toast("LoreReader 的書庫沒有這篇，句子沒有引用", { ms: 5000 });
      if (r && !key) return false; // the DOI question was cancelled
    }
    const s = PCReader.prose(res.text.trim()).replace(/[.。]\s*$/, "");
    const cite = key ? `~${res.paper.page ? `\\cite[p.~${res.paper.page}]{${key}}` : `\\cite{${key}}`}` : "";
    return PCReader.put(at, key, (whole) => `${whole ? s : s.replace(/^./, (c) => c.toLowerCase())}${cite}${whole ? "." : ""}`);
  }

  async function symbol(card, index, at) {
    if (!state.paper) return false;
    let res;
    try { res = await api(`/api/papers/${encodeURIComponent(state.paper.name)}/kb/notation`, { id: card.id, index }); }
    catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); return false; }
    const row = res.use?.[res.tid];
    if (!row?.latex) { LoreShell.toast("這個符號沒有 LaTeX", { err: true }); return false; }
    const body = row.macro ? `\\${row.macro}` : row.latex;
    const text = row.kind === "equation"
      ? `\\begin{equation}\n${body}${row.label ? `\n\\label{${row.label}}` : ""}\n\\end{equation}`
      : `$${body}$`;
    const ok = await PCReader.put(at, null, () => text);
    if (ok && res.added?.length) LoreShell.toast(`對照表新增 ${res.added.length} 筆`, { action: { label: "對照表", run: () => PCNotation.open(row.id) } });
    return ok;
  }

  /** The card's summary as a 註解 on the current paragraph. */
  async function note(card) {
    if (!state.paper) return false;
    const sec = sections()[state.cur.s];
    if (!sec || sec.paragraphs[state.cur.p] == null) { LoreShell.toast("先點一下要寫註解的段落"); return false; }
    const text = `${card.title}：${card.summary}`;
    try {
      await api(`/api/papers/${encodeURIComponent(state.paper.name)}/review/comments`, { file: sec.file, index: state.cur.p, exact: "", prefix: "", suffix: "", text });
    } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); return false; }
    window.PCReview?.load?.();
    LoreShell.toast("已貼到這段的註解");
    return true;
  }

  // ── a card ──
  function sourceLine(s) {
    const row = el("div", "kb-src");
    if (s.sha) {
      const b = el("button", "rd-page", s.page ? `p.${s.page}` : "LoreReader");
      b.title = "在 LoreReader 開啟";
      b.onmousedown = (e) => e.preventDefault();
      b.onclick = (e) => { e.stopPropagation(); LoreLinks.open("lr", { paper: s.sha, page: s.page || undefined }); };
      row.append(el("span", "kb-src-t", s.title || "PDF"), b);
    } else {
      row.append(el("span", "kb-src-t", [s.title, s.location].filter(Boolean).join(" · ") || s.type));
    }
    return row;
  }

  function cardRow(card, at, onDone) {
    const row = el("div", "kb-card");
    const head = el("div", "kb-head");
    head.append(el("b", "", card.title));
    if (card.matched && card.matched !== card.title) head.append(el("span", "kb-chip", card.matched));
    row.append(head);
    if (card.summary) row.append(el("div", "kb-sum", card.summary));
    card.sources.forEach((s) => row.append(sourceLine(s)));
    const ask = el("div", "rd-askbox");
    ask.hidden = true;
    const acts = el("div", "kb-acts");
    const btn = (label, title, fn) => {
      const b = el("button", "ghost small", label);
      b.title = title;
      b.onmousedown = (e) => e.preventDefault(); // keep the caret in the paragraph
      b.onclick = async () => { b.disabled = true; try { if (await fn(b)) onDone?.(); } finally { b.disabled = false; } };
      acts.append(b);
    };
    if (card.summary) {
      btn("定義句", "改寫成一句草稿", (b) => definition(card, at(), ask, b));
      btn("貼到筆記", "貼成這段的註解", () => note(card));
    }
    const open = el("button", "ghost small", "開啟");
    open.title = "在 KnowledgeBase 開啟";
    open.onmousedown = (e) => e.preventDefault();
    open.onclick = () => LoreLinks.open("kb", { node: card.id });
    acts.append(open);
    row.append(ask, acts);
    card.symbols.forEach((s, i) => {
      const line = el("div", "rd-sym");
      const m = el("span", "rd-math");
      try { m.innerHTML = window.katex ? katex.renderToString(s.latex, { throwOnError: false, displayMode: false }) : ""; } catch {}
      if (!m.innerHTML) m.textContent = s.latex;
      line.append(m, el("span", "rd-sym-main", s.meaning || (s.equation ? "方程式" : "符號")));
      const go = el("button", "ghost small", "插入");
      go.title = s.equation ? "插入方程式並登記到對照表" : "插入符號並登記到對照表";
      go.onmousedown = (e) => e.preventDefault();
      go.onclick = async () => { go.disabled = true; if (await symbol(card, i, at())) onDone?.(); go.disabled = false; };
      line.append(go);
      row.append(line);
    });
    return row;
  }

  /** Search box + list into `host`; `text()` is the paragraph the list follows. */
  function list(host, at, text, onDone, onCards) {
    const search = el("input", "search");
    search.type = "search";
    search.placeholder = "搜尋知識卡";
    search.setAttribute("aria-label", "搜尋知識卡");
    const rows = el("div", "kb-list");
    host.append(search, rows);
    let seq = 0, sig = "";
    const draw = async (force) => {
      const q = search.value.trim(), t = text();
      const key = `${state.paper?.name}|${q}|${q ? "" : t}`;
      if (!force && key === sig) return null;
      sig = key;
      const mine = ++seq;
      const r = await fetchCards(t, q);
      if (mine !== seq) return null;
      available = r.available;
      onCards?.(r, !!q);
      rows.innerHTML = "";
      r.cards.forEach((c) => rows.append(cardRow(c, at, onDone)));
      if (!r.cards.length) rows.append(el("p", "empty", q ? "沒有符合的卡" : "這段沒有提到知識卡"));
      return r;
    };
    let t = 0;
    search.oninput = () => { clearTimeout(t); t = setTimeout(() => draw(), 200); };
    return { draw, search };
  }

  // ── the right panel ──
  function makeBlock() {
    const aside = document.querySelector("aside.right");
    if (!aside) return null;
    const n = String(aside.querySelectorAll(":scope > .block:not(.rv-panel):not([hidden])").length + 1).padStart(2, "0");
    const b = LoreShell.block({ n, title: "知識卡", key: "pc.kb", open: false });
    b.classList.add("kb-block");
    b.hidden = true;
    const l = list(b.body, () => PCReader.target(), paragraph, null, (r, searching) => {
      b.hidden = !r.available;
      b.setCount(r.cards.length ? String(r.cards.length) : "");
      if (toggle) toggle.hidden = !r.available;
    });
    b.draw = l.draw;
    aside.append(b);
    return b;
  }

  function side() {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      if (!state.paper) { if (block) block.hidden = true; return; }
      block ??= makeBlock();
      if (!block) return;
      await block.draw();
    }, 200);
  }

  // ── the right-click popover ──
  let pop = null;
  function close() {
    pop?.remove();
    pop = null;
    document.removeEventListener("mousedown", away, true);
    document.removeEventListener("keydown", esc, true);
  }
  const away = (e) => { if (pop && !pop.contains(e.target)) close(); };
  const esc = (e) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); } };

  /** The list at `anchor`; `at` is context.js's place in the paragraph (with vet and done). */
  async function openAt(anchor, at) {
    close();
    pop = el("div", "menu kb-pop");
    pop.setAttribute("role", "dialog");
    pop.setAttribute("aria-label", "知識卡");
    const l = list(pop, () => at, () => sections()[at.si]?.paragraphs[at.pi] ?? "", close);
    document.body.append(pop);
    const r = anchor?.getBoundingClientRect?.() ?? { left: innerWidth / 2 - 180, bottom: 120 };
    pop.style.left = `${Math.max(8, Math.min(r.left, innerWidth - pop.offsetWidth - 8))}px`;
    pop.style.top = `${Math.max(8, Math.min(r.bottom + 6, innerHeight - 220))}px`;
    setTimeout(() => { document.addEventListener("mousedown", away, true); document.addEventListener("keydown", esc, true); });
    await l.draw(true);
    pop.style.maxHeight = `${Math.max(180, innerHeight - parseFloat(pop.style.top) - 12)}px`;
    l.search.focus();
  }

  // ── 生成時參考知識卡 (the 寫作 block) ──
  let toggle = null;
  async function setting() {
    const cond = document.querySelector(".compose .cond");
    if (!cond || toggle) return;
    toggle = el("label", "span2 kb-ctx");
    toggle.hidden = !available;
    const seg = el("span", "seg");
    const off = el("button", "", "關"), on = el("button", "", "開");
    off.dataset.v = "0"; on.dataset.v = "1";
    seg.append(off, on);
    const set = (v) => { off.setAttribute("aria-pressed", String(!v)); on.setAttribute("aria-pressed", String(v)); };
    toggle.append(el("span", "", "生成時參考知識卡"), seg);
    cond.append(toggle);
    let v = false;
    try { v = !!(await api("/api/kb/settings")).context; } catch {}
    set(v);
    seg.onclick = async (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      const want = b.dataset.v === "1";
      try { await api("/api/kb/settings", { context: want }, "PUT"); set(want); } catch (err) { LoreShell.toast(err.message, { err: true }); }
    };
  }

  // app.js (el, api, state) loads after this file
  addEventListener("load", () => { setting(); side(); });
  const paper = document.querySelector("#paper");
  if (paper) new MutationObserver(side).observe(paper, { subtree: true, attributes: true, attributeFilter: ["class"], childList: true });
  return { side, openAt, close };
})();
window.PCKbCards = PCKbCards;
