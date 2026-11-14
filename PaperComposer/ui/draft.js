// 討論草稿: build a paper draft through a conversation.
//   1 主題      key value, the "interesting" hypothesis, the lens
//   2 實驗記錄  upload records → experiments and a timeline (the plan starts when it is read)
//   3 對話      one question at a time. The plan (question groups and their items) is the
//               list on the left: progress, jump, delete, add. The record's own answers are
//               confirmed with one click; the likely next answers are asked ahead (server)
//   4 主張      claim register: every claim once, with its home section
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
    ["interview", "3", "對話"],
    ["claims", "4", "主張"],
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
  /** The step a session phase shows (題組 is part of 對話). */
  const stepOf = (phase) => Math.max(0, STEPS.findIndex(([k]) => k === (phase === "plan" ? "interview" : phase)));
  const stepIdx = (k) => STEPS.findIndex(([x]) => x === k);

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
      if (D.step == null) { D.step = stepOf(sess().phase); D.wantOpen = true; }
      D.err = null;
    } catch (e) { D.err = e.message; }
    draw();
    nudge();
  }

  /** Run a call that returns the new view; keep the window responsive. */
  // the button just pressed (WebKit does not focus buttons on click, so focus alone can't tell)
  let pressed = null;
  document.addEventListener("pointerdown", (e) => { pressed = { el: e.target, t: Date.now() }; }, true);
  function consumedField() {
    const byClick = pressed && Date.now() - pressed.t < 1500 ? pressed.el.closest?.("button") : null;
    const a = byClick ?? document.activeElement;
    if (!a?.closest?.("#dlgDraft")) return null;
    if (!byClick && a.matches("input, textarea") && a.dataset.typed) return a;
    const r = a.closest(".df-row");
    if (!r) return null;
    const inRow = r.querySelector("input[data-typed], textarea[data-typed]");
    if (inRow) return inRow;
    const prev = r.previousElementSibling;
    return prev?.matches?.("input[data-typed], textarea[data-typed]") ? prev : null;
  }
  async function run(label, fn, auto = false) {
    if (D.busy) return;
    // the field this action sends (the focused one, or the one in / just above the button's row) is
    // cleared after a success, kept after a failure; an automatic run (the first question asked by
    // itself after a poll) sends none
    const c = auto ? null : consumedField();
    if (c) c.dataset.consumed = "1";
    D.busy = label; D.err = null; draw();
    let ok = false;
    try { const v = await fn(); if (v) D.data = v; ok = true; }
    catch (e) { D.err = e.message; }
    D.dropConsumed = ok;
    if (!ok) document.querySelectorAll("#dlgDraft [data-consumed]").forEach((x) => delete x.dataset.consumed);
    D.busy = null; draw();
    schedulePrep();
    nudge();
  }

  /** 預想選項 are prepared in the background: look again until every group has them. */
  function schedulePrep() {
    clearTimeout(D.prepTimer);
    if (!D.data || !(sess().groups.some((g) => g.prep === "running") || sess().figures?.running || D.data.planning)) return;
    D.prepTimer = setTimeout(async () => {
      if (!$("#dlgDraft").open || D.busy) { schedulePrep(); return; }
      try { D.data = await api(base()); draw(); nudge(); } catch { /* next time */ }
      schedulePrep();
    }, D.data.planning ? 1500 : 3000);
  }
  const call = (path, body, method) => api(base() + path, body ?? {}, method);
  const go = (i) => { D.step = i; if (STEPS[i]?.[0] === "interview") D.wantOpen = true; draw(); nudge(); };

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
  /** What the author typed (and not yet saved) and which folds are open survive a redraw (a poll,
   * another save). Only fields he typed in are kept, only back into the same field (same label,
   * same order, same value as when it was drawn), and the field a successful action consumed is
   * left as the new draw has it. */
  const FIELDS = "input:not([type=file]):not([type=checkbox]):not([type=radio]), textarea, details";
  document.addEventListener("input", (e) => {
    if (!e.isTrusted || !e.target.closest?.("#dlgDraft")) return;
    e.target.dataset.typed = "1";
    if (D) D.lastTypedEl = e.target;
  }, true);
  function fieldKeys(root) {
    const seen = {};
    return [...root.querySelectorAll(FIELDS)].map((x) => {
      const label = x.tagName === "DETAILS" ? `d:${x.querySelector("summary")?.textContent.replace(/（[^）]*）.*$/, "") ?? ""}` : `${x.tagName}:${x.getAttribute("aria-label") || x.placeholder || x.closest("label")?.textContent || ""}`;
      const k = `${D.step}|${label}`;
      seen[k] = (seen[k] ?? 0) + 1;
      return [`${k}#${seen[k]}`, x];
    });
  }
  function keepTyping(main, side) {
    const snap = new Map();
    let focus = null;
    for (const root of [main, side]) for (const [k, x] of fieldKeys(root)) {
      if (x.tagName === "DETAILS") snap.set(k, { open: x.open });
      else if (x.dataset.typed && !(D.dropConsumed && x.dataset.consumed)) snap.set(k, { value: x.value, orig: x.dataset.orig ?? "", consumed: !!x.dataset.consumed });
      if (x === document.activeElement && !(D.dropConsumed && x.dataset.consumed)) focus = { k, start: x.selectionStart, end: x.selectionEnd };
    }
    return () => {
      for (const root of [main, side]) for (const [k, x] of fieldKeys(root)) {
        const s = snap.get(k);
        if (s && "open" in s) x.open = s.open;
        else if (s && "value" in s && (x.dataset.orig ?? "") === s.orig) {
          x.value = s.value; x.dataset.typed = "1";
          if (s.consumed && D.busy) x.dataset.consumed = "1";
          x.dispatchEvent(new Event("input"));
        }
        if (focus?.k === k && x.focus) { x.focus(); try { x.setSelectionRange(focus.start, focus.end); } catch { /* not a text field */ } }
      }
    };
  }
  /** Each field remembers the value it was drawn with. */
  function markDrawn(main, side) {
    for (const root of [main, side]) for (const x of root.querySelectorAll(FIELDS)) if (x.tagName !== "DETAILS") x.dataset.orig = x.value;
  }

  function draw() {
    if (!D) return;
    const main = $("#dfMain"), side = $("#dfSide"), steps = $("#dfSteps");
    const restore = D.drawnStep === D.step ? keepTyping(main, side) : null;
    D.drawnStep = D.step;
    try { drawInner(main, side, steps); markDrawn(main, side); } finally { restore?.(); D.dropConsumed = false; }
  }
  function drawInner(main, side, steps) {
    main.innerHTML = ""; side.innerHTML = ""; steps.innerHTML = "";
    if (!D.data) { main.append(el("p", "df-note bad", D.err ?? "載入中…")); return; }
    $("#dfLens").textContent = `視角：${lens().name}`;
    const phaseAt = stepOf(sess().phase);
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
    const key0 = STEPS[D.step][0];
    // 對話 shows its wait where the answer goes
    if (D.busy && key0 !== "interview") main.append(el("div", "df-busy", `${D.busy}…`));
    if (D.err) main.append(el("div", "df-note bad", D.err));
    const key = STEPS[D.step][0];
    ({ topic, record, interview, claims, review, template, outline, draft: drafting, audit, revise })[key](main, side);
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
    // typed or pasted record text: stored as a record file like an upload
    const txt = el("textarea", "np-input df-rectext");
    txt.rows = 6;
    txt.placeholder = "直接輸入或貼上實驗紀錄";
    txt.setAttribute("aria-label", "實驗紀錄文字");
    const addText = () => {
      const v = txt.value;
      if (!v.trim()) return;
      run("加入", async () => {
        const r = await fetch(`${base()}/records/text`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ text: v }) });
        if (!r.ok) throw new Error(await r.text());
        return r.json();
      });
    };
    txt.onkeydown = (e) => { if ((e.key === "Enter" && !e.isComposing) && (e.metaKey || e.ctrlKey)) { e.preventDefault(); addText(); } };
    // 搜尋檔案: LoreEcho finds it (through LoreShelf), each file chosen there arrives as a record
    const find = LoreFindFile.button({ purpose: "實驗紀錄", exts: ["md", "txt", "csv", "tsv", "json", "log", "pdf"], context: { paper: D.name }, onAdded: () => { if (!D.busy && $("#dlgDraft").open) reload(); } });
    main.append(row(file, find), txt, row(btn("加入", addText, "ghost")));
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
    // what the record step found: folded; the conflicts stay in view
    const fold = (title, n) => { const d = el("details", "df-fold"); d.append(el("summary", "", n ? `${title}（${n}）` : title)); main.append(d); return d; };
    if (sess().record_summary) fold("摘要").append(el("p", "df-text", sess().record_summary));
    if (sess().experiments.length) {
      const expBox = fold("實驗", sess().experiments.length);
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
      expBox.append(t);
    }
    // 待補 and 加入圖表 fold away; 待補 opens when it holds something
    const pendBox = el("details", "df-fold");
    pendBox.append(el("summary", "", `待補${(sess().pending ?? []).length ? `（${sess().pending.length}）` : ""}`));
    pendingBlock(pendBox, true);
    main.append(pendBox);
    if (sess().conflicts.length) {
      main.append(h("記錄與論文對不上的數字"));
      const ul = el("ul", "df-list warn");
      sess().conflicts.forEach((c) => ul.append(el("li", "", c)));
      main.append(ul);
    }
    if (sess().thoughts.length) {
      const thBox = fold("記錄裡你的想法", sess().thoughts.length);
      const ul = el("ul", "df-thoughts");
      sess().thoughts.forEach((t) => { const li = el("li"); li.append(el("span", "df-gid", t.id), document.createTextNode(` ${t.text}`)); if (t.about.length) li.append(el("span", "df-role", t.about.join(" "))); ul.append(li); });
      thBox.append(ul);
    }
    const floatBox = el("details", "df-fold");
    floatBox.append(el("summary", "", "加入圖表"));
    main.append(floatBox);
    D.floats ??= PCNew.floats({
      paper: D.name,
      sections: (state.paper?.sections ?? []).map((x) => x.title),
      styles: D.tableStyles ?? [],
      keyValue: D.data.key_value,
      title: "",
      figuresBefore: (D.data.material?.floats ?? []).filter((f) => f.kind !== "table").length,
      onSaved: (v) => { D.data = v; D.floats = null; if (state.paper?.name) openPaper(state.paper.name); draw(); },
    });
    floatBox.append(D.floats.el);
    if (sess().tables.length) {
      const tbBox = fold("從記錄整理出的表格", sess().tables.length);
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
        tbBox.append(card);
      });
    }
    const floats = D.data.material?.floats ?? [];
    if (floats.length) {
      const flBox = fold("論文裡已有的圖表", floats.length);
      const ul = el("ul", "df-list");
      floats.forEach((f) => ul.append(el("li", "", `${f.kind === "table" ? "表" : "圖"} ${f.id}（${f.section}）：${f.caption || "（沒有標題）"}`)));
      flBox.append(ul);
    }
    if (sess().timeline.length) {
      const tlBox = fold("時間線", sess().timeline.length);
      const ol = el("ol", "df-timeline");
      sess().timeline.forEach((s) => {
        const li = el("li");
        li.append(el("b", "", `T${s.step} ${s.what}`));
        if (s.observed) li.append(el("div", "", `看到：${s.observed}`));
        if (s.changed) li.append(el("div", "", `接著：${s.changed}`));
        ol.append(li);
      });
      tlBox.append(ol);
    }
    // the step's one main action, at the end: 整理 until it has run, then 下一步
    const done = sess().experiments.length > 0;
    const tidy = btn(done ? "重新整理" : "整理實驗記錄", () => run("整理實驗記錄", () => call("/record")), done ? "ghost small" : "primary small");
    main.append(row(tidy, stepTag("record"), el("span", "df-grow"), ...(done ? [btn("下一步", () => goInterview(), "primary small")] : [])));
  }

  // ── 3 對話 ───────────────────────────────────────────
  // The page's job: read one question, answer (pick, type or say it), go on. The plan is the
  // quiet list on the left (where you are, how much is left); 寫法筆記 and 圖表規劃 are folded
  // on the right; 深度討論 takes the right side while it is open.
  const stateOf = (gid) => D.data.states.find((s) => s.group === gid);
  const turnsOf = (gid) => D.data.turns.filter((t) => t.group === gid);
  const model = () => $("#dfModel").value;
  const live = (g) => g.status !== "done" && g.status !== "skipped";
  /** Seconds of the author's time per question (reading, answering, waiting), measured on a1. */
  const SEC_PER_Q = 40;
  const SHORT_SEC = { Introduction: "前言", Methods: "方法", Results: "結果", Discussion: "討論", Conclusion: "結論" };
  const groupName = (g) => g.id === "wp" ? "待補" : g.title;
  /** In the list: the title without its parenthetical (labels, record ids, notes to the model). */
  const shortName = (g) => g.id === "wp" ? "待補" : (g.title.replace(/[（(][^）)]*[）)]/g, "").trim() || g.title);
  /** Items still open in a group (done = nothing missing; left out = not counted). */
  function itemProgress(g) {
    const st = stateOf(g.id);
    const items = sess().items.filter((i) => i.group === g.id && !i.excluded);
    const done = items.filter((i) => { const s = st?.items.find((x) => x.id === i.id); return s && !s.missing.length; }).length;
    return { done, all: items.length };
  }
  /** Items whose record answer waits for 對／不對 (shown before the group's first question). */
  const toConfirm = (gid) => (D.data.to_confirm?.[gid] ?? []).map((id) => sess().items.find((i) => i.id === id)).filter(Boolean);
  /** Questions answered, and about how many are left (each open group: its open items, at most its budget). */
  function overall() {
    const groups = sess().groups;
    const asked = groups.reduce((n, g) => n + g.turns, 0);
    let left = 0;
    groups.filter(live).forEach((g) => {
      const st = stateOf(g.id);
      if (st?.ready || st?.over_budget) return;
      const p = itemProgress(g);
      const conf = g.turns ? 0 : toConfirm(g.id).length;
      left += (conf ? 1 : 0) + Math.max(conf ? 0 : 1, Math.min((st?.budget ?? 8) - g.turns - (conf ? 1 : 0), p.all - p.done - conf));
    });
    return { asked, left, mins: Math.max(1, Math.round((left * SEC_PER_Q) / 60)) };
  }
  /** The group after `g` still to talk about (in plan order, wrapping). */
  function nextGroup(g) {
    const gs = sess().groups;
    const i = gs.indexOf(g);
    return [...gs.slice(i + 1), ...gs.slice(0, i)].find((x) => live(x) && x.id !== g.id) ?? null;
  }

  /** Into 對話 (from 實驗記錄's 下一步, the step list, a group): the plan when there is none,
   * else the shown group's first question (asked ahead by the server, so usually at once). */
  function goInterview(gid) {
    D.step = stepIdx("interview");
    if (gid) D.group = gid;
    D.wantOpen = true;
    draw();
    if (!sess().groups.length && !D.data.planning) run("準備問題", () => call("/plan"), true);
    else nudge();
  }
  /** After a navigation (never from a redraw alone): ask the shown group's first question once
   * its options are ready, and have the server prepare what comes next. */
  function nudge() {
    if (!D?.data || STEPS[D.step]?.[0] !== "interview" || D.busy) return;
    const g = sess().groups.find((x) => x.id === D.group);
    if (!g) return;
    prefetch(g.id);
    if (!D.wantOpen) return;
    if (!live(g) || turnsOf(g.id).length || toConfirm(g.id).length) { D.wantOpen = false; return; }
    if (g.prep === "running") return;
    D.wantOpen = false;
    run("準備第一題", () => call("/turn", { group: g.id, model: model() }), true);
  }
  /** The server prepares the group's options and asks its likely next turn ahead (once per state). */
  function prefetch(gid) {
    const k = `${gid}|${turnsOf(gid).length}|${sess().groups.find((x) => x.id === gid)?.prep ?? ""}|${model()}`;
    if (D.prefetched === k) return;
    D.prefetched = k;
    api(base() + "/prefetch", { group: gid, model: model() }).catch(() => { /* only ahead of time */ });
  }

  function interview(main, side) {
    const groups = sess().groups;
    if (!groups.length) {
      main.append(el("div", "df-busy", D.data.planning || D.busy ? "準備問題…" : ""));
      if (!D.data.planning && !D.busy) main.replaceChildren(row(btn("準備問題", () => run("準備問題", () => call("/plan")))));
      return;
    }
    if (!D.group || !groups.some((g) => g.id === D.group)) D.group = sess().active ?? groups.find(live)?.id ?? groups[0].id;
    const g = groups.find((x) => x.id === D.group);
    const wrap = el("div", "df-iv");
    wrap.append(planList(groups, g), questionPane(g));
    main.append(wrap);
    const d = D.disc && sess().discussions?.find((x) => x.id === D.disc && x.group === g.id && x.status === "open");
    if (d) discussion(side, d);
    else if (D.sidePanel === "notes") writingNotes(side);
    else if (D.sidePanel === "figs") figureFold(side);
  }
  /** 寫法筆記 / 圖表規劃: opened beside the question only when wanted. */
  function sideToggle(k, label) {
    const b = btn(label, () => { D.sidePanel = D.sidePanel === k ? null : k; draw(); }, "ghost small df-sidetog");
    b.disabled = false;
    b.setAttribute("aria-pressed", String(D.sidePanel === k));
    if (k === "figs" && figOff()) b.classList.add("warn");
    return b;
  }
  const figLive = (k) => (sess().figures?.items ?? []).filter((x) => x.kind === k && x.status !== "removed").length;
  const figOff = () => { const fp = sess().figures; const o = (k, t) => t && (figLive(k) < t[0] || figLive(k) > t[1]); return !!fp && (o("figure", fp.figures) || o("table", fp.tables)); };
  const figCount = () => sess().figures?.running ? "圖表規劃中" : `圖 ${figLive("figure")}・表 ${figLive("table")}`;

  // ── the plan, on the left ───────────────────────────
  function planList(groups, cur) {
    const nav = el("nav", "df-plan");
    nav.setAttribute("aria-label", "題組");
    let sec = null;
    groups.forEach((g) => {
      const s = g.id === "wp" ? "待補" : g.id.startsWith("v") ? "修訂" : g.id.startsWith("a") ? "審稿" : (SHORT_SEC[g.section] ?? g.section);
      if (s !== sec) { nav.append(el("div", "df-plan-sec", s)); sec = s; }
      const on = g.id === cur.id;
      const r = el("div", `df-pg${on ? " on" : ""}${live(g) ? "" : ` ${g.status}`}`);
      const b = el("button", "df-pg-t");
      b.title = groupName(g);
      b.append(el("span", "t", g.id === "wp" ? "還在等的結果" : shortName(g)));
      const p = itemProgress(g);
      b.append(el("span", "n", g.status === "done" ? "✓" : g.status === "skipped" ? "—" : p.all ? `${p.done}/${p.all}` : ""));
      b.setAttribute("aria-current", String(on));
      b.onclick = () => { D.group = g.id; D.picked = null; D.pickItem = null; D.wantOpen = true; draw(); nudge(); };
      r.append(b);
      if (!/^(wp|v|a)/.test(g.id)) {
        const del = iconButton("trash", "刪除這一組", async (e) => {
          if (!(await LoreShell.ask(e.currentTarget, [`刪除「${groupName(g)}」？`], [{ label: "取消", value: null }, { label: "刪除", value: true, danger: true, primary: true }]))) return;
          savePlan({ groups: sess().groups.filter((x) => x.id !== g.id).map((x) => ({ ...x })), items: [] });
        });
        del.classList.add("df-pg-del");
        r.append(del);
      }
      nav.append(r);
      if (on) nav.append(itemList(g));
    });
    return nav;
  }
  const savePlan = (edit) => run("儲存", () => call("/plan", edit, "PUT"));
  /** The shown group's items: ✓ done, ○ open; × leaves one out of the paper; add one by typing or voice. */
  function itemList(g) {
    const box = el("div", "df-gitems");
    const st = stateOf(g.id);
    sess().items.filter((i) => i.group === g.id).forEach((it) => {
      const s = st?.items.find((x) => x.id === it.id);
      const ok = !it.excluded && s && !s.missing.length;
      const r = el("div", `df-gi${ok ? " ok" : ""}${it.excluded ? " off" : ""}`);
      r.append(el("span", "m", it.excluded ? "–" : ok ? "✓" : "○"), el("span", "t", it.label));
      r.title = [it.label, it.stance].filter(Boolean).join("\n");
      const x = iconButton(it.excluded ? "undo" : "x", it.excluded ? "放回來" : "不寫進論文", () => run("更新", () => call(`/items/${it.id}/exclude`, { excluded: !it.excluded })));
      x.classList.add("df-gi-x");
      r.append(x);
      box.append(r);
    });
    if (/^(wp|v|a)/.test(g.id)) return box;
    const add = input("", "加一項");
    add.setAttribute("aria-label", `加一項 ${g.id}`);
    const go = () => {
      const v = add.value.trim();
      if (!v) return;
      savePlan({ groups: sess().groups.map((x) => ({ ...x })), items: [{ id: "", group: g.id, label: v, role: "design" }] });
    };
    add.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); go(); } };
    const r = row(add);
    const mic = micButton(add);
    if (mic) r.append(mic);
    r.classList.add("df-gi-add");
    box.append(r);
    return box;
  }

  // ── the question ────────────────────────────────────
  function questionPane(g) {
    const pane = el("section", "df-q");
    const all = overall();
    const total = all.asked + all.left;
    const head = el("div", "df-prog");
    const bar = el("div", "df-bar");
    const fill = el("span");
    fill.style.width = `${total ? Math.round((100 * all.asked) / total) : 0}%`;
    bar.append(fill);
    head.append(el("span", "df-prog-t", all.left ? `已答 ${all.asked} 題・約剩 ${all.left} 題（約 ${all.mins} 分鐘）` : `已答 ${all.asked} 題`), bar);
    head.append(sideToggle("notes", `寫法筆記 ${(sess().writing_notes ?? []).length}`), sideToggle("figs", figCount()));
    if (sess().groups.some(live)) head.append(btn("夠了，下一步", () => go(stepIdx("claims")), "ghost small"));
    pane.append(head);
    pane.append(el("h3", "df-qg", groupName(g)));
    const turns = turnsOf(g.id);
    const st = stateOf(g.id);
    const conf = toConfirm(g.id);
    // the record already says it: one click instead of a question each
    if (!turns.length && conf.length && live(g)) {
      pane.append(confirmCard(g, conf));
      if (D.busy) pane.append(el("div", "df-busy", `${D.busy}…`));
      return pane;
    }
    if (!turns.length) {
      if (live(g) && (g.prep === "running" || D.busy)) pane.append(el("div", "df-busy", g.prep === "running" ? "準備這一組的選項…" : `${D.busy}…`));
      else if (live(g)) pane.append(row(btn("問第一題", () => run("準備第一題", () => call("/turn", { group: g.id, model: model() })))));
      else pane.append(groupFoot(g, st));
      return pane;
    }
    // what was said before: one line, opened on demand; the question being asked stands apart
    const last = turns.at(-1);
    const enough = (st?.ready || st?.over_budget) && !D.goOn?.[g.id];
    const asking = last.who === "ai" && live(g) && !enough;
    const past = last.who === "ai" && live(g) ? turns.slice(0, -1) : turns;
    if (past.length) {
      const fold = el("details", "df-fold df-histfold");
      fold.append(el("summary", "", `前面的對話（${past.length}）`));
      const log = el("div", "df-log df-hist");
      past.forEach((t) => log.append(el("div", `df-msg ${t.who}`, t.text)));
      fold.append(log);
      fold.ontoggle = () => { if (fold.open) log.scrollTop = log.scrollHeight; };
      pane.append(fold);
    }
    if (live(g) && enough) {
      // this group has what it needs: the way on is the page's main thing
      const nx = nextGroup(g);
      const card = el("div", "df-enough");
      card.append(el("b", "", "這一組夠了"));
      if (nx) card.append(btn(`下一組：${shortName(nx)} →`, () => nextFrom(g, nx)));
      else card.append(btn("對話完成，下一步：主張", async () => { await closeGroup(g); go(stepIdx("claims")); }));
      if (last.who === "ai") card.append(btn("繼續回答", () => { (D.goOn ??= {})[g.id] = true; draw(); }, "ghost small"));
      pane.append(card);
      if (D.busy) pane.append(el("div", "df-busy", `${D.busy}…`));
      return pane;
    }
    if (asking) {
      // the reply to the last answer is quiet; the question is what the eye lands on
      const q = el("div", "df-msg ai cur");
      const lines = last.text.split("\n").map((x) => x.trim()).filter(Boolean);
      const ask = lines.pop() ?? "";
      if (lines.length) q.append(el("div", "df-reply", lines.join(" ")));
      const qa = el("div", "df-qrow");
      qa.append(el("div", "df-ask", ask));
      const n = discussionsOf(g.id).length;
      const db = btn("不懂這題", () => openDiscussion(g.id), "ghost small");
      if (n) db.append(el("span", "df-disc-n", String(n)));
      qa.append(db);
      q.append(qa);
      pane.append(q);
    }
    if (D.busy) pane.append(el("div", "df-busy", `${D.busy}…`));
    if (asking) {
      const talking = !!(D.disc && sess().discussions?.some((x) => x.id === D.disc && x.group === g.id && x.status === "open"));
      const ch = choicesOf(last);
      if (ch.length) pane.append(choiceCards(ch));
      const ta = el("textarea", "np-input df-answer");
      ta.rows = 2;
      ta.disabled = talking;
      ta.placeholder = talking ? "深度討論中" : ch.length ? "都不是，我自己說" : "回答";
      ta.setAttribute("aria-label", `回答 ${g.id}`);
      ta.value = D.draftAnswer?.[g.id] ?? "";
      ta.oninput = () => ((D.draftAnswer ??= {})[g.id] = ta.value);
      ta.onkeydown = (e) => { if ((e.key === "Enter" && !e.isComposing) && (e.ctrlKey || e.metaKey)) send(ta.value); };
      const ans = row(ta);
      ans.classList.add("df-ansrow");
      const mic = micButton(ta);
      if (mic) ans.append(mic);
      ans.append(btn("送出", () => send(ta.value)));
      if (talking) ans.querySelectorAll("button").forEach((x) => (x.disabled = true));
      pane.append(ans);
      if (!ch.length) { const r = row(btn("給我選項", () => send("不確定，請給我幾個選項"), "ghost small")); pane.append(r); }
    }
    pane.append(groupFoot(g, st));
    return pane;
  }
  /** The end of a group: on to the next (main when it has what it needs), or the interview's end. */
  function groupFoot(g, st) {
    const foot = row();
    foot.classList.add("df-gfoot");
    const nx = nextGroup(g);
    const enough = st?.ready || st?.over_budget;
    if (!live(g)) {
      foot.append(btn("重新打開", () => run("更新題組", () => call(`/group/${g.id}`, { action: "reopen" })), "ghost small"));
    }
    if (!nx && (!live(g) || enough)) {
      const done = el("div", "df-done");
      done.append(el("b", "", "對話完成"), btn("下一步：主張", async () => { if (live(g)) await closeGroup(g); go(stepIdx("claims")); }));
      foot.append(el("span", "df-grow"), done);
      return foot;
    }
    if (enough && live(g)) foot.append(el("span", "df-note ok", "這一組夠了"));
    foot.append(el("span", "df-grow"));
    if (nx) foot.append(btn(live(g) && !enough ? "略過，下一組 →" : "下一組 →", () => nextFrom(g, nx), enough || !live(g) ? "primary small" : "ghost small"));
    return foot;
  }
  /** Close this group (略過 when nothing was said) and open the next one. */
  function nextFrom(g, nx) {
    run("下一組", async () => {
      const v = live(g) ? await call(`/group/${g.id}`, { action: g.turns > 0 ? "close" : "skip" }) : null;
      D.group = nx.id; D.picked = null; D.pickItem = null; D.wantOpen = true;
      return v;
    });
  }
  const closeGroup = (g) => call(`/group/${g.id}`, { action: g.turns > 0 ? "close" : "skip" }).then((v) => (D.data = v)).catch(() => {});

  /** 記錄寫的，對嗎？ The record's answers of a group: 對 for all at once, 不對 per item. */
  function confirmCard(g, conf) {
    const card = el("div", "df-conf");
    card.append(el("h4", "", "記錄寫的，對嗎？"));
    D.notRight ??= {};
    const no = (D.notRight[g.id] ??= []);
    conf.forEach((it) => {
      const r = el("div", `df-conf-i${no.includes(it.id) ? " no" : ""}`);
      const txt = el("div", "t");
      txt.append(el("b", "", it.label), el("span", "", it.from_record.text));
      const ev = el("span", "ev");
      it.from_record.evidence.forEach((e) => { const x = el("span", "df-ev", e); x.title = evidenceText(e); ev.append(x); });
      txt.append(ev);
      const t = el("button", "ghost small", "不對");
      t.setAttribute("aria-pressed", String(no.includes(it.id)));
      t.disabled = !!D.busy;
      t.onclick = () => { const k = no.indexOf(it.id); if (k >= 0) no.splice(k, 1); else no.push(it.id); draw(); };
      r.append(txt, t);
      card.append(r);
    });
    const yes = conf.filter((it) => !no.includes(it.id)).map((it) => it.id);
    const go1 = btn(no.length ? (yes.length ? "其餘都對，下一題" : "下一題") : "都對，下一題", () => run("下一題", async () => {
      const v = await call("/confirm", { group: g.id, items: yes, no: [...no], model: model() });
      delete D.notRight[g.id];
      return v;
    }));
    card.append(row(el("span", "df-grow"), go1));
    return card;
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

  // ── 圖表規劃 (folded in 對話): keep, drop, pick how to draw, or say your own ──
  const TOOL_ZH = { flow: "流程圖", plot: "繪圖", table: "表格", image: "自備影像" };
  function figureFold(side) {
    const fp = sess().figures;
    const head = el("div", "df-dhead");
    const h4 = el("h4", "", `圖表規劃　${figCount()}`);
    head.append(h4, iconButton("x", "收起", () => { D.sidePanel = null; draw(); }));
    side.append(head);
    if (fp && figOff()) side.append(el("span", "df-badge warn", `期刊：圖 ${fp.figures?.[0]}–${fp.figures?.[1]}・表 ${fp.tables?.[0]}–${fp.tables?.[1]}`));
    figurePlan(side);
  }
  function figurePlan(main) {
    const fp = sess().figures;
    const head = row();
    if (!fp?.running) head.append(btn(fp?.items?.length ? "重新規劃" : "規劃圖表", () => run("規劃圖表", () => call("/figures")), "ghost small"));
    if (fp?.engine) head.append(engineTag(fp.engine, fp.seconds != null ? `${fp.seconds} 秒` : ""));
    main.append(head);
    if (fp?.failed) main.append(note(`沒有完成：${fp.failed}`, "warn"));
    if (!fp?.items?.length) return;
    const items = fp.items.map((x) => ({ ...x }));
    const save = () => run("儲存圖表", () => call("/figures", { items }, "PUT"));
    const list = el("div", "df-figs");
    items.forEach((it) => {
      const off = it.status === "removed";
      const card = el("div", `df-card df-fig ${off ? "no" : ""}`);
      const top = row(el("b", "", it.title), el("span", "df-role", TOOL_ZH[it.tool] ?? it.tool));
      top.append(el("span", "grow"));
      // its stand-in is in the paper: make the real figure here, it takes the stand-in's place and label
      const si = it.kind === "figure" && !off && (D.data.stand_ins ?? []).find((x) => x.label === it.label);
      if (si) top.append(btn("用流程圖做", () => makeFromStandIn(it.label, "flow"), "ghost small"), btn("用繪圖做", () => makeFromStandIn(it.label, "plot"), "ghost small"));
      top.append(iconButton(off ? "undo" : "trash", off ? "加回來" : "刪除", () => { it.status = off ? "kept" : "removed"; save(); }));
      card.append(top);
      if (it.supports) card.append(el("p", "df-text", it.supports));
      const ev = el("div", "df-tags");
      (it.evidence ?? []).forEach((e) => ev.append(el("span", "df-badge", e)));
      (it.pending ?? []).forEach((p) => ev.append(el("span", "df-badge warn", `待補 ${p}`)));
      if (it.claims?.length) ev.append(el("span", "df-fig-claims", `支撐 ${it.claims.length} 個主張`));
      if (ev.childElementCount) card.append(ev);
      (it.warnings ?? []).forEach((w) => card.append(note(w, "warn")));
      if (!off) {
        const opts = el("div", "df-opts");
        (it.options ?? []).forEach((o, k) => {
          const b = el("button", "ghost", o);
          b.setAttribute("aria-pressed", String(it.status === "kept" && !it.own && (it.chosen ?? 0) === k));
          b.disabled = !!D.busy;
          b.onclick = () => { it.chosen = k; it.own = ""; it.status = "kept"; save(); };
          opts.append(b);
        });
        const own = input(it.own, "自己說");
        own.setAttribute("aria-label", `自己說 ${it.id}`);
        const keepOwn = () => { if (own.value === (it.own ?? "")) return; it.own = own.value; it.status = "kept"; save(); };
        own.onchange = keepOwn;
        const r = row(own);
        const mic = micButton(own, keepOwn);
        if (mic) r.append(mic);
        card.append(opts, r);
      }
      list.append(card);
    });
    main.append(list);
    const add = input("", "加一張圖或表");
    add.setAttribute("aria-label", "加一張圖或表");
    const addIt = () => { const v = add.value.trim(); if (!v) return; items.push({ id: "", kind: /表|table/i.test(v) ? "table" : "figure", label: "", title: v, tool: "" }); save(); };
    add.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); addIt(); } };
    const r = row(add);
    const mic = micButton(add);
    if (mic) r.append(mic);
    r.append(btn("加入", addIt, "ghost small"));
    main.append(r);
  }

  // ── 寫法筆記: how he said to write, in his words; the drafts follow them ──
  const SCOPE_ZH = [["paragraph", "這段"], ["section", "這節"], ["paper", "整篇"]];
  function writingNotes(side, end = false) {
    const ns = sess().writing_notes ?? [];
    const edit = ns.map((n) => ({ ...n }));
    const save = () => run("儲存寫法筆記", () => call("/notes", { notes: edit }, "PUT"));
    const box = el("div", "df-sidebox");
    const head = el("div", "df-dhead");
    head.append(el("h4", "", "寫法筆記"));
    if (!end) head.append(iconButton("x", "收起", () => { D.sidePanel = null; draw(); }));
    box.append(head);
    const ul = el("ul", "df-notes");
    edit.forEach((n, k) => {
      const li = el("li");
      const t = input(n.text, "", "textarea");
      t.rows = 2;
      t.setAttribute("aria-label", `寫法筆記 ${n.id}`);
      const keepT = () => { if (t.value === n.text) return; n.text = t.value; save(); };
      t.onchange = keepT;
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
      const r = row(sc, keep);
      const mic = micButton(t, keepT);
      if (mic) r.append(mic);
      r.append(iconButton("trash", "刪除", () => { edit.splice(k, 1); save(); }));
      li.append(t, r);
      if (n.conflict) li.append(note("和標點規則衝突：以標點規則為準", "warn"));
      ul.append(li);
    });
    box.append(ul);
    const add = input("", "加一條寫法");
    add.setAttribute("aria-label", "加一條寫法");
    const addIt = () => { const v = add.value.trim(); if (!v) return; edit.push({ id: "", text: v, scope: "paper", group: D.group ?? "" }); save(); };
    add.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); addIt(); } };
    const r = row(add);
    const mic = micButton(add);
    if (mic) r.append(mic);
    r.append(btn("加入", addIt, "ghost small"));
    box.append(r);
    if (end || edit.some((n) => n.keep && !n.kept)) {
      const b = btn("存進我的寫法", () => run("存進我的寫法", () => call("/notes/keep")), "ghost small");
      b.disabled = !!D.busy || !edit.some((n) => n.keep && !n.kept);
      box.append(b);
    }
    side.append(box);
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
    if (!d.messages.length) log.append(note("這裡的內容不會寫進論文。"));
    if (D.busy) log.append(el("div", "df-busy", `${D.busy}…`));
    side.append(log);
    setTimeout(() => (log.scrollTop = log.scrollHeight), 0);
    if (D.discAnswer != null) {
      // the answer drawn from the discussion: the author edits it, then it goes to the interview
      side.append(el("h4", "", "這題的回答"));
      const ans = input(D.discAnswer, "", "textarea");
      ans.rows = 5;
      ans.setAttribute("aria-label", "這題的回答");
      ans.oninput = () => (D.discAnswer = ans.value);
      const mic = micButton(ans);
      const r = row(btn("繼續討論", () => { D.discAnswer = null; draw(); }, "ghost"), el("span", "df-grow"), btn("送出為回答", () => run("送出回答", async () => { const v = await call(`/discuss/${d.id}/close`, { answer: ans.value, model: model() }); D.disc = null; D.discAnswer = null; return v; })));
      if (mic) r.prepend(mic);
      side.append(ans, r);
      return;
    }
    const ta = el("textarea", "np-input");
    ta.rows = 3;
    ta.placeholder = "想問的";
    ta.setAttribute("aria-label", "深度討論");
    ta.value = D.discDraft ?? "";
    ta.oninput = () => (D.discDraft = ta.value);
    const say = () => { const t = ta.value.trim(); if (!t) return; run("討論中", async () => { const v = await call(`/discuss/${d.id}`, { text: t }); D.discDraft = ""; return v; }); };
    ta.onkeydown = (e) => { if ((e.key === "Enter" && !e.isComposing) && (e.ctrlKey || e.metaKey)) say(); };
    const sum = btn("整理成這題的回答", () => run("整理回答", async () => { const r = await call(`/discuss/${d.id}/answer`, {}); D.discAnswer = r.answer ?? ""; return null; }), "primary small");
    // something to sum up only once the author has said something
    if (!d.messages.some((m) => m.who === "me")) sum.disabled = true;
    const r = row(btn("送出", say, "ghost small"), el("span", "df-grow"), sum);
    const mic = micButton(ta);
    if (mic) r.prepend(mic);
    side.append(ta, r);
    if (d.item) side.append(row(btn("這題不用寫進論文", () => run("略過這題", async () => { const v = await call(`/discuss/${d.id}/close`, { exclude: true }); D.disc = null; return v; }), "ghost small")));
  }

  function send(text) {
    text = (text ?? "").trim();
    if (!text) return;
    const gid = D.group;
    const picked = [...(D.picked ?? [])];
    const item = D.pickItem ?? null;
    run("思考下一個問題", async () => {
      const v = await call("/turn", { group: gid, answer: text, picked, item, model: model() });
      // a question back opened 深度討論 (the model saw it was not an answer)
      const d = v.session.discussions?.find((x) => x.group === gid && x.status === "open");
      if (d && d.messages.at(-1)?.who === "ai") D.disc = d.id;
      if (D.draftAnswer) D.draftAnswer[gid] = "";
      D.picked = null; D.pickItem = null; D.pickedOpt = null;
      return v;
    });
  }

  // ── 按住說話 (lore-voice, when the machine has it), on every field of 對話 ─────────
  // The words go into the field as said; `onFinal` runs when the phrase ends (a field that
  // saves on change saves then). Voice is not a trusted input event, so the field is marked
  // typed here: a poll's redraw keeps it like typed text.
  let voiceOk = null, vEvents = null, vCur = null;
  function micButton(field, onFinal) {
    const b = iconButton("mic", "按住說話", () => {});
    b.classList.add("df-mic");
    b.hidden = true;
    (async () => {
      if (voiceOk == null) { try { voiceOk = !!(await api("/api/voice")).available; } catch { voiceOk = false; } }
      b.hidden = !voiceOk;
    })();
    const text = (e) => { try { return JSON.parse(e.data).text ?? ""; } catch { return ""; } };
    const put = (t) => {
      if (!vCur) return;
      const f = vCur.field;
      f.value = (vCur.base + t).trimStart();
      f.dataset.typed = "1";
      f.dispatchEvent(new Event("input"));
    };
    const start = async (e) => {
      e.preventDefault();
      if (!vEvents) {
        vEvents = new EventSource("/api/voice/events");
        vEvents.addEventListener("partial", (x) => put(text(x)));
        vEvents.addEventListener("final", (x) => { put(text(x)); const c = vCur; c?.field.focus(); c?.onFinal?.(); });
        vEvents.addEventListener("error", (x) => { let v = {}; try { v = JSON.parse(x.data ?? "{}"); } catch {} if (v.kind) { voiceOk = false; LoreShell.toast(v.message ?? "語音無法使用", { err: true }); } });
      }
      vCur = { field, onFinal, base: field.value ? `${field.value.trimEnd()} ` : "" };
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
  // Possible answers prepared from the record: per question (the turn names which of the item's
  // candidates answer it, never one already picked), else the turn's own options. A click puts
  // one in the answer box (two combine, edit freely); 1/2/3 on the keyboard; or just type.
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
  /** The choices under question `t`: its candidates (a turn from before shows the item's), else its options. */
  function choicesOf(t) {
    const it = t.about ? sess().items.find((x) => x.id === t.about) : null;
    const ids = t.cands ?? it?.candidates?.map((c) => c.id) ?? [];
    const cands = (it?.candidates ?? []).filter((c) => ids.includes(c.id)).map((c) => ({ ...c, item: it.id }));
    if (cands.length) return cands;
    return (t.options ?? []).map((o, k) => ({ id: null, key: `o${k}`, text: o, evidence: [] }));
  }
  function choiceCards(list) {
    const box = el("div", "df-cands");
    D.picked ??= [];
    list.forEach((c, k) => {
      const on = c.id ? D.picked.includes(c.id) : D.pickedOpt === c.text;
      const b = el("button", `df-cand${on ? " on" : ""}`);
      b.disabled = !!D.busy;
      // a long choice reads as its point, then the rest quieter
      const m = c.text.match(/^(.{8,}?[。；：]|.{8,60}?，)(.+)$/s);
      const tx = el("span", "t");
      if (m && c.text.length > 44) tx.append(el("b", "", m[1].replace(/[；：，]$/, "")), el("span", "more", m[2].trim()));
      else tx.textContent = c.text;
      b.append(el("span", "n", String(k + 1)), tx);
      if (c.evidence?.length) {
        const ev = el("span", "ev");
        c.evidence.forEach((e) => { const x = el("span", "df-ev", e); x.title = evidenceText(e); ev.append(x); });
        b.append(ev);
      }
      b.onclick = () => pickChoice(c);
      box.append(b);
    });
    return box;
  }
  function pickChoice(c) {
    const ta = $(".df-answer");
    if (!ta) return;
    if (c.id) {
      // candidates of one item per answer
      if (D.pickItem && D.pickItem !== c.item) D.picked = [];
      D.pickItem = c.item;
      D.picked ??= [];
      if (D.picked.includes(c.id)) return;
      D.picked.push(c.id);
    } else {
      if (D.pickedOpt === c.text) return;
      D.pickedOpt = c.text;
    }
    ta.value = ta.value.trim() ? `${ta.value.trim()}；${c.text}` : c.text;
    ta.dataset.typed = "1";
    ((D.draftAnswer ??= {})[D.group] = ta.value);
    draw();
    const again = $(".df-answer"); if (again) { again.focus(); again.setSelectionRange(again.value.length, again.value.length); }
  }
  // 1 / 2 / 3 pick a choice when the answer box is not being typed in
  document.addEventListener("keydown", (e) => {
    if (!D || !$("#dlgDraft")?.open || STEPS[D.step]?.[0] !== "interview") return;
    if (!/^[1-3]$/.test(e.key) || e.metaKey || e.ctrlKey || e.altKey) return;
    if (document.activeElement?.matches("textarea, input, select")) return;
    const last = turnsOf(D.group).at(-1);
    const c = last?.who === "ai" ? choicesOf(last)[+e.key - 1] : null;
    if (c) { e.preventDefault(); pickChoice(c); }
  });

  // ── 4 主張 ─────────────────────────────────────────
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
      if (!f.resolved) r.append(btn("回到對話追問", () => run("建立追問", async () => { const v = await call(`/audit/${f.id}/ask`); D.group = `a${f.id}`; D.step = stepIdx("interview"); return v; }), "ghost"));
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
  function pendingBlock(main, noHead) {
    const items = (sess().pending ?? []).map((x) => ({ ...x, cells: [...(x.cells ?? [])], note: { ...x.note } }));
    if (!noHead) main.append(hq("待補", "還在等資料、但論文一定要有的結果。對話會問你為什麼需要、預期什麼；草稿照預期方向寫並標出來；結果出來後在這裡「填結果」。"));
    items.forEach((it) => main.append(pendingCard(it, false, items)));
    const add = input("", "還在等的結果（例如：外部醫院的 Dice）");
    const addP = () => {
      const v = add.value.trim();
      if (!v) return;
      run("新增待補", async () => { const r = await call("/pending", { items: [...items, { id: "", what: v }] }, "PUT"); return r; });
    };
    add.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); addP(); } };
    main.append(row(add, btn("＋ 待補", addP, "ghost")));
  }
  /** One pending item: its fields (record step) or only its fill-in (修訂). */
  function pendingCard(it, fillOnly, all) {
    const card = el("div", `df-card df-pend ${it.status}`);
    const rep = reportOf(it.id);
    const where = rep && (rep.places.length || rep.dependents.length) ? `${rep.places.length} 處・依賴 ${rep.dependents.length} 句` : "";
    card.append(row(el("span", "df-gid", it.id), ...(fillOnly || it.status !== "waiting" ? [el("b", "", it.what)] : []), el("span", `df-badge ${it.status === "waiting" ? "warn" : "flag"}`, it.status === "waiting" ? "等資料" : `已回填${it.direction ? `・${DIR_ZH[it.direction]}` : ""}`), el("span", "df-note", where)));
    if (!fillOnly && it.status === "waiting") {
      const f = (k, ph) => { const i = input(it[k], ""); i.setAttribute("aria-label", ph); i.oninput = () => (it[k] = i.value); i.onchange = () => savePending(all); const l = el("label", "df-field"); l.append(el("span", "", ph), i); return l; };
      const exp = el("select", "np-input df-home");
      exp.append(el("option", "", "實驗：—"));
      sess().experiments.forEach((e) => { const o = el("option", "", `${e.id} ${e.title}`); o.value = e.id; exp.append(o); });
      exp.value = it.experiment ?? ""; if (!it.experiment) exp.selectedIndex = 0;
      exp.onchange = () => { it.experiment = exp.selectedIndex ? exp.value : null; savePending(all); };
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
