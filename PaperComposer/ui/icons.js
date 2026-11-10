// Line icons (Lucide-style, 24px grid). One set for the whole UI so every
// icon button looks the same. Buttons declare `data-icon="name"`; the label
// lives in aria-label and shows as a tooltip on hover.

const ICONS = {
  pen: '<path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"/>',
  database: '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/><path d="M3 12a9 3 0 0 0 18 0"/>',
  layout: '<rect x="3" y="3" width="18" height="7" rx="1"/><rect x="3" y="14" width="9" height="7" rx="1"/><rect x="16" y="14" width="5" height="7" rx="1"/>',
  file: '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5Z"/><path d="M14 2v6h6"/><path d="M16 13H8M16 17H8M10 9H8"/>',
  chevron: '<path d="m6 9 6 6 6-6"/>',
  up: '<path d="m18 15-6-6-6 6"/>',
  listOrdered: '<path d="M10 6h11M10 12h11M10 18h11"/><path d="M4 4h1v4M4 8h2M4 14h2l-2 2.5h2"/>',
  ai: '<text x="12" y="17.2" text-anchor="middle" font-size="15" font-weight="700" font-family="Noto Sans TC, -apple-system, sans-serif" fill="currentColor" stroke="none" letter-spacing=".5">AI</text>',
  book: '<path d="M2 4h6a4 4 0 0 1 4 4v13a3 3 0 0 0-3-3H2Z"/><path d="M22 4h-6a4 4 0 0 0-4 4v13a3 3 0 0 1 3-3h7Z"/>',
  palette: '<path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.93 0 1.65-.75 1.65-1.69 0-.44-.18-.84-.44-1.13-.29-.29-.44-.65-.44-1.13a1.64 1.64 0 0 1 1.67-1.67h2c3.05 0 5.55-2.5 5.55-5.55C21.97 6.01 17.46 2 12 2Z"/><circle cx="13.5" cy="6.5" r="1"/><circle cx="17.5" cy="10.5" r="1"/><circle cx="8.5" cy="7.5" r="1"/><circle cx="6.5" cy="12.5" r="1"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/>',
  moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  contrast: '<circle cx="12" cy="12" r="9"/><path d="M12 18a6 6 0 0 0 0-12Z" fill="currentColor"/>',
  plus: '<path d="M5 12h14M12 5v14"/>',
  save: '<path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2Z"/><path d="M17 21v-8H7v8M7 3v5h8"/>',
  trash: '<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>',
  external: '<path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  x: '<path d="M18 6 6 18M6 6l12 12"/>',
  copy: '<rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>',
  redo: '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>',
  insert: '<path d="M11 12H3M16 6H3M16 18H3M18 9v6M21 12h-6"/>',
  replace: '<path d="m16 3 4 4-4 4M20 7H4M8 21l-4-4 4-4M4 17h16"/>',
  // table editing
  rowAbove: '<rect x="3" y="12" width="18" height="8" rx="1"/><path d="M12 3v6M9 6h6"/>',
  rowBelow: '<rect x="3" y="4" width="18" height="8" rx="1"/><path d="M12 15v6M9 18h6"/>',
  colLeft: '<rect x="12" y="3" width="8" height="18" rx="1"/><path d="M3 12h6M6 9v6"/>',
  colRight: '<rect x="4" y="3" width="8" height="18" rx="1"/><path d="M15 12h6M18 9v6"/>',
  rowDelete: '<rect x="3" y="7" width="18" height="10" rx="1"/><path d="M9 12h6"/>',
  colDelete: '<rect x="7" y="3" width="10" height="18" rx="1"/><path d="M9.5 12h5"/>',
  merge: '<path d="M3 4v16M21 4v16M7 12h10M10 9l-3 3 3 3M14 9l3 3-3 3"/>',
  split: '<path d="M12 4v16M8 9l-3 3 3 3M16 9l3 3-3 3"/>',
  lineAbove: '<path d="M3 5h18" stroke-width="2.6"/><rect x="3" y="10" width="18" height="10" rx="1" stroke-dasharray="2 2.5"/>',
  lineBelow: '<rect x="3" y="4" width="18" height="10" rx="1" stroke-dasharray="2 2.5"/><path d="M3 19h18" stroke-width="2.6"/>',
  code: '<path d="m16 18 6-6-6-6M8 6l-6 6 6 6"/>',
  undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>',
  tableStyle: '<rect x="3" y="4" width="18" height="16" rx="1.5"/><path d="M3 9h18M3 9.01h18" stroke-width="2.2"/><path d="M3 14h18M10 9v11" stroke-dasharray="1.5 2"/>',
  insert2: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 8v8M8 12h8"/>',
  quote: '<path d="M7 7h4v4c0 3-1.5 5-4 6M14 7h4v4c0 3-1.5 5-4 6"/>',
  library: '<path d="M4 4v16M8 4v16M12 4l4 16M17 4l3 16"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>',
  upload: '<path d="M12 15V4M8 8l4-4 4 4M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"/>',
  tag: '<path d="M3 12V4a1 1 0 0 1 1-1h8l9 9-9 9Z"/><circle cx="8" cy="8" r="1.4"/>',
  pdf: '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5Z"/><path d="M14 2v6h6"/><path d="M8.5 16v-4h1.2a1.2 1.2 0 0 1 0 2.4H8.5M13 12v4h.8a2 2 0 0 0 0-4Z"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  back: '<path d="m15 18-6-6 6-6"/>',
  send: '<path d="M22 2 11 13M22 2l-7 20-4-9-9-4Z"/>',
  export: '<path d="M12 3v12M7 10l5 5 5-5M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2"/>',
  cloud: '<path d="M17.5 19H8a5 5 0 1 1 .9-9.92A6 6 0 0 1 20.5 11a4 4 0 0 1-3 8Z"/>',
  cloudOff: '<path d="M17.5 19H8a5 5 0 1 1 .9-9.92A6 6 0 0 1 20.5 11a4 4 0 0 1-3 8Z"/><path d="M4 4l16 16"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"/>',
  // writing toolbar
  bold: '<path d="M7 5h6a3.5 3.5 0 0 1 0 7H7zM7 12h7a3.5 3.5 0 0 1 0 7H7z"/>',
  italic: '<path d="M10 5h8M6 19h8M14 5l-4 14"/>',
  mathInline: '<text x="12" y="16.5" text-anchor="middle" font-size="13" font-style="italic" font-family="Georgia, serif" fill="currentColor" stroke="none">$x$</text>',
  symbol: '<text x="12" y="17" text-anchor="middle" font-size="16" font-family="Georgia, serif" fill="currentColor" stroke="none">α</text>',
  equation: '<path d="M17 5H7l5 7-5 7h10"/>',
  image: '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="10" r="1.6"/><path d="m21 16-5-5-9 9"/>',
  ref: '<path d="M9 7H6a4 4 0 0 0 0 8h3M15 7h3a4 4 0 0 1 0 8h-3M8 11h8"/>',
  footnote: '<path d="M4 18h10M4 13h16M4 8h16"/><text x="19" y="8" text-anchor="middle" font-size="8" fill="currentColor" stroke="none">1</text>',
  list: '<path d="M9 6h11M9 12h11M9 18h11"/><circle cx="4.5" cy="6" r="1" fill="currentColor"/><circle cx="4.5" cy="12" r="1" fill="currentColor"/><circle cx="4.5" cy="18" r="1" fill="currentColor"/>',
  textSize: '<path d="M4 7V5h10v2M9 5v14M7 19h4M14 13v-1h7v1M17.5 12v7M16 19h3"/>',
  star: '<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3l-5.6 2.9 1.1-6.2L3 9.6l6.2-.9Z"/>',
  // start screen: one large icon per mode
  modePaper: '<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h7"/><path d="M13 2l5 5v3"/><path d="M13 2v5h5M8 9h3M8 13h5M8 17h3"/><path d="M20.4 12.6a1.4 1.4 0 0 1 2 2L17 20l-2.6.6.6-2.6Z"/>',
  modeBook: '<path d="M4 19.5V4.5A2.5 2.5 0 0 1 6.5 2H20v15H6.5A2.5 2.5 0 0 0 4 19.5Zm0 0A2.5 2.5 0 0 0 6.5 22H20v-5"/><path d="M8 7h8M8 11h5"/>',
  // 開啟 LaTeX: an open folder marked TeX; 開啟 Word: a page marked W
  openLatex: '<path d="M3 19V6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v1"/><path d="M3 19l2.6-8.2A1.6 1.6 0 0 1 7.1 9.7H21.5L19 19Z"/><text x="12.6" y="17.3" text-anchor="middle" font-size="6.4" font-weight="700" font-family="Georgia, serif" fill="currentColor" stroke="none">TeX</text>',
  openWord: '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5Z"/><path d="M14 2v6h6"/><path d="M8.5 12l1.6 6 1.9-4.6 1.9 4.6 1.6-6"/>',
  comment: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>',
  more: '<circle cx="5" cy="12" r="1" fill="currentColor"/><circle cx="12" cy="12" r="1" fill="currentColor"/><circle cx="19" cy="12" r="1" fill="currentColor"/>',
};

