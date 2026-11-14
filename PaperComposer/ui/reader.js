// 讀過的論文: LoreReader's library while writing (server: crates/server/src/reader.rs).
//
// - The cite popup (PCRefs.picker: the toolbar's 引用 and the right-click 引用文獻…)
//   gets a second tab, 「讀過的」: papers read in LoreReader, searchable. Picking one
//   files its BibTeX into the library (Crossref from its DOI; deduped by DOI, title and
//   key), inserts \cite{key} and writes the paper's .bib like the library tab does.
//   A paper opens to its quotes, highlights and notes: each goes in as a quotation
//   (``…''~\cite[p.~N]{key}) or as a paraphrase draft written by the model, with the
//   citation; and to its symbols and equations: 插入 puts one in (inline $…$, an
//   equation as an equation environment with its label) and registers it — with
//   the symbols an equation uses — in this paper's 對照表 under LoreReader's id
//   (POST /api/papers/{name}/reader/notation; never the global registry). A LaTeX the
//   對照表 already has keeps its row: the inserted text uses that row's macro.
// - The right panel gets 「讀過的」 while the current paragraph cites a paper read in
//   LoreReader: that paper's quotes and notes, read-only, each with its page linked
//   back to LoreReader.
// Every insertion is checked against the notation registry before it is saved.

