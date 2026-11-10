// LoreForge line icons (Lucide-style, 24px grid, stroke 1.75), one set for
// PaperComposer, SlideComposer, LoreReader and KnowledgeBase: the union of the
// four sets the apps used to copy. Buttons declare `data-icon="name"`; the label
// lives in aria-label and shows as a hover hint.
//
//   ICONS                name → SVG body
//   icon(name)           an <svg class="i"> (an old name in ICON_ALIASES works too)
//   paintIcons(root)     fill every [data-icon] (call again after changing data-icon)
//   iconButton(name, label, onclick)   an icon-only .ico button
//
// One name per glyph. Where two apps drew the same thing differently,
// SlideComposer's (then LoreReader's) drawing won and the name stayed
// (file, pdf, save, image, star, moon, fit). Where one name meant two things,
// the second meaning got its own name: ICON_RENAMES lists those call sites, which
// the app changes when it adopts this file. ICON_ALIASES are old names of the
// same glyph and keep working.
//
// Shared slots use these names: 首頁 home · 文件選單 chevron · 對照表 book ·
// 匯出 download · 列印 printer · 另存新檔 save · 開啟 folder · AI spark ·
// 設定 gear · 復原 undo · 重做 redo · 搜尋 search · 同步 cloud / cloudOff ·
// 抽屜 panelLeft / panel · 關閉 x.
//
// commons/crates/lore-shell checks that this file parses, has no duplicate
// names, and covers every icon name the four apps use.

const ICON_ALIASES = { export: "download", list: "listUl", listOrdered: "listOl", panelRight: "panel" };

/** Old name → new name at the listed call sites (a different meaning, not an alias). */
const ICON_RENAMES = [
  { app: "SlideComposer", old: "back", name: "sendBack", where: "panels.js 移到最下層 (the old set defined back twice; the button drew the chevron)" },
  { app: "LoreReader", old: "back", name: "sendBack", where: "unused, the same double definition" },
  { app: "LoreReader", old: "insert", name: "insertPage", where: "index.html #pgInsert 插入頁面…" },
  { app: "LoreReader", old: "quote", name: "excerpt", where: "app.js 存成引用句…, the 引用句 block" },
  { app: "LoreReader", old: "merge", name: "mergeFiles", where: "unused" },
  { app: "PaperComposer", old: "redo", name: "recast", where: "app.js 重新生成 (redo now means 重做)" },
];

