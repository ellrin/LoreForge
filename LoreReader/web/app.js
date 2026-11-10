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
let toastTimer;
function toast(msg, err = false) {
  $(".toast")?.remove();
  const t = h("div", { class: `toast${err ? " err" : ""}`, role: "status" }, msg);
  document.body.append(t);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.remove(), err ? 6000 : 3000);
}
const fail = (e) => toast(e.message ?? String(e), true);
const norm = (s) => (s ?? "").replace(/\s+/g, " ").trim();
const clamp = (x, a, b) => Math.max(a, Math.min(b, x));
const STATE_LABEL = { learning: "學習中", known: "記得", stale: "久未複習", none: "學習中（尚無紀錄）", new: "沒有卡片" };
const SOURCE_LABEL = { kb: "知識庫的詞", expansion: "縮寫全名", compound: "連字號複合詞", acronym: "縮寫", word: "單字" };
const NOUN_STATE = { known: "記得", learning: "學習中", stale: "久未複習", none: "學習中（尚無紀錄）", new: "沒有卡片" };
const GRADE = { certain: ["✅", "確定"], defined: ["🟢", "有定義"], inferred: ["🟡", "推測"], conflict: ["🔴", "衝突"], unresolved: ["⚪", "查不到"] };
const RESOLVER = { program: "程式", local: "地端模型", claude: "Claude", author: "作者" };
const ROLE = { background: "背景", gap: "缺口", method: "方法步驟", result: "結果", discussion: "討論", other: "其他" };
const SECTION_KIND = { front: "標題與作者", abstract: "摘要", intro: "引言／相關研究", method: "方法", result: "結果", discussion: "討論", other: "其他" };
const MODEL_LABEL = { auto: "自動", local: "地端", haiku: "Haiku", sonnet: "Sonnet", opus: "Opus" };
const COLOR_LABEL = { yellow: "黃", green: "綠", blue: "藍", pink: "粉紅" };

// ── state ─────────────────────────────────────────────────────────────────
const S = {
  info: null, kb: null,
  sha: null, paper: null,
  docs: new Map(),          // sha → Promise<PDFDocumentProxy>
  texts: new Map(),         // "sha:page" → Promise<{text, items:[{start,end}]}>
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
  pages: [],                // view items: {sha, page, w, h, el, token, rendered, divs, strs}
  zoom: 1, fit: true, cur: 0,
  annotations: [], quotes: [], chat: [], att: [],
  tab: "notes", termsOn: false,
  search: { q: "", hits: [], i: -1 },
  sending: false,
};
try { S.termsOn = localStorage.getItem("lr.terms") !== "0"; S.tab = localStorage.getItem("lr.tab") || "nouns"; } catch {}

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
    return { text, items };
  })());
  return S.texts.get(k);
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
  S.sha = null; S.paper = null;
  $("#app").classList.add("at-home");
  $("#home").hidden = false;
  $("#paperName").textContent = "";
  document.title = "LoreReader";
  const [info, list] = await Promise.all([api("/api/info"), api("/api/papers")]);
  S.info = info;
  $("#libError").hidden = !info.library_error;
  $("#libError").textContent = info.library_error ?? "";
  $("#libInfo").textContent = `書庫：${info.library} · 匯出：${info.exports} · 註記資料：${info.workspace}`;
  const cards = list.papers.map((p) => h("button", { class: `card${p.present ? "" : " missing"}`, onclick: () => p.present ? go(p.sha256, 1) : toast("這份 PDF 不在這台電腦的書庫裡", true) },
    h("b", { title: p.title }, p.title),
    h("span", { class: "meta", title: p.original_name }, `${p.pages} 頁 · ${p.added_at.slice(0, 10)} · ${p.original_name}`),
    p.doi ? h("span", { class: "meta" }, `doi:${p.doi}`) : null,
    p.derived_from ? h("span", { class: "tag" }, "由頁面編輯產生") : null,
    p.present ? null : h("span", { class: "tag" }, "不在這台電腦的書庫")));
  $("#paperList").replaceChildren(...(cards.length ? cards : [h("p", { class: "empty" }, "書庫是空的。按「開啟 PDF」加入第一份論文；PDF 會複製進書庫（不在任何 git repo 裡），原檔不動。")]));
}

function go(sha, page = 1) {
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
      toast(r.new ? `已加入書庫：${r.paper.title}` : `書庫裡已經有：${r.paper.title}`);
    } catch (e) { fail(e); }
  }
  return last;
}

// ── open a paper ──────────────────────────────────────────────────────────
async function openPaper(sha, page) {
  $("#home").hidden = true;
  $("#app").classList.remove("at-home");
  if (S.sha !== sha) {
    const r = await api(`/api/papers/${sha}`);
    S.sha = sha; S.paper = r.paper; S.arr = null;
    $("#paperName").textContent = r.paper.title;
    $("#paperName").title = r.paper.title;
    document.title = `${r.paper.title} · LoreReader`;
    const doc = await getDoc(sha);
    S.orig = Array.from({ length: doc.numPages }, (_, i) => ({ sha, page: i + 1 }));
    S.search = { q: "", hits: [], i: -1 }; $("#searchInput").value = ""; $("#searchCount").textContent = "";
    S.att = []; S.chatDraft = "";
    S.nouns = null; S.nounMarks = new Map(); S.focusNoun = null;
    S.symbols = null; S.symSpans = []; S.symMarks = new Map();
    S.paras = null; S.curPara = null; S.chatCtx = null; S.chatScope = "all";
    await Promise.all([loadAnnotations(), loadQuotes(), loadChat()]);
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
  const edited = !!S.arr;
  $("#editChip").hidden = !edited;
  $("#editActions").hidden = !edited;
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
    drawHighlights(p);
    decorate(p);
  } catch (e) {
    if (token === p.token) { p.rendered = false; console.error(e); }
  }
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
  if (t && !$("#sorter").matches(":hover")) t.scrollIntoView({ block: "nearest" });
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
    const t = h("div", { class: "thumb", draggable: "true", dataset: { i }, "aria-current": i === S.cur ? "true" : "false", title: own(p) ? `第 ${p.page} 頁` : "從其他 PDF 插入的頁面" },
      h("span", { class: "no" }, i + 1), frame);
    t.addEventListener("click", () => { scrollToPage(i); if (innerWidth <= 1100) togglePanel("sorter", false); });
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
      h("h3", {}, "另存新檔"),
      h("p", { class: "muted", style: "margin:0;font-size:12.5px" }, `新的 PDF（${S.pages.length} 頁）會存進書庫，也會複製一份到匯出資料夾（不覆蓋舊檔）。原本的 PDF 不會改動。`),
      h("label", {}, "標題", input),
      h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => done(null) }, "取消"), h("button", { class: "btn primary" }, "儲存")));
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
        h("h3", {}, "插入頁面：選一份 PDF"),
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
        h("p", { class: "muted", style: "margin:0;font-size:12.5px" }, "點選要插入的頁面（依點選順序）。"),
        grid,
        h("div", { class: "actions", style: "justify-content:space-between;align-items:center" },
          h("span", { class: "muted", style: "font-size:13px;display:flex;gap:6px;align-items:center" }, "插在第", after, "頁之後（0 = 最前面）"),
          h("span", { style: "display:flex;gap:8px" }, h("button", { type: "button", class: "btn", onclick: step1 }, "換一份"), go)));
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
      h("p", { class: "muted", style: "margin:0;font-size:12.5px" }, "依點選順序接起來，存成書庫裡的一份新 PDF（並複製到匯出資料夾）。原檔不動。"),
      ul, h("label", {}, "新 PDF 的標題", title),
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
  if (start == null || end == null || end <= start) {
    const at = ptext.replace(/\s+/g, " ").indexOf(text);
    start = at >= 0 ? at : 0; end = start + text.length;
  }
  const exact = ptext.slice(start, end);
  return {
    p, text, rects, start, end, pageText: ptext,
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
  const m = $("#ctxMenu");
  const item = (icon, label, fn, disabled) => h("button", { type: "button", role: "menuitem", disabled, onclick: () => { closeMenus(); fn(); } }, h("span", { "data-icon": icon }), label);
  m.replaceChildren(
    item("copy", "複製", () => navigator.clipboard.writeText(info.text).then(() => toast("已複製"), () => document.execCommand("copy"))),
    h("hr"),
    h("button", { type: "button", role: "menuitem", disabled: true, style: "opacity:1;cursor:default;color:var(--muted)" }, h("span", { "data-icon": "highlighter" }), "螢光標記"),
    h("div", { class: "swrow" }, (S.info?.colors ?? ["yellow", "green", "blue", "pink"]).map((c) => h("button", { type: "button", class: `sw c-${c}`, "aria-label": `${COLOR_LABEL[c]}色螢光`, title: `${COLOR_LABEL[c]}色`, onclick: () => { closeMenus(); addAnnotation(info, { kind: "highlight", color: c }); } }))),
    item("pen", "加註記…", () => noteDialog(info)),
    h("hr"),
    item("languages", "翻譯", () => translate(info.text, info.context, info.box, info)),
    item("chat", "問 Claude（附上這段）", () => attach(info)),
    item("quote", "存成引用句…", () => quoteDialog(info)),
    item("book", "加入知識庫…", () => addCardDialog({ title: info.text, sha: info.p.sha, page: info.p.page, context: info.context })),
  );
  paintIcons(m);
  placeAt(m, e.clientX, e.clientY);
}

