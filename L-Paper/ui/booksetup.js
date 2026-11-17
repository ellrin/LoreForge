// 寫書 start panel (docs/book.md 統一入口): the one place a book begins. What it starts from
// (L-Reader papers, files, a folder, keywords, knowledge cards — mixed, or nothing for a blank
// book), which book it goes into, and how the book looks. 新書 on the home, L-Reader's 寫 review
// and ?book-setup=<id> all open this; another app fills part of it in first (POST /api/book/setup)
// and the author still adds and removes. ?book-setup=mock shows it without a server.
"use strict";

const PCBookSetup = (() => {
  const E = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c);
    return e;
  };
  const ic = (name) => E("span", { "data-icon": name, class: "bks-ic" });
  const paint = (el) => { try { window.paintIcons?.(el); } catch {} };
  const toast = (m, err) => LoreShell.toast(m, err ? { err: true, ms: 7000 } : undefined);
  const base = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";
  // IME: the Enter that picks a candidate is not "add" (WebKit ends the composition first)
  let imeEnd = 0;
  document.addEventListener("compositionend", () => (imeEnd = performance.now()), true);
  const composing = (e) => e.isComposing || e.keyCode === 229 || performance.now() - imeEnd < 80;

  const FORMATS = [["notes", "電子書"], ["review1", "Review 單欄"], ["review2", "Review 雙欄"]];
  const LEN = [["short", "短"], ["standard", "標準"], ["long", "長"]];
  const LANG = [["en-zh", "英中"], ["en", "English"], ["zh", "中文"]];
  const PAGE = [["b5", "B5"], ["a4", "A4"]];
  const DEFAULTS = { sources: [], keywords: [], cards: [], extras: {}, book: "new", title: "", folder: "", format: "notes", page: "b5", length: "standard", lang: "en-zh", make_cards: true };
  const KEEP = ["format", "page", "length", "lang", "make_cards"];   // remembered between books
  const pref = {
    get() { try { return JSON.parse(localStorage.getItem("pc.bookSetup") ?? "{}") ?? {}; } catch { return {}; } },
    set(s) { try { localStorage.setItem("pc.bookSetup", JSON.stringify(Object.fromEntries(KEEP.map((k) => [k, s[k]])))); } catch {} },
  };

  const S = { root: null, id: null, mock: false, setup: null, opt: null, cardNames: {}, picker: false, busy: false };

  // ── the server, or a pretend one for ?book-setup=mock ──
  const MOCK = {
    options: { books: [{ id: "b1", title: "Contrastive Learning from the Ground Up" }], folder: "/Users/ellrin/Documents/Books", formats: FORMATS.map(([id, label]) => ({ id, label })),
      papers: [{ sha: "a".repeat(64), title: "Gated Contrast for Synthetic Signals", year: "2024" }, { sha: "b".repeat(64), title: "A Survey of Method A and Method B", year: "2023" }, { sha: "c".repeat(64), title: "Small-Data Benchmarks Revisited", year: "2022" }] },
    setup: { sources: [{ kind: "lr", sha: "a".repeat(64), title: "Gated Contrast for Synthetic Signals" }], extras: { terms: [{ term: "InfoNCE", n: 4 }, { term: "hard negatives", n: 2 }] } },
    cards: (q) => [["k1", "softmax", "learning"], ["k2", "mutual information", "unknown"], ["k3", "temperature scaling", "unknown"], ["k4", "data augmentation", "learning"]].filter(([, t]) => !q || t.includes(q.toLowerCase())).map(([id, title, state]) => ({ id, title, state })),
  };
  const srv = {
    async options() {
      if (S.mock) return MOCK.options;
      try { return await api("/api/book/options"); }
      catch {
        // an older server: what it has
        const [b, p] = await Promise.all([api("/api/books").catch(() => ({})), api("/api/reader/papers").catch(() => ({}))]);
        return { books: b.books ?? [], folder: b.folder ?? "", formats: [{ id: "notes", label: "電子書" }], papers: p.papers ?? [] };
      }
    },
    async load(id) {
      if (S.mock) return MOCK.setup;
      if (id) return api(`/api/book/setup/${encodeURIComponent(id)}`);
      return {};
    },
    /** A place on the server for this panel's choices (L-Echo's file search adds to it). */
    async hold() {
      if (S.mock || S.id) return S.id;
      try { S.id = (await api("/api/book/setup", body())).setup ?? null; } catch { S.id = null; }
      return S.id;
    },
    async save() { if (!S.mock && S.id) await api(`/api/book/setup/${encodeURIComponent(S.id)}`, body(), "PUT").catch(() => {}); },
    cards: async (q) => (S.mock ? MOCK.cards(q) : (await api(`/api/book/cards?q=${encodeURIComponent(q)}`)).cards ?? []),
    dialog: (kind, start) => api("/api/shell/dialog", { kind, start: start || null }),
  };

  const body = () => ({ ...S.setup, sources: S.setup.sources.map(({ kind, sha, path, title }) => (kind === "lr" ? { kind, sha, title } : { kind, path })) });
  const hasStart = () => S.setup.sources.length + S.setup.keywords.length + S.setup.cards.length + (S.setup.extras?.terms?.length ?? 0) > 0;
  const sameSource = (a, b) => a.kind === b.kind && (a.kind === "lr" ? a.sha === b.sha : a.path === b.path);
  function addSource(x) { if (!S.setup.sources.some((y) => sameSource(x, y))) S.setup.sources.push(x); }

  function shell() {
    if (S.root) return S.root;
    S.root = E("section", { class: "bkj bks", id: "bookSetup", "aria-label": "新書" });
    document.body.append(S.root);
    return S.root;
  }

  // ── 01 起點 ──
  const KIND = { lr: ["library", "論文"], file: ["file", "檔案"], folder: ["folder", "資料夾"] };
  function chip(icon, text, title, onRemove, cls = "") {
    return E("span", { class: `bks-chip ${cls}`, title }, ic(icon), E("span", { class: "t" }, text),
      E("button", { type: "button", class: "x", "aria-label": `移除 ${text}`, onclick: onRemove }, ic("x")));
  }
  function startBlock() {
    const b = LoreShell.block({ n: "01", title: "起點", key: "bks.start" });
    const st = S.setup;
    const chips = E("div", { class: "bks-chips" });
    const redraw = () => { draw(); };
    st.sources.forEach((x, i) => chips.append(chip(KIND[x.kind][0], x.kind === "lr" ? (x.title || x.sha.slice(0, 8)) : (x.label || base(x.path)), x.kind === "lr" ? KIND.lr[1] : x.path, () => { st.sources.splice(i, 1); redraw(); }, x.kind)));
    (st.extras?.terms ?? []).forEach((t, i) => chips.append(chip("tag", t.term, "L-Reader 的紅字名詞", () => { st.extras.terms.splice(i, 1); redraw(); }, "kw")));
    st.keywords.forEach((k, i) => chips.append(chip("tag", k, "關鍵字", () => { st.keywords.splice(i, 1); redraw(); }, "kw")));
    st.cards.forEach((id, i) => chips.append(chip("cards", S.cardNames[id] ?? id, "知識卡", () => { st.cards.splice(i, 1); redraw(); }, "card")));
    if (!chips.childElementCount) chips.append(E("span", { class: "bks-empty" }, "空白的書"));

    // add: papers of L-Reader, files, a folder, L-Echo's search, cards; keywords are typed
    const papers = (S.opt?.papers ?? []).filter((p) => !st.sources.some((x) => x.kind === "lr" && x.sha === p.sha));
    const addPaper = E("button", { type: "button", class: "btn small", disabled: !papers.length, title: papers.length ? null : "L-Reader 沒有其他論文", onclick: (e) => {
      LoreShell.menu(e.currentTarget, papers.slice(0, 40).map((p) => ({ label: `${p.title || p.sha.slice(0, 8)}${p.year ? `　${p.year}` : ""}`, run: () => { addSource({ kind: "lr", sha: p.sha, title: p.title }); redraw(); } })));
    } }, ic("library"), "論文");
    addPaper.dataset.opensMenu = "";
    const pick = async (kind) => {
      try {
        const r = await srv.dialog(kind === "file" ? "pdf" : "folder", st.folder);
        const paths = r.paths ?? (r.path ? [r.path] : []);
        paths.forEach((path) => addSource({ kind, path }));
        if (paths.length) redraw();
      } catch (e) { typePath(kind, e.message); }
    };
    const addFile = E("button", { type: "button", class: "btn small", onclick: () => (S.mock ? (addSource({ kind: "file", path: "/Users/ellrin/Papers/method-a.pdf" }), redraw()) : pick("file")) }, ic("file"), "檔案");
    const addFolder = E("button", { type: "button", class: "btn small", onclick: () => (S.mock ? (addSource({ kind: "folder", path: "/Users/ellrin/Papers/contrastive" }), redraw()) : pick("folder")) }, ic("folder"), "資料夾");
    const addCard = E("button", { type: "button", class: "btn small", "aria-pressed": String(S.picker), onclick: () => { S.picker = !S.picker; redraw(); } }, ic("cards"), "知識卡");
    const kw = E("input", { class: "set-input bks-kw", placeholder: "加關鍵字", "aria-label": "加關鍵字", spellcheck: "false" });
    const addKw = () => {
      const words = kw.value.split(/[,，、;\n]+/).map((w) => w.trim()).filter(Boolean);
      if (!words.length) return;
      for (const w of words) if (!st.keywords.some((k) => k.toLowerCase() === w.toLowerCase())) st.keywords.push(w);
      S.focusKw = true;
      redraw();
    };
    kw.onkeydown = (e) => { if (e.key === "Enter" && !composing(e)) { e.preventDefault(); addKw(); } };
    const adds = E("div", { class: "bks-adds" }, E("span", { class: "ql" }, "加入"), addPaper, addFile, addFolder);
    // L-Echo's 搜尋檔案 (the shared way to find a file): what it picks lands in this panel
    if (window.LoreFindFile && S.id) {
      adds.append(LoreFindFile.button({ purpose: "寫書", exts: ["pdf"], context: { setup: S.id }, onAdded: async () => {
        try { const v = await srv.load(S.id); (v.sources ?? []).forEach(addSource); draw(); } catch {}
      } }));
    }
    adds.append(addCard, kw);
    b.body.append(chips, adds);
    if (S.picker) b.body.append(cardPicker(redraw));
    if (S.focusKw) { S.focusKw = false; setTimeout(() => kw.focus()); }
    return b;
  }

  /** No system dialog (the browser, or an older app): the path is typed or pasted. */
  async function typePath(kind) {
    let input;
    const ok = await LoreShell.form({
      title: kind === "file" ? "檔案" : "資料夾",
      build: (box) => { input = E("input", { class: "set-input", placeholder: kind === "file" ? "/…/paper.pdf" : "/…/papers", spellcheck: "false", style: "width:100%" }); box.append(input); setTimeout(() => input.focus()); },
      actions: [{ id: "go", label: "加入", primary: true }],
    });
    const path = input?.value.trim();
    if (ok === "go" && path) { addSource({ kind, path }); draw(); }
  }

  /** Cards that are not 記得 yet: search, click to add. */
  function cardPicker(redraw) {
    const box = E("div", { class: "bks-picker" });
    const q = E("input", { class: "set-input", placeholder: "找知識卡", "aria-label": "找知識卡", spellcheck: "false" });
    const list = E("div", { class: "bks-cards" });
    let seq = 0;
    const fill = async () => {
      const mine = ++seq;
      let cards = [];
      try { cards = await srv.cards(q.value.trim()); } catch (e) { list.replaceChildren(E("span", { class: "bks-empty" }, e.message)); return; }
      if (mine !== seq) return;
      const left = cards.filter((c) => !S.setup.cards.includes(c.id));
      list.replaceChildren(...(left.length ? left.map((c) => E("button", { type: "button", class: `chip ${c.state ?? "unknown"}`, title: c.state === "learning" ? "學習中" : "不熟", onclick: () => { S.cardNames[c.id] = c.title; S.setup.cards.push(c.id); redraw(); } }, c.title)) : [E("span", { class: "bks-empty" }, "沒有")]));
    };
    let t = 0;
    q.oninput = () => { clearTimeout(t); t = setTimeout(fill, 200); };
    q.onkeydown = (e) => { if (e.key === "Escape") { e.stopPropagation(); S.picker = false; redraw(); } };
    box.append(q, list);
    fill();
    setTimeout(() => q.focus());
    return box;
  }

  // ── 02 書 ──
  function bookBlock() {
    const st = S.setup;
    const b = LoreShell.block({ n: "02", title: "書", key: "bks.book" });
    const books = E("div", { class: "bks-books", role: "radiogroup", "aria-label": "書" });
    for (const [id, label, title] of [["new", "新書"], ...(S.opt?.books ?? []).map((x) => [x.id, `加進〈${x.title || x.id}〉`, x.path])]) {
      books.append(E("button", { type: "button", title, "aria-pressed": String(st.book === id), onclick: () => { st.book = id; draw(); } }, label));
    }
    b.body.append(books);
    if (st.book === "new") {
      const title = E("input", { class: "set-input bks-title", value: st.title ?? "", placeholder: hasStart() ? "書名（可留空）" : "書名", "aria-label": "書名", oninput: (e) => (st.title = e.target.value) });
      const folder = E("input", { class: "set-input bks-folder", value: st.folder ?? "", placeholder: "存放的資料夾", "aria-label": "存放的資料夾", spellcheck: "false", oninput: (e) => (st.folder = e.target.value) });
      const row = E("div", { class: "bks-row" }, folder);
      if (window.PCOpen?.desktop?.() || S.mock) {
        row.append(E("button", { type: "button", class: "btn small", onclick: async () => {
          if (S.mock) return;
          try { const r = await srv.dialog("folder", folder.value); if (r?.path) { st.folder = r.path; folder.value = r.path; folder.scrollLeft = folder.scrollWidth; } } catch (e) { toast(e.message, true); }
        } }, ic("folder"), "選擇…"));
      }
      b.body.append(title, row);
      setTimeout(() => (folder.scrollLeft = folder.scrollWidth));
    }
    return b;
  }

  // ── 03 樣式 ──
  function seg(key, items, label) {
    const g = E("div", { class: "seg", role: "radiogroup", "aria-label": label });
    for (const [v, t] of items) g.append(E("button", { type: "button", "aria-pressed": String(S.setup[key] === v), onclick: () => { S.setup[key] = v; pref.set(S.setup); draw(); } }, t));
    return g;
  }
  /** A format as a small page: the margin column of the notes, one column, two columns. */
  function formatCard(id, label, on, usable) {
    const lines = (n) => Array.from({ length: n }, (_, i) => E("i", { style: `width:${[100, 92, 96, 70, 100, 88, 94, 60][i % 8]}%` }));
    const page = E("span", { class: `bks-page ${id}`, "aria-hidden": "true" },
      E("b"),
      id === "review2" ? E("span", { class: "cols" }, E("span", {}, ...lines(6)), E("span", {}, ...lines(6))) : E("span", { class: "col" }, ...lines(6)),
      id === "notes" ? E("span", { class: "side" }, ...lines(2)) : null);
    return E("button", { type: "button", class: "bks-format", "aria-pressed": String(on), disabled: !usable, onclick: () => { S.setup.format = id; pref.set(S.setup); draw(); } }, page, E("span", { class: "nm" }, label));
  }
  function lookBlock() {
    const st = S.setup;
    const b = LoreShell.block({ n: "03", title: "樣式", key: "bks.look" });
    const have = new Set((S.opt?.formats ?? []).map((f) => f.id));
    const adding = st.book !== "new";
    const grid = E("div", { class: "bks-look" });
    if (!adding) {
      grid.append(E("span", { class: "ql" }, "格式"), E("div", { class: "bks-formats", role: "radiogroup", "aria-label": "格式" }, ...FORMATS.map(([id, label]) => formatCard(id, label, st.format === id, have.has(id)))));
      if (st.format === "notes") grid.append(E("span", { class: "ql" }, "頁面"), seg("page", PAGE, "頁面"));
    }
    if (hasStart()) {
      grid.append(E("span", { class: "ql" }, "篇幅"), seg("length", LEN, "篇幅"), E("span", { class: "ql" }, "語言"), seg("lang", LANG, "語言"));
      const sw = E("input", { type: "checkbox", checked: st.make_cards, id: "bksCards", onchange: (e) => { st.make_cards = e.target.checked; pref.set(st); } });
      grid.append(E("span", { class: "ql" }, "知識卡"), E("label", { class: "bks-switch", for: "bksCards" }, sw, "不懂的名詞建立知識卡（標成不熟）"));
    }
    b.body.append(grid);
    return b;
  }

  function draw() {
    const root = shell();
    const head = E("header", { class: "bkj-head" }, ic("bookWrite"), E("h2", {}, "新書"), E("span", { style: "flex:1" }),
      E("button", { type: "button", class: "ico close", "aria-label": "關閉 Esc", onclick: close }, ic("x")));
    if (!S.setup) { root.replaceChildren(head, E("div", { class: "bkj-body" }, E("p", { class: "bkj-msg" }, "…"))); paint(root); return; }
    const top = root.querySelector(".bkj-body")?.scrollTop ?? 0;
    const bodyEl = E("div", { class: "bkj-body wide bks-body" }, startBlock(), bookBlock(), lookBlock());
    const foot = E("footer", { class: "bkj-foot" }, E("span", { style: "flex:1" }));
    if (hasStart()) foot.append(
      E("button", { type: "button", class: "btn", disabled: S.busy, onclick: () => start("direct") }, "直接產生"),
      E("button", { type: "button", class: "btn primary", disabled: S.busy, onclick: () => start("review") }, "先看大綱"));
    else foot.append(E("button", { type: "button", class: "btn primary", disabled: S.busy || S.setup.book !== "new", onclick: blank }, "建立空白的書"));
    root.replaceChildren(head, bodyEl, foot);
    bodyEl.scrollTop = top;
    paint(root);
  }

  async function blank() {
    const st = S.setup;
    if (!st.title.trim()) { S.root.querySelector(".bks-title")?.focus(); return; }
    if (!st.folder.trim()) { S.root.querySelector(".bks-folder")?.focus(); return; }
    if (S.mock) { toast("示範：建立空白的書"); return; }
    S.busy = true; draw();
    try {
      const r = await api("/api/books", { title: st.title.trim(), folder: st.folder.trim(), page: st.page, format: st.format });
      close();
      await openPaper(r.id);
      toast("已建立");
    } catch (e) { toast(e.message, true); } finally { S.busy = false; if (S.root && !S.root.hidden) draw(); }
  }

  async function start(mode) {
    const st = S.setup;
    if (st.book === "new" && !st.folder.trim()) { S.root.querySelector(".bks-folder")?.focus(); return; }
    if (S.mock) { window.__bookSetup = { ...body(), mode }; close(); PCBookJob.open("mock"); return; }
    S.busy = true; draw();
    try {
      const v = await api("/api/book", { ...body(), mode });
      close();
      PCBookJob.open(v.job);
    } catch (e) { toast(e.message, true); } finally { S.busy = false; if (S.root && !S.root.hidden) draw(); }
  }

  const esc = (e) => { if (e.key === "Escape" && S.root && !S.root.hidden && !document.querySelector("dialog[open], .ask, .menu")) { e.preventDefault(); close(); } };
  function close() {
    if (S.root) S.root.hidden = true;
    document.removeEventListener("keydown", esc, true);
    try { const u = new URL(location.href); u.searchParams.delete("book-setup"); history.replaceState(null, "", u); } catch {}
  }

  /** Open the panel: empty, or with what another app (or `prefill`) already chose. */
  async function open(id, prefill) {
    S.mock = id === "mock";
    S.id = id && !S.mock && id !== "new" ? id : null;
    S.setup = null; S.picker = false; S.busy = false; S.cardNames = {};
    shell().hidden = false;
    document.addEventListener("keydown", esc, true);
    draw();
    try {
      const [opt, given] = await Promise.all([srv.options(), srv.load(S.id)]);
      S.opt = opt;
      const v = { ...DEFAULTS, ...pref.get(), folder: opt.folder ?? "", ...given, ...(prefill ?? {}) };
      S.setup = { ...v, sources: [...(v.sources ?? [])], keywords: [...(v.keywords ?? [])], cards: [...(v.cards ?? [])], extras: { ...(v.extras ?? {}), terms: [...(v.extras?.terms ?? [])] } };
      if (!(opt.formats ?? []).some((f) => f.id === S.setup.format)) S.setup.format = "notes";
      // the book that already has one of these papers is the likely target
      const holder = (opt.books ?? []).find((b) => S.setup.sources.some((x) => x.kind === "lr" && (b.papers ?? []).includes(x.sha)));
      if (holder && !given?.book) S.setup.book = holder.id;
      await srv.hold();
    } catch (e) { S.setup = { ...DEFAULTS, ...pref.get() }; S.opt = S.opt ?? { books: [], formats: [], papers: [] }; toast(e.message, true); }
    if (!S.root.hidden) draw();
  }

  // start-up: ?book-setup=<id> (another Lore app sent the author here)
  setTimeout(() => { const id = new URL(location.href).searchParams.get("book-setup"); if (id) open(id); }, 400);
  return { open, close, state: () => S };
})();
