// The frame: PaperComposer in the shared Lore shell (commons/ui/shell.js, docs/shell.md).
// Top bar: [導覽] brand · 文件 ▾ (最近 · 開啟 LaTeX… · 開啟 Word… · 新論文 · 另存新檔… · 首頁)
// · one status chip (存檔 and git sync) · 寫作｜審閱 · 本篇文獻 資料庫 模板 · 討論草稿 (AI, last)
// … 對照表 · 匯出 ▾ (PDF Word LaTeX 列印) · 齒輪. ⌘S saves (the paragraph being edited,
// the database) and syncs in the background; ⌘F searches; ⌘, opens the gear.
// ≤1100px the navigator and the right panel are the shell's drawers.

const PCFrame = (() => {
  let shell = null;
  const picker = () => shell?.el.querySelector(".picker");

  /** The workspace's papers and the folders opened where they are, newest first. */
  async function recent() {
    let papers = [];
    try { papers = await api("/api/home/papers"); } catch {}
    if (!papers.length) return [];
    return [{ head: "最近" }, ...papers.slice(0, 6).map((p) => ({
      icon: p.path ? "openLatex" : "modePaper", label: p.name, title: p.path ?? p.name,
      current: p.name === state.paper?.name, run: () => openPaper(p.name),
    }))];
  }

  /** 另存新檔…: a copy beside the original (shell.rs), then the copy is open. */
  function saveAs() {
    if (!state.paper) return;
    const m = menuForm("另存新檔", null, picker());
    const name = el("input", "field");
    name.value = `${state.paper.name} copy`;
    const go = el("button", "primary small", "儲存");
    const run = async () => {
      const n = name.value.trim();
      if (!n) return name.focus();
      go.disabled = true;
      await settle();
      try {
        const r = await api(`/api/papers/${encodeURIComponent(state.paper.name)}/save-as`, { name: n });
        m.hidden = true;
        state.workspace = await api("/api/workspace").catch(() => state.workspace);
        await openPaper(r.id);
        refreshGit();
        LoreShell.toast(`已另存為 ${r.id}`);
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); go.disabled = false; }
    };
    go.onclick = run;
    name.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing) run(); };
    m.append(name, go);
    keepOnScreen(m);
    name.select();
  }

  /** A paragraph being edited saves on blur: let it land. */
  async function settle() {
    const ta = document.querySelector("#paper .para > textarea, #paper .field-edit");
    if (!ta) return;
    // its onblur saves; a window without focus fires no blur, so run it directly
    if (document.activeElement === ta && document.hasFocus()) ta.blur(); else ta.onblur?.();
    for (let t = 0; t < 80 && document.querySelector("#paper .para > textarea, #paper .field-edit"); t++) await new Promise((ok) => setTimeout(ok, 50));
    while (state.saving) await new Promise((ok) => setTimeout(ok, 50));
  }

  /** ⌘S: the paragraph being edited and the database are saved; git syncs in the background. */
  async function save() {
    await settle();
    if (state.db.dirty.size) await saveDb();
    if (state.git?.repo) syncNow();
    else status();
  }

  /** The one chip: 儲存中 / 未儲存 / 未能儲存 / 同步中 / 未同步 / 已儲存. */
  function status() {
    if (!shell) return;
    const g = state.git;
    const ta = document.querySelector("#paper .para > textarea");
    const editing = ta && ta.value.trim() !== (ta._was ?? "").trim();
    if (state.saveError) return shell.setStatus("error", { title: state.saveError, action: save });
    if (state.saving) return shell.setStatus("saving");
    if (editing || state.db.dirty.size) return shell.setStatus("dirty", { title: "⌘S", action: save });
    if (state.syncing) return shell.setStatus("syncing");
    if (g?.repo && (g.changed.length || g.ahead)) {
      const n = g.changed.length ? `${g.changed.length} 項` : `${g.ahead} 個 commit`;
      return shell.setStatus("unsynced", { title: `${n}未同步 ⌘S`, action: syncNow });
    }
    const title = !g?.repo ? "只存在這台電腦" : !g.remote ? "沒有遠端" : state.lastSync ? `已同步 ${state.lastSync}` : "已同步";
    shell.setStatus("saved", { title });
  }

  /** 列印: the compiled PDF in the system's print dialog (compiled first when needed). */
  async function print() {
    if (!state.paper) return;
    const url = () => `/api/papers/${encodeURIComponent(state.paper.name)}/export/file?kind=pdf&t=${Date.now()}`;
    let r = await fetch(url(), { method: "HEAD" }).catch(() => null);
    if (!r?.ok) {
      const ok = await PCExport.run("pdf");
      if (!ok) return;
    }
    PCExport.close();
    document.querySelector("#printFrame")?.remove();
    const f = el("iframe");
    f.id = "printFrame";
    f.title = "列印";
    f.style.cssText = "position:fixed;right:0;bottom:0;width:0;height:0;border:0;visibility:hidden";
    f.src = url();
    f.onload = () => {
      try { f.contentWindow.focus(); f.contentWindow.print(); }
      catch { window.open(f.src, "_blank", "noopener"); }
    };
    document.body.append(f);
  }

  const DB_TABLES = () => [
    ...TABLES.map(([name, label]) => ({ label, current: dbOpen() && state.db.name === name, run: () => openDbTab(name) })),
    { label: "文獻庫", current: dbOpen() && state.db.name === "@refs", run: () => openDbTab("@refs") },
    "-",
    { label: "拆解好句子…", run: () => { openDbTab(state.db.name === "@refs" ? TABLES[0][0] : state.db.name); setTimeout(() => $("#dcText").focus(), 50); } },
  ];

  function mount() {
    shell = LoreShell.mount({
      app: "pc",
      resize: { left: "aside.left", right: "aside.right", defaults: { left: 248, right: 300 } },
      brand: "PAPER/COMPOSER",
      host: "header.top",
      home: () => PCHome.show(1),
      picker: {
        icon: "file",
        name: "",
        recent,
        actions: [
          { icon: "openLatex", label: "開啟 LaTeX…", run: () => PCOpen.latex(picker()) },
          { icon: "openWord", label: "開啟 Word…", run: () => PCOpen.word(picker()) },
          { icon: "plus", label: "新論文", run: () => PCNew.open() },
          { icon: "save", label: "另存新檔…", run: saveAs, get disabled() { return !state.paper; } },
        ],
      },
      primary: [
        { id: "actRefs", icon: "quote", label: "本篇文獻", run: () => openWindow("dlgPaperRefs") },
        { id: "actDb", icon: "database", label: "資料庫", menu: DB_TABLES },
        { id: "actTpl", icon: "layout", label: "模板", run: () => openWindow("dlgTpl") },
        { id: "actNotes", icon: "notebook", label: "筆記", run: () => PCNotes.open() },
      ],
      ai: { id: "actDraft", icon: "spark", label: "討論草稿", run: () => PCDraft.open() },
      notation: () => PCNotation.open(),
      export: [
        { icon: "pdf", label: "PDF", kbd: "⌘P", run: () => PCExport.run("pdf") },
        { icon: "file", label: "Word", run: () => PCExport.run("docx") },
        { icon: "code", label: "LaTeX（zip）", run: () => PCExport.run("latex") },
        "-",
        { icon: "printer", label: "列印", run: print },
        { icon: "folder", label: "在資料夾中顯示", run: () => PCExport.reveal() },
      ],
      drawers: { left: "aside.left", leftLabel: "導覽", leftIcon: "panelLeft", right: "aside.right", rightLabel: "面板" },
      search: () => { if (!document.querySelector("dialog[open]")) PCFind.open(false); },
      save,
      keys: [
        { label: "取代", keys: navigator.platform.startsWith("Mac") ? "⌘⌥F" : "Ctrl+H" },
        { label: "匯出 PDF", keys: "⌘P" },
        { label: "生成", keys: "⌘↵" },
      ],
      ...Setup.shellParts(),
    });
    shell.slots.lead.append($("#modeSwitch"));
    // the paper menu's forms (開啟 LaTeX…, 另存新檔…) hang under the picker
    picker().id = "actFile";
    // 做成簡報 (commons/ui/makedeck.js): SlideComposer reads the paper's folder
    LoreMakeDeck.mount({
      before: "header.top .slot-primary #actDraft",
      key: "pc.todeck",
      source: async ({ check }) => {
        if (typeof state === "undefined" || !state.paper) return null;
        if (check) return true;
        const src = await (await fetch(`/api/papers/${encodeURIComponent(state.paper.name)}/todeck`)).json();
        if (!src.path) throw new Error(src.error ?? "找不到論文資料夾");
        return { kind: "pc", path: src.path };
      },
    });
    wireDrawers();
    return shell;
  }

  // drawers: Esc or a click on the page closes them, picking a heading closes the navigator
  function wireDrawers() {
    const narrow = matchMedia("(max-width: 1100px)");
    const open = (w) => document.querySelector(`.drawer-${w}`)?.classList.contains("open");
    document.addEventListener("keydown", (e) => {
      if (e.key !== "Escape" || !narrow.matches || document.querySelector("dialog[open]")) return;
      if (open("left") || open("right")) { shell.drawer("left", false); shell.drawer("right", false); }
    });
    $("#pageArea").addEventListener("pointerdown", () => { if (narrow.matches) { shell.drawer("left", false); shell.drawer("right", false); } });
    document.querySelector("aside.left").addEventListener("click", (e) => {
      if (narrow.matches && e.target.closest(".outline button[data-s][data-p]")) shell.drawer("left", false);
    });
    // a folded block: a click anywhere on its head (not its ? help) opens it
    document.querySelector("aside.right").addEventListener("click", (e) => {
      const head = e.target.closest(".block[data-open='false'] > .block-head");
      if (!head || e.target.closest(".help, .toggle")) return;
      head.querySelector(".toggle")?.click();
    });
  }

  /** The start screen: brand and gear only. */
  function home(on) {
    if (!shell) return;
    shell.home(on);
    if (on) { shell.drawer("left", false); shell.drawer("right", false); }
  }

  return {
    mount, status, home, save, print, settle,
    setName: (n, title) => { shell?.setName(n); if (title != null && picker()) picker().title = title; },
    get shell() { return shell; },
  };
})();
