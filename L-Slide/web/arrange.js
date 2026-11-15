// 版型 (recipes): blocks placed on the slide (fractions of the slide), each
// with a role — title, text, image, card, table, caption, takeaway, a coloured
// panel — plus the composition's guide lines (輔助線). Two sources:
//   recipes.json             GraphicDesigner's 16:9 recipes (scripts/import-gd-recipes.mjs)
//   arrangements/*.json      the author's own (private workspace), same block format
// A slide remembers the recipe id it was laid out with (slide.arrangement).
"use strict";

const ARR = { catalog: null, filter: "academic", target: null };

async function loadArrangements(force = false) {
  if (S.arr && !force) return S.arr;
  const [cat, mine] = await Promise.all([
    ARR.catalog ? Promise.resolve(ARR.catalog) : fetch("recipes.json").then((r) => r.json()).catch(() => ({ recipes: [] })),
    api(deckApi("/arrangements")),
  ]);
  ARR.catalog = cat;
  // own layouts name a composition; its lines (default parameters) are their 輔助線
  const compo = (p) => cat.compositions?.[p.composition ?? "rule-of-thirds"] ?? {};
  const own = (mine.presets ?? []).map((p) => ({ guides: compo(p).guides ?? [], anchors: compo(p).anchors ?? [], ...p, own: true, blocks: (p.blocks ?? []).map(normBlock) }));
  S.arr = { recipes: [...own, ...cat.recipes.map((r) => ({ ...r, blocks: r.blocks.map(normBlock) }))], pages: mine.pages ?? [] };
  return S.arr;
}
/** Own recipes write rect as [x, y, w, h]; the catalog as {x, y, w, h}. */
function normBlock(b) {
  const r = Array.isArray(b.rect) ? { x: b.rect[0], y: b.rect[1], w: b.rect[2], h: b.rect[3] } : b.rect;
  return { shape: "rect", filled: false, panel: null, points: null, ...b, rect: r };
}
const recipeById = (id) => S.arr?.recipes.find((r) => r.id === id);
/** 學術: the author's own and plain rectangular splits without colour panels; 設計: the rest. */
function isAcademic(r) {
  if (r.class) return r.class === "academic";
  return r.cut === "rect" && !r.blocks.some((b) => b.panel || b.filled || b.role === "background" || b.shape !== "rect");
}

// ── Blocks → objects ──────────────────────────────────────────────────────
function tagOr(...tags) { return tags.find((t) => S.style?.tags?.[t]) ?? defaultTag(); }
function palette(role) {
  const c = S.style?.palette?.colors ?? {}, t = S.style?.palette?.tints ?? {};
  const first = (...k) => k.map((x) => c[x]).find(Boolean);
  const v = {
    primary: first("accent-1", "accent") ?? paletteHex()[0]?.hex,
    accent: first("emphasis", "accent-2", "accent-1"),
    dark: first("inverse", "ink") ?? inkColor(),
    light: first("surface") ?? (t.card ?? t.neutral ?? Object.values(t)[0])?.hex,
    onDark: first("on-inverse") ?? "FFFFFF",
  }[role];
  return (v ?? "FFFFFF").toUpperCase();
}
const darkPanel = (p) => p === "primary" || p === "dark" || p === "accent";
function para(tag, color) { const p = newPara([], tag); if (color && p.end) p.end = { ...p.end, color }; return p; }

/** Geometry of a block: rectangle, ellipse, or polygon (custom path in its own box). */
function blockGeom(b, f) {
  if (b.shape === "ellipse") return { prst: "ellipse" };
  if (b.points) {
    const W = S.deck.size.w, H = S.deck.size.h;
    const d = b.points.map((p, i) => `${i ? "L" : "M"}${((p.x * W - f.x) / f.w).toFixed(5)} ${((p.y * H - f.y) / f.h).toFixed(5)}`).join(" ") + " Z";
    return { prst: "custom", path: d };
  }
  return { prst: "rect" };
}

