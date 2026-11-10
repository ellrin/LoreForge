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
const KIND_LABEL = { acronym: "縮寫", expansion: "縮寫全名", phrase: "專有名詞", hyphenated: "連字號術語", rare: "少見字", kb: "候選術語" };
const COLOR_LABEL = { yellow: "黃", green: "綠", blue: "藍", pink: "粉紅" };

// ── state ─────────────────────────────────────────────────────────────────
const S = {
  info: null, kb: null,
  sha: null, paper: null,
  docs: new Map(),          // sha → Promise<PDFDocumentProxy>
  texts: new Map(),         // "sha:page" → Promise<{text, items:[{start,end}]}>
  terms: new Map(),         // "sha:page" → Promise<{spans, candidates, kb}>
  orig: [],                 // [{sha, page}] of the open paper
  arr: null,                // pending edited list, or null
  pages: [],                // view items: {sha, page, w, h, el, token, rendered, divs, strs}
  zoom: 1, fit: true, cur: 0,
  annotations: [], quotes: [], chat: [], att: [],
  tab: "notes", termsOn: false,
  search: { q: "", hits: [], i: -1 },
  sending: false,
};
try { S.termsOn = localStorage.getItem("lr.terms") === "1"; S.tab = localStorage.getItem("lr.tab") || "notes"; } catch {}

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
      items.push({ start, end: text.length });
      if (it.hasEOL) text += "\n";
    }
    return { text, items };
  })());
  return S.texts.get(k);
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
    S.att = [];
    await Promise.all([loadAnnotations(), loadQuotes(), loadChat()]);
    await buildView();
    refreshKb();
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
    if (S.termsOn) ensureTerms(p);
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
async function addCardDialog({ title, sha, page, context, hint }) {
  hidePop();
  let kb = S.kb;
  try { kb = S.kb = await api("/api/kb/status"); } catch {}
  updateLegend();
  const t = h("input", { value: title.length > 120 ? title.slice(0, 120) : title });
  const sum = h("textarea", { placeholder: "用自己的話寫一兩句：這是什麼、在這篇論文裡做什麼" });
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
  if (r === "retry") return addCardDialog({ title: t.value, sha, page, context, hint });
  if (r !== true) return;
  try {
    const out = await api("/api/kb/cards", { body: { sha, page, title: t.value, summary: sum.value, initial_state: state, context } });
    toast(`已加入知識庫：${out.node.title}（${state === "known" ? "已掌握" : "學習中"}）`);
    // The new card changes the colouring of every page.
    S.terms.clear();
    S.pages.filter((p) => p.rendered).forEach((p) => S.termsOn && ensureTerms(p));
  } catch (e) { fail(e); }
}

function updateLegend() {
  $("#legend").hidden = !S.termsOn;
  $("#actTerms").setAttribute("aria-pressed", S.termsOn ? "true" : "false");
  $("#kbNote").textContent = S.kb && !S.kb.ok ? "· KnowledgeBase 沒有回應：只標候選術語" : "";
}

function ensureTerms(p) {
  const k = key(p.sha, p.page);
  if (!S.terms.has(k)) S.terms.set(k, pageText(p.sha, p.page).then(({ text }) => api("/api/terms", { body: { text } })));
  S.terms.get(k).then((r) => {
    p.termData = r;
    if (r.kb && !r.kb.ok && S.kb?.ok !== false) { S.kb = { ok: false, error: r.kb.error }; updateLegend(); }
    decorate(p);
  }).catch((e) => { S.terms.delete(k); console.error(e); });
}

/** Re-mark one page's text layer: knowledge colouring, then search hits on top. */
async function decorate(p) {
  if (!p.divs) return;
  const { items } = await pageText(p.sha, p.page);
  const marks = [];
  if (S.termsOn && p.termData) {
    p.termData.spans?.forEach((s, n) => marks.push({ s: s.start, e: s.end, cls: s.state === "known" ? "kt-known" : "kt-learn", t: `s${n}` }));
    p.termData.candidates?.forEach((c, n) => marks.push({ s: c.start, e: c.end, cls: "kt-new", t: `c${n}` }));
  }
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
  const t = mk.dataset.t, n = +t.slice(1);
  const box = mk.getBoundingClientRect();
  const anchor = { left: box.left, top: box.top, bottom: box.bottom };
  const { text } = await pageText(p.sha, p.page);
  if (t[0] === "s") {
    const s = p.termData.spans[n];
    const head = h("h4", {}, s.title, h("span", { class: "st" }, h("i", { class: `s-${s.state}` }), STATE_LABEL[s.state] ?? s.state));
    showPop(anchor, head, h("div", { class: "muted" }, "讀取卡片…"));
    let summary = "";
    try { const v = await api(`/api/kb/nodes/${s.node_id}`); summary = v.node?.summary ?? ""; } catch (err) { summary = err.message; }
    showPop(anchor, head, h("div", { class: "out" }, summary || "（這張卡還沒有摘要）"),
      h("div", { class: "actions" },
        h("button", { type: "button", class: "btn small", onclick: () => translate(s.text, surrounding(text, s.start, s.end), box, null) }, h("span", { "data-icon": "languages" }), "翻譯"),
        h("a", { class: "btn small", href: `${S.info?.kb_url ?? "http://127.0.0.1:7700"}/#/node/${s.node_id}`, target: "_blank", rel: "noopener" }, h("span", { "data-icon": "external" }), "在 KnowledgeBase 開啟")));
  } else {
    const c = p.termData.candidates[n];
    const ctx = surrounding(text, c.start, c.end);
    showPop(anchor,
      h("h4", {}, c.text, h("span", { class: "st" }, h("i", { class: "s-new" }), STATE_LABEL.new)),
      h("div", { class: "src" }, `${KIND_LABEL[c.kind] ?? c.kind} · 由${p.termData.candidate_source === "kb" ? " KnowledgeBase" : "離線規則"}找出`),
      h("div", { class: "actions" },
        h("button", { type: "button", class: "btn small", onclick: () => translate(c.text, ctx, box, { p, context: ctx }) }, h("span", { "data-icon": "languages" }), "翻譯"),
        h("button", { type: "button", class: "btn small primary", onclick: () => addCardDialog({ title: c.text, sha: p.sha, page: p.page, context: ctx }) }, h("span", { "data-icon": "book" }), "加入知識庫")));
  }
}

