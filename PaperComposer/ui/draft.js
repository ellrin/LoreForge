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
//   8 審稿      reviewer pass (Opus); questions go back to the conversation
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
  const SEC_ZH = { Introduction: "前言", Methods: "方法", Results: "結果", Discussion: "討論", Conclusion: "結論" };
  const REV_ZH = { open: "討論中", planned: "有修改方案", done: "完成", dropped: "放棄" };
  const ACT_ZH = { "insert-after": "插在這段之後", "insert-before": "插在這段之前", replace: "取代這段", append: "加在章節最後" };
  const STATUS_ZH = { pending: "未開始", active: "進行中", done: "完成", skipped: "略過" };
  let D = null;

  const base = () => `/api/papers/${encodeURIComponent(D.name)}/draft`;
  const sess = () => D.data.session;
  const lens = () => D.data.lens;

  async function open() {
    if (!state.paper?.name) { flash("先開啟一篇論文"); return; }
    const d = $("#dlgDraft");
    if (!d.open) d.showModal();
    if (!D || D.name !== state.paper.name) D = { name: state.paper.name, data: null, step: null, group: null, busy: null, err: null };
    if (!D.tableStyles) { try { D.tableStyles = await api("/api/textable/styles"); } catch { D.tableStyles = []; } }
    $("#dfModel").value = store.get("pc.draftModel", "auto");
    $("#dfModel").onchange = (e) => store.set("pc.draftModel", e.target.value);
    $("#dfRestart").onclick = () => {
      if (!D?.data || D.busy || !confirm("重新開始？目前的討論會移到 draft/archive/。")) return;
      run("重新開始", async () => { const v = await call("/restart"); D.step = 0; D.group = null; D.comments = null; D.draftAnswer = null; return v; });
    };
    await reload();
  }

  async function reload() {
    try {
      D.data = await api(base());
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
    return m.kind;
  }

  // ── references ───────────────────────────────────────
  const refOf = (key) => D.data.material?.refs.find((r) => r.key === key);
  function openDoi(doi) { api("/api/open-link", { doi }).catch((e) => flash(e.message)); }
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
        r.onclick = () => {
          if (!confirm(`回到「${label}」？之後步驟的結果會清掉（先存一份在 draft/archive/）。`)) return;
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
    main.append(h("有意思的地方（假說，之後對話會再確認）"));
    const acc = input(sess().hook.reader_accepts, "同行已經承認的事實是什麼？例如：標註不確定的影像通常直接排除", "textarea");
    const tw = input(sess().hook.twist, "你加了什麼讓人意外的東西？例如：不排除，而是依品質與面積降權", "textarea");
    main.append(el("label", "np-label", "讀者已經承認"), acc, el("label", "np-label", "你的轉折"), tw);
    main.append(h("視角"));
    const avail = lens().available;
    const sel = el("select", "np-input");
    (avail.length ? avail : ["default"]).forEach((n) => { const o = el("option", "", n === "default" && !avail.length ? "default（內建）" : n); o.value = n; sel.append(o); });
    sel.value = sess().lens || lens().name;
    main.append(sel, note(avail.length ? "視角檔在工作區的 lens/ 資料夾：決定要追問什麼、怎麼判讀回答。" : "工作區沒有 lens/ 資料夾，使用內建的中性視角。可以從 private-example 複製 lens/default.toml 改成你自己的。"));
    const levels = el("ul", "df-levels");
    lens().scale.levels.forEach((l) => levels.append(el("li", "", `${l.id} ${l.label}${l.means ? `：${l.means}` : ""}`)));
    main.append(levels);
    main.append(row(btn("儲存，下一步", () => run("儲存", async () => {
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
    main.append(h("上傳實驗記錄"));
    main.append(note("接受 .md .txt .csv .tsv .json .log 與 PDF；Excel、Word 請另存成 CSV 或純文字。記錄裡的順序（日期、版本、run 編號）越完整，越能問出你當時的想法；你在記錄裡寫下的想法，會在問答時當作選項給你。論文裡已有的表格、圖與文獻會自動帶入，不用重貼。整理由 Opus 負責。"));
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
    main.append(file);
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
    main.append(row(btn(sess().experiments.length ? "重新整理" : "整理實驗記錄", () => run("整理實驗記錄", () => call("/record"))), nextBtn()));
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
        const tb = el("table", "df-table");
        const hr = el("tr"); t.columns.forEach((c) => hr.append(el("th", "", c))); tb.append(hr);
        t.rows.forEach((r) => { const tr = el("tr"); r.forEach((c) => tr.append(el("td", "", c))); tb.append(tr); });
        card.append(tb);
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
    main.append(h("題組"));
    main.append(note("每一組最後要整理出：說明、動機、結論。題組數只是估計，對話後還會校正。"));
    const r = row();
    if (!started) r.append(btn(sess().groups.length ? "重新規劃" : "規劃題組", () => run("規劃題組", () => call("/plan"))));
    else r.append(note("已經開始對話，只能在下面直接修改。"));
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
      c.onclick = () => { D.group = g.id; draw(); };
      chips.append(c);
    });
    main.append(chips);
    const g = groups.find((x) => x.id === D.group);
    const st = stateOf(g.id);
    main.append(el("p", "df-purpose", g.purpose));
    const log = el("div", "df-log");
    const turns = turnsOf(g.id);
    turns.forEach((t, i) => {
      const b = el("div", `df-msg ${t.who}`);
      b.append(el("div", "", t.text));
      if (t.who === "ai" && t.options?.length && i === turns.length - 1) {
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
      const ta = el("textarea", "np-input df-answer");
      ta.rows = 3;
      ta.placeholder = "回答（Ctrl+Enter 送出）。不確定也可以直接說，會給你選項。";
      ta.value = D.draftAnswer?.[g.id] ?? "";
      ta.oninput = () => ((D.draftAnswer ??= {})[g.id] = ta.value);
      ta.onkeydown = (e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) send(ta.value); };
      main.append(ta);
      main.append(row(btn("送出", () => send(ta.value)), btn("不確定，給我選項", () => send("不確定，請給我幾個選項"), "ghost")));
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
    ledger(side, g, st);
  }

  function send(text) {
    text = (text ?? "").trim();
    if (!text) return;
    const gid = D.group;
    run("思考下一個問題", async () => {
      const v = await call("/turn", { group: gid, answer: text, model: $("#dfModel").value });
      if (D.draftAnswer) D.draftAnswer[gid] = "";
      return v;
    });
  }

  function ledger(side, g, st) {
    side.append(el("h4", "", "這一組要整理出"));
    const sum = el("dl", "df-sum");
    [["說明", g.summary.explain], ["動機", g.summary.motive], ["結論", g.summary.conclusion], ["讀者承認", g.summary.reader_accepts], ["轉折", g.summary.twist]].forEach(([k, v]) => {
      sum.append(el("dt", "", k), el("dd", v ? "" : "empty", v || "還沒有"));
    });
    side.append(sum);
    side.append(el("h4", "", "狀態表"));
    const ul = el("ul", "df-ledger");
    sess().items.filter((i) => i.group === g.id).forEach((it) => {
      const li = el("li");
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
      if (it.refs?.length) { const rr = el("div", "df-refs"); it.refs.forEach((k) => rr.append(refChip(k))); li.append(rr); }
      ul.append(li);
    });
    side.append(ul);
  }

  // ── 4.5 主張 ─────────────────────────────────────────
  function claims(main) {
    main.append(h("主張登記"));
    main.append(note("每個主張只在一個章節完整說一次，其他地方只簡短回扣；Abstract 與 Conclusion 最後從這裡取最強的主張。"));
    main.append(row(btn(sess().claims.length ? "重新產生" : "產生主張登記", () => run("整理主張", () => call("/claims"))), nextBtn()));
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
    main.append(row(btn(sess().review ? "重新校正" : "校正", () => run("校正", () => call("/review"))), nextBtn()));
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
      try { D.tpls = await api("/api/templates"); } catch { D.tpls = []; }
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
    main.append(h("版型"), PCNew.templateList({ tpls: D.tpls, selected: current, suggested: choice.suggested, onPick: (v) => { picked = v; apply.disabled = !!D.busy || picked === current; } }), row(later, apply));
  }

  // ── 6 大綱 ───────────────────────────────────────────
  function outline(main) {
    main.append(h("段落大綱"));
    main.append(note("每一段標出類型與組成（事實、理由層級、結論）。逐章看過，有意見寫在章節下方，按「修改這一章」。"));
    main.append(row(btn(sess().outline.length ? "全部重新產生" : "產生大綱", () => run("產生大綱", () => call("/outline"))), nextBtn()));
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
    main.append(h("草稿"));
    main.append(note("逐章生成，每段用你的句型庫寫，並照大綱的組成、你的風格筆記與對照表（登記的巨集、\\ref 標籤、縮寫）。每段寫完會跑一次對照表檢查：手打的「Table 1」或不存在的標籤會自動重寫一次，其他問題列在段落下方。建議最後才生成 Introduction 與 Conclusion。引擎用右側「寫作」選的那個。"));
    if (!sess().outline.length) { main.append(note("先完成大綱。")); return; }
    const order = ["Methods", "Results", "Discussion", "Introduction", "Conclusion"];
    order.forEach((sec) => {
      if (!sess().outline.some((o) => o.section === sec)) return;
      const box = el("div", "df-sec");
      const d = sess().drafts.find((x) => x.section === sec);
      const r = row(el("h4", "", `${sec}　${SEC_ZH[sec]}`), btn(d ? "重新生成" : "生成", () => run(`生成 ${sec}`, () => call("/compose", { section: sec, engine: chosenEngine() })), "ghost"));
      if (d) r.append(btn("複製", () => { navigator.clipboard.writeText(d.paras.map((p) => p.text).join("\n\n")); flash("已複製"); }, "ghost"));
      box.append(r);
      if (d?.new_symbols?.length) box.append(newSymbols(d.new_symbols));
      d?.paras.forEach((p) => {
        const card = el("div", "df-card");
        card.append(el("span", "df-gid", p.para), el("p", "df-draft", p.text));
        if (p.problems.length) card.append(note(p.problems.join("；"), "warn"));
        box.append(card);
      });
      main.append(box);
    });
    main.append(row(nextBtn("下一步：審稿")));
  }

  // ── 8 審稿 ───────────────────────────────────────────
  const KIND_ZH = { citation: "引用", contradiction: "矛盾", overclaim: "宣稱過度", "missing-number": "缺數字", impossible: "數字不可能", generic: "空泛", repeat: "重複", "buried-origin": "發想埋太晚", "missing-why": "缺理由" };
  function audit(main) {
    main.append(h("審稿檢查（Opus）"));
    main.append(note("檢查矛盾、宣稱強度、缺數字、空泛句、重複與埋掉的發想。需要你補充的，可以回到對話追問。"));
    main.append(row(btn(sess().audit.length ? "重新審稿" : "審稿", () => run("審稿中", () => call("/audit")))));
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
    box.append(row(btn("登記到對照表", () => run("登記符號", () => call("/notation/register", { symbols: list })), "ghost")));
    return box;
  }

  // ── 9 修訂 ───────────────────────────────────────────
  // Something has to go into the finished paper: new data after a review, a
  // figure, more citations. Each request is discussed in its own group, then
  // Opus proposes paragraph-level changes the author applies one by one.
  async function libraryKeys() {
    if (D.library) return D.library;
    try { D.library = (await api("/api/refs"))?.refs ?? []; } catch { D.library = []; }
    return D.library;
  }

  function revise(main) {
    main.append(h("新增修訂"));
    main.append(note("老師或審稿人要你補什麼？補了什麼實驗、數據、圖、文獻？寫下來，先討論放在哪裡、會影響哪些地方，再產生具體修改。新的實驗記錄請在第 2 步上傳並重新整理；新文獻先在文獻視窗加入文獻庫。"));
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
      if (r.status !== "dropped") acts.append(btn(r.plan ? "重新產生修改方案" : "產生修改方案（Opus）", () => run("產生修改方案", () => call(`/revisions/${r.id}/plan`)), r.plan ? "ghost" : "primary small"));
      acts.append(btn(r.status === "dropped" ? "恢復" : "放棄", () => run("更新", () => call(`/revisions/${r.id}/status`, { status: r.status === "dropped" ? "open" : "dropped" })), "ghost"));
      card.append(acts);
      if (r.plan) {
        card.append(el("p", "df-text", r.plan.summary));
        if (r.plan.new_symbols?.length) card.append(newSymbols(r.plan.new_symbols));
        if (r.plan.effects.length) {
          const ul = el("ul", "df-list warn");
          r.plan.effects.forEach((e) => ul.append(el("li", "", `還要注意：${e}`)));
          card.append(ul);
        }
        r.plan.changes.forEach((c) => {
          const cc = el("div", `df-change ${c.applied === true ? "yes" : c.applied === false ? "no" : ""}`);
          cc.append(row(el("span", "df-gid", c.id), el("span", "df-badge", ACT_ZH[c.action] ?? c.action), el("span", "df-plan", `${c.file}${c.index != null ? ` ¶${c.index}` : ""}`)));
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