/** The objects a recipe puts on a slide. Ids are set later; `slot` names keep roles for re-layout. */
function recipeObjects(r) {
  const W = S.deck.size.w, H = S.deck.size.h;
  const out = [], n = {};
  const name = (role) => { n[role] = (n[role] ?? 0) + 1; return `${role}-${n[role]}`; };
  for (const b of r.blocks) {
    const f = { x: Math.round(b.rect.x * W), y: Math.round(b.rect.y * H), w: Math.round(b.rect.w * W), h: Math.round(b.rect.h * H), rot: 0 };
    const geom = blockGeom(b, f);
    const fill = b.panel ? { kind: "solid", color: palette(b.panel) } : b.filled ? { kind: "solid", color: palette("primary") } : { kind: "none" };
    const onFill = b.panel ? (darkPanel(b.panel) ? palette("onDark") : null) : b.filled ? palette("onDark") : null;
    const inset = b.panel || b.filled ? [228600, 152400, 228600, 152400] : [0, 0, 0, 0];
    const text = (paras, anchor = "t") => ({ insets: inset, anchor, wrap: true, autofit: "none", paras });
    const role = b.role;
    let o = null;
    if (role === "image" || role === "background" || role === "logo" || (role === "subject" && !b.filled && !b.panel)) {
      o = { slot: name("image"), type: "shape", geom, fill: { kind: "none" } };
    } else if (role === "title") {
      o = { slot: name("title"), type: b.panel || b.filled ? "shape" : "text", geom, fill, text: text([para(tagOr("title"), onFill)], "ctr") };
    } else if (role === "text") {
      o = { slot: name("text"), type: b.panel || b.filled ? "shape" : "text", geom, fill, text: text([para(tagOr("label", "subtitle"), onFill), para(tagOr("body"), onFill)]) };
    } else if (role === "caption") {
      o = { slot: name("caption"), type: "text", geom, fill: { kind: "none" }, text: text([para(tagOr("caption", "col-head", "body"))], "b") };
    } else if (role === "card") {
      o = { slot: name("card"), type: "shape", geom: { prst: "roundRect", adj: { adj: 8000 } }, fill: { kind: "solid", color: palette("light") },
        text: { ...text([para(tagOr("label")), para(tagOr("body"))]), insets: [152400, 121920, 152400, 121920] } };
    } else if (role === "takeaway") {
      o = { slot: name("takeaway"), type: "shape", geom: { prst: "roundRect", adj: { adj: 12000 } }, fill: { kind: "solid", color: palette("dark") },
        text: { ...text([para(tagOr("takeaway"), palette("onDark"))], "ctr"), insets: [228600, 45720, 228600, 45720] } };
    } else if (role === "table") {
      const nr = 4, nc = 3, rh = Math.round(Math.min(f.h / nr, 0.42 * IN));
      const cell = () => ({ text: { insets: [91440, 45720, 91440, 45720], anchor: "t", wrap: true, autofit: "none", paras: [newPara([], tagOr("table"))] }, borders: [null, null, null, null] });
      o = { slot: name("table"), type: "table", frame: { ...f, h: rh * nr }, cols: Array(nc).fill(Math.round(f.w / nc)), rows: [...Array(nr)].map(() => ({ h: rh, cells: [...Array(nc)].map(cell) })), style_id: "{5940675A-B579-460E-94D1-54222C63F5DA}" };
    } else if (role === "cta") {
      o = { slot: name("text"), type: "shape", geom, fill: { kind: "solid", color: palette(b.panel ?? "accent") }, text: text([para(tagOr("label"), palette("onDark"))], "ctr") };
    } else if ((role === "subject" || role === "other") && (b.panel || b.filled)) {
      // colour panels: part of the design (kept and exported, never an "empty" slot)
      o = { slot: name("deco"), type: "shape", geom, fill };
    }
    if (!o) continue;
    out.push({ id: 0, name: b.name ?? o.slot, frame: f, ...o, ...(o.frame ? { frame: o.frame } : {}) });
  }
  // colour panels go to the back
  return [...out.filter((o) => slotKind(o) === "deco"), ...out.filter((o) => slotKind(o) !== "deco")];
}
const slotKind = (o) => (o.slot ?? "").replace(/-\d+$/, "");

/** JS twin of lore-canvas `Object::is_empty_slot` (decorations are never empty). */
function isEmptySlot(o) {
  if (!o.slot || slotKind(o) === "deco") return false;
  const blank = (t) => !t || t.paras.every((p) => p.runs.every((r) => r.kind !== "math" && !(r.text ?? "").trim()));
  if (o.type === "text" || o.type === "shape") return blank(o.text);
  if (o.type === "table") return o.rows.every((r) => r.cells.every((c) => blank(c.text)));
  return false;
}

