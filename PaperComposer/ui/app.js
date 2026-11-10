// PaperComposer UI.
// Top bar: this paper (picker, references, export), the workspace (database,
// templates as windows) and the app (sync, settings). The writing toolbar sits
// over the page. Below that:
// the right AI & sync, appearance and light/dark.
// Left: navigator of sections and paragraphs. Centre: the whole paper.
// Right: one-shot writing box and checks for the current paragraph.

const ROLES = ["Abstract", "Introduction", "Related work", "Method", "Result", "Discussion", "Conclusion", "Any"];
const TABLES = [
  ["phrases.csv", "句型"],
  ["expressions.csv", "片語"],
  ["vocabulary.csv", "用詞"],
  ["rules.csv", "規則"],
  ["exemplars.csv", "範例"],
  ["equations.csv", "公式"],
  ["authors.csv", "作者"],
  ["affiliations.csv", "機構"],
];

const $ = (s) => document.querySelector(s);
const el = (tag, cls, text) => { const n = document.createElement(tag); if (cls) n.className = cls; if (text != null) n.textContent = text; return n; };
const pad = (n) => String(n).padStart(2, "0");
// Choices worth keeping across devices also go to the workspace's
// config.toml (through /api/settings), which syncs with the rest of it.
const SYNCED = ["theme", "mode", "dim", "zoom", "engine", "claudeVia", "navOpen", "paper", "tableStyle", "tplKind", "template", "newTemplate"];
const store = {
  get(k, d) { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } },
  set(k, v) {
    try { localStorage.setItem(k, JSON.stringify(v)); } catch {}
    if (SYNCED.includes(k.replace(/^pc\./, ""))) store.saveSoon();
  },
  saveSoon() {
    clearTimeout(store.t);
    store.t = setTimeout(() => {
      const ui = {};
      SYNCED.forEach((k) => { const v = store.get(`pc.${k}`, null); if (v !== null) ui[k] = v; });
      api("/api/settings", ui, "PUT").catch(() => {});
    }, 800);
  },
  /** Bring in config.toml; it wins over what this browser remembers. */
  async load() {
    let ui = {};
    try { ui = await api("/api/settings"); } catch { return; }
    Object.entries(ui).forEach(([k, v]) => { if (SYNCED.includes(k)) try { localStorage.setItem(`pc.${k}`, JSON.stringify(v)); } catch {} });
  },
};

const state = {
  workspace: null,
  paper: null,        // { name, sections: [{file, title, paragraphs}] }
  reports: [],        // per section: array of paragraph reports
  cur: { s: 0, p: 0 },
  profile: null,
  ruleText: {},
  engines: null,     // { local: {configured, status}, claude: {configured, model} }
  claude: null,
  composed: null,
  soft: {},           // soft-check results by section + paragraph text
  db: { name: "phrases.csv", tables: {}, dirty: new Set(), issues: {} },
};

async function api(path, body, method) {
  const opts = body !== undefined
    ? { method: method ?? "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }
    : undefined;
  const r = await fetch(path, opts);
  if (!r.ok) throw new Error((await r.text()) || `${r.status}`);
  const text = await r.text();
  return text ? JSON.parse(text) : null;
}

// ── Theme ──────────────────────────────────────────────
function applyTheme() {
  const t = store.get("pc.theme", "orange");
  const sys = matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  const m = store.get("pc.mode", sys);
  const root = document.documentElement;
  root.dataset.theme = t;
  root.dataset.mode = m;
  root.classList.toggle("dim-paper", store.get("pc.dim", false));
  document.querySelectorAll(".theme[data-theme]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.theme === t)));
  document.querySelectorAll("#lookMode button").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.v === m)));
  const dim = store.get("pc.dim", false) ? "dim" : "white";
  document.querySelectorAll("#lookPaper button").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.v === dim)));
}

// ── Paper ──────────────────────────────────────────────
const sections = () => state.paper?.sections ?? [];
const curSection = () => sections()[state.cur.s];
const roleOf = (title) => ROLES.find((r) => r.toLowerCase() === title.toLowerCase())
  ?? ROLES.find((r) => title.toLowerCase().startsWith(r.toLowerCase().slice(0, 5))) ?? "Any";
const rank = (s) => ({ info: 1, warning: 2, error: 3 }[s] ?? 0);

async function openPaper(name, { keepHome = false } = {}) {
  const same = state.paper?.name === name; // reopened (after a pull): keep the place
  if (!keepHome) PCHome.close();
  try {
    const res = await api(`/api/papers/${encodeURIComponent(name)}`);
    state.paper = { name, sections: res.sections, front: res.front ?? "", back: res.back ?? "", fields: res.fields ?? [], bibliography: res.bibliography ?? [], meta: res.meta ?? { key_value: "", authors: [] }, word_limits: res.word_limits ?? {} };
    PCTex.setMacros(res.macros ?? {});
    PCTex.setNumbers(res.numbers);
    PCTex.setFigureBase(`/api/papers/${encodeURIComponent(name)}/figure`);
    state.cur = { s: 0, p: 0 };
    store.set("pc.paper", name);
  } catch (e) {
    state.paper = null;
  }
  $("#paperName").textContent = state.paper?.name ?? "—";
  // the checks, and every figure's size (so its place is kept before it loads)
  await Promise.all([checkAll(), loadFigureSizes()]);
  if (!same) $("#pageArea").scrollTop = 0;
  render(!same);
}

/** Width and height of the paper's figures, read from their files by the server. */
async function loadFigureSizes() {
  if (!state.paper) return;
  const all = [state.paper.front, ...sections().flatMap((s) => s.paragraphs)].join("\n");
  const paths = [...new Set([...all.matchAll(/\\includegraphics\s*(?:\[[^\]]*\])?\s*\{([^}]*)\}/g)].map((m) => m[1].trim()))];
  if (!paths.length) return;
  try { PCTex.setFigureSizes(await api(`/api/papers/${encodeURIComponent(state.paper.name)}/figure-sizes`, { paths })); } catch {}
}

async function checkSection(i) {
  const s = sections()[i];
  if (!s) return;
  const text = s.paragraphs.join("\n\n");
  try { state.reports[i] = text.trim() ? await api("/api/check", { text, section: roleOf(s.title) }) : []; }
  catch { state.reports[i] = []; }
}
async function checkAll() {
  state.reports = [];
  await Promise.all(sections().map((_, i) => checkSection(i)));
}

async function saveSection(i) {
  const s = sections()[i];
  const saved = await api(`/api/papers/${encodeURIComponent(state.paper.name)}/sections/${encodeURIComponent(s.file)}`,
    { title: s.title, paragraphs: s.paragraphs }, "PUT");
  sections()[i] = saved;
  await checkSection(i);
}

// Drawing the paper. render() rebuilds everything: opening a paper, and
// changes that reach past one section file (title block, numbers from a
// compile, sync). An edit inside one section goes through refreshSection(),
// which redraws only the paragraphs whose text changed and then renumbers
// (equations, figures, tables, theorems, \ref texts) without drawing any
// formula again. Formulas and figures are drawn near the view only (lazy.js).
function render(fresh) {
  const all = () => {
    PCTex.number(sections(), state.paper?.front ?? "");
    renderNav(); renderKeyValue(); renderPaper(); renderSide(); renderStatus();
  };
  if (fresh) all(); else keepPlace(all);
  PCLazy.revealView($("#paper")); // what is shown is drawn before the first paint
}

/** Rebuild the page without moving the paragraph at the top of the view. */
function keepPlace(fn) {
  const area = $("#pageArea"), top = area.getBoundingClientRect().top;
  const at = [...document.querySelectorAll("#paper > .para")].find((p) => p.getBoundingClientRect().bottom > top);
  const place = at && { s: at.dataset.s, p: at.dataset.p, y: at.getBoundingClientRect().top - top };
  fn();
  const now = place && paraEl(place.s, place.p);
  if (!now) return;
  PCLazy.reveal(now);
  area.scrollTop += now.getBoundingClientRect().top - top - place.y;
}

/** After the paragraphs of section si changed (state already holds them). */
function refreshSection(si) {
  const paper = $("#paper");
  const sec = sections()[si];
  const head = document.getElementById(`sec-${si}`);
  if (!state.paper || !sec || !head) { render(); return; }
  PCTex.number(sections(), state.paper.front);
  const old = [...paper.querySelectorAll(`:scope > [data-s="${si}"]`)];
  const pool = new Map();
  old.forEach((e) => { if (!pool.has(e._sig)) pool.set(e._sig, []); pool.get(e._sig).push(e); });
  // keep every paragraph whose text (and check marks) did not change
  const nodes = sec.paragraphs.map((text, pi) => {
    const kept = pool.get(paraSig(si, pi, text))?.shift();
    if (kept) { kept.dataset.p = pi; return kept; }
    return paraNode(si, pi, text); // drawn at once: this is where the user is
  });
  if (!sec.paragraphs.length) nodes.push(addParaNode(si));
  const keep = new Set(nodes);
  old.forEach((e) => { if (!keep.has(e)) e.remove(); });
  let at = head;
  nodes.forEach((n) => { if (at.nextSibling !== n) at.after(n); at = n; });
  PCTex.renumber(paper);
  PCLazy.watch(paper, nodes);
  renderNavSection(si);
  updateNavNumbers();
  markCurrent();
  renderSide();
  renderStatus();
}

