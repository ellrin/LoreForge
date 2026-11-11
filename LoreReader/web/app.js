// LoreReader — the reader page. No build step: an ES module over the vendored pdf.js.
//
// The view is a list of page refs {sha, page}. Normally that is every page of the open
// paper; page operations (delete, duplicate, reorder, insert) edit a pending list that
// "另存新檔" turns into a new PDF in the library (the original is never modified).
// Annotations, citable sentences, term cards and chat attachments always name the
// page's own PDF (sha) and its page number in that PDF.

import * as pdfjsLib from "./vendor/pdfjs/pdf.min.mjs";

pdfjsLib.GlobalWorkerOptions.workerSrc = new URL("./vendor/pdfjs/pdf.worker.min.mjs", import.meta.url).href;
const FONTS = new URL("./vendor/pdfjs/standard_fonts/", import.meta.url).href;

// ── small helpers ─────────────────────────────────────────────────────────
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
function h(tag, attrs = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs ?? {})) {
    if (v == null || v === false) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else if (k === "dataset") Object.assign(el.dataset, v);
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of kids.flat()) if (c != null && c !== false) el.append(c.nodeType ? c : String(c));
  return el;
}
async function api(path, { method, body, raw } = {}) {
  const opt = { method: method ?? (body !== undefined ? "POST" : "GET"), headers: {} };
  if (body !== undefined) {
    if (raw) opt.body = body;
    else { opt.body = JSON.stringify(body); opt.headers["content-type"] = "application/json"; }
  }
  const r = await fetch(path, opt);
  const v = await r.json().catch(() => ({}));
  if (!r.ok) { const e = new Error(v.error ?? `${r.status}`); e.data = v; e.status = r.status; throw e; }
  return v;
}
/** The shell's toast (commons/ui/shell.js). */
const toast = (msg, err = false) => LoreShell.toast(msg, { err });
const fail = (e) => toast(e.message ?? String(e), true);
const norm = (s) => (s ?? "").replace(/\s+/g, " ").trim();
const clamp = (x, a, b) => Math.max(a, Math.min(b, x));
const STATE_LABEL = { learning: "學習中", known: "記得", stale: "學習中", none: "學習中", new: "沒卡" };
const NOUN_STATE = { known: "記得", learning: "學習中", stale: "學習中", none: "學習中", new: "沒卡" };
/** 要學 = a term without a card (red); 沒卡 = no card but a general word or a name. */
/** A word from the author's own papers (my-vocab) counts as 記得; a KB card wins. */
const isKnown = (n) => n.state === "known" || (n.state === "new" && n.used);
const nounState = (n) => (n.state === "new" ? (n.used ? "記得" : n.red ? "要學" : "沒卡") : NOUN_STATE[n.state] ?? n.state);
const STATE_TIP = { learning: "學習中", stale: "學習中（久未複習）", none: "學習中（還沒有紀錄）", known: "記得" };
const GRADE = { certain: ["✅", "確定"], defined: ["🟢", "有定義"], inferred: ["🟡", "推測"], conflict: ["🔴", "衝突"], unresolved: ["⚪", "查不到"] };
const RESOLVER = { program: "規則", local: "地端", claude: "Claude", author: "作者" };
const GRADE_ORDER = { conflict: 0, inferred: 1, defined: 2, certain: 3, unresolved: 4 };
const ROLE = { background: "背景", gap: "缺口", method: "方法", result: "結果", discussion: "討論", other: "其他" };
const SECTION_KIND = { front: "標題與作者", abstract: "摘要", intro: "引言／相關研究", method: "方法", result: "結果", discussion: "討論", other: "其他" };
const MODEL_LABEL = { auto: "自動", local: "地端", haiku: "Haiku", sonnet: "Sonnet", opus: "Opus" };
const COLOR_LABEL = { yellow: "黃", green: "綠", blue: "藍", pink: "粉紅" };

// ── state ─────────────────────────────────────────────────────────────────
const S = {
  info: null, kb: null,
  sha: null, paper: null,
  docs: new Map(),          // sha → Promise<PDFDocumentProxy>
  texts: new Map(),         // "sha:page" → Promise<{text, items:[{start,end}]}>
  lineNums: new Map(),      // "sha:page" → [[start, end]]: margin line numbers (blanked in the text)
  nouns: null,              // the noun list of the open paper (GET …/nouns)
  nounMarks: new Map(),     // page → [{s, e, cls, i}] from the noun list
  focusNoun: null,          // {i, k}: the noun whose occurrences are lit, and the current one
  nounView: { cat: "", state: "", q: "", sort: "page", dir: 1 },
  readStatus: "",           // what the reading aids are doing ("讀取全文…", errors)
  symbols: null,            // the paper's symbol table (symbols.json)
  symSpans: [],             // unique math runs: {page, s, e, bbox, latex, ents: [entry index]}
  symMarks: new Map(),      // page → [{s, e, k}] (k = index into symSpans)
  symView: { grade: "" },
  paras: null,              // GET …/paragraphs: {paragraphs, status, progress, default_question}
  curPara: null,            // the paragraph the chat is about (id)
  chatCtx: null,            // {paragraph, chip: {kind, id, label}|null} for the next question
  chatScope: "all",         // all | para
  orig: [],                 // [{sha, page}] of the open paper
  arr: null,                // pending edited list, or null
  exp: null,                // GET /api/papers/{sha}/export: dirty, last export, defaults
  pages: [],                // view items: {sha, page, w, h, el, token, rendered, divs, strs}
  zoom: 1, fit: true, cur: 0,
  annotations: [], quotes: [], chat: [], att: [],
  termsOn: false,
  models: null,             // GET /api/models: each feature's engine choice (the engine chips)
  search: { q: "", hits: [], i: -1 },
  sending: false,
  used: {},                 // task → the engine that answered last (engine chip label)
};
try { S.fieldSkipped = localStorage.getItem("lr.fieldSkipped") === "1"; } catch {}
try { S.termsOn = localStorage.getItem("lr.terms") !== "0"; } catch {}
/** The shell (top bar, gear, drawers), mounted in wire(). */
let shell = null;

const key = (sha, page) => `${sha}:${page}`;
const getDoc = (sha) => {
  if (!S.docs.has(sha)) S.docs.set(sha, pdfjsLib.getDocument({ url: `/api/papers/${sha}/pdf`, standardFontDataUrl: FONTS, isEvalSupported: false }).promise);
  return S.docs.get(sha);
};

/** Page text from pdf.js text content: items with `str` in order (one per text-layer
 *  span), a newline after items that end a line. Offsets are JS string (UTF-16) indices. */
function pageText(sha, page) {
  const k = key(sha, page);
  if (!S.texts.has(k)) S.texts.set(k, (async () => {
    const pg = await (await getDoc(sha)).getPage(page);
    const tc = await pg.getTextContent();
    let text = ""; const items = [];
    for (const it of tc.items) {
      if (it.str === undefined) continue;
      const start = text.length;
      text += it.str;
      items.push({ start, end: text.length, raw: it });
      if (it.hasEOL) text += "\n";
    }
    return blankLineNums({ text, items }, S.lineNums.get(k));
  })());
  return S.texts.get(k);
}

/** Margin line numbers ("the" + "275") as spaces, so search, quotes and offsets never
 *  see them (same length: every offset stays valid). */
function blankLineNums(t, ranges) {
  if (!ranges?.length) return t;
  let text = t.text;
  for (const [s, e] of ranges) text = text.slice(0, s) + " ".repeat(e - s) + text.slice(e);
  t.text = text;
  return t;
}

/** The line numbers the server found (`[page, start, end]`): kept per page, blanked in
 *  the page texts already read. */
async function setLineNums(sha, list) {
  const per = new Map();
  for (const [page, s, e] of list ?? []) {
    const k = key(sha, page);
    if (!per.has(k)) per.set(k, []);
    per.get(k).push([s, e]);
  }
  for (const [k, ranges] of per) {
    if (S.lineNums.has(k)) continue;
    S.lineNums.set(k, ranges);
    if (S.texts.has(k)) blankLineNums(await S.texts.get(k), ranges);
  }
}

/** One page for the reading aids: the page text plus every item's PDF font name,
 *  size and position (PDF points, y up). Font names come from the font objects pdf.js
 *  loads for the operator list. */
async function pageRich(sha, page) {
  const { text, items } = await pageText(sha, page);
  const pg = await (await getDoc(sha)).getPage(page);
  await pg.getOperatorList();
  const fonts = {};
  const font = (f) => {
    if (!(f in fonts)) {
      let o = null;
      try { o = pg.commonObjs.has(f) ? pg.commonObjs.get(f) : null; } catch {}
      fonts[f] = { name: o?.name ?? "", italic: !!o?.italic, bold: !!o?.bold };
    }
    return fonts[f];
  };
  const vp = pg.getViewport({ scale: 1 });
  return {
    page, text, width: vp.width, height: vp.height,
    items: items.map(({ start, end, raw }) => {
      const t = raw.transform, f = font(raw.fontName);
      return { s: start, e: end, str: raw.str, font: f.name, italic: f.italic, bold: f.bold, size: Math.hypot(t[2], t[3]) || Math.abs(t[0]), x: t[4], y: t[5], w: raw.width, h: raw.height, eol: !!raw.hasEOL };
    }),
  };
}

// ── library (home) ────────────────────────────────────────────────────────
async function showHome() {
  S.sha = null; S.paper = null; S.exp = null; S.arr = null;
  $("#app").classList.add("at-home");
  $("#home").hidden = false;
  shell.home(true);
  shell.setName("");
  updateStatus();
  closeFind();
  document.title = "LoreReader";
  const [info, list] = await Promise.all([api("/api/info"), api("/api/papers")]);
  S.info = info;
  $("#libError").hidden = !info.library_error;
  $("#libError").textContent = info.library_error ?? "";
  HOME.papers = list.papers;
  renderHome();
}

// 首頁: 01 開始 (tiles) · 02 最近 (first-page thumbnails) · 03 書庫 (filter + sort)
const HOME = { papers: [], sort: "opened", q: "" };
try { HOME.sort = localStorage.getItem("lr.homeSort") || "opened"; } catch {}
const openedAt = () => { try { return JSON.parse(localStorage.getItem("lr.opened") || "{}"); } catch { return {}; } };
const recentOrder = () => { try { return JSON.parse(localStorage.getItem("lr.recent") || "[]"); } catch { return []; } };
function shortDate(ms) {
  const d = new Date(ms), now = new Date(), p = (n) => String(n).padStart(2, "0");
  return d.toDateString() === now.toDateString() ? `${p(d.getHours())}:${p(d.getMinutes())}` : `${d.getFullYear() === now.getFullYear() ? "" : d.getFullYear() + "/"}${p(d.getMonth() + 1)}/${p(d.getDate())}`;
}
/** Author / year when the paper's record carries them (free fields of paper.json). */
function byline(p) {
  const a = Array.isArray(p.authors) ? p.authors.join(", ") : (p.authors || p.author || "");
  return [a, p.year].filter(Boolean).join(" · ");
}
const where = (p) => p.original_path || p.original_name || p.title;
function missingToast() { toast("這份 PDF 不在這台電腦的書庫裡", true); }

function renderHome() {
  const at = openedAt(), order = recentOrder();
  const rank = (p) => { const i = order.indexOf(p.sha256); return i < 0 ? 1e9 : i; };
  // 02 最近: papers opened here, newest first
  const recent = HOME.papers.filter((p) => p.present && order.includes(p.sha256)).sort((x, y) => rank(x) - rank(y)).slice(0, 8);
  $("#recentBlock").hidden = !recent.length;
  $("#recentList").replaceChildren(...recent.map((p) => {
    const frame = h("div", { class: "frame" });
    const card = h("button", { class: "paper-card", title: where(p), onclick: () => go(p.sha256, 1) }, frame,
      h("span", { class: "name" }, p.title), h("span", { class: "when" }, at[p.sha256] ? shortDate(at[p.sha256]) : ""));
    thumbWhenVisible(frame, p.sha256);
    return card;
  }));
  renderLibrary();
}

function renderLibrary() {
  const at = openedAt(), order = recentOrder();
  const rank = (p) => { const i = order.indexOf(p.sha256); return i < 0 ? 1e9 : i; };
  const q = HOME.q.trim().toLowerCase();
  const hay = (p) => [p.title, p.original_name, p.doi, byline(p)].filter(Boolean).join("\n").toLowerCase();
  const cmp = {
    opened: (x, y) => (at[y.sha256] ?? 0) - (at[x.sha256] ?? 0) || rank(x) - rank(y) || String(y.added_at).localeCompare(String(x.added_at)),
    added: (x, y) => String(y.added_at).localeCompare(String(x.added_at)),
    title: (x, y) => x.title.localeCompare(y.title, "zh-Hant"),
  }[HOME.sort];
  const list = HOME.papers.filter((p) => !q || hay(p).includes(q)).sort(cmp);
  $("#libCount").textContent = q ? `${list.length} / ${HOME.papers.length}` : HOME.papers.length || "";
  $$("#libSort button").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.sort === HOME.sort)));
  const rows = list.map((p) => {
    const meta = [byline(p), `${p.pages} 頁`, p.added_at.slice(0, 10), p.doi ? `doi:${p.doi}` : "", p.derived_from ? "編輯版" : "", p.present ? "" : "不在這台電腦"].filter(Boolean).join(" · ");
    return h("div", { class: `lib-row${p.present ? "" : " missing"}` },
      h("button", { class: "lib-open", title: where(p), onclick: () => p.present ? go(p.sha256, 1) : missingToast() },
        h("b", {}, p.title), h("span", { class: "meta" }, meta)),
      p.present ? h("button", { class: "ico small rename", "aria-label": "改標題", "data-icon": "pen", onclick: () => renameDialog(p) }) : null,
      p.present ? h("button", { class: "ico small mine", "aria-label": "我的論文", "aria-pressed": p.mine ? "true" : "false", "data-icon": "star", onclick: (ev) => toggleMine(p, ev.currentTarget) }) : null);
  });
  $("#paperList").replaceChildren(...(rows.length ? rows : [h("p", { class: "empty" }, HOME.papers.length ? "沒有符合的論文" : "書庫是空的")]));
  paintIcons($("#paperList"));
}

/** First-page thumbnail, drawn once when the card scrolls into view (own loader, document released). */
const thumbCache = new Map();
const thumbSeen = new IntersectionObserver((es) => es.forEach((e) => { if (e.isIntersecting) { thumbSeen.unobserve(e.target); drawThumb(e.target).catch(() => {}); } }));
function thumbWhenVisible(frame, sha) { frame.dataset.sha = sha; thumbSeen.observe(frame); }
async function drawThumb(frame) {
  const sha = frame.dataset.sha;
  let url = thumbCache.get(sha);
  if (!url) {
    const task = pdfjsLib.getDocument({ url: `/api/papers/${sha}/pdf`, standardFontDataUrl: FONTS, isEvalSupported: false });
    const doc = await task.promise;
    try {
      const pg = await doc.getPage(1);
      const base = pg.getViewport({ scale: 1 });
      const vp = pg.getViewport({ scale: (180 * Math.min(2, devicePixelRatio || 1)) / base.width });
      const cv = document.createElement("canvas");
      cv.width = Math.ceil(vp.width); cv.height = Math.ceil(vp.height);
      await pg.render({ canvasContext: cv.getContext("2d"), viewport: vp }).promise;
      url = cv.toDataURL("image/jpeg", 0.85);
      thumbCache.set(sha, url);
    } finally { await doc.destroy(); }
  }
  frame.append(h("img", { src: url, alt: "" }));
}

/** 貼上路徑: a small form (the browser has no real file path). */
async function pathDialog() {
  let inp;
  await LoreShell.form({
    title: "貼上路徑",
    build: (body) => { inp = h("input", { class: "np-input", placeholder: "PDF 的完整路徑", "aria-label": "PDF 路徑", autocomplete: "off" }); body.append(inp); },
    actions: [{ label: "加入", primary: true, run: async () => {
      const path = inp.value.trim();
      if (!path) return false;
      try { const r = await api("/api/papers/import", { body: { path } }); go(r.paper.sha256, 1); } catch (x) { fail(x); return false; }
    } }],
  });
}

/** 「我的論文」: its words become the author's own vocabulary (never red). */
/** Rename a paper (the title the library, the quotes and the KB source show). */
async function renameDialog(paper) {
  const inp = h("input", { value: paper.title });
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); close(inp.value.trim()); } },
    h("h3", {}, "標題"), inp,
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"), h("button", { class: "btn primary" }, "儲存"))));
  if (!r || r === paper.title) return;
  try {
    const out = await api(`/api/papers/${paper.sha256}`, { method: "PUT", body: { title: r } });
    paper.title = out.paper.title;
    if (S.sha === paper.sha256) { S.paper = out.paper; shell.setName(out.paper.title); document.title = `${out.paper.title} · LoreReader`; }
    if (!S.sha) showHome();
  } catch (e) { fail(e); }
}

async function toggleMine(p, btn) {
  const on = !p.mine;
  btn.disabled = true;
  try {
    let pages;
    if (on) {
      const doc = await getDoc(p.sha256);
      pages = [];
      for (let n = 1; n <= doc.numPages; n++) { const { text } = await pageText(p.sha256, n); pages.push({ page: n, text }); }
    }
    await api(`/api/papers/${p.sha256}/mine`, { body: { on, pages } });
    p.mine = on;
    btn.setAttribute("aria-pressed", on ? "true" : "false");
    toast(on ? "已標為我的論文" : "已取消我的論文");
  } catch (e) { fail(e); }
  btn.disabled = false;
}

/** Leaving the open paper with notes that are not in a PDF yet: ask once, like the desktop
 *  shell does on close (儲存 / 不儲存 / 取消). `false` = stay. */
async function confirmLeave() {
  if (!S.sha) return true;
  await refreshExport();
  if (!S.exp?.dirty) return true;
  const last = S.exp.last?.exists ? S.exp.last : null;
  const r = await LoreShell.ask(statusAnchor(), ["儲存筆記到新的 PDF？", `《${S.paper.title}》${last ? ` → ${last.name}` : ""}`],
    [{ label: "取消", value: null }, { label: "不儲存", value: "skip" }, { label: last ? "儲存" : "儲存…", value: "save", primary: true }]);
  if (r === "skip") {
    try { await api(`/api/papers/${S.sha}/export/decline`, { body: {} }); } catch (e) { fail(e); }
    return true;
  }
  if (r === "save") return last ? saveNotesLast() : saveNotesAs();
  return false;
}

