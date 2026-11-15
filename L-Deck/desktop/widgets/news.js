// 新聞：依自訂關鍵字抓 Google News RSS（Swift 端抓，不需要爬網頁）。
// 點標題 → 用瀏覽器開原文；⚙ → 編輯關鍵字。
// 每 3 小時更新一次；抓不到（斷網）時保留舊的、5 分鐘後或網路恢復時重抓；記憶體吃緊時（壓力警告或用量超過 maxMemUse）跳過，10 分鐘後再試。
// AI 摘要：.nw-summary 先預留，等決定用哪種模型再接上。
import { h, injectCSS } from '../core/util.js';

injectCSS('news', `
:where(.nw) { height: 100%; display: flex; flex-direction: column; gap: 8px; }
:where(.nw-tabs) { display: flex; gap: 6px; align-items: center; }
:where(.nw-tabs-scroll) { flex: 1; min-width: 0; display: flex; gap: 6px; align-items: center; overflow-x: auto; scrollbar-width: none; padding: 2px 4px; }
:where(.nw-tabs-scroll)::-webkit-scrollbar { display: none; }
:where(.nw-tabs) > *, :where(.nw-tabs-scroll) > * { flex-shrink: 0; white-space: nowrap; }
:where(.nw-list) { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 8px; }
:where(.nw-item) { cursor: pointer; display: flex; flex-direction: column; gap: 2px; }
:where(.nw-title) { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; line-height: 1.4; }
:where(.nw-edit) { display: flex; flex-direction: column; gap: 10px; }
:where(.nw-kws) { display: flex; gap: 6px; flex-wrap: wrap; }
:where(.nw-summary:empty) { display: none; }
`);

// Swift 端 NewsFetcher 認得的語言（對應 Google News 地區版）
const LANGS = [['zh-TW', '中文'], ['en', 'English']];

// 把 n 依權重分成整數份（最大餘數法），例如 10 × [0.7, 0.3] → [7, 3]
const split = (n, w) => {
  const sum = w.reduce((a, b) => a + b, 0) || 1;
  const raw = w.map(x => n * x / sum), out = raw.map(Math.floor);
  raw.map((x, i) => [x - out[i], i]).sort((a, b) => b[0] - a[0])
    .slice(0, n - out.reduce((a, b) => a + b, 0)).forEach(([, i]) => out[i]++);
  return out;
};

const ago = ms => {
  const m = Math.max(0, Math.round((Date.now() - ms) / 60000));
  return m < 60 ? `${m} 分鐘前` : m < 1440 ? `${Math.round(m / 60)} 小時前` : `${Math.round(m / 1440)} 天前`;
};

