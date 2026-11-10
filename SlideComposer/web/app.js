// SlideComposer editor: state, server sync, slide sorter, canvas interactions
// (select / move / resize / rotate / snap / draw / connect / crop / text editing),
// keyboard shortcuts. Side panels and dialogs are in panels.js.
"use strict";

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const EMU = SC.EMU_PX;
const IN = 914400;
const clone = (x) => structuredClone(x);

const S = {
  info: null, name: null, deck: null, style: null, backdrops: [], status: {},
  cur: null, sel: [], zoom: 1, fit: true, snap: true, grid: false, tool: "select",
  editing: null, editingCell: null, crop: null, latex: new Map(), latexWant: new Set(),
  thumbs: new WeakMap(), clip: null, composing: false, textSel: null,
};

// ── Server ────────────────────────────────────────────────────────────────
async function api(path, opts = {}) {
  const init = { method: opts.method ?? (opts.body !== undefined ? "POST" : "GET"), headers: {} };
  if (opts.body !== undefined) {
    if (opts.raw) init.body = opts.body;
    else { init.body = JSON.stringify(opts.body); init.headers["content-type"] = "application/json"; }
  }
  let r;
  try { r = await fetch(path, init); } catch { throw new Error("連不到 SlideComposer 伺服器，請確認它還在執行。"); }
  const ct = r.headers.get("content-type") ?? "";
  const j = ct.includes("json") ? await r.json() : null;
  if (!r.ok) throw new Error(humanError(j?.error ?? `${r.status} ${r.statusText}`));
  return j;
}

/** Server / OS / pandoc messages → a short Traditional Chinese sentence. */
function humanError(msg) {
  const m = String(msg ?? "");
  const os = (t) => t
    .replace(/No such file or directory \(os error 2\)/, "檔案或資料夾不存在")
    .replace(/Permission denied \(os error 13\)/, "沒有權限讀寫")
    .replace(/File exists \(os error 17\)/, "檔案已存在")
    .replace(/Is a directory \(os error 21\)/, "這是資料夾，不是檔案");
  if (/pandoc not found/i.test(m)) return "找不到 pandoc，無法轉換方程式。請先安裝 pandoc（brew install pandoc）。";
  if (/pandoc failed/i.test(m)) return "方程式轉換失敗：LaTeX 語法可能有誤（例如括號沒有成對）。";
  if (/no math found/i.test(m)) return "方程式轉換失敗：pandoc 沒有產生方程式。";
  let x;
  if ((x = /^(.*\.pptx): (.*)$/i.exec(m))) return `無法讀取 ${x[1]}：${os(x[2])}`;
  if (/^(方程式：)?(檔案讀寫失敗|無法讀取)/.test(m)) return os(m);
  if ((x = /^io: (.*)$/.exec(m))) return `檔案讀寫失敗：${os(x[1])}`;
  if ((x = /^pptx: (.*)$/.exec(m))) return `這個檔案不是可讀取的 PPTX（${x[1]}）`;
  if (/^json: /.test(m)) return "資料格式錯誤，無法讀取。";
  if (/^(4|5)\d\d /.test(m)) return `伺服器回應錯誤（${m}）`;
  return os(m);
}
const deckApi = (p) => `/api/decks/${encodeURIComponent(S.name)}${p}`;

function toast(msg, err = false) {
  $(".toast")?.remove();
  const t = document.createElement("div");
  t.className = "toast" + (err ? " err" : "");
  t.textContent = msg;
  document.body.append(t);
  setTimeout(() => t.remove(), err ? 5000 : 2600);
}

// Every server transaction goes through one queue, in order (a later op never
// races an earlier one; undo waits for pending edits).
let queue = Promise.resolve();
function serial(fn) {
  const p = queue.then(fn, fn);
  queue = p.catch(() => {});
  return p;
}

/** Apply ops on the server (validated, undoable); refresh the slides it returns. */
async function commit(ops, label) {
  flushNudge();
  if (!ops.length) return;
  return serial(async () => {
    try {
      absorb(await api(deckApi("/ops"), { body: { ops, label } }));
    } catch (e) {
      toast(e.message, true);
      renderAll();
    }
  });
}

function absorb(r, { edited = true } = {}) {
  // local frames not yet sent (arrow-key nudges) survive the refresh
  const pending = nudgeBuf.ids.size ? [...nudgeBuf.ids].map((id) => [id, findObj(id, S.deck.slides.find((x) => x.uid === nudgeBuf.uid)?.objects ?? [])?.frame]) : [];
  const byUid = new Map(S.deck.slides.map((s) => [s.uid, s]));
  for (const s of r.slides) byUid.set(s.uid, s);
  S.deck.slides = r.order.map((u) => byUid.get(u)).filter(Boolean);
  if (r.defaults) S.deck.defaults = r.defaults;
  S.status = r.status;
  for (const [id, f] of pending) { const o = f && findObj(id, S.deck.slides.find((x) => x.uid === nudgeBuf.uid)?.objects ?? []); if (o) o.frame = f; }
  if (!S.deck.slides.some((s) => s.uid === S.cur)) S.cur = S.deck.slides[0]?.uid ?? null;
  const sl = slide();
  if (sl) S.sel = S.sel.filter((id) => sl.objects.some((o) => o.id === id));
  rebindDrag();
  if (edited) savedChip();
  else undoButtons();
  if ("file" in r) setFile(r.file);
  renderAll();
}

function savedChip() {
  $("#saveChip").className = "saved";
  undoButtons();
}
function undoButtons() {
  $("#actUndo").disabled = !S.status.undo;
  $("#actRedo").disabled = !S.status.redo;
  $("#actUndo").setAttribute("aria-label", S.status.undo ? `復原「${S.status.undo}」(⌘Z)` : "復原 (⌘Z)");
  $("#actRedo").setAttribute("aria-label", S.status.redo ? `重做「${S.status.redo}」(⇧⌘Z)` : "重做 (⇧⌘Z)");
}

async function undo() { await exitEdit(); flushNudge(); return serial(async () => { try { absorb(await api(deckApi("/undo"), { body: {} })); } catch (e) { toast(e.message, true); } }); }
async function redo() { await exitEdit(); flushNudge(); return serial(async () => { try { absorb(await api(deckApi("/redo"), { body: {} })); } catch (e) { toast(e.message, true); } }); }

// ── The .pptx the deck is bound to: autosave back into it ─────────────────
// The server writes deck.json on every edit; the file itself is written
// after a short pause (debounce), never over a file changed elsewhere.
const FILE = { timer: 0, busy: null, asked: false };
const FILE_LABEL = { saved: "已儲存", pending: "儲存中", saving: "儲存中", changed: "檔案已變更", missing: "找不到檔案", error: "未能儲存" };
function setFile(f) {
  S.file = f ?? null;
  if (S.file?.state !== "changed") $(".ext-bar")?.remove();
  const b = $("#fileState");
  b.hidden = !S.file;
  $("#saveChip").hidden = !!S.file; // the file state next to the name says it all
  if (!S.file) return;
  const st = S.file.state;
  b.className = `fstate ed-only ${st}`;
  b.textContent = "";
  const dot = document.createElement("i");
  b.append(dot, FILE_LABEL[st] ?? "");
  b.title = S.file.path;
  b.setAttribute("aria-label", `${S.file.file}：${FILE_LABEL[st] ?? ""}`);
  if (st === "pending") scheduleWrite();
  if (st === "changed") changedOnDisk();
  else if (st === "missing" && !FILE.asked) askConflict();
}
/** The bound file changed elsewhere: no unsaved edits → reload by itself; else a bar lets the author choose. */
function changedOnDisk() {
  if (!S.file.dirty && !FILE.busy && !S.editing && !S.editingCell) { reloadFile(true); return; }
  if ($(".ext-bar")) return;
  const done = (fn) => async () => { $(".ext-bar")?.remove(); await fn(); };
  const bar = h("div", { class: "ext-bar", role: "alert" },
    h("span", {}, `${S.file.file} 已在別處修改`),
    h("button", { class: "btn small", onclick: done(() => reloadFile()) }, h("span", { "data-icon": "refresh" }), "重新載入"),
    h("button", { class: "btn small primary", onclick: done(() => writeFile(true)) }, h("span", { "data-icon": "save" }), "保留我的版本"));
  $(".top").after(bar);
  paintIcons(bar);
}
let watching = 0;
/** Look at the bound file every few seconds (and when the window gets focus) for changes made elsewhere. */
function watchFile() {
  if (watching) return;
  const look = async () => {
    if (document.hidden || !S.file || !S.name || FILE.looking) return;
    if (!["saved", "changed"].includes(S.file.state)) return;
    FILE.looking = true;
    const name = S.name;
    try {
      const r = await api(`/api/decks/${encodeURIComponent(name)}/file`);
      if (S.name === name && r.file && ["saved", "changed", "missing"].includes(S.file?.state) && (r.file.state !== S.file.state || r.file.dirty !== S.file.dirty)) setFile(r.file);
    } catch {}
    FILE.looking = false;
  };
  watching = setInterval(look, 2500);
  addEventListener("focus", look);
}
function scheduleWrite(ms = 900) {
  clearTimeout(FILE.timer);
  FILE.timer = setTimeout(writeFile, ms);
}
async function writeFile(force = false) {
  clearTimeout(FILE.timer);
  if (!S.file || !S.name) return;
  if (FILE.busy) { await FILE.busy; if (!force && S.file?.state !== "pending") return; }
  const name = S.name;
  setFile({ ...S.file, state: "saving" });
  FILE.busy = api(`/api/decks/${encodeURIComponent(name)}/file/write`, { body: { force } })
    .then((r) => { if (S.name === name) { setFile(r.file); if (r.warning) toast(r.warning, true); } })
    .catch((e) => { if (S.name === name) { setFile({ ...S.file, state: "error" }); toast(e.message, true); } })
    .finally(() => { FILE.busy = null; });
  return FILE.busy;
}
/** Leaving the page inside the debounce: hand the write to the server anyway. */
function flushFileOnLeave() {
  if (!S.file || !S.name || !["pending", "saving"].includes(S.file.state)) return;
  navigator.sendBeacon?.(deckApi("/file/write"), new Blob(["{}"], { type: "application/json" }));
}
/** The file changed elsewhere (or vanished): the author decides. */
function askConflict() {
  FILE.asked = true;
  const missing = S.file.state === "missing";
  dialog((box, close) => {
    const done = (fn) => async () => { close(); FILE.asked = false; await fn(); };
    const btns = [];
    if (!missing) btns.push(h("button", { class: "btn", onclick: done(reloadFile) }, h("span", { "data-icon": "refresh" }), "重新載入"));
    btns.push(h("button", { class: "btn", onclick: done(() => writeFile(true)) }, h("span", { "data-icon": "save" }), missing ? "寫回原位置" : "保留我的版本，覆蓋"));
    btns.push(h("button", { class: "btn primary", onclick: done(saveAs) }, h("span", { "data-icon": "download" }), "另存新檔…"));
    box.append(h("h3", {}, missing ? `找不到 ${S.file.file}` : `${S.file.file} 已在別處修改`), h("div", { class: "info mono" }, S.file.path), h("div", { class: "actions" }, ...btns));
  });
  // Esc / closing keeps everything as is (nothing is written); ask again on the next save
  $("#dlg").addEventListener("close", () => { FILE.asked = false; }, { once: true });
}
async function reloadFile(auto = false) {
  if (FILE.reloading) return;
  FILE.reloading = true;
  try {
    await api(deckApi("/file/reload"), { body: {} });
    await openDeck(S.name);
    if (auto) toast("檔案已在別處修改，已重新載入");
  } catch (e) { toast(e.message, true); } finally { FILE.reloading = false; }
}
async function shellDialog(kind, name) {
  if (S.info?.shell !== "desktop") return undefined;
  const r = await api("/api/shell/dialog", { body: { kind, name } });
  return r.path ?? null;
}
/** 開啟…: a .pptx where it lives (native dialog), bound to its deck. Browser: import a copy. */
async function openFile() {
  S.info ??= await api("/api/info");
  if (S.info.shell !== "desktop") { $("#impFile").click(); return; }
  try {
    const path = await shellDialog("open");
    if (!path) return;
    const r = await api("/api/files/open", { body: { path } });
    openByName(r.name);
  } catch (e) { toast(e.message, true); }
}
/** 另存新檔…: bind the deck to a chosen .pptx and write it there. Browser: download. */
async function saveAs() {
  S.info ??= await api("/api/info");
  const base = S.file?.file?.replace(/\.pptx$/i, "") ?? S.name;
  if (S.info.shell !== "desktop") {
    const a = document.createElement("a");
    a.href = deckApi("/pptx");
    a.download = `${base}.pptx`;
    a.click();
    return;
  }
  try {
    const path = await shellDialog("save", `${base}.pptx`);
    if (!path) return;
    setFile({ ...(S.file ?? { path, file: path.split("/").pop() }), state: "saving" });
    const r = await api(deckApi("/file/save-as"), { body: { path } });
    setFile(r.file);
    if (r.warning) toast(r.warning, true);
  } catch (e) { toast(e.message, true); if (S.file?.state === "saving") setFile({ ...S.file, state: "error" }); }
}

// ── Model helpers ─────────────────────────────────────────────────────────
const slide = () => S.deck?.slides.find((s) => s.uid === S.cur);
const slideNo = (uid) => S.deck.slides.findIndex((s) => s.uid === uid) + 1;
function findObj(id, list = slide()?.objects ?? []) {
  for (const o of list) {
    if (o.id === id) return o;
    if (o.type === "group") { const f = findObj(id, o.children); if (f) return f; }
  }
  return null;
}
function maxId(sl = slide()) {
  let m = 1;
  const walk = (v) => v.forEach((o) => { m = Math.max(m, o.id); if (o.type === "group") walk(o.children); });
  walk(sl.objects);
  return m;
}
const selObjs = () => S.sel.map((id) => findObj(id)).filter(Boolean);
/** DOM element of a slide object on the canvas — only in the slide's own object
 *  layer (layout/backdrop objects reuse the same ids). */
