// 筆記: the workspace's notes/ folder (crates/server/src/notes.rs, commons/crates/lore-notes).
// Notes and 靈感 (from LoreEcho or written here) in one window: list, read (rendered like the
// paper's paragraphs, KaTeX for math), edit as LaTeX with a live preview, delete to .trash/.
// Every save runs the 對照表 checks on the server; a blocking finding keeps the text here and
// shows why. New files (LoreEcho writes them while this runs) appear on the next poll.

window.PCNotes = (() => {
  const POLL_MS = 3000;
  const SOURCES = { loreecho: "學習夥伴", manual: "手寫" };
  const st = { notes: [], kind: "note", q: "", sel: null, sig: "", edit: null, findings: {}, timer: null, macros: null };
  let dlg = null;

  const h = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e[k] = v;
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null) e.append(c);
    return e;
  };
  const icon = (name) => h("span", { "data-icon": name });
  const paint = (root) => window.paintIcons?.(root);
  const q = (sel) => dlg.querySelector(sel);

  // the window's own sheet (one <script> line in index.html loads this file)
  if (!document.querySelector('link[href="notes.css"]')) document.head.append(h("link", { rel: "stylesheet", href: "notes.css" }));

  function build() {
    dlg = h("dialog", { class: "dialog big notes-dlg", id: "dlgNotes", "aria-label": "筆記" },
      h("div", { class: "dlg" },
        h("div", { class: "dlg-head" },
          h("h2", {}, "筆記"),
          h("div", { class: "seg", id: "notesKind", role: "group", "aria-label": "類別" },
            h("button", { "data-v": "note", "aria-pressed": "true" }, "筆記 ", h("span", { class: "count" })),
            h("button", { "data-v": "idea", "aria-pressed": "false" }, "靈感 ", h("span", { class: "count" }))),
          h("input", { type: "search", id: "notesQ", placeholder: "搜尋", "aria-label": "搜尋筆記" }),
          h("button", { class: "btn small", id: "notesNew", "aria-label": "新筆記" }, icon("plus"), "新筆記"),
          h("button", { class: "ico close", "data-icon": "x", "aria-label": "關閉 Esc" })),
        h("div", { class: "notes-body" },
          h("ol", { class: "notes-list", id: "notesList", "aria-label": "清單" }),
          h("section", { class: "notes-main", id: "notesMain" }))));
    document.body.append(dlg);
    paint(dlg);
    q("#notesKind").onclick = (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      st.kind = b.dataset.v;
      st.sel = null;
      renderList();
    };
    q("#notesQ").oninput = (e) => { st.q = e.target.value; renderList(); };
    q("#notesNew").onclick = () => leaveEdit().then((ok) => ok && edit(null));
    q(".close").onclick = () => close();
    dlg.addEventListener("cancel", (e) => { e.preventDefault(); close(); });
    dlg.addEventListener("close", () => { stopPoll(); if (paperMacros) setMacros(paperMacros); });
  }

  // ── data ──────────────────────────────────────────────
  async function refresh(force = false) {
    let res;
    try {
      res = await fetch("/api/notes");
      if (!res.ok) throw new Error(await res.text());
    } catch (e) {
      if (force) LoreShell.toast(`筆記：${e.message}`, { err: true });
      return;
    }
    const { notes } = await res.json();
    const sig = notes.map((n) => `${n.id}:${n.modified}`).join("|");
    if (sig === st.sig && !force) return;
    st.sig = sig;
    st.notes = notes;
    renderList();
  }

  // KaTeX macros: the open paper's table (app.js sets it) plus the global registry's, which a note
  // may use when the paper does not; while this window is open only, the paper keeps its own.
  let paperMacros = null;
  const setMacros = PCTex.setMacros;
  PCTex.setMacros = (m) => { paperMacros = m; setMacros(m); };
  async function macros() {
    try { st.macros = (await (await fetch("/api/notes/macros")).json()).macros ?? {}; } catch { st.macros ??= {}; }
    setMacros({ ...st.macros, ...(paperMacros ?? {}) });
  }

  function startPoll() {
    stopPoll();
    st.timer = setInterval(() => { if (!document.hidden) refresh(); }, POLL_MS);
  }
  function stopPoll() { clearInterval(st.timer); st.timer = null; }
  document.addEventListener("visibilitychange", () => { if (!document.hidden && dlg?.open) refresh(); });
  window.addEventListener("focus", () => { if (dlg?.open) refresh(); });

  // ── list ──────────────────────────────────────────────
  const matches = (n, words) => {
    const hay = `${n.title}\n${n.tags.join(" ")}\n${n.body}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  };
  const shown = () => {
    const words = st.q.toLowerCase().split(/\s+/).filter(Boolean);
    return st.notes.filter((n) => n.kind === st.kind && matches(n, words));
  };
  const when = (iso) => (iso ? iso.slice(0, 16).replace("T", " ") : "");
  const badge = (src) => h("span", { class: "notes-src", "data-src": src || "manual" }, SOURCES[src || "manual"] ?? src);

  function renderList() {
    for (const b of q("#notesKind").querySelectorAll("button")) {
      b.setAttribute("aria-pressed", String(b.dataset.v === st.kind));
      b.querySelector(".count").textContent = st.notes.filter((n) => n.kind === b.dataset.v).length;
    }
    const list = q("#notesList");
    const rows = shown();
    if (st.sel && !st.notes.some((n) => n.id === st.sel)) st.sel = null;
    if (!st.sel && rows.length && !st.edit) st.sel = rows[0].id;
    list.replaceChildren(...rows.map((n) => h("li", {},
      h("button", { class: "notes-row", "aria-current": n.id === st.sel ? "true" : null, onclick: () => pick(n.id) },
        h("span", { class: "t" }, titleNode(n)),
        h("span", { class: "m" }, badge(n.source), h("span", { class: "d" }, when(n.created)))))));
    if (!rows.length) list.append(h("li", { class: "notes-empty" }, st.q ? "沒有符合的" : st.kind === "idea" ? "沒有靈感" : "沒有筆記"));
    if (!st.edit) renderRead();
  }

  async function pick(id) {
    if (!(await leaveEdit())) return;
    st.sel = id;
    renderList();
  }

  // ── read ──────────────────────────────────────────────
  /** LaTeX body → the paper's paragraph look (blank lines split paragraphs). */
  function rendered(body) {
    const box = h("div", { class: "notes-paper" });
    PCTex.resetCounters(); // a note numbers its equations from (1); the paper resets before its own
    const chunks = body.split(/\n[ \t]*\n/).map((c) => c.trim()).filter(Boolean);
    for (const c of chunks) {
      const p = h("div", { class: "notes-para" });
      p.append(PCTex.render(c));
      box.append(p);
    }
    PCTex.hydrate?.(box);
    return box;
  }

  /** Titles may hold inline math ($k$-fold): drawn like the text. */
  function titleNode(n) {
    const span = h("span", {});
    span.append(PCTex.render(n.title || n.id));
    PCTex.hydrate?.(span);
    return span;
  }

  /** Findings: the blocking ones in full; the rest (or, when blocked, their count) shortened. */
  function findingsList(list, blocked = false) {
    const shown = (list ?? []).filter((f) => f.severity !== "info");
    if (!shown.length) return null;
    const stop = shown.filter((f) => f.blocking);
    const rest = shown.filter((f) => !f.blocking);
    const MAX = 3;
    const rows = blocked ? stop : rest.slice(0, MAX);
    const more = blocked ? rest.length : rest.length - rows.length;
    return h("ul", { class: `notes-findings${blocked ? " blocked" : ""}` },
      rows.map((f) => h("li", { class: f.blocking ? "block" : f.severity },
        h("span", { class: "msg" }, f.message),
        f.line ? h("span", { class: "ln" }, `第 ${f.line} 行`) : null)),
      more > 0 ? h("li", { class: "more" }, `另有 ${more} 項提醒`) : null);
  }

  function renderRead() {
    const main = q("#notesMain");
    const n = st.notes.find((x) => x.id === st.sel);
    if (!n) { main.replaceChildren(); return; }
    const more = h("button", { class: "ico small", "data-icon": "more", "aria-label": "更多" });
    more.onclick = () => LoreShell.menu(more, [
      { icon: "insert", label: "插入到論文", disabled: true, title: "之後提供" },
      "-",
      { icon: "trash", label: "刪除", run: () => remove(n, more) },
    ]);
    main.replaceChildren(...[
      h("header", { class: "notes-head" },
        h("h3", {}, titleNode(n)),
        h("button", { class: "btn small", onclick: () => edit(n), "aria-label": "編輯" }, icon("pen"), "編輯"),
        more),
      h("div", { class: "notes-meta" }, badge(n.source), h("span", {}, when(n.created)),
        n.tags.map((t) => h("span", { class: "chip" }, t)),
        n.kb.length ? h("span", { class: "chip", title: n.kb.join(", ") }, `知識卡 ${n.kb.length}`) : null),
      findingsList(st.findings[n.id]),
      rendered(n.body)].filter(Boolean));
    paint(main);
  }

  async function remove(n, anchor) {
    const ok = await LoreShell.ask(anchor, `刪除「${n.title || n.id}」？`, [{ label: "取消", value: null }, { label: "刪除", value: true, danger: true, primary: true }]);
    if (!ok) return;
    const r = await fetch(`/api/notes/${encodeURIComponent(n.id)}`, { method: "DELETE" });
    if (!r.ok) { LoreShell.toast(await r.text(), { err: true }); return; }
    st.sel = null;
    await refresh(true);
    LoreShell.toast("已移到垃圾桶");
  }

  // ── edit ──────────────────────────────────────────────
  function edit(n) {
    st.edit = { id: n?.id ?? null, dirty: false };
    const main = q("#notesMain");
    const title = h("input", { class: "notes-title", value: n?.title ?? "", placeholder: "標題", "aria-label": "標題" });
    const kind = h("div", { class: "seg", role: "group", "aria-label": "類別" },
      ["note", "idea"].map((k) => h("button", { "data-v": k, "aria-pressed": String((n?.kind ?? st.kind) === k) }, k === "note" ? "筆記" : "靈感")));
    kind.onclick = (e) => {
      const b = e.target.closest("button");
      if (!b) return;
      for (const x of kind.children) x.setAttribute("aria-pressed", String(x === b));
      st.edit.dirty = true;
    };
    const tags = h("input", { class: "notes-tags", value: n?.tags.join(", ") ?? "", placeholder: "標籤", "aria-label": "標籤（逗號分隔）" });
    const text = h("textarea", { class: "notes-src-text", spellcheck: "false", "aria-label": "LaTeX", placeholder: "LaTeX" });
    text.value = n?.body ?? "";
    const preview = h("div", { class: "notes-preview" });
    const issues = h("div", { class: "notes-issues" });
    const save = h("button", { class: "btn small primary", "aria-label": "儲存 ⌘↵" }, "儲存");
    const cancel = h("button", { class: "btn small" }, "取消");
    let t = null;
    const draw = () => preview.replaceChildren(rendered(text.value));
    for (const el of [title, tags, text]) el.addEventListener("input", () => { st.edit.dirty = true; if (el === text) { clearTimeout(t); t = setTimeout(draw, 200); } });
    text.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); save.click(); }
    });
    cancel.onclick = async () => { if (await leaveEdit(cancel)) renderRead(); };
    save.onclick = async () => {
      save.disabled = true;
      const body = {
        title: title.value.trim(), body: text.value,
        kind: kind.querySelector('[aria-pressed="true"]').dataset.v,
        tags: tags.value.split(/[,，]/).map((s) => s.trim()).filter(Boolean),
      };
      const id = st.edit.id;
      try {
        const r = await fetch(id ? `/api/notes/${encodeURIComponent(id)}` : "/api/notes", {
          method: id ? "PUT" : "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
        });
        if (r.status === 422) {
          const v = await r.json();
          issues.replaceChildren(findingsList(v.findings, true) ?? "");
          return;
        }
        if (!r.ok) throw new Error(await r.text());
        const v = await r.json();
        st.findings[v.id] = v.findings;
        st.edit = null;
        st.sel = v.id;
        st.kind = body.kind;
        await refresh(true);
        LoreShell.toast("已儲存");
      } catch (e) {
        LoreShell.toast(e.message, { err: true });
      } finally {
        save.disabled = false;
      }
    };
    main.replaceChildren(
      h("header", { class: "notes-head edit" }, title, kind),
      h("div", { class: "notes-meta" }, tags),
      issues,
      h("div", { class: "notes-split" }, text, preview),
      h("footer", { class: "notes-foot" }, cancel, save));
    paint(main);
    draw();
    renderList();
    (n ? text : title).focus();
  }

  /** Leave the editor; asks first when it holds unsaved text. */
  async function leaveEdit(anchor) {
    if (!st.edit) return true;
    if (st.edit.dirty) {
      const ok = await LoreShell.ask(anchor ?? q(".close"), "放下沒儲存的修改？", [{ label: "繼續編輯", value: null }, { label: "放下", value: true, danger: true, primary: true }]);
      if (!ok) return false;
    }
    st.edit = null;
    return true;
  }

  // ── open / close ──────────────────────────────────────
  async function open() {
    if (!dlg) build();
    if (!dlg.open) dlg.showModal();
    await macros();
    await refresh(true);
    startPoll();
  }

  async function close() {
    if (!(await leaveEdit())) return;
    renderList();
    dlg.close();
  }

  return { open, close, refresh };
})();
