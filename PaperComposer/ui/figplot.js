// 繪圖 in a paper (lore-plot, docs/plot.md §8): the shared editor from /plot/, figures saved as
// figures/<stem>.pdf + <stem>.plot.json by the server (crates/server/src/plot.rs).
//
//   PCPlot.create()            → { file, caption } | null   (context.js inserts the figure block)
//   double-click on a figure   → re-edit when its .plot.json is next to it; the image reloads
(() => {
  const base = () => `/api/papers/${encodeURIComponent(state.paper.name)}`;
  let loading = null;
  function load() {
    if (window.LorePlot?.edit) return Promise.resolve();
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
    loading = ["plot.js", "charts.js", "charts3d.js", "formats.js", "editor.js"].reduce((p, f) => p.then(() => script(`/plot/${f}`)), Promise.resolve()).then(() => { window.LorePlot.katexBase = "/plot/katex/"; });
    loading.catch(() => (loading = null));
    return loading;
  }
  async function macros() {
    try { const r = await fetch(`${base()}/plot/macros`); return r.ok ? await r.json() : {}; } catch { return {}; }
  }
  /** The paper's other figures made here (to pull into a composite). */
  async function library() {
    const r = await fetch(`${base()}/plot/list`);
    return r.ok ? (await r.json()).map((f) => ({ name: f.file, spec: f.spec })) : [];
  }
  async function save(body) {
    const r = await fetch(`${base()}/plot`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    return r.json();
  }
  const stemOf = (spec) => {
    const p = spec.panels?.[0] ?? {};
    const words = [spec.caption, p.title, p.yaxis?.label, p.type].find((s) => s && /[A-Za-z]/.test(s)) ?? "figure";
    return words.replace(/\$[^$]*\$/g, " ").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").split("-").slice(0, 4).join("-") || "figure";
  };

  /** A new figure: the editor, then the PDF and spec in figures/. */
  async function create() {
    try { await load(); } catch (e) { LoreShell.toast(e.message, { err: true }); return null; }
    let out = null;
    await LorePlot.edit({
      target: "paper", title: "新圖", macros: await macros(), api: "/plot/api", library,
      save: async ({ spec, svg }) => {
        try {
          const r = await save({ stem: stemOf(spec), spec, svg });
          out = { file: r.file, caption: spec.caption ?? "" };
          return true;
        } catch (e) { return e.message; }
      },
    });
    return out;
  }

  /** Re-edit `path` (figures/x.pdf) if this tool made it; false otherwise. */
  async function reedit(path) {
    const r = await fetch(`${base()}/plot?file=${encodeURIComponent(path)}`).catch(() => null);
    if (!r?.ok) return false;
    const { spec } = await r.json();
    try { await load(); } catch (e) { LoreShell.toast(e.message, { err: true }); return true; }
    await LorePlot.edit({
      spec, target: "paper", title: path.split("/").pop(), macros: await macros(), api: "/plot/api", library,
      save: async ({ spec, svg }) => {
        try {
          const saved = await save({ replace: path, spec, svg });
          refresh(path, saved.file);
          if (saved.file !== path) LoreShell.toast(`另存為 ${saved.file}`);
          return true;
        } catch (e) { return e.message; }
      },
    });
    return true;
  }
  function refresh(path, file) {
    for (const img of document.querySelectorAll("#paper .tex-figure img")) {
      if (img._path !== path && img._path !== path.replace(/\.pdf$/, "")) continue;
      if (file !== path) continue;
      const u = new URL(img.src, location.href);
      u.searchParams.set("v", Date.now());
      img.src = u.toString();
    }
  }

  // double-click on a figure made here → the editor (else the paragraph editor as before)
  document.addEventListener("dblclick", async (e) => {
    const img = e.target.closest?.("#paper .tex-figure img");
    if (!img?._path || !/(^|\/)[^/]+(\.pdf)?$/i.test(img._path) || /\.(png|jpe?g|eps)$/i.test(img._path)) return;
    e.stopPropagation();
    e.preventDefault();
    const path = /\.pdf$/i.test(img._path) ? img._path : `${img._path}.pdf`;
    if (!(await reedit(path))) {
      // not ours: what a double-click did before (edit the paragraph)
      img.closest(".para, [data-pi]")?.ondblclick?.(new MouseEvent("dblclick"));
    }
  }, true);

  window.PCPlot = { create, reedit };
})();
