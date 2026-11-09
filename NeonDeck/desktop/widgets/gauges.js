// CPU / 記憶體 / 儲存空間 三個儀表。V2 顯示成環形，V3 顯示成斜膠囊（主題 CSS 決定）。
import { h, fmtGB, injectCSS } from '../core/util.js';

injectCSS('gauges', `
:where(.gg) { height: 100%; display: flex; justify-content: space-between; align-items: center; gap: 10px; }
:where(.gg-item) { flex: 1; display: flex; flex-direction: column; align-items: center; gap: 6px; }
:where(.gg-dial) { position: relative; width: 96px; height: 96px; }
:where(.gg-dial svg) { width: 100%; height: 100%; }
:where(.gg-center) { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; }
`);

const R = 38, C = 2 * Math.PI * R;
const NS = 'http://www.w3.org/2000/svg';
const svg = (tag, a) => { const e = document.createElementNS(NS, tag); for (const k in a) e.setAttribute(k, a[k]); return e; };

function dial() {
  const s = svg('svg', { viewBox: '0 0 96 96', 'aria-hidden': 'true' });
  const g = svg('g', { transform: 'rotate(-90 48 48)' });
  const arc = svg('circle', { class: 'gg-arc', cx: 48, cy: 48, r: R, fill: 'none', 'stroke-dasharray': `0 ${C}` });
  g.append(svg('circle', { class: 'gg-track', cx: 48, cy: 48, r: R, fill: 'none' }), arc,
    svg('circle', { class: 'gg-dots', cx: 48, cy: 48, r: 46, fill: 'none', 'stroke-dasharray': '2 4' }));
  s.append(g);
  return { s, set: f => arc.setAttribute('stroke-dasharray', `${(C * Math.max(0, Math.min(1, f))).toFixed(1)} ${C}`) };
}

export default {
  title: 'CORE',
  tag: 'SYS',
  defaultSize: { w: 400, h: 200 },
  minSize: { w: 220, h: 60 },
  channels: ['stats', 'disk'],

  mount(body, ctx) {
    const items = ['cpu', 'mem', 'ssd'].map(k => {
      const d = dial();
      const val = h('span.gg-val.num'), sub = h('span.gg-sub'), lbl = h('span.gg-lbl');
      const key = h('span.gg-key', {}, k.toUpperCase());   // 精簡模式（V3 膠囊）用的短標籤
      const el = h(`div.gg-item.gg-${k}`, {}, key, h('div.gg-dial', {}, d.s, h('div.gg-center', {}, val, sub)), lbl);
      return { k, el, d, val, sub, lbl };
    });
    const [cpu, mem, ssd] = items;
    cpu.lbl.textContent = 'CPU'; mem.lbl.textContent = 'MEM'; ssd.lbl.textContent = 'SSD';
    body.append(h('div.gg', {}, items.map(i => i.el)));

    ctx.on('stats', s => {
      if (s.cpu != null) {
        cpu.val.textContent = Math.round(s.cpu * 100) + '%';
        cpu.d.set(s.cpu);
        cpu.el.dataset.level = s.cpu > .85 ? 'danger' : s.cpu > .7 ? 'warn' : 'ok';
      }
      if (s.mem) {
        const f = s.mem.used / s.mem.total;
        mem.val.textContent = (s.mem.used / 2 ** 30).toFixed(1) + 'G';
        mem.sub.textContent = `/ ${Math.round(s.mem.total / 2 ** 30)}G`;
        mem.lbl.textContent = `MEM ${Math.round(f * 100)}%`;
        mem.d.set(f);
        mem.el.dataset.level = f > .9 ? 'danger' : f > .75 ? 'warn' : 'ok';
      }
    });
    ctx.on('disk', d => {
      if (!d.total) return;
      ssd.val.textContent = fmtGB(d.free).replace(' ', '');
      ssd.sub.textContent = '可用';
      ssd.lbl.textContent = `SSD / ${fmtGB(d.total)}`;
      ssd.d.set(1 - d.free / d.total);
    });
  },
};
