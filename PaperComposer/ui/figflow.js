// 流程圖 in a paper or a book (lore-flow, docs/flow-figures.md §6): the shared editor from /flow/;
// figures saved as figures/<stem>.pdf + <stem>.flow.json by the server (crates/server/src/flow.rs),
// the maths set with TeX glyphs (LoreFlow.paperSvg → the shared mathsvg.js).
//
//   PCFlow.create()   → { file, caption } | null   (context.js inserts the figure block through figure-block)
//   PCFlow.reedit(p)  → re-edit figures/x.pdf when its .flow.json is next to it (the figure's 編輯 button)
(() => {
  const base = () => `/api/papers/${encodeURIComponent(state.paper.name)}`;
  let loading = null;
  function load() {
    if (window.LoreFlow?.edit) return Promise.resolve();
    if (loading) return loading;
    const css = document.createElement("link");
    css.rel = "stylesheet";
    css.href = "/flow/editor.css";
    document.head.append(css);
    loading = new Promise((ok, no) => {
      const s = document.createElement("script");
      s.src = "/flow/editor.js";
      s.onload = ok;
      s.onerror = () => no(new Error("讀不到 /flow/editor.js"));
      document.head.append(s);
    });
    loading.catch(() => (loading = null));
    return loading;
  }
  async function macros() {
    try { const r = await fetch(`${base()}/plot/macros`); return r.ok ? await r.json() : {}; } catch { return {}; }
  }
  async function source() {
    try { const r = await fetch(`${base()}/flow/source`); return r.ok ? (await r.json()).text ?? "" : ""; } catch { return ""; }
  }
  /** Papers read in LoreReader, as other starting points (title + abstract). */
  async function sources() {
    const own = await source();
    const out = own ? [{ label: state.paper?.title || state.paper?.name || "這篇", text: own }] : [];
    return out;
  }
  async function save(spec, replace) {
    const svg = await LoreFlow.paperSvg({ spec, api: "/flow/api", doc: state.paper.name, macros: await macros() });
    const r = await fetch(`${base()}/flow`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ stem: spec.dsl.match(/^fig\s+(\S+)/m)?.[1] ?? "flow", replace, spec, svg }) });
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    return r.json();
  }
  // a paper column (version 1 frame: the text width, about 175 mm), height in proportion
  const FRAME = [496, 300];

  async function create() {
    try { await load(); } catch (e) { LoreShell.toast(e.message, { err: true }); return null; }
    let out = null;
    await LoreFlow.edit({
      target: "paper", title: "新流程圖", api: "/flow/api", doc: state.paper.name, frame: FRAME, source: await source(), sources,
      save: async ({ spec }) => {
        try {
          const r = await save(spec, null);
          if (r.registered?.length) LoreShell.toast(`對照表加入 ${r.registered.join("、")}`, { ms: 5000 });
          out = { file: r.file, caption: r.caption ?? "" };
          return true;
        } catch (e) { return e.message; }
      },
    });
    return out;
  }

  async function reedit(path) {
    const r = await fetch(`${base()}/flow?file=${encodeURIComponent(path)}`).catch(() => null);
    if (!r?.ok) return false;
    const { spec } = await r.json();
    try { await load(); } catch (e) { LoreShell.toast(e.message, { err: true }); return true; }
    await LoreFlow.edit({
      spec, target: "paper", title: path.split("/").pop(), api: "/flow/api", doc: state.paper.name, sources,
      save: async ({ spec }) => {
        try {
          const saved = await save(spec, path);
          window.PCFigures?.reload(path, saved.file);
          if (saved.file !== path) LoreShell.toast(`另存為 ${saved.file}`);
          return true;
        } catch (e) { return e.message; }
      },
    });
    return true;
  }

  window.PCFlow = { create, reedit };
})();
