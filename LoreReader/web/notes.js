// LoreReader — text boxes placed anywhere on a page (annotation kind "text").
//
// Double-click a page: an empty box opens at the pointer, ready to type (the browser's
// word / paragraph selection is suppressed). Enter = new line, ⌘↵ or a click outside saves,
// Esc on an empty box drops it; an empty box is never kept. A saved box: click = edit,
// drag = move, the corner = resize, × or ⌫ (while selected) = delete, ⌘Z = undo the last
// change to a box. Triple-click selects the clicked line only (the selection toolbar follows).
//
// Boxes live in annotations.json like highlights: {kind: "text", rects: [[x0, y0, x1, y1]]
// (page fractions, top-left origin), note: the text}. They are drawn in a layer of the page
// element, sized by --scale-factor, so they stay put at every zoom, while a pinch scales
// the pages with a CSS transform, and through re-layouts (a box being edited is redrawn with
// its draft and keeps the focus). Font size, padding and line height are annotate.rs's
// TEXT_PT / TEXT_PAD / TEXT_LEAD, so the PDF export wraps the same way.
//
// app.js hooks: drawHighlights() calls LRNotes.draw(p); notesPane() calls LRNotes.item(a);
// window.LRHost = {S, h, api, renderSide, refreshExport, jumpTo}.

const PT = 10, PAD = 4, LEAD = 1.3;   // annotate.rs TEXT_PT, TEXT_PAD, TEXT_LEAD (points)
const DEFAULT_W = 170;                // a new box's width (points)
const host = () => window.LRHost;
const clamp = (x, a, b) => Math.max(a, Math.min(b, x));

const NT = {
  sel: null,        // id of the selected (not edited) box
  edit: null,       // {id|null, sha, page, rect, text, before, at}: the box being typed in (id null = new)
  undo: [],         // [{type: add|del|edit, a, before}]
};

const pageOf = (el) => host().S.pages[+el.closest(".page")?.dataset.i];
const pageEls = (sha, page) => host().S.pages.filter((p) => p.sha === sha && p.page === page && p.el);
const toast = (msg, opt) => LoreShell.toast(msg, opt);
const fail = (e) => toast(e.message ?? String(e), { err: true });

// ── drawing ───────────────────────────────────────────────────────────────
function draw(p) {
  const { S, h } = host();
  if (!p?.el) return;
  if (NT.paper !== S.sha) { NT.paper = S.sha; NT.undo = []; NT.sel = null; if (NT.edit?.sha !== S.sha) NT.edit = null; }
  let layer = p.el.querySelector(":scope > .tn-layer");
  if (!layer) layer = h("div", { class: "tn-layer" });
  p.el.append(layer); // after the text layer
  const own = p.sha === S.sha ? S.annotations.filter((a) => a.kind === "text" && a.page === p.page) : [];
  const e = NT.edit;
  const items = own.map((a) => (e && e.id === a.id ? { ...a, note: e.text, rects: [e.rect] } : a));
  if (e && !e.id && e.sha === p.sha && e.page === p.page) items.push({ id: null, page: e.page, note: e.text, rects: [e.rect] });
  layer.replaceChildren(...items.map((a) => box(a, p)));
  layer.querySelectorAll(".tn").forEach(fit);
  const ed = e && e.sha === p.sha && e.page === p.page && layer.querySelector(e.id ? `.tn[data-id="${e.id}"]` : ".tn.fresh");
  if (ed) {
    const ta = ed.querySelector("textarea"), keep = e.caret ?? ta.value.length;
    ta.focus({ preventScroll: true });
    ta.setSelectionRange(keep, keep);
  }
}

function redraw(sha, page) { pageEls(sha, page).forEach(draw); }

function place(el, r) {
  Object.assign(el.style, { left: `${r[0] * 100}%`, top: `${r[1] * 100}%`, width: `${(r[2] - r[0]) * 100}%`, minHeight: `${(r[3] - r[1]) * 100}%` });
}

/** The textarea's height follows its text. */
function fit(el) {
  const ta = el.querySelector("textarea");
  ta.style.height = "auto";
  ta.style.height = `${ta.scrollHeight}px`;
}

