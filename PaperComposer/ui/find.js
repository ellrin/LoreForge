// Find & Replace over the paper: Ctrl/Cmd+F opens the bar, Ctrl+H (or
// Cmd+Alt+F: Cmd+H hides the app on macOS) opens it with the replace row.
//
// Matching reads what the reader sees: each paragraph's LaTeX source is
// searched with command names, comments, math and the arguments of \cite,
// \ref, \label, \includegraphics… hidden (same length, so a match's offsets
// are offsets in the source and a replacement goes straight back into it).
// "搜尋公式與指令" hides only comments. Matches are marked in the drawn page
// by wrapping text nodes in <mark> (never inside formulas, citations or
// references), again whenever the page is redrawn.
//
// A replacement is checked against the notation registry first
// (POST /api/papers/{name}/notation/check); what it adds is shown with
// 「仍要取代」/「取消」. Changes are saved one section at a time; the last
// replacement can be undone (復原, or Ctrl/Cmd+Z while the bar has focus).

const PCFind = (() => {
  const opt = { case: false, word: false, all: false, scope: "all" };
  let bar, dock, q, r, count, repRow, note, menu, moreBtn, scopeTag;
  const MAC = /Mac/.test(navigator.platform);
  const REPLACE_KEY = MAC ? "⌘⌥F" : "Ctrl+H";
  let asking = null;  // the 仍要取代/取消 question waiting for an answer
  let hits = [];      // { s, p, a, b }: s = -1 for a title-block field, p its place in frontList()
  let cur = -1;
  let undo = null;    // { paper, secs: [{ si, file, before, after }], front: [{ field, index, before, after }], n }
  const okd = new Set(); // findings already accepted with 仍要取代, per find → replace
  let marked = new Set();
  let busy = false;
  let running = false; // one replacement at a time (a held Enter, a double click)
  /** Run a replacement unless one is already under way. */
  const once = (fn) => async () => {
    if (running) return;
    running = true;
    try { await fn(); } finally { running = false; }
  };
  /** Still a match where it was found (the text may have changed since). */
  const holds = (h) => { const t = source(h); return h.b <= t.length && matches(regex(q.value), visible(t, opt.all)).some(([a, b]) => a === h.a && b === h.b); };

  // ── Masking ─────────────────────────────────────────
  const HOLE = "\0", ESC = "\u0001";
  // commands whose arguments are not prose: name → required arguments hidden (all [..] too)
  const SKIP = Object.fromEntries([
    ..."cite citep citet citealp citealt citeauthor Citeauthor citeyear citeyearpar Citet Citep Citealp parencite Parencite textcite Textcite autocite footcite nocite ref eqref pageref cref Cref autoref nameref vref label includegraphics url input include bibliography bibliographystyle usepackage documentclass hspace vspace color pagestyle thispagestyle cline hline toprule midrule bottomrule cmidrule"
      .split(" ").map((n) => [n, 1]),
    ["href", 1], ["textcolor", 1], ["colorbox", 1], ["setlength", 2], ["addtolength", 2], ["newcommand", 2], ["renewcommand", 2], ["providecommand", 2], ["DeclareMathOperator", 2], ["multicolumn", 2], ["multirow", 2], ["resizebox", 2], ["scalebox", 1], ["rotatebox", 1],
  ]);
  const MATH_ENV = /^(equation|align|alignat|gather|multline|eqnarray|flalign|displaymath|math|dmath|split)\*?$/;
  // environment arguments after \begin{env} (column specs, widths)
  const ENV_ARGS = { tabular: 1, "tabular*": 2, tabularx: 2, tabulary: 2, array: 1, longtable: 1, wrapfigure: 2, wraptable: 2, minipage: 1, subfigure: 1, subtable: 1, multicols: 1 };

  /** End of the {…} group at i (exclusive), or -1. */
  function group(s, i) {
    if (s[i] !== "{") return -1;
    let d = 0;
    for (let k = i; k < s.length; k++) {
      const c = s[k];
      if (c === "\\") { k++; continue; }
      if (c === "{") d++;
      else if (c === "}" && --d === 0) return k + 1;
    }
    return s.length;
  }
  /** End of the […] at i (exclusive), or -1. */
  function optional(s, i) {
    if (s[i] !== "[") return -1;
    let d = 0;
    for (let k = i + 1; k < s.length; k++) {
      const c = s[k];
      if (c === "\\") { k++; continue; }
      if (c === "{") d++;
      else if (c === "}") d--;
      else if (c === "]" && d <= 0) return k + 1;
    }
    return s.length;
  }
  const blank = (s, i) => { while (i < s.length && (s[i] === " " || s[i] === "\t")) i++; return i; };
  /** Past n required arguments (and any optional ones around them) from j. */
  function args(s, j, n) {
    let k = j;
    for (;;) {
      const t = blank(s, k);
      const o = optional(s, t);
      if (o > 0) { k = o; continue; }
      if (n > 0) { const g = group(s, t); if (g > 0) { k = g; n--; continue; } }
      return k;
    }
  }
  /** Close of a delimiter from i, skipping escaped characters: index past it, or the end. */
  function until(s, i, close) {
    for (let k = i; k < s.length; k++) {
      if (s.startsWith(close, k)) return k + close.length;
      if (s[k] === "\\") k++;
    }
    return s.length;
  }

  /** The source with what the reader does not see as prose hidden; same length. */
  function visible(s, all) {
    const out = s.split("");
    const hide = (a, b) => { for (let k = a; k < b; k++) out[k] = HOLE; };
    const n = s.length;
    let i = 0;
    while (i < n) {
      const c = s[i];
      if (c === "%") { let j = s.indexOf("\n", i); if (j < 0) j = n; hide(i, j); i = j; continue; }
      if (c !== "\\" && all) { i++; continue; }
      if (c === "\\") {
        const d = s[i + 1];
        if (d === undefined) { hide(i, i + 1); i++; continue; }
        if ("%&$#_{}".includes(d)) { out[i] = ESC; i += 2; continue; } // \% shows as %
        if (all) { i += 2; continue; }
        if (/[a-zA-Z@]/.test(d)) {
          let j = i + 1;
          while (j < n && /[a-zA-Z@]/.test(s[j])) j++;
          let name = s.slice(i + 1, j);
          if (s[j] === "*") { j++; name += "*"; }
          if (name === "begin" || name === "end") {
            const k = blank(s, j), e = group(s, k);
            if (e < 0) { hide(i, j); i = j; continue; }
            const env = s.slice(k + 1, e - 1);
            if (name === "begin" && MATH_ENV.test(env)) { const z = s.indexOf(`\\end{${env}}`, e); const end = z < 0 ? n : z + env.length + 6; hide(i, end); i = end; continue; }
            const z = name === "begin" ? args(s, e, ENV_ARGS[env] ?? 0) : e;
            hide(i, z); i = z; continue;
          }
          const base = name.replace(/\*$/, "");
          const z = base in SKIP ? args(s, j, SKIP[base]) : j;
          hide(i, z); i = z; continue;
        }
        if (d === "(") { const z = until(s, i + 2, "\\)"); hide(i, z); i = z; continue; }
        if (d === "[") { const z = until(s, i + 2, "\\]"); hide(i, z); i = z; continue; }
        hide(i, i + 2); i += 2; continue; // \\ \, \' …
      }
      if (c === "$") {
        const z = s[i + 1] === "$" ? until(s, i + 2, "$$") : until(s, i + 1, "$");
        hide(i, z); i = z; continue;
      }
      if (c === "{" || c === "}") { out[i] = HOLE; i++; continue; }
      i++;
    }
    return out.join("");
  }

  // ── Matching ────────────────────────────────────────
  /** The query as a regex: literal text, any run of spaces matches any
   *  whitespace or ~, a special character may be escaped in the source. */
  function regex(text) {
    if (!text.trim()) return null;
    const src = text.split(/(\s+)/).filter(Boolean).map((t) => /^\s+$/.test(t) ? "[\\s~]+"
      : [...t].map((ch) => ("%&$#_{}".includes(ch) ? `${ESC}?` : "") + ch.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("")).join("");
    const w = opt.word ? ["(?<![\\p{L}\\p{N}_])", "(?![\\p{L}\\p{N}_])"] : ["", ""];
    try { return new RegExp(w[0] + src + w[1], `gu${opt.case ? "" : "i"}`); } catch { return null; }
  }
  function matches(re, text) {
    const out = [];
    if (!re) return out;
    re.lastIndex = 0;
    for (let m; (m = re.exec(text));) {
      if (!m[0].length) { re.lastIndex++; continue; }
      out.push([m.index, m.index + m[0].length]);
    }
    return out;
  }

  const FRONT = ["title", "abstract", "keywords"];
  const frontList = () => (state.paper?.fields ?? []).filter((f) => FRONT.includes(f.field));
  const source = (h) => (h.s < 0 ? frontList()[h.p]?.value : sections()[h.s]?.paragraphs[h.p]) ?? "";
  const same = (x, y) => x && y && x.s === y.s && x.p === y.p;
  const before = (x, y) => x.s !== y.s ? x.s < y.s : x.p !== y.p ? x.p < y.p : x.a < y.a;

  function compute() {
    const re = regex(q.value);
    const out = [];
    if (!re || !state.paper) return out;
    const one = (s, p, text) => matches(re, visible(text, opt.all)).forEach(([a, b]) => out.push({ s, p, a, b }));
    if (opt.scope === "all") frontList().forEach((f, i) => one(-1, i, f.value));
    sections().forEach((sec, si) => {
      if (opt.scope === "section" && si !== state.cur.s) return;
      sec.paragraphs.forEach((t, pi) => one(si, pi, t));
    });
    return out;
  }

  // ── The page ────────────────────────────────────────
  function blockOf(s, p) {
    if (s < 0) { const f = frontList()[p]; return f && document.querySelector(`#front [data-field="${f.field}"][data-index="${f.index}"]`); }
    return paraEl(s, p);
  }
  const NOT_PROSE = ".math, .katex, .cite, .ref, .pc-num, code, textarea, .tbl";

  function unmark(block) {
    block.querySelectorAll("mark.find-hit").forEach((m) => m.replaceWith(...m.childNodes));
    block.normalize();
    block.classList.remove("find-para", "find-cur");
  }

  /** Wrap what the query matches in the block's drawn text; returns how many. */
  function markBlock(block, re) {
    if (block.classList.contains("tbl") || block.querySelector(":scope > textarea")) return -1;
    const nodes = [];
    const walk = document.createTreeWalker(block, NodeFilter.SHOW_TEXT, {
      acceptNode: (n) => n.parentElement.closest(NOT_PROSE) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
    });
    let text = "";
    for (let n; (n = walk.nextNode());) { nodes.push([n, text.length]); text += n.data; }
    const found = matches(re, text);
    for (let k = found.length - 1; k >= 0; k--) {
      const [a, b] = found[k];
      for (let x = nodes.length - 1; x >= 0; x--) {
        const [n, at] = nodes[x], end = at + n.data.length;
        if (end <= a || at >= b) continue;
        const s0 = Math.max(a, at) - at, e0 = Math.min(b, end) - at;
        const mid = s0 ? n.splitText(s0) : n;
        if (e0 - s0 < mid.data.length) mid.splitText(e0 - s0);
        const m = document.createElement("mark");
        m.className = "find-hit";
        m.dataset.k = k;
        mid.replaceWith(m);
        m.append(mid);
      }
    }
    return found.length;
  }

  /** Marks, the current match, the counter and the navigator's badges. */
  function paint() {
    const re = q.value ? regex(q.value) : null;
    const groups = new Map();
    hits.forEach((h, i) => {
      const key = `${h.s}:${h.p}`;
      if (!groups.has(key)) groups.set(key, { s: h.s, p: h.p, list: [] });
      groups.get(key).list.push(i);
    });
    const now = new Set();
    groups.forEach((g) => {
      const block = blockOf(g.s, g.p);
      if (!block) return;
      unmark(block);
      const n = re ? markBlock(block, re) : -1;
      now.add(block);
      block._findFit = n === g.list.length; // drawn matches line up with the source's
      if (!block._findFit) block.classList.add("find-para");
      const at = g.list.indexOf(cur);
      if (at >= 0) {
        if (block._findFit) block.querySelectorAll(`mark.find-hit[data-k="${at}"]`).forEach((m) => m.classList.add("cur"));
        else block.classList.add("find-cur");
      }
    });
    marked.forEach((b) => { if (!now.has(b) && b.isConnected) unmark(b); });
    marked = now;
    count.textContent = !q.value ? "" : hits.length ? `${cur + 1}/${hits.length}` : "0";
    bar.classList.toggle("none", !!q.value && !hits.length);
    badges();
  }

  const perPara = () => {
    const m = new Map();
    hits.forEach((h) => m.set(`${h.s}:${h.p}`, (m.get(`${h.s}:${h.p}`) ?? 0) + 1));
    return m;
  };
  function setBadge(b, n) {
    let x = b.querySelector(":scope > .find-badge");
    if (!n) { x?.remove(); return; }
    if (!x) { x = el("span", "find-badge"); b.append(x); }
    x.textContent = String(n);
  }
  /** Navigator: a count on each paragraph (and the title block) holding matches. */
  function badges() {
    const m = perPara();
    document.querySelectorAll("#nav .outline button[data-s][data-p]").forEach((b) => setBadge(b, m.get(`${b.dataset.s}:${b.dataset.p}`) ?? 0));
    const fb = document.querySelector("#nav .sec.front > button");
    if (fb) setBadge(fb, hits.filter((h) => h.s < 0).length);
  }
  /** Hook for navLeaf (app.js): a new leaf gets its count. */
  function badge(b) {
    if (!isOpen() || !hits.length) return;
    setBadge(b, hits.filter((h) => h.s === +b.dataset.s && h.p === +b.dataset.p).length);
  }

  /** Make match i current: its paragraph selected, its navigator item opened, in view. */
  function go(i) {
    if (!hits.length) { cur = -1; paint(); return; }
    cur = (i + hits.length) % hits.length;
    const h = hits[cur];
    if (h.s >= 0 && (state.cur.s !== h.s || state.cur.p !== h.p)) select(h.s, h.p);
    paint();
    const block = blockOf(h.s, h.p);
    if (!block) return;
    const target = block.querySelector("mark.find-hit.cur") ?? block;
    const area = $("#pageArea"), ar = area.getBoundingClientRect(), tr = target.getBoundingClientRect();
    if (tr.top < ar.top + 110 || tr.bottom > ar.bottom - 48 || !tr.height) PCLazy.jump(target, { block: "center", behavior: "auto" });
    else PCLazy.reveal(target);
    selectInEditor();
  }
  /** A paragraph open in its editor: select the current match there. */
  function selectInEditor() {
    const h = hits[cur];
    const ta = h && h.s >= 0 && paraEl(h.s, h.p)?.querySelector(":scope > textarea");
    if (ta && ta.value === source(h)) ta.setSelectionRange(h.a, h.b);
  }

  /** The first match at or after the current paragraph. */
  function fromHere() {
    const at = { s: state.cur.s, p: state.cur.p, a: 0 };
    const i = hits.findIndex((h) => !before(h, at));
    return i < 0 ? 0 : i;
  }

  /** Search again (the text or the paper changed); keep the place. */
  function refresh(place) {
    const was = place ?? hits[cur];
    hits = compute();
    if (!hits.length) cur = -1;
    else if (was) { const i = hits.findIndex((h) => !before(h, was)); cur = i < 0 ? 0 : i; }
    else cur = Math.min(Math.max(cur, 0), hits.length - 1);
    paint();
  }

  // ── Replacing ───────────────────────────────────────
  /** What goes into the source: in prose, a bare % & # would end the line or break the table. */
  const replacement = () => (opt.all ? r.value : r.value.replace(/(?<!\\)([%&#])/g, "\\$1"));
  const apiBase = () => `/api/papers/${encodeURIComponent(state.paper.name)}`;

  /** The paragraph editor saves on blur: wait for it before changing paragraphs. */
  async function settle() {
    const ta = document.querySelector("#paper .para > textarea");
    if (!ta) return;
    ta.blur();
    for (let t = 0; t < 60 && document.querySelector("#paper .para > textarea"); t++) await new Promise((ok) => setTimeout(ok, 50));
    refresh();
  }

  /** The registry's checks on the change; true when it may go ahead. */
  async function vet(was, now) {
    let res;
    try { res = await api(`${apiBase()}/notation/check`, { before: was, texts: now, find: q.value }); }
    catch (e) { res = { blocking: [`對照表檢查失敗：${e.message}`] }; }
    const pair = `${q.value}\u0001${r.value}\u0001`;
    const lines = [
      ...(res.entries ?? []).map((x) => `對照表登記：${x}`),
      ...(res.blocking ?? []), ...(res.shown ?? []),
      ...(res.new_symbols?.length ? [`對照表沒有的符號：${res.new_symbols.join("、")}`] : []),
    ].filter((x) => !okd.has(pair + x));
    if (!lines.length) return true;
    return new Promise((ok) => {
      note.replaceChildren();
      const msg = el("div", "find-msg");
      lines.slice(0, 3).forEach((l) => msg.append(el("div", "", l)));
      if (lines.length > 3) msg.append(el("div", "", `…＋${lines.length - 3}`));
      if ((res.entries ?? []).length) msg.append(el("div", "find-sub", "建議改對照表"));
      const done = asking = (yes) => { asking = null; note.hidden = true; note.replaceChildren(); if (yes) lines.forEach((l) => okd.add(pair + l)); ok(yes); };
      const go = el("button", "ghost", "仍要取代"); go.onclick = () => done(true);
      const no = el("button", "ghost", "取消"); no.onclick = () => done(false);
      const row = el("div", "find-acts"); row.append(go, no);
      note.className = "find-note warn";
      note.append(msg, row);
      note.hidden = false;
      no.focus();
    });
  }

  /** Apply new texts to paragraphs and title fields; one save per section. */
  async function commit(changes, n) {
    const paper = state.paper.name;
    const secs = new Map(), front = [];
    for (const c of changes) {
      if (c.s < 0) { const f = frontList()[c.p]; front.push({ field: f.field, index: f.index, before: f.value, after: c.text }); continue; }
      if (!secs.has(c.s)) secs.set(c.s, { si: c.s, file: sections()[c.s].file, before: [...sections()[c.s].paragraphs], after: null });
      sections()[c.s].paragraphs[c.p] = c.text;
    }
    busy = true;
    try {
      for (const x of secs.values()) {
        try { await saveSection(x.si); } catch (e) { flash(`儲存失敗：${e.message}`, 6000); }
        x.after = [...sections()[x.si].paragraphs];
        refreshSection(x.si);
      }
      for (const f of front) {
        try {
          const res = await api(`${apiBase()}/front`, { field: f.field, index: f.index, value: f.after }, "PUT");
          Object.assign(state.paper, { front: res.front, back: res.back, fields: res.fields });
          f.after = res.fields.find((x) => x.field === f.field && x.index === f.index)?.value ?? f.after;
        } catch (e) { flash(`儲存失敗：${e.message}`, 6000); }
      }
      if (front.length) render();
      refreshGit();
    } finally { busy = false; }
    undo = { paper, secs: [...secs.values()], front, n };
    showUndo();
  }

  function showUndo() {
    note.replaceChildren();
    note.className = "find-note";
    const b = el("button", "ghost", "復原");
    b.onclick = revert;
    const row = el("div", "find-acts");
    row.append(el("span", "find-sub", `已取代 ${undo.n} 處`), b);
    note.append(row);
    note.hidden = false;
  }

  /** Put back the paragraphs and fields the last replacement changed. */
  async function revert() {
    const u = undo;
    if (!u) return;
    const eq = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);
    const intact = state.paper?.name === u.paper
      && u.secs.every((x) => sections()[x.si]?.file === x.file && eq(sections()[x.si].paragraphs, x.after))
      && u.front.every((f) => state.paper.fields.find((x) => x.field === f.field && x.index === f.index)?.value === f.after);
    if (!intact) { flash("之後已修改，無法復原"); undo = null; note.hidden = true; return; }
    undo = null;
    note.hidden = true;
    busy = true;
    try {
      for (const x of u.secs) {
        sections()[x.si].paragraphs = [...x.before];
        try { await saveSection(x.si); } catch (e) { flash(`儲存失敗：${e.message}`, 6000); }
        refreshSection(x.si);
      }
      for (const f of u.front) {
        try {
          const res = await api(`${apiBase()}/front`, { field: f.field, index: f.index, value: f.before }, "PUT");
          Object.assign(state.paper, { front: res.front, back: res.back, fields: res.fields });
        } catch (e) { flash(`儲存失敗：${e.message}`, 6000); }
      }
      if (u.front.length) render();
      refreshGit();
    } finally { busy = false; }
    refresh();
  }

  const replaceOne = once(async () => {
    if (!state.paper) return;
    await settle();
    if (cur < 0 || !hits.length) return;
    const h = hits[cur];
    if (!holds(h)) { refresh(); return; }
    const text = source(h), rep = replacement();
    const next = text.slice(0, h.a) + rep + text.slice(h.b);
    if (!(await vet([text], [next]))) return;
    await commit([{ s: h.s, p: h.p, text: next }], 1);
    // carry on after what was put in (replacing "a" with "ab" must not loop)
    refresh({ s: h.s, p: h.p, a: h.a + rep.length });
    if (hits.length) go(cur);
  });

  const replaceAll = once(async () => {
    if (!state.paper) return;
    await settle();
    if (!hits.length) return;
    const rep = replacement();
    const by = new Map();
    hits.forEach((h) => { const k = `${h.s}:${h.p}`; if (!by.has(k)) by.set(k, { s: h.s, p: h.p, list: [] }); by.get(k).list.push(h); });
    const changes = [...by.values()].map((g) => {
      let t = source(g);
      for (const h of [...g.list].reverse()) t = t.slice(0, h.a) + rep + t.slice(h.b);
      return { s: g.s, p: g.p, was: source(g), text: t };
    });
    if (!(await vet(changes.map((c) => c.was), changes.map((c) => c.text)))) return;
    await commit(changes, hits.length);
    refresh();
  });

  // ── The bar ─────────────────────────────────────────
  const isOpen = () => !!dock && !dock.hidden;

  function button(icon, label, fn, cls) {
    const b = iconButton(icon, label, fn);
    if (cls) b.classList.add(cls);
    return b;
  }
  function textButton(text, label, fn) {
    const b = el("button", "ico find-opt", text);
    b.setAttribute("aria-label", label);
    b.setAttribute("aria-pressed", "false");
    b.onclick = fn;
    return b;
  }

  function build() {
    dock = el("div", "find-dock");
    dock.hidden = true;
    bar = el("div", "find-bar");
    bar.setAttribute("role", "search");
    const row = el("div", "find-row");
    const toggle = button("chevron", `取代 ${REPLACE_KEY}`, () => showReplace(repRow.hidden), "find-toggle");
    q = el("input", "find-q");
    q.type = "text";
    q.placeholder = "尋找";
    q.setAttribute("aria-label", "尋找");
    q.spellcheck = false;
    count = el("span", "find-count");
    scopeTag = el("span", "find-scope", "本節");
    scopeTag.hidden = true;
    const caseB = textButton("Aa", "大小寫相符", () => flip("case", caseB));
    const wordB = textButton("ab", "全字相符", () => flip("word", wordB));
    const more = moreBtn = button("more", "更多選項", () => toggleMenu(more));
    row.append(toggle, q, scopeTag, count,
      button("up", "上一個 Shift+Enter", () => go(cur - 1)),
      button("chevron", "下一個 Enter", () => go(cur + 1)),
      caseB, wordB, more,
      button("x", "關閉 Esc", close));
    repRow = el("div", "find-row rep");
    repRow.hidden = true;
    r = el("input", "find-q");
    r.type = "text";
    r.placeholder = "取代為…";
    r.setAttribute("aria-label", "取代為");
    r.spellcheck = false;
    const one = el("button", "ghost", "取代"); one.onclick = replaceOne; one.title = "Enter";
    const all = el("button", "ghost", "全部取代"); all.onclick = replaceAll; all.title = "Ctrl+Alt+Enter";
    repRow.append(el("span", "find-pad"), r, one, all);
    note = el("div", "find-note");
    note.hidden = true;
    menu = el("div", "menu find-menu");
    menu.hidden = true;
    menu.setAttribute("role", "menu");
    bar.append(row, repRow, note, menu);
    dock.append(bar);
    $("#keyValue").after(dock);

    q.addEventListener("input", () => { hits = compute(); go(fromHere()); });
    q.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); go(cur + (e.shiftKey ? -1 : 1)); }
    });
    r.addEventListener("keydown", (e) => {
      if (e.key !== "Enter" || e.isComposing) return;
      e.preventDefault();
      if ((e.ctrlKey || e.metaKey) && e.altKey) replaceAll(); else replaceOne();
    });
    bar.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        if (!menu.hidden) { hideMenu(); return; }
        close();
      }
    });
    document.addEventListener("click", (e) => {
      if (!e.target.isConnected) return; // a menu item redrew the menu
      if (!menu.hidden && !e.target.closest?.(".find-menu, .find-bar .ico")) hideMenu();
    });

    // the page is redrawn (render, refreshSection, another paper): search and mark again
    let queued = false;
    new MutationObserver(() => {
      if (!isOpen() || queued) return;
      queued = true;
      requestAnimationFrame(() => { queued = false; if (isOpen()) refresh(); });
    }).observe($("#paper"), { childList: true });
    // a paragraph opened in its editor: the current match is selected there
    $("#paper").addEventListener("focusin", (e) => { if (isOpen() && e.target.tagName === "TEXTAREA") selectInEditor(); });
  }

  function flip(k, b) {
    opt[k] = !opt[k];
    b.setAttribute("aria-pressed", String(opt[k]));
    refresh();
    if (hits.length) go(cur);
  }

  /** Options hidden in ⋯ that are not the default show on the bar. */
  function markOptions() {
    scopeTag.hidden = opt.scope !== "section";
    moreBtn.classList.toggle("set", opt.scope === "section" || !!opt.all);
  }
  function hideMenu() { menu.hidden = true; moreBtn.setAttribute("aria-expanded", "false"); }
  function toggleMenu(btn) {
    if (!menu.hidden) { hideMenu(); return; }
    menu.replaceChildren();
    const item = (label, on, fn) => {
      const b = el("button", "", label);
      b.setAttribute("aria-current", String(on));
      b.onclick = () => { fn(); hideMenu(); toggleMenu(btn); };
      menu.append(b);
    };
    menu.append(el("div", "hm-title", "範圍"));
    item("全文", opt.scope === "all", () => { opt.scope = "all"; markOptions(); refresh(); });
    item("本節", opt.scope === "section", () => { opt.scope = "section"; markOptions(); refresh(); });
    menu.append(el("div", "sep"));
    item("搜尋公式與指令", opt.all, () => { opt.all = !opt.all; markOptions(); refresh(); });
    menu.hidden = false;
    btn.setAttribute("aria-expanded", "true");
  }

  function showReplace(on) {
    repRow.hidden = !on;
    bar.querySelector(".find-toggle").setAttribute("aria-expanded", String(on));
  }

  /** Open the bar (with the replace row for Ctrl+H); a short selection becomes the query. */
  function open(replace) {
    if (!state.paper) return;
    if (!dock) build();
    const ta = document.activeElement?.tagName === "TEXTAREA" ? document.activeElement : null;
    const picked = (ta ? ta.value.slice(ta.selectionStart, ta.selectionEnd) : String(getSelection() ?? "")).trim();
    dock.hidden = false;
    if (replace) showReplace(true);
    if (picked && picked.length < 200 && !picked.includes("\n")) q.value = picked;
    (replace && q.value ? r : q).focus();
    q.select();
    if (replace && q.value) r.select();
    hits = compute();
    if (hits.length) go(fromHere()); else paint();
  }

  function close() {
    if (!dock || dock.hidden) return;
    asking?.(false);
    dock.hidden = true;
    hideMenu();
    note.hidden = true;
    hits = [];
    cur = -1;
    undo = null;
    marked.forEach((b) => b.isConnected && unmark(b));
    marked = new Set();
    badges();
  }

  document.addEventListener("keydown", (e) => {
    if (document.querySelector("dialog[open]")) return;
    const mod = e.ctrlKey || e.metaKey;
    const k = e.code.startsWith("Key") ? e.code.slice(3).toLowerCase() : e.key.toLowerCase(); // Alt changes e.key on macOS
    if (mod && !e.shiftKey && k === "f") { e.preventDefault(); open(e.altKey); return; }
    // Ctrl+H = replace, except on macOS where Ctrl+H deletes backward in text fields (there: ⌘⌥F)
    if (!MAC && e.ctrlKey && !e.metaKey && !e.shiftKey && !e.altKey && k === "h") { e.preventDefault(); open(true); return; }
    if (!isOpen()) return;
    const inBar = bar.contains(e.target);
    // the event's target: an editor's own Esc blurs it before this listener runs
    const typing = !inBar && e.target.matches?.("textarea, input, select, [contenteditable], [contenteditable] *");
    if (mod && !e.shiftKey && !e.altKey && k === "z" && undo && !typing && !busy) { e.preventDefault(); revert(); return; }
    // Esc with nothing else to close: the bar
    if (e.key === "Escape" && !inBar && !typing && !document.querySelector(".menu:not([hidden]):not(.find-menu), .pop:not([hidden])")) {
      e.stopImmediatePropagation();
      close();
    }
  });

  /** The masked source (context.js maps a click in the drawn page back to it). */
  const mask = (src, o = {}) => visible(src, !!o.all);
  return { open, close, badge, visible, mask, HOLE, ESC, isOpen };
})();
window.PCFind = PCFind;
