// 知識庫：今日數字、記得嗎？（到期卡片一次一張）、最近學到。資料來自本機 kb-server（KnowledgeBase）。
// 點卡片 → 用瀏覽器開知識庫網頁（#/node/<id>）。
// 到期通知：在知識庫「設定」的通知時間（預設 09:00、21:00）向 kb-server 認領這個時段，
//   認領成功且有到期卡片就跳 macOS 通知；同一時段只會通知一次。NeonDeck 沒開時改由 kb-server 自己通知。
//   config.notify = false：這個小工具不處理通知（交給 kb-server，要全部關掉請到知識庫設定）。
// kb-server 沒開（或用到一半斷線）→ 顯示離線；每 pollMinutes 分鐘更新一次，頁面隱藏時暫停。
// 小尺寸（小工具高度 < 320）：數字縮成一行、按鈕排成一列、不顯示「最近學到」。
// 預覽（python3 -m http.server）連不到 kb-server 時用 core/mock.js 的假資料；網址加 ?kb=offline 看離線畫面。
import { h, pad2, injectCSS } from '../core/util.js';

injectCSS('knowledge', `
:where(.kn) { height: 100%; display: flex; flex-direction: column; gap: 10px; min-height: 0; }
:where(.kn-stats) { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; text-align: center; }
:where(.kn-stats > div) { display: flex; flex-direction: column; gap: 2px; padding: 4px 0; }
:where(.kn-stats .num) { font-size: 20px; line-height: 1.1; }
:where(.kn-sec) { display: flex; align-items: baseline; gap: 6px; }
:where(.kn-sec .grow) { flex: 1; }
:where(.kn-quiz) { display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; }
:where(.kn-q) { line-height: 1.45; overflow-wrap: anywhere; }
:where(.kn-al) { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
:where(.kn-ans) { line-height: 1.45; max-height: 6.2em; overflow-y: auto; overflow-wrap: anywhere; }
:where(.kn-actions) { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
:where(.kn-actions .push) { margin-left: auto; }
:where(.kn-recent) { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 6px; }
:where(.kn-item) { display: flex; flex-direction: column; gap: 1px; cursor: pointer; min-width: 0; text-align: left; background: none; border: 0; padding: 0; color: inherit; font: inherit; }
:where(.kn-item:focus-visible, .kn button:focus-visible) { outline: 1px solid currentColor; outline-offset: 2px; }
:where(.kn-item .t) { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
:where(.kn-item .s) { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; line-height: 1.35; }
:where(.kn-off) { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; line-height: 1.5; min-height: 0; overflow-y: auto; }
:where(.kn-off-row) { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
:where(.kn-help) { padding: 0 8px; min-width: 24px; }
:where(.kn-recent-wrap) { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 6px; }
:where(.kn .lab-short) { display: none; }
:where(.kn.compact) { gap: 6px; }
:where(.kn.compact .kn-recent-wrap, .kn.compact .kn-quiz-head) { display: none; }
:where(.kn.compact .kn-stats) { display: flex; gap: 12px; justify-content: flex-start; text-align: left; white-space: nowrap; overflow: hidden; }
:where(.kn.compact .kn-stats > div) { flex-direction: row; align-items: baseline; gap: 4px; padding: 0; }
:where(.kn.compact .kn-stats .num) { font-size: 15px; order: 2; }
:where(.kn.compact .lab-full) { display: none; }
:where(.kn.compact .lab-short) { display: inline; }
:where(.kn.compact .kn-quiz) { padding: 6px 8px; gap: 4px; min-height: 0; }
:where(.kn.compact .kn-q) { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
:where(.kn.compact .kn-ans) { max-height: 2.9em; }
:where(.kn.compact .kn-actions) { flex-wrap: nowrap; gap: 4px; min-width: 0; }
:where(.kn.narrow .kn-actions) { flex-wrap: nowrap; gap: 4px; min-width: 0; }
.kn:is(.compact, .narrow) .kn-actions .btn { padding: 4px 9px; font-size: 12px; flex: none; }   /* 要蓋過主題的 .btn 內距 */
:where(.kn.compact .kn-note) { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
:where(.kn.tiny .kn-al) { display: none; }
`);

