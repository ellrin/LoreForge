// Word count: the status bar shows this section and the whole paper; its
// button opens a list of the parts (abstract, each section) to tick in or
// out, and whether captions, table cells and footnotes count. Counted here
// from the paragraph sources, so it follows every edit at once.
// English counts words (like pc_core::text::word_count: no inline math, no
// \command{arg}, but the text of \emph{…}, \textbf{…} and the like stays);
// Chinese, Japanese and Korean count characters. Display math never counts.
// The choices are kept per paper; a journal's word_limits (template.toml)
// show beside the numbers.
const PCWords = (() => {
  const CJK = /[぀-ヿ㐀-䶿一-鿿豈-﫿가-힯]|[\ud840-\ud87e][\udc00-\udfff]/g;
  const MATH_ENVS = "equation|align|alignat|flalign|gather|multline|eqnarray|displaymath|math|split|dmath|dgroup";
  const SKIP_ENVS = "verbatim|lstlisting|minted|comment|algorithmic|tikzpicture|pgfpicture";
  const FLOATS = "figure|table|wrapfigure|wraptable|sidewaysfigure|sidewaystable|subfigure|subtable|algorithm";
  const TABULAR = "tabular|tabularx|tabulary|tabular\\*|longtable|supertabular|xtabular|tblr";
  // commands whose argument is text on the page
  const KEEP = new Set(["emph", "textbf", "textit", "textsl", "textsc", "texttt", "textrm", "textsf", "textup", "textmd",
    "textnormal", "underline", "uline", "hl", "mbox", "text", "enquote", "textquote", "foreignlanguage", "makebox", "fbox", "framebox"]);
  // the parts that do not count by default
  const AWAY = /^\s*(appendi|supplement|acknowledg|reference|bibliograph|附錄|致謝|誌謝|謝辭|參考文獻|補充)/i;

  const zero = () => ({ w: 0, c: 0 });
  const add = (a, b) => ({ w: a.w + b.w, c: a.c + b.c });
  const n = (x) => x.w + x.c;

  /** End (exclusive) of the {…} group opening at i. */
  function groupEnd(s, i) {
    let d = 0;
    for (; i < s.length; i++) {
      const ch = s[i];
      if (ch === "\\") { i++; continue; }
      if (ch === "{") d++;
      else if (ch === "}" && --d === 0) return i + 1;
    }
    return s.length;
  }

  /** Cut every \cmd{…} out of s; returns [rest, [arguments]]. */
  function takeCommand(s, name) {
    const out = [];
    const re = new RegExp(`\\\\${name}\\*?\\s*(\\[[^\\]]*\\]\\s*)?\\{`, "g");
    let rest = "", at = 0, m;
    while ((m = re.exec(s))) {
      const open = m.index + m[0].length - 1, end = groupEnd(s, open);
      rest += s.slice(at, m.index) + " ";
      out.push(s.slice(open + 1, end - 1));
      at = re.lastIndex = end;
    }
    return [rest + s.slice(at), out];
  }

  /** Words and CJK characters of plain-ish LaTeX (no environments left that matter). */
  function prose(s) {
    // inline math
    s = s.replace(/\$[^$]*\$/g, " ").replace(/\\\([\s\S]*?\\\)/g, " ");
    let out = "";
    for (let i = 0; i < s.length; i++) {
      const ch = s[i];
      if (ch === "\\") {
        let j = i + 1;
        while (j < s.length && /[a-zA-Z@]/.test(s[j])) j++;
        const name = s.slice(i + 1, j);
        if (!name) { i++; out += " "; continue; } // \\ \% \, …
        if (s[j] === "*") j++;
        while (s[j] === " ") j++;
        while (s[j] === "[") { const e = s.indexOf("]", j); if (e < 0) break; j = e + 1; }
        if (s[j] === "{" && !KEEP.has(name)) j = groupEnd(s, j);
        else if (s[j] === "{") { // keep the argument: drop only the braces
          out += " ";
          i = j; continue;
        }
        out += " ";
        i = j - 1;
      } else if (ch === "~") out += " ";
      else if (ch === "{" || ch === "}") { /* grouping only */ }
      else out += ch;
    }
    const c = (out.match(CJK) ?? []).length;
    const w = out.replace(CJK, " ").split(/[\s　-〿＀-￯]+/).filter((t) => /[\p{L}\p{N}]/u.test(t)).length;
    return { w, c };
  }

  const memo = new Map();
  /** One paragraph: { body, cap, tab, fn } as {w, c}. */
  function count(src) {
    const hit = memo.get(src);
    if (hit) return hit;
    let s = String(src).replace(/(^|[^\\])%.*$/gm, "$1");
    s = s.replace(/\$\$[\s\S]*?\$\$/g, " ").replace(/\\\[[\s\S]*?\\\]/g, " ");
    s = s.replace(new RegExp(`\\\\begin\\{(${MATH_ENVS}|${SKIP_ENVS})(\\*?)\\}[\\s\\S]*?\\\\end\\{\\1\\2\\}`, "g"), " ");
    let fns;
    [s, fns] = takeCommand(s, "footnote");
    let caps = [], tabs = [];
    s = s.replace(new RegExp(`\\\\begin\\{(${FLOATS})(\\*?)\\}([\\s\\S]*?)\\\\end\\{\\1\\2\\}`, "g"), (_, _e, _st, inner) => {
      let c;
      [inner, c] = takeCommand(inner, "caption");
      caps.push(...c);
      inner.replace(new RegExp(`\\\\begin\\{(${TABULAR})\\}([\\s\\S]*?)\\\\end\\{\\1\\}`, "g"), (_m, _t, body) => { tabs.push(body); return ""; });
      return " ";
    });
    let c2;
    [s, c2] = takeCommand(s, "captionof\\{[^}]*\\}"); caps.push(...c2);
    s = s.replace(new RegExp(`\\\\begin\\{(${TABULAR})\\}([\\s\\S]*?)\\\\end\\{\\1\\}`, "g"), (_m, _t, body) => { tabs.push(body); return " "; });
    const cell = (b) => prose(b.replace(/^\s*\{[^}]*\}/, "").replace(/&|\\\\/g, " ").replace(/\\(hline|toprule|midrule|bottomrule|cline\{[^}]*\}|cmidrule(\([^)]*\))?\{[^}]*\})/g, " "));
    const r = {
      body: prose(s),
      cap: caps.map(prose).reduce(add, zero()),
      tab: tabs.map(cell).reduce(add, zero()),
      fn: fns.map(prose).reduce(add, zero()),
    };
    if (memo.size > 4000) memo.clear();
    memo.set(src, r);
    return r;
  }

  // ── the paper's parts and the choices ──
  const key = () => `pc.words.${state.paper?.name ?? ""}`;
  function prefs() {
    const p = store.get(key(), {}) ?? {};
    return { parts: p.parts ?? {}, cap: !!p.cap, tab: !!p.tab, fn: !!p.fn };
  }
  const savePrefs = (p) => store.set(key(), p);

  function parts() {
    if (!state.paper) return [];
    const v = [];
    const abs = (state.paper.fields ?? []).filter((f) => f.field === "abstract").map((f) => f.value);
    if (abs.length) v.push({ id: "abstract", title: "摘要", texts: abs, away: false, abstract: true });
    sections().forEach((s, i) => v.push({ id: s.file || `#${i}`, title: s.title || `${pad(i + 1)}`, texts: s.paragraphs, away: AWAY.test(s.title ?? ""), si: i }));
    return v;
  }
  const on = (p, pr) => pr.parts[p.id] ?? !p.away;

  function tally(texts, pr) {
    return texts.map(count).reduce((a, r) => {
      let t = add(a, r.body);
      if (pr.cap) t = add(t, r.cap);
      if (pr.tab) t = add(t, r.tab);
      if (pr.fn) t = add(t, r.fn);
      return t;
    }, zero());
  }

  function limits() { return state.paper?.word_limits ?? {}; }
  function limitOf(p) {
    const L = limits();
    if (p.abstract) return L.abstract;
    const sec = L.sections ?? {};
    const k = Object.keys(sec).find((t) => t.toLowerCase() === (p.title ?? "").trim().toLowerCase());
    return k ? sec[k] : undefined;
  }

  function summary() {
    const pr = prefs();
    const ps = parts().map((p) => ({ ...p, on: on(p, pr), n: tally(p.texts, pr) }));
    const all = ps.reduce((a, p) => add(a, p.n), zero());
    const total = ps.filter((p) => p.on).reduce((a, p) => add(a, p.n), zero());
    return { pr, ps, all, total };
  }

  const num = (x) => x.toLocaleString("en-US");
  /** One number in 字 (English words + CJK characters, as the app counts elsewhere). */
  const fmt = (x) => num(n(x));
  /** Mixed text: the two kinds, for a tooltip. */
  const kinds = (x) => (x.w && x.c ? `英文 ${num(x.w)} 字 · 中文 ${num(x.c)} 字` : "");

  // ── status bar button and popover ──
  let btn = null, pop = null;

  function label(b, s) {
    const cur = s.ps.find((p) => p.si === state.cur.s);
    b.innerHTML = "";
    const over = limits().total != null && n(s.total) > limits().total;
    b.append(
      cur ? `本節 ${fmt(cur.n)} 字 · ` : "",
      el("span", over ? "wc-over" : "", `全文 ${fmt(s.total)} 字`),
      el("span", "wc-caret", " ▴"),
    );
    b.title = kinds(s.total);
  }

  /** The words part of the status bar: a button opening the popover. */
  function button() {
    const s = summary();
    btn = el("button", "sync wc-btn");
    btn.setAttribute("aria-haspopup", "dialog");
    btn.setAttribute("aria-expanded", String(!!pop));
    label(btn, s);
    btn.onclick = () => (pop ? close() : open());
    if (pop) paint(s);
    return btn;
  }

  function open() {
    pop = el("div", "menu wc-pop");
    pop.setAttribute("role", "dialog");
    pop.setAttribute("aria-label", "字數");
    document.body.append(pop);
    paint(summary());
    btn?.setAttribute("aria-expanded", "true");
  }
  function close() {
    pop?.remove(); pop = null;
    btn?.setAttribute("aria-expanded", "false");
  }

  function place() {
    if (!pop || !btn?.isConnected) return;
    const r = btn.getBoundingClientRect();
    pop.style.bottom = `${innerHeight - r.top + 6}px`;
    pop.style.left = `${Math.max(8, Math.min(r.left, innerWidth - pop.offsetWidth - 8))}px`;
  }

  function change(fn) {
    const pr = prefs();
    fn(pr);
    savePrefs(pr);
    const s = summary();
    if (btn?.isConnected) label(btn, s);
    paint(s);
  }

  function check(text, checked, fn) {
    const l = el("label", "wc-check");
    const c = el("input");
    c.type = "checkbox"; c.checked = checked;
    c.onchange = () => fn(c.checked);
    l.append(c, el("span", "wc-name", text));
    return l;
  }

  function limitText(x, max) {
    const span = el("span", "wc-n");
    span.append(el("span", n(x) > max ? "wc-over" : "", fmt(x)));
    if (max != null) span.append(el("span", "wc-lim", ` / ${num(max)}`));
    span.title = kinds(x);
    return span;
  }

  function paint(s) {
    if (!pop) return;
    const scroll = pop.querySelector(".wc-list")?.scrollTop ?? 0;
    pop.innerHTML = "";
    const head = el("div", "wc-head");
    head.append(el("b", "", "字數"));
    // back to the default selection, offered only when it differs
    const isDefault = s.ps.every((p) => p.on === !p.away) && !s.pr.cap && !s.pr.tab && !s.pr.fn;
    if (!isDefault) {
      const reset = el("button", "link wc-reset", "還原預設");
      reset.onclick = () => change((pr) => { pr.parts = {}; pr.cap = pr.tab = pr.fn = false; });
      head.append(reset);
    }
    pop.append(head);

    const list = el("div", "wc-list");
    s.ps.forEach((p) => {
      const row = el("div", "wc-row");
      if (p.si === state.cur.s) row.classList.add("cur");
      row.append(check(p.title, p.on, (v) => change((pr) => { pr.parts[p.id] = v; })), limitText(p.n, limitOf(p)));
      list.append(row);
    });
    if (!s.ps.length) list.append(el("div", "wc-row empty", "—"));
    pop.append(list, el("div", "sep"));

    const opts = el("div", "wc-opts");
    [["cap", "含圖表標題"], ["tab", "含表格內容"], ["fn", "含註腳"]].forEach(([k, t]) =>
      opts.append(check(t, s.pr[k], (v) => change((pr) => { pr[k] = v; }))));
    pop.append(opts);

    const foot = el("div", "wc-foot");
    const max = limits().total;
    foot.append(el("span", max != null && n(s.total) > max ? "wc-over" : "", `合計 ${fmt(s.total)} 字`));
    if (max != null) foot.append(el("span", "wc-lim", ` / ${num(max)} 上限`));
    foot.title = kinds(s.total);
    pop.append(foot);
    pop.querySelector(".wc-list").scrollTop = scroll;
    place();
  }

  document.addEventListener("pointerdown", (e) => {
    if (pop && !e.target.closest?.(".wc-pop, .wc-btn")) close();
  }, true);
  document.addEventListener("keydown", (e) => {
    if (pop && e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); btn?.focus(); }
  }, true);
  addEventListener("resize", () => place());

  return { button, count, summary, close };
})();
