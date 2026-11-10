// Side panel (format / animations / slide), dialogs (table, equation,
// components, progressive reveal). Text formatting goes through style-pack tags
// and emphasis tags only — no free font or size.
"use strict";

const CM = 360000;
const h = (tag, attrs = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else if (k.startsWith("on")) e[k] = v;
    else if (k === "style") e.style.cssText = v;
    else if (v !== false && v != null) e.setAttribute(k, v === true ? "" : v);
  }
  for (const k of kids.flat()) if (k != null && k !== false) e.append(k.nodeType ? k : document.createTextNode(k));
  return e;
};
const sec = (title, ...kids) => h("div", { class: "sec" }, title ? h("h4", {}, title) : null, ...kids);
const row = (...kids) => h("div", { class: "row" }, ...kids);
const ib = (icon, label, onclick, pressed) => { const b = iconButton(icon, label, onclick); if (pressed !== undefined) b.setAttribute("aria-pressed", pressed ? "true" : "false"); return b; };

function swatches(cur, onpick, { none = true } = {}) {
  const box = h("div", { class: "swatches" });
  if (none) box.append(h("button", { class: "sw none", title: "無", "aria-pressed": cur == null ? "true" : "false", onclick: () => onpick(null) }));
  for (const { hex, name } of paletteHex()) {
    box.append(h("button", { class: "sw", title: `${name} #${hex}`, style: `background:#${hex}`, "aria-pressed": cur === hex ? "true" : "false", onclick: () => onpick(hex) }));
  }
  return box;
}
// Display names for style-pack keys: the pack's own `label`/`name` when it has
// one, else this map, else the key. The key stays visible as secondary text.
const ZH = {
  tags: { "cover-title": "封面標題", venue: "期刊／年份", subtitle: "副標題", title: "標題", body: "內文", "body-dense": "內文（密集）", takeaway: "重點結論", label: "小標", caption: "圖說", table: "表格", reference: "參考文獻", equation: "方程式", note: "註記", heading: "小節標題" },
  emphasis: { key: "關鍵結果", note: "符號註記", "concept-1": "概念 1", "concept-2": "概念 2", "concept-3": "概念 3", strong: "粗體", ghost: "變淡" },
  alpha: { overlay: "遮罩", "overlay-strong": "遮罩（深）", "overlay-light": "遮罩（淺）", "ghost-picture": "變淡圖片", "ghost-picture-light": "變淡圖片（淺）", "ghost-line": "變淡線條" },
  arrows: { default: "標準箭頭", bold: "粗箭頭", elbow: "肘形箭頭", dashed: "虛線箭頭", curved: "曲線箭頭" },
  components: { "numbered-circle": "編號圓圈", "model-trapezoid": "模型梯形", "tab-card": "標籤卡", "highlight-box": "重點框（紅框）", "semantic-overlay": "語意淡色遮罩" },
};
function zh(kind, key) {
  const def = { tags: S.style?.tags, emphasis: S.style?.emphasis, arrows: S.style?.arrows, components: S.style?.components }[kind]?.[key];
  return def?.label ?? def?.name ?? ZH[kind]?.[key] ?? key;
}
function alphaSelect(cur, onpick) {
  const levels = [["100%", 1], ...Object.entries(S.style?.palette?.alpha ?? {}).map(([k, v]) => [`${zh("alpha", k)} ${Math.round(v * 100)}% · ${k}`, v])];
  if (!levels.some(([, v]) => Math.abs(v - cur) < 1e-6)) levels.push([`${Math.round(cur * 100)}%`, cur]);
  const s = h("select", { onchange: (e) => onpick(+e.target.value) }, levels.map(([l, v]) => h("option", { value: v, selected: Math.abs(v - cur) < 1e-6 }, l)));
  return s;
}

// ── Text formatting ───────────────────────────────────────────────────────
function textTargets() {
  return selObjs().filter((o) => o.text || o.type === "table");
}
function eachBody(o, fn) {
  if (o.text) fn(o.text);
  if (o.type === "table") o.rows.forEach((r) => r.cells.forEach((c) => fn(c.text)));
}
async function bodyChange(label, fnRuns, fnParas) {
  if (S.editing) {
    editTransform((tb, sel) => {
      let a = sel.a, b = sel.b;
      if (a.p === b.p && a.o === b.o) { a = { p: a.p, o: 0 }; b = { p: a.p, o: Infinity }; }
      if (fnRuns) fnRuns(runsInRange(tb, a, b).filter((r) => r.kind === "text" || r.kind === "field"));
      if (fnParas) fnParas(selectedParas(tb, sel));
    });
    renderSide();
    return;
  }
  const ops = [];
  for (const o of textTargets()) {
    const c = clone(o);
    eachBody(c, (tb) => {
      if (fnRuns) fnRuns(tb.paras.flatMap((p) => p.runs).filter((r) => r.kind === "text" || r.kind === "field"));
      if (fnParas) fnParas(tb.paras);
    });
    if (o.type === "table") ops.push(setObj(o.id, "/rows", c.rows));
    else ops.push(setObj(o.id, "/text", c.text));
  }
  await commit(ops, label);
}
const applyTag = (tag) => bodyChange("套用標籤", (runs) => runs.forEach((r) => { r.tag = tag; r.look = tagLook(tag, r.em) ?? r.look; }), (paras) => paras.forEach((p) => { if (!p.runs.length) p.end = tagLook(tag) ?? p.end; }));
function applyEm(em, on) {
  return bodyChange("強調", (runs) => runs.forEach((r) => {
    r.em = on ? em : null;
    if (r.tag) r.look = tagLook(r.tag, r.em, r.look.size) ?? r.look;
    else if (on && S.style.emphasis[em]?.color) { r.look = { ...r.look, color: S.style.emphasis[em].color.toUpperCase(), bold: r.look.bold || !!S.style.emphasis[em].bold }; }
  }));
}
const applyPara = (label, fn) => bodyChange(label, null, (paras) => paras.forEach(fn));
const applySize = (size) => bodyChange("字級（標籤範圍內）", (runs) => runs.forEach((r) => { if (r.tag) r.look = { ...r.look, size }; }));

/** Current text info for the panel (from the edit selection or the whole object). */
function textInfo() {
  let tbs = [];
  let paras = [];
  if (S.editing) {
    const root = editRoot();
    const o = findObj(S.editing);
    if (root && o) {
      const tb = domToText(root, S.editOrig);
      const sel = getTextSel(root) ?? { a: { p: 0, o: 0 }, b: { p: 0, o: 0 } };
      paras = selectedParas(tb, sel);
      tbs = [tb];
    }
  } else {
    for (const o of textTargets()) eachBody(o, (tb) => { tbs.push(tb); paras.push(...tb.paras); });
  }
  const runs = paras.flatMap((p) => p.runs).filter((r) => r.kind === "text" && r.text.trim());
  const uniq = (v) => [...new Set(v)];
  return { tbs, paras, runs, tags: uniq(runs.map((r) => r.tag ?? "")), ems: uniq(runs.map((r) => r.em ?? "")) };
}

