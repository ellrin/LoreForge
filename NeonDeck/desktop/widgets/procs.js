// 程序排行：CPU「正在處理」/ 記憶體大戶（依 App 合併）。
// 點一下 → 展開說明（這是什麼、為什麼佔用）；雙擊 → 切到那個 App。
// 滑鼠停在上面時暫停更新，列表不會跳來跳去。
import { h, fmtBytes, injectCSS } from '../core/util.js';
import { explain, shortWhat } from '../core/procinfo.js';

injectCSS('procs', `
:where(.pr) { height: 100%; display: flex; flex-direction: column; gap: 6px; }
:where(.pr-tabs) { display: flex; gap: 6px; align-items: center; }
:where(.pr-list) { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
:where(.pr-row) { display: grid; grid-template-columns: 20px minmax(0, 1fr) 70px 58px; gap: 8px; align-items: center; padding: 4px 8px; cursor: pointer; }
:where(.pr-name) { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
:where(.pr-val) { text-align: right; }
:where(.pr-bar) { height: 5px; padding: 1px; }
:where(.pr-bar i) { display: block; height: 100%; }
:where(.pr-detail) { display: none; grid-column: 2 / -1; line-height: 1.5; }
:where(.pr-row.sel .pr-detail) { display: block; }
:where(.pr-row.sel .pr-bar) { visibility: hidden; }
:where(.pr-acts) { display: flex; gap: 6px; margin-top: 6px; flex-wrap: wrap; }
`);

export default {
  title: 'PROC.TOP',
  tag: 'PROC',
  defaultSize: { w: 400, h: 300 },
  minSize: { w: 260, h: 120 },
  channels: ['procs'],
  defaultConfig: { mode: 'tabs', rows: 5 },   // mode: tabs（CPU / MEM 切換）| cpu | mem

  mount(body, ctx) {
    const cfg = ctx.config;
    let tab = cfg.mode === 'tabs' ? (cfg.tab || 'cpu') : cfg.mode;
    let selected = null;      // cpu: pid；mem: name
    let last = null;
    let hovering = false;

    const list = h('div.pr-list');
    const status = h('span.s.pr-status');
    const tabBtn = k => h(`button.chip.mini.edit-live${tab === k ? '.on' : ''}`, {
      onclick: () => { tab = cfg.tab = k; selected = null; ctx.save(); renderTabs(); render(); },
    }, h('span', {}, k.toUpperCase()));
    const tabs = h('div.pr-tabs');
    function renderTabs() {
      if (cfg.mode === 'tabs') tabs.replaceChildren(status, h('span', { style: { flex: 1 } }), tabBtn('cpu'), tabBtn('mem'));
      else tabs.replaceChildren(status);
    }
    renderTabs();
    body.append(h('div.pr', {}, tabs, list));
    body.addEventListener('mouseenter', () => { hovering = true; status.textContent = '⏸ 暫停更新'; });
    body.addEventListener('mouseleave', () => { hovering = false; render(); });

    const open = r => ctx.call('activateApp', { appPath: r.appPath, pid: r.pid });

    function row(i, r, kind, max) {
      const isCPU = kind === 'cpu';
      const key = isCPU ? r.pid : r.name;
      const val = isCPU ? r.cpu.toFixed(1) + '%' : fmtBytes(r.mem, r.mem > 1e9 ? 1 : 0);
      const frac = isCPU ? r.cpu / max : r.mem / max;
      const display = isCPU ? r.name.replace(/^Google Chrome /, 'Chrome ') : r.name;
      const sub = isCPU ? shortWhat(r.name, r.app) : (r.appPath ? `×${r.count} 程序` : explain(r.name, null).what);
      const e = explain(r.name, isCPU ? r.app : (r.appPath ? r.name : null));

      const acts = [];
      if (r.appPath) acts.push(h('button.chip.mini.on.edit-live', { onclick: ev => { ev.stopPropagation(); open(r); } }, h('span', {}, `切到 ${isCPU ? r.app : r.name}`)));
      if (r.path || r.appPath) acts.push(h('button.chip.mini.edit-live', { onclick: ev => { ev.stopPropagation(); ctx.call('revealInFinder', r.appPath || r.path); } }, h('span', {}, 'Finder')));
      acts.push(h('button.chip.mini.edit-live', { onclick: ev => { ev.stopPropagation(); ctx.call('activityMonitor'); } }, h('span', {}, '活動監視器')));

      const detail = h('div.pr-detail.s', {},
        isCPU
          ? [r.app ? h('span', {}, '屬於 ', h('b.hl', {}, r.app), ' · ') : '', e.what, h('br'), e.why ? e.why + ' · ' : '', `記憶體 ${fmtBytes(r.mem, 0)} · PID ${r.pid}`]
          : [r.appPath ? `${r.count} 個程序合計` : e.what, e.why ? h('span', {}, h('br'), e.why) : ''],
        h('div.pr-acts', {}, acts));

      return h(`div.pr-row.edit-live${selected === key ? '.sel' : ''}`, {
        title: isCPU ? r.path : (r.appPath || r.name),
        onclick: () => { selected = selected === key ? null : key; render(true); },
        ondblclick: () => { if (r.appPath) open(r); },
      },
      h('span.pr-rank.num', {}, String(i + 1).padStart(2, '0')),
      h('span.pr-name', {}, display, h('span.s', {}, ' · ' + sub)),
      h('span.pr-bar.well', {}, h('i', { style: { width: Math.max(2, frac * 100) + '%' } })),
      h('span.pr-val', {}, val),
      detail);
    }

    function render(force = false) {
      if (!last || (hovering && !force)) return;
      if (!hovering) status.textContent = tab === 'cpu' ? '正在處理 · 近 3 秒' : '依 App 合併';
      const kind = tab;
      const rows = (kind === 'cpu' ? last.cpu : last.mem).slice(0, cfg.rows || 5);
      if (!rows.length) { list.replaceChildren(h('div.s', {}, last.ready ? '目前很閒 ✓' : '取樣中…')); return; }
      const max = Math.max(...rows.map(r => kind === 'cpu' ? r.cpu : r.mem));
      list.replaceChildren(...rows.map((r, i) => row(i, r, kind, max)));
    }

    ctx.on('procs', p => { last = p; render(); });
  },
};
