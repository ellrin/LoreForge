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
    render();
    state.cur = { s: si, p: pi };
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
    p.style.top = `${r.bottom + 6}px`;
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

  function references(anchor) {
    const labels = PCTex.labels, kinds = PCTex.kinds;
    const keys = Object.keys(labels);
    if (!keys.length) return flash("論文裡還沒有 \\label");
    const ta = editor();
    if (!ta) return;
    popover(anchor, (p) => {
      p.classList.add("ref-pop");
      const q = el("input", "search");
      q.placeholder = "搜尋標籤";
      const list = el("div", "ref-pick");
      const NAME = { fig: "Fig.", tab: "Table", eq: "式", sec: "Section", alg: "Algorithm", thm: "Theorem" };
      const draw = () => {
        list.innerHTML = "";
        const f = q.value.trim().toLowerCase();
        keys.filter((k) => !f || k.toLowerCase().includes(f)).forEach((k) => {
          const b = el("button");
          b.append(el("span", "what", `${NAME[kinds[k]] ?? ""} ${labels[k]}`.trim()), el("span", "key", k));
          b.onmousedown = (e) => e.preventDefault();
          b.onclick = () => { put(`${REF_PREFIX[kinds[k]] ?? "\\ref"}{${k}}`); p.remove(); };
          list.append(b);
        });
      };
      q.oninput = draw;
      q.onkeydown = (e) => { if (e.key === "Enter") list.querySelector("button")?.click(); };
      draw();
      p.append(q, list);
      setTimeout(() => q.focus());
    });
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

  return { build, wrap, put, block };
})();