function textSection() {
  const t = textInfo();
  if (!t.tbs.length) return null;
  const tags = Object.entries(S.style?.tags ?? {});
  const curTag = t.tags.length === 1 ? t.tags[0] : t.tags.length ? "*" : t.paras[0]?.end ? "" : "";
  const sel = h("select", { onchange: (e) => e.target.value && e.target.value !== "*" && applyTag(e.target.value) },
    t.tags.length > 1 ? h("option", { value: "*", selected: true }, "（多個標籤）") : null,
    t.tags.includes("") ? h("option", { value: "", selected: t.tags.length === 1 }, "（未標籤：匯入的格式）") : null,
    tags.map(([k, v]) => h("option", { value: k, selected: curTag === k }, `${zh("tags", k)} ${v.size}pt${v.bold ? " 粗體" : ""} · ${k}`)));
  const ems = h("div", { class: "row" });
  for (const [k, e] of Object.entries(S.style?.emphasis ?? {})) {
    const on = t.ems.length === 1 && t.ems[0] === k;
    ems.append(h("button", { class: "em", "aria-pressed": on ? "true" : "false", title: `${k}${e.meaning ? `：${e.meaning}` : ""}${k === "strong" ? "（⌘B）" : ""}`, onclick: () => applyEm(k, !on) }, h("i", { style: `background:#${e.color ?? "000"}` }), zh("emphasis", k), h("small", {}, k)));
  }
  ems.append(h("button", { class: "em", onclick: () => applyEm(null, false) }, "清除"));
  const tagDef = S.style?.tags?.[curTag];
  const sizeCtl = tagDef?.size_range ? row(h("label", {}, "字級", h("input", { type: "number", step: 0.5, min: tagDef.size_range[0], max: tagDef.size_range[1], value: t.runs[0]?.look.size ?? tagDef.size, onchange: (e) => applySize(Math.min(tagDef.size_range[1], Math.max(tagDef.size_range[0], +e.target.value))) })), h("span", { class: "info" }, `${tagDef.size_range[0]}–${tagDef.size_range[1]}pt`)) : null;
  const p0 = t.paras[0] ?? {};
  const alignSeg = h("div", { class: "seg" }, [["l", "tL", "靠左"], ["ctr", "tC", "置中"], ["r", "tR", "靠右"], ["just", "tJ", "左右對齊"]].map(([v, ic, lab]) => {
    const b = h("button", { "aria-pressed": p0.align === v ? "true" : "false", "aria-label": lab, title: lab, onclick: () => applyPara("對齊", (p) => (p.align = v)) });
    b.append(icon(ic));
    return b;
  }));
  const buKind = p0.bullet?.kind === "num" ? "num" : p0.bullet?.kind === "char" ? "char" : "none";
  const setBullet = (kind, scheme) => applyPara("項目符號", (p) => {
    p.bullet = kind === "none" ? { kind: "none" } : kind === "char" ? { kind: "char", ch: (p.level ?? 0) ? "–" : "•" } : { kind: "num", scheme, start: 1 };
    setLevel(p, p.level ?? 0);
  });
  const bullets = h("div", { class: "seg" },
    h("button", { "aria-pressed": buKind === "none" ? "true" : "false", onclick: () => setBullet("none") }, "無"),
    h("button", { "aria-pressed": buKind === "char" ? "true" : "false", onclick: () => setBullet("char") }, "• 項目"),
    h("button", { "aria-pressed": buKind === "num" && p0.bullet.scheme === "arabicPeriod" ? "true" : "false", onclick: () => setBullet("num", "arabicPeriod") }, "1."),
    h("button", { "aria-pressed": buKind === "num" && p0.bullet.scheme === "alphaLcPeriod" ? "true" : "false", onclick: () => setBullet("num", "alphaLcPeriod") }, "a."));
  const lvl = row(ib("outdent", "減少階層", () => applyPara("階層", (p) => setLevel(p, 0))), ib("indent", "增加階層（第 2 層）", () => applyPara("階層", (p) => setLevel(p, 1))), h("span", { class: "info" }, `階層 ${(p0.level ?? 0) + 1}`));
  const ls = h("select", { onchange: (e) => applyPara("行距", (p) => { p.line = +e.target.value; delete p.line_pt; }) },
    [0.9, 1, 1.15, 1.2, 1.5, 2].map((v) => h("option", { value: v, selected: Math.abs((p0.line ?? 1) - v) < 0.001 }, `${v} 倍`)));
  const before = h("input", { type: "number", step: 1, min: 0, value: p0.before_pt ?? 0, title: "段前 pt", onchange: (e) => applyPara("段距", (p) => (p.before_pt = +e.target.value || undefined)) });
  const tb0 = t.tbs[0];
  const anchor = h("div", { class: "seg" }, [["t", "上"], ["ctr", "中"], ["b", "下"]].map(([v, l]) => h("button", { "aria-pressed": tb0?.anchor === v ? "true" : "false", onclick: () => setBodyProp("anchor", v) }, l)));
  return sec("文字",
    row(h("label", {}, "標籤", sel)),
    sizeCtl,
    h("div", { class: "info" }, "強調"), ems,
    row(alignSeg, anchor),
    row(bullets), lvl,
    row(h("label", {}, "行距", ls), h("label", {}, "段前", before)),
    t.tags.includes("") ? h("div", { class: "warn-box" }, "部分文字沒有對應的標籤（保留匯入格式）。選一個標籤即可統一。") : null);
}
async function setBodyProp(k, v) {
  if (S.editing) await exitEdit();
  const ops = selObjs().filter((o) => o.text).map((o) => setObj(o.id, `/text/${k}`, v));
  await commit(ops, "文字方塊");
}

