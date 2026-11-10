// 開啟 LaTeX… / 開啟 Word…: papers edited where they are (opened.rs).
// Desktop: native dialogs through /api/shell/dialog, then a card for what
// happens next (the restructuring preview, where to unpack or convert).
// Browser (501): the old import for LaTeX, an upload for Word.
// Also: 最近開啟, and noticing edits made elsewhere (another editor, git pull).

const PCOpen = (() => {
  const desktop = () => state.workspace?.shell === "desktop";
  const baseName = (p) => String(p ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";
  const parentOf = (p) => String(p ?? "").replace(/[\\/][^\\/]*[\\/]?$/, "");

  async function dialog(kind, extra = {}) {
    try { return (await api("/api/shell/dialog", { kind, ...extra }))?.path ?? null; }
    catch (e) { if (e.status === 501) return undefined; throw e; }
  }

  /** 開啟 LaTeX…: a project's .tex (its folder opens) or an archive. */
  async function latex(anchor) {
    if (!desktop()) { importForm(anchor); return; }
    const p = await dialog("latex").catch((e) => { flash(e.message, 6000); return null; });
    if (p) await path(p, anchor);
  }

  /** 開啟 Word…: a .docx, converted into a LaTeX folder (the .docx stays as it is). */
  async function word(anchor) {
    if (!desktop()) { uploadWord(); return; }
    const p = await dialog("word").catch((e) => { flash(e.message, 6000); return null; });
    if (p) await path(p, anchor);
  }

  function uploadWord() {
    const f = el("input");
    f.type = "file";
    f.accept = ".docx";
    f.onchange = async () => {
      const file = f.files[0];
      if (!file) return;
      flash("轉換中…", 60000);
      try {
        const r = await fetch(`/api/open/word-upload?name=${encodeURIComponent(file.name)}`, { method: "POST", body: file });
        if (!r.ok) throw new Error(await r.text());
        await done(await r.json());
      } catch (e) { pandocOr(e, $("#actFile")); }
    };
    f.click();
  }

  /** Open whatever `p` is: a paper, a LaTeX folder, an archive, a .docx. */
  async function path(p, anchor = $("#actFile")) {
    let r;
    try { r = await api("/api/open/inspect", { path: p }); }
    catch (e) { flash(e.message, 6000); return; }
    await route(r, anchor);
  }

  async function route(r, anchor) {
    if (r.kind === "paper") return done(r);
    if (r.kind === "latex") return preview(r, anchor);
    if (r.kind === "archive") return destCard(r, anchor, "存到", "解壓縮", "/api/open/extract");
    if (r.kind === "word") {
      if (!r.pandoc) return pandocCard(anchor);
      return destCard(r, anchor, "存到", "轉成 LaTeX", "/api/open/word");
    }
  }

  async function done(r) {
    $("#fileMenu").hidden = true;
    state.workspace = await api("/api/workspace").catch(() => state.workspace);
    await openPaper(r.id);
    PCHome.refresh();
    const c = r.converted;
    if (c) {
      const bits = [`${c.sections} 節`];
      if (c.changes) bits.push(`${c.changes} 處修訂`);
      if (c.comments) bits.push(`${c.comments} 則註解`);
      if (c.notation?.length) bits.push(`${c.notation.length} 處編號沒對上`);
      flash(`已轉成 LaTeX：${bits.join("、")}`, 5000);
      if (c.changes || c.comments) window.PCReview?.setMode("review");
    }
  }

  const row = (m, label, ...content) => {
    const d = el("div", "nc-row");
    d.append(el("span", "nc-label", label), ...content);
    m.append(d);
    return d;
  };
  const list = (items) => {
    const ul = el("ul", "nc-list");
    items.forEach((parts) => { const li = el("li"); li.append(...parts); ul.append(li); });
    return ul;
  };
  const code = (t) => el("code", "", t);

  function foot(m, okLabel, onOk) {
    const f = el("div", "form-foot");
    const no = el("button", "ghost small", "取消");
    no.onclick = () => { m.hidden = true; };
    const ok = el("button", "primary small", okLabel);
    ok.onclick = async () => {
      ok.disabled = true; no.disabled = true;
      try { await onOk(); } catch (e) { pandocOr(e, null); ok.disabled = false; no.disabled = false; }
    };
    f.append(no, ok);
    m.append(f);
    keepOnScreen(m);
    ok.focus();
    return ok;
  }

  /** First open of a LaTeX folder: every file that will change, nothing written yet. */
  function preview(r, anchor) {
    const plan = r.plan;
    const m = menuForm(`開啟 ${baseName(plan.dir)}`, null, anchor);
    m.classList.add("next-card", "open-card");
    m.append(el("div", "open-path", plan.dir));
    if (plan.main !== "main.tex") row(m, "主檔", code(plan.main), " → ", code("main.tex"));
    const created = plan.created.filter((f) => !(f === "main.tex" && plan.main !== "main.tex"));
    if (created.length) row(m, "新增", list(created.map((f) => [code(f)])));
    if (plan.changed.length) row(m, "改寫", list(plan.changed.map((f) => [code(f)])));
    if (plan.merged.length) row(m, "併入", list(plan.merged.filter(([f]) => f !== plan.main).map(([f, into]) => [code(f), " → ", code(into)])));
    if (plan.moved.length) row(m, "移動", list(plan.moved.map(([f, to]) => [code(f), " → ", code(to)])));
    row(m, "設定", code(".papercomposer/"));
    foot(m, "確定", async () => {
      const res = await api("/api/open/latex", { path: r.path, stamp: plan.stamp });
      await done(res);
    });
  }

  /** Where an archive unpacks / a .docx converts to: beside it, or a folder picked. */
  function destCard(r, anchor, label, okLabel, endpoint) {
    const m = menuForm(baseName(r.path), null, anchor);
    m.classList.add("next-card", "open-card");
    let dest = r.dest;
    const where = el("span", "open-path", dest);
    const change = el("button", "ghost small", "更改…");
    change.onclick = async () => {
      const dir = await dialog("folder", { start: parentOf(dest) }).catch(() => null);
      if (dir) { dest = `${dir.replace(/[\\/]$/, "")}/${baseName(dest)}`; where.textContent = dest; }
    };
    const line = row(m, label, where);
    if (desktop()) line.append(change);
    foot(m, okLabel, async () => {
      flash(`${okLabel}中…`, 120000);
      const res = await api(endpoint, { path: r.path, dest });
      flash("", 1);
      if (res.kind === "paper") await done(res);
      else await route(res, anchor);
    });
  }

  function pandocCard(anchor) {
    const m = menuForm("需要 pandoc", null, anchor);
    m.classList.add("next-card", "open-card");
    const cmd = navigator.platform.startsWith("Mac") ? "brew install pandoc" : "pandoc.org/installing";
    row(m, "安裝", code(cmd));
    const f = el("div", "form-foot");
    const ok = el("button", "ghost small", "好");
    ok.onclick = () => { m.hidden = true; };
    f.append(ok);
    m.append(f);
    keepOnScreen(m);
  }

  function pandocOr(e, anchor) {
    if (String(e.message).includes("pandoc-missing")) pandocCard(anchor ?? $("#actFile"));
    else flash(e.message, 7000);
  }

  // ── 最近開啟 ─────────────────────────────────────────
  async function recent() {
    try { return await api("/api/open/recent"); } catch { return []; }
  }

  /** The list under the start screen's cards; empty → nothing drawn. */
  async function recentList() {
    const items = await recent();
    const box = el("div", "open-recent");
    if (!items.length) return box;
    box.append(el("div", "home-sub", "最近開啟"));
    const ul = el("ul");
    for (const it of items) {
      const li = el("li");
      const b = el("button", "open-item");
      b.type = "button";
      b.disabled = !it.exists;
      b.title = it.exists ? it.path : `找不到 ${it.path}`;
      b.append(icon(it.kind === "word" ? "openWord" : "openLatex"), el("span", "open-name", it.id), el("span", "open-where", parentOf(it.path)));
      b.onclick = () => path(it.path, b);
      li.append(b);
      if (!it.exists) {
        const x = iconButton("x", "從清單移除", async () => {
          try { await api("/api/open/forget", { id: it.id }); li.remove(); } catch (e) { flash(e.message, 6000); }
        });
        li.append(x);
      }
      ul.append(li);
    }
    box.append(ul);
    return box;
  }

  // ── Edits made elsewhere ──────────────────────────────
  let asking = null;

  /** 檔案在別處改過: 重新載入 / 保留我的版本. One question at a time. */
  function conflict() {
    if (asking) return asking;
    asking = new Promise((resolve) => {
      const bar = el("div", "open-conflict");
      bar.setAttribute("role", "alertdialog");
      const reload = el("button", "primary small", "重新載入");
      const mine = el("button", "ghost small", "保留我的版本");
      const end = (v) => { bar.remove(); asking = null; resolve(v); };
      reload.onclick = () => end("reload");
      mine.onclick = () => end("mine");
      bar.append(el("span", "", "這篇在別處改過"), reload, mine);
      document.body.append(bar);
    });
    return asking;
  }

  const editing = () => document.querySelector("#paper .para > textarea, #paper textarea.field-edit");

  async function check() {
    const p = state.paper;
    if (!p?.stamp || document.hidden || PCHome.isOpen() || asking) return;
    let now;
    try { now = (await api(`/api/papers/${encodeURIComponent(p.name)}/stamp`)).stamp; } catch { return; }
    if (state.paper !== p || now === p.stamp) return;
    const ta = editing();
    if (!ta) {
      await openPaper(p.name, { keepHome: true });
      flash("已載入別處的修改");
      return;
    }
    const pick = await conflict();
    if (state.paper !== p) return;
    if (pick === "reload") {
      ta.dataset.discard = "1"; // its blur does not save
      await openPaper(p.name, { keepHome: true });
    } else {
      // my paragraph is written over theirs when it is saved, then the rest is read again
      p.force = true;
      p.stamp = now;
    }
  }

  function watch() {
    window.addEventListener("focus", check);
    document.addEventListener("visibilitychange", check);
    setInterval(check, 4000);
  }

  return { latex, word, path, recentList, conflict, watch, desktop };
})();
