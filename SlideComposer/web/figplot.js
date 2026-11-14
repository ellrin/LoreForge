// 繪圖 on a slide (lore-plot, docs/plot.md §8): the shared editor from /plot/; the figure becomes a
// PNG picture that carries its spec (crates/server/src/plot.rs), so it re-opens after a .pptx round trip.
//   toolbar 圖表 (#insPlot)     → a new figure, inserted as a picture
//   the selected figure's 編輯 button → the editor; 完成 replaces the picture's image in place (one undo step)
// Uses app.js's globals: S, api, deckApi, commit, insertObj, baseObj, freeSpot, findObj, topObjEl, slide, toast.
(() => {
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
    loading = ["mathsvg.js", "plot.js", "charts.js", "charts3d.js", "eqcore.js", "simcore.js", "equation.js", "eqlive.js", "eqsim.js", "formats.js", "editor.js"].reduce((p, f) => p.then(() => script(`/plot/${f}`)), Promise.resolve()).then(() => { window.LorePlot.katexBase = "/plot/katex/"; });
    loading.catch(() => (loading = null));
    return loading;
  }
  const EMU_MM = 36000;
  /** The deck's notation entries (colour roles of equation figures: the same registry as LoreReader's and PaperComposer's). */
  async function notation() {
    try { const r = await fetch(deckApi("/notation")); return r.ok ? (await r.json()).notation?.entries ?? [] : []; } catch { return []; }
  }
  /** The picture: a 300 dpi PNG, or for an equation figure an animated GIF (both carry the spec). */
  async function render(spec, svg, frames, gifFrames) {
    const eq = spec.panels?.find((p) => p.type === "equation")?.eq;
    const body = { spec, svg, dpi: 300 };
    if ((eq?.animate || eq?.sim) && gifFrames) {
      // paced by visual change, the end held, heatmaps dithered (sweeps and simulations alike)
      const f = await gifFrames();
      Object.assign(body, { frames: f.frames, delays: f.delays, dither: f.dither, ms: f.ms, ...(f.maxW ? { max_w: f.maxW } : {}) });
    } else if (eq?.animate && frames) {
      body.frames = await frames(eq.animate.frames ?? 24);
      body.ms = eq.animate.ms ?? 80;
    }
    return api(deckApi("/plot"), { body }); // { asset, px, mm, gif? }
  }
  /** Picture frame for a figure of w×h mm: its print size, at most 90 % of the slide. */
  function frameFor(mm, at) {
    const [wmm, hmm] = mm ?? [240, 135];
    let w = wmm * EMU_MM, h = hmm * EMU_MM;
    const k = Math.min(1, (S.deck.size.w * 0.9) / w, (S.deck.size.h * 0.9) / h);
    w = Math.round(w * k);
    h = Math.round(h * k);
    if (at) return { x: at.x + Math.round((at.w - w) / 2), y: at.y + Math.round((at.h - h) / 2), w, h };
    const [x, y] = freeSpot(Math.round((S.deck.size.w - w) / 2), Math.round((S.deck.size.h - h) / 2), w, h);
    return { x, y, w, h };
  }

  async function create(kind) {
    if (!S.deck) return;
    try { await load(); } catch (e) { toast(e.message, true); return; }
    let made = null;
    await LorePlot.edit({
      target: "slide", title: kind === "equation" ? "方程式" : "新圖表", api: "/plot/api", kind, notation: await notation(),
      save: async ({ spec, svg, frames, gifFrames }) => {
        try { made = { ...(await render(spec, svg, frames, gifFrames)), name: spec.panels?.[0]?.title || (kind === "equation" ? "方程式" : "圖表") }; return true; } catch (e) { return e.message; }
      },
    });
    if (!made) return;
    const f = frameFor(made.mm);
    await insertObj({ ...baseObj("picture", f, made.name), asset: made.asset, crop: { l: 0, t: 0, r: 0, b: 0 }, alpha: 1, clip: "rect" }, "插入圖表");
  }

  async function reedit(o) {
    let spec;
    try {
      const r = await fetch(`${deckApi("/plot")}?asset=${encodeURIComponent(o.asset)}`);
      if (!r.ok) return false;
      spec = (await r.json()).spec;
    } catch { return false; }
    try { await load(); } catch (e) { toast(e.message, true); return true; }
    let made = null;
    await LorePlot.edit({
      spec, target: "slide", title: o.name || "圖表", api: "/plot/api", notation: await notation(),
      save: async ({ spec, svg, frames, gifFrames }) => { try { made = await render(spec, svg, frames, gifFrames); return true; } catch (e) { return e.message; } },
    });
    if (!made) return true;
    // the same picture, a new image; the height follows the figure's new aspect (width kept)
    const ratio = made.mm ? made.mm[1] / made.mm[0] : o.frame.h / o.frame.w;
    const h = Math.round(o.frame.w * ratio);
    const ops = [{ op: "set_obj", slide: S.cur, id: o.id, path: "/asset", value: made.asset }, { op: "set_obj", slide: S.cur, id: o.id, path: "/crop", value: { l: 0, t: 0, r: 0, b: 0 } }];
    if (Math.abs(h - o.frame.h) > 1) ops.push({ op: "set_obj", slide: S.cur, id: o.id, path: "/frame/h", value: h });
    await commit(ops, "更新圖表");
    return true;
  }

  document.addEventListener("DOMContentLoaded", () => {
    document.getElementById("insPlot")?.addEventListener("click", () => create());
    document.getElementById("insLiveEq")?.addEventListener("click", () => create("equation"));
  });
  // the selected figure's 編輯 button (app.js renderOverlay): pictures whose PNG carries a spec
  const known = new Map(); // asset → true | false (null while asking)
  (window.SC_FIGURE_TOOLS ??= []).push({
    match: (o) => {
      if (o.type !== "picture" || !o.asset) return null;
      if (!known.has(o.asset)) {
        known.set(o.asset, null);
        fetch(`${deckApi("/plot")}?asset=${encodeURIComponent(o.asset)}`).then((r) => { known.set(o.asset, r.ok); if (r.ok) renderOverlay(); }).catch(() => known.set(o.asset, false));
        return null;
      }
      return known.get(o.asset) ? { label: "編輯圖表", icon: "chart" } : null;
    },
    open: (o) => reedit(o),
  });
  window.SCPlot = { create, reedit, load }; // load: the plot scripts (present.js 互動 uses eqsim.js)
})();
