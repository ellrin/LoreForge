// <notation-panel> — the notation registry (對照表) as a framework-free Web
// Component. No build step: load it with a plain <script src>. KaTeX is used
// when window.katex exists (load vendor/katex/katex.min.js + .css first);
// otherwise LaTeX is shown as code.
//
// Data in:   panel.notation = {version, entries:[…]}   (notation.json)
//            panel.report   = {issues:[…], …}          (ym-notation check report)
//            panel.mode     = "macro" | "literal"       (插入 mode)
//            attributes src="notation.json" report-src="report.json" mode="macro"
// Events out (bubbling, composed; data in event.detail):
//   notation-insert  {id, entry, mode, math, text}   text to put at the caret (math: caret already in math)
//   notation-change  {id, field, value, entry, notation}   one field edited inline
//   notation-add     {entry, notation}                a new row
//   notation-delete  {id, entry, notation}
//   notation-jump    {file, line, col, para, sentence, context, issue}
//   notation-select  {id, entry}
//   notation-loaded  {what: "notation"|"report", url}   after src / report-src finished loading
// Globals: NotationPanel (the class, with static helpers), NotationTooltip.

(() => {
  const KIND = { symbol: "符號", operator: "運算子", equation: "方程式", figure: "圖", table: "表", abbrev: "縮寫", term: "術語" };
  const KINDS = Object.keys(KIND);
  const SEV = { error: "錯誤", warning: "警告", info: "提示" };
  const CAT = { symbol: "符號", equation: "方程式", abbrev: "縮寫", ref: "引用", style: "寫法", registry: "對照表" };

  const STYLE = `
notation-panel{--np-bg:#fff;--np-fg:#1d2228;--np-mute:#6b7280;--np-line:#e3e6ea;--np-soft:#f5f7f9;--np-accent:#2563eb;
  --np-err:#c62828;--np-warn:#b26a00;--np-info:#4b6584;--np-hi:#fff7d6;
  display:flex;flex-direction:column;min-height:0;font:13px/1.45 "Noto Sans TC",system-ui,sans-serif;color:var(--np-fg);background:var(--np-bg);border:1px solid var(--np-line);border-radius:8px;overflow:hidden}
@media (prefers-color-scheme:dark){notation-panel:not([theme=light]){--np-bg:#16191d;--np-fg:#e6e8eb;--np-mute:#9aa3ad;--np-line:#2b3036;--np-soft:#1e2227;--np-accent:#6ea8fe;--np-err:#ff6b6b;--np-warn:#f0b54a;--np-info:#9fb3c8;--np-hi:#3a3420}}
notation-panel[theme=dark]{--np-bg:#16191d;--np-fg:#e6e8eb;--np-mute:#9aa3ad;--np-line:#2b3036;--np-soft:#1e2227;--np-accent:#6ea8fe;--np-err:#ff6b6b;--np-warn:#f0b54a;--np-info:#9fb3c8;--np-hi:#3a3420}
notation-panel .np-tabs{display:flex;gap:2px;border-bottom:1px solid var(--np-line);background:var(--np-soft);padding:4px 6px 0}
notation-panel .np-tab{border:0;background:none;color:var(--np-mute);padding:6px 12px;border-radius:6px 6px 0 0;cursor:pointer;font:inherit}
notation-panel .np-tab[aria-selected=true]{background:var(--np-bg);color:var(--np-fg);font-weight:600;box-shadow:0 -1px 0 var(--np-line) inset}
notation-panel .np-badge{display:inline-block;min-width:18px;padding:0 5px;margin-left:4px;border-radius:9px;background:var(--np-line);font-size:11px;text-align:center}
notation-panel .np-badge.err{background:var(--np-err);color:#fff}
notation-panel .np-bar{display:flex;flex-wrap:wrap;gap:6px;align-items:center;padding:8px;border-bottom:1px solid var(--np-line)}
notation-panel .np-chip{border:1px solid var(--np-line);background:var(--np-bg);color:var(--np-fg);border-radius:14px;padding:2px 10px;cursor:pointer;font:inherit}
notation-panel .np-chip[aria-pressed=true]{border-color:var(--np-accent);color:var(--np-accent);font-weight:600}
notation-panel .np-search{flex:1;min-width:120px;border:1px solid var(--np-line);border-radius:6px;padding:4px 8px;background:var(--np-bg);color:var(--np-fg);font:inherit}
notation-panel .np-btn{border:1px solid var(--np-line);background:var(--np-bg);color:var(--np-fg);border-radius:6px;padding:3px 9px;cursor:pointer;font:inherit}
notation-panel .np-btn:hover{border-color:var(--np-accent)}
notation-panel .np-btn.primary{background:var(--np-accent);border-color:var(--np-accent);color:#fff}
notation-panel .np-seg{display:inline-flex;border:1px solid var(--np-line);border-radius:6px;overflow:hidden}
notation-panel .np-seg button{border:0;background:var(--np-bg);color:var(--np-mute);padding:3px 9px;cursor:pointer;font:inherit}
notation-panel .np-seg button[aria-pressed=true]{background:var(--np-accent);color:#fff}
notation-panel .np-body{flex:1;min-height:0;overflow:auto}
notation-panel table{border-collapse:collapse;width:100%}
notation-panel th{position:sticky;top:0;background:var(--np-soft);text-align:left;font-weight:600;color:var(--np-mute);padding:5px 8px;border-bottom:1px solid var(--np-line);cursor:pointer;white-space:nowrap;user-select:none}
notation-panel th[data-dir=asc]::after{content:" ▲";font-size:9px}
notation-panel th[data-dir=desc]::after{content:" ▼";font-size:9px}
notation-panel td{padding:5px 8px;border-bottom:1px solid var(--np-line);vertical-align:top}
notation-panel tr.sel td{background:var(--np-hi)}
notation-panel td.ed{cursor:text}
notation-panel td.ed:hover{outline:1px dashed var(--np-line)}
notation-panel td.empty{color:var(--np-mute)}
notation-panel td input{width:100%;box-sizing:border-box;font:inherit;padding:2px 4px;border:1px solid var(--np-accent);border-radius:4px;background:var(--np-bg);color:var(--np-fg)}
notation-panel td input.bad{border-color:var(--np-err)}
notation-panel .np-kind{font-size:11px;color:var(--np-mute);white-space:nowrap}
notation-panel .np-code{font-family:"JetBrains Mono",ui-monospace,monospace;font-size:12px;color:var(--np-mute)}
notation-panel .np-ops{white-space:nowrap}
notation-panel .np-origin{font-size:10px;color:var(--np-mute);border:1px solid var(--np-line);border-radius:4px;padding:0 3px;margin-left:4px}
notation-panel .np-issue{padding:7px 10px;border-bottom:1px solid var(--np-line);display:grid;grid-template-columns:auto 1fr;gap:2px 8px}
notation-panel .np-sev{font-size:11px;font-weight:600;border-radius:4px;padding:0 5px;height:fit-content;margin-top:1px}
notation-panel .np-sev.error{color:#fff;background:var(--np-err)}
notation-panel .np-sev.warning{color:#fff;background:var(--np-warn)}
notation-panel .np-sev.info{color:var(--np-info);border:1px solid var(--np-info)}
notation-panel .np-msg{grid-column:2}
notation-panel .np-locs{grid-column:2;display:flex;flex-wrap:wrap;gap:4px}
notation-panel .np-loc{border:0;background:var(--np-soft);color:var(--np-accent);border-radius:4px;padding:1px 6px;cursor:pointer;font:12px "JetBrains Mono",ui-monospace,monospace}
notation-panel .np-loc.rel{color:var(--np-mute)}
notation-panel .np-details{grid-column:2;color:var(--np-mute);font-size:12px;margin:2px 0 0;padding-left:16px}
notation-panel .np-empty{padding:24px;text-align:center;color:var(--np-mute)}
.np-tip{position:fixed;z-index:10000;max-width:320px;padding:8px 10px;border-radius:8px;background:#1f2937;color:#f3f4f6;font:13px/1.45 "Noto Sans TC",system-ui,sans-serif;box-shadow:0 6px 24px rgba(0,0,0,.25);pointer-events:none}
.np-tip .np-tip-sym{font-size:18px;margin-bottom:4px}
.np-tip .np-tip-meta{color:#9ca3af;font-size:12px;margin-top:4px}
`;

  function injectStyle() {
    if (document.getElementById("notation-panel-style")) return;
    const s = document.createElement("style");
    s.id = "notation-panel-style";
    s.textContent = STYLE;
    document.head.append(s);
  }

  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  };

  /** Rough canonical form of a symbol's LaTeX (mirrors ym_notation::math::canonical for common cases). */
  function canon(latex) {
    if (!latex) return "";
    let s = String(latex).replace(/\s+/g, "");
    s = s.replace(/\\(bm|mathbf|mathbold|pmb)\{/g, "\\boldsymbol{").replace(/\\(bm|mathbf)([a-zA-Z])/g, "\\boldsymbol{$2}");
    s = s.replace(/\\widetilde/g, "\\tilde").replace(/\\widehat/g, "\\hat").replace(/\\overline/g, "\\bar");
    s = s.replace(/\\(tilde|hat|bar|vec|dot)(\\[a-zA-Z]+|[a-zA-Z])/g, "\\$1{$2}");
    s = s.replace(/([_^])(\\[a-zA-Z]+|[a-zA-Z0-9])/g, "$1{$2}");
    return s;
  }

  /** Macro map for KaTeX: {"\\speed": "u"} (same rules as ym_notation::katex_macros). */
  function katexMacros(notation) {
    const out = {};
    for (const e of notation?.entries ?? []) {
      const m = macroName(e);
      if (!m || !/^[A-Za-z]+$/.test(m)) continue;
      let body = null;
      switch (e.kind ?? "symbol") {
        case "symbol": case "equation": body = e.latex; break;
        case "operator": {
          const w = (e.latex ?? "").replace(/^\\operatorname\{(.*)\}$/, "$1");
          body = /^[A-Za-z0-9 -]+$/.test(w) ? `\\operatorname{${w}}` : e.latex;
          break;
        }
        case "abbrev": body = e.short ? `\\text{${e.short}}` : null; break;
        case "term": body = e.meaning_en || e.short ? `\\text{${e.meaning_en || e.short}}` : null; break;
        case "figure": case "table": body = null; break;
      }
      if (body) out["\\" + m] = body;
    }
    return out;
  }

  const macroName = (e) => (e?.macro ?? "").trim().replace(/^\\/, "") || null;

  /** What 插入 puts at the caret (same rules as ym_notation::insert_text). */
  function insertText(e, mode, inMath) {
    const mac = mode === "macro" ? macroName(e) : null;
    const wrap = (s) => (inMath ? s : `$${s}$`);
    switch (e.kind ?? "symbol") {
      case "symbol": case "operator":
        return wrap(mac ? `\\${mac}` : e.latex ?? "");
      case "equation":
        if (e.label && !inMath) return `\\eqref{${e.label}}`;
        return wrap(mac ? `\\${mac}` : e.latex ?? "");
      case "figure": case "table":
        if (mac) return `\\${mac}{}`;
        return `${e.kind === "figure" ? "Fig." : "Table"}~\\ref{${e.label ?? ""}}`;
      case "abbrev":
        return mac ? `\\${mac}{}` : e.short ?? "";
      case "term":
        return mac ? `\\${mac}{}` : e.meaning_en ?? e.meaning ?? "";
    }
    return "";
  }

  function renderMath(node, tex, macros, display = false) {
    if (window.katex && tex) {
      try {
        window.katex.render(tex, node, { throwOnError: false, displayMode: display, macros: { ...macros } });
        return;
      } catch (_) { /* fall through */ }
    }
    node.textContent = tex ?? "";
    node.classList.add("np-code");
  }

  /** Text with $…$ segments rendered by KaTeX. */
  function renderInline(node, text, macros) {
    node.textContent = "";
    String(text ?? "").split(/(\$[^$]+\$)/g).forEach((part) => {
      if (part.length > 2 && part.startsWith("$") && part.endsWith("$")) {
        const m = el("span");
        renderMath(m, part.slice(1, -1), macros);
        node.append(m);
      } else if (part) node.append(part);
    });
  }

  /** The main column of a row: what it is. */
  function primary(e) {
    switch (e.kind ?? "symbol") {
      case "figure": case "table": return ["label", e.label];
      case "abbrev": return ["short", e.short];
      case "term": return ["meaning_en", e.meaning_en];
      default: return ["latex", e.latex];
    }
  }

  const COLUMNS = [
    { key: "kind", title: "種類", sort: (e) => KINDS.indexOf(e.kind ?? "symbol") },
    { key: "primary", title: "符號／內容", sort: (e) => String(primary(e)[1] ?? "").toLowerCase() },
    { key: "macro", title: "巨集", edit: "macro", sort: (e) => e.macro ?? "" },
    { key: "meaning", title: "意義", edit: "meaning", sort: (e) => e.meaning ?? "" },
    { key: "detail", title: "英文／全稱", sort: (e) => (e.kind === "abbrev" ? e.long : e.meaning_en) ?? "" },
    { key: "gloss", title: "註解", edit: "gloss", sort: (e) => e.gloss ?? "" },
    { key: "defined_at", title: "定義位置", edit: "defined_at", sort: (e) => e.defined_at ?? "" },
  ];

  class NotationPanel extends HTMLElement {
    static get observedAttributes() { return ["src", "report-src", "mode"]; }

    constructor() {
      super();
      this._notation = { version: 1, entries: [] };
      this._report = null;
      this._mode = "macro";
      this._kind = "all";
      this._query = "";
      this._sort = { key: "kind", dir: "asc" };
      this._tab = "table";
      this._sevFilter = { error: true, warning: true, info: true };
      this._selected = null;
      this._origin = {};
      this.macros = {};
      this.inMath = () => false;
    }

    connectedCallback() {
      injectStyle();
      if (!this._built) this._build();
      this._render();
    }

    attributeChangedCallback(name, _old, val) {
      if (name === "src" && val) this.load(val);
      if (name === "report-src" && val) this.loadReport(val);
      if (name === "mode" && val) this.mode = val;
    }

    get notation() { return this._notation; }
    set notation(n) {
      this._notation = n && Array.isArray(n.entries) ? n : { version: 1, entries: [] };
      this._origin = {};
      for (const e of this._notation.entries) if (e.origin) this._origin[e.id] = e.origin;
      this._render();
    }
    get report() { return this._report; }
    set report(r) { this._report = r; this._render(); }
    get mode() { return this._mode; }
    set mode(m) { this._mode = m === "literal" ? "literal" : "macro"; this._render(); }

    async load(url) {
      this.notation = await (await fetch(url)).json();
      this._emit("notation-loaded", { what: "notation", url });
    }
    async loadReport(url) {
      this.report = await (await fetch(url)).json();
      this._emit("notation-loaded", { what: "report", url });
    }

    showTab(t) { this._tab = t === "checks" ? "checks" : "table"; this._render(); }

    /** Highlight a row (by id) and scroll it into view. */
    select(id) {
      this._selected = id;
      this._tab = "table";
      this._kind = "all";
      this._render();
      this.querySelector(`tr[data-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "nearest" });
      const entry = this.entry(id);
      if (entry) this._emit("notation-select", { id, entry });
    }

    entry(id) { return this._notation.entries.find((e) => e.id === id); }

    /** Entry for a LaTeX snippet or a macro call (`\speed`). */
    findByLatex(tex) {
      const t = String(tex ?? "").trim();
      const mac = t.match(/^\\([A-Za-z]+)$/)?.[1];
      if (mac) {
        const byMacro = this._notation.entries.find((e) => macroName(e) === mac);
        if (byMacro) return byMacro;
      }
      const c = canon(t);
      return this._notation.entries.find((e) => e.latex && canon(e.latex) === c) ?? null;
    }

    allMacros() { return { ...katexMacros(this._notation), ...this.macros }; }

    _emit(type, detail) { this.dispatchEvent(new CustomEvent(type, { detail, bubbles: true, composed: true })); }

    _build() {
      this._built = true;
      this.textContent = "";
      const tabs = el("div", "np-tabs");
      tabs.setAttribute("role", "tablist");
      this._tabTable = el("button", "np-tab", "對照表");
      this._tabChecks = el("button", "np-tab", "校對");
      for (const [b, t] of [[this._tabTable, "table"], [this._tabChecks, "checks"]]) {
        b.setAttribute("role", "tab");
        b.onclick = () => this.showTab(t);
        tabs.append(b);
      }
      this._bar = el("div", "np-bar");
      this._body = el("div", "np-body");
      this.append(tabs, this._bar, this._body);
    }

    _render() {
      if (!this._built) return;
      const n = this._notation.entries.length;
      const issues = this._report?.issues ?? [];
      const errs = issues.filter((i) => i.severity === "error").length;
      this._tabTable.innerHTML = "";
      this._tabTable.append("對照表", el("span", "np-badge", String(n)));
      this._tabChecks.innerHTML = "";
      this._tabChecks.append("校對", el("span", "np-badge" + (errs ? " err" : ""), String(issues.length)));
      this._tabTable.setAttribute("aria-selected", String(this._tab === "table"));
      this._tabChecks.setAttribute("aria-selected", String(this._tab === "checks"));
      if (this._tab === "table") this._renderTable(); else this._renderChecks();
    }

    // ── table ────────────────────────────────────────────────────────────
    _renderTable() {
      const bar = this._bar;
      bar.textContent = "";
      const counts = {};
      for (const e of this._notation.entries) counts[e.kind ?? "symbol"] = (counts[e.kind ?? "symbol"] ?? 0) + 1;
      for (const k of ["all", ...KINDS]) {
        if (k !== "all" && !counts[k]) continue;
        const b = el("button", "np-chip", k === "all" ? `全部 ${this._notation.entries.length}` : `${KIND[k]} ${counts[k]}`);
        b.setAttribute("aria-pressed", String(this._kind === k));
        b.onclick = () => { this._kind = k; this._renderTable(); };
        bar.append(b);
      }
      const q = el("input", "np-search");
      q.type = "search";
      q.placeholder = "搜尋符號、意義、巨集…";
      q.value = this._query;
      q.oninput = () => { this._query = q.value; this._renderRows(); };
      const seg = el("span", "np-seg");
      seg.title = "插入方式：巨集（改表一處、全文跟著變）或原始 LaTeX";
      for (const [m, label] of [["macro", "巨集"], ["literal", "原文"]]) {
        const b = el("button", "", label);
        b.setAttribute("aria-pressed", String(this._mode === m));
        b.onclick = () => { this.mode = m; this.setAttribute("mode", m); };
        seg.append(b);
      }
      const add = el("button", "np-btn primary", "新增");
      add.onclick = () => this._add();
      bar.append(q, seg, add);
      this._renderRows();
    }

    _rows() {
      const f = this._query.trim().toLowerCase();
      let rows = this._notation.entries.filter((e) => this._kind === "all" || (e.kind ?? "symbol") === this._kind);
      if (f) {
        rows = rows.filter((e) =>
          [e.id, e.latex, e.macro, e.meaning, e.meaning_en, e.gloss, e.short, e.long, e.label, e.caption, ...(e.tags ?? [])]
            .some((v) => v && String(v).toLowerCase().includes(f)));
      }
      const col = COLUMNS.find((c) => c.key === this._sort.key) ?? COLUMNS[0];
      const dir = this._sort.dir === "asc" ? 1 : -1;
      return rows
        .map((e, i) => [e, i])
        .sort((a, b) => {
          const x = col.sort(a[0]), y = col.sort(b[0]);
          return (x < y ? -1 : x > y ? 1 : a[1] - b[1]) * dir;
        })
        .map(([e]) => e);
    }

    _renderRows() {
      const body = this._body;
      body.textContent = "";
      const rows = this._rows();
      if (!rows.length) {
        body.append(el("div", "np-empty", this._notation.entries.length ? "沒有符合的項目" : "對照表是空的，按「新增」或從論文自動擷取"));
        return;
      }
      const macros = this.allMacros();
      const table = el("table");
      const head = el("tr");
      for (const c of [...COLUMNS, { key: "ops", title: "" }]) {
        const th = el("th", "", c.title);
        if (c.sort) {
          if (this._sort.key === c.key) th.dataset.dir = this._sort.dir;
          th.onclick = () => {
            this._sort = { key: c.key, dir: this._sort.key === c.key && this._sort.dir === "asc" ? "desc" : "asc" };
            this._renderRows();
          };
        }
        head.append(th);
      }
      const thead = el("thead");
      thead.append(head);
      const tbody = el("tbody");
      for (const e of rows) tbody.append(this._row(e, macros));
      table.append(thead, tbody);
      body.append(table);
    }

    _row(e, macros) {
      const tr = el("tr");
      tr.dataset.id = e.id;
      if (e.id === this._selected) tr.classList.add("sel");
      tr.onclick = (ev) => {
        if (ev.target.closest("button,input")) return;
        this._selected = e.id;
        this.querySelectorAll("tr.sel").forEach((r) => r.classList.remove("sel"));
        tr.classList.add("sel");
        this._emit("notation-select", { id: e.id, entry: e });
      };
      const kind = el("td", "np-kind", KIND[e.kind ?? "symbol"] ?? e.kind);
      if (this._origin[e.id] === "global") kind.append(el("span", "np-origin", "全域"));
      tr.append(kind);
      // primary
      const [pfield, pval] = primary(e);
      const p = el("td", "ed");
      const kindNow = e.kind ?? "symbol";
      if (["symbol", "operator", "equation"].includes(kindNow) && pval) {
        renderMath(p, pval, macros, kindNow === "equation" && /\\\\|&/.test(pval));
      } else {
        p.textContent = pval ?? "—";
        if (!pval) p.classList.add("empty");
        if (kindNow === "figure" || kindNow === "table") {
          p.classList.add("np-code");
          if (e.caption) p.title = e.caption;
        }
      }
      this._editable(p, e, pfield);
      tr.append(p);
      // macro
      const m = el("td", "ed np-code", macroName(e) ? "\\" + macroName(e) : "");
      this._editable(m, e, "macro");
      tr.append(m);
      for (const f of ["meaning"]) {
        const td = el("td", "ed", e[f] ?? "");
        this._editable(td, e, f);
        tr.append(td);
      }
      const dfield = kindNow === "abbrev" ? "long" : kindNow === "figure" || kindNow === "table" ? "caption" : "meaning_en";
      const d = el("td", "ed", e[dfield] ?? "");
      this._editable(d, e, dfield);
      tr.append(d);
      const g = el("td", "ed", e.gloss ?? "");
      this._editable(g, e, "gloss");
      tr.append(g);
      const at = el("td", "ed np-code", e.defined_at ?? "");
      this._editable(at, e, "defined_at");
      tr.append(at);
      const ops = el("td", "np-ops");
      const ins = el("button", "np-btn", "插入");
      ins.title = this._mode === "macro" && macroName(e) ? `插入 \\${macroName(e)}` : "插入 LaTeX";
      ins.onmousedown = (ev) => ev.preventDefault(); // keep the editor's caret
      ins.onclick = () => this.insert(e.id);
      const del = el("button", "np-btn", "刪除");
      del.onclick = () => {
        if (!confirm(`刪除「${e.id}」？`)) return;
        this._notation.entries = this._notation.entries.filter((x) => x !== e);
        this._emit("notation-delete", { id: e.id, entry: e, notation: this._notation });
        this._render();
      };
      ops.append(ins, " ", del);
      tr.append(ops);
      return tr;
    }

    /** Emit notation-insert for an entry (also callable by the host). */
    insert(id) {
      const e = this.entry(id);
      if (!e) return;
      const mode = this._mode;
      this._emit("notation-insert", { id, entry: e, mode, math: insertText(e, mode, true), text: insertText(e, mode, false), inMath: !!this.inMath() });
    }

    _editable(td, e, field) {
      td.title = td.title || "雙擊編輯";
      td.ondblclick = () => {
        if (td.querySelector("input")) return;
        const input = el("input");
        const old = field === "macro" ? (macroName(e) ?? "") : (e[field] ?? "");
        input.value = old;
        td.textContent = "";
        td.append(input);
        input.focus();
        input.select();
        const check = () => {
          const bad = field === "macro" && input.value && !/^\\?[A-Za-z]+$/.test(input.value.trim());
          input.classList.toggle("bad", bad);
          return !bad;
        };
        input.oninput = check;
        let done = false;
        const finish = (save) => {
          if (done) return;
          done = true;
          if (save && check() && input.value !== old) this.setField(e.id, field, input.value);
          else this._renderRows();
        };
        input.onkeydown = (ev) => {
          if (ev.key === "Enter") finish(true);
          if (ev.key === "Escape") finish(false);
        };
        input.onblur = () => finish(true);
      };
    }

    /** Change one field (empty → removed) and emit notation-change. */
    setField(id, field, value) {
      const e = this.entry(id);
      if (!e) return;
      let v = typeof value === "string" ? value.trim() : value;
      if (field === "macro" && v) v = v.replace(/^\\/, "");
      if (v === "" || v == null) delete e[field]; else e[field] = v;
      this._emit("notation-change", { id, field, value: e[field] ?? null, entry: e, notation: this._notation });
      this._render();
    }

    /** Add an entry (a template from the current kind filter unless given). */
    add(entry) { return this._add(entry); }

    _add(entry) {
      const kind = entry?.kind ?? (this._kind === "all" ? "symbol" : this._kind);
      const base = entry?.id ?? (entry?.latex ? canon(entry.latex).replace(/[^A-Za-z0-9]+/g, "-").replace(/^-|-$/g, "") : `new-${kind}`) ?? "new";
      let id = base || "new", k = 2;
      while (this.entry(id)) id = `${base}-${k++}`;
      const e = { id, kind, ...(entry ?? {}) };
      e.id = id;
      this._notation.entries.push(e);
      this._emit("notation-add", { entry: e, notation: this._notation });
      this._selected = id;
      this._kind = "all";
      this._query = "";
      this._tab = "table";
      this._render();
      this.querySelector(`tr[data-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "nearest" });
      return e;
    }

    // ── checks ───────────────────────────────────────────────────────────
    _renderChecks() {
      const bar = this._bar;
      bar.textContent = "";
      const issues = this._report?.issues ?? [];
      for (const s of ["error", "warning", "info"]) {
        const n = issues.filter((i) => i.severity === s).length;
        const b = el("button", "np-chip", `${SEV[s]} ${n}`);
        b.setAttribute("aria-pressed", String(this._sevFilter[s]));
        b.onclick = () => { this._sevFilter[s] = !this._sevFilter[s]; this._renderChecks(); };
        bar.append(b);
      }
      const cat = el("select", "np-btn");
      cat.append(new Option("全部類別", ""));
      for (const [k, v] of Object.entries(CAT)) cat.append(new Option(v, k));
      cat.value = this._cat ?? "";
      cat.onchange = () => { this._cat = cat.value; this._renderChecks(); };
      bar.append(cat);
      const body = this._body;
      body.textContent = "";
      if (!this._report) {
        body.append(el("div", "np-empty", "尚未校對"));
        return;
      }
      const macros = this.allMacros();
      const shown = issues.filter((i) => this._sevFilter[i.severity] && (!this._cat || i.category === this._cat));
      if (!shown.length) {
        body.append(el("div", "np-empty", issues.length ? "沒有符合篩選的項目" : "沒有發現問題"));
        return;
      }
      for (const i of shown) body.append(this._issue(i, macros));
    }

    _issue(i, macros) {
      const box = el("div", "np-issue");
      box.append(el("span", `np-sev ${i.severity}`, SEV[i.severity] ?? i.severity));
      const msg = el("div", "np-msg");
      renderInline(msg, i.message, macros);
      box.append(msg);
      const locs = el("div", "np-locs");
      const loc = (l, rel) => {
        const b = el("button", "np-loc" + (rel ? " rel" : ""), `${l.file}:${l.line}`);
        b.title = `${rel ? "相關位置" : "跳到"}：第 ${l.para + 1} 段第 ${l.para_sentence ?? l.sentence} 句（${l.context}）\n${l.snippet ?? ""}`;
        b.onclick = () => this._emit("notation-jump", { ...l, issue: i });
        return b;
      };
      for (const l of i.locations ?? []) locs.append(loc(l, false));
      for (const l of i.related ?? []) locs.append(loc(l, true));
      if (i.entry && this.entry(i.entry)) {
        const b = el("button", "np-btn", "在表中顯示");
        b.onclick = () => this.select(i.entry);
        locs.append(b);
      }
      if ((i.code === "undefined_symbol" || i.code === "unregistered_symbol") && i.subject && !this.findByLatex(i.subject)) {
        const b = el("button", "np-btn", "加入對照表");
        b.onclick = () => this._add({ kind: "symbol", latex: i.subject });
        locs.append(b);
      }
      if (i.code === "abbrev_never_expanded" && i.subject && !this._notation.entries.some((e) => e.short === i.subject)) {
        const b = el("button", "np-btn", "加入縮寫");
        b.onclick = () => this._add({ kind: "abbrev", short: i.subject, id: `abbr-${i.subject.toLowerCase()}` });
        locs.append(b);
      }
      if (locs.childNodes.length) box.append(locs);
      if (i.details?.length) {
        const ul = el("ul", "np-details");
        for (const d of i.details) {
          const li = el("li");
          renderInline(li, d, macros);
          ul.append(li);
        }
        box.append(ul);
      }
      return box;
    }
  }

  // ── hover tooltip ────────────────────────────────────────────────────
  /**
   * Tooltip with an entry's rendering, meaning, gloss and definition place.
   *   const tip = new NotationTooltip(panel);            // or new NotationTooltip(() => notationObject)
   *   tip.attach(previewElement);                        // hovers over [data-notation] inside it
   *   tip.show(x, y, "\\boldsymbol{w}"); tip.hide();      // manual use (e.g. over KaTeX nodes)
   * data-notation holds an entry id, a LaTeX snippet or a macro (`\speed`).
   */
  class NotationTooltip {
    constructor(source) {
      this.source = source;
      this.node = null;
    }
    _notation() {
      if (this.source instanceof NotationPanel) return this.source.notation;
      return typeof this.source === "function" ? this.source() : this.source;
    }
    find(key) {
      if (key && typeof key === "object") return key;
      const n = this._notation();
      const entries = n?.entries ?? [];
      const byId = entries.find((e) => e.id === key);
      if (byId) return byId;
      const mac = String(key ?? "").trim().match(/^\\([A-Za-z]+)$/)?.[1];
      if (mac) {
        const m = entries.find((e) => macroName(e) === mac);
        if (m) return m;
      }
      const c = canon(key);
      return entries.find((e) => e.latex && canon(e.latex) === c) ?? entries.find((e) => e.short && e.short === key) ?? null;
    }
    show(x, y, key) {
      const e = this.find(key);
      if (!e) return this.hide();
      injectStyle();
      if (!this.node) {
        this.node = el("div", "np-tip");
        document.body.append(this.node);
      }
      const t = this.node;
      t.textContent = "";
      const macros = katexMacros(this._notation());
      const sym = el("div", "np-tip-sym");
      if (e.latex && ["symbol", "operator", "equation"].includes(e.kind ?? "symbol")) renderMath(sym, e.latex, macros);
      else sym.textContent = e.short ?? e.label ?? e.meaning_en ?? e.id;
      t.append(sym);
      const meaning = [e.meaning, e.meaning_en ?? e.long].filter(Boolean).join(" · ");
      if (meaning) t.append(el("div", "", meaning));
      if (e.gloss) t.append(el("div", "", `註：${e.gloss}`));
      const meta = [
        e.shape && `形狀 ${e.shape}`,
        e.unit && `單位 ${e.unit}`,
        e.macro && `\\${macroName(e)}`,
        e.defined_at && `定義於 ${e.defined_at}`,
        e.kb && `知識卡 ${e.kb}`,
      ].filter(Boolean).join("　");
      if (meta) t.append(el("div", "np-tip-meta", meta));
      t.style.display = "block";
      const w = t.offsetWidth, h = t.offsetHeight;
      t.style.left = `${Math.max(8, Math.min(x + 12, innerWidth - w - 8))}px`;
      t.style.top = `${y + 16 + h > innerHeight ? Math.max(8, y - h - 8) : y + 16}px`;
    }
    hide() { if (this.node) this.node.style.display = "none"; }
    attach(root, selector = "[data-notation]") {
      root.addEventListener("mouseover", (ev) => {
        const t = ev.target.closest?.(selector);
        if (t && root.contains(t)) this.show(ev.clientX, ev.clientY, t.dataset.notation);
      });
      root.addEventListener("mouseout", (ev) => {
        const t = ev.target.closest?.(selector);
        if (t && !t.contains(ev.relatedTarget)) this.hide();
      });
      return this;
    }
    /** Render text with $…$ through KaTeX, marking segments that match an entry with data-notation. */
    renderText(node, text) {
      const n = this._notation();
      const macros = katexMacros(n);
      node.textContent = "";
      String(text ?? "").split(/(\$[^$]+\$)/g).forEach((part) => {
        if (part.length > 2 && part.startsWith("$") && part.endsWith("$")) {
          const tex = part.slice(1, -1);
          const span = el("span");
          renderMath(span, tex, macros);
          if (this.find(tex)) span.dataset.notation = tex;
          node.append(span);
        } else if (part) node.append(part);
      });
    }
  }

  NotationPanel.insertText = insertText;
  NotationPanel.katexMacros = katexMacros;
  NotationPanel.canon = canon;
  NotationPanel.KIND = KIND;
  window.NotationPanel = NotationPanel;
  window.NotationTooltip = NotationTooltip;
  if (!customElements.get("notation-panel")) customElements.define("notation-panel", NotationPanel);
})();
