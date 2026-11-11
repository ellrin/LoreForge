// <notation-panel> — the notation registry (對照表) as a framework-free Web
// Component. No build step: load it with a plain <script src>. KaTeX is used
// when window.katex exists (load vendor/katex/katex.min.js + .css first);
// otherwise LaTeX is shown as code.
//
// Data in:   panel.notation = {version, entries:[…], conventional:[…], ignored:[…]}   (notation.json)
//            panel.report   = {issues:[…], …}          (lore-notation check report)
//            panel.mode     = "macro" | "literal"       (插入 mode: 用巨集 / 符號直接寫)
//            panel.docMacros = ["foo", …]               commands the document already defines (macro check)
//            attributes src="notation.json" report-src="report.json" mode="macro"
//            attribute readonly (or entry.readonly on one row): no inline edit, no 刪除 (and no 新增 when on the panel)
//            panel.actions = [{ id, label, title?, show?(entry), disabled?(entry), run?(entry) }]  extra buttons per row
//            Colours come from the app's theme tokens (--bg --text --muted --line --panel --accent-text …) when
//            they exist, else the built-in light/dark palette; set --np-* on the element to override.
// Events out (bubbling, composed; data in event.detail):
//   notation-insert  {id, entry, mode, math, text}   text to put at the caret (math: caret already in math)
//   notation-change  {id, field, value, entry, notation}   one field edited inline; id null and field
//                    "ignored" / "conventional" when a check finding is dismissed or marked conventional
//   notation-add     {entry, notation}                a new row (also an undone delete)
//   notation-delete  {id, entry, notation}
//   notation-jump    {file, line, col, para, sentence, context, issue}
//   notation-select  {id, entry}
//   notation-action  {action, id, entry}              a custom row action was pressed
//   notation-loaded  {what: "notation"|"report", url}   after src / report-src finished loading
// Globals: NotationPanel (the class, with static helpers), NotationTooltip.

