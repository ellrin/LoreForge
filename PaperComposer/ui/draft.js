// 討論草稿: build a paper draft through a conversation.
//   1 主題      key value, the "interesting" hypothesis, the lens
//   2 實驗記錄  upload records → experiments and a timeline
//   3 題組      question groups per section (editable)
//   4 對話      one question at a time; the ledger on the right says what
//               each item still lacks (decided by the server from the lens)
//   4.5 主張    claim register: every claim once, with its home section
//   5 校正      narrative chain and suggestions (accept / reject)
//   ★ 版型      the journal template, up to 3 suggested; 稍後 keeps the plain
//               article. Changing it re-lays main.tex; the sections keep every word
//   6 大綱      paragraph-level outline, revised section by section
//   7 草稿      drafted through the phrase bank, section by section
//   8 審稿      reviewer pass; questions go back to the conversation
// Every step runs on Claude when it is available, else on the local model;
// the engine that ran shows next to the step's button (engineTag). Each
// step's explanation sits behind a 「?」 next to its heading (hq).
// Everything is saved in the paper folder (draft/), so it can be resumed.
// 重新開始 moves draft/ to draft/archive/<time>/; 從這步重來 (on a finished
// step) clears what the later steps produced, after a copy in the archive.
const PCDraft = (() => {
  const STEPS = [
    ["topic", "1", "主題"],
    ["record", "2", "實驗記錄"],
    ["plan", "3", "題組"],
    ["interview", "4", "對話"],
    ["claims", "4.5", "主張"],
    ["review", "5", "校正"],
    ["template", "★", "版型"],
    ["outline", "6", "大綱"],
    ["draft", "7", "草稿"],
    ["audit", "8", "審稿"],
    ["revise", "9", "修訂"],
  ];
  const SECTIONS = ["Introduction", "Methods", "Results", "Discussion", "Conclusion"];
  const SEC_ZH = { Introduction: "前言", Methods: "方法", Results: "結果", Discussion: "討論", Conclusion: "結論", Abstract: "摘要" };
  const REV_ZH = { open: "討論中", planned: "有修改方案", done: "完成", dropped: "放棄" };
  const DIR_ZH = { matches: "符合預期", contradicts: "與預期相反", unsure: "不確定" };
  const ACT_ZH = { "replace-text": "改摘要", "insert-after": "插在這段之後", "insert-before": "插在這段之前", replace: "取代這段", append: "加在章節最後" };
  const STATUS_ZH = { pending: "未開始", active: "進行中", done: "完成", skipped: "略過" };
  let D = null;

  const base = () => `/api/papers/${encodeURIComponent(D.name)}/draft`;
  const sess = () => D.data.session;
  const lens = () => D.data.lens;

  async function open() {
    if (!state.paper?.name) { LoreShell.toast("先開啟一篇論文"); return; }
    const d = $("#dlgDraft");
    if (!d.open) d.showModal();
    if (!D || D.name !== state.paper.name) D = { name: state.paper.name, data: null, step: null, group: null, busy: null, err: null };
    if (!D.tableStyles) { try { D.tableStyles = await api("/api/textable/styles"); } catch { D.tableStyles = []; } }
    $("#dfModel").value = store.get("pc.draftModel", "auto");
    $("#dfModel").onchange = (e) => store.set("pc.draftModel", e.target.value);
    $("#dfRestart").onclick = async (e) => {
      if (!D?.data || D.busy) return;
      if (!(await LoreShell.ask(e.currentTarget, ["重新開始？", "目前的討論會移到 draft/archive/。"], [{ label: "取消", value: null }, { label: "重新開始", value: true, danger: true, primary: true }]))) return;
      run("重新開始", async () => { const v = await call("/restart"); D.step = 0; D.group = null; D.comments = null; D.draftAnswer = null; return v; });
    };
    await reload();
  }

  async function reload() {
    try {
      D.data = await api(base());
      schedulePrep();
      if (D.step == null) D.step = Math.max(0, STEPS.findIndex(([k]) => k === sess().phase));
      D.err = null;
    } catch (e) { D.err = e.message; }
    draw();
  }

  /** Run a call that returns the new view; keep the window responsive. */
  async function run(label, fn) {
    if (D.busy) return;
    D.busy = label; D.err = null; draw();
    try { const v = await fn(); if (v) D.data = v; }
    catch (e) { D.err = e.message; }
    D.busy = null; draw();
    schedulePrep();
  }

  /** 預想選項 are prepared in the background: look again until every group has them. */
  function schedulePrep() {
    clearTimeout(D.prepTimer);
    if (!D.data || !(sess().groups.some((g) => g.prep === "running") || sess().figures?.running)) return;
    D.prepTimer = setTimeout(async () => {
      if (!$("#dlgDraft").open || D.busy) { schedulePrep(); return; }
      try { D.data = await api(base()); draw(); } catch { /* next time */ }
      schedulePrep();
    }, 3000);
  }
  const call = (path, body, method) => api(base() + path, body ?? {}, method);
  const go = (i) => { D.step = i; draw(); };

  // ── names from the lens ──────────────────────────────
  const levelName = (d) => { const l = lens().scale.levels[d]; return l ? `${l.id} ${l.label}` : `${d}`; };
  const modeName = (m) => lens().modes.find((x) => x.id === m)?.label ?? m;
  const flagName = (f) => lens().flags.find((x) => x.id === f)?.label ?? f;
  function missingText(m) {
    if (m.kind === "mode") return "待分類";
    if (m.kind === "depth") return `需到 ${levelName(m.what)}`;
    if (m.kind === "flag") return `需要「${flagName(m.what)}」`;
    if (m.kind === "number") return "需要數字";
    if (m.kind === "explain") return "待說明";
    return m.kind;
  }

  // ── references ───────────────────────────────────────
  const refOf = (key) => D.data.material?.refs.find((r) => r.key === key);
  function openDoi(doi) { api("/api/open-link", { doi }).catch((e) => LoreShell.toast(e.message, { err: true })); }
  /** A reference as a chip: key, short title, DOI link when there is one. */
  function refChip(key) {
    const r = refOf(key);
    const c = el("span", "df-ref");
    c.append(el("b", "", key));
    if (r?.title) c.append(el("span", "", ` ${r.title.length > 60 ? r.title.slice(0, 58) + "…" : r.title}${r.year ? ` (${r.year})` : ""}`));
    if (r?.doi) { const a = el("button", "df-doi", "DOI"); a.title = `https://doi.org/${r.doi}`; a.onclick = () => openDoi(r.doi); c.append(a); }
    return c;
  }
  function refsPanel(main) {
    const refs = D.data.material?.refs ?? [];
    main.append(h(`本篇文獻（${refs.length}）`));
    const r = row(btn("本篇文獻", () => openWindow("dlgPaperRefs"), "ghost"), btn(D.refsOpen ? "收起貼上" : "貼上要引用的文章", () => { D.refsOpen = !D.refsOpen; draw(); }, "ghost"), btn("重新讀取", () => reload(), "ghost"));
    main.append(r);
    if (D.refsOpen) {
      // the old new-paper wizard's 文獻 step: each paper checked and filed,
      // the note on it kept as its purpose in this paper
      D.refImporter ??= PCRefs.importer({ paper: () => D?.name ?? null, onDone: () => reload() });
      main.append(D.refImporter.el);
    }
    if (!refs.length) return;
    const ul = el("ul", "df-reflist");
    refs.forEach((x) => {
      const li = el("li");
      li.append(refChip(x.key));
      const meta = [x.tags.join("、"), x.purpose].filter(Boolean).join("　");
      if (meta) li.append(el("div", "df-note", meta));
      ul.append(li);
    });
    main.append(ul);
  }

  // ── frame ────────────────────────────────────────────
  function draw() {
    if (!D) return;
    const main = $("#dfMain"), side = $("#dfSide"), steps = $("#dfSteps");
    main.innerHTML = ""; side.innerHTML = ""; steps.innerHTML = "";
    if (!D.data) { main.append(el("p", "df-note bad", D.err ?? "載入中…")); return; }
    $("#dfLens").textContent = `視角：${lens().name}`;
    const phaseAt = STEPS.findIndex(([k]) => k === sess().phase);
    STEPS.forEach(([k, n, label], i) => {
      // the journal is chosen in step 1: ★ 版型 only shows for a session that has none yet
      if (k === "template" && D.data.journal && D.step !== i) return;
      const li = el("li");
      const b = el("button");
      b.append(el("span", "c", n), el("span", "", label));
      b.setAttribute("aria-current", String(i === D.step));
      if (i < phaseAt) b.classList.add("done");
      b.onclick = () => go(i);
      li.append(b);
      if (i < phaseAt) {
        const r = el("button", "df-rewind", "從這步重來");
        r.disabled = !!D.busy;
        r.onclick = async () => {
          if (!(await LoreShell.ask(r, [`回到「${label}」？`, "之後步驟的結果會清掉（先存一份在 draft/archive/）。"], [{ label: "取消", value: null }, { label: "從這步重來", value: true, danger: true, primary: true }]))) return;
          run("重來", async () => { const v = await call("/rewind", { phase: k }); D.step = i; return v; });
        };
        li.append(r);
      }
      steps.append(li);
    });
    if (D.busy) main.append(el("div", "df-busy", `${D.busy}…（模型思考中，可以先看其他步驟）`));
    if (D.err) main.append(el("div", "df-note bad", D.err));
    const key = STEPS[D.step][0];
    ({ topic, record, plan, interview, claims, review, template, outline, draft: drafting, audit, revise })[key](main, side);
    $("#dlgDraft").classList.toggle("with-side", side.childElementCount > 0);
  }

  const h = (text) => el("h3", "df-h", text);
  /** A heading with its explanation behind a 「?」 (the app's help tip). */
  function hq(text, help) {
    const t = h(text);
    const q = el("button", "help", "?");
    q.dataset.tip = help;
    q.setAttribute("aria-label", "說明");
    t.append(q);
    return t;
  }
  /** Which engine produced a result, in a few characters (why, on hover). */
  function engineTag(label, why) {
    if (!label) return document.createTextNode("");
    const t = el("span", `df-engine${label === "本機" ? " local" : ""}`, label);
    t.title = why ? `改用${label}：${why}` : `由 ${label} 產生`;
    return t;
  }
  const stepTag = (k) => engineTag(sess().engines?.[k]?.label, sess().engines?.[k]?.note);
  const note = (text, cls = "") => el("p", `df-note ${cls}`, text);
  function btn(label, onclick, cls = "primary small") {
    const b = el("button", cls, label);
    b.onclick = onclick;
    b.disabled = !!D.busy;
    return b;
  }
  function row(...kids) { const r = el("div", "df-row"); r.append(...kids); return r; }
  function input(value, placeholder, tag = "input") {
    const i = el(tag, "np-input");
    i.value = value ?? "";
    if (placeholder) i.placeholder = placeholder;
    if (tag === "textarea") i.rows = 2;
    return i;
  }
  const avail0 = () => lens().available.length > 0;
  const nextBtn = (label) => btn(label ?? "下一步", () => go(D.step + 1), "ghost");

  // ── 1 主題 ───────────────────────────────────────────
  function topic(main) {
    const kvHead = el("div", "np-label");
    kvHead.append(el("span", "", "Key value"));
    const count = el("span", "np-count");
    kvHead.append(count);
    const kv = input(D.data.key_value, "這篇論文通篇要強調的一件事，例如：提升髖部骨折判讀的可解釋性");
    kv.classList.add("kv");
    const recount = () => { const [n, max, unit] = PCNew.kvLength(kv.value); count.textContent = `${n} / ${max} ${unit}`; count.classList.toggle("bad", n > max); };
    kv.oninput = recount;
    recount();
    main.append(kvHead, kv);
    PCNew.kvCheck(kv);
    // the journal first: plan, 預想選項, outline and 審稿 follow its habits (期刊寫法)
    const jHead = hq("投稿期刊", "規劃、預想選項、大綱、草稿與審稿都照這本期刊的寫法與投稿要求。按「儲存，下一步」才套用；之後也能換，大綱會照新期刊重新產生。");
    main.append(jHead);
    const current = D.data.journal ?? "";
    D.pickJournal ??= current;
    const jbox = el("div", "df-journal");
    const showList = () => {
      // a pick folds the list to one line with the journal's 期刊寫法 under it
      jbox.replaceChildren(PCNew.templateList({ tpls: D.tpls ?? [], selected: D.pickJournal, suggested: [], journalsOnly: true, paper: D.name, onPick: (v) => { D.pickJournal = v; D.journalOpen = false; setTimeout(showChosen); } }));
    };
    const showChosen = () => {
      const t = (D.tpls ?? []).find((x) => x.id === D.pickJournal);
      const r = row(el("b", "", t ? t.name : D.pickJournal), btn("換期刊", () => { D.journalOpen = true; showList(); }, "ghost"));
      jbox.replaceChildren(r);
      if (typeof PCVenue !== "undefined" && t) jbox.append(PCVenue.panel({ template: t.id, paper: D.name }));
    };
    (async () => {
      if (!D.tpls) { try { D.tpls = (await api("/api/templates")).filter((j) => j.kind !== "book"); } catch { D.tpls = []; } }
      if (D.pickJournal && !D.journalOpen) showChosen(); else showList();
    })();
    main.append(jbox);
    main.append(h("有意思的地方（假說，之後對話會再確認）"));
    const acc = input(sess().hook.reader_accepts, "同行已經承認的事實是什麼？例如：標註不確定的影像通常直接排除", "textarea");
    const tw = input(sess().hook.twist, "你加了什麼讓人意外的東西？例如：不排除，而是依品質與面積降權", "textarea");
    main.append(el("label", "np-label", "讀者已經承認"), acc, el("label", "np-label", "你的轉折"), tw);
    main.append(hq("視角", avail0() ? "視角檔在工作區的 lens/ 資料夾：決定要追問什麼、怎麼判讀回答。" : "工作區沒有 lens/ 資料夾，使用內建的中性視角。可以從 private-example 複製 lens/default.toml 改成你自己的。"));
    const avail = lens().available;
    const sel = el("select", "np-input");
    (avail.length ? avail : ["default"]).forEach((n) => { const o = el("option", "", n === "default" && !avail.length ? "default（內建）" : n); o.value = n; sel.append(o); });
    sel.value = sess().lens || lens().name;
    main.append(sel);
    const levels = el("ul", "df-levels");
    lens().scale.levels.forEach((l) => levels.append(el("li", "", `${l.id} ${l.label}${l.means ? `：${l.means}` : ""}`)));
    main.append(levels);
    main.append(row(btn("儲存，下一步", () => run("儲存", async () => {
      const pick = D.pickJournal ?? "";
      if (pick !== (D.data.journal ?? "")) {
        if (sess().outline.length && !(await LoreShell.ask($("#dfMain"), ["換投稿期刊？", "章節照新期刊排，大綱要重新產生。"], [{ label: "取消", value: null }, { label: "換期刊", value: true, primary: true }]))) return null;
        await call("/template", { template: pick || null, stay: true }, "PUT");
        if (state.paper?.name === D.name) openPaper(D.name);
      }
      D.journalOpen = false;
      const v = await call("/topic", { key_value: kv.value, reader_accepts: acc.value, twist: tw.value, lens: sel.value }, "PUT");
      if (state.paper?.name === D.name && state.paper.meta) { state.paper.meta.key_value = v.key_value; renderKeyValue(); }
      D.step = 1;
      return v;
    }))));
    setTimeout(() => { if (!kv.value) kv.focus(); });
    refsPanel(main);
  }

  // ── 2 實驗記錄 ───────────────────────────────────────
  function record(main) {
    main.append(hq("上傳實驗記錄", "接受 .md .txt .csv .tsv .json .log 與 PDF；Excel、Word 請另存成 CSV 或純文字。記錄裡的順序（日期、版本、run 編號）越完整，越能問出你當時的想法；你在記錄裡寫下的想法，會在問答時當作選項給你。論文裡已有的表格、圖與文獻會自動帶入，不用重貼。沒有 Claude 時由本機模型分段整理，表格裡在記錄找不到的數字會標出來。"));
    const file = el("input");
    file.type = "file"; file.multiple = true;
    file.onchange = () => run("上傳", async () => {
      let v = null;
      for (const f of file.files) {
        const r = await fetch(`${base()}/records?file=${encodeURIComponent(f.name)}`, { method: "POST", body: f });
        if (!r.ok) throw new Error(`${f.name}：${await r.text()}`);
        v = await r.json();
      }
      return v;
    });
    // a path typed or pasted (LoreShelf's 「拷貝路徑」, Finder's ⌥⌘C): a file, or every file in a folder
    const path = el("input", "df-path");
    path.type = "text";
    path.placeholder = "或貼上路徑（檔案或資料夾）";
    path.setAttribute("aria-label", "實驗記錄的路徑");
    const addPath = () => {
      const v = path.value.trim();
      if (!v) return;
      run("加入", async () => {
        const r = await fetch(`${base()}/records/path`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: v }) });
        if (!r.ok) throw new Error(await r.text());
        const skipped = Number(r.headers.get("x-skipped") || 0);
        if (skipped) LoreShell.toast(`略過 ${skipped} 個不能用的檔案`);
        path.value = "";
        return r.json();
      });
    };
    path.onkeydown = (e) => { if (e.key === "Enter") { e.preventDefault(); addPath(); } };
    // 搜尋檔案: LoreShelf's catalog (brought up in the background when needed); a click adds the file
    const find = el("div", "df-find");
    find.hidden = true;
    const fq = el("input", "df-path");
    fq.type = "search";
    fq.placeholder = "檔名、內容或一句描述";
    fq.setAttribute("aria-label", "搜尋檔案");
    const hits = el("ul", "df-list df-hits");
    let seq = 0;
    const search = async () => {
      const q = fq.value.trim();
      const my = ++seq;
      if (!q) { hits.replaceChildren(); return; }
      hits.replaceChildren(el("li", "df-muted", "搜尋中…"));
      try {
        const r = await fetch(`/api/shelf/search?q=${encodeURIComponent(q)}`);
        if (my !== seq) return;
        if (!r.ok) { hits.replaceChildren(el("li", "df-muted", await r.text())); return; }
        const v = await r.json();
        if (my !== seq) return;
        if (!v.items.length) { hits.replaceChildren(el("li", "df-muted", "沒有找到")); return; }
        hits.replaceChildren(...v.items.map((it) => {
          const li = el("li", "df-hit");
          const date = (it.modified || "").slice(0, 10);
          li.append(el("span", "df-hit-name", it.name), el("span", "df-muted", [date, it.size ? `${Math.ceil(it.size / 1024)} KB` : ""].filter(Boolean).join(" · ")));
          li.tabIndex = 0;
          const add = () => run("加入", async () => {
            const x = await fetch(`${base()}/records/shelf`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: it.id }) });
            if (!x.ok) throw new Error(await x.text());
            const skipped = Number(x.headers.get("x-skipped") || 0);
            if (skipped) LoreShell.toast(`略過 ${skipped} 個不能用的檔案`);
            return x.json();
          });
          li.onclick = add;
          li.onkeydown = (e) => { if (e.key === "Enter") add(); };
          return li;
        }));
      } catch (e) { if (my === seq) hits.replaceChildren(el("li", "df-muted", String(e.message || e))); }
    };
    let timer = null;
    fq.oninput = () => { clearTimeout(timer); timer = setTimeout(search, 300); };
    fq.onkeydown = (e) => { if (e.key === "Enter") { e.preventDefault(); clearTimeout(timer); search(); } };
    find.append(fq, hits);
    const findBtn = btn("搜尋檔案", () => { find.hidden = !find.hidden; if (!find.hidden) fq.focus(); }, "small");
    main.append(row(file, findBtn, path, btn("加入", addPath, "small")), find);
    const list = el("ul", "df-list");
    sess().records.forEach((r) => {
      const li = el("li", "", `${r.file}（${Math.ceil(r.bytes / 1024)} KB）`);
      li.append(iconButton("trash", "移除", () => run("移除", async () => {
        const x = await fetch(`${base()}/records/${encodeURIComponent(r.file)}`, { method: "DELETE" });
        if (!x.ok) throw new Error(await x.text());
        return x.json();
      })));
      list.append(li);
    });
    if (sess().records.length) main.append(list);
    main.append(row(btn(sess().experiments.length ? "重新整理" : "整理實驗記錄", () => run("整理實驗記錄", () => call("/record"))), stepTag("record"), nextBtn()));
    if (sess().record_summary) main.append(h("摘要"), el("p", "df-text", sess().record_summary));
    if (sess().experiments.length) {
      main.append(h("實驗"));
      const t = el("table", "df-table");
      t.innerHTML = "<tr><th>#</th><th>實驗</th><th>改了什麼</th><th>結果</th><th>目的</th><th>模式</th></tr>";
      sess().experiments.forEach((e) => {
        const tr = el("tr");
        [e.id, e.title, e.change, `${e.metrics} ${e.result}`.trim(), e.purpose, e.mode ? modeName(e.mode) : "?"].forEach((x, i) => {
          const td = el("td", "", x);
          if (i === 4 && !e.purpose_sure) td.append(el("span", "df-badge warn", "待確認"));
          tr.append(td);
        });
        t.append(tr);
      });
      main.append(t);
    }
    pendingBlock(main);
    if (sess().conflicts.length) {
      main.append(h("記錄與論文對不上的數字"));
      const ul = el("ul", "df-list warn");
      sess().conflicts.forEach((c) => ul.append(el("li", "", c)));
      main.append(ul);
    }
    if (sess().thoughts.length) {
      main.append(h("記錄裡你的想法"));
      const ul = el("ul", "df-thoughts");
      sess().thoughts.forEach((t) => { const li = el("li"); li.append(el("span", "df-gid", t.id), document.createTextNode(` ${t.text}`)); if (t.about.length) li.append(el("span", "df-role", t.about.join(" "))); ul.append(li); });
      main.append(ul);
    }
    main.append(h("加入圖表"));
    D.floats ??= PCNew.floats({
      paper: D.name,
      sections: (state.paper?.sections ?? []).map((x) => x.title),
      styles: D.tableStyles ?? [],
      keyValue: D.data.key_value,
      title: "",
      figuresBefore: (D.data.material?.floats ?? []).filter((f) => f.kind !== "table").length,
      onSaved: (v) => { D.data = v; D.floats = null; if (state.paper?.name) openPaper(state.paper.name); draw(); },
    });
    main.append(D.floats.el);
    if (sess().tables.length) {
      main.append(h("從記錄整理出的表格"));
      const secs = (state.paper?.sections ?? []).map((x) => x.title);
      sess().tables.forEach((t) => {
        const card = el("div", "df-card");
        card.append(row(el("span", "df-gid", t.id), el("b", "", t.caption), t.inserted ? el("span", "df-badge flag", "已放進論文") : el("span")));
        if (t.note) card.append(note(t.note));
        card.append(holeTable(t));
        if (!t.inserted) {
          const pick = el("select", "np-input df-home");
          (secs.length ? secs : ["Results"]).forEach((x) => { const o = el("option", "", `放進 ${x}`); o.value = x; pick.append(o); });
          const res = secs.find((x) => /result/i.test(x)); if (res) pick.value = res;
          card.append(row(pick, btn("插入論文", () => run("插入表格", async () => { const v = await call(`/tables/${t.id}/insert`, { section: pick.value }); if (state.paper?.name) openPaper(state.paper.name); return v; }), "ghost")));
        }
        main.append(card);
      });
    }
    const floats = D.data.material?.floats ?? [];
    if (floats.length) {
      main.append(h("論文裡已有的圖表"));
      const ul = el("ul", "df-list");
      floats.forEach((f) => ul.append(el("li", "", `${f.kind === "table" ? "表" : "圖"} ${f.id}（${f.section}）：${f.caption || "（沒有標題）"}`)));
      main.append(ul);
    }
    if (sess().timeline.length) {
      main.append(h("時間線"));
      const ol = el("ol", "df-timeline");
      sess().timeline.forEach((s) => {
        const li = el("li");
        li.append(el("b", "", `T${s.step} ${s.what}`));
        if (s.observed) li.append(el("div", "", `看到：${s.observed}`));
        if (s.changed) li.append(el("div", "", `接著：${s.changed}`));
        ol.append(li);
      });
      main.append(ol);
    }
  }

  // ── 3 題組 ───────────────────────────────────────────
  function plan(main) {
    const started = sess().groups.some((g) => g.turns > 0);
    main.append(hq("題組", "每一組最後要整理出：說明、動機、結論。題組數只是估計，對話後還會校正。"));
    // the figure plan is at the end of this page: a way there with its counts
    const fpl = sess().figures;
    if (fpl?.items?.length || fpl?.running) {
      const n = (k) => (fpl.items ?? []).filter((x) => x.kind === k && x.status !== "removed").length;
      const jump = btn(fpl.running ? "圖表規劃中…" : `圖表規劃：圖 ${n("figure")}、表 ${n("table")} ↓`, () => $("#dfFigPlan")?.scrollIntoView({ behavior: "smooth" }), "ghost small");
      jump.disabled = false;
      main.append(row(jump));
    }
    const r = row();
    if (!started) r.append(btn(sess().groups.length ? "重新規劃" : "規劃題組", () => run("規劃題組", () => call("/plan"))), stepTag("plan"));
    else r.append(note("已經開始對話，只能在下面直接修改。"), stepTag("plan"));
    main.append(r);
    if (!sess().groups.length) return;
    const edit = { groups: sess().groups.map((g) => ({ ...g })), items: [] };
    const itemsByGroup = {};
    sess().items.forEach((i) => (itemsByGroup[i.group] ??= []).push({ ...i }));
    SECTIONS.forEach((sec) => {
      const gs = edit.groups.filter((g) => g.section === sec);
      const box = el("div", "df-sec");
      box.append(el("h4", "", `${sec}　${SEC_ZH[sec]}`));
      gs.forEach((g) => {
        const card = el("div", "df-card");
        const title = input(g.title, "題組標題");
        title.oninput = () => (g.title = title.value);
        const purpose = input(g.purpose, "這一組要釐清什麼", "textarea");
        purpose.oninput = () => (g.purpose = purpose.value);
        const head = row(el("span", "df-gid", g.id), title);
        head.append(iconButton("trash", "刪除題組", () => { edit.groups = edit.groups.filter((x) => x !== g); save(); }));
        card.append(head, purpose);
        const ul = el("ul", "df-items");
        (itemsByGroup[g.id] ?? []).forEach((it) => {
          const li = el("li", "", `${it.label}`);
          li.append(el("span", "df-role", it.role || "—"));
          ul.append(li);
        });
        const add = input("", "新增要釐清的項目，Enter 加入");
        add.onkeydown = (e) => { if (e.key === "Enter" && add.value.trim()) { edit.items.push({ id: "", group: g.id, label: add.value.trim(), role: "design" }); save(); } };
        card.append(ul, add);
        box.append(card);
      });
      const addG = btn(`＋ 新增 ${SEC_ZH[sec]} 題組`, () => { edit.groups.push({ id: "", section: sec, title: "新題組", purpose: "" }); save(); }, "ghost small");
      box.append(addG);
      main.append(box);
    });
    function save() { run("儲存題組", () => call("/plan", edit, "PUT")); }
    main.append(row(btn("儲存修改", save), nextBtn("開始對話")));
    figurePlan(main);
  }

  // ── 圖表規劃: proposed with the groups; keep, drop, pick how to draw, or say your own ──
  const TOOL_ZH = { flow: "流程圖", plot: "繪圖", table: "表格", image: "自備影像" };
  function figurePlan(main) {
    const fp = sess().figures;
    const fh = hq("圖表規劃", "依實驗記錄、Key value 和期刊習慣提出的圖表。每張選一種畫法，或刪掉、自己說；大綱和草稿會用這份清單的 \\ref。");
    fh.id = "dfFigPlan";
    main.append(fh);
    const head = row();
    if (fp?.running) head.append(note("圖表規劃中…"));
    else head.append(btn(fp?.items?.length ? "重新規劃圖表" : "規劃圖表", () => run("規劃圖表", () => call("/figures")), "ghost"));
    if (fp?.engine) head.append(engineTag(fp.engine, fp.seconds != null ? `${fp.seconds} 秒` : ""));
    main.append(head);
    if (fp?.failed) main.append(note(`沒有完成：${fp.failed}`, "warn"));
    if (!fp?.items?.length) return;
    const items = fp.items.map((x) => ({ ...x }));
    const save = () => run("儲存圖表", () => call("/figures", { items }, "PUT"));
    const kept = (k) => items.filter((x) => x.kind === k && x.status !== "removed").length;
    const count = (k, zh, t) => `${zh} ${kept(k)}${t ? `／期刊 ${t[0]}–${t[1]}` : ""}`;
    const over = (k, t) => t && (kept(k) < t[0] || kept(k) > t[1]);
    main.append(row(el("span", `df-badge ${over("figure", fp.figures) ? "warn" : ""}`, count("figure", "圖", fp.figures)), el("span", `df-badge ${over("table", fp.tables) ? "warn" : ""}`, count("table", "表", fp.tables))));
    const list = el("div", "df-figs");
    items.forEach((it) => {
      const off = it.status === "removed";
      const card = el("div", `df-card df-fig ${off ? "no" : ""}`);
      const top = row(el("span", "df-gid", it.id), el("b", "", it.title), el("span", "df-role", TOOL_ZH[it.tool] ?? it.tool));
      top.append(el("span", "grow"));
      // its stand-in is in the paper: make the real figure here, it takes the stand-in's place and label
      const si = it.kind === "figure" && !off && (D.data.stand_ins ?? []).find((x) => x.label === it.label);
      if (si) {
        top.append(btn("用流程圖做", () => makeFromStandIn(it.label, "flow"), "ghost small"), btn("用繪圖做", () => makeFromStandIn(it.label, "plot"), "ghost small"));
      }
      top.append(btn(off ? "加回來" : "刪除", () => { it.status = off ? "kept" : "removed"; save(); }, "ghost small"));
      card.append(top);
      if (it.supports) card.append(el("p", "df-text", it.supports));
      // what it is made from (record ids); the claims it backs only as a count
      const ev = el("div", "df-tags");
      (it.evidence ?? []).forEach((e) => ev.append(el("span", "df-badge", e)));
      (it.pending ?? []).forEach((p) => ev.append(el("span", "df-badge warn", `待補 ${p}`)));
      if (it.claims?.length) ev.append(el("span", "df-fig-claims", `支撐 ${it.claims.length} 個主張`));
      if (ev.childElementCount) card.append(ev);
      (it.warnings ?? []).forEach((w) => card.append(note(w, "warn")));
      if (!off) {
        // options first; 「都不是，我自己說」 last
        const opts = el("div", "df-opts");
        (it.options ?? []).forEach((o, k) => {
          const b = el("button", "ghost", o);
          // nothing looks chosen until he chooses (a proposal is not a decision)
          b.setAttribute("aria-pressed", String(it.status === "kept" && !it.own && (it.chosen ?? 0) === k));
          b.disabled = !!D.busy;
          b.onclick = () => { it.chosen = k; it.own = ""; it.status = "kept"; save(); };
          opts.append(b);
        });
        const own = input(it.own, "都不是，我自己說");
        own.onchange = () => { it.own = own.value; it.status = "kept"; save(); };
        card.append(opts, own);
      }
      list.append(card);
    });
    main.append(list);
    const add = input("", "新增一張圖或表（說它要畫什麼），Enter 加入");
    add.onkeydown = (e) => { if (e.key === "Enter" && add.value.trim()) { items.push({ id: "", kind: /表|table/i.test(add.value) ? "table" : "figure", label: "", title: add.value.trim(), tool: "" }); save(); } };
    main.append(add);
  }

  // ── 4 對話 ───────────────────────────────────────────
  const stateOf = (gid) => D.data.states.find((s) => s.group === gid);
  const turnsOf = (gid) => D.data.turns.filter((t) => t.group === gid);

  function interview(main, side) {
    const groups = sess().groups;
    if (!groups.length) { main.append(note("先在「題組」規劃題組。")); return; }
    if (!D.group || !groups.some((g) => g.id === D.group)) D.group = sess().active ?? groups.find((g) => g.status === "pending")?.id ?? groups[0].id;
    const chips = el("div", "df-chips");
    groups.forEach((g) => {
      const c = el("button", `df-chip s-${g.status}`, g.title);
      c.prepend(el("span", "c", g.id.startsWith("v") ? "修訂" : (SEC_ZH[g.section] ?? g.section)));
      c.setAttribute("aria-pressed", String(g.id === D.group));
      if (stateOf(g.id)?.ready && g.status !== "done") c.classList.add("ready");
      c.onclick = () => { D.group = g.id; D.picked = null; D.pickItem = null; draw(); };
      chips.append(c);
    });
    main.append(chips);
    const g = groups.find((x) => x.id === D.group);
    const st = stateOf(g.id);
    main.append(el("p", "df-purpose", g.purpose));
    if (g.prep === "running") main.append(note("準備選項中…"));
    else if (g.prep === "failed") main.append(row(note("選項沒有準備好", "warn"), btn("重試", () => run("準備選項", () => call(`/group/${g.id}`, { action: "prepare" })), "ghost")));
    const log = el("div", "df-log");
    const turns = turnsOf(g.id);
    turns.forEach((t, i) => {
      const b = el("div", `df-msg ${t.who}`);
      b.append(el("div", "", t.text));
      if (t.who === "ai" && t.engine) b.append(engineTag(t.engine));
      // the question being asked: talk it over apart from the interview (深度討論)
      if (t.who === "ai" && i === turns.length - 1) {
        const n = discussionsOf(g.id).length;
        const db = btn("深度討論", () => openDiscussion(g.id), "ghost small df-discbtn");
        if (n) db.append(el("span", "df-disc-n", String(n)));
        b.append(db);
      }
      const it = t.who === "ai" && i === turns.length - 1 && t.about ? sess().items.find((x) => x.id === t.about) : null;
      if (it?.candidates?.length) b.append(candidateCards(it));
      else if (t.who === "ai" && t.options?.length && i === turns.length - 1) {
        const opts = el("div", "df-opts");
        t.options.forEach((o) => { const ob = el("button", "ghost", o); ob.disabled = !!D.busy; ob.onclick = () => send(o); opts.append(ob); });
        b.append(opts);
      }
      log.append(b);
    });
    main.append(log);
    setTimeout(() => (log.scrollTop = log.scrollHeight), 0);

    if (!turns.length) {
      main.append(row(btn("開始這一組", () => run("準備第一個問題", () => call("/turn", { group: g.id, model: $("#dfModel").value })))));
    } else {
      // while 深度討論 is open the answer comes from there (one place to write at a time)
      const talking = !!(D.disc && sess().discussions?.some((x) => x.id === D.disc && x.group === g.id && x.status === "open"));
      const ta = el("textarea", "np-input df-answer");
      ta.rows = 3;
      ta.disabled = talking;
      ta.placeholder = talking ? "正在深度討論：在右邊整理成回答，或關閉討論" : "回答（Ctrl+Enter 送出）";
      ta.value = D.draftAnswer?.[g.id] ?? "";
      ta.oninput = () => ((D.draftAnswer ??= {})[g.id] = ta.value);
      ta.onkeydown = (e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) send(ta.value); };
      main.append(ta);
      const mic = micButton(ta, g.id);
      if (mic) ta.after(mic);
      const last = turns.at(-1);
      const shown = last?.who === "ai" && last.about && sess().items.find((x) => x.id === last.about)?.candidates?.length;
      const sendRow = row(btn("送出", () => send(ta.value)), ...(shown ? [] : [btn("不確定，給我選項", () => send("不確定，請給我幾個選項"), "ghost")]));
      if (talking) sendRow.querySelectorAll("button").forEach((x) => (x.disabled = true));
      main.append(sendRow);
    }
    const foot = row();
    if (st?.ready && g.status !== "done") foot.append(note("這一組的資訊已經足夠。", "ok"));
    else if (st?.over_budget && g.status !== "done") foot.append(note(`已經問了 ${g.turns} 題，剩下沒問到的可以略過。`, "warn"));
    foot.append(btn(g.status === "done" ? "重新打開" : "結束這一組", () => run("更新題組", () => call(`/group/${g.id}`, { action: g.status === "done" ? "reopen" : "close" })), st?.ready ? "primary small" : "ghost"));
    if (g.status !== "done" && g.status !== "skipped") foot.append(btn("略過", () => run("更新題組", () => call(`/group/${g.id}`, { action: "skip" })), "ghost"));
    const nextG = groups.find((x) => x.status === "pending" && x.id !== g.id);
    if (nextG) foot.append(btn(`下一組：${nextG.title}`, () => { D.group = nextG.id; draw(); }, "ghost"));
    else if (groups.every((x) => x.status === "done" || x.status === "skipped")) foot.append(nextBtn("下一步：主張登記"));
    main.append(foot);
    const d = D.disc && sess().discussions?.find((x) => x.id === D.disc && x.group === g.id && x.status === "open");
    if (d) discussion(side, d);
    else { ledger(side, g, st); writingNotes(side); }
  }

  async function makeFromStandIn(label, tool) {
    const r = tool === "flow" ? await PCFlow.create() : await PCPlot.create();
    if (!r) return;
    try {
      await PCStandIn.replace(label, r);
      LoreShell.toast(`已取代預留的 ${label}`);
      if (state.paper?.name) openPaper(state.paper.name);
      D.data = await api(base());
      draw();
    } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
  }

  // ── 寫法筆記: how he said to write, in his words; the drafts follow them ──
  const SCOPE_ZH = [["paragraph", "這段"], ["section", "這節"], ["paper", "整篇"]];
  function writingNotes(side, end = false) {
    const ns = sess().writing_notes ?? [];
    if (!ns.length) return;
    side.append(el("h4", "", "寫法筆記"));
    const edit = ns.map((n) => ({ ...n }));
    const save = () => run("儲存寫法筆記", () => call("/notes", { notes: edit }, "PUT"));
    const ul = el("ul", "df-notes");
    edit.forEach((n, k) => {
      const li = el("li");
      const t = input(n.text, "", "textarea");
      t.rows = 2;
      t.onchange = () => { n.text = t.value; save(); };
      const sc = el("select", "np-input df-scope");
      SCOPE_ZH.forEach(([v, z]) => { const o = el("option", "", z); o.value = v; sc.append(o); });
      sc.value = n.scope;
      sc.style.cssText = "width:auto;flex:none";
      sc.onchange = () => { n.scope = sc.value; save(); };
      const keep = el("label", "df-keep");
      const cb = el("input");
      cb.type = "checkbox";
      cb.checked = !!n.keep;
      cb.disabled = !!n.kept;
      cb.onchange = () => { n.keep = cb.checked; save(); };
      keep.append(cb, document.createTextNode(n.kept ? "已存進我的寫法" : "以後也用"));
      li.append(t, row(sc, keep, iconButton("trash", "刪除", () => { edit.splice(k, 1); save(); })));
      if (n.conflict) li.append(note("和標點規則衝突：以標點規則為準", "warn"));
      ul.append(li);
    });
    side.append(ul);
    if (end || edit.some((n) => n.keep && !n.kept)) {
      const b = btn("存進我的寫法", () => run("存進我的寫法", () => call("/notes/keep")), "ghost small");
      b.disabled = !!D.busy || !edit.some((n) => n.keep && !n.kept);
      side.append(b);
    }
  }

  // ── 深度討論: one question talked over apart from the interview; never material for the paper ──
  const discussionsOf = (gid) => (sess().discussions ?? []).filter((x) => x.group === gid);
  function openDiscussion(gid) {
    run("打開深度討論", async () => {
      const v = await call("/discuss", { group: gid });
      const d = v.session.discussions?.filter((x) => x.group === gid && x.status === "open").at(-1);
      D.disc = d?.id ?? null; D.discAnswer = null;
      return v;
    });
  }
  function discussion(side, d) {
    const head = el("div", "df-dhead");
    head.append(el("h4", "", "深度討論"), iconButton("x", "關閉（不送回答）", () => run("關閉討論", async () => { const v = await call(`/discuss/${d.id}/close`, {}); D.disc = null; D.discAnswer = null; return v; })));
    const q = el("p", "df-dq", d.question);
    q.title = "按一下看整題";
    q.onclick = () => q.classList.toggle("open");
    side.append(head, q);
    const log = el("div", "df-log df-dlog");
    for (const m of d.messages) log.append(el("div", `df-msg ${m.who}`, m.text));
    if (!d.messages.length) log.append(note("問什麼都可以；這裡的內容不會寫進論文。"));
    if (D.busy) log.append(el("div", "df-busy", `${D.busy}…`));
    side.append(log);
    setTimeout(() => (log.scrollTop = log.scrollHeight), 0);
    if (D.discAnswer != null) {
      // the answer drawn from the discussion: the author edits it, then it goes to the interview
      side.append(el("h4", "", "這題的回答"));
      const ans = input(D.discAnswer, "", "textarea");
      ans.rows = 5;
      ans.oninput = () => (D.discAnswer = ans.value);
      side.append(ans, row(
        btn("送出為回答", () => run("送出回答", async () => { const v = await call(`/discuss/${d.id}/close`, { answer: ans.value, model: $("#dfModel").value }); D.disc = null; D.discAnswer = null; return v; })),
        btn("繼續討論", () => { D.discAnswer = null; draw(); }, "ghost")));
      return;
    }
    const ta = el("textarea", "np-input");
    ta.rows = 3;
    ta.placeholder = "想問的（Ctrl+Enter 送出）";
    ta.value = D.discDraft ?? "";
    ta.oninput = () => (D.discDraft = ta.value);
    const say = () => { const t = ta.value.trim(); if (!t) return; run("討論中", async () => { const v = await call(`/discuss/${d.id}`, { text: t }); D.discDraft = ""; return v; }); };
    ta.onkeydown = (e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) say(); };
    const sum = btn("整理成這題的回答", () => run("整理回答", async () => { const r = await call(`/discuss/${d.id}/answer`, {}); D.discAnswer = r.answer ?? ""; return null; }), "primary small");
    // something to sum up only once the author has said something
    if (!d.messages.some((m) => m.who === "me")) sum.disabled = true;
    side.append(ta, row(btn("送出", say, "ghost small"), sum));
    if (d.item) side.append(row(btn("這題不用寫進論文", () => run("略過這題", async () => { const v = await call(`/discuss/${d.id}/close`, { exclude: true }); D.disc = null; return v; }), "ghost small")));
  }

  function send(text) {
    text = (text ?? "").trim();
    if (!text) return;
    const gid = D.group;
    const picked = [...(D.picked ?? [])];
    const item = D.pickItem ?? null;
    run("思考下一個問題", async () => {
      const v = await call("/turn", { group: gid, answer: text, picked, item, model: $("#dfModel").value });
      // a question back opened 深度討論 (the model saw it was not an answer)
      const d = v.session.discussions?.find((x) => x.group === gid && x.status === "open");
      if (d && d.messages.at(-1)?.who === "ai") D.disc = d.id;
      if (D.draftAnswer) D.draftAnswer[gid] = "";
      D.picked = null; D.pickItem = null;
      return v;
    });
  }

  // ── 按住說話 (lore-voice, when the machine has it) ─────────
  let voiceOk = null, vEvents = null, vCur = null;
  function micButton(ta, gid) {
    const b = iconButton("mic", "按住說話", () => {});
    b.classList.add("df-mic");
    b.hidden = true;
    (async () => {
      if (voiceOk == null) { try { voiceOk = !!(await api("/api/voice")).available; } catch { voiceOk = false; } }
      b.hidden = !voiceOk;
    })();
    const text = (e) => { try { return JSON.parse(e.data).text ?? ""; } catch { return ""; } };
    const put = (t) => { if (!vCur) return; vCur.ta.value = (vCur.base + t).trimStart(); (D.draftAnswer ??= {})[vCur.gid] = vCur.ta.value; };
    const start = async (e) => {
      e.preventDefault();
      if (!vEvents) {
        vEvents = new EventSource("/api/voice/events");
        vEvents.addEventListener("partial", (x) => put(text(x)));
        vEvents.addEventListener("final", (x) => { put(text(x)); vCur?.ta.focus(); });
        vEvents.addEventListener("error", (x) => { let v = {}; try { v = JSON.parse(x.data ?? "{}"); } catch {} if (v.kind) { voiceOk = false; LoreShell.toast(v.message ?? "語音無法使用", { err: true }); } });
      }
      vCur = { ta, gid, base: ta.value ? `${ta.value.trimEnd()} ` : "" };
      b.classList.add("on");
      try { await api("/api/voice/listen", { continuous: false, language: "zh-TW" }); } catch (err) { b.classList.remove("on"); LoreShell.toast(err.message, { err: true }); }
    };
    const stop = () => { if (!b.classList.contains("on")) return; b.classList.remove("on"); api("/api/voice/listen/stop", {}).catch(() => {}); };
    b.onpointerdown = start;
    b.onpointerup = stop;
    b.onpointerleave = stop;
    return b;
  }

  // ── 預想選項 ──────────────────────────────────────────
  // Possible answers prepared from the record before the interview: a click
  // puts one in the answer box (click two to combine, edit freely); 1/2/3 on
  // the keyboard; 「都不是」 = your own words.
  const evidenceText = (id) => {
    const x = sess().experiments.find((e) => e.id === id);
    if (x) return `${x.title}：${x.change} ${x.metrics} ${x.result}`.trim();
    const th = sess().thoughts.find((e) => e.id === id);
    if (th) return th.text;
    const st = sess().timeline.find((e) => `T${e.step}` === id);
    if (st) return `${st.what} → ${st.observed}`;
    const tb = sess().tables.find((e) => e.id === id);
    if (tb) return tb.caption;
    const pd = (sess().pending ?? []).find((e) => e.id === id);
    return pd ? pd.what : id;
  };
  function candidateCards(it, withNone = true) {
    const box = el("div", "df-cands");
    D.picked ??= [];
    it.candidates.forEach((c, k) => {
      const b = el("button", `df-cand${D.picked.includes(c.id) ? " on" : ""}`);
      b.disabled = !!D.busy;
      b.append(el("span", "n", String(k + 1)), el("span", "t", c.text));
      if (D.pickItem && D.pickItem !== it.id) b.classList.remove("on");
      if (c.evidence?.length) {
        const ev = el("span", "ev");
        c.evidence.forEach((e) => { const x = el("span", "df-ev", e); x.title = evidenceText(e); ev.append(x); });
        b.append(ev);
      }
      b.onclick = () => pickCandidate(it, c);
      box.append(b);
    });
    if (!withNone) return box;
    const none = el("button", "ghost df-none", "都不是，我自己說");
    none.disabled = !!D.busy;
    none.onclick = () => { D.picked = []; D.pickItem = null; const ta = $(".df-answer"); if (ta) { ta.value = ""; ta.focus(); } box.querySelectorAll(".df-cand").forEach((x) => x.classList.remove("on")); };
    box.append(none);
    return box;
  }
  function pickCandidate(it, c) {
    const ta = $(".df-answer");
    if (!ta) return;
    // candidates of one item per answer
    if (D.pickItem && D.pickItem !== it.id) D.picked = [];
    D.pickItem = it.id;
    D.picked ??= [];
    if (D.picked.includes(c.id)) return;
    D.picked.push(c.id);
    ta.value = ta.value.trim() ? `${ta.value.trim()}；${c.text}` : c.text;
    ((D.draftAnswer ??= {})[D.group] = ta.value);
    ta.focus();
    draw();
    const again = $(".df-answer"); if (again) { again.focus(); again.setSelectionRange(again.value.length, again.value.length); }
  }
  // 1 / 2 / 3 pick a candidate when the answer box is not being typed in
  document.addEventListener("keydown", (e) => {
    if (!D || !$("#dlgDraft")?.open || STEPS[D.step]?.[0] !== "interview") return;
    if (!/^[1-3]$/.test(e.key) || e.metaKey || e.ctrlKey || e.altKey) return;
    if (document.activeElement?.matches("textarea, input, select")) return;
    const last = turnsOf(D.group).at(-1);
    const it = last?.about && sess().items.find((x) => x.id === last.about);
    const c = it?.candidates?.[+e.key - 1];
    if (c) { e.preventDefault(); pickCandidate(it, c); }
  });

  function ledger(side, g, st) {
    // 待補項目 has no summary of its own: each result's four parts are on its item
    if (g.id !== "wp") {
      side.append(el("h4", "", "這一組要整理出"));
      const sum = el("dl", "df-sum");
      [["說明", g.summary.explain], ["動機", g.summary.motive], ["結論", g.summary.conclusion], ["讀者承認", g.summary.reader_accepts], ["轉折", g.summary.twist]].forEach(([k, v]) => {
        sum.append(el("dt", "", k), el("dd", v ? "" : "empty", v || "還沒有"));
      });
      side.append(sum);
    }
    side.append(el("h4", "", "狀態表"));
    const ul = el("ul", "df-ledger");
    sess().items.filter((i) => i.group === g.id).forEach((it) => {
      const li = el("li");
      if (it.excluded) {
        li.classList.add("excluded");
        li.append(el("b", "", it.label), row(el("span", "df-badge soft", "不寫進論文"), btn("恢復", () => run("恢復這題", () => call(`/items/${it.id}/exclude`, { excluded: false })), "ghost small")));
        ul.append(li);
        return;
      }
      const s = st?.items.find((x) => x.id === it.id);
      li.classList.toggle("ok", !!s && !s.missing.length);
      li.append(el("b", "", it.label));
      const tags = el("div", "df-tags");
      if (it.mode) tags.append(el("span", "df-badge", modeName(it.mode)));
      if (it.depth != null) tags.append(el("span", "df-badge lv", levelName(it.depth)));
      it.flags.forEach((f) => tags.append(el("span", "df-badge flag", flagName(f))));
      if (it.number) tags.append(el("span", "df-badge", it.number));
      (s?.missing ?? []).forEach((m) => tags.append(el("span", "df-badge warn", missingText(m))));
      if (s?.preferred != null) tags.append(el("span", "df-badge soft", `可再到 ${levelName(s.preferred)}`));
      li.append(tags);
      if (it.stance) li.append(el("div", "df-stance", it.stance));
      const lastAbout = turnsOf(g.id).at(-1)?.about;
      if (it.candidates?.length && lastAbout !== it.id && turnsOf(g.id).length) {
        const d = el("details", "df-lcands");
        d.append(el("summary", "", `選項 ${it.candidates.length}`), candidateCards(it, false));
        li.append(d);
      }
      const pi = it.pending && (sess().pending ?? []).find((x) => x.id === it.pending);
      if (pi) {
        const dl = el("dl", "df-sum");
        [["是什麼", pi.note.what], ["為什麼必要", pi.note.why], ["預期", pi.note.expect], ["相反時", pi.note.if_not]].forEach(([k, v]) => dl.append(el("dt", "", k), el("dd", v ? "" : "empty", v || "—")));
        li.append(dl);
      }
      if (it.refs?.length) { const rr = el("div", "df-refs"); it.refs.forEach((k) => rr.append(refChip(k))); li.append(rr); }
      ul.append(li);
    });
    side.append(ul);
  }

  // ── 4.5 主張 ─────────────────────────────────────────
  function claims(main) {
    main.append(hq("主張登記", "每個主張只在一個章節完整說一次，其他地方只簡短回扣；Abstract 與 Conclusion 最後從這裡取最強的主張。"));
    main.append(row(btn(sess().claims.length ? "重新產生" : "產生主張登記", () => run("整理主張", () => call("/claims"))), stepTag("claims"), nextBtn()));
    if (!sess().claims.length) return;
    const edit = sess().claims.map((c) => ({ ...c }));
    edit.forEach((c, i) => {
      const card = el("div", "df-card");
      const zh = input(c.zh, "主張（中文）", "textarea");
      zh.oninput = () => (c.zh = zh.value);
      const en = input(c.en, "One English sentence");
      en.oninput = () => (c.en = en.value);
      const home = el("select", "np-input df-home");
      SECTIONS.forEach((s) => { const o = el("option", "", `完整說在 ${s}`); o.value = s; home.append(o); });
      home.value = c.home || "Results";
      home.onchange = () => (c.home = home.value);
      const head = row(el("span", "df-gid", c.id), home);
      if (c.depth != null) head.append(el("span", "df-badge lv", levelName(c.depth)));
      head.append(iconButton("trash", "刪除", () => { edit.splice(i, 1); save(); }));
      card.append(head, zh, en);
      const meta = [c.evidence && `證據：${c.evidence}`, c.reader_accepts && `讀者承認：${c.reader_accepts}`, c.twist && `轉折：${c.twist}`].filter(Boolean).join("　");
      if (meta) card.append(note(meta));
      if (c.refs?.length) { const rr = el("div", "df-refs"); c.refs.forEach((k) => rr.append(refChip(k))); card.append(rr); }
      main.append(card);
    });
    function save() { run("儲存主張", () => call("/claims", { claims: edit }, "PUT")); }
    main.append(row(btn("儲存修改", save)));
  }

  // ── 5 校正 ───────────────────────────────────────────
  function review(main) {
    main.append(h("章節順序與敘事"));
    const toOutline = D.data.journal ? btn("下一步", () => run("儲存", async () => { const v = await call("/template", { later: true }, "PUT"); D.step = STEPS.findIndex(([k]) => k === "outline"); return v; }), "ghost") : nextBtn();
    main.append(row(btn(sess().review ? "重新校正" : "校正", () => run("校正", () => call("/review"))), stepTag("review"), toOutline));
    const r = sess().review;
    if (!r) return;
    const title = (id) => sess().groups.find((g) => g.id === id)?.title ?? id;
    const chain = el("ol", "df-chain");
    r.chain.forEach((l) => {
      const li = el("li", l.follows ? "" : "broken");
      li.append(el("b", "", `${title(l.group)}`), el("div", "", `讀完之後：${l.conclusion}`));
      if (l.note) li.append(el("div", "df-note", l.note));
      chain.append(li);
    });
    main.append(chain);
    if (!r.suggestions.length) { main.append(note("沒有需要調整的地方。", "ok")); return; }
    main.append(h("建議"));
    r.suggestions.forEach((x) => {
      const card = el("div", `df-card ${x.accepted === true ? "yes" : x.accepted === false ? "no" : ""}`);
      card.append(row(el("span", "df-badge", x.kind), el("b", "", x.detail)));
      if (x.groups.length) card.append(note(x.groups.map(title).join("、")));
      if (x.reason) card.append(note(`理由：${x.reason}`));
      const decide = (v) => run("儲存", () => call("/review", { decisions: { [x.id]: v } }, "PUT"));
      card.append(row(btn("採用", () => decide(true), x.accepted === true ? "primary small" : "ghost"), btn("不採用", () => decide(false), x.accepted === false ? "primary small" : "ghost")));
      main.append(card);
    });
  }

  // ── ★ 版型 ───────────────────────────────────────────
  async function template(main) {
    const id = (D.drawId = (D.drawId ?? 0) + 1);
    if (!D.tpls) {
      try { D.tpls = (await api("/api/templates")).filter((j) => j.kind !== "book"); } catch { D.tpls = []; }
    }
    let choice = null;
    try { choice = await api(base() + "/templates"); } catch { choice = { current: null, suggested: [] }; }
    if (id !== D.drawId || STEPS[D.step][0] !== "template") return;
    const current = choice.current ?? "";
    let picked = current;
    const apply = btn("套用", () => run("套用版型", async () => {
      const v = await call("/template", { template: picked || null }, "PUT");
      if (state.paper?.name === D.name) openPaper(D.name);
      D.step = STEPS.findIndex(([k]) => k === "outline");
      return v;
    }));
    apply.disabled = !!D.busy || picked === current;
    const later = btn("稍後", () => run("儲存", async () => { const v = await call("/template", { later: true }, "PUT"); D.step = STEPS.findIndex(([k]) => k === "outline"); return v; }), "ghost");
    main.append(h("版型"), PCNew.templateList({ tpls: D.tpls, selected: current, suggested: choice.suggested, paper: D.name, onPick: (v) => { picked = v; apply.disabled = !!D.busy || picked === current; } }), row(later, apply));
  }

  // ── 6 大綱 ───────────────────────────────────────────
  function outline(main) {
    main.append(hq("段落大綱", "每一段標出類型與組成（事實、理由層級、結論）。逐章看過，有意見寫在章節下方，按「修改這一章」。沒有 Claude 時由本機模型逐章產生。"));
    main.append(row(btn(sess().outline.length ? "全部重新產生" : "產生大綱", () => run("產生大綱", () => call("/outline"))), stepTag("outline"), nextBtn()));
    if (!sess().outline.length) return;
    const edit = sess().outline.map((o) => ({ ...o, paras: o.paras.map((p) => ({ ...p })) }));
    SECTIONS.forEach((sec) => {
      const parts = edit.filter((o) => o.section === sec);
      if (!parts.length) return;
      const box = el("div", "df-sec");
      const conf = el("input"); conf.type = "checkbox"; conf.checked = parts.every((p) => p.confirmed);
      conf.onchange = () => { parts.forEach((p) => (p.confirmed = conf.checked)); save(); };
      const lab = el("label", "df-confirm"); lab.append(conf, document.createTextNode("這一章確認"));
      box.append(row(el("h4", "", `${sec}　${SEC_ZH[sec]}`), lab));
      parts.forEach((o) => {
        if (o.subsection) box.append(el("h5", "", o.subsection));
        o.paras.forEach((p) => {
          const card = el("div", "df-card");
          card.append(row(el("span", "df-gid", p.id), el("span", "df-badge", p.kind), el("span", "df-plan", p.plan)));
          const zh = input(p.zh, "", "textarea");
          zh.oninput = () => (p.zh = zh.value);
          const en = input(p.en.join("\n"), "English points, one per line", "textarea");
          en.classList.add("mono");
          en.rows = Math.max(2, p.en.length);
          en.oninput = () => (p.en = en.value.split("\n").map((x) => x.trim()).filter(Boolean));
          card.append(zh, en);
          if (p.sources.length) card.append(note(`來源：${p.sources.join("、")}`));
          if (p.pending?.length) card.append(pendChips(p.pending));
          if (p.cites?.length) { const rr = el("div", "df-refs"); p.cites.forEach((k) => rr.append(refChip(k))); card.append(rr); }
          box.append(card);
        });
      });
      const cm = input(D.comments?.[sec] ?? "", `對 ${sec} 的意見，例如：第二段跟第三段合併、發想放到第一段`, "textarea");
      cm.oninput = () => ((D.comments ??= {})[sec] = cm.value);
      box.append(cm, row(btn("修改這一章", () => run(`修改 ${sec}`, () => call("/outline", { section: sec, comment: cm.value })), "ghost")));
      main.append(box);
    });
    function save() { run("儲存大綱", () => call("/outline", { outline: edit }, "PUT")); }
    main.append(row(btn("儲存修改", save)));
  }

  // ── 7 草稿 ───────────────────────────────────────────
  function drafting(main) {
    main.append(hq("草稿", "逐章生成，每段用你的句型庫寫，並照大綱的組成、你的風格筆記與對照表（登記的巨集、\\ref 標籤、縮寫）。每段寫完會跑一次對照表檢查：手打的「Table 1」或不存在的標籤會自動重寫一次，其他問題列在段落下方。建議最後才生成 Introduction 與 Conclusion，Abstract 最後。引擎用右側「寫作」選的那個。放進論文：寫進論文的章節檔（Methods 對到 Materials and Methods），摘要寫進標題區；原來的文字先存在 draft/archive/，已插入的表格與圖留著。"));
    if (!sess().outline.length) { main.append(note("先完成大綱。")); return; }
    const order = ["Methods", "Results", "Discussion", "Introduction", "Conclusion", "Abstract"];
    const drafts = sess().drafts;
    const waiting = drafts.filter((d) => !d.placed);
    const all = row(btn(waiting.length ? `全部放進論文（${waiting.length}）` : "已全部放進論文", () => putIn(null)));
    all.firstChild.disabled = !!D.busy || !waiting.length;
    if (drafts.length) main.append(all);
    order.forEach((sec) => {
      if (sec === "Abstract" ? !drafts.some((x) => x.section !== "Abstract") : !sess().outline.some((o) => o.section === sec)) return;
      const box = el("div", "df-sec");
      const d = drafts.find((x) => x.section === sec);
      const r = row(el("h4", "", `${sec}　${SEC_ZH[sec]}`), btn(d ? "重新生成" : "生成", () => run(`生成 ${sec}`, () => call("/compose", { section: sec, engine: chosenEngine() })), d ? "ghost" : "primary small"));
      if (d?.engine) r.append(engineTag(d.engine));
      if (d) r.append(d.placed ? el("span", "df-badge flag", "已放進論文") : btn("放進論文", () => putIn(sec), "ghost"));
      if (d && sec === "Abstract") r.append(el("span", "df-note", `${d.paras.map((p) => p.text).join(" ").split(/\s+/).filter(Boolean).length} 字`));
      box.append(r);
      if (d?.new_symbols?.length) box.append(newSymbols(d.new_symbols));
      // the same notice on many paragraphs (an abbreviation never spelled out) shows once, above them
      const key = (x) => x.replace(/\d+ 次/g, "N 次");
      const seen = {};
      (d?.paras ?? []).forEach((p) => new Set(p.problems.map(key)).forEach((k) => (seen[k] = (seen[k] ?? 0) + 1)));
      const common = Object.keys(seen).filter((k) => seen[k] > 1);
      if (common.length) box.append(note(common.map((k) => k.replace(/在正文用了 N 次，/, "")).join("；"), "warn"));
      d?.paras.forEach((p) => {
        const card = el("div", "df-card");
        card.append(row(el("span", "df-gid", p.para), ...(p.pending?.length ? [pendChips(p.pending)] : [])), el("p", "df-draft", p.text));
        const own = p.problems.filter((x) => !common.includes(key(x)));
        if (own.length) card.append(note(own.join("；"), "warn"));
        box.append(card);
      });
      main.append(box);
    });
    main.append(row(nextBtn("下一步：審稿")));
  }

  /** 放進論文: one drafted section (or all) into the paper; the editor reloads. */
  function putIn(sec) {
    run("放進論文", async () => {
      const v = await call("/insert", sec ? { section: sec } : {});
      if (state.paper?.name === D.name) openPaper(D.name);
      LoreShell.toast(sec ? `${sec} 已放進論文` : "已放進論文");
      return v;
    });
  }

  // ── 8 審稿 ───────────────────────────────────────────
  const KIND_ZH = { venue: "期刊要求", citation: "引用", contradiction: "矛盾", overclaim: "宣稱過度", "missing-number": "缺數字", impossible: "數字不可能", generic: "空泛", repeat: "重複", "buried-origin": "發想埋太晚", "missing-why": "缺理由", punctuation: "標點句長" };
  function audit(main, side) {
    writingNotes(side, true);
    main.append(hq("審稿檢查", "檢查矛盾、宣稱強度、缺數字、空泛句、重複與埋掉的發想。需要你補充的，可以回到對話追問。沒有 Claude 時由本機模型逐章審。"));
    main.append(row(btn(sess().audit.length ? "重新審稿" : "審稿", () => run("審稿中", () => call("/audit"))), stepTag("audit")));
    pendingList(main);
    sess().audit.forEach((f) => {
      const card = el("div", `df-card ${f.resolved ? "no" : ""}`);
      card.append(row(el("span", "df-badge warn", KIND_ZH[f.kind] ?? f.kind), el("span", "df-gid", f.para)));
      if (f.quote) card.append(el("blockquote", "df-quote", f.quote));
      card.append(el("p", "df-text", f.why));
      if (f.ask) card.append(note(`要問你：${f.ask}`));
      const r = row(btn(f.resolved ? "標為未處理" : "已處理", () => run("儲存", () => call("/audit", { resolved: { [f.id]: !f.resolved } }, "PUT")), "ghost"));
      if (!f.resolved) r.append(btn("回到對話追問", () => run("建立追問", async () => { const v = await call(`/audit/${f.id}/ask`); D.group = `a${f.id}`; D.step = 3; return v; }), "ghost"));
      card.append(r);
      main.append(card);
    });
  }

  /** Symbols the generated text uses that the notation registry lacks. */
  function newSymbols(list) {
    const box = el("div", "df-card");
    box.append(note(`對照表裡沒有這些符號：${list.join("、")}。登記後到對照表補上意思（標為「待確認」）。`, "warn"));
    box.append(row(btn("登記到對照表", () => run("登記符號", async () => {
      const v = await call("/notation/register", { symbols: list });
      if (window.PCNotation) PCNotation.open(list[0] ?? "");
      return v;
    }), "ghost")));
    return box;
  }

  // ── 待補 ─────────────────────────────────────────────
  // Results still waiting for data: marked in the record step (an experiment,
  // table cells / rows / columns), explained in the 待補項目 group, written in
  // the expected direction with marks, listed first in 審稿, filled in by 修訂.
  const pendOf = (id) => (sess().pending ?? []).find((x) => x.id === id);
  const reportOf = (id) => (D.data.pending_report ?? []).find((x) => x.id === id);
  /** A sentence of LaTeX as plain words: marks as 「…」, commands dropped, their text kept. */
  function plainTex(t) {
    return (t ?? "")
      .replace(/\\pendingdep\s*\{[^}]*\}/g, "")
      .replace(/\\pending(?:para)?\s*\{[^}]*\}\s*\{((?:[^{}]|\{[^{}]*\})*)\}/g, "［待補：$1］")
      .replace(/\\pendingcell\s*\{[^}]*\}\s*\{[^}]*\}/g, "［待補］")
      .replace(/\\(?:eq)?ref\s*\{([^}]*)\}/g, "$1")
      .replace(/\\cite[a-z]*\s*(\[[^\]]*\])?\{[^}]*\}/g, "")
      .replace(/\\[a-zA-Z]+\*?\s*/g, "").replace(/[{}]/g, "").replace(/~/g, " ").replace(/\s+/g, " ").trim();
  }
  function pendChips(ids) {
    const r = el("span", "df-pchips");
    ids.forEach((id) => { const c = el("span", "df-pchip", id); c.title = pendOf(id)?.what ?? ""; r.append(c); });
    return r;
  }
  function savePending(items, tables) {
    return run("儲存待補", () => call("/pending", { items, ...(tables ? { tables } : {}) }, "PUT"));
  }
  function pendingBlock(main) {
    const items = (sess().pending ?? []).map((x) => ({ ...x, cells: [...(x.cells ?? [])], note: { ...x.note } }));
    main.append(hq("待補", "還在等資料、但論文一定要有的結果。對話會問你為什麼需要、預期什麼；草稿照預期方向寫並標出來；結果出來後在這裡「填結果」。"));
    items.forEach((it) => main.append(pendingCard(it, false, items)));
    if (items.some((x) => x.status === "waiting")) main.append(row(btn("儲存待補", () => savePending(items), "ghost")));
    const add = input("", "還在等的結果（例如：外部醫院的 Dice）");
    add.onkeydown = (e) => { if (e.key === "Enter" && add.value.trim()) savePending([...items, { id: "", what: add.value.trim() }]); };
    main.append(row(add, btn("＋ 待補", () => { if (add.value.trim()) savePending([...items, { id: "", what: add.value.trim() }]); }, "ghost")));
  }
  /** One pending item: its fields (record step) or only its fill-in (修訂). */
  function pendingCard(it, fillOnly, all) {
    const card = el("div", `df-card df-pend ${it.status}`);
    const rep = reportOf(it.id);
    const where = rep ? `${rep.places.length} 處・依賴 ${rep.dependents.length} 句` : "";
    card.append(row(el("span", "df-gid", it.id), ...(fillOnly || it.status !== "waiting" ? [el("b", "", it.what)] : []), el("span", `df-badge ${it.status === "waiting" ? "warn" : "flag"}`, it.status === "waiting" ? "等資料" : `已回填${it.direction ? `・${DIR_ZH[it.direction]}` : ""}`), el("span", "df-note", where)));
    if (!fillOnly && it.status === "waiting") {
      const f = (k, ph) => { const i = input(it[k], ph); i.oninput = () => (it[k] = i.value); const l = el("label", "df-field"); l.append(el("span", "", ph), i); return l; };
      const exp = el("select", "np-input df-home");
      exp.append(el("option", "", "實驗：—"));
      sess().experiments.forEach((e) => { const o = el("option", "", `${e.id} ${e.title}`); o.value = e.id; exp.append(o); });
      exp.value = it.experiment ?? ""; if (!it.experiment) exp.selectedIndex = 0;
      exp.onchange = () => (it.experiment = exp.selectedIndex ? exp.value : null);
      const ex = el("label", "df-field"); ex.append(el("span", "", "實驗"), exp);
      exp.options[0].textContent = "—";
      const grid = el("div", "df-pfields");
      grid.append(f("what", "量什麼"), f("waits_for", "等什麼資料"), f("expected", "預期方向"), f("due", "預計何時"), ex);
      card.append(grid);
      const tb = it.table && sess().tables.find((x) => x.id === it.table);
      const hole = (hh) => hh.row == null ? `${tb.columns[hh.col] ?? "?"} 欄` : hh.col == null ? `${tb.rows[hh.row]?.[0] ?? "?"} 列` : `${tb.rows[hh.row]?.[0] ?? "?"} / ${tb.columns[hh.col] ?? "?"}`;
      const foot = row();
      if (tb && it.cells.length) foot.append(el("span", "df-note", `${it.table}：${it.cells.map(hole).join("、")}`));
      foot.append(el("span", "df-grow"), iconButton("trash", "刪除", () => savePending(all.filter((x) => x !== it))));
      card.append(foot);
    }
    const left = rep && (rep.places.length || rep.dependents.length);
    if (it.result) card.append(note(`結果：${it.result}`));
    if (it.status === "waiting" || left) card.append(fillForm(it, rep));
    return card;
  }
  function fillForm(it, rep) {
    const box = el("details", "df-fill");
    box.append(el("summary", "", "填結果"));
    // what goes into the paper, as typed: each number mark and each table cell
    const named = [...new Set((rep?.places ?? []).filter((p) => p.kind === "number" || p.kind === "cell").map((p) => p.arg).filter(Boolean))];
    const vals = { ...(it.values ?? {}) };
    const grid = el("div", "df-fill-cells");
    // each mark gets its own value; without named marks, one result
    const res = input(it.result ?? "", "0.74");
    if (!named.length) grid.append(el("label", "", "結果"), res);
    named.forEach((c) => { const i = input(it.values?.[c] ?? ""); i.oninput = () => (vals[c] = i.value); grid.append(el("label", "", c), i); });
    box.append(grid);
    let direction = it.direction ?? null;
    const dirs = el("div", "df-seg");
    const go1 = btn("回填", () => go(direction));
    const paint = () => { dirs.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x.dataset.v === direction))); go1.disabled = !!D.busy || !direction; };
    Object.entries(DIR_ZH).forEach(([v, label]) => { const x = el("button", "ghost", label); x.dataset.v = v; x.onclick = () => { direction = v; paint(); }; dirs.append(x); });
    paint();
    box.append(row(dirs, go1));
    const go = (direction) => run("回填", async () => {
      // with named marks each takes its own value; a blank one keeps its mark (never another field's number)
      const result = named.length ? "" : res.value;
      const v = await call(`/pending/${encodeURIComponent(it.id)}/fill`, { result, values: vals, direction });
      const r = v.session.revisions.find((x) => x.pending === it.id);
      if (direction === "matches") D.step = STEPS.findIndex(([k]) => k === "revise");
      else if (r) { D.group = r.group; D.step = STEPS.findIndex(([k]) => k === "interview"); }
      return v;
    });
    return box;
  }
  /** 待補清單: each item, where it is and what depends on it. */
  function pendingList(main) {
    const rep = (D.data.pending_report ?? []).filter((e) => e.places.length || e.dependents.length || e.status === "waiting");
    if (!rep.length) return;
    main.append(h(`待補清單（${rep.length}）`));
    rep.forEach((e) => {
      const card = el("div", "df-card df-pend");
      card.append(row(el("span", "df-gid", e.id), el("b", "", e.what || "—"), e.unknown ? el("span", "df-badge warn", "對話沒有這一項") : el("span", "df-badge", e.status === "waiting" ? "等資料" : "已回填")));
      const ul = el("ul", "df-plist");
      const at = (p) => p.file.startsWith("draft:") ? `草稿 ${p.file.slice(6)}` : `${p.section || p.file}${p.para != null ? ` ¶${p.para + 1}` : ""}`;
      // one line per paragraph: the cells of a table together, a sentence once
      const lines = [];
      [...e.places.map((p) => [p, false]), ...e.dependents.map((p) => [p, true])].forEach(([p, dep]) => {
        const key = `${at(p)}|${p.kind === "cell" ? "cell" : plainTex(p.sentence)}`;
        const was = lines.find((x) => x.key === key);
        if (was) { if (p.kind === "cell") was.cells.push(p.arg); return; }
        lines.push({ key, where: at(p), dep: dep && !e.places.some((q) => at(q) === at(p) && q.sentence === p.sentence), cells: p.kind === "cell" ? [p.arg] : [], text: plainTex(p.sentence) });
      });
      lines.forEach((x) => { const li = el("li", x.dep ? "dep" : ""); li.append(el("span", `df-badge${x.dep ? " soft" : ""}`, x.where), el("span", "", x.cells.length ? `表格：${x.cells.join("、")}` : x.text)); ul.append(li); });
      if (ul.childElementCount) card.append(ul);
      main.append(card);
    });
  }
  /** A results table; with 待補 on, a click marks a cell, a header a column, a row's label the row. */
  function holeTable(t) {
    const items = (sess().pending ?? []).filter((x) => x.status === "waiting");
    const wrap = el("div", "df-holes");
    const pick = el("select", "np-input df-home");
    pick.append(el("option", "", "標待補…"));
    items.forEach((x) => { const o = el("option", "", `${x.id} ${x.what}`); o.value = x.id; pick.append(o); });
    pick.value = D.holeItem?.[t.id] ?? ""; if (!D.holeItem?.[t.id]) pick.selectedIndex = 0;
    pick.onchange = () => { (D.holeItem ??= {})[t.id] = pick.selectedIndex ? pick.value : null; draw(); };
    const cur = D.holeItem?.[t.id];
    const itemAt = (r, c) => items.find((x) => x.table === t.id && x.cells.some((hh) => (hh.row == null || hh.row === r) && (hh.col == null || hh.col === c) && !(hh.col == null && c === 0)));
    const toggle = (hole) => {
      const all = (sess().pending ?? []).map((x) => ({ ...x, cells: [...(x.cells ?? [])] }));
      const it = all.find((x) => x.id === cur); if (!it) return;
      if (it.table !== t.id) { it.table = t.id; it.cells = []; }
      const k = it.cells.findIndex((hh) => hh.row === hole.row && hh.col === hole.col);
      if (k >= 0) it.cells.splice(k, 1); else it.cells.push(hole);
      savePending(all);
    };
    const tb = el("table", `df-table${cur ? " picking" : ""}`);
    const hr = el("tr");
    t.columns.forEach((c, ci) => { const th = el("th", "", c); if (cur && ci > 0) { th.onclick = () => toggle({ row: null, col: ci }); th.title = "整欄待補"; } hr.append(th); });
    tb.append(hr);
    t.rows.forEach((r, ri) => {
      const tr = el("tr");
      t.columns.forEach((_, ci) => {
        const x = itemAt(ri, ci);
        const td = el("td", x ? "hole" : "", x ? x.id : (r[ci] ?? ""));
        if (cur) { td.onclick = () => toggle(ci === 0 ? { row: ri, col: null } : { row: ri, col: ci }); td.title = ci === 0 ? "整列待補" : "這格待補"; }
        tr.append(td);
      });
      tb.append(tr);
    });
    wrap.append(tb);
    if (items.length && !t.inserted) {
      const nr = input("", "新的一列（等資料）");
      const addRow = () => {
        if (!cur || !nr.value.trim()) return;
        const tables = sess().tables.map((x) => x.id === t.id ? { ...x, rows: [...x.rows, [nr.value.trim(), ...x.columns.slice(1).map(() => "")]] } : x);
        const all = (sess().pending ?? []).map((x) => ({ ...x, cells: [...(x.cells ?? [])] }));
        const it = all.find((x) => x.id === cur);
        if (it.table !== t.id) { it.table = t.id; it.cells = []; }
        it.cells.push({ row: t.rows.length, col: null });
        savePending(all, tables);
      };
      const nc = input("", "新的一欄（等資料）");
      const addCol = () => {
        if (!cur || !nc.value.trim()) return;
        const tables = sess().tables.map((x) => x.id === t.id ? { ...x, columns: [...x.columns, nc.value.trim()], rows: x.rows.map((r) => [...r, ""]) } : x);
        const all = (sess().pending ?? []).map((x) => ({ ...x, cells: [...(x.cells ?? [])] }));
        const it = all.find((x) => x.id === cur);
        if (it.table !== t.id) { it.table = t.id; it.cells = []; }
        it.cells.push({ row: null, col: t.columns.length });
        savePending(all, tables);
      };
      wrap.prepend(row(pick, ...(cur ? [nr, btn("＋ 列", addRow, "ghost"), nc, btn("＋ 欄", addCol, "ghost")] : [])));
    }
    return wrap;
  }

  // ── 9 修訂 ───────────────────────────────────────────
  // Something has to go into the finished paper: new data after a review, a
  // figure, more citations. Each request is discussed in its own group, then
  // the model proposes paragraph-level changes the author applies one by one.
  async function libraryKeys() {
    if (D.library) return D.library;
    try { D.library = (await api("/api/refs"))?.refs ?? []; } catch { D.library = []; }
    return D.library;
  }

  function revise(main) {
    const waiting = (sess().pending ?? []).filter((x) => x.status === "waiting" || (reportOf(x.id)?.places.length ?? 0) + (reportOf(x.id)?.dependents.length ?? 0) > 0);
    if (waiting.length) {
      main.append(h(`回填（${waiting.length}）`));
      waiting.forEach((it) => main.append(pendingCard(it, true)));
    }
    main.append(hq("新增修訂", "老師或審稿人要你補什麼？補了什麼實驗、數據、圖、文獻？寫下來，先討論放在哪裡、會影響哪些地方，再產生具體修改。新的實驗記錄請在第 2 步上傳並重新整理；新文獻先在文獻視窗加入文獻庫。"));
    const req = input(D.revDraft ?? "", "例：審稿人要求外部驗證。我補了另一家醫院 120 張影格，Dice 0.71；想放在 Results 主表之後，並在 Discussion 提一句。", "textarea");
    req.rows = 4;
    req.oninput = () => (D.revDraft = req.value);
    const sec = el("select", "np-input df-home");
    SECTIONS.forEach((x) => { const o = el("option", "", `大概在 ${x}`); o.value = x; sec.append(o); });
    sec.value = "Results";
    const fig = el("input"); fig.type = "file"; fig.accept = ".png,.jpg,.jpeg,.pdf,.eps";
    const refBox = el("div", "df-refpick");
    const chosen = new Set();
    libraryKeys().then((lib) => {
      const inPaper = new Set((D.data.material?.refs ?? []).map((r) => r.key));
      const sel = el("select", "np-input");
      sel.append(el("option", "", "＋ 從文獻庫加入要引用的文獻…"));
      lib.filter((r) => !inPaper.has(r.key)).forEach((r) => { const o = el("option", "", `${r.key} — ${r.title}`); o.value = r.key; sel.append(o); });
      const tags = el("div", "df-refs");
      sel.onchange = () => { if (sel.value && sel.selectedIndex > 0) { chosen.add(sel.value); const c = el("span", "df-ref"); c.append(el("b", "", sel.value)); tags.append(c); } sel.selectedIndex = 0; };
      refBox.append(sel, tags);
    });
    main.append(req, row(sec, el("span", "df-note", "新圖檔："), fig), refBox);
    main.append(row(btn("開始討論", () => run("建立修訂", async () => {
      let figure = null;
      const f = fig.files?.[0];
      if (f) {
        const name = f.name.replace(/[^A-Za-z0-9._-]/g, "-");
        const r = await fetch(`/api/papers/${encodeURIComponent(D.name)}/figures?file=${encodeURIComponent(name)}`, { method: "POST", body: f });
        if (!r.ok) throw new Error(await r.text());
        figure = name;
      }
      const v = await call("/revisions", { request: req.value, section: sec.value, refs: [...chosen], figure });
      D.revDraft = "";
      const r = v.session.revisions[v.session.revisions.length - 1];
      D.group = r.group; D.step = STEPS.findIndex(([k]) => k === "interview");
      return v;
    }))));

    const revs = sess().revisions ?? [];
    if (!revs.length) return;
    main.append(h("修訂清單"));
    revs.slice().reverse().forEach((r) => {
      const card = el("div", `df-card ${r.status === "dropped" ? "no" : ""}`);
      card.append(row(el("span", "df-gid", r.id), el("span", "df-badge", REV_ZH[r.status] ?? r.status), el("b", "", r.request.length > 80 ? r.request.slice(0, 78) + "…" : r.request)));
      const meta = row();
      if (r.figure) meta.append(el("span", "df-badge", `圖：${r.figure}`));
      r.refs.forEach((k) => meta.append(refChip(k)));
      if (meta.childElementCount) card.append(meta);
      const g = sess().groups.find((x) => x.id === r.group);
      if (g?.summary.conclusion) card.append(note(`討論結論：${g.summary.conclusion}`));
      const acts = row(btn("到對話討論", () => { D.group = r.group; go(STEPS.findIndex(([k]) => k === "interview")); }, "ghost"));
      if (r.status !== "dropped") acts.append(btn(r.plan ? "重新產生修改方案" : "產生修改方案", () => run("產生修改方案", () => call(`/revisions/${r.id}/plan`)), r.plan ? "ghost" : "primary small"));
      acts.append(btn(r.status === "dropped" ? "恢復" : "放棄", () => run("更新", () => call(`/revisions/${r.id}/status`, { status: r.status === "dropped" ? "open" : "dropped" })), "ghost"));
      card.append(acts);
      if (r.plan) {
        card.append(row(el("p", "df-text", r.plan.summary), engineTag(r.plan.engine)));
        if (r.plan.new_symbols?.length) card.append(newSymbols(r.plan.new_symbols));
        if (r.plan.effects.length) {
          const ul = el("ul", "df-list warn");
          r.plan.effects.forEach((e) => ul.append(el("li", "", `還要注意：${e}`)));
          card.append(ul);
        }
        r.plan.changes.forEach((c) => {
          const cc = el("div", `df-change ${c.applied === true ? "yes" : c.applied === false ? "no" : ""}`);
          cc.append(row(el("span", "df-gid", c.id), el("span", "df-badge", ACT_ZH[c.action] ?? c.action), el("span", "df-plan", `${c.file}${c.index != null ? ` ¶${c.index}` : ""}`), ...(c.rewrite ? [el("span", "df-badge warn", "要改寫")] : [])));
          if (c.rewrite) cc.classList.add("rewrite");
          if (c.before) { const d = el("details"); d.append(el("summary", "", "原段落"), el("p", "df-old", c.before)); cc.append(d); }
          cc.append(el("p", "df-draft", c.text));
          if (c.reason) cc.append(note(c.reason));
          if (c.applied == null) cc.append(row(
            btn("套用", () => run("套用修改", async () => { const v = await call(`/revisions/${r.id}/changes/${c.id}`, { apply: true }); if (state.paper?.name) openPaper(state.paper.name); return v; })),
            btn("不採用", () => run("更新", () => call(`/revisions/${r.id}/changes/${c.id}`, { apply: false })), "ghost")));
          else cc.append(el("span", "df-note", c.applied ? "已套用到論文" : "不採用"));
          card.append(cc);
        });
      }
      main.append(card);
    });
  }

  return { open, reload };
})();
