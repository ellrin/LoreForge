// New paper: a five-step window.
//   01 template
//   02 authors, picked from the profile's authors.csv (new people are saved there)
//   03 key value (required) and, optionally, a bullet outline that Claude
//      turns into title suggestions and a section skeleton
//   04 tables and figures (optional): results the user already has, pasted
//      from a spreadsheet or picked as image files, each put in a chosen
//      section in order; captions are written by the engine when left empty
//   05 references (optional): a pasted list of papers to cite, split by
//      Claude, checked against the databases and filed one by one
// The folder name is asked last, suggested from the title or key value.
// The outline and the reference list need Claude; everything else works without it.
const PCNew = (() => {
  let st = null;

  // Key value limit, same rule as the server: CJK characters + Latin words
  // against 50 when there is any CJK, else words against 30.
  const CJK = /[㐀-䶿一-鿿豈-﫿぀-ヿ가-힯]/g;
  function kvLength(s) {
    const cjk = (s.match(CJK) ?? []).length;
    const words = s.replace(CJK, " ").split(/[\s，。、；：！？（）「」,;:!?()]+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;
    return cjk ? [cjk + words, 50, "字"] : [words, 30, "個英文單字"];
  }

  async function open() {
    $("#fileMenu").hidden = true;
    st = { step: 0, name: "", nameEdited: false, template: store.get("pc.newTemplate", ""), tpls: [], people: { authors: [], affiliations: [] }, chosen: [], key: "", outline: "", plan: null, usePlan: true, title: "", busy: false, tables: [], styles: [], sampleFor: null, sampleSections: [], refs: null, drawId: 0 };
    try { st.tpls = await api("/api/templates"); } catch {}
    try { st.styles = await api("/api/textable/styles"); } catch {}
    await PCRefs.load();
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
    document.querySelectorAll("#npTabs button").forEach((b) => b.setAttribute("aria-selected", String(+b.dataset.step === st.step)));
    const body = $("#npBody");
    body.innerHTML = "";
    st.drawId++;
    [stepTemplate, stepAuthors, stepKey, stepTables, stepRefs][st.step](body);
    const foot = $("#npFoot");
    foot.innerHTML = "";
    if (st.step > 0) { const b = el("button", "ghost", "上一步"); b.onclick = () => go(st.step - 1); foot.append(b); }
    foot.append(el("span", "grow"));
    if (st.step < 4) { const b = el("button", "primary small", "下一步"); b.onclick = () => go(st.step + 1); foot.append(b); return; }
    // last step: the folder name, then create
    if (!st.nameEdited) st.name = autoName();
    const name = el("input", "np-input np-name");
    name.id = "npName";
    name.value = st.name;
    name.placeholder = "資料夾名稱";
    name.oninput = () => { st.name = name.value; st.nameEdited = true; };
    name.onkeydown = (e) => { if (e.key === "Enter") create(); };
    const tip = el("button", "help", "?");
    tip.dataset.tip = "論文存在 papers/ 底下的資料夾名稱，依標題或 Key value 自動產生，可以改。";
    const b = el("button", "primary small", st.busy ? "建立中…" : "建立論文");
    b.disabled = st.busy;
    b.onclick = create;
    foot.append(name, tip, b);
  }

  function go(step) {
    if (step > 2 && st.step <= 2 && !st.key.trim()) { st.step = 2; draw(); flash("Key value 必填：這篇論文通篇要強調的一件事"); return; }
    st.step = step;
    draw();
  }

  // Folder name from the title (else the key value): its main English words,
  // or the first characters of a Chinese key value; never one that exists.
  const STOP = new Set("a an the of for and or in on with to via by from using based towards toward into at its is are as".split(" "));
  function autoName() {
    const src = st.title.trim() || st.key.trim();
    let base = (src.toLowerCase().match(/[a-z0-9]+/g) ?? []).filter((w) => !STOP.has(w)).slice(0, 5).join("-");
    if (!base) base = (src.match(/[\p{L}\p{N}]+/gu) ?? []).join("").slice(0, 16);
    if (!base) base = "paper";
    const taken = new Set(state.workspace?.papers ?? []);
    let name = base, n = 2;
    while (taken.has(name)) name = `${base}-${n++}`;
    return name;
  }

  // ── 01 template ──
  function stepTemplate(body) {
    const q = el("input", "np-input");
    q.type = "search";
    q.placeholder = "搜尋期刊或研討會";
    const list = el("div", "tpl-list");
    const fill = () => {
      list.innerHTML = "";
      const query = q.value.trim().toLowerCase();
      const option = (value, label, sub) => {
        const b = el("button", "tpl");
        b.append(el("span", "name", label));
        if (sub) b.append(el("span", "sub", sub));
        b.setAttribute("aria-pressed", String(value === st.template));
        b.onclick = () => { st.template = value; store.set("pc.newTemplate", value); fill(); };
        list.append(b);
      };
      if (!query) option("", "空白", "一般 article");
      const hit = (t) => !query || `${t.name} ${t.short ?? ""} ${t.publisher ?? ""} ${t.id}`.toLowerCase().includes(query);
      [["journal", "期刊"], ["conference", "研討會"]].forEach(([k, title]) => {
        const group = st.tpls.filter((t) => (t.kind === "conference" ? "conference" : "journal") === k && hit(t)).sort((a, b) => a.name.localeCompare(b.name));
        if (!group.length) return;
        list.append(el("div", "tpl-group", title));
        group.forEach((t) => option(t.id, t.name, [t.short, t.publisher].filter(Boolean).join(" · ")));
      });
    };
    q.oninput = fill;
    fill();
    body.append(label("版型"), q, list);
    setTimeout(() => q.focus());
  }

  // ── 02 authors ──
  function stepAuthors(body) {
    body.append(el("p", "np-note", "從作者資料庫勾選，依序排列；可以略過，之後再到論文中修改。新增的人會存進資料庫，下次直接選。"));
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
      } catch (e) { flash(e.message, 6000); save.disabled = false; }
    };
    const grid = el("div", "np-grid");
    grid.append(given, family, email, orcid);
    const row = el("div", "np-row");
    row.append(cancel, save);
    f.append(grid, role, sel, newBox, row);
    setTimeout(() => given.focus());
    return f;
  }

  // ── 03 key value and outline ──
  function stepKey(body) {
    const kv = el("input", "np-input kv");
    kv.placeholder = "這篇論文通篇要強調的一件事，例如：提升髖部骨折判讀的可解釋性";
    kv.value = st.key;
    const count = el("span", "np-count");
    const recount = () => { const [n, max, unit] = kvLength(kv.value); count.textContent = `${n} / ${max} ${unit}`; count.classList.toggle("bad", n > max); };
    kv.oninput = () => { st.key = kv.value; recount(); };
    recount();
    const kvHead = label("Key value（必填）", "論文一定要有一個核心主張，可以是願景，例如「提升 XX 效能」。生成段落與檢查（R106 離題檢查）都會以它為準。中文最多 50 字，英文最多 30 個單字。");
    kvHead.append(count);

    const outline = el("textarea", "np-input outline");
    outline.rows = 7;
    outline.placeholder = "選填。條列這篇大概有哪些章節、各要說什麼，例如：\n- 背景：髖部骨折常見、X 光判讀易漏看\n- 方法：弱監督 + 注意力引導\n- 結果：準確率與 IoU 提升\n- 討論：臨床可用性與限制";
    outline.value = st.outline;
    outline.oninput = () => (st.outline = outline.value);
    const on = claudeReady();
    const gen = el("button", "ghost", st.busy ? "Claude 產生中…" : st.plan ? "重新產生" : "用 Claude 產生標題與架構");
    gen.prepend(icon("ai"));
    gen.disabled = !on || st.busy;
    if (!on) gen.title = "連接 Claude 後可用";
    gen.onclick = generate;
    const genRow = el("div", "np-row start");
    genRow.append(gen);
    if (!on) genRow.append(el("span", "np-note", "連接 Claude 後可用；不填大綱也能建立。"));

    const title = el("input", "np-input");
    title.placeholder = "標題（選填，之後可改）";
    title.value = st.title;
    title.oninput = () => (st.title = title.value);

    body.append(kvHead, kv, label("大綱"), outline, genRow);
    if (st.plan) {
      if (st.plan.titles.length) {
        const ts = el("div", "np-titles");
        st.plan.titles.forEach((t) => {
          const b = el("button", "np-title", t);
          b.setAttribute("aria-pressed", String(t === st.title));
          b.onclick = () => { st.title = t; draw(); };
          ts.append(b);
        });
        body.append(label("標題建議（點選套用）"), ts);
      }
      const use = el("label", "np-check");
      const cb = el("input");
      cb.type = "checkbox";
      cb.checked = st.usePlan;
      cb.onchange = () => { st.usePlan = cb.checked; };
      use.append(cb, el("span", "", "用這個架構取代版型的範例章節"));
      const sk = el("ol", "np-skeleton");
      st.plan.sections.forEach((s) => {
        const li = el("li");
        li.append(el("b", "", s.title), el("p", "", s.intent));
        if (s.subsections.length) {
          const ul = el("ul");
          s.subsections.forEach((x) => { const i = el("li"); i.append(el("b", "", x.title), el("p", "", x.intent)); ul.append(i); });
          li.append(ul);
        }
        sk.append(li);
      });
      body.append(label("章節架構"), use, sk);
    }
    body.append(label("標題"), title);
    setTimeout(() => (st.key ? outline : kv).focus());
  }

  async function generate() {
    const [n, max] = kvLength(st.key);
    if (!st.key.trim() || n > max) { flash(!st.key.trim() ? "先填 Key value" : "Key value 太長"); return; }
    if (!st.outline.trim()) { flash("先寫大綱"); return; }
    st.busy = true; draw();
    try {
      st.plan = await api("/api/wizard/outline", { template: st.template || null, outline: st.outline, key_value: st.key, engine: claudeVia() });
      st.usePlan = true;
      if (!st.title && st.plan.titles[0]) st.title = st.plan.titles[0];
    } catch (e) { flash(e.message, 6000); }
    st.busy = false; draw();
  }

  // ── 04 tables ──
  const BLANK_SECTIONS = ["Introduction", "Related work", "Method", "Result", "Discussion", "Conclusion"];

  /** Sections the paper will have: the skeleton, else the template's sample, else the blank ones. */
  async function paperSections() {
    if (st.plan && st.usePlan) return st.plan.sections.map((s) => s.title);
    if (!st.template) return BLANK_SECTIONS;
    if (st.sampleFor !== st.template) {
      try {
        const r = await fetch(`/api/templates/${encodeURIComponent(st.template)}/sample`);
        const t = r.ok ? await r.text() : "";
        st.sampleSections = [...t.matchAll(/^[ \t]*\\section\{([^}]*)\}/gm)].map((m) => m[1].trim());
      } catch { st.sampleSections = []; }
      st.sampleFor = st.template;
    }
    return st.sampleSections.length ? st.sampleSections : BLANK_SECTIONS;
  }

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

  async function stepTables(body) {
    const id = st.drawId;
    const sections = await paperSections();
    if (id !== st.drawId) return;
    const note = label("已有的結果", "表格：從 Excel 或 Google 試算表選取儲存格直接貼上（也可以貼 CSV 或 Markdown 表格），第一列是表頭，樣式預設用這個期刊的。圖片：PNG、JPG、PDF 或 EPS。依序放在指定章節的最後；標題留白會自動產生。");
    body.append(note);
    const def = sections.find((s) => /result/i.test(s)) ?? sections[sections.length - 1];
    st.tables.forEach((t, i) => body.append(t.kind === "figure" ? figureCard(t, i, sections) : tableCard(t, i, sections)));
    const addT = el("button", "ghost np-add", "新增表格");
    addT.prepend(icon("plus"));
    addT.onclick = () => { st.tables.push({ kind: "table", caption: "", section: def, data: "", style: "" }); draw(); setTimeout(() => [...body.querySelectorAll(".np-table textarea")].pop()?.focus()); };
    const addF = el("button", "ghost np-add", "新增圖片");
    addF.prepend(icon("image"));
    addF.onclick = () => pickImage((f) => { st.tables.push({ kind: "figure", caption: "", section: def, file: f }); draw(); autoCaption(st.tables[st.tables.length - 1]); });
    const row = el("div", "np-row start");
    row.append(addT, addF);
    body.append(row);
  }

  function pickImage(done) {
    const i = el("input");
    i.type = "file";
    i.accept = ".png,.jpg,.jpeg,.pdf,.eps";
    i.onchange = () => i.files[0] && done(i.files[0]);
    i.click();
  }

  /** figures/ name: fig2-roc-curves.png, plain characters only. */
  function figureFile(t) {
    const n = st.tables.filter((x) => x.kind === "figure").indexOf(t) + 1;
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
    t.capJob = api("/api/wizard/caption", { kind: t.kind, data: t.data ?? "", file: t.kind === "figure" ? t.file.name : "", section: t.section, key_value: st.key, title: st.title, engine: chosenEngine() })
      .then((r) => { if (r.caption && (!t.caption.trim() || force)) t.caption = r.caption; else if (force && !r.caption) flash("從檔名看不出這張圖的內容，請自己填標題"); })
      .catch((e) => flash(e.message, 5000))
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

  /** Caption input with a button that writes one. */
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

  function sectionSelect(t, sections) {
    const sec = el("select", "np-input");
    if (!sections.includes(t.section)) t.section = sections.find((s) => /result/i.test(s)) ?? sections[sections.length - 1];
    sections.forEach((s) => sec.append(new Option(s, s)));
    sec.value = t.section;
    sec.onchange = () => (t.section = sec.value);
    return sec;
  }

  const moveButtons = (i) => {
    const move = (d, ic, tip) => iconButton(ic, tip, () => { const j = i + d; if (j < 0 || j >= st.tables.length) return; [st.tables[i], st.tables[j]] = [st.tables[j], st.tables[i]]; draw(); });
    return [move(-1, "up", "上移"), move(1, "chevron", "下移"), iconButton("x", "移除", () => { st.tables.splice(i, 1); draw(); })];
  };

  function figureCard(t, i, sections) {
    const c = el("div", "np-table");
    const head = el("div", "np-row start");
    head.append(el("b", "np-tn", `圖 ${st.tables.slice(0, i + 1).filter((x) => x.kind === "figure").length}`));
    head.append(...captionInput(t, "標題（caption），留白會自動產生"), ...moveButtons(i));
    const where = el("div", "np-grid");
    const change = el("button", "ghost", t.file.name);
    change.title = "換一張";
    change.onclick = () => pickImage((f) => { t.file = f; draw(); });
    where.append(sectionSelect(t, sections), change);
    const prev = el("div", "np-preview");
    if (/\.(png|jpe?g)$/i.test(t.file.name)) {
      const img = el("img", "np-fig");
      img.src = URL.createObjectURL(t.file);
      prev.append(img);
    }
    c.append(head, where, prev);
    return c;
  }

  function tableCard(t, i, sections) {
    const c = el("div", "np-table");
    const head = el("div", "np-row start");
    head.append(el("b", "np-tn", `表 ${st.tables.slice(0, i + 1).filter((x) => x.kind !== "figure").length}`));
    const [cap, gen] = captionInput(t, "標題（caption），留白會自動產生");
    const sec = sectionSelect(t, sections);
    const sty = el("select", "np-input");
    sty.append(new Option("期刊預設樣式", ""));
    st.styles.forEach((s) => sty.append(new Option(s.name, s.id)));
    sty.value = t.style;
    sty.onchange = () => (t.style = sty.value);
    head.append(cap, gen, ...moveButtons(i));
    const where = el("div", "np-grid");
    where.append(sec, sty);
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

  // ── 05 references ──
  function stepRefs(body) {
    body.append(el("p", "np-note", "把想引用的文章一次貼上（可以夾雜備註）。Claude 先算出有幾篇，再一篇一篇查證、建檔、加標籤，存進文獻庫；你對每篇的備註會記成這篇論文的「引用用途」。可以邊匯入邊繼續其他步驟。"));
    if (!claudeReady()) body.append(el("p", "np-note bad", "連接 Claude 後可用。"));
    st.refs ??= PCRefs.importer();
    body.append(st.refs.el);
  }

  async function create() {
    if (!st.name.trim()) { $("#npName")?.focus(); flash("請填資料夾名稱"); return; }
    if (st.refs?.running()) { flash("文獻還在匯入中，請稍候"); return; }
    const tables = st.tables.filter((t) => t.kind === "figure" || t.data.trim());
    if (tables.some((t) => t.kind !== "figure" && grid(t.data).length < 2)) { flash("表格至少要有表頭和一列資料"); return; }
    const [n, max, unit] = kvLength(st.key);
    if (!st.key.trim()) { go(2); flash("Key value 必填：這篇論文通篇要強調的一件事"); return; }
    if (n > max) { go(2); flash(`Key value 最多 ${max} ${unit}`); return; }
    st.busy = true; draw();
    try {
      await Promise.all(tables.map((t) => autoCaption(t)));
      const r = await api("/api/papers", {
        name: st.name.trim(), template: st.template || null, key_value: st.key, title: st.title.trim() || null,
        authors: st.chosen, outline: st.plan && st.usePlan ? st.plan.sections : [],
        tables: tables.map((t) => (t.kind === "figure"
          ? { kind: "figure", caption: t.caption, section: t.section, file: figureFile(t) }
          : { caption: t.caption, section: t.section, data: t.data, style: t.style || null })),
      });
      for (const t of tables.filter((x) => x.kind === "figure")) {
        const up = await fetch(`/api/papers/${encodeURIComponent(r.name)}/figures?file=${encodeURIComponent(figureFile(t))}`, { method: "POST", body: t.file });
        if (!up.ok) flash(`${t.file.name}：${await up.text()}`, 6000);
      }
      // the user's note on each filed paper becomes its purpose in this paper
      for (const it of st.refs?.items() ?? []) {
        if (it.key && it.guess.note) await api(`/api/papers/${encodeURIComponent(r.name)}/citations`, { key: it.key, purpose: it.guess.note }).catch(() => {});
      }
      $("#dlgNew").close();
      await opened(r.name);
    } catch (e) { flash(e.message, 6000); st.busy = false; draw(); }
  }

  function label(text, tip) {
    const l = el("div", "np-label");
    l.append(el("span", "", text));
    if (tip) { const h = el("button", "help", "?"); h.dataset.tip = tip; l.append(h); }
    return l;
  }

  function wire() {
    document.querySelectorAll("#npTabs button").forEach((b) => (b.onclick = () => go(+b.dataset.step)));
  }

  /** Has the user filled anything in that closing would lose? */
  const dirty = () => !!st && !st.busy && !!(st.name.trim() || st.key.trim() || st.outline.trim() || st.title.trim() || st.chosen.length || st.plan || st.tables.length);

  return { open, wire, kvLength, dirty };
})();
