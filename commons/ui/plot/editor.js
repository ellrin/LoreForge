// LorePlot.edit(): the figure editor, a full-screen dialog over the host page.
//
//   const r = await LorePlot.edit({
//     spec,            // a spec to re-open (null: start from data)
//     title,           // shown in the bar ("新圖", the file name)
//     target,          // "paper" | "slide" | null (standalone): the default style and the 完成 label
//     macros,          // KaTeX macros (the document's notation registry)
//     api,             // "/plot/api" (null: demo, no server — CSV/TSV/JSON in the page, SVG export only)
//     styles,          // style list (default: GET <api>/styles, or styles/*.json next to this file)
//     save,            // async ({ spec, svg, w, h }) → true | message; 完成 calls it (null: no 完成)
//   });                // → { spec, svg } when saved, null when closed
//
// Editor chrome uses the app theme (theme.css tokens, base.css widgets); the figure never does.
// Needs plot.js loaded first; uses LoreShell (shell.js) for toast/menus when the page has it.
(() => {
  "use strict";
  const LP = window.LorePlot;
  if (!LP) throw new Error("plot.js must load before editor.js");
  const $ = (s, r = document) => r.querySelector(s);
  const clone = (o) => (o == null ? o : JSON.parse(JSON.stringify(o)));
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  function h(tag, attrs, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs ?? {})) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k.startsWith("on")) e[k] = v;
      else if (k === "text") e.textContent = v;
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
  function menu(anchor, items) {
    if (window.LoreShell?.menu) return window.LoreShell.menu(anchor, items);
    const m = h("div", { class: "menu" });
    for (const it of items) {
      if (!it || it.sep) { m.append(h("div", { class: "menu-sep" })); continue; }
      const b = h("button", { type: "button" }, it.label);
      b.onclick = () => { m.remove(); it.run?.(); };
      m.append(b);
    }
    const r = anchor.getBoundingClientRect();
    Object.assign(m.style, { left: `${r.left}px`, top: `${r.bottom + 4}px` });
    (anchor.closest("dialog") ?? document.body).append(m);
    setTimeout(() => document.addEventListener("click", () => m.remove(), { once: true }));
  }

  // ── table text → { columns, rows } (pasted cells, CSV, TSV, JSON) ─────
  function parseDelimited(text) {
    const lines = text.replace(/\r\n?/g, "\n").split("\n").filter((l) => l.trim() !== "");
    if (!lines.length) return null;
    const sep = lines[0].includes("\t") ? "\t" : lines[0].includes(";") && !lines[0].includes(",") ? ";" : ",";
    const split = (l) => {
      const out = [];
      let cur = "", q = false;
      for (let i = 0; i < l.length; i++) {
        const c = l[i];
        if (q) { if (c === '"' && l[i + 1] === '"') { cur += '"'; i++; } else if (c === '"') q = false; else cur += c; }
        else if (c === '"') q = true;
        else if (c === sep) { out.push(cur); cur = ""; }
        else cur += c;
      }
      out.push(cur);
      return out.map((s) => s.trim());
    };
    const rows = lines.map(split);
    const columns = rows[0].map((c, i) => c || `col${i + 1}`);
    return { columns, rows: rows.slice(1).map((r) => columns.map((_, i) => cell(r[i]))) };
  }
  function cell(s) {
    if (s == null) return null;
    const t = String(s).trim();
    if (t === "") return null;
    const n = Number(t.replace(/[−–]/g, "-"));
    return Number.isFinite(n) && /^[\s+\-−–]?[\d.]/.test(t) ? n : t;
  }
  /** "0.81 ± 0.02" columns → value + value_sd columns. */
  function splitPm(t) {
    const pm = t.columns.map((_, i) => t.rows.some((r) => /±|\\pm/.test(String(r[i] ?? ""))));
    if (!pm.some(Boolean)) return t;
    const columns = [], rows = t.rows.map(() => []);
    t.columns.forEach((c, i) => {
      columns.push(c);
      if (pm[i]) columns.push(`${c}_sd`);
      t.rows.forEach((r, k) => {
        if (!pm[i]) { rows[k].push(r[i]); return; }
        const m = String(r[i] ?? "").split(/±|\\pm/);
        rows[k].push(cell(m[0]?.replace(/[$\\{}]|textbf|mathbf/g, "")), cell(m[1]?.replace(/[$\\{}]|textbf|mathbf/g, "")));
      });
    });
    return { columns, rows, name: t.name };
  }
  function parseText(text, name = "") {
    const s = text.trim();
    if (s.startsWith("[") || s.startsWith("{")) {
      try {
        const j = JSON.parse(s);
        if (Array.isArray(j) && j.length && typeof j[0] === "object" && !Array.isArray(j[0])) {
          const columns = [...new Set(j.flatMap((o) => Object.keys(o)))];
          return { name, columns, rows: j.map((o) => columns.map((c) => cell(o[c]))) };
        }
        if (j.columns && j.rows) return { name, columns: j.columns, rows: j.rows };
      } catch {}
    }
    const t = parseDelimited(s);
    return t ? splitPm({ ...t, name }) : null;
  }
  LP.parseText = parseText;

  // ── the editor ──────────────────────────────────────────────────────
  LP.edit = function (opts = {}) {
    return new Promise((resolve) => openEditor(opts, resolve));
  };

  async function loadStyles(api) {
    if (api) {
      try {
        const r = await fetch(`${api}/styles`);
        if (r.ok) return await r.json();
      } catch {}
    }
    const base = LP.base + "styles/";
    const ids = await (await fetch(base + "index.json")).json();
    return { styles: await Promise.all(ids.map(async (i) => (await fetch(`${base}${i}.json`)).json())), default: "clean" };
  }

  const PART_BLOCK = { series: "series", group: "series", xaxis: "axes", yaxis: "axes", legend: "legend", title: "panel", panellabel: "layout", note: "notes", colorbar: "opts", cells: "opts", inset: "inset", grid: "style", axes: "axes", figure: "layout" };
  const MARKERS = [["o", "●"], ["s", "■"], ["^", "▲"], ["v", "▼"], ["D", "◆"], ["x", "✕"], ["+", "+"], ["none", "無"]];
  const DASHES = [["solid", "實線"], ["dash", "虛線"], ["dot", "點線"], ["dashdot", "點虛線"]];
  const LEG_POS = [["best", "自動"], ["upper right", "右上"], ["upper left", "左上"], ["lower right", "右下"], ["lower left", "左下"], ["upper center", "上中"], ["lower center", "下中"], ["center right", "右中"], ["outside right", "圖外右側"], ["below", "圖下方"], ["above", "圖上方"]];
  const TYPE_ICON = { line: "chart", bar: "chart", scatter: "chart", box: "chart", violin: "chart", hist: "chart", heatmap: "grid", roc: "chart", pr: "chart", bland: "chart", km: "chart", radar: "chart" };

  async function openEditor(opts, resolve) {
    const api = opts.api === undefined ? (location.protocol.startsWith("http") && !/\/commons\/ui\//.test(location.pathname) ? "/plot/api" : null) : opts.api;
    const S = { spec: null, pi: 0, hist: [], fut: [], zoom: "fit", styleList: [], defStyle: "clean", last: null, open: { data: true, type: true } };
    const dlg = h("dialog", { class: "lp-editor", "aria-label": "繪圖" });
    document.body.append(dlg);

    // top bar
    const name = h("span", { class: "lp-name" }, opts.title || "新圖");
    const undoB = iconBtn("undo", "復原 ⌘Z", () => undo());
    const redoB = iconBtn("redo", "重做 ⇧⌘Z", () => redo());
    const styleSel = h("select", { class: "set-input lp-style", "aria-label": "樣式" });
    const aiB = h("button", { type: "button", class: "ghost", "aria-label": "自動產生" }, ic("spark"), h("span", { class: "lp-lbl" }, "自動"));
    const expB = iconBtn("download", "匯出", (e) => exportMenu(e.currentTarget));
    const cancelB = h("button", { type: "button", class: "ghost" }, "取消");
    const doneB = h("button", { type: "button", class: "primary small" }, opts.target === "slide" ? "插入投影片" : opts.target === "paper" ? "存入論文" : "完成");
    const top = h("header", { class: "top lp-top" }, h("span", { class: "lp-brand" }, ic("chart"), "繪圖"), name, h("span", { class: "lp-sep" }), undoB, redoB, styleSel, aiB, h("span", { class: "lp-grow" }), expB, cancelB, opts.save ? doneB : null);
    const left = h("aside", { class: "lp-left", "aria-label": "資料與種類" });
    const stage = h("main", { class: "lp-stage" });
    const sheet = h("div", { class: "lp-sheet" });
    stage.append(sheet);
    const right = h("aside", { class: "lp-right", "aria-label": "屬性" });
    const zoomSel = h("select", { class: "lp-zoom", "aria-label": "縮放" }, ...[["fit", "符合視窗"], ["1", "實際大小"], ["1.5", "150%"], ["2", "200%"], ["3", "300%"]].map(([v, t]) => h("option", { value: v }, t)));
    const info = h("span", { class: "lp-info" });
    const warn = h("span", { class: "lp-warn" });
    const status = h("footer", { class: "statusbar lp-status" }, info, warn, h("span", { class: "grow" }), zoomSel);
    dlg.append(top, h("div", { class: "lp-body" }, left, h("div", { class: "lp-mid" }, stage, status), right));
    dlg.showModal();
    if (typeof window.paintIcons === "function") window.paintIcons(dlg);

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
    dlg.addEventListener("cancel", (e) => { e.preventDefault(); cancelB.click(); });
    async function confirmLose() {
      if (window.LoreShell?.ask) return (await window.LoreShell.ask(cancelB, ["放棄這次的修改？"], [{ label: "放棄", value: true }, { label: "繼續編輯", value: null, primary: true }])) === true;
      return confirm("放棄這次的修改？");
    }
    const onKey = (e) => {
      if (!dlg.open) return;
      const inField = e.target.matches?.("input, textarea, select, [contenteditable]");
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !inField) { e.preventDefault(); e.shiftKey ? redo() : undo(); }
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && opts.save) { e.preventDefault(); doneB.click(); }
    };
    document.addEventListener("keydown", onKey, true);

    // styles
    try {
      const st = opts.styles ? { styles: opts.styles, default: opts.defaultStyle } : await loadStyles(api);
      S.styleList = st.styles ?? [];
      S.defStyle = (opts.target === "slide" ? "slide" : null) ?? st.default ?? "clean";
      if (opts.target === "slide" && !S.styleList.some((s) => s.id === "slide")) S.defStyle = st.default ?? "clean";
      if (opts.target !== "slide") S.defStyle = st.default ?? "clean";
    } catch (e) {
      toast("讀不到樣式：" + e.message, { err: true });
    }
    for (const s of S.styleList) styleSel.append(h("option", { value: s.id }, s.label ?? s.id));
    styleSel.onchange = () => change((sp) => { sp.style = styleSel.value; sp.size = null; }, "樣式");

    S.spec = opts.spec ? clone(opts.spec) : null;
    if (S.spec && !S.spec.panels?.length) S.spec = null;

    // ── history ──
    let typing = null;
    function change(fn, label, coalesce) {
      const before = JSON.stringify(S.spec);
      fn(S.spec);
      if (JSON.stringify(S.spec) === before) return;
      if (!(coalesce && typing === coalesce)) { S.hist.push(before); if (S.hist.length > 200) S.hist.shift(); }
      typing = coalesce ?? null;
      S.fut = [];
      refresh(label === "structure");
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

    // ── render ──
    let tok = 0, timer = null;
    function refresh(rebuild = true) {
      undoB.disabled = !S.hist.length;
      redoB.disabled = !S.fut.length;
      if (S.spec) styleSel.value = S.spec.style ?? S.defStyle;
      if (rebuild) { buildLeft(); buildRight(); }
      clearTimeout(timer);
      timer = setTimeout(draw, 60);
    }
    const style = () => LP.resolveStyle(S.spec?.style ?? S.defStyle, S.styleList, S.spec?.set);
    async function draw() {
      if (!S.spec) { sheet.innerHTML = ""; sheet.append(h("div", { class: "lp-empty" }, "左邊匯入或貼上資料")); sheet.style.cssText = ""; return; }
      const my = ++tok;
      let r;
      try {
        r = await LP.render(S.spec, { styles: S.styleList, macros: opts.macros });
      } catch (e) {
        warn.textContent = e.message;
        return;
      }
      if (my !== tok) return;
      S.last = r;
      sheet.innerHTML = r.svg;
      const svg = sheet.firstElementChild;
      const pxmm = 96 / 25.4;
      let z = S.zoom === "fit" ? Math.min((stage.clientWidth - 48) / (r.w * pxmm), (stage.clientHeight - 48) / (r.h * pxmm), 4) : +S.zoom;
      if (!(z > 0)) z = 1;
      svg.style.width = `${r.w * pxmm * z}px`;
      svg.style.height = `${r.h * pxmm * z}px`;
      info.textContent = `${round(r.w)} × ${round(r.h)} mm · ${Math.round(z * 100)}%`;
      warn.textContent = r.warnings.join("　");
      mark();
    }
    const round = (v) => Math.round(v * 10) / 10;
    zoomSel.onchange = () => { S.zoom = zoomSel.value; draw(); };
    new ResizeObserver(() => { if (S.zoom === "fit") { clearTimeout(timer); timer = setTimeout(draw, 80); } }).observe(stage);

    // click on the figure → its block
    sheet.addEventListener("click", (e) => {
      const partEl = e.target.closest?.("[data-part]");
      const panelEl = e.target.closest?.('[data-part^="panel:"]');
      if (panelEl) {
        const pi = +panelEl.dataset.part.split(":")[1];
        if (pi !== S.pi) { S.pi = pi; buildLeft(); buildRight(); mark(); }
      }
      if (!partEl) return;
      const [kind, a, b] = partEl.dataset.part.split(":");
      const key = PART_BLOCK[kind];
      if (!key) return;
      let target = right.querySelector(`[data-block="${key}"]`);
      if ((kind === "series" || kind === "group") && right.querySelector(`[data-series="${kind === "group" ? b ?? a : b ?? a}"]`)) target = right.querySelector(`[data-series="${b ?? a}"]`) ?? target;
      if (kind === "note") target = right.querySelector(`[data-note="${b}"]`) ?? target;
      const blk = target?.closest(".block");
      blk?.setOpen?.(true);
      target?.scrollIntoView({ block: "nearest", behavior: "smooth" });
      target?.classList.add("lp-flash");
      setTimeout(() => target?.classList.remove("lp-flash"), 900);
    });
    function mark() {
      for (const g of sheet.querySelectorAll('[data-part^="panel:"]')) g.classList.toggle("lp-cur", (S.spec?.panels?.length ?? 0) > 1 && +g.dataset.part.split(":")[1] === S.pi);
    }

    // ── left: data and chart type ──
    function buildLeft() {
      left.textContent = "";
      const bData = block(null, "資料", "data", S.spec ? Object.keys(S.spec.data ?? {}).length : 0);
      const fileIn = h("input", { type: "file", hidden: true, accept: ".csv,.tsv,.txt,.json,.xlsx,.tex,text/csv,application/json" });
      fileIn.onchange = () => { if (fileIn.files[0]) importFile(fileIn.files[0]); fileIn.value = ""; };
      const row = h("div", { class: "lp-row" },
        h("button", { type: "button", class: "ghost", onclick: () => fileIn.click() }, ic("upload"), "匯入檔案"),
        h("button", { type: "button", class: "ghost", onclick: pasteDialog }, ic("copy"), "貼上表格"),
        fileIn);
      bData.body.append(row);
      const tables = Object.entries(S.spec?.data ?? {});
      const p = panel();
      for (const [id, t] of tables) {
        const head = h("div", { class: "lp-thead" },
          h("span", { class: "lp-tname" }, t.name || id),
          h("span", { class: "lp-dim" }, `${t.rows?.length ?? 0}×${t.columns?.length ?? 0}`),
          p && p.data !== id ? h("button", { type: "button", class: "ghost lp-use", onclick: () => change((sp) => { sp.panels[S.pi].data = id; }, "structure") }, "用在這格") : null,
          iconBtn("trash", "刪除這份資料", () => {
            if (S.spec.panels.some((q) => q.data === id)) return toast("還有格在用這份資料");
            change((sp) => { delete sp.data[id]; }, "structure");
          }, "ico tiny"));
        bData.body.append(head, dataGrid(id, t));
      }
      left.append(bData);
      if (S.spec) {
        const bType = block(null, "種類", "type");
        const grid = h("div", { class: "lp-types" });
        for (const t of LP.TYPES) {
          const b = h("button", { type: "button", "aria-pressed": String(p?.type === t.id) }, h("span", { class: "lp-tico" }, ic(TYPE_ICON[t.id] ?? "chart")), h("span", {}, t.label));
          b.onclick = () => setType(t.id);
          grid.append(b);
        }
        bType.body.append(grid);
        left.append(bType);
      }
      if (typeof window.paintIcons === "function") window.paintIcons(left);
    }
    function dataGrid(id, t) {
      const wrap = h("div", { class: "lp-grid" });
      const tbl = h("table", {});
      const max = 200;
      const tr0 = h("tr", {});
      t.columns.forEach((c, ci) => {
        const th = h("th", { contenteditable: "plaintext-only", spellcheck: "false" }, c);
        th.onblur = () => {
          const v = th.textContent.trim();
          if (!v || v === c) return;
          change((sp) => renameColumn(sp, id, c, v), "structure");
        };
        tr0.append(th);
      });
      tbl.append(h("thead", {}, tr0));
      const tb = h("tbody", {});
      t.rows.slice(0, max).forEach((r, ri) => {
        const tr = h("tr", {});
        t.columns.forEach((_, ci) => {
          const td = h("td", { contenteditable: "plaintext-only", spellcheck: "false" }, r[ci] == null ? "" : String(r[ci]));
          td.onblur = () => {
            const v = cell(td.textContent);
            if (v === r[ci]) return;
            change((sp) => { sp.data[id].rows[ri][ci] = v; }, "data");
          };
          td.onkeydown = (e) => { if (e.key === "Enter") { e.preventDefault(); td.blur(); } };
          tr.append(td);
        });
        tb.append(tr);
      });
      tbl.append(tb);
      wrap.append(tbl);
      if (t.rows.length > max) wrap.append(h("div", { class: "lp-dim" }, `只顯示前 ${max} 列`));
      return wrap;
    }
    function renameColumn(sp, id, from, to) {
      const t = sp.data[id];
      t.columns = t.columns.map((c) => (c === from ? to : c));
      for (const p of sp.panels.filter((q) => q.data === id)) {
        for (const k of ["x", "y", "group", "size", "event"]) if (p[k] === from) p[k] = to;
        for (const s of p.series ?? []) for (const k of ["y", "err", "x", "y2", "score", "truth"]) if (s[k] === from) s[k] = to;
      }
    }
    async function importFile(f) {
      try {
        let tables = null;
        const lower = f.name.toLowerCase();
        if (api) {
          const r = await fetch(`${api}/import?name=${encodeURIComponent(f.name)}`, { method: "POST", body: f });
          const j = await r.json().catch(() => ({}));
          if (!r.ok) throw new Error(j.error ?? r.statusText);
          tables = j.tables;
        } else if (/\.(xlsx|tex)$/.test(lower)) throw new Error("示範頁不能讀 XLSX／LaTeX（需要伺服器）");
        else tables = [parseText(await f.text(), f.name)].filter(Boolean);
        if (!tables?.length) throw new Error("檔案裡沒有表格");
        addTables(tables);
      } catch (e) {
        toast(e.message, { err: true, ms: 6000 });
      }
    }
    function pasteDialog() {
      const ta = h("textarea", { class: "set-input lp-paste", placeholder: "從 Excel、Google 試算表、CSV、LaTeX tabular 或 JSON 貼上；第一列是欄名", spellcheck: "false" });
      const ok = h("button", { type: "button", class: "primary small" }, "加入");
      const no = h("button", { type: "button", class: "ghost" }, "取消");
      const d = h("dialog", { class: "dialog small lp-pastedlg" }, h("div", { class: "dlg-head" }, h("h2", {}, "貼上表格")), h("div", { class: "dlg-body" }, ta), h("div", { class: "dlg-foot" }, h("span", { class: "grow" }), no, ok));
      dlg.append(d);
      d.showModal();
      ta.focus();
      no.onclick = () => d.remove();
      ok.onclick = async () => {
        const text = ta.value;
        let tables = null;
        if (/\\begin\{tabular|&.*\\\\/.test(text) && api) {
          const r = await fetch(`${api}/import?name=pasted.tex`, { method: "POST", body: text });
          tables = (await r.json().catch(() => ({}))).tables;
        } else tables = [parseText(text, "貼上")].filter(Boolean);
        if (!tables?.length) return toast("看不出表格（第一列要是欄名）", { err: true });
        d.remove();
        addTables(tables);
      };
    }
    function addTables(tables) {
      if (!S.spec) {
        const t = tables[0];
        const type = guessType(t);
        S.spec = LP.starter(type, clean(t), S.defStyle);
        tables.slice(1).forEach((tt, k) => (S.spec.data[`t${k + 2}`] = clean(tt)));
        S.hist.push("null");
        S.pi = 0;
        refresh(true);
        return;
      }
      change((sp) => {
        for (const t of tables) {
          let k = 1;
          while (sp.data[`t${k}`]) k++;
          sp.data[`t${k}`] = clean(t);
          if (sp.panels[S.pi] && tables.length === 1) sp.panels[S.pi].data = `t${k}`;
        }
      }, "structure");
    }
    const clean = (t) => ({ name: t.name ?? "", columns: [...t.columns], rows: t.rows.map((r) => t.columns.map((_, i) => (r[i] === undefined ? null : r[i]))) });
    function guessType(t) {
      const firstNumeric = t.rows.every((r) => r[0] == null || isNum(r[0]));
      return firstNumeric ? "line" : "bar";
    }
    function setType(type) {
      change((sp) => {
        const p = sp.panels[S.pi];
        const t = sp.data[p.data] ?? Object.values(sp.data)[0];
        const fresh = LP.starter(type, t, sp.style).panels[0];
        const keep = { title: p.title, legend: p.legend, notes: p.notes, data: p.data, span: p.span };
        // keep the series the author already named / coloured when the new type uses columns the same way
        if (["line", "bar", "box", "violin", "hist", "radar"].includes(type) && ["line", "bar", "box", "violin", "hist", "radar"].includes(p.type) && p.series?.length) fresh.series = p.series;
        if (["line", "bar", "radar"].includes(type) && ["line", "bar", "radar"].includes(p.type)) { fresh.xaxis = p.xaxis; fresh.yaxis = p.yaxis; fresh.x = p.x; }
        sp.panels[S.pi] = { ...fresh, ...Object.fromEntries(Object.entries(keep).filter(([, v]) => v !== undefined)), type };
      }, "structure");
    }

    // ── right: properties ──
    const panel = () => S.spec?.panels?.[S.pi];
    function block(n, title, key, count) {
      const b = window.LoreShell?.block ? window.LoreShell.block({ n, title, count, key: `plot.${key}`, open: S.open[key] ?? !["inset", "notes", "opts"].includes(key) }) : fallbackBlock(n, title);
      b.dataset.block = key;
      return b;
    }
    function fallbackBlock(n, title) {
      const b = h("section", { class: "block", "data-open": "true" }, h("div", { class: "block-head" }, n ? h("span", { class: "n" }, n) : null, h("h3", {}, title)));
      b.body = h("div", { class: "block-body" });
      b.append(b.body);
      b.setOpen = () => {};
      return b;
    }
    const row = (label, ...ctl) => h("label", { class: "lp-f" }, h("span", { class: "k" }, label), ...ctl);
    function text(get, set, ph, co) {
      const i = h("input", { class: "set-input", value: get() ?? "", placeholder: ph ?? "", spellcheck: "false" });
      i.oninput = () => change(() => set(i.value), "edit", co ?? i);
      return i;
    }
    function number(get, set, o = {}) {
      const i = h("input", { class: "set-input lp-num", type: "number", step: o.step ?? "any", value: get() ?? "", placeholder: o.ph ?? "自動" });
      if (o.min != null) i.min = o.min;
      i.oninput = () => change(() => set(i.value === "" ? null : +i.value), "edit", i);
      return i;
    }
    function select(get, set, options, o = {}) {
      const s = h("select", { class: "set-input" }, ...options.map(([v, t]) => h("option", { value: v ?? "" }, t)));
      s.value = get() ?? "";
      s.onchange = () => change(() => set(s.value === "" ? null : s.value), o.structure ? "structure" : "edit");
      return s;
    }
    function check(get, set, label) {
      const c = h("input", { type: "checkbox" });
      c.checked = !!get();
      c.onchange = () => change(() => set(c.checked), "structure");
      return h("label", { class: "lp-check" }, c, label);
    }
    function seg(get, set, options) {
      const g = h("div", { class: "seg" });
      for (const [v, t] of options) {
        const b = h("button", { type: "button", "aria-pressed": String((get() ?? options[0][0]) === v) }, t);
        b.onclick = () => change(() => set(v), "structure");
        g.append(b);
      }
      return g;
    }
    function color(get, set, auto) {
      const c = h("input", { type: "color", class: "lp-color", value: get() || auto || "#000000" });
      c.oninput = () => change(() => set(c.value), "edit", c);
      const wrap = h("span", { class: "lp-colorw" }, c);
      if (get()) wrap.append(iconBtn("x", "用樣式的顏色", () => change(() => set(null), "structure"), "ico tiny"));
      return wrap;
    }
    const columnsOf = (p) => (S.spec.data[p?.data] ?? Object.values(S.spec.data)[0])?.columns ?? [];
    const colSel = (p, get, set, none = "—") => select(get, set, [[null, none], ...columnsOf(p).map((c) => [c, c])], { structure: true });

    function buildRight() {
      const keepScroll = right.scrollTop;
      for (const b of right.querySelectorAll(".block")) S.open[b.dataset.block] = b.dataset.open === "true";
      right.textContent = "";
      if (!S.spec) {
        right.append(h("p", { class: "lp-dim lp-pad" }, "先放資料"));
        return;
      }
      const sp = S.spec, p = panel();
      const st = style();
      // 01 版面
      const bL = block("01", "版面", "layout");
      const widths = st.size.widths ?? [];
      const wNames = ["單欄", "1.5 欄", "雙欄"];
      const sizeRow = h("div", { class: "lp-row" }, ...widths.map((w, k) => {
        const on = Math.abs((sp.size?.w ?? st.size.w) - w) < 0.05;
        const b = h("button", { type: "button", class: "ghost lp-chipbtn", "aria-pressed": String(on) }, `${wNames[k] ?? ""} ${w}`);
        b.onclick = () => change((s) => { s.size = { w, h: s.size?.h ?? st.size.h }; }, "structure");
        return b;
      }));
      bL.body.append(sizeRow,
        row("寬 mm", number(() => sp.size?.w ?? st.size.w, (v) => { sp.size = { w: v ?? st.size.w, h: sp.size?.h ?? st.size.h }; }, { step: 1 })),
        row("高 mm", number(() => sp.size?.h ?? st.size.h, (v) => { sp.size = { w: sp.size?.w ?? st.size.w, h: v ?? st.size.h }; }, { step: 1 })),
        row("格", number(() => sp.grid?.cols ?? 1, (v) => { sp.grid = { ...(sp.grid ?? {}), cols: Math.max(1, Math.round(v ?? 1)) }; }, { step: 1, min: 1 }), h("span", { class: "lp-x" }, "欄 ×"),
          number(() => sp.grid?.rows ?? 1, (v) => { sp.grid = { ...(sp.grid ?? {}), rows: Math.max(1, Math.round(v ?? 1)) }; }, { step: 1, min: 1 }), h("span", { class: "lp-x" }, "列")),
        row("間距 mm", number(() => sp.grid?.gap ?? 5, (v) => { sp.grid = { ...(sp.grid ?? {}), gap: v ?? 5 }; }, { step: 0.5 })),
        row("格標", seg(() => sp.grid?.labels ?? "none", (v) => { sp.grid = { ...(sp.grid ?? {}), labels: v }; }, [["none", "無"], ["a", "a"], ["A", "A"], ["(a)", "(a)"]])));
      right.append(bL);

      // 02 格
      const bP = block("02", "格", "panel", sp.panels.length > 1 ? `${S.pi + 1}/${sp.panels.length}` : "");
      const tabs = h("div", { class: "lp-row lp-ptabs" }, ...sp.panels.map((q, k) => {
        const b = h("button", { type: "button", class: "ghost lp-chipbtn", "aria-pressed": String(k === S.pi) }, String.fromCharCode(97 + k));
        b.onclick = () => { S.pi = k; buildLeft(); buildRight(); mark(); };
        return b;
      }),
      iconBtn("plus", "加一格", () => change((s) => {
        const cp = clone(s.panels[S.pi]);
        cp.title = "";
        s.panels.splice(S.pi + 1, 0, cp);
        const n = s.panels.length, cols = s.grid?.cols ?? 1, rows = s.grid?.rows ?? 1;
        if (n > cols * rows) s.grid = { ...(s.grid ?? {}), cols: cols * rows === cols ? cols + 1 : cols, rows: cols * rows === cols ? rows : rows + 1, labels: s.grid?.labels && s.grid.labels !== "none" ? s.grid.labels : "a" };
        if (s.size?.w || true) s.size = { w: (s.size?.w ?? st.size.w) < (widths.at(-1) ?? 175) && n > 1 ? widths.at(-1) ?? 175 : s.size?.w ?? st.size.w, h: s.size?.h ?? st.size.h };
        S.pi += 1;
      }, "structure"), "ico small"),
      sp.panels.length > 1 ? iconBtn("trash", "刪除這格", () => change((s) => { s.panels.splice(S.pi, 1); S.pi = Math.max(0, S.pi - 1); }, "structure"), "ico small") : null);
      bP.body.append(tabs, row("標題", text(() => p.title, (v) => (p.title = v), "可寫 $…$")));
      if ((sp.grid?.cols ?? 1) > 1) bP.body.append(row("佔幾欄", number(() => p.span ?? 1, (v) => (p.span = v && v > 1 ? Math.round(v) : undefined), { step: 1, min: 1 })));
      // columns used by this type
      const colsFor = {
        line: [["x", "x 欄"]], bar: [["x", "類別欄"]], radar: [["x", "軸名稱欄"]], heatmap: [["x", "列名欄"]],
        scatter: [["x", "x 欄"], ["y", "y 欄"], ["group", "分組欄"], ["size", "大小欄"]],
        box: [["group", "分組欄（長格式）"], ["y", "數值欄（長格式）"]], violin: [["group", "分組欄（長格式）"], ["y", "數值欄（長格式）"]],
        km: [["x", "時間欄"], ["event", "事件欄（1／0）"], ["group", "分組欄"]],
      }[p.type] ?? [];
      for (const [k, label] of colsFor) bP.body.append(row(label, colSel(p, () => p[k], (v) => { p[k] = v; })));
      right.append(bP);

      // 03 系列
      if (!["heatmap", "km", "scatter"].includes(p.type) || p.type === "scatter") right.append(seriesBlock(p, st));

      // 04 軸
      if (p.type !== "radar") {
        const bA = block("04", "軸", "axes");
        for (const [k, nm] of [["xaxis", "x"], ["yaxis", "y"]]) {
          const ax = (p[k] ??= {});
          const g = h("div", { class: "lp-group" }, h("div", { class: "lp-gh" }, `${nm} 軸`));
          g.append(row("標籤", text(() => ax.label, (v) => (ax.label = v), "可寫 $\\alpha$")),
            row("單位", text(() => ax.unit, (v) => (ax.unit = v || undefined), "mm")),
            row("範圍", number(() => ax.min, (v) => (ax.min = v ?? undefined)), h("span", { class: "lp-x" }, "到"), number(() => ax.max, (v) => (ax.max = v ?? undefined))),
            row("刻度", text(() => (ax.ticks ?? []).join(", "), (v) => (ax.ticks = v.trim() ? v.split(/[,\s]+/).map(Number).filter(Number.isFinite) : undefined), "自動，或 0, 0.5, 1")),
            row("格式", select(() => ax.fmt, (v) => (ax.fmt = v ?? undefined), [[null, "自動"], [".1f", "0.1"], [".2f", "0.12"], [".3f", "0.123"], ["%", "百分比"], [".1%", "12.3%"], ["d", "整數"], ["e", "10ⁿ"]])),
            h("div", { class: "lp-row" },
              check(() => ax.log, (v) => (ax.log = v || undefined), "對數"),
              check(() => ax.reverse, (v) => (ax.reverse = v || undefined), "反向"),
              check(() => ax.grid, (v) => (ax.grid = v), "格線"),
              check(() => ax.hidden, (v) => (ax.hidden = v || undefined), "隱藏")));
          if (k === "xaxis") g.append(row("字旋轉", select(() => String(ax.rotate ?? 0), (v) => (ax.rotate = +v || undefined), [["0", "0°"], ["30", "30°"], ["45", "45°"], ["90", "90°"]])));
          bA.body.append(g);
        }
        right.append(bA);
      }

      // 05 圖例
      const bG = block("05", "圖例", "legend");
      const lg = (p.legend ??= {});
      bG.body.append(
        h("div", { class: "lp-row" }, check(() => lg.show !== false, (v) => (lg.show = v ? undefined : false), "顯示"), check(() => lg.frame ?? st.legend.frame, (v) => (lg.frame = v), "外框")),
        row("位置", select(() => lg.pos ?? st.legend.pos ?? "best", (v) => (lg.pos = v), LEG_POS)),
        row("欄數", number(() => lg.cols, (v) => (lg.cols = v ? Math.max(1, Math.round(v)) : undefined), { step: 1, min: 1 })),
        row("字級 pt", number(() => lg.size, (v) => (lg.size = v ?? undefined), { step: 0.5 })),
        row("標題", text(() => lg.title, (v) => (lg.title = v || undefined))));
      right.append(bG);

      // 06 選項 (type-specific)
      const ob = optsBlock(p);
      if (ob) right.append(ob);

      // 07 註記
      right.append(notesBlock(p));

      // 08 放大小圖
      if (["line", "roc", "pr", "scatter", "km"].includes(p.type)) {
        const bI = block("08", "放大小圖", "inset");
        const on = !!p.inset;
        bI.body.append(h("div", { class: "lp-row" }, check(() => on, (v) => (p.inset = v ? { x: [0, 0.2], y: [0.6, 1], at: [0.45, 0.35, 0.45, 0.45] } : null), "顯示")));
        if (on) {
          const r4 = (arr, k) => number(() => p.inset[arr][k], (v) => { p.inset[arr][k] = v ?? 0; }, { step: 0.01 });
          bI.body.append(row("x 範圍", r4("x", 0), h("span", { class: "lp-x" }, "到"), r4("x", 1)), row("y 範圍", r4("y", 0), h("span", { class: "lp-x" }, "到"), r4("y", 1)),
            row("位置", r4("at", 0), r4("at", 1)), row("大小", r4("at", 2), r4("at", 3)));
        }
        right.append(bI);
      }

      // 09 樣式（這張圖自己的）
      right.append(styleBlock(st));
      // caption (PaperComposer)
      if (opts.target === "paper") {
        const bC = block("10", "圖說", "caption");
        const ta = h("textarea", { class: "set-input lp-cap", rows: 3, placeholder: "插入論文時的 \\caption" }, sp.caption ?? "");
        ta.oninput = () => change(() => (sp.caption = ta.value), "edit", ta);
        bC.body.append(ta);
        right.append(bC);
      }
      if (typeof window.paintIcons === "function") window.paintIcons(right);
      right.scrollTop = keepScroll;
    }

    function seriesBlock(p, st) {
      const b = block("03", p.type === "scatter" ? "群組" : "系列", "series", p.type === "scatter" ? "" : String(p.series?.length ?? 0));
      const cols = columnsOf(p);
      const t = S.spec.data[p.data] ?? Object.values(S.spec.data)[0];
      if (p.type === "scatter") {
        const names = p.group ? [...new Set((t?.rows ?? []).map((r) => String(r[t.columns.indexOf(p.group)] ?? "")))] : [];
        if (!names.length) { b.body.append(h("p", { class: "lp-dim" }, "選了分組欄後可改每組的名稱與顏色")); return b; }
        p.series ??= [];
        names.forEach((g, i) => {
          let s = p.series.find((q) => q.group === g);
          const ensure = () => { s = p.series.find((q) => q.group === g); if (!s) { s = { group: g }; p.series.push(s); } return s; };
          const auto = (st.colors.palette ?? [])[i % (st.colors.palette?.length || 1)];
          const box = h("div", { class: "lp-ser", "data-series": String(i) },
            h("div", { class: "lp-gh" }, g),
            row("名稱", text(() => s?.label, (v) => (ensure().label = v || undefined), g)),
            row("顏色", color(() => s?.color, (v) => (ensure().color = v || undefined), auto), select(() => s?.marker, (v) => (ensure().marker = v ?? undefined), [[null, "●"], ...MARKERS.slice(1, 7)])));
          b.body.append(box);
        });
        return b;
      }
      (p.series ?? []).forEach((s, i) => {
        const sst = { color: s.color };
        const auto = ((p.type === "line" || p.type === "roc" || p.type === "pr" || p.type === "radar") && st.colors.lines?.length ? st.colors.lines : st.colors.palette)[i % (st.colors.palette?.length || 1)];
        const box = h("div", { class: "lp-ser", "data-series": String(i) });
        const head = h("div", { class: "lp-gh" }, h("span", { class: "lp-swatch", style: `background:${s.color || auto}` }), s.label || s.y || s.score || `系列 ${i + 1}`, h("span", { class: "lp-grow" }),
          i > 0 ? iconBtn("up", "往前", () => change(() => { const a = p.series; [a[i - 1], a[i]] = [a[i], a[i - 1]]; }, "structure"), "ico tiny") : null,
          iconBtn(s.hidden ? "eyeOff" : "eye", s.hidden ? "顯示" : "隱藏", () => change(() => (s.hidden = s.hidden ? undefined : true), "structure"), "ico tiny"),
          iconBtn("trash", "刪除系列", () => change(() => p.series.splice(i, 1), "structure"), "ico tiny"));
        box.append(head);
        const fields = {
          roc: [["score", "分數欄"], ["truth", "真值欄（1／0）"], ["x", "或：x 欄"], ["y", "y 欄"]], pr: [["score", "分數欄"], ["truth", "真值欄（1／0）"], ["x", "或：x 欄"], ["y", "y 欄"]],
          bland: [["y", "方法 A"], ["y2", "方法 B"]],
          line: [["y", "y 欄"], ["err", "誤差欄"]], bar: [["y", "數值欄"], ["err", "誤差欄"]],
        }[p.type] ?? [["y", "數值欄"]];
        for (const [k, label] of fields) box.append(row(label, colSel(p, () => s[k], (v) => (s[k] = v ?? undefined))));
        box.append(row("名稱", text(() => s.label, (v) => (s.label = v), "可寫 $…$")));
        box.append(row("顏色", color(() => s.color, (v) => (s.color = v || undefined), auto)));
        if (["line", "roc", "pr", "radar", "km"].includes(p.type)) {
          box.append(row("線", select(() => s.dash, (v) => (s.dash = v ?? undefined), [[null, "樣式"], ...DASHES]), number(() => s.width, (v) => (s.width = v ?? undefined), { step: 0.1, ph: "粗細" })));
          if (p.type !== "roc" && p.type !== "pr") box.append(row("點", select(() => s.marker, (v) => (s.marker = v ?? undefined), [[null, "樣式"], ...MARKERS]), number(() => s.msize, (v) => (s.msize = v ?? undefined), { step: 0.5, ph: "大小" })));
        }
        b.body.append(box);
      });
      if (!["bland"].includes(p.type) || !(p.series?.length)) {
        const used = new Set((p.series ?? []).map((s) => s.y ?? s.score));
        const free = cols.filter((c, k) => k > 0 && !used.has(c));
        const add = h("button", { type: "button", class: "ghost" }, ic("plus"), "加系列");
        add.onclick = () => change(() => {
          p.series ??= [];
          const c = free[0] ?? cols[1] ?? cols[0];
          p.series.push(p.type === "roc" || p.type === "pr" ? { score: c, truth: p.series[0]?.truth, label: c } : { y: c, label: c });
        }, "structure");
        b.body.append(add);
      }
      return b;
    }

    function optsBlock(p) {
      const o = (p.opts ??= {});
      const set = (k) => (v) => (o[k] = v === false || v == null ? undefined : v);
      const get = (k) => () => o[k];
      const f = [];
      switch (p.type) {
        case "line": f.push(row("誤差", seg(() => o.err_style ?? "band", set("err_style"), [["band", "色帶"], ["bars", "誤差線"]])), check(get("step"), set("step"), "階梯")); break;
        case "bar":
          f.push(row("排法", seg(() => o.mode ?? "grouped", set("mode"), [["grouped", "並排"], ["stacked", "堆疊"]])), h("div", { class: "lp-row" }, check(get("horizontal"), set("horizontal"), "橫向"), check(get("values"), set("values"), "標數值")));
          if (o.values) f.push(row("數值格式", text(() => o.fmt, set("fmt"), ".2f、%")));
          break;
        case "scatter": f.push(h("div", { class: "lp-row" }, check(get("ellipse"), set("ellipse"), "95% 橢圓"), check(get("hull"), set("hull"), "外框（凸包）")), row("透明度", number(() => o.alpha, set("alpha"), { step: 0.05 }))); break;
        case "box": case "violin": f.push(h("div", { class: "lp-row" }, check(get("points"), set("points"), "疊上資料點"), check(get("mean"), set("mean"), "平均（▲）"))); break;
        case "hist": f.push(row("組數", number(() => o.bins, set("bins"), { step: 1, min: 2 })), h("div", { class: "lp-row" }, check(get("density"), set("density"), "密度"), check(get("stacked"), set("stacked"), "堆疊"))); break;
        case "heatmap":
          f.push(row("色彩", select(() => o.cmap, set("cmap"), [[null, "樣式"], ...LP.CMAPS.map((c) => [c, c])])), row("正規化", seg(() => o.normalize ?? "none", (v) => (o.normalize = v === "none" ? undefined : v), [["none", "原值"], ["rows", "每列"]])),
            h("div", { class: "lp-row" }, check(() => o.values !== false, (v) => (o.values = v ? undefined : false), "標數值"), check(get("colorbar"), set("colorbar"), "色條"), check(get("top"), set("top"), "欄名在上")), row("數值格式", text(() => o.fmt, set("fmt"), "d、.2f")));
          break;
        case "roc": case "pr":
          f.push(h("div", { class: "lp-row" }, check(() => o.auc !== false, (v) => (o.auc = v ? undefined : false), "圖例寫 AUC"), p.type === "roc" ? check(() => o.diagonal !== false, (v) => (o.diagonal = v ? undefined : false), "對角線") : null),
            row("AUC 名稱", text(() => o.auc_label, set("auc_label"), p.type === "roc" ? "AUC、AUROC" : "AP、AUPRC")));
          break;
        case "bland": f.push(check(() => o.limits !== false, (v) => (o.limits = v ? undefined : false), "標平均差與 ±1.96 SD")); break;
        case "km": f.push(check(() => o.censor !== false, (v) => (o.censor = v ? undefined : false), "設限記號")); break;
        case "radar": f.push(check(() => o.fill !== false, (v) => (o.fill = v ? undefined : false), "填色")); break;
      }
      if (!f.length) return null;
      const b = block("06", "選項", "opts");
      b.body.append(...f);
      return b;
    }

    function notesBlock(p) {
      const b = block("07", "註記", "notes", p.notes?.length ? String(p.notes.length) : "");
      (p.notes ?? []).forEach((n, k) => {
        const box = h("div", { class: "lp-ser", "data-note": String(k) });
        box.append(h("div", { class: "lp-gh" }, { text: "文字", arrow: "箭頭", bracket: "顯著性", hline: "水平線", vline: "垂直線" }[n.type] ?? "文字", h("span", { class: "lp-grow" }),
          iconBtn("trash", "刪除註記", () => change(() => p.notes.splice(k, 1), "structure"), "ico tiny")));
        box.append(row("文字", text(() => n.text, (v) => (n.text = v), n.type === "bracket" ? "*、**、n.s.、$p<0.01$" : "可寫 $…$")));
        const nb = (key, label) => row(label, number(() => n[key], (v) => (n[key] = v ?? undefined), { step: "any" }));
        if (n.type === "hline") box.append(nb("y", "y"));
        else if (n.type === "vline") box.append(nb("x", "x"));
        else if (n.type === "bracket") {
          box.append(row("從", number(() => n.x, (v) => (n.x = v ?? 0), { step: 1, ph: "類別" }), number(() => n.s, (v) => (n.s = v ?? undefined), { step: 1, ph: "系列" })),
            row("到", number(() => n.x2, (v) => (n.x2 = v ?? 0), { step: 1, ph: "類別" }), number(() => n.s2, (v) => (n.s2 = v ?? undefined), { step: 1, ph: "系列" })), nb("y", "高度"));
        } else {
          box.append(row("位置", number(() => n.x, (v) => (n.x = v ?? 0)), number(() => n.y, (v) => (n.y = v ?? 0))));
          if (n.type === "arrow") box.append(row("箭尾", number(() => n.x2, (v) => (n.x2 = v ?? undefined)), number(() => n.y2, (v) => (n.y2 = v ?? undefined))));
          box.append(row("座標", seg(() => n.coords ?? "data", (v) => (n.coords = v === "data" ? undefined : v), [["data", "資料"], ["axes", "比例 0–1"]])));
        }
        b.body.append(box);
      });
      const add = h("button", { type: "button", class: "ghost" }, ic("plus"), "加註記");
      add.onclick = () => menu(add, [
        { label: "文字", run: () => addNote({ type: "text", text: "text", x: 0.5, y: 0.9, coords: "axes" }) },
        { label: "箭頭", run: () => addNote({ type: "arrow", text: "", x: 0.5, y: 0.5, x2: 0.7, y2: 0.7, coords: "axes" }) },
        { label: "顯著性括號", run: () => addNote({ type: "bracket", x: 0, x2: 1, text: "*" }) },
        { label: "水平線", run: () => addNote({ type: "hline", y: 0.5, text: "" }) },
        { label: "垂直線", run: () => addNote({ type: "vline", x: 0.5, text: "" }) },
      ]);
      const addNote = (n) => change(() => { p.notes ??= []; p.notes.push(n); S.open.notes = true; }, "structure");
      b.body.append(add);
      return b;
    }

    function styleBlock(st) {
      const sp = S.spec;
      const set = (k) => (v) => { sp.set ??= {}; if (v == null || v === "") delete sp.set[k]; else sp.set[k] = v; };
      const get = (k) => () => LP.getPath(st, k);
      const b = block("09", "樣式", "style", Object.keys(sp.set ?? {}).length ? `改了 ${Object.keys(sp.set).length} 項` : "");
      const pal = st.colors.palette ?? [];
      const sw = h("div", { class: "lp-row lp-pal" }, ...pal.map((c, k) => {
        const i = h("input", { type: "color", class: "lp-color", value: c, title: `顏色 ${k + 1}` });
        i.oninput = () => change(() => { const a = [...pal]; a[k] = i.value; set("colors.palette")(a); }, "edit", i);
        return i;
      }));
      b.body.append(
        h("div", { class: "lp-dim lp-fontname" }, (st.font.family ?? [])[0] ?? ""),
        row("字級 pt", number(get("font.size"), (v) => { for (const k of ["font.size", "font.label", "font.title"]) set(k)(v); }, { step: 0.5 }), h("span", { class: "lp-x" }, "刻度"), number(get("font.tick"), set("font.tick"), { step: 0.5 })),
        row("圖例 pt", number(get("legend.size"), set("legend.size"), { step: 0.5 }), h("span", { class: "lp-x" }, "格標"), number(get("font.panel"), set("font.panel"), { step: 0.5 })),
        row("線寬 pt", number(get("lines.width"), set("lines.width"), { step: 0.1 }), h("span", { class: "lp-x" }, "軸"), number(get("axes.line"), set("axes.line"), { step: 0.1 })),
        row("點 pt", number(get("lines.marker"), set("lines.marker"), { step: 0.5 }), h("span", { class: "lp-x" }, "散佈"), number(get("scatter.size"), set("scatter.size"), { step: 0.5 })),
        row("格線", seg(() => st.axes.grid, set("axes.grid"), [["none", "無"], ["y", "y"], ["x", "x"], ["both", "全部"]])),
        row("外框", seg(() => st.axes.spines, set("axes.spines"), [["lb", "左下"], ["box", "四邊"], ["none", "無"]])),
        row("刻度", seg(() => st.axes.tick_dir, set("axes.tick_dir"), [["out", "朝外"], ["in", "朝內"]])),
        row("長條寬", number(get("bars.width"), set("bars.width"), { step: 0.05 }), h("span", { class: "lp-x" }, "間隙"), number(get("bars.gap"), set("bars.gap"), { step: 0.01 })),
        row("數學字", seg(() => st.font.math, set("font.math"), [["text", "同內文"], ["katex", "TeX"]])),
        row("背景", seg(() => (st.colors.background === "none" ? "none" : "white"), (v) => set("colors.background")(v === "none" ? "none" : null), [["white", "白"], ["none", "透明"]])),
        h("div", { class: "lp-gh" }, "色盤"), sw);
      if (Object.keys(sp.set ?? {}).length) b.body.append(h("button", { type: "button", class: "ghost", onclick: () => change(() => (sp.set = {}), "structure") }, ic("refresh"), "回到樣式預設"));
      return b;
    }

    // ── AI ──
    aiB.onclick = () => {
      if (!api) return toast("示範頁沒有模型");
      const tables = S.spec ? Object.values(S.spec.data) : [];
      if (!tables.length) return toast("先放資料");
      const ta = h("textarea", { class: "set-input lp-paste", rows: 4, placeholder: "例：比較我們的方法與 4 個 baseline 的 Dice 和 HD95，分組長條加誤差線" });
      const ok = h("button", { type: "button", class: "primary small" }, ic("spark"), "產生");
      const no = h("button", { type: "button", class: "ghost" }, "取消");
      const d = h("dialog", { class: "dialog small" }, h("div", { class: "dlg-head" }, h("h2", {}, "自動產生")), h("div", { class: "dlg-body" }, ta), h("div", { class: "dlg-foot" }, h("span", { class: "grow" }), no, ok));
      dlg.append(d);
      d.showModal();
      if (typeof window.paintIcons === "function") window.paintIcons(d);
      no.onclick = () => d.remove();
      ok.onclick = async () => {
        if (!ta.value.trim()) return;
        ok.disabled = true;
        ok.lastChild.textContent = "產生中…";
        try {
          const r = await fetch(`${api}/propose`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ tables, sentence: ta.value, style: S.spec.style ?? S.defStyle, target: opts.target ?? null }) });
          const j = await r.json().catch(() => ({}));
          if (!r.ok) throw new Error(j.error ?? r.statusText);
          change((sp) => { Object.assign(sp, { ...j.spec, data: sp.data }); }, "structure");
          S.pi = 0;
          refresh(true);
          d.remove();
          if (j.engine) toast(`用 ${j.engine} 產生`);
        } catch (e) {
          toast(e.message, { err: true, ms: 6000 });
          ok.disabled = false;
          ok.lastChild.textContent = "產生";
        }
      };
    };

    // ── export / done ──
    async function finalSvg() {
      const r = await LP.render(S.spec, { styles: S.styleList, macros: opts.macros });
      return r;
    }
    function exportMenu(anchor) {
      if (!S.spec) return toast("先放資料");
      const go = async (format, dpi) => {
        try {
          const r = await finalSvg();
          let blob;
          if (api) {
            const res = await fetch(`${api}/export`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ svg: r.svg, spec: S.spec, format, dpi, w: r.w, h: r.h }) });
            if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error ?? res.statusText);
            blob = await res.blob();
          } else if (format === "svg") blob = new Blob([r.svg], { type: "image/svg+xml" });
          else throw new Error("示範頁只能匯出 SVG");
          const a = h("a", { href: URL.createObjectURL(blob), download: `${(opts.title || "figure").replace(/\.[a-z]+$/i, "")}.${format}` });
          document.body.append(a);
          a.click();
          a.remove();
        } catch (e) {
          toast(e.message, { err: true, ms: 6000 });
        }
      };
      menu(anchor, [
        { label: "PDF（論文）", icon: "pdf", run: () => go("pdf") },
        { label: "SVG", icon: "image", run: () => go("svg") },
        { label: "PNG 300 dpi", icon: "image", run: () => go("png", 300) },
        { label: "PNG 600 dpi", icon: "image", run: () => go("png", 600) },
        { sep: true },
        { label: "規格 JSON", icon: "code", run: () => { const a = h("a", { href: URL.createObjectURL(new Blob([JSON.stringify(S.spec, null, 2)], { type: "application/json" })), download: "figure.plot.json" }); document.body.append(a); a.click(); a.remove(); } },
      ]);
    }
    doneB.onclick = async () => {
      if (!S.spec) return toast("先放資料");
      doneB.disabled = true;
      const label = doneB.textContent;
      doneB.textContent = "儲存中…";
      try {
        const r = await finalSvg();
        const ok = await opts.save({ spec: clone(S.spec), svg: r.svg, w: r.w, h: r.h });
        if (ok === true || ok === undefined) return close({ spec: S.spec, svg: r.svg });
        if (typeof ok === "string") toast(ok, { err: true, ms: 6000 });
      } catch (e) {
        toast(e.message, { err: true, ms: 6000 });
      }
      doneB.disabled = false;
      doneB.textContent = label;
    };

    S.zoom = "fit";
    refresh(true);
  }
})();