async function go(sha, page = 1) {
  if (sha !== S.sha && !(await confirmLeave())) return;
  const u = new URL(location.href);
  u.search = sha ? `?paper=${sha}&page=${page}` : "";
  history.pushState(null, "", u);
  route();
}

async function importFiles(files) {
  let last = null;
  for (const f of files) {
    try {
      const r = await api(`/api/papers/import?name=${encodeURIComponent(f.name)}`, { body: await f.arrayBuffer(), raw: true });
      last = r.paper;
      toast(r.restored ? `已還原 ${r.restored} 則註記：${r.paper.title}` : r.new ? `已加入書庫：${r.paper.title}` : `書庫裡已經有：${r.paper.title}`);
    } catch (e) { fail(e); }
  }
  return last;
}

// ── open a paper ──────────────────────────────────────────────────────────
async function openPaper(sha, page) {
  $("#home").hidden = true;
  $("#app").classList.remove("at-home");
  shell.home(false);
  if (S.sha !== sha) {
    const r = await api(`/api/papers/${sha}`);
    S.sha = sha; S.paper = r.paper; S.arr = null;
    shell.setName(r.paper.title);
    document.title = `${r.paper.title} · LoreReader`;
    remember(sha);
    const doc = await getDoc(sha);
    S.orig = Array.from({ length: doc.numPages }, (_, i) => ({ sha, page: i + 1 }));
    S.search = { q: "", hits: [], i: -1 }; $("#searchInput").value = ""; $("#searchCount").textContent = ""; closeFind();
    S.att = []; S.chatDraft = "";
    S.nouns = null; S.nounMarks = new Map(); S.focusNoun = null;
    S.symbols = null; S.symSpans = []; S.symMarks = new Map();
    S.paras = null; S.curPara = null; S.chatCtx = null; S.chatScope = "all";
    await Promise.all([loadAnnotations(), loadQuotes(), loadChat(), refreshExport()]);
    await buildView();
    refreshKb();
    readPaper(sha);
  }
  renderSide();
  scrollToPage(clamp(page - 1, 0, S.pages.length - 1), false);
}

async function loadAnnotations() { S.annotations = (await api(`/api/papers/${S.sha}/annotations`)).annotations; }
async function loadQuotes() { S.quotes = (await api(`/api/papers/${S.sha}/quotes`)).quotes; }
async function loadChat() { S.chat = (await api(`/api/papers/${S.sha}/chat`)).messages; }
async function refreshKb() {
  try { S.kb = await api("/api/kb/status"); } catch { S.kb = { ok: false }; }
  updateLegend();
}

/** KnowledgeBase is not running: a one-line note with 開啟 KnowledgeBase; the page checks
 *  again every few seconds and refreshes the noun list once it answers. */
function kbNote() {
  const btn = h("button", { type: "button", class: "btn ghost small", onclick: async () => {
    btn.disabled = true;
    try { await api("/api/kb/open", { method: "POST" }); } catch (e) { fail(e); }
    watchKb(true);
  } }, "開啟 KnowledgeBase");
  return h("div", { class: "kb-note" }, h("span", {}, "KnowledgeBase 未開啟"), btn);
}
let kbTimer = 0;
function watchKb(soon = false) {
  if (kbTimer) { if (!soon) return; clearTimeout(kbTimer); }
  const tick = async () => {
    kbTimer = 0;
    await refreshKb();
    if (S.kb?.ok) { refreshNouns(); return; }
    kbTimer = setTimeout(tick, document.hidden ? 15000 : 4000);
  };
  kbTimer = setTimeout(tick, soon ? 1500 : 4000);
}

// ── the page list ─────────────────────────────────────────────────────────
const list = () => S.arr ?? S.orig;
let pageObserver, thumbObserver;

async function buildView() {
  const refs = list();
  const items = [];
  for (const r of refs) {
    const pg = await (await getDoc(r.sha)).getPage(r.page);
    const vp = pg.getViewport({ scale: 1 });
    items.push({ sha: r.sha, page: r.page, w: vp.width, h: vp.height, token: 0 });
  }
  S.pages = items;
  $("#editActions").hidden = !S.arr;
  updateStatus();
  $("#pageTotal").textContent = `/ ${items.length}`;
  if (S.fit) S.zoom = fitZoom();
  layoutPages();
  buildThumbs();
}

function fitZoom() {
  const vw = $("#viewport").clientWidth - 40;
  const maxW = Math.max(...S.pages.map((p) => p.w), 1);
  return clamp(vw / maxW, 0.3, 3);
}

function layoutPages() {
  pageObserver?.disconnect();
  pageObserver = new IntersectionObserver((es) => {
    for (const e of es) {
      const p = S.pages[+e.target.dataset.i];
      if (!p) continue;
      if (e.isIntersecting) { if (!p.rendered) renderPage(p); }
      else if (p.rendered) unrenderPage(p);
    }
  }, { root: $("#viewport"), rootMargin: "1200px 0px" });
  const els = S.pages.map((p, i) => {
    p.rendered = false; p.token++;
    p.el = h("div", { class: "page", dataset: { i } }, h("span", { class: "pgno" }, i + 1));
    sizePage(p);
    pageObserver.observe(p.el);
    return p.el;
  });
  $("#pages").replaceChildren(...els);
  $("#zoomChip").textContent = `${Math.round(S.zoom * 100)}%`;
  $("#actFit").setAttribute("aria-pressed", S.fit ? "true" : "false");
}

function sizePage(p) {
  p.el.style.width = `${Math.floor(p.w * S.zoom)}px`;
  p.el.style.height = `${Math.floor(p.h * S.zoom)}px`;
  p.el.style.setProperty("--total-scale-factor", S.zoom);
  p.el.style.setProperty("--scale-factor", S.zoom);
}

function unrenderPage(p) {
  p.token++; p.rendered = false; p.divs = null;
  p.el.querySelectorAll("canvas, .textLayer, .hl-layer").forEach((x) => x.remove());
}

async function renderPage(p) {
  const token = ++p.token;
  p.rendered = true;
  try {
    const pg = await (await getDoc(p.sha)).getPage(p.page);
    if (token !== p.token) return;
    const vp = pg.getViewport({ scale: S.zoom });
    const dpr = Math.min(window.devicePixelRatio || 1, 3);
    const canvas = h("canvas", { class: "pg" });
    canvas.width = Math.floor(vp.width * dpr);
    canvas.height = Math.floor(vp.height * dpr);
    await pg.render({ canvasContext: canvas.getContext("2d"), viewport: pg.getViewport({ scale: S.zoom * dpr }) }).promise;
    if (token !== p.token) return;
    const hl = h("div", { class: "hl-layer" });
    const tl = h("div", { class: "textLayer" });
    p.el.querySelectorAll("canvas, .textLayer, .hl-layer").forEach((x) => x.remove());
    p.el.append(canvas, hl, tl);
    const layer = new pdfjsLib.TextLayer({ textContentSource: await pg.getTextContent(), container: tl, viewport: vp });
    await layer.render();
    if (token !== p.token) return;
    tl.append(h("div", { class: "endOfContent" }));
    p.divs = layer.textDivs;
    p.strs = layer.textContentItemsStr;
    p.divIndex = new Map(p.divs.map((d, i) => [d, i]));
    await refitText(p);
    if (token !== p.token) return;
    drawHighlights(p);
    decorate(p);
  } catch (e) {
    if (token === p.token) { p.rendered = false; console.error(e); }
  }
}

/** pdf.js sizes every text-layer span from a canvas measurement of a fallback font, but
 *  WebKit (the desktop app) lays the same text out several per cent wider, so a mark in
 *  the middle of a long line (a table cell, an abstract) drifts off its glyphs. Measure
 *  the spans as laid out and stretch each one to its PDF width: one read pass, one write
 *  pass. Rotated text keeps pdf.js's own fit. */
async function refitText(p) {
  const { items } = await pageText(p.sha, p.page);
  const fix = [];
  p.divs.forEach((div, i) => {
    const t = items[i]?.raw?.transform;
    if (!t || t[1] !== 0 || t[2] !== 0 || !p.strs[i]?.trim()) return;
    const want = items[i].raw.width * S.zoom;
    const have = div.getBoundingClientRect().width;
    if (want > 0 && have > 0) fix.push([div, want / have]);
  });
  for (const [div, f] of fix) {
    if (Math.abs(f - 1) < 0.002) continue;
    const sx = parseFloat(div.style.getPropertyValue("--scale-x")) || 1;
    div.style.setProperty("--scale-x", sx * f);
  }
}

/** Scroll `box` (and nothing around it: WebKit's scrollIntoView also scrolls the page's
 *  clipped ancestors, which shifts the whole window) so `el` is in view. `top` = space
 *  kept free at the top (a sticky header). */
function scrollWithin(box, el, block = "center", top = 0) {
  if (!box || !el) return;
  const b = box.getBoundingClientRect(), r = el.getBoundingClientRect();
  let y = box.scrollTop, x = box.scrollLeft;
  if (block === "center") y += r.top - b.top - (box.clientHeight - r.height) / 2;
  else if (r.top < b.top + top) y += r.top - b.top - top - 4;
  else if (r.bottom > b.top + box.clientHeight) y += r.bottom - b.top - box.clientHeight + 4;
  if (r.left < b.left || r.right > b.left + box.clientWidth) x += r.left - b.left - (box.clientWidth - r.width) / 2;
  box.scrollTo({ top: Math.max(0, y), left: Math.max(0, x) });
}

function setZoom(z, fit = false) {
  const vpEl = $("#viewport");
  const frac = vpEl.scrollTop / Math.max(1, vpEl.scrollHeight);
  S.fit = fit;
  S.zoom = clamp(z, 0.3, 4);
  layoutPages();
  vpEl.scrollTop = frac * vpEl.scrollHeight;
}

function scrollToPage(i, smooth = true) {
  const p = S.pages[i];
  if (!p) return;
  const vpEl = $("#viewport");
  vpEl.scrollTo({ top: p.el.offsetTop - 12, behavior: smooth ? "smooth" : "auto" });
  setCur(i);
}

function setCur(i) {
  if (i === S.cur && $("#pageInput").value === String(i + 1)) return;
  S.cur = i;
  $("#pageInput").value = i + 1;
  $$(".thumb").forEach((t) => t.setAttribute("aria-current", +t.dataset.i === i ? "true" : "false"));
  const t = $(`.thumb[data-i="${i}"]`);
  if (t && !$("#sorter").matches(":hover")) scrollWithin($("#sorter"), t, "nearest", $(".sorter-head")?.offsetHeight ?? 0);
  if (S.sha && !S.arr) {
    const u = new URL(location.href);
    u.searchParams.set("page", S.pages[i].page);
    history.replaceState(null, "", u);
  }
}

let scrollRaf = 0;
function onScroll() {
  cancelAnimationFrame(scrollRaf);
  scrollRaf = requestAnimationFrame(() => {
    const vpEl = $("#viewport");
    const mid = vpEl.scrollTop + vpEl.clientHeight * 0.35;
    let i = S.pages.findIndex((p) => p.el.offsetTop + p.el.offsetHeight > mid);
    if (i < 0) i = S.pages.length - 1;
    setCur(i);
  });
}

// ── thumbnails ────────────────────────────────────────────────────────────
function buildThumbs() {
  thumbObserver?.disconnect();
  thumbObserver = new IntersectionObserver((es) => {
    for (const e of es) if (e.isIntersecting) { renderThumb(e.target); thumbObserver.unobserve(e.target); }
  }, { root: $("#sorter"), rootMargin: "400px 0px" });
  const own = (r) => r.sha === S.sha;
  const thumbs = S.pages.map((p, i) => {
    const frame = h("div", { class: "frame" });
    frame.style.aspectRatio = `${p.w} / ${p.h}`;
    if (!own(p)) frame.append(h("span", { class: "from" }, `插入 · 原第 ${p.page} 頁`));
    const t = h("div", { class: "thumb", draggable: "true", dataset: { i }, "aria-current": i === S.cur ? "true" : "false", title: own(p) ? `第 ${p.page} 頁` : null },
      h("span", { class: "no" }, i + 1), frame);
    t.addEventListener("click", () => { scrollToPage(i); if (innerWidth <= 1100) shell.drawer("left", false); });
    thumbObserver.observe(t);
    return t;
  });
  $("#thumbs").replaceChildren(...thumbs);
  $("#thumbCount").textContent = `${S.pages.length} 頁`;
}

async function renderThumb(t) {
  const p = S.pages[+t.dataset.i];
  if (!p) return;
  try {
    const pg = await (await getDoc(p.sha)).getPage(p.page);
    const scale = (132 * Math.min(devicePixelRatio || 1, 2)) / p.w;
    const vp = pg.getViewport({ scale });
    const c = h("canvas");
    c.width = Math.floor(vp.width); c.height = Math.floor(vp.height);
    await pg.render({ canvasContext: c.getContext("2d"), viewport: vp }).promise;
    t.querySelector(".frame").prepend(c);
  } catch (e) { console.error(e); }
}

function wireThumbDrag() {
  const box = $("#thumbs");
  let from = null;
  box.addEventListener("dragstart", (e) => {
    const t = e.target.closest(".thumb"); if (!t) return;
    from = +t.dataset.i; t.classList.add("dragging");
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(from));
  });
  const clear = () => $$(".thumb").forEach((x) => x.classList.remove("drop-before", "drop-after", "dragging"));
  box.addEventListener("dragend", clear);
  box.addEventListener("dragover", (e) => {
    const t = e.target.closest(".thumb"); if (!t || from == null) return;
    e.preventDefault();
    const r = t.getBoundingClientRect();
    const after = e.clientY > r.top + r.height / 2;
    $$(".thumb").forEach((x) => x.classList.remove("drop-before", "drop-after"));
    t.classList.add(after ? "drop-after" : "drop-before");
  });
  box.addEventListener("drop", (e) => {
    const t = e.target.closest(".thumb"); if (!t || from == null) return;
    e.preventDefault();
    const r = t.getBoundingClientRect();
    let to = +t.dataset.i + (e.clientY > r.top + r.height / 2 ? 1 : 0);
    clear();
    const a = [...list()];
    const [moved] = a.splice(from, 1);
    if (to > from) to--;
    a.splice(to, 0, moved);
    from = null;
    setArrangement(a, to);
  });
}

// ── page operations (pending until 另存新檔) ───────────────────────────────
async function setArrangement(a, focus) {
  const same = a.length === S.orig.length && a.every((r, i) => r.sha === S.orig[i].sha && r.page === S.orig[i].page);
  S.arr = same ? null : a;
  await buildView();
  scrollToPage(clamp(focus ?? S.cur, 0, S.pages.length - 1), false);
}

function dupPage() {
  const a = [...list()];
  a.splice(S.cur + 1, 0, { ...a[S.cur] });
  setArrangement(a, S.cur + 1);
}
function delPage() {
  const a = [...list()];
  if (a.length <= 1) { toast("至少要留一頁", true); return; }
  a.splice(S.cur, 1);
  setArrangement(a, Math.min(S.cur, a.length - 1));
}

async function saveArrangement() {
  const def = `${S.paper.title}（編輯）`;
  const r = await dialog((d, done) => {
    const input = h("input", { value: def });
    return h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); done(input.value.trim() || def); } },
      h("h3", {}, "另存"),
      h("label", {}, "標題", input),
      h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => done(null) }, "取消"), h("button", { class: "btn primary" }, "另存")));
  });
  if (r == null) return;
  try {
    const out = await api("/api/compose", { body: { pages: list().map(({ sha, page }) => ({ sha, page })), title: r, op: "edit" } });
    toast(`已存成新 PDF：${out.export}`);
    S.arr = null;
    go(out.paper.sha256, 1);
  } catch (e) { fail(e); }
}

async function insertDialog() {
  const lib = (await api("/api/papers")).papers.filter((p) => p.present);
  let chosen = null; const picked = [];
  const body = h("div", { class: "dlg" });
  const after = h("input", { type: "number", min: 0, max: S.pages.length, value: S.cur + 1, style: "width:72px" });
  const done = await dialog((d, close) => {
    const step1 = () => {
      const file = h("input", { type: "file", accept: "application/pdf,.pdf", hidden: true, onchange: async (e) => { const p = await importFiles([...e.target.files]); if (p) { chosen = p; step2(); } } });
      body.replaceChildren(
        h("h3", {}, "插入頁面"),
        h("ul", { class: "pick-list" }, lib.map((p) => h("li", {}, h("button", { type: "button", onclick: () => { chosen = p; step2(); } }, h("span", { class: "ellipsis" }, p.title), h("span", { class: "muted mono" }, `${p.pages} 頁`))))),
        h("div", { class: "actions" },
          h("label", { class: "btn", style: "flex-direction:row;color:var(--text);font-size:13px" }, h("span", { "data-icon": "upload" }), "從電腦選 PDF…", file),
          h("button", { type: "button", class: "btn", onclick: () => close(false) }, "取消")));
      paintIcons(body);
    };
    const step2 = async () => {
      picked.length = 0;
      const grid = h("div", { class: "pick-grid" });
      const go = h("button", { type: "button", class: "btn primary", disabled: true, onclick: () => close(true) }, "插入");
      body.replaceChildren(
        h("h3", { class: "ellipsis", title: chosen.title }, `插入頁面：${chosen.title}`),
        h("p", { class: "muted", style: "margin:0;font-size:12.5px" }, "依點選順序"),
        grid,
        h("div", { class: "actions", style: "justify-content:space-between;align-items:center" },
          h("span", { class: "muted", style: "font-size:13px;display:flex;gap:6px;align-items:center" }, "插在第", after, "頁之後"),
          h("span", { style: "display:flex;gap:8px" }, h("button", { type: "button", class: "btn", onclick: step1 }, "換一份 PDF"), go)));
      const doc = await getDoc(chosen.sha256);
      for (let n = 1; n <= doc.numPages; n++) {
        const c = h("canvas");
        const b = h("button", { type: "button", class: "pick", "aria-pressed": "false", onclick: () => {
          const i = picked.indexOf(n);
          if (i < 0) picked.push(n); else picked.splice(i, 1);
          $$(".pick", grid).forEach((x, k) => { const o = picked.indexOf(k + 1); x.setAttribute("aria-pressed", o >= 0); x.lastChild.textContent = o >= 0 ? `第 ${k + 1} 頁 · #${o + 1}` : `第 ${k + 1} 頁`; });
          go.disabled = !picked.length;
        } }, c, h("span", {}, `第 ${n} 頁`));
        grid.append(b);
        doc.getPage(n).then((pg) => { const vp = pg.getViewport({ scale: 180 / pg.getViewport({ scale: 1 }).width }); c.width = vp.width; c.height = vp.height; return pg.render({ canvasContext: c.getContext("2d"), viewport: vp }).promise; }).catch(() => {});
      }
    };
    step1();
    return body;
  });
  if (!done || !picked.length) return;
  const a = [...list()];
  const at = clamp(+after.value || 0, 0, a.length);
  a.splice(at, 0, ...picked.map((page) => ({ sha: chosen.sha256, page })));
  setArrangement(a, at);
}

