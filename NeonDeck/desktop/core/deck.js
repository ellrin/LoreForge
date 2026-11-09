/*
 * NeonDeck 核心：橋接 Swift、載入小工具、主題切換、版面編輯。
 *
 * ── 主題 ────────────────────────────────────────────────
 *   themes/<name>.css           美術風格
 *   themes/<name>.layout.json   這個主題的預設排版（第一次切到該主題時套用）
 *   切換主題時，小工具的「設定」共用（鬧鐘、捷徑、新聞關鍵字不會不見），「位置」每個主題各自記。
 *
 * ── layout.json ─────────────────────────────────────────
 *   { theme, widgets: { id: { type, config } },
 *     positions: { theme: { "<螢幕寬>x<高>": { ref: [w, h], items: { id: { x, y, w, h } } } } } }
 *   位置依「主題 × 螢幕解析度」分開存。座標以 ref（排版時的畫布大小）為準，
 *   顯示時整張畫布等比例縮放置中，所以換螢幕不會跑版。
 *   沒排過的解析度：借用比例最接近的那份來顯示；在這台螢幕上一編輯，就另存一份給它，不會動到原本的。
 *   主題附的 themes/<name>.layout.json 可用舊的扁平格式 { id: rect }，搭配 "screen": [w, h]（省略 = LEGACY_REF）。
 *
 * ── 小工具模組（widgets/<type>.js）─────────────────────────
 *   export default {
 *     title: 'NET.LINK', tag: 'NET',      // 標題列文字、標籤（主題決定要不要顯示）
 *     defaultSize: { w, h }, minSize: { w, h },
 *     channels: ['stats'],                 // 需要 Swift 推送的頻道：stats / procs / slow / disk
 *     defaultConfig: { ... },
 *     mount(body, ctx) { ...; return { resize() {}, destroy() {} } }
 *   }
 *
 * ── ctx ─────────────────────────────────────────────────
 *   ctx.config / ctx.save()     設定（存在 layout.json，跨主題共用）
 *   ctx.on(ch, fn)              訂閱頻道（含最後一次的值），卸載時自動取消；'theme' 頻道 = 主題切換 / CSS 熱更新
 *   ctx.call(method, params)    呼叫 Swift，回傳 Promise
 *   ctx.info / ctx.editing / ctx.setTitle(text) / ctx.el
 */

const bridge = window.webkit?.messageHandlers?.deck;
const listeners = new Map();
const lastValue = new Map();

let mockImpl;   // 不在 App 裡（例如用 Chrome 預覽）時改用假資料
async function call(method, params = null) {
  if (bridge) return bridge.postMessage({ method, params });
  mockImpl ||= (await import('./mock.js')).createMock(emit);
  return mockImpl(method, params);
}

function on(channel, fn, { replay = true } = {}) {
  if (!listeners.has(channel)) listeners.set(channel, new Set());
  listeners.get(channel).add(fn);
  if (replay && lastValue.has(channel)) { try { fn(lastValue.get(channel)); } catch (e) { console.error(e); } }
  return () => listeners.get(channel)?.delete(fn);
}

function emit(channel, payload) {
  lastValue.set(channel, payload);
  listeners.get(channel)?.forEach(fn => { try { fn(payload); } catch (e) { console.error(`[${channel}]`, e); } });
}

window.__deck = { receive: ({ c, p }) => emit(c, p) };

// JS 錯誤寫到 ~/Library/Logs/NeonDeck.log
window.addEventListener('error', e => call('log', `ERROR ${e.message} @ ${e.filename}:${e.lineno}`).catch(() => {}));
window.addEventListener('unhandledrejection', e => call('log', `REJECT ${e.reason?.stack || e.reason}`).catch(() => {}));
const _cerr = console.error;
console.error = (...a) => { _cerr(...a); call('log', 'console.error ' + a.map(x => x?.stack || String(x)).join(' ')).catch(() => {}); };

// ---------------------------------------------------------------------------

const DEFAULT_THEME = 'v3-diagonal';
const desktop = document.getElementById('desktop');
const themeLink = document.getElementById('theme-css');
const SNAP = 8;

