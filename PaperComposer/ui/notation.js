// 對照表: the notation registry (symbols, equations, abbreviations, labels) of
// the open paper, shown with the shared <notation-panel> (commons/ui). Rows
// are the author's global registry (shared with the other apps) plus this
// paper's own; the server splits what is saved back into the two files.
//
// Opening it from elsewhere (a warning that says "see 對照表"):
//   PCNotation.open()                       just open
//   PCNotation.open("\\vw")                 at the row of that macro / LaTeX
//   PCNotation.open("tab:main")             at the row with that label
//   PCNotation.open("CV")                   at that abbreviation
//   PCNotation.open({ id } | { latex } | { label } | { macro } | { tab: "checks" })
// or put data-notation="<same string>" on any element (clicking it opens),
// or dispatch document "pc:notation" with the target as detail.
const PCNotation = (() => {
  let built = false, panel, dlg, stateEl, timer = 0, scope = "document";

  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  };
  const base = () => `/api/papers/${encodeURIComponent(state.paper.name)}/notation`;

  /** The panel component and this screen's css load on first use. */
  function assets() {
    if (!document.querySelector("link[data-pcn]")) {
      const l = el("link");
      l.rel = "stylesheet";
      l.href = "notation.css";
      l.dataset.pcn = "1";
      document.head.append(l);
    }
    if (customElements.get("notation-panel")) return Promise.resolve();
    return new Promise((ok, no) => {
      const s = el("script");
      s.src = "/notation-panel.js";
      s.onload = ok;
      s.onerror = () => no(new Error("對照表元件載入失敗"));
      document.head.append(s);
    });
  }

  function build() {
    if (built) return;
    built = true;
    dlg = el("dialog", "dialog big");
    dlg.id = "dlgNotation";
    dlg.setAttribute("aria-label", "對照表");
    const box = el("div", "dlg");
    const head = el("div", "dlg-head");
    head.append(el("h2", "", "對照表"));
    stateEl = el("span", "chip");
    stateEl.hidden = true;
    const seg = el("div", "pcn-scope");
    seg.setAttribute("role", "group");
    seg.setAttribute("aria-label", "新增到");
    for (const [v, t] of [["document", "本篇"], ["global", "全域"]]) {
      const b = el("button", "", t);
      b.type = "button";
      b.dataset.v = v;
      b.setAttribute("aria-pressed", String(v === scope));
      b.onclick = () => {
        scope = v;
        seg.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x.dataset.v === scope)));
      };
      seg.append(b);
    }
    const close = el("button", "ico close");
    close.type = "button";
    close.dataset.icon = "x";
    close.setAttribute("aria-label", "關閉 Esc");
    head.append(stateEl, seg, close);
    const note = el("div", "pcn-note");
    note.id = "pcnMigration";
    note.hidden = true;
    panel = document.createElement("notation-panel");
    panel.setAttribute("mode", "macro");
    box.append(head, note, panel);
    dlg.append(box);
    document.body.append(dlg);
    paintIcons(dlg);
    close.onclick = () => dlg.close();
    dlg.addEventListener("click", (e) => { if (e.target === dlg) dlg.close(); });

    // a row added while "全域" is chosen goes to the global file
    panel.addEventListener("notation-add", (e) => { if (e.detail.entry && scope === "global") e.detail.entry.origin = "global"; });
    for (const ev of ["notation-change", "notation-add", "notation-delete"]) panel.addEventListener(ev, save);
    panel.addEventListener("notation-jump", jump);
  }

  function save() {
    clearTimeout(timer);
    stateEl.hidden = false;
    stateEl.textContent = "未儲存";
    timer = setTimeout(async () => {
      try {
        const r = await api(base(), panel.notation, "PUT");
        panel.report = r.report;
        panel.docMacros = r.macros ?? [];
        stateEl.textContent = "已儲存";
      } catch (e) {
        stateEl.textContent = "儲存失敗";
        flash(e.message, 5000);
      }
    }, 500);
  }

  /** A finding's place: open that paragraph of the paper. */
  function jump(e) {
    const { file, para } = e.detail ?? {};
    const secs = state.paper?.sections ?? [];
    const si = secs.findIndex((s) => file === s.file || file === `sections/${s.file}` || file?.endsWith(`/${s.file}`));
    if (si < 0) return flash("這個位置不在正文裡");
    const pi = Math.max(0, Math.min(para ?? 0, (secs[si].paragraphs?.length ?? 1) - 1));
    dlg.close();
    select(si, pi);
    paraEl(si, pi)?.scrollIntoView({ block: "center" });
  }

  function show(r) {
    panel.notation = r.notation;
    panel.report = r.report;
    panel.docMacros = r.macros ?? [];
    const note = $("#pcnMigration");
    note.replaceChildren();
    note.hidden = !r.migration?.clashes?.length;
    if (!note.hidden) {
      const ids = r.migration.clashes.map((c) => c.id).join("、");
      note.append(el("span", "", `舊的 profile/notation.json 有 ${r.migration.clashes.length} 筆與全域不同（${ids}），已保留全域的`));
      const ok = el("button", "ghost", "知道了");
      ok.type = "button";
      ok.onclick = async () => {
        note.hidden = true;
        try { await api(`${base()}/seen`, {}); } catch { /* shown again next time */ }
      };
      note.append(ok);
    }
  }

  /** The row `t` points at: id, label, macro (`\vw`), LaTeX, abbreviation. */
  function find(t) {
    if (!t) return null;
    if (typeof t === "object") {
      return (t.id && panel.entry(t.id)) || (t.label && find(String(t.label))) || (t.macro && find(`\\${String(t.macro).replace(/^\\/, "")}`)) || (t.latex && find(String(t.latex))) || null;
    }
    const s = String(t).trim();
    const rows = panel.notation.entries;
    return panel.entry(s) || rows.find((e) => e.label === s) || rows.find((e) => e.kind === "abbrev" && e.short === s) || panel.findByLatex(s) || null;
  }

  async function open(target) {
    if (!state.paper) return flash("先開啟一篇論文");
    try {
      await assets();
      build();
      show(await api(base()));
    } catch (e) {
      return flash(e.message, 5000);
    }
    stateEl.hidden = true;
    if (!dlg.open) dlg.showModal();
    if (target && typeof target === "object" && target.tab) panel.showTab(target.tab);
    const hit = find(target);
    if (hit) panel.select(hit.id);
    else if (target && typeof target === "string") flash(`對照表沒有 ${target}`);
  }

  function init() {
    const b = el("button", "ico");
    b.id = "actNotation";
    b.dataset.icon = "book";
    b.setAttribute("aria-label", "對照表");
    b.onclick = () => open();
    const anchor = document.querySelector("#actDraft");
    if (anchor) anchor.after(b);
    else document.querySelector(".paper-bar")?.append(b);
    paintIcons(b.parentElement);
    document.addEventListener("click", (e) => {
      const t = e.target.closest?.("[data-notation]");
      if (!t) return;
      e.preventDefault();
      open(t.dataset.notation);
    });
    document.addEventListener("pc:notation", (e) => open(e.detail));
  }
  init();

  return { open };
})();
