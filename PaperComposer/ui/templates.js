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

  // ── House rules: [ref_words], [captions], [limits], [word_limits] ──
  const REF_KINDS = [["fig", "圖"], ["tab", "表"], ["eq", "式"], ["sec", "節"]];
  const REF_DEFAULT = { fig: "Fig.", tab: "Table", eq: "", sec: "Section" };
  const RULE_NAMES = {
    "captions.table": "表", "captions.figure": "圖", "word_limits.abstract": "摘要", "word_limits.total": "全文",
    "limits.pages": "頁數", "limits.figures": "圖", "limits.tables": "表", "limits.references": "參考文獻",
    "limits.keywords": "關鍵字上限", "limits.keywords_min": "關鍵字下限", "limits.anonymous": "雙盲",
    "requires.highlights": "Highlights", "requires.graphical_abstract": "Graphical abstract", "reference_style": "文獻格式",
  };
  const num = (v, ph = "") => { const i = el("input"); i.type = "number"; i.min = "0"; i.inputMode = "numeric"; i.value = v ?? ""; i.placeholder = ph; return i; };
  const intOf = (i) => { const n = parseInt(i.value, 10); return Number.isFinite(n) && n > 0 ? n : undefined; };
  const block = (title, ...kids) => { const b = el("div", "tr-block"); b.append(el("div", "tr-h", title), ...kids); return b; };
  const tick = (label, on) => { const l = el("label", "tr-tick"); const c = el("input"); c.type = "checkbox"; c.checked = !!on; l.append(c, el("span", "", label)); return [l, c]; };

  /** The rules section of a journal: { node, value() } — value() is what PUT sends.
   *  `src` (key → { quote, from }) shows where each value came from: a line of
   *  the author guidelines, the template's own text, or the class's house style.
   *  A value changed here loses its source. */
  function rulesSection(j, src = null) {
    const rw = j.ref_words ?? {};
    const cleverDetected = !!j.cleveref && !Object.keys(rw).length;
    let mode = /^\s*cleveref\s*$/i.test(rw.mode ?? "") || cleverDetected ? "cleveref" : "words";
    // saved sources are text: a class's house style reads "<publisher> 慣例（<class>）"
    const sources = src ?? Object.fromEntries(Object.entries(j.sources ?? {}).map(([k, q]) => [k, { quote: q, from: /慣例（[^）]+）$/.test(q) ? "class" : undefined }]));
    const FROM = { page: "作者須知", template: "範本", class: "class" };
    const watched = []; // [key, () => current value as text, value at start]
    const watch = (key, read) => watched.push([key, read, read()]);
    const KIND_NAME = Object.fromEntries(REF_KINDS);
    const what = (k) => {
      const m = k.match(/^ref_words\.(?:(sentence_start)\.)?(\w+)$/);
      if (m) return `${KIND_NAME[m[2]] ?? m[2]}·${m[1] ? "句首" : "句中"}`;
      if (k.startsWith("word_limits.sections.")) return k.slice("word_limits.sections.".length);
      return RULE_NAMES[k] ?? k;
    };
    // one line per sentence, naming the values it gave
    const quote = (...keys) => {
      const box = el("div", "tr-quotes");
      const lines = new Map();
      for (const k of keys) {
        const s = sources[k];
        if (!s?.quote) continue;
        const id = `${s.from ?? ""}\u0000${s.quote}`;
        if (!lines.has(id)) lines.set(id, { s, keys: [] });
        lines.get(id).keys.push(k);
      }
      for (const { s, keys: ks } of lines.values()) {
        const q = el("div", `tr-quote${s.from === "class" ? " cls" : ""}`);
        q.dataset.keys = ks.join(" ");
        const head = el("span", "tr-from", [s.from ? FROM[s.from] ?? s.from : "", ks.map(what).join("、")].filter(Boolean).join(" · "));
        q.append(head, el("span", "", s.from === "class" ? s.quote : `「${s.quote}」`));
        box.append(q);
      }
      return box;
    };

    // references: mode, then a word per kind (mid-sentence / sentence start)
    const modeSeg = el("div", "seg tr-mode");
    const modes = [["words", "文字＋\\ref"], ["cleveref", "\\cref"]];
    const grid = el("div", "tr-words");
    grid.append(el("span"), el("span", "tr-col", "句中"), el("span", "tr-col", "句首"));
    const inputs = {};
    for (const [k, label] of REF_KINDS) {
      const mid = input(rw[k] ?? "", REF_DEFAULT[k] || (k === "eq" ? "\\eqref" : ""));
      const start = input(rw.sentence_start?.[k] ?? "", "同句中");
      mid.setAttribute("aria-label", `${label} 句中`);
      start.setAttribute("aria-label", `${label} 句首`);
      inputs[k] = [mid, start];
      watch(`ref_words.${k}`, () => mid.value.trim());
      watch(`ref_words.sentence_start.${k}`, () => start.value.trim());
      grid.append(el("span", "tr-kind", label), mid, start);
    }
    const sample = el("code", "tr-sample");
    const snippet = (k, atStart) => {
      if (mode === "cleveref") return `\\${atStart ? "Cref" : "cref"}{${k}:1}`;
      const [mid, start] = inputs[k];
      const w = (atStart && start.value.trim()) || mid.value.trim() || REF_DEFAULT[k];
      const cmd = k === "eq" ? "eqref" : "ref";
      return w ? `${w}~\\${cmd}{${k}:1}` : `\\${cmd}{${k}:1}`;
    };
    const paint = () => {
      modeSeg.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.v === mode)));
      grid.hidden = mode === "cleveref";
      sample.textContent = mode === "cleveref" ? `${snippet("fig", true)} · ${snippet("fig", false)}`
        : ["fig", "tab", "eq", "sec"].map((k) => snippet(k, false)).join(" · ");
    };
    for (const [v, label] of modes) {
      const b = el("button", "", label);
      b.type = "button";
      b.dataset.v = v;
      b.onclick = () => { mode = v; paint(); };
      modeSeg.append(b);
    }
    grid.addEventListener("input", paint);
    const refHead = el("div", "tr-row");
    refHead.append(modeSeg);
    if (cleverDetected) refHead.append(el("span", "tr-note", "範本載入 cleveref"));
    const refKeys = REF_KINDS.flatMap(([k]) => [`ref_words.${k}`, `ref_words.sentence_start.${k}`]);

    // captions
    const capSel = (v) => {
      const s = el("select");
      [["", "—"], ["above", "上方"], ["below", "下方"]].forEach(([k, l]) => s.append(new Option(l, k)));
      s.value = v === "above" || v === "below" ? v : "";
      return s;
    };
    const capTab = capSel(j.captions?.table);
    const capFig = capSel(j.captions?.figure);
    watch("captions.table", () => capTab.value);
    watch("captions.figure", () => capFig.value);
    const caps = el("div", "tr-pairs");
    caps.append(field("表說明", capTab), field("圖說明", capFig));

    // counts: pages, figures, tables, references, keywords
    const lim = j.limits ?? {};
    const counted = (key, label, v) => { const i = num(v, "—"); watch(key, () => i.value.trim()); return [field(label, i), i]; };
    const [pagesF, pages] = counted("limits.pages", "頁數", lim.pages);
    const [figsF, figs] = counted("limits.figures", "圖", lim.figures);
    const [tabsF, tabs] = counted("limits.tables", "表", lim.tables);
    const [refsF, refs] = counted("limits.references", "參考文獻", lim.references);
    const kwMin = num(lim.keywords_min, "最少");
    const kwMax = num(lim.keywords, "最多");
    kwMin.setAttribute("aria-label", "關鍵字 最少");
    kwMax.setAttribute("aria-label", "關鍵字 最多");
    watch("limits.keywords_min", () => kwMin.value.trim());
    watch("limits.keywords", () => kwMax.value.trim());
    const kwBox = el("div", "tr-range");
    kwBox.append(kwMin, el("span", "", "–"), kwMax);
    const [exLabel, exRefs] = tick("頁數不含參考文獻", lim.pages_exclude_refs);
    const [anLabel, anon] = tick("雙盲匿名", lim.anonymous);
    watch("limits.anonymous", () => String(anon.checked));
    const countGrid = el("div", "tr-counts");
    countGrid.append(pagesF, figsF, tabsF, refsF, field("關鍵字", kwBox));
    const countTicks = el("div", "tr-ticks");
    countTicks.append(exLabel, anLabel);

    // what the submission must include besides the paper
    const req = j.requires ?? {};
    const [hlLabel, hl] = tick("Highlights", req.highlights);
    const [gaLabel, ga] = tick("Graphical abstract", req.graphical_abstract);
    watch("requires.highlights", () => String(hl.checked));
    watch("requires.graphical_abstract", () => String(ga.checked));
    const refStyle = input(j.reference_style ?? "", "Vancouver、APA…");
    watch("reference_style", () => refStyle.value.trim());
    const reqTicks = el("div", "tr-ticks");
    reqTicks.append(hlLabel, gaLabel);

    // word limits
    const wl = j.word_limits ?? {};
    const abs = num(wl.abstract, "—");
    const total = num(wl.total, "—");
    watch("word_limits.abstract", () => abs.value.trim());
    watch("word_limits.total", () => total.value.trim());
    const wordRow = el("div", "tr-pairs");
    wordRow.append(field("摘要", abs), field("全文", total));
    const secs = el("div", "tr-secs");
    const addSec = (title = "", n = "") => {
      const row = el("div", "tr-sec");
      const t = input(title, "章節標題");
      const c = num(n, "字數");
      row.dataset.start = `${title}=${n}`;
      row.append(t, c, iconButton("x", "移除", () => row.remove()));
      secs.append(row);
      return t;
    };
    Object.entries(wl.sections ?? {}).forEach(([t, n]) => addSec(t, n));
    const more = el("button", "ghost small tr-add", "章節");
    more.type = "button";
    more.prepend(icon("plus"));
    more.onclick = () => addSec().focus();
    const secKeys = Object.keys(wl.sections ?? {}).map((t) => `word_limits.sections.${t}`);

    const node = el("section", "tpl-rules");
    node.append(
      el("h4", "", "投稿規則"),
      el("div", "tr-grid"),
    );
    node.lastChild.append(
      block("引用", refHead, grid, sample, quote(...refKeys)),
      block("圖表說明", caps, quote("captions.table", "captions.figure")),
      block("字數上限", wordRow, secs, more, quote("word_limits.abstract", "word_limits.total", ...secKeys)),
      block("篇幅與數量", countGrid, countTicks, quote("limits.pages", "limits.figures", "limits.tables", "limits.references", "limits.keywords_min", "limits.keywords", "limits.anonymous")),
      block("投稿附件", reqTicks, field("文獻格式要求", refStyle), quote("requires.highlights", "requires.graphical_abstract", "reference_style")),
    );
    paint();

    function value() {
      const words = {};
      const start = {};
      for (const [k] of REF_KINDS) {
        const [mid, st] = inputs[k];
        if (mid.value.trim()) words[k] = mid.value.trim();
        if (st.value.trim()) start[k] = st.value.trim();
      }
      if (Object.keys(start).length) words.sentence_start = start;
      // cleveref in sample.tex counts while nothing is set: write the mode only when it differs
      if (mode === "cleveref") { if (Object.keys(words).length || !j.cleveref) words.mode = "cleveref"; }
      else if (!Object.keys(words).length && j.cleveref) words.mode = "words";
      const captions = {};
      if (capTab.value) captions.table = capTab.value;
      if (capFig.value) captions.figure = capFig.value;
      const limits = {};
      for (const [k, i] of [["pages", pages], ["figures", figs], ["tables", tabs], ["references", refs], ["keywords", kwMax], ["keywords_min", kwMin]]) if (intOf(i)) limits[k] = intOf(i);
      if (exRefs.checked) limits.pages_exclude_refs = true;
      if (anon.checked) limits.anonymous = true;
      const requires = {};
      if (hl.checked) requires.highlights = true;
      if (ga.checked) requires.graphical_abstract = true;
      const word_limits = {};
      if (intOf(abs)) word_limits.abstract = intOf(abs);
      if (intOf(total)) word_limits.total = intOf(total);
      const sections = {};
      const keepSec = new Set();
      secs.querySelectorAll(".tr-sec").forEach((r) => {
        const [t, c] = r.querySelectorAll("input");
        if (t.value.trim() && intOf(c)) {
          sections[t.value.trim()] = intOf(c);
          if (r.dataset.start === `${t.value.trim()}=${intOf(c)}`) keepSec.add(`word_limits.sections.${t.value.trim()}`);
        }
      });
      if (Object.keys(sections).length) word_limits.sections = sections;
      // a source stays only while its value is as it was read
      const changed = new Set(watched.filter(([, read, was]) => read() !== was).map(([k]) => k));
      const kept = {};
      for (const [k, s] of Object.entries(sources)) {
        if (!s?.quote || changed.has(k)) continue;
        if (k.startsWith("word_limits.sections.") && !keepSec.has(k)) continue;
        kept[k] = s.quote;
      }
      return { ref_words: words, captions, limits, requires, word_limits, reference_style: refStyle.value.trim(), sources: kept };
    }
    return { node, value };
  }

  async function open(select) {
    kind ??= store.get("pc.tplKind", "journal");
    try { list = await api("/api/templates"); } catch (e) { list = []; LoreShell.toast(e.message, { err: true, ms: 5000 }); }
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
    add.onclick = () => { current = null; newFlow(); paintList(); };
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
    if (current) detail();
    else newFlow();
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
      const go = iconButton("external", "作者須知", () => api("/api/open-link", { url: j.url }).catch((e) => LoreShell.toast(e.message, { err: true })));
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
    const rules = rulesSection(j);

    const out = el("div", "tpl-out");
    const save = el("button", "primary small", "儲存");
    save.onclick = async () => {
      save.disabled = true;
      try {
        const r = await api(`/api/templates/${encodeURIComponent(j.id)}`, {
          name: name.value, short: short.value, publisher: publisher.value, kind: kindSel.value, url: url.value, notes: notes.value,
          class_options: opts.value, bib_style: style.value, ...rules.value(),
        }, "PUT");
        Object.assign(j, r);
        LoreShell.toast("已儲存模板");
        await refreshWorkspace();
        draw();
        refreshGit();
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
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
    main.append(head, sub, grid, rules.node, acts);
  }

  /** 新增期刊 from a template file and the author guidelines (newtemplate.js). */
  function newFlow() {
    PCNewTemplate.open($("#tplMain"), {
      kind,
      rulesSection,
      fromExisting: list.length ? newForm : null,
      done: async (id) => {
        current = id;
        store.set("pc.template", id);
        await refreshWorkspace();
        await open(id);
        refreshGit();
      },
    });
  }

  function newForm() {
    const main = $("#tplMain");
    main.innerHTML = "";
    const head = el("div", "tpl-head");
    head.append(el("h3", "", kind === "conference" ? "新增研討會" : "新增期刊"));
    const back = el("button", "ghost small nt-switch", "從模板檔");
    back.prepend(icon("upload"));
    back.onclick = newFlow;
    head.append(back);
    main.append(head);
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
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); go.disabled = false; }
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
    catch (e) { LoreShell.toast(e.message, { err: true }); return; }
    main.innerHTML = "";
    const head = el("div", "tpl-head");
    head.append(el("h3", "", `${j.name} · sample.tex`));
    const ta = el("textarea", "tpl-sample");
    ta.value = text;
    ta.spellcheck = false;
    const save = el("button", "primary small", "儲存");
    save.onclick = async () => {
      try { await api(`/api/templates/${encodeURIComponent(j.id)}/sample`, { text: ta.value }, "PUT"); LoreShell.toast("已儲存範本"); await open(); refreshGit(); }
      catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
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
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
    };
    go.onclick = make;
    n.onkeydown = (e) => { if (e.key === "Enter") make(); };
    row.append(n, go);
    main.append(row);
    n.focus();
  }

  return { open };
})();
