// node web/tests/selection.test.js — what a rewrite (潤飾, L-Echo) sees of a slide: scope and context
// (`selectionContext` in arrange.js), without a browser.
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const web = path.join(__dirname, "..");
const ctx = vm.createContext({ console, Math, JSON, Map, Set, localStorage: { getItem: () => null } });
vm.runInContext(fs.readFileSync(path.join(web, "arrange.js"), "utf8") + "\n;globalThis.selectionContext = selectionContext;", ctx);
const paraText = (p) => p.runs.map((r) => r.text ?? "").join("");
const box = (id, y, x, ...paras) => ({ id, type: "text", frame: { x, y, w: 100, h: 100 }, text: { paras: paras.map((t) => ({ runs: [{ kind: "text", text: t }] })) } });
const at = (sl, n = 3, id = 2, pi = 0) => JSON.parse(JSON.stringify(ctx.selectionContext(sl, n, id, pi, paraText)));

// the title by each of the three tests; the title is left out of the context
for (const mark of [{ ph: "title" }, { ph: "ctrTitle" }, { slot: "title-2" }]) {
  const sl = { objects: [{ ...box(1, 0, 0, "Attention is all you need"), ...mark }, box(2, 500, 0, "picked")] };
  const r = at(sl);
  assert.equal(r.scope, "第 3 頁：Attention is all you need", JSON.stringify(mark));
  assert.ok(!r.context.includes("Attention"), "the title is not context");
}

// no title (an empty title slot does not count)
{
  const sl = { objects: [{ ...box(1, 0, 0, " "), slot: "title" }, box(2, 500, 0, "picked")] };
  assert.equal(at(sl, 7).scope, "第 7 頁");
}

// the same box's other paragraphs first, then the slide's text top then left, then the notes
{
  const sl = {
    notes: "Say this slowly.",
    objects: [
      box(5, 900, 50, "bottom"),
      box(4, 300, 600, "middle right"),
      box(2, 100, 0, "picked", "second line", ""),
      box(3, 300, 10, "middle left", "more"),
      { ...box(6, 0, 0, "Results"), ph: "title" },
    ],
  };
  const r = at(sl);
  assert.equal(r.scope, "第 3 頁：Results");
  assert.deepEqual(r.context.split("\n"), [
    "同一個文字框：second line",
    "本頁文字：middle left / more",
    "本頁文字：middle right",
    "本頁文字：bottom",
    "講者備註：Say this slowly.",
  ]);
}

// notes are capped at 600 characters, the whole context at about 2,500
{
  const r = at({ notes: "n".repeat(900), objects: [box(2, 0, 0, "picked")] });
  assert.equal(r.context, `講者備註：${"n".repeat(600)}…`);
  const long = { notes: "keep me", objects: [box(2, 0, 0, "picked"), ...[1, 2, 3, 4].map((k) => box(10 + k, k * 10, 0, "w".repeat(900)))] };
  const c = at(long).context;
  assert.ok(c.length <= 2500, `capped (${c.length})`);
  assert.ok(c.endsWith("…") && !c.includes("講者備註"), "what does not fit is cut");
}

console.log("selection.test.js ok");