// The paper's key value over the page: what every paragraph should serve.
// Click to change it.
function renderKeyValue() {
  const bar = $("#keyValue");
  bar.innerHTML = "";
  bar.hidden = !state.paper;
  if (!state.paper) return;
  const kv = state.paper.meta?.key_value ?? "";
  bar.append(el("span", "k", "KEY VALUE"));
  const v = el("button", kv ? "v" : "v empty", kv || "尚未設定：點這裡填寫這篇論文通篇要強調的一件事");
  v.title = "點擊修改。生成段落與 R106 離題檢查都以它為準。";
  v.onclick = () => {
    const input = el("input", "kv-edit");
    input.value = kv;
    const count = el("span", "np-count");
    const recount = () => { const [n, max, unit] = PCNew.kvLength(input.value); count.textContent = `${n} / ${max} ${unit}`; count.classList.toggle("bad", n > max); };
    const save = async () => {
      if (input.value.trim() === kv) { renderKeyValue(); return; }
      try {
        state.paper.meta = await api(`/api/papers/${encodeURIComponent(state.paper.name)}/meta`, { key_value: input.value }, "PUT");
        state.soft = {};
        flash("已更新 Key value");
        refreshGit();
      } catch (e) { flash(e.message, 6000); }
      renderKeyValue();
    };
    input.oninput = recount;
    input.onkeydown = (e) => { if (e.key === "Enter") save(); if (e.key === "Escape") renderKeyValue(); };
    input.onblur = save;
    v.replaceWith(input, count);
    recount();
    input.focus();
  };
  bar.append(v);
}

// ── Navigator ──────────────────────────────────────────
// Each section file, and inside it a tree of its heading paragraphs
// (\section, \subsection, \subsubsection) with their paragraphs. Files and
// headings fold; a folded node's items are only created when it is opened,
// so a book's navigator holds what is shown, not two thousand labels.
// Open/closed is kept in pc.navOpen: files by file name (open unless closed),
// headings by "file#label" or "file#level:title#n" (closed unless opened).
const NAV_LEVEL = { section: 0, subsection: 1, subsubsection: 2 };
const navOpen = () => store.get("pc.navOpen", {});
function setNavOpen(changes) {
  const open = navOpen();
  Object.entries(changes).forEach(([k, v]) => { open[k] = v; });
  store.set("pc.navOpen", open);
}

/** One file's paragraphs as a tree: { children: [{ pi } | { pi, head, key, children, n }] }. */
function navTree(sec) {
  const root = { children: [], n: { para: 0, eq: 0, fig: 0, tab: 0 } };
  const stack = [{ level: -1, node: root }];
  const seen = {};
  sec.paragraphs.forEach((text, pi) => {
    const t = PCTex.stripComments(text).trim();
    if (!t) return; // commented-out paragraphs stay on the page (faint %), not in the outline
    const head = PCTex.heading(text);
    if (head) {
      const level = NAV_LEVEL[head[0]] ?? 1;
      while (stack[stack.length - 1].level >= level) stack.pop();
      const lab = t.match(/\\label\{([^}]*)\}/)?.[1];
      const id = lab ? lab : `${head[0]}:${head[1]}`;
      seen[id] = (seen[id] ?? 0) + 1;
      const node = { pi, head, key: `${sec.file}#${id}${lab ? "" : `#${seen[id]}`}`, children: [], n: { para: 0, eq: 0, fig: 0, tab: 0 } };
      stack[stack.length - 1].node.children.push(node);
      stack.push({ level, node });
      return;
    }
    stack[stack.length - 1].node.children.push({ pi });
    const kind = navKind(text)?.[0];
    stack.forEach(({ node }) => {
      node.n.para++;
      if (kind === "式") node.n.eq++;
      else if (kind === "圖") node.n.fig++;
      else if (kind === "表") node.n.tab++;
    });
  });
  return root;
}

const navCountTitle = (n) => [`${n.para} 段`, n.eq && `${n.eq} 式`, n.fig && `${n.fig} 圖`, n.tab && `${n.tab} 表`].filter(Boolean).join(" · ");

function renderNav() {
  $("#navTitle").textContent = state.paper?.name ?? "";
  const nav = $("#nav");
  nav.innerHTML = "";
  if (!state.paper) { nav.append(el("li", "empty", "從「檔案」開啟論文")); return; }
  if (state.paper.front?.trim()) {
    const li = el("li", "sec front");
    const head = el("button");
    head.append(el("span", "caret", ""), el("span", "", "標題與摘要"));
    head.onclick = () => PCLazy.jump(document.getElementById("front"), { block: "start", behavior: "smooth" });
    li.append(head);
    nav.append(li);
  }
  sections().forEach((sec, si) => nav.append(navSection(si)));
  PCLazy.watch(nav);
}

/** Redraw one file's part of the navigator (after its paragraphs changed). */
function renderNavSection(si) {
  const old = $(`#nav .sec[data-s="${si}"]`);
  if (!old) { renderNav(); return; }
  const li = navSection(si);
  old.replaceWith(li);
  PCLazy.watch($("#nav"), li.querySelectorAll("button"));
}

function navSection(si) {
  const sec = sections()[si];
  const tree = navTree(sec);
  const li = el("li", "sec");
  li.dataset.s = si;
  li.dataset.open = String(navOpen()[sec.file] !== false);
  const head = el("button", "sec-head");
  const title = el("span", "t");
  title.append(PCTex.render(sec.title));
  head.append(el("span", "caret", "▾"), title, el("span", "count", String(tree.n.para)));
  head.title = navCountTitle(tree.n);
  const ol = el("ol", "outline");
  const fill = () => { if (ol._filled) return; ol._filled = true; PCTex.lazily(() => navItems(ol, si, tree.children)); };
  if (li.dataset.open === "true") fill();
  li._tree = tree;
  li._fill = fill;
  head.onclick = (e) => {
    const now = li.dataset.open !== "true";
    if (e.altKey) setSubtree(li, tree, sec.file, now);
    li.dataset.open = String(now);
    setNavOpen({ [sec.file]: now });
    if (now) { fill(); PCLazy.watch($("#nav"), ol.querySelectorAll("button")); }
    PCLazy.jump(document.getElementById(`sec-${si}`), { block: "start", behavior: "smooth" });
  };
  li.append(head, ol);
  return li;
}

function navItems(ol, si, children) {
  children.forEach((c) => ol.append(c.head ? navHeading(si, c) : navLeaf(si, c.pi)));
}

function navLeaf(si, pi) {
  const text = sections()[si].paragraphs[pi];
  const b = el("button");
  b.dataset.s = si;
  b.dataset.p = pi;
  b.setAttribute("aria-current", String(si === state.cur.s && pi === state.cur.p));
  const rep = state.reports[si]?.[pi];
  // Label: rendered first line; blocks show their kind and number.
  const t = el("span", "t");
  const kind = navKind(text);
  if (kind) { t.textContent = kind; b.dataset.kind = "1"; } // updateNavNumbers rewrites it
  else t.append(PCTex.render(PCTex.stripComments(text).trim().split("\n")[0]));
  b.append(el("span", `status ${rep ? rep.status ?? "" : ""}`), el("span", "i", pad(pi + 1)), t);
  b.onclick = () => { const s = +b.dataset.s, p = +b.dataset.p; select(s, p); PCLazy.jump(paraEl(s, p)); };
  const item = el("li");
  item.append(b);
  return item;
}

function navHeading(si, node) {
  const file = sections()[si].file;
  const li = el("li", `sub lv${NAV_LEVEL[node.head[0]] ?? 1}`);
  li.dataset.open = String(navOpen()[node.key] === true);
  li.dataset.key = node.key;
  const b = el("button", "sub-head");
  b.dataset.s = si;
  b.dataset.p = node.pi;
  b.setAttribute("aria-current", String(si === state.cur.s && node.pi === state.cur.p));
  const t = el("span", "t");
  t.append(PCTex.render(node.head[1]));
  b.append(el("span", "caret", "▾"), t, el("span", "count", String(node.n.para)));
  b.title = navCountTitle(node.n);
  const ol = el("ol", "outline");
  const fill = () => { if (ol._filled) return; ol._filled = true; PCTex.lazily(() => navItems(ol, si, node.children)); };
  if (li.dataset.open === "true") fill();
  li._fill = fill;
  li._node = node;
  // like a file: a click folds or opens it and goes to the heading; Alt+click does the whole subtree
  b.onclick = (e) => {
    const now = li.dataset.open !== "true";
    if (e.altKey) setSubtree(li, node, file, now);
    li.dataset.open = String(now);
    setNavOpen({ [node.key]: now });
    if (now) { fill(); PCLazy.watch($("#nav"), ol.querySelectorAll("button")); }
    const s = +b.dataset.s, p = +b.dataset.p;
    select(s, p);
    PCLazy.jump(paraEl(s, p), { block: "start", behavior: "smooth" });
  };
  li.append(b, ol);
  return li;
}

/** Open or close every heading under a node (Alt+click), then redraw it. */
function setSubtree(li, node, file, open) {
  const changes = {};
  const walk = (n) => n.children.forEach((c) => { if (c.head) { changes[c.key] = open; walk(c); } });
  walk(node);
  setNavOpen(changes);
  const ol = li.querySelector(":scope > ol");
  ol.innerHTML = "";
  ol._filled = false;
  if (open || li.dataset.open === "true") { li._fill(); PCLazy.watch($("#nav"), ol.querySelectorAll("button")); }
}

/** Open or close every file and heading of the paper. */
function setAllNav(open) {
  const changes = {};
  sections().forEach((sec) => {
    changes[sec.file] = open;
    const walk = (n) => n.children.forEach((c) => { if (c.head) { changes[c.key] = open; walk(c); } });
    walk(navTree(sec));
  });
  setNavOpen(changes);
  renderNav();
  markCurrent();
}

/** Open the files and headings above paragraph (si, pi) so its item exists. */
function openNavPath(si, pi) {
  const li = $(`#nav .sec[data-s="${si}"]`);
  if (!li || !PCTex.stripComments(sections()[si]?.paragraphs[pi] ?? "").trim()) return null; // comments are not listed
  const opened = {};
  if (li.dataset.open !== "true") { li.dataset.open = "true"; opened[sections()[si].file] = true; }
  li._fill();
  let ol = li.querySelector(":scope > ol");
  for (;;) {
    const hit = ol.querySelector(`:scope > li > button[data-s="${si}"][data-p="${pi}"]`);
    if (hit) { if (Object.keys(opened).length) setNavOpen(opened); return hit; }
    // the heading whose span holds pi: the last one at this level starting at or before it
    const subs = [...ol.querySelectorAll(":scope > li.sub")].filter((s) => s._node.pi <= pi);
    const sub = subs[subs.length - 1];
    if (!sub) return null;
    if (sub.dataset.open !== "true") { sub.dataset.open = "true"; opened[sub.dataset.key] = true; }
    sub._fill();
    ol = sub.querySelector(":scope > ol");
  }
}