// ── annotations ───────────────────────────────────────────────────────────
async function addAnnotation(info, { kind, color, note = "" }) {
  try {
    const r = await api(`/api/papers/${info.p.sha}/annotations`, { body: { page: info.p.page, kind, color, note, quote: info.quote, rects: info.rects } });
    getSelection().removeAllRanges();
    if (info.p.sha === S.sha) { S.annotations.push(r.annotation); if (S.tab === "notes") renderSide(); }
    S.pages.filter((p) => p.sha === info.p.sha && p.page === info.p.page && p.rendered).forEach(drawHighlights);
    toast(kind === "note" ? "已加註記" : "已標記");
  } catch (e) { fail(e); }
}

async function noteDialog(info) {
  const ta = h("textarea", { placeholder: "寫下想法、疑問或要再查的地方" });
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
  if (innerWidth <= 1100) togglePanel("side", false);
}

// ── citable sentences ─────────────────────────────────────────────────────
async function quoteDialog(info) {
  const ta = h("textarea", {}, info.text);
  const note = h("input", { placeholder: "一行備註（可省略），例如：用在引言、反例" });
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); close({ text: ta.value, note: note.value }); } },
    h("h3", {}, `存成引用句 · 第 ${info.p.page} 頁`),
    h("label", {}, "句子（可修剪）", ta),
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
  showPop(anchor, head, h("div", { class: "muted" }, "翻譯中…（本機模型優先）"));
  try {
    const r = await api("/api/translate", { body: { text, context } });
    const kids = [head, h("div", { class: "out" }, r.translation), h("div", { class: "src" }, `${r.engine}${r.fallback_used ? "（本機模型無法使用，改用 Claude）" : ""}`)];
    const acts = h("div", { class: "actions" },
      h("button", { type: "button", class: "btn small", onclick: () => navigator.clipboard.writeText(r.translation).then(() => toast("已複製譯文")) }, h("span", { "data-icon": "copy" }), "複製譯文"));
    if (info) acts.append(h("button", { type: "button", class: "btn small primary", onclick: () => { hidePop(); addCardDialog({ title: text, sha: info.p.sha, page: info.p.page, context: info.context, hint: r.translation }); } }, h("span", { "data-icon": "book" }), "加入知識庫"));
    showPop(anchor, ...kids, acts);
  } catch (e) {
    showPop(anchor, head, h("div", { class: "err-box" }, e.message), h("div", { class: "muted", style: "font-size:12px" }, "翻譯用本機模型（config.toml 的 [engine] url），連不上時改用 Claude。"));
  }
}

// ── knowledge base ────────────────────────────────────────────────────────
async function addCardDialog({ title, sha, page, context, hint, summary }) {
  hidePop();
  let kb = S.kb;
  try { kb = S.kb = await api("/api/kb/status"); } catch {}
  updateLegend();
  const t = h("input", { value: title.length > 120 ? title.slice(0, 120) : title });
  const sum = h("textarea", { placeholder: "用自己的話寫一兩句：這是什麼、在這篇論文裡做什麼" }, summary ?? "");
  let state = "learning";
  const seg = h("div", { class: "seg", role: "group", "aria-label": "初始狀態" });
  const segHint = h("span", { class: "seg-hint" });
  const paintSeg = () => {
    seg.replaceChildren(...[["learning", "學習中"], ["known", "已掌握"]].map(([v, l]) => h("button", { type: "button", "aria-pressed": v === state ? "true" : "false", onclick: () => { state = v; paintSeg(); } }, l)));
    segHint.textContent = state === "known" ? "已經熟的詞：直接記為「記得」，不排明天的確認，30 天後再問一次。" : "新學的詞：明天問一次、再過 3 天問一次，都記得才算記得。";
  };
  paintSeg();
  const status = kb?.ok ? null : h("div", { class: "err-box" }, kb?.error ?? "KnowledgeBase 沒有回應。", " 開啟 KnowledgeBase 後按「重新檢查」。");
  const draftBtn = h("button", { type: "button", class: "btn", disabled: !kb?.ok, onclick: async () => {
    draftBtn.disabled = true; draftBtn.textContent = "起草中…";
    try { const r = await api("/api/kb/draft", { body: { term: t.value, context } }); if (r.draft?.summary) sum.value = r.draft.summary; toast(`草稿：${r.assistant ?? ""}`); }
    catch (e) { fail(e); }
    draftBtn.disabled = false; draftBtn.replaceChildren(h("span", { "data-icon": "spark" }), "用模型起草摘要"); paintIcons(draftBtn);
  } }, h("span", { "data-icon": "spark" }), "用模型起草摘要");
  const r = await dialog((d, close) => h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); close(true); } },
    h("h3", {}, "加入知識庫"),
    status,
    h("label", {}, "標題", t),
    hint ? h("div", { class: "quote-box" }, h("span", { class: "muted" }, "譯文參考："), hint) : null,
    h("label", {}, "摘要", sum),
    h("div", { style: "display:flex;flex-direction:column;gap:4px" }, h("span", { style: "font-size:12.5px;color:var(--muted)" }, "初始狀態"), seg, segHint),
    h("div", { class: "muted", style: "font-size:12px;overflow-wrap:anywhere" }, `來源：${S.paper?.sha256 === sha ? S.paper.title : "PDF"} · p.${page}（在 KnowledgeBase 可點回這一頁）`),
    h("div", { class: "actions" },
      kb?.ok ? draftBtn : h("button", { type: "button", class: "btn", onclick: () => close("retry") }, "重新檢查"),
      h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"),
      h("button", { class: "btn primary", disabled: !kb?.ok }, "加入知識庫"))));
  if (r === "retry") return addCardDialog({ title: t.value, sha, page, context, hint, summary: sum.value });
  if (r !== true) return;
  try {
    const out = await api("/api/kb/cards", { body: { sha, page, title: t.value, summary: sum.value, initial_state: state, context } });
    toast(`已加入知識庫：${out.node.title}（${state === "known" ? "已掌握" : "學習中"}）`);
    // The new card changes the noun list and the colouring of every page.
    refreshNouns();
  } catch (e) { fail(e); }
}