let layout = { theme: DEFAULT_THEME, widgets: {}, positions: {} };
let available = { widgets: [], themes: [] };
let info = {};
let editing = false;
const mounted = new Map();     // id -> { id, def, el, inst, unsubs }

const uid = () => Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
const snap = v => Math.round(v / SNAP) * SNAP;

// ---------- 依螢幕解析度的版面 ----------

const LEGACY_REF = [1512, 982];   // 舊版 layout.json / 主題預設排版是在這個大小排的
let view = { key: '', entry: null, s: 1 };   // 目前顯示用的那份版面與縮放比例

const screenKey = () => `${innerWidth}x${innerHeight}`;
const themeScreens = (t = layout.theme) => (layout.positions[t] ||= {});
const isRect = v => v && typeof v === 'object' && typeof v.x === 'number';

/** 舊的扁平格式 { id: rect } → { "WxH": { ref, items } } */
function normalizeScreens(p, ref = LEGACY_REF) {
  if (!p || typeof p !== 'object') return {};
  const vals = Object.values(p);
  if (vals.length && vals.every(isRect)) return { [`${ref[0]}x${ref[1]}`]: { ref: [...ref], items: p } };
  for (const [k, e] of Object.entries(p)) if (!e?.items || !Array.isArray(e.ref)) delete p[k];
  return p;
}

/** 這個螢幕自己的版面；沒有就挑比例、大小最接近的 */
function pickEntry() {
  const screens = themeScreens();
  const key = screenKey();
  if (screens[key]) return screens[key];
  const W = innerWidth, H = innerHeight;
  let best = null, bestScore = Infinity;
  for (const e of Object.values(screens)) {
    const [w, h] = e.ref;
    const score = Math.abs(Math.log((W / H) / (w / h))) * 4 + Math.abs(Math.log((W * H) / (w * h)));
    if (score < bestScore) { bestScore = score; best = e; }
  }
  return best || (screens[key] = { ref: [W, H], items: {} });
}

/** 把畫布縮放置中到目前螢幕 */
function applyView() {
  const [w, h] = view.entry.ref;
  const s = Math.min(innerWidth / w, innerHeight / h);
  const ox = Math.round((innerWidth - w * s) / 2), oy = Math.round((innerHeight - h * s) / 2);
  view.s = s;
  Object.assign(desktop.style, {
    inset: 'auto', left: '0', top: '0', width: w + 'px', height: h + 'px',
    transformOrigin: '0 0',
    transform: s === 1 && !ox && !oy ? '' : `translate(${ox}px, ${oy}px) scale(${s})`,
  });
}

function selectView() {
  view.key = screenKey();
  view.entry = pickEntry();
  applyView();
}

/** 顯示用（唯讀） */
const pos = () => view.entry.items;

/** 要修改版面時呼叫：這個螢幕還沒有自己的版面，就把目前借用的那份複製一份給它 */
function ownPos() {
  const screens = themeScreens();
  if (screens[view.key] !== view.entry) {
    view.entry = screens[view.key] = structuredClone(view.entry);
  }
  return view.entry.items;
}

/** 對目前主題的每一份版面做同一件事（新增 / 刪除小工具要同步到各螢幕） */
const eachEntry = fn => Object.values(themeScreens()).forEach(fn);

let saveTimer;
function saveLayout() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => call('saveLayout', layout).catch(console.error), 300);
  reportHits();
}

