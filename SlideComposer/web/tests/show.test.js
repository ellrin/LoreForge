// node web/tests/show.test.js — 選頁 logic without a browser (run by crates/server/tests/show.rs).
// present.js is loaded into a context with small stand-ins for the editor.
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const web = path.join(__dirname, "..");
const slides = Array.from({ length: 13 }, (_, i) => ({ uid: `s${i + 1}`, hidden: i === 6, objects: [] }));
const ctx = vm.createContext({
  console, Math, JSON, Map, Set, URL: class { constructor() { this.searchParams = { has: () => false }; } },
  location: { href: "http://x/" }, addEventListener() {}, removeEventListener() {}, setTimeout, clearTimeout, setInterval, clearInterval,
  document: { addEventListener() {}, removeEventListener() {} }, devicePixelRatio: 2, performance: { now: () => 0 },
  S: { deck: { slides, size: { w: 12192000, h: 6858000 } } },
});
ctx.window = ctx; ctx.globalThis = ctx;
vm.runInContext(fs.readFileSync(path.join(web, "present.js"), "utf8") + "\n;globalThis.SHOW = SHOW;", ctx);
// values cross the context as JSON (arrays from another realm compare unequal)
const run = (code) => { const v = vm.runInContext(`JSON.stringify(${code})`, ctx); return v === undefined ? undefined : JSON.parse(v); };
const exec = (code) => vm.runInContext(code, ctx);

// 選頁 layout: the deck fills the window when it fits, else the minimum width and scrolling
{
  const a = 9 / 16;
  const l13 = run(`pickLayout(13, 1384, 844, ${a})`);
  assert.equal(l13.cols, 4);
  assert.ok(l13.cw > 300, `13 slides at 1440×900 get large thumbnails (${l13.cw})`);
  const rows = Math.ceil(13 / l13.cols);
  assert.ok(rows * (l13.cw * a + 26) + (rows - 1) * 14 <= 844 + 1, "all rows fit the height");
  const l60 = run(`pickLayout(60, 1384, 844, ${a})`);
  assert.ok(l60.cw >= 180, "never under the minimum width");
  const l3 = run(`pickLayout(3, 1384, 844, ${a})`);
  assert.ok(l3.cols <= 3 && l3.cw > 400);
}

// a hidden slide chosen in 選頁 joins the show in deck order
{
  exec(`SHOW.list = S.deck.slides.filter((s) => !s.hidden)`);
  assert.equal(run("SHOW.list.length"), 12);
  const i = run(`showIndexOf("s7")`);
  assert.equal(i, 6);
  assert.deepEqual(run("SHOW.list.map((s) => s.uid)").slice(5, 8), ["s6", "s7", "s8"]);
  assert.equal(run(`showIndexOf("s7")`), 6, "only once");
  assert.equal(run(`showIndexOf("nope")`), -1);
}



