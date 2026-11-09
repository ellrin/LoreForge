// 提醒事項（與 macOS / iPhone「提醒事項」雙向同步）
import { h, pad2, injectCSS } from '../core/util.js';

injectCSS('reminders', `
:where(.rm) { height: 100%; display: flex; flex-direction: column; gap: 8px; }
:where(.rm-list) { flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: 5px; }
:where(.rm-item) { display: grid; grid-template-columns: auto 1fr auto; gap: 8px; align-items: center; }
:where(.rm-item .t) { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
:where(.rm-item.done .t) { text-decoration: line-through; opacity: .4; }
:where(.rm-check) { width: 14px; height: 14px; padding: 0; }
:where(.rm-add) { display: flex; gap: 6px; }
:where(.rm-add input) { flex: 1; min-width: 0; }
`);

export default {
  title: 'REMINDERS',
  defaultSize: { w: 320, h: 300 },
  minSize: { w: 220, h: 140 },
  defaultConfig: { lists: [], max: 50 },   // lists: 只顯示這些清單名稱（空 = 全部）

  mount(body, ctx) {
    let access = 'unknown';
    let items = [];
    const list = h('div.rm-list');
    const input = h('input.edit-live', {
      type: 'text', placeholder: '新增提醒…（Enter）',
      onkeydown: e => { if (e.key === 'Enter' && !e.isComposing) add(); },
    });
    body.append(h('div.rm', {}, list, h('div.rm-add', {}, input)));

    async function load() {
      if (access !== 'granted') access = await ctx.call('access', 'reminders').catch(() => 'denied');
      items = access === 'granted' ? await ctx.call('reminders').catch(() => []) : [];
      render();
    }

    async function add() {
      const title = input.value.trim();
      if (!title) return;
      input.value = '';
      await ctx.call('addReminder', { title });
      load();
    }

    function render() {
      if (access !== 'granted') {
        list.replaceChildren(h('div.small.dim', {}, '沒有提醒事項權限。請到「系統設定 ▸ 隱私權與安全性 ▸ 提醒事項」允許 NeonDeck。'),
          h('button.btn.edit-live', { onclick: () => { access = 'unknown'; load(); } }, '重新要求權限'));
        return;
      }
      const filter = ctx.config.lists || [];
      const now = Date.now();
      const shown = items
        .filter(r => !filter.length || filter.includes(r.list))
        .sort((a, b) => (a.due ?? Infinity) - (b.due ?? Infinity))
        .slice(0, ctx.config.max || 50);
      ctx.setTitle(`${ctx.config.title ?? 'REMINDERS'} · ${shown.length}`);

      list.replaceChildren(...(shown.length ? shown.map(r => {
        const row = h('div.rm-item');
        const check = h('button.rm-check.check.edit-live', {
          onclick: async () => {
            row.classList.add('done');
            const ok = await ctx.call('setReminder', { id: r.id, completed: true });
            if (!ok) row.classList.remove('done');
          },
        });
        let due = '';
        if (r.due) {
          const d = new Date(r.due);
          due = `${d.getMonth() + 1}/${d.getDate()}` + (r.dueHasTime ? ` ${pad2(d.getHours())}:${pad2(d.getMinutes())}` : '');
        }
        const dueCls = r.due && r.due < now ? 'span.small.danger' : 'span.small.dim';
        row.append(check, h('span.t', { title: r.list }, r.title), h(dueCls, {}, due));
        row.style.setProperty('--list-color', r.color || '');
        return row;
      }) : [h('div.small.dim', {}, '全部完成 ✓')]));
    }

    ctx.on('remindersChanged', load, { replay: false });
    load();
  },
};