const objEl = (id) => $(`#paper .objects .obj[data-id="${id}"]`);
const setObj = (id, path, value) => ({ op: "set_obj", slide: S.cur, id, path, value });

function tagLook(tag, em, keepSize) {
  const t = S.style?.tags?.[tag];
  if (!t) return null;
  const l = { font: t.font, ea_font: t.ea_font ?? t.font, size: t.size, bold: !!t.bold, color: (t.color ?? "000000").toUpperCase() };
  if (keepSize && t.size_range && keepSize >= t.size_range[0] && keepSize <= t.size_range[1]) l.size = keepSize;
  if (em === "ghost") l.color = S.style.ghost.text_color.toUpperCase();
  else if (em && S.style.emphasis?.[em]) {
    const e = S.style.emphasis[em];
    if (e.color) l.color = e.color.toUpperCase();
    l.bold = l.bold || !!e.bold;
  }
  return l;
}
const defaultTag = () => (S.style?.tags?.body ? "body" : Object.keys(S.style?.tags ?? {})[0]);
/** The style tag whose look matches (empty paragraphs remember their tag only as a look). */
function tagForLook(l) {
  if (!l || !S.style) return defaultTag();
  for (const t of Object.keys(S.style.tags)) {
    const x = tagLook(t);
    if (x && x.size === l.size && x.bold === !!l.bold && x.color === (l.color ?? "").toUpperCase() && (x.font === l.font || x.ea_font === l.ea_font)) return t;
  }
  return defaultTag();
}
function newRun(text, tag = defaultTag(), em = null) {
  return { kind: "text", text, tag, em, look: tagLook(tag, em) ?? { font: "Calibri", size: 14, color: "000000" } };
}
function newPara(runs, tag = defaultTag()) {
  return { align: "l", level: 0, bullet: { kind: "none" }, mar_l: 0, indent: 0, runs, end: tagLook(tag) ?? { font: "Calibri", size: 14, color: "000000" } };
}
function paletteHex() {
  const p = S.style?.palette ?? {};
  const out = [];
  const add = (h, name) => { h = h.toUpperCase(); if (!out.some((x) => x.hex === h)) out.push({ hex: h, name }); };
  for (const [n, h] of Object.entries(p.colors ?? {})) add(h, n);
  for (const [n, t] of Object.entries(p.tints ?? {})) add(t.hex, `${n}：${t.meaning ?? ""}`);
  for (const [n, seq] of Object.entries(p.sequences ?? {})) seq.forEach((h, i) => add(h, `${n}[${i}]`));
  if (!out.length) ["000000", "FFFFFF", "FF0000", "A6A6A6"].forEach((h) => add(h, h));
  return out;
}
/** New shapes use palette colours only: a light tint to fill, the line/ink role to outline. */
function defaultFill() {
  const p = S.style?.palette ?? {};
  const t = p.tints ?? {};
  const h = t.neutral?.hex ?? t.card?.hex ?? Object.values(t)[0]?.hex ?? p.colors?.surface ?? paletteHex().at(-1)?.hex ?? "FFFFFF";
  return { kind: "solid", color: h.toUpperCase() };
}
function luminance(hex) {
  const n = parseInt(hex, 16);
  return 0.2126 * ((n >> 16) & 255) + 0.7152 * ((n >> 8) & 255) + 0.0722 * (n & 255);
}
function inkColor() {
  const c = S.style?.palette?.colors ?? {};
  const h = c.ink ?? c.line ?? [...paletteHex()].sort((a, b) => luminance(a.hex) - luminance(b.hex))[0]?.hex ?? "000000";
  return h.toUpperCase();
}

// ── Rendering ─────────────────────────────────────────────────────────────
function ctxFor(sl, extra = {}) {
  return {
    size: S.deck.size, backdrops: S.backdrops, slideNo: slideNo(sl.uid),
    assetUrl: (a) => `${deckApi("/asset")}?id=${encodeURIComponent(a)}`,
    latex: (omml) => S.latex.get(omml),
    needLatex: (omml) => { if (omml && !S.latex.has(omml)) { S.latexWant.add(omml); queueLatex(); } },
    ...extra,
  };
}

let latexTimer = null;
function queueLatex() {
  clearTimeout(latexTimer);
  latexTimer = setTimeout(async () => {
    const want = [...S.latexWant].filter((m) => !S.latex.has(m));
    S.latexWant.clear();
    if (!want.length) return;
    try {
      const r = await api(deckApi("/latex"), { body: { omml: want } });
      r.results.forEach((x, i) => S.latex.set(want[i], x.latex ?? "\\text{?}"));
      renderCanvas();
      renderThumbs(true);
    } catch (e) { want.forEach((m) => S.latex.set(m, "\\text{(pandoc?)}")); }
  }, 60);
}

function renderAll() {
  renderThumbs();
  renderCanvas();
  renderSide();
  const sl = slide();
  const n = $("#notes");
  if (sl && document.activeElement !== n) n.value = sl.notes ?? "";
  $("#slideCount").textContent = `${S.deck.slides.length} 頁`;
  statusLine();
}

function renderThumbs(force = false) {
  const box = $("#thumbs");
  const W = SC.px(S.deck.size.w);
  const scale = 180 / W;
  const keep = new Map($$(".thumb", box).map((t) => [t.dataset.uid, t]));
  box.textContent = "";
  S.deck.slides.forEach((sl, i) => {
    let t = keep.get(sl.uid);
    const cached = S.thumbs.get(sl);
    if (!t || !cached || force) {
      t = document.createElement("div");
      t.className = "thumb";
      t.dataset.uid = sl.uid;
      t.draggable = true;
      t.innerHTML = `<span class="no"></span><div class="frame"></div>`;
      const node = SC.renderSlide(sl, ctxFor(sl));
      node.style.transform = `scale(${scale})`;
      t.querySelector(".frame").append(node);
      S.thumbs.set(sl, node);
    }
    t.querySelector(".no").textContent = i + 1;
    t.classList.toggle("hidden-slide", !!sl.hidden);
    t.setAttribute("aria-current", sl.uid === S.cur ? "true" : "false");
    box.append(t);
  });
}

function fitZoom() {
  const vp = $("#viewport");
  const W = SC.px(S.deck.size.w), H = SC.px(S.deck.size.h);
  return Math.max(0.2, Math.min((vp.clientWidth - 64) / W, (vp.clientHeight - 64) / H));
}

function renderCanvas() {
  const sl = slide();
  const paper = $("#paper");
  paper.textContent = "";
  if (!sl) return;
  if (S.fit) S.zoom = fitZoom();
  const W = SC.px(S.deck.size.w), H = SC.px(S.deck.size.h);
  const wrap = $("#paperWrap");
  wrap.style.width = `${W * S.zoom}px`;
  wrap.style.height = `${H * S.zoom}px`;
  paper.style.transform = `scale(${S.zoom})`;
  paper.style.width = `${W}px`;
  paper.style.height = `${H}px`;
  const node = SC.renderSlide(sl, ctxFor(sl, { editing: S.editing, editingCell: S.editingCell }));
  paper.append(node);
  if (S.grid) paper.append(Object.assign(document.createElement("div"), { className: "gridlines", style: `background-size:${SC.px(IN / 4)}px ${SC.px(IN / 4)}px` }));
  $("#zoomChip").textContent = `${Math.round(S.zoom * 100)}%`;
  statusLine();
  if (S.editing) mountEditor();
  if (S.editingCell) mountCellEditor();
  renderGuides();
  renderOverlay();
}

/** Screen box (in the overlay's px) of a frame. */
const sx = (emu) => (emu / EMU) * S.zoom;

function renderOverlay(guides = []) {
  const ov = $("#overlay");
  ov.textContent = "";
  const sl = slide();
  if (!sl) return;
  const objs = selObjs().filter((o) => sl.objects.includes(o));
  for (const o of objs) {
    const f = o.frame;
    const b = document.createElement("div");
    b.className = "sel-box";
    Object.assign(b.style, { left: `${sx(f.x)}px`, top: `${sx(f.y)}px`, width: `${sx(f.w)}px`, height: `${sx(f.h)}px`, transform: f.rot ? `rotate(${f.rot}deg)` : "" });
    ov.append(b);
    if (objs.length === 1 && !S.editing) {
      if (o.type === "connector") {
        const [a, c] = connPoints(f);
        const at = [o.start, o.end];
        [a, c].forEach((p, i) => {
          const h = handle(ov, `end${at[i] ? " attached" : ""}`, sx(p[0]), sx(p[1]));
          h.dataset.end = i;
          h.style.cursor = "crosshair";
        });
        // elbow / curved connectors: the middle segment moves (adj1, like PowerPoint's yellow handle)
        if (/^(bent|curved)Connector3$/.test(o.geom)) {
          const [mx, my] = connMid(o);
          const h = handle(ov, "adj", sx(mx), sx(my));
          h.dataset.adj = "adj1";
          h.style.cursor = Math.abs(f.rot % 180) === 90 ? "ns-resize" : "ew-resize";
        }
      } else if (S.crop === o.id) {
        for (const [k, x, y] of cropHandles(o)) { const h = handle(ov, "crop", x, y); h.dataset.crop = k; }
      } else {
        const pts = { nw: [0, 0], n: [0.5, 0], ne: [1, 0], e: [1, 0.5], se: [1, 1], s: [0.5, 1], sw: [0, 1], w: [0, 0.5] };
        for (const [k, [u, v]] of Object.entries(pts)) {
          const [x, y] = rotPt(f, f.x + u * f.w, f.y + v * f.h);
          const h = handle(ov, "", sx(x), sx(y));
          h.dataset.resize = k;
          h.style.cursor = cursorFor(k, f.rot);
        }
        const [rx, ry] = rotPt(f, f.x + f.w / 2, f.y - (22 / S.zoom) * EMU);
        handle(ov, "rot", sx(rx), sx(ry)).dataset.rotate = "1";
      }
    }
  }
  if (objs.length > 1) {
    const b = bbox(objs.map((o) => o.frame));
    const d = document.createElement("div");
    d.className = "sel-box";
    d.style.cssText = `left:${sx(b.x) - 3}px;top:${sx(b.y) - 3}px;width:${sx(b.w) + 6}px;height:${sx(b.h) + 6}px;border-style:dashed`;
    ov.append(d);
  }
  for (const g of guides) {
    const d = document.createElement("div");
    d.className = `guide ${g.v ? "v" : "h"}`;
    if (g.v) d.style.left = `${sx(g.v)}px`; else d.style.top = `${sx(g.h)}px`;
    ov.append(d);
  }
}
function handle(ov, cls, x, y) {
  const h = document.createElement("div");
  h.className = `handle ${cls}`;
  h.style.left = `${x}px`;
  h.style.top = `${y}px`;
  ov.append(h);
  return h;
}
function cursorFor(k, rot = 0) {
  const dirs = ["n", "ne", "e", "se", "s", "sw", "w", "nw"];
  const i = (dirs.indexOf(k) + Math.round(((rot % 360) + 360) % 360 / 45)) % 8;
  return ["ns-resize", "nesw-resize", "ew-resize", "nwse-resize", "ns-resize", "nesw-resize", "ew-resize", "nwse-resize"][i];
}
function rotPt(f, x, y) {
  if (!f.rot) return [x, y];
  const cx = f.x + f.w / 2, cy = f.y + f.h / 2, t = (f.rot * Math.PI) / 180;
  const dx = x - cx, dy = y - cy;
  return [cx + dx * Math.cos(t) - dy * Math.sin(t), cy + dx * Math.sin(t) + dy * Math.cos(t)];
}
function bbox(fs) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const f of fs) { x0 = Math.min(x0, f.x); y0 = Math.min(y0, f.y); x1 = Math.max(x1, f.x + f.w); y1 = Math.max(y1, f.y + f.h); }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}
function connPoints(f) {
  const x0 = f.flip_h ? f.x + f.w : f.x, x1 = f.flip_h ? f.x : f.x + f.w;
  const y0 = f.flip_v ? f.y + f.h : f.y, y1 = f.flip_v ? f.y : f.y + f.h;
  return [rotPt(f, x0, y0), rotPt(f, x1, y1)];
}
/** A point in the frame's own (unrotated, unflipped) 0..1 space → slide space, and back. */
function framePt(f, u, v) {
  const x = f.x + (f.flip_h ? 1 - u : u) * f.w, y = f.y + (f.flip_v ? 1 - v : v) * f.h;
  return rotPt(f, x, y);
}
function frameUV(f, p) {
  const c = [f.x + f.w / 2, f.y + f.h / 2], t = (-(f.rot ?? 0) * Math.PI) / 180;
  const dx = p[0] - c[0], dy = p[1] - c[1];
  const x = c[0] + dx * Math.cos(t) - dy * Math.sin(t), y = c[1] + dx * Math.sin(t) + dy * Math.cos(t);
  let u = f.w ? (x - f.x) / f.w : 0.5, v = f.h ? (y - f.y) / f.h : 0.5;
  if (f.flip_h) u = 1 - u;
  if (f.flip_v) v = 1 - v;
  return [u, v];
}
function connMid(o) {
  return framePt(o.frame, (o.adj?.adj1 ?? 50000) / 100000, 0.5);
}
function connFrame(a, b) {
  return { x: Math.round(Math.min(a[0], b[0])), y: Math.round(Math.min(a[1], b[1])), w: Math.round(Math.abs(b[0] - a[0])), h: Math.round(Math.abs(b[1] - a[1])), flip_h: b[0] < a[0], flip_v: b[1] < a[1] };
}
/** Connection sites (same order as lore-canvas geom::sites). */
/** Vertices of a custom path (0..1 box): one connection site each, same order as lore-canvas geom::path_vertices. */
function pathVertices(path) {
  const toks = path.replace(/,/g, " ").match(/[A-Za-z]|-?[\d.]+(?:e-?\d+)?/g) ?? [];
  const out = [];
  let i = 0, start = null;
  const same = (a, b) => a && b && Math.abs(a[0] - b[0]) < 1e-9 && Math.abs(a[1] - b[1]) < 1e-9;
  while (i < toks.length) {
    const c = toks[i++];
    const pairs = { M: 1, L: 1, Q: 2, C: 3 }[c];
    if (c === "Z" || c === "z") { if (out.length > 1 && same(start, out.at(-1))) out.pop(); continue; }
    if (!pairs) continue;
    let last = [0, 0];
    for (let k = 0; k < pairs; k++) { last = [+toks[i] || 0, +toks[i + 1] || 0]; i += 2; }
    if (c === "M") start = last;
    if (!same(out.at(-1), last)) out.push(last);
  }
  return out;
}
function sites(o) {
  const f = o.frame;
  let pts;
  const verts = o.type !== "connector" && o.geom?.prst === "custom" && o.geom.path ? pathVertices(o.geom.path) : [];
  if (verts.length) {
    pts = verts.map(([u, v]) => [f.x + (f.flip_h ? 1 - u : u) * f.w, f.y + (f.flip_v ? 1 - v : v) * f.h]);
  } else if (o.type !== "connector" && o.geom?.prst === "ellipse") {
    pts = [...Array(8)].map((_, i) => { const a = ((-90 - 45 * i) * Math.PI) / 180; return [f.x + f.w / 2 + (f.w / 2) * Math.cos(a), f.y + f.h / 2 + (f.h / 2) * Math.sin(a)]; });
  } else {
    pts = [[f.x + f.w / 2, f.y], [f.x, f.y + f.h / 2], [f.x + f.w / 2, f.y + f.h], [f.x + f.w, f.y + f.h / 2]];
  }
  return pts.map(([x, y]) => rotPt(f, x, y));
}
/** The connection site of `o` closest to `toward`. */
function closestSite(o, toward) {
  let best = null;
  sites(o).forEach((q, i) => {
    const d = Math.hypot(q[0] - toward[0], q[1] - toward[1]);
    if (!best || d < best.d) best = { d, id: o.id, site: i, p: q };
  });
  return best;
}
/** The site a pointer at `p` means: within 12 px of one, else (`loose`) the nearest site of the
 *  shape the pointer is over, marked `loose` so the other end can re-aim it (centre-facing). */