function updateLegend() {
  $("#legend").hidden = !S.termsOn;
  $("#actTerms").setAttribute("aria-pressed", S.termsOn ? "true" : "false");
  $("#kbNote").textContent = S.kb && !S.kb.ok ? "· KnowledgeBase 沒有回應：沒有卡片的狀態" : "";
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
  p.divs.forEach((div, i) => {
    const it = items[i], str = p.strs[i];
    if (!it) return;
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
  if (S.tab === "symbols") renderSide();
  renderTabCounts();
}

function symMeaning(e) {
  return e.meaning || e.meaning_en || "";
}

function symPopover(sp, anchor, sticky) {
  if (!sp) return;
  const ents = sp.ents.map((i) => S.symbols.entries[i]);
  const kids = [h("div", { class: "sym-run" }, tex(sp.latex), h("span", { class: "src" }, `p.${sp.page}${sp.eq ? ` · 式 ${sp.eq.replace(/^eq:/, "")}` : ""}`))];
  for (const e of ents) {
    const m = symMeaning(e);
    const ev = e.evidence?.[0];
    kids.push(h("div", { class: "sym-def" },
      h("div", { class: "sym-head" }, tex(e.latex ?? e.id), gradeChip(e.grade)),
      m ? h("div", { class: "sym-mean" }, mixed(m)) : h("div", { class: "muted" }, "這篇裡沒找到定義句。"),
      ev?.quote ? h("div", { class: "sym-quote" }, mixed(ev.quote), h("span", { class: "src" }, ` p.${ev.page || "?"} · ${RESOLVER[ev.by] ?? ev.by}`)) : null));
  }
  if (sticky) kids.push(h("div", { class: "actions" }, h("button", { type: "button", class: "btn small", onclick: () => { hidePop(); setTab("symbols"); setTimeout(() => $(`.sym-item[data-i="${sp.ents[0]}"]`)?.scrollIntoView({ block: "center" }), 60); } }, h("span", { "data-icon": "sigma" }), "在符號表中顯示")));
  showPop(anchor, ...kids);
  $("#pop").classList.toggle("hover-pop", !sticky);
}

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
  if (innerWidth <= 1100) togglePanel("side", false);
}

function symbolsPane() {
  const f = S.symbols;
  if (!f) return [h("p", { class: "empty" }, S.readStatus || "讀取全文…")];
  const ents = f.entries ?? [];
  const count = (g) => ents.filter((e) => e.grade === g).length;
  const out = [];
  out.push(h("div", { class: "nn-stats" },
    h("span", {}, `${ents.length} 個符號 · ${f.equations?.length ?? 0} 條獨立方程式`),
    h("span", { class: "muted" }, Object.keys(GRADE).map((g) => `${GRADE[g][0]} ${count(g)}`).join("　")),
    h("span", { class: "muted" }, "程式依字型、字級、基線找出數學；定義句用規則找，🟢 一定附原文。不用模型。")));
  if (f.glyph_errors?.length) out.push(h("div", { class: "warn-box" }, `字形對照表有錯，已略過：${f.glyph_errors.join("；")}`));
  const sel = h("select", { "aria-label": "等級", onchange: (e) => { S.symView.grade = e.target.value; renderSide(); } },
    h("option", { value: "" }, "全部等級"), ...Object.entries(GRADE).map(([g, [i, l]]) => h("option", { value: g }, `${i} ${l}`)));
  sel.value = S.symView.grade;
  out.push(h("div", { class: "sym-tools" }, sel,
    h("button", { type: "button", class: "btn small", onclick: () => symbolDialog(null) }, h("span", { "data-icon": "plus" }), "新增"),
    h("button", { type: "button", class: "ico small", "aria-label": "重新偵測（字形對照表改過之後）", "data-icon": "refresh", onclick: rebuildSymbols }),
    h("button", { type: "button", class: "ico small", "aria-label": "字形對照表…", "data-icon": "gear", onclick: glyphDialog })));
  const conv = (e) => (e.tags?.includes("conventional") ? 1 : 0);
  const shown = ents.map((e, i) => ({ e, i })).filter(({ e }) => !S.symView.grade || e.grade === S.symView.grade).sort((a, b) => conv(a.e) - conv(b.e) || a.i - b.i);
  const list = h("div", { class: "sym-list" }, shown.length ? shown.map(({ e, i }) => {
    const occ = e.occurrences ?? [];
    const pages = [...new Set(occ.map((o) => o.page))];
    const ev = e.evidence?.[0];
    return h("div", { class: "sym-item", dataset: { i } },
      h("div", { class: "sym-head" },
        h("button", { type: "button", class: "sym-latex", title: e.latex, onclick: () => occ[0] && flashBox(occ[0].page, occ[0].bbox) }, tex(e.latex ?? e.id)),
        gradeChip(e.grade),
        e.tags?.includes("conventional") ? h("span", { class: "sym-by", title: "慣用符號（不需要定義）" }, "慣用") : null,
        h("span", { class: "sym-by" }, RESOLVER[e.resolved_by] ?? e.resolved_by),
        h("button", { class: "ico small", "aria-label": "編輯", "data-icon": "pen", onclick: () => symbolDialog(e) })),
      symMeaning(e) ? h("div", { class: "sym-mean" }, mixed(symMeaning(e)), e.meaning && e.meaning_en ? h("span", { class: "muted" }, " · ", mixed(e.meaning_en)) : null) : null,
      ev?.quote ? h("div", { class: "sym-quote" }, mixed(ev.quote), h("span", { class: "src" }, ` p.${ev.page || "?"}${e.evidence.length > 1 ? ` · 另有 ${e.evidence.length - 1} 句` : ""}`)) : null,
      e.note ? h("div", { class: "sym-note" }, e.note) : null,
      h("div", { class: "sym-meta" },
        h("span", {}, `${occ.length} 次`),
        ...pages.slice(0, 8).map((pg) => h("button", { type: "button", class: "pg-chip", onclick: () => { const o = occ.find((x) => x.page === pg); flashBox(pg, o.bbox); } }, `p.${pg}`)),
        pages.length > 8 ? h("span", {}, "…") : null,
        ...eqChips(e.equations ?? [], f)),
      !occ.length && e.first == null ? h("div", { class: "muted", style: "font-size:11.5px" }, "（作者加的；這次沒在文中偵測到）") : null);
  }) : [h("p", { class: "empty" }, "沒有符合的符號。")]);
  out.push(list);
  if (f.equations?.length) {
    out.push(h("h4", { class: "sym-sub" }, "獨立方程式"));
    out.push(h("div", { class: "sym-list eqs" }, f.equations.map((q) => h("button", { type: "button", class: "eq-item", onclick: () => flashBox(q.page, q.bbox) },
      h("span", { class: "eq-no" }, q.label ?? "（無編號）"), h("span", { class: "src" }, `p.${q.page}`), q.latex ? h("span", { class: "eq-tex" }, tex(q.latex)) : null))));
  }
  return out;
}
/** Numbered equations as chips (at most 6); unnumbered ones as one count. */
function eqChips(ids, f) {
  const jump = (id) => { const q = f.equations.find((x) => x.id === id); if (q) flashBox(q.page, q.bbox); };
  const numbered = ids.filter((id) => !/^eq:p\d+-/.test(id));
  const other = ids.filter((id) => /^eq:p\d+-/.test(id));
  const out = numbered.slice(0, 6).map((id) => h("button", { type: "button", class: "pg-chip eq", onclick: () => jump(id) }, q_label(id)));
  if (numbered.length > 6) out.push(h("span", {}, `…另 ${numbered.length - 6} 條`));
  if (other.length) out.push(h("button", { type: "button", class: "pg-chip eq", title: "無編號的獨立方程式；按一下跳到第一條", onclick: () => jump(other[0]) }, `無編號式 ×${other.length}`));
  return out;
}
const q_label = (id) => (/^eq:p\d+-/.test(id) ? "式（無編號）" : `式 (${id.replace(/^eq:/, "")})`);

async function rebuildSymbols() {
  try { setSymbols(await api(`/api/papers/${S.sha}/symbols/rebuild`, { body: {} })); toast("已重新偵測（你改過的條目保留）"); }
  catch (e) { if (e.data?.no_text) readPaper(S.sha); else fail(e); }
}

async function symbolDialog(e) {
  const latex = h("input", { value: e?.latex ?? "", placeholder: "例：w_i、\\hat{y}、\\mathbf{x}", class: "mono" });
  const preview = h("div", { class: "tex-preview" });
  const paint = () => preview.replaceChildren(latex.value.trim() ? tex(latex.value, true) : h("span", { class: "muted" }, "（預覽）"));
  latex.addEventListener("input", paint); paint();
  const zh = h("textarea", { placeholder: "這個符號在這篇論文裡的意思（中文）" }, e?.meaning ?? "");
  const en = h("input", { value: e?.meaning_en ?? "", placeholder: "原文定義（英文，可用 $…$）" });
  const note = h("input", { value: e?.note ?? "", placeholder: "備註（可省略）" });
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
    h("p", { class: "muted", style: "margin:0;font-size:12px" }, "你改過的條目記為「作者」，重新偵測不會蓋掉。"),
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
    h("p", { class: "muted", style: "margin:0;font-size:12.5px;overflow-wrap:anywhere" }, `PDF 字元 → LaTeX。這裡只寫要蓋過內建表的條目；存在 ${g.path}。共用的放 private/shared/glyphs.toml。存檔後按「重新偵測」。`),
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
  if (S.tab === "nouns") renderSide();
  try {
    const doc = await getDoc(sha);
    const pages = [];
    for (let n = 1; n <= doc.numPages; n++) pages.push(await pageRich(sha, n));
    if (S.sha !== sha) return;
    const r = await api(`/api/papers/${sha}/read`, { body: { pages } });
    if (S.sha !== sha) return;
    S.readStatus = "";
    S.words = r.words;
    setSymbols(r.symbols);
    setNouns(r.nouns);
    loadParagraphs(sha);
    if (r.nouns.classification.pending > 0) classifyNouns(false);
  } catch (e) {
    if (S.sha !== sha) return;
    S.readStatus = `讀不到全文：${e.message}`;
    if (S.tab === "nouns") renderSide();
  }
}

async function refreshNouns() {
  if (!S.sha || !S.nouns) return;
  try { setNouns(await api(`/api/papers/${S.sha}/nouns`)); } catch (e) { if (e.data?.no_text) readPaper(S.sha); else fail(e); }
}

function nounClass(n) {
  if (n.state === "known") return "kt-known";
  if (["learning", "stale", "none"].includes(n.state)) return "kt-learn";
  if (n.red) return n.category ? "kt-new" : "kt-new kt-unc";
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
  S.pages.filter((p) => p.rendered).forEach(decorate);
  if (S.tab === "nouns") renderSide();
  renderTabCounts();
}

async function classifyNouns(again) {
  if (!S.sha || S.classifying) return;
  const sha = S.sha;
  S.classifying = true; S.classifyError = null;
  if (S.tab === "nouns") renderSide();
  try {
    const r = await api(`/api/papers/${sha}/nouns/classify`, { body: { again } });
    if (S.sha === sha) setNouns(r.nouns);
  } catch (e) {
    S.classifyError = e.data?.llm_unavailable ? "沒有可用的模型：名詞先不分類（右上「模型」可以換）。" : e.message;
  }
  S.classifying = false;
  if (S.tab === "nouns" && S.sha === sha) renderSide();
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
  if (S.tab === "nouns") renderSide();
  const pi = S.pages.findIndex((p) => p.sha === S.sha && p.page === page);
  if (pi < 0) { toast("這一頁不在目前的頁面清單裡", true); return; }
  scrollToPage(pi, false);
  setTimeout(() => $(".nf-cur")?.scrollIntoView({ block: "center" }), 250);
}
function clearNounFocus() {
  S.focusNoun = null;
  S.pages.filter((p) => p.rendered).forEach(decorate);
  if (S.tab === "nouns") renderSide();
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
    toast(`「${n.text}」已加入忽略清單（ignore-words.txt）`);
  } catch (e) { fail(e); }
}

