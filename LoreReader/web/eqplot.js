// 動起來 (docs/living-equations.md): a display equation of the paper → the shared plotting tool's
// equation editor (lore-plot, served at /plot/). The server's `equation-plot` task proposes the
// figure from the equation's LaTeX, the symbol table rows and the sentence around it, then
// checks it; when that fails the editor opens on the catalogue (manual) with the reason.
//
//   LREq.animate({ latex, symbols: [{ latex, meaning }], context, title, anchor })
// A composite loss or a distribution objective (KL, a weighted sum of losses, a mixture, k-means, softmax with
// label smoothing) first offers the matching simulation scenes (docs/plot.md §11) next to `anchor`; 曲線 = as before.
(() => {
  let loading = null;
  function load() {
    if (window.LorePlot?.edit && window.LorePlot.eqSim) return Promise.resolve();
    if (loading) return loading;
    const css = document.createElement("link");
    css.rel = "stylesheet";
    css.href = "/plot/editor.css";
    document.head.append(css);
    const script = (src) => new Promise((ok, no) => {
      const s = document.createElement("script");
      s.src = src;
      s.onload = ok;
      s.onerror = () => no(new Error(`讀不到 ${src}`));
      document.head.append(s);
    });
    // the order lore_plot::assets::SCRIPTS lists
    loading = ["mathsvg.js", "plot.js", "charts.js", "charts3d.js", "eqcore.js", "simcore.js", "equation.js", "eqlive.js", "eqsim.js", "formats.js", "editor.js"].reduce((p, f) => p.then(() => script(`/plot/${f}`)), Promise.resolve()).then(() => { window.LorePlot.katexBase = "/plot/katex/"; });
    loading.catch(() => (loading = null));
    return loading;
  }
  const toast = (m, o) => window.LoreShell?.toast?.(m, o);

  async function animate({ latex, symbols = [], context = "", title = "", anchor = null }) {
    try { await load(); } catch (e) { toast(e.message, { err: true }); return; }
    // the paper's symbols (symbols.json, lore-notation rows): a colour role set there wins
    const notation = symbols.map((s, i) => ({ id: `lr${i}`, kind: "symbol", latex: s.latex, ...(Number.isInteger(s.color) ? { color: s.color } : {}) }));
    await window.LorePlot.animateLatex({ latex, symbols, context, title, anchor, api: "/plot/api", notation });
  }
  window.LREq = { animate, load };
})();
