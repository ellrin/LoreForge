// 骨架: the skeleton-first draft (docs/long-paper.md §G, docs/skeleton-api.md).
//   left    sections with their progress
//   middle  the skeleton, always there: section → frame (a few paragraphs under one purpose) →
//           paragraph (what it says → points → text). Drag, Alt+↑↓, ＋ between, undo.
//   right   the margin: threads anchored to paragraphs, like review comments. The AI asks, the
//           author answers, the next question shows at once while the answer is folded into the
//           paragraph (its points grow in place).
// Edits are ops on stable ids, applied here first and then sent in order; the page never sends a
// whole skeleton. While something runs on the server the view is polled.
//   PCSkeleton.open(paper)              the dialog (made on first use)
//   PCSkeleton.mount(root, { call })    into any element (the demo page); call(path, body) → View
const PCSkeleton = (() => {
  const h = (tag, cls, text) => { const n = document.createElement(tag); if (cls) n.className = cls; if (text != null) n.textContent = text; return n; };
  const ico = (name, label, fn, cls = "ico small") => {
    const b = h("button", cls);
    b.type = "button";
    b.setAttribute("aria-label", label);
    if (typeof icon === "function") b.append(icon(name));
    if (fn) b.onclick = (e) => { e.stopPropagation(); fn(e); };
    return b;
  };
  const btn = (label, fn, cls = "btn small") => { const b = h("button", cls, label); b.type = "button"; b.onclick = (e) => { e.stopPropagation(); fn(e); }; return b; };
  const toast = (msg, opt) => (window.LoreShell?.toast ? LoreShell.toast(msg, opt) : console.warn(msg));

  const STATES = ["outline", "points", "text", "done"];
  const STATE_ZH = { outline: "骨架", points: "要點", text: "文字", done: "完成" };
  const KINDS = [["table", "表"], ["figure", "圖"], ["flow", "流程圖"], ["equation", "公式"], ["algorithm", "偽代碼"], ["finding", "發現"]];
  const TARGET_OF = { table: "table", figure: "figure", flow: "figure", equation: "equation" };
  const THREAD_ZH = { conflict: "不一致", choice: "取捨", missing: "要補", note: "你說", edit: "修補", refine: "細修" };
  const BUSY_ZH = { fill: "整理中", write: "寫作中", refine: "細修中", repair: "修補中" };
  const PRESETS = [["wording", "用字"], ["style", "我的風格"], ["check", "檢查"]];
  const SEC_ZH = { introduction: "前言", related: "相關研究", methods: "方法", results: "結果", discussion: "討論", conclusion: "結論" };

  const S = {
    root: null, call: null, view: null, err: null,
    tab: "skeleton",            // inventory | skeleton | style
    hints: true,                // the hint layer (roles, links, jumps)
    active: null,               // the thread the author opened (else view.next)
    openThreads: new Set(),     // finished threads shown in full
    folded: new Set(),          // collapsed frames
    editing: null,              // { id, field }
    drafts: {},                 // unsent text by key
    noteAt: null,               // anchor of the note being written
    moved: new Set(),           // paragraphs added or moved here (they show the reason line)
    grew: new Set(),            // keys that just appeared (one highlight)
    rose: new Set(),            // frames whose level just rose
    inflight: 0, chain: Promise.resolve(), tmp: 0, poll: null, drag: null, style: null, inline: false,
  };

  // ── the view ─────────────────────────────────────────
  const paras = (v = S.view) => (v?.sections ?? []).flatMap((s) => s.blocks.flatMap((b) => b.paras));
  const blocks = (v = S.view) => (v?.sections ?? []).flatMap((s) => s.blocks);
  const levelOf = (b) => (b.paras.length ? STATES[Math.min(...b.paras.map((p) => STATES.indexOf(p.state)))] : "outline");
  function find(id, v = S.view) {
    for (const s of v?.sections ?? []) {
      if (s.id === id) return { section: s };
      for (const [bi, b] of s.blocks.entries()) {
        if (b.id === id) return { section: s, block: b, at: bi };
        for (const [pi, p] of b.paras.entries()) if (p.id === id) return { section: s, block: b, para: p, at: pi };
      }
    }
    return null;
  }
  const threadsAt = (id) => (S.view?.threads ?? []).filter((t) => t.anchor === id);
  const nextThread = () => {
    const ts = S.view?.threads ?? [];
    const open = (t) => t && t.status === "open" && (t.kind === "conflict" || t.kind === "choice" || t.kind === "missing");
    return (open(ts.find((t) => t.id === S.active)) && S.active) || (open(ts.find((t) => t.id === S.view.next)) && S.view.next) || ts.find(open)?.id || null;
  };
  const working = () => !!S.view && (!!S.view.job || paras().some((p) => p.busy) || S.view.threads.some((t) => t.status === "answered"));

  function setView(v, force) {
    if (!v || typeof v !== "object") return;
    if (!force && S.view && v.rev < S.view.rev) return;
    const old = S.view;
    if (old?.exists && v.exists) {
      const before = new Map(paras(old).map((p) => [p.id, p]));
      for (const p of paras(v)) {
        const o = before.get(p.id);
        if (!o) continue;
        const had = new Set(o.points.map((x) => x.text));
        p.points.forEach((x) => { if (!had.has(x.text)) S.grew.add(`pt:${p.id}:${x.text}`); });
        if (!o.text && p.text) S.grew.add(`tx:${p.id}`);
      }
      const lv = new Map(blocks(old).map((b) => [b.id, STATES.indexOf(levelOf(b))]));
      for (const b of blocks(v)) if (lv.has(b.id) && STATES.indexOf(levelOf(b)) > lv.get(b.id)) S.rose.add(b.id);
      if (old.job && !v.job && old.job.kind === "plan") S.tab = "skeleton";
    }
    if (v.job?.error && v.job.error !== old?.job?.error) toast(v.job.error, { err: true });
    S.view = v;
    if (!v.exists && S.tab === "skeleton" && !v.can_convert) S.tab = "inventory";
    draw();
    watch();
  }

  function watch() {
    clearTimeout(S.poll);
    if (!working() || !S.root?.isConnected) return;
    S.poll = setTimeout(async () => {
      if (!S.inflight && !S.drag) { try { setView(await S.call("")); return; } catch { /* next time */ } }
      watch();
    }, 800);
  }

  /** A call that answers with the view. */
  async function run(path, body, label) {
    try { setView(await S.call(path, body ?? {}), true); }
    catch (e) { toast(label ? `${label}：${e.message}` : e.message, { err: true }); try { setView(await S.call(""), true); } catch { /* keep */ } }
  }

  // ── ops: here first, then the server, in order ───────
  function local(o) {
    const v = S.view;
    if (o.op === "edit") {
      const f = find(o.id);
      Object.assign(f?.para ?? f?.block ?? {}, o.fields);
      if (f?.para) f.para.touched = true;
    } else if (o.op === "reason") { const p = find(o.id)?.para; if (p) p.reason = o.reason; }
    else if (o.op === "done") { const p = find(o.id)?.para; if (p && p.text) p.state = o.done ? "done" : "text"; }
    else if (o.op === "remove") {
      const f = find(o.id);
      if (f?.para) f.block.paras.splice(f.at, 1);
      else if (f?.block) f.section.blocks.splice(f.at, 1);
    } else if (o.op === "move") {
      const f = find(o.id);
      const to = find(o.to);
      if (!f || !to) return;
      if (f.para && to.block) {
        f.block.paras.splice(f.at, 1);
        to.block.paras.splice(Math.min(o.at, to.block.paras.length), 0, f.para);
        f.para.touched = true;
        S.moved.add(o.id);
        if (o.reason != null) f.para.reason = o.reason;
      } else if (f.block && !f.para && to.section) {
        f.section.blocks.splice(f.at, 1);
        to.section.blocks.splice(Math.min(o.at, to.section.blocks.length), 0, f.block);
      }
    } else if (o.op === "add_para") {
      const b = find(o.block)?.block;
      b?.paras.splice(o.at, 0, { id: o.tmp, kind: "", say: "", points: [], link_prev: "", link_next: "", jump: null, floats: [], cites: [], sources: [], pending: [], text: "", state: "outline", busy: null, problems: [], reason: "", touched: true });
    } else if (o.op === "add_block") {
      const s = find(o.section)?.section;
      s?.blocks.splice(o.at, 0, { id: o.tmp, title: "", purpose: "", of: null, paras: [] });
    }
    v.dirty = true;
    v.can_undo = true;
  }

  function send(o) {
    const { tmp, ...wire } = o;
    S.inflight++;
    const p = S.chain.then(() => S.call("/ops", { ops: [wire] }));
    S.chain = p.then(() => {}, () => {});
    return p.then(
      (v) => { S.inflight--; if (!S.inflight) setView(v, true); return v; },
      (e) => { S.inflight--; toast(e.message, { err: true }); if (!S.inflight) S.call("").then((v) => setView(v, true), () => {}); },
    );
  }

  /** Apply an op here and send it. Moves and removals glide (FLIP). */
  function op(o) {
    const first = rects();
    if (o.op !== "undo") local(o);
    draw();
    glide(first);
    return send(o);
  }

  async function add(kind, where) {
    const tmp = `tmp${++S.tmp}`;
    const o = kind === "para" ? { op: "add_para", block: where.block, at: where.at, say: "", reason: "", tmp } : { op: "add_block", section: where.section, at: where.at, title: "", of: null, tmp };
    S.editing = { id: tmp, field: kind === "para" ? "say" : "title" };
    const first = rects();
    local(o);
    draw();
    glide(first);
    const typed = () => S.drafts[`${S.editing?.field}:${tmp}`];
    const v = await send(o);
    // the server's id for it: the one now at that place
    const real = kind === "para" ? find(where.block, v)?.block?.paras[where.at]?.id : find(where.section, v)?.section?.blocks[where.at]?.id;
    if (!real) return;
    const f = kind === "para" ? "say" : "title";
    if (kind === "para" && S.pendingCommit?.id === tmp) S.moved.add(real);
    if (S.pendingCommit?.id === tmp) { const c = S.pendingCommit; S.pendingCommit = null; if (c.value) op({ op: "edit", id: real, fields: { [f]: c.value } }); return; }
    if (kind === "para") S.moved.add(real);
    const keep = typed();
    S.editing = { id: real, field: f };
    if (keep != null) { S.drafts[`${f}:${real}`] = keep; delete S.drafts[`${f}:${tmp}`]; }
    draw();
  }

  function commit(id, field, value) {
    const key = `${field}:${id}`;
    delete S.drafts[key];
    if (S.editing?.id === id && S.editing.field === field) S.editing = null;
    value = value.trim();
    if (id.startsWith("tmp")) { S.pendingCommit = { id, value }; const f = find(id); Object.assign(f?.para ?? f?.block ?? {}, { [field]: value }); draw(); return; }
    const f = find(id);
    const cur = (f?.para ?? f?.block)?.[field] ?? "";
    if (field === "reason") { if (value !== cur) op({ op: "reason", id, reason: value }); else draw(); return; }
    if (value !== cur) op({ op: "edit", id, fields: { [field]: value } }); else draw();
  }

  function remove(id) {
    const f = find(id);
    if (!f) return;
    const what = f.para ? "段落" : "框";
    op({ op: "remove", id });
    toast(`已刪除${what}`, { action: { label: "復原", run: () => op({ op: "undo" }) }, ms: 6000 });
  }

  function shift(id, d) {
    const f = find(id);
    if (!f) return;
    if (f.para) {
      const all = blocks();
      const bi = all.indexOf(f.block);
      if (f.at + d >= 0 && f.at + d < f.block.paras.length) op({ op: "move", id, to: f.block.id, at: f.at + d });
      else if (all[bi + d]) op({ op: "move", id, to: all[bi + d].id, at: d < 0 ? all[bi + d].paras.length : 0 });
    } else if (f.block) {
      const secs = S.view.sections;
      const si = secs.indexOf(f.section);
      if (f.at + d >= 0 && f.at + d < f.section.blocks.length) op({ op: "move", id, to: f.section.id, at: f.at + d });
      else if (secs[si + d]) op({ op: "move", id, to: secs[si + d].id, at: d < 0 ? secs[si + d].blocks.length : 0 });
    }
    requestAnimationFrame(() => S.root.querySelector(`[data-id="${id}"]`)?.focus({ preventScroll: true }));
  }

  // ── motion ───────────────────────────────────────────
  const rects = () => new Map([...(S.root?.querySelectorAll(".sk-canvas [data-id]") ?? [])].map((n) => [n.dataset.id, n.getBoundingClientRect()]));
  function glide(first) {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    S.root.querySelectorAll(".sk-canvas [data-id]").forEach((n) => {
      const a = first.get(n.dataset.id);
      if (!a) { n.animate([{ opacity: 0, transform: "scale(.98)" }, { opacity: 1, transform: "none" }], { duration: 180, easing: "ease-out" }); return; }
      const b = n.getBoundingClientRect();
      const dy = a.top - b.top;
      if (Math.abs(dy) > 1) n.animate([{ transform: `translateY(${dy}px)` }, { transform: "none" }], { duration: 200, easing: "cubic-bezier(.2,.8,.2,1)" });
    });
  }

  // ── text ─────────────────────────────────────────────
  /** LaTeX prose as it reads: refs and cites as marks, math through KaTeX when it is loaded. */
  function prose(src) {
    const out = document.createDocumentFragment();
    const re = /\$([^$]+)\$|\\(ref|cite[tp]?|eqref)\{([^}]*)\}|\\pending\{([^}]*)\}\{([^}]*)\}|\\(?:emph|textit|textbf)\{([^}]*)\}|\\([%&#_$])|~/g;
    let at = 0;
    for (let m; (m = re.exec(src)); ) {
      if (m.index > at) out.append(src.slice(at, m.index));
      at = re.lastIndex;
      if (m[1] != null) {
        const s = h("span", "sk-math");
        if (window.katex) { try { katex.render(m[1], s, { throwOnError: false }); } catch { s.textContent = m[1]; } } else s.textContent = m[1];
        out.append(s);
      } else if (m[2]) out.append(h("span", m[2] === "ref" || m[2] === "eqref" ? "sk-ref" : "sk-cite", m[3]));
      else if (m[4] != null) { const s = h("span", "sk-pend", m[5]); s.title = m[4]; out.append(s); }
      else if (m[6] != null) out.append(h("em", "", m[6]));
      else if (m[7]) out.append(m[7]);
      else out.append(" ");
    }
    if (at < src.length) out.append(src.slice(at));
    return out;
  }

  /** A field that turns into an editor in place. */
  function field(id, name, value, cls, placeholder, multiline) {
    const key = `${name}:${id}`;
    if (S.editing?.id === id && S.editing.field === name) {
      const t = h("textarea", `sk-edit ${cls}`);
      t.rows = 1;
      t.dataset.k = key;
      t.placeholder = placeholder;
      t.value = S.drafts[key] ?? value ?? "";
      const fit = () => { t.style.height = "auto"; t.style.height = `${t.scrollHeight}px`; };
      t.oninput = () => { S.drafts[key] = t.value; fit(); };
      t.onkeydown = (e) => {
        e.stopPropagation();
        if (e.key === "Enter" && !e.isComposing && !(multiline && e.shiftKey)) { e.preventDefault(); commit(id, name, t.value); }
        else if (e.key === "Escape") { e.preventDefault(); delete S.drafts[key]; S.editing = null; if (id.startsWith("tmp") && !t.value.trim()) { /* an empty new one stays, to be named later */ } draw(); }
      };
      t.onblur = () => { if (S.editing?.id === id && S.editing.field === name && !S.redrawing) commit(id, name, t.value); };
      t.onclick = (e) => e.stopPropagation();
      requestAnimationFrame(fit);
      return t;
    }
    const d = h("div", `${cls}${value ? "" : " sk-empty"}`, value || placeholder);
    d.onclick = (e) => { e.stopPropagation(); S.editing = { id, field: name }; draw(); };
    return d;
  }

  // ── the skeleton ─────────────────────────────────────
  const dot = (state, busy) => { const d = h("i", "sk-dot"); d.dataset.s = state; if (busy) d.dataset.busy = "1"; return d; };

  function paraNode(p, n, b) {
    const li = h("article", "sk-para");
    li.dataset.id = p.id;
    li.dataset.s = p.state;
    li.tabIndex = 0;
    const ts = threadsAt(p.id);
    const act = nextThread();
    if (ts.some((t) => t.id === act) || S.noteAt === p.id) li.classList.add("sk-anchor");
    if (p.busy) li.dataset.busy = p.busy;

    const head = h("header", "sk-phead");
    const grip = h("span", "sk-grip");
    grip.setAttribute("aria-hidden", "true");
    grip.onpointerdown = (e) => dragStart(e, p.id, "para");
    head.append(grip, dot(p.state, p.busy), h("span", "sk-n", `¶${n}`));
    if (p.kind) head.append(h("span", "sk-kind", p.kind));
    if (p.busy) head.append(h("span", "sk-busy", BUSY_ZH[p.busy] ?? p.busy));
    const tools = h("span", "sk-tools");
    if (!p.id.startsWith("tmp")) {
      tools.append(ico("comment", "對這段說一句", () => { S.noteAt = p.id; draw(); }));
      tools.append(ico("pen", p.text ? "重寫文字" : "寫文字", () => run("/write", { id: p.id }, "寫文字")));
      if (p.text) {
        const pol = ico("polish", "細修", (e) => menu(e.currentTarget, PRESETS.map(([id, label]) => [label, () => run("/refine", { para: p.id, preset: id }, label)])));
        tools.append(pol, ico("check", p.state === "done" ? "取消完成" : "這段完成", () => op({ op: "done", id: p.id, done: p.state !== "done" })));
        if (p.state === "done") tools.lastChild.setAttribute("aria-pressed", "true");
      }
      tools.append(ico("trash", "刪除", () => remove(p.id)));
    }
    head.append(tools);
    li.append(head);

    const hasText = !!p.text;
    // once there is text, what the paragraph is for stays as a caption above it (the hint layer)
    if (!hasText || S.hints) li.append(field(p.id, "say", p.say, hasText ? "sk-say sk-cap" : "sk-say", "這段要說什麼"));
    if (S.hints && (p.link_prev || p.link_next || p.jump)) {
      const l = h("div", "sk-links");
      if (p.link_prev) l.append(h("span", "sk-link up", p.link_prev));
      if (p.link_next) l.append(h("span", "sk-link down", p.link_next));
      if (p.jump) {
        const j = h("button", "sk-jump", "跳");
        j.type = "button";
        j.setAttribute("aria-expanded", S.openThreads.has(`jump:${p.id}`));
        j.onclick = (e) => { e.stopPropagation(); toggle(S.openThreads, `jump:${p.id}`); draw(); };
        l.append(j);
        if (S.openThreads.has(`jump:${p.id}`)) { const w = h("div", "sk-jumpwhy"); w.append(h("span", "", p.jump.why), h("b", "", p.jump.fix)); l.append(w); }
      }
      li.append(l);
    }
    if (p.points.length && !hasText) {
      const ul = h("ul", "sk-points");
      for (const x of p.points) {
        const i = h("li", "", x.text);
        if (x.from) i.dataset.from = x.from;
        const k = `pt:${p.id}:${x.text}`;
        if (S.grew.delete(k)) i.classList.add("sk-grew");
        ul.append(i);
      }
      li.append(ul);
    }
    if (hasText) {
      if (S.editing?.id === p.id && S.editing.field === "text") li.append(field(p.id, "text", p.text, "sk-text", "", true));
      else {
        const t = h("div", "sk-text");
        t.append(prose(p.text));
        if (S.grew.delete(`tx:${p.id}`)) t.classList.add("sk-grew");
        t.ondblclick = (e) => { e.stopPropagation(); S.editing = { id: p.id, field: "text" }; draw(); };
        li.append(t);
      }
      for (const x of p.problems) li.append(h("p", "sk-problem", x));
    }
    const chips = [...p.floats.map((x) => ["sk-ref", x]), ...p.cites.map((x) => ["sk-cite", x]), ...p.pending.map((x) => ["sk-pend", x])];
    if (chips.length && !hasText) { const c = h("div", "sk-chips"); chips.forEach(([cls, x]) => c.append(h("span", cls, x))); li.append(c); }
    // a paragraph the author added or moved: one optional line of why, read at the next 暫存
    if (p.reason || (p.touched && S.moved.has(p.id))) li.append(field(p.id, "reason", p.reason, "sk-reason", "理由（可空）"));
    if (S.inline) for (const t of ts) li.append(threadNode(t));
    if (S.inline && S.noteAt === p.id) li.append(noteNode(p.id));

    li.onkeydown = (e) => {
      if (e.target !== li) return;
      if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) { e.preventDefault(); shift(p.id, e.key === "ArrowUp" ? -1 : 1); }
      else if (e.key === "Enter") { e.preventDefault(); S.editing = { id: p.id, field: hasText ? "text" : "say" }; draw(); }
      else if (e.key === "Backspace" || e.key === "Delete") { e.preventDefault(); remove(p.id); }
      else if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); const all = [...S.root.querySelectorAll(".sk-para")]; all[all.indexOf(li) + (e.key === "ArrowDown" ? 1 : -1)]?.focus(); }
    };
    return li;
  }

  const gap = (fn, label) => { const g = h("button", "sk-gap"); g.type = "button"; g.setAttribute("aria-label", label); g.append(h("span", "", "＋")); g.onclick = (e) => { e.stopPropagation(); fn(); }; return g; };

  function blockNode(b, s, count) {
    const level = levelOf(b);
    const box = h("section", "sk-block");
    box.dataset.id = b.id;
    box.dataset.s = level;
    if (S.folded.has(b.id)) box.dataset.folded = "1";
    if (S.rose.delete(b.id)) box.classList.add("sk-rose");
    const head = h("header", "sk-bhead");
    const grip = h("span", "sk-grip");
    grip.onpointerdown = (e) => dragStart(e, b.id, "block");
    const mark = h("span", "sk-level");
    mark.dataset.s = level;
    if (level === "done" && typeof icon === "function") mark.append(icon("check"));
    const titles = h("div", "sk-btitles");
    titles.append(field(b.id, "title", b.title, "sk-btitle", "這幾段要做什麼"));
    if (b.of) { const o = find(b.of); const name = o?.block?.title || o?.para?.say; if (name) titles.append(h("span", "sk-of", `展開「${name}」`)); }
    const dots = h("span", "sk-dots");
    b.paras.forEach((p) => dots.append(dot(p.state, p.busy)));
    const tools = h("span", "sk-tools");
    if (!b.id.startsWith("tmp")) {
      tools.append(ico("comment", "對這個框說一句", () => { S.noteAt = b.id; draw(); }));
      if (b.paras.length) tools.append(ico("pen", "寫這個框的文字", () => run("/write", { id: b.id }, "寫文字")));
      tools.append(ico("trash", "刪除這個框", () => remove(b.id)));
    }
    const fold = ico("chevron", S.folded.has(b.id) ? "展開" : "摺起來", () => { toggle(S.folded, b.id); const first = rects(); draw(); glide(first); }, "ico small sk-fold");
    head.append(grip, mark, titles, dots, tools, fold);
    box.append(head);
    if (S.folded.has(b.id)) return box;
    if (b.purpose || S.editing?.id === b.id) box.append(field(b.id, "purpose", b.purpose, "sk-purpose", "讀者看完這幾段會知道什麼"));
    const list = h("div", "sk-paras");
    list.dataset.block = b.id;
    list.append(gap(() => add("para", { block: b.id, at: 0 }), "在這裡加一段"));
    b.paras.forEach((p, i) => { list.append(paraNode(p, count.n++, b), gap(() => add("para", { block: b.id, at: i + 1 }), "在這裡加一段")); });
    box.append(list);
    if (S.inline) for (const t of threadsAt(b.id)) box.append(threadNode(t));
    return box;
  }

  function canvas() {
    const c = h("div", "sk-canvas");
    const count = { n: 1 };
    for (const s of S.view.sections) {
      const sec = h("section", "sk-section");
      sec.dataset.id = s.id;
      sec.dataset.sec = s.id;
      const hd = h("h3", "sk-stitle", s.title);
      sec.append(hd);
      const list = h("div", "sk-blocks");
      list.dataset.section = s.id;
      s.blocks.forEach((b) => list.append(blockNode(b, s, count)));
      if (S.folded.size) for (const b of s.blocks) if (S.folded.has(b.id)) count.n += b.paras.length;
      sec.append(list, btn("＋ 框", () => add("block", { section: s.id, at: s.blocks.length }), "ghost sk-addblock"));
      c.append(sec);
    }
    return c;
  }

  // ── the margin ───────────────────────────────────────
  const toggle = (set, k) => (set.has(k) ? set.delete(k) : set.add(k));

  function changeNode(t, c) {
    const d = h("div", "sk-change");
    d.dataset.s = c.state;
    if (c.before) d.append(h("del", "", c.before));
    d.append(h("ins", "", c.after));
    if (c.why) d.append(h("p", "sk-why", c.why));
    const acts = h("div", "sk-acts");
    const send = (accept) => run("/change", { thread: t.id, change: c.id, accept });
    if (c.state === "proposed") acts.append(btn("採用", () => send(true), "btn small primary"), btn("不採用", () => send(false), "ghost"));
    else if (c.state === "applied" && t.kind === "edit") acts.append(btn("還原", () => send(false), "ghost"));
    else acts.append(h("span", "sk-cstate", { applied: "已採用", reverted: "已還原", rejected: "不採用" }[c.state] ?? ""));
    d.append(acts);
    return d;
  }

  function answer(t, text, picked) {
    text = (text ?? "").trim();
    if (!text && !picked.length) return;
    const said = text || t.options.filter((o) => picked.includes(o.id)).map((o) => o.text).join("；");
    delete S.drafts[`ans:${t.id}`];
    t.status = "answered";
    t.messages.push({ who: "me", text: said });
    const p = find(t.anchor)?.para;
    if (p) p.busy = "fill";
    S.active = null;
    S.view.next = null;
    draw();
    reveal(nextThread());
    run("/answer", { thread: t.id, text, picked, skip: false }, "回答");
  }

  function threadNode(t) {
    const act = nextThread() === t.id;
    const openQ = t.status === "open" && t.kind !== "edit" && t.kind !== "refine";
    const full = act || S.openThreads.has(t.id) || (t.kind === "refine" && t.changes.some((c) => c.state === "proposed"));
    const d = h("div", "sk-thread");
    d.dataset.thread = t.id;
    d.dataset.anchor = t.anchor;
    d.dataset.kind = t.kind;
    d.dataset.s = t.status;
    if (act) d.classList.add("sk-active");
    const tag = h("span", "sk-tag", THREAD_ZH[t.kind] ?? t.kind);
    if (!full) {
      // one line: what it is and where it stands
      const line = h("button", "sk-line");
      line.type = "button";
      const txt = t.status === "answered" ? "整理中" : t.status === "skipped" ? "略過" : t.conclusion || t.question || t.changes[0]?.why || "";
      line.append(tag, h("span", "sk-ltext", txt));
      line.onclick = () => { if (openQ) S.active = t.id; else toggle(S.openThreads, t.id); draw(); reveal(t.id); };
      d.append(line);
      return d;
    }
    const top = h("div", "sk-thead");
    top.append(tag);
    if (!act && S.openThreads.has(t.id)) top.append(ico("x", "收起來", () => { S.openThreads.delete(t.id); draw(); }, "ico tiny"));
    d.append(top);
    if (openQ) {
      d.append(h("p", "sk-q", t.question));
      const key = `ans:${t.id}`;
      const ta = h("textarea", "sk-answer");
      ta.rows = 2;
      ta.dataset.k = key;
      ta.placeholder = t.options.length ? "都不是，我自己說" : "回答";
      ta.value = S.drafts[key] ?? "";
      const opts = h("div", "sk-opts");
      t.options.forEach((o, i) => {
        const b = h("button", "sk-opt");
        b.type = "button";
        b.append(h("kbd", "", String(i + 1)), h("span", "", o.text));
        // a click answers with it; Shift-click puts it in the box to change first
        b.onclick = (e) => { if (e.shiftKey) { S.drafts[key] = o.text; draw(); } else answer(t, "", [o.id]); };
        opts.append(b);
      });
      ta.oninput = () => { S.drafts[key] = ta.value; };
      ta.onkeydown = (e) => {
        e.stopPropagation();
        if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); answer(t, ta.value, []); }
        else if (!ta.value && /^[1-9]$/.test(e.key) && t.options[e.key - 1]) { e.preventDefault(); answer(t, "", [t.options[e.key - 1].id]); }
      };
      const row = h("div", "sk-acts");
      row.append(btn("略過", () => { t.status = "skipped"; S.active = null; S.view.next = null; draw(); reveal(nextThread()); run("/answer", { thread: t.id, text: "", picked: [], skip: true }); }, "ghost"), h("span", "sk-grow"), ico("send", "送出 Enter", () => answer(t, ta.value, []), "ico small sk-send"));
      d.append(opts, ta, row);
      return d;
    }
    for (const m of t.messages) d.append(h("p", `sk-msg ${m.who}`, m.text));
    if (t.conclusion) d.append(h("p", "sk-concl", t.conclusion));
    for (const c of t.changes) d.append(changeNode(t, c));
    return d;
  }

  function noteNode(anchor) {
    const d = h("div", "sk-thread sk-active");
    d.dataset.thread = `note:${anchor}`;
    d.dataset.anchor = anchor;
    d.dataset.kind = "note";
    const key = `note:${anchor}`;
    const ta = h("textarea", "sk-answer");
    ta.rows = 2;
    ta.dataset.k = key;
    ta.placeholder = "想補充或想改的";
    ta.value = S.drafts[key] ?? "";
    ta.oninput = () => { S.drafts[key] = ta.value; };
    const close = () => { S.noteAt = null; delete S.drafts[key]; draw(); };
    const go = () => { const text = ta.value.trim(); if (!text) return; S.noteAt = null; delete S.drafts[key]; const p = find(anchor)?.para; if (p) p.busy = "fill"; draw(); run("/note", { anchor, text }, "補充"); };
    ta.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); go(); } else if (e.key === "Escape") { e.preventDefault(); close(); } };
    const row = h("div", "sk-acts");
    row.append(btn("取消", close, "ghost"), h("span", "sk-grow"), ico("send", "送出 Enter", go, "ico small sk-send"));
    const top = h("div", "sk-thead");
    top.append(h("span", "sk-tag", THREAD_ZH.note));
    d.append(top, ta, row);
    requestAnimationFrame(() => { if (document.activeElement?.dataset?.k !== key) ta.focus({ preventScroll: true }); });
    return d;
  }

  function margin() {
    const m = h("aside", "sk-margin");
    for (const t of S.view.threads) m.append(threadNode(t));
    if (S.noteAt) m.append(noteNode(S.noteAt));
    return m;
  }

  /** Each thread beside its paragraph; the one being answered sits exactly there, the others make room. */
  function place() {
    const m = S.root.querySelector(".sk-margin");
    const scroll = S.root.querySelector(".sk-scroll");
    if (!m || !scroll || S.inline) return;
    const base = m.getBoundingClientRect().top;
    const cards = [...m.children].map((n) => {
      let a = S.root.querySelector(`.sk-canvas [data-id="${n.dataset.anchor}"]`);
      // a folded frame: its paragraphs' threads gather at the frame
      if (!a) { const b = find(n.dataset.anchor)?.block; a = b && S.root.querySelector(`.sk-canvas [data-id="${b.id}"]`); }
      return { n, want: a ? a.getBoundingClientRect().top - base : 0, hgt: n.offsetHeight, act: n.classList.contains("sk-active"), gone: !a };
    });
    cards.forEach((c) => (c.n.hidden = c.gone));
    const live = cards.filter((c) => !c.gone).sort((a, b) => a.want - b.want);
    const act = live.find((c) => c.act);
    let y = 0;
    for (const c of live) {
      let top = Math.max(c.want, y);
      if (act && c !== act && top < act.want + act.hgt + 8 && top + c.hgt + 8 > act.want) top = act.want + act.hgt + 8;
      if (c === act) top = c.want;
      c.n.style.top = `${top}px`;
      if (c !== act || top + c.hgt > y) y = Math.max(y, top + c.hgt + 8);
    }
    m.style.minHeight = `${y}px`;
  }

  function reveal(threadId) {
    if (!threadId) return;
    requestAnimationFrame(() => {
      const t = (S.view.threads.find((x) => x.id === threadId) ?? {}).anchor;
      const a = t && S.root.querySelector(`.sk-canvas [data-id="${t}"]`);
      const sc = S.root.querySelector(".sk-scroll");
      if (!a || !sc) return;
      const r = a.getBoundingClientRect();
      const box = sc.getBoundingClientRect();
      if (r.top < box.top + 40 || r.bottom > box.bottom - 120) sc.scrollTo({ top: sc.scrollTop + r.top - box.top - 90, behavior: "smooth" });
      S.root.querySelector(`.sk-thread[data-thread="${threadId}"] .sk-answer`)?.focus({ preventScroll: true });
    });
  }

  // ── dragging (pointer events: smooth in the desktop web view, where HTML drag is not) ──
  function dragStart(e, id, kind) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const src = S.root.querySelector(`.sk-canvas [data-id="${id}"]`);
    const r = src.getBoundingClientRect();
    const d = { id, kind, src, x0: e.clientX, y0: e.clientY, dx: e.clientX - r.left, dy: e.clientY - r.top, on: false, to: null };
    S.drag = d;
    const sc = S.root.querySelector(".sk-scroll");
    const move = (ev) => {
      if (!d.on) {
        if (Math.hypot(ev.clientX - d.x0, ev.clientY - d.y0) < 4) return;
        d.on = true;
        d.ghost = src.cloneNode(true);
        d.ghost.classList.add("sk-ghost");
        d.ghost.style.width = `${r.width}px`;
        d.ghost.querySelectorAll("[data-k]").forEach((n) => n.removeAttribute("data-k"));
        S.root.append(d.ghost);
        d.bar = h("div", "sk-dropbar");
        S.root.append(d.bar);
        src.classList.add("sk-dragging");
        S.root.classList.add("sk-dragmode");
      }
      d.ghost.style.transform = `translate(${ev.clientX - d.dx}px, ${ev.clientY - d.dy}px)`;
      d.y = ev.clientY;
      target(ev.clientX, ev.clientY);
    };
    const target = (x, y) => {
      const lists = [...S.root.querySelectorAll(kind === "para" ? ".sk-paras" : ".sk-blocks")];
      let best = null;
      for (const l of lists) {
        const lr = l.getBoundingClientRect();
        const items = [...l.children].filter((n) => n.matches(kind === "para" ? ".sk-para" : ".sk-block") && n !== src);
        const dist = y < lr.top ? lr.top - y : y > lr.bottom ? y - lr.bottom : 0;
        if (best && dist >= best.dist) continue;
        let at = items.length, lineY = items.length ? items[items.length - 1].getBoundingClientRect().bottom + 4 : lr.top + 6;
        for (const [i, n] of items.entries()) { const nr = n.getBoundingClientRect(); if (y < nr.top + nr.height / 2) { at = i; lineY = nr.top - 4; break; } }
        best = { dist, to: kind === "para" ? l.dataset.block : l.dataset.section, at, lineY, left: lr.left, width: lr.width };
      }
      d.to = best;
      if (best) { d.bar.style.transform = `translate(${best.left}px, ${best.lineY}px)`; d.bar.style.width = `${best.width}px`; }
    };
    // near the top or bottom edge the list scrolls by itself
    const tick = () => {
      if (S.drag !== d) return;
      if (d.on) { const b = sc.getBoundingClientRect(); const v = d.y < b.top + 48 ? -1 : d.y > b.bottom - 48 ? 1 : 0; if (v) { sc.scrollTop += v * 12; target(d.x0, d.y); } }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    const up = () => {
      removeEventListener("pointermove", move);
      removeEventListener("pointerup", up);
      removeEventListener("pointercancel", up);
      S.drag = null;
      d.ghost?.remove();
      d.bar?.remove();
      src.classList.remove("sk-dragging");
      S.root.classList.remove("sk-dragmode");
      if (!d.on || !d.to) return;
      const f = find(id);
      const same = kind === "para" ? f.block.id === d.to.to : f.section.id === d.to.to;
      if (same && d.to.at === f.at) return;
      op({ op: "move", id, to: d.to.to, at: d.to.at });
      if (kind === "para" && !same) { S.editing = { id, field: "reason" }; draw(); }
    };
    addEventListener("pointermove", move);
    addEventListener("pointerup", up);
    addEventListener("pointercancel", up);
  }

  // ── menus ────────────────────────────────────────────
  function menu(at, items) {
    document.querySelector(".menu.sk-menu")?.remove();
    const m = h("div", "menu sk-menu");
    for (const [label, fn] of items) m.append(btn(label, () => { m.remove(); fn(); }, ""));
    (at.closest("dialog") ?? document.body).append(m);
    const r = at.getBoundingClientRect();
    m.style.top = `${Math.min(r.bottom + 4, innerHeight - m.offsetHeight - 8)}px`;
    m.style.left = `${Math.min(r.left, innerWidth - m.offsetWidth - 8)}px`;
    setTimeout(() => addEventListener("pointerdown", function off(e) { if (!m.contains(e.target)) { m.remove(); removeEventListener("pointerdown", off, true); } }, true));
  }

  // ── left: sections and progress ──────────────────────
  function nav() {
    const n = h("nav", "sk-nav");
    for (const s of S.view.sections) {
      const b = h("button", "sk-navsec");
      b.type = "button";
      const ps = s.blocks.flatMap((x) => x.paras);
      b.append(h("span", "sk-navname", SEC_ZH[s.role] ?? s.title));
      const bar = h("span", "sk-bar");
      ps.forEach((p) => { const i = h("i"); i.dataset.s = p.state; bar.append(i); });
      b.append(bar);
      b.onclick = () => S.root.querySelector(`[data-sec="${s.id}"]`)?.scrollIntoView({ behavior: "smooth", block: "start" });
      n.append(b);
    }
    const all = paras();
    const left = S.view.threads.filter((t) => t.status === "open" && ["conflict", "choice", "missing"].includes(t.kind)).length;
    const sum = h("div", "sk-sum");
    const cell = (v, label) => { const c = h("span"); c.append(h("b", "", String(v)), label); return c; };
    sum.append(cell(all.length, "段"), cell(all.filter((p) => p.text).length, "有文字"), cell(all.filter((p) => p.state === "done").length, "完成"));
    if (left) sum.append(cell(left, "題"));
    n.append(sum);
    return n;
  }

  // ── 素材 ─────────────────────────────────────────────
  function inventory() {
    const v = S.view;
    const main = h("div", "sk-inv");
    const keep = new Set(v.inventory.filter((x) => x.keep).map((x) => x.id));
    const save = (body) => run("/inventory", body, "素材");
    for (const [kind, zh] of KINDS) {
      const items = v.inventory.filter((x) => x.kind === kind);
      const sec = h("section", "sk-invsec");
      const hd = h("header", "sk-invhead");
      hd.append(h("h3", "", zh), h("span", "sk-count", String(items.filter((x) => x.keep).length)));
      const tk = TARGET_OF[kind];
      const range = tk && v.targets?.[tk];
      if (range && kind !== "flow") {
        const n = v.inventory.filter((x) => x.keep && TARGET_OF[x.kind] === tk).length;
        const t = h("span", "sk-target", `期刊 ${range[0]}–${range[1]}`);
        if (n < range[0] || n > range[1]) t.classList.add("off");
        hd.append(t);
      }
      sec.append(hd);
      for (const x of items) {
        const row = h("label", "sk-invrow");
        const cb = h("input");
        cb.type = "checkbox";
        cb.checked = x.keep;
        cb.onchange = () => { cb.checked ? keep.add(x.id) : keep.delete(x.id); x.keep = cb.checked; save({ keep: [...keep], add: [] }); };
        const body = h("span", "sk-invbody");
        body.append(h("b", "", x.title));
        if (x.note) body.append(h("span", "sk-invnote", x.note));
        const meta = h("span", "sk-chips");
        if (x.pending) meta.append(h("span", "sk-pend", "等資料"));
        x.evidence.forEach((e) => meta.append(h("span", "sk-src", e)));
        row.append(cb, body, meta);
        sec.append(row);
      }
      const addRow = h("div", "sk-invadd");
      const inp = h("input", "sk-input");
      inp.placeholder = `加一個${zh}`;
      inp.dataset.k = `inv:${kind}`;
      inp.value = S.drafts[`inv:${kind}`] ?? "";
      inp.oninput = () => { S.drafts[`inv:${kind}`] = inp.value; };
      inp.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing && inp.value.trim()) { const title = inp.value.trim(); delete S.drafts[`inv:${kind}`]; save({ keep: [...keep], add: [{ kind, title, note: "" }] }); } };
      addRow.append(inp);
      sec.append(addRow);
      main.append(sec);
    }
    const foot = h("div", "sk-invfoot");
    const go = btn(v.exists ? "重排骨架" : "排骨架", async (e) => {
      if (v.exists && window.LoreShell?.ask && !(await LoreShell.ask(e.currentTarget, ["重排骨架？", "現在的骨架會先存一份。"], [{ label: "取消", value: null }, { label: "重排", value: true, danger: true, primary: true }]))) return;
      run("/plan", { redo: !!v.exists }, "排骨架");
    }, v.exists ? "btn" : "btn primary");
    go.disabled = !!v.job;
    foot.append(go);
    if (!v.exists && v.can_convert) foot.append(btn("改用骨架", () => run("/convert", {}, "改用骨架"), "btn"));
    main.append(foot);
    return main;
  }

  // ── 風格 ─────────────────────────────────────────────
  function styleView() {
    const main = h("div", "sk-style");
    if (!S.style) { main.append(h("p", "sk-wait", "…")); S.call("/style").then((r) => { S.style = r; if (S.tab === "style") draw(); }, (e) => toast(e.message, { err: true })); return main; }
    for (const s of S.style.sections) {
      const sec = h("section", "sk-invsec");
      sec.append(h("h3", "", SEC_ZH[find(s.id)?.section?.role] ?? s.title));
      for (const r of s.rows) {
        const row = h("div", "sk-srow");
        if (r.ok === false) row.classList.add("off");
        row.append(h("span", "sk-slabel", r.label));
        const track = h("span", "sk-track");
        const fmt = (x) => (Number.isInteger(x) ? String(x) : x.toFixed(1));
        if (r.mine && r.draft != null) {
          // the author's range in the middle half of the track; the draft's value on the same scale
          const [lo, hi] = r.mine;
          const span = Math.max(hi - lo, 1e-6);
          const pos = Math.max(0, Math.min(100, 25 + ((r.draft - lo) / span) * 50));
          const band = h("i", "sk-band");
          const mark = h("i", "sk-mark");
          mark.style.left = `${pos}%`;
          track.append(band, mark);
        }
        row.append(track, h("span", "sk-sval", r.draft == null ? "—" : fmt(r.draft)), h("span", "sk-smine", r.mine ? `${fmt(r.mine[0])}–${fmt(r.mine[1])}` : ""));
        sec.append(row);
      }
      for (const p of s.paras) {
        const n = paras().findIndex((x) => x.id === p.id) + 1;
        const b = h("button", "sk-sissue");
        b.type = "button";
        b.append(h("span", "sk-n", `¶${n}`), h("span", "", p.issues.join("、")));
        b.onclick = () => { S.tab = "skeleton"; draw(); requestAnimationFrame(() => { const a = S.root.querySelector(`[data-id="${p.id}"]`); a?.scrollIntoView({ block: "center" }); a?.focus(); }); };
        sec.append(b);
      }
      main.append(sec);
    }
    return main;
  }

  // ── the frame of the page ────────────────────────────
  function head() {
    const v = S.view;
    const bar = h("div", "sk-head");
    const tabs = h("div", "seg");
    for (const [id, label] of [["inventory", "素材"], ["skeleton", "骨架"], ["style", "風格"]]) {
      const b = btn(label, () => { S.tab = id; if (id === "style") S.style = null; draw(); }, "");
      b.setAttribute("aria-pressed", S.tab === id);
      b.disabled = id !== "inventory" && !v.exists;
      tabs.append(b);
    }
    bar.append(tabs);
    if (v.job) { const c = h("span", "chip busy sk-job", `${v.job.label}${v.job.total ? ` ${v.job.done}/${v.job.total}` : ""}`); bar.append(c); }
    bar.append(h("span", "sk-grow"));
    if (v.exists && S.tab === "skeleton") {
      const undo = ico("undo", "復原 ⌘Z", () => op({ op: "undo" }), "ico");
      undo.disabled = !v.can_undo;
      const lay = ico("layers", "提示", () => { S.hints = !S.hints; draw(); }, "ico");
      lay.setAttribute("aria-pressed", S.hints);
      const write = btn("寫文字", (e) => menu(e.currentTarget, [["全部", () => run("/write", { id: "all" }, "寫文字")], ...v.sections.map((s) => [SEC_ZH[s.role] ?? s.title, () => run("/write", { id: s.id }, "寫文字")])]), "btn small");
      const place = ico("insert", "放進論文", () => run("/place", {}, "放進論文").then(() => toast("已放進論文")), "ico");
      place.disabled = !paras().some((p) => p.text);
      const save = btn("暫存", () => run("/save", {}, "暫存"), v.dirty ? "btn small primary" : "btn small");
      save.disabled = !v.dirty || !!v.job;
      bar.append(undo, lay, write, place, save);
    }
    return bar;
  }

  function draw() {
    if (!S.root) return;
    const v = S.view;
    // what is being typed survives a redraw
    const a = document.activeElement;
    const keep = a && S.root.contains(a) && a.dataset?.k ? { k: a.dataset.k, s: a.selectionStart, e: a.selectionEnd } : null;
    const sc = S.root.querySelector(".sk-scroll");
    const top = sc?.scrollTop ?? 0;
    S.redrawing = true;
    S.root.replaceChildren();
    S.redrawing = false;
    if (!v) { S.root.append(h("p", "sk-wait", S.err ?? "…")); return; }
    S.inline = S.root.clientWidth < 900;
    S.root.classList.toggle("sk-inline", S.inline);
    S.root.classList.toggle("sk-nohints", !S.hints);
    S.root.append(head());
    const body = h("div", "sk-body");
    if (S.tab === "inventory" || !v.exists) body.append(inventory());
    else if (S.tab === "style") body.append(styleView());
    else {
      const scroll = h("div", "sk-scroll");
      const page = h("div", "sk-page");
      page.append(canvas());
      if (!S.inline) page.append(margin());
      scroll.append(page);
      body.append(nav(), scroll);
      body.classList.add("sk-three");
    }
    S.root.append(body);
    const sc2 = S.root.querySelector(".sk-scroll");
    if (sc2) sc2.scrollTop = top;
    place();
    if (keep) {
      const n = S.root.querySelector(`[data-k="${CSS.escape(keep.k)}"]`);
      if (n) { n.focus({ preventScroll: true }); try { n.setSelectionRange(keep.s, keep.e); } catch { /* not a text field */ } }
    } else if (S.editing) {
      const n = S.root.querySelector(`[data-k="${CSS.escape(`${S.editing.field}:${S.editing.id}`)}"]`);
      if (n) { n.focus({ preventScroll: true }); n.setSelectionRange(n.value.length, n.value.length); }
    }
  }

  function mount(root, opt) {
    S.root = root;
    S.call = opt.call;
    S.view = null;
    S.active = null;
    S.editing = null;
    root.classList.add("sk-root");
    root.onkeydown = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "z" && !e.shiftKey && !e.target.matches("textarea, input")) { e.preventDefault(); if (S.view?.can_undo) op({ op: "undo" }); }
    };
    new ResizeObserver(() => { const inl = root.clientWidth < 900; if (inl !== S.inline) draw(); else place(); }).observe(root);
    draw();
    return S.call("").then((v) => setView(v, true), (e) => { S.err = e.message; draw(); });
  }

  function open(paper) {
    let dlg = document.querySelector("#dlgSkeleton");
    if (!dlg) {
      dlg = h("dialog", "dialog big sk-dialog");
      dlg.id = "dlgSkeleton";
      dlg.setAttribute("aria-label", "骨架");
      const box = h("div", "dlg");
      const hd = h("div", "dlg-head");
      hd.append(h("h2", "", "骨架"), ico("x", "關閉 Esc", () => dlg.close(), "ico close"));
      const root = h("div");
      root.id = "skRoot";
      box.append(hd, root);
      dlg.append(box);
      document.body.append(dlg);
      dlg.addEventListener("close", () => clearTimeout(S.poll));
    }
    const base = `/api/papers/${encodeURIComponent(paper)}/draft/skeleton`;
    dlg.showModal();
    return mount(dlg.querySelector("#skRoot"), { call: (path, body) => api(base + path, body) });
  }

  return { open, mount, _state: S, _draw: draw };
})();
