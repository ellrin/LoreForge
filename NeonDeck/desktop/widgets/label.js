// 分組標題（例如「SYS 系統」），只是裝飾，色調跟著 config.hue 或主題。
import { h, injectCSS } from '../core/util.js';

injectCSS('label', `
:where(.lb) { height: 100%; display: flex; align-items: center; gap: 8px; }
`);

export default {
  title: 'LABEL',
  defaultSize: { w: 400, h: 28 },
  minSize: { w: 80, h: 20 },
  defaultConfig: { hideHeader: true, tag: 'SYS', text: '系統', hue: '' },

  mount(body, ctx) {
    body.append(h('div.lb', {}, h('b.lb-tag', {}, ctx.config.tag || ''), h('span.lb-text', {}, ctx.config.text || '')));
  },
};