async function nounCard(n) {
  const { page, context } = await nounContext(n, S.focusNoun?.i === S.nouns.entries.indexOf(n) ? S.focusNoun.k : 0);
  addCardDialog({ title: n.text, sha: S.sha, page, context });
}

function stateChip(state) {
  return h("span", { class: "st" }, h("i", { class: `s-${state}` }), NOUN_STATE[state] ?? state);
}

async function nounPopover(n, anchor, p) {
  if (!n) return;
  const i = S.nouns.entries.indexOf(n);
  const head = h("h4", {}, n.text, stateChip(n.state));
  const meta = h("div", { class: "src" }, `${n.category ?? (n.kb ? "知識庫" : "未分類")} · ${SOURCE_LABEL[n.source] ?? n.source}${n.short ? `（${n.short}）` : ""} · ${n.count} 次 · 第一次在 p.${n.first_page}`);
  const acts = h("div", { class: "actions" },
    h("button", { type: "button", class: "btn small", onclick: () => { hidePop(); setTab("nouns"); focusNoun(i, 0); } }, h("span", { "data-icon": "search" }), `所有出現位置（${n.count}）`));
  const ctx = await nounContext(n);
  acts.append(h("button", { type: "button", class: "btn small", onclick: () => translate(n.text, ctx.context, null, { p: { sha: S.sha, page: ctx.page }, context: ctx.context }) }, h("span", { "data-icon": "languages" }), "翻譯"));
  if (!n.kb) {
    acts.append(h("button", { type: "button", class: "btn small", onclick: () => ignoreNoun(n) }, h("span", { "data-icon": "eyeOff" }), "這不是術語"));
    acts.append(h("button", { type: "button", class: "btn small primary", onclick: () => nounCard(n) }, h("span", { "data-icon": "book" }), "加入知識庫"));
    showPop(anchor, head, meta, acts);
    return;
  }
  acts.append(h("a", { class: "btn small", href: `${S.info?.kb_url ?? "http://127.0.0.1:7700"}/#/node/${n.kb.node_id}`, target: "_blank", rel: "noopener" }, h("span", { "data-icon": "external" }), "在 KnowledgeBase 開啟"));
  showPop(anchor, head, meta, h("div", { class: "muted" }, "讀取卡片…"));
  let summary = "";
  try { const v = await api(`/api/kb/nodes/${n.kb.node_id}`); summary = v.node?.summary ?? ""; } catch (err) { summary = err.message; }
  showPop(anchor, head, meta, h("div", { class: "out" }, summary || "（這張卡還沒有摘要）"), acts);
}

/** Right-click on a coloured term (no selection): the term's own menu. */
function nounMenu(n, x, y) {
  const m = $("#ctxMenu");
  const i = S.nouns.entries.indexOf(n);
  const item = (icon, label, fn) => h("button", { type: "button", role: "menuitem", onclick: () => { closeMenus(); fn(); } }, h("span", { "data-icon": icon }), label);
  m.replaceChildren(
    h("div", { class: "menu-title" }, n.text),
    item("search", `所有出現位置（${n.count}）`, () => { setTab("nouns"); focusNoun(i, 0); }),
    n.kb ? null : item("eyeOff", "這不是術語", () => ignoreNoun(n)),
    n.kb ? null : item("book", "加入知識庫…", () => nounCard(n)),
  );
  paintIcons(m);
  placeAt(m, x, y);
}

const NOUN_COLS = [["text", "詞"], ["category", "分類"], ["state", "知識庫"], ["count", "次數"], ["page", "頁"]];
const STATE_SHORT = { new: "無卡", learning: "學習中", stale: "久未", none: "學習中", known: "記得" };
const STATE_ORDER = { new: 0, learning: 1, stale: 1, none: 1, known: 2 };