// 互動 survives showRender: the stage (the audience root itself) is emptied on every render — black
// screen, a step back, a re-sync — and the live view must come back into it (the laser vanished that way once).
{
  class El {
    constructor(tag) { this.tagName = tag; this.children = []; this.parent = null; this.style = {}; this.dataset = {}; this.hidden = false; this.className = ""; this.clientWidth = 1280; this.clientHeight = 720;
      const cls = new Set(); this.classList = { add: (c) => cls.add(c), remove: (c) => cls.delete(c), toggle: (c, on) => (on ?? !cls.has(c)) ? cls.add(c) : cls.delete(c), contains: (c) => cls.has(c) }; }
    append(...ns) { for (const n of ns) { n.parent?.children.splice(n.parent.children.indexOf(n), 1); n.parent = this; this.children.push(n); } }
    remove() { if (this.parent) { this.parent.children.splice(this.parent.children.indexOf(this), 1); this.parent = null; } }
    set textContent(_) { for (const c of this.children) c.parent = null; this.children = []; }
    get isConnected() { let n = this; while (n.parent) n = n.parent; return n === ctx.document.body; }
    querySelector(sel) { const want = sel.replace(/^\./, ""); const walk = (n) => { for (const c of n.children) { if (c.className.split(" ").includes(want)) return c; const r = walk(c); if (r) return r; } return null; }; return walk(this) ?? (sel === ".laser" ? null : new El("stub")); }
    querySelectorAll() { return []; }
    addEventListener() {}
    setAttribute() {}
    focus() {}
    matches() { return false; }
    contains(n) { while (n) { if (n === this) return true; n = n.parent; } return false; }
    set innerHTML(_) { this.textContent = ""; }
    getBoundingClientRect() { return { left: 0, top: 0, width: 1280, height: 720 }; }
  }
  ctx.document.createElement = (t) => new El(t);
  ctx.document.body = new El("body");
  ctx.SC = { renderSlide: () => new El("div"), px: (v) => v / 9525 };
  ctx.ctxFor = () => ({});
  ctx.slideNo = () => 1;
  ctx.deckApi = (p) => p;
  ctx.paintIcons = () => {};
  ctx.fetch = () => Promise.resolve({ ok: false });
  ctx.BroadcastChannel = class { postMessage(m) { ctx.sent.push(m); } };
  ctx.sent = [];
  let destroyed = 0;
  ctx.LorePlot = { eqSim: {
    mount: (el, spec, o) => ({ role: o.role, state: () => ({ t: "full", eq: {}, snap: {} }), apply() {}, destroy: () => destroyed++ }),
    mountCurve: (el, spec, o) => ({ role: o.role, state: () => ({ t: "curve" }), apply() {}, destroy: () => destroyed++ }),
  } };
  const spec = { panels: [{ type: "equation", eq: { sim: { scene: "fit1d" } } }] };
  exec(`SHOW.list = S.deck.slides.filter((s) => !s.hidden); SHOW.i = 0; SHOW.on = true;`);
  // the audience window: its root is the stage
  exec(`setRoot("audience"); showRender(0);`);
  const stage = ctx.SHOW.stage;
  assert.ok(stage === ctx.SHOW.root, "the audience's stage is its root");
  // the presenter opens 互動: the audience follows
  return (async () => {
    await ctx.liveFollow({ t: "full", open: spec });
    const el = ctx.SHOW.live.el;
    assert.equal(ctx.SHOW.live.view.role, "view", "the audience only renders");
    assert.ok(stage.children.includes(el), "mounted in the stage");
    // black screen, a step back, a re-sync: each one empties the stage
    ctx.onShowMessage({ uid: "s1", step: 0, black: true, laser: { on: false }, zoom: { k: 1, x: 0.5, y: 0.5 } });
    assert.ok(stage.children.includes(el), "black screen keeps 互動");
    ctx.onShowMessage({ uid: "s1", step: 0, black: false, laser: { on: false }, zoom: { k: 1, x: 0.5, y: 0.5 } });
    exec(`showRender(0)`);
    assert.ok(stage.children.includes(el) && ctx.SHOW.live?.el === el, "showRender keeps the same live view (not a new one)");
    assert.equal(destroyed, 0);
    assert.ok(stage.querySelector(".laser"), "and the laser dot");
    // another slide closes it; the presenter's null closes it
    ctx.onShowMessage({ uid: "s2", step: 0, black: false, laser: { on: false }, zoom: { k: 1, x: 0.5, y: 0.5 } });
    assert.equal(ctx.SHOW.live, null, "a different slide leaves 互動");
    assert.ok(!stage.children.includes(el));
    await ctx.liveFollow({ t: "full", open: spec });
    await ctx.liveFollow(null);
    assert.equal(ctx.SHOW.live, null, "the presenter's Esc reaches the audience");
    // the presenter: its view controls and sends; 對調螢幕 carries the state over
    exec(`SHOW.i = 0; setRoot("single"); showRender(0);`);
    await ctx.liveOpen(spec);
    assert.equal(ctx.SHOW.live.view.role, "full", "a single screen gets the controls");
    const snap = run("snapshot()");
    assert.ok(snap.live && snap.live.spec, "the state crosses 對調螢幕");
    exec(`showRender(0)`);
    assert.ok(ctx.SHOW.stage.children.includes(ctx.SHOW.live.el), "single screen: survives showRender too");
    // Esc in real full screen arrives as leaving full screen: it ends 互動, not the show
    exec(`showFsChange()`);
    assert.equal(ctx.SHOW.live, null, "leaving full screen in 互動 closes 互動");
    assert.equal(ctx.SHOW.on, true, "…and keeps the show");
    console.log("show.test.js ok");
    process.exit(0);
  })().catch((e) => { console.error(e); process.exit(1); });
}
