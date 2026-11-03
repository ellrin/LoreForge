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

async function openPaper(name) {
  try {
    const res = await api(`/api/papers/${encodeURIComponent(name)}`);
    state.paper = { name, sections: res.sections, front: res.front ?? "", back: res.back ?? "", fields: res.fields ?? [], bibliography: res.bibliography ?? [], meta: res.meta ?? { key_value: "", authors: [] } };
    PCTex.setMacros(res.macros ?? {});
    PCTex.setNumbers(res.numbers);
    PCTex.setFigureBase(`/api/papers/${encodeURIComponent(name)}/figure`);
    state.cur = { s: 0, p: 0 };
    store.set("pc.paper", name);
  } catch (e) {
    state.paper = null;
  }
  $("#paperName").textContent = state.paper?.name ?? "—";
  await checkAll();
  render();
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

function render() {
  PCTex.number(sections(), state.paper?.front ?? "");
  renderNav(); renderKeyValue(); renderPaper(); renderSide(); renderStatus();
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

function renderNav() {
  $("#navTitle").textContent = state.paper?.name ?? "";
  const nav = $("#nav");
  nav.innerHTML = "";
  if (!state.paper) { nav.append(el("li", "empty", "從「檔案」開啟論文")); return; }
  const open = store.get("pc.navOpen", {});
  if (state.paper.front?.trim()) {
    const li = el("li", "sec front");
    const head = el("button");
    head.append(el("span", "caret", ""), el("span", "", "標題與摘要"));
    head.onclick = () => document.getElementById("front")?.scrollIntoView({ block: "start", behavior: "smooth" });
    li.append(head);
    nav.append(li);
  }
  sections().forEach((sec, si) => {
    const li = el("li", "sec");
    li.dataset.open = String(open[sec.file] !== false);
    const head = el("button");
    head.append(el("span", "caret", "▾"), el("span", "", sec.title), el("span", "count", String(sec.paragraphs.filter((t) => PCTex.stripComments(t).trim()).length)));
    head.onclick = () => {
      const now = li.dataset.open !== "true";
      li.dataset.open = String(now);
      open[sec.file] = now;
      store.set("pc.navOpen", open);
      document.getElementById(`sec-${si}`)?.scrollIntoView({ block: "start", behavior: "smooth" });
    };
    const ol = el("ol", "outline");
    sec.paragraphs.forEach((text, pi) => {
      // commented-out paragraphs stay on the page (faint %), not in the outline
      if (!PCTex.stripComments(text).trim()) return;
      const b = el("button");
      b.setAttribute("aria-current", String(si === state.cur.s && pi === state.cur.p));
      const rep = state.reports[si]?.[pi];
      // Label: rendered first line; display equations show their number.
      const t = el("span", "t");
      const kind = navKind(text);
      if (kind) t.textContent = kind;
      else t.append(PCTex.render(PCTex.stripComments(text).trim().split("\n")[0]));
      b.append(el("span", `status ${rep ? rep.status ?? "" : ""}`), el("span", "i", pad(pi + 1)), t);
      b.onclick = () => { select(si, pi); paraEl(si, pi)?.scrollIntoView({ block: "center", behavior: "smooth" }); };
      const item = el("li");
      item.append(b);
      ol.append(item);
    });
    li.append(head, ol);
    nav.append(li);
  });
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

const paraEl = (si, pi) => document.querySelector(`.para[data-s="${si}"][data-p="${pi}"]`);

function renderPaper() {
  const paper = $("#paper");
  paper.innerHTML = "";
  if (!state.paper) return;
  PCTex.resetCounters();
  // main.tex around the sections: title block first, back matter and references last
  if (state.paper.front.trim()) {
    const front = PCTex.frontNode(state.paper.front);
    front.id = "front";
    front.querySelectorAll("[data-field]").forEach((n) => (n.ondblclick = (e) => { e.stopPropagation(); editField(n); }));
    paper.append(front);
  }
  sections().forEach((sec, si) => {
    const h = el("h2");
    h.append(PCTex.render(sec.title));
    h.id = `sec-${si}`;
    paper.append(h);
    sec.paragraphs.forEach((text, pi) => {
      if (PCTable.isTable(text)) {
        const p = el("div", "para");
        p.dataset.s = si;
        p.dataset.p = pi;
        paper.append(p);
        PCTable.mount(p, si, pi, text);
        return;
      }
      if (!PCTex.stripComments(text).trim()) {
        // only comments: keep a handle to edit it, nothing to read
        const p = el("div", "para comment-only", "%");
        p.title = "註解";
        p.dataset.s = si;
        p.dataset.p = pi;
        p.onclick = () => select(si, pi);
        p.ondblclick = () => edit(p, si, pi);
        paper.append(p);
        return;
      }
      const head = PCTex.heading(text);
      const block = PCTex.isDisplay(text);
      const whole = block || PCTex.needsWhole(text);
      const p = el(head ? "div" : block ? "div" : "p", head ? `para head ${head[0]}` : block ? "para display" : "para");
      p.dataset.s = si;
      p.dataset.p = pi;
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
      p.onclick = () => select(si, pi);
      p.ondblclick = () => edit(p, si, pi);
      paper.append(p);
    });
    if (!sec.paragraphs.length) {
      const p = el("p", "para add-para", "雙擊新增段落");
      p.ondblclick = () => { sec.paragraphs.push(""); render(); edit(paraEl(si, 0), si, 0); };
      paper.append(p);
    }
  });
  const back = state.paper.back.trim();
  if (back) { const d = el("div", "tex-back"); d.append(PCTex.render(back)); if (d.textContent.trim()) paper.append(d); }
  if (state.paper.bibliography.length) paper.append(PCTex.bibliographyNode(state.paper.bibliography));
  markCurrent();
}

function select(si, pi) {
  state.cur = { s: si, p: pi };
  markCurrent();
  renderSide();
}

function markCurrent() {
  document.querySelectorAll(".para").forEach((p) =>
    p.classList.toggle("current", +p.dataset.s === state.cur.s && +p.dataset.p === state.cur.p));
  document.querySelectorAll(".nav .outline button").forEach((b) => b.setAttribute("aria-current", "false"));
  const navSec = document.querySelectorAll(".nav .sec")[state.cur.s];
  navSec?.querySelectorAll(".outline button")[state.cur.p]?.setAttribute("aria-current", "true");
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
    render();
    refreshGit();
  };
  p.replaceChildren(ta);
  p.ondblclick = null;
  fit();
  ta.focus();
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
  s.scrollIntoView({ block: "center", behavior: "smooth" });
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

/** Claude menu: the model, and how Claude connects. */
function toggleModelMenu(btn) {
  const m = $("#modelMenu");
  if (!m.hidden) { m.hidden = true; return; }
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
  m.hidden = false;
  const r = btn.getBoundingClientRect();
  m.style.top = `${r.bottom + 4}px`;
  m.style.left = `${Math.max(8, Math.min(r.left, innerWidth - m.offsetWidth - 8))}px`;
}

function renderEngineSwitch() {
  const pick = chosenEngine();
  const e = state.engines;
  $('#cEngine [data-v="claude"] .lbl').textContent = claudeReady() ? `Claude ${modelName(e.model)}` : "Claude";
  document.querySelectorAll("#cEngine button").forEach((b) => {
    const v = b.dataset.v;
    const ready = v === "local" ? !!e?.local.configured : claudeReady();
    b.classList.toggle("off", !ready);
    b.setAttribute("aria-pressed", String(v === "local" ? pick === "local" : pick != null && pick !== "local"));
    b.title = ready ? "" : v === "local" ? "尚未部署本機模型，點此設定" : "尚未連接 Claude，點此設定";
    // Unavailable: take the user to where it gets set up.
    b.onclick = () => {
      if (!ready) { Setup.open(v === "local" ? "model" : "claude"); return; }
      store.set("pc.engine", v);
      renderEngineSwitch();
      renderStatus();
      // Claude: also offer its models and connection.
      if (v === "claude") toggleModelMenu(b); else $("#modelMenu").hidden = true;
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
  render();
  refreshGit();
}

function renderStatus() {
  const words = state.reports.flat().reduce((n, r) => n + (r?.words ?? 0), 0);
  const paras = sections().reduce((n, s) => n + s.paragraphs.length, 0);
  const sec = curSection();
  const bar = $("#statusbar");
  bar.innerHTML = "";
  bar.append(el("span", "accent", sec ? `${pad(state.cur.s + 1)} / ${sec.title}` : "—"), el("span", "", `${paras} 段 · ${words} 字`));
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
    if (r.pulled && state.paper) await openPaper(state.paper.name);
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
    if (d.id !== "dlgNew") d.addEventListener("click", (e) => { if (e.target === d) closeWindow(d); });
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

// Paper menu: the papers to switch to; "新論文" and "匯入" turn the menu
// into a small form in place.
function toggleFileMenu() {
  const m = $("#fileMenu");
  if (!m.hidden) { m.hidden = true; return; }
  fileMenuList();
  const r = $("#actFile").getBoundingClientRect();
  m.style.top = `${r.bottom + 4}px`;
  m.style.left = `${r.left}px`;
  m.hidden = false;
}

function fileMenuList() {
  const m = $("#fileMenu");
  m.className = "menu";
  m.innerHTML = "";
  const papers = state.workspace?.papers ?? [];
  if (!papers.length) m.append(el("div", "empty", "還沒有論文"));
  papers.forEach((name) => {
    const b = el("button", "", name);
    b.prepend(icon("file"));
    b.setAttribute("aria-current", String(name === state.paper?.name));
    b.onclick = () => { m.hidden = true; openPaper(name); };
    m.append(b);
  });
  m.append(el("div", "sep"));
  const add = el("button", "", "新論文");
  add.prepend(icon("plus"));
  add.onclick = () => PCNew.open();
  const imp = el("button", "", "匯入");
  imp.prepend(icon("upload"));
  imp.onclick = importForm;
  m.append(add, imp);
}

// Form header inside the menu: back to the list, a title, an optional "?".
function menuForm(title, tip) {
  const m = $("#fileMenu");
  m.className = "menu form";
  m.innerHTML = "";
  const head = el("div", "form-head");
  head.append(iconButton("back", "返回", fileMenuList), el("b", "", title));
  if (tip) { const h = el("button", "help", "?"); h.dataset.tip = tip; head.append(h); }
  m.append(head);
  return m;
}

function importForm() {
  const m = menuForm("匯入", "現有的 LaTeX 論文：.zip、.tar.gz 或 .tex，或 arXiv 上的論文（下載作者上傳的原始檔）。每個 \\section 會成為一個可編輯的章節。");
  const name = el("input", "field");
  name.placeholder = "論文名稱（可留空）";
  const kv = el("input", "field");
  kv.placeholder = "Key value（必填）：通篇要強調的一件事";
  name.kv = kv;
  const file = el("button", "ghost wide", "選擇檔案…");
  file.prepend(icon("upload"));
  file.onclick = () => { if (!kv.value.trim()) { flash("Key value 必填"); kv.focus(); return; } pickProject(name); };
  const or = el("div", "form-label", "或");
  const arxiv = el("input", "field");
  arxiv.placeholder = "arXiv 編號或網址";
  const go = el("button", "primary small", "匯入");
  const run = () => importArxiv(arxiv, name);
  go.onclick = run;
  arxiv.onkeydown = (e) => { if (e.key === "Enter") run(); if (e.key === "Escape") m.hidden = true; };
  const foot = el("div", "form-foot");
  foot.append(go);
  m.append(name, kv, file, or, arxiv, foot);
  name.focus();
}

async function importArxiv(input, nameInput) {
  const id = input.value.trim();
  if (!id) { input.focus(); return; }
  if (!nameInput.kv.value.trim()) { flash("Key value 必填"); nameInput.kv.focus(); return; }
  input.disabled = true;
  flash("從 arXiv 下載中…", 60000);
  try {
    const r = await api("/api/papers/import-arxiv", { id, name: nameInput.value.trim() || null, key_value: nameInput.kv.value });
    await opened(r.name);
  } catch (e) {
    flash(e.message, 6000);
    input.disabled = false;
    input.focus();
  }
}

// Import a LaTeX project from a file: .zip, .tar.gz or .tex. The name box
// names it; empty takes the file's name.
function pickProject(input) {
  const f = el("input");
  f.type = "file";
  f.accept = ".zip,.tar.gz,.tgz,.gz,.tex";
  f.onchange = async () => {
    const file = f.files[0];
    if (!file) return;
    const name = input.value.trim() || file.name.replace(/\.(tar\.gz|tgz|zip|gz|tex)$/i, "").replace(/[^\p{L}\p{N} ._-]+/gu, "-").replace(/^[.-]+/, "") || "imported";
    input.disabled = true;
    flash("匯入中…", 60000);
    try {
      const r = await fetch(`/api/papers/import?name=${encodeURIComponent(name)}&key_value=${encodeURIComponent(input.kv.value)}`, { method: "POST", body: file });
      if (!r.ok) throw new Error(await r.text());
      await opened(name);
    } catch (e) {
      flash(e.message, 6000);
    } finally {
      input.disabled = false;
    }
  };
  f.click();
}

async function opened(name) {
  state.workspace = await api("/api/workspace");
  $("#fileMenu").hidden = true;
  await openPaper(name);
  refreshGit();
  flash(`已建立 ${name}`);
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
    if (!e.target.closest("#fileMenu, #actFile")) $("#fileMenu").hidden = true;
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
  $("#actFile").onclick = toggleFileMenu;
  $("#actRefs").onclick = () => openWindow("dlgPaperRefs");
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
  PCWrite.build();
  paintIcons();
  applyTheme();
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
  const papers = state.workspace?.papers ?? [];
  const last = store.get("pc.paper", null);
  if (papers.length) await openPaper(papers.includes(last) ? last : papers[0]);
  else render();
  await loadEngines();
  // setup.js loads after this file; by now it is defined.
  Setup.wire();
  await Setup.maybeOpen();
  // A local model may still be starting in the background.
  setTimeout(loadEngines, 15000);
}

boot();