// 告訴 Swift 哪些區域是小工具：只有這些地方接收滑鼠，其他地方的點擊 / 右鍵穿透到原本的桌面
function reportHits() {
  // 小工具本身 + 小工具打開的浮層（加上 class="hit-extra" 的元素，例如彈出框、響鈴畫面）
  const els = [...[...mounted.values()].map(m => m.el), ...document.querySelectorAll('.hit-extra')];
  const rects = els.map(el => {
    const r = el.getBoundingClientRect();
    return [r.x, r.y, r.width, r.height];
  }).filter(r => r[2] > 0 && r[3] > 0);
  call('hitRegions', { editing, rects }).catch(() => {});
}
// 換螢幕 / 解析度改變：換成該螢幕的版面（或重新縮放）
let resizeTimer;
window.addEventListener('resize', () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(async () => {
    if (!view.entry) return;
    const prev = view.entry;
    selectView();
    if (view.entry !== prev) {
      for (const id of [...mounted.keys()]) unmountWidget(id);
      await mountAll();
    }
    call('log', `screen ${view.key}, layout ref=${view.entry.ref.join('x')}, scale=${view.s.toFixed(3)}`).catch(() => {});
    reportHits();
  }, 150);
});
document.addEventListener('contextmenu', e => { if (!e.target.closest('input, textarea')) e.preventDefault(); });

// ---------- 主題 ----------

async function themeDefaults(name) {
  try { return await (await fetch(`themes/${name}.layout.json?v=${Date.now()}`)).json(); }
  catch { return { widgets: {}, positions: {} }; }
}

/** 確保目前主題有排版；沒有就套用主題附的預設排版 */
async function ensureThemeLayout(name) {
  layout.positions[name] = normalizeScreens(layout.positions[name]);
  if (Object.values(layout.positions[name]).some(e => Object.keys(e.items).length)) return;
  const d = await themeDefaults(name);
  for (const [id, w] of Object.entries(d.widgets || {})) {
    if (!layout.widgets[id]) layout.widgets[id] = structuredClone(w);
  }
  layout.positions[name] = normalizeScreens(structuredClone(d.positions || {}), d.screen || LEGACY_REF);
}

function loadThemeCSS(name) {
  return new Promise(resolve => {
    themeLink.onload = themeLink.onerror = () => resolve();
    themeLink.href = `themes/${name}.css?v=${Date.now()}`;
    document.body.dataset.theme = name;
  });
}

async function switchTheme(name) {
  if (!available.themes.includes(name)) name = available.themes.includes(DEFAULT_THEME) ? DEFAULT_THEME : available.themes[0];
  for (const id of [...mounted.keys()]) unmountWidget(id);
  layout.theme = name;
  await ensureThemeLayout(name);
  selectView();
  await loadThemeCSS(name);
  await mountAll();
  emit('theme', name);
  const sel = document.getElementById('eb-theme');
  if (sel) sel.value = name;
  saveLayout();
}

on('setTheme', name => switchTheme(name), { replay: false });

on('cssChanged', () => {
  // CSS 熱更新：只換樣式表，不重新載入頁面
  document.querySelectorAll('link[rel=stylesheet]').forEach(l => {
    const u = new URL(l.href, location.href);
    u.searchParams.set('v', Date.now());
    l.href = u.pathname + u.search;
  });
  setTimeout(() => { emit('theme', layout.theme); reportHits(); }, 150);
}, { replay: false });

// ---------- 頻道訂閱 ----------

function updateSubscriptions() {
  const set = new Set();
  for (const m of mounted.values()) (m.def.channels || []).forEach(c => set.add(c));
  call('subscribe', [...set]).catch(() => {});
}

// ---------- 掛載小工具 ----------

const defCache = new Map();
async function loadDef(type) {
  if (!defCache.has(type)) defCache.set(type, import(`../widgets/${type}.js?v=${Date.now()}`).then(m => m.default));
  return defCache.get(type);
}

function place(el, p) {
  // 超出畫布的只在顯示時拉回來，不改存檔
  const [W, H] = view.entry.ref;
  const w = Math.min(p.w, W), h = Math.min(p.h, H);
  el.style.left = Math.max(0, Math.min(p.x, W - w)) + 'px';
  el.style.top = Math.max(0, Math.min(p.y, H - h)) + 'px';
  el.style.width = w + 'px';
  el.style.height = h + 'px';
}