export default {
  title: 'NEWS.FEED',
  tag: 'NEWS',
  defaultSize: { w: 400, h: 260 },
  minSize: { w: 240, h: 140 },
  channels: ['stats'],     // 只用來判斷記憶體是否吃緊
  defaultConfig: { keywords: ['Apple', 'AI'], langs: ['zh-TW', 'en'], langShare: { 'zh-TW': 0.7, en: 0.3 }, perKeyword: 10, refreshMin: 180, maxMemUse: 0.85 },

  mount(body, ctx) {
    const cfg = ctx.config;
    cfg.keywords ||= [];
    // 舊設定（只有單一 lang、每個關鍵字 6 則）→ 中英都抓、共 10 則（7:3）
    if ('lang' in cfg) { delete cfg.lang; cfg.langs = ['zh-TW', 'en']; cfg.langShare = { 'zh-TW': 0.7, en: 0.3 }; cfg.perKeyword = Math.max(cfg.perKeyword || 0, 10); ctx.save(); }
    if (!cfg.langs?.length) cfg.langs = ['zh-TW'];
    let data = [];            // [{ keyword, items }]
    let tab = '*';
    let editing = false;
    let loading = false;
    let mem = null;           // 最新的記憶體狀態
    let lastFetch = 0;
    let note = '';
    ctx.on('stats', s => { if (s.mem) mem = { use: s.mem.used / s.mem.total, pressure: s.pressure }; });
    const memBusy = () => mem && (mem.pressure >= 2 || mem.use > (cfg.maxMemUse ?? 0.85));

    const tabs = h('div.nw-tabs');
    const tabScroll = h('div.nw-tabs-scroll', {
      // 滑鼠滾輪直向捲動 → 標籤列橫向捲動
      onwheel: e => { if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) { tabScroll.scrollLeft += e.deltaY; e.preventDefault(); } },
    });
    const summary = h('div.nw-summary.well.s');
    const list = h('div.nw-list');
    body.append(h('div.nw', {}, tabs, summary, list));

    let retry;
    async function load(force = false) {
      clearTimeout(retry);
      if (!cfg.keywords.length) { data = []; render(); return; }
      if (!force && memBusy()) {
        note = `記憶體吃緊（${Math.round(mem.use * 100)}%），延後更新`;
        render();
        retry = setTimeout(load, 10 * 60000);
        return;
      }
      loading = true; note = ''; render();
      // 每種語言各抓一次（Google News 各地區版），同一關鍵字的結果合併
      // 每個關鍵字共 perKeyword 則，依 langShare 分給各語言（預設台灣 70%）
      const quota = split(cfg.perKeyword, cfg.langs.map(l => cfg.langShare?.[l] ?? 1));
      const per = await Promise.all(cfg.langs.map((lang, i) => quota[i] <= 0 ? [] :
        ctx.call('fetchNews', { keywords: cfg.keywords, lang, limit: quota[i] }).catch(() => [])));
      const got = cfg.keywords.map(keyword => ({
        keyword, items: per.flatMap(r => r.find(g => g.keyword === keyword)?.items || []),
      }));
      loading = false;
      if (got.some(g => g.items.length)) { data = got; lastFetch = Date.now(); }
      else {
        // 全部抓空 = 多半是斷網：保留舊新聞，5 分鐘後再試（網路恢復時也會馬上重抓）
        note = '連不上新聞來源，稍後重試';
        retry = setTimeout(load, 5 * 60000);
      }
      render();
    }
    const onOnline = () => load();
    window.addEventListener('online', onOnline);

    function render() {
      tabScroll.replaceChildren(
        ...['*', ...cfg.keywords].map(k => h(`button.chip.mini.edit-live${tab === k ? '.on' : ''}`, {
          onclick: () => { tab = k; render(); },
        }, h('span', {}, k === '*' ? '全部' : k))));
      tabs.replaceChildren(
        tabScroll,
        h('button.chip.mini.edit-live', { title: '編輯關鍵字', onclick: () => { editing = !editing; render(); } }, h('span', {}, editing ? '完成' : '⚙')),
      );

      if (editing) {
        const input = h('input.edit-live', {
          type: 'text', placeholder: '輸入關鍵字後按 Enter',
          onkeydown: e => {
            if (e.key !== 'Enter' || e.isComposing) return;
            const v = input.value.trim();
            if (v && !cfg.keywords.includes(v)) { cfg.keywords.push(v); ctx.save(); load(true); }
            input.value = '';
          },
        });
        list.replaceChildren(h('div.nw-edit', {},
          h('div.nw-kws', {}, cfg.keywords.map(k => h('button.chip.mini.on.edit-live', {
            title: '移除', onclick: () => { cfg.keywords = cfg.keywords.filter(x => x !== k); if (tab === k) tab = '*'; ctx.save(); load(true); },
          }, h('span', {}, `${k} ✕`)))),
          input,
          h('div.nw-kws', {}, LANGS.map(([code, label]) => h(`button.chip.mini.edit-live${cfg.langs.includes(code) ? '.on' : ''}`, {
            onclick: () => {
              const on = cfg.langs.includes(code);
              if (on && cfg.langs.length === 1) return;   // 至少留一種
              cfg.langs = on ? cfg.langs.filter(x => x !== code) : [...cfg.langs, code];
              ctx.save(); render(); load(true);
            },
          }, h('span', {}, label)))),
          h('div.s', {}, `每個關鍵字取最新 ${cfg.perKeyword} 則 · 每 ${Math.round(cfg.refreshMin / 60)} 小時更新 · 記憶體超過 ${Math.round((cfg.maxMemUse ?? .85) * 100)}% 時跳過`)));
        setTimeout(() => input.focus(), 0);
        return;
      }

      if (loading && !data.length) { list.replaceChildren(h('div.s', {}, '抓取中…')); return; }
      if (!cfg.keywords.length) { list.replaceChildren(h('div.s', {}, '按 ⚙ 加入想追的關鍵字')); return; }

      // 合併、去重（同一則新聞可能被多個關鍵字抓到）、依時間排序
      const seen = new Set();
      const items = data.filter(g => tab === '*' || g.keyword === tab)
        .flatMap(g => g.items.map(it => ({ ...it, keyword: g.keyword })))
        .filter(it => !seen.has(it.title) && seen.add(it.title))
        .sort((a, b) => b.time - a.time);
      summary.textContent = note || (lastFetch ? `更新於 ${new Date(lastFetch).toLocaleTimeString('zh-TW', { hour: '2-digit', minute: '2-digit', hour12: false })}` : '');
      list.replaceChildren(...(items.length ? items.map(it => h('div.nw-item.edit-live', {
        title: it.title, onclick: () => ctx.call('open', it.link),
      },
      h('span.s', {}, h('b.nw-kw', {}, it.keyword), ` · ${it.source} · ${ago(it.time)}`),
      h('span.nw-title', {}, it.title))) : [h('div.s', {}, '沒有結果')]));
    }

    // 等第一筆記憶體數據進來再決定要不要抓（最多等 3 秒）
    setTimeout(() => load(), mem ? 0 : 3000);
    const iv = setInterval(() => load(), (cfg.refreshMin || 180) * 60000);
    return { destroy: () => { clearInterval(iv); clearTimeout(retry); window.removeEventListener('online', onOnline); } };
  },
};