const ICONS = {
  // ── SlideComposer's set (the base)
  undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>',
  redo: '<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13"/>',
  cursor: '<path d="m4 4 7 16 2.5-6.5L20 11Z"/>',
  text: '<path d="M4 7V4h16v3M9 20h6M12 4v16"/>',
  square: '<rect x="4" y="5" width="16" height="14" rx="1"/>',
  rounded: '<rect x="4" y="5" width="16" height="14" rx="4"/>',
  circle: '<ellipse cx="12" cy="12" rx="8" ry="7"/>',
  triangle: '<path d="M12 4 21 20H3Z"/>',
  trapezoid: '<path d="M7 5h10l4 14H3Z"/>',
  polygon: '<path d="M12 3 21 9.5 17.5 20h-11L3 9.5Z"/>',
  line: '<path d="M5 19 19 5"/>',
  conn: '<path d="M4 18 18 6"/><path d="M12 6h6v6"/>',
  elbow: '<path d="M4 6h8v12h6"/><path d="m15 15 3 3-3 3"/>',
  curve: '<path d="M4 6c10 0 6 12 16 12"/><path d="m17 15 3 3-3 3"/>',
  table: '<rect x="3" y="4" width="18" height="16" rx="1"/><path d="M3 10h18M3 15h18M9 4v16M15 4v16"/>',
  sigma: '<path d="M18 5H6l6 7-6 7h12"/>',
  puzzle: '<path d="M10 3h4v3a2 2 0 1 0 4 0V3h3v7h-3a2 2 0 1 0 0 4h3v7h-7v-3a2 2 0 1 0-4 0v3H3v-7h3a2 2 0 1 0 0-4H3V3h7Z"/>',
  alignL: '<path d="M4 4v16M8 7h12v4H8zM8 14h7v4H8z"/>',
  alignC: '<path d="M12 4v16M6 7h12v4H6zM8 14h8v4H8z"/>',
  alignR: '<path d="M20 4v16M4 7h12v4H4zM9 14h7v4H9z"/>',
  alignT: '<path d="M4 4h16M7 8h4v12H7zM14 8h4v7h-4z"/>',
  alignM: '<path d="M4 12h16M7 6h4v12H7zM14 8h4v8h-4z"/>',
  alignB: '<path d="M4 20h16M7 4h4v12H7zM14 9h4v7h-4z"/>',
  distH: '<path d="M4 4v16M20 4v16M9 8h6v8H9z"/>',
  distV: '<path d="M4 4h16M4 20h16M8 9h8v6H8z"/>',
  front: '<rect x="8" y="8" width="12" height="12" rx="1" fill="currentColor" fill-opacity=".25"/><path d="M4 16V4h12"/>',
  sendBack: '<rect x="4" y="4" width="12" height="12" rx="1"/><path d="M8 20h12V8" />',
  group: '<rect x="3" y="3" width="8" height="8" rx="1"/><rect x="13" y="13" width="8" height="8" rx="1"/><path d="M3 15v6h6M21 9V3h-6" stroke-dasharray="2 2"/>',
  ungroup: '<rect x="3" y="3" width="8" height="8" rx="1"/><rect x="13" y="13" width="8" height="8" rx="1"/>',
  dup: '<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M4 16V5a1 1 0 0 1 1-1h11"/>',
  crop: '<path d="M6 2v14a2 2 0 0 0 2 2h14"/><path d="M18 22V8a2 2 0 0 0-2-2H2"/>',
  grid: '<path d="M3 3h18v18H3zM3 9h18M3 15h18M9 3v18M15 3v18"/>',
  magnet: '<path d="M6 3v8a6 6 0 0 0 12 0V3h-4v8a2 2 0 0 1-4 0V3Z"/><path d="M6 7h4M14 7h4"/>',
  zoomIn: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5M8 11h6M11 8v6"/>',
  zoomOut: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5M8 11h6"/>',
  fit: '<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>',
  steps: '<rect x="3" y="4" width="7" height="6" rx="1"/><rect x="14" y="4" width="7" height="6" rx="1" opacity=".45"/><rect x="3" y="14" width="7" height="6" rx="1" opacity=".45"/><rect x="14" y="14" width="7" height="6" rx="1" opacity=".45"/>',
  film: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 4v16M17 4v16M3 9h4M3 15h4M17 9h4M17 15h4"/>',
  eyeOff: '<path d="M3 3l18 18M10.6 5.1A10.6 10.6 0 0 1 12 5c6.5 0 10 7 10 7a17 17 0 0 1-3.2 4.1M6.6 6.6A17 17 0 0 0 2 12s3.5 7 10 7a9.8 9.8 0 0 0 5.4-1.6"/><path d="M9.9 9.9a3 3 0 0 0 4.2 4.2"/>',
  pdf: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z"/><path d="M14 3v6h6M8 15h8M8 18h5"/>',
  save: '<path d="M5 3h11l5 5v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z"/><path d="M7 3v6h8M7 21v-7h10v7"/>',
  listUl: '<path d="M9 6h12M9 12h12M9 18h12"/><circle cx="4.5" cy="6" r="1"/><circle cx="4.5" cy="12" r="1"/><circle cx="4.5" cy="18" r="1"/>',
  listOl: '<path d="M10 6h11M10 12h11M10 18h11M4 4v4M3 18h3l-3 2h3M3 10.5a1.5 1.5 0 0 1 3 0c0 1.5-3 2-3 3.5h3"/>',
  indent: '<path d="M3 4h18M10 9h11M10 14h11M3 19h18M3 9l4 2.5L3 14"/>',
  outdent: '<path d="M3 4h18M10 9h11M10 14h11M3 19h18M7 9l-4 2.5L7 14"/>',
  tL: '<path d="M3 5h18M3 10h12M3 15h18M3 20h10"/>',
  tC: '<path d="M3 5h18M6 10h12M3 15h18M7 20h10"/>',
  tR: '<path d="M3 5h18M9 10h12M3 15h18M11 20h10"/>',
  tJ: '<path d="M3 5h18M3 10h18M3 15h18M3 20h18"/>',
  graph: '<circle cx="5" cy="6" r="2.5"/><circle cx="19" cy="7" r="2.5"/><circle cx="11" cy="18" r="2.5"/><path d="M7.4 6.3 16.5 7M6.3 8.2l3.5 7.6M17.6 9.2l-5.1 6.9"/>',
  share: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><path d="m8.6 13.5 6.8 4M15.4 6.5l-6.8 4"/>',
  spark: '<path d="M12 3v4M12 17v4M3 12h4M17 12h4M6 6l2.5 2.5M15.5 15.5 18 18M18 6l-2.5 2.5M8.5 15.5 6 18"/>',
  moon: '<path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8Z"/>',
  pen: '<path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"/>',
  chevron: '<path d="m6 9 6 6 6-6"/>',
  book: '<path d="M2 4h6a4 4 0 0 1 4 4v13a3 3 0 0 0-3-3H2Z"/><path d="M22 4h-6a4 4 0 0 0-4 4v13a3 3 0 0 1 3-3h7Z"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/>',
  plus: '<path d="M5 12h14M12 5v14"/>',
  trash: '<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>',
  external: '<path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
  x: '<path d="M18 6 6 18M6 6l12 12"/>',
  copy: '<rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z"/>',
  upload: '<path d="M12 15V4M8 8l4-4 4 4M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"/>',
  download: '<path d="M12 4v11M8 11l4 4 4-4M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  back: '<path d="m15 18-6-6 6-6"/>',
  send: '<path d="M22 2 11 13M22 2l-7 20-4-9-9-4Z"/>',
  cloud: '<path d="M17.5 19H8a5 5 0 1 1 .9-9.92A6 6 0 0 1 20.5 11a4 4 0 0 1-3 8Z"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"/>',
  news: '<path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Zm0 0a2 2 0 0 1-2-2v-9c0-1.1.9-2 2-2h2"/><path d="M18 14h-8M15 18h-5M10 6h8v4h-8z"/>',
  cards: '<rect x="2" y="6" width="14" height="16" rx="2"/><path d="M6 2h12a2 2 0 0 1 2 2v14"/>',
  tree: '<path d="M4 4h6v6H4zM14 14h6v6h-6zM7 10v4a3 3 0 0 0 3 3h4"/>',
  alert: '<circle cx="12" cy="12" r="9"/><path d="M12 7v6M12 16.5v.5"/>',
  mail: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="m22 7-10 6L2 7"/>',
  calendar: '<rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/>',
  refresh: '<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/>',
  check: '<path d="M20 6 9 17l-5-5"/>',
  menu: '<path d="M4 6h16M4 12h16M4 18h16"/>',
  panel: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M15 4v16"/>',
  eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>',
  code: '<path d="m16 18 6-6-6-6M8 6l-6 6 6 6"/>',
  chat: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2Z"/>',
  chart: '<path d="M3 3v18h18"/><path d="M8 17v-5M13 17V8M18 17v-9"/>',
  flame: '<path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.07-2.14-.22-4.05 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.15.43-2.29 1-3a2.5 2.5 0 0 0 2.5 2.5Z"/>',
  lock: '<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>',
  camera: '<path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3Z"/><circle cx="12" cy="13" r="3"/>',
  recast: '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  target: '<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="5"/><circle cx="12" cy="12" r="1"/>',
  star: '<path d="m12 3 2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9Z"/>',
  layers: '<path d="m12 2 10 5-10 5L2 7Z"/><path d="m2 12 10 5 10-5"/><path d="m2 17 10 5 10-5"/>',
  notebook: '<path d="M5 3h12a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5Z"/><path d="M9 3v18"/><path d="M12.5 8h3M12.5 12h3"/>',
  image: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21"/>',
  play: '<path d="M7 4.5v15l12-7.5Z"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  speaker: '<path d="M11 5 6 9H2v6h4l5 4Z"/><path d="M15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14"/>',
  mic: '<rect x="9" y="2" width="6" height="12" rx="3"/><path d="M19 10v1a7 7 0 0 1-14 0v-1M12 18v4M8 22h8"/>',
  link: '<path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"/>',
  // ── LoreReader
  next: '<path d="m9 18 6-6-6-6"/>',
  up: '<path d="m18 15-6-6-6 6"/>',
  printer: '<path d="M6 9V3h12v6"/><rect x="3" y="9" width="18" height="8" rx="2"/><path d="M6 14h12v7H6z"/>',
  excerpt: '<path d="M4 17c0-5 2-8 6-10M14 17c0-5 2-8 6-10"/><path d="M4 17h4v-4H4zM14 17h4v-4h-4z"/>',
  languages: '<path d="M4 5h9M8.5 3v2M6 5c1 4 4 7 7 8M11 5c-1 4-4 7-7 8"/><path d="m13 21 4-9 4 9M14.5 18h5"/>',
  highlighter: '<path d="m9 11-6 6v3h9l3-3"/><path d="m22 12-4.6 4.6a2 2 0 0 1-2.8 0l-5.2-5.2a2 2 0 0 1 0-2.8L14 4"/>',
  insertPage: '<rect x="4" y="3" width="16" height="7" rx="1"/><rect x="4" y="14" width="16" height="7" rx="1" stroke-dasharray="2 2"/><path d="M12 15.5v4M10 17.5h4"/>',
  mergeFiles: '<path d="M8 3v6a4 4 0 0 0 4 4 4 4 0 0 1 4 4v4M16 3v6a4 4 0 0 1-4 4"/><path d="m13 18 3 3 3-3"/>',
  file: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z"/><path d="M14 3v6h6"/>',
  // ── KnowledgeBase
  sliders: '<path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>',
  pause: '<path d="M8 5v14M16 5v14"/>',
  // ── PaperComposer
  database: '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/><path d="M3 12a9 3 0 0 0 18 0"/>',
  layout: '<rect x="3" y="3" width="18" height="7" rx="1"/><rect x="3" y="14" width="9" height="7" rx="1"/><rect x="16" y="14" width="5" height="7" rx="1"/>',
  ai: '<text x="12" y="17.2" text-anchor="middle" font-size="15" font-weight="700" font-family="Noto Sans TC, -apple-system, sans-serif" fill="currentColor" stroke="none" letter-spacing=".5">AI</text>',
  palette: '<path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.93 0 1.65-.75 1.65-1.69 0-.44-.18-.84-.44-1.13-.29-.29-.44-.65-.44-1.13a1.64 1.64 0 0 1 1.67-1.67h2c3.05 0 5.55-2.5 5.55-5.55C21.97 6.01 17.46 2 12 2Z"/><circle cx="13.5" cy="6.5" r="1"/><circle cx="17.5" cy="10.5" r="1"/><circle cx="8.5" cy="7.5" r="1"/><circle cx="6.5" cy="12.5" r="1"/>',
  contrast: '<circle cx="12" cy="12" r="9"/><path d="M12 18a6 6 0 0 0 0-12Z" fill="currentColor"/>',
  insert: '<path d="M11 12H3M16 6H3M16 18H3M18 9v6M21 12h-6"/>',
  replace: '<path d="m16 3 4 4-4 4M20 7H4M8 21l-4-4 4-4M4 17h16"/>',
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
  tableStyle: '<rect x="3" y="4" width="18" height="16" rx="1.5"/><path d="M3 9h18M3 9.01h18" stroke-width="2.2"/><path d="M3 14h18M10 9v11" stroke-dasharray="1.5 2"/>',
  insert2: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M12 8v8M8 12h8"/>',
  quote: '<path d="M7 7h4v4c0 3-1.5 5-4 6M14 7h4v4c0 3-1.5 5-4 6"/>',
  library: '<path d="M4 4v16M8 4v16M12 4l4 16M17 4l3 16"/>',
  tag: '<path d="M3 12V4a1 1 0 0 1 1-1h8l9 9-9 9Z"/><circle cx="8" cy="8" r="1.4"/>',
  cloudOff: '<path d="M17.5 19H8a5 5 0 1 1 .9-9.92A6 6 0 0 1 20.5 11a4 4 0 0 1-3 8Z"/><path d="M4 4l16 16"/>',
  bold: '<path d="M7 5h6a3.5 3.5 0 0 1 0 7H7zM7 12h7a3.5 3.5 0 0 1 0 7H7z"/>',
  italic: '<path d="M10 5h8M6 19h8M14 5l-4 14"/>',
  mathInline: '<text x="12" y="16.5" text-anchor="middle" font-size="13" font-style="italic" font-family="Georgia, serif" fill="currentColor" stroke="none">$x$</text>',
  symbol: '<text x="12" y="17" text-anchor="middle" font-size="16" font-family="Georgia, serif" fill="currentColor" stroke="none">α</text>',
  equation: '<path d="M17 5H7l5 7-5 7h10"/>',
  ref: '<path d="M9 7H6a4 4 0 0 0 0 8h3M15 7h3a4 4 0 0 1 0 8h-3M8 11h8"/>',
  footnote: '<path d="M4 18h10M4 13h16M4 8h16"/><text x="19" y="8" text-anchor="middle" font-size="8" fill="currentColor" stroke="none">1</text>',
  textSize: '<path d="M4 7V5h10v2M9 5v14M7 19h4M14 13v-1h7v1M17.5 12v7M16 19h3"/>',
  modePaper: '<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h7"/><path d="M13 2l5 5v3"/><path d="M13 2v5h5M8 9h3M8 13h5M8 17h3"/><path d="M20.4 12.6a1.4 1.4 0 0 1 2 2L17 20l-2.6.6.6-2.6Z"/>',
  modeBook: '<path d="M4 19.5V4.5A2.5 2.5 0 0 1 6.5 2H20v15H6.5A2.5 2.5 0 0 0 4 19.5Zm0 0A2.5 2.5 0 0 0 6.5 22H20v-5"/><path d="M8 7h8M8 11h5"/>',
  openLatex: '<path d="M3 19V6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v1"/><path d="M3 19l2.6-8.2A1.6 1.6 0 0 1 7.1 9.7H21.5L19 19Z"/><text x="12.6" y="17.3" text-anchor="middle" font-size="6.4" font-weight="700" font-family="Georgia, serif" fill="currentColor" stroke="none">TeX</text>',
  openWord: '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5Z"/><path d="M14 2v6h6"/><path d="M8.5 12l1.6 6 1.9-4.6 1.9 4.6 1.6-6"/>',
  comment: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>',
  more: '<circle cx="5" cy="12" r="1" fill="currentColor"/><circle cx="12" cy="12" r="1" fill="currentColor"/><circle cx="19" cy="12" r="1" fill="currentColor"/>',
  panelLeft: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>',
  // ── shell
  home: '<path d="M3 10.5 12 3l9 7.5"/><path d="M5 9v11a1 1 0 0 0 1 1h4v-6h4v6h4a1 1 0 0 0 1-1V9"/>',
};

function icon(name) {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("viewBox", "0 0 24 24");
  s.setAttribute("aria-hidden", "true");
  s.classList.add("i");
  s.innerHTML = ICONS[name] ?? ICONS[ICON_ALIASES[name]] ?? "";
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
  let hint, timer, keyed = false;
  const hide = () => { clearTimeout(timer); hint?.remove(); hint = null; };
  document.addEventListener("mouseover", (e) => {
    const b = e.target.closest?.(".ico[aria-label]");
    // Buttons with a hover menu show their label as the menu title instead.
    if (!b || keyed || b.dataset.menu || b.contains(e.relatedTarget)) return;
    hide();
    timer = setTimeout(() => {
      // The button may be gone (redrawn after a key press).
      if (!b.isConnected) return;
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
  // A redraw after a key press can put a new button under a still mouse;
  // only a real mouse move brings the labels back.
  document.addEventListener("keydown", () => { keyed = true; hide(); });
  document.addEventListener("mousemove", () => { keyed = false; });
})();

/** An icon-only button with a hover label. */
function iconButton(name, label, onclick) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "ico";
  b.setAttribute("aria-label", label);
  b.append(icon(name));
  if (onclick) b.onclick = onclick;
  return b;
}