function box(a, p) {
  const { h } = host();
  const editing = NT.edit && (NT.edit.id ? NT.edit.id === a.id : !a.id);
  const el = h("div", { class: `tn${a.id ? "" : " fresh"}${editing ? " editing" : ""}${NT.sel && NT.sel === a.id ? " sel" : ""}`, dataset: a.id ? { id: a.id } : {} });
  place(el, a.rects[0]);
  const ta = h("textarea", { class: "tn-text", rows: "1", spellcheck: "false", "aria-label": `文字 · 第 ${a.page} 頁` });
  ta.value = a.note ?? "";
  ta.readOnly = !editing;
  const del = h("button", { type: "button", class: "tn-del", title: "刪除 ⌫", "aria-label": "刪除" }, "×");
  const grip = h("span", { class: "tn-grip", "aria-hidden": "true" });
  el.append(ta, del, grip);

  ta.addEventListener("input", () => { if (NT.edit) { NT.edit.text = ta.value; NT.edit.caret = ta.selectionStart; } fit(el); });
  ta.addEventListener("keydown", (e) => {
    if (!NT.edit) return;
    NT.edit.caret = ta.selectionStart;
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); commit(); }
    else if (e.key === "Escape") {
      e.preventDefault(); e.stopPropagation();
      if (!ta.value.trim()) dropEdit(); else commit(true);
    }
  });
  ta.addEventListener("blur", () => setTimeout(() => {
    // a redraw (zoom, scroll, another annotation) replaced the box: the new one has the focus
    if (!ta.isConnected || !NT.edit || document.activeElement === ta) return;
    commit();
  }, 0));
  // Keep the page's handlers (selection toolbar, double-click, word popovers) out of the box.
  for (const t of ["mouseup", "click", "dblclick", "contextmenu"]) el.addEventListener(t, (e) => e.stopPropagation());
  el.addEventListener("mousedown", (e) => {
    e.stopPropagation();
    // the third click of a triple-click lands on the box the double-click just opened
    if (e.detail >= 3 && !a.id && !(NT.edit?.text ?? "").trim()) { e.preventDefault(); dropEdit(); selectLine(e.clientX, e.clientY); }
  });
  // × keeps the focus in the text (no blur → save first), then drops the box
  del.addEventListener("pointerdown", (e) => { e.stopPropagation(); e.preventDefault(); });
  del.addEventListener("mousedown", (e) => e.preventDefault());
  del.addEventListener("click", () => {
    const e = NT.edit;
    if (e && (e.id ? e.id === a.id : !a.id)) NT.edit = null;
    if (a.id) remove(a.id);
    else if (e) redraw(e.sha, e.page);
  });
  el.addEventListener("pointerdown", (e) => onPointer(e, el, a, p, e.target === grip ? "resize" : "move"));
  return el;
}

// ── move, resize, click to edit ───────────────────────────────────────────
function onPointer(e, el, a, p, mode) {
  if (e.button !== 0) return;
  // The third click of a triple-click on the edge of the box the double-click just opened
  // (preventing pointerdown below would also swallow its mousedown, which carries detail 3).
  if (!a.id && NT.edit && !NT.edit.id && !NT.edit.text.trim() && Date.now() - NT.edit.at < 700) {
    e.preventDefault();
    dropEdit();
    selectLine(e.clientX, e.clientY);
    return;
  }
  const editing = el.classList.contains("editing");
  if (editing && mode === "move" && e.target.tagName === "TEXTAREA") return; // caret, text selection
  e.preventDefault();
  const pr = p.el.getBoundingClientRect(); // includes a pinch's CSS scale
  const r0 = currentRect(el, p);
  const x0 = e.clientX, y0 = e.clientY;
  let moved = false;
  el.setPointerCapture(e.pointerId);
  if (a.id && !editing) select(a.id);
  const move = (ev) => {
    const dx = (ev.clientX - x0) / pr.width, dy = (ev.clientY - y0) / pr.height;
    if (!moved && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 3) return;
    moved = true;
    el.classList.add("dragging");
    let r;
    if (mode === "resize") {
      const minW = (PT * 2 + PAD * 2) / p.w, minH = (PT * LEAD + PAD * 2) / p.h;
      r = [r0[0], r0[1], clamp(r0[2] + dx, r0[0] + minW, 1), clamp(r0[3] + dy, r0[1] + minH, 1)];
    } else {
      const w = r0[2] - r0[0], hh = r0[3] - r0[1];
      const nx = clamp(r0[0] + dx, 0, 1 - w), ny = clamp(r0[1] + dy, 0, Math.max(0, 1 - hh));
      r = [nx, ny, nx + w, ny + hh];
    }
    el._rect = r;
    place(el, r);
    fit(el);
  };
  const up = () => {
    el.removeEventListener("pointermove", move);
    el.removeEventListener("pointerup", up);
    el.removeEventListener("pointercancel", up);
    el.classList.remove("dragging");
    if (moved) {
      const r = el._rect;
      if (NT.edit && (NT.edit.id ? NT.edit.id === a.id : !a.id)) { NT.edit.rect = r; el.querySelector("textarea").focus({ preventScroll: true }); }
      else if (a.id) save(a.id, { rects: [r] });
    } else if (!editing && a.id) startEdit(a, p);
  };
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerup", up);
  el.addEventListener("pointercancel", up);
}