function nearestSite(p, exclude, loose = false) {
  const tol = (12 / S.zoom) * EMU;
  let best = null;
  const cands = slide().objects.filter((o) => o.id !== exclude && o.type !== "connector" && !o.hidden);
  for (const o of cands) {
    sites(o).forEach((q, i) => {
      const d = Math.hypot(q[0] - p[0], q[1] - p[1]);
      if (d < tol && (!best || d < best.d)) best = { d, id: o.id, site: i, p: q };
    });
  }
  if (best || !loose) return best;
  for (const o of [...cands].reverse()) {
    const [u, v] = frameUV(o.frame, p);
    if (u >= 0 && u <= 1 && v >= 0 && v <= 1) return { ...closestSite(o, p), loose: true };
  }
  return null;
}
/** Both ends of a connector being drawn: loose ends face each other. `a0` = where the start is when unglued. */
function connEnds(start, a0, end, p) {
  let a = start, b = end;
  const centre = (h) => { const f = findObj(h.id).frame; return [f.x + f.w / 2, f.y + f.h / 2]; };
  if (b?.loose) b = closestSite(findObj(b.id), a ? centre(a) : a0);
  if (a?.loose) a = closestSite(findObj(a.id), b ? b.p : p);
  return { a, b, p0: a ? a.p : a0, p: b ? b.p : p };
}
/** Connection sites of the shape under the pointer (the hot one filled). */
function showSites(hot, exclude, only = hot?.id) {
  const ov = $("#overlay");
  $$(".site", ov).forEach((s) => s.remove());
  if (only == null) return;
  for (const o of slide().objects) {
    if (o.id === exclude || o.type === "connector" || o.id !== only) continue;
    sites(o).forEach((q, i) => {
      const d = document.createElement("div");
      d.className = "site" + (hot && hot.id === o.id && hot.site === i && !hot.loose ? " hot" : "");
      d.style.left = `${sx(q[0])}px`;
      d.style.top = `${sx(q[1])}px`;
      ov.append(d);
    });
  }
}
function cropHandles(o) {
  const f = o.frame;
  return [["l", f.x, f.y + f.h / 2], ["r", f.x + f.w, f.y + f.h / 2], ["t", f.x + f.w / 2, f.y], ["b", f.x + f.w / 2, f.y + f.h]].map(([k, x, y]) => [k, sx(x), sx(y)]);
}

// ── Pointer interaction ───────────────────────────────────────────────────
function toEmu(ev) {
  const r = $("#paperWrap").getBoundingClientRect();
  return [((ev.clientX - r.left) / S.zoom) * EMU, ((ev.clientY - r.top) / S.zoom) * EMU];
}
function topObjEl(target) {
  let e = target.closest?.(".objects .obj");
  if (!e) return null;
  while (e.parentElement && !e.parentElement.classList.contains("objects")) {
    const up = e.parentElement.closest(".obj");
    if (!up) break;
    e = up;
  }
  return e;
}

/** Snap candidates from objects not being moved. */
function snapLines(skip) {
  const xs = [0, S.deck.size.w / 2, S.deck.size.w], ys = [0, S.deck.size.h / 2, S.deck.size.h];
  for (const o of slide().objects) {
    if (skip.has(o.id) || o.hidden) continue;
    const f = o.frame;
    xs.push(f.x, f.x + f.w / 2, f.x + f.w);
    ys.push(f.y, f.y + f.h / 2, f.y + f.h);
  }
  return { xs, ys };
}
function snap1(vals, cands, tol) {
  let best = null;
  for (const v of vals) for (const c of cands) {
    const d = c - v;
    if (Math.abs(d) <= tol && (!best || Math.abs(d) < Math.abs(best.d))) best = { d, line: c };
  }
  return best;
}
const gridStep = IN / 8;

let drag = null;

function onPointerDown(ev) {
  if (ev.button !== 0 || !slide()) return;
  const vp = $("#viewport");
  const onPaper = $("#paperWrap").contains(ev.target);
  if (!onPaper && ev.target !== vp) return;
  // a notes / panel field keeps focus otherwise (the canvas prevents default): commit it now
  const ae = document.activeElement;
  if (ae && ae !== document.body && !$("#paper").contains(ae)) ae.blur();
  if (S.editing || S.editingCell) {
    if (ev.target.closest(".obj.editing") || ev.target.closest("td[data-editing]")) return;
    exitEdit();
    return;
  }
  const p = toEmu(ev);
  if (!onPaper) {
    // grey area around the slide: marquee (select tool) or nothing
    if (S.tool !== "select" || poly) return;
    ev.preventDefault();
    S.crop = null;
    startMarquee(ev, p);
    return;
  }
  // handles
  const h = ev.target.closest(".handle");
  if (h) {
    const o = selObjs()[0];
    ev.preventDefault();
    if (h.dataset.resize) drag = { kind: "resize", k: h.dataset.resize, o, f0: { ...o.frame }, p0: p };
    else if (h.dataset.rotate) drag = { kind: "rotate", o, f0: { ...o.frame } };
    else if (h.dataset.end) drag = { kind: "end", end: +h.dataset.end, o, f0: { ...o.frame }, pts: connPoints(o.frame) };
    else if (h.dataset.adj) drag = { kind: "adj", key: h.dataset.adj, o, a0: { ...(o.adj ?? {}) } };
    else if (h.dataset.crop) drag = { kind: "crop", k: h.dataset.crop, o, f0: { ...o.frame }, c0: { ...o.crop }, p0: p };
    capture(ev);
    return;
  }
  // drawing tools
  if (S.tool !== "select") {
    ev.preventDefault();
    if (S.tool === "polygon") { polygonClick(p, ev); return; }
    const conn = /^(line|arrow|conn-)/.test(S.tool);
    const st = conn && S.tool.startsWith("conn") ? nearestSite(p, null, true) : null;
    drag = { kind: "draw", tool: S.tool, p0: st ? st.p : p, a0: p, start: st, p };
    capture(ev);
    return;
  }
  const oel = topObjEl(ev.target);
  if (!oel) { startMarquee(ev, p); return; }
  const id = +oel.dataset.id;
  if (ev.metaKey || ev.ctrlKey) {
    // ⌘-drag: marquee even over objects; ⌘-click (no drag): toggle this object
    ev.preventDefault();
    drag = { kind: "marquee", p0: p, p, add: [...S.sel], toggle: id, live: false };
    capture(ev);
    return;
  }
  if (ev.shiftKey) {
    S.sel = S.sel.includes(id) ? S.sel.filter((x) => x !== id) : [...S.sel, id];
  } else if (!S.sel.includes(id)) {
    S.sel = [id];
    if (S.crop && S.crop !== id) S.crop = null;
  }
  renderOverlay();
  renderSide();
  ev.preventDefault();
  drag = { kind: "move", p0: p, starts: selObjs().filter((o) => slide().objects.includes(o)).map((o) => ({ o, f: { ...o.frame } })), moved: false };
  capture(ev);
}
function startMarquee(ev, p) {
  if (!ev.shiftKey) S.sel = [];
  drag = { kind: "marquee", p0: p, p, add: ev.shiftKey ? [...S.sel] : [], live: true };
  renderOverlay();
  renderSide();
  capture(ev);
}
function capture(ev) {
  try { $("#viewport").setPointerCapture(ev.pointerId); } catch {}
}
/** After a server refresh during a drag: point the drag at the new objects, keep the live frames. */
function rebindDrag() {
  if (!drag) return;
  const re = (o) => {
    const n = o && findObj(o.id);
    if (!n || n === o) return o;
    n.frame = o.frame;
    if (o.crop) n.crop = o.crop;
    return n;
  };
  drag.starts?.forEach((s) => (s.o = re(s.o)));
  if (drag.o) drag.o = re(drag.o);
}

/** Attached connectors follow the objects being dragged (display only, throttled;
 *  the server re-routes them for real in the commit). */
function liveConnectors(d) {
  const sl = slide();
  if (!d.conns) {
    const ids = new Set(d.starts ? d.starts.map((s) => s.o.id) : [d.o.id]);
    d.movedIds = ids;
    d.conns = sl.objects.filter((o) => o.type === "connector" && !ids.has(o.id) && ((o.start && ids.has(o.start.id)) || (o.end && ids.has(o.end.id))));
  }
  if (!d.conns.length) return;
  const now = performance.now();
  if (d.connT && now - d.connT < 32) { clearTimeout(d.connTimer); d.connTimer = setTimeout(() => liveConnectors(d), 34); return; }
  d.connT = now;
  const ctx = ctxFor(sl);
  for (const c of d.conns) {
    let [a, b] = connPoints(c.frame);
    const site = (at) => { const t = sl.objects.find((x) => x.id === at.id); return t ? sites(t)[at.site] : null; };
    if (c.start) a = site(c.start) ?? a;
    if (c.end) b = site(c.end) ?? b;
    const e = objEl(c.id);
    if (e) e.replaceWith(SC.renderObject({ ...c, frame: { ...connFrame(a, b), rot: 0 } }, ctx));
  }
}

