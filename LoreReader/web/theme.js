// Light / dark mode, the same mechanism as the other LoreForge apps: `data-mode` on
// <html>, remembered in localStorage. Order: this app's own choice (lr.mode) → the
// shared preference (window.LORE_PREFS.theme from /prefs.js; null until lore-home has
// a synced theme setting) → dark. Loaded in <head> so the first paint has the mode.
(() => {
  const ok = (m) => m === "light" || m === "dark";
  let m = null;
  try { m = localStorage.getItem("lr.mode"); } catch {}
  if (!ok(m)) m = window.LORE_PREFS?.theme;
  if (ok(m)) document.documentElement.dataset.mode = m;
  window.loreTheme = {
    get: () => document.documentElement.dataset.mode,
    set(m) { if (!ok(m)) return; document.documentElement.dataset.mode = m; try { localStorage.setItem("lr.mode", m); } catch {} },
    toggle() { this.set(this.get() === "dark" ? "light" : "dark"); },
  };
})();