function nounsPane() {
  const n = S.nouns;
  const out = [];
  if (!n) {
    out.push(h("p", { class: "empty" }, S.readStatus || "讀取全文…"));
    return out;
  }
  const st = n.stats;
  const v = S.nounView;
  const cls = n.classification;
  const engine = S.classifying ? "分類中…（一篇只呼叫一次模型）" : cls.engine ? `分類：${cls.engine}` : "尚未分類";
  out.push(h("div", { class: "nn-stats" },
    h("span", {}, `${st.words.toLocaleString()} 字 → ${st.units} 個詞`),
    h("span", { class: "muted" }, `沒有卡片 ${st.no_card} · 學習中 ${st.learning} · 記得 ${st.known}`),
    h("span", { class: "muted nn-engine" }, engine, cls.unclassified && !S.classifying ? ` · 未分類 ${cls.unclassified}` : "",
      !S.classifying && cls.unclassified ? h("button", { type: "button", class: "btn ghost small", onclick: () => classifyNouns(true) }, "分類") : null)));
  if (S.classifyError) out.push(h("div", { class: "warn-box" }, S.classifyError));
  if (n.kb && !n.kb.ok) out.push(h("div", { class: "warn-box" }, "KnowledgeBase 沒有回應：全部當作沒有卡片。開啟 KnowledgeBase 後重新開啟論文。"));
  const catSel = h("select", { "aria-label": "分類", onchange: (e) => { v.cat = e.target.value; renderSide(); } },
    h("option", { value: "" }, "全部分類"), ...n.categories.map((c) => h("option", { value: c }, c)), h("option", { value: "-" }, "未分類"));
  catSel.value = v.cat;
  const stSel = h("select", { "aria-label": "知識庫狀態", onchange: (e) => { v.state = e.target.value; renderSide(); } },
    h("option", { value: "" }, "全部狀態"), h("option", { value: "red" }, "紅字（要學）"), h("option", { value: "new" }, "沒有卡片"), h("option", { value: "learning" }, "學習中／久未複習"), h("option", { value: "known" }, "記得"));
  stSel.value = v.state;
  const q = h("input", { type: "search", placeholder: "篩選", value: v.q, "aria-label": "篩選詞", oninput: (e) => { v.q = e.target.value; clearTimeout(S.nqTimer); S.nqTimer = setTimeout(() => { renderSide(); $(".nn-tools input")?.focus(); }, 200); } });
  out.push(h("div", { class: "nn-tools" }, catSel, stSel, q));
  let rows = n.entries.map((e, i) => ({ e, i }));
  if (v.cat) rows = rows.filter(({ e }) => (v.cat === "-" ? !e.category && !e.kb : e.category === v.cat));
  if (v.state) rows = rows.filter(({ e }) => (v.state === "red" ? e.red : v.state === "learning" ? ["learning", "stale", "none"].includes(e.state) : e.state === v.state));
  if (v.q.trim()) { const qq = v.q.trim().toLowerCase(); rows = rows.filter(({ e }) => e.text.toLowerCase().includes(qq) || e.key.includes(qq)); }
  const catIdx = (e) => (e.category ? n.categories.indexOf(e.category) : e.kb ? -1 : 99);
  const key = { text: (e) => e.text.toLowerCase(), category: catIdx, state: (e) => STATE_ORDER[e.state] ?? 0, count: (e) => -e.count, page: (e) => e.first_page * 1e6 + e.occ[0][1] }[v.sort];
  rows.sort((a, b) => { const x = key(a.e), y = key(b.e); return (x < y ? -1 : x > y ? 1 : 0) * v.dir || a.i - b.i; });
  const head = h("div", { class: "nn-row nn-head", role: "row" }, NOUN_COLS.map(([k, label]) => h("button", { type: "button", class: v.sort === k ? "on" : "", "aria-sort": v.sort === k ? (v.dir > 0 ? "ascending" : "descending") : "none", onclick: () => { if (v.sort === k) v.dir = -v.dir; else { v.sort = k; v.dir = 1; } renderSide(); } }, label, v.sort === k ? (v.dir > 0 ? " ↑" : " ↓") : "")), h("span"));
  const list = rows.map(({ e, i }) => {
    const on = S.focusNoun?.i === i;
    const row = h("div", { class: `nn-row${on ? " on" : ""}`, role: "row", tabindex: "0", title: `${e.text}${e.short ? `（${e.short}）` : ""}`,
      onclick: (ev) => { if (!ev.target.closest("button")) (on ? focusNoun(i, S.focusNoun.k + 1) : focusNoun(i, 0)); },
      oncontextmenu: (ev) => { ev.preventDefault(); nounMenu(e, ev.clientX, ev.clientY); },
      onkeydown: (ev) => { if (ev.key === "Enter") focusNoun(i, on ? S.focusNoun.k + 1 : 0); } },
      h("span", { class: "nn-term" }, h("i", { class: `nn-dot ${nounClass(e) || "kt-none"}` }), h("span", { class: "ellipsis" }, e.text)),
      h("span", { class: "nn-cat ellipsis" }, e.category ?? (e.kb ? "—" : "未分類")),
      h("span", { class: "nn-st", title: NOUN_STATE[e.state] }, h("i", { class: `s-${e.state}` }), STATE_SHORT[e.state] ?? e.state),
      h("span", { class: "nn-num" }, on ? `${S.focusNoun.k + 1}/${e.count}` : e.count),
      h("span", { class: "nn-num" }, e.first_page),
      h("span", { class: "nn-acts" },
        on ? h("button", { class: "ico small", "aria-label": "上一個出現位置", "data-icon": "up", onclick: () => focusNoun(i, S.focusNoun.k - 1) }) : null,
        on ? h("button", { class: "ico small", "aria-label": "下一個出現位置", "data-icon": "chevron", onclick: () => focusNoun(i, S.focusNoun.k + 1) }) : null,
        !on && !e.kb ? h("button", { class: "ico small", "aria-label": "這不是術語", "data-icon": "eyeOff", onclick: () => ignoreNoun(e) }) : null,
        !on && !e.kb ? h("button", { class: "ico small", "aria-label": "加入知識庫…", "data-icon": "book", onclick: () => nounCard(e) }) : null,
        on ? h("button", { class: "ico small", "aria-label": "取消標示", "data-icon": "x", onclick: clearNounFocus }) : null));
    return row;
  });
  out.push(h("div", { class: "nn-table", role: "table", "aria-label": "名詞清單" }, head, ...(list.length ? list : [h("p", { class: "empty" }, "沒有符合的詞。")])));
  out.push(h("p", { class: "muted nn-foot" }, "點一個詞：文中所有出現的位置都會亮起來，再點一下（或 ↓）跳到下一個。右鍵「這不是術語」寫進忽略清單。紅字＝專業術語、沒有卡片；一般學術詞與人名只列出、不標紅。"));
  return out;
}

// ── paragraphs (track B) ──────────────────────────────────────────────────
async function loadParagraphs(sha) {
  clearTimeout(S.paraTimer);
  try {
    const r = await api(`/api/papers/${sha}/paragraphs`);
    if (S.sha !== sha) return;
    S.paras = r;
    if (S.tab === "paras") renderSide(); else renderTabCounts();
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
  renderSide();
  jumpPara(n.id);
  setTimeout(() => $(`.para[data-id="${n.id}"]`)?.scrollIntoView({ block: "nearest" }), 60);
  void vpEl;
}

function chipLabel(kind, id) {
  if (kind === "term") return S.nouns?.entries.find((e) => e.key === id)?.text ?? id;
  if (kind === "symbol") return S.symbols?.entries.find((e) => e.id === id)?.latex ?? id;
  return q_label(id);
}

function chipMenu(p, kind, id, ev) {
  const m = $("#ctxMenu");
  const label = chipLabel(kind, id);
  const item = (icon, text, fn) => h("button", { type: "button", role: "menuitem", onclick: () => { closeMenus(); fn(); } }, h("span", { "data-icon": icon }), text);
  const box = ev.currentTarget.getBoundingClientRect();
  const anchor = { left: box.left, top: box.top, bottom: box.bottom };
  m.replaceChildren(
    h("div", { class: "menu-title" }, kind === "symbol" ? tex(label) : label),
    item("eye", "看定義", () => showChipDef(kind, id, anchor)),
    item("chat", "問 Claude", () => askClaude(p.id, { kind, id, label: kind === "symbol" ? label : label })),
    kind === "equation" ? null : item("book", "加入知識庫…", () => addCardDialog({ title: kind === "symbol" ? "" : label, sha: S.sha, page: p.page, context: p.text.slice(0, 600) })));
  paintIcons(m);
  placeAt(m, box.left, box.bottom + 4);
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
    h("dd", {}, h("span", { class: "k" }, "作用目標"), mixed(q.target || "—")),
    h("dd", {}, h("span", { class: "k" }, "作用結果"), mixed(q.effect || "—")))));
  if (a.motive) out.append(h("div", { class: "motive" }, h("span", { class: "k" }, "動機"), mixed(a.motive)));
  if (a.results?.length) out.append(...a.results.map((r) => h("dl", { class: "resrow" },
    ...[["指標", r.metric], ["比較對象", r.comparison], ["數值", r.values], ["差距", r.difference], ["檢定", r.test], ["顯著性", r.significance]].map(([k, v]) => h("dd", {}, h("span", { class: "k" }, k), mixed(v || "—"))))));
  return out.childElementCount ? out : null;
}