function onPointerMove(ev) {
  if (!drag) {
    if (S.tool.startsWith("conn") && slide() && $("#paperWrap").contains(ev.target)) showSites(nearestSite(toEmu(ev), null, true));
    return;
  }
  const p = toEmu(ev);
  const noSnap = ev.altKey || !S.snap;
  const tol = (6 / S.zoom) * EMU;
  if (drag.kind === "move") {
    let dx = p[0] - drag.p0[0], dy = p[1] - drag.p0[1];
    if (!drag.moved && Math.hypot(dx, dy) < (3 / S.zoom) * EMU) return;
    drag.moved = true;
    if (ev.shiftKey) { if (Math.abs(dx) > Math.abs(dy)) dy = 0; else dx = 0; }
    const b = bbox(drag.starts.map((s) => s.f));
    const guides = [];
    if (!noSnap) {
      const L = snapLines(new Set(drag.starts.map((s) => s.o.id)));
      const sxv = snap1([b.x + dx, b.x + dx + b.w / 2, b.x + dx + b.w], L.xs, tol);
      const syv = snap1([b.y + dy, b.y + dy + b.h / 2, b.y + dy + b.h], L.ys, tol);
      if (sxv) { dx += sxv.d; guides.push({ v: sxv.line }); } else if (S.grid) dx = Math.round((b.x + dx) / gridStep) * gridStep - b.x;
      if (syv) { dy += syv.d; guides.push({ h: syv.line }); } else if (S.grid) dy = Math.round((b.y + dy) / gridStep) * gridStep - b.y;
    }
    for (const s of drag.starts) {
      s.o.frame = { ...s.f, x: Math.round(s.f.x + dx), y: Math.round(s.f.y + dy) };
      const e = objEl(s.o.id);
      if (e) { e.style.left = `${SC.px(s.o.frame.x)}px`; e.style.top = `${SC.px(s.o.frame.y)}px`; }
    }
    liveConnectors(drag);
    renderOverlay(guides);
  } else if (drag.kind === "marquee") {
    if (!drag.live) {
      if (Math.hypot(p[0] - drag.p0[0], p[1] - drag.p0[1]) < (3 / S.zoom) * EMU) return;
      drag.live = true;
    }
    drag.p = p;
    const r = rectOf(drag.p0, p);
    const ids = slide().objects.filter((o) => !o.hidden && o.frame.x >= r.x && o.frame.y >= r.y && o.frame.x + o.frame.w <= r.x + r.w && o.frame.y + o.frame.h <= r.y + r.h).map((o) => o.id);
    S.sel = [...new Set([...drag.add, ...ids])];
    renderOverlay();
    const m = document.createElement("div");
    m.className = "marquee";
    Object.assign(m.style, { left: `${sx(r.x)}px`, top: `${sx(r.y)}px`, width: `${sx(r.w)}px`, height: `${sx(r.h)}px` });
    $("#overlay").append(m);
  } else if (drag.kind === "resize") {
    resizeTo(drag, p, ev.shiftKey !== (drag.o.type === "picture" && drag.k.length === 2), noSnap ? 0 : tol);
  } else if (drag.kind === "rotate") {
    const f = drag.f0, cx = f.x + f.w / 2, cy = f.y + f.h / 2;
    let a = (Math.atan2(p[1] - cy, p[0] - cx) * 180) / Math.PI + 90;
    a = ((a % 360) + 360) % 360;
    if (ev.shiftKey || !noSnap) { const s = ev.shiftKey ? 15 : 5; const r = Math.round(a / s) * s; if (ev.shiftKey || Math.abs(r - a) < 2) a = r; }
    drag.o.frame = { ...f, rot: a % 360 };
    liveFrame(drag.o);
    liveConnectors(drag);
  } else if (drag.kind === "adj") {
    const [u] = frameUV(drag.o.frame, p);
    drag.o.adj = { ...drag.a0, [drag.key]: Math.round(u * 100000) };
    liveFrame(drag.o, true);
  } else if (drag.kind === "end") {
    let hot = nearestSite(p, drag.o.id, true);
    if (hot?.loose) hot = closestSite(findObj(hot.id), drag.pts[1 - drag.end]);
    const pts = [...drag.pts];
    pts[drag.end] = hot ? hot.p : p;
    drag.hot = hot;
    drag.o.frame = { ...connFrame(pts[0], pts[1]), rot: 0 };
    liveFrame(drag.o, true);
    showSites(hot, drag.o.id);
  } else if (drag.kind === "draw") {
    let q = p;
    const conn = /^(line|arrow|conn-)/.test(drag.tool);
    let hot = null;
    if (conn) {
      if (drag.tool.startsWith("conn")) {
        hot = nearestSite(p, null, true);
        const e = connEnds(drag.start, drag.a0, hot, p);
        drag.p0 = e.p0;
        q = e.p;
        drag.end = e.b;
        drag.startR = e.a;
      }
      if (ev.shiftKey && !hot) { const dx = q[0] - drag.p0[0], dy = q[1] - drag.p0[1]; if (Math.abs(dx) > Math.abs(dy)) q = [q[0], drag.p0[1]]; else q = [drag.p0[0], q[1]]; }
    }
    drag.p = q;
    renderOverlay();
    if (hot) showSites(hot);
    const d = document.createElement("div");
    d.className = "draft";
    const r = rectOf(drag.p0, q);
    Object.assign(d.style, { left: `${sx(r.x)}px`, top: `${sx(r.y)}px`, width: `${sx(r.w)}px`, height: `${sx(r.h)}px` });
    if (conn) { d.style.border = "0"; d.innerHTML = `<svg width="${sx(r.w) + 2}" height="${sx(r.h) + 2}" style="overflow:visible;position:absolute"><line x1="${sx(drag.p0[0] - r.x)}" y1="${sx(drag.p0[1] - r.y)}" x2="${sx(q[0] - r.x)}" y2="${sx(q[1] - r.y)}" stroke="#3b82f6" stroke-width="2"/></svg>`; }
    $("#overlay").append(d);
  } else if (drag.kind === "crop") {
    cropTo(drag, p);
  }
}

function rectOf(a, b) {
  return { x: Math.min(a[0], b[0]), y: Math.min(a[1], b[1]), w: Math.abs(b[0] - a[0]), h: Math.abs(b[1] - a[1]) };
}

function liveFrame(o, rerender) {
  if (rerender) {
    const e = objEl(o.id);
    if (e) e.replaceWith(SC.renderObject(o, ctxFor(slide())));
  } else {
    const e = objEl(o.id);
    if (e) { const f = o.frame; Object.assign(e.style, { left: `${SC.px(f.x)}px`, top: `${SC.px(f.y)}px`, width: `${SC.px(f.w)}px`, height: `${SC.px(f.h)}px`, transform: f.rot ? `rotate(${f.rot}deg)` : "" }); }
  }
  renderOverlay();
}

function resizeTo(d, p, keepAspect, tol) {
  const f = d.f0, t = ((f.rot ?? 0) * Math.PI) / 180;
  const ddx = p[0] - d.p0[0], ddy = p[1] - d.p0[1];
  // pointer delta in the object's local axes
  let lx = ddx * Math.cos(t) + ddy * Math.sin(t), ly = -ddx * Math.sin(t) + ddy * Math.cos(t);
  let l = -f.w / 2, r = f.w / 2, tp = -f.h / 2, b = f.h / 2;
  if (d.k.includes("w")) l += lx;
  if (d.k.includes("e")) r += lx;
  if (d.k.includes("n")) tp += ly;
  if (d.k.includes("s")) b += ly;
  const guides = [];
  if (tol && !f.rot) {
    // snap the moving edges
    const L = snapLines(new Set([d.o.id]));
    const cx0 = f.x + f.w / 2, cy0 = f.y + f.h / 2;
    if (d.k.includes("w")) { const s = snap1([cx0 + l], L.xs, tol); if (s) { l += s.d; guides.push({ v: s.line }); } }
    if (d.k.includes("e")) { const s = snap1([cx0 + r], L.xs, tol); if (s) { r += s.d; guides.push({ v: s.line }); } }
    if (d.k.includes("n")) { const s = snap1([cy0 + tp], L.ys, tol); if (s) { tp += s.d; guides.push({ h: s.line }); } }
    if (d.k.includes("s")) { const s = snap1([cy0 + b], L.ys, tol); if (s) { b += s.d; guides.push({ h: s.line }); } }
  }
  let w = Math.max(EMU, r - l), h = Math.max(EMU, b - tp);
  if (keepAspect && d.k.length === 2 && f.w && f.h) {
    const ar = f.w / f.h;
    if (w / h > ar) h = w / ar; else w = h * ar;
    if (d.k.includes("w")) l = r - w; else r = l + w;
    if (d.k.includes("n")) tp = b - h; else b = tp + h;
  }
  const mcx = (l + r) / 2, mcy = (tp + b) / 2;
  const cx = f.x + f.w / 2 + mcx * Math.cos(t) - mcy * Math.sin(t);
  const cy = f.y + f.h / 2 + mcx * Math.sin(t) + mcy * Math.cos(t);
  d.o.frame = { ...f, x: Math.round(cx - w / 2), y: Math.round(cy - h / 2), w: Math.round(w), h: Math.round(h) };
  liveFrame(d.o, d.o.type === "group" || d.o.type === "table" || d.o.type === "connector" || d.o.type === "shape" || d.o.type === "text" || d.o.type === "picture");
  liveConnectors(d);
  renderOverlay(guides);
}

function cropTo(d, p) {
  const f = d.f0, c = d.c0;
  const fullW = f.w / (1 - c.l - c.r), fullH = f.h / (1 - c.t - c.b);
  const fullX = f.x - c.l * fullW, fullY = f.y - c.t * fullH;
  const nf = { ...f }, nc = { ...c };
  if (d.k === "l") { const x = Math.min(Math.max(p[0], fullX), f.x + f.w - EMU); nc.l = (x - fullX) / fullW; nf.x = x; nf.w = f.x + f.w - x; }
  if (d.k === "r") { const x = Math.max(Math.min(p[0], fullX + fullW), f.x + EMU); nc.r = (fullX + fullW - x) / fullW; nf.w = x - f.x; }
  if (d.k === "t") { const y = Math.min(Math.max(p[1], fullY), f.y + f.h - EMU); nc.t = (y - fullY) / fullH; nf.y = y; nf.h = f.y + f.h - y; }
  if (d.k === "b") { const y = Math.max(Math.min(p[1], fullY + fullH), f.y + EMU); nc.b = (fullY + fullH - y) / fullH; nf.h = y - f.y; }
  nf.x = Math.round(nf.x); nf.y = Math.round(nf.y); nf.w = Math.round(nf.w); nf.h = Math.round(nf.h);
  d.o.frame = nf;
  d.o.crop = nc;
  liveFrame(d.o, true);
  // ghost of the full image
  const g = document.createElement("img");
  g.className = "crop-ghost";
  g.src = ctxFor(slide()).assetUrl(d.o.asset);
  Object.assign(g.style, { left: `${sx(fullX)}px`, top: `${sx(fullY)}px`, width: `${sx(fullW)}px`, height: `${sx(fullH)}px` });
  $("#overlay").prepend(g);
}

async function onPointerUp(ev) {
  if (!drag) return;
  const d = drag;
  drag = null;
  clearTimeout(d.connTimer);
  $$(".site").forEach((s) => s.remove());
  if (d.kind === "move" && d.moved) {
    await commit(d.starts.map((s) => setObj(s.o.id, "/frame", s.o.frame)), "移動");
  } else if (d.kind === "resize" || d.kind === "rotate") {
    await commit([setObj(d.o.id, "/frame", d.o.frame)], d.kind === "resize" ? "縮放" : "旋轉");
  } else if (d.kind === "end") {
    const key = d.end === 0 ? "start" : "end";
    await commit([setObj(d.o.id, "/frame", d.o.frame), setObj(d.o.id, `/${key}`, d.hot ? { id: d.hot.id, site: d.hot.site } : null)], "調整連接線");
  } else if (d.kind === "adj") {
    await commit([setObj(d.o.id, "/adj", d.o.adj)], "調整連接線中段");
  } else if (d.kind === "crop") {
    await commit([setObj(d.o.id, "/frame", d.o.frame), setObj(d.o.id, "/crop", d.o.crop)], "裁切");
  } else if (d.kind === "marquee") {
    if (!d.live && d.toggle != null) S.sel = S.sel.includes(d.toggle) ? S.sel.filter((x) => x !== d.toggle) : [...S.sel, d.toggle];
    renderOverlay();
    renderSide();
  } else if (d.kind === "draw") {
    await finishDraw(d);
  } else {
    renderOverlay();
  }
}

// ── Creating objects ──────────────────────────────────────────────────────
function baseObj(type, frame, name) {
  return { id: maxId() + 1, name, frame: { ...frame, rot: 0 }, type };
}
function defaultLine() { return { color: inkColor(), width_pt: 1, dash: "solid", head: "none", tail: "none" }; }

async function insertObj(o, label) {
  await commit([{ op: "insert_obj", slide: S.cur, parent: null, index: 9999, obj: o }], label);
  S.sel = [o.id];
  renderOverlay();
  renderSide();
}

async function finishDraw(d) {
  const tool = d.tool;
  const r = rectOf(d.p0, d.p);
  const tiny = r.w < (4 / S.zoom) * EMU && r.h < (4 / S.zoom) * EMU;
  setTool("select");
  if (/^(line|arrow|conn-)/.test(tool)) {
    if (tiny) { renderOverlay(); return; }
    const arrow = S.style?.arrows?.default;
    const line = { color: (arrow?.color ?? inkColor()).toUpperCase(), width_pt: arrow?.width_pt ?? 1.5, dash: "solid", head: "none", tail: tool === "line" ? "none" : arrow?.tail ?? "triangle" };
    const geom = { "conn-elbow": "bentConnector3", "conn-curved": "curvedConnector3" }[tool] ?? "straightConnector1";
    const o = { ...baseObj("connector", connFrame(d.p0, d.p), tool === "line" ? "直線" : "箭頭"), geom, line, start: (d.startR ?? d.start) ? { id: (d.startR ?? d.start).id, site: (d.startR ?? d.start).site } : null, end: d.end ? { id: d.end.id, site: d.end.site } : null };
    if (!o.start) delete o.start;
    if (!o.end) delete o.end;
    await insertObj(o, "插入線條");
    return;
  }
  let f = { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.w), h: Math.round(r.h) };
  if (tool === "text") {
    if (tiny) f = { x: f.x, y: f.y, w: 3 * IN, h: 0.4 * IN };
    const tb = { insets: [91440, 45720, 91440, 45720], anchor: "t", wrap: true, autofit: "shape", paras: [newPara([])] };
    const o = { ...baseObj("text", f, "文字方塊"), geom: { prst: "rect" }, fill: { kind: "none" }, text: tb };
    await insertObj(o, "插入文字方塊");
    enterEdit(o.id);
    return;
  }
  if (tiny) f = { x: f.x, y: f.y, w: 1.5 * IN, h: 1 * IN };
  const names = { rect: "矩形", roundRect: "圓角矩形", ellipse: "橢圓", triangle: "三角形", trapezoid: "梯形" };
  const o = { ...baseObj("shape", f, names[tool] ?? tool), geom: { prst: tool }, fill: defaultFill(), line: defaultLine(),
    text: { insets: [91440, 45720, 91440, 45720], anchor: "ctr", wrap: true, autofit: "none", paras: [{ ...newPara([]), align: "ctr" }] } };
  await insertObj(o, `插入${names[tool] ?? "圖形"}`);
}

