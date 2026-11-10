// 論文: material from LoreReader's papers (read-only, sc-server reader.rs) — page crops as
// pictures (the whole page is the asset, `crop` picks the region, so it can be re-cropped),
// equations and symbols as native OMML (registered in the deck's notation), highlights and
// notes as text boxes in the deck's tags, and a `Ref:` line. Loaded after app.js; uses its
// globals (S, api, deckApi, insertObj, commit, setObj, baseObj, newPara, newRun, tagLook,
// defaultTag, freeSpot, measureText, h, ib, paintIcons, toast) and wraps renderSide to
// follow the selection.
"use strict";

(() => {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = "reader.css";
  document.head.append(link);
})();

const RD = { open: false, papers: null, sha: null, paper: null, tab: "fig", page: null, rect: null, pics: {}, picFor: null, lastSel: "" };
const rdKey = () => `sc.reader.${S.name}`;
const rdPage = (sha, n, dpi) => `/api/reader/papers/${sha}/page/${n}?dpi=${dpi}`;

function rdButton() {
  if ($("#insPaper")) return;
  const b = h("button", { class: "ico", id: "insPaper", "data-icon": "news", "aria-label": "論文", "aria-pressed": "false", onclick: () => rdToggle() });
  const after = $("#insComp");
  if (after) after.after(b); else $("#tools")?.append(b);
  paintIcons(b.parentElement);
}

function rdShell() {
  let p = $("#rdPanel");
  if (p) return p;
  p = h("aside", { class: "rd", id: "rdPanel", "aria-label": "論文", hidden: true });
  document.body.append(p);
  return p;
}

async function rdToggle(force) {
  RD.open = force ?? !RD.open;
  $("#insPaper")?.setAttribute("aria-pressed", RD.open ? "true" : "false");
  rdShell().hidden = !RD.open;
  if (!RD.open) return;
  try {
    const [l, pics] = await Promise.all([api("/api/reader/papers"), api(deckApi("/reader/pictures")).catch(() => ({ pictures: {} }))]);
    RD.papers = l.papers;
    RD.pics = pics.pictures ?? {};
    let want = RD.sha;
    try { want ??= localStorage.getItem(rdKey()); } catch {}
    if (want && RD.papers.some((p) => p.sha === want)) await rdChoose(want, false);
    else { RD.sha = null; RD.paper = null; }
  } catch (e) { toast(e.message, true); }
  RD.lastSel = "";
  rdFollowSelection();
  rdRender();
}

async function rdChoose(sha, render = true) {
  RD.paper = await api(`/api/reader/papers/${sha}`);
  if (RD.sha !== sha) { RD.page = null; RD.rect = null; }
  RD.sha = sha;
  try { localStorage.setItem(rdKey(), sha); } catch {}
  if (render) rdRender();
}

// ── Rendering ──────────────────────────────────────────────────────────────
function rdRender() {
  const p = rdShell();
  if (!RD.open) return;
  p.textContent = "";
  const close = ib("x", "關閉", () => rdToggle(false));
  close.classList.add("small");
  if (!RD.paper) {
    p.append(h("div", { class: "rd-head" }, h("h3", {}, "論文"), close), rdList());
    paintIcons(p);
    return;
  }
  const info = RD.paper.paper;
  const back = ib("back", "論文", () => { RD.paper = null; RD.sha = null; rdRender(); });
  back.classList.add("small");
  const ref = h("button", { class: "rd-ref", title: RD.paper.ref, onclick: rdInsertRef }, "Ref");
  const tabs = h("div", { class: "rd-tabs", role: "tablist" },
    ...[["fig", "圖"], ["eq", "式"], ["notes", "筆記"]].map(([k, t]) => h("button", { role: "tab", "aria-selected": RD.tab === k ? "true" : "false", onclick: () => { RD.tab = k; rdRender(); } }, t)));
  const body = h("div", { class: "rd-body" });
  p.append(h("div", { class: "rd-head" }, back, h("h3", { class: "rd-title", title: info.title }, info.title), ref, close), tabs, body);
  if (RD.tab === "fig") rdFigures(body, info);
  else if (RD.tab === "eq") rdEquations(body);
  else rdNotes(body);
  paintIcons(p);
}

