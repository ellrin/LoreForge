// Writing toolbar over the page: text style, math, inserted elements and
// links. Inline tools act at the caret of the paragraph being edited (or
// start editing the current one); blocks (equation, figure, list) become a
// paragraph of their own after the caret or after the current paragraph.

const PCWrite = (() => {
  const SYMBOLS = [
    ["α", "alpha"], ["β", "beta"], ["γ", "gamma"], ["δ", "delta"], ["ε", "epsilon"], ["ζ", "zeta"], ["η", "eta"], ["θ", "theta"],
    ["κ", "kappa"], ["λ", "lambda"], ["μ", "mu"], ["ν", "nu"], ["ξ", "xi"], ["π", "pi"], ["ρ", "rho"], ["σ", "sigma"],
    ["τ", "tau"], ["φ", "phi"], ["χ", "chi"], ["ψ", "psi"], ["ω", "omega"],
    ["Γ", "Gamma"], ["Δ", "Delta"], ["Θ", "Theta"], ["Λ", "Lambda"], ["Π", "Pi"], ["Σ", "Sigma"], ["Φ", "Phi"], ["Ψ", "Psi"], ["Ω", "Omega"],
    ["∑", "sum"], ["∏", "prod"], ["∫", "int"], ["∂", "partial"], ["∇", "nabla"], ["√", "sqrt{}"], ["∞", "infty"],
    ["±", "pm"], ["×", "times"], ["·", "cdot"], ["÷", "div"], ["∘", "circ"], ["⊗", "otimes"], ["⊕", "oplus"],
    ["≤", "leq"], ["≥", "geq"], ["≠", "neq"], ["≈", "approx"], ["≡", "equiv"], ["∝", "propto"], ["∼", "sim"],
    ["∈", "in"], ["∉", "notin"], ["⊂", "subset"], ["⊆", "subseteq"], ["∪", "cup"], ["∩", "cap"], ["∅", "emptyset"],
    ["→", "rightarrow"], ["←", "leftarrow"], ["⇒", "Rightarrow"], ["⇔", "Leftrightarrow"], ["↦", "mapsto"],
    ["∀", "forall"], ["∃", "exists"], ["¬", "neg"], ["∧", "land"], ["∨", "lor"],
    ["ℝ", "mathbb{R}"], ["ℕ", "mathbb{N}"], ["ℤ", "mathbb{Z}"], ["𝔼", "mathbb{E}"], ["ℓ", "ell"],
    ["x̂", "hat{x}"], ["x̄", "bar{x}"], ["x̃", "tilde{x}"], ["ẋ", "dot{x}"], ["‖·‖", "|x\\|"],
  ];
  const REF_PREFIX = { fig: "Fig.~\\ref", tab: "Table~\\ref", sec: "Section~\\ref", alg: "Algorithm~\\ref", thm: "Theorem~\\ref", eq: "\\eqref" };

  const active = () => document.querySelector(".para textarea");

  /** The textarea to work in: the one open, or the current paragraph opened for editing. */
  function editor() {
    const ta = active();
    if (ta) return ta;
    if (!state.paper || !curSection()) { flash("先開啟一篇論文"); return null; }
    const p = paraEl(state.cur.s, state.cur.p);
    if (!p) { flash("先點一下要寫的段落"); return null; }
    edit(p, state.cur.s, state.cur.p);
    const t = active();
    if (t) t.setSelectionRange(t.value.length, t.value.length);
    return t;
  }

  function changed(ta) { ta.dispatchEvent(new Event("input")); ta.focus(); }

  /** Wrap the selection (or a placeholder) in before…after, selecting the inside. */
  function wrap(before, after, placeholder = "") {
    const ta = editor();
    if (!ta) return;
    const { selectionStart: a, selectionEnd: b } = ta;
    const inner = ta.value.slice(a, b) || placeholder;
    ta.setRangeText(before + inner + after, a, b, "end");
    ta.setSelectionRange(a + before.length, a + before.length + inner.length);
    changed(ta);
  }

  /** Insert text at the caret; the caret goes to `caret` within it (default: end). */
  function put(text, caret = text.length) {
    const ta = editor();
    if (!ta) return;
    const a = ta.selectionStart;
    ta.setRangeText(text, a, ta.selectionEnd, "end");
    ta.setSelectionRange(a + caret, a + caret);
    changed(ta);
  }

  // Inside $…$ (an odd number of unescaped $ before the caret)?
  function inMath(ta) {
    const before = ta.value.slice(0, ta.selectionStart).replace(/\\\$/g, "");
    return (before.match(/\$/g)?.length ?? 0) % 2 === 1 || /\\begin\{(equation|align|gather|multline)\*?\}(?![\s\S]*\\end\{)/.test(before);
  }

  /** A block on its own paragraph: at the caret while editing, else after the current paragraph. */
  async function block(text, caret) {
    const ta = active();
    if (ta) {
      const a = ta.selectionStart;
      const pre = ta.value.slice(0, a).replace(/\s+$/, "");
      const post = ta.value.slice(ta.selectionEnd).replace(/^\s+/, "");
      const head = pre ? `${pre}\n\n` : "";
      ta.value = `${head}${text}${post ? `\n\n${post}` : ""}`;
      ta.setSelectionRange(head.length + caret, head.length + caret);
      changed(ta);
      return;
    }
    if (!state.paper || !curSection()) return flash("先開啟一篇論文");
    const si = state.cur.s, pi = sections()[si].paragraphs.length ? state.cur.p + 1 : 0;
    sections()[si].paragraphs.splice(pi, 0, text);
    try { await saveSection(si); } catch (e) { flash(`儲存失敗：${e.message}`); }
    state.cur = { s: si, p: pi };
    refreshSection(si);
    const p = paraEl(si, pi);
    if (!p) return;
    edit(p, si, pi);
    active()?.setSelectionRange(caret, caret);
  }

  // ── popovers ──
  function popover(anchor, build) {
    document.querySelector(".wb-pop")?.remove();
    const p = el("div", "pop wb-pop");
    build(p);
    document.body.append(p);
    const r = anchor.getBoundingClientRect();
    const h = p.offsetHeight;
    // below the anchor; above it when there is no room (a right-click near the bottom)
    p.style.top = `${r.bottom + 6 + h > innerHeight - 8 && r.top - h - 6 > 8 ? r.top - h - 6 : Math.max(8, Math.min(r.bottom + 6, innerHeight - h - 8))}px`;
    p.style.left = `${Math.max(8, Math.min(r.left, innerWidth - p.offsetWidth - 8))}px`;
    const close = (e) => { if (!p.contains(e.target) && e.target !== anchor) { p.remove(); document.removeEventListener("mousedown", close, true); } };
    setTimeout(() => document.addEventListener("mousedown", close, true));
    p.addEventListener("keydown", (e) => { if (e.key === "Escape") { p.remove(); active()?.focus(); } });
    return p;
  }

  function symbols(anchor) {
    const ta = editor();
    if (!ta) return;
    popover(anchor, (p) => {
      p.classList.add("sym-pop");
      const q = el("input", "search");
      q.placeholder = "alpha、leq…";
      const grid = el("div", "sym-grid");
      const draw = () => {
        grid.innerHTML = "";
        const f = q.value.trim().toLowerCase();
        SYMBOLS.filter(([, cmd]) => !f || cmd.toLowerCase().includes(f)).forEach(([glyph, cmd]) => {
          const b = el("button", "sym", glyph);
          b.title = `\\${cmd}`;
          b.onmousedown = (e) => e.preventDefault();
          b.onclick = () => {
            const tex = `\\${cmd}`;
            const caret = tex.includes("{}") ? tex.indexOf("{}") + 1 : tex.length;
            if (inMath(ta)) put(tex, caret);
            else put(`$${tex}$`, caret + 1);
          };
          grid.append(b);
        });
      };
      q.oninput = draw;
      q.onkeydown = (e) => { if (e.key === "Enter") grid.querySelector("button")?.click(); };
      draw();
      p.append(q, grid);
      setTimeout(() => q.focus());
    });
  }

  /** What a reference to `key` writes: the template's form (tex.js) or the default prefix. */
  function refSnippet(kind, key, atSentenceStart) {
    return PCTex.refSnippet?.(kind, key, atSentenceStart) ?? `${REF_PREFIX[kind] ?? "\\ref"}{${key}}`;
  }
  /** Nothing but spaces since the last . ! ? (or the paragraph's start). */
  const sentenceStart = (before) => { const t = before.replace(/[\s~]+$/, ""); return !t || /[.!?]$/.test(t); };

  // ── what each label stands for: a caption's start, a heading, an equation ──
  const labelCache = new Map(); // paragraph text → { key: [text, isTex] }
  /** Inside of the {…} group opening at i. */
  function groupAt(s, i) {
    let d = 0;
    for (let k = i; k < s.length; k++) {
      if (s[k] === "\\") { k++; continue; }
      if (s[k] === "{") d++;
      else if (s[k] === "}" && --d === 0) return s.slice(i + 1, k);
    }
    return s.slice(i + 1);
  }
  const plain = (tex) => PCFind.mask(tex).replace(/[\0\u0001]/g, "").replace(/[\s~]+/g, " ").trim();
  function labelTexts(text) {
    let out = labelCache.get(text);
    if (out) return out;
    out = {};
    const opened = (re) => [...text.matchAll(re)].map((m) => ({ at: m.index, body: groupAt(text, m.index + m[0].length - 1) }));
    const caps = opened(/\\caption\*?\s*(?:\[[^\]]*\])?\s*\{/g);
    const heads = opened(/\\(?:sub)*section\*?\s*\{/g);
    const near = (list, at) => [...list].reverse().find((c) => c.at < at) ?? list.find((c) => c.at > at);
    for (const m of text.matchAll(/\\label\s*\{([^}]*)\}/g)) {
      const c = near(caps, m.index) ?? near(heads, m.index);
      if (c) { out[m[1]] = [plain(c.body), false]; continue; }
      // an equation: its own LaTeX up to the label
      const b = text.lastIndexOf("\\begin{", m.index);
      const from = b < 0 ? 0 : text.indexOf("}", b) + 1;
      out[m[1]] = [text.slice(from, m.index).replace(/\s+/g, " ").trim(), true];
    }
    if (labelCache.size > 2000) labelCache.clear();
    labelCache.set(text, out);
    return out;
  }
  function describeLabels() {
    const all = {};
    sections().forEach((s) => {
      (s.tail?.match(/\\label\{([^}]*)\}/g) ?? []).forEach((l) => (all[l.slice(7, -1)] = [plain(s.title), false]));
      s.paragraphs.forEach((t) => { if (t.includes("\\label")) Object.assign(all, labelTexts(t)); });
    });
    return all;
  }

  /** Pick a label to refer to: number and the start of its caption, heading or
   *  equation. Without `pick` the reference goes in at the caret of the
   *  paragraph being edited; with it, pick(kind, key) inserts it. */
  function references(anchor, pick) {
    const labels = PCTex.labels, kinds = PCTex.kinds;
    const ORDER = ["fig", "tab", "eq", "thm", "alg", "sec"];
    const rank = (k) => { const i = ORDER.indexOf(kinds[k]); return i < 0 ? ORDER.length : i; };
    const keys = Object.keys(labels).map((k, i) => [k, i]).sort((a, b) => rank(a[0]) - rank(b[0]) || a[1] - b[1]).map(([k]) => k);
    if (!keys.length) return flash("論文裡還沒有 \\label");
    const ta = pick ? null : editor();
    if (!pick && !ta) return;
    const texts = describeLabels();
    return popover(anchor, (p) => {
      p.classList.add("ref-pop");
      const q = el("input", "search");
      q.placeholder = "搜尋圖、表、式、章節";
      const list = el("div", "ref-pick");
      const NAME = { fig: "Fig.", tab: "Table", eq: "式", sec: "Section", alg: "Algorithm", thm: "Theorem" };
      const choose = (k) => {
        p.remove();
        if (pick) return pick(kinds[k], k);
        put(refSnippet(kinds[k], k, sentenceStart(ta.value.slice(0, ta.selectionStart))));
      };
      const draw = () => {
        list.innerHTML = "";
        const f = q.value.trim().toLowerCase();
        keys.forEach((k) => {
          const what = `${NAME[kinds[k]] ?? ""} ${labels[k]}`.trim();
          const [cap, tex] = texts[k] ?? ["", false];
          if (f && ![k, what, cap].some((x) => x.toLowerCase().includes(f))) return;
          const b = el("button");
          b.title = k;
          b.append(el("span", "what", what), el("span", `cap${tex || !cap ? " tex" : ""}`, cap || k));
          b.onmousedown = (e) => e.preventDefault();
          b.onclick = () => choose(k);
          list.append(b);
        });
        if (!list.firstChild) list.append(el("p", "empty", "沒有符合的項目"));
      };
      const move = (d) => {
        const bs = [...list.querySelectorAll("button")];
        const i = bs.indexOf(document.activeElement);
        (i + d < 0 ? q : bs[Math.min(i + d, bs.length - 1)])?.focus();
      };
      q.oninput = draw;
      p.addEventListener("keydown", (e) => {
        if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); move(e.key === "ArrowDown" ? 1 : -1); }
        else if (e.key === "Enter" && e.target === q) list.querySelector("button")?.click();
      });
      draw();
      p.append(q, list);
      setTimeout(() => q.focus());
    });
  }

  // ── collect a sentence into the phrase bank ──
  /** The selection: inside the paragraph being edited, else on the page. */
  function selectedText() {
    const ta = active();
    if (ta && ta.selectionEnd > ta.selectionStart) return ta.value.slice(ta.selectionStart, ta.selectionEnd);
    return String(document.getSelection() ?? "");
  }

  /** Select a sentence, click: the engine splits it into a pattern and
   *  phrases, the user ticks what to keep, and it is saved in one step. */
  async function collect(anchor, picked) {
    const text = (picked ?? selectedText()).trim();
    if (!text) return flash("先選取想收藏的句子");
    const section = roleOf(curSection()?.title ?? "Any");
    const p = popover(anchor, (p) => {
      p.classList.add("collect-pop");
      p.append(el("p", "note", "拆解中…"));
    });
    let props;
    try {
      props = await api("/api/decompose", { text, section, engine: chosenEngine() });
    } catch (e) {
      p.innerHTML = "";
      p.append(el("p", "note bad", e.message));
      return;
    }
    p.innerHTML = "";
    const picks = [];
    const group = (label, items, show, kind) => {
      if (!items.length) return;
      const head = el("div", "prop-head");
      head.append(label, el("span", "c", String(items.length)));
      const ul = el("ul", "prop");
      items.forEach((x) => {
        const li = el("li");
        const cb = el("input"); cb.type = "checkbox"; cb.checked = true;
        const [meta, main, sub] = show(x);
        const body = el("div");
        const t = el("div", "t");
        t.append(kind === "phrases" ? slotted(main) : main);
        body.append(el("div", "meta", meta), t);
        if (sub) body.append(el("div", "u", sub));
        li.append(cb, body);
        ul.append(li);
        picks.push([cb, kind, x]);
      });
      p.append(head, ul);
    };
    group("句型", props.phrases, (x) => [`${x.section} · ${x.function} · ${x.position}`, x.template, x.example], "phrases");
    group("片語", props.expressions, (x) => [`${x.kind}${x.section ? " · " + x.section : ""}`, x.text, x.usage], "expressions");
    if (!picks.length) { p.append(el("p", "note", "這句沒有可收藏的句型")); return; }
    const save = el("button", "primary small", "加入句型庫");
    save.onclick = async () => {
      const chosen = { phrases: [], expressions: [] };
      picks.filter(([cb]) => cb.checked).forEach(([, kind, x]) => chosen[kind].push(x));
      if (!chosen.phrases.length && !chosen.expressions.length) { p.remove(); return; }
      // The database editor holds unsaved edits: add there, so its save does not drop these.
      const open = state.db?.dirty;
      if (open?.has("phrases.csv") || open?.has("expressions.csv")) {
        chosen.phrases.forEach((x) => addRow("phrases.csv", { section: x.section, function: x.function, position: x.position, template: x.template, example: x.example, tags: (x.tags ?? []).join(";") }));
        chosen.expressions.forEach((x) => addRow("expressions.csv", { kind: x.kind, text: x.text, usage: x.usage, section: x.section, example: x.example }));
        p.remove();
        return flash("資料庫有未儲存的修改：已加進編輯中的表格，記得儲存", 6000);
      }
      save.disabled = true; save.textContent = "儲存中…";
      try {
        const r = await api("/api/phrases/add", chosen);
        const n = r.phrases.length + r.expressions.length;
        const skipped = chosen.phrases.length + chosen.expressions.length - n;
        flash(n ? `已加入 ${n} 筆${skipped ? `（${skipped} 筆已經有了）` : ""}` : "都已經在句型庫裡了");
        try { state.profile = await api("/api/profile"); } catch {}
        if (typeof refreshGit === "function") refreshGit();
        p.remove();
      } catch (e) {
        save.disabled = false; save.textContent = "加入句型庫";
        flash(e.message, 6000);
      }
    };
    p.append(save);
  }

  // ── the bar ──
  const TOOLS = [
    [["bold", "粗體　Ctrl+B", () => wrap("\\textbf{", "}")], ["italic", "斜體　Ctrl+I", () => wrap("\\emph{", "}")]],
    [
      ["mathInline", "行內數學", () => wrap("$", "$")],
      ["symbol", "符號", (b) => symbols(b)],
      ["equation", "方程式", () => { const t = "\\begin{equation}\n  \n  \\label{eq:}\n\\end{equation}"; block(t, t.indexOf("\n") + 3); }],
    ],
    [
      ["tableStyle", "表格", (b) => PCTable.insertNew(b)],
      ["image", "圖", () => { const t = "\\begin{figure}[t]\n  \\centering\n  \\includegraphics[width=\\linewidth]{}\n  \\caption{}\n  \\label{fig:}\n\\end{figure}"; block(t, t.indexOf("]{}") + 2); }],
      ["list", "項目清單", () => { const t = "\\begin{itemize}\n  \\item \n\\end{itemize}"; block(t, t.indexOf("\\item ") + 6); }],
      ["listOrdered", "編號清單（例如列出 contributions）", () => { const t = "\\begin{enumerate}\n  \\item \n  \\item \n\\end{enumerate}"; block(t, t.indexOf("\\item ") + 6); }],
    ],
    [
      ["quote", "引用", (b) => PCRefs.picker(b), "wbCite"],
      ["ref", "參照", (b) => references(b)],
      ["footnote", "註腳", () => wrap("\\footnote{", "}")],
    ],
    [["star", "收藏句型：選取喜歡的句子，拆解後加入句型庫", (b) => collect(b)]],
  ];

  function build() {
    const bar = $("#writebar");
    bar.innerHTML = "";
    TOOLS.forEach((group) => {
      const g = el("div", "grp");
      group.forEach(([ic, label, fn, id]) => {
        const b = iconButton(ic, label);
        if (id) b.id = id;
        // keep the caret in the paragraph being edited
        b.onmousedown = (e) => e.preventDefault();
        b.onclick = () => fn(b);
        g.append(b);
      });
      bar.append(g);
    });
    document.addEventListener("keydown", (e) => {
      if (!(e.ctrlKey || e.metaKey) || !e.target.matches?.(".para textarea")) return;
      if (e.key === "b") { e.preventDefault(); wrap("\\textbf{", "}"); }
      if (e.key === "i") { e.preventDefault(); wrap("\\emph{", "}"); }
    });
  }

  return { build, wrap, put, block, references, collect, popover, refSnippet, sentenceStart };
})();
