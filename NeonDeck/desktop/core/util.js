// 小工具共用的小函式。自己寫小工具時可以 import 進來用。

/** 建立 DOM：h('div.cls#id', { style, onclick, ... }, ...children) */
export function h(tag, props = {}, ...children) {
  const [, name = 'div', rest = ''] = tag.match(/^([a-z0-9-]*)(.*)$/i);
  const el = document.createElement(name || 'div');
  for (const m of rest.matchAll(/([.#])([\w-]+)/g)) {
    if (m[1] === '.') el.classList.add(m[2]); else el.id = m[2];
  }
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else if (k in el && k !== 'list') el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) if (c != null && c !== false) el.append(c);
  return el;
}

/** 讀 CSS 變數（主題色），canvas 畫圖時用 */
export function cssVar(el, name, fallback = '') {
  return getComputedStyle(el).getPropertyValue(name).trim() || fallback;
}

/** bytes → [數字, 單位]，以 1024 進位 */
export function splitBytes(b, digits) {
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1000 && i < u.length - 1) { b /= 1024; i++; }
  const d = digits ?? (i === 0 ? 0 : b < 10 ? 2 : b < 100 ? 1 : 0);
  return [b.toFixed(d), u[i]];
}
export const fmtBytes = (b, d) => splitBytes(b, d).join(' ');
export const fmtRate = b => { const [n, u] = splitBytes(b); return [n, u + '/s']; };

/** 十進位 GB（與 Finder 顯示一致，用在硬碟容量） */
export const fmtGB = b => b >= 1e12 ? (b / 1e12).toFixed(2) + ' TB' : Math.round(b / 1e9) + ' GB';

export function fmtDuration(sec) {
  sec = Math.floor(sec);
  const d = Math.floor(sec / 86400), hh = Math.floor(sec % 86400 / 3600), mm = Math.floor(sec % 3600 / 60);
  const pad = n => String(n).padStart(2, '0');
  return d > 0 ? `${d}d ${pad(hh)}:${pad(mm)}` : `${pad(hh)}:${pad(mm)}:${pad(sec % 60)}`;
}

export const pad2 = n => String(n).padStart(2, '0');

/** 固定長度的歷史紀錄 */
export class Ring {
  constructor(n) { this.n = n; this.a = new Array(n).fill(0); }
  push(v) { this.a.push(v); if (this.a.length > this.n) this.a.shift(); }
  get max() { return Math.max(...this.a); }
}

/**
 * 在 canvas 上畫折線圖（自動處理 Retina 與尺寸）。
 * series: [{ data: number[], color, fill: true/false, width }]
 */
export function drawSpark(canvas, series, { max = null, min = 0, grid = null } = {}) {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth, hgt = canvas.clientHeight;
  if (!w || !hgt) return;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(hgt * dpr)) {
    canvas.width = Math.round(w * dpr); canvas.height = Math.round(hgt * dpr);
  }
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, hgt);

  const top = max ?? Math.max(1e-9, ...series.flatMap(s => s.data));
  const range = top - min || 1;

  if (grid) {
    ctx.strokeStyle = grid; ctx.lineWidth = 1;
    ctx.beginPath();
    for (let i = 1; i < 4; i++) { const y = Math.round(hgt * i / 4) + .5; ctx.moveTo(0, y); ctx.lineTo(w, y); }
    ctx.stroke();
  }

  for (const s of series) {
    const n = s.data.length;
    if (n < 2) continue;
    const step = w / (n - 1);
    ctx.beginPath();
    s.data.forEach((v, i) => {
      const y = hgt - Math.min(1, Math.max(0, (v - min) / range)) * (hgt - 1.5) - .75;
      i ? ctx.lineTo(i * step, y) : ctx.moveTo(0, y);
    });
    if (s.fill) {
      ctx.save();
      ctx.lineTo(w, hgt); ctx.lineTo(0, hgt); ctx.closePath();
      const g = ctx.createLinearGradient(0, 0, 0, hgt);
      g.addColorStop(0, withAlpha(s.color, .35)); g.addColorStop(1, withAlpha(s.color, 0));
      ctx.fillStyle = g; ctx.fill();
      ctx.restore();
      ctx.beginPath();
      s.data.forEach((v, i) => {
        const y = hgt - Math.min(1, Math.max(0, (v - min) / range)) * (hgt - 1.5) - .75;
        i ? ctx.lineTo(i * step, y) : ctx.moveTo(0, y);
      });
    }
    ctx.strokeStyle = s.color; ctx.lineWidth = s.width ?? 1.25; ctx.lineJoin = 'round';
    ctx.stroke();
  }
}

/** '#00f0ff' / 'rgb(...)' → 帶透明度的 rgba */
export function withAlpha(color, a) {
  const c = color.trim();
  if (c.startsWith('#')) {
    let hex = c.slice(1);
    if (hex.length === 3) hex = [...hex].map(x => x + x).join('');
    const n = parseInt(hex.slice(0, 6), 16);
    return `rgba(${n >> 16 & 255},${n >> 8 & 255},${n & 255},${a})`;
  }
  const m = c.match(/rgba?\(([^)]+)\)/);
  if (m) { const [r, g, b] = m[1].split(',').map(s => s.trim()); return `rgba(${r},${g},${b},${a})`; }
  return c;
}

/** 分段式進度條：stops = [{ to: 0~1, color }]（累積比例） */
export function segBar(el, stops, segments = 32) {
  if (el.childElementCount !== segments) {
    el.replaceChildren(...Array.from({ length: segments }, () => h('i')));
  }
  [...el.children].forEach((seg, i) => {
    const t = (i + .5) / segments;
    const s = stops.find(s => t < s.to);
    seg.style.background = s ? s.color : '';
    seg.classList.toggle('on', !!s);
  });
}

/** 小工具注入自己的「結構」CSS（只注入一次）。請用 :where() 包住選擇器，讓主題可以輕鬆覆蓋。 */
export function injectCSS(id, css) {
  if (document.getElementById('css-' + id)) return;
  document.head.prepend(h('style#css-' + id, { textContent: css }));
}
