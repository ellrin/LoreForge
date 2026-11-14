// Start screen (封面). Level 1: what to do, one large card per mode (MODES:
// a new mode is one entry), then 最近 (one list, both kinds of papers). At launch the
// paper that was open last opens again. Level 2 (寫論文): new paper, import, and the
// workspace's papers to continue, most recently edited first. Level 2 (寫書): 新書 and
// the books in books.toml, most recently edited first.
// The brand and 首頁 in the paper menu come back here.

const PCHome = (() => {
  // 討論草稿 phases (session.json `phase`) as draft.js names its steps
  const PHASES = { topic: "主題", record: "實驗記錄", plan: "題組", interview: "對話", claims: "主張", review: "校正", outline: "大綱", draft: "草稿", audit: "審稿", revise: "修訂" };

  const MODES = [
    { id: "paper", icon: "modePaper", label: "寫論文", enabled: true, onOpen: () => show(2) },
    { id: "book", icon: "modeBook", label: "寫書", enabled: true, onOpen: () => show(2, "book") },
    { id: "latex", icon: "openLatex", label: "開啟 LaTeX", enabled: true, onOpen: (card) => PCOpen.latex(card) },
    { id: "word", icon: "openWord", label: "開啟 Word", enabled: true, onOpen: (card) => PCOpen.word(card) },
    { id: "notes", icon: "notebook", label: "筆記", enabled: true, onOpen: () => PCNotes.open() },
  ];

  let level = 1;
  let mode = "paper"; // which level 2: "paper" (寫論文) or "book" (寫書)
  let papers = null; // last /api/home/papers
  let books = null; // last /api/books
  const root = () => $("#home");
  const isOpen = () => !root().hidden;

  function show(n = level, m = n === 2 ? mode : "paper") {
    level = n;
    mode = m;
    root().hidden = false;
    document.querySelector(".app").classList.add("at-home");
    PCFrame.home(true);
    $("#fileMenu").hidden = true;
    draw(true);
  }

  function close() {
    if (!isOpen()) return;
    root().hidden = true;
    document.querySelector(".app").classList.remove("at-home");
    PCFrame.home(false);
    $("#fileMenu").hidden = true;
  }


  function card(art, label, onClick, enabled = true) {
    const b = el("button", "home-card");
    b.type = "button";
    b.disabled = !enabled;
    b.append(art, el("span", "home-label", label));
    if (enabled && onClick) b.onclick = () => onClick(b);
    return b;
  }

  /** A numbered section of the start panel (same as SlideComposer's 01 開始 / 02 最近). */
  function block(n, title, body) {
    const b = el("section", "home-block");
    const h = el("div", "home-bh");
    h.append(el("span", "n", n), el("h3", "", title));
    b.append(h, body);
    return b;
  }

  function iconArt(name) {
    const a = el("div", "home-art");
    a.append(icon(name));
    return a;
  }

  async function draw(focus) {
    let box = root();
    box.innerHTML = "";
    box.dataset.level = level;
    if (level === 1) {
      const inner = el("div", "home-inner");
      const grid = el("div", "home-grid modes");
      MODES.forEach((m) => {
        const c = card(iconArt(m.icon), m.label, m.onOpen, m.enabled);
        c.dataset.mode = m.id;
        c.dataset.opensMenu = ""; // a mode may open a form in the paper menu
        grid.append(c);
      });
      inner.append(block("01", "開始", grid));
      box.append(inner);
      if (focus) grid.querySelector(".home-card:not(:disabled)")?.focus();
      // 最近: one list, the workspace's papers and the folders opened where they are
      try { papers = await api("/api/home/papers"); } catch { papers = papers ?? []; }
      if (grid.isConnected && level === 1 && papers.length) inner.append(block("02", "最近", recentList(papers.slice(0, 6))));
      return;
    }

    const inner = el("div", "home-inner wide");
    box.append(inner);
    box = inner;
    if (mode === "book") return drawBooks(box, focus);
    const head = el("div", "home-head");
    const back = iconButton("back", "返回", () => show(1));
    const add = el("button", "primary small", "＋ 新論文");
    add.onclick = () => PCNew.open();
    const opener = (label, ic, fn) => {
      const b = el("button", "ghost", label);
      b.prepend(icon(ic));
      b.dataset.opensMenu = "";
      b.onclick = () => fn(b);
      return b;
    };
    head.append(back, el("h2", "", "寫論文"), el("span", "grow"), add, opener("LaTeX…", "openLatex", PCOpen.latex), opener("Word…", "openWord", PCOpen.word));
    // the desktop app opens files where they are; arXiv still downloads into the workspace
    if (PCOpen.desktop()) head.append(opener("arXiv", "upload", importForm));
    const sub = el("div", "home-sub", "繼續");
    const grid = el("div", "home-grid papers");
    box.append(head, sub, grid);
    if (focus) add.focus();
    // the last list at once, then the fresh one
    const fill = () => {
      const had = grid.contains(document.activeElement) && document.activeElement.dataset.paper;
      sub.hidden = !papers?.some((p) => p.kind !== "book");
      grid.replaceChildren(...(papers ?? []).filter((p) => p.kind !== "book").map(paperCard));
      if (had) grid.querySelector(`[data-paper="${CSS.escape(had)}"]`)?.focus();
    };
    if (papers) fill();
    try { papers = await api("/api/home/papers"); }
    catch (e) { papers = papers ?? []; LoreShell.toast(e.message, { err: true, ms: 6000 }); }
    if (grid.isConnected) fill(); // unless the screen was redrawn or left meanwhile
  }

  // ── 寫書 ──────────────────────────────────────────────
  async function drawBooks(box, focus) {
    const head = el("div", "home-head");
    const add = el("button", "primary small", "＋ 新書");
    add.dataset.opensMenu = "";
    add.onclick = () => newBookForm(add);
    const fromPaper = el("button", "ghost small", "從論文寫…");
    fromPaper.dataset.opensMenu = "";
    fromPaper.onclick = () => paperMenu(fromPaper);
    head.append(iconButton("back", "返回", () => show(1)), el("h2", "", "寫書"), el("span", "grow"), fromPaper, add);
    const jobsBox = el("div", "home-jobs");
    const sub = el("div", "home-sub", "繼續");
    const grid = el("div", "home-grid papers");
    box.append(head, jobsBox, sub, grid);
    if (focus) add.focus();
    const fill = () => {
      sub.hidden = !books?.books?.length;
      grid.replaceChildren(...(books?.books ?? []).map(bookCard));
    };
    if (books) fill();
    PCBookJob.list().then((jobs) => {
      // unfinished 寫書 jobs: waiting for the review, stopped, failed, interrupted
      jobsBox.replaceChildren(...jobs.map((j) => {
        const b = el("button", "home-job");
        const state = j.state === "review" ? "等你微調" : j.state === "running" ? "進行中" : j.state === "failed" ? "沒有完成" : `做到第 ${j.step ?? "?"} 步`;
        b.append(icon("bookWrite"), el("span", "open-name", j.title || "寫書"), el("span", "open-where", state));
        b.onclick = () => PCBookJob.open(j.id);
        return b;
      }));
    }).catch(() => {});
    try { books = await api("/api/books"); }
    catch (e) { books = books ?? { books: [] }; LoreShell.toast(e.message, { err: true, ms: 6000 }); }
    if (grid.isConnected) fill();
  }

  /** 從論文寫…: LoreReader's papers; one starts a new book (the review window first). */
  async function paperMenu(anchor) {
    let papers = [];
    try { papers = (await api("/api/reader/papers")).papers ?? []; } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); return; }
    if (!papers.length) { LoreShell.toast("LoreReader 沒有論文"); return; }
    LoreShell.menu(anchor, papers.slice(0, 30).map((p) => ({ label: p.title || p.sha.slice(0, 8), run: async () => {
      try { await PCBookJob.start(p.sha, { folder: books?.folder ?? undefined }); } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
    } })));
  }

  function bookCard(b) {
    const art = el("div", "home-art page");
    const faux = () => fauxPage({ title: b.title, name: b.title });
    if (b.thumb) {
      const img = el("img");
      img.alt = "";
      img.loading = "lazy";
      img.src = `/api/home/papers/${encodeURIComponent(b.id)}/thumb?t=${b.modified}`;
      img.onerror = () => img.replaceWith(faux());
      art.append(img);
    } else art.append(faux());
    const c = card(art, b.title || b.id, () => openPaper(b.id));
    c.dataset.paper = b.id;
    c.title = b.path;
    c.setAttribute("aria-current", String(b.id === state.paper?.name));
    const item = el("div", "home-item");
    item.append(c);
    return item;
  }

  /** 新書: title and the folder it goes in (the last one used by default). */
  function newBookForm(anchor) {
    const m = menuForm("新書", null, anchor);
    const title = el("input", "field");
    title.placeholder = "書名";
    const row = el("div", "form-row");
    const folder = el("input", "field");
    folder.placeholder = "存放的資料夾";
    folder.value = books?.folder ?? "";
    row.append(folder);
    let page = "b5";
    const pages = el("div", "seg");
    for (const [v, t] of [["b5", "B5"], ["a4", "A4"]]) {
      const b = el("button", "", t);
      b.type = "button";
      b.setAttribute("aria-pressed", String(v === page));
      b.onclick = () => { page = v; pages.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x === b))); };
      pages.append(b);
    }
    const pageRow = el("div", "form-row");
    pageRow.append(el("span", "form-label", "頁面"), pages);
    if (PCOpen.desktop()) {
      const pick = el("button", "ghost fit", "選擇…");
      pick.prepend(icon("folder"));
      pick.onclick = async () => {
        try {
          const r = await api("/api/shell/dialog", { kind: "folder", start: folder.value || null });
          if (r.path) folder.value = r.path;
        } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
      };
      row.append(pick);
    }
    const go = el("button", "primary small", "建立");
    const create = async () => {
      if (!title.value.trim()) { title.focus(); return; }
      if (!folder.value.trim()) { folder.focus(); return; }
      go.disabled = true;
      try {
        const r = await api("/api/books", { title: title.value.trim(), folder: folder.value.trim(), page });
        m.hidden = true;
        books = null;
        await openPaper(r.id);
        LoreShell.toast("已建立");
      } catch (e) {
        LoreShell.toast(e.message, { err: true, ms: 6000 });
        go.disabled = false;
      }
    };
    go.onclick = create;
    for (const i of [title, folder]) i.onkeydown = (e) => { if ((e.key === "Enter" && !e.isComposing)) create(); if (e.key === "Escape") m.hidden = true; };
    const foot = el("div", "form-row end");
    foot.append(go);
    m.append(title, row, pageRow, foot);
    keepOnScreen(m);
    folder.scrollLeft = folder.scrollWidth; // the folder's own name, not the start of the path
    title.focus();
  }

  const baseName = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";

  /** Name, and the folder it sits in (a paper opened where it is). */
  function recentList(items) {
    const box = el("div", "open-recent");
    const ul = el("ul");
    for (const p of items) {
      const li = el("li");
      const b = el("button", "open-item");
      b.type = "button";
      b.dataset.paper = p.name;
      b.title = p.path ?? p.name;
      const where = p.path ? baseName(p.path.replace(/[\\/][^\\/]*[\\/]?$/, "")) : "";
      b.append(icon(p.kind === "book" ? "modeBook" : p.path ? "openLatex" : "modePaper"), el("span", "open-name", p.name), el("span", "open-where", where));
      b.onclick = () => openPaper(p.name);
      if (p.kind !== "book") b.oncontextmenu = (e) => paperContext(e, p);
      li.append(b);
      ul.append(li);
    }
    box.append(ul);
    return box;
  }

  function paperCard(p) {
    const art = el("div", "home-art page");
    if (p.thumb) {
      const img = el("img");
      img.alt = "";
      img.loading = "lazy";
      img.src = `/api/home/papers/${encodeURIComponent(p.name)}/thumb?t=${p.modified}`;
      img.onerror = () => img.replaceWith(fauxPage(p));
      art.append(img);
    } else art.append(fauxPage(p));
    const c = card(art, p.name, () => openPaper(p.name));
    c.dataset.paper = p.name;
    c.oncontextmenu = (e) => paperContext(e, p);
    // opened where it is: a folder mark, the place on hover
    if (p.path) {
      c.title = p.path;
      const f = el("span", "home-where");
      f.append(icon("folder"));
      art.append(f);
    }
    c.setAttribute("aria-current", String(p.name === state.paper?.name));
    if (p.phase) {
      const b = el("span", "home-badge", PHASES[p.phase] ?? p.phase);
      art.append(b);
    }
    const item = el("div", "home-item");
    // the paper's journal template, tidying and .bib (what the old paper menu offered)
    const more = iconButton("library", "整理與文獻", async () => { await openPaper(p.name); importCard(p.name); });
    more.classList.add("home-more");
    more.dataset.opensMenu = "";
    item.append(c, more);
    return item;
  }

  // ── 刪除論文 ──────────────────────────────────────────
  /** Right-click on a paper: 刪除論文… (workspace) or 從清單移除 (opened where it is). */
  function paperContext(e, p) {
    e.preventDefault();
    const at = { x: e.clientX, y: e.clientY };
    LoreShell.menu(at, [{ icon: "trash", label: p.path ? "從清單移除" : "刪除論文…", run: () => removePaper(p.name, at) }]);
  }

  /** Ask, then the server trashes a workspace paper or unlists one opened elsewhere (remove.rs). */
  async function removePaper(name, anchor) {
    let p = papers?.find((x) => x.name === name);
    if (!p) {
      try { papers = await api("/api/home/papers"); } catch {}
      p = papers?.find((x) => x.name === name) ?? { name };
    }
    if (p.kind === "book") return;
    const lines = p.path ? [`從清單移除「${name}」？`, "資料夾留在原處。"] : [`刪除「${name}」？`, "移到垃圾桶，可從那裡復原。"];
    const go = p.path ? "移除" : "刪除";
    if (!(await LoreShell.ask(anchor, lines, [{ label: "取消", value: null }, { label: go, value: true, danger: true, primary: true }]))) return;
    const open = state.paper?.name === name;
    if (open) await PCFrame.settle(); // the paragraph being edited lands before the folder goes
    let r;
    try { r = await api(`/api/papers/${encodeURIComponent(name)}/delete`, {}); }
    catch (err) { LoreShell.toast(err.message, { err: true, ms: 6000 }); return; }
    papers = (papers ?? []).filter((x) => x.name !== name);
    if (store.get("pc.paper", null) === name) store.set("pc.paper", null);
    if (open) {
      state.paper = null;
      PCFrame.setName("—", "");
      render(true);
      show(2, "paper");
    } else refresh();
    state.workspace = await api("/api/workspace").catch(() => state.workspace);
    if (typeof refreshGit === "function") refreshGit();
    LoreShell.toast(r.action === "trashed" ? "已移到垃圾桶" : "已從清單移除");
  }

  /** A page drawn from the title when the paper has not been compiled yet. */
  function fauxPage(p) {
    const f = el("div", "faux");
    f.append(el("div", "t", p.title || p.name));
    for (let i = 0; i < 9; i++) f.append(el("i"));
    return f;
  }

  // arrows move between cards; Esc goes back a level (or to the open paper)
  function onKey(e) {
    if (!isOpen() || document.querySelector("dialog[open]")) return;
    if (e.key === "Escape") {
      if (!$("#fileMenu").hidden) return; // the menu closes first
      if (level === 2) show(1);
      else if (state.paper) close();
      return;
    }
    const keys = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    if (!keys[e.key] || e.target.closest?.("input, textarea, select, #fileMenu")) return;
    const cards = [...root().querySelectorAll(".home-card:not(:disabled), .home-head button")];
    const i = cards.indexOf(document.activeElement);
    if (i < 0) { cards[0]?.focus(); e.preventDefault(); return; }
    const [dx, dy] = keys[e.key];
    const r = cards[i].getBoundingClientRect();
    const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    // nearest card in that direction
    let best = null, bd = Infinity;
    cards.forEach((c, j) => {
      if (j === i) return;
      const q = c.getBoundingClientRect();
      const x = q.left + q.width / 2 - cx, y = q.top + q.height / 2 - cy;
      if ((dx && Math.sign(x) !== dx) || (dy && Math.sign(y) !== dy) || (dx && Math.abs(y) > r.height / 2) ) return;
      const d = dx ? Math.abs(x) : Math.abs(y) * 4 + Math.abs(x);
      if (d < bd) { bd = d; best = c; }
    });
    if (best) { best.focus(); e.preventDefault(); }
  }

  function wire() {
    document.addEventListener("keydown", onKey);
  }

  /** Redraw level 2 after the papers changed (sync, import). */
  const refresh = () => { if (isOpen()) draw(false); };

  return { show, close, isOpen, refresh, wire, removePaper, MODES };
})();