async function mergeDialog() {
  const lib = (await api("/api/papers")).papers.filter((p) => p.present);
  if (lib.length < 2) { toast("書庫裡至少要有兩份 PDF 才能合併", true); return; }
  const order = [];
  const title = h("input", { value: "合併的 PDF" });
  const ok = await dialog((d, close) => {
    const ul = h("ul", { class: "pick-list" });
    const go = h("button", { class: "btn primary", disabled: true }, "合併");
    const paint = () => {
      ul.replaceChildren(...lib.map((p) => {
        const o = order.indexOf(p.sha256);
        return h("li", {}, h("button", { type: "button", "aria-pressed": o >= 0 ? "true" : "false", onclick: () => { o >= 0 ? order.splice(o, 1) : order.push(p.sha256); paint(); } },
          h("span", { class: "ellipsis" }, p.title), h("span", { class: o >= 0 ? "ord" : "muted mono" }, o >= 0 ? `#${o + 1}` : `${p.pages} 頁`)));
      }));
      go.disabled = order.length < 2;
    };
    paint();
    return h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); close(true); } },
      h("h3", {}, "合併 PDF"),
      h("p", { class: "muted", style: "margin:0;font-size:12.5px" }, "依點選順序"),
      ul, h("label", {}, "標題", title),
      h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(false) }, "取消"), go));
  });
  if (!ok) return;
  const pages = order.flatMap((sha) => { const p = lib.find((x) => x.sha256 === sha); return Array.from({ length: p.pages }, (_, i) => ({ sha, page: i + 1 })); });
  try {
    const out = await api("/api/compose", { body: { pages, title: title.value.trim(), op: "merge" } });
    toast(`已合併：${out.export}`);
    go(out.paper.sha256, 1);
  } catch (e) { fail(e); }
}

// ── generic dialog ────────────────────────────────────────────────────────
/** `build(dialog, close)` returns the content; resolves with what `close(v)` gets. */
function dialog(build) {
  const d = $("#dlg");
  return new Promise((resolve) => {
    let settled = false;
    const close = (v) => { if (settled) return; settled = true; d.close(); resolve(v); };
    d.replaceChildren(build(d, close));
    paintIcons(d);
    d.onclose = () => close(null);
    d.showModal();
    d.querySelector("input:not([type=file]):not([type=number]), textarea")?.focus();
  });
}

// ── selection ─────────────────────────────────────────────────────────────
/** Character offset in the page text of a DOM position inside a text layer. */
function domOffset(p, node, offset) {
  let el = node.nodeType === 3 ? node.parentElement : node;
  let div = null;
  for (let x = el; x && !x.classList?.contains("textLayer"); x = x.parentElement) if (p.divIndex?.has(x)) { div = x; break; }
  if (!div) return null;
  const i = p.divIndex.get(div);
  let n = 0;
  const walker = document.createTreeWalker(div, NodeFilter.SHOW_TEXT);
  for (let t = walker.nextNode(); t; t = walker.nextNode()) {
    if (t === node) return { i, off: n + offset };
    n += t.data.length;
  }
  return { i, off: node === div ? (offset ? div.textContent.length : 0) : 0 };
}

/** The current selection inside one page: text, page item, offsets and rects. */
async function selectionInfo() {
  const sel = getSelection();
  if (!sel || sel.isCollapsed || !sel.rangeCount) return null;
  const text = norm(sel.toString());
  if (!text) return null;
  const range = sel.getRangeAt(0);
  const pageEl = (range.startContainer.nodeType === 3 ? range.startContainer.parentElement : range.startContainer).closest?.(".page");
  if (!pageEl) return null;
  const p = S.pages[+pageEl.dataset.i];
  const pr = pageEl.getBoundingClientRect();
  const rects = [];
  for (const r of range.getClientRects()) {
    if (r.width < 1 || r.height < 1 || r.bottom < pr.top || r.top > pr.bottom) continue;
    const q = [(r.left - pr.left) / pr.width, (r.top - pr.top) / pr.height, (r.right - pr.left) / pr.width, (r.bottom - pr.top) / pr.height].map((x) => clamp(x, 0, 1));
    const last = rects[rects.length - 1];
    if (last && Math.abs(last[1] - q[1]) < 0.004 && Math.abs(last[3] - q[3]) < 0.004 && q[0] <= last[2] + 0.01) { last[0] = Math.min(last[0], q[0]); last[2] = Math.max(last[2], q[2]); }
    else rects.push(q);
  }
  const { text: ptext, items } = await pageText(p.sha, p.page);
  let start = null, end = null;
  const a = domOffset(p, range.startContainer, range.startOffset);
  const b = pageEl.contains(range.endContainer) ? domOffset(p, range.endContainer, range.endOffset) : null;
  if (a && items[a.i]) start = items[a.i].start + a.off;
  if (b && items[b.i]) end = items[b.i].start + b.off;
  const fromDom = !(start == null || end == null || end <= start);
  if (!fromDom) {
    const at = ptext.replace(/\s+/g, " ").indexOf(text);
    start = at >= 0 ? at : 0; end = start + text.length;
  }
  const exact = ptext.slice(start, end);
  return {
    p, text: (fromDom && norm(exact)) || text, rects, start, end, pageText: ptext,
    quote: { exact: norm(exact) || text, prefix: norm(ptext.slice(Math.max(0, start - 40), start)), suffix: norm(ptext.slice(end, end + 40)) },
    context: surrounding(ptext, start, end),
    box: range.getBoundingClientRect(),
  };
}

/** The sentences around [start, end): from the start of the sentence before through the
 *  end of the sentence after (at most ~900 characters). */
function surrounding(text, start, end) {
  const t = text.replace(/\n/g, " ");
  const stop = /[.!?。！？](\s|$)/g;
  let s = 0, cuts = [];
  for (let m; (m = stop.exec(t));) cuts.push(m.index + 1);
  const before = cuts.filter((c) => c <= start);
  s = before.length >= 2 ? before[before.length - 2] : 0;
  const after = cuts.filter((c) => c >= end);
  let e = after.length >= 2 ? after[1] : t.length;
  if (e - s > 900) { s = Math.max(s, start - 400); e = Math.min(e, end + 400); }
  return norm(t.slice(s, e));
}

// ── selection actions: the toolbar, the shortcuts and the context menu ────
/** What can be done with a selection; `key` is its shortcut (digits = highlight colours). */
function selActions(info) {
  const colors = S.info?.colors ?? ["yellow", "green", "blue", "pink"];
  return [
    ...colors.map((c, i) => ({ key: String(i + 1), color: c, label: `螢光（${COLOR_LABEL[c]}）`, run: () => addAnnotation(info, { kind: "highlight", color: c }) })),
    { key: "n", icon: "pen", label: "加註記…", run: () => noteDialog(info) },
    { key: "c", icon: "copy", label: "複製", hint: "C", run: () => navigator.clipboard.writeText(info.text).then(() => toast("已複製"), () => document.execCommand("copy")) },
    { key: "t", icon: "languages", label: "翻譯", hint: "T", run: () => translate(info.text, info.context, info.box, info) },
    { key: "a", icon: "chat", label: "問這篇", hint: "A", run: () => attach(info) },
    { key: "q", icon: "excerpt", label: "存成引用句…", hint: "Q", run: () => quoteDialog(info) },
    { key: "k", icon: "cards", label: "加入知識庫…", hint: "K", run: () => addCardDialog({ title: info.text, sha: info.p.sha, page: info.p.page, context: info.context }) },
  ].map((x) => ({ hint: x.key === "n" ? "N" : undefined, ...x }));
}

const hideSelBar = () => { $("#selBar").hidden = true; };

/** The small bar above a finished selection. */
async function showSelBar() {
  const info = await selectionInfo();
  if (!info) { hideSelBar(); return; }
  const bar = $("#selBar");
  const go = (x) => () => { hideSelBar(); x.run(); };
  const acts = selActions(info);
  bar.replaceChildren(
    ...acts.filter((x) => x.color).map((x) => h("button", { type: "button", class: `sw c-${x.color}`, title: `${x.label} (${x.key})`, "aria-label": x.label, onclick: go(x) })),
    h("span", { class: "sep" }),
    ...acts.filter((x) => !x.color).map((x) => h("button", { type: "button", class: "ico small", title: `${x.label} (${x.hint})`, "aria-label": x.label, "data-icon": x.icon, onclick: go(x) })));
  paintIcons(bar);
  bar.hidden = false;
  const b = info.box, w = bar.offsetWidth, hh = bar.offsetHeight;
  bar.style.left = `${clamp(b.left + b.width / 2 - w / 2, 8, innerWidth - w - 8)}px`;
  bar.style.top = `${b.top - hh - 8 < 56 ? Math.min(b.bottom + 8, innerHeight - hh - 8) : b.top - hh - 8}px`;
}

/** Shortcuts while a selection is inside a page: 1–4 highlight, N note, C copy, T translate,
 *  A ask, Q quote, K knowledge base. */
function onSelKey(e) {
  if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing || e.key.length !== 1) return;
  if (e.target.closest?.("input, textarea, select, dialog, [contenteditable]")) return;
  const sel = getSelection();
  if (!sel || sel.isCollapsed || !sel.anchorNode) return;
  if (!(sel.anchorNode.nodeType === 3 ? sel.anchorNode.parentElement : sel.anchorNode).closest?.(".page")) return;
  const k = e.key.toLowerCase();
  if (!"1234ncataqk".includes(k)) return;
  e.preventDefault();
  selectionInfo().then((info) => { if (!info) return; hideSelBar(); selActions(info).find((x) => x.key === k)?.run(); });
}

// ── context menu ──────────────────────────────────────────────────────────
function closeMenus() { $("#ctxMenu").hidden = true; }
function placeAt(el, x, y) {
  el.hidden = false;
  const w = el.offsetWidth, hh = el.offsetHeight;
  el.style.left = `${clamp(x, 8, innerWidth - w - 8)}px`;
  el.style.top = `${y + hh + 8 > innerHeight ? Math.max(8, y - hh) : y}px`;
}

async function onContextMenu(e) {
  if (!e.target.closest(".page")) return;
  const mk = e.target.closest?.(".mk[data-t^=n]");
  if (mk && getSelection().isCollapsed) {
    const n = S.nouns?.entries[+mk.dataset.t.slice(1)];
    if (n) { e.preventDefault(); nounMenu(n, e.clientX, e.clientY); }
    return;
  }
  const info = await selectionInfo();
  if (!info) return;
  e.preventDefault();
  $("#selBar").hidden = true;
  const m = $("#ctxMenu");
  const item = (x) => h("button", { type: "button", role: "menuitem", onclick: () => { closeMenus(); x.run(); } }, h("span", { "data-icon": x.icon }), x.label, x.hint ? h("span", { class: "muted menu-sub" }, x.hint) : null);
  const acts = selActions(info);
  const pick = (k) => acts.find((x) => x.key === k);
  m.replaceChildren(
    item(pick("c")),
    h("hr"),
    h("div", { class: "swrow" }, h("span", { class: "swicon", "data-icon": "highlighter", title: "螢光" }), acts.filter((x) => x.color).map((x) => h("button", { type: "button", class: `sw c-${x.color}`, "aria-label": x.label, title: `${COLOR_LABEL[x.color]}色 (${x.key})`, onclick: () => { closeMenus(); x.run(); } }))),
    item(pick("n")),
    h("hr"),
    item(pick("t")), item(pick("a")), item(pick("q")), item(pick("k")),
  );
  paintIcons(m);
  placeAt(m, e.clientX, e.clientY);
}

// ── annotations ───────────────────────────────────────────────────────────
async function addAnnotation(info, { kind, color, note = "" }) {
  try {
    const r = await api(`/api/papers/${info.p.sha}/annotations`, { body: { page: info.p.page, kind, color, note, quote: info.quote, rects: info.rects } });
    getSelection().removeAllRanges();
    if (info.p.sha === S.sha) { S.annotations.push(r.annotation); renderSide("notes"); }
    S.pages.filter((p) => p.sha === info.p.sha && p.page === info.p.page && p.rendered).forEach(drawHighlights);
    toast(kind === "note" ? "已加註記" : "已螢光");
    refreshExport();
  } catch (e) { fail(e); }
}

async function noteDialog(info) {
  const ta = h("textarea");
  let color = "yellow";
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); close(ta.value.trim()); } },
    h("h3", {}, `加註記 · 第 ${info.p.page} 頁`),
    h("div", { class: "quote-box" }, info.text),
    h("label", {}, "註記", ta),
    h("div", { class: "swrow", style: "display:flex;gap:6px" }, ["yellow", "green", "blue", "pink"].map((c) => h("button", { type: "button", class: `sw c-${c}`, "aria-pressed": c === color ? "true" : "false", "aria-label": `${COLOR_LABEL[c]}色`, onclick: (e) => { color = c; $$(".sw", e.target.parentElement).forEach((x) => x.setAttribute("aria-pressed", x === e.target)); } }))),
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"), h("button", { class: "btn primary" }, "儲存"))));
  if (r == null) return;
  addAnnotation(info, { kind: "note", color, note: r });
}

function drawHighlights(p) {
  const layer = p.el.querySelector(".hl-layer");
  if (!layer) return;
  const own = S.annotations.filter((a) => p.sha === S.sha && a.page === p.page);
  layer.replaceChildren(...own.flatMap((a) => (a.rects ?? []).map((r, k) => {
    const d = h("div", { class: `hl c-${a.color}${a.note && k === 0 ? " has-note" : ""}`, dataset: { id: a.id }, title: a.note || "" });
    Object.assign(d.style, { left: `${r[0] * 100}%`, top: `${r[1] * 100}%`, width: `${(r[2] - r[0]) * 100}%`, height: `${(r[3] - r[1]) * 100}%` });
    return d;
  })));
  markUnderHl(p);
}

function jumpTo(page, id) {
  const i = S.pages.findIndex((p) => p.sha === S.sha && p.page === page);
  if (i < 0) { toast("這一頁不在目前的頁面清單裡", true); return; }
  const p = S.pages[i];
  const a = id && S.annotations.find((x) => x.id === id);
  const vpEl = $("#viewport");
  const y = p.el.offsetTop + (a?.rects?.[0] ? a.rects[0][1] * p.el.offsetHeight - vpEl.clientHeight * 0.3 : -12);
  vpEl.scrollTo({ top: y, behavior: "smooth" });
  setCur(i);
  if (id) setTimeout(() => $$(`.hl[data-id="${id}"]`).forEach((x) => { x.classList.remove("flash"); void x.offsetWidth; x.classList.add("flash"); }), 450);
  if (innerWidth <= 1100) shell.drawer("right", false);
}

// ── citable sentences ─────────────────────────────────────────────────────
async function quoteDialog(info) {
  const ta = h("textarea", {}, info.text);
  const note = h("input", { placeholder: "例：用在引言" });
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); close({ text: ta.value, note: note.value }); } },
    h("h3", {}, `存成引用句 · 第 ${info.p.page} 頁`),
    h("label", {}, "句子", ta),
    h("label", {}, "備註", note),
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"), h("button", { class: "btn primary" }, "儲存"))));
  if (!r) return;
  try {
    await api(`/api/papers/${info.p.sha}/quotes`, { body: { text: r.text, page: info.p.page, note: r.note } });
    getSelection().removeAllRanges();
    if (info.p.sha === S.sha) { await loadQuotes(); renderSide(); }
    toast("已存成引用句");
  } catch (e) { fail(e); }
}

// ── translate ─────────────────────────────────────────────────────────────
function showPop(anchor, ...kids) {
  const pop = $("#pop");
  pop.replaceChildren(...kids);
  paintIcons(pop);
  const r = anchor ?? { left: innerWidth / 2 - 180, bottom: 120, top: 100 };
  pop.hidden = false;
  const below = r.bottom + 8 + pop.offsetHeight < innerHeight;
  pop.style.left = `${clamp(r.left, 8, innerWidth - pop.offsetWidth - 8)}px`;
  pop.style.top = `${below ? r.bottom + 8 : Math.max(8, r.top - pop.offsetHeight - 8)}px`;
}
const hidePop = () => { $("#pop").hidden = true; };

async function translate(text, context, box, info) {
  const anchor = box ? { left: box.left, top: box.top, bottom: box.bottom } : null;
  const head = h("h4", {}, h("span", { class: "ellipsis", style: "max-width:100%" }, text.length > 80 ? `${text.slice(0, 80)}…` : text));
  showPop(anchor, head, h("div", { class: "muted" }, "翻譯中…"));
  try {
    const r = await api("/api/translate", { body: { text, context } });
    S.used["lr-translate"] = r.engine;
    const kids = [head, h("div", { class: "out" }, r.translation), h("div", { class: "src eng-line" }, engineChip("lr-translate"), r.fallback_used ? " 改用 Claude" : "")];
    const acts = h("div", { class: "actions" },
      h("button", { type: "button", class: "btn small", onclick: () => navigator.clipboard.writeText(r.translation).then(() => toast("已複製")) }, h("span", { "data-icon": "copy" }), "複製"));
    if (info) acts.append(h("button", { type: "button", class: "btn small", onclick: () => { hidePop(); addCardDialog({ title: text, sha: info.p.sha, page: info.p.page, context: info.context, hint: r.translation }); } }, h("span", { "data-icon": "cards" }), "加入知識庫…"));
    showPop(anchor, ...kids, acts);
  } catch (e) {
    showPop(anchor, head, h("div", { class: "err-box" }, e.message));
  }
}

