// Zoom of the paper in the middle of the window: Ctrl + wheel (a touchpad
// pinch arrives as one), a trackpad pinch in Safari's WebKit, Ctrl +/−/0, and
// the −/%/+ control in the status bar. The point under the pointer stays put.
const PCZoom = (() => {
  const MIN = 0.5, MAX = 2.5, KEY = "pc.zoom";
  let z = 1;
  try { z = clamp(+JSON.parse(localStorage.getItem(KEY)) || 1); } catch {}

  function clamp(v) { return Math.min(MAX, Math.max(MIN, v)); }
  const paper = () => document.getElementById("paper");
  const area = () => document.getElementById("pageArea");

  /** Set the zoom, keeping the paper point under (cx, cy) in place. */
  function set(next, cx, cy) {
    next = Math.round(clamp(next) * 100) / 100;
    const p = paper(), a = area();
    if (!p || !a) return;
    if (cx == null) { const r = a.getBoundingClientRect(); cx = r.left + r.width / 2; cy = r.top + r.height / 2; }
    const r0 = p.getBoundingClientRect();
    const lx = (cx - r0.left) / z, ly = (cy - r0.top) / z;
    z = next;
    p.style.zoom = z === 1 ? "" : String(z);
    const r1 = p.getBoundingClientRect();
    a.scrollLeft += r1.left + lx * z - cx;
    a.scrollTop += r1.top + ly * z - cy;
    try { localStorage.setItem(KEY, JSON.stringify(z)); } catch {}
    const pct = document.querySelector(".zoom .pct");
    if (pct) pct.textContent = `${Math.round(z * 100)}%`;
  }

  function step(dir) { set(dir ? Math.round((z + dir * 0.1) * 10) / 10 : 1); }

  function init() {
    const p = paper();
    if (p && z !== 1) p.style.zoom = String(z);
    const a = area();
    a.addEventListener("wheel", (e) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      const dy = e.deltaY * (e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1);
      set(z * Math.exp(-Math.max(-50, Math.min(50, dy)) * 0.003), e.clientX, e.clientY);
    }, { passive: false });
    // WebKit (macOS) reports a trackpad pinch as gesture events.
    let start = 1;
    a.addEventListener("gesturestart", (e) => { e.preventDefault(); start = z; });
    a.addEventListener("gesturechange", (e) => { e.preventDefault(); set(start * e.scale, e.clientX, e.clientY); });
    document.addEventListener("keydown", (e) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || document.querySelector("dialog[open]")) return;
      if (e.key === "=" || e.key === "+" || e.code === "NumpadAdd") { e.preventDefault(); step(1); }
      else if (e.key === "-" || e.code === "NumpadSubtract") { e.preventDefault(); step(-1); }
      else if (e.key === "0" || e.code === "Numpad0") { e.preventDefault(); step(0); }
    });
  }

  /** The −/%/+ control for the status bar. */
  function control() {
    const box = document.createElement("span");
    box.className = "zoom";
    const b = (label, title, fn, cls) => {
      const x = document.createElement("button");
      x.textContent = label; x.title = title; if (cls) x.className = cls;
      x.onclick = fn;
      box.append(x);
    };
    b("−", "縮小（Ctrl −）", () => step(-1));
    b(`${Math.round(z * 100)}%`, "重設為 100%（Ctrl 0）；Ctrl + 滾輪或觸控板捏合也可縮放", () => step(0), "pct");
    b("+", "放大（Ctrl +）", () => step(1));
    return box;
  }

  return { init, control, set, get: () => z };
})();
