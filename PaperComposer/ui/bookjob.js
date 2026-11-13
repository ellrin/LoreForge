// 寫書 job view (docs/book.md): progress, the review window (微調) and the finished book. The
// work runs on the server (crates/server/src/book); this page only watches it, so closing never
// stops it. Opened by ?book-job=<id> (LoreReader's 寫 review card, LoreLinks pc bookjob) or from the
// home's 寫書 list. ?book-job=mock&at=run|review|done|interrupted shows every state without a model.
// Same look and behaviour as SlideComposer's 論文報告 window (todeck.js).
"use strict";

const PCBookJob = (() => {
  const STAGES = [["source", "讀論文"], ["terms", "名詞"], ["plan", "大綱"], ["research", "找資料"], ["review", "微調"], ["write", "撰寫"], ["check", "檢查"]];
  const DEPTH = [["brief", "簡短"], ["standard", "標準"], ["deep", "深入"]];
  const E = (tag, attrs = {}, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c);
    return e;
  };
  const ic = (name) => E("span", { "data-icon": name, style: "display:inline-flex" });
  const paint = (el) => { try { window.paintIcons?.(el); } catch {} };
  const toast = (m, err) => LoreShell.toast(m, err ? { err: true, ms: 7000 } : undefined);
  const J = { id: null, snap: null, at: 0, timer: 0, root: null, mode: "", busy: false, live: false, keep: null };
  const fmt = (ms) => { const s = Math.max(0, Math.round(ms / 1000)); return s < 60 ? `${s} 秒` : `${Math.floor(s / 60)} 分 ${String(s % 60).padStart(2, "0")} 秒`; };

  // ── the server, or a synthetic job for ?book-job=mock ──
  const enc = encodeURIComponent;
  const API = {
    async j(path, opt) {
      const r = await fetch(path, opt);
      const v = await r.json().catch(() => ({}));
      if (!r.ok) throw new Error(v.error ?? `${r.status}`);
      return v;
    },
    post(path, body) { return this.j(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}) }); },
    get: (id) => API.j(`/api/book/jobs/${enc(id)}`),
    list: () => API.j("/api/book/jobs?unfinished=1").then((v) => v.jobs ?? []),
    stop: (id) => API.post(`/api/book/jobs/${enc(id)}/stop`),
    resume: (id) => API.post(`/api/book/jobs/${enc(id)}/resume`),
    discard: (id) => API.j(`/api/book/jobs/${enc(id)}`, { method: "DELETE" }).catch(() => {}),
    review: (id, body) => API.post(`/api/book/jobs/${enc(id)}/review`, body),
    params: (id, body) => API.post(`/api/book/jobs/${enc(id)}/params`, body),
    imports: (id) => API.post(`/api/book/jobs/${enc(id)}/imports`),
    open: (id) => API.j(`/api/book/jobs/${enc(id)}/open`),
  };
  const MOCK = (() => {
    const at = new URL(location.href).searchParams.get("at") ?? "run";
    let t0 = Date.now(), reviewed = null, stopped = false, pending = 2;
    const opts = { length: "standard", lang: "en-zh", page: "b5", book: "new", folder: "/Users/ellrin/Documents/Books" };
    const t = (term, state = "unknown") => ({ term, state });
    const chapters = [
      { id: "c1", title: "Vectors and similarity", depth: "brief", terms: [t("cosine similarity", "learning"), t("dot product", "learning"), t("embedding")], figures: 1, sources: 3 },
      { id: "c2", title: "Softmax and temperature", depth: "standard", terms: [t("softmax", "learning"), t("temperature scaling"), t("logits")], figures: 1, sources: 2 },
      { id: "c3", title: "Contrastive learning", depth: "standard", terms: [t("positive pair"), t("negative sample"), t("data augmentation")], figures: 1, sources: 4 },
      { id: "c4", title: "The InfoNCE loss", depth: "deep", terms: [t("InfoNCE"), t("mutual information"), t("noise-contrastive estimation")], figures: 2, sources: 5 },
      { id: "c5", title: "Hard negatives and the gate", depth: "deep", terms: [t("hard negatives"), t("gating")], figures: 1, sources: 3 },
    ];
    const suggestions = [
      { op: "depth", id: "c1", depth: "brief", reason: "這兩個詞你已經在學了，短一點就好" },
      { op: "merge", id: "c2", with: "c1", reason: "相似度和 softmax 一起講比較順" },
      { op: "retitle", id: "c5", title: "Hard negatives: why the gate helps", reason: "標題說出這章要回答的問題" },
    ];
    function snap() {
      const base = { id: "mock", title: "Gated Contrast for Synthetic Signals", options: { ...opts }, pending_imports: pending };
      if (at === "interrupted") return { ...base, state: "interrupted", step: 4, stages: STAGES.map(([id], i) => ({ id, state: i < 3 ? "done" : "pending", ms: i < 3 ? 4000 : 0 })) };
      const dur = [3, 2, 9, 6];
      const el = at === "review" || at === "done" ? 1e6 : (Date.now() - t0) / 1000;
      let acc = 0, cur = null;
      const st = dur.map((d, i) => {
        const s = { id: STAGES[i][0], state: "pending", ms: 0, f: Math.min(1, Math.max(0, (el - acc) / d)) };
        if (el >= acc + d) { s.state = "done"; s.ms = d * 1000; } else if (cur == null) { cur = i; s.state = "running"; s.ms = (el - acc) * 1000; }
        acc += d;
        return s;
      });
      st[0].counts = { sections: Math.round(9 * st[0].f), fig: Math.round(5 * st[0].f), refs: 41, questions: 3 };
      st[1].counts = { terms: Math.round(58 * st[1].f), teach: Math.round(14 * st[1].f), known: 21 };
      if (st[2].state === "running") { st[2].chars = Math.round(2400 * st[2].f); st[2].agoMs = 600; }
      if (st[2].state === "done") st[2].counts = { chapters: 5, suggestions: 3 };
      st[3].counts = { cards: Math.round(6 * st[3].f), sources: Math.round(11 * st[3].f), imported: 0, pending: 2 };
      const out = { ...base, state: cur == null ? "review" : "running" };
      out.stages = STAGES.map(([id], i) => st[i] ?? { id, state: "pending", ms: 0 });
      if (cur == null) { out.stages[4] = { id: "review", state: "running", ms: 0 }; out.review = { chapters, suggestions, leftOut: ["batch size", "projection head"], notes: [], license: "cc-by" }; }
      if (reviewed || at === "done") {
        out.state = "done";
        out.stages = STAGES.map(([id]) => ({ id, state: "done", ms: 6000, counts: id === "write" ? { chapter: 5, chapters: 5, figures: 6 } : id === "check" ? { problems: 1, pages: 38 } : undefined }));
        out.result = { title: "Contrastive Learning from the Ground Up", chapters: chapters.map((c) => ({ title: c.title })), pages: 38, compiled: true, problems: [{ kind: "chapter", message: "The InfoNCE loss：新符號登記為待確認：\\beta" }], notes: ["LoreReader 沒有開啟：2 篇等開啟後加進書庫"] };
      }
      if (stopped) out.state = "stopped";
      return out;
    }
    return {
      get: async () => snap(), list: async () => [], stop: async () => { stopped = true; }, resume: async () => { stopped = false; t0 = Date.now(); }, discard: async () => {},
      review: async (_id, body) => { reviewed = body; window.__bookReview = body; },
      params: async (_id, body) => Object.assign(opts, body),
      imports: async () => { pending = 0; return { imported: 2, pending: [] }; },
      open: async () => { throw new Error("示範工作沒有書"); },
    };
  })();
  const api = () => (J.id === "mock" ? MOCK : API);

  function countText(st) {
    const c = st.counts;
    if (!c) return "";
    const p = [];
    if (c.sections != null) p.push(`章節 ${c.sections}`);
    if (c.fig != null) p.push(`圖 ${c.fig}`);
    if (c.questions) p.push(`問題 ${c.questions}`);
    if (c.terms != null) p.push(c.teach != null ? `名詞 ${c.terms} → 要講 ${c.teach}` : `名詞 ${c.terms}`);
    if (c.chapters != null && c.chapter == null) p.push(`${c.chapters} 章`);
    if (c.suggestions) p.push(`建議 ${c.suggestions}`);
    if (c.cards != null) p.push(`知識卡 ${c.cards}`);
    if (c.sources != null) p.push(`資料 ${c.sources}`);
    if (c.pending) p.push(`待匯入 ${c.pending}`);
    if (c.offline) p.push("沒有網路");
    if (c.chapter != null) p.push(`第 ${c.chapter}/${c.chapters} 章`);
    if (c.pages) p.push(`${c.pages} 頁`);
    if (c.problems != null && c.pages != null) p.push(`待處理 ${c.problems}`);
    return p.join(" · ");
  }

  function shell() {
    if (J.root) return J.root;
    J.root = E("section", { class: "bkj", id: "bookJob", "aria-label": "寫書" });
    document.body.append(J.root);
    return J.root;
  }
  const spacer = () => E("span", { style: "flex:1" });
  function head(snap) {
    return E("header", { class: "bkj-head" }, ic("bookWrite"), E("h2", {}, snap?.title || "寫書"), spacer(),
      E("button", { type: "button", class: "ico close", "aria-label": "關閉 Esc", onclick: close }, ic("x")));
  }

  /** LoreReader was not running when papers were downloaded: open it, then let the server hand them over. */
  function importsBar(snap) {
    const n = snap.pending_imports ?? 0;
    if (!n) return null;
    const btn = E("button", { type: "button", class: "btn small", onclick: async () => {
      btn.disabled = true;
      try {
        if (J.id !== "mock") await LoreLinks.ensure("lr");
        const r = await api().imports(J.id);
        toast(r.pending?.length ? `還有 ${r.pending.length} 篇沒有加進去` : "已加進 LoreReader 書庫");
        poll();
      } catch (e) { toast(e.message, true); btn.disabled = false; }
    } }, "開啟 LoreReader");
    return E("div", { class: "bkj-note" }, ic("download"), E("span", {}, `${n} 篇論文等 LoreReader 開啟後加進書庫`), spacer(), btn);
  }

  // ── progress ──
  function paintProgress() {
    const snap = J.snap, root = shell(), now = Date.now() - J.at;
    const list = E("ol", { class: "bkj-stages" });
    for (const [id, name] of STAGES) {
      const st = snap.stages.find((x) => x.id === id) ?? { id, state: "pending", ms: 0 };
      const run = st.state === "running" && snap.state === "running";
      const parts = [];
      const ct = countText(st);
      if (ct) parts.push(E("span", {}, ct));
      if (run && st.chars != null) {
        parts.push(E("span", {}, `${st.chars.toLocaleString()} 字`));
        const ago = (st.agoMs ?? 0) + now;
        parts.push(E("span", { class: ago > 30000 ? "stale" : "" }, `${Math.max(1, Math.round(ago / 1000))} 秒前還在寫`));
      }
      const el = st.state === "done" ? st.ms : st.state === "running" ? st.ms + (run ? now : 0) : 0;
      list.append(E("li", { class: `bkj-st ${st.state}`, "aria-current": run ? "step" : null },
        E("span", { class: "mk", "aria-hidden": "true" }, st.state === "done" ? ic("check") : ""),
        E("span", { class: "nm" }, name),
        E("span", { class: "ct" }, ...parts.flatMap((x, i) => (i ? [E("i", {}, "·"), x] : [x]))),
        E("span", { class: "tm" }, el > 0 ? fmt(el) : "")));
    }
    const body = E("div", { class: "bkj-body" }, list);
    const msg = snap.state === "failed" ? snap.error ?? "沒有完成" : snap.state === "stopped" ? "已停止" : snap.state === "interrupted" ? `上次做到第 ${snap.step ?? "?"} 步` : "";
    if (msg) body.append(E("p", { class: "bkj-msg" + (snap.state === "failed" ? " err" : "") }, msg));
    if (snap.state === "done" && snap.result) body.append(doneView(snap.result));
    const bar = importsBar(snap);
    if (bar) body.append(bar);
    const foot = E("footer", { class: "bkj-foot" }, spacer());
    if (snap.state === "running") foot.append(E("button", { type: "button", class: "btn danger", onclick: stop }, "停止"));
    else if (snap.state === "done") foot.append(E("button", { type: "button", class: "btn primary", onclick: openBook }, "打開書"));
    else foot.append(E("button", { type: "button", class: "btn", onclick: discard }, "捨棄"), E("button", { type: "button", class: "btn primary", onclick: resume }, "繼續"));
    root.replaceChildren(head(snap), body, foot);
    paint(root);
    J.mode = "progress";
  }

  function doneView(r) {
    const box = E("div", { class: "bkj-done" });
    const facts = [`${r.chapters?.length ?? 0} 章`];
    if (r.pages) facts.push(`${r.pages} 頁`);
    if (r.compileNote) facts.push(r.compileNote);
    box.append(E("div", { class: "bkj-book" }, E("strong", {}, r.title ?? ""), E("span", { class: "muted" }, facts.join(" · "))));
    const probs = r.problems ?? [];
    if (probs.length) {
      const b = LoreShell.block({ n: "01", title: "待處理", count: String(probs.length), key: "bkj.problems" });
      b.body.append(E("ul", { class: "bkj-list" }, ...probs.map((p) => E("li", {}, p.message))));
      box.append(b);
    }
    const notes = r.notes ?? [];
    if (notes.length) box.append(E("ul", { class: "bkj-list muted" }, ...notes.map((n) => E("li", {}, n))));
    return box;
  }

  async function stop() { try { await api().stop(J.id); } catch (e) { toast(e.message, true); } poll(); }
  async function resume() { try { await api().resume(J.id); } catch (e) { toast(e.message, true); } J.live = true; poll(); }
  async function discard() { try { await api().discard(J.id); } catch (e) { toast(e.message, true); } close(); }
  async function openBook() {
    try {
      const r = await api().open(J.id);
      close();
      await openPaper(r.id);
    } catch (e) { toast(e.message, true); }
  }

  // ── the review window ──
  function paintReview() {
    const snap = J.snap, rv = snap.review ?? { chapters: [], suggestions: [] };
    const root = shell();
    const keep = J.keep?.id === J.id ? J.keep : null;
    const rows = rv.chapters.map((c) => ({ ...c, include: keep?.off?.includes(c.id) ? false : true, terms: [...(c.terms ?? [])] }));
    const opts = { length: "standard", lang: "en-zh", page: "b5", ...(snap.options ?? {}) };
    const sugg = (rv.suggestions ?? []).map((x, i) => ({ ...x, k: i, state: "" }));
    let newId = 0;
    const body = E("div", { class: "bkj-body wide" });
    const hasS = sugg.length > 0;
    const nb = (i) => String(i + (hasS ? 1 : 0)).padStart(2, "0");
    const bS = hasS ? LoreShell.block({ n: "01", title: "建議", count: String(sugg.length), key: "bkj.sugg" }) : null;
    const bOut = LoreShell.block({ n: nb(1), title: "章節", count: String(rows.length), key: "bkj.chapters" });

    const list = E("ol", { class: "bkj-chapters" });
    const depthSeg = (r) => {
      const g = E("div", { class: "seg small", role: "radiogroup", "aria-label": "深度" });
      for (const [v, t] of DEPTH) g.append(E("button", { type: "button", "aria-pressed": String(r.depth === v), onclick: () => { r.depth = v; paintRows(); } }, t));
      return g;
    };
    function paintRows() {
      list.textContent = "";
      bOut.setCount(String(rows.filter((r) => r.include).length));
      rows.forEach((r, i) => {
        const terms = E("span", { class: "tm" }, ...r.terms.map((t) => E("span", { class: `chip ${t.state ?? "unknown"}`, title: t.state === "learning" ? "學習中" : "不懂" }, t.term)));
        list.append(E("li", { class: "row" + (r.include ? "" : " off") },
          E("input", { type: "checkbox", checked: r.include, "aria-label": "這一章要寫", onchange: (e) => { r.include = e.target.checked; paintRows(); } }),
          E("span", { class: "mv" },
            E("button", { type: "button", class: "ico tiny", "aria-label": "上移", disabled: i === 0, onclick: () => { [rows[i - 1], rows[i]] = [rows[i], rows[i - 1]]; paintRows(); } }, ic("up")),
            E("button", { type: "button", class: "ico tiny down", "aria-label": "下移", disabled: i === rows.length - 1, onclick: () => { [rows[i + 1], rows[i]] = [rows[i], rows[i + 1]]; paintRows(); } }, ic("up"))),
          E("span", { class: "no" }, String(rows.slice(0, i + 1).filter((x) => x.include).length).padStart(2, "0")),
          E("input", { class: "ti", value: r.title, "aria-label": "章名", onchange: (e) => (r.title = e.target.value) }),
          depthSeg(r),
          terms));
      });
      paint(list);
      paintSugg();
    }

    // 建議: one structured edit each; 套用 changes the rows above, 忽略 drops it
    const at = (id) => rows.findIndex((r) => r.id === id);
    const ch = (id) => `第 ${at(id) + 1} 章`;
    const depthName = (d) => DEPTH.find(([v]) => v === d)?.[1] ?? d;
    function describe(x) {
      switch (x.op) {
        case "retitle": return `${ch(x.id)}章名 → ${x.title}`;
        case "depth": return `${ch(x.id)} → ${depthName(x.depth)}`;
        case "move": return `${ch(x.id)}移到${x.to ? `${ch(x.to)}後` : "最前面"}`;
        case "remove": return `不寫${ch(x.id)}`;
        case "merge": return `${ch(x.with)}併入${ch(x.id)}`;
        case "add": return `${x.to ? `${ch(x.to)}後` : "最前面"}加一章：${x.title}`;
        default: return x.op;
      }
    }
    const usable = (x) => [x.id, x.with, x.to].every((id) => !id || at(id) >= 0);
    function applyS(x) {
      const i = at(x.id);
      switch (x.op) {
        case "retitle": rows[i].title = x.title; break;
        case "depth": rows[i].depth = x.depth; break;
        case "move": { const [r] = rows.splice(i, 1); rows.splice(x.to ? at(x.to) + 1 : 0, 0, r); break; }
        case "remove": rows[i].include = false; break;
        case "merge": { const w = rows[at(x.with)]; rows[i].terms.push(...w.terms.filter((t) => !rows[i].terms.some((u) => u.term === t.term))); rows.splice(at(x.with), 1); break; }
        case "add": rows.splice(x.to ? at(x.to) + 1 : 0, 0, { id: `n${++newId}`, title: x.title, depth: "standard", include: true, terms: (x.terms ?? []).map((term) => ({ term, state: "unknown" })) }); break;
      }
      x.state = "applied";
      paintRows();
    }
    const sl = E("ul", { class: "bkj-sugg" });
    function paintSugg() {
      if (!bS) return;
      sl.textContent = "";
      const open = sugg.filter((x) => x.state !== "ignored");
      bS.setCount(String(open.filter((x) => !x.state).length));
      for (const x of open) {
        const done = x.state === "applied", ok = usable(x);
        sl.append(E("li", { class: done ? "done" : "" },
          E("span", { class: "why" }, x.reason),
          E("span", { class: "what" }, describe(x)),
          done ? E("span", { class: "st" }, ic("check"), "已套用")
            : E("span", { class: "acts" },
              E("button", { type: "button", class: "btn small", onclick: () => { x.state = "ignored"; paintSugg(); } }, "忽略"),
              E("button", { type: "button", class: "btn small", disabled: !ok, onclick: () => applyS(x) }, "套用"))));
      }
      paint(sl);
    }
    if (bS) bS.body.append(sl);
    bOut.body.append(list);
    paintRows();

    // 篇幅／語言 write the outline again (the job comes back here); the page size is kept for the book
    const setOpt = async (key, v) => {
      if (J.busy || opts[key] === v) return;
      J.busy = true;
      try {
        J.keep = { id: J.id, off: rows.filter((r) => !r.include).map((r) => r.id) };
        await api().params(J.id, { [key]: v });
        opts[key] = v;
        if (key !== "page") { J.mode = ""; J.live = true; poll(); } else paintOpts();
      } catch (e) { toast(e.message, true); paintOpts(); } finally { J.busy = false; }
    };
    const optSeg = (key, items) => {
      const g = E("div", { class: "seg", role: "radiogroup", "data-k": key });
      for (const [v, t] of items) g.append(E("button", { type: "button", "aria-pressed": String(opts[key] === v), "data-v": v, onclick: () => setOpt(key, v) }, t));
      return g;
    };
    const strip = E("div", { class: "bkj-opts" },
      E("span", { class: "ql" }, "篇幅"), optSeg("length", [["short", "短"], ["standard", "標準"], ["long", "長"]]),
      E("span", { class: "ql" }, "語言"), optSeg("lang", [["en-zh", "英中"], ["en", "English"], ["zh", "中文"]]),
      ...(opts.book && opts.book !== "new" ? [] : [E("span", { class: "ql" }, "頁面"), optSeg("page", [["b5", "B5"], ["a4", "A4"]])]));
    // a new book: where it goes (the last folder used, changeable here)
    if (!opts.book || opts.book === "new") {
      const path = E("input", { class: "set-input bkj-folder", value: opts.folder ?? "", "aria-label": "存放的資料夾", spellcheck: "false" });
      const setFolder = async (f) => {
        f = String(f ?? "").trim();
        if (!f || f === opts.folder || J.busy) return;
        J.busy = true;
        try { await api().params(J.id, { folder: f }); opts.folder = f; path.value = f; toast("書會存在這裡"); } catch (e) { toast(e.message, true); path.value = opts.folder ?? ""; } finally { J.busy = false; }
      };
      path.onchange = () => setFolder(path.value);
      const pick = window.PCOpen?.desktop?.() ? E("button", { type: "button", class: "btn small", onclick: async () => {
        try { const r = await API.post("/api/shell/dialog", { kind: "folder", start: path.value || null }); if (r?.path) setFolder(r.path); } catch (e) { toast(e.message, true); }
      } }, ic("folder"), "選擇…") : null;
      strip.append(E("span", { class: "ql" }, "存放"), path, ...(pick ? [pick] : []));
      setTimeout(() => (path.scrollLeft = path.scrollWidth));
    }
    function paintOpts() { strip.querySelectorAll(".seg").forEach((g) => g.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(opts[g.dataset.k] === b.dataset.v)))); }

    body.append(strip, ...(bS ? [bS] : []), bOut);
    if (rv.leftOut?.length) {
      const bL = LoreShell.block({ n: nb(2), title: "沒排進章節的名詞", count: String(rv.leftOut.length), key: "bkj.left", open: false });
      bL.body.append(E("div", { class: "bkj-left" }, ...rv.leftOut.map((t) => E("span", { class: "chip" }, t))));
      body.append(bL);
    }
    for (const n of rv.notes ?? []) body.append(E("div", { class: "bkj-note" }, ic("download"), E("span", {}, n)));
    const bar = importsBar(snap);
    if (bar) body.append(bar);

    const submit = async (skip) => {
      if (J.busy) return;
      if (!skip && !rows.some((r) => r.include)) { toast("至少要留一章"); return; }
      J.busy = true;
      try {
        await api().review(J.id, skip ? { skip: true } : { chapters: rows.map((r) => ({ id: r.id, title: r.title, depth: r.depth, include: r.include, terms: r.terms.map((t) => t.term) })) });
        J.keep = null; J.mode = ""; J.live = true;
        poll();
      } catch (e) { toast(e.message, true); } finally { J.busy = false; }
    };
    root.replaceChildren(head(snap), body, E("footer", { class: "bkj-foot" }, spacer(),
      E("button", { type: "button", class: "btn", onclick: () => submit(true) }, "略過"),
      E("button", { type: "button", class: "btn primary", onclick: () => submit(false) }, "開始寫")));
    paint(root);
    J.mode = "review";
  }

  // ── polling ──
  async function poll() {
    clearTimeout(J.timer);
    if (!J.id) return;
    try {
      const snap = await api().get(J.id);
      J.snap = snap; J.at = Date.now();
      if (snap.state === "running") J.live = true;
      if (snap.state === "review") { if (J.mode !== "review") paintReview(); }
      else paintProgress();
      if (snap.state === "running" || snap.state === "review") J.timer = setTimeout(poll, 1000);
    } catch (e) {
      shell().replaceChildren(head(null), E("div", { class: "bkj-body" }, E("p", { class: "bkj-msg err" }, e.message)),
        E("footer", { class: "bkj-foot" }, spacer(), E("button", { type: "button", class: "btn primary", onclick: poll }, "重試")));
      paint(J.root);
      J.mode = "";
    }
  }
  setInterval(() => { if (J.id && J.mode === "progress" && J.snap?.state === "running") paintProgress(); }, 1000);

  function open(id) {
    J.id = id; J.snap = null; J.mode = ""; J.live = false;
    try { const u = new URL(location.href); u.searchParams.set("book-job", id); history.replaceState(null, "", u); } catch {}
    shell().hidden = false;
    document.addEventListener("keydown", esc, true);
    poll();
  }
  const esc = (e) => { if (e.key === "Escape" && J.root && !J.root.hidden && !document.querySelector("dialog[open], .ask, .menu")) { e.preventDefault(); close(); } };
  function close() {
    clearTimeout(J.timer);
    J.id = null; J.mode = "";
    if (J.root) J.root.hidden = true;
    document.removeEventListener("keydown", esc, true);
    try { const u = new URL(location.href); u.searchParams.delete("book-job"); u.searchParams.delete("at"); history.replaceState(null, "", u); } catch {}
  }

  /** 從論文寫…: a LoreReader paper → a new book (the default folder) with the review window. */
  async function start(sha, { book = "new", folder, page } = {}) {
    const v = await API.post("/api/book", { source: { kind: "lr", sha }, book, folder, page, mode: "review" });
    open(v.job);
  }

  // start-up: ?book-job=<id>, else 「上次做到第 N 步，繼續？」 for a book job a quit left unfinished
  async function boot() {
    const u = new URL(location.href);
    const id = u.searchParams.get("book-job");
    if (id) { open(id); return; }
    let jobs = [];
    try { jobs = (await API.list()).filter((j) => j.state === "interrupted"); } catch {}
    const j = jobs[0];
    if (!j) return;
    const pick = await LoreShell.form({
      title: "寫書",
      build: (b) => b.append(E("p", {}, `${j.title || "上次的書"}：上次做到第 ${j.step ?? "?"} 步，繼續？`)),
      actions: [{ id: "drop", label: "捨棄", run: async () => { await API.discard(j.id); } }, { id: "go", label: "繼續", primary: true }],
    });
    if (pick === "go") { try { await API.resume(j.id); } catch (e) { toast(e.message, true); } open(j.id); }
  }
  setTimeout(boot, 500);
  /** Book jobs not finished yet (running ones too), newest first: the home's 寫書 list. */
  const unfinished = () => API.j("/api/book/jobs").then((v) => (v.jobs ?? []).filter((j) => j.state !== "done"));
  return { open, close, start, list: unfinished, get id() { return J.id; } };
})();
