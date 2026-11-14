// LoreFlow.edit(): the flow-figure editor, a full-screen dialog over the host page (docs/flow-figures.md).
// Same frame as the plotting editor (LorePlot.edit): top bar · left · figure · right · 助手.
//
//   const r = await LoreFlow.edit({
//     spec,        // a figure to re-open (lore-flow FlowSpec JSON), null = start a new one
//     title,       // shown in the bar
//     target,      // "slide" | "paper" | null: the 完成 label and the frame
//     frame,       // [w, h] pt the figure fits in (default: the spec's, else a 16:9 slide)
//     pageWidth,   // mm: the document's width (a paper's \linewidth), offered as 「本文」 next to 寬
//     doc,         // the document whose notation registry is in force (sent to the server)
//     source,      // starting material for step 1 (a paper's title and method), or ""
//     sources,     // async () => [{ label, text }]: papers to start from (LoreReader, PaperComposer)
//     api,         // "/flow/api"
//     save,        // async ({ spec }) → true | message; 完成 calls it (null: no 完成)
//   });          // → { spec } when saved, null when closed
//
// Steps on the left (each one short exchange with the model, the author decides), the L1 text,
// layout, colours and legend on the right. Editor chrome uses the app theme; the figure never does.
(() => {
  "use strict";
  const LF = (window.LoreFlow = window.LoreFlow || {});
  const clone = (o) => (o == null ? o : JSON.parse(JSON.stringify(o)));
  function h(tag, attrs, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs ?? {})) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e[k] = v;
      else if (k === "value") e.value = v;
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) e.append(c instanceof Node ? c : document.createTextNode(String(c)));
    return e;
  }
  const ic = (name) => (typeof window.icon === "function" ? window.icon(name) : document.createTextNode(""));
  const toast = (m, o) => (window.LoreShell?.toast ? window.LoreShell.toast(m, o) : console.log(m));
  function iconBtn(name, label, fn, cls = "ico") {
    const b = h("button", { type: "button", class: cls, "aria-label": label, title: label });
    b.append(ic(name));
    b.onclick = fn;
    return b;
  }
  function block(n, title, key, open = true) {
    if (window.LoreShell?.block) return window.LoreShell.block({ n, title, key: `flow.${key}`, open });
    const el = h("section", { class: "block", "data-open": "true" }, h("div", { class: "block-head" }, h("span", { class: "n" }, n), h("h3", {}, title)));
    el.body = h("div", { class: "block-body" });
    el.append(el.body);
    el.setOpen = () => {};
    return el;
  }
  async function post(api, path, body) {
    const r = await fetch(`${api}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || `${r.status}`);
    return j;
  }
  // KaTeX for symbols in the steps (the figure itself is drawn by the server)
  let katexLoading = null;
  function katex(base) {
    if (window.katex) return Promise.resolve();
    if (katexLoading) return katexLoading;
    const css = h("link", { rel: "stylesheet", href: `${base}/katex/katex.min.css` });
    document.head.append(css);
    katexLoading = new Promise((ok) => { const s = h("script", { src: `${base}/katex/katex.min.js` }); s.onload = ok; s.onerror = ok; document.head.append(s); });
    return katexLoading;
  }
  function tex(s) {
    const span = h("span");
    String(s ?? "").split(/(\$[^$]+\$)/).forEach((part) => {
      if (/^\$[^$]+\$$/.test(part) && window.katex) {
        const m = h("span");
        try { window.katex.render(part.slice(1, -1), m, { throwOnError: false }); } catch { m.textContent = part; }
        span.append(m);
      } else span.append(part);
    });
    return span;
  }
  const DASHES = [["solid", "實線"], ["dash", "虛線"], ["sysDash", "短虛線"], ["sysDot", "點線"], ["lgDash", "長虛線"], ["dashDot", "虛點線"]];
  const DASH_SVG = { solid: "", dash: "4 3", sysDash: "3 1", sysDot: "1 1", lgDash: "8 3", dashDot: "4 3 1 3", lgDashDot: "8 3 1 3" };
  const LINE_NAMES = { data: "資料流", supervision: "監督", feedback: "回饋", "loss-out": "損失輸出", compare: "比較", optional: "可選", shared: "共享權重", skip: "跳接", concat: "串接", down: "降採樣", up: "升採樣", cross: "跨 block", stage: "階段", repeat: "重複", zoom: "放大" };

  /**
   * The figure for a paper: its print size in mm, every line holding LaTeX set again with TeX glyphs
   * (KaTeX flattened into SVG by the shared commons/ui/mathsvg.js, as the plotting tool does), so the
   * PDF the server prints has real maths. `macros`: the document's registered macros.
   */
  LF.paperSvg = async function ({ spec, api = "/flow/api", doc = "", macros = {} }) {
    const base = api.replace(/\/api$/, "");
    const r = await fetch(`${api}/svg`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ spec, doc, tex: true }) });
    if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || `${r.status}`);
    const text = await r.text();
    if (!window.LoreMathSvg) await new Promise((ok, no) => { const sc = h("script", { src: `${base}/mathsvg.js` }); sc.onload = ok; sc.onerror = () => no(new Error("讀不到 mathsvg.js")); document.head.append(sc); });
    const MS = window.LoreMathSvg;
    await MS.loadKatex(`${base}/katex/`);
    const dom = new DOMParser().parseFromString(text, "image/svg+xml");
    const lines = [...dom.querySelectorAll("text[data-tex]")];
    const fontOf = (el) => {
      const f = el.getAttribute("data-font") || "";
      return f === "KaTeX_Main" || !f ? "KaTeX_Main, 'Times New Roman', serif" : `'${f}', Carlito, 'Helvetica Neue', Arial, sans-serif`;
    };
    // the fonts load before anything is measured (measurements are cached)
    const host = MS.ensureHost();
    const probe = document.createElement("span");
    probe.className = "lp-t";
    probe.innerHTML = lines.map((el) => MS.htmlOf(el.getAttribute("data-tex"), macros)).join(" ");
    host.append(probe);
    try { await document.fonts.ready; } catch {}
    probe.remove();
    for (const el of lines) {
      const m = MS.measure(el.getAttribute("data-tex"), { family: fontOf(el), size: +el.getAttribute("data-size") || 10, weight: el.getAttribute("data-bold") ? 700 : 400, math: "katex", color: el.getAttribute("data-color") || "#000" }, macros);
      const anchor = el.getAttribute("text-anchor") || "start";
      const g = new DOMParser().parseFromString(`<svg xmlns="http://www.w3.org/2000/svg">${m.place(+el.getAttribute("x"), +el.getAttribute("y"), { anchor })}</svg>`, "image/svg+xml").documentElement.firstElementChild;
      if (!g) continue;
      const node = dom.importNode(g, true);
      if (el.getAttribute("transform")) { const w = dom.createElementNS("http://www.w3.org/2000/svg", "g"); w.setAttribute("transform", el.getAttribute("transform")); w.append(node); el.replaceWith(w); } else el.replaceWith(node);
    }
    return new XMLSerializer().serializeToString(dom.documentElement);
  };

  LF.edit = function (opts = {}) {
    return new Promise((resolve) => openEditor(opts, resolve));
  };

  async function openEditor(opts, resolve) {
    const api = opts.api ?? "/flow/api";
    const base = api.replace(/\/api$/, "");
    const frame = opts.frame ?? opts.spec?.frame ?? (opts.target === "paper" ? [496, 300] : [672, 357]);
    const S = {
      spec: opts.spec ? clone(opts.spec) : { lore_flow: 1, dsl: "", frame },
      hist: [], fut: [], cat: null, info: null, step: null, replies: {}, busy: {}, assist: false,
    };
    S.spec.frame = S.spec.frame ?? frame;
    const disc = () => (S.spec.discussion ??= { source: opts.source ?? "", answers: {} });
    disc();
    if (!S.spec.discussion.source && opts.source) S.spec.discussion.source = opts.source;

    const dlg = h("dialog", { class: "lf-editor", "aria-label": "流程圖" });
    document.body.append(dlg);
    const name = h("span", { class: "lf-name" }, opts.title || "新流程圖");
    const undoB = iconBtn("undo", "復原 ⌘Z", () => undo());
    const redoB = iconBtn("redo", "重做 ⇧⌘Z", () => redo());
    const aiB = h("button", { type: "button", class: "ghost", "aria-label": "助手", "aria-pressed": "false" }, ic("spark"), h("span", { class: "lf-lbl" }, "助手"));
    const svgB = iconBtn("download", "下載 SVG", () => downloadSvg());
    const cancelB = h("button", { type: "button", class: "ghost" }, "取消");
    const doneB = h("button", { type: "button", class: "primary small" }, opts.target === "slide" ? "插入投影片" : opts.target === "paper" ? "存入論文" : "完成");
    const top = h("header", { class: "top lf-top" }, h("span", { class: "lf-brand" }, ic("graph"), "流程圖"), name, h("span", { class: "lf-sep" }), undoB, redoB, aiB, h("span", { class: "lf-grow" }), svgB, cancelB, opts.save ? doneB : null);
    const left = h("aside", { class: "lf-left", "aria-label": "步驟" });
    const stage = h("main", { class: "lf-stage" });
    const sheet = h("div", { class: "lf-sheet" });
    stage.append(sheet);
    const info = h("span", { class: "lf-info" });
    const warn = h("span", { class: "lf-warn" });
    const status = h("footer", { class: "statusbar lf-status" }, info, warn, h("span", { class: "grow" }));
    const assist = h("aside", { class: "lf-assist", "aria-label": "助手", hidden: true });
    const right = h("aside", { class: "lf-right", "aria-label": "版面與顏色" });
    const body = h("div", { class: "lf-body" }, left, h("div", { class: "lf-mid" }, stage, status), assist, right);
    dlg.append(top, body);
    dlg.showModal();
    if (typeof window.paintIcons === "function") window.paintIcons(dlg);
    katex(base).then(() => { if (S.cat) buildLeft(); });

    let closed = false;
    const close = (val) => {
      if (closed) return;
      closed = true;
      dlg.close();
      dlg.remove();
      document.removeEventListener("keydown", onKey, true);
      resolve(val);
    };
    cancelB.onclick = async () => {
      if (S.hist.length && !(await confirmLose())) return;
      close(null);
    };
    dlg.addEventListener("cancel", (e) => { if (e.target !== dlg) return; e.preventDefault(); cancelB.click(); });
    async function confirmLose() {
      if (window.LoreShell?.ask) return (await window.LoreShell.ask(cancelB, ["放棄這次的修改？"], [{ label: "放棄", value: true }, { label: "繼續編輯", value: null, primary: true }])) === true;
      return confirm("放棄這次的修改？");
    }
    doneB.onclick = async () => {
      if (!S.spec.dsl.trim()) { toast("還沒有圖", { err: true }); return; }
      if (S.info?.error) { toast(S.info.error, { err: true }); return; }
      doneB.disabled = true;
      const ok = await opts.save({ spec: clone(S.spec) });
      doneB.disabled = false;
      if (ok === true) close({ spec: clone(S.spec) });
      else if (ok) toast(ok, { err: true });
    };
    const onKey = (e) => {
      if (!dlg.open) return;
      const inField = e.target.matches?.("input, textarea, select, [contenteditable]");
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !inField) { e.preventDefault(); e.shiftKey ? redo() : undo(); }
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && opts.save) { e.preventDefault(); doneB.click(); }
    };
    document.addEventListener("keydown", onKey, true);
    aiB.onclick = () => {
      S.assist = !S.assist;
      aiB.setAttribute("aria-pressed", String(S.assist));
      assist.hidden = !S.assist;
      body.classList.toggle("lf-with-assist", S.assist);
      if (S.assist) buildAssist();
    };

    try { S.cat = await (await fetch(`${api}/catalogue`)).json(); } catch (e) { toast("讀不到流程圖設定：" + e.message, { err: true }); S.cat = { steps: [], layouts: [], palette: [], lines: [] }; }
    aiB.disabled = !S.cat.llm;
    const STEPS = [...S.cat.steps, { id: "list", title: "線性列表" }];
    // a new figure opens on the first step (the material may be filled in already), a half-discussed one where it stopped
    const begun = Object.keys(S.spec.discussion?.answers ?? {}).length > 0;
    S.step = opts.spec?.dsl ? null : begun ? (STEPS.find((s) => !answered(s.id))?.id ?? "list") : "start";

    // ── history ──
    let typing = null;
    function change(fn, coalesce) {
      const before = JSON.stringify(S.spec);
      fn(S.spec);
      if (JSON.stringify(S.spec) === before) return;
      if (!(coalesce && typing === coalesce)) { S.hist.push(before); if (S.hist.length > 200) S.hist.shift(); }
      typing = coalesce ?? null;
      S.fut = [];
      refresh(!coalesce);
    }
    function undo() {
      if (!S.hist.length) return;
      S.fut.push(JSON.stringify(S.spec));
      S.spec = JSON.parse(S.hist.pop());
      typing = null;
      refresh(true);
    }
    function redo() {
      if (!S.fut.length) return;
      S.hist.push(JSON.stringify(S.spec));
      S.spec = JSON.parse(S.fut.pop());
      typing = null;
      refresh(true);
    }

    // ── preview ──
    let tok = 0, timer = null, layTok = 0, layFor = "";
    function refresh(rebuild = true) {
      undoB.disabled = !S.hist.length;
      redoB.disabled = !S.fut.length;
      if (rebuild) buildLeft();
      clearTimeout(timer);
      timer = setTimeout(draw, 120);
    }
    async function draw() {
      if (!S.spec.dsl.trim()) {
        S.info = null;
        sheet.innerHTML = "";
        sheet.append(h("div", { class: "lf-empty" }, "從左邊開始"));
        info.textContent = ""; warn.textContent = "";
        buildRight();
        return;
      }
      const my = ++tok;
      let r;
      try { r = await post(api, "/preview", { spec: S.spec, doc: opts.doc ?? "", width: 1200 }); } catch (e) { warn.textContent = e.message; return; }
      if (my !== tok) return;
      S.info = r;
      if (r.error) { warn.textContent = r.error; warn.classList.add("lf-err"); }
      else { warn.classList.remove("lf-err"); warn.textContent = [...(r.problems ?? []), ...(r.warnings ?? [])].join("　"); }
      if (r.svg) {
        sheet.innerHTML = r.svg;
        const svg = sheet.firstElementChild;
        const vb = svg.viewBox.baseVal;
        const z = Math.min((stage.clientWidth - 48) / vb.width, (stage.clientHeight - 48) / vb.height, 3);
        svg.style.width = `${Math.max(80, vb.width * z)}px`;
        svg.style.height = `${Math.max(40, vb.height * z)}px`;
        svg.removeAttribute("width"); svg.removeAttribute("height");
        info.textContent = r.size ? `${Math.round(r.size[0] * 0.3528)} × ${Math.round(r.size[1] * 0.3528)} mm · ${(r.blocks ?? []).length} 個 block` : "";
      }
      buildRight();
      buildList();
    }
    new ResizeObserver(() => { clearTimeout(timer); timer = setTimeout(draw, 120); }).observe(stage);

    async function downloadSvg() {
      try {
        const r = await fetch(`${api}/svg`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ spec: S.spec, doc: opts.doc ?? "" }) });
        if (!r.ok) throw new Error((await r.json().catch(() => ({}))).error || r.status);
        const a = h("a", { href: URL.createObjectURL(await r.blob()), download: `${(opts.title || "flow").replace(/[\\/:*?"<>|]/g, "_")}.svg` });
        a.click();
      } catch (e) { toast(e.message, { err: true }); }
    }

    // ── left: the steps ──
    function answered(id) {
      const a = S.spec.discussion?.answers?.[id];
      if (id === "list") return !!S.spec.dsl.trim();
      if (id === "start") return !!(a || S.spec.discussion?.source || "").trim();
      return Array.isArray(a) ? a.length > 0 : !!(a && String(a).trim());
    }
    function summary(id) {
      const a = S.spec.discussion?.answers?.[id];
      if (id === "list") return S.spec.dsl.trim() ? `${(S.info?.blocks ?? []).length || ""} block` : "";
      if (id === "start") return a || S.spec.discussion?.source || "";
      if (Array.isArray(a)) return id === "vars" ? a.map((x) => x.latex ? `$${x.latex}$` : x.id).join(" ") : `${a.length} 個 block`;
      return a ?? "";
    }
    function setAnswer(id, v, coalesce) {
      change((sp) => { (sp.discussion ??= { source: "", answers: {} }).answers ??= {}; sp.discussion.answers[id] = v; }, coalesce);
    }
    function buildLeft() {
      left.innerHTML = "";
      const list = h("ol", { class: "lf-steps" });
      STEPS.forEach((st, i) => {
        const cur = S.step === st.id;
        const li = h("li", { class: `lf-step${cur ? " cur" : ""}${answered(st.id) ? " done" : ""}` });
        const v = summary(st.id);
        const head = h("button", { type: "button", class: "lf-sh", "aria-expanded": String(cur) }, h("span", { class: "n" }, String(i + 1)), h("span", { class: "t" }, st.title), h("span", { class: "v" }, cur ? "" : tex(String(v).replace(/\s+/g, " "))));
        head.onclick = () => { S.step = cur ? null : st.id; buildLeft(); };
        li.append(head);
        if (cur) li.append(stepBody(st, i));
        list.append(li);
      });
      left.append(list);
    }
    function nav(i) {
      const prev = i > 0 ? h("button", { type: "button", class: "btn small", onclick: () => { S.step = STEPS[i - 1].id; buildLeft(); } }, "上一步") : null;
      const next = i < STEPS.length - 1 ? h("button", { type: "button", class: "btn small", onclick: () => { S.step = STEPS[i + 1].id; buildLeft(); if (S.cat.llm && !S.replies[S.step] && S.step !== "list") ask(S.step); } }, "下一步") : null;
      return h("div", { class: "lf-row" }, prev, h("span", { class: "lf-grow" }), next);
    }
    async function ask(step) {
      S.busy[step] = true;
      buildLeft();
      try {
        S.replies[step] = await post(api, "/discuss", { step, source: S.spec.discussion?.source ?? "", answers: S.spec.discussion?.answers ?? {}, doc: opts.doc ?? "" });
        const r = S.replies[step];
        // the io rows are a proposal the author edits: take them when nothing is there yet
        if (step === "io" && Array.isArray(r.rows) && !(S.spec.discussion?.answers?.io ?? []).length) setAnswer("io", r.rows);
      } catch (e) { toast(e.message, { err: true }); }
      S.busy[step] = false;
      buildLeft();
    }
    function reply(step) {
      const r = S.replies[step];
      const out = [];
      if (S.busy[step]) out.push(h("p", { class: "lf-wait" }, "想一下"));
      if (r?.say) out.push(h("p", { class: "lf-say" }, r.say));
      if (r?.question) out.push(h("p", { class: "lf-q" }, r.question));
      return out;
    }
    function askBtn(step) {
      return S.cat.llm ? h("button", { type: "button", class: "btn small", disabled: S.busy[step], onclick: () => ask(step) }, ic("spark"), S.replies[step] ? "再問一次" : "問模型") : null;
    }
    function stepBody(st, i) {
      const b = h("div", { class: "lf-sbody" });
      const a = S.spec.discussion?.answers ?? {};
      if (st.id === "start") {
        const ta = h("textarea", { class: "set-input lf-ta", rows: 9, placeholder: "論文題目與方法，或一段描述", "aria-label": "出發點" });
        ta.value = S.spec.discussion?.source ?? "";
        ta.oninput = () => change((sp) => { disc(); sp.discussion.source = ta.value; }, "source");
        b.append(ta);
        if (opts.sources) {
          const pick = h("button", { type: "button", class: "btn small" }, ic("news"), "從論文");
          pick.onclick = async () => {
            let list = [];
            try { list = await opts.sources(); } catch (e) { toast(e.message, { err: true }); return; }
            if (!list.length) { toast("沒有開著的論文"); return; }
            const items = list.map((x) => ({ label: x.label, run: () => { ta.value = x.text; change((sp) => { disc(); sp.discussion.source = x.text; }); } }));
            if (window.LoreShell?.menu) window.LoreShell.menu(pick, items); else items[0].run();
          };
          b.pick = pick;
        }
      }
      if (st.id !== "list") b.append(...reply(st.id));
      if (st.id === "vars") b.append(varsPicker());
      else if (st.id === "io") b.append(ioTable());
      else if (st.id === "list") b.append(listStep());
      else {
        const opts2 = S.replies[st.id]?.options ?? [];
        const cur = a[st.id] ?? "";
        const ta = h("textarea", { class: "set-input lf-ta", rows: 3, "aria-label": st.title, placeholder: "你的決定" });
        {
          if (opts2.length) {
            const box = h("div", { class: "lf-opts" });
            opts2.forEach((o, k) => {
              const text = o.detail ? `${o.label}：${o.detail}` : o.label;
              const btn = h("button", { type: "button", class: `lf-opt${S.replies[st.id].pick === k ? " rec" : ""}`, "aria-pressed": String(cur === text) }, h("b", {}, o.label), o.detail ? h("span", {}, o.detail) : null);
              btn.onclick = () => setAnswer(st.id, text);
              box.append(btn);
            });
            b.append(box);
          }
          if (st.id !== "start") {
            ta.value = cur;
            ta.oninput = () => setAnswer(st.id, ta.value, `a-${st.id}`);
            b.append(ta);
          }
        }
      }
      b.append(h("div", { class: "lf-row" }, st.id !== "list" ? askBtn(st.id) : null, b.pick ?? null));
      b.append(nav(i));
      return b;
    }
    let notation = null;
    function varsPicker() {
      const wrap = h("div", { class: "lf-opts" });
      const chosen = () => S.spec.discussion?.answers?.vars ?? [];
      const has = (id) => chosen().some((x) => x.id === id);
      const toggle = (x, on) => setAnswer("vars", on ? [...chosen().filter((y) => y.id !== x.id), x] : chosen().filter((y) => y.id !== x.id));
      const row = (x) => {
        const cb = h("input", { type: "checkbox", checked: has(x.id) });
        cb.onchange = () => toggle({ id: x.id, latex: x.latex, meaning: x.meaning ?? "", registered: x.registered !== false }, cb.checked);
        return h("label", { class: "lf-sym" }, cb, h("span", { class: "m" }, tex(`$${x.latex}$`)), h("span", { class: "d" }, x.meaning ?? ""), x.registered === false ? h("span", { class: "lf-new" }, "新") : null);
      };
      const seen = new Set();
      for (const x of [...(S.replies.vars?.options ?? []), ...chosen()]) { if (seen.has(x.id)) continue; seen.add(x.id); wrap.append(row(x)); }
      // the registry: pick symbols the model did not offer
      const sel = h("select", { class: "set-input", "aria-label": "從對照表加" }, h("option", { value: "" }, "從對照表加…"));
      const fill = () => (notation ?? []).filter((e) => !seen.has(e.id)).forEach((e) => sel.append(h("option", { value: e.id }, `${e.id} — ${e.meaning || e.latex}`)));
      if (notation) fill(); else post(api, "/notation", { doc: opts.doc ?? "" }).then((r) => { notation = r.entries ?? []; fill(); }).catch(() => {});
      sel.onchange = () => { const e = (notation ?? []).find((x) => x.id === sel.value); if (e) toggle({ id: e.id, latex: e.latex, meaning: e.meaning, registered: true }, true); };
      wrap.append(sel);
      return wrap;
    }
    function ioTable() {
      const rows = S.spec.discussion?.answers?.io ?? [];
      const wrap = h("div", { class: "lf-opts" });
      const put = (i, k, v) => setAnswer("io", rows.map((r, j) => (j === i ? { ...r, [k]: v } : r)), `io-${i}-${k}`);
      rows.forEach((r, i) => {
        const field = (k, label) => { const x = h("input", { class: "set-input", value: r[k] ?? "", "aria-label": label }); x.oninput = () => put(i, k, x.value); return h("div", { class: "lf-f" }, h("span", { class: "k" }, label), x); };
        const name = h("input", { class: "set-input lf-bname", value: r.block ?? "", "aria-label": "block" });
        name.oninput = () => put(i, "block", name.value);
        wrap.append(h("div", { class: "lf-type" }, name, field("input", "輸入"), field("output", "輸出"), field("what", "是什麼")));
      });
      const add = h("button", { type: "button", class: "btn small", onclick: () => setAnswer("io", [...rows, { block: `block ${rows.length + 1}`, input: "", output: "", what: "" }]) }, ic("plus"), "block");
      wrap.append(h("div", { class: "lf-row" }, add));
      return wrap;
    }
    let listEl = null;
    function listStep() {
      const wrap = h("div", { class: "lf-opts" });
      const gen = h("button", { type: "button", class: "btn small", disabled: !S.cat.llm || S.busy.list }, ic("spark"), S.spec.dsl.trim() ? "重新產生" : "產生列表");
      gen.onclick = async () => {
        S.busy.list = true; buildLeft();
        try {
          const r = await post(api, "/graph", { source: S.spec.discussion?.source ?? "", answers: S.spec.discussion?.answers ?? {}, doc: opts.doc ?? "" });
          change((sp) => { sp.dsl = r.dsl; });
          if (r.findings?.length) toast(r.findings.map((f) => f.message).join("；"));
        } catch (e) { toast(e.message, { err: true }); }
        S.busy.list = false; buildLeft();
      };
      if (S.busy.list) wrap.append(h("p", { class: "lf-wait" }, "畫列表"));
      listEl = h("pre", { class: "lf-linear", "aria-label": "每個 block 一行" });
      fillList();
      wrap.append(listEl, h("div", { class: "lf-row" }, gen));
      return wrap;
    }
    function fillList() { if (listEl) listEl.textContent = (S.info?.linear ?? []).join("\n") || "—"; }
    function buildList() { fillList(); const s = left.querySelector(".lf-step:last-child .lf-sh .v"); if (s && S.step !== "list") s.textContent = summary("list"); }

    // ── right: L1 text, layout, colours, legend ──
    let codeEl = null;
    function buildRight() {
      const keep = right.scrollTop;
      const act = document.activeElement;
      if (act === codeEl) { buildRightRest(); return; }
      // typing in a field on the right: rebuilt when it loses focus
      if (right.contains(act) && act.matches("input[type=text], input:not([type]), textarea")) { act.addEventListener("blur", () => setTimeout(buildRight), { once: true }); return; }
      right.innerHTML = "";
      const b1 = block("01", "列表", "dsl");
      codeEl = h("textarea", { class: "set-input lf-code", spellcheck: "false", "aria-label": "列表（L1）", rows: 12 });
      codeEl.value = S.spec.dsl;
      codeEl.oninput = () => change((sp) => { sp.dsl = codeEl.value; }, "dsl");
      b1.body.append(codeEl);
      right.append(b1);
      right.append(h("div", { class: "lf-rest" }));
      buildRightRest();
      right.scrollTop = keep;
    }
    function buildRightRest() {
      const rest = right.querySelector(".lf-rest");
      if (!rest) return;
      rest.innerHTML = "";
      if (!S.info || S.info.error) return;
      rest.append(layoutBlock(), colourBlock(), legendBlock());
    }
    let layCache = { key: "", list: [] };
    function layoutBlock() {
      const b = block("02", "版面", "layout");
      const grid = h("div", { class: "lf-layouts" });
      const fill = (list) => {
        grid.innerHTML = "";
        for (const l of list) {
          const btn = h("button", { type: "button", class: "lf-lay", "aria-pressed": String((S.spec.layout ?? "") === (l.id === "auto" ? "" : l.id) || S.spec.layout === l.id), title: l.name }, h("span", { class: "lf-pic" }), h("span", {}, l.name, l.private ? h("span", { class: "lf-mine" }, " 我的") : null));
          btn.querySelector(".lf-pic").innerHTML = l.svg;
          btn.onclick = () => change((sp) => { sp.layout = l.id === "auto" ? "" : l.id; sp.dirs = {}; });
          grid.append(btn);
        }
      };
      const key = JSON.stringify([S.spec.dsl, S.spec.types, S.spec.blocks, S.spec.nodes, S.spec.lines, S.spec.legend, S.spec.theme]);
      if (layCache.key === key) fill(layCache.list);
      else {
        grid.append(h("p", { class: "lf-wait" }, "排版"));
        const my = ++layTok;
        post(api, "/layouts", { spec: S.spec, doc: opts.doc ?? "", width: 160 }).then((r) => { if (my !== layTok) return; layCache = { key, list: r.layouts ?? [] }; fill(layCache.list); }).catch(() => {});
      }
      b.body.append(grid);
      // width (mm; the frame is in pt): the document's own by default, any other by hand
      const PT = 72 / 25.4;
      const wNow = Math.round((S.spec.frame[0] / PT) * 10) / 10;
      const setW = (mm) => { if (!(mm > 20)) return; change((sp) => { const k = (mm * PT) / sp.frame[0]; sp.frame = [Math.round(mm * PT * 10) / 10, Math.round(sp.frame[1] * k)]; }); };
      const wIn = h("input", { class: "set-input lf-w", type: "number", step: "0.1", min: "20", value: String(wNow), "aria-label": "寬 mm" });
      wIn.onchange = () => setW(+wIn.value);
      const wRow = h("div", { class: "lf-f" }, h("span", { class: "k" }, "寬 mm"), wIn);
      if (opts.pageWidth) {
        const on = Math.abs(wNow - opts.pageWidth) < 0.6;
        const pb = h("button", { type: "button", class: "ghost lf-wchip", "aria-pressed": String(on) }, `本文 ${opts.pageWidth}`);
        pb.onclick = () => setW(opts.pageWidth);
        wRow.append(pb);
      }
      b.body.append(wRow);
      // direction per block
      b.body.append(h("div", { class: "lf-sub" }, "方向"));
      for (const bl of S.info.blocks ?? []) {
        const dir = S.spec.dirs?.[bl.id] ?? "";
        const seg = h("span", { class: "lf-seg" }, ...[["", "自動"], ["right", "→"], ["down", "↓"]].map(([v, t]) => {
          const x = h("button", { type: "button", "aria-pressed": String(dir === v), "aria-label": v === "right" ? "左到右" : v === "down" ? "上到下" : "自動" }, t);
          x.onclick = () => change((sp) => { sp.dirs ??= {}; if (v) sp.dirs[bl.id] = v; else delete sp.dirs[bl.id]; });
          return x;
        }));
        b.body.append(h("div", { class: "lf-f" }, h("span", { class: "k", title: bl.title }, bl.title || bl.id), seg));
      }
      return b;
    }
    function swatchPop(anchor, colours, pick) {
      const pop = h("div", { class: "menu lf-swatches" });
      for (const [nm, fill, line] of colours) {
        const x = h("button", { type: "button", title: nm, style: `background:#${fill};border:2px solid #${line}` });
        x.onclick = () => { pop.remove(); pick(fill, line); };
        pop.append(x);
      }
      const r = anchor.getBoundingClientRect();
      Object.assign(pop.style, { position: "fixed", left: `${Math.min(r.left, innerWidth - 240)}px`, top: `${r.bottom + 4}px`, width: "220px", zIndex: 1000 });
      dlg.append(pop);
      setTimeout(() => document.addEventListener("click", (e) => { if (!pop.contains(e.target)) pop.remove(); }, { once: true }));
    }
    function colourBlock() {
      const b = block("03", "顏色與線", "colour");
      const types = S.spec.types ?? [];
      const pal = S.cat.palette ?? [];
      b.body.append(h("div", { class: "lf-sub" }, "block 種類"));
      types.forEach((t, i) => {
        const sw = h("button", { type: "button", class: "lf-sw", "aria-label": "顏色", style: `background:#${t.fill};border:2px solid #${t.line}` });
        sw.onclick = () => swatchPop(sw, pal, (fill, line) => change((sp) => { sp.types[i] = { ...sp.types[i], fill, line }; }));
        const lab = h("input", { class: "set-input", value: t.label ?? "", "aria-label": "名稱" });
        lab.oninput = () => change((sp) => { sp.types[i].label = lab.value; }, `type-${i}`);
        const del = iconBtn("trash", "刪除", () => change((sp) => {
          const id = sp.types[i].id;
          sp.types.splice(i, 1);
          for (const m of [sp.blocks ?? {}, sp.nodes ?? {}]) for (const k of Object.keys(m)) if (m[k] === id) delete m[k];
        }), "ico small");
        b.body.append(h("div", { class: "lf-f" }, sw, lab, del));
      });
      const add = h("button", { type: "button", class: "btn small" }, ic("plus"), "種類");
      add.onclick = () => change((sp) => {
        sp.types ??= [];
        let n = sp.types.length + 1;
        while (sp.types.some((t) => t.id === `t${n}`)) n++;
        const [, fill, line] = pal[(sp.types.length) % Math.max(1, pal.length)] ?? ["", "EDEDED", "595959"];
        sp.types.push({ id: `t${n}`, label: "", fill, line });
      });
      b.body.append(h("div", { class: "lf-row" }, add));
      const typeSel = (val, set) => {
        const s = h("select", { class: "set-input" }, h("option", { value: "" }, "—"), ...types.map((t) => h("option", { value: t.id, selected: val === t.id }, t.label || t.id)));
        s.onchange = () => set(s.value);
        return s;
      };
      if (types.length) {
        b.body.append(h("div", { class: "lf-sub" }, "block"));
        for (const bl of S.info.blocks ?? []) {
          b.body.append(h("div", { class: "lf-f" }, h("span", { class: "k", title: bl.title }, bl.title || bl.id), typeSel(S.spec.blocks?.[bl.id] ?? "", (v) => change((sp) => { sp.blocks ??= {}; if (v) sp.blocks[bl.id] = v; else delete sp.blocks[bl.id]; }))));
        }
        const mods = h("details", { class: "lf-mods", open: Object.keys(S.spec.nodes ?? {}).length > 0 || S.modsOpen }, h("summary", { class: "lf-sub" }, "模組"));
        mods.ontoggle = () => { S.modsOpen = mods.open; };
        b.body.append(mods);
        for (const n of S.info.nodes ?? []) {
          if (["label", "op"].includes(n.module)) continue;
          mods.append(h("div", { class: "lf-f" }, h("span", { class: "k", title: n.label }, n.id), typeSel(S.spec.nodes?.[n.id] ?? "", (v) => change((sp) => { sp.nodes ??= {}; if (v) sp.nodes[n.id] = v; else delete sp.nodes[n.id]; }))));
        }
      }
      b.body.append(h("div", { class: "lf-sub" }, "線"));
      const inks = [["黑", "000000", "000000"], ["灰", "7F7F7F", "7F7F7F"], ...pal.map(([n, , l]) => [n, l, l])];
      for (const l of S.info.lines ?? []) {
        const look = S.spec.lines?.[l.id] ?? {};
        const color = look.color ?? l.color;
        const sw = h("button", { type: "button", class: "lf-sw", "aria-label": "顏色", style: `background:#${color}` });
        sw.onclick = () => swatchPop(sw, inks, (c) => change((sp) => { sp.lines ??= {}; sp.lines[l.id] = { ...(sp.lines[l.id] ?? {}), color: c }; }));
        const dash = h("select", { class: "set-input", "aria-label": "線型" }, ...DASHES.map(([v, t]) => h("option", { value: v, selected: (look.dash ?? l.dash) === v }, t)));
        dash.onchange = () => change((sp) => { sp.lines ??= {}; sp.lines[l.id] = { ...(sp.lines[l.id] ?? {}), dash: dash.value }; });
        const mean = h("input", { class: "set-input", value: look.meaning ?? "", placeholder: "意思", "aria-label": "意思" });
        mean.oninput = () => change((sp) => { sp.lines ??= {}; sp.lines[l.id] = { ...(sp.lines[l.id] ?? {}), meaning: mean.value }; }, `mean-${l.id}`);
        b.body.append(h("div", { class: "lf-f" }, h("span", { class: "k" }, LINE_NAMES[l.id] ?? l.id), sw, dash), h("div", { class: "lf-f" }, h("span", { class: "k" }), mean));
      }
      return b;
    }
    function markSvg(key) {
      const [kind, id] = key.split(":");
      const svg = (inner) => { const s = h("span", { class: "lf-mark" }); s.innerHTML = `<svg viewBox="0 0 26 14" width="26" height="14">${inner}</svg>`; return s; };
      if (kind === "line") {
        const l = (S.info.lines ?? []).find((x) => x.id === id) ?? {};
        const look = S.spec.lines?.[id] ?? {};
        return svg(`<line x1="1" y1="7" x2="25" y2="7" stroke="#${look.color ?? l.color ?? "000000"}" stroke-width="1.6" stroke-dasharray="${DASH_SVG[look.dash ?? l.dash] ?? ""}"/>`);
      }
      const t = (S.spec.types ?? []).find((x) => x.id === id);
      return svg(t ? `<rect x="7" y="1" width="12" height="12" fill="#${t.fill}" stroke="#${t.line}"/>` : "");
    }
    function legendBlock() {
      const b = block("04", "圖例", "legend");
      const lg = S.spec.legend ?? {};
      const show = lg.show || (lg.rows?.length ? "on" : "auto");
      const seg = (vals, cur, set) => h("span", { class: "lf-seg" }, ...vals.map(([v, t]) => { const x = h("button", { type: "button", "aria-pressed": String(cur === v) }, t); x.onclick = () => set(v); return x; }));
      b.body.append(h("div", { class: "lf-f" }, h("span", { class: "k" }, "圖例"), seg([["auto", "自動"], ["on", "自選"], ["off", "不放"]], show, (v) => change((sp) => {
        sp.legend ??= {};
        sp.legend.show = v;
        if (v === "on" && !(sp.legend.rows ?? []).length) sp.legend.rows = clone(S.info.suggestions ?? []);
      }))));
      if (show === "on") {
        b.body.append(h("div", { class: "lf-f" }, h("span", { class: "k" }, "位置"), seg([["below", "圖下方"], ["above", "圖上方"]], lg.pos || "below", (v) => change((sp) => { sp.legend.pos = v; }))));
        const rows = lg.rows ?? [];
        const all = [...rows, ...(S.info.suggestions ?? []).filter((s) => !rows.some((r) => r.key === s.key))];
        for (const r of all) {
          const on = rows.some((x) => x.key === r.key);
          const cb = h("input", { type: "checkbox", checked: on, "aria-label": "放進圖例" });
          cb.onchange = () => change((sp) => { sp.legend.rows = cb.checked ? [...(sp.legend.rows ?? []), { key: r.key, label: r.label }] : (sp.legend.rows ?? []).filter((x) => x.key !== r.key); });
          const lab = h("input", { type: "text", class: "set-input", value: r.label ?? "", disabled: !on, "aria-label": "文字" });
          lab.oninput = () => change((sp) => { const x = sp.legend.rows.find((y) => y.key === r.key); if (x) x.label = lab.value; }, `leg-${r.key}`);
          b.body.append(h("label", { class: "lf-legrow" }, cb, markSvg(r.key), lab));
        }
      }
      return b;
    }

    // ── 助手 ──
    function buildAssist() {
      assist.innerHTML = "";
      const ta = h("textarea", { class: "set-input lf-ta", rows: 3, placeholder: "例：block 2 改成直的、legend 放下面", "aria-label": "要怎麼改" });
      const out = h("div", { class: "lf-alts" });
      const go = h("button", { type: "button", class: "btn small primary" }, "照這句改");
      const alts = h("button", { type: "button", class: "btn small" }, "幾種版面");
      const busy = (on) => { go.disabled = alts.disabled = on; if (on) { out.innerHTML = ""; out.append(h("p", { class: "lf-wait" }, "想一下")); } };
      go.onclick = async () => {
        if (!ta.value.trim() || !S.spec.dsl.trim()) return;
        busy(true);
        try {
          const r = await post(api, "/assist", { mode: "adjust", spec: S.spec, request: ta.value, doc: opts.doc ?? "" });
          out.innerHTML = "";
          if (r.note) out.append(h("p", { class: "lf-note" }, r.note));
          if (r.question) out.append(h("p", { class: "lf-q" }, r.question));
          const keep = S.spec.discussion;
          change((sp) => { for (const k of Object.keys(sp)) delete sp[k]; Object.assign(sp, r.spec, { discussion: keep }); });
          out.append(h("ol", { class: "lf-changes" }, ...(r.changes ?? []).map((c) => h("li", {}, h("b", {}, c.path), " ", h("span", {}, `${JSON.stringify(c.before) ?? "—"} → ${JSON.stringify(c.after)}`), c.why ? h("i", {}, c.why) : null))));
        } catch (e) { out.innerHTML = ""; out.append(h("p", { class: "lf-note lf-err" }, e.message)); }
        busy(false);
      };
      alts.onclick = async () => {
        if (!S.spec.dsl.trim()) return;
        busy(true);
        try {
          const r = await post(api, "/assist", { mode: "alternatives", spec: S.spec, request: ta.value, doc: opts.doc ?? "" });
          out.innerHTML = "";
          if (r.note) out.append(h("p", { class: "lf-note" }, r.note));
          for (const a of r.alternatives ?? []) {
            const pic = h("div", { class: "lf-pic" });
            pic.innerHTML = a.svg;
            const use = h("button", { type: "button", class: "btn small" }, "套用");
            use.onclick = () => change((sp) => { sp.layout = a.spec.layout ?? ""; sp.dirs = a.spec.dirs ?? {}; sp.legend = a.spec.legend ?? sp.legend; });
            out.append(h("div", { class: "lf-alt" }, pic, h("p", {}, a.why), use));
          }
        } catch (e) { out.innerHTML = ""; out.append(h("p", { class: "lf-note lf-err" }, e.message)); }
        busy(false);
      };
      assist.append(ta, h("div", { class: "lf-row" }, go, alts), out);
    }

    refresh(true);
  }
})();
