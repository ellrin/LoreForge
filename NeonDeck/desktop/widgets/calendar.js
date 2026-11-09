// 月曆 + 行程（讀取 macOS「行事曆」，包含 iCloud / Google 等已登入的帳號）
import { h, pad2, injectCSS } from '../core/util.js';

injectCSS('calendar', `
:where(.cal) { height: 100%; display: flex; flex-direction: column; gap: 8px; }
:where(.cal-nav) { display: flex; align-items: center; justify-content: space-between; }
:where(.cal-grid) { display: grid; grid-template-columns: repeat(7, 1fr); gap: 2px; text-align: center; }
:where(.cal-grid > .day) { position: relative; padding: 4px 0 7px; cursor: pointer; }
:where(.cal-grid > .day.other) { opacity: .25; }
:where(.cal-grid > .day .dots) { position: absolute; left: 0; right: 0; bottom: 2px; display: flex; justify-content: center; gap: 2px; }
:where(.cal-grid > .day .dots i) { width: 3px; height: 3px; border-radius: 50%; }
:where(.cal-list) { flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: 4px; }
:where(.cal-ev) { display: grid; grid-template-columns: 3px auto 1fr; gap: 8px; align-items: center; }
:where(.cal-ev .bar) { align-self: stretch; }
:where(.cal-ev .t) { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
`);

const WEEK = ['日', '一', '二', '三', '四', '五', '六'];
const sameDay = (a, b) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
const dayKey = d => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;

export default {
  title: 'CALENDAR',
  defaultSize: { w: 420, h: 420 },
  minSize: { w: 260, h: 220 },
  defaultConfig: { weekStart: 0, upcomingDays: 7 },

  mount(body, ctx) {
    const ws = ctx.config.weekStart || 0;
    let view = new Date(); view.setDate(1);
    let selected = new Date();
    let events = [];
    let access = 'unknown';

    const title = h('span.mid');
    const grid = h('div.cal-grid');
    const listTitle = h('div.small.dim');
    const list = h('div.cal-list');
    const nav = h('div.cal-nav', {},
      h('button.btn.ghost.edit-live', { onclick: () => { view.setMonth(view.getMonth() - 1); load(); } }, '◀'),
      title,
      h('button.btn.ghost.edit-live', { onclick: () => { view = new Date(); view.setDate(1); selected = new Date(); load(); } }, '今天'),
      h('button.btn.ghost.edit-live', { onclick: () => { view.setMonth(view.getMonth() + 1); load(); } }, '▶'));
    body.append(h('div.cal', {}, nav, grid, listTitle, list));

    async function load() {
      if (access !== 'granted') {
        access = await ctx.call('access', 'calendar').catch(() => 'denied');
      }
      if (access === 'granted') {
        const start = new Date(view.getFullYear(), view.getMonth(), 1 - 7);
        const end = new Date(view.getFullYear(), view.getMonth() + 1, 7 + (ctx.config.upcomingDays || 7));
        const now = new Date();
        const s = Math.min(start, now), e = Math.max(end, new Date(now.getTime() + 86400000 * (ctx.config.upcomingDays || 7)));
        events = await ctx.call('events', { start: s, end: e }).catch(() => []);
      }
      render();
    }

    function render() {
      title.textContent = `${view.getFullYear()} / ${pad2(view.getMonth() + 1)}`;
      const byDay = new Map();
      for (const ev of events) {
        // 多天的行程每天都標一個點
        const d = new Date(ev.start); d.setHours(0, 0, 0, 0);
        const endD = new Date(ev.end - 1);
        while (d <= endD) { const k = dayKey(d); if (!byDay.has(k)) byDay.set(k, []); byDay.get(k).push(ev); d.setDate(d.getDate() + 1); }
      }

      const cells = [];
      for (let i = 0; i < 7; i++) cells.push(h('div.wd.small.dim', {}, WEEK[(i + ws) % 7]));
      const first = new Date(view);
      first.setDate(1 - ((first.getDay() - ws + 7) % 7));
      const today = new Date();
      for (let i = 0; i < 42; i++) {
        const d = new Date(first); d.setDate(first.getDate() + i);
        if (i >= 35 && d.getMonth() !== view.getMonth()) break;
        const evs = byDay.get(dayKey(d)) || [];
        const cls = ['day', d.getMonth() !== view.getMonth() && 'other', sameDay(d, today) && 'today',
          sameDay(d, selected) && 'selected', (d.getDay() === 0 || d.getDay() === 6) && 'weekend'].filter(Boolean).join('.');
        cells.push(h('div.' + cls + '.edit-live', { onclick: () => { selected = d; render(); } },
          String(d.getDate()),
          h('span.dots', {}, evs.slice(0, 3).map(e => h('i', { style: { background: e.color || 'currentColor' } })))));
      }
      grid.replaceChildren(...cells);

      if (access !== 'granted') {
        listTitle.textContent = '';
        list.replaceChildren(h('div.small.dim', {},
          access === 'notDetermined' ? '需要行事曆權限' : '沒有行事曆權限。請到「系統設定 ▸ 隱私權與安全性 ▸ 行事曆」允許 NeonDeck。'),
          h('button.btn.edit-live', { onclick: () => { access = 'unknown'; load(); } }, '重新要求權限'));
        return;
      }

      // 選了今天 → 顯示接下來 N 天；選了其他天 → 只顯示那天
      const isToday = sameDay(selected, today);
      let shown;
      if (isToday) {
        const limit = Date.now() + 86400000 * (ctx.config.upcomingDays || 7);
        shown = events.filter(e => e.end > Date.now() && e.start < limit);
        listTitle.textContent = `UPCOMING · ${ctx.config.upcomingDays || 7} 天`;
      } else {
        shown = byDay.get(dayKey(selected)) || [];
        listTitle.textContent = `${selected.getMonth() + 1}/${selected.getDate()}（${WEEK[selected.getDay()]}）`;
      }
      shown.sort((a, b) => a.start - b.start);
      list.replaceChildren(...(shown.length ? shown.map(e => {
        const s = new Date(e.start);
        const when = e.allDay ? '全天' : `${pad2(s.getHours())}:${pad2(s.getMinutes())}`;
        const prefix = isToday && !sameDay(s, today) ? `${s.getMonth() + 1}/${s.getDate()} ` : '';
        return h('div.cal-ev.small', { title: [e.title, e.location, e.calendar].filter(Boolean).join('\n') },
          h('span.bar', { style: { background: e.color || 'currentColor' } }),
          h('span.dim', {}, prefix + when),
          h('span.t', {}, e.title));
      }) : [h('div.small.dim', {}, '沒有行程')]));
    }

    ctx.on('calendarChanged', load, { replay: false });
    // 跨日時自動更新「今天」
    const iv = setInterval(() => { if (!sameDay(selected, new Date()) && sameDay(selected, new Date(Date.now() - 60000))) { selected = new Date(); load(); } }, 60000);
    load();
    return { destroy: () => clearInterval(iv) };
  },
};
