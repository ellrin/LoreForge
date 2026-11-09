// 小工具範本：複製這個檔案並改名（不要以 _ 開頭），存檔後桌面就會自動重新載入，
// 然後在「編輯版面 ▸ 新增小工具」選單就看得到它。
import { h, injectCSS } from '../core/util.js';

// 只寫「排版」相關的 CSS，用 :where() 包住，顏色字型交給主題
injectCSS('my-widget', `
:where(.mw) { height: 100%; display: flex; align-items: center; justify-content: center; }
`);

export default {
  title: 'MY WIDGET',               // 標題列文字（layout.json 的 config.title 可覆蓋）
  defaultSize: { w: 240, h: 140 },
  minSize: { w: 120, h: 80 },
  channels: ['stats'],              // 需要系統數據才填：stats(每秒) / procs(5 秒) / slow(30 秒) / disk(一次)
  defaultConfig: { greeting: 'HELLO' },

  mount(body, ctx) {
    const label = h('span.big.c1');
    body.append(h('div.mw', {}, label));

    ctx.on('stats', s => {
      label.textContent = `${ctx.config.greeting} · CPU ${Math.round(s.cpu * 100)}%`;
    });

    // 其他可用：
    //   ctx.call('open', '/Applications/Safari.app')   開 App / 檔案 / 網址
    //   ctx.call('notify', { title, body, sound: 'Glass' })
    //   ctx.config.xxx = 1; ctx.save()                 存設定到 layout.json
    //   ctx.info                                       主機名、晶片、開機時間…

    return {
      resize() {},    // 尺寸變化時
      destroy() {},   // 被移除時（清掉 setInterval 等）
    };
  },
};