function parasPane() {
  const d = S.paras;
  if (!d) return [h("p", { class: "empty" }, S.readStatus || "整理段落…")];
  const st = d.status ?? {};
  const pr = d.progress;
  const out = [];
  out.push(h("div", { class: "para-top" },
    h("div", { class: "progress", role: "progressbar", "aria-valuenow": pr.understood, "aria-valuemax": pr.total, "aria-label": "懂了的段落" }, h("i", { style: `width:${pr.total ? (pr.understood / pr.total) * 100 : 0}%` })),
    h("span", { class: "para-count" }, `懂了 ${pr.understood}/${pr.total}`),
    h("button", { type: "button", class: "btn small", onclick: nextNotUnderstood }, h("span", { "data-icon": "next" }), "下一段還沒懂的")));
  const runLine = st.running ? `分析中 ${st.done}/${st.total}${st.engine ? `（${st.engine}）` : ""}：方法與結果段先跑` : st.error ? null : st.total ? `分析完成 ${st.done}/${st.total}${st.engine ? `（${st.engine}）` : ""}` : "";
  out.push(h("div", { class: "muted para-status" }, runLine, " 不提供整段的中文摘要或翻譯，只標出卡住的地方。"));
  if (st.error) out.push(h("div", { class: "warn-box" }, st.error, " 標籤（生詞、符號、方程式）照樣列出。", h("button", { type: "button", class: "btn ghost small", onclick: async () => { try { S.paras = await api(`/api/papers/${S.sha}/paragraphs/run`, { body: {} }); renderSide(); loadParagraphs(S.sha); } catch (e) { fail(e); } } }, "再試一次")));
  let section = null;
  for (const p of d.paragraphs) {
    if (p.section_kind === "front") continue;
    if (p.section !== section) { section = p.section; out.push(h("h4", { class: "para-sec" }, section || SECTION_KIND[p.section_kind] || "（未命名）")); }
    const a = p.analysis;
    const role = a ? ROLE[a.role] ?? a.role : st.running ? "分析中…" : "—";
    const chips = [
      ...p.chips.terms.map((id) => h("button", { type: "button", class: "chip-b term", onclick: (ev) => chipMenu(p, "term", id, ev) }, chipLabel("term", id))),
      ...p.chips.symbols.map((id) => { const e = S.symbols?.entries.find((x) => x.id === id); return h("button", { type: "button", class: `chip-b sym g-${e?.grade ?? "unresolved"}`, title: e ? GRADE[e.grade]?.[1] : "", onclick: (ev) => chipMenu(p, "symbol", id, ev) }, tex(e?.latex ?? id), h("span", { class: "gi" }, GRADE[e?.grade]?.[0] ?? "")); }),
      ...p.chips.equations.map((id) => h("button", { type: "button", class: "chip-b eq", onclick: (ev) => chipMenu(p, "equation", id, ev) }, q_label(id))),
    ];
    out.push(h("div", { class: `para${p.understood ? " done" : ""}${S.curPara === p.id ? " cur" : ""}`, dataset: { id: p.id } },
      h("div", { class: "para-head" }, h("span", { class: `role r-${a?.role ?? "none"}` }, role), h("span", { class: "src" }, `p.${p.page} · ${p.words} 字`), p.understood ? h("span", { class: "ok-mark" }, "✓ 懂了") : null),
      h("button", { type: "button", class: "para-text", title: "跳到這段", onclick: () => { S.curPara = p.id; renderSide(); jumpPara(p.id); } }, p.text),
      chips.length ? h("div", { class: "para-chips" }, ...chips) : h("div", { class: "muted", style: "font-size:11.5px" }, "沒有生詞或符號。"),
      analysisBlock(a),
      h("div", { class: "para-acts" },
        h("button", { type: "button", class: "btn small", onclick: () => askClaude(p.id, null) }, h("span", { "data-icon": "alert" }), "這段我卡住了"),
        h("button", { type: "button", class: `btn small${p.understood ? " primary" : ""}`, "aria-pressed": p.understood ? "true" : "false", onclick: () => setUnderstood(p, !p.understood) }, h("span", { "data-icon": "check" }), "懂了"))));
  }
  return out;
}

