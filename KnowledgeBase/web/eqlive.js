// 動起來 on a card (docs/living-equations.md, docs/plot.md §11): a display equation's ✦ (md.js) opens the shared
// equation tool (lore-plot, served at /plot/ by kb-server) — the same live view, colours and notation registry as
// LoreReader and the composers. A composite loss or a distribution objective first offers its simulation scenes;
// otherwise the editor opens on the catalogue with the card's LaTeX (KnowledgeBase has no equation-plot model task).
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
  /** The author's registry (對照表): colour roles for the symbols. */
  async function notation() {
    try { const r = await fetch("/api/notation"); return r.ok ? (await r.json()).entries ?? [] : []; } catch { return []; }
  }
  async function animate(btn) {
    const latex = btn.closest(".eq-block")?.dataset.tex;
    if (!latex) return;
    btn.disabled = true;
    try {
      await load();
      const title = document.querySelector("#edTitle")?.value?.trim() || "動起來";
      await window.LorePlot.animateLatex({ latex, anchor: btn, title, api: "/plot/api", notation: await notation(), propose: false });
    } catch (e) {
      window.LoreShell?.toast?.(e.message, { err: true });
    }
    btn.disabled = false;
  }
  document.addEventListener("click", (e) => {
    const b = e.target.closest?.(".eq-block .eq-go");
    if (!b) return;
    e.preventDefault();
    e.stopPropagation();
    animate(b);
  });
  // md.js output is innerHTML: paint the ✦ icons when blocks appear
  new MutationObserver(() => { if (document.querySelector(".eq-block .eq-go:not(:has(svg))")) window.paintIcons?.(document.body); }).observe(document.documentElement, { childList: true, subtree: true });
  window.KBEq = { animate, load };
})();