async function mountWidget(id, order) {
  const w = layout.widgets[id];
  const p = pos()[id];
  if (!w || !p) return;
  const el = document.createElement('section');
  el.className = `widget w-${w.type}`;
  el.dataset.id = id;
  el.style.zIndex = order;
  el.innerHTML = `
    <div class="w-frame"><div class="w-panel">
      <header class="w-head"><span class="w-tag"></span><span class="w-title"></span></header>
      <div class="w-body"></div>
    </div></div>
    <button class="w-del" title="從這個主題移除">✕</button>
    <div class="w-resize"></div>`;
  place(el, p);
  desktop.appendChild(el);
  const body = el.querySelector('.w-body');
  const titleEl = el.querySelector('.w-title');
  const unsubs = [];

  let def;
  try {
    def = await loadDef(w.type);
  } catch (e) {
    titleEl.textContent = w.type;
    body.innerHTML = `<div class="w-error">載入 widgets/${w.type}.js 失敗\n${e}</div>`;
    mounted.set(id, { id, def: { channels: [] }, el, inst: {}, unsubs });
    wireEditing(el, id, {});
    return;
  }

  w.config = { ...structuredClone(def.defaultConfig || {}), ...(w.config || {}) };
  const cfg = w.config;
  if (cfg.className) el.classList.add(...String(cfg.className).split(/\s+/).filter(Boolean));
  if (cfg.hue) el.style.setProperty('--h', cfg.hue);
  el.classList.toggle('no-head', !!cfg.hideHeader);
  titleEl.textContent = cfg.title ?? def.title ?? w.type;
  el.querySelector('.w-tag').textContent = cfg.tag ?? def.tag ?? '';

  const ctx = {
    id, el, info,
    get config() { return w.config; },
    get editing() { return editing; },
    get theme() { return layout.theme; },
    save: saveLayout,
    call,
    on: (c, fn, opts) => { const u = on(c, fn, opts); unsubs.push(u); return u; },
    setTitle: t => { titleEl.textContent = t; },
    refreshHits: () => requestAnimationFrame(reportHits),   // 打開 / 關閉 .hit-extra 浮層後呼叫
  };

  let inst = {};
  try {
    inst = def.mount(body, ctx) || {};
  } catch (e) {
    console.error(e);
    body.innerHTML = `<div class="w-error">${w.type}.mount() 錯誤\n${e.stack || e}</div>`;
  }
  if (inst.resize) {
    const ro = new ResizeObserver(() => inst.resize());
    ro.observe(body);
    unsubs.push(() => ro.disconnect());
  }
  mounted.set(id, { id, def, el, inst, unsubs });
  wireEditing(el, id, def);
}

async function mountAll() {
  const ids = Object.keys(pos());
  await Promise.all(ids.map((id, i) => mountWidget(id, i + 1)));
  updateSubscriptions();
  reportHits();
}

function unmountWidget(id) {
  const m = mounted.get(id);
  if (!m) return;
  try { m.inst.destroy?.(); } catch (e) { console.error(e); }
  m.unsubs.forEach(u => u());
  m.el.remove();
  mounted.delete(id);
}

// ---------- 編輯：拖曳 / 縮放 / 刪除 ----------