// ── model menu ────────────────────────────────────────────────────────────
async function modelDialog() {
  let m;
  try { m = await api("/api/models"); } catch (e) { fail(e); return; }
  const rows = h("div", { class: "model-rows" });
  const paint = () => rows.replaceChildren(...m.tasks.map((t) => h("div", { class: "model-row" },
    h("div", {}, h("b", {}, t.label), h("span", { class: "muted mono" }, ` ${t.id} · 預設 ${t.default}`)),
    h("div", { class: "seg", role: "group", "aria-label": t.label }, m.choices.map((c) => h("button", { type: "button", "aria-pressed": t.choice === c ? "true" : "false", onclick: async () => {
      try { m = await api("/api/models", { method: "PUT", body: { task: t.id, model: c } }); paint(); } catch (e) { fail(e); }
    } }, MODEL_LABEL[c] ?? c))))));
  paint();
  await dialog((d, close) => h("div", { class: "dlg" },
    h("h3", {}, "模型"),
    h("p", { class: "muted", style: "margin:0;font-size:12.5px" }, "每個工作各自選地端模型或 Claude（Haiku／Sonnet／Opus）。「自動」照 tasks.toml 的預設順序；選了某一個就只用那一個，用不了會直接說原因，不會偷偷改用別的。"),
    m.local_engine ? null : h("div", { class: "warn-box" }, "沒有設定地端模型（config.toml 的 [engine] url）：選「地端」的工作會失敗。"),
    rows,
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn primary", onclick: () => close(true) }, "完成"))));
}

// ── search ────────────────────────────────────────────────────────────────
let searchTimer;
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
  $("#searchCount").textContent = needle.length >= 2 ? (S.search.hits.length ? `0/${S.search.hits.length}` : "沒有") : "";
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
  setTimeout(() => $(".sr-cur")?.scrollIntoView({ block: "center" }), 300);
}

// ── chat ──────────────────────────────────────────────────────────────────
function attach(info) {
  S.att.push({ text: info.text, page: info.p.page, context: info.context });
  getSelection().removeAllRanges();
  setTab("chat");
  if (innerWidth <= 1100) togglePanel("side", true);
  setTimeout(() => $("#chatInput")?.focus(), 50);
}

async function sendChat() {
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
    if (ctx) { body.paragraph = ctx.paragraph; if (ctx.chip) body.chip = { kind: ctx.chip.kind, id: ctx.chip.id }; }
    const r = await api(`/api/papers/${S.sha}/chat`, { body });
    S.chat.pop();
    await loadChat();
  } catch (e) {
    S.chat.pop(); S.att = att; S.chatDraft = text; S.chatCtx = ctx;
    S.chatError = e.message;
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
  if (innerWidth <= 1100) togglePanel("side", true);
  setTimeout(() => { const t = $("#chatInput"); t?.focus(); t?.select(); }, 60);
}

// ── side panel ────────────────────────────────────────────────────────────
function setTab(t) {
  S.tab = t;
  try { localStorage.setItem("lr.tab", t); } catch {}
  renderSide();
}

function renderTabCounts() {
  $$(".tabs [data-tab]").forEach((b) => {
    b.setAttribute("aria-selected", b.dataset.tab === S.tab ? "true" : "false");
    const t = b.dataset.tab;
    const n = t === "notes" ? S.annotations.length : t === "quotes" ? S.quotes.length : t === "nouns" ? (S.nouns?.entries.filter((e) => e.red).length ?? 0) : t === "symbols" ? (S.symbols?.entries?.length ?? 0) : t === "paras" && S.paras ? `${S.paras.progress.understood}/${S.paras.progress.total}` : 0;
    b.querySelector(".n")?.remove();
    if (n) b.append(h("span", { class: "n" }, n));
  });
}

function renderSide() {
  renderTabCounts();
  const pane = $("#pane");
  const keep = S.tab === "nouns" ? $(".nn-table", pane)?.scrollTop : null;
  $(".composer")?.remove();
  pane.classList.toggle("chat-pane", S.tab === "chat");
  pane.classList.toggle("nouns-pane", S.tab === "nouns");
  if (S.tab === "paras") { const keep2 = pane.scrollTop; pane.replaceChildren(...parasPane()); pane.scrollTop = keep2; }
  else if (S.tab === "symbols") pane.replaceChildren(...symbolsPane());
  else if (S.tab === "nouns") { pane.replaceChildren(...nounsPane()); if (keep != null && $(".nn-table", pane)) $(".nn-table", pane).scrollTop = keep; }
  else if (S.tab === "notes") pane.replaceChildren(...notesPane());
  else if (S.tab === "quotes") pane.replaceChildren(...quotesPane());
  else { pane.replaceChildren(...chatPane()); $("#side").append(composer()); pane.scrollTop = pane.scrollHeight; }
  paintIcons($("#side"));
}

function notesPane() {
  if (!S.annotations.length) return [h("p", { class: "empty" }, "選取文字後按右鍵：螢光標記（四種顏色）或加註記。註記存在私人資料夾，用頁碼和原文定位，放大縮小都不會跑掉。")];
  const sorted = [...S.annotations].sort((a, b) => a.page - b.page || (a.rects?.[0]?.[1] ?? 0) - (b.rects?.[0]?.[1] ?? 0));
  return [h("ul", { class: "items" }, sorted.map((a) => h("li", { class: "item", onclick: (e) => { if (!e.target.closest("button")) jumpTo(a.page, a.id); } },
    h("span", { class: `dot c-${a.color}` }),
    h("span", { class: "body" }, h("span", { class: "txt" }, a.quote?.exact || "（無文字）"), a.note ? h("span", { class: "note" }, a.note) : null, h("span", { class: "meta" }, `p.${a.page} · ${a.kind === "note" ? "註記" : "螢光"} · ${a.created_at.slice(0, 10)}`)),
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
    S.pages.filter((p) => p.rendered && p.page === a.page).forEach(drawHighlights);
  } catch (e) { fail(e); }
}

async function delAnnotation(a) {
  try {
    await api(`/api/papers/${S.sha}/annotations/${a.id}`, { method: "DELETE" });
    S.annotations = S.annotations.filter((x) => x.id !== a.id);
    renderSide();
    S.pages.filter((p) => p.rendered && p.page === a.page).forEach(drawHighlights);
  } catch (e) { fail(e); }
}

function quotesPane() {
  if (!S.quotes.length) return [h("p", { class: "empty" }, "選取一句話後按右鍵 →「存成引用句」。之後在 PaperComposer 可以引用（所有論文的引用句：GET /api/quotes）。")];
  return [h("ul", { class: "items" }, S.quotes.map((q) => h("li", { class: "item", onclick: (e) => { if (!e.target.closest("button")) jumpTo(q.page); } },
    h("span", { class: "dot q" }),
    h("span", { class: "body" }, h("span", { class: "txt" }, `“${q.text}”`), q.note ? h("span", { class: "note" }, q.note) : null, h("span", { class: "meta" }, `p.${q.page} · ${q.created_at.slice(0, 10)}`)),
    h("span", { class: "acts" },
      h("button", { class: "ico small", "aria-label": "複製（含頁碼）", "data-icon": "copy", onclick: () => navigator.clipboard.writeText(`“${q.text}” (p. ${q.page})`).then(() => toast("已複製")) }),
      h("button", { class: "ico small", "aria-label": "刪除", "data-icon": "trash", onclick: async () => { try { await api(`/api/papers/${S.sha}/quotes/${q.id}`, { method: "DELETE" }); await loadQuotes(); renderSide(); } catch (e) { fail(e); } } })))))];
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
  const head = h("div", { class: "pane-head" }, h("h4", {}, "問 Claude · 對話存在這篇論文的私人資料夾"),
    S.chat.length ? h("button", { class: "ico small", "aria-label": "清除對話", "data-icon": "trash", onclick: async () => { try { await api(`/api/papers/${S.sha}/chat`, { method: "DELETE" }); S.chat = []; renderSide(); } catch (e) { fail(e); } } }) : null);
  const cur = paraOf(S.curPara);
  const scope = h("div", { class: "seg chat-scope", role: "group", "aria-label": "顯示哪些對話" },
    h("button", { type: "button", "aria-pressed": S.chatScope !== "para" ? "true" : "false", onclick: () => { S.chatScope = "all"; renderSide(); } }, "全部"),
    h("button", { type: "button", "aria-pressed": S.chatScope === "para" ? "true" : "false", disabled: !cur, title: cur ? `p.${cur.page} ${cur.text.slice(0, 40)}…` : "先在「段落」選一段", onclick: () => { S.chatScope = "para"; renderSide(); } }, cur ? `這段（p.${cur.page}）` : "這段"));
  const shown = S.chatScope === "para" && cur ? S.chat.filter((m) => m.paragraph === cur.id) : S.chat;
  const msgs = shown.map((m) => h("div", { class: `msg ${m.role}` },
    m.role === "user" && m.paragraph ? h("button", { type: "button", class: "para-badge", onclick: () => { S.curPara = m.paragraph; jumpPara(m.paragraph); } }, `段落 p.${paraOf(m.paragraph)?.page ?? "?"}`, m.chip ? [" · 卡住：", chipText(m.chip)] : " · 整段") : null,
    ...(m.attachments ?? []).map((a) => h("span", { class: "att", title: a.context }, `p.${a.page}「${a.text}」`)),
    m.role === "assistant" ? mixed(m.text) : m.text,
    m.role === "assistant" && m.engine ? h("span", { class: "eng" }, m.engine) : null,
    m.role === "assistant" && m.paragraph ? answerActions(m) : null,
    m.pending ? h("span", { class: "eng" }, "Claude 思考中…") : null));
  const err = S.chatError ? h("div", { class: "err-box" }, S.chatError) : null;
  S.chatError = null;
  return [head, scope, h("div", { class: "msgs" }, msgs.length ? msgs : [h("p", { class: "empty" }, S.chatScope === "para" ? "這段還沒有對話。在「段落」按「這段我卡住了」或某個標籤的「問 Claude」。" : "選取文字後按右鍵 →「問 Claude」，會附上這段話、頁碼和所在段落；或在「段落」分頁按「這段我卡住了」。")]), err].filter(Boolean);
}

/** Under an answer about a paragraph: write it back to the symbol table, keep it as
 *  this paragraph's note, or add the term to the KB. */
function answerActions(m) {
  const p = paraOf(m.paragraph);
  const syms = p?.chips?.symbols ?? [];
  return h("div", { class: "ans-acts" },
    syms.length ? h("button", { type: "button", class: "btn small", onclick: () => writeBackDialog(m, p) }, h("span", { "data-icon": "sigma" }), "寫回符號表") : null,
    h("button", { type: "button", class: "btn small", onclick: () => saveParaNote(m, p) }, h("span", { "data-icon": "pen" }), "存成這段的註記"),
    h("button", { type: "button", class: "btn small", onclick: () => addCardDialog({ title: m.chip?.kind === "term" ? (m.chip.label ?? m.chip.id) : "", sha: S.sha, page: p?.page ?? 1, context: p?.text?.slice(0, 600), summary: m.text.slice(0, 300) }) }, h("span", { "data-icon": "book" }), "加入知識庫"));
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
    h("h3", {}, "寫回符號表"),
    h("label", {}, "符號", sel),
    h("label", {}, "意思", zh),
    h("div", { style: "display:flex;flex-direction:column;gap:4px" }, h("span", { style: "font-size:12.5px;color:var(--muted)" }, "等級（依據是 Claude 的回答；有引用原文定義句才選 🟢）"), seg),
    h("div", { class: "quote-box" }, h("span", { class: "muted" }, "依據："), m.text.slice(0, 400), m.text.length > 400 ? "…" : ""),
    h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "取消"), h("button", { class: "btn primary" }, "寫回"))));
  if (r !== true) return;
  try {
    await api(`/api/papers/${S.sha}/symbols/${encodeURIComponent(pick.id)}`, { method: "PUT", body: { meaning: zh.value, grade, by: "claude", evidence: m.text, page: p.page } });
    setSymbols(await api(`/api/papers/${S.sha}/symbols`));
    toast(`已寫回 ${pick.latex}（由 Claude 解）`);
  } catch (e) { fail(e); }
}

