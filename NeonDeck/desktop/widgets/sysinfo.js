// 一行系統資訊：macOS 版本、晶片、開機時間、IP、電源
import { h, injectCSS } from '../core/util.js';

injectCSS('sysinfo', `
:where(.si) { height: 100%; display: flex; align-items: center; gap: 16px; flex-wrap: wrap; white-space: nowrap; }
`);

export default {
  title: 'SYS.INFO',
  tag: 'INFO',
  defaultSize: { w: 400, h: 32 },
  minSize: { w: 160, h: 24 },
  channels: ['stats', 'slow'],
  defaultConfig: { hideHeader: true },

  mount(body, ctx) {
    const up = h('span'), ip = h('span'), pw = h('span');
    body.append(h('div.si.s', {}, h('span', {}, ctx.info.os || ''), h('span', {}, ctx.info.chip || ''), up, ip, pw));
    ctx.on('stats', s => {
      if (!s.uptime) return;
      const m = Math.floor(s.uptime / 60), d = Math.floor(m / 1440), hh = Math.floor(m % 1440 / 60);
      up.textContent = 'UP ' + (d ? `${d}d ${hh}h` : `${hh}h${String(m % 60).padStart(2, '0')}m`);
    });
    ctx.on('slow', s => {
      ip.textContent = s.ip || 'OFFLINE';
      pw.textContent = s.battery ? `${s.battery.percent}%${s.battery.charging ? ' ⚡' : ''}` : 'AC';
    });
  },
};
