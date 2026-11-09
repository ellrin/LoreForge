// 網速：下載 / 上傳 + 近 60 秒圖表。圖表樣式由主題的 --net-chart 決定：bars（柱狀）或 line（面積線）。
import { h, cssVar, fmtRate, fmtBytes, Ring, drawSpark, withAlpha, injectCSS } from '../core/util.js';

injectCSS('net', `
:where(.nt) { height: 100%; display: flex; flex-direction: column; gap: 8px; }
:where(.nt-speeds) { display: flex; gap: 24px; align-items: baseline; }
:where(.nt-chart) { flex: 1; min-height: 24px; position: relative; }
:where(.nt-chart canvas) { position: absolute; inset: 0; width: 100%; height: 100%; }
:where(.nt-foot) { display: flex; justify-content: space-between; }
`);

export default {
  title: 'NET.LINK',
  tag: 'NET',
  defaultSize: { w: 400, h: 190 },
  minSize: { w: 220, h: 90 },
  channels: ['stats'],

  mount(body, ctx) {
    const down = new Ring(60), up = new Ring(60);
    const dv = h('span.num.nt-down'), du = h('span.unit'), uv = h('span.num.nt-up'), uu = h('span.unit');
    const canvas = h('canvas');
    const rx = h('span'), tx = h('span'), peak = h('span');
    body.append(h('div.nt', {},
      h('div.nt-speeds', {},
        h('span.nt-d', {}, h('span.arrow', {}, '▼ '), dv, ' ', du),
        h('span.nt-u', {}, h('span.arrow', {}, '▲ '), uv, ' ', uu)),
      h('div.nt-chart.well', {}, canvas),
      h('div.nt-foot.s', {}, h('span', {}, 'Σ↓ ', rx), h('span', {}, 'PEAK ', peak), h('span', {}, 'Σ↑ ', tx))));

    let c1, c2, style;
    const readTheme = () => {
      c1 = cssVar(body, '--net-down', cssVar(body, '--h', '#2DE2FF'));
      c2 = cssVar(body, '--net-up', '#9DF0FF');
      style = cssVar(body, '--net-chart', 'line');
    };
    readTheme();

    function draw() {
      const top = Math.max(64 * 1024, down.max, up.max) * 1.1;
      peak.textContent = fmtRate(Math.max(down.max, up.max)).join(' ');
      if (style !== 'bars') {
        drawSpark(canvas, [{ data: down.a, color: c1, fill: true }, { data: up.a, color: c2, fill: false, width: 1 }], { max: top });
        return;
      }
      // 柱狀：最近 24 筆，越新越亮
      const dpr = devicePixelRatio || 1, w = canvas.clientWidth, hh = canvas.clientHeight;
      if (!w || !hh) return;
      canvas.width = w * dpr; canvas.height = hh * dpr;
      const g = canvas.getContext('2d');
      g.setTransform(dpr, 0, 0, dpr, 0, 0);
      const data = down.a.slice(-24), n = data.length, gap = 3, bw = (w - gap * (n - 1)) / n;
      data.forEach((v, i) => {
        const bh = Math.max(2, (v / top) * (hh - 2));
        g.fillStyle = withAlpha(c1, .35 + .65 * (i / (n - 1)));
        if (i === n - 1) { g.shadowColor = c1; g.shadowBlur = 10; }
        g.fillRect(i * (bw + gap), hh - bh, bw, bh);
        g.shadowBlur = 0;
      });
    }

    ctx.on('stats', s => {
      if (s.down == null) return;
      down.push(s.down); up.push(s.up);
      [dv.textContent, du.textContent] = fmtRate(s.down);
      [uv.textContent, uu.textContent] = fmtRate(s.up);
      rx.textContent = fmtBytes(s.rx); tx.textContent = fmtBytes(s.tx);
      draw();
    });
    ctx.on('theme', () => { readTheme(); draw(); });
    return { resize: draw };
  },
};
