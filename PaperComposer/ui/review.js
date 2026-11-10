// Review mode (審閱): tracked changes and comments, a mode of its own beside
// writing. The top bar's 「寫作｜審閱」 switches; the choice is kept per viewer.
//
// The record lives in the paper's review.json (GET /api/papers/{name}/review),
// never in the .tex. The server records changes when a section is saved while
// 追蹤修訂 is on — every edit path goes through that save, in either mode — at
// paragraph granularity ({before, after}); this file draws them:
//
// - writing mode: today's page; a paragraph with an open comment gets a dot
//   in its left margin, and the status bar offers 「審閱 N」.
// - review mode: the writing toolbar gives way to the review toolbar, the
//   right panel to the review list, the navigator counts items per
//   paragraph, and the page shows insertions (green underline), deletions
//   (red strike), comment ranges (yellow) and a numbered marker per item.
//   顯示: 標記 (marks) / 只看結果 (current text) / 原稿 (before).
//
// Word-level diffs work on LaTeX tokens that keep $…$, \cmd[..]{..} and
// environments whole, each run drawn through PCTex.render, so math still
// renders; a paragraph that cannot be drawn that way (environments, comments,
// line breaks, headings, tables) shows the old text struck above the new.
//
// Comments anchor on source text {exact, prefix, suffix}: a selection on the
// drawn page is found in the source through the same masking find.js uses
// (PCFind.mask); when that fails the comment holds the whole paragraph.