/** The box's rect as shown (its height can exceed the stored one when the text needs it). */
function currentRect(el, p) {
  const pr = p.el.getBoundingClientRect(), r = el.getBoundingClientRect();
  return [(r.left - pr.left) / pr.width, (r.top - pr.top) / pr.height, (r.right - pr.left) / pr.width, (r.bottom - pr.top) / pr.height];
}

function select(id) {
  NT.sel = id;
  document.querySelectorAll(".tn.sel").forEach((x) => x.classList.remove("sel"));
  if (id) document.querySelectorAll(`.tn[data-id="${id}"]`).forEach((x) => x.classList.add("sel"));
}

// ── editing ───────────────────────────────────────────────────────────────
function startEdit(a, p, caret) {
  if (NT.edit) commit();
  select(null);
  NT.edit = { id: a.id, sha: p.sha, page: a.page, rect: [...a.rects[0]], text: a.note ?? "", before: a.id ? snapshot(a) : null, caret };
  redraw(p.sha, a.page);
}

/** A new box with the pointer at the start of its first line. */
function create(p, cx, cy) {
  const pr = p.el.getBoundingClientRect();
  const fx = (cx - pr.left) / pr.width, fy = (cy - pr.top) / pr.height;
  const w = Math.min(DEFAULT_W / p.w, 0.9), hh = (PAD * 2 + PT * LEAD) / p.h;
  const x0 = clamp(fx - PAD / p.w, 0, 1 - w), y0 = clamp(fy - (PAD + (PT * LEAD) / 2) / p.h, 0, 1 - hh);
  if (NT.edit) commit();
  select(null);
  NT.edit = { id: null, sha: p.sha, page: p.page, rect: [x0, y0, x0 + w, y0 + hh], text: "", before: null, at: Date.now() };
  redraw(p.sha, p.page);
}

const snapshot = (a) => ({ note: a.note, rects: a.rects.map((r) => [...r]) });

/** Save the box being typed in: a new one is added, a changed one updated, an empty one dropped. */
async function commit(keepSelected = false) {
  const e = NT.edit;
  if (!e) return;
  NT.edit = null;
  const { S, api } = host();
  const text = e.text.replace(/\s+$/, "");
  // the box grows with its text (never below the height it was given)
  const el = pageEls(e.sha, e.page)[0]?.el.querySelector(e.id ? `.tn[data-id="${e.id}"]` : ".tn.fresh");
  const p = pageEls(e.sha, e.page)[0];
  let rect = e.rect;
  if (el && p) { const cur = currentRect(el, p); rect = [rect[0], rect[1], rect[2], Math.max(rect[3], cur[3])]; }
  try {
    if (!text.trim()) {
      if (e.id) await remove(e.id, { quiet: true });
      else redraw(e.sha, e.page);
      return;
    }
    if (!e.id) {
      const r = await api(`/api/papers/${e.sha}/annotations`, { body: { page: e.page, kind: "text", color: "yellow", note: text, quote: { exact: "" }, rects: [rect] } });
      if (e.sha === S.sha) S.annotations.push(r.annotation);
      NT.undo.push({ type: "add", a: r.annotation });
      if (keepSelected) NT.sel = r.annotation.id;
    } else {
      const a = S.annotations.find((x) => x.id === e.id);
      if (a && (a.note !== text || JSON.stringify(a.rects[0]) !== JSON.stringify(rect))) {
        await save(e.id, { note: text, rects: [rect] }, e.before, false);
      }
      if (keepSelected) NT.sel = e.id;
    }
    changed(e.sha, e.page);
  } catch (err) { fail(err); redraw(e.sha, e.page); }
}