// ── knowledge base ────────────────────────────────────────────────────────
async function addCardDialog({ title, sha, page, context, hint, summary }) {
  hidePop();
  let kb = S.kb;
  try { kb = S.kb = await api("/api/kb/status"); } catch {}
  updateLegend();
  const t = h("input", { value: title.length > 120 ? title.slice(0, 120) : title });
  const sum = h("textarea", { placeholder: "用自己的話寫一兩句" }, summary ?? "");
  let state = "learning";
  const seg = h("div", { class: "seg", role: "group", "aria-label": "初始狀態" });
  const segHint = h("span", { class: "seg-hint" });
  const paintSeg = () => {
    seg.replaceChildren(...[["learning", "學習中"], ["known", "記得"]].map(([v, l]) => h("button", { type: "button", "aria-pressed": v === state ? "true" : "false", onclick: () => { state = v; paintSeg(); } }, l)));
    segHint.textContent = "";
  };
  paintSeg();
  const status = kb?.ok ? null : kbNote();
  const draftBtn = h("button", { type: "button", class: "btn", disabled: !kb?.ok, onclick: async () => {
    draftBtn.disabled = true; draftBtn.textContent = "起草中…";
    try { const r = await api("/api/kb/draft", { body: { term: t.value, context, sha, page } }); if (r.draft?.summary) sum.value = r.draft.summary; toast(`草稿：${r.assistant ?? ""}`); }
    catch (e) { fail(e); }
    draftBtn.disabled = false; draftBtn.replaceChildren(h("span", { "data-icon": "spark" }), "起草摘要"); paintIcons(draftBtn);
  } }, h("span", { "data-icon": "spark" }), "起草摘要");
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); close(true); } },
    h("h3", {}, "加入知識庫"),
    status,
    h("label", {}, "標題", t),
    hint ? h("div", { class: "quote-box" }, h("span", { class: "muted" }, "譯文："), hint) : null,
    h("label", {}, "摘要", sum),
    h("div", { style: "display:flex;flex-direction:column;gap:4px" }, h("span", { style: "font-size:12.5px;color:var(--muted)" }, "狀態"), seg),
    h("div", { class: "muted", style: "font-size:12px;overflow-wrap:anywhere" }, `${S.paper?.sha256 === sha ? S.paper.title : "PDF"} · p.${page}`),
    h("div", { class: "actions" },
      kb?.ok ? draftBtn : h("button", { type: "button", class: "btn", onclick: () => close("retry") }, "重新檢查"),
      h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"),
      h("button", { class: "btn primary", disabled: !kb?.ok }, "儲存"))));
  if (r === "retry") return addCardDialog({ title: t.value, sha, page, context, hint, summary: sum.value });
  if (r !== true) return;
  try {
    const out = await api("/api/kb/cards", { body: { sha, page, title: t.value, summary: sum.value, initial_state: state, context } });
    toast(`已加入知識庫：${out.node.title}`);
    // The new card changes the noun list and the colouring of every page.
    refreshNouns();
  } catch (e) { fail(e); }
}

function updateLegend() {
  $("#actTerms")?.setAttribute("aria-pressed", S.termsOn ? "true" : "false");
}

/** Re-mark one page's text layer: knowledge colouring, then search hits on top. */
async function decorate(p) {
  if (!p.divs) return;
  const { items } = await pageText(p.sha, p.page);
  const marks = [];
  if (p.sha === S.sha) {
    for (const m of S.nounMarks.get(p.page) ?? []) {
      const lit = S.focusNoun?.i === m.i;
      const cur = lit && S.focusNoun.cur?.[0] === p.page && S.focusNoun.cur?.[1] === m.s;
      const cls = [S.termsOn ? m.cls : "", lit ? "nf" : "", cur ? "nf-cur" : ""].filter(Boolean).join(" ");
      if (cls) marks.push({ s: m.s, e: m.e, cls, t: `n${m.i}` });
    }
  }
  if (p.sha === S.sha) for (const m of S.symMarks.get(p.page) ?? []) marks.push({ s: m.s, e: m.e, cls: "sy", t: `y${m.k}` });
  const pi = S.pages.indexOf(p);
  S.search.hits.forEach((x, n) => { if (x.i === pi) marks.push({ s: x.start, e: x.end, cls: n === S.search.i ? "sr sr-cur" : "sr" }); });
  const lnums = S.lineNums.get(key(p.sha, p.page)) ?? [];
  p.divs.forEach((div, i) => {
    const it = items[i], str = p.strs[i];
    if (!it) return;
    if (lnums.length) div.classList.toggle("lnum", lnums.some(([s, e]) => s <= it.start && it.end <= e && it.end > it.start));
    const mine = marks.filter((m) => m.s < it.end && m.e > it.start);
    if (!mine.length) { if (div.dataset.marked) { div.textContent = str; delete div.dataset.marked; } return; }
    // classes per character; later marks win
    const per = Array.from({ length: str.length }, () => null);
    for (const m of mine) for (let c = Math.max(0, m.s - it.start); c < Math.min(str.length, m.e - it.start); c++) per[c] = m;
    const out = [];
    let c = 0;
    while (c < str.length) {
      const m = per[c]; let d = c + 1;
      while (d < str.length && per[d] === m) d++;
      const piece = str.slice(c, d);
      out.push(m ? h("span", { class: `mk ${m.cls}`, dataset: m.t ? { t: m.t } : {} }, piece) : document.createTextNode(piece));
      c = d;
    }
    div.replaceChildren(...out);
    div.dataset.marked = "1";
  });
  markUnderHl(p);
}

/** Knowledge colouring under a highlight would mix the two colours: such terms keep an
 *  underline only (`.under-hl`). */
function markUnderHl(p) {
  if (!p.el) return;
  const rects = p.sha === S.sha ? S.annotations.filter((a) => a.page === p.page).flatMap((a) => a.rects ?? []) : [];
  const marks = p.el.querySelectorAll(".mk.kt-new, .mk.kt-learn, .mk.under-hl");
  if (!marks.length) return;
  const pr = rects.length ? p.el.getBoundingClientRect() : null;
  marks.forEach((m) => {
    let on = false;
    if (pr) {
      const r = m.getBoundingClientRect();
      const cx = (r.left + r.width / 2 - pr.left) / pr.width, cy = (r.top + r.height / 2 - pr.top) / pr.height;
      on = rects.some((q) => cx >= q[0] && cx <= q[2] && cy >= q[1] && cy <= q[3]);
    }
    m.classList.toggle("under-hl", on);
  });
}

async function onPageClick(e) {
  const mk = e.target.closest?.(".mk[data-t]");
  if (!mk || !getSelection().isCollapsed) return;
  const p = S.pages[+mk.closest(".page").dataset.i];
  const t = mk.dataset.t;
  const box = mk.getBoundingClientRect();
  if (t[0] === "n") nounPopover(S.nouns?.entries[+t.slice(1)], { left: box.left, top: box.top, bottom: box.bottom }, p);
  if (t[0] === "y") { clearTimeout(S.symHover); symPopover(S.symSpans[+t.slice(1)], { left: box.left, top: box.top, bottom: box.bottom }, true); }
}

// ── symbols (variables and equations, step 1) ─────────────────────────────
/** KaTeX for one LaTeX string (inline unless `display`); the source as text if KaTeX is missing. */
function tex(latex, display = false) {
  const el = h("span", { class: display ? "tex tex-d" : "tex" });
  try { el.innerHTML = window.katex.renderToString(latex, { throwOnError: false, displayMode: display, errorColor: "currentColor", strict: "ignore" }); }
  catch { el.textContent = latex; }
  return el;
}
/** Text with `$…$` math rendered. */
function mixed(text) {
  const out = h("span", { class: "mixed" });
  String(text ?? "").split(/(\$[^$]+\$)/).forEach((part) => out.append(part.startsWith("$") && part.endsWith("$") && part.length > 2 ? tex(part.slice(1, -1)) : part));
  return out;
}
function gradeChip(g) {
  const [icon, label] = GRADE[g] ?? ["⚪", g];
  return h("span", { class: `grade g-${g}`, title: label }, icon, " ", label);
}

function setSymbols(f) {
  S.symbols = f;
  const spans = new Map();
  (f?.entries ?? []).forEach((e, i) => {
    for (const o of e.occurrences ?? []) {
      const k = `${o.page}:${o.s}:${o.e}`;
      if (!spans.has(k)) spans.set(k, { page: o.page, s: o.s, e: o.e, bbox: o.bbox, latex: o.latex, eq: o.eq, ents: [] });
      spans.get(k).ents.push(i);
    }
  });
  S.symSpans = [...spans.values()];
  S.symMarks = new Map();
  S.symSpans.forEach((sp, k) => {
    if (!S.symMarks.has(sp.page)) S.symMarks.set(sp.page, []);
    S.symMarks.get(sp.page).push({ s: sp.s, e: sp.e, k });
  });
  S.pages.filter((p) => p.rendered).forEach(decorate);
  renderSide("symbols");
  if (ntDlg?.open) ntPanel.notation = notationView();
}

function symMeaning(e) {
  return e.meaning || e.meaning_en || "";
}

function symPopover(sp, anchor, sticky) {
  if (!sp) return;
  const ents = sp.ents.map((i) => S.symbols.entries[i]);
  const known = ents.filter((e) => e.grade !== "unresolved");
  const none = ents.filter((e) => e.grade === "unresolved");
  const kids = [h("div", { class: "sym-run" }, tex(sp.latex), h("span", { class: "src" }, `p.${sp.page}${sp.eq ? ` · ${q_label(sp.eq)}` : ""}`))];
  for (const e of known) {
    const m = symMeaning(e);
    const ev = e.evidence?.[0];
    kids.push(h("div", { class: "sym-def" },
      h("div", { class: "sym-head" }, tex(e.latex ?? e.id), gradeChip(e.grade)),
      m ? h("div", { class: "sym-mean" }, mixed(m)) : null,
      ev?.quote ? h("div", { class: "sym-quote" }, mixed(ev.quote), h("span", { class: "src" }, ` p.${ev.page || "?"}${byLabel(ev.by)}`)) : null));
  }
  if (none.length) kids.push(h("div", { class: "sym-none" }, gradeChip("unresolved"), ...none.map((e) => tex(e.latex ?? e.id))));
  if (sticky) kids.push(h("div", { class: "actions" }, h("button", { type: "button", class: "btn small", onclick: () => { hidePop(); setTab("symbols"); setTimeout(() => scrollWithin($("#side"), $(`.sym-item[data-i="${sp.ents[0]}"]`)), 60); } }, h("span", { "data-icon": "sigma" }), "符號表")));
  showPop(anchor, ...kids);
  $("#pop").classList.toggle("hover-pop", !sticky);
}
/** " · 作者" etc.; nothing for the rules (the default). */
const byLabel = (by) => (by && by !== "program" ? ` · ${RESOLVER[by] ?? by}` : "");

function onPageHover(e) {
  const mk = e.target.closest?.(".mk[data-t^=y]");
  clearTimeout(S.symHover);
  if (!mk) return;
  S.symHover = setTimeout(() => {
    if (!$("#pop").hidden && !$("#pop").classList.contains("hover-pop")) return;
    const b = mk.getBoundingClientRect();
    symPopover(S.symSpans[+mk.dataset.t.slice(1)], { left: b.left, top: b.top, bottom: b.bottom }, false);
  }, 220);
}
function onPageHoverOut(e) {
  if (!e.target.closest?.(".mk[data-t^=y]")) return;
  clearTimeout(S.symHover);
  if ($("#pop").classList.contains("hover-pop") && !e.relatedTarget?.closest?.("#pop")) S.symHover = setTimeout(() => { if ($("#pop").classList.contains("hover-pop")) hidePop(); }, 250);
}

/** Scroll to a box on a page (fractions, top-left origin) and flash it. */
function flashBox(page, bbox) {
  const i = S.pages.findIndex((p) => p.sha === S.sha && p.page === page);
  if (i < 0) { toast("這一頁不在目前的頁面清單裡", true); return; }
  const p = S.pages[i];
  const vpEl = $("#viewport");
  vpEl.scrollTo({ top: p.el.offsetTop + bbox[1] * p.el.offsetHeight - vpEl.clientHeight * 0.3, behavior: "smooth" });
  setCur(i);
  const pad = 0.004;
  const d = h("div", { class: "boxflash" });
  Object.assign(d.style, { left: `${(bbox[0] - pad) * 100}%`, top: `${(bbox[1] - pad) * 100}%`, width: `${(bbox[2] - bbox[0] + 2 * pad) * 100}%`, height: `${(bbox[3] - bbox[1] + 2 * pad) * 100}%` });
  p.el.append(d);
  setTimeout(() => d.remove(), 2600);
  if (innerWidth <= 1100) shell.drawer("right", false);
}

function symbolsPane() {
  const f = S.symbols;
  if (!f) return [h("p", { class: "empty" }, S.readStatus || "…")];
  const ents = f.entries ?? [];
  const count = (g) => ents.filter((e) => e.grade === g).length;
  const out = [];
  const gsel = (g) => h("button", { type: "button", class: `nn-stat${S.symView.grade === g ? " on" : ""}`, "aria-pressed": S.symView.grade === g ? "true" : "false", title: GRADE[g][1], onclick: () => { S.symView.grade = S.symView.grade === g ? "" : g; renderSide(); } }, GRADE[g][0], " ", h("b", {}, count(g)));
  out.push(h("div", { class: "sym-tools" },
    h("span", { class: "sym-total" }, `${ents.length} 符號 · ${f.equations?.length ?? 0} 式`),
    ...["conflict", "inferred", "defined", "certain", "unresolved"].map(gsel),
    h("span", { style: "flex:1" }),
    h("button", { type: "button", class: "ico small", "aria-label": "對照表", "data-icon": "book", onclick: () => openNotation() }),
    h("button", { type: "button", class: "ico small", "aria-label": "新增符號…", "data-icon": "plus", onclick: () => symbolDialog(null) }),
    h("button", { type: "button", class: "ico small", "aria-label": "重新偵測", "data-icon": "refresh", onclick: rebuildSymbols }),
    h("button", { type: "button", class: "ico small", "aria-label": "字形對照表…", "data-icon": "gear", onclick: glyphDialog })));
  if (f.glyph_errors?.length) out.push(h("div", { class: "warn-box" }, `字形對照表有錯：${f.glyph_errors.join("；")}`));
  const conv = (e) => (e.tags?.includes("conventional") ? 1 : 0);
  const shown = ents.map((e, i) => ({ e, i }))
    .filter(({ e }) => !S.symView.grade || e.grade === S.symView.grade)
    .sort((a, b) => conv(a.e) - conv(b.e) || (GRADE_ORDER[a.e.grade] ?? 9) - (GRADE_ORDER[b.e.grade] ?? 9) || a.i - b.i);
  S.symOpen ??= new Set();
  const list = h("div", { class: "sym-list" }, shown.length ? shown.map(({ e, i }) => {
    const occ = e.occurrences ?? [];
    const pages = [...new Set(occ.map((o) => o.page))];
    const ev = e.evidence?.[0];
    const by = e.resolved_by && e.resolved_by !== "program" ? h("span", { class: "sym-by" }, RESOLVER[e.resolved_by] ?? e.resolved_by) : null;
    const edit = h("button", { class: "ico small", "aria-label": "編輯…", "data-icon": "pen", onclick: () => symbolDialog(e) });
    const latexBtn = h("button", { type: "button", class: "sym-latex", title: e.latex, onclick: () => occ[0] && flashBox(occ[0].page, occ[0].bbox) }, tex(e.latex ?? e.id));
    const folded = e.grade === "unresolved" && !S.symOpen.has(e.id);
    if (folded) {
      return h("div", { class: "sym-item one", dataset: { i } },
        h("div", { class: "sym-head" }, latexBtn,
          h("button", { type: "button", class: "sym-more", "aria-label": "展開", onclick: () => { S.symOpen.add(e.id); renderSide(); } }, occ.length ? `${occ.length} 次` : "文中沒偵測到"),
          e.tags?.includes("conventional") ? h("span", { class: "sym-by" }, "慣用") : null, by, edit));
    }
    const pageLinks = pages.slice(0, 6).map((pg) => h("button", { type: "button", class: "pg-link", onclick: () => { const o = occ.find((x) => x.page === pg); flashBox(pg, o.bbox); } }, `p.${pg}`));
    return h("div", { class: "sym-item", dataset: { i } },
      h("div", { class: "sym-head" }, latexBtn, gradeChip(e.grade),
        e.tags?.includes("conventional") ? h("span", { class: "sym-by" }, "慣用") : null, by,
        e.grade === "unresolved" ? h("button", { type: "button", class: "ico small", "aria-label": "收起", "data-icon": "up", onclick: () => { S.symOpen.delete(e.id); renderSide(); } }) : null,
        edit),
      symMeaning(e) ? h("div", { class: "sym-mean" }, mixed(symMeaning(e)), e.meaning && e.meaning_en ? h("span", { class: "muted" }, " · ", mixed(e.meaning_en)) : null) : null,
      ev?.quote ? h("div", { class: "sym-quote" }, mixed(ev.quote), h("span", { class: "src" }, ` p.${ev.page || "?"}${e.evidence.length > 1 ? ` · 另有 ${e.evidence.length - 1} 句` : ""}`)) : null,
      e.note ? h("div", { class: "sym-note" }, e.note) : null,
      h("div", { class: "sym-meta" },
        occ.length ? h("span", {}, `${occ.length} 次`) : h("span", {}, "文中沒偵測到"),
        ...pageLinks,
        pages.length > 6 ? h("span", {}, `＋${pages.length - 6}`) : null,
        ...eqChips(e.equations ?? [], f)));
  }) : [h("p", { class: "empty" }, "沒有符合的符號")]);
  out.push(list);
  if (f.equations?.length) {
    out.push(h("h4", { class: "sym-sub" }, "式"));
    out.push(h("div", { class: "sym-list eqs" }, f.equations.map((q) => h("button", { type: "button", class: "eq-item", onclick: () => flashBox(q.page, q.bbox) },
      h("span", { class: "eq-no" }, q.label ?? "無編號"), h("span", { class: "src" }, `p.${q.page}`), q.latex ? h("span", { class: "eq-tex" }, tex(q.latex)) : null))));
  }
  return out;
}