function rdList() {
  const box = h("div", { class: "rd-body" });
  if (!RD.papers?.length) { box.append(h("div", { class: "rd-empty" }, "LoreReader 書庫是空的")); return box; }
  for (const p of RD.papers) {
    const meta = [`${p.pages} 頁`, p.symbols ? `${p.symbols} 符號` : null, p.annotations + p.quotes ? `${p.annotations + p.quotes} 筆記` : null].filter(Boolean).join(" · ");
    box.append(h("button", { class: "rd-paper", onclick: () => rdChoose(p.sha).catch((e) => toast(e.message, true)) },
      h("span", { class: "rd-ptitle" }, p.title), h("span", { class: "rd-meta" }, meta, p.pdf ? null : " · 沒有 PDF")));
  }
  return box;
}

function rdFigures(body, info) {
  if (!info.pdf) { body.append(h("div", { class: "rd-empty" }, "LoreReader 書庫裡沒有這篇的 PDF")); return; }
  if (!RD.page) {
    const grid = h("div", { class: "rd-grid" });
    for (let n = 1; n <= info.pages; n++) {
      const img = h("img", { loading: "lazy", alt: `第 ${n} 頁`, src: rdPage(RD.sha, n, 36) });
      img.onerror = () => { img.replaceWith(h("span", { class: "rd-noimg" }, "無法顯示")); };
      grid.append(h("button", { class: "rd-thumb", onclick: () => { RD.page = n; RD.rect = null; rdRender(); } }, img, h("span", {}, n)));
    }
    body.append(grid);
    return;
  }
  const n = RD.page;
  const nav = h("div", { class: "rd-nav" },
    ib("cards", "所有頁面", () => { RD.page = null; rdRender(); }),
    ib("chevron", "上一頁", () => { if (n > 1) { RD.page = n - 1; RD.rect = null; rdRender(); } }),
    h("span", { class: "rd-pno" }, `${n} / ${info.pages}`),
    ib("chevron", "下一頁", () => { if (n < info.pages) { RD.page = n + 1; RD.rect = null; rdRender(); } }));
  const stage = h("div", { class: "rd-page" });
  const img = h("img", { alt: `第 ${n} 頁`, src: rdPage(RD.sha, n, 120), draggable: "false" });
  img.onerror = () => toast("無法顯示這一頁（需要 poppler 的 pdftoppm）", true);
  stage.append(img);
  // the paper's display equations on this page are ready-made regions
  for (const e of RD.paper.equations.filter((e) => e.page === n && e.bbox)) {
    const [x0, y0, x1, y1] = e.bbox;
    stage.append(h("button", { class: "rd-hint", title: e.number ? `(${e.number})` : "方程式", style: `left:${x0 * 100}%;top:${y0 * 100}%;width:${(x1 - x0) * 100}%;height:${(y1 - y0) * 100}%`,
      onclick: (ev) => { ev.stopPropagation(); RD.rect = rdPad(e.bbox); rdRender(); } }));
  }
  const sel = h("div", { class: "rd-sel" });
  stage.append(sel);
  const show = () => {
    sel.hidden = !RD.rect;
    if (RD.rect) { const [a, b, c, d] = RD.rect; sel.style.cssText = `left:${a * 100}%;top:${b * 100}%;width:${(c - a) * 100}%;height:${(d - b) * 100}%`; }
    for (const b of acts.querySelectorAll("button")) b.disabled = !RD.rect;
  };
  stage.onpointerdown = (ev) => {
    if (ev.button !== 0 || ev.target.classList.contains("rd-hint")) return;
    const r = stage.getBoundingClientRect();
    const at = (e) => [Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), Math.min(1, Math.max(0, (e.clientY - r.top) / r.height))];
    const p0 = at(ev);
    stage.setPointerCapture(ev.pointerId);
    stage.onpointermove = (e) => { const p = at(e); RD.rect = [Math.min(p0[0], p[0]), Math.min(p0[1], p[1]), Math.max(p0[0], p[0]), Math.max(p0[1], p[1])]; show(); };
    stage.onpointerup = () => {
      stage.onpointermove = stage.onpointerup = null;
      if (RD.rect && (RD.rect[2] - RD.rect[0] < 0.01 || RD.rect[3] - RD.rect[1] < 0.01)) RD.rect = null;
      show();
    };
  };
  const target = rdSelectedPic();
  const acts = h("div", { class: "rd-acts" },
    target && target.info.sha === RD.sha && target.info.page === n ? h("button", { class: "btn", onclick: () => rdRecrop(target.o) }, "更新裁切") : null,
    h("button", { class: "btn primary", onclick: () => rdInsertPicture(n, RD.rect) }, "插入"));
  body.append(nav, stage, acts);
  show();
}