let poly = null, polyT = 0;
function polygonClick(p, ev) {
  if (!poly) poly = [];
  // the second click of a double-click (same spot, quickly) ends the shape: it is not a new point
  const last = poly[poly.length - 1], now = performance.now();
  const again = last && now - polyT < 500 && Math.hypot(p[0] - last[0], p[1] - last[1]) < (5 / S.zoom) * EMU;
  polyT = now;
  if (again) { if (poly.length >= 3) finishPolygon(); return; }
  poly.push(p);
  renderOverlay();
  const r = bbox(poly.map(([x, y]) => ({ x, y, w: 0, h: 0 })));
  const d = document.createElement("div");
  d.className = "draft";
  d.style.border = "0";
  d.innerHTML = `<svg style="position:absolute;overflow:visible;left:0;top:0"><polyline points="${poly.map(([x, y]) => `${sx(x)},${sx(y)}`).join(" ")}" fill="none" stroke="#3b82f6" stroke-width="2"/></svg>`;
  $("#overlay").append(d);
  void r;
}
async function finishPolygon() {
  const pts = poly;
  poly = null;
  setTool("select");
  if (!pts || pts.length < 3) return;
  const b = bbox(pts.map(([x, y]) => ({ x, y, w: 0, h: 0 })));
  const path = pts.map(([x, y], i) => `${i ? "L" : "M"}${((x - b.x) / (b.w || 1)).toFixed(4)} ${((y - b.y) / (b.h || 1)).toFixed(4)}`).join(" ") + " Z";
  const o = { ...baseObj("shape", { x: Math.round(b.x), y: Math.round(b.y), w: Math.round(b.w), h: Math.round(b.h) }, "多邊形"), geom: { prst: "custom", path }, fill: defaultFill(), line: defaultLine() };
  await insertObj(o, "插入多邊形");
}

function setTool(t) {
  S.tool = t;
  if (t !== "polygon") poly = null;
  $$(".site").forEach((s) => s.remove());
  $$("[data-tool]").forEach((b) => b.setAttribute("aria-pressed", b.dataset.tool === t ? "true" : "false"));
  $("#paperWrap").style.cursor = t === "select" ? "" : "crosshair";
}

// ── Text editing (contenteditable; IME only while editing) ─────────────────
function textOf(o) {
  return o?.text ?? null;
}
function enterEdit(id) {
  const o = findObj(id);
  if (!o || !o.text) return;
  S.editing = id;
  S.crop = null;
  S.sel = [slide().objects.find((x) => x.id === id) ? id : S.sel[0]].filter(Boolean);
  S.editOrig = clone(o.text);
  S.editBase = clone(o.text);
  renderCanvas();
  renderSide();
}
function editRoot() {
  const e = objEl(S.editing);
  return e ? $(":scope > .tx > .tb", e) : null;
}
function mountEditor() {
  const e = objEl(S.editing);
  const tb = e && $(":scope > .tx > .tb", e);
  if (!tb) { S.editing = null; return; }
  e.classList.add("editing");
  tb.contentEditable = "true";
  tb.spellcheck = false;
  tb.addEventListener("compositionstart", () => (S.composing = true));
  tb.addEventListener("compositionend", () => (S.composing = false));
  tb.addEventListener("keydown", editKey);
  tb.addEventListener("paste", (ev) => {
    ev.preventDefault();
    const t = ev.clipboardData.getData("text/plain");
    document.execCommand("insertText", false, t);
  });
  requestAnimationFrame(() => {
    tb.focus();
    if (S.textSel) { restoreSel(tb, S.textSel); S.textSel = null; }
    else if (S.caretAt) {
      const r = document.caretRangeFromPoint?.(S.caretAt[0], S.caretAt[1]);
      S.caretAt = null;
      if (r && caretOk(tb, r.startContainer)) { const s = getSelection(); s.removeAllRanges(); s.addRange(r); }
      else caretToEnd(tb);
    } else caretToEnd(tb);
  });
}

/** A caret position the IME can compose in: inside a text node of a paragraph
 *  (not in a bullet or equation), or in an empty paragraph. */
function caretOk(root, node) {
  const p = (node.nodeType === 3 ? node.parentElement : node)?.closest?.(".p");
  if (!p || !root.contains(p) || node.parentElement?.closest(".bu, .math")) return false;
  return node.nodeType === 3 || !p.textContent.replace(/\u200b/g, "").length;
}
/** Caret at the end of the last text node of the last paragraph (never directly
 *  in the editing root: an IME composition there leaves a stray character). */
function caretToEnd(root) {
  const ps = paraEls(root);
  const p = ps[ps.length - 1];
  const r = document.createRange();
  if (!p) { r.selectNodeContents(root); r.collapse(false); }
  else {
    const w = document.createTreeWalker(p, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.parentElement.closest(".bu, .math") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT) });
    let last = null;
    for (let n = w.nextNode(); n; n = w.nextNode()) last = n;
    if (last) r.setStart(last, last.data.length);
    else {
      // empty paragraph: before its placeholder <br>
      const kids = [...p.childNodes];
      const br = kids.findIndex((c) => c.nodeName === "BR");
      r.setStart(p, br >= 0 ? br : kids.length);
    }
    r.collapse(true);
  }
  const s = getSelection();
  s.removeAllRanges();
  s.addRange(r);
}

/** Offsets in a paragraph element: text chars, math/br = 1, bullets = 0. */
function walkPara(pEl, fn) {
  let off = 0;
  const visit = (n) => {
    if (n.nodeType === 3) { if (fn(n, off, n.data.length) === false) return false; off += n.data.length; return; }
    if (n.nodeType !== 1) return;
    if (n.classList.contains("bu")) return;
    if (n.classList.contains("math") || n.tagName === "BR") {
      const isPlaceholder = n.tagName === "BR" && n === pEl.lastChild && !pEl.textContent.length && !$(".math", pEl);
      if (isPlaceholder) return;
      if (fn(n, off, 1) === false) return false;
      off += 1;
      return;
    }
    for (const c of [...n.childNodes]) if (visit(c) === false) return false;
  };
  for (const c of [...pEl.childNodes]) if (visit(c) === false) break;
  return off;
}
function paraEls(root) { return [...root.children].filter((c) => c.nodeType === 1); }
function posOf(root, node, offset) {
  const ps = paraEls(root);
  let pi = ps.findIndex((p) => p === node || p.contains(node));
  if (pi < 0) {
    if (node === root) { pi = Math.min(offset, ps.length - 1); return { p: Math.max(0, pi), o: offset >= ps.length ? walkPara(ps[ps.length - 1], () => {}) : 0 }; }
    return { p: 0, o: 0 };
  }
  const pEl = ps[pi];
  let res = null;
  if (node === pEl) {
    // offset counts child nodes
    let o = 0;
    walkPara(pEl, (n, off, len) => { const top = [...pEl.childNodes].indexOf(n.parentElement === pEl ? n : n); if (top >= 0 && top < offset) o = off + len; });
    return { p: pi, o };
  }
  walkPara(pEl, (n, off, len) => {
    if (n === node) { res = off + (n.nodeType === 3 ? Math.min(offset, len) : 0); return false; }
    if (n.nodeType === 1 && n.contains(node)) { res = off; return false; }
  });
  return { p: pi, o: res ?? walkPara(pEl, () => {}) };
}
function getTextSel(root) {
  const s = getSelection();
  if (!s.rangeCount || !root.contains(s.anchorNode)) return null;
  const r = s.getRangeAt(0);
  const a = posOf(root, r.startContainer, r.startOffset), b = posOf(root, r.endContainer, r.endOffset);
  return { a, b };
}
function restoreSel(root, sel) {
  const ps = paraEls(root);
  const point = ({ p, o }) => {
    const pEl = ps[Math.min(p, ps.length - 1)];
    if (!pEl) return [root, 0];
    let res = null;
    walkPara(pEl, (n, off, len) => {
      if (o <= off + len && n.nodeType === 3) { res = [n, o - off]; return false; }
      if (o <= off && n.nodeType === 1) { res = [n.parentNode, [...n.parentNode.childNodes].indexOf(n)]; return false; }
    });
    return res ?? [pEl, pEl.childNodes.length];
  };
  try {
    const r = document.createRange();
    r.setStart(...point(sel.a));
    r.setEnd(...point(sel.b));
    const s = getSelection(); s.removeAllRanges(); s.addRange(r);
  } catch {}
}

/** Read the edited DOM back into a TextBody (keeps run/paragraph provenance). */
function domToText(root, orig) {
  const tb = clone(orig);
  tb.paras = [];
  for (const pEl of paraEls(root)) {
    const pi = pEl.dataset.pi !== undefined ? +pEl.dataset.pi : orig.paras.length - 1;
    const base = clone(orig.paras[pi] ?? orig.paras[orig.paras.length - 1] ?? newPara([]));
    const para = { ...base, runs: [] };
    if (pEl.dataset.override) Object.assign(para, JSON.parse(pEl.dataset.override));
    let last = base.runs.find((r) => r.kind === "text") ?? null;
    const push = (r) => {
      const prev = para.runs[para.runs.length - 1];
      if (prev && prev.kind === "text" && r.kind === "text" && prev.tag === r.tag && prev.em === r.em && JSON.stringify(prev.look) === JSON.stringify(r.look) && JSON.stringify(prev.src) === JSON.stringify(r.src)) prev.text += r.text;
      else para.runs.push(r);
    };
    const visit = (n, inherit) => {
      if (n.nodeType === 3) {
        const t = n.data.replace(/​/g, "");
        if (!t) return;
        const r = inherit ? clone(inherit) : last ? clone(last) : newRun("", tagForLook(para.end));
        r.kind = "text"; r.text = t;
        delete r.field;
        push(r);
        return;
      }
      if (n.nodeType !== 1 || n.classList.contains("bu")) return;
      if (n.tagName === "BR") {
        const isPlaceholder = n === pEl.lastChild && !pEl.textContent.replace(/​/g, "").length && !$(".math", pEl);
        if (isPlaceholder) return;
        const src = n.dataset.pi !== undefined ? orig.paras[+n.dataset.pi]?.runs[+n.dataset.ri] : null;
        push(src?.kind === "br" ? clone(src) : { kind: "br", text: "", tag: last?.tag ?? defaultTag(), look: clone(last?.look ?? para.end) });
        return;
      }
      if (n.classList.contains("math")) {
        const src = orig.paras[+n.dataset.pi]?.runs[+n.dataset.ri];
        if (src) push(clone(src));
        return;
      }
      if (n.dataset?.pi !== undefined && n.dataset.ri !== undefined) {
        const src = orig.paras[+n.dataset.pi]?.runs[+n.dataset.ri];
        if (src) {
          const r = clone(src);
          if (n.dataset.tag !== undefined || n.dataset.em !== undefined) {
            r.tag = n.dataset.tag ?? r.tag;
            r.em = n.dataset.em === "" ? null : n.dataset.em ?? r.em;
            r.look = tagLook(r.tag, r.em, r.look.size) ?? r.look;
          }
          if (r.kind === "field") { r.text = n.textContent; push(r); return; }
          last = r;
          for (const c of [...n.childNodes]) visit(c, r);
          return;
        }
      }
      for (const c of [...n.childNodes]) visit(c, inherit);
    };
    for (const c of [...pEl.childNodes]) visit(c, null);
    if (!para.end) para.end = clone(last?.look ?? newPara([]).end);
    tb.paras.push(para);
  }
  if (!tb.paras.length) tb.paras.push(newPara([]));
  return tb;
}

async function exitEdit() {
  if (S.editingCell) return exitCellEdit();
  if (!S.editing) return;
  const id = S.editing;
  const root = editRoot();
  const o = findObj(id);
  let next = null;
  if (root && o) next = domToText(root, S.editOrig);
  S.editing = null;
  if (next && JSON.stringify(next) !== JSON.stringify(S.editBase)) {
    o.text = next;
    await commit([setObj(id, "/text", next)], "編輯文字");
  } else {
    renderCanvas();
  }
  renderSide();
}

/** While editing: sync the DOM to the model, change it, re-mount, keep the selection. */
function editTransform(fn) {
  const root = editRoot();
  if (!root) return false;
  const sel = getTextSel(root) ?? { a: { p: 0, o: 0 }, b: { p: 0, o: 0 } };
  const o = findObj(S.editing);
  const tb = domToText(root, S.editOrig);
  fn(tb, sel);
  // the transformed body becomes the new editing base (so spans map 1:1 to runs)
  o.text = tb;
  S.editOrig = clone(tb);
  S.textSel = sel;
  renderCanvas();
  return true;
}

/** Split runs so [a, b) is covered exactly by whole runs; returns those runs. */
function runsInRange(tb, a, b) {
  const out = [];
  for (let p = a.p; p <= b.p; p++) {
    const para = tb.paras[p];
    if (!para) continue;
    const from = p === a.p ? a.o : 0, to = p === b.p ? b.o : Infinity;
    const runs = [];
    let off = 0;
    for (const r of para.runs) {
      const len = r.kind === "text" || r.kind === "field" ? r.text.length : 1;
      const s = Math.max(from, off), e = Math.min(to, off + len);
      if (r.kind === "text" && e > s && (s > off || e < off + len)) {
        const pre = { ...clone(r), text: r.text.slice(0, s - off) };
        const mid = { ...clone(r), text: r.text.slice(s - off, e - off) };
        const post = { ...clone(r), text: r.text.slice(e - off) };
        if (pre.text) runs.push(pre);
        runs.push(mid); out.push(mid);
        if (post.text) runs.push(post);
      } else {
        runs.push(r);
        if (e > s || (from === to && false)) out.push(r);
      }
      off += len;
    }
    para.runs = runs;
  }
  return out;
}

function selectedParas(tb, sel) {
  const out = [];
  for (let p = sel.a.p; p <= sel.b.p; p++) if (tb.paras[p]) out.push(tb.paras[p]);
  return out;
}

function editKey(ev) {
  if (S.composing) return;
  if (ev.key === "Escape") { ev.preventDefault(); exitEdit(); return; }
  if (ev.key === "Tab") {
    ev.preventDefault();
    editTransform((tb, sel) => selectedParas(tb, sel).forEach((p) => setLevel(p, Math.max(0, Math.min(1, (p.level ?? 0) + (ev.shiftKey ? -1 : 1))))));
  }
  if ((ev.metaKey || ev.ctrlKey) && ["b", "i", "u"].includes(ev.key.toLowerCase())) { ev.preventDefault(); formatKey(ev.key.toLowerCase()); }
}
/** ⌘B toggles the strong emphasis tag; ⌘I / ⌘U have no free formatting. */
function formatKey(k) {
  if (k === "b" && S.style?.emphasis?.strong) {
    const t = textInfo();
    applyEm("strong", !(t.ems.length === 1 && t.ems[0] === "strong"));
  } else toast(k === "b" ? "這個風格包沒有粗體強調標籤。" : "格式只能用標籤與強調設定（⌘B＝粗體強調）。");
}

