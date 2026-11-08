// Table editor. A table paragraph is shown as a real table; click to select
// cells, double-click a cell to type, and use the toolbar to add or remove
// rows and columns, merge, draw rules and change the font size. The grid
// comes from the server's LaTeX parser and is written back as LaTeX after
// every change. Tables the parser cannot read are shown as code.

const PCTable = (() => {
  const TABLE_RE = /^\s*\\begin\{(table\*?|sidewaystable\*?|tabular)\}/;
  const cache = new Map(); // paragraph text → { table } | { error }
  const BOOKTABS = new Set(["toprule", "midrule", "bottomrule", "botrule", "cmidrule"]);
  const FULL = new Set(["hline", "toprule", "midrule", "bottomrule", "botrule"]);
  const SIZES = [["", "正常"], ["small", "small"], ["footnotesize", "footnotesize"], ["scriptsize", "scriptsize"]];
  let active = null; // { si, pi, t, sel, history, box }
  let styles = null; // built-in table styles
  let focusNext = null; // { si, pi }: activate this table once mounted
  const fits = new Map(); // table JSON → measurement result

  const isTable = (text) => TABLE_RE.test(text);
  const RESIZED_RE = /\\resizebox\*?\s*\{\s*\\(textwidth|linewidth|columnwidth|hsize)|\\begin\{adjustbox\}\s*\{[^}]*\bwidth\s*=/;

  async function parsed(text) {
    if (!cache.has(text)) {
      try { cache.set(text, await api("/api/textable/parse", { text })); }
      catch (e) { cache.set(text, { error: e.message }); }
    }
    return cache.get(text);
  }

  const clone = (t) => JSON.parse(JSON.stringify(t));
  const blank = () => ({ text: "", colspan: 1, rowspan: 1, covered: false, align: null });
  const booktabs = (t) => t.rules.flat().some((r) => BOOKTABS.has(r.kind));

  // ── mounting ──
  async function mount(p, si, pi, text) {
    p.classList.add("tbl");
    p.innerHTML = "";
    const r = await parsed(text);
    if (r.error) {
      p.classList.add("as-code");
      const pre = el("pre", "tex-code", text);
      const tag = el("span", "code-tag", "程式碼");
      tag.title = r.error;
      p.append(tag, pre);
      return;
    }
    let keep = active && active.si === si && active.pi === pi ? active : null;
    const wantFocus = focusNext && focusNext.si === si && focusNext.pi === pi;
    const box = { p, si, pi, t: keep ? keep.t : clone(r.table), sel: keep ? keep.sel : null, history: keep ? keep.history : [] };
    draw(box);
    p.onclick = (e) => { e.stopPropagation(); activate(box); };
    if (keep) { active = null; activate(box); }
    if (wantFocus) { focusNext = null; activate(box); p.scrollIntoView({ block: "center", behavior: "smooth" }); }
  }

  // ── styles ──
  async function loadStyles() {
    if (!styles) { try { styles = await api("/api/textable/styles"); } catch { styles = []; } }
    return styles;
  }

  /** A tiny table drawn with the style's lines, for the gallery. */
  function thumb(st) {
    const L = st.look;
    const box = el("div", `thumb rules-${L.rules}` + (L.env !== "table" ? ` env-${L.env.replace("*", "s")}` : ""));
    const cap = el("i", "cap");
    if (!L.caption_below) box.append(cap);
    const t = el("table");
    const cols = L.env === "table" ? 3 : 5;
    const sample = { results: ["", "b", "u"], ablation: ["✓", "✓", "×"], meanstd: ["±", "±", ""], decimal: [".", ".", "."] }[st.id];
    for (let r = 0; r < 4; r++) {
      const tr = el("tr");
      for (let c = 0; c < cols; c++) {
        const td = el("td");
        if (r === 0 && st.id === "multiheader" && c === 1) { td.colSpan = 2; c++; }
        const mark = r > 0 && c > 0 ? sample?.[(r + c) % 3] : "";
        if (mark === "b") td.className = "best"; else if (mark === "u") td.className = "second"; else if (mark) td.textContent = mark;
        tr.append(td);
      }
      if (st.id === "groups" && (r === 1)) tr.className = "grp";
      t.append(tr);
    }
    box.append(t);
    if (L.caption_below) box.append(cap);
    if (L.notes) box.append(el("i", "note"));
    return box;
  }

  /** Pick a style: a grouped gallery anchored to `anchor`. */
  async function gallery(anchor, current, onPick) {
    const list = await loadStyles();
    document.querySelector("#styleGallery")?.remove();
    const pop = el("div", "pop gallery");
    pop.id = "styleGallery";
    pop.onclick = (e) => e.stopPropagation();
    const groups = [...new Set(list.map((s) => s.group))];
    groups.forEach((g) => {
      pop.append(el("div", "g-title", g));
      const grid = el("div", "g-grid");
      list.filter((s) => s.group === g).forEach((st) => {
        const b = el("button", "tile");
        b.setAttribute("aria-current", String(st.id === current));
        b.title = st.from;
        b.append(thumb(st), el("span", "", st.name));
        b.onclick = () => { pop.remove(); onPick(st); };
        grid.append(b);
      });
      pop.append(grid);
    });
    document.body.append(pop);
    const r = anchor.getBoundingClientRect();
    const h = pop.offsetHeight, w = pop.offsetWidth;
    pop.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
    pop.style.top = `${r.top - h - 8 > 8 ? r.top - h - 8 : Math.min(r.bottom + 8, innerHeight - h - 8)}px`;
    const close = (e) => { if (!pop.contains(e.target)) { pop.remove(); document.removeEventListener("mousedown", close); } };
    setTimeout(() => document.addEventListener("mousedown", close));
  }

  async function needPackages(pkgs) {
    if (!pkgs?.length || !state.paper) return;
    try {
      const r = await api(`/api/papers/${encodeURIComponent(state.paper.name)}/packages`, { packages: pkgs });
      if (r.added.length) flash(`main.tex 已加入 ${r.added.map((p) => `\\usepackage{${p}}`).join(" ")}`, 5000);
    } catch (e) { flash(e.message, 6000); }
  }

  async function restyle(box, st) {
    let r;
    try { r = await api("/api/textable/apply", { table: box.t, style: st.id }); }
    catch (e) { flash(e.message, 6000); return; }
    await change(box, (t) => { Object.keys(t).forEach((k) => delete t[k]); Object.assign(t, r.table); });
    needPackages(r.packages);
  }

  /** Insert a new table after the current paragraph, chosen from the gallery. */
  async function insertNew(anchor) {
    const sec = curSection();
    if (!sec) return flash("先開啟一篇論文");
    gallery(anchor, store.get("pc.tableStyle", "booktabs"), async (st) => {
      store.set("pc.tableStyle", st.id);
      let r;
      try { r = await api("/api/textable/new", { style: st.id }); } catch (e) { flash(e.message, 6000); return; }
      const si = state.cur.s;
      const at = sec.paragraphs.length ? state.cur.p + 1 : 0;
      sec.paragraphs.splice(at, 0, r.text);
      cache.set(r.text, { table: r.table });
      try { await saveSection(si); } catch (e) { flash(`儲存失敗：${e.message}`); return; }
      await needPackages(r.packages);
      focusNext = { si, pi: at };
      state.cur = { s: si, p: at };
      render();
      refreshGit();
    });
  }

  // ── drawing ──
  function draw(box) {
    const { p, t } = box;
    p.innerHTML = "";
    const fig = el("figure", `tex-table size-${t.font || "normal"}`);
    if (t.env.startsWith("table*") || t.env.startsWith("sideways")) fig.classList.add("wide");
    const cap = () => {
      if (!t.caption && !t.label) return null;
      const c = el("figcaption");
      const n = t.label ? PCTex.labels[t.label] : null;
      c.append(el("b", "", n ? `Table ${n}. ` : "Table. "), PCTex.render(t.caption ?? ""));
      return c;
    };
    if (!t.caption_below) { const c = cap(); if (c) fig.append(c); }
    const table = el("table");
    const nrows = t.rows.length;
    t.rows.forEach((row, r) => {
      const tr = el("tr");
      row.forEach((cell, c) => {
        if (cell.covered) return;
        const td = el("td");
        if (cell.colspan > 1) td.colSpan = cell.colspan;
        if (cell.rowspan > 1) td.rowSpan = cell.rowspan;
        td.dataset.r = r; td.dataset.c = c;
        const align = (cell.align ?? t.cols[c] ?? "l").replace(/[@{}|]/g, "")[0];
        td.style.textAlign = align === "c" ? "center" : align === "r" ? "right" : "left";
        td.append(PCTex.render(cell.text));
        // rules: above this row, and below the last row
        const lineStyle = (rules, col) => {
          let w = 0;
          rules.forEach((x) => {
            const inRange = x.from == null || (col + 1 >= x.from && col + 1 <= x.to);
            if (!inRange) return;
            w = Math.max(w, x.kind === "toprule" || x.kind === "bottomrule" || x.kind === "botrule" ? 1.4 : 0.7);
          });
          return w ? `${w}px solid currentColor` : "";
        };
        td.style.borderTop = lineStyle(t.rules[r] ?? [], c);
        if (r + cell.rowspan >= nrows) td.style.borderBottom = lineStyle(t.rules[nrows] ?? [], c);
        if (t.vlines?.[c]) td.style.borderLeft = "0.7px solid currentColor";
        if (t.vlines?.[c + cell.colspan]) td.style.borderRight = "0.7px solid currentColor";
        if (inSel(box, r, c)) td.classList.add("sel");
        td.onmousedown = (e) => startSelect(box, r, c, e);
        td.onmouseenter = (e) => { if (e.buttons === 1 && box.dragging) extendSelect(box, r, c); };
        td.ondblclick = (e) => { e.stopPropagation(); editCell(box, td, r, c); };
        tr.append(td);
      });
      table.append(tr);
    });
    // \resizebox{\textwidth}{!}{…} / adjustbox width=…: LaTeX scales the table
    // to the text width, so the preview does too instead of running off the page.
    if (t.before.some((l) => RESIZED_RE.test(PCTex.stripComments(l)))) {
      const wrap = el("div", "tbl-scale");
      wrap.title = "LaTeX 會把這個表格縮放到版面寬度（\\resizebox）";
      wrap.append(table);
      fig.append(wrap);
      fitWidth(wrap, table);
    } else fig.append(table);
    if (t.caption_below) { const c = cap(); if (c) fig.append(c); }
    // Lines around the tabular: layout commands vanish, notes are shown.
    const notes = [...t.before, ...t.after].map((l) => PCTex.stripComments(l).trim())
      .filter((l) => l && !/^\\(begin|end)\{(center|flushleft|flushright|small|footnotesize|scriptsize|adjustbox|threeparttable|minipage)\}/.test(l))
      .filter((l) => !/^(\\(vspace\*?|hspace\*?|bigskip|medskip|smallskip|centering|small|footnotesize|scriptsize|tiny|normalsize|setlength|renewcommand|resizebox|scalebox|captionsetup|label)\b[^a-zA-Z]?[^]*|[{}]|\}\s*%?)$/.test(l) || /\\(footnote|tnote|item)/.test(l));
    if (notes.length) {
      const n = el("div", "tex-notes");
      n.append(PCTex.render(notes.join("\n")));
      if (n.textContent.trim()) fig.append(n);
    }
    p.append(fig);
    if (active === box) p.append(toolbar(box));
  }

  /** Scale `table` down to the width of `wrap` (kept in step when the page is resized or zoomed). */
  function fitWidth(wrap, table) {
    let lastW = -1;
    const apply = () => {
      const avail = wrap.clientWidth;
      if (!avail || avail === lastW && table.dataset.scaled) return;
      lastW = avail;
      table.style.transform = "";
      wrap.style.height = "";
      const w = table.offsetWidth;
      if (!w) return;
      const k = Math.min(1, avail / w);
      table.dataset.scaled = String(k);
      table.style.margin = k < 1 ? "0" : "0 auto";
      if (k < 1) {
        table.style.transform = `scale(${k})`;
        wrap.style.height = `${Math.ceil(table.offsetHeight * k)}px`;
      }
    };
    requestAnimationFrame(apply);
    if (window.ResizeObserver) new ResizeObserver(apply).observe(wrap);
  }

  // Selection and toolbar change without rebuilding the table, so a second
  // click (double-click) still lands on the same cell element.
  function paintSel(box) {
    box.p.querySelectorAll("td").forEach((td) => td.classList.toggle("sel", inSel(box, +td.dataset.r, +td.dataset.c)));
  }

  // ── selection ──
  function inSel(box, r, c) {
    const s = box.sel;
    return !!s && r >= Math.min(s.r0, s.r1) && r <= Math.max(s.r0, s.r1) && c >= Math.min(s.c0, s.c1) && c <= Math.max(s.c0, s.c1);
  }
  const rect = (s) => ({ r0: Math.min(s.r0, s.r1), r1: Math.max(s.r0, s.r1), c0: Math.min(s.c0, s.c1), c1: Math.max(s.c0, s.c1) });

  function startSelect(box, r, c, e) {
    if (e.target.isContentEditable) return;
    activate(box);
    if (e.shiftKey && box.sel) { box.sel.r1 = r; box.sel.c1 = c; }
    else box.sel = { r0: r, c0: c, r1: r + box.t.rows[r][c].rowspan - 1, c1: c + box.t.rows[r][c].colspan - 1 };
    box.dragging = true;
    document.addEventListener("mouseup", () => (box.dragging = false), { once: true });
    paintSel(box);
  }
  function extendSelect(box, r, c) {
    box.sel.r1 = r + box.t.rows[r][c].rowspan - 1;
    box.sel.c1 = c + box.t.rows[r][c].colspan - 1;
    paintSel(box);
  }

  function activate(box) {
    if (active === box) return;
    const prev = active;
    active = box;
    if (prev) { prev.sel = null; prev.p.querySelector(".tbl-bar")?.remove(); paintSel(prev); }
    if (!box.sel) box.sel = { r0: 0, c0: 0, r1: 0, c1: 0 };
    select(box.si, box.pi); // highlight in the navigator
    if (!box.p.querySelector(".tbl-bar")) box.p.append(toolbar(box));
    paintSel(box);
    checkFit(box);
  }

  // ── fitting the page: measured by LaTeX with the paper's class ──
  let fitTimer;
  function checkFit(box) {
    clearTimeout(fitTimer);
    fitTimer = setTimeout(async () => {
      if (!state.paper) return;
      const key = `${state.paper.name}\n${JSON.stringify(box.t)}`;
      if (!fits.has(key)) {
        try { fits.set(key, await api(`/api/papers/${encodeURIComponent(state.paper.name)}/table-fit`, { table: box.t })); }
        catch (e) { fits.set(key, { error: e.message }); }
      }
      if (active === box) showFit(box, fits.get(key));
    }, 500);
  }

  const SIZE_NAME = { normalsize: "正常", small: "small", footnotesize: "footnotesize", scriptsize: "scriptsize" };
  const ENV_NAME = { table: "單欄", "table*": "跨雙欄", sidewaystable: "橫式" };

  function showFit(box, r) {
    const slot = box.p.querySelector(".tbl-bar .fit-slot");
    if (!slot) return;
    slot.innerHTML = "";
    if (r.error) {
      const chip = el("span", "fit", "—");
      chip.title = r.error;
      slot.append(chip);
      return;
    }
    const a = r.advice;
    const pct = Math.round((a.current.width / a.current.avail) * 100);
    const chip = el("span", `fit ${a.ok ? (pct > 95 ? "tight" : "ok") : "bad"}`, a.ok ? `${pct}%` : a.reason);
    chip.title = `${a.ok ? "" : `${a.reason}：`}${ENV_NAME[a.current.env.startsWith("sideways") ? "sidewaystable" : a.current.env] ?? a.current.env}可用 ${a.current.avail.toFixed(0)}pt · 表格 ${a.current.width.toFixed(0)}pt（${SIZE_NAME[a.current.font]}）`;
    slot.append(chip);
    if (!a.ok && a.suggest) {
      const s = a.suggest;
      const label = [s.env !== a.current.env ? ENV_NAME[s.env.startsWith("sideways") ? "sidewaystable" : s.env] : "", s.font !== a.current.font ? SIZE_NAME[s.font] : ""].filter(Boolean).join(" + ");
      const fix = el("button", "ghost fit-fix", `改成${label}`);
      fix.onclick = async () => {
        await change(box, (t) => {
          t.font = s.font === "normalsize" ? null : s.font;
          if (s.env !== t.env) { t.env = s.env; if (s.env !== "table") t.placement = s.env.startsWith("sideways") ? null : "t"; }
        });
        if (s.env.startsWith("sideways")) needPackages(["rotating"]);
        checkFit(box);
      };
      slot.append(fix);
    }
  }
  function deactivate() {
    if (!active) return;
    const b = active;
    active = null;
    b.sel = null;
    b.p.querySelector(".tbl-bar")?.remove();
    paintSel(b);
  }

  // ── cell typing ──
  function editCell(box, td, r, c) {
    activate(box);
    const cell = box.t.rows[r][c];
    td.textContent = cell.text;
    td.contentEditable = "plaintext-only";
    td.classList.add("editing");
    td.focus();
    document.getSelection().selectAllChildren(td);
    const done = (move) => {
      td.onblur = null;
      td.contentEditable = "false";
      const v = td.textContent.replace(/\n/g, " ").trim();
      if (v !== cell.text) change(box, (t) => { t.rows[r][c].text = v; });
      else draw(box);
      if (move) {
        // next visible cell, wrapping to the next row
        const flat = [];
        box.t.rows.forEach((row, rr) => row.forEach((x, cc) => { if (!x.covered) flat.push([rr, cc]); }));
        const i = flat.findIndex(([rr, cc]) => rr === r && cc === c) + move;
        const next = flat[(i + flat.length) % flat.length];
        box.sel = { r0: next[0], c0: next[1], r1: next[0], c1: next[1] };
        draw(box);
        const ntd = box.p.querySelector(`td[data-r="${next[0]}"][data-c="${next[1]}"]`);
        if (ntd) editCell(box, ntd, next[0], next[1]);
      }
    };
    td.onkeydown = (e) => {
      if (e.key === "Tab") { e.preventDefault(); done(e.shiftKey ? -1 : 1); }
      else if (e.key === "Enter" || e.key === "Escape") { e.preventDefault(); done(0); }
      e.stopPropagation();
    };
    td.onblur = () => done(0);
  }

  // ── changes: every edit goes through here, then is saved as LaTeX ──
  async function change(box, fn) {
    box.history.push(JSON.stringify(box.t));
    if (box.history.length > 100) box.history.shift();
    fn(box.t);
    draw(box);
    await save(box);
    if (active === box) checkFit(box);
  }
  async function undo(box) {
    const prev = box.history.pop();
    if (!prev) return;
    box.t = JSON.parse(prev);
    draw(box);
    await save(box);
  }
  async function save(box) {
    let text;
    try { text = (await api("/api/textable/render", box.t)).text; }
    catch (e) { flash(`表格無法寫回：${e.message}`, 6000); return; }
    cache.set(text, { table: clone(box.t) });
    sections()[box.si].paragraphs[box.pi] = text;
    try { await saveSection(box.si); } catch (e) { flash(`儲存失敗：${e.message}`); }
    refreshGit();
  }

  // ── operations ──
  // Origin (top-left) of the span covering (r, c).
  function origin(t, r, c) {
    for (let rr = r; rr >= 0; rr--) {
      for (let cc = c; cc >= 0; cc--) {
        const x = t.rows[rr][cc];
        if (!x.covered && rr + x.rowspan > r && cc + x.colspan > c) return [rr, cc];
      }
    }
    return [r, c];
  }

  function insertRow(t, k) {
    const row = t.cols.map((_, c) => {
      const cell = blank();
      if (k > 0 && k < t.rows.length) {
        const [or, oc] = origin(t, k, c);
        if (or < k) { cell.covered = true; if (oc === c) t.rows[or][oc].rowspan++; }
      }
      return cell;
    });
    t.rows.splice(k, 0, row);
    t.gaps.splice(k, 0, null);
    t.rules.splice(k === t.rows.length - 1 ? k : k + 1, 0, []);
  }

  function deleteRows(t, r0, r1) {
    const n = r1 - r0 + 1;
    if (n >= t.rows.length) return flash("至少要留一列");
    t.cols.forEach((_, c) => {
      for (let r = 0; r < t.rows.length; r++) {
        const x = t.rows[r][c];
        if (x.covered || x.rowspan === 1) continue;
        const end = r + x.rowspan - 1;
        if (r < r0 && end >= r0) x.rowspan -= Math.min(end, r1) - r0 + 1;
        else if (r >= r0 && r <= r1 && end > r1) {
          // the span outlives the deleted rows: move its origin below them
          const moved = { ...x, rowspan: end - r1 };
          t.rows[r1 + 1][c] = moved;
          for (let k = 1; k < moved.colspan; k++) t.rows[r1 + 1][c + k] = { ...blank(), covered: true };
        }
      }
    });
    const atEnd = r1 === t.rows.length - 1;
    t.rows.splice(r0, n);
    t.gaps.splice(r0, n);
    t.rules.splice(atEnd ? r0 : r0 + 1, n);
  }

  function insertCol(t, k) {
    const ref = t.cols[Math.min(k, t.cols.length - 1)];
    t.cols.splice(k, 0, /^[lcr]$/.test(ref) ? ref : "c");
    t.vlines.splice(k === t.cols.length - 1 ? k : k + 1, 0, false);
    t.rows.forEach((row, r) => {
      const cell = blank();
      if (k > 0 && k < row.length) {
        const [or, oc] = origin(t, r, k);
        if (oc < k) { cell.covered = true; if (or === r) t.rows[or][oc].colspan++; }
      }
      row.splice(k, 0, cell);
    });
    const shift = (x) => (x != null && x >= k + 1 ? x + 1 : x);
    t.rules.forEach((rs) => rs.forEach((x) => { x.from = shift(x.from); x.to = shift(x.to); }));
  }

  function deleteCols(t, c0, c1) {
    const n = c1 - c0 + 1;
    if (n >= t.cols.length) return flash("至少要留一欄");
    t.rows.forEach((row) => {
      for (let c = 0; c < row.length; c++) {
        const x = row[c];
        if (x.covered || x.colspan === 1) continue;
        const end = c + x.colspan - 1;
        if (c < c0 && end >= c0) x.colspan -= Math.min(end, c1) - c0 + 1;
        else if (c >= c0 && c <= c1 && end > c1) row[c1 + 1] = { ...x, colspan: end - c1 };
      }
      row.splice(c0, n);
    });
    const atEnd = c1 === t.cols.length - 1;
    t.cols.splice(c0, n);
    t.vlines.splice(atEnd ? c0 : c0 + 1, n);
    // partial rules: drop deleted columns, shift the rest
    const map = (x) => (x > c1 + 1 ? x - n : Math.min(x, c0 + 1));
    t.rules.forEach((rs, i) => {
      t.rules[i] = rs.filter((x) => {
        if (x.from == null) return true;
        if (x.from >= c0 + 1 && x.to <= c1 + 1) return false;
        x.from = x.from > c1 + 1 ? x.from - n : x.from >= c0 + 1 ? c0 + 1 : x.from;
        x.to = map(x.to);
        return x.to >= x.from;
      });
    });
  }

  // Grow a rectangle until no span crosses its edge.
  function snap(t, s) {
    let { r0, r1, c0, c1 } = s;
    let grew = true;
    while (grew) {
      grew = false;
      for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) {
        const [or, oc] = origin(t, r, c);
        const x = t.rows[or][oc];
        const nr0 = Math.min(r0, or), nc0 = Math.min(c0, oc), nr1 = Math.max(r1, or + x.rowspan - 1), nc1 = Math.max(c1, oc + x.colspan - 1);
        if (nr0 !== r0 || nc0 !== c0 || nr1 !== r1 || nc1 !== c1) { r0 = nr0; c0 = nc0; r1 = nr1; c1 = nc1; grew = true; }
      }
    }
    return { r0, r1, c0, c1 };
  }

  function merge(t, s) {
    const { r0, r1, c0, c1 } = snap(t, s);
    if (r0 === r1 && c0 === c1) return flash("先拖曳選取要合併的格子");
    const texts = [];
    for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) {
      const x = t.rows[r][c];
      if (!x.covered && x.text.trim()) texts.push(x.text.trim());
      t.rows[r][c] = { ...blank(), covered: true };
    }
    t.rows[r0][c0] = { text: texts.join(" "), colspan: c1 - c0 + 1, rowspan: r1 - r0 + 1, covered: false, align: c1 > c0 ? "c" : null };
  }

  function split(t, s) {
    const { r0, r1, c0, c1 } = rect(s);
    let any = false;
    for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) {
      const x = t.rows[r][c];
      if (x.covered || (x.colspan === 1 && x.rowspan === 1)) continue;
      any = true;
      for (let rr = r; rr < r + x.rowspan; rr++) for (let cc = c; cc < c + x.colspan; cc++) t.rows[rr][cc] = blank();
      t.rows[r][c] = { ...blank(), text: x.text };
    }
    if (!any) flash("選取的格子沒有合併");
  }

  // Toggle a rule at boundary b across the selected columns.
  function toggleRule(t, b, c0, c1) {
    const rules = t.rules[b];
    const whole = c0 === 0 && c1 === t.cols.length - 1;
    if (whole) {
      const has = rules.some((x) => FULL.has(x.kind));
      if (has) { t.rules[b] = rules.filter((x) => !FULL.has(x.kind)); return; }
      const kind = !booktabs(t) ? "hline" : b === 0 ? "toprule" : b === t.rows.length ? (t.rules.flat().some((x) => x.kind === "botrule") ? "botrule" : "bottomrule") : "midrule";
      rules.push({ kind, from: null, to: null, trim: null });
      return;
    }
    const from = c0 + 1, to = c1 + 1;
    const i = rules.findIndex((x) => x.from === from && x.to === to);
    if (i >= 0) rules.splice(i, 1);
    else rules.push({ kind: booktabs(t) ? "cmidrule" : "cline", from, to, trim: booktabs(t) ? "lr" : null });
  }

  // ── toolbar ──
  function toolbar(box) {
    const bar = el("div", "tbl-bar");
    bar.onclick = (e) => e.stopPropagation();
    bar.onmousedown = (e) => e.preventDefault(); // keep cell focus and selection
    const s = () => rect(box.sel);
    const groups = [
      [["rowAbove", "上方插入列", () => change(box, (t) => insertRow(t, s().r0))],
       ["rowBelow", "下方插入列", () => change(box, (t) => insertRow(t, s().r1 + 1))],
       ["colLeft", "左側插入欄", () => change(box, (t) => insertCol(t, s().c0))],
       ["colRight", "右側插入欄", () => change(box, (t) => insertCol(t, s().c1 + 1))]],
      [["rowDelete", "刪除列", () => { const x = s(); change(box, (t) => deleteRows(t, x.r0, x.r1)); box.sel = { r0: Math.max(0, x.r0 - 1), c0: x.c0, r1: Math.max(0, x.r0 - 1), c1: x.c0 }; }],
       ["colDelete", "刪除欄", () => { const x = s(); change(box, (t) => deleteCols(t, x.c0, x.c1)); box.sel = { r0: x.r0, c0: Math.max(0, x.c0 - 1), r1: x.r0, c1: Math.max(0, x.c0 - 1) }; }]],
      [["merge", "合併", () => change(box, (t) => merge(t, s()))],
       ["split", "取消合併", () => change(box, (t) => split(t, s()))]],
      [["lineAbove", "上方加／減線", () => { const x = s(); change(box, (t) => toggleRule(t, x.r0, x.c0, x.c1)); }],
       ["lineBelow", "下方加／減線", () => { const x = s(); change(box, (t) => toggleRule(t, x.r1 + 1, x.c0, x.c1)); }]],
    ];
    groups.forEach((g) => {
      const wrap = el("span", "grp");
      g.forEach(([ic, label, fn]) => wrap.append(iconButton(ic, label, fn)));
      bar.append(wrap);
    });
    const size = el("select", "size");
    size.setAttribute("aria-label", "字體大小");
    SIZES.forEach(([v, l]) => size.append(new Option(l, v)));
    size.value = box.t.font ?? "";
    size.onchange = () => change(box, (t) => { t.font = size.value || null; });
    const sizeWrap = el("span", "grp");
    sizeWrap.append(icon("textSize"), size);
    bar.append(sizeWrap);
    const styleBtn = iconButton("tableStyle", "樣式", () => gallery(styleBtn, box.t.style, (st) => restyle(box, st)));
    const styleWrap = el("span", "grp");
    styleWrap.append(styleBtn);
    bar.prepend(styleWrap);
    const fitSlot = el("span", "grp fit-slot");
    bar.append(fitSlot);
    const tail = el("span", "grp end");
    tail.append(
      iconButton("undo", "復原 Ctrl+Z", () => undo(box)),
      iconButton("code", "以 LaTeX 編輯", () => { deactivate(); edit(box.p, box.si, box.pi); }),
    );
    bar.append(tail);
    return bar;
  }

  // Click elsewhere ends table editing; keys act on the active table.
  document.addEventListener("click", (e) => { if (active && !e.target.closest(".tbl, .hint, .tip, #styleGallery")) deactivate(); });
  document.addEventListener("keydown", (e) => {
    if (!active || e.target.isContentEditable || /INPUT|TEXTAREA|SELECT/.test(e.target.tagName)) return;
    if ((e.ctrlKey || e.metaKey) && e.key === "z") { e.preventDefault(); undo(active); }
    else if (e.key === "Escape") deactivate();
    else if (e.key === "Enter" && active.sel) {
      e.preventDefault();
      const { r0, c0 } = rect(active.sel);
      const [r, c] = origin(active.t, r0, c0);
      const td = active.p.querySelector(`td[data-r="${r}"][data-c="${c}"]`);
      if (td) editCell(active, td, r, c);
    }
  });

  return { isTable, mount, insertNew, ops: { insertRow, deleteRows, insertCol, deleteCols, merge, split, toggleRule } };
})();