function rdPad([x0, y0, x1, y1], d = 0.008) {
  return [Math.max(0, x0 - d), Math.max(0, y0 - d), Math.min(1, x1 + d), Math.min(1, y1 + d)];
}

function rdTex(latex, display) {
  const el = h("span", { class: "rd-tex" });
  try { katex.render(latex, el, { throwOnError: false, displayMode: !!display }); } catch { el.textContent = latex; }
  return el;
}

function rdEquations(body) {
  const { equations, symbols } = RD.paper;
  if (!equations.length && !symbols.length) { body.append(h("div", { class: "rd-empty" }, "這篇還沒有符號表")); return; }
  if (equations.length) {
    body.append(h("h4", {}, "方程式"));
    for (const e of equations) {
      const pic = ib("image", "插入裁切圖", () => rdInsertPicture(e.page, rdPad(e.bbox)));
      pic.classList.add("small");
      pic.disabled = !RD.paper.paper.pdf || !e.bbox;
      const omml = ib("sigma", "插入方程式", () => rdInsertEquation(e.latex, e.id, null));
      omml.classList.add("small");
      body.append(h("div", { class: "rd-item rd-eq" }, rdTex(e.latex, true), h("span", { class: "rd-meta" }, e.number ? `(${e.number}) ` : "", `p.${e.page}`), h("span", { class: "rd-btns" }, pic, omml)));
    }
  }
  if (symbols.length) {
    body.append(h("h4", {}, "符號"));
    for (const s of symbols) {
      const meaning = s.meaning || s.meaning_en || "";
      body.append(h("button", { class: "rd-item rd-sym", title: "插入", onclick: () => rdInsertEquation(s.latex, s.id, meaning) },
        rdTex(s.latex), h("span", { class: "rd-mean" }, meaning || "—"), s.page ? h("span", { class: "rd-meta" }, `p.${s.page}`) : null));
    }
  }
}

function rdNotes(body) {
  const { notes, quotes } = RD.paper;
  if (!notes.length && !quotes.length) { body.append(h("div", { class: "rd-empty" }, "這篇還沒有螢光或註記")); return; }
  const item = (text, sub, page, color) => h("button", { class: "rd-item rd-note", title: "插入", onclick: () => rdInsertText(text) },
    color ? h("i", { class: `rd-dot ${color}` }) : null, h("span", { class: "rd-ntext" }, text), sub ? h("span", { class: "rd-sub" }, sub) : null, h("span", { class: "rd-meta" }, `p.${page}`));
  if (notes.length) {
    body.append(h("h4", {}, "螢光與註記"));
    for (const n of notes) body.append(n.text ? item(n.text, n.note, n.page, n.color) : item(n.note, null, n.page, n.color));
  }
  if (quotes.length) {
    body.append(h("h4", {}, "引用句"));
    for (const q of quotes) body.append(item(q.text, q.note, q.page));
  }
}

