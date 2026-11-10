// Start screen (封面). Level 1: what to do, one large card per mode (MODES:
// a new mode is one entry), then 最近 (one list, both kinds of papers). At launch the
// paper that was open last opens again. Level 2 (寫論文): new paper, import, and the
// workspace's papers to continue, most recently edited first.
// The paper button in the top bar comes back here.

const PCHome = (() => {
  // 討論草稿 phases (session.json `phase`) as draft.js names its steps
  const PHASES = { topic: "主題", record: "實驗記錄", plan: "題組", interview: "對話", claims: "主張", review: "校正", outline: "大綱", draft: "草稿", audit: "審稿", revise: "修訂" };

  const MODES = [
    { id: "paper", icon: "modePaper", label: "寫論文", enabled: true, onOpen: () => show(2) },
    { id: "book", icon: "modeBook", label: "寫書", enabled: false },
    { id: "latex", icon: "openLatex", label: "開啟 LaTeX", enabled: true, onOpen: (card) => PCOpen.latex(card) },
    { id: "word", icon: "openWord", label: "開啟 Word", enabled: true, onOpen: (card) => PCOpen.word(card) },
  ];

  let level = 1;
  let papers = null; // last /api/home/papers
  const root = () => $("#home");
  const isOpen = () => !root().hidden;

  function show(n = level) {
    level = n;
    root().hidden = false;
    document.querySelector(".app").classList.add("at-home");
    $("#actFile").setAttribute("aria-expanded", "true");
    $("#fileMenu").hidden = true;
    draw(true);
    reopenLast();
  }

  function close() {
    if (!isOpen()) return;
    root().hidden = true;
    document.querySelector(".app").classList.remove("at-home");
    $("#actFile").setAttribute("aria-expanded", "false");
    $("#fileMenu").hidden = true;
  }

  /** The top-bar paper button: open this screen, or go back to the open paper. */
  function toggle() {
    if (isOpen() && state.paper) close();
    else show(isOpen() ? 1 : level);
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
      sub.hidden = !papers?.length;
      grid.replaceChildren(...(papers ?? []).map(paperCard));
      if (had) grid.querySelector(`[data-paper="${CSS.escape(had)}"]`)?.focus();
    };
    if (papers) fill();
    try { papers = await api("/api/home/papers"); }
    catch (e) { papers = papers ?? []; flash(e.message, 6000); }
    if (grid.isConnected) fill(); // unless the screen was redrawn or left meanwhile
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
      b.append(icon(p.path ? "openLatex" : "modePaper"), el("span", "open-name", p.name), el("span", "open-where", where));
      b.onclick = () => openPaper(p.name);
      li.append(b);
      ul.append(li);
    }
    box.append(ul);
    return box;
  }

  /** At launch: back to the paper that was open last, if it is still there. */
  let launched = false;
  async function reopenLast() {
    if (launched) return;
    launched = true;
    const last = store.get("pc.paper", null);
    if (!last || state.paper || new URLSearchParams(location.search).has("open")) return;
    try {
      papers = await api("/api/home/papers");
      if (!state.paper && papers.some((p) => p.name === last)) await openPaper(last);
    } catch {}
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

  return { show, close, toggle, isOpen, refresh, wire, MODES };
})();