function wireEditing(el, id, def) {
  const min = def.minSize || { w: 80, h: 40 };

  el.querySelector('.w-del').addEventListener('click', e => {
    e.stopPropagation();
    unmountWidget(id);
    eachEntry(e => delete e.items[id]);   // 這個主題的每個螢幕版面都移除
    // 沒有任何主題用到它，就連設定一起刪掉
    const used = Object.values(layout.positions).some(scr => Object.values(scr).some(e => e.items?.[id]));
    if (!used) delete layout.widgets[id];
    saveLayout();
    updateSubscriptions();
  });

  const startDrag = (e, mode) => {
    if (!editing || e.button !== 0) return;
    if (mode === 'move' && e.target.closest('.w-del, .w-resize, .edit-live')) return;
    e.preventDefault();
    e.stopPropagation();
    const p = ownPos()[id];
    if (!p) return;
    const sx = e.clientX, sy = e.clientY;
    const orig = { ...p };
    const [maxW, maxH] = view.entry.ref;   // 座標都是畫布座標（縮放前）
    const s = view.s;
    el.classList.add('dragging');
    el.setPointerCapture(e.pointerId);
    const move = ev => {
      const dx = (ev.clientX - sx) / s, dy = (ev.clientY - sy) / s;
      if (mode === 'move') {
        p.x = Math.max(0, Math.min(maxW - p.w, snap(orig.x + dx)));
        p.y = Math.max(0, Math.min(maxH - p.h, snap(orig.y + dy)));
      } else {
        p.w = Math.max(min.w, Math.min(maxW - p.x, snap(orig.w + dx)));
        p.h = Math.max(min.h, Math.min(maxH - p.y, snap(orig.h + dy)));
      }
      place(el, p);
    };
    const up = () => {
      el.classList.remove('dragging');
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      el.removeEventListener('pointercancel', up);
      saveLayout();
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
  };
  el.addEventListener('pointerdown', e => startDrag(e, 'move'));
  el.querySelector('.w-resize').addEventListener('pointerdown', e => startDrag(e, 'resize'));
}

function setEditing(v, tellSwift = true) {
  editing = v;
  document.body.classList.toggle('editing', v);
  emit('editMode', v);
  if (tellSwift) call('setEditMode', v).catch(() => {});
  reportHits();
}
on('editMode', v => { if (v !== editing) setEditing(v, false); }, { replay: false });

async function addWidget(type) {
  let def;
  try { def = await loadDef(type); } catch (e) { console.error(e); return; }
  const size = def.defaultSize || { w: 300, h: 200 };
  const id = type + '-' + uid();
  layout.widgets[id] = { type, config: structuredClone(def.defaultConfig || {}) };
  ownPos();
  // 每個螢幕的版面都加進去，放在各自畫布正中間
  eachEntry(e => {
    const [W, H] = e.ref;
    e.items[id] = { x: snap((W - size.w) / 2), y: snap((H - size.h) / 2), w: size.w, h: size.h };
  });
  await mountWidget(id, Object.keys(pos()).length);
  saveLayout();
  updateSubscriptions();
}
on('addWidget', type => addWidget(type), { replay: false });

document.addEventListener('keydown', e => { if (e.key === 'Escape' && editing) setEditing(false); });

function setupEditBar() {
  const add = document.getElementById('eb-add');
  for (const t of available.widgets) add.append(new Option(t, t));
  add.addEventListener('change', () => { if (add.value) addWidget(add.value); add.value = ''; });
  const theme = document.getElementById('eb-theme');
  for (const t of available.themes) theme.append(new Option(`主題：${t}`, t));
  theme.value = layout.theme;
  theme.addEventListener('change', () => switchTheme(theme.value));
  document.getElementById('eb-done').addEventListener('click', () => setEditing(false));
}

// ---------- 啟動 ----------

async function boot() {
  let init;
  try { init = await call('ready'); }
  catch (e) {
    desktop.innerHTML = `<div class="w-error" style="position:absolute;top:60px;left:40px">${e.message}</div>`;
    return;
  }
  available = { widgets: init.widgets || [], themes: init.themes || [] };
  info = init.info || {};
  emit('info', info);

  // 深拷貝：Swift → JS 傳遞時，內容相同的物件（例如兩個空的 {}）可能變成同一個參照
  const l = JSON.parse(JSON.stringify(init.layout || {}));
  // 舊格式（widgets 是陣列）直接換成新格式
  layout = Array.isArray(l.widgets) || !l.positions
    ? { theme: DEFAULT_THEME, widgets: {}, positions: {} }
    : { theme: l.theme || DEFAULT_THEME, widgets: l.widgets || {}, positions: l.positions || {} };
  // 舊格式的位置（沒分螢幕）→ 當作在 LEGACY_REF 上排的
  for (const t of Object.keys(layout.positions)) layout.positions[t] = normalizeScreens(layout.positions[t]);

  setupEditBar();
  await switchTheme(layout.theme);
  call('log', `booted ${mounted.size} widgets, theme=${layout.theme}, screen=${view.key}, layout ref=${view.entry.ref.join("x")}, scale=${view.s.toFixed(3)}`).catch(() => {});
}

boot();
