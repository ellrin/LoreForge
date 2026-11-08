// Settings: workspace & sync, local model, Claude, appearance.
// On first run it walks through the first three as steps; the gear in the
// top bar reopens it as plain settings, without the step buttons.
// Every user points the app at their own private workspace; nothing here is
// tied to a particular account or repository.

const Setup = (() => {
  let st = null;
  let step = 0;
  let mode = null; // workspace tab; defaults to "new" until a workspace is set
  let selected = null;
  let poll = null;
  let changed = false; // workspace changed: reload the app on close

  const gb = (bytes) => `${(bytes / 1024 ** 3).toFixed(bytes > 1024 ** 3 * 10 ? 0 : 1)} GB`;
  const mb = (bytes) => bytes < 1024 ** 3 ? `${Math.round(bytes / 1024 ** 2)} MB` : gb(bytes);

  async function refresh() {
    try { st = await api("/api/setup/status"); } catch (e) { flash(e.message); return; }
    render();
  }

  const STEPS = ["workspace", "model", "claude", "look"];
  const LAST = 2; // the guided steps end at Claude; appearance is settings only
  let guided = false;

  /** Open on step `s` (index or name), or on the first unfinished one. */
  async function open(s, showVia, settings = true) {
    if (typeof s === "string") s = STEPS.indexOf(s);
    guided = !settings;
    const dlg = $("#dlgSetup");
    if (!dlg.open) dlg.showModal();
    if (showVia) via = showVia;
    await Promise.all([refresh(), loadEngines(true)]);
    if (s == null) { const i = done().indexOf(false); s = i < 0 ? 0 : i; }
    step = s;
    render();
  }

  const close = () => $("#dlgSetup").close();

  // Runs however the dialog closes: button, Esc, backdrop or "完成".
  function onClosed() {
    clearInterval(poll);
    clearInterval(ccPoll); ccPoll = null;
    via = null;
    store.set("pc.setupSeen", true);
    if (changed) location.reload();
  }

  const gitOk = () => st.git_tool.installed && !!st.git_tool.name && !!st.git_tool.email;
  const done = () => [
    // A workspace that syncs also needs Git and a commit author.
    !!st && !st.workspace.demo && st.workspace.has_profile && (!st.workspace.git?.remote || gitOk()),
    !!st && (!!st.running || st.models.some((m) => m.installed)),
    !!(state.engines?.subscription.configured || state.engines?.api.configured),
  ];

  function render() {
    if (!st) return;
    const d = done();
    document.querySelectorAll("#wzTabs button").forEach((b, i) => {
      b.setAttribute("aria-selected", String(i === step));
      b.querySelector(".st").textContent = d[i] ? "✓" : "";
      b.onclick = () => { step = i; render(); };
    });
    document.querySelectorAll(".wizard .step").forEach((s) => (s.hidden = +s.dataset.step !== step));
    $(".wz-foot").hidden = !guided || step > LAST;
    $("#wzPrev").disabled = step === 0;
    $("#wzNext").textContent = step === LAST ? "完成" : "下一步";
    [renderWorkspace, renderModels, renderClaude, () => {}][step]();
  }

  // ── 01 workspace ──
  function check(ok, text) { const li = el("li"); li.append(el("span", `status ${ok ? "" : "warning"}`), el("span", "", text)); return li; }

  function renderWorkspace() {
    const w = st.workspace;
    const now = $("#wzWsNow");
    now.innerHTML = "";
    mode ??= w.demo ? "new" : "open";
    const ul = el("ul", "checks");
    if (!w.demo) {
      ul.append(check(w.has_profile, w.root));
      ul.append(check(true, w.git?.remote ? `同步 ${w.git.remote}` : "只存在這台電腦"));
    }
    now.append(ul);
    document.querySelectorAll("#wzWsMode button").forEach((b) => {
      b.setAttribute("aria-pressed", String(b.dataset.v === mode));
      b.onclick = () => { mode = b.dataset.v; renderWorkspace(); };
    });
    document.querySelectorAll('.step[data-step="0"] .form').forEach((f) => (f.hidden = f.dataset.mode !== mode));
    // Example paths in this OS's style.
    const home = isWin ? "C:\\Users\\you\\Documents" : "/Users/you/Documents";
    $("#wzNewPath").placeholder = `${home}${isWin ? "\\" : "/"}my-paper-workspace`;
    $("#wzCloneParent").placeholder = home;
    $("#wzOpenPath").placeholder = `${home}${isWin ? "\\" : "/"}my-paper-workspace`;
    renderGit();
  }

  async function visibility(input, out) {
    out.textContent = "";
    out.className = "vis";
    const url = input.value.trim();
    if (!url) return;
    try {
      const r = await api(`/api/setup/visibility?url=${encodeURIComponent(url)}`);
      if (r.host !== "github" || r.public == null) return;
      out.textContent = r.public ? "公開 repo：請先改成 private" : "private";
      out.className = r.public ? "vis bad" : "vis ok";
    } catch {}
  }

  async function workspaceAction(path, body, btn) {
    btn.disabled = true;
    try {
      const r = await api(path, body);
      changed = true;
      flash(r.note ?? `已使用 ${r.root}${r.added ? `，補上 ${r.added} 個檔案` : ""}`, r.note ? 8000 : 3000);
      await refresh();
    } catch (e) { flash(e.message, 8000); }
    finally { btn.disabled = false; }
  }

  // Git: only shown when the workspace syncs, or is about to.
  let editAuthor = false;
  function renderGit() {
    const g = st.git_tool;
    const syncing = !!st.workspace.git?.remote || mode === "clone" || (mode === "new" && !!$("#wzNewRemote").value.trim());
    $("#wzGitBox").hidden = !syncing;
    if (!syncing) return;
    const ul = $("#wzGitNow");
    ul.innerHTML = "";
    ul.append(check(g.installed, g.installed ? g.version : "需要安裝 Git"));
    if (g.installed) {
      const li = check(!!(g.name && g.email), g.name && g.email ? `作者 ${g.name} <${g.email}>` : "設定 commit 作者");
      if (g.name && g.email) li.lastChild.append(iconButton("pen", "修改作者", () => { editAuthor = true; renderGit(); }));
      ul.append(li);
    }
    $("#wzGitForm").hidden = !g.installed || (!!(g.name && g.email) && !editAuthor);
    if (!$("#wzGitName").value) $("#wzGitName").value = g.name ?? "";
    if (!$("#wzGitEmail").value) $("#wzGitEmail").value = g.email ?? "";
  }

  // ── 03 local model ──
  function renderModels() {
    const s = st.system;
    const sys = $("#wzSys");
    sys.innerHTML = "";
    const parts = [`記憶體 ${Math.round(s.memory_gb)} GB${s.unified_memory ? "（統一記憶體）" : ""}`];
    if (s.gpu) parts.push(`${s.gpu}${s.gpu_memory_gb ? ` ${Math.round(s.gpu_memory_gb)} GB` : ""}`);
    sys.append(el("p", "note", parts.join(" · ")));

    if (!selected) selected = st.running ?? st.models.find((m) => m.installed)?.entry.id ?? st.recommended;
    const ul = $("#wzModels");
    ul.innerHTML = "";
    st.models.forEach(({ entry: m, fits, installed }) => {
      const li = el("li", "model" + (m.id === selected ? " on" : "") + (fits ? "" : " dim"));
      const head = el("div", "mh");
      head.append(el("b", "", `${m.name} ${m.variant}`), el("span", "sum", m.summary));
      const tags = el("div", "tags");
      if (m.id === st.recommended) tags.append(el("span", "tag acc", "推薦"));
      if (m.uncensored) tags.append(el("span", "tag warn", "Uncensored"));
      if (st.running === m.id) tags.append(el("span", "tag ok", "使用中"));
      else if (installed) tags.append(el("span", "tag", "已下載"));
      if (!fits) tags.append(el("span", "tag bad", `需 ${m.min_memory_gb} GB`));
      head.append(tags);
      const meta = el("div", "meta", `約 ${+m.approx_gb.toFixed(1)} GB · 至少 ${m.min_memory_gb} GB 記憶體 · ${m.license}${m.vision ? " · 可看圖" : ""}`);
      li.append(head, meta);
      li.onclick = () => { selected = m.id; $("#wzPlan").innerHTML = ""; renderModels(); };
      ul.append(li);
    });
    if (!st.models.some((m) => m.fits)) ul.append(el("li", "note", "記憶體不足：改用 Claude"));
    renderModelActions();
  }

  function renderModelActions() {
    const box = $("#wzPlan");
    if (box.dataset.busy === "1") return;
    box.innerHTML = "";
    const m = st.models.find((x) => x.entry.id === selected);
    if (!m) return;
    const acts = el("div", "actions-row");
    if (st.running === selected) {
      const b = el("button", "ghost", "停止");
      b.onclick = async () => { await api("/api/models/stop", {}); refresh(); };
      acts.append(b);
    } else if (m.installed) {
      const b = el("button", "primary small", "啟用");
      b.onclick = async () => {
        b.disabled = true; b.textContent = "啟動中…";
        try { await api("/api/models/start", { id: selected }); flash("模型已啟用"); } catch (e) { flash(e.message, 8000); }
        refresh();
      };
      acts.append(b);
    } else {
      const b = el("button", "primary small", "準備下載");
      b.disabled = !m.fits;
      b.onclick = () => preparePlan(b);
      acts.append(b);
    }
    box.append(acts);
  }

  async function preparePlan(btn) {
    btn.disabled = true; btn.textContent = "查詢檔案中…";
    let plan;
    try { plan = await api("/api/models/plan", { id: selected }); }
    catch (e) { flash(e.message, 8000); btn.disabled = false; btn.textContent = "準備下載"; return; }
    const box = $("#wzPlan");
    box.innerHTML = "";
    const t = el("table", "plan");
    plan.files.forEach((f) => {
      const tr = el("tr");
      tr.append(el("td", "k", { model: "模型", mmproj: "看圖", runtime: "llama.cpp" }[f.kind] ?? f.kind), el("td", "n", f.name), el("td", "s", f.installed ? "已有" : mb(f.size)), el("td", "src", f.source));
      t.append(tr);
    });
    box.append(t);
    const acts = el("div", "actions-row");
    const go = el("button", "primary small", `確認下載 ${mb(plan.total_to_download)}`);
    go.onclick = () => install();
    const cancel = el("button", "ghost", "取消");
    cancel.onclick = () => renderModelActions();
    acts.append(cancel, go);
    box.append(acts);
  }

  async function install() {
    try { await api("/api/models/install", { id: selected }); }
    catch (e) { flash(e.message, 8000); return; }
    const box = $("#wzPlan");
    box.dataset.busy = "1";
    clearInterval(poll);
    poll = setInterval(async () => {
      let p;
      try { p = await api("/api/models/progress"); } catch { return; }
      box.innerHTML = "";
      p.files.sort((a, b) => a.name.localeCompare(b.name)).forEach((f) => {
        const row = el("div", "prog");
        const pct = f.total ? Math.min(100, (f.done / f.total) * 100) : 0;
        const bar = el("div", "bar"); const fill = el("i"); fill.style.width = `${pct}%`; bar.append(fill);
        row.append(el("span", "n", f.name), bar, el("span", "s", f.state === "error" ? f.error : `${mb(f.done)} / ${mb(f.total)}`));
        box.append(row);
      });
      const phase = p.phase;
      box.append(el("p", "note", phase === "starting" ? "下載完成，正在啟動模型…" : phase === "ready" ? "已啟用" : phase.startsWith("error") ? phase.slice(7) : "下載中…"));
      if (phase === "ready" || phase.startsWith("error")) {
        clearInterval(poll);
        box.dataset.busy = "0";
        await refresh();
        if (phase === "ready") {
          flash("本機模型已啟用");
          loadEngines();
        }
      }
    }, 1000);
  }

  // ── 04 claude ──
  // Which connection the Claude step is showing; switching tabs to a ready
  // one also switches what Claude uses.
  let via = null;
  let ccPoll = null;
  const isWin = /Win/i.test(navigator.platform);
  const INSTALL = isWin ? "irm https://claude.ai/install.ps1 | iex" : "curl -fsSL https://claude.ai/install.sh | bash";

  async function refreshCC() {
    await loadEngines(true);
    if (step === 2) renderClaude();
  }

  function renderClaude() {
    const e = state.engines;
    if (!e) return;
    via ??= claudeVia();
    document.querySelectorAll("#wzVia button").forEach((b) => {
      const v = b.dataset.v;
      b.setAttribute("aria-pressed", String(v === via));
      b.querySelector(".st")?.remove();
      if (e[v]?.configured) b.append(el("span", "st", v === claudeVia() ? " ● 使用中" : " ✓"));
      b.onclick = () => {
        via = v;
        if (e[v]?.configured) { store.set("pc.claudeVia", v); renderEngineSwitch(); renderStatus(); }
        $("#wzTestOut").textContent = "";
        renderClaude();
      };
    });
    document.querySelectorAll(".step[data-step=\"2\"] .via").forEach((d) => (d.hidden = d.dataset.via !== via));

    // subscription
    const cc = e.subscription.status;
    const ul = $("#ccChecks");
    ul.innerHTML = "";
    ul.append(check(cc.installed, cc.installed ? `Claude Code ${cc.version ?? ""}` : "安裝 Claude Code"));
    if (cc.installed) {
      const text = cc.ready ? `已登入 ${cc.account ?? ""}`
        : cc.logged_in ? "改用 claude.ai 帳號登入" : "登入 claude.ai 帳號";
      ul.append(check(cc.ready, text));
    }
    $("#ccInstall").hidden = cc.installed;
    $("#ccCmd").textContent = INSTALL;
    $("#ccLogin").hidden = cc.ready;
    $("#ccLogin").disabled = !cc.installed || !!ccPoll;
    $("#ccLogin").textContent = ccPoll ? "等待瀏覽器登入…" : "用瀏覽器登入";

    // api
    const c = { configured: e.api.configured, masked: e.api.masked, source: e.api.source };
    const editing = !c.configured || editingKey;
    $("#wzKeyShow").hidden = editing;
    $("#wzKeyEnter").hidden = !editing;
    $("#wzClaudeSave").hidden = !editing;
    $("#wzKeyCancel").hidden = !c.configured;
    $("#wzKeyMasked").textContent = c.masked ?? "";
    $("#wzClaudeRemove").hidden = c.source !== "keychain";
    $("#wzKeyEdit").setAttribute("aria-label", c.source === "env" ? "更換 key（目前來自環境變數）" : "更換 key");

    const sel = $("#wzClaudeModel");
    if (!sel.options.length) e.models.forEach((m) => sel.append(new Option(modelName(m), m)));
    sel.value = e.model;
    $("#wzTest").disabled = !e[via]?.configured;
  }

  async function loginCC() {
    try { await api("/api/claude-code/login", {}); }
    catch (err) { flash(err.message, 8000); return; }
    flash("已在瀏覽器開啟 claude.ai 登入頁，完成後回到這裡", 6000);
    let tries = 0;
    clearInterval(ccPoll);
    ccPoll = setInterval(async () => {
      tries++;
      await loadEngines(true);
      const ok = state.engines?.subscription.configured;
      if (ok || tries > 100) {
        clearInterval(ccPoll); ccPoll = null;
        if (ok) { store.set("pc.claudeVia", "subscription"); flash("已用 claude.ai 帳號登入"); renderEngineSwitch(); renderStatus(); }
      }
      if (step === 2) renderClaude();
    }, 3000);
    renderClaude();
  }

  async function testClaude() {
    const out = $("#wzTestOut");
    const btn = $("#wzTest");
    btn.disabled = true; out.className = "note"; out.textContent = "測試中…";
    let r;
    try { r = await api("/api/engines/test", { engine: via }); } catch (err) { r = { ok: false, error: err.message }; }
    btn.disabled = false;
    out.className = r.ok ? "note ok" : "note bad";
    out.textContent = r.ok ? `✓ 連線成功（${(r.ms / 1000).toFixed(1)} 秒，${via === "subscription" ? "走訂閱" : "走 API 計費"}）` : `✕ ${r.error}`;
  }

  let editingKey = false;
  function editKey(on) {
    if (on && st?.claude.source === "env") {
      flash("這把 key 來自環境變數，App 改不到。請改用 scripts/dev.sh --reset-key 更換", 8000);
      return;
    }
    editingKey = on;
    $("#wzClaudeKey").value = "";
    renderClaude();
    if (on) $("#wzClaudeKey").focus();
  }

  async function saveClaude(body) {
    let ok = true;
    try { state.claude = await api("/api/settings/claude", body); }
    catch (e) { ok = false; flash(e.message, 8000); }
    if (ok) flash(body.key === "" ? "已移除 Claude key" : body.key ? "已儲存 Claude key" : "已切換模型");
    if (ok && body.key) store.set("pc.claudeVia", "api");
    await loadEngines();
    if (ok && body.key !== undefined) { editingKey = false; $("#wzClaudeKey").value = ""; }
    refresh();
  }

  function wire() {
    $("#wzPrev").onclick = () => { if (step > 0) { step--; render(); } };
    $("#wzNext").onclick = () => { if (step < LAST) { step++; render(); } else close(); };
    $("#wzSkip").onclick = () => { if (step < LAST) { step++; render(); } else close(); };
    const dlg = $("#dlgSetup");
    dlg.addEventListener("close", onClosed);
    // A click on the backdrop lands on the <dialog> itself, not its content.
    dlg.addEventListener("click", (e) => { if (e.target === dlg) close(); });
    $("#wzClose").onclick = close;

    $("#wzOpen").onclick = (e) => workspaceAction("/api/setup/workspace", { path: $("#wzOpenPath").value, init: $("#wzOpenInit").checked }, e.target);
    $("#wzClone").onclick = (e) => workspaceAction("/api/setup/clone", { url: $("#wzCloneUrl").value, parent: $("#wzCloneParent").value }, e.target);
    $("#wzNew").onclick = (e) => workspaceAction("/api/setup/new", { path: $("#wzNewPath").value, remote: $("#wzNewRemote").value || null }, e.target);
    $("#wzGhNew").onclick = async () => {
      try { await api("/api/open/github-new-repo", {}); flash("建好後把網址貼回來"); }
      catch (e) { flash(e.message, 6000); }
      $("#wzNewRemote").focus();
    };
    $("#wzNewRemote").oninput = renderGit;
    $("#wzCloneUrl").onchange = () => visibility($("#wzCloneUrl"), $("#wzCloneVis"));
    $("#wzNewRemote").onchange = () => visibility($("#wzNewRemote"), $("#wzNewVis"));

    $("#wzGitSave").onclick = async () => {
      try { await api("/api/setup/git-identity", { name: $("#wzGitName").value, email: $("#wzGitEmail").value }); flash("已設定作者"); editAuthor = false; }
      catch (e) { flash(e.message, 6000); }
      refresh();
    };
    $("#wzClaudeGetKey").onclick = () => openKeysPage("#wzClaudeKey");
    $("#wzClaudeRemove").onclick = () => { if (confirm("從這台電腦的金鑰圈移除 Claude key？")) saveClaude({ key: "" }); };
    $("#wzKeyEdit").onclick = () => editKey(true);
    $("#wzKeyCancel").onclick = () => editKey(false);
    $("#wzClaudeModel").onchange = () => saveClaude({ model: $("#wzClaudeModel").value });
    $("#wzClaudeSave").onclick = () => {
      const k = $("#wzClaudeKey").value.trim();
      if (!k) { $("#wzClaudeKey").focus(); return; }
      saveClaude({ key: k, model: $("#wzClaudeModel").value });
    };
    $("#ccRecheck").onclick = refreshCC;
    $("#ccLogin").onclick = loginCC;
    $("#ccCopy").onclick = () => { navigator.clipboard?.writeText(INSTALL); flash("已複製安裝指令"); };
    $("#wzTest").onclick = testClaude;
    $("#wzClaudeKey").onkeydown = (e) => { if (e.key === "Enter") $("#wzClaudeSave").click(); };
  }

  /** First run: no workspace configured yet. */
  async function maybeOpen() {
    try {
      const s = await api("/api/setup/status");
      if (s.first_run && !store.get("pc.setupSeen", false)) open(0, null, false);
    } catch {}
  }

  return { open, wire, maybeOpen };
})();