/** Equation, figure and table numbers in the navigator's labels, after renumbering. */
function updateNavNumbers() {
  const nav = $("#nav");
  nav.querySelectorAll('button[data-kind="1"]').forEach((b) => {
    const t = navKind(sections()[+b.dataset.s]?.paragraphs[+b.dataset.p] ?? "");
    const label = b.querySelector(".t");
    if (t && label.textContent !== t) label.textContent = t;
  });
  PCTex.updateRefs(nav);
}

// Navigator label for a block paragraph: its kind, number and title.
function navKind(text) {
  if (!PCTex.stripComments(text).trim()) return "註解";
  const head = PCTex.heading(text);
  if (head) return head[1];
  const t = PCTex.stripComments(text).trim();
  const env = t.match(/^\\begin\{([a-zA-Z*]+)\}/)?.[1];
  if (!env) return null;
  const lab = t.match(/\\label\{([^}]*)\}/)?.[1];
  const n = lab && PCTex.labels[lab] ? ` ${PCTex.labels[lab]}` : "";
  const base = env.replace("*", "");
  if (/^(equation|align|gather|multline|eqnarray|flalign|alignat)$/.test(base)) return `式${n ? ` (${n.trim()})` : ""}`;
  if (/^(figure|wrapfigure)$/.test(base)) return /\\begin\{tabular/.test(t) && !/\\includegraphics/.test(t) ? `表${n}` : `圖${n}`;
  if (/^(table|sidewaystable)$/.test(base)) return `表${n}`;
  if (/^algorithm/.test(base)) return `演算法${n}`;
  if (/^(itemize|enumerate|description)$/.test(base)) return "清單";
  if (/^(theorem|lemma|proposition|corollary|definition|remark|assumption|example|proof)$/.test(base)) return `${base[0].toUpperCase()}${base.slice(1)}${n}`;
  return null;
}

const paraEl = (si, pi) => document.querySelector(`#paper > .para[data-s="${si}"][data-p="${pi}"]`);

function renderPaper() {
  const paper = $("#paper");
  if (!state.paper) { paper.innerHTML = ""; return; }
  PCTex.resetCounters();
  const page = document.createDocumentFragment();
  PCTex.lazily(() => {
    // main.tex around the sections: title block first, back matter and references last
    if (state.paper.front.trim()) {
      const front = PCTex.frontNode(state.paper.front);
      front.id = "front";
      front.querySelectorAll("[data-field]").forEach((n) => (n.ondblclick = (e) => { e.stopPropagation(); editField(n); }));
      page.append(front);
    }
    sections().forEach((sec, si) => {
      const h = el("h2");
      h.append(PCTex.render(sec.title));
      h.id = `sec-${si}`;
      page.append(h);
      sec.paragraphs.forEach((text, pi) => page.append(paraNode(si, pi, text)));
      if (!sec.paragraphs.length) page.append(addParaNode(si));
    });
    const back = state.paper.back.trim();
    if (back) { const d = el("div", "tex-back"); d.append(PCTex.render(back)); if (d.textContent.trim()) page.append(d); }
    if (state.paper.bibliography.length) page.append(PCTex.bibliographyNode(state.paper.bibliography));
  });
  paper.replaceChildren(page);
  PCLazy.watch(paper);
  markCurrent();
}

/** What a paragraph's drawing depends on: its text and, for prose, its check marks. */
function paraSig(si, pi, text) {
  if (PCTable.isTable(text)) return `T${pi}\u0001${text}`; // the table editor holds its place
  const rep = state.reports[si]?.[pi];
  return rep ? `${text}\u0001${JSON.stringify([rep.sentence_text, rep.findings.map((f) => [f.sentence, f.severity])])}` : text;
}

/** One paragraph of the page. Handlers read data-s/data-p, so a kept paragraph can move. */
function paraNode(si, pi, text) {
  const at = (p) => [+p.dataset.s, +p.dataset.p];
  let p;
  if (PCTable.isTable(text)) {
    p = el("div", "para");
    p.dataset.s = si;
    p.dataset.p = pi;
    PCTable.mount(p, si, pi, text);
  } else if (!PCTex.stripComments(text).trim()) {
    // only comments: keep a handle to edit it, nothing to read
    p = el("div", "para comment-only", "%");
    p.title = "註解";
  } else {
    const head = PCTex.heading(text);
    const block = PCTex.isDisplay(text);
    const whole = block || PCTex.needsWhole(text);
    p = el(head ? "div" : block ? "div" : "p", head ? `para head ${head[0]}` : block ? "para display" : "para");
    if (head) {
      p.append(PCTex.render(head[1]));
    } else if (whole) {
      // Blocks, and prose with comments or line breaks: render as a whole, without sentence marks.
      p.append(PCTex.render(text));
    } else {
      const rep = state.reports[si]?.[pi];
      const flagged = new Map();
      rep?.findings.forEach((f) => {
        if (f.sentence == null) return;
        if (rank(f.severity) > rank(flagged.get(f.sentence))) flagged.set(f.sentence, f.severity);
      });
      (rep?.sentence_text ?? [text.replace(/\s+/g, " ")]).forEach((s, i) => {
        const span = el("span", "s" + (flagged.has(i) ? ` flag ${flagged.get(i)}` : ""));
        span.append(PCTex.render(s), " ");
        p.append(span);
      });
    }
  }
  p.dataset.s = si;
  p.dataset.p = pi;
  p._sig = paraSig(si, pi, text);
  if (!p.classList.contains("tbl")) {
    p.onclick = () => select(...at(p));
    p.ondblclick = () => edit(p, ...at(p));
  }
  return p;
}

function addParaNode(si) {
  const p = el("p", "para add-para", "雙擊新增段落");
  p.dataset.s = si;
  p._sig = "\u0002add";
  p.ondblclick = () => {
    const s = +p.dataset.s;
    sections()[s].paragraphs.push("");
    refreshSection(s);
    edit(paraEl(s, 0), s, 0);
  };
  return p;
}

/** Make (si, pi) the current paragraph; its item in the navigator is opened and shown. */
function select(si, pi) {
  state.cur = { s: si, p: pi };
  const b = openNavPath(si, pi);
  markCurrent();
  if (b) {
    PCLazy.watch($("#nav"), $(`#nav .sec[data-s="${si}"]`)?.querySelectorAll("button") ?? []);
    // keep it in sight in the navigator's own scroll
    const pane = $("aside.left"), r = b.getBoundingClientRect(), pr = pane.getBoundingClientRect();
    if (r.top < pr.top || r.bottom > pr.bottom) pane.scrollTop += r.top - pr.top - pr.height / 2;
  }
  renderSide();
}

function markCurrent() {
  document.querySelectorAll("#paper > .para.current").forEach((p) => p.classList.remove("current"));
  paraEl(state.cur.s, state.cur.p)?.classList.add("current");
  document.querySelectorAll('#nav button[aria-current="true"]').forEach((b) => b.setAttribute("aria-current", "false"));
  $(`#nav button[data-s="${state.cur.s}"][data-p="${state.cur.p}"]`)?.setAttribute("aria-current", "true");
}

function edit(p, si, pi) {
  if (!p) return;
  select(si, pi);
  const ta = document.createElement("textarea");
  ta.value = sections()[si].paragraphs[pi] ?? "";
  const fit = () => { ta.style.height = "auto"; ta.style.height = ta.scrollHeight + "px"; };
  ta.oninput = fit;
  ta.onkeydown = (e) => { if (e.key === "Escape" || (e.key === "Enter" && (e.metaKey || e.ctrlKey))) ta.blur(); };
  ta.onblur = async () => {
    const v = ta.value.trim();
    const list = sections()[si].paragraphs;
    if (v) list[pi] = v; else list.splice(pi, 1);
    try { await saveSection(si); } catch (e) { flash(`儲存失敗：${e.message}`); }
    refreshSection(si);
    refreshGit();
  };
  p._sig = null; // holds the editor now: never kept as it is
  p.replaceChildren(ta);
  p.ondblclick = null;
  fit();
  // the writing toolbar is sticky: keep the editor's top below it (.page-area scroll-padding in styles.css)
  ta.focus({ preventScroll: true });
  ta.scrollIntoView({ block: "nearest" });
}

// Edit one field of the title block in place: its LaTeX, saved into main.tex.
function editField(n) {
  const field = n.dataset.field, index = +n.dataset.index;
  const cur = state.paper.fields.find((f) => f.field === field && f.index === index)?.value ?? "";
  const ta = document.createElement("textarea");
  ta.className = "field-edit";
  ta.value = cur;
  const fit = () => { ta.style.height = "auto"; ta.style.height = ta.scrollHeight + "px"; };
  ta.oninput = fit;
  ta.onkeydown = (e) => { if (e.key === "Escape" || (e.key === "Enter" && (e.metaKey || e.ctrlKey))) ta.blur(); };
  ta.onblur = async () => {
    const v = ta.value.trim();
    if (v !== cur.trim()) {
      try {
        const r = await api(`/api/papers/${encodeURIComponent(state.paper.name)}/front`, { field, index, value: v }, "PUT");
        Object.assign(state.paper, { front: r.front, back: r.back, fields: r.fields });
        refreshGit();
      } catch (e) { flash(`儲存失敗：${e.message}`, 6000); }
    }
    render();
  };
  n.replaceChildren(ta);
  n.ondblclick = null;
  fit();
  ta.focus();
}

// ── Right panel ────────────────────────────────────────
function renderSide() { renderFindings(); syncCompose(); }

// Check panel: the selected paragraph is brought in; hard rules show at
// once, soft rules (rules.csv, type soft) run on the model only when asked.
const softKey = () => {
  const sec = curSection();
  return sec ? `${roleOf(sec.title)}\n${sec.paragraphs[state.cur.p] ?? ""}` : "";
};

