// ⌘F: find (and replace) across the text of the whole deck — text boxes, table cells, groups.
// The find bar sits under the tool capsule (shell search hook). Matches are found inside a
// paragraph's text (runs joined; an equation counts as one placeholder character, so a match
// never crosses it); every match on the open slide is highlighted, the current one stronger.
// Replace changes a match that lies inside one run (formatting stays); a match that crosses
// runs is left alone and counted. One 取代全部 is one undo step. Loaded after app.js.
"use strict";

const FIND = { open: false, q: "", hits: [], i: -1, key: null, replace: false };
const FIND_MATH = "\u0000";

/** Every paragraph of the deck that can hold text: { uid, id, cell?: [r, c], pi, text, runs }. */
function findParas() {
  const out = [];
  const tbs = (o) => [...(o.text ? [{ tb: o.text }] : []), ...(o.rows ?? []).flatMap((rw, r) => rw.cells.map((c, ci) => ({ tb: c.text, cell: [r, ci] })))];
  const walk = (uid, objs) => objs.forEach((o) => {
    if (o.type === "group") walk(uid, o.children);
    for (const { tb, cell } of tbs(o)) tb?.paras?.forEach((p, pi) => out.push({ uid, id: o.id, cell, pi, runs: p.runs, text: p.runs.map((r) => (r.kind === "math" || r.kind === "equation" ? FIND_MATH : r.text ?? "")).join("") }));
  });
  S.deck?.slides.forEach((s) => walk(s.uid, s.objects));
  return out;
}

function findAll(q) {
  const hits = [];
  if (!q) return hits;
  const needle = q.toLowerCase();
  for (const p of findParas()) {
    const hay = p.text.toLowerCase();
    for (let at = hay.indexOf(needle); at >= 0; at = hay.indexOf(needle, at + needle.length)) hits.push({ ...p, start: at, len: q.length });
  }
  return hits;
}
const hitKey = (h) => h && `${h.uid}/${h.id}/${h.cell ?? ""}/${h.pi}/${h.start}`;

function findRefresh(keepCurrent = true) {
  const was = keepCurrent ? FIND.key : null;
  FIND.hits = findAll(FIND.q);
  let i = was ? FIND.hits.findIndex((h) => hitKey(h) === was) : -1;
  if (i < 0) i = FIND.hits.length ? Math.min(Math.max(FIND.i, 0), FIND.hits.length - 1) : -1;
  FIND.i = i;
  FIND.key = hitKey(FIND.hits[i]);
  findCount();
}
function findCount() {
  const n = FIND.hits.length;
  $("#findCount").textContent = !FIND.q ? "" : n ? `${FIND.i + 1} / ${n}` : "無";
  $("#findInput").classList.toggle("none", !!FIND.q && !n);
}

/** Show hit i: its slide, the object selected, the matches painted. */
function findGo(i) {
  const n = FIND.hits.length;
  if (!n) { findCount(); paintFind(); return; }
  FIND.i = ((i % n) + n) % n;
  const h = FIND.hits[FIND.i];
  FIND.key = hitKey(h);
  if (S.cur !== h.uid) gotoSlide(h.uid);
  else if (S.editing || S.editingCell) exitEdit();
  S.sel = [h.id];
  renderOverlay();
  renderSide();
  findCount();
  paintFind();
  document.querySelector(".paper .find-cur")?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
}

/** The text of a rendered paragraph, in the same character positions as findParas (bullets skipped, equations one char). */
function domParaText(p) {
  const parts = [];
  const walk = (n) => {
    if (n.nodeType === 3) parts.push({ node: n, text: n.nodeValue });
    else if (n.nodeType === 1) {
      if (n.classList.contains("bu")) return;
      if (n.classList.contains("math")) { parts.push({ node: null, text: FIND_MATH }); return; }
      n.childNodes.forEach(walk);
    }
  };
  walk(p);
  return parts;
}
function rangeFor(parts, start, len) {
  let pos = 0, r = null;
  const end = start + len;
  for (const { node, text } of parts) {
    const a = pos, b = pos + text.length;
    pos = b;
    if (!node || b <= start || a >= end) continue;
    if (!r) { r = new Range(); r.setStart(node, Math.max(0, start - a)); }
    r.setEnd(node, Math.min(text.length, end - a));
  }
  return r;
}
function hitRange(h) {
  const el = objEl(h.id);
  if (!el) return null;
  const host = h.cell ? el.querySelectorAll("tr")[h.cell[0]]?.children[h.cell[1]] : el;
  const para = host?.querySelectorAll(".tx > .p, .tx .p")[h.pi];
  if (!para) return null;
  const ps = [...host.querySelectorAll(".tx .p")].filter((p) => !p.parentElement.closest(".p"));
  return rangeFor(domParaText(ps[h.pi] ?? para), h.start, h.len);
}