(() => {
  const KIND = { symbol: "符號", operator: "運算子", equation: "方程式", figure: "圖", table: "表", abbrev: "縮寫", term: "術語" };
  const KINDS = Object.keys(KIND);
  const SEV = { error: "錯誤", warning: "警告", info: "提示" };
  const CAT = { symbol: "符號", equation: "方程式", abbrev: "縮寫", ref: "引用", style: "寫法", registry: "對照表" };
  const CONTEXT = { body: "正文", caption: "圖表說明", table: "表格", heading: "標題", abstract: "摘要", equation: "方程式", definition: "定義" };
  // findings whose subject can be "標為慣用" (an abbreviation or a symbol that needs no definition)
  const CONVENTIONAL_CODES = ["abbrev_never_expanded", "abbrev_abstract", "abbrev_used_before_expansion", "undefined_symbol", "unregistered_symbol"];

  const STYLE = `
notation-panel{--np-bg:var(--bg,#fff);--np-fg:var(--text,#1d2228);--np-mute:var(--muted,#6b7280);--np-line:var(--line,#e3e6ea);--np-soft:var(--panel,#f5f7f9);--np-accent:var(--accent-text,#2563eb);--np-ink:var(--accent-ink,#fff);
  --np-err:var(--err,#c62828);--np-warn:var(--warn,#b26a00);--np-on-warn:var(--bg,#fff);--np-info:var(--muted,#4b6584);--np-hi:var(--accent-soft,#fff7d6);--np-eq-max:220px;
  position:relative;display:flex;flex-direction:column;min-height:0;font:13px/1.45 "Noto Sans TC",system-ui,sans-serif;color:var(--np-fg);background:var(--np-bg);border:1px solid var(--np-line);border-radius:8px;overflow:hidden}
@media (prefers-color-scheme:dark){notation-panel:not([theme=light]){--np-bg:var(--bg,#16191d);--np-fg:var(--text,#e6e8eb);--np-mute:var(--muted,#9aa3ad);--np-line:var(--line,#2b3036);--np-soft:var(--panel,#1e2227);--np-accent:var(--accent-text,#6ea8fe);--np-ink:var(--accent-ink,#0b1220);--np-err:var(--err,#ff6b6b);--np-warn:var(--warn,#f0b54a);--np-on-warn:var(--bg,#1d1a12);--np-info:var(--muted,#9fb3c8);--np-hi:var(--accent-soft,#3a3420)}}
notation-panel[theme=dark]{--np-bg:var(--bg,#16191d);--np-fg:var(--text,#e6e8eb);--np-mute:var(--muted,#9aa3ad);--np-line:var(--line,#2b3036);--np-soft:var(--panel,#1e2227);--np-accent:var(--accent-text,#6ea8fe);--np-ink:var(--accent-ink,#0b1220);--np-err:var(--err,#ff6b6b);--np-warn:var(--warn,#f0b54a);--np-on-warn:var(--bg,#1d1a12);--np-info:var(--muted,#9fb3c8);--np-hi:var(--accent-soft,#3a3420)}
notation-panel .np-tabs{display:flex;gap:2px;border-bottom:1px solid var(--np-line);background:var(--np-soft);padding:4px 6px 0}
notation-panel .np-tab{border:0;background:none;color:var(--np-mute);padding:6px 12px;border-radius:6px 6px 0 0;cursor:pointer;font:inherit}
notation-panel .np-tab[aria-selected=true]{background:var(--np-bg);color:var(--np-fg);font-weight:600;box-shadow:0 -1px 0 var(--np-line) inset}
notation-panel .np-badge{display:inline-block;min-width:18px;padding:0 5px;margin-left:4px;border-radius:9px;background:var(--np-line);font-size:11px;text-align:center}
notation-panel .np-badge.err{background:var(--np-err);color:var(--np-on-warn)}
notation-panel .np-bar{display:flex;flex-wrap:wrap;gap:6px;align-items:center;padding:8px;border-bottom:1px solid var(--np-line)}
notation-panel .np-chip{border:1px solid var(--np-line);background:var(--np-bg);color:var(--np-fg);border-radius:14px;padding:2px 10px;cursor:pointer;font:inherit;white-space:nowrap}
notation-panel .np-chip[aria-pressed=true]{border-color:var(--np-accent);color:var(--np-accent);font-weight:600}
notation-panel .np-search{flex:1;min-width:120px;border:1px solid var(--np-line);border-radius:6px;padding:4px 8px;background:var(--np-bg);color:var(--np-fg);font:inherit}
notation-panel .np-btn{border:1px solid var(--np-line);background:var(--np-bg);color:var(--np-fg);border-radius:6px;padding:3px 9px;cursor:pointer;font:inherit;white-space:nowrap}
notation-panel .np-btn:hover{border-color:var(--np-accent)}
notation-panel .np-btn.primary{width:auto;display:inline-block;padding:3px 9px;border:1px solid var(--np-accent);border-radius:6px;font-weight:400;background:var(--np-accent);color:var(--np-ink)}
notation-panel .np-btn.quiet{border-color:transparent;color:var(--np-mute);padding:3px 6px}
notation-panel .np-btn.quiet:hover{color:var(--np-fg);border-color:var(--np-line)}
notation-panel .np-seg{display:inline-flex;border:1px solid var(--np-line);border-radius:6px;overflow:hidden}
notation-panel .np-seg button{border:0;background:var(--np-bg);color:var(--np-mute);padding:3px 9px;cursor:pointer;font:inherit;white-space:nowrap}
notation-panel .np-seg button[aria-pressed=true]{background:var(--np-accent);color:var(--np-ink)}
notation-panel .np-body{flex:1;min-height:0;overflow:auto}
notation-panel table{border-collapse:separate;border-spacing:0;width:100%}
notation-panel th{position:sticky;top:0;z-index:2;background:var(--np-soft);text-align:left;font-weight:600;color:var(--np-mute);padding:5px 8px;border-bottom:1px solid var(--np-line);cursor:pointer;white-space:nowrap;user-select:none}
notation-panel th[data-dir=asc]::after{content:" ▲";font-size:9px}
notation-panel th[data-dir=desc]::after{content:" ▼";font-size:9px}
notation-panel td{padding:5px 8px;border-bottom:1px solid var(--np-line);vertical-align:top;background:var(--np-bg)}
notation-panel tr.sel td{background:var(--np-hi)}
notation-panel td.ed{cursor:text}
notation-panel td.ed:hover{outline:1px dashed var(--np-line);outline-offset:-1px}
notation-panel td.empty,notation-panel .np-fallback{color:var(--np-mute)}
notation-panel .np-fallback{font-style:italic}
notation-panel td input{width:100%;min-width:8em;box-sizing:border-box;font:inherit;padding:2px 4px;border:1px solid var(--np-accent);border-radius:4px;background:var(--np-bg);color:var(--np-fg)}
notation-panel td input.bad{border-color:var(--np-err);box-shadow:0 0 0 1px var(--np-err)}
notation-panel td.bad-macro{box-shadow:inset 0 0 0 1px var(--np-err)}
notation-panel .np-why{color:var(--np-err);font-size:12px;margin-top:3px;white-space:normal;font-family:"Noto Sans TC",system-ui,sans-serif}
notation-panel .np-kind{font-size:11px;color:var(--np-mute);white-space:nowrap}
notation-panel .np-code{font-family:"JetBrains Mono",ui-monospace,monospace;font-size:12px;color:var(--np-mute)}
notation-panel .np-clamp{display:block;max-width:var(--np-eq-max);overflow:hidden;white-space:nowrap}
notation-panel .np-clamp.over{-webkit-mask-image:linear-gradient(to right,#000 80%,transparent);mask-image:linear-gradient(to right,#000 80%,transparent)}
notation-panel .np-txt{display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;max-width:22em}
notation-panel th.np-ops,notation-panel td.np-ops{position:sticky;right:0;z-index:1;white-space:nowrap;box-shadow:-6px 0 6px -6px rgba(0,0,0,.25)}
notation-panel th.np-ops{z-index:3}
notation-panel .np-origin{font-size:10px;color:var(--np-mute);border:1px solid var(--np-line);border-radius:4px;padding:0 3px;margin-left:4px}
notation-panel .np-flag{display:inline-block;margin-left:4px;font-size:10px;font-weight:600;border-radius:8px;padding:0 5px;border:0;cursor:pointer;color:var(--np-on-warn);background:var(--np-warn)}
notation-panel .np-flag.error{background:var(--np-err)}
notation-panel .np-flag.info{background:none;color:var(--np-info);border:1px solid var(--np-info)}
notation-panel .np-issue{padding:7px 10px;border-bottom:1px solid var(--np-line);display:grid;grid-template-columns:auto minmax(0,1fr);gap:3px 8px}
notation-panel .np-issue.dismissed{opacity:.55}
notation-panel .np-sev{font-size:11px;font-weight:600;border-radius:4px;padding:0 5px;height:fit-content;margin-top:1px;white-space:nowrap}
notation-panel .np-sev.error{color:var(--np-on-warn);background:var(--np-err)}
notation-panel .np-sev.warning{color:var(--np-on-warn);background:var(--np-warn)}
notation-panel .np-sev.info{color:var(--np-info);border:1px solid var(--np-info)}
notation-panel .np-msg{grid-column:2;overflow-wrap:anywhere}
notation-panel .np-fix{grid-column:2;color:var(--np-mute);font-size:12px}
notation-panel .np-locs,notation-panel .np-acts{grid-column:2;display:flex;flex-wrap:wrap;gap:4px;align-items:center}
notation-panel .np-loc{border:0;background:var(--np-soft);color:var(--np-accent);border-radius:4px;padding:1px 6px;cursor:pointer;font:12px "JetBrains Mono",ui-monospace,monospace}
notation-panel .np-loc.rel{color:var(--np-mute)}
notation-panel .np-loc.more{font-family:inherit;color:var(--np-mute)}
notation-panel .np-loc small{font-family:"Noto Sans TC",system-ui,sans-serif;color:var(--np-mute);margin-left:4px}
notation-panel .np-details{grid-column:2;color:var(--np-mute);font-size:12px;margin:2px 0 0;padding-left:16px}
notation-panel .np-empty{padding:24px;text-align:center;color:var(--np-mute)}
notation-panel .np-toast{position:absolute;left:50%;bottom:12px;transform:translateX(-50%);z-index:5;display:flex;gap:10px;align-items:center;max-width:calc(100% - 24px);padding:7px 10px 7px 14px;border-radius:8px;background:var(--np-fg);color:var(--np-bg);box-shadow:0 6px 24px rgba(0,0,0,.25)}
notation-panel .np-toast .np-btn{background:transparent;color:inherit;border-color:currentColor}
notation-panel[data-narrow] thead{display:none}
notation-panel[data-narrow] table,notation-panel[data-narrow] tbody{display:block}
notation-panel[data-narrow] tr{display:flex;flex-wrap:wrap;align-items:baseline;gap:2px 8px;padding:8px 10px;border-bottom:1px solid var(--np-line)}
notation-panel[data-narrow] tr.sel{background:var(--np-hi)}
notation-panel[data-narrow] td{display:block;border:0;padding:1px 0;background:transparent;flex:1 1 100%;min-width:0}
notation-panel[data-narrow] td.np-kind{flex:0 0 auto;order:-1}
notation-panel[data-narrow] td.np-prim{flex:1 1 0;order:-1;font-size:15px}
notation-panel[data-narrow] td.blank{display:none}
notation-panel[data-narrow] td[data-label]:not(.np-prim):not(.np-kind):not(.np-ops)::before{content:attr(data-label);display:inline-block;min-width:5em;margin-right:6px;color:var(--np-mute);font-size:11px}
notation-panel[data-narrow] td.np-ops{position:static;box-shadow:none;text-align:right;padding-top:4px}
notation-panel[data-narrow] .np-clamp{--np-eq-max:100%;max-width:100%}
notation-panel[data-narrow] .np-txt{max-width:none;display:inline}
.np-tip{position:fixed;z-index:10000;max-width:min(640px,92vw);padding:8px 10px;border-radius:8px;background:var(--text,#1f2937);color:var(--bg,#f3f4f6);font:13px/1.45 "Noto Sans TC",system-ui,sans-serif;box-shadow:0 6px 24px rgba(0,0,0,.25);pointer-events:none;overflow:hidden}
.np-tip .np-tip-sym{font-size:18px;margin-bottom:4px;overflow:hidden}
.np-tip .np-tip-meta{opacity:.7;font-size:12px;margin-top:4px}
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

  /** Rough canonical form of a symbol's LaTeX (mirrors lore_notation::math::canonical for common cases). */
  function canon(latex) {
    if (!latex) return "";
    let s = String(latex).replace(/\s+/g, "");
    s = s.replace(/\\(bm|mathbf|mathbold|pmb)\{/g, "\\boldsymbol{").replace(/\\(bm|mathbf)([a-zA-Z])/g, "\\boldsymbol{$2}");
    s = s.replace(/\\widetilde/g, "\\tilde").replace(/\\widehat/g, "\\hat").replace(/\\overline/g, "\\bar");
    s = s.replace(/\\(tilde|hat|bar|vec|dot)(\\[a-zA-Z]+|[a-zA-Z])/g, "\\$1{$2}");
    s = s.replace(/([_^])(\\[a-zA-Z]+|[a-zA-Z0-9])/g, "$1{$2}");
    return s;
  }

  // ── macro names: the same rules as lore_notation::macros::check_macro_name ──
  const RESERVED = new Set(`begin end documentclass usepackage input include newcommand renewcommand providecommand def let edef
    gdef relax par item label ref pageref cite caption section subsection subsubsection paragraph
    subparagraph chapter part title author date maketitle thanks footnote emph textbf textit texttt
    textrm textsf textsc textup text mbox hbox vbox box makebox framebox parbox rule hspace vspace
    hfill vfill quad qquad newline linebreak pagebreak newpage clearpage centering raggedright raggedleft
    small large Large LARGE huge Huge tiny scriptsize footnotesize normalsize bf it rm sf tt sc
    em sl color textcolor url href the value arabic roman Roman alph Alph today LaTeX TeX
    and or if fi else do to in at of on by over atop above choose bibliography
    bibliographystyle tableofcontents appendix index glossary includegraphics graphicspath tabular hline cline
    multicolumn multirow toprule midrule bottomrule cmidrule eqref tag nonumber notag intertext ensuremath
    DeclareMathOperator operatorname mathop limits nolimits displaystyle textstyle scriptstyle left right
    middle big Big bigg Bigg frac dfrac tfrac sqrt binom sum prod int oint lim log ln exp
    sin cos tan max min sup inf arg det dim ker deg gcd Pr mod bmod pmod
    mathbf mathrm mathit mathcal mathbb mathfrak mathscr mathsf mathtt boldsymbol bm pmb hat widehat
    tilde widetilde bar overline underline vec dot ddot check breve acute grave mathring overbrace
    underbrace overrightarrow overleftarrow stackrel overset underset xrightarrow xleftarrow
    cdot cdots ldots dots vdots ddots times div pm mp ast star circ bullet oplus otimes odot
    cup cap setminus wedge vee neg lnot forall exists nexists emptyset varnothing infty partial
    nabla prime leq le geq ge neq ne approx sim simeq equiv cong propto subset subseteq supset
    supseteq notin ni mid parallel perp leftarrow rightarrow Leftarrow Rightarrow leftrightarrow
    Leftrightarrow mapsto gets uparrow downarrow langle rangle lceil rceil lfloor rfloor lbrace rbrace
    vert Vert top bot dagger ddagger ell hbar imath jmath aleph Re Im wp angle triangle
    square checkmark coloneqq triangleq S P AA aa ae AE o O l L i j c d b t u v H k r G ss`.split(/\s+/));
  const GREEK = new Set(`alpha beta gamma delta epsilon varepsilon zeta eta theta vartheta iota kappa varkappa
    lambda mu nu xi pi varpi rho varrho sigma varsigma tau upsilon phi varphi chi
    psi omega Gamma Delta Theta Lambda Xi Pi Sigma Upsilon Phi Psi Omega varGamma
    varDelta varTheta varLambda varXi varPi varSigma varUpsilon varPhi varPsi varOmega ell
    hbar imath jmath aleph digamma`.split(/\s+/));

  /** Why `name` cannot be a macro (null when it can). `others`: macros of the other rows; `existing`: the document's own commands. */
  function macroProblem(name, others = [], existing = []) {
    const n = String(name ?? "").trim().replace(/^\\/, "");
    if (!n) return null;
    if (!/^[A-Za-z]+$/.test(n)) return `\\${n} 只能用英文字母（LaTeX 指令名稱不能有數字或符號）`;
    if (RESERVED.has(n) || GREEK.has(n) || n.startsWith("text") || n.startsWith("math")) return `\\${n} 是 LaTeX 既有指令，不能覆蓋`;
    if ([...existing].includes(n)) return `\\${n} 已在文件中定義`;
    if (others.includes(n)) return `巨集 \\${n} 已用在對照表的另一列`;
    return null;
  }

  /** Macro map for KaTeX: {"\\speed": "u"} (same rules as lore_notation::katex_macros). */
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

  /** `\name` at the very end: a letter typed next would join the command name. */
  const endsWithControlWord = (s) => /(^|[^\\])(\\\\)*\\[A-Za-z]+$/.test(s);

  /** What 插入 puts at the caret (same rules as lore_notation::insert_text). */
  function insertText(e, mode, inMath) {
    const mac = mode === "macro" ? macroName(e) : null;
    // in math, `\speed` + a following letter would read as `\speedx`: end with a space
    const wrap = (s) => (inMath ? (endsWithControlWord(s) ? s + " " : s) : `$${s}$`);
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

  const detailField = (e) => {
    const k = e.kind ?? "symbol";
    return k === "abbrev" ? "long" : k === "figure" || k === "table" ? "caption" : "meaning_en";
  };

  // `always`: shown even when every row is empty (意義 falls back to the English meaning).
  const COLUMNS = [
    { key: "kind", title: "種類", always: true, sort: (e) => KINDS.indexOf(e.kind ?? "symbol") },
    { key: "primary", title: "符號／內容", always: true, sort: (e) => String(primary(e)[1] ?? "").toLowerCase() },
    { key: "macro", title: "巨集", val: (e) => macroName(e), sort: (e) => e.macro ?? "" },
    { key: "meaning", title: "意義", always: true, sort: (e) => e.meaning ?? e.meaning_en ?? "" },
    // the English meaning of a row without a Chinese one is already shown (muted) under 意義
    { key: "detail", title: "英文／全稱", val: (e) => (detailField(e) === "meaning_en" && !e.meaning && (e.kind ?? "symbol") !== "term" ? null : e[detailField(e)]), sort: (e) => e[detailField(e)] ?? "" },
    { key: "gloss", title: "註解", val: (e) => e.gloss, sort: (e) => e.gloss ?? "" },
    { key: "defined_at", title: "定義位置", val: (e) => e.defined_at, sort: (e) => e.defined_at ?? "" },
  ];

  const sameIgnore = (a, b) => a.code === b.code && (a.subject ?? null) === (b.subject ?? null);

  class NotationPanel extends HTMLElement {
    static get observedAttributes() { return ["src", "report-src", "mode", "readonly"]; }

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
      this._allColumns = false;
      this._showDismissed = false;
      this._onlyEntry = null;
      this._expanded = new Set();
      this.macros = {};
      this.actions = [];
      this.docMacros = [];
      this.inMath = () => false;
    }

    connectedCallback() {
      injectStyle();
      if (!this._built) this._build();
      this._render();
      if (!this._ro && window.ResizeObserver) {
        // card layout when the panel itself is narrow (a side pane), whatever the window size
        this._ro = new ResizeObserver(() => this._fit());
        this._ro.observe(this);
      }
      this._fit();
    }

    disconnectedCallback() { this._ro?.disconnect(); this._ro = null; }

    _fit() {
      const narrow = this.clientWidth > 0 && this.clientWidth < 600;
      if (narrow !== this.hasAttribute("data-narrow")) this.toggleAttribute("data-narrow", narrow);
    }

    attributeChangedCallback(name, _old, val) {
      if (name === "src" && val) this.load(val);
      if (name === "report-src" && val) this.loadReport(val);
      if (name === "mode" && val) this.mode = val;
      if (name === "readonly" && this._built) this._render();
    }

    get readOnly() { return this.hasAttribute("readonly"); }
    set readOnly(v) { this.toggleAttribute("readonly", !!v); }

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

    /** Why the macro of row `id` (or `name` for it) is not usable; null when fine. */
    macroProblem(id, name) {
      const e = this.entry(id);
      const n = name ?? macroName(e);
      const others = this._notation.entries.filter((x) => x.id !== id).map(macroName).filter(Boolean);
      return macroProblem(n, others, this.docMacros ?? []);
    }

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
      const issues = this._visibleIssues();
      const errs = issues.filter((i) => i.severity === "error").length;
      this._tabTable.innerHTML = "";
      this._tabTable.append("對照表", el("span", "np-badge", String(n)));
      this._tabChecks.innerHTML = "";
      this._tabChecks.append("校對", el("span", "np-badge" + (errs ? " err" : ""), String(issues.length)));
      this._tabTable.setAttribute("aria-selected", String(this._tab === "table"));
      this._tabChecks.setAttribute("aria-selected", String(this._tab === "checks"));
      if (this._tab === "table") this._renderTable(); else this._renderChecks();
    }

    // ── dismissed / conventional findings ────────────────────────────────
    _subjectsOf(i) { return i.subjects?.length ? i.subjects : [i.subject ?? null]; }

    /** A finding the author dismissed or whose subject is marked conventional. */
    _dismissed(i) {
      const ign = this._notation.ignored ?? [];
      const conv = this._notation.conventional ?? [];
      const subs = this._subjectsOf(i);
      if (subs.every((s) => ign.some((x) => x.code === i.code && (x.subject == null || x.subject === s)))) return true;
      // like the checker: a conventional abbreviation / symbol is quiet in every abbreviation and symbol check
      return ["abbrev", "symbol", "equation"].includes(i.category) && !i.subjects?.length && i.subject != null && conv.some((c) => c === i.subject || canon(c) === canon(i.subject));
    }

    _visibleIssues() { return (this._report?.issues ?? []).filter((i) => !this._dismissed(i)); }

    /** 忽略: never report this finding again (stored in notation.json → ignored). */
    dismiss(issue, undo = false) {
      const list = (this._notation.ignored ??= []);
      for (const s of this._subjectsOf(issue)) {
        const item = s == null ? { code: issue.code } : { code: issue.code, subject: s };
        const k = list.findIndex((x) => sameIgnore(x, item));
        if (undo && k >= 0) list.splice(k, 1);
        if (!undo && k < 0) list.push(item);
      }
      if (!list.length) delete this._notation.ignored;
      this._emit("notation-change", { id: null, field: "ignored", value: this._notation.ignored ?? [], entry: null, notation: this._notation });
      this._render();
    }

    /** 標為慣用: the abbreviation / symbol needs no definition (notation.json → conventional). */
    markConventional(subject, undo = false) {
      const list = (this._notation.conventional ??= []);
      const k = list.indexOf(subject);
      if (undo && k >= 0) list.splice(k, 1);
      if (!undo && k < 0) list.push(subject);
      if (!list.length) delete this._notation.conventional;
      this._emit("notation-change", { id: null, field: "conventional", value: this._notation.conventional ?? [], entry: null, notation: this._notation });
      this._render();
    }

    /** Entry ids → the visible findings about them. */
    _issuesByEntry() {
      const out = new Map();
      const add = (id, i) => { if (!out.has(id)) out.set(id, []); if (!out.get(id).includes(i)) out.get(id).push(i); };
      for (const i of this._visibleIssues()) {
        if (i.entry && this.entry(i.entry)) { add(i.entry, i); continue; }
        for (const s of this._subjectsOf(i)) {
          if (!s) continue;
          const e = this.findByLatex(s) ?? this._notation.entries.find((x) => x.short === s || x.label === s);
          if (e) add(e.id, i);
        }
      }
      return out;
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
      seg.title = "按「插入」時：用巨集（\\speed，改對照表一處，全文跟著變）或直接寫符號的 LaTeX（u）";
      for (const [m, label] of [["macro", "用巨集"], ["literal", "符號直接寫"]]) {
        const b = el("button", "", label);
        b.setAttribute("aria-pressed", String(this._mode === m));
        b.onclick = () => { this.mode = m; this.setAttribute("mode", m); };
        seg.append(b);
      }
      const add = el("button", "np-btn primary", "新增");
      add.onclick = () => this._add();
      if (this.readOnly) bar.append(q, seg); else bar.append(q, seg, add);
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

    /** Columns to show: empty ones (no row has a value) are hidden unless asked for. */
    _columns() {
      const all = this._notation.entries;
      const cols = COLUMNS.filter((c) => c.always || this._allColumns || all.some((e) => c.val(e)));
      return { cols, hidden: COLUMNS.length - cols.length };
    }

    _renderRows() {
      const body = this._body;
      body.textContent = "";
      const rows = this._rows();
      if (!rows.length) {
        body.append(el("div", "np-empty", this._notation.entries.length ? "沒有符合的項目" : "對照表是空的，按「新增」或從論文自動擷取"));
        return;
      }
      const { cols, hidden } = this._columns();
      if (hidden || this._allColumns) {
        const t = el("button", "np-btn quiet", this._allColumns ? "隱藏空白欄" : `顯示空白欄（${hidden}）`);
        t.title = this._allColumns ? "沒有任何內容的欄位收起來" : "有些欄位每一列都是空的，已收起來；要填寫時再打開";
        t.onclick = () => { this._allColumns = !this._allColumns; this._renderRows(); };
        this._bar.querySelector(".np-cols")?.remove();
        t.classList.add("np-cols");
        this._bar.append(t);
      } else this._bar.querySelector(".np-cols")?.remove();
      const macros = this.allMacros();
      const flags = this._issuesByEntry();
      const table = el("table");
      const head = el("tr");
      for (const c of [...cols, { key: "ops", title: "" }]) {
        const th = el("th", c.key === "ops" ? "np-ops" : "", c.title);
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
      for (const e of rows) tbody.append(this._row(e, macros, cols, flags.get(e.id) ?? []));
      table.append(thead, tbody);
      body.append(table);
      // fade the right edge of clamped equations that do not fit
      requestAnimationFrame(() => {
        for (const c of body.querySelectorAll(".np-clamp")) c.classList.toggle("over", c.scrollWidth > c.clientWidth + 1);
      });
    }

    _tip() { return (this._tipObj ??= new NotationTooltip(this)); }

    _row(e, macros, cols, issues) {
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
      const kindNow = e.kind ?? "symbol";
      const cell = (key, cls) => {
        const td = el("td", cls);
        td.dataset.label = COLUMNS.find((c) => c.key === key)?.title ?? "";
        return td;
      };
      const show = new Set(cols.map((c) => c.key));
      // kind (+ origin, + findings about this row)
      const kind = cell("kind", "np-kind");
      kind.append(KIND[kindNow] ?? kindNow);
      if (this._origin[e.id] === "global") kind.append(el("span", "np-origin", "全域"));
      const badMacro = macroName(e) ? this.macroProblem(e.id) : null;
      if (issues.length || badMacro) {
        const worst = badMacro || issues.some((i) => i.severity === "error") ? "error" : issues.some((i) => i.severity === "warning") ? "warning" : "info";
        const f = el("button", `np-flag ${worst}`, String(issues.length + (badMacro ? 1 : 0)));
        f.title = [badMacro, ...issues.map((i) => i.message.split(" — ")[0].replace(/\$/g, ""))].filter(Boolean).join("\n") + "\n（按一下看校對結果）";
        f.setAttribute("aria-label", `${issues.length + (badMacro ? 1 : 0)} 個校對問題`);
        f.onclick = () => { this._onlyEntry = e.id; this.showTab("checks"); };
        kind.append(f);
      }
      tr.append(kind);
      // primary
      const [pfield, pval] = primary(e);
      const p = cell("primary", "ed np-prim");
      if (["symbol", "operator", "equation"].includes(kindNow) && pval) {
        const box = el("span", "np-clamp");
        renderMath(box, pval, macros);
        p.append(box);
        // the full rendering on hover (long equations are clamped)
        p.onmouseenter = (ev) => { if (box.classList.contains("over") || kindNow === "equation") this._tip().show(ev.clientX, ev.clientY, e); };
        p.onmouseleave = () => this._tip().hide();
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
      if (show.has("macro")) {
        const m = cell("macro", "ed np-code");
        m.textContent = macroName(e) ? "\\" + macroName(e) : "";
        if (!macroName(e)) m.classList.add("blank");
        if (badMacro) {
          m.classList.add("bad-macro");
          m.title = badMacro;
          m.append(el("div", "np-why", badMacro));
        }
        this._editable(m, e, "macro");
        tr.append(m);
      }
      // 意義: Chinese meaning, else the English one (muted) so the column is never blank for nothing
      const mean = cell("meaning", "ed");
      if (e.meaning) mean.append(el("span", "np-txt", e.meaning));
      else if (e.meaning_en && kindNow !== "term") {
        const fb = el("span", "np-txt np-fallback", e.meaning_en);
        fb.title = "還沒有中文意義，先顯示英文（雙擊填寫中文）";
        mean.append(fb);
      } else mean.classList.add("blank");
      this._editable(mean, e, "meaning");
      tr.append(mean);
      if (show.has("detail")) {
        const df = detailField(e);
        const d = cell("detail", "ed");
        // already shown (muted) under 意義: do not repeat it
        if (df === "meaning_en" && !e.meaning && kindNow !== "term") d.classList.add("blank");
        else if (e[df]) { const t = el("span", "np-txt", e[df]); t.title = e[df]; d.append(t); } else d.classList.add("blank");
        this._editable(d, e, df);
        tr.append(d);
      }
      if (show.has("gloss")) {
        const g = cell("gloss", "ed");
        if (e.gloss) g.append(el("span", "np-txt", e.gloss)); else g.classList.add("blank");
        this._editable(g, e, "gloss");
        tr.append(g);
      }
      if (show.has("defined_at")) {
        const at = cell("defined_at", "ed np-code", e.defined_at ?? "");
        at.textContent = e.defined_at ?? "";
        if (!e.defined_at) at.classList.add("blank");
        this._editable(at, e, "defined_at");
        tr.append(at);
      }
      const ops = el("td", "np-ops");
      const ins = el("button", "np-btn", "插入");
      ins.title = this._mode === "macro" && macroName(e) ? `插入 \\${macroName(e)}` : "插入 LaTeX";
      ins.onmousedown = (ev) => ev.preventDefault(); // keep the editor's caret
      ins.onclick = () => this.insert(e.id);
      ops.append(ins);
      for (const a of this.actions ?? []) {
        if (a.show && !a.show(e)) continue;
        const ab = el("button", "np-btn", a.label);
        if (a.title) ab.title = a.title;
        ab.disabled = !!a.disabled?.(e);
        ab.onclick = () => { a.run?.(e); this._emit("notation-action", { action: a.id, id: e.id, entry: e }); };
        ops.append(" ", ab);
      }
      if (!this.readOnly && !e.readonly) {
        const del = el("button", "np-btn quiet", "刪除");
        del.title = "刪除這一列（可以復原）";
        del.onclick = () => this.remove(e.id);
        ops.append(" ", del);
      }
      tr.append(ops);
      return tr;
    }

    /** Delete a row; a toast offers 復原 for a few seconds. */
    remove(id) {
      const i = this._notation.entries.findIndex((x) => x.id === id);
      if (i < 0) return;
      const [e] = this._notation.entries.splice(i, 1);
      this._emit("notation-delete", { id: e.id, entry: e, notation: this._notation });
      this._render();
      const t = this._toast();
      const what = el("span");
      what.append("已刪除 ");
      const kindNow = e.kind ?? "symbol";
      if (e.latex && ["symbol", "operator", "equation"].includes(kindNow)) {
        const s = el("span", "np-clamp");
        s.style.display = "inline-block";
        s.style.verticalAlign = "bottom";
        renderMath(s, e.latex, this.allMacros());
        what.append(s);
      } else what.append(primary(e)[1] ?? e.id);
      const m = e.meaning ?? e.meaning_en ?? e.long;
      if (m) what.append(`（${m}）`);
      const undo = el("button", "np-btn", "復原");
      undo.onclick = () => {
        this._notation.entries.splice(Math.min(i, this._notation.entries.length), 0, e);
        this._emit("notation-add", { entry: e, notation: this._notation });
        t.remove();
        this._selected = e.id;
        this._render();
      };
      t.append(what, undo);
    }

    _toast() {
      this.querySelector(".np-toast")?.remove();
      clearTimeout(this._toastTimer);
      const t = el("div", "np-toast");
      t.setAttribute("role", "status");
      this.append(t);
      this._toastTimer = setTimeout(() => t.remove(), 8000);
      return t;
    }

    /** Emit notation-insert for an entry (also callable by the host). */
    insert(id) {
      const e = this.entry(id);
      if (!e) return;
      const mode = this._mode;
      this._emit("notation-insert", { id, entry: e, mode, math: insertText(e, mode, true), text: insertText(e, mode, false), inMath: !!this.inMath() });
    }

    /** Open the inline editor of a row's field (as a double click would). */
    edit(id, field) {
      const tr = this.querySelector(`tr[data-id="${CSS.escape(id)}"]`);
      const td = [...(tr?.querySelectorAll("td.ed") ?? [])].find((x) => x._field === field) ?? tr?.querySelector("td.ed");
      td?._edit?.();
    }

    _editable(td, e, field) {
      if (this.readOnly || e.readonly) return;
      td.title = td.title || "雙擊編輯";
      td._field = field;
      td._edit = () => {
        if (td.querySelector("input")) return;
        const input = el("input");
        const old = field === "macro" ? (macroName(e) ?? "") : (e[field] ?? "");
        input.value = old;
        input.setAttribute("aria-label", COLUMNS.find((c) => c.key === field)?.title ?? field);
        const why = el("div", "np-why");
        why.setAttribute("aria-live", "polite");
        td.textContent = "";
        td.append(input, why);
        td.classList.remove("blank");
        input.focus();
        input.select();
        const problem = () => (field === "macro" ? this.macroProblem(e.id, input.value) : null);
        const check = () => {
          const p = problem();
          input.classList.toggle("bad", !!p);
          why.textContent = p ?? "";
          return !p;
        };
        input.oninput = check;
        let done = false;
        const finish = (save) => {
          if (done) return;
          if (save && !check()) return; // keep the editor open with the reason under it
          done = true;
          if (save && input.value !== old) this.setField(e.id, field, input.value);
          else this._renderRows();
        };
        input.onkeydown = (ev) => {
          if (ev.key === "Enter") { ev.preventDefault(); finish(true); }
          if (ev.key === "Escape") { done = false; finish(false); }
        };
        // leaving an invalid name throws it away; a valid one is saved
        input.onblur = () => (check() ? finish(true) : finish(false));
      };
      td.ondblclick = td._edit;
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
      this.querySelector(`tr[data-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "center" });
      // a blank row: start typing its symbol / content right away
      if (!primary(e)[1]) this.edit(id, primary(e)[0]);
      else if (!e.meaning) this.edit(id, "meaning");
      return e;
    }

    // ── checks ───────────────────────────────────────────────────────────
    _renderChecks() {
      const bar = this._bar;
      bar.textContent = "";
      const all = this._report?.issues ?? [];
      const issues = this._visibleIssues();
      for (const s of ["error", "warning", "info"]) {
        const n = issues.filter((i) => i.severity === s).length;
        const b = el("button", "np-chip", `${SEV[s]} ${n}`);
        b.setAttribute("aria-pressed", String(this._sevFilter[s]));
        b.onclick = () => { this._sevFilter[s] = !this._sevFilter[s]; this._renderChecks(); };
        bar.append(b);
      }
      const cat = el("select", "np-btn");
      cat.setAttribute("aria-label", "類別");
      cat.append(new Option("全部類別", ""));
      for (const [k, v] of Object.entries(CAT)) cat.append(new Option(v, k));
      cat.value = this._cat ?? "";
      cat.onchange = () => { this._cat = cat.value; this._renderChecks(); };
      bar.append(cat);
      if (this._onlyEntry) {
        const e = this.entry(this._onlyEntry);
        const b = el("button", "np-chip", `只看：${e ? primary(e)[1] ?? e.id : this._onlyEntry} ✕`);
        b.setAttribute("aria-pressed", "true");
        b.title = "顯示全部";
        b.onclick = () => { this._onlyEntry = null; this._renderChecks(); };
        bar.append(b);
      }
      const hiddenN = all.length - issues.length;
      if (hiddenN) {
        const b = el("button", "np-btn quiet", this._showDismissed ? "收起已略過" : `已略過 ${hiddenN}`);
        b.title = "標為忽略或慣用的項目";
        b.onclick = () => { this._showDismissed = !this._showDismissed; this._renderChecks(); };
        bar.append(b);
      }
      const body = this._body;
      body.textContent = "";
      if (!this._report) {
        body.append(el("div", "np-empty", "尚未校對"));
        return;
      }
      const macros = this.allMacros();
      const mine = this._onlyEntry ? new Set(this._issuesByEntry().get(this._onlyEntry) ?? []) : null;
      const shown = (this._showDismissed ? all : issues).filter((i) => this._sevFilter[i.severity] && (!this._cat || i.category === this._cat) && (!mine || mine.has(i)));
      const bad = this._onlyEntry ? this.macroProblem(this._onlyEntry) : null;
      if (bad) body.append(this._issue({ code: "registry_invalid", category: "registry", severity: "error", message: `巨集：${bad}`, entry: this._onlyEntry }, macros));
      if (!shown.length && !bad) {
        body.append(el("div", "np-empty", all.length ? "沒有符合篩選的項目" : "沒有發現問題"));
        return;
      }
      for (const i of shown) body.append(this._issue(i, macros));
    }

    _issue(i, macros) {
      const box = el("div", "np-issue");
      const dismissed = this._dismissed(i);
      if (dismissed) box.classList.add("dismissed");
      box.append(el("span", `np-sev ${i.severity}`, SEV[i.severity] ?? i.severity));
      // "problem — how to fix it": the fix on its own line
      const [main, ...fix] = String(i.message ?? "").split(" — ");
      const msg = el("div", "np-msg");
      renderInline(msg, main, macros);
      box.append(msg);
      if (fix.length) {
        const f = el("div", "np-fix");
        renderInline(f, "建議：" + fix.join(" — "), macros);
        box.append(f);
      }
      // jump chips: one per line, the first and "+N" until expanded
      const key = `${i.code}|${i.subject ?? ""}|${i.locations?.[0]?.ord ?? ""}`;
      const uniq = (ls) => {
        const seen = new Set();
        return (ls ?? []).filter((l) => { const k = `${l.file}:${l.line}`; if (seen.has(k)) return false; seen.add(k); return true; });
      };
      const locs = uniq(i.locations);
      const rel = uniq(i.related).filter((r) => !locs.some((l) => l.file === r.file && l.line === r.line));
      const chip = (l, isRel) => {
        const b = el("button", "np-loc" + (isRel ? " rel" : ""), `${l.file}:${l.line}`);
        const ctx = CONTEXT[l.context] ?? l.context;
        if (ctx && l.context !== "body") b.append(el("small", "", ctx));
        b.title = `${isRel ? "相關位置" : "跳到"}：第 ${l.para + 1} 段第 ${l.para_sentence ?? l.sentence} 句（${ctx}）\n${l.snippet ?? ""}`;
        b.onclick = () => this._emit("notation-jump", { ...l, issue: i });
        return b;
      };
      if (locs.length || rel.length) {
        const row = el("div", "np-locs");
        const open = this._expanded.has(key);
        const firstN = 1;
        locs.slice(0, open ? locs.length : firstN).forEach((l) => row.append(chip(l, false)));
        if (!open && locs.length > firstN) {
          const more = el("button", "np-loc more", `+${locs.length - firstN}`);
          more.title = `還有 ${locs.length - firstN} 處，按一下全部列出`;
          more.onclick = () => { this._expanded.add(key); this._renderChecks(); };
          row.append(more);
        }
        rel.slice(0, open ? rel.length : 2).forEach((l) => row.append(chip(l, true)));
        if (!open && rel.length > 2) {
          const more = el("button", "np-loc more rel", `+${rel.length - 2}`);
          more.onclick = () => { this._expanded.add(key); this._renderChecks(); };
          row.append(more);
        }
        box.append(row);
      }
      if (i.details?.length) {
        const ul = el("ul", "np-details");
        for (const d of i.details) {
          const li = el("li");
          renderInline(li, d, macros);
          ul.append(li);
        }
        box.append(ul);
      }
      const acts = el("div", "np-acts");
      if (i.entry && this.entry(i.entry) && !this._onlyEntry) {
        const b = el("button", "np-btn", "在表中顯示");
        b.onclick = () => this.select(i.entry);
        acts.append(b);
      }
      if ((i.code === "undefined_symbol" || i.code === "unregistered_symbol") && i.subject && !this.findByLatex(i.subject)) {
        const b = el("button", "np-btn", "加入對照表");
        b.onclick = () => this._add({ kind: "symbol", latex: i.subject });
        acts.append(b);
      }
      if (i.code === "abbrev_never_expanded" && i.subject && !this._notation.entries.some((e) => e.short === i.subject)) {
        const b = el("button", "np-btn", "加入縮寫");
        b.onclick = () => this._add({ kind: "abbrev", short: i.subject, id: `abbr-${i.subject.toLowerCase()}` });
        acts.append(b);
      }
      if (i.code !== "registry_invalid" || i.locations?.length) {
        const conv = (this._notation.conventional ?? []).includes(i.subject);
        if (CONVENTIONAL_CODES.includes(i.code) && i.subject && (!dismissed || conv)) {
          const b = el("button", "np-btn quiet", conv ? "取消慣用" : "標為慣用（不再提醒）");
          b.title = conv ? "" : `「${i.subject}」是大家熟悉的寫法，不需要定義或展開；記在 notation.json，之後的校對不再提出`;
          b.onclick = () => this.markConventional(i.subject, conv);
          acts.append(b);
        }
        if (!conv) {
          const b = el("button", "np-btn quiet", dismissed ? "取消忽略" : "忽略");
          b.title = dismissed ? "" : "這一項不用改；記在 notation.json，之後的校對不再提出";
          b.onclick = () => this.dismiss(i, dismissed);
          acts.append(b);
        }
      }
      if (acts.childNodes.length) box.append(acts);
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
  NotationPanel.macroProblem = macroProblem;
  NotationPanel.KIND = KIND;
  window.NotationPanel = NotationPanel;
  window.NotationTooltip = NotationTooltip;
  if (!customElements.get("notation-panel")) customElements.define("notation-panel", NotationPanel);
})();