function renderFindings() {
  const ul = $("#findings");
  ul.innerHTML = "";
  const head = $("#chkHead");
  head.innerHTML = "";
  const box = $("#softBox");
  box.innerHTML = "";
  const sec = curSection();
  const r = state.reports[state.cur.s]?.[state.cur.p];
  if (!r) { ul.append(el("li", "empty", "選一個段落")); return; }
  head.append(el("span", "acc", `${sec.title} · ${pad(state.cur.p + 1)}`), el("span", "", `${r.sentences} 句 · ${r.words} 字`));

  // hard rules
  if (!r.findings.length) { const li = el("li"); li.append(el("span", "status"), el("span", "m", "格式規則都通過")); ul.append(li); }
  r.findings.forEach((f) => ul.append(findingItem(f.severity, f.message, null, f.rule, f.sentence)));

  // soft rules
  if (!r.pending.length) return;
  const done = state.soft[softKey()];
  const list = el("ul", "findings soft");
  if (done?.results) {
    done.results.forEach((x) => list.append(findingItem(x.pass ? "ok" : x.severity, x.message, x.reason, x.rule, x.sentence)));
  } else {
    r.pending.forEach((id) => list.append(findingItem("info", state.ruleText[id] ?? id, null, id, null, true)));
  }
  const bar = el("div", "soft-bar");
  const go = el("button", "primary small", done?.busy ? "檢查中…" : done?.results ? "重新檢查" : `檢查 ${r.pending.length} 條規則`);
  go.disabled = !!done?.busy;
  go.onclick = () => softCheck(done?.results ? true : false);
  bar.append(el("span", done?.error ? "note bad" : "note", done?.error ? (done.error === "no engine" ? "先連接引擎" : `失敗：${done.error}`) : "軟規則"), go);
  box.append(bar, list);
}

function findingItem(status, message, reason, rule, sentence, pending) {
  const li = el("li", pending ? "pending" : "");
  const body = el("span");
  body.append(el("span", "m", message));
  if (sentence != null) body.append(el("span", "where", `第 ${sentence + 1} 句`));
  body.append(el("span", "rid", rule));
  if (reason) body.append(el("span", "why", reason));
  li.append(el("span", `status ${status === "ok" ? "" : status}`), body);
  if (sentence != null) { li.classList.add("go"); li.onclick = () => pointAt(state.cur.s, state.cur.p, sentence); }
  return li;
}

/** Flash one sentence of a paragraph in the paper. */
function pointAt(si, pi, i) {
  const s = paraEl(si, pi)?.querySelectorAll(".s")[i];
  if (!s) return;
  PCLazy.jump(s);
  s.classList.remove("blink"); void s.offsetWidth; s.classList.add("blink");
}

async function softCheck(again) {
  const key = softKey();
  const sec = curSection();
  if (!key || !sec) return;
  const list = sec.paragraphs;
  const p = state.cur.p;
  state.soft[key] = { busy: true };
  renderFindings();
  let res;
  try {
    res = await api("/api/check/soft", {
      text: list[p], section: roleOf(sec.title),
      before: list[p - 1] ?? null, after: list[p + 1] ?? null,
      engine: chosenEngine(),
      key_value: state.paper?.meta?.key_value || null,
    });
  } catch (e) { res = { error: e.message }; }
  state.soft[key] = res.error ? { error: res.error } : { results: res.results };
  if (!again && res.cached) flash("沿用上次的檢查結果");
  // Only redraw if the user is still on this paragraph.
  if (softKey() === key) renderFindings();
}

function syncCompose() {
  const sel = $("#cSection");
  if (!sel.options.length) ROLES.forEach((r) => sel.append(new Option(r, r)));
  const sec = curSection();
  if (sec) sel.value = roleOf(sec.title);
}

const seg = (id) => $(`#${id} [aria-pressed="true"]`)?.dataset.v;

// ── Writing engine switch ──────────────────────────────
// Two engines in the UI: local, and Claude. Claude connects one of two ways,
// chosen by the user: their claude.ai login through Claude Code
// ("subscription") or an API key ("api"). The server is told which.
const VIA = { subscription: "訂閱", api: "API" };

/** How Claude connects: the saved choice, else whichever is set up. */
function claudeVia() {
  const e = state.engines;
  const want = store.get("pc.claudeVia", null);
  if (want && (!e || e[want]?.configured)) return want;
  if (e?.subscription.configured) return "subscription";
  if (e?.api.configured) return "api";
  return want ?? "subscription";
}
const claudeReady = () => !!state.engines?.[claudeVia()]?.configured;

/** The engine sent to the server: "local", "subscription", "api" or null. */
function chosenEngine() {
  const e = state.engines;
  if (!e) return null;
  const want = store.get("pc.engine", null);
  if (want === "local" && e.local.configured) return "local";
  if (want === "claude" && claudeReady()) return claudeVia();
  return e.local.configured ? "local" : claudeReady() ? claudeVia() : null;
}

async function loadEngines(fresh = false) {
  try { state.engines = await api(`/api/engines${fresh ? "?fresh=1" : ""}`); } catch { state.engines = null; }
  renderEngineSwitch();
  renderStatus();
  markSettings();
}

/** claude-sonnet-5-5 → Sonnet 5.5 */
const modelName = (id) => {
  const [, family = id, ...ver] = String(id).split("-");
  return `${family[0].toUpperCase()}${family.slice(1)} ${ver.join(".")}`.trim();
};

/** Place the shared engine menu under its button. */
function showEngineMenu(m, btn) {
  m.hidden = false;
  const r = btn.getBoundingClientRect();
  m.style.top = `${r.bottom + 4}px`;
  m.style.left = `${Math.max(8, Math.min(r.left, innerWidth - m.offsetWidth - 8))}px`;
}

/** Local menu: the downloaded models; picking one starts it in place of the running one. */
function toggleLocalMenu(btn) {
  const m = $("#modelMenu");
  if (!m.hidden && m.dataset.for === "local") { m.hidden = true; return; }
  m.dataset.for = "local";
  const loc = state.engines.local;
  m.innerHTML = "";
  m.append(el("div", "hm-title", "本機模型"));
  loc.models.forEach((x) => {
    const b = el("button", "", x.name);
    if (x.uncensored) b.append(el("span", "tag warn", "Uncensored"));
    b.append(el("span", "v", x.variant.replace(/^abliterated /, "")));
    b.setAttribute("aria-current", String(x.id === loc.running));
    b.title = x.summary;
    b.onclick = async () => {
      m.hidden = true;
      store.set("pc.engine", "local");
      if (x.id !== loc.running) {
        flash(`啟動 ${x.name}…`, 120000);
        try { await api("/api/models/start", { id: x.id }); flash(`已改用 ${x.name} ${x.variant}`); }
        catch (err) { flash(err.message, 8000); }
      }
      loadEngines();
    };
    m.append(b);
  });
  m.append(el("div", "sep"));
  const more = el("button", "", "下載或管理模型…");
  more.onclick = () => { m.hidden = true; Setup.open("model"); };
  m.append(more);
  showEngineMenu(m, btn);
}

/** Claude menu: the model, and how Claude connects. */
function toggleModelMenu(btn) {
  const m = $("#modelMenu");
  if (!m.hidden && m.dataset.for === "claude") { m.hidden = true; return; }
  m.dataset.for = "claude";
  const e = state.engines;
  m.innerHTML = "";
  m.append(el("div", "hm-title", "模型"));
  e.models.forEach((id) => {
    const b = el("button", "", modelName(id));
    b.setAttribute("aria-current", String(id === e.model));
    b.onclick = async () => {
      m.hidden = true;
      try { await api("/api/settings/claude", { model: id }); flash(`已改用 ${modelName(id)}`); }
      catch (err) { flash(err.message, 6000); }
      loadEngines();
    };
    m.append(b);
  });
  m.append(el("div", "sep"), el("div", "hm-title", "連線方式"));
  [["subscription", "登入帳號（訂閱）"], ["api", "API key"]].forEach(([via, label]) => {
    const ready = !!e[via]?.configured;
    const b = el("button", ready ? "" : "off", ready ? label : `${label}・未設定`);
    b.setAttribute("aria-current", String(via === claudeVia() && ready));
    b.onclick = () => {
      m.hidden = true;
      if (!ready) { setTimeout(() => Setup.open("claude", via)); return; }
      store.set("pc.claudeVia", via);
      store.set("pc.engine", "claude");
      flash(`Claude 改用${label}`);
      renderEngineSwitch();
      renderStatus();
    };
    m.append(b);
  });
  showEngineMenu(m, btn);
}

function renderEngineSwitch() {
  const pick = chosenEngine();
  const e = state.engines;
  $('#cEngine [data-v="claude"] .lbl').textContent = claudeReady() ? `Claude ${modelName(e.model)}` : "Claude";
  const run = e?.local.models?.find((x) => x.id === e.local.running);
  $('#cEngine [data-v="local"] .lbl').textContent = run ? run.name : "本機";
  document.querySelectorAll("#cEngine button").forEach((b) => {
    const v = b.dataset.v;
    // Local counts as available once a model is downloaded: the menu starts it.
    const ready = v === "local" ? !!e?.local.configured || !!e?.local.models?.length : claudeReady();
    b.classList.toggle("off", !ready);
    b.setAttribute("aria-pressed", String(v === "local" ? pick === "local" : pick != null && pick !== "local"));
    b.title = ready ? "" : v === "local" ? "尚未部署本機模型，點此設定" : "尚未連接 Claude，點此設定";
    // Unavailable: take the user to where it gets set up.
    b.onclick = () => {
      if (!ready) { Setup.open(v === "local" ? "model" : "claude"); return; }
      store.set("pc.engine", v);
      renderEngineSwitch();
      renderStatus();
      // Both offer their models; Claude also its connection.
      if (v === "claude") toggleModelMenu(b); else toggleLocalMenu(b);
    };
  });
}

function slotted(template) {
  const frag = document.createDocumentFragment();
  template.split(/(\{[^}]*\})/).forEach((part) => {
    if (!part) return;
    if (part.startsWith("{")) frag.append(el("i", "", part)); else frag.append(part);
  });
  return frag;
}

