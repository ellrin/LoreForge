// 新增期刊 from what the author has: drop the journal's LaTeX template
// (.zip or folder) or Word template (.docx), give the author-guidelines URL,
// review every rule with the sentence it came from, then 建立. Nothing is
// written to the workspace before 建立 (the upload waits in the cache).
// Server side: crates/server/src/newtemplate.rs.

const PCNewTemplate = (() => {
  // its styles (and those of the editor's added rule fields) come with it
  document.head.append(Object.assign(document.createElement("link"), { rel: "stylesheet", href: "newtemplate.css" }));

  const field = (label, control) => { const l = el("label", "tf"); l.append(el("span", "lbl", label), control); return l; };
  const input = (value = "", placeholder = "") => { const i = el("input"); i.value = value; i.placeholder = placeholder; return i; };
  let job = null; // staging id
  let found = null; // what the upload is (GET /api/newtemplate/{id})
  let read = null; // the last reading of the guidelines

  /** The engine the reading runs on: Claude when it is set up, else local. */
  const engine = () => {
    try { if (claudeReady()) return claudeVia(); } catch {}
    return state.engines?.local?.configured ? "local" : null;
  };
  const engineName = () => { const e = engine(); return e === "local" ? "本機" : e ? "Claude" : ""; };

  /** Every file of a drop, folders walked: [{ file, path }]. */
  async function dropped(dt) {
    const out = [];
    const walk = async (entry, prefix) => {
      if (entry.isFile) {
        const file = await new Promise((ok, no) => entry.file(ok, no));
        out.push({ file, path: prefix ? `${prefix}${entry.name}` : null });
      } else if (entry.isDirectory) {
        const reader = entry.createReader();
        for (;;) {
          const batch = await new Promise((ok, no) => reader.readEntries(ok, no));
          if (!batch.length) break;
          for (const e of batch) await walk(e, `${prefix}${entry.name}/`);
        }
      }
    };
    const entries = [...(dt.items ?? [])].map((i) => i.webkitGetAsEntry?.()).filter(Boolean);
    if (entries.length) for (const e of entries) await walk(e, "");
    else for (const f of dt.files ?? []) out.push({ file: f, path: null });
    // several loose files are one template: keep each under its own name
    if (out.length > 1) out.forEach((o) => { o.path ??= o.file.name; });
    return out;
  }

  function open(host, opts) {
    const { kind, rulesSection, fromExisting, done } = opts;
    job = null; found = null; read = null;
    host.innerHTML = "";

    const head = el("div", "tpl-head");
    head.append(el("h3", "", kind === "conference" ? "新增研討會" : "新增期刊"));
    if (fromExisting) {
      const alt = el("button", "ghost small nt-switch", "從現有模板複製");
      alt.prepend(icon("copy"));
      alt.onclick = fromExisting;
      head.append(alt);
    }

    // the drop zone
    const drop = el("div", "nt-drop");
    drop.tabIndex = 0;
    const dropIcon = el("div", "nt-drop-icon");
    dropIcon.append(icon("upload"));
    const dropText = el("div", "nt-drop-text");
    dropText.append(el("b", "", "LaTeX 模板（.zip、資料夾）或 Word 範本（.docx）"));
    const pickers = el("div", "nt-pick");
    const pickFile = el("button", "ghost small", "選擇檔案");
    const pickDir = el("button", "ghost small", "選擇資料夾");
    pickers.append(pickFile, pickDir);
    dropText.append(pickers);
    drop.append(dropIcon, dropText);
    const state_ = el("div", "nt-found");

    const chooser = (dir) => {
      const i = el("input");
      i.type = "file";
      if (dir) i.webkitdirectory = true;
      else { i.accept = ".zip,.tar.gz,.tgz,.docx,.dotx,.tex,.cls,.sty,.bst,.bib"; i.multiple = true; }
      i.onchange = () => {
        const files = [...i.files];
        const items = files.map((f) => ({ file: f, path: dir ? f.webkitRelativePath : files.length > 1 ? f.name : null }));
        if (items.length) upload(items);
      };
      i.click();
    };
    pickFile.onclick = (e) => { e.stopPropagation(); chooser(false); };
    pickDir.onclick = (e) => { e.stopPropagation(); chooser(true); };
    drop.onclick = () => chooser(false);
    drop.onkeydown = (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); chooser(false); } };
    drop.ondragover = (e) => { e.preventDefault(); drop.classList.add("over"); };
    drop.ondragleave = () => drop.classList.remove("over");
    drop.ondrop = async (e) => {
      e.preventDefault();
      drop.classList.remove("over");
      const items = await dropped(e.dataTransfer);
      if (items.length) upload(items);
    };

    // the journal
    const name = input("", "Journal of …");
    const short = input("", "簡稱");
    const url = input("", "https://…");
    url.type = "url";
    const readBtn = el("button", "ghost small nt-read-btn", "讀取規則");
    const badge = el("span", "cv-badge", engineName());
    if (engineName()) readBtn.append(badge);
    const urlRow = el("div", "nt-url");
    urlRow.append(url, readBtn);
    const form = el("div", "tpl-form");
    const urlField = field("作者須知", urlRow);
    urlField.classList.add("wide");
    form.append(field("名稱", name), field("簡稱", short), urlField);
    const readState = el("div", "nt-readstate");
    const paste = el("div", "nt-paste");
    paste.hidden = true;
    const pasteText = el("textarea");
    pasteText.rows = 5;
    pasteText.placeholder = "貼上作者須知的文字";
    const pasteGo = el("button", "ghost small", "讀取");
    paste.append(pasteText, pasteGo);

    const rulesBox = el("div", "nt-rules");
    let rules = null;
    const out = el("div", "tpl-out");
    const go = el("button", "primary small", "建立");
    const acts = el("div", "tpl-actions");
    acts.append(go, out);
    host.append(head, drop, state_, form, readState, paste, rulesBox, acts);

    /** One rule into the card's values `j` and sources `src`. */
    const set = (j, src, r) => {
      const v = r.value;
      if (r.key === "name") { if (!name.value.trim()) name.value = v; return; }
      if (r.key === "word_limits.sections") {
        j.word_limits = { ...(j.word_limits ?? {}), sections: { ...(j.word_limits?.sections ?? {}), [r.section]: +v } };
        src[`word_limits.sections.${r.section}`] = { quote: r.quote, from: r.from };
        return;
      }
      const val = /^(limits|word_limits)\./.test(r.key) && r.key !== "limits.anonymous" ? +v : v === "yes" ? true : v;
      const [a, b, c] = r.key.split(".");
      if (!b) j[a] = val;
      else {
        j[a] = { ...(j[a] ?? {}) };
        if (!c) j[a][b] = val;
        else j[a][b] = { ...(j[a][b] ?? {}), [c]: val };
      }
      src[r.key] = { quote: r.quote, from: r.from };
    };
    let shown = {}; // the sources the card shows, with where each came from

    /** Rebuild the card with `apply` on top: from nothing (`reset`), or from what it holds now. */
    function rebuild(apply, reset) {
      let j = {};
      let src = {};
      if (rules && !reset) {
        const v = rules.value();
        j = { ...v };
        src = Object.fromEntries(Object.entries(v.sources ?? {}).map(([k, q]) => [k, shown[k] ?? { quote: q }]));
      }
      apply.forEach((r) => set(j, src, r));
      shown = src;
      rules = rulesSection({ ...j, cleveref: false }, src);
      rulesBox.innerHTML = "";
      rulesBox.append(rules.node);
    }

    function paintFound() {
      state_.innerHTML = "";
      drop.classList.toggle("has", !!found && found.source !== "none");
      if (!found) return;
      if (found.error) { state_.append(el("div", "cv-state bad", found.error)); return; }
      if (found.source === "none") return;
      const what = found.source === "latex" ? "LaTeX" : "Word";
      const cls = found.class ? `${found.class}${found.family ? `（${found.family}）` : ""}` : "";
      state_.append(el("div", "cv-state ok", [what, found.main, cls, `${found.files.length} 個檔案`].filter(Boolean).join(" · ")));
      if (found.missing?.length) state_.append(el("div", "cv-state bad", `缺少：${found.missing.join("、")}`));
      for (const n of found.notation ?? []) state_.append(el("div", "cv-state bad", n));
    }

    async function ensureJob() {
      if (!job) job = (await api("/api/newtemplate", {})).id;
      return job;
    }

    async function upload(items) {
      state_.innerHTML = "";
      state_.append(el("div", "cv-state", "上傳中…"));
      try {
        job = (await api("/api/newtemplate", {})).id;
        for (const { file, path } of items) {
          const q = `name=${encodeURIComponent(file.name)}${path ? `&path=${encodeURIComponent(path)}` : ""}`;
          const r = await fetch(`/api/newtemplate/${job}/files?${q}`, { method: "POST", body: file });
          if (!r.ok) throw new Error(await r.text());
        }
        state_.innerHTML = "";
        state_.append(el("div", "cv-state", items.some((i) => /\.(docx|dotx)$/i.test(i.file.name)) ? "轉換 Word…" : "讀取中…"));
        found = await api(`/api/newtemplate/${job}`);
        if (found.name && !name.value.trim()) name.value = found.name;
      } catch (e) { found = { error: e.message }; }
      paintFound();
      rebuild([...(found?.defaults ?? []), ...(read?.rules ?? [])], true);
      paint();
    }

    async function readRules(text) {
      readBtn.disabled = pasteGo.disabled = true;
      readState.className = "nt-readstate";
      readState.textContent = "讀取中…";
      try {
        await ensureJob();
        const body = { url: url.value.trim(), text: text ?? "" };
        const e = engine();
        if (e) body.engine = e;
        read = await api(`/api/newtemplate/${job}/rules`, body);
        const n = read.rules.length;
        const bits = [];
        if (read.fetch_error) bits.push(read.fetch_error);
        else if (read.chars) bits.push(`${read.title || "作者須知"} · ${read.chars.toLocaleString()} 字`);
        bits.push(n ? `讀到 ${n} 項` : "沒有讀到規則");
        if (read.engine) bits.push(read.engine);
        readState.textContent = bits.join(" · ");
        readState.className = `nt-readstate ${read.fetch_error ? "bad" : n ? "ok" : ""}`;
        paste.hidden = !read.fetch_error;
        if (text) paste.hidden = true;
        rebuild(read.rules, false);
      } catch (err) {
        readState.textContent = err.message;
        readState.className = "nt-readstate bad";
        paste.hidden = false;
      }
      readBtn.disabled = pasteGo.disabled = false;
      paint();
    }
    readBtn.onclick = () => readRules();
    url.onkeydown = (e) => { if (e.key === "Enter") readRules(); };
    pasteGo.onclick = () => { if (pasteText.value.trim()) readRules(pasteText.value); };

    function paint() {
      const ready = found && !found.error && found.source !== "none";
      go.disabled = !ready;
      readBtn.disabled = !url.value.trim() && !ready;
    }
    url.oninput = paint;

    go.onclick = async () => {
      if (!name.value.trim()) { name.focus(); return; }
      go.disabled = true;
      out.textContent = "";
      try {
        const v = rules.value();
        const meta = {
          name: name.value.trim(), short: short.value.trim(), kind, url: url.value.trim(),
          ...v,
        };
        const j = await api(`/api/newtemplate/${job}/create`, meta);
        LoreShell.toast("已建立模板");
        await done(j.id);
      } catch (e) { out.textContent = e.message; out.className = "tpl-out bad"; go.disabled = false; }
    };

    rebuild([], true);
    paint();
  }

  return { open };
})();