const bridged = !!window.webkit?.messageHandlers?.deck;
const qs = new URLSearchParams(location.search);
const parseHM = s => { const m = /^(\d{1,2}):(\d{2})$/.exec(String(s).trim()); return m ? +m[1] * 60 + +m[2] : null; };
const today = () => { const d = new Date(); return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`; };
const md = iso => { const d = new Date(iso); return `${d.getMonth() + 1}/${d.getDate()}`; };

export default {
  title: 'KNOWLEDGE',
  tag: 'KB',
  defaultSize: { w: 360, h: 420 },
  minSize: { w: 240, h: 240 },
  defaultConfig: { server: 'http://127.0.0.1:7700', recentDays: 7, recentMax: 8, pollMinutes: 5, notify: true, sound: 'Glass' },

  mount(body, ctx) {
    const cfg = ctx.config;
    const base = () => String(cfg.server || 'http://127.0.0.1:7700').replace(/\/+$/, '');
    let mock = null, mockP = null;    // 預覽用假 kb-server
    let online = null;                // null = 還不知道
    let stats = null, due = [], recent = [];
    let reveal = false, answerText = '', busy = false, note = '';
    let notifyTimes = ['09:00', '21:00'];
    let lastLoad = 0, lastSlot = null, showHelp = false;

    async function api(method, path, body) {
      if (mock) return mock(method, path, body);
      const ctl = new AbortController();
      const t = setTimeout(() => ctl.abort(), 6000);
      try {
        const r = await fetch(base() + path, {
          method, signal: ctl.signal,
          headers: body ? { 'content-type': 'application/json' } : {},
          body: body ? JSON.stringify(body) : undefined,
        });
        const data = await r.json().catch(() => ({}));
        if (!r.ok) throw Object.assign(new Error(data.error || r.statusText), { status: r.status });
        return data;
      } catch (e) {
        // 瀏覽器預覽：一開始就連不到真的 kb-server 才改用假資料（?kb=offline 可看離線畫面）；
        // 用到一半斷線則顯示離線，不偷偷換成假資料。
        if (!bridged && !e.status && online !== true && qs.get('kb') !== 'offline') {
          mockP ||= import('../core/mock.js').then(m => m.kbMock());
          mock = await mockP;
          return mock(method, path, body);
        }
        throw e;
      } finally { clearTimeout(t); }
    }

    const open = route => ctx.call('open', `${base()}/#/${route}`).catch(() => {});

    // ---------- DOM ----------
    const statsEl = h('div.kn-stats');
    const quizHead = h('div.kn-sec.kn-quiz-head', {}, h('span.s', {}, '記得嗎？'), h('span.grow'), h('span.s.kn-count'));
    const quiz = h('div.kn-quiz.well');
    const recentHead = h('div.kn-sec', {}, h('span.s.grow'), h('span.s'));
    const recentList = h('div.kn-recent');
    const main = h('div.kn', {}, statsEl, h('div', {}, quizHead, quiz),
      h('div.kn-recent-wrap', {}, recentHead, recentList));
    body.append(main);

    // ---------- 資料 ----------
    async function load() {
      lastLoad = Date.now();
      try {
        const [st, d, r] = await Promise.all([
          api('GET', '/api/stats'),
          api('GET', '/api/recall/due?limit=30'),
          api('GET', `/api/recent?days=${cfg.recentDays || 7}&limit=${cfg.recentMax || 8}`),
        ]);
        stats = st.stats;
        if (Array.isArray(st.notify?.times)) notifyTimes = st.notify.times;
        const cur = due[0]?.id;
        due = d.nodes || [];
        if (due[0]?.id !== cur) { reveal = false; answerText = ''; }
        recent = r.nodes || [];
        online = true;
      } catch (e) {
        online = false;
      }
      render();
      checkSlot();
    }

    async function answer(outcome) {
      const n = due[0];
      if (!n || busy) return;
      busy = true;
      try {
        const r = await api('POST', `/api/recall/${n.id}`, { outcome, via: 'neondeck' });
        const p = r.progress;
        note = r.step?.became_known ? `「${n.title}」記住了！`
          : outcome === 'remembered' ? (r.step?.advanced ? `連續記得 ${p.streak} 次，${md(p.next_check_at)} 再問` : '還沒到期，先記一筆')
          : `沒關係，${md(p.next_check_at)} 再問一次`;
        due.shift();
        reveal = false; answerText = '';
        busy = false;
        render();
        load();
      } catch (e) {
        busy = false;
        if (!e.status) { online = false; note = '剛才的作答沒有送出'; }   // 連不到 → 離線畫面
        else note = '作答失敗：' + (/[\u3400-\u9fff]/.test(e.message) ? e.message : (e.status === 404 ? '找不到這張卡' : '伺服器錯誤'));
        render();
      }
    }

    async function showAnswer() {
      const n = due[0];
      if (!n) return;
      reveal = true;
      answerText = n.summary || '';
      render();
      if (!answerText) {
        try {
          const v = await api('GET', `/api/nodes/${n.id}`);
          const b = (v.node?.body || '').replace(/\s+/g, ' ').trim();
          answerText = b ? (b.length > 200 ? b.slice(0, 200) + '…' : b) : '（這張卡沒有摘要）';
        } catch (e) {
          if (!e.status) { online = false; note = ''; }
          answerText = '（讀不到內容）';
        }
        render();
      }
    }

    // ---------- 畫面 ----------
    function render() {
      ctx.setTitle(`${cfg.title ?? 'KNOWLEDGE'}${mock ? ' · 預覽資料' : ''}`);
      if (online === false) {
        main.replaceChildren(h('div.kn-off', {},
          h('span.danger', {}, '知識庫離線'),
          note ? h('span.s', {}, note) : null,
          h('div.kn-off-row', {},
            h('button.btn', { onclick: () => { note = ''; load(); } }, h('span', {}, '重試')),
            h('button.btn.ghost.kn-help', { onclick: () => { showHelp = !showHelp; render(); }, 'aria-expanded': String(showHelp), 'aria-label': '怎麼啟動知識庫', title: '怎麼啟動知識庫' }, h('span', {}, '?'))),
          showHelp ? h('span.s', {}, `連不到 ${base()}。在 KnowledgeBase/public 執行 cargo run -p kb-server，或用 scripts/install-launchd.sh 設定登入時自動啟動。`) : null));
        return;
      }
      if (!main.contains(statsEl)) main.replaceChildren(statsEl, h('div', {}, quizHead, quiz), h('div.kn-recent-wrap', {}, recentHead, recentList));
      if (!stats) { statsEl.replaceChildren(h('span.s', {}, '讀取中…')); return; }

      const cell = (v, label, short, level) => h('div', { 'data-level': level || null, title: `${label} ${v}` }, h('span.num', {}, String(v)),
        h('span.s', {}, h('span.lab-full', {}, label), h('span.lab-short', {}, short)));
      statsEl.replaceChildren(
        cell(stats.new_today, '今日新增', '新增'),
        cell(stats.due, '待複習', '待複習', stats.due > 0 ? 'warn' : null),
        cell(stats.known_week, '7 天內記住', '記住'));

      const n = due[0];
      quizHead.lastChild.textContent = due.length ? `剩 ${stats.due} 張` : '';
      if (!n) {
        quiz.replaceChildren(...[
          h('div.kn-q', {}, stats.total ? '今天沒有到期的卡片 ✓' : '還沒有知識卡'),
          note ? h('div.s.kn-note', {}, note) : null,
          h('div.kn-actions', {}, h('button.btn.ghost', { onclick: () => open(stats.total ? 'review' : 'new') }, h('span', {}, stats.total ? '開啟知識庫' : '新增第一張卡')))].filter(Boolean));
      } else {
        quiz.replaceChildren(...[
          h('div.kn-q', {}, '還記得「', h('b.hl', {}, n.title), '」嗎？'),
          n.aliases?.length ? h('div.s.kn-al', {}, n.aliases.join(' · ')) : null,
          reveal ? h('div.kn-ans.s', {}, answerText || '…') : null,
          h('div.kn-actions', {},
            h('button.btn.primary', { onclick: () => answer('remembered'), disabled: busy }, h('span', {}, '記得')),
            h('button.btn', { onclick: () => answer('forgot'), disabled: busy }, h('span', {}, '忘了')),
            reveal ? null : h('button.btn.ghost', { onclick: showAnswer, title: '看答案' }, h('span', {}, compact ? '答案' : '看答案')),
            h('button.btn.ghost.push', { onclick: () => open(`node/${n.id}`), title: '在知識庫開啟這張卡', 'aria-label': `在知識庫開啟「${n.title}」` }, h('span', { 'aria-hidden': 'true' }, '↗'))),
          note ? h('div.s.kn-note', {}, note) : null].filter(Boolean));
      }

      recentHead.firstChild.textContent = `最近學到 · ${cfg.recentDays || 7} 天`;
      recentHead.lastChild.textContent = recent.length ? String(recent.length) : '';
      recentList.replaceChildren(...(recent.length ? recent.map(r => h('button.kn-item', { type: 'button', onclick: () => open(`node/${r.id}`), title: '在知識庫開啟' },
        h('span.t', {}, r.title),
        r.summary ? h('span.s', {}, r.summary) : null)) : [h('span.s', {}, '這幾天沒有新卡片')]));
    }

    // ---------- 到期通知 ----------
    /** 今天最近一個已經過了的通知時段，例如 "2026-11-09T09:00"。 */
    function currentSlot() {
      const d = new Date(), now = d.getHours() * 60 + d.getMinutes();
      const past = notifyTimes.map(parseHM).filter(m => m != null && m <= now);
      if (!past.length) return null;
      const m = Math.max(...past);
      return `${today()}T${pad2(Math.floor(m / 60))}:${pad2(m % 60)}`;
    }

    async function checkSlot() {
      if (cfg.notify === false || online === false) return;
      const slot = currentSlot();
      if (!slot || slot === lastSlot) return;
      try {
        const c = await api('POST', '/api/reminders/claim', { by: 'neondeck' });
        lastSlot = slot;
        if (c.claimed) ctx.call('notify', { title: c.title, body: c.body, sound: cfg.sound || undefined }).catch(() => {});
      } catch { /* 下一分鐘再試 */ }
    }

    // ---------- 排程 ----------
    const poll = setInterval(() => {
      if (document.hidden) return;   // 隱藏時暫停
      if (Date.now() - lastLoad >= (cfg.pollMinutes || 5) * 60000 - 1000) load();
    }, 30000);
    const slotTimer = setInterval(checkSlot, 60000);   // 只在時段變了才連線
    const onVis = () => { if (!document.hidden && Date.now() - lastLoad > 60000) load(); };
    document.addEventListener('visibilitychange', onVis);

    // 依整個小工具的高度切換（主題的標題列、內距不同，body 高度不可靠）
    let compact = false;
    function fit() {
      const H = ctx.el?.offsetHeight || body.clientHeight + 70;
      const c = H < 320;
      main.classList.toggle('tiny', H < 250);
      main.classList.toggle('narrow', (ctx.el?.offsetWidth || body.clientWidth + 60) < 330);
      if (c !== compact) { compact = c; main.classList.toggle('compact', c); if (stats) render(); }
    }
    fit();
    load();
    return {
      resize: fit,
      destroy() {
        clearInterval(poll); clearInterval(slotTimer);
        document.removeEventListener('visibilitychange', onVis);
      },
    };
  },
};