async function compose() {
  const points = $("#points").value.trim();
  if (!points) { $("#points").focus(); return; }
  const list = curSection()?.paragraphs ?? [];
  const body = {
    points,
    section: $("#cSection").value,
    function: $("#cFunction").value || null,
    min_words: Number($("#cMin").value) || 120,
    max_words: Number($("#cMax").value) || 180,
    language: seg("cLang"),
    engine: chosenEngine(),
    // Neighbours, so the paragraph links into its place in the text.
    before: list[state.cur.p] ?? null,
    after: list[state.cur.p + 1] ?? null,
    key_value: state.paper?.meta?.key_value || null,
  };
  const btn = $("#generate");
  btn.disabled = true;
  btn.firstElementChild.textContent = "生成中";
  try { state.composed = await api("/api/compose", body); }
  catch (e) { state.composed = { error: e.message, references: [] }; }
  finally { btn.disabled = false; btn.firstElementChild.textContent = "生成"; }
  renderResult();
}

function renderResult() {
  const box = $("#result");
  box.innerHTML = "";
  const r = state.composed;
  if (!r) return;
  const wrap = el("div", "result");
  if (r.draft) {
    const meta = el("div", "meta");
    const cov = el("span"); cov.append(el("b", "", `${r.check.covered}/${r.check.total}`), " 句用句型庫");
    meta.append(cov, el("span", "", `${r.check.words} 字`));
    wrap.append(meta);
    const d = el("div", "draft");
    r.draft.sentences.forEach((s) => {
      const span = el("span", "sent");
      span.append(PCTex.render(s.text));
      const ref = r.references.find((h) => h.phrase.id === s.phrase_id);
      if (ref) span.title = ref.phrase.template;
      d.append(span, el("sup", s.phrase_id ? "" : "free", s.phrase_id ?? "自由"), " ");
    });
    wrap.append(d);
    if (r.check.problems.length) {
      const ul = el("ul", "problems");
      r.check.problems.forEach((p) => ul.append(el("li", "", p)));
      wrap.append(ul);
    }
    const text = r.draft.sentences.map((s) => s.text).join("\n");
    const acts = el("div", "actions");
    acts.append(
      iconButton("insert", "插入到目前段落之後", () => placeDraft(text, "insert")),
      iconButton("replace", "取代目前段落", () => placeDraft(text, "replace")),
      iconButton("redo", "重新生成", compose),
      iconButton("copy", "複製", () => navigator.clipboard?.writeText(text.replace(/\n/g, " "))),
    );
    wrap.append(acts);
  } else {
    if (r.error === "no engine") {
      const go = el("button", "ghost", "連接引擎");
      go.onclick = () => Setup.open("claude");
      wrap.append(go);
    } else wrap.append(el("p", "note bad", `生成失敗：${r.error}`));
    if (r.prompt) {
      const acts = el("div", "actions");
      acts.append(iconButton("copy", "複製提示詞", () => navigator.clipboard?.writeText(`${r.prompt.system}\n\n${r.prompt.user}`)));
      wrap.append(acts);
    }
  }
  if (r.references?.length) {
    const ul = el("ul", "refs");
    r.references.forEach(({ phrase }) => {
      const li = el("li"); const t = el("span"); t.append(slotted(phrase.template));
      li.append(el("span", "", phrase.id), t); ul.append(li);
    });
    if (r.draft) { const det = el("details"); det.append(el("summary", "note", `參考句型 ${r.references.length}`), ul); wrap.append(det); }
    else wrap.append(ul);
  }
  box.append(wrap);
}

async function placeDraft(text, how) {
  const sec = curSection();
  if (!sec) return;
  if (how === "replace" && sec.paragraphs.length) sec.paragraphs[state.cur.p] = text;
  else {
    const at = sec.paragraphs.length ? state.cur.p + 1 : 0;
    sec.paragraphs.splice(at, 0, text);
    state.cur.p = at;
  }
  try { await saveSection(state.cur.s); } catch (e) { flash(`儲存失敗：${e.message}`); }
  refreshSection(state.cur.s);
  refreshGit();
}

function renderStatus() {
  const paras = sections().reduce((n, s) => n + s.paragraphs.length, 0);
  const sec = curSection();
  const bar = $("#statusbar");
  bar.innerHTML = "";
  bar.append(el("span", "accent", sec ? `${pad(state.cur.s + 1)} / ${sec.title}` : "—"), el("span", "", `${paras} 段`));
  if (state.paper) bar.lastChild.append(" · ", PCWords.button()); else PCWords.close();
  const e = state.engines, pick = chosenEngine();
  const eng = el("button", "sync"); const dot = el("span", "dot");
  const [ok, name] = pick === "local" ? [e.local.status.connected, e.local.status.name]
    : pick ? [true, `Claude ${VIA[pick]} · ${modelName(e.model)}`] : [false, "連接引擎"];
  dot.style.background = ok ? "var(--ok)" : "var(--faint)";
  eng.append(dot, name);
  eng.title = "寫作引擎（右側可切換）";
  eng.onclick = () => Setup.open(pick && pick !== "local" ? "claude" : "model");
  bar.append(eng, PCZoom.control());
  paintSync();
}

// The gear carries a dot while the app still needs setting up: no workspace
// of the user's own, or no model to write with.
function markSettings() {
  const need = !state.profile || state.profile.demo || !state.engines || !(state.engines.local.status.connected || state.engines.subscription.configured || state.engines.api.configured);
  $("#actSettings").classList.toggle("attn", !!need);
  $("#actSettings").setAttribute("aria-label", state.profile?.demo ? "設定（還沒連接你的工作區）" : need ? "設定（還沒連接模型）" : "設定");
}

// ── Git sync ───────────────────────────────────────────
// The cloud in the top bar: synced, changes waiting, or not set up.
function paintSync() {
  const g = state.git;
  const b = $("#actSync");
  let label, st;
  if (state.syncing) { label = "同步中"; st = "busy"; }
  else if (!g?.repo) { label = "設定同步"; st = "off"; }
  else if (g.changed.length) { label = `${g.changed.length} 項未同步　Ctrl+S`; st = "pending"; }
  else if (g.ahead) { label = `${g.ahead} 個 commit 未推送　Ctrl+S`; st = "pending"; }
  else if (!g.remote) { label = "已存檔（沒有遠端）"; st = "ok"; }
  else { label = state.lastSync ? `已同步 ${state.lastSync}` : "已同步"; st = "ok"; }
  b.dataset.state = st;
  b.dataset.icon = st === "off" ? "cloudOff" : "cloud";
  paintIcons(b.parentElement);
  b.setAttribute("aria-label", label);
  b.disabled = !!state.syncing;
}

async function refreshGit() {
  try { state.git = await api("/api/git/status"); } catch { state.git = null; }
  renderStatus();
}

async function syncNow() {
  if (!state.git?.repo || state.syncing) return;
  // Finish any paragraph being edited first, so it is part of the commit.
  const active = document.activeElement;
  if (active?.tagName === "TEXTAREA" && active.closest(".para")) { active.blur(); await new Promise((r) => setTimeout(r, 200)); }
  if (state.db.dirty.size) await saveDb();
  state.syncing = true;
  renderStatus();
  try {
    const r = await api("/api/git/sync", {});
    state.git = r.status;
    state.lastSync = new Date().toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit", hour12: false });
    if (r.pulled && state.paper) await openPaper(state.paper.name, { keepHome: true });
    if (r.pulled) PCHome.refresh();
    flash(r.committed ? `已同步：${r.committed}` : "沒有新的修改");
  } catch (e) {
    flash(e.message, 8000);
    await refreshGit();
  } finally {
    state.syncing = false;
    renderStatus();
  }
}

let flashTimer;
function flash(msg, ms = 3000) {
  let t = $("#flash");
  if (!t) { t = el("div", "tip flash"); t.id = "flash"; }
  // An open modal dialog sits in the top layer; anything outside it is hidden behind.
  ($("dialog[open]") ?? document.body).append(t);
  t.textContent = msg;
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => t.remove(), ms);
}

// ── Windows ────────────────────────────────────────────
// The paper is always the page. Workspace tools (database, templates) and
// this paper's references open as windows over it; Esc or the backdrop
// closes them.
const dbOpen = () => $("#dlgDb").open;

function openWindow(id) {
  const d = $(`#${id}`);
  if (!d.open) d.showModal();
  if (id === "dlgTpl") PCTemplates.open();
  if (id === "dlgPaperRefs") PCRefs.paperPage();
  if (id === "dlgDraft") PCDraft.open();
}

// The database window: one tab per table, and the reference library.
function openDbTab(name) {
  state.db.name = name;
  if (!dbOpen()) $("#dlgDb").showModal();
  openDb();
}

// Closing with unsaved database edits asks first.
function closeWindow(d) {
  if (d.id === "dlgDb" && state.db.dirty.size && !confirm("資料庫有未儲存的修改，仍要關閉？")) return;
  if (d.id === "dlgNew" && PCNew.dirty() && !confirm("新論文還沒建立，填的內容會不見，仍要關閉？")) return;
  d.close();
}

function wireWindows() {
  document.querySelectorAll("dialog.dialog:not(#dlgSetup)").forEach((d) => {
    d.querySelector(".close")?.addEventListener("click", () => closeWindow(d));
    // A click on the backdrop lands on the <dialog> itself, not its content.
    // Windows you fill in (new paper) stay open on a stray click outside.
    if (d.id !== "dlgNew" && d.id !== "dlgDraft") d.addEventListener("click", (e) => { if (e.target === d) closeWindow(d); });
    d.addEventListener("cancel", (e) => { e.preventDefault(); closeWindow(d); });
  });
}

// ── Hover menus on the top-bar icons ───────────────────
// Clicking an icon does its main thing; hovering offers the finer entries.
const MENUS = {
  db: () => [
    ...TABLES.map(([name, label]) => [label, () => openDbTab(name), dbOpen() && state.db.name === name]),
    ["文獻庫", () => openDbTab("@refs"), dbOpen() && state.db.name === "@refs"],
    "-",
    ["拆解好句子…", () => { openDbTab(state.db.name === "@refs" ? TABLES[0][0] : state.db.name); setTimeout(() => $("#dcText").focus(), 50); }],
  ],
  export: () => [
    ["PDF　Ctrl+P", () => PCExport.run("pdf")],
    ["Word", () => PCExport.run("docx")],
    ["LaTeX（zip）", () => PCExport.run("latex")],
    "-",
    ["在資料夾中顯示", PCExport.reveal],
  ],
};

