// Settings in the shared gear (LoreShell.settings, commons/ui/shell.js):
//   一般  the workspace's folder and the commit author
//   外觀  the shared theme (the shell) and 紙張 (white or dimmed page)
//   模型  connections and routes (/api/engines, the shell); 下載／登入… open this
//         app's own steps in a window over it (本機模型: download, start; Claude:
//         sign in, API key, model)
//   同步  the workspace: sync now, new / clone / open another folder
// The steps' markup is #setupParts in index.html; each tab moves its part in.
// On first run the gear opens by itself on the first thing still to do; the gear
// (markSettings in app.js) and those tabs carry a dot until it is done.
// Every user points the app at their own private workspace; nothing here is
// tied to a particular account or repository.

const Setup = (() => {
  let st = null;
  let shown = null; // "local" | "claude" while the model window shows that part
  let mode = null; // workspace tab; defaults to "new" until a workspace is set
  let selected = null;
  let poll = null;
  let changed = false; // workspace changed: reload the app on close

  const gb = (bytes) => `${(bytes / 1024 ** 3).toFixed(bytes > 1024 ** 3 * 10 ? 0 : 1)} GB`;
  const mb = (bytes) => bytes < 1024 ** 3 ? `${Math.round(bytes / 1024 ** 2)} MB` : gb(bytes);
  // the parts, kept once: a tab switch empties the shell's body, and a part out of the page
  // goes back into #setupParts (park) so its $("#wz…") lookups keep working
  const PARTS = {};
  const part = (id) => (PARTS[id] ??= document.getElementById(id));
  const park = () => { const home = document.getElementById("setupParts"); for (const n of Object.values(PARTS)) if (!n.isConnected) home.append(n); };
  const row = (k, ...v) => { const r = el("div", "set-row"); r.append(el("span", "k", k), ...v); return r; };
  const btn = (label, onclick) => { const b = el("button", "btn small", label); b.type = "button"; b.onclick = onclick; return b; };

  async function refresh() {
    try { st = await api("/api/setup/status"); } catch (e) { LoreShell.toast(e.message, { err: true }); return; }
    render();
  }

  const gitOk = () => st.git_tool.installed && !!st.git_tool.name && !!st.git_tool.email;
  /** [workspace, local model, Claude] set up? */
  const done = () => [
    // A workspace that syncs also needs Git and a commit author.
    !!st && !st.workspace.demo && st.workspace.has_profile && (!st.workspace.git?.remote || gitOk()),
    !!st && (!!st.running || st.models.some((m) => m.installed)),
    !!(state.engines?.subscription.configured || state.engines?.api.configured),
  ];

  /** Redraw whatever part is on screen. */
  function render() {
    if (!st) return;
    if (part("partWorkspace").isConnected) renderWorkspace();
    if (part("partGit").isConnected) renderGit();
    if (shown === "local") renderModels();
    if (shown === "claude") renderClaude();
    renderGeneral();
    renderSyncState();
    markTabs();
  }

  /** Older entry points by step name: the tab, or the model window. */
  function open(s, showVia) {
    if (s === "model" || s === "local") return model("local");
    if (s === "claude") return model("claude", showVia);
    LoreShell.settings(s === "workspace" ? "sync" : s === "look" ? "look" : undefined);
  }

  // ── the gear's tabs ──
  let generalBox = null, syncBox = null;
  function general(box) {
    generalBox = el("div", "set-sec");
    box.append(generalBox, part("partGit"));
    renderGeneral();
    refresh();
  }
  function renderGeneral() {
    if (!generalBox?.isConnected || !st) return;
    const w = st.workspace;
    const where = el("span", "v mono", w.demo ? "示範資料" : w.root);
    where.title = w.root ?? "";
    generalBox.replaceChildren(el("div", "set-label", "工作區"),
      row("資料夾", where, btn("更換…", () => LoreShell.settings("sync"))),
      row("同步", el("span", "v", w.git?.remote ?? "只存在這台電腦")));
  }
  function sync(box) {
    syncBox = el("div", "set-sec");
    const ws = el("div", "set-sec");
    ws.append(el("div", "set-label", "工作區"), part("partWorkspace"));
    box.append(syncBox, ws);
    renderSyncState();
    refresh();
  }
  function renderSyncState() {
    if (!syncBox?.isConnected) return;
    const g = state.git;
    const now = btn(state.syncing ? "同步中…" : "立即同步", async () => { await syncNow(); renderSyncState(); });
    now.disabled = !g?.repo || !!state.syncing;
    const what = !g?.repo ? "只存在這台電腦" : g.changed.length ? `${g.changed.length} 項未同步` : g.ahead ? `${g.ahead} 個 commit 未推送` : !g.remote ? "已存檔（沒有遠端）" : state.lastSync ? `已同步 ${state.lastSync}` : "已同步";
    syncBox.replaceChildren(el("div", "set-label", "同步"), row("狀態", el("span", "v", what), now));
  }
  function look(box) { box.append(part("partPaper")); applyTheme(); }

  /** 下載／登入…: this app's own steps for one connection, in a window over the gear. */
  async function model(kind, showVia) {
    const dlg = $("#dlgModel");
    shown = kind;
    $("#dlgModelTitle").textContent = kind === "local" ? "本機模型" : "Claude";
    $("#dlgModelBody").replaceChildren(part(kind === "local" ? "partLocal" : "partClaude"));
    park();
    if (showVia) via = showVia;
    if (!dlg.open) dlg.showModal();
    paintIcons(dlg);
    await Promise.all([refresh(), loadEngines(true)]);
    render();
  }

  // The first-run dots on the tabs still to do (the gear's own dot: markSettings).
  function markTabs() {
    const dlg = document.querySelector("dialog.lore-settings[open]");
    if (!dlg || !st) return;
    const d = done();
    const need = { sync: !d[0], model: !d[1] && !d[2] };
    dlg.querySelectorAll(".tabs button[data-tab]").forEach((b) => {
      const on = !!need[b.dataset.tab];
      if (b.classList.contains("attn") !== on) b.classList.toggle("attn", on);
    });
  }

  // 模型 while /api/engines is not there (the shell shows 「—」): this app's two
  // connections stay reachable from the gear.
  function modelFallback(dlg) {
    const body = dlg.querySelector('.dlg-body[data-tab="model"]');
    if (!body || !body.querySelector(":scope > .set-empty")) return;
    const e = state.engines;
    const local = e?.local.status.connected ? e.local.status.name : e?.local.models?.length ? "已下載" : "未安裝";
    const claude = claudeReady() ? `${VIA[claudeVia()]} · ${modelName(e.model)}` : "未連線";
    const sec = el("div", "set-sec pc-model");
    sec.append(el("div", "set-label", "連線"),
      row("本機", el("span", "v mono", local), btn("設定…", () => model("local"))),
      row("Claude", el("span", "v mono", claude), btn("設定…", () => model("claude"))));
    body.replaceChildren(sec);
  }

  // 模型 → 本機 model picker: through this app's own start (it also keeps config.toml
  // [model] active, which PUT /api/engines does not), then the tab reads /api/engines again.
  function localPick(dlg) {
    const sel = dlg.querySelector('.dlg-body[data-tab="model"] select[aria-label="本機模型"]');
    if (!sel || sel._pc) return;
    sel._pc = true;
    sel.onchange = async () => {
      sel.disabled = true;
      LoreShell.toast("啟動中…", { ms: 120000 });
      try { await api("/api/models/start", { id: sel.value }); LoreShell.toast("模型已啟用"); }
      catch (e) { LoreShell.toast(e.message, { err: true, ms: 8000 }); }
      await loadEngines(true);
      await LoreShell.engines.get(true);
      LoreShell.settings("model");
    };
  }

  /** Watch the shell's settings dialog: tab dots, the 模型 fallback, closing. */
  function watchSettings() {
    const hook = (dlg) => {
      if (dlg._pc) return;
      dlg._pc = true;
      new MutationObserver(() => { park(); markTabs(); modelFallback(dlg); localPick(dlg); }).observe(dlg, { childList: true, subtree: true });
      dlg.addEventListener("close", onClosed);
      markTabs();
      modelFallback(dlg);
    };
    const find = () => { const d = document.querySelector("dialog.lore-settings"); if (d) hook(d); return d; };
    if (!find()) {
      const mo = new MutationObserver(() => { if (find()) mo.disconnect(); });
      mo.observe(document.body, { childList: true });
    }
  }

  // Runs however the gear and the model window close.
  function onClosed() {
    if (document.querySelector("dialog.lore-settings[open], #dlgModel[open]")) return;
    clearInterval(poll);
    clearInterval(ccPoll); ccPoll = null;
    via = null;
    shown = null;
    store.set("pc.setupSeen", true);
    if (changed) location.reload();
  }

  /** What frame.js hands LoreShell.mount. */
  function shellParts() {
    return { general, look, sync, modelSetup: (kind) => model(kind === "claude" ? "claude" : "local") };
  }

  // ── workspace ──
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
    part("partWorkspace").querySelectorAll(".form").forEach((f) => (f.hidden = f.dataset.mode !== mode));
    // Example paths in this OS's style.
    const home = isWin ? "C:\\Users\\you\\Documents" : "/Users/you/Documents";
    $("#wzNewPath").placeholder = `${home}${isWin ? "\\" : "/"}my-paper-workspace`;
    $("#wzCloneParent").placeholder = home;
    $("#wzOpenPath").placeholder = `${home}${isWin ? "\\" : "/"}my-paper-workspace`;
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
      LoreShell.toast(r.note ?? `已使用 ${r.root}${r.added ? `，補上 ${r.added} 個檔案` : ""}`, { ms: r.note ? 8000 : 3000 });
      await refresh();
    } catch (e) { LoreShell.toast(e.message, { err: true, ms: 8000 }); }
    finally { btn.disabled = false; }
  }

  // Git and the commit author (一般): shown when the workspace syncs, or is about to.
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

  // ── 本機模型 ──
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
        try { await api("/api/models/start", { id: selected }); LoreShell.toast("模型已啟用"); } catch (e) { LoreShell.toast(e.message, { err: true, ms: 8000 }); }
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
    catch (e) { LoreShell.toast(e.message, { err: true, ms: 8000 }); btn.disabled = false; btn.textContent = "準備下載"; return; }
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
    catch (e) { LoreShell.toast(e.message, { err: true, ms: 8000 }); return; }
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
          LoreShell.toast("本機模型已啟用");
          loadEngines();
        }
      }
    }, 1000);
  }

  // ── Claude ──
  // Which connection the Claude part is showing; switching to a ready one also
  // switches what Claude uses.
  let via = null;
  let ccPoll = null;
  const isWin = /Win/i.test(navigator.platform);
  const INSTALL = isWin ? "irm https://claude.ai/install.ps1 | iex" : "curl -fsSL https://claude.ai/install.sh | bash";

  async function refreshCC() {
    await loadEngines(true);
    if (shown === "claude") renderClaude();
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
    part("partClaude").querySelectorAll(".via").forEach((d) => (d.hidden = d.dataset.via !== via));

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
    catch (err) { LoreShell.toast(err.message, { err: true, ms: 8000 }); return; }
    LoreShell.toast("已在瀏覽器開啟 claude.ai 登入頁，完成後回到這裡", { ms: 6000 });
    let tries = 0;
    clearInterval(ccPoll);
    ccPoll = setInterval(async () => {
      tries++;
      await loadEngines(true);
      const ok = state.engines?.subscription.configured;
      if (ok || tries > 100) {
        clearInterval(ccPoll); ccPoll = null;
        if (ok) { store.set("pc.claudeVia", "subscription"); LoreShell.toast("已用 claude.ai 帳號登入"); renderEngineSwitch(); renderStatus(); }
      }
      if (shown === "claude") renderClaude();
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
      LoreShell.toast("這把 key 來自環境變數，App 改不到。請改用 scripts/dev.sh --reset-key 更換", { ms: 8000 });
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
    catch (e) { ok = false; LoreShell.toast(e.message, { err: true, ms: 8000 }); }
    if (ok) LoreShell.toast(body.key === "" ? "已移除 Claude key" : body.key ? "已儲存 Claude key" : "已切換模型");
    if (ok && body.key) store.set("pc.claudeVia", "api");
    await loadEngines();
    if (ok && body.key !== undefined) { editingKey = false; $("#wzClaudeKey").value = ""; }
    refresh();
  }

  function wire() {
    ["partWorkspace", "partGit", "partLocal", "partClaude", "partPaper"].forEach(part);
    watchSettings();
    const md = $("#dlgModel");
    md.addEventListener("close", () => { shown = null; onClosed(); });
    md.querySelector(".close").onclick = () => md.close();
    md.addEventListener("click", (e) => { if (e.target === md) md.close(); });

    $("#wzOpen").onclick = (e) => workspaceAction("/api/setup/workspace", { path: $("#wzOpenPath").value, init: $("#wzOpenInit").checked }, e.target);
    $("#wzClone").onclick = (e) => workspaceAction("/api/setup/clone", { url: $("#wzCloneUrl").value, parent: $("#wzCloneParent").value }, e.target);
    $("#wzNew").onclick = (e) => workspaceAction("/api/setup/new", { path: $("#wzNewPath").value, remote: $("#wzNewRemote").value || null }, e.target);
    $("#wzGhNew").onclick = async () => {
      try { await api("/api/open/github-new-repo", {}); LoreShell.toast("建好後把網址貼回來"); }
      catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
      $("#wzNewRemote").focus();
    };
    $("#wzNewRemote").oninput = renderGit;
    $("#wzCloneUrl").onchange = () => visibility($("#wzCloneUrl"), $("#wzCloneVis"));
    $("#wzNewRemote").onchange = () => visibility($("#wzNewRemote"), $("#wzNewVis"));

    $("#wzGitSave").onclick = async () => {
      try { await api("/api/setup/git-identity", { name: $("#wzGitName").value, email: $("#wzGitEmail").value }); LoreShell.toast("已設定作者"); editAuthor = false; }
      catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
      refresh();
    };
    $("#wzClaudeGetKey").onclick = () => openKeysPage("#wzClaudeKey");
    $("#wzClaudeRemove").onclick = async (e) => {
      const yes = await LoreShell.ask(e.currentTarget, "從這台電腦的金鑰圈移除 Claude key？", [{ label: "取消", value: null }, { label: "移除", value: true, danger: true, primary: true }]);
      if (yes) saveClaude({ key: "" });
    };
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
    $("#ccCopy").onclick = () => { navigator.clipboard?.writeText(INSTALL); LoreShell.toast("已複製安裝指令"); };
    $("#wzTest").onclick = testClaude;
    $("#wzClaudeKey").onkeydown = (e) => { if (e.key === "Enter") $("#wzClaudeSave").click(); };
  }

  /** First run: no workspace yet. The gear opens on the first thing to do. */
  async function maybeOpen() {
    try {
      const s = await api("/api/setup/status");
      if (!s.first_run || store.get("pc.setupSeen", false)) return;
      st = s;
      const d = done();
      if (!d[0]) LoreShell.settings("sync");
      else if (!d[1] && !d[2]) LoreShell.settings("model");
      else return;
      render();
    } catch {}
  }

  return { open, model, wire, maybeOpen, shellParts, syncShown: () => renderSyncState() };
})();
