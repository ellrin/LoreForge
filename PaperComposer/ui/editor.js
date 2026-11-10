// Editing the paper's structure, and waiting on the model.
//
// PCGap: a thin ＋ line between paragraphs. Pointing at the lower edge of a
// paragraph (or of a section heading) shows it; a click opens a new, empty
// paragraph there. (A blank line typed in a paragraph also starts a new
// one: the server splits it, paper.rs render_section.)
//
// PCSections: add, rename, delete, move a section. In the navigator, each
// section has a ⋯ menu (also on right-click) and can be dragged; the
// heading on the page is renamed with a double-click. Deleting asks first
// and can be undone: the file is kept in sections/unused/ (sections.rs).
// A `\section{…}` typed into a paragraph becomes a section of its own.
//
// PCWait: elapsed seconds and 停止 for a request that can take a while
// (生成, 軟規則, 拆解). 停止 aborts the fetch; the server drops the work
// with the connection (tests/abort.rs).
//
// Uses app.js globals (state, sections, api, el, openPaper…) only
// inside functions, so it may load before app.js. Its styles are editor.css.

(() => {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = "editor.css";
  document.head.append(link);
})();

// ── Waiting ─────────────────────────────────────────────
const PCWait = (() => {
  /** A request that may be stopped: { signal, stop(), secs(), end() }; onTick(secs) every second. */
  function start(onTick) {
    const ctl = new AbortController();
    const t0 = Date.now();
    const secs = () => Math.floor((Date.now() - t0) / 1000);
    const timer = setInterval(() => onTick?.(secs()), 1000);
    onTick?.(0);
    return { signal: ctl.signal, stop: () => ctl.abort(), secs, end: () => clearInterval(timer), get stopped() { return ctl.signal.aborted; } };
  }
  const aborted = (e) => e?.name === "AbortError";
  const label = (what, s) => `${what} ${s} 秒`;
  /** A row "label N 秒 [停止]" for a popover or panel; returns { row, wait }. */
  function row(what) {
    const r = el("div", "wait-row");
    const t = el("span", "wait-t");
    const stop = el("button", "ghost small", "停止");
    r.append(t, stop);
    const wait = start((s) => { t.textContent = label(what, s); });
    stop.onclick = () => wait.stop();
    return { row: r, wait };
  }
  return { start, aborted, label, row };
})();

// ── Inserting a paragraph between two ───────────────────
const PCGap = (() => {
  let bar = null;
  let at = null; // { si, pi }: the new paragraph's place

  function make() {
    bar = el("button", "pc-gap");
    bar.type = "button";
    bar.title = "新增段落";
    bar.setAttribute("aria-label", "新增段落");
    bar.onmousedown = (e) => e.preventDefault();
    bar.onclick = () => { const a = at; hide(); if (a) insertParagraph(a.si, a.pi); };
    bar.onmouseleave = (e) => { if (!e.relatedTarget?.closest?.("#paper")) hide(); };
    document.body.append(bar);
  }
  function hide() { if (bar) bar.hidden = true; at = null; }

  /** The paragraph or heading whose lower edge is near y, and the new paragraph's place. */
  function target(x, y) {
    for (const dy of [0, -10]) {
      const n = document.elementFromPoint(x, y + dy)?.closest?.("#paper > .para, #paper > h2");
      if (!n || n.classList.contains("add-para")) continue;
      const r = n.getBoundingClientRect();
      if (y < r.bottom - 7 || y > r.bottom + 14) continue;
      if (n.tagName === "H2") {
        const si = +n.id.replace("sec-", "");
        if (!sections()[si]?.paragraphs.length) return null; // the empty section's own 雙擊新增段落
        return { si, pi: 0, r };
      }
      if (n.querySelector(":scope > textarea")) return null;
      return { si: +n.dataset.s, pi: +n.dataset.p + 1, r };
    }
    return null;
  }

  function move(e) {
    if (!state.paper || document.querySelector("#paper textarea") || e.buttons) return hide();
    if (bar && e.target === bar) return;
    const t = target(e.clientX, e.clientY);
    if (!t) return hide();
    if (!bar) make();
    const page = $("#paper").getBoundingClientRect();
    at = { si: t.si, pi: t.pi };
    Object.assign(bar.style, { left: `${page.left}px`, width: `${page.width}px`, top: `${t.r.bottom + 1}px` });
    bar.hidden = false;
  }

  function wire() {
    const paper = $("#paper");
    if (!paper || paper._gap) return;
    paper._gap = true;
    paper.addEventListener("mousemove", move);
    paper.addEventListener("mouseleave", (e) => { if (e.relatedTarget !== bar) hide(); });
    $("#pageArea")?.addEventListener("scroll", hide, { passive: true });
  }
  return { wire, hide };
})();

