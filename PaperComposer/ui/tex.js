// Preview of LaTeX: math through KaTeX, text commands as HTML, and the
// elements a paper inserts (figures, tables, lists, theorems, algorithms)
// as blocks. Numbers for labels and citations come from the last compile
// when there is one, so they match the PDF; otherwise they are counted here.
// This is a reading aid for the editor; the submitted PDF is always built by
// real LaTeX with the publisher's class.

const MATH_ENVS = ["equation", "align", "gather", "multline", "eqnarray", "displaymath", "math", "flalign", "alignat"];

window.PCTex = (() => {
  let macros = {};
  let labels = {};   // label -> printed number
  let kinds = {};    // label -> "fig" | "tab" | "eq" | "sec" | "thm" | "alg"
  let cites = {};    // key -> { n, author?, year? }
  let compiled = {}; // labels from the last compile
  let figureBase = "";

  // Font packages KaTeX does not know, mapped to what it does.
  const BUILTIN = {
    "\\mathbbmss": "\\mathbb", "\\mathbbm": "\\mathbb", "\\mathds": "\\mathbb", "\\bbm": "\\mathbb",
    "\\bm": "\\boldsymbol", "\\Checkmark": "\\checkmark", "\\XSolid": "\\times", "\\mathbold": "\\boldsymbol",
    "\\textsc": "\\text", "\\up": "\\text", "\\degree": "^\\circ", "\\ensuremath": "",
  };

  function setMacros(m) { macros = { ...BUILTIN, ...m }; }

  /** Numbers from the server: { labels, cites, compiled }. */
  function setNumbers(n) {
    compiled = n?.labels ?? {};
    cites = n?.cites ?? {};
  }
  function setFigureBase(url) { figureBase = url; }

  // ── Numbering ───────────────────────────────────────
  const FLOAT_KIND = { figure: "fig", "figure*": "fig", wrapfigure: "fig", table: "tab", "table*": "tab", sidewaystable: "tab", algorithm: "alg", "algorithm*": "alg" };
  const THEOREMS = ["theorem", "lemma", "proposition", "corollary", "definition", "remark", "assumption", "example", "conjecture", "claim", "problem"];
  const THM_NAME = { theorem: "Theorem", lemma: "Lemma", proposition: "Proposition", corollary: "Corollary", definition: "Definition", remark: "Remark", assumption: "Assumption", example: "Example", conjecture: "Conjecture", claim: "Claim", problem: "Problem", proof: "Proof" };

  /** Count labels in reading order; a compile's numbers win where known.
   *  `sections` is [{ title, paragraphs }]. */
  function number(sections, front = "") {
    labels = {};
    kinds = {};
    const count = { eq: 0, fig: 0, tab: 0, alg: 0, thm: 0 };
    let sec = 0, sub = 0, subsub = 0;
    const put = (key, n, kind) => { kinds[key] = kind; labels[key] = compiled[key] ?? String(n); };
    // the title block comes first: a figure there is Fig. 1
    const all = front.trim() ? [{ front: true, paragraphs: [front] }, ...sections] : sections;
    all.forEach((s) => {
      if (!s.front) { sec++; sub = 0; subsub = 0; }
      (s.tail?.match(/\\label\{([^}]*)\}/g) ?? []).forEach((l) => put(l.slice(7, -1), sec, "sec"));
      s.paragraphs.forEach((raw) => {
        const p = stripComments(raw);
        // walk environments and headings in order
        const re = /\\(subsubsection|subsection)\*?\{|\\begin\{([a-zA-Z*]+)\}|\\label\{([^}]*)\}|\\\\|\\(nonumber|notag)\b|\\end\{([a-zA-Z*]+)\}/g;
        const stack = [];
        let pending = null;
        for (const m of p.matchAll(re)) {
          if (m[1] === "subsection") { sub++; subsub = 0; pending = { kind: "sec", n: `${sec}.${sub}` }; }
          else if (m[1] === "subsubsection") { subsub++; pending = { kind: "sec", n: `${sec}.${sub}.${subsub}` }; }
          else if (m[2]) {
            const env = m[2], base = env.replace("*", "");
            const frame = { env, n: null, kind: null, numbered: false };
            if (FLOAT_KIND[env]) { frame.kind = FLOAT_KIND[env]; frame.n = ++count[frame.kind]; }
            else if (MATH_ENVS.includes(base) && !env.endsWith("*") && base !== "math" && base !== "displaymath") {
              frame.kind = "eq"; frame.n = ++count.eq; frame.numbered = true; frame.multi = ["align", "gather", "eqnarray", "flalign", "alignat"].includes(base);
            } else if (THEOREMS.includes(base)) { frame.kind = "thm"; frame.n = ++count.thm; }
            stack.push(frame);
            pending = null;
          } else if (m[3]) {
            const top = [...stack].reverse().find((f) => f.kind);
            if (top) put(m[3], top.n, top.kind);
            else if (pending) put(m[3], pending.n, pending.kind);
          } else if (m[0] === "\\\\") {
            // next row of a multi-line equation gets its own number
            const top = stack[stack.length - 1];
            if (top?.multi && top.numbered) { top.n = ++count.eq; }
            else if (top?.multi) { top.numbered = true; top.n = ++count.eq; }
          } else if (m[4]) {
            const top = stack[stack.length - 1];
            if (top?.multi && top.numbered) { count.eq--; top.numbered = false; }
            else if (top?.kind === "eq" && top.numbered) { count.eq--; top.numbered = false; }
          } else if (m[5]) {
            stack.pop();
          }
        }
      });
    });
    return labels;
  }

  // Running counters while the page renders in reading order: a labelled
  // item takes its known number (and moves the counter there), others count on.
  const ctr = { eq: 0, fig: 0, tab: 0, alg: 0, thm: 0 };
  function resetCounters() { Object.keys(ctr).forEach((k) => (ctr[k] = 0)); }
  const ROMAN = [[1000, "M"], [900, "CM"], [500, "D"], [400, "CD"], [100, "C"], [90, "XC"], [50, "L"], [40, "XL"], [10, "X"], [9, "IX"], [5, "V"], [4, "IV"], [1, "I"]];
  const toRoman = (n) => ROMAN.reduce((s, [v, r]) => { while (n >= v) { s += r; n -= v; } return s; }, "");
  const fromRoman = (r) => { let n = 0, i = 0; for (const [v, sym] of ROMAN) while (r.startsWith(sym, i)) { n += v; i += sym.length; } return i === r.length ? n : NaN; };
  function next(kind, key) {
    const known = key ? compiled[key] ?? null : null;
    if (known) {
      const v = /^\d+$/.test(known) ? +known : fromRoman(known);
      if (!Number.isNaN(v)) ctr[kind] = v; else ctr[kind]++;
      return known;
    }
    ctr[kind]++;
    // the class numbers this kind in Roman (IEEE tables): follow it
    const roman = Object.entries(compiled).some(([k, v]) => kinds[k] === kind && /^[IVXL]+$/.test(v));
    return roman ? toRoman(ctr[kind]) : String(ctr[kind]);
  }

  // ── Source helpers ──────────────────────────────────
  // Code shown as typed: \verb|…| and verbatim-like environments. A % inside
  // is code, not a comment.
  const CODE_ENVS = "verbatim\\*?|Verbatim|BVerbatim|lstlisting|minted|alltt";
  const VERBATIM_RE = new RegExp(`\\\\begin\\{(${CODE_ENVS})\\}[\\s\\S]*?\\\\end\\{\\1\\}|\\\\verb\\*?([^a-zA-Z\\s*])[^\\n]*?\\2`, "g");
  /** Drop % comments (and the line break they swallow), keep \%. */
  function stripComments(t) {
    const strip = (s) => s.replace(/(?<!\\)%[^\n]*(\n[ \t]*)?/g, "");
    let out = "", last = 0;
    for (const m of t.matchAll(VERBATIM_RE)) {
      out += strip(t.slice(last, m.index)) + m[0];
      last = m.index + m[0].length;
    }
    return out + strip(t.slice(last));
  }
  /** \begin{lstlisting}…: a <pre> of the code as typed. */
  function codeNode(env, body) {
    let b = body;
    if (env !== "verbatim" && env !== "verbatim*" && env !== "alltt") { const o = optional(b, 0); if (o) b = b.slice(o[1]); }
    if (env === "minted") { const g = required(b, 0); if (g) b = b.slice(g[1]); }
    b = b.replace(/^[ \t]*\n/, "").replace(/\n[ \t]*$/, "");
    const pre = el("pre", "tex-code");
    const code = el("code");
    if (env === "lstlisting") {
      // escapeinside (*…*) holds LaTeX, usually comments
      b.split(/\(\*([\s\S]*?)\*\)/).forEach((part, x) => code.append(x % 2 ? render(part) : document.createTextNode(part)));
    } else code.textContent = b;
    pre.append(code);
    return pre;
  }

  // Read a balanced {group} starting at s[i] === "{". Returns [content, nextIndex].
  function group(s, i) {
    let depth = 0;
    for (let j = i; j < s.length; j++) {
      if (s[j] === "\\") { j++; continue; }
      if (s[j] === "{") depth++;
      else if (s[j] === "}" && --depth === 0) return [s.slice(i + 1, j), j + 1];
    }
    return [s.slice(i + 1), s.length];
  }
  // Optional [arg] at s[i] (spaces skipped). Returns [content, nextIndex] or null.
  function optional(s, i) {
    let j = i;
    while (s[j] === " ") j++;
    if (s[j] !== "[") return null;
    let depth = 0;
    for (let k = j; k < s.length; k++) {
      if (s[k] === "{") depth++;
      else if (s[k] === "}") depth--;
      else if (s[k] === "]" && depth === 0) return [s.slice(j + 1, k), k + 1];
    }
    return null;
  }
  // {arg} at s[i] after spaces and line breaks.
  function required(s, i) {
    let j = i;
    while (s[j] === " " || s[j] === "\n") j++;
    return s[j] === "{" ? group(s, j) : null;
  }
  /** Body of \begin{env}...\end{env} starting at i (just after \begin{env}); nested same-name envs balanced. */
  function envBody(s, i, env) {
    const open = `\\begin{${env}}`, close = `\\end{${env}}`;
    let depth = 1, j = i;
    while (depth > 0) {
      const a = s.indexOf(open, j), b = s.indexOf(close, j);
      if (b < 0) return [s.slice(i), s.length];
      if (a >= 0 && a < b) { depth++; j = a + open.length; }
      else { depth--; j = b + close.length; if (depth === 0) return [s.slice(i, b), j]; }
    }
    return [s.slice(i), s.length];
  }

  const el = (tag, cls, text) => { const n = document.createElement(tag); if (cls) n.className = cls; if (text != null) n.textContent = text; return n; };

  // ── Math ────────────────────────────────────────────
  function katexNode(src, display) {
    const span = document.createElement(display ? "div" : "span");
    span.className = display ? "math display" : "math";
    src = stripComments(src);
    if (!window.katex) { span.textContent = src; return span; }
    try {
      katex.render(src, span, { displayMode: display, throwOnError: false, macros: { ...macros }, strict: "ignore", trust: false });
    } catch {
      span.textContent = src;
    }
    return span;
  }

  function envNode(env, body) {
    const base = env.replace("*", "");
    const star = env.endsWith("*");
    const src = stripComments(body);
    const rows = src.split(/\\\\(?:\[[^\]]*\])?/);
    if (["align", "eqnarray", "flalign", "alignat", "gather"].includes(base)) {
      // KaTeX tags rows of align/gather; every row gets its number or \notag
      // (an untagged row would be numbered by KaTeX's own counter).
      const out = base === "gather" ? "gather" : "align";
      const tagged = rows.filter((r, x) => r.trim() || x < rows.length - 1).map((r) => {
        const lab = r.match(/\\label\{([^}]*)\}/);
        let clean = r.replace(/\\label\{[^}]*\}/g, "").replace(/\\nonumber|\\notag/g, "");
        if (base === "eqnarray") clean = clean.replace(/&\s*([=<>]|\\[a-zA-Z]+)\s*&/, "&$1 ");
        if (base === "alignat") clean = clean.replace(/^\s*\{\d+\}/, "");
        const num = !star && !/\\nonumber|\\notag/.test(r) ? next("eq", lab?.[1]) : null;
        return `${clean}${num ? `\\tag{${num}}` : "\\notag"}`;
      });
      return katexNode(`\\begin{${out}}${tagged.join("\\\\\n")}\n\\end{${out}}`, true);
    }
    const lab = src.match(/\\label\{([^}]*)\}/);
    let inner = src.replace(/\\label\{[^}]*\}/g, "");
    const num = !star && base !== "math" && base !== "displaymath" && !/\\nonumber|\\notag/.test(inner) ? next("eq", lab?.[1]) : null;
    if (base === "multline") inner = `\\begin{gathered}${inner}\n\\end{gathered}`;
    const tagged = num ? `${inner}\n\\tag{${num}}` : inner;
    return katexNode(tagged, true);
  }

  // ── Citations and references ────────────────────────
  function compress(nums) {
    const n = [...new Set(nums)].sort((a, b) => a - b);
    const out = [];
    for (let i = 0; i < n.length; i++) {
      let j = i;
      while (j + 1 < n.length && n[j + 1] === n[j] + 1) j++;
      out.push(j - i >= 2 ? `${n[i]}–${n[j]}` : n.slice(i, j + 1).join(", "));
      i = j;
    }
    return out.join(", ");
  }

  function citeNode(cmd, keys, note) {
    const n = el("span", "cite");
    const found = keys.map((k) => cites[k]);
    n.title = keys.join(", ");
    if (found.some((c) => !c)) n.classList.add("missing");
    const authorYear = found.length && found.every((c) => c?.author);
    if (authorYear) {
      const one = (c) => cmd === "citet" || cmd === "textcite" ? `${c.author} (${c.year})`
        : cmd === "citeauthor" ? c.author : cmd === "citeyear" ? c.year : `${c.author}, ${c.year}`;
      const body = found.map(one).join("; ");
      n.textContent = cmd === "citet" || cmd === "textcite" || cmd === "citeauthor" || cmd === "citeyear" || cmd === "citealp" ? body
        : `(${body}${note ? `, ${note}` : ""})`;
      return n;
    }
    const nums = found.map((c, i) => c?.n ?? keys[i]);
    const ints = nums.every((x) => /^\d+$/.test(x));
    n.textContent = `[${ints ? compress(nums.map(Number)) : nums.join(", ")}${note ? `, ${note}` : ""}]`;
    return n;
  }

  const REF_PREFIX = { fig: "Fig.", tab: "Table", eq: "Eq.", sec: "Section", alg: "Algorithm", thm: "Theorem" };
  function refNode(cmd, key) {
    const n = el("span", "ref");
    const num = labels[key] ?? compiled[key];
    n.title = key;
    if (!num) { n.classList.add("missing"); n.textContent = `[${key}]`; return n; }
    const kind = kinds[key] ?? (key.match(/^(fig|tab|eq|sec|alg|thm)/)?.[1]);
    if (cmd === "eqref") n.textContent = `(${num})`;
    else if (/^(autoref|cref|Cref)$/.test(cmd)) n.textContent = `${REF_PREFIX[kind] ?? ""} ${kind === "eq" ? `(${num})` : num}`.trim();
    else n.textContent = num;
    return n;
  }

  // ── Blocks ──────────────────────────────────────────
  function figureNode(env, body) {
    const fig = el("figure", "tex-figure" + (env.endsWith("*") ? " wide" : ""));
    const imgs = el("div", "imgs");
    for (const m of body.matchAll(/\\includegraphics\s*(\[[^\]]*\])?\s*\{([^}]*)\}/g)) {
      const w = m[1]?.match(/width\s*=\s*([\d.]*)\s*\\(textwidth|columnwidth|linewidth)/);
      const box = el("div", "img");
      const img = el("img");
      img.alt = m[2];
      img.loading = "lazy";
      img.src = `${figureBase}?path=${encodeURIComponent(m[2].trim())}`;
      img.onerror = () => { box.classList.add("missing"); box.textContent = m[2]; };
      if (w) box.style.width = `${Math.min(100, Math.round(parseFloat(w[1] || "1") * (w[2] === "textwidth" && !env.endsWith("*") ? 100 : 100)))}%`;
      box.append(img);
      imgs.append(box);
    }
    if (/\\begin\{(tikzpicture|axis)\}/.test(body)) imgs.append(el("div", "img missing tikz", "TikZ"));
    // tables inside a figure (rare) and subfigure captions are kept as text
    fig.append(imgs);
    const cap = captionOf(body);
    if (cap) fig.append(captionNode(env, cap, body));
    return fig;
  }

  function captionOf(body) {
    // the last \caption at this level: subfigure captions come first
    let at = -1, i = 0;
    while ((i = body.indexOf("\\caption", i)) >= 0) {
      if (!/^\\caption(of|setup|width)/.test(body.slice(i))) at = i;
      i += 8;
    }
    if (at < 0) return null;
    let j = at + "\\caption".length;
    const opt = optional(body, j);
    if (opt) j = opt[1];
    const arg = required(body, j);
    return arg ? arg[0] : null;
  }

  function captionNode(env, cap, body) {
    const kind = FLOAT_KIND[env] ?? "fig";
    const lab = body.match(/\\label\{([^}]*)\}/g)?.pop()?.slice(7, -1);
    const num = next(kind === "fig" ? "fig" : kind, lab);
    const c = el(kind === "tab" ? "div" : "figcaption", "caption");
    c.append(el("b", "", `${kind === "tab" ? "Table" : kind === "alg" ? "Algorithm" : "Fig."}${num ? ` ${num}` : ""}`), ". ");
    c.append(render(cap));
    return c;
  }

  /** A tabular as an HTML table: rows, cells, spans and rules. */
  function tabularNode(src) {
    const t = el("table", "tex-tabular");
    let s = stripComments(src);
    const begin = s.match(/\\begin\{(tabular\*?|tabularx|longtable|tabulary)\}/);
    if (!begin) return null;
    let j = s.indexOf(begin[0]) + begin[0].length;
    if (begin[1] === "tabular*" || begin[1] === "tabularx" || begin[1] === "tabulary") { const w = required(s, j); if (w) j = w[1]; }
    const pos = optional(s, j); if (pos) j = pos[1];
    const spec = required(s, j); if (spec) j = spec[1];
    const [body] = envBody(s, j, begin[1]);
    let ruleNext = false, row = null;
    const rows = body.split(/\\\\(?:\[[^\]]*\])?|\\tabularnewline/);
    rows.forEach((r) => {
      let text = r;
      // rules before the row
      const ruleRe = /^\s*(\\(hline|toprule|midrule|bottomrule|botrule|colrule|specialrule\s*\{[^}]*\}\s*\{[^}]*\}\s*\{[^}]*\}|cline\s*\{[^}]*\}|cmidrule\s*(\([^)]*\))?\s*\{[^}]*\}|hhline\s*\{[^}]*\}|rule\s*\{[^}]*\}\s*\{[^}]*\})\s*)+/;
      const m = text.match(ruleRe);
      if (m) { ruleNext = true; text = text.slice(m[0].length); if (row) row.classList.add("rule-below"); }
      if (!text.trim()) return;
      row = el("tr");
      if (ruleNext) row.classList.add("rule-above");
      ruleNext = false;
      splitCells(text).forEach((cell) => {
        let content = cell.trim(), span = 1, rowspan = 1;
        const mc = content.match(/^\\multicolumn\s*\{(\d+)\}\s*\{[^}]*\}\s*/);
        if (mc) { span = +mc[1]; const g = required(content, mc[0].length); content = g ? g[0] : ""; }
        const mr = content.match(/^\\multirow\s*(\[[^\]]*\])?\s*\{(\d+)\}\s*(\[[^\]]*\])?\s*\{[^}]*\}\s*(\[[^\]]*\])?\s*/);
        if (mr) { rowspan = Math.max(1, +mr[2]); const g = required(content, mr[0].length); content = g ? g[0] : ""; }
        const td = el("td");
        if (span > 1) td.colSpan = span;
        if (rowspan > 1) td.dataset.rowspan = rowspan;
        td.append(render(content));
        row.append(td);
      });
      t.append(row);
    });
    if (ruleNext && row) row.classList.add("rule-below");
    return t;
  }

  // Split a row on & outside braces and math.
  function splitCells(r) {
    const out = [];
    let depth = 0, math = false, cur = "";
    for (let i = 0; i < r.length; i++) {
      const c = r[i];
      if (c === "\\") { cur += c + (r[i + 1] ?? ""); i++; continue; }
      if (c === "{") depth++;
      else if (c === "}") depth--;
      else if (c === "$") math = !math;
      else if (c === "&" && depth === 0 && !math) { out.push(cur); cur = ""; continue; }
      cur += c;
    }
    out.push(cur);
    return out;
  }

  function tableNode(env, body) {
    const box = el("div", "tex-table" + (env.endsWith("*") ? " wide" : ""));
    const cap = captionOf(body);
    const captionFirst = cap && body.indexOf("\\caption") < body.search(/\\begin\{tabular/);
    if (cap && captionFirst) box.append(captionNode(env, cap, body));
    const tab = tabularNode(body);
    if (tab) box.append(tab);
    const notes = body.match(/\\begin\{tablenotes\}([\s\S]*?)\\end\{tablenotes\}/);
    if (notes) box.append(listNode("itemize", notes[1], "notes"));
    if (cap && !captionFirst) box.append(captionNode(env, cap, body));
    return box;
  }

  function listNode(env, body, cls = "") {
    const list = el(env === "enumerate" ? "ol" : "ul", cls);
    const items = stripComments(body).split(/\\item\b/).slice(1);
    items.forEach((it) => {
      const li = el("li");
      let text = it;
      const opt = optional(text, 0);
      if (opt) { li.classList.add("labelled"); li.append(el("b", "", opt[0].replace(/[{}]/g, "") + " ")); text = text.slice(opt[1]); }
      li.append(render(text.trim()));
      list.append(li);
    });
    return list;
  }

  function theoremNode(env, body, opt) {
    const base = env.replace("*", "");
    const box = el("div", `tex-thm ${base}`);
    const lab = body.match(/\\label\{([^}]*)\}/)?.[1];
    const num = base !== "proof" ? next("thm", lab) : null;
    const head = el("b", "", `${THM_NAME[base] ?? base}${num ? ` ${num}` : ""}`);
    if (opt) { head.append(" ("); head.append(render(opt)); head.append(")"); }
    head.append(". ");
    if (base === "proof") head.className = "proof-head";
    const p = el("div");
    p.append(head, render(body.trim()));
    if (base === "proof") p.append(el("span", "qed", " ∎"));
    box.append(p);
    return box;
  }

  function algorithmNode(env, body) {
    const box = el("div", "tex-alg");
    const cap = captionOf(body);
    if (cap) box.append(captionNode(env, cap, body));
    const lines = el("ol", "alg-lines");
    const KEY = { STATE: "", Statex: "", State: "", REQUIRE: "Require:", Require: "Require:", ENSURE: "Ensure:", Ensure: "Ensure:", INPUT: "Input:", OUTPUT: "Output:", RETURN: "return", Return: "return",
      FOR: "for", For: "for", ENDFOR: "end for", EndFor: "end for", IF: "if", If: "if", ELSIF: "else if", ElsIf: "else if", ELSE: "else", Else: "else", ENDIF: "end if", EndIf: "end if",
      WHILE: "while", While: "while", ENDWHILE: "end while", EndWhile: "end while", FORALL: "for all", ForAll: "for all", REPEAT: "repeat", Repeat: "repeat", UNTIL: "until", Until: "until", COMMENT: "▷", Comment: "▷", Function: "function", EndFunction: "end function", Procedure: "procedure", EndProcedure: "end procedure" };
    const OPENS = /^(FOR|For|IF|If|WHILE|While|FORALL|ForAll|REPEAT|Repeat|Function|Procedure|ELSE|Else|ELSIF|ElsIf)$/;
    const CLOSES = /^(ENDFOR|EndFor|ENDIF|EndIf|ENDWHILE|EndWhile|UNTIL|Until|EndFunction|EndProcedure|ELSE|Else|ELSIF|ElsIf)$/;
    const inner = stripComments(body).replace(/\\caption\s*\{/, "\\PCcaption{").replace(/\\label\{[^}]*\}/g, "");
    const re = /\\([A-Z][A-Za-z]*)\b/g;
    const parts = [];
    let m, last = null;
    while ((m = re.exec(inner))) {
      if (!(m[1] in KEY)) continue;
      if (last) parts.push({ ...last, text: inner.slice(last.end, m.index) });
      last = { cmd: m[1], end: m.index + m[0].length };
    }
    if (last) parts.push({ ...last, text: inner.slice(last.end).replace(/\\end\{algorithmic\}[\s\S]*/, "") });
    let depth = 0;
    parts.forEach(({ cmd, text }) => {
      if (CLOSES.test(cmd)) depth = Math.max(0, depth - 1);
      const li = el("li");
      li.style.paddingLeft = `${depth * 1.2}em`;
      let t = text.trim();
      const cond = t.startsWith("{") ? group(t, 0) : null;
      if (KEY[cmd]) li.append(el("b", "", KEY[cmd] + " "));
      if (cond) { li.append(render(cond[0])); t = t.slice(cond[1]); if (/^(FOR|For|WHILE|While|FORALL|ForAll)$/.test(cmd)) li.append(el("b", "", " do")); if (/^(IF|If|ELSIF|ElsIf)$/.test(cmd)) li.append(el("b", "", " then")); }
      li.append(render(t.replace(/\\end\{algorithmic\}|\\begin\{algorithmic\}(\[[^\]]*\])?/g, "")));
      lines.append(li);
      if (OPENS.test(cmd)) depth++;
    });
    box.append(lines);
    return box;
  }

  // ── Text ────────────────────────────────────────────
  const SYMBOLS = {
    "\\%": "%", "\\&": "&", "\\_": "_", "\\#": "#", "\\$": "$", "\\{": "{", "\\}": "}", "\\ldots": "…", "\\dots": "…", "\\textellipsis": "…",
    "\\LaTeX": "LaTeX", "\\TeX": "TeX", "\\\\": "\n", "\\pm": "±", "\\times": "×", "\\Checkmark": "✓", "\\checkmark": "✓", "\\XSolid": "✗", "\\ding{51}": "✓", "\\ding{55}": "✗",
    "\\textdegree": "°", "\\degree": "°", "\\textbackslash": "\\", "\\S": "§", "\\dag": "†", "\\ddag": "‡", "\\copyright": "©", "\\textregistered": "®", "\\texttrademark": "™",
    "\\textendash": "–", "\\textemdash": "—", "\\textasciitilde": "~", "\\textbullet": "•", "\\ss": "ß", "\\o": "ø", "\\O": "Ø", "\\ae": "æ", "\\AE": "Æ", "\\aa": "å", "\\l": "ł", "\\L": "Ł", "\\i": "ı", "\\OE": "Œ", "\\oe": "œ",
    "\\,": " ", "\\;": " ", "\\:": " ", "\\ ": " ", "\\@": "", "\\/": "", "\\-": "", "\\textless": "<", "\\textgreater": ">", "\\textmu": "µ", "\\textperthousand": "‰",
  };
  const SYMBOL_KEYS = Object.keys(SYMBOLS).sort((a, b) => b.length - a.length);
  const ACCENTS = { "'": "́", "`": "̀", "^": "̂", '"': "̈", "~": "̃", "c": "̧", "v": "̌", "u": "̆", "=": "̄", ".": "̇", "H": "̋", "k": "̨" };

  // Commands dropped together with their arguments (layout only).
  const DROP = { vspace: 1, hspace: 1, "vspace*": 1, "hspace*": 1, label: 1, setlength: 2, addtolength: 2, captionsetup: 1, color: 1, rule: 2, phantom: 1, hphantom: 1, vphantom: 1,
    bibliographystyle: 1, nocite: 1, pagestyle: 1, thispagestyle: 1, enlargethispage: 1, renewcommand: 2, newcommand: 2, linespread: 1, fontsize: 2, arraystretch: 0, IEEEpeerreviewmaketitle: 0,
    bstctlcite: 1, marginpar: 1, todo: 1, index: 1, glsresetall: 0, specialrule: 3, addlinespace: 0, vskip: 0, hskip: 0, centering: 0, raggedright: 0, raggedleft: 0, noindent: 0, indent: 0,
    clearpage: 0, newpage: 0, pagebreak: 0, linebreak: 0, nolinebreak: 0, hfill: 0, vfill: 0, medskip: 0, bigskip: 0, smallskip: 0, par: 0, protect: 0, relax: 0, null: 0, leavevmode: 0,
    tiny: 0, scriptsize: 0, footnotesize: 0, small: 0, normalsize: 0, large: 0, Large: 0, LARGE: 0, huge: 0, Huge: 0, selectfont: 0, bfseries: 0, itshape: 0, rmfamily: 0, sffamily: 0, ttfamily: 0, normalfont: 0,
    markboth: 2, markright: 1, IEEEpubid: 1, IEEEoverridecommandlockouts: 0, IEEEpubidadjcol: 0, IEEEdisplaynontitleabstractindextext: 0, IEEEaftertitletext: 1,
    titlerunning: 1, authorrunning: 1, date: 1, newblock: 0, BibitemOpen: 0, BibitemShut: 1, urlstyle: 1, appendices: 0, backmatter: 0, bibliography: 1, printbibliography: 0,
    maketitle: 0, appendix: 0, frenchspacing: 0, sloppy: 0, balance: 0, IEEEPARstart: 0, hline: 0, toprule: 0, midrule: 0, bottomrule: 0,
    lstset: 1, lstdefinestyle: 2, lstloadlanguages: 1, definecolor: 3, hypersetup: 1, sisetup: 1, graphicspath: 1, usetikzlibrary: 1, theoremstyle: 1 };
  // Commands whose last argument is shown as text.
  const PASS = { textrm: 1, textsf: 1, textnormal: 1, textup: 1, textmd: 1, text: 1, mbox: 1, hbox: 1, fbox: 1, centerline: 1, makebox: 1, framebox: 1, parbox: 2, resizebox: 3, scalebox: 2, rotatebox: 2, raisebox: 2,
    adjustbox: 2, NoHyper: 1, fnm: 1, sur: 1, spfx: 1, orgdiv: 1, orgname: 1, orgaddress: 1, street: 1, city: 1, postcode: 1, state: 1, country: 1, bibinfo: 2, bibfield: 2, natexlab: 1, textsuperscript: 1, IEEEauthorrefmark: 1, acrshort: 1, acrlong: 1, acrfull: 1, gls: 1, ac: 1, acp: 1, acs: 1, acl: 1, acf: 1, sethlcolor: 1 };

  /** Render a LaTeX prose string into a DocumentFragment. */
  function render(input) {
    const text = stripComments(input);
    const frag = document.createDocumentFragment();
    let buf = "";
    const flush = () => {
      if (!buf) return;
      frag.append(buf.replace(/---/g, "—").replace(/--/g, "–").replace(/~/g, " ").replace(/``/g, "“").replace(/''/g, "”").replace(/`/g, "‘").replace(/[ \t]*\n[ \t]*/g, " "));
      buf = "";
    };
    const add = (node) => { flush(); frag.append(node); };
    let i = 0;
    while (i < text.length) {
      const c = text[i];
      const rest = text.slice(i, i + 40);
      if (rest.startsWith("$$")) {
        const end = text.indexOf("$$", i + 2);
        if (end > 0) { add(katexNode(text.slice(i + 2, end), true)); i = end + 2; continue; }
      }
      if (rest.startsWith("\\[")) {
        const end = text.indexOf("\\]", i + 2);
        if (end > 0) { add(katexNode(text.slice(i + 2, end), true)); i = end + 2; continue; }
      }
      if (rest.startsWith("\\begin{")) {
        const [env, after] = group(text, i + 6);
        const [body, next] = envBody(text, after, env);
        add(block(env, body));
        i = next;
        continue;
      }
      if (c === "$") {
        let end = i + 1;
        while ((end = text.indexOf("$", end)) > 0 && text[end - 1] === "\\") end++;
        if (end > 0) { add(katexNode(text.slice(i + 1, end), false)); i = end + 1; continue; }
      }
      if (rest.startsWith("\\(")) {
        const end = text.indexOf("\\)", i + 2);
        if (end > 0) { add(katexNode(text.slice(i + 2, end), false)); i = end + 2; continue; }
      }
      if (c === "{" || c === "}") { i++; continue; }
      if (c === "\\") {
        // inline code: \verb+…+, \lstinline[opt]{…} or \lstinline|…|
        const vm = text.slice(i, i + 12).match(/^\\(verb|lstinline)(?![a-zA-Z])\*?/);
        if (vm) {
          let k = i + vm[0].length, code = null;
          if (vm[1] === "lstinline") { const o = optional(text, k); if (o) k = o[1]; }
          if (vm[1] === "lstinline" && text[k] === "{") { const g = group(text, k); code = g[0]; k = g[1]; }
          else if (text[k] && !/[\sa-zA-Z]/.test(text[k])) {
            const end = text.indexOf(text[k], k + 1);
            if (end > 0) { code = text.slice(k + 1, end); k = end + 1; }
          }
          if (code != null) { add(el("code", "tex-verb", code)); i = k; continue; }
        }
        // accents: \'e \"{o} \c{c}
        const acc = text[i + 1];
        if (ACCENTS[acc] && (/[^a-zA-Z]/.test(acc) ? true : !/[a-zA-Z]/.test(text[i + 2] ?? "") || text[i + 2] === "{")) {
          let j = i + 2;
          while (text[j] === " ") j++;
          let base;
          if (text[j] === "{") { const g = group(text, j); base = g[0]; j = g[1]; } else { base = text[j] ?? ""; j++; }
          buf += (base.replace(/^\\i$/, "ı") + ACCENTS[acc]).normalize("NFC");
          i = j;
          continue;
        }
        // A symbol command; word commands such as \dots must not run into a letter.
        const sym = SYMBOL_KEYS.find((k) => text.startsWith(k, i) && (!/[a-zA-Z]$/.test(k) || !/[a-zA-Z]/.test(text[i + k.length] ?? "")));
        if (sym) { buf += SYMBOLS[sym]; i += sym.length; if (/[a-zA-Z]$/.test(sym) && text[i] === "{" && text[i + 1] === "}") i += 2; continue; }
        const name = text.slice(i).match(/^\\([a-zA-Z@]+)(\*?)/);
        if (name) {
          const cmd = name[1], star = name[2];
          let j = i + name[0].length;
          const node = command(cmd, star, text, j);
          if (node) { if (node.node) add(node.node); i = node.next; continue; }
          // user macros used in text: expand
          const def = macros[`\\${cmd}`];
          if (def != null && !(`\\${cmd}` in BUILTIN)) {
            const nargs = Math.max(0, ...[...def.matchAll(/#(\d)/g)].map((x) => +x[1]));
            let body = def, k = j;
            for (let a = 1; a <= nargs; a++) { const g = required(text, k); if (!g) break; body = body.split(`#${a}`).join(g[0]); k = g[1]; }
            // a macro of math only renders as math
            if (/^\s*\\(math|bold|operatorname|mathrm|mathcal|mathbb|boldsymbol|bm|hat|tilde|bar|vec|left|frac|sum|prod)/.test(body) || /[_^]/.test(body)) add(katexNode(body, false));
            else frag.append(render(body));
            i = k;
            if (text[i] === " " && nargs === 0 && /[a-zA-Z]/.test(text[i + 1] ?? "")) buf += " ";
            continue;
          }
          // unknown: drop the command, keep its argument
          i = j;
          const opt = optional(text, i); if (opt) i = opt[1];
          continue;
        }
      }
      buf += c;
      i++;
    }
    flush();
    return frag;
  }

  /** Known commands. Returns { node?, next } or null. */
  function command(cmd, star, text, j) {
    const wrapIn = (tag, cls, content) => { const n = el(tag, cls); n.append(render(content)); return n; };
    const arg = () => required(text, j);
    if (cmd in DROP) {
      let k = j;
      if (star === "" && text[k] === "*") k++;
      const o = optional(text, k); if (o) k = o[1];
      for (let a = 0; a < DROP[cmd]; a++) { const g = required(text, k); if (!g) break; k = g[1]; }
      if (cmd === "cmidrule") { const p = text.slice(k).match(/^\s*\([^)]*\)/); if (p) k += p[0].length; }
      return { next: k };
    }
    if (cmd in PASS) {
      let k = j;
      let content = "";
      const o = optional(text, k); if (o) k = o[1];
      for (let a = 0; a < PASS[cmd]; a++) { const g = required(text, k); if (!g) break; content = g[0]; k = g[1]; }
      if (cmd === "parbox" || cmd === "makebox" || cmd === "framebox") { const o2 = optional(text, k); if (o2) k = o2[1]; }
      return { node: wrapIn("span", "", content), next: k };
    }
    const simple = { emph: "em", textit: "em", textsl: "em", textbf: "strong", underline: "u", ul: "u", uline: "u", hl: "mark", textsubscript: "sub", textsuperscript: "sup", texttt: "code", textsc: "span" };
    if (cmd in simple) {
      const g = arg(); if (!g) return null;
      const n = wrapIn(simple[cmd], "", g[0]);
      if (cmd === "textsc") n.style.fontVariant = "small-caps";
      return { node: n, next: g[1] };
    }
    if (cmd === "textcolor" || cmd === "colorbox") {
      const c = arg(); if (!c) return null;
      const g = required(text, c[1]); if (!g) return null;
      const n = wrapIn("span", "", g[0]);
      if (/^[a-z]+$/i.test(c[0])) n.style[cmd === "colorbox" ? "background" : "color"] = c[0];
      return { node: n, next: g[1] };
    }
    if (cmd === "abstract" || cmd === "keywords") {
      const g = arg(); if (!g) return null;
      const d = el("div", cmd === "abstract" ? "tex-abstract" : "tex-keywords");
      d.append(el("b", "", cmd === "abstract" ? "Abstract. " : "Keywords: "), render(g[0]));
      return { node: d, next: g[1] };
    }
    if (cmd === "email") { const g = arg(); if (!g) return null; return { node: el("code", "email", g[0]), next: g[1] }; }
    if (cmd === "IEEEmembership") { const g = arg(); if (!g) return null; return { node: wrapIn("em", "membership", `, ${g[0]}`), next: g[1] }; }
    if (cmd === "and" || cmd === "AND") return { node: document.createTextNode(", "), next: j };
    if (cmd === "doi") { const g = arg(); if (!g) return null; return { node: el("code", "", `doi:${g[0]}`), next: g[1] }; }
    if (cmd === "urlprefix") return { node: document.createTextNode("URL "), next: j };
    if (cmd === "url" || cmd === "path") { const g = arg(); if (!g) return null; return { node: el("code", "", g[0]), next: g[1] }; }
    if (cmd === "href") { const u = arg(); if (!u) return null; const g = required(text, u[1]); if (!g) return null; const n = wrapIn("span", "link", g[0]); n.title = u[0]; return { node: n, next: g[1] }; }
    if (/^(cite|citep|citet|citealp|citealt|citeauthor|citeyear|parencite|textcite|autocite|footcite|supercite)$/.test(cmd)) {
      let k = j;
      const o1 = optional(text, k); if (o1) k = o1[1];
      const o2 = optional(text, k); if (o2) k = o2[1];
      const g = required(text, k); if (!g) return null;
      const keys = g[0].split(",").map((s) => s.trim()).filter(Boolean);
      return { node: citeNode(cmd, keys, (o2 ?? o1)?.[0]), next: g[1] };
    }
    if (/^(ref|eqref|autoref|cref|Cref|pageref|vref|subref)$/.test(cmd)) {
      const g = arg(); if (!g) return null;
      const keys = g[0].split(",").map((s) => s.trim());
      const span = el("span");
      keys.forEach((k, x) => { if (x) span.append(", "); span.append(refNode(cmd, k)); });
      return { node: keys.length === 1 ? span.firstChild : span, next: g[1] };
    }
    if (cmd === "footnote" || cmd === "footnotetext" || cmd === "thanks") {
      let k = j; const o = optional(text, k); if (o) k = o[1];
      const g = required(text, k); if (!g) return null;
      const n = el("sup", "fn", "*");
      n.title = g[0];
      return { node: n, next: g[1] };
    }
    if (cmd === "footnotemark") { let k = j; const o = optional(text, k); if (o) k = o[1]; return { node: el("sup", "fn", "*"), next: k }; }
    if (/^(section|subsection|subsubsection|paragraph|subparagraph)$/.test(cmd)) {
      let k = j; const o = optional(text, k); if (o) k = o[1];
      const g = required(text, k); if (!g) return null;
      const tag = cmd === "paragraph" || cmd === "subparagraph" ? "strong" : "span";
      const n = wrapIn(tag, `tex-head ${cmd}`, g[0]);
      if (tag === "strong") n.append(" ");
      return { node: n, next: g[1] };
    }
    if (cmd === "caption") { let k = j; const o = optional(text, k); if (o) k = o[1]; const g = required(text, k); if (!g) return null; return { node: wrapIn("span", "caption", g[0]), next: g[1] }; }
    if (cmd === "includegraphics") {
      let k = j; const o = optional(text, k); if (o) k = o[1];
      const g = required(text, k); if (!g) return null;
      return { node: figureNode("figure", text.slice(j - "\\includegraphics".length - star.length, g[1])), next: g[1] };
    }
    if (cmd === "item") { let k = j; const o = optional(text, k); if (o) k = o[1]; return { node: document.createTextNode(o ? `${o[0]} ` : "• "), next: k }; }
    return null;
  }

  function block(env, body) {
    const base = env.replace("*", "");
    if (MATH_ENVS.includes(base)) return envNode(env, body);
    if (new RegExp(`^(${CODE_ENVS})$`).test(env)) return codeNode(env, body);
    if (FLOAT_KIND[env] === "fig" || base === "subfigure") {
      // a figure holding only a table is a table
      if (/\\begin\{tabular/.test(body) && !/\\includegraphics/.test(body)) return tableNode(env, body);
      return figureNode(env, body);
    }
    if (FLOAT_KIND[env] === "tab") return tableNode(env, body);
    if (FLOAT_KIND[env] === "alg") return algorithmNode(env, body);
    if (/^(tabular\*?|tabularx|tabulary|longtable|threeparttable)$/.test(env)) { const t = tabularNode(`\\begin{${env}}${body}\\end{${env}}`) ?? tableNode(env, body); return t; }
    if (/^(itemize|enumerate|description|compactitem|compactenum|inparaenum|asparaenum)$/.test(base)) return listNode(base.includes("enum") ? "enumerate" : "itemize", body);
    if (base === "algorithmic") return algorithmNode("algorithm", body);
    if (THEOREMS.includes(base) || base === "proof") {
      const o = optional(body, 0);
      return theoremNode(env, o ? body.slice(o[1]) : body, o?.[0]);
    }
    if (/^(tikzpicture|axis|pgfpicture|groupplot)$/.test(base)) return el("div", "img missing tikz", "TikZ");
    if (base === "abstract") { const d = el("div", "tex-abstract"); d.append(el("b", "", "Abstract—"), render(body)); return d; }
    if (base === "thebibliography") return document.createTextNode("");
    if (base === "IEEEkeywords" || base === "keywords" || base === "keyword") { const d = el("div", "tex-keywords"); d.append(el("b", "", base === "IEEEkeywords" ? "Index Terms—" : "Keywords: "), render(body)); return d; }
    // center, minipage, quote, …: show what is inside
    const d = el("div", `tex-env ${base}`);
    let b = body;
    const o = optional(b, 0); if (o) b = b.slice(o[1]);
    if (base === "minipage") { const w = required(b, 0); if (w) b = b.slice(w[1]); }
    d.append(render(b));
    return d;
  }

  /** Every \\cmd*[opt]{arg} in t: [{opt, arg}], and t without them. */
  function extract(t, cmd) {
    const items = [];
    let out = "", i = 0;
    const re = new RegExp(`\\\\${cmd}(?![a-zA-Z])\\*?`, "g");
    let m;
    while ((m = re.exec(t))) {
      let k = m.index + m[0].length;
      const o = optional(t, k); if (o) k = o[1];
      const g = required(t, k);
      if (!g) continue;
      items.push({ opt: o?.[0], arg: g[0] });
      out += t.slice(i, m.index);
      i = g[1];
      re.lastIndex = g[1];
    }
    return { items, rest: out + t.slice(i) };
  }

  /** Title block and everything else before the first section. */
  // Mark an element as the editable field `field` number `index` of main.tex.
  const mark = (n, field, index) => { n.dataset.field = field; n.dataset.index = index; return n; };

  /** Title block and everything else before the first section. Elements
   *  holding a field of main.tex carry data-field/data-index for editing. */
  function frontNode(src) {
    let t = stripComments(src);
    const box = el("header", "tex-front");
    const title = extract(t, "title"); t = title.rest;
    title.items.forEach((x, i) => { const h = mark(el("h1", "title"), "title", i); h.append(render(x.arg)); box.append(h); });
    const authors = el("div", "authors");
    let ai = 0;
    for (const cmd of ["author", "IEEEauthorblockN"]) {
      const a = extract(t, cmd); t = a.rest;
      a.items.forEach((x) => {
        // one \author may hold several people: \and separates them, \\ breaks lines inside one
        const item = mark(el("span", "author-item"), "author", ai++);
        x.arg.split(/\\(?:and|AND|And)(?![a-zA-Z])/).forEach((who) => {
          const lines = who.split(/\\\\(?:\[[^\]]*\])?/).map((l) => l.trim()).filter((l) => l.replace(/[\s,]/g, ""));
          if (!lines.length) return;
          const block = el("div", "author");
          lines.forEach((l, i) => { const d = el("div", i ? "line" : "name"); d.append(render(l.replace(/^,\s*/, ""))); block.append(d); });
          if (x.opt && /^[\d,\s*]+$/.test(x.opt)) block.firstChild.append(el("sup", "", x.opt));
          item.append(block);
        });
        if (!item.children.length) item.append(el("div", "author empty", "雙擊輸入作者"));
        authors.append(item);
      });
    }
    if (authors.children.length) box.append(authors);
    let fi = 0;
    for (const cmd of ["affil", "address", "institute", "IEEEauthorblockA"]) {
      const a = extract(t, cmd); t = a.rest;
      a.items.forEach((x) => {
        const d = mark(el("div", "affil"), "affil", fi++);
        if (x.opt) d.append(el("sup", "", x.opt.replace(/\*/g, "")));
        d.append(render(x.arg.replace(/\\\\/g, ", ")));
        box.append(d);
      });
    }
    const emails = extract(t, "email"); t = emails.rest;
    if (emails.items.length) box.append(el("div", "affil", emails.items.map((x) => x.arg).join(", ")));
    box.append(render(t));
    box.querySelectorAll(".tex-abstract").forEach((n, i) => mark(n, "abstract", i));
    box.querySelectorAll(".tex-keywords").forEach((n, i) => mark(n, "keywords", i));
    if (!box.querySelector(".tex-abstract")) box.append(mark(el("div", "tex-abstract empty", "雙擊新增摘要"), "abstract", 0));
    return box;
  }

  /** The reference list, numbered the way the paper cites. */
  function bibliographyNode(entries) {
    const box = el("section", "tex-bib");
    box.append(el("h2", "", "References"));
    const ol = el("ol");
    entries.forEach((e, x) => {
      const li = el("li");
      const c = cites[e.key];
      const authorYear = c?.author;
      li.append(el("span", "bib-n", authorYear ? "" : `[${c?.n ?? x + 1}]`));
      const body = el("span", "bib-text");
      body.append(render(e.text.replace(/\\newblock\s*/g, " ")));
      li.append(body);
      li.title = e.key;
      ol.append(li);
    });
    box.append(ol);
    return box;
  }

  /** True when a paragraph is a block (an environment, display math, a heading) rather than running prose. */
  function isDisplay(p) {
    const t = stripComments(p).trim();
    return /^\\begin\{/.test(t) || t.startsWith("\\[") || t.startsWith("$$");
  }
  /** `\subsection{Title}` on its own: [level, title] or null. */
  function heading(p) {
    const t = stripComments(p).trim();
    const m = t.match(/^\\(subsubsection|subsection|section)\*?\{/);
    if (!m) return null;
    const [title, next] = group(t, m[0].length - 1);
    return /^\s*(\\label\{[^}]*\}\s*)?$/.test(t.slice(next)) ? [m[1], title] : null;
  }
  /** True when prose needs rendering as a whole: comments, environments, or line-level commands inside. */
  function needsWhole(p) { return /(^|[^\\])%/.test(p) || /\\begin\{/.test(p) || /\\\\/.test(p); }

  /** Number of a subsection heading's label or position, for display. */
  function headingNumber(p) { const l = p.match(/\\label\{([^}]*)\}/); return l ? labels[l[1]] : null; }

  return {
    setMacros, setNumbers, setFigureBase, number, resetCounters, frontNode, bibliographyNode, render, isDisplay, heading, headingNumber, needsWhole, stripComments, tabularNode,
    numberEquations: (paragraphs) => number([{ paragraphs }]),
    citeNumber: (key) => cites[key]?.n ?? null,
    get labels() { return labels; },
    get kinds() { return kinds; },
  };
})();