const PCReview = (() => {
  let data = null;            // { tracking, changes: [{…, place}], comments, me }
  let paper = null;           // the paper `data` belongs to
  let mode = store.get("pc.reviewMode", "write");
  let view = "markup";        // markup | final | original
  let tab = "all";            // all | changes | comments
  let resolvedShown = false;
  let cur = null;             // current item key: "c:<id>" | "m:<id>"
  let bar, panel, observer, queued = false;
  let wordRefs = [];          // [{ key, title, checked }] from a Word reference list
  let refsBusy = false;

  const base = () => `/api/papers/${encodeURIComponent(state.paper.name)}`;
  const reviewing = () => mode === "review" && !!state.paper;
  const norm = (t) => String(t ?? "").replace(/[\s~]+/g, " ").trim();
  const VIEWS = { markup: "標記", final: "只看結果", original: "原稿" };

  // ── Data ────────────────────────────────────────────
  async function load() {
    if (!state.paper) { data = null; paper = null; after(); return; }
    const name = state.paper.name;
    try { data = await api(`${base()}/review`); paper = name; }
    catch { data = null; paper = null; }
    // references read from a Word list: checked on Crossref only when asked
    try { wordRefs = (await api(`${base()}/word-refs`)).refs ?? []; } catch { wordRefs = []; }
    if (state.paper?.name !== name) return;
    after();
  }
  /** After a section was saved (saveSection): the server may have recorded it. */
  const refresh = () => load();

  const secIndex = (file) => sections().findIndex((s) => s.file === file);

  /** Every item, in document order: changes and comments with their place. */
  function items() {
    if (!data || paper !== state.paper?.name) return [];
    const out = [];
    data.changes.forEach((c) => {
      const si = secIndex(c.file);
      const lost = c.place?.kind === "lost";
      const pi = lost ? c.index : c.place?.index ?? c.index;
      out.push({ key: `c:${c.id}`, kind: "change", c, si, pi, gap: c.place?.kind === "gap", lost, under: c.place?.kind === "under" });
    });
    data.comments.forEach((c) => {
      if (c.resolved && !resolvedShown) return;
      out.push({ key: `m:${c.id}`, kind: "comment", c, si: secIndex(c.file), pi: c.index, orphan: c.orphan });
    });
    const at = (x) => (x.si < 0 ? 1e9 : x.si) * 1e6 + x.pi * 4 + (x.gap ? 0 : x.kind === "change" ? 1 : 2);
    return out.sort((a, b) => at(a) - at(b) || String(a.c.at).localeCompare(String(b.c.at)));
  }
  /** Open items: every change, every unresolved comment. */
  function count() {
    if (!data || paper !== state.paper?.name) return 0;
    return data.changes.length + data.comments.filter((c) => !c.resolved).length;
  }
  const visibleItems = () => items().filter((x) => tab === "all" || (tab === "changes") === (x.kind === "change"));
  const itemAt = (si, pi) => items().filter((x) => x.si === si && !x.lost && !x.orphan && (x.gap ? Math.min(x.pi, sections()[si].paragraphs.length - 1) : x.pi) === pi);

  // ── Small helpers ───────────────────────────────────
  // one stable colour per author (hash of the name); none of them reads as insert, delete or comment
  const AUTHOR = ["#4f8cff", "#c084fc", "#f472b6", "#38bdf8", "#a78bfa", "#fb923c", "#94a3b8", "#e879f9"];
  function colour(name) {
    let h = 0;
    for (const ch of String(name).toLowerCase()) h = (h * 31 + ch.codePointAt(0)) >>> 0;
    return AUTHOR[h % AUTHOR.length];
  }
  function when(c) {
    if (c.source === "word") return "Word 匯入";
    const d = new Date(c.at);
    if (isNaN(d)) return "";
    const now = new Date(), day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
    const diff = Math.round((day(now) - day(d)) / 864e5);
    if (diff === 0) return d.toLocaleTimeString("zh-TW", { hour: "2-digit", minute: "2-digit", hour12: false });
    if (diff === 1) return "昨天";
    return d.getFullYear() === now.getFullYear() ? `${d.getMonth() + 1}/${d.getDate()}` : `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`;
  }
  function where(x) {
    const sec = sections()[x.si];
    if (!sec) return "";
    const word = sec.title.replace(/\\[a-zA-Z]+\*?|[{}$]/g, "").trim().split(/\s+/)[0] ?? "";
    return `${word.length > 10 ? word.slice(0, 9) + "…" : word} ${pad(x.pi + 1)}`;
  }

  // ── LaTeX tokens and word diff ──────────────────────
  /** End of the {…} group at i (exclusive). */
  function group(s, i) {
    let d = 0;
    for (let k = i; k < s.length; k++) {
      if (s[k] === "\\") { k++; continue; }
      if (s[k] === "{") d++;
      else if (s[k] === "}" && --d === 0) return k + 1;
    }
    return s.length;
  }
  function until(s, i, close) {
    for (let k = i; k < s.length; k++) {
      if (s.startsWith(close, k)) return k + close.length;
      if (s[k] === "\\") k++;
    }
    return s.length;
  }
  const CJK = /[　-鿿가-힯豈-﫿＀-￯]/;
  /** Words, spaces and punctuation; math, commands with their arguments and environments whole. */
  function tokens(s) {
    const out = [];
    let i = 0;
    while (i < s.length) {
      const c = s[i];
      let j = i + 1;
      if (/\s/.test(c)) { while (j < s.length && /\s/.test(s[j])) j++; }
      else if (c === "%") { j = s.indexOf("\n", i); if (j < 0) j = s.length; }
      else if (c === "$") j = s[i + 1] === "$" ? until(s, i + 2, "$$") : until(s, i + 1, "$");
      else if (c === "{") j = group(s, i);
      else if (c === "\\") {
        const d = s[i + 1] ?? "";
        if (d === "(") j = until(s, i + 2, "\\)");
        else if (d === "[") j = until(s, i + 2, "\\]");
        else if (/[a-zA-Z@]/.test(d)) {
          j = i + 1;
          while (j < s.length && /[a-zA-Z@]/.test(s[j])) j++;
          const name = s.slice(i + 1, j);
          if (s[j] === "*") j++;
          if (name === "begin" && s[j] === "{") {
            const env = s.slice(j + 1, group(s, j) - 1);
            const end = `\\end{${env}}`;
            let depth = 0, k = j;
            for (; k < s.length; k++) {
              if (s.startsWith(`\\begin{${env}}`, k)) depth++;
              else if (s.startsWith(end, k) && --depth === 0) { k += end.length; break; }
            }
            j = Math.min(k, s.length);
          } else {
            for (;;) {
              if (s[j] === "[") { const e = until(s, j + 1, "]"); j = e; continue; }
              if (s[j] === "{") { j = group(s, j); continue; }
              break;
            }
          }
        } else j = Math.min(i + 2, s.length);
      } else if (/\d/.test(c)) j = i + s.slice(i).match(/^\d+(?:[.,]\d+)*/)[0].length;
      else if (CJK.test(c)) j = i + 1;
      else if (/[\p{L}\p{N}]/u.test(c)) { while (j < s.length && /[\p{L}\p{N}'’-]/u.test(s[j]) && !CJK.test(s[j])) j++; }
      out.push(s.slice(i, j));
      i = j;
    }
    return out;
  }

  /** [{ t: "eq" | "del" | "ins", text }] from before to after, or null when too long. */
  function diff(a, b) {
    const x = tokens(a), y = tokens(b);
    if (x.length * y.length > 400000) return null;
    const m = x.length, n = y.length;
    const l = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
    for (let i = m - 1; i >= 0; i--) for (let j = n - 1; j >= 0; j--) {
      l[i][j] = x[i] === y[j] || (/^\s+$/.test(x[i]) && /^\s+$/.test(y[j])) ? l[i + 1][j + 1] + 1 : Math.max(l[i + 1][j], l[i][j + 1]);
    }
    const raw = [];
    const push = (t, text) => { const last = raw[raw.length - 1]; if (last?.t === t) last.text += text; else raw.push({ t, text }); };
    let i = 0, j = 0;
    while (i < m || j < n) {
      if (i < m && j < n && (x[i] === y[j] || (/^\s+$/.test(x[i]) && /^\s+$/.test(y[j])))) { push("eq", y[j]); i++; j++; }
      else if (j < n && (i === m || l[i][j + 1] >= l[i + 1][j])) { push("ins", y[j]); j++; }
      else { push("del", x[i]); i++; }
    }
    // a lone space between two changes joins them: one deletion, then one insertion
    const out = [];
    for (let k = 0; k < raw.length; k++) {
      const r = raw[k];
      if (r.t === "eq" && /^\s+$/.test(r.text) && raw[k - 1] && raw[k + 1] && raw[k - 1].t !== "eq" && raw[k + 1].t !== "eq") {
        out.push({ t: "del", text: r.text }, { t: "ins", text: r.text });
        continue;
      }
      out.push({ ...r });
    }
    const merged = [];
    for (let k = 0; k < out.length;) {
      if (out[k].t === "eq") { merged.push(out[k]); k++; continue; }
      let del = "", ins = "";
      for (; k < out.length && out[k].t !== "eq"; k++) { if (out[k].t === "del") del += out[k].text; else ins += out[k].text; }
      if (del) merged.push({ t: "del", text: del });
      if (ins) merged.push({ t: "ins", text: ins });
    }
    return merged;
  }

  /** Can this paragraph be drawn piece by piece? */
  const inline = (t) => t != null && !PCTex.needsWhole(t) && !PCTex.heading(t) && !PCTex.isDisplay(t) && !PCTable.isTable(t);

  function runNode(r) {
    const n = el(r.t === "del" ? "del" : r.t === "ins" ? "ins" : "span", r.t === "eq" ? "" : `rv-${r.t}`);
    n.append(PCTex.render(r.text));
    return n;
  }
  /** A whole paragraph drawn as the page draws it (no check marks). */
  function drawText(t) {
    const f = document.createDocumentFragment();
    const head = PCTex.heading(t);
    f.append(PCTex.render(head ? head[1] : t));
    return f;
  }
  /** before → after on the page: word marks when safe, else old struck over new. */
  function diffNode(before, after) {
    const f = document.createDocumentFragment();
    const runs = inline(before) && inline(after) ? diff(before, after) : null;
    if (runs) { runs.forEach((r) => f.append(runNode(r))); return f; }
    const old = el("div", "rv-old"); old.append(drawText(before));
    const now = el("div", "rv-new"); now.append(drawText(after));
    f.append(old, now);
    return f;
  }
  /** A short form for the list: the changed runs with a few words around them. */
  function diffSummary(c) {
    const box = el("div", "rv-txt");
    const clip = (t, n = 160) => (t.length > n ? `${t.slice(0, n)}…` : t);
    if (c.before == null || c.after == null) {
      const n = el(c.before == null ? "ins" : "del", c.before == null ? "rv-ins" : "rv-del");
      const t = c.before ?? c.after;
      if (inline(t)) n.append(PCTex.render(clip(t))); else n.textContent = clip(norm(t), 120);
      box.append(n);
      return box;
    }
    const runs = inline(c.before) && inline(c.after) ? diff(c.before, c.after) : null;
    if (!runs) {
      const d = el("del", "rv-del", clip(norm(c.before), 90)), i = el("ins", "rv-ins", clip(norm(c.after), 90));
      box.append(d, " ", i);
      return box;
    }
    const WORDS = 4;
    runs.forEach((r, k) => {
      if (r.t !== "eq") { box.append(runNode(r)); return; }
      const t = tokens(r.text);
      const first = k === 0, last = k === runs.length - 1;
      if (t.length <= WORDS * 4 + 1 && !first && !last) { box.append(runNode(r)); return; }
      const head = first ? "" : t.slice(0, WORDS * 2).join("");
      const tail = last ? "" : t.slice(-WORDS * 2).join("");
      if (head) box.append(runNode({ t: "eq", text: head }));
      if (head || tail) box.append(el("span", "rv-gap", "…"));
      if (tail) box.append(runNode({ t: "eq", text: tail }));
    });
    return box;
  }

  // ── Comment anchors ─────────────────────────────────
  const esc = (ch) => ch.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  /** Where `c` sits in the paragraph source: [a, b] or null. */
  function anchorIn(src, c) {
    const e = norm(c.exact);
    if (!e) return null;
    const re = new RegExp(e.split(" ").map((w) => [...w].map(esc).join("")).join("[\\s~]+"), "g");
    const sq = (t) => t.replace(/[\s~]+/g, " ");
    const pre = sq(c.prefix ?? ""), suf = sq(c.suffix ?? "");
    let best = null;
    for (const m of src.matchAll(re)) {
      const before = sq(src.slice(0, m.index)), after = sq(src.slice(m.index + m[0].length));
      let s1 = 0; while (s1 < pre.length && s1 < before.length && before[before.length - 1 - s1] === pre[pre.length - 1 - s1]) s1++;
      let s2 = 0; while (s2 < suf.length && s2 < after.length && after[s2] === suf[s2]) s2++;
      if (!best || s1 + s2 > best.score) best = { a: m.index, b: m.index + m[0].length, score: s1 + s2 };
    }
    return best;
  }
  const HOLES = /[\0\u0001]/g;
  /** The reader's text of src[a, b] and how often it shows before a. */
  function shownRange(src, a, b) {
    const mask = PCFind.mask(src);
    const clean = (t) => t.replace(HOLES, "").replace(/[\s~]+/g, " ");
    const q = clean(mask.slice(a, b)).trim();
    if (!q) return null;
    const re = new RegExp(q.split(" ").map((w) => [...w].map(esc).join("")).join("\\s+"), "g");
    const k = [...clean(mask.slice(0, a)).matchAll(re)].length;
    return { re, k };
  }
  const NOT_PROSE = ".math, .katex, .cite, .ref, .pc-num, code, textarea, .tbl, sup.fn, .rv-mk, del";
  /** Wrap the k-th match of re in the drawn paragraph's text with <mark>. */
  function markText(p, re, k, cls, key) {
    const nodes = [];
    const walk = document.createTreeWalker(p, NodeFilter.SHOW_TEXT, {
      acceptNode: (n) => (n.parentElement.closest(NOT_PROSE) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
    });
    let text = "";
    for (let n; (n = walk.nextNode());) { nodes.push([n, text.length]); text += n.data; }
    const found = [...text.matchAll(re)][k];
    if (!found) return false;
    const a = found.index, b = a + found[0].length;
    for (let x = nodes.length - 1; x >= 0; x--) {
      const [n, at] = nodes[x], end = at + n.data.length;
      if (end <= a || at >= b) continue;
      const s0 = Math.max(a, at) - at, e0 = Math.min(b, end) - at;
      const mid = s0 ? n.splitText(s0) : n;
      if (e0 - s0 < mid.data.length) mid.splitText(e0 - s0);
      const m = el("mark", cls);
      m.dataset.key = key;
      mid.replaceWith(m);
      m.append(mid);
    }
    return true;
  }

  /** An anchor from a selection: in the paragraph's editor, or on the drawn page. */
  function anchorFrom(si, pi, sel) {
    const src = sections()[si]?.paragraphs[pi] ?? "";
    const around = (a, b) => ({ exact: src.slice(a, b), prefix: src.slice(Math.max(0, a - 40), a), suffix: src.slice(b, b + 40) });
    if (sel?.ta && sel.b > sel.a && sel.ta.value === src) return around(sel.a, sel.b);
    const t = norm(sel?.text ?? "");
    if (!t) return { exact: "", prefix: "", suffix: "" };
    // the reader's text in the masked source: hidden characters may sit between the letters
    const gap = "[\\0\\u0001]*";
    const body = t.split(" ").map((w) => [...w].map(esc).join(gap)).join(`(?:[\\s~\\0\\u0001])+`);
    let re;
    try { re = new RegExp(body, "g"); } catch { return { exact: "", prefix: "", suffix: "" }; }
    const found = [...PCFind.mask(src).matchAll(re)];
    if (!found.length) return { exact: "", prefix: "", suffix: "" };
    let k = 0;
    const p = paraEl(si, pi);
    if (found.length > 1 && sel.range && p) {
      const r = document.createRange();
      r.setStart(p, 0);
      r.setEnd(sel.range.startContainer, sel.range.startOffset);
      k = Math.min(found.length - 1, (norm(String(r)).match(new RegExp(t.split(" ").map((w) => [...w].map(esc).join("")).join("\\s+"), "g")) ?? []).length);
    }
    const m = found[k];
    return around(m.index, m.index + m[0].length);
  }

  // ── Adding a comment ────────────────────────────────
  /** The inline box: type, Enter adds, Esc cancels. */
  function ask(anchorRect, placeholder, onSubmit) {
    document.querySelector(".rv-ask")?.remove();
    const box = el("div", "menu rv-ask");
    const input = el("textarea");
    input.rows = 2;
    input.placeholder = placeholder;
    const acts = el("div", "rv-ask-acts");
    const no = el("button", "ghost", "取消"), yes = el("button", "primary small", "註解");
    acts.append(no, yes);
    box.append(input, acts);
    document.body.append(box);
    box.style.left = `${Math.max(8, Math.min(anchorRect.left, innerWidth - box.offsetWidth - 8))}px`;
    box.style.top = `${Math.max(8, Math.min(anchorRect.bottom + 6, innerHeight - box.offsetHeight - 8))}px`;
    const close = () => { box.remove(); document.removeEventListener("mousedown", outside, true); };
    const outside = (e) => { if (!box.contains(e.target)) close(); };
    const go = async () => { const v = input.value.trim(); if (!v) { input.focus(); return; } yes.disabled = true; if (await onSubmit(v)) close(); else yes.disabled = false; };
    input.onkeydown = (e) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
      if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); go(); }
    };
    no.onclick = close;
    yes.onclick = go;
    setTimeout(() => document.addEventListener("mousedown", outside, true));
    input.focus();
  }

  /** ＋ 註解 / 新增註解: a selection (editor or page) or the current paragraph. */
  function comment(sel) {
    if (!state.paper) return;
    let si = sel?.si, pi = sel?.pi;
    if (si == null) {
      const ta = document.querySelector("#paper .para > textarea");
      const s = getSelection();
      if (ta && ta.selectionEnd > ta.selectionStart) {
        const p = ta.parentElement;
        sel = { ta, a: ta.selectionStart, b: ta.selectionEnd };
        si = +p.dataset.s; pi = +p.dataset.p;
      } else if (s && !s.isCollapsed && s.anchorNode && $("#paper").contains(s.anchorNode)) {
        const p = s.anchorNode.parentElement?.closest("#paper > .para");
        if (p) { si = +p.dataset.s; pi = +p.dataset.p; sel = { text: String(s), range: s.getRangeAt(0).cloneRange() }; }
      }
      if (si == null) { si = state.cur.s; pi = state.cur.p; sel = null; }
    }
    const sec = sections()[si];
    if (!sec || sec.paragraphs[pi] == null) { LoreShell.toast("先點一下要註解的段落"); return; }
    const anchor = anchorFrom(si, pi, sel);
    const p = paraEl(si, pi);
    const r = sel?.range?.getBoundingClientRect?.() ?? (sel?.ta ? sel.ta.getBoundingClientRect() : p?.getBoundingClientRect());
    ask(r ?? { left: innerWidth / 2 - 160, bottom: innerHeight / 3 }, anchor.exact ? `「${norm(anchor.exact).slice(0, 24)}」` : "註解", async (text) => {
      try {
        data = await api(`${base()}/review/comments`, { file: sec.file, index: pi, ...anchor, text });
        paper = state.paper.name;
        const made = data.comments[data.comments.length - 1];
        if (made) cur = `m:${made.id}`;
        getSelection()?.removeAllRanges();
        after();
        return true;
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); return false; }
    });
  }

  // ── Decisions ───────────────────────────────────────
  /** The paragraph editor saves on blur: let it finish first. */
  async function settle() {
    const ta = document.querySelector("#paper .para > textarea");
    if (!ta) return;
    ta.blur();
    for (let t = 0; t < 80 && document.querySelector("#paper .para > textarea"); t++) await new Promise((ok) => setTimeout(ok, 50));
  }

  async function applied(res) {
    data = res.review;
    if (res.stamp && state.paper) state.paper.stamp = res.stamp;
    paper = state.paper.name;
    for (const sec of res.sections ?? []) {
      const si = secIndex(sec.file);
      if (si < 0) continue;
      sections()[si] = sec;
      setBase(sec);
      await checkSection(si);
      refreshSection(si);
    }
    if (res.issues?.length) LoreShell.toast(res.issues.slice(0, 3).join("　"), { ms: 6000 });
    if (res.sections?.length) refreshGit();
    after();
  }

  async function decide(id, action) {
    await settle();
    const list = items(), at = list.findIndex((x) => x.key === `c:${id}`);
    try {
      await applied(await api(`${base()}/review/changes/${encodeURIComponent(id)}`, { action }));
      const next = items();
      cur = next[Math.min(Math.max(at, 0), next.length - 1)]?.key ?? null;
      paint(); renderPanel();
    } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
  }

  async function all(action) {
    await settle();
    try {
      const res = await api(`${base()}/review/all`, { action });
      await applied(res);
      if (res.skipped) LoreShell.toast(`${res.skipped} 項段落之後又改過，沒有拒絕`, { ms: 6000 });
    } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
  }

  async function editComment(id, body) {
    try { data = await api(`${base()}/review/comments/${encodeURIComponent(id)}`, body, "PUT"); after(); }
    catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
  }

  async function setTracking(on) {
    try { data = await api(`${base()}/review/tracking`, { on }, "PUT"); paper = state.paper.name; after(); }
    catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
  }

  // ── Current item ────────────────────────────────────
  function go(key, scroll = true) {
    cur = key;
    const x = items().find((i) => i.key === key);
    paintCurrent();
    renderPanel();
    renderBar();
    if (!x || x.si < 0 || !scroll) return;
    const target = document.querySelector(`#paper [data-rv-key~="${CSS.escape(key)}"]`) ?? paraEl(x.si, Math.min(x.pi, sections()[x.si].paragraphs.length - 1));
    if (target) PCLazy.jump(target, { block: "center", behavior: "smooth" });
    if (!x.gap && x.pi < sections()[x.si].paragraphs.length && (state.cur.s !== x.si || state.cur.p !== x.pi)) select(x.si, x.pi);
  }
  function step(d) {
    const list = visibleItems();
    if (!list.length) return;
    const i = list.findIndex((x) => x.key === cur);
    go(list[(i < 0 ? (d > 0 ? 0 : list.length - 1) : (i + d + list.length) % list.length)].key);
  }

  // ── The page ────────────────────────────────────────
  function paintCurrent() {
    document.querySelectorAll("#paper .rv-cur").forEach((n) => n.classList.remove("rv-cur"));
    if (!reviewing() || !cur) return;
    document.querySelectorAll(`#paper [data-rv-key~="${CSS.escape(cur)}"]`).forEach((n) => n.classList.add("rv-cur"));
  }

  /** Redraw what review mode shows on the page (idempotent). */
  function paint() {
    const paperEl = $("#paper");
    if (!paperEl) return;
    observer?.disconnect();
    try {
      paperEl.querySelectorAll(":scope > .rv-ghost").forEach((g) => g.remove());
      const list = items();
      const number = new Map(list.map((x, i) => [x.key, i + 1]));
      const show = reviewing();
      paperEl.querySelectorAll(":scope > .para[data-s][data-p]").forEach((p) => {
        const si = +p.dataset.s, pi = +p.dataset.p;
        const text = sections()[si]?.paragraphs[pi];
        if (text == null) return;
        // writing mode: a dot where a comment is open
        const open = list.some((x) => x.kind === "comment" && !x.c.resolved && !x.orphan && x.si === si && x.pi === pi);
        p.classList.toggle("rv-has-cm", !show && open);
        if (p.querySelector(":scope > textarea")) return; // being edited
        const here = show ? list.filter((x) => x.si === si && x.pi === pi && !x.gap && !x.lost && !x.orphan) : [];
        const change = here.find((x) => x.kind === "change" && !x.under);
        const comments = here.filter((x) => x.kind === "comment" && !x.c.resolved);
        const key = here.length && view !== "final" ? JSON.stringify([view, text, here.map((x) => [x.key, number.get(x.key), x.c.before, x.c.exact])]) : view === "final" && here.length ? `final:${here.map((x) => x.key).join(" ")}` : "";
        if ((p._rvKey ?? "") === key) return;
        const n = paraNode(si, pi, text);
        if (key) decorate(n, si, pi, text, here, change, comments, number);
        n._rvKey = key;
        if (key) n._sig = null; // never kept by refreshSection as it is
        p.replaceWith(n);
      });
      if (show && view !== "final") ghosts(paperEl, list, number);
      paintCurrent();
    } finally {
      observer?.takeRecords();
      observer?.observe(paperEl, { childList: true });
    }
  }

  function marker(n, x, num) {
    const m = el("span", `rv-mk ${x.kind === "comment" ? "cm" : "chg"}`, String(num));
    m.dataset.rvKey = x.key;
    m.title = x.c.author;
    m.onclick = (e) => { e.stopPropagation(); go(x.key, false); };
    n.append(m);
  }

  function decorate(n, si, pi, text, here, change, comments, number) {
    n.dataset.rvKey = here.map((x) => x.key).join(" ");
    n.classList.add("rv-para");
    const c = change?.c;
    // changes by others under this one: the page shows everything since the first
    let before = c?.before;
    for (let k = 0; c && before != null && k < here.length; k++) {
      const u = here.find((x) => x.under && norm(x.c.after) === norm(before));
      if (!u) break;
      before = u.c.before;
    }
    if (view === "original") {
      if (c && before == null) { n.classList.add("rv-hidden"); return; }
      if (c && !PCTable.isTable(text)) { n.replaceChildren(drawText(before)); n.classList.add("rv-was"); }
      return;
    }
    if (view === "final") return;
    if (c && !n.classList.contains("tbl")) {
      if (before == null) {
        const ins = el(inline(text) ? "ins" : "div", "rv-ins");
        ins.append(drawText(text));
        n.replaceChildren(ins);
      } else n.replaceChildren(diffNode(before, text));
    } else if (c) n.classList.add("rv-tbl-chg");
    comments.forEach((x) => {
      const at = anchorIn(text, x.c);
      const shown = at && shownRange(text, at.a, at.b);
      if (shown) markText(n, shown.re, shown.k, "rv-cm", x.key);
    });
    // numbered markers in the right margin, one per item
    const col = el("span", "rv-mks");
    n.append(col);
    here.forEach((x) => marker(col, x, number.get(x.key)));
  }

  /** Deleted paragraphs: struck, where they were. */
  function ghosts(paperEl, list, number) {
    list.filter((x) => x.gap && x.si >= 0).forEach((x) => {
      const g = el(view === "original" ? "p" : "div", `rv-ghost${view === "original" ? " rv-was" : ""}`);
      g.dataset.rvKey = x.key;
      const body = view === "original" ? g : el("del", "rv-del");
      if (body !== g) g.append(body);
      body.append(drawText(x.c.before ?? ""));
      if (view !== "original") {
        const col = el("span", "rv-mks");
        marker(col, x, number.get(x.key));
        g.append(col);
      }
      g.onclick = () => go(x.key, false);
      const len = sections()[x.si].paragraphs.length;
      const at = x.pi < len ? paraEl(x.si, x.pi) : null;
      if (at) at.before(g);
      else {
        const last = len ? paraEl(x.si, len - 1) : document.getElementById(`sec-${x.si}`);
        let anchor = last;
        while (anchor?.nextElementSibling?.classList.contains("rv-ghost")) anchor = anchor.nextElementSibling;
        anchor?.after(g);
      }
    });
  }

  function schedule() {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => { queued = false; if (state.paper) { paint(); badges(); } });
  }

  // ── Navigator badges ────────────────────────────────
  function setBadge(b, n) {
    let x = b.querySelector(":scope > .rv-badge");
    if (!n) { x?.remove(); return; }
    if (!x) { x = el("span", "rv-badge"); b.append(x); }
    x.textContent = String(n);
  }
  /** Hook for navLeaf (app.js). */
  function badge(b) {
    if (!reviewing()) return;
    setBadge(b, itemAt(+b.dataset.s, +b.dataset.p).filter((x) => x.kind === "change" || !x.c.resolved).length);
  }
  function badges() {
    document.querySelectorAll("#nav .outline button[data-s][data-p]").forEach((b) => {
      if (reviewing()) badge(b); else setBadge(b, 0);
    });
  }

  // ── Mode switch (top bar) and status bar ────────────
  function renderSwitch() {
    const sw = $("#modeSwitch");
    if (!sw) return;
    sw.hidden = !state.paper;
    sw.replaceChildren();
    const n = count();
    [["write", "寫作"], ["review", "審閱"]].forEach(([m, label]) => {
      const b = el("button", "", label);
      b.setAttribute("aria-pressed", String(mode === m));
      if (m === "review") {
        if (data?.tracking) { const d = el("span", "rv-dot"); d.title = "追蹤修訂"; b.prepend(d); }
        if (n) b.append(el("span", "rv-count", String(n)));
      }
      b.onclick = () => setMode(m);
      sw.append(b);
    });
  }
  /** Hook for renderStatus (app.js): 「審閱 N」 in writing mode. */
  function status(barEl) {
    const n = count();
    if (reviewing() || !n || !state.paper) return;
    const b = el("button", "sync rv-status", `審閱 ${n}`);
    b.onclick = () => setMode("review");
    barEl.append(b);
  }

  function setMode(m) {
    mode = m;
    store.set("pc.reviewMode", m);
    after();
    renderStatus();
  }

  // ── Toolbar ─────────────────────────────────────────
  function menuAt(btn, entries) {
    document.querySelector(".rv-menu")?.remove();
    const m = el("div", "menu rv-menu");
    m.setAttribute("role", "menu");
    entries.forEach(([label, fn, on, off]) => {
      const b = el("button", "", label);
      if (on != null) b.setAttribute("aria-current", String(on));
      b.disabled = !!off;
      b.onclick = () => { m.remove(); fn(); };
      m.append(b);
    });
    document.body.append(m);
    const r = btn.getBoundingClientRect();
    m.style.top = `${r.bottom + 4}px`;
    m.style.left = `${Math.max(8, Math.min(r.left, innerWidth - m.offsetWidth - 8))}px`;
    btn.setAttribute("aria-expanded", "true");
    const close = (e) => {
      if (e.type === "keydown" && e.key !== "Escape") return;
      if (e.type === "mousedown" && (m.contains(e.target) || e.target === btn)) return;
      m.remove(); btn.setAttribute("aria-expanded", "false");
      document.removeEventListener("mousedown", close, true);
      document.removeEventListener("keydown", close, true);
    };
    setTimeout(() => { document.addEventListener("mousedown", close, true); document.addEventListener("keydown", close, true); });
  }

  function tb(label, fn, cls) {
    const b = el("button", `rv-tb${cls ? ` ${cls}` : ""}`);
    b.append(label);
    b.onmousedown = (e) => e.preventDefault(); // keep the selection on the page
    b.onclick = () => fn(b);
    return b;
  }

  function renderBar() {
    if (!bar) return;
    bar.replaceChildren();
    if (!reviewing()) return;
    const g = (...xs) => { const d = el("div", "grp"); d.append(...xs); bar.append(d); };
    const track = tb("追蹤修訂", () => setTracking(!data?.tracking), "rv-track");
    track.setAttribute("aria-pressed", String(!!data?.tracking));
    if (data?.tracking) track.prepend(el("span", "rv-dot"));
    g(track);
    g(tb("＋ 註解", () => comment()));
    const list = visibleItems(), i = list.findIndex((x) => x.key === cur);
    const prev = tb("‹", () => step(-1)), next = tb("›", () => step(1));
    prev.setAttribute("aria-label", "上一項"); next.setAttribute("aria-label", "下一項");
    prev.disabled = next.disabled = !list.length;
    g(prev, el("span", "rv-pos", list.length ? `${i < 0 ? "–" : i + 1} / ${list.length}` : "0"), next);
    const x = list[i];
    const ok = tb("✓ 接受", () => x && decide(x.c.id, "accept"), "ok");
    const no = tb("✕ 拒絕", () => x && decide(x.c.id, "reject"), "no");
    ok.disabled = no.disabled = !x || x.kind !== "change";
    if (x?.lost || x?.under) no.disabled = true;
    const n = data?.changes.length ?? 0, open = data?.comments.filter((c) => !c.resolved).length ?? 0;
    const allB = tb("全部 ⌄", (b) => menuAt(b, [
      ["接受全部", () => all("accept"), null, !n],
      ["拒絕全部", () => all("reject"), null, !n],
      ["解決全部註解", () => all("resolve"), null, !open],
    ]));
    g(ok, no, allB);
    g(tb(`顯示：${VIEWS[view]} ⌄`, (b) => menuAt(b, Object.entries(VIEWS).map(([k, label]) => [label, () => { view = k; paint(); renderBar(); }, view === k]))));
  }

  // ── The review list (right panel) ───────────────────
  function who(x) {
    const d = el("div", "rv-who");
    const av = el("span", "rv-av");
    av.style.background = colour(x.c.author);
    d.append(av, el("span", "rv-name", x.c.author), el("span", "rv-time", when(x.c)), el("span", "rv-where", where(x)));
    return d;
  }

  function changeCard(x) {
    const card = el("div", "rv-item");
    card.append(who(x));
    if (x.lost) card.append(el("div", "rv-note", "原文已更動"));
    card.append(diffSummary(x.c));
    const acts = el("div", "rv-acts");
    const ok = el("button", "ghost small ok", "✓"), no = el("button", "ghost small no", "✕");
    ok.setAttribute("aria-label", "接受"); no.setAttribute("aria-label", "拒絕");
    ok.title = "接受"; no.title = "拒絕";
    ok.onclick = (e) => { e.stopPropagation(); decide(x.c.id, "accept"); };
    no.onclick = (e) => { e.stopPropagation(); decide(x.c.id, "reject"); };
    no.disabled = x.lost || x.under;
    if (x.under) no.title = "先處理之後的修改";
    acts.append(ok, no);
    card.append(acts);
    return card;
  }

  function commentCard(x) {
    const c = x.c;
    const card = el("div", `rv-item${c.resolved ? " resolved" : ""}`);
    card.append(who(x));
    if (x.orphan) {
      const n = el("div", "rv-note", "原文已更動");
      if (c.exact) n.append(el("span", "rv-quote", `「${norm(c.exact).slice(0, 40)}」`));
      card.append(n);
    }
    card.append(el("div", "rv-txt", c.text));
    c.replies.forEach((r) => {
      const d = el("div", "rv-reply");
      d.append(el("b", "", `${r.author}：`), r.text);
      card.append(d);
    });
    if (!c.resolved) {
      const input = el("input", "rv-input");
      input.placeholder = "回覆…";
      input.onclick = (e) => e.stopPropagation();
      input.onkeydown = (e) => {
        if (e.key === "Enter" && !e.isComposing && input.value.trim()) editComment(c.id, { reply: input.value });
      };
      card.append(input);
    }
    const acts = el("div", "rv-acts");
    const done = el("button", "ghost small", c.resolved ? "重新開啟" : "已解決");
    done.onclick = (e) => { e.stopPropagation(); editComment(c.id, { resolved: !c.resolved }); };
    acts.append(done);
    card.append(acts);
    return card;
  }

  function renderPanel() {
    if (!panel) return;
    panel.hidden = !reviewing();
    if (panel.hidden) return;
    const scroll = panel.scrollTop;
    const box = panel.body;
    box.replaceChildren();
    const list = items();
    const nChg = list.filter((x) => x.kind === "change").length;
    const nCm = list.filter((x) => x.kind === "comment" && !x.c.resolved).length;
    panel.setCount(nChg + nCm || "");
    const tabs = el("div", "rv-tabs");
    [["all", "全部", nChg + nCm], ["changes", "修改", nChg], ["comments", "註解", nCm]].forEach(([k, label, n]) => {
      const b = el("button", "", label);
      b.append(el("span", "c", String(n)));
      b.setAttribute("aria-pressed", String(tab === k));
      b.onclick = () => { tab = k; renderPanel(); renderBar(); };
      tabs.append(b);
    });
    box.append(tabs);
    const shown = visibleItems();
    if (!shown.length) box.append(el("p", "rv-empty", "沒有待處理的項目"));
    shown.forEach((x) => {
      const card = x.kind === "change" ? changeCard(x) : commentCard(x);
      card.classList.toggle("cur", x.key === cur);
      card.onclick = () => go(x.key);
      box.append(card);
    });
    // resolved comments: hidden from counts, shown on request
    const nRes = data?.comments.filter((c) => c.resolved).length ?? 0;
    if (nRes && tab !== "changes") {
      const t = el("button", "link rv-resolved", `${resolvedShown ? "隱藏" : "顯示"}已解決 ${nRes}`);
      t.onclick = () => { resolvedShown = !resolvedShown; after(); };
      box.append(t);
    }
    refsRow();
    panel.scrollTop = scroll;
  }

  /** Word's reference list as read into fields: 「查證」 asks Crossref / PubMed. */
  function refsRow() {
    const open = wordRefs.filter((r) => !r.checked).length;
    if (!open) return;
    const row = el("div", "rv-refs");
    row.append(el("span", "", `Word 文獻 ${open}`));
    const b = el("button", "ghost small", refsBusy ? "查證中…" : "查證");
    b.disabled = refsBusy;
    b.onclick = async () => {
      refsBusy = true; renderPanel();
      try {
        const r = await api(`${base()}/word-refs/confirm`, {});
        wordRefs = r.refs ?? wordRefs;
        const bits = [`已更新 ${r.updated.length}`];
        if (r.ambiguous.length) bits.push(`待確認 ${r.ambiguous.join("、")}`);
        if (r.missing.length) bits.push(`查不到 ${r.missing.join("、")}`);
        LoreShell.toast(bits.join("；"), { ms: 6000 });
      } catch (e) { LoreShell.toast(e.message, { err: true, ms: 6000 }); }
      refsBusy = false; renderPanel();
    };
    row.append(b);
    panel.body.append(row);
  }

  // ── Everything after the data or the mode changed ───
  function after() {
    const on = reviewing();
    document.querySelector(".app")?.classList.toggle("reviewing", on);
    $("#writebar").hidden = on;
    if (bar) bar.hidden = !on;
    if (cur && !items().some((x) => x.key === cur)) cur = null;
    renderSwitch();
    renderBar();
    renderPanel();
    paint();
    badges();
    renderStatus();
  }

  function init() {
    bar = el("div", "writebar capsule reviewbar");
    bar.id = "reviewbar";
    bar.setAttribute("role", "toolbar");
    bar.setAttribute("aria-label", "審閱工具");
    bar.hidden = true;
    $("#writebar").after(bar);
    // the review list: the shell's numbered block (01 審閱 N ±)
    panel = LoreShell.block({ n: "01", title: "審閱", key: "pc.review" });
    panel.classList.add("rv-panel");
    panel.hidden = true;
    $("aside.right").append(panel);
    observer = new MutationObserver(schedule);
    observer.observe($("#paper"), { childList: true });
    // the navigator is rebuilt (renderNav / renderNavSection): count again
    new MutationObserver(() => { if (reviewing()) schedule(); }).observe($("#nav"), { childList: true });
    renderSwitch();
  }

  return { init, load, refresh, badge, status, comment, count, setMode, get mode() { return mode; } };
})();
window.PCReview = PCReview;
PCReview.init();
