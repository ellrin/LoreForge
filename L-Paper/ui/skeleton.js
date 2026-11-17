// 骨架: the skeleton-first draft (docs/long-paper.md §G, docs/skeleton-api.md).
//   left    sections with their progress
//   middle  the skeleton, always there: section → frame (a few paragraphs under one purpose) →
//           paragraph (what it says → points → text). Drag, Alt+↑↓, ＋ between, undo.
//   right   the margin: threads anchored to paragraphs, like review comments. The AI asks, the
//           author answers, the next question shows at once while the answer is folded into the
//           paragraph (its points grow in place).
// Edits are ops on stable ids, applied here first and then sent in order; the page never sends a
// whole skeleton. While something runs on the server the view is polled.
// Drawing: every draw builds the tree again, then `morph` keeps each node whose data did not
// change (leaves carry a signature). So a poll never touches the box being typed in (an IME
// composition survives), positions animate, and what just grew plays its highlight once.
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
  function toast(msg, opt = {}) {
    if (window.LoreShell?.toast) return LoreShell.toast(msg, opt);
    // without the shell (the demo page): the same toast, simply
    document.querySelector(".toast-stack")?.remove();
    const s = h("div", "toast-stack");
    const t = h("div", `toast${opt.err ? " err" : ""}`);
    t.append(h("span", "", msg));
    if (opt.action) t.append(btn(opt.action.label, () => { s.remove(); opt.action.run(); }, "toast-act"));
    s.append(t);
    document.body.append(s);
    setTimeout(() => s.remove(), opt.ms ?? 2600);
  }
  // asked through the shell, next to the button; the demo page has no shell and goes ahead
  const sure = async (at, lines, label) => (window.LoreShell?.ask ? !!(await LoreShell.ask(at, lines, [{ label: "取消", value: null }, { label, value: true, danger: true, primary: true }])) : true);

  const STATES = ["outline", "points", "text", "done"];
  const KINDS = [["table", "表"], ["figure", "圖"], ["flow", "流程圖"], ["equation", "公式"], ["algorithm", "偽代碼"], ["finding", "發現"]];
  const TARGET_OF = { table: "table", figure: "figure", flow: "figure", equation: "equation" };
  const THREAD_ZH = { conflict: "不一致", choice: "取捨", missing: "要補", note: "你說", edit: "修補", refine: "細修" };
  const BUSY_ZH = { fill: "整理中", write: "寫作中", refine: "細修中", repair: "修補中" };
  const PRESETS = [["wording", "用字"], ["style", "我的風格"], ["check", "檢查"]];
  const SEC_ZH = { introduction: "前言", related: "相關研究", methods: "方法", results: "結果", discussion: "討論", conclusion: "結論" };
  const ASKED = ["conflict", "choice", "missing"];
  const STALE = "有新的回答還沒寫進文字"; // the server's mark on a text older than an answer (plan.rs STALE_TEXT)

  const fresh = () => ({
    root: null, call: null, view: null, err: null,
    tab: "skeleton",            // inventory | skeleton | style
    hints: true,                // the hint layer (roles, links, jumps)
    active: null,               // the thread the author opened (else view.next)
    openThreads: new Set(),     // finished threads shown in full, open jump notes
    folded: new Set(),          // collapsed frames
    editing: null,              // { id, field }
    drafts: {},                 // unsent text by key
    noteAt: null,               // anchor of the note being written
    moved: new Set(),           // paragraphs added or moved here (they show the reason line)
    grew: new Set(),            // keys that just appeared (one highlight)
    rose: new Set(),            // frames that just firmed up or had their last question answered
    inflight: 0, chain: Promise.resolve(), seq: 0, tmp: 0, poll: null, drag: null, style: null, inline: false,
    pendingCommit: null, redrawing: false, ro: null, jobSince: null, clock: null,
  });
  let S = fresh();

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
  const thread = (id) => S.view?.threads.find((t) => t.id === id);
  const threadsAt = (id) => (S.view?.threads ?? []).filter((t) => t.anchor === id);
  const isOpenQ = (t) => !!t && t.status === "open" && ASKED.includes(t.kind);
  const nextThread = () => {
    const ts = S.view?.threads ?? [];
    return (isOpenQ(thread(S.active)) && S.active) || (isOpenQ(thread(S.view.next)) && S.view.next) || ts.find(isOpenQ)?.id || null;
  };
  /** Questions still open in a frame (on it or on its paragraphs). */
  const openIn = (b, v = S.view) => { const ids = new Set([b.id, ...b.paras.map((p) => p.id)]); return (v?.threads ?? []).filter((t) => isOpenQ(t) && ids.has(t.anchor)).length; };
  // a job that failed stays in the view with its error until the next action: nothing to wait for
  const working = () => !!S.view && ((!!S.view.job && !S.view.job.error) || paras().some((p) => p.busy) || S.view.threads.some((t) => t.status === "answered"));

  function setView(v, force) {
    if (!v || typeof v !== "object") return;
    const old = S.view;
    if (!force && old && v.rev <= old.rev) return; // nothing new: the page stays as it is
    if (old?.exists && v.exists) {
      const before = new Map(paras(old).map((p) => [p.id, p]));
      for (const p of paras(v)) {
        const o = before.get(p.id);
        if (!o) continue;
        const had = new Set(o.points.map((x) => x.text));
        p.points.forEach((x) => { if (!had.has(x.text)) S.grew.add(`pt:${p.id}:${x.text}`); });
        if (!o.text && p.text) S.grew.add(`tx:${p.id}`);
      }
      const was = new Map(blocks(old).map((b) => [b.id, [STATES.indexOf(levelOf(b)), openIn(b, old)]]));
      for (const b of blocks(v)) {
        const w = was.get(b.id);
        if (w && (STATES.indexOf(levelOf(b)) > w[0] || (w[1] > 0 && openIn(b, v) === 0))) S.rose.add(b.id);
      }
    }
    // the skeleton has just been made (planned, planned again, or converted): show it
    if (v.exists && old && (!old.exists || (old.job && !v.job && (old.job.kind === "plan" || old.job.kind === "convert")))) S.tab = "skeleton";
    if (v.job && !v.job.error) S.jobSince ??= Date.now(); else S.jobSince = null;
    if (v.job?.error && v.job.error !== old?.job?.error) toast(v.job.error, { err: true });
    S.view = v;
    if (!v.exists && S.tab !== "inventory") S.tab = "inventory";
    draw();
    watch();
  }

  function watch() {
    clearTimeout(S.poll);
    if (!working() || !S.root?.isConnected) return;
    S.poll = setTimeout(async () => {
      const seq = S.seq;
      if (!S.inflight && !S.drag) {
        try {
          const v = await S.call("");
          // something was done here meanwhile: that answer is on its way, this one is old
          if (seq === S.seq && !S.inflight && !S.drag) { setView(v); }
        } catch { /* next time */ }
      }
      watch();
    }, 800);
  }

  /** Send in order. The view that comes back is shown once nothing else is on its way. */
  function post(path, body, label) {
    S.inflight++;
    S.seq++;
    const p = S.chain.then(() => S.call(path, body ?? {}));
    S.chain = p.then(() => {}, () => {});
    return p.then(
      (v) => { S.inflight--; if (!S.inflight) setView(v, true); return v; },
      (e) => {
        S.inflight--;
        toast(label ? `${label}：${e.message}` : e.message, { err: true });
        if (!S.inflight) S.call("").then((v) => setView(v, true), () => {});
        return null;
      },
    );
  }

  // ── ops: here first, then the server ─────────────────
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

  /** Apply an op here and send it. What moves glides to its new place. */
  function op(o) {
    const first = rects();
    if (o.op !== "undo") local(o);
    draw();
    glide(first);
    const { tmp, ...wire } = o;
    return post("/ops", { ops: [wire] });
  }

  async function add(kind, where) {
    const tmp = `tmp${++S.tmp}`;
    const f = kind === "para" ? "say" : "title";
    const o = kind === "para" ? { op: "add_para", block: where.block, at: where.at, say: "", reason: "", tmp } : { op: "add_block", section: where.section, at: where.at, title: "", of: null, tmp };
    S.editing = { id: tmp, field: f };
    if (kind === "para") S.moved.add(tmp);
    const v = await op(o);
    if (!v) return;
    // the server's id for it: the one now at that place
    const real = kind === "para" ? find(where.block, v)?.block?.paras[where.at]?.id : find(where.section, v)?.section?.blocks[where.at]?.id;
    if (!real) return;
    if (kind === "para") { S.moved.delete(tmp); S.moved.add(real); }
    if (S.pendingCommit?.id === tmp) { const c = S.pendingCommit; S.pendingCommit = null; if (c.value) op({ op: "edit", id: real, fields: { [f]: c.value } }); else draw(); return; }
    if (S.editing?.id === tmp) {
      const typed = S.drafts[`${f}:${tmp}`];
      S.editing = { id: real, field: f };
      if (typed != null) { S.drafts[`${f}:${real}`] = typed; delete S.drafts[`${f}:${tmp}`]; }
    }
    draw();
  }

  function commit(id, field, value) {
    delete S.drafts[`${field}:${id}`];
    if (S.editing?.id === id && S.editing.field === field) S.editing = null;
    value = value.trim();
    const f = find(id);
    const obj = f?.para ?? f?.block;
    if (!obj) { draw(); return; }
    if (id.startsWith("tmp")) { S.pendingCommit = { id, value }; obj[field] = value; draw(); return; }
    if (value === (obj[field] ?? "")) { draw(); return; }
    op(field === "reason" ? { op: "reason", id, reason: value } : { op: "edit", id, fields: { [field]: value } });
  }

  function remove(id) {
    const f = find(id);
    if (!f) return;
    op({ op: "remove", id });
    toast(f.para ? "已刪除段落" : "已刪除框", { action: { label: "復原", run: () => op({ op: "undo" }) }, ms: 6000 });
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
    S.root.querySelector(`.sk-canvas [data-id="${id}"]`)?.focus({ preventScroll: true });
  }

  // ── motion ───────────────────────────────────────────
  const calm = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
  const rects = () => new Map([...(S.root?.querySelectorAll(".sk-canvas [data-id]") ?? [])].map((n) => [n.dataset.id, n.getBoundingClientRect()]));
  function glide(first) {
    if (calm()) return;
    S.root.querySelectorAll(".sk-canvas [data-id]").forEach((n) => {
      const a = first.get(n.dataset.id);
      if (!a) { n.animate([{ opacity: 0, transform: "scale(.98)" }, { opacity: 1, transform: "none" }], { duration: 180, easing: "ease-out" }); return; }
      const dy = a.top - n.getBoundingClientRect().top;
      if (Math.abs(dy) > 1) n.animate([{ transform: `translateY(${dy}px)` }, { transform: "none" }], { duration: 200, easing: "cubic-bezier(.2,.8,.2,1)" });
    });
  }
  /** A frame that just firmed up: one ring, and its mark pops. */
  function settle() {
    for (const id of S.rose) {
      const n = S.root.querySelector(`.sk-block[data-id="${id}"]`);
      if (!n || calm()) continue;
      n.animate([{ boxShadow: "0 0 0 0 var(--accent-soft)" }, { boxShadow: "0 0 0 7px var(--accent-soft)", offset: 0.4 }, { boxShadow: "0 0 0 0 transparent" }], { duration: 800, easing: "ease-out" });
      n.querySelector(".sk-level")?.animate([{ transform: "scale(.4)" }, { transform: "scale(1.25)", offset: 0.6 }, { transform: "none" }], { duration: 480, easing: "ease-out" });
    }
    S.rose.clear();
  }

  // ── keyed drawing ────────────────────────────────────
  /** A leaf: reused as it is while its signature stays the same. */
  const leaf = (node, key, sig) => { node.dataset.mk = key; node.__sig = typeof sig === "string" ? sig : JSON.stringify(sig); return node; };
  /** A container: its keyed children are matched one by one. */
  const group = (node, key) => { node.dataset.mk = key; return node; };
  function morph(old, neu) {
    for (const a of [...old.attributes]) if (a.name !== "style" && !neu.hasAttribute(a.name)) old.removeAttribute(a.name);
    for (const a of [...neu.attributes]) if (a.name !== "style" && old.getAttribute(a.name) !== a.value) old.setAttribute(a.name, a.value);
    const have = new Map([...old.children].map((c) => [c.dataset.mk, c]));
    let ref = old.firstElementChild;
    for (const n of [...neu.children]) {
      const o = have.get(n.dataset.mk);
      let use = n;
      if (o && o.tagName === n.tagName && (o.__sig === undefined) === (n.__sig === undefined)) {
        have.delete(n.dataset.mk);
        if (n.__sig === undefined) { morph(o, n); use = o; }
        else if (o.__sig === n.__sig) use = o;
        else { if (o === ref) ref = o.nextElementSibling; o.remove(); }
      }
      if (use === ref) ref = ref.nextElementSibling;
      else old.insertBefore(use, ref);
    }
    have.forEach((c) => c.remove());
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

  const isEditing = (id, name) => S.editing?.id === id && S.editing.field === name;
  /** A field that turns into an editor in place. */
  function field(id, name, value, cls, placeholder, multiline) {
    const key = `${name}:${id}`;
    if (isEditing(id, name)) {
      const t = h("textarea", `sk-edit ${cls}`);
      t.rows = 1;
      t.dataset.k = key;
      t.placeholder = placeholder;
      t.value = S.drafts[key] ?? value ?? "";
      const fit = () => { t.style.height = "auto"; t.style.height = `${t.scrollHeight}px`; };
      t.oninput = () => { S.drafts[key] = t.value; fit(); };
      t.onkeydown = (e) => {
        e.stopPropagation();
        if (e.key === "Enter" && !e.isComposing && !(multiline && !(e.metaKey || e.ctrlKey))) { e.preventDefault(); commit(id, name, t.value); }
        else if (e.key === "Escape") { e.preventDefault(); delete S.drafts[key]; S.editing = null; draw(); }
      };
      t.onblur = () => { if (isEditing(id, name) && !S.redrawing && t.isConnected) commit(id, name, t.value); };
      t.onclick = (e) => e.stopPropagation();
      queueMicrotask(fit); // once it is in the page
      return t;
    }
    const d = h("div", `${cls}${value ? "" : " sk-empty"}`, value || placeholder);
    d.onclick = (e) => { e.stopPropagation(); S.editing = { id, field: name }; draw(); };
    return d;
  }
  const editingOn = (id) => (S.editing?.id === id ? S.editing.field : "");

  // ── the skeleton ─────────────────────────────────────
  const dot = (state, busy, big) => {
    const d = h("i", "sk-dot");
    d.dataset.s = state;
    if (busy) d.dataset.busy = "1";
    if (big && state === "done" && typeof icon === "function") { d.classList.add("sk-tick"); d.append(icon("check")); }
    return d;
  };

  function paraNode(p, n) {
    const ts = threadsAt(p.id);
    const act = nextThread();
    const anchored = ts.some((t) => t.id === act) || S.noteAt === p.id;
    const jumpOpen = S.openThreads.has(`jump:${p.id}`);
    const reasonLine = !!p.reason || (p.touched && S.moved.has(p.id)) || isEditing(p.id, "reason");
    const li = leaf(h("article", "sk-para"), p.id, [p, n, anchored, S.hints, editingOn(p.id), jumpOpen, reasonLine, S.inline && [ts, act, S.noteAt === p.id, [...S.openThreads]]]);
    const id = p.id;
    li.dataset.id = id;
    li.dataset.s = p.state;
    li.tabIndex = 0;
    if (anchored) li.classList.add("sk-anchor");
    if (p.busy) li.dataset.busy = p.busy;
    const hasText = !!p.text;

    const head = h("header", "sk-phead");
    const grip = h("span", "sk-grip");
    grip.onpointerdown = (e) => dragStart(e, id, "para");
    head.append(grip, dot(p.state, p.busy, true), h("span", "sk-n", `¶${n}`));
    if (p.kind) head.append(h("span", "sk-kind", p.kind));
    if (p.busy) head.append(h("span", "sk-busy", BUSY_ZH[p.busy] ?? p.busy));
    const tools = h("span", "sk-tools");
    if (!id.startsWith("tmp")) {
      tools.append(ico("comment", "對這段說一句", () => openNote(id)));
      tools.append(ico("spark", hasText ? "重寫文字" : "寫文字", async (e) => {
        if (hasText && !(await sure(e.currentTarget, ["重寫這一段？", "現在的文字會被換掉，可以復原。"], "重寫"))) return;
        post("/write", { id }, "寫文字");
      }));
      if (hasText) {
        tools.append(ico("polish", "細修", (e) => menu(e.currentTarget, PRESETS.map(([preset, label]) => [label, () => post("/refine", { para: id, preset }, label)]))));
        const done = ico("check", p.state === "done" ? "取消完成" : "這段完成", () => op({ op: "done", id, done: find(id)?.para?.state !== "done" }));
        if (p.state === "done") done.setAttribute("aria-pressed", "true");
        tools.append(done);
      }
      tools.append(ico("trash", "刪除", () => remove(id)));
    }
    head.append(tools);
    li.append(head);

    // once there is text, what the paragraph is for stays as a caption above it (the hint layer)
    if (!hasText || S.hints) li.append(field(id, "say", p.say, hasText ? "sk-say sk-cap" : "sk-say", "這段要說什麼"));
    if (S.hints && (p.link_prev || p.link_next || p.jump)) {
      const l = h("div", "sk-links");
      if (p.link_prev) l.append(h("span", "sk-link up", p.link_prev));
      if (p.link_next) l.append(h("span", "sk-link down", p.link_next));
      if (p.jump) {
        const j = h("button", "sk-jump", "跳");
        j.type = "button";
        j.setAttribute("aria-expanded", jumpOpen);
        j.onclick = (e) => { e.stopPropagation(); toggle(S.openThreads, `jump:${id}`); draw(); };
        l.append(j);
        if (jumpOpen) { const w = h("div", "sk-jumpwhy"); w.append(h("span", "", p.jump.why), h("b", "", p.jump.fix)); l.append(w); }
      }
      li.append(l);
    }
    if (p.points.length && !hasText) {
      const ul = h("ul", "sk-points");
      for (const x of p.points) {
        const i = h("li", "", x.text);
        if (x.from) i.dataset.from = x.from;
        if (S.grew.delete(`pt:${id}:${x.text}`)) i.classList.add("sk-grew");
        ul.append(i);
      }
      li.append(ul);
    }
    if (hasText) {
      if (isEditing(id, "text")) li.append(field(id, "text", p.text, "sk-text", "", true));
      else {
        const t = h("div", "sk-text");
        t.append(prose(p.text));
        if (S.grew.delete(`tx:${id}`)) t.classList.add("sk-grew");
        t.ondblclick = (e) => { e.stopPropagation(); S.editing = { id, field: "text" }; draw(); };
        li.append(t);
      }
      for (const x of p.problems) {
        const pr = h("p", "sk-problem", x);
        if (x === STALE) pr.append(btn("重寫", () => post("/write", { id }, "寫文字"), "ghost sk-inlinebtn"));
        li.append(pr);
      }
    }
    const chips = [...p.floats.map((x) => ["sk-ref", x]), ...p.cites.map((x) => ["sk-cite", x]), ...p.pending.map((x) => ["sk-pend", x])];
    if (chips.length && !hasText) { const c = h("div", "sk-chips"); chips.forEach(([cls, x]) => c.append(h("span", cls, x))); li.append(c); }
    // a paragraph the author added or moved: one optional line of why, read at the next 暫存
    if (reasonLine) li.append(field(id, "reason", p.reason, "sk-reason", "理由（可空）"));
    if (S.inline) { for (const t of ts) li.append(threadNode(t)); if (S.noteAt === id) li.append(noteNode(id)); }

    li.onkeydown = (e) => {
      if (e.target !== li) return;
      if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) { e.preventDefault(); shift(id, e.key === "ArrowUp" ? -1 : 1); }
      else if (e.key === "Enter") { e.preventDefault(); S.editing = { id, field: hasText ? "text" : "say" }; draw(); }
      else if ((e.key === "Backspace" || e.key === "Delete") && !hasText) { e.preventDefault(); remove(id); }
      else if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); const all = [...S.root.querySelectorAll(".sk-para")]; all[all.indexOf(li) + (e.key === "ArrowDown" ? 1 : -1)]?.focus(); }
    };
    return li;
  }

  const gap = (key, fn) => {
    const g = leaf(h("button", "sk-gap"), key, key);
    g.type = "button";
    g.setAttribute("aria-label", "在這裡加一段");
    g.append(h("span", "", "＋"));
    g.onclick = (e) => { e.stopPropagation(); fn(); };
    return g;
  };

  function blockNode(b, count) {
    const id = b.id;
    const level = levelOf(b);
    const folded = S.folded.has(id);
    const left = openIn(b);
    const box = group(h("section", "sk-block"), id);
    box.dataset.id = id;
    box.dataset.s = level;
    if (folded) box.dataset.folded = "1";
    if (!left && (S.view.threads ?? []).some((t) => ASKED.includes(t.kind) && (t.anchor === id || b.paras.some((p) => p.id === t.anchor)))) box.dataset.asked = "1";

    const o = b.of && find(b.of);
    const ofName = o?.block?.title || o?.para?.say || "";
    const now = thread(nextThread())?.anchor;
    const here = now === id || b.paras.some((p) => p.id === now);
    const head = leaf(h("header", "sk-bhead"), "head", [id, b.title, ofName, level, b.paras.map((p) => [p.state, !!p.busy]), folded, editingOn(id), left, here]);
    const grip = h("span", "sk-grip");
    grip.onpointerdown = (e) => dragStart(e, id, "block");
    const mark = h("span", "sk-level");
    mark.dataset.s = level;
    if (level === "done" && typeof icon === "function") mark.append(icon("check"));
    const titles = h("div", "sk-btitles");
    titles.append(field(id, "title", b.title, "sk-btitle", "這幾段要做什麼"));
    if (ofName) titles.append(h("span", "sk-of", `展開「${ofName}」`));
    const dots = h("span", "sk-dots");
    b.paras.forEach((p) => dots.append(dot(p.state, p.busy)));
    const tools = h("span", "sk-tools");
    if (!id.startsWith("tmp")) {
      tools.append(ico("comment", "對這個框說一句", () => openNote(id)));
      if (b.paras.some((p) => !p.text)) tools.append(ico("spark", "寫這個框還沒寫的段落", () => post("/write", { id }, "寫文字")));
      tools.append(ico("trash", "刪除這個框", () => remove(id)));
    }
    const fold = ico("chevron", folded ? "展開" : "摺起來", () => { toggle(S.folded, id); const first = rects(); draw(); glide(first); }, "ico small sk-fold");
    head.append(grip, mark, titles);
    if (left) { const l = h("span", "sk-left", `${left} 題`); if (here) l.classList.add("now"); head.append(l); }
    head.append(dots, tools, fold);
    box.append(head);
    if (folded) { count.n += b.paras.length; return box; }
    if (b.purpose || isEditing(id, "purpose") || !id.startsWith("tmp")) {
      const pu = leaf(h("div", "sk-purposebox"), "purpose", [b.purpose, isEditing(id, "purpose")]);
      pu.append(field(id, "purpose", b.purpose, "sk-purpose", "讀者看完這幾段會知道什麼"));
      box.append(pu);
    }
    const list = group(h("div", "sk-paras"), "paras");
    list.dataset.block = id;
    list.append(gap(`g:${id}:0`, () => add("para", { block: id, at: 0 })));
    b.paras.forEach((p, i) => { list.append(paraNode(p, count.n++), gap(`g:${id}:${i + 1}`, () => add("para", { block: id, at: i + 1 }))); });
    box.append(list);
    if (S.inline) { for (const t of threadsAt(id)) box.append(threadNode(t)); if (S.noteAt === id) box.append(noteNode(id)); }
    return box;
  }

  function canvas() {
    const c = group(h("div", "sk-canvas"), "canvas");
    const count = { n: 1 };
    for (const s of S.view.sections) {
      const sec = group(h("section", "sk-section"), s.id);
      sec.dataset.sec = s.id;
      sec.append(leaf(h("h3", "sk-stitle", SEC_ZH[s.role] ?? s.title), "title", s.title));
      const list = group(h("div", "sk-blocks"), "blocks");
      list.dataset.section = s.id;
      s.blocks.forEach((b) => list.append(blockNode(b, count)));
      const more = leaf(btn("＋ 框", () => add("block", { section: s.id, at: find(s.id).section.blocks.length }), "ghost sk-addblock"), "add", s.id);
      sec.append(list, more);
      c.append(sec);
    }
    return c;
  }

  // ── the margin ───────────────────────────────────────
  const toggle = (set, k) => (set.has(k) ? set.delete(k) : set.add(k));
  const mic = (ta) => (typeof PCDraft !== "undefined" && PCDraft.micButton ? PCDraft.micButton(ta) : null);

  function changeNode(t, c) {
    const d = h("div", "sk-change");
    d.dataset.s = c.state;
    if (c.before) d.append(h("del", "", c.before));
    d.append(h("ins", "", c.after));
    if (c.why) d.append(h("p", "sk-why", c.why));
    const acts = h("div", "sk-acts");
    const send = (accept) => post("/change", { thread: t.id, change: c.id, accept });
    if (c.state === "proposed") acts.append(btn("採用", () => send(true), "btn small"), btn("不採用", () => send(false), "ghost"));
    else if (c.state === "applied" && t.kind === "edit") acts.append(btn("還原", () => send(false), "ghost"));
    else acts.append(h("span", "sk-cstate", { applied: "已採用", reverted: "已還原", rejected: "不採用" }[c.state] ?? ""));
    d.append(acts);
    return d;
  }

  function answer(tid, text, picked, skip) {
    const t = thread(tid);
    text = (text ?? "").trim();
    if (!t || (!skip && !text && !picked.length)) return;
    delete S.drafts[`ans:${tid}`];
    if (skip) t.status = "skipped";
    else {
      t.status = "answered";
      t.messages.push({ who: "me", text: text || t.options.filter((o) => picked.includes(o.id)).map((o) => o.text).join("；") });
      const p = find(t.anchor)?.para;
      if (p) p.busy = "fill";
    }
    S.active = null;
    S.view.next = null;
    draw();
    reveal(nextThread());
    post("/answer", { thread: tid, text, picked, skip: !!skip }, "回答");
  }

  function threadNode(t) {
    const id = t.id;
    const act = nextThread() === id;
    const openQ = isOpenQ(t);
    const full = act || S.openThreads.has(id) || (t.kind === "refine" && t.changes.some((c) => c.state === "proposed"));
    const d = leaf(h("div", "sk-thread"), `t:${id}`, [t, act, full]);
    d.dataset.thread = id;
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
      line.onclick = () => { if (isOpenQ(thread(id))) S.active = id; else toggle(S.openThreads, id); draw(); reveal(id); };
      d.append(line);
      return d;
    }
    const top = h("div", "sk-thead");
    top.append(tag);
    if (!act && S.openThreads.has(id)) top.append(ico("x", "收起來", () => { S.openThreads.delete(id); draw(); }, "ico tiny"));
    d.append(top);
    if (openQ) {
      d.append(h("p", "sk-q", t.question));
      const key = `ans:${id}`;
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
        b.onclick = (e) => { if (e.shiftKey) { ta.value = o.text; S.drafts[key] = o.text; ta.focus(); } else answer(id, "", [o.id]); };
        opts.append(b);
      });
      ta.oninput = () => { S.drafts[key] = ta.value; };
      ta.onkeydown = (e) => {
        e.stopPropagation();
        if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); answer(id, ta.value, []); }
        else if (!ta.value && /^[1-9]$/.test(e.key) && t.options[e.key - 1]) { e.preventDefault(); answer(id, "", [t.options[e.key - 1].id]); }
      };
      const row = h("div", "sk-acts");
      row.append(btn("略過", () => answer(id, "", [], true), "ghost"), h("span", "sk-grow"));
      const m = mic(ta);
      if (m) row.append(m);
      row.append(ico("send", "送出 Enter", () => answer(id, ta.value, []), "ico small sk-send"));
      d.append(opts, ta, row);
      return d;
    }
    for (const m of t.messages) d.append(h("p", `sk-msg ${m.who}`, m.text));
    if (t.conclusion) d.append(h("p", "sk-concl", t.conclusion));
    for (const c of t.changes) d.append(changeNode(t, c));
    return d;
  }

  function openNote(anchor) {
    S.noteAt = anchor;
    draw();
    S.root.querySelector(`[data-k="${CSS.escape(`note:${anchor}`)}"]`)?.focus({ preventScroll: true });
  }

  function noteNode(anchor) {
    const key = `note:${anchor}`;
    const d = leaf(h("div", "sk-thread sk-active"), key, key);
    d.dataset.thread = key;
    d.dataset.anchor = anchor;
    d.dataset.kind = "note";
    const ta = h("textarea", "sk-answer");
    ta.rows = 2;
    ta.dataset.k = key;
    ta.placeholder = "想補充或想改的";
    ta.value = S.drafts[key] ?? "";
    ta.oninput = () => { S.drafts[key] = ta.value; };
    const close = () => { S.noteAt = null; delete S.drafts[key]; draw(); };
    const go = () => {
      const text = ta.value.trim();
      if (!text) return;
      S.noteAt = null;
      delete S.drafts[key];
      const p = find(anchor)?.para;
      if (p) p.busy = "fill";
      draw();
      post("/note", { anchor, text }, "補充");
    };
    ta.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); go(); } else if (e.key === "Escape") { e.preventDefault(); close(); } };
    const row = h("div", "sk-acts");
    row.append(btn("取消", close, "ghost"), h("span", "sk-grow"));
    const m = mic(ta);
    if (m) row.append(m);
    row.append(ico("send", "送出 Enter", go, "ico small sk-send"));
    const top = h("div", "sk-thead");
    top.append(h("span", "sk-tag", THREAD_ZH.note));
    d.append(top, ta, row);
    return d;
  }

  function margin() {
    const m = group(h("aside", "sk-margin"), "margin");
    for (const t of S.view.threads) m.append(threadNode(t));
    if (S.noteAt) m.append(noteNode(S.noteAt));
    return m;
  }

  /** Each thread beside its paragraph; the one being answered sits exactly there, the others make room. */
  function place() {
    const m = S.root.querySelector(".sk-margin");
    if (!m || S.inline) return;
    const base = m.getBoundingClientRect().top;
    const cards = [...m.children].map((n) => {
      let a = S.root.querySelector(`.sk-canvas [data-id="${n.dataset.anchor}"]`);
      // a folded frame is put away with its threads; only one still being answered stays, at the frame
      if (!a && n.classList.contains("sk-active")) { const b = find(n.dataset.anchor)?.block; a = b && S.root.querySelector(`.sk-canvas [data-id="${b.id}"]`); }
      return { n, a };
    });
    cards.forEach((c) => (c.n.hidden = !c.a));
    const live = cards.filter((c) => c.a).map((c) => ({ n: c.n, want: c.a.getBoundingClientRect().top - base, hgt: c.n.offsetHeight, act: c.n.classList.contains("sk-active") })).sort((x, y) => x.want - y.want);
    const act = live.find((c) => c.act);
    let y = 0;
    for (const c of live) {
      let top = c === act ? c.want : Math.max(c.want, y);
      if (act && c !== act && top < act.want + act.hgt + 8 && top + c.hgt + 8 > act.want) top = act.want + act.hgt + 8;
      // a card seen for the first time appears in place; the others glide
      if (!c.n.style.top) { c.n.style.transition = "none"; c.n.style.top = `${top}px`; void c.n.offsetHeight; c.n.style.transition = ""; }
      else c.n.style.top = `${top}px`;
      y = Math.max(y, top + c.hgt + 8);
    }
    m.style.minHeight = `${y}px`;
  }

  /** Bring a thread's paragraph and card into view and put the caret in its answer box. Right after a
   *  draw, never on a later frame: a frame may not come (a hidden or throttled window). */
  function reveal(threadId) {
    if (!threadId) return;
    const anchor = thread(threadId)?.anchor;
    // in a folded frame the paragraph is not drawn: the frame stands for it
    const a = S.root.querySelector(`.sk-canvas [data-id="${anchor}"]`) ?? S.root.querySelector(`.sk-canvas [data-id="${find(anchor)?.block?.id}"]`);
    const card = S.root.querySelector(`.sk-thread[data-thread="${threadId}"]`);
    const sc = S.root.querySelector(".sk-scroll");
    card?.querySelector(".sk-answer")?.focus({ preventScroll: true });
    if (!a || !sc) return;
    const box = sc.getBoundingClientRect();
    const top = a.getBoundingClientRect().top;
    const bottom = Math.max(a.getBoundingClientRect().bottom, card?.getBoundingClientRect().bottom ?? 0);
    if (top < box.top + 40 || bottom > box.bottom - 24) sc.scrollTo({ top: sc.scrollTop + top - box.top - 90, behavior: calm() ? "auto" : "smooth" });
  }

  // ── dragging (pointer events: smooth in the desktop web view, where HTML drag is not) ──
  function dragStart(e, id, kind) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const src = S.root.querySelector(`.sk-canvas [data-id="${id}"]`);
    const r = src.getBoundingClientRect();
    const d = { id, kind, x0: e.clientX, y0: e.clientY, dx: e.clientX - r.left, dy: e.clientY - r.top, on: false, to: null, x: e.clientX, y: e.clientY };
    S.drag = d;
    const sc = S.root.querySelector(".sk-scroll");
    const target = () => {
      const lists = [...S.root.querySelectorAll(kind === "para" ? ".sk-paras" : ".sk-blocks")];
      let best = null;
      for (const l of lists) {
        if (kind === "block" && src.contains(l)) continue;
        const lr = l.getBoundingClientRect();
        const items = [...l.children].filter((n) => n.matches(kind === "para" ? ".sk-para" : ".sk-block") && n !== src);
        const dist = d.y < lr.top ? lr.top - d.y : d.y > lr.bottom ? d.y - lr.bottom : 0;
        if (best && dist >= best.dist) continue;
        let at = items.length, lineY = items.length ? items[items.length - 1].getBoundingClientRect().bottom + 3 : lr.top + 4;
        for (const [i, n] of items.entries()) { const nr = n.getBoundingClientRect(); if (d.y < nr.top + nr.height / 2) { at = i; lineY = nr.top - 5; break; } }
        best = { dist, to: kind === "para" ? l.dataset.block : l.dataset.section, at, lineY, left: lr.left, width: lr.width };
      }
      d.to = best;
      if (best) { d.bar.style.transform = `translate(${best.left}px, ${best.lineY}px)`; d.bar.style.width = `${best.width}px`; }
    };
    const move = (ev) => {
      d.x = ev.clientX;
      d.y = ev.clientY;
      if (!d.on) {
        if (Math.hypot(d.x - d.x0, d.y - d.y0) < 4) return;
        d.on = true;
        S.seq++;
        d.ghost = src.cloneNode(true);
        d.ghost.classList.add("sk-ghost");
        d.ghost.style.width = `${r.width}px`;
        d.ghost.querySelectorAll("[data-k], [data-mk]").forEach((n) => { n.removeAttribute("data-k"); n.removeAttribute("data-mk"); });
        d.bar = h("div", "sk-dropbar");
        (S.root.closest("dialog") ?? document.body).append(d.ghost, d.bar);
        src.classList.add("sk-dragging");
        S.root.classList.add("sk-dragmode");
      }
      d.ghost.style.transform = `translate(${d.x - d.dx}px, ${d.y - d.dy}px)`;
      target();
    };
    // near the top or bottom edge the list scrolls by itself
    const tick = () => {
      if (S.drag !== d) return;
      if (d.on) { const b = sc.getBoundingClientRect(); const v = d.y < b.top + 56 ? -1 : d.y > b.bottom - 56 ? 1 : 0; if (v) { sc.scrollTop += v * 10; target(); } }
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
      S.root.querySelector(".sk-dragging")?.classList.remove("sk-dragging");
      S.root.classList.remove("sk-dragmode");
      if (!d.on) return;
      const f = find(id);
      const same = f && d.to && (kind === "para" ? f.block.id === d.to.to : f.section.id === d.to.to);
      if (!f || !d.to || (same && d.to.at === f.at)) { watch(); return; }
      if (kind === "para") S.editing = { id, field: "reason" };
      op({ op: "move", id, to: d.to.to, at: d.to.at });
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
    const all = paras();
    const left = S.view.threads.filter(isOpenQ).length;
    const n = leaf(h("nav", "sk-nav"), "nav", [S.view.sections.map((s) => [s.id, s.title, s.blocks.flatMap((b) => b.paras.map((p) => p.state))]), left]);
    for (const s of S.view.sections) {
      const b = h("button", "sk-navsec");
      b.type = "button";
      b.append(h("span", "sk-navname", SEC_ZH[s.role] ?? s.title));
      const bar = h("span", "sk-bar");
      s.blocks.flatMap((x) => x.paras).forEach((p) => { const i = h("i"); i.dataset.s = p.state; bar.append(i); });
      b.append(bar);
      b.onclick = () => S.root.querySelector(`[data-sec="${s.id}"]`)?.scrollIntoView({ behavior: calm() ? "auto" : "smooth", block: "start" });
      n.append(b);
    }
    const sum = h("div", "sk-sum");
    const cell = (v, label) => { const c = h("span"); c.append(h("b", "", String(v)), label); return c; };
    sum.append(cell(all.length, "段"), cell(all.filter((p) => p.text).length, "有文字"), cell(all.filter((p) => p.state === "done").length, "完成"));
    if (left) sum.append(cell(left, "題"));
    n.append(sum);
    return n;
  }

  // ── 素材 ─────────────────────────────────────────────
  /** The shared drawing tools, for a figure the paper already holds a place for. */
  const drawTool = (x) => (typeof PCDraft !== "undefined" && PCDraft.makeFloat && (x.tool === "flow" || x.tool === "plot") && x.label ? () => PCDraft.makeFloat(x.label, x.tool) : null);

  function inventory() {
    const v = S.view;
    const main = group(h("div", "sk-inv"), "inv");
    const keep = () => S.view.inventory.filter((x) => x.keep).map((x) => x.id);
    for (const [kind, zh] of KINDS) {
      const items = v.inventory.filter((x) => x.kind === kind);
      const tk = TARGET_OF[kind];
      const range = (kind !== "flow" && tk && v.targets?.[tk]) || null;
      const n = range ? v.inventory.filter((x) => x.keep && TARGET_OF[x.kind] === tk).length : 0;
      const sec = leaf(h("section", "sk-invsec"), kind, [items, range, n]);
      const hd = h("header", "sk-invhead");
      hd.append(h("h3", "", zh), h("span", "sk-count", String(items.filter((x) => x.keep).length)));
      if (range) {
        // figures are counted with the flow charts, as the journal counts them
        const t = h("span", "sk-target", `${kind === "figure" && n !== items.filter((x) => x.keep).length ? `含流程圖 ${n}・` : ""}期刊 ${range[0]}–${range[1]}`);
        if (n < range[0] || n > range[1]) t.classList.add("off");
        hd.append(t);
      }
      sec.append(hd);
      for (const x of items) {
        const row = h("label", "sk-invrow");
        const cb = h("input");
        cb.type = "checkbox";
        cb.checked = x.keep;
        cb.onchange = () => { const it = S.view.inventory.find((y) => y.id === x.id); if (it) it.keep = cb.checked; draw(); post("/inventory", { keep: keep(), add: [] }, "素材"); };
        const body = h("span", "sk-invbody");
        body.append(h("b", "", x.title));
        if (x.note) body.append(h("span", "sk-invnote", x.note));
        const meta = h("span", "sk-chips");
        if (x.pending) meta.append(h("span", "sk-pend", "等資料"));
        x.evidence.forEach((e) => meta.append(h("span", "sk-src", e)));
        const tool = drawTool(x);
        if (tool) meta.append(ico(x.tool === "flow" ? "conn" : "chart", x.tool === "flow" ? "畫流程圖" : "繪圖", tool, "ico tiny"));
        row.append(cb, body, meta);
        sec.append(row);
      }
      const addRow = h("div", "sk-invadd");
      const inp = h("input", "sk-input");
      const key = `inv:${kind}`;
      inp.placeholder = `加一個${zh}`;
      inp.dataset.k = key;
      inp.value = S.drafts[key] ?? "";
      inp.oninput = () => { S.drafts[key] = inp.value; };
      inp.onkeydown = (e) => { if (e.key === "Enter" && !e.isComposing && inp.value.trim()) { const title = inp.value.trim(); delete S.drafts[key]; inp.value = ""; post("/inventory", { keep: keep(), add: [{ kind, title, note: "" }] }, "素材"); } };
      addRow.append(inp);
      sec.append(addRow);
      main.append(sec);
    }
    const foot = leaf(h("div", "sk-invfoot"), "foot", [v.exists, v.can_convert, !!v.job]);
    if (!v.exists && v.can_convert) foot.append(btn("改用骨架", () => post("/convert", {}, "改用骨架"), "btn"));
    const go = btn(v.exists ? "重排骨架" : "排骨架", async (e) => {
      if (S.view.exists && !(await sure(e.currentTarget, ["重排骨架？", "現在的骨架會先存一份。"], "重排"))) return;
      post("/plan", { redo: !!S.view.exists }, "排骨架");
    }, v.exists ? "btn" : "btn primary");
    go.disabled = !!v.job && !v.job.error;
    foot.append(go);
    main.append(foot);
    return main;
  }

  // ── 風格 ─────────────────────────────────────────────
  function styleView() {
    const main = leaf(h("div", "sk-style"), "style", [S.style]);
    if (!S.style) { main.append(h("p", "sk-wait", "…")); S.call("/style").then((r) => { S.style = r; if (S.tab === "style") draw(); }, (e) => toast(e.message, { err: true })); return main; }
    const fmt = (x) => (Number.isInteger(x) ? String(x) : Math.abs(x) < 1 ? x.toFixed(2) : x.toFixed(1));
    for (const s of S.style.sections) {
      const sec = h("section", "sk-invsec");
      const hd = h("div", "sk-srow sk-shead");
      hd.append(h("h3", "", SEC_ZH[find(s.id)?.section?.role] ?? s.title), h("span"), h("span", "sk-sval", "草稿"), h("span", "sk-smine", "我的"));
      sec.append(hd);
      for (const r of s.rows) {
        const row = h("div", "sk-srow");
        if (r.ok === false) row.classList.add("off");
        row.append(h("span", "sk-slabel", r.label));
        const track = h("span", "sk-track");
        const one = r.mine && r.mine[0] === r.mine[1];
        if (r.mine && r.draft != null) {
          // the author's range is the middle half of the track, the draft's value on the same scale;
          // a single typical value is a tick in the middle, the draft left or right of it
          const [lo, hi] = r.mine;
          const pos = one ? (r.draft === lo ? 50 : r.draft < lo ? 22 : 78) : Math.max(0, Math.min(100, 25 + ((r.draft - lo) / (hi - lo)) * 50));
          const mark = h("i", "sk-mark");
          mark.style.left = `${pos}%`;
          track.append(h("i", one ? "sk-band one" : "sk-band"), mark);
        } else track.classList.add("none");
        row.append(track, h("span", "sk-sval", r.draft == null ? "—" : fmt(r.draft)), h("span", "sk-smine", !r.mine ? "" : one ? fmt(r.mine[0]) : `${fmt(r.mine[0])}–${fmt(r.mine[1])}`));
        sec.append(row);
      }
      for (const p of s.paras) {
        const b = h("button", "sk-sissue");
        b.type = "button";
        b.append(h("span", "sk-n", `¶${paras().findIndex((x) => x.id === p.id) + 1}`), h("span", "", p.issues.join("、")));
        b.onclick = () => { S.tab = "skeleton"; draw(); const a = S.root.querySelector(`.sk-canvas [data-id="${p.id}"]`); a?.scrollIntoView({ block: "center" }); a?.focus(); };
        sec.append(b);
      }
      main.append(sec);
    }
    return main;
  }

  // ── the frame of the page ────────────────────────────
  function head() {
    const v = S.view;
    const written = paras().some((p) => p.text);
    const bar = leaf(h("div", "sk-head"), "head", [S.tab, v.exists, v.job, v.dirty, v.can_undo, S.hints, written, v.sections.map((s) => [s.id, s.title])]);
    const tabs = h("div", "seg");
    for (const [id, label] of [["inventory", "素材"], ["skeleton", "骨架"], ["style", "風格"]]) {
      const b = btn(label, () => { S.tab = id; if (id === "style") S.style = null; draw(); }, "");
      b.setAttribute("aria-pressed", S.tab === id);
      b.disabled = id !== "inventory" && !v.exists;
      tabs.append(b);
    }
    bar.append(tabs);
    if (v.job?.error) bar.append(h("span", "chip err", `${v.job.label}：${v.job.error}`));
    else if (v.job) {
      // a long job counts its seconds, so waiting never looks like being stuck (tick() keeps it current)
      const c = h("span", "chip busy sk-job", jobText());
      bar.append(c);
    }
    bar.append(h("span", "sk-grow"));
    if (v.exists && S.tab === "skeleton") {
      const undo = ico("undo", "復原 ⌘Z", () => op({ op: "undo" }), "ico");
      undo.disabled = !v.can_undo;
      const lay = btn("提示", () => { S.hints = !S.hints; draw(); }, "ghost sk-layer");
      lay.setAttribute("aria-pressed", S.hints);
      const writeIn = async (at, id) => {
        const left = S.view.threads.filter(isOpenQ).length;
        if (left && !(await sure(at, [`還有 ${left} 題沒答`, "現在寫，之後的回答要再重寫那幾段。"], "照寫"))) return;
        post("/write", { id }, "寫文字");
      };
      const write = btn("寫文字", (e) => { const at = e.currentTarget; menu(at, [["還沒寫的全部", () => writeIn(at, "all")], ...S.view.sections.map((s) => [SEC_ZH[s.role] ?? s.title, () => writeIn(at, s.id)])]); }, "btn small");
      const save = btn("暫存", () => post("/save", {}, "暫存"), "btn small sk-save");
      save.disabled = !v.dirty || (!!v.job && !v.job.error);
      if (v.dirty) save.dataset.dirty = "1";
      const place = btn("放進論文", async (e) => {
        if (!(await sure(e.currentTarget, ["放進論文？", "論文裡這些章節原有的文字會先存一份。"], "放進論文"))) return;
        if (await post("/place", {}, "放進論文")) toast("已放進論文");
      }, "btn small");
      place.disabled = !written;
      bar.append(undo, lay, h("span", "vsep"), save, write, place);
    }
    return bar;
  }

  const jobText = () => {
    const j = S.view?.job;
    if (!j) return "";
    const sec = S.jobSince ? Math.round((Date.now() - S.jobSince) / 1000) : 0;
    return `${j.label}${j.total > 1 ? ` ${j.done}/${j.total}` : ""}${sec >= 3 ? `　${sec} 秒` : ""}`;
  };
  const tick = () => { const c = S.root?.querySelector(".sk-job"); if (c) c.textContent = jobText(); };

  function build() {
    const v = S.view;
    const top = h("div");
    if (!v) { top.append(leaf(h("p", "sk-wait", S.err ?? "…"), "wait", S.err ?? "")); return top; }
    top.append(head());
    const body = group(h("div", "sk-body"), `body:${v.exists ? S.tab : "inventory"}`);
    if (S.tab === "inventory" || !v.exists) body.append(inventory());
    else if (S.tab === "style") body.append(styleView());
    else {
      const scroll = group(h("div", "sk-scroll"), "scroll");
      const page = group(h("div", "sk-page"), "page");
      page.append(canvas());
      if (!S.inline) page.append(margin());
      scroll.append(page);
      body.append(nav(), scroll);
      body.classList.add("sk-three");
    }
    top.append(body);
    return top;
  }

  function draw() {
    if (!S.root) return;
    // what is being typed survives a redraw, also when its card had to be rebuilt
    const a = document.activeElement;
    const keep = a && S.root.contains(a) && a.dataset?.k ? { k: a.dataset.k, s: a.selectionStart, e: a.selectionEnd } : null;
    S.inline = S.root.clientWidth < 900;
    S.root.classList.toggle("sk-inline", S.inline);
    S.root.classList.toggle("sk-nohints", !S.hints);
    const next = build();
    S.redrawing = true;
    // morph keeps the root's own attributes: only the children are matched
    const shell = S.root.cloneNode(false);
    shell.append(...next.children);
    morph(S.root, shell);
    S.redrawing = false;
    place();
    settle();
    const now = document.activeElement;
    const want = keep?.k ?? (S.editing ? `${S.editing.field}:${S.editing.id}` : null);
    if (want && now?.dataset?.k !== want) {
      const n = S.root.querySelector(`[data-k="${CSS.escape(want)}"]`);
      if (n) {
        n.focus({ preventScroll: true });
        try { keep ? n.setSelectionRange(keep.s, keep.e) : n.setSelectionRange(n.value.length, n.value.length); } catch { /* not a text field */ }
      }
    }
  }

  function mount(root, opt) {
    clearTimeout(S.poll);
    clearInterval(S.clock);
    S.ro?.disconnect();
    S = fresh();
    S.clock = setInterval(tick, 1000);
    S.root = root;
    S.call = opt.call;
    root.replaceChildren();
    root.classList.add("sk-root");
    root.onkeydown = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "z" && !e.shiftKey && !e.target.matches("textarea, input")) { e.preventDefault(); if (S.view?.can_undo) op({ op: "undo" }); }
    };
    S.ro = new ResizeObserver(() => { if ((root.clientWidth < 900) !== S.inline) draw(); else place(); });
    S.ro.observe(root);
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
      dlg.addEventListener("close", () => { clearTimeout(S.poll); clearInterval(S.clock); });
    }
    const base = `/api/papers/${encodeURIComponent(paper)}/draft/skeleton`;
    dlg.showModal();
    return mount(dlg.querySelector("#skRoot"), { call: (path, body) => api(base + path, body) });
  }

  /** Stop looking for changes (the page is put away; mount starts again). */
  const stop = () => { clearTimeout(S.poll); clearInterval(S.clock); S.ro?.disconnect(); };

  return { open, mount, stop, state: () => S, redraw: draw };
})();
