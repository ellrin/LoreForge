// New paper: the authors, then one of two starts.
//   作者        picked from the profile's authors.csv (new people are saved there)
//   跟 AI 討論  the paper is made on the plain article and 討論草稿 opens at
//               1 主題 (key value, hypothesis, …; the journal template comes
//               later, in its ★ 版型 step)
//   空白        pick the template, the paper is made on it, the editor opens
// The folder name sits in the footer, suggested (no key value or title is
// known yet at this point, so the suggestion is a dated name).
// Also here, for 討論草稿: the template list (templateList) and the
// tables-and-figures editor (floats) that the old wizard had.
const PCNew = (() => {
  let st = null;

  // Key value limit, same rule as the server: CJK characters + Latin words
  // against 50 when there is any CJK, else words against 30.
  const CJK = /[㐀-䶿一-鿿豈-﫿぀-ヿ가-힯]/g;
  function kvLength(s) {
    const cjk = (s.match(CJK) ?? []).length;
    const words = s.replace(CJK, " ").split(/[\s，。、；：！？（）「」,;:!?()]+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;
    return cjk ? [cjk + words, 50, "字"] : [words, 30, "個英文單字"];
  }

  async function open() {
    $("#fileMenu").hidden = true;
    st = { step: "authors", name: "", nameEdited: false, template: store.get("pc.newTemplate", ""), tpls: [], people: { authors: [], affiliations: [] }, chosen: [], busy: false };
    try { st.tpls = (await api("/api/templates")).filter((j) => j.kind !== "book"); } catch {}
    if (st.template && !st.tpls.some((t) => t.id === st.template)) st.template = "";
    await loadPeople();
    const d = $("#dlgNew");
    if (!d.open) d.showModal();
    draw();
  }

  async function loadPeople() {
    try { st.people = await api("/api/people"); } catch { st.people = { authors: [], affiliations: [] }; }
  }

  const affil = (id) => st.people.affiliations.find((f) => f.id === id);
  const person = (id) => st.people.authors.find((a) => a.id === id);
  const fullName = (a) => [a.given, a.family].filter(Boolean).join(" ");

  function draw() {
    const body = $("#npBody");
    body.innerHTML = "";
    if (st.step === "template") stepTemplate(body);
    else stepAuthors(body);
    const foot = $("#npFoot");
    foot.innerHTML = "";
    if (st.step === "template") { const b = el("button", "ghost", "上一步"); b.onclick = () => { st.step = "authors"; draw(); }; foot.append(b); }
    foot.append(el("span", "grow"));
    if (!st.nameEdited) st.name = autoName();
    const name = el("input", "np-input np-name");
    name.id = "npName";
    name.value = st.name;
    name.placeholder = "資料夾名稱";
    name.oninput = () => { st.name = name.value; st.nameEdited = true; };
    foot.append(name);
    if (st.step === "template") {
      const b = el("button", "primary small", st.busy ? "建立中…" : "建立論文");
      b.disabled = st.busy;
      b.onclick = () => create("blank");
      name.onkeydown = (e) => { if (e.key === "Enter") create("blank"); };
      foot.append(b);
      return;
    }
    const blank = el("button", "ghost", "空白");
    blank.disabled = st.busy;
    blank.onclick = () => { if (checkName()) { st.step = "template"; draw(); } };
    // what 討論草稿 needs, said before step 1: Claude, or else the local model
    const claude = typeof claudeReady === "function" && claudeReady();
    const local = !!state.engines?.local?.configured;
    const none = !!state.engines && !claude && !local;
    const ai = el("button", "primary small", st.busy ? "建立中…" : none ? "連接模型" : "跟 AI 討論");
    ai.prepend(icon("ai"));
    ai.disabled = st.busy;
    ai.onclick = () => (none ? Setup.open("claude") : create("ai"));
    name.onkeydown = (e) => { if (e.key === "Enter") ai.click(); };
    const note = state.engines && !claude ? el("span", "np-engine-note", none ? "沒有 Claude 也沒有本機模型" : "沒有 Claude：改用本機模型，品質較低") : null;
    if (note) foot.insertBefore(note, name);
    foot.append(blank, ai);
  }

  // A dated folder name (nothing else is known yet); never one that exists.
  function autoName() {
    const d = new Date();
    const base = `paper-${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;
    const taken = new Set(state.workspace?.papers ?? []);
    let name = base, n = 2;
    while (taken.has(name)) name = `${base}-${n++}`;
    return name;
  }

  function checkName() {
    if (st.name.trim()) return true;
    $("#npName")?.focus();
    LoreShell.toast("請填資料夾名稱");
    return false;
  }

  // ── 版型 ──
  /** The template list: search, 建議 first, then journals and conferences, each with a
   *  line of its 期刊寫法 (venue.js); base formats are not offered. Under the list, the
   *  chosen journal's 期刊寫法 panel; a name that is not there → 新增期刊.
   *  opts: tpls, selected ("" = the plain article), suggested (ids), onPick(id),
   *  paper (the paper's name, for its keywords; else the open paper), journalsOnly (no 空白). */
  function templateList(opts) {
    const box = el("div", "np-tpls");
    const q = el("input", "np-input");
    q.type = "search";
    q.placeholder = "搜尋期刊或研討會";
    const list = el("div", "tpl-list");
    const below = el("div", "np-venue");
    let selected = opts.selected ?? "";
    const venue = typeof PCVenue !== "undefined" ? PCVenue : null;
    const paper = () => opts.paper ?? state.paper?.name ?? null;
    const showPanel = () => {
      const t = opts.tpls.find((x) => x.id === selected);
      below.replaceChildren(...(venue && t && !t.hidden ? [venue.panel({ template: selected, paper: paper() })] : []));
    };
    const addJournal = (query) => {
      list.hidden = true;
      q.hidden = true;
      below.replaceChildren(venue.newJournal({
        query,
        onCancel: () => { list.hidden = false; q.hidden = false; showPanel(); },
        onDone: async (id) => {
          try { const all = await api("/api/templates"); opts.tpls.splice(0, opts.tpls.length, ...all.filter((j) => j.kind !== "book")); } catch {}
          list.hidden = false; q.hidden = false; q.value = "";
          selected = id; opts.onPick?.(id); fill(); showPanel();
        },
      }));
    };
    const fill = () => {
      list.innerHTML = "";
      const query = q.value.trim().toLowerCase();
      const option = (t, label, sub) => {
        const value = t?.id ?? "";
        const b = el("button", "tpl");
        b.append(el("span", "name", label));
        if (sub) b.append(el("span", "sub", sub));
        const habit = t && venue?.summary(t.id);
        if (habit) b.append(el("span", `habit${habit.stale ? " old" : ""}`, habit.summary));
        b.setAttribute("aria-pressed", String(value === selected));
        b.onclick = () => { selected = value; opts.onPick?.(value); fill(); showPanel(); setTimeout(() => below.scrollIntoView({ block: "nearest", behavior: "smooth" }), 250); };
        list.append(b);
      };
      const sub = (t) => [t.short, t.publisher].filter(Boolean).join(" · ");
      const hit = (t) => !query || `${t.name} ${t.short ?? ""} ${t.publisher ?? ""} ${t.id}`.toLowerCase().includes(query);
      // base formats and umbrella entries are not targets (the current one still shows)
      const offered = (t) => !t.hidden || t.id === selected;
      const sugg = (opts.suggested ?? []).map((id) => opts.tpls.find((t) => t.id === id)).filter((t) => t && hit(t) && offered(t));
      if (sugg.length) {
        list.append(el("div", "tpl-group", "建議"));
        sugg.forEach((t) => option(t, t.name, sub(t)));
      }
      if (!query && !opts.journalsOnly) option(null, "空白", "一般 article");
      let shown = sugg.length;
      [["journal", "期刊"], ["conference", "研討會"]].forEach(([k, title]) => {
        const group = opts.tpls.filter((t) => (t.kind === "conference" ? "conference" : "journal") === k && hit(t) && offered(t) && !sugg.includes(t)).sort((a, b) => a.name.localeCompare(b.name));
        if (!group.length) return;
        shown += group.length;
        list.append(el("div", "tpl-group", title));
        group.forEach((t) => option(t, t.name, sub(t)));
      });
      if (venue && query.length >= 2) {
        const add = el("button", "tpl tpl-add");
        add.append(el("span", "name", `新增期刊「${q.value.trim()}」`));
        add.prepend(icon("plus"));
        add.onclick = () => addJournal(q.value.trim());
        if (!shown) list.append(add); else { list.append(el("div", "tpl-group", "")); list.append(add); }
      }
    };
    q.oninput = fill;
    fill();
    venue?.loadSummaries().then(() => { if (box.isConnected || !list.hidden) fill(); });
    showPanel();
    box.append(q, list, below);
    setTimeout(() => q.focus());
    return box;
  }

  function stepTemplate(body) {
    body.append(label("版型"), templateList({ tpls: st.tpls, selected: st.template, onPick: (v) => { st.template = v; store.set("pc.newTemplate", v); } }));
  }

  // ── 作者 ──
  function stepAuthors(body) {
    const chosen = el("ol", "np-chosen");
    if (!st.chosen.length) chosen.append(el("li", "empty", "還沒有選作者"));
    st.chosen.forEach((c, i) => {
      const a = person(c.id);
      if (!a) return;
      const li = el("li");
      const who = el("div", "who");
      who.append(el("b", "", fullName(a)), el("span", "sub", a.affiliations.map((id) => affil(id)?.organization ?? id).join(" · ")));
      const toggle = (key, text) => {
        const t = el("button", "tchip", text);
        t.setAttribute("aria-pressed", String(!!c[key]));
        t.onclick = () => { c[key] = !c[key]; draw(); };
        return t;
      };
      const move = (d, ic, tip) => iconButton(ic, tip, () => { const j = i + d; if (j < 0 || j >= st.chosen.length) return; [st.chosen[i], st.chosen[j]] = [st.chosen[j], st.chosen[i]]; draw(); });
      const del = iconButton("x", "移除", () => { st.chosen.splice(i, 1); draw(); });
      li.append(el("span", "n", String(i + 1)), who, toggle("corresponding", "通訊作者"), toggle("equal", "同等貢獻"), move(-1, "up", "上移"), move(1, "chevron", "下移"), del);
      chosen.append(li);
    });

    const pick = el("div", "np-pick");
    const q = el("input", "np-input");
    q.type = "search";
    q.placeholder = "搜尋作者";
    const list = el("div", "np-people");
    const fill = () => {
      list.innerHTML = "";
      const query = q.value.trim().toLowerCase();
      const rest = st.people.authors.filter((a) => !st.chosen.some((c) => c.id === a.id))
        .filter((a) => !query || `${fullName(a)} ${a.role} ${a.email} ${a.affiliations.map((id) => affil(id)?.organization ?? "").join(" ")}`.toLowerCase().includes(query));
      if (!st.people.authors.length) list.append(el("p", "empty", "作者資料庫是空的，先新增一位"));
      rest.forEach((a) => {
        const b = el("button", "np-person");
        b.append(el("b", "", fullName(a)), el("span", "sub", [a.role, ...a.affiliations.map((id) => affil(id)?.organization ?? id)].filter(Boolean).join(" · ")));
        b.onclick = () => { st.chosen.push({ id: a.id, corresponding: !st.chosen.length, equal: false }); draw(); };
        list.append(b);
      });
    };
    q.oninput = fill;
    fill();
    const add = el("button", "ghost wide", "新增作者");
    add.prepend(icon("plus"));
    add.onclick = () => { add.replaceWith(personForm()); };
    pick.append(q, list, add);
    body.append(label("這篇的作者"), chosen, label("作者資料庫"), pick);
  }

  /** Inline form: a new person, with an existing or a new institution. */
  function personForm() {
    const f = el("div", "np-form");
    const input = (ph, req) => { const i = el("input", "np-input"); i.placeholder = ph + (req ? "（必填）" : ""); return i; };
    const given = input("名 given name"), family = input("姓 family name", true), email = input("email"), orcid = input("ORCID，例如 0000-0002-1825-0097"), role = input("角色，例如 指導教授、合作醫師");
    const sel = el("select", "np-input");
    sel.append(new Option("— 選擇機構 —", ""));
    st.people.affiliations.forEach((a) => sel.append(new Option([a.department, a.organization].filter(Boolean).join(", "), a.id)));
    sel.append(new Option("＋ 新增機構…", "@new"));
    const org = input("機構 organization", true), dept = input("系所 department"), city = input("城市 city"), country = input("國家 country");
    const newBox = el("div", "np-grid");
    newBox.append(org, dept, city, country);
    newBox.hidden = true;
    sel.onchange = () => { newBox.hidden = sel.value !== "@new"; };
    const save = el("button", "primary small", "存進資料庫並加入");
    const cancel = el("button", "ghost", "取消");
    cancel.onclick = draw;
    save.onclick = async () => {
      if (!family.value.trim()) return family.focus();
      if (!sel.value) return sel.focus();
      if (sel.value === "@new" && !org.value.trim()) return org.focus();
      save.disabled = true;
      try {
        const r = await api("/api/people", {
          author: { id: "", given: given.value.trim(), family: family.value.trim(), email: email.value.trim(), orcid: orcid.value.trim(), affiliations: sel.value === "@new" ? [] : [sel.value], role: role.value.trim(), note: "" },
          affiliation: sel.value === "@new" ? { id: "", department: dept.value.trim(), organization: org.value.trim(), city: city.value.trim(), country: country.value.trim() } : null,
        });
        await loadPeople();
        st.chosen.push({ id: r.id, corresponding: !st.chosen.length, equal: false });
        draw();
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); save.disabled = false; }
    };
    const grid = el("div", "np-grid");
    grid.append(given, family, email, orcid);
    const row = el("div", "np-row");
    row.append(cancel, save);
    f.append(grid, role, sel, newBox, row);
    setTimeout(() => given.focus());
    return f;
  }

  /** "ai": the plain article, then 討論草稿 at 1 主題. "blank": the chosen template, then the editor. */
  async function create(start) {
    if (st.busy || !checkName()) return;
    st.busy = true; draw();
    try {
      const template = start === "blank" ? st.template || null : null;
      // on a journal template: the sections in the journal's own arrangement (期刊寫法); none yet →
      // the template's sample, and the 期刊寫法 is made in the background
      let outline = [];
      if (template) {
        try { outline = (await api(`/api/venue/t/${encodeURIComponent(template)}/skeleton`)).outline ?? []; } catch {}
        if (!outline.length) api(`/api/venue/t/${encodeURIComponent(template)}/make`, {}).catch(() => {});
      }
      const r = await api("/api/papers", { name: st.name.trim(), template, key_value: "", authors: st.chosen, outline });
      st = null;
      $("#dlgNew").close();
      // a new paper is not an import: open it without the import message
      state.workspace = await api("/api/workspace");
      await openPaper(r.name);
      refreshGit();
      if (start === "ai") openWindow("dlgDraft");
    } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); if (st) { st.busy = false; draw(); } }
  }

  function label(text, tip) {
    const l = el("div", "np-label");
    l.append(el("span", "", text));
    if (tip) { const h = el("button", "help", "?"); h.dataset.tip = tip; l.append(h); }
    return l;
  }

  // ── tables and figures (討論草稿 step 2) ──
  // Results the user already has, pasted from a spreadsheet or picked as
  // image files, each put at the end of a chosen section in order; captions
  // are written by the engine when left empty.

  /** Rows of cells, the same way the server reads them: tabs, a Markdown table, or CSV. */
  function grid(text) {
    const lines = text.split(/\r?\n/).filter((l) => l.trim());
    if (!lines.length) return [];
    if (lines.every((l) => l.trim().startsWith("|"))) return lines.filter((l) => !/^[\s|:+-]+$/.test(l)).map((l) => l.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim()));
    if (lines.some((l) => l.includes("\t"))) return lines.map((l) => l.split("\t").map((c) => c.trim()));
    return lines.map((l) => {
      const out = []; let cur = "", q = false;
      for (let i = 0; i < l.length; i++) {
        const c = l[i];
        if (c === '"') { if (q && l[i + 1] === '"') { cur += '"'; i++; } else q = !q; }
        else if (c === "," && !q) { out.push(cur.trim()); cur = ""; }
        else cur += c;
      }
      out.push(cur.trim());
      return out;
    });
  }

  function pickImage(done) {
    const i = el("input");
    i.type = "file";
    i.accept = ".png,.jpg,.jpeg,.pdf,.eps";
    i.onchange = () => i.files[0] && done(i.files[0]);
    i.click();
  }

  /** The editor. opts: paper, sections (titles), styles, keyValue, title,
   *  figuresBefore (figures the paper has, so names do not clash), onSaved(view). */
  function floats(opts) {
    const tables = [];
    const box = el("div", "np-floats");
    const sections = opts.sections.length ? opts.sections : ["Results"];
    const def = () => sections.find((s) => /result/i.test(s)) ?? sections[sections.length - 1];
    let busy = false;

    /** figures/ name: fig3-roc-curves.png, plain characters only. */
    function figureFile(t) {
      const n = (opts.figuresBefore ?? 0) + tables.filter((x) => x.kind === "figure").indexOf(t) + 1;
      const m = t.file.name.match(/^(.*?)(?:\.([^.]+))?$/);
      const stem = (m[1] ?? "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40);
      return `fig${n}${stem ? `-${stem}` : ""}.${(m[2] ?? "png").toLowerCase()}`;
    }

    /** Ask the writing engine for a caption when the user left it empty. */
    function autoCaption(t, force) {
      if (t.capJob) return t.capJob;
      if ((t.caption.trim() && !force) || !chosenEngine()) return;
      if (t.kind !== "figure" && grid(t.data).length < 2) return;
      t.capBusy = true; repaintCaption(t);
      t.capJob = api("/api/wizard/caption", { kind: t.kind, data: t.data ?? "", file: t.kind === "figure" ? t.file.name : "", section: t.section, key_value: opts.keyValue ?? "", title: opts.title ?? "", engine: chosenEngine() })
        .then((r) => { if (r.caption && (!t.caption.trim() || force)) t.caption = r.caption; else if (force && !r.caption) LoreShell.toast("從檔名看不出這張圖的內容，請自己填標題"); })
        .catch((e) => LoreShell.toast(e.message, { err: true, ms: 5000 }))
        .finally(() => { t.capBusy = false; t.capJob = null; repaintCaption(t); });
      return t.capJob;
    }
    const repaintCaption = (t) => {
      const cap = t.capEl;
      if (!cap?.isConnected) return;
      cap.value = t.caption;
      cap.disabled = !!t.capBusy;
      cap.placeholder = t.capBusy ? "產生標題中…" : cap.dataset.ph;
    };

    function captionInput(t, ph) {
      const cap = el("input", "np-input");
      cap.dataset.ph = ph;
      cap.oninput = () => (t.caption = cap.value);
      t.capEl = cap;
      repaintCaption(t);
      const gen = iconButton("ai", chosenEngine() ? "自動產生標題" : "連接寫作引擎後可自動產生", () => autoCaption(t, true));
      gen.disabled = !chosenEngine();
      return [cap, gen];
    }

    function sectionSelect(t) {
      const sec = el("select", "np-input");
      if (!sections.includes(t.section)) t.section = def();
      sections.forEach((s) => sec.append(new Option(s, s)));
      sec.value = t.section;
      sec.onchange = () => (t.section = sec.value);
      return sec;
    }

    const moveButtons = (i) => {
      const move = (d, ic, tip) => iconButton(ic, tip, () => { const j = i + d; if (j < 0 || j >= tables.length) return; [tables[i], tables[j]] = [tables[j], tables[i]]; render(); });
      return [move(-1, "up", "上移"), move(1, "chevron", "下移"), iconButton("x", "移除", () => { tables.splice(i, 1); render(); })];
    };

    function figureCard(t, i) {
      const c = el("div", "np-table");
      const head = el("div", "np-row start");
      head.append(el("b", "np-tn", `圖 ${tables.slice(0, i + 1).filter((x) => x.kind === "figure").length}`));
      head.append(...captionInput(t, "標題（caption），留白會自動產生"), ...moveButtons(i));
      const where = el("div", "np-grid");
      const change = el("button", "ghost", t.file.name);
      change.title = "換一張";
      change.onclick = () => pickImage((f) => { t.file = f; render(); });
      where.append(sectionSelect(t), change);
      const prev = el("div", "np-preview");
      if (/\.(png|jpe?g)$/i.test(t.file.name)) {
        const img = el("img", "np-fig");
        img.src = URL.createObjectURL(t.file);
        prev.append(img);
      }
      c.append(head, where, prev);
      return c;
    }

    function tableCard(t, i) {
      const c = el("div", "np-table");
      const head = el("div", "np-row start");
      head.append(el("b", "np-tn", `表 ${tables.slice(0, i + 1).filter((x) => x.kind !== "figure").length}`));
      const [cap, gen] = captionInput(t, "標題（caption），留白會自動產生");
      const sty = el("select", "np-input");
      sty.append(new Option("期刊預設樣式", ""));
      (opts.styles ?? []).forEach((s) => sty.append(new Option(s.name, s.id)));
      sty.value = t.style;
      sty.onchange = () => (t.style = sty.value);
      head.append(cap, gen, ...moveButtons(i));
      const where = el("div", "np-grid");
      where.append(sectionSelect(t), sty);
      const data = el("textarea", "np-input data");
      data.rows = 5;
      data.placeholder = "Model\tAUC\tSensitivity\nResNet50\t0.91\t0.85\n…（從試算表複製貼上）";
      data.value = t.data;
      const prev = el("div", "np-preview");
      const show = () => {
        prev.innerHTML = "";
        const g = grid(data.value);
        if (!g.length) return;
        const cols = Math.max(...g.map((r) => r.length));
        prev.append(el("span", "np-note", `${g.length - 1} 列資料 × ${cols} 欄（第一列為表頭）`));
        const tb = el("table");
        g.slice(0, 5).forEach((r, ri) => { const tr = el("tr"); for (let k = 0; k < cols; k++) tr.append(el(ri ? "td" : "th", "", r[k] ?? "")); tb.append(tr); });
        prev.append(tb);
        if (g.length > 5) prev.append(el("span", "np-note", `…還有 ${g.length - 5} 列`));
      };
      data.oninput = () => { t.data = data.value; show(); };
      data.onblur = () => autoCaption(t);
      show();
      c.append(head, where, data, prev);
      return c;
    }

    async function save() {
      const list = tables.filter((t) => t.kind === "figure" || t.data.trim());
      if (!list.length || busy) return;
      if (list.some((t) => t.kind !== "figure" && grid(t.data).length < 2)) { LoreShell.toast("表格至少要有表頭和一列資料"); return; }
      busy = true; render();
      try {
        await Promise.all(list.map((t) => autoCaption(t)));
        const base = `/api/papers/${encodeURIComponent(opts.paper)}`;
        for (const t of list.filter((x) => x.kind === "figure")) {
          const up = await fetch(`${base}/figures?file=${encodeURIComponent(figureFile(t))}`, { method: "POST", body: t.file });
          if (!up.ok) throw new Error(`${t.file.name}：${await up.text()}`);
        }
        const v = await api(`${base}/draft/floats`, {
          tables: list.map((t) => (t.kind === "figure"
            ? { kind: "figure", caption: t.caption, section: t.section, file: figureFile(t) }
            : { caption: t.caption, section: t.section, data: t.data, style: t.style || null })),
        });
        tables.length = 0;
        opts.onSaved?.(v);
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
      busy = false; render();
    }

    function render() {
      box.innerHTML = "";
      tables.forEach((t, i) => box.append(t.kind === "figure" ? figureCard(t, i) : tableCard(t, i)));
      const addT = el("button", "ghost np-add", "新增表格");
      addT.prepend(icon("plus"));
      addT.onclick = () => { tables.push({ kind: "table", caption: "", section: def(), data: "", style: "" }); render(); setTimeout(() => [...box.querySelectorAll(".np-table textarea")].pop()?.focus()); };
      const addF = el("button", "ghost np-add", "新增圖片");
      addF.prepend(icon("image"));
      addF.onclick = () => pickImage((f) => { tables.push({ kind: "figure", caption: "", section: def(), file: f }); render(); autoCaption(tables[tables.length - 1]); });
      const row = el("div", "np-row start");
      row.append(addT, addF);
      if (tables.length) {
        const put = el("button", "primary small", busy ? "放進論文中…" : "放進論文");
        put.disabled = busy;
        put.onclick = save;
        row.append(el("span", "grow"), put);
      }
      box.append(row);
    }
    render();
    return { el: box, pending: () => tables.length > 0 };
  }

  function wire() {}

  /** Has the user filled anything in that closing would lose? */
  const dirty = () => !!st && !st.busy && !!(st.nameEdited || st.chosen.length);

  // ── Key value check (pc_core::thesis): advice under the field, never a gate ──
  /** Ask about `value` and show the problems and rewrites in `box`; a rewrite clicked → onPick(text).
   *  `current()` (optional): the field's value now — stale answers are dropped. */
  async function kvAdvice(value, box, onPick, current) {
    const v = (value ?? "").trim();
    box.replaceChildren();
    box.hidden = true;
    if (!v) return;
    const seq = (box.seq = (box.seq ?? 0) + 1);
    let r;
    try { r = await api("/api/keyvalue/check", { key_value: v, engine: typeof chosenEngine === "function" ? chosenEngine() : null }); }
    catch { return; }
    if (box.seq !== seq || (current && current().trim() !== v)) return;
    if (!r?.problems?.length && !r?.rewrites?.length) return;
    box.append(...(r.problems ?? []).map((p) => el("span", "kv-problem", p.label)));
    (r.rewrites ?? []).forEach((t) => {
      const b = el("button", "ghost small kv-rewrite", t);
      b.title = "用這句";
      b.onclick = () => { box.replaceChildren(); box.hidden = true; onPick(t); };
      box.append(b);
    });
    box.hidden = false;
  }

  /** Wire a Key value input: checked on blur (debounced) and by its 檢查 button; returns the box. */
  function kvCheck(input, onPick = () => {}) {
    const box = el("div", "kv-advice");
    box.hidden = true;
    const btn = el("button", "ghost small kv-check", "檢查");
    btn.type = "button";
    let timer = 0, last = "";
    const run = (force) => {
      clearTimeout(timer);
      const v = input.value.trim();
      if (!force && v === last) return;
      timer = setTimeout(() => { last = v; kvAdvice(v, box, pick, () => input.value); }, force ? 0 : 400);
    };
    const pick = (t) => { input.value = t; last = t.trim(); input.dispatchEvent(new Event("input")); onPick(t); };
    input.addEventListener("blur", () => run(false));
    btn.onclick = () => run(true);
    input.after(btn, box);
    return box;
  }

  return { open, wire, kvLength, dirty, templateList, floats, kvAdvice, kvCheck };
})();