function dropEdit() {
  const e = NT.edit;
  if (!e) return;
  NT.edit = null;
  if (e.id) {
    // Esc on an emptied box: the box goes (undo brings it back)
    remove(e.id);
  } else redraw(e.sha, e.page);
}

/** PUT a change to a saved box; `before` (default: as it is now) is what ⌘Z restores. */
async function save(id, patch, before, redrawAfter = true) {
  const { S, api } = host();
  const a = S.annotations.find((x) => x.id === id);
  if (!a) return;
  const prev = before ?? snapshot(a);
  try {
    const r = await api(`/api/papers/${S.sha}/annotations/${id}`, { method: "PUT", body: patch });
    Object.assign(a, r.annotation);
    NT.undo.push({ type: "edit", a, before: prev });
    if (redrawAfter) changed(S.sha, a.page);
  } catch (err) { fail(err); redraw(S.sha, a.page); }
}

async function remove(id, { quiet = false } = {}) {
  const { S, api } = host();
  const a = S.annotations.find((x) => x.id === id);
  if (!a) return;
  try {
    await api(`/api/papers/${S.sha}/annotations/${id}`, { method: "DELETE" });
    S.annotations = S.annotations.filter((x) => x.id !== id);
    if (NT.sel === id) NT.sel = null;
    NT.undo.push({ type: "del", a });
    changed(S.sha, a.page);
    if (!quiet) toast("已刪除文字", { action: { label: "復原", run: undo } });
  } catch (err) { fail(err); }
}

/** After a change: the page, the 02 註記 list and the 未儲存 chip. */
function changed(sha, page) {
  const { renderSide, refreshExport } = host();
  redraw(sha, page);
  renderSide("notes");
  refreshExport();
}

async function undo() {
  const u = NT.undo.pop();
  if (!u) return;
  const { S, api } = host();
  try {
    if (u.type === "add") {
      await api(`/api/papers/${S.sha}/annotations/${u.a.id}`, { method: "DELETE" });
      S.annotations = S.annotations.filter((x) => x.id !== u.a.id);
    } else if (u.type === "del") {
      // back as a new annotation (new id) with the same place and text
      const r = await api(`/api/papers/${S.sha}/annotations`, { body: { page: u.a.page, kind: "text", color: u.a.color, note: u.a.note, quote: { exact: "" }, rects: u.a.rects } });
      S.annotations.push(r.annotation);
      NT.undo.forEach((x) => { if (x.a.id === u.a.id) x.a = r.annotation; });
    } else {
      const r = await api(`/api/papers/${S.sha}/annotations/${u.a.id}`, { method: "PUT", body: u.before });
      const a = S.annotations.find((x) => x.id === u.a.id);
      if (a) Object.assign(a, r.annotation);
    }
    changed(S.sha, u.a.page);
  } catch (err) { fail(err); }
}