const hm = { btn: null, timer: null };
function openHoverMenu(btn) {
  const m = $("#hoverMenu");
  m.innerHTML = "";
  m.append(el("div", "hm-title", btn.getAttribute("aria-label")));
  MENUS[btn.dataset.menu]().forEach((it) => {
    if (it === "-") { m.append(el("div", "sep")); return; }
    const [label, fn, current] = it;
    const b = el("button", "", label);
    b.setAttribute("aria-current", String(!!current));
    // Run after this click has finished bubbling: the document click handler
    // closes open menus, and would close the one this item just opened.
    b.onclick = () => { closeHoverMenu(true); setTimeout(fn); };
    m.append(b);
  });
  m.hidden = false;
  const r = btn.getBoundingClientRect();
  m.style.top = `${r.bottom + 4}px`;
  m.style.left = `${Math.max(8, Math.min(r.left, innerWidth - m.offsetWidth - 8))}px`;
  hm.btn = btn;
}
function closeHoverMenu(now) {
  clearTimeout(hm.timer);
  const go = () => { $("#hoverMenu").hidden = true; hm.btn = null; };
  if (now) go(); else hm.timer = setTimeout(go, 220);
}
function wireHoverMenus() {
  document.querySelectorAll("[data-menu]").forEach((b) => {
    b.addEventListener("mouseenter", () => {
      clearTimeout(hm.timer);
      if (hm.btn === b) return;
      hm.timer = setTimeout(() => openHoverMenu(b), hm.btn ? 0 : 120);
    });
    b.addEventListener("mouseleave", () => closeHoverMenu());
    b.addEventListener("click", () => closeHoverMenu(true));
  });
  const m = $("#hoverMenu");
  m.addEventListener("mouseenter", () => clearTimeout(hm.timer));
  m.addEventListener("mouseleave", () => closeHoverMenu());
}

// A small form in the paper menu, under `anchor` (the paper button by
// default): back closes it, a title, an optional "?".
function menuForm(title, tip, anchor = $("#actFile")) {
  const m = $("#fileMenu");
  m.className = "menu form";
  m.innerHTML = "";
  const r = anchor.getBoundingClientRect();
  m.style.top = `${r.bottom + 4}px`;
  m.style.left = `${r.left}px`;
  m.hidden = false;
  const head = el("div", "form-head");
  head.append(iconButton("back", "返回", () => { m.hidden = true; }), el("b", "", title));
  if (tip) { const h = el("button", "help", "?"); h.dataset.tip = tip; head.append(h); }
  m.append(head);
  return m;
}

function importForm(anchor) {
  const m = menuForm("匯入論文", "現有的 LaTeX 論文：壓縮檔（.zip、.tar.gz）、單一 .tex、整個資料夾，或 arXiv 上的論文（下載作者上傳的原始檔）。每個 \\section 會成為一個可編輯的章節。匯入後會列出可以整理的檔案與可加入文獻庫的參考文獻，由你決定要不要做。", anchor);
  const name = el("input", "field");
  name.placeholder = "論文名稱（可留空）";
  const kv = el("input", "field");
  kv.placeholder = "Key value（必填）：通篇要強調的一件事";
  name.kv = kv;
  const need = () => { if (kv.value.trim()) return true; flash("Key value 必填"); kv.focus(); return false; };
  const fromPc = el("div", "form-label", "從這台電腦");
  const row = el("div", "form-row");
  const file = el("button", "ghost wide", "壓縮檔或 .tex…");
  file.prepend(icon("upload"));
  file.title = ".zip、.tar.gz 或單一 .tex";
  file.onclick = () => { if (need()) pickProject(name); };
  const folder = el("button", "ghost wide", "資料夾…");
  folder.prepend(icon("folder"));
  folder.title = "含 main.tex 的論文資料夾（.git 等隱藏檔不會帶入）";
  folder.onclick = () => { if (need()) pickFolder(name); };
  row.append(file, folder);
  const fromArxiv = el("div", "form-label", "從 arXiv");
  const arow = el("div", "form-row");
  const arxiv = el("input", "field");
  arxiv.placeholder = "編號或網址，例如 2401.01234";
  const go = el("button", "primary small", "下載並匯入");
  const run = () => { if (need()) importArxiv(arxiv, name); };
  go.onclick = run;
  arxiv.onkeydown = (e) => { if (e.key === "Enter") run(); if (e.key === "Escape") m.hidden = true; };
  arow.append(arxiv, go);
  m.append(name, kv, fromPc, row, fromArxiv, arow);
  keepOnScreen(m);
  name.focus();
}

// A file already picked (start screen, 打開檔案): name and Key value, then import.
function importFileForm(file, anchor) {
  const m = menuForm("匯入論文", null, anchor);
  const name = el("input", "field");
  name.value = importName(file.name);
  name.placeholder = "論文名稱";
  const kv = el("input", "field");
  kv.placeholder = "Key value（必填）：通篇要強調的一件事";
  name.kv = kv;
  const go = el("button", "primary small", "匯入");
  const run = () => {
    if (!kv.value.trim()) { flash("Key value 必填"); kv.focus(); return; }
    uploadProject(name, name.value.trim() || importName(file.name), file);
  };
  go.onclick = run;
  kv.onkeydown = (e) => { if (e.key === "Enter") run(); };
  m.append(el("div", "form-label", file.name), name, kv, go);
  keepOnScreen(m);
  kv.focus();
}

async function importArxiv(input, nameInput) {
  const id = input.value.trim();
  if (!id) { input.focus(); return; }
  input.disabled = true;
  flash("從 arXiv 下載中…", 60000);
  try {
    const r = await api("/api/papers/import-arxiv", { id, name: nameInput.value.trim() || null, key_value: nameInput.kv.value });
    await opened(r.name, r);
  } catch (e) {
    flash(e.message, 6000);
    input.disabled = false;
    input.focus();
  }
}

const importName = (raw) => raw.replace(/\.(tar\.gz|tgz|zip|gz|tex)$/i, "").replace(/[^\p{L}\p{N} ._-]+/gu, "-").replace(/^[.-]+/, "") || "imported";

// Import a LaTeX project from a file: .zip, .tar.gz or .tex. The name box
// names it; empty takes the file's name.
function pickProject(input) {
  const f = el("input");
  f.type = "file";
  f.accept = ".zip,.tar.gz,.tgz,.gz,.tex";
  f.onchange = () => {
    const file = f.files[0];
    if (file) uploadProject(input, input.value.trim() || importName(file.name), file);
  };
  f.click();
}

// A whole folder: packed into a tar here (the server unpacks .tar like .tar.gz).
function pickFolder(input) {
  const f = el("input");
  f.type = "file";
  f.webkitdirectory = true;
  f.multiple = true;
  f.onchange = async () => {
    const files = [...f.files].filter((x) => !(x.webkitRelativePath || x.name).split("/").some((seg) => seg.startsWith(".")));
    if (!files.length) return;
    if (!files.some((x) => /\.tex$/i.test(x.name))) { flash("這個資料夾裡沒有 .tex 檔", 5000); return; }
    const top = (files[0].webkitRelativePath || "").split("/")[0];
    try {
      const tar = await packTar(files);
      uploadProject(input, input.value.trim() || importName(top), tar);
    } catch (e) { flash(e.message, 6000); }
  };
  f.click();
}

/** A ustar archive of `files` (paths from webkitRelativePath). */
async function packTar(files) {
  const enc = new TextEncoder();
  const parts = [];
  const field = (h, off, len, str) => { const b = enc.encode(str); if (b.length > len) throw new Error(`路徑太長：${str}`); h.set(b, off); };
  const octal = (n, len) => n.toString(8).padStart(len - 1, "0") + "\0";
  for (const f of files) {
    const path = f.webkitRelativePath || f.name;
    const bytes = enc.encode(path);
    let name = path, prefix = "";
    if (bytes.length > 100) {
      // split at a "/" so that the name part fits in 100 bytes
      const cut = [...path].map((c, i) => (c === "/" ? i : -1)).filter((i) => i > 0 && enc.encode(path.slice(i + 1)).length <= 100 && enc.encode(path.slice(0, i)).length <= 155)[0];
      if (cut == null) throw new Error(`路徑太長：${path}`);
      prefix = path.slice(0, cut); name = path.slice(cut + 1);
    }
    const data = new Uint8Array(await f.arrayBuffer());
    const h = new Uint8Array(512);
    field(h, 0, 100, name);
    field(h, 100, 8, "0000644\0");
    field(h, 108, 8, "0000000\0");
    field(h, 116, 8, "0000000\0");
    field(h, 124, 12, octal(data.length, 12));
    field(h, 136, 12, octal(Math.floor((f.lastModified || Date.now()) / 1000), 12));
    h.fill(32, 148, 156); // checksum counted as spaces
    h[156] = 48; // "0": regular file
    field(h, 257, 6, "ustar\0");
    field(h, 263, 2, "00");
    field(h, 345, 155, prefix);
    const sum = h.reduce((a, b) => a + b, 0);
    field(h, 148, 8, octal(sum, 7) + " ");
    parts.push(h, data, new Uint8Array((512 - (data.length % 512)) % 512));
  }
  parts.push(new Uint8Array(1024));
  return new Blob(parts, { type: "application/x-tar" });
}

async function uploadProject(input, name, body) {
  input.disabled = true;
  flash("匯入中…", 60000);
  try {
    const r = await fetch(`/api/papers/import?name=${encodeURIComponent(name)}&key_value=${encodeURIComponent(input.kv.value)}`, { method: "POST", body });
    if (!r.ok) throw new Error(await r.text());
    await opened(name, await r.json().catch(() => null));
  } catch (e) {
    flash(e.message, 6000);
  } finally {
    input.disabled = false;
  }
}