function eqChips(ids, f) {
  const jump = (id) => { const q = f.equations.find((x) => x.id === id); if (q) flashBox(q.page, q.bbox); };
  const numbered = ids.filter((id) => !/^eq:p\d+-/.test(id));
  const other = ids.filter((id) => /^eq:p\d+-/.test(id));
  const out = numbered.slice(0, 6).map((id) => h("button", { type: "button", class: "pg-link", onclick: () => jump(id) }, q_label(id)));
  if (numbered.length > 6) out.push(h("span", {}, `…另 ${numbered.length - 6} 條`));
  if (other.length) out.push(h("button", { type: "button", class: "pg-link", onclick: () => jump(other[0]) }, `無編號 ×${other.length}`));
  return out;
}
const q_label = (id) => (/^eq:p\d+-/.test(id) ? "無編號" : `式 (${id.replace(/^eq:/, "")})`);

async function rebuildSymbols() {
  try { setSymbols(await api(`/api/papers/${S.sha}/symbols/rebuild`, { body: {} })); toast("已重新偵測"); }
  catch (e) { if (e.data?.no_text) readPaper(S.sha); else fail(e); }
}

async function symbolDialog(e) {
  const latex = h("input", { value: e?.latex ?? "", placeholder: "例：w_i、\\hat{y}、\\mathbf{x}", class: "mono" });
  const preview = h("div", { class: "tex-preview" });
  const paint = () => preview.replaceChildren(latex.value.trim() ? tex(latex.value, true) : "");
  latex.addEventListener("input", paint); paint();
  const zh = h("textarea", { placeholder: "中文" }, e?.meaning ?? "");
  const en = h("input", { value: e?.meaning_en ?? "", placeholder: "可用 $…$" });
  const note = h("input", { value: e?.note ?? "" });
  let grade = e?.grade && e.grade !== "unresolved" ? e.grade : "certain";
  const seg = h("div", { class: "seg", role: "group", "aria-label": "等級" });
  const paintSeg = () => seg.replaceChildren(...Object.entries(GRADE).map(([g, [i, l]]) => h("button", { type: "button", "aria-pressed": g === grade ? "true" : "false", onclick: () => { grade = g; paintSeg(); } }, `${i} ${l}`)));
  paintSeg();
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (ev) => { ev.preventDefault(); close(true); } },
    h("h3", {}, e ? "編輯符號" : "新增符號"),
    e?.evidence?.length ? h("div", { class: "quote-box" }, mixed(e.evidence[0].quote), h("span", { class: "muted" }, ` p.${e.evidence[0].page}`)) : null,
    h("label", {}, "LaTeX", latex), preview,
    h("label", {}, "意思", zh), h("label", {}, "原文定義", en),
    h("div", { style: "display:flex;flex-direction:column;gap:4px" }, h("span", { style: "font-size:12.5px;color:var(--muted)" }, "等級"), seg),
    h("label", {}, "備註", note),
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"), h("button", { class: "btn primary" }, "儲存"))));
  if (r !== true) return;
  const body = { latex: latex.value, meaning: zh.value, meaning_en: en.value, grade, note: note.value };
  try {
    if (e) await api(`/api/papers/${S.sha}/symbols/${encodeURIComponent(e.id)}`, { method: "PUT", body });
    else await api(`/api/papers/${S.sha}/symbols`, { body });
    setSymbols(await api(`/api/papers/${S.sha}/symbols`));
    toast("已儲存");
  } catch (x) { fail(x); }
}

async function glyphDialog() {
  let g;
  try { g = await api("/api/glyphs"); } catch (e) { fail(e); return; }
  const ta = h("textarea", { class: "mono", style: "min-height:220px", spellcheck: "false" }, g.text);
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (ev) => { ev.preventDefault(); close(true); } },
    h("h3", {}, "字形對照表"),
    h("p", { class: "muted", style: "margin:0;font-size:12.5px" }, "PDF 字元 → LaTeX"),
    g.errors?.length ? h("div", { class: "warn-box" }, g.errors.join("；")) : null,
    ta,
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"), h("button", { class: "btn primary" }, "儲存"))));
  if (r !== true) return;
  try { await api("/api/glyphs", { method: "PUT", body: { text: ta.value } }); await rebuildSymbols(); } catch (e) { fail(e); }
}

function toggleTerms() {
  S.termsOn = !S.termsOn;
  try { localStorage.setItem("lr.terms", S.termsOn ? "1" : "0"); } catch {}
  updateLegend();
  S.pages.filter((p) => p.rendered).forEach(decorate);
}

// ── the noun list (track A) ───────────────────────────────────────────────
/** Read the whole paper (text, fonts, positions) and send it to the server once;
 *  the noun list (and later the symbol table and paragraphs) are built from it. */
async function readPaper(sha) {
  S.readStatus = "讀取全文…";
  renderSide("nouns");
  try {
    const doc = await getDoc(sha);
    const pages = [];
    for (let n = 1; n <= doc.numPages; n++) pages.push(await pageRich(sha, n));
    if (S.sha !== sha) return;
    const r = await api(`/api/papers/${sha}/read`, { body: { pages } });
    if (S.sha !== sha) return;
    S.readStatus = "";
    api(`/api/papers/${sha}`).then(({ paper }) => {
      if (S.sha === sha && paper.title !== S.paper.title) {
        S.paper = paper; shell.setName(paper.title); document.title = `${paper.title} · LoreReader`;
      }
    }).catch(() => {});
    S.words = r.words;
    await setLineNums(sha, r.line_numbers);
    setSymbols(r.symbols);
    setNouns(r.nouns);
    loadParagraphs(sha);
    if (r.nouns.classification.pending > 0) classifyNouns(false);
  } catch (e) {
    if (S.sha !== sha) return;
    S.readStatus = `讀不到全文：${e.message}`;
    renderSide("nouns");
  }
}

async function refreshNouns() {
  if (!S.sha || !S.nouns) return;
  try { setNouns(await api(`/api/papers/${S.sha}/nouns`)); } catch (e) { if (e.data?.no_text) readPaper(S.sha); else fail(e); }
}

function nounClass(n) {
  if (n.state === "known" || n.used) return "kt-known";
  if (["learning", "stale", "none"].includes(n.state)) return "kt-learn";
  if (n.red) return "kt-new";
  return "";
}

function setNouns(n) {
  const focusKey = S.focusNoun ? S.nouns?.entries[S.focusNoun.i]?.key : null;
  S.nouns = n;
  S.nounMarks = new Map();
  n.entries.forEach((e, i) => {
    const cls = nounClass(e);
    for (const [page, s, en] of e.occ) {
      if (!S.nounMarks.has(page)) S.nounMarks.set(page, []);
      S.nounMarks.get(page).push({ s, e: en, cls, i });
    }
  });
  if (focusKey) {
    const i = n.entries.findIndex((e) => e.key === focusKey);
    S.focusNoun = i >= 0 ? { ...S.focusNoun, i } : null;
  }
  if (n.kb && !n.kb.ok && S.kb?.ok !== false) { S.kb = { ok: false, error: n.kb.error }; updateLegend(); }
  if (n.kb && !n.kb.ok) watchKb();
  S.pages.filter((p) => p.rendered).forEach(decorate);
  renderSide("nouns");
}

async function classifyNouns(again) {
  if (!S.sha || S.classifying) return;
  const sha = S.sha;
  S.classifying = true; S.classifyError = null;
  renderSide("nouns");
  try {
    const r = await api(`/api/papers/${sha}/nouns/classify`, { body: { again } });
    if (S.sha === sha) { setNouns(r.nouns); loadParagraphs(sha); }
  } catch (e) {
    S.classifyError = e.data?.llm_unavailable ? "沒有可用的模型，名詞未分類。" : e.message;
  }
  S.classifying = false;
  if (S.sha === sha) renderSide("nouns");
}

/** Light every occurrence of noun `i` and go to occurrence `k` (wraps). */
function focusNoun(i, k = 0) {
  const n = S.nouns?.entries[i];
  if (!n) return;
  const len = n.occ.length;
  const kk = ((k % len) + len) % len;
  const [page, s] = n.occ[kk];
  S.focusNoun = { i, k: kk, cur: [page, s] };
  S.pages.filter((p) => p.rendered).forEach(decorate);
  renderSide("nouns");
  const pi = S.pages.findIndex((p) => p.sha === S.sha && p.page === page);
  if (pi < 0) { toast("這一頁不在目前的頁面清單裡", true); return; }
  scrollToPage(pi, false);
  setTimeout(() => scrollWithin($("#viewport"), $(".nf-cur")), 250);
}
function clearNounFocus() {
  S.focusNoun = null;
  S.pages.filter((p) => p.rendered).forEach(decorate);
  renderSide("nouns");
}

/** The sentence around a noun's first occurrence (for KB cards and translation). */
async function nounContext(n, k = 0) {
  const [page, s, e] = n.occ[k] ?? n.occ[0];
  const { text } = await pageText(S.sha, page);
  return { page, context: surrounding(text, s, e) };
}

async function ignoreNoun(n) {
  try {
    await api("/api/ignore", { body: { term: n.text } });
    hidePop();
    if (S.focusNoun && S.nouns.entries[S.focusNoun.i]?.key === n.key) S.focusNoun = null;
    await refreshNouns();
    toast(`已忽略：${n.text}`);
  } catch (e) { fail(e); }
}

async function nounCard(n) {
  const { page, context } = await nounContext(n, S.focusNoun?.i === S.nouns.entries.indexOf(n) ? S.focusNoun.k : 0);
  addCardDialog({ title: n.text, sha: S.sha, page, context });
}

function stateChip(n) {
  return h("span", { class: "st", title: STATE_TIP[n.state] ?? "" }, h("i", { class: `s-${n.state === "new" && !n.red ? "none-card" : n.state}` }), nounState(n));
}

async function nounPopover(n, anchor, p) {
  if (!n) return;
  const i = S.nouns.entries.indexOf(n);
  const head = h("h4", {}, n.text, stateChip(n));
  const meta = h("div", { class: "src" }, [n.category, n.short, `${n.count} 次`, `p.${n.first_page}`].filter(Boolean).join(" · "));
  const acts = h("div", { class: "actions" },
    h("button", { type: "button", class: "btn small", onclick: () => { hidePop(); setTab("nouns"); focusNoun(i, 0); } }, h("span", { "data-icon": "search" }), `${n.count} 處`));
  const ctx = await nounContext(n);
  acts.append(h("button", { type: "button", class: "btn small", onclick: () => translate(n.text, ctx.context, null, { p: { sha: S.sha, page: ctx.page }, context: ctx.context }) }, h("span", { "data-icon": "languages" }), "翻譯"));
  if (!n.kb) {
    acts.append(h("button", { type: "button", class: "btn small", onclick: () => ignoreNoun(n) }, h("span", { "data-icon": "eyeOff" }), "忽略"));
    acts.append(h("button", { type: "button", class: "btn small", onclick: () => nounCard(n) }, h("span", { "data-icon": "cards" }), "加入知識庫…"));
    showPop(anchor, head, meta, acts);
    return;
  }
  acts.append(h("button", { type: "button", class: "btn small", onclick: () => openKbCard(n.kb.node_id) }, h("span", { "data-icon": "external" }), "開啟卡片"));
  showPop(anchor, head, meta, h("div", { class: "muted" }, "…"));
  let summary = "";
  try { const v = await api(`/api/kb/nodes/${n.kb.node_id}`); summary = v.node?.summary ?? ""; } catch (err) { summary = err.message; }
  showPop(anchor, head, meta, h("div", { class: "out" }, summary || "（沒有摘要）"), acts);
}

/** Right-click on a coloured term (no selection): the term's own menu. */
function nounMenu(n, x, y) {
  const i = S.nouns.entries.indexOf(n);
  LoreShell.menu({ x, y }, [
    { head: n.text },
    { icon: "search", label: `${n.count} 處`, run: () => { setTab("nouns"); focusNoun(i, 0); } },
    ...(n.kb ? [] : [{ icon: "eyeOff", label: "忽略", run: () => ignoreNoun(n) }, { icon: "cards", label: "加入知識庫…", run: () => nounCard(n) }]),
  ]);
}

const NOUN_COLS = [["text", "詞"], ["category", "分類"], ["count", "次數"], ["page", "頁"]];
const STATE_ORDER = { new: 0, learning: 1, stale: 1, none: 1, known: 2 };

function nounsPane() {
  const n = S.nouns;
  const out = [];
  if (!n) {
    out.push(h("p", { class: "empty" }, S.readStatus || "讀取全文…"));
    return out;
  }
  if (!n.field_set && !S.fieldSkipped) out.push(fieldAsk());
  const v = S.nounView;
  const cls = n.classification;
  const count = (f) => n.entries.filter(f).length;
  const stat = (key, label, dot, num) => h("button", { type: "button", class: `nn-stat${v.state === key ? " on" : ""}`, "aria-pressed": v.state === key ? "true" : "false", onclick: () => { v.state = v.state === key ? "" : key; renderSide(); } }, h("i", { class: `nn-dot ${dot}` }), label, " ", h("b", {}, num));
  out.push(h("div", { class: "nn-stats" },
    stat("red", "要學", "kt-new", count((e) => e.red)),
    stat("learning", "學習中", "kt-learn", count((e) => ["learning", "stale", "none"].includes(e.state))),
    stat("known", "記得", "kt-known", count(isKnown)),
    S.classifying ? h("span", { class: "muted" }, "分類中…") : cls.unclassified ? h("button", { type: "button", class: "btn ghost small", title: "整篇送一次模型", onclick: () => classifyNouns(true) }, `分類（${cls.unclassified}）`) : null,
    engineChip("lr-classify-terms")));
  if (S.classifyError) out.push(h("div", { class: "note-line" }, S.classifyError));
  if (n.kb && !n.kb.ok) out.push(kbNote());
  const catSel = h("select", { "aria-label": "分類", onchange: (e) => { v.cat = e.target.value; renderSide(); } },
    h("option", { value: "" }, "全部分類"), ...n.categories.map((c) => h("option", { value: c }, c)), h("option", { value: "-" }, "未分類"));
  catSel.value = v.cat;
  const stSel = h("select", { "aria-label": "知識庫狀態", onchange: (e) => { v.state = e.target.value; renderSide(); } },
    h("option", { value: "" }, "全部狀態"), h("option", { value: "red" }, "要學"), h("option", { value: "new" }, "沒卡"), h("option", { value: "learning" }, "學習中"), h("option", { value: "known" }, "記得"), h("option", { value: "used" }, "用過"));
  stSel.value = v.state;
  const q = h("input", { type: "search", placeholder: "篩選", value: v.q, "aria-label": "篩選詞", oninput: (e) => { v.q = e.target.value; clearTimeout(S.nqTimer); S.nqTimer = setTimeout(() => { renderSide(); $(".nn-tools input")?.focus(); }, 200); } });
  out.push(h("div", { class: "nn-tools" }, catSel, stSel, q));
  let rows = n.entries.map((e, i) => ({ e, i }));
  if (v.cat) rows = rows.filter(({ e }) => (v.cat === "-" ? !e.category && !e.kb : e.category === v.cat));
  if (v.state) rows = rows.filter(({ e }) => (v.state === "red" ? e.red : v.state === "used" ? e.used : v.state === "known" ? isKnown(e) : v.state === "new" ? e.state === "new" && !e.used : v.state === "learning" ? ["learning", "stale", "none"].includes(e.state) : e.state === v.state));
  if (v.q.trim()) { const qq = v.q.trim().toLowerCase(); rows = rows.filter(({ e }) => e.text.toLowerCase().includes(qq) || e.key.includes(qq)); }
  const catIdx = (e) => (e.category ? n.categories.indexOf(e.category) : e.kb ? -1 : 99);
  const key = { text: (e) => e.text.toLowerCase(), category: catIdx, state: (e) => STATE_ORDER[e.state] ?? 0, count: (e) => -e.count, page: (e) => e.first_page * 1e6 + e.occ[0][1] }[v.sort];
  rows.sort((a, b) => { const x = key(a.e), y = key(b.e); return (x < y ? -1 : x > y ? 1 : 0) * v.dir || a.i - b.i; });
  const head = h("div", { class: "nn-row nn-head", role: "row" }, NOUN_COLS.map(([k, label]) => h("button", { type: "button", class: v.sort === k ? "on" : "", "aria-sort": v.sort === k ? (v.dir > 0 ? "ascending" : "descending") : "none", onclick: () => { if (v.sort === k) v.dir = -v.dir; else { v.sort = k; v.dir = 1; } renderSide(); } }, label, v.sort === k ? (v.dir > 0 ? " ↑" : " ↓") : "")), h("span"));
  // 基礎科學詞 are folded into one group at the end (unless asked for by the filter)
  const BASIC = "基礎科學詞";
  const basicRows = v.cat === BASIC ? [] : rows.filter(({ e }) => e.category === BASIC);
  if (basicRows.length) rows = rows.filter(({ e }) => e.category !== BASIC);
  const nounRow = ({ e, i }) => {
    const on = S.focusNoun?.i === i;
    const row = h("div", { class: `nn-row${on ? " on" : ""}`, role: "row", tabindex: "0", title: `${e.text}${e.short ? `（${e.short}）` : ""}`,
      onclick: (ev) => { if (!ev.target.closest("button")) (on ? focusNoun(i, S.focusNoun.k + 1) : focusNoun(i, 0)); },
      oncontextmenu: (ev) => { ev.preventDefault(); nounMenu(e, ev.clientX, ev.clientY); },
      onkeydown: (ev) => { if (ev.key === "Enter") focusNoun(i, on ? S.focusNoun.k + 1 : 0); } },
      h("span", { class: "nn-term", title: e.used ? "記得（我的論文用過）" : nounState(e) }, h("i", { class: `nn-dot ${nounClass(e) || "kt-none"}` }), h("span", { class: "ellipsis" }, e.text), e.used ? h("span", { class: "nn-src" }, "用過") : null),
      h("span", { class: "nn-cat ellipsis" }, on ? `${S.focusNoun.k + 1}/${e.count}` : e.category ?? (e.kb ? "—" : "未分類")),
      h("span", { class: "nn-num" }, e.count),
      h("span", { class: "nn-num" }, e.first_page),
      h("span", { class: "nn-acts" },
        on ? h("button", { class: "ico small", "aria-label": "上一個出現位置", "data-icon": "up", onclick: () => focusNoun(i, S.focusNoun.k - 1) }) : null,
        on ? h("button", { class: "ico small", "aria-label": "下一個出現位置", "data-icon": "chevron", onclick: () => focusNoun(i, S.focusNoun.k + 1) }) : null,
        !on && !e.kb ? h("button", { class: "ico small", "aria-label": "忽略", "data-icon": "eyeOff", onclick: () => ignoreNoun(e) }) : null,
        !on && !e.kb ? h("button", { class: "ico small", "aria-label": "加入知識庫…", "data-icon": "cards", onclick: () => nounCard(e) }) : null,
        on ? h("button", { class: "ico small", "aria-label": "取消標示", "data-icon": "x", onclick: clearNounFocus }) : null));
    return row;
  };
  const list = rows.map(nounRow);
  if (basicRows.length) {
    list.push(h("button", { type: "button", class: "nn-group", "aria-expanded": S.basicOpen ? "true" : "false", onclick: () => { S.basicOpen = !S.basicOpen; renderSide(); } }, h("span", { "data-icon": S.basicOpen ? "up" : "chevron" }), `${BASIC} ${basicRows.length}`));
    if (S.basicOpen) list.push(...basicRows.map(nounRow));
  }
  out.push(h("div", { class: "nn-table", role: "table", "aria-label": "名詞清單" }, head, ...(list.length ? list : [h("p", { class: "empty" }, "沒有符合的詞。")])));
  return out;
}