// ── Object sections ───────────────────────────────────────────────────────
function arrangeSection(objs) {
  const one = objs.length === 1 ? objs[0] : null;
  const box = h("div", { class: "grid4" });
  if (one) {
    const f = one.frame;
    // handlers read the object at event time: the field may outlive one panel refresh (it keeps focus)
    const live = () => findObj(one.id)?.frame ?? f;
    const num = (label, key, scale = CM, digits = 2) => h("label", {}, label, h("input", { type: "number", step: 0.01, "data-k": `frame-${key}`, value: (f[key] / scale).toFixed(digits), onchange: (e) => commit([setObj(one.id, "/frame", { ...live(), [key]: Math.round(+e.target.value * scale) })], "位置") }));
    box.append(num("X cm", "x"), num("Y cm", "y"), num("寬 cm", "w"), num("高 cm", "h"));
    box.append(h("label", {}, "旋轉°", h("input", { type: "number", step: 1, "data-k": "frame-rot", value: Math.round(f.rot ?? 0), onchange: (e) => commit([setObj(one.id, "/frame", { ...live(), rot: ((+e.target.value % 360) + 360) % 360 })], "旋轉") })));
  }
  const alignTo = (how) => {
    const fs = objs.map((o) => o.frame);
    const b = fs.length === 1 ? { x: 0, y: 0, w: S.deck.size.w, h: S.deck.size.h } : bbox(fs);
    const ops = objs.map((o) => {
      const f = { ...o.frame };
      if (how === "left") f.x = b.x; if (how === "center") f.x = Math.round(b.x + (b.w - f.w) / 2); if (how === "right") f.x = b.x + b.w - f.w;
      if (how === "top") f.y = b.y; if (how === "middle") f.y = Math.round(b.y + (b.h - f.h) / 2); if (how === "bottom") f.y = b.y + b.h - f.h;
      return setObj(o.id, "/frame", f);
    });
    commit(ops, "對齊");
  };
  const distribute = (axis) => {
    if (objs.length < 3) return toast("至少選三個物件才能均分", true);
    const hz = axis === "h";
    const sorted = [...objs].sort((a, b) => (hz ? a.frame.x - b.frame.x : a.frame.y - b.frame.y));
    const total = sorted.reduce((s, o) => s + (hz ? o.frame.w : o.frame.h), 0);
    const first = sorted[0].frame, last = sorted[sorted.length - 1].frame;
    const span = hz ? last.x + last.w - first.x : last.y + last.h - first.y;
    const gap = (span - total) / (sorted.length - 1);
    let pos = hz ? first.x : first.y;
    const ops = sorted.map((o) => { const f = { ...o.frame }; if (hz) { f.x = Math.round(pos); pos += f.w + gap; } else { f.y = Math.round(pos); pos += f.h + gap; } return setObj(o.id, "/frame", f); });
    commit(ops, "均分");
  };
  return sec("排列",
    one ? box : h("div", { class: "info" }, `已選 ${objs.length} 個物件`),
    row(ib("alignL", "靠左對齊", () => alignTo("left")), ib("alignC", "水平置中", () => alignTo("center")), ib("alignR", "靠右對齊", () => alignTo("right")),
      ib("alignT", "靠上對齊", () => alignTo("top")), ib("alignM", "垂直置中", () => alignTo("middle")), ib("alignB", "靠下對齊", () => alignTo("bottom")),
      ib("distH", "水平均分", () => distribute("h")), ib("distV", "垂直均分", () => distribute("v"))),
    row(ib("front", "移到最上層 (⇧⌘])", () => zorder("front")), ib("sendBack", "移到最下層 (⇧⌘[)", () => zorder("back")),
      h("button", { class: "btn small", onclick: () => zorder("forward") }, "上移一層"), h("button", { class: "btn small", onclick: () => zorder("backward") }, "下移一層")),
    row(ib("group", "群組 (⌘G)", group), ib("ungroup", "取消群組 (⇧⌘G)", ungroup), ib("dup", "複製 (⌘D)", duplicate), ib("trash", "刪除", deleteSel),
      one ? h("label", { class: "row" }, h("input", { type: "checkbox", checked: !!one.hidden, onchange: (e) => commit([setObj(one.id, "/hidden", e.target.checked)], "隱藏物件") }), "隱藏") : null));
}

function fillLineSection(o) {
  const fill = o.fill ?? { kind: "none" };
  const fillHex = fill.kind === "solid" ? fill.color : null;
  const line = o.line;
  const geoms = [["rect", "矩形"], ["roundRect", "圓角矩形"], ["ellipse", "橢圓"], ["triangle", "三角形"], ["trapezoid", "梯形"], ["parallelogram", "平行四邊形"], ["diamond", "菱形"], ["homePlate", "五邊形箭頭"], ["chevron", "V 形箭頭"], ["rightArrow", "右箭號"]];
  const g = o.geom?.prst;
  return sec("填色與外框",
    o.type === "shape" || o.type === "text" ? row(h("label", {}, "形狀", h("select", { onchange: (e) => commit([setObj(o.id, "/geom", { prst: e.target.value })], "變更形狀") },
      [...geoms, ...(geoms.some(([k]) => k === g) ? [] : [[g, g === "custom" ? "自由形狀" : g]])].map(([k, l]) => h("option", { value: k, selected: k === g }, l))))) : null,
    h("div", { class: "info" }, fill.kind === "gradient" ? "填色：漸層（保留原樣）" : "填色"),
    swatches(fillHex, (hex) => commit([setObj(o.id, "/fill", hex ? { kind: "solid", color: hex, alpha: fill.alpha ?? 1 } : { kind: "none" })], "填色")),
    fill.kind === "solid" ? row(h("label", {}, "不透明度", alphaSelect(fill.alpha ?? 1, (a) => commit([setObj(o.id, "/fill", { ...fill, alpha: a })], "透明度")))) : null,
    h("div", { class: "info" }, "外框"),
    swatches(line?.color ?? null, (hex) => commit([setObj(o.id, "/line", hex ? { ...(line ?? { width_pt: 1, dash: "solid", head: "none", tail: "none" }), color: hex } : null)], "外框")),
    line ? strokeControls(o, line, "/line") : null);
}

function strokeControls(o, line, path) {
  const set = (patch, label) => commit([setObj(o.id, path, { ...line, ...patch })], label);
  return h("div", { class: "sec" },
    row(h("label", {}, "粗細", h("select", { onchange: (e) => set({ width_pt: +e.target.value }, "線條粗細") }, [0.5, 0.75, 1, 1.5, 2.25, 3, 4.5].concat(line.width_pt).filter((v, i, a) => a.indexOf(v) === i).sort((a, b) => a - b).map((v) => h("option", { value: v, selected: v === line.width_pt }, `${v} pt`)))),
      h("label", {}, "樣式", h("select", { onchange: (e) => set({ dash: e.target.value }, "虛線") }, [["solid", "實線"], ["dash", "虛線"], ["sysDash", "短虛線"], ["sysDot", "點線"], ["lgDash", "長虛線"], ["dashDot", "虛點線"]].map(([v, l]) => h("option", { value: v, selected: v === line.dash }, l))))),
    row(h("label", {}, "不透明度", alphaSelect(line.alpha ?? 1, (a) => set({ alpha: a }, "線條透明度")))));
}

