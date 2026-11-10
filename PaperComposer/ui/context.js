// Right-click menu on the paper: cite, refer to a figure / table / equation,
// add a figure or a table, collect a phrase, cut and copy.
//
// In a paragraph being edited (its textarea) things go in at the caret. On a
// drawn paragraph the paragraph stays as it is: a thin caret marks the
// clicked point, which is mapped back to the LaTeX source through the same
// masking find.js searches with (PCFind.mask) — letters and digits of the
// drawn prose are counted between citations / references / formulas, the
// same letters in the masked source. A click on a citation, reference or
// formula itself puts the new text right after it. What was inserted is
// flashed once the paragraph is redrawn.
//
// Every insertion goes into sections()[si].paragraphs[pi] as \cite{…},
// \ref{…} / \eqref{…} (through PCWrite.refSnippet, which defers to the
// template's form when tex.js has one), checked against the notation
// registry (POST /api/papers/{name}/notation/check) before it is saved.
//
// ⌥ or Shift + right-click gives the system menu. 貼上 is not offered: a
// web page cannot read the clipboard without a prompt (WKWebView asks with
// its own Paste bubble), so ⌘V stays the way to paste.

const PCContext = (() => {
  const MAC = /Mac/.test(navigator.platform);
  const CITE = /^(cite|citep|citet|citealp|citealt|citeauthor|citeyear|parencite|textcite|autocite|footcite|supercite|Citet|Citep|Citealp|Citeauthor|Parencite|Textcite)$/;
  const REF = /^(ref|eqref|autoref|cref|Cref|pageref|vref|subref)$/;
  const NOTE = /^(footnote|footnotetext|thanks)$/;
  const MATH_ENV = /^(equation|align|alignat|gather|multline|eqnarray|flalign|displaymath|math|dmath)\*?$/;
  const ATOM = ".math, .cite, .ref, sup.fn";
  const NOT_PROSE = ".math, .katex, .cite, .ref, .pc-num, code, textarea, .tbl, sup.fn";
  const WORD = /[\p{L}\p{N}]/u;
  const SPACE = /[\s~]/;
  const POPS = ".wb-pop, #refPop, #styleGallery";

  let menu = null;    // the open menu
  let marker = null;  // the caret on the drawn page (also the popovers' anchor)
  let down = null;    // selection as it was when the right button went down
  let watch = 0;

  // ── LaTeX source: citations, references, footnotes and formulas, in order ──
  function group(s, i) {
    if (s[i] !== "{") return -1;
    let d = 0;
    for (let k = i; k < s.length; k++) {
      if (s[k] === "\\") { k++; continue; }
      if (s[k] === "{") d++;
      else if (s[k] === "}" && --d === 0) return k + 1;
    }
    return s.length;
  }
  function optional(s, i) {
    if (s[i] !== "[") return -1;
    for (let k = i + 1; k < s.length; k++) {
      if (s[k] === "\\") { k++; continue; }
      if (s[k] === "]") return k + 1;
    }
    return s.length;
  }
  const blank = (s, i) => { while (s[i] === " " || s[i] === "\t") i++; return i; };
  function close(s, i, end) {
    for (let k = i; k < s.length; k++) {
      if (s.startsWith(end, k)) return k + end.length;
      if (s[k] === "\\") k++;
    }
    return s.length;
  }

  /** [{ a, b, kind: "cite" | "ref" | "fn" | "math" }] in source order. */
  function atoms(s) {
    const out = [];
    const n = s.length;
    for (let i = 0; i < n;) {
      const c = s[i];
      if (c === "%") { const j = s.indexOf("\n", i); i = j < 0 ? n : j; continue; }
      if (c === "$") {
        const z = s[i + 1] === "$" ? close(s, i + 2, "$$") : close(s, i + 1, "$");
        out.push({ a: i, b: z, kind: "math" }); i = z; continue;
      }
      if (c !== "\\") { i++; continue; }
      const d = s[i + 1];
      if (d === "(" || d === "[") { const z = close(s, i + 2, d === "(" ? "\\)" : "\\]"); out.push({ a: i, b: z, kind: "math" }); i = z; continue; }
      if (!/[a-zA-Z]/.test(d ?? "")) { i += 2; continue; }
      let j = i + 1;
      while (j < n && /[a-zA-Z]/.test(s[j])) j++;
      const name = s.slice(i + 1, j);
      if (s[j] === "*") j++;
      if (CITE.test(name) || REF.test(name) || NOTE.test(name)) {
        let k = j;
        for (let o; (o = optional(s, blank(s, k))) > 0;) k = o;
        const g = group(s, blank(s, k));
        if (g < 0) { i = j; continue; }
        out.push({ a: i, b: g, kind: CITE.test(name) ? "cite" : REF.test(name) ? "ref" : "fn" });
        i = g; continue;
      }
      if (name === "begin") {
        const g = group(s, blank(s, j));
        const env = g > 0 ? s.slice(blank(s, j) + 1, g - 1) : "";
        if (MATH_ENV.test(env)) { const z = close(s, g, `\\end{${env}}`); out.push({ a: i, b: z, kind: "math" }); i = z; continue; }
      }
      i = j;
    }
    return out;
  }
  const kindOf = (e) => (e.matches(".cite") ? "cite" : e.matches(".math") ? "math" : e.matches("sup.fn") ? "fn" : "ref");

  /** The drawn counterparts of atoms(): \ref{a,b} draws two .ref in one plain span. */
  function drawnAtoms(p) {
    const out = [];
    p.querySelectorAll(ATOM).forEach((e) => {
      if (e.parentElement.closest(ATOM)) return;
      const up = e.parentElement;
      const multi = e.matches(".ref") && up !== p && up.tagName === "SPAN" && !up.className && up.querySelectorAll(":scope > .ref").length > 1;
      const node = multi ? up : e;
      if (out[out.length - 1]?.node !== node) out.push({ node, kind: multi ? "ref" : kindOf(e) });
    });
    return out;
  }
  /** The source atom matching drawn atom i: the same kind, the same rank. */
  function sourceAtom(src, drawn, i) {
    const kind = drawn[i].kind;
    const rank = drawn.slice(0, i).filter((x) => x.kind === kind).length;
    return atoms(src).filter((x) => x.kind === kind)[rank] ?? null;
  }

  const nonWsBefore = (s, off) => s.slice(0, off).replace(/\s/g, "").length;
  function offsetOf(s, nonWs) {
    let seen = 0, at = 0;
    while (at < s.length && seen < nonWs) { if (!/\s/.test(s[at])) seen++; at++; }
    return at;
  }

  // ── a click on a drawn paragraph → a place in its source ──
  function caretAt(x, y) {
    if (document.caretRangeFromPoint) return document.caretRangeFromPoint(x, y);
    const c = document.caretPositionFromPoint?.(x, y);
    if (!c) return null;
    const r = document.createRange();
    r.setStart(c.offsetNode, c.offset);
    return r;
  }

  /** { nonWs, rect } for a click at (x, y) on drawn paragraph p; nonWs null = end of paragraph. */
  function locate(p, src, x, y, target) {
    const drawn = drawnAtoms(p);
    // on a citation, reference, formula or footnote mark: right after it
    const hit = target.closest?.(ATOM);
    if (hit && p.contains(hit)) {
      const i = drawn.findIndex((d) => d.node === hit || d.node.contains(hit));
      const a = i >= 0 ? sourceAtom(src, drawn, i) : null;
      const r = (drawn[i]?.node ?? hit).getBoundingClientRect();
      return { nonWs: a ? nonWsBefore(src, a.b) : null, rect: { left: r.right, top: r.top, height: r.height } };
    }
    const range = caretAt(x, y);
    let node = range?.startContainer, off = range?.startOffset ?? 0;
    if (!node || node.nodeType !== 3 || !p.contains(node) || node.parentElement.closest(NOT_PROSE)) return { nonWs: null, rect: null };
    // never inside a word: to its end
    while (off < node.data.length && WORD.test(node.data[off]) && off > 0 && WORD.test(node.data[off - 1])) off++;
    const rect = caretRect(node, off);

    // drawn: letters since the last atom before the click, then what follows them
    const sources = atoms(src);
    const aligned = sources.length === drawn.length && drawn.every((d, i) => d.kind === sources[i].kind);
    const atomNodes = new Set(drawn.map((d) => d.node));
    let k = 0, letters = 0, tail = 0, spaced = false;
    const walk = document.createTreeWalker(p, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
      acceptNode: (n) => {
        if (n.nodeType === 3) return NodeFilter.FILTER_ACCEPT;
        if (atomNodes.has(n)) return NodeFilter.FILTER_ACCEPT;
        return n.matches(NOT_PROSE) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_SKIP;
      },
    });
    // letters: since the last atom; tail: marks after the last letter; spaced: a space just before the click
    const count = (text) => {
      for (const ch of text) {
        if (WORD.test(ch)) { letters++; tail = 0; spaced = false; }
        else if (/\s/.test(ch)) spaced = true;
        else { tail++; spaced = false; }
      }
    };
    for (let n; (n = walk.nextNode());) {
      if (n.nodeType === 1) {
        if (aligned) { k++; letters = 0; tail = 0; spaced = false; }
        // skip the atom's own text
        let last = n; while (last.lastChild) last = last.lastChild;
        walk.currentNode = last;
        continue;
      }
      if (n.parentElement.closest(NOT_PROSE)) continue;
      if (n === node) { count(n.data.slice(0, off)); break; }
      count(n.data);
    }

    // source: the same letters from the same atom on, atoms themselves skipped
    const mask = PCFind.mask(src);
    const skip = sources;
    let i = aligned && k ? sources[k - 1].b : 0;
    const inAtom = (j) => skip.find((x) => j >= x.a && j < x.b);
    let seen = 0;
    if (letters) {
      for (; i < src.length; i++) {
        const a = inAtom(i);
        if (a) { i = a.b - 1; continue; }
        if (WORD.test(mask[i]) && ++seen === letters) { i++; break; }
      }
    }
    // the marks after the last letter (a period, a parenthesis), then a space
    for (let t = tail; t > 0 && i < src.length; i++) {
      const ch = mask[i];
      if (ch === PCFind.HOLE || ch === PCFind.ESC || SPACE.test(ch)) continue;
      if (WORD.test(ch) || inAtom(i)) break;
      t--;
    }
    if (spaced) while (i < src.length && (SPACE.test(src[i]))) i++;
    return { nonWs: nonWsBefore(src, i), rect };
  }

  function caretRect(node, off) {
    const r = document.createRange();
    r.setStart(node, off);
    r.setEnd(node, off);
    let b = r.getClientRects()[0] ?? r.getBoundingClientRect();
    if (!b || !b.height) {
      // a collapsed range at a line edge may have no box: use the character before
      if (off > 0) { r.setStart(node, off - 1); const c = r.getClientRects(); const last = c[c.length - 1]; if (last) return { left: last.right, top: last.top, height: last.height }; }
      return null;
    }
    return { left: b.left, top: b.top, height: b.height };
  }

  // ── inserting ──
  const base = () => `/api/papers/${encodeURIComponent(state.paper.name)}`;
  const stripped = (s) => s.replace(/\s/g, "");

  /** The paragraph editor saves on blur: wait for it to finish before changing paragraphs. */
  async function settle() {
    const ta = document.querySelector("#paper .para > textarea");
    if (!ta) return;
    ta.blur();
    for (let t = 0; t < 80 && document.querySelector("#paper .para > textarea"); t++) await new Promise((ok) => setTimeout(ok, 50));
  }

  /** The paragraph's text now, if it is still the one clicked. */
  function fresh(at) {
    const t = sections()[at.si]?.paragraphs[at.pi];
    if (t == null || (at.sig != null && stripped(t) !== at.sig)) { flash("段落已變更，請再點一次"); return null; }
    return t;
  }

  /** The notation registry's checks on what changes; true when it may go ahead. */
  /** A question beside the paragraph, in the app's own look: true = 仍要插入. Esc / outside = 取消. */
  function ask(at, lines) {
    return new Promise((resolve) => {
      const box = el("div", "menu ctx-ask");
      box.setAttribute("role", "alertdialog");
      const msg = el("div", "ctx-ask-msg");
      lines.slice(0, 6).forEach((l) => msg.append(el("div", "", l)));
      const acts = el("div", "ctx-ask-acts");
      const no = el("button", "ghost", "取消"), yes = el("button", "primary small", "仍要插入");
      acts.append(no, yes);
      box.append(msg, acts);
      document.body.append(box);
      const r = (paraEl(at.si, at.pi) ?? $("#paper")).getBoundingClientRect();
      box.style.left = `${Math.max(8, Math.min(r.left, innerWidth - box.offsetWidth - 8))}px`;
      box.style.top = `${Math.max(8, Math.min(r.bottom + 6, innerHeight - box.offsetHeight - 8))}px`;
      const done = (v) => {
        box.remove();
        document.removeEventListener("keydown", key, true);
        document.removeEventListener("mousedown", outside, true);
        resolve(v);
      };
      const key = (e) => { if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); done(false); } };
      const outside = (e) => { if (!box.contains(e.target)) done(false); };
      document.addEventListener("keydown", key, true);
      document.addEventListener("mousedown", outside, true);
      no.onclick = () => done(false);
      yes.onclick = () => done(true);
      yes.focus();
    });
  }

  async function vet(before, after, at) {
    let r;
    try { r = await api(`${base()}/notation/check`, { before, texts: after, find: "" }); }
    catch (e) { return ask(at, [`對照表檢查失敗：${e.message}`]); }
    const lines = [...(r.blocking ?? []), ...(r.shown ?? []), ...(r.new_symbols?.length ? [`對照表沒有的符號：${r.new_symbols.join("、")}`] : [])];
    if (r.blocking?.length) return ask(at, lines);
    if (lines.length) flash(lines.slice(0, 3).join("　"), 6000);
    return true;
  }

  /** Put `now` in place of the clicked paragraph (and `block` after it), checked and saved. */
  async function commit(at, was, now, block) {
    if (!(await vet([was], block ? [now, block] : [now], at))) return false;
    const sec = sections()[at.si];
    sec.paragraphs[at.pi] = now;
    if (block != null) sec.paragraphs.splice(at.pi + 1, 0, block);
    try { await saveSection(at.si); } catch (e) { flash(`儲存失敗：${e.message}`, 6000); return false; }
    return true;
  }

  /** After the paragraph is redrawn: flash what went in, or reopen the editor after it. */
  function land(at, was) {
    const now = sections()[at.si]?.paragraphs[at.pi];
    if (now == null || now === was) return;
    const a = stripped(was), b = stripped(now);
    let pre = 0;
    while (pre < a.length && a[pre] === b[pre]) pre++;
    const p = paraEl(at.si, at.pi);
    if (!p) return;
    if (at.fromTa) {
      edit(p, at.si, at.pi);
      const ta = p.querySelector(":scope > textarea");
      if (ta) { const end = offsetOf(ta.value, pre + b.length - a.length); ta.setSelectionRange(end, end); }
      return;
    }
    const src = atoms(now);
    const i = src.findIndex((x) => nonWsBefore(now, x.a) >= pre);
    if (i < 0) return;
    const kind = src[i].kind;
    const rank = src.slice(0, i).filter((x) => x.kind === kind).length;
    const node = drawnAtoms(p).filter((x) => x.kind === kind)[rank]?.node;
    if (!node) return;
    const area = $("#pageArea").getBoundingClientRect(), r = node.getBoundingClientRect();
    if (r.top < area.top + 60 || r.bottom > area.bottom - 20) node.scrollIntoView({ block: "center", behavior: "smooth" });
    node.classList.remove("ctx-new");
    void node.offsetWidth;
    node.classList.add("ctx-new");
    setTimeout(() => node.classList.remove("ctx-new"), 2200);
  }

  function cite(at) {
    PCRefs.picker(marker, {
      si: at.si, pi: at.pi, nonWs: at.nonWs,
      vet: async (was, now) => (fresh(at) != null) && vet([was], [now], at),
      done: (was) => { land(at, was); unmark(); },
    });
    keepMarker();
  }

  function reference(at, anchor) {
    PCWrite.references(anchor ?? marker, async (kind, key) => {
      await settle();
      const was = fresh(at);
      if (was == null) return unmark();
      const snippet = PCWrite.refSnippet(kind, key, PCWrite.sentenceStart(was.slice(0, offsetOf(was, at.nonWs))));
      const now = PCRefs.insertAt(was, at.nonWs, snippet, " ");
      if (await commit(at, was, now)) { refreshSection(at.si); refreshGit(); land(at, was); }
      unmark();
    });
    keepMarker();
  }

  /** A reference to the new float in the clicked paragraph (if a point was clicked). */
  function withRef(at, was, kind, key) {
    if (at.nonWs == null) return was;
    const snippet = PCWrite.refSnippet(kind, key, PCWrite.sentenceStart(was.slice(0, offsetOf(was, at.nonWs))));
    return PCRefs.insertAt(was, at.nonWs, snippet, " ");
  }
  /** label: like the draft wizard's (lib.rs float_label), never one already taken. */
  function newLabel(prefix, words, n) {
    const w = words.split(/[^A-Za-z0-9]+/).filter((x) => x.length > 2).slice(0, 3).map((x) => x.toLowerCase());
    const stem = w.length ? `${prefix}:${w.join("-")}` : `${prefix}:${prefix[0]}${n}`;
    let label = stem;
    for (let k = 2; label in PCTex.labels; k++) label = `${stem}-${k}`;
    return label;
  }
  const texText = (s) => s.trim().replace(/(?<!\\)([%&#])/g, "\\$1");

  function table(at) {
    PCTable.insertNew(marker, async (r) => {
      await settle();
      const was = fresh(at);
      if (was == null) { unmark(); return null; }
      const t = r.table;
      const n = Object.values(PCTex.kinds).filter((k) => k === "tab").length + 1;
      t.label = newLabel("tab", "", n);
      const where = state.paper.captions?.table;
      if (where === "above" || where === "below") t.caption_below = where === "below";
      let text = r.text;
      try { text = (await api("/api/textable/render", t)).text; } catch (e) { flash(e.message, 6000); unmark(); return null; }
      const now = withRef(at, was, "tab", t.label);
      if (!(await commit(at, was, now, text))) { unmark(); return null; }
      return { si: at.si, pi: at.pi + 1, text: sections()[at.si].paragraphs[at.pi + 1], table: t, after: () => { land(at, was); unmark(); } };
    });
    keepMarker();
  }

  function figure(at) {
    const p = PCWrite.popover(marker, (p) => {
      p.classList.add("ctx-fig");
      const file = el("input");
      file.type = "file";
      file.accept = ".png,.jpg,.jpeg,.pdf,.eps,image/png,image/jpeg,application/pdf";
      file.hidden = true;
      const pickBtn = el("button", "ghost ctx-file", "選擇圖檔");
      pickBtn.prepend(icon("upload"));
      pickBtn.onclick = () => file.click();
      const name = el("span", "ctx-fname");
      const cap = el("input", "search");
      cap.placeholder = "圖說";
      const engine = chosenEngine();
      const gen = iconButton("ai", engine ? "自動產生圖說" : "連接寫作引擎後可自動產生", async () => {
        if (!file.files[0]) return flash("先選擇圖檔");
        gen.disabled = true; cap.placeholder = "產生中…";
        try {
          const title = state.paper.fields?.find((f) => f.field === "title")?.value ?? "";
          const r = await api("/api/wizard/caption", { kind: "figure", data: "", file: file.files[0].name, section: sections()[at.si]?.title ?? "", key_value: state.paper.meta?.key_value ?? "", title, engine: chosenEngine() });
          if (r.caption) cap.value = r.caption; else flash("從檔名看不出這張圖的內容，請自己填圖說");
        } catch (e) { flash(e.message, 5000); }
        gen.disabled = false; cap.placeholder = "圖說";
      });
      gen.disabled = !engine;
      const go = el("button", "primary small", "插入");
      go.disabled = true;
      file.onchange = () => { name.textContent = file.files[0]?.name ?? ""; go.disabled = !file.files[0]; };
      go.onclick = async () => {
        const f = file.files[0];
        if (!f) return;
        go.disabled = true; go.textContent = "插入中…";
        const ok = await placeFigure(at, f, cap.value);
        if (ok) p.remove(); else { go.disabled = false; go.textContent = "插入"; }
      };
      cap.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing && !go.disabled) go.click(); };
      const row1 = el("div", "ctx-row");
      row1.append(pickBtn, name, file);
      const row2 = el("div", "ctx-row");
      row2.append(cap, gen);
      const foot = el("div", "ctx-row end");
      foot.append(go);
      p.append(row1, row2, foot);
    });
    keepMarker();
    return p;
  }

  async function placeFigure(at, f, caption) {
    await settle();
    const was = fresh(at);
    if (was == null) return false;
    const m = f.name.match(/^(.*?)(?:\.([^.]+))?$/);
    const stem = (m[1] ?? "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40) || "figure";
    const ext = (m[2] ?? "png").toLowerCase();
    const used = new Set(sections().flatMap((s) => s.paragraphs.flatMap((t) => [...t.matchAll(/\\includegraphics\*?(?:\[[^\]]*\])?\{([^}]*)\}/g)].map((x) => x[1].replace(/^figures\//, "")))));
    let fname = `${stem}.${ext}`;
    for (let k = 2; used.has(fname); k++) fname = `${stem}-${k}.${ext}`;
    const up = await fetch(`${base()}/figures?file=${encodeURIComponent(fname)}`, { method: "POST", body: f });
    if (!up.ok) { flash(await up.text(), 6000); return false; }
    const n = Object.values(PCTex.kinds).filter((k) => k === "fig").length + 1;
    const label = newLabel("fig", caption || stem, n);
    const capLines = [`  \\caption{${texText(caption)}}`, `  \\label{${label}}`];
    const img = `  \\includegraphics[width=\\linewidth]{figures/${fname}}`;
    const body = state.paper.captions?.figure === "above" ? [...capLines, img] : [img, ...capLines];
    const block = ["\\begin{figure}[t]", "  \\centering", ...body, "\\end{figure}"].join("\n");
    const now = withRef(at, was, "fig", label);
    if (!(await commit(at, was, now, block))) return false;
    try {
      const r = await api(`${base()}/packages`, { packages: ["graphicx"] });
      if (r.added?.length) flash(`main.tex 已加入 \\usepackage{graphicx}`, 5000);
    } catch {}
    refreshSection(at.si);
    refreshGit();
    land(at, was);
    unmark();
    return true;
  }

  // ── clipboard ──
  function copy(d, cut) {
    if (d.ta) {
      d.ta.focus();
      d.ta.setSelectionRange(d.a, d.b);
      if (document.execCommand(cut ? "cut" : "copy")) return;
      navigator.clipboard?.writeText(d.text).then(() => {
        if (cut) { d.ta.setRangeText("", d.a, d.b, "end"); d.ta.dispatchEvent(new Event("input")); }
      }, () => flash("無法使用剪貼簿"));
      return;
    }
    if (String(getSelection() ?? "") === d.text && document.execCommand("copy")) return;
    navigator.clipboard?.writeText(d.text).catch(() => flash("無法使用剪貼簿"));
  }

  // ── the marker ──
  function mark(rect, x, y) {
    unmark();
    marker = el("div", "ctx-caret");
    if (rect) Object.assign(marker.style, { left: `${rect.left - 1}px`, top: `${rect.top}px`, height: `${rect.height}px` });
    else { marker.classList.add("off"); Object.assign(marker.style, { left: `${x}px`, top: `${y}px`, height: "1px" }); }
    document.body.append(marker);
  }
  function unmark() {
    clearInterval(watch);
    marker?.remove();
    marker = null;
  }
  /** Keep the marker while a popover started from the menu is open. */
  function keepMarker() {
    clearInterval(watch);
    let seen = false, ticks = 0;
    watch = setInterval(() => {
      ticks++;
      if (document.querySelector(POPS)) seen = true;
      // closed, or never opened (nothing to refer to, the library failed to load)
      else if (seen || !marker || ticks > 30) unmark();
    }, 200);
  }

  // ── the menu ──
  function hide(keep) {
    if (!menu) return;
    menu.remove();
    menu = null;
    document.removeEventListener("keydown", keys, true);
    document.removeEventListener("mousedown", outside, true);
    removeEventListener("blur", hideAll);
    removeEventListener("resize", hideAll);
    $("#pageArea")?.removeEventListener("scroll", hideAll);
    if (!keep) unmark();
  }
  const hideAll = () => hide(false);
  function outside(e) { if (!menu?.contains(e.target)) hide(false); }

  function items() { return [...menu.querySelectorAll("button:not([disabled])")]; }
  function focusItem(i) {
    const list = items();
    list.forEach((b) => b.classList.remove("on"));
    const b = list[(i + list.length) % list.length];
    b?.classList.add("on");
  }
  function keys(e) {
    if (!menu) return;
    const list = items(), i = list.findIndex((b) => b.classList.contains("on"));
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); hide(false); return; }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); e.stopPropagation(); focusItem(i < 0 ? (e.key === "ArrowDown" ? 0 : -1) : i + (e.key === "ArrowDown" ? 1 : -1)); return; }
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      // the row of 剪下 / 複製 reads left to right too
      e.preventDefault(); e.stopPropagation();
      if (e.key === "ArrowRight" && list[i]?.classList.contains("sub")) { list[i].click(); return; }
      focusItem(i < 0 ? 0 : i + (e.key === "ArrowRight" ? 1 : -1));
      return;
    }
    if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); (list[i] ?? list[0])?.click(); return; }
    if (e.key === "Tab") { e.preventDefault(); e.stopPropagation(); return; }
    hide(false); // typing on: the menu goes
  }

  function show(x, y, d, at) {
    hide(false);
    menu = el("div", "menu ctx-menu");
    menu.setAttribute("role", "menu");
    const add = (ic, label, fn, cls) => {
      const b = el("button", cls ?? "");
      b.setAttribute("role", "menuitem");
      if (ic) b.append(icon(ic));
      b.append(el("span", "lbl", label));
      if (cls === "sub") b.append(el("span", "more", "▸"));
      // never take the focus: a paragraph being edited would save and close
      b.onmousedown = (e) => e.preventDefault();
      b.onmouseenter = () => focusItem(items().indexOf(b));
      b.onclick = () => { const keep = fn(b); hide(keep !== false); };
      menu.append(b);
      return b;
    };
    const sep = () => menu.append(el("div", "sep"));
    if (at.nonWs != null) {
      add("quote", "引用文獻…", () => { settle().then(() => cite(at)); });
      // the list opens where the menu was, from this item down
      add("ref", "參照圖表…", (b) => {
        const r = b.getBoundingClientRect(), a = el("div", "ctx-anchor");
        Object.assign(a.style, { left: `${r.left}px`, top: `${r.top - 6}px` });
        document.body.append(a);
        reference(at, a);
        a.remove();
      });
    }
    add("image", "新增圖…", () => { figure(at); });
    add("tableStyle", "新增表…", () => { table(at); });
    if (d.text.trim()) {
      sep();
      add("star", "收藏句型", () => { PCWrite.collect(marker, d.text); keepMarker(); });
      sep();
      const row = el("div", "ctx-clip");
      menu.append(row);
      const clip = (label, fn) => {
        const b = el("button", "", label);
        b.setAttribute("role", "menuitem");
        b.onmousedown = (e) => e.preventDefault();
        b.onmouseenter = () => focusItem(items().indexOf(b));
        b.onclick = () => { hide(false); fn(); };
        row.append(b);
      };
      if (d.ta) clip("剪下", () => copy(d, true));
      clip("複製", () => copy(d, false));
    }
    document.body.append(menu);
    const w = menu.offsetWidth, h = menu.offsetHeight;
    menu.style.left = `${Math.max(8, x + w + 8 > innerWidth ? x - w : x)}px`;
    menu.style.top = `${Math.max(8, y + h + 8 > innerHeight ? y - h : y)}px`;
    document.addEventListener("keydown", keys, true);
    setTimeout(() => {
      document.addEventListener("mousedown", outside, true);
      addEventListener("blur", hideAll);
      addEventListener("resize", hideAll);
      $("#pageArea")?.addEventListener("scroll", hideAll);
    });
  }

  // The selection before the right button: WebKit selects the word under the
  // pointer before `contextmenu` fires.
  document.addEventListener("mousedown", (e) => {
    if (!(e.button === 2 || (MAC && e.button === 0 && e.ctrlKey))) return;
    down = capture(e.target);
  }, true);

  function capture(target) {
    const ta = target.closest?.("#paper > .para > textarea");
    if (ta) return { t: Date.now(), ta, a: ta.selectionStart, b: ta.selectionEnd, text: ta.value.slice(ta.selectionStart, ta.selectionEnd) };
    const s = getSelection();
    const text = s && !s.isCollapsed && $("#paper").contains(s.anchorNode) ? String(s) : "";
    return { t: Date.now(), text };
  }

  document.addEventListener("contextmenu", (e) => {
    if (e.altKey || e.shiftKey || !state.paper) return; // the system menu
    const ta = e.target.closest?.("#paper > .para > textarea");
    const p = ta ? ta.parentElement : e.target.closest?.("#paper > .para");
    if (!p || p.matches(".tbl, .add-para, .comment-only")) return;
    e.preventDefault();
    const d = down && Date.now() - down.t < 2000 ? down : capture(e.target);
    down = null;
    const si = +p.dataset.s, pi = +p.dataset.p;
    let at;
    if (ta) {
      // keep a real selection; a word WebKit selected on the click becomes a caret after it
      if (d.a !== d.b) ta.setSelectionRange(d.a, d.b);
      else if (ta.selectionStart !== ta.selectionEnd) ta.setSelectionRange(ta.selectionEnd, ta.selectionEnd);
      const caret = d.a !== d.b ? d.b : ta.selectionStart;
      at = { si, pi, nonWs: nonWsBefore(ta.value, caret), fromTa: true, sig: null };
      mark(null, e.clientX, e.clientY);
    } else {
      if (!d.text) getSelection()?.removeAllRanges();
      const src = sections()[si]?.paragraphs[pi] ?? "";
      const prose = !p.matches(".head, .display");
      const hit = prose ? locate(p, src, e.clientX, e.clientY, e.target) : { nonWs: null, rect: null };
      at = { si, pi, nonWs: hit.nonWs, sig: stripped(src) };
      // nowhere to map (a gap, a heading, a block): the end of the paragraph, before its full stop
      if (prose && at.nonWs == null) {
        const m = src.match(/[.!?]\s*$/);
        at.nonWs = nonWsBefore(src, m ? src.length - m[0].length : src.trimEnd().length);
      }
      mark(hit.rect, e.clientX, e.clientY);
      if (state.cur.s !== si || state.cur.p !== pi) select(si, pi);
    }
    show(e.clientX, e.clientY, d, at);
  });

  return { atoms, locate, isOpen: () => !!menu };
})();
window.PCContext = PCContext;