async function opened(name, result) {
  state.workspace = await api("/api/workspace");
  $("#fileMenu").hidden = true;
  await openPaper(result?.name || name);
  refreshGit();
  flash(`已匯入 ${result?.name || name}`);
  if (result) importCard(result.name || name, result, true);
}

// After an import (or from the paper menu): which journal template the paper
// matched, what tidying would change, and how many references could go into
// the library. Nothing happens until a button says so.
async function importCard(name, info, fresh = false) {
  if (!info) {
    try { info = await api(`/api/papers/${encodeURIComponent(name)}/import-summary`); }
    catch (e) { flash(e.message, 6000); return; }
  }
  const m = menuForm(fresh ? "匯入完成：下一步" : "整理與文獻", "整理只動這篇論文的資料夾：圖檔改成沒有空格的名字（\\includegraphics 跟著改）、移除沒載入的 .cls，並讓 Git 略過編譯產生的 *-eps-converted-to.pdf。加入文獻庫會把論文自己的 .bib 條目複製到 refs/library.bib，已有的（同 key 或同 DOI）會略過。");
  m.classList.add("next-card");
  const line = (label, ...content) => {
    const d = el("div", "nc-row");
    d.append(el("span", "nc-label", label), ...content);
    m.append(d);
    return d;
  };
  // journal
  if (info.journal) line("期刊", el("span", "", info.journal_name || info.journal), el("span", "nc-sub", "由 .cls 比對出的模板"));
  else line("期刊", el("span", "nc-sub", "沒有對到 publishers/ 裡的模板；論文自帶的 class 照常使用"));
  if (fresh && info.authors) line("作者", info.authors.length ? el("span", "", info.authors.join("、")) : el("span", "nc-sub", "\\author 裡沒有作者資料庫的人"));
  // tidy
  const plan = info.tidy;
  const tidyBox = line("整理");
  if (info.tidied) tidyBox.append(el("span", "nc-ok", "已整理"));
  else if (!plan || (!plan.renames?.length && !plan.remove?.length && !plan.gitignore?.length)) tidyBox.append(el("span", "nc-sub", "不需要整理"));
  else {
    const ul = el("ul", "nc-list");
    for (const [from, to] of plan.renames ?? []) { const li = el("li"); li.append(el("code", "", from), " → ", el("code", "", to)); ul.append(li); }
    for (const f of plan.remove ?? []) { const li = el("li"); li.append("刪除沒用到的 ", el("code", "", f)); ul.append(li); }
    for (const g of plan.gitignore ?? []) { const li = el("li"); li.append(".gitignore 加入 ", el("code", "", g), "（編譯時產生的檔案，保留但不同步）"); ul.append(li); }
    const go = el("button", "primary small", "整理");
    go.onclick = async () => {
      go.disabled = true;
      try {
        const done = await api(`/api/papers/${encodeURIComponent(name)}/tidy`, {});
        const n = (done.renames?.length ?? 0) + (done.remove?.length ?? 0) + (done.gitignore?.length ?? 0);
        ul.replaceWith(el("span", "nc-ok", `已整理 ${n} 項`));
        go.remove();
        if (done.renames?.length) await openPaper(name);
        refreshGit();
      } catch (e) { flash(e.message, 6000); go.disabled = false; }
    };
    tidyBox.append(ul, go);
  }
  // bibliography
  const bib = info.bib;
  const bibBox = line("文獻");
  if (!bib || !bib.files?.length) bibBox.append(el("span", "nc-sub", "論文沒有自己的 .bib"));
  else {
    const n = bib.added?.length ?? 0, k = bib.skipped?.length ?? 0;
    const txt = el("span", "", n ? `${bib.files.join("、")}：${n} 筆可加入文獻庫` : `${bib.files.join("、")} 的條目都已在文獻庫`);
    bibBox.append(txt);
    if (k) bibBox.append(el("span", "nc-sub", `${k} 筆已在庫中，會略過`));
    if (bib.errors?.length) bibBox.append(el("span", "nc-sub", `${bib.errors.length} 筆讀不懂：${bib.errors.slice(0, 2).join("；")}`));
    if (n) {
      const go = el("button", "primary small", "加入文獻庫");
      go.onclick = async () => {
        go.disabled = true;
        try {
          const done = await api(`/api/papers/${encodeURIComponent(name)}/bib/merge`, {});
          txt.replaceWith(el("span", "nc-ok", `已加入 ${done.added?.length ?? 0} 筆`));
          go.remove();
          refreshGit();
        } catch (e) { flash(e.message, 6000); go.disabled = false; }
      };
      bibBox.append(go);
    }
  }
  const foot = el("div", "form-foot");
  const ok = el("button", "ghost small", "完成");
  ok.onclick = () => { m.hidden = true; };
  foot.append(ok);
  m.append(foot);
  m.hidden = false;
  keepOnScreen(m);
}

/** A popup menu moved left as far as needed to stay inside a narrow window. */
function keepOnScreen(m) {
  const w = m.offsetWidth;
  const left = parseFloat(m.style.left) || 0;
  if (left + w > innerWidth - 8) m.style.left = `${Math.max(8, innerWidth - w - 8)}px`;
}

// templates

/** Open the Console key page in the system browser, then wait in the key field. */
async function openKeysPage(input) {
  try { await api("/api/open/anthropic-keys", {}); flash("建立 key 後貼回這裡"); }
  catch (e) { flash(e.message, 6000); }
  $(input).focus();
}

// ── Database editor ────────────────────────────────────
async function openDb() {
  const tabs = $("#dbTabs");
  if (!tabs.children.length) {
    TABLES.forEach(([name, label]) => {
      const b = el("button");
      b.setAttribute("role", "tab");
      b.dataset.name = name;
      b.append(label, el("span", "c", ""));
      b.onclick = () => showTable(name);
      tabs.append(b);
    });
    // the reference library shared by every paper
    const lib = el("button");
    lib.setAttribute("role", "tab");
    lib.dataset.name = "@refs";
    lib.append("文獻庫", el("span", "c", ""));
    lib.onclick = () => showTable("@refs");
    tabs.append(lib);
    const ds = $("#dbSection");
    ROLES.forEach((r) => ds.append(new Option(r, r)));
    const dc = $("#dcSection");
    ROLES.forEach((r) => dc.append(new Option(r, r)));
  }
  await Promise.all(TABLES.map(async ([name]) => {
    if (!state.db.dirty.has(name)) {
      try { state.db.tables[name] = await api(`/api/tables/${name}`); } catch { state.db.tables[name] = { headers: [], rows: [] }; }
    }
  }));
  $("#dcSection").value = roleOf(curSection()?.title ?? "Any");
  showTable(state.db.name);
}

const COL_CLASS = { id: "id", priority: "short", type: "short", kind: "short", position: "short", severity: "short", threshold: "short" };

function showTable(name) {
  state.db.name = name;
  document.querySelectorAll("#dbTabs button").forEach((b) => {
    b.setAttribute("aria-selected", String(b.dataset.name === name));
    if (b.dataset.name !== "@refs") b.querySelector(".c").textContent = state.db.tables[b.dataset.name]?.rows.length ?? "";
  });
  const refs = name === "@refs";
  $("#dlgDb .db").hidden = refs;
  $("#refsPane").hidden = !refs;
  if (refs) { PCRefs.renderPage(); return; }
  const t = state.db.tables[name];
  const grid = $("#dbGrid");
  grid.innerHTML = "";
  const thead = el("thead"); const hr = el("tr");
  t.headers.forEach((h) => hr.append(el("th", "", h)));
  hr.append(el("th", "", ""));
  thead.append(hr);
  const tbody = el("tbody");
  t.rows.forEach((row, ri) => tbody.append(rowEl(t, row, ri)));
  grid.append(thead, tbody);
  $("#dbSection").hidden = !t.headers.includes("section");
  $("#dbDirty").hidden = !state.db.dirty.has(name);
  const iss = state.db.issues[name] ?? [];
  $("#dbIssues").textContent = iss.map((i) => `第 ${i.line} 行 ${i.column ?? ""}：${i.message}`).join("　");
  applyFilter();
}

function rowEl(t, row, ri) {
  const tr = el("tr");
  if (row._new) tr.className = "new";
  t.headers.forEach((h, ci) => {
    const td = el("td", COL_CLASS[h] ?? "");
    const ta = el("textarea", "cell");
    ta.rows = 1;
    ta.value = row[ci] ?? "";
    ta.oninput = () => { row[ci] = ta.value; markDirty(); };
    td.append(ta);
    tr.append(td);
  });
  const td = el("td");
  const del = iconButton("trash", "刪除這一列", () => { t.rows.splice(ri, 1); markDirty(); showTable(state.db.name); });
  del.classList.add("del");
  td.append(del);
  tr.append(td);
  return tr;
}

function markDirty(name = state.db.name) {
  state.db.dirty.add(name);
  if (name === state.db.name) $("#dbDirty").hidden = false;
}

function applyFilter() {
  const q = $("#dbFilter").value.trim().toLowerCase();
  const sec = $("#dbSection").value;
  const t = state.db.tables[state.db.name];
  const si = t.headers.indexOf("section");
  document.querySelectorAll("#dbGrid tbody tr").forEach((tr, i) => {
    const row = t.rows[i];
    const okQ = !q || row.some((c) => String(c).toLowerCase().includes(q));
    const okS = !sec || si < 0 || row[si] === sec || row[si] === "Any";
    tr.hidden = !(okQ && okS);
  });
}

function nextId(t, prefix) {
  const ii = t.headers.indexOf("id");
  const max = Math.max(0, ...t.rows.map((r) => parseInt(String(r[ii] ?? "").replace(prefix, ""), 10)).filter((n) => !isNaN(n)));
  return `${prefix}${String(max + 1).padStart(3, "0")}`;
}
const PREFIX = { "phrases.csv": "P", "expressions.csv": "X", "rules.csv": "R", "exemplars.csv": "E", "equations.csv": "Q", "authors.csv": "A", "affiliations.csv": "F" };

