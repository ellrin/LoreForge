// Export: compile the paper to PDF with the journal's class, or convert it to
// Word. The result opens in a small panel under the export button: pages and
// time, or the errors with a jump to the paragraph each one is in.

const PCExport = (() => {
  const KIND = { pdf: "PDF", docx: "Word", latex: "LaTeX" };
  let busy = false;
  let lastKind = "pdf";

  const paperUrl = () => `/api/papers/${encodeURIComponent(state.paper.name)}`;

  async function run(kind = "pdf") {
    if (!state.paper || busy) return;
    // A paragraph being edited saves on blur; let that land first.
    const ta = document.querySelector(".para textarea");
    if (ta) { ta.blur(); await new Promise((r) => setTimeout(r, 400)); }
    busy = true;
    lastKind = kind;
    show(kind, null);
    let r;
    try { r = await api(`${paperUrl()}/export`, { kind }); }
    catch (e) { r = { ok: false, errors: [{ message: e.message }] }; }
    busy = false;
    show(kind, r);
    // A compile knows every number: refresh labels and citations on the page.
    if (kind !== "docx" && r.ok) {
      try {
        const res = await api(paperUrl());
        PCTex.setNumbers(res.numbers);
        state.paper.bibliography = res.bibliography ?? [];
        render();
      } catch { /* keep the page as is */ }
    }
  }

  async function open(kind, reveal) {
    try { await api(`${paperUrl()}/export/open`, { kind, reveal }); }
    catch (e) { flash(e.message); }
  }

  // Where an error is: the section's title and paragraph, or the file name.
  function place(e) {
    const file = e.file?.split("/").pop();
    const si = sections().findIndex((s) => s.file === file);
    const name = si >= 0 ? sections()[si].title : (e.file ?? "");
    const at = [si >= 0 && e.para != null ? `¶${e.para + 1}` : "", e.line ? `第 ${e.line} 行` : ""].filter(Boolean).join(" · ");
    if (!name && !at) return null;
    const b = el("button", "ex-at");
    b.append(el("span", "ex-sec", name), el("span", "", at));
    if (si >= 0) {
      b.onclick = () => {
        close();
        const pi = e.para ?? 0;
        select(si, pi);
        (paraEl(si, pi) ?? document.getElementById(`sec-${si}`))?.scrollIntoView({ block: "center", behavior: "smooth" });
      };
    } else b.disabled = true;
    return b;
  }

  function item(cls, text, extra = []) {
    const li = el("li", cls);
    li.append(el("span", "ex-msg", text), ...extra.filter(Boolean));
    return li;
  }

  function show(kind, r) {
    const p = $("#exportPop");
    p.innerHTML = "";
    const head = el("div", "ex-head");
    head.append(el("b", "", KIND[kind]));
    if (!r) {
      head.append(el("span", "ex-state busy", kind === "docx" ? "轉換中…" : "編譯中…"));
    } else if (r.ok) {
      const facts = [r.pages ? `${r.pages} 頁` : "", `${r.seconds} 秒`].filter(Boolean).join(" · ");
      const go = el("button", "primary small", "開啟");
      go.onclick = () => { close(); open(kind, kind === "latex"); };
      head.append(el("span", "ex-state ok", facts), go, iconButton("folder", "在資料夾中顯示", () => open(kind, true)));
    } else {
      head.append(el("span", "ex-state bad", "失敗"));
    }
    p.append(head);

    if (r) {
      const ul = el("ul", "ex-list");
      (r.errors ?? []).forEach((e) => {
        if (e.message === "pandoc-missing") {
          const cmd = navigator.platform.startsWith("Mac") ? "brew install pandoc" : "pandoc.org/installing";
          ul.append(item("err", "需要 Pandoc", [el("code", "ex-hint", cmd)]));
          return;
        }
        ul.append(item("err", e.message, [e.context && el("code", "ex-ctx", e.context), e.hint && el("span", "ex-hint", e.hint), place(e)]));
      });
      if (r.undefined_cites?.length) ul.append(item("warn", "找不到的引用", [el("code", "ex-ctx", r.undefined_cites.join(", "))]));
      if (r.undefined_refs?.length) ul.append(item("warn", "找不到的標籤", [el("code", "ex-ctx", r.undefined_refs.join(", "))]));
      if (r.overfull) ul.append(item("warn", `${r.overfull} 行超出版面`));
      (r.warnings ?? []).forEach((w) => ul.append(item("warn", w)));
      if (ul.children.length) p.append(ul);
    }

    p.hidden = false;
    const b = $("#actExport").getBoundingClientRect();
    p.style.top = `${b.bottom + 6}px`;
    p.style.left = `${Math.max(8, Math.min(b.left, innerWidth - p.offsetWidth - 8))}px`;
  }

  function close() { $("#exportPop").hidden = true; }

  return {
    run, close,
    reveal: () => open(lastKind, true),
    get busy() { return busy; },
  };
})();
