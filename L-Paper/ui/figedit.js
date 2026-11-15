// The 編輯 button on a figure the plotting or flow-chart tool made (docs/plot.md §8, docs/flow-figures.md §6):
// click a figure in the paper → a small button at its corner when its spec (.plot.json / .flow.json) is next
// to it; the button opens that tool's editor. A double-click on a figure is not special.
(() => {
  const base = () => `/api/papers/${encodeURIComponent(state.paper.name)}`;
  const known = new Map(); // path → "plot" | "flow" | null
  async function toolOf(path) {
    if (known.has(path)) return known.get(path);
    let tool = null;
    for (const t of ["plot", "flow"]) {
      const r = await fetch(`${base()}/${t}?file=${encodeURIComponent(path)}`).catch(() => null);
      if (r?.ok) { tool = t; break; }
    }
    known.set(path, tool);
    return tool;
  }
  const pathOf = (img) => (img?._path ? (/\.pdf$/i.test(img._path) ? img._path : /\.(png|jpe?g|eps|svg)$/i.test(img._path) ? null : `${img._path}.pdf`) : null);
  let btn = null, cur = null;
  function hide() { btn?.remove(); btn = null; document.querySelectorAll("#paper .tex-figure.fig-picked").forEach((f) => f.classList.remove("fig-picked")); cur = null; }
  function place() {
    if (!btn || !cur?.isConnected) return hide();
    const r = cur.getBoundingClientRect();
    Object.assign(btn.style, { left: `${Math.min(r.right - 34, innerWidth - 40)}px`, top: `${Math.max(r.top + 6, 52)}px` });
  }
  async function pick(img) {
    const path = pathOf(img);
    if (!path) return hide();
    const tool = await toolOf(path);
    if (!tool) return hide();
    hide();
    cur = img;
    img.closest(".tex-figure")?.classList.add("fig-picked");
    btn = document.createElement("button");
    btn.type = "button";
    btn.className = "ico small fig-edit";
    btn.setAttribute("aria-label", tool === "flow" ? "編輯流程圖" : "編輯圖表");
    btn.title = btn.getAttribute("aria-label");
    btn.append(window.icon ? window.icon(tool === "flow" ? "graph" : "chart") : "✎");
    btn.onclick = async (e) => {
      e.stopPropagation();
      const p = path;
      hide();
      if (tool === "flow") await window.PCFlow?.reedit(p); else await window.PCPlot?.reedit(p);
    };
    document.body.append(btn);
    place();
  }
  document.addEventListener("click", (e) => {
    if (btn?.contains(e.target)) return;
    const img = e.target.closest?.("#paper .tex-figure img");
    if (img) pick(img); else hide();
  }, true);
  addEventListener("scroll", place, true);
  addEventListener("resize", place);
  /** A figure file was rewritten: reload its pictures (and forget what we knew about it). */
  function reload(path, file) {
    known.delete(path);
    known.delete(file);
    for (const img of document.querySelectorAll("#paper .tex-figure img")) {
      if (img._path !== path && img._path !== path.replace(/\.pdf$/, "")) continue;
      if (file !== path) continue;
      const u = new URL(img.src, location.href);
      u.searchParams.set("v", Date.now());
      img.src = u.toString();
    }
  }
  window.PCFigures = { reload, pick };
})();