async function saveParaNote(m, p) {
  try {
    const r = await api(`/api/papers/${S.sha}/annotations`, { body: { page: p.page, kind: "note", color: "blue", note: m.text, quote: { exact: p.text.slice(0, 160), prefix: "", suffix: "" }, rects: [], paragraph: p.id, chip: m.chip ?? null } });
    S.annotations.push(r.annotation);
    toast("已存成這段的註記（「註記」分頁）");
  } catch (e) { fail(e); }
}

function composer() {
  const ctxChip = S.chatCtx ? (() => {
    const p = paraOf(S.chatCtx.paragraph);
    return h("span", { class: "attchip ctx", title: "會一起送出：論文標題、章節、這段原文、卡住的點在本篇的定義、你的知識庫狀態" },
      h("span", {}, `段落 p.${p?.page ?? "?"}`, S.chatCtx.chip ? [" · 卡住：", chipText(S.chatCtx.chip)] : " · 整段所有標籤"),
      h("button", { class: "ico small", "aria-label": "不附段落", "data-icon": "x", onclick: () => { S.chatCtx = null; renderSide(); } }));
  })() : null;
  const chips = h("div", { class: "chips" }, ctxChip, S.att.map((a, i) => h("span", { class: "attchip", title: a.context }, h("span", {}, `p.${a.page}「${a.text}」`),
    h("button", { class: "ico small", "aria-label": "移除引文", "data-icon": "x", onclick: () => { S.att.splice(i, 1); renderSide(); } }))));
  const ta = h("textarea", { id: "chatInput", placeholder: S.att.length || S.chatCtx ? "想問這段什麼？" : "問這篇論文的問題（⌘/Ctrl＋Enter 送出）" });
  ta.value = S.chatDraft ?? "";
  ta.addEventListener("input", () => { S.chatDraft = ta.value; });
  ta.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); sendChat(); } });
  return h("div", { class: "composer" }, chips, ta, h("div", { class: "row" }, h("button", { class: "btn small primary", disabled: S.sending, onclick: sendChat }, h("span", { "data-icon": "send" }), S.sending ? "等待回覆…" : "送出")));
}

// ── print ─────────────────────────────────────────────────────────────────
async function print() {
  if (S.arr) { toast("頁面有未儲存的變更：先「另存新檔」再列印", true); return; }
  try {
    const r = await api("/api/print", { body: { sha: S.sha } });
    if (r.native) { toast("正在準備列印…"); return; }
  } catch (e) { fail(e); return; }
  const w = window.open(`print.html?sha=${S.sha}&auto=1`, "_blank");
  if (!w) location.href = `print.html?sha=${S.sha}&auto=1`;
}

// ── narrow widths: drawers ────────────────────────────────────────────────
function togglePanel(which, force) {
  const app = $("#app");
  const cls = which === "sorter" ? "show-sorter" : "show-side";
  const on = force ?? !app.classList.contains(cls);
  app.classList.toggle(cls, on);
  if (on) app.classList.remove(which === "sorter" ? "show-side" : "show-sorter");
  $("#toggleThumbs").setAttribute("aria-pressed", app.classList.contains("show-sorter"));
  $("#toggleSide").setAttribute("aria-pressed", app.classList.contains("show-side"));
}

// ── wiring ────────────────────────────────────────────────────────────────
function wire() {
  paintIcons();
  $("#actHome").onclick = () => go(null);
  $("#actMode").onclick = () => window.loreTheme.toggle();
  $("#openFile").onchange = async (e) => { const p = await importFiles([...e.target.files]); e.target.value = ""; if (p) go(p.sha256, 1); else showHome(); };
  $("#pathForm").onsubmit = async (e) => { e.preventDefault(); try { const r = await api("/api/papers/import", { body: { path: $("#pathInput").value.trim() } }); $("#pathInput").value = ""; go(r.paper.sha256, 1); } catch (x) { fail(x); } };
  $("#actMerge").onclick = () => mergeDialog().catch(fail);
  $("#actPrev").onclick = () => scrollToPage(Math.max(0, S.cur - 1));
  $("#actNext").onclick = () => scrollToPage(Math.min(S.pages.length - 1, S.cur + 1));
  $("#pageInput").addEventListener("change", (e) => scrollToPage(clamp((+e.target.value || 1) - 1, 0, S.pages.length - 1)));
  $("#actZoomIn").onclick = () => setZoom(S.zoom * 1.2);
  $("#actZoomOut").onclick = () => setZoom(S.zoom / 1.2);
  $("#actFit").onclick = () => setZoom(fitZoom(), true);
  $("#actPrint").onclick = print;
  $("#actTerms").onclick = toggleTerms;
  $("#actModels").onclick = () => modelDialog();
  $("#pgDup").onclick = dupPage;
  $("#pgDel").onclick = delPage;
  $("#pgInsert").onclick = () => insertDialog().catch(fail);
  $("#editSave").onclick = saveArrangement;
  $("#editDiscard").onclick = () => setArrangement(S.orig.map((r) => ({ ...r })), 0);
  $("#toggleThumbs").onclick = () => togglePanel("sorter");
  $("#toggleSide").onclick = () => togglePanel("side");
  $$(".tabs [data-tab]").forEach((b) => (b.onclick = () => setTab(b.dataset.tab)));
  $("#searchInput").addEventListener("input", (e) => { clearTimeout(searchTimer); searchTimer = setTimeout(() => runSearch(e.target.value), 250); });
  $("#searchInput").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); stepSearch(e.shiftKey ? -1 : 1); } });
  $("#searchNext").onclick = () => stepSearch(1);
  $("#searchPrev").onclick = () => stepSearch(-1);
  $("#viewport").addEventListener("scroll", onScroll, { passive: true });
  $("#pages").addEventListener("contextmenu", onContextMenu);
  $("#pages").addEventListener("click", onPageClick);
  $("#pages").addEventListener("mouseover", onPageHover);
  $("#pages").addEventListener("mouseout", onPageHoverOut);
  $("#pop").addEventListener("mouseleave", () => { if ($("#pop").classList.contains("hover-pop")) hidePop(); });
  wireThumbDrag();
  document.addEventListener("mousedown", (e) => {
    if (!e.target.closest("#ctxMenu")) closeMenus();
    if (!e.target.closest("#pop") && !e.target.closest(".mk[data-t]") && !e.target.closest("dialog")) hidePop();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { closeMenus(); hidePop(); }
    if ((e.metaKey || e.ctrlKey) && e.key === "f" && S.sha) { e.preventDefault(); $("#searchInput").focus(); $("#searchInput").select(); }
  });
  let rt;
  addEventListener("resize", () => { clearTimeout(rt); rt = setTimeout(() => { if (S.sha && S.fit) setZoom(fitZoom(), true); }, 150); });
  addEventListener("popstate", route);
  // Drop PDFs anywhere to add them.
  document.addEventListener("dragover", (e) => { if ([...e.dataTransfer.types].includes("Files")) e.preventDefault(); });
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
window.LR = { S, pageText, pageRich, decorate, focusNoun };
