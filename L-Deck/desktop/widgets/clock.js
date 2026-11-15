// 時鐘。V2 / V3 的裝飾都在 DOM 裡（刻度環、秒數斜格、日期色塊），由主題 CSS 決定顯示哪些、怎麼排。
import { h, pad2, injectCSS } from '../core/util.js';

injectCSS('clock', `
:where(.ck) { position: relative; width: 100%; height: 100%; container-type: size; }
:where(.ck-rings) { position: absolute; inset: 0; width: 100%; height: 100%; display: none; overflow: visible; }
:where(.ck-stack) { position: relative; height: 100%; display: flex; flex-direction: column; justify-content: center; gap: 8px; }
:where(.ck-dates), :where(.ck-bottom) { display: flex; align-items: center; gap: 10px; }
:where(.ck-time) { position: relative; line-height: 1; white-space: nowrap; }
:where(.ck-segs) { display: none; gap: 5px; }
:where(.ck-segs i) { width: 30px; height: 9px; }
`);

const WD = ['SUN', 'MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT'];
const MON = ['JAN', 'FEB', 'MAR', 'APR', 'MAY', 'JUN', 'JUL', 'AUG', 'SEP', 'OCT', 'NOV', 'DEC'];
const SVGNS = 'http://www.w3.org/2000/svg';
const svg = (tag, attrs) => { const e = document.createElementNS(SVGNS, tag); for (const k in attrs) e.setAttribute(k, attrs[k]); return e; };

export default {
  title: 'CHRONO',
  tag: 'TIME',
  defaultSize: { w: 620, h: 620 },
  minSize: { w: 200, h: 120 },
  defaultConfig: { hideHeader: true, hour12: false, zone: '' },

  mount(body, ctx) {
    // ---- V2：刻度環（秒數走黃色弧線）----
    const R = 246, C = 2 * Math.PI * R;
    const rings = svg('svg', { class: 'ck-rings', viewBox: '0 0 620 620', 'aria-hidden': 'true' });
    const g = svg('g', { transform: 'rotate(-90 310 310)' });
    g.append(
      svg('circle', { class: 'r-outer', cx: 310, cy: 310, r: 296, fill: 'none' }),
      svg('circle', { class: 'r-ticks', cx: 310, cy: 310, r: 272, fill: 'none', 'stroke-dasharray': '2 26.48' }),
      svg('circle', { class: 'r-hours', cx: 310, cy: 310, r: 272, fill: 'none', 'stroke-dasharray': '4 138.42' }),
      svg('circle', { class: 'r-track', cx: 310, cy: 310, r: R, fill: 'none' }),
    );
    const secArc = svg('circle', { class: 'r-sec', cx: 310, cy: 310, r: R, fill: 'none', 'stroke-dasharray': `0 ${C}` });
    g.append(secArc, svg('circle', { class: 'r-inner', cx: 310, cy: 310, r: 226, fill: 'none', 'stroke-dasharray': '120 40 30 40' }));
    rings.append(g, svg('path', { class: 'r-cross', d: 'M60 310h40M520 310h40M310 60v26M310 534v26' }));

    // ---- 文字 ----
    const zone = h('span.ck-zone');
    const wd = h('span.ck-chip.ck-wd'), day = h('span.ck-chip.ck-day');
    const secA = h('span.ck-sec.ck-sec-a');
    const hm = h('span.ck-hm');
    const ap = h('span.ck-ap');
    const segs = h('div.ck-segs', {}, Array.from({ length: 12 }, () => h('i')));
    const secB = h('span.ck-sec.ck-sec-b');
    const zone2 = h('span.ck-zone.ck-zone-b');

    body.append(h('div.ck', {}, rings, h('div.ck-stack', {},
      zone,
      h('div.ck-dates', {}, wd, day, zone2, secA),
      h('div.ck-time', {}, hm, ap),
      h('div.ck-bottom', {}, segs, secB),
    )));

    const tzName = ctx.config.zone || (() => {
      const off = -new Date().getTimezoneOffset() / 60;
      const city = (Intl.DateTimeFormat().resolvedOptions().timeZone || '').split('/').pop()?.slice(0, 3).toUpperCase();
      return `${city} ${off >= 0 ? '+' : ''}${pad2(off)}`;
    })();
    zone.textContent = `LOCAL // ${tzName}`;
    zone2.textContent = tzName;

    let t;
    const tick = () => {
      const d = new Date();
      let hr = d.getHours();
      if (ctx.config.hour12) { ap.textContent = hr < 12 ? 'AM' : 'PM'; hr = hr % 12 || 12; } else ap.textContent = '';
      const text = `${pad2(hr)}:${pad2(d.getMinutes())}`;
      if (hm.textContent !== text) { hm.textContent = text; hm.dataset.text = text; }   // data-text 給主題做故障 / 描邊複本
      const s = d.getSeconds();
      secA.textContent = secB.textContent = ':' + pad2(s);
      secArc.setAttribute('stroke-dasharray', `${(C * s / 60).toFixed(1)} ${C}`);
      [...segs.children].forEach((seg, i) => {
        seg.className = i < Math.floor(s / 5) ? 'on' : i === Math.floor(s / 5) ? 'half' : '';
      });
      wd.textContent = WD[d.getDay()];
      day.textContent = ctx.theme === 'v2-center'
        ? `${d.getFullYear()}.${pad2(d.getMonth() + 1)}.${pad2(d.getDate())}`
        : `${pad2(d.getDate())} ${MON[d.getMonth()]} ${d.getFullYear()}`;
      t = setTimeout(tick, 1000 - d.getMilliseconds() + 5);
    };
    tick();
    ctx.on('theme', () => { clearTimeout(t); tick(); }, { replay: false });
    return { destroy: () => clearTimeout(t) };
  },
};
