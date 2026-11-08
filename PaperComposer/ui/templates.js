// Journal templates: a searchable list of journals on the left, the chosen
// journal on the right. A new journal starts as a copy of another and is
// tuned here: its name, class options, bibliography style, rules, sample.

const PCTemplates = (() => {
  let list = [];
  let current = null;
  let query = "";
  let kind = null; // journal | conference, remembered per viewer
  const KINDS = [["journal", "期刊"], ["conference", "研討會"]];
  const kindOf = (j) => (j.kind === "conference" ? "conference" : "journal");

  const field = (label, input, tip) => {
    const l = el("label", "tf");
    const head = el("span", "lbl", label);
    if (tip) { const h = el("button", "help", "?"); h.type = "button"; h.dataset.tip = tip; head.append(h); }
    l.append(head, input);
    return l;
  };
  const input = (value = "", placeholder = "") => { const i = el("input"); i.value = value; i.placeholder = placeholder; return i; };

  async function open(select) {
    kind ??= store.get("pc.tplKind", "journal");
    try { list = await api("/api/templates"); } catch (e) { list = []; flash(e.message, 5000); }
    const picked = select && list.find((j) => j.id === select);
    if (picked) { current = picked.id; kind = kindOf(picked); store.set("pc.tplKind", kind); store.set("pc.template", picked.id); }
    if (current && !list.some((j) => j.id === current)) current = null;
    current ??= store.get("pc.template", list[0]?.id ?? null);
    draw();
  }

  async function refreshWorkspace() {
    try { state.workspace = await api("/api/workspace"); } catch {}
  }

  function draw() {
    const box = $("#publishers");
    box.innerHTML = "";
    const side = el("div", "tpl-side");
    const tabs = el("div", "seg tpl-kinds");
    KINDS.forEach(([k, label]) => {
      const b = el("button", "", `${label} ${list.filter((j) => kindOf(j) === k).length}`);
      b.setAttribute("aria-pressed", String(k === kind));
      b.onclick = () => { kind = k; store.set("pc.tplKind", k); draw(); };
      tabs.append(b);
    });
    const search = el("input", "search");
    search.type = "search";
    search.placeholder = kind === "conference" ? "搜尋研討會" : "搜尋期刊";
    search.value = query;
    const ul = el("div", "tpl-items");
    const add = el("button", "ghost wide", kind === "conference" ? "新增研討會" : "新增期刊");
    add.prepend(icon("plus"));
    add.onclick = () => { current = null; newForm(); paintList(); };
    const paintList = () => {
      ul.innerHTML = "";
      const q = query.trim().toLowerCase();
      const shown = list.filter((j) => kindOf(j) === kind && (!q || `${j.name} ${j.short ?? ""} ${j.publisher ?? ""} ${j.id} ${j.class}`.toLowerCase().includes(q)));
      if (!shown.length) ul.append(el("p", "empty", q ? "沒有符合的模板" : "還沒有模板"));
      shown.forEach((j) => {
        const b = el("button", "tpl-item");
        b.append(el("span", "name", j.name), el("span", "sub", [j.short, j.publisher, j.class].filter(Boolean).join(" · ")));
        b.setAttribute("aria-current", String(j.id === current));
        b.onclick = () => { current = j.id; store.set("pc.template", j.id); paintList(); detail(); };
        ul.append(b);
      });
    };
    search.oninput = () => { query = search.value; paintList(); };
    const conv = el("button", "ghost wide cv-open", "上傳模板");
    conv.prepend(icon("upload"));
    conv.append(el("span", "cv-badge", "Claude"));
    conv.onclick = () => { current = null; paintList(); PCConvert.open($("#tplMain")); };
    const row = el("div", "tpl-adds");
    row.append(add, conv);
    side.append(tabs, search, row, ul);
    const main = el("div", "tpl-main");
    main.id = "tplMain";
    box.append(side, main);
    paintList();
    if (current && kindOf(list.find((j) => j.id === current) ?? {}) !== kind) current = list.find((j) => kindOf(j) === kind)?.id ?? null;
    if (current) detail(); else if (!list.length) main.append(el("p", "empty", "把期刊的模板資料夾放進工作區的 publishers/，或從現有模板新增。"));
    else newForm();
  }

  function detail() {
    const main = $("#tplMain");
    const j = list.find((x) => x.id === current);
    main.innerHTML = "";
    if (!j) return;
    const name = input(j.name, "期刊全名");
    const short = input(j.short ?? "", "TMI");
    const publisher = input(j.publisher ?? "", "IEEE");
    const url = input(j.url ?? "", "https://…");
    const opts = input(j.class_options, "journal");
    const style = el("select");
    const styles = [...new Set([...(j.bib_styles ?? []), j.bib_style].filter(Boolean))];
    if (!j.bib_style) style.append(new Option("（範本未指定）", ""));
    styles.forEach((s) => style.append(new Option(s, s)));
    style.value = j.bib_style;
    const kindSel = el("select");
    KINDS.forEach(([k, label]) => kindSel.append(new Option(label, k)));
    kindSel.value = kindOf(j);
    const notes = el("textarea");
    notes.rows = 6;
    notes.value = j.notes ?? "";
    notes.placeholder = "字數限制、圖檔格式、參考文獻規定…";

    const head = el("div", "tpl-head");
    head.append(el("h3", "", j.name));
    if (j.url) {
      const go = iconButton("external", "作者須知", () => api("/api/open-link", { url: j.url }).catch((e) => flash(e.message)));
      head.append(go);
    }
    const sub = el("div", "tpl-sub", [`publishers/${j.id}`, j.based_on ? `衍生自 ${j.based_on}` : ""].filter(Boolean).join(" · "));

    const grid = el("div", "tpl-form");
    grid.append(
      field("名稱", name),
      field("簡稱", short),
      field("出版社／主辦", publisher),
      field("類別", kindSel),
      field("作者須知", url),
      field(`\\documentclass 選項（${j.class || "—"}）`, opts, "寫在 sample.tex 第一個 \\documentclass[…] 裡，例如 journal、twocolumn、referee。"),
      field("文獻格式", style, "模板資料夾裡的 .bst。Springer Nature 以 class 選項指定（例如 sn-vancouver-num）。"),
    );
    const notesField = field("投稿規定", notes);
    notesField.classList.add("wide");
    grid.append(notesField);

    const out = el("div", "tpl-out");
    const save = el("button", "primary small", "儲存");
    save.onclick = async () => {
      save.disabled = true;
      try {
        const r = await api(`/api/templates/${encodeURIComponent(j.id)}`, {
          name: name.value, short: short.value, publisher: publisher.value, kind: kindSel.value, url: url.value, notes: notes.value,
          class_options: opts.value, bib_style: style.value,
        }, "PUT");
        Object.assign(j, r);
        flash("已儲存模板");
        await refreshWorkspace();
        draw();
        refreshGit();
      } catch (e) { flash(e.message, 6000); }
      save.disabled = false;
    };
    const check = el("button", "ghost", "試編譯");
    check.onclick = async () => {
      check.disabled = true;
      out.textContent = "編譯中…";
      try {
        const r = await api(`/api/templates/${encodeURIComponent(j.id)}/check`, {});
        out.textContent = r.ok ? `可以編譯 · ${r.pages} 頁 · ${r.seconds} 秒` : `無法編譯：${r.errors?.[0]?.message ?? ""}${r.errors?.[0]?.hint ? `（${r.errors[0].hint}）` : ""}`;
        out.className = `tpl-out ${r.ok ? "ok" : "bad"}`;
      } catch (e) { out.textContent = e.message; }
      check.disabled = false;
    };
    const sample = el("button", "ghost", "編輯範本…");
    sample.onclick = () => editSample(j);
    const use = el("button", "ghost", "用這個模板建立論文");
    use.onclick = () => usePaper(j, main);
    const acts = el("div", "tpl-actions");
    acts.append(save, check, sample, use, out);
    main.append(head, sub, grid, acts);
  }

  function newForm() {
    const main = $("#tplMain");
    main.innerHTML = "";
    main.append(el("h3", "", kind === "conference" ? "新增研討會" : "新增期刊"));
    const name = input("", "IEEE Transactions on Medical Imaging");
    const short = input("", "TMI");
    const from = el("select");
    list.forEach((j) => from.append(new Option(`${j.name}${j.short ? ` (${j.short})` : ""}`, j.id)));
    const grid = el("div", "tpl-form");
    grid.append(field("名稱", name), field("簡稱", short), field("以這個模板為基礎", from, "複製這個模板的 class、樣式檔與範本，之後在這裡微調；原本的模板不受影響。"));
    const go = el("button", "primary small", "建立");
    go.onclick = async () => {
      if (!name.value.trim()) { name.focus(); return; }
      go.disabled = true;
      try {
        const j = await api("/api/templates", { name: name.value, short: short.value, kind, from: from.value });
        current = j.id;
        store.set("pc.template", j.id);
        await refreshWorkspace();
        await open();
        refreshGit();
      } catch (e) { flash(e.message, 6000); go.disabled = false; }
    };
    const acts = el("div", "tpl-actions");
    acts.append(go);
    main.append(grid, acts);
    if (!list.length) { go.disabled = true; main.append(el("p", "empty", "先放一個模板資料夾進 publishers/，才能從它衍生。")); }
    name.focus();
  }

  async function editSample(j) {
    const main = $("#tplMain");
    let text;
    try { text = await fetch(`/api/templates/${encodeURIComponent(j.id)}/sample`).then((r) => (r.ok ? r.text() : Promise.reject(new Error("讀取失敗")))); }
    catch (e) { flash(e.message); return; }
    main.innerHTML = "";
    const head = el("div", "tpl-head");
    head.append(el("h3", "", `${j.name} · sample.tex`));
    const ta = el("textarea", "tpl-sample");
    ta.value = text;
    ta.spellcheck = false;
    const save = el("button", "primary small", "儲存");
    save.onclick = async () => {
      try { await api(`/api/templates/${encodeURIComponent(j.id)}/sample`, { text: ta.value }, "PUT"); flash("已儲存範本"); await open(); refreshGit(); }
      catch (e) { flash(e.message, 6000); }
    };
    const back = el("button", "ghost", "取消");
    back.onclick = detail;
    const acts = el("div", "tpl-actions");
    acts.append(save, back);
    main.append(head, ta, acts);
  }

  function usePaper(j, main) {
    const row = el("div", "tpl-actions");
    const n = input("", "論文名稱");
    const go = el("button", "primary small", "建立");
    const make = async () => {
      if (!n.value.trim()) { n.focus(); return; }
      try {
        await api("/api/papers", { name: n.value.trim(), template: j.id });
        $("#dlgTpl").close();
        await opened(n.value.trim());
      } catch (e) { flash(e.message, 6000); }
    };
    go.onclick = make;
    n.onkeydown = (e) => { if (e.key === "Enter") make(); };
    row.append(n, go);
    main.append(row);
    n.focus();
  }

  return { open };
})();
