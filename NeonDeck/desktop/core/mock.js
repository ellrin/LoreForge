// 瀏覽器預覽模式用的假 Swift 橋接（在 NeonDeck.app 裡不會載入）。
// 用法：cd desktop && python3 -m http.server 8765 → 打開 http://localhost:8765
// 可以用 Chrome DevTools 調主題；版面變更只存在記憶體，不會寫回 layout.json。

async function listDir(dir, ext) {
  try {
    const html = await (await fetch(dir + '/')).text();
    return [...html.matchAll(/href="([^"]+)"/g)].map(m => decodeURIComponent(m[1]))
      .filter(n => n.endsWith('.' + ext) && !n.startsWith('_')).map(n => n.slice(0, -ext.length - 1)).sort();
  } catch { return []; }
}

export function createMock(emit) {
  document.documentElement.style.background = '#000';   // 模擬黑色桌布
  let t = 0, timer;
  const GiB = 2 ** 30;
  const now = Date.now();
  const day = 86400000;
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const at = (d, hh, mm = 0) => today.getTime() + d * day + (hh * 60 + mm) * 60000;

  const fakeStats = () => {
    t++;
    const wave = (p, a, b) => a + (b - a) * (0.5 + 0.5 * Math.sin(t / p)) * (0.6 + 0.4 * Math.random());
    return {
      cpu: wave(7, .04, .45), down: wave(5, 2e4, 9e6), up: wave(9, 5e3, 6e5),
      rx: 5.9e9 + t * 3e6, tx: 1.3e9 + t * 2e5,
      mem: { used: wave(30, 17, 21) * GiB, total: 32 * GiB, app: 14 * GiB, wired: 2.7 * GiB, compressed: 1.5 * GiB, swap: 0 },
      pressure: 1, load: [2.1, 2.7, 3.6], uptime: 8000 + t,
    };
  };

  return async (method, params) => {
    switch (method) {
      case 'ready':
        return {
          layout: { ...(await (await fetch('layout.json')).json()), ...(new URLSearchParams(location.search).get('theme') ? { theme: new URLSearchParams(location.search).get('theme') } : {}) },
          themes: await listDir('themes', 'css'),
          widgets: await listDir('widgets', 'js'),
          info: { host: 'Preview-Mac', os: 'macOS 27.0', chip: 'Apple M6', cores: '6P+6E', boot: now - 8000e3, memTotal: 32 * GiB },
        };
      case 'subscribe':
        clearInterval(timer);
        emit('disk', { free: 824e9, total: 995e9, at: now });
        emit('slow', { battery: { percent: 100, charging: false, ac: true }, ip: '192.168.0.19', thermal: 'nominal' });
        const procs = () => emit('procs', { ready: true, cores: 12,
          cpu: [
            { pid: 4988, name: 'Google Chrome Helper (Renderer)', app: 'Google Chrome', appPath: '/Applications/Google Chrome.app', path: '/Applications/Google Chrome.app/…/Google Chrome Helper (Renderer)', cpu: 14.6, mem: 312e6 },
            { pid: 7747, name: 'Code Helper (Plugin)', app: 'Visual Studio Code', appPath: '/Applications/Visual Studio Code.app', path: '…', cpu: 9.8, mem: 448e6 },
            { pid: 6780, name: 'Claude', app: 'Claude', appPath: '/Applications/Claude.app', path: '…', cpu: 6.1, mem: 514e6 },
            { pid: 512, name: 'mds_stores', app: null, appPath: null, path: '/System/Library/…/mds_stores', cpu: 4.3, mem: 80e6 },
            { pid: 333, name: 'assistantd', app: null, appPath: null, path: '/usr/libexec/assistantd', cpu: 2.1, mem: 40e6 },
          ],
          mem: [
            { name: 'Google Chrome', appPath: '/Applications/Google Chrome.app', mem: 2.1e9, count: 14, pid: 4978 },
            { name: 'Visual Studio Code', appPath: '/Applications/Visual Studio Code.app', mem: 2.0e9, count: 16, pid: 1748 },
            { name: 'Claude', appPath: '/Applications/Claude.app', mem: 1.4e9, count: 6, pid: 6780 },
            { name: 'MTLCompilerService', appPath: null, mem: 492e6, count: 16, pid: 900 },
            { name: 'Safari', appPath: '/Applications/Safari.app', mem: 380e6, count: 3, pid: 901 },
          ] });
        procs();
        for (let i = 0; i < 60; i++) emit('stats', fakeStats());
        timer = setInterval(() => emit('stats', fakeStats()), 1000);
        return true;
      case 'fetchNews': return (params.keywords || []).map((k, ki) => ({ keyword: k, items: [
        { title: `${k}：新款 MacBook Pro M6 速測，效能升級真有感？`, source: 'Mobile01', link: 'https://example.com', time: Date.now() - (ki * 3 + 1) * 3600e3 },
        { title: `${k} 高層專訪：小型企業主也成了主要買家`, source: 'INSIDE', link: 'https://example.com', time: Date.now() - (ki * 5 + 2) * 3600e3 },
        { title: `${k} 新品發表時間曝光，維持平價路線但規格不陽春`, source: '自由電子報', link: 'https://example.com', time: Date.now() - (ki * 7 + 4) * 3600e3 },
      ] }));
      case 'access': return 'granted';
      case 'events': return [
        { id: 1, title: '設計評審', start: at(0, 10), end: at(0, 11), color: '#00F0FF', calendar: '工作' },
        { id: 2, title: '健身', start: at(0, 19, 30), end: at(0, 20, 30), color: '#05FFA1', calendar: '個人' },
        { id: 3, title: 'Sprint planning', start: at(1, 9, 30), end: at(1, 10, 30), color: '#00F0FF', calendar: '工作' },
        { id: 4, title: '朋友生日', start: at(3, 0), end: at(4, 0), allDay: true, color: '#FF2A6D', calendar: '個人' },
      ];
      case 'reminders': return [
        { id: 'a', title: '回覆設計稿意見', due: at(-1, 18), dueHasTime: true, list: '工作', color: '#FF2A6D' },
        { id: 'b', title: '繳電話費', due: at(2, 0), list: '個人', color: '#FCEE0A' },
        { id: 'c', title: '買咖啡豆', list: '個人', color: '#FCEE0A' },
      ];
      case 'appIcon': return null;
      case 'pickApps': return [];
      case 'refreshDisk': return { free: 824e9, total: 995e9, at: Date.now() };
      case 'log': console.log('[deck]', params); return true;
      default: console.log('[mock]', method, params); return true;
    }
  };
}