// ── paragraphs (track B) ──────────────────────────────────────────────────
async function loadParagraphs(sha) {
  clearTimeout(S.paraTimer);
  try {
    const r = await api(`/api/papers/${sha}/paragraphs`);
    if (S.sha !== sha) return;
    S.paras = r;
    renderSide("paras");
    if (r.status?.running) S.paraTimer = setTimeout(() => loadParagraphs(sha), 2000);
  } catch (e) { if (S.sha === sha) console.error(e); }
}

async function setUnderstood(p, on) {
  try {
    await api(`/api/papers/${S.sha}/paragraphs/${p.id}/understood`, { body: { on } });
    p.understood = on;
    S.paras.progress.understood += on ? 1 : -1;
    renderSide();
  } catch (e) { fail(e); }
}

function jumpPara(pid) {
  const p = paraOf(pid);
  if (!p) return;
  const r = p.ranges[0];
  if (r.bbox[3] > 0) flashBox(r.page, r.bbox);
  else jumpTo(r.page);
}

/** The next paragraph not marked 懂了 after the one in view (wraps). */
function nextNotUnderstood() {
  const ps = S.paras?.paragraphs.filter((p) => p.section_kind !== "front") ?? [];
  if (!ps.length) return;
  const vpEl = $("#viewport");
  const curPage = S.pages[S.cur]?.page ?? 1;
  const start = Math.max(0, ps.findIndex((p) => p.page >= curPage && p.id !== S.curPara));
  const order = [...ps.slice(start), ...ps.slice(0, start)];
  const n = order.find((p) => !p.understood && p.id !== S.curPara) ?? order.find((p) => !p.understood);
  if (!n) { toast("每一段都標了「懂了」"); return; }
  S.curPara = n.id;
  renderSide("paras");
  jumpPara(n.id);
  setTimeout(() => scrollWithin($("#side"), $(`.para[data-id="${n.id}"]`), "nearest"), 60);
  void vpEl;
}

function chipLabel(kind, id) {
  if (kind === "term") return S.nouns?.entries.find((e) => e.key === id)?.text ?? id;
  if (kind === "symbol") return S.symbols?.entries.find((e) => e.id === id)?.latex ?? id;
  return q_label(id);
}

function chipMenu(p, kind, id, ev) {
  const label = chipLabel(kind, id);
  const btn = ev.currentTarget;
  const box = btn.getBoundingClientRect();
  const anchor = { left: box.left, top: box.top, bottom: box.bottom };
  LoreShell.menu(btn, [
    { head: kind === "symbol" ? tex(label) : label },
    { icon: "eye", label: "看定義", run: () => showChipDef(kind, id, anchor) },
    { icon: "chat", label: "問這篇", run: () => askClaude(p.id, { kind, id, label }) },
    ...(kind === "equation" ? [] : [{ icon: "cards", label: "加入知識庫…", run: () => addCardDialog({ title: kind === "symbol" ? "" : label, sha: S.sha, page: p.page, context: p.text.slice(0, 600) }) }]),
  ]);
}

function showChipDef(kind, id, anchor) {
  if (kind === "term") {
    const n = S.nouns?.entries.find((e) => e.key === id);
    if (n) nounPopover(n, anchor);
  } else if (kind === "symbol") {
    const i = S.symbols?.entries.findIndex((e) => e.id === id);
    const e = S.symbols?.entries[i];
    if (e) symPopover({ page: e.occurrences?.[0]?.page ?? 1, latex: e.latex, ents: [i] }, anchor, true);
  } else {
    const q = S.symbols?.equations.find((x) => x.id === id);
    if (q) flashBox(q.page, q.bbox);
  }
}

function analysisBlock(a) {
  if (!a) return null;
  const out = h("div", { class: "para-an" });
  if (a.equations?.length) out.append(...a.equations.map((q) => h("dl", { class: "eqnote" },
    h("dt", { class: "eqref" }, q.ref ? mixed(q.ref) : "式"),
    h("dd", {}, h("span", { class: "k" }, "輸入"), mixed(q.inputs || "—")),
    h("dd", {}, h("span", { class: "k" }, "輸出"), mixed(q.outputs || "—")),
    h("dd", {}, h("span", { class: "k" }, "對象"), mixed(q.target || "—")),
    h("dd", {}, h("span", { class: "k" }, "效果"), mixed(q.effect || "—")))));
  if (a.motive) out.append(h("div", { class: "motive" }, h("span", { class: "k" }, "動機"), mixed(a.motive)));
  if (a.results?.length) out.append(...a.results.map((r) => h("dl", { class: "resrow" },
    ...[["指標", r.metric], ["比較對象", r.comparison], ["數值", r.values], ["差距", r.difference], ["檢定", r.test], ["顯著性", r.significance]].map(([k, v]) => h("dd", {}, h("span", { class: "k" }, k), mixed(v || "—"))))));
  return out.childElementCount ? out : null;
}

function parasPane() {
  const d = S.paras;
  if (!d) return [h("p", { class: "empty" }, S.readStatus || "…")];
  const st = d.status ?? {};
  const pr = d.progress;
  S.paraOpen ??= new Set();
  S.chipOpen ??= new Set();
  const out = [];
  out.push(h("div", { class: "para-top" },
    h("div", { class: "progress", role: "progressbar", "aria-valuenow": pr.understood, "aria-valuemax": pr.total, title: `懂了 ${pr.understood}/${pr.total}` }, h("i", { style: `width:${pr.total ? (pr.understood / pr.total) * 100 : 0}%` })),
    h("button", { type: "button", class: "btn small", onclick: nextNotUnderstood }, h("span", { "data-icon": "next" }), "下一段未懂"),
    engineChip("lr-paragraph")));
  if (st.running) out.push(h("div", { class: "muted para-status" }, `分析中 ${st.done}/${st.total}`));
  if (st.error) out.push(h("div", { class: "warn-box" }, st.error, " ", h("button", { type: "button", class: "btn ghost small", onclick: async () => { try { S.paras = await api(`/api/papers/${S.sha}/paragraphs/run`, { body: {} }); renderSide(); loadParagraphs(S.sha); } catch (e) { fail(e); } } }, "重試")));
  // only what can block understanding: terms to learn, symbols not settled, equations
  const blocking = (kind, id) => {
    if (kind === "term") return S.nouns?.entries.find((e) => e.key === id)?.red !== false;
    if (kind === "symbol") return ["unresolved", "inferred", "conflict"].includes(S.symbols?.entries.find((x) => x.id === id)?.grade ?? "unresolved");
    return true;
  };
  let section = null;
  for (const p of d.paragraphs) {
    if (p.section_kind === "front") continue;
    if (p.section !== section) { section = p.section; out.push(h("h4", { class: "para-sec" }, section || SECTION_KIND[p.section_kind] || "")); }
    const a = p.analysis;
    const role = a ? ROLE[a.role] ?? a.role : st.running ? "…" : "—";
    if (p.understood && !S.paraOpen.has(p.id)) {
      out.push(h("button", { type: "button", class: `para one${S.curPara === p.id ? " cur" : ""}`, dataset: { id: p.id }, title: p.text.slice(0, 120), onclick: () => { S.paraOpen.add(p.id); renderSide(); } },
        h("span", { class: "role" }, role), h("span", { class: "src" }, `p.${p.page}`), h("span", { class: "para-one-text" }, p.text), h("span", { class: "ok-mark" }, "✓")));
      continue;
    }
    const chips = [
      ...p.chips.terms.filter((id) => blocking("term", id)).map((id) => h("button", { type: "button", class: "chip-b", onclick: (ev) => chipMenu(p, "term", id, ev) }, chipLabel("term", id))),
      ...p.chips.symbols.filter((id) => blocking("symbol", id)).map((id) => { const e = S.symbols?.entries.find((x) => x.id === id); return h("button", { type: "button", class: "chip-b", title: e ? GRADE[e.grade]?.[1] : "", onclick: (ev) => chipMenu(p, "symbol", id, ev) }, tex(e?.latex ?? id), e?.grade === "conflict" ? h("span", { class: "gi" }, GRADE.conflict[0]) : null); }),
      ...p.chips.equations.map((id) => h("button", { type: "button", class: "chip-b", onclick: (ev) => chipMenu(p, "equation", id, ev) }, q_label(id))),
    ];
    const open = S.chipOpen.has(p.id);
    const chipBox = chips.length ? h("div", { class: `para-chips${open ? "" : " folded"}` }, ...chips) : null;
    const more = chips.length ? h("button", { type: "button", class: "chip-more", hidden: !open, onclick: () => { open ? S.chipOpen.delete(p.id) : S.chipOpen.add(p.id); renderSide(); } }, open ? "收起" : "") : null;
    out.push(h("div", { class: `para${p.understood ? " done" : ""}${S.curPara === p.id ? " cur" : ""}`, dataset: { id: p.id } },
      h("div", { class: "para-head" }, h("span", { class: "role" }, role), h("span", { class: "src" }, `p.${p.page}`),
        p.understood ? h("button", { type: "button", class: "ico small ok-btn", "aria-label": "收起", onclick: () => { S.paraOpen.delete(p.id); renderSide(); } }, "✓") : null),
      h("button", { type: "button", class: "para-text", title: "跳到這段", onclick: () => { S.curPara = p.id; renderSide(); jumpPara(p.id); } }, p.text),
      chipBox, more,
      analysisBlock(a),
      h("div", { class: "para-acts" },
        h("button", { type: "button", class: "btn small", onclick: () => askClaude(p.id, null) }, h("span", { "data-icon": "chat" }), "這段我卡住了"),
        p.understood
          ? h("button", { type: "button", class: "btn small ghost-muted", onclick: () => setUnderstood(p, false) }, "取消懂了")
          : h("button", { type: "button", class: "btn small", onclick: () => setUnderstood(p, true) }, h("span", { "data-icon": "check" }), "懂了"))));
  }
  requestAnimationFrame(foldChips);
  return out;
}

/** Chips beyond two lines are folded behind "＋N": the overflow chips get `hidden`
 *  (not clipped), rows are measured with getBoundingClientRect relative to the box.
 *  Runs again when the drawer opens or the window resizes (a hidden panel has no size). */
function foldChips() {
  for (const box of $$(".para-chips.folded")) {
    const more = box.nextElementSibling;
    if (!more?.classList.contains("chip-more")) continue;
    const chips = [...box.children];
    chips.forEach((c) => { c.hidden = false; });
    if (!box.getBoundingClientRect().width) continue;
    const top0 = box.getBoundingClientRect().top;
    const rows = [];
    for (const c of chips) {
      const t = Math.round(c.getBoundingClientRect().top - top0);
      if (!rows.some((r) => Math.abs(r - t) < 4)) rows.push(t);
    }
    rows.sort((x, y) => x - y);
    const limit = rows[2];
    let n = 0;
    if (limit != null) for (const c of chips) if (c.getBoundingClientRect().top - top0 >= limit - 3) { c.hidden = true; n++; }
    more.hidden = n === 0;
    more.textContent = `＋${n}`;
  }
}

function defaultLabel(ref) {
  const [kind, name] = String(ref).split(":");
  if (kind === "local") return "地端";
  return MODEL_LABEL[name] ?? "Claude";
}

// ── model menu ────────────────────────────────────────────────────────────
/** Asked once, where the red terms are: the field decides what counts as basic. */
function fieldAsk() {
  const inp = h("input", { placeholder: "例如：視網膜影像分析", "aria-label": "你的領域", maxlength: 200 });
  const save = async () => {
    const f = inp.value.trim();
    if (!f) return;
    try {
      await api("/api/reader", { method: "PUT", body: { field: f } });
      await refreshNouns();
      if (S.nouns?.classification.pending > 0) classifyNouns(false);
    } catch (e) { fail(e); }
  };
  return h("form", { class: "nn-field", onsubmit: (e) => { e.preventDefault(); save(); } },
    h("label", {}, "你的領域", inp),
    h("button", { class: "btn small" }, "確定"),
    h("button", { type: "button", class: "btn small ghost", onclick: () => { try { localStorage.setItem("lr.fieldSkipped", "1"); } catch {} S.fieldSkipped = true; renderSide(); } }, "略過"));
}

/** A feature's own engine (the kit's engine chip): 自動 · 雲 · 地, kept as the task's
 *  override in the shared engine settings (`PUT /api/engines` overrides). An older
 *  models.json choice (the former 模型 dialog) still wins until the chip is changed; it
 *  shows as 地 or 雲 (with the Claude model's name), and changing the chip clears it. */
function engineChip(task) {
  const legacy = S.models?.tasks.find((x) => x.id === task)?.choice ?? "auto";
  const d = S.engines;
  const t = d?.tasks?.find((x) => x.id === task) ?? S.models?.tasks.find((x) => x.id === task);
  const value = legacy !== "auto" ? (legacy === "local" ? "local" : "cloud") : d?.overrides?.[task] ?? "auto";
  const used = ["haiku", "sonnet", "opus"].includes(legacy) ? MODEL_LABEL[legacy] : engineName(S.used[task] ?? d?.tasks?.find((x) => x.id === task)?.last);
  return LoreShell.engineChip({ task, value, used, label: `${t?.label ?? "模型"}：模型`, onChange: async (v) => {
    if (!d) { S.models = await api("/api/models", { method: "PUT", body: { task, model: v } }); return; }
    await LoreShell.engines.put({ overrides: { [task]: v } });
    if (legacy !== "auto") S.models = await api("/api/models", { method: "PUT", body: { task, model: "auto" } });
  } });
}
/** "claude:sonnet" → "sonnet", "local:qwen3.5-9b" → "qwen3.5-9b". */
const engineName = (id) => (id ? String(id).replace(/^(claude|local)[:/-]/, "") : "");

// ── search: the ⌘F bar over the pages ─────────────────────────────────────
let searchTimer;
function openFind() {
  if (!S.sha) return;
  $("#find").hidden = false;
  $("#searchInput").focus();
  $("#searchInput").select();
}
function closeFind() {
  $("#find").hidden = true;
  if (S.search.q) { S.search = { q: "", hits: [], i: -1 }; $("#searchInput").value = ""; $("#searchCount").textContent = ""; S.pages.filter((p) => p.rendered).forEach(decorate); }
}
async function runSearch(q) {
  S.search = { q, hits: [], i: -1 };
  const needle = q.trim().toLowerCase();
  if (needle.length >= 2) {
    for (let i = 0; i < S.pages.length; i++) {
      const p = S.pages[i];
      const { text } = await pageText(p.sha, p.page);
      if (S.search.q !== q) return;
      const hay = text.toLowerCase();
      for (let at = hay.indexOf(needle); at >= 0; at = hay.indexOf(needle, at + needle.length)) S.search.hits.push({ i, start: at, end: at + needle.length });
    }
  }
  $("#searchCount").textContent = needle.length >= 2 ? (S.search.hits.length ? `0/${S.search.hits.length}` : "0") : "";
  S.pages.filter((p) => p.rendered).forEach(decorate);
  if (S.search.hits.length) stepSearch(1);
}
function stepSearch(d) {
  const n = S.search.hits.length;
  if (!n) return;
  const firstFromHere = S.search.i < 0 ? S.search.hits.findIndex((x) => x.i >= S.cur) : -1;
  S.search.i = S.search.i < 0 ? (firstFromHere >= 0 ? firstFromHere : 0) : (S.search.i + d + n) % n;
  const hit = S.search.hits[S.search.i];
  $("#searchCount").textContent = `${S.search.i + 1}/${n}`;
  S.pages.filter((p) => p.rendered).forEach(decorate);
  scrollToPage(hit.i, false);
  setTimeout(() => scrollWithin($("#viewport"), $(".sr-cur")), 300);
}

// ── chat ──────────────────────────────────────────────────────────────────
function attach(info) {
  S.att.push({ text: info.text, page: info.p.page, context: info.context });
  getSelection().removeAllRanges();
  setTab("chat");
  setTimeout(() => $("#chatInput")?.focus(), 50);
}