// ── Applying ──────────────────────────────────────────────────────────────
/** New slide (Enter, ＋): the deck default for content pages, laid out right away. */
async function addArrangedSlide(kind = "content") {
  await loadArrangements();
  // the new page takes this page's recipe (the cover's own design is not repeated)
  const here = slide();
  const fromHere = here && here !== S.deck.slides[0] ? here.arrangement : null;
  const r = recipeById(fromHere) ?? recipeById(S.deck.defaults?.[kind]);
  let objects = r ? recipeObjects(r) : [];
  // a recipe brings its own title: the layout's title placeholder is not wanted twice
  try {
    absorb(await api(deckApi("/new-slide"), { body: { after: S.cur, kind, arrangement: r?.id ?? null, objects } }));
    const i = S.deck.slides.findIndex((s) => s.uid === S.cur);
    const uid = S.deck.slides[i + 1]?.uid ?? S.cur;
    gotoSlide(uid);
    if (r) await dedupeTitle();
  } catch (e) { toast(e.message, true); }
}
/** When the layout gave a title placeholder and the recipe a title block, keep the placeholder at the block's place. */
async function dedupeTitle() {
  const sl = slide();
  const ph = sl.objects.find((o) => (o.ph === "title" || o.ph === "ctrTitle") && !o.slot);
  const blk = sl.objects.find((o) => slotKind(o) === "title" && isEmptySlot(o));
  if (ph && blk) await commit([setObj(ph.id, "/frame", blk.frame), { op: "delete_obj", slide: sl.uid, id: blk.id }], "版型標題");
}

/** Lay out this slide with a recipe (one undo step). Filled blocks move to the new block of
 *  the same role, in order; the title moves to the recipe's title; empty blocks and old
 *  colour panels go; anything else stays where it is. */
async function applyArrangement(id, extra = []) {
  const sl = slide(), r = recipeById(id);
  if (!sl || !r) return;
  const fresh = recipeObjects(r);
  const ops = [];
  let next = maxId() + 1;
  const used = new Set();
  const isTitle = (o) => slotKind(o) === "title" || o.ph === "title" || o.ph === "ctrTitle";
  for (const nw of fresh) {
    const k = slotKind(nw);
    const keep = k === "deco" ? null
      : sl.objects.find((o) => !used.has(o.id) && (k === "title" ? isTitle(o) && (!isEmptySlot(o) || o.ph) : slotKind(o) === k && !isEmptySlot(o)));
    if (keep) {
      used.add(keep.id);
      ops.push(setObj(keep.id, "/frame", keep.type === "picture" ? fitInto(keep, nw.frame) : nw.frame));
      if (!keep.ph) ops.push(setObj(keep.id, "/slot", nw.slot));
    } else {
      ops.push({ op: "insert_obj", slide: sl.uid, parent: null, index: k === "deco" ? 0 : 9999, obj: { ...nw, id: next++ } });
    }
  }
  for (const o of sl.objects) {
    if (used.has(o.id) || !o.slot) continue;
    if (isEmptySlot(o) || slotKind(o) === "deco") ops.push({ op: "delete_obj", slide: sl.uid, id: o.id });
    else ops.push(setObj(o.id, "/slot", null));
  }
  ops.push({ op: "set_slide", slide: sl.uid, path: "/arrangement", value: id }, ...extra);
  await commit(ops, "套用版型");
}

/** A picture's frame that keeps its aspect ratio inside `box` (centred). */
function fitInto(o, box) {
  const ar = o.frame.w / o.frame.h || 1;
  let w = box.w, h = Math.round(w / ar);
  if (h > box.h) { h = box.h; w = Math.round(h * ar); }
  return { x: Math.round(box.x + (box.w - w) / 2), y: Math.round(box.y + (box.h - h) / 2), w, h, rot: 0 };
}

/** An empty image block gets a picture (double-click or drop), cropped to cover the block. */
async function fillImageSlot(o, file) {
  const ext = (file.name.split(".").pop() || "png").toLowerCase();
  try {
    const r = await api(`${deckApi("/assets")}?ext=${encodeURIComponent(ext)}`, { body: await file.arrayBuffer(), raw: true });
    const url = URL.createObjectURL(file);
    const img = await new Promise((res, rej) => { const i = new Image(); i.onload = () => res(i); i.onerror = rej; i.src = url; });
    URL.revokeObjectURL(url);
    const box = o.frame, ia = img.naturalWidth / img.naturalHeight, ba = box.w / box.h;
    const crop = ia > ba ? { l: (1 - ba / ia) / 2, t: 0, r: (1 - ba / ia) / 2, b: 0 } : { l: 0, t: (1 - ia / ba) / 2, r: 0, b: (1 - ia / ba) / 2 };
    const clip = o.geom?.prst === "ellipse" ? "ellipse" : "rect";
    const pic = { id: o.id, name: file.name, slot: o.slot, type: "picture", asset: r.asset, crop, alpha: 1, clip, frame: { ...box } };
    await commit([setObj(o.id, "", pic)], "放入圖片");
  } catch (e) { toast(e.message, true); }
}

