// 知識卡輪播：從知識庫挑「最近新增」和「學習中、還不熟」的卡，每分鐘換一次。
// 資料由 Swift 端直接讀 private/KnowledgeBase 的 JSON（ctx.call('kbCards')），不需要 kb-server；
// Swift 端有快取（看資料夾修改時間），沒變時不讀檔也不重傳，所以每 5 分鐘問一次很便宜。
// 越新、連續記得次數越少的卡越常出現；一輪全部輪完才重新洗牌，不會一直出現同一張。
// 一次顯示幾張：config.count = 'auto'（依小工具高度 1 或 2 張）或 1 / 2。
// 點卡片 → 開 KnowledgeBase App 並進到那張卡（lore-kb://node/<id>；App 沒開會自動啟動）。
import { h, injectCSS } from '../core/util.js';

injectCSS('cards', `
:where(.kc) { height: 100%; display: flex; flex-direction: column; gap: 14px; min-height: 0; }
:where(.kc-card) { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 4px; padding: 0;
  cursor: pointer; text-align: left; border: 0; background: none; color: inherit; font: inherit; overflow: hidden; }
:where(.kc-card:focus-visible) { outline: 1px solid currentColor; outline-offset: 2px; }
:where(.kc-head) { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
:where(.kc-t) { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
:where(.kc-badge) { flex: none; }
:where(.kc-al) { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
:where(.kc-sum) { line-height: 1.45; overflow: hidden; overflow-wrap: anywhere; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: var(--kc-lines, 4); }
:where(.kc-off) { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
`);

const DAY = 86400000;

export default {
  title: 'KB.CARDS',
  tag: 'KB',
  defaultSize: { w: 400, h: 190 },
  minSize: { w: 220, h: 110 },
  defaultConfig: { count: 'auto', rotateSec: 60, newDays: 14 },

  mount(body, ctx) {
    const cfg = ctx.config;
    let online = null;          // false = 找不到知識庫資料夾
    let pool = [], queue = [], shown = [];
    let fallback = false;
    let version = -1;           // Swift 端快取的版本；沒變時不重傳       // true = 沒有新的 / 不熟的卡，改輪播全部
    let lastLoad = 0;

    const main = h('div.kc');
    body.append(main);

    // 權重：新卡（newDays 內建立，越新越重）+ 不熟（streak 越少越重）
    function weight(n) {
      const age = (Date.now() - n.created) / DAY;
      const fresh = fallback ? 1 / (1 + age / 30) : Math.max(0, 1 - age / (cfg.newDays || 14));
      const weak = n.state === 'learning' || n.state === 'stale' ? 1 / (1 + (n.streak || 0)) : 0;
      return 0.2 + 3 * fresh + 2 * weak;
    }
    // 依權重洗牌（權重大的比較常排前面），整輪輪完再洗
    const shuffle = () => pool.map(n => [Math.pow(Math.random(), 1 / weight(n)), n]).sort((a, b) => b[0] - a[0]).map(x => x[1]);

    async function load() {
      lastLoad = Date.now();
      try {
        const r = await ctx.call('kbCards', { since: version });
        if (r?.unchanged) { online = true; return; }
        if (!r?.cards) throw new Error('missing');
        version = r.version;
        // 優先：學習中、久沒複習、或 newDays 天內新增但還沒熟的卡；一張都沒有 → 全部的卡（越新越常出現）
        const cutoff = Date.now() - (cfg.newDays || 14) * DAY;
        const focus = r.cards.filter(n => n.state === 'learning' || n.state === 'stale' || (n.state !== 'known' && n.created >= cutoff));
        fallback = !focus.length;
        pool = fallback ? r.cards : focus;
        const byId = new Map(pool.map(n => [n.id, n]));
        queue = queue.filter(q => byId.has(q.id)).map(q => byId.get(q.id));
        online = true;
        if (!shown.length) next(); else render();
      } catch {
        online = false;
        render();
      }
    }

    const count = () => {
      if (cfg.count === 1 || cfg.count === 2) return cfg.count;
      return (ctx.el?.offsetHeight || body.clientHeight + 70) >= 300 ? 2 : 1;
    };

    function next() {
      const k = Math.min(count(), pool.length);
      shown = [];
      while (shown.length < k) {
        if (!queue.length) queue = shuffle().filter(n => !shown.includes(n));
        if (!queue.length) break;
        shown.push(queue.shift());
      }
      render();
    }

    function render() {
      if (online === false) {
        main.replaceChildren(h('div.kc-off', {},
          h('span.danger', {}, '找不到知識庫資料夾（private/KnowledgeBase）'),
          h('button.btn.edit-live', { onclick: load }, h('span', {}, '重試'))));
        return;
      }
      if (online === null) { main.replaceChildren(h('span.s', {}, '讀取中…')); return; }
      if (!shown.length) {
        main.replaceChildren(h('span.s', {}, '知識庫還沒有卡片'));
        return;
      }
      main.style.setProperty('--kc-lines', shown.length > 1 ? 2 : 4);
      main.replaceChildren(...shown.map(n => {
        const isNew = Date.now() - n.created < 7 * DAY;
        const text = n.summary || n.body || '';
        return h('button.kc-card.edit-live', { type: 'button', title: '在知識庫開啟', onclick: () => ctx.call('open', `lore-kb://node/${encodeURIComponent(n.id)}`).catch(() => {}) },
          h('div.kc-head', {},
            h('b.kc-t.hl', {}, n.title),
            h('span.s.kc-badge', {}, n.state === 'known' ? '' : isNew ? '新' : n.streak ? `記得 ${n.streak} 次` : '不熟')),
          n.aliases?.length ? h('span.s.kc-al', {}, n.aliases.join(' · ')) : null,
          text ? h('span.s.kc-sum', {}, text) : null);
      }));
    }

    const rot = setInterval(() => { if (!document.hidden && online) next(); }, Math.max(10, cfg.rotateSec || 60) * 1000);
    const poll = setInterval(() => {
      if (!document.hidden && Date.now() - lastLoad >= 5 * 60000 - 1000) load();   // 本機檔案，讀起來很便宜
    }, 30000);
    let lastCount = 0;
    const fit = () => { const c = count(); if (lastCount && c !== lastCount && online) next(); lastCount = c; };
    fit();
    load();
    return { resize: fit, destroy() { clearInterval(rot); clearInterval(poll); } };
  },
};
