// node web/tests/selection.test.js — the page text L-Echo gets around a selection in a PDF
// (`pageWindow` in app.js, taken out of the module source: app.js imports pdf.js and needs a browser).
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const src = fs.readFileSync(path.join(__dirname, "..", "app.js"), "utf8");
const grab = (head) => {
  const at = src.indexOf(head);
  assert.ok(at >= 0, head);
  return src.slice(at, src.indexOf("\n}\n", at) + 3);
};
const ctx = vm.createContext({});
vm.runInContext(`${src.match(/^const norm = .*$/m)[0]}\n${grab("function pageWindow(")}\nglobalThis.pageWindow = pageWindow;`, ctx);
const { pageWindow } = ctx;

const page = "A".repeat(2000) + "  the selected words\n" + "B".repeat(2000);
const s = 2002, e = s + "the selected words".length;

// about 1,200 characters on each side, the selection in the middle, whitespace folded
{
  const w = pageWindow(page, s, e);
  assert.ok(w.includes("the selected words"));
  assert.equal(w, "A".repeat(1198) + " the selected words " + "B".repeat(1199));
  assert.ok(w.length <= 2500);
}
// near the start of the page: what there is
assert.equal(pageWindow("short page, the term here.", 12, 20), "short page, the term here.");
// position unknown: the start of the page
assert.equal(pageWindow(page, null, null), "A".repeat(2000) + " the selected words " + "B".repeat(379));
assert.equal(pageWindow(page, 5, 5), pageWindow(page, null, null));
// a long selection leaves itself out: the context stays near 2,500
{
  const long = "x".repeat(4000);
  const t = "A".repeat(1500) + long + "B".repeat(1500);
  const w = pageWindow(t, 1500, 5500);
  assert.equal(w, "A".repeat(1200) + " … " + "B".repeat(1200));
  assert.ok(!w.includes("x"));
}
console.log("ok");