// ── 輔助線: the recipe's composition lines and anchors, over everything ────
const GUIDES = { on: (() => { try { return localStorage.getItem("sc.guides") === "1"; } catch { return false; } })() };
function toggleGuides(on) {
  GUIDES.on = on;
  try { localStorage.setItem("sc.guides", on ? "1" : "0"); } catch {}
  renderGuides();
  renderSide(); // thumbnails show the lines too
}
function guideSvg(r, W, H, cls = "guide-layer") {
  const NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("class", cls);
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  svg.setAttribute("width", W);
  svg.setAttribute("height", H);
  const add = (tag, attrs, c) => { const e = document.createElementNS(NS, tag); for (const [a, v] of Object.entries(attrs)) e.setAttribute(a, v); e.setAttribute("class", c); svg.append(e); };
  for (const p of r.guides ?? []) {
    const c = `g-${p.visual ? "v" : "c"}-${p.weight === "sub" ? "sub" : "main"}`;
    if (p.kind === "line") add("line", { x1: p.a.x * W, y1: p.a.y * H, x2: p.b.x * W, y2: p.b.y * H }, c);
    else if (p.kind === "polyline") add(p.closed ? "polygon" : "polyline", { points: p.points.map((q) => `${q.x * W},${q.y * H}`).join(" ") }, c);
    else if (p.kind === "circle") add("ellipse", { cx: p.c.x * W, cy: p.c.y * H, rx: p.rx * W, ry: p.ry * H }, c);
  }
  for (const a of r.anchors ?? []) add("circle", { cx: a.x * W, cy: a.y * H, r: 3 }, "g-anchor");
  return svg;
}
function renderGuides() {
  const wrap = $("#paperWrap");
  wrap.querySelector(".guide-layer")?.remove();
  const sl = slide();
  const r = sl && recipeById(sl.arrangement);
  if (!GUIDES.on || !r) return;
  wrap.querySelector(".overlay").before(guideSvg(r, (S.deck.size.w / EMU) * S.zoom, (S.deck.size.h / EMU) * S.zoom));
}

// ── Thumbnails: the recipe on this deck's page, with sample text ─────────
function recipeThumb(r, layout) {
  const fill = (o, vals) => o.text && { ...o.text, paras: o.text.paras.map((p, j) => ({ ...p, runs: vals[j] ? [{ kind: "text", text: vals[j], look: p.end }] : [] })) };
  const sample = { title: ["標題文字"], text: ["子標題", "內文段落，按 Enter 分段。"], caption: ["圖說"], card: ["卡片標題", "內文"], takeaway: ["重點一句話"] };
  const objs = recipeObjects(r).map((o, i) => {
    const k = slotKind(o);
    const vals = o.fill?.kind === "solid" && k === "text" && o.text?.paras.length === 1 ? ["標籤"] : sample[k];
    return { ...o, id: i + 2, slot: null, ...(vals && o.text ? { text: fill(o, vals) } : {}), ...(k === "image" ? { fill: { kind: "solid", color: "C9CCD3" } } : {}) };
  });
  const node = SC.renderSlide({ uid: `thumb-${r.id}`, layout, objects: objs }, { size: S.deck.size, backdrops: S.backdrops, slideNo: 2, assetUrl: () => "", latex: () => undefined, needLatex: () => {} });
  // 輔助線 on: the composition lines over the thumbnail too
  if (GUIDES.on) node.append(guideSvg(r, SC.px(S.deck.size.w), SC.px(S.deck.size.h), "guide-layer thumb-guides"));
  return node;
}

/** The page a default is about: the cover = first page, the end page = last page when it uses
 *  the end layout, content = this page unless it is the cover / end, else the first content page. */
function pageForKind(kind) {
  const ss = S.deck.slides, endL = S.style?.layouts?.end;
  const isEnd = (x) => endL && x.layout === endL && x === ss.at(-1);
  if (kind === "cover") return ss[0];
  if (kind === "end") return ss.length > 1 && isEnd(ss.at(-1)) ? ss.at(-1) : null;
  const cur = slide();
  if (cur && cur !== ss[0] && !isEnd(cur)) return cur;
  return ss.find((x, i) => i > 0 && !isEnd(x)) ?? null;
}