function addRow(name, values = {}) {
  const t = state.db.tables[name];
  const row = t.headers.map(() => "");
  if (t.headers.includes("id") && PREFIX[name]) row[t.headers.indexOf("id")] = nextId(t, PREFIX[name]);
  Object.entries(values).forEach(([k, v]) => { const i = t.headers.indexOf(k); if (i >= 0) row[i] = v; });
  if (name === "phrases.csv" && !values.priority) row[t.headers.indexOf("priority")] = "3";
  row._new = true;
  t.rows.push(row);
  markDirty(name);
  return row;
}

async function saveDb() {
  const names = [...state.db.dirty];
  for (const name of names) {
    const t = state.db.tables[name];
    try {
      state.db.issues[name] = await api(`/api/tables/${name}`, { headers: t.headers, rows: t.rows.map((r) => [...r]) }, "PUT");
      state.db.dirty.delete(name);
      t.rows.forEach((r) => delete r._new);
    } catch (e) { flash(`${name} 儲存失敗：${e.message}`); }
  }
  try { state.profile = await api("/api/profile"); } catch {}
  showTable(state.db.name);
  const n = Object.values(state.db.issues).flat().length;
  flash(n ? `已儲存，有 ${n} 筆格式問題` : "已儲存");
  refreshGit();
}

// decompose
async function decompose() {
  const text = $("#dcText").value.trim();
  if (!text) { $("#dcText").focus(); return; }
  const out = $("#dcOut");
  const btn = $("#dcRun");
  btn.disabled = true; btn.textContent = "拆解中";
  out.innerHTML = "";
  try {
    const p = await api("/api/decompose", { text, section: $("#dcSection").value, engine: chosenEngine() });
    renderProposals(p);
  } catch (e) {
    out.append(el("p", "note", e.message));
  } finally { btn.disabled = false; btn.textContent = "拆解"; }
}

function renderProposals(p) {
  const out = $("#dcOut");
  out.innerHTML = "";
  const groups = [
    ["phrases.csv", "句型", p.phrases, (x) => [`${x.section} · ${x.function} · ${x.position}`, x.template, x.example]],
    ["expressions.csv", "片語", p.expressions, (x) => [`${x.kind}${x.section ? " · " + x.section : ""}`, x.text, x.usage]],
  ];
  const picks = [];
  groups.forEach(([name, label, items, show]) => {
    if (!items.length) return;
    const head = el("div", "prop-head"); head.append(label, el("span", "c", String(items.length)));
    const ul = el("ul", "prop");
    items.forEach((x) => {
      const li = el("li");
      const cb = el("input"); cb.type = "checkbox"; cb.checked = true;
      const [meta, main, sub] = show(x);
      const body = el("div");
      const t = el("div", "t"); t.append(name === "phrases.csv" ? slotted(main) : main);
      body.append(el("div", "meta", meta), t);
      if (sub) body.append(el("div", "u", sub));
      li.append(cb, body);
      ul.append(li);
      picks.push([cb, name, x]);
    });
    out.append(head, ul);
  });
  const add = el("button", "primary small", "加入勾選項目");
  add.style.marginTop = "8px";
  add.onclick = () => {
    let n = 0;
    picks.filter(([cb]) => cb.checked).forEach(([, name, x]) => {
      if (name === "phrases.csv") addRow(name, { section: x.section, function: x.function, position: x.position, template: x.template, example: x.example, tags: (x.tags ?? []).join(";") });
      else addRow(name, { kind: x.kind, text: x.text, usage: x.usage, section: x.section, example: x.example });
      n++;
    });
    out.innerHTML = "";
    out.append(el("p", "note", `已加入 ${n} 筆，記得儲存`));
    showTable(state.db.name);
  };
  out.append(add);
}

// ── Wiring ─────────────────────────────────────────────
function wire() {
  document.querySelectorAll(".block .toggle").forEach((t) => {
    t.onclick = () => {
      const b = t.closest(".block");
      const open = b.dataset.open !== "true";
      b.dataset.open = String(open);
      t.textContent = open ? "−" : "+";
    };
  });

  let tip;
  const closeTip = () => { tip?.remove(); tip = null; document.querySelectorAll(".help").forEach((h) => h.setAttribute("aria-expanded", "false")); };
  // "?" tips, also for ones added later (menus, forms)
  const toggleTip = (h) => {
    const was = h.getAttribute("aria-expanded") === "true";
    closeTip();
    if (was) return;
    tip = el("div", "tip", h.dataset.tip);
    (h.closest("dialog") ?? document.body).append(tip);
    const r = h.getBoundingClientRect();
    tip.style.top = `${r.bottom + 6}px`;
    tip.style.left = `${Math.max(8, Math.min(r.left - 100, innerWidth - 260))}px`;
    h.setAttribute("aria-expanded", "true");
  };
  document.addEventListener("click", (e) => {
    const help = e.target.closest?.(".help");
    if (help) { toggleTip(help); return; }
    closeTip();
    // a control that replaced itself (a menu turning into a form) is not "outside"
    if (!e.target.isConnected) return;
    if (!e.target.closest("#fileMenu, #actFile, [data-opens-menu]")) $("#fileMenu").hidden = true;
    if (!e.target.closest("#modelMenu, #cEngine")) $("#modelMenu").hidden = true;
    if (!e.target.closest("#exportPop, #actExport, #hoverMenu") && !PCExport.busy) PCExport.close();
  });
  document.addEventListener("keydown", (e) => {
    // Ctrl+S: save database edits, then commit and push the workspace.
    if ((e.ctrlKey || e.metaKey) && e.key === "s") { e.preventDefault(); syncNow(); return; }
    // Ctrl+P: the submission PDF instead of printing the page.
    if ((e.ctrlKey || e.metaKey) && e.key === "p") { e.preventDefault(); if (!document.querySelector("dialog[open]")) PCExport.run("pdf"); return; }
    // Esc closes tips, menus and popovers; open windows close themselves.
    if (e.key !== "Escape" || document.querySelector("dialog[open]")) return;
    closeTip(); $("#fileMenu").hidden = true; $("#modelMenu").hidden = true; closeHoverMenu(true); PCExport.close();
  });

  PCZoom.init();
  PCNew.wire();
  wireHoverMenus();
  wireWindows();
  $("#actFile").onclick = () => PCHome.toggle();
  $("#actRefs").onclick = () => openWindow("dlgPaperRefs");
  $("#actDraft").onclick = () => PCDraft.open();
  $("#actExport").onclick = () => PCExport.run("pdf");
  $("#actDb").onclick = () => openDbTab(state.db.name);
  $("#actTpl").onclick = () => openWindow("dlgTpl");
  $("#actSync").onclick = () => (state.git?.repo ? syncNow() : Setup.open("workspace"));
  $("#actSettings").onclick = () => Setup.open();

  document.querySelectorAll(".theme[data-theme]").forEach((b) => { b.onclick = () => { store.set("pc.theme", b.dataset.theme); applyTheme(); }; });

  document.querySelectorAll(".seg").forEach((s) => {
    s.querySelectorAll("button").forEach((b) => {
      b.onclick = () => s.querySelectorAll("button").forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
    });
  });
  document.querySelectorAll("#lookMode button").forEach((b) => b.addEventListener("click", () => { store.set("pc.mode", b.dataset.v); applyTheme(); }));
  document.querySelectorAll("#lookPaper button").forEach((b) => b.addEventListener("click", () => { store.set("pc.dim", b.dataset.v === "dim"); applyTheme(); }));
  $("#generate").onclick = compose;
  $("#points").onkeydown = (e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) compose(); };

  $("#dbFilter").oninput = applyFilter;
  $("#dbSection").onchange = applyFilter;
  $("#dbAdd").onclick = () => {
    const sec = $("#dbSection").value;
    addRow(state.db.name, sec ? { section: sec } : {});
    showTable(state.db.name);
    const rows = document.querySelectorAll("#dbGrid tbody tr");
    rows[rows.length - 1]?.querySelector(".cell:not([value])")?.focus();
  };
  $("#dbSave").onclick = saveDb;
  $("#dcRun").onclick = decompose;
}

async function boot() {
  // formulas and figures are drawn near the view: the page (held in place
  // while they are drawn above it) and the navigator's open items
  PCLazy.attach($("#paper"), $("#pageArea"), (c) => c.children, true);
  PCLazy.attach($("#nav"), $("aside.left"), (c) => c.querySelectorAll("button"), false);
  $("#navOpenAll").onclick = () => setAllNav(true);
  $("#navCloseAll").onclick = () => setAllNav(false);
  PCWrite.build();
  paintIcons();
  applyTheme();
  PCHome.wire(); // before wire(): Esc on the start screen sees the menu still open
  wire();
  try { state.workspace = await api("/api/workspace"); } catch {}
  // Bring in work from other devices before showing anything.
  try {
    const r = await api("/api/git/pull", {});
    if (r?.pulled) flash("已從遠端更新");
  } catch (e) { flash(e.message, 8000); }
  await store.load();
  applyTheme();
  PCZoom.restore();
  await refreshGit();
  try {
    state.profile = await api("/api/profile");
    state.ruleText = await api("/api/profile/rules");
    // The writing database's name sits in its window; a demo workspace
    // (none of the user's own connected yet) marks the gear.
    const pn = $("#profileName");
    pn.textContent = state.profile.error ? "資料庫讀取失敗" : state.profile.demo ? "示範資料" : state.profile.name;
    pn.title = state.profile.error ?? state.profile.path;
    pn.classList.toggle("warn", !!state.profile.error || state.profile.demo || state.profile.issues.length > 0);
    if (state.profile.issues.length) pn.title += "\n" + state.profile.issues.map((x) => `${x.file}:${x.line} ${x.column ?? ""} ${x.message}`).join("\n");
    markSettings();
    state.profile.functions.forEach((f) => $("#cFunction").append(new Option(f.replace(/_/g, " "), f)));
  } catch {
    flash("伺服器未啟動", 8000);
  }
  // the app opens on the start screen; a paper opens from there
  render();
  PCHome.show(1);
  await loadEngines();
  // setup.js loads after this file; by now it is defined.
  Setup.wire();
  await Setup.maybeOpen();
  // A local model may still be starting in the background.
  setTimeout(loadEngines, 15000);
}

boot();