// ── Sections ────────────────────────────────────────────
const PCSections = (() => {
  const base = () => `/api/papers/${encodeURIComponent(state.paper.name)}`;
  let menu = null;

  /** Braces balanced, one line: a title main.tex can read. */
  function titleError(t) {
    if (!t.trim()) return "章節名稱不能空白";
    if (/\n/.test(t)) return "章節名稱只能一行";
    let d = 0;
    for (let i = 0; i < t.length; i++) {
      if (t[i] === "\\") { i++; continue; }
      if (t[i] === "{") d++;
      else if (t[i] === "}" && --d < 0) break;
    }
    return d === 0 ? null : "章節名稱的大括號沒有成對";
  }

  /** Read the paper again after a change to its files, then go to section `file`. */
  async function reload(stamp, file) {
    if (stamp) state.paper.stamp = stamp; // our own change, not one from elsewhere (open.js)
    await openPaper(state.paper.name, { keepHome: true });
    const si = sections().findIndex((s) => s.file === file);
    if (si >= 0) {
      select(si, 0);
      PCLazy.jump(document.getElementById(`sec-${si}`), { block: "start" });
    }
    refreshGit();
    return si;
  }

  function closeMenu() { menu?.remove(); menu = null; document.removeEventListener("mousedown", outside, true); document.removeEventListener("keydown", key, true); }
  const outside = (e) => { if (menu && !menu.contains(e.target)) closeMenu(); };
  const key = (e) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); closeMenu(); } };

  /** A small box under `anchor` in the app's menu look. */
  function popup(anchor, cls) {
    closeMenu();
    menu = el("div", `menu sx-menu ${cls ?? ""}`);
    document.body.append(menu);
    const place = () => {
      const r = anchor.getBoundingClientRect();
      menu.style.left = `${Math.max(8, Math.min(r.left, innerWidth - menu.offsetWidth - 8))}px`;
      menu.style.top = `${Math.max(8, Math.min(r.bottom + 4, innerHeight - menu.offsetHeight - 8))}px`;
    };
    setTimeout(() => { document.addEventListener("mousedown", outside, true); document.addEventListener("keydown", key, true); });
    return { box: menu, place };
  }

  /** ⋯ / right-click on a section in the navigator. */
  function openMenu(si, anchor) {
    const n = sections().length;
    // what follows opens under the section's line (the ⋯ is gone once the pointer leaves)
    const head = () => document.querySelector(`#nav .sec[data-s="${si}"] > .sec-head`) ?? anchor;
    const { box, place } = popup(anchor);
    const item = (label, fn, off) => {
      const b = el("button", off ? "off" : "", label);
      b.disabled = !!off;
      b.onclick = () => { closeMenu(); fn(); };
      box.append(b);
    };
    item("改名", () => rename(si));
    item("在後面新增章節", () => askTitle(si, head()));
    box.append(el("div", "sep"));
    item("上移", () => move(si, si - 1), si === 0);
    item("下移", () => move(si, si + 1), si === n - 1);
    box.append(el("div", "sep"));
    item("刪除章節…", () => askDelete(si, head()));
    place();
  }

  /** The title of a new section, after section `si` (-1: first). */
  function askTitle(si, anchor) {
    const { box, place } = popup(anchor, "form");
    const input = el("input", "field");
    input.placeholder = "章節名稱";
    const err = el("div", "sx-err");
    const go = el("button", "primary small", "新增");
    const row = el("div", "sx-row");
    row.append(input, go);
    box.append(row, err);
    place();
    input.focus();
    const submit = async () => {
      const t = input.value.trim();
      const bad = titleError(t);
      if (bad) { err.textContent = bad; input.focus(); return; }
      go.disabled = true;
      try {
        const r = await api(`${base()}/section-ops/add`, { title: t, after: sections()[si]?.file ?? null });
        closeMenu();
        await reload(r.stamp, r.file);
        LoreShell.toast(`已新增「${t}」`);
      } catch (e) { err.textContent = e.message; go.disabled = false; }
    };
    go.onclick = submit;
    input.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); submit(); } };
  }

  /** Labels of a section and the places elsewhere that refer to them. */
  function refsTo(si) {
    const sec = sections()[si];
    const labels = [...`${sec.tail ?? ""}\n${sec.paragraphs.join("\n")}`.matchAll(/\\label\{([^}]*)\}/g)].map((m) => m[1]);
    if (!labels.length) return 0;
    const re = new RegExp(`\\\\(?:ref|eqref|autoref|cref|Cref|pageref|nameref)\\{[^}]*\\b(?:${labels.map((l) => l.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})\\b`, "g");
    const others = [state.paper.front, ...sections().filter((_, i) => i !== si).flatMap((s) => s.paragraphs)].join("\n");
    return (others.match(re) ?? []).length;
  }

  function askDelete(si, anchor) {
    const sec = sections()[si];
    const { box, place } = popup(anchor, "sx-ask");
    box.setAttribute("role", "alertdialog");
    const words = sec.paragraphs.length;
    const refs = refsTo(si);
    const msg = el("div", "sx-msg");
    msg.append(el("div", "", `刪除「${PCTex.stripComments(sec.title)}」？`));
    if (words) msg.append(el("div", "sx-sub", `${words} 段`));
    if (refs) msg.append(el("div", "sx-sub bad", `其他地方有 ${refs} 處引用這一章`));
    const acts = el("div", "sx-acts");
    const no = el("button", "ghost", "取消"), yes = el("button", "primary small danger", "刪除");
    acts.append(no, yes);
    box.append(msg, acts);
    place();
    no.onclick = closeMenu;
    yes.onclick = async () => {
      yes.disabled = true;
      try {
        const r = await api(`${base()}/section-ops/delete`, { file: sec.file });
        closeMenu();
        await reload(r.stamp, r.after);
        undoToast(`已刪除「${PCTex.stripComments(sec.title)}」`, async () => {
          const back = await api(`${base()}/section-ops/restore`, r); // what the delete said, as it is
          await reload(back.stamp, back.file);
        });
      } catch (e) { closeMenu(); LoreShell.toast(`刪除失敗：${e.message}`, { err: true, ms: 6000 }); }
    };
  }

  /** A note with 復原, gone after a while. */
  function undoToast(text, undo) {
    document.querySelector(".sx-toast")?.remove();
    const t = el("div", "tip sx-toast");
    const b = el("button", "ghost small", "復原");
    t.append(el("span", "", text), b);
    document.body.append(t);
    const timer = setTimeout(() => t.remove(), 12000);
    b.onclick = async () => {
      clearTimeout(timer);
      t.remove();
      try { await undo(); LoreShell.toast("已復原"); } catch (e) { LoreShell.toast(`無法復原：${e.message}`, { err: true, ms: 6000 }); }
    };
  }

  /** Move section `si` to place `to`. */
  async function move(si, to) {
    const files = sections().map((s) => s.file);
    if (to < 0 || to >= files.length || to === si) return;
    const [f] = files.splice(si, 1);
    files.splice(to, 0, f);
    try {
      const r = await api(`${base()}/section-ops/order`, { files });
      await reload(r.stamp, f);
    } catch (e) { LoreShell.toast(`無法移動：${e.message}`, { err: true, ms: 6000 }); }
  }

  /** Rename section `si` on its heading on the page: Enter / outside saves, Esc cancels. */
  function rename(si) {
    const h = document.getElementById(`sec-${si}`);
    const sec = sections()[si];
    if (!h || !sec || h.querySelector("input")) return;
    PCLazy.jump(h, { block: "nearest" });
    const was = sec.title;
    const input = el("input", "sx-title");
    input.value = was;
    h.replaceChildren(input);
    const draw = () => { h.replaceChildren(PCTex.render(sections()[si]?.title ?? was)); };
    let done = false;
    const finish = async (save) => {
      if (done) return;
      const v = input.value.trim();
      const bad = save && v !== was.trim() ? titleError(v) : null;
      if (bad) { LoreShell.toast(bad, { ms: 4000 }); input.focus(); return; }
      done = true;
      if (!save || v === was.trim()) { draw(); return; }
      sec.title = v;
      try { await saveSection(si); } catch (e) { sec.title = was; LoreShell.toast(`儲存失敗：${e.message}`, { err: true, ms: 6000 }); }
      draw();
      renderNavSection(si);
      markCurrent();
      renderStatus();
      refreshGit();
    };
    input.onkeydown = (e) => {
      if (e.isComposing) return;
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); finish(false); }
      else if (e.key === "Enter") { e.preventDefault(); finish(true); }
    };
    input.onblur = () => finish(true);
    input.focus();
    input.select();
  }

  // Dragging a section in the navigator.
  let dragging = null;
  function draggable(li, si) {
    const head = li.querySelector(":scope > .sec-head");
    if (!head) return;
    head.draggable = true;
    head.ondragstart = (e) => { dragging = si; e.dataTransfer.effectAllowed = "move"; e.dataTransfer.setData("text/plain", String(si)); li.classList.add("sx-dragging"); };
    head.ondragend = () => { dragging = null; li.classList.remove("sx-dragging"); document.querySelectorAll(".sx-drop-before, .sx-drop-after").forEach((n) => n.classList.remove("sx-drop-before", "sx-drop-after")); };
    head.ondragover = (e) => {
      if (dragging == null || dragging === si) return;
      e.preventDefault();
      const r = head.getBoundingClientRect();
      const after = e.clientY > r.top + r.height / 2;
      li.classList.toggle("sx-drop-after", after);
      li.classList.toggle("sx-drop-before", !after);
    };
    head.ondragleave = () => li.classList.remove("sx-drop-before", "sx-drop-after");
    head.ondrop = (e) => {
      e.preventDefault();
      const from = dragging;
      const after = li.classList.contains("sx-drop-after");
      li.classList.remove("sx-drop-before", "sx-drop-after");
      if (from == null) return;
      let to = si + (after ? 1 : 0);
      if (from < to) to--;
      move(from, to);
    };
  }

  /** Called by app.js for each section of the navigator. */
  function decorate(li, si) {
    const more = el("button", "sx-more", "⋯");
    more.title = "章節";
    more.setAttribute("aria-label", "章節");
    more.onclick = (e) => { e.stopPropagation(); openMenu(+li.dataset.s, more); };
    li.querySelector(":scope > .sec-head")?.addEventListener("contextmenu", (e) => { e.preventDefault(); openMenu(+li.dataset.s, more); });
    li.prepend(more);
    draggable(li, si);
  }

  /** The navigator's last line: ＋ 新增章節. */
  function addRow() {
    const li = el("li", "sx-add");
    const b = el("button", "", "＋ 新增章節");
    b.onclick = () => askTitle(sections().length - 1, b);
    li.append(b);
    return li;
  }

  /** After a save: a `\section{…}` typed in section `si` becomes a section of its own. */
  async function splitTyped(si) {
    const sec = sections()[si];
    if (!sec) return false;
    try {
      const r = await api(`${base()}/section-ops/split`, { file: sec.file });
      if (!r.files?.length) return false;
      const at = await reload(r.stamp, r.files[0]);
      LoreShell.toast(r.files.length > 1 ? `已分出 ${r.files.length} 個章節` : `已新增章節「${PCTex.stripComments(sections()[at]?.title ?? "")}」`);
      return true;
    } catch (e) { LoreShell.toast(`無法分出章節：${e.message}`, { err: true, ms: 6000 }); return false; }
  }

  return { decorate, addRow, rename, splitTyped, openMenu, titleError };
})();