const PCReader = (() => {
  // its own stylesheet, after styles.css
  document.head.append(Object.assign(document.createElement("link"), { rel: "stylesheet", href: "reader.css" }));
  let list = null;      // papers from /api/reader/papers (null = not loaded)
  let available = false;
  let listFor = null;   // the paper name the list's keys were looked up for
  const details = new Map(); // sha -> { paper, items, symbols }
  const KIND = { quote: "引用句", highlight: "劃線", note: "筆記" };

  async function load(force) {
    const name = state.paper?.name ?? "";
    if (list && !force && listFor === name) return list;
    if (force) details.clear(); // LoreReader may have new highlights since
    try {
      const r = await api(`/api/reader/papers?paper=${encodeURIComponent(name)}`);
      list = r.papers; available = r.available;
    } catch { list = []; available = false; }
    listFor = name;
    return list;
  }

  async function detail(sha) {
    if (!details.has(sha)) details.set(sha, await api(`/api/reader/papers/${sha}`));
    return details.get(sha);
  }

  const meta = (r) => [shortAuthors(r.authors), r.year, r.venue].filter(Boolean).join(" · ");
  function shortAuthors(a) {
    const names = (a || "").split(/\s+and\s+/).map((s) => s.trim()).filter(Boolean);
    const last = (n) => (n.includes(",") ? n.split(",")[0] : n.split(/\s+/).pop()).replace(/[{}]/g, "");
    return names.length > 2 ? `${last(names[0])} et al.` : names.map(last).join(" & ");
  }
  const matches = (r, q) => {
    const hay = `${r.title} ${r.authors} ${r.year} ${r.venue} ${r.doi} ${r.key ?? ""}`.toLowerCase();
    return q.trim().toLowerCase().split(/\s+/).every((w) => hay.includes(w));
  };

  function openPage(sha, page) {
    api("/api/reader/open", { sha, page }).catch((e) => LoreShell.toast(e.message, { err: true, ms: 5000 }));
  }
  function pageLink(sha, page) {
    const a = el("button", "rd-page", page ? `p.${page}` : "");
    a.title = "在 LoreReader 開啟這一頁";
    a.onmousedown = (e) => e.preventDefault();
    a.onclick = (e) => { e.stopPropagation(); openPage(sha, page); };
    return a;
  }

  // ── where things go ──
  /** The caret in the paragraph being edited, else the end of the current paragraph. */
  function target() {
    const ta = document.querySelector("#paper > .para > textarea");
    if (ta) {
      const p = ta.closest(".para");
      return { si: +p.dataset.s, pi: +p.dataset.p, nonWs: ta.value.slice(0, ta.selectionStart).replace(/\s/g, "").length, ta };
    }
    return { si: state.cur.s, pi: state.cur.p, nonWs: null };
  }

  /** The paragraph editor saves on blur: let it finish first. */
  async function settle() {
    const ta = document.querySelector("#paper > .para > textarea");
    if (!ta) return;
    ta.blur();
    for (let t = 0; t < 80 && document.querySelector("#paper > .para > textarea"); t++) await new Promise((ok) => setTimeout(ok, 50));
  }

  /** LaTeX for text copied from a PDF. */
  function tex(s) {
    return s.replace(/\\/g, "\u0000").replace(/([{}%&#_$])/g, "\\$1").replace(/\^/g, "\\^{}").replace(/~/g, "\\~{}")
      .replace(/\u0000/g, "\\textbackslash{}").replace(/[“”]/g, "\"").replace(/"([^"]*)"/g, "``$1''");
  }
  /** Model prose: math stays in $…$, the rest has %, & and # escaped. */
  const prose = (s) => s.split(/(\$[^$]*\$)/).map((x, i) => (i % 2 ? x : x.replace(/(^|[^\\])([%&#])/g, "$1\\$2"))).join("");

  /** Is the text before `at` the end of a sentence (or nothing)? */
  function sentenceAt(was, at) {
    if (!was || at.nonWs == null) return true;
    let i = 0, seen = 0;
    while (i < was.length && seen < at.nonWs) { if (!/\s/.test(was[i])) seen++; i++; }
    return /(^|[.!?。:;])\s*$/.test(was.slice(0, i));
  }

  /** Put `make(sentence)` at `at`, checked, saved, the .bib written (when `key`: a citation). */
  async function put(at, key, make) {
    if (!state.paper) return false;
    const given = !!at.vet;
    if (!given) await settle();
    const sec = sections()[at.si];
    if (!sec) return false;
    const was = sec.paragraphs.length ? sec.paragraphs[at.pi] ?? null : null;
    const whole = sentenceAt(was, at);
    const snippet = make(whole);
    let now;
    if (was == null) now = snippet;
    else if (at.nonWs == null) now = `${was.trimEnd()} ${snippet}`;
    else now = PCRefs.insertAt(was, at.nonWs, snippet, whole ? " " : "~");
    if (given) { if (!(await at.vet(was ?? "", now))) return false; }
    else if (window.PCContext?.vet && !(await PCContext.vet([was ?? ""], [now], at))) return false;
    if (was == null) sec.paragraphs.push(now); else sec.paragraphs[at.pi] = now;
    try { await saveSection(at.si); } catch (e) { LoreShell.toast(`儲存失敗：${e.message}`, { err: true, ms: 6000 }); return false; }
    if (key) {
      try {
        const r = await api(`/api/papers/${encodeURIComponent(state.paper.name)}/bib`, {});
        if (r.missing.length) LoreShell.toast(`文獻庫裡沒有：${r.missing.join(", ")}`, { ms: 6000 });
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
    }
    refreshSection(at.si);
    refreshGit();
    if (given) at.done?.(was ?? "", now);
    else if (at.ta) {
      // keep writing after what went in
      const p = paraEl(at.si, at.pi);
      edit(p, at.si, at.pi);
      const ta = p?.querySelector(":scope > textarea");
      const end = now.indexOf(snippet);
      if (ta && end >= 0) ta.setSelectionRange(end + snippet.length, end + snippet.length);
    }
    side();
    return true;
  }

  // ── the library entry ──
  /** The key this paper is cited by, filing it first. `host` shows the DOI question. */
  async function ensureKey(r, host) {
    if (r.key) return r.key;
    const ask = (body) => api(`/api/reader/papers/${r.sha}/cite`, { paper: state.paper?.name, ...body });
    let res;
    try { res = await ask({}); } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); return null; }
    while (res && !res.key) {
      res = await question(host, r, res, ask);
    }
    if (!res) return null;
    r.key = res.key;
    if (res.added) { LoreShell.toast(`已加入文獻庫：${res.key}`); PCRefs.load?.(); }
    return res.key;
  }

  /** No DOI, or Crossref failed: a DOI to type, or the paper's own data. */
  function question(host, r, res, ask) {
    return new Promise((resolve) => {
      host.innerHTML = "";
      host.hidden = false;
      const box = el("div", "rd-ask");
      box.append(el("div", res.need_doi ? "" : "bad", res.need_doi ? "這篇沒有 DOI" : res.error));
      const row = el("div", "row-end");
      const doi = el("input", "rd-doi");
      doi.placeholder = "DOI";
      doi.value = res.doi ?? "";
      const go = el("button", "primary small", "查詢");
      const own = el("button", "ghost", "只用標題");
      own.title = r.title;
      const cancel = el("button", "ghost", "取消");
      const finish = (v) => { host.innerHTML = ""; host.hidden = true; resolve(v); };
      go.onclick = async () => {
        if (!doi.value.trim()) return doi.focus();
        go.disabled = true;
        try { finish(await ask({ doi: doi.value.trim() })); } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); go.disabled = false; }
      };
      doi.onkeydown = (e) => { if ((e.key === "Enter" && !e.isComposing)) go.click(); };
      own.onclick = async () => { try { finish(await ask({ doi: doi.value.trim() || undefined, minimal: true })); } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); } };
      cancel.onclick = () => finish(null);
      row.append(doi, go);
      if (res.need_doi || res.can_minimal) row.append(own);
      row.append(cancel);
      box.append(row);
      host.append(box);
      setTimeout(() => doi.focus());
    });
  }

  // ── what goes in ──
  const citeCmd = (key, page) => (page ? `\\cite[p.~${page}]{${key}}` : `\\cite{${key}}`);

  async function insertCite(r, at, host) {
    const key = await ensureKey(r, host);
    if (!key) return false;
    return put(at, key, () => `\\cite{${key}}`);
  }

  async function insertQuote(r, item, at, host) {
    const key = await ensureKey(r, host);
    if (!key) return false;
    const body = tex(item.text.trim().replace(/[.。]\s*$/, ""));
    return put(at, key, (whole) => `\`\`${whole ? body : body.replace(/^./, (c) => c.toLowerCase())}''~${citeCmd(key, item.page)}${whole ? "." : ""}`);
  }

  async function insertParaphrase(r, item, at, host, btn) {
    const engine = typeof chosenEngine === "function" ? chosenEngine() : null;
    if (!engine) { LoreShell.toast("連接本機模型或 Claude 後可改寫", { ms: 5000 }); return false; }
    const key = await ensureKey(r, host);
    if (!key) return false;
    const lang = document.querySelector('#cLang [aria-pressed="true"]')?.dataset.v ?? "en";
    const label = btn?.textContent;
    if (btn) { btn.disabled = true; btn.textContent = "改寫中…"; }
    let res;
    try { res = await api("/api/reader/paraphrase", { text: item.text, note: item.note, language: lang, engine }); }
    catch (e) { res = { error: e.message }; }
    if (btn) { btn.disabled = false; btn.textContent = label; }
    if (!res.text) { LoreShell.toast(res.error ?? "改寫失敗", { ms: 6000 }); return false; }
    const s = prose(res.text.trim()).replace(/[.。]\s*$/, "");
    return put(at, key, (whole) => `${whole ? s : s.replace(/^./, (c) => c.toLowerCase())}~\\cite{${key}}${whole ? "." : ""}`);
  }

  // ── pieces shared by the popup and the side panel ──
  function itemRow(r, item, at, host, onDone) {
    const row = el("div", `rd-item ${item.kind}`);
    const head = el("div", "rd-item-head");
    head.append(el("span", "rd-kind", KIND[item.kind] ?? item.kind), pageLink(r.sha, item.page));
    if (item.color && item.kind !== "quote") head.querySelector(".rd-kind").dataset.color = item.color;
    const acts = el("span", "rd-acts");
    if (item.text) {
      const q = el("button", "ghost small", "引文");
      q.title = "原文加引號與引用";
      const p = el("button", "ghost small", "改寫");
      p.title = "改寫草稿與引用";
      for (const b of [q, p]) b.onmousedown = (e) => e.preventDefault();
      q.onclick = async () => { if (await insertQuote(r, item, at(), host)) onDone?.(); };
      p.onclick = async () => { if (await insertParaphrase(r, item, at(), host, p)) onDone?.(); };
      acts.append(q, p);
    }
    head.append(acts);
    row.append(head);
    if (item.text) row.append(el("div", "rd-text", item.text));
    if (item.note) row.append(el("div", "rd-note", item.note));
    return row;
  }

  /** 插入: register in this paper's 對照表 (LoreReader's id), then put it in. */
  async function insertSymbol(r, s, at) {
    if (!state.paper) return false;
    let res;
    try { res = await api(`/api/papers/${encodeURIComponent(state.paper.name)}/reader/notation`, { sha: r.sha, ids: [s.id] }); }
    catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); return false; }
    const row = res.use?.[s.id];
    if (!row?.latex) { LoreShell.toast("這個符號沒有 LaTeX", { err: true }); return false; }
    const body = row.macro ? `\\${row.macro}` : row.latex;
    const text = row.kind === "equation"
      ? `\\begin{equation}\n${body}${row.label ? `\n\\label{${row.label}}` : ""}\n\\end{equation}`
      : `$${body}$`;
    const ok = await put(at, null, () => text);
    if (ok && res.added?.length) LoreShell.toast(`對照表新增 ${res.added.length} 筆`, { action: { label: "對照表", run: () => PCNotation.open(row.id) } });
    return ok;
  }

  function symbolRow(r, s, at, onDone) {
    const row = el("div", "rd-sym");
    const m = el("span", "rd-math");
    try { m.innerHTML = window.katex ? katex.renderToString(s.latex, { throwOnError: false, displayMode: false }) : ""; } catch {}
    if (!m.innerHTML) m.textContent = s.latex;
    const body = el("span", "rd-sym-main");
    body.append(el("span", "", s.meaning || s.meaning_en || (s.kind === "equation" ? `式 ${s.number ?? ""}`.trim() : "—")));
    if (s.quote) body.title = s.quote;
    row.append(m, body, s.page ? pageLink(r.sha, s.page) : el("span"));
    const go = el("button", "ghost small", "插入");
    go.onmousedown = (e) => e.preventDefault(); // keep the caret in the paragraph
    go.onclick = async () => { go.disabled = true; if (await insertSymbol(r, s, at())) onDone?.(); go.disabled = false; };
    row.append(go);
    return row;
  }

  // ── the cite popup's 「讀過的」 tab ──
  function attach(p, at) {
    if (p.querySelector(".rd-tabs")) return;
    const others = [...p.children];
    const tabs = el("div", "rd-tabs seg");
    tabs.setAttribute("role", "tablist");
    const libTab = el("button", "", "文獻庫");
    const readTab = el("button", "", "讀過的");
    tabs.append(libTab, readTab);
    const box = el("div", "mode rd-mode");
    box.hidden = true;
    p.prepend(tabs);
    p.append(box);
    let shown = null; // the other tab's modes as they were
    const show = (read) => {
      libTab.setAttribute("aria-pressed", String(!read));
      readTab.setAttribute("aria-pressed", String(read));
      store.set("pc.citeTab", read ? "read" : "lib");
      if (read) {
        if (!shown) shown = others.map((n) => n.hidden);
        others.forEach((n) => { n.hidden = true; });
        box.hidden = false;
        readMode(box, at, p);
      } else {
        if (shown) others.forEach((n, i) => { n.hidden = shown[i]; });
        shown = null;
        box.hidden = true;
        p.querySelector(".search")?.focus();
      }
    };
    libTab.onclick = () => show(false);
    readTab.onclick = () => show(true);
    show(store.get("pc.citeTab", "lib") === "read");
  }

  async function readMode(box, at, p) {
    box.innerHTML = "";
    const search = el("input", "search");
    search.type = "search";
    search.placeholder = "搜尋讀過的論文";
    const ask = el("div", "rd-askbox");
    ask.hidden = true;
    const rows = el("div", "cite-list");
    box.append(search, ask, rows);
    rows.append(el("p", "empty", "…"));
    await load(true);
    const close = () => p.remove();
    const draw = () => {
      rows.innerHTML = "";
      if (!available) return rows.append(el("p", "empty", "找不到 LoreReader 的書庫"));
      if (!list.length) return rows.append(el("p", "empty", "LoreReader 裡還沒有論文"));
      list.filter((r) => matches(r, search.value)).forEach((r) => {
        const b = el("button", "cite-row");
        const main = el("span", "c-main");
        main.append(el("b", "", r.title || "（無標題）"), el("span", "meta", meta(r) || r.doi || ""));
        const side = el("span", "c-side");
        if (r.quotes + r.marks) side.append(el("span", "tchip", `劃記 ${r.quotes + r.marks}`));
        if (r.key) side.append(el("span", "tchip on", r.key));
        b.append(main, side);
        b.title = "點兩下直接引用";
        b.onclick = () => paperView(box, r, at, close);
        b.ondblclick = async () => { if (await insertCite(r, at, ask)) close(); };
        rows.append(b);
      });
      if (!rows.children.length) rows.append(el("p", "empty", "沒有符合的論文"));
    };
    search.oninput = draw;
    draw();
    setTimeout(() => search.focus());
  }

  async function paperView(box, r, at, close) {
    box.innerHTML = "";
    const head = el("div", "row-end rd-head");
    const back = iconButton("back", "回到清單", () => readMode(box, at, box.closest(".cite-pop")));
    const title = el("div", "rd-title");
    title.append(el("b", "", r.title || "（無標題）"), el("span", "meta", meta(r)));
    const cite = el("button", "primary small", "插入引用");
    head.append(back, title, cite);
    const ask = el("div", "rd-askbox");
    ask.hidden = true;
    const body = el("div", "cite-list rd-body");
    box.append(head, ask, body);
    cite.onclick = async () => { if (await insertCite(r, at, ask)) close(); };
    let d;
    try { d = await detail(r.sha); } catch (e) { body.append(el("p", "empty", e.message)); return; }
    if (!d.items.length) body.append(el("p", "empty", "沒有劃線或引用句"));
    d.items.forEach((it) => body.append(itemRow(r, it, () => at, ask, close)));
    const syms = d.symbols.filter((s) => s.kind !== "equation"), eqs = d.symbols.filter((s) => s.kind === "equation");
    for (const [label, list] of [["符號", syms], ["方程式", eqs]]) {
      if (!list.length) continue;
      body.append(el("div", "rd-sub", `${label} ${list.length}`));
      list.forEach((s) => body.append(symbolRow(r, s, () => at, close)));
    }
    fit(box.closest(".cite-pop"));
  }

  /** The popup grew after it was placed: keep it inside the window. */
  function fit(p) {
    if (!p) return;
    const r = p.getBoundingClientRect();
    if (r.bottom > innerHeight - 8) p.style.top = `${Math.max(8, innerHeight - r.height - 8)}px`;
  }

  // PCRefs.picker opens the popup; the tab is added to it (refs.js stays as it is).
  if (typeof PCRefs !== "undefined") {
    const picker = PCRefs.picker;
    PCRefs.picker = async (anchor, given) => {
      const at = given ?? target(); // before the picker blurs the editor
      await picker(anchor, given);
      const p = document.querySelector("#refPop.cite-pop");
      if (p) attach(p, given ?? { si: at.si, pi: at.pi, nonWs: at.nonWs, ta: at.ta });
    };
  }

  // ── the right panel: what the current paragraph cites from LoreReader ──
  let block = null, timer = 0, shownFor = "";
  function makeBlock() {
    const aside = document.querySelector("aside.right");
    if (!aside) return null;
    // the shell's numbered block, after 01 寫作 and 02 檢查
    const n = String(aside.querySelectorAll(":scope > .block:not(.rv-panel)").length + 1).padStart(2, "0");
    const b = LoreShell.block({ n, title: "讀過的", key: "pc.read" });
    b.classList.add("rd-block");
    b.body.classList.add("rd-side");
    b.hidden = true;
    aside.append(b);
    return b;
  }

  async function side() {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      block ??= makeBlock();
      if (!block) return;
      const text = state.paper ? sections()[state.cur.s]?.paragraphs[state.cur.p] ?? "" : "";
      const keys = [...text.matchAll(/\\[A-Za-z]*cite[a-z]*\*?(?:\[[^\]]*\])*\{([^}]*)\}/g)].flatMap((m) => m[1].split(",").map((k) => k.trim()));
      if (!keys.length) { block.hidden = true; shownFor = ""; return; }
      await load();
      const read = (list ?? []).filter((r) => r.key && keys.includes(r.key));
      const sig = `${state.paper?.name}|${state.cur.s}|${state.cur.p}|${read.map((r) => r.sha).join(",")}`;
      if (!read.length) { block.hidden = true; shownFor = ""; return; }
      if (sig === shownFor) return;
      shownFor = sig;
      const body = block.querySelector(".rd-side");
      body.innerHTML = "";
      for (const r of read) {
        const h = el("div", "rd-side-head");
        h.append(el("b", "", r.title), el("span", "meta", [meta(r), r.key].filter(Boolean).join(" · ")));
        body.append(h);
        let d;
        try { d = await detail(r.sha); } catch { continue; }
        if (!d.items.length) body.append(el("p", "empty", "沒有劃線或引用句"));
        d.items.forEach((it) => body.append(itemRow(r, it, target, null)));
      }
      block.hidden = false;
    }, 150);
  }

  // the current paragraph changes (class "current") or is redrawn
  const paper = document.querySelector("#paper");
  if (paper) new MutationObserver(side).observe(paper, { subtree: true, attributes: true, attributeFilter: ["class"], childList: true });

  // kbcards.js puts its definitions, symbols and equations in the same way
  return { load, attach, side, tex, put, ensureKey, target, prose, papers: () => list ?? [] };
})();
window.PCReader = PCReader;
