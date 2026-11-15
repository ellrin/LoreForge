// 鬧鐘：一排膠囊。點膠囊開關、懸停出現 ✕ 刪除、＋ 打開新增面板。
// 響鈴時：系統音效 + 系統通知 + 畫面中央的響鈴面板（貪睡 / 關閉）。
import { h, pad2, injectCSS } from '../core/util.js';

injectCSS('alarm', `
:where(.al) { height: 100%; display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
:where(.al-chip) { position: relative; }
.al-chip > .x { display: none; position: absolute; top: -8px; right: -8px; width: 16px; height: 16px; padding: 0; border-radius: 50%; font-size: 9px; line-height: 16px; }
.al-chip:hover > .x { display: block; }
:where(.al-pop) { position: fixed; z-index: 10001; display: flex; flex-direction: column; gap: 10px; padding: 16px; width: 280px; }
:where(.al-pop .row) { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
:where(.al-pop input[type=text]) { flex: 1; min-width: 0; }
:where(.al-ring) { position: fixed; z-index: 10002; left: 50%; top: 50%; transform: translate(-50%, -50%); display: flex; flex-direction: column; align-items: center; gap: 14px; padding: 28px 40px; }
`);

const DAYS = ['日', '一', '二', '三', '四', '五', '六'];
const dayText = d => d.length === 7 ? '每天' : d.join() === '1,2,3,4,5' ? '平日' : d.join() === '0,6' ? '週末' : d.length ? d.map(x => DAYS[x]).join('') : '單次';

export default {
  title: 'ALARM',
  tag: 'ALM',
  defaultSize: { w: 420, h: 44 },
  minSize: { w: 120, h: 36 },
  defaultConfig: { hideHeader: true, alarms: [{ id: 'a1', time: '07:30', label: '起床', days: [1, 2, 3, 4, 5], on: true }], sound: 'Glass', snoozeMin: 5 },

  mount(body, ctx) {
    const cfg = ctx.config;
    cfg.alarms ||= [];
    const strip = h('div.al');
    body.append(strip);

    function render() {
      const sorted = [...cfg.alarms].sort((a, b) => a.time.localeCompare(b.time));
      strip.replaceChildren(
        ...sorted.map(a => h(`button.al-chip.chip${a.on ? '.on' : '.off'}.edit-live`, {
          title: a.label || '鬧鐘',
          onclick: e => { if (e.target.closest('.x')) return; a.on = !a.on; ctx.save(); render(); },
        }, h('span', {}, `${a.on ? 'ALARM ' : ''}${a.time} ${dayText(a.days)}`),
        h('span.x.btn', { onclick: () => { cfg.alarms = cfg.alarms.filter(x => x !== a); ctx.save(); render(); } }, '✕'))),
        h('button.al-chip.chip.add.edit-live', { onclick: openPop, title: '新增鬧鐘' }, h('span', {}, '＋')),
      );
    }

    // ---- 新增面板（浮在小工具上方，用 hit-extra 讓它能被點）----
    let pop;
    function openPop() {
      if (pop) return closePop();
      const timeIn = h('input', { type: 'time', value: '07:30' });
      const labelIn = h('input', { type: 'text', placeholder: '標籤（選填）' });
      const days = new Set();
      const dayBtns = DAYS.map((d, i) => h('button.chip.mini', {
        onclick: e => { days.has(i) ? days.delete(i) : days.add(i); e.currentTarget.classList.toggle('on'); },
      }, h('span', {}, d)));
      pop = h('div.al-pop.panel-pop.hit-extra', {},
        h('div.row', {}, timeIn, labelIn),
        h('div.row', {}, dayBtns),
        h('div.row', { style: { justifyContent: 'flex-end' } },
          h('button.btn', { onclick: closePop }, '取消'),
          h('button.btn.primary', {
            onclick: () => {
              if (!timeIn.value) return;
              cfg.alarms.push({ id: Date.now().toString(36), time: timeIn.value, label: labelIn.value.trim(), days: [...days].sort(), on: true });
              ctx.save(); render(); closePop();
            },
          }, '加入')));
      const r = ctx.el.getBoundingClientRect();
      pop.style.left = Math.max(8, Math.min(innerWidth - 300, r.left)) + 'px';
      pop.style.top = (r.top > 220 ? r.top - 170 : r.bottom + 10) + 'px';
      document.body.append(pop);
      ctx.refreshHits();
      timeIn.focus();
    }
    function closePop() { pop?.remove(); pop = null; ctx.refreshHits(); }

    // ---- 響鈴 ----
    let ringTimer, ringEl;
    const fired = new Set();
    const snoozes = [];

    function ring(a, text) {
      stopRing();
      ringEl = h('div.al-ring.panel-pop.alerting.hit-extra', {},
        h('div.al-ring-time', {}, a.time),
        h('div', {}, text),
        h('div', { style: { display: 'flex', gap: '10px' } },
          h('button.btn', { onclick: () => snooze(a) }, `貪睡 ${cfg.snoozeMin} 分`),
          h('button.btn.primary', { onclick: stopRing }, '關閉')));
      document.body.append(ringEl);
      ctx.refreshHits();
      ctx.call('notify', { title: '⏰ ' + (a.label || '鬧鐘'), body: a.time, sound: cfg.sound });
      let n = 0;
      ringTimer = setInterval(() => { if (++n > 36) stopRing(); else ctx.call('notify', { sound: cfg.sound, title: '', body: '' }).catch(() => {}); }, 5000);
    }
    function stopRing() { clearInterval(ringTimer); ringEl?.remove(); ringEl = null; ctx.refreshHits(); }
    function snooze(a) {
      stopRing();
      const t = new Date(Date.now() + cfg.snoozeMin * 60000);
      snoozes.push({ at: `${pad2(t.getHours())}:${pad2(t.getMinutes())}`, alarm: a });
    }

    const check = () => {
      const d = new Date();
      const hm = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
      const key = `${d.toDateString()} ${hm}`;
      for (const a of cfg.alarms) {
        if (!a.on || a.time !== hm || fired.has(a.id + key)) continue;
        if (a.days.length && !a.days.includes(d.getDay())) continue;
        fired.add(a.id + key);
        if (!a.days.length) { a.on = false; ctx.save(); render(); }
        ring(a, a.label || '時間到');
      }
      for (let i = snoozes.length - 1; i >= 0; i--) {
        if (snoozes[i].at === hm) ring(snoozes.splice(i, 1)[0].alarm, '貪睡結束');
      }
    };
    const iv = setInterval(check, 15000);
    render();
    check();
    return { destroy: () => { clearInterval(iv); stopRing(); closePop(); } };
  },
};
