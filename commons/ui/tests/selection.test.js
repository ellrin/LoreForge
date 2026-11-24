// node commons/ui/tests/selection.test.js — when LoreSelection (commons/ui/selection.js) hands a
// selection to L-Echo: a new text goes, the same text goes again only after a gesture of the author's
// that changed the selection, and an undo / redo (the browser's or the app's quiet()) never goes.
// A fake document and fetch; real timers (the page waits ~400 ms before it sends).
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const src = fs.readFileSync(path.join(__dirname, "..", "selection.js"), "utf8");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function page() {
  const on = {};
  const document = {
    addEventListener: (t, f) => (on[t] ??= []).push(f),
  };
  const fire = (t, e = {}) => (on[t] ?? []).forEach((f) => f({ type: t, ...e }));
  const sent = [];
  let n = 0;
  const fetch = async (url, o = {}) => {
    if (o.method === "POST" && url === "/api/lore/selection") {
      sent.push(JSON.parse(o.body).text);
      return { ok: true, status: 200, json: async () => ({ request: `r${++n}` }) };
    }
    return { ok: true, status: 200, json: async () => ({ open: true, pending: [] }) };   // the poll: still open
  };
  const ctx = vm.createContext({ window: {}, document, fetch, console, setTimeout, clearTimeout, Date, JSON, Set, encodeURIComponent });
  vm.runInContext(src, ctx);
  let text = "";
  ctx.window.LoreSelection.watch({ gather: () => (text ? { doc: "d", text } : null), apply: async () => true });
  return {
    sel: ctx.window.LoreSelection, sent, fire,
    select(t) { text = t; fire("selectionchange"); },
    /** a press, the selection made while held, the release */
    async drag(t) { fire("mousedown"); this.select(""); this.select(t); fire("mouseup"); await sleep(480); },
  };
}

(async () => {
  // a new selection goes; the same one staying on screen does not, nor a click that leaves it alone
  {
    const p = page();
    await p.drag("first sentence.");
    assert.deepEqual(p.sent, ["first sentence."]);
    p.fire("selectionchange"); p.fire("keyup"); await sleep(480);
    p.fire("mousedown"); p.fire("mouseup"); await sleep(480);          // a button click: selection untouched
    p.fire("keydown"); p.fire("keyup"); await sleep(480);              // ⌘C
    assert.deepEqual(p.sent, ["first sentence."]);
    // the same text by a new gesture: handed over again (L-Reader, item 4)
    await p.drag("first sentence.");
    assert.deepEqual(p.sent, ["first sentence.", "first sentence."]);
    // and after that, staying on screen is quiet again
    p.fire("mousedown"); p.fire("mouseup"); await sleep(480);
    assert.equal(p.sent.length, 2);
  }
  // the browser's undo in a text field: the text it selects is not a selection of his
  {
    const p = page();
    await p.drag("rewritten words.");
    p.fire("keydown");
    p.fire("beforeinput", { inputType: "historyUndo" });
    p.select("the original words.");
    p.fire("keyup"); await sleep(480);
    assert.deepEqual(p.sent, ["rewritten words."]);
    // a selection he makes himself after it still goes
    await sleep(600);
    await p.drag("the original words.");
    assert.deepEqual(p.sent, ["rewritten words.", "the original words."]);
  }
  // the app's own undo / redo through quiet(): what it leaves selected is not sent, its result is returned
  {
    const p = page();
    await p.drag("one.");
    const r = await p.sel.quiet(async () => { p.select("two."); await sleep(30); return 7; });
    p.select("two.");   // a late selectionchange after the undo's re-render
    await sleep(480);
    assert.equal(r, 7);
    assert.deepEqual(p.sent, ["one."]);
    // typing text in a field (input but no undo): a caret only, nothing to send
    p.fire("keydown"); p.fire("beforeinput", { inputType: "insertText" }); p.select(""); p.fire("keyup"); await sleep(480);
    assert.deepEqual(p.sent, ["one."]);
  }
  console.log("selection.test.js: ok");
  process.exit(0);   // the page's poll timers would keep node running
})().catch((e) => { console.error(e); process.exit(1); });