/** Bullet characters per level (level 2 uses the style's dash). */
const BULLET_CH = ["•", "–"];
function setLevel(p, lvl) {
  p.level = lvl;
  if (p.bullet?.kind === "char" && BULLET_CH.includes(p.bullet.ch ?? "•")) p.bullet = { ...p.bullet, ch: BULLET_CH[lvl ? 1 : 0] };
  if (p.bullet?.kind !== "none") { p.mar_l = lvl ? 685800 : 228600; p.indent = -228600; }
  else { p.mar_l = lvl ? 457200 : 0; p.indent = 0; }
}

// Table cells
function mountCellEditor() {
  const { id, r, c } = S.editingCell;
  const td = $(`td[data-r="${r}"][data-c="${c}"]`, objEl(id) ?? document.createElement("div"));
  if (!td) { S.editingCell = null; return; }
  td.dataset.editing = "1";
  const tb = $(".tb", td);
  tb.contentEditable = "true";
  tb.addEventListener("compositionstart", () => (S.composing = true));
  tb.addEventListener("compositionend", () => (S.composing = false));
  tb.addEventListener("keydown", (ev) => {
    if (S.composing) return;
    if (ev.key === "Escape") { ev.preventDefault(); exitCellEdit(); }
    if (ev.key === "Tab") { ev.preventDefault(); exitCellEdit(ev.shiftKey ? -1 : 1); }
  });
  objEl(id).classList.add("editing");
  requestAnimationFrame(() => { tb.focus(); caretToEnd(tb); });
}
async function exitCellEdit(move = 0) {
  const ec = S.editingCell;
  if (!ec) return;
  const o = findObj(ec.id);
  const td = objEl(ec.id)?.querySelector(`td[data-r="${ec.r}"][data-c="${ec.c}"]`);
  const orig = o.rows[ec.r].cells[ec.c].text;
  const next = td ? domToText($(".tb", td), orig) : orig;
  S.editingCell = null;
  if (JSON.stringify(next) !== JSON.stringify(orig)) {
    o.rows[ec.r].cells[ec.c].text = next;
    await commit([setObj(ec.id, `/rows/${ec.r}/cells/${ec.c}/text`, next)], "編輯儲存格");
  } else renderCanvas();
  if (move) {
    const cells = [];
    o.rows.forEach((row, r) => row.cells.forEach((c, ci) => { if (!c.hmerge && !c.vmerge) cells.push([r, ci]); }));
    const i = cells.findIndex(([r, c]) => r === ec.r && c === ec.c) + move;
    if (cells[i]) { S.editingCell = { id: ec.id, r: cells[i][0], c: cells[i][1] }; renderCanvas(); }
  }
}

function onDblClick(ev) {
  if (S.tool === "polygon") { finishPolygon(); return; }
  // pointer capture retargets the event to the viewport: hit-test again
  const target = document.elementFromPoint(ev.clientX, ev.clientY) ?? ev.target;
  if (S.editing || S.editingCell || !$("#paperWrap").contains(target)) return;
  // an empty image slot: pick a picture for it
  const slotEl = topObjEl(target);
  if (slotEl?.classList.contains("slot-empty") && slotEl.dataset.slot === "image") {
    const o = findObj(+slotEl.dataset.id);
    const pick = $("#imgFile");
    pick.dataset.slot = o.id;
    pick.click();
    return;
  }
  const td = target.closest(".obj.t-table td");
  if (td && !S.editing) {
    const id = +td.closest(".obj.t-table").dataset.id;
    S.sel = [id];
    S.editingCell = { id, r: +td.dataset.r, c: +td.dataset.c };
    renderCanvas();
    return;
  }
  // innermost object with text (group children included)
  let e = target.closest(".obj");
  while (e && !findObj(+e.dataset.id)?.text) e = e.parentElement?.closest(".obj");
  if (e) {
    const o = findObj(+e.dataset.id);
    if (o.equation) { openEquation(o); return; }
    S.caretAt = [ev.clientX, ev.clientY];
    enterEdit(o.id);
    return;
  }
  // crop: top-level pictures only (group children cannot be resized on their own yet)
  const pe = target.closest(".obj.t-picture");
  if (pe && slide().objects.some((o) => o.id === +pe.dataset.id)) { S.crop = +pe.dataset.id; S.sel = [S.crop]; renderOverlay(); renderSide(); }
}

// ── Clipboard, keyboard ───────────────────────────────────────────────────
function freshIds(objs, start) {
  let n = start;
  const map = (o) => { o.id = ++n; if (o.type === "group") o.children.forEach(map); };
  objs.forEach(map);
  return objs;
}
function copySel() {
  const objs = selObjs();
  if (!objs.length) return;
  S.clip = { objs: clone(objs), from: S.cur };
  try { navigator.clipboard?.writeText(JSON.stringify({ slidecomposer: S.clip.objs })); } catch {}
  toast(`已複製 ${objs.length} 個物件`);
}
async function paste(offset = true) {
  if (!S.clip) return;
  const objs = freshIds(clone(S.clip.objs), maxId());
  const d = offset && S.clip.from === S.cur ? IN / 10 : 0;
  for (const o of objs) { o.frame.x += d; o.frame.y += d; delete o.start; delete o.end; }
  await commit(objs.map((o) => ({ op: "insert_obj", slide: S.cur, parent: null, index: 9999, obj: o })), "貼上");
  S.sel = objs.map((o) => o.id);
  renderOverlay(); renderSide();
}
async function duplicate() {
  const objs = selObjs();
  if (!objs.length) return;
  S.clip = { objs: clone(objs), from: S.cur };
  await paste(true);
}
async function deleteSel() {
  if (!S.sel.length) return;
  const ids = S.sel;
  S.sel = [];
  await commit(ids.map((id) => ({ op: "delete_obj", slide: S.cur, id })), "刪除");
}
async function zorder(how) {
  const sl = slide();
  const ids = S.sel.filter((id) => sl.objects.some((o) => o.id === id));
  if (!ids.length) return;
  const idx = (id) => sl.objects.findIndex((o) => o.id === id);
  const n = sl.objects.length;
  const ops = [];
  const sorted = [...ids].sort((a, b) => idx(a) - idx(b));
  if (how === "front") sorted.forEach((id) => ops.push({ op: "move_obj", slide: S.cur, id, index: n - 1 }));
  if (how === "back") [...sorted].reverse().forEach((id) => ops.push({ op: "move_obj", slide: S.cur, id, index: 0 }));
  if (how === "forward") [...sorted].reverse().forEach((id) => ops.push({ op: "move_obj", slide: S.cur, id, index: Math.min(n - 1, idx(id) + 1) }));
  if (how === "backward") sorted.forEach((id) => ops.push({ op: "move_obj", slide: S.cur, id, index: Math.max(0, idx(id) - 1) }));
  await commit(ops, "圖層順序");
}
// Arrow-key nudges move the objects locally at once; a burst of presses (gaps
// under NUDGE_MS) is sent as one transaction = one undo step.
const NUDGE_MS = 450;
const nudgeBuf = { uid: null, ids: new Set(), timer: 0 };
function nudge(dx, dy) {
  const sl = slide();
  const objs = selObjs().filter((o) => sl.objects.includes(o));
  if (!objs.length) return;
  if (nudgeBuf.uid && nudgeBuf.uid !== S.cur) flushNudge();
  nudgeBuf.uid = S.cur;
  const d = { o: objs[0], starts: objs.map((o) => ({ o })) };
  for (const o of objs) {
    o.frame = { ...o.frame, x: Math.round(o.frame.x + dx), y: Math.round(o.frame.y + dy) };
    nudgeBuf.ids.add(o.id);
    const e = objEl(o.id);
    if (e) { e.style.left = `${SC.px(o.frame.x)}px`; e.style.top = `${SC.px(o.frame.y)}px`; }
  }
  liveConnectors(d);
  renderOverlay();
  clearTimeout(nudgeBuf.timer);
  nudgeBuf.timer = setTimeout(flushNudge, NUDGE_MS);
}
function flushNudge() {
  clearTimeout(nudgeBuf.timer);
  if (!nudgeBuf.ids.size) return;
  const uid = nudgeBuf.uid, ids = [...nudgeBuf.ids];
  nudgeBuf.ids = new Set();
  nudgeBuf.uid = null;
  const objs = S.deck.slides.find((x) => x.uid === uid)?.objects ?? [];
  const ops = ids.map((id) => findObj(id, objs)).filter(Boolean).map((o) => ({ op: "set_obj", slide: uid, id: o.id, path: "/frame", value: o.frame }));
  return commit(ops, "微調位置");
}
async function group() {
  if (S.sel.length < 2) return;
  const id = maxId() + 1;
  await commit([{ op: "group", slide: S.cur, ids: S.sel, id }], "群組");
  S.sel = [id];
  renderOverlay(); renderSide();
}
async function ungroup() {
  const o = selObjs()[0];
  if (!o || o.type !== "group") return;
  const kids = o.children.map((c) => c.id);
  await commit([{ op: "ungroup", slide: S.cur, id: o.id }], "取消群組");
  S.sel = kids;
  renderOverlay(); renderSide();
}

function onKey(ev) {
  const t = ev.target;
  if (S.editing || S.editingCell) return;
  if (t.closest?.("input, textarea, select, [contenteditable='true'], dialog")) return;
  const mod = ev.metaKey || ev.ctrlKey;
  const k = ev.key;
  if (mod && k === "Enter") { ev.preventDefault(); startShow(!ev.shiftKey); return; }
  if (S.focus === "slides" && slideKey(ev, mod, k)) return;
  if (mod && k.toLowerCase() === "z") { ev.preventDefault(); ev.shiftKey ? redo() : undo(); return; }
  if (mod && k.toLowerCase() === "y") { ev.preventDefault(); redo(); return; }
  if (mod && k === "c") { ev.preventDefault(); copySel(); return; }
  if (mod && k === "x") { ev.preventDefault(); copySel(); deleteSel(); return; }
  if (mod && k === "v") { ev.preventDefault(); paste(); return; }
  if (mod && k === "d") { ev.preventDefault(); duplicate(); return; }
  if (mod && k === "a") { ev.preventDefault(); S.sel = slide().objects.map((o) => o.id); renderOverlay(); renderSide(); return; }
  if (mod && ["b", "i", "u"].includes(k.toLowerCase())) { ev.preventDefault(); if (textTargets().length) formatKey(k.toLowerCase()); return; }
  if (mod && k.toLowerCase() === "g") { ev.preventDefault(); ev.shiftKey ? ungroup() : group(); return; }
  if (mod && (k === "]" || k === "}")) { ev.preventDefault(); zorder(ev.shiftKey ? "front" : "forward"); return; }
  if (mod && (k === "[" || k === "{")) { ev.preventDefault(); zorder(ev.shiftKey ? "back" : "backward"); return; }
  if (mod && k.toLowerCase() === "s") { ev.preventDefault(); if (ev.shiftKey) { saveAs(); return; } if (S.file) { writeFile(); return; } api(deckApi("/save"), { body: {} }).then(() => toast("已儲存")); return; }
  if (mod && k.toLowerCase() === "o") { ev.preventDefault(); openFile(); return; }
  if (k === "Delete" || k === "Backspace") { if (S.sel.length) { ev.preventDefault(); deleteSel(); } return; }
  if (k === "Escape") { if (poly) { poly = null; setTool("select"); renderOverlay(); return; } S.crop = null; S.sel = []; setTool("select"); renderOverlay(); renderSide(); return; }
  if (k === "Enter" && S.crop) { S.crop = null; renderOverlay(); return; }
  if (k === "Enter" && poly) { finishPolygon(); return; }
  if (k === "Enter" && S.sel.length === 1) { const o = selObjs()[0]; if (o.text) { ev.preventDefault(); enterEdit(o.id); } return; }
  const step = ev.shiftKey ? IN / 10 : IN / 100;
  const arrows = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
  if (arrows[k] && S.sel.length) { ev.preventDefault(); nudge(...arrows[k]); return; }
  if (!S.sel.length && (k === "ArrowDown" || k === "PageDown" || k === "ArrowUp" || k === "PageUp")) {
    const i = S.deck.slides.findIndex((s) => s.uid === S.cur) + (k === "ArrowDown" || k === "PageDown" ? 1 : -1);
    if (S.deck.slides[i]) gotoSlide(S.deck.slides[i].uid);
    ev.preventDefault();
    return;
  }
  if (!mod && k === "v") setTool("select");
  if (!mod && k === "t") setTool("text");
}

