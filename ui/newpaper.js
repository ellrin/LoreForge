// New paper: a three-step window.
//   01 name and template
//   02 authors, picked from the profile's authors.csv (new people are saved there)
//   03 key value (required) and, optionally, a bullet outline that Claude
//      turns into title suggestions and a section skeleton.
// Only step 03's outline needs Claude; everything else works without it.
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
    st = { step: 0, name: "", template: store.get("pc.newTemplate", ""), tpls: [], people: { authors: [], affiliations: [] }, chosen: [], key: "", outline: "", plan: null, usePlan: true, title: "", busy: false };
    try { st.tpls = await api("/api/templates"); } catch {}
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
    [stepTemplate, stepAuthors, stepKey][st.step](body);
    const foot = $("#npFoot");
    foot.innerHTML = "";
    if (st.step > 0) { const b = el("button", "ghost", "上一步"); b.onclick = () => go(st.step - 1); foot.append(b); }
    foot.append(el("span", "grow"));
    if (st.step < 2) { const b = el("button", "primary small", "下一步"); b.onclick = () => go(st.step + 1); foot.append(b); }
    else { const b = el("button", "primary small", st.busy ? "建立中…" : "建立論文"); b.disabled = st.busy; b.onclick = create; foot.append(b); }
  }

  function go(step) {
    if (step > 0 && !st.name.trim()) { st.step = 0; draw(); flash("請先填論文名稱"); $("#npName")?.focus(); return; }
    st.step = step;
    draw();
  }

  // ── 01 name and template ──
  function stepTemplate(body) {
    const name = el("input", "np-input");
    name.id = "npName";
    name.placeholder = "論文名稱（資料夾名稱，例如 hip-fracture-2027）";
    name.value = st.name;
    name.oninput = () => (st.name = name.value);
    name.onkeydown = (e) => { if (e.key === "Enter") go(1); };
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
    body.append(label("名稱"), name, label("版型"), q, list);
    setTimeout(() => name.focus());
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

  async function create() {
    if (!st.name.trim()) { go(0); return; }
    const [n, max, unit] = kvLength(st.key);
    if (!st.key.trim()) { flash("Key value 必填：這篇論文通篇要強調的一件事"); return; }
    if (n > max) { flash(`Key value 最多 ${max} ${unit}`); return; }
    st.busy = true; draw();
    try {
      const r = await api("/api/papers", {
        name: st.name.trim(), template: st.template || null, key_value: st.key, title: st.title.trim() || null,
        authors: st.chosen, outline: st.plan && st.usePlan ? st.plan.sections : [],
      });
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

  return { open, wire, kvLength };
})();
