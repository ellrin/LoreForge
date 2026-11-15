// App / 資料夾 / 網址捷徑。編輯模式下可以按 ＋ 從 Finder 選 App 加入、按 ✕ 移除。
import { h, injectCSS } from '../core/util.js';

injectCSS('launcher', `
:where(.ln) { height: 100%; overflow-y: auto; display: grid; grid-template-columns: repeat(auto-fill, minmax(var(--ln-size, 76px), 1fr)); gap: 10px; align-content: start; }
:where(.ln-item) { position: relative; display: flex; flex-direction: column; align-items: center; gap: 4px; padding: 6px 2px; cursor: pointer; }
:where(.ln-item img) { width: 44px; height: 44px; }
:where(.ln-item .ph) { width: 44px; height: 44px; display: grid; place-items: center; }
:where(.ln-item .n) { max-width: 100%; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
:where(.ln-item .x) { display: none; position: absolute; top: 0; right: 4px; }
:where(body.editing .ln-item .x) { display: block; }
:where(.ln-add) { display: none; }
:where(body.editing .ln-add) { display: flex; }
`);

export default {
  title: 'LAUNCHPAD',
  defaultSize: { w: 760, h: 130 },
  minSize: { w: 100, h: 90 },
  defaultConfig: {
    // path：App / 檔案 / 資料夾的路徑，或 https:// 網址；name 可自訂；icon 可填圖片路徑覆蓋
    apps: [
      { path: '/System/Library/CoreServices/Finder.app', name: 'Finder' },
      { path: '/Applications/Safari.app', name: 'Safari' },
      { path: '/Applications/Google Chrome.app', name: 'Chrome' },
      { path: '/Applications/Visual Studio Code.app', name: 'VS Code' },
      { path: '/System/Applications/Utilities/Terminal.app', name: 'Terminal' },
      { path: '/System/Applications/Notes.app', name: '備忘錄' },
    ],
  },

  mount(body, ctx) {
    const grid = h('div.ln');
    body.append(grid);

    async function render() {
      const apps = ctx.config.apps || [];
      const nodes = await Promise.all(apps.map(async (a, i) => {
        const isURL = /^[a-z]+:\/\//i.test(a.path);
        let icon = a.icon || (isURL ? null : await ctx.call('appIcon', a.path).catch(() => null));
        const img = icon ? h('img', { src: icon, draggable: false }) : h('span.ph.mid', {}, isURL ? '⌘' : '?');
        return h('div.ln-item.edit-live', {
          title: a.path,
          onclick: e => { if (!e.target.closest('.x') && !ctx.editing) ctx.call('open', a.path); },
        }, img, h('span.n.small', {}, a.name || a.path.split('/').pop().replace(/\.app$/, '')),
        h('button.x.btn.ghost', {
          onclick: () => { ctx.config.apps.splice(i, 1); ctx.save(); render(); },
        }, '✕'));
      }));
      const add = h('div.ln-item.ln-add.edit-live', {
        onclick: async () => {
          const picked = await ctx.call('pickApps').catch(() => []);
          if (!picked.length) return;
          ctx.config.apps = [...(ctx.config.apps || []), ...picked];
          ctx.save(); render();
        },
      }, h('span.ph.big', {}, '＋'), h('span.n.small', {}, '加入'));
      grid.replaceChildren(...nodes, add);
    }
    render();
  },
};