// ── Slides ────────────────────────────────────────────────────────────────
/** Keys go to the slide list after a click there, to objects after a click on the canvas. */
function setFocus(where) {
  S.focus = where;
  $("#thumbs").classList.toggle("focus", where === "slides");
}
/** Slide-list keys: copy / cut / paste / duplicate / delete slides, Enter = new slide. */
function slideKey(ev, mod, k) {
  const key = k.toLowerCase();
  if (mod && key === "c") { ev.preventDefault(); copySlide(); return true; }
  if (mod && key === "x") { ev.preventDefault(); copySlide(); delSlide(); return true; }
  if (mod && key === "v" && S.clip?.slides) { ev.preventDefault(); pasteSlides(); return true; }
  if (mod && key === "d") { ev.preventDefault(); dupSlide(); return true; }
  if (!mod && k === "Enter") { ev.preventDefault(); addSlide(); return true; }
  if (!mod && (k === "Delete" || k === "Backspace")) { ev.preventDefault(); delSlide(); return true; }
  if (!mod && (k === "ArrowDown" || k === "ArrowUp" || k === "PageDown" || k === "PageUp")) {
    const i = S.deck.slides.findIndex((x) => x.uid === S.cur) + (k === "ArrowDown" || k === "PageDown" ? 1 : -1);
    if (S.deck.slides[i]) gotoSlide(S.deck.slides[i].uid);
    ev.preventDefault();
    return true;
  }
  return false;
}
function copySlide() {
  const sl = slide();
  if (!sl) return;
  S.clip = { slides: [clone(sl)] };
  toast("已複製投影片");
}
async function pasteSlides() {
  const i = S.deck.slides.findIndex((x) => x.uid === S.cur);
  const copies = S.clip.slides.map((sl) => ({ ...clone(sl), uid: nextUid() }));
  await commit(copies.map((c, n) => ({ op: "insert_slide", index: i + 1 + n, slide: c })), "貼上投影片");
  gotoSlide(copies.at(-1).uid);
}
function gotoSlide(uid) {
  if (document.activeElement === $("#notes")) $("#notes").blur(); // commits the notes of the slide they belong to
  flushNudge();
  if (S.editing || S.editingCell) exitEdit();
  S.cur = uid;
  S.sel = [];
  S.crop = null;
  const u = new URL(location.href);
  u.searchParams.set("s", uid);
  history.replaceState(null, "", u);
  renderAll();
  $(`.thumb[data-uid="${uid}"]`)?.scrollIntoView({ block: "nearest" });
}
async function addSlide() {
  // the deck's default arrangement for content pages, laid out right away (00 版型)
  await addArrangedSlide("content");
}
function nextUid() {
  let n = S.deck.next_uid ?? 1;
  for (const s of S.deck.slides) { const m = /^s(\d+)$/.exec(s.uid); if (m) n = Math.max(n, +m[1] + 1); }
  S.deck.next_uid = n + 1;
  return `s${n}`;
}
async function dupSlide() {
  const sl = slide();
  const copy = clone(sl);
  copy.uid = nextUid();
  const i = S.deck.slides.indexOf(sl);
  await commit([{ op: "insert_slide", index: i + 1, slide: copy }], "複製投影片");
  gotoSlide(copy.uid);
}
async function delSlide() {
  if (S.deck.slides.length <= 1) return toast("至少要留一頁", true);
  const i = S.deck.slides.findIndex((s) => s.uid === S.cur);
  const uid = S.cur;
  S.cur = S.deck.slides[i + 1]?.uid ?? S.deck.slides[i - 1].uid;
  await commit([{ op: "delete_slide", slide: uid }], "刪除投影片");
}
async function hideSlide() {
  const sl = slide();
  await commit([{ op: "set_slide", slide: sl.uid, path: "/hidden", value: !sl.hidden }], sl.hidden ? "顯示投影片" : "隱藏投影片");
}

function wireSorter() {
  const box = $("#thumbs");
  box.addEventListener("click", (e) => { const t = e.target.closest(".thumb"); setFocus("slides"); if (t) gotoSlide(t.dataset.uid); });
  let dragUid = null;
  box.addEventListener("dragstart", (e) => { dragUid = e.target.closest(".thumb")?.dataset.uid; e.dataTransfer.effectAllowed = "move"; });
  box.addEventListener("dragover", (e) => {
    e.preventDefault();
    $$(".thumb", box).forEach((t) => t.classList.remove("drop-before"));
    e.target.closest(".thumb")?.classList.add("drop-before");
  });
  box.addEventListener("drop", async (e) => {
    e.preventDefault();
    const t = e.target.closest(".thumb");
    $$(".thumb", box).forEach((x) => x.classList.remove("drop-before"));
    if (!t || !dragUid || t.dataset.uid === dragUid) return;
    const from = S.deck.slides.findIndex((s) => s.uid === dragUid);
    let to = S.deck.slides.findIndex((s) => s.uid === t.dataset.uid);
    if (from < to) to -= 1;
    await commit([{ op: "move_slide", slide: dragUid, index: to }], "移動投影片");
  });
}

// ── Images ────────────────────────────────────────────────────────────────
async function insertImageFile(file) {
  const ext = (file.name.split(".").pop() || "png").toLowerCase();
  try {
    const r = await api(`${deckApi("/assets")}?ext=${encodeURIComponent(ext)}`, { body: await file.arrayBuffer(), raw: true });
    const img = new Image();
    img.src = ctxFor(slide()).assetUrl(r.asset);
    await img.decode();
    const maxW = S.deck.size.w * 0.6, maxH = S.deck.size.h * 0.6;
    let w = img.naturalWidth * EMU, h = img.naturalHeight * EMU;
    const k = Math.min(1, maxW / w, maxH / h);
    w *= k; h *= k;
    const [x, y] = freeSpot(Math.round((S.deck.size.w - w) / 2), Math.round((S.deck.size.h - h) / 2), w, h);
    const f = { x, y, w: Math.round(w), h: Math.round(h) };
    await insertObj({ ...baseObj("picture", f, file.name), asset: r.asset, crop: { l: 0, t: 0, r: 0, b: 0 }, alpha: 1, clip: "rect" }, "插入圖片");
  } catch (e) { toast(e.message, true); }
}

// ── Narrow widths: slide list and side panel become drawers ───────────────
function togglePanel(which, force) {
  const app = $("#app");
  const cls = which === "sorter" ? "show-sorter" : "show-side";
  const on = force ?? !app.classList.contains(cls);
  app.classList.toggle(cls, on);
  $(which === "sorter" ? "#toggleSorter" : "#toggleSide").setAttribute("aria-pressed", on ? "true" : "false");
}

// ── Boot ──────────────────────────────────────────────────────────────────
// Fonts the deck names that this machine lacks: a chip in the top bar, a list
// on click (what each one becomes), closed by Esc or a click elsewhere.
function showFonts(list) {
  const chip = $("#fontChip"), pop = $("#fontPop");
  const off = (list ?? []).filter((f) => f.how === "substitute" || f.how === "missing");
  pop.hidden = true;
  chip.setAttribute("aria-expanded", "false");
  chip.hidden = !off.length;
  if (!off.length) return;
  const missing = off.filter((f) => f.how === "missing").length;
  chip.textContent = `字型代替 ${off.length}`;
  chip.title = "這份簡報用到的字型，這台電腦有些沒有；點開看用什麼代替";
  chip.classList.toggle("warn", missing > 0); // substitutes alone are no warning, only a missing font is
  chip.classList.toggle("err", missing > 0);
  pop.textContent = "";
  const head = document.createElement("p");
  head.className = "muted";
  head.textContent = "這台電腦沒有下列字型，畫面與 PDF 用右邊的字代替；字寬不同時換行可能和 PowerPoint 不一樣。規則可以寫在 private/shared/fonts.toml 的 [fallback]，字型檔可以放在 private/shared/fonts/。";
  pop.append(head);
  const ul = document.createElement("ul");
  for (const f of off) {
    const li = document.createElement("li");
    const a = document.createElement("span"); a.className = "fname"; a.textContent = f.name; a.title = f.name;
    const arrow = document.createElement("span"); arrow.className = "muted"; arrow.textContent = "→";
    const b = document.createElement("span"); b.className = "fname" + (f.used ? "" : " err"); b.textContent = f.used ?? "找不到，用系統預設字"; b.title = (f.chain ?? []).join("、");
    li.append(a, arrow, b);
    ul.append(li);
  }
  pop.append(ul);
}

async function openDeck(name) {
  const r = await api(`/api/decks/${encodeURIComponent(name)}`);
  SC.addFontStacks(r.font_stacks);
  showFonts(r.fonts);
  S.name = r.name;
  S.deck = r.deck;
  S.style = r.style;
  S.backdrops = r.backdrops;
  S.status = r.status;
  const want = new URL(location.href).searchParams.get("s");
  S.cur = S.deck.slides.some((s) => s.uid === want) ? want : S.deck.slides[0]?.uid;
  S.arr = null;
  loadArrangements().then(() => { renderSide(); renderGuides(); }).catch(() => {});
  S.sel = [];
  $("#deckName").textContent = name;
  document.title = `${name} · SlideComposer`;
  $("#home").hidden = true;
  document.body.classList.remove("at-home");
  $("#saveChip").className = "saved";
  undoButtons();
  FILE.asked = false;
  setFile(r.file);
  renderAll();
}

/** Status bar: position, layout, save state (zoom is set by renderCanvas). */
function statusLine() {
  const sl = slide();
  if (!sl) return;
  $("#stPos").textContent = `${String(slideNo(sl.uid)).padStart(2, "0")} / ${String(S.deck.slides.length).padStart(2, "0")}`;
  $("#stLayout").textContent = sl.layout ?? "";
}

// ── Home: 01 開始 (tiles) and 02 最近 (first-slide thumbnails) ─────────────
function goHome() {
  // an edit still inside the autosave delay goes to the file now
  clearTimeout(FILE.timer);
  flushFileOnLeave();
  const u = new URL(location.href);
  u.search = "";
  history.pushState(null, "", u);
  showHome();
}
function openByName(name) {
  location.search = `?deck=${encodeURIComponent(name)}`;
}

async function showHome() {
  $("#home").hidden = false;
  document.body.classList.add("at-home");
  $("#deckName").textContent = "";
  setFile(null);
  document.title = "SlideComposer";
  S.reveal = null;
  S.deck = null;
  renderSide();
  const info = await api("/api/info");
  S.info = info;
  const list = $("#deckList");
  list.textContent = "";
  $("#recentBlock").hidden = !info.decks.length;
  const recent = [...info.decks].sort((a, b) => b.modified - a.modified).slice(0, 8);
  for (const d of recent) {
    const card = document.createElement("button");
    card.className = "deck-card";
    card.innerHTML = `<div class="frame"></div><span class="name"></span><span class="when"></span>`;
    card.querySelector(".name").textContent = d.name;
    card.title = d.name;
    card.querySelector(".when").textContent = shortDate(d.modified);
    card.onclick = () => openByName(d.name);
    list.append(card);
    coverWhenVisible(card.querySelector(".frame"), d.name);
  }
  $("#envInfo").textContent = "";
  for (const [k, v, ok] of [["資料夾", info.workspace, true], ["簡報", info.decks_dir, true], ["pandoc", info.pandoc ? "✓" : "✗  brew install pandoc", info.pandoc], ["Chrome", info.chrome ? "✓" : "✗  Google Chrome", info.chrome]]) {
    const dt = document.createElement("dt"); dt.textContent = k;
    const dd = document.createElement("dd"); dd.textContent = v; if (!ok) dd.className = "bad";
    $("#envInfo").append(dt, dd);
  }
}
function shortDate(sec) {
  const d = new Date(sec * 1000), now = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return d.toDateString() === now.toDateString() ? `${p(d.getHours())}:${p(d.getMinutes())}` : `${d.getFullYear() === now.getFullYear() ? "" : d.getFullYear() + "/"}${p(d.getMonth() + 1)}/${p(d.getDate())}`;
}

/** Render a deck's first slide into `frame` once it scrolls into view. */
const coverSeen = new IntersectionObserver((es) => es.forEach((e) => { if (e.isIntersecting) { coverSeen.unobserve(e.target); drawCover(e.target).catch(() => {}); } }));
function coverWhenVisible(frame, name) { frame.dataset.deck = name; coverSeen.observe(frame); }
async function drawCover(frame) {
  const name = frame.dataset.deck;
  const r = await api(`/api/decks/${encodeURIComponent(name)}`);
  SC.addFontStacks(r.font_stacks);
  const sl = r.deck.slides.find((s) => !s.hidden) ?? r.deck.slides[0];
  if (!sl) return;
  const latex = new Map();
  const node = SC.renderSlide(sl, {
    size: r.deck.size, backdrops: r.backdrops, slideNo: 1,
    assetUrl: (a) => `/api/decks/${encodeURIComponent(name)}/asset?id=${encodeURIComponent(a)}`,
    latex: (omml) => latex.get(omml), needLatex: () => {},
  });
  const W = SC.px(r.deck.size.w);
  node.style.transform = `scale(${frame.clientWidth / W})`;
  frame.append(node);
}

function wireHome() {
  $("#tileNew").onclick = openNewDeck;
  const tile = $("#tileImport");
  // desktop: the native dialog gives the real path, so the file is edited where it lives
  tile.addEventListener("click", (e) => { if (S.info?.shell === "desktop" && e.target !== $("#impFile")) { e.preventDefault(); openFile(); } });
  $("#impFile").onchange = (e) => { const f = e.target.files[0]; e.target.value = ""; if (f) importPptx(f); };
  let depth = 0;
  tile.addEventListener("dragenter", (e) => { e.preventDefault(); depth++; tile.classList.add("over"); });
  tile.addEventListener("dragleave", () => { if (--depth <= 0) { depth = 0; tile.classList.remove("over"); } });
  tile.addEventListener("dragover", (e) => e.preventDefault());
  tile.addEventListener("drop", (e) => { e.preventDefault(); depth = 0; tile.classList.remove("over"); const f = e.dataTransfer.files[0]; if (f) importPptx(f); });
  $("#newForm").onsubmit = async (e) => {
    e.preventDefault();
    const style = $("#baseList input:checked")?.value;
    if (!style) return;
    const name = $("#newName").value.trim() || `簡報 ${shortDate(Date.now() / 1000).replace(":", "")}`;
    try { const r = await api("/api/decks", { body: { name, style } }); openByName(r.name); } catch (x) { toast(x.message, true); }
  };
  $$("dialog .close").forEach((b) => (b.onclick = () => b.closest("dialog").close()));
}
async function importPptx(f) {
  if (!/\.pptx$/i.test(f.name)) { toast("只能匯入 .pptx", true); return; }
  toast(`匯入 ${f.name}…`);
  try {
    const r = await api(`/api/decks/import?name=${encodeURIComponent(f.name.replace(/\.pptx$/i, ""))}`, { body: await f.arrayBuffer(), raw: true });
    openByName(r.name);
  } catch (x) { toast(x.message, true); }
}