// ── triple-click: the clicked line only ──────────────────────────────────
function selectLine(cx, cy) {
  const pageEl = document.elementsFromPoint(cx, cy).find((x) => x.classList?.contains("page"));
  const tl = pageEl?.querySelector(".textLayer");
  if (!tl) return;
  const spans = [...tl.querySelectorAll(":scope > span, :scope .markedContent > span")].filter((s) => s.textContent.trim() && !s.classList.contains("lnum"));
  const rs = spans.map((s) => ({ s, r: s.getBoundingClientRect() }));
  const hit = rs.find(({ r }) => cy >= r.top - 1 && cy <= r.bottom + 1 && cx >= r.left - 2 && cx <= r.right + 2)
    ?? rs.filter(({ r }) => cy >= r.top && cy <= r.bottom).sort((a, b) => dist(a.r, cx) - dist(b.r, cx))[0];
  if (!hit) { getSelection().removeAllRanges(); return; }
  const mid = (r) => (r.top + r.bottom) / 2;
  const hh = hit.r.height;
  const line = rs.filter(({ r }) => Math.abs(mid(r) - mid(hit.r)) < hh * 0.5).sort((a, b) => a.r.left - b.r.left);
  // the run of spans around the hit with no column-sized gap between neighbours
  let i = line.indexOf(hit), j = i;
  while (i > 0 && line[i].r.left - line[i - 1].r.right < hh * 2) i--;
  while (j < line.length - 1 && line[j + 1].r.left - line[j].r.right < hh * 2) j++;
  const range = document.createRange();
  range.setStart(line[i].s, 0);
  range.setEnd(line[j].s, line[j].s.childNodes.length);
  const sel = getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
  // the selection toolbar opens on the page's mouseup, which a prevented pointerdown swallows
  document.getElementById("pages").dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
}
const dist = (r, x) => (x < r.left ? r.left - x : x > r.right ? x - r.right : 0);

// ── the 02 註記 list ──────────────────────────────────────────────────────
function item(a) {
  const { h, jumpTo, S } = host();
  const edit = () => {
    jumpTo(a.page, a.id);
    setTimeout(() => { const p = S.pages.find((x) => x.sha === S.sha && x.page === a.page); if (p) startEdit(a, p); }, 450);
  };
  return h("li", { class: "item", onclick: (e) => { if (!e.target.closest("button")) jumpTo(a.page, a.id); } },
    h("span", { class: "dot tn-dot" }),
    h("span", { class: "body" }, h("span", { class: "txt" }, a.note), h("span", { class: "meta" }, `p.${a.page} · ${(a.created_at ?? "").slice(5, 10)}`)),
    h("span", { class: "acts" },
      h("button", { class: "ico small", "aria-label": "編輯文字", "data-icon": "pen", onclick: edit }),
      h("button", { class: "ico small", "aria-label": "刪除", "data-icon": "trash", onclick: () => remove(a.id) })));
}

// ── wiring ────────────────────────────────────────────────────────────────
function wire() {
  const pages = document.getElementById("pages");
  // The second and third press of a click on a page: no word / paragraph / page selection.
  // Figure-only pages, image areas, the gaps between pages and the page-number labels
  // included: the browser would otherwise select the whole page element.
  pages.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || e.detail < 2 || e.target.closest(".tn")) return;
    e.preventDefault();
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    if (!e.target.closest(".page") || e.target.closest(".pgno")) { getSelection().removeAllRanges(); return; }
    const p = pageOf(e.target);
    if (!p || p.sha !== host().S.sha) return;
    if (e.detail === 2) { getSelection().removeAllRanges(); create(p, e.clientX, e.clientY); }
    else {
      if (NT.edit && !NT.edit.id && !NT.edit.text.trim()) dropEdit();
      selectLine(e.clientX, e.clientY);
    }
  }, true);
  // A click anywhere else drops the selection of a box.
  document.addEventListener("pointerdown", (e) => { if (NT.sel && !e.target.closest?.(".tn")) select(null); }, true);
  document.addEventListener("keydown", (e) => {
    if (e.target.closest?.("input, textarea, select, [contenteditable], dialog")) return;
    if ((e.key === "Backspace" || e.key === "Delete") && NT.sel && !e.metaKey && !e.ctrlKey) { e.preventDefault(); remove(NT.sel); }
    else if (e.key === "Escape" && NT.sel) select(null);
    else if (e.key.toLowerCase() === "z" && (e.metaKey || e.ctrlKey) && !e.shiftKey && NT.undo.length && host().S.sha) { e.preventDefault(); undo(); }
  });
}

window.LRNotes = { draw, item, undo, _state: NT };
wire();