function toggleTerms() {
  S.termsOn = !S.termsOn;
  try { localStorage.setItem("lr.terms", S.termsOn ? "1" : "0"); } catch {}
  updateLegend();
  S.pages.filter((p) => p.rendered).forEach((p) => (S.termsOn ? ensureTerms(p) : decorate(p)));
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
  if (S.sending || (!text && !S.att.length)) return;
  S.sending = true;
  const att = S.att; S.att = [];
  const pending = { role: "user", text, attachments: att, at: new Date().toISOString(), pending: true };
  S.chat.push(pending);
  ta.value = "";
  renderSide();
  try {
    const r = await api(`/api/papers/${S.sha}/chat`, { body: { text, attachments: att } });
    delete pending.pending;
    S.chat.push(r.reply);
  } catch (e) {
    S.chat.pop(); S.att = att; ta.value = text;
    S.chatError = e.message;
  }
  S.sending = false;
  renderSide();
}

// ── side panel ────────────────────────────────────────────────────────────
function setTab(t) {
  S.tab = t;
  try { localStorage.setItem("lr.tab", t); } catch {}
  renderSide();
}

function renderSide() {
  $$(".tabs [data-tab]").forEach((b) => {
    b.setAttribute("aria-selected", b.dataset.tab === S.tab ? "true" : "false");
    const n = b.dataset.tab === "notes" ? S.annotations.length : b.dataset.tab === "quotes" ? S.quotes.length : 0;
    b.querySelector(".n")?.remove();
    if (n) b.append(h("span", { class: "n" }, n));
  });
  const pane = $("#pane");
  $(".composer")?.remove();
  pane.classList.toggle("chat-pane", S.tab === "chat");
  if (S.tab === "notes") pane.replaceChildren(...notesPane());
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

function chatPane() {
  const head = h("div", { class: "pane-head" }, h("h4", {}, "問 Claude · 對話存在這篇論文的私人資料夾"),
    S.chat.length ? h("button", { class: "ico small", "aria-label": "清除對話", "data-icon": "trash", onclick: async () => { try { await api(`/api/papers/${S.sha}/chat`, { method: "DELETE" }); S.chat = []; renderSide(); } catch (e) { fail(e); } } }) : null);
  const msgs = S.chat.map((m) => h("div", { class: `msg ${m.role}` },
    ...(m.attachments ?? []).map((a) => h("span", { class: "att", title: a.context }, `p.${a.page}「${a.text}」`)),
    m.text,
    m.role === "assistant" && m.engine ? h("span", { class: "eng" }, m.engine) : null,
    m.pending ? h("span", { class: "eng" }, "Claude 思考中…") : null));
  const err = S.chatError ? h("div", { class: "err-box" }, S.chatError) : null;
  S.chatError = null;
  return [head, h("div", { class: "msgs" }, msgs.length ? msgs : [h("p", { class: "empty" }, "選取文字後按右鍵 →「問 Claude」，會附上這段話、頁碼和所在段落。")]), err].filter(Boolean);
}

function composer() {
  const chips = h("div", { class: "chips" }, S.att.map((a, i) => h("span", { class: "attchip", title: a.context }, h("span", {}, `p.${a.page}「${a.text}」`),
    h("button", { class: "ico small", "aria-label": "移除引文", "data-icon": "x", onclick: () => { S.att.splice(i, 1); renderSide(); } }))));
  const ta = h("textarea", { id: "chatInput", placeholder: S.att.length ? "想問這段什麼？" : "問這篇論文的問題（⌘/Ctrl＋Enter 送出）" });
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
  wireThumbDrag();
  document.addEventListener("mousedown", (e) => {
    if (!e.target.closest("#ctxMenu")) closeMenus();
    if (!e.target.closest("#pop") && !e.target.closest(".mk[data-t]")) hidePop();
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
window.LR = { S, pageText, decorate };