/** 新簡報: pick a base (底版) by its preview, name optional. */
function openNewDeck() {
  const box = $("#baseList");
  box.textContent = "";
  const styles = S.info?.style_list ?? (S.info?.styles ?? []).map((id) => ({ id, name: id, previews: 0 }));
  styles.forEach((st, i) => {
    const l = document.createElement("label");
    l.className = "base-opt";
    l.title = st.name;
    l.innerHTML = `<input type="radio" name="base"><div class="thumb16"></div><span></span>`;
    const r = l.querySelector("input");
    r.value = st.id;
    r.checked = i === 0;
    l.querySelector("span").textContent = st.name;
    box.append(l);
    drawStyleCover(l.querySelector(".thumb16"), st.id).catch(() => l.querySelector(".thumb16").classList.add("blank"));
  });
  $("#newBtn").disabled = !styles.length;
  $("#newName").value = "";
  $("#dlgNew").showModal();
  $("#newName").focus();
}

/** A style's cover page, rendered like a slide (layout background, starter objects). */
async function drawStyleCover(frame, id) {
  const r = await api(`/api/styles/${encodeURIComponent(id)}/sample`);
  const sl = r.slides[0];
  if (!sl) return;
  const node = SC.renderSlide(sl, {
    size: r.size, backdrops: r.backdrops, slideNo: 1,
    assetUrl: (a) => `/api/styles/${encodeURIComponent(id)}/asset?id=${encodeURIComponent(a)}`,
    latex: () => undefined, needLatex: () => {},
  });
  node.style.transform = `scale(${frame.clientWidth / SC.px(r.size.w)})`;
  frame.textContent = "";
  frame.append(node);
}

/** Top-bar menus: the deck picker (recent decks + 新簡報) and export. */
function popMenu(anchor, items) {
  $(".menu")?.remove();
  const m = document.createElement("div");
  m.className = "menu";
  m.setAttribute("role", "menu");
  for (const it of items) {
    if (it === "-") { const hr = document.createElement("div"); hr.className = "menu-sep"; m.append(hr); continue; }
    const b = document.createElement("button");
    b.setAttribute("role", "menuitem");
    if (it.icon) { const i = document.createElement("span"); i.dataset.icon = it.icon; b.append(i); }
    b.append(it.label);
    if (it.current) b.setAttribute("aria-current", "true");
    b.onclick = () => { m.remove(); anchor.setAttribute("aria-expanded", "false"); it.run(); };
    m.append(b);
  }
  document.body.append(m);
  paintIcons(m);
  const r = anchor.getBoundingClientRect();
  m.style.top = `${r.bottom + 4}px`;
  m.style.left = `${Math.max(8, Math.min(r.left, innerWidth - m.offsetWidth - 8))}px`;
  anchor.setAttribute("aria-expanded", "true");
  setTimeout(() => document.addEventListener("pointerdown", (e) => { if (!m.contains(e.target)) { m.remove(); anchor.setAttribute("aria-expanded", "false"); } }, { once: true }), 0);
}
async function deckMenu(anchor) {
  const info = await api("/api/info");
  S.info = info;
  const decks = [...info.decks].sort((a, b) => b.modified - a.modified).slice(0, 10);
  popMenu(anchor, [
    ...decks.map((d) => ({ label: d.name, current: d.name === S.name, run: () => { if (d.name !== S.name) openByName(d.name); } })),
    "-",
    { icon: "folder", label: "開啟…", run: openFile },
    { icon: "download", label: "另存新檔…", run: saveAs },
    "-",
    { icon: "plus", label: "新簡報", run: openNewDeck },
    { icon: "back", label: "首頁", run: goHome },
  ]);
}
function exportMenu(anchor) {
  popMenu(anchor, [
    { icon: "download", label: "PPTX", run: async () => { toast("匯出 PPTX…"); try { const r = await api(deckApi("/export"), { body: {} }); exportCard(r.path, r.warning); } catch (e) { toast(e.message, true); } } },
    { icon: "pdf", label: "PDF", run: async () => { toast("產生 PDF…"); try { const r = await api(deckApi("/pdf"), { body: {} }); exportCard(r.path); } catch (e) { toast(e.message, true); } } },
  ]);
}
/** After an export: file name, page count, 在 Finder 顯示 and 開啟. */
function exportCard(path, warning) {
  $(".toast")?.remove();
  $(".export-card")?.remove();
  const name = String(path).split(/[\\/]/).pop();
  const pages = S.deck?.slides.filter((s) => !s.hidden).length ?? 0;
  const act = (reveal) => () => api("/api/export/open", { body: { path, reveal } }).catch((e) => toast(e.message, true));
  const card = h("div", { class: "export-card", role: "status" },
    h("div", { class: "ec-main" }, h("b", { class: "ec-name", title: path }, name), h("span", { class: "muted" }, `${pages} 頁`)),
    warning ? h("div", { class: "ec-warn" }, warning) : null,
    h("div", { class: "ec-actions" },
      h("button", { class: "btn small", onclick: act(true) }, h("span", { "data-icon": "folder" }), "在 Finder 顯示"),
      h("button", { class: "btn small primary", onclick: act(false) }, h("span", { "data-icon": "external" }), "開啟"),
      h("button", { class: "ico small", "data-icon": "x", "aria-label": "關閉", onclick: () => card.remove() })));
  document.body.append(card);
  paintIcons(card);
  setTimeout(() => card.remove(), warning ? 20000 : 12000);
}

// ── 方程式與變數: the deck's notation registry (shared <notation-panel>) ──────
const NT = { lines: {}, timer: 0 };
async function openNotation() {
  const r = await api(deckApi("/notation"));
  showNotation(r);
  $("#dlgNotation").showModal();
}
function showNotation(r) {
  const p = $("#ntPanel");
  NT.lines = r.lines ?? {};
  p.notation = r.notation;
  p.report = r.report;
}
function wireNotation() {
  const p = $("#ntPanel");
  const save = () => {
    clearTimeout(NT.timer);
    $("#ntState").hidden = false;
    $("#ntState").textContent = "未儲存";
    NT.timer = setTimeout(async () => {
      try {
        const r = await api(deckApi("/notation"), { method: "PUT", body: p.notation });
        NT.lines = r.lines ?? {};
        p.report = r.report;
        $("#ntState").textContent = "已儲存";
      } catch (e) { toast(e.message, true); }
    }, 500);
  };
  // an edited global row becomes the deck's own (the global file is never written here)
  p.addEventListener("notation-change", (e) => { if (e.detail.entry?.origin === "global") e.detail.entry.origin = "document"; });
  for (const ev of ["notation-change", "notation-add", "notation-delete"]) p.addEventListener(ev, save);
  p.addEventListener("notation-jump", (e) => {
    const at = NT.lines[e.detail.file]?.[(e.detail.line ?? 1) - 1];
    if (!at) return;
    $("#dlgNotation").close();
    gotoSlide(at[0]);
    S.sel = [at[1]];
    renderOverlay();
    renderSide();
  });
  $("#ntExtract").onclick = async () => {
    try {
      const r = await api(deckApi("/notation/extract"), { body: {} });
      showNotation(r);
      toast(r.added ? `新增 ${r.added} 筆` : "沒有新的");
    } catch (e) { toast(e.message, true); }
  };
}

// ── Settings: 外觀 (the theme all LoreForge apps share, theme.js) and folders ─────────
function wireSettings() {
  LoreTheme.picker($("#themePick"));
  $("#actLook").onclick = () => dialog((box, close) => {
    const pick = h("div", {});
    LoreTheme.picker(pick);
    box.append(h("h3", {}, "外觀"), pick, h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: close }, "關閉")));
  });
}
async function openSettings() {
  if (!S.info) { try { S.info = await api("/api/info"); } catch {} }
  if (S.info && !$("#envInfo").childElementCount) {
    for (const [k, v] of [["資料夾", S.info.workspace], ["簡報", S.info.decks_dir]]) {
      const dt = document.createElement("dt"); dt.textContent = k;
      const dd = document.createElement("dd"); dd.textContent = v;
      $("#envInfo").append(dt, dd);
    }
  }
  $("#dlgSet").showModal();
}

function wire() {
  paintIcons();
  const vp = $("#viewport");
  vp.addEventListener("pointerdown", () => setFocus("canvas"), true);
  vp.addEventListener("pointerdown", onPointerDown);
  vp.addEventListener("pointermove", onPointerMove);
  vp.addEventListener("pointerup", onPointerUp);
  vp.addEventListener("pointercancel", onPointerUp);
  vp.addEventListener("dblclick", onDblClick);
  document.addEventListener("keydown", onKey);
  addEventListener("resize", () => { if (S.deck) { renderCanvas(); } });
  $$("[data-tool]").forEach((b) => (b.onclick = () => setTool(b.dataset.tool)));
  $("#actUndo").onclick = undo;
  $("#actRedo").onclick = redo;
  $("#actZoomIn").onclick = () => { S.fit = false; S.zoom = Math.min(4, S.zoom * 1.25); renderCanvas(); };
  $("#actZoomOut").onclick = () => { S.fit = false; S.zoom = Math.max(0.2, S.zoom / 1.25); renderCanvas(); };
  $("#actFit").onclick = () => { S.fit = true; renderCanvas(); };
  $("#actGrid").onclick = (e) => { S.grid = !S.grid; e.currentTarget.setAttribute("aria-pressed", S.grid); renderCanvas(); };
  $("#actSnap").onclick = (e) => { S.snap = !S.snap; e.currentTarget.setAttribute("aria-pressed", S.snap); };
  $("#actHome").onclick = goHome;
  $("#zoomChip").onclick = () => { S.fit = true; renderCanvas(); };
  $("#actSettings").onclick = openSettings;
  $("#deckPicker").onclick = (e) => deckMenu(e.currentTarget);
  $("#fileState").onclick = () => { if (S.file?.state === "changed") { $(".ext-bar")?.remove(); changedOnDisk(); } else if (S.file?.state === "missing") askConflict(); else if (S.file?.state === "error" || S.file?.state === "pending") writeFile(); };
  addEventListener("pagehide", flushFileOnLeave);
  watchFile();
  $("#actExport").onclick = (e) => exportMenu(e.currentTarget);
  $("#actShow").onclick = (e) => popMenu(e.currentTarget, [
    { icon: "play", label: "從這頁放映", run: () => startShow(true) },
    { icon: "refresh", label: "從頭放映", run: () => startShow(false) },
    { icon: "panel", label: "簡報者檢視（延伸螢幕）", run: () => startShow(true, "presenter") },
  ]);
  $("#actNotation").onclick = openNotation;
  wireNotation();
  wireHome();
  wireSettings();
  $("#fontChip").onclick = (e) => {
    e.stopPropagation();
    const pop = $("#fontPop"), open = pop.hidden;
    pop.hidden = !open;
    $("#fontChip").setAttribute("aria-expanded", String(open));
    if (open) { const r = $("#fontChip").getBoundingClientRect(); pop.style.top = `${r.bottom + 6}px`; pop.style.left = `${Math.max(8, Math.min(r.left, innerWidth - pop.offsetWidth - 8))}px`; }
  };
  document.addEventListener("click", (e) => { if (!e.target.closest("#fontPop")) { $("#fontPop").hidden = true; $("#fontChip").setAttribute("aria-expanded", "false"); } });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("#fontPop").hidden) { $("#fontPop").hidden = true; $("#fontChip").setAttribute("aria-expanded", "false"); } });
  $("#slideAdd").onclick = addSlide;
  $("#slideDup").onclick = dupSlide;
  $("#slideDel").onclick = delSlide;
  $("#slideHide").onclick = hideSlide;
  $("#insImage").onclick = () => $("#imgFile").click();
  $("#imgFile").onchange = (e) => {
    const f = e.target.files[0], into = e.target.dataset.slot;
    e.target.value = "";
    delete e.target.dataset.slot;
    if (!f) return;
    const o = into ? findObj(+into) : null;
    if (o) fillImageSlot(o, f); else insertImageFile(f);
  };
  $("#insImage").addEventListener("click", () => delete $("#imgFile").dataset.slot, true);
  $("#insTable").onclick = () => openTableDialog();
  $("#insEq").onclick = () => openEquation(null);
  $("#insComp").onclick = (e) => openComponentMenu(e.currentTarget);
  $("#actReveal").onclick = () => openReveal();
  // notes belong to the slide that was current when the field got focus
  let notesUid = null;
  $("#notes").addEventListener("focus", () => (notesUid = S.cur));
  $("#notes").addEventListener("change", (e) => {
    const uid = notesUid ?? S.cur;
    const sl = S.deck?.slides.find((x) => x.uid === uid);
    if (sl && e.target.value !== (sl.notes ?? "")) commit([{ op: "set_slide", slide: uid, path: "/notes", value: e.target.value }], "講者備註");
  });
  $("#notes").addEventListener("blur", () => setTimeout(() => (notesUid = null), 0));
  $("#toggleSorter").onclick = () => togglePanel("sorter");
  $("#toggleSide").onclick = () => togglePanel("side");
  wireSorter();
  addEventListener("popstate", boot);
  const pane = $("#viewport");
  pane.addEventListener("dragover", (e) => e.preventDefault());
  pane.addEventListener("drop", (e) => {
    e.preventDefault();
    const f = [...e.dataTransfer.files].find((x) => x.type.startsWith("image/"));
    if (!f || !S.deck) return;
    // dropped on an empty image slot: it fills that slot
    const at = topObjEl(document.elementFromPoint(e.clientX, e.clientY));
    const o = at?.classList.contains("slot-empty") && at.dataset.slot === "image" ? findObj(+at.dataset.id) : null;
    if (o) fillImageSlot(o, f); else insertImageFile(f);
  });
}

async function boot() {
  const name = new URL(location.href).searchParams.get("deck");
  try {
    if (!name) { await showHome(); return; }
    await openDeck(name);
    if (new URL(location.href).searchParams.get("audience")) startAudience();
  } catch (e) { toast(e.message, true); await showHome(); }
}

wire();
boot();