/** Highlight every match of the open slide (CSS Custom Highlight API: no DOM change, nothing to undo). */
function paintFind() {
  if (!window.CSS?.highlights) return;
  CSS.highlights.delete("find-all");
  CSS.highlights.delete("find-cur");
  if (!FIND.open || !FIND.q || !S.deck || S.editing || S.editingCell) return;
  if (FIND.hits.length === 0 || FIND.key !== hitKey(FIND.hits[FIND.i])) findRefresh();
  const all = new Highlight(), cur = new Highlight();
  FIND.hits.forEach((h, i) => {
    if (h.uid !== S.cur) return;
    const r = hitRange(h);
    if (r) (i === FIND.i ? cur : all).add(r);
  });
  CSS.highlights.set("find-all", all);
  CSS.highlights.set("find-cur", cur);
}

function openFind() {
  if (!S.deck) return;
  FIND.open = true;
  $("#findBar").hidden = false;
  const input = $("#findInput");
  const sel = String(getSelection?.() ?? "").trim();
  if (sel && !sel.includes("\n") && sel.length < 80 && !FIND.q) { input.value = sel; FIND.q = sel; }
  findRefresh();
  input.focus();
  input.select();
  paintFind();
}
function closeFind() {
  FIND.open = false;
  $("#findBar").hidden = true;
  CSS.highlights?.delete("find-all");
  CSS.highlights?.delete("find-cur");
}

// ── Replace ────────────────────────────────────────────────────────────────
/** Text box with `hits` (all in one object/cell) replaced; { tb, done, skipped }. */
function replaceInTextBox(tb, hits, rep) {
  const out = clone(tb);
  let done = 0, skipped = 0;
  for (const h of [...hits].sort((a, b) => b.start - a.start)) {
    const para = out.paras[h.pi];
    let pos = 0, ok = false;
    for (const run of para.runs) {
      const isText = run.kind !== "math" && run.kind !== "equation";
      const w = isText ? (run.text ?? "").length : 1;
      if (isText && h.start >= pos && h.start + h.len <= pos + w) {
        const a = h.start - pos;
        run.text = run.text.slice(0, a) + rep + run.text.slice(a + h.len);
        ok = true;
        break;
      }
      pos += w;
    }
    if (ok) done++; else skipped++;
  }
  return { tb: out, done, skipped };
}
async function replaceHits(hits) {
  const rep = $("#replaceInput").value;
  const groups = new Map();
  for (const h of hits) { const k = `${h.uid}/${h.id}/${h.cell ?? ""}`; if (!groups.has(k)) groups.set(k, []); groups.get(k).push(h); }
  const ops = [];
  let done = 0, skipped = 0;
  for (const hs of groups.values()) {
    const h0 = hs[0];
    const sl = S.deck.slides.find((s) => s.uid === h0.uid);
    const o = findObj(h0.id, sl?.objects ?? []);
    const tb = h0.cell ? o?.rows?.[h0.cell[0]]?.cells?.[h0.cell[1]]?.text : o?.text;
    if (!tb) continue;
    const r = replaceInTextBox(tb, hs, rep);
    done += r.done; skipped += r.skipped;
    if (r.done) ops.push({ op: "set_obj", slide: h0.uid, id: h0.id, path: h0.cell ? `/rows/${h0.cell[0]}/cells/${h0.cell[1]}/text` : "/text", value: r.tb });
  }
  if (ops.length) await commit(ops, "取代");
  findRefresh(false);
  findGo(FIND.i);
  if (skipped) toast(`${skipped} 處跨越不同格式，沒有取代`);
  return done;
}

function wireFind() {
  const input = $("#findInput");
  let timer = 0;
  input.addEventListener("input", () => {
    clearTimeout(timer);
    timer = setTimeout(() => { FIND.q = input.value; findRefresh(false); if (FIND.hits.length) findGo(Math.max(FIND.hits.findIndex((h) => h.uid === S.cur), 0)); else paintFind(); }, 120);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); closeFind(); }
    else if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); if (FIND.q !== input.value) { clearTimeout(timer); FIND.q = input.value; findRefresh(false); findGo(0); } else findGo(FIND.i + (e.shiftKey ? -1 : 1)); }
  });
  $("#replaceInput").addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); closeFind(); }
    else if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); $("#replaceOne").click(); }
  });
  $("#findNext").onclick = () => findGo(FIND.i + 1);
  $("#findPrev").onclick = () => findGo(FIND.i - 1);
  $("#findClose").onclick = closeFind;
  $("#findReplace").onclick = (e) => {
    FIND.replace = !FIND.replace;
    e.currentTarget.setAttribute("aria-pressed", String(FIND.replace));
    for (const id of ["replaceInput", "replaceOne", "replaceAll"]) $(`#${id}`).hidden = !FIND.replace;
    if (FIND.replace) $("#replaceInput").focus();
  };
  $("#replaceOne").onclick = async () => { const h = FIND.hits[FIND.i]; if (h) await replaceHits([h]); };
  $("#replaceAll").onclick = async () => { const n = FIND.hits.length; if (n && (await replaceHits(FIND.hits)) >= 0) toast(`已取代 ${n} 處`); };
  paintIcons($("#findBar"));
}
wireFind();