// ---------------------------------------------------------------------------
// widgets/knowledge.js 預覽用的假 kb-server（連不到 http://127.0.0.1:7700 時才用；只存在記憶體）
export function kbMock() {
  const day = 86400000, now = Date.now();
  const iso = t => new Date(t).toISOString();
  const card = (id, title, aliases, summary, createdDaysAgo, state, dueNow) => ({
    id, title, aliases, summary, kind: 'concept', space: 'personal', tags: [], domains: [],
    created_at: iso(now - createdDaysAgo * day), updated_at: iso(now - createdDaysAgo * day), revision: 1,
    state, streak: 0, next_check_at: iso(dueNow ? now - 3600e3 : now + 2 * day), score: 10,
  });
  const cards = [
    card('n01MOCK0000000000000000001', 'Dice coefficient', ['DSC', 'Dice score'], '衡量兩個集合重疊程度的指標，2|A∩B| / (|A|+|B|)，常用於分割評估。', 1, 'learning', true),
    card('n01MOCK0000000000000000002', 'Hausdorff distance', ['HD95'], '兩個點集之間「最遠的最近距離」，對邊界誤差很敏感；HD95 取第 95 百分位降低離群值影響。', 3, 'learning', true),
    card('n01MOCK0000000000000000003', 'Attention U-Net', [], '在 skip connection 加上注意力閘門，讓解碼器只取有用的編碼器特徵。', 4, 'learning', true),
    card('n01MOCK0000000000000000004', 'Focal loss', ['FL'], '降低容易樣本的權重，讓訓練專注在難分類的樣本；γ 控制下降速度。', 0, 'learning', false),
    card('n01MOCK0000000000000000005', 'Test-time augmentation', ['TTA'], '推論時對同一張影像做多種增強再平均預測。', 2, 'learning', false),
    card('n01MOCK0000000000000000006', '空間解析度', ['spatial resolution'], '每個像素（體素）代表的實際大小，例如 0.5 mm。', 6, 'known', false),
  ];
  const due = () => cards.filter(c => new Date(c.next_check_at).getTime() <= Date.now());
  let knownWeek = 1, claimedSlot = null;
  const json = v => JSON.parse(JSON.stringify(v));
  return async (method, path, body) => {
    const url = new URL(path, 'http://kb.mock');
    const p = url.pathname;
    if (p === '/api/stats') {
      const t0 = new Date(); t0.setHours(0, 0, 0, 0);
      return { now: iso(Date.now()), notify: { times: ['09:00', '21:00'] }, stats: {
        today: `${t0.getFullYear()}-${String(t0.getMonth() + 1).padStart(2, '0')}-${String(t0.getDate()).padStart(2, '0')}`, total: cards.length, due: due().length, known_week: knownWeek,
        new_today: cards.filter(c => new Date(c.created_at) >= t0).length, updated_today: 0,
        learning: cards.filter(c => c.state === 'learning').length, known: cards.filter(c => c.state === 'known').length, stale: 0 } };
    }
    if (p === '/api/recall/due') return { nodes: json(due().slice(0, +(url.searchParams.get('limit') || 50))) };
    if (p === '/api/recent') {
      const from = Date.now() - (+url.searchParams.get('days') || 7) * day;
      return { nodes: json(cards.filter(c => new Date(c.updated_at) >= from).slice(0, +(url.searchParams.get('limit') || 50))) };
    }
    const m = p.match(/^\/api\/(recall|nodes)\/([\w-]+)$/);
    const c = m && cards.find(x => x.id === m[2]);
    if (m?.[1] === 'nodes' && c) return { space: c.space, node: { ...json(c), body: c.summary }, progress: null };
    if (m?.[1] === 'recall' && c && method === 'POST') {
      const ok = body?.outcome === 'remembered';
      c.streak = ok ? c.streak + 1 : 0;
      const known = ok && c.streak >= 2;
      if (known) { c.state = 'known'; knownWeek++; }
      c.next_check_at = iso(Date.now() + (known ? 30 : ok ? 3 : 1) * day);
      return { progress: { state: c.state, streak: c.streak, next_check_at: c.next_check_at }, step: { advanced: true, became_known: known } };
    }
    if (p === '/api/reminders/claim') {
      const d = due(), slot = new Date().toISOString().slice(0, 13);
      const claimed = slot !== claimedSlot && d.length > 0;
      claimedSlot = slot;
      return { slot, claimed, due: d.length, title: `記得嗎？${d.length} 張知識卡到期`, body: d.slice(0, 3).map(x => x.title).join('、') };
    }
    throw Object.assign(new Error('mock: ' + p), { status: 404 });
  };
}