async function sendChat(model) {
  const ta = $("#chatInput");
  const text = ta.value.trim();
  const ctx = S.chatCtx;
  if (S.sending || (!text && !S.att.length && !ctx)) return;
  S.sending = true;
  const att = S.att; S.att = [];
  const pending = { role: "user", text, attachments: att, at: new Date().toISOString(), pending: true, paragraph: ctx?.paragraph, chip: ctx?.chip };
  S.chat.push(pending);
  S.chatDraft = ""; S.chatCtx = null;
  renderSide();
  try {
    const body = { text, attachments: att };
    if (typeof model === "string") body.model = model;
    if (ctx) { body.paragraph = ctx.paragraph; if (ctx.chip) body.chip = { kind: ctx.chip.kind, id: ctx.chip.id }; }
    const r = await api(`/api/papers/${S.sha}/chat`, { body });
    S.chat.pop();
    await loadChat();
  } catch (e) {
    S.chat.pop(); S.att = att; S.chatDraft = text; S.chatCtx = ctx;
    S.chatError = e.message;
    // lore-llm's one-click alternative (「改用本機」／「改用 Claude」) when the other engine could answer
    S.chatAlt = e.data?.alternative ?? null;
    S.chatLocal = false;
    if (!S.chatAlt && e.data?.llm_unavailable && !model) { try { S.chatLocal = !!(await api("/api/models")).local_engine; } catch {} }
  }
  S.sending = false;
  renderSide();
}

/** Ask Claude about a paragraph (and one stuck point in it): the server attaches title,
 *  section, the paragraph, the point's known definition and the author's KB states. */
function askClaude(pid, chip) {
  S.curPara = pid;
  S.chatCtx = { paragraph: pid, chip: chip ?? null };
  S.chatDraft = S.paras?.default_question ?? "只解釋我卡住的點，不要翻譯整段";
  S.chatScope = "para";
  setTab("chat");
  setTimeout(() => { const t = $("#chatInput"); t?.focus(); t?.select(); }, 60);
}

// ── side panel: the kit's numbered blocks ────────────────────────────────
// 01 名詞 and 02 註記 open by default; 符號, 段落, 引用句 and 問這篇 start folded (the
// AI button opens 問這篇). A block's body is built only while it is open.
const BLOCKS = [
  ["nouns", "名詞", true, "紅：要學 · 黃：學習中 · 綠：記得"],
  ["notes", "註記", true],
  ["symbols", "符號", false],
  ["paras", "段落", false],
  ["quotes", "引用句", false],
  ["chat", "問這篇", false],
];
const blocks = {};
function buildBlocks() {
  BLOCKS.forEach(([id, title, open, help], i) => {
    const b = LoreShell.block({ n: String(i + 1).padStart(2, "0"), title, open, help, key: `lr.${id}` });
    b.dataset.block = id;
    const set = b.setOpen;
    b.setOpen = (on) => { set(on); if (on) { renderSide(id); paintIcons(b); } };
    blocks[id] = b;
  });
  $("#side").replaceChildren(...Object.values(blocks));
}
const isOpen = (id) => blocks[id]?.dataset.open === "true";

/** Open a block (and the right drawer on narrow windows) and bring it into view. */
function setTab(t) {
  const b = blocks[t];
  if (!b) return;
  if (!isOpen(t)) b.setOpen(true);
  if (innerWidth <= 1100) { shell.drawer("right", true); requestAnimationFrame(foldChips); }
  const side = $("#side");
  requestAnimationFrame(() => side.scrollTo({ top: Math.max(0, b.offsetTop - 8), behavior: "smooth" }));
}

function renderTabCounts() {
  const c = {
    nouns: S.nouns?.entries.filter((e) => e.red).length || "",
    notes: S.annotations.length || "",
    symbols: S.symbols?.entries?.length || "",
    paras: S.paras ? `${S.paras.progress.understood}/${S.paras.progress.total}` : "",
    quotes: S.quotes.length || "",
    chat: S.chat.length || "",
  };
  for (const [id, b] of Object.entries(blocks)) b.setCount(c[id]);
}

/** Build the open blocks' bodies (`which` = one block only). */
function renderSide(which) {
  renderTabCounts();
  for (const id of which ? [which] : Object.keys(blocks)) {
    if (!isOpen(id)) continue;
    const body = blocks[id].body;
    if (id === "nouns") {
      const keep = $(".nn-table", body)?.scrollTop;
      body.replaceChildren(...nounsPane());
      if (keep != null && $(".nn-table", body)) $(".nn-table", body).scrollTop = keep;
    } else if (id === "paras") body.replaceChildren(...parasPane());
    else if (id === "symbols") body.replaceChildren(...symbolsPane());
    else if (id === "notes") body.replaceChildren(...notesPane());
    else if (id === "quotes") body.replaceChildren(...quotesPane());
    else {
      body.replaceChildren(...chatPane(), composer());
      const m = $(".msgs", body);
      if (m) m.scrollTop = m.scrollHeight;
    }
    body.classList.add(`b-${id}`);
    paintIcons(body);
  }
}

function notesPane() {
  if (!S.annotations.length) return [h("p", { class: "empty" }, "選取文字後按 1–4（螢光）或 N（註記）")];
  const sorted = [...S.annotations].sort((a, b) => a.page - b.page || (a.rects?.[0]?.[1] ?? 0) - (b.rects?.[0]?.[1] ?? 0));
  return [h("ul", { class: "items" }, sorted.map((a) => h("li", { class: "item", onclick: (e) => { if (!e.target.closest("button")) jumpTo(a.page, a.id); } },
    h("span", { class: `dot c-${a.color}` }),
    h("span", { class: "body" }, h("span", { class: "txt" }, a.quote?.exact || "（無文字）"), a.note ? h("span", { class: "note" }, a.note) : null, h("span", { class: "meta" }, `p.${a.page} · ${a.created_at.slice(5, 10)}`)),
    h("span", { class: "acts" },
      h("button", { class: "ico small", "aria-label": "編輯註記", "data-icon": "pen", onclick: () => editNote(a) }),
      h("button", { class: "ico small", "aria-label": "刪除", "data-icon": "trash", onclick: () => delAnnotation(a) })))))];
}

async function editNote(a) {
  const ta = h("textarea", {}, a.note ?? "");
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); close(ta.value); } },
    h("h3", {}, `註記 · 第 ${a.page} 頁`), h("div", { class: "quote-box" }, a.quote?.exact ?? ""), h("label", {}, "註記", ta),
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"), h("button", { class: "btn primary" }, "儲存"))));
  if (r == null) return;
  try {
    const out = await api(`/api/papers/${S.sha}/annotations/${a.id}`, { method: "PUT", body: { note: r } });
    Object.assign(a, out.annotation);
    renderSide();
    refreshExport();
    S.pages.filter((p) => p.rendered && p.page === a.page).forEach(drawHighlights);
  } catch (e) { fail(e); }
}

async function delAnnotation(a) {
  try {
    await api(`/api/papers/${S.sha}/annotations/${a.id}`, { method: "DELETE" });
    S.annotations = S.annotations.filter((x) => x.id !== a.id);
    renderSide();
    refreshExport();
    S.pages.filter((p) => p.rendered && p.page === a.page).forEach(drawHighlights);
  } catch (e) { fail(e); }
}

function quotesPane() {
  if (!S.quotes.length) return [h("p", { class: "empty" }, "選取文字後按 Q")];
  return [h("ul", { class: "items" }, S.quotes.map((q) => h("li", { class: "item", onclick: (e) => { if (!e.target.closest("button")) jumpTo(q.page); } },
    h("span", { class: "dot q" }),
    h("span", { class: "body" }, h("span", { class: "txt" }, `“${q.text}”`), q.note ? h("span", { class: "note" }, q.note) : null, h("span", { class: "meta" }, `p.${q.page} · ${q.created_at.slice(5, 10)}`)),
    h("span", { class: "acts" },
      h("button", { class: "ico small", "aria-label": "複製", "data-icon": "copy", onclick: () => navigator.clipboard.writeText(`“${q.text}” (p. ${q.page})`).then(() => toast("已複製")) }),
      h("button", { class: "ico small", "aria-label": "刪除", "data-icon": "trash", onclick: async () => { try { await api(`/api/papers/${S.sha}/quotes/${q.id}`, { method: "DELETE" }); await loadQuotes(); renderSide("quotes"); } catch (e) { fail(e); } } })))))];
}

/** A chip's label: symbols as KaTeX (stored as `$…$` or bare LaTeX), the rest as text. */
function chipText(c) {
  const l = c.label ?? c.id;
  if (c.kind !== "symbol") return mixed(l);
  return l.startsWith("$") ? mixed(l) : tex(l);
}

function paraOf(pid) {
  return S.paras?.paragraphs.find((p) => p.id === pid);
}

function chatPane() {
  const cur = paraOf(S.curPara);
  const scope = h("div", { class: "seg chat-scope", role: "group", "aria-label": "顯示哪些對話" },
    h("button", { type: "button", "aria-pressed": S.chatScope !== "para" ? "true" : "false", onclick: () => { S.chatScope = "all"; renderSide(); } }, "全部"),
    h("button", { type: "button", "aria-pressed": S.chatScope === "para" ? "true" : "false", disabled: !cur, title: cur ? `p.${cur.page} ${cur.text.slice(0, 40)}…` : "先在「段落」選一段", onclick: () => { S.chatScope = "para"; renderSide(); } }, cur ? `這段（p.${cur.page}）` : "這段"));
  const head = h("div", { class: "pane-head" }, scope, h("span", { style: "flex:1" }),
    S.chat.length ? h("button", { class: "ico small", "aria-label": "清除全部對話", "data-icon": "trash", onclick: async () => { try { await api(`/api/papers/${S.sha}/chat`, { method: "DELETE" }); S.chat = []; renderSide(); } catch (e) { fail(e); } } }) : null);
  const shown = S.chatScope === "para" && cur ? S.chat.filter((m) => m.paragraph === cur.id) : S.chat;
  const lastEngine = [...S.chat].reverse().find((m) => m.role === "assistant" && m.engine)?.engine;
  if (lastEngine) S.used["lr-chat"] = lastEngine;
  const msgs = shown.map((m) => h("div", { class: `msg ${m.role}` },
    m.role === "user" && m.paragraph ? h("button", { type: "button", class: "para-badge", onclick: () => { S.curPara = m.paragraph; jumpPara(m.paragraph); } }, `p.${paraOf(m.paragraph)?.page ?? "?"}`, m.chip ? [" · ", chipText(m.chip)] : " · 整段") : null,
    ...(m.attachments ?? []).map((a) => h("span", { class: "att", title: a.context }, `p.${a.page}「${a.text}」`)),
    m.role === "assistant" ? mixed(m.text) : m.text,
    m.role === "assistant" && (m.engine || m.paragraph) ? h("div", { class: "eng-row" }, h("span", { class: "eng" }, m.engine ?? ""), m.paragraph ? answerActions(m) : null) : null,
    m.pending ? h("span", { class: "eng" }, "思考中…") : null));
  const alt = S.chatAlt ? h("button", { type: "button", class: "btn small", onclick: () => sendChat(S.chatAlt.engine === "local" ? "local" : "cloud") }, S.chatAlt.label)
    : S.chatLocal ? h("button", { type: "button", class: "btn small", onclick: () => sendChat("local") }, "改用本機") : null;
  const err = S.chatError ? h("div", { class: "err-box" }, S.chatError, alt) : null;
  S.chatError = null;
  return [head, h("div", { class: "msgs" }, msgs.length ? msgs : [h("p", { class: "empty" }, S.chatScope === "para" ? "這段還沒有對話" : "選取文字後按 A")]), err].filter(Boolean);
}

/** Under an answer about a paragraph: write it back to the symbol table, keep it as
 *  this paragraph's note, or add the term to the KB. */
function answerActions(m) {
  const p = paraOf(m.paragraph);
  const syms = p?.chips?.symbols ?? [];
  return h("span", { class: "ans-acts" },
    syms.length ? h("button", { type: "button", class: "ico small", "aria-label": "存進符號表…", "data-icon": "sigma", onclick: () => writeBackDialog(m, p) }) : null,
    h("button", { type: "button", class: "ico small", "aria-label": "存成註記", "data-icon": "pen", onclick: () => saveParaNote(m, p) }),
    h("button", { type: "button", class: "ico small", "aria-label": "加入知識庫…", "data-icon": "cards", onclick: () => addCardDialog({ title: m.chip?.kind === "term" ? (m.chip.label ?? m.chip.id) : "", sha: S.sha, page: p?.page ?? 1, context: p?.text?.slice(0, 600), summary: m.text.slice(0, 300) }) }));
}

async function writeBackDialog(m, p) {
  const ents = (p.chips.symbols ?? []).map((id) => S.symbols?.entries.find((e) => e.id === id)).filter(Boolean);
  if (!ents.length) return;
  let pick = ents.find((e) => m.chip?.kind === "symbol" && e.id === m.chip.id) ?? ents[0];
  const sel = h("select", { onchange: (ev) => { pick = ents[+ev.target.value]; zh.value = pick.meaning ?? ""; } }, ents.map((e, i) => h("option", { value: i, selected: e === pick }, `${e.latex}（${GRADE[e.grade]?.[0] ?? ""}）`)));
  const firstLine = m.text.split(/\n|。/)[0].slice(0, 120);
  const zh = h("textarea", {}, pick.meaning || firstLine);
  let grade = pick.grade === "defined" || pick.grade === "certain" ? pick.grade : "inferred";
  const seg = h("div", { class: "seg", role: "group", "aria-label": "等級" });
  const paintSeg = () => seg.replaceChildren(...["certain", "defined", "inferred"].map((g) => h("button", { type: "button", "aria-pressed": g === grade ? "true" : "false", onclick: () => { grade = g; paintSeg(); } }, `${GRADE[g][0]} ${GRADE[g][1]}`)));
  paintSeg();
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (ev) => { ev.preventDefault(); close(true); } },
    h("h3", {}, "存進符號表"),
    h("label", {}, "符號", sel),
    h("label", {}, "意思", zh),
    h("div", { style: "display:flex;flex-direction:column;gap:4px" }, h("span", { style: "font-size:12.5px;color:var(--muted)" }, "等級"), seg),
    h("div", { class: "quote-box" }, h("span", { class: "muted" }, "依據："), m.text.slice(0, 400), m.text.length > 400 ? "…" : ""),
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"), h("button", { class: "btn primary" }, "儲存"))));
  if (r !== true) return;
  try {
    await api(`/api/papers/${S.sha}/symbols/${encodeURIComponent(pick.id)}`, { method: "PUT", body: { meaning: zh.value, grade, by: "claude", evidence: m.text, page: p.page } });
    setSymbols(await api(`/api/papers/${S.sha}/symbols`));
    toast(`已存進符號表：${pick.latex}`);
  } catch (e) { fail(e); }
}

async function saveParaNote(m, p) {
  try {
    const r = await api(`/api/papers/${S.sha}/annotations`, { body: { page: p.page, kind: "note", color: "blue", note: m.text, quote: { exact: p.text.slice(0, 160), prefix: "", suffix: "" }, rects: [], paragraph: p.id, chip: m.chip ?? null } });
    S.annotations.push(r.annotation);
    toast("已存成註記");
  } catch (e) { fail(e); }
}

function composer() {
  const ctxChip = S.chatCtx ? (() => {
    const p = paraOf(S.chatCtx.paragraph);
    return h("span", { class: "attchip ctx" },
      h("span", {}, `p.${p?.page ?? "?"}`, S.chatCtx.chip ? [" · ", chipText(S.chatCtx.chip)] : " · 整段"),
      h("button", { class: "ico small", "aria-label": "不附段落", "data-icon": "x", onclick: () => { S.chatCtx = null; renderSide(); } }));
  })() : null;
  const chips = h("div", { class: "chips" }, ctxChip, S.att.map((a, i) => h("span", { class: "attchip", title: a.context }, h("span", {}, `p.${a.page}「${a.text}」`),
    h("button", { class: "ico small", "aria-label": "移除", "data-icon": "x", onclick: () => { S.att.splice(i, 1); renderSide(); } }))));
  const ta = h("textarea", { id: "chatInput", placeholder: S.att.length || S.chatCtx ? "想問這段什麼？" : "問這篇論文" });
  ta.value = S.chatDraft ?? "";
  ta.addEventListener("input", () => { S.chatDraft = ta.value; });
  ta.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); sendChat(); } });
  return h("div", { class: "composer" }, chips, ta, h("div", { class: "row" }, engineChip("lr-chat"), h("span", { class: "grow" }), h("button", { class: "btn small", disabled: S.sending, title: "⌘↵", onclick: sendChat }, h("span", { "data-icon": "send" }), S.sending ? "等待回覆…" : "送出")));
}

// ── save with notes ───────────────────────────────────────────────────────
// A copy of the paper's PDF with the notes written in (the original never changes).
// Desktop: the native save panel; browser: a download. "儲存" overwrites the last copy.
async function refreshExport() {
  if (!S.sha) return;
  try { S.exp = await api(`/api/papers/${S.sha}/export`); } catch { S.exp = null; }
  updateStatus();
}

/** The shell's status chip: 頁面未儲存 (pending page operations: click = 另存…),
 *  未儲存 (notes not in a PDF yet: click = 另存新檔（含筆記）), 已儲存 after a save. */
function updateStatus() {
  if (!shell) return;
  if (!S.sha) { shell.setStatus(null); return; }
  if (S.arr) shell.setStatus("dirty", { label: "頁面未儲存", title: "另存…", action: () => saveArrangement() });
  else if (S.exp?.dirty) shell.setStatus("dirty", { title: "另存新檔（含筆記）" });
  else if (S.exp?.last?.exists || S.savedHere === S.sha) shell.setStatus("saved", { title: S.exp?.last?.name ?? "" });
  else shell.setStatus(null);
}
/** Where a question about saving appears: the status chip, else the document menu. */
function statusAnchor() {
  const chip = shell?.el.querySelector(".fstate");
  return chip && !chip.hidden ? chip : shell?.el.querySelector(".picker");
}

function exported(r) {
  S.savedHere = S.sha;
  if (r.export) S.exp = r.export;
  toast(`已存：${r.name}`);
  refreshExport();
}

