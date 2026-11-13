// 流程圖 on a slide (lore-flow, docs/flow-figures.md): the shared editor from /flow/; the figure becomes
// one picture — a group of native shapes, connectors and text that keeps its aspect ratio and carries
// its spec (crates/server/src/flow.rs), so it re-opens after a .pptx round trip.
//   toolbar 流程圖 (#insFlow), AI → 流程圖  → a new figure
//   double-click a flow figure (⌥ double-click: edit its text as usual), 排列 → 編輯流程圖
//                                          → the editor; 完成 replaces the figure in place (one undo step)
// Uses app.js's globals: S, api, deckApi, commit, findObj, topObjEl, renderOverlay, renderSide, toast.
(() => {
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
  const EMU_PT = 12700;
  /** The box a figure is drawn into: most of the slide, in pt. */
  const frame = () => [Math.round((S.deck.size.w * 0.9) / EMU_PT), Math.round((S.deck.size.h * 0.78) / EMU_PT)];
  /** The paper open in the 論文 panel, as a starting point. */
  async function sources() {
    const p = window.RD?.paper?.paper ?? (typeof RD !== "undefined" ? RD.paper?.paper : null);
    if (!p) return [];
    const text = [p.title, p.abstract].filter(Boolean).join("\n\n");
    return text ? [{ label: p.title || "論文", text }] : [];
  }
  async function place(spec, replace) {
    const r = await api(deckApi("/flow"), { body: { slide: S.cur, spec, replace } });
    await commit(r.ops, replace ? "更新流程圖" : "插入流程圖");
    S.sel = [r.id];
    renderOverlay();
    renderSide();
  }
  async function create() {
    if (!S.deck) return;
    try { await load(); } catch (e) { toast(e.message, true); return; }
    let src = [];
    try { src = await sources(); } catch {}
    await LoreFlow.edit({
      target: "slide", title: "新流程圖", api: "/flow/api", doc: S.name, frame: frame(),
      source: src[0]?.text ?? "", sources,
      save: async ({ spec }) => { try { await place(spec, null); return true; } catch (e) { return e.message; } },
    });
  }
  async function reedit(o) {
    if (!o?.data?.lore_flow) return false;
    try { await load(); } catch (e) { toast(e.message, true); return true; }
    await LoreFlow.edit({
      spec: o.data, target: "slide", title: o.name.replace(/^flow:/, "") || "流程圖", api: "/flow/api", doc: S.name, sources,
      save: async ({ spec }) => { try { await place(spec, o.id); return true; } catch (e) { return e.message; } },
    });
    return true;
  }

  document.addEventListener("DOMContentLoaded", () => document.getElementById("insFlow")?.addEventListener("click", create));
  // capture: before the canvas starts editing a text inside the figure
  document.addEventListener("dblclick", (ev) => {
    if (typeof S === "undefined" || S.editing || ev.altKey) return;
    // pointer capture retargets the event to the viewport: hit-test again
    const target = document.elementFromPoint(ev.clientX, ev.clientY) ?? ev.target;
    if (!document.getElementById("paperWrap")?.contains(target)) return;
    const el = topObjEl(target);
    const o = el && findObj(+el.dataset.id);
    if (o?.type !== "group" || !o.data?.lore_flow) return;
    ev.stopPropagation();
    ev.preventDefault();
    reedit(o);
  }, true);
  window.SCFlow = { create, reedit };
})();