// ── Inserting ──────────────────────────────────────────────────────────────
function rdCrop([x0, y0, x1, y1]) {
  return { l: x0, t: y0, r: 1 - x1, b: 1 - y1 };
}

async function rdInsertPicture(page, rect) {
  if (!rect || !S.deck) return;
  try {
    const r = await api(deckApi("/reader/picture"), { body: { sha: RD.sha, page } });
    RD.pics[r.asset] = { sha: RD.sha, page, title: RD.paper.paper.title };
    const [x0, y0, x1, y1] = rect;
    let w = (x1 - x0) * r.w, hh = (y1 - y0) * r.h;
    const k = Math.min((S.deck.size.w * 0.6) / w, (S.deck.size.h * 0.6) / hh);
    w = Math.round(w * k); hh = Math.round(hh * k);
    const [x, y] = freeSpot(Math.round((S.deck.size.w - w) / 2), Math.round((S.deck.size.h - hh) / 2), w, hh);
    const name = `${RD.paper.paper.title.slice(0, 40)} p.${page}`;
    await insertObj({ ...baseObj("picture", { x, y, w, h: hh }, name), asset: r.asset, crop: rdCrop(rect), alpha: 1, clip: "rect" }, "插入論文圖");
  } catch (e) { toast(e.message, true); }
}

/** Same page, new region: the picture keeps its width and place. */
async function rdRecrop(o) {
  if (!RD.rect) return;
  const info = RD.pics[o.asset];
  const img = new Image();
  img.src = ctxFor(slide()).assetUrl(o.asset);
  try { await img.decode(); } catch { return; }
  const [x0, y0, x1, y1] = RD.rect;
  const hh = Math.round(o.frame.w * ((y1 - y0) * img.naturalHeight) / ((x1 - x0) * img.naturalWidth));
  if (!info) return;
  await commit([setObj(o.id, "/crop", rdCrop(RD.rect)), setObj(o.id, "/frame", { ...o.frame, h: hh })], "裁切");
}

async function rdRegister(ids) {
  try {
    const r = await api(deckApi("/reader/notation"), { body: { sha: RD.sha, ids } });
    if (r.added?.length) toast(`對照表新增 ${r.added.length} 筆`);
  } catch (e) { toast(e.message, true); }
}

/** A native equation box (LaTeX → OMML via lore-math); with a meaning, "symbol：meaning" in the body tag. */
async function rdInsertEquation(latex, id, meaning) {
  if (!S.deck || !latex) return;
  const tag = defaultTag();
  const body = tagLook(tag) ?? { font: "Calibri", size: 14, color: inkColor() };
  const solo = meaning === null;
  const size = solo ? (S.style?.tags?.equation?.size ?? 14) : body.size;
  let omml;
  try { omml = (await api("/api/math", { body: { latex, size } })).omml; } catch (e) { toast(e.message, true); return; }
  S.latex.set(omml, latex);
  let tb;
  if (solo) {
    const look = tagLook("equation") ?? { font: "Cambria Math", size, color: inkColor() };
    const mrun = { kind: "math", text: latex, tag: S.style?.tags?.equation ? "equation" : null, look, omml, latex };
    tb = { insets: [0, 0, 0, 0], anchor: "t", wrap: false, autofit: "shape", paras: [{ ...newPara([mrun], "equation"), end: look }] };
  } else {
    const mrun = { kind: "math", text: latex, tag: null, look: { ...body, font: "Cambria Math", ea_font: "Cambria Math" }, omml, latex };
    const runs = meaning ? [mrun, newRun(`：${meaning}`, tag)] : [mrun];
    tb = { insets: [91440, 45720, 91440, 45720], anchor: "t", wrap: false, autofit: "shape", paras: [newPara(runs, tag)] };
  }
  const [w, hh] = measureText(tb);
  const [x, y] = freeSpot(Math.round((S.deck.size.w - w) / 2), Math.round(S.deck.size.h * (solo ? 0.42 : 0.62) - hh / 2), w, hh);
  await insertObj({ ...baseObj("text", { x, y, w, h: hh }, solo ? "方程式" : "符號"), geom: { prst: "rect" }, fill: { kind: "none" }, text: tb, equation: solo || undefined }, solo ? "插入方程式" : "插入符號");
  if (id) await rdRegister([id]);
}