function connectorSection(o) {
  const line = o.line ?? { color: inkColor(), width_pt: 1.5, dash: "solid", head: "none", tail: "triangle" };
  const ends = [["none", "無"], ["triangle", "三角"], ["stealth", "尖箭"], ["arrow", "開放"], ["oval", "圓"], ["diamond", "菱形"]];
  const set = (patch, label) => commit([setObj(o.id, "/line", { ...line, ...patch })], label);
  const presets = Object.entries(S.style?.arrows ?? {});
  return sec("線條／連接線",
    row(h("div", { class: "seg" }, [["straightConnector1", "直線"], ["bentConnector3", "肘形"], ["curvedConnector3", "曲線"]].map(([v, l]) => h("button", { "aria-pressed": o.geom === v ? "true" : "false", onclick: () => commit([setObj(o.id, "/geom", v)], "連接線種類") }, l)))),
    presets.length ? row(h("label", {}, "箭頭樣式", h("select", { onchange: (e) => { const a = S.style.arrows[e.target.value]; if (a) commit([setObj(o.id, "/line", { color: a.color.toUpperCase(), width_pt: a.width_pt, dash: a.dash, head: a.head || "none", tail: a.tail, alpha: line.alpha ?? 1 }), setObj(o.id, "/geom", a.connector)], "箭頭樣式"); } }, h("option", { value: "" }, "—"), presets.map(([k]) => h("option", { value: k }, `${zh("arrows", k)} · ${k}`))))) : null,
    row(h("label", {}, "起點", h("select", { onchange: (e) => set({ head: e.target.value }, "箭頭") }, ends.map(([v, l]) => h("option", { value: v, selected: v === line.head }, l)))),
      h("label", {}, "終點", h("select", { onchange: (e) => set({ tail: e.target.value }, "箭頭") }, ends.map(([v, l]) => h("option", { value: v, selected: v === line.tail }, l))))),
    swatches(line.color, (hex) => hex && set({ color: hex }, "線條顏色"), { none: false }),
    strokeControls(o, line, "/line"),
    o.start || o.end ? row(h("span", { class: "info" }, `黏著：${o.start ? `起點→#${o.start.id}` : ""} ${o.end ? `終點→#${o.end.id}` : ""}`), h("button", { class: "btn small", onclick: () => commit([setObj(o.id, "/start", null), setObj(o.id, "/end", null)], "解除黏著") }, "解除")) : null);
}

function pictureSection(o) {
  const addMask = () => commit([setObj(o.id, "/masks", [...(o.masks ?? []), { x: 0.35, y: 0.35, w: 0.3, h: 0.3 }])], "白色遮罩");
  return sec("圖片",
    row(h("label", {}, "不透明度", alphaSelect(o.alpha ?? 1, (a) => commit([setObj(o.id, "/alpha", a)], "圖片透明度")))),
    row(h("button", { class: "btn small", "aria-pressed": S.crop === o.id, onclick: () => { S.crop = S.crop === o.id ? null : o.id; renderOverlay(); renderSide(); } }, h("span", { "data-icon": "crop" }), S.crop === o.id ? "完成裁切" : "裁切"),
      h("button", { class: "btn small", onclick: () => { const c = o.crop, f = o.frame; const W = f.w / (1 - c.l - c.r), H = f.h / (1 - c.t - c.b); commit([setObj(o.id, "/crop", { l: 0, t: 0, r: 0, b: 0 }), setObj(o.id, "/frame", { ...f, x: Math.round(f.x - c.l * W), y: Math.round(f.y - c.t * H), w: Math.round(W), h: Math.round(H) })], "重設裁切"); } }, "重設裁切")),
    h("div", { class: "info" }, `裁切 左 ${(o.crop.l * 100).toFixed(1)}% 上 ${(o.crop.t * 100).toFixed(1)}% 右 ${(o.crop.r * 100).toFixed(1)}% 下 ${(o.crop.b * 100).toFixed(1)}%`),
    row(h("button", { class: "btn small", onclick: addMask }, "加白色遮罩"), (o.masks ?? []).length ? h("button", { class: "btn small", onclick: () => commit([setObj(o.id, "/masks", [])], "移除遮罩") }, `移除遮罩（${o.masks.length}）`) : null),
    (o.masks ?? []).map((m, i) => row(h("span", { class: "info" }, `遮罩 ${i + 1}`), ...["x", "y", "w", "h"].map((k) => h("input", { type: "number", step: 0.01, min: 0, max: 1, value: m[k].toFixed(2), style: "width:54px", onchange: (e) => { const ms = clone(o.masks); ms[i][k] = +e.target.value; commit([setObj(o.id, "/masks", ms)], "遮罩"); } })))),
    h("div", { class: "info" }, "外框"),
    swatches(o.line?.color ?? null, (hex) => commit([setObj(o.id, "/line", hex ? { ...(o.line ?? { width_pt: 2.25, dash: "solid", head: "none", tail: "none" }), color: hex } : null)], "圖片外框")),
    (o.effects ?? []).length ? h("div", { class: "info" }, `保留的影像效果：${o.effects.join("、")}`) : null);
}

function tableSection(o) {
  const ec = S.editingCell?.id === o.id ? S.editingCell : null;
  const r = ec?.r ?? o.rows.length - 1, c = ec?.c ?? o.cols.length - 1;
  const emptyCell = (src) => ({ text: { insets: [91440, 45720, 91440, 45720], anchor: "t", wrap: true, autofit: "none", paras: [newPara([], "table")] }, borders: [null, null, null, null], ...(src ? { src } : {}) });
  const t = clone(o);
  const save = (label) => commit([setObj(o.id, "", { ...o, cols: t.cols, rows: t.rows, frame: { ...o.frame, w: t.cols.reduce((a, b) => a + b, 0), h: t.rows.reduce((a, b) => a + b.h, 0) } })], label);
  return sec("表格",
    row(h("button", { class: "btn small", onclick: () => { const base = t.rows[r]; t.rows.splice(r + 1, 0, { h: base.h, src: base.src, cells: base.cells.map((x) => emptyCell(x.src)) }); save("插入列"); } }, "下方插入列"),
      h("button", { class: "btn small", onclick: () => { if (t.rows.length > 1) { t.rows.splice(r, 1); save("刪除列"); } } }, "刪除列")),
    row(h("button", { class: "btn small", onclick: () => { t.cols.splice(c + 1, 0, t.cols[c]); t.rows.forEach((row) => row.cells.splice(c + 1, 0, emptyCell(row.cells[c]?.src))); save("插入欄"); } }, "右方插入欄"),
      h("button", { class: "btn small", onclick: () => { if (t.cols.length > 1) { t.cols.splice(c, 1); t.rows.forEach((row) => row.cells.splice(c, 1)); save("刪除欄"); } } }, "刪除欄")),
    h("div", { class: "info" }, ec ? `儲存格 (${r + 1}, ${c + 1})` : ""),
    ec ? swatches(o.rows[r].cells[c].fill ?? null, (hex) => commit([setObj(o.id, `/rows/${r}/cells/${c}/fill`, hex)], "儲存格底色")) : null);
}

function slidePane() {
  const sl = slide();
  if (!sl) return [];
  const layouts = S.backdrops.map((b) => b.layout);
  const warn = SC.renderWarnings(sl);
  let untagged = 0, runs = 0;
  const visit = (o) => { const tbs = []; if (o.text) tbs.push(o.text); if (o.type === "table") o.rows.forEach((r) => r.cells.forEach((c) => tbs.push(c.text))); if (o.type === "group") o.children.forEach(visit); tbs.forEach((tb) => tb.paras.forEach((p) => p.runs.forEach((r) => { if (r.kind === "text" && r.text.trim()) { runs++; if (!r.tag) untagged++; } }))); };
  sl.objects.forEach(visit);
  return [
    sec("",
      row(h("label", {}, "版面配置", h("select", { onchange: (e) => commit([{ op: "set_slide", slide: sl.uid, path: "/layout", value: e.target.value }], "版面配置") }, layouts.map((l) => h("option", { selected: l === sl.layout }, l))))),
      row(h("label", { class: "row" }, h("input", { type: "checkbox", checked: !!sl.hidden, onchange: hideSlide }), "隱藏（備用頁）")),
      h("div", { class: "info mono" }, `${sl.objects.length} 物件 · ${runs} 段文字 · 未標籤 ${untagged}`)),
    warn.length ? sec("無法完整顯示（匯出原樣保留）", h("ul", { class: "warn-box" }, warn.map((w) => h("li", {}, warningText(w))))) : null,
  ];
}

/** `12 fill:group` → a sentence. */
function warningText(w) {
  const m = /^(\d+) ([a-z-]+):(.*)$/.exec(w);
  if (!m) return w;
  const [, id, kind, d] = m;
  const o = findObj(+id);
  const who = o?.name ? `「${o.name}」` : `物件 #${id}`;
  const fills = { group: "沿用群組的填色", pattern: "圖樣填滿", blip: "圖片填滿", picture: "圖片填滿" };
  switch (kind) {
    case "locked": return `${who}是 ${d} 物件，只能移動，內容無法在這裡編輯。`;
    case "geom": return `${who}的形狀（${d}）以矩形近似顯示。`;
    case "conn": return `${who}的連接線樣式（${d}）以直線近似顯示。`;
    case "pic-effects": return `${who}的圖片效果（${d.split("+").join("、")}）沒有顯示。`;
    case "fill": return `${who}的填色（${fills[d] ?? d}）以淡灰色代替。`;
    default: return `${who}：${kind} ${d}`;
  }
}

function animPane() {
  const sl = slide();
  if (!sl) return [];
  const anims = sl.anims ?? [];
  const names = { appear: "出現", fade: "淡入", fly_in: "飛入", wipe: "擦去", zoom: "縮放" };
  const trig = { click: "按一下", with: "與前一動畫同時", after: "接續前一動畫" };
  const save = (list, label) => commit([{ op: "set_slide", slide: sl.uid, path: "/anims", value: list }], label);
  const one = selObjs()[0];
  let eff = "fade";
  const list = h("ol", { class: "anim-list" }, anims.map((a, i) => {
    const o = findObj(a.obj);
    return h("li", { class: S.sel.includes(a.obj) ? "cur" : "" },
      h("span", { class: "n" }, i + 1),
      h("div", {}, h("div", {}, `${names[a.effect] ?? a.effect} · ${o?.name ?? `#${a.obj}`}`),
        row(h("select", { onchange: (e) => { const l = clone(anims); l[i].trigger = e.target.value; save(l, "動畫觸發"); } }, Object.entries(trig).map(([k, v]) => h("option", { value: k, selected: k === a.trigger }, v))),
          h("select", { onchange: (e) => { const l = clone(anims); l[i].effect = e.target.value; save(l, "動畫效果"); } }, Object.entries(names).map(([k, v]) => h("option", { value: k, selected: k === a.effect }, v))),
          h("input", { type: "number", step: 100, min: 0, value: a.duration_ms, title: "毫秒", style: "width:64px", onchange: (e) => { const l = clone(anims); l[i].duration_ms = +e.target.value; save(l, "動畫時間"); } }))),
      h("div", { class: "row" },
        ib("chevron", "上移", () => { if (!i) return; const l = clone(anims); [l[i - 1], l[i]] = [l[i], l[i - 1]]; save(l, "動畫順序"); }),
        ib("x", "移除", () => { const l = clone(anims); l.splice(i, 1); save(l, "移除動畫"); })));
  }));
  return [sec("",
    one ? row(h("select", { onchange: (e) => (eff = e.target.value) }, Object.entries(names).map(([k, v]) => h("option", { value: k, selected: k === "fade" }, v))),
      h("button", { class: "btn small primary", onclick: () => save([...anims, ...S.sel.map((id) => ({ obj: id, effect: eff, trigger: "click", duration_ms: 500, delay_ms: 0 }))], "加入動畫") }, h("span", { "data-icon": "plus" }), "加入")) : null,
    anims.length ? list : null)];
}

// ── 00 版型: 輔助線, defaults (封面 / 內頁 / 封底), the recipe gallery ──────────
const KIND_NAMES = { cover: "封面", content: "內頁", end: "封底" };
const FILTERS = { academic: "學術", design: "設計", all: "全部" };

/** A recipe thumbnail (lazy: drawn when it scrolls into view). */
const THUMBS = new Map(); // `${deck}|${recipe}|${layout}` → rendered slide node
const thumbSeen = new IntersectionObserver((es) => es.forEach((e) => {
  if (!e.isIntersecting) return;
  thumbSeen.unobserve(e.target);
  const r = recipeById(e.target.dataset.recipe);
  if (!r || e.target.firstChild) return;
  const key = `${S.name}|${r.id}|${e.target.dataset.layout}|${GUIDES.on}`;
  let node = THUMBS.get(key);
  if (!node) { node = recipeThumb(r, e.target.dataset.layout); THUMBS.set(key, node); }
  else if (node.isConnected) node = node.cloneNode(true);
  node.style.transform = `scale(${e.target.clientWidth / SC.px(S.deck.size.w)})`;
  e.target.append(node);
}));
function thumbFrame(r, layout) {
  const f = h("div", { class: "rthumb", "data-recipe": r.id, "data-layout": layout ?? "" });
  let cached = THUMBS.get(`${S.name}|${r.id}|${layout ?? ""}|${GUIDES.on}`);
  if (cached?.isConnected) cached = cached.cloneNode(true);
  if (cached) { f.append(cached); requestAnimationFrame(() => (cached.style.transform = `scale(${f.clientWidth / SC.px(S.deck.size.w)})`)); }
  else thumbSeen.observe(f);
  return f;
}

function arrangePane() {
  const sl = slide();
  if (!sl) return [];
  if (!S.arr) { loadArrangements().then(() => { renderSide(); renderGuides(); }).catch(() => {}); return [h("div", { class: "empty" }, "…")]; }

  const guides = h("label", { class: "row check" }, h("input", { type: "checkbox", checked: GUIDES.on, onchange: (e) => toggleGuides(e.target.checked) }), "輔助線");

  const chips = h("div", { class: "seg filters" }, Object.entries(FILTERS).map(([k, v]) =>
    h("button", { "aria-pressed": String(ARR.filter === k), onclick: () => { ARR.filter = k; renderSide(); } }, v)));

  const list = S.arr.recipes.filter((r) => ARR.filter === "all" || (ARR.filter === "academic") === isAcademic(r));
  const current = sl.arrangement;
  const gallery = h("div", { class: "recipes" }, list.map((r) =>
    h("button", { class: "recipe", "aria-pressed": String(current === r.id), title: r.description || r.name,
      onclick: () => applyArrangement(r.id) },
      thumbFrame(r, sl.layout),
      h("span", {}, r.name.replace(/^[^｜]*｜/, "")))));

  return [guides, chips, gallery];
}

// ── Side panel: numbered blocks (PaperComposer); explanations only behind "?" ──
const BLOCKS = [
  { id: "reveal", title: "漸進揭露", when: () => !!S.reveal, kids: () => revealPane(),
    help: "這一頁複製成數頁，每頁強調一組物件，其餘依底版的規則變淡（圖片透明、文字灰、線條淡）。在投影片上選物件（Shift 多選）後加入步驟，至少兩步。議程加粗：清單的每個第一層項目產生一頁，當前項目加粗。" },
  { id: "arr", n: "00", title: "版型", kids: () => arrangePane(),
    key: () => [S.cur, slide()?.arrangement, slide()?.layout, ARR.filter, !!S.arr, GUIDES.on].join("|"),
    help: "點縮圖就套用到這一頁；按 Enter 或＋新增的頁沿用目前這頁的版型。輔助線是這個版型的構圖線，只在編輯時顯示。圖片位置點兩下或把圖拖進去就能放圖；沒填的位置放映與匯出時不會出現。" },
  { id: "fmt", n: "01", title: "格式", kids: () => formatKids(),
    help: "文字只能套用標籤，顏色只從色票挑；字級只能在標籤允許的範圍內調整。連接線：把端點拖到圖形的連接點就會黏著；肘形線中間的黃點可以調整中段位置。" },
  { id: "page", n: "02", title: "頁面", kids: () => slidePane(),
    help: "版面配置來自底版。隱藏的頁是備用頁，匯出時保留、放映時略過。" },
  { id: "anim", n: "03", title: "動畫", kids: () => animPane(),
    help: "選物件後加入進場動畫，依清單順序播放。通常用漸進揭露代替動畫。" },
];
const blockOpen = (() => { try { return JSON.parse(localStorage.getItem("sc.blocks") ?? "{}"); } catch { return {}; } })();
function isOpen(b) { return b.id === "reveal" || (blockOpen[b.id] ?? b.id !== "anim"); }

function renderSide() {
  const box = $("#blocks");
  if (!S.deck) { box.textContent = ""; box.dataset.ids = ""; return; }
  if (S.reveal && S.reveal.uid !== S.cur) S.reveal = null;
  const want = BLOCKS.filter((b) => !b.when || b.when());
  const ids = want.map((b) => b.id).join(",");
  if (box.dataset.ids !== ids) {
    box.dataset.ids = ids;
    box.textContent = "";
    for (const b of want) box.append(blockShell(b));
    paintIcons(box);
  }
  for (const b of want) {
    const el = box.querySelector(`.block[data-id="${b.id}"]`);
    el.dataset.open = isOpen(b) ? "true" : "false";
    if (!isOpen(b)) continue;
    const pane = el.querySelector(".pane");
    // a block with its own key (00 版型) is left alone while that key holds
    if (b.key) { const k = b.key(); if (pane.dataset.key === k) continue; fillPane(pane, b.kids(), k); continue; }
    fillPane(pane, b.kids(), [b.id, S.cur, S.sel.join(","), S.editing, S.crop, S.reveal ? "r" : ""].join("|"));
  }
}
function blockShell(b) {
  const head = h("div", { class: "block-head" }, b.n ? h("span", { class: "n" }, b.n) : null, h("h3", {}, b.title), helpButton(b.help));
  if (b.id === "reveal") {
    const close = ib("x", "關閉", () => { S.reveal = null; renderSide(); });
    close.classList.add("small");
    close.style.marginLeft = "auto";
    head.append(close);
  } else {
    const toggle = h("button", { class: "toggle", "aria-label": "收合" }, isOpen(b) ? "−" : "+");
    toggle.onclick = () => {
      blockOpen[b.id] = !isOpen(b);
      try { localStorage.setItem("sc.blocks", JSON.stringify(blockOpen)); } catch {}
      toggle.textContent = blockOpen[b.id] ? "−" : "+";
      renderSide();
    };
    head.append(toggle);
  }
  return h("section", { class: "block", "data-id": b.id }, head, h("div", { class: "block-body" }, h("div", { class: "pane" })));
}
/** The "?" next to a block title: the only place with explanatory text. */
function helpButton(text) {
  if (!text) return null;
  const b = h("button", { class: "help", "aria-label": "說明", "aria-expanded": "false" }, "?");
  b.onclick = (e) => {
    e.stopPropagation();
    const was = $(".tip");
    was?.remove();
    $$(".help[aria-expanded=true]").forEach((x) => x.setAttribute("aria-expanded", "false"));
    if (was?.dataset.for === text) return;
    const t = h("div", { class: "tip", "data-for": text }, text);
    document.body.append(t);
    const r = b.getBoundingClientRect();
    t.style.top = `${r.bottom + 6}px`;
    t.style.left = `${Math.max(8, Math.min(r.left - 8, innerWidth - t.offsetWidth - 8))}px`;
    b.setAttribute("aria-expanded", "true");
    setTimeout(() => document.addEventListener("pointerdown", () => { t.remove(); b.setAttribute("aria-expanded", "false"); }, { once: true }), 0);
  };
  return b;
}
function fillPane(pane, kids, key) {
  const ae = document.activeElement;
  // a focused number field (X/Y/W/H…) is kept: refresh values in place instead of rebuilding under it
  if (pane.dataset.key === key && ae?.dataset?.k && pane.contains(ae)) {
    const fresh = h("div", {}, ...kids.flat().filter(Boolean));
    const now = $$("[data-k]", pane), next = $$("[data-k]", fresh);
    if (now.length === next.length && now.every((e, i) => e.dataset.k === next[i].dataset.k)) {
      now.forEach((e, i) => { if (e !== ae || +e.value !== +next[i].value) e.value = next[i].value; });
      return;
    }
  }
  pane.dataset.key = key;
  pane.textContent = "";
  for (const k of kids.flat()) if (k) pane.append(k);
  if (!pane.childElementCount) pane.append(h("div", { class: "empty" }, "—"));
  paintIcons(pane);
}
function formatKids() {
  const kids = [];
  const objs = selObjs();
  const one = objs.length === 1 ? objs[0] : null;
  const editingObj = S.editing ? findObj(S.editing) : null;
  if (editingObj || textTargets().length) kids.push(textSection());
  if (one) {
    if (one.type === "shape" || one.type === "text") kids.push(fillLineSection(one));
    if (one.type === "connector") kids.push(connectorSection(one));
    if (one.type === "picture") kids.push(pictureSection(one));
    if (one.type === "table") kids.push(tableSection(one));
    if (one.type === "locked") kids.push(sec("鎖定物件", h("div", { class: "info" }, one.kind)));
    if (one.equation) kids.push(sec("方程式", row(h("button", { class: "btn small", onclick: () => openEquation(one) }, h("span", { "data-icon": "sigma" }), "LaTeX"))));
  }
  if (objs.length) kids.push(arrangeSection(objs));
  return kids;
}

/** KaTeX parse errors → short Chinese hints. */
function katexZh(m) {
  m = String(m).replace(/^KaTeX parse error: /, "");
  let x;
  if (/Unexpected end of input|Expected '\}'|expected '\}'/.test(m)) return "括號沒有成對（少了 }）";
  if ((x = /Undefined control sequence: (\\\S+)/.exec(m))) return `不認得的指令 ${x[1]}`;
  if (/Extra \}/.test(m)) return "多了一個 }";
  if (/Double (sub|super)script/.test(m)) return "上標或下標重複，請用大括號分組";
  return m;
}
/** Where a new object of size w×h goes: the wanted spot (x, y) when free, else the first free place
 *  in the content area (reading order), else one cascade step per object already there. */
function freeSpot(x, y, w = 0, h = 0) {
  const d = Math.round(IN / 8), sz = S.deck.size, m = Math.round(0.4 * IN);
  const objs = (slide()?.objects ?? []).filter((o) => o.type !== "connector" && !o.hidden && !isEmptySlot(o) && slotKind(o) !== "deco" && o.frame.w * o.frame.h < 0.6 * sz.w * sz.h);
  const hit = (px, py) => objs.some((o) => px < o.frame.x + o.frame.w - 1 && px + w > o.frame.x + 1 && py < o.frame.y + o.frame.h - 1 && py + h > o.frame.y + 1);
  if (!hit(x, y)) return [x, y];
  const step = Math.round(IN / 4);
  for (let py = m; py + h <= sz.h - m; py += step) {
    if (!hit(x, py)) return [x, py];
    for (let px = m; px + w <= sz.w - m; px += step) if (!hit(px, py)) return [px, py];
  }
  for (let k = 1; k < 40; k++) if (!objs.some((o) => Math.abs(o.frame.x - (x + k * d)) < d / 2 && Math.abs(o.frame.y - (y + k * d)) < d / 2)) return [x + k * d, y + k * d];
  return [x, y];
}
/** What a person calls an object: its text, else its kind. */
function objLabel(o) {
  const t = (o.text?.paras ?? []).flatMap((p) => p.runs).map((r) => r.text ?? "").join(" ").replace(/\s+/g, " ").trim();
  if (o.equation) return "方程式";
  if (t) return t.length > 14 ? `${t.slice(0, 14)}…` : t;
  return { table: "表格", picture: "圖片", connector: "連接線", group: "群組" }[o.type] ?? o.name ?? `#${o.id}`;
}
/** Rendered size (EMU) of a no-wrap text body, as the canvas draws it. */
async function measureMath(tb) {
  measureText(tb);                       // makes the browser fetch the KaTeX fonts
  try { await document.fonts.ready; } catch {}
  const [w, hh] = measureText(tb);
  return [w + Math.round(0.08 * IN), hh]; // a little air: superscripts and italics overhang the measured box
}
function measureText(tb) {
  const host = h("div", { class: "slide", style: "position:fixed;left:-10000px;top:0;width:4000px;height:2000px;visibility:hidden" });
  const t = SC.renderText(tb, { latex: (m) => S.latex.get(m) });
  host.append(t);
  document.body.append(host);
  const r = $(".tb", t).getBoundingClientRect();
  host.remove();
  const [l, tp, rr, b] = tb.insets;
  return [Math.max(Math.round(0.3 * IN), Math.ceil(r.width * EMU) + l + rr + 2 * EMU), Math.max(Math.round(0.2 * IN), Math.ceil(r.height * EMU) + tp + b)];
}

// ── Dialogs ───────────────────────────────────────────────────────────────
function dialog(build) {
  const d = $("#dlg");
  if (d.open) d.close();
  d.textContent = "";
  d.removeAttribute("style");
  const box = h("div", { class: "dlg" });
  build(box, () => d.close());
  d.append(box);
  paintIcons(d);
  d.showModal();
  return d;
}

function openTableDialog() {
  dialog((box, close) => {
    const r = h("input", { type: "number", min: 1, max: 30, value: 4 }), c = h("input", { type: "number", min: 1, max: 12, value: 3 });
    box.append(h("h3", {}, "插入表格"), row(h("label", {}, "列", r), h("label", {}, "欄", c)), h("div", { class: "info" }, "格式：無樣式、表格格線；文字標籤 table。"),
      h("div", { class: "actions" }, h("button", { class: "btn", onclick: close }, "取消"), h("button", { class: "btn primary", onclick: async () => {
        const nr = +r.value, nc = +c.value;
        const W = Math.round(S.deck.size.w * 0.84), rh = 0.4 * IN;
        const cell = () => ({ text: { insets: [91440, 45720, 91440, 45720], anchor: "t", wrap: true, autofit: "none", paras: [newPara([], "table")] }, borders: [null, null, null, null] });
        const [tx, ty] = freeSpot(Math.round((S.deck.size.w - W) / 2), Math.round(S.deck.size.h / 3), W, Math.round(rh * nr));
        const o = { ...baseObj("table", { x: tx, y: ty, w: Math.round(W), h: Math.round(rh * nr) }, "表格"),
          cols: Array(nc).fill(Math.round(W / nc)), rows: [...Array(nr)].map(() => ({ h: Math.round(rh), cells: [...Array(nc)].map(cell) })), style_id: "{5940675A-B579-460E-94D1-54222C63F5DA}" };
        close();
        await insertObj(o, "插入表格");
      } }, "插入")));
  });
}

function openEquation(o) {
  const run = o?.text?.paras.flatMap((p) => p.runs).find((r) => r.kind === "math");
  const cur = run ? run.latex ?? S.latex.get(run.omml) ?? "" : "";
  dialog((box, close) => {
    const ta = h("textarea", { placeholder: "例如 \\mathcal{L} = \\lambda_1 L_{adv} + \\lambda_2 L_{rec}" }, cur);
    const pv = h("div", { class: "preview" });
    const msg = h("div", { class: "info" }, "存檔時經 pandoc（lore-math）轉成原生 OMML，PowerPoint 可直接編輯。");
    let ok = false;
    const go = h("button", { class: "btn primary", onclick: async () => {
        const latex = ta.value.trim();
        if (!latex || !ok) return;
        const size = S.style?.tags?.equation?.size ?? 14;
        let omml;
        try { omml = (await api("/api/math", { body: { latex, size } })).omml; } catch (e) { toast(e.message, true); return; }
        S.latex.set(omml, latex);
        const look = tagLook("equation") ?? { font: "Cambria Math", size, color: inkColor() };
        const mrun = { kind: "math", text: latex, tag: S.style?.tags?.equation ? "equation" : null, look, omml, latex };
        close();
        if (o) {
          const t = clone(o.text);
          let done = false;
          t.paras.forEach((p) => p.runs.forEach((r, i) => { if (!done && r.kind === "math") { p.runs[i] = { ...r, omml, latex, text: latex }; done = true; } }));
          // only a box that is just the equation follows its size
          const solo = o.text.paras.length === 1 && o.text.paras[0].runs.every((r) => r.kind === "math" || !(r.text ?? "").trim());
          const ops = [setObj(o.id, "/text", t)];
          if (solo && !o.frame.rot) { const [w, hh] = await measureMath(t); ops.push(setObj(o.id, "/frame", { ...o.frame, w, h: hh })); }
          await commit(ops, "編輯方程式");
        } else {
          const tb = { insets: [0, 0, 0, 0], anchor: "t", wrap: false, autofit: "shape", paras: [{ ...newPara([mrun], "equation"), end: look }] };
          const [w, hh] = await measureMath(tb);
          const [x, y] = freeSpot(Math.round((S.deck.size.w - w) / 2), Math.round((S.deck.size.h - hh) / 2), w, hh);
          await insertObj({ ...baseObj("text", { x, y, w, h: hh }, "方程式"), geom: { prst: "rect" }, fill: { kind: "none" }, text: tb, equation: true }, "插入方程式");
        }
      } }, o ? "更新" : "插入");
    const upd = () => {
      const v = ta.value.trim();
      try { katex.render(v || "\\;", pv, { throwOnError: false, displayMode: true }); } catch {}
      let err = null;
      try { if (v) katex.renderToString(v, { throwOnError: true }); } catch (e) { err = e; }
      ok = !!v && !err;
      go.disabled = !ok;
      msg.style.color = err ? "var(--err)" : "";
      msg.textContent = err ? `LaTeX 有錯誤，修正後才能${o ? "更新" : "插入"}：${katexZh(err.message)}` : v ? "存檔時經 pandoc（lore-math）轉成原生 OMML，PowerPoint 可直接編輯。" : "輸入 LaTeX，下方即時預覽。";
    };
    ta.oninput = upd;
    upd();
    box.append(h("h3", {}, o ? "編輯方程式" : "插入方程式"), ta, pv, msg, h("div", { class: "actions" }, h("button", { class: "btn", onclick: close }, "取消"), go));
    setTimeout(() => ta.focus(), 30);
  });
}

function openComponentMenu(anchor) {
  $(".menu")?.remove();
  const comps = Object.entries(S.style?.components ?? {}).filter(([k]) => k !== "gradient-bar");
  const m = h("div", { class: "menu" });
  const r = anchor.getBoundingClientRect();
  m.style.left = `${r.left}px`;
  m.style.top = `${r.bottom + 4}px`;
  let n = 1 + slide().objects.filter((o) => o.geom?.prst === "ellipse" && o.frame.w < 400000).length;
  for (const [k] of comps) {
    m.append(h("button", { onclick: async () => {
      m.remove();
      try {
        // each new component lands next to the previous one instead of on top of it
        const [x, y] = freeSpot(Math.round(S.deck.size.w / 2 - IN / 2), Math.round(S.deck.size.h / 2 - IN / 3), IN, Math.round(0.67 * IN));
        const o = await api(deckApi("/component"), { body: { name: k, slide: S.cur, x, y, n: k === "numbered-circle" ? n : 1 } });
        await insertObj(o, `插入${zh("components", k)}`);
      } catch (e) { toast(e.message, true); }
    } }, zh("components", k), h("span", { class: "muted mono", style: "margin-left:auto;font-size:11px" }, k)));
  }
  for (const [k] of Object.entries(S.style?.arrows ?? {})) {
    m.append(h("button", { onclick: async () => {
      m.remove();
      const a = S.style.arrows[k];
      const [x, y] = freeSpot(Math.round(S.deck.size.w / 2 - IN), Math.round(S.deck.size.h / 2), 2 * IN, Math.round(0.2 * IN));
      await insertObj({ ...baseObj("connector", connFrame([x, y], [x + 2 * IN, y]), zh("arrows", k)), geom: a.connector, line: { color: a.color.toUpperCase(), width_pt: a.width_pt, dash: a.dash, head: a.head || "none", tail: a.tail } }, "插入箭頭");
    } }, zh("arrows", k), h("span", { class: "muted mono", style: "margin-left:auto;font-size:11px" }, k)));
  }
  if (!comps.length) m.append(h("div", { class: "info", style: "padding:8px" }, "這份簡報的風格包沒有元件。"));
  document.body.append(m);
  setTimeout(() => document.addEventListener("pointerdown", (e) => { if (!m.contains(e.target)) m.remove(); }, { once: true }), 0);
}

/** Progressive reveal, docked in the side panel (the slide stays clickable). */
function openReveal() {
  const sl = slide();
  if (!sl) return;
  S.reveal = { uid: sl.uid, steps: [], mode: "ghost", future: "ghost", agenda: null };
  togglePanel("side", true);
  renderSide();
}
function agendaCount(o) {
  return o.text.paras.filter((p) => !(p.level ?? 0) && p.runs.some((r) => r.kind === "math" || (r.text ?? "").trim())).length;
}
function revealPane() {
  const R = S.reveal;
  const sl = slide();
  const close = () => { S.reveal = null; renderSide(); };
  const list = h("ol", { class: "steps" });
  R.steps.forEach((st, i) => list.append(h("li", {}, h("b", {}, `步驟 ${i + 1}`), h("span", { class: "ids" }, st.map((id) => (findObj(id) ? objLabel(findObj(id)) : `#${id}`)).join("、")), ib("x", "移除這個步驟", () => { R.steps.splice(i, 1); renderSide(); }))));
  const lists = sl.objects.filter((o) => o.text && agendaCount(o) > 1);
  if (R.agenda == null || !lists.some((o) => o.id === R.agenda)) R.agenda = lists.find((o) => S.sel.includes(o.id))?.id ?? lists[0]?.id ?? null;
  const ghostBox = h("div", { class: "sec" },
    row(h("button", { class: "btn small primary", disabled: !S.sel.length, onclick: () => { R.steps.push([...S.sel]); S.sel = []; renderOverlay(); renderSide(); } }, h("span", { "data-icon": "plus" }), `步驟 ${R.steps.length + 1}${S.sel.length ? `（${S.sel.length}）` : ""}`)),
    list,
    row(h("label", {}, "之後的物件", h("select", { onchange: (e) => (R.future = e.target.value) }, h("option", { value: "ghost", selected: R.future === "ghost" }, "變淡"), h("option", { value: "hide", selected: R.future === "hide" }, "先隱藏")))));
  const agendaBox = h("div", { class: "sec" },
    lists.length ? row(h("label", {}, "清單", h("select", { onchange: (e) => (R.agenda = +e.target.value) }, lists.map((o) => h("option", { value: o.id, selected: o.id === R.agenda }, `${objLabel(o)} · ${agendaCount(o)} 項`))))) : h("div", { class: "empty" }, "—"));
  const pick = (m) => { R.mode = m; renderSide(); };
  const seg = h("div", { class: "seg" },
    h("button", { "aria-pressed": R.mode === "ghost" ? "true" : "false", onclick: () => pick("ghost") }, "複製頁＋變淡"),
    h("button", { "aria-pressed": R.mode === "agenda" ? "true" : "false", onclick: () => pick("agenda") }, "議程加粗"));
  const g = S.style?.ghost;
  const ready = R.mode === "ghost" ? R.steps.length >= 2 : R.agenda != null;
  return [h("div", { class: "pane-inner sec" },
    seg,
    R.mode === "ghost" ? ghostBox : agendaBox,
    R.mode === "ghost" && g ? h("div", { class: "info mono" }, `變淡：圖片 ${Math.round(g.picture_alpha * 100)}%、線條 ${Math.round(g.line_alpha * 100)}%、填色 ${Math.round((g.fill_alpha || g.line_alpha) * 100)}%、文字 #${g.text_color}`) : null,
    h("div", { class: "actions row" }, h("button", { class: "btn small", onclick: close }, "取消"), h("button", { class: "btn small primary", disabled: !ready, title: ready ? "" : "至少兩個步驟", onclick: async () => {
      // a connector appears with the step of the shape it points to (else the one it starts from)
      const stepOf = (id) => R.steps.findIndex((st) => st.includes(id));
      const steps = R.steps.map((st) => [...st]);
      for (const c of sl.objects.filter((o) => o.type === "connector" && stepOf(o.id) < 0)) {
        const i = [c.end?.id, c.start?.id].map((id) => (id == null ? -1 : stepOf(id))).find((k) => k >= 0);
        if (i != null && i >= 0) steps[i].push(c.id);
      }
      const spec = R.mode === "ghost" ? { mode: "ghost", steps, future: R.future } : { mode: "agenda", obj: R.agenda, em: "strong" };
      await serial(async () => {
        try { absorb(await api(deckApi("/reveal"), { body: { slide: R.uid, spec } })); S.reveal = null; renderSide(); toast("已產生漸進揭露頁"); } catch (e) { toast(e.message, true); }
      });
    } }, R.mode === "ghost" ? `產生 ${R.steps.length} 頁` : "產生")))];
}