function icon(name) {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("viewBox", "0 0 24 24");
  s.setAttribute("aria-hidden", "true");
  s.classList.add("i");
  s.innerHTML = ICONS[name] ?? "";
  return s;
}

/** Fill every `[data-icon]` element (call again after changing data-icon). */
function paintIcons(root = document) {
  root.querySelectorAll("[data-icon]").forEach((n) => {
    n.querySelector(":scope > svg.i")?.remove();
    n.prepend(icon(n.dataset.icon));
  });
}

// Hover labels for icon buttons. Fixed-position and clamped to the window, so
// they are never clipped by scrolling panels; inside a modal they go into the
// dialog, which sits above everything else.
(() => {
  let hint, timer;
  const hide = () => { clearTimeout(timer); hint?.remove(); hint = null; };
  document.addEventListener("mouseover", (e) => {
    const b = e.target.closest?.(".ico[aria-label]");
    // Buttons with a hover menu show their label as the menu title instead.
    if (!b || b.dataset.menu || b.contains(e.relatedTarget)) return;
    hide();
    timer = setTimeout(() => {
      hint = document.createElement("div");
      hint.className = "hint";
      hint.textContent = b.getAttribute("aria-label");
      (b.closest("dialog") ?? document.body).append(hint);
      const r = b.getBoundingClientRect();
      const w = hint.offsetWidth, h = hint.offsetHeight;
      const below = r.bottom + 6 + h < innerHeight;
      hint.style.top = `${below ? r.bottom + 6 : r.top - 6 - h}px`;
      hint.style.left = `${Math.max(6, Math.min(r.left + r.width / 2 - w / 2, innerWidth - w - 6))}px`;
    }, 350);
  });
  document.addEventListener("mouseout", (e) => {
    const b = e.target.closest?.(".ico[aria-label]");
    if (b && !b.contains(e.relatedTarget)) hide();
  });
  document.addEventListener("mousedown", hide);
})();

/** An icon-only button with a hover label. */
function iconButton(name, label, onclick) {
  const b = document.createElement("button");
  b.className = "ico";
  b.setAttribute("aria-label", label);
  b.append(icon(name));
  if (onclick) b.onclick = onclick;
  return b;
}