async function saveNotesAs() {
  try {
    if (S.exp?.save_dialog !== false) {
      const r = await api(`/api/papers/${S.sha}/export`, { body: { choose: true } });
      if (r.cancelled) return false;
      if (r.native !== false) { exported(r); return true; }
    }
    const res = await fetch(`/api/papers/${S.sha}/export.pdf`, { method: "POST" });
    if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error ?? `${res.status}`);
    const url = URL.createObjectURL(await res.blob());
    h("a", { href: url, download: S.exp?.default_name ?? "notes.pdf" }).click();
    setTimeout(() => URL.revokeObjectURL(url), 10000);
    S.savedHere = S.sha;
    refreshExport();
    return true;
  } catch (e) { fail(e); return false; }
}

async function saveNotesLast() {
  try { exported(await api(`/api/papers/${S.sha}/export`, { body: { last: true } })); return true; } catch (e) { fail(e); return false; }
}

/** 另存新檔（含筆記）: LoreReader's save (private/ is scratch). With an earlier copy, a
 *  menu at `anchor`: 儲存 (overwrite that copy) · 另存新檔…. */
function saveNotes(anchor, { ask = true } = {}) {
  if (!S.sha) return;
  if (S.arr) { toast("頁面未儲存：先在縮圖欄另存", true); if (innerWidth <= 1100) shell.drawer("left", true); return; }
  const last = S.exp?.last;
  if (!last?.exists || !ask) { saveNotesAs(); return; }
  LoreShell.menu(anchor ?? statusAnchor(), [
    { icon: "save", label: "儲存", kbd: last.name, run: saveNotesLast },
    { icon: "copy", label: "另存新檔…", run: saveNotesAs },
  ]);
}

// ── print ─────────────────────────────────────────────────────────────────
async function print() {
  if (S.arr) { toast("先另存再列印", true); return; }
  try {
    const r = await api("/api/print", { body: { sha: S.sha } });
    if (r.native) { toast("正在準備列印…"); return; }
  } catch (e) { fail(e); return; }
  const w = window.open(`print.html?sha=${S.sha}&auto=1`, "_blank");
  if (!w) location.href = `print.html?sha=${S.sha}&auto=1`;
}

// ── 對照表: this paper's symbols.json in the shared <notation-panel> ──────────
// Document scope: the rows are this paper's symbols and equations. A row the global
// 對照表 (shared/notation.json) already has shows 全域; 加入全域 copies the selected row
// there. Edits of LaTeX / 意思 / 原文定義 / 備註 go to the paper's symbol table;
// 插入 copies the LaTeX; 新增 opens 新增符號; rows are not deleted here.
let ntDlg = null, ntPanel = null, ntGlobal = new Set();
const NT_FIELDS = { latex: "latex", meaning: "meaning", meaning_en: "meaning_en", note: "note" };
const texKey = (t) => String(t ?? "").replace(/\s+/g, "");

function notationAssets() {
  if (customElements.get("notation-panel")) return Promise.resolve();
  return new Promise((ok, no) => {
    const sc = h("script", { src: "/notation-panel.js" });
    sc.onload = ok;
    sc.onerror = () => no(new Error("對照表元件載入失敗"));
    document.head.append(sc);
  });
}

function notationView() {
  const f = S.symbols ?? {};
  const origin = (e) => (e.latex && ntGlobal.has(texKey(e.latex)) ? "global" : "document");
  const sym = (f.entries ?? []).map((e) => ({ ...e, kind: e.kind ?? "symbol", origin: origin(e) }));
  const eqs = (f.equations ?? []).map((e) => ({ ...e, kind: "equation", origin: origin(e), readonly: true }));
  return { version: 1, entries: [...sym, ...eqs] };
}

function buildNotation() {
  ntPanel = document.createElement("notation-panel");
  ntPanel.setAttribute("mode", "literal");
  const close = h("button", { type: "button", class: "ico close", "data-icon": "x", "aria-label": "關閉 Esc" });
  ntDlg = h("dialog", { class: "dialog lr-notation", "aria-label": "對照表" },
    h("div", { class: "dlg" }, h("div", { class: "dlg-head" }, h("h2", {}, "對照表"), h("span", { class: "grow" }), close), ntPanel));
  document.body.append(ntDlg);
  paintIcons(ntDlg);
  close.onclick = () => ntDlg.close();
  ntPanel.actions = [{
    id: "global", label: "加入全域",
    show: (e) => e.kind !== "equation" && !!e.latex && !ntGlobal.has(texKey(e.latex)),
    run: (e) => addToGlobal(e.id),
  }];
  ntPanel.addEventListener("notation-insert", (e) => {
    const t = e.detail.entry?.latex ?? e.detail.text ?? "";
    navigator.clipboard.writeText(t).then(() => toast(`已複製 ${t}`), () => toast("無法複製", true));
  });
  ntPanel.addEventListener("notation-change", async (e) => {
    const { id, field, value } = e.detail;
    if (!id) return;
    const sym = S.symbols?.entries.find((x) => x.id === id);
    if (!sym || !NT_FIELDS[field]) { toast(sym ? "這一欄在 LoreReader 不能改" : "式不能在這裡改", true); ntPanel.notation = notationView(); return; }
    try {
      await api(`/api/papers/${S.sha}/symbols/${encodeURIComponent(id)}`, { method: "PUT", body: { [NT_FIELDS[field]]: value ?? "" } });
      setSymbols(await api(`/api/papers/${S.sha}/symbols`));
    } catch (x) { fail(x); ntPanel.notation = notationView(); }
  });
  ntPanel.addEventListener("notation-add", () => { ntPanel.notation = notationView(); symbolDialog(null); });
  ntPanel.addEventListener("notation-delete", () => { ntPanel.notation = notationView(); toast("符號表的列不能刪除", true); });
}

async function openNotation() {
  if (!S.sha) return;
  try { await notationAssets(); } catch (e) { fail(e); return; }
  if (!ntDlg) buildNotation();
  try { ntGlobal = new Set(((await api("/api/notation/global")).entries ?? []).map((e) => texKey(e.latex)).filter(Boolean)); } catch { ntGlobal = new Set(); }
  ntPanel.setAttribute("theme", document.documentElement.dataset.mode === "dark" ? "dark" : "light");
  ntPanel.notation = notationView();
  if (!ntDlg.open) ntDlg.showModal();
}

async function addToGlobal(id) {
  const e = S.symbols?.entries.find((x) => x.id === id);
  if (!e) return;
  try {
    await api(`/api/papers/${S.sha}/symbols/${encodeURIComponent(e.id)}/global`, { body: {} });
    ntGlobal.add(texKey(e.latex));
    ntPanel.notation = notationView();
    toast(`已加入全域：${e.latex}`);
  } catch (x) { fail(x); }
}

// ── the gear: 一般 (你的領域, folders, KnowledgeBase) ──────────────────────
function setRow(k, ...v) {
  return h("div", { class: "set-row" }, h("span", { class: "k" }, k), ...v);
}
async function generalTab(el) {
  let rd = null;
  try { [rd, S.info] = await Promise.all([api("/api/reader"), api("/api/info")]); } catch (e) { el.append(h("div", { class: "set-empty" }, e.message)); return; }
  const field = h("input", { class: "set-input", value: rd.set ? rd.field : "", placeholder: rd.default, "aria-label": "你的領域", maxlength: 200, style: "flex:1" });
  field.addEventListener("change", async () => {
    try {
      await api("/api/reader", { method: "PUT", body: { field: field.value } });
      toast("已儲存");
      await refreshNouns();
      if (S.nouns?.classification.pending > 0) classifyNouns(false);
    } catch (e) { fail(e); }
  });
  const path = (p) => h("span", { class: "v mono", title: p ?? "" }, p ?? "—");
  el.append(
    h("div", { class: "set-sec" }, h("div", { class: "set-label" }, "名詞"), setRow("你的領域", field)),
    h("div", { class: "set-sec" }, h("div", { class: "set-label" }, "資料夾"), setRow("書庫", path(S.info.library)), setRow("匯出", path(S.info.exports))),
    h("div", { class: "set-sec" }, h("div", { class: "set-label" }, "KnowledgeBase"), setRow("網址", path(S.info.kb_url))));
}

/** A KnowledgeBase card: `lore-kb://node/<id>` (the app), else the KB's http address. */
function openKbCard(id) {
  const port = +new URL(S.info?.kb_url ?? "http://127.0.0.1:7700").port;
  if (port) LoreLinks.ports.kb = port;
  LoreLinks.open("kb", { node: id });
}

// ── 最近的論文 (the document menu) ─────────────────────────────────────────
function remember(sha) {
  try {
    const r = JSON.parse(localStorage.getItem("lr.recent") || "[]").filter((x) => x !== sha);
    localStorage.setItem("lr.recent", JSON.stringify([sha, ...r].slice(0, 12)));
    localStorage.setItem("lr.opened", JSON.stringify({ ...JSON.parse(localStorage.getItem("lr.opened") || "{}"), [sha]: Date.now() }));
  } catch {}
}
async function recentPapers() {
  let order = [];
  try { order = JSON.parse(localStorage.getItem("lr.recent") || "[]"); } catch {}
  let list = [];
  try { list = (await api("/api/papers")).papers.filter((p) => p.present); } catch (e) { fail(e); }
  const rank = (p) => { const i = order.indexOf(p.sha256); return i < 0 ? 1e9 : i; };
  list.sort((a, b) => rank(a) - rank(b) || String(b.added_at).localeCompare(String(a.added_at)));
  return list.slice(0, 8).map((p) => ({ icon: "pdf", label: p.title, title: p.original_name, current: p.sha256 === S.sha, run: () => { if (p.sha256 !== S.sha) go(p.sha256, 1); } }));
}

// ── wiring ────────────────────────────────────────────────────────────────
function mountShell() {
  shell = LoreShell.mount({
    app: "lr",
    host: "header.top",
    brand: "LORE/READER",
    home: () => go(null),
    picker: {
      icon: "pdf",
      recent: recentPapers,
      actions: [
        { icon: "upload", label: "加入 PDF…", run: () => $("#openFile").click() },
        { icon: "layers", label: "合併 PDF…", run: () => mergeDialog().catch(fail) },
        { icon: "pen", label: "改標題…", run: () => { if (S.paper) renameDialog(S.paper); } },
        "-",
        { icon: "save", label: "另存新檔（含筆記）…", run: () => saveNotes(null, { ask: false }) },
      ],
    },
    status: { onClick: () => saveNotes(statusAnchor()) },
    primary: [{ id: "actFind", icon: "search", label: "搜尋 ⌘F", run: openFind }],
    ai: { id: "actAsk", icon: "spark", label: "問這篇", run: () => { setTab("chat"); setTimeout(() => $("#chatInput")?.focus(), 80); } },
    notation: () => openNotation(),
    export: [
      { icon: "save", label: "另存新檔（含筆記）…", run: () => saveNotes(null, { ask: false }) },
      "-",
      { icon: "printer", label: "列印", run: print },
    ],
    drawers: { left: "#sorter", leftLabel: "頁面縮圖", leftIcon: "cards", right: "#side", rightLabel: "面板" },
    search: openFind,
    save: () => { if (S.sha && !S.arr && S.exp?.last?.exists) saveNotesLast(); else saveNotes(statusAnchor()); },
    general: generalTab,
    keys: [
      { group: "選取文字", label: "螢光（黃 · 綠 · 藍 · 粉紅）", keys: "1–4" },
      { group: "選取文字", label: "加註記", keys: "N" },
      { group: "選取文字", label: "複製", keys: "C" },
      { group: "選取文字", label: "翻譯", keys: "T" },
      { group: "選取文字", label: "問這篇", keys: "A" },
      { group: "選取文字", label: "存成引用句", keys: "Q" },
      { group: "選取文字", label: "加入知識庫", keys: "K" },
      { group: "問這篇", label: "送出", keys: "⌘↵" },
    ],
  });
  const terms = h("button", { type: "button", class: "ico", id: "actTerms", "data-icon": "target", "aria-label": "標示名詞", title: "紅：要學 · 黃：學習中 · 綠：記得", "aria-pressed": "false", onclick: toggleTerms });
  shell.slots.lead.append(terms);
  paintIcons(shell.el);
  // the drawer toggles: 段落 chips are measured once the right panel is visible
  shell.el.querySelector(".slot-primary .narrow-only")?.addEventListener("click", () => requestAnimationFrame(foldChips));
}

function wire() {
  mountShell();
  buildBlocks();
  paintIcons();
  api("/api/models").then((m) => { S.models = m; renderSide(); }).catch(() => {});
  LoreShell.engines.get().then((d) => { S.engines = d; renderSide(); });
  document.addEventListener("lore-engines", (e) => { S.engines = e.detail; renderSide(); });
  $("#openFile").onchange = async (e) => { const p = await importFiles([...e.target.files]); e.target.value = ""; if (p) go(p.sha256, 1); else if (!S.sha) showHome(); };
  $("#tilePath").onclick = () => pathDialog();
  $("#libFilter").oninput = (e) => { HOME.q = e.target.value; renderLibrary(); };
  $$("#libSort button").forEach((b) => { b.onclick = () => { HOME.sort = b.dataset.sort; try { localStorage.setItem("lr.homeSort", HOME.sort); } catch {} renderLibrary(); }; });
  $("#actMerge").onclick = () => mergeDialog().catch(fail);
  $("#actPrev").onclick = () => scrollToPage(Math.max(0, S.cur - 1));
  $("#actNext").onclick = () => scrollToPage(Math.min(S.pages.length - 1, S.cur + 1));
  $("#pageInput").addEventListener("change", (e) => scrollToPage(clamp((+e.target.value || 1) - 1, 0, S.pages.length - 1)));
  $("#actZoomIn").onclick = () => setZoom(S.zoom * 1.2);
  $("#actZoomOut").onclick = () => setZoom(S.zoom / 1.2);
  $("#zoomChip").onclick = () => setZoom(1);
  $("#actFit").onclick = () => setZoom(fitZoom(), true);
  $("#pgDup").onclick = dupPage;
  $("#pgDel").onclick = delPage;
  $("#pgInsert").onclick = () => insertDialog().catch(fail);
  $("#editSave").onclick = saveArrangement;
  $("#editDiscard").onclick = () => setArrangement(S.orig.map((r) => ({ ...r })), 0);
  $("#searchInput").addEventListener("input", (e) => { clearTimeout(searchTimer); searchTimer = setTimeout(() => runSearch(e.target.value), 250); });
  $("#searchInput").addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); stepSearch(e.shiftKey ? -1 : 1); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); closeFind(); }
  });
  $("#searchNext").onclick = () => stepSearch(1);
  $("#searchPrev").onclick = () => stepSearch(-1);
  $("#searchClose").onclick = closeFind;
  $("#viewport").addEventListener("scroll", onScroll, { passive: true });
  $("#pages").addEventListener("contextmenu", onContextMenu);
  $("#pages").addEventListener("mouseup", () => setTimeout(showSelBar, 10));
  document.addEventListener("keyup", (e) => { if (e.shiftKey && e.key.startsWith("Arrow") && e.target.closest?.(".viewport")) showSelBar(); });
  document.addEventListener("selectionchange", () => { if (getSelection()?.isCollapsed) hideSelBar(); });
  $("#selBar").addEventListener("mousedown", (e) => e.preventDefault());
  $("#viewport").addEventListener("scroll", hideSelBar, { passive: true });
  $("#pages").addEventListener("click", onPageClick);
  $("#pages").addEventListener("mouseover", onPageHover);
  $("#pages").addEventListener("mouseout", onPageHoverOut);
  $("#pop").addEventListener("mouseleave", () => { if ($("#pop").classList.contains("hover-pop")) hidePop(); });
  wireThumbDrag();
  document.addEventListener("mousedown", (e) => {
    if (!e.target.closest("#ctxMenu")) closeMenus();
    if (!e.target.closest("#selBar")) hideSelBar();
    if (!e.target.closest("#pop") && !e.target.closest(".mk[data-t]") && !e.target.closest("dialog")) hidePop();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { closeMenus(); hidePop(); hideSelBar(); if (!$("#find").hidden && !document.querySelector("dialog[open]")) closeFind(); }
    onSelKey(e);
  });
  let rt;
  addEventListener("resize", () => { clearTimeout(rt); rt = setTimeout(() => { if (S.sha && S.fit) setZoom(fitZoom(), true); if (isOpen("paras")) foldChips(); }, 150); });
  addEventListener("popstate", async () => {
    const to = new URL(location.href).searchParams.get("paper");
    if (S.sha && to !== S.sha && !(await confirmLeave())) { history.pushState(null, "", `?paper=${S.sha}&page=${S.pages[S.cur]?.page ?? 1}`); return; }
    route();
  });
  addEventListener("beforeunload", (e) => { if (S.exp?.dirty) { e.preventDefault(); e.returnValue = ""; } });
  // Drop PDFs anywhere to add them.
  document.addEventListener("dragover", (e) => { if ([...e.dataTransfer.types].includes("Files")) e.preventDefault(); });
  let drag = 0;
  const hasFiles = (e) => [...(e.dataTransfer?.types ?? [])].includes("Files");
  document.addEventListener("dragenter", (e) => { if (hasFiles(e)) { drag++; $("#tileAdd").classList.add("over"); } });
  document.addEventListener("dragleave", (e) => { if (hasFiles(e) && --drag <= 0) { drag = 0; $("#tileAdd").classList.remove("over"); } });
  document.addEventListener("drop", () => { drag = 0; $("#tileAdd").classList.remove("over"); }, true);
  document.addEventListener("drop", async (e) => {
    const files = [...e.dataTransfer.files].filter((f) => f.type === "application/pdf" || /\.pdf$/i.test(f.name));
    if (!files.length) return;
    e.preventDefault();
    const p = await importFiles(files);
    if (p) go(p.sha256, 1);
  });
}

async function route() {
  const q = new URL(location.href).searchParams;
  const sha = q.get("paper");
  try {
    if (!S.info) S.info = await api("/api/info");
    if (sha) await openPaper(sha, +q.get("page") || 1);
    else await showHome();
  } catch (e) {
    fail(e);
    if (sha) { S.sha = null; await showHome().catch(fail); }
  }
}

wire();
updateLegend();
route();
// For checks and the console.
window.LR = { S, pageText, pageRich, decorate, focusNoun, setZoom };
