// References: the library page and the insert-citation picker.
// Bibliographic data comes from refs/library.bib; one-liners, summaries,
// tags and PDF file names from refs/notes.csv. PDFs stay on this machine.

const PCRefs = (() => {
  let refs = [];
  let tags = [];
  let pdfDir = null;
  let filter = { q: "", tags: new Set() };

  async function load() {
    try {
      const r = await api("/api/refs");
      refs = r.refs; pdfDir = r.pdf_dir;
      tags = await api("/api/refs/tags");
    } catch { refs = []; }
  }
  const tagLabel = (id) => tags.find((t) => t.id === id)?.label ?? id;

  function matches(r, f) {
    const q = f.q.trim().toLowerCase();
    const hay = `${r.key} ${r.title} ${r.authors} ${r.year} ${r.venue} ${r.one_liner}`.toLowerCase();
    return (!q || q.split(/\s+/).every((w) => hay.includes(w))) && [...f.tags].every((t) => r.tags.includes(t));
  }

  function tagChips(f, onChange) {
    const row = el("div", "chips-row");
    tags.forEach((t) => {
      const b = el("button", "tchip", t.label);
      b.setAttribute("aria-pressed", String(f.tags.has(t.id)));
      b.onclick = () => { f.tags.has(t.id) ? f.tags.delete(t.id) : f.tags.add(t.id); b.setAttribute("aria-pressed", String(f.tags.has(t.id))); onChange(); };
      row.append(b);
    });
    return row;
  }

  async function openPdf(r) {
    try { await api(`/api/refs/${encodeURIComponent(r.key)}/open`, {}); }
    catch (e) { flash(e.message, 4000); }
  }

  async function saveNote(r, body) {
    try { await api(`/api/refs/${encodeURIComponent(r.key)}`, body, "PUT"); Object.assign(r, body); refreshGit(); }
    catch (e) { flash(e.message, 6000); }
  }

  // ── library page ──
  async function renderPage() {
    await load();
    const box = $("#refsBody");
    box.innerHTML = "";
    const search = el("input", "search");
    search.type = "search";
    search.placeholder = "搜尋";
    search.value = filter.q;
    const list = el("div", "ref-list");
    const draw = () => {
      list.innerHTML = "";
      const shown = refs.filter((r) => matches(r, filter));
      if (!refs.length) list.append(el("p", "empty", "匯入 .bib 開始"));
      shown.forEach((r) => list.append(row(r)));
      $("#refsCount").textContent = `${shown.length} / ${refs.length}`;
    };
    search.oninput = () => { filter.q = search.value; draw(); };
    const bar = el("div", "refs-bar");
    libImporter ??= { open: false, ui: importer({ paper: () => (libImporter.attach && state.paper ? state.paper.name : null), onDone: () => { refreshGit(); renderPage(); } }), attach: false };
    const batchBox = el("div", "batch");
    const drawBatch = () => {
      batchBox.hidden = !libImporter.open;
      batchBox.innerHTML = "";
      if (!libImporter.open) return;
      if (state.paper) {
        const l = el("label", "np-check");
        const cb = el("input"); cb.type = "checkbox"; cb.checked = libImporter.attach;
        cb.onchange = () => (libImporter.attach = cb.checked);
        l.append(cb, el("span", "", `把每篇的備註記到「${state.paper.name}」的引用用途`));
        batchBox.append(l);
      }
      batchBox.append(libImporter.ui.el);
    };
    const pasteBtn = iconButton("list", claudeReady() ? "貼上想引用的文章，逐篇查證後建檔" : "連接 Claude 後可貼上文獻清單逐篇查證", () => { libImporter.open = !libImporter.open; drawBatch(); });
    pasteBtn.disabled = !claudeReady();
    bar.append(search, pasteBtn, iconButton("upload", "匯入 BibTeX", (e) => importPop(e.currentTarget)), iconButton("folder", "這台電腦的 PDF 資料夾", (e) => pdfPop(e.currentTarget)));
    queueBox ??= el("div", "queue");
    box.append(bar, batchBox, await intake(), queueBox, tagChips(filter, draw), list);
    drawBatch();
    drawQueue();
    draw();
  }

  // ── a pasted list of papers to cite ──
  // Claude splits the text into papers (and copies the user's remarks on
  // each); Crossref, arXiv or PubMed say whether each exists; the ones that
  // do are filed (BibTeX from the database, notes and tags from the model)
  // and saved, a few at a time, with a progress bar. Papers that need a
  // decision wait for the user: pick a candidate, or file one no database has
  // from the user's own entry.
  const MARK = { queued: "·", checking: "…", filing: "…", saved: "✓", exists: "＝", ambiguous: "?", missing: "✕", error: "!", skipped: "–" };
  const LABEL = { saved: "已存入", exists: "已在文獻庫", ambiguous: "需要確認", missing: "查不到", error: "失敗", skipped: "略過" };

  /** opts.paper(): the paper whose citation purposes get the user's notes (or null). */
  function importer(opts = {}) {
    const st = { text: "", items: [], phase: "idle", error: "" };
    const box = el("div", "importer");

    async function file(it, work) {
      it.state = "filing"; it.msg = "建檔、產生標籤中…"; draw();
      try {
        let q = work.doi && work.source !== "pubmed" ? await api("/api/refs/propose", { doi: work.doi, engine: chosenEngine() }) : null;
        // no DOI record to fetch (or doi.org failed): file the database's own record
        if (!q || (q.error && work.names?.length)) q = await api("/api/refs/propose-record", { work, engine: chosenEngine() });
        if (q.exists) { it.state = "exists"; it.key = q.exists; it.msg = `已在文獻庫：${q.exists}`; }
        else if (q.error) { it.state = "error"; it.msg = q.error; }
        else {
          const r = await api("/api/refs/add", { bibtex: q.bibtex, summary: q.summary, one_liner: q.one_liner, tags: q.tags ?? [], pdf: null });
          it.state = "saved"; it.key = r.key; it.tags = q.tags ?? [];
          it.msg = `${r.key}${q.source === "text" ? "（書目取自你貼的文字）" : ""}${q.note_error ? `；簡介沒有產生：${q.note_error}` : ""}`;
        }
      } catch (e) { it.state = "error"; it.msg = e.message; }
      await keepNote(it);
      draw();
    }

    async function keepNote(it) {
      const paper = opts.paper?.();
      if (!paper || !it.key || !it.guess.note || it.noteKept) return;
      try { await api(`/api/papers/${encodeURIComponent(paper)}/citations`, { key: it.key, purpose: it.guess.note }); it.noteKept = true; } catch {}
    }

    async function check(it) {
      it.state = "checking"; it.msg = "查證中…"; draw();
      try {
        const r = await api("/api/refs/verify", it.guess);
        const v = r.verdict;
        it.candidates = v.candidates ?? [];
        if (r.exists) { it.state = "exists"; it.key = r.exists; it.work = v.work; it.msg = `已在文獻庫：${r.exists}`; await keepNote(it); }
        else if (v.status === "found") { it.work = v.work; await file(it, v.work); return; }
        else { it.state = v.status; it.msg = v.note; }
      } catch (e) { it.state = "error"; it.msg = e.message; }
      draw();
    }

    async function run() {
      if (!st.text.trim() || st.phase === "running") return;
      st.phase = "running"; st.error = ""; st.items = []; draw();
      try {
        const r = await api("/api/refs/extract", { text: st.text, engine: claudeVia() });
        st.items = r.papers.map((g) => ({ guess: g, state: "queued", msg: "" }));
      } catch (e) { st.error = e.message; st.phase = "idle"; draw(); return; }
      draw();
      // three at a time: the model's notes are the slow part
      let next = 0;
      const worker = async () => { while (next < st.items.length) { const it = st.items[next++]; await check(it); } };
      await Promise.all([worker(), worker(), worker()]);
      st.phase = "done";
      draw();
      opts.onDone?.(st.items);
    }

    const finished = (it) => ["saved", "exists", "ambiguous", "missing", "error", "skipped"].includes(it.state);

    function draw() {
      box.innerHTML = "";
      const ta = el("textarea", "batch-text");
      ta.rows = st.items.length ? 3 : 8;
      ta.placeholder = "貼上想引用的文章，可以很多篇、可以夾雜備註：參考文獻清單、標題、DOI、PMID、arXiv 都可以。\nClaude 會算出有幾篇並抄下你對每篇的備註；是否真的存在由 Crossref／arXiv／PubMed 查證，查不到的不會自動建檔。";
      ta.value = st.text;
      ta.disabled = st.phase === "running";
      ta.oninput = () => (st.text = ta.value);
      const go = el("button", "primary small", st.phase === "running" ? "處理中…" : st.items.length ? "重新分析" : "分析並匯入");
      go.disabled = st.phase === "running" || !claudeReady();
      if (!claudeReady()) go.title = "連接 Claude 後可用";
      go.onclick = run;
      const head = el("div", "row-end");
      if (st.error) head.append(el("span", "note bad", st.error));
      head.append(go);
      box.append(ta, head);
      if (st.phase === "running" && !st.items.length) { box.append(el("p", "note", "Claude 正在找出有幾篇文章…")); return; }
      if (!st.items.length) return;

      const n = st.items.length, done = st.items.filter(finished).length;
      const bar = el("div", "progress");
      const fill = el("i");
      fill.style.width = `${Math.round((done / n) * 100)}%`;
      bar.append(fill);
      const count = (s) => st.items.filter((i) => i.state === s).length;
      const sum = el("div", "progress-line");
      sum.append(el("b", "", `${done} / ${n} 篇`), el("span", "", ["saved", "exists", "ambiguous", "missing", "error"].filter((s) => count(s)).map((s) => `${LABEL[s]} ${count(s)}`).join("・")));
      box.append(bar, sum);

      const ul = el("ul", "batch-list");
      st.items.forEach((it) => {
        const li = el("li", `b-${it.state}`);
        li.append(el("span", "mark", MARK[it.state] ?? ""));
        const body = el("div", "b-body");
        const w = it.work;
        body.append(el("b", "", w?.title || it.guess.title || it.guess.doi || it.guess.arxiv));
        body.append(el("span", "meta", [w?.authors || it.guess.authors, w?.year || it.guess.year, w?.venue || it.guess.venue].filter(Boolean).join(" · ")));
        if (it.msg) body.append(el("span", "note", it.msg));
        if (it.tags?.length) { const t = el("div", "b-tags"); it.tags.forEach((x) => t.append(el("span", "tchip on", tagLabel(x)))); body.append(t); }
        if (it.guess.note) body.append(el("span", "b-note", it.guess.note));
        const acts = el("div", "b-cands");
        if (it.state === "ambiguous") it.candidates.forEach((c) => {
          const b = el("button", "ghost", `${c.title}${c.year ? `（${c.year}）` : ""}${c.authors ? ` · ${c.authors}` : ""}`);
          b.onclick = () => { it.work = c; file(it, c); };
          acts.append(b);
        });
        if ((it.state === "missing" || it.state === "ambiguous") && it.guess.title && it.guess.authors && it.guess.year) {
          const b = el("button", "ghost", "用我貼的書目建檔（資料庫查不到時）");
          b.onclick = async () => {
            it.state = "filing"; it.msg = "建檔中…"; draw();
            try {
              const q = await api("/api/refs/propose-record", { guess: it.guess, engine: chosenEngine() });
              if (q.exists) { it.state = "exists"; it.key = q.exists; }
              else if (q.error) { it.state = "error"; it.msg = q.error; }
              else { const r = await api("/api/refs/add", { bibtex: q.bibtex, summary: q.summary, one_liner: q.one_liner, tags: q.tags ?? [], pdf: null }); it.state = "saved"; it.key = r.key; it.tags = q.tags ?? []; it.msg = `${r.key}（書目取自你貼的文字）`; }
            } catch (e) { it.state = "error"; it.msg = e.message; }
            await keepNote(it); draw();
          };
          acts.append(b);
        }
        if (it.state === "error") { const b = el("button", "ghost", "重試"); b.onclick = () => (it.work ? file(it, it.work) : check(it)); acts.append(b); }
        if (["ambiguous", "missing", "error"].includes(it.state)) { const b = el("button", "ghost", "略過"); b.onclick = () => { it.state = "skipped"; it.msg = ""; draw(); }; acts.append(b); }
        if (acts.children.length) body.append(acts);
        li.append(body);
        ul.append(li);
      });
      box.append(ul);
    }

    draw();
    return {
      el: box,
      items: () => st.items,
      running: () => st.phase === "running" || st.items.some((i) => i.state === "checking" || i.state === "filing"),
      keepNotes: async () => { for (const it of st.items) await keepNote(it); },
    };
  }

  let libImporter = null;

  // ── bringing papers in: unlinked PDFs, or a DOI ──
  const queue = []; // proposals waiting for the user
  let queueBox = null; // created on first render: el() comes from app.js, loaded later
  let busy = 0;

  async function intake() {
    const row = el("div", "intake");
    const doi = el("input");
    doi.placeholder = "DOI";
    const add = el("button", "ghost", "加入");
    add.onclick = () => { if (doi.value.trim()) { propose({ doi: doi.value.trim() }); doi.value = ""; } };
    doi.onkeydown = (e) => { if (e.key === "Enter") add.click(); };
    row.append(doi, add);
    if (pdfDir) {
      let files = [];
      try { files = (await api("/api/refs/pdfs")).files.filter((f) => !f.key && !queue.some((q) => q.pdf === f.name)); } catch {}
      if (files.length) {
        const b = el("button", "primary small", `匯入 ${files.length} 篇 PDF`);
        b.onclick = () => { b.remove(); files.forEach((f) => propose({ pdf: f.name })); };
        row.prepend(b);
      }
    }
    return row;
  }

  async function propose(body) {
    busy++;
    drawQueue();
    let r;
    try { r = await api("/api/refs/propose", { ...body, engine: chosenEngine() }); }
    catch (e) { r = { error: e.message, ...body }; }
    busy--;
    if (r.exists) flash(`已在文獻庫：${r.exists}`);
    else queue.push(r);
    drawQueue();
  }

  function drawQueue() {
    if (!queueBox) return;
    queueBox.innerHTML = "";
    if (busy) queueBox.append(el("p", "note", `查詢中 ${busy} 篇…`));
    queue.forEach((q) => queueBox.append(card(q)));
  }

  function card(q) {
    const c = el("div", "card");
    const drop = () => { queue.splice(queue.indexOf(q), 1); drawQueue(); };
    const retry = (doi) => { drop(); propose({ pdf: q.pdf ?? undefined, doi }); };
    const doiFix = (label) => {
      const row = el("div", "row-end");
      const inp = el("input");
      inp.placeholder = "DOI";
      inp.value = q.doi && !q.need_doi ? q.doi : "";
      const go = el("button", "ghost", label);
      go.onclick = () => inp.value.trim() && retry(inp.value.trim());
      inp.onkeydown = (e) => { if (e.key === "Enter") go.click(); };
      row.append(inp, go);
      return row;
    };
    const head = el("div", "card-head");
    head.append(el("span", "file", q.pdf ?? q.doi ?? ""), iconButton("x", "略過", drop));
    c.append(head);
    if (q.need_doi || q.error) {
      c.append(el("p", "note bad", q.error ?? "PDF 裡找不到 DOI"), doiFix("查詢"));
      return c;
    }
    const t = el("div", "ref-title");
    t.append(el("b", "", q.title), el("span", "meta", [q.authors, q.year, q.venue].filter(Boolean).join(" · ")));
    c.append(t);
    if (q.title_mismatch) c.append(el("p", "note bad", "標題和 PDF 內容對不上：確認 DOI"), doiFix("重新查詢"));
    if (q.preprint_only) c.append(el("p", "note warn", "只有預印本"));
    const one = el("input", "one");
    one.value = q.one_liner;
    one.placeholder = "一句話：這篇在說什麼";
    const sum = el("textarea", "summary");
    sum.rows = 3;
    sum.value = q.summary;
    sum.placeholder = "簡介";
    const chips = el("div", "chips-row");
    const chosen = new Set(q.tags);
    tags.forEach((tg) => {
      const b = el("button", "tchip", tg.label);
      b.setAttribute("aria-pressed", String(chosen.has(tg.id)));
      b.onclick = () => { chosen.has(tg.id) ? chosen.delete(tg.id) : chosen.add(tg.id); b.setAttribute("aria-pressed", String(chosen.has(tg.id))); };
      chips.append(b);
    });
    const ok = el("button", "primary small", "加入");
    ok.onclick = async () => {
      try {
        const r = await api("/api/refs/add", { bibtex: q.bibtex, summary: sum.value.trim(), one_liner: one.value.trim(), tags: [...chosen], pdf: q.pdf });
        drop();
        flash(`已加入 ${r.key}`);
        refreshGit();
        renderPage();
      } catch (e) { flash(e.message, 6000); }
    };
    const foot = el("div", "row-end");
    if (q.title_mismatch) {
      // Probably the wrong record: only a deliberate, quieter "add anyway".
      ok.className = "ghost";
      ok.textContent = "仍要加入";
      foot.append(ok);
      c.append(foot);
      return c;
    }
    foot.append(ok);
    c.append(one, sum, chips, foot);
    if (q.note_error) c.append(el("p", "note", `簡介沒有產生：${q.note_error}`));
    return c;
  }

  function row(r) {
    const it = el("div", "ref-item");
    const head = el("div", "ref-head");
    const pdf = iconButton("pdf", r.pdf_found ? "開啟 PDF" : "本機找不到這篇的 PDF", () => openPdf(r));
    pdf.classList.toggle("off", !r.pdf_found);
    const title = el("div", "ref-title");
    title.append(el("b", "", r.title || r.key), el("span", "meta", [r.authors, r.year, r.venue].filter(Boolean).join(" · ")));
    head.append(pdf, title, el("code", "key", r.key));
    const one = el("div", "one-liner", r.one_liner);
    one.contentEditable = "plaintext-only";
    one.dataset.placeholder = "一句話：這篇在說什麼";
    one.onblur = () => { const v = one.textContent.trim(); if (v !== r.one_liner) saveNote(r, { one_liner: v }); };
    one.onkeydown = (e) => { if (e.key === "Enter") { e.preventDefault(); one.blur(); } };
    const tagRow = el("div", "ref-tags");
    const drawTags = () => {
      tagRow.innerHTML = "";
      r.tags.forEach((t) => tagRow.append(el("span", "tchip on", tagLabel(t))));
      tagRow.append(iconButton("tag", "標籤", (e) => tagPop(e.currentTarget, r, drawTags)));
    };
    drawTags();
    const sum = el("textarea", "summary");
    sum.rows = 3;
    sum.placeholder = "簡介";
    sum.value = r.summary;
    sum.hidden = true;
    sum.onblur = () => { if (sum.value.trim() !== r.summary) saveNote(r, { summary: sum.value.trim() }); };
    title.onclick = () => { sum.hidden = !sum.hidden; if (!sum.hidden) sum.focus(); };
    it.append(head, one, tagRow, sum);
    return it;
  }

  function pop(anchor, build) {
    document.querySelector("#refPop")?.remove();
    const p = el("div", "pop ref-pop");
    p.id = "refPop";
    p.onclick = (e) => e.stopPropagation();
    build(p);
    document.body.append(p);
    const r = anchor.getBoundingClientRect();
    p.style.top = `${Math.min(r.bottom + 6, innerHeight - p.offsetHeight - 8)}px`;
    p.style.left = `${Math.max(8, Math.min(r.left, innerWidth - p.offsetWidth - 8))}px`;
    const close = (e) => { if (!p.contains(e.target) && e.target !== anchor) { p.remove(); document.removeEventListener("mousedown", close); } };
    setTimeout(() => document.addEventListener("mousedown", close));
    return p;
  }

  function tagPop(anchor, r, redraw) {
    pop(anchor, (p) => {
      tags.forEach((t) => {
        const b = el("button", "tchip", t.label);
        b.setAttribute("aria-pressed", String(r.tags.includes(t.id)));
        b.onclick = async () => {
          const next = r.tags.includes(t.id) ? r.tags.filter((x) => x !== t.id) : [...r.tags, t.id];
          b.setAttribute("aria-pressed", String(next.includes(t.id)));
          await saveNote(r, { tags: next });
          redraw();
        };
        p.append(b);
      });
    }).classList.add("tag-pop");
  }

  function importPop(anchor) {
    pop(anchor, (p) => {
      const ta = el("textarea");
      ta.rows = 8;
      ta.placeholder = "@article{key, ...}";
      const file = el("input");
      file.type = "file";
      file.accept = ".bib,text/plain";
      file.onchange = async () => { ta.value = await file.files[0].text(); };
      const go = el("button", "primary small", "匯入");
      go.onclick = async () => {
        try {
          const r = await api("/api/refs/import", { bibtex: ta.value });
          flash(`加入 ${r.added.length} 篇${r.skipped.length ? `，略過重複 ${r.skipped.length} 篇` : ""}${r.errors.length ? `，${r.errors.length} 筆格式錯誤` : ""}`, 5000);
          p.remove();
          renderPage();
          refreshGit();
        } catch (e) { flash(e.message, 6000); }
      };
      const row2 = el("div", "row-end");
      row2.append(file, go);
      p.append(ta, row2);
    }).classList.add("import-pop");
  }

  function pdfPop(anchor) {
    pop(anchor, (p) => {
      const input = el("input");
      input.value = pdfDir ?? "";
      input.placeholder = "/Users/you/Papers";
      const go = el("button", "primary small", "儲存");
      go.onclick = async () => {
        try { await api("/api/settings/pdf-dir", { path: input.value }); flash("已設定"); p.remove(); renderPage(); }
        catch (e) { flash(e.message, 6000); }
      };
      const row2 = el("div", "row-end");
      row2.append(input, go);
      p.append(row2);
    }).classList.add("pdf-pop");
  }

  // ── insert citation ──
  /** Where to insert: the open paragraph editor's caret, else the end of the
   *  selected paragraph. Counted in non-space characters so it survives the
   *  one-sentence-per-line rewrite on save. */
  function target() {
    const ta = document.querySelector(".para textarea");
    if (ta) {
      const p = ta.closest(".para");
      const before = ta.value.slice(0, ta.selectionStart);
      return { si: +p.dataset.s, pi: +p.dataset.p, nonWs: before.replace(/\s/g, "").length, ta };
    }
    return { si: state.cur.s, pi: state.cur.p, nonWs: null };
  }

  /** `sep` goes between the text before and `cite`: "~" keeps a citation on
   *  the line of its word; a reference (Fig.~\ref) takes a space. */
  function insertAt(text, nonWs, cite, sep = "~") {
    let at;
    if (nonWs == null) {
      // end of the paragraph, before its final punctuation
      const m = text.match(/[.!?]\s*$/);
      at = m ? text.length - m[0].length : text.trimEnd().length;
    } else {
      let seen = 0;
      at = 0;
      while (at < text.length && seen < nonWs) { if (!/\s/.test(text[at])) seen++; at++; }
    }
    const pre = text.slice(0, at).replace(/\s+$/, "");
    const post = text.slice(at).replace(/^[ \t]+/, "");
    const glue = pre && !pre.endsWith("~") && !(sep === " " && /[([{]$/.test(pre)) ? sep : "";
    const after = post && !/^[\s.,;:!?)\]]/.test(post) ? " " : "";
    return `${pre}${glue}${cite}${after}${post}`;
  }

  /** `given` ({ si, pi, nonWs, vet?, done? }, context.js): insert there instead of
   *  at the caret; vet(before, after) may stop it, done(before, after) follows. */
  async function picker(anchor, given) {
    if (!state.paper || !curSection()) return flash("先開啟一篇論文");
    const at = given ?? target();
    if (at.ta) at.ta.blur(); // save the paragraph being edited first
    await load();
    const f = { q: "", tags: new Set() };
    const chosen = new Set();
    let lastReq = null; // the Claude request as the user last edited it
    pop(anchor, (p) => {
      p.classList.add("cite-pop");
      const search = el("input", "search");
      search.type = "search";
      search.placeholder = "搜尋文獻庫";
      const list = el("div", "cite-list");
      const purpose = el("input", "purpose");
      purpose.placeholder = "在這裡用來說明…（選填）";
      const go = el("button", "primary small", "插入");
      const draw = () => {
        list.innerHTML = "";
        const shown = refs.filter((r) => matches(r, f));
        if (!refs.length) list.append(el("p", "empty", "文獻庫是空的"));
        shown.forEach((r) => {
          const b = el("button", "cite-row");
          b.setAttribute("aria-pressed", String(chosen.has(r.key)));
          const main = el("span", "c-main");
          main.append(el("b", "", r.one_liner || r.title || r.key), el("span", "meta", [r.authors, r.year, r.venue].filter(Boolean).join(" · ")));
          const side = el("span", "c-side");
          r.tags.slice(0, 3).forEach((t) => side.append(el("span", "tchip on", tagLabel(t))));
          if (r.pdf_found) side.append(icon("pdf"));
          b.append(main, side);
          b.onclick = () => { chosen.has(r.key) ? chosen.delete(r.key) : chosen.add(r.key); b.setAttribute("aria-pressed", String(chosen.has(r.key))); go.textContent = chosen.size ? `插入 ${chosen.size}` : "插入"; };
          b.ondblclick = () => { chosen.add(r.key); go.click(); };
          list.append(b);
        });
      };
      search.oninput = () => { f.q = search.value; draw(); };
      search.onkeydown = (e) => { if (e.key === "Enter" && chosen.size) go.click(); };
      go.onclick = async () => {
        if (!chosen.size) return;
        const keys = [...chosen];
        p.remove();
        if (!given) await new Promise((r) => setTimeout(r, 250)); // let the paragraph save land
        const sec = sections()[at.si];
        if (!sec) return;
        const cite = `\\cite{${keys.join(",")}}`;
        const was = sec.paragraphs.length ? sec.paragraphs[at.pi] : null;
        const now = was == null ? cite : insertAt(was, at.nonWs, cite);
        if (at.vet && !(await at.vet(was ?? "", now))) return;
        if (was == null) sec.paragraphs.push(now);
        else sec.paragraphs[at.pi] = now;
        try { await saveSection(at.si); } catch (e) { flash(`儲存失敗：${e.message}`); return; }
        const name = encodeURIComponent(state.paper.name);
        if (purpose.value.trim()) for (const k of keys) await api(`/api/papers/${name}/citations`, { key: k, purpose: purpose.value.trim() }).catch(() => {});
        try {
          const r = await api(`/api/papers/${name}/bib`, {});
          if (r.missing.length) flash(`文獻庫裡沒有：${r.missing.join(", ")}`, 6000);
        } catch (e) { flash(e.message, 6000); }
        refreshSection(at.si);
        refreshGit();
        at.done?.(was ?? "", now);
      };
      const foot = el("div", "row-end");
      foot.append(purpose, go);
      // Search for new papers: a Claude feature, greyed out without Claude.
      const claudeOn = !!state.engines && (state.engines.subscription.configured || state.engines.api.configured);
      const find = iconButton("search", claudeOn ? "用 Claude 找新文獻" : "連接 Claude 後可搜尋新文獻", () => searchMode());
      find.disabled = !claudeOn;
      const top = el("div", "row-end");
      top.append(search, find);
      const listMode = el("div", "mode");
      listMode.append(top, tagChips(f, draw), list, foot);
      const searchBox = el("div", "mode");
      searchBox.hidden = true;
      p.append(listMode, searchBox);
      draw();
      setTimeout(() => search.focus());

      function searchMode() {
        listMode.hidden = true;
        searchBox.hidden = false;
        searchBox.innerHTML = "";
        const req = el("textarea", "request");
        req.rows = 3;
        // What the user typed wins; then the request they last wrote here;
        // only then a suggestion from the paragraph.
        const typed = search.value.trim();
        if (typed) req.value = typed;
        else if (lastReq != null) req.value = lastReq;
        else {
          const para = sections()[at.si]?.paragraphs[at.pi] ?? "";
          const lead = para.replace(/\\[a-zA-Z]+\*?(\[[^\]]*\])?(\{[^}]*\})?/g, " ").replace(/[{}$~%]/g, " ").split(/(?<=[.!?。])\s+/).slice(0, 2).join(" ").replace(/\s+/g, " ").trim();
          req.value = lead ? `幫我找關於「${lead}」的文獻` : "";
        }
        req.oninput = () => { lastReq = req.value; };
        req.placeholder = "幫我找…";
        const want = { q: "", tags: new Set() };
        const back = iconButton("back", "回到文獻庫", () => { searchBox.hidden = true; listMode.hidden = false; draw(); });
        const send = iconButton("send", "搜尋 Ctrl+Enter", () => run());
        req.onkeydown = (e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) run(); };
        const head = el("div", "row-end");
        head.append(back, req, send);
        const results = el("div", "cite-list");
        searchBox.append(head, tagChips(want, () => {}), results);
        setTimeout(() => { req.focus(); req.setSelectionRange(req.value.length, req.value.length); });

        async function run() {
          if (!req.value.trim()) return req.focus();
          results.innerHTML = "";
          results.append(el("p", "note", "Claude 搜尋中…"));
          send.disabled = true;
          let r;
          try { r = await api("/api/refs/search", { request: req.value.trim(), tags: [...want.tags], engine: claudeVia() }); }
          catch (e) { r = { error: e.message }; }
          send.disabled = false;
          results.innerHTML = "";
          if (r.queries?.length) results.append(el("p", "note", r.queries.join(" · ")));
          if (r.error && !r.results?.length) results.append(el("p", "note bad", r.error));
          (r.results ?? []).forEach((c) => results.append(hit(c)));
          if (r.results && !r.results.length && !r.error) results.append(el("p", "note", "沒有找到"));
        }

        function hit(c) {
          const row = el("div", "hit-row");
          const main = el("span", "c-main");
          const t = el("b", "link", c.title);
          t.onclick = () => api("/api/open-link", { doi: c.doi }).catch((e) => flash(e.message));
          main.append(t, el("span", "meta", [c.authors, c.year, c.venue, `cited ${c.cited_by}`].filter(Boolean).join(" · ")));
          if (c.reason) main.append(el("span", "why", c.reason));
          const side = el("span", "c-side");
          if (c.oa_url) side.append(iconButton("pdf", "開啟免費 PDF", () => api("/api/open-link", { url: c.oa_url }).catch((e) => flash(e.message))));
          const add = el("button", "ghost", c.in_library ? "已在文獻庫" : "加入");
          const pick = (key) => { chosen.add(key); go.textContent = `插入 ${chosen.size}`; add.textContent = "✓ 已選"; add.disabled = true; };
          add.onclick = async () => {
            if (c.in_library) return pick(c.in_library);
            add.disabled = true;
            add.textContent = "加入中…";
            try {
              const prop = await api("/api/refs/propose", { doi: c.doi, engine: chosenEngine() });
              if (prop.exists) return pick(prop.exists);
              if (prop.error || !prop.bibtex) throw new Error(prop.error ?? "無法取得書目");
              const r = await api("/api/refs/add", { bibtex: prop.bibtex, summary: prop.summary, one_liner: prop.one_liner || c.reason, tags: prop.tags });
              await load();
              refreshGit();
              pick(r.key);
            } catch (e) { flash(e.message, 6000); add.disabled = false; add.textContent = "加入"; }
          };
          side.append(add);
          row.append(main, side);
          return row;
        }
      }
    });
  }

  // ── this paper's references ──
  // What the paper cites, in citation order, with what each one is used for.
  async function paperPage() {
    const box = $("#paperRefs");
    box.innerHTML = "";
    if (!state.paper) { box.append(el("p", "empty", "先開啟一篇論文")); return; }
    const name = encodeURIComponent(state.paper.name);
    await load();
    let c;
    try { c = await api(`/api/papers/${name}/citations`); } catch (e) { box.append(el("p", "empty", e.message)); return; }
    const bar = el("div", "refs-bar");
    const add = el("button", "primary small", "插入引用…");
    add.onclick = () => { $("#dlgPaperRefs").close(); PCRefs.picker($("#wbCite") ?? $("#actRefs")); };
    bar.append(add);
    box.append(bar);
    const local = c.local ?? [];
    const find = (k) => refs.find((x) => x.key === k) ?? local.find((x) => x.key === k);
    const missing = c.cited.filter((k) => !find(k));
    $("#paperRefsCount").textContent = `${c.cited.length} 篇${missing.length ? ` · ${missing.length} 篇找不到書目` : ""}`;
    if (!c.cited.length) { box.append(el("p", "empty", "這篇還沒有引用")); return; }
    const list = el("ol", "paper-ref-list");
    c.cited.forEach((key, i) => {
      const r = find(key);
      const li = el("li", r ? "" : "missing");
      const num = PCTex.citeNumber?.(key) ?? String(i + 1);
      li.append(el("span", "n", `[${num}]`));
      const main = el("div", "body");
      if (r) {
        const t = el("div", "title", r.title || key);
        const meta = [r.authors, r.year, r.venue].filter(Boolean).join(" · ");
        main.append(t, el("div", "meta", refs.includes(r) ? meta : `${meta}　（論文的 .bib）`));
        if (r.one_liner) main.append(el("div", "one", r.one_liner));
      } else {
        main.append(el("div", "title", key), el("div", "meta", "找不到書目"));
      }
      const purpose = el("input", "purpose");
      purpose.placeholder = "在這篇用來說明…";
      purpose.value = c.purposes[key] ?? "";
      purpose.onchange = async () => {
        try { await api(`/api/papers/${name}/citations`, { key, purpose: purpose.value }); refreshGit(); }
        catch (e) { flash(e.message, 5000); }
      };
      main.append(purpose);
      li.append(main);
      list.append(li);
    });
    box.append(list);
  }

  return { renderPage, picker, insertAt, paperPage, importer, load };
})();