async function rdInsertText(text) {
  if (!S.deck || !text) return;
  const tag = defaultTag();
  const w = Math.round(S.deck.size.w * 0.45);
  const tb = { insets: [91440, 45720, 91440, 45720], anchor: "t", wrap: true, autofit: "shape", paras: [newPara([newRun(text, tag)], tag)] };
  const lines = Math.max(1, Math.ceil(text.length / 60));
  const hh = Math.round(((tagLook(tag)?.size ?? 14) * 1.4 * lines / 72) * IN + 91440);
  const [x, y] = freeSpot(Math.round(S.deck.size.w * 0.08), Math.round(S.deck.size.h * 0.3), w, hh);
  await insertObj({ ...baseObj("text", { x, y, w, h: hh }, "論文筆記"), geom: { prst: "rect" }, fill: { kind: "none" }, text: tb }, "插入筆記");
}

/** `Ref:` small, bottom left (tag `source`, else `caption`). */
async function rdInsertRef() {
  if (!S.deck || !RD.paper) return;
  const tags = S.style?.tags ?? {};
  const tag = tags.source ? "source" : tags.caption ? "caption" : defaultTag();
  const size = tagLook(tag)?.size ?? 9;
  const hh = Math.round((size * 1.5 / 72) * IN + 91440);
  const tb = { insets: [91440, 45720, 91440, 45720], anchor: "b", wrap: true, autofit: "shape", paras: [newPara([newRun(RD.paper.ref, tag)], tag)] };
  const x = Math.round(0.3 * IN), y = S.deck.size.h - hh - Math.round(0.15 * IN);
  await insertObj({ ...baseObj("text", { x, y, w: Math.round(S.deck.size.w * 0.7), h: hh }, "Ref"), geom: { prst: "rect" }, fill: { kind: "none" }, text: tb }, "插入 Ref");
}

// ── Following the canvas selection ─────────────────────────────────────────
function rdSelectedPic() {
  if (S.sel.length !== 1) return null;
  const o = findObj(S.sel[0]);
  const info = o?.type === "picture" ? RD.pics[o.asset] : null;
  return info ? { o, info } : null;
}

/** A paper picture selected on the slide opens its page with its crop, ready to change. */
function rdFollowSelection() {
  const key = `${S.cur}|${S.sel.join(",")}`;
  if (!RD.open || key === RD.lastSel) return;
  RD.lastSel = key;
  const t = rdSelectedPic();
  if (!t) { if (RD.tab === "fig" && RD.paper) rdRender(); return; }
  const go = async () => {
    if (RD.sha !== t.info.sha) {
      if (!RD.papers?.some((p) => p.sha === t.info.sha)) return;
      await rdChoose(t.info.sha, false);
    }
    const c = t.o.crop ?? { l: 0, t: 0, r: 0, b: 0 };
    RD.tab = "fig";
    RD.page = t.info.page;
    RD.rect = [c.l, c.t, 1 - c.r, 1 - c.b];
    rdRender();
  };
  go().catch((e) => toast(e.message, true));
}

{
  const base = renderSide;
  let deck = null;
  // eslint-disable-next-line no-global-assign
  renderSide = function () {
    base();
    if (S.name !== deck) {
      // another deck: its own paper choice and pictures
      deck = S.name;
      Object.assign(RD, { sha: null, paper: null, page: null, rect: null, pics: {}, lastSel: "" });
      if (RD.open) { rdToggle(!!S.name); return; }
    }
    rdFollowSelection();
  };
}

// the editor builds its toolbar once; the button joins it
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", rdButton);
else rdButton();
